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
  FB3 recommend it for discovery and the filter RPCs.
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
     height at or above it (`cfilters_detached`);
   - the `birth` and `have_cfilters` rows.

   No other filter the wallet obtains itself is kept: none the syncers
   fetch, none the missing-filter pass fetches (item 9 of "What reads
   stored filters" below), and not the genesis block's filter, which
   dcrwallet writes at creation (Context), unless the birthday height is 0.
   For a wallet born after genesis that filter is one of the filters below
   the birthday, and the owner's sentence drops it with the rest. Filters a
   client supplies (`importcfiltersv2`, `cmd/repaircfilters`; item 11) are
   not settled by this item: whether the sentence covers them is the first
   part of FB5. A wallet whose birthday is the genesis block keeps every
   filter, exactly as dcrwallet does; which wallets those are is item 2.
   Of the filters that can lie below a birthday, the genesis filter is the
   only one that needs no fetch: it is built locally from the genesis
   block, as upgrade 15 builds it. A store whose birthday is genesis
   therefore holds it although no block connection writes it, written at
   creation as dcrwallet writes it or, for a birthday pending at creation
   that resolves at genesis (item 3: a height birthday of 0, or a time
   birthday earlier than block 1's timestamp), when it resolves. The
   headers stay because difficulty validation walks back over stored headers
   (`wallet/difficulty.go:54`, `:593`, `:864`), the filter checks of the
   missing-filter fetch read them (`wallet/wallet.go:1377`), and
   `getblockheader` returns them, detached ones included (ADR-0003 Decision
   item 5 and Open point 5).
2. **Both sync modes.** The store's contents do not depend on the mode it
   was synced in, and one wallet may be run in either. Whether the filters
   below the birthday are fetched at all during the initial sync is open
   point FB1.
3. **No schema change.** `purse-store`'s chain tables already allow a
   main-chain block without its filter and a detached header without its
   filter (`crates/purse-store/src/chain.rs`, invariants 3 and 7; ADR-0003,
   addendum "Chain tables keyed by height, as built"). Schema version 1 is
   unchanged. What the `have_cfilters` row then means is FB9.
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
   neither fetching nor failing (item 7). No reader would use a filter
   fetched there: the SPV announcement path fetches a fresh filter for
   every node of the new best chain (`spv/sync.go:1488-1519`), RPC mode one
   for every announced block (`chain/sync.go:860-869`), and the store keeps
   none below the birthday; a later reader that matches or serves that
   block's filter falls under the rule above. The presence checks of the
   missing-filter pass (`MissingCFiltersHeight`, the pass's own
   `CFilterV2` reads, and `InsertMissingCFilters`' read of a parent's
   filter; item 9) only test whether a filter is stored; what they do is
   FB9.
5. **How the divergence is recorded.** In `docs/PARITY.md`, under
   "Deliberate divergences from dcrwallet", as an owner decision about
   dcr-purse's own store, beside the database decision. It has no
   `docs/QUIRKS.md` entry: it is not a dcrwallet defect, and the owner's
   earlier design departures (the database, the P2P user agent) have none.
   Each RPC-visible difference that follows from the answers to FB2, FB3,
   FB5 and FB9 gets its own PARITY.md row when the code is written, under
   ADR-0012 Rule 10.

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
   Decrediton's requests were not examined.
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
   A birthday above the tip stays pending ("searches until it is passed",
   `:208-209`), the marker stays at genesis, where creation put it
   (`wallet/udb/upgrades.go:909`), and the first rescan starts at height 1
   (item 4); dcr-purse holds no filters below the tip in that state, and the
   rescan's reads fall under FB2.
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
   this for a typed birthday: fetch on demand (FB2).
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
     birthday the store keeps none (where that rule lives is FB4).
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
     the birthday included, is part of FB1.
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
   what dcr-purse does with a store that has no `birth` row is FB10. No
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
   notifications are sent as dcrwallet sends them.
10. **Filters served to clients.** `getcfilterv2` returns the stored filter
    of a block by hash (`methods.go:1955-1977`; `wallet/wallet.go:833-842`);
    gRPC `GetCFilters` streams the stored filters of a range, the whole main
    chain by default (`server.go:2179-2211`; `wallet.go:855-878`);
    `validatepredcp0005cf` hashes every stored filter below the DCP0005
    height and compares the hash with a constant (`methods.go:5392-5399`;
    `wallet.go:5230-5268`; `validate/validate.go:107-130`; QK-0215). Each
    reads a filter below the birthday of any wallet born after its range
    starts (FB3). dcrwallet serves no filter to peers: it never sends
    `cfilterv2` or `cfiltersv2` (spec 14 §3.3).
11. **Filters a client supplies.** `importcfiltersv2` writes the filters it
    is given at consecutive main-chain heights, unchecked against the
    headers and without touching `havecfilters` (`methods.go:1981-2002`;
    `wallet/wallet.go:5278-5287`; `wallet/udb/cfilters.go:34-67`; QK-0061,
    QK-0106). `cmd/repaircfilters`, which `purse-tools` is to port (brief
    §5), reads a file of pre-DCP0005 filters and imports them from height 0
    in batches (`cmd/repaircfilters/main.go:186-237`), for
    `validatepredcp0005cf` to check afterwards. On a dcr-purse wallet born
    after height 0 those writes land below the birthday (FB5).
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

Nothing was run for this record: no oracle run, no benchmark, no timed
measurement, and no live sync in either mode; the reorganisation paths of
item 7 were read in the source, not exercised.

## Consequences

- The SPV and RPC syncers (brief §6 Phases 5, 6 and 10) decide, per block,
  whether its filter is fetched and passed to the store; the rescan, the
  discovery and the filter RPCs gain a path for filters below the birthday
  once FB2 and FB3 are answered. Each path that matches a filter against
  the wallet or returns one to a client gets a test that a missing filter
  is fetched or fails and is never skipped (decision item 4), the SPV
  announcement path after a reorganisation across the birthday included
  (item 7). The detach path of `ChainSwitch` gets a test that a block
  below the birthday is detached with its side-chain node rebuilt without
  a filter, and that a later scan of that block on the announcement path
  fetches its filter.
- Comparisons with Go of a wallet born after genesis run under the
  projection of FB6; until it is specified, such comparisons use wallets
  whose birthday is genesis, which keep every filter. In the brief these
  are the Phase 4 and Phase 6 exits, the Phase 8 (M3) soak, the Phase 10
  (M5) exit and its §11 row, the 1.0 exit, and §7 Layers 6 and 7, each of
  which carries a dated pointer here.
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

## Open questions for the project owner

Each with a recommendation.

- **FB1. Fetching below the birthday during the initial sync.** The store
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
  resolves.
- **FB2. Rescans and discoveries from below the birthday in SPV mode**
  (items 5 and 6). S15 decided, for a typed birthday, to fetch on demand for
  rescans below it. Recommended: the same for every birthday and for
  discovery: fetch the filters of the range from peers, check those above
  the DCP0005 heights against the stored headers as the initial sync does,
  accept those below as dcrwallet's sync accepts them, use them, and keep
  none of them, so that decision item 1 holds afterwards. A rescan with
  dcrwallet's defaults then fetches about a million filters on mainnet each
  time it runs (not measured). The genesis filter, which `discoverusage`
  and `DiscoverUsage` read from their default start at the genesis block
  (item 6), is built locally as upgrade 15 builds it rather than fetched.
- **FB3. The filter RPCs below the birthday** (item 10). Options: fetch on
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
- **FB4. Where the rule lives.** Recommended: in the syncers, which do not
  pass a filter below the birthday to the store; the `purse-udb` port of
  `ExtendMainChain` accepts a block without a filter only below the birthday
  and refuses one above it, and its other operations stay as dcrwallet's
  (apart from the start of `MissingCFiltersHeight`'s walk, if FB9 is
  answered as recommended), so that ADR-0002's operation scripts, which
  always supply filters, compare as before.
- **FB5. `importcfiltersv2` and `repaircfilters` below the birthday** (item
  11). First, whether the owner's sentence covers filters a user supplies
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
- **FB6. The logical export** (item 12). Recommended: a comparator
  projection that drops, on both sides, the `[cfilters]` records of heights
  below the `birth` record's height (a side filter's height read from its
  `sideheader`) and compares the segment holding the birthday in full mode;
  `havecfilters` is compared as it is if FB9 is answered as recommended
  (1 on both sides), and dropped from the comparison otherwise. Specified in
  ADR-0001 with the Rust producer of `[cfilters]`. And, because the Rust
  side would otherwise list one `cfmissing` line per height below the
  birthday, about a million on mainnet, a `[cfilters]` record stating that
  range once, which raises the section's version under ADR-0001 §10.
- **FB7. Wallets created through gRPC.** Under QK-0247 and QK-0554,
  `CreateWallet` gives genesis or `birth_height` as the birthday (item 2),
  so wallets created that way, Decrediton's among them, gain nothing from
  this decision unless the client sends a height with `set_birth_time`. The
  birthday reaches clients (`walletinfo`, gRPC `BirthBlock`,
  `methods.go:5531-5538`, `server.go:628-648`) and the restore contract.
  Recommended: leave QK-0247 and QK-0554 to their own dispositions under
  ADR-0012 Rule 1, and accept that such wallets keep every filter.
- **FB8. The birthday block under a time birthday** (item 3). The child
  that resolves the birthday is above it, and its filter, which dcrwallet's
  first rescan reads (item 4), is kept under decision item 1 with no late
  decision needed. The question is the birthday block's own filter, which
  is known to be the birthday block's only when the child arrives.
  Recommended: keep it, as decision item 1 says, by deciding one block late
  in the syncer, so that both the birthday block's filter and the child's
  are kept. Fetching it again when the child arrives would also keep it, at
  the cost of a second request for a filter the initial sync may already
  have had.
- **FB9. What `have_cfilters` means in dcr-purse.** In dcrwallet the row is
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
- **FB10. A store with no `birth` row** (item 8). dcrwallet's `BirthState`
  reads an absent row as no birthday, and no database at the pin lacks one
  (item 2). `purse-store` reads an absent row as none set
  (`purse_store::MetaKey::Birth`, `crates/purse-store/src/keys.rs`). With
  no birthday, the owner's sentence drops nothing on a plain reading.
  Options: treat an absent row as the genesis block and keep every filter;
  or refuse to connect blocks until a birthday is set. Recommended: the
  former. If dcr-purse's creation writes the row as dcrwallet's does (item
  2), the case arises only from a defect or a damaged store, and keeping
  every filter loses nothing there.
