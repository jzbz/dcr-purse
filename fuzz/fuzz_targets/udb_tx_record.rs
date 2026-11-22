// SPDX-License-Identifier: ISC
//! Transaction record fuzz target: `purse_udb::txstore::TxRecord::new`
//! (dcrwallet's `NewTxRecord`) on arbitrary bytes.
//!
//! The transaction store records whatever bytes its caller hands it: a
//! transaction relayed by a peer or returned by an RPC server. Nothing may
//! panic; a record keeps the caller's bytes verbatim, its hash is the
//! decoded transaction's, and serializing the decoded transaction gives a
//! prefix of the bytes (the decoder stops at the transaction's end).

#![no_main]

use libfuzzer_sys::fuzz_target;
use purse_udb::txstore::TxRecord;

fuzz_target!(|data: &[u8]| {
    if let Ok(rec) = TxRecord::new(data, 0) {
        assert_eq!(rec.serialized(), data, "the record keeps the caller's bytes");
        assert_eq!(&rec.hash().0, rec.msg_tx().tx_hash().as_bytes());
        let again = rec.msg_tx().serialize();
        assert!(data.starts_with(&again), "the decoded transaction is a prefix");
        let _ = rec.tx_type();
    }
});
