// SPDX-License-Identifier: ISC
//! Wallet store open fuzz target: a store file may be damaged outside the
//! engine (a disk fault, a bad restore), and opening it with the integrity
//! check on must return `Ok` or `Err`, never panic.
//!
//! redb documents that an open without the check trusts a file it shut
//! down cleanly and may panic on one corrupted since, while with the check
//! such a file fails to open (`Builder::set_check_integrity_on_open`). The
//! store does not turn the check on by itself: `OpenOptions` leaves it off
//! unless the caller asks (`OpenOptions::check_integrity`), and when to ask
//! (a restored backup, an open after an unclean shutdown) is for the loader
//! and `purse-udb` to decide; nothing decides it yet. This target fuzzes
//! the checked open only. The default, unchecked open is not fuzzed: redb
//! does not promise it survives a file damaged outside redb. (After an
//! unclean shutdown redb runs its own repair walk on open whatever the
//! flag says.) A store that opens is walked table by table, its meta rows
//! read through the typed accessors and its chain tables checked
//! (`ReadTx::check_chain`); the row decoders themselves are the
//! `store_decode` target's.
//!
//! Run it without debug assertions (`cargo fuzz run -O`), as CI does: redb
//! 5.0.0 has a `debug_assert!` that a damaged page number reaches
//! (`PageNumber::address_range`, `regional_start < region_size`), even with
//! the integrity check on; the release build, which the wallet ships, has
//! no such assertion and did not panic on that input.
//!
//! The first byte picks the mode. Raw bytes almost never get past redb's
//! header checksums, so the other mode starts from the checked-in
//! version-1 store and applies the input as patches (a 3-byte offset, a
//! length byte, then the bytes).

#![no_main]

use std::ops::Bound;
use std::sync::Mutex;

use libfuzzer_sys::fuzz_target;
use purse_store::{BackendError, OpenOptions, StorageBackend, TableId};

static BASE: &[u8] = include_bytes!("../../crates/purse-store/tests/data/schema-v1.redb");

/// An in-memory image.
#[derive(Debug)]
struct Image(Mutex<Vec<u8>>);

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

fn exercise(bytes: Vec<u8>) {
    let Ok(store) = OpenOptions::new()
        .check_integrity(true)
        .cache_size(1 << 20)
        .open_on_backend(Image(Mutex::new(bytes)))
    else {
        return;
    };
    let Ok(tx) = store.read() else { return };
    for &t in TableId::ALL {
        let Ok(rows) = tx.range_raw(t, Bound::Unbounded, Bound::Unbounded) else {
            continue;
        };
        for r in rows.take(10_000) {
            if r.is_err() {
                break;
            }
        }
    }
    let _ = tx.tip();
    let _ = tx.processed_marker();
    let _ = tx.unspent_count();
    let _ = tx.birth_state();
    let _ = tx.have_cfilters();
    let _ = tx.wallet_row::<purse_store::wallet::Slip0044Account0>();
    // The chain walk reads every chain table and both header fields.
    let _ = tx.check_chain();
    if let Ok(Some(tip)) = tx.tip() {
        let _ = tx.header(&tip.hash);
        let _ = tx.cfilters_in(tip.height.saturating_sub(3)..).map(|i| i.take(4).count());
    }
    drop(tx);
    let _ = store.update(|w| w.put_raw(TableId::Tickets, &[1; 32], b""));
}

fuzz_target!(|data: &[u8]| {
    let Some((&mode, rest)) = data.split_first() else {
        return;
    };
    if mode & 1 == 0 {
        exercise(rest.to_vec());
        return;
    }
    let mut image = BASE.to_vec();
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
    exercise(image);
});
