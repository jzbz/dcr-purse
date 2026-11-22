# ADR-0011 — Error-message fidelity

- **Status:** Accepted as proposed (decision D9; project owner,
  2026-11-18), the answers proposed under Open points included. Two inputs
  it builds on were decided the same day: the reference Go is go1.27.4,
  pinned and moved deliberately (spec 23 §13 item 1); and seed words quoted
  in decode errors stay in the RPC reply and are kept out of the daemon's
  log (`docs/QUIRKS.md` QK-0729).
- **Date:** 2026-11-18

## Context

**The question.** A JSON-RPC `error.message` and a gRPC status message
are `err.Error()` verbatim (dcrwallet `internal/rpc/jsonrpc/errors.go:16-49`,
`internal/rpc/rpcserver/server.go:80-83` at the pin; spec 22 §8.2). That
text joins dcrwallet's operation names, kind strings, separators and
format strings with leaves authored by dcrd's modules, Go's standard
library, third-party packages and the OS, plus environment data. W4
promises "the same error codes, and error messages that match dcrwallet's
own text" and leaves the line for standard-library text to D9; W5 promises
"the same status codes and messages" (brief §3). The README promises
"exact results and error text" (`README.md:27`). The brief recommends
byte-exact for dcrwallet-authored text and "exact code plus best-effort
text" for library leaves, listed in `docs/PARITY.md` (brief §9 D9).

**Why now.** Phase 0 exits with D1–D10 decided (brief §6). Crates in
progress already emit leaves (`crates/purse-dcrutil/src/amount.rs:126-135`;
the `purse-seed` error bridge, `docs/PARITY.md:14`). The Phase 7 golden
harness needs a mask rule for error text; the Phase 8 config suite requires
"resolved config or exact error" (brief §6).

**Facts that bear on it.**

- Codes come from the outermost classified kind (spec 16 §8.2, QK-0629).
  dcrwallet has 154 `Op` constants, 38 `Opf` formats and 1,050 `E` call
  sites (spec 22 §13 item 1). Upstream never tests `Error()` text (spec 23
  §3.2).
- `purse-errors` reproduces the errors package byte for byte, both
  separators included, confirmed against the oracle
  (`crates/purse-errors/src/lib.rs:9-13`; QK-0625 and QK-0628
  dispositions).
- Deterministic library leaves have been ported exactly at small cost:
  `encoding/hex` errors with Go's `%#U` (`crates/purse-seed/src/error.rs:159-185`;
  decoder `walletseed.rs:270-300`); in dcroxide, `strconv`,
  `time.Duration`, `net.SplitHostPort`, `*os.PathError` and errno texts
  (`dcroxide:crates/dcroxide-node/src/gostd.rs:819-863`, private to that
  crate) and go-socks texts (`dcroxide:crates/dcroxide-node/src/socks.rs:50-64`).
  dcrd-module texts come through dcroxide: `purse-hdkeychain` returns
  `secp256k1.ParsePubKey`'s texts at v4.4.1
  (`crates/purse-hdkeychain/src/lib.rs:337-387`).
- Network-stack leaves reach clients too. A failed `RpcSync` dial reports
  `Wallet synchronization stopped: rpcsyncer.Run: …` around the
  `wsrpc.Dial` error: connection refused, a TLS failure, or
  `websocket: bad handshake` (spec 20 §5.5.9 item 4; spec 15 §4.1.2). The
  vspd client wraps `net/http` errors in `<METHOD> <url>: ` (spec 11 §8.2),
  returned raw by `processunmanagedticket` and in `ProcessManagedTickets`
  (spec 18 §5.15 step 7; spec 19 §5.6.61). grpc-go statuses beyond
  oversize and unknown method were not enumerated (spec 20 §13.1 item 5).
  dcrjson's `encoding/json`-derived texts do not reach the daemon's
  clients (spec 16 §4.3.1; a parse failure becomes `-32600 Invalid
  request`, §5.5.2).
- The versions dcrwallet links are in its `go.mod:5-45` at the pin
  (secp256k1 v4.4.1, txscript v4.1.2, dcrjson v4.2.0, go-flags v1.5.0,
  vspd client v4.0.2, wsrpc v2.3.8, gorilla/websocket v1.5.1, grpc
  v1.83.2). Go is pinned because it authors part of what the oracle pins
  (`crates/purse-testutil/src/lib.rs:41-46`). dcroxide's reference is
  go1.27.3, and its dcrjson Go-JSON texts still follow go1.26.2, a
  recorded divergence (`dcroxide:docs/PARITY.md:132-148`). A file-by-file
  comparison of go1.27.3 and go1.27.4 (non-test sources of strconv,
  encoding/hex, encoding/json, fmt, time, io, context, os, syscall, net,
  net/http, crypto/tls, crypto/x509) finds no changed error string.
  dcrwallet has no release build recipe, so the Go behind a user's binary
  is unknown (spec 23 §13 item 1).
- Go's Unix `Errno.Error` is a table compiled into `syscall` (go1.27.4
  `syscall/syscall_unix.go:110-118`; `zerrors_linux_amd64.go:1359`
  `no such file or directory`, `:1428` `RFS specific error`). Rust's
  `io::Error` prints the C library's text plus ` (os error 2)`. dcroxide
  approximates Go by lower-casing the first letter of the C library's text
  (`gostd.rs:819-837`), which cannot yield `RFS specific error`. On
  Windows Go asks `FormatMessage` for English (US), falling back to the
  system language (`syscall_windows.go:165-180`). Rust's Windows rendering
  at the pinned toolchain is UNVERIFIED.
- Golden captures compare raw bytes after masks; a mask is declared per
  method and field, "added only after the capture tool shows the field
  varying between two Go runs" (spec 23 §4.6, §5.8.4; brief §7 Layer 3).
  Layer 3 names no platform.
- What clients match is unknown, and the specs disagree. Spec 18 §13 D3
  says code `-14` "is what clients match on"; spec 22 §9 item 1 says kind
  strings are read "and sometimes match[ed]" by Decrediton, dcrctl users
  and scripts. Neither rests on a reading: Decrediton's source was not
  read (spec 20 §13.1 item 3(e); brief §6 Phase 0), nor dcrctl's wallet
  commands (spec 23 §13 item 8). dcrwallet's own wsrpc client shows only
  `message` (spec 22 §8.2).

## Options considered

**A. Byte-exact everywhere.** Needs every TLS, x509, `net/http` and
HTTP/2 transport text emulated over a Rust stack before the Phase 9 and 12
exits; Go's deliberately unstable protobuf text (spec 19 Q22) cannot be
matched at all.

**B. The brief's recommendation.** Byte-exact for dcrwallet-authored
text; library leaves exact in code with best-effort text, the inexact ones
listed. Testable as C is: listed leaves are masked, the rest gated. Its
strength: no client is known to depend on library text, and no phase exit
waits on emulating a network stack's texts. Its weakness is the default: a
library leaf nobody has met yet is inexact until shown otherwise, so the
list must also name the cheap leaves that are in fact reproduced, and
"Go libraries" leaves dcrd's modules unplaced.

**C. Exact by default, exceptions per leaf.** Every leaf byte-exact unless
a single leaf is listed. Most fidelity; but the network-stack texts above
are unported, uncosted and on the Phase 9 and 12 exit paths.

**D. Codes exact, messages free.** Cheapest. Breaks W4, W5 and the
README; every capture masks its message.

**E. Hybrid.** C's default for every class, with the network-stack and
transport texts listed at the start as bounded class entries.

## Decision

Option E. It confirms the brief for dcrwallet-authored text, places dcrd
modules with it, and sharpens the brief for library leaves: exact is the
default for a leaf not yet met, because every deterministic leaf met so
far was ported exactly by a small function; best effort is confined to
named, bounded classes whose cost is unknown. A capture that fails on an
unlisted leaf is fixed by porting it or by a new, reviewed ledger entry.
Rules:

| Class | Examples | Rule |
|---|---|---|
| 1. dcrwallet | `loader.OpenExistingWallet: item does not exist:: wallet.OpenDB: missing database file` (spec 20 §8.1); `unrecognized curve X` (spec 21 §4.1.2) | Byte-exact, including Go verbs inside its formats (`%q`, `%v` of maps as Go sorts them, `%T` names) |
| 2. dcrd modules | `malformed public key: invalid length: 0` (spec 18 §8.2); `wire` texts behind `Could not decode Tx:` (spec 20 §13.1 item 7); `-32600: Invalid request` (spec 16 §8.1) | Byte-exact at the linked version; owned by the dcroxide crate; a gap is fixed in dcroxide first and the pin moved, as for `5c6742f` |
| 3. Go standard library | `encoding/hex: invalid byte: U+0067 'g'` (spec 20 §5.8.2); `strconv.ParseFloat: parsing "X": invalid syntax` (spec 21 §4.1.2); `context deadline exceeded` (spec 16 §8.3); `*os.PathError`, `*net.OpError`, `*url.Error` wrappers | Byte-exact against go1.27.4; TLS, x509 and `net/http` client transport texts: ET-001, ET-002 |
| 4. Third party | go-flags parse errors (spec 21 §8.2); vspd client prefixes (spec 11 §8.2); go-socks; gorilla's server replies, close texts and `websocket: bad handshake`; `wsrpc: unknown response ID` (spec 15 §8.1); grpc-go statuses of spec 20 §8.1 and `rpc error: code = … desc = …` | Byte-exact; a VSP server's `message` passes verbatim; other grpc-go transport statuses: ET-003; protobuf text: ET-004 |
| 5. OS errors | `open <path>: no such file or directory` (spec 21 §5 step 13) | Go's rendering from generated tables: Unix per CI OS; Windows English (US); Windows codes outside the table: ET-005 |
| 6. Environment data | paths, remote addresses, ports, durations | Format exact, data equalised |

1. **Codes are always exact:** JSON-RPC codes, gRPC status codes, HTTP
   statuses, websocket close codes and the TLS alerts a server sends (a Go
   client prints them: spec 16 §11, `TestUntrustedClientCert`). No ledger
   entry covers a code. One exception: an error converted from a Go
   panic has no Go code (spec 16 §8.4: no response, or process exit); its
   code is chosen case by case under ADR-0012 and recorded as a
   divergence.
2. **Class 6.** Harnesses pass explicit paths, injected clock and
   randomness, and fixed ports to both sides. Texts embedding default
   paths or names that ADR-0005 changes are compared with explicit paths;
   with defaults they follow ADR-0005, recorded as one divergence.
3. **Mechanism.** The `Display` of any error type that can become a leaf
   of a client-visible message is the Go text, carried as a `purse-errors`
   foreign leaf (`crates/purse-errors/src/any.rs:135`, `:192`). Rust's own
   texts (`io::Error`, redb, rustls) reach such a message only as a ledger
   entry's policy allows. `purse-boltread` keeps its own texts~~: the importer is not
   an RPC surface~~ (`crates/purse-boltread/src/error.rs:243-279`). (Reason
   void, ADR-0013; restated in the addendum.)
4. **Standing exceptions.** Secrets never appear in error text. Proposed
   under brief §4 principle 4 and the third standing rule (ADR-0012): the
   `signrawtransaction` WIF map echo (dcrwallet
   `wallet/wallet.go:4820-4822`, spec 08 Q6) is redacted, keeping Go's
   format; reporting it upstream is a candidate for D13. The seed-word echo
   follows QK-0729. These are divergences in PARITY.md, not ledger
   entries.

**The ledger.** A new PARITY.md section, "Error text not reproduced byte
for byte", one row per entry: ID (`ET-NNN`); scope (one leaf, or a class
bounded by Go package and the surfaces it reaches); Go origin
(`package@version`, `file:line` where one); dcr-purse's text policy;
reason; the pinning test. Wallet-authored and wrapper text around an entry
stays exact. Initial entries:

| ID | Scope | Policy |
|---|---|---|
| ET-001 | `crypto/tls` and `crypto/x509` failure texts in outbound dials (dcrd, VSP) and the `http: TLS handshake error` stderr line (spec 16 §8.5) | Own text after Go's `tls: ` / `x509: ` prefix |
| ET-002 | `net/http` client transport texts inside the vspd client's `<METHOD> <url>: ` (dial errors excluded, class 3) | Own text |
| ET-003 | grpc-go transport statuses not in spec 20 §8.1 | Code and text exact once the ADR-0004 spike enumerates them; until then own text |
| ET-004 | protobuf text rendering in a status message (spec 19 Q22) | Own stable rendering |
| ET-005 | Windows OS codes outside the generated table | Rust's text without ` (os error N)` |
| ET-006 | store-engine texts under dcrwallet's `Op` and kind | redb-derived own text |

**Enforcement.** Golden captures compare whole messages byte for byte
with two kinds of mask. *Nondeterminism masks* follow spec 23 §5.8.4 (two
Go runs must differ) and carry no ledger entry. *Exception masks* each
name one `ET-NNN` and cover only the span its scope describes. Both may
cover a span inside a field: a row gives method, error exit, field,
a regular expression over Go's captured text whose named groups are
masked, and the reason (`nondet` or the ET ID). Spec 23 §4.6 and §5.8.4
are amended to allow this; the capture tooling fixes the file format in
spec 23 before the Phase 7 harness is built. Per-crate oracle
differentials compare rendered text, as `purse-errors`, `purse-seed` and
`purse-hdkeychain` do. OS texts: the oracle renders `*os.PathError`,
`*os.SyscallError` and `*net.OpError` for a fixed code list on each CI OS
(Linux x86-64 and arm64, macOS, Windows), and a unit test of the Go-text
crate compares on the same runner. Vectors are regenerated at each
reference-Go bump, so a changed Go text shows as a diff.

## Consequences

- **Work created.** A shared Go-text crate in dcroxide, extracted from
  `gostd` and `dcroxide-rpc`'s helpers; it replaces `gostd`'s lower-casing
  with Go's generated tables, a dcroxide behaviour change landed there
  first. The table generator and per-OS vectors; the mask syntax and the
  spec 23 amendments; the ledger section; enumeration of grpc-go transport
  statuses in the ADR-0004 spike.
- **Work avoided.** Emulating TLS, x509, `net/http` and HTTP/2 transport
  texts before the Phase 9 and 12 exits; per-message "close enough"
  judgements; whole-message masks.
- **Risks accepted.** A user comparing with a dcrwallet built by another
  Go may see different standard-library texts. Listed classes differ by
  design; a client matching TLS or HTTP text (none known) would break.
  Texts the specs never enumerated (spec 17 §13 item 6, spec 19 §13 item
  2, spec 20 §13.1 item 7) surface only when captures reach them.
- **Constraints.** ADR-0009: its relevance test counts error text as
  client-visible, so a text change in a skewed module needs a dcroxide
  parameter; the secp256k1 v4.4.1 / v4.4.2 pseudo-version skew is a
  class 2 risk, settled by the differential of spec 23 §13 item 10.
  ADR-0004: the transport emits grpc-go's enumerated texts. ADR-0003:
  store failures keep dcrwallet's `Op` and kind. ADR-0002:
  operation-script results compare error text under these rules.
  dcroxide's go1.26.2 dcrjson texts matter only where they reach a
  dcr-purse client, which spec 16 §4.3.1 says none does.
- **Revisit if** the Decrediton reading finds string matching on a listed
  path (that entry must become exact), a reference-Go bump rewords many
  texts, a class entry must widen beyond its stated scope, or a new class
  entry is proposed after Phase 7.

## Open points

All seven were decided as proposed (project owner, 2026-11-18). They are
kept as the record of what was asked and answered.

1. **Do log lines carry this fidelity?** Proposed: an error inside a log
   line is the reply's rendered text and inherits fidelity; log-only
   leaves are listed, not gated. W7 governs the line format.
2. **What does the redacted WIF message say?** Proposed:
   `no key for address (needed: <addr>, have map[<addr>:[redacted] ...])`,
   each WIF replaced by `[redacted]`.
3. **Where does the Go-text code live?** Proposed: a dcroxide library
   crate, landed there first; dcroxide is unreleased, so no shim.
4. **Must dcroxide's go1.27.3 texts match go1.27.4?** Proposed: yes, as
   dcr-purse's differentials see them; no work is due now (no changed
   error string), and the comparison is repeated at each bump of either
   reference.
5. **Windows OS texts?** Proposed: an English (US) table generated by
   running Go once on an English Windows CI runner, used as data. It
   matches Go wherever English message resources are installed and needs
   no foreign call. Alternatives: Rust's system-language text (cheapest;
   differs on non-English installations), or `FormatMessage` with an
   English language ID in a dcroxide crate under a narrowly scoped unsafe
   exception, as `dcroxide:crates/dcroxide-winsvc/src/lib.rs:36-44` does.
   Whether Rust's and Go's texts agree on English Windows is UNVERIFIED;
   one CI run over the code list settles it.
6. **Should common TLS verification failures be mapped to Go's text?**
   Proposed: yes for unknown authority, expiry and name mismatch, once the
   TLS library in use reports them as distinct variants; ET-001 narrows.
7. **Who reads Decrediton for string matching, and when?** Proposed: the
   Phase 0 reading of its launcher and gRPC calls, recorded in specs 19–21
   before Phase 8 (brief §6). This decision does not wait for it; a
   finding of string matching triggers the revisit above.

## Addendum, 2026-11-18 — ADR-0013

On 2026-11-18 the project owner decided that dcr-purse imports no
dcrwallet database: migration is by seed restore only (ADR-0013). The
decision of this ADR stands. One statement rested on the import:

- **Rule 3, last sentence.** "`purse-boltread` keeps its own texts: the
  importer is not an RPC surface" now reads: `purse-boltread` keeps its
  own texts because it is a test utility, not part of the wallet; none of
  its texts reaches a client, a log line or an exit status of dcr-purse.
  The cited texts (`crates/purse-boltread/src/error.rs:243-279`) are
  unchanged. The body's old reason is struck in place, not removed, since
  git history does not yet hold the accepted text. The owner may later
  remove `purse-boltread` (ADR-0013 Open point 2).

Unaffected, and checked: Rule 2's comparison of texts that embed default
paths or names, since ADR-0005's name, data directory and file names stand
(its import front end is void, so no import text exists to compare).
