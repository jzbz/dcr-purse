# ADR-0016 — D12: Upstream tracking

- **Status:** **Accepted** as proposed (decision D12; project owner,
  2026-11-19), with the ten open points answered as recommended; see
  "Decision by the project owner" before "What was not tested". The
  repository facts, the history of dcrwallet's repository and the
  measurements under Context are not outcomes of it.
- **Date:** 2026-11-19

## Context

**The question.** When and how the parity pin moves: the dcrwallet commit
every parity claim names, and with it the dcrd modules dcrwallet links, the
releases of the peers and clients the acceptance scenarios run against, and
everything in this repository that is written against the pin. The brief
recommends following dcrwallet master until v2.2 is tagged, then release
tags, with "the dcrwallet and dcrd pins" moving together (brief §9 D12;
§4 item 7; §10 R9), and asks that the Decrediton, `dcrctl`, vspd and dcrd
versions be recorded next to the dcrwallet pin and move with it (brief §6
Phase 0, fourth bullet).

**Why now.** The pin was set once, at `54b85b33`, and has never moved.
Several accepted records already depend on a pin move happening in a
defined way, and none defines it:

- ADR-0009 item 2: the change that moves a pin redoes `docs/SKEW.md`; item
  6 reads D12's "dcrwallet and dcrd pins" as dcrwallet and the acceptance
  dcrd release, and makes dcroxide's revision a third pin, moved under
  ADR-0006 (`docs/adr/0009-version-skew-against-dcroxide.md:172-175`,
  `:215-222`).
- ADR-0006 rule 2: how the dcroxide revision moves, what that commit runs
  and who reviews it (`docs/adr/0006-consuming-dcroxide-crates.md:39`).
- ADR-0012 Rule 5 and Rule 12: every "not observable" trace and every
  QUIRKS entry is re-checked at each pin move
  (`docs/adr/0012-standing-quirk-policy.md:152-154`, `:225-230`; its
  Consequences add "every Rule 5 trace repeated in the pin review", `:258-261`).
- ADR-0001 repeats its audit at every pin move
  (`docs/adr/0001-logical-export-format.md:868`); ADR-0002 regenerates the
  runner's second copy of udb (`docs/adr/0002-operation-script-format.md:615`).
- Spec 23 §5.4.2 item 3: oracle versions move only when the pin moves
  (`docs/specs/23-upstream-tests-oracle-plan.md:1398`); spec 25 carries
  over dcroxide's rule that frozen batteries are regenerated, never
  re-expected by hand, when a move changes the package they came from
  (`docs/specs/25-reuse-dcr-rs-and-conventions.md:644`).

Phase 0 exits with D1 to D10 decided, not D12, but the first move will meet
all of the above at once, and the longer the pin stays, the larger that
move becomes (the measurements below).

**Precedent.** dcroxide decided the same question for dcrd in its ADR-0013
(accepted 2026-10-26): master until dcrd tags v2.2.0, moving at upstream
milestones and on its owner's word; from the tag, every release tag of the
newest line, with master read ahead into dated notes that are not parity
claims; one branch; each move one PARITY.md section titled by its range
that ports or accounts for every upstream commit in it, starting from a
tool's review list; the watch is the owner's (dcroxide
`docs/adr/0013-upstream-tracking-release-tags.md:121-176`). It also lists
what a tag pin breaks in tooling built for a master pin (`:190-259`).

### How dcrwallet moves

Read from a dcrwallet clone whose `origin/master` is `54b85b33`, the pin
(the clone was not fetched for this record, so later commits, if any, are
not counted).

- **Releases come with dcrd's.** `release-v2.0.0` is dated 2024-05-16, the
  day of dcrd's v2.0.0; `release-v2.1.0` 2025-11-25, a day after dcrd's
  v2.1.0 (dcroxide ADR-0013, Context). Point releases: v2.0.1 to v2.0.6 in
  nine months; v2.1.1 to v2.1.7 in eleven (2025-11-26, 12-08, 12-30,
  2026-04-07, 04-10, 08-18, 10-12). Each release tag is the same commit as
  a Go module version tag of `decred.org/dcrwallet/v5`: `release-v2.1.0`
  and `v5.0.0`, `release-v2.1.7` and `v5.1.2` (both commit `599b5850`).
- **Master moves slowly.** From the v2.1 branch point `4d526846`
  (2025-11-25) to the pin, master has 50 commits in 345 days, by month 7,
  8, 5, 3, 0, 11, 2, 0, 0, 6, 0, 6, 2. `release-v2.1` has 38 commits past
  the branch point, 37 of them carrying the `[release-v2.1]` prefix; 31
  master commits have a backport of the same subject.
- **The dcrd modules do not move on their own.** dcrwallet's `go.mod` at
  the pin and at `release-v2.1.7` require the same versions of every dcrd
  module; they differ only in `golang.org/x/mod`, `golang.org/x/tools` and
  a `tool` line. Neither has a `replace` directive (spec 23 §5.4.2 item 1).
  The dcrd modules dcrwallet links are therefore not a separate pin: they
  follow from dcrwallet's `go.mod`, which `tools/oracle/dcrwallet.go.mod`
  copies and `docs/SKEW.md` records.
- **Upstream fixes defects of the kind QUIRKS.md lists.** Among master's
  last commits before the pin: "Disallow negative txn idx for
  createsignature RPC" (`54b85b33`), "wallet: Validate header heights
  against parents" (`abb367b4`), "mixing: add missing bounds check"
  (`520c9890`), "spv: Validate merkle root for all fetched blocks"
  (`2c37f240`), "spv: Disconnect peer falsely spending owned UTXOs"
  (`ffc81a46`), "Validate tx signature before adding to wallet"
  (`9d99335a`), "jsonrpc: Avoid nil pointer dereference" (`43b095a5`). A
  pin that never moves keeps reproducing defects upstream has fixed
  (ADR-0017 lists the open ones).
- **The acceptance dcrd.** dcrwallet requires dcrd JSON-RPC API 8.3.0
  (`chain/sync.go:33`). dcrd `release-v2.1.7` and dcrd `46e9df48`, the
  commit dcroxide pins, both advertise 8.3.0 (`internal/rpcserver/
  rpcserver.go`, `jsonrpcSemverMajor`/`Minor`/`Patch`, read at each).

### What names the pin in this repository

- **Code and the oracle.** `DCRWALLET_PARITY_COMMIT`
  (`crates/purse-testutil/src/lib.rs:39`); the oracle's requirement
  (`tools/oracle/go.mod:11`, `go.sum:3-4`); the verbatim copy of
  dcrwallet's `go.mod` (`tools/oracle/dcrwallet.go.mod`), whose hash a Go
  test holds to `go.sum` (`tools/oracle/internal/deps/gomod_test.go:80-90`);
  the skew checks (`crates/purse-testutil/tests/skew.rs`), which hold
  `docs/SKEW.md`'s header to the pins.
- **The oracle's copies of upstream source.** The operation-script runner
  links copies of dcrwallet's `wallet/udb` (every non-test file),
  `wallet/internal/snacl`, `internal/compat` and `kdf`, with the listed
  rewrites that inject randomness, clock and processor count
  (`tools/oracle/cmd/oprunner/internal/README.md`); the snacl oracle links
  a verbatim copy of `wallet/internal/snacl/snacl.go`
  (`tools/oracle/internal/snaclcopy/README.md`). Go tests hold each copy to
  the upstream file of the version `go.mod` pins (`copies_test.go`,
  `verbatim_test.go`), so after a `go.mod` bump the copies still compile
  but those tests fail until the copies are regenerated
  (`PURSE_REGEN_COPIES=1 go test ./cmd/oprunner -run
  TestCopiesAreVerbatim`; `snacl.go` replaced by hand). The header of
  `copies_test.go` lists a hand audit of the unmodified dependencies the
  copies call, which a move repeats; ADR-0002 requires the same
  (`docs/adr/0002-operation-script-format.md:615-618`). These run only
  under `go test` in `tools/oracle`, not in the cargo workspace.
- **Frozen files.** The 71 files under `crates/*/tests/data` fall into
  three groups by how they depend on the pin:

  | Group | Files | What ties them to the pin |
  |---|---|---|
  | Go-generated, pin checked by a test | `purse-errors` (1), `purse-seed` (1), `purse-snacl` (3 of 4: the vector files and `generated_db.txt`), `purse-mixsolver` `flint_vectors.jsonl` | the frozen test requires the header to name `DCRWALLET_PARITY_COMMIT` (`crates/purse-errors/tests/oracle.rs:656-668`, a plain test there; `crates/purse-seed/tests/frozen.rs:44`; `crates/purse-snacl/tests/frozen.rs:24-35`; `crates/purse-mixsolver/tests/frozen.rs:50-54`) |
  | Go-generated or copied at the pin, pin not checked | `purse-hdkeychain`'s five vector files (the regenerator writes "as linked by dcrwallet 54b85b336077" into their headers, `crates/purse-hdkeychain/tests/frozen.rs:205-223`, but the frozen tests read them through `verify_integrity`, `tests/common/mod.rs:414-430`, which checks the entry count and body digest only); `purse-dcrutil`'s three vector files (they name "dcrwallet 54b85b336077" in prose; `tests/frozen.rs` checks counts and answers only); `purse-export-check`'s `full-simnet.pe` and its six `opvectors/` pairs (no commit in their headers; `.opv` says "at the parity pin"); `purse-snacl`'s `generated-simnet.db.gz`; `purse-boltread`'s `upstream/*.db.gz` (copied from `wallet/udb/testdata` at the pin, named only in a README) | nothing: a move that forgets them passes every test |
  | Not tied to the dcrwallet commit | `purse-mixsolver`'s `self_vectors.txt` ("SELF-GENERATED", computed by the crate, not by Go); `purse-boltread`'s `frozen-*.db`, digests and stats (bbolt v1.3.12) and `purse-dcrutil`'s `txsort/*.hex` (dcrd `dcrutil/v4` v4.0.3), which move only when dcrwallet's `go.mod` requirement moves; `purse-export-check`'s `opscripts/*.json` (inputs written by hand) | a `go.mod` requirement, or nothing |

  Their regenerators: ignored tests named `regenerate_frozen_vectors` in
  `purse-errors`, `purse-seed`, `purse-snacl`, `purse-dcrutil` (each in
  `tests/oracle.rs`) and `purse-hdkeychain` (`tests/frozen.rs:302`),
  `regenerate_flint_vectors` in `purse-mixsolver`; and two that are gated
  by an environment variable instead of `#[ignore]`:
  `regenerate_frozen_export` (`PURSE_REGEN_EXPORT=1`,
  `crates/purse-export-check/tests/oracle.rs:333-336`) and
  `frozen_vectors_match_go` with `PURSE_REGEN_OPVECTORS=1`
  (`crates/purse-export-check/tests/opscripts.rs:29`, `:695`).
  Spec 25 requires a battery's kept exporter, before it is run at a new
  pin, to reproduce the frozen file byte for byte at the commit the file
  was dumped at (`docs/specs/25-reuse-dcr-rs-and-conventions.md:644`).
  `full-simnet.pe` cannot meet that literally, nor a header-only check
  after regeneration: its header says "Volatile values differ on every
  regeneration", so both comparisons have to mask those values, which no
  test defines yet. A move today therefore rewrites the four checked
  batteries' headers whatever changed, and nothing makes it look at the
  second group.
- **Text.** `README.md:8`, the brief (line 5, §2's table, §4 item 7),
  `docs/PARITY.md:3`, `docs/specs/README.md:3`, each spec's header,
  `docs/QUIRKS.md:3`, `docs/SKEW.md:5`, and the ADRs that name it:
  ADR-0001, 0007, 0009 and 0014 to 0018. ADR-0002 to 0006, 0008 and 0010
  to 0013 cite dcrwallet as `path:line` and name no commit (no
  `54b85b33`, "parity pin" or `DCRWALLET_PARITY_COMMIT` in them); they
  were all written at `54b85b33`, the only pin there has been.
- **Citations.** The specs, QUIRKS.md and the ADRs cite dcrwallet as
  `path:line` at the pin. A prototype (below) finds 7,929 citations of
  that form into files of dcrwallet's own module across `docs/`. Citations
  into dependencies are written `module@version/path:line` and move only
  when a `go.mod` requirement moves.

No document says where the acceptance releases (Decrediton, `dcrctl`,
vspd, dcrd) are recorded; spec 23 leaves the vspd release open
(`docs/specs/23-upstream-tests-oracle-plan.md:2740`). No CI job is
scheduled (`.github/workflows/ci.yml` has no `schedule` trigger).

### Measured: what a move does to the citations

A prototype (`pinmove.py`, kept in the scratch directory of this work,
not in the repository) reads a dcrwallet clone with `git show` and
`git diff` only. For each citation `path.go:N` into dcrwallet's module,
and the `:N` forms that follow it in the same citation, it maps line N
from the old revision to the new one with a line diff and classifies it:
*same* (unchanged, same number), *shifted* (unchanged, new number),
*touched* (a human must re-read it) or *gone* (the file does not exist
at the new revision). A citation of one line is touched when that line
changed; a range `N-M` is touched when any line inside it changed or when
it is no longer contiguous at the new revision (lines inserted inside it
leave every old line mapped but break adjacency). A range is therefore
untouched only if every line in it is unchanged and each line's
successor still follows it at the new revision. It also lists, per document, the commits in the range that touch the
files the document cites. Three moves were simulated from the pin
backwards over real history; a forward move of the same range changes the
same lines. The counts are of the documents as they stand with this
record.

| Range simulated | Commits | Files changed (of them Go) | Same | Shifted | Touched | Gone | Documents to review |
|---|---|---|---|---|---|---|---|
| `54b85b33` and `c0fee6b5` (2026-08-18) | 8 | 9 (5) | 7,614 | 295 | 20 | 0 | 28 |
| `54b85b33` and `release-v2.1.7` | 88 (50 + 38) | 20 (18) | 6,504 | 1,343 | 77 | 5 | 42 |
| `54b85b33` and `4d526846` (v2.1 branch point) | 50 | 32 (28) | 5,544 | 2,123 | 257 | 5 | 44 |

Each run took under a second on an x86_64 Linux desktop. For the 8-commit
range, QUIRKS.md alone has 116 shifted citations and 2 touched; the
spec with the most shifted is spec 13 (39, 5 touched). A first
version of the prototype checked only the two ends of a range and
undercounted touched citations about tenfold for the 8-commit range (2
against 20) and threefold for the others; the rule above is the one
`tools/pinmove` is to implement (Decision item 6). dcroxide saw the same
effect: its one-commit `3360ae4d` move shifted 62 citations of one file by
four lines (dcroxide ADR-0013, Context).

What the table says: a small move is cheap in citations if the remapping
is mechanical, and a whole cycle is not, because of the touched ones,
which need reading; and a hand remap of thousands of shifted citations is
neither cheap nor reliable. The prototype classifies; it does not rewrite.

## Options considered

**Before v2.2.0 is tagged.**

- **A. Freeze at `54b85b33` until the tag.** One move, at the tag, of
  every master commit since the pin plus the release. Cheapest until then;
  the specs, QUIRKS and the oracle keep describing defects upstream has
  fixed, and the one move is a whole cycle (the last row above, or larger).
- **B. Follow master closely** (every upstream commit, or weekly). Small
  moves, each with the full checklist below; about four commits a month
  at master's recent pace, so the checklist would run for one or two
  commits at a time.
- **C. Master at upstream milestones and on the owner's word** (the
  brief's recommendation, dcroxide's shape until its tag). Moves at a
  dcrwallet point release (whose backports come from master, so master
  then carries released fixes), at an upstream fix of a defect dcr-purse
  tracks, or when a phase about to start needs an upstream change.
- **D. Pin `release-v2.1.7` now and follow v2.1 point releases until
  v2.2.0.** Parity with what users run today. It contradicts the parity
  target the owner set (dcrwallet v2.2, brief line 5); it also gives up 19
  master-only commits the specs already describe.

**After the tag.**

- **(i) Every release tag of the newest line**, with master read ahead
  (dcroxide ADR-0013 items 1 to 3).
- **(ii) Only `.0` tags.** Fewer moves; point releases carry the fixes
  users run, and the brief's "then release tags" covers them.
- **(iii) Also point releases of an older line** after a newer `.0` tag.
  Two lines of claims; rejected for the reasons dcroxide gives (its
  ADR-0013, Alternatives).

**Who and how.** The move as one reviewed change made by an engineer and
signed by the owner (ADR-0006 rule 2's shape), or a standing CI job that
moves the pin automatically. The latter cannot port upstream changes into
crates already pinned, write a PARITY.md range section or re-read touched
citations, so it can only report.

## Decision (accepted, 2026-11-19)

1. **Until dcrwallet tags v2.2.0: master, option C.** The pin stays where
   it is until one of these, and then moves to master's head:
   (a) dcrwallet tags a point release on its newest line;
   (b) master fixes a defect on ADR-0017's list, or one that is
   security-relevant in ADR-0017's sense;
   (c) a phase is about to start whose specs master has changed;
   (d) the owner's word.
   (a) and (b) are moves the owner times; (c) is raised by the engineer
   starting the phase. Nothing moves on a calendar.
2. **From v2.2.0: every release tag of dcrwallet's newest line**
   (`release-v2.2.0`, `release-v2.2.N`, then the next `.0`), option (i),
   at the owner's timing, and promptly for a release that fixes a
   security-relevant defect or that dcrwallet or dcrd call mandatory.
   "dcrwallet at the pin" is the release tag's commit, which is the commit
   of the module version tag the oracle then requires (`v5.1.2` for
   `release-v2.1.7` today; a `/v6` module path, if v2.2 brings one, changes
   the require line, not the rule), plus every dependency at the version
   that commit's `go.mod` requires. A fix on master that no release carries
   is not followed (dcroxide ADR-0013 item 8), except as ADR-0017 item 6
   provides.
3. **Master is read ahead, not claimed.** At each point release, and at
   least monthly while there is master activity past the pin, the tool of
   item 6 reports master's commits past the pin, the per-spec review list
   and the citations they would touch, into a dated note in
   `docs/UPSTREAM.md`. A note is not a parity claim and says so; it is
   what the owner decides a move from.
4. **The other pins.**
   - *dcrd modules:* no pin of their own; they follow dcrwallet's
     `go.mod`. A move that changes a requirement redoes SKEW.md under
     ADR-0009 item 2, and if dcrwallet now requires a dcrd module newer
     than dcroxide's pin carries, ADR-0009 item 6 applies.
   - *Acceptance releases* (dcrd and the `dcrctl` it ships with, vspd,
     Decrediton): recorded in a table at the head of `docs/PARITY.md`
     beside the dcrwallet pin. Rule: the newest release of each at the
     time of the move. Before v2.2 that is dcrd `release-v2.1.7` today
     (JSON-RPC API 8.3.0, which dcrwallet requires); from v2.2 the
     releases of the same cycle. dcroxide-backed scenarios use dcroxide at
     its pinned revision, whose dcrd pin is then a second dcrd in the
     matrix (brief §7, Layer 7). The table is filled when Phase 0's
     acceptance work picks the releases, not by this record.
   - *dcroxide revision:* not tied to the dcrwallet pin; ADR-0006 rule 2.
   - *Reference Go:* not tied; it moves by its own deliberate bump
     (`crates/purse-testutil/src/lib.rs:41-46`; ADR-0009).
5. **The move.** One change on master, containing nothing but the move
   and what the move requires, made by an engineer and signed by the
   owner. In order:
   1. Fetch dcrwallet; run the tool of item 6 from the pin to the target.
      Its output is the review list: the commits, the files, the
      citations by class, the QUIRKS entries citing changed files, the
      frozen batteries whose generating packages changed (the packages
      each oracle command links, from `go list -deps`, against the changed
      packages), and the `go.mod` delta.
   2. Before anything is bumped, run every kept exporter at the old pin
      and require it to reproduce its frozen file byte for byte (spec 25,
      `docs/specs/25-reuse-dcr-rs-and-conventions.md:644`; `full-simnet.pe`
      with its volatile values masked, once a test defines the mask). An
      exporter that does not reproduce its own file is fixed first.
   3. Bump `DCRWALLET_PARITY_COMMIT`, the oracle's requirement and
      `go.sum` (`go get decred.org/dcrwallet/v5@<commit>`), and
      `tools/oracle/dcrwallet.go.mod`.
   4. Regenerate the oracle's copies of upstream source: the oprunner
      copies with `PURSE_REGEN_COPIES=1 go test ./cmd/oprunner -run
      TestCopiesAreVerbatim`, reviewing the diff and every rewrite count
      the test reports as changed (a new clock read or randomness draw
      upstream); `internal/snaclcopy/snacl.go` replaced by the new
      upstream file with its package clause changed. Repeat the hand audit
      of the unmodified dependencies listed in `copies_test.go`'s header,
      and run `go test ./...` in `tools/oracle`, which the cargo workspace
      does not run.
   5. Run the workspace with `PURSE_REQUIRE_ORACLE=1`. Every live
      differential now runs against the new Go. A failure is an upstream
      change in a crate already ported: it is ported in this change, or
      the move waits.
   6. Regenerate every Go-generated frozen file of the first two groups
      under Context, "Frozen files", with its regenerator (the ignored
      `regenerate_*` tests, `PURSE_REGEN_EXPORT=1` and
      `PURSE_REGEN_OPVECTORS=1`), and re-copy the files copied from the
      pin (`purse-boltread`'s `upstream/`). A file whose package the
      review list did not flag must differ in its header only (and
      `full-simnet.pe` in its masked volatile values); any other difference
      stops the move until it is explained. Files of the third group move
      only with the `go.mod` requirement they depend on; `self_vectors.txt`
      never moves with the pin.
   7. Redo `docs/SKEW.md` if a requirement changed; otherwise update its
      header and add a dated line saying no requirement changed.
   8. Citations: apply the mechanical remap to every *shifted* citation
      in the specs, QUIRKS.md, PARITY.md, SKEW.md, the brief and the
      README; re-read every *touched* one and correct the text it
      supports; re-locate every *gone* one. ADRs are dated records and are
      not remapped: their citations stay at the pin in force on their date
      (an addendum's at the pin on the addendum's date). Ten ADRs name no
      commit, so the rule is written once, in `docs/adr/README.md`, with
      the pins and the dates each was in force; the move adds its line
      there.
   9. Each spec on the per-spec review list gets a dated "Pin move" note:
      the commits read and what changed in the behaviour it describes, or
      that nothing did.
   10. QUIRKS.md: every entry citing a changed file is re-checked (ADR-0012
      Rule 12), Rule 5 traces over changed files are repeated, and an entry
      upstream fixed gets a dated line naming the fixing commit; its
      disposition then follows ADR-0017 item 6.
   11. `docs/PARITY.md`: one section per move, titled by its range
      (`54b85b33..<new> (N commits)`), that ports or accounts for every
      upstream commit in it (dcroxide ADR-0013 item 6); the pins table.
   12. The text that names the pin (README, brief, PARITY.md, the specs'
       README and headers, QUIRKS.md's header, the pins line of
       `docs/adr/README.md`).
   13. Re-run the acceptance scenarios of the milestones reached, as
       ADR-0006 rule 2(d) does for the dcroxide revision.
   The owner reviews the range section and the touched citations, and
   signs.
6. **Tooling, built before the first move.** `tools/pinmove`, a Go
   command beside `tools/oracle` (Go is already a test-only language
   here), with three modes: `report` (items 3 and 5.1), `cite` (rewrites
   shifted citations in the documents item 5.8 names and lists the rest;
   never edits `docs/adr/`), and `batteries` (which regenerators the
   changed packages require, by package for every Go-generated file,
   whether or not the file names the pin). `cite` classifies a range as
   touched when any line inside it changed or the range is no longer
   contiguous at the new revision, never by its two ends alone (Context,
   "Measured"). Its own tests run on fixed revisions of a small Go
   repository built in the test, including a change inside a cited range
   and an insertion inside one. A `purse-testutil` check extends the skew
   test to every place that names the pin, so a move that misses one
   fails; it cannot see files that do not name the pin, which is what Open
   point 8 is about.
7. **Work in flight.** A move lands alone, never mixed with feature work.
   A branch in progress at the old pin lands before the move or is
   rebased after it and re-runs its differentials against the new oracle;
   nothing lands at an old pin after a move. A trigger (a) or (c) may be
   deferred by the owner while a phase is porting a package the move
   changes; a trigger (b) is not deferred past the end of the phase in
   progress.

## Consequences

- **Cost per move**, from the measurements: for a move of about eight
  commits, some 300 mechanical remaps, 20 touched citations to re-read,
  a review list across about 28 documents, the regeneration of the
  oracle's copies and of every Go-generated frozen file (the four checked
  batteries at least in their headers, and the second group's files,
  which nothing checks today), and the PARITY.md range section. For a
  whole cycle, about 2,100 remaps and 257 touched. Option C keeps moves
  near the first size; option A makes the first move the second size.
- **Reproduced defects that upstream fixes leave the specification** at
  the next move, instead of staying in the port until the tag.
- **Parity claims stop naming unreleased code** at v2.2.0; before it,
  they name master, as the owner's target requires.
- **Release-line-only defects come with the pin** after the tag, and are
  reproduced or recorded (dcroxide ADR-0013, Consequences). Backports are
  seen twice: from the release line at a point release, and as their
  master originals at the next `.0` move, matched by subject.
- **Work created:** `tools/pinmove` and its tests; the pin check in
  `purse-testutil`; `docs/UPSTREAM.md`; the pins table in PARITY.md.
- **Work avoided:** a standing job that moves pins; a second line of
  claims; tracking dcrwallet's database version (ADR-0013 removed it).
- **Constrains:** ADR-0009 (each move feeds its ledger; its item 6's
  reading of D12 is confirmed here); ADR-0012 (Rules 5 and 12 run inside
  item 5); ADR-0017 (its list is re-checked at each move); the four
  frozen batteries whose tests require the current pin in their headers,
  which every move therefore touches; `docs/adr/README.md`, which gains
  the line of pins and dates.
- **Revisit when:** dcrwallet tags v2.2.0 (the first tag move needs the
  module-version form of the oracle's requirement and the release tag in
  the skew check's header); dcrwallet's cadence changes markedly; a move
  needs more than a day of review, which would argue for smaller moves.

## Open points

Each for the owner, with a recommendation. All ten were decided as
recommended (project owner, 2026-11-19); each is marked.

1. *Cadence before the tag: option C with triggers (a) to (d)?*
   **Decided as recommended (project owner, 2026-11-19).** Recommended: yes. Alternatives: A (freeze until the tag), cheaper now
   and one large move later; B (follow master closely), smallest moves and
   the most checklist runs.
2. *After the tag, every point release of the newest line?* **Decided as recommended (project owner, 2026-11-19).**
   Recommended:
   yes (option (i)); a security or mandatory release promptly.
3. *Who watches?* **Decided as recommended (project owner, 2026-11-19).** Recommended: the owner, as for dcroxide; the engineer
   starting a phase raises trigger (c). Plus a monthly scheduled CI job
   that runs `tools/pinmove report` against dcrwallet's master and newest
   tag and publishes the report as a job artifact, never failing the
   build. Alternative: no scheduled job; the report runs at each point
   release by hand.
4. *Rewrite shifted citations mechanically?* **Decided as recommended (project owner, 2026-11-19).**
   Recommended: yes, in the
   specs, QUIRKS.md, PARITY.md, SKEW.md, the brief and the README, with
   the touched ones listed for a human; never in ADRs, whose citations
   stay at the pin in force on their date, recorded once in
   `docs/adr/README.md` because ten ADRs name no commit. Alternatives:
   leave citations at the pin they were written at and mark each spec
   with that pin, which saves the rewrite and leaves the specs naming
   several pins; for the ADRs, a dated addendum naming `54b85b33` in each
   of the ten, which says the same at ten times the length.
5. *Where the read-ahead notes live.* **Decided as recommended (project owner, 2026-11-19).**
   Recommended: `docs/UPSTREAM.md`,
   dated sections, separate from PARITY.md's range sections, which record
   what the port claims.
6. *Where the acceptance releases are recorded, and how chosen.*
   **Decided as recommended (project owner, 2026-11-19).** Recommended: a table at the head of PARITY.md; the newest release of
   each at the time of the move; filled by Phase 0's acceptance work.
7. *Tool language.* **Decided as recommended (project owner, 2026-11-19).** Recommended: Go, under `tools/pinmove`, beside the
   oracle. Alternatives: a Rust `xtask`, or Python, which the repository
   does not use today.
8. *Frozen files and the pin.* **Decided as recommended (project owner, 2026-11-19).** Four batteries' tests require the
   current pin in their headers, so every move rewrites them; the second
   group (`purse-hdkeychain`'s vectors, `purse-dcrutil`'s vectors, `purse-export-check`'s frozen export
   and opvectors, `purse-snacl`'s generated database, `purse-boltread`'s
   upstream copies) is Go-generated or copied at the pin but nothing
   checks which pin, so a move that forgets them passes; the third group
   is not tied to the commit. Recommended: make the rule uniform, as a
   separate change before the first move: every Go-generated or copied
   file names the dcrwallet commit in its header (or, for binary files,
   in a manifest beside them), and a frozen test per crate requires it to
   equal `DCRWALLET_PARITY_COMMIT`; then keep regenerating all of them at
   each move with the header-only check of item 5.6. Alternative: record
   the commit each file was generated at and the pin it was last checked
   at separately, so an unchanged file need not be rewritten; this needs
   a change to every frozen test, and `tools/pinmove batteries` would
   then decide by package which files a move must regenerate.
9. *Specs of phases not started.* **Decided as recommended (project owner, 2026-11-19).**
   Recommended: they are remapped and
   their touched citations re-read at every move, not deferred until
   their phase, so that no spec drifts from the pin it names.
10. *Deferring a security fix.* **Decided as recommended (project owner, 2026-11-19).**
    Recommended: trigger (b) is never
    deferred past the end of the phase in progress (item 7).

## Decision by the project owner (2026-11-19)

Accepted as proposed, every open point as recommended. The answers:

1. *Before v2.2.0:* option C; the pin moves to master's head on triggers
   (a) to (d) of Decision item 1, never on a calendar.
2. *From v2.2.0:* every release tag of dcrwallet's newest line (option
   (i)); a security or mandatory release promptly.
3. *Who watches:* the owner; the engineer starting a phase raises trigger
   (c); a monthly scheduled CI job runs `tools/pinmove report` against
   dcrwallet's master and newest tag and publishes the report as a job
   artifact, never failing the build.
4. *Citations:* shifted ones rewritten mechanically in the specs,
   QUIRKS.md, PARITY.md, SKEW.md, the brief and the README, touched ones
   listed for a human; never in ADRs, whose citations stay at the pin in
   force on their date, recorded once in `docs/adr/README.md` (already
   there: its second paragraph).
5. *Read-ahead notes:* `docs/UPSTREAM.md`, dated sections.
6. *Acceptance releases:* a table at the head of PARITY.md, the newest
   release of each at the time of the move, filled by Phase 0's acceptance
   work.
7. *Tool language:* Go, under `tools/pinmove`.
8. *Frozen files:* the uniform rule, as a separate change before the first
   move: every Go-generated or copied file names the dcrwallet commit in
   its header or a manifest beside it, and a frozen test per crate
   requires it to equal `DCRWALLET_PARITY_COMMIT`; all of them are
   regenerated at each move, with the header-only check of item 5.6.
9. *Specs of phases not started:* remapped and re-read at every move.
10. *A security fix:* trigger (b) is never deferred past the end of the
    phase in progress.

ADR-0017's list, which trigger (b) and item 5.10 refer to, is accepted
with its reporting route changed (the owner delivers reports privately;
ADR-0017, "Decision by the project owner"); nothing in this record
depends on how a defect reaches upstream. Nothing of "Work created" is
built by the acceptance: `tools/pinmove` and its tests, the pin check in
`purse-testutil`, `docs/UPSTREAM.md`, the pins table in PARITY.md, the
frozen-file rule of answer 8 and the scheduled job of answer 3 are each a
later change, all before the first move. *Done (2026-11-20):
`tools/pinmove` and its tests, `docs/UPSTREAM.md`, the pins table in
PARITY.md and the scheduled job (`.github/workflows/upstream.yml`); the
pin check in `purse-testutil` and the frozen-file rule of answer 8 are
still to come.*

## What was not tested

- The prototype classifies citations and lists commits; it rewrites
  nothing. Its pattern misses citations whose `:N` items are separated
  from their path by other text, and it ignores citations into
  dependencies. Its counts are of `path.go:N` and `path.proto:N` forms
  only.
- The simulated moves run backwards over past history; no forward move
  was made, and nothing about dcrwallet's future cadence is known.
- The dcrwallet clone was read as it stood; it was not fetched, so the
  record does not know whether master has moved past `54b85b33`.
- Which packages each oracle command links was not computed per battery;
  item 5.1 describes the computation, not a result.
- The procedure's steps for the oracle's copies (item 5.4) and the
  reproduce-first check (item 5.2) are taken from the copies' READMEs and
  spec 25; neither was run for a move.
- Decrediton's, vspd's and `dcrctl`'s release cadences were not examined.
