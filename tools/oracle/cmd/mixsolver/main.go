// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

// Command mixsolver is the oracle for crates/purse-mixsolver: the mixing
// root solver dcrwallet uses, decred.org/cspp/v2@v2.4.0 (cgo over the C
// library FLINT), under the acceptance rule of the mixing client
// (github.com/decred/dcrd/mixing@v0.7.5/mixclient/client.go:2162-2187).
//
// # Two builds
//
// The solver package needs cgo and FLINT's headers and library, so the
// solver commands are behind the build tag "flint":
//
//   - Untagged (plain `go build ./cmd/mixsolver`, which is what the Rust
//     harness and `go build ./...`, `go vet ./...` and `go test ./...` do):
//     a stub. It serves "coefficients" (pure Go) and answers every solver
//     command with an error of kind "no-flint", which the Rust test reads as
//     "skip" (or, under PURSE_REQUIRE_FLINT, as a failure).
//
//   - Tagged: the real oracle. With FLINT installed (Ubuntu: apt-get install
//     libflint-dev libgmp-dev libmpfr-dev), from tools/oracle:
//
//     go build -tags flint -o "$DIR/purse-oracle-mixsolver" ./cmd/mixsolver
//     go build -o "$DIR/csppsolver" decred.org/cspp/v2/cmd/csppsolver
//
//     then run the Rust tests with PURSE_ORACLE_DIR="$DIR" (the harness then
//     takes prebuilt binaries from there instead of building them) and
//     PURSE_REQUIRE_FLINT=1. The second binary is the reference solver
//     program itself, which "solverrpc.roots" drives exactly as dcrwallet
//     does. The cgo link line is the solver package's own (solver.go:3-13:
//     -lflint -lmpfr -lgmp -lm, with /usr/local added to the default search
//     paths, which still apply); this file adds only the header for FLINT's
//     version.
//
// # Commands
//
// Field elements travel as lower-case hex strings (big.Int.Text(16)).
//
//	coefficients     {"power_sums": [hex]} -> [hex]: mixing.Coefficients,
//	                 the client's own construction of the solver's input
//	                 (dcnet.go:116-133); served by both builds.
//	flint.version    -> {"version": FLINT_VERSION, "release": __FLINT_RELEASE}
//	roots            {"coeffs": [hex]} -> {"class", "accepted", "roots",
//	                 "fp_roots", "flint_order", "count", "error"}:
//	                 solver.Roots(coeffs, mixing.F) in process, then the
//	                 client's rule (flint.go clientRule). class is
//	                   "ok"        no error and len(roots) == len(coeffs)-1:
//	                               accepted; roots sorted ascending as the
//	                               client sorts them (client.go:2181-2183),
//	                               fp_roots the hex of each sorted root's
//	                               big.Int.Bytes(), the exact contents of the
//	                               client's factored-polynomial message
//	                               (:2184-2195; "" for the root 0);
//	                   "count"     no error but another count: rejected
//	                               (client.go:2174), count holds it;
//	                   "repeated"  an error with a RepeatedRoot method
//	                               (solver.go:105-123): rejected;
//	                   "error"     any other error (e.g. "too few
//	                               coefficients", solver.go:42-44): rejected;
//	                   "exited"    solverrpc.ErrSolverProcessExited (only
//	                               over solverrpc): the client's fallback.
//	                 Every rejection is the same outcome for the client:
//	                 triggeredBlame on every local peer (client.go:2173-2179).
//	                 flint_order is the solver's answer before sorting, when
//	                 it returned one: FLINT's factor order.
//	solverrpc.roots  {"solver": path, "coeffs": [hex]} -> as "roots", through
//	                 solverrpc.Roots and the csppsolver program at path, gob
//	                 over its stdio: the call client.go:2162 makes. The first
//	                 request fixes the program for the process's lifetime.
//	root-factors     {"coeffs": [hex]} -> {"roots": [hex], "exps": [int]}:
//	                 solver.RootFactors as it reports every factor.
package main

import (
	"encoding/json"
	"fmt"
	"math/big"

	"github.com/decred/dcrd/mixing"
	"github.com/jzbz/dcr-purse/tools/oracle/internal/linejson"
)

// noFlint is the stub's answer to every solver command.
const noFlint = "built without FLINT: rebuild with `go build -tags flint` on a host with FLINT"

func parseHex(list []string) ([]*big.Int, error) {
	out := make([]*big.Int, len(list))
	for i, s := range list {
		v, ok := new(big.Int).SetString(s, 16)
		if !ok {
			return nil, fmt.Errorf("element %d: bad hex %q", i, s)
		}
		out[i] = v
	}
	return out, nil
}

func hexList(list []*big.Int) []string {
	out := make([]string, len(list))
	for i, v := range list {
		out[i] = v.Text(16)
	}
	return out
}

// parseCoeffs reads {"coeffs": [hex]}.
func parseCoeffs(raw json.RawMessage) ([]*big.Int, error) {
	var args struct {
		Coeffs []string `json:"coeffs"`
	}
	if err := json.Unmarshal(raw, &args); err != nil {
		return nil, err
	}
	return parseHex(args.Coeffs)
}

func main() {
	handlers := map[string]linejson.Handler{
		"coefficients": func(raw json.RawMessage) (any, error) {
			var args struct {
				PowerSums []string `json:"power_sums"`
			}
			if err := json.Unmarshal(raw, &args); err != nil {
				return nil, err
			}
			sums, err := parseHex(args.PowerSums)
			if err != nil {
				return nil, err
			}
			return hexList(mixing.Coefficients(sums)), nil
		},
	}
	for name, h := range solverHandlers() {
		handlers[name] = h
	}
	linejson.Serve(handlers)
}
