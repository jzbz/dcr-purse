// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! dcrwallet's key sealing, byte for byte: a port of its
//! `wallet/internal/snacl` and `kdf` packages and of the per-account
//! passphrase sealing in `wallet/udb`, at the parity pin.
//!
//! dcr-purse seals its keys in dcrwallet's blob formats, which are tested
//! against values dcrwallet itself sealed, so every format, parameter
//! encoding and error here matches Go's.
//!
//! # The two schemes
//!
//! **Global passphrases (snacl).** Each of the public and private
//! passphrases derives a master [`SecretKey`] with scrypt; its
//! [`Parameters`] (salt, cost and the SHA-256 digest of the key, which
//! checks the passphrase) are stored marshalled, 88 bytes (`mpub`, `mpriv`).
//! Each master key seals a random 32-byte [`CryptoKey`] (`cpub`, `cpriv`),
//! and the crypto keys seal everything else: extended keys, imported keys,
//! script hashes. A sealed value is `nonce(24) || tag(16) || ciphertext`,
//! NaCl secretbox (XSalsa20-Poly1305).
//!
//! **Per-account passphrases (kdf + udb).** An account given its own
//! passphrase stores [`Argon2idParams`] (25 bytes, `kdf-params`) and its
//! extended private key sealed with XChaCha20-Poly1305 under the
//! Argon2id-derived [`AccountKey`]: `nonce(24) || ciphertext || tag(16)`, no
//! associated data.
//!
//! | Go | Rust |
//! |---|---|
//! | `snacl.NewSecretKey` | [`SecretKey::new`], [`SecretKey::new_with`] |
//! | `(*SecretKey).Marshal`, `Unmarshal` | [`SecretKey::marshal`], [`SecretKey::unmarshal`], [`Parameters`] |
//! | `(*SecretKey).DeriveKey` | [`SecretKey::derive_key`], [`SecretKey::derive_key_with`] |
//! | `(*SecretKey).Encrypt`, `Decrypt`, `Zero` | [`SecretKey::encrypt`], [`SecretKey::decrypt`], [`SecretKey::zero`] |
//! | `snacl.GenerateCryptoKey` | [`CryptoKey::generate`], [`CryptoKey::generate_with`] |
//! | `(*CryptoKey).Encrypt`, `Decrypt`, `Zero` | [`CryptoKey::encrypt`], [`CryptoKey::decrypt`], [`CryptoKey::zero`] |
//! | `kdf.NewArgon2idParams` | [`Argon2idParams::new`], [`Argon2idParams::new_with`] |
//! | `MarshalBinary`, `UnmarshalBinary` | [`Argon2idParams::marshal_binary`], [`Argon2idParams::unmarshal_binary`] |
//! | `kdf.DeriveKey(.., 32)`, udb `argon2idKey` | [`Argon2idParams::derive_key`] |
//! | udb `seal`, `unseal` | [`AccountKey::seal`], [`AccountKey::unseal`] |
//!
//! # Randomness
//!
//! Every operation that draws randomness has a `_with` form taking a
//! [`RandomSource`]; the plain form uses [`OsRandom`]. Each makes one draw
//! of exactly the bytes Go draws (salt, nonce or key), so a test can
//! reproduce Go's output byte for byte.
//!
//! # Stored parameters and resource bounds
//!
//! Parameters read from a database are honoured as stored: scrypt `N`, `r`,
//! `p` and Argon2's time, memory and threads all change the key. Go checks
//! scrypt's parameters (with x/crypto's errors, reproduced) and nothing else;
//! here a derivation also fails, before allocating, when the parameters ask
//! for more than [`Limits`] allows, and fails with an error where Go panics
//! (Argon2 time or threads of 0). Tests use small parameters through the
//! same entry points (`N = 2`, Argon2 memory of 8 KiB per thread).
//!
//! Argon2's stored thread count is its lane count; it does not decide how
//! many threads run. The `argon2` crate fills the lanes of each slice in
//! parallel through rayon, as Go starts a goroutine per lane, and each
//! derivation runs them on a rayon pool made for it alone: one thread per
//! lane, at most one per CPU this process may use
//! (`std::thread::available_parallelism`, which honours a cgroup CPU quota;
//! the count new parameters store is [`num_cpu`], Go's), the calling thread
//! among them and working with the rest. The others are
//! scoped threads, joined before the derivation returns, so a derivation
//! leaves no thread running and rayon's global pool is never started. A
//! count of 255 runs on as many threads as a count equal to the CPU count,
//! and the bounds above hold whatever the count. If the operating system
//! refuses a thread, those already started finish and the calling thread
//! fills every lane alone: slower, and the same key. The key does not
//! depend on the number of threads. The cap is per derivation, not per
//! process: `n` derivations at once run up to `n` times as many threads
//! as one, and more threads than CPUs, where Go's goroutines share
//! `GOMAXPROCS` threads. A derivation made on a thread of some other rayon
//! pool runs its lanes on that pool, whose threads outlive it.
//!
//! # Secrets
//!
//! [`CryptoKey`], [`AccountKey`], a [`SecretKey`]'s key and decrypted
//! [`SecretBytes`] are wiped on drop and never formatted. The keys live on
//! the heap and are built there, so moving one copies a pointer, not the
//! key. Passphrases are borrowed (`&[u8]`) and left to the caller, except
//! that an Argon2id derivation copies its passphrase to the heap for as
//! long as it runs (its pool runs it as a job, which cannot borrow) and
//! wipes the copy. Comparisons Go makes with `subtle.ConstantTimeCompare`
//! (the master key digest) and the authentication tags are constant time.
//! Buffers this crate fills with plaintext are wiped on every exit,
//! including errors.
//!
//! What this crate cannot wipe, because the dependencies that hold it free
//! it unwiped, as Go's garbage collector frees Go's:
//!
//! - **scrypt** (the `scrypt` crate): its `V` table (`128 * r * N` bytes),
//!   `B` (`128 * r * p`) and `T` (`128 * r`) buffers. `V`'s first block is
//!   PBKDF2-HMAC-SHA256(passphrase, salt, 1), enough to test passphrases
//!   cheaply, and `B` ends holding the final PBKDF2's input.
//! - **HMAC-SHA256** (the `hmac` crate, under scrypt's PBKDF2): the key pad,
//!   the passphrase XORed with ipad and opad, left on the stack.
//! - **PBKDF2** (the `pbkdf2` crate): its last block on the stack, which for
//!   scrypt's final step is the derived master key.
//! - **XSalsa20** (the `salsa20` crate): the HSalsa20 subkey of each box,
//!   a stack temporary; the cipher state itself is wiped.
//! - **Argon2id** (the `argon2` crate): its memory blocks, H0 and block
//!   hashes are wiped, and so are the Blake2b states it drops; the output
//!   buffer of its final Blake2b (`blake2b_long`), which holds the derived
//!   account key, is not, nor are the 64-byte digests of its long-output
//!   path, from which the first memory blocks are made, nor the
//!   compression function's stack temporaries (each new block before it
//!   is stored, and the permutation's working copy). The whole derivation
//!   runs as one job on whichever of its pool's threads takes it, most
//!   often one the pool started rather than the calling thread, so H0's
//!   buffers, the 64-byte digests and the final output buffer holding the
//!   account key are left on that thread's stack, which also wipes and
//!   frees the memory blocks. The compression temporaries are left on the
//!   stack of each thread that filled a segment. The threads a pool
//!   started have finished before the derivation returns, but their
//!   stacks are freed unwiped (the C library may keep one for a later
//!   thread). A derivation made on a thread of some other rayon pool
//!   leaves all of these on that pool's threads, which live on.
//!
//! Beyond those, copies the compiler makes on the stack when values move are
//! wiped only as far as the code above controls them: best effort, and
//! dependent on code generation.
//!
//! # Errors
//!
//! [`Error`] renders as dcrwallet's `*errors.Error` does
//! (`secretkey.DeriveKey: invalid passphrase`, `secretkey.Decrypt:
//! encryption/decryption error:: cryptokey.Decrypt`, ...), carries the
//! dcrwallet [`Kind`](purse_errors::Kind) and [`Op`] of each exit, and
//! converts without loss into [`purse_errors::Error`].

mod account;
mod error;
mod kdf;
mod limits;
mod random;
mod secret;
mod secretbox;
mod snacl;

pub use account::{
    AccountKey, POLY1305_TAG_SIZE, XCHACHA20_NONCE_SIZE, XCHACHA20POLY1305_OVERHEAD,
};
pub use error::{Error, ErrorCause, Kdf, Op};
pub use kdf::{ARGON2ID_MARSHALED_LEN, ARGON2ID_SALT_LEN, Argon2idParams, num_cpu};
pub use limits::Limits;
pub use random::{OsRandom, RandomSource, RandomSourceError};
pub use secret::SecretBytes;
pub use snacl::{
    CryptoKey, DEFAULT_N, DEFAULT_P, DEFAULT_R, KEY_SIZE, MARSHALLED_LEN, NONCE_SIZE, OVERHEAD,
    Parameters, SecretKey,
};
