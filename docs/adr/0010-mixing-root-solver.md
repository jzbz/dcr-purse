# ADR-0010 — The mixing root solver

- **Status:** Accepted as proposed (decision D8; project owner,
  2026-11-18), the answers proposed under Open points included: a native
  solver in process, and `--csppsolver` accepted and never executed, which
  the owner approved as an exception to W6 and to the standing quirk
  policy.
- **Date:** 2026-11-18

## Context

**The question.** In StakeShuffle the peers' power sums define a
polynomial over F = 2¹²⁷ − 1 whose roots are the slot-reservation (SR)
messages; sorted ascending, a root's index is its owner's slot (spec 12
§5.9.3). A peer that advertises solving publishes the roots in a
factored-polynomial (FP) message (§5.11.1). dcrwallet spawns an external
program for this. dcr-purse must decide how it obtains the roots and what
`--csppsolver` means. Phase 8 needs the option's meaning for its
Go-generated configuration suite; Phase 11 builds `purse-mixsolver`
(`docs/dcr-purse-project-brief.md` §5, §6). D1-D10 are due at the Phase
0 exit.

**What dcrwallet does at the pin** (spec 12 §5.11; spec 21 §5.2 step
22). Under `--mixing` only, an explicit `--csppsolver` makes the solver
mandatory; otherwise `csppsolver` is tried from `PATH`, and failure logs
`Unable to start csppsolver; must rely on other peers publishing
results`. A mandatory solver that fails to start or fails a probe call
fails start-up with `csppsolver process is not operating properly:
<err>` (`config.go:739-758`, `testsolver.go:18-35`); spec 13 §9 item 18
marks this compatibility-critical. A started solver sets pair-request
flag bit 0 (`mixclient/client.go:414-419`); a session with no flagged
request stops after the key exchange (spec 12 §5.7 step 4). The program
serves Go `net/rpc` with `gob` over stdio and calls FLINT's
`fmpz_mod_poly_factor` through cgo, with `/usr/local` added to the
include and library search paths (`cspp/v2@v2.4.0/solver/solver.go:3-13`):
it links whatever FLINT the build host has. A dead child yields
`ErrSolverProcessExited`; the wallet stops advertising and keeps running
(`solverrpc/rpc.go:159-162`, `client.go:2163-2172`). Other errors, or a
root count other than `mtot`, set `triggeredBlame`; otherwise the sorted
roots go into an identical, unchecked FP per local peer
(`client.go:2173-2197`; QK-0325).

**What the solver computes.** Input: `mtot + 1` canonical coefficients,
leading coefficient F − 1 (spec 12 §5.9.3). FLINT reports `mtot` factors
of exponent 1 exactly when the polynomial has `mtot` distinct roots in
F, so the accepted set and the sorted output are fixed by the
mathematics. Nothing secret enters: coefficients and roots are
broadcast, and the wallet's own slot is found afterwards in constant
time (`client.go:2585-2606`). A run PRNG, which blame replays, must not
be touched (§4.8).

**Failure modes.** Colliding SR values blame both owners; a wrong SR mix
makes the polynomial almost surely not split, and blame finds `bad SR
mix` (§5.12 steps 10-11). A solver that fails on an honest polynomial
sends a reveal instead of a DC. Every peer holding roots, its own or
from an FP, then blames that wallet for `missing messages` (§5.12 step
13; `blame.go:414-427`). Only when no other peer obtained roots does the
fallback blame it, for a false failure accusation (§5.12 step 12,
"Fallback"). Either way it is excluded. Derived from reading.

**Size and time bounds** (spec 12 §5.5, §6).

| Bound | Value |
|---|---|
| peers; messages per peer | 4 to 64; at most 16 |
| `mtot` (the degree) after size limiting | at most 1024 |
| `mtot` whose SR can be encoded | at most 512 (Q8; UNVERIFIED by execution, §13 item 3) |
| stage timeout T; `maxJitter` | 30 s, 3 s; testnet3 10 s, 1 s |
| `recvSR`; `recvDC` | begin + 3T; begin + 4T |

The roots are sought under a context ending at `recvSR`; DC and FP are
then sent together after a delay of up to `maxJitter`
(`client.go:1918-1956`, `:599`, `:727-728`). Two budgets follow. Other
peers wait for the wallet's DC until `recvDC` (`:1968`). Go peers without
a solver wait for an FP only until `recvSR`, then spin (§5.11.1;
QK-0315): last SR + solve + `maxJitter` must end before `recvSR`. After
`recvSR`, Go creates each local FP, and marks each local peer on
failure, only at random (`client.go:562-566`, `:2175`, `:2188`). Stages
end when all messages arrive (§5.5), so with prompt peers the last SR
arrives near begin + 3·`maxJitter`, leaving an FP budget near 2.6T: 26 s
on testnet3, 78 s on mainnet. A late peer can shrink it to nothing.
Derived from reading, not from a run.

**Existing code.** dcroxide's ADR-0012 bans C from anything that ships
or that `cargo test` builds; spec 25 §12.5 reads a spawned program as
outside its link scope. dcr-purse has no solver code and does not depend
on dcroxide-mixing (root `Cargo.toml:18-25`), whose `FieldInt` wraps a
`pub(crate)` `u128`, derives no ordering and folds after every multiply
(`dcroxide:crates/dcroxide-mixing/src/field.rs:19-117` at the pin). The
release profile aborts on panic (root `Cargo.toml:36-40`).

## Options considered

**1. Spawn `csppsolver`.** Full parity. Costs a Rust `net/rpc` client
with a `gob` codec whose bytes were never captured (spec 12 §13 item 5),
sequence matching for concurrent sessions, and child supervision.
Without the program a wallet still mixes on other peers' FPs, but cannot
carry a session of non-solvers past the key exchange. Whether users get
a prebuilt or bundled `csppsolver` is UNVERIFIED (Decrediton's packaging
settles it). Tests need FLINT or a test double; upstream skips them
without a solver (spec 12 §11). The child isolates crashes.

**2. Native solver; `--csppsolver` parsed, never run.** No child, no
`gob`, no C at run time; every dcr-purse mixer solves. Correctness and
speed become the project's, a solver panic is a daemon panic, and two
start-up behaviours diverge.

**2b. As 2, plus a start check.** With `--csppsolver` explicit under
`--mixing`, start the program and kill it at once; a start error fails
start-up with Go's text (`solverrpc/rpc.go:55-71`). Keeps the likeliest
observable failure, a bad path. Costs: running a user-named program for
nothing on every start, matching Go's `os/exec` error texts per
platform (ADR-0011), and the probe still not reproduced.

**3. Both: native by default, external when set.** Its best case is a
field fallback: a proven solver users can switch to without a release if
the native one proves slow or wrong. It costs option 1 plus option 2,
two paths kept tested, one needing FLINT, and C back in the run-time
path when used. It adds no interoperability: the output is fixed.

Spec 12 §12.4's 300-800 lines cover either solver choice.

## Decision

Option 2. Option 3's fallback is not kept: the gate, property tests and
FLINT vectors catch a slow or wrong solver before release, and a second
path would need FLINT-backed testing for as long as it exists.

1. **Crate.** `purse-mixsolver` exposes one pure function: canonical
   residues (`u128`) in, the `mtot` roots ascending out, or an error. It
   accepts exactly when the leading coefficient is non-zero and the
   monic g divides x^F − x:
   1. make g monic; compute h = x^((F−1)/2) mod g;
   2. fail unless x·h² ≡ x (mod g) (rejects repeated roots and factors
      without a root);
   3. split by Cantor-Zassenhaus: gcd(g, h − 1), then
      gcd(g, (x + δ)^((F−1)/2) − 1) recursively;
   4. check each root by Horner evaluation and distinctness; sort.

   It owns its arithmetic, reducing once per accumulated product from
   the first implementation; Karatsuba is the fallback. Each δ is the
   BLAKE-256 hash of the coefficients and an attempt counter, reduced
   mod F (`dcroxide-crypto`, already pinned), so every run reproduces. An
   attempt separates two roots with probability about ½; the attempt
   bound makes defeating the public sequence by choosing SR messages
   cost at least 2⁶⁴ trials. Every error takes Go's non-exit path,
   triggered blame. No input may cause a panic: the crate denies
   clippy's `indexing_slicing`, `unwrap_used`, `expect_used` and
   `panic`, and sizes use checked arithmetic.
2. **Flag.** The client always sets flag bit 0; its "solver exited"
   fallback is unreachable in production.
3. **`--csppsolver`** keeps Go's name, type, default, description and
   file handling and is never executed; set explicitly under `--mixing`,
   it draws one start-up warning. This is an exception to W6 and to
   reproduce-by-default, outside the standing classes of ADR-0012 (D10),
   justified by this decision and needing the owner's approval (Open
   point 1). Option 2b is not taken: the failure it keeps guards a
   solver dcr-purse never runs. Dispositions: QK-0326, QK-0602 not
   applicable; QK-0325 split, the self-check diverging and one identical
   FP per local peer reproduced; QK-0315 unreachable in production and,
   if ported for tests (Open point 5), waiting and giving up at the
   session deadline.
4. **Correctness.** Property tests at degrees 4 to 1024: random distinct
   roots, 0 and F − 1 included, recovered exactly; repeated roots,
   irreducible quadratics, random polynomials, zero leading coefficient
   rejected. A fuzz target: no panic, bounded time, every root verifies.
   An oracle binary of its own, `tools/oracle/cmd/mixsolver`, linking
   `cspp/v2@v2.4.0/solver` under the client's acceptance rule, built
   only by a test gated on `PURSE_ORACLE_FLINT`, set on the one Linux
   leg that installs FLINT. It writes frozen vectors (splitting,
   repeated, non-splitting, the probe input `[1, 2]`, non-monic; spec 12
   §13 item 6) with the FLINT version; every leg checks them. Phase 11
   sessions against Go peers close the loop.
5. **Speed gate, at the Phase 11 exit.** A CI job on `ubuntu-24.04-arm`
   (`.github/workflows/ci.yml:40`) times 21 fixed seeded instances at
   `mtot` = 512 on one core and fails if the median exceeds 1 s: a
   twenty-sixth of the testnet3 FP budget, the rest left to late SRs,
   slower user hardware and concurrent sessions. `mtot` = 1024 goes to
   the bench ledger, and becomes a 4 s gate if an oracle run shows that
   an SR with `mtot` = 513 encodes (Q8). Estimate, not measurement:
   about 10⁸ multiply-accumulates at 512, near 0.3 s at 3 ns each.
6. **Placement.** dcr-purse: the node never solves
   (`dcroxide:crates/dcroxide-mixing/src/mixpool.rs:2942-2955` checks
   sizes only). No dcroxide change is needed: roots sort as `u128`,
   Go's numeric order, and `purse-mixclient` converts `FieldInt` through
   its public byte functions. Adding dcroxide-mixing after spec 12 §13
   item 7's pin-delta check is `purse-mixclient`'s work. A raw accessor,
   if later wanted, lands in dcroxide first, as ADR-0006 proposes (the
   practice used for dcroxide 5c6742f).

Against the brief: this **confirms** a native solver and **sharpens**
the rest: the mathematics defines the output, FLINT vectors confirm
agreement, `--csppsolver` is accepted but never run, and timing is
bounded by `mtot`, not the peer count, against stated budgets.

## Consequences

- **Work created:** the crate, tests, fuzz target and gate job; the
  `mixsolver` oracle and FLINT on one leg; PARITY.md and QUIRKS.md
  entries; Phase 8 `--mixing` expectations, the Go suite generated with
  no `csppsolver` on `PATH`; a spec 12 §5.11.1 fix for the late-context
  randomness of FP creation and `triggeredBlame`.
- **Work avoided:** the `gob` codec and capture, the `net/rpc` client,
  child supervision, FLINT on macOS and Windows.
- **Risks accepted:** a solver bug excludes the wallet or is caught by
  step 4 before an FP leaves; it never yields wrong slots silently. A
  solver panic aborts the daemon, and peers' SR data reaches it; the
  no-panic rules and fuzz target are the defence. Since dcr-purse always
  advertises solving, Go peers without a solver depend on its FP: a
  solve too slow for `recvSR` gets them excluded for a DC timeout
  (QK-0315), a brief R4 harm the speed gate guards against. A
  configuration Go refuses now starts.
- **Constrains:** `purse-mixclient` calls the solver directly, with no
  process-global state; under the OS-thread model ADR-0008 proposes,
  the session's thread blocks for the solve as Go's goroutine does.
- **Revisit if** the gate cannot be met or non-solving Go peers are seen
  failing on late FPs (option 3 is then the remedy); if upstream changes
  the field or the acceptance rule; or if Decrediton's launcher relies
  on `--csppsolver`'s failure, UNVERIFIED until its launcher is read.

## Open points

All five were decided as proposed (project owner, 2026-11-18). They are
kept as the record of what was asked and answered.

1. May an explicit `--csppsolver` naming a missing or broken program
   stop failing start-up under `--mixing`, as an approved exception to
   W6 and to ADR-0012 (D10)? Proposed: yes, over option 2b.
2. Which level and text for the ignored-option line? Proposed: warning,
   `--csppsolver is ignored: mixing roots are solved in process`, once,
   only under `--mixing`.
3. Is a 1 s median at `mtot` = 512 on `ubuntu-24.04-arm` the right gate,
   with 1024 informational until Q8 is settled? Proposed: yes.
4. Does dcroxide's ADR-0012 oracle exception, written for the C that cgo
   links into the oracle, cover FLINT in a separate oracle binary, with
   the live differential on one leg only? Proposed: yes; the binary
   never ships and no other test builds it.
5. Should the non-solving FP wait be ported behind a test-only switch,
   to keep spec 12 §5.11.1's FP validation rule pinned? Proposed: yes,
   with QK-0315's divergence.
