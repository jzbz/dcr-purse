// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! Backup of a running store, and verification of a backup handed in from
//! outside: the part of ADR-0018 that belongs to `purse-store`.
//!
//! # A backup
//!
//! [`Store::backup`] writes a consistent copy of the whole store at one
//! instant without stopping the wallet (ADR-0018 Decision items 1 to 3):
//!
//! 1. The name is checked ([`check_backup_name`]) and the backup lock
//!    taken: one backup runs at a time, a second is refused
//!    ([`BACKUP_RUNNING_TEXT`]). The backups directory is created if it is
//!    missing, with any missing parent, each with mode [`DIR_MODE`] (0700)
//!    on Unix, and the new directories' names are synced into their
//!    parents at once (Unix); a backup whose name exists there is refused
//!    (kind `Exist`) before anything is written.
//! 2. Every entry of `<dir>/.partial/` ([`PARTIAL_DIR`]) is removed: one
//!    backup runs at a time, so whatever is there was left by an
//!    interrupted one. Nothing outside `.partial/` is ever removed, and
//!    `.partial` itself must be a directory, not a link to one (created
//!    with mode 0700 on Unix when missing).
//! 3. A new redb file is created as `.partial/<name>` (refusing an existing
//!    file), with mode [`FILE_MODE`] (0600, dcrwallet's mode for
//!    `wallet.db`) on Unix and a [`BACKUP_CACHE_SIZE`] page cache. A backup
//!    holds everything the store holds, so it is created as private as the
//!    store.
//! 4. One read transaction of the live store is the snapshot. Every table
//!    it holds is copied, rows as raw bytes in key order, into one write
//!    transaction of the new file, and the backup record
//!    ([`BackupRecord`]) is written into the copy's `meta` table under
//!    [`BACKUP_RECORD_KEY`]. The transaction commits with
//!    `Durability::Immediate`. Writers and readers of the live store are
//!    never blocked: the snapshot is an ordinary read transaction.
//! 5. The copy is compacted with `CompactionMode::Balanced` until redb
//!    reports that no further compaction is possible (Decision item 2; on a
//!    fresh copy `Maximum` gives the same file, Open point 5).
//! 6. The file is closed, renamed to `<dir>/<name>`, a rename within one
//!    directory tree, and both directories are synced (Unix; Windows has
//!    no directory sync, Open point 7).
//!
//! A crash or power cut at any point leaves the live store as it was (the
//! backup only reads it) and either no file under the final name or a
//! complete one: the rename is the last step, and it happens only after
//! the commit and the compaction have returned. An error before the rename
//! removes the backup's own file from `.partial/`; a stop asked by the
//! observer ([`Store::backup_observed`]), which is how process shutdown
//! stops a backup, leaves it for the next backup to remove.
//!
//! A snapshot sees every commit made before it, those of initial sync's
//! relaxed batches included, which the live store itself may still lose
//! to a power cut (they are headers and filters, fetched again).
//!
//! [`Store::backup_to_backend`] does steps 4 and 5 into a storage backend
//! instead of a file, for fault-injection tests.
//!
//! # The backup record
//!
//! One row of the copy's `meta` table, [`BACKUP_RECORD_KEY`]
//! (`backup`), whose layout is on [`BackupRecord`]: its format, the
//! network, the schema version, the dcr-purse version that wrote it, the
//! time from the caller's clock, the number of rows copied and their
//! digest. The live store never holds the row: a backup copies every row
//! of its snapshot except one under that key, and `restore` (Phase 8)
//! removes it. [`MetaKey`] does not name it, so a typed walk
//! of a backup's `meta` table stops at it; [`ReadTx::check_rows`] skips
//! it.
//!
//! **The digest** is BLAKE-256 (`dcroxide-crypto`) over every row of the
//! snapshot, tables in the byte order of their names and rows in key
//! order, each row as three length-prefixed fields: the table's name, the
//! key and the value, each length a u64 big-endian. The backup record's own
//! row is not a row of the snapshot and is not digested.
//!
//! # Verifying a file handed in from outside
//!
//! [`verify_backup`] never opens the file it is given as a store: it copies
//! it into a scratch directory, opens the copy with redb's integrity check
//! (a walk of every page's checksum, where a plain or read-only open may
//! panic on a damaged file; Decision item 6), and deletes the copy
//! afterwards. Only a regular file is copied, and no more of it than its
//! length when it was opened ([`VERIFY_NOT_A_FILE_TEXT`] refuses a
//! directory, a FIFO, a socket or a device, which could be read without
//! end). The type is checked by path before the file is opened and again
//! on the open handle, and on Unix the open carries `O_NONBLOCK`, so that a
//! FIFO with no writer is refused instead of blocking the open forever.
//! The copy is created with mode [`FILE_MODE`] on Unix under a name
//! beginning [`VERIFY_COPY_PREFIX`], a prefix reserved to it in the scratch
//! directory. On Unix the copy's name is removed as soon as the file is
//! created, before a byte is copied, so the copy has no name while it is
//! checked and a killed process leaves nothing behind. On other platforms
//! the name stays until the check ends, and a process killed in between
//! leaves the copy; every verification therefore first removes whatever
//! file carries the reserved prefix in its scratch directory, so the next
//! one into the same directory deletes it. The file being verified is
//! never removed that way, even when it lies in the scratch directory
//! under the reserved prefix: it is recognised by device and inode on
//! Unix and by canonical path elsewhere, and when it cannot be recognised
//! nothing is removed. It refuses, with an error and
//! never by panicking: a file that is not a redb database or fails the
//! integrity check; a redb database that is not a dcr-purse store; a store with no
//! schema version, version 0, or a version newer than the build (kind
//! `Invalid`, [`Error::newer_version_info`]); a backup record that does not
//! decode, whose schema version is not the store's, whose network is not
//! the one asked for, or whose row count or digest is not that of the
//! rows in the file ([`Error::unverified`]). A store of this build's
//! version must then have every table of the schema, every row must decode
//! by its table's types, and the chain, record and transaction tables must
//! hold their invariants ([`ReadTx::check_rows`], [`ReadTx::check_chain`],
//! [`ReadTx::check_records`], [`ReadTx::check_tx_tables`]; ADR-0018 Open
//! point BK4). The invariants of the transaction tables that need a
//! transaction decoded (every record's bytes a transaction, every credit,
//! unmined credit and debit against the output or input of its transaction
//! it names, an unmined input's spender spending the outpoint, QK-0785's
//! spender, no live row of a regular transaction in a block marked
//! invalid) are `purse-udb`'s, which a
//! caller runs on the same read transaction through [`verify_backup_with`]
//! (`purse-udb`'s `txstore::verify_backup`); [`verify_backup`] alone does
//! not check them. Neither checks what [`crate::txtables`] lists as not
//! checked: a record's key against its transaction's hash, a credit's
//! flags against its transaction, that a spent credit's debit exists, a
//! commitment against its ticket, and the rules the reproduced states rule
//! out. Nor is anything checked of the auxiliary tables (VSP records,
//! treasury and agenda policies) beyond their encodings and the index
//! rules of [`crate::records`]; a consistency check of their own is a
//! later addition. The checks accept every state of the transaction tables
//! that dcrwallet's defects, which dcr-purse reproduces, leave (listed in
//! [`crate::txtables`]), so a store that passes is consistent as dcr-purse
//! writes it, not free of those states. A wallet does not run these checks
//! when it opens its own store; only this verify path and the tests do.
//! A plain copy of a stopped wallet's store has
//! no backup record and is checked the same way without it (Decision item
//! 5). The only bytes the verify path parses itself are the backup
//! record's, which are bounded by its layout; the rest is redb's to read.
//!
//! # Not here
//!
//! The JSON-RPC method `backupwallet` (Phase 7), the `verify` and
//! `restore` commands of the separate tool (Phase 8) and the user
//! documentation (Phase 13) are built on these functions later. Restoring
//! (removing the record, migrating, renaming into place) is the tool's.
//! The tool is to verify through `purse-udb`'s `txstore::verify_backup`,
//! which adds the decoded checks, and its documentation is to state what
//! verify does not check, as the paragraph above does (ADR-0018 BK4).
//!
//! [`DIR_MODE`]: crate::DIR_MODE
//! [`FILE_MODE`]: crate::FILE_MODE

use std::fs::{self, File};
use std::io;
use std::ops::ControlFlow;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use dcroxide_crypto::blake256::Blake256;
use redb::backends::FileBackend;
use redb::{
    CompactionMode, Database, Durability, ReadTransaction, ReadableDatabase as _,
    ReadableTable as _, StorageBackend, TableDefinition, TableHandle as _,
};

use crate::codec::{Codec, DecodeError, Reader};
use crate::error::{Error, Result};
use crate::keys::MetaKey;
use crate::schema::{
    AccountNames, Accounts, AcctHistory, AddrByAcctSha, AddrByHash, AddrByPath, AgendaPrefs,
    BalanceHeights, Balances, BlockByHash, Blocks, CFilters, CFiltersDetached, Commitments,
    CommitmentsUnspent, Credits, CreditsInvalidated, Debits, DebitsInvalidated, Headers,
    HeadersDetached, History, MultisigOuts, MultisigUnspent, TSpendPolicies, Table, TableId,
    TicketAgendaPrefs, Tickets, TreasuryKeyPolicies, TxMined, TxUnmined, UnminedCredits,
    UnminedInputs, Unpublished, Unspent, VspFeeStatus, VspHosts, VspPubKeys, VspTSpendPolicies,
    VspTickets, VspTreasuryKeyPolicies, Wallet,
};
use crate::store::{
    FORMAT_MAGIC, SCHEMA_VERSION, Store, create_private_dirs, create_private_file, sync_dir,
    sync_parents,
};
use crate::txn::{Iter, Raw, ReadTx, WriteReader, WriteTx, definition, meta_u32};
use crate::wallet;

/// The directory of a network's backups, inside its data directory
/// (`<appdata>/<net>/backups/`, ADR-0018 Decision item 4). The wallet
/// passes `<appdata>/<net>/` joined with this name to [`Store::backup`].
pub const BACKUPS_DIR: &str = "backups";

/// The directory inside the backups directory that holds a backup until
/// it is published (ADR-0018 Decision item 3). Its leading dot keeps it out
/// of reach of every name [`check_backup_name`] accepts.
pub const PARTIAL_DIR: &str = ".partial";

/// The page cache of the file a backup writes: 64 MiB (ADR-0018 Decision
/// item 1), which bounds the backup's memory beside the live store's own
/// cache.
pub const BACKUP_CACHE_SIZE: usize = 64 << 20;

/// The page cache of the copy [`verify_backup`] opens.
pub const VERIFY_CACHE_SIZE: usize = 64 << 20;

/// The key of the backup record in a backup's `meta` table.
pub const BACKUP_RECORD_KEY: &[u8] = b"backup";

/// The format of the backup record this build writes, its first byte.
pub const BACKUP_RECORD_FORMAT: u8 = 1;

/// The refusal of a backup while another runs (ADR-0018 Decision item 1).
pub const BACKUP_RUNNING_TEXT: &str = "a backup is already running";

/// The refusal of an empty backup name.
pub const BACKUP_NAME_EMPTY_TEXT: &str = "backup name is empty";

/// The refusal of a backup name with a path separator (`/` or `\` on every
/// platform) or any other directory component.
pub const BACKUP_NAME_SEPARATOR_TEXT: &str = "backup name contains a path separator";

/// The refusal of a backup name containing `..`.
pub const BACKUP_NAME_DOTDOT_TEXT: &str = "backup name contains ..";

/// The refusal of a backup name beginning with a dot, which keeps
/// [`PARTIAL_DIR`] and every hidden name out of a client's reach.
pub const BACKUP_NAME_DOT_TEXT: &str = "backup name begins with a dot";

/// The refusal of a backup name that is not a file name on every platform
/// the wallet runs on (see [`check_backup_name`]).
pub const BACKUP_NAME_NOT_PORTABLE_TEXT: &str = "backup name is not a portable file name";

/// The refusal of a backup when `.partial` in the backups directory is not
/// a directory (a file, or a link that would lead the clean-up elsewhere).
pub const PARTIAL_NOT_DIR_TEXT: &str = "the backups directory's .partial is not a directory";

/// The refusal of a file handed to [`verify_backup`] that is not a regular
/// file (a directory, a FIFO, a socket, a device): it is not copied.
pub const VERIFY_NOT_A_FILE_TEXT: &str = "the file to verify is not a regular file";

/// The prefix of the copies [`verify_backup`] makes in its scratch
/// directory. Every file whose name begins with it in that directory is
/// reserved to verification and removed by the next one.
pub const VERIFY_COPY_PREFIX: &str = ".verify-";

/// The longest backup name, in bytes: the common file-name limit of the
/// file systems the wallet runs on.
pub const MAX_BACKUP_NAME_LEN: usize = 255;

/// At most this many compaction passes. redb's `compact` already repeats
/// until it makes no progress, so the second call normally reports that
/// nothing is left; the bound only keeps a misbehaving engine from looping.
const MAX_COMPACTIONS: u32 = 16;

/// Names Windows reserves for devices, refused with or without an
/// extension, compared without regard to ASCII case: Microsoft's list in
/// "Naming Files, Paths, and Namespaces" (`CON`, `PRN`, `AUX`, `NUL`, and
/// `COM` and `LPT` followed by a digit 0 to 9 or a superscript `¹`, `²` or
/// `³`), and the console names `CONIN$` and `CONOUT$`, which `CreateFile`
/// opens as console handles. Go reserves the same apart from `COM0` and
/// `LPT0` (`internal/filepathlite/path_windows.go`, `isReservedBaseName`),
/// and only the whole base name for the console names; refusing those two
/// as a stem as well, and `COM0` and `LPT0`, costs nothing.
const WINDOWS_DEVICES: [&str; 32] = [
    "CON", "PRN", "AUX", "NUL", "COM0", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7",
    "COM8", "COM9", "COM¹", "COM²", "COM³", "LPT0", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6",
    "LPT7", "LPT8", "LPT9", "LPT¹", "LPT²", "LPT³", "CONIN$", "CONOUT$",
];

/// Checks a backup name as ADR-0018 Decision item 4 asks, refusing (kind
/// `Invalid`, each with its own text) a name that is empty, contains a
/// path separator (`/` or `\`, on every platform) or `..`, or begins with
/// a dot.
///
/// Beyond that list, and so that the answer is the same on every platform
/// (ADR-0012), it refuses as [`BACKUP_NAME_NOT_PORTABLE_TEXT`] a name that
/// Windows would not keep as one file name in the backups directory: one
/// with a control character or one of `: * ? " < > |` (a `:` names a drive
/// or an alternate data stream there), one ending in a dot or a space
/// (Windows drops them, so `w.` would be `w`), a device name such as `CON`,
/// `nul.txt`, `COM¹` or `CONOUT$`, or one longer than
/// [`MAX_BACKUP_NAME_LEN`] bytes. A name
/// that passes is exactly one plain path component everywhere. Whether the
/// name exists is checked by the backup itself.
pub fn check_backup_name(name: &str) -> Result<()> {
    if name.is_empty() {
        return Err(Error::invalid(BACKUP_NAME_EMPTY_TEXT));
    }
    if name.contains(['/', '\\']) {
        return Err(Error::invalid(BACKUP_NAME_SEPARATOR_TEXT));
    }
    if name.contains("..") {
        return Err(Error::invalid(BACKUP_NAME_DOTDOT_TEXT));
    }
    if name.starts_with('.') {
        return Err(Error::invalid(BACKUP_NAME_DOT_TEXT));
    }
    let stem = name.split('.').next().unwrap_or(name);
    let not_portable = name.len() > MAX_BACKUP_NAME_LEN
        || name.contains(|c: char| c.is_control() || ":*?\"<>|".contains(c))
        || name.ends_with(['.', ' '])
        || WINDOWS_DEVICES
            .iter()
            .any(|d| stem.trim_end().eq_ignore_ascii_case(d));
    if not_portable {
        return Err(Error::invalid(BACKUP_NAME_NOT_PORTABLE_TEXT));
    }
    // What the rules above already imply, checked against this platform's
    // own path parser.
    let mut parts = Path::new(name).components();
    match (parts.next(), parts.next()) {
        (Some(Component::Normal(n)), None) if n == name => Ok(()),
        _ => Err(Error::invalid(BACKUP_NAME_SEPARATOR_TEXT)),
    }
}

/// What the caller tells a backup about itself, for its record.
#[derive(Clone, Copy, Debug)]
pub struct BackupInfo<'a> {
    /// The network's name, as the data directory names it (`mainnet`,
    /// `testnet3`, `simnet`, `regnet`); 1 to 255 bytes. The store does not
    /// know its network, so the caller states it.
    pub network: &'a str,
    /// The dcr-purse version writing the backup (ADR-0005 Decision item
    /// 9); 1 to 255 bytes.
    pub version: &'a str,
    /// The time of the backup, Unix nanoseconds from the wallet's injected
    /// clock (`Clock::now_wall`, ADR-0008).
    pub time: i64,
}

/// A published backup.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Backup {
    /// The file written: the backups directory as the caller gave it,
    /// joined with the name.
    pub path: PathBuf,
    /// The backup record written into it.
    pub record: BackupRecord,
}

/// A step of a backup, reported to the observer of
/// [`Store::backup_observed`] and [`Store::backup_to_backend_observed`]
/// once the step is done.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum BackupStep<'a> {
    /// The backup's file exists and holds an empty redb database
    /// (`.partial/` cleaned first, for a file).
    Prepared,
    /// One table of the snapshot, named, has been copied into the write
    /// transaction (not yet committed).
    TableCopied(&'a str),
    /// The copy and its record are committed durably.
    Committed,
    /// The copy is compacted.
    Compacted,
    /// The database is closed; for a file, the rename comes next.
    Closed,
}

/// The backup record: what a backup says about itself, in its `meta`
/// table under [`BACKUP_RECORD_KEY`] (ADR-0018 Decision item 1).
///
/// Layout (54 to 564 bytes): format u8 (1) ‖ network length u8 ‖ network
/// (UTF-8, 1 to 255 bytes) ‖ schema version u32 BE (not 0) ‖ version
/// length u8 ‖ dcr-purse version (UTF-8, 1 to 255 bytes) ‖ time i64 BE
/// (Unix nanoseconds) ‖ row count u64 BE ‖ digest (32 bytes, BLAKE-256 of
/// the rows as the module documentation defines it).
///
/// A record of another format is refused by the decoder as an unknown
/// format: a later format is read by a later build.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct BackupRecord {
    network: String,
    schema_version: u32,
    version: String,
    time: i64,
    rows: u64,
    digest: [u8; 32],
}

/// The longest network and version texts of a [`BackupRecord`], in bytes.
pub const MAX_RECORD_TEXT_LEN: usize = 255;

fn check_text(what: &'static str, s: &str) -> Result<()> {
    if s.is_empty() || s.len() > MAX_RECORD_TEXT_LEN {
        Err(Error::invalid(what))
    } else {
        Ok(())
    }
}

impl BackupRecord {
    /// A record, refusing (kind `Invalid`) a network or version that is
    /// empty or longer than [`MAX_RECORD_TEXT_LEN`] bytes, and schema
    /// version 0.
    pub fn new(
        network: &str,
        schema_version: u32,
        version: &str,
        time: i64,
        rows: u64,
        digest: [u8; 32],
    ) -> Result<BackupRecord> {
        check_text("backup network must be 1 to 255 bytes", network)?;
        check_text("backup version must be 1 to 255 bytes", version)?;
        if schema_version == 0 {
            return Err(Error::invalid("backup schema version 0"));
        }
        Ok(BackupRecord {
            network: network.to_owned(),
            schema_version,
            version: version.to_owned(),
            time,
            rows,
            digest,
        })
    }

    /// The network the backed-up wallet is on.
    pub fn network(&self) -> &str {
        &self.network
    }

    /// The schema version of the copied store.
    pub fn schema_version(&self) -> u32 {
        self.schema_version
    }

    /// The dcr-purse version that wrote the backup.
    pub fn version(&self) -> &str {
        &self.version
    }

    /// The time of the backup, Unix nanoseconds.
    pub fn time(&self) -> i64 {
        self.time
    }

    /// The number of rows copied.
    pub fn rows(&self) -> u64 {
        self.rows
    }

    /// The digest of the rows copied.
    pub fn digest(&self) -> [u8; 32] {
        self.digest
    }
}

/// A length a record field's constructor bounds to 255.
fn len_u8(s: &str) -> u8 {
    u8::try_from(s.len()).unwrap_or(u8::MAX)
}

impl Codec for BackupRecord {
    type Bytes = Vec<u8>;

    fn encode(&self) -> Vec<u8> {
        let mut v = Vec::with_capacity(
            54usize
                .saturating_add(self.network.len())
                .saturating_add(self.version.len()),
        );
        v.push(BACKUP_RECORD_FORMAT);
        v.push(len_u8(&self.network));
        v.extend_from_slice(self.network.as_bytes());
        v.extend_from_slice(&self.schema_version.to_be_bytes());
        v.push(len_u8(&self.version));
        v.extend_from_slice(self.version.as_bytes());
        v.extend_from_slice(&self.time.to_be_bytes());
        v.extend_from_slice(&self.rows.to_be_bytes());
        v.extend_from_slice(&self.digest);
        v
    }

    fn decode(bytes: &[u8]) -> std::result::Result<Self, DecodeError> {
        let mut r = Reader::new("backup record", bytes);
        if r.u8()? != BACKUP_RECORD_FORMAT {
            return Err(r.field("unknown format"));
        }
        let network = text(&mut r, "empty network", "network is not UTF-8")?;
        let schema_version = r.u32_be()?;
        if schema_version == 0 {
            return Err(r.field("schema version 0"));
        }
        let version = text(&mut r, "empty version", "version is not UTF-8")?;
        let record = BackupRecord {
            network,
            schema_version,
            version,
            time: r.i64_be()?,
            rows: r.u64_be()?,
            digest: r.array()?,
        };
        r.finish()?;
        Ok(record)
    }
}

/// A record text: length u8 (not 0) ‖ UTF-8 bytes.
fn text(
    r: &mut Reader<'_>,
    empty: &'static str,
    utf8: &'static str,
) -> std::result::Result<String, DecodeError> {
    let n = r.u8()?;
    if n == 0 {
        return Err(r.field(empty));
    }
    let b = r.bytes(usize::from(n))?;
    std::str::from_utf8(b)
        .map(str::to_owned)
        .map_err(|_| r.field(utf8))
}

/// The digest of a backup's rows (module documentation) and their count.
struct RowDigest {
    h: Blake256,
    rows: u64,
}

impl RowDigest {
    fn new() -> RowDigest {
        RowDigest {
            h: Blake256::new(),
            rows: 0,
        }
    }

    fn feed(&mut self, table: &str, key: &[u8], value: &[u8]) {
        for part in [table.as_bytes(), key, value] {
            self.h.update(&(part.len() as u64).to_be_bytes());
            self.h.update(part);
        }
        self.rows = self.rows.saturating_add(1);
    }

    fn finish(self) -> ([u8; 32], u64) {
        (self.h.finalize(), self.rows)
    }
}

/// The names of every table a read transaction sees, in byte order,
/// refusing a multimap table (the store has none).
fn table_names(txn: &ReadTransaction) -> Result<Vec<String>> {
    if txn.list_multimap_tables()?.next().is_some() {
        return Err(Error::foreign("a multimap table"));
    }
    let mut names: Vec<String> = txn.list_tables()?.map(|h| h.name().to_owned()).collect();
    names.sort_unstable();
    Ok(names)
}

fn is_record_row(table: &str, key: &[u8]) -> bool {
    table == TableId::Meta.name() && key == BACKUP_RECORD_KEY
}

/// The builder of the file a backup writes.
fn backup_builder() -> redb::Builder {
    let mut b = Database::builder();
    b.set_cache_size(BACKUP_CACHE_SIZE);
    b
}

/// The builder of the copy a verification opens: the integrity check on
/// open, always (ADR-0018 Decision item 6).
fn verify_builder() -> redb::Builder {
    let mut b = Database::builder();
    b.set_cache_size(VERIFY_CACHE_SIZE);
    b.set_check_integrity_on_open(true);
    b
}

/// An observer that never stops the backup.
fn carry_on(_: BackupStep<'_>) -> ControlFlow<()> {
    ControlFlow::Continue(())
}

/// Fails with [`Error::backup_stopped`] when the observer asks to stop.
fn observe(
    observer: &mut dyn FnMut(BackupStep<'_>) -> ControlFlow<()>,
    step: BackupStep<'_>,
) -> Result<()> {
    match observer(step) {
        ControlFlow::Continue(()) => Ok(()),
        ControlFlow::Break(()) => Err(Error::stopped()),
    }
}

fn check_info(info: &BackupInfo<'_>) -> Result<()> {
    BackupRecord::new(info.network, 1, info.version, info.time, 0, [0; 32]).map(drop)
}

impl Store {
    /// Writes a backup of the store as `<dir>/<name>` (module
    /// documentation, "A backup"): a consistent snapshot copied into a new
    /// store file with its backup record, committed durably, compacted,
    /// and published by rename. `dir` is the backups directory,
    /// `<appdata>/<net>/`[`BACKUPS_DIR`] for the wallet; it is created if
    /// it is missing.
    ///
    /// Refused before anything is written: a name [`check_backup_name`]
    /// refuses; a [`BackupInfo`] text that is empty or over 255 bytes; a
    /// backup while another runs ([`BACKUP_RUNNING_TEXT`]); a name that
    /// exists in `dir` (kind `Exist`, whatever is there, a broken link
    /// included); a `.partial` that is not a directory
    /// ([`PARTIAL_NOT_DIR_TEXT`]).
    ///
    /// Writers and readers of the store are not blocked; the store may
    /// grow by what is written during the backup, since the snapshot's
    /// pages are not reused while it lives. The name is checked again just
    /// before the rename, which on every platform would replace a file
    /// created under that name in between (ADR-0018, addendum "The
    /// `purse-store` part, as built", open point BK3).
    ///
    /// On Unix the backup file is created with mode [`FILE_MODE`] (0600)
    /// and every directory the backup creates with [`DIR_MODE`] (0700); a
    /// backups directory that exists keeps its mode. When the backup
    /// creates the backups directory (or parents of it), the new names are
    /// synced into their parents before the copy starts, so that a backup
    /// that has returned survives a power cut with the directory that holds
    /// it.
    ///
    /// An error after the directories were synced cannot happen; one from
    /// the directory sync itself comes after the rename, with the file
    /// published.
    ///
    /// [`DIR_MODE`]: crate::DIR_MODE
    /// [`FILE_MODE`]: crate::FILE_MODE
    pub fn backup(&self, dir: &Path, name: &str, info: &BackupInfo<'_>) -> Result<Backup> {
        self.backup_observed(dir, name, info, &mut carry_on)
    }

    /// [`Store::backup`], reporting each step to `observer` once it is
    /// done. When the observer returns `Break`, the backup stops there with
    /// [`Error::backup_stopped`] and leaves its file in `.partial/` as it
    /// is, for the next backup to remove: this is how process shutdown
    /// stops a backup (ADR-0018 Decision item 4: a started backup runs to
    /// its end otherwise, whatever happens to the request that asked for
    /// it). A `Break` at any step stops the backup unpublished,
    /// [`BackupStep::Closed`] included: the file is complete in
    /// `.partial/` then, and the next backup removes it. The last step
    /// reported is [`BackupStep::Closed`]; after a `Continue` there, what
    /// is left is the second check of the name (which fails, removing the
    /// file from `.partial/`, if something took the name meanwhile), the
    /// rename and the directory syncs, any of which can still fail.
    pub fn backup_observed(
        &self,
        dir: &Path,
        name: &str,
        info: &BackupInfo<'_>,
        observer: &mut dyn FnMut(BackupStep<'_>) -> ControlFlow<()>,
    ) -> Result<Backup> {
        check_backup_name(name)?;
        check_info(info)?;
        let _guard = self.backup_guard()?;
        let created = create_private_dirs(dir).map_err(Error::io)?;
        sync_parents(&created).map_err(Error::io)?;
        let target = dir.join(name);
        refuse_existing(&target)?;
        let partial_dir = dir.join(PARTIAL_DIR);
        prepare_partial_dir(&partial_dir)?;
        let partial = partial_dir.join(name);
        let file = match create_private_file(&partial) {
            Ok(f) => f,
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                return Err(Error::exists(partial));
            }
            Err(e) => return Err(Error::io(e)),
        };
        let written: Result<BackupRecord> = (|| {
            let mut db = backup_builder().create_with_backend(FileBackend::new(file))?;
            observe(observer, BackupStep::Prepared)?;
            let record = self.copy_into(&mut db, info, observer)?;
            db.close()?;
            observe(observer, BackupStep::Closed)?;
            Ok(record)
        })();
        let record = match written {
            Ok(r) => r,
            Err(e) => {
                if !e.backup_stopped() {
                    // The file is this backup's own and was never
                    // published; the database is closed by now.
                    let _ = fs::remove_file(&partial);
                }
                return Err(e);
            }
        };
        if let Err(e) = refuse_existing(&target) {
            let _ = fs::remove_file(&partial);
            return Err(e);
        }
        fs::rename(&partial, &target).map_err(Error::io)?;
        sync_dir(dir).map_err(Error::io)?;
        sync_dir(&partial_dir).map_err(Error::io)?;
        Ok(Backup {
            path: target,
            record,
        })
    }

    /// Steps 4 and 5 of a backup (module documentation) into an empty
    /// storage backend instead of a file, closing the database at the end:
    /// for fault-injection tests of the copy and its compaction. Refused
    /// as [`Store::backup`] refuses, and when the backend is not empty.
    pub fn backup_to_backend(
        &self,
        backend: impl StorageBackend,
        info: &BackupInfo<'_>,
    ) -> Result<BackupRecord> {
        self.backup_to_backend_observed(backend, info, &mut carry_on)
    }

    /// [`Store::backup_to_backend`] with an observer, as
    /// [`Store::backup_observed`] has.
    pub fn backup_to_backend_observed(
        &self,
        backend: impl StorageBackend,
        info: &BackupInfo<'_>,
        observer: &mut dyn FnMut(BackupStep<'_>) -> ControlFlow<()>,
    ) -> Result<BackupRecord> {
        check_info(info)?;
        let _guard = self.backup_guard()?;
        let mut db = backup_builder().create_with_backend(backend)?;
        {
            let txn = db.begin_read()?;
            if txn.list_tables()?.next().is_some() || txn.list_multimap_tables()?.next().is_some() {
                return Err(Error::invalid("backend is not empty"));
            }
        }
        observe(observer, BackupStep::Prepared)?;
        let record = self.copy_into(&mut db, info, observer)?;
        db.close()?;
        observe(observer, BackupStep::Closed)?;
        Ok(record)
    }

    /// Copies a snapshot of the store into `dest` with its backup record,
    /// commits durably and compacts.
    fn copy_into(
        &self,
        dest: &mut Database,
        info: &BackupInfo<'_>,
        observer: &mut dyn FnMut(BackupStep<'_>) -> ControlFlow<()>,
    ) -> Result<BackupRecord> {
        let snap = self.database().begin_read()?;
        let schema_version = {
            let meta = snap.open_table(definition(TableId::Meta))?;
            let v = meta
                .get(MetaKey::Version.name())?
                .map(|g| meta_u32(g.value(), "schema version"))
                .transpose()?;
            v.ok_or_else(Error::not_initialized)?
        };
        let names = table_names(&snap)?;
        let mut txn = dest.begin_write()?;
        txn.set_durability(Durability::Immediate)?;
        let mut digest = RowDigest::new();
        for name in &names {
            let def = TableDefinition::<Raw, Raw>::new(name);
            {
                let src = snap.open_table(def)?;
                let mut dst = txn.open_table(def)?;
                for e in src.iter()? {
                    let (k, v) = e?;
                    let (k, v) = (k.value(), v.value());
                    if is_record_row(name, k) {
                        continue;
                    }
                    dst.insert(k, v)?;
                    digest.feed(name, k, v);
                }
            }
            if let Err(e) = observe(observer, BackupStep::TableCopied(name)) {
                let _ = txn.abort();
                return Err(e);
            }
        }
        snap.close()?;
        let (digest, rows) = digest.finish();
        let record = BackupRecord::new(
            info.network,
            schema_version,
            info.version,
            info.time,
            rows,
            digest,
        )?;
        {
            let mut meta = txn.open_table(definition(TableId::Meta))?;
            meta.insert(BACKUP_RECORD_KEY, record.encode().as_slice())?;
        }
        txn.commit()?;
        observe(observer, BackupStep::Committed)?;
        for _ in 0..MAX_COMPACTIONS {
            if !dest.compact(CompactionMode::Balanced)? {
                break;
            }
        }
        observe(observer, BackupStep::Compacted)?;
        Ok(record)
    }
}

/// Refuses a path where something exists, a broken link included.
fn refuse_existing(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(_) => Err(Error::exists(path.to_owned())),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(Error::io(e)),
    }
}

/// Creates `.partial`, or empties it: everything in it was left by an
/// interrupted backup. A `.partial` that is not a directory, a link to
/// one included, is refused, so that the clean-up never reaches outside
/// it; links inside it are removed, never followed. A new `.partial` has
/// mode [`DIR_MODE`] on Unix; its name is made durable by the sync of the
/// backups directory after the rename.
///
/// [`DIR_MODE`]: crate::DIR_MODE
fn prepare_partial_dir(dir: &Path) -> Result<()> {
    match fs::symlink_metadata(dir) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            return create_private_dirs(dir).map(drop).map_err(Error::io);
        }
        Err(e) => return Err(Error::io(e)),
        Ok(m) if !m.is_dir() => return Err(Error::invalid(PARTIAL_NOT_DIR_TEXT)),
        Ok(_) => {}
    }
    for entry in fs::read_dir(dir).map_err(Error::io)? {
        let entry = entry.map_err(Error::io)?;
        let path = entry.path();
        let kind = entry.file_type().map_err(Error::io)?;
        let removed = if kind.is_dir() {
            fs::remove_dir_all(&path)
        } else {
            // A link to a directory is removed as a directory on Windows.
            fs::remove_file(&path).or_else(|e| fs::remove_dir(&path).map_err(|_| e))
        };
        removed.map_err(Error::io)?;
    }
    Ok(())
}

/// What [`verify_backup`] found in a file that verifies.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Verified {
    /// The backup record, or `None` for a plain copy of a stopped store.
    pub record: Option<BackupRecord>,
    /// The store's schema version.
    pub schema_version: u32,
    /// The rows in the file, the backup record's own row excepted.
    pub rows: u64,
    /// Their digest (module documentation).
    pub digest: [u8; 32],
    /// Whether the store's consistency checks ran: they do for a store of
    /// this build's schema version, and not for an older one, which a
    /// restore migrates before checking it.
    pub consistency_checked: bool,
}

static SCRATCH_SEQ: AtomicU64 = AtomicU64::new(0);

/// The file [`verify_backup`] was handed, as the scratch sweep recognises
/// it: by device and inode of the open handle on Unix, elsewhere by its
/// canonical path (std has no stable file identity there). `None` when the
/// file cannot be identified, and the sweep then removes nothing.
#[cfg(unix)]
struct Input(u64, u64);

#[cfg(unix)]
impl Input {
    fn of(_file: &Path, opened: &fs::Metadata) -> Option<Input> {
        use std::os::unix::fs::MetadataExt as _;
        Some(Input(opened.dev(), opened.ino()))
    }

    /// Whether `path`, followed through links, is the input file.
    fn is(&self, path: &Path) -> bool {
        use std::os::unix::fs::MetadataExt as _;
        fs::metadata(path).is_ok_and(|m| (m.dev(), m.ino()) == (self.0, self.1))
    }
}

#[cfg(not(unix))]
struct Input(PathBuf);

#[cfg(not(unix))]
impl Input {
    fn of(file: &Path, _opened: &fs::Metadata) -> Option<Input> {
        fs::canonicalize(file).ok().map(Input)
    }

    /// Whether `path`, followed through links, is the input file.
    fn is(&self, path: &Path) -> bool {
        fs::canonicalize(path).is_ok_and(|p| p == self.0)
    }
}

/// Removes every file of `dir` whose name begins with
/// [`VERIFY_COPY_PREFIX`]: copies a killed verification left behind (on
/// Unix only in the instant between creating a copy and removing its
/// name). The file being verified is never one of them, whatever its name
/// and wherever it lies: an entry that is it, or a link to it, stays, and
/// when the file cannot be identified nothing is removed. Errors are
/// ignored: a copy another verification still holds open may refuse removal
/// on Windows, and is removed by a later one. A directory under the prefix
/// is not a copy and is left alone.
fn sweep_scratch(dir: &Path, input: Option<&Input>) {
    let Some(input) = input else {
        return;
    };
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let reserved = entry
            .file_name()
            .to_str()
            .is_some_and(|n| n.starts_with(VERIFY_COPY_PREFIX));
        let is_dir = entry.file_type().is_ok_and(|t| t.is_dir());
        if reserved && !is_dir && !input.is(&entry.path()) {
            let _ = fs::remove_file(entry.path());
        }
    }
}

/// Opens the file handed to [`verify_backup`] for reading without
/// blocking on what it names. On Unix the open carries `O_NONBLOCK`, so a
/// FIFO with no writer (which a plain read-only open waits on forever) or a
/// device that waits for a carrier returns at once, and the type check on
/// the handle refuses it; on a regular file the flag changes nothing. The
/// type check by path before the open does not suffice alone: the path may
/// name something else by the time it is opened.
fn open_untrusted(file: &Path) -> io::Result<File> {
    let mut o = fs::OpenOptions::new();
    o.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        o.custom_flags(libc::O_NONBLOCK);
    }
    o.open(file)
}

/// Refuses what is not a regular file with [`VERIFY_NOT_A_FILE_TEXT`].
fn regular_file(meta: io::Result<fs::Metadata>) -> Result<fs::Metadata> {
    match meta {
        Ok(m) if m.is_file() => Ok(m),
        Ok(_) => Err(Error::invalid(VERIFY_NOT_A_FILE_TEXT)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Err(Error::missing()),
        Err(e) => Err(Error::io(e)),
    }
}

/// Creates a new file in `dir`, with [`FILE_MODE`] on Unix, under a name
/// beginning [`VERIFY_COPY_PREFIX`] that no other call uses.
///
/// [`FILE_MODE`]: crate::FILE_MODE
fn scratch_file(dir: &Path) -> Result<(PathBuf, File)> {
    let mut last = None;
    for _ in 0..64 {
        let n = SCRATCH_SEQ.fetch_add(1, Ordering::Relaxed);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.subsec_nanos());
        let path = dir.join(format!(
            "{VERIFY_COPY_PREFIX}{}-{nanos}-{n}.redb",
            std::process::id()
        ));
        match create_private_file(&path) {
            Ok(f) => return Ok((path, f)),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => last = Some(e),
            Err(e) => return Err(Error::io(e)),
        }
    }
    Err(Error::io(last.unwrap_or_else(|| {
        io::Error::from(io::ErrorKind::AlreadyExists)
    })))
}

/// Verifies a backup handed in from outside (module documentation,
/// "Verifying a file handed in from outside"): `file` is copied into
/// `scratch` (created if missing, mode [`DIR_MODE`] on Unix), the copy is
/// checked and then deleted, and `file` is only ever read. `network`, when
/// given, must be the one the backup record names; a file without a record
/// has no network to compare.
///
/// File names beginning [`VERIFY_COPY_PREFIX`] in `scratch` are reserved
/// to verification: each call first removes every such file there, a copy
/// a killed call left included, except `file` itself (or a link to it)
/// when it lies there under such a name. A process killed during the call
/// leaves no copy on Unix, where the copy has no name once created;
/// elsewhere it leaves one, which the next call into the same `scratch`
/// removes.
///
/// A missing file is refused with dcrwallet's `missing database file`
/// (kind `NotExist`); one that is not a regular file (a directory, a FIFO,
/// a socket, a device) with [`VERIFY_NOT_A_FILE_TEXT`] (kind `Invalid`)
/// before anything is copied, on every platform, and without waiting on
/// it: the type is checked by path before the open and again on the open
/// handle, and on Unix the open itself cannot block. The copy takes the
/// file's length when it was opened and no more: a file that grows
/// meanwhile is checked as it was.
///
/// The consistency checks are the store's own, the transaction tables'
/// included ([`ReadTx::check_tx_tables`]); a caller that also checks what
/// needs a transaction decoded passes that check to
/// [`verify_backup_with`] (`purse-udb`'s `txstore::verify_backup`).
///
/// [`DIR_MODE`]: crate::DIR_MODE
pub fn verify_backup(file: &Path, scratch: &Path, network: Option<&str>) -> Result<Verified> {
    verify_backup_with(file, scratch, network, |_| Ok(()))
}

/// [`verify_backup`], with `check` run last on the copy, on the read
/// transaction the store's consistency checks used, when those ran
/// ([`Verified::consistency_checked`]; not for an older schema version).
/// Its error is returned as the verification's. It is for the checks a
/// crate above the store makes of the same rows: `purse-udb` passes the
/// transaction tables' decoded checks (records holding transactions,
/// credits, unmined credits and debits against their transactions'
/// outputs and inputs, an unmined input's spender spending the outpoint,
/// QK-0785's spender, no live row of a regular transaction in a block
/// marked invalid), which the store cannot make without decoding
/// transactions.
pub fn verify_backup_with(
    file: &Path,
    scratch: &Path,
    network: Option<&str>,
    check: impl FnOnce(&ReadTx) -> Result<()>,
) -> Result<Verified> {
    // By path first: Windows cannot open a directory as a file at all, and
    // would report that as an I/O error rather than as what it is.
    regular_file(fs::metadata(file))?;
    let src = match open_untrusted(file) {
        Ok(f) => f,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Err(Error::missing()),
        Err(e) => return Err(Error::io(e)),
    };
    // The handle is authoritative: the path may have changed since.
    let meta = regular_file(src.metadata())?;
    let input = Input::of(file, &meta);
    create_private_dirs(scratch).map_err(Error::io)?;
    sweep_scratch(scratch, input.as_ref());
    let (copy, mut dst) = scratch_file(scratch)?;
    // On Unix the copy loses its name at once: the open handle keeps the
    // file, and the kernel frees it when the handle closes, however the
    // process ends.
    let unnamed = cfg!(unix) && fs::remove_file(&copy).is_ok();
    let verified: Result<Verified> = (|| {
        io::copy(&mut io::Read::take(src, meta.len()), &mut dst).map_err(Error::io)?;
        verify_backup_on_backend_with(FileBackend::new(dst), network, check)
    })();
    // The copy's database is closed by now.
    if !unnamed {
        let _ = fs::remove_file(&copy);
    }
    verified
}

/// The checks of [`verify_backup`] on a storage backend holding the copy:
/// the backend is opened with redb's integrity check, which writes to it.
pub fn verify_backup_on_backend(
    backend: impl StorageBackend,
    network: Option<&str>,
) -> Result<Verified> {
    verify_backup_on_backend_with(backend, network, |_| Ok(()))
}

/// [`verify_backup_on_backend`] with a caller's check, as
/// [`verify_backup_with`].
pub fn verify_backup_on_backend_with(
    backend: impl StorageBackend,
    network: Option<&str>,
    check: impl FnOnce(&ReadTx) -> Result<()>,
) -> Result<Verified> {
    let db = verify_builder()
        .create_with_backend(backend)
        .map_err(|e| untrusted(Error::from(e)))?;
    verify_db(Store::new(db), network, check).map_err(untrusted)
}

/// An engine error that a file of someone else's can cause and that the
/// store's own files cannot: a table of another type. The store reports
/// it as a misuse of the engine (kind `Bug`); from a file handed in, it
/// means the file is not a dcr-purse store.
fn untrusted(e: Error) -> Error {
    use redb::Error as E;
    match e.engine() {
        Some(
            E::TableTypeMismatch(_)
            | E::TableIsMultimap(_)
            | E::TableIsNotMultimap(_)
            | E::TypeDefinitionChanged(_),
        ) => Error::foreign("a table of another type"),
        _ => e,
    }
}

fn verify_db(
    store: Store,
    network: Option<&str>,
    check: impl FnOnce(&ReadTx) -> Result<()>,
) -> Result<Verified> {
    let version = store.inspect()?;
    if version > SCHEMA_VERSION {
        return Err(Error::newer_version(version, SCHEMA_VERSION));
    }
    let mut digest = RowDigest::new();
    let mut record_bytes = None;
    {
        let txn = store.database().begin_read()?;
        for name in &table_names(&txn)? {
            let t = txn.open_table(TableDefinition::<Raw, Raw>::new(name))?;
            for e in t.iter()? {
                let (k, v) = e?;
                let (k, v) = (k.value(), v.value());
                if is_record_row(name, k) {
                    record_bytes = Some(v.to_vec());
                } else {
                    digest.feed(name, k, v);
                }
            }
        }
        txn.close()?;
    }
    let (digest, rows) = digest.finish();
    let record = record_bytes
        .map(|b| BackupRecord::decode(&b).map_err(|e| Error::corrupt(TableId::Meta, e)))
        .transpose()?;
    if let Some(r) = &record {
        if r.schema_version != version {
            return Err(Error::bad_backup(format!(
                "the backup record names schema version {}, the store holds version {version}",
                r.schema_version
            )));
        }
        if let Some(n) = network
            && r.network != n
        {
            return Err(Error::other_network(r.network.clone(), n.to_owned()));
        }
        if r.rows != rows {
            return Err(Error::bad_backup(format!(
                "the backup record counts {} rows, the file holds {rows}",
                r.rows
            )));
        }
        if r.digest != digest {
            return Err(Error::bad_backup(
                "the rows' digest is not the backup record's".to_owned(),
            ));
        }
    }
    let consistency_checked = version == SCHEMA_VERSION;
    if consistency_checked {
        store.check_tables()?;
        let tx = store.read()?;
        tx.check_rows()?;
        tx.check_chain()?;
        tx.check_records()?;
        tx.check_tx_tables()?;
        check(&tx)?;
        tx.close()?;
    }
    store.close()?;
    Ok(Verified {
        record,
        schema_version: version,
        rows,
        digest,
        consistency_checked,
    })
}

/// Decodes every row a walk of one table yields.
fn rows_of<T: Table>(walk: Result<Iter<'_, T>>) -> Result<()> {
    for r in walk? {
        r?;
    }
    Ok(())
}

/// The rows' check, defined for a read transaction and for a write
/// transaction's shared-borrow reads (`WriteTx::check_rows`).
macro_rules! rows_check {
    () => {
        /// Decodes every key and value of every table by its table's types
        /// and fails with the first corrupt-row error ([`Error::corrupt_row`]):
        /// `meta` row by row through its typed accessors (its `format` row must
        /// be [`FORMAT_MAGIC`], and the backup record's row, which only a
        /// backup holds, is skipped), every `wallet` row by its value type, and
        /// every other table through [`ReadTx::iter`]. Costs a walk of the
        /// whole store; for a store whose origin is not trusted
        /// ([`verify_backup`]) and for tests.
        pub fn check_rows(&self) -> Result<()> {
            for &id in TableId::ALL {
                match id {
                    TableId::Meta => {
                        for r in self.range_raw(
                            TableId::Meta,
                            std::ops::Bound::Unbounded,
                            std::ops::Bound::Unbounded,
                        )? {
                            let (k, v) = r?;
                            if k == BACKUP_RECORD_KEY {
                                continue;
                            }
                            let key = MetaKey::decode(&k).map_err(|e| Error::corrupt(id, e))?;
                            if key == MetaKey::Format && v != FORMAT_MAGIC {
                                return Err(Error::foreign("unknown format row"));
                            }
                        }
                        self.schema_version()?;
                        self.tip()?;
                        self.birth_state()?;
                        self.have_cfilters()?;
                        self.processed_marker()?;
                    }
                    TableId::Wallet => {
                        rows_of(self.iter::<Wallet>())?;
                        self.wallet_row::<wallet::CoinTypeLegacy>()?;
                        self.wallet_row::<wallet::CoinTypeSlip0044>()?;
                        self.wallet_row::<wallet::CryptoKeyPriv>()?;
                        self.wallet_row::<wallet::CryptoKeyPub>()?;
                        self.wallet_row::<wallet::LastAccount>()?;
                        self.wallet_row::<wallet::LastImportedAccount>()?;
                        self.wallet_row::<wallet::MasterKeyPriv>()?;
                        self.wallet_row::<wallet::MasterKeyPub>()?;
                        self.wallet_row::<wallet::Slip0044Account0>()?;
                        self.wallet_row::<wallet::WatchOnly>()?;
                    }
                    TableId::Accounts => rows_of(self.iter::<Accounts>())?,
                    TableId::AccountNames => rows_of(self.iter::<AccountNames>())?,
                    TableId::AddrByHash => rows_of(self.iter::<AddrByHash>())?,
                    TableId::AddrByPath => rows_of(self.iter::<AddrByPath>())?,
                    TableId::AddrByAcctSha => rows_of(self.iter::<AddrByAcctSha>())?,
                    TableId::Headers => rows_of(self.iter::<Headers>())?,
                    TableId::Blocks => rows_of(self.iter::<Blocks>())?,
                    TableId::BlockByHash => rows_of(self.iter::<BlockByHash>())?,
                    TableId::CFilters => rows_of(self.iter::<CFilters>())?,
                    TableId::HeadersDetached => rows_of(self.iter::<HeadersDetached>())?,
                    TableId::CFiltersDetached => rows_of(self.iter::<CFiltersDetached>())?,
                    TableId::TxMined => rows_of(self.iter::<TxMined>())?,
                    TableId::TxUnmined => rows_of(self.iter::<TxUnmined>())?,
                    TableId::Unpublished => rows_of(self.iter::<Unpublished>())?,
                    TableId::History => rows_of(self.iter::<History>())?,
                    TableId::AcctHistory => rows_of(self.iter::<AcctHistory>())?,
                    TableId::Credits => rows_of(self.iter::<Credits>())?,
                    TableId::CreditsInvalidated => rows_of(self.iter::<CreditsInvalidated>())?,
                    TableId::Unspent => rows_of(self.iter::<Unspent>())?,
                    TableId::UnminedCredits => rows_of(self.iter::<UnminedCredits>())?,
                    TableId::UnminedInputs => rows_of(self.iter::<UnminedInputs>())?,
                    TableId::Debits => rows_of(self.iter::<Debits>())?,
                    TableId::DebitsInvalidated => rows_of(self.iter::<DebitsInvalidated>())?,
                    TableId::Tickets => rows_of(self.iter::<Tickets>())?,
                    TableId::Commitments => rows_of(self.iter::<Commitments>())?,
                    TableId::CommitmentsUnspent => rows_of(self.iter::<CommitmentsUnspent>())?,
                    TableId::Balances => rows_of(self.iter::<Balances>())?,
                    TableId::BalanceHeights => rows_of(self.iter::<BalanceHeights>())?,
                    TableId::MultisigOuts => rows_of(self.iter::<MultisigOuts>())?,
                    TableId::MultisigUnspent => rows_of(self.iter::<MultisigUnspent>())?,
                    TableId::VspTickets => rows_of(self.iter::<VspTickets>())?,
                    TableId::VspFeeStatus => rows_of(self.iter::<VspFeeStatus>())?,
                    TableId::VspHosts => rows_of(self.iter::<VspHosts>())?,
                    TableId::VspPubKeys => rows_of(self.iter::<VspPubKeys>())?,
                    TableId::TSpendPolicies => rows_of(self.iter::<TSpendPolicies>())?,
                    TableId::TreasuryKeyPolicies => rows_of(self.iter::<TreasuryKeyPolicies>())?,
                    TableId::VspTSpendPolicies => rows_of(self.iter::<VspTSpendPolicies>())?,
                    TableId::VspTreasuryKeyPolicies => {
                        rows_of(self.iter::<VspTreasuryKeyPolicies>())?
                    }
                    TableId::AgendaPrefs => rows_of(self.iter::<AgendaPrefs>())?,
                    TableId::TicketAgendaPrefs => rows_of(self.iter::<TicketAgendaPrefs>())?,
                }
            }
            Ok(())
        }
    };
}

impl ReadTx {
    rows_check!();
}

impl WriteReader<'_, '_> {
    rows_check!();
}

impl WriteTx<'_> {
    /// [`ReadTx::check_rows`] over this transaction's rows, its own writes
    /// included (opening, and so creating, every table first): a check of
    /// a state before it is committed, for tests.
    pub fn check_rows(&mut self) -> Result<()> {
        self.checked_reader()?.check_rows()
    }
}
