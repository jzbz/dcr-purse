// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

package main

import (
	"context"
	"path/filepath"
	"testing"

	"decred.org/dcrwallet/v5/errors"
	"decred.org/dcrwallet/v5/wallet/walletdb"
	"github.com/decred/dcrd/chaincfg/v3"
	"github.com/jzbz/dcr-purse/tools/oracle/cmd/oprunner/internal/rand"
	"github.com/jzbz/dcr-purse/tools/oracle/cmd/oprunner/internal/udb"
)

// purse_udb::initialize and initialize_watch_only (crates/purse-udb/src/
// initialize.rs) reproduce two answers of udb's Initialize that no script
// reaches, since the runner's setup always starts from an empty database:
// an Initialize that fails commits nothing, so a later one on the same
// database succeeds; and an Initialize or InitializeWatchOnly of a
// database Initialize created fails first, before its arguments are looked
// at, with CreateTopLevelBucket's error, kind IO wrapping bbolt's Exist
// error. This test pins both in the copied udb the runner runs.
func TestInitializeOfACreatedDatabase(t *testing.T) {
	rand.Seed([32]byte{7})
	ctx := context.Background()
	params := chaincfg.SimNetParams()
	db, err := walletdb.Create("bdb", filepath.Join(t.TempDir(), "wallet.db"))
	if err != nil {
		t.Fatal(err)
	}
	defer db.Close()
	seed := make([]byte, 32)
	err = udb.Initialize(ctx, db, params, seed, []byte("public"), nil)
	if err == nil || err.Error() != "invalid operation: private passphrase may not be empty" {
		t.Fatalf("an empty private passphrase gave %v", err)
	}
	if err := udb.Initialize(ctx, db, params, seed, []byte("public"), []byte("private")); err != nil {
		t.Fatalf("Initialize after a failed one: %v", err)
	}
	const want = "I/O error:: item already exists: bucket already exists"
	for name, err := range map[string]error{
		"Initialize":          udb.Initialize(ctx, db, params, seed[:3], []byte("public"), nil),
		"InitializeWatchOnly": udb.InitializeWatchOnly(ctx, db, params, "xyz", []byte("public")),
	} {
		if err == nil || err.Error() != want || !errors.Is(err, errors.IO) {
			t.Errorf("%s of a created database gave %v, want %q (kind IO)", name, err, want)
		}
		if v := errorValue(err); v["kind"] != "IO" {
			t.Errorf("%s: encoded as %v", name, v)
		}
	}
}
