// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! A current-version (26) wallet database the oracle wrote through udb's
//! public API, frozen in `tests/data` (see `generated_db.txt`): both
//! passphrase schemes end to end, a second account with its own passphrase,
//! an imported voting account, an imported private key and an imported
//! script. Every sealed value is opened, checked against what Go's API
//! reported, and reproduced byte for byte from the stored nonce; wrong
//! passphrases give the errors Go's API gave. The live counterpart, on a
//! database written for the run, is `generated_database_against_go` in
//! `tests/oracle.rs`.

mod common;

use common::*;
use purse_testutil::hex;
use sha2::{Digest, Sha256};

#[test]
fn frozen_generated_database() {
    let line = read_generated_expect();
    let bytes = gunzip(&data_dir().join(GENERATED_DB));
    assert_eq!(
        line["db_sha256"].as_str(),
        Some(hex(&Sha256::digest(&bytes)).as_str()),
        "{GENERATED_DB} is not the database the expectations were recorded from"
    );
    let walk = check_generated_db(bytes, &line["request"], &line["result"]);
    assert_eq!(walk.accounts.len(), 3);
}
