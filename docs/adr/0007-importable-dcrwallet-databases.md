# ADR-0007 — Which dcrwallet databases can be imported, and what an import carries

- **Status:** Superseded by [ADR-0013](0013-no-import-seed-restore-only.md)
  (2026-11-18): superseded in full by the project owner's decision of
  2026-11-18 that dcr-purse imports no dcrwallet database and migration is
  by seed restore only. Nothing below is current; it is kept as the record of
  what was decided and why. Before that it was accepted in part
  (decision D4, project owner). 2026-11-14: one schema version only; older
  and pre-unified databases are refused with a message telling the user to
  open them once with a current dcrwallet; the 25 upgrade steps are not
  ported (brief §9 D4; `docs/PARITY.md`, "Deliberate divergences").
  2026-11-18: that one version is the pinned dcrwallet's database version,
  26 today; if dcrwallet raises it before v2.2, dcr-purse imports the new
  version and refuses 26 (Decision item 3). Proposed at the time: Decision
  items 2 and 4 to 7.
- **Date:** 2026-11-18

## Context

D4 left open what an import carries and how it behaves at the edges (brief
§9 D4, §3 W2). `purse-import` is built on the answers in Phase 3, ADR-0003's
schema must know which chain data arrives by import, and the Phase 0 exit
requires D1 to D10 decided (brief §6). D4 already removed the bbolt write
path, sized at L (spec 01 §12.5; brief §11), and the upgrade chain, about
1,200 lines of Go (spec 05 §12.5); PARITY.md marks that code n/a.

**The version record.** Top-level bucket `meta`, key `ver`, a 4-byte
big-endian integer; no `meta` marks a pre-unified database (spec 05 §4.1).
At the pin `DBVersion` is 26 (`wallet/udb/upgrades.go:213`), written by every
release from 2.0.0 to 2.1.7 (spec 05 §6.1), and a new database is always
created at `DBVersion` (spec 05 §5.6). dcrwallet upgrades a lower version on
open, in one transaction (spec 05 §5.1, §5.2), and refuses a higher one with
"database has been upgraded to an unknown newer version"
(`wallet/udb/open.go:36-38`); refusing lower versions is dcr-purse's
divergence (PARITY.md). Upgrades cannot be undone
(`wallet/udb/upgrades.go:210-212`). Writers differ inside version 26: in the
dcrwallet repository `git log release-v2.0.0..54b85b33 -- wallet/udb` lists
35 commits. Files initialised before `a83b85bb` (first in 2.1.0) hold a
`wstakemgr` bucket (spec 05 §4.5), and fixes such as `1b5e52ef` ("Fix reorg
transaction loss") and `66461422` (birthday of existing wallets) mean a 2.0.x
file can hold states the pinned code would not write.

**The reader.** `purse-boltread` is differential-tested against bbolt at 1 to
64 KiB pages (`crates/purse-boltread/tests/oracle.rs`). It never writes. In
one non-blocking attempt it takes a shared flock on Unix and, on Windows,
opens the file denying write sharing (`crates/purse-boltread/src/db.rs:225-275`;
PARITY.md); brief W2's "takes the lock dcrwallet takes" is not exact. It
selects the meta page as bbolt does and offers `verify_unchanged`. Upstream
fixtures at versions 1 to 24 (eight files) are under
`crates/purse-boltread/tests/data/upstream/`; none is at 26 (brief §7).

**Headers and filters.** `wtxmgr/h` holds every header ever accepted, side
chains included; `wtxmgr/cf` holds a 16-byte key and the version 2 filter
per block, without proof; `havecfilters` records completeness (spec 03 §4.3,
§4.4, §4.5.12). Both sync modes write them from genesis, checking each
fetched filter (RPC: spec 15 §5.5; SPV: spec 14 §4.15). `tip` and every
main-chain block need a header (I1) and, when `havecfilters` is `01`, a filter
(I13) (spec 03 §4.8); the chain cannot extend while filters are missing
(`wallet/udb/txmined.go:208-211`). Rescans and discovery match stored filters;
the ticket price comes from stored headers (spec 09 §1.1 item 8). SPV-mode
`getblockheader` answers from `h`, side chains included (spec 17 §5.3.17).

Above DCP0005 (mainnet 431,488, testnet3 323,328), and at every height on
other networks, dcrwallet accepts a fetched filter only if its proof
verifies against the header's `StakeRoot` (`validate/validate.go:19-25`,
`:69-102`; spec 15 §5.5.1). The commitment set has one item, so the root is
the filter hash (dcrd `CalcCommitmentRootV1`; dcroxide repository,
`crates/dcroxide-blockchain/src/validate.rs`, `calc_commitment_root_v1`): a
stored filter is checkable from its stored header. Below DCP0005 the set is
checked on request against a hard-coded hash (spec 08 §5.13; spec 09 §3.8).
`importcfiltersv2` writes filters unchecked at any height (spec 08 §5.14).
A header row is 212 bytes; the size of `h` and `cf` in a synced mainnet
wallet and the time of a full re-fetch are UNVERIFIED (the oracle's `stats`
over a wallet synced at the pin, and a timed sync, settle them).

**Seed-only restore.** Discovery (Phase 5) recovers BIP0044 accounts and
address use (`wallet/discovery.go:786` names them `account-<n>`); a restore
with discovery is in the M2 exit (Phase 6). The seed does not determine
imported keys, scripts, xpubs, account names or preferences (brief §1, §9
D14). By inference from where they are stored, it cannot recover
last-returned counters and per-account passphrase parameters (`acctvars`,
spec 02 §3.5), VSP records (spec 05 §4.3), unmined transactions (spec 03
§4.4) or the birthday (spec 03 §4.3).

## Options considered

**Headers and filters.**

- *Carry both, checked at import.* No traffic or first-sync delay; queries,
  ticket price and rescans equal the original's at once. Every carried filter
  is checkable. Cost: the store holds `h` and `cf`.
- *Re-fetch both.* Without `h`, `tip` and block records lose their headers
  (I1) and filters (I13); the importer would strip block placement and the
  wallet rebuild it by rescan, a path dcrwallet does not take. One RPC per
  block since genesis in RPC mode.
- *Carry headers, re-fetch filters.* Reuses dcrwallet's missing-filter path
  (spec 06 §5.8), but the chain cannot extend until it completes, and
  pre-DCP0005 filters arrive unchecked. No trust gain: the same commitment
  check applies to carried filters.

**Versions after a pin move.**

- *26 only, whatever the pin.* The text read literally. Once the pin's
  `DBVersion` rises, users of the target cannot import, and the oracle no
  longer writes 26.
- *The pin's `DBVersion` only.* A user at an older version upgrades with
  dcrwallet first, as D4 requires. No upgrade logic ever; a pin move brings
  only decoder work for the new layout.
- *Floor 26, ceiling at the pin.* Every 2.x user imports directly. But W2
  promises answers as dcrwallet at the pin would give, and it upgrades a 26
  file before answering (spec 05 §5.1). Keeping 26 means porting the effect
  of every later step of class R or L (spec 05 §6.1), the work D4 removed,
  plus a corpus from an older dcrwallet; each version-raising pin move (D12)
  waits on it.
- *Accept newer, decoding what the pin knows.* A step may change the meaning
  of existing bytes (version 8 reused credit flag `0x10`, spec 05 §4.6.6):
  funds data could be misread silently.

**Seed-only restore.** Document it as the fallback with a CI cross-check;
add an optional command that takes the seed and compares derived account
xpubs with the imported wallet, which checks the user's own wallet as R2's
"cross-check of what the import produced" (brief §10) can be read to ask,
at the cost of seed handling in one more gated code path (brief §7); or
leave it undocumented, giving a user with a lost file no described path.

## Decision

1. **Version 26 only** (accepted, 2026-11-14). Older and pre-unified
   databases are refused; no upgrade step is ported.

2. **Detection and refusal.** The importer opens the source with
   `purse-boltread`, reads `meta/ver` and never writes the source.

   | Found | Refusal (key wording) |
   | --- | --- |
   | locked, not bbolt, corrupt | the reader's own text (`crates/purse-boltread/src/error.rs`) |
   | no `meta`; `waddrmgr`, `wtxmgr` present | pre-unified; open it once with a current dcrwallet, let it finish syncing, stop it, import again |
   | no `meta`, otherwise | "not a dcrwallet wallet database" |
   | `ver` not 4 bytes, or 0 | "unreadable schema version record" |
   | below 26 | "imports schema version 26 only"; open it once with a current dcrwallet, which upgrades it irreversibly, stop it, import again |
   | above 26 | "newer than the version this dcr-purse imports (26)" |
   | `b` row 0 not the genesis | "belongs to <network>, not <network>" |

   The pre-unified remedy is UNVERIFIED: `Migrate` may not cover every such
   layout and resets a version 2 store's tip to genesis (spec 05 §5.5, §13
   item 3). Opening a pre-unified file with a 2.x release under the oracle
   settles it.

3. **The importable version is the pinned dcrwallet's `DBVersion`, one
   version only**, 26 today. A pin move that raises it replaces the version
   in the same move: decoders for the change, a Go-written corpus, and the
   number in item 2, W2 and PARITY.md. The previous version is then refused
   with item 2's upgrade message. This keeps D4's one-version rule and remedy
   and never ports upgrade logic; Accepted (project owner,
   2026-11-18).

4. **Headers and filters are carried and checked.** Refused on failure: each
   `b` row's header exists, hashes to its key, has the row's height and links
   to the row below; row 0 is the genesis; `tip` is the top row; side-chain
   headers hash to their keys; each main-chain filter decodes, its key equals
   bytes 36 to 52 of its header, and I13 holds. Reported on failure: for each
   stored main-chain filter above genesis, at or above DCP0005 on mainnet and
   testnet3 and at every height elsewhere, its hash (spec 15 §5.5.1 step 2)
   equals the header's `StakeRoot`; on mainnet and testnet3 the pre-DCP0005
   set hash, computed only when the tip reaches activation − 1 with every
   filter present, else reported "not checked: wallet not synced past
   DCP0005". Mismatching filters are carried as found, because dcrwallet at
   the pin keeps and serves them. `havecfilters = 00` is carried; the first
   sync fetches as dcrwallet would. Only the headers' proof of work and
   difficulty remain unverified.

5. **Unusual states.**

   | State | Disposition |
   | --- | --- |
   | mid-upgrade | cannot exist on disk; the old version is refused |
   | left by a crash | accepted: the meta page bbolt would open |
   | both meta pages invalid, damaged tree, big-endian host | refused (spec 01 §4.1) |
   | page size 1 to 64 KiB | accepted |
   | watching-only | accepted as found, sealed xprivs included (spec 02 §5.9) |
   | legacy coin type, `slip0044acct0` parked | accepted (spec 02 §5.10) |
   | migrated, or holding dead data (`wstakemgr`) | accepted; dead data not imported |
   | left by a defect of an earlier 2.x release (a reorg that lost a transaction) | carried as found: dcrwallet at the pin opens it as is |
   | rescan pending, birthday unresolved, filters missing | markers carried verbatim; the first sync resumes (spec 09 §3.1, §3.2) |
   | anomalies Go tolerates (stray `up`, orphan `tix`) | carried and reported (spec 03 §4.8 I8, I10) |
   | a row Go's decoders reject or panic on | refused, naming the row (ADR-0012, D10) |
   | filesystem without locks | refused unless an explicit option states dcrwallet is stopped; `verify_unchanged` then runs |

6. **Atomicity, passphrases, report.** The source is read as one snapshot,
   checked by `verify_unchanged` after the last read. The new wallet is
   built at a temporary path in the destination directory, flushed,
   re-opened and verified, then moved by a rename that cannot replace a file
   (or, lacking one, after an existence check under the data-directory lock),
   and the directory is flushed. A failure deletes the temporary file; an
   existing wallet is never overwritten. The data directory and its lock are
   ADR-0005's (D1). The import takes the public passphrase (default
   `public`, `wallet/wallet.go:66`), checks it as dcrwallet does (spec 02
   §5.5), unseals the account xpubs and re-derives every chain address. It
   never takes the private passphrase, the seed or an account passphrase;
   sealed private data is carried byte for byte. The report gives source
   identity, counts per kind, balances per account from source and from new
   store, chain state, warnings, and what a seed-only restore would lose; its
   fields and format are ADR-0005's.

7. **Seed-only restore is the documented fallback for corrupt or lost
   databases, from Phase 6 (M2), outside import.** A database refused for its
   version is pointed to item 2's upgrade, which loses nothing; before Phase
   6 that is the only route. CI runs the R2 cross-check over the corpus: from
   Phase 3, each BIP0044 account's xpub derived from the recorded seed equals
   the imported one; from Phase 6, a restore with discovery finds the same
   accounts. The optional seed command is rejected: sealed xpubs are
   authenticated (XSalsa20-Poly1305, spec 02 §1), so corruption fails the
   unseal, and item 6 re-derives every address; a file from another seed is
   the user's choice, not an import defect.

## Consequences

- **Work created.** Detection, checks and report; a corpus with watching-only,
  legacy coin-type and migrated wallets, a pending rescan, `havecfilters =
  00`, bad filters above and below DCP0005, a crash-left file, 16 KiB pages,
  and wallets written by a 2.0.x and a 2.1.x release; refusal tests over the
  upstream fixtures, a pre-unified file and an edited version-27 file; a seed
  record per corpus wallet and the seed harness; a pin-move checklist entry
  for `DBVersion`; a correction of brief W2's lock sentence (brief §12 item 6).
- **Work avoided.** Upgrade logic, now and at every pin move; an import-only
  re-fetch path; seed handling at import.
- **Constraints.** ADR-0003 holds every header and filter, the "filters
  missing" state and markers verbatim. Side-chain headers are carried because
  `getblockheader` reads them; the in-memory side-chain forest is not (W2).
  ADR-0001's export covers the tip, main-chain header hashes, the side-chain
  header set, a digest per filter, `havecfilters`, rescan point and birthday.
  ADR-0005 carries item 2's messages and item 6's report.
- **Risks accepted.** Proof of work and difficulty are not re-checked in
  Phase 3; filters failing a commitment check are carried with a warning;
  between a version-raising pin move and the v2.2 tag, a user at the old
  version needs an unreleased dcrwallet to upgrade.
- **Facts not established.** Mainnet sizes and re-fetch times; the
  pre-unified remedy; whether dcrwallet master has passed 26 since the pin.
- **Revisit** if carried filters often fail the commitment checks, if
  ADR-0003's size budget cannot hold `h` and `cf`, or if users must still
  import a version a pin move has passed.

## Open points

1. Should filters failing a commitment check be marked missing for re-fetch?
   Proposed: no; carry, warn, and point to `importcfiltersv2` and, below
   DCP0005, `repaircfilters`, which dcr-purse will implement (Phase 7;
   `purse-tools`).
2. Should the import re-check proof of work and difficulty once
   `purse-chain` exists (Phase 5)? Proposed: yes, unless it dominates import
   time.
3. Should an import without the file lock be possible? Proposed: yes, behind
   item 5's explicit option.
4. Decided (project owner, 2026-11-18): "version 26 only" means the pinned
   dcrwallet's `DBVersion` only; if a pin move raises it, the new version is
   imported and 26 is refused.
5. Should an import proceed without the public passphrase? Proposed: no.
6. Should a user-run seed check exist outside import? Proposed: not in Phase
   3; reconsider with the seed restore in Phase 6.
