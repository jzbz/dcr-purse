# Phase 0 exit record (scaffolding, oracle rig, decisions)

The brief's Phase 0 exit (section 6) has three clauses: CI green on three
platforms; the Rust reader's dump of Go-written databases equal to the Go
dumper's; D1 to D10 decided, D4 having since been superseded by ADR-0013.
This record names, for each, what holds it, with files, tests and counts,
and what is left. It also lists, at the end, what the Phase 0 section asks
for beyond its exit sentence and where each stands. Parity pin: dcrwallet
`54b85b33`, bbolt v1.3.12 as dcrwallet links it. dcroxide pin: `48f5b5e`.
*(The pin moved to `1587ca1` on 2026-11-23, after this check:
`docs/SKEW.md`, 2026-11-23 section.)*
The check was made at `69a6f6d`, on the project owner's instruction of
2026-11-21 to check D1 to D10 and close Phase 0. It found the first two
clauses held and the third short by eight points that the owner had not
settled and the records did not defer, and put them to the owner with a
recommendation each. On 2026-11-22 the owner answered "Accept all eight".
The record was brought up to date the same day, with those decisions
recorded in the ADRs they belong to, and read again after each of the
owner's later answers of 2026-11-22: on the questions of
[ADR-0019](adr/0019-filters-below-the-birthday.md), FB1 to FB10, "go
with your suggestions for fb1-10" (recorded in `ff59d84`), and FB11 and
FB12, found while recording those answers, "Accept both"; and on the
three things put last, ADR-0019's FB13, ADR-0013's remaining proposals
and a cut-off rule for questions arising in working out ADR-0019,
"Accept all three". Every commit since `69a6f6d` changes documents only.

**Phase 0 is closed (2026-11-22).** The first two clauses hold. The
third holds by section 3's test: each of D1 to D10 is decided,
superseded, or decided with points its own record defers to a named
phase or event, or that the cut-off rule defers, and no point is left
that is neither decided nor deferred. The eight points of the check, and
points 9, 10 and 11 found or put since, are decided, each as
recommended ("Verdict"). The cut-off rule the owner accepted on
2026-11-22, in the owner's accepted words, is: "Questions that arise in
working out ADR-0019 are settled in the phase that implements it and do
not count against Phase 0's exit." It covers questions that arise in
working out ADR-0019 and nothing else; it defers nothing in D1 to D10 that
is not such a question, and at the close no point of D1 to D10 rests on
it, since every question ADR-0019 has raised is decided. Closed on the
project owner's instruction of 2026-11-21 to check D1 to D10 and close
Phase 0, and the owner's four answers of 2026-11-22. What is left beyond
the exit sentence stays listed, under "What is left", among it one case
waiting on the owner under ADR-0012's Rule 11, QK-0760's residual case,
which this record reads as a case under the decided policy and not a
point of D10 (section 3, "A case open for the owner under ADR-0012").

*Later on 2026-11-22 the project owner accepted every recommendation then
open. For this record that is: QK-0760's residual case is deferred
to Phase 5, as proposed, and no longer waits on the owner; and this
record's readings stand, among them that such a case is one under the
decided policy and not a point of D10, the reading of ADR-0013's §7 Layer
6 bullet with ADR-0014's amendment 5, and the reading of the cut-off
rule's "the phase that implements it". By signing the first slice of
`purse-udb`'s transaction store (`e5df76b`), after being told what
signing it accepts, the owner also accepted two points the pointers of
that change put: the slice's reading of a `birth` row pending from both a
height and a time, and the slice shipping before `PkScriptLocs`, whose
deadline ADR-0006 now states as before the slice that ports `AddCredit`
and `Rollback`'s credit step (ADR-0006, addendum "2026-11-22 (later)").
ADR-0006's open point 4 is not decided. The text below stays as written,
with dated pointers to this note; nothing of the verdict changes.*

All counts below are for the default sizes the normal test suite
(`PURSE_REQUIRE_ORACLE=1 cargo test --workspace`, as CI runs it on Linux,
macOS and Windows) uses, unless stated otherwise. They were read from one
run on an x86_64 Linux desktop, with `PURSE_TEST_SEED` unset, so each
test's inputs are fixed by its label.

## 1. CI on three platforms

> CI green on three platforms

**Held by** `.github/workflows/ci.yml`. Its `test` job runs
`cargo test --workspace --locked --no-fail-fast` with
`PURSE_REQUIRE_ORACLE=1` (a missing Go oracle fails a differential test
rather than skipping it) on `ubuntu-latest`, `ubuntu-24.04-arm`,
`macos-latest` and `windows-latest`, so on Linux (x86-64 and arm64), macOS
and Windows. The other jobs: the same suite with overflow checks off, as
release builds ship; rustfmt and clippy with `-D warnings`; `cargo doc` with
`-D warnings`; cargo-deny; `go vet` and `go test` for `tools/oracle` and
`tools/pinmove`; the FLINT differential of `purse-mixsolver`; the
mixsolver speed gate (ADR-0010); the benchmark ratio gate against Go
(ADR-0014); and a fuzz smoke of 60 s per target *(and, since later on
2026-11-22, a constant-time job for the wiped base58 codecs, timing and
disassembly: ADR-0012, the note of that date under the addendum of
2026-11-21)* *(later still on 2026-11-22 its timing steps moved to a
workflow of their own, `.github/workflows/constant-time.yml`, run on the
paths that can change their result and nightly, and the job kept the
disassembly: ADR-0012, addendum "2026-11-22 (later)")*. The run on `69a6f6d`, the commit the check was made at, is
GitHub Actions run 44892877145: all 14 jobs passed. The run on `6fa3851`
(ADR-0019 and pointers to it) is run 44989643104: all 14 jobs passed. The
run on `ff59d84` (the owner's answers to ADR-0019's FB1 to FB10) is run
44997525002: all 14 jobs passed. In each of the three the `test` job on
`ubuntu-latest`, `ubuntu-24.04-arm`, `macos-latest` and `windows-latest`
was among them. The clause rests on those three runs. No run exists yet
for the commit carrying this record, which changes documents only. Of the
files it changes, a test reads only ADR-0001
(`crates/purse-export-check/tests/frozen.rs`, which checks that every
registry section is specified there; the commit changes it by dated
pointers only), and the workspace suite passes on this record's tree on
an x86_64 Linux desktop ("Verdict"). That commit's CI result is not
claimed here.

## 2. The reader's dump against Go's

> the Rust reader's dump of Go-written databases (all fixtures in `wallet/udb/testdata` plus a funded simnet wallet) equals the Go dumper's

As amended: the funded simnet wallet is dropped (below), so the clause is
held for every upstream fixture and for generated databases.

**Held by** `crates/purse-boltread/tests/oracle.rs`, against the oracle
command `boltdump` (`tools/oracle/cmd/boltdump`, Go bbolt v1.3.12), which
writes a database's whole bucket tree as a canonical sorted hex dump with
its txid, entry count and SHA-256. Each test has Go and `purse-boltread`
dump the same file and compares the two dumps byte for byte, reporting the
first differing line; then compares Go's `BucketStats` with the reader's,
field for field, and runs random cursor scripts (first, last, next, prev,
seek to keys of chosen kinds) through both, op for op. 16 tests, all
passing:

| Test | Databases | What is compared |
|---|---|---|
| `fixtures_match_go` | every fixture database in dcrwallet's `wallet/udb/testdata` at the pin: `v1`, `v2`, `v3`, `v4`, `v5`, `v7`, `v11` and `v24.db.gz` (the eight `.db.gz` files the directory holds, copied byte for byte into `crates/purse-boltread/tests/data/upstream`), plus the two frozen generated files `frozen-4k.db` and `frozen-deep-4k.db` and `frozen-4k.db` with its newest meta page invalidated | 11 databases: Go's dump digest, entry count and txid against `tests/data/dump-digests.txt`; for the 10 whole files the two dumps byte for byte, the stats of every bucket against Go and against `tests/data/bucket-stats.json`, and a 60-operation cursor script per bucket. The test asserts that it checked 8 + 3 files. Entry counts from Go: v1 3,462; v2 59; v3 56; v4 79; v5 100; v7 80; v11 223; v24 87 |
| `small_1k`, `small_4k`, `small_16k_binary_names`, `small_64k` | databases Go generates from a recipe and a seed, at page sizes 1,024, 4,096, 16,384 and 65,536 | dumps byte for byte (218, 136, 193 and 1,311 entries), stats of every bucket, 11 cursor scripts of 300 operations each |
| `empty_database` | an empty database, 4,096-byte pages | dump, stats, one cursor script |
| `inline_only_4k`, `inline_only_16k` | about 340 inline buckets | dumps (931 and 928 entries), stats of 40 buckets, 21 cursor scripts of 200 operations |
| `deep_4k`, `deep_16k` | 156 nested buckets, 70 MB and 206 MB files | dumps (12,126 and 10,315 entries), stats of 40 buckets, 13 cursor scripts of 400 operations in each |
| `overflow_4k`, `overflow_16k` | values on overflow pages, 17 MB and 43 MB files | dumps (214 and 270 entries), stats of every bucket, 11 cursor scripts of 300 operations |
| `large_4k` | at least 200,000 keys (210,516 entries) in a 104 MB file | dump, stats of every bucket, 7 cursor scripts of 2,000 operations |
| `lock_against_go_writer`, `go_reader_and_rust_reader_coexist`, `dcrwallet_open_waits_for_the_reader` | a database a Go process holds open | the file lock against Go, with a variant of each test for Windows' share modes |

Beside it: `crates/purse-boltread/tests/corrupt.rs` (`targeted_cases_match_go`
and `synthetic_files` compare damaged and synthetic files with Go;
`random_damage_never_panics` and the targeted mutations hold the reader to
errors, never panics), `tests/fixtures.rs` (`fixtures_match_recorded_go_dumps_and_stats`
replays the recorded Go dumps and stats without Go), and the fuzz target
`fuzz/fuzz_targets/boltread_open.rs`. Databases dcrwallet's own code wrote
(`tools/oracle/cmd/walletgen`: the upstream fixtures upgraded to version
26, fresh simnet wallets with and without its scripted scenario) are read
with `purse-boltread` by `crates/purse-export-check/tests/oracle.rs`
(`generated_databases_export_deterministically_and_match_the_file`, which
checks the Go export against what the reader reads) and by `purse-snacl`'s
tests, but no test dumps them through `boltdump` and compares the dumps.

**The funded simnet wallet is dropped.** No funded simnet wallet was ever
dumped and compared. The reader was built as the foundation of an importer
and has been a test utility only since the project owner dropped the
import on 2026-11-18 (ADR-0013). ADR-0013 Open point 2, decided as proposed
by the project owner on 2026-11-19, keeps `purse-boltread` as a test
utility and says: "the funded-simnet-wallet comparison of the old Phase 0
exit, never run, is dropped". The brief's Phase 0 bullet and Exit say the
same.

**Not covered:** a database with a real chain history behind it (a funded
simnet wallet, or any wallet that synced). Generated databases cover the
page shapes, sizes and page sizes above; they do not cover what bbolt does
over a long life of small commits.

## 3. D1 to D10

> D1–D10 decided (D4 has since been superseded by ADR-0013)

Each decision's record and Status line, who decided and when, what is still
open inside it, and a verdict: **decided**; **decided, with points the
record defers** (to a named phase or event); or **not decided in full**
(a point is marked Proposed, awaiting the owner, or left to the owner
without a proposal, and in either case not deferred). A recommendation or
a draft counts as no decision. *Since 2026-11-22 a point is also deferred
when the cut-off rule the owner accepted that day covers it: "Questions
that arise in working out ADR-0019 are settled in the phase that
implements it and do not count against Phase 0's exit." The rule covers
questions that arise in working out ADR-0019 and nothing else; it defers
nothing in D1 to D10 that is not such a question. At the close no point
rests on it ("ADR-0019, accepted after the check" below).* The check of
2026-11-21 found D1, D2, D3 and D10 not decided in full, on eight points
between them; the rows below give each point, the recommendation put with
it, and the owner's decision of 2026-11-22, in the owner's words "Accept
all eight", which accepts each point as recommended. Where a decision is
recorded, the row names the addendum or Status line. Read again on
2026-11-22 at `6fa3851`, D2's record carried one more question, which
ADR-0019 brought into it after the check (point 9), and read again after
each of the owner's answers of the same day on ADR-0019's questions it
carried FB11 and FB12 (point 10), then FB13 (point 10), all since decided
(D2's row, and "ADR-0019, accepted after the check" below). D1's row
counts the two points ADR-0013 still marked Proposed at the check, put to
the owner as point 11 and decided ("Proposed in ADR-0013" below).

| D | Record | Status, as the record states it | Open inside it | Verdict |
|---|---|---|---|---|
| D1 Name and drop-in identity | ADR-0005, retitled "Name, data directory and identity" on 2026-11-22 (file name kept); ADR-0013 for the part the import took away | ADR-0005: "Accepted (decision D1; project owner, 2026-11-18) as proposed, with one change: the dcrd certificate fallback also tries dcroxide's directory"; the import front end void by ADR-0013 (Accepted, project owner, 2026-11-18); addendum of 2026-11-19: the P2P user agent `/dcrwire:1.0.0/dcr-purse:0.1.0/` and the JSON-RPC `version` key `dcrwallet` (project owner, 2026-11-19); "Decided, 2026-11-22 (project owner): the three items the addendum of 2026-11-18 marks Proposed are accepted as proposed" (addendum of 2026-11-22) | Deferred by the record: "Installing as `dcrwallet` for Decrediton (c) still waits for a Phase 8 addendum"; the shape of the value under the `dcr-purse` key, "decided in Phase 7, when the JSON-RPC server is written" (project owner, 2026-11-20). **Decided on 2026-11-22, as recommended** (points 1 to 3 under "Verdict"), the three items the addendum of 2026-11-18 marked Proposed: "Shared directory" (with `--appdata` naming a dcrwallet directory, dcr-purse uses its `rpc.cert`, `rpc.key` and `clients.pem`, shares `peers.json` and puts its store beside a `wallet.db` it never opens), accepted as proposed; "Title" (retitle the record "Name, data directory and identity", keeping the file name), accepted; and the "Phase 8 exit" bullet's replacement of the brief's M3 sentence on the old `rpc.cert` ("an existing `dcrctl --wallet` configuration connects once its `rpccert` names dcr-purse's `rpc.cert`; nothing is carried over"), accepted as proposed, which decides the same bullet of ADR-0013 Open point 6 and the brief's Phase 8 exit. Whether dcr-purse's `peers.json` stays readable by dcrwallet stays with ADR-0009 Decision item 5, as the addendum says. Nothing in code reads a data directory yet, so the shared directory is neither built nor tested. **Decided on 2026-11-22, as recommended** (point 11 under "Verdict", option (A); "Accept all three"), the two points ADR-0013 still marked Proposed at the check, counted against D1 on the plain reading of the test above: its Open point 6's bullets other than Phase 8's (how the tests of Phases 3 to 7, §7 Layer 6 and M1 reach matching state without an import) and its Open point 8 (three losses of a restore found after the decision), each as proposed. ADR-0013 marks no point Proposed; the brief's *Proposed* markers on those texts are lifted; the two losses join ADR-0013 Decision item 4's list and its copies; the third fact is owed to the user documentation of ADR-0013 Open point 4, not yet written | decided, with points the record defers (to Phases 7 and 8) |
| D2 Storage engine and schema | ADR-0003 | "Engine **Accepted**: redb (project owner, 2026-11-14, brief §9 D2), major 5 (project owner, 2026-11-18 ...). The schema, the rules around it and the answers to the nine open points are **Accepted** as recommended (project owner, 2026-11-19)"; height keys and unmerged tables (project owner, 2026-11-19); R1 to R4 and M1 to M3 "each as recommended" (project owner, 2026-11-21); "Confirmed, 2026-11-22 (project owner): redb 5 is D2's engine" | **Decided on 2026-11-22, as recommended** (point 4 under "Verdict"): the confirmation of the engine by a Phase 0 prototype that the decision of 2026-11-14 made redb subject to ("to be confirmed by a Phase 0 prototype measured against a large wallet's workload", brief §9 D2), which no record had stated: the acceptance of 2026-11-19 covered "the schema, the rules around it and the answers to the nine open points", Open point 1 reading "Accept Decision items 2 to 8", and the closing of Phase 2 accepted the measurement of the store as built without saying the engine was confirmed. The owner confirmed redb 5 on the prototype's figures (ADR-0003's spike, `spikes/store-redb` with its Go baseline `tools/oracle/cmd/storebench`, wallets of 10k, 100k and 1m transactions) and Phase 2's ([PHASE2-EXIT.md](PHASE2-EXIT.md), clause 5), both from a shared desktop, recorded by pointer in ADR-0003's Status and brief §9 D2. Deferred by the record: open points 2 and 7 (the defects seen in the spike and public keys in `addr_by_hash`, decided in Phase 3); item 6 of the owner's decisions of 2026-11-19 (whether the wallet opens its own store with redb's integrity check, `OpenOptions::check_integrity`, "is decided in Phase 3"); R4 of 2026-11-21 (the multisig stake-tree flag after a spend and unspend, QK-0054, "decided in Phase 3 with an oracle run of `redeemmultisigouts`", its disposition open until that run); M2 and M3 (the dedicated-machine run before Phase 3's exit, which could move the commit rows, and the measurement with Phase 3's transaction store at its exit) *(2026-11-24, project owner: M2 after the next transaction-store slice, and not before; ADR-0014, addendum of that date; not made)*. Not a decision, but owed: the nightly job of item 9 of 2026-11-19 (the model-based property test at a large count), which CI does not have yet. **Decided on 2026-11-22, as recommended** (point 9 under "Verdict"): ADR-0019's FB9, option (a), and FB5, which ADR-0003 Decision item 2's `meta` row and its addendum of 2026-11-21 left open since `6fa3851`, decided with FB1 to FB10 ("go with your suggestions for fb1-10"); and FB11 and FB12, which the same addendum then named "**Proposed**", decided as recommended ("Accept both"; point 10). ADR-0003 records each by a dated pointer; no table, key, value or invariant changes. **Decided on 2026-11-22, as recommended** (point 10 under "Verdict"; "Accept all three"): ADR-0019's FB13, which the same addendum named "**Proposed**", as option (a): when the first block a chain switch attaches while a birthday set from time is pending shows a block the store holds without its filter, below the stored tip, to be the birthday block, the syncer fetches that block's filter, checks it and writes it as part of the same chain switch, so the addendum's content of `cfilters` and the meaning of `have_cfilters` under FB9 (a) stay exact. ADR-0003 records it by a dated pointer, with the cut-off rule: a question that arises from now on in working out ADR-0019, one bearing on what ADR-0003's tables hold included, is deferred by that rule to the phase that implements ADR-0019. None is open at the close | decided, with points the record defers (to Phase 3, and by the cut-off rule any later question arising in working out ADR-0019) |
| D3 How dcroxide crates are consumed | ADR-0006 | "Accepted (decision D3; project owner, 2026-11-18) as proposed, with one change: dcr-purse keeps the latest stable Rust"; open points 1 to 6 decided 2026-11-18; addenda of 2026-11-19 and 2026-11-20 decided by the owner; "Decided, 2026-11-22 (project owner): both items of the addendum of 2026-11-18 marked Proposed are confirmed" (addendum of 2026-11-22) | Deferred by the record: open point 4, dcroxide's CI building dcr-purse, "not now; revisit before Phase 3, when the wallet first links `dcroxide-stake` and `dcroxide-gcs`" (no crate links them yet; Phase 3's address manager slice links neither, so the event named is `purse-udb`'s transaction store) *(later on 2026-11-22 the transaction store's first slice links both, without default features; open point 4 was not revisited before it, and is put to the owner with that change)*. **Decided on 2026-11-22, as recommended** (points 5 and 6 under "Verdict"), the two items of the addendum of 2026-11-18 marked Proposed, both already settled in substance by accepted text. Item 1, `PkScriptLocs`, with ADR-0013 Open point 9: confirmed by pointer to rule 3's table (accepted 2026-11-18, due "at the latest Phase 3") and ADR-0003 Decision item 2 (accepted 2026-11-19, which keeps each credit's script offset and length, "so dcroxide's `PkScriptLocs` stays needed"), and, as an explicit change, now due before `purse-udb`'s transaction store, the part of Phase 3 that writes credits, instead of before Phase 3, whose start had passed (the address manager slice, `94afd6e`, accepted 2026-11-20). The dcroxide pin `48f5b5e` does not carry it: it is still to be written in dcroxide. Item 2, `purse-boltread` "not offered as part of M1 and not published while it is a test utility": confirmed by pointer to ADR-0013 Open point 2 (decided 2026-11-19; `crates/purse-boltread/Cargo.toml` carries `publish = false`), with a dated pointer on ADR-0006's Consequences line "Publishing" | decided, with a point the record defers (open point 4, before the transaction store) |
| D4 Importable dcrwallet databases | ADR-0007, superseded by ADR-0013 | ADR-0007: "Superseded by ADR-0013 (2026-11-18): superseded in full by the project owner's decision of 2026-11-18 that dcr-purse imports no dcrwallet database and migration is by seed restore only". ADR-0013: "It supersedes ADR-0007 in full, so decision D4 of the brief falls away". The ADR index: "Superseded by ADR-0013 (2026-11-18); kept as the record"; brief §9 D4 the same | nothing: no database is imported, so no version is importable and dcrwallet's database version is not tracked | superseded, as the exit sentence allows |
| D5 gRPC transport and protobuf | ADR-0004 | "**Accepted** (project owner, 2026-11-19), against the proposal: option (A), tonic, not (B1); the other questions as recommended"; in the owner's words, "ok go with tonic and your recommendations for the rest": tonic and prost over hyper and h2, the tokio runtime confined to the listener | The record defers: how the 222 messages are generated without `protoc` ("settled at the start of Phase 12"); which of tonic's 19 gRPC-layer differences Decrediton can observe (established first in Phase 12, with Decrediton's call inventory); ADR-0014's Phase 12 speed criterion, measured once the listener exists. The Phase 0 spike was made and recorded: `spikes/grpc-intree` (in-tree HTTP/2, option C) and `spikes/grpc-tonic` (option A) answer `VersionService.Version` to an unmodified grpc-go v1.83.2 client over mutual TLS (`tools/oracle/cmd/grpcclient`, reference server built as dcrwallet builds its own; `tests/interop.rs` `grpc_go_client_cases`, 13 cases in each spike, with raw HTTP/2 and I/O-level cases beside them). Its other half was not: "Decrediton was not available", and only Linux x86_64 was run (ADR-0004, "What remains open") | decided, with points the record defers |
| D6 Concurrency model | ADR-0008 | "Accepted (decision D6; project owner, 2026-11-19): OS threads everywhere, with an async runtime allowed only inside the gRPC listener ... Channels and multi-way `select` come from `crossbeam-channel` ... Open points 2 to 4 were decided as proposed"; open point 1 (`crossbeam-channel`) 2026-11-18; addenda: tonic at the listener seam (ADR-0004), the speed departures (ADR-0014), rayon for Argon2id's lanes, a pool per derivation (project owner, 2026-11-19), no process-wide lane budget for now (project owner, 2026-11-20) | nothing undecided. The one "proposed" left in its text (Consequences, "Risks accepted": a slow client of `GetTransactions`, `GetTickets` or `GetCFilters` grows the redb file, "proposed: accept it, as Go behaves so") stands under the heading of risks the record accepts, in the record the owner accepted. The brief's §9 D6 still read "Proposed", which was out of date against ADR-0008 and the ADR index; corrected with this record | decided |
| D7 Version skew against dcroxide | ADR-0009; the ledger `docs/SKEW.md` | "Accepted as proposed (decision D7; project owner, 2026-11-18), the answers proposed under Open points included"; addenda of 2026-11-19, 2026-11-20 and 2026-11-21 record the pin moves to dcroxide `f4b3d2e`, `2074168` and `48f5b5e` | Deferred by the record: open point 3, reusing `dcroxide-addrmgr` with four recorded divergences "conditional on the Phase 10 differential" | decided, with a point the record defers |
| D8 Mixing root solver | ADR-0010 | "Accepted as proposed (decision D8; project owner, 2026-11-18), the answers proposed under Open points included: a native solver in process, and `--csppsolver` accepted and never executed" | nothing undecided; `purse-mixsolver` is built, with the FLINT differential and the speed gate in CI | decided |
| D9 Error-message fidelity | ADR-0011 | "Accepted as proposed (decision D9; project owner, 2026-11-18), the answers proposed under Open points included" | nothing undecided; open point 7 names the Decrediton reading before Phase 8 as a trigger for revisiting, and "This decision does not wait for it" | decided |
| D10 Standing quirk policy | ADR-0012 | "Accepted (decision D10; project owner, 2026-11-18) as proposed, with one change" (QK-0279 fixed); "Amended, 2026-11-19 (project owner)" (Rule 7's default: funds held until a restart or an unlock are fixed); "Settled, 2026-11-20 (project owner)"; "Decided, 2026-11-21 (project owner)" (the base58 digit search to be replaced, scheduled at low priority, not yet done) *(done later on 2026-11-22: ADR-0012's Status line now says so)*; "Decided, 2026-11-22 (project owner)" (addendum of 2026-11-22) | **Decided on 2026-11-22, as recommended** (points 7 and 8 under "Verdict"): item 3 of the addendum of 2026-11-19, how a Rule 7 case whose difference a client could observe is brought to the owner, "as Rule 11 brings one, the proposed disposition in the QUIRKS entry, marked \"pending owner\", named in the change submitted for signing", confirmed as proposed; and the second point under "Not settled" in the same addendum, "Whether brief §3 W1 itself should cover these funds", which had no proposal: W1's text stays as it is and the class is governed by Rule 7's default alone. Not counted: "what, if anything, must be run before the owner passes a report on is the owner's to decide" (addendum of 2026-11-19 on the route of upstream reports, item 1), which is the owner's standing discretion over delivering reports, not a pending decision. Deferred by the record: QK-0203 (Phase 5) and the fee credits of QK-0282 (Phase 9), each decided after an oracle run; the base58 digit search's replacement, decided and scheduled at low priority with no phase named, not done (addendum of 2026-11-21) *(made later on 2026-11-22: the note of that date under the same addendum)*. Not counted, and named: QK-0760's residual case (`docs/QUIRKS.md`, QK-0760; `docs/PARITY.md`, `purse-udb`), a per-case ruling under Rules 7 and 11 that waits on the owner, with a proposed deferral to Phase 5 the owner has not accepted *(later on 2026-11-22: deferred to Phase 5 as proposed, project owner; see the dated note at the top)*. It is a case brought under the decided policy, not a point of D10's record, which does not name it ("A case open for the owner under ADR-0012" below) | decided, with points the record defers (to Phases 5 and 9), and a decided change not yet made (the base58 digit search) *(made later on 2026-11-22: ADR-0012, the note of that date under the addendum of 2026-11-21)*; one case under the policy, QK-0760's residual case, waits on the owner outside the record *(later on 2026-11-22: deferred to Phase 5 as proposed, project owner; see the dated note at the top)* |

All ten hold: D1, D2, D3, D5, D7 and D10 decided with points their
records defer to later phases or events, D6, D8 and D9 decided with
nothing open, and D4 superseded by ADR-0013, as the exit sentence allows.
The eight points the check of 2026-11-21 found open (three in D1's
record, the engine's confirmation for D2, two in D3's, two in D10's) were
decided by the project owner on 2026-11-22, each as recommended, and so
were the points found or put after them: ADR-0019's FB9 and FB5 (point
9), FB11, FB12 and FB13 (point 10) for D2, and ADR-0013's Open point 6's
other bullets and Open point 8 for D1 (point 11).

**The markers left, read again at the close.** Every marker of a point
still Proposed, open or left to the owner in the records of D1 to D10
(ADR-0003, 0004, 0005, 0006, 0008, 0009, 0010, 0011, 0012 and 0013; ADR-0007
is superseded), read on 2026-11-22 after the owner's last answer was
recorded, and why each counts against no clause:

| Record | Marker | Why it does not count |
|---|---|---|
| ADR-0003 (D2) | Decision item 9, "Proposed for the owner (Open point 4)"; Open points 1 to 9 "each for the owner"; the file name "awaiting" the owner | each decided: Open points 1 to 9 as recommended on 2026-11-19 (Open point 4 is item 9), the file name `wallet.redb` on 2026-11-21; each carries its mark |
| ADR-0003 (D2) | the addendum of 2026-11-21 naming ADR-0019's FB9, FB11, FB12 and FB13 "**Proposed**" | each followed by a dated pointer recording its decision of 2026-11-22 |
| ADR-0003 (D2) | open points 2 and 7, item 6 of 2026-11-19 (the integrity check), R4 (QK-0054), M2, M3 | deferred by the record to Phase 3, its oracle run or its exit |
| ADR-0003 (D2) | the nightly job of item 9 of 2026-11-19 | a decided piece of work not yet built, not a pending decision |
| ADR-0004 (D5) | "Questions for the owner" 1 to 7; "What remains open" | the questions answered by the acceptance of 2026-11-19 (option (A), "the other questions as recommended"); what remains open is untested ground and work the record defers to Phase 12 |
| ADR-0005 (D1) | Open point 6, "Proposed: no, independent" | accepted with the record as proposed on 2026-11-18 |
| ADR-0005 (D1) | (c), installing as `dcrwallet` for Decrediton; the shape of the value under the `dcr-purse` key | deferred by the record to Phase 8 and Phase 7 |
| ADR-0005 (D1) | "release version identity is open" | dcroxide's, not dcr-purse's |
| ADR-0006 (D3) | open point 4, "Proposed: not now; revisit before Phase 3" | accepted as proposed, and deferred by the record to before `purse-udb`'s transaction store |
| ADR-0008 (D6) | "Not decided, and the owner's" (a process-wide lane budget) | decided 2026-11-20, marked |
| ADR-0009 (D7) | Open points' "Proposed:" lines; open point 3 | accepted as proposed with the record ("the answers proposed under Open points included"); open point 3 conditional on the Phase 10 differential |
| ADR-0010 (D8) | none | — |
| ADR-0011 (D9) | Open points' "Proposed:" lines; open point 7's revisit | accepted as proposed with the record; the revisit is a trigger, "This decision does not wait for it" |
| ADR-0012 (D10) | "Not settled" points of 2026-11-19; "pending owner" | each "Not settled" point decided (2026-11-20 and 2026-11-22), marked; "pending owner" is the decided procedure by which Rule 11, and Rule 7 where a client could observe the difference, bring each case to the owner, and one case waits under it now, QK-0760's residual case *(later on 2026-11-22: deferred to Phase 5 as proposed, project owner; see the dated note at the top)*, which no record of D1 to D10 names and which counts as a case under the policy, not a point of D10 ("A case open for the owner under ADR-0012" below); the owner's discretion over delivering reports is not a pending decision; QK-0203 and QK-0282 deferred to Phases 5 and 9 |
| ADR-0013 (D1 part, D4) | Open points 6 and 8 | decided 2026-11-22, marked; no point marked Proposed |
| ADR-0013 (D1 part, D4) | Open point 6's §7 Layer 6 bullet: the record's reading that ADR-0014's amendment 5 stands over the bullet's words "the requests the backend saw", which the owner has not ruled on | not a point Proposed or left to the owner: the bullet (2026-11-22) and amendment 5 (2026-11-19, accepted for this layer) are both accepted, and the reading reconciles them as the only one under which R1 and R2, both accepted, pass Layer 6; the owner may confirm or correct it *(later on 2026-11-22: it stands, project owner; see the dated note at the top)* |
| ADR-0013 (D1 part, D4) | Decision item 9, "Not decided" | part of what the owner accepted on 2026-11-18: two possibilities recorded as not plans, no question put |
| ADR-0013 (D1 part, D4) | the user documentation of Open point 4 | decided 2026-11-19 and not yet written; Open point 8 adds to it |

No marker is left in those records that names a point neither decided
nor deferred. The cut-off rule defers nothing at the close: ADR-0019's
questions FB1 to FB13 are all decided, and what ADR-0019 already left to
its code (the reply of the filter RPCs when a connected backend fails to
serve a filter, under its rule 3; the specification of the projection and
range record of its rule 6) concerns the filter RPCs and the export, not
D2's tables, and is in any case left by ADR-0019 itself to the phases
that implement it.

**A case open for the owner under ADR-0012.** Outside those records, one
QUIRKS entry waits on the owner: QK-0760's residual case (`docs/QUIRKS.md`,
QK-0760, its last paragraph; `docs/PARITY.md`, `purse-udb`). QK-0760's fix
(Rule 7) replaces, in the SLIP0044 coin type upgrade, the address rows
account 0 recorded under the legacy coin type, so that no address is
signed for with another address's key. It leaves the other outcome the
entry names: after the upgrade, a payment to a legacy address that an
earlier instance of the seed handed out is neither recognised nor
spendable until the seed is restored again, as in Go. Closing that too
would keep account 0's legacy keys and rows beside the SLIP0044 account;
the entry proposes keeping its disposition for the address manager slice
and deciding that alternative in Phase 5, with discovery and the wallet's
transaction filter, where its recognition can be tested end to end. The
owner has neither ruled on the case nor accepted that deferral. This
record does not count it against D10. ADR-0012 decides a policy, and its
Rule 11 provides that a case no rule settles goes to the owner when it is
found, with the proposed disposition in its QUIRKS entry; a case waiting
under that procedure is the decided policy at work, not a part of it left
undecided. The records have treated such cases so: QK-0744 (pending owner
under Rule 11, ruled 2026-11-19) and the first half of QK-0035 (pending
owner under Rule 11, ruled 2026-11-20) waited on the owner while D10
stood Accepted, and QK-0744's ruling is recorded in its QUIRKS entry and
PARITY.md, not in D10's record. No record of D1 to D10 names QK-0760;
QK-0203 and QK-0282 are in D10's row because the owner's addendum of
2026-11-19 defers them by name. Read the other way, any case found in a
later phase would reopen Phase 0. That reading is this record's, and the
owner has not ruled on it. Two things are stated plainly. Rule 11 asks
that a case be named in the change submitted for signing; the change that
added the entry (`94afd6e`) named QK-0760's fix but not the residual case,
so the entry now carries Rule 11's mark, "pending owner", and the change
carrying this record names it. And Rule 11's interim rule ("Until the
ruling the code reproduces Go, unless Rule 6 or 7 applies") is not met to
the letter: Rule 7 applies, yet the residual outcome is Go's, because the
alternative needs the transaction filter of Phase 5. No code that exists
can meet the outcome, since nothing yet discovers addresses, syncs or
matches a payment. If the owner accepts the proposed deferral, the case is
deferred to Phase 5 as QK-0203 is; on either reading it is listed under
"What is left". *(Later on 2026-11-22 the owner deferred the case to
Phase 5, as proposed, and ruled that this record's readings stand, the
reading above among them; see the dated note at the top. QK-0760's entry
and `docs/PARITY.md` record the deferral.)*

**Proposed in ADR-0013.** ADR-0013, which the ADR index lists against D1
(part) and D4, had three open points still Proposed at the check: 6 (how
the tests that relied on import reach matching state), 8 (the
last-returned counters, the birthday and account discovery needing an
unlocked wallet, to join the list of what a restore loses) and 9
(`PkScriptLocs`). Point 9 and point 6's Phase 8 bullet, the replacement of
the M3 sentence, which ADR-0005's addendum also states, were decided on
2026-11-22 with D3 and D1 (above). Point 6's other bullets (how the tests
of Phases 3 to 7, §7 Layer 6 and M1 reach matching state without an
import) and point 8 stayed Proposed, and no record deferred them. This
record had read them as concerning the restore, not D1's question (name
and drop-in identity), and so as counting against no clause; the owner
had not ruled on that reading. Read plainly, section 3's test counted
them against D1, since ADR-0013 is the record the ADR index lists against
D1 (part) and D1's row cites it for the part the import took away. So
they were put to the owner as point 11 under "Verdict", and on 2026-11-22
the owner decided both as proposed ("Accept all three"), which makes the
reading moot. ADR-0013 now marks every open point decided. Each text of
Open point 6 being decided was compared with the brief's text that
carried its *Proposed* marker: the Phase 3, 4, 5 and 7 bullets, Phase 6's
M2 bullet and the M1 and M2 rows of §11 carry ADR-0013's proposal in
substance (the Phase 3 exit keeps beside it the sweep of the
`wallet`-level query methods, which predates ADR-0013); §7 Layer 6 carries
it with one part already changed by an accepted decision, ADR-0014's
amendment 5, accepted on 2026-11-19 with ADR-0014 Open point 4 ("with
amendment 5, without which R1 and R2 fail Layer 6"), under which the
layer compares the probe decisions and outcomes rather than "the requests
the backend saw" where R1, R2 and R6 to R8 apply, and with ADR-0019's
pointer on "the same filters". ADR-0013's bullet, accepted as proposed,
still says "the requests the backend saw". ADR-0013 reads the two
accepted texts together, amendment 5 standing over those words where R1,
R2 and R6 to R8 apply, so that the layer is the one the brief carries.
That is the record's reading, not the owner's words: point 11 as put did
not show the difference between the bullet and the brief's text, and the
owner has not ruled on the reading. It is not a point left undecided by
the test above: both texts are accepted, neither is marked Proposed or
left to the owner, and the reading is the only one under which R1 and
R2, both accepted, pass Layer 6 ("The markers left, read again at the
close" above). *(Later on 2026-11-22 the owner ruled that this record's
readings stand, this one among them; see the dated note at the top.)*

**ADR-0019, accepted after the check.** On 2026-11-21, after the check was
made, the project owner decided "we will drop filters below the wallet's
birthday" ([ADR-0019](adr/0019-filters-below-the-birthday.md), committed
in `6fa3851`). The ADR index lists it against no decision, and it changes
no table, key, value or invariant of ADR-0003's schema (its Decision item
3). But ADR-0003, D2's record, gained with it a pointer in Decision item
2's schema table and an addendum, and both left part of what its tables
hold to ADR-0019's open questions: FB9, what the `meta` row
`have_cfilters` means for a store without the filters below the
birthday, and FB5, whether filters a client supplies below the birthday
are stored. Neither was deferred: ADR-0019's Status puts the rule's
implementation in the sync phases, not the answers to its questions, and
the code they bear on includes methods of dcrwallet's `udb.Store`
(`ExtendMainChain`, `MissingCFiltersHeight`, `InsertMissingCFilters`,
`ImportCFiltersV2`; `wallet/udb/txmined.go:203`, `:408`, `:430`,
`wallet/udb/cfilters.go:34`), which `purse-udb`'s transaction store ports
in Phase 3. So D2 was not decided in full, and the question was put to
the owner as point 9.

On 2026-11-22 the owner answered all ten of ADR-0019's questions, "go
with your suggestions for fb1-10" (recorded in `ff59d84`): FB9 as option
(a), the row meaning "no filter at or above the birthday height is
missing", 1 in every store dcr-purse creates as in Go, with the udb-level
`MissingCFiltersHeight` walking from the birthday height; FB5 as
recommended, supplied filters written as dcrwallet writes them; and the
other eight, FB4 (the rule enforced in the syncers, `purse-udb`'s port of
`ExtendMainChain` accepting a block without a filter only below the
birthday) and FB10 (a store with no `birth` row treated as born at
genesis) among them. That is point 9's option (A). The reading this
record gave of FB1 to FB4, FB6 to FB8 and FB10, that they counted against
no clause, no longer matters: all are decided.

Recording those answers found two more questions, which ADR-0003's
addendum named **Proposed** in `ff59d84`: FB11, how the store records the
filter of a block connected before its child is known while a birthday
set from time is pending, where FB4 and ADR-0019's decision item 1 as
worded could not both hold; and FB12, what the rules keyed on the
birthday height do while a birthday is pending. Both bear on what
`cfilters` and `cfilters_detached` hold and on where
`MissingCFiltersHeight` starts, so by the test above they counted against
D2 as FB9 and FB5 did. On 2026-11-22 the owner answered "Accept both":
FB11 as option (i) with (r2), FB12 as option (a) (ADR-0019, addendum
"2026-11-22 (later)"; ADR-0003, a dated pointer in the same addendum).

Checking those two answers before recording them, case by case under a
pending birthday (a block inside a batch, the last block of a batch, the
initial-sync tip, blocks announced after the initial sync, a detach while
pending, a height birthday above 0, a height-0 birthday, a time birthday
that resolves at genesis, supplied filters), found them consistent with
each other, with FB1 to FB10 and with ADR-0019's decision items in every
case but one, now ADR-0019's FB13. While a birthday set from time is
pending, a reorganisation whose first attached block has a timestamp
after the birthday's time resolves the birthday at the fork point
(`wallet/chainntfns.go:169`, `:186-196`). When the fork point lies below
the stored tip and is not the genesis block, its child on the detached
branch showed it below the birthday block, so the store holds it without
its filter; FB11 (i)'s write covers only a block whose child was not
known when it was connected, and the announcement paths fetch the filters
of the attached blocks only. Left so, the store lacks a main-chain filter
at the birthday height, against ADR-0019's decision item 1 and ADR-0003's
addendum, and `have_cfilters` reads 1 while, under FB9 (a)'s meaning, a
filter at or above the birthday is missing. ADR-0019 recommends option
(a): fetch that filter with the attached blocks' and write it through
`put_cfilter_at` in the transaction that connects the resolving block.
ADR-0003's addendum named FB13 **Proposed**, and its alternative would
narrow what that addendum says `cfilters` holds and what FB9 (a) says the
row means, so by the test above it counted against D2. No record
deferred it (point 10 under "Verdict").

On 2026-11-22 the owner was shown three things with recommendations:
FB13, option (a), that the syncer fetches the fork-point block's filter,
checks it and writes it as part of the same chain switch; ADR-0013's
remaining proposals (point 11), accept as proposed; and a cut-off rule,
proposed in these words: "Questions that arise in working out ADR-0019
are settled in the phase that implements it and do not count against
Phase 0's exit." The owner answered "Accept all three". That is the whole
of the owner's words. FB13 is decided as option (a) (ADR-0019, Status
block, mark on FB13 and addendum "2026-11-22 (FB13)"; ADR-0003, a dated
pointer in the same addendum; ADR-0001, the pointer under its Status
line). Checking the answer before recording it, against FB1 to FB12 and
ADR-0019's decision items, found no further case: it adds a request and a
write only in the case FB13 names, through the `put_cfilter_at` write
FB11 (i) already uses, and keeps decision item 1, FB9 (a) and the detach
exception as they stand. ADR-0019's rule 13 applies it to every
`ChainSwitch` call of both syncers at the pin, the RPC initial sync
(`getHeaders`, `chain/sync.go:325`, which also runs after the initial
sync through `getMissingHeaders`' fallback) among them, a path FB13's
analysis had not named and option (a)'s "whenever" covers; on the
announcement paths it adds one request to what rules 1 and 3 of ADR-0019
state, each of which carries a dated pointer to it. That check, like the
checks before it, was a reading of the source at the pin; nothing was
run.

**The cut-off rule.** Recorded in the owner's accepted words, dated
2026-11-22, where it governs: ADR-0019's Status block, with a note that a
question found later is put to the owner, with a recommendation, in the
phase that implements ADR-0019; ADR-0003's addendum of 2026-11-21, D2's
record, where such a question is deferred by the rule; and section 3's
test above. It covers questions that arise in working out ADR-0019 and
nothing else; it defers nothing in D1 to D10 that is not such a question.
ADR-0019 is implemented in more than one phase (the Phase 3 port of
`purse-udb`'s transaction store; the sync phases 5, 6 and 10; the RPC
servers for its filter RPCs, Phase 7 for `getcfilterv2`,
`validatepredcp0005cf` and `importcfiltersv2` and Phase 12 for gRPC
`GetCFilters`; and `purse-tools`' port of `cmd/repaircfilters`, whose
phase the brief does not fix);
ADR-0019 reads "the phase that implements it" as the phase that
implements the part of the record a question bears on, a reading the
owner has not ruled on. At the close the rule defers nothing: ADR-0019's
questions FB1 to FB13 are all decided. It matters for D2 only if a
question found later bears on what ADR-0003's tables hold; such a
question then does not reopen Phase 0. *(Later on 2026-11-22 the owner
ruled that this record's readings stand, ADR-0019's reading of "the phase
that implements it" among them; see the dated note at the top.)*

## What else the Phase 0 section lists

| Item | Status |
|---|---|
| Cargo workspace under dcroxide's lint, deny and toolchain policy | built. `Cargo.toml` `[workspace.lints]` is dcroxide's at `48f5b5e` (`unsafe_code` forbid, `missing_docs` deny, clippy `all` warn and `arithmetic_side_effects`); `deny.toml` keeps dcroxide's pure-Rust bans, `multiple-versions = "deny"` and crates.io plus dcroxide's repository as the only sources; `rust-toolchain.toml` pins 1.100.0, equal to `rust-version`, the latest stable as ADR-0006 rule 8 has it. CI runs clippy with `-D warnings` and cargo-deny. **Left:** dcroxide's `clippy.toml` (`disallowed-methods`: rustls's default-provider builders, `install_default` and ureq's default agent) has no counterpart here. No crate of the main workspace links rustls or ureq yet; the two gRPC spikes call `builder_with_provider` by hand. Wanted when TLS enters the workspace (Phases 7 and 12) |
| CI on Linux, macOS and Windows | built (clause 1) |
| Fuzz infrastructure | built: `fuzz/`, a cargo-fuzz workspace of its own on nightly, 11 targets (`backup_verify`, `boltread_open`, `hdkeychain_parse`, `mixsolver_roots`, `seed_decode`, `sntrup_decapsulate`, `store_decode`, `store_open`, `txauthor`, `udb_account_keys`, `udb_imported_data`) *(a twelfth, `wif_decode`, added later on 2026-11-22 with the base58 change, and a thirteenth, `udb_tx_record`, with the first slice of `purse-udb`'s transaction store)*; CI's fuzz smoke runs each for 60 s. No longer soak of any target has been run (PARITY.md, status values) |
| `tools/oracle`, a Go module pinned to dcrwallet `54b85b33` | built: `tools/oracle/go.mod` requires `decred.org/dcrwallet/v5` at `v5.0.1-0.20261105142009-54b85b336077`, with Go 1.25's GODEBUG defaults as dcrwallet's daemon; 17 commands under `cmd/`, each a line-JSON binary; the Rust side is `purse-testutil` (`oracle_or_skip`, `PURSE_REQUIRE_ORACLE`) |
| First shim: database dumper | built: `cmd/boltdump` (commands `dump`, `stats`, `cursor`, `generate` and the lock holders `hold`, `hold_start`, `hold_wait`), used by clause 2 |
| First shim: logical exporter | built: `cmd/walletexport`, format `purse-export 2` (ADR-0001, accepted by the project owner, 2026-11-19); the strict parser and conformance checks are `crates/purse-export-check`, the Rust producer of the address-manager sections is in `purse-udb`; the transaction-store sections' Rust producer waits for Phase 3 *(since later on 2026-11-22 `purse-udb` also produces the chain and transaction sections of the rows its transaction store's first slice writes; the sections of credits, debits, ticket commitments, multisig outputs and balances wait for later slices)* |
| First shim: operation-script runner | built: `cmd/oprunner`, format `purse-opscript 1` (ADR-0002, accepted by the project owner, 2026-11-19); the Rust runner and differential are `crates/purse-udb/tests/oracle.rs` (`generated_scripts_match_go`, 24 generated scripts by default, and the hand-written scripts), the corpus also run through Go by `purse-export-check` *(since later on 2026-11-22 also the transaction store's, `generated_txstore_scripts_match_go` and `generated_txstore_group_scripts_match_go`, 24 and 8 generated scripts by default)* |
| First shim: address deriver | built, under another name: no command is called the address deriver; its role is the `wallet` command of `cmd/hdkeychain` (`wallet.go`: dcrwallet's `udb.HDKeysFromSeed`, account derivation as `Manager.NewAccount` does, the first address of each branch through `deriveChildAddress` and `compat.HD2Address`), compared by `crates/purse-hdkeychain/tests/wallet_diff.rs` ([PHASE1-EXIT.md](PHASE1-EXIT.md), section 1) *(since 2026-11-22 by `crates/purse-udb/tests/wallet_diff.rs`, through `purse-udb`'s address manager: the dated note of that date in the same section)*; `cmd/oprunner` derives addresses through dcrwallet's address manager too |
| Pins of the acceptance clients and peers (Decrediton, `dcrctl`, vspd, dcrd) recorded next to the dcrwallet pin (D12) | **not done.** The table exists ([PARITY.md](PARITY.md), "Pins and acceptance releases", ADR-0016 Decision item 4) and its four rows are empty: "Phase 0's acceptance work, which picks the releases, has not been done". The rule would give dcrd `release-v2.1.7` today; vspd's, Decrediton's and `dcrctl`'s releases have not been examined |
| Decrediton's wallet launcher and gRPC call inventory read into specs 19 to 21 | **not done.** Spec 21 §13, open question 1 ("Decrediton's exact invocation ... was not verified: Decrediton's source is not among the pinned inputs") stands. The brief has it done "before Phase 8"; ADR-0004 (tonic's differences Decrediton can observe), ADR-0011 open point 7 and ADR-0015 (the status ending an overflowed stream) wait on it |
| Docs skeleton: the brief, `specs/`, PARITY.md, QUIRKS.md, an ADR directory | built: the brief; 25 specifications under `docs/specs` with an index; `docs/PARITY.md`; `docs/QUIRKS.md` (QK-0001 to QK-0760) *(QK-0761 to QK-0769 added later on 2026-11-22 with the rest of the address manager, and QK-0780 and QK-0781 with the first slice of `purse-udb`'s transaction store)*; `docs/adr` (ADR-0001 to ADR-0019 and an index); beside them `docs/SKEW.md` (D7), `docs/UPSTREAM.md` (D12), `docs/bench-ledger.md` (ADR-0014) and the exit records |
| Decision ADRs D1 to D10 drafted | done: each of D1 to D10 has a record (clause 3) |
| Spike for D2: a redb schema prototype measured on a large wallet's workload | made: `spikes/store-redb`, Go baseline `tools/oracle/cmd/storebench`, recorded in ADR-0003 ("What was measured, and how", "Results"); redb 5 confirmed on its figures and Phase 2's by the project owner on 2026-11-22 (clause 3, D2) |
| Spike for D5: a minimal gRPC server answering `VersionService.Version` to an unmodified grpc-go client and to Decrediton over mutual TLS | made for grpc-go, not for Decrediton: `spikes/grpc-intree` and `spikes/grpc-tonic`, recorded in ADR-0004; "Decrediton was not available". The spikes are workspaces of their own, outside CI; their tests run by hand (ADR-0004's addendum of 2026-11-20 records them passing at dcroxide `2074168`; they were not run for this record) |

None of the items marked left or not done is part of the exit sentence.
This record does not hide them: the acceptance pins are due before the
first milestone is accepted on a simnet scenario against real Go peers
(brief §4 principle 8), the Decrediton reading before Phase 8, and the D5
spike's Decrediton half falls to Phase 12, whose exit runs Decrediton
itself.

## Verdict

The check of 2026-11-21 found eight points, in the records of D1, D2, D3
and D10, that the project owner had not settled and the records did not
defer, and put each to the owner with a recommendation. Items 1 to 3 and 7
needed a decision; item 4 the confirmation the owner's own decision of
2026-11-14 asked for; items 5 and 6 were settled in substance by accepted
text and needed a confirmation by pointer, item 5 also a new deadline;
item 8 had no proposal in its record. On 2026-11-22 the owner, shown the
eight in a table with their recommendations, answered "Accept all eight".
That is the whole of the owner's words; it accepts each point as
recommended. Below, each point as it was put, then what was decided and
where it is recorded; then point 9, the question found on 2026-11-22 in
D2's record, decided the same day; then point 10, the questions found
while recording that decision, FB11 and FB12 decided the same day ("Accept
both") and FB13, found in checking those answers, decided later that day;
then point 11, the two points ADR-0013 still marked Proposed, decided
with FB13 ("Accept all three"), with the cut-off rule the owner accepted
in the same answer.

1. **ADR-0005 (D1), "Shared directory".** Accept that `--appdata` naming a
   dcrwallet directory makes dcr-purse use that directory's TLS files,
   share its `peers.json` and keep its store beside a `wallet.db` it never
   opens, as the addendum of 2026-11-18 proposes, on the ground the owner
   gave on 2026-11-18 for the larger risk: only an explicit choice
   produces it. Recommended: accept as proposed; whether dcr-purse's
   `peers.json` stays readable by dcrwallet stays with ADR-0009 item 5,
   as the addendum says. **Decided (2026-11-22): accepted as proposed**
   (ADR-0005, addendum of 2026-11-22, item 1). Not built: no code reads a
   data directory yet.
2. **ADR-0005 (D1), "Title".** Retitle the record "Name, data directory
   and identity", keeping its file name. Recommended: accept; the import
   front end the title names is void. **Decided (2026-11-22): accepted.**
   The heading and the ADR index carry the new title; the file name
   `0005-name-data-directory-and-import-front-end.md` is kept (item 2 of
   the same addendum).
3. **ADR-0005 (D1), "Phase 8 exit", with ADR-0013 Open point 6's Phase 8
   bullet.** The brief's M3 sentence on the old `rpc.cert` is replaced by:
   an existing `dcrctl --wallet` configuration connects once its
   `rpccert` names dcr-purse's `rpc.cert`; nothing is carried over.
   Recommended: accept as proposed. It follows from ADR-0005's TLS bullet
   (no file is taken from dcrwallet's directory) and from the user
   documentation of ADR-0013 Open point 4, decided on 2026-11-19.
   Accepting it accepts the same bullet of Open point 6; that point's
   other bullets stay Proposed. **Decided (2026-11-22): accepted as
   proposed** (item 3 of the same addendum; ADR-0013 Open point 6, its
   Phase 8 bullet marked decided). The brief's Phase 8 exit now carries
   the sentence; Open point 6's other bullets stay Proposed. *(Decided
   as proposed later the same day, point 11.)*
4. **ADR-0003 (D2), the engine.** Confirm redb 5 as D2's engine, the
   confirmation the decision of 2026-11-14 made it subject to ("to be
   confirmed by a Phase 0 prototype measured against a large wallet's
   workload"). The owner's acceptance of 2026-11-19 covered ADR-0003's
   other Decision items, not this one. Recommended: confirm, by pointer
   in ADR-0003's Status and in brief §9 D2, on two sets of figures the
   owner has already accepted. The prototype's (ADR-0003, "Results" and
   "Performance", wallets of 10k, 100k and 1m transactions): balances
   670x to 465,000x faster at the depths the wallet asks for, header and
   filter ingest 2.3x to 3.0x faster with every commit durable; slower on
   opening for writing, on hot backup (4x to 8x), on recovery after a
   crash and in file size at 1m, each weighed there. And Phase 2's, of
   `purse-store` as built against dcrwallet's udb (ADR-0003, addendum of
   2026-11-20, and M1): 36,800x on a balance query and 1.86x on a range
   query at their lowest, and "no slower than Go on ADR-0003's workload"
   holding on every figure. Both sets are from a shared desktop; the
   dedicated-machine run (M2) is still owed, and could move the commit
   rows, which are bound by the machine's sync time, not the several-fold
   ones. **Decided (2026-11-22): redb 5 confirmed as D2's engine**
   (ADR-0003, Status line "Confirmed, 2026-11-22", and a dated note on
   Decision item 1; brief §9 D2 and the storage row of §8). M2 stays owed
   before Phase 3's exit. *(2026-11-24, project owner: M2 is to be made
   after the next transaction-store slice, and not before; ADR-0014,
   addendum of that date. Not made.)*
5. **ADR-0006 (D3), addendum of 2026-11-18, item 1, with ADR-0013 Open
   point 9.** As recorded: "keep the item due before Phase 3", and drop it
   only if ADR-0003's schema stores no script offset. Keeping it is
   already settled: rule 3's table (accepted 2026-11-18) has it due "at
   the latest Phase 3", and ADR-0003 Decision item 2 (accepted 2026-11-19)
   keeps each credit's script offset and length, "so dcroxide's
   `PkScriptLocs` stays needed". The recorded deadline has passed: Phase
   3's address manager slice started without it on 2026-11-20 (`94afd6e`,
   accepted by the owner), and the dcroxide pin `48f5b5e` does not carry
   it. Recommended: confirm item 1 and Open point 9 by pointer to those
   two accepted texts, and, as an explicit change rather than as
   proposed, make the item due before the part of Phase 3 that writes
   credits, `purse-udb`'s transaction store, the only code that calls it.
   That is the shape of the exception the owner made for the stake gate
   on 2026-11-20 (ADR-0006, addendum of that date, item 1). **Decided
   (2026-11-22): confirmed, with the deadline changed: `PkScriptLocs` is
   due before `purse-udb`'s transaction store, not before Phase 3**
   (ADR-0006, addendum of 2026-11-22, item 1, and a dated note in rule
   3's table; ADR-0013 Open point 9; brief §2.1 and §5). It is not yet
   written in dcroxide. The only Rust computations of the offsets today
   are test-local or spike code, both named `pk_script_locs`: one in
   `crates/purse-udb/tests/measure.rs`, which writes the transaction-store
   rows for ADR-0003's measurement until Phase 3's transaction store
   replaces it, and one in `spikes/store-redb/src/chain.rs`, the D2
   spike's, outside the workspace. No shipped crate has one.
6. **ADR-0006 (D3), addendum of 2026-11-18, item 2.** `purse-boltread` is
   not offered as part of M1 and not published while it is a test
   utility; the M1 offer is `purse-seed` from crates.io and
   `purse-hdkeychain` as a git dependency. Already settled: the owner
   decided on 2026-11-19 that it stays a test utility "marked not for
   release" (ADR-0013 Open point 2), and its manifest carries `publish =
   false`. Recommended: confirm by pointer to Open point 2, and give
   ADR-0006's Consequences line "Publishing", which still names
   `purse-boltread` in the M1 offer, a dated pointer to it. **Decided
   (2026-11-22): confirmed as proposed** (ADR-0006, addendum of
   2026-11-22, item 2; the "Publishing" line carries the dated pointer).
7. **ADR-0012 (D10), addendum of 2026-11-19, item 3.** A Rule 7 case whose
   difference a client could observe is brought as Rule 11 brings one:
   the proposed disposition in the QUIRKS entry, marked "pending owner",
   named in the change submitted for signing. Recommended: confirm as
   proposed. **Decided (2026-11-22): confirmed as proposed** (ADR-0012,
   addendum of 2026-11-22, item 1).
8. **ADR-0012 (D10), addendum of 2026-11-19, "Not settled", second
   point.** Whether brief §3 W1 itself should cover funds a defect leaves
   unavailable only until a restart or a manual unlock. The record has no
   proposal and names no phase. Recommended: W1's text stays as it is and
   the class is governed by Rule 7's default alone. W1 is the surface
   where a failure loses money; these funds are not lost, and Rule 7
   already fixes every such defect with a PARITY.md row and a test of
   Go's behaviour, so widening W1 would change no code. The owner may
   instead rule the question outside D10, since its answer amends the
   brief and not the policy; it is then listed as left, not as open in
   D10. **Decided (2026-11-22): W1's text stays as it is, and the class is
   governed by Rule 7's default alone** (ADR-0012, addendum of 2026-11-22,
   item 2; brief §3, last paragraph, and §9 D10). The alternative of
   ruling it outside D10 was not taken, so nothing of it is left.

9. **ADR-0003 (D2), ADR-0019's FB9 and FB5.** Not among the eight: it
   entered D2's record with `6fa3851`, after the check, and was found when
   the record was read again on 2026-11-22 (section 3, "ADR-0019, accepted
   after the check"). ADR-0003 leaves the meaning of `have_cfilters` for a
   store without the filters below the birthday to FB9, marked Proposed,
   and filters a client supplies below the birthday to FB5; no record
   defers either. The owner can settle it in one of two ways:
   - *(A)* decide FB9 and FB5 as ADR-0019 recommends: for FB9 option (a),
     the row meaning "no filter at or above the birthday height is
     missing", 1 in every store dcr-purse creates as in Go, with the
     udb-level `MissingCFiltersHeight` walking from the birthday height;
     for FB5, the sentence read as about what the wallet fetches and keeps
     of its own accord, and supplied filters written as dcrwallet writes
     them. Recorded in ADR-0019 and ADR-0003 by dated notes; the schema,
     the code and the doc comments change only with the code that
     implements ADR-0019, as ADR-0019's Consequences say.
   - *(B)* rule that FB9 and FB5 lie outside D2's question (the engine and
     the schema, whose layout ADR-0019 leaves unchanged) and defer them to
     a named event. The event cannot be the sync phases alone: FB9's
     options differ in what `purse-udb`'s port of `ExtendMainChain`,
     `MissingCFiltersHeight` and `InsertMissingCFilters` does, and FB5 in
     its port of `ImportCFiltersV2`, all methods of the Phase 3
     transaction store. The event would be: before `purse-udb`'s
     transaction store ports those methods.

   Recommended: (A). The analysis in ADR-0019 is complete, both answers
   keep the `ExtendMainChain` gate, the missing-filter pass,
   `InsertMissingCFilters` and `ImportCFiltersV2` ported unchanged apart
   from the walk's start, and the Phase 3 port needs the answers in
   any case; deciding them now leaves D2's record with nothing open but
   what it defers to Phase 3. With either answer, and no other decision,
   the third clause holds by section 3's test. The same answer can
   confirm or correct this record's two readings under that test: that
   ADR-0013 Open point 6's other bullets and Open point 8 concern the
   restore, and that ADR-0019's FB1 to FB4, FB6 to FB8 and FB10, which
   ADR-0003 names only in passing, leave nothing of D2 open, FB4 and FB10
   included (section 3, "ADR-0019, accepted after the check"). It can
   also decide FB4 as ADR-0019 recommends, which the Phase 3 port of
   `ExtendMainChain` needs before it is written, and FB10; the clause does
   not depend on either. **Decided (2026-11-22): (A).** The owner, shown
   FB1 to FB10 with their recommendations, answered "go with your
   suggestions for fb1-10": FB9 as option (a) and FB5 as recommended, and
   with them FB4, FB10 and the other six (ADR-0019, addendum of
   2026-11-22; ADR-0003, a dated pointer in its addendum of 2026-11-21;
   recorded in `ff59d84`). The statement above that either answer would
   let the third clause hold did not survive the recording: it found FB11
   and FB12, and checking their answers found FB13 (point 10). The answer
   did not rule on the reading that ADR-0013 Open point 6's other bullets
   and Open point 8 concern the restore rather than D1; that reading is
   put to the owner as point 11. The other reading, about FB1 to FB4, FB6
   to FB8 and FB10, no longer matters, since all are decided.

10. **ADR-0003 (D2), ADR-0019's FB11, FB12 and FB13.** Recording the
    answers to FB1 to FB10 found two more questions, and ADR-0003's
    addendum of 2026-11-21 named both Proposed: FB11, how the store records
    the filter of a block connected before its child is known while a
    birthday set from time is pending, and FB12, what the rules keyed on
    the birthday height do while a birthday is pending. On 2026-11-22 the
    owner, shown them in a table with their recommendations (FB11: option
    (i) with (r2); FB12: option (a)) and told that accepting them would let
    Phase 0 close, answered "Accept both". That is the whole of the
    owner's words. **Decided (2026-11-22): FB11 as (i) with (r2), FB12 as
    (a)** (ADR-0019, Status block, marks on FB11 and FB12 and addendum
    "2026-11-22 (later)"; ADR-0003, the same addendum, a dated pointer;
    ADR-0001, the pointer under its Status line). Checking the two answers
    found one case they do not cover (section 3, "ADR-0019, accepted after
    the check"), now ADR-0019's FB13, which ADR-0003 names Proposed: the
    filter of a block the store already holds without it, below the stored
    tip, that the first block a reorganisation attaches while a time
    birthday is pending shows to be the birthday block. No decided rule
    obtains or writes it, and without it `have_cfilters` would read 1 while
    a filter at or above the birthday is missing. The owner can settle it
    in one of two ways:
    - *(A)* decide FB13 as ADR-0019 recommends, its option (a): the syncer
      requests that block's filter with the attached blocks' filters,
      checks it against the stored header, and writes it through
      `put_cfilter_at` in the transaction that connects the resolving
      block, as FB11 (i) writes the filter of a block whose child was not
      known; one request dcrwallet does not make at that point, which tells
      the peer the birthday block, a disclosure of the kind FB1 accepted.
    - *(B)* rule that FB13 lies outside D2's question, as a rule of the
      syncers, the content of `cfilters` being fixed already by ADR-0019's
      decision item 1 and the meaning of `have_cfilters` by FB9 (a), and
      defer it to a named event. The event cannot be the sync phases
      alone: FB13's option (b) would change what `purse-udb`'s port of
      `MissingCFiltersHeight` does, a method of the Phase 3 transaction
      store. It would be: before `purse-udb`'s transaction store.

    Recommended: (A). It is the only option that keeps the rules already
    decided as they stand, adds no store operation, and leaves D2's record
    with nothing open but what it defers to Phase 3. Either answer leaves
    D2's record, as it stands, with nothing open that it does not defer;
    the third clause also needs point 11 answered. **Decided
    (2026-11-22): (A).** The owner, shown FB13 with the recommendation of
    option (a), that the syncer fetches the fork-point block's filter,
    checks it and writes it as part of the same chain switch, together
    with point 11 and a cut-off rule, answered "Accept all three". That is
    the whole of the owner's words. FB13 is decided as option (a)
    (ADR-0019, Status block, mark on FB13 and addendum "2026-11-22
    (FB13)"; ADR-0003, a dated pointer in its addendum of 2026-11-21;
    ADR-0001, the pointer under its Status line). Checking the answer
    before recording it found no further case (section 3, "ADR-0019,
    accepted after the check"). The cut-off rule, accepted in the same
    answer, in the owner's accepted words: "Questions that arise in
    working out ADR-0019 are settled in the phase that implements it and
    do not count against Phase 0's exit." It covers those questions and
    nothing else, and defers nothing at the close.

11. **ADR-0013 (D1, part), Open point 6's other bullets and Open point 8.**
    Not a new question: section 3 has listed both since the check, under
    this record's reading that they concern the restore, not D1's question
    (name and drop-in identity), and so count against no clause. The owner
    has not ruled on that reading (point 9). Read plainly, section 3's test
    counts them against D1: ADR-0013 is the record the ADR index lists
    against D1 (part) and D1's row cites for the part the import took away,
    both points are marked Proposed there, and no record defers them.
    Open point 6, apart from its Phase 8 bullet (decided on 2026-11-22,
    point 3), says how the tests that relied on an import reach matching
    state: in Phases 3 and 4, by running the same operation script through
    Go and Rust (ADR-0002) or restoring the same seed on both sides against
    the same scripted chain history, the logical exports compared first;
    in Phase 5, by restoring the same seed on both sides through both
    discovery variants at default, small and raised gap limits; Phase 6's
    M2 scenario as a restore from seed compared with Go restored from the
    same seed; Phase 7's golden captures from states the oracle
    operation-script runner writes under injected time and randomness;
    brief §7 Layer 6 as a Restore layer; and M1 as equal logical exports of
    wallets built by the same operation scripts. The brief carries each as
    its *Proposed* text (Phases 3, 4, 6 and 7, §7 Layer 6, the M1 and M2
    rows of §11). Open point 8 puts to the owner three facts ADR-0013's
    Context records as found after the decision: a restore does not recover
    the last-returned address counters as dcrwallet keeps them (spec 09
    §4.16.1 step 6.3), nor the birthday unless the user gives one (spec 21
    §5.4.1 step 2.4, §9 item 17), and account discovery runs only when the
    wallet is unlocked as the syncer is created (spec 09 §4.16.1 step 3.2).
    Once confirmed, the first two join Decision item 4's list and its
    copies (brief §1, `README.md`, `docs/PARITY.md`), and the third joins
    the user documentation of Open point 4. The owner can settle them in
    one of three ways:
    - *(A)* decide Open point 6's other bullets and Open point 8 as
      proposed. ADR-0013 marks them decided; the brief's *Proposed* markers
      on them are lifted; the two losses join Decision item 4's list and
      its copies, and the third fact the user documentation of Open point
      4, which is not yet written.
    - *(B)* defer them to named events: the bullet for Phases 3 and 4, and
      M1's, before `purse-udb`'s transaction store, the part of Phase 3
      whose exit corpus they shape; the Phase 5 and §7 Layer 6 bullets, and
      Open point 8, to Phase 5, which writes account and address discovery
      and the rescan; the Phase 6 M2 bullet to Phase 6; the Phase 7 bullet
      to Phase 7.
    - *(C)* confirm this record's reading: both lie outside D1. They stay
      Proposed in ADR-0013, with no event named, and are listed under "What
      is left".

    Recommended: (A). Each bullet of Open point 6 rests on what the owner
    has already accepted: the operation-script format (ADR-0002, accepted
    2026-11-19), on which `purse-udb`'s differential against Go already
    runs (`crates/purse-udb/tests/oracle.rs`, `generated_scripts_match_go`),
    and restore from seed, which W2 as decided on 2026-11-19 (ADR-0013 Open
    point 1) makes the migration contract. Open point 8's three facts are
    read from the specifications and only add to what the documentation
    tells a user a restore loses. Deciding them leaves ADR-0013 with no
    point marked Proposed; a bullet found wanting when its phase's tests
    are written can be amended by an addendum, as decided points have been
    (point 5). (B) keeps them open with an event each, for no gain the
    record can name; (C) leaves points that change the brief's exits with
    no event at which they must be decided. With point 10 answered, any of
    the three answers leaves no point of D1 to D10 that is neither decided
    nor deferred, on the records as they stand. Points 9 and 10 made the
    same statement, and recording point 9's answer found more; it is a
    reading of the records as they stand, not a promise. **Decided
    (2026-11-22): (A).** The owner, shown the two points with the
    recommendation to accept them as proposed, together with FB13 (point
    10) and the cut-off rule, answered "Accept all three". Open point 6's
    other bullets and Open point 8 are decided as proposed (ADR-0013,
    Status, the preamble of its Open points, marks on Open points 6 and 8,
    and dated notes on Decision item 4, on Context's "Found after the
    decision" and on Open point 4). The brief's *Proposed* markers on Open
    point 6's texts are lifted with dated notes: the Phase 3, 4 and 5
    exits, Phase 6's M2 exit, the Phase 7 golden-capture item, §7 Layers 3
    and 6, and the M1 and M2 rows of §11. Phase 5's exit and Layer 3 also
    carried the marker on the text of Open point 6 (its Phase 5 bullet, and
    its Phase 7 bullet, which names §7 Layer 3), though this point named
    neither. Each text was compared with ADR-0013's proposal (section 3,
    "Proposed in ADR-0013"). The first two facts of Open point 8 join
    ADR-0013 Decision item 4's list and its copies in brief §1, `README.md`
    and `docs/PARITY.md`; the third, that account discovery runs only when
    the wallet is unlocked as the syncer is created, is owed to the user
    documentation of ADR-0013 Open point 4, which is not yet written (Open
    point 5's documentation, also unwritten, already tells the user to
    unlock before the sync starts). Outside the brief, the texts that
    named Open point 6 Proposed carry the decision as dated notes, their
    marker kept: ADR-0002 (two places, "(Proposed; *decided as proposed,
    project owner, 2026-11-22*)"), ADR-0005 (a dated pointer on item 3 of
    its addendum of 2026-11-22) and `docs/specs/README.md` (the
    superseded passages of spec 23 §5.8, "decided as proposed, project
    owner, 2026-11-22; until then Proposed"); the ADR index's row for
    ADR-0013 records it. No other marker was changed. The reading this
    record gave, that the two points lie outside D1, is moot. ADR-0013
    reads the §7 Layer 6 bullet with ADR-0014's amendment 5 standing over
    its words "the requests the backend saw", a reading the owner has not
    ruled on (section 3, "Proposed in ADR-0013"). *(Later on 2026-11-22:
    it stands, project owner; see the dated note at the top.)*

**The re-check after point 11.** With points 10 and 11 recorded, the
records of D1 to D10 were read again for any point marked Proposed, open
or left to the owner (section 3, "The markers left, read again at the
close"). Every marker found is decided and marked, accepted with its
record, deferred by its record to a named phase or event, or not a pending
decision (a decided piece of work not yet built, a revisit trigger, or the
owner's standing discretion over delivering reports). None rests on the
cut-off rule. None counts against the third clause. Outside those
records, one case waits on the owner under ADR-0012's Rule 11, QK-0760's
residual case; it is a case under the decided policy, not a point of D10,
and is listed as left (section 3, "A case open for the owner under
ADR-0012", a reading the owner has not ruled on). *(Later on 2026-11-22
the owner deferred the case to Phase 5, as proposed, and ruled that this
record's readings stand; see the dated note at the top.)*

**Closed (2026-11-22).** Phase 0 is closed on the project owner's
instruction of 2026-11-21 to check D1 to D10 and close Phase 0, and the
owner's four answers of 2026-11-22: "Accept all eight" (the eight points
above), "go with your suggestions for fb1-10" (ADR-0019's FB1 to FB10,
point 9), "Accept both" (FB11 and FB12, point 10), and "Accept all three"
(FB13, point 10; ADR-0013's Open point 6's other bullets and Open point
8, point 11; and the cut-off rule). Every exit clause holds. CI is green
on three platforms: GitHub Actions runs 44892877145 on `69a6f6d`,
44989643104 on `6fa3851` and 44997525002 on `ff59d84`, all 14 jobs passed
in each, the `test` job on Linux (x86-64 and arm64), macOS and Windows
among them; no run exists yet for the commit carrying this record, which
changes documents only. The reader's dump equals Go's for every fixture
database in `wallet/udb/testdata` and for 12 generated databases (the
funded simnet wallet dropped, ADR-0013 Open point 2). D1 to D10 are
decided by section 3's test: D1, D2, D3, D5, D7 and D10 with points their
records defer to later phases or events, D6, D8 and D9 with nothing open,
and D4 superseded by ADR-0013; no point is left that is neither decided
nor deferred, and none rests on the cut-off rule, which covers questions
that arise in working out ADR-0019 and nothing else. What this record
lists as left stays listed as left, carried to the phases and events
named, QK-0760's residual case among it, pending owner under ADR-0012's
Rule 11 and not counted against D10. *(Later on 2026-11-22 the case was
deferred to Phase 5, as proposed, project owner; see the dated note at
the top.)*

**What is left**, none of it part of the exit sentence, carried past the
close:

- the acceptance releases of Decrediton, `dcrctl`, vspd and dcrd, whose
  rows in [PARITY.md](PARITY.md) are empty, due before the first
  milestone is accepted on a simnet scenario against real Go peers (brief
  §4 principle 8);
- the reading of Decrediton's wallet launcher and gRPC call inventory into
  specs 19 to 21, before Phase 8;
- the D5 spike's Decrediton half, which falls to Phase 12, whose exit runs
  Decrediton itself;
- a counterpart of dcroxide's `clippy.toml`, wanted when TLS enters the
  workspace (Phases 7 and 12);
- a fuzz soak: no target has run longer than CI's 60 s smoke;
- a reader comparison on a database with a real chain history behind it
  (section 2);
- the nightly job ADR-0003 asks for (item 9 of 2026-11-19: the large model
  run, and the cursor conformance test's seed sweep), which does not
  exist;
- `PkScriptLocs`, still to be written in `dcroxide-wire`, and the pin
  moved to it, before `purse-udb`'s transaction store; *(later on
  2026-11-22 the transaction store's first slice, which records no
  script offset and leaves `AddCredit` and every other credit step to a
  later slice, was written without it; whether a slice that writes no
  credit falls under this deadline is put to the owner with that
  change)* *(accepted by the project owner on signing that change, later
  on 2026-11-22: the deadline is restated as before the slice that ports
  `AddCredit` and `Rollback`'s credit step; ADR-0006, addendum
  "2026-11-22 (later)")* *(2026-11-23: written in dcroxide `1587ca1`,
  and the pin moved to it; ADR-0006, the dated note in rule 3's table)*
- what the records of D1, D2, D3, D5, D7 and D10 defer: D1 to Phases 7
  and 8 (the shape of the value under the `dcr-purse` key; (c),
  installing as `dcrwallet` for Decrediton); D2 to Phase 3 (open points 2
  and 7, the integrity check of item 6 of 2026-11-19, R4's oracle run for
  QK-0054, the dedicated-machine run M2 before the exit and the
  measurement M3 at it) *(2026-11-24: M2 after the next transaction-store
  slice, project owner; ADR-0014, addendum of that date)*; D3's open point 4 (dcroxide's CI building
  dcr-purse), before the transaction store *(not revisited before the
  transaction store's first slice, later on 2026-11-22; put to the owner
  with that change)*; D5 to Phase 12 (generating
  the messages without `protoc`, which of tonic's gRPC-layer differences
  Decrediton can observe, ADR-0014's Phase 12 speed criterion); D7 to
  Phase 10 (reusing `dcroxide-addrmgr`, conditional on that phase's
  differential); D10 to Phases 5 and 9 (QK-0203, and QK-0282's fee
  credits, each decided after an oracle run);
- D10's replacement of the base58 digit search with a constant-time
  lookup, decided and scheduled at low priority, not done, no phase named
  (ADR-0012, addendum of 2026-11-21) *(made later on 2026-11-22: the note
  of that date under the same addendum)*;
- QK-0760's residual case, pending owner under ADR-0012 Rules 7 and 11:
  whether account 0's legacy keys and rows are kept after the SLIP0044
  upgrade, so that a payment to a legacy address an earlier instance of
  the seed handed out is recognised and spendable; proposed for Phase 5,
  with discovery and the wallet's transaction filter, a deferral the owner
  has not accepted; until then dcr-purse's outcome in that case is Go's
  (section 3, "A case open for the owner under ADR-0012") *(deferred to
  Phase 5 as proposed later on 2026-11-22, project owner; see the dated
  note at the top)*;
- ADR-0019's rule, decided and not implemented: the syncers and the
  Phase 3 port of `ExtendMainChain`, whose check FB4 and FB11 set, and of
  `MissingCFiltersHeight`, whose start FB9 and FB12 set *(later on
  2026-11-22 the transaction store's first slice ports both, with FB11
  (i) and FB12 (a), and reads a `birth` row pending from both a height
  and a time as below the birthday only for a block below both, a
  reading put to the owner under the cut-off rule; the syncers, FB12 (a)
  item 2's genesis filter and the projection remain)* *(that reading
  accepted by the project owner on signing that change, later on
  2026-11-22)*; its questions
  FB1 to FB13 are decided; the projection of FB6 and FB12 and the range
  record are decided and not specified (ADR-0001); what the filter RPCs
  reply when a connected backend fails to serve a filter is left to the
  code (ADR-0019 rule 3); and any question its implementation raises is
  settled in the phase that implements it, by the cut-off rule;
- the shared directory of ADR-0005, decided and neither built nor tested
  (`purse-config`, Phase 8);
- the user documentation of ADR-0013 Open point 4 (decided 2026-11-19),
  not written: it is to list Decision item 4's losses, the two Open point
  8 added among them, and to say that account discovery runs only when
  the wallet is unlocked as the syncer is created (Open point 8), beside
  what Open point 5 asks of it.

**Checked for this record:** at `ff59d84` with this record's changes, on
an x86_64 Linux desktop, `PURSE_REQUIRE_ORACLE=1 cargo test --workspace
--locked --no-fail-fast` (108 test binaries, 633 passed, 0 failed, 31
ignored), `cargo fmt --all --check`, `cargo doc --workspace --no-deps`
with `-D warnings`, and `go test` for `tools/pinmove` pass. Not run for
it: CI on GitHub for the commit carrying this record, and so macOS and
Windows for it; the D5 spikes, which are outside CI; no timed run. The
cases of ADR-0019's FB11, FB12 and FB13 were read in the source at the
pin, not exercised.
