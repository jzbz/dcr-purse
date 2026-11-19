// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

// Package walletexport writes the logical export of a dcrwallet wallet.db
// at schema version 26, format "purse-export 2"
// (docs/adr/0001-logical-export-format.md). It is the body of
// cmd/walletexport, kept here so that cmd/oprunner can export the
// databases its scripts build in the same process.
//
// The export streams: sections are written in registry order as they are
// produced, the large row sets (addresses, blocks, headers, filters,
// transactions, credits, debits) are written row by row in an order
// obtained from their bucket cursors or from a sort of fixed-width keys,
// and the closed-world claims are kept per bucket, so memory grows with
// the number of rows only through those sort keys and the address API's
// answers, never through the rendered text.
package walletexport

import (
	"context"
	"crypto/sha256"
	"encoding/binary"
	"encoding/hex"
	"errors"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"sort"
	"strconv"
	"strings"
	"time"

	"decred.org/dcrwallet/v5/wallet/udb"
	"decred.org/dcrwallet/v5/wallet/walletdb"
	"github.com/decred/dcrd/chaincfg/v3"
	bolt "go.etcd.io/bbolt"
)

// schemaVersion is the only dcrwallet database version the format covers
// (wallet/udb/upgrades.go: DBVersion = 26).
const schemaVersion = 26

// ExportArgs are the arguments of the export command (see cmd/walletexport).
type ExportArgs struct {
	Path     string  `json:"path"`
	Out      string  `json:"out"`
	Network  string  `json:"network"`
	PubPass  *string `json:"pubpass"`
	PrivPass *string `json:"privpass"`
	// AcctPass maps account numbers (decimal strings) to per-account
	// passphrases, for [privcheck].
	AcctPass     map[string]string `json:"acctpass"`
	Chain        string            `json:"chain"` // "full" (default) or "digest"
	AllowUnknown bool              `json:"allow_unknown"`
	// Comments are written as "# " lines after the magic line.
	Comments []string `json:"comments"`
	// Exporter is the @exporter meta (not compared); empty means
	// "tools/oracle/cmd/walletexport".
	Exporter string `json:"-"`
}

// ExportResult is what the export command returns.
type ExportResult struct {
	Version  int               `json:"version"`
	SHA256   string            `json:"sha256"`
	MSHA256  string            `json:"msha256"`
	File     string            `json:"file_sha256"`
	Lines    int               `json:"lines"`
	Sections []SectionSummary  `json:"sections"`
	Counts   map[string]int    `json:"counts"`
	Metas    map[string]string `json:"metas"`
}

func netParams(name string) (*chaincfg.Params, error) {
	switch name {
	case "mainnet":
		return chaincfg.MainNetParams(), nil
	case "testnet3":
		return chaincfg.TestNet3Params(), nil
	case "simnet":
		return chaincfg.SimNetParams(), nil
	case "regnet":
		return chaincfg.RegNetParams(), nil
	}
	return nil, fmt.Errorf("unknown network %q", name)
}

// exporter holds one export run: the read transaction, the API handles
// (nil without the public passphrase), the claims and the section buffers.
type exporter struct {
	tx     *bolt.Tx
	dbtx   walletdb.ReadTx
	params *chaincfg.Params
	mgr    *udb.Manager
	store  *udb.Store
	digest bool
	claims *claims
	out    *exportWriter
	bufs   map[string]*sectionBuf
	// filled by the address manager pass, used by later passes
	acctNums     []uint32
	acctHasPriv  map[uint32]bool     // the account row's sealed xpriv is non-empty
	acctKDF      map[uint32]bool     // the account has its own passphrase
	keyAddrs     map[string]addrInfo // bucket key -> API view
	privKeyAddrs []string            // bucket keys of imported keys with a sealed private key
	// the sealed halves of the parked SLIP0044 account 0 row, when it
	// decodes
	slip0044Pub, slip0044Priv []byte
}

// sec returns the buffer of a section whose producers do not emit in
// order; flush writes it.
func (e *exporter) sec(name string) *sectionBuf {
	b := e.bufs[name]
	if b == nil {
		def := sectionByName[name]
		if def == nil {
			panic("no section " + name)
		}
		b = &sectionBuf{def: def}
		e.bufs[name] = b
	}
	return b
}

// flush writes the buffered section name (empty if nothing was added).
func (e *exporter) flush(name string) {
	e.sec(name).write(e.out)
}

// bad records a row or field that fails a check Go's reader performs.
func (e *exporter) bad(path []string, key []byte, reason string, value []byte) {
	val := vX(value)
	if value == nil {
		val = absent
	}
	e.sec("bad").add("bad", vS([]byte(pathString(path))), vX(key), reason, val)
}

func (e *exporter) orphan(index, ref, val string) {
	e.sec("orphans").add("orphan", index, vS([]byte(ref)), vS([]byte(val)))
}

// bucket returns the bucket at path (nil if missing) and claims every
// bucket entry along it.
func (e *exporter) bucket(path ...string) *bolt.Bucket {
	var b *bolt.Bucket
	for i, p := range path {
		if i == 0 {
			b = e.tx.Bucket([]byte(p))
		} else {
			b = b.Bucket([]byte(p))
		}
		if b == nil {
			return nil
		}
		e.claims.claim(path[:i], []byte(p))
	}
	return b
}

// each iterates over the key/value pairs of b (nested buckets skipped and
// left unclaimed unless the caller claims them), claiming every pair.
func (e *exporter) each(path []string, b *bolt.Bucket, f func(k, v []byte)) {
	if b == nil {
		return
	}
	e.claims.claimAll(path)
	c := b.Cursor()
	for k, v := c.First(); k != nil; k, v = c.Next() {
		if v == nil {
			continue
		}
		f(k, v)
	}
}

func readVersion(tx *bolt.Tx) (uint32, error) {
	meta := tx.Bucket([]byte("meta"))
	if meta == nil {
		return 0, errors.New("no meta bucket: not a unified dcrwallet database")
	}
	v := meta.Get([]byte("ver"))
	if len(v) != 4 {
		return 0, fmt.Errorf("meta/ver has %d bytes", len(v))
	}
	return binary.BigEndian.Uint32(v), nil
}

// Export writes the export of the database at args.Path to args.Out.
func Export(args ExportArgs) (ExportResult, error) {
	res, err := export(args)
	if err != nil {
		return ExportResult{}, err
	}
	return *res, nil
}

func export(args ExportArgs) (res *ExportResult, err error) {
	if args.Network == "" {
		return nil, errors.New("network is required")
	}
	params, err := netParams(args.Network)
	if err != nil {
		return nil, err
	}
	chainMode := args.Chain
	if chainMode == "" {
		chainMode = "full"
	}
	if chainMode != "full" && chainMode != "digest" {
		return nil, fmt.Errorf("chain must be full or digest, not %q", chainMode)
	}
	if args.Out == "" {
		return nil, errors.New("out is required")
	}
	for _, c := range args.Comments {
		for i := 0; i < len(c); i++ {
			if c[i] < 0x20 || c[i] > 0x7e {
				return nil, errors.New("comments must be printable ASCII")
			}
		}
		if strings.HasSuffix(c, " ") {
			return nil, errors.New("comments may not end in a space")
		}
	}

	db, err := bolt.Open(args.Path, 0600, &bolt.Options{ReadOnly: true, Timeout: 2 * time.Second})
	if err != nil {
		return nil, err
	}
	defer db.Close()

	ctx := context.Background()
	rodb := roDB{db}
	var ver uint32
	if err := db.View(func(tx *bolt.Tx) error {
		ver, err = readVersion(tx)
		return err
	}); err != nil {
		return nil, err
	}
	if ver != schemaVersion {
		return nil, fmt.Errorf("database version %d: the format covers version %d only", ver, schemaVersion)
	}

	var mgr *udb.Manager
	var store *udb.Store
	decrypt := "none"
	if args.PubPass != nil {
		mgr, store, err = udb.Open(ctx, rodb, params, []byte(*args.PubPass))
		if err != nil {
			return nil, fmt.Errorf("udb.Open: %w", err)
		}
		defer mgr.Close()
		decrypt = "public"
	}
	derived, privcheck := "0", "0"
	if mgr != nil {
		derived = "1"
		if args.PrivPass != nil {
			privcheck = "1"
		}
	}
	metas := [][2]string{
		{"network", args.Network},
		{"chain", chainMode},
		{"decrypt", decrypt},
		{"derived", derived},
		{"privcheck", privcheck},
		{"origin", "dcrwallet-udb/" + strconv.Itoa(schemaVersion)},
		{"exporter", exporterName(args.Exporter)},
	}

	tx, err := db.Begin(false)
	if err != nil {
		return nil, err
	}
	defer tx.Rollback()

	// The export is written to a temporary file beside out and renamed
	// over it only when complete, so a failed export leaves no partial
	// file under the requested name.
	f, err := os.CreateTemp(filepath.Dir(args.Out), filepath.Base(args.Out)+".*.tmp")
	if err != nil {
		return nil, err
	}
	tmp := f.Name()
	// CreateTemp makes the file 0600; an export holds no secret.
	if err := f.Chmod(0o644); err != nil {
		f.Close()
		os.Remove(tmp)
		return nil, err
	}
	defer func() {
		if f != nil {
			f.Close()
		}
		if err != nil {
			os.Remove(tmp)
		}
	}()
	fileHash := sha256.New()
	counter := &lineCounter{}

	e := &exporter{
		tx:          tx,
		dbtx:        roTx{tx},
		params:      params,
		mgr:         mgr,
		store:       store,
		digest:      chainMode == "digest",
		claims:      newClaims(),
		out:         newExportWriter(io.MultiWriter(f, fileHash, counter), args.Comments, metas),
		bufs:        make(map[string]*sectionBuf),
		keyAddrs:    make(map[string]addrInfo),
		acctHasPriv: make(map[uint32]bool),
		acctKDF:     make(map[uint32]bool),
	}
	defer func() {
		if r := recover(); r != nil {
			ef, ok := r.(exportFault)
			if !ok {
				panic(r)
			}
			res, err = nil, ef.err
		}
	}()

	// meta/ver is consumed by the version check above.
	e.bucket("meta")
	e.claims.claim([]string{"meta"}, []byte("ver"))

	// Producers run in registry order; each writes its sections, streamed
	// or from a buffer, before the next starts. [orphans] and [bad] are
	// filled along the way and written after the last raw producer.
	if err := e.addressManager(); err != nil {
		return nil, err
	}
	if err := e.txStore(); err != nil {
		return nil, err
	}
	e.aux()
	e.flush("orphans")
	e.flush("bad")

	walk := e.claims.walk(tx)
	walk.emit(e.sec("residue"), e.sec("unknown"))
	if len(walk.unknown) > 0 && !args.AllowUnknown {
		u := walk.unknown[0]
		return nil, fmt.Errorf("unclassified entry %s %s (%d in all)", u[0], u[1], len(walk.unknown))
	}
	e.flush("unknown")
	if mgr != nil {
		e.derived()
	}
	e.flush("derived")
	if privcheck == "1" {
		e.privcheck([]byte(*args.PrivPass), args.AcctPass)
	}
	e.flush("privcheck")
	e.flush("residue")

	sums, d, md, err := e.out.finish()
	if err != nil {
		return nil, err
	}
	if err := f.Close(); err != nil {
		return nil, err
	}
	f = nil
	if err := os.Rename(tmp, args.Out); err != nil {
		return nil, err
	}
	res = &ExportResult{
		Version:  formatVersion,
		SHA256:   d,
		MSHA256:  md,
		File:     hex.EncodeToString(fileHash.Sum(nil)),
		Lines:    counter.n,
		Sections: sums,
		Counts:   make(map[string]int),
		Metas:    make(map[string]string),
	}
	for _, s := range sums {
		res.Counts[s.Name] = s.Records
	}
	for _, m := range metas {
		res.Metas[m[0]] = m[1]
	}
	return res, nil
}

type lineCounter struct{ n int }

func (c *lineCounter) Write(p []byte) (int, error) {
	for _, b := range p {
		if b == '\n' {
			c.n++
		}
	}
	return len(p), nil
}

// SectionsResult is what the sections command returns: the magic line, the
// section registry, the enumerated values of every e field and the metas
// the eof digests cover.
type SectionsResult struct {
	Magic         string              `json:"magic"`
	Version       int                 `json:"version"`
	Sections      []SectionInfo       `json:"sections"`
	Enums         map[string][]string `json:"enums"`
	ComparedMetas []string            `json:"compared_metas"`
}

// SectionInfo describes one section of the registry.
type SectionInfo struct {
	Name     string   `json:"name"`
	Version  int      `json:"version"`
	Compared bool     `json:"compared"`
	Types    []string `json:"types"`
}

func exporterName(name string) string {
	if name == "" {
		return "tools/oracle/cmd/walletexport"
	}
	return name
}

// Sections returns the format version and the section registry.
func Sections() SectionsResult {
	res := SectionsResult{Magic: magic, Version: formatVersion, Enums: enumValues, ComparedMetas: comparedMetas}
	for _, s := range schema {
		info := SectionInfo{Name: s.name, Version: s.version, Compared: s.compared}
		for _, t := range s.types {
			info.Types = append(info.Types, t.decl())
		}
		res.Sections = append(res.Sections, info)
	}
	return res
}

// recovered runs f, turning a panic into an error.
func recovered(f func() error) (err error) {
	defer func() {
		if r := recover(); r != nil {
			err = fmt.Errorf("panic: %v", r)
		}
	}()
	return f()
}

func sortedU32(m map[uint32]bool) []uint32 {
	out := make([]uint32, 0, len(m))
	for k := range m {
		out = append(out, k)
	}
	sort.Slice(out, func(i, j int) bool { return out[i] < out[j] })
	return out
}
