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

/// The operations a generated watching-only script draws from, besides
/// [`GEN_OPS`]: public key imports, which only a watching-only wallet
/// accepts, and conversion.
pub const WATCH_OPS: &[&str] = &[
    "mgr.import_public_key",
    "mgr.import_public_key",
    "mgr.convert_to_watching_only",
];

/// The operations a generated passphrase script draws from, besides
/// [`GEN_OPS`] (half its draws come from each list): per-account
/// passphrases and imported voting accounts.
pub const PASS_OPS: &[&str] = &[
    "mgr.set_account_passphrase",
    "mgr.set_account_passphrase",
    "mgr.unlock_account",
    "mgr.unlock_account",
    "mgr.unlock_account",
    "mgr.lock_account",
    "mgr.lock_account",
    "mgr.import_voting_account",
    "mgr.import_voting_account",
    "mgr.account_has_passphrase",
];

/// The operations a generated passphrase-change script draws from, besides
/// [`GEN_OPS`] (half its draws come from each list): changes of either
/// passphrase, and the operations that use the passphrases or the keys the
/// crypto keys seal.
pub const CHANGE_OPS: &[&str] = &[
    "mgr.change_passphrase",
    "mgr.change_passphrase",
    "mgr.change_passphrase",
    "mgr.unlock",
    "mgr.unlocked_with_passphrase",
    "mgr.lock",
    "rt.reopen",
    "mgr.account_extended_priv_key",
    "mgr.coin_type_priv_key",
    "mgr.private_key",
];

/// The most Argon2id derivations (`SetAccountPassphrase`, `UnlockAccount`,
/// `ImportVotingAccount`) a generated passphrase script draws: each costs
/// dcrwallet's full Argon2id (256 MiB) on each side, and the export checks
/// each account with its own passphrase once more.
const ARGON2_OPS: usize = 8;

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

/// An account for the per-account passphrase operations: BIP0044 accounts
/// (0 most often), the reserved imported account, imported voting or xpub
/// accounts, and an account that does not exist.
fn pass_account(rng: &mut SplitMix64) -> u32 {
    match rng.below(9) {
        0..=2 => 0,
        3 => 1,
        4 => 2,
        5 => 0x7FFF_FFFF,
        _ => 0x8000_0000 | u32::try_from(rng.below(3)).expect("small"),
    }
}

/// An account passphrase, from a small set so that unlocks often match:
/// sometimes empty (a removal, or a refusal), sometimes beyond ASCII.
fn account_passphrase(rng: &mut SplitMix64) -> &'static str {
    ["", "a", "a", "b", "b", "pässwörd ✓"][usize::try_from(rng.below(6)).expect("small")]
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
        "mgr.import_public_key" => {
            // Mostly the compressed keys of the import keys, so that they
            // collide with imported private keys; sometimes a key Go
            // refuses (uncompressed, a bad format byte, a bad length).
            let k = import_pub_key(&import_key(rng));
            let pk = match rng.below(8) {
                0 => {
                    let mut v = k.to_vec();
                    v[0] = 4;
                    v
                }
                1 => k[..32].to_vec(),
                2 => {
                    let mut v = k.to_vec();
                    v.push(0);
                    v
                }
                _ => k.to_vec(),
            };
            json!({"op": op, "pubkey": hex(&pk)})
        }
        "mgr.import_script" => json!({"op": op, "script": import_script(rng)}),
        "mgr.set_account_passphrase" | "mgr.unlock_account" => json!({
            "op": op,
            "account": pass_account(rng),
            "pass": account_passphrase(rng),
        }),
        "mgr.lock_account" => json!({"op": op, "account": pass_account(rng)}),
        "mgr.import_voting_account" => {
            // A few keys of one seed, so that a key is imported again
            // (accepted until its first address is recorded, then refused).
            let coin = if rng.below(2) == 0 {
                chain.legacy_coin_type
            } else {
                chain.slip0044_coin_type
            };
            json!({
                "op": op,
                "name": account_name(rng),
                "pass": account_passphrase(rng),
                "xpriv": {"seed": "262728292a2b2c2d2e2f303132333435",
                          "path": ["44'", format!("{coin}'"), format!("{}'", rng.below(3))]},
            })
        }
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

/// How a generated watching-only script makes its wallet.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum WatchKind {
    /// Created from an account extended public key under the legacy coin
    /// type (`InitializeWatchOnly`).
    XpubLegacy,
    /// Created from an account extended public key under the SLIP0044 coin
    /// type.
    XpubSlip0044,
    /// Created from a seed on the legacy coin type, then converted
    /// (`ConvertToWatchingOnly`).
    ConvertedLegacy,
    /// Created from a seed, upgraded to the SLIP0044 coin type, then
    /// converted.
    ConvertedSlip0044,
}

/// The kind of a script [`generate_watching`] made, read back from it.
pub fn watch_kind(script: &Value) -> WatchKind {
    let ops = script["ops"].as_array().expect("ops");
    let upgraded = ops
        .first()
        .is_some_and(|o| o["op"] == "mgr.upgrade_to_slip0044_coin_type");
    match (&script["setup"]["xpub"], upgraded) {
        (Value::Null, false) => WatchKind::ConvertedLegacy,
        (Value::Null, true) => WatchKind::ConvertedSlip0044,
        (x, _) => {
            let net = script["setup"]["network"].as_str().expect("network");
            let chain = ChainParams::by_name(net).expect("network");
            if x["path"][1] == format!("{}'", chain.legacy_coin_type) {
                WatchKind::XpubLegacy
            } else {
                WatchKind::XpubSlip0044
            }
        }
    }
}

/// A random watching-only script: a wallet created from an account
/// extended public key (of either coin type's account path, from a random
/// seed), or a wallet created from a seed (on either coin type) that runs a
/// few operations, is converted, and runs more, with a second conversion
/// (a no-op) and reopens among them. Its operations are [`GEN_OPS`] and
/// [`WATCH_OPS`], drawn as [`generate`] draws them, with one exclusion:
/// after a conversion and before a reopen, `mgr.private_key` is drawn as
/// `mgr.have_private_key`, since Go's `PrivateKey` then answers from its
/// memory, not its store: for an imported key it dereferences the nil
/// private crypto key and panics (QK-0763), and for an account it had
/// cached it finds the sealed private key it emptied (QK-0764);
/// `private_keys_after_a_conversion_answer_from_the_store` in
/// `tests/oracle.rs` pins both.
pub fn generate_watching(rng: &mut SplitMix64, id: usize, shape: Shape) -> Value {
    let network = shape.network.unwrap_or_else(|| network(rng));
    let kind = match rng.below(4) {
        0 => WatchKind::XpubLegacy,
        1 => WatchKind::XpubSlip0044,
        2 => WatchKind::ConvertedLegacy,
        _ => WatchKind::ConvertedSlip0044,
    };
    generate_watching_as(rng, id, network, kind, shape)
}

/// [`generate_watching`] with the network and the kind given.
pub fn generate_watching_as(
    rng: &mut SplitMix64,
    id: usize,
    network: &str,
    kind: WatchKind,
    shape: Shape,
) -> Value {
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
    let xpub = match kind {
        WatchKind::XpubLegacy | WatchKind::XpubSlip0044 => {
            let coin = if kind == WatchKind::XpubLegacy {
                chain.legacy_coin_type
            } else {
                chain.slip0044_coin_type
            };
            Some(json!({"seed": hex(&seed),
                        "path": ["44'", format!("{coin}'"), format!("{}'", rng.below(3))]}))
        }
        _ => None,
    };
    if kind == WatchKind::ConvertedSlip0044 {
        ops.push(json!({"op": "mgr.upgrade_to_slip0044_coin_type"}));
    }
    // A converted wallet runs this many operations before its conversion.
    let convert_at = match kind {
        WatchKind::ConvertedLegacy | WatchKind::ConvertedSlip0044 => Some(rng.below(8)),
        _ => None,
    };
    let mut watching = xpub.is_some();
    // Converted in this process and not reopened since (QK-0763).
    let mut converted_live = false;
    let mut scrypt_ops = 0;
    let mut account0_recorded = false;
    let n = 8 + rng.below(24);
    for step in 0..n {
        if !watching && convert_at.is_some_and(|c| step >= c) {
            ops.push(json!({"op": "mgr.convert_to_watching_only"}));
            watching = true;
            converted_live = true;
            continue;
        }
        let all = GEN_OPS.len() + WATCH_OPS.len();
        let pick = usize::try_from(rng.below(all as u64)).expect("small");
        let mut op = GEN_OPS
            .get(pick)
            .copied()
            .unwrap_or_else(|| WATCH_OPS[pick - GEN_OPS.len()]);
        if op == "mgr.convert_to_watching_only" && !watching {
            // Conversions happen at `convert_at`; before it, this draw is
            // a harmless read instead.
            op = "mgr.watching_only";
        }
        // A watching-only wallet refuses an unlock before any scrypt; a
        // reopen derives the public master key on either kind.
        let scrypt = op == "rt.reopen" || (op == "mgr.unlock" && !watching);
        if costly && scrypt {
            if scrypt_ops >= shape.costly_scrypt_ops {
                op = "mgr.is_locked";
            } else {
                scrypt_ops += 1;
            }
        }
        if account0_recorded && op == "mgr.upgrade_to_slip0044_coin_type" {
            op = "mgr.coin_type";
        }
        if op == "mgr.private_key" && converted_live {
            op = "mgr.have_private_key";
        }
        let v = operation(rng, op, &chain, &privpass);
        if op == "rt.reopen" {
            converted_live = false;
        }
        if op == "mgr.record_derived_address"
            || (op == "mgr.sync_account_to_addr_index" && v["account"] == 0)
        {
            account0_recorded = true;
        }
        ops.push(v);
    }
    if !watching {
        ops.push(json!({"op": "mgr.convert_to_watching_only"}));
    }
    let mut setup = json!({
        "network": network,
        "pubpass": pubpass,
        "privpass": privpass,
        "rand": hex(&rng.bytes(32)),
        "clock": 1_794_990_000,
    });
    match xpub {
        Some(x) => {
            setup["xpub"] = x;
            setup["privpass"] = json!("");
        }
        None => setup["seed"] = json!(hex(&seed)),
    }
    json!({
        "format": "purse-opscript 1",
        "name": format!("generated watching-only {id}"),
        "setup": setup,
        "ops": ops,
    })
}

/// A random script of per-account passphrases and imported voting
/// accounts: a wallet created from a seed (three scripts in four; half of
/// them upgrade to the SLIP0044 coin type first) or from an account
/// extended public key (watching-only, where only voting accounts can be
/// unlocked), running [`PASS_OPS`] and [`GEN_OPS`], half of each. At most
/// [`ARGON2_OPS`] Argon2id operations are drawn (later ones become
/// `mgr.account_has_passphrase`), and no coin type upgrade follows a
/// `SetAccountPassphrase` of account 0 (it becomes `mgr.coin_type`): Go's
/// upgrade keeps account 0's Argon2id parameters over a key the wallet
/// passphrase seals, where dcr-purse's drops them (QK-0767;
/// `coin_type_upgrade_drops_account_0s_own_passphrase` in
/// `tests/oracle.rs`). No script converts a wallet: the conversions of
/// accounts with their own passphrase that diverge (QK-0765, QK-0766) are
/// pinned by `account_keys_after_a_conversion_answer_from_the_store`, and
/// corpus script `23-voting-convert` covers the rest.
pub fn generate_passphrase(rng: &mut SplitMix64, id: usize, shape: Shape) -> Value {
    let network = shape.network.unwrap_or_else(|| network(rng));
    let watching = rng.below(4) == 0;
    generate_passphrase_as(rng, id, network, watching, shape)
}

/// [`generate_passphrase`] with the network and the wallet's kind given.
pub fn generate_passphrase_as(
    rng: &mut SplitMix64,
    id: usize,
    network: &str,
    watching: bool,
    shape: Shape,
) -> Value {
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
    let xpub = if watching {
        let coin = if rng.below(2) == 0 {
            chain.legacy_coin_type
        } else {
            chain.slip0044_coin_type
        };
        Some(json!({"seed": hex(&seed),
                    "path": ["44'", format!("{coin}'"), format!("{}'", rng.below(3))]}))
    } else {
        if rng.below(2) == 0 {
            ops.push(json!({"op": "mgr.upgrade_to_slip0044_coin_type"}));
        }
        None
    };
    let mut scrypt_ops = 0;
    // Most seed wallets start unlocked, as a passphrase can be set only on
    // an unlocked account.
    if !watching && rng.below(3) != 0 {
        ops.push(json!({"op": "mgr.unlock", "pass": privpass}));
        if costly {
            scrypt_ops += 1;
        }
    }
    // The passphrases set or imported so far, with the accounts they were
    // given to (an import's guessed as the next voting account): an unlock
    // or lock mostly picks one of these pairs.
    let mut given: Vec<(Value, Value)> = Vec::new();
    let mut imports = 0u32;
    let mut argon2_ops = 0;
    let mut account0_recorded = false;
    let mut account0_passphrase = false;
    for _ in 0..8 + rng.below(24) {
        let mut op = if rng.below(2) == 0 {
            PASS_OPS[usize::try_from(rng.below(PASS_OPS.len() as u64)).expect("small")]
        } else {
            GEN_OPS[usize::try_from(rng.below(GEN_OPS.len() as u64)).expect("small")]
        };
        let scrypt = op == "rt.reopen" || (op == "mgr.unlock" && !watching);
        if costly && scrypt {
            if scrypt_ops >= shape.costly_scrypt_ops {
                op = "mgr.is_locked";
            } else {
                scrypt_ops += 1;
            }
        }
        if matches!(
            op,
            "mgr.set_account_passphrase" | "mgr.unlock_account" | "mgr.import_voting_account"
        ) {
            if argon2_ops >= ARGON2_OPS {
                op = "mgr.account_has_passphrase";
            } else {
                argon2_ops += 1;
            }
        }
        if (account0_recorded || account0_passphrase) && op == "mgr.upgrade_to_slip0044_coin_type" {
            op = "mgr.coin_type";
        }
        let mut v = operation(rng, op, &chain, &privpass);
        match op {
            "mgr.set_account_passphrase" => {
                given.push((v["account"].clone(), v["pass"].clone()));
            }
            "mgr.import_voting_account" => {
                given.push((json!(0x8000_0000_u32 + imports), v["pass"].clone()));
                imports += 1;
            }
            "mgr.unlock_account" | "mgr.lock_account" if !given.is_empty() && rng.below(4) != 0 => {
                let (account, pass) =
                    &given[usize::try_from(rng.below(given.len() as u64)).expect("small")];
                v["account"] = account.clone();
                if op == "mgr.unlock_account" {
                    v["pass"] = pass.clone();
                }
            }
            _ => {}
        }
        if op == "mgr.record_derived_address"
            || (op == "mgr.sync_account_to_addr_index" && v["account"] == 0)
        {
            account0_recorded = true;
        }
        if op == "mgr.set_account_passphrase" && v["account"] == 0 {
            account0_passphrase = true;
        }
        ops.push(v);
    }
    let mut setup = json!({
        "network": network,
        "pubpass": pubpass,
        "privpass": privpass,
        "rand": hex(&rng.bytes(32)),
        "clock": 1_794_990_000,
        "ncpu": 1 + rng.below(4),
    });
    match xpub {
        Some(x) => {
            setup["xpub"] = x;
            setup["privpass"] = json!("");
        }
        None => setup["seed"] = json!(hex(&seed)),
    }
    json!({
        "format": "purse-opscript 1",
        "name": format!("generated passphrase {id}"),
        "setup": setup,
        "ops": ops,
    })
}

/// How a generated passphrase-change script makes its wallet.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ChangeKind {
    /// Created from a seed (half of them upgrade to the SLIP0044 coin type
    /// first), with per-account passphrases among the operations.
    Seed,
    /// Created from an account extended public key of either coin type
    /// (watching-only: every private change is refused).
    Xpub,
    /// Created from a seed (either coin type), converted to watching-only
    /// after a few operations.
    Converted,
}

/// The kind of a script [`generate_change`] made, read back from it.
pub fn change_kind(script: &Value) -> ChangeKind {
    if !script["setup"]["xpub"].is_null() {
        return ChangeKind::Xpub;
    }
    let converts = script["ops"]
        .as_array()
        .expect("ops")
        .iter()
        .any(|o| o["op"] == "mgr.convert_to_watching_only");
    if converts {
        ChangeKind::Converted
    } else {
        ChangeKind::Seed
    }
}

/// The scrypt cost a generated passphrase-change script may spend on a
/// network other than simnet, beyond creation, opening and the export's
/// check, counted in derivations: a change derives the old passphrase's
/// key and the new one, an unlock of a locked wallet and a reopen one each.
fn change_budget(shape: Shape) -> usize {
    shape.costly_scrypt_ops * 2
}

/// A random script of passphrase changes: a wallet of one of the
/// [`ChangeKind`]s, running [`CHANGE_OPS`] and [`GEN_OPS`], half of each
/// (with, on a seed's wallet, per-account passphrases among the first half,
/// and on a wallet created from an account extended public key, public key
/// imports). A change names the current passphrase of its half three times
/// in four, and another passphrase otherwise; the generator follows the
/// passphrases as the changes set them, so later unlocks, reopens and the
/// export use the current ones. The exclusions of [`generate`],
/// [`generate_watching`] and [`generate_passphrase`] hold: no coin type
/// upgrade after an address of account 0 may have been recorded (QK-0760)
/// or after account 0 was given its own passphrase (QK-0767); no
/// `mgr.private_key` between a conversion and the next reopen (QK-0763,
/// QK-0764); no per-account passphrase on a wallet that is converted
/// (QK-0765, QK-0766).
pub fn generate_change(rng: &mut SplitMix64, id: usize, shape: Shape) -> Value {
    let network = shape.network.unwrap_or_else(|| network(rng));
    let kind = match rng.below(4) {
        0 | 1 => ChangeKind::Seed,
        2 => ChangeKind::Xpub,
        _ => ChangeKind::Converted,
    };
    generate_change_as(rng, id, network, kind, shape)
}

/// [`generate_change`] with the network and the kind given.
pub fn generate_change_as(
    rng: &mut SplitMix64,
    id: usize,
    network: &str,
    kind: ChangeKind,
    shape: Shape,
) -> Value {
    const SEED_EXTRA: &[&str] = &[
        "mgr.set_account_passphrase",
        "mgr.unlock_account",
        "mgr.lock_account",
    ];
    const XPUB_EXTRA: &[&str] = &["mgr.import_public_key"];
    let chain = ChainParams::by_name(network).expect("network");
    let costly = network != "simnet";
    let seed_len = 16 + usize::try_from(rng.below(49)).expect("small");
    let seed = rng.bytes(seed_len);
    let mut pubpass = passphrase(rng);
    let mut privpass = passphrase(rng);
    if privpass.is_empty() {
        privpass.push('x');
    }
    let setup_pubpass = pubpass.clone();
    let setup_privpass = privpass.clone();
    let mut ops = Vec::new();
    let xpub = if kind == ChangeKind::Xpub {
        let coin = if rng.below(2) == 0 {
            chain.legacy_coin_type
        } else {
            chain.slip0044_coin_type
        };
        Some(json!({"seed": hex(&seed),
                    "path": ["44'", format!("{coin}'"), format!("{}'", rng.below(3))]}))
    } else {
        if rng.below(2) == 0 {
            ops.push(json!({"op": "mgr.upgrade_to_slip0044_coin_type"}));
        }
        None
    };
    let mut watching = xpub.is_some();
    let mut scrypt_cost = 0;
    if !watching && rng.below(3) != 0 {
        ops.push(json!({"op": "mgr.unlock", "pass": privpass}));
        if costly {
            scrypt_cost += 1;
        }
    }
    let convert_at = (kind == ChangeKind::Converted).then(|| rng.below(10));
    let extra: &[&str] = match kind {
        ChangeKind::Seed => SEED_EXTRA,
        ChangeKind::Xpub => XPUB_EXTRA,
        ChangeKind::Converted => &[],
    };
    let mut converted_live = false;
    let mut argon2_ops = 0;
    let mut account0_recorded = false;
    let mut account0_passphrase = false;
    let mut given: Vec<(Value, Value)> = Vec::new();
    let n = 8 + rng.below(24);
    for step in 0..n {
        if !watching && convert_at.is_some_and(|c| step >= c) {
            ops.push(json!({"op": "mgr.convert_to_watching_only"}));
            watching = true;
            converted_live = true;
            continue;
        }
        let mut op = if rng.below(2) == 0 {
            let all = CHANGE_OPS.len() + extra.len();
            let pick = usize::try_from(rng.below(all as u64)).expect("small");
            CHANGE_OPS
                .get(pick)
                .copied()
                .unwrap_or_else(|| extra[pick - CHANGE_OPS.len()])
        } else {
            GEN_OPS[usize::try_from(rng.below(GEN_OPS.len() as u64)).expect("small")]
        };
        // A change's arguments are drawn before its cost is known: private
        // or public, the current passphrase or another, and the new one.
        let change = (op == "mgr.change_passphrase").then(|| {
            let private = rng.below(2) == 0;
            let current = if private { &privpass } else { &pubpass };
            let old = if rng.below(4) == 0 {
                passphrase(rng)
            } else {
                current.clone()
            };
            let new = if rng.below(10) == 0 {
                old.clone()
            } else {
                passphrase(rng)
            };
            (private, old, new)
        });
        let cost = match (op, &change) {
            ("rt.reopen", _) => 1,
            ("mgr.unlock", _) if !watching => 1,
            ("mgr.change_passphrase", Some((private, _, _))) if !(*private && watching) => 2,
            _ => 0,
        };
        if costly && cost > 0 {
            if scrypt_cost + cost > change_budget(shape) {
                op = "mgr.is_locked";
            } else {
                scrypt_cost += cost;
            }
        }
        if matches!(op, "mgr.set_account_passphrase" | "mgr.unlock_account") {
            if argon2_ops >= ARGON2_OPS / 2 {
                op = "mgr.account_has_passphrase";
            } else {
                argon2_ops += 1;
            }
        }
        if (account0_recorded || account0_passphrase) && op == "mgr.upgrade_to_slip0044_coin_type" {
            op = "mgr.coin_type";
        }
        if op == "mgr.private_key" && converted_live {
            op = "mgr.have_private_key";
        }
        let mut v = match (op, change) {
            ("mgr.change_passphrase", Some((private, old, new))) => {
                let current = if private { &mut privpass } else { &mut pubpass };
                if old == *current && !(private && watching) {
                    current.clone_from(&new);
                }
                json!({"op": op, "old": old, "new": new, "private": private})
            }
            _ => operation(rng, op, &chain, &privpass),
        };
        match op {
            "mgr.set_account_passphrase" => {
                given.push((v["account"].clone(), v["pass"].clone()));
            }
            "mgr.unlock_account" | "mgr.lock_account" if !given.is_empty() && rng.below(4) != 0 => {
                let (account, pass) =
                    &given[usize::try_from(rng.below(given.len() as u64)).expect("small")];
                v["account"] = account.clone();
                if op == "mgr.unlock_account" {
                    v["pass"] = pass.clone();
                }
            }
            "rt.reopen" => converted_live = false,
            _ => {}
        }
        if op == "mgr.record_derived_address"
            || (op == "mgr.sync_account_to_addr_index" && v["account"] == 0)
        {
            account0_recorded = true;
        }
        if op == "mgr.set_account_passphrase" && v["account"] == 0 {
            account0_passphrase = true;
        }
        ops.push(v);
    }
    if !watching && kind == ChangeKind::Converted {
        ops.push(json!({"op": "mgr.convert_to_watching_only"}));
    }
    let mut setup = json!({
        "network": network,
        "pubpass": setup_pubpass,
        "privpass": setup_privpass,
        "rand": hex(&rng.bytes(32)),
        "clock": 1_794_990_000,
        "ncpu": 1 + rng.below(4),
    });
    match xpub {
        Some(x) => {
            setup["xpub"] = x;
            setup["privpass"] = json!("");
        }
        None => setup["seed"] = json!(hex(&seed)),
    }
    json!({
        "format": "purse-opscript 1",
        "name": format!("generated change {id}"),
        "setup": setup,
        "ops": ops,
    })
}
