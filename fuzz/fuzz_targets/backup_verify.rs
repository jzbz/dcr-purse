// SPDX-License-Identifier: ISC
//! Backup verify fuzz target: a backup handed to `verify` or `restore` is
//! external input (ADR-0018 Decision item 6), so the verify path must
//! return `Ok` or `Err` on any file, never panic.
//!
//! The first byte picks the mode. Even: the rest is decoded as a backup
//! record, the only bytes the verify path parses itself; a record that
//! decodes must encode to the same bytes. Odd: the rest is applied as
//! patches (a 3-byte offset, a length byte, then the bytes) to a backup of
//! the checked-in version-1 store, made once with `backup_to_backend`, and
//! the patched image is verified with `purse-udb`'s
//! `txstore::verify_backup_on_backend` (the store's verify with the
//! transaction tables' decoded checks, as the tool is to call it), which
//! opens it with redb's integrity check. An image that verifies must carry
//! the original backup record: the digest check stands between a damaged
//! row and `Ok`.
//!
//! Run it without debug assertions (`cargo fuzz run -O`), as CI does, for
//! the reason `store_open.rs` gives: redb 5.0.0 has a `debug_assert!` that
//! a damaged page number reaches even with the integrity check on, and the
//! release build, which the wallet ships, has no such assertion.

#![no_main]

use std::sync::{Arc, Mutex, OnceLock};

use libfuzzer_sys::fuzz_target;
use purse_store::{BackendError, BackupInfo, BackupRecord, Codec, OpenOptions, StorageBackend};
use purse_udb::txstore::verify_backup_on_backend;

static FIXTURE: &[u8] = include_bytes!("../../crates/purse-store/tests/data/schema-v1.redb");

/// An in-memory image the caller can read back after redb is done with it.
#[derive(Debug, Clone)]
struct Image(Arc<Mutex<Vec<u8>>>);

impl StorageBackend for Image {
    fn len(&self) -> Result<u64, BackendError> {
        Ok(self.0.lock().expect("lock").len() as u64)
    }

    fn read(&self, offset: u64, out: &mut [u8]) -> Result<(), BackendError> {
        let data = self.0.lock().expect("lock");
        let a = usize::try_from(offset).map_err(|_| BackendError::Unsupported)?;
        let src = a
            .checked_add(out.len())
            .and_then(|end| data.get(a..end))
            .ok_or_else(|| {
                BackendError::Io(std::io::Error::from(std::io::ErrorKind::UnexpectedEof))
            })?;
        out.copy_from_slice(src);
        Ok(())
    }

    fn set_len(&self, len: u64) -> Result<(), BackendError> {
        let len = usize::try_from(len).map_err(|_| BackendError::Unsupported)?;
        if len > 64 << 20 {
            return Err(BackendError::Io(std::io::Error::other("image too large")));
        }
        self.0.lock().expect("lock").resize(len, 0);
        Ok(())
    }

    fn sync_data(&self) -> Result<(), BackendError> {
        Ok(())
    }

    fn write(&self, offset: u64, src: &[u8]) -> Result<(), BackendError> {
        let mut data = self.0.lock().expect("lock");
        let a = usize::try_from(offset).map_err(|_| BackendError::Unsupported)?;
        let end = a
            .checked_add(src.len())
            .filter(|&e| e <= 64 << 20)
            .ok_or_else(|| BackendError::Io(std::io::Error::other("image too large")))?;
        if data.len() < end {
            data.resize(end, 0);
        }
        data[a..end].copy_from_slice(src);
        Ok(())
    }
}

/// A backup of the fixture and its record, made once.
fn base() -> &'static (Vec<u8>, BackupRecord) {
    static BASE: OnceLock<(Vec<u8>, BackupRecord)> = OnceLock::new();
    BASE.get_or_init(|| {
        let store = OpenOptions::new()
            .open_on_backend(Image(Arc::new(Mutex::new(FIXTURE.to_vec()))))
            .expect("open the fixture");
        let out = Image(Arc::new(Mutex::new(Vec::new())));
        let info = BackupInfo {
            network: "simnet",
            version: "fuzz",
            time: 0,
        };
        let record = store
            .backup_to_backend(out.clone(), &info)
            .expect("back up the fixture");
        let bytes = out.0.lock().expect("lock").clone();
        (bytes, record)
    })
}

fuzz_target!(|data: &[u8]| {
    let Some((&mode, rest)) = data.split_first() else {
        return;
    };
    if mode & 1 == 0 {
        if let Ok(r) = BackupRecord::decode(rest) {
            assert_eq!(r.encode(), rest, "a backup record decoded from a non-canonical encoding");
        }
        return;
    }
    let (image, record) = base();
    let mut image = image.clone();
    let mut p = rest;
    while p.len() >= 4 {
        let offset = usize::from(p[0]) << 16 | usize::from(p[1]) << 8 | usize::from(p[2]);
        let len = usize::from(p[3]);
        let Some(patch) = p.get(4..4 + len) else {
            break;
        };
        if let Some(dst) = offset
            .checked_add(len)
            .and_then(|end| image.get_mut(offset..end))
        {
            dst.copy_from_slice(patch);
        }
        p = &p[4 + len..];
    }
    if let Ok(v) = verify_backup_on_backend(Image(Arc::new(Mutex::new(image))), Some("simnet")) {
        assert_eq!(v.record.as_ref(), Some(record), "a damaged backup verified as another");
    }
});
