// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//go:build flint

package main

/*
#cgo CFLAGS: -I/usr/local/include
#include <flint/flint.h>

static const char *purse_flint_version(void) {
#ifdef FLINT_VERSION
	return FLINT_VERSION;
#else
	return "unknown";
#endif
}

static long purse_flint_release(void) {
#ifdef __FLINT_RELEASE
	return (long)__FLINT_RELEASE;
#else
	return 0;
#endif
}
*/
import "C"

import (
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"math/big"
	"sort"
	"sync"

	"decred.org/cspp/v2/solver"
	"decred.org/cspp/v2/solverrpc"
	"github.com/decred/dcrd/mixing"
	"github.com/jzbz/dcr-purse/tools/oracle/internal/linejson"
)

// rootsResult is the answer of "roots" and "solverrpc.roots"; see the
// package comment.
type rootsResult struct {
	Class      string   `json:"class"`
	Accepted   bool     `json:"accepted"`
	Roots      []string `json:"roots"`
	FPRoots    []string `json:"fp_roots"`
	FlintOrder []string `json:"flint_order"`
	Count      int      `json:"count"`
	Error      string   `json:"error"`
}

// repeatedRoot is the method the repeated-root errors of solver.Roots
// (solver.go:105-108) and of solverrpc.Roots (rpc.go:131-134) carry.
type repeatedRoot interface {
	RepeatedRoot() *big.Int
}

// clientRule is what Client.roots does with the solver's answer
// (mixclient/client.go:2162-2187), as data instead of actions:
//
//   - ErrSolverProcessExited: "exited", the fallback that clears the solver
//     flag and waits for other peers' FPs (:2163-2172);
//   - any other error, or a root count other than len(a)-1: every local peer
//     sets triggeredBlame (:2173-2179); class "repeated" for the
//     repeated-root error, "error" for other errors, "count" otherwise;
//   - otherwise "ok": the roots sorted by big.Int.Cmp (:2181-2183) and each
//     root's big-endian minimal bytes (:2184-2187), which is exactly what
//     every local peer's factored-polynomial message carries (:2188-2195).
func clientRule(a, roots []*big.Int, err error) rootsResult {
	res := rootsResult{Roots: []string{}, FPRoots: []string{}, FlintOrder: []string{}}
	var rr repeatedRoot
	switch {
	case errors.Is(err, solverrpc.ErrSolverProcessExited):
		res.Class = "exited"
		res.Error = err.Error()
	case err != nil:
		res.Error = err.Error()
		if errors.As(err, &rr) {
			res.Class = "repeated"
		} else {
			res.Class = "error"
		}
	case len(roots) != len(a)-1:
		res.Class = "count"
		res.Count = len(roots)
		res.FlintOrder = hexList(roots)
	default:
		res.Class = "ok"
		res.Accepted = true
		res.Count = len(roots)
		res.FlintOrder = hexList(roots)
		sort.Slice(roots, func(i, j int) bool {
			return roots[i].Cmp(roots[j]) == -1
		})
		res.Roots = hexList(roots)
		for _, root := range roots {
			res.FPRoots = append(res.FPRoots, hex.EncodeToString(root.Bytes()))
		}
	}
	return res
}

// rpcSolver is the csppsolver program solverrpc runs: set by the first
// "solverrpc.roots" request, and fixed afterwards because solverrpc starts
// its process once per process (rpc.go:48-53, 92-97).
var (
	rpcSolverMu sync.Mutex
	rpcSolver   string
)

func solverHandlers() map[string]linejson.Handler {
	return map[string]linejson.Handler{
		"flint.version": func(json.RawMessage) (any, error) {
			return map[string]any{
				"version": C.GoString(C.purse_flint_version()),
				"release": int64(C.purse_flint_release()),
			}, nil
		},
		"roots": func(raw json.RawMessage) (any, error) {
			a, err := parseCoeffs(raw)
			if err != nil {
				return nil, err
			}
			// The solver linked in process; "solverrpc.roots" is the
			// client's own path through the csppsolver program.
			roots, err := solver.Roots(a, mixing.F)
			return clientRule(a, roots, err), nil
		},
		"solverrpc.roots": func(raw json.RawMessage) (any, error) {
			var args struct {
				Solver string `json:"solver"`
			}
			if err := json.Unmarshal(raw, &args); err != nil {
				return nil, err
			}
			a, err := parseCoeffs(raw)
			if err != nil {
				return nil, err
			}
			rpcSolverMu.Lock()
			defer rpcSolverMu.Unlock()
			switch rpcSolver {
			case "":
				rpcSolver = args.Solver
				solverrpc.SolverProcess = args.Solver
				if err := solverrpc.StartSolver(); err != nil {
					return nil, fmt.Errorf("start %s: %w", args.Solver, err)
				}
			case args.Solver:
			default:
				return nil, fmt.Errorf("solverrpc already runs %s, not %s", rpcSolver, args.Solver)
			}
			// client.go:2162: the call Client.roots makes.
			roots, err := solverrpc.Roots(a, mixing.F)
			return clientRule(a, roots, err), nil
		},
		"root-factors": func(raw json.RawMessage) (any, error) {
			a, err := parseCoeffs(raw)
			if err != nil {
				return nil, err
			}
			roots, exps, err := solver.RootFactors(a, mixing.F)
			if err != nil {
				return nil, err
			}
			return map[string]any{"roots": hexList(roots), "exps": exps}, nil
		},
	}
}
