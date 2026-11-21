# Phase 2 exit record (store design)

The brief's Phase 2 exit (section 6) has five clauses. This record names,
for each, the tests or the measurement that hold it, what they compare and
how many cases, and what is left. It also lists, at the end, what the
Phase 2 section asks for beyond its exit sentence and where each stands.
The store is `crates/purse-store` (schema version 1, redb 5.0.0, ADR-0003).
Parity pin: dcrwallet `54b85b33`, bbolt v1.3.12 as dcrwallet links it.
dcroxide pin: `2074168`. The record was written at `a5cdb88` plus the
uncommitted changes that add the remaining records, the backup path and
the measurement.

*Closed by the project owner on 2026-11-21, on the terms under "Verdict"
below: every exit clause is held by tests that pass, and what this record
lists as left stays listed as left, carried to the phases named.*

All counts below are for the default sizes the normal test suite
(`PURSE_REQUIRE_ORACLE=1 cargo test --workspace`, as CI runs it on Linux,
macOS and Windows) uses, unless stated otherwise. They were read from one
run on an x86_64 Linux desktop, with `PURSE_TEST_SEED` unset, so each
test's inputs are fixed by its label. Setting `PURSE_TEST_SEED` varies them,
and each test prints the value that reproduces it.

## 1. Fault injection

> the store passes fault injection (a crash at any point leaves the last committed state)

**Held by** `crates/purse-store/tests/powerloss.rs`, over a storage backend
that counts every write, length change and sync and can cut the power after
any of them (`tests/support/powerloss.rs`, a port of dcroxide's). A
reference run of a scripted workload records the digest of every table
after every commit. The workload is wallet-shaped writes, a transaction
record of several pages, overwrites, a bulk delete that shrinks the file,
VSP tickets, hosts, multisig outputs and treasury policies through the
record operations, an initial sync of header and filter batches through the
chain operations, a header-only reorganisation, and a detached block
promoted back. Then a fresh run is cut at every storage operation in turn.
The image left is reopened through the store's own open, which repairs it,
and must:

- hold exactly the rows of some commit;
- lose no commit acknowledged as durable;
- pass both chain checks (`ReadTx::check_chain` and the tests' own
  `support::chain`) and both record checks (`ReadTx::check_records` and
  `support::records`);
- take another durable commit and reopen cleanly.

Cuts are clean (every unsynced write undone), torn (each 4 KiB piece of
each unsynced write kept or undone at random) or a killed process's (every
write kept, synced or not).

| Test | What | Default size |
|---|---|---|
| `every_clean_cut_lands_on_the_last_durable_commit` | durable commits, clean cut at every operation | 26 commits, 482 storage operations, 37 syncs: 483 trials, 471 at the last durable commit, 12 before the store was created |
| `every_torn_cut_lands_on_a_committed_state` | the same, torn | 483 trials, 471 at the last durable commit, 12 not created |
| `every_kill_lands_on_a_committed_state` | the same, every write kept | 483 trials: 446 at the last durable commit, 26 at the commit in flight, 11 not created |
| `relaxed_sync_loses_only_batches_since_the_last_durable_commit` | initial-sync batches committed without durability (ADR-0003 Decision item 9), clean cut | 455 trials; at most 2 non-durable batches lost, below the interval |
| `relaxed_sync_torn_or_killed_stays_within_the_bound` | the same, torn and killed | 455 trials each, at most 2 batches lost |
| `a_dishonest_sync_is_caught` | a backend whose sync makes nothing durable must fail the sweep | 471 failures in 483 trials |
| `a_reorganisation_cut_anywhere_leaves_the_old_chain_or_the_new` | one durable reorganisation, also writing and deleting wallet rows, cut at every operation from open to close, each kind of cut | depth 1: 45 operations, 138 trials (95 old chain, 43 new); depth 40: 66 operations, 201 trials (158 old, 43 new) |
| `a_relaxed_reorganisation_cut_anywhere_leaves_the_old_chain_or_the_new` | the same as a relaxed header batch made durable by `finish` | depth 1: 35 operations, 108 trials; depth 4: 40 operations, 123 trials |
| `a_failed_commit_reports_an_unknown_outcome`, `backends_open_and_refuse_as_files_do`, `two_initial_syncs_share_the_durable_bound`, `concurrent_syncs_keep_the_durable_bound` | an engine failure inside a commit is reported as an unknown outcome; the durable bound across several syncs and threads | 6 operations; 643 durable commits, longest non-durable run 49 |

**Also holding the clause:** `tests/backup.rs`
`a_power_cut_at_every_storage_operation_of_a_backup_publishes_nothing_partial`.
It cuts the power at each of the 107 storage operations of a backup's copy
and compaction, clean, torn and kept. 81 (80 kept) images are refused by
`verify_backup`, and 27 (28) verify. Every image that verifies carries the
reference backup record, and the source store sees no operation.
`a_killed_verify_leaves_no_copy_behind` kills a real process four times
while it verifies a backup: on Unix no copy holding a byte is left, and
the next verify leaves no copy on any platform.
`verify_refuses_what_is_not_a_regular_file` refuses a directory on every
platform, and on Unix a device, a socket and a FIFO with no writer, which
a plain read-only open would wait on forever, within a time limit.
`verify_never_removes_its_input_from_scratch` holds that the file handed
to verify is never removed, even when it lies in the scratch directory
under the prefix reserved to verify's copies.
`a_backup_into_the_empty_directory_path_succeeds` backs up into the
directory `""` (the current one) in a child process and requires the
backup to be published and returned without an error.
`tests/migration.rs` cuts migrations (clause 4), and `tests/model.rs` cuts
the power inside its random sequences (clause 2).

**Not covered:** a real process killed at random moments. The spike killed
its prototype's writer 300 times (ADR-0003, "Correctness"); `purse-store`
has no such sweep. The kept cut models a killed process at every storage
operation, but not a kill between them inside redb's own code. The rename
and directory syncs that publish a backup, and the sync of a newly created
backups directory into its parent, are never cut. The sweeps ran on
redb 5.0.0 only (the release pinned), on an x86_64 Linux desktop's file
system semantics as the backend models them. Windows and macOS run the same
in-memory sweeps in CI, but no real file system's behaviour under power
loss is tested.

## 2. Model-based property test

> and a model-based property test

**Held by** `crates/purse-store/tests/model.rs`. `the_store_behaves_as_its_model`
runs random sequences against the store and against an in-memory model (a
map per table, with the chain and record operations written out on it as
their documentation states them). The operations are:

- write transactions, committed or abandoned: puts, deletes, deletes
  through a write cursor, refused puts, and reads of the transaction's own
  writes;
- reads: gets, bounded ranges both ways, prefixes, counts, cursor walks;
- snapshots held across a commit;
- initial syncs: finished, dropped or cut;
- chain operations: header batches and durable updates, with refusals;
- record operations;
- clean reopens and power losses.

After every operation the store's whole contents must equal the model's,
and both chain checks and both record checks must pass. After a power loss
the contents must equal the model as of some commit since the last durable
one. One case in eight runs on a file. A failing sequence is shrunk
(`a_failing_sequence_shrinks` holds the shrinker).

Default size: 48 sequences (`PURSE_STORE_MODEL_CASES`) running 733 updates,
176 abandoned updates, 185 chain updates (29 abandoned), 164 record updates
(23 abandoned), 313 reads, 104 snapshots, 307 initial syncs, 196 reopens
and 189 power losses. Chain operations: 1,390 blocks connected, 509
detached, 183 reorganisations, 36 promoted, 3,147 refusals. Record
operations committed: 90 hosts, 121 tickets written and 50 deleted, 131
multisig outputs written and 52 deleted, 99 policy writes or clears, 81
agenda preferences, 17 empty hosts and 33 tickets for an unknown host
refused.

**Not covered:** the large nightly run ADR-0003's owner decision item 9
asks for. CI has no nightly job yet, so the property test has only ever run
at its default size and at the sizes of hand runs. *Still so at the close
(2026-11-21); carried to the nightly job when it exists.*

## 3. Cursor conformance against bbolt

> a cursor-conformance test runs the spec 04 §5.1 cases against `purse-store` and against bbolt through the oracle

**Held by** `crates/purse-store/tests/conformance.rs`, against bbolt
v1.3.12 itself through dcrwallet's `walletdb` interface and bdb driver
(the oracle `tools/oracle/cmd/boltcursor`), and against the frozen vectors
that oracle wrote. The oracle fixes bbolt's page size, so every host runs
the same layout.

| Test | What | Default size |
|---|---|---|
| `cursors_match_bbolt` | the named cases (spec 04 §5.1 items 1 to 6 and spec 01 §5.7's edge cases; items 7 and 8 have no counterpart, since the store's cursors have no `Close` and return owned copies), a random corpus, the pinned divergences and bbolt's panics, at 4 KiB pages | 617 scripts, 40,662 operations compared, 3,568 skipped by the documented rule; 69 random scripts span several leaves |
| `multileaf_defects_at_16_kib_pages`, `multileaf_defects_at_64_kib_pages` | the same at 16 and 64 KiB pages | 622 scripts, 67,344 compared; 631 scripts, 42,419 compared (one bbolt panic, QK-0748, in an operation not compared) |
| `moves_off_the_ends_match_bbolt` | steps off either end, and the moves after them | 400 scripts, 7,495 compared, plus 100 edge scripts at each larger page size |
| `cursors_match_bbolt_on_every_record_table` | the named cases and a corpus with the cursor on each of the twelve record tables | 132 scripts and 4,244 operations compared per table |
| `unspent_multisig_listing_is_bbolts_mu_walk` | one script's unspent multisig outputs against bbolt's walk of dcrwallet's `mu` keys | 40 key sets of up to 400 outpoints |
| `frozen_vectors` | the oracle's recorded answers replayed without Go | 188 scripts, 5,503 operations compared |
| `key_limits_match_bbolt`, `bbolt_last_on_an_emptied_multileaf_bucket_never_returns`, `a_go_panic_on_an_uncompared_op_is_accepted_wherever_it_falls`, `the_rule_skips_moves_after_next_ran_off_over_deletions`, `the_rule_compares_moves_after_prev_ran_off_the_start`, `writes_between_moves_follow_the_rule`, `the_write_cursor_follows_its_documented_rule`, `typed_cursor_follows_the_raw_cursor`, `put_limits`, `inverted_ranges_are_empty` | key limits, the QK-0750 hang, and the rule for what bbolt's layout decides | named scripts |

`crates/purse-store/tests/cursor.rs` holds the same cursors against a
model of bbolt's cursor written from its source (`cursors_match_the_bbolt_model`),
and spells out the spec 04 §5.1 cases input selection relies on
(`spec_04_cases`). Where bbolt's answer depends on its page layout, or
bbolt misbehaves under mutation (QK-0747 to QK-0750), the store does not
follow it. The project owner accepted this (ADR-0003, addendum "Decisions
of the project owner", item 8), and the divergences are pinned by named
scripts and recorded in `docs/PARITY.md`.

**Under other seeds.** The random corpora follow `PURSE_TEST_SEED`. Over
seeds 1 to 150 (a sweep of `cursors_match_bbolt`, both
`multileaf_defects_at_*` tests and `cursors_match_bbolt_on_every_record_table`
on an x86_64 Linux desktop), no operation the rule compares differed from
bbolt, but two defects of the test itself, both present at `a5cdb88`, made
some seeds fail, and both are fixed:

- The guard that the rule does not skip too much required ten compared
  operations for each skipped one; the ratio follows the seed, and 9, 9
  and 7 of the 150 seeds fell below ten at 4, 16 and 64 KiB pages (lowest
  8.32, 8.85 and 8.75; 11.4 at the default seed). The guard is now six
  (`MIN_COMPARED_PER_SKIPPED`), four for the smaller per-record-table
  corpus (lowest 5.31; it was five), and the floor of random scripts over
  several leaves is 35 (lowest 46; it was 50).
- A script in which bbolt panics on its last operation, outside the
  compared ones, has a result for every operation but no final contents
  (the oracle rolls the transaction back); the comparison demanded them.
  That failed 4 seeds of the 16 KiB corpus, 2 of the 64 KiB one and 1 of
  the record tables' corpus. `a_go_panic_on_an_uncompared_op_is_accepted_wherever_it_falls`
  now holds both places a panic can fall.

After the fix the four tests passed at every seed from 1 to 190 (760
runs; seeds 151 to 190 were run only after it). The guards' new values
come from the same seeds they were then tested on; a seed beyond them
could still fall below one, though the margins above make that unlikely
rather than impossible.

*Accepted (project owner, 2026-11-21, with the other recommendations;
ADR-0003, addendum of that date, item 5):* the loosened guards, that is
the three values above (`MIN_COMPARED_PER_SKIPPED` six, four for the
per-record-table corpus, and the floor of 35 multi-leaf scripts). The
second fix, accepting a panic on an uncompared last operation, is a
correction of the test that `ef15691` made and this clause describes; the
owner's decision does not name it. The fix's report
recommended a sweep of these tests over seeds in the nightly job ADR-0003's
owner decision item 9 asks for, once that job exists. Neither exists; the
sweep is still to do.

**Not covered:** bbolt's own page sizes other than 4, 16 and 64 KiB. Input
selection's random draws are checked against dcrwallet as the set of
outputs it may choose, not draw by draw (ADR-0003 Decision item 3); the
selection itself is Phase 3's.

## 4. Migration under crashes

> a migration test applies a dummy version step with a crash injected at every write and ends in the old or the new version, never between

**Held by** `crates/purse-store/tests/migration.rs`. A version-1 store is
opened by a build whose schema is version 2, through a dummy step that
rewrites, adds and deletes rows in several tables, record rows and both
record indexes among them. Power is lost at every storage operation of
that open, the repair included. The image is read with redb directly,
never through the store's open, which would migrate it. It must hold
exactly the version-1 rows at version 1 or exactly the version-2 rows at
version 2. Then the version-2 build must bring it to version 2, and a
version-1 build must refuse a version-2 image.

| Test | What | Default size |
|---|---|---|
| `a_migration_cut_at_every_write_leaves_the_old_or_the_new_version` | clean cut | 66 storage operations, 67 trials: 53 at version 1, 14 at version 2 |
| `a_torn_migration_leaves_the_old_or_the_new_version` | torn cut | 67 trials: 54 and 13 |
| `a_killed_migration_leaves_the_old_or_the_new_version` | every write kept | 67 trials: 54 and 13 |
| `a_two_step_migration_cut_anywhere_stands_at_a_version` | versions 1 to 3 in two steps | 74 operations, 75 trials: 54, 8 and 13 at versions 1, 2 and 3 |
| `a_store_newer_than_the_build_is_refused_with_its_rows_unchanged`, `a_refused_open_cut_anywhere_leaves_the_newer_store` | a newer store is refused with dcrwallet's text and left unchanged, even under a cut | — |
| `a_failed_migration_commit_reports_an_unknown_outcome` | an engine failure inside a step's commit | 66 operations: 50 unknown outcomes, 4 known |
| `every_released_schema_fixture_opens_and_upgrades_to_current` | the checked-in version-1 fixture (`tests/data/schema-v1.redb`, a row in every table) matches its pinned SHA-256, opens at the current version, every row decodes, the rows equal the expected ones, and it opens again | 1 fixture |

**Not covered:** a real migration. None exists: no schema version has been
released, so version 1 has changed in place three times (ADR-0003's
addenda), each time with the fixture regenerated. A redb file-format
migration (ADR-0003 Decision item 7) is not built.

## 5. The measured gains against dcrwallet

> the schema ADR records the measured gains against dcrwallet on the same workload

**Held by** ADR-0003's addendum "`purse-store` against dcrwallet on the
same workload" and the bench ledger's entry of 2026-11-20 of the same name.
They were measured by `crates/purse-udb/tests/measure.rs`
(`hot_paths_against_dcrwallet`, ignored), against dcrwallet's `wallet/udb`
over bbolt through `tools/oracle/cmd/storebench` (`setup`, `ingest` and the
new `hot` command). Both sides ingest the same generated history with the
wallet's call sequence: 40,000 blocks, 100,075 transactions, 72,242
addresses, 58,382 unspent outputs. Their canonical exports must be equal
line for line, and every timed query's answer equal, before anything is
timed. The timed work is the same on both sides (a range decodes and
keeps every transaction, its check text built after the timing on both;
a first run of the same day formatted Go's ranges inside the timing and
overstated their gain, and is superseded). Three sessions were run, three
rounds each, with the sides alternating, the test pinned to four cores,
on a shared, lightly loaded desktop.

Results, as dcrwallet's time over purse-store's (medians of 9 rounds per
side; no range overlaps, and purse-store was the faster on every figure in
every session):

| path | gain |
|---|---|
| balance of one account (minconf 1 and 6) | 88,800x to 102,500x |
| balances of every account | 36,800x |
| ranges over blocks (1,000 blocks either way, the 100 most recent transactions, the whole history) | 1.86x to 2.05x |
| address lookups by script (`Manager.Address`, `AddrAccount`) | 1.8x and 1.8x |
| account lookups (`LookupAccount`, `AccountProperties`) | 1.9x and 4.0x |
| commits (a block at the tip, one address recorded) | 1.6x and 1.9x |
| history sync per block | 1.7x |

The store was slower on no figure measured. `same_data_and_answers_on_both_sides`
(not ignored) runs the same pipeline at 240 blocks in every test run, so
the export comparison and the query answers are held in CI.

**Not covered:**

- The transaction store's own code. `purse-udb`'s transaction store is
  Phase 3's, so the test writes the transaction-store rows and composes the
  balance and range queries itself, following the schema (the address
  manager is `purse-udb`'s own).
- Stake, coinbase, unmined, stake-invalidation and reorganisation paths,
  and input selection.
- A dedicated machine. ADR-0014 §4 decides exit criteria there.
- Cold reads, the spike's 1m size, memory, store open, backup and crash
  recovery times.
- ADR-0014's row for Phases 2 and 3: "≥N times faster on balance, range
  queries (N from ADR-0003)", and no record has set N. "No slower than Go"
  holds on every figure measured. *N set on 2026-11-21: 1,000 for a
  balance query, 1.5 for a range query (ADR-0014, addendum of that date).*

These are the addendum's Open points M1 to M3. *Decided as recommended
(project owner, 2026-11-21; ADR-0003, addendum of that date): N as above;
the dedicated-machine run before Phase 3's exit, together with the Phase 1
run still owed, started only when the owner says when and where (what
counts as the dedicated machine: ADR-0014, addendum of 2026-11-21); the
store measured again with Phase 3's transaction store as part of Phase 3's
exit. None of these runs has been made.*

## What else the Phase 2 section lists

| Item | Status |
|---|---|
| The schema for the wallet's hot paths, with order that reaches a client kept | built (ADR-0003 Decision items 2 to 4 and its addenda) |
| Versioning, migrations, refusal of a newer store, a fixture per released version | built (clause 4); no version released yet |
| Main-chain headers and filters by height, detached ones by hash; tables not merged | built (ADR-0003, addendum "Chain tables keyed by height, as built") |
| The records ADR-0003 Decision item 2 has Phase 2 add (multisig, voting accounts, VSP, agenda and treasury, spec 05's aux buckets; imported keys and scripts) | built in schema version 1 (ADR-0003, addendum "The remaining records, as built"). Open points R1 to R4 await the owner. *R1 to R4 decided as recommended (project owner, 2026-11-21; R4: QK-0054 decided in Phase 3 with an oracle run).* The multisig records are checked against bbolt's order only, not against dcrwallet's udb (Phase 3) |
| The snapshot copy, backup record and verify path of ADR-0018 | built as library functions (ADR-0018, addendum "The `purse-store` part, as built"). Open points BK1 to BK6 await the owner. *BK1 to BK6 decided as recommended (project owner, 2026-11-21; ADR-0018, addendum of that date).* The JSON-RPC method and the tool are Phases 7 and 8 |
| The store's file name, other than `wallet.db`, "fixed here" | **proposed, not fixed:** `wallet.redb` (`purse_store::STORE_FILE_NAME`), ADR-0018, addendum "The `purse-store` part, as built", Open point BK1, awaiting the owner. *Fixed 2026-11-21 (project owner): `wallet.redb` (ADR-0018, addendum of that date, item 1).* |
| `purse-boltread` | a test utility since 2026-11-18 (ADR-0013); not a Phase 2 gate |

## Verdict

Each of the five clauses of the exit is held, by the tests and the
measurement named above, at the counts given. That the phase is closed is
the project owner's call, as it was for Phase 1. Before that call, these
are open:

1. The store's file name: proposed as `wallet.redb`, not yet fixed by the
   owner (ADR-0018, addendum "The `purse-store` part, as built", BK1).
   *Fixed 2026-11-21: below.*
2. The open points the owner has not decided: R1 to R4 (the records), BK1
   to BK6 (the backup), M1 to M3 (N for ADR-0014's row, a dedicated-machine
   run, measuring again with Phase 3's transaction store). *Decided
   2026-11-21: below.*
3. The test runs not done: a dedicated-machine run, a fuzz soak of the
   store's targets, and the nightly model run. None is part of the exit
   sentence. ADR-0003's owner decision item 9 ties the fuzz soak to the
   store's PARITY row moving to pinned. *Still not done at the close;
   carried forward: below.*

**Closed (project owner, 2026-11-21).** The owner accepted every open
point of items 1 and 2 as recommended and closed Phase 2 on these terms
(ADR-0003, addendum of 2026-11-21, item 4):

1. The store's file name is `wallet.redb` (ADR-0018 BK1).
2. R1 to R4, BK1 to BK6 and M1 to M3 are decided as recommended (ADR-0003
   and ADR-0018, addenda of 2026-11-21; N for ADR-0014's row in ADR-0014's
   addendum of the same date). `libc` stays a direct, Unix-only
   dependency of `purse-store` (ADR-0018, addendum of 2026-11-21, item 7).
3. What this record lists as left stays listed as left, carried to the
   phase named:
   - the multisig records' differential against dcrwallet's udb, and the
     oracle run of `redeemmultisigouts` that decides QK-0054 (R4): Phase 3;
   - the transaction tables' consistency check, and `verify_backup`
     running it (BK4): Phase 3;
   - input selection itself, whose draws clause 3 checks only as the set
     of outputs dcrwallet may choose: Phase 3;
   - the dedicated-machine run (M2): before Phase 3's exit, together with
     Phase 1's, when and where the owner says (ADR-0014, addendum of
     2026-11-21);
   - the measurement with the transaction store's own code (M3): Phase
     3's exit;
   - the large model run and the cursor conformance test's seed sweep:
     the nightly job of ADR-0003's owner decision item 9, which does not
     exist yet;
   - a fuzz soak of the store's targets: before the store's PARITY row
     moves to pinned (the same item 9);
   - the backup's JSON-RPC method and the mapping of the name refusals
     (BK2): Phase 7; the tool's `verify` and `restore`, `restore` creating
     the store 0600 (BK6): Phase 8; the user documentation, with the
     rename's window (BK3): Phase 13.

   What the clauses above list as not covered without naming a phase (a
   sweep of real process kills, cuts of a backup's rename and directory
   syncs, real file systems under power loss, other bbolt page sizes, a
   real migration, which needs a released schema version, and a redb
   file-format migration, which is not built; in the measurement, input
   selection, coinbase and stake-invalidation paths, cold reads, the 1m
   size, memory, store open, backup and recovery times) stays listed as
   not covered.
