// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

// Command boltcursor is the bbolt v1.3.12 oracle for crates/purse-store's
// cursors (brief section 6 Phase 2, the cursor-conformance test; spec 04
// section 5.1, spec 01 section 5.7). It runs scripts of cursor and bucket
// operations through dcrwallet's walletdb interface and its bdb driver, so
// the answers are those of the bbolt dcrwallet links, reached through
// dcrwallet's own adapter. It speaks the linejson protocol (one JSON
// request per line on stdin, one response per line on stdout).
//
// Commands:
//
//	run {path, entries, other?, writable, ops}
//	    Creates a database at path (which must not exist) with
//	    walletdb.Create("bdb", path), and in one committed transaction
//	    the top-level buckets "c" holding entries and "o" holding other
//	    (each a list of [key, value] in hex). Then runs ops in a second
//	    transaction, read-write when writable is true (and committed),
//	    read-only otherwise, against one cursor on "c" created at the
//	    start of that transaction. Returns {results, final?}: one result
//	    per op that ran, and for a writable script the contents of "c" and
//	    "o" read back in a third, read-only transaction. The file is
//	    removed before the command returns.
//
// Each op is {op, k?, v?, hi?, rev?}, keys and values in hex:
//
//	first, last, next, prev  the cursor move          -> {k, v}
//	seek {k}                 Seek(k)                  -> {k, v}
//	delete                   the cursor's Delete      -> {} or {err}
//	put {k, v}               Put on "c"               -> {} or {err}
//	del {k}                  Delete on "c"            -> {} or {err}
//	get {k}                  Get on "c"               -> {v}
//	oput {k, v}, odel {k}    Put, Delete on "o"       -> {} or {err}
//	fresh                    replace the cursor by a new one on "c" -> {}
//	count                    KeyN of "c"              -> {n}
//	range {k, hi?, rev?}     the keys in [k, hi) (hi absent: no upper
//	                         bound), walked as dcrwallet walks a range,
//	                         with a second cursor -> {kv: [[k, v], ...]}
//
// In {k, v} a member is null when Go's slice is nil (the end of the
// bucket); an empty value is "". An error is {err, kind?}: the text of
// the walletdb error and, for an *errors.Error, its kind. A Go panic in an
// op is caught and reported as {panic: text}, a bounds-check panic's text
// cut after "out of range" as tools/oracle/cmd/oprunner cuts it (ADR-0002),
// since the indices the runtime prints can differ between architectures;
// the script stops there and
// the transaction is rolled back, and no final contents are returned. A
// read-only script may hold only first, last, next, prev, seek, get,
// fresh, count and range.
package main

import (
	"bytes"
	"context"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"strings"

	werrors "decred.org/dcrwallet/v5/errors"
	"decred.org/dcrwallet/v5/wallet/walletdb"
	"github.com/jzbz/dcr-purse/tools/oracle/internal/linejson"

	_ "decred.org/dcrwallet/v5/wallet/drivers/bdb"
)

func main() {
	linejson.Serve(map[string]linejson.Handler{
		"run": cmdRun,
	})
}

var (
	bucketC = []byte("c")
	bucketO = []byte("o")
)

type op struct {
	Op  string  `json:"op"`
	K   string  `json:"k"`
	V   string  `json:"v"`
	Hi  *string `json:"hi"`
	Rev bool    `json:"rev"`
}

type runArgs struct {
	Path     string      `json:"path"`
	Entries  [][2]string `json:"entries"`
	Other    [][2]string `json:"other"`
	Writable bool        `json:"writable"`
	Ops      []op        `json:"ops"`
}

type runResult struct {
	Results []map[string]any       `json:"results"`
	Final   map[string][][2]string `json:"final,omitempty"`
}

// errPanicked stops a script after an op panicked, rolling its
// transaction back.
var errPanicked = errors.New("op panicked")

func hexOrNil(b []byte) any {
	if b == nil {
		return nil
	}
	return hex.EncodeToString(b)
}

func moved(k, v []byte) map[string]any {
	return map[string]any{"k": hexOrNil(k), "v": hexOrNil(v)}
}

func errResult(err error) map[string]any {
	if err == nil {
		return map[string]any{}
	}
	r := map[string]any{"err": err.Error()}
	var e *werrors.Error
	if errors.As(err, &e) && e.Kind != 0 {
		r["kind"] = e.Kind.String()
	}
	return r
}

func unhex(s string) ([]byte, error) {
	b, err := hex.DecodeString(s)
	if err != nil {
		return nil, fmt.Errorf("bad hex %q: %w", s, err)
	}
	return b, nil
}

func load(b walletdb.ReadWriteBucket, pairs [][2]string) error {
	for _, kv := range pairs {
		k, err := unhex(kv[0])
		if err != nil {
			return err
		}
		v, err := unhex(kv[1])
		if err != nil {
			return err
		}
		if err := b.Put(k, v); err != nil {
			return fmt.Errorf("setup put %x: %w", k, err)
		}
	}
	return nil
}

func dump(b walletdb.ReadBucket) [][2]string {
	out := [][2]string{}
	c := b.ReadCursor()
	for k, v := c.First(); k != nil; k, v = c.Next() {
		out = append(out, [2]string{hex.EncodeToString(k), hex.EncodeToString(v)})
	}
	c.Close()
	return out
}

// walkRange lists the entries with keys in [lo, hi) the way dcrwallet's
// range queries walk them: forward by Seek(lo) then Next while below hi;
// backward by Seek(hi) (Last when unbounded), one Prev unless Seek found
// nothing, then Prev while at or above lo (spec 04 section 5.1 item 3:
// Prev after a Seek that found nothing returns the last key).
func walkRange(b walletdb.ReadBucket, lo, hi []byte, unbounded, rev bool) [][2]string {
	out := [][2]string{}
	c := b.ReadCursor()
	defer c.Close()
	add := func(k, v []byte) {
		out = append(out, [2]string{hex.EncodeToString(k), hex.EncodeToString(v)})
	}
	if !rev {
		for k, v := c.Seek(lo); k != nil; k, v = c.Next() {
			if !unbounded && bytes.Compare(k, hi) >= 0 {
				break
			}
			add(k, v)
		}
		return out
	}
	var k, v []byte
	if unbounded {
		k, v = c.Last()
	} else {
		k, v = c.Seek(hi)
		k, v = c.Prev()
	}
	for ; k != nil && bytes.Compare(k, lo) >= 0; k, v = c.Prev() {
		add(k, v)
	}
	return out
}

// script runs ops against the buckets of one transaction. rw is nil for a
// read-only script.
type script struct {
	r       walletdb.ReadBucket
	rw      walletdb.ReadWriteBucket
	o       walletdb.ReadWriteBucket
	c       walletdb.ReadCursor
	results []map[string]any
}

func (s *script) cursor() {
	if s.rw != nil {
		s.c = s.rw.ReadWriteCursor()
	} else {
		s.c = s.r.ReadCursor()
	}
}

// boundsPanics are the runtime's bounds-check panics, whose printed indices
// can depend on the architecture the binary was compiled for (see
// tools/oracle/cmd/oprunner's canonPanic).
var boundsPanics = []string{
	"runtime error: slice bounds out of range",
	"runtime error: index out of range",
}

// canonPanic drops the indices from a bounds-check panic's text, so that a
// result is the same on every architecture. Any other text is unchanged.
func canonPanic(text string) string {
	for _, prefix := range boundsPanics {
		if strings.HasPrefix(text, prefix) {
			return prefix
		}
	}
	return text
}

// step runs one op, catching a panic.
func (s *script) step(o op) (res map[string]any, err error) {
	defer func() {
		if p := recover(); p != nil {
			res = map[string]any{"panic": canonPanic(fmt.Sprint(p))}
			err = errPanicked
		}
	}()
	k, err := unhex(o.K)
	if err != nil {
		return nil, err
	}
	v, err := unhex(o.V)
	if err != nil {
		return nil, err
	}
	writable := func() error {
		if s.rw == nil {
			return fmt.Errorf("op %q in a read-only script", o.Op)
		}
		return nil
	}
	switch o.Op {
	case "first":
		return moved(s.c.First()), nil
	case "last":
		return moved(s.c.Last()), nil
	case "next":
		return moved(s.c.Next()), nil
	case "prev":
		return moved(s.c.Prev()), nil
	case "seek":
		return moved(s.c.Seek(k)), nil
	case "get":
		return map[string]any{"v": hexOrNil(s.r.Get(k))}, nil
	case "count":
		return map[string]any{"n": s.r.KeyN()}, nil
	case "fresh":
		s.c.Close()
		s.cursor()
		return map[string]any{}, nil
	case "range":
		var hi []byte
		if o.Hi != nil {
			if hi, err = unhex(*o.Hi); err != nil {
				return nil, err
			}
		}
		return map[string]any{"kv": walkRange(s.r, k, hi, o.Hi == nil, o.Rev)}, nil
	case "delete":
		if err := writable(); err != nil {
			return nil, err
		}
		return errResult(s.c.(walletdb.ReadWriteCursor).Delete()), nil
	case "put":
		if err := writable(); err != nil {
			return nil, err
		}
		return errResult(s.rw.Put(k, v)), nil
	case "del":
		if err := writable(); err != nil {
			return nil, err
		}
		return errResult(s.rw.Delete(k)), nil
	case "oput":
		if err := writable(); err != nil {
			return nil, err
		}
		return errResult(s.o.Put(k, v)), nil
	case "odel":
		if err := writable(); err != nil {
			return nil, err
		}
		return errResult(s.o.Delete(k)), nil
	}
	return nil, fmt.Errorf("unknown op %q", o.Op)
}

func (s *script) run(ops []op) error {
	s.cursor()
	defer func() { s.c.Close() }()
	for _, o := range ops {
		res, err := s.step(o)
		if res != nil {
			s.results = append(s.results, res)
		}
		if err != nil {
			return err
		}
	}
	return nil
}

func cmdRun(raw json.RawMessage) (any, error) {
	var a runArgs
	if len(raw) == 0 {
		return nil, errors.New("missing args")
	}
	if err := json.Unmarshal(raw, &a); err != nil {
		return nil, err
	}
	if a.Path == "" {
		return nil, errors.New("missing path")
	}
	if _, err := os.Stat(a.Path); err == nil {
		return nil, fmt.Errorf("%s exists", a.Path)
	}
	ctx := context.Background()
	db, err := walletdb.Create("bdb", a.Path)
	if err != nil {
		return nil, err
	}
	defer os.Remove(a.Path)
	defer db.Close()

	err = walletdb.Update(ctx, db, func(tx walletdb.ReadWriteTx) error {
		c, err := tx.CreateTopLevelBucket(bucketC)
		if err != nil {
			return err
		}
		o, err := tx.CreateTopLevelBucket(bucketO)
		if err != nil {
			return err
		}
		if err := load(c, a.Entries); err != nil {
			return err
		}
		return load(o, a.Other)
	})
	if err != nil {
		return nil, fmt.Errorf("setup: %w", err)
	}

	s := &script{}
	var runErr error
	if a.Writable {
		runErr = walletdb.Update(ctx, db, func(tx walletdb.ReadWriteTx) error {
			s.rw = tx.ReadWriteBucket(bucketC)
			s.r = s.rw
			s.o = tx.ReadWriteBucket(bucketO)
			return s.run(a.Ops)
		})
	} else {
		runErr = walletdb.View(ctx, db, func(tx walletdb.ReadTx) error {
			s.r = tx.ReadBucket(bucketC)
			return s.run(a.Ops)
		})
	}
	out := runResult{Results: s.results}
	if out.Results == nil {
		out.Results = []map[string]any{}
	}
	if errors.Is(runErr, errPanicked) {
		return out, nil
	}
	if runErr != nil {
		return nil, runErr
	}
	if a.Writable {
		err = walletdb.View(ctx, db, func(tx walletdb.ReadTx) error {
			out.Final = map[string][][2]string{
				"c": dump(tx.ReadBucket(bucketC)),
				"o": dump(tx.ReadBucket(bucketO)),
			}
			return nil
		})
		if err != nil {
			return nil, err
		}
	}
	return out, nil
}
