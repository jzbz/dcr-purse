# ADR-0006 — How dcroxide crates are consumed

- **Status:** Accepted (decision D3; project owner, 2026-11-18) as proposed, with one change: dcr-purse keeps the latest stable Rust and does not follow dcroxide's point-release rule (rule 8; Open point 6). The repository has worked this way since its first commit. The redb version is the owner's decision of 2026-11-18 (ADR-0003) and is not reopened here.
- **Date:** 2026-11-18

## Context

dcr-purse takes its Decred primitives from dcroxide, which covers most of the dcrd modules dcrwallet imports (brief §2.1; spec 24 §14). D3 asks how: inside the dcroxide workspace, a sibling workspace with pinned git dependencies, or published crates (brief §9 D3; also spec 24 §13 D-1, D-2, D-9; spec 25 §13.1 D-2). The brief recommends a separate workspace at a pinned revision, additive changes landed in dcroxide first, and an allow-listed git source. Phase 0 cannot exit without D1–D10 decided (brief §6, Phase 0 exit), and later phases need dcroxide additions (brief §5, last paragraph), so the rules are needed now.

Facts that bear on it:

- **In use since 2026-11-18.** `3237b79` declared dcroxide crates as git dependencies at one `rev` ("Every dcroxide crate names the same `rev`; move them together", root `Cargo.toml:16-25`). `b40513c` moved every entry to dcroxide `5c6742f`, which had added HD derivation's secp256k1 entry points to `dcroxide-dcrec` the same day, and moved `purse-hdkeychain` onto them. (The pin was `f4b3d2e` from 2026-11-19, `2074168` from 2026-11-20, and is `48f5b5e` since 2026-11-21; addenda below and `docs/SKEW.md`.) The work in progress adds `-chainhash`, `-dcrjson` and `-wire` at that revision.
- **Sources.** `deny.toml` denies unknown registries and git sources and admits `[sources.allow-org] github = ["jzbz"]` (`deny.toml:57-63`): every repository of the organisation, dcr-rs included.
- **Duplicates.** Both workspaces set `multiple-versions = "deny"` with one skip, `syn:2` (`deny.toml:23`, `:52-55`; dcroxide `deny.toml`). The root `Cargo.lock` holds one `digest` (0.11.3), one `k256` (0.14.0, reached only through `dcroxide-dcrec`), and `syn` 2 and 3. The work in progress resolves `scrypt` and `argon2` on `digest` 0.11, so the duplicate brief §8 expected from `argon2` 0.5.3 does not arise.
- **Stake and gcs enter in Phase 3.** dcrwallet's `wallet/udb` imports `blockchain/stake` (`txdb.go:16`; also `txmined.go`, `txunmined.go`, `txquery.go`, `treasury.go`) and `gcs` (`cfilters.go:11-12`, for `Store.CFilterV2`). Phase 3 ports that package as `purse-udb`, and M1 is P1–P3 (brief §6 Phase 3, §11). `dcroxide-gcs` depends on `dcroxide-stake`, which depends on `dcroxide-database` (dcroxide `crates/dcroxide-stake/Cargo.toml:13`, `crates/dcroxide-gcs/Cargo.toml`), which depended on `redb = "4"`, locked at 4.5.0, at the revision this record was written against (`5c6742f`; dcroxide `crates/dcroxide-database/Cargo.toml:15`, `Cargo.lock`). With purse-store on redb 5, Phase 3 would then have put redb 4 and 5 in the root lock, which `multiple-versions = "deny"` refuses. Since dcroxide `f4b3d2e` it depends on `redb = "5"`, locked at 5.0.0 (addendum of 2026-11-19 below). Since dcroxide `80423a0` `dcroxide-stake` depends on `dcroxide-database` only through its default feature `storage`, which `dcroxide-gcs` leaves off (dated note in that addendum).
- **Toolchain.** dcr-purse pins Rust 1.100.0 (`rust-toolchain.toml`; `Cargo.toml:12`). dcroxide pins 1.99.1, and its ADR-0014 part 1 makes 1.100.0 ineligible "until it has a `.1`" or 1.101.0 ships without one. Brief §6 Phase 0 puts the workspace "under dcroxide's lint, deny and toolchain policy". The reference Go differs too: 1.27.4 here (owner, 2026-11-18; `.github/workflows/ci.yml:17`), 1.27.3 in dcroxide (its `ci.yml:42`). The release profile is copied, since profiles come from the building workspace (spec 24 §12.1; root `Cargo.toml:36-41`).
- **What the differentials cover.** `tools/oracle/go.mod` links dcrd modules at dcrwallet's versions (`hdkeychain` v3.1.3, `secp256k1` v4.4.1, `wire` v1.8.0), not dcroxide's dcrd master. A dcroxide change that alters a value the wallet reaches is caught only on paths an existing differential covers; `tools/oracle/cmd` has shims for errors, seeds, hdkeychain, the bolt dump, dcrutil and snacl, none for chainhash, dcrjson or wire as such.
- **dcroxide is unpublished.** Its 35 crates are 0.1.0 with path dependencies between them; `dcroxide-secp256k1-vartime` (a dependency of `dcroxide-dcrec`), `-sigdiff` and `-testutil` set `publish = false` (spec 24 §12.1). dcroxide ADR-0014 part 1 records no crates.io release. dcroxide keeps one branch, master (its ADR-0013 item 4). `cargo publish` refuses a dependency given only by git: `purse-hdkeychain` and `purse-dcrutil` depend on dcroxide; `purse-errors`, `purse-seed`, `purse-boltread` and `purse-snacl` do not (their manifests).
- **Node refactors.** The brief counts the extractions from `dcroxide-node`/`dcroxide-rpc` listed in rule 3 as refactors, not additions (brief §5, last paragraph; spec 24 §12.5). Linking `dcroxide-node` instead would link the whole node.
- **Key agreement.** dcroxide ports dcrd's `mixing` root package and re-exports its DC-net functions (dcroxide `crates/dcroxide-mixing/src/lib.rs:23-27`), though outside tests only `mixclient` calls `SRMixPads` (`mixing@v0.7.5`: `mixclient/client.go:1845`, `blame.go:386`). Its ledger records `keyagreement.go` as "Not yet ported", deferred with the client (dcroxide `docs/PARITY.md:1035`). That file at dcroxide's dcrd pin `46e9df48` is byte-identical to `mixing@v0.7.5`'s, and both require the same sntrup4591761 pseudo-version (`mixing/go.mod:7`; compared 2026-11-18).

## Options considered

**A. One workspace** (wallet crates join dcroxide's, or both move to a new repository). One commit per cross-cutting change (`5c6742f` and `b40513c` would be one); one lockfile, so no duplicate across the projects; one toolchain, deny policy and CI. Path filters keep node soaks off wallet-only changes, but a change to a shared crate runs both suites. Costs: dcroxide's dcrd pin moves and the wallet's dcrwallet pin moves gate each other, with two reference Go releases in one CI; dcroxide's `deny.toml` (`all-features = true`) and dependency ledger take in the gRPC, KDF and protobuf choices; the two PARITY ledgers share one history and one branch.

**B. Separate repository and workspace, git dependencies at one pinned revision** (current practice). Each project keeps its own cadence, CI, toolchain and reference Go. Costs: each needed dcroxide change is two commits in two repositories plus a pin move; the release profile and lints are copied by hand; duplicate-freedom holds per lockfile; dcroxide's CI does not run the wallet's suites, so a break shows at the next pin move.

**C. Published crates.** Exact registry versions, no git source. This requires of an unreleased node: making `dcroxide-secp256k1-vartime` publishable, version numbers with semver meaning, an MSRV floor for library crates (dcroxide ADR-0014 part 1), and a release before each change the wallet needs. Nothing is published (spec 24 §12.1). Copying the crates instead (spec 24 §12.3, form C) loses dcroxide's fixes and is "only justified if a required change is refused upstream"; it survives only as rule 4's exception.

**D. One repository, two workspaces, path dependencies** (spec 25 §13.1 D-2's sibling checkout, made reproducible). Atomic cross-cutting commits, with separate lockfiles, deny policies and workflows. Costs: there is no pin, so every dcroxide commit must keep the wallet's suites green and the wallet's CI gates the node; dcroxide ADR-0013 item 4's one-branch rule then governs wallet work; the PARITY ledgers and tags share a history.

The single-maintainer argument for A and D: one owner reviews and signs both sides under every option (brief §7, Gates), and a dcroxide addition carries dcroxide's gates under A as under B. A and D save only the second commit and the pin move: fourteen items in brief §5's last paragraph. That does not outweigh coupling the two upstream cadences.

## Decision

**B, confirming the brief's recommendation, with these rules.**

1. **One revision.** Every dcroxide crate is declared once, in root `[workspace.dependencies]`, at one `rev` on dcroxide master; members use `workspace = true`. `fuzz/Cargo.lock` resolves to the same revision and is updated in the commit that moves the pin; CI's `cargo metadata --locked` on `fuzz/` (`.github/workflows/ci.yml:33`) fails when it lags. A spike keeps the revision it was made at.
2. **Moving the pin.** The pin moves when the wallet needs something landed in dcroxide, when dcroxide fixes a defect in a linked crate, when a shared dependency generation moves, and at least once per wallet milestone. It does not otherwise follow dcroxide's commits or dcrd pin moves. The commit that moves it (a) changes only the revision and what the move requires, `fuzz/Cargo.lock` included; (b) lists the dcroxide commits in the range that touch linked crates; (c) passes the workspace suites with `PURSE_REQUIRE_ORACLE=1`, the frozen vectors, `cargo-deny` and the fuzz lock check; (d) re-runs the dcroxide-backed acceptance scenarios of the milestones reached, whose node is built from the pinned revision (brief §6 Phase 6 exit; §7 Layer 7); (e) records the revision and range in `docs/PARITY.md`, which names no revision today. The engineer making the move runs (a)–(e). The project owner reviews list (b) for wallet-visible effects in code no differential covers, as dcroxide ADR-0013 item 7 makes dcroxide's own pin watch the owner's, and signs the commit. The dcrd-module differences a move brings are ADR-0009's record (D7).
3. **Needing something from dcroxide.** The change is made in dcroxide first under dcroxide's rules (its brief §7 Gates, line 723, and §12 item 4 for consensus-tagged crates such as `dcrec`, `wire`, `stake`, `gcs` and `mixing`'s validation paths; `no_std` kept on the nine checked crates, spec 24 §12.1), merged to master, and then the pin moves. No fork, `[patch]`, path override or sibling path is committed in any manifest. Each item lands, and the pin reaches it, before the first wallet phase that needs it:

| dcroxide item (brief §5, last paragraph) | Kind | Before |
|---|---|---|
| `dcroxide-dcrec` remaining key entry points and `SignatureType` conversion; `dcroxide-crypto` `rand`; `dcroxide-wire` `PkScriptLocs` and small items | additive | Phase 1 (brief §6 Phase 1); `PkScriptLocs` at the latest Phase 3 (brief §2.1) |
| redb 5 across dcroxide; `dcroxide-stake` storage feature gate, with `dcroxide-gcs` taking stake without it | owner decision; additive | Phase 3 (rule 5) |
| SOCKS dialer with `Pool` and `ProxiedAddr`; websocket frame codec with both roles, in one dcroxide library crate | refactor | Phase 6 (spec 15 §12.2, open question 13) |
| HTTP/1.1 server; client-certificate verifier; `dcroxide-dcrjson` request decoder and bare-`Response` marshaller; `dcroxide-certgen` P-384 and client-CA templates | refactor; additive | Phase 7 (specs 16, 21) |
| go-flags layer at v1.5.0; slog backend and rotator at logrotate v1.0.0 | refactor | Phase 8 (spec 21) |
| Tor-resolver wrapper; `DynamicBanScore` in a small crate | refactor | Phase 10 (spec 24 §13 D-3) |
| `dcroxide-mixing` key agreement with a new `sntrup4591761` crate; client-usable pool | additive | Phase 11 (rule 6) |

   A refactor leaves the node's observable behaviour unchanged under dcroxide's suites and takes the library version as a parameter where dcrwallet's pin differs (brief §2.1). The project owner schedules the items, since they compete with dcroxide's own work. This answers spec 24 D-2 and D-9 and spec 15 open question 13: extract and extend in dcroxide; the codec's client role lives in dcroxide; `purse-wsrpc`, the JSON-RPC 2.0 layer over it, stays in dcr-purse.
4. **The one exception: a bounded wallet-side copy.** A wallet-side implementation of an item of rule 3 is written only when the project owner declines the dcroxide change (it would alter the node's behaviour) or defers it past the start of the phase that needs it. A declined item is recorded in `docs/PARITY.md` under "Deliberate divergences" with the reason. A deferred item is recorded there as temporary, naming the dcroxide item that replaces it, and is deleted in the first pin move that carries the dcroxide version.
5. **Duplicates across the two lockfiles.** The wallet's `cargo-deny` sees only the dcroxide dependencies the wallet links. Before adopting a third-party major that dcroxide's lock at the pin also holds, the wallet takes dcroxide's version, or dcroxide moves first. A `bans.skip` in one workspace is mirrored in the other or justified in its reason. secp256k1 is reached only through `dcroxide-dcrec`. redb is the owner-decided case: dcroxide moves to redb 5 and adds the stake gate, and purse-store may use redb 5 before the pin reaches that move only because no wallet crate links `dcroxide-stake` or `dcroxide-gcs` before Phase 3. **Phase 3 does not start until dcroxide's redb 5 move has landed and the pin has moved to it.** The gate is due at the same pin move; if it is late, Phase 3 links `dcroxide-database` until it lands, since the majors then match. The gate keeps the node's ticket database out of the wallet; it does not let the two redb majors differ. *Precondition met on 2026-11-19: dcroxide `f4b3d2e` moved `dcroxide-database` and `dcroxide-testutil` to redb 5.0.0 and the pin moved to it; the stake gate has not landed, so Phase 3, if it starts before the gate does, links `dcroxide-database` (addendum below).* *Gate met on 2026-11-20: dcroxide `80423a0` put the ticket database behind `dcroxide-stake`'s default feature `storage`, `dcroxide-gcs` takes `dcroxide-stake` without it, and the pin moved to `2074168`; a wallet crate takes `dcroxide-stake` with `default-features = false` and links neither `dcroxide-database` nor redb through it, so the fallback above is not needed.*
6. **Crates marked † in brief §5.** A crate goes to dcroxide when (a) it fills a gap in a dcrd package dcroxide already ports (spec 24 §5.10 step 3) or the node would call it; (b) dcroxide has recorded no decision against it; and (c) its reference version can be dcroxide's without breaking wallet parity (ADR-0009). After a move, the wallet's oracle differential for it keeps running in dcr-purse against dcrwallet's version. Otherwise it stays here.

| Crate | Home | Reason |
|---|---|---|
| `purse-hdkeychain` | dcr-purse | dcroxide has no hdkeychain port and records why (dcroxide ADR-0002:122-125; spec 24 §3.9) |
| `purse-dcrutil` | dcr-purse | dcroxide decided against a dcrutil crate and distributed its contents (dcroxide `docs/PARITY.md:1044`), though the node keeps private copies (`new_amount` twice, `app_data_dir`, `to_coin`; spec 24 §3.4) that a shared crate could replace; that is dcroxide's call |
| key agreement, `sntrup4591761` | dcroxide | fills dcroxide's partial `mixing` port, deferred there rather than excluded; file and KEM version are equal at both pins (Context) |
| `purse-mixclient` | dcr-purse | dcroxide's brief makes the client a non-goal and an "optional later library milestone" (its line 23) |
| `purse-wsrpc` | dcr-purse | jrick/wsrpc is not a dcrd module; it uses rule 3's codec |

7. **`deny.toml`** replaces `[sources.allow-org]` with `allow-git = ["https://github.com/jzbz/dcroxide"]`.
8. **Toolchain.** dcr-purse pins the latest stable Rust, 1.100.0 today, in `rust-toolchain.toml` and `rust-version` together, and moves to each new stable release by a deliberate bump. It does not follow dcroxide ADR-0014 part 1's rule of waiting for a point release (decided by the project owner, 2026-11-18, against the proposal; open point 6). The owner has also directed that dcroxide move to the latest stable Rust if nothing blocks it.

Rules 1–7 confirm and sharpen the brief's recommendation. Rule 8 departs from brief §6 Phase 0, which asks for dcroxide's toolchain policy.

## Consequences

- Work created in dcr-purse: the `deny.toml` change, the PARITY.md revision line, the toolchain move. In dcroxide: rule 3's items, scheduled by the project owner, redb 5 and the stake gate first, since Phase 3 waits on them.
- Work avoided: the wallet carries none of dcroxide's interop jobs or soaks; the dcrd and dcrwallet pin cadences stay independent.
- Risks accepted: a dcroxide change breaks the wallet unseen until the next pin move; each rule 3 item costs two signed commits and a pin move; the reference Go releases can drift.
- UNVERIFIED: whether cargo-deny's `multiple-versions` flags two revisions of one dcroxide crate at the same version (0.1.0). A scratch workspace under the artifacts directory, with two crates naming different dcroxide revisions, run through `cargo deny check bans`, settles it. A CI check for one dcroxide source per lockfile is added only if cargo-deny does not flag them.
- Publishing: no dcr-purse library that links dcroxide is publishable while dcroxide is not. R10's M1 offer (brief §10) is then `purse-seed` and `purse-boltread` from crates.io and `purse-hdkeychain` as a git dependency.
- If dcroxide tags releases, nothing changes: a tag is a revision. If dcroxide publishes to crates.io, the wallet moves to exact registry versions in one commit, drops the git source, rule 3 becomes "released first", and this ADR is revisited.
- Constrains: ADR-0003 (the redb major and Phase 3 start, rule 5); ADR-0004 (the gRPC stack's TLS and crypto generations match dcroxide's linked crates); ADR-0009 (each pin move feeds its skew record).
- Revisit, toward A or D, if a wallet phase exit waits on a pending dcroxide item, or one phase needs more than five pin moves.

## Open points

Decided as proposed (project owner, 2026-11-18), except point 6, which was decided the other way. They are kept as the record of what was asked and answered.

1. Should `deny.toml` admit only the dcroxide repository? Proposed: yes (rule 7).
2. Do key agreement and `sntrup4591761` go to dcroxide or stay in dcr-purse? For dcroxide: otherwise its `mixing` port stays partial for good, its ledger defers the file rather than excluding it, it already ports DC-net functions only the client calls, and the file is equal at both pins. For dcr-purse: no node code calls key agreement, and the KEM, the highest-risk new code (spec 24 §12.4), would come under dcroxide's gates. Proposed: dcroxide, as brief §5, spec 24 §12.3 and `docs/PARITY.md:57` already say; dcroxide's ledger row changes when it lands.
3. Should the pin also move on a cadence, not only on need? Proposed: at least once per wallet milestone (rule 2), so ADR-0009's skew record does not grow stale.
4. Should dcroxide's CI build dcr-purse at its pin? Proposed: not now; revisit before Phase 3, when the wallet first links `dcroxide-stake` and `dcroxide-gcs`.
5. Who declines or defers a dcroxide item under rule 4? Proposed: the project owner, on dcroxide's review of the change or at the phase start.
6. Does dcr-purse follow dcroxide ADR-0014's point-release rule? Decided (project owner, 2026-11-18), against the proposal: no. dcr-purse stays on the latest stable Rust (rule 8).

## Addendum, 2026-11-18 — ADR-0013

On 2026-11-18 the project owner decided that dcr-purse imports no dcrwallet database: migration is by seed restore only (ADR-0013). The decision of this ADR and its rules 1 to 8 stand. Two of its statements rested in part on the import:

1. **Rule 3, `PkScriptLocs` "at the latest Phase 3 (brief §2.1)".** The deadline stands; its reason is restated. Brief §2.1 gave two users, dcrwallet's credit rows "and so the importer". The importer is gone. In dcrwallet, `MsgTx.PkScriptLocs` is called only by the transaction store, which records each credit's script offset (`wallet/udb/txmined.go:1420`, `:1501`, `:1534`, `:2122`; spec 04 §4.5; spec 24 §3.1). `purse-udb` ports that logic in Phase 3, the first phase whose code can call it; whether dcr-purse's own schema stores the offset is ADR-0003's question. Proposed: keep the item due before Phase 3, since it is small and additive, and drop it from rule 3's table if ADR-0003's schema stores no script offset and no other caller appears.
2. **Consequences, "Publishing".** R10's M1 offer (brief §10) named `purse-boltread` beside `purse-seed` because the reader was the importer's foundation. `purse-boltread` is now a test utility only, not shipped in the wallet, and the owner may remove it later (ADR-0013 Open point 2). Proposed: it is not offered as part of M1 and not published while it is a test utility; the M1 offer is `purse-seed` from crates.io and `purse-hdkeychain` as a git dependency. The rest of that consequence (no library that links dcroxide is publishable while dcroxide is not) stands.

Unaffected, and checked: the Context item "Stake and gcs enter in Phase 3" and rule 5's gate ("Phase 3 does not start until dcroxide's redb 5 move has landed and the pin has moved to it"), whose reason is `purse-udb`'s use of `stake` and `gcs`, not the import; the Context's list of crates that do not depend on dcroxide, `purse-boltread` among them, which is a fact about manifests.

## Addendum, 2026-11-19 — dcroxide on redb 5

dcroxide `f4b3d2e` declares `redb = "5"` in `dcroxide-database` and `dcroxide-testutil` and locks 5.0.0, and its power-loss backend returns redb's `BackendError` (dcroxide `crates/dcroxide-database/Cargo.toml:15`, `Cargo.lock`, `crates/dcroxide-testutil/src/powerloss.rs`). The workspace pin moved from `5c6742f` to `f4b3d2e` the same day (`docs/SKEW.md`, 2026-11-19 section; ADR-0009 addendum). Rule 5's precondition for Phase 3, dcroxide's redb 5 move landed and the pin on it, is met. Its second half is not: `dcroxide-stake` still depends on `dcroxide-database` unconditionally, so a wallet crate that links `dcroxide-stake` or `dcroxide-gcs` links the node's database crate, with redb at whatever 5.x dcroxide's lock resolves (5.0.0 at this revision, the release the root manifest pins exactly). The Context bullet "Stake and gcs enter in Phase 3" describes the earlier revision.

*2026-11-20: the second half is met too. dcroxide `80423a0` makes `dcroxide-database` an optional dependency of `dcroxide-stake` behind its default feature `storage`, and `dcroxide-gcs` takes `dcroxide-stake` with `default-features = false` (dcroxide `crates/dcroxide-stake/Cargo.toml`, `crates/dcroxide-gcs/Cargo.toml`). The pin moved from `f4b3d2e` to `2074168` the same day (`docs/SKEW.md`, 2026-11-20 section; ADR-0009 addendum of that date). The range also holds the other items of the addendum below: the k256 tables (`5ace7a2`), the small additions (`69dfe8d`: `Int32N` and the other bounded draws, the versioned `is_null_data_script`, lookup errors carried through `SignError::other`, and the guard against a pay-to-pubkey script whose key does not parse, with two more where dcrd panics or recurses without end), and the timing checks and micro benchmark (`8f2b246`). No dcr-purse crate links `dcroxide-stake` or `dcroxide-gcs` yet; the first that does takes `dcroxide-stake` with `default-features = false`.*

## Addendum, 2026-11-19 — Items the owner scheduled

Rule 3 has the project owner schedule the dcroxide items. On 2026-11-19
the owner decided, as recommended:

1. **`dcroxide-stake`'s storage gate** lands, and the pin reaches it,
   before Phase 3 starts. Rule 5's fallback (Phase 3 linking
   `dcroxide-database` while the gate is late) stays as written.
   *Landed, and the pin on it, on 2026-11-20; items 2 to 4 landed in
   the same range (the dated note in the addendum on redb 5 above).*
2. **The k256 precomputed tables** (ADR-0014 O1) are dcroxide commit
   `5ace7a2`; Phase 1's speed clause is measured again once the pin
   includes it (ADR-0014, addendum of 2026-11-19).
   *In the pin since 2026-11-20 and measured again the same day: the
   secret paths meet Phase 1's row; the clause stays open on base58
   decoding (ADR-0014, addendum of 2026-11-20).*
3. **Small additions found while porting are queued**
   in dcroxide: bounded draws `Uint32N` and `Int32N` among
   `dcroxide-crypto`'s global random functions (spec 24, `rand.Int32N`);
   an `is_null_data_script` that takes the script version (spec 24, the
   `IsNullDataScript` row); `KeyDb` and `ScriptDb` errors that carry the
   caller's own error; a guard in `sign_tx_output` against a
   pay-to-pubkey script whose key does not parse (QK-0745). Each is
   additive.
4. **dudect-style checks and a micro benchmark for dcroxide's secp256k1**
   are a follow-up in dcroxide.

Other decisions of the same day about dcroxide's own work are dcroxide's
to record.

## Addendum, 2026-11-20 — Decisions of the project owner: the address manager before the gate, and the change-position draw

On 2026-11-20 the project owner made two decisions about the schedule and
use of dcroxide items. The text above stays as written, apart from the
dated notes added to it.

1. **The address manager slice came before the gate.** Item 1 of the
   addendum "Items the owner scheduled" has `dcroxide-stake`'s storage
   gate land, and the pin reach it, before Phase 3 starts, and ADR-0003's
   addendum of 2026-11-19 (item 10) says the same. The owner accepted the
   address manager slice of Phase 3, `purse-udb` (commit `94afd6e`), ahead
   of the pin reaching the gate: an exception to that schedule. The slice
   links neither `dcroxide-stake` nor `dcroxide-gcs`, so it needed neither
   the gate nor rule 5's fallback. The pin move of the same day, to
   dcroxide `2074168`, puts the pin on the gate (the dated notes in rule 5
   and in the addendum on redb 5), so the part of Phase 3 that links them,
   `purse-udb`'s transaction store, starts with the gate in the pin.
2. **`purse-txauthor` draws through dcroxide's generator.** Now that
   `dcroxide_crypto::rand` has dcrd's bounded draws (`Int32N` and the
   rest, item 3 of that addendum, landed in dcroxide `69dfe8d`), the
   crate's injectable `RandomSource` and its own `Uint32N` are replaced by
   dcroxide's generator: process-wide for `randomize_output_position`,
   and a caller's `Prng` for the `_with` forms that tests seed. This was
   done in the same pin move (`docs/SKEW.md`, 2026-11-20 section;
   `docs/PARITY.md`, `purse-txauthor`), as rule 4 deletes a wallet-side
   copy in the first move that carries dcroxide's version. The oracle
   differentials and the frozen vectors are unchanged.
