// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

// Package deps keeps every module an oracle binary may link in go.mod and
// go.sum, so that adding a binary under cmd/ never needs `go mod tidy`.
package deps

import (
	_ "decred.org/dcrwallet/v5/errors"
	_ "decred.org/dcrwallet/v5/pgpwordlist"
	_ "decred.org/dcrwallet/v5/wallet"
	_ "decred.org/dcrwallet/v5/wallet/udb"
	_ "decred.org/dcrwallet/v5/walletseed"
	_ "github.com/decred/dcrd/hdkeychain/v3"
	_ "go.etcd.io/bbolt"
)
