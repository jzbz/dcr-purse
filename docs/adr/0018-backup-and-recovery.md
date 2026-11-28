# ADR-0018 — D15: Backup and recovery of the store

- **Status:** **Accepted** as proposed (decision D15; project owner,
  2026-11-19), with the ten open points answered as recommended; see
  "Decision by the project owner" before "What was not tested". The
  repository facts and measurements under Context are not outcomes of it;
  ADR-0003's Decision item 8 and its Open point 6, both accepted, are the
  starting point, and ADR-0003 gains the addendum of Open point 9.
- **Settled, 2026-11-21 (project owner):** the open points BK1 to BK6 of
  the addendum "The `purse-store` part, as built", each as recommended:
  the store's file name is `wallet.redb`. `libc` stays a direct, Unix-only
  dependency of `purse-store`. See "Addendum, 2026-11-21" at the end.
- **Built, 2026-11-26:** BK4, the transaction tables' consistency check,
  run by `verify_backup`; one open point, BK7 (where the check lives),
  pending owner. See "Addendum, 2026-11-26" at the end.
- **Date:** 2026-11-19

## Context

**The question.** What a backup of dcr-purse's own store is and how it is
made, both with the wallet stopped and while it runs; through which
surface a user asks for one; what a user restores from; whether backups
are encrypted; how a backup is verified; what a crash or a power cut
during a backup leaves; and what the user documentation promises. The
brief asks for "file copy when stopped, a consistent snapshot while
running if `redb` allows it, and what a user restores from" (brief §9,
D15) and for "a documented, tested procedure for backing up the dcr-purse
database" before 1.0 (brief §6 Phase 13).

**By when.** The snapshot copy is a `purse-store` operation and belongs in
its API when Phase 2 designs it; the surface is Phase 7 (JSON-RPC) and
Phase 8 (daemon and tools); the documented, tested procedure is a Phase 13
exit item.

**Decisions in force.**

- [ADR-0003](0003-wallet-store-engine-and-schema.md) (Accepted): redb 5
  with `Durability::Immediate` and one-phase commits with checksums;
  quick repair off (Open point 3); Decision item 6, schema versioning,
  "the same check runs on a store restored from a backup"; item 7, an
  engine file-format break crossed by a standalone migrator, and "the
  logical export cannot serve as the copy"; item 8, the backup is "a
  snapshot copied table by table into a new file in one durable commit
  ..., which also compacts it. A raw copy of the file is safe only while
  no process has it open"; Open point 6, "compact only through backup in
  1.0".
- [ADR-0013](0013-no-import-seed-restore-only.md) (Accepted): no
  dcrwallet database is read; migration is by seed restore, which does not
  recover the state listed in its Decision item 4. A backup of dcr-purse's
  store is the only thing that carries that state across a lost disk.
- [ADR-0012](0012-standing-quirk-policy.md) (Accepted): external input,
  "a file" among it, never panics the process (Rule 2); secrets never move
  to a sink their supplier does not hold (Rule 6).
- [ADR-0008](0008-concurrency-model.md) (Accepted): threads; release
  builds abort on panic (`Cargo.toml:32-40`); readers never wait for the
  writer.
- [ADR-0005](0005-name-data-directory-and-import-front-end.md)
  (Accepted): the store lives in `<appdata>/<net>/`, "named per ADR-0003";
  ADR-0003 does not name it (Open point 6 below). A separate binary keeps
  the daemon's options identical to dcrwallet's (its "Options considered,
  Import entry point").

### What dcrwallet offers

All Go citations are to dcrwallet `54b85b33`.

- **No backup surface.** JSON-RPC lists `backupwallet` among dead names
  (`internal/rpc/jsonrpc/methods.go:192`); because the name has no
  `dcrjson` registration, a client gets `-32600 "Invalid request"`, not
  the intended `-1 "Method unimplemented"` (QK-0432; spec 16 §4.9). The
  gRPC API has no backup RPC (`rpc/api.proto`; specs 19 and 20). The
  database layer has `Copy`, bbolt's `Tx.Copy` inside a read transaction
  (`wallet/walletdb/interface.go:193`, `wallet/internal/bdb/db.go:288-296`),
  and nothing outside tests calls it.
- **No documentation.** Neither `README.md` nor `docs/` mentions backing
  up `wallet.db`. The creation prompt tells the user to keep the seed
  (`internal/prompt/prompt.go:288-293`), and the gRPC documentation says
  "the seed is not saved in the wallet database and clients should make
  their users backup the seed" (`rpc/documentation/api.md:130-132`).
- **What dcrwallet users do today**, then, is keep the seed, and, if they
  think of it, copy `wallet.db` while dcrwallet is stopped. A second
  opener of a running dcrwallet's database blocks forever (bbolt is
  opened with no timeout, `wallet/internal/bdb/db.go:321`; QK-0009); a
  plain file copy is not stopped by bbolt's advisory lock on Unix. What a
  copy of a running dcrwallet's file contains was not measured. Whether
  Decrediton offers any backup of the wallet file is not known: its source
  was not among the inputs of the specs (brief §6 Phase 0).

### What redb 5.0.0 provides

Checked against the redb 5.0.0 source in the Cargo registry.

- **A read transaction is a snapshot**: "only data committed before
  calling this method is visible", and it coexists with writes
  (`src/db.rs:606-613`). Pages it can reach are not reused until it ends
  (`docs/design.md:387-393`), so the file can grow by what writers change
  meanwhile.
- **No page-level copy.** redb has no backup or copy API (no such item in
  `src/`); a copy is made by reading a snapshot and inserting its rows
  into a new database, as ADR-0003's spike did.
- **`Database::create` opens an existing database** instead of refusing
  it, and creates the file with `truncate(false)` (`src/db.rs:822-829`,
  `:2403-2411`): a backup must create its file under a name it has
  checked is free.
- **Durability.** A commit with `Durability::Immediate` is persistent when
  `commit` returns (`src/transactions.rs:381-388`), by one `fsync` after
  data and checksums are written (`docs/design.md:339-354`). A crash rolls
  non-durable commits back to the last durable one (`:326-337`).
- **Locking.** The default mode, `ExclusiveWriter`, locks the whole file
  and excludes every other process (`src/db.rs:2241`, `:2254-2257`). On
  Unix the lock is advisory, so it stops other redb opens, not a file
  copy; on Windows, redb's region locks are mandatory
  (`src/tree_store/page_store/file_backend/range_lock.rs:90-91`), so a
  copy of a running wallet's file should fail there (not tested on
  Windows).
- **Other processes reading a running store** need `SingleWriter` mode
  (`src/db.rs:2258-2260`), introduced in 4.5.0 on 2026-11-05
  (`CHANGELOG.md:127-138`); 4.6.0 fixed seven defects in the
  multi-process modes and 5.0.0 one more (`CHANGELOG.md:76-77`,
  `:99-117`).
- **Untrusted files.** A plain open "trusts a file that was shut down
  cleanly and may panic if it was corrupted outside redb";
  `Builder::set_check_integrity_on_open` verifies every page's checksum
  and returns `Corrupted` instead (`src/db.rs:831-835`, `:2376-2389`). The
  check "does not apply to read-only opens" and commits to the file it
  opens (`:2383`). Checksums are XXH3, not cryptographic, and redb's own
  design notes advise two-phase commits for malicious input
  (`docs/design.md:356-385`).
- **Compaction** needs no live transaction or savepoint on the handle
  (`src/db.rs:1237-1262`); `CompactionMode::Balanced` keeps page contents,
  `Maximum` also packs partly filled pages (`:2280-2297`).

### Experiments

A prototype, kept outside the repository, on redb 5.0.0, Rust 1.100.0, an
x86_64 Linux desktop with an NVMe drive under btrfs on dm-crypt, under load
from other work. The store: three tables of 600,000 rows each (32-byte keys
with 300-byte values, 36 with 60, 8 with 200), 649 MB on disk. The backup:
a read transaction opened on the live store, every table copied in key
order into a new file created under a temporary name, a digest of every row
of the snapshot written into that file in the same write transaction,
committed with `Durability::Immediate`, the file closed, renamed to its
final name, the directory synced. A "writer" is a thread of the same
process committing one small durable transaction after another (one row
and a counter), as a wallet ingesting blocks does. Single runs unless
stated. The first set of runs is in the table; a second set, made after
review on the same machine under heavier load from other work (writer
commit p99 of 50 to 475 ms during backups, against about 11 to 40 ms in
the first set), re-measured the points the first set did not settle and
is marked "rerun".

| Measurement | Result |
|---|---|
| Hot backup, no writer (3 runs) | 2.74, 2.82, 2.86 s; 570 MB; digest of the copy equal to the snapshot's every time |
| Hot backup, writer running (3 runs) | 2.76 to 3.04 s; 330 to 345 writer commits meanwhile; copy equal to the snapshot every time |
| Writer commit latency, 1 GiB caches (default; 3 runs, then 2 more) | p50 5.3 ms before, 5.5 ms during; p99 5.6 ms before, 11.7 to 39.9 ms during; maximum during 338 to 787 ms in the first three runs, 47 and 438 ms in the two more |
| Writer commit latency, backup file cache 64 MiB (3 runs) | p99 10.3 to 10.7 ms during; maximum 11 to 63 ms |
| Rerun: backup file cache 64 MiB against 1 GiB, alternating, 6 runs each | p99 during 53 to 185 ms against 95 to 474 ms; maximum during 221 to 575 ms against 205 to 812 ms (medians of the maxima about 290 against 590 ms) |
| Peak memory of the backing-up process | 108 MiB with 64 MiB caches on both files; 557 MiB with the live store's cache at 1 GiB; 935 MiB with both at 1 GiB. Bounded by the caches, not by the store's size |
| Destination committed in batches (20,000 or 100,000 rows, non-durable, last one durable) | no change in time, memory or size against one transaction |
| Live store size during the writer runs | unchanged (the writer's commits are small) |
| Backup without compaction killed at a random point (80 runs, 0 to 3.2 s: 40 with one write transaction, as Decision item 1 proposes, and 40 with batched non-durable commits, which it does not) | the live store reopened with its digest unchanged 80 of 80; the final name present only when the backup had finished (12: 8 and 4), and verified each time; otherwise the temporary file: empty (65), or complete with its digest record when the kill fell between commit and rename (3, all with one transaction). Never a partial copy |
| Rerun: backup with compaction (Decision items 1 to 3: one write transaction, `Balanced` compaction of the temporary file, close, rename) killed at a random point (60 runs, 0 to 8 s) | the live store reopened with its digest unchanged 60 of 60; the final name present only when the backup had finished (7), verified each time; otherwise the temporary file: empty (35), or opening with the integrity check and holding the backup record (18), killed after the commit, most of them during compaction by the timing (the run did not record the phase; their digests were not checked). Never a file under the final name that was not complete |
| Raw copy of the open store, writer of one row per commit (15 copies; an in-process chunked copy whose chunk size and pause were not recorded) | every copy needed repair on open; all 15 opened consistent |
| Raw copy of the open store, writer of 200 or 2,000 rows per commit (50 copies, 3 runs; the same unrecorded chunked copy) | 22 copies could not be opened at all ("Failed to repair database. All roots are corrupted"): 19 of 20, 2 of 15, 1 of 15. None opened inconsistent |
| Rerun: raw copy with `cp --reflink=never` (15 copies each), writer of 1, 200 or 2,000 rows per commit | unopenable 13, 14 and 1 of 15; the rest opened consistent after repair |
| Rerun: raw copy in process, 128 KiB chunks, no pause (15 copies each), writer of 200 or 2,000 rows per commit | unopenable 15 and 12 of 15 (some with "Primary is corrupted despite 2-phase commit"); the rest opened consistent after repair |
| File copy of the closed store | 0.86 s; opened without repair; digest equal |
| Second process opening a store the first holds | `Database::open` and `ReadOnlyDatabase::open` both fail: "Database already open. Cannot acquire lock." (Linux) |
| Sizes | live store 649 MB; backup 570 MB; the backup compacted afterwards, `Balanced` or `Maximum`, 412 MB in 2.7 to 3.7 s (single runs); the live store compacted in place, 524 MB (`Balanced`, 15.5 s) or 416 MB (`Maximum`, 11.3 s), which needs it closed to everyone else |
| Rerun: compaction of a fresh backup, 3 runs each mode | identical sizes (422 MB from 570 MB; the live store had grown by the writer runs) and, in a separate check on a 150,000-row store, identical leaf and branch page counts and fragmented bytes per table: the snapshot copy inserts in key order and leaves nothing for `Maximum` to pack. `Balanced` 4.3, 2.4, 2.5 s; `Maximum` 2.3, 2.3, 2.3 s; inside the backup, before the rename, 3.1 to 3.5 s against 3.0 to 4.1 s |
| Verifying a backup | read-only open and digest of every row, 0.28 to 0.84 s; `check_integrity` on a writable open, 0.67 s |

What the table shows. A consistent backup of a running store works with
redb 5.0.0 as ADR-0003 found, at about 230 MB/s here, and its memory is set
by the caches. A small cache on the backup file appeared to reduce the
wallet's worst commit stalls behind the backup's final sync in most runs,
but the ranges overlap in both sets and the effect is not separated from
the machine's load; it is a reasonable default, not a measured
guarantee. A raw copy of a running store is not merely unsafe in
principle: with a plain `cp` it gave an unopenable file in 28 of 45
copies, 13 of 15 even under a writer of one row per commit, and every
copy that opened needed repair. How often a copy straddles a commit
depends on how long the copy takes against the writer's pace, so the
rate is specific to this machine and load; that some copies are
unopenable is not. A killed backup never left a file under the final name
that was not complete, with or without the compaction step. The fresh
backup is smaller than the live file but not compact: ADR-0003 item 8's
"which also compacts it" holds only after an explicit compaction of the
backup file (Decision item 2), where `Balanced` and `Maximum` give the
same file. Power loss was not simulated (see "What was not tested").

## Options considered

**What a backup is.**

- **B1.** A snapshot of one read transaction copied into a new redb file,
  in process, by the running wallet (ADR-0003 item 8).
- **B2.** A raw copy of the store file while the wallet runs. Measured
  above: unopenable copies.
- **B3.** A file copy with the wallet stopped. Safe, measured above; needs
  the wallet down.
- **B4.** A redb persistent savepoint. It lives inside the same file, so
  it survives no loss of the disk; not a backup.
- **B5.** Another process reading the running store under `SingleWriter`
  mode. Changes the lock protocol of every open for a mode with eight
  defects fixed in its first two weeks.
- **B6.** A dump in another format: the logical export or a new one. The
  logical export masks values and digests headers and is not a reload
  format (ADR-0003 item 7); a new format needs a loader, its own tests and
  versioning, to reproduce what B1's file already is.
- **B7.** An atomic file-system snapshot (btrfs, ZFS, LVM, Volume Shadow
  Copy) taken by the user. It captures the file as a power cut would,
  which redb recovers from (ADR-0003, power-loss sweep); not tested here
  and outside dcr-purse's control.

**Surface.**

- **S1.** JSON-RPC `backupwallet`, the name dcrwallet reserves as dead.
- **S2.** A new gRPC method. Changes `api.proto` (W5); Decrediton would not
  call it.
- **S3.** A separate tool binary for a stopped wallet: verify and
  restore.
- **S4.** New daemon options (`--backup=`, `--restorebackup=`), as
  `--create` is an option. They add options dcrwallet does not have.
- **S5.** Documentation only (B3 and B7).

**Encryption.**

- **E1.** None beyond what the store already seals.
- **E2.** An optional passphrase-encrypted container around the backup
  file (Argon2id and XChaCha20-Poly1305, both already in the graph through
  `purse-snacl`).

| | B1 + S1 + S3, E1 (proposed) | B3 + S5 only | B1 + S2 | B5 | B6 |
|---|---|---|---|---|---|
| Wallet stays up | yes | no | yes | yes | yes |
| Consistent | one snapshot | yes | one snapshot | one snapshot | one snapshot |
| Restorable without new code | yes: it is a store | yes | yes | yes | no: a loader |
| Divergence from dcrwallet | `backupwallet` answers | none | a new gRPC method | none on the wire | as B1 |
| Risk | a JSON-RPC client chooses where the wallet writes (bounded by item 4) | users copy a running store anyway | Decrediton cannot use it | young locking code on every open | a second format |

## Decision (accepted, 2026-11-19)

1. **A backup is a consistent snapshot copied into a new store file
   (B1).** The running wallet opens a read transaction and copies every
   table, rows as raw bytes in key order, into a new redb file, with a
   64 MiB cache on that file, in one write transaction committed with
   `Durability::Immediate`. In the same transaction it writes a backup
   record into the copy's `meta` table: the format of the record, the
   network, the schema version, the dcr-purse version, the time from the
   injected clock (ADR-0008), the number of rows and a digest of every
   row of the snapshot (table name, key and value, length-prefixed),
   BLAKE-256 from `dcroxide-crypto`. Live write transactions and readers
   are not blocked. One backup runs at a time.
2. **Compaction belongs to the backup.** After the commit the copy is
   compacted with `CompactionMode::Balanced` until no further compaction
   is possible (412 against 570 MB above, 2.3 to 4.3 s; on a fresh copy
   `Maximum` gives the same file, Open point 5). This is the
   compaction ADR-0003 Open point 6 assigns to backup; the wallet's own
   store is never compacted in place in 1.0.
3. **Publication is atomic.** The file is written as `.partial/<name>`
   under `backups/`, a directory reserved for the backup's temporary
   files, which no name a client can give reaches (item 4 refuses path
   separators and a leading dot). Since one backup runs at a time, every
   file in `.partial/` when a backup starts was left by an interrupted
   one, and the backup removes them all; it removes nothing outside
   `.partial/`. It refuses to start if `backups/<name>` exists. After
   compaction it closes the file, renames it to `backups/<name>`, a rename
   within one file system, and syncs `backups/` and `.partial/` (Windows,
   where a directory cannot be synced: Open point 7). A crash at any point
   leaves the live store as it was, and either no file under the final
   name or a complete, verified one. Measured with process kills: 40 kills
   of the copy and publication in one write transaction without compaction
   (and 40 with batched commits), and 60 kills of the whole procedure with
   compaction; power loss is not tested. The experiments wrote the
   temporary file beside the final name; the reserved directory changes
   only its path and the rename's source directory, and was not itself
   run. The compaction phase rewrites only the temporary file, so the
   ordering (rename last) is what keeps an interrupted compaction out of
   the final name. A finished backup cannot be told from an interrupted
   one by its contents (the sweeps left complete temporary files with
   their backup record when a kill fell after the commit), so the rule
   that decides what may be deleted is the directory, never the file's
   name or contents: a file under `backups/` itself, whatever its name, is
   never removed or overwritten by dcr-purse.
4. **Surface: JSON-RPC `backupwallet "name"` (S1).** One string
   parameter, a file name without directory components; the file is
   written in `<appdata>/<net>/backups/`, created on first use. A name
   that is empty, contains a path separator or `..`, begins with a dot
   (which keeps `.partial` and every hidden name out of the client's
   reach), or exists is refused with the invalid-parameter code and a
   message of dcr-purse's own. The reply is the absolute path written,
   after the rename. Once started, the backup does not observe its
   request's context: it runs to publication or failure as one local
   operation whatever happens to the caller, logs the outcome with the
   path, and writes its reply if the connection still takes it; only
   process shutdown stops it, leaving its temporary file for the next
   backup to remove. The reason is dcrwallet's HTTP POST transport, which
   by source analysis cancels a POST handler's context about 10 s after
   the request began (`http.Server.ReadTimeout`,
   `internal/rpc/jsonrpc/server.go:95`, `:109`; QK-0447; spec 16 §7.3),
   which websocket requests escape. The copy and compaction measured
   above take about 6 to 7 s on a 570 MB store, under that limit; ADR-0003's
   hot backup of its largest store, 8.5 to 9.2 s, plus 2.3 to 4.3 s of
   compaction, is over it, and slower disks and larger wallets were not
   measured. Whether dcr-purse reproduces the cancellation is spec 16's
   Open question O1 and its decision D5, not settled; this item makes the
   answer irrelevant to `backupwallet`: a `dcrctl` caller of a long backup
   receives the reply as long as its own HTTP client waits, and a caller
   that gave up finds the file under the name it chose and the outcome in
   the log. The method stays out of
   `help`'s listing, as dcrwallet's dead names are (`noHelp`), so `help`
   stays byte-identical to dcrwallet's (W4). The restriction to one
   directory is deliberate: a JSON-RPC credential already spends funds,
   but writing files where the client chooses would add a different power,
   creating a file in any directory the wallet's user can write. No gRPC
   method (S2) in 1.0.
5. **A separate tool for a stopped wallet (S3)**, in `purse-tools`
   beside the two tools ported from dcrwallet (brief §5), so the daemon's
   options stay dcrwallet's:
   - `verify <file>`: copies the file to a temporary location, opens the
     copy with `set_check_integrity_on_open`, checks the backup record
     (format, network, schema version not newer than the build), the
     digest and row count, and the store's consistency check (ADR-0003
     item 4); reports and deletes the copy. The backup file itself is
     never written after publication.
   - `restore <file>`: runs `verify`'s checks on a copy made in
     `<appdata>/<net>/`, removes the backup record, migrates an older
     schema (ADR-0003 item 6), and renames the copy to the store's name,
     refusing if a store exists: the user moves the existing store aside
     first. It does not cross a redb file-format break: `purse-tools` is
     in the main workspace, which links one redb major (ADR-0006 rule 5),
     so a backup in an older redb format is refused with a message naming
     the standalone migrator of ADR-0003 item 7, which the user runs on
     the backup first. The tool refuses to run while the store is open
     (redb's lock).
   - With the wallet stopped, a plain file copy of the store is also a
     valid backup (B3), and `verify` accepts it (it has no backup record,
     so only the integrity and consistency checks apply).
6. **A file handed in from outside is never opened trusting it.** A
   backup given to `verify` or `restore` is external input: it is opened
   only by a writable open with `set_check_integrity_on_open`, on a copy,
   never by a plain or read-only open, either of which can panic on a
   damaged file and, with `panic = "abort"`, end the process (ADR-0012
   Rule 2). The wallet's own store, which only dcr-purse writes, keeps
   redb's plain open and its repair after an unclean shutdown.
7. **No encryption added (E1).** A backup holds what the store holds and
   is as sensitive as the store: private keys sealed by the private
   passphrase (scrypt and secretbox, spec 02), account extended public
   keys sealed by the public passphrase, which defaults to `public`
   (spec 21 §6; `config.go:351`, `wallet/wallet.go:66`), and
   transactions, addresses, account names, VSP records and preferences in
   the clear, as in the live store and as in dcrwallet's `wallet.db`. The
   seed is not stored. The documentation says so (item 9).
8. **What a user restores from.** The seed restores keys, accounts by
   number and on-chain history within discovery's limits (ADR-0013, W2).
   A backup restores, in addition, as of the backup's instant, everything
   ADR-0013 Decision item 4 lists except two items that are not store
   state: outpoint locks, which live in memory only, and the mixed-account
   configuration, which is the options `mixedaccount`,
   `ticketsplitaccount` and `changeaccount` in the configuration file
   (ADR-0013 item 4, notes; dcrwallet `config.go:173-177`, resolved by
   account name at start-up, `dcrwallet.go:327`). The account names those
   options refer to are in the store, so a configuration file kept beside
   the backup resolves again after a restore. A restored backup is older than the chain: the wallet
   syncs forward from its stored tip, and because the processed-
   transactions marker is behind the new tip it runs address discovery
   and a rescan from that marker, as dcrwallet does after any long
   shutdown (`wallet/rescan.go:476-492`; RPC mode `chain/sync.go:355-385`;
   SPV mode `spv/sync.go:1851-1874`).
9. **What the user documentation promises.**
   - Keep the seed; it is the only recovery when every copy of the store
     is lost, and a backup does not replace it.
   - A backup covers the store only. The configuration file
     (`dcr-purse.conf`, which holds among other options the mixed-account
     settings) and the TLS files (`rpc.cert`, `rpc.key` and any client CA
     file) are separate files in the data directory; keep a copy of them
     with the backup. After a restore, a kept configuration file resolves
     its account options again, because the account names come back with
     the store.
   - `backupwallet` writes a consistent copy of the whole wallet at one
     instant, without stopping the wallet; the file in `backups/` may be
     copied anywhere once the call has returned.
   - With the wallet stopped, copying the store file is a valid backup.
     Copying it while the wallet runs can produce a file that cannot be
     opened at all; never do it. An atomic file-system snapshot captures
     the file as a power cut would, which the store recovers from; this
     sentence waits for B7 to be tested (Phase 13).
   - A backup is as sensitive as the wallet: anyone holding it can read
     the wallet's addresses and history, and can try passphrases offline
     against its private keys. Store it accordingly; a strong private
     passphrase is the protection.
   - Restoring a backup returns everything to the backup's instant:
     accounts, names, imported keys and scripts, VSP records and
     preferences created later are gone; the wallet then catches up with
     the chain and runs discovery, which finds later funds on addresses
     within the gap limits; addresses handed out after the backup may be
     handed out again.
   - A backup opens in the dcr-purse version that made it and in later
     ones (across a change of the store's file format, after the
     standalone migrator named in `restore`'s message has converted it),
     never in an earlier one, and never in dcrwallet; dcrwallet
     backups (`wallet.db`) are not read (ADR-0013).
   - Run `verify` on a backup before relying on it.
   - `backups/.partial/` belongs to the backup itself: whatever is in it
     is deleted by the next backup. Nothing else in `backups/` is ever
     deleted or overwritten by dcr-purse.

**PARITY entries.** `backupwallet` answers, where dcrwallet answers
`-32600 "Invalid request"` (QK-0432, whose disposition changes for that
one name; the other seven dead names keep dcrwallet's reply). The tool is
new and has no dcrwallet counterpart; no other surface changes.

## Consequences

- **`purse-store`** gains the snapshot copy, the backup record, the digest
  and the verify path in Phase 2, tested with the store: a property test
  that a backup taken under a concurrent writer equals the snapshot; the
  kill sweep above as a test; the power-loss rig of ADR-0003 run over a
  backup (see "What was not tested"); a corrupted-backup corpus that
  `verify` and `restore` refuse without panicking; fuzzing of the backup
  record's decoder (brief §7 Layer 5).
- **Phase 7** adds `backupwallet` with its refusals and the PARITY line,
  tested with: every refused name (empty, a separator, `..`, a leading
  dot, an existing name); a backup named `w.tmp` followed by one named
  `w`, and a file a user placed in `backups/` under any name, each
  untouched by later backups, while files in `.partial/` are removed;
  a backup over HTTP POST that lasts longer than the server's read
  timeout (the backup slowed by an injected delay), which must publish
  and reply; and a caller that disconnects mid-backup, after which the
  file must still be published and logged;
  **Phase 8** the tool; **Phase 13** the documentation and a CI job that
  restores a backup made by every released version (beside ADR-0003 item
  6's fixture of every schema version).
- **Cost while running.** A backup reads the whole store through the
  wallet's cache, displacing hot pages until they are read again, and
  holds a read transaction for its duration, so pages freed meanwhile are
  not reused and the store can grow by what is written during the backup.
  The wallet's commits were slowed to a p99 of about 11 ms (maximum 63 ms)
  with the 64 MiB backup cache on a lightly loaded run, and to a p99 of
  53 to 185 ms (maximum up to 575 ms) under heavy load from other work.
- **Risk accepted.** A JSON-RPC client can fill the disk with backups;
  each needs a new name. The backups directory is never pruned by
  dcr-purse.
- **ADR-0003 gains an addendum if this record is accepted.** Its Decision
  item 8 says the snapshot copy "also compacts it", and its Open point 6
  was decided on the premise "on backup, which compacts anyway"; the
  measurements above refute both (a fresh copy of 570 MB, 412 MB only
  after an explicit compaction). The addendum, dated, corrects item 8 to
  say that a backup is compacted by an explicit step (this record's
  Decision item 2), restates Open point 6's premise, and leaves its
  decision, compaction only through backup in 1.0, as it is. ADR-0003's
  text stays as written (`docs/adr/README.md`). Open point 9.
- **Revisit** if redb gains a page-level copy, if users need backups
  written elsewhere than `backups/` (a configuration option, Open point
  3), or if `SingleWriter` mode matures enough for an out-of-process
  backup (B5).

## Open points

Each for the owner, with a recommendation. All ten were decided as
recommended (project owner, 2026-11-19); each is marked.

1. *B1 with compaction, S1 plus S3, E1?* **Decided as recommended (project owner, 2026-11-19).** Recommended: yes, as above.
2. *`backupwallet` at all?* **Decided as recommended (project owner, 2026-11-19).** It is a JSON-RPC divergence for a method
   dcrwallet reserves and answers with an error. The alternative is
   backups only with the wallet stopped (B3 and the tool), which keeps
   JSON-RPC identical but takes the wallet down for every backup and
   pushes users toward copying a running store, which the experiments
   show to be dangerous. Recommended: implement it, hidden from `help`.
3. *Where `backupwallet` writes.* **Decided as recommended (project owner, 2026-11-19).** Recommended: only `<appdata>/<net>/
   backups/`, by file name. Alternatives: any absolute path, as bitcoind's
   `backupwallet` takes, which lets an RPC credential create files
   anywhere the wallet's user can write; or a `backupdir` option, a
   configuration option dcrwallet does not have (W6). Revisit after 1.0
   with users' requests.
4. *Encryption.* **Decided as recommended (project owner, 2026-11-19).** Recommended: none in 1.0 (E1), documented. E2 would
   protect history and addresses in a backup taken off the machine, but
   adds a container format, a passphrase to lose, and a decryption step
   before every restore, for data the live store keeps in the clear
   anyway. Revisit if users ask for it.
5. *Compaction mode for the backup.* **Decided as recommended (project owner, 2026-11-19).** On a fresh snapshot copy the two
   modes give the same file: the copy inserts in key order and fills its
   pages, so `Maximum` finds nothing more to pack than `Balanced`
   (identical sizes and per-table page counts, Experiments), and their
   times overlap across runs (`Balanced` 2.4 to 4.3 s, `Maximum` 2.3 s,
   three runs each). Recommended: `Balanced`, as Decision item 2 says,
   with the record stating that the choice makes no difference to a
   backup. The modes differ only on a live, fragmented store (524 against
   416 MB above; `src/db.rs:2280-2297`), where whether to leave room for
   inserts matters; that belongs to in-place compaction, which 1.0 does
   not do (ADR-0003 Open point 6).
6. *The store's file name.* **Decided as recommended (project owner, 2026-11-19).** ADR-0005 defers it to ADR-0003, which does not
   give one; `restore`, the documentation's copy instructions and the
   tool's messages need it. Recommended: fix it in Phase 2 with the
   schema, a name other than `wallet.db`, for ADR-0005's reason: a
   dcrwallet started by mistake would otherwise find and open a file of
   that name ("Options considered", option (d)).
7. *Windows.* **Decided as recommended (project owner, 2026-11-19).** Directory sync does not exist there; renaming with
   `MoveFileExW(MOVEFILE_WRITE_THROUGH)` needs `windows-sys` or similar
   (no `unsafe` in dcr-purse crates, ADR-0008 Facts; the call would come
   from a crate that already wraps it). Recommended: measure on the
   Windows CI runner in Phase 8 and record what is guaranteed there; until
   then the documentation says a backup is complete once the call has
   returned, which holds for the file's contents.
8. *A gRPC method later?* **Decided as recommended (project owner, 2026-11-19).** Recommended: not before Decrediton's call
   inventory shows a use for it (brief §6 Phase 0).
9. *The addendum to ADR-0003.* **Decided as recommended (project owner, 2026-11-19).** ADR-0003 is accepted and says the
   snapshot copy compacts the backup, which the experiments here refute.
   Recommended: add the dated addendum described under Consequences when
   this record is accepted, written by the same change; the alternative,
   leaving ADR-0003 as it is with this record as the only correction,
   leaves an accepted record stating a measured falsehood that Phase 2
   would read first.
10. *`backupwallet` and request cancellation.* **Decided as recommended (project owner, 2026-11-19).**
   Recommended: a started
   backup ignores its request's cancellation and runs to its end (item 4),
   so a POST caller is never left with a cancelled call whose backup may
   or may not have been published. The alternative, cancelling the
   backup with the request, is closer to how every other handler treats
   its context, but under a reproduced 10 s POST cancellation it would
   make backups of large wallets impossible through `dcrctl` and would
   need the documentation to send such users to the websocket.

## Decision by the project owner (2026-11-19)

Accepted as proposed, every open point as recommended. The answers:

1. *What a backup is:* B1 with compaction, the JSON-RPC surface S1 and the
   tool S3, no added encryption (E1).
2. *`backupwallet`:* implemented, hidden from `help`.
3. *Where it writes:* only `<appdata>/<net>/backups/`, by file name;
   revisited after 1.0 with users' requests.
4. *Encryption:* none in 1.0 (E1), documented; revisited if users ask
   for it.
5. *Compaction mode:* `Balanced`, the record stating that on a fresh
   snapshot copy the choice makes no difference.
6. *The store's file name:* fixed in Phase 2 with the schema, a name other
   than `wallet.db`. `purse-store` as committed takes the path from its
   caller, so the name is still to be chosen (a follow-up of Phase 2).
   *2026-11-20: proposed as `wallet.redb` (`purse_store::STORE_FILE_NAME`;
   addendum "The `purse-store` part, as built", Open point BK1).* *Decided
   2026-11-21 (project owner): the name is `wallet.redb` (addendum of
   2026-11-21, item 1).*
7. *Windows:* measured on the Windows CI runner in Phase 8 and what is
   guaranteed there recorded; until then the documentation says a backup
   is complete once the call has returned.
8. *A gRPC method:* not before Decrediton's call inventory shows a use.
9. *ADR-0003:* the addendum is written by this change (ADR-0003, addendum
   of 2026-11-19, "Decisions of the project owner").
10. *Cancellation:* a started backup ignores its request's cancellation
    and runs to its end (Decision item 4).

What the acceptance leaves to do is "Consequences" above: the snapshot
copy, backup record, digest and verify path in `purse-store` with their
tests (not written yet: the Phase 2 change that built the store left
backup out), `backupwallet` and its PARITY line with QK-0432's changed
disposition for that one name in Phase 7, the tool in Phase 8, the
documentation and the restore-every-release CI job in Phase 13.

## What was not tested

- Power loss during a backup (only process kills); the backup's
  durability rests on redb's `Durability::Immediate` and the rename and
  directory sync, each documented, not exercised by a cut.
- Which phase each kill of the compaction sweep hit was not recorded, and
  the temporary files it left were opened and checked for the backup
  record but their digests were not compared.
- The effect of the backup file's cache size on the wallet's commit
  stalls: suggestive in both sets of runs, not separated from load.
- Windows and macOS: the mandatory lock's effect on a file copy, rename
  durability, all timings.
- Stores of real wallets: the experiments used synthetic rows; ADR-0003's
  hot backups of its workload stores (8.5 to 9.2 s at its largest) are the
  closest real-shaped figures.
- Atomic file-system snapshots (B7).
- The reserved `.partial/` directory of Decision item 3: the kill sweeps
  wrote the temporary file beside the final name.
- `backupwallet` over HTTP POST past dcrwallet's 10 s read timeout: no
  server exists yet, and the cancellation itself is unverified on a live
  dcrwallet (spec 16 Open question O1).
- What a copy of a running dcrwallet's `wallet.db` contains.
- The VSP client's behaviour after a restore rolls a ticket's fee-payment
  state back (spec 11; QK-0276 records that a repeated `payfee` is
  answered as already received, which suggests no second payment, but no
  run confirms it).
- Decrediton: whether it offers or expects any backup of the wallet file.

## Addendum, 2026-11-20 — The `purse-store` part, as built

"Consequences" gives `purse-store` the snapshot copy, the backup record,
the digest and the verify path in Phase 2, and the owner's answer to Open
point 6 fixes the store's file name there. This addendum records what was
built (`crates/purse-store/src/backup.rs`, module `purse_store::backup`,
whose documentation is the reference), where it differs from the
decision text and why, what was not built or tested, and the questions it
leaves for the owner. The decision above is unchanged. The addendum's
own open points are labelled BK1 to BK6, apart from the backup designs B1
to B7 compared under "Options considered".

**Decision item 1, as built.** `Store::backup(dir, name, info)` takes one
read transaction of the live store as the snapshot and copies every table
it lists, in the byte order of the table names, rows as raw bytes in key
order, into one write transaction of a new redb file with a 64 MiB cache
(`BACKUP_CACHE_SIZE`), committed with `Durability::Immediate`. The backup
record is written into the copy's `meta` table under the key `backup` in
the same transaction (`BackupRecord`; layout, 54 to 564 bytes: format u8,
1 ‖ network length u8 ‖ network ‖ schema version u32 BE ‖ version length
u8 ‖ dcr-purse version ‖ time i64 BE ‖ row count u64 BE ‖ digest, 32
bytes). The digest is BLAKE-256 (`dcroxide-crypto`, a new dependency of
`purse-store`, already in the workspace graph) over every row of the
snapshot, each row as its table's name, key and value, each
length-prefixed by a u64 big-endian length. Points the decision text left
open, settled as follows: the store does not know its network, so the
caller states the network's name and the dcr-purse version (each 1 to 255
bytes of UTF-8, `BackupInfo`); the time is Unix nanoseconds, the unit of
ADR-0008's `Clock::now_wall`; a `backup` row in the store being copied
(a backup file opened directly as a store, without `restore`) is neither
copied nor digested, so a backup of it carries the same rows and a
record of its own. One backup at a time: a second, through either entry
point, is refused at once with `a backup is already running` (kind
`Invalid`), not queued. Writers were checked not to be blocked: the tests
commit from inside a running backup and from another thread under its
snapshot.

**Decision item 2, as built.** After the commit, `compact` with
`CompactionMode::Balanced` is called until it returns `false`, at most 16
times (redb's `compact` already repeats until it makes no progress, so the
second call reports nothing left; the bound only stops a misbehaving
engine from looping). A published backup reopened and compacted again
reports nothing to do (`tests/backup.rs`).

**Decision item 3, as built.** The backup checks the name (below), takes
the backup lock, creates `dir` if it is missing, refuses (kind `Exist`) a
name under which anything exists in `dir`, a broken link included, before
it writes anything, removes every entry of `dir/.partial/`, creates
`.partial/<name>` exclusively, copies, commits, compacts, closes the
file, checks the final name again, renames, and syncs `dir` and
`.partial/` (Unix; a no-op on Windows, Open point 7). When the backup
creates `dir`, and any missing parent of it, it syncs each new directory's
parent at once (Unix), so that a backup that has returned survives a power
cut together with the directory holding it; without that sync a first
backup's whole `backups/` entry could be lost on a file system that does
not order directory updates, though `backup` had returned. A `dir` given
as the empty relative path (the current directory) is synced as `.`: the
sync once opened `""`, which always fails, so such a backup published its
file and then returned an error, and a retry under the name was refused
(`tests/backup.rs`, run in a child process because the current directory
is the whole process's). Two additions
to the text, both on the side of never touching anything outside
`.partial/`: a `.partial` that is not a directory, a file or a link to a
directory included, is refused rather than cleaned
(`PARTIAL_NOT_DIR_TEXT`); and a link inside `.partial/` is removed, never
followed. An ordinary error before the rename removes the backup's own
file from `.partial/`; a stop asked by the caller's observer
(`Store::backup_observed`, the hook the daemon's shutdown will use, since
a started backup otherwise ignores its request, Decision item 4) leaves it
for the next backup, as a crash does. A stop is honoured at every step,
`Closed` (the last, after the file is complete and closed) included: the
backup is then not published and the complete file stays in `.partial/`
for the next backup to remove. After `Closed` only the second check of the
name, the rename and the directory syncs remain, and each can still fail.
The final name is checked again just before the rename, but the rename
itself (`std::fs::rename`) replaces a file created under that name between
the check and the rename, on Unix and on Windows alike; Open point BK3.
*Decided 2026-11-21 (project owner): the rename with the second check is
kept in 1.0, and the window is to be recorded in the user documentation
(addendum of that date, item 3).*

**File modes.** A backup holds everything the store holds, keys included,
so on Unix the backup file is created with mode 0600 and every directory
the backup creates (`backups/` and missing parents, `.partial/`, verify's
scratch directory) with 0700, the modes dcrwallet gives its database file
(`bolt.Open(dbPath, 0600, nil)`, `wallet/internal/bdb/db.go:321`) and its
directories (`os.MkdirAll(path, 0700)`, `walletsetup.go:375`), and which
spec 01 lists among the parity constants. The live store file is created
0600 the same way (`Store::create_with`; it was created with the
platform's default, 0644 under the usual umask, before this change). The
umask can only narrow these modes. A directory that exists keeps the mode
its owner gave it: the backup does not change permissions it did not
create. On Windows the files and directories get the default access
control of their parent, as Go's `os.OpenFile` gives dcrwallet's there.
`purse_store::FILE_MODE` and `DIR_MODE` name the two modes. Open point
BK6. *Accepted 2026-11-21 (project owner), with Phase 8's `restore` to
create the restored store 0600 the same way (addendum of that date,
item 6).*

**Decision item 4, the library's part.** `check_backup_name` refuses, each
with a text of dcr-purse's own (kind `Invalid`), an empty name, one with a
path separator (`/` or `\` on every platform), one containing `..`, and
one beginning with a dot. Beyond that list, so that the answer is the same
on every platform (ADR-0012), it refuses as `backup name is not a portable
file name` a name Windows would not keep as one file in the backups
directory: a control character or one of `: * ? " < > |` (`C:w` names a
drive there and `w:s` an alternate data stream), a trailing dot or space
(dropped by Windows, so `w.` would be `w`), a device name with or without
an extension, or more than 255 bytes. The device names, compared without
regard to ASCII case, are Microsoft's list (`CON`, `PRN`, `AUX`, `NUL`, and
`COM` and `LPT` followed by a digit 0 to 9 or a superscript `¹`, `²` or
`³`) and the console names `CONIN$` and `CONOUT$`, which `CreateFile` opens
as console handles; Go's `isReservedBaseName`
(`internal/filepathlite/path_windows.go`) reserves the same apart from
`COM0` and `LPT0`, and the console names only as a whole base name, so
refusing `CONOUT$.txt` is stricter than Go and costs nothing. A name it
accepts is exactly one plain path component. Open point BK2. *Accepted
2026-11-21 (project owner; addendum of that date, item 2).* The mapping
of these refusals to
JSON-RPC's invalid-parameter code, the absolute path in the reply, the
log line, and `help` staying silent are Phase 7's.

**Decision items 5 and 6, the library's part.** `verify_backup(file,
scratch, network)` copies the file into the scratch directory (a new file,
created exclusively, mode 0600 on Unix), opens the copy only through a
writable open with `set_check_integrity_on_open`, and removes the copy;
`file` is only ever read (the tests compare its bytes after every call).
Only a regular file is copied, and no more of it than its length when it
was opened: a directory, a FIFO, a socket or a device (`/dev/zero` would be
copied without end) is refused before anything is copied (`the file to
verify is not a regular file`, kind `Invalid`), on every platform. The
type is checked by path before the open (Windows cannot open a directory
as a file, and would otherwise report an I/O error) and again on the open
handle, which is authoritative; on Unix the open carries `O_NONBLOCK`
(the `libc` crate's constant, already in the graph through redb; *kept
as a direct, Unix-only dependency of `purse-store` by the project owner,
2026-11-21, addendum of that date, item 7*), because
a plain read-only open of a FIFO with no writer waits for one forever and
the path can change between the check and the open. The copy's name
begins `.verify-`, a prefix reserved to verification in the scratch
directory. On Unix the name
is removed as soon as the file is created, before a byte is copied: the
open handle keeps the file, and the kernel frees it when the handle
closes, however the process ends, so a killed verify leaves no copy. On
Windows the name stays until the check ends, and a verify killed in
between leaves its copy, a file exactly as sensitive as the backup it
copies; each verify therefore first removes every file named with the
reserved prefix in its scratch directory, so the next verify into the same
directory removes it. The file being verified is never one of those, even
when it lies in the scratch directory under the prefix or is reached
through a link there: the sweep recognises it by device and inode on Unix
and by canonical path elsewhere, and removes nothing when it cannot
recognise it. A caller (Phase 8's `verify` and `restore`) that
gives each run a different scratch directory must remove a killed run's
directory itself. It refuses, never panicking: a file that is not a redb
database or fails
the integrity check; a redb database that is not a dcr-purse store (no
`meta` table, an unknown `format` row, a table of other key or value
types, which redb reports as a misuse of the engine and the verify path
reports as a foreign file); no schema version, version 0, or a version
newer than the build (`Error::newer_version_info`); a backup record that
does not decode, whose schema version is not the store's, whose network
is not the one asked for (kind `Invalid`, `Error::wrong_network`), or
whose row count or digest is not that of the rows in the file (kind `Io`,
`Error::unverified`). For a store of this build's schema version it then
requires every table of the schema, decodes every key and value of every
table by its types (`ReadTx::check_rows`, new), and runs the store's
consistency checks, `check_chain` and `check_records`. A plain copy of a
stopped store has no record and passes on the integrity and consistency
checks alone, as item 5 says. `verify_backup_on_backend` runs the same
checks on a storage backend, for the fuzz target. Two limits of what
"the store's consistency check (ADR-0003 item 4)" can mean today: the
store has no check yet of the transaction tables' indexes and balances,
which come with the transaction store in Phase 3 (Open point BK4;
*decided 2026-11-21: `verify_backup` runs that check when Phase 3 builds
it, addendum of that date, item 4*; *built 2026-11-26, addendum of that
date*); and for
a store of an older schema version the consistency checks are skipped
and the result says so (`consistency_checked: false`), because `restore`
migrates before it checks, and no older version exists yet, so that path
is not exercised. `restore` itself, the tool and its messages are Phase
8's and were not built.

**Open point 6, the file name.** `purse_store::STORE_FILE_NAME` is
`wallet.redb`, a name other than `wallet.db` as the owner decided; the
name itself is proposed here and not decided (Open point BK1). *Decided
2026-11-21 (project owner): the name is `wallet.redb` (addendum of that
date, item 1).* The store's
functions take their path from the caller and never add the name; the
tests of `purse-udb` that create a wallet's store use the constant, and
ADR-0005 Decision item 2 carries a dated pointer to this addendum.

**Schema.** The `backup` row of `meta` exists only in backup files. It is
an addition to schema version 1 made before any release: no wallet has
been written at version 1 and no migration exists or is needed, and
version 1 may still change without one until a release ships (as the
addenda of ADR-0003 say of its own changes). `MetaKey` does not name the
row, so a typed walk of a backup's `meta` table stops at it with a
corrupt-row error; `check_rows` skips it, and `restore` removes it. The
checked-in version-1 fixture is unchanged.

**Tests** (`crates/purse-store/tests/backup.rs`, 21 tests, two of them
the child halves of the kill test and of the backup into the current
directory, which do nothing on their own; Linux on
x86_64 only in this change):

- A backup taken while another thread commits durably equals one of the
  states the writer committed (the writer digests the store after every
  commit) and not the last, the observer holding the snapshot open until
  the writer has committed twice under it: 4 cases by default
  (`PURSE_STORE_BACKUP_CASES`), each also verified and opened as a store.
- A power cut at every storage operation of a backup's copy, commit and
  compaction into a power-loss backend (the harness of the store's
  power-loss suite), clean, torn and with every write kept (a killed
  process's page cache): on the fixture with 40 further rows of each of
  three tables, 107 operations; with a clean or torn cut 81 images were
  refused and 27 verified, with every write kept 80 and 28. Every image
  that verified carried exactly the complete backup's record, and every
  one of the 27 cuts that fell after the commit had returned, during the
  compaction or the close, verified. The source store's own backend saw
  no storage operation during any backup, its digest was unchanged, and
  it took a durable commit and reopened afterwards. A transient storage
  error at every operation gives an error or a backup that verifies, the
  source unchanged, and the store takes further backups (the lock is
  released on every path).
- A stop at every step of a file backup (`Prepared`, each of the 42
  tables copied, `Committed`, `Compacted`, `Closed`) publishes nothing,
  leaves the source as it was, and leaves `.partial/<name>` refused by
  verify before the commit (kind `NotExist`, no tables) and verifying as
  the backup from the commit on; the next backup removes everything in
  `.partial/` (files and a nested directory) and publishes, and a user's
  file in the backups directory is untouched.
- Every name refusal, the device names above with superscript digits,
  `COM0`, `LPT0` and the console names included, with nothing written and
  the backups directory not created; an existing name refused before
  `.partial/` is cleaned; a
  `.partial` that is a file, and (Unix only) one that links elsewhere,
  refused with the files there untouched; a link inside `.partial/`
  removed without following it.
- `verify_backup` refuses a missing file (kind `NotExist`), empty, zeroed,
  text and random files, seven truncations, foreign redb files, a newer
  version (edited, and honestly written by a store of version 2), version
  0, no version, an unknown format row, an added row, a record with the
  wrong row count, schema version or network, three undecodable records,
  an undecodable row, an unknown meta row, a removed header row and a
  missing table, each with its own error. One bit flipped at 464
  positions spread over a backup file of the fixture with 30 further rows
  per table: 133 refused, 331 verified, each of those as the original
  backup (the flip fell where no check reads, such as unused page space).
  These ran with redb's debug assertions on, as the test profile builds
  it, and none fired.
- (Unix) The store file, a backup and verify's scratch directory have no
  permission bit outside the owner's, whatever the umask, and the owner can
  read and write them; a backups directory created two levels deep and
  `.partial/` likewise; an existing backups directory keeps its 0755.
- `verify_backup` refuses a directory (and on Unix `/dev/zero` and
  `/dev/null`) before copying anything, the scratch directory left empty.
  It removes files named with the reserved prefix that a killed verify
  left, and nothing else in its scratch directory (a user's file, a
  directory under the prefix). A child process verifying a backup of the
  fixture with 400 further rows per table in a loop is killed four times,
  150 to 360 ms after it starts: on Unix it never leaves a copy holding a
  byte (an empty file could be caught in the instant between its creation
  and the removal of its name), and the next verify leaves the scratch
  directory without a copy, on every platform. With the early removal of
  the name disabled, the same test failed at once on a 1,118,208-byte copy
  left behind.
- The record round-trips (2,000 random records), decodes only canonical
  encodings (20,000 mutated encodings, 5,712 of which decoded, each
  re-encoding to its input), has its layout pinned byte for byte and
  every decoder refusal pinned by its text.

The fuzz target `fuzz/fuzz_targets/backup_verify.rs` decodes arbitrary
backup records and verifies patched images of a backup of the fixture;
CI's smoke job runs it without debug assertions, as it runs `store_open`
and for the same reason (the job's comment). It was run for 121 seconds
(53,883 inputs) and, after the last changes to this module, again for 60
seconds (28,151 inputs), both without debug assertions on an x86_64 Linux
desktop, with no crash. It verifies through `verify_backup_on_backend`, so
it does not reach `verify_backup`'s file handling (the regular-file check,
the copy and its removal), which the tests above hold.

**Not built or not tested.**

- The JSON-RPC method (Phase 7), the tool's `verify` and `restore`
  commands and `restore`'s removal of the record, migration and rename
  (Phase 8), and the user documentation (Phase 13).
- A sweep of process kills with real processes, as the Experiments ran:
  the power-loss sweep's kept-writes cut is the in-process equivalent at
  every storage operation of the copy and compaction, and the stop at
  every step covers the boundaries between file-system steps, but the
  rename and the directory syncs themselves are not cut: they are file
  system calls outside the power-loss backend, so power loss across the
  rename is not exercised (as before this change). Nor is the sync of a
  new backups directory into its parent: it is called, and a backup into a
  directory created two levels deep is tested, but no power cut shows that
  the entry survives.
- Windows and macOS: CI runs the suite there; the link and mode tests are
  Unix only, the directory sync is a no-op on Windows (Open point 7, to be
  measured in Phase 8), and on Windows a killed verify's copy stays until
  the next verify into the same scratch directory (the kill test holds
  that next verify to removing it). The refusal of a FIFO, a socket and a
  device without blocking is tested on Unix only (Windows has no FIFO a
  test can make with std); the refusal of a directory is tested on every
  platform. On Windows the sweep's recognition of the file being verified
  rests on `std::fs::canonicalize`, tested by CI's Windows run of the same
  test but not examined for paths `canonicalize` cannot resolve (the sweep
  then removes nothing). Whether the backup and the store
  inherit an access control list on Windows that keeps other users out was
  not examined.
- No time or memory of the built backup or verify was measured; the
  Experiments' figures are the prototype's.
- Unbounded allocation in verify: the bytes the verify path parses itself
  are the record's, at most 564; rows are streamed, never collected; what
  redb allocates reading a hostile file is bounded by its integrity check
  and, in the fuzz targets, by their 64 MiB image limit, not by this code,
  and was not measured.
- The verify path for an older schema version (none exists).

**Open points**, each for the owner, with a recommendation; none is
decided: *All six decided as recommended (project owner, 2026-11-21); each
is marked, and the addendum of that date records them.*

- BK1. *The store's file name: `wallet.redb`.* **Decided as recommended
  (project owner, 2026-11-21).** Other than `wallet.db`, as
  decided; it says what the file is, and `purse-udb`'s tests already used
  it. Recommended: accept. Changing it is one constant before a release.
- BK2. *The name refusals beyond Decision item 4's list* (control
  characters, `: * ? " < > |`, a trailing dot or space, Windows device
  and console names as listed above, `COM0` and `LPT0` included, over 255
  bytes), refused on every platform so that a name means
  the same file everywhere and never reaches outside the directory on
  Windows. Recommended: accept, Phase 7 answering them with the same
  invalid-parameter code as the others. **Decided as recommended (project
  owner, 2026-11-21).**
- BK3. *The rename replaces a file created under the final name between the
  last check and the rename.* **Decided as recommended (project owner,
  2026-11-21).** Only something writing into `backups/`
  itself, during the microseconds between the two calls, can race it,
  but then the backup overwrites that file, which item 3 says never
  happens. The alternative publishes with a hard link from `.partial/`
  to the final name, which fails if the name exists, then removes the
  `.partial/` name; some file systems (FAT, some network shares) have no
  hard links, so it needs the rename as a fallback. Recommended: keep the
  rename with the second check in 1.0 and record the window in the user
  documentation; revisit if Phase 7's tests or users show the race.
- BK4. *The transaction tables' consistency check in verify.* **Decided
  as recommended (project owner, 2026-11-21).** The store
  checks its chain and record tables; the indexes and balances of the
  transaction tables (ADR-0003 Decision item 4) have no check in
  `purse-store` yet. Recommended: when the transaction store builds that
  check in Phase 3, `verify_backup` runs it too, and the tool's
  documentation says until then what verify does not check. *Built
  2026-11-26 (addendum of that date).*
- BK5. *The record's caller-stated fields*: the network as the data
  directory names it, the version string of ADR-0005 item 9, the time in
  Unix nanoseconds. Recommended: accept. **Decided as recommended
  (project owner, 2026-11-21).**
- BK6. *File modes on Unix*: **Decided as recommended (project owner,
  2026-11-21).** 0600 for the store, a backup and verify's
  copy, 0700 for every directory the store creates, existing directories
  left as they are; dcrwallet's modes, spec 01's parity constants. The
  alternative of copying the live store's mode onto a backup would carry a
  mode loosened by hand onto every backup. Recommended: accept, and have
  Phase 8's `restore` create the restored store 0600 the same way.

## Addendum, 2026-11-21 — Decisions of the project owner: BK1 to BK6, and `libc`

On 2026-11-21 the project owner accepted the six open points of the
addendum "The `purse-store` part, as built" as recommended, and decided
one point about `purse-store`'s dependencies. The text above stays as
written, apart from the dated pointers added to it.

1. **BK1, the file name.** The store's file name is `wallet.redb`
   (`purse_store::STORE_FILE_NAME`), as built. This is the name the
   owner's answer to Open point 6 left to be fixed in Phase 2 with the
   schema (item 6 of "Decision by the project owner (2026-11-19)" above,
   and ADR-0003, addendum of 2026-11-19 "Decisions of the project owner",
   item 5; both
   carry a dated pointer); ADR-0005 Decision item 2 carries one too.
2. **BK2, the portable-name refusals.** The refusals beyond Decision
   item 4's list stay, on every platform, and Phase 7 answers them with
   the same invalid-parameter code as the others.
3. **BK3, the rename.** The rename with the second check of the name is
   kept in 1.0, and the window between that check and the rename is to be
   recorded in the user documentation; it is revisited if Phase 7's tests
   or users show the race. The user documentation is Phase 13's and not
   written, so that record is still to do.
4. **BK4, the transaction tables in verify.** When the transaction store
   builds the check of the transaction tables' indexes and balances in
   Phase 3, `verify_backup` runs it too, and until then the tool's
   documentation says what verify does not check. Neither exists yet: the
   check is Phase 3's and the tool Phase 8's.
5. **BK5, the record's caller-stated fields.** The network as the data
   directory names it, the version string of ADR-0005 item 9 and the time
   in Unix nanoseconds stay as built.
6. **BK6, the Unix file modes.** 0600 for the store, a backup and
   verify's copy, 0700 for every directory the store creates, and
   existing directories left as they are, stay as built; Phase 8's
   `restore` is to create the restored store 0600 the same way.
7. **`libc` stays.** `purse-store` keeps `libc` as a direct dependency on
   Unix only, for `O_NONBLOCK` in `verify_backup` (its manifest gives the
   reason). The owner was told that it is the Rust bindings crate, that it
   compiles no C, and that it was already in the lock file through redb
   and getrandom, and decided to keep it.

**What follows.** Nothing in `purse-store`'s code changes. The
documentation of `purse_store::STORE_FILE_NAME`, which called the name
proposed and awaiting the owner, now states it as decided (a comment
only). Item 6 of "Decision by the project owner (2026-11-19)" above gains
a dated pointer. ADR-0003 gains an addendum of the same date, and the
dated pointer in item 5 of its addendum of 2026-11-19 "Decisions of the
project owner";
`docs/PHASE2-EXIT.md`, the brief (Phase 2 and D15), the README and the
ADR index say the same.

## Addendum, 2026-11-26 — BK4 built: the transaction tables' consistency check

Open point BK4, decided on 2026-11-21 (item 4 of that addendum), has the
check of the transaction tables' indexes and balances built with the
transaction store and run by `verify_backup`. The transaction store is
complete since its selection slice, so the check is built now. The text
above stays as written, apart from the dated pointers added to it.

**What was built.** In `purse-store`, `ReadTx::check_tx_tables` and
`WriteTx::check_tx_tables` (module `purse_store::txtables`, whose
documentation is the reference) check thirteen invariants of the
transaction tables (ADR-0003 Decision items 2, 4 and 5) by one walk of
each table with point lookups into the others: history rows against the
blocks' counts; `tx_mined` and `history` as each other's index;
unmined inputs and unmined credits against `tx_unmined`, an unmined
credit's class against its flags and its transaction's unpublished mark;
every unspent output against its live credit (amount, account, flags,
script location, class); every credit and debit, live or invalidated,
against its record; debits naming credits and carrying their amounts;
a spent credit's debit, where it exists, spending the credit's
outpoint; invalidated rows only in blocks marked invalid; unspent
commitment entries against their commitments; `balances` and
`balance_heights` against a recomputation from the unspent outputs,
unmined credits and commitments; `acct_history` against a recomputation
from the records' credits and debits. Each failure is a kind-`Io` error
naming the invariant (`Error::inconsistency`); `Error::inconsistent`
becomes public so that a check made outside the crate reports the same
way. `verify_backup` and `verify_backup_on_backend` run it after
`check_records`; `verify_backup_with` and `verify_backup_on_backend_with`
take a caller's check, run last on the same read transaction. The
balance classes' rules move into `purse-store` as `BalanceClass::mined`
and `BalanceClass::unmined`, the definitions the variants' documentation
already stated; `purse-udb`'s `txstore::mined_class` and `unmined_class`
call them, so there is one definition. In `purse-udb`,
`txstore::check_decoded` checks the seven invariants that need a
transaction decoded: every record, mined or unmined, holds a
transaction that decodes, whether or not a row names it; every credit,
live or invalidated, is the output of
its record's transaction it names (index in range, the output's value,
the script location and type code `AddCredit` takes from it), and every
unmined credit likewise against its unmined record; every debit, live or
invalidated, is an input of its record's transaction spending the
outpoint of the credit it names; an unmined input's spender has an input
spending that outpoint; the spender an unspent output's credit may name
by a live debit of it (QK-0785) is a stake transaction or a record at
another height; and no regular transaction of a block marked invalid
has a live credit or debit (the converse of invariant 9, which needs
the transaction's type: dcrwallet's stake invalidation moves them all
in the step that sets the mark, and `InsertMinedTx` and `AddCredit`
write only invalidated rows for such a transaction). It reads each
mined record's credits and debits by range and each unmined record's
credits by range, and decodes each record once. `txstore::verify_backup` and `txstore::verify_backup_on_backend`
are `purse-store`'s with it. The multisig index stays `check_records`'
invariant, and the auxiliary tables (VSP records, treasury and agenda
policies) are not part of the check: their invariants are those of
`purse_store::records`, and anything more they need is a later addition.

**Where the check lives (BK7, below).** `purse-udb` depends on
`purse-store`, not the reverse. Every invariant the property tests'
row checker recomputes is a relation between typed rows of
`purse-store`'s tables except those that read a transaction's inputs or
type, so those (invariants 1 to 5 and 10 to 13 of the module's list),
and four more found to hold (6 to 9: credits and debits have their
records, debits name credits, spent credits and their debits,
invalidated rows lie in invalid blocks), live in `purse-store`, and
`purse-store`'s `verify_backup` checks them on its own, without
depending on a caller to remember a hook; the decoded seven live in
`purse-udb`, which already decodes transactions and owns the script type
codes `AddCredit` writes, and reach the verify path through
`verify_backup_with`. The alternatives were the whole check
in `purse-udb` behind a trait, which leaves `purse-store`'s verify
checking nothing of these tables, and `dcroxide-wire` and
`dcroxide-stake` as dependencies of `purse-store`, a new dependency edge
and wallet rules in the store crate.

**States the check accepts.** dcr-purse reproduces dcrwallet's defects
under ADR-0012 Rule 1, so a check that refused the rows they leave would
refuse stores dcr-purse legitimately writes. From docs/QUIRKS.md and the
property tests' own exceptions, the check has no rule that: a hash is
not both mined and unmined, an outpoint not both an unspent output and
an unmined credit, and a live credit naming no spender has its unspent
row (QK-0781, with QK-0734's state as dcrwallet leaves it: two credits of
one outpoint at two heights share one unspent row, which a rollback of
the higher block deletes while the lower credit stays; and QK-0100: a
zero-value credit `Rollback` unspends gets no unspent row); an unspent
output's credit names no spender (QK-0785: allowed for a live debit of
that credit of the kinds stake invalidation leaves, and, by the same
mechanism over the state QK-0095 over QK-0781 leaves (below), for a
spender whose debit is gone or spends the outpoint's credit at another
height: dcrwallet's `stakeValidate` puts an invalidated credit back
unchanged, spender included, and its unspent row unconditionally, and
re-spends only what the validated block's own debits spend); a debit's credit
exists when the credit is recorded above it (QK-0101); an unmined
transaction's inputs have rows naming it, or an unspent commitment entry
belongs to an unredeemed commitment (QK-0060); the null outpoint has no
unmined-inputs row (QK-0087, QK-0780); an unpublished mark has an
unmined record (QK-0098, QK-0058); a ticket row has a record (QK-0056); a
record's credits are live (QK-0736); a multisig record agrees with the
transactions, debits or tree of its output (QK-0786, QK-0787, QK-0789,
QK-0097, QK-0054); a spent credit's spender has a debit spending that
credit (QK-0095 over QK-0781: `InsertMinedTx` debits a transaction's
inputs before its test that the record exists; once a second credit of
the spent outpoint is recorded at another height, the outpoint's unspent
row names it, and inserting the recorded spender again spends that
second credit and overwrites the debit, whose key has one slot per
transaction, height and input, to name it, while the first credit still
names the spender; a rollback of the spender's block then deletes the
debit and leaves the first credit naming a spender with no debit at
all; dcrwallet leaves the same rows, since its `InsertMinedTx` calls
`spendCredit` on the credit `existsUnspent` names and `putDebit` under
`keyDebit`; so invariant 8 refuses only a debit spending another
outpoint, and invariant 5 only an invalidated debit of the unspent
output's credit or a debit of another outpoint: once the first credit's
block is disapproved and approved again, stake validation gives it its
unspent row back with that spender, with no debit when the spender's
block was rolled back, or with a live debit of the outpoint's credit at
another height when the spender is recorded below its funder. The first
form was found by review after the first version of the check refused
it; both are written by the store's operations and run against Go, which
leaves the same rows). A mined credit without the expiry flag of its unmined credit
(QK-0079, QK-0066) agrees with every invariant: its class follows the
flags it has, and no invariant compares a credit's flags with its
transaction.
Invariants 6 and 9 rely on two states dcr-purse never
holds: a detached block's invalidated rows, which it deletes (ADR-0003
Decision item 5; QK-0081, QK-0782), and a credit without its record,
which it refuses (QK-0783). The stale spender bytes of QK-0053 and
QK-0055 are not representable in the store's rows. QK-0734's and
QK-0735's states are fixed (ADR-0012 Rule 7) and no operation leaves
them. By a reading of each entry (not a test), the other entries of
docs/QUIRKS.md that concern the transaction store are about what a query,
an iterator or the input source answers, an error or its text, a Go
panic, byte layouts the store's typed rows do not have (QK-0068,
QK-0074), values no rule reads (QK-0080's record bytes, QK-0064's
times), the unpublished mark a repeated insert leaves (QK-0058, which no
rule reads beyond the credit classes) or the removals of `PruneUnmined`,
whose rows are `RemoveUnconfirmed`'s (QK-0059, QK-0099, QK-0108); none
leaves a row the check could refuse.

**Not checked.** Neither part holds: a record's key against its
transaction's hash (no transaction is hashed; that its bytes decode is
checked); a credit's flags against
its transaction (QK-0079 makes the expiry flag differ legitimately);
that a spent credit's debit exists, since a deleted debit row leaves the
rows of QK-0095 over QK-0781 after a rollback, so it is refused only
where another rule sees it (the account history of the debit's record);
a commitment against its ticket's commitment output; and the rules the
states above rule out (`tickets`, `unpublished`, multisig records beyond
their index). `purse_store::txtables` lists these under "Not checked",
and `purse-store`'s backup module repeats them where it says what
verify does not check.

**The fixture.** The checked-in version-1 fixture held one row in every
table, its transaction rows chosen independently (a history position
past its block's count, debits without records, an invalidated credit
in a valid block) and its records' bytes not transactions, so the check
refused it and every backup of it, which the backup tests and the fuzz
target verify. Its transaction rows are rewritten consistent for both
parts of the check (block 1 disapproved by block 2, with an invalidated
credit, a debit spending it and its spent credit; block 2 with two
records, a spent and two unspent credits; an unpublished unmined
transaction with an input and a credit; a commitment; the sums and
account history they give), each record holding a serialized
transaction with the inputs its debits and unmined inputs name and the
outputs its credits name (values, P2PKH scripts at the locations the
credits record); the records' keys stay placeholders, not their
transactions' hashes, which no check reads. Every table still holds a
row, and the fixture was regenerated with the documented command, run
twice, byte-identical (282,624 bytes, SHA-256 `971b6220…603457`); its
pinned SHA-256 changed with it, as for the three earlier changes to
version 1 before any release (ADR-0003, pointer of this date). Its
schema, tables, keys and values are unchanged;
`every_released_schema_fixture_opens_and_upgrades_to_current` now also
requires `check_tx_tables` to pass, and `purse-udb`'s
`the_schema_fixture_passes_the_full_verify` verifies the fixture, and a
backup of it, with `txstore::verify_backup`.

**Tests.**

- `check_rows` in `crates/purse-udb/tests/txstore.rs` runs
  `check_tx_tables` and `check_decoded` after its own recomputation, so
  every state of every random sequence and targeted test is checked by
  both after every step; the reopen steps run them, with `check_chain`
  and `check_records`, on the read transaction. Both random-sequence
  tests passed at `PURSE_TEST_SEED` 0 to 399 (800 runs, 0 failures) and
  at 454, 509, 519, 523 and 563, the seeds beyond 399 that reach QK-0095
  over QK-0781 (454, 509, 523; 519 and 563 then roll the spender's block
  back); by the tests' own comments and QK-0785's entry seeds 0 to 399
  reach QK-0781's doubled output (76, 78 and 257) and QK-0785's state
  (214), and every seed planned revalidations through QK-0734's fix. A
  local variant of those tests biased toward rollbacks, credits and
  disapprovals (300 steps, four rounds; not in the suite), with which
  review found the revalidated form of QK-0095 over QK-0781 that the
  check's first version refused (seeds 30021, 30197 and 30261), passed
  at seeds 30000 to 30299 (600 runs) with the check as revised. The
  input selection check of those tests expected `Spender` to answer for
  every spent credit; for a credit whose spender has no record at the
  height it names (the rolled-back state above) it now expects Go's
  `IO` error, the answer `TxStore::spender` gives with Go's text.
- The operation-script runner checks the store as
  `txstore::verify_backup` does (every row decoded, the chain, record
  and transaction tables) after every operation and before every export,
  each `rt.checkpoint`'s and the final one, so every state a script
  passes through is checked, not only its last: the 40 hand-written
  corpus scripts (01 to 41; 38-upstream-tx-tests and the auxiliary
  records' 39 to 41 among them) and the 192 generated scripts of the
  frozen vectors (the auxiliary set's 8 among them) replayed without Go,
  and every script of the oracle tests run against Go (among them the
  tests and corpus records that reproduce QK-0054, QK-0060's re-created
  commitment entries, QK-0081, QK-0082, QK-0097, QK-0101, QK-0734,
  QK-0736, QK-0781 to QK-0789) passed.
  *Dated note, 2026-11-27 (the creation entry point):* the runner's
  setup now creates the wallet through `purse_udb::initialize` or
  `initialize_watch_only` (Go's `udb.Initialize` and
  `InitializeWatchOnly`), and the runner also checks the store after the
  setup's creation and opening, before the first operation, so a script
  without operations (37 of the creation set's 121: its 21 seed wallets
  created by `Initialize` alone, with no birth row, and its 16
  watching-only wallets) is checked there as well as before its final
  export. The 121 creation scripts of `tests/data/creation_vectors.txt`
  replayed without Go, and `same_seed_creation_matches_go` against Go at
  `PURSE_TEST_SEED` 0, `0x7a3e0b01` and `0x7a3e0b02` (121 scripts, 105
  operations each, every record and export equal, as first recorded in
  PARITY.md), passed. `creation_draws_on_gos_stream`, which drives the
  creation outside the runner, holds the store to the same check after
  the creation, after each operation and before its export, and the
  property tests of `tests/initialize.rs` hold every wallet a creation
  leaves whole to it; both passed. No state the creation path writes was
  refused, and no check was changed.
- `every_state_of_go_s_reproduced_defects_passes_the_check`: each state
  above written by the store's operations alone, on a store of its own,
  with the rows that make it asserted, so that the check is held to
  accept it whether or not a random sequence or script reaches it: 22
  states of 16 entries (QK-0054, QK-0056, QK-0060 three ways, QK-0079,
  QK-0087 with QK-0780, QK-0095 five ways, alone, over QK-0781 before
  and after the rollback, and, the funder's block then disapproved and
  approved again, an unspent output whose credit names a spender with no
  debit or, the spender below its funder, a spender whose live debit
  spends the outpoint's credit at another height, QK-0097, QK-0098, QK-0100, QK-0101, QK-0736,
  QK-0781 two ways, QK-0785 two ways, QK-0786, QK-0787). Each passes
  `check_tx_tables` and `check_decoded` in the write transaction and the
  property tests' row checker, the read side's checks once committed,
  and `txstore::verify_backup_on_backend` of an in-memory backup.
  QK-0789's state is reached, and checked after every step, by the
  multisig variant of
  `a_revalidated_transaction_merges_its_unmined_copy_keeping_the_row_invariants`.
- `each_broken_invariant_of_the_transaction_tables_is_refused`: on a
  store built by the operations holding a row of every kind the check
  reads, 55 direct row edits, each breaking one invariant or one rule
  within one (each field an unspent output shares with its credit, each
  direction of the history index, a debit's amount against a live and an
  invalidated credit, invariants 6 and 8 over the invalidated tables as
  well as the live ones, a record whose bytes do not decode, a valid
  block's mark flipped to invalid, a position skipped, a missing
  per-height sum, ...), 39 refused by `check_tx_tables` and 16 by
  `check_decoded`, each with that invariant's text and passing once
  undone in the same transaction (one edit, a regular transaction of the
  credit's own block as QK-0785's spender, also leaves a debit of an
  input its record lacks, which `check_decoded` checks after the
  spender); and four states of reproduced defects written the same way
  accepted (a dangling debit, QK-0101; a missing unmined-inputs row,
  QK-0060; an outpoint both unspent and an unmined credit, QK-0781; a
  stray unpublished mark, QK-0098). Each of 26 mutations of the check's
  first version that weakens one rule (dropping it, or a field or bound
  of it) makes this test or `purse-store`'s own fail; review then found
  five weakenings of invariants 6 to 8 that no edit separated, and each
  of 15 mutations of the version described here (those five, invariant
  5's accepted spenders widened, emptied or put back as first written,
  each of the decoded rules 5 to 7 dropped or, for 7, made to refuse
  stake transactions too, the paging cut to one page, the orphan count
  skipped) makes this test, `check_decoded_reads_past_the_first_page`,
  `every_state_of_go_s_reproduced_defects_passes_the_check`, the random
  sequences or an oracle test fail. Both runs were by hand on a copy of
  the tree and are not part of the suite.
- `verify_backup_runs_the_transaction_tables_check`: a backup file of that
  store and a plain copy of it stopped verify with `txstore::verify_backup`;
  each of the 55 edits committed and backed up into memory is refused by
  `txstore::verify_backup_on_backend` with its text, `purse-store`'s
  verify alone refusing exactly the 39 of its part; each backup taken
  once the edit is undone verifies; the last bad state, backed up to a
  file, is refused through the file path.
- `the_transaction_tables_check_refuses_each_broken_invariant`, in
  `purse-store`'s `tests/migration.rs`: on the fixture's rows, one edit
  per invariant (13) refused by `check_tx_tables` with its text and
  passing once rolled back, so the store crate's verify is tested in its
  own crate.
- `the_schema_fixture_passes_the_full_verify` (`purse-udb`): the
  checked-in fixture and a backup of it pass `txstore::verify_backup`.
- `check_decoded_reads_past_the_first_page` (`purse-udb`):
  `check_decoded` reads the tables it walks 512 rows at a time; each of
  them (`unmined_inputs`, `unspent`, `tx_mined`, `tx_unmined`, and
  `unmined_credits` when a credit lacks its record) gets 1,025 rows the
  check accepts and then one, sorting last, that it refuses, refused
  with its text and passing once deleted.
- Against Go (`crates/purse-udb/tests/oracle.rs`):
  `a_spender_inserted_again_over_a_twice_credited_funder_matches_go`
  runs QK-0095 over QK-0781 with the spender in block 1 and in block 2,
  the runner's checks after every operation, every record and the export
  equal. The rolled-back form cannot be compared by export: the store
  keeps a credit's spender by height, Go by height and block hash, and
  `a_spender_of_a_detached_block_mined_again_at_its_height_differs_from_go`
  shows the one difference that follows (a spender mined again at that
  height in another block: Go's `Spender` fails with `IO`, dcr-purse's
  answers; the export names a different spender block), recorded in
  PARITY.md, pending owner.
  `a_funder_s_block_revalidated_over_a_twice_credited_funder_matches_go`
  runs the two revalidated states above, the runner's checks after every
  operation: every record equal before the queries, and every query but
  those QK-0734's fix changes (PARITY.md item (23): the approval merges
  the unmined copy of the funder that the rollback returned, where Go
  keeps it and counts the output twice), each side's answer to those
  pinned; in the exports, beside Go's unmined copy, the spender's block
  and the debit's credit block are the detached block on Go's side and
  the block now at that height on dcr-purse's, as PARITY.md records.
- The backup tests of `purse-store` (`tests/backup.rs`, 21 tests) pass on
  the regenerated fixture with the check in the verify path, and the
  fuzz target `backup_verify` now verifies through `purse-udb`'s
  `txstore::verify_backup_on_backend`, so that it reaches the decoded
  part (built, not run).

**Cost.** `check_tx_tables`: one walk of each transaction table in key
order with point lookups, `tx_mined` walked once for two invariants:
O(n log n) in the rows. `check_decoded`: one walk of `unmined_inputs`,
`unspent`, `tx_mined` and `tx_unmined`, a page of 512 rows at a time,
four range lookups per mined record and one per unmined record, and one
decoding of each record (and of the unmined record each unmined input
names): O(n log n) in the rows plus a linear pass over the
transactions' bytes. No
table is collected: the balances are recomputed into one entry per
(account, class) and per (account, windowed class, height) holding an
output, the account history checked record by record with a final
count, and `check_decoded` holds one record's credits and debits and
one decoded transaction at a time. An unrecorded local timing, release
build, on a synthetic consistent store of the measurement workload's
size (40,000 blocks, 100,000 records, each a serialized one-input
one-output transaction, and credits, 49,500 debits, 50,500 unspent
outputs and per-height sums, 100,000 account history rows), warm, on
an x86_64 Linux desktop under heavy load from other work: 0.28 to 0.35 s
per `check_tx_tables` and 0.12 to 0.15 s per `check_decoded`, five
rounds each, two runs, for the check's first version; for the version
described here, which decodes every record and looks up the block of
each record with a live row, 0.26 to 0.47 s and 0.15 to 0.27 s (0.26 to
0.28 s and 0.15 s in the less loaded of the two runs). Not a run of
record. Suite times are given with
the commit, unrecorded.

**Not built or not tested.**

- The tool's `verify` and `restore` (Phase 8), which are to call
  `purse-udb`'s `txstore::verify_backup`.
- The fuzz targets were not run for this change; `backup_verify` reaches
  the consistency checks only through images whose rows keep the backup
  record's digest, which the verify path checks first.
- The check is not run when a wallet opens its own store, only by the
  verify path and the tests.
- Windows and macOS: CI's.
- The timing above is the only one; nothing was measured against a
  wallet's real store.
- The mutation runs above were by hand; they are not in the suite.

**Open point**, for the owner, with a recommendation; pending owner:

- BK7. *Where the check lives.* The thirteen invariants the store can
  read from its typed rows in `purse-store`, run by its `verify_backup`
  on its own; the seven that need a transaction decoded in `purse-udb`,
  reaching the verify path through
  `verify_backup_with`, so that the full check is `purse-udb`'s
  `txstore::verify_backup`, which Phase 8's tool is to call. Evidence:
  `purse-udb` depends on `purse-store`, and decoding a transaction in
  `purse-store` would add `dcroxide-wire` and `dcroxide-stake` to its
  dependencies. Recommended: accept as built; the alternative of a
  `purse-store` check that decodes transactions is the one to revisit if
  the tool should depend on `purse-store` alone.
