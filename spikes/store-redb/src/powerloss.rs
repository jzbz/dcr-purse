//! Storage that models power loss under the store, for redb 5.
//!
//! A port of `dcroxide-testutil`'s `PowerLossBackend` (dcroxide
//! `crates/dcroxide-testutil/src/powerloss.rs`), which at the revision this
//! spike was written against (`5c6742f`) implemented redb 4's
//! `StorageBackend` with `std::io::Error` results and did not build against
//! redb 5, whose trait returns `BackendError` (dcroxide moved it to redb 5
//! at `f4b3d2e`). Two changes beyond
//! the port: the storage is an in-memory image rather than a file (a sweep
//! over every storage operation of a run reopens thousands of images), and
//! a cut can tear the unsynced writes instead of discarding them all.
//!
//! A process kill leaves the page cache intact, so every byte written and
//! never synced survives it, and a store that skipped its durability step
//! passes a kill test anyway. This backend discards what was never synced
//! (or, torn, a part of it chosen at 4 KiB granularity, which is within
//! what redb assumes of the media: single-byte writes are atomic and a
//! write changes no byte outside its range, redb `docs/design.md`,
//! "Assumptions about underlying media"), which is what tells a durable
//! commit from a merely written one.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use purse_testutil::SplitMix64;
use redb::{BackendError, StorageBackend};

const NEVER: u64 = u64::MAX;
const CHUNK: usize = 4096;

#[derive(Debug, Default)]
struct Inner {
    data: Vec<u8>,
    /// `(offset, previous bytes)` for each write and shrink since the last
    /// honest sync, oldest first.
    undo: Vec<(usize, Vec<u8>)>,
    durable_len: usize,
}

/// Storage that models power loss (see the module documentation).
#[derive(Debug)]
pub struct PowerLoss {
    inner: Mutex<Inner>,
    /// With false, `sync_data` is acknowledged without making anything
    /// durable: the failure a suite must detect.
    honest_sync: bool,
    ops: AtomicU64,
    syncs: AtomicU64,
    fail_at: AtomicU64,
    refused: AtomicBool,
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
        })
    }

    /// Lets `n` more writes, length changes and syncs through, then fails
    /// every later one: the power goes out after the `n`th.
    pub fn power_fails_after(&self, n: u64) {
        let at = self.ops.load(Ordering::SeqCst).saturating_add(n);
        self.fail_at.store(at, Ordering::SeqCst);
    }

    /// Whether a scheduled cut refused an operation.
    pub fn power_failed(&self) -> bool {
        self.refused.load(Ordering::SeqCst)
    }

    /// Writes, length changes and syncs accepted so far.
    pub fn ops(&self) -> u64 {
        self.ops.load(Ordering::SeqCst)
    }

    /// Honest syncs so far.
    pub fn syncs(&self) -> u64 {
        self.syncs.load(Ordering::SeqCst)
    }

    /// The current contents, as a clean shutdown would leave them.
    pub fn image(&self) -> Vec<u8> {
        self.inner.lock().expect("lock").data.clone()
    }

    /// The image a power cut leaves. Without `torn`, every write since the
    /// last honest sync is undone and the length goes back to the durable
    /// one. With `torn = Some(seed)`, each 4 KiB piece of each unsynced
    /// write is kept or undone at random, and the length is the durable or
    /// the current one at random.
    pub fn cut(&self, torn: Option<u64>) -> Vec<u8> {
        let inner = self.inner.lock().expect("lock");
        let mut data = inner.data.clone();
        let mut rng = torn.map(SplitMix64);
        // Ranges whose newer bytes were kept; an older write's undo must
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
            data.len()
        } else {
            inner.durable_len
        };
        data.resize(len, 0);
        data
    }

    fn admit(&self) -> Result<(), BackendError> {
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
                Err(BackendError::Io(std::io::Error::other("power cut")))
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
#[derive(Debug)]
pub struct Shared(pub Arc<PowerLoss>);

impl StorageBackend for Shared {
    fn len(&self) -> Result<u64, BackendError> {
        Ok(self.0.inner.lock().expect("lock").data.len() as u64)
    }

    fn read(&self, offset: u64, out: &mut [u8]) -> Result<(), BackendError> {
        let inner = self.0.inner.lock().expect("lock");
        let a = usize::try_from(offset).map_err(|_| BackendError::Unsupported)?;
        match inner.data.get(a..a + out.len()) {
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
        inner.data.resize(len, 0);
        Ok(())
    }

    fn sync_data(&self) -> Result<(), BackendError> {
        self.0.admit()?;
        if !self.0.honest_sync {
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
        let previous = inner.data[a..a + data.len()].to_vec();
        inner.undo.push((a, previous));
        inner.data[a..a + data.len()].copy_from_slice(data);
        Ok(())
    }
}
