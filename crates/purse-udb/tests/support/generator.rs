// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! The seeded generator of random operation scripts over the ported
//! operations, shared by the live differential (`tests/oracle.rs`), the
//! frozen generated vectors (`tests/frozen.rs`) and the reopen property
//! (`tests/properties.rs`).
//!
//! A script draws its network from all four (simnet most often: its scrypt
//! cost is N = 2, every other network's is dcrwallet's full N = 262144),
//! its seed (16 to 64 bytes), passphrases and operations. Half the scripts
//! begin with the SLIP0044 coin type upgrade, as `dcrwallet --create` does
//! for a generated seed; the rest stay on the legacy coin type unless a
//! later upgrade succeeds. Arguments come from small sets, so that account
//! names collide, imported keys are recorded again as derived addresses,
//! lookups hit and miss, and counters reach their edges. No script passes
//! an account Go's marking methods panic on, or records a derived row in
//! the reserved account (`divergences_from_go_are_as_recorded` in
//! `tests/oracle.rs` covers those), and none upgrades the coin type after
//! an operation that may have recorded an address of account 0, where
//! dcr-purse derives the recorded rows again and Go keeps the legacy ones
//! (QK-0760; `coin_type_upgrade_rederives_legacy_rows` in
//! `tests/oracle.rs` covers that).

#![allow(
    clippy::arithmetic_side_effects,
    reason = "test code: small counts, lengths and generator sizes"
)]

use dcroxide_dcrec::secp256k1::ReducedPrivateKey;
use dcroxide_txscript::stdaddr;
use purse_testutil::serde_json::{Value, json};
use purse_testutil::{SplitMix64, hex};
use purse_udb::addrmgr::ChainParams;

/// The operations a generated script draws from (a name listed twice is
/// drawn twice as often).
pub const GEN_OPS: &[&str] = &[
    "mgr.watching_only",
    "mgr.is_locked",
    "mgr.coin_type",
    "mgr.coin_type_priv_key",
    "mgr.upgrade_to_slip0044_coin_type",
    "mgr.encrypt",
    "mgr.decrypt",
    "mgr.lock",
    "mgr.unlock",
    "mgr.unlock",
    "mgr.unlocked_with_passphrase",
    "mgr.last_account",
    "mgr.last_imported_account",
    "mgr.account_name",
    "mgr.for_each_account",
    "mgr.account_properties",
    "mgr.account_has_passphrase",
    "mgr.account_extended_pub_key",
    "mgr.account_extended_priv_key",
    "mgr.account_extended_priv_key",
    "mgr.account_branch_extended_pub_key",
    "udb.hd_keys_from_seed",
    "rt.reopen",
    "mgr.lookup_account",
    "mgr.new_account",
    "mgr.new_account",
    "mgr.rename_account",
    "mgr.import_xpub_account",
    "mgr.address",
    "mgr.address",
    "mgr.addr_account",
    "mgr.exists_hash160",
    "mgr.import_private_key",
    "mgr.import_private_key",
    "mgr.import_script",
    "mgr.private_key",
    "mgr.private_key",
    "mgr.have_private_key",
    "mgr.redeem_script",
    "mgr.sync_account_to_addr_index",
    "mgr.sync_account_to_addr_index",
    "mgr.record_derived_address",
    "mgr.mark_used",
    "mgr.mark_used_child_index",
    "mgr.mark_returned_child_index",
    "mgr.for_each_account_address",
    "mgr.for_each_active_account_address",
    "mgr.for_each_active_address",
];

/// The networks, by how often a script draws them.
const NETWORKS: [(&str, u64); 4] = [
    ("simnet", 82),
    ("regnet", 6),
    ("testnet3", 6),
    ("mainnet", 6),
];

/// How a generated script is shaped.
#[derive(Clone, Copy, Debug)]
pub struct Shape {
    /// Force this network instead of drawing one.
    pub network: Option<&'static str>,
    /// The most scrypt derivations (unlocks of a locked wallet, reopens)
    /// a script on a network other than simnet may run, beyond creation,
    /// opening and the export's check: each costs dcrwallet's full scrypt
    /// on each side.
    pub costly_scrypt_ops: usize,
}

impl Default for Shape {
    fn default() -> Self {
        Shape {
            network: None,
            costly_scrypt_ops: 2,
        }
    }
}

/// The private keys a generated script imports: small scalars, the
/// largest one below the group order, and one that reduces modulo it.
pub fn import_key(rng: &mut SplitMix64) -> [u8; 32] {
    let mut k = [0u8; 32];
    match rng.below(7) {
        0..=3 => k[31] = 1 + u8::try_from(rng.below(4)).expect("small"),
        4 => {
            k = unhex32("fffffffffffffffffffffffffffffffebaaedce6af48a03bbfd25e8cd0364140");
        }
        5 => {
            k = unhex32("fffffffffffffffffffffffffffffffebaaedce6af48a03bbfd25e8cd0364142");
        }
        _ => k[0] = 0x42,
    }
    k
}

fn unhex32(s: &str) -> [u8; 32] {
    purse_testutil::unhex(s).try_into().expect("32 bytes")
}

/// The compressed public key of an import key (Go's `WIF.PubKey`).
pub fn import_pub_key(k: &[u8; 32]) -> [u8; 33] {
    ReducedPrivateKey::from_bytes(k)
        .public_point()
        .serialize_compressed()
}

/// The scripts a generated script imports.
fn import_script(rng: &mut SplitMix64) -> String {
    match rng.below(4) {
        0 => "51".to_owned(),
        1 => "52".to_owned(),
        2 => String::new(),
        _ => format!("5121{}51ae", hex(&import_pub_key(&import_key(rng)))),
    }
}

/// An address argument in one of Go's reference forms.
fn addr_arg(rng: &mut SplitMix64, chain: &ChainParams) -> Value {
    match rng.below(8) {
        0..=2 => {
            let acct: u32 = match rng.below(8) {
                0..=3 => 0,
                4 => 1,
                5 => 0x7FFF_FFFF,
                6 => 0x8000_0000,
                _ => 2,
            };
            let branch = if rng.below(10) == 0 { 2 } else { rng.below(2) };
            let index = if rng.below(12) == 0 {
                0x8000_0000
            } else {
                rng.below(8)
            };
            json!({"acct": acct, "branch": branch, "index": index})
        }
        3 | 4 => {
            let pk = import_pub_key(&import_key(rng));
            let pkh =
                stdaddr::new_address_pub_key_hash_ecdsa_secp256k1_v0(&stdaddr::hash160(&pk), chain)
                    .expect("p2pkh");
            json!(pkh.encode())
        }
        5 => {
            let pk = import_pub_key(&import_key(rng));
            let p2pk =
                stdaddr::new_address_pub_key_ecdsa_secp256k1_v0_raw(&pk, chain).expect("p2pk");
            json!(p2pk.encode())
        }
        _ => json!({"p2sh": import_script(rng)}),
    }
}

/// The account names a generated script uses. The last one holds a
/// no-break space and a control character, which Go's `%q` writes as
/// `\u00a0` and `\x01` where Rust's `Debug` would write `\u{a0}` and
/// `\u{1}`, so its `Exist` texts check the quoting (ADR-0011; the
/// frozen generated vectors hold Go's `"a\u00a0b\x01"`, and
/// `quotes_as_go_does` in `src/addrmgr/accounts.rs` pins this name).
fn account_name(rng: &mut SplitMix64) -> &'static str {
    [
        "default",
        "one",
        "two",
        "imported",
        "",
        "päss \"q\"",
        "a\u{a0}b\u{1}",
    ][usize::try_from(rng.below(7)).expect("small")]
}

/// An account for the methods that never panic in Go (loading fails
/// cleanly for an unknown one).
fn any_account(rng: &mut SplitMix64) -> u32 {
    match rng.below(7) {
        0..=2 => 0,
        3 => 1,
        4 => 2,
        5 => 0x7FFF_FFFF,
        _ => 0x8000_0000 | u32::try_from(rng.below(2)).expect("small"),
    }
}

/// A passphrase: printable ASCII, sometimes empty, sometimes beyond ASCII.
pub fn passphrase(rng: &mut SplitMix64) -> String {
    match rng.below(8) {
        0 => String::new(),
        1 => "pässwörd ✓".to_owned(),
        _ => (0..1 + rng.below(24))
            .map(|_| char::from(0x20 + u8::try_from(rng.below(95)).expect("small")))
            .collect(),
    }
}

fn account(rng: &mut SplitMix64) -> u32 {
    match rng.below(6) {
        0..=2 => 0,
        3 => 0x7FFF_FFFF,
        4 => u32::try_from(rng.below(4)).expect("small"),
        _ => 0x8000_0000 | u32::try_from(rng.below(3)).expect("small"),
    }
}

fn network(rng: &mut SplitMix64) -> &'static str {
    let total: u64 = NETWORKS.iter().map(|n| n.1).sum();
    let mut x = rng.below(total);
    for (name, weight) in NETWORKS {
        if x < weight {
            return name;
        }
        x -= weight;
    }
    "simnet"
}

/// One random operation.
fn operation(rng: &mut SplitMix64, op: &str, chain: &ChainParams, privpass: &str) -> Value {
    match op {
        "mgr.unlock" | "mgr.unlocked_with_passphrase" => {
            let pass = if rng.below(3) == 0 {
                passphrase(rng)
            } else {
                privpass.to_owned()
            };
            json!({"op": op, "pass": pass})
        }
        "mgr.encrypt" | "mgr.decrypt" => {
            let key = if rng.below(2) == 0 {
                "public"
            } else {
                "private"
            };
            // A decrypt never opens a value of the other side's keys, so
            // its data is random: short boxes, missing nonces, bad tags.
            let len = usize::try_from(rng.below(72)).expect("small");
            json!({"op": op, "key": key, "data": hex(&rng.bytes(len))})
        }
        "mgr.account_name"
        | "mgr.account_properties"
        | "mgr.account_has_passphrase"
        | "mgr.account_extended_pub_key"
        | "mgr.account_extended_priv_key" => json!({"op": op, "account": account(rng)}),
        "mgr.account_branch_extended_pub_key" => {
            let branch = match rng.below(4) {
                0 => 0,
                1 => 1,
                2 => u32::try_from(rng.below(1 << 31)).expect("31 bits"),
                _ => 0x8000_0000,
            };
            json!({"op": op, "account": account(rng), "branch": branch})
        }
        "udb.hd_keys_from_seed" => {
            let len = usize::try_from(rng.below(70)).expect("small");
            json!({"op": op, "seed": hex(&rng.bytes(len))})
        }
        "mgr.lookup_account" | "mgr.new_account" => {
            json!({"op": op, "name": account_name(rng)})
        }
        "mgr.rename_account" => {
            let acct = match rng.below(5) {
                0 => 9,
                _ => any_account(rng),
            };
            json!({"op": op, "account": acct, "name": account_name(rng)})
        }
        "mgr.import_xpub_account" => json!({
            "op": op,
            "name": account_name(rng),
            "xpub": {"seed": "000102030405060708090a0b0c0d0e0f",
                     "path": ["44'", "1'", format!("{}'", rng.below(3))]},
        }),
        "mgr.address"
        | "mgr.addr_account"
        | "mgr.private_key"
        | "mgr.have_private_key"
        | "mgr.redeem_script"
        | "mgr.mark_used" => json!({"op": op, "addr": addr_arg(rng, chain)}),
        "mgr.exists_hash160" => {
            let h = if rng.below(2) == 0 {
                stdaddr::hash160(&import_pub_key(&import_key(rng))).to_vec()
            } else {
                rng.bytes(20)
            };
            json!({"op": op, "hash160": hex(&h)})
        }
        "mgr.import_private_key" => json!({"op": op, "key": hex(&import_key(rng))}),
        "mgr.import_script" => json!({"op": op, "script": import_script(rng)}),
        "mgr.sync_account_to_addr_index" => {
            let index = if rng.below(15) == 0 {
                0x8000_0000
            } else {
                rng.below(9)
            };
            let branch = if rng.below(10) == 0 { 2 } else { rng.below(2) };
            json!({"op": op, "account": any_account(rng), "index": index, "branch": branch})
        }
        "mgr.record_derived_address" => {
            // Account 0 only. Marking an address of an unknown account
            // panics in Go; a derived row in the reserved account makes
            // that account's address walk fail, so the exports' imported
            // keys go unchecked (`nopass`) and the masked comparison
            // proves nothing about them (corpus script 06 covers that row
            // without imported keys).
            let branch = rng.below(2);
            let child = rng.below(12);
            if rng.below(2) == 0 {
                json!({"op": op, "account": 0, "branch": branch, "child": child,
                       "pubkey": hex(&import_pub_key(&import_key(rng)))})
            } else {
                json!({"op": op, "account": 0, "branch": branch, "child": child})
            }
        }
        "mgr.mark_used_child_index" | "mgr.mark_returned_child_index" => {
            // Accounts that always exist (an unknown one panics in Go).
            let acct = if rng.below(4) == 0 { 0x7FFF_FFFF } else { 0 };
            let branch = if rng.below(8) == 0 { 2 } else { rng.below(2) };
            let child = match rng.below(10) {
                0 => u64::from(u32::MAX),
                1 => 0x8000_0000,
                _ => rng.below(12),
            };
            json!({"op": op, "account": acct, "branch": branch, "child": child})
        }
        "mgr.for_each_account_address" | "mgr.for_each_active_account_address" => {
            json!({"op": op, "account": any_account(rng)})
        }
        _ => json!({"op": op}),
    }
}

/// A random script of the ported operations.
pub fn generate(rng: &mut SplitMix64, id: usize, shape: Shape) -> Value {
    let network = shape.network.unwrap_or_else(|| network(rng));
    let chain = ChainParams::by_name(network).expect("network");
    let costly = network != "simnet";
    let seed_len = 16 + usize::try_from(rng.below(49)).expect("small");
    let seed = rng.bytes(seed_len);
    let pubpass = passphrase(rng);
    let mut privpass = passphrase(rng);
    if privpass.is_empty() {
        privpass.push('x');
    }
    let mut ops = Vec::new();
    if rng.below(2) == 0 {
        ops.push(json!({"op": "mgr.upgrade_to_slip0044_coin_type"}));
    }
    let mut scrypt_ops = 0;
    // Whether an operation so far may have recorded an address of account
    // 0. An upgrade after one would reach the recorded divergence QK-0760
    // (`coin_type_upgrade_rederives_legacy_rows` in `tests/oracle.rs`), so
    // the generator draws `mgr.coin_type` in its place; neither draws from
    // the stream, so the rest of the script is unchanged.
    let mut account0_recorded = false;
    for _ in 0..8 + rng.below(24) {
        let mut op = GEN_OPS[usize::try_from(rng.below(GEN_OPS.len() as u64)).expect("small")];
        if costly && matches!(op, "mgr.unlock" | "rt.reopen") {
            if scrypt_ops >= shape.costly_scrypt_ops {
                op = "mgr.is_locked";
            } else {
                scrypt_ops += 1;
            }
        }
        if account0_recorded && op == "mgr.upgrade_to_slip0044_coin_type" {
            op = "mgr.coin_type";
        }
        let v = operation(rng, op, &chain, &privpass);
        if op == "mgr.record_derived_address"
            || (op == "mgr.sync_account_to_addr_index" && v["account"] == 0)
        {
            account0_recorded = true;
        }
        ops.push(v);
    }
    json!({
        "format": "purse-opscript 1",
        "name": format!("generated {id}"),
        "setup": {
            "network": network,
            "seed": hex(&seed),
            "pubpass": pubpass,
            "privpass": privpass,
            "rand": hex(&rng.bytes(32)),
            "clock": 1_794_990_000,
        },
        "ops": ops,
    })
}
