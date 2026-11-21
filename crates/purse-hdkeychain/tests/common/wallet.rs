// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! The wallet-level derivation the oracle's `wallet` command reports
//! (`tools/oracle/cmd/hdkeychain/wallet.go`), built from this crate and
//! dcroxide-txscript's `stdaddr` the way dcrwallet composes hdkeychain and
//! stdaddr:
//!
//! - `udb.HDKeysFromSeed`: the master key, `m/44'/<coin>'` for the legacy
//!   and the SLIP0044 coin type with `child`, account 0 of each from the
//!   coin type key as derived (possibly stored stripped), and the check
//!   that both branches of both accounts derive;
//! - `udb.Manager.NewAccount` (reached through `Wallet.NextAccount`):
//!   account 1 from the coin type key re-parsed from its string (padded back
//!   to 32 bytes);
//! - the account's extended public key as stored (the neutered string),
//!   re-parsed, its branches 0 and 1, and child 0 of each as a P2PKH
//!   address (`deriveBranches`, `deriveChildAddress`, `compat.HD2Address`).
//!
//! One line per item, in this order for the legacy then the SLIP0044 coin
//! type:
//!
//! ```text
//! C|<coin type>|<coin type key neutered>
//! A|<coin type>|<account>|<account xpub>|<branch 0 address 0>|<branch 1 address 0>
//! E|<coin type>|<account>|<error>
//! ```
//!
//! or the single line `E|<error>` when `HDKeysFromSeed` fails.
//!
//! These tests compose the steps themselves: no wallet crate existed when
//! they were written. `purse-udb` derives accounts and addresses since
//! 2026-11-20 (`addrmgr::hd_keys_from_seed`, `Manager::new_account`,
//! `Manager::account_extended_pub_key`), and these tests should call it
//! instead; that switch is carried to Phase 3 (`docs/PHASE1-EXIT.md`,
//! section 1, the dated note under "Not covered").

use dcroxide_chaincfg::Params;
use dcroxide_txscript::stdaddr;
use purse_errors::{AnyError, DAEMON_SEPARATOR, Kind, e};
use purse_hdkeychain::{Error, ExtendedKey, HdVersions};
use purse_testutil::serde_json::{self, json};
use purse_testutil::{Oracle, hex};
use sha2::{Digest, Sha256};

use super::{h, par_map};

/// A network as the wallet sees it: its name for the oracle, its chain
/// parameters (coin types, address prefixes) and its HD versions.
pub struct WalletNet {
    pub name: &'static str,
    pub params: Params,
    pub versions: HdVersions,
}

/// The four Decred networks, in the order of [`super::NETS`].
pub fn wallet_nets() -> Vec<WalletNet> {
    [
        ("mainnet", dcroxide_chaincfg::mainnet_params()),
        ("testnet3", dcroxide_chaincfg::testnet3_params()),
        ("simnet", dcroxide_chaincfg::simnet_params()),
        ("regnet", dcroxide_chaincfg::regnet_params()),
    ]
    .into_iter()
    .map(|(name, params)| {
        let versions = HdVersions {
            private: params.hd_private_key_id,
            public: params.hd_public_key_id,
        };
        WalletNet {
            name,
            params,
            versions,
        }
    })
    .collect()
}

/// The accounts derived for each coin type: 0 as wallet creation derives
/// it, 1 as `udb.Manager.NewAccount` does.
pub const WALLET_ACCOUNTS: [u32; 2] = [0, 1];

/// `HDKeysFromSeed`'s wrapping of an invalid child below the coin type
/// keys: `errors.E(errors.Seed, hdkeychain.ErrUnusableSeed)`.
fn unusable(err: Error) -> String {
    if err == Error::InvalidChild {
        e!(Kind::Seed, AnyError::new(Error::UnusableSeed.to_string())).render(DAEMON_SEPARATOR)
    } else {
        err.to_string()
    }
}

/// `udb.HDKeysFromSeed`: (coin type, coin type key, account 0 key) for the
/// legacy then the SLIP0044 coin type.
fn hd_keys_from_seed(
    seed: &[u8],
    net: &WalletNet,
) -> Result<[(u32, ExtendedKey, ExtendedKey); 2], String> {
    let root = ExtendedKey::new_master(seed, &net.versions).map_err(|e| e.to_string())?;
    let coin = |coin_type: u32| {
        root.child(h(44))
            .and_then(|p| p.child(h(coin_type)))
            .map_err(|e| e.to_string())
    };
    let (legacy, slip0044) = (net.params.legacy_coin_type, net.params.slip0044_coin_type);
    let ct_legacy = coin(legacy)?;
    let ct_slip0044 = coin(slip0044)?;
    let acct_legacy = ct_legacy.child(h(0)).map_err(unusable)?;
    let acct_slip0044 = ct_slip0044.child(h(0)).map_err(unusable)?;
    for acct in [&acct_legacy, &acct_slip0044] {
        acct.child(0).map_err(unusable)?;
        acct.child(1).map_err(unusable)?;
    }
    Ok([
        (legacy, ct_legacy, acct_legacy),
        (slip0044, ct_slip0044, acct_slip0044),
    ])
}

/// The P2PKH address of a key (`compat.HD2Address`).
fn address(k: &ExtendedKey, params: &Params) -> Result<String, String> {
    let hash = stdaddr::hash160(k.serialized_pub_key());
    stdaddr::new_address_pub_key_hash_ecdsa_secp256k1_v0(&hash, params)
        .map(|a| a.encode())
        .map_err(|e| e.to_string())
}

/// The `A|` line of an account (or its `E|` line).
fn account_line(coin_type: u32, account: u32, acct: &ExtendedKey, net: &WalletNet) -> String {
    let derive = || -> Result<String, String> {
        let xpub_string = acct.neuter().string();
        let xpub = ExtendedKey::new_key_from_string(&xpub_string, &net.versions)
            .map_err(|e| e.to_string())?;
        let mut addrs = Vec::with_capacity(2);
        for branch in [0, 1] {
            let branch_key = xpub.child(branch).map_err(|e| e.to_string())?;
            let child = branch_key.child(0).map_err(|e| e.to_string())?;
            addrs.push(address(&child, &net.params)?);
        }
        Ok(format!(
            "A|{coin_type}|{account}|{}|{}|{}",
            *xpub_string, addrs[0], addrs[1]
        ))
    };
    derive().unwrap_or_else(|err| format!("E|{coin_type}|{account}|{err}"))
}

/// The lines of one seed.
pub fn wallet_lines(seed: &[u8], net: &WalletNet) -> Vec<String> {
    let keys = match hd_keys_from_seed(seed, net) {
        Ok(k) => k,
        Err(err) => return vec![format!("E|{err}")],
    };
    let mut lines = Vec::with_capacity(6);
    for (coin_type, ct, acct0) in &keys {
        lines.push(format!("C|{coin_type}|{}", *ct.neuter().string()));
        for account in WALLET_ACCOUNTS {
            if account == 0 {
                lines.push(account_line(*coin_type, account, acct0, net));
                continue;
            }
            let acct = ExtendedKey::new_key_from_string(&ct.string(), &net.versions)
                .and_then(|reparsed| reparsed.child(h(account)));
            match acct {
                Ok(a) => lines.push(account_line(*coin_type, account, &a, net)),
                Err(err) => lines.push(format!("E|{coin_type}|{account}|{err}")),
            }
        }
    }
    lines
}

/// The digest of one seed's lines, as the oracle computes it.
pub fn wallet_digest(lines: &[String]) -> String {
    let mut hasher = Sha256::new();
    for l in lines {
        hasher.update(l.as_bytes());
        hasher.update(b"\n");
    }
    hex(&hasher.finalize())
}

/// What a wallet comparison matched.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WalletCompared {
    /// Seeds whose answers were compared and equal.
    pub seeds: usize,
    /// Account lines (`A|`) among them: account xpubs, each with the first
    /// address of both branches.
    pub accounts: usize,
    /// Error lines among them.
    pub errors: usize,
}

impl WalletCompared {
    fn add(&mut self, lines: &[String]) {
        self.seeds += 1;
        self.accounts += lines.iter().filter(|l| l.starts_with("A|")).count();
        self.errors += lines.iter().filter(|l| l.starts_with("E|")).count();
    }

    pub fn merge(&mut self, other: WalletCompared) {
        self.seeds += other.seeds;
        self.accounts += other.accounts;
        self.errors += other.errors;
    }
}

fn go_wallet(
    oracle: &mut Oracle,
    net: &WalletNet,
    seeds: &[Vec<u8>],
    digest: bool,
) -> serde_json::Value {
    let seeds: Vec<String> = seeds.iter().map(|s| hex(s)).collect();
    oracle.call(
        "wallet",
        json!({ "net": net.name, "digest": digest, "seeds": seeds }),
    )
}

/// Go's lines for each seed.
pub fn go_wallet_lines(
    oracle: &mut Oracle,
    net: &WalletNet,
    seeds: &[Vec<u8>],
) -> Vec<Vec<String>> {
    serde_json::from_value(go_wallet(oracle, net, seeds, false)).expect("wallet lines")
}

fn first_difference(go: &[String], rust: &[String], seed: &[u8]) -> String {
    for i in 0..go.len().max(rust.len()) {
        let g = go.get(i).map_or("<none>", String::as_str);
        let r = rust.get(i).map_or("<none>", String::as_str);
        if g != r {
            return format!("seed {} line {i}:\n  go:   {g}\n  rust: {r}", hex(seed));
        }
    }
    "no difference".to_owned()
}

/// Compares seeds with Go line by line; panics with the first difference.
pub fn compare_wallet_lines(
    oracle: &mut Oracle,
    net: &WalletNet,
    seeds: &[Vec<u8>],
) -> WalletCompared {
    let go = go_wallet_lines(oracle, net, seeds);
    assert_eq!(
        go.len(),
        seeds.len(),
        "Go answered a different number of seeds"
    );
    let mut compared = WalletCompared::default();
    for (g, seed) in go.iter().zip(seeds) {
        let r = wallet_lines(seed, net);
        assert!(
            *g == r,
            "mismatch on {}: {}",
            net.name,
            first_difference(g, &r, seed)
        );
        compared.add(&r);
    }
    compared
}

/// Compares seeds with Go by digest, in batches, Go and Rust working on a
/// batch at once; re-asks Go for full lines to report a mismatch.
pub fn compare_wallet_digests(
    oracle: &mut Oracle,
    net: &WalletNet,
    seeds: &[Vec<u8>],
    batch: usize,
) -> WalletCompared {
    let mut compared = WalletCompared::default();
    for chunk in seeds.chunks(batch.max(1)) {
        let (go, rust) = std::thread::scope(|s| {
            let go = s.spawn(|| go_wallet(oracle, net, chunk, true));
            let rust = par_map(chunk, |seed| {
                let lines = wallet_lines(seed, net);
                (wallet_digest(&lines), lines)
            });
            (go.join().expect("oracle thread"), rust)
        });
        let go: Vec<String> = serde_json::from_value(go).expect("wallet digests");
        assert_eq!(
            go.len(),
            chunk.len(),
            "Go answered a different number of seeds"
        );
        for ((g, (r, lines)), seed) in go.iter().zip(&rust).zip(chunk) {
            if g != r {
                let gl = go_wallet_lines(oracle, net, std::slice::from_ref(seed));
                panic!(
                    "digest mismatch on {}: {}",
                    net.name,
                    first_difference(&gl[0], lines, seed)
                );
            }
            compared.add(lines);
        }
    }
    compared
}

// ---------------------------------------------------------------------------
// Leading-zero search along dcrwallet's own routes.
// ---------------------------------------------------------------------------

mod filter {
    use dcroxide_dcrec::secp256k1;
    use hmac::{Hmac, KeyInit, Mac};
    use sha2::Sha512;

    fn hmac512(key: &[u8], data: &[u8]) -> [u8; 64] {
        let mut mac = <Hmac<Sha512> as KeyInit>::new_from_slice(key).expect("any key length");
        mac.update(data);
        mac.finalize().into_bytes().into()
    }

    /// (32-byte key, chain code) of the master, or None if unusable.
    pub fn master(seed: &[u8]) -> Option<([u8; 32], [u8; 32])> {
        let lr = hmac512(b"Bitcoin seed", seed);
        let (k, c) = lr.split_at(32);
        let k: [u8; 32] = k.try_into().ok()?;
        secp256k1::check_scalar(&k).ok()?;
        Some((k, c.try_into().ok()?))
    }

    /// Hardened child of a parent stored as `stored` (Decred's preimage:
    /// the stored bytes left-aligned after the 0x00 byte): (32-byte child
    /// key, chain code).
    pub fn hardened(stored: &[u8], cc: &[u8; 32], i: u32) -> Option<([u8; 32], [u8; 32])> {
        let mut data = [0u8; 37];
        data[1..1 + stored.len()].copy_from_slice(stored);
        data[33..].copy_from_slice(&i.to_be_bytes());
        let ilr = hmac512(cc, &data);
        let (il, ir) = ilr.split_at(32);
        let il: [u8; 32] = il.try_into().ok()?;
        let sum = *secp256k1::add_private_tweak(&il, stored).ok()?.serialize();
        Some((sum, ir.try_into().ok()?))
    }
}

fn stripped(b: &[u8; 32]) -> &[u8] {
    &b[super::leading_zeros(b)..]
}

/// The most leading zero bytes of a private key at each hardened depth
/// (master, purpose, coin type, account) on the routes dcrwallet takes for
/// mainnet's coin types 20 and 42: purpose and coin type keys by `Child`
/// (stored stripped), account 0 from the stripped coin type key, account 1
/// from the coin type key padded by a re-parse. A fast filter (HMAC-SHA512
/// and a scalar addition per step, no point multiplication); it selects
/// candidates only, and every candidate is compared with Go.
pub fn screen_wallet(seed: &[u8]) -> [usize; 4] {
    let lz = super::leading_zeros;
    let mut best = [0usize; 4];
    let Some((mk, mc)) = filter::master(seed) else {
        return best;
    };
    best[0] = lz(&mk);
    let Some((pk, pc)) = filter::hardened(&mk, &mc, h(44)) else {
        return best;
    };
    best[1] = lz(&pk);
    for coin in [20, 42] {
        let Some((ck, cc)) = filter::hardened(stripped(&pk), &pc, h(coin)) else {
            continue;
        };
        best[2] = best[2].max(lz(&ck));
        for (acct, stored) in [(0, stripped(&ck)), (1, &ck[..])] {
            if let Some((ak, _)) = filter::hardened(stored, &cc, h(acct)) {
                best[3] = best[3].max(lz(&ak));
            }
        }
    }
    best
}

/// Runs [`screen_wallet`] over `workers` deterministic streams of
/// `per_worker` 32-byte seeds and returns every seed with a leading zero
/// byte on some key, with its counts.
pub fn search_wallet(base: u64, workers: u64, per_worker: u64) -> Vec<(Vec<u8>, [usize; 4])> {
    let ws: Vec<u64> = (0..workers).collect();
    let found = par_map(&ws, |&w| {
        let mut rng = purse_testutil::SplitMix64(base ^ w.wrapping_mul(0x9e37_79b9_7f4a_7c15));
        let mut out = Vec::new();
        let mut seed = [0u8; 32];
        for _ in 0..per_worker {
            rng.fill(&mut seed);
            let z = screen_wallet(&seed);
            if z.iter().any(|&n| n > 0) {
                out.push((seed.to_vec(), z));
            }
        }
        out
    });
    found.into_iter().flatten().collect()
}
