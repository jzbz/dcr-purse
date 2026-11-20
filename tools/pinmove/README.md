# pinmove

The tooling ADR-0016 (upstream tracking) calls for before the first move of
the dcrwallet parity pin. A Go module of its own (standard library only),
beside `tools/oracle`. It reads a dcrwallet clone with git plumbing and
never fetches, commits or writes to it; the clone is given with `-upstream`.

```sh
cd tools/pinmove
go run . report -upstream ../../../dcrwallet -to origin/master -o report.md -json report.json
go run . cite   -upstream ../../../dcrwallet -to origin/master        # list only
go run . cite   -upstream ../../../dcrwallet -to origin/master -w     # rewrite
go run . batteries -upstream ../../../dcrwallet -to origin/master -o batteries.md
go run . survey -upstream ../../../dcrwallet
```

ADR-0016 item 5 runs `report` (5.1) before `DCRWALLET_PARITY_COMMIT` is
bumped (5.3) and `cite` (5.8) after it, when the default `-from` is
already the new pin. So `cite` at 5.8 is given the old pin, which `HEAD`
still holds until the move is committed:

```sh
old=$(git show HEAD:crates/purse-testutil/src/lib.rs | sed -n 's/.*DCRWALLET_PARITY_COMMIT: &str = "\([0-9a-f]*\)".*/\1/p')
go run . cite -upstream ../../../dcrwallet -from "$old" -to origin/master -w
```

An empty range (`-from` and `-to` one commit) is refused by `cite` and
flagged at the head of the report and on standard error by `report`, since
every citation would look unchanged.

`-from` defaults to `DCRWALLET_PARITY_COMMIT` (`crates/purse-testutil`);
`-repo` defaults to the nearest directory above the working directory that
holds `Cargo.toml` and `docs/`. The documents read are `README.md` and every
Markdown file under `docs/`; the comments of every Rust and Go file under
`crates/`, `fuzz/` and `tools/oracle` are read as well (below). `report`
and `batteries` take `-oracle`, the oracle's Go module (default
`tools/oracle`), whose packages they list with
`go list -deps`; that needs the Go toolchain and the oracle's module
requirements (downloaded on first use, read only).

## Commands

- **report** writes the review list of ADR-0016 items 3 and 5.1: the
  commits of the range (both sides of it when neither revision contains the
  other, with commits of the same subject on the two sides paired, as a
  release branch's backports are), the files changed with their
  zero-context hunks, every citation that resolves to an upstream file
  classified (the rest counted), the touched, gone and invalid ones
  listed, the citations of a name that is a file both upstream and in this
  repository (`README.md:77`, `go.mod:3`) classified against the upstream
  file and listed when the range moves them, the citations in code
  comments (below) listed, the QUIRKS entries citing changed files, the per-document review
  list (the commits touching the files each document cites; a bare name
  several upstream files share, written without a line, does not count,
  and `docs/UPSTREAM.md` is left out) and the `go.mod` delta, with the
  dependency citations whose module moves. A module moves when its
  effective version changes: the target of a `replace` directive for it
  (one naming the required version before one naming the bare path; a
  local directory counts), or else its requirement. dcrwallet has changed
  replaces with the require lines as they were (183083d6, 92529a4b,
  99be2039). `-json` writes every classified citation as well.
- **cite** rewrites the *shifted* citations in the documents of ADR-0016
  item 5.8 (the specs, `docs/QUIRKS.md`, `docs/PARITY.md`, `docs/SKEW.md`,
  the brief and `README.md`) and lists the touched, gone, invalid and
  bare-root ones for a human, and the rewrites made by a rule that guesses
  a path (the root rule, a section's or sub-heading's file, completion
  from a list, a numeric code span) for checking, and the items whose
  enclosing declaration differs (below), the citations of a file both
  upstream and here, and the citations in code comments the range moves,
  none of which it rewrites. Without `-w` it only lists. It never edits
  `docs/adr/`, `docs/UPSTREAM.md` or any code. It skips a document that does not name
  the old revision's eight-digit abbreviation (it runs before the move
  updates the text naming the pin). With `-w` it ends each document it
  covers with the comment `<!-- pinmove-cited-at: <new commit> -->`
  (replacing an earlier one) and skips a document whose comment names
  another commit than the old revision, so a second run over the same
  range, before or after the first is committed, changes nothing; a
  document without the comment is taken to be at the old revision. With
  `-w` it also refuses documents with uncommitted changes (`-allow-dirty`
  overrides), so that its rewrite is a diff of its own.
- **survey** reports how the documents' citations parse: every written
  form with its resolution and count, the share parsed, every unparsed form
  with examples, the unresolved written paths, and per document the
  directives proposed for bare names (below).

- **batteries** (ADR-0016 items 5.1, 5.6 and 6) lists every file under
  `crates/*/tests/data` with the battery it belongs to, how its head names
  the pin (the full commit, 12 or 8 digits, only the words "parity pin",
  a README beside a binary file, or not at all), its group in ADR-0016's
  sense (1: Go-generated and the pin checked by a frozen test; 2:
  Go-generated or copied at the pin, the pin unchecked; 3: not tied to the
  dcrwallet commit), and a verdict for the range: *regenerate* when the
  range changes a non-test Go file of an upstream package one of the
  battery's oracle commands links (directly or through one of the oracle's
  copies of upstream source), moves the `go.mod` requirement of a
  dependency module they link, or changes an upstream file the battery
  copies or reads; *pin-only* for every other battery tied to the pin,
  which a move regenerates all the same and which must then differ in the
  pin it names only (item 5.6); *does-not-move* otherwise. It also lists
  the oracle's copies of upstream source (item 5.4) that the range changes,
  and the files the uniform rule of ADR-0016 answer 8 still has to give a
  checked pin. Without `-to` it describes the pin itself.

The batteries come from `batteries_table.go`: which regenerator writes
each set of files, which oracle commands it runs, which upstream files a
copy or a fixture comes from, and which frozen test checks the pin. The
packages each command links are not in the table; `go list -deps` gives
them, at the dcrwallet version the oracle requires (the report says when
that is not the old revision). The closure at the old pin decides: a
package the new revision adds to a command can only arrive through an
import added to a package already in it, which has then changed. The
granularity is the command, not the request: a battery whose command links
`wallet/udb` for one subcommand is flagged when `wallet/udb` changes even
if its files come from other subcommands. `TestRepoTable` fails when a file
under `crates/*/tests/data` is in no battery or in two, when a pattern
matches nothing, when a command or a copy does not exist, or when a file's
header names an oracle command its battery does not list; a new frozen
file is entered in the table in the change that adds it.

## Classes

Each line or range of a citation into an upstream file is mapped through
the zero-context Myers diff between the two revisions:

| Class | Meaning | cite |
|---|---|---|
| same | every line unchanged, at the same number | nothing |
| shifted | every line unchanged and still contiguous, at new numbers | rewrites |
| touched | a line inside changed, or lines were inserted between two of its lines | lists for re-reading |
| gone | the file does not exist at the new revision (a rename is named as a hint), or an object of another type, such as a symbolic link, took its path | lists for re-locating |
| invalid | the citation runs past the end of the file at the old revision | lists for correcting |

A range is never judged by its two ends alone (ADR-0016, Decision item 6).
Renames are not followed into the new file.

A shifted item keeps its text, but the Myers diff can pair identical lines
of one function with those of another: when a change appends a function
whose last lines repeat those of the one before it (`return nil`, `}`),
the cited lines of the first can be found in the second; an item left at
its old numbers can likewise sit in a declaration that replaced the one it
was in. So for every shifted and every same item in a file the range
changes, the enclosing declaration of its first and last lines (Go: the
func, with its receiver's type, type, var, const or import; protobuf: the
message, service, enum or rpc) is compared at the two revisions by
position: the old declaration's opening line, mapped through the diff,
must be unchanged and open the new declaration. Names alone would pass a
move into another `func init` or another `var (` block. Only when the
opening line itself changed (a new signature) are the names compared, so
that a changed signature is not a different declaration. Where the
declaration differs, the item is reported with both and `cite` lists it
instead of rewriting it. Other file types are not checked, and a pairing
within one declaration is not caught (`decl.go`). A declaration appended
whole that repeats the old one exactly is paired with it by the diff, and
its lines are then rewritten: nothing in the text tells the two apart.

## Code comments

Rust tests and oracle commands cite dcrwallet in their comments too
(`crates/purse-export-check/tests/opscripts.rs` names the
`wallet/chainntfns.go` lines it reproduces). ADR-0016 item 5.8 rewrites
documents only, so these are listed and never rewritten: `report` counts
them on a line of their own and lists every one the range shifts, touches
or removes, and `cite` lists the same for the move to correct by hand.
Only comments are read (`code.go` blanks code and string literals,
including raw strings and Rust character literals, keeping line numbers),
from the `.rs` and `.go` files under `crates/`, `fuzz/` and `tools/oracle`,
leaving out `testdata`, `target` and `tests/data` directories and the
oracle's copies of upstream source in `batteries_table.go`, whose comments
are upstream's and move with the copy (item 5.4). `tools/pinmove` itself
is left out: its tests cite synthetic repositories. The survey does not
cover code comments.

## The citation grammar

The full grammar and resolution rules are the comment at the head of
`citations.go`. In short: `path:N`, `path:N-M` (hyphen or en dash),
comma lists (`path:1,5,9-12`, `path:487-526, 1351-1355`),
`module@version/path:N` for a dependency (moves only with `go.mod`),
`goX.Y.Z/src/...` for the Go toolchain, continuations `:N` that belong to
the citation they follow in a list, numeric code spans after a citation's
code span (`` `wallet/rescan.go:157`, `234` ``, `` `spv/sync.go:205`–`206` ``;
after a comma, semicolon or "and", only when the line ends, punctuation,
"and", "or" or a parenthesized code span follows, so that the value in
`` `wallet/wallet.go:2328`, `4941` atoms `` is not read as a line),
and a parenthesized `:N` in a section whose heading or first `Source:`
line names one file, or after a line holding only `` `path`: ``, which
heads what follows until the next heading.

Paths resolve against the upstream tree at the old revision. A path
written in full maps; so does a bare name completed from the path it
follows in a list, and a bare name of an upstream root Go file whose base
name no other upstream file has. A bare name after a dependency citation
is completed into that module only when it is not a file at the upstream
root and the module's copy in the Go module cache (`GOMODCACHE`, else
`GOPATH/pkg/mod`) does not lack it; when the module is not in the cache
the name is completed as before, so the resolution of such names depends
on what the cache holds where the tool runs. A bare name shared by several upstream
files (`config.go`, `log.go`, `methods.go`) is not guessed: it is listed,
never rewritten, unless the document declares it:

```markdown
<!-- pinmove: methods.go=internal/rpc/jsonrpc/methods.go client.go=dcrd/mixing@v0.7.5/mixclient/client.go helper.go=- -->
```

A directive maps a name as written to an upstream path, to
`module@version/path`, or to `-` (never upstream), for the whole document.
`survey` proposes directives where the document writes exactly one full
path of a name and every cited line exists in it; each proposal has to be
checked by reading before it is added.

## Tests

`go test ./...` runs the grammar cases, the line mapping, the comment
extraction, fuzz targets over the parser and the comment extraction
(`go test -fuzz FuzzParseDoc`, `go test -fuzz FuzzCommentText`), and tests on small git
repositories built in temporary directories: a synthetic upstream with
fixed revisions (a change inside a cited range, an insertion inside one, a
deleted and a renamed file, a `go.mod` bump) and synthetic documents that
`report` and `cite` run over, a rerun of `cite -w` before and after its
result is committed, `replace` directives added, removed and pointed at a
local directory with the require lines unchanged, a file replaced by a
symbolic link, a shifted item the diff moves into another function or
into another `func init` or `var (` block of the same name, an item at
unchanged numbers whose function was replaced, citations of a file both
upstream and local, citations in Rust and Go comments (with string
literals, a frozen-file directory and an oracle copy that must not be
read), the refusal of an empty range, the
review list's exclusions, a randomized comparison of the
classification against the truth of random edits, a synthetic upstream
with packages, a fixture file and a module bump against synthetic frozen
files for `batteries` (every verdict and group, copies, files in no
battery or in two), `go list` itself on a small oracle module that
replaces its upstream with a local directory (offline), and
`TestRepoTable` against this repository. The tests need `git` and `go`;
their commits are unsigned and read no user or system git configuration.

CI runs the tests on every push (`.github/workflows/ci.yml`, job
`pinmove`), and once a month `.github/workflows/upstream.yml` runs
`report`, `batteries` and `cite` (list only) against dcrwallet's master
and its newest final release tag (`release-vX.Y.Z`, never a release
candidate) and publishes the output as the job artifact `pinmove-report`;
that job never fails the build. Its output is read into the dated notes
of `docs/UPSTREAM.md`.
