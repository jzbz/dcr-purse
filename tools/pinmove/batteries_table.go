// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

package main

// RepoTable is dcr-purse's table of frozen files (every file under
// crates/*/tests/data) and of the oracle's copies of upstream source.
//
// TestRepoTable holds it to the repository: every file under
// crates/*/tests/data must match exactly one battery, every pattern must
// match a file, every command must exist under tools/oracle/cmd, every
// command a file's header names must be among its battery's commands,
// and every copy must exist.  A new frozen file therefore fails
// tools/pinmove's tests until it is entered here.
//
// What the table says by hand is what nothing in the repository states in
// a form a program can read: which regenerator writes a file, which oracle
// commands it runs, which upstream files a copy or a fixture comes from,
// and which frozen test requires the file to name DCRWALLET_PARITY_COMMIT.
// Which upstream packages each command links is not in the table: the
// batteries command asks `go list -deps` (ADR-0016, Decision item 5.1).
var RepoTable = Table{
	Batteries: []Battery{
		{
			Crate: "purse-errors", Files: []string{"errors-vectors.jsonl"},
			Kind: KindGoGenerated, Commands: []string{"errors"},
			Regenerate: "PURSE_REQUIRE_ORACLE=1 cargo test -p purse-errors --test oracle -- --ignored regenerate_frozen_vectors",
			PinCheck:   "crates/purse-errors/tests/oracle.rs `frozen_vectors`",
		},
		{
			Crate: "purse-seed", Files: []string{"seed_vectors.json"},
			Kind: KindGoGenerated, Commands: []string{"seed"},
			Regenerate: "cargo test -p purse-seed --test oracle regenerate_frozen_vectors -- --ignored",
			PinCheck:   "crates/purse-seed/tests/frozen.rs",
		},
		{
			Crate: "purse-snacl", Files: []string{"snacl_vectors.txt", "fixture_vectors.txt", "generated_db.txt"},
			Kind: KindGoGenerated, Commands: []string{"snacl"},
			Inputs:     []string{"wallet/udb/testdata/*.db.gz"},
			Regenerate: "PURSE_SNACL_REGEN=1 PURSE_REQUIRE_ORACLE=1 cargo test -p purse-snacl --test oracle -- --ignored regenerate_frozen_vectors",
			PinCheck:   "crates/purse-snacl/tests/frozen.rs `header_names_the_pin`",
			Note:       "snacl_vectors.txt and generated_db.txt do not regenerate byte for byte (Go draws salts, nonces and keys from the operating system's generator); fixture_vectors.txt does, and its inputs are dcrwallet's fixture databases",
		},
		{
			Crate: "purse-snacl", Files: []string{"generated-simnet.db.gz"},
			Kind: KindGoGenerated, Commands: []string{"snacl"},
			Regenerate: "PURSE_SNACL_REGEN=1 PURSE_REQUIRE_ORACLE=1 cargo test -p purse-snacl --test oracle -- --ignored regenerate_frozen_vectors",
			Note:       "binary; written by the same regenerator as generated_db.txt, which names the pin for it; not reproducible byte for byte",
		},
		{
			Crate: "purse-mixsolver", Files: []string{"flint_vectors.jsonl"},
			Kind: KindGoGenerated, Commands: []string{"mixsolver"}, Tags: "flint",
			Regenerate: "PURSE_MIXSOLVER_REGEN=1 cargo test -p purse-mixsolver --test oracle -- --ignored regenerate_flint_vectors (the oracle built with -tags flint)",
			PinCheck:   "crates/purse-mixsolver/tests/frozen.rs",
		},
		{
			Crate: "purse-mixsolver", Files: []string{"self_vectors.txt"},
			Kind:       KindSelf,
			Regenerate: "PURSE_MIXSOLVER_REGEN=1 cargo test -p purse-mixsolver --lib -- --ignored regenerate_self_vectors",
			Note:       "computed by the crate, not by Go; never moves with the pin",
		},
		{
			Crate: "purse-hdkeychain",
			Files: []string{"leading_zero_digests.txt", "leading_zero_lines.txt", "parse_corpus_digests.txt",
				"parse_corpus_lines.txt", "random_seed_digests.txt"},
			Kind: KindGoGenerated, Commands: []string{"hdkeychain"},
			Regenerate: "PURSE_HD_REGEN=1 cargo test --release -p purse-hdkeychain --test frozen -- --ignored regenerate_frozen_vectors",
			Note:       "the header names \"dcrwallet 54b85b336077\" in prose; tests/frozen.rs checks the entry count and body digest only",
		},
		{
			Crate: "purse-dcrutil", Files: []string{"amount_vectors.txt", "txsort_vectors.txt", "wif_vectors.txt"},
			Kind: KindGoGenerated, Commands: []string{"dcrutil"},
			Regenerate: "PURSE_DCRUTIL_REGEN=1 cargo test -p purse-dcrutil --test oracle -- --ignored regenerate_frozen_vectors",
			Note:       "the header names \"dcrwallet 54b85b336077\" in prose; tests/frozen.rs checks counts and answers only",
		},
		{
			Crate: "purse-dcrutil", Files: []string{"txsort/*.hex"},
			Kind: KindModuleCopy, Modules: []string{"github.com/decred/dcrd/dcrutil/v4"},
			Regenerate: "copy txsort/testdata of github.com/decred/dcrd/dcrutil/v4 at the version dcrwallet requires (txsort/README.md)",
		},
		{
			Crate: "purse-dcrutil", Files: []string{"txsort/README.md"}, Kind: KindDoc,
		},
		{
			Crate: "purse-export-check", Files: []string{"full-simnet.pe"},
			Kind: KindGoGenerated, Commands: []string{"walletgen", "walletexport"},
			Regenerate: "PURSE_REGEN_EXPORT=1 cargo test -p purse-export-check --test oracle regenerate_frozen_export",
			Note:       "no commit in its header; volatile values differ on every regeneration, and no test defines the mask a header-only comparison needs",
		},
		{
			Crate: "purse-export-check", Files: []string{"opvectors/*.opv", "opvectors/*.pe"},
			Kind: KindGoGenerated, Commands: []string{"oprunner"},
			Regenerate: "PURSE_REGEN_OPVECTORS=1 cargo test -p purse-export-check --test opscripts frozen_vectors_match_go",
			Note:       "the .opv header says \"at the parity pin\" and names no commit; the .pe files name none",
		},
		{
			Crate: "purse-export-check", Files: []string{"opscripts/*.json"},
			Kind: KindSelf,
			Note: "operation scripts written by hand: the inputs of the opvectors, not outputs of Go",
		},
		{
			Crate: "purse-boltread", Files: []string{"upstream/*.db.gz"},
			Kind: KindUpstreamCopy, Inputs: []string{"wallet/udb/testdata/*.db.gz"},
			Regenerate: "copy wallet/udb/testdata/*.db.gz of dcrwallet at the new pin byte for byte, update upstream/README.md, then regenerate dump-digests.txt and bucket-stats.json",
			Note:       "the commit is named only in upstream/README.md",
		},
		{
			Crate: "purse-boltread", Files: []string{"README.md", "upstream/README.md"}, Kind: KindDoc,
		},
		{
			Crate: "purse-boltread", Files: []string{"frozen-4k.db", "frozen-deep-4k.db", "dump-digests.txt", "bucket-stats.json"},
			Kind: KindGoGenerated, Commands: []string{"boltdump"},
			Inputs:     []string{"wallet/udb/testdata/*.db.gz"},
			Regenerate: "by hand through tools/oracle/cmd/boltdump's dump and stats commands (tests/data/README.md)",
			Note:       "bbolt v1.3.12 only; the digests and statistics also cover the upstream/ copies, so they move when those do",
		},
		{
			Crate: "purse-store", Files: []string{"cursor_vectors.jsonl"},
			Kind: KindGoGenerated, Commands: []string{"boltcursor"},
			Regenerate: "PURSE_REQUIRE_ORACLE=1 cargo test -p purse-store --test conformance -- --ignored regenerate_frozen_vectors",
			PinCheck:   "crates/purse-store/tests/conformance.rs `frozen_vectors`",
		},
		{
			Crate: "purse-store", Files: []string{"schema-v1.redb"},
			Kind: KindSelf,
			Note: "a released schema fixture written by purse-store; never regenerated (tests/migration.rs pins its SHA-256)",
		},
		{
			Crate: "purse-txauthor", Files: []string{"txauthor_vectors.txt"},
			Kind: KindGoGenerated, Commands: []string{"txauthor"},
			Regenerate: "PURSE_TXAUTHOR_REGEN=1 cargo test -p purse-txauthor --test oracle -- --ignored regenerate_frozen_vectors",
			Note:       "the header names \"54b85b336077\" in prose; tests/frozen.rs checks counts and answers only",
		},
		{
			Crate: "purse-udb", Files: []string{"addrmgr_opvectors.txt"},
			Kind: KindGoGenerated, Commands: []string{"oprunner"},
			Regenerate: "PURSE_UDB_REGEN=1 PURSE_REQUIRE_ORACLE=1 cargo test -p purse-udb --test frozen -- --ignored regenerate_frozen_vectors",
			PinCheck:   "crates/purse-udb/tests/frozen.rs `vectors_are_the_corpus_and_name_the_pin`",
		},
		{
			Crate: "purse-udb", Files: []string{"opvectors/*.pe"},
			Kind: KindGoGenerated, Commands: []string{"oprunner"},
			Regenerate: "PURSE_UDB_REGEN=1 PURSE_REQUIRE_ORACLE=1 cargo test -p purse-udb --test frozen -- --ignored regenerate_frozen_vectors",
			Note:       "Go's final exports of the hand-written scripts, written by the same regenerator as addrmgr_opvectors.txt, which names the pin for them and pins their digests; they name no commit",
		},
		{
			Crate: "purse-udb", Files: []string{"addrmgr_genvectors.txt"},
			Kind: KindGoGenerated, Commands: []string{"oprunner"},
			Regenerate: "PURSE_UDB_REGEN=1 PURSE_REQUIRE_ORACLE=1 cargo test -p purse-udb --test frozen -- --ignored regenerate_generated_vectors",
			PinCheck:   "crates/purse-udb/tests/frozen.rs `generated_vectors_are_the_seeded_set`",
		},
		{
			Crate: "purse-udb", Files: []string{"addrmgr_wovectors.txt"},
			Kind: KindGoGenerated, Commands: []string{"oprunner"},
			Regenerate: "PURSE_UDB_REGEN=1 PURSE_REQUIRE_ORACLE=1 cargo test -p purse-udb --test frozen -- --ignored regenerate_watching_only_vectors",
			Note:       "the header names the full dcrwallet commit; tests/frozen.rs checks the scripts, the entry count and the body digest, not the commit",
		},
		{
			Crate: "purse-udb", Files: []string{"addrmgr_passvectors.txt"},
			Kind: KindGoGenerated, Commands: []string{"oprunner"},
			Regenerate: "PURSE_UDB_REGEN=1 PURSE_REQUIRE_ORACLE=1 cargo test -p purse-udb --test frozen -- --ignored regenerate_passphrase_vectors",
			Note:       "the header names the full dcrwallet commit; tests/frozen.rs checks the scripts, the entry count and the body digest, not the commit",
		},
		{
			Crate: "purse-udb", Files: []string{"addrmgr_changevectors.txt"},
			Kind: KindGoGenerated, Commands: []string{"oprunner"},
			Regenerate: "PURSE_UDB_REGEN=1 PURSE_REQUIRE_ORACLE=1 cargo test -p purse-udb --test frozen -- --ignored regenerate_change_vectors",
			Note:       "the header names the full dcrwallet commit; tests/frozen.rs checks the scripts, the entry count and the body digest, not the commit",
		},
		{
			Crate: "purse-udb", Files: []string{"txstore_genvectors.txt"},
			Kind: KindGoGenerated, Commands: []string{"oprunner"},
			Regenerate: "PURSE_UDB_REGEN=1 PURSE_REQUIRE_ORACLE=1 cargo test -p purse-udb --test frozen -- --ignored regenerate_txstore_vectors",
			Note:       "the transaction store's generated scripts; the header names the full dcrwallet commit; tests/frozen.rs checks the scripts, the entry count and the body digest, not the commit",
		},
		{
			Crate: "purse-udb", Files: []string{"txstore_credit_genvectors.txt"},
			Kind: KindGoGenerated, Commands: []string{"oprunner"},
			Regenerate: "PURSE_UDB_REGEN=1 PURSE_REQUIRE_ORACLE=1 cargo test -p purse-udb --test frozen -- --ignored regenerate_txstore_credit_vectors",
			Note:       "the transaction store's generated scripts that draw the credit slice's operations; the header names the full dcrwallet commit; tests/frozen.rs checks the scripts, the entry count and the body digest, not the commit",
		},
		{
			Crate: "purse-udb", Files: []string{"txstore_cms_genvectors.txt"},
			Kind: KindGoGenerated, Commands: []string{"oprunner"},
			Regenerate: "PURSE_UDB_REGEN=1 PURSE_REQUIRE_ORACLE=1 cargo test -p purse-udb --test frozen -- --ignored regenerate_txstore_cms_vectors",
			Note:       "the transaction store's generated scripts that draw the commitment and multisig slice's operations; the header names the full dcrwallet commit; tests/frozen.rs checks the scripts, the entry count and the body digest, not the commit",
		},
		{
			Crate: "purse-udb", Files: []string{"txstore_select_genvectors.txt"},
			Kind: KindGoGenerated, Commands: []string{"oprunner"},
			Regenerate: "PURSE_UDB_REGEN=1 PURSE_REQUIRE_ORACLE=1 cargo test -p purse-udb --test frozen -- --ignored regenerate_txstore_select_vectors",
			Note:       "the transaction store's generated scripts that draw input selection and Spender, their random selections keyed per operation (ADR-0002 rand); the header names the full dcrwallet commit; tests/frozen.rs checks the scripts, the entry count and the body digest, not the commit",
		},
		{
			Crate: "purse-udb", Files: []string{"opscripts/*.json"},
			Kind: KindSelf,
			Note: "operation scripts written by hand: the inputs of addrmgr_opvectors.txt, not outputs of Go",
		},
		{
			Crate: "purse-udb", Files: []string{"wallet_lines.txt"},
			Kind: KindGoGenerated, Commands: []string{"hdkeychain"},
			Regenerate: "PURSE_HD_REGEN=1 PURSE_REQUIRE_ORACLE=1 cargo test --release -p purse-udb --test wallet_diff -- --ignored regenerate_wallet_vectors",
			PinCheck:   "crates/purse-udb/tests/wallet_diff.rs `frozen_wallet_lines` (the \"# dcrwallet commit:\" header field)",
			Note:       "moved from purse-hdkeychain on 2026-11-22 with the seed differential, body unchanged",
		},
		{
			Crate: "purse-txrules", Files: []string{"txrules_vectors.txt"},
			Kind: KindGoGenerated, Commands: []string{"txrules"},
			Regenerate: "PURSE_TXRULES_REGEN=1 cargo test -p purse-txrules --test oracle -- --ignored regenerate_frozen_vectors",
			Note:       "the header names \"54b85b336077\" in prose; tests/frozen.rs checks counts and answers only",
		},
		{
			Crate: "sntrup4591761", Files: []string{"sntrup_vectors.txt"},
			Kind: KindGoGenerated, Commands: []string{"sntrup"},
			Regenerate: "PURSE_SNTRUP_REGEN=1 PURSE_REQUIRE_ORACLE=1 cargo test --release -p sntrup4591761 --test oracle -- --ignored --exact regenerate_frozen_vectors",
			PinCheck:   "crates/sntrup4591761/tests/frozen.rs `replay_frozen_vectors`",
		},
		{
			Crate: "sntrup4591761", Files: []string{"sage128.gz"},
			Kind: KindModuleCopy, Modules: []string{"github.com/companyzero/sntrup4591761"},
			Regenerate: "copy testdata/sage128.gz of github.com/companyzero/sntrup4591761 at the version dcrwallet requires",
		},
		{
			Crate: "sntrup4591761", Files: []string{"README.md"}, Kind: KindDoc,
		},
	},
	Copies: []Copy{
		{
			Oracle: "dcrwallet.go.mod", Upstream: []string{"go.mod"},
			Regenerate: "copy dcrwallet's go.mod at the new pin verbatim; go test ./internal/deps (it holds the copy's hash to go.sum)",
		},
		{
			Oracle: "cmd/oprunner/internal/udb", Package: "cmd/oprunner/internal/udb", Upstream: []string{"wallet/udb/"},
			Regenerate: "PURSE_REGEN_COPIES=1 go test ./cmd/oprunner -run TestCopiesAreVerbatim, then repeat the audit in copies_test.go's header",
		},
		{
			Oracle: "cmd/oprunner/internal/snacl", Package: "cmd/oprunner/internal/snacl", Upstream: []string{"wallet/internal/snacl/snacl.go"},
			Regenerate: "PURSE_REGEN_COPIES=1 go test ./cmd/oprunner -run TestCopiesAreVerbatim",
		},
		{
			Oracle: "cmd/oprunner/internal/compat", Package: "cmd/oprunner/internal/compat", Upstream: []string{"internal/compat/compat.go"},
			Regenerate: "PURSE_REGEN_COPIES=1 go test ./cmd/oprunner -run TestCopiesAreVerbatim",
		},
		{
			Oracle: "cmd/oprunner/internal/kdf", Package: "cmd/oprunner/internal/kdf", Upstream: []string{"kdf/kdf.go"},
			Regenerate: "PURSE_REGEN_COPIES=1 go test ./cmd/oprunner -run TestCopiesAreVerbatim",
		},
		{
			Oracle: "cmd/oprunner/internal/rand", Package: "cmd/oprunner/internal/rand", Module: "github.com/decred/dcrd/crypto/rand",
			Upstream:   []string{"default.go", "doc.go", "uniform.go"},
			Regenerate: "PURSE_REGEN_COPIES=1 go test ./cmd/oprunner -run TestCopiesAreVerbatim",
		},
		{
			Oracle: "cmd/txauthor/internal/txauthor", Package: "cmd/txauthor/internal/txauthor", Upstream: []string{"wallet/txauthor/author.go"},
			Regenerate: "PURSE_REGEN_COPIES=1 go test ./cmd/txauthor -run TestCopiesAreVerbatim",
		},
		{
			Oracle: "cmd/txauthor/internal/rand", Package: "cmd/txauthor/internal/rand", Module: "github.com/decred/dcrd/crypto/rand",
			Upstream:   []string{"default.go", "doc.go", "uniform.go"},
			Regenerate: "PURSE_REGEN_COPIES=1 go test ./cmd/txauthor -run TestCopiesAreVerbatim",
		},
		{
			Oracle: "internal/snaclcopy/snacl.go", Package: "internal/snaclcopy", Upstream: []string{"wallet/internal/snacl/snacl.go"},
			Regenerate: "replace snacl.go with the upstream file, changing only its package clause; go test ./internal/snaclcopy",
		},
		{
			Oracle: "cmd/snacl/sealcopy.go", Package: "cmd/snacl", Upstream: []string{"wallet/udb/addressmanager.go"},
			Regenerate: "replace the block below the marker line by hand; go test ./cmd/snacl -run TestSealCopyIsVerbatim",
		},
		{
			Oracle: "cmd/hdkeychain/compat.go", Package: "cmd/hdkeychain", Upstream: []string{"internal/compat/compat.go"},
			Regenerate: "replace HD2Address by hand; go test ./cmd/hdkeychain -run TestHD2AddressIsVerbatim",
		},
	},
}
