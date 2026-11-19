// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! Resource bounds on key derivation from stored parameters.

/// Bounds on what one key derivation may cost.
///
/// Stored parameters are honoured as they are, whatever they are, as long as
/// Go accepts them and they fit these bounds. Go has no such bounds: given
/// parameters from a corrupt or hostile database it allocates what they ask
/// for (up to 2^63 bytes for scrypt, 4 TiB for Argon2) and runs for as long
/// as they say, so the process dies or hangs. Here a derivation over a
/// bound returns [`ErrorCause::MemoryLimit`](crate::ErrorCause::MemoryLimit)
/// or [`ErrorCause::WorkLimit`](crate::ErrorCause::WorkLimit) before
/// allocating anything.
///
/// [`Limits::DEFAULT`] admits sixteen times the work and four times the
/// memory of anything dcrwallet writes (scrypt `N = 2^18, r = 8, p = 1`,
/// 256 MiB; Argon2id time 1, 256 MiB): scrypt up to `N = 2^20` at `r = 8,
/// p = 1` (1 GiB and 3 KiB, for scrypt's working blocks beside its `128 *
/// r * N`-byte table), and Argon2id memory up to 1 GiB. A caller that must
/// open a database made with heavier parameters passes larger limits.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Limits {
    /// The most memory one derivation may allocate, in bytes: scrypt's
    /// `128 * r * (N + p + 2)`, Argon2's memory blocks times 1024.
    pub max_memory_bytes: u64,
    /// The most scrypt work, as `N * r * p`.
    pub max_scrypt_work: u64,
    /// The most Argon2 work, as the time cost times the number of 1 KiB
    /// memory blocks.
    pub max_argon2_work: u64,
}

impl Limits {
    /// 1 GiB and 1 MiB of memory (`2^30 + 2^20` bytes), scrypt work
    /// `2^25` and Argon2 work `2^22`.
    pub const DEFAULT: Limits = Limits {
        max_memory_bytes: (1 << 30) + (1 << 20),
        max_scrypt_work: 1 << 25,
        max_argon2_work: 1 << 22,
    };

    /// No bounds beyond Go's own parameter checks. An allocation scrypt
    /// cannot make then aborts the process, as it kills Go's; Argon2's
    /// returns [`ErrorCause::OutOfMemory`](crate::ErrorCause::OutOfMemory).
    pub const UNLIMITED: Limits = Limits {
        max_memory_bytes: u64::MAX,
        max_scrypt_work: u64::MAX,
        max_argon2_work: u64::MAX,
    };
}

impl Default for Limits {
    fn default() -> Self {
        Limits::DEFAULT
    }
}
