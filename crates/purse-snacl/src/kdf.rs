// Copyright (c) 2026 The dcr-purse developers
// Copyright (c) 2020 The Decred developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! Port of dcrwallet's `kdf` package: the Argon2id parameters of per-account
//! passphrases, their stored form and the key derivation.

use std::any::Any;
use std::io;
use std::num::NonZero;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};
use std::thread::{self, Scope, ScopedJoinHandle};

use argon2::{Algorithm, Argon2, Block, Version};
use purse_errors::Kind;
use rayon::{ThreadBuilder, ThreadPoolBuilder};
use zeroize::{Zeroize, Zeroizing};

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
    /// before it is freed. The lanes of each slice are filled in parallel
    /// by a thread pool made for this derivation, the calling thread among
    /// its threads, and the pool's threads finish before this returns; the
    /// key does not depend on the pool's size. Called on a thread of some
    /// other rayon pool, it runs the lanes on that pool instead (see
    /// "Stored parameters and resource bounds" in the crate documentation).
    /// A panic inside the derivation reaches the caller, as from
    /// `std::thread::scope`.
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
        argon2id(
            passphrase,
            self,
            limits,
            key.as_mut_bytes(),
            None,
            spawn_lane_thread,
        )
        .map_err(|cause| Error::new(None, Kind::Other, cause))?;
        Ok(key)
    }
}

/// x/crypto's `argon2.IDKey(passphrase, salt, time, memory, threads, 32)`
/// into `out`, with Go's panics turned into errors (`argon2.go:105-121`) and
/// this crate's resource bounds. The lanes run on `threads` threads, the
/// calling one included (by default [`lane_threads`]), each other one
/// started by `spawn`.
fn argon2id(
    passphrase: &[u8],
    params: &Argon2idParams,
    limits: &Limits,
    out: &mut [u8; 32],
    threads: Option<NonZero<usize>>,
    spawn: SpawnLane,
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
    let fill = Arc::new(Mutex::new(Fill {
        params: p,
        passphrase: Zeroizing::new(passphrase.to_vec()),
        salt: params.salt,
        memory: Zeroizing::new(working),
        key: Zeroizing::new([0; 32]),
        result: None,
    }));
    fill_lanes(&fill, threads.unwrap_or_else(|| lane_threads(lanes)), spawn);
    let mut fill = lock(&fill);
    out.copy_from_slice(&fill.key[..]);
    match fill.result.take() {
        Some(hashed) => hashed.map_err(rejected),
        None => Err(ErrorCause::Argon2Rejected(
            "the derivation did not run".to_owned(),
        )),
    }
}

/// The name of the threads a derivation starts for its lanes.
const LANE_THREAD_NAME: &str = "purse-argon2";

/// Starts one of a derivation's lane threads in `scope`, running `worker`
/// (rayon's loop for that thread). A refusal comes back as the error.
/// Tests put one that refuses in place of [`spawn_lane_thread`].
type SpawnLane = for<'scope, 'env> fn(
    &'scope Scope<'scope, 'env>,
    ThreadBuilder,
) -> io::Result<ScopedJoinHandle<'scope, ()>>;

/// The [`SpawnLane`] derivations use: a named scoped thread from
/// `std::thread::Builder`, which returns the operating system's refusal as
/// an error.
fn spawn_lane_thread<'scope>(
    scope: &'scope Scope<'scope, '_>,
    worker: ThreadBuilder,
) -> io::Result<ScopedJoinHandle<'scope, ()>> {
    thread::Builder::new()
        .name(LANE_THREAD_NAME.to_owned())
        .spawn_scoped(scope, move || worker.run())
}

/// How many threads fill `lanes` lanes, the calling thread included: one
/// per lane, at most one per CPU this process may use
/// (`std::thread::available_parallelism`, which honours a cgroup CPU quota,
/// as Go's scheduler does when it sizes `GOMAXPROCS`; not [`num_cpu`], the
/// count Go stores in new parameters).
fn lane_threads(lanes: u32) -> NonZero<usize> {
    let lanes = usize::try_from(lanes).unwrap_or(usize::MAX);
    NonZero::new(lanes.min(usable_parallelism())).unwrap_or(NonZero::<usize>::MIN)
}

/// A derivation, owned: rayon runs it as a `'static` job, which cannot
/// borrow the caller's passphrase, memory or output. Every secret in it
/// is wiped when it is dropped.
struct Fill {
    params: argon2::Params,
    /// A copy of the passphrase.
    passphrase: Zeroizing<Vec<u8>>,
    salt: [u8; ARGON2ID_SALT_LEN],
    memory: Zeroizing<Vec<Block>>,
    key: Zeroizing<[u8; 32]>,
    /// `None` until the derivation has run.
    result: Option<Result<(), argon2::Error>>,
}

impl Fill {
    /// Derives the key into `self.key`, then wipes and frees the memory
    /// blocks. Every lane runs on the rayon pool of the thread this is
    /// called on.
    fn run(&mut self) {
        let ctx = Argon2::new(Algorithm::Argon2id, Version::V0x13, self.params.clone());
        let hashed = ctx.hash_password_into_with_memory(
            &self.passphrase,
            &self.salt,
            &mut self.key[..],
            self.memory.as_mut_slice(),
        );
        // Wiped once and freed here: `Zeroizing` would wipe the whole
        // allocation a second time on drop, as spare capacity.
        self.memory.zeroize();
        drop(std::mem::take(&mut *self.memory));
        self.result = Some(hashed);
    }
}

/// Locks `fill`. Only a panic inside the derivation could poison it, and
/// that panic then unwinds the caller (or, under the release profile,
/// aborts the process), so a poisoned `Fill` is only dropped, which still
/// wipes it.
fn lock(fill: &Mutex<Fill>) -> MutexGuard<'_, Fill> {
    fill.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A panic's payload, as `std::panic` hands it over.
type Panic = Box<dyn Any + Send>;

/// Runs `fill` on a rayon pool of `threads` threads made for it, so rayon's
/// global pool is never started, and returns once the pool's threads have
/// finished.
///
/// The `argon2` crate hands each slice's lanes to rayon (its `parallel`
/// feature), and rayon starts its global pool for any parallel iterator
/// run outside a pool's thread. Inside one, the iterator splits across
/// that thread's pool. The calling thread is the pool's thread 0 and
/// takes part in the work, as `std::thread::scope`'s caller would; the
/// others are scoped threads from `spawn`. If one is refused, those
/// already started finish, and the calling thread alone fills every lane.
fn fill_lanes(fill: &Arc<Mutex<Fill>>, threads: NonZero<usize>, spawn: SpawnLane) {
    if rayon::current_thread_index().is_some() {
        // Called on a thread of some rayon pool (nothing in this workspace
        // does so): rayon cannot run this thread's loop a second time, and
        // the lanes run on the pool it belongs to.
        lock(fill).run();
        return;
    }
    let job = || {
        let fill = Arc::clone(fill);
        move || lock(&fill).run()
    };
    thread::scope(|scope| {
        if run_on_pool(threads, |worker| spawn(scope, worker), job()).is_err() {
            // Nothing ran: a pool fails before taking the job. A pool of
            // the calling thread alone starts no thread and cannot fail
            // on that account; if it fails all the same, `fill` keeps no
            // result and the derivation fails.
            let _ = run_on_pool(
                NonZero::<usize>::MIN,
                |_| Err(io::Error::other("a pool of one thread starts no thread")),
                job(),
            );
        }
    });
}

/// One attempt of [`fill_lanes`]: builds a pool of `threads` threads whose
/// thread 0 is the calling thread, hands it `job`, runs thread 0's loop
/// until the job is done, and joins the threads `spawn` started.
///
/// Those threads' loops return once the job is done and the pool is
/// dropped. When `spawn` fails, rayon ends the threads already started
/// (`Registry::new` in rayon-core) and the build fails before any job is
/// taken; they are joined all the same. A panic in `job` is resumed on
/// the calling thread once they are: rayon aborts the process on a panic
/// in a spawned job unless the pool has a panic handler.
fn run_on_pool<'scope>(
    threads: NonZero<usize>,
    mut spawn: impl FnMut(ThreadBuilder) -> io::Result<ScopedJoinHandle<'scope, ()>>,
    job: impl FnOnce() + Send + 'static,
) -> Result<(), rayon::ThreadPoolBuildError> {
    let mut caller = None;
    let mut started = Vec::new();
    let panicked: Arc<Mutex<Option<Panic>>> = Arc::default();
    let handler = Arc::clone(&panicked);
    let built = ThreadPoolBuilder::new()
        .num_threads(threads.get())
        .panic_handler(move |payload| {
            *handler.lock().unwrap_or_else(PoisonError::into_inner) = Some(payload);
        })
        .spawn_handler(|worker| {
            if worker.index() == 0 {
                caller = Some(worker);
            } else {
                started.push(spawn(worker)?);
            }
            Ok(())
        })
        .build();
    if let Ok(pool) = &built {
        pool.spawn(job);
    }
    // The job holds the pool open until it has run; with this handle
    // dropped, each thread's loop returns once the job is done.
    let built = built.map(drop);
    if built.is_ok()
        && let Some(caller) = caller
    {
        caller.run();
    }
    // std::thread::scope waits only until each thread's closure has
    // returned; joining also waits for its thread-local destructors, so
    // nothing runs on it any more (the kernel may list its task a moment
    // longer). A lane thread cannot panic: rayon aborts the process if its
    // loop does, and the job's panic went to the handler.
    for thread in started {
        let _ = thread.join();
    }
    let payload = panicked
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .take();
    if let Some(payload) = payload {
        std::panic::resume_unwind(payload);
    }
    built
}

/// The number of CPUs this process may use, counted as Go's
/// `runtime.NumCPU` counts them, for [`Argon2idParams::threads_for_cpus`]
/// (decided by the project owner on 2026-11-22; ADR-0012 Rule 9(b)).
///
/// On Linux and Android, Go counts the bits of the main thread's CPU
/// affinity mask (`sched_getaffinity` at process start-up, 1 if the call
/// fails or the mask is empty) and ignores cgroup CPU quotas. This reads
/// the same mask from the `Cpus_allowed` line of `/proc/self/status`,
/// which the kernel prints for the thread group leader, the thread Go asks
/// from, and counts its bits the same way (an empty mask counts 1). It is
/// read once, at the first call, and kept for the life of the process, as
/// Go keeps the count it took at start-up; a change of the affinity mask
/// between start-up and the first call is seen here and not by Go. Where
/// `/proc` is not mounted or the line is missing, the count falls back to
/// `std::thread::available_parallelism`, which counts the calling thread's
/// mask and also honours a cgroup CPU quota, so it can be lower than Go's
/// in a container with a quota; Go's system call does not fail there. The
/// workspace forbids `unsafe` code, so the system call itself is not made.
///
/// Elsewhere the count is `available_parallelism`, which differs from Go's
/// as follows (read from both sources, not run): on Windows it is
/// `GetSystemInfo`'s processor count, where Go counts the process affinity
/// mask (`GetProcessAffinityMask`, falling back to that count), so a
/// process restricted to fewer processors stores a higher count than Go's
/// would; on Apple systems it is the online processor count
/// (`sysconf(_SC_NPROCESSORS_ONLN)`), where Go reads `hw.ncpu`, which can
/// differ only while processors are offline. FreeBSD reads the affinity
/// mask on both sides.
///
/// The count shapes new parameters, which are stored, so no existing
/// passphrase depends on it. It does not cap the threads a derivation runs
/// on, which do not change its key (that cap honours a quota, see the
/// crate documentation, "Stored parameters and resource bounds").
pub fn num_cpu() -> usize {
    static COUNT: OnceLock<usize> = OnceLock::new();
    *COUNT.get_or_init(go_num_cpu)
}

/// [`num_cpu`]'s count, measured once.
#[cfg(any(target_os = "linux", target_os = "android"))]
fn go_num_cpu() -> usize {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|status| cpus_allowed(&status))
        .unwrap_or_else(usable_parallelism)
}

/// [`num_cpu`]'s count, measured once.
#[cfg(not(any(target_os = "linux", target_os = "android")))]
fn go_num_cpu() -> usize {
    usable_parallelism()
}

/// The CPUs `std::thread::available_parallelism` counts, or 1 when it is
/// unknown.
fn usable_parallelism() -> usize {
    std::thread::available_parallelism().map_or(1, NonZero::get)
}

/// The number of CPUs in the `Cpus_allowed` mask of a `/proc/<pid>/status`
/// text (comma-separated hexadecimal words), counted as Go's
/// `getCPUCount` counts the mask `sched_getaffinity` returns: the set
/// bits, or 1 when none is set. `None` when the text has no such line or
/// the line is not a mask.
#[cfg_attr(
    not(any(target_os = "linux", target_os = "android", test)),
    allow(dead_code, reason = "called on Linux and Android, and by its tests")
)]
fn cpus_allowed(status: &str) -> Option<usize> {
    let mask = status
        .lines()
        .find_map(|l| l.strip_prefix("Cpus_allowed:"))?
        .trim();
    if mask.is_empty() {
        return None;
    }
    let mut count = 0usize;
    for word in mask.split(',') {
        if word.is_empty() {
            return None;
        }
        for c in word.chars() {
            let nibble = c.to_digit(16)?;
            count = count.saturating_add(usize::try_from(nibble.count_ones()).ok()?);
        }
    }
    Some(count.max(1))
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use purse_testutil::hex;

    use super::*;

    /// Salt 00..0f, time 2, 4 lanes of 32 KiB: the 4-lane case of
    /// `tests/lanes.rs`, whose key Go derived.
    const FOUR_LANES: Argon2idParams = Argon2idParams {
        salt: [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15],
        time: 2,
        memory: 128,
        threads: 4,
    };
    const FOUR_LANES_KEY: &str = "d2f408ed9166f224f0c4b53d21cc4309df2b552b4bfc515b817f6f4ed8503421";

    fn derive(threads: usize, spawn: SpawnLane) -> String {
        let mut out = [0; 32];
        argon2id(
            b"lanes",
            &FOUR_LANES,
            &Limits::DEFAULT,
            &mut out,
            NonZero::new(threads),
            spawn,
        )
        .expect("derive");
        hex(&out)
    }

    /// The operating system's answer at its thread limit (`EAGAIN`).
    fn refused() -> io::Error {
        io::Error::from(io::ErrorKind::WouldBlock)
    }

    fn refuse_all<'scope>(
        _: &'scope Scope<'scope, '_>,
        _: ThreadBuilder,
    ) -> io::Result<ScopedJoinHandle<'scope, ()>> {
        Err(refused())
    }

    /// Starts threads 1 and 2, then refuses thread 3.
    fn refuse_third<'scope>(
        scope: &'scope Scope<'scope, '_>,
        worker: ThreadBuilder,
    ) -> io::Result<ScopedJoinHandle<'scope, ()>> {
        if worker.index() >= 3 {
            return Err(refused());
        }
        spawn_lane_thread(scope, worker)
    }

    /// Every path gives Go's key: the pool sized by the CPU count, pools
    /// smaller and larger than the lane count, the calling thread alone,
    /// and both fallbacks, from a first thread refused and from a third.
    #[test]
    fn every_path_gives_the_same_key() {
        let mut key = [0; 32];
        argon2id(
            b"lanes",
            &FOUR_LANES,
            &Limits::DEFAULT,
            &mut key,
            None,
            spawn_lane_thread,
        )
        .expect("derive");
        assert_eq!(hex(&key), FOUR_LANES_KEY, "default pool");
        for threads in [1, 2, 3, 4, 8] {
            assert_eq!(
                derive(threads, spawn_lane_thread),
                FOUR_LANES_KEY,
                "{threads} threads"
            );
        }
        assert_eq!(
            derive(4, refuse_all),
            FOUR_LANES_KEY,
            "every thread refused"
        );
        assert_eq!(
            derive(4, refuse_third),
            FOUR_LANES_KEY,
            "third thread refused"
        );
    }

    /// The threads a counting spawner has started, and those of them that
    /// have not finished. Each test has its own, as tests run concurrently.
    struct Count {
        started: AtomicUsize,
        running: AtomicUsize,
    }

    impl Count {
        const fn new() -> Self {
            Count {
                started: AtomicUsize::new(0),
                running: AtomicUsize::new(0),
            }
        }
    }

    static POOL_COUNT: Count = Count::new();
    static CAP_COUNT: Count = Count::new();

    /// [`spawn_lane_thread`] counting in `count` the threads it starts and
    /// those that have not finished, refusing thread 3 onwards when
    /// `refuse` is set. A thread counts as finished once its last
    /// thread-local value (the guard) has been destroyed, which is the last
    /// thing it runs.
    fn counted<'scope>(
        scope: &'scope Scope<'scope, '_>,
        worker: ThreadBuilder,
        refuse: bool,
        count: &'static Count,
    ) -> io::Result<ScopedJoinHandle<'scope, ()>> {
        struct Exit(&'static AtomicUsize);
        impl Drop for Exit {
            fn drop(&mut self) {
                self.0.fetch_sub(1, Ordering::SeqCst);
            }
        }
        thread_local!(static EXIT: std::cell::Cell<Option<Exit>> = const { std::cell::Cell::new(None) });
        if refuse && worker.index() >= 3 {
            return Err(refused());
        }
        count.running.fetch_add(1, Ordering::SeqCst);
        let started = thread::Builder::new()
            .name(LANE_THREAD_NAME.to_owned())
            .spawn_scoped(scope, move || {
                EXIT.set(Some(Exit(&count.running)));
                worker.run();
            });
        match &started {
            Ok(_) => count.started.fetch_add(1, Ordering::SeqCst),
            Err(_) => count.running.fetch_sub(1, Ordering::SeqCst),
        };
        started
    }

    fn counted_all<'scope>(
        scope: &'scope Scope<'scope, '_>,
        worker: ThreadBuilder,
    ) -> io::Result<ScopedJoinHandle<'scope, ()>> {
        counted(scope, worker, false, &POOL_COUNT)
    }

    fn counted_refusing<'scope>(
        scope: &'scope Scope<'scope, '_>,
        worker: ThreadBuilder,
    ) -> io::Result<ScopedJoinHandle<'scope, ()>> {
        counted(scope, worker, true, &POOL_COUNT)
    }

    fn counted_cap<'scope>(
        scope: &'scope Scope<'scope, '_>,
        worker: ThreadBuilder,
    ) -> io::Result<ScopedJoinHandle<'scope, ()>> {
        counted(scope, worker, false, &CAP_COUNT)
    }

    /// A derivation returns only after the threads its pool started have
    /// finished, those of a pool abandoned when a thread was refused too.
    /// The calling thread is thread 0, so a pool of 4 starts 3.
    #[test]
    fn pool_threads_end_with_the_derivation() {
        assert_eq!(derive(4, counted_all), FOUR_LANES_KEY);
        assert_eq!(POOL_COUNT.started.load(Ordering::SeqCst), 3);
        assert_eq!(POOL_COUNT.running.load(Ordering::SeqCst), 0);
        // Threads 1 and 2 start, 3 is refused, and the calling thread
        // derives alone.
        assert_eq!(derive(4, counted_refusing), FOUR_LANES_KEY);
        assert_eq!(POOL_COUNT.started.load(Ordering::SeqCst), 5);
        assert_eq!(POOL_COUNT.running.load(Ordering::SeqCst), 0);
    }

    /// With more lanes than CPUs, the default pool has one thread per CPU,
    /// the calling thread among them, and the key is the one the calling
    /// thread derives alone.
    #[test]
    fn threads_are_capped_at_the_cpu_count() {
        let cpus = usable_parallelism();
        let lanes = u8::try_from(cpus.saturating_add(1)).unwrap_or(u8::MAX);
        let params = Argon2idParams {
            salt: FOUR_LANES.salt,
            time: 1,
            memory: u32::from(lanes).saturating_mul(8),
            threads: lanes,
        };
        let derive_with = |threads, spawn| {
            let mut out = [0; 32];
            argon2id(
                b"lanes",
                &params,
                &Limits::DEFAULT,
                &mut out,
                threads,
                spawn,
            )
            .expect("derive");
            out
        };
        let key = derive_with(None, counted_cap);
        assert_eq!(
            CAP_COUNT.started.load(Ordering::SeqCst),
            usize::from(lanes).min(cpus).saturating_sub(1),
            "{lanes} lanes on {cpus} CPUs"
        );
        assert_eq!(CAP_COUNT.running.load(Ordering::SeqCst), 0);
        assert_eq!(key, derive_with(NonZero::new(1), spawn_lane_thread));
    }

    /// A panic in the job reaches the calling thread once the pool's
    /// threads have been joined, rather than aborting the process, and the
    /// calling thread leaves the pool.
    #[test]
    fn a_panic_reaches_the_calling_thread() {
        let caught = std::panic::catch_unwind(|| {
            thread::scope(|scope| {
                let _ = run_on_pool(
                    NonZero::new(3).expect("3"),
                    |worker| spawn_lane_thread(scope, worker),
                    || panic!("inside the derivation"),
                );
            });
        });
        let payload = caught.expect_err("the panic was lost");
        let message = payload
            .downcast_ref::<&str>()
            .copied()
            .or_else(|| payload.downcast_ref::<String>().map(String::as_str));
        assert_eq!(message, Some("inside the derivation"));
        assert_eq!(rayon::current_thread_index(), None);
    }

    /// A derivation made on a thread of another rayon pool runs its lanes
    /// on that pool and gives the same key.
    #[test]
    fn inside_another_pool() {
        let key = ThreadPoolBuilder::new()
            .num_threads(2)
            .build_scoped(
                |worker| worker.run(),
                |pool| pool.install(|| derive(4, spawn_lane_thread)),
            )
            .expect("pool");
        assert_eq!(key, FOUR_LANES_KEY);
    }

    /// No path starts rayon's global pool: `build_global` succeeds only
    /// in a process where it has not been started. The other tests in
    /// this binary derive too, concurrently, and would trip it as well.
    #[test]
    fn no_path_starts_the_global_pool() {
        every_path_gives_the_same_key();
        calling_thread_leaves_the_pool();
        inside_another_pool();
        rayon::ThreadPoolBuilder::new()
            .num_threads(1)
            .build_global()
            .expect("rayon's global pool was already started");
    }

    /// The calling thread is a rayon worker only while its pool's loop
    /// runs: afterwards it belongs to no pool.
    #[test]
    fn calling_thread_leaves_the_pool() {
        assert_eq!(derive(1, spawn_lane_thread), FOUR_LANES_KEY);
        assert_eq!(rayon::current_thread_index(), None);
        assert_eq!(derive(4, refuse_all), FOUR_LANES_KEY);
        assert_eq!(rayon::current_thread_index(), None);
        assert_eq!(derive(4, spawn_lane_thread), FOUR_LANES_KEY);
        assert_eq!(rayon::current_thread_index(), None);
    }

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

    /// The `Cpus_allowed` line as Linux prints it (`%*pb`: 32-bit words in
    /// hexadecimal, most significant first, separated by commas), counted
    /// as Go's `getCPUCount` counts the affinity mask: set bits, at least 1.
    #[test]
    fn cpus_allowed_counts_the_mask_as_go_does() {
        let status =
            |mask: &str| format!("Name:\tx\nCpus_allowed:\t{mask}\nCpus_allowed_list:\t0\n");
        for (mask, want) in [
            ("1", Some(1)),
            ("f", Some(4)),
            ("ff", Some(8)),
            ("ffffffff", Some(32)),
            ("00000001", Some(1)),
            ("5", Some(2)),
            ("80000000,00000000", Some(1)),
            ("ffff,ffffffff,ffffffff", Some(80)),
            ("00000000,00000000", Some(1)),
            ("0", Some(1)),
            ("FfFf", Some(16)),
            ("", None),
            ("ff,,ff", None),
            ("fg", None),
            ("0x1", None),
        ] {
            assert_eq!(cpus_allowed(&status(mask)), want, "{mask:?}");
        }
        assert_eq!(cpus_allowed("Name:\tx\nCpus_allowed_list:\t0-3\n"), None);
        assert_eq!(cpus_allowed(""), None);
    }

    /// On Linux the count is the mask of this process's main thread, as
    /// `/proc/self/status` prints it, and stays the same once taken.
    #[cfg(target_os = "linux")]
    #[test]
    fn num_cpu_reads_the_affinity_mask() {
        let status = std::fs::read_to_string("/proc/self/status").expect("/proc/self/status");
        let want = cpus_allowed(&status).expect("a Cpus_allowed line");
        assert_eq!(num_cpu(), want);
        assert_eq!(num_cpu(), want);
    }
}
