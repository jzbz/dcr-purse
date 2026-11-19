# ADR-0012 — Standing quirk policy

- **Status:** Accepted (decision D10; project owner, 2026-11-18) as
  proposed, with one change: the coins a partial VSP purchase leaves locked
  until a restart (QK-0279) are fixed, not reproduced (Rule 7; Open point
  3). Restated, not re-decided: the
  owner's rulings on the database divergence (2026-11-14,
  `docs/PARITY.md:70-72`), reference Go 1.27.4 (2026-11-18, spec 23 §13
  item 1) and QK-0729 (2026-11-18, `docs/QUIRKS.md:4395`).
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
accepts; Rule 9 is a new class.

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
(`crates/purse-hdkeychain/src/lib.rs:73-91`).

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
   confirmation.

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
