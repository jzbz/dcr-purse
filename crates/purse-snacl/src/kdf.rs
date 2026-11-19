// Copyright (c) 2026 The dcr-purse developers
// Copyright (c) 2020 The Decred developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! Port of dcrwallet's `kdf` package: the Argon2id parameters of per-account
//! passphrases, their stored form and the key derivation.

use argon2::{Algorithm, Argon2, Block, Version};
use purse_errors::Kind;
use zeroize::Zeroize;

use crate::account::AccountKey;
use crate::error::{Error, ErrorCause, Kdf};
use crate::limits::Limits;
use crate::random::{OsRandom, RandomSource};

/// The length of stored Argon2id parameters (Go's `kdf.MarshaledLen`).
pub const ARGON2ID_MARSHALED_LEN: usize = 25;

/// The length of an Argon2id salt.
pub const ARGON2ID_SALT_LEN: usize = 16;

/// Argon2 synchronization points per pass (the slices of a lane).
const SYNC_POINTS: u32 = 4;

/// The Argon2id cost and salt of a per-account passphrase (Go's
/// `kdf.Argon2idParams`), stored in the account's `kdf-params` variable.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Argon2idParams {
    /// The salt.
    pub salt: [u8; ARGON2ID_SALT_LEN],
    /// The number of passes over memory.
    pub time: u32,
    /// The memory cost in KiB.
    pub memory: u32,
    /// The parallelism, which also changes the derived key.
    pub threads: u8,
}

impl Argon2idParams {
    /// The time cost dcrwallet writes.
    pub const DEFAULT_TIME: u32 = 1;
    /// The memory cost dcrwallet writes, 256 MiB.
    pub const DEFAULT_MEMORY: u32 = 256 * 1024;

    /// New parameters with a salt from the operating system's random source
    /// and a thread count from [`num_cpu`] (Go's `kdf.NewArgon2idParams`,
    /// `kdf.go:30-39`, as dcrwallet calls it).
    ///
    /// # Errors
    ///
    /// As [`Argon2idParams::new_with`].
    pub fn new() -> Result<Self, Error> {
        Argon2idParams::new_with(&mut OsRandom, num_cpu())
    }

    /// New parameters: time [`Self::DEFAULT_TIME`], memory
    /// [`Self::DEFAULT_MEMORY`], threads
    /// [`Argon2idParams::threads_for_cpus`]`(num_cpu)` and a 16-byte salt
    /// read from `rng`.
    ///
    /// # Errors
    ///
    /// Only a failing source: no op, kind `Other`,
    /// [`ErrorCause::RandomSource`], which renders as the source's message
    /// alone, as Go returns its reader's error.
    pub fn new_with<R: RandomSource + ?Sized>(rng: &mut R, num_cpu: usize) -> Result<Self, Error> {
        let mut params = Argon2idParams {
            salt: [0; ARGON2ID_SALT_LEN],
            time: Self::DEFAULT_TIME,
            memory: Self::DEFAULT_MEMORY,
            threads: Self::threads_for_cpus(num_cpu),
        };
        rng.fill(&mut params.salt).map_err(|e| {
            Error::new(
                None,
                Kind::Other,
                ErrorCause::RandomSource(e.into_message()),
            )
        })?;
        Ok(params)
    }

    /// Go's `uint8(min(runtime.NumCPU(), 256))`: the CPU count, which
    /// truncates to 0 at 256 or more CPUs (QUIRKS QK-0034). Deriving with 0
    /// threads then fails ([`ErrorCause::Argon2ParallelismTooLow`]) where Go
    /// panics, so on such a machine neither can set an account passphrase.
    pub fn threads_for_cpus(num_cpu: usize) -> u8 {
        let n = num_cpu.min(256);
        // 256 truncates to 0, as Go's conversion does.
        u8::try_from(n).unwrap_or(0)
    }

    /// The stored form, `salt || time || memory || threads` with the two
    /// integers little-endian `uint32`s (Go's `MarshalBinary`).
    pub fn marshal_binary(&self) -> [u8; ARGON2ID_MARSHALED_LEN] {
        let mut out = [0u8; ARGON2ID_MARSHALED_LEN];
        let (salt, rest) = out.split_at_mut(ARGON2ID_SALT_LEN);
        salt.copy_from_slice(&self.salt);
        let (time, rest) = rest.split_at_mut(4);
        time.copy_from_slice(&self.time.to_le_bytes());
        let (memory, threads) = rest.split_at_mut(4);
        memory.copy_from_slice(&self.memory.to_le_bytes());
        threads.copy_from_slice(&[self.threads]);
        out
    }

    /// Reads the stored form (Go's `UnmarshalBinary`, `kdf.go:56-65`). No
    /// value is range-checked, as in Go.
    ///
    /// # Errors
    ///
    /// Any length but 25: no op, kind `Other`,
    /// [`ErrorCause::InvalidArgon2idParams`]
    /// (`invalid marshaled Argon2id parameters`).
    pub fn unmarshal_binary(data: &[u8]) -> Result<Self, Error> {
        let invalid = || Error::new(None, Kind::Other, ErrorCause::InvalidArgon2idParams);
        let data: &[u8; ARGON2ID_MARSHALED_LEN] = data.try_into().map_err(|_| invalid())?;
        let (salt, rest) = data
            .split_first_chunk::<ARGON2ID_SALT_LEN>()
            .ok_or_else(invalid)?;
        let (time, rest) = rest.split_first_chunk::<4>().ok_or_else(invalid)?;
        let (memory, rest) = rest.split_first_chunk::<4>().ok_or_else(invalid)?;
        let (&[threads], _) = rest.split_first_chunk::<1>().ok_or_else(invalid)?;
        Ok(Argon2idParams {
            salt: *salt,
            time: u32::from_le_bytes(*time),
            memory: u32::from_le_bytes(*memory),
            threads,
        })
    }

    /// The 32-byte key for `passphrase`, under [`Limits::DEFAULT`] (Go's
    /// `kdf.DeriveKey(passphrase, p, 32)`, which dcrwallet's
    /// `argon2idKey` calls; 32 bytes is the only length dcrwallet uses).
    ///
    /// # Errors
    ///
    /// As [`Argon2idParams::derive_key_with`].
    pub fn derive_key(&self, passphrase: &[u8]) -> Result<AccountKey, Error> {
        self.derive_key_with(passphrase, &Limits::DEFAULT)
    }

    /// Argon2id (version 0x13) of `passphrase` with these parameters, 32
    /// bytes long. The working memory is allocated fallibly and wiped
    /// before it is freed.
    ///
    /// # Errors
    ///
    /// All with no op and kind `Other`. Where Go panics: a time cost of 0
    /// ([`ErrorCause::Argon2RoundsTooSmall`]) or 0 threads
    /// ([`ErrorCause::Argon2ParallelismTooLow`]), with Go's panic messages
    /// as text. Where Go derives a key this crate cannot: a memory cost
    /// below 8 KiB per thread ([`ErrorCause::Argon2MemoryBelowMinimum`]).
    /// Where Go allocates without bound: parameters over `limits`
    /// ([`ErrorCause::MemoryLimit`], [`ErrorCause::WorkLimit`]) or memory
    /// that cannot be allocated ([`ErrorCause::OutOfMemory`]).
    pub fn derive_key_with(&self, passphrase: &[u8], limits: &Limits) -> Result<AccountKey, Error> {
        let mut key = AccountKey::default();
        argon2id(passphrase, self, limits, key.as_mut_bytes())
            .map_err(|cause| Error::new(None, Kind::Other, cause))?;
        Ok(key)
    }
}

/// x/crypto's `argon2.IDKey(passphrase, salt, time, memory, threads, 32)`
/// into `out`, with Go's panics turned into errors (`argon2.go:105-121`) and
/// this crate's resource bounds.
fn argon2id(
    passphrase: &[u8],
    params: &Argon2idParams,
    limits: &Limits,
    out: &mut [u8; 32],
) -> Result<(), ErrorCause> {
    if params.time < 1 {
        return Err(ErrorCause::Argon2RoundsTooSmall);
    }
    if params.threads < 1 {
        return Err(ErrorCause::Argon2ParallelismTooLow);
    }
    let lanes = u32::from(params.threads);
    let per_slice = lanes.saturating_mul(SYNC_POINTS);
    // Go raises a memory cost below 2*syncPoints*threads blocks to that and
    // still hashes the stored value into H0; the argon2 crate refuses it.
    if params.memory < per_slice.saturating_mul(2) {
        return Err(ErrorCause::Argon2MemoryBelowMinimum {
            memory: params.memory,
            threads: params.threads,
        });
    }
    // `memory / (syncPoints*threads) * (syncPoints*threads)` blocks, as Go
    // and the argon2 crate both compute.
    let blocks = params
        .memory
        .checked_div(per_slice)
        .unwrap_or(0)
        .saturating_mul(per_slice);
    let memory = u64::from(blocks).saturating_mul(1024);
    if memory > limits.max_memory_bytes {
        return Err(ErrorCause::MemoryLimit {
            kdf: Kdf::Argon2,
            needed: memory,
            limit: limits.max_memory_bytes,
        });
    }
    let work = u64::from(params.time).saturating_mul(u64::from(blocks));
    if work > limits.max_argon2_work {
        return Err(ErrorCause::WorkLimit {
            kdf: Kdf::Argon2,
            needed: work,
            limit: limits.max_argon2_work,
        });
    }
    let rejected = |e: argon2::Error| ErrorCause::Argon2Rejected(e.to_string());
    let p = argon2::Params::new(params.memory, params.time, lanes, Some(32)).map_err(rejected)?;
    let ctx = Argon2::new(Algorithm::Argon2id, Version::V0x13, p);
    let out_of_memory = ErrorCause::OutOfMemory {
        kdf: Kdf::Argon2,
        bytes: memory,
    };
    let count = usize::try_from(blocks).map_err(|_| out_of_memory.clone())?;
    let mut working: Vec<Block> = Vec::new();
    working
        .try_reserve_exact(count)
        .map_err(|_| out_of_memory)?;
    working.resize(count, Block::new());
    let hashed = ctx.hash_password_into_with_memory(passphrase, &params.salt, out, &mut working);
    working.zeroize();
    hashed.map_err(rejected)
}

/// The number of CPUs this process may use, for
/// [`Argon2idParams::threads_for_cpus`]: `std::thread::available_parallelism`,
/// or 1 when it is unknown.
///
/// Go's `runtime.NumCPU` counts the CPUs in the process's affinity mask;
/// `available_parallelism` also honours cgroup CPU quotas, so in a container
/// with a quota it can be lower. The count only shapes new parameters, which
/// are stored, so no existing passphrase depends on it.
pub fn num_cpu() -> usize {
    std::thread::available_parallelism().map_or(1, std::num::NonZero::get)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn threads_truncate_like_go() {
        assert_eq!(Argon2idParams::threads_for_cpus(0), 0);
        assert_eq!(Argon2idParams::threads_for_cpus(1), 1);
        assert_eq!(Argon2idParams::threads_for_cpus(255), 255);
        assert_eq!(Argon2idParams::threads_for_cpus(256), 0);
        assert_eq!(Argon2idParams::threads_for_cpus(100_000), 0);
    }

    #[test]
    fn marshal_round_trips() {
        let p = Argon2idParams {
            salt: [3; 16],
            time: 0xdead_beef,
            memory: 7,
            threads: 0,
        };
        assert_eq!(Argon2idParams::unmarshal_binary(&p.marshal_binary()), Ok(p));
    }
}
