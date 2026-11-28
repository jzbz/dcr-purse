// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

// Command storebench is the dcrwallet baseline for the store spike
// (spikes/store-redb, decision D2). It links dcrwallet's own wallet/udb over
// bbolt at the parity pin, ingests a generated wallet history through udb's
// real API with the call sequence the wallet uses (wallet/chainntfns.go:
// extendMainChain and processTransactionRecord), times the access paths the
// spike compares, and writes a canonical export for the correctness
// cross-check. It speaks the linejson protocol.
//
// Commands (paths are absolute; every time is in nanoseconds):
//
//	setup   {db, workload, seed, gap}
//	        Creates a simnet wallet at db (which must not exist) from the
//	        32-byte hex seed, creates the workload's accounts and records
//	        every address the workload names, then compares each recorded
//	        address's HASH160 with the workload's. Returns {genesis,
//	        accounts, addresses, mismatches, elapsed}.
//	ingest  {db, workload, gap}
//	        Applies the workload's operations, one walletdb.Update per write
//	        transaction the workload marks. Returns timing summaries.
//	bench   {db, ...} (see benchArgs)
//	        Runs the timed queries. Returns their summaries.
//	open    {db}
//	        Times walletdb.Open plus udb.Open once (the caller drops the
//	        file's page cache first for a cold figure).
//	export  {db, out, probes, scans, ranges, by_account, select_*}
//	        Writes the canonical export (format in the spike's README) to
//	        out and returns {sha256, lines}.
//	backup  {db, out, writer}
//	        Times walletdb's hot copy, with or without a concurrent writer.
//	cursor  {db, keys, walks}
//	        Creates a bbolt database at db holding keys in one bucket and
//	        returns what each walk (a seek, then n/p steps) returns, so the
//	        spike's cursor adapter is compared with bbolt itself.
//	headers {db, n, batch, filter_len, seed}
//	        Creates a wallet at db and attaches n generated headers with
//	        committed filters of about filter_len bytes through
//	        ExtendMainChain, batch headers per write transaction (SPV
//	        initial sync). Returns timings and bytes written.
//	hot     {db, ...} (see hotArgs in hot.go)
//	        Times the hot paths of brief section 6 Phase 2 that the
//	        measurement of purse-store against dcrwallet compares
//	        (crates/purse-udb/tests/measure.rs): balances, ranges over
//	        blocks, address and account lookups, and one-address commits.
//	        Returns the timings and, per query, a check value the Rust side
//	        must give too.
//
// The workload format is the spike's (spikes/store-redb/src/workload.rs),
// with one operation kind the spike does not write: 5, the 32-byte hash of
// an unmined transaction to abandon (wallet.AbandonTransaction: TxDetails,
// refused if mined, then RemoveUnconfirmed), a write transaction of its own.
package main

import (
	"bufio"
	"bytes"
	"context"
	"crypto/sha256"
	"encoding/binary"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"os"
	"slices"
	"sort"
	"strings"
	"time"

	dcrerrors "decred.org/dcrwallet/v5/errors"
	_ "decred.org/dcrwallet/v5/wallet/drivers/bdb"
	"decred.org/dcrwallet/v5/wallet/txrules"
	"decred.org/dcrwallet/v5/wallet/udb"
	"decred.org/dcrwallet/v5/wallet/walletdb"
	"github.com/decred/dcrd/blockchain/stake/v5"
	"github.com/decred/dcrd/chaincfg/chainhash"
	"github.com/decred/dcrd/chaincfg/v3"
	"github.com/decred/dcrd/dcrutil/v4"
	gcs2 "github.com/decred/dcrd/gcs/v4"
	"github.com/decred/dcrd/gcs/v4/blockcf2"
	"github.com/decred/dcrd/hdkeychain/v3"
	"github.com/decred/dcrd/txscript/v4/stdaddr"
	"github.com/decred/dcrd/txscript/v4/stdscript"
	"github.com/decred/dcrd/wire"
	"github.com/jzbz/dcr-purse/tools/oracle/internal/linejson"
)

var (
	waddrmgrKey = []byte("waddrmgr")
	wtxmgrKey   = []byte("wtxmgr")
	pubPass     = []byte("public")
	privPass    = []byte("private")
	params      = chaincfg.SimNetParams()
)

func main() {
	linejson.Serve(map[string]linejson.Handler{
		"setup":   cmdSetup,
		"ingest":  cmdIngest,
		"bench":   cmdBench,
		"open":    cmdOpen,
		"export":  cmdExport,
		"backup":  cmdBackup,
		"cursor":  cmdCursor,
		"headers": cmdHeaders,
		"hot":     cmdHot,
	})
}

// ioWriteBytes is this process's write_bytes from /proc/self/io: the bytes
// it caused to be sent to the storage layer (dirtied in the page cache), or
// -1 where the file does not exist.
func ioWriteBytes() int64 {
	b, err := os.ReadFile("/proc/self/io")
	if err != nil {
		return -1
	}
	for _, line := range strings.Split(string(b), "\n") {
		if v, ok := strings.CutPrefix(line, "write_bytes: "); ok {
			var n int64
			fmt.Sscan(v, &n)
			return n
		}
	}
	return -1
}

type backupArgs struct {
	DB     string `json:"db"`
	Out    string `json:"out"`
	Writer bool   `json:"writer"`
}

// cmdBackup times walletdb's hot copy (bbolt Tx.Copy inside a read
// transaction, which is what dcrwallet's backup paths use) into out, then
// fsyncs it. With writer, a goroutine keeps recording one new address of
// account 0's internal branch per write transaction while the copy runs.
func cmdBackup(args json.RawMessage) (any, error) {
	var a backupArgs
	if err := unmarshal(args, &a); err != nil {
		return nil, err
	}
	w, err := openWallet(a.DB)
	if err != nil {
		return nil, err
	}
	defer w.close()
	ctx := context.Background()
	var commits int
	var werr error
	stop := make(chan struct{})
	done := make(chan struct{})
	if a.Writer {
		var next uint32
		err = walletdb.View(ctx, w.db, func(dbtx walletdb.ReadTx) error {
			ns := dbtx.ReadBucket(waddrmgrKey)
			return w.manager.ForEachAccountAddress(ns, 0, func(ma udb.ManagedAddress) error {
				if pk, ok := ma.(udb.ManagedPubKeyAddress); ok && ma.Internal() && pk.Index() >= next {
					next = pk.Index() + 1
				}
				return nil
			})
		})
		if err != nil {
			return nil, err
		}
		go func() {
			defer close(done)
			for {
				select {
				case <-stop:
					return
				default:
				}
				child := next
				next++
				werr = walletdb.Update(ctx, w.db, func(dbtx walletdb.ReadWriteTx) error {
					ns := dbtx.ReadWriteBucket(waddrmgrKey)
					return w.manager.SyncAccountToAddrIndex(ns, 0, child, 1)
				})
				if werr != nil {
					return
				}
				commits++
			}
		}()
		time.Sleep(20 * time.Millisecond)
	} else {
		close(done)
	}
	f, err := os.Create(a.Out)
	if err != nil {
		return nil, err
	}
	t0 := time.Now()
	err = w.db.Copy(f)
	if err == nil {
		err = f.Sync()
	}
	d := time.Since(t0).Nanoseconds()
	close(stop)
	<-done
	f.Close()
	if err == nil {
		err = werr
	}
	if err != nil {
		return nil, err
	}
	fi, err := os.Stat(a.Out)
	if err != nil {
		return nil, err
	}
	return map[string]any{"elapsed": d, "size": fi.Size(), "writer_commits": commits}, nil
}

type cursorWalk struct {
	Seek  string `json:"seek"`
	Steps string `json:"steps"`
}

type cursorArgs struct {
	DB    string       `json:"db"`
	Keys  []string     `json:"keys"`
	Walks []cursorWalk `json:"walks"`
}

// cmdCursor records what bbolt's cursor returns for each walk over a bucket
// holding exactly keys: the seek's key, then one key (or "-") per step.
func cmdCursor(args json.RawMessage) (any, error) {
	var a cursorArgs
	if err := unmarshal(args, &a); err != nil {
		return nil, err
	}
	if _, err := os.Stat(a.DB); err == nil {
		return nil, fmt.Errorf("%s exists", a.DB)
	}
	db, err := walletdb.Create("bdb", a.DB)
	if err != nil {
		return nil, err
	}
	defer os.Remove(a.DB)
	defer db.Close()
	ctx := context.Background()
	bucket := []byte("t")
	err = walletdb.Update(ctx, db, func(dbtx walletdb.ReadWriteTx) error {
		b, err := dbtx.CreateTopLevelBucket(bucket)
		if err != nil {
			return err
		}
		for _, kh := range a.Keys {
			k, err := hex.DecodeString(kh)
			if err != nil {
				return err
			}
			if err := b.Put(k, []byte{}); err != nil {
				return err
			}
		}
		return nil
	})
	if err != nil {
		return nil, err
	}
	out := make([][]string, len(a.Walks))
	err = walletdb.View(ctx, db, func(dbtx walletdb.ReadTx) error {
		b := dbtx.ReadBucket(bucket)
		for i, wk := range a.Walks {
			k, err := hex.DecodeString(wk.Seek)
			if err != nil {
				return err
			}
			c := b.ReadCursor()
			r, _ := c.Seek(k)
			got := []string{hexOrDash(r)}
			for _, s := range wk.Steps {
				var kk []byte
				if s == 'n' {
					kk, _ = c.Next()
				} else {
					kk, _ = c.Prev()
				}
				got = append(got, hexOrDash(kk))
			}
			c.Close()
			out[i] = got
		}
		return nil
	})
	if err != nil {
		return nil, err
	}
	return map[string]any{"walks": out}, nil
}

type headersArgs struct {
	DB        string `json:"db"`
	N         int    `json:"n"`
	Batch     int    `json:"batch"`
	FilterLen int    `json:"filter_len"`
	Seed      string `json:"seed"`
}

// cmdHeaders measures header-plus-filter ingest: what SPV initial sync and
// a rescan from scratch write before any transaction (ExtendMainChain per
// block, a batch per write transaction, as ChainSwitch writes it). Header
// and filter construction is outside the timed region.
func cmdHeaders(args json.RawMessage) (any, error) {
	var a headersArgs
	if err := unmarshal(args, &a); err != nil {
		return nil, err
	}
	seed, err := hex.DecodeString(a.Seed)
	if err != nil {
		return nil, err
	}
	if _, err := os.Stat(a.DB); err == nil {
		return nil, fmt.Errorf("%s exists", a.DB)
	}
	ctx := context.Background()
	db, err := walletdb.Create("bdb", a.DB)
	if err != nil {
		return nil, err
	}
	if err := udb.Initialize(ctx, db, params, seed, pubPass, privPass); err != nil {
		db.Close()
		return nil, err
	}
	db.Close()
	w, err := openWallet(a.DB)
	if err != nil {
		return nil, err
	}
	defer w.close()
	var prev chainhash.Hash
	walletdb.View(ctx, w.db, func(dbtx walletdb.ReadTx) error {
		prev, _ = w.store.MainChainTip(dbtx)
		return nil
	})
	rng := newSplitMix(binary.LittleEndian.Uint64(seed))
	// Entries per filter: each costs about B+1 bits (Golomb-Rice coding),
	// plus the filter's leading count.
	entries := a.FilterLen * 8 / (int(blockcf2.B) + 1)
	var batches []int64
	var filterBytes int64
	io0 := ioWriteBytes()
	for start := 1; start <= a.N; start += a.Batch {
		n := min(a.Batch, a.N-start+1)
		hdrs := make([]wire.BlockHeader, n)
		hashes := make([]chainhash.Hash, n)
		filters := make([]*gcs2.FilterV2, n)
		for i := range n {
			h := &hdrs[i]
			h.Version = 1
			h.PrevBlock = prev
			rng.fill(h.MerkleRoot[:])
			rng.fill(h.StakeRoot[:])
			h.VoteBits = 1
			h.Voters = 5
			h.PoolSize = 64
			h.Bits = 0x207fffff
			h.SBits = 100_000_000
			h.Height = uint32(start + i)
			h.Size = 1000
			h.Timestamp = time.Unix(1_700_000_000+int64(start+i)*300, 0)
			h.Nonce = uint32(rng.next())
			hashes[i] = h.BlockHash()
			prev = hashes[i]
			data := make([][]byte, entries)
			for j := range data {
				data[j] = make([]byte, 32)
				rng.fill(data[j])
			}
			f, err := gcs2.NewFilterV2(blockcf2.B, blockcf2.M, blockcf2.Key(&h.MerkleRoot), data)
			if err != nil {
				return nil, err
			}
			filters[i] = f
			filterBytes += int64(len(f.Bytes()))
		}
		t0 := time.Now()
		err := walletdb.Update(ctx, w.db, func(dbtx walletdb.ReadWriteTx) error {
			ns := dbtx.ReadWriteBucket(wtxmgrKey)
			for i := range hdrs {
				if err := w.store.ExtendMainChain(ns, &hdrs[i], &hashes[i], filters[i]); err != nil {
					return err
				}
			}
			return nil
		})
		if err != nil {
			return nil, err
		}
		batches = append(batches, time.Since(t0).Nanoseconds())
	}
	io1 := ioWriteBytes()
	var total int64
	for _, d := range batches {
		total += d
	}
	fi, err := os.Stat(a.DB)
	if err != nil {
		return nil, err
	}
	return map[string]any{
		"n": a.N, "batch": a.Batch, "elapsed": total, "batches": summarize(batches),
		"avg_filter_bytes": float64(filterBytes) / float64(max(a.N, 1)),
		"io_write_bytes":   io1 - io0, "file_size": fi.Size(),
	}, nil
}

// splitMix is SplitMix64, as the spike's generator uses.
type splitMix struct{ s uint64 }

func newSplitMix(seed uint64) *splitMix { return &splitMix{s: seed} }

func (r *splitMix) next() uint64 {
	r.s += 0x9e3779b97f4a7c15
	z := r.s
	z = (z ^ (z >> 30)) * 0xbf58476d1ce4e5b9
	z = (z ^ (z >> 27)) * 0x94d049bb133111eb
	return z ^ (z >> 31)
}

func (r *splitMix) fill(b []byte) {
	for i := 0; i < len(b); i += 8 {
		var w [8]byte
		binary.LittleEndian.PutUint64(w[:], r.next())
		copy(b[i:], w[:])
	}
}

func unmarshal(args json.RawMessage, v any) error {
	if len(args) == 0 {
		return errors.New("missing args")
	}
	return json.Unmarshal(args, v)
}

// ---------------------------------------------------------------------------
// Workload

const (
	opSyncBlock = 1
	opUnmined   = 2
	opTipBlock  = 3
	opReorg     = 4
	opAbandon   = 5
)

type addrRange struct {
	account, branch uint32
	hashes          [][20]byte
}

type wlOp struct {
	kind        byte
	commit      bool
	unpublished bool
	rollbackTo  uint32
	hash        chainhash.Hash
	header      wire.BlockHeader
	txs         []*wire.MsgTx
}

type workload struct {
	accounts uint32
	ranges   []addrRange
	ops      []wlOp
}

type wlReader struct {
	r   *bufio.Reader
	err error
}

func (r *wlReader) u8() byte {
	var b [1]byte
	r.read(b[:])
	return b[0]
}

func (r *wlReader) u32() uint32 {
	var b [4]byte
	r.read(b[:])
	return binary.LittleEndian.Uint32(b[:])
}

func (r *wlReader) read(b []byte) {
	if r.err != nil {
		return
	}
	_, r.err = io.ReadFull(r.r, b)
}

func (r *wlReader) tx() *wire.MsgTx {
	n := r.u32()
	if r.err != nil {
		return nil
	}
	b := make([]byte, n)
	r.read(b)
	if r.err != nil {
		return nil
	}
	tx := new(wire.MsgTx)
	if err := tx.Deserialize(bytes.NewReader(b)); err != nil {
		r.err = err
		return nil
	}
	return tx
}

// readWorkload reads the file; with headerOnly, it stops after the address
// ranges.
func readWorkload(path string, headerOnly bool) (*workload, error) {
	f, err := os.Open(path)
	if err != nil {
		return nil, err
	}
	defer f.Close()
	r := &wlReader{r: bufio.NewReaderSize(f, 1<<20)}
	var magic [8]byte
	r.read(magic[:])
	if r.err == nil && string(magic[:]) != "PURSEWL1" {
		return nil, errors.New("not a workload file")
	}
	w := &workload{accounts: r.u32()}
	nranges := r.u32()
	for range nranges {
		if r.err != nil {
			break
		}
		ar := addrRange{account: r.u32(), branch: r.u32()}
		n := r.u32()
		ar.hashes = make([][20]byte, n)
		for i := range ar.hashes {
			r.read(ar.hashes[i][:])
		}
		w.ranges = append(w.ranges, ar)
	}
	if headerOnly || r.err != nil {
		return w, r.err
	}
	nops := r.u32()
	w.ops = make([]wlOp, 0, nops)
	for range nops {
		if r.err != nil {
			break
		}
		op := wlOp{kind: r.u8()}
		switch op.kind {
		case opSyncBlock, opTipBlock:
			op.commit = r.u8() != 0
			var hb [180]byte
			r.read(hb[:])
			if r.err == nil {
				r.err = op.header.Deserialize(bytes.NewReader(hb[:]))
			}
			ntx := r.u32()
			for range ntx {
				if r.err != nil {
					break
				}
				op.txs = append(op.txs, r.tx())
			}
		case opUnmined:
			op.unpublished = r.u8() != 0
			op.commit = true
			op.txs = []*wire.MsgTx{r.tx()}
		case opReorg:
			op.rollbackTo = r.u32()
		case opAbandon:
			op.commit = true
			r.read(op.hash[:])
		default:
			return nil, fmt.Errorf("bad op kind %d", op.kind)
		}
		w.ops = append(w.ops, op)
	}
	return w, r.err
}

// ---------------------------------------------------------------------------
// Opening

type wallet struct {
	db      walletdb.DB
	manager *udb.Manager
	store   *udb.Store
}

func openWallet(path string) (*wallet, error) {
	db, err := walletdb.Open("bdb", path)
	if err != nil {
		return nil, err
	}
	m, s, err := udb.Open(context.Background(), db, params, pubPass)
	if err != nil {
		db.Close()
		return nil, err
	}
	return &wallet{db: db, manager: m, store: s}, nil
}

func (w *wallet) close() { w.db.Close() }

// ---------------------------------------------------------------------------
// setup

type setupArgs struct {
	DB       string `json:"db"`
	Workload string `json:"workload"`
	Seed     string `json:"seed"`
}

func cmdSetup(args json.RawMessage) (any, error) {
	var a setupArgs
	if err := unmarshal(args, &a); err != nil {
		return nil, err
	}
	start := time.Now()
	seed, err := hex.DecodeString(a.Seed)
	if err != nil {
		return nil, err
	}
	wl, err := readWorkload(a.Workload, true)
	if err != nil {
		return nil, err
	}
	if _, err := os.Stat(a.DB); err == nil {
		return nil, fmt.Errorf("%s exists", a.DB)
	}
	ctx := context.Background()
	db, err := walletdb.Create("bdb", a.DB)
	if err != nil {
		return nil, err
	}
	if err := udb.Initialize(ctx, db, params, seed, pubPass, privPass); err != nil {
		db.Close()
		return nil, err
	}
	db.Close()
	w, err := openWallet(a.DB)
	if err != nil {
		return nil, err
	}
	defer w.close()

	var genesis chainhash.Hash
	var mismatches, addresses int
	err = walletdb.Update(ctx, w.db, func(dbtx walletdb.ReadWriteTx) error {
		ns := dbtx.ReadWriteBucket(waddrmgrKey)
		if err := w.manager.Unlock(ns, privPass); err != nil {
			return err
		}
		for acct := uint32(1); acct < wl.accounts; acct++ {
			n, err := w.manager.NewAccount(ns, fmt.Sprintf("acct-%d", acct))
			if err != nil {
				return err
			}
			if n != acct {
				return fmt.Errorf("new account %d, want %d", n, acct)
			}
		}
		for _, r := range wl.ranges {
			if len(r.hashes) == 0 {
				continue
			}
			err := w.manager.SyncAccountToAddrIndex(ns, r.account,
				uint32(len(r.hashes)-1), r.branch)
			if err != nil {
				return err
			}
		}
		// Compare every recorded address with the workload's.
		for _, r := range wl.ranges {
			for i, h := range r.hashes {
				addresses++
				addr, err := stdaddr.NewAddressPubKeyHashEcdsaSecp256k1V0(h[:], params)
				if err != nil {
					return err
				}
				ma, err := w.manager.Address(ns, addr)
				if err != nil {
					mismatches++
					continue
				}
				pk, ok := ma.(udb.ManagedPubKeyAddress)
				branch := uint32(0)
				if ma.Internal() {
					branch = 1
				}
				if !ok || ma.Account() != r.account || branch != r.branch ||
					pk.Index() != uint32(i) {
					mismatches++
				}
			}
		}
		w.manager.Lock()
		genesis, _ = w.store.MainChainTip(dbtx)
		return nil
	})
	if err != nil {
		return nil, err
	}
	return map[string]any{
		"genesis":    hex.EncodeToString(genesis[:]),
		"accounts":   wl.accounts,
		"addresses":  addresses,
		"mismatches": mismatches,
		"elapsed":    time.Since(start).Nanoseconds(),
	}, nil
}

// ---------------------------------------------------------------------------
// ingest

type ingestArgs struct {
	DB       string `json:"db"`
	Workload string `json:"workload"`
	Gap      uint32 `json:"gap"`
}

type ingestCounters struct {
	syncNs       int64
	credits      int
	commitments  int
	lookups      int
	lookupMisses int
}

// markUsedAddress is wallet.markUsedAddress (wallet/addresses.go:718).
func (w *wallet) markUsedAddress(dbtx walletdb.ReadWriteTx, ma udb.ManagedAddress, gap uint32, c *ingestCounters) error {
	ns := dbtx.ReadWriteBucket(waddrmgrKey)
	account := ma.Account()
	if err := w.manager.MarkUsed(dbtx, ma.Address()); err != nil {
		return err
	}
	if account == udb.ImportedAddrAccount {
		return nil
	}
	props, err := w.manager.AccountProperties(ns, account)
	if err != nil {
		return err
	}
	lastUsed := props.LastUsedExternalIndex
	branch := udb.ExternalBranch
	if ma.Internal() {
		lastUsed = props.LastUsedInternalIndex
		branch = udb.InternalBranch
	}
	start := time.Now()
	err = w.manager.SyncAccountToAddrIndex(ns, account,
		min(hdkeychain.HardenedKeyStart-1, lastUsed+gap), branch)
	c.syncNs += time.Since(start).Nanoseconds()
	return err
}

// processTx is wallet.processTransactionRecord (wallet/chainntfns.go:438)
// without what does not touch the database (outpoint locks, notifications)
// and without the multisig paths, which the workload never reaches.
func (w *wallet) processTx(dbtx walletdb.ReadWriteTx, rec *udb.TxRecord, header *wire.BlockHeader,
	blockMeta *udb.BlockMeta, gap uint32, c *ingestCounters) error {

	addrmgrNs := dbtx.ReadWriteBucket(waddrmgrKey)
	txmgrNs := dbtx.ReadWriteBucket(wtxmgrKey)
	var err error
	if header == nil {
		err = w.store.InsertMemPoolTx(dbtx, rec)
		if dcrerrors.Is(err, dcrerrors.Exist) {
			return nil
		}
	} else {
		err = w.store.InsertMinedTx(dbtx, rec, &blockMeta.Hash)
	}
	if err != nil {
		return err
	}
	for _, input := range rec.MsgTx.TxIn {
		_ = stdscript.IsMultiSigSigScript(0, input.SignatureScript)
	}
	for i, output := range rec.MsgTx.TxOut {
		class, addrs := stdscript.ExtractAddrs(output.Version, output.PkScript, params)
		if class == stdscript.STNonStandard {
			continue
		}
		if subClass, isStake := txrules.StakeSubScriptType(class); isStake {
			class = subClass
		}
		_ = class
		isTicketCommit := rec.TxType == stake.TxTypeSStx && i%2 == 1
		if isTicketCommit {
			addr, err := stake.AddrFromSStxPkScrCommitment(output.PkScript, params)
			if err != nil {
				continue
			}
			addrs = []stdaddr.Address{addr}
		} else if output.Value == 0 {
			continue
		}
		for _, addr := range addrs {
			c.lookups++
			ma, err := w.manager.Address(addrmgrNs, addr)
			if dcrerrors.Is(err, dcrerrors.NotExist) {
				c.lookupMisses++
				continue
			}
			if err != nil {
				return err
			}
			if isTicketCommit {
				err = w.store.AddTicketCommitment(txmgrNs, rec, uint32(i), ma.Account())
				c.commitments++
			} else {
				err = w.store.AddCredit(dbtx, rec, blockMeta, uint32(i), ma.Internal(), ma.Account())
				c.credits++
			}
			if err != nil {
				return err
			}
			if err := w.markUsedAddress(dbtx, ma, gap, c); err != nil {
				return err
			}
		}
	}
	if rec.TxType == stake.TxTypeSSGen || rec.TxType == stake.TxTypeSSRtx {
		if err := w.store.RedeemTicketCommitments(txmgrNs, rec, blockMeta); err != nil {
			return err
		}
	}
	return nil
}

// extendMainChain is wallet.extendMainChain (wallet/chainntfns.go:29)
// without checkpoints and notifications.
func (w *wallet) extendMainChain(dbtx walletdb.ReadWriteTx, header *wire.BlockHeader,
	txs []*wire.MsgTx, gap uint32, c *ingestCounters) (*chainhash.Hash, error) {

	txmgrNs := dbtx.ReadWriteBucket(wtxmgrKey)
	hash := header.BlockHash()
	filter, err := gcs2.FromBytesV2(blockcf2.B, blockcf2.M, nil)
	if err != nil {
		return nil, err
	}
	err = w.store.ExtendMainChain(txmgrNs, header, &hash, filter)
	if err != nil && !dcrerrors.Is(err, dcrerrors.Exist) {
		return nil, err
	}
	meta, err := w.store.GetBlockMetaForHash(txmgrNs, &hash)
	if err != nil {
		return nil, err
	}
	for _, tx := range txs {
		rec, err := udb.NewTxRecordFromMsgTx(tx, time.Now())
		if err != nil {
			return nil, err
		}
		if err := w.processTx(dbtx, rec, header, &meta, gap, c); err != nil {
			return nil, err
		}
	}
	return &hash, nil
}

// abandon is wallet.AbandonTransaction (wallet/wallet.go:5071) inside the
// caller's write transaction: the transaction's details, refused if mined in
// the main chain, then RemoveUnconfirmed.
func (w *wallet) abandon(dbtx walletdb.ReadWriteTx, hash *chainhash.Hash) error {
	ns := dbtx.ReadWriteBucket(wtxmgrKey)
	details, err := w.store.TxDetails(ns, hash)
	if err != nil {
		return fmt.Errorf("abandon %v: %w", hash, err)
	}
	if details.Block.Height != -1 {
		return fmt.Errorf("abandon %v: mined in main chain", hash)
	}
	return w.store.RemoveUnconfirmed(ns, &details.MsgTx, hash)
}

// hasStake reports whether a block holds a stake transaction.
func hasStake(txs []*wire.MsgTx) bool {
	for _, tx := range txs {
		if stake.DetermineTxType(tx) != stake.TxTypeRegular {
			return true
		}
	}
	return false
}

func cmdIngest(args json.RawMessage) (any, error) {
	var a ingestArgs
	if err := unmarshal(args, &a); err != nil {
		return nil, err
	}
	wl, err := readWorkload(a.Workload, false)
	if err != nil {
		return nil, err
	}
	w, err := openWallet(a.DB)
	if err != nil {
		return nil, err
	}
	defer w.close()
	ctx := context.Background()

	var c ingestCounters
	var syncTxns, syncBlocks, txs, pruned int
	var syncNs int64
	var tipBlock, tipBlockStake, unmined, unminedRemove, reorg []int64
	io0 := ioWriteBytes()
	start := time.Now()
	for i := 0; i < len(wl.ops); {
		// One write transaction: the ops up to and including the next one
		// marked commit.
		j := i
		for j < len(wl.ops) && !wl.ops[j].commit {
			j++
		}
		if j == len(wl.ops) {
			j--
		}
		group := wl.ops[i : j+1]
		t0 := time.Now()
		err := walletdb.Update(ctx, w.db, func(dbtx walletdb.ReadWriteTx) error {
			var last *chainhash.Hash
			for k := range group {
				op := &group[k]
				txs += len(op.txs)
				if op.kind == opReorg {
					// ChainSwitch: Rollback, then the new chain's blocks
					// in the same transaction (wallet/chainntfns.go:160).
					if _, err := w.store.Rollback(dbtx, int32(op.rollbackTo)); err != nil {
						return fmt.Errorf("rollback to %d: %w", op.rollbackTo, err)
					}
					continue
				}
				if op.kind == opAbandon {
					if err := w.abandon(dbtx, &op.hash); err != nil {
						return err
					}
					continue
				}
				if op.kind == opUnmined {
					rec, err := udb.NewTxRecordFromMsgTx(op.txs[0], time.Now())
					if err != nil {
						return err
					}
					rec.Unpublished = op.unpublished
					if err := w.processTx(dbtx, rec, nil, nil, a.Gap, &c); err != nil {
						return err
					}
					continue
				}
				h, err := w.extendMainChain(dbtx, &op.header, op.txs, a.Gap, &c)
				if err != nil {
					return fmt.Errorf("block %d: %w", op.header.Height, err)
				}
				last = h
			}
			if last != nil {
				if err := w.store.UpdateProcessedTxsBlockMarker(dbtx, last); err != nil {
					return err
				}
				// ChainSwitch's last step (wallet/chainntfns.go:255), with
				// the ticket price 0 it passes when the next one is unknown
				// (no unmined ticket is pruned); its error is returned here
				// where the wallet logs it.
				hashes, err := w.store.PruneUnmined(dbtx, 0)
				if err != nil {
					return err
				}
				pruned += len(hashes)
			}
			return nil
		})
		if err != nil {
			return nil, err
		}
		d := time.Since(t0).Nanoseconds()
		switch group[0].kind {
		case opSyncBlock:
			syncTxns++
			syncBlocks += len(group)
			syncNs += d
		case opTipBlock:
			tipBlock = append(tipBlock, d)
			if hasStake(group[0].txs) {
				tipBlockStake = append(tipBlockStake, d)
			}
		case opUnmined:
			unmined = append(unmined, d)
		case opAbandon:
			unminedRemove = append(unminedRemove, d)
		case opReorg:
			reorg = append(reorg, d)
		}
		i = j + 1
	}
	elapsed := time.Since(start).Nanoseconds()
	return map[string]any{
		"elapsed":         elapsed,
		"io_write_bytes":  ioWriteBytes() - io0,
		"reorg":           summarize(reorg),
		"ops":             len(wl.ops),
		"txs":             txs,
		"sync_txns":       syncTxns,
		"sync_blocks":     syncBlocks,
		"sync_ns":         syncNs,
		"tip_block":       summarize(tipBlock),
		"tip_block_stake": summarize(tipBlockStake),
		"unmined":         summarize(unmined),
		"unmined_remove":  summarize(unminedRemove),
		"pruned":          pruned,
		"addr_sync_ns":    c.syncNs,
		"credits":         c.credits,
		"commitments":     c.commitments,
		"lookups":         c.lookups,
		"lookup_misses":   c.lookupMisses,
	}, nil
}

// ---------------------------------------------------------------------------
// Timing helpers

type summary struct {
	N     int   `json:"n"`
	Total int64 `json:"total"`
	Min   int64 `json:"min"`
	P50   int64 `json:"p50"`
	P90   int64 `json:"p90"`
	P99   int64 `json:"p99"`
	Max   int64 `json:"max"`
}

// summarize uses the nearest-rank percentile, as the Rust side does.
func summarize(d []int64) summary {
	if len(d) == 0 {
		return summary{}
	}
	s := slices.Clone(d)
	slices.Sort(s)
	rank := func(p int) int64 {
		i := (p*len(s) + 99) / 100
		if i < 1 {
			i = 1
		}
		return s[i-1]
	}
	var total int64
	for _, v := range s {
		total += v
	}
	return summary{N: len(s), Total: total, Min: s[0], P50: rank(50), P90: rank(90),
		P99: rank(99), Max: s[len(s)-1]}
}

// repeat runs f at least minReps times and until budget has elapsed, at
// most maxReps times, timing each run.
func repeat(minReps, maxReps int, budget time.Duration, f func() error) (summary, error) {
	var d []int64
	start := time.Now()
	for len(d) < maxReps && (len(d) < minReps || time.Since(start) < budget) {
		t0 := time.Now()
		if err := f(); err != nil {
			return summary{}, err
		}
		d = append(d, time.Since(t0).Nanoseconds())
	}
	return summarize(d), nil
}

// ---------------------------------------------------------------------------
// open

type openArgs struct {
	DB string `json:"db"`
}

func cmdOpen(args json.RawMessage) (any, error) {
	var a openArgs
	if err := unmarshal(args, &a); err != nil {
		return nil, err
	}
	t0 := time.Now()
	w, err := openWallet(a.DB)
	if err != nil {
		return nil, err
	}
	d := time.Since(t0).Nanoseconds()
	// The first query after the open, so a cold figure includes the reads
	// the open itself defers.
	t1 := time.Now()
	err = walletdb.View(context.Background(), w.db, func(dbtx walletdb.ReadTx) error {
		_, err := w.store.AccountBalances(dbtx, 1)
		return err
	})
	first := time.Since(t1).Nanoseconds()
	w.close()
	if err != nil {
		return nil, err
	}
	return map[string]any{"open": d, "first_balance": first}, nil
}

// ---------------------------------------------------------------------------
// bench

type rangeSpec struct {
	Name  string `json:"name"`
	Begin int32  `json:"begin"`
	End   int32  `json:"end"`
	// Limit stops the walk once this many transactions were reported (0:
	// no limit).
	Limit int `json:"limit"`
}

type byAccountSpec struct {
	Name    string `json:"name"`
	Account uint32 `json:"account"`
	Limit   int    `json:"limit"`
}

type benchArgs struct {
	DB         string          `json:"db"`
	MinReps    int             `json:"min_reps"`
	MaxReps    int             `json:"max_reps"`
	BudgetMs   int             `json:"budget_ms"`
	MinConfs   []int32         `json:"minconfs"`
	Accounts   []uint32        `json:"accounts"`
	Targets    []int64         `json:"targets"`
	Ranges     []rangeSpec     `json:"ranges"`
	ByAccount  []byAccountSpec `json:"by_account"`
	Scripts    []string        `json:"scripts"`
	NextAddr   int             `json:"next_addr"`
	Gap        uint32          `json:"gap"`
	SelectReps int             `json:"select_reps"`
}

type timed struct {
	Name    string  `json:"name"`
	Summary summary `json:"summary"`
	Note    string  `json:"note,omitempty"`
}

func cmdBench(args json.RawMessage) (any, error) {
	var a benchArgs
	if err := unmarshal(args, &a); err != nil {
		return nil, err
	}
	w, err := openWallet(a.DB)
	if err != nil {
		return nil, err
	}
	defer w.close()
	ctx := context.Background()
	budget := time.Duration(a.BudgetMs) * time.Millisecond
	var out []timed
	add := func(name string, s summary, note string) {
		out = append(out, timed{Name: name, Summary: s, Note: note})
	}
	view := func(f func(dbtx walletdb.ReadTx) error) error {
		return walletdb.View(ctx, w.db, f)
	}

	// Balances: one account per call (AccountBalance runs the full scan)
	// and every account at once (AccountBalances, the same scan).
	for _, mc := range a.MinConfs {
		for _, acct := range a.Accounts {
			s, err := repeat(a.MinReps, a.MaxReps, budget, func() error {
				return view(func(dbtx walletdb.ReadTx) error {
					_, err := w.store.AccountBalance(dbtx, mc, acct)
					return err
				})
			})
			if err != nil {
				return nil, err
			}
			add(fmt.Sprintf("balance/minconf=%d/account=%d", mc, acct), s, "")
		}
		s, err := repeat(a.MinReps, a.MaxReps, budget, func() error {
			return view(func(dbtx walletdb.ReadTx) error {
				_, err := w.store.AccountBalances(dbtx, mc)
				return err
			})
		})
		if err != nil {
			return nil, err
		}
		add(fmt.Sprintf("balance_all/minconf=%d", mc), s, "")
	}

	// The unspent walks.
	var nUnspent int
	s, err := repeat(a.MinReps, a.MaxReps, budget, func() error {
		return view(func(dbtx walletdb.ReadTx) error {
			c, err := w.store.UnspentOutputs(dbtx)
			nUnspent = len(c)
			return err
		})
	})
	if err != nil {
		return nil, err
	}
	add("unspent/outputs", s, fmt.Sprintf("%d outputs", nUnspent))
	s, err = repeat(a.MinReps, a.MaxReps, budget, func() error {
		return view(func(dbtx walletdb.ReadTx) error {
			return w.store.ForEachUnspentOutpoint(dbtx, func(*wire.OutPoint) error { return nil })
		})
	})
	if err != nil {
		return nil, err
	}
	add("unspent/outpoints", s, "")

	// Input selection: MakeInputSource at minconf 1 (its KeyN walk
	// included), one target per call, the account rotating.
	var tipHeight int32
	view(func(dbtx walletdb.ReadTx) error {
		_, tipHeight = w.store.MainChainTip(dbtx)
		return nil
	})
	for _, target := range a.Targets {
		var selected, short int
		i := 0
		s, err := repeat(a.SelectReps, a.SelectReps, 0, func() error {
			acct := a.Accounts[i%len(a.Accounts)]
			i++
			return view(func(dbtx walletdb.ReadTx) error {
				src := w.store.MakeInputSource(dbtx, acct, 1, tipHeight, nil)
				d, err := src.SelectInputs(dcrutil.Amount(target))
				if err == nil {
					selected += len(d.Inputs)
					if int64(d.Amount) < target {
						short++
					}
				}
				return err
			})
		})
		if err != nil {
			return nil, err
		}
		add(fmt.Sprintf("select/target=%d", target), s,
			fmt.Sprintf("%d inputs selected, %d short", selected, short))
	}

	// Range queries.
	for _, r := range a.Ranges {
		var n int
		s, err := repeat(a.MinReps, a.MaxReps, budget, func() error {
			n = 0
			return view(func(dbtx walletdb.ReadTx) error {
				ns := dbtx.ReadBucket(wtxmgrKey)
				return w.store.RangeTransactions(ctx, ns, r.Begin, r.End,
					func(d []udb.TxDetails) (bool, error) {
						n += len(d)
						return r.Limit > 0 && n >= r.Limit, nil
					})
			})
		})
		if err != nil {
			return nil, err
		}
		add("range/"+r.Name, s, fmt.Sprintf("%d transactions", n))
	}
	for _, r := range a.ByAccount {
		var n int
		s, err := repeat(a.MinReps, a.MaxReps, budget, func() error {
			return view(func(dbtx walletdb.ReadTx) error {
				hashes, err := w.byAccount(dbtx, r.Account, r.Limit)
				n = len(hashes)
				return err
			})
		})
		if err != nil {
			return nil, err
		}
		add("by_account/"+r.Name, s, fmt.Sprintf("%d transactions", n))
	}

	// Address lookups by script, all in one read transaction; the summary
	// is of the whole batch.
	scripts := make([][]byte, len(a.Scripts))
	for i, h := range a.Scripts {
		scripts[i], _ = hex.DecodeString(h)
	}
	var hits int
	s, err = repeat(a.MinReps, a.MaxReps, budget, func() error {
		hits = 0
		return view(func(dbtx walletdb.ReadTx) error {
			ns := dbtx.ReadBucket(waddrmgrKey)
			for _, script := range scripts {
				_, addrs := stdscript.ExtractAddrs(0, script, params)
				for _, addr := range addrs {
					if _, err := w.manager.Address(ns, addr); err == nil {
						hits++
					}
				}
			}
			return nil
		})
	})
	if err != nil {
		return nil, err
	}
	add("lookup/batch", s, fmt.Sprintf("%d scripts, %d hits", len(scripts), hits))

	// Next-address bookkeeping: what the wallet writes per returned address
	// (wallet/addresses.go nextAddress, accountBranchChildUpdates.UpdateDB):
	// one write transaction recording the child, which here is one past the
	// last recorded external child of account 0, so each call derives and
	// stores one address.
	var next uint32
	err = view(func(dbtx walletdb.ReadTx) error {
		ns := dbtx.ReadBucket(waddrmgrKey)
		return w.manager.ForEachAccountAddress(ns, 0, func(ma udb.ManagedAddress) error {
			if pk, ok := ma.(udb.ManagedPubKeyAddress); ok && !ma.Internal() && pk.Index() >= next {
				next = pk.Index() + 1
			}
			return nil
		})
	})
	if err != nil {
		return nil, err
	}
	s, err = repeat(a.NextAddr, a.NextAddr, 0, func() error {
		child := next
		next++
		return walletdb.Update(ctx, w.db, func(dbtx walletdb.ReadWriteTx) error {
			ns := dbtx.ReadWriteBucket(waddrmgrKey)
			return w.manager.SyncAccountToAddrIndex(ns, 0, child, 0)
		})
	})
	if err != nil {
		return nil, err
	}
	add("next_address", s, "")

	fi, err := os.Stat(a.DB)
	if err != nil {
		return nil, err
	}
	return map[string]any{"timings": out, "file_size": fi.Size()}, nil
}

// byAccount emulates a per-account history query, which udb has no path
// for: walk the mined history backward from the tip and keep the
// transactions that credit or debit the account, resolving each credit's
// and each debited output's script to an account through the address
// manager, until limit are found.
func (w *wallet) byAccount(dbtx walletdb.ReadTx, account uint32, limit int) ([]chainhash.Hash, error) {
	ns := dbtx.ReadBucket(wtxmgrKey)
	addrmgrNs := dbtx.ReadBucket(waddrmgrKey)
	var out []chainhash.Hash
	acctOf := func(script []byte) (uint32, bool) {
		_, addrs := stdscript.ExtractAddrs(0, script, params)
		if len(addrs) == 0 {
			return 0, false
		}
		a, err := w.manager.AddrAccount(addrmgrNs, addrs[0])
		return a, err == nil
	}
	err := w.store.RangeTransactions(context.Background(), ns, 0x7fffffff, 0,
		func(details []udb.TxDetails) (bool, error) {
			for i := range details {
				d := &details[i]
				hit := false
				for _, c := range d.Credits {
					if a, ok := acctOf(d.MsgTx.TxOut[c.Index].PkScript); ok && a == account {
						hit = true
						break
					}
				}
				for j := 0; !hit && j < len(d.Debits); j++ {
					prev := d.MsgTx.TxIn[d.Debits[j].Index].PreviousOutPoint
					pd, err := w.store.TxDetails(ns, &prev.Hash)
					if err != nil {
						return false, err
					}
					if a, ok := acctOf(pd.MsgTx.TxOut[prev.Index].PkScript); ok && a == account {
						hit = true
					}
				}
				if hit {
					out = append(out, d.Hash)
					if len(out) >= limit {
						return true, nil
					}
				}
			}
			return false, nil
		})
	return out, err
}

// ---------------------------------------------------------------------------
// export

type exportArgs struct {
	DB             string          `json:"db"`
	Out            string          `json:"out"`
	MinConfs       []int32         `json:"minconfs"`
	Probes         []string        `json:"probes"`
	Scans          []string        `json:"scans"`
	Ranges         []rangeSpec     `json:"ranges"`
	ByAccount      []byAccountSpec `json:"by_account"`
	SelectAccounts uint32          `json:"select_accounts"`
	SelectDetail   bool            `json:"select_detail"`
	SelectMinConfs []int32         `json:"select_minconfs"`
	SelectTargets  []int64         `json:"select_targets"`
}

func hexOrDash(b []byte) string {
	if b == nil {
		return "-"
	}
	return hex.EncodeToString(b)
}

func cmdExport(args json.RawMessage) (any, error) {
	var a exportArgs
	if err := unmarshal(args, &a); err != nil {
		return nil, err
	}
	w, err := openWallet(a.DB)
	if err != nil {
		return nil, err
	}
	defer w.close()
	var lines []string
	emit := func(format string, v ...any) { lines = append(lines, fmt.Sprintf(format, v...)) }
	ctx := context.Background()
	err = walletdb.View(ctx, w.db, func(dbtx walletdb.ReadTx) error {
		ns := dbtx.ReadBucket(wtxmgrKey)
		addrmgrNs := dbtx.ReadBucket(waddrmgrKey)
		tipHash, tipHeight := w.store.MainChainTip(dbtx)
		emit("tip %d %s", tipHeight, hex.EncodeToString(tipHash[:]))
		unspent := ns.NestedReadBucket([]byte("u"))
		mi := ns.NestedReadBucket([]byte("mi"))
		emit("keyn %d", unspent.KeyN())

		// Accounts, in the address manager's iteration order.
		err := w.manager.ForEachAccount(addrmgrNs, func(acct uint32) error {
			if acct == udb.ImportedAddrAccount {
				return nil
			}
			p, err := w.manager.AccountProperties(addrmgrNs, acct)
			if err != nil {
				return err
			}
			emit("account %d %d %d %d %d", acct, p.LastUsedExternalIndex,
				p.LastUsedInternalIndex, p.LastReturnedExternalIndex,
				p.LastReturnedInternalIndex)
			return nil
		})
		if err != nil {
			return err
		}

		for _, mc := range a.MinConfs {
			bals, err := w.store.AccountBalances(dbtx, mc)
			if err != nil {
				return err
			}
			accts := make([]uint32, 0, len(bals))
			for acct := range bals {
				accts = append(accts, acct)
			}
			sort.Slice(accts, func(i, j int) bool { return accts[i] < accts[j] })
			for _, acct := range accts {
				b := bals[acct]
				emit("balance %d %d %d %d %d %d %d %d %d", mc, acct, b.Spendable,
					b.ImmatureCoinbaseRewards, b.ImmatureStakeGeneration,
					b.LockedByTickets, b.VotingAuthority, b.Unconfirmed, b.Total)
			}
		}

		credits, err := w.store.UnspentOutputs(dbtx)
		if err != nil {
			return err
		}
		emit("unspent %d", len(credits))
		for _, c := range credits {
			emit("utxo %s %d %d %d %d %t %s", hex.EncodeToString(c.Hash[:]), c.Index,
				c.Tree, c.Block.Height, c.Amount, c.FromCoinBase, hex.EncodeToString(c.PkScript))
		}

		// Cursor probes on the unspent bucket (spec 04 section 5.1): seek,
		// three steps one way, then two the other way, so that a probe
		// near either end also pins what a step does after a nil.
		for _, ph := range a.Probes {
			k, _ := hex.DecodeString(ph)
			c := unspent.ReadCursor()
			r0, _ := c.Seek(k)
			fwd := []string{hexOrDash(r0)}
			for i := range 5 {
				var kk []byte
				if i < 3 {
					kk, _ = c.Next()
				} else {
					kk, _ = c.Prev()
				}
				fwd = append(fwd, hexOrDash(kk))
			}
			c.Seek(k)
			var back []string
			for i := range 5 {
				var kk []byte
				if i < 3 {
					kk, _ = c.Prev()
				} else {
					kk, _ = c.Next()
				}
				back = append(back, hexOrDash(kk))
			}
			c.Close()
			emit("probe %s %s | %s", ph, strings.Join(fwd, " "), strings.Join(back, " "))
		}
		// The scans of randomUTXO (txmined.go:2358), both directions from
		// each key, with RandomUTXO's skip at minconf 1.
		skip := func(k, v []byte) bool {
			if mi.Get(k) != nil {
				return true
			}
			if len(v) < 4 {
				return true
			}
			h := int32(binary.BigEndian.Uint32(v))
			return h == -1 || h > tipHeight || tipHeight-h+1 < 1
		}
		scan := func(c walletdb.ReadCursor, k, v []byte, iter func() ([]byte, []byte)) []string {
			var keys [][]byte
			for ; k != nil; k, v = iter() {
				if len(keys) > 0 && !bytes.Equal(keys[0][:32], k[:32]) {
					break
				}
				if skip(k, v) {
					continue
				}
				keys = append(keys, append([]byte(nil), k...))
			}
			out := make([]string, len(keys))
			for i, kk := range keys {
				out[i] = hex.EncodeToString(kk)
			}
			return out
		}
		for _, sh := range a.Scans {
			k, _ := hex.DecodeString(sh)
			c := unspent.ReadCursor()
			k0, v0 := c.Seek(k)
			next := scan(c, k0, v0, c.Next)
			c.Seek(k)
			kp, vp := c.Prev()
			prev := scan(c, kp, vp, c.Prev)
			c.Close()
			emit("scan %s next %s prev %s", sh, strings.Join(next, ","), strings.Join(prev, ","))
		}

		for _, r := range a.Ranges {
			emit("range %s %d %d %d", r.Name, r.Begin, r.End, r.Limit)
			n := 0
			err := w.store.RangeTransactions(ctx, ns, r.Begin, r.End,
				func(details []udb.TxDetails) (bool, error) {
					emit("batch %d", len(details))
					for i := range details {
						d := &details[i]
						var sb strings.Builder
						fmt.Fprintf(&sb, "tx %s %d", hex.EncodeToString(d.Hash[:]), d.Block.Height)
						for _, c := range d.Credits {
							fmt.Fprintf(&sb, " c%d:%d:%t:%t", c.Index, c.Amount, c.Spent, c.Change)
						}
						for _, db := range d.Debits {
							fmt.Fprintf(&sb, " d%d:%d", db.Index, db.Amount)
						}
						lines = append(lines, sb.String())
					}
					n += len(details)
					return r.Limit > 0 && n >= r.Limit, nil
				})
			if err != nil {
				return err
			}
		}
		for _, r := range a.ByAccount {
			hashes, err := w.byAccount(dbtx, r.Account, r.Limit)
			if err != nil {
				return err
			}
			parts := make([]string, len(hashes))
			for i, h := range hashes {
				parts[i] = hex.EncodeToString(h[:])
			}
			emit("by_account %s %d %d %s", r.Name, r.Account, r.Limit, strings.Join(parts, ","))
		}

		// Stake invalidation state: the blocks marked invalid, and the
		// invalidated credits and debits of main-chain blocks. Rollback
		// leaves the rows of a detached block in ic and id under that
		// block's hash, where nothing reads them again; the export skips
		// them.
		mainHash := func(h int32) []byte {
			hash, err := w.store.GetMainChainBlockHashForHeight(ns, h)
			if err != nil {
				return nil
			}
			return hash[:]
		}
		for h := int32(0); h <= tipHeight; h++ {
			hash, err := w.store.GetMainChainBlockHashForHeight(ns, h)
			if err != nil {
				return err
			}
			if _, invalid := w.store.BlockInMainChain(dbtx, &hash); invalid {
				emit("invalid %d %s", h, hex.EncodeToString(hash[:]))
			}
		}
		err = ns.NestedReadBucket([]byte("ic")).ForEach(func(k, v []byte) error {
			h := int32(binary.BigEndian.Uint32(k[32:36]))
			if !bytes.Equal(k[36:68], mainHash(h)) {
				return nil
			}
			emit("inv_credit %s %d %d %d %t", hex.EncodeToString(k[:32]), h,
				binary.BigEndian.Uint32(k[68:72]), int64(binary.BigEndian.Uint64(v[:8])), v[8]&1 != 0)
			return nil
		})
		if err != nil {
			return err
		}
		err = ns.NestedReadBucket([]byte("id")).ForEach(func(k, v []byte) error {
			h := int32(binary.BigEndian.Uint32(k[32:36]))
			if !bytes.Equal(k[36:68], mainHash(h)) {
				return nil
			}
			ck := v[8:80]
			emit("inv_debit %s %d %d %d %s %d", hex.EncodeToString(k[:32]), h,
				binary.BigEndian.Uint32(k[68:72]), int64(binary.BigEndian.Uint64(v[:8])),
				hex.EncodeToString(ck[:32]), binary.BigEndian.Uint32(ck[68:72]))
			return nil
		})
		if err != nil {
			return err
		}

		// ForEachUnspentOutpoint, in its order.
		var fe []string
		err = w.store.ForEachUnspentOutpoint(dbtx, func(op *wire.OutPoint) error {
			fe = append(fe, fmt.Sprintf("fe %s %d %d", hex.EncodeToString(op.Hash[:]), op.Index, op.Tree))
			return nil
		})
		if err != nil {
			return err
		}
		emit("foreach %d", len(fe))
		lines = append(lines, fe...)

		// Selection with targets whose result does not depend on the
		// random draws: 0 takes the eligible list without the random
		// phase, and an unreachable target takes every eligible output.
		for acct := uint32(0); acct < a.SelectAccounts; acct++ {
			for _, mc := range a.SelectMinConfs {
				for _, target := range a.SelectTargets {
					src := w.store.MakeInputSource(dbtx, acct, mc, tipHeight, nil)
					d, err := src.SelectInputs(dcrutil.Amount(target))
					if err != nil {
						return err
					}
					sel := make([]string, len(d.Inputs))
					for i, in := range d.Inputs {
						op := in.PreviousOutPoint
						sel[i] = fmt.Sprintf("%s %d %d %d", hex.EncodeToString(op.Hash[:]), op.Index,
							op.Tree, in.ValueIn)
					}
					sort.Strings(sel)
					sum := sha256.Sum256([]byte(strings.Join(sel, "\n")))
					emit("select %d %d %d %d %d %s", acct, mc, target, len(sel), int64(d.Amount),
						hex.EncodeToString(sum[:]))
					if a.SelectDetail {
						for _, l := range sel {
							emit("sel %s", l)
						}
					}
				}
			}
		}

		// Each account's addresses in ForEachAccountAddress order.
		return w.manager.ForEachAccount(addrmgrNs, func(acct uint32) error {
			if acct == udb.ImportedAddrAccount {
				return nil
			}
			h := sha256.New()
			n := 0
			err := w.manager.ForEachAccountAddress(addrmgrNs, acct, func(ma udb.ManagedAddress) error {
				hh, ok := ma.Address().(stdaddr.Hash160er)
				if !ok {
					return fmt.Errorf("address %v has no HASH160", ma.Address())
				}
				h.Write(hh.Hash160()[:])
				n++
				return nil
			})
			emit("addrs %d %d %s", acct, n, hex.EncodeToString(h.Sum(nil)))
			return err
		})
	})
	if err != nil {
		return nil, err
	}
	body := strings.Join(lines, "\n") + "\n"
	if err := os.WriteFile(a.Out, []byte(body), 0o644); err != nil {
		return nil, err
	}
	sum := sha256.Sum256([]byte(body))
	return map[string]any{"sha256": hex.EncodeToString(sum[:]), "lines": len(lines)}, nil
}
