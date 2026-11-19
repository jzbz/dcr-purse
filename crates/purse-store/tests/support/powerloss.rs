// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! Storage that models power loss under the store, for redb 5.
//!
//! A port of dcroxide's `PowerLossBackend`
//! (`crates/dcroxide-testutil/src/powerloss.rs` at the pinned revision,
//! already on redb 5's `BackendError`), by way of the storage spike's port
//! (`spikes/store-redb/src/powerloss.rs`). It differs from dcroxide's in
//! three ways: the storage is an in-memory image rather than a file, since
//! a sweep over every storage operation of a run reopens thousands of
//! images; a cut can tear the unsynced writes instead of discarding them
//! all; and one operation can fail on its own, the power staying on
//! ([`PowerLoss::fail_once_after`]).
//!
//! A killed process leaves the page cache intact, so every byte written and
//! never synced survives it, and a store that skipped its durability step
//! passes a kill test anyway. This backend discards what was never synced
//! (or, torn, a part of it chosen at 4 KiB granularity, within what redb
//! assumes of the media: a write changes no byte outside its range and a
//! single byte is written atomically, redb `docs/design.md`, "Assumptions
//! about underlying media"), which is what tells a durable commit from a
//! merely written one.
//!
//! `honest_sync = false` acknowledges `sync_data` without making anything
//! durable, the failure a sweep must notice: a suite that passes against it
//! proves nothing about durability.
//!
//! [`PowerLoss::power_fails_after`] places the cut inside a commit rather
//! than between two: the backend accepts a given number of further writes,
//! length changes and syncs, then refuses every later one, so nothing after
//! the cut reaches the image and no sync after it succeeds. Sweeping that
//! number over a scripted run puts the cut at every storage operation the
//! run makes.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use purse_testutil::SplitMix64;
use redb::{BackendError, StorageBackend};

/// No cut scheduled.
const NEVER: u64 = u64::MAX;

/// The granularity at which a torn cut keeps or undoes an unsynced write.
const CHUNK: usize = 4096;

/// What a cut does to the writes since the last honest sync.
#[derive(Clone, Copy, Debug)]
pub enum Cut {
    /// Power loss: every unsynced write is undone and the length goes back
    /// to the durable one.
    Clean,
    /// Power loss with torn writes: each 4 KiB piece of each unsynced write
    /// is kept or undone at random (seeded), and the length is the durable
    /// or the current one at random.
    Torn(u64),
    /// A killed process: the page cache survives, so every write is kept,
    /// synced or not.
    Kept,
}

#[derive(Debug, Default)]
struct Inner {
    data: Vec<u8>,
    /// `(offset, previous bytes)` for each write and shrink since the last
    /// honest sync, oldest first; replayed in reverse to undo them.
    undo: Vec<(usize, Vec<u8>)>,
    /// The length as of the last honest sync.
    durable_len: usize,
}

/// Storage that models power loss (see the module documentation).
#[derive(Debug)]
pub struct PowerLoss {
    inner: Mutex<Inner>,
    honest_sync: bool,
    /// Writes, length changes and syncs accepted so far.
    ops: AtomicU64,
    /// Honest syncs so far.
    syncs: AtomicU64,
    /// The operation count at which the power fails.
    fail_at: AtomicU64,
    /// Set once an operation was refused for want of power.
    refused: AtomicBool,
    /// The operation count at which one operation fails alone.
    fail_once: AtomicU64,
}

impl PowerLoss {
    /// A backend holding `image`, all of it durable.
    pub fn new(image: Vec<u8>, honest_sync: bool) -> Arc<PowerLoss> {
        let durable_len = image.len();
        Arc::new(PowerLoss {
            inner: Mutex::new(Inner {
                data: image,
                undo: Vec::new(),
                durable_len,
            }),
            honest_sync,
            ops: AtomicU64::new(0),
            syncs: AtomicU64::new(0),
            fail_at: AtomicU64::new(NEVER),
            refused: AtomicBool::new(false),
            fail_once: AtomicU64::new(NEVER),
        })
    }

    /// Lets `n` more writes, length changes and syncs through, then fails
    /// every later one: the power goes out after the `n`th. `n = 0` fails
    /// the very next one.
    pub fn power_fails_after(&self, n: u64) {
        let at = self.ops.load(Ordering::SeqCst).saturating_add(n);
        self.fail_at.store(at, Ordering::SeqCst);
    }

    /// Lets `n` more writes, length changes and syncs through, fails the
    /// next one with an I/O error ("no space", say) and lets every later
    /// one through again: a transient storage error with the power on.
    /// The failed operation changes nothing and is not counted.
    pub fn fail_once_after(&self, n: u64) {
        let at = self.ops.load(Ordering::SeqCst).saturating_add(n);
        self.fail_once.store(at, Ordering::SeqCst);
    }

    /// Whether a scheduled cut refused an operation. A run that made
    /// exactly `n` operations reached the cut without feeling it.
    pub fn power_failed(&self) -> bool {
        self.refused.load(Ordering::SeqCst)
    }

    /// Writes, length changes and syncs accepted so far: the difference
    /// across a scripted run is how many places a cut can go.
    pub fn ops(&self) -> u64 {
        self.ops.load(Ordering::SeqCst)
    }

    /// Honest syncs so far.
    pub fn syncs(&self) -> u64 {
        self.syncs.load(Ordering::SeqCst)
    }

    /// The current contents, as a clean shutdown leaves them.
    pub fn image(&self) -> Vec<u8> {
        self.inner.lock().expect("lock").data.clone()
    }

    /// The image a cut of kind `how` leaves (see [`Cut`]).
    pub fn cut(&self, how: Cut) -> Vec<u8> {
        let inner = self.inner.lock().expect("lock");
        let mut data = inner.data.clone();
        let mut rng = match how {
            Cut::Kept => return data,
            Cut::Clean => None,
            Cut::Torn(seed) => Some(SplitMix64(seed)),
        };
        // Ranges whose newer bytes were kept: an older write's undo must
        // not overwrite them.
        let mut kept: Vec<(usize, usize)> = Vec::new();
        for (offset, previous) in inner.undo.iter().rev() {
            let mut at = 0;
            while at < previous.len() {
                let end = (at + CHUNK).min(previous.len());
                let (a, b) = (offset + at, offset + end);
                let keep = rng.as_mut().is_some_and(|r| r.next_u64() & 1 == 1);
                if keep {
                    kept.push((a, b));
                } else {
                    restore(&mut data, a, &previous[at..end], &kept);
                }
                at = end;
            }
        }
        let len = if rng.as_mut().is_some_and(|r| r.next_u64() & 1 == 1) {
            inner.data.len()
        } else {
            inner.durable_len
        };
        data.resize(len, 0);
        data
    }

    fn admit(&self) -> Result<(), BackendError> {
        let ops = self.ops.load(Ordering::SeqCst);
        if self
            .fail_once
            .compare_exchange(ops, NEVER, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            return Err(BackendError::Io(std::io::Error::other(
                "transient failure (PowerLoss)",
            )));
        }
        let fail_at = self.fail_at.load(Ordering::SeqCst);
        let admitted = self
            .ops
            .try_update(Ordering::SeqCst, Ordering::SeqCst, |ops| {
                (ops < fail_at).then(|| ops.saturating_add(1))
            });
        match admitted {
            Ok(_) => Ok(()),
            Err(_) => {
                self.refused.store(true, Ordering::SeqCst);
                Err(BackendError::Io(std::io::Error::other(
                    "power cut (PowerLoss)",
                )))
            }
        }
    }
}

/// Writes `bytes` at `a`, except where a kept range covers them.
fn restore(data: &mut Vec<u8>, a: usize, bytes: &[u8], kept: &[(usize, usize)]) {
    if data.len() < a + bytes.len() {
        data.resize(a + bytes.len(), 0);
    }
    if kept.is_empty() {
        data[a..a + bytes.len()].copy_from_slice(bytes);
        return;
    }
    for (i, &byte) in bytes.iter().enumerate() {
        let p = a + i;
        if !kept.iter().any(|&(x, y)| x <= p && p < y) {
            data[p] = byte;
        }
    }
}

/// A shared handle, so the test keeps the backend after redb takes it.
#[derive(Debug, Clone)]
pub struct Shared(pub Arc<PowerLoss>);

impl StorageBackend for Shared {
    fn len(&self) -> Result<u64, BackendError> {
        Ok(self.0.inner.lock().expect("lock").data.len() as u64)
    }

    fn read(&self, offset: u64, out: &mut [u8]) -> Result<(), BackendError> {
        let inner = self.0.inner.lock().expect("lock");
        let a = usize::try_from(offset).map_err(|_| BackendError::Unsupported)?;
        match a
            .checked_add(out.len())
            .and_then(|end| inner.data.get(a..end))
        {
            Some(src) => {
                out.copy_from_slice(src);
                Ok(())
            }
            None => Err(BackendError::Io(std::io::Error::from(
                std::io::ErrorKind::UnexpectedEof,
            ))),
        }
    }

    fn set_len(&self, len: u64) -> Result<(), BackendError> {
        self.0.admit()?;
        let len = usize::try_from(len).map_err(|_| BackendError::Unsupported)?;
        let mut inner = self.0.inner.lock().expect("lock");
        if len < inner.data.len() {
            let tail = inner.data[len..].to_vec();
            inner.undo.push((len, tail));
        }
        // Growth needs no undo: the cut goes back to the durable length,
        // and new positions read as zero, as the trait requires.
        inner.data.resize(len, 0);
        Ok(())
    }

    fn sync_data(&self) -> Result<(), BackendError> {
        self.0.admit()?;
        if !self.0.honest_sync {
            // Acknowledged, nothing made durable: the failure under test.
            return Ok(());
        }
        let mut inner = self.0.inner.lock().expect("lock");
        inner.undo.clear();
        inner.durable_len = inner.data.len();
        self.0.syncs.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    fn write(&self, offset: u64, data: &[u8]) -> Result<(), BackendError> {
        self.0.admit()?;
        let a = usize::try_from(offset).map_err(|_| BackendError::Unsupported)?;
        let mut inner = self.0.inner.lock().expect("lock");
        if inner.data.len() < a + data.len() {
            inner.data.resize(a + data.len(), 0);
        }
        // The bytes past the old end were zeros; restoring them is harmless,
        // and the cut's length removes them anyway.
        let previous = inner.data[a..a + data.len()].to_vec();
        inner.undo.push((a, previous));
        inner.data[a..a + data.len()].copy_from_slice(data);
        Ok(())
    }
}
