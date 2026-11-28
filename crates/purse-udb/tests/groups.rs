// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! `update` and `view` groups with address manager members (ADR-0002
//! section 2, "Transactions", and section 6): the Rust runner runs a group
//! as one `purse-store` transaction (`tests/support/runner/groups.rs`), and
//! the address manager's memory follows the committed store when an
//! `update` group rolls back, where Go's manager keeps what the
//! rolled-back transaction put in its memory (QK-0733, not reproduced:
//! ADR-0002, decision by the project owner of 2026-11-19, item 2).
//!
//! - `rolled_back_groups_follow_the_store_where_go_keeps_its_memory` pins
//!   every case found on both sides, record by record: a rolled-back
//!   `RenameAccount` (Go's `AccountProperties` reports the new name,
//!   QK-0733), an account read in a rolled-back transaction after
//!   `NewAccount`, `ImportXpubAccount` or `ImportVotingAccount` (Go keeps
//!   it cached and answers for an account its store does not hold, and a
//!   later account of that number takes the cached keys and name,
//!   QK-0790), a rolled-back `SetAccountPassphrase` that sets a
//!   passphrase and one that removes it (QK-0733) and a
//!   rolled-back `UpgradeToSLIP0044CoinType` (Go keeps account 0's
//!   SLIP0044 keys cached and records later addresses under them,
//!   QK-0791). Every other record is equal, and so are the exports where
//!   neither store took a write the other refused. Each case also checks,
//!   without Go, that the Rust records after a rolled-back group are those
//!   of the same script without that group: memory follows the store.
//! - `go_corpus_15_is_a_pinned_divergence`: the Phase 0 Go corpus script
//!   `15-rollback-bookkeeping` (`crates/purse-export-check`) is refused by
//!   the Rust runner, since its groups hold `ChangePassphrase` and
//!   `ConvertToWatchingOnly`, which in dcr-purse commit a transaction of
//!   their own (`groups::ITS_OWN_TRANSACTION`); Go's results show its
//!   memory after each rolled-back group; and the script without those
//!   groups runs on both sides, equal but for the rolled-back
//!   `SetAccountPassphrase`.
//! - `go_corpus_group_scripts_match_go`: the Go corpus scripts
//!   `01-accounts`, `02-addresses`, `16-tolerated-errors` and
//!   `17-resolution-errors`, whose groups hold address manager operations,
//!   run on both sides, every record and the exports equal.
//! - `members_that_cannot_share_the_group_transaction_are_refused`: the
//!   refusals, as script faults.
//! - `manager_members_may_not_tolerate_a_panic_stand_in`: an address
//!   manager member may not tolerate the kind of an error dcr-purse
//!   returns where Go panics (`IO` on `MarkUsed`, `MarkUsedChildIndex` and
//!   `MarkReturnedChildIndex`, QK-0039; `Crypto` on `PrivateKey` after a
//!   conversion and before a reopen, QK-0763), since Go's group would roll
//!   back where the Rust group commits; the same members tolerating other
//!   kinds, and QK-0630's `Other`, which no list names, end their groups
//!   on both sides alike.
//! - `a_voting_import_after_a_conversion_commits_where_gos_group_rolls_back`:
//!   QK-0765 in a group, pinned on both sides.
//! - `generated_process_transaction_record_scripts_match_go`: generated
//!   scripts shaped like dcrwallet's `processTransactionRecord`
//!   (`tests/support/ptrgen.rs`): `Address`, `MarkUsed`,
//!   `AccountProperties`, `SyncAccountToAddrIndex`, `RedeemScript` and
//!   the imports as members of the transaction store's `update` groups
//!   beside `InsertMinedTx`, `InsertMemPoolTx`, `AddCredit`,
//!   `AddTicketCommitment` and `AddMultisigOut`, groups that roll back
//!   after the manager wrote among them, on all four networks
//!   (`PURSE_UDB_PTR_SCRIPTS` sets how many), every record and the final
//!   exports equal; a fixed set of them is frozen (`tests/frozen.rs`,
//!   `ptr_genvectors.txt`).
//! - `a_corrupted_member_state_inside_a_group_is_caught` and
//!   `a_group_is_checked_after_it_commits_or_rolls_back`: the store's
//!   consistency checks (ADR-0018 BK4) run on an `update` group's
//!   transaction after every member, before it commits or rolls back, and
//!   on the store after every group, committed or rolled back: an edit
//!   made at each point, one of a kind refused by each of the five checks
//!   (`check_rows`, `check_chain`, `check_records`, `check_tx_tables` and
//!   `txstore::check_decoded`), is refused there.
//! - `process_transaction_record_scripts_avoid_the_pinned_sequences`: those
//!   scripts never count the `unspent` table in a group after a write
//!   (QK-0003; crate `txstore`, choice 15, pending owner) and never put in
//!   a group a member the Rust runner refuses or one whose rollback the
//!   tests above pin as a divergence.

mod support;

use std::collections::BTreeMap;
use std::path::PathBuf;

use purse_testutil::serde_json::{self, Value, json};
use purse_testutil::{SplitMix64, oracle_or_skip};
use support::compare::{
    check_privcheck, check_record, comparable, compare_exports, covered_digest, differential,
    go_run,
};
use support::ptrgen::{PTR_MEMBERS, generate_ptr};
use support::txgen::counts_after_unspent_writes;
use support::{runner, scratch};

/// The member that ends a group with an untolerated error, rolling it
/// back: a lookup of an account that does not exist (`NotExist`).
fn fail() -> Value {
    json!({"op": "mgr.lookup_account", "name": "no such account"})
}

/// A simnet script of `ops` under `name`, its stream key the byte `rand`
/// repeated.
fn script(name: &str, rand: &str, ops: &[Value]) -> Value {
    json!({
        "format": "purse-opscript 1",
        "name": format!("rolled-back group: {name}"),
        "setup": {"network": "simnet",
                  "seed": "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
                  "pubpass": "public", "privpass": "private",
                  "rand": rand.repeat(32), "clock": 1_794_990_000},
        "ops": ops,
    })
}

/// Whether the two final exports are compared.
enum Exports {
    /// Equal under the masked comparison (`compare_exports`): neither
    /// store took a write the other's memory refused, or the stores differ
    /// only in values the comparison masks (case
    /// `remove-account-passphrase`: the account sealed under a different
    /// passphrase of its own on each side, the records showing it pinned).
    Equal,
    /// Different: after the rolled-back group, one side wrote what the
    /// other refused (a row Go records from memory its store does not
    /// back, or a passphrase Go's memory refuses to set); the records
    /// that show it are pinned instead, and both exports must still pass
    /// their own checks.
    Differ,
}

/// One pinned case: the records where Go and dcr-purse differ, by index,
/// with Go's record and dcr-purse's (without `i`, `op` and `drawn`).
struct Case {
    name: &'static str,
    rand: &'static str,
    ops: Vec<Value>,
    div: Vec<(usize, Value, Value)>,
    exports: Exports,
    /// Whether a rolled-back group changes memory that is not the store's
    /// (`Unlock`, `Lock`), which a rollback leaves changed on both sides,
    /// so the records without the group are not the same.
    memory: bool,
}

/// A record without `i`, `op` and `drawn`, masked as the comparison masks.
fn outcome(rec: &Value) -> Value {
    let mut v = comparable(rec);
    if let Some(m) = v.as_object_mut() {
        m.remove("i");
        m.remove("op");
    }
    v
}

/// The cases (`rolled_back_groups_follow_the_store_where_go_keeps_its_memory`).
#[allow(
    clippy::too_many_lines,
    reason = "test data: each case's script and its pinned records"
)]
fn cases() -> Vec<Case> {
    vec![
        Case {
            name: "rename",
            rand: "71",
            ops: vec![
                json!({"op": "mgr.unlock", "pass": "private"}),
                json!({"op": "mgr.new_account", "name": "a"}),
                json!({"op": "mgr.account_properties", "account": 1}),
                json!({"op": "update", "ops": [{"op": "mgr.rename_account", "account": 1, "name": "b"}, {"op": "mgr.lookup_account", "name": "no such account"}]}),
                json!({"op": "mgr.account_properties", "account": 1}),
                json!({"op": "mgr.account_name", "account": 1}),
                json!({"op": "mgr.lookup_account", "name": "b"}),
                json!({"op": "mgr.lookup_account", "name": "a"}),
                json!({"op": "rt.reopen"}),
                json!({"op": "mgr.account_properties", "account": 1}),
            ],
            div: vec![(
                4,
                json!({"ok": {"account": 1, "encrypted": false, "extret": u32::MAX, "extused": u32::MAX, "importedkeys": 0, "intret": u32::MAX, "intused": u32::MAX, "name": "b", "type": 1, "unlocked": false}}),
                json!({"ok": {"account": 1, "encrypted": false, "extret": u32::MAX, "extused": u32::MAX, "importedkeys": 0, "intret": u32::MAX, "intused": u32::MAX, "name": "a", "type": 1, "unlocked": false}}),
            )],
            exports: Exports::Equal,
            memory: false,
        },
        Case {
            name: "new-account",
            rand: "72",
            ops: vec![
                json!({"op": "mgr.unlock", "pass": "private"}),
                json!({"op": "update", "ops": [{"op": "mgr.new_account", "name": "x"}, {"op": "mgr.account_extended_pub_key", "account": 1}, {"op": "mgr.lookup_account", "name": "no such account"}]}),
                json!({"op": "mgr.account_extended_pub_key", "account": 1}),
                json!({"op": "mgr.account_properties", "account": 1}),
                json!({"op": "mgr.account_branch_extended_pub_key", "account": 1, "branch": 0}),
                json!({"op": "mgr.last_account"}),
                json!({"op": "mgr.address", "addr": {"acct": 1, "branch": 0, "index": 0}}),
                json!({"op": "mgr.new_account", "name": "y"}),
                json!({"op": "mgr.account_name", "account": 1}),
                json!({"op": "mgr.account_properties", "account": 1}),
                json!({"op": "mgr.account_extended_pub_key", "account": 1}),
            ],
            div: vec![
                (
                    2,
                    json!({"ok": "spubVVu4DYjjJWabSnE9vjqE3Skx2V2CeG7H59tn2LrwKpZqGfShfsbrsx6ps6y8dsb4H5UGJuVkxouyPKfoAwg9wEG7awkHBeX21FbbswDxcSN"}),
                    json!({"err": {"kind": "NotExist", "text": "item does not exist: no account 1"}}),
                ),
                (
                    3,
                    json!({"err": {"kind": "IO", "text": "I/O error:: item does not exist: no account 1"}}),
                    json!({"err": {"kind": "NotExist", "text": "item does not exist: no account 1"}}),
                ),
                (
                    4,
                    json!({"ok": "spubVXUfMcSrDyxtbqumDD4gpDaKd6yghQGWbQXYUM57pbqjXvzbPXNSTsX6AMmKNLWtZP3F4HT4wC5x4GFzLuAupuSCfiMx5MwZFiUSGLCZbEF"}),
                    json!({"err": {"kind": "NotExist", "text": "item does not exist: no account 1"}}),
                ),
                (
                    6,
                    json!({"err": {"kind": "NotExist", "text": "item does not exist: no address SsWWi3xemg83UyY7QWLaVciaXMPwG6jqWDp"}}),
                    json!({"err": {"kind": "NotExist", "text": "mgr.address addr: AccountBranchExtendedPubKey: item does not exist: no account 1"}}),
                ),
                (
                    9,
                    json!({"ok": {"account": 1, "encrypted": false, "extret": u32::MAX, "extused": u32::MAX, "importedkeys": 0, "intret": u32::MAX, "intused": u32::MAX, "name": "x", "type": 1, "unlocked": false}}),
                    json!({"ok": {"account": 1, "encrypted": false, "extret": u32::MAX, "extused": u32::MAX, "importedkeys": 0, "intret": u32::MAX, "intused": u32::MAX, "name": "y", "type": 1, "unlocked": false}}),
                ),
            ],
            exports: Exports::Equal,
            memory: false,
        },
        Case {
            name: "new-account-rows",
            rand: "73",
            ops: vec![
                json!({"op": "mgr.unlock", "pass": "private"}),
                json!({"op": "update", "ops": [{"op": "mgr.new_account", "name": "x"}, {"op": "mgr.account_extended_pub_key", "account": 1}, {"op": "mgr.lookup_account", "name": "no such account"}]}),
                json!({"op": "mgr.sync_account_to_addr_index", "account": 1, "index": 0, "branch": 0}),
                json!({"op": "rt.reopen"}),
                json!({"op": "mgr.unlock", "pass": "private"}),
                json!({"op": "mgr.new_account", "name": "y"}),
                json!({"op": "mgr.for_each_account_address", "account": 1}),
                json!({"op": "mgr.address", "addr": {"acct": 1, "branch": 0, "index": 0}}),
            ],
            div: vec![
                (
                    2,
                    json!({"ok": null}),
                    json!({"err": {"kind": "NotExist", "text": "item does not exist: no account 1"}}),
                ),
                (
                    6,
                    json!({"ok": [{"account": 1, "addr": "SsWWi3xemg83UyY7QWLaVciaXMPwG6jqWDp", "addrhash": "17b92abce8b03f8ab99e177a6ff796d8363ef341", "imported": false, "index": 0, "internal": false, "kind": "pubkey", "multisig": false, "pubkey": "029b9fc1477a3d6d8e14540e81f2702454adc1da3b4e4b9b7623eace91f7d948f9"}]}),
                    json!({"ok": []}),
                ),
                (
                    7,
                    json!({"ok": {"account": 1, "addr": "SsWWi3xemg83UyY7QWLaVciaXMPwG6jqWDp", "addrhash": "17b92abce8b03f8ab99e177a6ff796d8363ef341", "imported": false, "index": 0, "internal": false, "kind": "pubkey", "multisig": false, "pubkey": "029b9fc1477a3d6d8e14540e81f2702454adc1da3b4e4b9b7623eace91f7d948f9"}}),
                    json!({"err": {"kind": "NotExist", "text": "item does not exist: no address SsWWi3xemg83UyY7QWLaVciaXMPwG6jqWDp"}}),
                ),
            ],
            exports: Exports::Differ,
            memory: false,
        },
        Case {
            name: "xpub",
            rand: "74",
            ops: vec![
                json!({"op": "update", "ops": [{"op": "mgr.import_xpub_account", "name": "xp", "xpub": {"seed": "a0a1a2a3a4a5a6a7a8a9aaabacadaeafb0b1b2b3b4b5b6b7b8b9babbbcbdbebf", "path": ["44'", "1'", "0'"]}}, {"op": "mgr.account_extended_pub_key", "account": 2_147_483_648_u32}, {"op": "mgr.lookup_account", "name": "no such account"}]}),
                json!({"op": "mgr.account_extended_pub_key", "account": 2_147_483_648_u32}),
                json!({"op": "mgr.account_properties", "account": 2_147_483_648_u32}),
                json!({"op": "mgr.last_imported_account"}),
                json!({"op": "mgr.address", "addr": {"acct": 2_147_483_648_u32, "branch": 0, "index": 0}}),
            ],
            div: vec![
                (
                    1,
                    json!({"ok": "spubVUevaJrSoinVirTodJFvyU4i4tezDByNCCouNhu8DUs2KphnufZ6pD2T9rL3qRqwg8AJ9tgapuYhPH69RHxQmm3s2d6XPzuHbADFmty3cK8"}),
                    json!({"err": {"kind": "NotExist", "text": "item does not exist: no account 2147483648"}}),
                ),
                (
                    2,
                    json!({"err": {"kind": "IO", "text": "I/O error:: item does not exist: no account 2147483648"}}),
                    json!({"err": {"kind": "NotExist", "text": "item does not exist: no account 2147483648"}}),
                ),
                (
                    4,
                    json!({"err": {"kind": "NotExist", "text": "item does not exist: no address SsWzcGw1tH6qu8RwXwEcecvtwHTMZF69zMp"}}),
                    json!({"err": {"kind": "NotExist", "text": "mgr.address addr: AccountBranchExtendedPubKey: item does not exist: no account 2147483648"}}),
                ),
            ],
            exports: Exports::Equal,
            memory: false,
        },
        Case {
            name: "voting",
            rand: "75",
            ops: vec![
                json!({"op": "mgr.unlock", "pass": "private"}),
                json!({"op": "update", "ops": [{"op": "mgr.import_voting_account", "name": "voter", "pass": "vp", "xpriv": {"seed": "c0c1c2c3c4c5c6c7c8c9cacbcccdcecfd0d1d2d3d4d5d6d7d8d9dadbdcdddedf", "path": []}}, {"op": "mgr.account_properties", "account": 2_147_483_648_u32}, {"op": "mgr.lookup_account", "name": "no such account"}]}),
                json!({"op": "mgr.account_has_passphrase", "account": 2_147_483_648_u32}),
                json!({"op": "mgr.unlock_account", "account": 2_147_483_648_u32, "pass": "vp"}),
                json!({"op": "mgr.account_extended_pub_key", "account": 2_147_483_648_u32}),
                json!({"op": "mgr.last_imported_account"}),
                json!({"op": "mgr.import_voting_account", "name": "voter2", "pass": "vp2", "xpriv": {"seed": "d0d1d2d3d4d5d6d7d8d9dadbdcdddedfe0e1e2e3e4e5e6e7e8e9eaebecedeeef", "path": []}}),
                json!({"op": "mgr.account_properties", "account": 2_147_483_648_u32}),
                json!({"op": "mgr.account_extended_pub_key", "account": 2_147_483_648_u32}),
                json!({"op": "mgr.unlock_account", "account": 2_147_483_648_u32, "pass": "vp"}),
                json!({"op": "mgr.unlock_account", "account": 2_147_483_648_u32, "pass": "vp2"}),
                json!({"op": "rt.reopen"}),
                json!({"op": "mgr.account_extended_pub_key", "account": 2_147_483_648_u32}),
            ],
            div: vec![
                (
                    2,
                    json!({"ok": {"has": true, "unlocked": false}}),
                    json!({"ok": {"has": false, "unlocked": false}}),
                ),
                (
                    3,
                    json!({"ok": null}),
                    json!({"err": {"kind": "NotExist", "text": "item does not exist: no account 2147483648"}}),
                ),
                (
                    4,
                    json!({"ok": "spubVNx6fk6e22GL2mHj9CJau6g2FkV2BjWWaxs5CFoiuEtRjvm4LWz3dnTPC8PoASrSy6TYbHV8GGEHki3gBnb7KgxFyfUYFDMWKNYgisz1sBV"}),
                    json!({"err": {"kind": "NotExist", "text": "item does not exist: no account 2147483648"}}),
                ),
                (
                    7,
                    json!({"ok": {"account": 2_147_483_648_u32, "encrypted": true, "extret": u32::MAX, "extused": u32::MAX, "importedkeys": 0, "intret": u32::MAX, "intused": u32::MAX, "name": "voter", "type": 2, "unlocked": true}}),
                    json!({"ok": {"account": 2_147_483_648_u32, "encrypted": true, "extret": u32::MAX, "extused": u32::MAX, "importedkeys": 0, "intret": u32::MAX, "intused": u32::MAX, "name": "voter2", "type": 2, "unlocked": false}}),
                ),
                (
                    8,
                    json!({"ok": "spubVNx6fk6e22GL2mHj9CJau6g2FkV2BjWWaxs5CFoiuEtRjvm4LWz3dnTPC8PoASrSy6TYbHV8GGEHki3gBnb7KgxFyfUYFDMWKNYgisz1sBV"}),
                    json!({"ok": "spubVNx6fk6e22GL2TsHW8q6JwTWcFMWfY2Z1qL55KMixrb4xhq7wLnrgJsZUNwUit8rZe1YaDPDxtnkRx8d64qSsPWiGZo6aJcaLG4MvCecyzx"}),
                ),
                (
                    9,
                    json!({"ok": null}),
                    json!({"err": {"kind": "Passphrase", "text": "invalid passphrase"}}),
                ),
                (
                    10,
                    json!({"err": {"kind": "Passphrase", "text": "invalid passphrase"}}),
                    json!({"ok": null}),
                ),
            ],
            exports: Exports::Equal,
            memory: false,
        },
        Case {
            name: "account-passphrase",
            rand: "76",
            ops: vec![
                json!({"op": "mgr.unlock", "pass": "private"}),
                json!({"op": "mgr.new_account", "name": "second"}),
                json!({"op": "update", "ops": [{"op": "mgr.set_account_passphrase", "account": 1, "pass": "acct-pass"}, {"op": "mgr.lookup_account", "name": "no such account"}]}),
                json!({"op": "mgr.account_has_passphrase", "account": 1}),
                json!({"op": "mgr.account_properties", "account": 1}),
                json!({"op": "mgr.unlock_account", "account": 1, "pass": "acct-pass"}),
                json!({"op": "mgr.lock"}),
                json!({"op": "mgr.unlock", "pass": "private"}),
                json!({"op": "mgr.account_extended_priv_key", "account": 1}),
                json!({"op": "mgr.set_account_passphrase", "account": 1, "pass": "p2"}),
                json!({"op": "rt.reopen"}),
                json!({"op": "mgr.unlock", "pass": "private"}),
                json!({"op": "mgr.account_has_passphrase", "account": 1}),
            ],
            div: vec![
                (
                    3,
                    json!({"ok": {"has": true, "unlocked": true}}),
                    json!({"ok": {"has": false, "unlocked": false}}),
                ),
                (
                    4,
                    json!({"ok": {"account": 1, "encrypted": true, "extret": u32::MAX, "extused": u32::MAX, "importedkeys": 0, "intret": u32::MAX, "intused": u32::MAX, "name": "second", "type": 1, "unlocked": true}}),
                    json!({"ok": {"account": 1, "encrypted": false, "extret": u32::MAX, "extused": u32::MAX, "importedkeys": 0, "intret": u32::MAX, "intused": u32::MAX, "name": "second", "type": 1, "unlocked": false}}),
                ),
                (
                    5,
                    json!({"ok": null}),
                    json!({"err": {"kind": "Crypto", "text": "encryption/decryption error: account is not encrypted with a unique passphrase"}}),
                ),
                (
                    8,
                    json!({"err": {"kind": "Locked", "text": "wallet locked: unable to access account extended privkey"}}),
                    json!({"ok": "sprvZGuhp3CqU92JEJ9gpiJDgJpDUTBiEoPRhvyBDxTKmV2rPs7Z8LHcL9nM1pq7dChCLvnLJe3Q5aVmhMD4cSt3CNVpi8xzfy1cVikQvV8vgf4"}),
                ),
                (
                    9,
                    json!({"err": {"kind": "Locked", "text": "wallet locked: account must be unlocked to set a unique account passphrase"}}),
                    json!({"ok": null}),
                ),
                (
                    12,
                    json!({"ok": {"has": false, "unlocked": false}}),
                    json!({"ok": {"has": true, "unlocked": false}}),
                ),
            ],
            exports: Exports::Differ,
            memory: false,
        },
        // A rolled-back removal (`SetAccountPassphrase` with an empty
        // passphrase; Go's `removeAccountPassphrase`,
        // wallet/udb/addressmanager.go:1837-1840, changes the cached
        // account before the commit): Go's memory reports no passphrase and,
        // after a wallet lock and unlock, opens the account's key with the
        // private crypto key and takes a new passphrase dcr-purse refuses;
        // after a reopen each store opens the account with its own
        // passphrase. The stores then hold the account sealed under
        // different passphrases, which the masked export comparison does
        // not see.
        Case {
            name: "remove-account-passphrase",
            rand: "79",
            ops: vec![
                json!({"op": "mgr.unlock", "pass": "private"}),
                json!({"op": "mgr.new_account", "name": "one"}),
                json!({"op": "mgr.set_account_passphrase", "account": 1, "pass": "a"}),
                json!({"op": "update", "ops": [{"op": "mgr.set_account_passphrase", "account": 1, "pass": ""}, {"op": "mgr.lookup_account", "name": "no such account"}]}),
                json!({"op": "mgr.account_has_passphrase", "account": 1}),
                json!({"op": "mgr.account_properties", "account": 1}),
                json!({"op": "mgr.account_extended_priv_key", "account": 1}),
                json!({"op": "mgr.lock"}),
                json!({"op": "mgr.unlock", "pass": "private"}),
                json!({"op": "mgr.account_has_passphrase", "account": 1}),
                json!({"op": "mgr.account_extended_priv_key", "account": 1}),
                json!({"op": "mgr.set_account_passphrase", "account": 1, "pass": "b"}),
                json!({"op": "rt.reopen"}),
                json!({"op": "mgr.unlock", "pass": "private"}),
                json!({"op": "mgr.unlock_account", "account": 1, "pass": "a"}),
                json!({"op": "mgr.unlock_account", "account": 1, "pass": "b"}),
                json!({"op": "mgr.account_has_passphrase", "account": 1}),
            ],
            div: vec![
                (
                    4,
                    json!({"ok": {"has": false, "unlocked": false}}),
                    json!({"ok": {"has": true, "unlocked": true}}),
                ),
                (
                    5,
                    json!({"ok": {"account": 1, "encrypted": false, "extret": u32::MAX, "extused": u32::MAX, "importedkeys": 0, "intret": u32::MAX, "intused": u32::MAX, "name": "one", "type": 1, "unlocked": false}}),
                    json!({"ok": {"account": 1, "encrypted": true, "extret": u32::MAX, "extused": u32::MAX, "importedkeys": 0, "intret": u32::MAX, "intused": u32::MAX, "name": "one", "type": 1, "unlocked": true}}),
                ),
                (
                    9,
                    json!({"ok": {"has": false, "unlocked": false}}),
                    json!({"ok": {"has": true, "unlocked": false}}),
                ),
                (
                    10,
                    json!({"ok": "sprvZGuhp3CqU92JEJ9gpiJDgJpDUTBiEoPRhvyBDxTKmV2rPs7Z8LHcL9nM1pq7dChCLvnLJe3Q5aVmhMD4cSt3CNVpi8xzfy1cVikQvV8vgf4"}),
                    json!({"err": {"kind": "Locked", "text": "wallet locked: unable to access account extended privkey"}}),
                ),
                (
                    11,
                    json!({"ok": null}),
                    json!({"err": {"kind": "Locked", "text": "wallet locked: account must be unlocked to set a unique account passphrase"}}),
                ),
                (
                    14,
                    json!({"err": {"kind": "Passphrase", "text": "invalid passphrase"}}),
                    json!({"ok": null}),
                ),
                (
                    15,
                    json!({"ok": null}),
                    json!({"err": {"kind": "Passphrase", "text": "invalid passphrase"}}),
                ),
            ],
            exports: Exports::Equal,
            memory: false,
        },
        // The same removal, then the store's account passphrase given at
        // once: Go refuses it (`Crypto`), its memory holding no passphrase,
        // where dcr-purse unlocks the account; without a new passphrase, a
        // reopen makes both sides answer from the store alike.
        Case {
            name: "remove-account-passphrase-unlock",
            rand: "7a",
            ops: vec![
                json!({"op": "mgr.unlock", "pass": "private"}),
                json!({"op": "mgr.new_account", "name": "one"}),
                json!({"op": "mgr.set_account_passphrase", "account": 1, "pass": "a"}),
                json!({"op": "update", "ops": [{"op": "mgr.set_account_passphrase", "account": 1, "pass": ""}, {"op": "mgr.lookup_account", "name": "no such account"}]}),
                json!({"op": "mgr.account_has_passphrase", "account": 1}),
                json!({"op": "mgr.unlock_account", "account": 1, "pass": "a"}),
                json!({"op": "mgr.lock"}),
                json!({"op": "mgr.unlock", "pass": "private"}),
                json!({"op": "mgr.account_extended_priv_key", "account": 1}),
                json!({"op": "rt.reopen"}),
                json!({"op": "mgr.unlock", "pass": "private"}),
                json!({"op": "mgr.account_has_passphrase", "account": 1}),
                json!({"op": "mgr.account_extended_priv_key", "account": 1}),
                json!({"op": "mgr.unlock_account", "account": 1, "pass": "a"}),
            ],
            div: vec![
                (
                    4,
                    json!({"ok": {"has": false, "unlocked": false}}),
                    json!({"ok": {"has": true, "unlocked": true}}),
                ),
                (
                    5,
                    json!({"err": {"kind": "Crypto", "text": "encryption/decryption error: account is not encrypted with a unique passphrase"}}),
                    json!({"ok": null}),
                ),
                (
                    8,
                    json!({"ok": "sprvZGuhp3CqU92JEJ9gpiJDgJpDUTBiEoPRhvyBDxTKmV2rPs7Z8LHcL9nM1pq7dChCLvnLJe3Q5aVmhMD4cSt3CNVpi8xzfy1cVikQvV8vgf4"}),
                    json!({"err": {"kind": "Locked", "text": "wallet locked: unable to access account extended privkey"}}),
                ),
            ],
            exports: Exports::Equal,
            memory: false,
        },
        Case {
            name: "upgrade",
            rand: "77",
            ops: vec![
                json!({"op": "mgr.account_extended_pub_key", "account": 0}),
                json!({"op": "update", "ops": [{"op": "mgr.upgrade_to_slip0044_coin_type"}, {"op": "mgr.lookup_account", "name": "no such account"}]}),
                json!({"op": "mgr.coin_type"}),
                json!({"op": "mgr.account_extended_pub_key", "account": 0}),
                json!({"op": "mgr.account_branch_extended_pub_key", "account": 0, "branch": 0}),
                json!({"op": "mgr.address", "addr": {"acct": 0, "branch": 0, "index": 0}}),
                json!({"op": "mgr.unlock", "pass": "private"}),
                json!({"op": "mgr.account_extended_priv_key", "account": 0}),
                json!({"op": "mgr.upgrade_to_slip0044_coin_type"}),
                json!({"op": "mgr.account_extended_pub_key", "account": 0}),
            ],
            div: vec![
                (
                    3,
                    json!({"ok": "spubVVrPNQr4JLvwfZJYSZP2nvpKRwE3GkYF2TxckSdVzizjtUff1SaaQatyQBvDGRrSpWMrfPLsReN4poLxtdNztcRBTrYJrWTDeEob3d3UMyq"}),
                    json!({"ok": "spubVVu4DYjjJWabPkg4yciSgUHyJr7wj76w1ihSQad5oEebGoaeBPRdbosDjCUpMsKz8sNj38P9PvBBssLn5anjBoNdrZJTn5QHd5oAuYCvG7u"}),
                ),
                (
                    4,
                    json!({"ok": "spubVWzmhE2qbz351brHSB8ejqyjaAkdVg3VmYDmw5g2HKfTycp2FtgTNR5m1StnBSPLbaqPXLezUQStwS6DsNvpGBHbSQmLyVt84CJzpTBEDAf"}),
                    json!({"ok": "spubVXHUnXdwCexq8zD5Fr4wBpGPaGKDjXi48P9enqaLGrH8h1ufJvskQyqAhpZYa43rzdis43xK28pCVsw4drgjBSzhA4RNYx782YqCgnuKe8G"}),
                ),
                (
                    5,
                    json!({"err": {"kind": "NotExist", "text": "item does not exist: no address SsYtnzeaDx25mZZyXAKmJTwwDzkDGemXF4R"}}),
                    json!({"err": {"kind": "NotExist", "text": "item does not exist: no address SsauyxW8SVNCnV7oKkm828wb67q9qRbicAV"}}),
                ),
                (
                    7,
                    json!({"ok": "sprvZGs2xuKATyNeT5E5LXr2RnsasuPYsHpPfF31x4DtSPTm1gLWTuGKrnaVYtfRCuxdtN1ZdpRKYeUXHFMF2ukVeFhf97TBVPk8duCf3Lw6bAt"}),
                    json!({"ok": "sprvZGuhp3CqU92JBGbbsbBSKLMEkpHTKeP5eVmqcCDUEu7cQ1FVdr7P41YjstbzzTTmK2fMKWxWbc3rL5xQcS3MXfHAkdhrNf1VEJ1LZS3ZX4h"}),
                ),
            ],
            exports: Exports::Equal,
            memory: false,
        },
        Case {
            name: "upgrade-rows",
            rand: "78",
            ops: vec![
                json!({"op": "mgr.account_extended_pub_key", "account": 0}),
                json!({"op": "update", "ops": [{"op": "mgr.upgrade_to_slip0044_coin_type"}, {"op": "mgr.lookup_account", "name": "no such account"}]}),
                json!({"op": "mgr.sync_account_to_addr_index", "account": 0, "index": 0, "branch": 0}),
                json!({"op": "rt.reopen"}),
                json!({"op": "mgr.for_each_account_address", "account": 0}),
                json!({"op": "mgr.address", "addr": {"acct": 0, "branch": 0, "index": 0}}),
            ],
            div: vec![(
                5,
                json!({"err": {"kind": "NotExist", "text": "item does not exist: no address SsauyxW8SVNCnV7oKkm828wb67q9qRbicAV"}}),
                json!({"ok": {"account": 0, "addr": "SsauyxW8SVNCnV7oKkm828wb67q9qRbicAV", "addrhash": "480083c8f78228f46a12ac65892bad165898049b", "imported": false, "index": 0, "internal": false, "kind": "pubkey", "multisig": false, "pubkey": "03bf9616da05cd78ae2b570bdbc2317f24f78859be84ef8b877956c8d90d6ec998"}}),
            )],
            exports: Exports::Differ,
            memory: false,
        },
        Case {
            name: "lock",
            rand: "70",
            ops: vec![
                json!({"op": "update", "ops": [{"op": "mgr.unlock", "pass": "private"}, {"op": "mgr.lookup_account", "name": "no such account"}]}),
                json!({"op": "mgr.is_locked"}),
                json!({"op": "mgr.account_extended_priv_key", "account": 0}),
                json!({"op": "update", "ops": [{"op": "mgr.lock"}, {"op": "mgr.lookup_account", "name": "no such account"}]}),
                json!({"op": "mgr.is_locked"}),
                json!({"op": "mgr.account_extended_priv_key", "account": 0}),
            ],
            div: vec![],
            exports: Exports::Equal,
            memory: true,
        },
    ]
}

/// The script `ops` without its `update` groups that end in an error.
fn without_rolled_back_groups(ops: &[Value]) -> Vec<Value> {
    ops.iter()
        .filter(|o| {
            !(o["op"] == "update" && o["ops"].as_array().and_then(|m| m.last()) == Some(&fail()))
        })
        .cloned()
        .collect()
}

/// QK-0733, QK-0790 and QK-0791 (PARITY.md, `purse-udb`, "Groups"): after
/// an `update` group that rolls back, Go's address manager keeps what the
/// group put in its memory, and dcr-purse's follows the store. Each case
/// pins Go's and dcr-purse's records where they differ; every other record
/// is equal, and the exports are equal unless the case says otherwise
/// (`Exports::Differ`: then they must differ, each passing its own
/// checks). Without Go, each case checks that the Rust records after a
/// rolled-back group are those of the same script without it, the
/// exports equal: the group left nothing in dcr-purse's memory.
#[test]
fn rolled_back_groups_follow_the_store_where_go_keeps_its_memory() {
    let mut oracle = oracle_or_skip("oprunner");
    for case in cases() {
        let what = case.name;
        let s = script(what, case.rand, &case.ops);
        let dir = scratch("rolled-back");
        let rust = runner::run(&s, &dir.join("rust")).unwrap_or_else(|e| panic!("{what}: {e}"));
        for (i, _, r) in &case.div {
            assert_eq!(outcome(&rust.results[*i]), *r, "{what}: Rust op {i}");
        }
        // Memory follows the store: the script without its rolled-back
        // groups gives the same records for the operations left, and the
        // same export.
        let kept = without_rolled_back_groups(&case.ops);
        let plain = runner::run(&script(what, case.rand, &kept), &dir.join("plain"))
            .unwrap_or_else(|e| panic!("{what} without its rolled-back groups: {e}"));
        let left: Vec<Value> = rust
            .results
            .iter()
            .filter(|r| !(r["op"] == "update" && r.get("err").is_some()))
            .map(outcome)
            .collect();
        let plain_records: Vec<Value> = plain.results.iter().map(outcome).collect();
        if !case.memory {
            assert_eq!(
                left, plain_records,
                "{what}: records without the rolled-back groups"
            );
        }
        compare_exports_rust(what, &rust.export, &plain.export);
        if let Some(o) = oracle.as_mut() {
            let (go, go_pe) = go_run(o, &s, &dir.join("go"));
            assert_eq!(go.len(), rust.results.len(), "{what}: record count");
            for (i, (g, r)) in go.iter().zip(&rust.results).enumerate() {
                match case.div.iter().find(|d| d.0 == i) {
                    Some((_, want, _)) => assert_eq!(outcome(g), *want, "{what}: Go op {i}"),
                    None => check_record(what, g, r),
                }
            }
            match case.exports {
                Exports::Equal => compare_exports(what, &go_pe, &rust.export),
                Exports::Differ => {
                    for (side, pe) in [("Go", &go_pe), ("Rust", &rust.export)] {
                        let ex = purse_export_check::parse(pe)
                            .unwrap_or_else(|e| panic!("{what}: {side} export: {e}"));
                        check_privcheck(&ex).unwrap_or_else(|e| panic!("{what}: {side}: {e}"));
                    }
                    assert_ne!(
                        covered_digest(&go_pe),
                        covered_digest(&rust.export),
                        "{what}: the stores were to differ"
                    );
                }
            }
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// Two Rust exports of the same state, compared as the differential
/// compares Go's with dcr-purse's (`covered_digest`).
fn compare_exports_rust(what: &str, a: &str, b: &str) {
    assert_eq!(
        covered_digest(a),
        covered_digest(b),
        "{what}: exports with and without the rolled-back groups"
    );
}

/// The Phase 0 Go corpus script `name` (`crates/purse-export-check`).
fn go_corpus(name: &str) -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../purse-export-check/tests/data/opscripts")
        .join(format!("{name}.json"));
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{name}: {e}"));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{name}: {e}"))
}

/// Whether `op` is a group holding a member the Rust runner refuses
/// because it commits a transaction of its own.
fn holds_its_own_transaction(op: &Value) -> bool {
    matches!(op["op"].as_str(), Some("update" | "view"))
        && op["ops"].as_array().is_some_and(|m| {
            m.iter().any(|x| {
                runner::groups::ITS_OWN_TRANSACTION.contains(&x["op"].as_str().unwrap_or_default())
            })
        })
}

/// Go corpus script `15-rollback-bookkeeping` (QK-0733; ADR-0002, decision
/// by the project owner of 2026-11-19, items 2 and 4; PARITY.md,
/// `purse-udb`, "Groups"). The Rust runner refuses it as written: its
/// first group holds `ChangePassphrase`, which in dcr-purse commits a
/// transaction of its own and cannot share the group's. Go's results show
/// its memory after the rolled-back groups: the old private passphrase
/// refused and the rolled-back new one accepted, an account passphrase
/// reported that its store does not hold, and a wallet reported
/// watching-only that is not, until a reopen; its imported voting account
/// and block do not outlive their groups. Without the four groups that
/// hold `ChangePassphrase` or `ConvertToWatchingOnly`, the script runs on
/// both sides, every record equal but the account passphrase after the
/// rolled-back `SetAccountPassphrase`, and the checkpoint and final exports
/// equal.
#[test]
fn go_corpus_15_is_a_pinned_divergence() {
    let s = go_corpus("15-rollback-bookkeeping");
    let dir = scratch("go-corpus-15");
    let refused = runner::run(&s, &dir.join("rust")).expect_err("the Rust runner refuses it");
    assert!(
        refused.starts_with("op 2: mgr.change_passphrase in update:"),
        "{refused}"
    );
    let ops = s["ops"].as_array().expect("ops");
    let kept: Vec<Value> = ops
        .iter()
        .filter(|o| !holds_its_own_transaction(o))
        .cloned()
        .collect();
    assert_eq!(
        ops.len() - kept.len(),
        4,
        "the groups the Rust runner refuses"
    );
    let mut rest = s.clone();
    rest["ops"] = Value::Array(kept);
    let rust = runner::run(&rest, &dir.join("rust-rest")).expect("the rest runs");
    // After the rolled-back SetAccountPassphrase (op 10 of the rest).
    let has = 11;
    assert_eq!(rust.results[has]["op"], "mgr.account_has_passphrase");
    assert_eq!(
        rust.results[has]["ok"],
        json!({"has": false, "unlocked": false})
    );
    let Some(mut o) = oracle_or_skip("oprunner") else {
        let _ = std::fs::remove_dir_all(&dir);
        return;
    };
    let (go, _) = go_run(&mut o, &s, &dir.join("go"));
    let at = |i: usize| outcome(&go[i]);
    let passphrase = json!({"err": {"kind": "Passphrase", "text": "invalid passphrase"}});
    // The rolled-back private passphrase change: Go refuses the old one
    // and accepts the new one until a reopen.
    assert_eq!(at(6), passphrase, "{}", go[6]);
    assert_eq!(at(7), json!({"ok": null}), "{}", go[7]);
    assert_eq!(at(11), json!({"ok": null}), "after the reopen: {}", go[11]);
    // The rolled-back account passphrase.
    assert_eq!(
        at(13),
        json!({"ok": {"has": true, "unlocked": true}}),
        "{}",
        go[13]
    );
    // The rolled-back voting account and block follow the store.
    assert_eq!(at(15), json!({"ok": 2_147_483_647}), "{}", go[15]);
    assert_eq!(go[16]["err"]["kind"], "NotExist", "{}", go[16]);
    assert_eq!(go[18]["ok"]["height"], 0, "{}", go[18]);
    // The rolled-back conversion: watching-only until a reopen.
    assert_eq!(at(24), json!({"ok": true}), "{}", go[24]);
    assert_eq!(go[26]["err"]["kind"], "WatchingOnly", "{}", go[26]);
    assert_eq!(at(29), json!({"ok": false}), "after the reopen: {}", go[29]);
    // The rest: every record equal but the account passphrase.
    let go_rest = differential_but(&mut o, &rest, &[has]);
    assert_eq!(
        outcome(&go_rest[has]),
        json!({"ok": {"has": true, "unlocked": true}})
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// `differential`, except at the records `but`, which differ by a pinned
/// divergence; returns Go's records.
fn differential_but(o: &mut purse_testutil::Oracle, s: &Value, but: &[usize]) -> Vec<Value> {
    let dir = scratch("diff-but");
    let (go, go_pe) = go_run(o, s, &dir.join("go"));
    let rust = runner::run(s, &dir.join("rust")).expect("Rust runner");
    assert_eq!(go.len(), rust.results.len(), "record count");
    for (i, (g, r)) in go.iter().zip(&rust.results).enumerate() {
        if !but.contains(&i) {
            check_record("15 without its refused groups", g, r);
        }
    }
    let go_cps = support::compare::go_checkpoints(&go, &dir.join("go"));
    support::compare::compare_checkpoints(
        "15 without its refused groups",
        &go_cps,
        &rust.checkpoints,
    );
    compare_exports("15 without its refused groups", &go_pe, &rust.export);
    let _ = std::fs::remove_dir_all(&dir);
    go
}

/// The Go corpus scripts whose groups hold address manager operations and
/// none that commits its own transaction: `01-accounts` (`NewAccount`
/// and `RenameAccount` in one `update`), `02-addresses` (`Address` in a
/// `view`), `16-tolerated-errors` (`Address` and `ImportScript`
/// tolerating `NotExist` and `Exist` beside transaction store and
/// treasury policy members, and `LookupAccount` in an `update` and a
/// `view`) and `17-resolution-errors` (`NewAccount` and two `Address`
/// members in an `update`), every record and the exports equal.
#[test]
fn go_corpus_group_scripts_match_go() {
    let Some(mut o) = oracle_or_skip("oprunner") else {
        return;
    };
    for name in [
        "01-accounts",
        "02-addresses",
        "16-tolerated-errors",
        "17-resolution-errors",
    ] {
        let n = differential(&mut o, name, &go_corpus(name)).len();
        eprintln!("Go corpus {name}: {n} operations equal");
    }
}

/// A test's edit of the store (`runner::Corruption`).
type Edit = fn(&mut purse_store::WriteTx<'_>) -> purse_store::Result<()>;

/// The store edits the consistency check tests make, each one refused by
/// a different check of the five (`runner/groups.rs`, `consistent_in`;
/// `consistent` in `runner.rs`), with the end of that check's text: an
/// `accounts` row that does not decode (`check_rows`), a `block_by_hash`
/// entry with no block (`check_chain`), a VSP host with no public-key row
/// (`check_records`), an unspent commitment entry with no commitment
/// (`check_tx_tables`; not an unmined credit with no unmined record,
/// which `txstore::check_decoded` refuses too, with the same text), and
/// an unmined record, alone in the transaction tables, whose transaction
/// does not decode (`txstore::check_decoded`; the four checks before it
/// take the row, whose bytes they do not decode). Removing any one check
/// from `consistent_in` or `consistent` fails the tests below.
const CORRUPTIONS: [(&str, &str, Edit); 5] = [
    ("rows", "accounts: account key len 18", |tx| {
        tx.put_raw(purse_store::TableId::Accounts, b"not an account key", b"x")
    }),
    ("chain", "block_by_hash and blocks differ in length", |tx| {
        tx.put_raw(
            purse_store::TableId::BlockByHash,
            &[0x11; 32],
            &5u32.to_be_bytes(),
        )
    }),
    (
        "records",
        "vsp_hosts and vsp_pubkeys differ in length",
        |tx| {
            tx.put::<purse_store::VspHosts>(
                &purse_store::VspHostId(0xffff),
                &purse_store::VspHost(b"unpaired".to_vec()),
            )
        },
    ),
    (
        "tx tables",
        "unspent commitment entry \
         3333333333333333333333333333333333333333333333333333333333333333:0 without its \
         commitment",
        |tx| {
            tx.put::<purse_store::CommitmentsUnspent>(
                &purse_store::Outpoint {
                    hash: purse_store::Hash32([0x33; 32]),
                    index: 0,
                },
                &purse_store::CommitmentUnspentRow {
                    unmined_spent: false,
                },
            )
        },
    ),
    (
        "decoded",
        "unmined record 2222222222222222222222222222222222222222222222222222222222222222 \
         does not decode: MsgTx.BtcDecode: unsupported transaction type",
        |tx| {
            tx.put::<purse_store::TxUnmined>(
                &purse_store::Hash32([0x22; 32]),
                &purse_store::TxUnminedRow {
                    received: 1_794_990_000,
                    tx: vec![0xde, 0xad, 0xbe, 0xef],
                },
            )
        },
    ),
];

/// A script whose operation 1 is the group `ops` (of kind `kind`),
/// between two top-level reads.
fn check_script(kind: &str, ops: &[Value]) -> Value {
    script(
        &format!("consistency check in a {kind} group"),
        "c1",
        &[
            json!({"op": "mgr.last_account"}),
            json!({"op": kind, "ops": ops}),
            json!({"op": "mgr.last_account"}),
        ],
    )
}

/// Verify's per-operation consistency check holds inside an `update`
/// group run as one transaction (`runner/groups.rs`, `consistent_in`;
/// ADR-0018 BK4): after every member, the one whose untolerated error
/// ends the group included, the group's write transaction is checked
/// with every write of the group so far, before it commits or rolls back.
/// A test's edit of that transaction right after a member is refused at
/// that member, by each of the five checks (one edit for each,
/// [`CORRUPTIONS`]), in a group that commits and in one
/// that rolls back, where the rollback would otherwise remove the edit
/// before the per-operation check after the group could see it. Without
/// an edit the scripts run.
#[test]
fn a_corrupted_member_state_inside_a_group_is_caught() {
    let read = json!({"op": "mgr.lookup_account", "name": "default"});
    let last = json!({"op": "mgr.last_account"});
    let committing = [last.clone(), read.clone(), last.clone()];
    let rolling_back = [last.clone(), read.clone(), fail()];
    for (ops, ends) in [(&committing, false), (&rolling_back, true)] {
        let script = check_script("update", ops);
        let run = runner::run(&script, &scratch("group-check"))
            .unwrap_or_else(|e| panic!("without an edit: {e}"));
        assert_eq!(
            run.results[1].get("err").is_some(),
            ends,
            "{:?}",
            run.results[1]
        );
        for (what, text, edit) in CORRUPTIONS {
            for (j, member) in ops.iter().enumerate() {
                let c = runner::Corruption {
                    op: 1,
                    member: Some(j),
                    edit,
                };
                let e = runner::run_corrupted(&script, &scratch("group-check"), c)
                    .err()
                    .unwrap_or_else(|| panic!("{what} after member {j}: not refused"));
                let want = format!(
                    "op 1: update member {j} ({}): the group's transaction fails the \
                     consistency checks: ",
                    member["op"].as_str().expect("op")
                );
                assert!(
                    e.starts_with(&want) && e.ends_with(text),
                    "{what} after member {j} (rolls back: {ends}): {e}"
                );
            }
        }
    }
}

/// The per-operation check after a group checks the store its transaction
/// left, committed or rolled back (`run` in `runner.rs`): a test's edit
/// committed right after the group's transaction ended is refused there,
/// after an `update` group that commits, one that rolls back and a `view`
/// group.
#[test]
fn a_group_is_checked_after_it_commits_or_rolls_back() {
    let read = json!({"op": "mgr.lookup_account", "name": "default"});
    for (kind, ops) in [
        ("update", vec![read.clone(), read.clone()]),
        ("update", vec![read.clone(), fail()]),
        ("view", vec![read.clone(), read.clone()]),
    ] {
        let script = check_script(kind, &ops);
        runner::run(&script, &scratch("group-check"))
            .unwrap_or_else(|e| panic!("{kind} without an edit: {e}"));
        for (what, text, edit) in CORRUPTIONS {
            let c = runner::Corruption {
                op: 1,
                member: None,
                edit,
            };
            let e = runner::run_corrupted(&script, &scratch("group-check"), c)
                .err()
                .unwrap_or_else(|| panic!("{what} after the {kind} group: not refused"));
            let want = format!("op 1 ({kind}): the store fails the consistency checks: ");
            assert!(
                e.starts_with(&want) && e.ends_with(text),
                "{what} after the {kind} group: {e}"
            );
        }
    }
}

/// The members the Rust runner refuses as script faults: the address
/// manager methods that commit a transaction of their own in dcr-purse
/// (`ChangePassphrase`, `ConvertToWatchingOnly`), in an `update` or a
/// `view`, and the operations Go's runner takes only at the top level (a
/// group, `rt.reopen`, `rt.checkpoint`). A changing member of a `view`
/// group is refused too, the address manager's included.
#[test]
fn members_that_cannot_share_the_group_transaction_are_refused() {
    let refused = [
        (
            json!({"op": "mgr.change_passphrase", "old": "public", "new": "p2", "private": false}),
            "update",
            "mgr.change_passphrase in update: dcr-purse's method commits a write transaction",
        ),
        (
            json!({"op": "mgr.convert_to_watching_only"}),
            "update",
            "mgr.convert_to_watching_only in update: dcr-purse's method commits",
        ),
        (
            json!({"op": "mgr.convert_to_watching_only"}),
            "view",
            "mgr.convert_to_watching_only in view: dcr-purse's method commits",
        ),
        (
            json!({"op": "rt.reopen"}),
            "update",
            "unknown op \"rt.reopen\" in update",
        ),
        (
            json!({"op": "rt.checkpoint", "name": "c"}),
            "view",
            "unknown op \"rt.checkpoint\" in view",
        ),
        (
            json!({"op": "view", "ops": []}),
            "update",
            "unknown op \"view\" in update",
        ),
        (
            json!({"op": "mgr.new_account", "name": "n"}),
            "view",
            "a changing operation needs a read-write transaction, not a view",
        ),
        (
            json!({"op": "mgr.import_script", "script": "51"}),
            "view",
            "a changing operation needs a read-write transaction, not a view",
        ),
    ];
    for (member, group, want) in refused {
        let s = script(
            "refused member",
            "79",
            &[
                json!({"op": "mgr.unlock", "pass": "private"}),
                json!({"op": group, "ops": [{"op": "mgr.last_account"}, member]}),
            ],
        );
        let dir = scratch("refused-member");
        let e = runner::run(&s, &dir).expect_err("refused");
        assert!(
            e.starts_with("op 1: ") && e.contains(want),
            "{member} in {group}: {e}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// The extended key of the imported voting accounts below: simnet's
/// BIP0044 account `i` of a fixed seed.
fn voting_key(i: u32) -> Value {
    json!({"seed": "2525252525252525252525252525252525252525252525252525252525252525",
           "path": ["44'", "115'", format!("{i}'")]})
}

/// Address manager members whose operation returns an error where Go
/// panics (PARITY.md, `purse-udb`, "Groups"; `groups::PANIC_STAND_INS`,
/// `groups::LIVE_CONVERSION_STAND_INS`). The comparison accepts any error
/// against Go's `Panic`, so a member tolerating the stand-in error would
/// let the Rust group commit where Go's rolls back, every record compared
/// equal: the Rust runner refuses such a member as a script fault.
/// `MarkUsed`, `MarkUsedChildIndex` and `MarkReturnedChildIndex` on an
/// account with no row may not tolerate `IO` (QK-0039), and `PrivateKey`
/// may not tolerate `Crypto` after a conversion in this process and
/// before a reopen (QK-0763). The same members tolerating another kind,
/// and `PrivateKey` tolerating `Crypto` after a reopen, run: their groups
/// end, or commit, on both sides alike. `SetAccountPassphrase` and
/// `ImportVotingAccount` at 256 processors return kind `Other`, which no
/// `tolerate` list can name (QK-0630), and their groups roll back on both
/// sides. The refusals need no oracle; the Go sides run when one is
/// available.
#[test]
#[allow(clippy::too_many_lines, reason = "test data: one script per case")]
fn manager_members_may_not_tolerate_a_panic_stand_in() {
    // An address row of account 9, which has no account row: the address
    // is `SsWWi3xemg83UyY7QWLaVciaXMPwG6jqWDp` on simnet.
    let derived = json!({"op": "mgr.record_derived_address", "account": 9, "branch": 0,
                         "child": 1, "pubkey":
                         "029b9fc1477a3d6d8e14540e81f2702454adc1da3b4e4b9b7623eace91f7d948f9"});
    let missing_account = |member: Value| {
        script(
            "manager panic stand-in",
            "a3",
            &[
                json!({"op": "mgr.sync_account_to_addr_index", "account": 0, "index": 2, "branch": 0}),
                derived.clone(),
                json!({"op": "update", "ops": [
                    {"op": "mgr.mark_used", "addr": {"acct": 0, "branch": 0, "index": 2}},
                    member,
                    {"op": "mgr.sync_account_to_addr_index", "account": 0, "index": 5, "branch": 1},
                ]}),
                json!({"op": "mgr.account_properties", "account": 0}),
                json!({"op": "mgr.for_each_active_address"}),
            ],
        )
    };
    let converted = |reopen: bool, tolerate: &str| {
        let mut ops = vec![
            json!({"op": "mgr.unlock", "pass": "private"}),
            json!({"op": "mgr.import_private_key",
                   "key": "1111111111111111111111111111111111111111111111111111111111111111",
                   "label": "k"}),
            json!({"op": "mgr.convert_to_watching_only"}),
        ];
        if reopen {
            ops.push(json!({"op": "rt.reopen"}));
        }
        ops.push(json!({"op": "update", "ops": [
            {"op": "mgr.sync_account_to_addr_index", "account": 0, "index": 3, "branch": 0},
            {"op": "mgr.private_key", "addr": {"ref": "k"}, "tolerate": [tolerate]},
        ]}));
        ops.push(json!({"op": "mgr.for_each_active_address"}));
        script("PrivateKey after a conversion", "a6", &ops)
    };
    let refused = [
        (
            missing_account(json!({"op": "mgr.mark_used_child_index", "account": 9,
                                   "branch": 0, "child": 1, "tolerate": ["IO"]})),
            "op 2: mgr.mark_used_child_index: tolerate: IO is the kind of an error dcr-purse \
             returns where Go panics, and Go never tolerates a panic",
        ),
        (
            missing_account(json!({"op": "mgr.mark_returned_child_index", "account": 9,
                                   "branch": 1, "child": 1, "tolerate": ["NotExist", "IO"]})),
            "op 2: mgr.mark_returned_child_index: tolerate: IO is the kind",
        ),
        (
            missing_account(json!({"op": "mgr.mark_used",
                                   "addr": "SsWWi3xemg83UyY7QWLaVciaXMPwG6jqWDp",
                                   "tolerate": ["IO"]})),
            "op 2: mgr.mark_used: tolerate: IO is the kind",
        ),
        (
            converted(false, "Crypto"),
            "op 3: mgr.private_key: tolerate: Crypto is the kind of an error dcr-purse returns \
             where Go panics, after a conversion before a reopen, and Go never tolerates a panic",
        ),
    ];
    for (s, want) in &refused {
        let dir = scratch("stand-in");
        let e = runner::run(s, &dir).expect_err("refused");
        assert!(e.contains(want), "{want}: {e}");
        let _ = std::fs::remove_dir_all(&dir);
    }
    // QK-0630's error has kind `Other`, which a tolerate list cannot name.
    let ncpu = |tolerate: &str| {
        let mut s = script(
            "QK-0630 in a group",
            "a8",
            &[
                json!({"op": "mgr.unlock", "pass": "private"}),
                json!({"op": "update", "ops": [
                    {"op": "mgr.sync_account_to_addr_index", "account": 0, "index": 3, "branch": 0},
                    {"op": "mgr.set_account_passphrase", "account": 0, "pass": "zero",
                     "tolerate": [tolerate]},
                    {"op": "mgr.sync_account_to_addr_index", "account": 0, "index": 5, "branch": 1},
                ]}),
                json!({"op": "update", "ops": [
                    {"op": "mgr.sync_account_to_addr_index", "account": 0, "index": 4, "branch": 0},
                    {"op": "mgr.import_voting_account", "name": "vote", "pass": "abc",
                     "xpriv": voting_key(1), "tolerate": ["Exist"]},
                ]}),
                json!({"op": "mgr.account_has_passphrase", "account": 0}),
                json!({"op": "mgr.last_imported_account"}),
                json!({"op": "mgr.for_each_active_address"}),
            ],
        );
        s["setup"]["ncpu"] = json!(256);
        s
    };
    let dir = scratch("stand-in");
    let e = runner::run(&ncpu("Other"), &dir).expect_err("refused");
    assert!(
        e.contains("tolerate: \"Other\" is not a dcrwallet error kind other than Other"),
        "{e}"
    );
    let _ = std::fs::remove_dir_all(&dir);
    // The same members otherwise run: the stand-in ends the group, or
    // after a reopen PrivateKey's error is Go's too.
    let accepted = [
        (
            "MarkUsedChildIndex tolerating NotExist",
            missing_account(json!({"op": "mgr.mark_used_child_index", "account": 9,
                                   "branch": 0, "child": 1, "tolerate": ["NotExist"]})),
            false,
        ),
        (
            "MarkUsed tolerating NotExist",
            missing_account(json!({"op": "mgr.mark_used",
                                   "addr": "SsWWi3xemg83UyY7QWLaVciaXMPwG6jqWDp",
                                   "tolerate": ["NotExist"]})),
            false,
        ),
        (
            "PrivateKey tolerating Passphrase before a reopen",
            converted(false, "Passphrase"),
            false,
        ),
        (
            "PrivateKey tolerating Crypto after a reopen",
            converted(true, "Crypto"),
            true,
        ),
        ("QK-0630 in a group", ncpu("Passphrase"), false),
    ];
    for (what, s, commits) in &accepted {
        let dir = scratch("stand-in");
        let rust = runner::run(s, &dir).unwrap_or_else(|e| panic!("{what}: {e}"));
        let groups: Vec<&Value> = rust
            .results
            .iter()
            .filter(|r| r["op"] == "update")
            .collect();
        assert!(!groups.is_empty(), "{what}");
        for g in groups {
            assert_eq!(g.get("err").is_none(), *commits, "{what}: {g}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
    let Some(mut o) = oracle_or_skip("oprunner") else {
        return;
    };
    for (s, what) in &refused {
        // Go's group ends at the member with a panic, and rolls back.
        let dir = scratch("stand-in");
        let (go, _) = go_run(&mut o, s, &dir.join("go"));
        let g = go.iter().find(|r| r["op"] == "update").expect("the group");
        let members = g["ops"].as_array().expect("members");
        assert_eq!(
            members.last().expect("a member")["err"]["kind"],
            "Panic",
            "{what}: {g}"
        );
        assert_eq!(g["err"]["kind"], "Panic", "{what}: {g}");
        let _ = std::fs::remove_dir_all(&dir);
    }
    for (what, s, _) in &accepted {
        differential(&mut o, what, s);
    }
}

/// QK-0765 in a group (PARITY.md, `purse-udb`, "Groups"): after a
/// conversion in this process and before a reopen, Go's
/// `ImportVotingAccount` seals with the nil private crypto key and
/// panics, so its group rolls back, the members before it included,
/// where dcr-purse imports the account, as Go does after a reopen, and
/// the group commits. No error stands in for the panic, so no refusal of
/// a `tolerate` list applies; the comparison fails at the member, as it
/// does at the top level, and the generated scripts never draw the
/// sequence. Both sides are pinned record by record.
#[test]
fn a_voting_import_after_a_conversion_commits_where_gos_group_rolls_back() {
    let s = script(
        "QK-0765 in a group",
        "a7",
        &[
            json!({"op": "mgr.unlock", "pass": "private"}),
            json!({"op": "mgr.convert_to_watching_only"}),
            json!({"op": "update", "ops": [
                {"op": "mgr.sync_account_to_addr_index", "account": 0, "index": 3, "branch": 0},
                {"op": "mgr.import_voting_account", "name": "late", "pass": "l",
                 "xpriv": voting_key(3)},
            ]}),
            json!({"op": "mgr.last_imported_account"}),
            json!({"op": "mgr.for_each_active_address"}),
            json!({"op": "rt.reopen"}),
            json!({"op": "mgr.last_imported_account"}),
        ],
    );
    let dir = scratch("qk0765-group");
    let rust = runner::run(&s, &dir.join("rust")).expect("Rust runner");
    let r = &rust.results;
    assert_eq!(r[2].get("err"), None, "the group commits: {}", r[2]);
    assert_eq!(r[2]["ops"][1]["ok"], 0x8000_0000_u32, "{}", r[2]);
    assert_eq!(r[3]["ok"], 0x8000_0000_u32, "{}", r[3]);
    let active = r[4]["ok"].as_array().map_or(0, Vec::len);
    assert!(active > 0, "the sync committed: {}", r[4]);
    assert_eq!(r[6]["ok"], 0x8000_0000_u32, "{}", r[6]);
    let Some(mut o) = oracle_or_skip("oprunner") else {
        let _ = std::fs::remove_dir_all(&dir);
        return;
    };
    let (go, _) = go_run(&mut o, &s, &dir.join("go"));
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(go.len(), r.len());
    assert_eq!(go[2]["err"]["kind"], "Panic", "Go's group ends: {}", go[2]);
    assert_eq!(go[2]["ops"][0]["ok"], Value::Null, "{}", go[2]);
    assert_eq!(go[2]["ops"][1]["err"]["kind"], "Panic", "{}", go[2]);
    assert_eq!(go[3]["ok"], 0x7fff_ffff_u32, "no import: {}", go[3]);
    assert_eq!(go[4]["ok"], json!([]), "the sync rolled back: {}", go[4]);
    assert_eq!(go[6]["ok"], 0x7fff_ffff_u32, "after the reopen: {}", go[6]);
    for i in [0, 1, 5] {
        check_record("QK-0765 in a group", &go[i], &r[i]);
    }
}

/// Generated scripts shaped like `processTransactionRecord`
/// (`tests/support/ptrgen.rs`): `InsertMinedTx` or `InsertMemPoolTx`,
/// `AddCredit`, `AddTicketCommitment` and `AddMultisigOut` in `update`
/// groups with the address manager members dcrwallet runs in the same
/// transaction (`Address`, `MarkUsed`, `AccountProperties`,
/// `SyncAccountToAddrIndex`, `RedeemScript`, the imports), groups ending
/// by an untolerated error and rolling back among them, on all four
/// networks (`PURSE_UDB_PTR_SCRIPTS` sets how many; the first four are
/// forced onto each network in turn), every record and the final exports
/// equal.
#[test]
fn generated_process_transaction_record_scripts_match_go() {
    let Some(mut o) = oracle_or_skip("oprunner") else {
        return;
    };
    let count: usize = std::env::var("PURSE_UDB_PTR_SCRIPTS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(4);
    let stream = "purse-udb-ptr-groups";
    let mut rng = SplitMix64::from_env(stream);
    let nets = ["simnet", "regnet", "testnet3", "mainnet"];
    let (mut ops, mut groups, mut rolled_back, mut tolerated) = (0, 0, 0, 0);
    let (mut members, mut mgr_rolled_back) = (0, 0);
    let mut endings: BTreeMap<String, usize> = BTreeMap::new();
    let mut undone: BTreeMap<String, usize> = BTreeMap::new();
    for id in 0..count {
        let script = generate_ptr(&mut rng, id, nets.get(id).copied());
        let results = differential(&mut o, &format!("ptr script {id}: {script}"), &script);
        for r in &results {
            ops += 1;
            let Some(ms) = r["ops"].as_array() else {
                continue;
            };
            groups += 1;
            ops += ms.len();
            tolerated += ms.iter().filter(|m| m["tolerated"] == true).count();
            let mgr = ms
                .iter()
                .filter(|m| m["op"].as_str().is_some_and(|n| n.starts_with("mgr.")))
                .count();
            members += mgr;
            if r.get("err").is_some() {
                rolled_back += 1;
                if mgr > 0 {
                    mgr_rolled_back += 1;
                }
                if let Some(last) = ms.last() {
                    let k = format!(
                        "{} {}",
                        last["op"].as_str().unwrap_or("?"),
                        last["err"]["kind"].as_str().unwrap_or("?")
                    );
                    *endings.entry(k).or_default() += 1;
                }
                // The address manager writes a rolled-back group undid.
                for m in ms.iter().filter(|m| m.get("ok").is_some()) {
                    let n = m["op"].as_str().unwrap_or("?");
                    if n.starts_with("mgr.") {
                        *undone.entry(n.to_owned()).or_default() += 1;
                    }
                }
            }
        }
    }
    eprintln!("groups ended by: {endings:?}");
    eprintln!("address manager members that succeeded in a rolled-back group: {undone:?}");
    eprintln!(
        "{count} generated ptr scripts ({stream}), {ops} operations equal: {groups} groups \
         ({members} address manager members), {rolled_back} ended by an error \
         ({mgr_rolled_back} of them after an address manager member ran), {tolerated} \
         tolerated errors"
    );
}

/// The `processTransactionRecord` scripts never make the sequences that
/// are pinned elsewhere or pending: no `update` group counts the `unspent`
/// table after a member that may write it (QK-0003; crate `txstore`,
/// choice 15, pending owner; `txgen::counts_after_unspent_writes`), no
/// group holds a member that commits its own transaction or registers an
/// address label, or creates, renames or upgrades an account or sets an
/// account passphrase (QK-0733, QK-0790, QK-0791), and every group with an
/// address manager member records a transaction's output.
#[test]
fn process_transaction_record_scripts_avoid_the_pinned_sequences() {
    let mut rng = SplitMix64(0x7a3e_0b00);
    let barred = [
        "mgr.new_account",
        "mgr.rename_account",
        "mgr.import_xpub_account",
        "mgr.import_voting_account",
        "mgr.set_account_passphrase",
        "mgr.upgrade_to_slip0044_coin_type",
        "mgr.unlock",
        "mgr.lock",
    ];
    let (mut groups, mut with_mgr) = (0, 0);
    for id in 0..64 {
        let script = generate_ptr(&mut rng, id, None);
        assert!(
            counts_after_unspent_writes(&script).is_empty(),
            "ptr script {id}: a count after an unspent write in a group"
        );
        for op in script["ops"].as_array().expect("ops") {
            let Some(ms) = op["ops"].as_array() else {
                continue;
            };
            groups += 1;
            let names: Vec<&str> = ms.iter().map(|m| m["op"].as_str().unwrap_or("")).collect();
            for (m, n) in ms.iter().zip(&names) {
                assert!(
                    !runner::groups::ITS_OWN_TRANSACTION.contains(n) && !barred.contains(n),
                    "ptr script {id}: {n} in a group"
                );
                assert!(m.get("label").is_none(), "ptr script {id}: a label: {m}");
                if n.starts_with("mgr.") {
                    assert!(PTR_MEMBERS.contains(n), "ptr script {id}: {n}");
                }
            }
            if names.iter().any(|n| n.starts_with("mgr.")) {
                with_mgr += 1;
                assert!(
                    names.iter().any(|n| matches!(
                        *n,
                        "store.add_credit"
                            | "store.add_ticket_commitment"
                            | "store.add_multisig_out"
                    )),
                    "ptr script {id}: {op}"
                );
            }
        }
    }
    assert!(with_mgr > groups / 4, "{with_mgr} of {groups} groups");
}
