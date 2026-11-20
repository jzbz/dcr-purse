// SPDX-License-Identifier: ISC
//! Account key-material fuzz target: `purse_udb::addrmgr::AccountKeys::decode`
//! on arbitrary bytes.
//!
//! The value is read back from the wallet's own store, so only a corrupted
//! file can hand it something its encoder did not write. Nothing may panic,
//! and whatever decodes re-encodes to the same bytes.

#![no_main]

use libfuzzer_sys::fuzz_target;
use purse_udb::addrmgr::AccountKeys;

fuzz_target!(|data: &[u8]| {
    if let Ok(keys) = AccountKeys::decode(data) {
        assert_eq!(keys.encode(), data, "a decoded value re-encodes to its bytes");
    }
});
