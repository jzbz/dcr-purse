# Architecture decision records

One record per decision the project owner makes or has made. Each states the
question, the options, one proposed outcome and what follows from it. A
record is **Proposed** until the owner decides; **Accepted** names who decided
and when. Dated records are extended with addenda, not rewritten.

A record's citations of dcrwallet source (`path:line`) refer to the parity
pin in force on the record's date, and an addendum's to the pin in force on
the addendum's date. Every record and addendum to date was written at
dcrwallet `54b85b33`, the only pin there has been; ten records name no
commit and rely on this paragraph.

The decisions D1 to D15 are listed in the
[project brief](../dcr-purse-project-brief.md), section 9.

| ADR | Decision | Subject | Status |
|---|---|---|---|
| [0001](0001-logical-export-format.md) | — | The logical export: the format-independent text form of a wallet | Accepted (project owner, 2026-11-19): format 2, revised for seed restore only (ADR-0013) and after review |
| [0002](0002-operation-script-format.md) | — | The operation-script format the Go and Rust runners share | Accepted (project owner, 2026-11-19), with Go's in-memory state after a rolled-back transaction not reproduced (QK-0733) |
| [0003](0003-wallet-store-engine-and-schema.md) | D2 | Wallet store engine and schema | Accepted: engine redb 5 (project owner, 2026-11-18); schema and its nine open points as recommended (project owner, 2026-11-19) |
| [0004](0004-grpc-transport.md) | D5 | gRPC transport | Accepted (project owner, 2026-11-19), against the proposal: tonic and prost, with tokio confined to the listener; the other questions as recommended |
| [0005](0005-name-data-directory-and-import-front-end.md) | D1 | Name, data directory and the import front end | Accepted (project owner, 2026-11-18), with the dcrd certificate fallback also trying dcroxide's directory; its import front end voided by ADR-0013 (2026-11-18), the rest (name, data directory, file names, version string, certificate fallback) standing; running as `dcrwallet` for Decrediton waits for a Phase 8 addendum |
| [0006](0006-consuming-dcroxide-crates.md) | D3 | How dcroxide crates are consumed | Accepted (project owner, 2026-11-18), with dcr-purse kept on the latest stable Rust |
| [0007](0007-importable-dcrwallet-databases.md) | D4 | Which dcrwallet databases can be imported, and what an import carries | Superseded by ADR-0013 (2026-11-18); kept as the record |
| [0008](0008-concurrency-model.md) | D6 | Concurrency model | Accepted (project owner, 2026-11-19): OS threads everywhere, `crossbeam-channel`, and a runtime allowed only inside the gRPC listener, which ADR-0004 now uses for tonic; addendum (ADR-0014) permits the speed departures from items 2 and 3 |
| [0009](0009-version-skew-against-dcroxide.md) | D7 | Version skew between dcrwallet's dependencies and dcroxide | Accepted (project owner, 2026-11-18) |
| [0010](0010-mixing-root-solver.md) | D8 | The mixing root solver | Accepted (project owner, 2026-11-18) |
| [0011](0011-error-message-fidelity.md) | D9 | Error-message fidelity | Accepted (project owner, 2026-11-18) |
| [0012](0012-standing-quirk-policy.md) | D10 | Standing quirk policy | Accepted (project owner, 2026-11-18), with QK-0279 fixed instead of reproduced |
| [0013](0013-no-import-seed-restore-only.md) | D1 (part), D4 | No database import: migration by seed restore only | Accepted (project owner, 2026-11-18); of its open points, the one list of what dropping the import leaves open, 1 to 5 and 7 are decided (D14's reframing among them) and 6, 8 and 9 are Proposed |
| [0014](0014-performance.md) | — | Performance as a goal | Goal and its precedence over blending in accepted (project owner, 2026-11-19); open points 1 to 12 all decided as recommended; its numbers are starting values, fixed from measurements |
| [0015](0015-notification-delivery.md) | D11 | Notification delivery to gRPC stream subscribers (JSON-RPC clients receive none) | Proposed: generation as Go's, a byte-bounded queue per client within a budget for all streams, overflow ending the stream of the client furthest behind; the sync streams keep Go's back-pressure; an addendum to ADR-0004 if accepted |
| [0016](0016-upstream-tracking.md) | D12 | Upstream tracking: when and how the dcrwallet pin and the pins that follow it move | Proposed: master at upstream milestones and on the owner's word until v2.2.0, then every release tag of the newest line; one signed change per move, from a review list a `tools/pinmove` command produces, with shifted citations remapped mechanically |
| [0017](0017-upstream-defects.md) | D13 | Upstream defects worth reporting | Proposed: 18 security-relevant candidates (five confirmed by running Go at the pin) reported privately through the Decred bug bounty by the owner after a run on current master (which amends ADR-0012 Open point 6, for the owner to accept), the rest as public issues once confirmed; nothing reported by the record |
| [0018](0018-backup-and-recovery.md) | D15 | Backup and recovery of the store | Proposed: a snapshot of the running store copied into a new, compacted store file (`backupwallet`), a separate tool to verify and restore, no added encryption; an addendum to ADR-0003 if accepted |

Not yet drafted: D14 (the way back to dcrwallet; its premise, a one-way
import, is gone, and its reframing as moving non-seed state by hand was
decided in ADR-0013 Open point 4). D4 (importable databases) fell away with ADR-0013,
which supersedes ADR-0007.
