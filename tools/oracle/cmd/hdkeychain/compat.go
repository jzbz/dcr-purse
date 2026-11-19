// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

package main

import (
	"github.com/decred/dcrd/hdkeychain/v3"
	"github.com/decred/dcrd/txscript/v4/stdaddr"
)

// HD2Address is a verbatim copy of dcrwallet's internal/compat.HD2Address,
// which wallet/addresses.go's deriveChildAddress calls to make every BIP0044
// address. The package is internal, so Go will not let this module import it;
// compat_test.go fails unless the function below is byte-identical to the one
// in the dcrwallet module version go.mod pins. (cmd/oprunner carries a copy of
// the whole package under its own internal/, which no other command may
// import.)

func HD2Address(k *hdkeychain.ExtendedKey, params stdaddr.AddressParams) (*stdaddr.AddressPubKeyHashEcdsaSecp256k1V0, error) {
	pk := k.SerializedPubKey()
	hash := stdaddr.Hash160(pk)
	return stdaddr.NewAddressPubKeyHashEcdsaSecp256k1V0(hash, params)
}
