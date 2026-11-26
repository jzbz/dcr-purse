# ADR-0012 — Standing quirk policy

- **Status:** Accepted (decision D10; project owner, 2026-11-18) as
  proposed, with one change: the coins a partial VSP purchase leaves locked
  until a restart (QK-0279) are fixed, not reproduced (Rule 7; Open point
  3). Restated, not re-decided: the
  owner's rulings on the database divergence (2026-11-14,
  `docs/PARITY.md:70-72`), reference Go 1.27.4 (2026-11-18, spec 23 §13
  item 1) and QK-0729 (2026-11-18, `docs/QUIRKS.md:4395`).
- **Amended, 2026-11-19 (project owner):** Rule 7's default for funds held
  only until a restart or an unlock is reversed: such defects are fixed by
  default, each with a PARITY.md row and a test showing Go's behaviour,
  and only a case whose difference a client could observe in a reply goes
  to the owner. QK-0279 is now an instance of the rule, not an exception.
  See "Addendum, 2026-11-19 — Decision by the project owner: Rule 7" at
  the end.
- **Settled, 2026-11-20 (project owner):** a fix under Rule 7's default
  is not a security-relevant candidate under ADR-0017, and the first half
  of QK-0035 is reproduced, not a Rule 7 case. See "Addendum, 2026-11-20"
  at the end.
- **Decided, 2026-11-21 (project owner):** the digit search of the wiped
  base58 codecs, which decode private strings, is to be replaced with a
  constant-time lookup under Rule 6; scheduled at low priority, not yet
  done. See "Addendum, 2026-11-21" at the end. *Done, 2026-11-22: that
  addendum's note of that date.*
- **Decided, 2026-11-22 (project owner):** a Rule 7 case whose difference
  a client could observe is brought as Rule 11 brings one, as proposed;
  and brief §3 W1's text stays as it is, the funds Rule 7's default fixes
  being governed by that default alone. See "Addendum, 2026-11-22" at the
  end.
- **Decided, 2026-11-22, later (project owner):** the base58 codecs'
  lookup as built, arithmetic on the alphabet's six runs, stands in place
  of the lookup reading all 58 entries that the addendum of 2026-11-21
  named; and the codecs' timing checks run in a workflow of their own,
  filtered on the paths that can change their result and run nightly,
  while the disassembly check stays in `ci.yml` on every push. See
  "Addendum, 2026-11-22 (later)" at the end.
- **Decided, 2026-11-24 (project owner):** the codecs' timing check
  gains diagnostics only, after a failure on GitHub that its log could
  not explain: a trace of every target, and a diagnostic pass of each
  `chars` target, a test of its own that is never judged. No threshold,
  floor, budget, retry or blocking change. One planted leak that the
  harness of `05f7862` found in every run, a dependent multiplication in
  `hdkeychain chars` at opt-level 3, is found less often as shipped: 39
  of 60 runs in six sets, from 1 to 10 of 10 a set on the same machine
  code, 30 of them by the plant's own slower path, against 70 of 70 (64)
  with `05f7862`'s harness; what holds from set to set is a smaller cost
  of the plant in the shipped binary (all 0 judged slower by a median of
  +1.04 ns an operation over the six sets, +0.92 to +1.44 a set, against
  +1.56, +1.38 to +1.90 a set), close to the 1 ns floor. The
  measurements put that down to the binary's layout, not to the
  verdict's rule; the addendum records it, open for the project owner's
  decision. See "Addendum, 2026-11-24" at the end. *Decided,
  2026-11-25: the next bullet.*
- **Decided, 2026-11-25 (project owner):** the loss of detection the
  bullet above records (the dependent-multiplication plant in
  `hdkeychain chars` at opt-level 3, 39 of 60 runs as shipped against 70
  of 70) is accepted as recorded, and no verdict rule changes. Making
  the timed operation less sensitive to code layout (for example an
  operation that works in place instead of copying its input to the
  stack) is planned as a separate, later change; it is not made. See the
  note of 2026-11-25 at the end of "Addendum, 2026-11-24".
- **Date:** 2026-11-18

## Context

**The question.** The brief reproduces a defect when an external
observer could depend on it, recorded in QUIRKS.md with a pinning test,
and gives three classes a standing rule: an error where Go panics on
malformed data, a deterministic order where Go's is random, a fix
recorded in PARITY.md where reproduction would lose funds or leak secrets
(brief §3, last paragraph; §9 D10; `README.md:102-106`). The policy
"takes effect when D10 is ratified" (`docs/PARITY.md:81`).

**Why now.** Phase 0 exits only with D1–D10 decided (brief §6, §11 M0),
and every task must reproduce a quirk or record its divergence (brief §12
item 1). QUIRKS.md holds 732 entries, all candidates (`docs/QUIRKS.md:5`);
11 have a disposition; 48 contain "panic" (case-insensitive search).

**Cases already met** under the unratified rule, with the rule below that
now decides each:

| Case | Record | Rule |
|---|---|---|
| `errors.match` meets a nil level: no match, walk continues | QK-0628 | 2(b) |
| typed-nil `*Error`, unknown kind, `E()` with no arguments: cannot be written | QK-0628; `PARITY.md:76` | 4 |
| foreign wrapper is an opaque leaf | QK-0728 | 5 |
| seed text is `&str` | `PARITY.md:77` | 4 |
| failing random source is an error | `PARITY.md:77` | 2(a) |
| seed words in `DecodeSeed` errors: reply kept, log redacted | QK-0729 | 6 |
| `Neuter`/`Zero` do not alias; `SerializedPrivKey` lent read-only | QK-0713, QK-0714 | 5 |
| private bytes of the empty private form: an error | `PARITY.md:78` | 7 |
| public child tweak in variable time | `crates/purse-hdkeychain/src/lib.rs:73-91` | 6 |
| malformed `wallet.db`: error where bbolt panics or overreads | QK-0730 | 2(a) |
| bad page size, depth > 64, page reached twice, keys out of order; page > 256 MiB | `PARITY.md:79`; `crates/purse-boltread/src/db.rs:25-55` | 8 |
| both meta pages invalid: probe with 4096; Windows share-mode open | `db.rs:62-65`, `:246-251` | 9(b), 9(c) |
| `NewAmount` overflow and fused rounding: amd64's answer | QK-0512; `crates/purse-dcrutil/src/amount.rs:10-42` | 9(a) |
| `sort.Sort` ties in `txsort`: Go's sort ported | `crates/purse-dcrutil/src/lib.rs:18-22` | 3 (not random) |

**dcroxide** records only reproductions in its QUIRKS.md (header) and
has no quirk-policy ADR. It ports dcrd's assertion panics as a process
stop (dcroxide `docs/PARITY.md:391-410`) and aborts on panic in release:
a daemon "cannot reason about state a panic left half-mutated"
(`:1150-1157`).

## Options considered

**A. The brief's rule as written.** No new work; each boundary in the
table is decided again per crate.

**B. Reproduce everything; every divergence is an owner decision.**
Maximal fidelity. A panic recovered on HTTP POST reproduces cheaply as a
dropped connection, websocket and gRPC panics as crashes a client or
peer can trigger; random order cannot be reproduced. The owner becomes
every phase's bottleneck.

**C. Fix every defect.** Simplest code. Breaks W4 and W5, whose clients
match on exact replies and error text (brief §3).

**D. Ratify the brief's rule as numbered rules with boundaries, a fourth
class, and a proof and record per outcome.** Costs the compliance work
under Consequences. An implementer can decide a case without the owner,
and a reviewer can check it.

## Decision

Option D. It confirms the brief's default test and its funds-and-secrets
class. It departs from the brief in five places, argued at each rule:
Rule 2 widens "malformed data" to any external input and keeps Go's
assertions as a stop; Rule 3(b) keeps wire-visible orders random; Rule 7
reads "unspendable" as surviving a restart; Rule 8 refuses some input Go
accepts; Rule 9 is a new class. *(Rule 7's reading stands for W1; what it
left outside, funds held until a restart or an unlock, is fixed by default
since 2026-11-19: addendum of that date.)*

**Rule 1 — Default.** A defect is reproduced when an external observer
could depend on it: it repeats for the same input and shows on a surface
W1–W7 name (network bytes, RPC replies and codes, CLI, configuration,
prompts, exit codes, log lines, files) or in wallet state reachable by
query (the logical export). Not surfaces: the Go library API (brief §1),
the bytes of `wallet.db` (W2), timing other than specified timeouts, Go
runtime panic wording. Aliasing and memory layout are not surfaces; a
difference in them is a divergence whose effect at each call site must
be shown absent under Rule 5.

**Rule 2 — Go panics.** In Go the outcome depends on the transport: on
HTTP POST net/http runs the JSON-RPC handler, recovers, writes "http:
panic serving …" to stderr and drops the connection
(`internal/rpc/jsonrpc/server.go:534`, `:595`; no `ErrorLog`,
`:97-110`); websocket handlers run in their own goroutine (`:448-459`)
and gRPC has no recovery, so there the process ends (the one `recover()`
re-raises, `dcrwallet.go:176-182`). A panic, overread or unbounded
recursion reachable from external input — a file, a peer, an RPC
argument (QK-0493), a system source — becomes no panic, with one outcome
on every transport. A Go assertion on the wallet's own state, reachable
only through a wallet defect (`wallet/sidechains.go:297`), stays a stop:
a Rust panic, aborting in release (`Cargo.toml:40`). For external input,
in order:
(a) If the Go function returns an error, return one, with the kind and
code it returns for its nearest comparable failure (`IO` for corrupt
stored data, spec 03 D2; the invalid-parameter code for an RPC
argument). The text is dcr-purse's own.
(b) If it returns only a value, return what Go computes when the
panicking step is taken as not matching, or the faulty element as absent
(QK-0628). Where the value decides whether data from a peer, client or
file is accepted (QK-0227), return the value that rejects it and check
it under Rule 7. The disposition states the direction and why.
(c) If no such value exists, make the Rust function fallible and
propagate as in (a) (spec 03 D2, `MainChainTip`).
(d) Or make the input unrepresentable (Rule 4).
Rejected: a dropped connection on HTTP POST only, which is cheap but
keeps a per-transport difference no client can act on.

**Rule 3 — Random order.** Only map iteration. Deterministic but
unspecified orders (sort ties, bbolt key order) are reproduced exactly
(QK-0175).
(a) In replies, logs and exports, and wherever map order picks the error
reported or the elements processed: ascending by the map key in its own
encoding (address strings by bytes, numbers numerically, hashes in
internal byte order as in dcroxide `docs/PARITY.md:473-476`). An order under a constraint is the constrained
order with ties broken by key (spec 03 D1). Where an early return
depends on map order, every element is processed when no reply shows the
difference (spec 12 Q9); otherwise the sorted order decides and the
PARITY line records which elements are always skipped. `makeOutputs`
reports the first failing address in sorted order.
(b) In bytes other parties see — `sendmany` outputs
(`internal/rpc/jsonrpc/methods.go:3567-3587`), tspend votes (QK-0136,
QK-0233) — a uniform shuffle from a generator separate from
`purse-rand`'s stream, seeded from the operating system even under test.
Go's map order never draws from `dcrd/crypto/rand`, the stream the det
oracle seeds (spec 23 §5.6.1, §5.6.2); a shuffle drawn from it would
shift every later draw (the change position, `wallet/createtx.go:444-445`;
coin selection; mixing, which W3 fixes). These sites are compared as sets
(spec 23 §5.6.5). Depends on spec 23 §13 item 2. Rejected: a sorted order
(brief §3; spec 18 D5), byte-comparable but marking dcr-purse's
transactions and votes (Open point 2).
Each site gets a PARITY.md line naming its order.

**Rule 4 — Unrepresentable values.** "Cannot be written" is an accepted
divergence when the value can arise only from Go code calling Go code
(typed-nil `*Error`, an unknown kind, `E()` with no arguments). A value
that can arrive from outside stays representable as far as Go represents
it (open proto enums, `docs/specs/19-grpc-wallet-service.md:315`). A
narrower input type (`&str` for a Go `string`) needs a Rule 5 trace
showing every call site at the pin passes valid UTF-8.

**Rule 5 — "Not observable".** The QUIRKS disposition carries a call-site
trace: the search run at the pin, every hit by file and line, and for
each why no output depends on the difference. Repeated at each pin move.

**Rule 6 — Secrets.** It triggers when reproduction would move secret
material (seeds and seed words, private keys including WIF, passphrases,
private chain codes, derived encryption keys) to a sink its supplier does
not hold: the log, another client, the network, a file, a crash report.
The fix is made at the narrowest point that stops that flow (QK-0729). A
reply to the authenticated caller who supplied the secret is not a sink,
as the owner ruled for QK-0729; secret values in a reply that no client
could depend on are redacted (Open point 1). Rejected: redacting every
reply that quotes a secret, which reverses QK-0729. Arithmetic on secret
values is constant time whatever Go does (dcroxide ADR-0017): no output
changes, so it is no divergence. Variable time stays only on values a
holder of public data can compute, with the residual exposure documented
(`crates/purse-hdkeychain/src/lib.rs:73-91`). *2026-11-21: the
base58 codecs' digit search is covered and its replacement scheduled,
addendum of that date.* *2026-11-22: replaced, the same addendum's note
of that date.*

**Rule 7 — Funds.** It triggers on an outcome W1 forbids: a key, address
or output not derived or not recognised; a spend unrecorded or wrongly
recorded in persisted state (QK-0082); funds unspendable after a
restart; a transaction moving a value, fee or destination other than
requested; a key everyone knows (the zero scalar, `PARITY.md:78`). Funds
held only until a restart or an unlock RPC are outside W1 and are
reproduced by default. QK-0279 is the decided exception (project owner,
2026-11-18): dcr-purse releases the fee credits reserved for tickets that
were not published, instead of leaving them locked until a restart. It is
a divergence recorded in `PARITY.md` when the purchase flow is built
(Phase 9), decided under Rule 11, and it does not widen W1.
*Amended (project owner, 2026-11-19): funds a defect leaves unavailable
until a restart or a manual unlock are no longer reproduced by default but
fixed, each fix with a PARITY.md row and a test showing Go's behaviour;
only a case whose difference a client could observe in a reply is brought
to the owner. QK-0279 now reads as an instance of that default, not as an
exception. The first sentence, W1's outcomes, is unchanged. Addendum of
that date.*

**Rule 8 — Input Go accepts.** The `wallet.db` reader may refuse input
bbolt would read; ~~the refusal aborts the import cleanly (W2).~~ (Void
clause: there is no import, ADR-0013; the reader is a test utility, see
the addendum, item 4.)
(a) Structural: the writer at the pin cannot produce the input, argued
beside the check (`MAX_TREE_DEPTH`, `MIN_PAGE_SIZE`, `db.rs:25-40`).
(b) Resource limits: configurable, the bound stated and far above the
largest value dcrwallet stores (`DEFAULT_MAX_PAGE_BYTES`, `db.rs:45-55`).
For other input this is no standing class: Rule 2 covers panics, Rule 11
the rest.

**Rule 9 — Platform-dependent behaviour (new class).**
(a) Compiler or instruction-set semantics (out-of-range float
conversion, multiply-add fusion): one answer on every target, that of
dcrwallet built for amd64 at the default GOAMD64 with the reference Go
(spec 18 D4). Rejected: following the target architecture, which keeps
an arm64 user's answers but needs per-architecture vectors and makes
results depend on the CI runner; Rust's saturating conversion, which
matches no amd64 build.
(b) Host properties Go queries (CPU count, OS page size, clock): the same
property, measured as Go measures it (QK-0630; where Go then panics,
Rule 2 applies). A fixed value is a recorded divergence (the 4096
fallback, `PARITY.md:79`).
(c) Behaviour dcrwallet defines per operating system (W6) is reproduced
per system; where `forbid(unsafe_code)` prevents it, the divergence is
recorded.

**Rule 10 — Proof and record.**

| Outcome | Proof | Record |
|---|---|---|
| Reproduced (Rule 1) | oracle-confirmed pinning test | QUIRKS disposition "reproduced" |
| Divergence (Rules 2, 3, 6–9) | test pinning dcr-purse's result; Go's shown by the oracle, or a source citation where no input reaches it; the differential excludes exactly those inputs and says so | PARITY.md line under the crate; QUIRKS disposition naming the rule |
| Unrepresentable (Rule 4) | type-level argument and a Rule 5 trace | as for a divergence |
| Not observable (Rule 5) | the call-site trace | QUIRKS disposition with the trace |
| Assertion kept (Rule 2) | citation that no external input reaches it | PARITY.md line |

**Rule 11 — Cases no rule fits.** The project owner decides. The
implementer writes the proposed disposition into the QUIRKS entry, marked
"pending owner", and names it in the change submitted for signing. Until
the ruling the code reproduces Go, unless Rule 6 or 7 applies; the
ruling is recorded as a divergence, marked "(project owner, date)".

**Rule 12 — QUIRKS.md lifecycle.** candidate → confirmed by an oracle
run, or by a source citation when no input reaches it (QK-0628); an
entry the oracle contradicts is corrected with its spec (brief §12 item
6) → disposition under Rule 10 → re-checked at each pin move. Entries
are never deleted or renumbered; a divergence with no entry gets a new
one.

**Relation to ADR-0011.** This ADR decides whether a defect is
reproduced; ADR-0011 decides how exactly reproduced text matches. Text
written under Rule 2 has no Go original. Rule 6 overrides text fidelity
only at the redaction point.

**Relation to D13.** A confirmed entry is an upstream-report candidate
when it is a Rule 6 or 7 divergence, a Rule 2 panic reachable from
network or RPC input (QK-0227, QK-0493), or a reproduced defect that
harms dcrwallet's users (QK-0279). D13 decides who submits and when.
*(Since 2026-11-19 a defect fixed under Rule 7's amended default, QK-0279
included, is a Rule 7 divergence and so a candidate under the first
clause; addendum of that date.)*

## Consequences

- Rewritten to cite this ADR: `PARITY.md:81`; `QUIRKS.md:5` and `:7`;
  `README.md:102-106` (Rule 3(b) is not "deterministic"). Each
  disposition gains its rule number.
- Records that fail the rules today: QK-0713 names areas, not lines,
  for 33 `.Zero()` calls outside tests (among them
  `wallet/wallet.go:3828-3830`); QK-0728 has no trace; `purse-seed`'s
  `&str` fails Rule 4 until Open point 4 is settled, since the seed
  prompt passes text that need not be UTF-8
  (`internal/prompt/prompt.go:320-350`); QK-0729 cites
  `internal/rpc/rpcserver/rpcserver.go:443-445`, absent at the pin (the
  log call is `rpcserver.go:444`). QK-0009 (the lock) and QK-0710 (the
  empty private form) need dispositions; the failing random source,
  `E()` with no arguments, the page-size refusal and the 4096 fallback
  need entries, from QK-0733.
- Work created: a PARITY line per map-ordered site and per kept
  assertion; the separate shuffle generator; Rule 2 tests on each RPC
  transport; every Rule 5 trace repeated in the pin review (brief §4
  item 7; D12).
- Spec questions closed (brief §6 requires them triaged): spec 18 D4 and
  D5; spec 03 D1 and D2; spec 06 §12.3 items 5–6 and §13 item 1; spec 12
  Q9; spec 02 §10 item 25. Constrained: spec 23 §13 item 2.
- If Open point 1 is answered "reproduce", brief §4 item 4 is edited; D13
  keeps QK-0201 as a defect either way.
- Risks accepted: an arm64 dcrwallet differs on out-of-range amounts and
  fused rounding; a client that saw a dropped connection or a crash sees
  an error; a kept assertion stops the daemon, which then needs a
  supervisor, as dcroxide's does.
- Revisit if an acceptance client (Decrediton, `dcrctl`, vspd) depends on
  a random order or a panic, or spec 23 §13 item 2 rules out a second
  generator.

## Open points

Decided as proposed (project owner, 2026-11-18), except point 3, which was
decided the other way. They are kept as the record of what was asked and
answered.

1. **QK-0201.** `signrawtransaction` errors carry every supplied WIF key
   (`%v` of the whole map, `wallet/wallet.go:4816-4823`; reply,
   `internal/rpc/jsonrpc/methods.go:5094-5113`); gRPC passes no keys
   (`internal/rpc/rpcserver/server.go:1623`, `:1688`). Reproduce, redact
   the values keeping the text's shape, or apply the brief's fix (§4
   item 4)? No client can depend on key values in a message, the text
   lists keys beyond the missing one, and client tools may log RPC
   errors as non-secret. Proposed: redact the values, keep the shape.
2. **Wire-visible order.** The brief says "deterministic"; Rule 3(b)
   keeps a shuffle, so those sites are compared as sets, not bytes, and a
   second generator exists. Accept the departure? Proposed: yes.
3. **Funds locked until a restart (QK-0279).** Decided (project owner,
   2026-11-18), against the proposal: the bug is fixed. W1's scope is
   unchanged; this one case is an owner-decided exception (Rule 7).
   *Since 2026-11-19 an instance of Rule 7's amended default rather than
   an exception; the decision stands (addendum of that date).*
4. **Non-UTF-8 seed prompt input.** How does `purse-prompt` convert raw
   terminal bytes? Proposed: lossy conversion to U+FFFD, recorded as a
   divergence; Go's matching sees those bytes as U+FFFD too
   (`crates/purse-seed/src/lib.rs:36-41`), so only the quoted text in an
   error differs.
5. ~~**QK-0082 on import.**~~ Rule 7 covers the defect in dcr-purse's own
   store. ~~On import of a `wallet.db` holding the dangling unspent entry,
   does dcr-purse repair it and report the repair? Proposed: yes, in the
   verification report.~~ (Void except its first sentence: there is no
   import, ADR-0013; see the addendum, item 5.)
6. **Security reports.** dcrwallet's `SECURITY.md` sends vulnerabilities
   to the Decred bug bounty and requires them reproducible on the latest
   release or master. Do Rule 6, Rule 7 and remote-panic candidates go
   there rather than to public issues? Proposed: yes, only after oracle
   confirmation. *The bounty route is superseded by private delivery by
   the owner (project owner, 2026-11-19); see the addendum of that date.*

## Addendum, 2026-11-18 — ADR-0013

On 2026-11-18 the project owner decided that dcr-purse imports no
dcrwallet database: migration is by seed restore only (ADR-0013).
`purse-boltread` stays, for now, as a test utility that is not part of the
wallet. The rules of this ADR stand. The statements below rested on the
import, and change as follows:

1. **Status, "the owner's rulings on the database divergence
   (2026-11-14)".** The own-format part of that ruling stands; its import
   is reversed by ADR-0013. Both are now recorded in `docs/PARITY.md`,
   "Deliberate divergences", under "The database".
2. **Context, the three `purse-boltread` rows of the table of cases**
   (malformed `wallet.db`, QK-0730, Rule 2(a); bad page size and the other
   structural refusals, Rule 8; the 4096 probe and the Windows share-mode
   open, Rules 9(b) and 9(c)). They stand as the record of how the test
   utility behaves. None of them is a divergence on a surface of the
   wallet, which reads no `wallet.db`.
3. **Rule 1, "the bytes of `wallet.db` (W2)"** among the things that are
   not surfaces. Still not a surface; the reason is now that dcr-purse
   neither reads nor writes that file (ADR-0013). W2 as written is void.
4. **Rule 8, input Go accepts.** It now governs only `purse-boltread`, a
   test utility. "The refusal aborts the import cleanly (W2)" has no
   import to act in: a refusal fails the test that read the file. Items
   (a) and (b) stand for the utility. The departure from the brief that
   the Decision section lists for Rule 8 concerns no input of the wallet.
5. **Open point 5, "QK-0082 on import".** Void: no import exists, so no
   `wallet.db` holding the dangling unspent entry ever reaches dcr-purse,
   and there is no verification report to record a repair in. Its first
   sentence stands: Rule 7 covers the defect in dcr-purse's own store,
   which never writes such an entry.

The void text of Rule 8 and of Open point 5 is struck in place, not
removed: `docs/adr/` had not been committed when this addendum was
written, so git history does not hold the accepted text. `purse-boltread`
is a test utility only and the owner may later remove it (ADR-0013 Open
point 2).

Citations of `docs/PARITY.md` by line number in this ADR (`:70-72`, `:76`
to `:79`, `:81`) refer to the file as it stood when the ADR was accepted
and no longer match it; the items are found by name under "Deliberate
divergences" (the database bullet, then one bullet per crate) and in that
section's closing paragraph, which now states this ADR's policy.

## Addendum, 2026-11-19 — ADR-0017: the route of upstream reports

On 2026-11-19 the project owner accepted ADR-0017 (D13) except its
reporting route, and decided its Open point 3, which proposed amending
Open point 6 above, against its recommendation as far as the bounty goes
(ADR-0017, "Decision by the project owner", which quotes the owner's
words). Open point 6 changes as follows:

1. **The bounty route is superseded.** Rule 6, Rule 7 and remote-panic
   candidates do not go to the Decred bug bounty, and no candidate goes to
   a public issue filed by this project. The owner delivers the reports
   personally to the developers over private channels, in an order and at
   a time of the owner's choosing. The precondition recorded with the
   route ("only after oracle confirmation") goes with it; what, if
   anything, must be run before the owner passes a report on is the
   owner's to decide.
2. **Reproducers stay out of this repository until upstream has released
   a fix** (ADR-0017 Open point 3, as recommended): a reproducer of a
   security-relevant candidate (ADR-0017's S-list), an oracle differential
   included, is kept outside the repository and enters it, if at all, only
   after the fix is released. Other entries are confirmed by oracle runs
   as Rule 12 says.
3. **Unchanged:** "Relation to D13" (which entries are candidates), and
   whether a defect is reproduced or diverged from in dcr-purse, which
   this policy's rules and ADR-0017 Decision item 6 decide independently
   of how upstream is told.

## Addendum, 2026-11-19 — Decision by the project owner: Rule 7 and funds unavailable until a restart

**What was put.** Rule 7 as accepted: defects that lose or misdirect funds
(W1's outcomes) are never reproduced; funds held only until a restart or
an unlock RPC are outside W1 and reproduced by default; QK-0279 is a
one-off exception the owner decided on 2026-11-18 (Open point 3).

**Decision.** On 2026-11-19 the owner chose the option put as "flip the
default but keep the record": such bugs, those that make funds unavailable
until a restart or a manual unlock, "are fixed by default, each with a
PARITY row and a test showing Go's behaviour, and only the cases where a
client could observe the difference in a reply are brought to the owner".
It was the recommended option. The text above stays as written, apart from
the dated pointers added to it.

**Rule 7, second half, as it now reads.** Funds a defect leaves
unavailable only until a restart or a manual unlock (an unlock RPC) are
fixed by default, not reproduced. QK-0279 is an instance of this default:
the fee credits a partial VSP purchase leaves locked are released. The
owner's decision of 2026-11-18 on it stands; what changes is that it no
longer stands alone. "It does not widen W1" holds for the class as
recorded: the default is a rule of this policy, and brief §3 W1's text is
unchanged. The first half of Rule 7, W1's outcomes, is unchanged.

**Procedure for each such fix.**

1. A PARITY.md row under the crate, and a QUIRKS disposition naming Rule
   7, as Rule 10 asks of a divergence.
2. A test showing Go's behaviour (the funds held until the restart or the
   unlock), beside the test pinning dcr-purse's; the differential
   excludes exactly those inputs and says so (Rule 10).
3. The owner is consulted only where a client could observe the
   difference in a reply; a case whose difference no reply shows is fixed
   without asking. The owner settled two points the same day, each as
   suggested. A difference counts when the outcome of a call itself
   changes: a call that fails in Go succeeds, or the reverse. A change
   only in which outputs `listlockunspent`, `listunspent` and their gRPC
   counterparts list, which nearly every fix of this class causes
   (QK-0178, QK-0179; spec 07), does not send the case to the owner; the
   PARITY.md row still records it. And while a case waits for the owner's
   ruling the code applies the fix, since that is the default. How such a
   case is brought is not in the owner's words; proposed, the owner's to
   confirm: as Rule 11 brings one, the proposed disposition in the QUIRKS
   entry, marked "pending owner", named in the change submitted for
   signing. *Confirmed as proposed (project owner, 2026-11-22); addendum
   of that date.*

**Entries met so far.** No QUIRKS entry has a disposition reproducing such
a defect by default, so no disposition changes. QK-0279's disposition
gains a pointer to this addendum. Two cases without a disposition may
fall in the class. QK-0203: `AbandonTransaction` does not release locked
input outpoints (read from the source; not run); it is decided in Phase 5,
which ports that code, after an oracle run. QK-0282 (fee-payment jobs left
idle) together with spec 11 Q6's second half and Q38: when `Process`
swallows a `feeaddress` or fee-transaction error, the purchase loop keeps
that ticket's fee credits locked for the job's reconcile retry, and until
a restart if that retry leaves the job idle (Q14, QK-0282). No QUIRKS
entry yet records that credit leak (QK-0278 records the swallowed errors,
QK-0279 only Q6's first half); it is decided in Phase 9, which ports that
code, after an oracle run. Entries that hold something other than funds
until a restart are not in the class: QK-0340 and QK-0609 (a created
wallet cannot mix until restarted), QK-0362 (`peers.json` not written
again), QK-0608 (certificates read once). Defects that leave state wrong across a restart, such as
QK-0734 and QK-0735, stay under the first half of Rule 7 or Rule 1, as
their dispositions say.

**Places amended with this addendum:** the Status line, the Decision
preamble, Rule 7, "Relation to D13" and Open point 3 above, by dated
pointers; brief §3 (last paragraph) and §9 D10; `docs/PARITY.md` (closing
paragraph of "Deliberate divergences"); `docs/QUIRKS.md` (header and
QK-0279); `README.md`; ADR-0017 (definition (c) and Open point 8); the
ADR index.

**Not settled by the owner's words, and the owner's to decide:**

- Whether a fix under this default is security-relevant under ADR-0017's
  clause (c), which admits "a Rule 6 or Rule 7 divergence" and describes
  the funds case as funds "lost as W1 defines it". Such a fix is a
  candidate under "Relation to D13"; no entry has been added to ADR-0017's
  S-list for it. *Decided 2026-11-20 (project owner): it is not; see the
  addendum of that date.*
- Whether brief §3 W1 itself should cover these funds; the decision
  amends this policy only. *Decided 2026-11-22 (project owner): W1's
  text stays as it is, and the class is governed by Rule 7's default
  alone; see the addendum of that date.*

## Addendum, 2026-11-20 — Decisions of the project owner: Rule 7 fixes and D13, and QK-0035

On 2026-11-20 the project owner settled two points about Rule 7. The text
above stays as written, apart from the dated pointers added to it.

1. **A fix under Rule 7's default is not a security-relevant candidate.**
   The first point the addendum of 2026-11-19 left to the owner ("Not
   settled") is decided: a defect that leaves funds unavailable until a
   restart or a manual unlock, fixed by default under Rule 7's second half,
   does not fall under ADR-0017's clause (c). It needs no attacker and
   affects only the wallet's own funds. So the test showing Go's behaviour
   that the procedure's item 2 asks of each such fix stays in the
   repository, and the rule keeping reproducers out of it (the addendum of
   2026-11-19 on the route of upstream reports, item 2) does not reach it.
   Such a fix is still a Rule 7 divergence, and so a candidate under the
   first clause of "Relation to D13", passed on by the owner like the
   other defects; no entry is added to ADR-0017's S-list (ADR-0017,
   addendum of 2026-11-20, item 2). The second point left open, whether
   brief §3 W1 itself should cover these funds, is not decided.
   *Decided 2026-11-22 (project owner): it should not; addendum of that
   date.*
2. **QK-0035, first half: reproduced (Rule 1), not a Rule 7 case.** An
   `Unlock` with a wrong passphrase on an unlocked wallet locks it, as
   dcrwallet documents (`wallet/udb/addressmanager.go:1510-1516`) and
   tests (`wallet/locking_test.go:61-85`, `testLockOnBadPassphrase`). The
   owner ruled it dcrwallet's intended behaviour, not a defect, so Rule
   7's second half does not apply to it and it is reproduced, as
   `purse-udb` already does. Its disposition was "pending owner" under
   Rule 11; QUIRKS.md and PARITY.md now record the ruling. The entry's
   second half (accounts with their own passphrase locked by the wallet
   lock) is unaffected: it waits for per-account passphrases.

**Places amended with this addendum:** the Status line and "Not settled"
above, by dated pointers; `docs/QUIRKS.md` (QK-0035); `docs/PARITY.md`
(`purse-udb`); ADR-0017 (its addendum of the same date); the ADR index.

## Addendum, 2026-11-21 — The digit search of the wiped base58 codecs

Decided by the project owner on 2026-11-21, as set out at the end; no
code changes with this record.
The review of the dcroxide pin move of 2026-11-21 (`docs/SKEW.md`,
2026-11-21 section, which does not mention it) noted a possible timing
question that no record held.

`purse-hdkeychain` and `purse-dcrutil` decode extended keys and WIF
strings with codecs of their own, over fixed buffers that are wiped,
because `dcroxide-base58` wipes neither its working limbs nor what it
returns (`crates/purse-hdkeychain/src/base58.rs`,
`crates/purse-dcrutil/src/base58.rs`). Each finds a character's digit
with a linear search of the alphabet (`digit_of`,
`ALPHABET.iter().position`), which stops at the match, so its time grows with the digit's value. When
the string is an extended private key or a WIF private key, those digits
are the private key's. decred/base58 v1.0.6, which dcrwallet links, looks
each character up in a 256-entry table (`b58[r]`), and `dcroxide-base58`
has done the same since `48f5b5e`; a table lookup by a secret index is a
data-dependent memory access too, but not a loop whose length follows the
digit. `purse-hdkeychain`'s crate documentation ("Timing") says
that the extended-key codec "branches on its digits, as Go's does" and
makes no constant-time claim for it.

Not examined: which callers pass private strings to these decoders and
what a timing observer could measure of them; no timing of the codecs
was taken, on either side.

**Decided (project owner, 2026-11-21):** the replacement of the digit
search is scheduled, at low priority. The search is to be replaced with a
lookup that reads all 58 entries and selects the match in constant time,
which changes no output and so is no divergence; a separate code change,
with the codecs' existing equivalence tests against `dcroxide-base58` and
a dudect-style check of its own. It is to land before anything that
parses private strings ships. The change is not made, and nothing of the
"Not examined" paragraph above has been examined since.

**Note, 2026-11-22: the replacement is made.** Both codecs
(`crates/purse-hdkeychain/src/base58.rs`,
`crates/purse-dcrutil/src/base58.rs`) find a character's digit with
`digit_plus_one` and, in the encoders, which indexed the alphabet by
each digit (`ALPHABET[d]`, as decred/base58 v1.0.6 does,
`base58.go:147`), a digit's character with `char_of`. Neither reads the
alphabet. Each computes, from the bounds of the alphabet's six runs of
consecutive characters (`1`-`9`, `A`-`H`, `J`-`N`, `P`-`Z`, `a`-`k`,
`m`-`z`), masks that say which run the character is in or which runs
the digit is past, using subtractions and shifts only; packs them into one
word; passes that word through `core::hint::black_box`; and only then
selects with them by AND and OR. A compile-time assertion checks that
the runs spell the alphabet. This differs in method from the decision
above, which named a lookup that reads all 58 entries and selects the
match; it is the same in effect (no output changes, no time that
depends on the character) with six masks in place of 58 and no read of
the alphabet. Whether that substitution stands is the owner's to
confirm. *Confirmed (project owner, 2026-11-22, later): the substitution
stands; addendum "2026-11-22 (later)".*

The barrier sits on the masks for a reason found in review. The first
form of the change, a masked scan of all 58 entries with only the
alphabet behind `black_box`, was branch-free in the workspace's release
build on x86_64 only because the compiler vectorised the scan: the
compiler still read each mask as a comparison and each masked AND as a
select, and at opt-level "s", and in the crate's object code before
linking, it compiled the select into a conditional branch on the
secret character (the decoder's per-entry `je`, and a 58-way
compare-and-jump chain on the digit in the encoder). The timing check
then failed at "s" by several hundred nanoseconds an operation. With the
masks behind the barrier, nothing after it is a select at all; before
it, the compiler may still form a mask with a compare and a conditional
move (it does at "s"), which takes the same time either way, and the
disassembly check below is what shows that no branch came of it. The
barrier also keeps the loops scalar.

The decoder now maps the whole string before its arithmetic and returns
nothing afterwards if any character was outside the alphabet, where it
stopped at the first such character before. No output, error or order
of errors changes: each decoder returns nothing for exactly the inputs it
did, and its callers are untouched. So this is no divergence, and
`docs/PARITY.md` has no row.

What is constant time, and on what evidence: on x86_64, with the pinned
compiler, both lookups compile to code with no conditional branch and no
memory access that depends on the character or the digit, and the
big-number arithmetic to code with no division instruction, at
opt-levels 1, 2, 3, "s" and "z", by the disassembly check below at each
level on an x86_64 Linux desktop; and the timing check found no difference at
opt-levels 3 and "s" on the same desktop, in runs that are not recorded.
No other architecture or compiler was examined, and the property is the
compiler's output, not the source's: a consumer building these crates
another way gets code these checks did not see, which the crates'
documentation says. What still takes a time that depends on the input, as
each module's documentation ("Timing") sets out: the length, which is
public; whether a string holds a character outside the alphabet, an
early exit kept because the result says the same; the count of leading
`1` characters, which encode leading zero bytes, none in any extended
key or WIF of the four networks, whose version and network bytes are not
zero; and the big-number arithmetic, whose limbs (decoding) and digits
(encoding) grow with the magnitude of each prefix of the value, which the
public leading fields set except through a carry near a boundary. The
rest of a parse (the checksum comparison, the field checks, a private
key's public key) was not examined with this change.

The encoders' division by 58. Review found that each encoder's
`val / 58` and `val % 58`, on a dividend that depends on every payload
byte before it (the chain code and key of an extended private key, the
key of a WIF), compiled at opt-level "z" to a hardware divide (`div`)
on x86_64, inside the digit loop of `ExtendedKey::string` and
`Wif::string`; at levels 1, 2, 3 and "s" the compiler turned it into a
multiplication, as the modules' documentation then said of "an optimised
build" without naming a level. A divide's latency depends on its
operands on several x86-64 and AArch64 cores; that a leak is measurable
here was inferred from published latencies, not shown. Each encoder now
divides with `divmod58`, a multiplication by 18,079 (ceil(2^20 / 58)) and
a shift for the quotient and a multiplication and a subtraction for the
remainder, exact below 2^17 against the encoder's largest dividend of
14,847 (a compile-time assertion), and tested against `/` and `%` on
every value below 2^17 (`divmod58_matches_division`). The digit loop is
kept out of line as `value_digits`, and each decoder as a whole, so that
the disassembly check reads them and fails on any division instruction;
with the plain division restored in a scratch build, the check failed in
both crates at "z" (two `div` each) and passed at 1, 2, 3 and "s",
which is the gap it closes. The arithmetic's other operations (additions, subtractions,
shifts, 32- and 64-bit multiplications) take a time that does not depend
on their operands on x86-64 and AArch64 cores by those cores'
documentation; that was not measured, and a 32-bit target, where the
decoder's 64-bit multiplication is several instructions or a call, was
not considered. The encoder's `len * 137 / 100`, which sizes the WIF
encoder's buffer, divides the public length and stays.

The checks. In each crate, `lookups_match_the_search_for_every_byte`
compares both lookups, one value at a time and through the slice maps
the codecs call, with the old search over all 256 byte values; the
equivalence tests against `dcroxide-base58`, the oracle differentials,
the frozen vectors and the `hdkeychain_parse` fuzz target are unchanged
and pass; a fuzz target for WIF strings, `wif_decode`, is added, since
that parser had none. Each crate's `src/ct_check.rs` holds ignored
targets of two kinds:

- `disassembly`, on `purse-testutil`'s `disasm` module (new): the test
  binary's own code, read with GNU binutils, for a probe of each lookup
  with no loop around it, which must have no conditional jump, no call
  and no memory access but a fixed address or an unindexed stack slot,
  and for the two loops the codecs call (`map_digits`, `map_chars`, kept
  out of line for it), which must have no call, take no table's address,
  and jump conditionally only backward or out of every loop; and for the
  big-number arithmetic (`value_digits` and each decoder), which may
  branch on the value's magnitude and call, and must have no division
  instruction, the one rule every shape of the check now has. dcroxide
  has no such check; it is added because the timing test cannot see a
  table load (below). Each of four regressions, built in a scratch copy,
  fails it at both opt-levels 3 and "s": a linear search like the
  decoder's old one, the first form's masked scan, the encoder's old
  `ALPHABET[d]` load, and the first form's masked scan in the encoder.
  Removing the barrier makes the compiler vectorise the decoder's loop
  at opt-level 3, whose length dispatch the loop rule rejects; that
  failure is the rule's, not a leak, and at "s" the form without the barrier passed. So the barrier
  was not shown necessary for the arithmetic form with this compiler; it
  is kept because the first form showed what the compiler may do with an
  unguarded select. The crate's object code before linking, emitted at
  opt-levels 3 and "s", was also read for the two loops: their only
  conditional jumps are a length guard and the back edge (at "s",
  `map_digits` there also calls the iterator adapter that pairs the two
  slices, on their lengths alone, which linking inlines). The loop rule
  is a heuristic on layout: a branch on the data laid out as a back edge
  or an exit passes it, and loop structure it does not know (an unrolled
  remainder, a vectorised loop) fails it.
- dudect's fixed-against-random timing test, following dcroxide's
  method: a reduced port of its harness (`crates/purse-testutil/src/ct.rs`;
  the workspace does not link `dcroxide-testutil`), on the lookups, the
  decoder and the encoder against random secrets, with the old search as
  a positive control that must be found. Only `digits` reliably finds a
  variable-time decoder lookup: the old search as it was written passed
  `decode`, while the same search written another way failed it. `chars`
  and `encode` cannot see the encoder's old form, a load from the 58-byte
  alphabet indexed by the digit, which stays in the first-level cache:
  with it restored both passed. That is why the disassembly check exists.

A new job of `.github/workflows/ci.yml` runs the timing targets and the
disassembly at the release profile and again at opt-level "s", 10 s a
timing target, pinned to one processor, failing as dcroxide's job does
on |t| above 10 with a difference of 1 ns an operation or more, and the
disassembly alone at opt-levels 1, 2 and "z". Whether the timing steps
stay free of false failures on shared runners is not established: the
targets were run only on an x86_64 Linux desktop, at 2 s, 5 s and 10 s a
target, as smoke tests of the harness; in the runs of the final form every
target passed at 3 and at "s" and both controls were found, as they
did again, at 10 s a target, after the encoders' division was rewritten.
None is recorded. There is no timing of record and no ledger entry.
*2026-11-22, later (project owner): the timing steps moved out of that
job into a workflow of their own, `.github/workflows/constant-time.yml`,
run on a change to the paths that can move their result and nightly; the
job keeps the disassembly, now at all five opt-levels in steps of its
own; addendum "2026-11-22 (later)".*

Speed, unrecorded, on the same desktop. The final form against the
parent commit, in six interleaved `purse-bench run` sessions each,
Rust alone, pinned to one processor at a load average near 2:
`hd.xpub_string` about 6.5% slower and `hd.parse_xpub` about 1% quicker
(medians). The barrier keeps the encoder's character pass scalar, under
3 ns a character more than the parent's table load; the old search's
branch on every character cost the decoder about as much as the
arithmetic does. The CI gate's step was run
locally on this tree, unrecorded, as follows. The first form was run
five times and passed three: one run at a load average near 100 failed
on seven rows (`hd.pub_child`, `hd.addr20k`, `crypto.hmac_sha512`,
`crypto.hash160`, `crypto.base58check_decode`, `snacl.seal_32`,
`snacl.seal_111`) and one near 46 on `hd.addr20k` alone, none of which
uses these codecs, and in all five the two extended-key rows were within
their limits. The final form was run twice at a load average near 2
and passed both (`hd.xpub_string` 1.011 against its baseline of 0.955,
limit 1.910, and `hd.parse_xpub` 0.547 against 0.576, limit 1.152, in
the first). After the encoders' division was rewritten (above) the step
was run once more, at a load average near 3, and passed
(`hd.xpub_string` 1.024, `hd.parse_xpub` 0.547); the interleaved
comparison with the parent commit, and its 6.5%, predate that change and
were not repeated. The "Not examined" paragraph above stands.

## Addendum, 2026-11-22 — Decisions of the project owner: how a Rule 7 case is brought, and W1

**What was put.** The two points of D10's record that the owner had not
settled and the record defers to no phase, each with the recommendation
of the Phase 0 exit check ([PHASE0-EXIT.md](../PHASE0-EXIT.md), points 7
and 8 of the eight it put to the owner). Point 7: item 3 of the procedure
in the addendum of 2026-11-19, marked "proposed, the owner's to confirm":
a Rule 7 case whose difference a client could observe is brought as Rule
11 brings one. Recommended: confirm as proposed. Point 8: the second
point under "Not settled" in the same addendum, whether brief §3 W1
itself should cover funds a defect leaves unavailable only until a
restart or a manual unlock, left to the owner with no proposal.
Recommended: W1's text stays as it is and the class is governed by Rule
7's default alone. W1 is the surface where a failure loses money; these
funds are not lost, and Rule 7 already fixes every such defect with a
PARITY.md row and a test of Go's behaviour, so widening W1 would change
no code.

**Decision.** On 2026-11-22 the owner, shown the eight points of that
check with their recommendations, answered "Accept all eight". For this
record that is both points, each as recommended. The text above stays as
written, apart from the dated pointers added to it.

1. **How a Rule 7 case is brought.** A defect under Rule 7's second half
   whose difference a client could observe in a reply goes to the owner
   as Rule 11 brings a case: the implementer writes the proposed
   disposition into the QUIRKS entry, marked "pending owner", and names it
   in the change submitted for signing. Unlike Rule 11's own cases, the
   code applies the fix while the case waits, since the fix is the default
   (procedure item 3, settled on 2026-11-19).
2. **W1.** Brief §3 W1's text stays as it is. Funds a defect leaves
   unavailable only until a restart or a manual unlock are governed by
   Rule 7's default alone; "It does not widen W1" (addendum of 2026-11-19)
   stands as the reading of both texts. The question was about the brief,
   not about this policy, and the answer changes neither.

**What follows.** No QUIRKS entry or PARITY.md row changes: no case of
the class is pending, and the procedure's items 1 and 2 are unchanged.
D10's record has no point marked Proposed or left to the owner without a
phase, apart from the owner's standing discretion over what, if
anything, is run before a report is passed on (addendum of 2026-11-19 on
the route of upstream reports, item 1), which is not a pending decision.
What it defers stays deferred: QK-0203 (Phase 5) and the fee
credits of QK-0282 (Phase 9), each decided after an oracle run (addendum
of 2026-11-19, "Entries met so far"), and the replacement of the base58
digit search, scheduled at low priority and not done (addendum of
2026-11-21). *Done later on 2026-11-22: the note of that date at the end
of the addendum of 2026-11-21.*

**Places amended with this addendum:** the Status line (a dated bullet);
procedure item 3 and the second point under "Not settled" in the addendum
of 2026-11-19, and the last sentence of item 1 of the addendum of
2026-11-20, by dated pointers; the brief (§3, last paragraph, and §9
D10); the ADR index; `docs/PHASE0-EXIT.md`.

## Addendum, 2026-11-22 (later) — Decisions of the project owner: the base58 lookup's form, and where its timing check runs

**Decision.** Later on 2026-11-22 the project owner accepted every
recommendation then open. For this record that is two points, both
about the addendum of 2026-11-21 and its note of 2026-11-22.
The text above stays as written, apart from the dated pointers added to
it. The other points of the same answer are rulings on single cases
under Rules 6, 7, 9 and 11 (QK-0760's residual case, QK-0761, QK-0762,
QK-0766 to QK-0769, QK-0780, QK-0781, the processor count of Rule 9(b))
and choices of `purse-udb`'s transaction store; as with QK-0744 and
QK-0035 before them, each is recorded in its QUIRKS entry, in
`docs/PARITY.md` and in the crate's documentation, not here.

1. **The lookup as built stands.** The decision of 2026-11-21 named a
   lookup that reads all 58 entries of the alphabet and selects the
   match in constant time. The replacement made on 2026-11-22 computes a
   character's digit, and a digit's character, from the bounds of the
   alphabet's six runs, with masks behind `core::hint::black_box` and no
   read of the alphabet (the note of that date). It is the same in effect
   (no output changes, no time that depends on the character), and the
   form the decision named, a masked scan, was the one the review found
   compiled into a branch on the secret character at opt-level "s". The
   note's "Whether that substitution stands is the owner's to confirm" is
   answered: it stands. No code changes with this point.
2. **The timing check runs in a workflow of its own.** dudect's targets
   (ten, at opt-levels 3 and "s") ran in `ci.yml`'s constant-time job on
   every push, beside the disassembly check. They now run in
   `.github/workflows/constant-time.yml`, as dcroxide's "Constant time"
   workflow runs its own: on a push to `master` or a pull request that
   changes a path that can move their result, nightly, and on dispatch.
   The paths are the two codec crates (`crates/purse-hdkeychain`,
   `crates/purse-dcrutil`, neither of which depends on another crate of
   this workspace but `purse-testutil`, a dev-dependency of both),
   `crates/purse-testutil` (the harness and the generator that draws the
   classes), `Cargo.lock` (a dependency's version, dcroxide's revision
   among them), `rust-toolchain.toml` (the compiler), the root
   `Cargo.toml` (its release profile; a path filter cannot narrow a file
   to one table, so any change to the manifest runs the workflow) and the
   workflow file itself. Each target runs for 10 s on pushes and pull
   requests, as in `ci.yml` before, and for 60 s nightly, as dcroxide's
   nightly does; a dispatch may set 1 to 90 s, the cap dcroxide's
   workflow puts on a dispatch of every target. The rule a target fails
   by, the pinning to the runner's last processor, the build before each
   timed step and the two opt-levels are unchanged. The disassembly check,
   which is deterministic and takes seconds a level, stays in `ci.yml`'s
   constant-time job on every push, now in steps of its own at opt-levels
   3, "s", 1, 2 and "z" (before, its runs at 3 and "s" came with the
   timing steps' filter); the job is renamed "constant time, disassembly".

What this leaves as it was: whether the timing steps stay free of false
failures on shared runners is still not established, and a change to a
path outside the filter that could move a timed binary would not run the
workflow until the next nightly run; the workflow has not run on GitHub.
Its commands, and the disassembly steps, were run on an x86_64 Linux
desktop as a smoke test that is not recorded, as were the targets before;
there is no timing of record and no ledger entry.

**Places amended with this addendum:** the Status line (a dated bullet);
the note of 2026-11-22 under the addendum of 2026-11-21, by two dated
pointers; `.github/workflows/ci.yml` and the new
`.github/workflows/constant-time.yml`; the module documentation of each
crate's `src/ct_check.rs`; `purse-dcrutil`'s crate documentation, which
gains a section on what of a WIF's handling is constant time; the ADR
index; `docs/PHASE0-EXIT.md`, whose list of CI's jobs carries a dated
pointer.

## Addendum, 2026-11-24 — Diagnostics for the codecs' timing check

**The failure.** The timing workflow (`.github/workflows/constant-time.yml`,
addendum "2026-11-22 (later)") failed once on GitHub, on `f925cc2` at
opt-level 3, on a runner whose processor was an AMD EPYC 9V74 (four
processors, two threads a core, under a hypervisor):
`hdkeychain chars` found all 0 quicker than the random class by 1.84 ns
an operation at t -59.66, the comparison that decided it in crop 2
(nominally the warm-up's quickest 18.8%), which held 20,288 of the class's
samples and 13,928 of the random class's, of about 730,000 each in the
run. In the same crop all 57 crossed |t| 10 on a difference of the other
sign, +0.28 ns (slower) at t 10.64, and was reported under the floor;
that crop held 19,583 of its samples, against the random class's 13,928.
The placebo passed (-0.03 ns at t -1.59), as did every other target, and
both controls were found. In the run before, on `f1efb01` at opt-level
"s" on an EPYC 7763, the same target reported all 0 under the floor,
+0.92 ns at t 100.63 in crop 75, and all 57 under the floor too, +0.88
ns at t 38.24 in crop 88, while the placebo passed (-0.21 ns at t
-1.51). No conclusion is drawn from all 57's figures here; they are
recorded because the diagnostic pass below asks the same question of
other inputs.

What was established about it, and what was not. Reading the machine
code of the encoder's character pass found no instruction path, branch or
memory access that depends on a digit; its instruction sequence is the
same in both crates; and on a Zen 4 x86_64 Linux desktop the hardware
counters gave every class equal retired instructions, branches, branch
mispredictions, loads and stores an operation. The magnitude was not
reproduced off GitHub: about 120 runs on that desktop gave no failure,
with differences about 0.03 ns an operation. The cause is not
identified. The failure's magnitude, its sign and the imbalance of its
deciding crop do not tell a leak from an artefact of the machine or the
harness: leaks of 1 to 2 ns an operation planted in a scratch copy give
the same shapes, a planted slower path among them deciding a run as a
quicker class in a thin crop. That crop 2 held 1.9% to 2.8% of each
class's samples, against its nominal 18.8%, is in the log: most of the
run's samples were slower than the warm-up's quantile, whose thresholds
come from the first 1,000 samples alone. Whether those thresholds decided
the verdict cannot be shown, since the log holds no data per sample.

**Decided (project owner, 2026-11-24): diagnostics only.** The harness
(`crates/purse-testutil/src/ct.rs`) and the two `chars` targets gain
output that lets the next failure explain itself, and nothing that
decides a verdict changes: |t| 10 to fail and 4.5 to report,
the floor of 1 ns an operation, the 100 samples a side, the warm-up
(1,000 samples, or a tenth of the budget), the crops and their
thresholds, the rule that picks the comparison deciding a class, the
budgets (10 s on pushes and pull requests, 60 s nightly, 1 to 90 s on
dispatch), the absence of retries and the job's blocking status are as
they were. No threshold, floor, budget, retry or blocking change.

1. **The trace.** The port of dcroxide's harness left out its
   per-target trace (dcroxide `8053abe`), as more than the targets then
   needed; it is ported. Every target prints, below its class lines and
   whatever its verdict: the samples and the warm-up's, the address of
   one of the harness's functions and of the first chunk of inputs, the
   processors the thread may run on (`Cpus_allowed_list`) and, new
   against dcroxide, the processor's name (the first `model name` of
   `/proc/cpuinfo`, `unknown` without one); the random class's mean in
   the uncropped test, in crop 32 (89.8%) and in six equal segments of
   the counted time; each class's t and difference uncropped and in crop
   32, whatever their |t|, and its t in each segment; and, also new, the
   samples crop 32 actually holds (of each class, of the random class
   and of every class together, as counts and percentages of the run's)
   beside its nominal share, and, for the comparison that decides each
   class, the test's nominal share and threshold beside the samples of
   the class and of the random class in it, each as a count and a
   percentage of that class's samples in the run, and the percentage of
   every class's together, so that a verdict decided in a thin tail, or
   a crop 32 that holds one, shows as such. The segments are told apart
   between chunks of samples, to the nearest chunk (one chunk is about a
   millisecond for the `chars` targets and up to about 16 ms for the
   `decode` and `encode` targets, measured at opt-level 3: at most about
   1% of a segment at the push budget), so that the loop over a chunk's
   samples counts each sample as it did before; a first form that also
   filed each sample in its segment inside that loop is described below.
   Known reports stay out: no target here has a failure measured and
   judged as one.
2. **A diagnostic pass of the `chars` targets.** Each `chars` target
   (purse-hdkeychain's and purse-dcrutil's) gains a test of its own,
   `chars_diagnostic`, which the workflow's filter runs right after
   `chars` in the same step, for the same budget: a second pass of the
   target's three classes and two more, reported under the target's name
   followed by ", diagnostic pass", printed and never judged, so that it
   cannot fail the job. The two are a fixed draw, one input of digits
   drawn once from the harness's generator, spread over the digits as the
   random class's are; and all 0 again, all 0's value under a name of its
   own (the duplicate of dcroxide `c582c7c`; the placebo it also added is
   already in these targets). `chars` itself, the verdict's pass, is as
   it was at `05f7862`, source for source. If a difference follows the
   repetition of one input, the fixed draw leans with all 0; if it
   follows the value, the two copies of all 0 lean together and the fixed
   draw does not; if it follows neither, the placebo or one copy alone
   says so. The pass times its own window after the verdict's, with its
   own warm-up and crop thresholds, so it shows only a difference that
   lasts into that window: of a passing event such as the one on
   `f925cc2` it may show nothing, and there the verdict pass's own trace
   (item 1) is what is left.

**Shares.** In `chars`, the random class and each of the three fixed
classes keep a share of 1/4, as before (measured from the trace: medians
of 24.98% to 25.03% for each of the four classes in both targets). In the
diagnostic pass each of the six has 1/6 (16.64% to 16.69%). Timing the
two classes in `chars` instead would have cut every share there from 1/4
to 1/6: at a fixed budget each class would get two thirds of its samples
(measured: all 0 had 25.0% of `hdkeychain chars`'s samples and 16.6% in
that arrangement, medians of 30 runs each), and t for a given difference
falls by the square root of that, to about 0.82 of itself, both sides of
each comparison losing alike. That arrangement was measured and
rejected: in a scratch test function with `hdkeychain chars`'s classes,
not the shipped target, a planted secret branch was found in 2 of 10
runs with the two classes in the verdict's pass, against 10 of 10 with
the verdict's pass as it is, its eight misses all below the floor (the
added copy of all 0 also changes how often the planted branch goes each
way, and with it what the plant costs).

**Power, measured through the shipped targets.** Three leaks were
planted in each crate's `map_chars`, each keyed on the first digit being
0, so that all 0 has it and the random class once in 58: one dependent
multiplication on every lookup's path, one 64-bit division whose
dividend is large for the secret (a variable-latency instruction), and a
branch on the secret whose two sides do the same work. Each was put in
the shipped `chars` target itself, whose operation called the planted
function under `#[cfg(test)]` with the plant chosen at run time, in a
copy of `05f7862`'s tree and in a copy of this change's, the same hook
in both. Each was timed ten times at the push budget of 10 s, before and
after interleaved, each run pinned to one processor with `taskset`, five
runs at a time on separate cores while other jobs ran (load averages 3.4
to 26.9); a target counts as found when one of its three judged classes
fails by the rule. In brackets, the median difference of all 0 in the
comparison that decided it, in nanoseconds an operation. These tables
give the first sets of runs; the two multiplication cells at opt-level 3
were timed again in later sets, given under "Later sets" below, and
their counts moved from set to set on the same machine code:

| target and plant, opt-level 3 | `05f7862` | as shipped |
| --- | --- | --- |
| `hdkeychain chars`, dependent multiplication | 30 of 30 (+1.60) | 15 of 20 (+1.04) |
| `hdkeychain chars`, variable-latency division | 10 of 10 (+2.00) | 10 of 10 (+1.50) |
| `hdkeychain chars`, secret branch | 29 of 30 (+1.23) | 17 of 20 (+1.13) |
| `dcrutil chars`, dependent multiplication | 30 of 30 (+1.17) | 20 of 20 (+2.00) |
| `dcrutil chars`, variable-latency division | 2 of 10 (+0.11) | 1 of 10 (+0.08) |
| `dcrutil chars`, secret branch | 0 of 10 (+0.41) | 0 of 10 (-0.08) |
| no leak, each target | 0 of 10 | 0 of 10 |

| target and plant, opt-level "s" | `05f7862` | as shipped |
| --- | --- | --- |
| `hdkeychain chars`, dependent multiplication | 10 of 10 (+2.45) | 10 of 10 (+2.90) |
| `hdkeychain chars`, variable-latency division | 10 of 10 (+1.14) | 10 of 10 (+1.07) |
| `hdkeychain chars`, secret branch | 10 of 10 (+1.54) | 10 of 10 (+2.04) |
| `dcrutil chars`, dependent multiplication | 10 of 10 (+2.75) | 10 of 10 (+2.65) |
| `dcrutil chars`, variable-latency division | 10 of 10 (+1.29) | 10 of 10 (+1.31) |
| `dcrutil chars`, secret branch | 10 of 10 (+1.92) | 10 of 10 (+2.08) |
| no leak, each target | 0 of 10 | 0 of 10 |

The three cells of the first table nearest the floor (the multiplication
and branch plants in `hdkeychain chars`, the multiplication plant in
`dcrutil chars`) were timed in a second set of ten runs each, before and
after interleaved, and `05f7862`'s binary ten more times beside the
variant described below; the first table gives the sets together. In the
eighty runs with no leak every judged class passed, none even reported.
At "s", 19 of the 120 finds were decided as all 0 quicker in a thin
crop (14 with `05f7862`'s harness, 5 as shipped), and at opt-level 3, 10
of the 164 finds.

The controls were found in every one of these runs. The finds of the
division plant in `dcrutil chars` (two with `05f7862`'s harness, one as
shipped) were each decided as all 0 quicker, by 1.03 to 1.62 ns in a thin
crop, a planted slower path read as a quicker class, as described under
"The failure"; on 53 digits that plant costs about a tenth of a
nanosecond, and the branch plant less than half of one.

**Later sets.** The two multiplication cells at opt-level 3 were timed
again in further sets of ten runs, with the binaries of the first sets
(or a second build of each tree whose loaded sections, its machine code
included, are byte for byte the same, apart from the build ID), run the
same way: 10 s, pinned with `taskset`, before and after interleaved,
other jobs running. The sets are numbered in the order they ran across
both cells. For each set: runs found, in brackets those decided by the plant's own
slower path (all 0 judged slower) and not by all 0 judged quicker in a
thin crop, then the median difference of all 0 over the runs that judged
it slower:

| `hdkeychain chars`, multiplication, set (load averages) | `05f7862` | as shipped |
| --- | --- | --- |
| first (3.4 to 8.2) | 10 of 10 (10), +1.41 | 7 of 10 (7), +1.04 |
| second (16.5 to 18.7) | 10 of 10 (10), +1.70 | 8 of 10 (6), +1.19 |
| beside the variant below (15.9 to 20.9) | 10 of 10 (8), +1.68 | not timed |
| third (6.0 to 18.1) | 10 of 10 (10), +1.38 | 1 of 10 (1), +0.92 |
| fourth (22.4 to 26.1) | 10 of 10 (10), +1.90 | 4 of 10 (4), +1.00 |
| fourth, the second build | 10 of 10 (10), +1.39 | 9 of 10 (8), +1.26 |
| fifth (22.7 to 26.2) | 10 of 10 (6), +1.60 | 10 of 10 (4), +1.44 |
| all | 70 of 70 (64), +1.56 | 39 of 60 (30), +1.04 |

| `dcrutil chars`, multiplication, set (load averages) | `05f7862` | as shipped |
| --- | --- | --- |
| first, second and beside the variant (6.0 to 20.2) | 30 of 30 (28), +1.17 | 20 of 20 (20), +2.00 |
| third (8.5 to 12.2) | 6 of 10 (6), +1.02 | 10 of 10 (10), +1.87 |
| sixth, the first build (heavy load, about 26) | 16 of 16 (7), +1.18 | not timed |
| sixth, the second build | 16 of 16 (11), +1.25 | not timed |

With the machine code unchanged, run conditions (load, and how the runs
were invoked: on which cores, in what order) moved these near-floor
counts: `05f7862`'s binary found the `dcrutil chars` plant in 30 of 30,
6 of 10 and 16 of 16 runs, and the shipped binary found the `hdkeychain
chars` plant in 1 of 10 in one set and 10 of 10 in another, six of those
ten decided as all 0 quicker. The sets under the heaviest load had the
most finds decided as all 0 quicker (up to 9 of 16), which is not the
plant's own slower path; no run without a plant produced a find of
either sign. A single count, such as the 15 of 20 of the first sets, is
therefore not a stable property of a binary. What held from set to set
is each plant's cost in each binary, measured by the median difference:
in `hdkeychain chars` the shipped binary's is +0.92 to +1.44 ns an
operation a set against +1.38 to +1.90 with `05f7862`'s harness, and in
`dcrutil chars` +1.87 to +2.00 against +1.02 to +1.25.

**Detection lost, and why it is recorded rather than restored.** One
loss remains. The multiplication plant in `hdkeychain chars` at
opt-level 3 costs less in the shipped binary than in `05f7862`'s (+1.04
against +1.56 ns an operation, the medians over every set), close enough
to the floor that it was found in 39 of 60 runs in six sets as shipped,
1 to 10 of 10 a set, 30 of them by the plant's own slower path, against
70 of 70 in seven sets, 64 by its slower path, with `05f7862`'s harness.
In the first sets every miss was under the floor (all 0 at +0.85 to
+0.99 ns, |t| above 10). The branch plant there was found in 17 of 20
against 29 of 30 in the first sets, its three misses at +0.91 to +0.98
ns. The multiplication plant in `dcrutil chars`, lost in the first form
(below), is found in 30 of 30 in three sets (+1.87 to +2.00, against
+1.02 to +1.25 with `05f7862`'s harness). This loss is recorded, not
restored, for the project owner to decide on. *(Decided, project owner,
2026-11-25: accepted as recorded; the note of that date at the end of
this addendum.)*

The first form of this change filed each counted sample in its segment
inside the loop over a chunk's samples, and ran the diagnostic pass
inside `chars`; with it, measured the same way through the shipped
targets, the branch plant in `hdkeychain chars` at opt-level 3 was found
in 9 of 20 runs against 20 of 20 with `05f7862`'s harness (7 of 12
against 12 of 12 in a second check), and the multiplication plant in
`dcrutil chars` in 0 of 10 against 10 of 10, every miss under the floor;
that form was replaced by the one above.

What these measurements show about the cause. The verdict's code is the
same: the functions that decide a verdict are unchanged in source, the
target `chars` is unchanged in source, and a replay of 7,200 simulated
runs through `05f7862`'s harness and this one gave identical rows and
class lines in every run. In the shipped binaries at opt-level 3 the
timed region around the planted function has the same instructions as in
`05f7862`'s, apart from registers, stack offsets, alignment padding and,
in each crate, one operand held in a register instead of on the stack
(or the reverse); what moves is where the code and the loop's stack
buffer lie. A planted cost of one multiplication, or a real one of the
same size, lies within a nanosecond of the floor, and the same plant
gave medians of +1.04 to +2.00 ns across these binaries: any change that
rebuilds these crates' test binaries moves such a cost, up or down. It
is the medians, not the counts, that follow the layout: at a cost this
near the floor, how often a run crosses it also moves between sets of
the same binary, as "Later sets" shows. A second arrangement of the
trace's bookkeeping (a helper kept out of line) was measured on the
three plants nearest the floor and did no better (`hdkeychain chars`
multiplication 7 of 10, branch 4 of 10, `dcrutil chars` multiplication
10 of 10, against 10, 10 and 10 of 10 with `05f7862`'s harness in the
same runs); arrangements were not searched further, since picking one by
its plants' results would fit the layout of this commit's binary and not
of the next.

**Wall time.** The two diagnostic passes add two budgets to each timing
step: run as written, each step took 100.1 s (two runs a level) at 10 s
a target with the harness of `05f7862` and 120.1 to 120.2 s (ten runs a
level) with this change, 20 s more; at 60 s a target that is 2 minutes
more a step, and at a dispatch's 90 s, 3 minutes. The job, about 37
minutes at 90 s, stays inside its 60-minute timeout. In those twenty
local runs every target passed (`dcrutil chars` reported in 7, |t| 4.5
to 5.7 on 0.01 ns an operation), both controls were found in every one,
and the disassembly job's steps passed. By the rule, the diagnostic pass
of `hdkeychain chars` reported all 0, all 0 again, all 57 and the fixed
draw in 8, 8, 9 and 9 of the twenty, on differences of 0.04 ns an
operation or less, and the placebo in none; that of `dcrutil chars`
passed every class.
These counts are one set's: two later sets of twenty steps on the same
code reported `dcrutil chars` in 13 and 15 of the twenty, |t| up to
11.98 on 0.02 ns an operation or less, with no judged target failing
and both controls found in every step.

**What was run, and what was not.** On the same Zen 4 x86_64 Linux
desktop, unrecorded: the harness's tests, which time nothing; the replay
above; the planted leaks and the runs with no leak, as described; and the
timing job's steps and the disassembly job's, as the workflows write
them. Not measured: the plants at a budget of 60 s. None of it ran on
GitHub or on an EPYC processor, and none is a run of record; there is no
ledger entry. Whether the trace explains the next failure, and whether
the failure recurs, is not known.

**Places amended with this addendum:** the Status line (a dated bullet);
`crates/purse-testutil/src/ct.rs` (the trace, the diagnostic pass and
their documentation, which also records why the trace was left out at
first and that known reports stay out); `src/ct_check.rs` in
`purse-hdkeychain` and `purse-dcrutil` (the `chars_diagnostic` tests,
their classes and the module documentation);
`.github/workflows/constant-time.yml` (comments only); the ADR index.

**Note of 2026-11-25 — decided, project owner.** The loss of detection
recorded above under "Detection lost, and why it is recorded rather than
restored" (the dependent-multiplication plant in `hdkeychain chars` at
opt-level 3, found in 39 of 60 runs as shipped against 70 of 70 with
`05f7862`'s harness) is accepted as recorded. No verdict rule changes:
the threshold of |t| 10 to fail and 4.5 to report, the floor of 1 ns an
operation, the warm-up, the crops, the budgets, the absence of retries
and the job's blocking status stay as this addendum leaves them. A
separate, later change is to make the timed operation less sensitive to
code layout, for example an operation that works in place instead of
copying its input to the stack, so that a cost near the floor moves less
from one binary to the next. That change is planned follow-up work: it is
not part of this note, nothing in the harness, the targets or the
workflow changes with it, and none of it has been written or measured.

Places amended with this note: the Status line (a dated bullet, and a
pointer on the bullet of 2026-11-24); the dated pointer under "Detection
lost, and why it is recorded rather than restored"; the ADR index.
