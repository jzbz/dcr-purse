// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! The database handle: create, open, close, the schema version and its
//! migrations, and the transactions.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError, TryLockError};

use redb::{Database, Durability, ReadableDatabase as _, StorageBackend, TableHandle as _};

use crate::error::{Error, Result};
use crate::keys::MetaKey;
use crate::schema::TableId;
use crate::sync::InitialSync;
use crate::txn::{ReadTx, WriteTx, definition, meta_u32};

/// The schema version this build writes. Version 1 is the first: the
/// schema of ADR-0003 Decision item 2 as the foundation lays it out.
pub const SCHEMA_VERSION: u32 = 1;

/// The `format` meta row of every dcr-purse wallet store. A redb file
/// without it is not one and is refused.
pub const FORMAT_MAGIC: &[u8] = b"dcr-purse wallet store";

/// The name of the wallet's store file in its network directory
/// (`<appdata>/<net>/`, ADR-0005 Decision item 2), where dcrwallet keeps
/// `wallet.db`.
///
/// The project owner decided that the name is fixed in Phase 2 with the
/// schema and is not `wallet.db`, so that a dcrwallet started on the same
/// directory by mistake finds no database to open (ADR-0003, addendum of
/// 2026-11-19 "Decisions of the project owner", item 5; ADR-0018 Open point
/// 6; ADR-0005, "Options considered", option (d)). The name itself,
/// `wallet.redb`, was decided by the project owner on 2026-11-21 (ADR-0018,
/// addendum "The `purse-store` part, as built", open point BK1, and the
/// addendum of 2026-11-21). The store's own functions take a path from
/// their caller and never add this name themselves; the wallet loader,
/// `restore` and the documentation use it.
pub const STORE_FILE_NAME: &str = "wallet.redb";

/// The mode of every file the store creates on Unix (the store, a backup,
/// a verification's copy): read and write for the owner only, dcrwallet's
/// `bolt.Open(dbPath, 0600, nil)` (`wallet/internal/bdb/db.go:321`; spec
/// 01's parity constants). The process umask can only narrow it.
pub const FILE_MODE: u32 = 0o600;

/// The mode of every directory the store creates on Unix (the backups
/// directory and its parents when missing, `.partial`, a verification's
/// scratch directory): dcrwallet's `os.MkdirAll(path, 0700)`
/// (`walletsetup.go:375`). An existing directory's mode is left alone.
pub const DIR_MODE: u32 = 0o700;

/// Creates a new file at `path` with [`FILE_MODE`] on Unix, refusing an
/// existing one (`create_new`). On other platforms the file gets the
/// directory's default access control.
pub(crate) fn create_private_file(path: &Path) -> io::Result<fs::File> {
    let mut o = fs::OpenOptions::new();
    o.read(true).write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        o.mode(FILE_MODE);
    }
    o.open(path)
}

/// Creates the directory `dir` and every missing parent, each with
/// [`DIR_MODE`] on Unix, and returns the directories it created, outermost
/// first (empty when `dir` existed), so that the caller can make their
/// names durable ([`sync_parents`]).
pub(crate) fn create_private_dirs(dir: &Path) -> io::Result<Vec<PathBuf>> {
    let mut missing = Vec::new();
    for a in dir.ancestors() {
        if a.as_os_str().is_empty() {
            break;
        }
        match fs::symlink_metadata(a) {
            Ok(_) => break,
            Err(e) if e.kind() == io::ErrorKind::NotFound => missing.push(a.to_path_buf()),
            Err(e) => return Err(e),
        }
    }
    if missing.is_empty() {
        return Ok(missing);
    }
    let mut b = fs::DirBuilder::new();
    b.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt as _;
        b.mode(DIR_MODE);
    }
    b.create(dir)?;
    missing.reverse();
    Ok(missing)
}

/// Makes the names of the directories `created` durable in their parents
/// (Unix): an fsync of each one's parent directory, outermost first.
/// Windows cannot open a directory to sync it without a call the store's
/// crates do not make (ADR-0018 Open point 7), so there it does nothing.
pub(crate) fn sync_parents(created: &[PathBuf]) -> io::Result<()> {
    for d in created {
        sync_dir(d.parent().unwrap_or(Path::new("")))?;
    }
    Ok(())
}

/// An fsync of the directory `dir` (Unix), making the names in it durable;
/// nothing on other platforms (see [`sync_parents`]). An empty `dir` is a
/// relative path's directory, the current one: a path such as `w` has the
/// parent `""`, which no open accepts.
#[cfg(unix)]
pub(crate) fn sync_dir(dir: &Path) -> io::Result<()> {
    let dir = if dir.as_os_str().is_empty() {
        Path::new(".")
    } else {
        dir
    };
    fs::File::open(dir).and_then(|f| f.sync_all())
}

/// An fsync of the directory `dir` (Unix); nothing on other platforms
/// (see [`sync_parents`]).
#[cfg(not(unix))]
pub(crate) fn sync_dir(_dir: &Path) -> io::Result<()> {
    Ok(())
}

/// One schema migration: the step from version `from` to `from + 1`.
///
/// Each step runs in its own write transaction, which also records the new
/// version, so a crash leaves the store at the old or the new version,
/// never between (ADR-0003 Decision item 6; brief §6 Phase 2).
#[derive(Clone, Copy, Debug)]
pub struct Migration {
    /// The version the step starts from.
    pub from: u32,
    /// The step. It must not commit or record the version itself.
    pub apply: fn(&mut WriteTx<'_>) -> Result<()>,
}

/// The migrations of this build, by starting version. There are none yet:
/// version 1 is the first released schema.
pub const MIGRATIONS: &[Migration] = &[];

/// A schema: the version a build writes and the steps that bring an older
/// store to it.
#[derive(Clone, Copy, Debug)]
pub struct Schema<'a> {
    /// The version written by this schema.
    pub version: u32,
    /// Steps from older versions, one per starting version.
    pub migrations: &'a [Migration],
}

impl Schema<'static> {
    /// This build's schema.
    pub const CURRENT: Schema<'static> = Schema {
        version: SCHEMA_VERSION,
        migrations: MIGRATIONS,
    };
}

impl Default for Schema<'static> {
    fn default() -> Self {
        Schema::CURRENT
    }
}

/// How to create or open a store.
#[derive(Clone, Debug)]
pub struct OpenOptions<'a> {
    schema: Schema<'a>,
    cache_size: Option<usize>,
    check_integrity: bool,
}

impl Default for OpenOptions<'static> {
    fn default() -> Self {
        OpenOptions::new()
    }
}

impl OpenOptions<'static> {
    /// This build's schema, redb's default cache, no integrity check on
    /// open.
    pub fn new() -> Self {
        OpenOptions {
            schema: Schema::CURRENT,
            cache_size: None,
            check_integrity: false,
        }
    }
}

impl<'a> OpenOptions<'a> {
    /// Uses `schema` instead of this build's (for tests of the migration
    /// mechanism).
    pub fn schema<'b>(self, schema: Schema<'b>) -> OpenOptions<'b> {
        OpenOptions {
            schema,
            cache_size: self.cache_size,
            check_integrity: self.check_integrity,
        }
    }

    /// The engine's page cache, in bytes (redb's default when unset).
    pub fn cache_size(mut self, bytes: usize) -> Self {
        self.cache_size = Some(bytes);
        self
    }

    /// Whether opening walks the whole file to check it (redb's
    /// `set_check_integrity_on_open`). Off by default: it costs a walk of
    /// the file on every open. redb documents that an open without it
    /// trusts a file that was shut down cleanly and may panic if the file
    /// was corrupted outside redb; with it, such corruption is an error.
    ///
    /// The store never turns it on by itself for an open of the wallet's
    /// own store: whether the wallet does is decided in Phase 3 (ADR-0003,
    /// addendum "Decisions of the project owner", item 6). The backup
    /// verify path always opens its copy of a file handed in from outside
    /// with it ([`crate::backup`], ADR-0018 Decision item 6). (After an
    /// unclean shutdown redb repairs the file on open whatever this says.)
    /// Only the checked open is fuzzed (`fuzz/fuzz_targets/store_open.rs`,
    /// `fuzz/fuzz_targets/backup_verify.rs`).
    pub fn check_integrity(mut self, on: bool) -> Self {
        self.check_integrity = on;
        self
    }

    fn builder(&self) -> redb::Builder {
        let mut b = Database::builder();
        if let Some(n) = self.cache_size {
            b.set_cache_size(n);
        }
        b.set_check_integrity_on_open(self.check_integrity);
        b
    }

    /// Creates a new store at `path`, which must not exist, running `init`
    /// in the creating transaction: the file is a complete store with
    /// whatever `init` writes, or (on any error) no file is left. On Unix
    /// the file is created with [`FILE_MODE`] (owner only), as dcrwallet
    /// creates `wallet.db`; its directory must exist.
    pub fn create_with<F>(&self, path: &Path, init: F) -> Result<Store>
    where
        F: FnOnce(&mut WriteTx<'_>) -> Result<()>,
    {
        // Created here, with the owner-only mode, so that redb opens an
        // existing file and the mode is the store's choice, not the
        // engine's default.
        match create_private_file(path) {
            Ok(f) => drop(f),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                return Err(Error::exists(path.to_owned()));
            }
            Err(e) => return Err(Error::io(e)),
        }
        let made = self
            .builder()
            .create(path)
            .map_err(Error::from)
            .and_then(|db| Store::initialize(db, self.schema.version, init));
        if made.is_err() {
            // The file is this call's own, empty or holding no committed
            // store; remove it so that a retry starts clean.
            let _ = fs::remove_file(path);
        }
        made
    }

    /// Creates a new store at `path` with nothing but the schema.
    pub fn create(&self, path: &Path) -> Result<Store> {
        self.create_with(path, |_| Ok(()))
    }

    /// Opens the store at `path`: refuses a missing file, a file that is
    /// not a dcr-purse wallet store and a store newer than the schema, and
    /// migrates an older one.
    ///
    /// The file is locked while the store is open. A second open of it,
    /// from this or another process, fails at once with an `Other` error
    /// ("Database already open"); bbolt, and so dcrwallet, waits instead
    /// (spec 01 §5.2; `docs/PARITY.md`).
    pub fn open(&self, path: &Path) -> Result<Store> {
        match fs::metadata(path) {
            Ok(_) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Err(Error::missing()),
            Err(e) => return Err(Error::io(e)),
        }
        let db = self.builder().open(path)?;
        Store::checked(db, &self.schema)
    }

    /// Creates a store on an empty storage backend (for fault-injection
    /// tests and in-memory stores), running `init` in the creating
    /// transaction.
    pub fn create_on_backend<F>(&self, backend: impl StorageBackend, init: F) -> Result<Store>
    where
        F: FnOnce(&mut WriteTx<'_>) -> Result<()>,
    {
        let db = self.builder().create_with_backend(backend)?;
        {
            let txn = db.begin_read()?;
            if txn.list_tables()?.next().is_some() {
                return Err(Error::invalid("backend is not empty"));
            }
        }
        Store::initialize(db, self.schema.version, init)
    }

    /// Opens the store held by a storage backend, with the checks and
    /// migrations of [`OpenOptions::open`].
    pub fn open_on_backend(&self, backend: impl StorageBackend) -> Result<Store> {
        let db = self.builder().create_with_backend(backend)?;
        Store::checked(db, &self.schema)
    }
}

/// An open wallet store.
///
/// `Store` is `Send` and `Sync`. Any number of read transactions run at
/// once and never wait; write transactions run one at a time, a second
/// waiting for the first to end. Every write transaction from
/// [`Store::update`] commits with `Durability::Immediate`; the only relaxed
/// commits are those of [`InitialSync`] (ADR-0003 Decision item 9; ADR-0014
/// Open point 2).
pub struct Store {
    db: Database,
    /// Held by [`Store::update_with`] from before its write transaction
    /// begins until the count below has been brought up to date after the
    /// commit. redb's own writer lock does not cover that last step: it is
    /// released inside `WriteTransaction::commit` (redb 5.0.0 drops the
    /// transaction's write slot there, waking the next writer before
    /// `commit` returns), so a count kept only under it could be updated by
    /// two writers at once, losing an increment or landing a durable
    /// commit's reset after a later relaxed commit's increment. Writers are
    /// serialised by redb anyway, so the lock adds no contention.
    commit_lock: Mutex<()>,
    /// Commits made without durability since the last durable one, by
    /// every [`InitialSync`] of this handle together. Written only under
    /// `commit_lock`, after the commit it counts; read without it, so that
    /// asking for it never waits on a write transaction.
    relaxed: AtomicU32,
    /// Held for the whole of a backup ([`crate::backup`]): one backup runs
    /// at a time (ADR-0018 Decision item 1). A second is refused, not
    /// queued; neither readers nor writers take it.
    backup_lock: Mutex<()>,
}

/// How [`Store::update_with`] commits.
#[derive(Clone, Copy, Debug)]
pub(crate) enum CommitMode {
    /// `Durability::Immediate`.
    Durable,
    /// `Durability::None`, unless this is the `interval`-th commit since
    /// the last durable one, which is durable.
    Relaxed {
        /// Between 1 and [`crate::MAX_DURABLE_INTERVAL`].
        interval: u32,
    },
}

impl Store {
    /// Creates a store at `path` with this build's schema
    /// ([`OpenOptions::create`]).
    pub fn create(path: &Path) -> Result<Store> {
        OpenOptions::new().create(path)
    }

    /// Opens the store at `path` with this build's schema
    /// ([`OpenOptions::open`]).
    pub fn open(path: &Path) -> Result<Store> {
        OpenOptions::new().open(path)
    }

    pub(crate) fn new(db: Database) -> Store {
        Store {
            db,
            commit_lock: Mutex::new(()),
            relaxed: AtomicU32::new(0),
            backup_lock: Mutex::new(()),
        }
    }

    /// The engine's handle, for the backup's snapshot copy and its
    /// verification ([`crate::backup`]).
    pub(crate) fn database(&self) -> &Database {
        &self.db
    }

    /// The backup lock, or the refusal of a second backup while one runs
    /// ([`crate::backup::BACKUP_RUNNING_TEXT`]). A backup that panicked
    /// left nothing the lock protects, so its poison is ignored.
    pub(crate) fn backup_guard(&self) -> Result<MutexGuard<'_, ()>> {
        match self.backup_lock.try_lock() {
            Ok(g) => Ok(g),
            Err(TryLockError::Poisoned(p)) => Ok(p.into_inner()),
            Err(TryLockError::WouldBlock) => {
                Err(Error::invalid(crate::backup::BACKUP_RUNNING_TEXT))
            }
        }
    }

    fn initialize<F>(db: Database, version: u32, init: F) -> Result<Store>
    where
        F: FnOnce(&mut WriteTx<'_>) -> Result<()>,
    {
        let store = Store::new(db);
        store.update(|tx| {
            tx.open_all()?;
            tx.put_raw(TableId::Meta, MetaKey::Format.name(), FORMAT_MAGIC)?;
            tx.set_schema_version(version)?;
            init(tx)
        })?;
        Ok(store)
    }

    /// Checks the format and version of an opened database and runs the
    /// migrations it needs.
    fn checked(db: Database, schema: &Schema<'_>) -> Result<Store> {
        let store = Store::new(db);
        let found = store.inspect()?;
        if found > schema.version {
            return Err(Error::newer_version(found, schema.version));
        }
        let mut v = found;
        while v < schema.version {
            let step = schema
                .migrations
                .iter()
                .find(|m| m.from == v)
                .ok_or_else(|| Error::no_migration(v, schema.version))?;
            let next = v
                .checked_add(1)
                .ok_or_else(|| Error::no_migration(v, schema.version))?;
            store
                .update(|tx| {
                    // Re-read inside the transaction: the step applies to
                    // exactly the version it was written for.
                    if tx.schema_version()? != Some(v) {
                        return Err(Error::invalid("schema version changed during migration"));
                    }
                    (step.apply)(tx)?;
                    tx.open_all()?;
                    tx.set_schema_version(next)
                })
                .map_err(|e| Error::migration(v, e))?;
            v = next;
        }
        store.check_tables()?;
        Ok(store)
    }

    /// The stored schema version, after checking that the database is a
    /// dcr-purse wallet store.
    pub(crate) fn inspect(&self) -> Result<u32> {
        let txn = self.db.begin_read()?;
        let meta = match txn.open_table(definition(TableId::Meta)) {
            Ok(t) => t,
            Err(redb::TableError::TableDoesNotExist(_)) => {
                // A file redb created but nothing committed to (a create
                // that crashed before its first commit) holds no tables;
                // anything else without `meta` is someone else's.
                return Err(if txn.list_tables()?.next().is_none() {
                    Error::not_initialized()
                } else {
                    Error::foreign("no meta table")
                });
            }
            Err(e) => return Err(e.into()),
        };
        use redb::ReadableTable as _;
        match meta.get(MetaKey::Format.name())? {
            Some(g) if g.value() == FORMAT_MAGIC => {}
            Some(_) => return Err(Error::foreign("unknown format row")),
            None => return Err(Error::foreign("no format row")),
        }
        let v = meta
            .get(MetaKey::Version.name())?
            .map(|g| meta_u32(g.value(), "schema version"))
            .transpose()?
            .ok_or_else(Error::not_initialized)?;
        if v == 0 {
            return Err(Error::foreign("schema version 0"));
        }
        Ok(v)
    }

    /// Fails unless every table of the schema is in the file.
    pub(crate) fn check_tables(&self) -> Result<()> {
        let txn = self.db.begin_read()?;
        let names: Vec<String> = txn.list_tables()?.map(|h| h.name().to_owned()).collect();
        for id in TableId::ALL {
            if !names.iter().any(|n| n == id.name()) {
                return Err(Error::missing_table(*id));
            }
        }
        Ok(())
    }

    /// Begins a read transaction: a snapshot of the last commit.
    pub fn read(&self) -> Result<ReadTx> {
        Ok(ReadTx::new(self.db.begin_read()?))
    }

    /// Runs `f` on a read transaction.
    pub fn view<R, E: From<Error>>(&self, f: impl FnOnce(&ReadTx) -> Result<R, E>) -> Result<R, E> {
        let tx = self.read()?;
        let r = f(&tx)?;
        tx.close()?;
        Ok(r)
    }

    /// Runs `f` in a write transaction and commits it durably when `f`
    /// returns `Ok`. Write transactions run one at a time: this waits for
    /// one in progress to end, so `f` must not begin another write
    /// transaction of the same store (that never returns, or panics).
    ///
    /// On `Err` nothing `f` wrote is kept, with one exception: an error
    /// for which [`Error::commit_outcome_unknown`] is true. The engine then
    /// failed inside the commit, after the transaction's writes may have
    /// reached storage (a failed sync, say), and the commit is kept or
    /// lost as a whole, which is known only after a reopen: redb refuses
    /// every later write transaction of this handle, and a read in this
    /// process may not show the commit even where the file holds it. The
    /// caller closes the store, reopens it and reads back what the commit
    /// was to write before acting on it (redb 5.0.0
    /// `WriteTransaction::commit`; `tests/store.rs`,
    /// `a_failed_commit_reports_an_unknown_outcome`).
    pub fn update<R, E: From<Error>>(
        &self,
        f: impl FnOnce(&mut WriteTx<'_>) -> Result<R, E>,
    ) -> Result<R, E> {
        Ok(self.update_with(CommitMode::Durable, f)?.0)
    }

    /// Runs `f` in a write transaction committed as `mode` says, and
    /// returns `f`'s value and whether the commit was durable.
    pub(crate) fn update_with<R, E: From<Error>>(
        &self,
        mode: CommitMode,
        f: impl FnOnce(&mut WriteTx<'_>) -> Result<R, E>,
    ) -> Result<(R, bool), E> {
        // Held to the end of this function: the count read below is the one
        // this commit extends or resets, and no other commit lands between
        // this commit and the count's update. A panic in an earlier `f`
        // poisons the lock with the count intact (the count is only written
        // after a commit returned, by code that cannot panic), so the
        // poison is ignored.
        let _serial = self
            .commit_lock
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let mut txn = self.db.begin_write().map_err(Error::from)?;
        let durable = match mode {
            CommitMode::Durable => true,
            CommitMode::Relaxed { interval } => {
                self.relaxed.load(Ordering::SeqCst) >= interval.saturating_sub(1)
            }
        };
        let durability = if durable {
            Durability::Immediate
        } else {
            Durability::None
        };
        txn.set_durability(durability).map_err(Error::from)?;
        let r = {
            let mut tx = WriteTx::new(&txn);
            f(&mut tx)
        };
        let r = match r {
            Ok(r) => r,
            Err(e) => {
                // Nothing of the transaction is kept whether or not the
                // abort itself reports an error; the caller's error is the
                // one that matters.
                let _ = txn.abort();
                return Err(e);
            }
        };
        match txn.commit() {
            Ok(()) => {}
            // Rolled back: nothing is kept.
            Err(e @ redb::CommitError::TransactionPoisoned) => return Err(Error::from(e).into()),
            // Kept or lost as a whole, unknown until a reopen.
            Err(redb::CommitError::Storage(e)) => return Err(Error::commit_unknown(e).into()),
            // A kind redb may add later: whether it rolled back is not
            // documented, so the outcome is unknown.
            Err(e) => return Err(Error::commit_unknown_other(e).into()),
        }
        // redb has released its writer lock by now; `commit_lock` still
        // excludes every other commit of this handle.
        #[cfg(test)]
        tests::AFTER_COMMIT.with(|h| h.borrow_mut().as_mut().map(|f| f()));
        if durable {
            // A durable commit makes every earlier one durable with it.
            self.relaxed.store(0, Ordering::SeqCst);
        } else {
            let n = self.relaxed.load(Ordering::SeqCst).saturating_add(1);
            self.relaxed.store(n, Ordering::SeqCst);
        }
        Ok((r, durable))
    }

    /// Commits made without durability since the last durable commit, by
    /// every [`InitialSync`] of this handle together, as of the last commit
    /// whose count update has finished. Never waits on a write transaction.
    pub(crate) fn relaxed_pending(&self) -> u32 {
        self.relaxed.load(Ordering::SeqCst)
    }

    /// Starts initial sync's relaxed header-and-filter commits
    /// ([`InitialSync`]).
    pub fn initial_sync(&self) -> InitialSync<'_> {
        InitialSync::new(self)
    }

    /// Closes the store, reporting an error the engine hit flushing or
    /// unlocking the file (dropping the store closes it too, ignoring
    /// such an error).
    pub fn close(self) -> Result<()> {
        self.db.close()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::sync::mpsc;
    use std::time::Duration;

    use super::*;

    thread_local! {
        /// Run by [`Store::update_with`] on this thread between a
        /// successful commit and the update of the relaxed-commit count,
        /// the window redb's writer lock no longer covers.
        pub(super) static AFTER_COMMIT: RefCell<Option<Box<dyn FnMut()>>> =
            const { RefCell::new(None) };
    }

    /// A durable commit's reset of the relaxed-commit count cannot land
    /// after relaxed commits another thread made once redb released its
    /// writer lock. Thread A commits durably and, in the window after its
    /// commit, waits (up to a time limit) for thread B to make five
    /// relaxed commits. Without a lock held past the commit, B makes them
    /// at once, A's reset then wipes their count, and `finish` would skip
    /// its durable commit with five batches non-durable; with it, B waits
    /// for A's reset and the count ends at 5.
    #[test]
    fn a_durable_reset_never_lands_after_a_later_relaxed_commit() {
        let store = OpenOptions::new()
            .create_on_backend(redb::backends::InMemoryBackend::new(), |_| Ok(()))
            .expect("create");
        let (committed_tx, committed_rx) = mpsc::channel::<()>();
        let (batches_tx, batches_rx) = mpsc::channel::<()>();
        let (done_tx, done_rx) = mpsc::channel::<()>();
        std::thread::scope(|scope| {
            let a = &store;
            scope.spawn(move || {
                AFTER_COMMIT.with(|h| {
                    *h.borrow_mut() = Some(Box::new(move || {
                        let _ = committed_tx.send(());
                        // Long enough for B's five in-memory commits when
                        // nothing holds B back.
                        let _ = batches_rx.recv_timeout(Duration::from_millis(500));
                    }));
                });
                a.update(|tx| tx.put_raw(TableId::Tickets, &[1; 32], b""))
                    .expect("durable update");
                AFTER_COMMIT.with(|h| h.borrow_mut().take());
                done_tx.send(()).expect("B waits");
            });
            let b = &store;
            scope.spawn(move || {
                committed_rx.recv().expect("A committed");
                let mut sync = b.initial_sync();
                for _ in 0..5 {
                    let ((), durable) = sync
                        .batch(|x| x.set_processed_marker(&crate::keys::Hash32([2; 32])))
                        .expect("batch");
                    assert!(!durable);
                }
                let _ = batches_tx.send(());
                done_rx.recv().expect("A's update returned");
                assert_eq!(sync.pending(), 5, "A's reset wiped B's relaxed commits");
                // So `finish` commits durably rather than skipping.
                sync.finish().expect("finish");
                assert_eq!(b.relaxed_pending(), 0);
            });
        });
    }

    /// The directory of a bare relative name is `""`, which no open
    /// accepts: syncing it syncs the current directory. A backup into the
    /// directory `""` once published its file and then failed here.
    #[test]
    fn an_empty_directory_path_syncs_the_current_directory() {
        sync_dir(Path::new("")).expect("empty path");
        sync_dir(Path::new(".")).expect("dot");
        sync_parents(&[PathBuf::from("a-bare-name")]).expect("bare name");
    }
}
