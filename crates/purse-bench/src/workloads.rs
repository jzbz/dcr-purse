// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! The workloads, in the order, with the names and on the inputs of the Go
//! side (`tools/oracle/cmd/bench`). Each group draws its inputs from
//! SplitMix64 with a seed of its own, so the two sides build them alike;
//! the `CHECK` lines prove it.

use std::io::Write;

use dcroxide_chaincfg::Params;
use dcroxide_txscript::stdaddr::{self, Address};
use hmac::{Hmac, KeyInit, Mac};
use purse_dcrutil::Amount;
use purse_hdkeychain::{ExtendedKey, HARDENED_KEY_START, HdVersions};
use purse_snacl::{Argon2idParams, CryptoKey, Limits, RandomSourceError, SecretKey};
use purse_testutil::{SplitMix64, hex};
use sha2::{Digest, Sha256, Sha512};

use crate::harness::Harness;

/// The number of addresses of the 20,000-address workload (200 in smoke
/// mode).
pub fn address_count(smoke: bool) -> u32 {
    if smoke { 200 } else { 20_000 }
}

/// scrypt's `N` for the master-key derivation: snacl's default, 16384
/// (1024 in smoke mode).
pub fn snacl_n(smoke: bool) -> i64 {
    if smoke { 1024 } else { purse_snacl::DEFAULT_N }
}

/// Argon2id's memory in KiB: dcrwallet's 256 MiB (1 MiB in smoke mode).
pub fn argon2_memory(smoke: bool) -> u32 {
    if smoke {
        1024
    } else {
        Argon2idParams::DEFAULT_MEMORY
    }
}

/// The solver's degrees: a small mix, a 64-peer mix and ADR-0010's gate
/// size (4, 8 and 32 in smoke mode).
pub fn mix_degrees(smoke: bool) -> [usize; 3] {
    if smoke { [4, 8, 32] } else { [20, 64, 512] }
}

/// Runs every selected workload.
pub fn run_all<W: Write>(h: &mut Harness<W>) {
    hd(h);
    primitives(h);
    snacl(h);
    kdf(h);
    amounts(h);
    mix(h);
}

/// SHA-256 over the lines, each followed by a newline, in hex.
pub fn sha256_lines<S: AsRef<str>>(lines: &[S]) -> String {
    let mut hasher = Sha256::new();
    for l in lines {
        hasher.update(l.as_ref().as_bytes());
        hasher.update(b"\n");
    }
    hex(&hasher.finalize())
}

fn h(i: u32) -> u32 {
    HARDENED_KEY_START | i
}

fn p2pkh(pub_key: &[u8], params: &Params) -> Address {
    stdaddr::new_address_pub_key_hash_ecdsa_secp256k1_v0(&stdaddr::hash160(pub_key), params)
        .expect("a 20-byte hash")
}

/// A random source that serves fixed bytes, for the values the Go side
/// builds with a fixed nonce or salt.
fn fixed(bytes: &[u8]) -> impl FnMut(&mut [u8]) -> Result<(), RandomSourceError> + '_ {
    move |buf: &mut [u8]| {
        buf.copy_from_slice(&bytes[..buf.len()]);
        Ok(())
    }
}

fn hd<W: Write>(bench: &mut Harness<W>) {
    let params = dcroxide_chaincfg::mainnet_params();
    let versions = HdVersions {
        private: params.hd_private_key_id,
        public: params.hd_public_key_id,
    };
    let seed: Vec<u8> = (0..32u8).collect();
    let master = ExtendedKey::new_master(&seed, &versions).expect("master");
    let acct = [h(44), h(42), h(0)]
        .into_iter()
        .try_fold(master.clone(), |k, i| k.child(i))
        .expect("account");
    let acct_pub = acct.neuter();
    let acct_pub_string = acct_pub.string();
    let branch_pub = acct_pub.child(0).expect("branch");
    let branch_priv = acct.child(0).expect("branch");

    bench.check("hd.master_pub", hex(master.serialized_pub_key()));
    bench.check("hd.acct_xpub", &*acct_pub_string);
    let h7 = acct.child(h(7)).expect("child");
    bench.check("hd.hardened7_pub", hex(h7.serialized_pub_key()));
    let p7 = branch_priv.child(7).expect("child");
    bench.check("hd.priv7_pub", hex(p7.serialized_pub_key()));
    let c7 = branch_pub.child(7).expect("child");
    bench.check("hd.pub7_pub", hex(c7.serialized_pub_key()));
    bench.check(
        "hd.pub7_script",
        hex(&p2pkh(c7.serialized_pub_key(), &params).payment_script().1),
    );

    let mut ctr = 0u32;
    let mut next = move || {
        ctr = ctr.wrapping_add(1) & 0x7fff_ffff;
        ctr
    };
    bench.bench("hd.master", |n| {
        let mut x = 0u64;
        for _ in 0..n {
            let m = ExtendedKey::new_master(&seed, &versions).expect("master");
            x ^= u64::from(m.serialized_pub_key()[1]);
        }
        x
    });
    bench.bench("hd.hardened_child", |n| {
        let mut x = 0u64;
        for _ in 0..n {
            let c = acct.child(h(next())).expect("child");
            x ^= u64::from(c.serialized_pub_key()[1]);
        }
        x
    });
    bench.bench("hd.priv_child", |n| {
        let mut x = 0u64;
        for _ in 0..n {
            let c = branch_priv.child(next()).expect("child");
            x ^= u64::from(c.serialized_pub_key()[1]);
        }
        x
    });
    bench.bench("hd.pub_child", |n| {
        let mut x = 0u64;
        for _ in 0..n {
            let c = branch_pub.child(next()).expect("child");
            x ^= u64::from(c.serialized_pub_key()[1]);
        }
        x
    });
    bench.bench("hd.address", |n| {
        let mut x = 0u64;
        for _ in 0..n {
            let c = branch_pub.child(next()).expect("child");
            x ^= u64::from(p2pkh(c.serialized_pub_key(), &params).encode().as_bytes()[3]);
        }
        x
    });
    bench.bench("hd.script", |n| {
        let mut x = 0u64;
        for _ in 0..n {
            let c = branch_pub.child(next()).expect("child");
            let (_, s) = p2pkh(c.serialized_pub_key(), &params).payment_script();
            x ^= u64::from(s[5]);
        }
        x
    });

    // The 20,000-address workload: the first addresses of a branch, as
    // dcrwallet derives them for discovery and for the address pool, on one
    // thread.
    let count = address_count(bench.config().smoke);
    let addrs = || -> Vec<String> {
        (0..count)
            .map(|i| {
                let c = branch_pub.child(i).expect("child");
                p2pkh(c.serialized_pub_key(), &params).encode()
            })
            .collect()
    };
    if bench.selected("hd.addr20k") {
        let a = addrs();
        bench.check("hd.addr20k_first", &a[0]);
        bench.check("hd.addr20k_last", &a[a.len() - 1]);
        bench.check("hd.addr20k_sha256", sha256_lines(&a));
    }
    bench.bench_once("hd.addr20k", || addrs().len() as u64);

    bench.bench("hd.xpub_string", |n| {
        let mut x = 0u64;
        for _ in 0..n {
            x ^= u64::from(acct_pub.string().as_bytes()[5]);
        }
        x
    });
    bench.bench("hd.parse_xpub", |n| {
        let mut x = 0u64;
        for _ in 0..n {
            let k = ExtendedKey::new_key_from_string(&acct_pub_string, &versions).expect("parse");
            x ^= u64::from(k.serialized_pub_key()[1]);
        }
        x
    });
}

fn primitives<W: Write>(bench: &mut Harness<W>) {
    let params = dcroxide_chaincfg::mainnet_params();
    let mut rng = SplitMix64(0x7072_696d);
    let key = rng.bytes(32);
    let mut msg = rng.bytes(37);
    let mut pub_key = vec![0x02];
    pub_key.extend_from_slice(&rng.bytes(32));
    let mut payload = rng.bytes(20);
    let ver = params.pub_key_hash_addr_id;

    let hmac = |msg: &[u8]| {
        let mut mac = Hmac::<Sha512>::new_from_slice(&key).expect("any key length");
        mac.update(msg);
        mac.finalize().into_bytes()
    };
    bench.check("crypto.hmac_sha512", hex(&hmac(&msg)));
    bench.check("crypto.hash160", hex(&stdaddr::hash160(&pub_key)));
    let encoded = dcroxide_base58::check_encode(&payload, ver);
    bench.check("crypto.base58check", &encoded);

    bench.bench("crypto.hmac_sha512", |n| {
        let mut x = 0u64;
        for i in 0..n {
            msg[36] = i as u8;
            x ^= u64::from(hmac(&msg)[0]);
        }
        x
    });
    bench.bench("crypto.hash160", |n| {
        let mut x = 0u64;
        for i in 0..n {
            pub_key[32] = i as u8;
            x ^= u64::from(stdaddr::hash160(&pub_key)[0]);
        }
        x
    });
    bench.bench("crypto.base58check_encode", |n| {
        let mut x = 0u64;
        for i in 0..n {
            payload[19] = i as u8;
            x ^= u64::from(dcroxide_base58::check_encode(&payload, ver).as_bytes()[5]);
        }
        x
    });
    bench.bench("crypto.base58check_decode", |n| {
        let mut x = 0u64;
        for _ in 0..n {
            let (p, _) = dcroxide_base58::check_decode(&encoded).expect("decode");
            x ^= u64::from(p[0]);
        }
        x
    });
}

fn snacl<W: Write>(bench: &mut Harness<W>) {
    // The setup derives a key at full cost: skipped when nothing here runs.
    if !bench.any_selected(&[
        "snacl.derive",
        "snacl.seal_32",
        "snacl.open_32",
        "snacl.seal_111",
        "snacl.open_111",
    ]) {
        return;
    }
    let mut rng = SplitMix64(0x736e_6163);
    let pass = b"bench passphrase";
    let salt = rng.bytes(32);
    let n = snacl_n(bench.config().smoke);

    // A master key's stored parameters, made with this salt.
    let sk = SecretKey::new_with(
        pass,
        n,
        purse_snacl::DEFAULT_R,
        purse_snacl::DEFAULT_P,
        &mut fixed(&salt),
        &Limits::DEFAULT,
    )
    .expect("new secret key");
    let marshalled = sk.marshal();
    bench.check("snacl.derive_key", hex(sk.key().as_bytes()));
    bench.bench_once("snacl.derive", || {
        let mut sk = SecretKey::unmarshal(&marshalled).expect("unmarshal");
        sk.derive_key(pass).expect("derive");
        u64::from(sk.key().as_bytes()[0])
    });

    let ck_bytes: [u8; 32] = rng.bytes(32).try_into().expect("32 bytes");
    let ck = CryptoKey::from_bytes(&ck_bytes);
    for size in [32usize, 111] {
        let mut plain = rng.bytes(size);
        let nonce = rng.bytes(24);
        // A box as CryptoKey::encrypt writes it, with a fixed nonce.
        let sealed = ck.encrypt_with(&plain, &mut fixed(&nonce)).expect("seal");
        bench.check(&format!("snacl.sealed_{size}"), hex(&sealed));
        assert_eq!(
            ck.decrypt(&sealed).expect("open").as_bytes(),
            &plain[..],
            "snacl: the box does not open"
        );
        bench.bench(&format!("snacl.seal_{size}"), |n| {
            let mut x = 0u64;
            for i in 0..n {
                plain[0] = i as u8;
                let s = ck.encrypt(&plain).expect("seal");
                x ^= u64::from(s[30]);
            }
            x
        });
        bench.bench(&format!("snacl.open_{size}"), |n| {
            let mut x = 0u64;
            for _ in 0..n {
                let p = ck.decrypt(&sealed).expect("open");
                x ^= u64::from(p.as_bytes()[0]);
            }
            x
        });
    }
}

fn kdf<W: Write>(bench: &mut Harness<W>) {
    let mut rng = SplitMix64(0x6b64_6600);
    let pass = b"bench passphrase";
    for threads in [1u8, 4] {
        let mut params = Argon2idParams {
            salt: [0; 16],
            time: 1,
            memory: argon2_memory(bench.config().smoke),
            threads,
        };
        rng.fill(&mut params.salt);
        let name = format!("kdf.argon2id_t{threads}");
        let derive = || params.derive_key(pass).expect("argon2id");
        if bench.selected(&name) {
            bench.check(&format!("{name}_key"), hex(derive().as_bytes()));
        }
        bench.bench_once(&name, || u64::from(derive().as_bytes()[0]));
    }
}

/// 1,024 amounts in atoms of every magnitude from one atom to the money
/// supply, a quarter of them negative.
pub fn amount_inputs() -> Vec<Amount> {
    let mut rng = SplitMix64(0x616d_6f75);
    (0..1024)
        .map(|_| {
            let digits = 1 + rng.next_u64() % 16;
            let m = 10u64.pow(digits as u32);
            let mut a = (rng.next_u64() % m) as i64;
            if rng.next_u64().is_multiple_of(4) {
                a = -a;
            }
            Amount(a)
        })
        .collect()
}

fn amounts<W: Write>(bench: &mut Harness<W>) {
    let input = amount_inputs();
    let strings: Vec<String> = input.iter().map(ToString::to_string).collect();
    bench.check("dcrutil.amount_strings", sha256_lines(&strings));
    bench.bench("dcrutil.amount_string", |n| {
        let mut x = 0u64;
        for i in 0..n {
            x ^= input[(i % 1024) as usize].to_string().len() as u64;
        }
        x
    });
}

/// The mixing field's modulus, 2^127 − 1.
const P: u128 = (1 << 127) - 1;

/// `a·b mod P` for `a, b < P`: the 254-bit product from 64-bit halves,
/// folded at bit 127 (2^127 ≡ 1).
fn mul_mod(a: u128, b: u128) -> u128 {
    let (a1, a0) = (a >> 64, a & u128::from(u64::MAX));
    let (b1, b0) = (b >> 64, b & u128::from(u64::MAX));
    let mid = a0 * b1 + a1 * b0;
    let (lo, carry) = (a0 * b0).overflowing_add(mid << 64);
    let hi = a1 * b1 + (mid >> 64) + u128::from(carry);
    let s = 2 * hi + (lo >> 127) + (lo & P);
    let s = (s & P) + (s >> 127);
    if s >= P { s - P } else { s }
}

fn add_mod(a: u128, b: u128) -> u128 {
    let s = a + b;
    if s >= P { s - P } else { s }
}

fn sub_mod(a: u128, b: u128) -> u128 {
    if a >= b { a - b } else { a + (P - b) }
}

/// The coefficients, constant term first, of (F − 1)·∏(x − r) over
/// `degree` distinct roots from SplitMix64 (the leading coefficient
/// `mixing.Coefficients` gives every polynomial the client solves), and the
/// roots in ascending order.
pub fn mix_instance(degree: usize) -> (Vec<u128>, Vec<u128>) {
    let mut rng = SplitMix64(0x6d69_7800 + degree as u64);
    let mut roots = Vec::with_capacity(degree);
    let mut seen = std::collections::BTreeSet::new();
    while roots.len() < degree {
        let hi = u128::from(rng.next_u64());
        let lo = u128::from(rng.next_u64());
        let v = ((hi << 64) | lo) & P;
        if v == P || !seen.insert(v) {
            continue;
        }
        roots.push(v);
    }
    let mut poly = vec![1u128];
    for &r in &roots {
        let mut next = vec![0u128; poly.len() + 1];
        for (i, &c) in poly.iter().enumerate() {
            next[i + 1] = add_mod(next[i + 1], c);
            next[i] = sub_mod(next[i], mul_mod(c, r));
        }
        poly = next;
    }
    for c in &mut poly {
        *c = mul_mod(*c, P - 1);
    }
    roots.sort_unstable();
    (poly, roots)
}

fn mix<W: Write>(bench: &mut Harness<W>) {
    for degree in mix_degrees(bench.config().smoke) {
        let name = format!("mix.roots_{degree}");
        if !bench.selected(&name) {
            continue;
        }
        let (coeffs, roots) = mix_instance(degree);
        // The CHECK covers the instance both sides solve; each side holds
        // its own solver's roots to it (the Go side in `checkRoots`).
        let lines: Vec<String> = roots.iter().map(|r| format!("{r:x}")).collect();
        bench.check(&format!("{name}_sha256"), sha256_lines(&lines));
        assert_eq!(
            purse_mixsolver::roots(&coeffs).as_ref(),
            Ok(&roots),
            "{name}: the solver's roots"
        );
        bench.bench_once(&name, || {
            purse_mixsolver::roots(&coeffs).expect("roots").len() as u64
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mul_mod_agrees_with_small_cases() {
        assert_eq!(mul_mod(P - 1, P - 1), 1);
        assert_eq!(mul_mod(2, P - 1), P - 2);
        assert_eq!(mul_mod(1 << 126, 2), 1);
        assert_eq!(mul_mod(12345, 67890), 12345 * 67890);
        let mut rng = SplitMix64(7);
        for _ in 0..1000 {
            let a = (u128::from(rng.next_u64()) << 64 | u128::from(rng.next_u64())) % P;
            let b = (u128::from(rng.next_u64()) << 64 | u128::from(rng.next_u64())) % P;
            // (a·b)·1 = a·b, and a·b = b·a, and a·(b+1) = a·b + a.
            assert_eq!(mul_mod(a, b), mul_mod(b, a));
            assert_eq!(mul_mod(a, add_mod(b, 1)), add_mod(mul_mod(a, b), a));
        }
    }

    #[test]
    fn mix_instances_solve() {
        for degree in [1, 2, 5, 20] {
            let (coeffs, roots) = mix_instance(degree);
            assert_eq!(coeffs.len(), degree + 1);
            assert_eq!(coeffs[degree], P - 1);
            assert_eq!(purse_mixsolver::roots(&coeffs), Ok(roots));
        }
    }
}
