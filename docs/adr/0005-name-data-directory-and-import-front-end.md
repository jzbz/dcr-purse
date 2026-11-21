# ADR-0005 — Name, data directory and the import front end

- **Addendum, 2026-11-18:** the import front end is void (ADR-0013, project
  owner, 2026-11-18); the name, data directory, file names, version string
  and certificate fallback stand. See the Addendum below; void text is
  struck in place or moved, verbatim, to Appendix A.
- **Addendum, 2026-11-19:** the P2P user agent carries dcr-purse's own
  token, `/dcrwire:1.0.0/dcr-purse:0.1.0/`, and the JSON-RPC `version`
  reply keeps the key `dcrwallet` (project owner, 2026-11-19). Item 9 is
  amended to that extent and Open point 2 reversed for the user agent; see
  "Addendum, 2026-11-19 — Decision by the project owner", before
  Appendix A.
- **Status:** Accepted (decision D1; project owner, 2026-11-18) as
  proposed, with one change: the dcrd certificate fallback also tries
  dcroxide's directory (Decision item 2; Open point 5). The name
  `dcr-purse`, hyphen included, and the crate prefix `purse-` are final
  (Decision item 1). Installing as `dcrwallet` for Decrediton (c) still
  waits for a Phase 8 addendum. It builds on the project owner's decisions:
  own database, ~~one-way import of one schema version that never modifies
  the source~~ (2026-11-14: brief §1, W2, D2, D4; `docs/PARITY.md`,
  "Deliberate divergences"; ~~ADR-0007~~); engine `redb` 5 (2026-11-18;
  recorded in ADR-0003).
- **Date:** 2026-11-18

## Addendum, 2026-11-18 — ADR-0013

On 2026-11-18 the project owner decided that dcr-purse imports no dcrwallet
database: migration is by seed restore only (ADR-0013). This record was
accepted the same day with an import front end, which is now void. The
sections below keep the accepted text: what is void is struck in place, or,
where a whole item, option or open point is void, moved verbatim to
Appendix A and replaced by a marker. The text is kept in this file because
`docs/adr/` had not been committed when this addendum was written, so git
history does not hold the accepted version. Item and point numbers keep
their gaps, because the status line and other records cite them.

**What stands.** Decision item 1 (name, prefix). Item 2 (data directory,
`dcr-purse.conf`, `dcr-purse.log`, options as dcrwallet's, no environment
variables, the dcrd certificate fallback through dcrd's and then dcroxide's
directory) except its last two sentences. Item 9 (version string and user
agent, v3). Item 10 ((c) waits for Phase 8), without its references to
items 3 and 4 and to the passphrase path. Open points 1, 2, 5 and 6. Of the
options: (a) without its explicit import, (c), (d) and the version options;
with (b)'s offer void, the identity chosen is (a). *(Item 9's user agent
and Open point 2 changed on 2026-11-19: the user agent carries dcr-purse's
own token, the JSON-RPC key stays `dcrwallet`; addendum of that date.)*

**What is void.** The choice of (b) in the part it adds to (a), the
first-start offer, and the import flag of the Decision preamble. Item 2's
last two sentences (`dcrwallet.conf` read as an import source; `--appdata`
naming a dcrwallet directory and the import writing `dcr-purse.conf`
there). Items 3 (finding the source, `--importfrom`), 4 (`--import`, its
steps, the one-way notice, commit order), 5 (the offer), 6 (carrying and
rewriting `dcrwallet.conf`; copying `rpc.cert`/`rpc.key`, CA files and
`peers.json`), 7 (exit statuses) and 8 (the report). Open points 3, 4 and
7. The options "Import entry point" and "Files". Every consequence, risk
and constraint that concerned them: in Consequences, the import parts of
"Work" and "PARITY.md divergences", the whole of "Risks accepted" as
written, and the whole of "Constraints".

**What follows, added by this addendum.** These sentences were not part of
what the owner accepted with this record; each follows from item 2 or from
ADR-0013, and those marked Proposed await the owner.

- **Configuration.** With item 2's last two sentences void, dcr-purse reads
  no `dcrwallet.conf`: its configuration file is `dcr-purse.conf` alone.
  Restoring a seed is `--create` answering yes to the existing-seed prompt
  (spec 21 §5.4.1, §5.4.10), as in dcrwallet.
- **First start.** Without a wallet, dcr-purse prints dcrwallet's `The
  wallet does not exist.  Run with the --create option to initialize and
  create it.` and exits 1 (spec 21 §5.2 step 20), as (a) describes. Whether
  it looks for a dcrwallet directory is ADR-0013 Open point 3, decided by
  the project owner on 2026-11-19: it does not.
- **TLS.** No file is taken from dcrwallet's directory. When `rpc.key` does
  not exist, the start that brings up the RPC servers generates a new
  `rpc.cert` and `rpc.key` (spec 16 §5.1.2, §5.1.3; spec 21 §5.3.5), here
  in dcr-purse's directory. A client that pinned dcrwallet's certificate
  must be given dcr-purse's, or the operator names the existing pair with
  the `rpccert` and `rpckey` options (spec 21 §4.1.1, options 50 and 51);
  ADR-0013 Open point 4 proposes the user documentation. The dcrd CA file
  is chosen as dcrwallet chooses it, `<appdata>/dcrd.cert` if it exists,
  else for a localhost `rpcconnect` dcrd's `rpc.cert` in dcrd's directory
  (spec 21 §5.2 step 26, §9 item 6), then, per item 2, dcroxide's.
- **Decrediton.** Without an import, Decrediton, which expects to open an
  existing wallet, needs the wallet created or restored through its own
  flow (ADR-0013 Decision item 8). Item 10 stands.
- **Work.** What remains of "Work" is `purse-config` (`AppDataDir` with a
  name; the dcrd CA fallback with dcroxide's directory last) and a
  build-time rustc capture; tests cover the directory and file names per OS
  and network, the fallback order, and the version string in each place
  item 9 names.
- **PARITY.md divergences.** Binary, directory, config and log names; the
  dcroxide certificate fallback (item 2); the build metadata; the `-V` and
  startup parenthetical.
- **Phase 8 exit.** The brief's M3 sentence "after an import, an existing
  `dcrctl` configuration with the old `rpc.cert` connects unchanged" no
  longer applies. Its Proposed replacement is ADR-0013 Open point 6: an
  existing `dcrctl --wallet` configuration connects once its `rpccert`
  names dcr-purse's `rpc.cert`; nothing is carried over.
- **Shared directory, Proposed.** With no import, `--appdata` naming a
  dcrwallet directory no longer recreates (d)'s stale-database risk: dcr-purse
  then uses that directory's `rpc.cert`, `rpc.key` and `clients.pem` (spec
  21 §9 item 5), shares `<net>/peers.json` and puts its store in `<net>/`,
  but never opens `wallet.db`. Proposed: accept this, on the ground the
  owner gave for the larger risk, that only an explicit choice produces it.
  Whether dcr-purse's `peers.json` stays readable by dcrwallet's addrmgr
  v3.0.0 matters only then, and is not settled here (ADR-0009 item 5
  governs the address manager).
- **Amendments done.** The amendments listed under "To amend on
  acceptance" are made: `README.md` ("What 'drop-in' means" names
  `dcr-purse.conf` and the own data directory), brief §1 and W6, and
  `docs/specs/README.md`, which marks spec 21 §9 item 1 and §13 item 11
  superseded.
- **Title, Proposed.** Retitle this record "Name, data directory and
  identity", keeping the file name, which the brief and `docs/adr/README.md`
  link to.

## Context

D1 (brief §9) fixes the name, data directory, ~~import source,~~ version
and user agent~~, carried files and import front end~~. ~~Phase 8 builds the
import command and offer, and its exit needs an old `dcrctl` configuration
to connect after an import; Phase 3 builds the front end first (brief
§6).~~

**dcrwallet at the pin.**

- Data directory `~/.dcrwallet`, `~/Library/Application Support/Dcrwallet`,
  `%LOCALAPPDATA%\Dcrwallet`: database `<net>/wallet.db`, address book
  `<net>/peers.json`, TLS and CA files at the top, log
  `logs/<net>/dcrwallet.log` (spec 21 §4.1.3).
- Config file `<appdata>/dcrwallet.conf`, never written by the daemon; with
  `--appdata=<dir>` it is `<dir>/dcrwallet.conf`, and a missing one costs one
  `WRN` line. `appdata=` in it rebases only path options not set explicitly;
  `testnet=1`/`simnet=1` in it select the network (spec 21 §5.2 steps 5, 8,
  9, 13, 17). The shipped sample shows, commented out,
  `rpccert=~/.dcrwallet/rpc.cert` and `rpckey=~/.dcrwallet/rpc.key`
  (spec 21 §4.1.4).
- Actions are flags; positional arguments are ignored; every option is a
  config-file key, so `create=1` there reruns the wizard on every start
  (spec 21 §4.1.4, §4.1.5, §9 item 26). Without a wallet, startup prints
  `The wallet does not exist.  Run with the --create option …` and exits 1;
  a Go panic exits 2 (spec 21 §5.2 step 20, §8.1).
- One version string reaches `-V`, printed as `<argv[0] basename without
  extension> version <v> (Go version <go> <GOOS>/<GOARCH>)`
  (`config.go:407-414`), the startup log line (same parenthetical,
  `dcrwallet.go:89`), JSON-RPC `version` under key `dcrwallet`, and the user
  agent `/dcrwire:1.0.0/dcrwallet:<v>/` (spec 14 §3.2); `getinfo` reports
  2020000 (spec 21 §9 item 13). `<v>` is `2.2.0-pre`, plus `+<9-hex
  revision>` for a git build, so the build metadata varies between Go builds
  (spec 16 §4.8); release values are not in the repository (spec 21 §13
  item 2).
- dcrd (`46e9df48`) uses a peer's user agent only in log lines
  (`server.go:2700`, `peer/log.go:132`) and `getpeerinfo`'s `subver`
  (`internal/rpcserver/rpcserver.go:2839`); crawlers' use is UNVERIFIED
  (spec 14 §8 item 2).
- Decrediton's invocation, its `--appdata` included, is unread (spec 21 §13
  item 1); the brief schedules the reading before Phase 8 (§6, Phase 0).

**dcroxide** runs as `dcroxide` with `~/.dcroxide`, `dcroxide.conf`,
`dcroxide.log` and `DCROXIDE_*` variables (dcroxide `docs/PARITY.md`, "The
port answers to its own name throughout";
`crates/dcroxide-node/src/config.rs:46`); no ADR records it. Its costs:
`dcrd.conf` must be pointed at, and vector comparisons map names back
(`as_dcrd_names`). Its user agent is its own token; its release version
identity is open, tied to dcrd's `release.local` build metadata (dcroxide
ADR-0014 §5, §6).

**In this repository.** The store is not `wallet.db`, so a shared
directory would hold two copies of one wallet that diverge at the first
write (brief §9 D1); spec 21 §9 item 1 and §13 item 11 predate that and
assume opening in place. "dcr-purse" was a working name until 2026-11-18,
when the project owner made it final; the `purse-` prefix is in use since
2026-11-18 (`README.md`, `Cargo.toml`). No code reads a data directory,
config or version yet. Release builds abort on panic (`Cargo.toml`).

## Options considered

**Identity.** (a) Own name and directory~~, explicit import~~: dcroxide's
shape; a user who starts dcr-purse first meets "does not exist". ~~(b) As
(a), plus an offer at first start: costs a prompt, a search rule and a
non-terminal rule.~~ (c) Also installable as `dcrwallet` for Decrediton:
~~the import runs unprompted, so the one-way notice never reaches the
user;~~ not designable before the launcher is read. (d) dcrwallet's names
and directory: clients keep default paths, but the two cannot coexist, a
dcrwallet started by mistake opens the stale `wallet.db`, and both write
`dcrwallet.log`.

**Import entry point.** Void (ADR-0013); the accepted text is in Appendix A.

**Version.** (v1) dcrwallet's string: dcr-purse's faults are attributed to
dcrwallet. (v2) Own token, `dcr-purse:2.2.0-pre`: the only option software
can tell apart; departs from a compatibility-critical item (spec 14 §8
item 2). (v3) `dcrwallet:2.2.0-pre+dcr-purse.0.1.0`: names dcr-purse only to
a reader of the full string; SemVer 2.0.0 §10 ignores build metadata for
precedence and `getinfo` stays 2020000, so to software it is v1.

**Files.** Void (ADR-0013); the accepted text is in Appendix A.

## Decision

~~(b), confirming the brief, with an import flag;~~ (c) waits for Phase 8.
~~The offer appears only where dcrwallet's start fails, so no start that
succeeds today changes; over (a) plus a hint it saves the migration's one
command and shows the one-way notice at the moment of choice. A flag,
because the offer needs the importer in the daemon anyway and dcrwallet's
actions are flags.~~ (Void parts: ADR-0013; what stands is (a), see the
Addendum.)

1. **Name.** `dcr-purse`, hyphen included, with the crate prefix `purse-`;
   both final (accepted, project owner, 2026-11-18). *One exception
   (project owner, 2026-11-19): the crate `sntrup4591761` keeps the name
   of the Go package it ports, without the prefix (brief §5).*
2. **Data directory.** dcrwallet's `AppDataDir` with the name `dcr-purse`
   (`~/.dcr-purse`, elsewhere `Dcr-purse`). Inside: `dcr-purse.conf`;
   `<net>/` with the store (named per ADR-0003) and `peers.json`
   *(the store's file name is fixed in Phase 2 with the schema, a name
   other than `wallet.db`: project owner, 2026-11-19; ADR-0003, addendum
   of that date, item 5; ADR-0018 Open point 6)* *(proposed name, not yet
   decided: `wallet.redb`, `purse_store::STORE_FILE_NAME`; ADR-0018,
   addendum of 2026-11-20 "The `purse-store` part, as built", open point
   BK1)* *(decided: `wallet.redb`, project owner, 2026-11-21; ADR-0018,
   addendum of that date, item 1)*;
   `logs/<net>/dcr-purse.log`, named for the program as dcroxide's is, so
   the two never share a log; TLS and CA files under dcrwallet's names.
   Options behave as dcrwallet's; no environment variables; the dcrd
   certificate fallback tries dcrd's `rpc.cert` in dcrd's directory, as
   dcrwallet does, and then, if that file does not exist, `rpc.cert` in
   dcroxide's directory (decided by the project owner, 2026-11-18, against
   the proposal; a divergence recorded in `PARITY.md` when the option is
   built). ~~`dcrwallet.conf` is read only as an import source. `--appdata`
   may name a dcrwallet directory; the import then writes `dcr-purse.conf`
   there (item 6).~~

Items 3 to 8: void (ADR-0013); the accepted text is in Appendix A.

9. **Version (v3)** `2.2.0-pre+dcr-purse.0.1.0`, in the places Go's goes.
   `-V` and the startup line: `dcr-purse version 2.2.0-pre+dcr-purse.0.1.0
   (Rust version 1.100.0 linux/amd64)`, rustc captured at build time, OS
   and architecture in Go's names. JSON-RPC `buildmetadata` is
   `dcr-purse.0.1.0`. Reason: the token's consumers are unknown, D10
   reproduces by default, and v3 sends software what every dcrwallet sends;
   moving to v2 once a consumer is known is one change. Confirmed once the
   release build metadata is read (spec 21 §13 item 2). Answers spec 16
   D1, spec 14 §12 item 8, spec 21 §13 items 2 and 3. *Amended (project
   owner, 2026-11-19): the P2P user agent is not v3's but dcr-purse's own
   token, `/dcrwire:1.0.0/dcr-purse:0.1.0/`; the JSON-RPC `version` reply
   keeps the key `dcrwallet` with the value this item gives, and
   `getinfo` its number; `-V` and the startup line say `0.1.0`, the reply
   gains a `dcr-purse` key, and a build from a checkout appends its
   revision to the token (spec 14 §12 item 8); addendum of that date,
   item 3.*
10. **(c) waits** for a Phase 8 addendum. Nothing in items 2 ~~to 4~~ is
    known to rule it out; the config and log names ~~and the passphrase
    path~~ will be revisited there.

## Consequences

- **Work.** `purse-config` (`AppDataDir` with a name~~, items 3 and 6~~);
  ~~`purse-import` (front end, report, statuses); `purse-prompt`; two help
  lines;~~ a build-time rustc capture. ~~Tests include a kill at each step
  of item 4, the path rewrite, the network from `testnet=1`, and the source
  byte-identical afterwards.~~
- **Golden harness** must mask `buildmetadata` and `versionstring` under
  `dcrwallet` in `version` captures, and the user agent in P2P captures.
  *(Since 2026-11-19 the user agent differs in its token, not only in its
  build metadata, and with it the length of the `version` message: 31
  bytes of user agent against Go's 45, or 35 without a revision (spec 14
  §3.2). What the P2P mask must cover is the harness's to work out: the
  field, its length prefix, and the header's payload length and checksum
  (spec 14 §3.1) all change with it; addendum of that date.)*
- **PARITY.md divergences:** binary, directory, config and log names (tools
  and Decrediton views reading `dcrwallet.log` find nothing, spec 21 §9
  item 14); ~~the two options; the offer;~~ the build metadata; the `-V`
  and startup parenthetical. *Added 2026-11-19: the P2P user agent's token
  (addendum of that date).*
- **To amend on acceptance:** `README.md`, `What "drop-in" means`; brief §1
  ("the same `dcrwallet.conf`") and W6 ("the same data directory
  resolution"); spec 21 §9 item 1 and §13 item 11, marked superseded in
  `docs/specs/README.md`. (Done; see the Addendum.)
- **Risks accepted.** Void as written (ADR-0013); the accepted text is in
  Appendix A, and the Addendum's "Shared directory" proposes what replaces
  it.
- **Constraints.** Void (ADR-0013); the accepted text is in Appendix A.
- **Revisit** when Decrediton's launcher is read or a consumer of the user
  agent is found. *(For the user agent this was overtaken on 2026-11-19 by
  the owner's decision, not by a consumer; addendum of that date.)*

## Open points

Decided as proposed (project owner, 2026-11-18), except point 5, which was
decided the other way~~; point 4 stays with the Phase 8 addendum~~. They are
kept as the record of what was asked and answered.

1. Decided (project owner, 2026-11-18): `dcr-purse`, hyphen included, is
   final.
2. Should the user agent carry its own token now (v2)? Proposed: no, until
   a consumer is known. *Reversed for the P2P user agent (project owner,
   2026-11-19): it carries dcr-purse's own token, `dcr-purse:0.1.0`, which
   is dcr-purse's own version rather than v2's `dcr-purse:2.2.0-pre`; the
   JSON-RPC `version` reply keeps the key `dcrwallet`. Addendum of that
   date.*

Points 3 and 4: void (ADR-0013); the accepted text is in Appendix A.

5. Should the dcrd certificate fallback also try `~/.dcroxide`? Decided
   (project owner, 2026-11-18), against the proposal: yes, after dcrd's
   directory. A default dcroxide and dcr-purse pair then needs no `cafile`.
   No existing dcrwallet setup changes: the second location is tried only
   when the first has no certificate.
6. Should dcroxide's open version identity (its ADR-0014 §6) follow this?
   Proposed: no, independent; the node's user agent is already its own.
   *Unaffected by the decision of 2026-11-19, which gives dcr-purse a
   token of its own as the node has; the two identities stay independent.*

Point 7: void (ADR-0013); the accepted text is in Appendix A.

## Addendum, 2026-11-19 — Decision by the project owner: the user agent and the `version` reply

**What was put.** Item 9 chose v3: the user agent
`/dcrwire:1.0.0/dcrwallet:2.2.0-pre+dcr-purse.0.1.0/` and the JSON-RPC
`version` reply under the key `dcrwallet`, so that software sees a
dcrwallet. It was chosen while looking identical to dcrwallet on the
network was a goal. The owner has since put speed ahead of that goal
(brief §3 W3, "Interoperability on the network"; ADR-0014, accepted
2026-11-19, whose Decision §1, tier 4, makes blending in no longer a
constraint).

**Decision.** On 2026-11-19 the owner chose the option put as "split: own
token on the P2P user agent, but keep the dcrwallet key in the JSON-RPC
version reply so existing RPC clients such as Decrediton that look up that
key still find it", the recommended one. The reason put with it:
dcr-purse will behave differently on the wire (faster sync patterns, under
ADR-0014), so dcrwallet's name in the user agent would attribute that
behaviour to dcrwallet, while the RPC key is something clients may depend
on; dcroxide already uses its own token. The text above stays as written,
apart from the dated pointers added to it.

1. **The P2P user agent** is `/dcrwire:1.0.0/dcr-purse:0.1.0/`, where
   dcrwallet sends `/dcrwire:1.0.0/dcrwallet:<version>/` (spec 14 §3.2).
   `0.1.0` is dcr-purse's own version, the one item 9 puts in the build
   metadata. This replaces item 9 for the user agent only.
2. **The JSON-RPC `version` reply** keeps the key `dcrwallet`, so a client
   that looks it up finds it. `dcrwalletjsonrpcapi` stays `11.0.0` (spec
   16 §4.8).
3. **Settled by the owner later the same day, as suggested:** `0.1.0`
   wherever dcr-purse names itself, and the dcrwallet-compatible number
   where a client reads it to decide what the wallet can do.
   - The `-V` output and the startup line read `dcr-purse version 0.1.0
     (Rust version …)`, replacing item 9's
     `2.2.0-pre+dcr-purse.0.1.0` there.
   - The `version` reply gains a key `dcr-purse` reporting `0.1.0`, so a
     tool can identify the wallet over RPC; a client that looks up the
     keys it knows is not disturbed by another.
   - The value under the `dcrwallet` key stays item 9's (`versionstring`
     `2.2.0-pre+dcr-purse.0.1.0`, `buildmetadata` `dcr-purse.0.1.0`), and
     `getinfo` stays 2020000: a client that compares either against a
     minimum would take `0.1.0` for a very old dcrwallet.
   - A build from a git checkout appends its revision to the user agent's
     token, as Go's appends `+<9-hex revision>` (spec 14 §12 item 8), so
     a development build is told from a release in peer lists.

   Decrediton's use of the `version` reply is unread (Context, above;
   the brief schedules the reading before Phase 8). The value under the
   `dcrwallet` key is confirmed against that reading then, with item 9's
   "Confirmed once the release build metadata is read".

**What follows.**

- **PARITY.md** records the user agent among the divergences decided ahead
  of the code (Phase 10, where the SPV peer is built). It is a
  wire-visible difference of the kind brief W3 says is recorded there.
- **Golden harness.** P2P captures must mask the user agent and what
  depends on its length, which now differ: the field's length prefix and
  the message header's payload length and checksum (spec 14 §3.1;
  Consequences, above).
- **Specs.** Spec 14 §3.2's field note ("A drop-in replacement must emit
  the same `dcrwallet:<semver>` token") and §8 item 2, spec 21 §9 item 13
  (the P2P user agent component `dcrwallet:<version>/`) and spec 24 §9
  item 12 are superseded for the user agent, and listed as such in
  `docs/specs/README.md`. Spec 16's D1 and O6 (the value under the
  `dcrwallet` key) stay answered by item 9; its `-V` and startup line
  are answered by item 3 above.
- **Interoperability.** dcrd uses a peer's user agent only in log lines
  and `getpeerinfo`'s `subver` (Context, above); spec 24 §9 item 12 says
  some operators filter on it, and crawlers' use is UNVERIFIED (spec 14
  §8 item 2). Brief W3 requires that nothing dcr-purse sends gets it
  banned, throttled, disconnected or deprioritised; the interop gate of
  ADR-0014 runs against dcrd and dcroxide nodes, not against operators'
  filters.
- **Open point 6** is unaffected.

**Not settled by the owner's words, and the owner's to decide:** the
shape of the value under the new `dcr-purse` key (proposed: the same
fields as under `dcrwallet`), fixed when Phase 7 builds the reply.
*Decided (project owner, 2026-11-20): the shape is decided in Phase 7,
when the JSON-RPC server is written.*

## Appendix A — Void text, as accepted on 2026-11-18

Void since 2026-11-18 (ADR-0013). Kept verbatim as the record of what was
accepted; none of it is current.

**Options considered, "Import entry point".** A flag, as `--create`; or a
separate binary or subcommand (Phase 3's test binary made final). A binary
keeps the daemon's options identical to dcrwallet's apart from names (spec
21 §9 item 2), avoids the `import=1` config hazard, and keeps the bbolt
parser out of the process that holds keys; but the offer then needs the
importer in-process anyway or must point elsewhere. A subcommand breaks a
grammar that ignores positional arguments.

**Options considered, "Files".** Copy, reference in place, or generate.
Generating breaks clients that pinned `rpc.cert`. Referencing ties
dcr-purse to a directory the user may delete, where a missing `rpc.key`
makes it write a new certificate (spec 16 §5.1.2).

**Decision items 3 to 8.**

3. **Finding the source**, first match: (1) `--importfrom=<path>`, a
   dcrwallet directory or a database file; (2) `wallet.db` in dcr-purse's
   own `<appdata>/<net>/`; (3) the directory named by `appdata=` in
   dcrwallet's default `dcrwallet.conf`, else dcrwallet's default directory.
   The source config is `<dir>/dcrwallet.conf` for (1) and (2) and the
   default file for (3); a database file has none and carries nothing else.
   The network is the one the command line or an existing `dcr-purse.conf`
   selects, else the one the source config selects, so (3) finds what a
   plain `dcrwallet` opens. `--importfrom` without `--import` fails with
   `--importfrom requires --import`. Both are config-file keys.
4. **`dcr-purse --import`** imports, reports and exits, like `--create`,
   with no yes/no question:
   1. Refuse if dcr-purse has a wallet for the network.
   2. Open the source with one non-blocking lock attempt (W2,
      `crates/purse-boltread/src/db.rs`); on `Locked`, say dcrwallet has
      `<path>` open. Hash it (SHA-256); refuse schemas other than 26 with
      ADR-0007's text.
   3. Public passphrase: `--walletpass` (default `public`), then, on a
      terminal, `Enter public wallet passphrase: ` (spec 21 §9 item 27);
      never the private passphrase or seed.
   4. Print: `The wallet is copied into dcr-purse; the dcrwallet database is
      not changed. Going back to dcrwallet is clean only until dcr-purse
      changes the wallet. After that the original is stale, and keys,
      scripts and xpubs imported into dcr-purse cannot be recovered from
      the seed.` (brief §1). D14 may add one sentence on an export.
   5. Build the store under a temporary name; compare the logical exports
      of source and store (ADR-0001).
   6. Write carried files and the report under temporary names; re-check
      the source with `Db::verify_unchanged` and the hash.
   7. Rename the carried files, then the store. The store's rename is the
      commit point: before it no wallet exists, and files already renamed
      are identical copies a rerun reports as kept. Failure or interruption
      before it removes the temporaries; after a crash, the next start or
      import removes them and logs it.
5. **First-start offer.** When a start without creation flags, `--import`
   or `--noinitialload` would fail for want of a wallet and item 3 finds a
   source: on a
   terminal, name it, print the notice, ask
   `Import this wallet now? (n/no/y/yes) [no]: ` (spec 21 §5.4.6); yes runs
   item 4, reloads the configuration and continues. Otherwise print
   dcrwallet's message and `A dcrwallet wallet was found at <path>; run
   with --import to import it.`, and exit 1.
6. **Carried files**, copied, never over an existing file (reported
   "kept"); only `dcr-purse.conf` when the source directory is dcr-purse's.

   | File | Action |
   |---|---|
   | source config | to `dcr-purse.conf` if absent; 0600, or the source's mode if stricter; `appdata` lines commented out with a marker; a `cafile`, `rpccert`, `rpckey`, `clientcafile`, `dcrdclientcert`, `dcrdclientkey` or `logdir` value that expands (spec 21 §5.2.1) into the source directory or the source config's directory is rewritten to the same relative path under dcr-purse's, the original line commented out, and a file it names is carried |
   | `rpc.cert`, `rpc.key` | both or neither, 0600 |
   | `clients.pem`, `dcrd.cert`, `dcrd-client.cert`, `dcrd-client.key` | if present; keys 0600 |
   | `<net>/peers.json` | if present |
   | logs, `<net>/seed`, `testnet/` | no |

7. **Exit statuses of `--import`:** 0 done; 1 configuration or other
   failure; 3 source locked; 4 source refused (missing, foreign, schema not
   26, corrupt); 5 dcr-purse already has the wallet or holds it open; 6
   public passphrase wrong or needed without a terminal; 7 verification
   difference; 8 interrupted. The offer exits 1. 2 is skipped to avoid
   confusion with dcrwallet's panic status; a dcr-purse panic aborts
   (SIGABRT), under ADR-0012.
8. **Report**, on stdout and as `import-report.txt` (0600): source hash
   before and after, per-account values as read and stored (brief §6
   Phase 3), counts per ADR-0001 item kind, carried, kept and rewritten
   files, differences. No key material. Layout in the Phase 3 spec.

**Consequences, "Risks accepted".** `rpc.key` exists twice. `import=1` in a
config file fails every later start with 5, as `create=1` does. `--appdata`
naming a dcrwallet directory recreates (d)'s stale-database risk, accepted
because only an explicit choice produces it and brief §1 already accepts
that staleness. (b)'s reach to Decrediton-created wallets is UNVERIFIED.

**Consequences, "Constraints".** ADR-0001: the export covers everything the
report counts. ADR-0003: the store's rename is the commit point. ADR-0007:
the refusal text; headers and filters in the report. ADR-0009: the address
manager reads a v3.0.0 `peers.json`, or it is not carried. D14: at most one
sentence, decided before Phase 8 if any.

**Open points 3, 4 and 7.**

3. Should a path option inside the source be rewritten, or the import
   refused naming it? Proposed: rewritten.
4. If Decrediton keeps wallets under its own directory, should item 3
   search it? Proposed: decide in the Phase 8 addendum; until then
   `--importfrom`.
7. What switches (c)'s silent import on? Proposed: an explicit option,
   never the binary's name.
