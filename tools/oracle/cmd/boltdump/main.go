// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

// Command boltdump is the bbolt v1.3.12 oracle for crates/purse-boltread.
// It speaks the linejson protocol (one JSON request per line on stdin, one
// response per line on stdout).
//
// Commands:
//
//	dump     {path, out?, preload_freelist?}
//	         Opens path read-only with bbolt and writes the canonical dump
//	         (format below) to out (when given). Returns
//	         {sha256, entries, txid, page_size, pgid, free_page_n}.
//	         Errors carry a kind: invalid, version, checksum, toosmall,
//	         timeout, other.
//	generate {path, seed, recipe}
//	         Creates a database at path (which must not exist) from a recipe
//	         and a PRNG seed; see the recipe type for the knobs. Returns
//	         {txid, page_size, file_size, keys, buckets}.
//	cursor   {path, bucket, ops}
//	         bucket is a list of hex name components (empty: the root
//	         bucket); ops is a list of {op, key?} with op one of first, last,
//	         next, prev, seek, get, bucket. Returns one {key, value, bucket}
//	         per op (key and value hex or null; for get, key echoes the
//	         argument and value is Get's result; for bucket, bucket says
//	         whether Bucket(key) is non-nil).
//	stats    {path, bucket}
//	         Returns bbolt's BucketStats for the bucket.
//	hold     {path, timeout_ms?, read_only?}
//	         Opens path read-write with dcrwallet's options (bolt.Open(path,
//	         0600, nil), plus a lock timeout, 10 s unless given, so a test
//	         cannot hang) and keeps it open, holding the exclusive file lock
//	         (with read_only, opens read-only and holds the shared lock).
//	hold_start {path}
//	         Starts exactly dcrwallet's open, bolt.Open(path, 0600, nil),
//	         with no lock timeout, in the background, and returns at once.
//	hold_wait {timeout_ms}
//	         Waits up to timeout_ms for the open started by hold_start:
//	         {done: false} if it is still waiting for the lock; {done: true}
//	         once it has opened (the database is then held as by hold); its
//	         error if it failed.
//	release  {}
//	         Closes the database opened by hold or hold_start.
//
// Dump format (version 1). The dump lists every entry of the bucket tree,
// one line per entry, each line ending in "\n", in cursor order, depth
// first: a nested bucket's own line is followed immediately by the lines of
// its contents, before the next entry of its parent. Fields are separated by
// one space; byte strings are lower-case hex (an empty string is empty).
//
//	b <path> <name> <sequence>
//	k <path> <key> <value>
//
// <path> is the path of the bucket holding the entry: "/" for the root
// bucket, otherwise "/" followed by the hex names of the buckets from the
// top level down, joined by "/" (for example "/6d657461/6b6579"). A "b" line
// is a nested bucket with that name, <sequence> its NextSequence counter in
// decimal; a "k" line is a key/value pair. Whether a bucket is stored inline
// and which pages hold what are not part of the dump. The digest is the
// SHA-256 of the whole dump, and entries is its number of lines.
package main

import (
	"bufio"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"math/rand/v2"
	"os"
	"strconv"
	"time"

	"github.com/jzbz/dcr-purse/tools/oracle/internal/linejson"
	bolt "go.etcd.io/bbolt"
)

func main() {
	linejson.Serve(map[string]linejson.Handler{
		"dump":       cmdDump,
		"generate":   cmdGenerate,
		"cursor":     cmdCursor,
		"stats":      cmdStats,
		"hold":       cmdHold,
		"hold_start": cmdHoldStart,
		"hold_wait":  cmdHoldWait,
		"release":    cmdRelease,
	})
}

func unmarshal(args json.RawMessage, v any) error {
	if len(args) == 0 {
		return errors.New("missing args")
	}
	return json.Unmarshal(args, v)
}

// classify gives an open error its machine-readable kind.
func classify(err error) error {
	kind := "other"
	switch {
	case errors.Is(err, bolt.ErrInvalid):
		kind = "invalid"
	case errors.Is(err, bolt.ErrVersionMismatch):
		kind = "version"
	case errors.Is(err, bolt.ErrChecksum):
		kind = "checksum"
	case errors.Is(err, bolt.ErrTimeout):
		kind = "timeout"
	case err.Error() == "file size too small":
		kind = "toosmall"
	}
	return &linejson.Error{Kind: kind, Text: err.Error()}
}

func openRO(path string, preload bool) (*bolt.DB, error) {
	db, err := bolt.Open(path, 0600, &bolt.Options{
		ReadOnly:        true,
		Timeout:         2 * time.Second,
		PreLoadFreelist: preload,
	})
	if err != nil {
		return nil, classify(err)
	}
	return db, nil
}

// ---------------------------------------------------------------- dump

type dumpArgs struct {
	Path            string `json:"path"`
	Out             string `json:"out"`
	PreloadFreelist bool   `json:"preload_freelist"`
}

type dumpResult struct {
	SHA256    string `json:"sha256"`
	Entries   uint64 `json:"entries"`
	TxID      uint64 `json:"txid"`
	PageSize  int    `json:"page_size"`
	PgID      uint64 `json:"pgid"`
	FreePageN int    `json:"free_page_n"`
}

type dumper struct {
	w       *bufio.Writer
	entries uint64
	line    []byte
}

func (d *dumper) emit(kind byte, path, key, val []byte, seq uint64, isBucket bool) error {
	l := d.line[:0]
	l = append(l, kind, ' ')
	l = append(l, path...)
	l = append(l, ' ')
	l = hex.AppendEncode(l, key)
	l = append(l, ' ')
	if isBucket {
		l = strconv.AppendUint(l, seq, 10)
	} else {
		l = hex.AppendEncode(l, val)
	}
	l = append(l, '\n')
	d.line = l
	d.entries++
	_, err := d.w.Write(l)
	return err
}

func childPath(path, name []byte) []byte {
	p := make([]byte, 0, len(path)+1+2*len(name))
	p = append(p, path...)
	if len(path) > 1 {
		p = append(p, '/')
	}
	return hex.AppendEncode(p, name)
}

func (d *dumper) bucket(path []byte, b *bolt.Bucket) error {
	c := b.Cursor()
	for k, v := c.First(); k != nil; k, v = c.Next() {
		if v != nil {
			if err := d.emit('k', path, k, v, 0, false); err != nil {
				return err
			}
			continue
		}
		child := b.Bucket(k)
		if child == nil {
			return fmt.Errorf("entry %x reads as a bucket but Bucket() finds none", k)
		}
		if err := d.emit('b', path, k, nil, child.Sequence(), true); err != nil {
			return err
		}
		if err := d.bucket(childPath(path, k), child); err != nil {
			return err
		}
	}
	return nil
}

func cmdDump(raw json.RawMessage) (any, error) {
	var args dumpArgs
	if err := unmarshal(raw, &args); err != nil {
		return nil, err
	}
	db, err := openRO(args.Path, args.PreloadFreelist)
	if err != nil {
		return nil, err
	}
	defer db.Close()

	h := sha256.New()
	var sink io.Writer = h
	var f *os.File
	if args.Out != "" {
		f, err = os.Create(args.Out)
		if err != nil {
			return nil, err
		}
		defer f.Close()
		sink = io.MultiWriter(h, f)
	}
	d := &dumper{w: bufio.NewWriterSize(sink, 1<<20)}
	var res dumpResult
	err = db.View(func(tx *bolt.Tx) error {
		res.TxID = uint64(tx.ID())
		res.PageSize = db.Info().PageSize
		res.PgID = uint64(tx.Size()) / uint64(res.PageSize)
		return d.bucket([]byte("/"), tx.Cursor().Bucket())
	})
	if err != nil {
		return nil, err
	}
	if err := d.w.Flush(); err != nil {
		return nil, err
	}
	if f != nil {
		if err := f.Close(); err != nil {
			return nil, err
		}
	}
	res.SHA256 = hex.EncodeToString(h.Sum(nil))
	res.Entries = d.entries
	res.FreePageN = db.Stats().FreePageN
	return res, nil
}

// ---------------------------------------------------------------- generate

// recipe describes a database to generate. Sizes are inclusive [min, max]
// ranges; percentages and permilles are per decision.
type recipe struct {
	// PageSize is bbolt's Options.PageSize (4096, 16384, ...).
	PageSize int `json:"page_size"`
	// Commits is the number of write transactions (at least 1). The first
	// creates the tree; each later one mutates it.
	Commits int `json:"commits"`
	// TopBuckets top-level buckets are created, each with SubBuckets
	// children per level down to Depth further levels.
	TopBuckets int `json:"top_buckets"`
	Depth      int `json:"depth"`
	SubBuckets int `json:"sub_buckets"`
	// Keys is the number of keys put into each bucket by the first commit.
	Keys      [2]int `json:"keys"`
	KeySize   [2]int `json:"key_size"`
	ValueSize [2]int `json:"value_size"`
	// LargePermille of values get a size from LargeSize instead (values
	// larger than a page make overflow pages).
	LargePermille int    `json:"large_permille"`
	LargeSize     [2]int `json:"large_size"`
	// InlineBuckets tiny buckets (1-3 small pairs, so they stay inline)
	// and EmptyBuckets empty ones are created in random parents.
	InlineBuckets int `json:"inline_buckets"`
	EmptyBuckets  int `json:"empty_buckets"`
	// Later commits: each bucket gets AddPercent of its first-commit key
	// count as new keys, has DeletePercent of its keys deleted and
	// OverwritePercent overwritten; DropPercent is the chance per commit of
	// deleting one random nested bucket; SeqPercent the chance per bucket
	// and commit of a NextSequence call.
	AddPercent       int `json:"add_percent"`
	DeletePercent    int `json:"delete_percent"`
	OverwritePercent int `json:"overwrite_percent"`
	DropPercent      int `json:"drop_percent"`
	SeqPercent       int `json:"seq_percent"`
	// BinaryNames gives buckets random binary names instead of readable ones.
	BinaryNames bool `json:"binary_names"`
}

type generateArgs struct {
	Path   string `json:"path"`
	Seed   uint64 `json:"seed"`
	Recipe recipe `json:"recipe"`
}

type generateResult struct {
	TxID     uint64 `json:"txid"`
	PageSize int    `json:"page_size"`
	FileSize int64  `json:"file_size"`
	Keys     int    `json:"keys"`
	Buckets  int    `json:"buckets"`
}

type gen struct {
	r   *rand.Rand
	rc  *recipe
	all [][][]byte // paths of every bucket created (some may be dropped)
}

func (g *gen) between(r [2]int) int {
	if r[1] <= r[0] {
		return r[0]
	}
	return r[0] + g.r.IntN(r[1]-r[0]+1)
}

func (g *gen) bytes(n int) []byte {
	b := make([]byte, n)
	for i := range b {
		b[i] = byte(g.r.Uint32())
	}
	return b
}

func (g *gen) key() []byte {
	n := g.between(g.rc.KeySize)
	if n < 1 {
		n = 1
	}
	return g.bytes(n)
}

func (g *gen) value() []byte {
	if g.rc.LargePermille > 0 && g.r.IntN(1000) < g.rc.LargePermille {
		return g.bytes(g.between(g.rc.LargeSize))
	}
	return g.bytes(g.between(g.rc.ValueSize))
}

func (g *gen) name(prefix string, i int) []byte {
	if g.rc.BinaryNames {
		return g.bytes(1 + g.r.IntN(12))
	}
	return []byte(fmt.Sprintf("%s-%03d", prefix, i))
}

func lookup(tx *bolt.Tx, path [][]byte) *bolt.Bucket {
	b := tx.Bucket(path[0])
	for _, n := range path[1:] {
		if b == nil {
			return nil
		}
		b = b.Bucket(n)
	}
	return b
}

func extend(path [][]byte, name []byte) [][]byte {
	p := make([][]byte, len(path)+1)
	copy(p, path)
	p[len(path)] = name
	return p
}

func (g *gen) fill(b *bolt.Bucket, n int) (int, error) {
	put := 0
	for i := 0; i < n; i++ {
		k := g.key()
		if b.Bucket(k) != nil {
			continue
		}
		if err := b.Put(k, g.value()); err != nil {
			return put, err
		}
		put++
	}
	return put, nil
}

func (g *gen) createTree(tx *bolt.Tx) (int, error) {
	keys := 0
	type item struct {
		path  [][]byte
		level int
	}
	var queue []item
	for i := 0; i < g.rc.TopBuckets; i++ {
		name := g.name("top", i)
		if _, err := tx.CreateBucketIfNotExists(name); err != nil {
			return keys, err
		}
		queue = append(queue, item{[][]byte{name}, 0})
	}
	for len(queue) > 0 {
		it := queue[0]
		queue = queue[1:]
		b := lookup(tx, it.path)
		g.all = append(g.all, it.path)
		n, err := g.fill(b, g.between(g.rc.Keys))
		keys += n
		if err != nil {
			return keys, err
		}
		if it.level < g.rc.Depth {
			for j := 0; j < g.rc.SubBuckets; j++ {
				name := g.name(fmt.Sprintf("sub%d", it.level+1), j)
				if b.Get(name) != nil {
					continue
				}
				if _, err := b.CreateBucketIfNotExists(name); err != nil {
					return keys, err
				}
				queue = append(queue, item{extend(it.path, name), it.level + 1})
			}
		}
	}
	parents := append([][][]byte(nil), g.all...)
	pick := func() *bolt.Bucket {
		if len(parents) == 0 || g.r.IntN(4) == 0 {
			return nil
		}
		return lookup(tx, parents[g.r.IntN(len(parents))])
	}
	for i := 0; i < g.rc.InlineBuckets; i++ {
		name := g.name("tiny", i)
		var b *bolt.Bucket
		var err error
		if p := pick(); p != nil {
			if p.Get(name) != nil {
				continue
			}
			b, err = p.CreateBucketIfNotExists(name)
		} else {
			b, err = tx.CreateBucketIfNotExists(name)
		}
		if err != nil {
			return keys, err
		}
		for j := 1 + g.r.IntN(3); j > 0; j-- {
			if err := b.Put(g.bytes(1+g.r.IntN(8)), g.bytes(g.r.IntN(24))); err != nil {
				return keys, err
			}
			keys++
		}
	}
	for i := 0; i < g.rc.EmptyBuckets; i++ {
		name := g.name("empty", i)
		var err error
		if p := pick(); p != nil {
			if p.Get(name) != nil {
				continue
			}
			_, err = p.CreateBucketIfNotExists(name)
		} else {
			_, err = tx.CreateBucketIfNotExists(name)
		}
		if err != nil {
			return keys, err
		}
	}
	return keys, nil
}

func (g *gen) mutate(tx *bolt.Tx) (int, error) {
	added := 0
	for _, path := range g.all {
		b := lookup(tx, path)
		if b == nil {
			continue // dropped earlier
		}
		var plain [][]byte
		c := b.Cursor()
		for k, v := c.First(); k != nil; k, v = c.Next() {
			if v != nil {
				plain = append(plain, append([]byte(nil), k...))
			}
		}
		for _, k := range plain {
			switch r := g.r.IntN(100); {
			case r < g.rc.DeletePercent:
				if err := b.Delete(k); err != nil {
					return added, err
				}
				added--
			case r < g.rc.DeletePercent+g.rc.OverwritePercent:
				if err := b.Put(k, g.value()); err != nil {
					return added, err
				}
			}
		}
		n, err := g.fill(b, g.between(g.rc.Keys)*g.rc.AddPercent/100)
		added += n
		if err != nil {
			return added, err
		}
		if g.r.IntN(100) < g.rc.SeqPercent {
			for j := 1 + g.r.IntN(5); j > 0; j-- {
				if _, err := b.NextSequence(); err != nil {
					return added, err
				}
			}
		}
	}
	if len(g.all) > 0 && g.r.IntN(100) < g.rc.DropPercent {
		i := g.r.IntN(len(g.all))
		path := g.all[i]
		if len(path) > 1 {
			if parent := lookup(tx, path[:len(path)-1]); parent != nil && parent.Bucket(path[len(path)-1]) != nil {
				if err := parent.DeleteBucket(path[len(path)-1]); err != nil {
					return added, err
				}
			}
		}
	}
	return added, nil
}

func cmdGenerate(raw json.RawMessage) (any, error) {
	var args generateArgs
	if err := unmarshal(raw, &args); err != nil {
		return nil, err
	}
	rc := &args.Recipe
	if rc.Commits < 1 {
		rc.Commits = 1
	}
	if _, err := os.Stat(args.Path); err == nil {
		return nil, fmt.Errorf("%s already exists", args.Path)
	}
	db, err := bolt.Open(args.Path, 0600, &bolt.Options{
		PageSize: rc.PageSize,
		Timeout:  2 * time.Second,
		NoSync:   true, // test data: durability is not needed, the format is unchanged
	})
	if err != nil {
		return nil, err
	}
	g := &gen{r: rand.New(rand.NewPCG(args.Seed, args.Seed^0x5bd1e9955bd1e995)), rc: rc}
	keys := 0
	err = db.Update(func(tx *bolt.Tx) error {
		n, err := g.createTree(tx)
		keys += n
		return err
	})
	for i := 1; err == nil && i < rc.Commits; i++ {
		err = db.Update(func(tx *bolt.Tx) error {
			n, err := g.mutate(tx)
			keys += n
			return err
		})
	}
	if err != nil {
		db.Close()
		return nil, err
	}
	var res generateResult
	_ = db.View(func(tx *bolt.Tx) error {
		res.TxID = uint64(tx.ID())
		return nil
	})
	res.PageSize = db.Info().PageSize
	if err := db.Close(); err != nil {
		return nil, err
	}
	fi, err := os.Stat(args.Path)
	if err != nil {
		return nil, err
	}
	res.FileSize = fi.Size()
	res.Keys = keys
	res.Buckets = len(g.all) + rc.InlineBuckets + rc.EmptyBuckets
	return res, nil
}

// ---------------------------------------------------------------- cursor

type cursorOp struct {
	Op  string `json:"op"`
	Key string `json:"key"`
}

type cursorArgs struct {
	Path   string     `json:"path"`
	Bucket []string   `json:"bucket"`
	Ops    []cursorOp `json:"ops"`
}

type cursorResult struct {
	Key    *string `json:"key"`
	Value  *string `json:"value"`
	Bucket bool    `json:"bucket"`
}

func hexOrNil(b []byte) *string {
	if b == nil {
		return nil
	}
	s := hex.EncodeToString(b)
	return &s
}

func decodePath(parts []string) ([][]byte, error) {
	path := make([][]byte, len(parts))
	for i, p := range parts {
		b, err := hex.DecodeString(p)
		if err != nil {
			return nil, err
		}
		path[i] = b
	}
	return path, nil
}

func openBucket(tx *bolt.Tx, parts []string) (*bolt.Bucket, error) {
	path, err := decodePath(parts)
	if err != nil {
		return nil, err
	}
	b := tx.Cursor().Bucket()
	for _, n := range path {
		b = b.Bucket(n)
		if b == nil {
			return nil, &linejson.Error{Kind: "nobucket", Text: fmt.Sprintf("no bucket %x", n)}
		}
	}
	return b, nil
}

func cmdCursor(raw json.RawMessage) (any, error) {
	var args cursorArgs
	if err := unmarshal(raw, &args); err != nil {
		return nil, err
	}
	db, err := openRO(args.Path, false)
	if err != nil {
		return nil, err
	}
	defer db.Close()
	var out []cursorResult
	err = db.View(func(tx *bolt.Tx) error {
		b, err := openBucket(tx, args.Bucket)
		if err != nil {
			return err
		}
		c := b.Cursor()
		for _, op := range args.Ops {
			key, err := hex.DecodeString(op.Key)
			if err != nil {
				return err
			}
			var k, v []byte
			switch op.Op {
			case "first":
				k, v = c.First()
			case "last":
				k, v = c.Last()
			case "next":
				k, v = c.Next()
			case "prev":
				k, v = c.Prev()
			case "seek":
				k, v = c.Seek(key)
			case "get":
				out = append(out, cursorResult{Key: hexOrNil(key), Value: hexOrNil(b.Get(key))})
				continue
			case "bucket":
				out = append(out, cursorResult{Key: hexOrNil(key), Bucket: b.Bucket(key) != nil})
				continue
			default:
				return fmt.Errorf("unknown op %q", op.Op)
			}
			out = append(out, cursorResult{Key: hexOrNil(k), Value: hexOrNil(v), Bucket: k != nil && v == nil})
		}
		return nil
	})
	if err != nil {
		return nil, err
	}
	if out == nil {
		out = []cursorResult{}
	}
	return out, nil
}

// ---------------------------------------------------------------- stats

type statsArgs struct {
	Path   string   `json:"path"`
	Bucket []string `json:"bucket"`
}

type statsResult struct {
	BranchPageN       int `json:"branch_page_n"`
	BranchOverflowN   int `json:"branch_overflow_n"`
	LeafPageN         int `json:"leaf_page_n"`
	LeafOverflowN     int `json:"leaf_overflow_n"`
	KeyN              int `json:"key_n"`
	Depth             int `json:"depth"`
	BranchAlloc       int `json:"branch_alloc"`
	BranchInuse       int `json:"branch_inuse"`
	LeafAlloc         int `json:"leaf_alloc"`
	LeafInuse         int `json:"leaf_inuse"`
	BucketN           int `json:"bucket_n"`
	InlineBucketN     int `json:"inline_bucket_n"`
	InlineBucketInuse int `json:"inline_bucket_inuse"`
}

func cmdStats(raw json.RawMessage) (any, error) {
	var args statsArgs
	if err := unmarshal(raw, &args); err != nil {
		return nil, err
	}
	db, err := openRO(args.Path, false)
	if err != nil {
		return nil, err
	}
	defer db.Close()
	var res statsResult
	err = db.View(func(tx *bolt.Tx) error {
		b, err := openBucket(tx, args.Bucket)
		if err != nil {
			return err
		}
		s := b.Stats()
		res = statsResult{
			BranchPageN: s.BranchPageN, BranchOverflowN: s.BranchOverflowN,
			LeafPageN: s.LeafPageN, LeafOverflowN: s.LeafOverflowN,
			KeyN: s.KeyN, Depth: s.Depth,
			BranchAlloc: s.BranchAlloc, BranchInuse: s.BranchInuse,
			LeafAlloc: s.LeafAlloc, LeafInuse: s.LeafInuse,
			BucketN: s.BucketN, InlineBucketN: s.InlineBucketN,
			InlineBucketInuse: s.InlineBucketInuse,
		}
		return nil
	})
	if err != nil {
		return nil, err
	}
	return res, nil
}

// ---------------------------------------------------------------- hold

var held *bolt.DB

type holdArgs struct {
	Path      string `json:"path"`
	TimeoutMS int    `json:"timeout_ms"`
	ReadOnly  bool   `json:"read_only"`
}

type holdResult struct {
	db  *bolt.DB
	err error
}

// pending is the result channel of an open started by hold_start.
var pending chan holdResult

func cmdHold(raw json.RawMessage) (any, error) {
	var args holdArgs
	if err := unmarshal(raw, &args); err != nil {
		return nil, err
	}
	if held != nil || pending != nil {
		return nil, errors.New("a database is already held")
	}
	timeout := 10 * time.Second
	if args.TimeoutMS > 0 {
		timeout = time.Duration(args.TimeoutMS) * time.Millisecond
	}
	db, err := bolt.Open(args.Path, 0600, &bolt.Options{Timeout: timeout, ReadOnly: args.ReadOnly})
	if err != nil {
		return nil, classify(err)
	}
	held = db
	return struct{}{}, nil
}

func cmdHoldStart(raw json.RawMessage) (any, error) {
	var args holdArgs
	if err := unmarshal(raw, &args); err != nil {
		return nil, err
	}
	if held != nil || pending != nil {
		return nil, errors.New("a database is already held")
	}
	ch := make(chan holdResult, 1)
	go func() {
		db, err := bolt.Open(args.Path, 0600, nil)
		ch <- holdResult{db, err}
	}()
	pending = ch
	return struct{}{}, nil
}

type waitResult struct {
	Done bool `json:"done"`
}

func cmdHoldWait(raw json.RawMessage) (any, error) {
	var args holdArgs
	if err := unmarshal(raw, &args); err != nil {
		return nil, err
	}
	if pending == nil {
		return nil, errors.New("no open was started")
	}
	select {
	case r := <-pending:
		pending = nil
		if r.err != nil {
			return nil, classify(r.err)
		}
		held = r.db
		return waitResult{Done: true}, nil
	case <-time.After(time.Duration(args.TimeoutMS) * time.Millisecond):
		return waitResult{Done: false}, nil
	}
}

func cmdRelease(json.RawMessage) (any, error) {
	if held == nil {
		return nil, errors.New("no database is held")
	}
	err := held.Close()
	held = nil
	if err != nil {
		return nil, err
	}
	return struct{}{}, nil
}
