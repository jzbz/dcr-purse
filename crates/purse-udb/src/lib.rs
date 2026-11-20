// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! dcrwallet's `wallet/udb` behaviour over dcr-purse's own store
//! (`purse-store`, ADR-0003), and the Rust producer of the logical export
//! (ADR-0001 section 11).
//!
//! This is the crate the brief's crate plan names for Phase 3 (brief
//! section 5, `purse-udb`: the address manager and transaction store
//! logic). It ports behaviour, not layout: dcr-purse never reads or writes
//! a dcrwallet `wallet.db` (ADR-0013), so what must match Go is what the
//! API returns and what the logical export shows, never the bytes of a
//! bbolt bucket (brief section 3, W2).
//!
//! It holds the address manager's creation from a seed, its opening, the
//! wallet lock, the coin type (its private key and the SLIP0044 upgrade),
//! BIP0044 and imported xpub accounts, addresses (derivation and
//! recording, the used and returned indexes, lookups, private keys and
//! redeem scripts), imported keys and scripts, and `Encrypt`/`Decrypt`;
//! watching-only wallets, voting accounts, per-account passphrases,
//! passphrase changes and the whole transaction store are later slices. The
//! table below is the complete inventory of the address manager's Go entry
//! points at the parity pin (dcrwallet `54b85b33`; spec 02 sections 3.3
//! and 3.4), each with its status.
//!
//! # Go entry points
//!
//! "Ported" is this crate as it stands; "later" names what is not written.
//! Every "ported" row is checked against Go through the oracle (see "How
//! it is verified").
//!
//! | Go (`wallet/udb`) | Rust | Status |
//! |---|---|---|
//! | `Initialize` (address manager part: `createAddressManager`) | [`Manager::create`] | ported |
//! | `InitializeWatchOnly` (`createWatchOnly`) | — | later |
//! | `Open` (address manager part: `loadManager`) | [`Manager::open`] | ported |
//! | `HDKeysFromSeed`, `CoinTypes` | [`addrmgr::hd_keys_from_seed`], [`addrmgr::coin_types`] | ported |
//! | `ValidateAccountName`, `IsImportedVoting` | [`addrmgr::validate_account_name`], [`addrmgr::is_imported_voting`] | ported |
//! | `Manager.WatchingOnly`, `ChainParams`, `IsLocked` | [`Manager::watching_only`], [`Manager::chain_params`], [`Manager::is_locked`] | ported |
//! | `Manager.Lock`, `Unlock`, `UnlockedWithPassphrase` | [`Manager::lock`], [`Manager::unlock`], [`Manager::unlocked_with_passphrase`] | ported |
//! | `Manager.CoinType` | [`Manager::coin_type`] | ported |
//! | `Manager.LastAccount`, `LastImportedAccount`, `AccountName`, `ForEachAccount` | [`Manager::last_account`], [`Manager::last_imported_account`], [`Manager::account_name`], [`Manager::for_each_account`] | ported |
//! | `Manager.AccountProperties`, `AccountHasPassphrase` | [`Manager::account_properties`], [`Manager::account_has_passphrase`] | ported (accounts without their own passphrase, which no ported operation can give an account) |
//! | `Manager.AccountExtendedPubKey`, `AccountExtendedPrivKey`, `AccountBranchExtendedPubKey` | [`Manager::account_extended_pub_key`], [`Manager::account_extended_priv_key`], [`Manager::account_branch_extended_pub_key`] | ported |
//! | `Manager.Encrypt`, `Decrypt` (`CKTPublic`, `CKTPrivate`) | [`Manager::encrypt_public`], [`Manager::encrypt_private`], [`Manager::decrypt_public`], [`Manager::decrypt_private`] | ported (one method per key: Go's invalid key type cannot be passed) |
//! | `Manager.Close` | dropping the [`Manager`] (keys are wiped on drop) | ported (as drop; `rt.reopen` drops and opens) |
//! | `Manager.CoinTypePrivKey`, `UpgradeToSLIP0044CoinType` | [`Manager::coin_type_priv_key`], [`Manager::upgrade_to_slip0044_coin_type`] | ported (the upgrade writes Go's zero counters, QK-0027, drops account 0's cached entry instead of replacing its keys before the commit, QK-0043, and derives account 0's recorded address rows again from the SLIP0044 key where Go keeps the legacy rows, QK-0760) |
//! | `Manager.NewAccount`, `RenameAccount`, `LookupAccount` | [`Manager::new_account`], [`Manager::rename_account`], [`Manager::lookup_account`] | ported |
//! | `Manager.ChangePassphrase`, `ConvertToWatchingOnly` | — | later |
//! | `Manager.ImportXpubAccount` | [`Manager::import_xpub_account`] | ported |
//! | `Manager.ImportVotingAccount` | — | later (voting accounts: the Argon2id-sealed private key and the per-account passphrase) |
//! | `Manager.SetAccountPassphrase`, `UnlockAccount`, `LockAccount` | — | later (per-account passphrase rows; the key-material layout already carries their Argon2id parameters) |
//! | `Manager.Address`, `AddrAccount`, `ExistsHash160` | [`Manager::address`], [`Manager::addr_account`], [`Manager::exists_hash160`] | ported |
//! | `Manager.ImportPrivateKey`, `ImportPublicKey`, `ImportScript` | [`Manager::import_private_key`], [`Manager::import_public_key`], [`Manager::import_script`] | ported (`ImportPublicKey` only refuses here, since it needs a watching-only wallet) |
//! | `Manager.PrivateKey`, `HavePrivateKey`, `RedeemScript` | [`Manager::private_key`], [`Manager::have_private_key`], [`Manager::redeem_script`] | ported (`redeemScriptForHash160` is the body of `redeem_script`; its one other caller is the transaction store's) |
//! | `Manager.SyncAccountToAddrIndex`, `RecordDerivedAddress` | [`Manager::sync_account_to_addr_index`], [`Manager::record_derived_address`] | ported (`RecordDerivedAddress` refuses a branch other than 0 or 1, QK-0759) |
//! | `Manager.MarkUsed`, `MarkUsedChildIndex`, `MarkReturnedChildIndex` | [`Manager::mark_used`], [`Manager::mark_used_child_index`], [`Manager::mark_returned_child_index`] | ported (an account with no row is an `IO` error, where Go panics, QK-0039) |
//! | `Manager.ForEachAccountAddress`, `ForEachActiveAccountAddress`, `ForEachActiveAddress` | [`Manager::for_each_account_address`], [`Manager::for_each_active_account_address`], [`Manager::for_each_active_address`] | ported |
//! | `Wallet.UpgradeToSLIP0044CoinType` (`wallet/cointype.go`) | — | later (wallet crate) |
//! | `Migrate`, `upgradeManager`, the 25 database upgrades | — | not ported: no dcrwallet database is read (ADR-0013); a new wallet is written in its final form |
//! | `ObtainUserInputFunc`, `Manager.closed` | — | not ported: unused at the pin (spec 02 quirk 22) |
//!
//! The transaction store (`Store`, specs 03 and 04) and the aux buckets
//! (spec 05: VSP, agenda and treasury records) are later slices of this
//! crate.
//!
//! # Store rows
//!
//! dcrwallet's `waddrmgr` bucket tree (spec 02 section 4) against the
//! `purse-store` tables this crate uses. No schema addition is needed:
//! schema version 1 already has every row the ported operations write.
//!
//! | dcrwallet (bbolt) | purse-store | Notes |
//! |---|---|---|
//! | `main/mpub`, `main/mpriv` | `wallet` rows `master_pub`, `master_priv` ([`purse_store::MasterKeyParams`]) | the 88-byte snacl parameters, typed |
//! | `main/cpub`, `main/cpriv` | `wallet` rows `crypto_pub`, `crypto_priv` | the sealed crypto keys, verbatim |
//! | `main/ctpub`, `ctpriv`, `ctpub-slip0044`, `ctpriv-slip0044` | `wallet` rows `cointype_legacy`, `cointype_slip0044` ([`purse_store::CoinTypeKeys`]) | each half optional, as in dcrwallet; the legacy row's public half decides the active coin type |
//! | `main/slip0044acct0` | `wallet` row `slip0044_acct0` ([`purse_store::Slip0044Acct0Row`]) | the two version-1 counters (0 at creation) kept unconverted, for the coin type upgrade (spec 02 quirk 1) |
//! | `main/watchonly` | `wallet` row `watch_only` | |
//! | `meta/lastaccount`, `meta/lastimportedaccount` | `wallet` rows `last_account`, `last_imported` | an absent `last_imported` reads as 0x7FFFFFFF, as `fetchLastImportedAccount` reads it |
//! | `acct/<n>` (type, sealed xpub, sealed xpriv) and `acctvars/<n>/` (`extused`, `intused`, `extret`, `intret`, `name`, `kdf-params`) and `acctididx/<n>` | `accounts` row `<n LE>` ([`purse_store::AccountRow`]): type, the four counters, the name, and the key material ([`addrmgr::AccountKeys`]: sealed xpub, sealed xpriv, optional Argon2id parameters) | the name is stored once by number here (the row) where dcrwallet keeps two copies (`acctvars`, `acctididx`); with the name index below, two copies against dcrwallet's three (ADR-0001 section 11 allows rendering the three from these); the private key's scheme and the parameters live in one value, so they change together |
//! | `acctnameidx` | `account_names` (name → account number) | the second copy of the name, as the key, as in dcrwallet |
//! | `addr` (key SHA-256 of the HASH160: type, account, add time, raw data) | `addr_by_hash` row `<HASH160>` ([`purse_store::AddrRow`]): a derived row's account, branch and index; an imported row's account and data ([`addrmgr::ImportedData`]: a key's sealed public and private keys, or a script's sealed HASH160 and the script in the clear) | no add time: written by dcrwallet, read by nothing (spec 02 section 10 item 22); the export renders it `0`. dcrwallet's bucket order (SHA-256 of the HASH160) is recovered by sorting where it reaches an output |
//! | `addracctidx/<hash>` (flat) | — | the account of a row is in the row, and dcrwallet repoints the flat entry on every insert, so the two never differ |
//! | `addracctidx/<account>/` (per account) | `addr_by_acct_sha` row `<account LE> ‖ <SHA-256 of the HASH160>` → HASH160 | the key is dcrwallet's nested key, so a prefix walk is `ForEachAccountAddress` order; an entry left by a row recorded again under another account stays, as in dcrwallet (QK-0757) |
//! | — | `addr_by_path` | not written: no operation reads it (ADR-0003's gap-limit use belongs to the wallet crate); the export refuses a store where it is not empty |
//! | `main/mgrver`, `main/mgrcreated`, `meta/addrpool*`, `usedaddrs` | — | not kept: dead or write-only at the pin (ADR-0001 section 9; ADR-0003, addendum "Schema version 1 as built") |
//!
//! # Locking and secrets
//!
//! Concurrency follows ADR-0008: a synchronous API, no runtime. A
//! [`Manager`] is shared by reference; its in-memory state (lock state,
//! account cache, keys) is behind one `RwLock`, always taken after the
//! caller's store transaction, never the reverse (Go's order: transaction,
//! then `Manager.mtx`; spec 02 section 7). Methods whose Go version takes
//! the read lock but inserts into the cache (`AccountProperties`,
//! `AccountHasPassphrase`) take the write lock. Every method that touches
//! the store takes the caller's transaction ([`addrmgr::StoreRead`] for
//! reads, `purse_store::WriteTx` for writes), as Go's take a bucket, so the
//! manager never begins a transaction and its writes commit or roll back
//! with the caller's.
//!
//! What is held decrypted, and when (purse-snacl's "Secrets" section
//! applies to every key type named here):
//!
//! | Secret | Held | Wiped |
//! |---|---|---|
//! | public master key (scrypt-derived) | from [`Manager::open`] until drop | on drop |
//! | public crypto key | from open until drop | on drop |
//! | account extended public keys | in the cache from first use until drop | on drop |
//! | private master key's derived key | from a successful [`Manager::unlock`] until lock | zeroed by lock and on drop |
//! | private crypto key | while unlocked | dropped (wiped) by lock |
//! | account extended private keys | in the cache while unlocked | dropped (wiped) by lock, every account included |
//! | passphrase hash and hasher key | hash while unlocked, key from open | hash dropped by lock; both wiped on drop |
//! | seed, coin type and account keys during [`Manager::create`] | for the call only | on return, success or error |
//! | the coin type private key and the new account's private key during [`Manager::new_account`] | for the call only | on return, success or error |
//! | the coin type private key from [`Manager::coin_type_priv_key`] | by the caller | when the caller drops it ([`purse_hdkeychain::ExtendedKey`] wipes itself); the decrypted string on return |
//! | account 0's new private key during [`Manager::upgrade_to_slip0044_coin_type`] (opened to check it, as Go opens it to cache it) | for the call only | on return; the dropped cache entry's keys on drop |
//! | the plaintext of [`Manager::decrypt_private`] | by the caller | when the caller drops it (`purse_snacl::SecretBytes` wipes itself) |
//! | a derived address's or imported key's private key from [`Manager::private_key`] | by the caller | when the caller drops it ([`addrmgr::ReducedPrivateKey`] wipes itself); the decrypted imported key bytes on return |
//! | a WIF's private key in [`Manager::import_private_key`] | borrowed from the caller, sealed, never copied | left to the caller (Go zeroes the caller's WIF, QK-0040) |
//!
//! Methods that return keys return owned copies ([`purse_hdkeychain::ExtendedKey`]
//! wipes itself on drop), where Go returns the cached pointers, so no
//! caller can zero or keep alive the cache's keys (spec 02 section 12
//! hazard 6). Passphrases are borrowed and left to the caller. The
//! passphrase hasher is HMAC-SHA256 under a 32-byte key drawn at open, in
//! place of Go's keyed BLAKE2b-256: it is memory-only, so any keyed PRF
//! serves (spec 02 section 12 hazard 3); the draw is Go's size, 32 bytes.
//!
//! What the dependencies free unwiped (scrypt's tables and buffers, the
//! HMAC and PBKDF2 stack copies, k256's internal copies, stack copies the
//! compiler makes when values move) is listed in the "Secrets" sections of
//! `purse-snacl` and `purse-hdkeychain` and applies here unchanged; this
//! crate adds no buffer of its own that holds a secret unwiped.
//!
//! Memory stays consistent with the store: dcr-purse does not reproduce the
//! in-memory state Go keeps after a rolled-back transaction (ADR-0002,
//! decision item 2; QK-0733). The account cache holds only what never
//! changes once written (keys and type), filled only from read
//! transactions, never from a write transaction that may still roll back;
//! an account's name, which `RenameAccount` changes, is read from its row
//! wherever Go reads its cached copy.
//!
//! # Randomness
//!
//! Every draw comes from a caller-supplied `purse_snacl::RandomSource`, in
//! Go's order and sizes: creation draws the two master key salts and the
//! two crypto keys (32 bytes each) and one 24-byte nonce per sealed value;
//! open draws the 32-byte hasher key; after open, each sealed value (an
//! account's keys, an imported key's two halves, a script's HASH160, an
//! [`Manager::encrypt_public`] or [`Manager::encrypt_private`] call) draws
//! its 24-byte nonce from the source the manager keeps. Go's
//! `Initialize` also runs its upgrades, which this crate does not, so the
//! draw streams of the two sides are not yet aligned (ADR-0002 open item
//! 2): comparisons use the masked profile.
//!
//! # The oracle operation scripts
//!
//! The operation-script format is ADR-0002's (`purse-opscript 1`), and the
//! Go side is the existing oracle `tools/oracle/cmd/oprunner`, which runs
//! dcrwallet's own udb on a bbolt database with injected randomness, clock
//! and processor count, and writes `results.jsonl` and a logical export
//! per checkpoint. A script is one JSON object:
//!
//! ```json
//! {"format": "purse-opscript 1", "name": "example",
//!  "setup": {"network": "simnet", "seed": "<hex>", "pubpass": "public",
//!            "privpass": "private", "rand": "<64 hex>", "clock": 1794990000},
//!  "ops": [{"op": "mgr.unlock", "pass": "private"},
//!          {"op": "mgr.account_extended_priv_key", "account": 0},
//!          {"op": "rt.reopen"}]}
//! ```
//!
//! and each operation's result is `{"i", "op", "ok"}` or `{"i", "op",
//! "err": {"kind", "text"}}`, the text rendered with the daemon's
//! separator `":: "`. The test runner of this crate
//! (`tests/support/runner.rs`) reads the same scripts and writes the same
//! records over [`Manager`] and `purse-store`; it implements the ported
//! operations (setup with a seed; `udb.coin_types`, `udb.hd_keys_from_seed`,
//! `udb.validate_account_name`, `udb.is_imported_voting`; every `mgr.`
//! operation of the "ported" rows above; `rt.reopen`, `rt.clock_set`,
//! `rt.clock_advance`), with Go's address reference forms and import
//! labels, and refuses every other operation, `update` and `view` groups
//! and `rt.checkpoint` included, as not yet ported, so a script can never
//! pass by skipping one.
//!
//! # How it is verified
//!
//! - `tests/oracle.rs`: the live differential. Each script runs through
//!   Go's `oprunner` and through the Rust runner; the per-operation results
//!   must be equal (value, or error kind and text; `~r` values, which
//!   follow each side's random stream, by length), and the two final
//!   exports must be equal, section by section, under the masked
//!   observable profile (ADR-0001 section 8) for every section [`export()`]
//!   covers, with every `[privcheck]` verdict `yes`. Hand-written scripts
//!   cover each operation's paths; seeded generated scripts
//!   (`tests/support/generator.rs`) vary network (all four), coin type
//!   (half upgrade to SLIP0044 first), seed, passphrases, operation order
//!   and account, address and import arguments (`PURSE_TEST_SEED`,
//!   `PURSE_UDB_SCRIPTS`); a search finds seeds whose purpose or coin type
//!   key is stored stripped (QK-0028) and compares those, account 1
//!   included; a setup Go refuses fails both runs with the same text; the
//!   recorded divergences (QK-0039, QK-0759, QK-0760) are pinned on both
//!   sides, and the generated scripts avoid exactly their inputs.
//! - `tests/frozen.rs`: Go's results and exports for the hand-written
//!   scripts and for a fixed set of 48 generated scripts (every network,
//!   both coin types), checked in with counts and hashes, so the comparison
//!   runs without Go.
//! - `tests/upstream.rs`: the ported upstream tests the crate reaches
//!   (`TestCoinTypes` and `TestCoinTypeUpgrade` of `cointype_test.go`;
//!   `testManagerAPI`, `TestManagerImports` and `TestManagerEncryptDecrypt`
//!   of `addressmanager_test.go` on testnet3 with upstream's seed,
//!   passphrases and import vectors; `TestAddresses`' address vectors).
//! - `tests/properties.rs`: create, open and lock invariants over random
//!   seeds and passphrases, the two codecs, the counters against a model of
//!   Go's rules, the order and content of the address lists, the account
//!   cache across rolled-back transactions and the coin type upgrade, the
//!   upgrade leaving no address signed for with another address's key
//!   across a restart (QK-0760), `Unlock`'s choice among failing accounts
//!   (QK-0755),
//!   derivation against `purse-hdkeychain` on every network and coin type,
//!   and a manager opened again answering every read as the live one did
//!   after a generated script. A unit test in `src/addrmgr/mod.rs` checks
//!   that a locked manager holds no private key, crypto key, passphrase
//!   hash or master key through random sequences.
//! - `tests/export.rs`: the export parses under `purse-export-check`, its
//!   registry equals the parser's, it refuses a store holding rows it
//!   does not export, and its options' `Debug` hides the passphrases.

#![forbid(unsafe_code)]

pub mod addrmgr;
pub mod export;

pub use addrmgr::Manager;
pub use export::export;
