# Architecture decision records

One record per decision the project owner makes or has made. Each states the
question, the options, one proposed outcome and what follows from it. A
record is **Proposed** until the owner decides; **Accepted** names who decided
and when. Dated records are extended with addenda, not rewritten.

The decisions D1 to D15 are listed in the
[project brief](../dcr-purse-project-brief.md), section 9.

| ADR | Decision | Subject | Status |
|---|---|---|---|
| [0001](0001-logical-export-format.md) | — | The logical export: the format-independent text form of a wallet | Proposed; format 2, revised for seed restore only (ADR-0013) and after review |
| [0002](0002-operation-script-format.md) | — | The operation-script format the Go and Rust runners share | Proposed; revised after review |
| [0003](0003-wallet-store-engine-and-schema.md) | D2 | Wallet store engine and schema | Engine accepted: redb 5 (project owner, 2026-11-18); schema and open points Proposed, revised after review |
| [0004](0004-grpc-transport.md) | D5 | gRPC transport | Proposed; revised after review, and the recommendation changed |
| [0005](0005-name-data-directory-and-import-front-end.md) | D1 | Name, data directory and the import front end | Accepted (project owner, 2026-11-18), with the dcrd certificate fallback also trying dcroxide's directory; its import front end voided by ADR-0013 (2026-11-18), the rest (name, data directory, file names, version string, certificate fallback) standing; running as `dcrwallet` for Decrediton waits for a Phase 8 addendum |
| [0006](0006-consuming-dcroxide-crates.md) | D3 | How dcroxide crates are consumed | Accepted (project owner, 2026-11-18), with dcr-purse kept on the latest stable Rust |
| [0007](0007-importable-dcrwallet-databases.md) | D4 | Which dcrwallet databases can be imported, and what an import carries | Superseded by ADR-0013 (2026-11-18); kept as the record |
| [0008](0008-concurrency-model.md) | D6 | Concurrency model | Accepted (project owner, 2026-11-19): OS threads everywhere, `crossbeam-channel`, and a runtime allowed only inside the gRPC listener if ADR-0004's spike calls for it; addendum (ADR-0014) permits the speed departures from items 2 and 3 |
| [0009](0009-version-skew-against-dcroxide.md) | D7 | Version skew between dcrwallet's dependencies and dcroxide | Accepted (project owner, 2026-11-18) |
| [0010](0010-mixing-root-solver.md) | D8 | The mixing root solver | Accepted (project owner, 2026-11-18) |
| [0011](0011-error-message-fidelity.md) | D9 | Error-message fidelity | Accepted (project owner, 2026-11-18) |
| [0012](0012-standing-quirk-policy.md) | D10 | Standing quirk policy | Accepted (project owner, 2026-11-18), with QK-0279 fixed instead of reproduced |
| [0013](0013-no-import-seed-restore-only.md) | D1 (part), D4 | No database import: migration by seed restore only | Accepted (project owner, 2026-11-18); of its open points, the one list of what dropping the import leaves open, 1 to 5 and 7 are decided (D14's reframing among them) and 6, 8 and 9 are Proposed |
| [0014](0014-performance.md) | — | Performance as a goal | Goal and its precedence over blending in accepted (project owner, 2026-11-19); open points 1 to 12 all decided as recommended; its numbers are starting values, fixed from measurements |

Not yet drafted: D11 (notification delivery), D12 (upstream tracking), D13
(upstream defects worth reporting), D14 (the way back to dcrwallet; its
premise, a one-way import, is gone, and its reframing as moving non-seed
state by hand was decided in ADR-0013 Open point 4), D15 (backup and
recovery of the store). D4 (importable databases) fell away with ADR-0013,
which supersedes ADR-0007.
