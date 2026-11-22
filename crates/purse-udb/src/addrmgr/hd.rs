// Copyright (c) 2014 The btcsuite developers
// Copyright (c) 2015-2025 The Decred developers
// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! The BIP0044 key hierarchy as udb derives it: `HDKeysFromSeed`,
//! `deriveCoinTypeKey`, `deriveAccountKey`, `checkBranchKeys`
//! (`wallet/udb/addressmanager.go:2664-2882`), and the chain parameters
//! udb reads.

use dcroxide_chaincfg::Params;
use dcroxide_txscript::stdaddr::AddressParamsV0;
use dcroxide_wire::CurrencyNet;
use purse_errors::{AnyError, Kind, e};
use purse_hdkeychain::{ExtendedKey, HARDENED_KEY_START, HdVersions};

use super::{EXTERNAL_BRANCH, INTERNAL_BRANCH, MAX_ACCOUNT_NUM, MAX_COIN_TYPE};

/// What udb reads of a network's `*chaincfg.Params`: its name, the
/// extended-key version bytes, the address and WIF prefixes, both coin
/// types and whether it is simnet (which selects the cheap scrypt cost).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ChainParams {
    /// `Params.Name` (`mainnet`, `testnet3`, `simnet`, `regnet`).
    pub name: &'static str,
    /// The extended private and public key version bytes.
    pub hd: HdVersions,
    /// The version 0 address prefixes and the WIF prefix.
    pub ids: AddrIds,
    /// `Params.LegacyCoinType`.
    pub legacy_coin_type: u32,
    /// `Params.SLIP0044CoinType`.
    pub slip0044_coin_type: u32,
    /// `Params.Net == wire.SimNet`.
    pub simnet: bool,
}

impl From<&Params> for ChainParams {
    fn from(p: &Params) -> Self {
        ChainParams {
            name: p.name,
            hd: HdVersions::from(p),
            ids: AddrIds {
                pub_key: p.pub_key_addr_id,
                pub_key_hash_ecdsa: p.pub_key_hash_addr_id,
                pub_key_hash_ed25519: p.pkh_edwards_addr_id,
                pub_key_hash_schnorr: p.pkh_schnorr_addr_id,
                script_hash: p.script_hash_addr_id,
                private_key: p.private_key_id,
            },
            legacy_coin_type: p.legacy_coin_type,
            slip0044_coin_type: p.slip0044_coin_type,
            simnet: p.net == CurrencyNet::SIM_NET,
        }
    }
}

/// A network's version 0 address prefixes (`Params.PubKeyAddrID`,
/// `PubKeyHashAddrID`, `PKHEdwardsAddrID`, `PKHSchnorrAddrID`,
/// `ScriptHashAddrID`) and its WIF prefix (`Params.PrivateKeyID`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct AddrIds {
    /// Pay-to-pubkey addresses.
    pub pub_key: [u8; 2],
    /// Pay-to-pubkey-hash ECDSA secp256k1 addresses.
    pub pub_key_hash_ecdsa: [u8; 2],
    /// Pay-to-pubkey-hash Ed25519 addresses.
    pub pub_key_hash_ed25519: [u8; 2],
    /// Pay-to-pubkey-hash Schnorr secp256k1 addresses.
    pub pub_key_hash_schnorr: [u8; 2],
    /// Pay-to-script-hash addresses.
    pub script_hash: [u8; 2],
    /// WIF private keys.
    pub private_key: [u8; 2],
}

impl AddressParamsV0 for ChainParams {
    fn addr_id_pub_key_v0(&self) -> [u8; 2] {
        self.ids.pub_key
    }
    fn addr_id_pub_key_hash_ecdsa_v0(&self) -> [u8; 2] {
        self.ids.pub_key_hash_ecdsa
    }
    fn addr_id_pub_key_hash_ed25519_v0(&self) -> [u8; 2] {
        self.ids.pub_key_hash_ed25519
    }
    fn addr_id_pub_key_hash_schnorr_v0(&self) -> [u8; 2] {
        self.ids.pub_key_hash_schnorr
    }
    fn addr_id_script_hash_v0(&self) -> [u8; 2] {
        self.ids.script_hash
    }
}

impl ChainParams {
    /// The parameters of the network named as Go's `Params.Name` names it
    /// (`mainnet`, `testnet3`, `simnet`, `regnet`), or `None`.
    pub fn by_name(name: &str) -> Option<ChainParams> {
        let p = match name {
            "mainnet" => dcroxide_chaincfg::mainnet_params(),
            "testnet3" => dcroxide_chaincfg::testnet3_params(),
            "simnet" => dcroxide_chaincfg::simnet_params(),
            "regnet" => dcroxide_chaincfg::regnet_params(),
            _ => return None,
        };
        Some(ChainParams::from(&p))
    }

    /// The scrypt cost of new master keys on this network (Go's
    /// `scryptOptionsForNet`, `wallet/udb/addressmanager.go:119-129`):
    /// `N = 2, r = 1, p = 1` on simnet, `N = 262144, r = 8, p = 1`
    /// everywhere else.
    pub fn scrypt_cost(&self) -> (i64, i64, i64) {
        if self.simnet {
            (2, 1, 1)
        } else {
            (262_144, 8, 1)
        }
    }
}

/// Go's `CoinTypes`: the legacy and SLIP0044 coin types of a network.
pub fn coin_types(params: &ChainParams) -> (u32, u32) {
    (params.legacy_coin_type, params.slip0044_coin_type)
}

/// The four keys of [`hd_keys_from_seed`], all private, wiped on drop.
#[derive(Clone, Debug)]
pub struct HdKeys {
    /// `m/44'/<legacy coin type>'`.
    pub coin_type_legacy: ExtendedKey,
    /// `m/44'/<SLIP0044 coin type>'`.
    pub coin_type_slip0044: ExtendedKey,
    /// `m/44'/<legacy coin type>'/0'`.
    pub account0_legacy: ExtendedKey,
    /// `m/44'/<SLIP0044 coin type>'/0'`.
    pub account0_slip0044: ExtendedKey,
}

/// An hdkeychain error as udb returns it: unwrapped, with Go's text.
pub(crate) fn hd_err(err: purse_hdkeychain::Error) -> AnyError {
    AnyError::foreign(err)
}

/// `errors.E(errors.Seed, hdkeychain.ErrUnusableSeed)`: `invalid seed:
/// unusable seed`.
pub(crate) fn unusable_seed() -> AnyError {
    e!(
        Kind::Seed,
        AnyError::foreign(purse_hdkeychain::Error::UnusableSeed)
    )
    .into()
}

/// Go's `deriveCoinTypeKey` (`wallet/udb/addressmanager.go:2664-2689`):
/// `m/44'/<coin type>'` from a master key. Both steps use `Child`, so the
/// coin type step lays out its preimage from the purpose key as it was
/// stored, stripped of leading zero bytes when it has them.
///
/// # Errors
///
/// A coin type above [`MAX_COIN_TYPE`]: `Invalid` `coin type <n>`.
/// Otherwise hdkeychain's error, unwrapped.
pub fn derive_coin_type_key(master: &ExtendedKey, coin_type: u32) -> Result<ExtendedKey, AnyError> {
    if coin_type > MAX_COIN_TYPE {
        return Err(e!(Kind::Invalid, format!("coin type {coin_type}")).into());
    }
    let purpose = master.child(44 | HARDENED_KEY_START).map_err(hd_err)?;
    purpose
        .child(coin_type | HARDENED_KEY_START)
        .map_err(hd_err)
}

/// Go's `deriveAccountKey` (`wallet/udb/addressmanager.go:2698-2706`):
/// `<coin type key>/<account>'`.
///
/// # Errors
///
/// An account above [`MAX_ACCOUNT_NUM`]: `Invalid` `account <n>`.
/// Otherwise hdkeychain's error, unwrapped.
pub fn derive_account_key(
    coin_type_key: &ExtendedKey,
    account: u32,
) -> Result<ExtendedKey, AnyError> {
    if account > MAX_ACCOUNT_NUM {
        return Err(e!(Kind::Invalid, format!("account {account}")).into());
    }
    coin_type_key
        .child(account | HARDENED_KEY_START)
        .map_err(hd_err)
}

/// Go's `checkBranchKeys` (`wallet/udb/addressmanager.go:2719-2728`):
/// derives both branch keys of an account key and discards them.
pub(crate) fn check_branch_keys(account_key: &ExtendedKey) -> Result<(), purse_hdkeychain::Error> {
    account_key.child(EXTERNAL_BRANCH)?;
    account_key.child(INTERNAL_BRANCH)?;
    Ok(())
}

/// Go's `HDKeysFromSeed` (`wallet/udb/addressmanager.go:2807-2882`): both
/// coin type keys and both account 0 keys of a seed.
///
/// The purpose key is derived twice, once per coin type, and the account
/// keys are derived from the coin type keys as `Child` left them, so a coin
/// type key with leading zero bytes gives account 0 a different key than
/// plain BIP32 would (spec 02 section 5.1). Every intermediate key is wiped
/// when dropped, where Go leaves the master and purpose keys in memory.
///
/// # Errors
///
/// In Go's order: hdkeychain's `ErrInvalidSeedLen` or `ErrUnusableSeed`
/// from the master key, unwrapped; a coin type step's error, unwrapped; an
/// invalid account 0 or branch child: `Seed` wrapping `ErrUnusableSeed`
/// (`invalid seed: unusable seed`); any other error of those steps,
/// unwrapped.
pub fn hd_keys_from_seed(seed: &[u8], params: &ChainParams) -> Result<HdKeys, AnyError> {
    let root = ExtendedKey::new_master(seed, &params.hd).map_err(hd_err)?;
    let (legacy, slip0044) = coin_types(params);
    let coin_type_legacy = derive_coin_type_key(&root, legacy)?;
    let coin_type_slip0044 = derive_coin_type_key(&root, slip0044)?;

    // An invalid child anywhere below the coin type keys makes the seed
    // unusable; any other error passes through.
    let unusable = |err: AnyError| -> AnyError {
        if err.find_foreign::<purse_hdkeychain::Error>()
            == Some(&purse_hdkeychain::Error::InvalidChild)
            && err.as_kind().is_none()
        {
            unusable_seed()
        } else {
            err
        }
    };
    let account0_legacy = derive_account_key(&coin_type_legacy, 0).map_err(unusable)?;
    let account0_slip0044 = derive_account_key(&coin_type_slip0044, 0).map_err(unusable)?;
    for key in [&account0_legacy, &account0_slip0044] {
        check_branch_keys(key).map_err(|err| match err {
            purse_hdkeychain::Error::InvalidChild => unusable_seed(),
            other => hd_err(other),
        })?;
    }
    Ok(HdKeys {
        coin_type_legacy,
        coin_type_slip0044,
        account0_legacy,
        account0_slip0044,
    })
}
