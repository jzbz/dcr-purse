// SPDX-License-Identifier: ISC
//! Imported-address fuzz target: `purse_udb::addrmgr::ImportedData::decode`
//! on arbitrary bytes.
//!
//! The value is read back from the wallet's own store (the data of an
//! imported key's or script's `addr_by_hash` row), so only a corrupted file
//! can hand it something its encoder did not write. Nothing may panic, and
//! whatever decodes re-encodes to the same bytes.

#![no_main]

use libfuzzer_sys::fuzz_target;
use purse_udb::addrmgr::ImportedData;

fuzz_target!(|data: &[u8]| {
    if let Ok(v) = ImportedData::decode(data) {
        assert_eq!(v.encode(), data, "a decoded value re-encodes to its bytes");
    }
});
