# Upstream read-ahead notes

Dated notes on what dcrwallet has done past the parity pin, newest first
(ADR-0016, Decision item 3 and answer 5). **A note is not a parity claim.**
It records what `tools/pinmove` showed on its date, so that the owner can
decide whether and when the pin moves; what the port claims is in
[PARITY.md](PARITY.md), whose range sections are written by the moves
themselves.

A note is added at each dcrwallet point release and at least monthly while
master has commits past the pin. Its source is the `pinmove-report`
artifact of the monthly workflow `.github/workflows/upstream.yml`, or a
local run:

```sh
cd tools/pinmove
go run . report    -upstream ../../../dcrwallet -to origin/master -o report.md
go run . report    -upstream ../../../dcrwallet -to release-vX.Y.Z -o report-tag.md
go run . batteries -upstream ../../../dcrwallet -to origin/master
```

Each note gives the revisions read, the commits past the pin with what
they change in the behaviour the specs describe, the citations by class,
the frozen files and oracle copies a move would regenerate, the `go.mod`
delta, any fix of a defect on ADR-0017's list or of a security-relevant
one (trigger (b) of ADR-0016 item 1), and whether a trigger has fired.
`cite` never rewrites this file: a note's file names and numbers stay as
they were on its date.

## 2026-11-19: no commit past the pin; first dry runs of `tools/pinmove`

**Revisions.** The dcrwallet clone was fetched over HTTPS on 2026-11-19.
Master's head is `54b85b33` (2026-11-05), the parity pin itself: master
has no commit past the pin, so no trigger of ADR-0016 item 1 comes from
master. The newest release tag is `release-v2.1.7` (`599b5850`,
2026-10-12, module version `v5.1.2`).

**`release-v2.1.7` against the pin.** The release line has 38 commits past
the v2.1 branch point `4d526846`. 31 of them have a commit of the same
subject (the `[release-v2.1]` prefix stripped) on master before the pin;
the other 7 are the version bumps to 2.1.1 through 2.1.7. By subject,
then, the pin carries every change `release-v2.1.7` carries but its
version string, and trigger (a) for that release asks for nothing the pin
lacks. The pairing is by
subject only; the diffs of the pairs were not compared. Master has 19
commits past the branch point with no counterpart on the release line,
among them "Disallow negative txn idx for createsignature RPC"
(`54b85b33`), "wallet: Validate header heights against parents"
(`abb367b4`), "wallet: Check vsp treasury policy correctly" (`870ec621`),
"jsonrpc: Avoid nil pointer dereference" (`43b095a5`), "Ensure VSP fee
payment errors are logged" (`e23864fc`), "Consistent preconditions for
SPV/RPC sync" (`a9b2aa45`), "Update wire to fix a potential
use-after-free" (`bb6e8b41`) and three changes to jsonrpc error handling
and messages (`e1381881`, `d4bb0dc0`, `2998d7e2`); the rest are comments,
tests, refactors, logger configuration, tool configuration and the
development version bump.

**Dry runs.** With no commit past the pin, three moves were simulated
backwards from it over real history, as ADR-0016's measurements were: the
documents cite the pin, and a forward move over the same range changes the
same lines. Each `report` ran on the repository as of this note (the
runs counted here are of 2026-11-20, against the clone fetched the day
before); `cite -w` ran on a scratch copy of the documents and the code,
and in all three runs left every ADR and every code file byte for byte
unchanged. Citation counts are items, in every document; code comments
(230 Rust and Go files under `crates/`, `fuzz/` and `tools/oracle`) are
counted in a column of their own; frozen-file counts are files under
`crates/*/tests/data` (79, of which 4 are READMEs). The tree counted is
`b44f267`'s; this note was committed after the height-keyed chain tables,
`purse-udb` and their documents, which add 32 frozen files in four
batteries (28 batteries in all) and ten QUIRKS entries (760), and the runs
were not repeated on that tree.

| Move simulated | Commits | Files changed (Go) | `go.mod` requirement changes | Same | Shifted | Touched | Gone | Invalid at the pin | `cite -w` rewrote | Bare-root shifted, touched | Both upstream and local: touched | Code comments: same, shifted, touched, invalid | Frozen files: regenerate, pin-only, do not move | Oracle copies to regenerate |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| `54b85b33` to `c0fee6b5` (2026-08-18) | 8 | 9 (5) | 17 | 8,003 | 313 | 21 | 0 | 8 | 308 | 0, 14 | 3 | 93, 0, 0, 1 | 34, 11, 30 | 2 |
| `54b85b33` to `release-v2.1.7` | 88 (38 + 50) | 20 (18) | 2 | 6,850 | 1,402 | 76 | 9 | 8 | 1,361 | 45, 14 | 0 | 92, 0, 1, 1 | 23, 18, 34 | 2 |
| `54b85b33` to `4d526846` (v2.1 branch point) | 50 | 32 (28) | 19 | 5,822 | 2,251 | 255 | 9 | 8 | 2,164 | 45, 35 | 3 | 88, 4, 1, 1 | 35, 10, 30 | 3 |

Each `report`, with the `go list` calls of its frozen-file section, took
0.43 to 0.55 seconds (median 0.46, five runs per range) on an x86_64
Linux desktop.

What they showed:

- **QUIRKS.md.** The entries citing a changed file number 111, 357 and 458
  of the 750 for the three ranges; those with a touched or gone citation
  number 4, 13 and 33. ADR-0016 item 5.10 re-checks every entry citing a
  changed file, which for the 8-commit range is 111 entries, most of them
  because `internal/rpc/jsonrpc/methods.go` changed by a few lines.
- **Names that are files both upstream and here.** dcrwallet's
  `README.md` and `go.mod` share their names with this repository's, so a
  line citation of either is classified against the upstream file and
  listed, never rewritten. Line 77 of dcrwallet's `README.md` (the Go
  versions it names) differs in the `c0fee6b5` and `4d526846` ranges, which
  touches the three citations of it: QK-0641's and two in spec 22. Of the 39
  such items, 6 run past the end of the upstream file and most likely mean
  this repository's file; they are counted, not listed.
- **Code comments.** 94 items of citations in Rust and Go comments
  resolve to an upstream file. In the `4d526846` range four of them shift
  by 32 lines (the diff from the pin replaces line 310 of
  `wallet/chainntfns.go` by three and removes lines 382 to 415): the
  ones in `crates/purse-export-check/tests/opscripts.rs`,
  `tools/oracle/cmd/storebench/main.go` and two in
  `tools/oracle/cmd/oprunner/script.go`. The `wallet/addresses.go` range
  cited in `tools/oracle/cmd/oprunner/main.go` is touched in that range and
  in `release-v2.1.7`'s. A move corrects these by hand; `cite` lists them.
- **Declarations.** No shifted or same item in any of the three ranges
  sits in another declaration at the new revision than at the pin.
- **Frozen files.** The `go.mod` changes decide most of the frozen-file
  verdicts: commit `597873d6` ("Update modules", 2026-10-12) moves 17
  requirements, and the two ranges containing it regenerate 12 and 13 of
  the 24 batteries. In the `release-v2.1.7` range only `golang.org/x/mod` and
  `golang.org/x/tools` move, which no oracle command links, and the
  changed packages decide: `wallet/udb` (`addressdb.go`, `upgrades.go`),
  `wallet` and `internal/loggers` regenerate 6 batteries (`purse-snacl`'s
  two, `purse-hdkeychain`'s two, `purse-export-check`'s frozen export and
  opvectors) and the oprunner copy of `wallet/udb`. The verdicts are per
  oracle command, not per request: `purse-hdkeychain`'s five digest files
  are flagged by `wallet/udb` because the `hdkeychain` command links it for
  its wallet subcommand only.
- **Files without a checked pin.** 32 Go-generated or copied files carry no
  pin a test checks (ADR-0016's second group, which answer 8's uniform
  rule is to close): `purse-hdkeychain`'s five vector files,
  `purse-dcrutil`'s three, `purse-txauthor`'s and `purse-txrules`'s vector
  files (both added after ADR-0016 counted the groups), `purse-export-check`'s
  `full-simnet.pe` and twelve opvector files, `purse-snacl`'s
  `generated-simnet.db.gz` and `purse-boltread`'s eight `upstream/` copies.
  `tools/pinmove batteries` lists them.
- **Regeneration is not always byte for byte.** `purse-snacl`'s
  `snacl_vectors.txt`, `generated_db.txt` and `generated-simnet.db.gz`
  draw from the operating system's generator and differ on every
  regeneration, and `full-simnet.pe` has volatile values; item 5.6's
  "differs in the pin only" cannot hold for them without a mask or a
  replay, which no test defines yet.
- **Citations wrong at the pin.** 8 mapped citations run past the end of
  their file at the pin: QK-0602's of `testsolver.go`; spec 15's two of
  `deployments/deployments.go` and two of `internal/netparams/params.go`;
  spec 16's of `ipc.go`; spec 19's of `internal/cfgutil/curve.go`; and
  spec 16's of `methods.go` that list inheritance attributes to
  `internal/rpchelp/methods.go` where `internal/rpc/jsonrpc/methods.go` is
  meant. 11 bare-root citations do too: four of `params.go` in specs 16 and
  21 (the root file has 10 lines; `internal/netparams/params.go` is
  probably meant) and seven of `SECURITY.md` in spec 25, which refer to
  dcr-rs's file. `report` and `cite` list each with its line.
- **Citations nothing classifies.** 5,207 line citations resolve to no
  file, so neither `report` nor `cite` classifies them; `tools/pinmove
  survey` lists them by written name. 1,899 have no path to attach to (a
  `:N` after prose, neither in a list after a citation nor in a section
  that names one file). 968 name a base name several upstream files share
  (`methods.go` 419, `server.go` 292, then `db.go` and `methods_test.go`),
  which only a directive or a full path settles. 770 name a base name that
  exactly one upstream file outside the root has (`client.go`,
  `txmined.go`, `txdb.go`, `calls.go`, `addressdb.go`, `api.proto`), which
  a directive or a full path would map. 1,570 name files of other
  repositories or of dependencies written without their module (`lib.rs`,
  `rpc.go`, `extendedkey.go`, `hd.rs`, `bucket.go`), which need a `-`
  directive or a `module@version/` path. `survey` proposes directives for
  963 items; a move re-locates the rest by hand unless the documents gain
  directives or full paths first. Bare names of root files that other
  upstream files share (`config.go`, `log.go`, `params.go`) are not among
  the 5,207: those 390 citations are classified as bare-root and listed,
  never rewritten.
