# ADR-0018 — D15: Backup and recovery of the store

- **Status:** Proposed (decision D15). Nothing in it is decided. The
  repository facts and measurements under Context are not outcomes of it;
  ADR-0003's Decision item 8 and its Open point 6, both accepted, are the
  starting point.
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

## Decision (proposed)

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

Each for the owner, with a recommendation.

1. *B1 with compaction, S1 plus S3, E1?* Recommended: yes, as above.
2. *`backupwallet` at all?* It is a JSON-RPC divergence for a method
   dcrwallet reserves and answers with an error. The alternative is
   backups only with the wallet stopped (B3 and the tool), which keeps
   JSON-RPC identical but takes the wallet down for every backup and
   pushes users toward copying a running store, which the experiments
   show to be dangerous. Recommended: implement it, hidden from `help`.
3. *Where `backupwallet` writes.* Recommended: only `<appdata>/<net>/
   backups/`, by file name. Alternatives: any absolute path, as bitcoind's
   `backupwallet` takes, which lets an RPC credential create files
   anywhere the wallet's user can write; or a `backupdir` option, a
   configuration option dcrwallet does not have (W6). Revisit after 1.0
   with users' requests.
4. *Encryption.* Recommended: none in 1.0 (E1), documented. E2 would
   protect history and addresses in a backup taken off the machine, but
   adds a container format, a passphrase to lose, and a decryption step
   before every restore, for data the live store keeps in the clear
   anyway. Revisit if users ask for it.
5. *Compaction mode for the backup.* On a fresh snapshot copy the two
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
6. *The store's file name.* ADR-0005 defers it to ADR-0003, which does not
   give one; `restore`, the documentation's copy instructions and the
   tool's messages need it. Recommended: fix it in Phase 2 with the
   schema, a name other than `wallet.db`, for ADR-0005's reason: a
   dcrwallet started by mistake would otherwise find and open a file of
   that name ("Options considered", option (d)).
7. *Windows.* Directory sync does not exist there; renaming with
   `MoveFileExW(MOVEFILE_WRITE_THROUGH)` needs `windows-sys` or similar
   (no `unsafe` in dcr-purse crates, ADR-0008 Facts; the call would come
   from a crate that already wraps it). Recommended: measure on the
   Windows CI runner in Phase 8 and record what is guaranteed there; until
   then the documentation says a backup is complete once the call has
   returned, which holds for the file's contents.
8. *A gRPC method later?* Recommended: not before Decrediton's call
   inventory shows a use for it (brief §6 Phase 0).
9. *The addendum to ADR-0003.* ADR-0003 is accepted and says the
   snapshot copy compacts the backup, which the experiments here refute.
   Recommended: add the dated addendum described under Consequences when
   this record is accepted, written by the same change; the alternative,
   leaving ADR-0003 as it is with this record as the only correction,
   leaves an accepted record stating a measured falsehood that Phase 2
   would read first.
10. *`backupwallet` and request cancellation.* Recommended: a started
   backup ignores its request's cancellation and runs to its end (item 4),
   so a POST caller is never left with a cancelled call whose backup may
   or may not have been published. The alternative, cancelling the
   backup with the request, is closer to how every other handler treats
   its context, but under a reproduced 10 s POST cancellation it would
   make backups of large wallets impossible through `dcrctl` and would
   need the documentation to send such users to the websocket.

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
