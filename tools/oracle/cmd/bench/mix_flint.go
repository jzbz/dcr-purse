// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//go:build flint

package main

import (
	"math/big"

	"decred.org/cspp/v2/solver"
	"github.com/decred/dcrd/mixing"
)

// mixSolve times solver.Roots in process, the cgo call into FLINT that
// dcrwallet's csppsolver program makes (dcrwallet reaches it through
// solverrpc and a child process, whose round trip is not timed here).
func mixSolve(name string, coeffs, want []*big.Int) {
	got, err := solver.Roots(coeffs, mixing.F)
	must(err)
	if err := checkRoots(got, want); err != nil {
		panic(name + ": " + err.Error())
	}
	benchOnce(name, func() uint64 {
		r, err := solver.Roots(coeffs, mixing.F)
		must(err)
		return uint64(len(r))
	})
}
