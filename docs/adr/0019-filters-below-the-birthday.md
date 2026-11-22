# ADR-0019 — Committed filters below the wallet's birthday

- **Status:** **Accepted** (project owner, 2026-11-21): "we will drop
  filters below the wallet's birthday." That sentence is the whole of the
  owner's decision. Decision items 1 to 5 state what it means for the
  store, read from the source and from records already accepted. Where the
  source leaves a choice, this record does not make it: each such choice
  is an open question at the end (FB1 to FB10), **Proposed** with a
  recommendation. Not implemented: the rule belongs to the sync phases
  (brief §6 Phases 5, 6 and 10), and this record changes no code, no
  schema and no test.
- **Decided, 2026-11-22 (project owner):** FB1 to FB10, each as
  recommended, FB9 as its option (a). Shown a table of the ten questions
  with their recommendations, the owner answered "go with your
  suggestions for fb1-10"; that is the whole of the answer. Each question
  is marked, and "Addendum, 2026-11-22 — FB1 to FB10 decided as
  recommended" at the end states each rule once, as what dcr-purse is to
  do. Before answering, the owner was told that under FB7 a wallet
  created through Decrediton may see no saving until QK-0247 and QK-0554
  are dealt with, and that what Decrediton sends at creation was not
  examined; both still hold. Recording the answers, and the review of
  that record, found one conflict and one gap. The conflict: FB4 and
  decision item 1 as worded cannot both hold for a block connected
  before its child is known while a birthday set from time is pending,
  which may turn out to be the birthday block or to lie below it (the
  addendum's FB11). The gap: the rules decided for FB1, FB4, FB6 and FB9
  are keyed on the birthday height, which decision item 1 defines only
  once the birthday is resolved (the addendum's FB12). FB11 and FB12 are
  **Proposed**, each with a recommendation. Still not implemented: the
  addendum changes no code, no schema, no export format and no test.
  *(Both decided as recommended later the same day; next bullet.)*
- **Decided, 2026-11-22, later (project owner):** FB11 as recommended,
  option (i) with (r2), and FB12 as recommended, option (a). Shown FB11
  and FB12 in a table with those recommendations, and told that accepting
  them would let Phase 0 close, the owner answered "Accept both"; that is
  the whole of the answer. Each is marked, and "Addendum, 2026-11-22
  (later) — FB11 and FB12 decided as recommended" at the end restates the
  rules they change. Checking the two answers against each other and
  against the rules already decided found one case neither covers: a
  block the store holds without its filter, below the stored tip, that the
  first block a reorganisation attaches shows to be the birthday block
  (that addendum's FB13), **Proposed** with a recommendation. Still not
  implemented: the addendum changes no code, no schema, no export format
  and no test. *(FB13 decided as recommended later the same day, with a
  cut-off rule for questions found later; next bullet.)*
- **Decided, 2026-11-22, after FB11 and FB12 (project owner):** FB13 as
  recommended, option (a): the syncer fetches the fork-point block's
  filter, checks it and writes it as part of the same chain switch. The
  owner was shown three things with recommendations: FB13, option (a);
  ADR-0013's remaining proposals, accept as proposed; and a cut-off rule.
  The owner answered "Accept all three"; that is the whole of the answer.
  FB13 is marked, and "Addendum, 2026-11-22 (FB13) — FB13 decided as
  recommended, and the cut-off rule" at the end states the rule once.
  **The cut-off rule**, in the words the owner accepted: "Questions that
  arise in working out ADR-0019 are settled in the phase that implements
  it and do not count against Phase 0's exit." It covers questions that
  arise in working out this record and nothing else; it defers nothing in
  D1 to D10 that is not such a question. A question found later in
  working out this record is put to the owner, with a recommendation, in
  the phase that implements this record, and is settled there; this
  record's code lies in the Phase 3 port of `purse-udb`'s transaction
  store (FB4, FB9), in the sync phases (brief §6 Phases 5, 6 and 10), in
  the RPC servers for the filter RPCs of rules 3 and 5 (Phase 7 for
  `getcfilterv2`, `validatepredcp0005cf` and `importcfiltersv2`; Phase 12
  for gRPC `GetCFilters`), and in `purse-tools`' port of
  `cmd/repaircfilters`, whose phase the brief does not fix.
  Still not implemented: the addendum changes no code, no schema, no
  export format and no test. *(Later on 2026-11-22 the first slice of
  `purse-udb`'s transaction store implements the store's part of rules 4,
  9 and 10, with FB11 (i)'s widening of rule 4 and FB12 (a)'s pending
  states: which blocks `ExtendMainChain` accepts without a filter, and
  where `MissingCFiltersHeight` starts (`docs/PARITY.md`, `purse-udb`
  transaction store, item (1); `crates/purse-udb/src/txstore`). A
  `birth` row pending from both a height and a time, which this record
  does not name, is read there as below the birthday only for a block
  below both; that reading is put to the owner under the cut-off rule.
  Not implemented: the rest, among it FB12 (a) item 2's genesis filter,
  which that slice writes at creation, as dcrwallet does, and keeps
  whatever birthday is set later, and the projection of rule 6.)*
- **Date:** 2026-11-21. Citations of dcrwallet source are to the parity pin
  `54b85b33` (index, second paragraph).

## Context

**What dcrwallet stores.** Every block the wallet connects to its main chain
goes through `ExtendMainChain`, which writes an empty block record by
height, and the 180-byte header and the block's version-2 committed filter
by the block's hash (`wallet/udb/txmined.go:203-272`; the filter at
`:269-271`). It refuses to extend the chain unless the `havecfilters` row is
exactly 1 (`:208-211`), and it refuses block 0 (`:205-206`). The genesis
block's filter is written another way: when the database is created,
upgrade 15 (`blockcf2Upgrade`) builds it locally from the genesis block,
with nothing fetched and independent of any sync, and stores it under the
genesis hash (`wallet/udb/upgrades.go:1313-1323`); the same upgrade then
sets `havecfilters` to 1 because the tip is the genesis block
(`:1325-1333`). Spec 05 lists `wtxmgr/cf/<genesis hash>` among the rows of
a new database. Both sync modes fetch a filter for every header and
check it against the header's commitment before the block is connected: SPV
in the second stage of its initial sync (`spv/sync.go:1713-1773`,
`spv/backend.go:149-258`, the check at `:234-246`), RPC mode one
`getcfilterv2` per header (`chain/sync.go:250-271`, the check at
`:263-266`). Neither the header bucket nor the filter bucket is ever pruned
(ADR-0001 §6.10; spec 03 I1), so a wallet holds a filter for every block
from genesis to its tip, and for every block a reorganisation detached.

**What that costs.** Measured:

- ADR-0003, "Header and filter ingest (SPV initial sync)", on 1,050,000
  generated blocks, each with a generated filter of 270 bytes on average.
  That size is an assumption: mainnet's filter sizes were not measured.
  dcrwallet's bbolt file after the ingest was 1,020 MiB; the spike's
  prototype was 629 MiB with every commit durable and 1,079 MiB with
  relaxed commits.
- ADR-0003, addendum "Phase 2 measurements", table "Headers and filters":
  the same number of blocks through `purse-store`'s height layout, each
  with a generated filter of 170 to 369 bytes (270 on average, the same
  assumption), left a file of 1,088 MiB, 1,029 MiB after close.
- ADR-0014, Context, evidence [S]: a dcrwallet SPV restore on mainnet from a
  random seed without a birthday, to height 1,133,862, left a file of 1.42
  GB; it received 205 MB of headers and 453 MB of filter messages (about 400
  bytes a block, framing and inclusion proofs included, so the stored filter
  sizes cannot be read off it exactly), and spent 2,030 s of its 2,263 s in
  headers and filters.

The owner was told that a wallet never needs to rescan blocks older than its
first key, and that keeping only headers below the birthday would cut a new
wallet's file to roughly 200 MiB. That premise does not hold at the pin for
imported material: an extended public key imported at creation or by
`importxpub`, imported private keys, public keys and scripts, and voting
accounts imported from a seed can all have history below the wallet's
birthday, which only a rescan or a discovery from below it finds (items 5
and 6 of "What reads stored filters"). A birthday typed at a restore that
is later than the wallet's first use would be another case; it was not
examined. Separately, dcrwallet's rescans and discoveries start at height 0
or at the genesis block by default, so with its defaults a wallet reads
filters below its birthday whether or not it needs them (items 5 and 6;
FB2). The 200 MiB figure is an estimate, not a measurement. The headers
alone are 189,000,000 bytes (180 MiB) at 1,050,000 blocks; the block
records, the hash index and the engine's page overhead come on top of them,
so the file will be larger than the headers' payload, by an amount that was
not measured.

**The decision this extends.** ADR-0014's candidate S15, "No filters below a
typed birthday", was decided as recommended on 2026-11-19 (its Open point
5): the wallet tells its peers it has no interest before a date it was
given, "fetching on demand for rescans below it". S15 spoke of a birthday
the user typed at a restore and of what the initial sync fetches. This
record speaks of the wallet's birthday however it was set, a new wallet's
included, and of what the store keeps.

**Why it needs a record.** The filters dcrwallet persists are wallet state
the logical export covers (ADR-0001 §6.10, and the coverage table
"Everything dcrwallet persists"), which ADR-0012 Rule 1 counts as a surface.
ADR-0001's decision on chain data in comparisons rested on the store keeping
every header and filter (its "Decision by the project owner", item 5), and
the brief's SPV exit compares filters with a Go wallet's (§6 Phase 10).
Dropping them is a deliberate departure from what dcrwallet stores.

**Why a record of its own.** The index's convention is one record per
decision, with addenda for what follows from a decision already recorded.
This one answers no open point of ADR-0003 or ADR-0014: it is a new question
with its own options and consequences, spanning ADR-0003 (what the
`cfilters` tables hold), ADR-0014's S15 (what the initial sync fetches) and
ADR-0001 (what a comparison sees). Each of those gains a dated pointer here.

## Options considered

- **K. Keep every filter, as dcrwallet does.** Every read the wallet makes
  is served from the store, and no rule depends on the birthday. The file
  is the size measured above.
- **D. Keep headers for every block and filters only from the birthday on.**
  Chosen. It saves the filters below the birthday, nearly all of them for a
  new wallet. It costs a set of rules that depend on the birthday (items 3
  and 7 below; FB1, FB4, FB6, FB8, FB9), and for every range below the
  birthday it goes to the network exactly as F does everywhere: S15 decided
  that for rescans ("fetching on demand for rescans below it"), and FB2 and
  FB3 recommend it for discovery and the filter RPCs. *(2026-11-22: FB2
  and FB3 decided so, project owner; addendum of that date.)*
- **F. Keep no filters and fetch each one when it is read.** Mentioned to
  the owner and not chosen. Its strengths: one rule for every block, with
  none of the handling that depends on the birthday, and the smallest file.
  Against D the file is only slightly smaller for a recent birthday: D
  keeps the filters from the birthday on, about 288 for a new wallet whose
  birthday is dcrwallet's default, a day behind the tip, and about 105,000
  more for each year the wallet runs, some 28 MB a year at ADR-0003's
  assumed 270-byte average (an estimate, at five-minute blocks). Its
  weakness: every rescan, discovery,
  reorganisation and filter RPC goes to the network (the readers are listed
  under "What reads stored filters" below), the rescans above the birthday
  that D serves from the store included.

## Decision (accepted, 2026-11-21)

The decision is the owner's sentence quoted in the Status line. The items
below state what it means for the store; none of them makes a choice the
sentence leaves open.

1. **What is kept and what is not.** The *birthday height* is the height in
   the store's `birth` row once the birthday is resolved, that is with
   neither of its "set from height" and "set from time" flags (dcrwallet's
   `BirthdayState`, `wallet/udb/txmined.go:323-394`; spec 09 §3.2). The
   store keeps:
   - the header of every block, main-chain (`headers`, by height) and
     detached (`headers_detached`, by hash), as now;
   - every block record and the hash index (`blocks`, `block_by_hash`), as
     now;
   - the committed filter of every main-chain block at or above the birthday
     height (`cfilters`), and of every detached block whose header states a
     height at or above it (`cfilters_detached`) *(2026-11-22, later:
     apart from blocks detached while a birthday set from time was
     pending that the store held without a filter, whose filters are not
     fetched when the birthday then resolves at or below them, FB11,
     project owner)*;
   - the `birth` and `have_cfilters` rows.

   No other filter the wallet obtains itself is kept: none the syncers
   fetch, none the missing-filter pass fetches (item 9 of "What reads
   stored filters" below), and not the genesis block's filter, which
   dcrwallet writes at creation (Context), unless the birthday height is 0.
   For a wallet born after genesis that filter is one of the filters below
   the birthday, and the owner's sentence drops it with the rest. Filters a
   client supplies (`importcfiltersv2`, `cmd/repaircfilters`; item 11) are
   not settled by this item: whether the sentence covers them is the first
   part of FB5. *(2026-11-22: it does not; they are written as dcrwallet
   writes them, FB5, project owner.)* A wallet whose birthday is the
   genesis block keeps every filter, exactly as dcrwallet does; which
   wallets those are is item 2.
   Of the filters that can lie below a birthday, the genesis filter is the
   only one that needs no fetch: it is built locally from the genesis
   block, as upgrade 15 builds it. A store whose birthday is genesis
   therefore holds it although no block connection writes it, written at
   creation as dcrwallet writes it or, for a birthday pending at creation
   that resolves at genesis (item 3: a height birthday of 0, or a time
   birthday earlier than block 1's timestamp), when it resolves.
   *(2026-11-22, later: for a height birthday of 0, at creation instead,
   as dcrwallet writes it, FB12 (a) 1, project owner; a time birthday that
   resolves at genesis still gets it when it resolves, through
   `put_cfilter_at`, FB11 (i).)* The
   headers stay because difficulty validation walks back over stored headers
   (`wallet/difficulty.go:54`, `:593`, `:864`), the filter checks of the
   missing-filter fetch read them (`wallet/wallet.go:1377`), and
   `getblockheader` returns them, detached ones included (ADR-0003 Decision
   item 5 and Open point 5).
2. **Both sync modes.** The store's contents do not depend on the mode it
   was synced in, and one wallet may be run in either. Whether the filters
   below the birthday are fetched at all during the initial sync is open
   point FB1. *(2026-11-22: they are not, in either mode, FB1, project
   owner.)*
3. **No schema change.** `purse-store`'s chain tables already allow a
   main-chain block without its filter and a detached header without its
   filter (`crates/purse-store/src/chain.rs`, invariants 3 and 7; ADR-0003,
   addendum "Chain tables keyed by height, as built"). Schema version 1 is
   unchanged. What the `have_cfilters` row then means is FB9. *(2026-11-22:
   "no filter at or above the birthday height is missing", 1 in every
   store dcr-purse creates, FB9 option (a), project owner.)*
4. **A missing filter is never read as "no match".** dcrwallet's readers
   fail on a missing filter rather than skip the block: `fetchRawCFilter2`
   returns kind `NotExist`, "no cfilter saved for block <hash>"
   (`wallet/udb/txdb.go:1992-1999`), and `GetMainChainCFilters`
   (`txmined.go:525-528`), the SPV rescan (`spv/backend.go:528-532`) and
   `ChainSwitch` (`wallet/chainntfns.go:134-137`) return it. In dcr-purse
   every reader that matches a filter below the birthday against the
   wallet's addresses and outpoints (the rescans and discoveries of items 5
   and 6, the SPV announcement scan of item 7) or returns one to a client
   (the filter RPCs of item 10) either obtains the filter (the announcement
   path of item 7, which fetches it as dcrwallet does; FB2, FB3) or fails;
   none treats the block as one without a match, which would hide the
   wallet's own transactions (brief §10 R8), an outcome W1 forbids (an
   output not recognised, ADR-0012 Rule 7) and ADR-0014 never trades (§1,
   tier 1). Two kinds of read neither match nor serve a filter, and the
   rule does not apply to them. `ChainSwitch`'s read when it detaches a
   block only carries the stored filter into the side-chain node it
   rebuilds; below the birthday dcr-purse rebuilds that node without one,
   neither fetching nor failing (item 7). *(2026-11-22, later: while a
   birthday is pending, also for every main-chain block the store holds
   without a filter, FB11, project owner.)* No reader would use a filter
   fetched there: the SPV announcement path fetches a fresh filter for
   every node of the new best chain (`spv/sync.go:1488-1519`), RPC mode one
   for every announced block (`chain/sync.go:860-869`), and the store keeps
   none below the birthday; a later reader that matches or serves that
   block's filter falls under the rule above. The presence checks of the
   missing-filter pass (`MissingCFiltersHeight`, the pass's own
   `CFilterV2` reads, and `InsertMissingCFilters`' read of a parent's
   filter; item 9) only test whether a filter is stored; what they do is
   FB9. *(2026-11-22: under FB9 (a) the pass returns before walking, and
   `MissingCFiltersHeight` walks from the birthday height, project owner.)*
5. **How the divergence is recorded.** In `docs/PARITY.md`, under
   "Deliberate divergences from dcrwallet", as an owner decision about
   dcr-purse's own store, beside the database decision. It has no
   `docs/QUIRKS.md` entry: it is not a dcrwallet defect, and the owner's
   earlier design departures (the database, the P2P user agent) have none.
   Each RPC-visible difference that follows from the answers to FB2, FB3,
   FB5 and FB9 gets its own PARITY.md row when the code is written, under
   ADR-0012 Rule 10. *(2026-11-22: with the answers given, the addendum
   lists the differences those rows are to record; none is written yet,
   since no code is. The existing row names the answers.)*

## What reads stored filters in dcrwallet, and what dcr-purse must do

Found by reading every reference to `CFilterV2`, `CFiltersV2`,
`GetMainChainCFilters`, `RangeCFiltersV2`, `havecfilters`, `MissingCFilters`
(which also matches `IsMissingMainChainCFilters` and
`InsertMissingCFilters`), `fetchRawCFilter2`, `putRawCFilter` and
`BirthState` at the pin, tests excluded. Besides the readers of stored
filters, item 7 covers the one path that scans freshly fetched filters, the
SPV announcement path, because a reorganisation can bring blocks below the
birthday into it. Nothing here was run.

1. **The initial sync does not read stored filters.** It fetches, checks and
   writes them (Context above). Header-chain validity does not depend on
   them: the SPV syncer validates each batch's proof of work and stake
   difficulty (`ValidateHeaderChainDifficulties`, `spv/backend.go:353`;
   `wallet/difficulty.go:837-890`) and marks checkpoint mismatches
   (`spv/backend.go:340-341`) before any filter is requested, and the filter
   check (`validate.CFilterV2HeaderCommitment`,
   `validate/validate.go:71-102`) proves a filter against a header, never
   the reverse. On mainnet below height 431,488 and on testnet3 below
   323,328 the check accepts any filter without a proof
   (`validate/validate.go:19-25`, `:81-89`; QK-0242, QK-0428), so a wallet
   whose birthday is above those heights stores no filter that could not be
   checked. Skipping the filters below the birthday therefore weakens no
   header check. What it saves in sync time is bounded by the share of
   filter fetching in ADR-0014's 2,030 s of headers and filters, which was
   not separated from header fetching; the header stage alone had 1,085 s of
   wire time (ADR-0014, Context). One effect near the tip: SPV's two-minute
   filter watchdog, which prunes the side-chain forest when no peer serves a
   batch's filters (`spv/backend.go:175-205`), runs only where filters are
   fetched, so it does not run for blocks below a birthday that lies above
   the tip (item 3). In RPC mode, blocks that arrive by notification get
   only the filter proof and the checkpoint check (QK-0403); those blocks
   are at the tip, above any resolved birthday, except after a
   reorganisation across it (item 7).
2. **Which wallets have a birthday above genesis.** Every database at the
   pin starts with the birthday resolved at genesis (the 25-to-26 upgrade,
   `wallet/udb/upgrades.go:1665-1689`). Creation then sets it as follows
   (spec 09 §3.2): a new seed through `--create`, now less 24 hours, set
   from time (`walletsetup.go:192-196`); a restored seed through `--create`,
   the date or block number the user typed (`walletsetup.go:166-191`), or
   genesis when the user typed none (`:198-204`); gRPC `CreateWallet`,
   genesis, or the request's `birth_height` when `set_birth_time` is set,
   its time discarded (`internal/rpc/rpcserver/server.go:2713-2752`;
   QK-0247, QK-0554); watching-only and simulation wallets keep genesis.
   Only a new seed and a restore with a typed date or block drop anything. A
   wallet created through gRPC, Decrediton's route, keeps every filter
   unless its client sends `set_birth_time` with a height (FB7).
   Decrediton's requests were not examined. *(2026-11-22: FB7 decided as
   recommended: such wallets keep every filter; still not examined.)*
3. **The birthday while it is pending.** `ChainSwitch` connects each block
   first (`extendMainChain`, `wallet/chainntfns.go:177`) and only then tests
   whether that block resolves a pending birthday (`:186-189`): set from
   height H, block H+1 resolves it, at H; set from time T, the first block
   with a timestamp after T resolves it, at that block's parent. It then
   moves the processed-transactions marker to the birthday block
   (`:190-205`). Every block connected while the birthday is pending is
   therefore at or below the birthday block once it resolves, except the
   last one, the block that resolves it: that block is the birthday block's
   child, one above it, and the first rescan starts there (item 4), so its
   filter is kept under decision item 1. With a height birthday the
   birthday block is known when it arrives. With a time birthday it is
   known only when its child arrives, so the syncer keeps the birthday
   block's filter by deciding one block late or by fetching it then (FB8).
   *(2026-11-22: one block late, FB8, project owner; for a block connected
   before its child is known, the addendum's FB11, decided later that day:
   connected without its filter, which is written when its child shows it
   to be the birthday block, (i) with (r2).)*
   A birthday above the tip stays pending ("searches until it is passed",
   `:208-209`), the marker stays at genesis, where creation put it
   (`wallet/udb/upgrades.go:909`), and the first rescan starts at height 1
   (item 4); dcr-purse holds no filters below the tip in that state, and the
   rescan's reads fall under FB2. *(2026-11-22: fetched on demand, FB2.)*
4. **The initial discovery and rescan start above the birthday.** Both sync
   modes start discovery and the rescan at the rescan point, the main-chain
   child of the processed-transactions marker (`wallet/rescan.go:476-498`;
   `spv/sync.go:1850-1900`; `chain/sync.go:354-383`), which item 3 puts at
   the birthday block, so once the birthday has resolved they read filters
   from the block above it on. The rescan never scans the genesis block
   (`wallet/rescan.go:246-253`).
5. **Rescans from a height the client gives.** `rescanwallet` starts at 0
   when no height is given (`internal/rpc/jsonrpc/methods.go:4223-4241`);
   `importprivkey`, `importpubkey` and `importscript` rescan by default,
   from 0 unless told otherwise (`:2008-2062`, `:2073-2120`, `:2128-2172`);
   gRPC `Rescan` starts where asked
   (`internal/rpc/rpcserver/server.go:442-476`), `ImportExtendedPublicKey`
   from `scan_from` (`:706-729`), and `ImportPrivateKey`, `ImportScript` and
   `ImportVotingAccountFromSeed` always from 0 when they rescan, since a
   nonzero `scan_from` with `rescan` set is refused (QK-0531). All go
   through `RescanFromHeight` or `RescanProgressFromHeight` to `rescan`
   (`wallet/rescan.go:212-356`), which hands batches of main-chain block
   hashes to the backend (`:323`). In SPV mode the backend reads the stored
   filter of every block in the batch first and fails at the first missing
   one (`spv/backend.go:523-535`). In RPC mode it sends the hashes to dcrd's
   `rescan` method and reads no stored filter (`chain/backend.go:53-55`;
   `rpc/client/dcrd/calls.go:438-468`). So in RPC mode nothing changes; in
   SPV mode, with dcrwallet's defaults and with every gRPC import, these
   rescans read every filter from height 1. An extended public key imported
   at creation (`walletsetup.go:206-212`) or by `importxpub`, which never
   rescans (`methods.go:2179-2192`), has its history below the birthday
   found only by such a rescan, in dcrwallet as in dcr-purse. S15 answered
   this for a typed birthday: fetch on demand (FB2). *(2026-11-22: for
   every birthday, FB2, project owner.)*
6. **Discovery from a block the client gives.** `discoverusage` and gRPC
   `DiscoverUsage` start at the genesis block unless given another
   (`methods.go:824-855`; `server.go:4286-4310`), and
   `ImportVotingAccountFromSeed` with `discover_usage` always does
   (`server.go:840-860`; QK-0531). In SPV mode account discovery reads the
   stored filters from that block through `filterBlocks`
   (`wallet/discovery.go:363-415`), and address discovery loads every stored
   filter from it to the tip at once (`:185-197`), both through
   `GetMainChainCFilters`, which fails at the first missing one
   (`wallet/udb/txmined.go:505-541`). In RPC mode both use dcrd's
   exists-address index and read no filter (`wallet/discovery.go:765-771`,
   `:830-836`; `chain/backend.go:87-89`). S15 did not name discovery (FB2).
   *(2026-11-22: discovery fetches on demand as rescans do, FB2, project
   owner.)*
7. **Reorganisations around the birthday.** A reorganisation whose fork
   point lies below the birthday meets blocks below it on four paths.
   - *Detaching.* `ChainSwitch` reads the stored filter of every main-chain
     block it detaches, to rebuild its side-chain node, and fails if one is
     missing (`wallet/chainntfns.go:125-149`, the read at `:134`). A
     detached block below the birthday has no stored filter under decision
     item 1, so dcr-purse rebuilds its node without one, the exception
     decision item 4 names; failing instead would refuse a chain switch
     dcrwallet makes. Reconnecting a block needs
     its filter for the store, since `ExtendMainChain` serializes it
     (`txmined.go:269-271`; spec 09 §3.3: a nil filter panics); below the
     birthday the store keeps none (where that rule lives is FB4;
     *2026-11-22: in the syncers, FB4, project owner*). *(2026-11-22,
     later: while a birthday is pending, the exception covers every
     main-chain block the store holds without a filter, below the
     birthday or not, FB11, project owner; for a block held without its
     filter that a reorganisation shows to be the birthday block, FB13,
     Proposed; decided later the same day as option (a), its filter
     fetched, checked and written with the chain switch, project owner.)*
   - *The SPV initial sync.* Its second stage fetches the filters of every
     node of the new best chain from the first one lacking a filter
     (`spv/sync.go:1736-1748`; the forest copies a filter between duplicate
     nodes, `wallet/sidechains.go:94-99`), and its third stage connects the
     batch without scanning it (`:1774-1811`). In dcrwallet a node rebuilt
     from the store carries its filter and is not fetched again. In
     dcr-purse a rebuilt node below the birthday carries none, and Go's
     selection rule, which takes the nodes from the first one without a
     filter on (`:1738-1743`), would fetch it again together with every
     node after it, although in the initial sync only the store would read
     the filter and the store keeps none below the birthday. Which nodes the
     stage fetches in dcr-purse, the rebuilt ones and the other nodes below
     the birthday included, is part of FB1. *(2026-11-22: from the first
     node at or above the birthday that lacks a filter, none below it,
     FB1, project owner.)*
   - *SPV after the initial sync.* Announced blocks go through
     `handleBlockAnnouncements` (`spv/sync.go:1381`). It fetches the
     filters of every node of the new best chain, whether or not a node
     already holds one (`:1488-1508`), checks each against its header and
     sets it on the node (`:1510-1519`), and, when the rescan point is nil,
     that is once the wallet has processed transactions through its tip,
     scans all of them against the wallet's addresses and outpoints
     (`:1522-1540`; `scanChain`, `:1290-1370`, which reads each node's
     filter with no nil check, `:1321-1323`) before `ChainSwitch` records
     the matches with the reconnected blocks (`:1542`). With a fork point
     below the birthday, the new best chain includes blocks below it.
     dcr-purse fetches, checks and scans those filters as dcrwallet does and
     does not store them (decision item 1); it never treats such a block as
     one without a match (decision item 4), and a filter it cannot fetch
     fails the announcement, as in dcrwallet (`:1503-1508`). These requests
     are the ones dcrwallet makes, so they disclose nothing more. With the
     rescan point not nil the filters are fetched and checked but not
     scanned; dcrwallet then stores them all, dcr-purse those at or above
     the birthday only. The "must not fetch" question of FB1 is about the
     initial sync only: on this path not fetching would leave dcr-purse no
     filter to scan with.
   - *RPC mode.* Each block dcrd announces has its filter fetched and
     checked before it joins the forest (`chain/sync.go:834-883`, the fetch
     at `:860`); the wallet's transactions in it come from dcrd's
     notification, not from the filter. After a reorganisation across the
     birthday some announced blocks are below it; dcr-purse fetches and
     checks their filters as dcrwallet does and does not store them.

   Such a reorganisation needs a fork point below the birthday: for a
   wallet created with dcrwallet's default, its birthday a day behind the
   tip, about 288 blocks deep. In SPV mode `handleBlockAnnouncements`
   refuses a first announced header more than 256 blocks below the tip
   (`spv/sync.go:1400-1405`), so such a side chain builds up over several
   announcements: unlikely, not ruled out. If the birthday block itself is
   detached, dcrwallet leaves the `birth` row naming it, since nothing
   writes the row after creation except the resolution of item 3; dcr-purse
   keeps the same row, which the restore contract compares (ADR-0001, "The
   restore contract"), and applies decision item 1 by its height.
8. **A birthday that is unset, or moved earlier.** `BirthState` reads as
   unset for a row that is not 45 bytes long, an absent row included
   (`txmined.go:376-379`), which no database at the pin holds (item 2);
   what dcr-purse does with a store that has no `birth` row is FB10
   (*2026-11-22: treated as genesis, FB10, project owner*). No
   surface at the pin moves a resolved birthday: `SetBirthState` is called
   only at creation (`walletsetup.go:255`; `server.go:2749`), and
   `SetBirthStateAndScan` has no caller (`wallet/rescan.go:532-581`;
   QK-0243). dcr-purse needs no path that backfills filters after the
   birthday moves; a pin move that adds such a surface reopens this item.
9. **`have_cfilters` and the missing-filter fetch.** At every start both
   syncers run `FetchMissingCFilters` first (`spv/sync.go:412-424`;
   `chain/sync.go:655-667`). It does nothing unless the row is absent or 0
   (`txmined.go:400-403`); then it starts at the first main-chain block
   without a filter, found by `MissingCFiltersHeight` walking from genesis
   (`txmined.go:405-421`; `wallet/wallet.go:1291-1415`). That walk returns
   `NotExist` when no main-chain block lacks a filter (`txmined.go:420`),
   which the pass returns as an error (`wallet.go:1295-1305`) and both
   syncers abort on (`spv/sync.go:418-420`; `chain/sync.go:660-663`). The
   pass fetches and checks filters to the tip in spans of 2,000
   (`wallet.go:1316-1414`) and records each span through
   `InsertMissingCFilters` (`txmined.go:430-490`), which reads the stored
   filter of the span's first block's parent and refuses the span if it is
   missing ("block hashes are not ordered or previous cfilters are
   missing", `:461-469`; a span starting at the genesis block is exempt,
   `:447`), and writes 1 to the row when the span ends at the tip
   (`:479-487`). The pass's loop ends only when it reads the row as 1 again
   (`wallet.go:1326-1328`, `:1359-1361`). The started and finished sync
   notifications are sent even when nothing is missing
   (`spv/sync.go:239-255`; gRPC `FETCHED_MISSING_CFILTERS_STARTED` and
   `_FINISHED`, `server.go:2885-2905`, `:3049-3069`). Under dcrwallet's
   meaning of the row, a store without the filters below the birthday
   holds 0, and the walk from genesis would name the lowest height without
   a filter, 0 for a wallet born after genesis (decision item 1), from
   which the pass would fetch every filter below the birthday. What the
   row means in dcr-purse, and so what the pass, `InsertMissingCFilters`
   and the udb-level `MissingCFiltersHeight` (which ADR-0002's operation
   scripts can call, `store.missing_cfilters_height`) do, is FB9. Under its
   recommendation the row stays 1, the pass never walks or fetches, and the
   notifications are sent as dcrwallet sends them. *(2026-11-22: decided
   as recommended, FB9 option (a), project owner.)*
10. **Filters served to clients.** `getcfilterv2` returns the stored filter
    of a block by hash (`methods.go:1955-1977`; `wallet/wallet.go:833-842`);
    gRPC `GetCFilters` streams the stored filters of a range, the whole main
    chain by default (`server.go:2179-2211`; `wallet.go:855-878`);
    `validatepredcp0005cf` hashes every stored filter below the DCP0005
    height and compares the hash with a constant (`methods.go:5392-5399`;
    `wallet.go:5230-5268`; `validate/validate.go:107-130`; QK-0215). Each
    reads a filter below the birthday of any wallet born after its range
    starts (FB3; *2026-11-22: fetched on demand and checked; with no
    backend connected, the missing-filter error; project owner*).
    dcrwallet serves no filter to peers: it never sends
    `cfilterv2` or `cfiltersv2` (spec 14 §3.3).
11. **Filters a client supplies.** `importcfiltersv2` writes the filters it
    is given at consecutive main-chain heights, unchecked against the
    headers and without touching `havecfilters` (`methods.go:1981-2002`;
    `wallet/wallet.go:5278-5287`; `wallet/udb/cfilters.go:34-67`; QK-0061,
    QK-0106). `cmd/repaircfilters`, which `purse-tools` is to port (brief
    §5), reads a file of pre-DCP0005 filters and imports them from height 0
    in batches (`cmd/repaircfilters/main.go:186-237`), for
    `validatepredcp0005cf` to check afterwards. On a dcr-purse wallet born
    after height 0 those writes land below the birthday (FB5;
    *2026-11-22: written as dcrwallet writes them, project owner*).
12. **The logical export and paired comparisons.** A Rust export of a wallet
    born after genesis lists a `cfmissing` record for every main-chain
    height below the birthday, where Go's lists a `cfilter` record (or a
    `cfseg` digest) for each (ADR-0001 §6.10); the `sidecfilter` records of
    detached blocks below the birthday are absent on the Rust side; the
    4096-height segment holding the birthday digests different items.
    `[headers]` and `[blocks]` are unaffected. `havecfilters` agrees (1 on
    both sides) under FB9's recommendation and differs (0 against 1) under
    its alternative. Restore differentials, the scripted-history replays
    and the paired-wallet soaks (ADR-0001, Context) can all compare wallets
    in that state, so each needs a projection, which ADR-0001's open item 2
    already asks for wallets that synced different header ranges (FB6).
    *(2026-11-22: FB6 and FB9 (a) decided as recommended, project owner:
    `havecfilters` is 1 on both sides, and the projection is decided and
    not yet specified.)*
13. **Backup and restore.** A backup is the snapshot of every table copied
    into a new store (ADR-0018 Decision item 1), so it carries exactly the
    filters the store holds and is smaller by the filters not held (not
    measured); a store restored from it keeps its birthday and its filters,
    and verify's chain checks already accept blocks without filters
    (decision item 3). A wallet restored from its seed instead gets the
    birthday its creation sets (item 2), genesis unless the user types one,
    and then keeps every filter. Nothing in ADR-0018 changes.

## What is not measured

- Mainnet's real filter sizes: ADR-0003's 270-byte average is an assumption,
  and ADR-0014's 453 MB of filter messages include framing and proofs.
- The size of a store that keeps headers for every block and filters only
  from a recent birthday on; the "roughly 200 MiB" is an estimate (Context).
- Sync time with the filters below the birthday skipped, in either mode.
- The cost of fetching filters on demand for a rescan or a discovery from
  below the birthday, which with dcrwallet's defaults starts at height 1
  (items 5 and 6).
- The size of a Rust export of such a wallet (item 12).
- *(2026-11-22: the answers to FB1 to FB10 add items, listed in the
  addendum of that date under "Not measured, not examined, not done".)*

Nothing was run for this record: no oracle run, no benchmark, no timed
measurement, and no live sync in either mode; the reorganisation paths of
item 7 were read in the source, not exercised.

## Consequences

- The SPV and RPC syncers (brief §6 Phases 5, 6 and 10) decide, per block,
  whether its filter is fetched and passed to the store; the rescan, the
  discovery and the filter RPCs gain a path for filters below the birthday
  once FB2 and FB3 are answered (*2026-11-22: answered, project owner:
  fetch on demand, check, keep none; with no backend connected, the
  filter RPCs return the missing-filter error*). Each path that
  matches a filter against
  the wallet or returns one to a client gets a test that a missing filter
  is fetched or fails and is never skipped (decision item 4), the SPV
  announcement path after a reorganisation across the birthday included
  (item 7). The detach path of `ChainSwitch` gets a test that a block
  below the birthday is detached with its side-chain node rebuilt without
  a filter, and that a later scan of that block on the announcement path
  fetches its filter. *(2026-11-22: the tests the answers add are listed
  in the addendum of that date, "What the code owes".)*
- Comparisons with Go of a wallet born after genesis run under the
  projection of FB6; until it is specified, such comparisons use wallets
  whose birthday is genesis, which keep every filter. In the brief these
  are the Phase 4 and Phase 6 exits, the Phase 8 (M3) soak, the Phase 10
  (M5) exit and its §11 row, the 1.0 exit, and §7 Layers 6 and 7, each of
  which carries a dated pointer here. *(2026-11-22: FB6 decided as
  recommended, project owner; the projection is specified in ADR-0001
  with the Rust producer of `[cfilters]`, so until then the sentence
  stands. "Phase 6" in it, and in the next item, is a slip for Phase 5,
  whose exit carries the pointer; Phase 6's does not. A wallet whose
  birthday is genesis keeps every filter only once it holds the genesis
  filter: a seed restored without a birthday, whose birthday dcrwallet
  sets from height 0, gets it when block 1 resolves the birthday under
  decision item 1 as written, so a comparison before block 1 still
  differs; the addendum's FB12 recommends writing it at creation.)*
  *(2026-11-22, later: decided so, FB12 (a) 1, project owner; such a
  wallet holds the genesis filter from creation, as Go's does.)*
- Amended with dated pointers by this record: ADR-0003 (Status, the Context
  paragraph on restore and rescan, the `meta` and `cfilters` rows of
  Decision item 2, Open point 5, and an addendum), ADR-0001 (Status, open
  item 2 of "What remains open" and item 5 of the owner's decision),
  ADR-0014 (Status, S15 and Open point 5, amendment 5), `docs/PARITY.md`
  (deliberate divergences), the brief (the Phase 4, 6, 8, 10 and 1.0 exits,
  §7 Layers 6 and 7, R8 and the §11 M5 row), the README and the ADR index.
  The doc comments that give dcrwallet's meaning of the `have_cfilters`
  row, those of `purse_store::MetaKey::HaveCFilters`
  (`crates/purse-store/src/keys.rs`), of `ReadTx::have_cfilters`,
  `WriteTx::have_cfilters` and `WriteTx::set_have_cfilters`
  (`src/txn.rs`), and of the chain tables (`src/chain.rs`,
  `src/schema.rs`), change with the code that implements this record if
  FB9 is answered as recommended, and stay as they are otherwise.
  *(2026-11-22: FB9 was answered as recommended, so they change with that
  code; until it is written they give dcrwallet's meaning, which is the
  store's as built.)*

## Open questions for the project owner

Each with a recommendation. *All ten decided as recommended (project
owner, 2026-11-22), FB9 as option (a); each is marked, its options and
reasoning kept as the record of what was chosen between, and the addendum
of that date states the rules.* *FB11 and FB12, found while recording
those answers, were decided as recommended later the same day; the
addendum "2026-11-22 (later)" restates the rules they change and puts
FB13, found in checking them, **Proposed**.* *FB13 was decided as
recommended later the same day, option (a); the addendum "2026-11-22
(FB13)" states its rule and records the cut-off rule the owner accepted
with it, under which a question found later in working out this record
is settled in the phase that implements it.*

- **FB1. Fetching below the birthday during the initial sync.**
  **Decided as recommended (project owner, 2026-11-22); for the birthday
  height while a birthday is pending, the addendum's FB12.** The store
  keeps none of these filters; the question is whether the initial sync
  asks for them. It does not cover the announcement paths after the initial
  sync, where dcrwallet fetches the filters of every block it connects and
  dcr-purse must too (item 7). Skipping them saves the transfer and its time
  (item 1). What skipping discloses depends on the mode:
  - In SPV mode dcrwallet asks its peers for every filter from genesis (the
    second stage, `spv/sync.go:1713-1773`), so those requests look the same
    for every wallet at a given tip, and its rescan reads the stored filters
    locally and asks peers only for the blocks that match. Skipping tells
    the peers roughly when the wallet was born: for a new wallet, that it is
    new. ADR-0014 makes such a disclosure the owner's call (§1, tier 3),
    and S15 accepted it for a typed birthday.
  - In RPC mode, once the birthday has resolved, dcrwallet's first `rescan`
    call already sends dcrd the main-chain hashes from the block after the
    birthday on (`chain/sync.go:354-379`; `wallet/rescan.go:233-236`,
    `:323`; `chain/backend.go:53-55`; `rpc/client/dcrd/calls.go:447`; item
    4), so skipping the `getcfilterv2` calls below the birthday tells dcrd
    nothing it does not learn from dcrwallet. The exception is a birthday
    still pending above the tip (item 3): dcrwallet's rescan then starts at
    height 1 and does not reveal the birthday, while skipping every filter
    below the tip would reveal that it lies above the tip.

  Fetching and checking them and then discarding them discloses nothing
  beyond what dcrwallet discloses and saves only space. The answer also
  sets which nodes the SPV second stage selects after a reorganisation has
  rebuilt side-chain nodes below the birthday without filters (item 7):
  when skipping, the stage selects from the first node at or above the
  birthday that lacks a filter and fetches none below it; when fetching,
  checking and discarding, Go's rule holds unchanged, from the first node
  without a filter on (`spv/sync.go:1738-1743`), so the rebuilt nodes and
  every node after them are fetched again. Recommended: skip
  them in both modes, for every birthday, as S15 does for a typed one: in
  SPV mode under the disclosure S15 accepted, in RPC mode at no disclosure
  once the birthday has resolved, and, while a birthday is pending above
  the tip, at a disclosure to dcrd of the kind S15 accepted towards peers.
  If the owner prefers dcrwallet's disclosure in that one state, the RPC
  syncer fetches, checks and discards the filters until the birthday
  resolves. *(2026-11-22: not taken; the recommendation was decided, so
  the RPC syncer skips them in that state too, project owner.)*
- **FB2. Rescans and discoveries from below the birthday in SPV mode**
  (items 5 and 6). **Decided as recommended (project owner,
  2026-11-22).** S15 decided, for a typed birthday, to fetch on demand for
  rescans below it. Recommended: the same for every birthday and for
  discovery: fetch the filters of the range from peers, check those above
  the DCP0005 heights against the stored headers as the initial sync does,
  accept those below as dcrwallet's sync accepts them, use them, and keep
  none of them, so that decision item 1 holds afterwards. A rescan with
  dcrwallet's defaults then fetches about a million filters on mainnet each
  time it runs (not measured). The genesis filter, which `discoverusage`
  and `DiscoverUsage` read from their default start at the genesis block
  (item 6), is built locally as upgrade 15 builds it rather than fetched.
- **FB3. The filter RPCs below the birthday** (item 10). **Decided as
  recommended (project owner, 2026-11-22).** Options: fetch on
  demand from the backend and answer as dcrwallet answers; or return
  dcrwallet's own missing-filter error (`NotExist`, "no cfilter saved for
  block <hash>"), a reply that differs from Go's. ADR-0014 never trades
  byte-compatible RPC results (§1, tier 1). Recommended: fetch on demand for
  `getcfilterv2` and `GetCFilters`, from peers in SPV mode and from dcrd's
  `getcfilterv2` in RPC mode, each checked as in FB2; with no backend
  connected, the missing-filter error. For `validatepredcp0005cf`, hash the
  filters fetched the same way, so that it answers whether the pre-DCP0005
  filters the network serves hash correctly; its reply is then Go's for the
  same filters. For the genesis block, which `getcfilterv2` can be asked
  for by hash and `validatepredcp0005cf` hashes first, the filter is built
  locally as upgrade 15 builds it, with nothing fetched, so those answers
  need no backend. Which clients call these three was not examined.
- **FB4. Where the rule lives.** **Decided as recommended (project owner,
  2026-11-22); for a block connected before its child is known under a
  pending time birthday it conflicts with decision item 1 as worded, the
  addendum's FB11, and while a birthday is pending its check has no
  birthday height to use, the addendum's FB12.** *(2026-11-22, later:
  both decided as recommended, project owner: the check widened under
  FB11 (i), and the height while a birthday is pending given by FB12 (a);
  rule 4 as it now stands is in the addendum "2026-11-22 (later)".)*
  Recommended: in the
  syncers, which do not
  pass a filter below the birthday to the store; the `purse-udb` port of
  `ExtendMainChain` accepts a block without a filter only below the birthday
  and refuses one above it, and its other operations stay as dcrwallet's
  (apart from the start of `MissingCFiltersHeight`'s walk, if FB9 is
  answered as recommended), so that ADR-0002's operation scripts, which
  always supply filters, compare as before.
- **FB5. `importcfiltersv2` and `repaircfilters` below the birthday** (item
  11). **Decided as recommended (project owner, 2026-11-22): the sentence
  is about what the wallet fetches and keeps of its own accord, and
  supplied filters are written.** First, whether the owner's sentence
  covers filters a user supplies
  as well as those the wallet fetches; decision item 1 settles only the
  latter. If it covers them, the write option below is out and the choice
  is between the other two; choosing the write option means the owner
  narrowing the decision to the filters the wallet obtains itself. Options:
  write the filters as dcrwallet does, so that the store then holds
  filters below the birthday the user supplied; accept the call and skip
  those heights; refuse it. Recommended: read the sentence as being about
  what the wallet fetches and keeps of its own accord, and write the
  supplied filters as dcrwallet does, the user having asked for them; let
  every reader serve a stored filter wherever one is held, falling back to
  FB2 and FB3 only where none is.
- **FB6. The logical export** (item 12). **Decided as recommended
  (project owner, 2026-11-22); not yet specified; for a birthday still
  pending, the addendum's FB12.** Recommended: a comparator
  projection that drops, on both sides, the `[cfilters]` records of heights
  below the `birth` record's height (a side filter's height read from its
  `sideheader`) and compares the segment holding the birthday in full mode;
  `havecfilters` is compared as it is if FB9 is answered as recommended
  (1 on both sides), and dropped from the comparison otherwise. Specified in
  ADR-0001 with the Rust producer of `[cfilters]`. And, because the Rust
  side would otherwise list one `cfmissing` line per height below the
  birthday, about a million on mainnet, a `[cfilters]` record stating that
  range once, which raises the section's version under ADR-0001 §10.
- **FB7. Wallets created through gRPC.** **Decided as recommended
  (project owner, 2026-11-22),** after being told that a wallet created
  through Decrediton may then see no saving until those quirks are dealt
  with, and that what Decrediton sends at creation was not examined.
  Under QK-0247 and QK-0554,
  `CreateWallet` gives genesis or `birth_height` as the birthday (item 2),
  so wallets created that way, Decrediton's among them, gain nothing from
  this decision unless the client sends a height with `set_birth_time`. The
  birthday reaches clients (`walletinfo`, gRPC `BirthBlock`,
  `methods.go:5531-5538`, `server.go:628-648`) and the restore contract.
  Recommended: leave QK-0247 and QK-0554 to their own dispositions under
  ADR-0012 Rule 1, and accept that such wallets keep every filter.
- **FB8. The birthday block under a time birthday** (item 3). **Decided
  as recommended (project owner, 2026-11-22); for a block connected
  before its child is known, the addendum's FB11.** The child
  that resolves the birthday is above it, and its filter, which dcrwallet's
  first rescan reads (item 4), is kept under decision item 1 with no late
  decision needed. The question is the birthday block's own filter, which
  is known to be the birthday block's only when the child arrives.
  Recommended: keep it, as decision item 1 says, by deciding one block late
  in the syncer, so that both the birthday block's filter and the child's
  are kept. Fetching it again when the child arrives would also keep it, at
  the cost of a second request for a filter the initial sync may already
  have had.
- **FB9. What `have_cfilters` means in dcr-purse.** **Decided as
  recommended (project owner, 2026-11-22): option (a); where the walk
  starts while a birthday is pending, the addendum's FB12.** In dcrwallet the
  row is
  1 when every main-chain block has its filter: the upgrade that introduces
  it sets 1 only when the tip is the genesis block
  (`wallet/udb/upgrades.go:877-885`), `ExtendMainChain` refuses to extend
  the chain unless the row is exactly 1 (`txmined.go:208-211`), and
  `MissingCFiltersHeight` walks from genesis to the first block without a
  filter (`txmined.go:405-421`), which the missing-filter pass of item 9
  runs when the row is absent or 0. A database created at the pin runs
  those upgrades with only the genesis block present and ends at 1. Under
  that meaning a store without the filters below the birthday holds 0.
  Options:
  - *(a)* Keep the row's layout and give it the meaning "no filter at or
    above the birthday height is missing". A store dcr-purse creates sets
    it to 1 and keeps it there, since it never imports a database
    (ADR-0013), writes the filter of every block at or above the birthday
    that `ExtendMainChain` connects, from height 1, and, for a wallet whose
    birthday is genesis, holds the genesis filter, which no connection
    writes (decision item 1). The `ExtendMainChain` gate passes and the
    missing-filter pass returns before walking (it calls
    `MissingCFiltersHeight` only when the row is absent or 0,
    `wallet.go:1297-1305`), both as at the pin, `InsertMissingCFilters` is
    never reached from the pass, and the logical export shows 1 on both
    sides, as Go's does. What remains is the udb-level
    `MissingCFiltersHeight`, which ADR-0002's operation scripts can call
    whatever the row holds. Ported unchanged, it walks from genesis and
    returns the lowest stored main-chain height without a filter: 0 for a
    wallet born after genesis, whose genesis filter is not kept, while the
    row reads 1. Started at the birthday height instead, it returns
    `NotExist` in that state, in agreement with the row. For a wallet whose
    birthday is genesis the two are the same, and so are they for any
    operation script, since scripts supply every filter (FB4).
  - *(b)* Keep dcrwallet's meaning and write 0 for a wallet born after
    genesis. Four changes to the ported code follow, not one:
    1. the `ExtendMainChain` gate (`txmined.go:208-211`) must accept 0;
    2. the walk must start at the birthday height instead of genesis, or
       the pass would fetch every filter below the birthday; and since the
       row then stays 0, the pass walks at every start and, in the normal
       state with nothing missing at or above the birthday, gets `NotExist`
       (`txmined.go:420`), which at the pin it returns as an error that
       aborts both syncers (item 9), so the pass must read `NotExist` as
       nothing missing;
    3. when the first block missing a filter is the birthday block, its
       parent is below the birthday and has no stored filter, so
       `InsertMissingCFilters` refuses the span (`txmined.go:461-469`):
       that check must accept a first block at the birthday height;
    4. `InsertMissingCFilters` writes 1 when a span ends at the tip
       (`txmined.go:479-487`), which (b) forbids, and the pass's loop ends
       only when it reads 1 (`wallet.go:1326-1328`, `:1359-1361`). Without
       that write, after the span that reaches the tip the loop finds the
       span's first filter stored, steps forward 2,000 heights at a time
       (`:1334-1338`) until `GetMainChainBlockHashForHeight` fails past the
       tip, and returns that error (`:1330-1333`), so the loop needs
       another way to stop, such as ending after the span that reaches the
       tip.

    The exported `havecfilters` then differs from Go's (0 against 1) for
    every wallet born after genesis, which FB6's projection must drop.

  Recommended: (a), with `MissingCFiltersHeight` walking from the birthday
  height: the row states something true of every dcr-purse store, the
  gate, the pass and `InsertMissingCFilters` are ported unchanged, and the
  one change, the walk's start, makes the operation agree with the row.
- **FB10. A store with no `birth` row** (item 8). **Decided as
  recommended (project owner, 2026-11-22).** dcrwallet's `BirthState`
  reads an absent row as no birthday, and no database at the pin lacks one
  (item 2). `purse-store` reads an absent row as none set
  (`purse_store::MetaKey::Birth`, `crates/purse-store/src/keys.rs`). With
  no birthday, the owner's sentence drops nothing on a plain reading.
  Options: treat an absent row as the genesis block and keep every filter;
  or refuse to connect blocks until a birthday is set. Recommended: the
  former. If dcr-purse's creation writes the row as dcrwallet's does (item
  2), the case arises only from a defect or a damaged store, and keeping
  every filter loses nothing there.

## Addendum, 2026-11-22 — FB1 to FB10 decided as recommended (project owner)

On 2026-11-22 the project owner, shown a table of FB1 to FB10 with the
recommendations above, answered "go with your suggestions for fb1-10".
That is the whole of the answer: each of the ten is decided as
recommended, FB9 as its option (a). The questions above keep their
options and reasoning as the record of what was chosen between. The text
above stays as written apart from a second bullet in the Status block,
the decision mark at the head of each question, and the dated pointers
added to it, some of which sit inside an existing parenthesis or
sentence. Before answering, the owner was told that FB7 means a wallet created
through Decrediton may see no saving until QK-0247 and QK-0554 are dealt
with, and that what Decrediton sends at creation was not examined. Both
still hold.

Decided, not implemented: the rules below are what the sync phases
(brief §6 Phases 5, 6 and 10) are to build, and nothing of them exists in
code yet.

### The rules

What dcr-purse is to do, each stated once.

1. **FB1 — The initial sync.** Neither syncer requests the committed
   filter of a block below the birthday height during the initial sync, in
   SPV or RPC mode, whatever set the birthday. While a birthday is still
   pending above the tip, the RPC syncer skips them too, which tells dcrd
   that the birthday lies above the tip. After a reorganisation has rebuilt
   side-chain nodes below the birthday without filters (item 7), the SPV
   second stage selects from the first node at or above the birthday that
   lacks a filter and requests none below it. The announcement paths after
   the initial sync are unchanged: they fetch, check and, in SPV mode, scan
   the filters of every block of the new best chain as dcrwallet does, and
   the store keeps those at or above the birthday only (item 7).
   *(2026-11-22, FB13 decided as option (a), project owner: in one case
   the announcement paths make one request more than this sentence says.
   When the first block a chain switch attaches resolves a pending time
   birthday at a fork point the store holds without its filter, they also
   request, check and write that fork point's filter; rule 13 of the
   addendum "2026-11-22 (FB13)".)* Under a
   pending time birthday, a block connected before its child is known
   (the last block of a batch, the initial-sync tip) has its filter
   requested only when the child arrives and shows it to be the birthday
   block, with rule 8 (FB11, "Requests"); for the birthday height while a
   birthday is pending, FB12. *(Both decided later the same day, FB11 (r2)
   and FB12 (a); addendum "2026-11-22 (later)".)*
2. **FB2 — Rescans and discoveries from below the birthday, SPV mode.** For
   each block of the range below the birthday whose filter the store does
   not hold (FB5), the rescan or discovery fetches the filter from peers,
   checks it against the stored header as the initial sync does where
   DCP0005's commitment applies (mainnet from height 431,488, testnet3
   from 323,328), accepts it below those heights as dcrwallet's sync
   accepts it, uses it, and stores none of them, so that decision item 1
   holds afterwards. The genesis filter is built locally, as upgrade 15
   builds it. A filter that cannot be fetched or fails its check fails the
   rescan or discovery; the block is never treated as one without a match
   (decision item 4). RPC mode reads no stored filter there and is
   unchanged.
3. **FB3 — The filter RPCs below the birthday.** `getcfilterv2` and gRPC
   `GetCFilters` serve a stored filter where one is held, and otherwise
   fetch it on demand, from peers in SPV mode and through dcrd's
   `getcfilterv2` in RPC mode, checked as in rule 2 and not stored. With no
   backend connected they return dcrwallet's missing-filter error (`NotExist`,
   "no cfilter saved for block <hash>"). `validatepredcp0005cf` hashes the
   filters obtained the same way, so its reply is Go's for the same
   filters; because it obtains them the same way, with no backend
   connected it meets the missing-filter error at the first filter it
   neither holds nor can build and returns it, as Go's returns any error
   of its range over the stored filters (`wallet/wallet.go:5252-5255`).
   What any of the
   three replies when a connected backend fails to serve a filter is not
   decided: it is the code's to settle and its PARITY.md row's to record
   (What the code owes). The genesis filter is built locally, so an answer
   about the genesis block needs no backend.
4. **FB4 — Where the rule lives.** In the syncers, which pass no filter
   below the birthday to the store. `purse-udb`'s port of `ExtendMainChain`
   accepts a block without a filter only below the birthday height and
   refuses one at or above it. Its other operations are dcrwallet's, apart
   from the start of `MissingCFiltersHeight`'s walk (rule 9), so ADR-0002's
   operation scripts, which always supply filters, compare as before. For
   a block connected before its child is known while a birthday set from
   time is pending, see FB11 below; for what "the birthday height" is
   while a birthday is pending, FB12. *(Both decided later the same day;
   the rule as widened by FB11 (i) and read under FB12 (a) is restated in
   the addendum "2026-11-22 (later)".)*
5. **FB5 — Filters a client supplies.** The owner's sentence of 2026-11-21
   is read as being about the filters the wallet fetches and keeps of its
   own accord. `importcfiltersv2` and `repaircfilters` write the filters
   they are given as dcrwallet writes them, below the birthday included,
   and every reader serves a stored filter wherever one is held, fetching
   (rules 2 and 3) only where none is.
6. **FB6 — The logical export.** A comparison with Go of a wallet born
   after genesis runs under a comparator projection that drops, on both
   sides, the `[cfilters]` records of heights below the `birth` record's
   height (a side filter's height read from its `sideheader`), and
   compares the 4096-height segment holding the birthday in full mode;
   `havecfilters` is compared as it is, 1 on both sides (rule 9).
   `[cfilters]` gains a record that states a range of main-chain heights
   without a filter once, so that a Rust export does not list one
   `cfmissing` line per height below the birthday, about a million on
   mainnet; that raises the section's version under ADR-0001 §10. The
   projection and the record are specified in ADR-0001 together with the
   Rust producer of `[cfilters]`, and are not specified yet (ADR-0001,
   pointer of 2026-11-22). Two constraints on that specification follow
   from the other answers: under rule 5 a store may hold supplied filters
   below the birthday, so the heights without a filter there need not be
   one range; and under rule 10 a store with no `birth` record counts as
   born at genesis, so the projection drops nothing for it. A third
   matter is not settled by the answer and is FB12: the `birth` record's
   height names the birthday only once the birthday is resolved
   (decision item 1). While a birthday set from time is pending the
   record's height is 0 (dcrwallet writes the `BirthdayState` it is given,
   `wallet/udb/txmined.go:341-365`, and `walletsetup.go:192-196` leaves the
   height unset), and while a birthday set from height H is pending it is
   H. In the first state, and for a pending height birthday of 0, the
   projection as worded drops nothing, yet the Rust store lacks filters
   Go's holds: every main-chain filter under a pending time birthday
   (decision item 1 and item 3; FB11 for the one block whose child is not
   yet known), and the genesis filter in both, since decision item 1 writes
   it only when such a birthday resolves at genesis. A comparison in
   either state would report that difference as a divergence.
   *(Decided later the same day, FB12 (a), project owner: a pending height
   birthday counts at its height from creation, a height-0 birthday's
   genesis filter is written at creation, and under a pending time
   birthday the projection drops every `[cfilters]` record on both sides;
   addendum "2026-11-22 (later)".)*
7. **FB7 — Wallets created through gRPC.** QK-0247 and QK-0554 are left to
   their own dispositions under ADR-0012 Rule 1. A wallet whose
   `CreateWallet` request gives it a genesis birthday keeps every filter,
   so a wallet created through Decrediton may see no saving until those
   quirks are dealt with. What Decrediton sends at creation was not
   examined.
8. **FB8 — The birthday block under a time birthday.** Its filter is kept,
   with the child's: the syncer decides one block late whether a block is
   the birthday block, from its child's header, instead of fetching the
   filter a second time when the child arrives. For a block whose child
   is not known when it is connected, its filter is first requested when
   the child arrives (rule 1); how the store records it is FB11 below.
   *(Decided later the same day, FB11 (i): connected without it, and
   written through `put_cfilter_at` in the transaction that connects the
   child; addendum "2026-11-22 (later)".)*
9. **FB9 — What `have_cfilters` means.** Option (a). The row keeps its
   layout and means "no filter at or above the birthday height is
   missing". Every store dcr-purse creates writes 1 and keeps it. The
   `ExtendMainChain` gate, the missing-filter pass and
   `InsertMissingCFilters` are ported unchanged, so the pass returns before
   walking and its started and finished notifications are sent as
   dcrwallet sends them. The udb-level `MissingCFiltersHeight` walks from
   the birthday height instead of genesis and returns `NotExist` when
   nothing at or above it is missing. While a birthday is pending, the
   `birth` row's height is not the birthday height (rule 6), and a walk
   from it would find the genesis filter missing and return 0 while the
   row reads 1, the disagreement option (a) chose the new start to remove;
   where the walk starts in that state is FB12. *(Decided later the same
   day, FB12 (a): from H under a pending height birthday H, and, under a
   pending time birthday, `NotExist`; addendum "2026-11-22 (later)".)*
   The doc comments of
   `purse-store` that give dcrwallet's meaning of the row (listed under
   Consequences) change with the code that implements this record.
10. **FB10 — A store with no `birth` row** is treated as born at genesis:
    it keeps every filter, `ExtendMainChain` accepts no block without a
    filter, and `MissingCFiltersHeight` walks from height 0. dcr-purse's
    creation writes the row as dcrwallet's does, so the case arises only
    from a defect or a damaged store.

### The answers checked against each other and against accepted text

Before they were recorded, the ten answers were read against each other,
against Decision items 1 to 5, against ADR-0014's S15 and its §1 as
accepted, and against ADR-0001 and ADR-0003 as accepted.

- FB1 and FB2 widen S15 (ADR-0014 Open point 5) from a typed birthday to
  every birthday, as this record already widened what the store keeps.
  The disclosure FB1 makes to peers, and to dcrd while a birthday is
  pending above the tip, is the kind ADR-0014 §1 tier 3 leaves to the
  owner, and the owner's answer accepts it. FB2's requests tell peers the
  range a rescan or discovery reads, as S15's on-demand rule already did
  for a typed birthday.
- FB3 keeps Go's reply wherever a backend serves the filter. The
  missing-filter error with no backend connected is a reply Go does not
  give, since Go reads the filter from its store. W4 and W5 make
  byte-compatible results part of the contract, and ADR-0014 §1 tier 1
  never trades them for speed. The difference follows from the owner's
  decision of 2026-11-21 itself: with the filter neither stored nor
  reachable, no answer can be Go's. It is a deliberate divergence to be
  recorded under ADR-0012 Rule 10 when the code is written (decision item 5). This is
  stated here as a consequence of the decision, not a conflict between
  answers.
- FB5 settles what decision item 1 left open; FB2 and FB3 serve stored
  filters first, so a supplied filter is used where it is held.
- FB9 (a), FB10 and FB6 agree: a store with no `birth` row is born at
  genesis, so the row's 1 is true of it in dcrwallet's own sense, and
  nothing is projected away.
- FB4 agrees with ADR-0003 as accepted: the chain tables allow a block
  without its filter (invariants 3 and 7), and schema version 1 stands.
- FB6 agrees with ADR-0001 §10, under which a new record type raises the
  section's version and amends that ADR when it is specified.
- FB1 and FB8 agree. For a block whose child is not known when it is
  connected under a pending time birthday, rule 8's late decision and
  rule 1 together request its filter once, when the child shows it to be
  the birthday block, and not at all when the child shows it below
  (FB11, "Requests").
- **FB4 and decision item 1 conflict for that same kind of block.** See
  FB11.
- **FB1, FB4, FB6 and FB9 leave a gap while a birthday is pending.** Each
  is keyed on the birthday height, which decision item 1 defines only
  once the birthday is resolved. Found in review of this addendum's
  first draft. See FB12.
- *(Later on 2026-11-22: FB11 and FB12 decided as recommended, project
  owner. The conflict and the gap are resolved by those answers; the check
  of the answers is in the addendum "2026-11-22 (later)", which found one
  more case, FB13.)*

### FB11. A block connected before its child is known, under a pending time birthday

**Decided as recommended (project owner, 2026-11-22, later): option (i)
with (r2).** Found while recording the answers, and Proposed until the
owner's "Accept both". The options and reasoning below stay as the record
of what was chosen between. For a block the store holds without its
filter that a reorganisation's first attached block shows to be the
birthday block, which this question does not cover, see FB13 in the
addendum "2026-11-22 (later)".

With a birthday set from time T, a block is known to be the birthday
block only when its child arrives (item 3), and FB8 keeps its filter by
deciding one block late. Both syncers run the initial sync batch by
batch. In SPV mode the second stage takes one batch of headers and
fetches the filters it lacks, and the third stage connects that batch
(`spv/sync.go:1713-1773`, `:1774-1811`). In RPC mode the syncer fetches a
filter for every header of a batch and connects the batch before it asks
for the next headers (`chain/sync.go:244-271`, `:325`). So in the initial
sync a block is connected before its child's header is known in two
places: the last block of a batch, and the tip at the end of the sync.
After the initial sync the same holds for every block announced while
such a birthday is pending above the tip. A typed date must be at least
a day old (`internal/prompt/prompt.go:401-404`) and a new seed's is a day
behind the clock (item 2), so a birthday pending above the tip needs a
tip more than a day old, such as that of a stalled testnet or simnet.

Not every such block is in doubt. Along the chain being connected from
the stored tip, the first block whose own timestamp is after T resolves
the birthday and lies above it, and so does every block after it,
whatever their own timestamps: `ChainSwitch` resolves the birthday once,
at that block, and clears the flag (`wallet/chainntfns.go:186-196`;
item 3). The syncer tells this from the headers, and FB12 (a) 2 states
it as a rule. This question is about the blocks before that one. Of
those, a block whose child is not known when it is connected, here *a
block in doubt*, may turn out to be the birthday block or to lie below
it, and the syncer cannot tell which.

- *Requests: no conflict.* Rule 8 decides from the child's header whether
  a block is the birthday block, and rule 1 requests no filter below the
  birthday. Together they request the filter of a block in doubt when its
  child arrives and shows it to be the birthday block, with the child's
  own, and not at all when the child shows it below. That request is the
  first and only one for that filter. It is not the second request rule 8
  was chosen instead of: FB8's other option fetched again a filter the
  initial sync already had, and here the initial sync never asked for it.
  At a batch boundary the request moves to the next batch and adds none:
  in RPC mode every header gets its own `getcfilterv2` call
  (`chain/sync.go:254-258`), and in SPV mode the pin sends one
  `getcfilterv2` message per filter (`p2p/peering.go:1787-1811`; ADR-0014's
  S13 sends `getcfsv2` ranges instead, which the filter can join). At the
  tip it is requested with the child's filter when the child is announced
  (`spv/sync.go:1497-1508`; `chain/sync.go:860`), and the child's
  connection waits for it. The cost is in the syncer: the stage that
  requests a batch's filters also requests the filter of the block below
  the batch's first block whenever that first block resolves the birthday
  at its parent. Nothing is held between batches, so a restart in between
  needs no path of its own. This was read in the source, not run. It is
  option (r2) below. Blocks announced after the initial sync are not
  affected on this side: rule 1 leaves the announcement paths fetching
  every filter.
- *The store: FB4 against decision item 1.* FB4 lets `ExtendMainChain`
  accept a block without a filter only below the birthday, and decision
  item 1, with FB8, keeps the birthday block's filter and none below it.
  As worded, a block in doubt can be connected neither without its filter
  (it may be the birthday block) nor with it (it may lie below), unless a
  write follows the connection. FB4's check itself needs a birthday height
  to compare with, which the store does not hold while a birthday set from
  time is pending (FB12). None of the operations FB4 keeps as dcrwallet's
  makes that write: `InsertMissingCFilters` refuses every call while
  `have_cfilters` is 1 (`wallet/udb/txmined.go:432-435`), which under FB9
  (a) is always; `ImportCFiltersV2` (`wallet/udb/cfilters.go:34-67`)
  writes a filter at a main-chain height unchecked, as a client's
  operation; and dcrwallet has no operation that removes a main-chain
  filter. Decision item 1's genesis filter, written when a birthday
  pending at creation resolves at genesis, needs the same kind of write,
  since the genesis block is never connected through `ExtendMainChain`.
- *Detaching.* `ChainSwitch` reads the stored filter of every main-chain
  block it detaches and fails if one is missing
  (`wallet/chainntfns.go:125-137`), before it reads the birthday (`:167`).
  Decision item 4 and item 7 relieve that only for a block below the
  birthday. Under option (i) below, a block in doubt stands in the store
  without its filter, and a reorganisation that detaches it while the
  birthday is pending would fail a chain switch dcrwallet makes. The
  exception has to cover it, and every other block connected without a
  filter in that state: while a birthday is pending, `ChainSwitch`
  rebuilds, with no filter, the side-chain node of every main-chain block
  the store holds without one, neither fetching nor failing. If the block
  returns to the best chain, the announcement path fetches a fresh filter
  for it (`spv/sync.go:1488-1519`; `chain/sync.go:860-869`). The detached
  block can still end up at or above the birthday. When the birthday
  resolves at a height at or below the detached block's, by that chain
  switch (a first block of the new branch with a timestamp after T
  resolves it at the fork point, below every detached block) or later,
  decision item 1 asks for the block's filter in `cfilters_detached`, and
  the store has none. The same holds for a block detached while the
  birthday is pending whose filter was not kept because its child showed
  it below the birthday block on its own branch. Fetching such a filter
  is unattractive: the block is off the best chain, the pin's own SPV
  code notes that no peer may then serve its filter
  (`spv/backend.go:175-180`), and a chain switch would wait on or fail
  over a filter that nothing in the wallet's own operation reads. Not
  fetching leaves `cfilters_detached` without filters decision item 1
  asks for, for blocks detached while a time birthday was pending:
  `getcfilterv2` asked for one by hash then falls under rule 3, and the
  export lacks the `sidecfilter` record Go's carries for it, which the
  projection of rule 6 must account for when it is specified. Each case
  needs a reorganisation while the birthday is pending, and so a chain
  more than a day old (above).
- *Readers before the child arrives.* While the birthday is pending above
  the tip, the SPV initial discovery and rescan start at block 1 (items 3
  and 4; `spv/sync.go:1850-1892`) and read every filter to the tip before
  the tip's child arrives, the tip's own filter included. The store holds
  none of them in that state, so they fetch them under FB2, as item 3
  says. If the child then shows the tip to be the birthday block, the
  tip's filter is requested a second time. A single request would need a
  hand-off: under (r2) the rescan's copy kept for the syncer, under (r1)
  the syncer's held copy given to the rescan and the discovery. Without
  one, both options request that filter twice. It is one filter in a
  rescan which in that state fetches every filter from block 1, and the
  recommendation below builds no hand-off. Any later reader that reaches
  a block in doubt, a rescan or a filter RPC, likewise finds no stored
  filter and fetches it under FB2 or FB3.

Options for the store:

- *(i)* `ExtendMainChain` also accepts a block without a filter while a
  birthday set from time T is pending, provided the block's own timestamp
  is not after T, so that it still refuses, without its filter, the block
  that resolves the birthday (that block lies above, FB12 (a) 2). The
  syncer writes the filter of a block in doubt in the transaction that
  connects a child resolving the birthday at that block, through
  `purse-store`'s `put_cfilter_at` (built: the store's counterpart of the
  writes in dcrwallet's `InsertMissingCFilters` and `ImportCFiltersV2`),
  and writes nothing when the child shows the block below the birthday.
  A block announced after the initial sync already has its filter
  fetched by the announcement path; the syncer keeps that filter until
  the child arrives rather than request it again, and only after a
  restart in between fetches it when the child arrives. Detaching is as
  in the bullet above.
- *(ii)* Such a block is connected with its filter, as dcrwallet connects
  it, and the filter is removed in the transaction that connects a child
  showing the block below the birthday: an operation neither dcrwallet nor
  `purse-store` has, used by the syncers only.
- *(iii)* The filter of such a block is kept whether or not it turns out to
  be below the birthday, which narrows decision item 1 by at most one
  filter per batch boundary, one at the initial-sync tip and one per block
  announced while the birthday is pending.

(ii) and (iii) need the filter when the block is connected, so in the
initial sync they need (r1).

Options for the requests, in the initial sync:

- *(r2)* The requests as rules 1 and 8 give them ("Requests" above): none
  for a block in doubt until its child shows it to be the birthday block.
  Rule 1 holds exactly. Only (i) works with it.
- *(r1)* The syncer also requests the filter of a block in doubt (the
  last block of a batch, and the tip) when it connects it. That narrows
  the decided rule 1 by at most one request per batch of headers and one
  at the tip, each a request dcrwallet makes too. Under (i) the syncer
  then holds the filter until the child arrives, and fetches it then
  after a restart in between. It saves no request against (r2), which also
  requests each such filter once, and it waits less at the tip.

Recommended: (i) with (r2). (r2) is what rules 1 and 8 as decided give,
so it changes no decided rule, and (i) is the only store option that
works with it. (i) adds no store operation, only a caller of a write the
store already has and a widening of FB4's check that still refuses,
without its filter, the block that resolves the birthday. It keeps
decision item 1 exact for main-chain filters, and the same write serves
decision item 1's genesis filter. Operation scripts supply every filter,
so the widened acceptance changes nothing they compare. What the owner
is asked to decide is that widening of FB4, and, for blocks detached
while a time birthday is pending, that their side-chain nodes are rebuilt
without a filter and that `cfilters_detached` goes without the filters
decision item 1 would ask for once the birthday resolves at or below
them, none being fetched. (r1) would narrow the decided rule 1 and save
no request, at the cost of a held filter and a separate path after a
restart. Not examined: how often a block in doubt turns out to be the
birthday block in practice. For a new seed, whose birthday is a day
behind the clock, the tip case needs a tip more than a day old; the
batch-boundary case can arise in any initial sync.

### FB12. The birthday height while a birthday is pending

**Decided as recommended (project owner, 2026-11-22, later): option
(a).** Found in review of this addendum's first draft, and Proposed until
the owner's "Accept both". The options and reasoning below stay as the
record of what was chosen between.

Decision item 1 defines the birthday height only once the birthday is
resolved. Rules 1, 4, 6 and 9 compare heights with it, and none of them
says what it is while the `birth` row carries its "set from height" or
"set from time" flag. In that state the row holds the height it was given
(`SetBirthState`, `wallet/udb/txmined.go:341-365`): H for a birthday set
from height H, 0 for a restore with no birthday typed
(`walletsetup.go:198-204`) and for gRPC `CreateWallet`'s default
(`internal/rpc/rpcserver/server.go:2738-2743`), and 0 for a birthday set
from time, whose height nothing sets before it resolves
(`walletsetup.go:192-196`, and `:180-185` for a typed date). The `birth`
record of the logical export carries the row's height and flags. Two
states follow:

- *Set from height H.* Block H+1 resolves the birthday at H
  (`wallet/chainntfns.go:185-188`), so the birthday height is H from
  creation, and the row already holds it. Rules 1, 4, 6 and 9 work with
  the row's height as it is, with one exception. For H = 0, decision item
  1 writes the genesis filter when the birthday resolves, not at creation.
  Until block 1 is connected, the Rust store lacks the genesis filter while
  Go's holds it, the projection of rule 6 drops nothing, and
  `MissingCFiltersHeight` walking from 0 returns 0 while the row reads 1.
  This is the state of every seed restored without a birthday and of every
  wallet created through gRPC with the default birthday.
- *Set from time T.* The birthday height is not known until the first
  block with a timestamp after T is connected. Every main-chain block
  connected before then lies at or below the birthday block (item 3).
  The store holds none of their filters (the possible birthday block's
  is FB11) and not the genesis filter. As worded, rule 6's projection
  drops nothing, rule 4's check has no height to compare with, and rule
  9's walk from the row's 0 returns 0 while the row reads 1.

Options:

- *(a)* Define the pending states and write a height-0 birthday's genesis
  filter at creation:
  1. A birthday set from height H counts as at height H from creation, for
     rules 1, 4, 6 and 9. For H = 0 the genesis filter is written at
     creation, as dcrwallet writes it, rather than when block 1 resolves
     the birthday. Block 1 always resolves such a birthday at genesis, so
     nothing is kept that decision item 1 drops; only the timing in
     decision item 1 ("when it resolves") changes, for that one case.
  2. While a birthday set from time T is pending, the syncer classes
     blocks from their headers, along the chain being connected from the
     stored tip, whatever the `birth` row says when a request is decided
     (in SPV mode the second stage decides a batch's requests before the
     third stage's `ChainSwitch` writes the row, `spv/sync.go:1736-1757`,
     `:1774-1811`). The first block whose own timestamp is after T
     resolves the birthday and lies above it, and so does every block
     after it, whatever their own timestamps (`wallet/chainntfns.go:186-196`
     resolves the birthday once); rule 1 requests their filters and the
     syncer passes them to `ExtendMainChain`, including when the resolving
     block is the last of a batch or the tip. Of the blocks before it, one
     whose child is that block is the birthday block, and rule 1 requests
     its filter (rule 8); one whose child is known and is not that block
     lies below the birthday block, and gets no request; one whose child is
     not yet known is FB11's block in doubt. Rule 4's check is FB11's;
     rule 6's projection drops, on both sides, every `[cfilters]` record,
     main-chain and side, the genesis block's included; and rule 9's
     `MissingCFiltersHeight` returns `NotExist`, in agreement with the
     row's 1, which speaks of the birthday once resolved.
- *(b)* Keep the rules keyed on the row's height as worded and decision
  item 1's timing, and compare with Go only wallets whose birthday has
  resolved. `MissingCFiltersHeight` then returns 0 in the pending states
  while the row reads 1, and the syncers still need (a) 2's reading of
  "below the birthday" for rule 1, which (b) leaves to the code.

Recommended: (a). It changes nothing the store keeps once the birthday has
resolved. It makes the Consequences fallback hold from creation: a seed
restored without a birthday, which dcrwallet creates with a pending
height-0 birthday, then holds every filter Go's holds. It also lets a
fixture whose chain never resolves the birthday be compared. Its one
change to accepted text is the timing in decision item 1 for a height-0
birthday, which is why it is the owner's. Under (b), comparisons of
restored wallets wait for block 1, and frozen fixtures must contain it.

### What the code owes

With the code that implements this record, besides the tests listed under
Consequences *(later on 2026-11-22 FB11 and FB12 were answered as
recommended, so the items below that wait on them apply as their
recommendations state; the addendum "2026-11-22 (later)" adds items)*:

- a test for each syncer that no filter below the birthday is requested in
  the initial sync, for a height birthday, a time birthday and a birthday
  pending above the tip (rule 1), and that the SPV second stage requests
  none below the birthday after a reorganisation has rebuilt nodes there;
- tests that a rescan and each discovery variant from below the birthday
  in SPV mode fetch, check and use the filters and store none, fail when a
  filter cannot be fetched or fails its check, build the genesis filter
  locally, and read a supplied filter from the store (rules 2 and 5);
- tests that `getcfilterv2`, `GetCFilters` and `validatepredcp0005cf`
  answer as Go does through a backend, and that with none `getcfilterv2`
  and `GetCFilters` return the missing-filter error and
  `validatepredcp0005cf` returns the same error, which follows from its
  obtaining the filters the same way (rule 3);
- tests that `ExtendMainChain` accepts a block without a filter below the
  birthday and refuses one at or above it (rule 4), and that the birthday
  block's filter is kept under a time birthday, both inside a batch and
  across a batch boundary and at the initial-sync tip (rule 8; across a
  boundary and at the tip once FB11 is answered, with a test of the
  requests the answer allows, under its recommendation that a block in
  doubt gets no request until its child shows it to be the birthday
  block, and none at all when the child shows it below);
- a test, for each syncer, that under a pending time birthday the block
  whose own timestamp is the first after the birthday's time, and every
  block after it, are connected with their filters, the resolving block
  being the last of a batch, the initial-sync tip and a block inside a
  batch in turn (FB12 (a) 2 once answered), and, under FB11 (i), that
  `ExtendMainChain` refuses the resolving block without its filter;
- once FB11 is answered, a test that a reorganisation detaching a block
  the store holds without a filter while a birthday is pending succeeds,
  rebuilding the block's side-chain node without a filter and fetching
  nothing, that the announcement path fetches a fresh filter if the block
  returns to the best chain, and that `cfilters_detached` then holds what
  the answer says when the birthday resolves at or below the block's
  height;
- a test that, while a birthday is pending above the tip, the SPV initial
  discovery and rescan fetch the tip's filter under rule 2 rather than
  fail;
- tests that a new store holds 1 in `have_cfilters`, that the
  missing-filter pass returns before walking and sends both
  notifications, and that `MissingCFiltersHeight` walks from the birthday
  height (rule 9);
- a test that a store with no `birth` row keeps every filter (rule 10);
- once FB12 is answered, tests of rules 1, 4, 6 and 9 with a birthday set
  from height 0, from a height above 0 and from time, each still pending,
  among them a comparison with Go before block 1 for the first;
- the projection and the range record specified in ADR-0001, with
  `purse-export-check`'s registry and the Go exporter moved to the new
  section version (rule 6), the pending states of FB12 included, and the
  `sidecfilter` records of blocks detached while a time birthday was
  pending (FB11, "Detaching").

The rows of `docs/PARITY.md` decision item 5 asks for, written with that
code: the filter RPCs' missing-filter error with no backend connected,
where Go returns its stored filter, and their failure when a connected
backend cannot serve the filter, whose reply is the code's to settle and
that row's to record (rule 3); and the SPV rescans and discoveries from
below the birthday, which
need peers and fail when none serves a filter, where Go reads its store
(rule 2). FB5 and FB9 (a) leave no RPC-visible difference: supplied
filters are written as dcrwallet writes them, and the row reads 1 as
Go's. The udb-level `MissingCFiltersHeight` reaches no RPC; its changed
start is recorded with ADR-0002's operation scripts when it is ported.

### Not measured, not examined, not done

- Nothing is implemented. No code, schema, export format or test changed
  with this addendum, and nothing was run for it: no oracle run, no
  benchmark, no timed measurement, no sync.
- The cost of a rescan or a discovery from below the birthday in SPV mode.
  With dcrwallet's defaults (`rescanwallet` from 0, the importing methods
  that rescan from 0, `discoverusage` from the genesis block) each one
  fetches about a million filters from peers on mainnet, every time it
  runs; neither its time nor its traffic was measured. A default gRPC
  `GetCFilters` call (the whole main chain) and `validatepredcp0005cf`
  (every filter below 431,488 on mainnet) fetch the same way, also not
  measured.
- Below the DCP0005 heights a fetched filter cannot be checked against its
  header. Each rescan or discovery reaching there takes those filters
  afresh from whichever peer serves them, where dcrwallet takes them once,
  in its initial sync, and can check them later with
  `validatepredcp0005cf`. What that changes in the exposure to a peer
  serving false filters was not examined.
- What Decrediton sends at creation, and so whether wallets it creates
  gain anything (rule 7), was not examined.
- Which clients call the three filter RPCs (rule 3) was not examined.
- The items of "What is not measured" above stand.

### Places amended

This record: a second bullet in the Status block, the dated pointers
above, the mark on each question, and this addendum. ADR-0001 (a pointer
under its Status line, open item 2, and item 5 of the owner's decision);
ADR-0003 (the `meta` row of Decision item 2 and the addendum of
2026-11-21 "Committed filters below the birthday"); ADR-0014 (its Status
line, the S15 row and Open point 5); `docs/PARITY.md` (the deliberate
divergence, revised in place as that file's rows are); the brief (a
pointer of 2026-11-22 after each pointer of 2026-11-21 that named FB6 as
open: the Phase 4, 5, 8 and 1.0 exits and §7 Layer 7); the README; the
ADR index (the rows for ADR-0001, ADR-0014 and this record).

## Addendum, 2026-11-22 (later) — FB11 and FB12 decided as recommended (project owner)

Later on 2026-11-22 the project owner was shown FB11 and FB12 in a table
with their recommendations, FB11's option (i) with (r2) and FB12's option
(a), and was told that accepting them would let Phase 0 close. The owner
answered "Accept both". That is the whole of the answer: each of the two
is decided as recommended. The two questions keep their options and
reasoning as the record of what was chosen between. The text above stays
as written apart from a third bullet in the Status block, the marks on
FB11 and FB12, and the dated pointers added to it.

What FB11's recommendation names as the owner's to decide, and so what
the answer decides, is the widening of FB4 (rule 4 below) and, for blocks
detached while a time birthday is pending, that their side-chain nodes
are rebuilt without a filter and that `cfilters_detached` goes without
the filters decision item 1 would ask for once the birthday resolves at
or below them, none being fetched. FB12's answer decides the reading of
the pending states below, and the one change to accepted text it names:
the timing, in decision item 1, of a height-0 birthday's genesis filter.

Decided, not implemented: nothing of it exists in code yet.

### The rules where FB11 and FB12 change them

Each stated once, as what dcr-purse is to do. The rules of the addendum
above that are not named here stand as written.

1. **The birthday height (decision item 1, with FB12 (a)).** Once the
   birthday is resolved, it is the height in the `birth` row. While a
   birthday set from height H is pending, it is H, from creation. While a
   birthday set from time T is pending there is none, and the syncer
   classes blocks from their headers along the chain being connected,
   whatever the `birth` row says when a request is decided: the first
   block whose own timestamp is after T resolves the birthday and lies
   above it, and so does every block after it, whatever their own
   timestamps; the block whose child is that block is the birthday block;
   a block whose child is known and is not that block lies below the
   birthday block; a block whose child is not yet known is a block in
   doubt (FB11).
2. **The genesis filter (decision item 1, its timing changed by FB12 (a)
   1).** Written at creation, as dcrwallet writes it, when the birthday is
   the genesis block or is set from height 0: block 1 always resolves such
   a birthday at genesis, so nothing is kept that decision item 1 drops.
   For a birthday set from time that resolves at genesis, written when it
   resolves, built locally, through `put_cfilter_at` in the transaction
   that connects block 1 (FB11 (i)). Otherwise it is not kept.
3. **Rule 1 (FB1) under a pending time birthday (FB12 (a) 2, FB11
   (r2)).** The filters of the block that resolves the birthday and of
   every block after it are requested and passed to `ExtendMainChain`,
   whether the resolving block lies inside a batch, is the last of a batch
   or is the tip. The birthday block's filter is requested with them: with
   its batch when its child is in the same batch; with the next batch, as
   the filter of the block below that batch's first block, when the
   birthday block was the last of a batch; and when the child is
   announced, the child's connection waiting for it, when the birthday
   block was the initial-sync tip. A block shown below the birthday block
   gets no request, and a block in doubt gets none until its child shows
   it to be the birthday block. The announcement paths after the initial
   sync stay as rule 1 states them. *(2026-11-22, FB13 decided as option
   (a), project owner: except that, when the first block a chain switch
   attaches resolves a pending time birthday at a fork point the store
   holds without its filter, they also request, check and write that fork
   point's filter. In the initial sync of either mode, when the first
   block a batch attaches resolves the birthday at a fork point the store
   holds without its filter, the request this rule already makes for the
   birthday block (with the batch, whether the fork point's header is in
   the batch or the fork point is the block below the batch's first
   block) is that request, made once, and the filter is written as rule
   13 of the addendum "2026-11-22 (FB13)" says.)*
4. **Rule 4 (FB4), as widened by FB11 (i).** The syncers pass no filter
   below the birthday to the store. `purse-udb`'s port of `ExtendMainChain`
   accepts a block without a filter when, and only when, the block lies
   below the birthday height (the resolved birthday's, or a pending height
   birthday's H), or a birthday set from time T is pending and the block's
   own timestamp is not after T. It refuses every other block without a
   filter, the block that resolves a pending time birthday among them, and
   in a store with no `birth` row (rule 10) it accepts none. Its other
   operations stay as rule 4 says. Operation scripts supply every filter,
   so the widening changes nothing they compare.
5. **Rule 8 (FB8), with FB11 (i).** The birthday block's filter is kept.
   When the block's child is known as the block is connected, the filter is
   passed with the block. When the block is in doubt, it is connected
   without its filter, and the syncer writes the filter through
   `put_cfilter_at` in the transaction that connects the child resolving
   the birthday at it: the filter requested as item 3 says or, for a block
   announced after the initial sync, the filter the announcement path
   fetched, which the syncer keeps until the child arrives and fetches
   again when the child arrives only after a restart in between. Nothing
   is written when the child shows the block below the birthday block.
   Nothing is held between batches of the initial sync.
6. **Detaching while a birthday is pending (decision item 4 and item 7 of
   "What reads stored filters", widened by FB11).** While a birthday is
   pending, `ChainSwitch` rebuilds without a filter the side-chain node of
   every main-chain block the store holds without one, below the birthday
   or not, neither fetching nor failing. If such a block returns to the
   best chain, the announcement path fetches a fresh filter for it. Of the
   blocks detached while a birthday set from time is pending, those held
   without a filter get none in `cfilters_detached` when the birthday
   later resolves at or below their height; none is fetched, and
   `getcfilterv2` asked for one by hash falls under rule 3.
7. **Rule 6 (FB6) in the pending states (FB12 (a)).** Under a pending
   height birthday H, the projection drops the `[cfilters]` records of
   heights below H: none for H = 0, whose genesis filter both sides now
   hold. Under a pending time birthday it drops, on both sides, every
   `[cfilters]` record, main-chain and side, the genesis block's included.
   The `sidecfilter` records Go's export carries for blocks detached while
   a time birthday was pending, which the Rust store lacks under item 6,
   are for the projection to account for when it is specified. All of it
   is specified in ADR-0001 with the rest of the projection, and is not
   specified yet.
8. **Rule 9 (FB9) in the pending states (FB12 (a)).** Under a pending
   height birthday H, `MissingCFiltersHeight` walks from H. Under a pending
   time birthday it returns `NotExist`, in agreement with the row's 1,
   which speaks of the birthday once resolved.

### The two answers checked

Before they were recorded, the two answers were read against each other,
against FB1 to FB10 as decided and against decision items 1 to 5, case by
case under a pending birthday. The cases were read in the source at the
pin; nothing was run.

- *A block inside a batch.* Its child's header is in the batch, so the
  syncer knows whether it is the birthday block, lies below it or lies
  above; rules 1 and 8 request accordingly, and `ExtendMainChain`
  receives the birthday block's filter with the block. This agrees with
  FB8 and with FB4 as widened.
- *The last block of a batch.* Connected without its filter, which item 4
  accepts, its timestamp not being after T (a later one would have
  resolved the birthday). The next batch's first block decides: if its
  timestamp is after T, the last block is the birthday block, whose filter
  is requested with that batch and written in the child's transaction;
  otherwise nothing is requested or written. No state crosses the batch
  boundary, so a restart between batches needs no path of its own, and
  rule 1 holds exactly.
- *The initial-sync tip.* As the last block of a batch, with the request
  made when the child is announced. While the birthday is pending above
  the tip, the SPV initial rescan and discovery fetch the tip's filter
  under rule 2 and keep none, so the tip's filter can be requested twice;
  FB11 accepted that, building no hand-off.
- *Blocks announced after the initial sync.* The announcement path
  fetches, checks and, with the rescan point nil, scans the filter of
  every attached block (rule 1). The store keeps the filters of the block
  that resolves the birthday and of those after it, and that of a block in
  doubt only once its child shows it to be the birthday block.
- *A detach while pending.* Item 6. While a time birthday is pending the
  store holds no filter for any main-chain block apart from supplied ones
  (FB5), since only the birthday block's is ever written, in the
  transaction that resolves the birthday; without item 6 every
  reorganisation in that state would fail a chain switch dcrwallet makes.
  With it, none fails. The fork point of such a reorganisation is FB13
  below.
- *A height birthday above 0.* Known from creation (item 1), so no block
  is in doubt and FB11's widening does not apply: block H's filter is
  requested and passed when the block is connected, and block H+1
  resolves the birthday. Rules 1, 4, 6 and 9 key on H. A reorganisation
  that detaches block H after the birthday resolves leaves the `birth`
  row naming the old block, as item 7 of "What reads stored filters" says.
- *A height-0 birthday.* The genesis filter is written at creation, so the
  Rust store holds every filter Go's holds from creation: the projection
  drops nothing, `ExtendMainChain` accepts no block without a filter, and
  `MissingCFiltersHeight` from 0 returns `NotExist` when nothing is
  missing. This agrees with FB10, under which a store with no `birth` row
  keeps every filter, and makes the Consequences fallback hold from
  creation.
- *A time birthday that resolves at genesis.* Block 1's timestamp is after
  T: `ExtendMainChain` refuses it without its filter, the syncer passes it
  with its filter, and the genesis filter, built locally, is written in
  the same transaction (item 2).
- *Supplied filters (FB5) while pending.* Written as dcrwallet writes
  them and served by every reader (rules 2 and 3); under a pending time
  birthday the projection drops them on both sides.

The two answers agree with each other: FB12 (a) 2's classing gives FB11
its block in doubt and the resolving block that FB11 (i)'s widening still
refuses without its filter, and FB12 (a) 1's write at creation leaves
FB11 (i)'s write of the genesis filter only the time birthday that
resolves at genesis. No PARITY.md row follows from them: neither changes
an RPC reply, and the one case they add for a filter RPC, a block detached
while a time birthday was pending and asked for by `getcfilterv2`, falls
under the rows rule 3 already names.

- **One case neither answer covers.** A reorganisation while a time
  birthday is pending can resolve the birthday at a block the store
  already holds without its filter. See FB13.

### FB13. A stored block that a reorganisation shows to be the birthday block

**Decided as recommended (project owner, 2026-11-22, after FB11 and
FB12): option (a).** Found in checking the answers to FB11 and FB12, and
Proposed until the owner's "Accept all three". The options and reasoning
below stay as the record of what was chosen between; the rule is stated
once in the addendum "2026-11-22 (FB13)".

While a birthday set from time T is pending, every block on the main
chain has a timestamp not after T, since the first block with a later one
resolves the birthday. A reorganisation in that state detaches the blocks
above a fork point F and attaches a new branch. `ChainSwitch` reads the
birthday state once, before it connects the attached blocks
(`wallet/chainntfns.go:169`), and the first attached block whose
timestamp is after T resolves the birthday at its parent (`:186-196`).
When that is the first attached block, the birthday block is F. If F is
the stored tip, the reorganisation only extends the chain, F is a block in
doubt, and FB11 covers it. If F is the genesis block, decision item 1
covers it: the filter is built locally and written when the birthday
resolves (item 2 above). Otherwise F lies below the stored tip, and its
child on the detached branch, known before the reorganisation, showed it
below the birthday block, so the store holds F without its filter. No
decided rule obtains or writes that filter:

- FB11 (i)'s write is of a block in doubt, one whose child was not known
  when it was connected; F's child was known.
- The announcement paths fetch the filters of the attached blocks only: in
  SPV mode those of the nodes `EvaluateBestChain` returns, which start at
  the root of the side chain, above the fork point
  (`wallet/sidechains.go:417-462`; `spv/sync.go:1488-1519`), and in RPC
  mode that of each block dcrd announces (`chain/sync.go:860-869`). Rule
  1 leaves both as they are, and FB11's request of a block in doubt's
  filter when its child is announced covers the initial-sync tip only.
- In the SPV initial sync, after a reorganisation of the stored chain
  (item 7 of "What reads stored filters"), FB12 (a) 2's classing names F
  as the birthday block, so rule 1 requests its filter, but no rule names
  the write. *(The RPC initial sync, `getHeaders`, carries the same case:
  it fetches headers from the wallet's block locators and makes a chain
  switch per batch, which can detach stored blocks
  (`chain/sync.go:219-340`, `ChainSwitch` at `:325`, the reorganisation
  logged at `:329-331`), and it runs again after the initial sync when
  `getMissingHeaders` falls back to it (`:419-424`, reached from
  `handleBlockConnected`, `:857`). Not named in this list when FB13 was
  put; option (a)'s "whenever" covers it, and rule 13 of the addendum
  "2026-11-22 (FB13)" names it.)*

Left so, the store lacks a main-chain filter at the birthday height.
Decision item 1 is not met for it. Under FB9 (a) the `have_cfilters` row
reads 1, meaning that no filter at or above the birthday height is
missing, while one is; `MissingCFiltersHeight`, walking from the birthday
height, returns F's height while the row reads 1, the disagreement FB9
(a) and FB12 (a) were chosen to remove. A later reorganisation that
detaches F fails, since the exception of decision item 4 covers blocks
below the birthday and, while a birthday is pending, blocks held without
a filter, and F is neither once the birthday has resolved. Rules 2 and 3
fetch only the filters of blocks below the birthday, and F is the
birthday block, so the readers of F's filter keep dcrwallet's behaviour,
which fails on a missing filter (decision item 4): a rescan or discovery
in SPV mode that reads it fails with dcrwallet's missing-filter error
(`NotExist`, `wallet/udb/txdb.go:1992-1999`; `spv/backend.go:528-532`),
and so do `getcfilterv2` asked for F, gRPC `GetCFilters` over a range
that holds it (`wallet/wallet.go:833-878`) and `validatepredcp0005cf`
when F lies below the DCP0005 height, a backend connected or not, where
dcrwallet, holding F's filter, serves it. A comparison with Go after the
birthday has resolved reports F's filter missing, which the projection of
rule 6 does not drop.

Options:

- *(a)* Whenever the first attached block resolves a pending time
  birthday at a fork point the store holds without its filter, the
  syncer requests that block's filter with the attached blocks' filters,
  from peers in SPV mode and through dcrd's `getcfilterv2` in RPC mode,
  checks it against the stored header as rule 2 checks a filter, and
  writes it through `put_cfilter_at` in the transaction that connects the
  resolving block, as FB11 (i) writes a block in doubt's. F is on the new
  best chain, so its filter is served. The chain switch waits for it, as
  the child's connection waits at the initial-sync tip, and a filter that
  cannot be fetched or fails its check fails the chain switch, as an
  attached block's does (`spv/sync.go:1502-1508`). It is one request
  dcrwallet does not make at that point, since dcrwallet holds F's filter
  from when it connected F; it tells the peer which block is the birthday
  block, a disclosure of the kind FB1 accepted, the initial sync having
  already shown peers roughly where the birthday lies. It adds no store
  operation, keeps decision item 1, FB9 (a) and rule 9 exact, and leaves
  the detach exception as it is.
- *(b)* Leave F without its filter. That narrows decision item 1 by one
  main-chain filter at the birthday height, and the narrowing does not
  stay local: FB9 (a)'s meaning of the row needs an exception for it,
  `MissingCFiltersHeight` must step over it, the detach exception must
  cover it, rules 2 and 3 must be extended to it (or its readers fail
  where dcrwallet's serve its filter, a divergence of its own), and the
  projection of rule 6 must account for it.

Recommended: (a). It is the only option that keeps the rules already
decided as they stand; (b) would reopen FB9. Not examined: how often such
a reorganisation happens. It needs a reorganisation while a time birthday
is pending, so a tip more than a day old (FB11), and a first attached
block whose timestamp is after T where the detached child's was not.

### What the code owes, added

- tests that `ExtendMainChain`, while a birthday set from time T is
  pending, accepts a block without a filter only when its timestamp is not
  after T, and refuses the resolving block without its filter (rule 4 as
  widened);
- a test, for each syncer, that a block in doubt at a batch boundary and
  at the initial-sync tip gets no request until its child arrives, gets
  one with the child's when the child shows it to be the birthday block
  and none when the child shows it below, and that its filter is written
  in the child's transaction (items 3 and 5);
- a test that a block announced after the initial sync while a time
  birthday is pending has the filter the announcement path fetched written
  when its child shows it to be the birthday block, fetched again only
  after a restart in between;
- a test that a store created with a height-0 birthday holds the genesis
  filter before block 1 is connected, and, once the Rust producer of
  `[cfilters]` exists, a comparison with Go in that state (item 2);
- the detach tests "What the code owes" above lists for FB11, with
  `cfilters_detached` holding no filter for a block detached while held
  without one (item 6);
- tests of the projection in the pending states of item 7, with the rest
  of the projection;
- once FB13 is answered, a test, in each sync mode, of a reorganisation
  whose first attached block resolves a pending time birthday at a fork
  point below the stored tip, showing what the answer gives the store for
  that block. *(Answered later the same day, option (a); the tests are
  listed in the addendum "2026-11-22 (FB13)".)*

### Not measured, not examined, not done

- Nothing is implemented. No code, schema, export format or test changed
  with this addendum, and nothing was run for it: no oracle run, no
  benchmark, no timed measurement, no sync.
- How often a block in doubt turns out to be the birthday block, and how
  often a reorganisation happens while a time birthday is pending, were
  not examined.
- FB13 is open. *(Decided later the same day, option (a), project owner;
  addendum "2026-11-22 (FB13)".)*
- The items listed under "Not measured, not examined, not done" in the
  addendum above stand.

### Places amended

This record: a pointer at the end of the Status block's second bullet and
a third bullet; dated pointers in decision items 1 and 4, items 3 and 7 of
"What reads stored filters", the Consequences, the preamble of the open
questions, FB4's mark, rules 1, 4, 6, 8 and 9 and the checks of the
addendum above, and the head of its "What the code owes"; the marks on
FB11 and FB12; this addendum. ADR-0003 (the pointer of 2026-11-22 in its
addendum of 2026-11-21); ADR-0001 (the pointer under its Status line);
`docs/PARITY.md` (the deliberate divergence, revised in place); the
README; the ADR index (the rows for ADR-0001, ADR-0003 and this record);
the brief (the Phase 0 status, §9 D2 and the M0 row);
`docs/PHASE0-EXIT.md`.

## Addendum, 2026-11-22 (FB13) — FB13 decided as recommended, and the cut-off rule (project owner)

Later on 2026-11-22 the project owner was shown three things, each with a
recommendation: FB13 above, recommended option (a), that the syncer
fetches the fork-point block's filter, checks it and writes it as part of
the same chain switch; the proposals ADR-0013 still marked Proposed (its
Open point 6's bullets other than Phase 8's, and its Open point 8),
recommended: accept as proposed; and a cut-off rule, proposed to the owner
in these words: "Questions that arise in working out ADR-0019 are settled
in the phase that implements it and do not count against Phase 0's exit."
The owner answered "Accept all three". That is the whole of the answer.
For this record it decides FB13 as option (a) and adopts the cut-off rule;
ADR-0013 records its own part. FB13 keeps its options and reasoning as the
record of what was chosen between. The text above stays as written apart
from a fourth bullet in the Status block, the mark on FB13, and the dated
pointers added to it.

Decided, not implemented: nothing of it exists in code yet.

### The rule FB13 adds

13. **FB13 — A stored block that a chain switch shows to be the birthday
    block.** While a birthday set from time T is pending, when the first
    block a chain switch attaches has a timestamp after T, so that it
    resolves the birthday at the fork point F, and the store holds F
    without its filter (F below the stored tip and not the genesis block,
    whose filter rule 2 of the addendum "2026-11-22 (later)" builds
    locally), the syncer requests F's filter with the attached blocks'
    filters, from peers in SPV mode and through dcrd's `getcfilterv2` in
    RPC mode; checks it against F's stored header as FB2's rule 2 checks
    a filter; and writes it through `put_cfilter_at` in the transaction
    that connects the resolving block, as rule 5 of that addendum writes
    the filter of a block in doubt. The chain switch waits for the
    filter, and one that cannot be fetched or fails its check fails the
    chain switch, as an attached block's filter does
    (`spv/sync.go:1502-1508`). This holds for every `ChainSwitch` call
    of both syncers, which at the pin are four:
    - the SPV announcement path, `handleBlockAnnouncements`
      (`spv/sync.go:1542`), and the RPC announcement path,
      `handleBlockConnected` (`chain/sync.go:891`), after the initial
      sync;
    - the SPV initial sync after a reorganisation of the stored chain,
      `initialSyncHeaders` (`spv/sync.go:1811`);
    - the RPC initial sync after a reorganisation of the stored chain,
      `getHeaders` (`chain/sync.go:325`), which also runs after the
      initial sync when `getMissingHeaders` falls back to it on a full
      batch of missing headers (`chain/sync.go:419-424`, reached from
      `handleBlockConnected` at `:857`).

    In both initial syncs (`getHeaders` included when the fallback runs
    it), rule 3 of that addendum, by FB12 (a) 2's classing, already
    requests F's filter with the batch, whether F's header is in the
    batch or F is the block below the batch's first block; that request
    is the one this rule names, and no second one is made. On the
    announcement paths the request is one more than rule 1 and rule 3 of
    that addendum say those paths make, and each carries a dated pointer
    here. It is one request dcrwallet does not make at that point, since
    dcrwallet holds F's
    filter from when it connected F, and it tells the peer, or dcrd, which
    block is the birthday block: a disclosure of the kind FB1 accepted,
    which the owner's answer accepts. Decision item 1, FB9 (a), rule 9
    with rule 8 of that addendum, and the detach exception of rule 6 of
    that addendum stand as they are: once the birthday has resolved, the
    store holds F's filter, `have_cfilters` reads 1 truly under FB9 (a),
    `MissingCFiltersHeight` walking from the birthday height does not
    stop at F, and a later chain switch that detaches F reads F's stored
    filter as dcrwallet does.

### The cut-off rule

In the words the owner accepted: "Questions that arise in working out
ADR-0019 are settled in the phase that implements it and do not count
against Phase 0's exit."

- **Scope.** It covers questions that arise in working out this record,
  and nothing else. It defers nothing in D1 to D10 that is not such a
  question, and it changes nothing already decided here: Decision items
  1 to 5 and the rules of FB1 to FB13 stand, and are what the
  implementing phases build.
- **What it does.** A question found from now on in working out this
  record (in specifying the projection of rule 6 in ADR-0001, in writing
  the syncers, the rescans, the filter RPCs or the Phase 3 port, or in
  reviewing any of them) is put to the owner, with a recommendation, in
  the phase that implements this record, and is settled there. While it
  waits, it is marked Proposed as FB1 to FB13 were, and it neither counts
  against Phase 0's exit nor reopens Phase 0. In D2's record (ADR-0003,
  addendum of 2026-11-21) such a question is deferred by this rule.
- **Which phase.** This record is implemented in more than one phase: its
  store rules in the Phase 3 port of `purse-udb`'s transaction store
  (`ExtendMainChain`'s check, FB4 and FB11; `MissingCFiltersHeight`'s
  start, FB9 and FB12), its fetching rules in the sync phases (brief
  §6 Phases 5, 6 and 10), the replies of the filter RPCs under rules 3
  and 5 in the RPC servers (brief §6 Phase 7, JSON-RPC, for
  `getcfilterv2`, `validatepredcp0005cf` and `importcfiltersv2`, specs 16
  to 18; Phase 12, gRPC, for `GetCFilters`, spec 19), and FB5's
  `cmd/repaircfilters` in `purse-tools`' port of it (brief §5, spec 22),
  whose phase the brief does not fix. The on-demand fetch behind the
  filter RPCs may sit in wallet code an earlier phase writes; the replies
  rule 3 leaves to the code are built in Phases 7 and 12. The rule's
  words name "the phase that implements it"; this record reads that as
  the phase that implements the part of the record the question bears
  on. The owner has not ruled on that reading.
- **Already left to the code.** What `getcfilterv2`, `GetCFilters` and
  `validatepredcp0005cf` reply when a connected backend fails to serve a
  filter is, under rule 3, the code's to settle and its PARITY.md row's
  to record; the projection and the range record of rule 6 are decided
  and are specified with the Rust producer of `[cfilters]`. Both were
  already left to the implementing phases by this record; the cut-off
  rule adds that whatever question their working out raises is settled
  there too.

### The answer checked

Before it was recorded, the answer was read against FB1 to FB12 as
decided and against decision items 1 to 5, in the source at the pin;
nothing was run. Rule 13 adds a request and a write only in the case FB13
names. In the initial sync of either mode it adds no request, since rule
3 of the addendum "2026-11-22 (later)" already requests F's filter with
the batch, so the initial-sync requests of rule 1 and rule 3 are
unchanged and FB11 (r2) still holds exactly. On the announcement paths
it adds one request, so the sentences of rule 1 and rule 3 that say those
paths are unchanged no longer hold in that case; each carries a dated
pointer to rule 13. It adds no store operation: `put_cfilter_at` is the write FB11 (i) already
uses. It keeps decision item 1 exact at the birthday height, FB9 (a)'s
meaning of `have_cfilters` true, and rule 8 of that addendum (`NotExist`
while a time birthday is pending) unchanged, since F's filter is written
in the transaction that resolves the birthday. The detach exception is
not widened. The projection of rule 6 needs nothing for F: both stores
hold its filter once the birthday has resolved. No PARITY.md row follows:
`getcfilterv2` asked for F, `GetCFilters` over a range holding it and
`validatepredcp0005cf` read F's stored filter, as dcrwallet's do. The
paths that carry such a chain switch were read as the `ChainSwitch` calls
of both syncers at the pin, four in all (rule 13). FB13's analysis had
not named one of them, the RPC initial sync (`getHeaders`), which also
runs after the initial sync through `getMissingHeaders`' fallback;
option (a)'s "whenever" covers it, and rule 13 names it. No further case
was found.

### What the code owes, added

- a test, in each sync mode, of a chain switch whose first attached block
  resolves a pending time birthday at a fork point below the stored tip
  that the store holds without its filter: the fork point's filter is
  requested with the attached blocks', checked against its stored header
  and written in the transaction that connects the resolving block; after
  it, `have_cfilters` reads 1, `MissingCFiltersHeight` returns `NotExist`
  when nothing else is missing, and a later chain switch that detaches the
  fork point reads its stored filter;
- a test that such a chain switch fails when the fork point's filter
  cannot be fetched or fails its check;
- the same case in the SPV initial sync after a reorganisation of the
  stored chain, with the fork point's filter requested once;
- the same case in the RPC initial sync after a reorganisation of the
  stored chain, with the fork point's `getcfilterv2` made once, and in
  `getHeaders` run through `getMissingHeaders`' fallback after the
  initial sync, likewise.

### Not measured, not examined, not done

- Nothing is implemented. No code, schema, export format or test changed
  with this addendum, and nothing was run for it: no oracle run, no
  benchmark, no timed measurement, no sync.
- How long a chain switch waits for the extra request was not measured,
  and how often the case arises was not examined (FB13 above).
- The items listed under "Not measured, not examined, not done" in the two
  addenda above stand.

### Places amended

This record: a pointer at the end of the Status block's third bullet and
a fourth bullet; dated pointers in item 7 of "What reads stored filters",
the preamble of the open questions, rule 1 (after its sentence on the
announcement paths), rule 3 of the addendum "2026-11-22 (later)" (after
"stay as rule 1 states them"), the third bullet of FB13's analysis (the
RPC initial sync), and "What the code owes, added" and "Not measured,
not examined, not done" of the addendum "2026-11-22 (later)"; the mark on
FB13; this addendum. ADR-0003 (a pointer in its
addendum of 2026-11-21); ADR-0001 (the pointer under its Status line);
`docs/PARITY.md` (the deliberate divergence, revised in place); the
README; the ADR index (the rows for ADR-0001, ADR-0003 and this record);
the brief (the Phase 0 status, §9 D2 and the M0 row);
`docs/PHASE0-EXIT.md`.
