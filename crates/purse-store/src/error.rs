// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! The store's error, its dcrwallet kind, and its conversion into the
//! wallet-wide model.
//!
//! dcrwallet's bbolt driver wraps every engine error as
//! `errors.E(kind, err)`, choosing the kind from the bbolt sentinel
//! (`convertErr`, `wallet/internal/bdb/db.go:18-36`; spec 01 §8.1). The
//! store does the same with redb's errors: the kinds follow the nearest
//! bbolt sentinel, and the text is redb's own (ADR-0011 ET-006,
//! "store-engine texts under dcrwallet's `Op` and kind": redb-derived own
//! text). Errors the store raises itself use dcrwallet's text where
//! dcrwallet has the same condition.

use std::fmt;
use std::path::PathBuf;

use purse_errors::{AnyError, Kind, Op};

use crate::codec::DecodeError;
use crate::schema::TableId;

/// dcrwallet's text for a database whose version is newer than the build
/// (`wallet/udb/open.go:36-38`, kind `Invalid`).
pub const NEWER_VERSION_TEXT: &str = "database has been upgraded to an unknown newer version";

/// dcrwallet's text for a store without its metadata
/// (`wallet/udb/open.go:26-28`, kind `NotExist`).
pub const NOT_INITIALIZED_TEXT: &str = "database has not been initialized";

/// dcrwallet's text for opening a database file that does not exist
/// (`wallet/internal/bdb/db.go:316-323`, kind `NotExist`; spec 01 §3.2).
pub const MISSING_FILE_TEXT: &str = "missing database file";

/// A store error. [`Error::kind`] is the dcrwallet kind; `Display` renders
/// it as Go's `errors.E(kind, err)` does (`<kind text>: <cause>`).
#[derive(Debug)]
pub struct Error {
    repr: Repr,
}

#[derive(Debug)]
enum Repr {
    /// The engine failed.
    Engine(Box<redb::Error>),
    /// The engine failed inside a commit, after the transaction's writes
    /// may have reached storage: whether the commit is kept is unknown
    /// until the store is reopened.
    CommitUnknown(Box<redb::Error>),
    /// An I/O error outside the engine (creating the file).
    Io(std::io::Error),
    /// `create` found a file at the path.
    Exists(PathBuf),
    /// `open` found no file at the path.
    Missing,
    /// The file is a redb database but not a dcr-purse wallet store.
    Foreign(&'static str),
    /// The store has no schema version.
    NotInitialized,
    /// The store's schema version is newer than the build's.
    NewerVersion { found: u32, supported: u32 },
    /// The store's schema version is older than any migration reaches.
    NoMigration { found: u32, supported: u32 },
    /// A table of the schema is not in the file.
    MissingTable(TableId),
    /// A row that is not a valid encoding of its table's type.
    Corrupt { table: TableId, error: DecodeError },
    /// A request the store refuses (a caller error).
    Invalid(&'static str),
    /// A migration step failed.
    Migration { from: u32, source: Box<Error> },
}

impl Error {
    pub(crate) fn exists(path: PathBuf) -> Error {
        Error {
            repr: Repr::Exists(path),
        }
    }

    pub(crate) fn missing() -> Error {
        Error {
            repr: Repr::Missing,
        }
    }

    pub(crate) fn io(e: std::io::Error) -> Error {
        Error { repr: Repr::Io(e) }
    }

    pub(crate) fn foreign(why: &'static str) -> Error {
        Error {
            repr: Repr::Foreign(why),
        }
    }

    pub(crate) fn not_initialized() -> Error {
        Error {
            repr: Repr::NotInitialized,
        }
    }

    pub(crate) fn newer_version(found: u32, supported: u32) -> Error {
        Error {
            repr: Repr::NewerVersion { found, supported },
        }
    }

    pub(crate) fn no_migration(found: u32, supported: u32) -> Error {
        Error {
            repr: Repr::NoMigration { found, supported },
        }
    }

    pub(crate) fn missing_table(table: TableId) -> Error {
        Error {
            repr: Repr::MissingTable(table),
        }
    }

    pub(crate) fn corrupt(table: TableId, error: DecodeError) -> Error {
        Error {
            repr: Repr::Corrupt { table, error },
        }
    }

    pub(crate) fn invalid(why: &'static str) -> Error {
        Error {
            repr: Repr::Invalid(why),
        }
    }

    /// A commit's storage error (redb's `CommitError::Storage`): the
    /// transaction's changes are applied fully or not at all, but which is
    /// unknown until the store is reopened (redb 5.0.0
    /// `WriteTransaction::commit`).
    pub(crate) fn commit_unknown(e: redb::StorageError) -> Error {
        Error {
            repr: Repr::CommitUnknown(Box::new(e.into())),
        }
    }

    /// A commit error of a kind redb 5.0.0 does not have (its enum is
    /// non-exhaustive), whose outcome is therefore not documented.
    pub(crate) fn commit_unknown_other(e: redb::CommitError) -> Error {
        Error {
            repr: Repr::CommitUnknown(Box::new(e.into())),
        }
    }

    pub(crate) fn migration(from: u32, source: Error) -> Error {
        Error {
            repr: Repr::Migration {
                from,
                source: Box::new(source),
            },
        }
    }

    /// The dcrwallet kind of the error.
    pub fn kind(&self) -> Kind {
        match &self.repr {
            Repr::Engine(e) | Repr::CommitUnknown(e) => engine_kind(e),
            Repr::Io(_) => Kind::Io,
            Repr::Exists(_) => Kind::Exist,
            Repr::Missing | Repr::NotInitialized => Kind::NotExist,
            Repr::Foreign(_)
            | Repr::NewerVersion { .. }
            | Repr::NoMigration { .. }
            | Repr::Invalid(_) => Kind::Invalid,
            Repr::Corrupt { .. } | Repr::MissingTable(_) => Kind::Io,
            Repr::Migration { source, .. } => source.kind(),
        }
    }

    /// The engine's error, when the engine failed.
    pub fn engine(&self) -> Option<&redb::Error> {
        match &self.repr {
            Repr::Engine(e) | Repr::CommitUnknown(e) => Some(e),
            Repr::Migration { source, .. } => source.engine(),
            _ => None,
        }
    }

    /// Whether the error came from the engine inside a commit, after the
    /// transaction's writes may have reached storage, so that the commit
    /// may or may not be kept (redb 5.0.0 `WriteTransaction::commit`: the
    /// changes are applied fully or not at all, and "must not be assumed
    /// rolled back").
    ///
    /// Only [`Store::update`](crate::Store::update), the batches of
    /// [`InitialSync`](crate::InitialSync) and the migrations of an open
    /// raise it. After it the store refuses every further write
    /// transaction; it must be closed and reopened, and what the commit was
    /// to write read back, before anything is decided on it: a step that
    /// is not idempotent must not simply be retried. Every other error
    /// from those calls means nothing the transaction wrote is kept.
    pub fn commit_outcome_unknown(&self) -> bool {
        match &self.repr {
            Repr::CommitUnknown(_) => true,
            Repr::Migration { source, .. } => source.commit_outcome_unknown(),
            _ => false,
        }
    }

    /// For a store newer than the build: the version found and the newest
    /// this build supports (ADR-0003 Decision item 6).
    pub fn newer_version_info(&self) -> Option<(u32, u32)> {
        match &self.repr {
            Repr::NewerVersion { found, supported } => Some((*found, *supported)),
            _ => None,
        }
    }

    /// For a row that failed to decode: its table and the decode error.
    pub fn corrupt_row(&self) -> Option<(TableId, &DecodeError)> {
        match &self.repr {
            Repr::Corrupt { table, error } => Some((*table, error)),
            Repr::Migration { source, .. } => source.corrupt_row(),
            _ => None,
        }
    }

    /// For a failed migration step: the version it started from.
    pub fn migration_from(&self) -> Option<u32> {
        match &self.repr {
            Repr::Migration { from, .. } => Some(*from),
            _ => None,
        }
    }

    /// The cause's text, without the kind.
    fn cause(&self) -> String {
        match &self.repr {
            // redb renders its I/O errors with its own "I/O error: "
            // prefix, which would repeat the kind's text.
            Repr::Engine(e) | Repr::CommitUnknown(e) => match &**e {
                redb::Error::Io(io) => io.to_string(),
                e => e.to_string(),
            },
            Repr::Io(e) => e.to_string(),
            Repr::Exists(p) => format!("{} already exists", p.display()),
            Repr::Missing => MISSING_FILE_TEXT.to_owned(),
            Repr::Foreign(why) => format!("not a dcr-purse wallet store: {why}"),
            Repr::NotInitialized => NOT_INITIALIZED_TEXT.to_owned(),
            Repr::NewerVersion { .. } => NEWER_VERSION_TEXT.to_owned(),
            Repr::NoMigration { found, supported } => {
                format!("no migration from schema version {found} to {supported}")
            }
            Repr::MissingTable(table) => format!("{}: table missing", table.name()),
            Repr::Corrupt { table, error } => format!("{}: {error}", table.name()),
            Repr::Invalid(why) => (*why).to_owned(),
            Repr::Migration { from, source } => {
                format!("migration from schema version {from}: {}", source.cause())
            }
        }
    }

    /// The same error in the wallet-wide model, as Go's
    /// `errors.E(kind, err)` builds it: no op, the kind, and the cause as a
    /// text leaf. The caller adds its own op, as dcrwallet's callers do.
    pub fn to_purse_error(&self) -> purse_errors::Error {
        purse_errors::Error::from_parts(
            Op::default(),
            self.kind(),
            Some(AnyError::new(self.cause())),
        )
    }
}

/// The kind of an engine error, after the bbolt sentinel nearest to it
/// (spec 01 §8.1).
fn engine_kind(e: &redb::Error) -> Kind {
    use redb::Error as E;
    match e {
        // bbolt ErrInvalid ("invalid database") and OS errors: IO.
        E::Corrupted(_) | E::Io(_) | E::Backend(_) | E::PreviousIo => Kind::Io,
        // ErrDatabaseNotOpen, ErrValueTooLarge: Invalid. A file in an older
        // engine format is dcrwallet's "database upgrade required".
        E::DatabaseClosed | E::ValueTooLarge(_) | E::UpgradeRequired(_) => Kind::Invalid,
        // ErrBucketNotFound: NotExist.
        E::TableDoesNotExist(_) => Kind::NotExist,
        // Misuse of the engine by this crate.
        E::TableTypeMismatch(_)
        | E::TableIsMultimap(_)
        | E::TableIsNotMultimap(_)
        | E::TypeDefinitionChanged(_)
        | E::TableExists(_)
        | E::TableAlreadyOpen(..)
        | E::TransactionInProgress
        | E::TransactionPoisoned
        | E::ReadTransactionStillInUse(_)
        | E::LockPoisoned(_)
        | E::UnorderedKey
        | E::InvalidSavepoint
        | E::ImmediateDurabilityRequired
        | E::PersistentSavepointModified
        | E::NonDurableCommitUnsupported
        | E::EphemeralSavepointUnsupported
        | E::PersistentSavepointExists
        | E::EphemeralSavepointExists => Kind::Bug,
        // ErrDatabaseOpen and the unclassified rest: Other.
        _ => Kind::Other,
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let kind = self.kind();
        if kind == Kind::Other {
            f.write_str(&self.cause())
        } else {
            write!(f, "{}: {}", kind.as_str(), self.cause())
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match &self.repr {
            Repr::Engine(e) | Repr::CommitUnknown(e) => Some(&**e),
            Repr::Io(e) => Some(e),
            Repr::Corrupt { error, .. } => Some(error),
            Repr::Migration { source, .. } => Some(&**source),
            _ => None,
        }
    }
}

impl From<Error> for purse_errors::Error {
    fn from(e: Error) -> Self {
        e.to_purse_error()
    }
}

impl From<Error> for AnyError {
    fn from(e: Error) -> Self {
        AnyError::from(e.to_purse_error())
    }
}

macro_rules! from_redb {
    ($($t:ty),*) => {$(
        impl From<$t> for Error {
            fn from(e: $t) -> Self {
                Error { repr: Repr::Engine(Box::new(e.into())) }
            }
        }
    )*};
}

from_redb!(
    redb::Error,
    redb::DatabaseError,
    redb::TransactionError,
    redb::TableError,
    redb::StorageError,
    redb::CommitError,
    redb::SetDurabilityError
);

/// The store's result type.
pub type Result<T, E = Error> = std::result::Result<T, E>;
