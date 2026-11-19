// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

package main

import (
	"bytes"
	"encoding/hex"
	"encoding/json"
	"math"
	"sort"

	"decred.org/dcrwallet/v5/wallet/walletdb"
)

// opArgs are the members of one operation object other than "op". Every
// member must be read by the operation (a member it does not know is a
// script error), so a misspelt optional argument cannot pass unnoticed.
type opArgs struct {
	m    map[string]json.RawMessage
	used map[string]bool
	op   string
}

func parseOp(raw json.RawMessage) (*opArgs, string, error) {
	var m map[string]json.RawMessage
	dec := json.NewDecoder(bytes.NewReader(raw))
	dec.UseNumber()
	if err := dec.Decode(&m); err != nil || m == nil {
		return nil, "", scriptErrorf("an operation must be a JSON object: %s", raw)
	}
	var name string
	if err := json.Unmarshal(m["op"], &name); err != nil || name == "" {
		return nil, "", scriptErrorf("operation without an op name: %s", raw)
	}
	delete(m, "op")
	return &opArgs{m: m, used: make(map[string]bool), op: name}, name, nil
}

// done reports members the operation did not read.
func (a *opArgs) done() error {
	var unknown []string
	for k := range a.m {
		if !a.used[k] {
			unknown = append(unknown, k)
		}
	}
	if len(unknown) > 0 {
		sort.Strings(unknown)
		return scriptErrorf("%s: unknown argument(s) %v", a.op, unknown)
	}
	return nil
}

func (a *opArgs) has(key string) bool {
	_, ok := a.m[key]
	a.used[key] = true
	return ok
}

func (a *opArgs) raw(key string) (json.RawMessage, bool) {
	v, ok := a.m[key]
	a.used[key] = true
	return v, ok
}

func (a *opArgs) decode(key string, v any) error {
	raw, ok := a.raw(key)
	if !ok {
		return scriptErrorf("%s: missing argument %q", a.op, key)
	}
	dec := json.NewDecoder(bytes.NewReader(raw))
	dec.DisallowUnknownFields()
	if err := dec.Decode(v); err != nil {
		return scriptErrorf("%s: argument %q: %v", a.op, key, err)
	}
	return nil
}

func (a *opArgs) str(key string) (string, error) {
	var s string
	err := a.decode(key, &s)
	return s, err
}

func (a *opArgs) optStr(key, def string) (string, error) {
	if !a.has(key) {
		return def, nil
	}
	return a.str(key)
}

func (a *opArgs) boolean(key string) (bool, error) {
	var b bool
	err := a.decode(key, &b)
	return b, err
}

func (a *opArgs) optBool(key string, def bool) (bool, error) {
	if !a.has(key) {
		return def, nil
	}
	return a.boolean(key)
}

func (a *opArgs) int64v(key string) (int64, error) {
	raw, ok := a.raw(key)
	if !ok {
		return 0, scriptErrorf("%s: missing argument %q", a.op, key)
	}
	var n json.Number
	if err := json.Unmarshal(raw, &n); err != nil {
		return 0, scriptErrorf("%s: argument %q is not a number", a.op, key)
	}
	v, err := n.Int64()
	if err != nil {
		return 0, scriptErrorf("%s: argument %q is not an integer", a.op, key)
	}
	return v, nil
}

func (a *opArgs) ranged(key string, lo, hi int64) (int64, error) {
	v, err := a.int64v(key)
	if err != nil {
		return 0, err
	}
	if v < lo || v > hi {
		return 0, scriptErrorf("%s: argument %q out of range", a.op, key)
	}
	return v, nil
}

func (a *opArgs) u32(key string) (uint32, error) {
	v, err := a.ranged(key, 0, math.MaxUint32)
	return uint32(v), err
}

func (a *opArgs) i32(key string) (int32, error) {
	v, err := a.ranged(key, math.MinInt32, math.MaxInt32)
	return int32(v), err
}

func (a *opArgs) optU32(key string, def uint32) (uint32, error) {
	if !a.has(key) {
		return def, nil
	}
	return a.u32(key)
}

func (a *opArgs) optI32(key string, def int32) (int32, error) {
	if !a.has(key) {
		return def, nil
	}
	return a.i32(key)
}

func (a *opArgs) optInt64(key string, def int64) (int64, error) {
	if !a.has(key) {
		return def, nil
	}
	return a.int64v(key)
}

func (a *opArgs) hexBytes(key string) ([]byte, error) {
	s, err := a.str(key)
	if err != nil {
		return nil, err
	}
	b, err := hex.DecodeString(s)
	if err != nil {
		return nil, scriptErrorf("%s: argument %q is not hex", a.op, key)
	}
	return b, nil
}

// passphrase reads a passphrase given as a string.
func (a *opArgs) passphrase(key string) ([]byte, error) {
	s, err := a.str(key)
	return []byte(s), err
}

// opCtx is the database transaction an operation runs in: none (noTx),
// a read transaction, or a read-write one (wtx non-nil).
type opCtx struct {
	rtx walletdb.ReadTx
	wtx walletdb.ReadWriteTx
}

func (c *opCtx) addrmgr() walletdb.ReadBucket { return c.rtx.ReadBucket([]byte("waddrmgr")) }
func (c *opCtx) txmgr() walletdb.ReadBucket   { return c.rtx.ReadBucket([]byte("wtxmgr")) }
func (c *opCtx) addrmgrW() walletdb.ReadWriteBucket {
	return c.wtx.ReadWriteBucket([]byte("waddrmgr"))
}
func (c *opCtx) txmgrW() walletdb.ReadWriteBucket {
	return c.wtx.ReadWriteBucket([]byte("wtxmgr"))
}

type opKind int

const (
	noTx          opKind = iota // runs outside any database transaction
	readTx                      // its own walletdb.View when not in a group
	writeTx                     // its own walletdb.Update when not in a group
	walletSession               // a wallet-package call inside wallet.open/close; never in a group
)

// opDef is one operation of the vocabulary.
type opDef struct {
	kind opKind
	// calls names the Go entry points the operation runs.
	calls string
	fn    func(r *runner, c *opCtx, a *opArgs) (any, error)
}
