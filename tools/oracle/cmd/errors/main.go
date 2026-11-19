// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

// Command errors is the oracle for crates/purse-errors. It builds error
// values with dcrwallet's errors package from a JSON program and reports
// what Go makes of them: the rendered text, the shape of the chain after
// E's promote and collapse rules, errors.Is against every kind and against
// every other value of the program, errors.As for a Kind and an *Error,
// Unwrap and Stacks.
//
// Commands:
//
//	eval  {"sep": "<separator>", "nodes": [<node>...]}
//	kinds {"codes": [<int>...]}
//
// A node builds one error value and may refer to earlier nodes by index:
//
//	{"t":"e","args":[<arg>...],"stack":<bool>}   errors.E or errors.WithStack
//	{"t":"lit","op":"..","kind":<k>,"err":<idx|null>}  &errors.Error{...}
//	{"t":"new","text":".."}                      errors.New
//	{"t":"kind","kind":<k>}                      errors.Kind used as an error
//	{"t":"wrap","prefix":"..","errs":[<idx>...]} fmt.Errorf(prefix+"%w; %w...")
//	{"t":"join","errs":[<idx>...]}               errors.Join (may yield nil)
//
// An arg of "e" is one of {"op":".."}, {"kind":<k>}, {"str":".."} or
// {"err":<idx>}; the last passes the value of an earlier node, which is a
// nil error when that node evaluated to nil.
package main

import (
	"encoding/json"
	"fmt"
	"strings"

	"decred.org/dcrwallet/v5/errors"
	"github.com/jzbz/dcr-purse/tools/oracle/internal/linejson"
)

type arg struct {
	Op   *string `json:"op"`
	Kind *int    `json:"kind"`
	Str  *string `json:"str"`
	Err  *int    `json:"err"`
}

type node struct {
	T      string `json:"t"`
	Args   []arg  `json:"args"`
	Stack  bool   `json:"stack"`
	Op     string `json:"op"`
	Kind   int    `json:"kind"`
	Err    *int   `json:"err"`
	Text   string `json:"text"`
	Prefix string `json:"prefix"`
	Errs   []int  `json:"errs"`
}

type evalArgs struct {
	Sep   string `json:"sep"`
	Nodes []node `json:"nodes"`
}

// numKinds is the number of defined kinds (Other through Deployment).
const numKinds = 21

func build(nodes []node) ([]error, error) {
	vals := make([]error, len(nodes))
	ref := func(i, at int) (error, error) {
		if i < 0 || i >= at {
			return nil, fmt.Errorf("node %d: bad reference %d", at, i)
		}
		return vals[i], nil
	}
	for i, n := range nodes {
		switch n.T {
		case "e":
			if len(n.Args) == 0 {
				return nil, fmt.Errorf("node %d: E needs arguments", i)
			}
			args := make([]any, 0, len(n.Args))
			for _, a := range n.Args {
				switch {
				case a.Op != nil:
					args = append(args, errors.Op(*a.Op))
				case a.Kind != nil:
					args = append(args, errors.Kind(*a.Kind))
				case a.Str != nil:
					args = append(args, *a.Str)
				case a.Err != nil:
					v, err := ref(*a.Err, i)
					if err != nil {
						return nil, err
					}
					args = append(args, v)
				default:
					return nil, fmt.Errorf("node %d: empty arg", i)
				}
			}
			if n.Stack {
				vals[i] = errors.WithStack(args...)
			} else {
				vals[i] = errors.E(args...)
			}
		case "lit":
			e := &errors.Error{Op: errors.Op(n.Op), Kind: errors.Kind(n.Kind)}
			if n.Err != nil {
				v, err := ref(*n.Err, i)
				if err != nil {
					return nil, err
				}
				e.Err = v
			}
			vals[i] = e
		case "new":
			vals[i] = errors.New(n.Text)
		case "kind":
			vals[i] = errors.Kind(n.Kind)
		case "wrap":
			verbs := make([]string, len(n.Errs))
			wrapped := make([]any, len(n.Errs))
			for j, r := range n.Errs {
				v, err := ref(r, i)
				if err != nil {
					return nil, err
				}
				if v == nil {
					return nil, fmt.Errorf("node %d: wrap of a nil error", i)
				}
				verbs[j] = "%w"
				wrapped[j] = v
			}
			vals[i] = fmt.Errorf(n.Prefix+strings.Join(verbs, "; "), wrapped...)
		case "join":
			errs := make([]error, len(n.Errs))
			for j, r := range n.Errs {
				v, err := ref(r, i)
				if err != nil {
					return nil, err
				}
				errs[j] = v
			}
			vals[i] = errors.Join(errs...)
		default:
			return nil, fmt.Errorf("node %d: unknown type %q", i, n.T)
		}
	}
	return vals, nil
}

// same returns the index of the first node whose value is v (Go interface
// equality), or -1.
func same(vals []error, v error) int {
	for i, w := range vals {
		if w != nil && w == v {
			return i
		}
	}
	return -1
}

func hasStack(e *errors.Error) bool {
	inner := 0
	if ie, ok := e.Err.(*errors.Error); ok {
		inner = len(errors.Stacks(ie))
	}
	return len(errors.Stacks(e)) > inner
}

func shape(vals []error, v error) any {
	if v == nil {
		return nil
	}
	m := map[string]any{"same": same(vals, v)}
	switch x := v.(type) {
	case *errors.Error:
		m["t"] = "error"
		m["op"] = string(x.Op)
		m["kind"] = int(x.Kind)
		m["stack"] = hasStack(x)
		m["err"] = shape(vals, x.Err)
	case errors.Kind:
		m["t"] = "kind"
		m["kind"] = int(x)
	case interface{ Unwrap() error }:
		m["t"] = "wrap"
		m["text"] = v.Error()
		m["errs"] = shapes(vals, []error{x.Unwrap()})
	case interface{ Unwrap() []error }:
		if fmt.Sprintf("%T", v) == "*errors.joinError" {
			m["t"] = "join"
		} else {
			m["t"] = "wrap"
			m["text"] = v.Error()
		}
		m["errs"] = shapes(vals, x.Unwrap())
	default:
		if fmt.Sprintf("%T", v) != "*errors.errorString" {
			panic(fmt.Sprintf("unexpected error type %T", v))
		}
		m["t"] = "text"
		m["text"] = v.Error()
	}
	return m
}

func shapes(vals []error, errs []error) []any {
	out := make([]any, len(errs))
	for i, e := range errs {
		out[i] = shape(vals, e)
	}
	return out
}

// is reports errors.Is(err, target) as "t" or "f", or "p" when Go panics.
func is(err, target error) (r string) {
	defer func() {
		if recover() != nil {
			r = "p"
		}
	}()
	if errors.Is(err, target) {
		return "t"
	}
	return "f"
}

func eval(raw json.RawMessage) (any, error) {
	var args evalArgs
	if err := json.Unmarshal(raw, &args); err != nil {
		return nil, err
	}
	errors.Separator = args.Sep
	vals, err := build(args.Nodes)
	if err != nil {
		return nil, err
	}
	out := make([]any, len(vals))
	for i, v := range vals {
		if v == nil {
			out[i] = map[string]any{"nil": true}
			continue
		}
		isKind := make([]bool, numKinds)
		for k := range isKind {
			isKind[k] = errors.Is(v, errors.Kind(k))
		}
		var asKind any
		var k errors.Kind
		if errors.As(v, &k) {
			asKind = int(k)
		}
		var asError any
		var e *errors.Error
		if errors.As(v, &e) {
			asError = shape(vals, e)
		}
		var unwrap any
		if e, ok := v.(*errors.Error); ok {
			unwrap = shape(vals, e.Unwrap())
		}
		isTargets := make([]string, len(vals))
		for j, t := range vals {
			isTargets[j] = is(v, t)
		}
		out[i] = map[string]any{
			"text":     v.Error(),
			"shape":    shape(vals, v),
			"is_kind":  isKind,
			"as_kind":  asKind,
			"as_error": asError,
			"unwrap":   unwrap,
			"stacks":   len(errors.Stacks(v)),
			"is":       isTargets,
		}
	}
	return map[string]any{"nodes": out}, nil
}

func kinds(raw json.RawMessage) (any, error) {
	var args struct {
		Codes []int `json:"codes"`
	}
	if err := json.Unmarshal(raw, &args); err != nil {
		return nil, err
	}
	out := make([]string, len(args.Codes))
	for i, c := range args.Codes {
		out[i] = errors.Kind(c).Error()
	}
	return out, nil
}

func main() {
	linejson.Serve(map[string]linejson.Handler{
		"eval":  eval,
		"kinds": kinds,
	})
}
