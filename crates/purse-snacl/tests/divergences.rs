// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! Where this crate deliberately does what Go does not, so there is no Go
//! outcome to compare with: resource bounds on stored parameters (Go
//! allocates without bound), a failing random source (Go's cannot fail),
//! the key left after a failed derivation (Go leaves the wrong one), and
//! the redaction of secrets. The expected texts here are this crate's own.

use purse_errors::{DAEMON_SEPARATOR, Kind};
use purse_snacl::{
    AccountKey, Argon2idParams, CryptoKey, ErrorCause, Kdf, Limits, Parameters, RandomSourceError,
    SecretKey,
};

fn params(n: i64, r: i64, p: i64) -> Parameters {
    Parameters {
        salt: [1; 32],
        digest: [2; 32],
        n,
        r,
        p,
    }
}

#[test]
fn hostile_scrypt_costs_are_refused_before_allocating() {
    // Go accepts N = 2^40, r = 1 and would allocate 128 TiB.
    let mut sk = SecretKey::from_parameters(params(1 << 40, 1, 1));
    let e = sk.derive_key(b"x").expect_err("over the memory limit");
    assert_eq!(e.kind(), Kind::Other);
    assert!(matches!(
        e.cause(),
        Some(ErrorCause::MemoryLimit { kdf: Kdf::Scrypt, needed, limit })
            if *needed == 128 * ((1u64 << 40) + 3) && *limit == 1_074_790_400
    ));
    assert_eq!(
        e.to_string(),
        format!(
            "secretkey.DeriveKey: scrypt: parameters need {} bytes of memory, above the limit of 1074790400",
            128 * ((1u64 << 40) + 3)
        )
    );
    // Little memory, much work: p = 2^20 block mixes of 2^14 each.
    let mut sk = SecretKey::from_parameters(params(1 << 14, 1, 1 << 20));
    let e = sk.derive_key(b"x").expect_err("over the work limit");
    assert!(matches!(
        e.cause(),
        Some(ErrorCause::WorkLimit { kdf: Kdf::Scrypt, needed, .. }) if *needed == 1 << 34
    ));
    // Go's own refusals still come first.
    let mut sk = SecretKey::from_parameters(params(1 << 60, 1, 1));
    let e = sk.derive_key(b"x").expect_err("too large for Go");
    assert_eq!(e.cause(), Some(&ErrorCause::ScryptTooLarge));
}

/// What `Limits::DEFAULT` promises: four times the memory dcrwallet writes,
/// scrypt `N = 2^20, r = 8, p = 1` and Argon2id 1 GiB, and no more. Each case
/// is stopped by a work limit just below its work, so passing the memory
/// check is seen without running a 1 GiB derivation.
#[test]
fn default_limits_admit_four_times_dcrwallets_memory() {
    assert_eq!(Limits::DEFAULT.max_memory_bytes, (1 << 30) + (1 << 20));
    assert_eq!(Limits::default(), Limits::DEFAULT);
    // scrypt: 2^20 * 8 * 1 = 2^23 work; 128 * 8 * (2^20 + 3) bytes.
    let stop = Limits {
        max_scrypt_work: (1 << 23) - 1,
        ..Limits::DEFAULT
    };
    let mut sk = SecretKey::from_parameters(params(1 << 20, 8, 1));
    let e = sk
        .derive_key_with(b"x", &stop)
        .expect_err("stopped by work");
    assert_eq!(
        e.cause(),
        Some(&ErrorCause::WorkLimit {
            kdf: Kdf::Scrypt,
            needed: 1 << 23,
            limit: (1 << 23) - 1,
        })
    );
    let mut sk = SecretKey::from_parameters(params(1 << 21, 8, 1));
    let e = sk.derive_key(b"x").expect_err("eight times the memory");
    assert_eq!(
        e.cause(),
        Some(&ErrorCause::MemoryLimit {
            kdf: Kdf::Scrypt,
            needed: 1024 * ((1 << 21) + 3),
            limit: Limits::DEFAULT.max_memory_bytes,
        })
    );
    // Argon2id: 2^20 KiB, then the last 1 KiB blocks the limit admits.
    let stop = Limits {
        max_argon2_work: (1 << 20) - 1,
        ..Limits::DEFAULT
    };
    for memory in [1 << 20, (1 << 20) + 1024] {
        let p = argon2(1, memory, 1);
        let e = p.derive_key_with(b"x", &stop).expect_err("stopped by work");
        assert!(
            matches!(e.cause(), Some(ErrorCause::WorkLimit { kdf: Kdf::Argon2, needed, .. }) if *needed == u64::from(memory)),
            "{memory}: {e}"
        );
    }
    let e = argon2(1, (1 << 20) + 1028, 1)
        .derive_key_with(b"x", &stop)
        .expect_err("one block group over");
    assert!(matches!(
        e.cause(),
        Some(ErrorCause::MemoryLimit {
            kdf: Kdf::Argon2,
            needed: 1_074_794_496,
            ..
        })
    ));
}

fn argon2(time: u32, memory: u32, threads: u8) -> Argon2idParams {
    Argon2idParams {
        salt: [4; 16],
        time,
        memory,
        threads,
    }
}

fn zero_rng() -> impl FnMut(&mut [u8]) -> Result<(), RandomSourceError> {
    |b: &mut [u8]| {
        b.fill(0);
        Ok(())
    }
}

/// Limits other than the default reach the derivations: every bound, for
/// both functions, refuses one unit below the cost and admits the cost
/// exactly (the checks are inclusive), and the key is then the one the
/// default limits derive.
#[test]
fn limits_are_adjustable() {
    let tight = Limits {
        max_memory_bytes: 1024,
        ..Limits::DEFAULT
    };
    let e = SecretKey::new_with(b"x", 16, 1, 1, &mut zero_rng(), &tight).expect_err("2 KiB needed");
    assert_eq!(e.op().map(|o| o.as_str()), Some("snacl.NewSecretKey"));

    // scrypt N = 2^10, r = 1, p = 1: work 1024, memory 128 * (1024 + 3).
    let made =
        SecretKey::new_with(b"pw", 1 << 10, 1, 1, &mut zero_rng(), &Limits::DEFAULT).expect("new");
    let scrypt_cases = [
        (
            Limits {
                max_scrypt_work: 1023,
                ..Limits::DEFAULT
            },
            Some(ErrorCause::WorkLimit {
                kdf: Kdf::Scrypt,
                needed: 1024,
                limit: 1023,
            }),
        ),
        (
            Limits {
                max_scrypt_work: 1024,
                ..Limits::DEFAULT
            },
            None,
        ),
        (
            Limits {
                max_memory_bytes: 131_455,
                ..Limits::DEFAULT
            },
            Some(ErrorCause::MemoryLimit {
                kdf: Kdf::Scrypt,
                needed: 131_456,
                limit: 131_455,
            }),
        ),
        (
            Limits {
                max_memory_bytes: 131_456,
                ..Limits::DEFAULT
            },
            None,
        ),
        (Limits::UNLIMITED, None),
    ];
    for (limits, refusal) in scrypt_cases {
        let mut sk = SecretKey::from_parameters(*made.parameters());
        let got = sk.derive_key_with(b"pw", &limits);
        match refusal {
            Some(cause) => {
                let e = got.expect_err("over the limit");
                assert_eq!(e.op().map(|o| o.as_str()), Some("secretkey.DeriveKey"));
                assert_eq!(e.kind(), Kind::Other);
                assert_eq!(e.cause(), Some(&cause), "{limits:?}");
                assert_eq!(sk.key().as_bytes(), &[0; 32]);
            }
            None => {
                got.expect("at the limit");
                assert_eq!(sk.key().as_bytes(), made.key().as_bytes(), "{limits:?}");
            }
        }
    }

    // Argon2id time 1, 8 KiB, 1 thread: 8 blocks, work 8, 8192 bytes.
    let p = argon2(1, 8, 1);
    let key = p.derive_key(b"pw").expect("default limits");
    let argon2_cases = [
        (
            Limits {
                max_argon2_work: 7,
                ..Limits::DEFAULT
            },
            Some(ErrorCause::WorkLimit {
                kdf: Kdf::Argon2,
                needed: 8,
                limit: 7,
            }),
        ),
        (
            Limits {
                max_argon2_work: 8,
                ..Limits::DEFAULT
            },
            None,
        ),
        (
            Limits {
                max_memory_bytes: 8191,
                ..Limits::DEFAULT
            },
            Some(ErrorCause::MemoryLimit {
                kdf: Kdf::Argon2,
                needed: 8192,
                limit: 8191,
            }),
        ),
        (
            Limits {
                max_memory_bytes: 8192,
                ..Limits::DEFAULT
            },
            None,
        ),
        (Limits::UNLIMITED, None),
    ];
    for (limits, refusal) in argon2_cases {
        let got = p.derive_key_with(b"pw", &limits);
        match refusal {
            Some(cause) => {
                let e = got.map(|_| ()).expect_err("over the limit");
                assert_eq!(e.op(), None);
                assert_eq!(e.kind(), Kind::Other);
                assert_eq!(e.cause(), Some(&cause), "{limits:?}");
            }
            None => assert_eq!(
                got.expect("at the limit").as_bytes(),
                key.as_bytes(),
                "{limits:?}"
            ),
        }
    }
}

/// The digest check covers all 32 bytes: the right passphrase against a
/// stored digest with any one byte changed is refused as Go refuses it
/// (`subtle.ConstantTimeCompare` over the whole digest). The differential
/// corpus pins the same against Go; this keeps it pinned without Go.
#[test]
fn every_digest_byte_is_checked() {
    let made =
        SecretKey::new_with(b"pw", 16, 1, 1, &mut zero_rng(), &Limits::DEFAULT).expect("new");
    for at in 0..32 {
        let mut stored = *made.parameters();
        stored.digest[at] ^= 0x80;
        let mut sk = SecretKey::from_parameters(stored);
        let e = sk.derive_key(b"pw").expect_err("tampered digest");
        assert!(e.is_kind(Kind::Passphrase), "byte {at}: {e}");
        assert_eq!(e.to_string(), "secretkey.DeriveKey: invalid passphrase");
    }
    let mut sk = SecretKey::from_parameters(*made.parameters());
    sk.derive_key(b"pw").expect("untampered");
}

#[test]
fn hostile_argon2_costs_are_refused_before_allocating() {
    let p = Argon2idParams {
        salt: [0; 16],
        time: 1,
        memory: u32::MAX,
        threads: 1,
    };
    let e = p.derive_key(b"x").expect_err("4 TiB");
    assert!(matches!(
        e.cause(),
        Some(ErrorCause::MemoryLimit {
            kdf: Kdf::Argon2,
            ..
        })
    ));
    assert_eq!(e.op(), None);
    let p = Argon2idParams {
        salt: [0; 16],
        time: u32::MAX,
        memory: 8,
        threads: 1,
    };
    let e = p.derive_key(b"x").expect_err("4 billion passes");
    assert!(matches!(
        e.cause(),
        Some(ErrorCause::WorkLimit { kdf: Kdf::Argon2, needed, .. }) if *needed == u64::from(u32::MAX) * 8
    ));
    assert!(e.to_string().starts_with("argon2: parameters need "));
}

#[test]
fn a_failed_derivation_zeroes_the_key() {
    let mut rng = |b: &mut [u8]| {
        b.fill(0x5a);
        Ok(())
    };
    let sk = SecretKey::new_with(b"right", 4, 1, 1, &mut rng, &Limits::DEFAULT).expect("new");
    let mut again = SecretKey::from_parts(sk.key().clone(), *sk.parameters());
    assert_ne!(again.key().as_bytes(), &[0; 32]);
    let e = again.derive_key(b"wrong").expect_err("wrong passphrase");
    assert!(e.is_kind(Kind::Passphrase));
    assert_eq!(again.key().as_bytes(), &[0; 32]);
    again.derive_key(b"right").expect("right passphrase");
    assert_eq!(again.key().as_bytes(), sk.key().as_bytes());
    // A refused cost zeroes too.
    let mut bad = SecretKey::from_parts(sk.key().clone(), params(3, 1, 1));
    bad.derive_key(b"right").expect_err("N not a power of two");
    assert_eq!(bad.key().as_bytes(), &[0; 32]);
}

#[test]
fn a_failing_random_source_is_an_error() {
    let mut boom = |_: &mut [u8]| Err(RandomSourceError::new("boom"));
    let key = CryptoKey::from_bytes(&[3; 32]);
    let cases = [
        (
            CryptoKey::generate_with(&mut boom)
                .map(|_| ())
                .expect_err("gen"),
            "snacl.GenerateCryptoKey: boom",
        ),
        (
            key.encrypt_with(b"m", &mut boom)
                .map(|_| ())
                .expect_err("enc"),
            "cryptokey.Encrypt: boom",
        ),
        (
            SecretKey::from_parts(key.clone(), params(2, 1, 1))
                .encrypt_with(b"m", &mut boom)
                .map(|_| ())
                .expect_err("senc"),
            "secretkey.Encrypt:: cryptokey.Encrypt: boom",
        ),
        (
            SecretKey::new_with(b"p", 2, 1, 1, &mut boom, &Limits::DEFAULT)
                .map(|_| ())
                .expect_err("new"),
            "snacl.NewSecretKey: boom",
        ),
        (
            AccountKey::from_bytes(&[4; 32])
                .seal_with(b"m", &mut boom)
                .map(|_| ())
                .expect_err("seal"),
            "boom",
        ),
        (
            Argon2idParams::new_with(&mut boom, 4)
                .map(|_| ())
                .expect_err("kdf"),
            "boom",
        ),
    ];
    for (e, text) in cases {
        assert_eq!(e.to_string(), text);
        assert_eq!(
            e.root_cause(),
            Some(&ErrorCause::RandomSource("boom".into()))
        );
        assert_eq!(e.to_purse_error().render(DAEMON_SEPARATOR), text);
    }
}

#[test]
fn secrets_never_print() {
    let key = CryptoKey::from_bytes(&[0xab; 32]);
    let sk = SecretKey::from_parts(key.clone(), params(2, 1, 1));
    let ak = AccountKey::from_bytes(&[0xcd; 32]);
    let pt = key
        .decrypt(&key.encrypt(b"plaintext!").expect("enc"))
        .expect("dec");
    for s in [
        format!("{key:?}"),
        format!("{sk:?}"),
        format!("{ak:?}"),
        format!("{pt:?}"),
    ] {
        assert!(
            !s.contains("171") && !s.contains("ab") && !s.contains("plaintext!"),
            "{s}"
        );
        assert!(s.contains("redacted"), "{s}");
    }
}

#[test]
fn os_random_defaults_work() {
    let ck = CryptoKey::generate().expect("generate");
    let sealed = ck.encrypt(b"m").expect("encrypt");
    assert_eq!(ck.decrypt(&sealed).expect("decrypt").as_bytes(), b"m");
    let p = Argon2idParams::new().expect("params");
    assert_eq!(
        p.threads,
        Argon2idParams::threads_for_cpus(purse_snacl::num_cpu())
    );
    assert_eq!((p.time, p.memory), (1, 256 * 1024));
}
