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
//	run {path, page_size, entries, other?, writable, ops}
//	    Creates an empty database at path (which must not exist) with
//	    bbolt's Options.PageSize set to page_size, then opens it with
//	    walletdb.Open("bdb", path) and creates, in one committed
//	    transaction, the top-level buckets "c" holding entries and "o"
//	    holding other (each a list of [key, value] in hex). Then runs ops
//	    in a second transaction, read-write when writable is true (and
//	    committed), read-only otherwise, against one cursor on "c" created
//	    at the start of that transaction. Returns {results, final?,
//	    layout}: one result per op that ran; for a writable script the
//	    contents of "c" and "o" read back in a third, read-only
//	    transaction; and the shape of "c" as the setup transaction
//	    committed it, {page_size, os_page_size, leaf_pages, branch_pages,
//	    depth} (bbolt's BucketStats; leaf_pages is 0 for a bucket stored
//	    inline in its parent). The file is removed before the command
//	    returns.
//
// Why the page size is set. dcrwallet's driver opens with bolt.Open(path,
// 0600, nil) (wallet/internal/bdb/db.go:321), so a database it creates
// gets bbolt's default page size, os.Getpagesize(): 4096 bytes on x86-64
// Linux and Windows, 16384 on Apple silicon. How many keys a leaf holds,
// and so which cursor defects of bbolt a script reaches (QK-0747,
// QK-0749, QK-0750 need a bucket of several leaves), follows from it. The
// oracle therefore chooses the page size instead of taking the host's, so
// a script meets the same layout on every host. Only the empty file and the
// read-only layout query use bbolt directly; the setup and the script run
// through the driver: bbolt reads the page size of an existing file from
// its meta page (bbolt db.go:271-279), so the driver's bolt.Open gets the
// chosen size without being given it.
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
	"time"

	werrors "decred.org/dcrwallet/v5/errors"
	"decred.org/dcrwallet/v5/wallet/walletdb"
	"github.com/jzbz/dcr-purse/tools/oracle/internal/linejson"
	bolt "go.etcd.io/bbolt"

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
	PageSize int         `json:"page_size"`
	Entries  [][2]string `json:"entries"`
	Other    [][2]string `json:"other"`
	Writable bool        `json:"writable"`
	Ops      []op        `json:"ops"`
}

type runResult struct {
	Results []map[string]any       `json:"results"`
	Final   map[string][][2]string `json:"final,omitempty"`
	Layout  layout                 `json:"layout"`
}

// layout is the shape of bucket "c" after the setup transaction.
type layout struct {
	PageSize    int `json:"page_size"`
	OSPageSize  int `json:"os_page_size"`
	LeafPages   int `json:"leaf_pages"`
	BranchPages int `json:"branch_pages"`
	Depth       int `json:"depth"`
}

// create makes an empty bbolt database at path with the given page size.
func create(path string, pageSize int) error {
	db, err := bolt.Open(path, 0600, &bolt.Options{PageSize: pageSize, Timeout: 2 * time.Second})
	if err != nil {
		return err
	}
	return db.Close()
}

// shape reads the layout of bucket "c" with bbolt directly (walletdb has
// no statistics), read-only, while no other handle holds the file.
func shape(path string) (layout, error) {
	db, err := bolt.Open(path, 0600, &bolt.Options{ReadOnly: true, Timeout: 2 * time.Second})
	if err != nil {
		return layout{}, err
	}
	defer db.Close()
	l := layout{PageSize: db.Info().PageSize, OSPageSize: os.Getpagesize()}
	err = db.View(func(tx *bolt.Tx) error {
		b := tx.Bucket(bucketC)
		if b == nil {
			return errors.New(`no bucket "c"`)
		}
		st := b.Stats()
		l.LeafPages, l.BranchPages, l.Depth = st.LeafPageN, st.BranchPageN, st.Depth
		return nil
	})
	return l, err
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
	if a.PageSize <= 0 {
		return nil, errors.New("missing page_size")
	}
	if _, err := os.Stat(a.Path); err == nil {
		return nil, fmt.Errorf("%s exists", a.Path)
	}
	defer os.Remove(a.Path)
	if err := create(a.Path, a.PageSize); err != nil {
		return nil, fmt.Errorf("create: %w", err)
	}
	ctx := context.Background()
	db, err := walletdb.Open("bdb", a.Path)
	if err != nil {
		return nil, err
	}
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
	if cerr := db.Close(); err == nil {
		err = cerr
	}
	if err != nil {
		return nil, fmt.Errorf("setup: %w", err)
	}
	lay, err := shape(a.Path)
	if err != nil {
		return nil, fmt.Errorf("layout: %w", err)
	}
	if db, err = walletdb.Open("bdb", a.Path); err != nil {
		return nil, err
	}
	defer db.Close()

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
	out := runResult{Results: s.results, Layout: lay}
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
