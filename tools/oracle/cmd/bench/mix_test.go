// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

package main

import (
	"math/big"
	"testing"
)

// evalMod evaluates the polynomial (constant term first) at x mod fieldP.
func evalMod(coeffs []*big.Int, x *big.Int) *big.Int {
	acc := new(big.Int)
	for i := len(coeffs) - 1; i >= 0; i-- {
		acc.Mul(acc, x)
		acc.Add(acc, coeffs[i])
		acc.Mod(acc, fieldP)
	}
	return acc
}

// TestMixInstanceRoots holds the instance generator to its contract: the
// leading coefficient is F-1, the roots are distinct, ascending and below
// F, and each is a root of the polynomial.
func TestMixInstanceRoots(t *testing.T) {
	for _, d := range []int{1, 2, 5, 20, 64} {
		coeffs, roots := mixInstance(d)
		if len(coeffs) != d+1 || len(roots) != d {
			t.Fatalf("degree %d: %d coefficients, %d roots", d, len(coeffs), len(roots))
		}
		lead := new(big.Int).Sub(fieldP, big.NewInt(1))
		if coeffs[d].Cmp(lead) != 0 {
			t.Fatalf("degree %d: leading coefficient %v", d, coeffs[d])
		}
		for i, r := range roots {
			if r.Cmp(fieldP) >= 0 || (i > 0 && roots[i-1].Cmp(r) >= 0) {
				t.Fatalf("degree %d: root %d out of order or range", d, i)
			}
			if evalMod(coeffs, r).Sign() != 0 {
				t.Fatalf("degree %d: root %d is not a root", d, i)
			}
		}
	}
}

// TestCheckRoots holds the check the solver workloads apply to the
// solver's output: any order is accepted, and a wrong count, a wrong value
// with the right count, a repeated value or a nil is refused. The last two
// cases are what a count-only check let through.
func TestCheckRoots(t *testing.T) {
	_, want := mixInstance(8)
	reversed := make([]*big.Int, len(want))
	for i, r := range want {
		reversed[len(want)-1-i] = r
	}
	if err := checkRoots(reversed, want); err != nil {
		t.Fatalf("the right roots in another order: %v", err)
	}
	if err := checkRoots(want[:7], want); err == nil {
		t.Fatal("a missing root was accepted")
	}
	wrong := append([]*big.Int(nil), want...)
	wrong[3] = new(big.Int).Add(want[3], big.NewInt(1))
	if err := checkRoots(wrong, want); err == nil {
		t.Fatal("a wrong root with the right count was accepted")
	}
	repeated := append([]*big.Int(nil), want...)
	repeated[0] = want[1]
	if err := checkRoots(repeated, want); err == nil {
		t.Fatal("a repeated root with the right count was accepted")
	}
	withNil := append([]*big.Int(nil), want...)
	withNil[5] = nil
	if err := checkRoots(withNil, want); err == nil {
		t.Fatal("a nil root was accepted")
	}
}
