// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

package main

import (
	"fmt"
	"math/big"
	"sort"
)

// fieldP is the mixing field's modulus, 2^127 - 1 (mixing.F).
var fieldP = new(big.Int).Sub(new(big.Int).Lsh(big.NewInt(1), 127), big.NewInt(1))

// mixDegrees are the polynomial degrees (peers' messages) the solver
// workloads use: a small mix, a 64-peer mix and ADR-0010's gate size.
func mixDegrees() []int {
	if cfg.smoke {
		return []int{4, 8, 32}
	}
	return []int{20, 64, 512}
}

// mixInstance returns the coefficients, constant term first, of
// (F-1)·∏(x - r) over distinct roots r drawn from SplitMix64 (the leading
// coefficient mixing.Coefficients gives every polynomial the client
// solves), and the roots in ascending order.
func mixInstance(degree int) (coeffs []*big.Int, roots []*big.Int) {
	rng := splitMix(0x6d69_7800 + uint64(degree))
	seen := make(map[string]bool)
	for len(roots) < degree {
		hi, lo := rng.next(), rng.next()
		v := new(big.Int).SetUint64(hi)
		v.Lsh(v, 64)
		v.Or(v, new(big.Int).SetUint64(lo))
		v.And(v, fieldP)
		if v.Cmp(fieldP) == 0 || seen[v.String()] {
			continue
		}
		seen[v.String()] = true
		roots = append(roots, v)
	}
	poly := []*big.Int{big.NewInt(1)}
	for _, r := range roots {
		next := make([]*big.Int, len(poly)+1)
		for i := range next {
			next[i] = new(big.Int)
		}
		for i, c := range poly {
			// (x - r)·c·x^i
			next[i+1].Add(next[i+1], c)
			t := new(big.Int).Mul(c, r)
			next[i].Sub(next[i], t)
		}
		for _, c := range next {
			c.Mod(c, fieldP)
		}
		poly = next
	}
	lead := new(big.Int).Sub(fieldP, big.NewInt(1))
	for _, c := range poly {
		c.Mul(c, lead)
		c.Mod(c, fieldP)
	}
	sorted := append([]*big.Int(nil), roots...)
	sort.Slice(sorted, func(i, j int) bool { return sorted[i].Cmp(sorted[j]) < 0 })
	return poly, sorted
}

// checkRoots reports whether a solver's roots, in any order, are exactly
// the instance's roots (ascending, as mixInstance returns them): the same
// count and every value equal. The mix workloads call it on the solver's
// output before timing it, as the Rust side asserts on its own solver's;
// the roots CHECK both sides print covers only the instance they solve.
func checkRoots(got, want []*big.Int) error {
	if len(got) != len(want) {
		return fmt.Errorf("%d roots, want %d", len(got), len(want))
	}
	for i, r := range got {
		if r == nil {
			return fmt.Errorf("root %d is nil", i)
		}
	}
	sorted := append([]*big.Int(nil), got...)
	sort.Slice(sorted, func(i, j int) bool { return sorted[i].Cmp(sorted[j]) < 0 })
	for i, r := range sorted {
		if r.Cmp(want[i]) != 0 {
			return fmt.Errorf("root %d (sorted) is %v, want %s", i, r, want[i].Text(16))
		}
	}
	return nil
}

func rootLines(roots []*big.Int) []string {
	s := make([]string, len(roots))
	for i, r := range roots {
		s[i] = r.Text(16)
	}
	return s
}

func mix() {
	for _, d := range mixDegrees() {
		name := fmt.Sprintf("mix.roots_%d", d)
		if !selected(name) {
			continue
		}
		coeffs, roots := mixInstance(d)
		check(name+"_sha256", sha256Lines(rootLines(roots)))
		mixSolve(name, coeffs, roots)
	}
}
