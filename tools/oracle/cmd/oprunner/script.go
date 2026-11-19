// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

package main

import (
	"bytes"
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"maps"
	"os"
	"path/filepath"
	"runtime"
	"runtime/debug"
	"sort"
	"strconv"
	"strings"

	"decred.org/dcrwallet/v5/errors"
	"decred.org/dcrwallet/v5/wallet"
	"decred.org/dcrwallet/v5/wallet/walletdb"
	"github.com/decred/dcrd/chaincfg/chainhash"
	"github.com/decred/dcrd/chaincfg/v3"
	"github.com/decred/dcrd/txscript/v4/stdaddr"
	"github.com/jzbz/dcr-purse/tools/oracle/cmd/oprunner/internal/rand"
	"github.com/jzbz/dcr-purse/tools/oracle/cmd/oprunner/internal/seam"
	"github.com/jzbz/dcr-purse/tools/oracle/cmd/oprunner/internal/udb"
	"github.com/jzbz/dcr-purse/tools/oracle/internal/walletexport"

	_ "decred.org/dcrwallet/v5/wallet/drivers/bdb"
)

// scriptFormat is the value of a script's "format" member
// (docs/adr/0002-operation-script-format.md).
const scriptFormat = "purse-opscript 1"

// resultsFormat is the first line of results.jsonl.
const resultsFormat = "purse-opresults 1"

// Script is an operation script.
type Script struct {
	Format string            `json:"format"`
	Name   string            `json:"name"`
	Setup  Setup             `json:"setup"`
	Export *ExportOptions    `json:"export"`
	Ops    []json.RawMessage `json:"ops"`
}

// Setup creates the wallet and fixes the injected sources.
type Setup struct {
	Network string `json:"network"`
	// Seed (hex) creates the wallet with udb.Initialize; Xpub (a key
	// spec) creates a watching-only wallet with udb.InitializeWatchOnly.
	// Exactly one is given.
	Seed     *string         `json:"seed"`
	Xpub     json.RawMessage `json:"xpub"`
	PubPass  string          `json:"pubpass"`
	PrivPass string          `json:"privpass"`
	// Rand is the 32-byte key (hex) of the injected randomness stream.
	Rand string `json:"rand"`
	// Clock is the Unix second the injected clock starts at.
	Clock int64 `json:"clock"`
	// NCPU is the injected runtime.NumCPU() (default 1).
	NCPU *int `json:"ncpu"`
	// ChainTime is the base of block timestamps (default Clock).
	ChainTime *int64 `json:"chain_time"`
}

// ExportOptions controls the exports the runner writes.
type ExportOptions struct {
	// Chain is "full" (default) or "digest".
	Chain string `json:"chain"`
	// Privcheck (default true) passes the private passphrase and the
	// account passphrases the script set to the exporter.
	Privcheck *bool `json:"privcheck"`
}

type runArgs struct {
	Script Script `json:"script"`
	// Dir is an empty or missing directory the runner may write: the
	// database, the exports and results.jsonl.
	Dir string `json:"dir"`
}

// runner is the state of one script run.
type runner struct {
	ctx       context.Context
	dir       string
	params    *chaincfg.Params
	network   string
	dbPath    string
	db        walletdb.DB
	mgr       *udb.Manager
	store     *udb.Store
	pubPass   string
	privPass  *string
	acctPass  map[uint32]string
	chainTime int64
	nonce     uint32
	export    ExportOptions

	txs    map[string]*udb.TxRecord
	blocks map[string]chainhash.Hash
	addrs  map[string]stdaddr.Address

	checkpoints map[string]bool

	// session is the open wallet-package session (wallet.open to
	// wallet.close), nil outside one.
	session *wallet.Wallet
}

// bookkeeping is the runner state that names or unlocks database state:
// the passphrases the database holds (for rt.reopen and the exporter) and
// the labels of blocks and imported addresses. It is transactional with
// the database transaction that changed it: saved before every operation
// and group, and restored when that operation or group fails, so that a
// rolled-back change leaves no trace here (a rolled-back block label then
// names nothing, and using it is the unknown-label script fault).
// Transaction labels (fixtures, not database state), the extend counter
// and the injected clock are not part of it.
type bookkeeping struct {
	pubPass  string
	privPass *string
	acctPass map[uint32]string
	blocks   map[string]chainhash.Hash
	addrs    map[string]stdaddr.Address
}

func (r *runner) save() bookkeeping {
	return bookkeeping{
		pubPass:  r.pubPass,
		privPass: r.privPass,
		acctPass: maps.Clone(r.acctPass),
		blocks:   maps.Clone(r.blocks),
		addrs:    maps.Clone(r.addrs),
	}
}

func (r *runner) restore(b bookkeeping) {
	r.pubPass = b.pubPass
	r.privPass = b.privPass
	r.acctPass = b.acctPass
	r.blocks = b.blocks
	r.addrs = b.addrs
}

// scriptError is a fault of the script itself (an unknown operation, a
// missing or malformed argument, an unknown label). It ends the run: it
// is never recorded as an operation result.
type scriptError struct{ msg string }

func (e *scriptError) Error() string { return e.msg }

func scriptErrorf(format string, a ...any) error {
	return &scriptError{fmt.Sprintf(format, a...)}
}

// runnerFault is a defect of the runner itself: a panic whose innermost
// attributable frame is the runner's own code (see panicSite). Like a
// script fault it ends the run; it is never recorded as an operation
// result, so that a fault in the runner's encoding cannot pass for a
// dcrwallet panic.
type runnerFault struct{ msg string }

func (e *runnerFault) Error() string { return e.msg }

// fatal reports whether err ends the run instead of being an operation
// result.
func fatal(err error) bool {
	var se *scriptError
	var rf *runnerFault
	return errors.As(err, &se) || errors.As(err, &rf)
}

// panicError is a Go panic inside dcrwallet's code (the copied udb and the
// packages it calls). walletdb.Update and View roll their transaction back
// on a panic before it reaches the recover here (Update's deferred
// Rollback, wallet/walletdb/interface.go:224-252), so the run continues on
// the last committed state. It is recorded with the kind "Panic", which is
// none of dcrwallet's kinds; ADR-0012 (D10) has dcr-purse return an error
// where Go panics.
type panicError struct{ text string }

func (e *panicError) Error() string { return "panic: " + e.text }

// canonError is an error whose text the runner made canonical (see
// canonPointer). It wraps the original error, so that errors.Is still
// finds the original's kind.
type canonError struct {
	kind, text string
	err        error
}

func (e *canonError) Error() string { return e.text }
func (e *canonError) Unwrap() error { return e.err }

// recovered runs f and turns a panic into a *panicError when it arose in
// dcrwallet's code, or into a *runnerFault when it arose in the runner's.
func recovered(f func() error) (err error) {
	defer func() {
		p := recover()
		if p == nil {
			return
		}
		site, inUpstream := panicSite()
		if !inUpstream {
			err = &runnerFault{fmt.Sprintf("panic in the runner at %s: %v\n%s", site, p, debug.Stack())}
			return
		}
		err = &panicError{canonPanic(fmt.Sprint(p))}
	}()
	return f()
}

// boundsPanics are the runtime's bounds-check panics. The indices the
// runtime prints after them depend on the architecture the binary was
// compiled for: slicing a nil block header in extractBlockHeaderHeight
// (wallet/udb/txdb.go) reports "[128:0]" on amd64 and "[0:0]" on arm64,
// with Go 1.27.4 on both.
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

// internalPrefix is the import path prefix of the runner's copies.
const internalPrefix = "github.com/jzbz/dcr-purse/tools/oracle/cmd/oprunner/internal/"

// upstreamCopies are the copied dcrwallet and dcrd packages under
// internal/ (internal/seam is the runner's own).
var upstreamCopies = []string{"udb.", "snacl.", "kdf.", "compat.", "rand."}

// dcrwalletPrefix is the import path prefix of dcrwallet's own packages,
// linked unmodified: the wallet package of the wallet session, the
// unmodified udb under it, walletdb and its bdb driver.
const dcrwalletPrefix = "decred.org/dcrwallet/v5/"

// panicSite finds where the panic being recovered arose. It must be called
// from the deferred function that recovers. Walking outward from the
// panicking frame (the first frame past runtime.gopanic), runtime and
// dependency frames are skipped until the first frame that is either
// dcrwallet's code, a copied package or one of dcrwallet's own packages
// (upstream: the panic is dcrwallet behaviour, even when it surfaced in a
// dependency dcrwallet called), or the runner's own code, package main or
// internal/seam (a runner fault, even when it surfaced in a dependency the
// runner called, or in a callback udb ran). Returns that frame's function
// and whether it is upstream code.
func panicSite() (string, bool) {
	pcs := make([]uintptr, 128)
	n := runtime.Callers(1, pcs)
	frames := runtime.CallersFrames(pcs[:n])
	pastPanic := false
	for {
		f, more := frames.Next()
		switch {
		case !pastPanic:
			pastPanic = f.Function == "runtime.gopanic"
		case strings.HasPrefix(f.Function, "main.") ||
			strings.HasPrefix(f.Function, internalPrefix+"seam."):
			return f.Function, false
		case strings.HasPrefix(f.Function, dcrwalletPrefix):
			return f.Function, true
		case strings.HasPrefix(f.Function, internalPrefix):
			rest := strings.TrimPrefix(f.Function, internalPrefix)
			for _, pkg := range upstreamCopies {
				if strings.HasPrefix(rest, pkg) {
					return f.Function, true
				}
			}
		}
		if !more {
			return "an unattributable frame", false
		}
	}
}

// Kind names, indexed by errors.Kind (decred.org/dcrwallet/v5/errors).
var kindNames = []string{
	"Other", "Bug", "Invalid", "Permission", "IO", "Exist", "NotExist",
	"Encoding", "Crypto", "Locked", "Passphrase", "Seed", "WatchingOnly",
	"InsufficientBalance", "ScriptFailure", "Policy", "Consensus",
	"DoubleSpend", "Protocol", "NoPeers", "Deployment",
}

// daemonSeparator is the separator dcrwallet's daemon puts between nested
// errors: its init sets errors.Separator to ":: " (dcrwallet.go:36-39),
// replacing the package default ":\n\t" (errors/errors.go:27). Error texts
// are byte-exact under ADR-0011, so the runner renders every error as the
// daemon does. One assignment covers the copies under internal/ and the
// unmodified udb alike: all of them import decred.org/dcrwallet/v5/errors.
const daemonSeparator = ":: "

func init() {
	errors.Separator = daemonSeparator
}

// errorValue is the canonical encoding of an error: its full text (with
// the daemon's separator) and the kind errors.As finds in it ("Other" when
// there is none).
func errorValue(err error) map[string]any {
	if p, ok := err.(*panicError); ok {
		return map[string]any{"kind": "Panic", "text": p.text}
	}
	if c, ok := err.(*canonError); ok {
		return map[string]any{"kind": c.kind, "text": c.text}
	}
	kind := "Other"
	var k errors.Kind
	if errors.As(err, &k) && int(k) >= 0 && int(k) < len(kindNames) {
		kind = kindNames[k]
	}
	return map[string]any{"kind": kind, "text": err.Error()}
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
	return nil, scriptErrorf("unknown network %q", name)
}

// opRecord is one line of the results.
type opRecord struct {
	I     int        `json:"i"`
	Op    string     `json:"op"`
	OK    any        `json:"ok,omitempty"`
	Err   any        `json:"err,omitempty"`
	Drawn uint64     `json:"drawn,omitempty"`
	Ops   []opRecord `json:"ops,omitempty"`
	// Tolerated marks an error a group member's "tolerate" list matched:
	// the group went on.
	Tolerated bool `json:"tolerated,omitempty"`
}

// MarshalJSON writes "ok" even when the result is null, so that every
// record has exactly one of ok and err.
func (r opRecord) MarshalJSON() ([]byte, error) {
	m := map[string]any{"i": r.I, "op": r.Op}
	if r.Err != nil {
		m["err"] = r.Err
	} else {
		m["ok"] = r.OK
	}
	if r.Tolerated {
		m["tolerated"] = true
	}
	if r.Drawn != 0 {
		m["drawn"] = r.Drawn
	}
	if r.Ops != nil {
		m["ops"] = r.Ops
	}
	return marshalCanonical(m)
}

// marshalCanonical is encoding/json with HTML escaping off: object keys
// sorted (Go sorts map keys), no insignificant space.
func marshalCanonical(v any) ([]byte, error) {
	var buf bytes.Buffer
	enc := json.NewEncoder(&buf)
	enc.SetEscapeHTML(false)
	if err := enc.Encode(v); err != nil {
		return nil, err
	}
	return bytes.TrimSuffix(buf.Bytes(), []byte("\n")), nil
}

type runResult struct {
	Format  string         `json:"format"`
	Results []opRecord     `json:"results"`
	Exports map[string]any `json:"exports"`
	// ResultsSHA256 is the SHA-256 of results.jsonl.
	ResultsSHA256 string `json:"results_sha256"`
	Drawn         uint64 `json:"drawn"`
}

func cmdRun(raw json.RawMessage) (any, error) {
	var args runArgs
	if err := unmarshalStrict(raw, &args); err != nil {
		return nil, err
	}
	return run(&args.Script, args.Dir)
}

func unmarshalStrict(raw json.RawMessage, v any) error {
	if len(raw) == 0 {
		return scriptErrorf("missing args")
	}
	dec := json.NewDecoder(bytes.NewReader(raw))
	dec.DisallowUnknownFields()
	if err := dec.Decode(v); err != nil {
		return scriptErrorf("%v", err)
	}
	return nil
}

func run(s *Script, dir string) (res *runResult, err error) {
	if s.Format != scriptFormat {
		return nil, scriptErrorf("script format %q, want %q", s.Format, scriptFormat)
	}
	if dir == "" {
		return nil, scriptErrorf("dir is required")
	}
	if entries, err := os.ReadDir(dir); err == nil && len(entries) != 0 {
		return nil, scriptErrorf("%s is not empty", dir)
	}
	if err := os.MkdirAll(dir, 0o700); err != nil {
		return nil, err
	}
	r, err := setup(s, dir)
	if err != nil {
		return nil, err
	}
	defer func() {
		if r.mgr != nil {
			r.mgr.Close()
		}
		if r.db != nil {
			r.db.Close()
		}
	}()

	res = &runResult{Format: resultsFormat, Exports: make(map[string]any)}
	for i, raw := range s.Ops {
		rec, exp, err := r.top(i, raw)
		if err != nil {
			return nil, fmt.Errorf("op %d: %w", i, err)
		}
		res.Results = append(res.Results, rec)
		if exp != nil {
			res.Exports[exp.name] = exp.summary
		}
	}
	if r.session != nil {
		return nil, scriptErrorf("the script ends with a wallet session open (wallet.close missing)")
	}
	final, err := r.exportNow("final")
	if err != nil {
		return nil, fmt.Errorf("final export: %w", err)
	}
	res.Exports["final"] = final
	res.Drawn = rand.Drawn()

	// results.jsonl: the format line, then one record per top-level op.
	var buf bytes.Buffer
	buf.WriteString(resultsFormat + "\n")
	for _, rec := range res.Results {
		b, err := marshalCanonical(rec)
		if err != nil {
			return nil, err
		}
		buf.Write(b)
		buf.WriteByte('\n')
	}
	if err := os.WriteFile(filepath.Join(dir, "results.jsonl"), buf.Bytes(), 0o600); err != nil {
		return nil, err
	}
	sum := sha256.Sum256(buf.Bytes())
	res.ResultsSHA256 = hex.EncodeToString(sum[:])
	return res, nil
}

func setup(s *Script, dir string) (*runner, error) {
	st := &s.Setup
	params, err := netParams(st.Network)
	if err != nil {
		return nil, err
	}
	key, err := hex.DecodeString(st.Rand)
	if err != nil || len(key) != 32 {
		return nil, scriptErrorf("setup.rand must be 32 bytes of hex")
	}
	ncpu := 1
	if st.NCPU != nil {
		ncpu = *st.NCPU
	}
	if ncpu < 1 || ncpu > 1024 {
		return nil, scriptErrorf("setup.ncpu out of range")
	}
	var k [32]byte
	copy(k[:], key)
	rand.Seed(k)
	seam.SetNow(st.Clock)
	seam.SetNumCPU(ncpu)

	r := &runner{
		ctx:         context.Background(),
		dir:         dir,
		params:      params,
		network:     st.Network,
		dbPath:      filepath.Join(dir, "wallet.db"),
		pubPass:     st.PubPass,
		acctPass:    make(map[uint32]string),
		chainTime:   st.Clock,
		txs:         make(map[string]*udb.TxRecord),
		blocks:      make(map[string]chainhash.Hash),
		addrs:       make(map[string]stdaddr.Address),
		checkpoints: make(map[string]bool),
	}
	if st.ChainTime != nil {
		r.chainTime = *st.ChainTime
	}
	if s.Export != nil {
		r.export = *s.Export
	}
	if r.export.Chain == "" {
		r.export.Chain = "full"
	}

	db, err := walletdb.Create("bdb", r.dbPath)
	if err != nil {
		return nil, err
	}
	r.db = db
	switch {
	case st.Seed != nil && st.Xpub == nil:
		seed, err := hex.DecodeString(*st.Seed)
		if err != nil {
			return nil, scriptErrorf("setup.seed: %v", err)
		}
		err = udb.Initialize(r.ctx, db, params, seed, []byte(st.PubPass), []byte(st.PrivPass))
		if err != nil {
			return nil, fmt.Errorf("udb.Initialize: %w", err)
		}
		priv := st.PrivPass
		r.privPass = &priv
	case st.Seed == nil && st.Xpub != nil:
		xpub, err := r.keySpec(st.Xpub, false)
		if err != nil {
			return nil, err
		}
		err = udb.InitializeWatchOnly(r.ctx, db, params, xpub.String(), []byte(st.PubPass))
		if err != nil {
			return nil, fmt.Errorf("udb.InitializeWatchOnly: %w", err)
		}
	default:
		return nil, scriptErrorf("setup needs exactly one of seed and xpub")
	}
	r.mgr, r.store, err = udb.Open(r.ctx, db, params, []byte(st.PubPass))
	if err != nil {
		return nil, fmt.Errorf("udb.Open: %w", err)
	}
	return r, nil
}

// reopen closes the wallet and opens it again as dcrwallet does at
// start-up (walletdb.Open with the bdb driver, then udb.Open with the
// public passphrase), so that every in-memory state of the Manager is
// rebuilt from the database: locks, unlocked account keys, caches.
func (r *runner) reopen() error {
	if err := r.mgr.Close(); err != nil {
		return err
	}
	r.mgr, r.store = nil, nil
	if err := r.db.Close(); err != nil {
		return err
	}
	r.db = nil
	db, err := walletdb.Open("bdb", r.dbPath)
	if err != nil {
		return err
	}
	r.db = db
	r.mgr, r.store, err = udb.Open(r.ctx, db, r.params, []byte(r.pubPass))
	return err
}

type exportDone struct {
	name    string
	summary map[string]any
}

// top runs one top-level operation.
func (r *runner) top(i int, raw json.RawMessage) (opRecord, *exportDone, error) {
	a, name, err := parseOp(raw)
	if err != nil {
		return opRecord{}, nil, err
	}
	if _, ok := a.m["tolerate"]; ok {
		return opRecord{}, nil, scriptErrorf("%s: tolerate is valid only on a member of update or view", name)
	}
	if err := r.sessionAllows(name); err != nil {
		return opRecord{}, nil, err
	}
	switch name {
	case "update", "view":
		rec, err := r.group(i, name, a)
		return rec, nil, err
	case "rt.checkpoint":
		cp, err := a.str("name")
		if err != nil {
			return opRecord{}, nil, err
		}
		if err := a.done(); err != nil {
			return opRecord{}, nil, err
		}
		if cp == "final" || cp == "" || strings.ContainsAny(cp, `/\.`) || r.checkpoints[cp] {
			return opRecord{}, nil, scriptErrorf("bad or repeated checkpoint name %q", cp)
		}
		r.checkpoints[cp] = true
		before := rand.Drawn()
		sum, err := r.exportNow(cp)
		if err != nil {
			return opRecord{}, nil, err
		}
		rec := opRecord{I: i, Op: name, OK: sum, Drawn: rand.Drawn() - before}
		return rec, &exportDone{cp, sum}, nil
	case "rt.reopen":
		if err := a.done(); err != nil {
			return opRecord{}, nil, err
		}
		before := rand.Drawn()
		if err := r.reopen(); err != nil {
			return opRecord{}, nil, fmt.Errorf("rt.reopen: %w", err)
		}
		return opRecord{I: i, Op: name, Drawn: rand.Drawn() - before}, nil, nil
	case "wallet.open":
		before := rand.Drawn()
		opErr := recovered(func() error { return r.walletOpen(a) })
		if fatal(opErr) {
			return opRecord{}, nil, opErr
		}
		if err := a.done(); err != nil {
			return opRecord{}, nil, err
		}
		rec := opRecord{I: i, Op: name, Drawn: rand.Drawn() - before}
		if opErr != nil {
			rec.Err = errorValue(opErr)
		}
		return rec, nil, nil
	case "wallet.close":
		if err := a.done(); err != nil {
			return opRecord{}, nil, err
		}
		before := rand.Drawn()
		if err := r.walletClose(); err != nil {
			return opRecord{}, nil, fmt.Errorf("wallet.close: %w", err)
		}
		return opRecord{I: i, Op: name, Drawn: rand.Drawn() - before}, nil, nil
	}
	def, ok := ops[name]
	if !ok {
		return opRecord{}, nil, scriptErrorf("unknown op %q", name)
	}
	before := rand.Drawn()
	saved := r.save()
	var out any
	opErr := recovered(func() error {
		switch def.kind {
		case noTx, walletSession:
			var err error
			out, err = def.fn(r, &opCtx{}, a)
			return err
		case readTx:
			return walletdb.View(r.ctx, r.db, func(tx walletdb.ReadTx) error {
				var err error
				out, err = def.fn(r, &opCtx{rtx: tx}, a)
				return err
			})
		default:
			return walletdb.Update(r.ctx, r.db, func(tx walletdb.ReadWriteTx) error {
				var err error
				out, err = def.fn(r, &opCtx{rtx: tx, wtx: tx}, a)
				return err
			})
		}
	})
	if opErr != nil {
		// Whatever the operation registered went with its transaction
		// (a commit failure included).
		r.restore(saved)
	}
	if fatal(opErr) {
		return opRecord{}, nil, opErr
	}
	if err := a.done(); err != nil {
		return opRecord{}, nil, err
	}
	rec := opRecord{I: i, Op: name, Drawn: rand.Drawn() - before}
	if opErr != nil {
		rec.Err = errorValue(opErr)
	} else {
		rec.OK = out
	}
	return rec, nil, nil
}

// sessionAllows checks an operation against the wallet session: inside
// one, only the session's operations, rt.checkpoint and the clock are
// allowed (the copied udb's Manager is closed, and the wallet package's
// Manager caches state of its own); outside one, the session's operations
// other than wallet.open are faults.
func (r *runner) sessionAllows(name string) error {
	inSession := strings.HasPrefix(name, "wallet.") && name != "wallet.open"
	if r.session == nil {
		if inSession {
			return scriptErrorf("%s needs an open wallet session (wallet.open)", name)
		}
		return nil
	}
	switch {
	case inSession, name == "rt.checkpoint", name == "rt.clock_set", name == "rt.clock_advance":
		return nil
	}
	return scriptErrorf("%s is not allowed inside a wallet session (wallet.close first)", name)
}

// tolerateKinds reads a group member's optional "tolerate": dcrwallet
// error kinds (by name; not Other, which errors.Is never matches) whose
// errors the member may return without ending the group.
func tolerateKinds(a *opArgs) ([]errors.Kind, error) {
	if _, ok := a.m["tolerate"]; !ok {
		return nil, nil
	}
	var names []string
	if err := a.decode("tolerate", &names); err != nil {
		return nil, err
	}
	if len(names) == 0 {
		return nil, scriptErrorf("%s: tolerate is empty", a.op)
	}
	kinds := make([]errors.Kind, 0, len(names))
	for _, n := range names {
		k := -1
		for i := 1; i < len(kindNames); i++ {
			if kindNames[i] == n {
				k = i
			}
		}
		if k < 0 {
			return nil, scriptErrorf("%s: tolerate: %q is not a dcrwallet error kind other than Other", a.op, n)
		}
		kinds = append(kinds, errors.Kind(k))
	}
	return kinds, nil
}

// tolerated reports whether err matches one of kinds, as a Go caller's
// errors.Is(err, errors.<Kind>) does. A recovered panic never matches.
func tolerated(err error, kinds []errors.Kind) bool {
	if _, ok := err.(*panicError); ok {
		return false
	}
	for _, k := range kinds {
		if errors.Is(err, k) {
			return true
		}
	}
	return false
}

// group runs the operations of an "update" or "view" in one database
// transaction, as a Go caller doing several udb calls in one
// walletdb.Update or View does. A member's error ends the group and rolls
// the transaction back, unless the member's "tolerate" names its kind, as
// a Go caller that checks errors.Is(err, errors.Exist) and goes on in the
// same transaction does (wallet/chainntfns.go:46, 450-455, 486-494,
// 505-509; createtx.go:640): the error is recorded, marked tolerated, and
// the group continues. The group's record holds the records of the
// operations that ran and, when it rolled back, the ending error.
func (r *runner) group(i int, name string, a *opArgs) (opRecord, error) {
	var raws []json.RawMessage
	if err := a.decode("ops", &raws); err != nil {
		return opRecord{}, err
	}
	if err := a.done(); err != nil {
		return opRecord{}, err
	}
	rec := opRecord{I: i, Op: name, Ops: []opRecord{}}
	before := rand.Drawn()
	body := func(c *opCtx) error {
		for j, raw := range raws {
			sa, sname, err := parseOp(raw)
			if err != nil {
				return err
			}
			def, ok := ops[sname]
			if !ok {
				return scriptErrorf("unknown op %q in %s", sname, name)
			}
			if def.kind == walletSession {
				return scriptErrorf("%s cannot be a member of %s", sname, name)
			}
			if def.kind == writeTx && c.wtx == nil {
				return scriptErrorf("%s needs a read-write transaction, not a view", sname)
			}
			tol, err := tolerateKinds(sa)
			if err != nil {
				return err
			}
			b := rand.Drawn()
			var out any
			err = recovered(func() error {
				var err error
				out, err = def.fn(r, c, sa)
				return err
			})
			if fatal(err) {
				return err
			}
			if err := sa.done(); err != nil {
				return err
			}
			sub := opRecord{I: j, Op: sname, Drawn: rand.Drawn() - b}
			if err != nil {
				sub.Err = errorValue(err)
				if tolerated(err, tol) {
					sub.Tolerated = true
					rec.Ops = append(rec.Ops, sub)
					continue
				}
				rec.Ops = append(rec.Ops, sub)
				return err
			}
			sub.OK = out
			rec.Ops = append(rec.Ops, sub)
		}
		return nil
	}
	saved := r.save()
	err := recovered(func() error {
		if name == "view" {
			return walletdb.View(r.ctx, r.db, func(tx walletdb.ReadTx) error {
				return body(&opCtx{rtx: tx})
			})
		}
		return walletdb.Update(r.ctx, r.db, func(tx walletdb.ReadWriteTx) error {
			return body(&opCtx{rtx: tx, wtx: tx})
		})
	})
	if err != nil {
		// The transaction rolled back: so does everything its members
		// registered.
		r.restore(saved)
	}
	if fatal(err) {
		return opRecord{}, err
	}
	rec.Drawn = rand.Drawn() - before
	if err != nil {
		rec.Err = errorValue(err)
	} else {
		rec.OK = nil
	}
	return rec, nil
}

// exportNow snapshots the open database (walletdb.DB.Copy: one read
// transaction, so the snapshot is consistent) and writes its logical
// export with tools/oracle/internal/walletexport, the code of
// cmd/walletexport. The runner's own handles stay open, so the export
// changes no in-memory state of the script's wallet. The exporter opens
// the snapshot with dcrwallet's unmodified udb, whose draws come from the
// real dcrd/crypto/rand and never from the injected stream.
func (r *runner) exportNow(name string) (map[string]any, error) {
	snap := filepath.Join(r.dir, name+".db")
	f, err := os.OpenFile(snap, os.O_CREATE|os.O_EXCL|os.O_WRONLY, 0o600)
	if err != nil {
		return nil, err
	}
	if err := r.db.Copy(f); err != nil {
		f.Close()
		return nil, err
	}
	if err := f.Close(); err != nil {
		return nil, err
	}
	pub := r.pubPass
	args := walletexport.ExportArgs{
		Path:     snap,
		Out:      filepath.Join(r.dir, name+".pe"),
		Network:  r.network,
		PubPass:  &pub,
		Chain:    r.export.Chain,
		Exporter: "tools/oracle/cmd/oprunner",
	}
	if r.export.Privcheck == nil || *r.export.Privcheck {
		if r.privPass != nil {
			priv := *r.privPass
			args.PrivPass = &priv
		}
		if len(r.acctPass) > 0 {
			args.AcctPass = make(map[string]string)
			for acct, pass := range r.acctPass {
				args.AcctPass[strconv.FormatUint(uint64(acct), 10)] = pass
			}
		}
	}
	res, err := walletexport.Export(args)
	if err != nil {
		// The exporter opens the snapshot with the passphrases the
		// runner's bookkeeping holds; a failure here is a bookkeeping
		// fault or an exporter fault, never an operation result.
		return nil, fmt.Errorf("logical export %q: %w", name, err)
	}
	// The masked digests mask only "~" fields, and the "@" fields
	// (received and birthday times, ADR-0001 section 8) follow the
	// injected clock: they are marked "~c".
	secs := make([]any, 0, len(res.Sections))
	for _, s := range res.Sections {
		secs = append(secs, map[string]any{
			"name": s.Name, "records": s.Records,
			"sha256~cr": s.SHA256, "msha256~c": s.MSHA256,
		})
	}
	return map[string]any{
		"file":           name + ".pe",
		"sha256~cr":      res.SHA256,
		"msha256~c":      res.MSHA256,
		"file_sha256~cr": res.File,
		"lines":          res.Lines,
		"sections":       secs,
	}, nil
}

// sortedKeys returns the keys of m in increasing order.
func sortedKeys[V any](m map[string]V) []string {
	keys := make([]string, 0, len(m))
	for k := range m {
		keys = append(keys, k)
	}
	sort.Strings(keys)
	return keys
}
