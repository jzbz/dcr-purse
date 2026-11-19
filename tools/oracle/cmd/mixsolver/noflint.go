// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//go:build !flint

package main

import (
	"encoding/json"

	"github.com/jzbz/dcr-purse/tools/oracle/internal/linejson"
)

// solverHandlers answers every solver command with the "no-flint" error, so
// that the untagged build compiles and runs anywhere.
func solverHandlers() map[string]linejson.Handler {
	stub := func(json.RawMessage) (any, error) {
		return nil, &linejson.Error{Kind: "no-flint", Text: noFlint}
	}
	return map[string]linejson.Handler{
		"flint.version":   stub,
		"roots":           stub,
		"solverrpc.roots": stub,
		"root-factors":    stub,
	}
}
