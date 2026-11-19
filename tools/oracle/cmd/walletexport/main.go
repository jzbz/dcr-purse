// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

// Command walletexport writes the logical export of a dcrwallet wallet.db
// at schema version 26: the canonical, format-independent text form that
// docs/adr/0001-logical-export-format.md specifies (format "purse-export
// 2"). dcr-purse's own store shares no bytes with dcrwallet's database, so
// a Go wallet and a Rust wallet that reached their state by the same
// operations (operation scripts, ADR-0002) or by restoring the same seed
// against the same chain are compared through this text, which dcr-purse
// writes too (purse-udb, later).
//
// It speaks the linejson protocol (one JSON request per line on stdin, one
// response per line on stdout).
//
// Commands:
//
//	export   {path, out, network, pubpass?, privpass?, acctpass?, chain?,
//	         allow_unknown?, comments?}
//	         Opens path read-only with bbolt (no write lock, a 2 s lock
//	         timeout) and streams the export to a temporary file beside
//	         out, renamed to out once complete. network is mainnet,
//	         testnet3, simnet or regnet. With pubpass, udb.Open runs on the
//	         same read-only handle and the plaintext fields, [derived] and
//	         (with privpass, and acctpass {"<account>": passphrase} for
//	         accounts with their own passphrase) [privcheck] are filled.
//	         chain is "full" (default) or "digest". An entry no rule of the
//	         format consumes fails the export unless allow_unknown is set,
//	         when it goes to [unknown]. comments become "# " header lines.
//	         Returns {version, sha256, msha256, file_sha256, lines, sections:
//	         [{name, records, sha256, msha256}], counts: {section: records},
//	         metas: {name: value}}; sha256 and msha256 are the eof digests.
//	sections {}
//	         Returns {magic, version, sections: [{name, version, compared,
//	         types}], enums: {"type.field": [values]}, compared_metas},
//	         types being the "=type" declaration lines and compared false
//	         for the section the eof digests leave out.
//
// The exporter itself is tools/oracle/internal/walletexport, shared with
// cmd/oprunner; this command is its linejson front end.
//
// Where the data comes from. Every section but [derived] and [privcheck] is
// decoded from the raw buckets through bbolt cursors, because udb's row
// codecs are unexported and several stored fields have no read API; each
// decoder cites the Go that defines its encoding. Plaintext fields come
// from udb's public API on the same transaction: xpubs from
// Manager.Decrypt(CKTPublic, ...), address strings, hash160s and imported
// public keys from Manager.ForEachAccountAddress. [derived] is
// Manager.CoinType and AccountProperties, Store.MainChainTip,
// IsMissingMainChainCFilters, UnspentOutputCount, AccountBalances and
// UnspentTickets; [privcheck] is Manager.Unlock, UnlockAccount,
// Decrypt(CKTPrivate, ...) of each sealed coin-type key and of the parked
// SLIP0044 account 0 key, AccountExtendedPrivKey and PrivateKey.
package main

import (
	"encoding/json"
	"errors"

	"github.com/jzbz/dcr-purse/tools/oracle/internal/linejson"
	"github.com/jzbz/dcr-purse/tools/oracle/internal/walletexport"
)

func main() {
	linejson.Serve(map[string]linejson.Handler{
		"export":   cmdExport,
		"sections": cmdSections,
	})
}

func unmarshal(args json.RawMessage, v any) error {
	if len(args) == 0 {
		return errors.New("missing args")
	}
	return json.Unmarshal(args, v)
}

func cmdExport(raw json.RawMessage) (any, error) {
	var args walletexport.ExportArgs
	if err := unmarshal(raw, &args); err != nil {
		return nil, err
	}
	return walletexport.Export(args)
}

func cmdSections(json.RawMessage) (any, error) {
	return walletexport.Sections(), nil
}
