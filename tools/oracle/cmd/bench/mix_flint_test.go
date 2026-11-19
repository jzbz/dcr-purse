// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//go:build flint

package main

import (
	"testing"

	"decred.org/cspp/v2/solver"
	"github.com/decred/dcrd/mixing"
)

// TestFlintSolverRoots runs the reference solver on the workloads'
// instances (the smoke and full degrees) and holds its output to the
// instance's roots with the check the workloads apply before timing.
func TestFlintSolverRoots(t *testing.T) {
	for _, d := range []int{4, 8, 20, 32, 64} {
		coeffs, want := mixInstance(d)
		got, err := solver.Roots(coeffs, mixing.F)
		if err != nil {
			t.Fatalf("degree %d: %v", d, err)
		}
		if err := checkRoots(got, want); err != nil {
			t.Fatalf("degree %d: %v", d, err)
		}
	}
}
