// SPDX-License-Identifier: ISC
//! Extended key fuzz target: `new_key_from_string` on all four networks, and
//! derivation from whatever parses.
//!
//! These strings come from users and RPC clients (xpub import, account
//! keys).  Nothing may panic.  A key that parses prints as the string it
//! came from, and its children obey the package's own rules: a hardened
//! child of a public key is refused, a neutered key is public, and public
//! derivation from the neutered key equals neutering the private child.
//!
//! The first byte picks the mode.  Raw text exercises the length, base58 and
//! checksum layers, which mutation cannot get past; the other mode derives a
//! real key from the input as a seed and a short path, so every input
//! reaches the derivation code and the string round trip.

#![no_main]

use libfuzzer_sys::fuzz_target;
use purse_hdkeychain::{ExtendedKey, HARDENED_KEY_START, HdVersions};

const NETS: [HdVersions; 4] = [
    HdVersions::MAINNET,
    HdVersions::TESTNET3,
    HdVersions::SIMNET,
    HdVersions::REGNET,
];

fn check_key(key: &ExtendedKey, net: &HdVersions, index: u32) {
    let text = key.string();
    let reparsed = ExtendedKey::new_key_from_string(&text, net).expect("own string parses");
    assert_eq!(*reparsed.string(), *text);
    assert_eq!(reparsed.is_private(), key.is_private());

    let public = key.neuter();
    assert!(!public.is_private());
    assert!(public.child(index | HARDENED_KEY_START).is_err());

    let soft = index & !HARDENED_KEY_START;
    for std in [false, true] {
        let derive = |k: &ExtendedKey| if std { k.child_bip32_std(soft) } else { k.child(soft) };
        match (derive(key), derive(&public)) {
            (Ok(a), Ok(b)) => assert_eq!(*a.neuter().string(), *b.string()),
            (Err(_), Err(_)) => {}
            (a, b) => panic!("private {:?} but public {:?}", a.is_ok(), b.is_ok()),
        }
    }
}

fuzz_target!(|data: &[u8]| {
    let Some((&mode, rest)) = data.split_first() else {
        return;
    };
    let net = &NETS[usize::from(mode >> 1) % NETS.len()];
    if mode & 1 == 0 {
        // A real key: the first 32 bytes are the seed, the rest a path.
        let Some((seed, path)) = rest.split_at_checked(32) else {
            return;
        };
        let Ok(mut key) = ExtendedKey::new_master(seed, net) else {
            return;
        };
        for step in path.chunks_exact(4).take(8) {
            let index = u32::from_le_bytes([step[0], step[1], step[2], step[3]]);
            check_key(&key, net, index);
            match key.child(index) {
                Ok(child) => key = child,
                Err(_) => return,
            }
        }
        check_key(&key, net, 0);
        return;
    }
    let Ok(text) = std::str::from_utf8(rest) else {
        return;
    };
    for net in &NETS {
        if let Ok(key) = ExtendedKey::new_key_from_string(text, net) {
            assert_eq!(*key.string(), *text);
            check_key(&key, net, u32::from(mode));
        }
    }
});
