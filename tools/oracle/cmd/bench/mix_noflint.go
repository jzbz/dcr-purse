// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//go:build !flint

package main

import "math/big"

// mixSolve reports the solver workloads as skipped: the reference solver is
// cgo over FLINT, built only with the tag "flint" (see cmd/mixsolver).
func mixSolve(name string, _, _ []*big.Int) {
	skip(name, "built without the flint tag")
}
