// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

package walletexport

// Sections [chain] to [multisig]: raw rows of the wtxmgr namespace decoded
// per wallet/udb/txdb.go and txmined.go. Secondary indexes (u, mu, cmu) are
// folded into the records they point at; entries pointing at nothing become
// [orphans].
//
// Everything here streams. [blocks], [headers] and [cfilters] are driven by
// the wtxmgr/b cursor, whose BE32 height order is the record order, with a
// Get per height into h and cf. Buckets keyed by internal (byte-reversed)
// hashes are not in record order, which compares hashes in display order,
// so their keys are first collected as fixed-width sort keys (hash segments
// reversed), sorted, and the rows then read back with a Get each.

import (
	"bytes"
	"crypto/sha256"
	"encoding/binary"
	"fmt"
	"hash"
	"sort"

	bolt "go.etcd.io/bbolt"
)

// segSize is the height span of one chain digest segment (ADR-0001 6.10).
const segSize = 4096

func be32(b []byte) uint32 { return binary.BigEndian.Uint32(b) }
func be64(b []byte) uint64 { return binary.BigEndian.Uint64(b) }

func be32Bytes(v uint32) []byte {
	b := make([]byte, 4)
	binary.BigEndian.PutUint32(b, v)
	return b
}

// pad32 copies b into 32 bytes, zero-padded or truncated, as
// Store.MainChainTip and ProcessedTxsBlockMarker do (txmined.go:183-200,
// 277-282).
func pad32(b []byte) []byte {
	var h [32]byte
	copy(h[:], b)
	return h[:]
}

// segmenter accumulates the digest items of one record type per segment.
type segmenter struct {
	cur  int64
	n    uint64
	h    hash.Hash
	emit func(seg uint64, n uint64, sum []byte)
}

func newSegmenter(emit func(seg, n uint64, sum []byte)) *segmenter {
	return &segmenter{cur: -1, emit: emit}
}

func (s *segmenter) add(height uint32, item []byte) {
	seg := int64(height / segSize)
	if seg != s.cur {
		s.flush()
		s.cur = seg
		s.h = sha256.New()
	}
	s.h.Write(item)
	s.n++
}

func (s *segmenter) flush() {
	if s.cur >= 0 && s.n > 0 {
		s.emit(uint64(s.cur), s.n, s.h.Sum(nil))
	}
	s.n = 0
}

// ---------------------------------------------------------------- sorting

// keyArena holds fixed-width sort keys back to back, so that sorting a
// million keys costs their bytes and no per-key allocation.
type keyArena struct {
	w   int
	b   []byte
	tmp []byte
}

func newKeyArena(w int) *keyArena { return &keyArena{w: w, tmp: make([]byte, w)} }

func (a *keyArena) Len() int           { return len(a.b) / a.w }
func (a *keyArena) at(i int) []byte    { return a.b[i*a.w : (i+1)*a.w] }
func (a *keyArena) Less(i, j int) bool { return bytes.Compare(a.at(i), a.at(j)) < 0 }
func (a *keyArena) Swap(i, j int) {
	copy(a.tmp, a.at(i))
	copy(a.at(i), a.at(j))
	copy(a.at(j), a.tmp)
}

// A key layout lists the segments of a fixed-width bucket key: a positive
// n is n bytes kept as stored (BE32 numbers, whose byte order is numeric
// order), hashSeg a 32-byte hash, reversed into display order. The
// transform is its own inverse.
const hashSeg = -32

var (
	layHash      = []int{hashSeg}                // hash
	layOutpoint  = []int{hashSeg, 4}             // hash ‖ BE32 index
	layIncidence = []int{hashSeg, 4, hashSeg}    // hash ‖ BE32 height ‖ block hash
	layCredit    = []int{hashSeg, 4, hashSeg, 4} // incidence ‖ BE32 index (credits and debits)
)

func layoutWidth(layout []int) int {
	w := 0
	for _, n := range layout {
		if n < 0 {
			w -= n
		} else {
			w += n
		}
	}
	return w
}

func transform(layout []int, dst, k []byte) {
	off := 0
	for _, n := range layout {
		if n < 0 {
			for i := 0; i < -n; i++ {
				dst[off+i] = k[off-n-1-i]
			}
			off -= n
		} else {
			copy(dst[off:off+n], k[off:off+n])
			off += n
		}
	}
}

// rowSource is one bucket feeding a sorted section; tag distinguishes the
// buckets of one section (c and ic, d and id), as the last byte of the
// sort key, which is also the record order of the inv field.
type rowSource struct {
	path []string
	b    *bolt.Bucket
	tag  byte
}

// sortedRows reads every key/value entry of the sources (claiming them),
// sends keys of the wrong length to [bad], and calls f for every other
// row in record order.
func (e *exporter) sortedRows(layout []int, srcs []rowSource, f func(src rowSource, k, v []byte)) {
	e.sortedRowsWhere(layout, srcs, nil, f)
}

// sortedRowsWhere is sortedRows for the rows pre accepts (nil accepts
// all); a row pre rejects costs no sort key.
func (e *exporter) sortedRowsWhere(layout []int, srcs []rowSource, pre func(src rowSource, k, v []byte) bool, f func(src rowSource, k, v []byte)) {
	w := layoutWidth(layout)
	arena := newKeyArena(w + 1)
	buf := make([]byte, w+1)
	byTag := make(map[byte]rowSource)
	for _, src := range srcs {
		byTag[src.tag] = src
		if src.b == nil {
			continue
		}
		e.claims.claimAll(src.path)
		c := src.b.Cursor()
		for k, v := c.First(); k != nil; k, v = c.Next() {
			if v == nil {
				continue // a nested bucket: left to the closed-world walk
			}
			if len(k) != w {
				e.bad(src.path, k, "key", v)
				continue
			}
			if pre != nil && !pre(src, k, v) {
				continue
			}
			transform(layout, buf, k)
			buf[w] = src.tag
			arena.b = append(arena.b, buf...)
		}
	}
	sort.Sort(arena)
	key := make([]byte, w)
	for i := 0; i < arena.Len(); i++ {
		sk := arena.at(i)
		transform(layout, key, sk[:w])
		src := byTag[sk[w]]
		f(src, key, src.b.Get(key))
	}
}

// ---------------------------------------------------------------- [chain]

func (e *exporter) txStore() error {
	ns := e.bucket("wtxmgr")
	if ns == nil {
		return fmt.Errorf("no wtxmgr bucket")
	}
	root := []string{"wtxmgr"}
	get := func(key string) []byte {
		v := ns.Get([]byte(key))
		if v != nil {
			e.claims.claim(root, []byte(key))
		}
		return v
	}

	// [chain] (txmined.go:183-200, 213-231, 277-282, 341-406). tip is
	// the zero-padded copy MainChainTip returns; ExtendMainChain compares
	// and looks up the raw bytes (txmined.go:215-231), so a tip of any
	// length but 32 is also a [bad] record. lasttxsblock is only read
	// through the padded copy (txmined.go:277-319).
	chain := e.sec("chain")
	tip := get("tip")
	chain.add("tip", vH(pad32(tip)))
	switch {
	case tip == nil:
		e.bad(root, []byte("tip"), "missing", nil)
	case len(tip) != 32:
		e.bad(root, []byte("tip"), "len", tip)
	}
	chain.add("processed", vH(pad32(get("lasttxsblock"))))
	hc := absent
	if v := get("havecfilters"); v != nil {
		hc = vX(v)
	}
	chain.add("havecfilters", hc)
	if v := get("birthstate"); len(v) == 45 {
		chain.add("birth", "1", vB(v[0]&1 != 0), vB(v[0]&2 != 0), vH(v[1:33]),
			vU(uint64(be32(v[33:37]))), vI(int64(be64(v[37:45]))))
	} else {
		if v != nil {
			e.bad(root, []byte("birthstate"), "len", v)
		}
		chain.add("birth", "0", absent, absent, absent, absent, absent)
	}
	e.flush("chain")

	main := e.blocks()
	e.headersAndFilters(main)
	e.transactions()
	e.credits()
	e.debits()
	e.unmined()
	e.ticketsAndCommitments()
	e.multisig()
	return nil
}

// ---------------------------------------------------------------- [blocks]

// blockRow is a decoded wtxmgr/b row: hash(32), BE64 time, BE16 vote
// bits, invalidated byte, BE32 n, n hashes (txdb.go:216-243, 308-330).
type blockRow struct {
	height uint32
	hash   []byte
	n      uint32
	inv    bool
	trail  bool
}

func decodeBlockRow(k, v []byte) (blockRow, string) {
	if len(k) != 4 {
		return blockRow{}, "key"
	}
	if len(v) < 47 || uint64(len(v)) < 47+32*uint64(be32(v[43:47])) {
		return blockRow{}, "len"
	}
	n := be32(v[43:47])
	return blockRow{
		height: be32(k),
		hash:   v[:32],
		n:      n,
		inv:    v[42] != 0,
		trail:  uint64(len(v)) != 47+32*uint64(n),
	}, ""
}

// eachBlock calls f for every wtxmgr/b row that decodes, in height order;
// report also files the [bad] records (one pass of several does).
func (e *exporter) eachBlock(report bool, f func(r blockRow, v []byte)) {
	path := []string{"wtxmgr", "b"}
	b := e.tx.Bucket([]byte("wtxmgr")).Bucket([]byte("b"))
	if b == nil {
		return
	}
	c := b.Cursor()
	for k, v := c.First(); k != nil; k, v = c.Next() {
		if v == nil {
			continue
		}
		r, reason := decodeBlockRow(k, v)
		if reason != "" {
			if report {
				e.bad(path, k, reason, v)
			}
			continue
		}
		if report && r.trail {
			// appendRawBlockRecord appends after len(v), not after
			// 47+32n (txdb.go:266-274), so the next mined insert would
			// turn trailing bytes into a transaction hash.
			e.bad(path, k, "trail", v)
		}
		f(r, v)
	}
}

// mainSet is the sorted list of main-chain block hashes (stored order).
type mainSet [][32]byte

func (m mainSet) has(h []byte) bool {
	if len(h) != 32 {
		return false
	}
	i := sort.Search(len(m), func(i int) bool { return bytes.Compare(m[i][:], h) >= 0 })
	return i < len(m) && bytes.Equal(m[i][:], h)
}

func (e *exporter) blocks() mainSet {
	path := []string{"wtxmgr", "b"}
	e.bucket(path...)
	e.claims.claimAll(path)
	w := e.out.section("blocks")
	if e.digest {
		// Digest mode: the segment records precede the block records, so
		// one pass writes the segments and a second the listed blocks.
		seg := newSegmenter(func(s, n uint64, sum []byte) {
			w.add("blockseg", vU(s), vU(n), vX(sum))
		})
		e.eachBlock(false, func(r blockRow, v []byte) {
			if r.n == 0 && !r.inv {
				item := make([]byte, 0, 46)
				item = append(item, be32Bytes(r.height)...)
				item = append(item, v[:42]...)
				seg.add(r.height, item)
			}
		})
		seg.flush()
	}
	var main mainSet
	e.eachBlock(true, func(r blockRow, v []byte) {
		var h [32]byte
		copy(h[:], r.hash)
		main = append(main, h)
		if e.digest && r.n == 0 && !r.inv {
			return
		}
		hashes := make([][]byte, r.n)
		for i := range hashes {
			hashes[i] = v[47+32*i : 79+32*i]
		}
		w.add("block", vU(uint64(r.height)), vH(r.hash), vU(be64(v[32:40])),
			vU(uint64(binary.BigEndian.Uint16(v[40:42]))), vB(r.inv), vL(hashes))
	})
	w.close()
	sort.Slice(main, func(i, j int) bool { return bytes.Compare(main[i][:], main[j][:]) < 0 })
	return main
}

// ---------------------------------------------------------------- [headers] [cfilters]

type missing struct {
	height uint32
	hash   []byte
}

func (e *exporter) headersAndFilters(main mainSet) {
	// [headers]: wtxmgr/h, key block hash, value the header
	// (txdb.go:460-485).
	hpath := []string{"wtxmgr", "h"}
	hb := e.bucket(hpath...)
	w := e.out.section("headers")
	var miss []missing
	hseg := newSegmenter(func(s, n uint64, sum []byte) {
		w.add("hdrseg", vU(s), vU(n), vX(sum))
	})
	e.eachBlock(false, func(r blockRow, _ []byte) {
		var raw []byte
		if hb != nil {
			raw = hb.Get(r.hash)
		}
		switch {
		case raw == nil:
			miss = append(miss, missing{r.height, r.hash})
		case e.digest:
			item := append(append(be32Bytes(r.height), r.hash...), be32Bytes(uint32(len(raw)))...)
			hseg.add(r.height, append(item, raw...))
		default:
			w.add("header", vU(uint64(r.height)), vH(r.hash), vX(raw))
		}
	})
	hseg.flush()
	for _, m := range miss {
		w.add("hdrmissing", vU(uint64(m.height)), vH(m.hash))
	}
	side := func(_ rowSource, k, _ []byte) bool { return !main.has(k) }
	e.sortedRowsWhere(layHash, []rowSource{{path: hpath, b: hb}}, side, func(_ rowSource, k, v []byte) {
		w.add("sideheader", vH(k), vX(v))
	})
	w.close()

	// [cfilters]: wtxmgr/cf, value 16-byte key then the filter; shorter
	// is [bad] (txdb.go:1968-2005).
	cpath := []string{"wtxmgr", "cf"}
	cb := e.bucket(cpath...)
	w = e.out.section("cfilters")
	miss = miss[:0]
	cseg := newSegmenter(func(s, n uint64, sum []byte) {
		w.add("cfseg", vU(s), vU(n), vX(sum))
	})
	e.eachBlock(false, func(r blockRow, _ []byte) {
		var v []byte
		if cb != nil {
			v = cb.Get(r.hash)
		}
		switch {
		case len(v) < 16: // absent, or [bad] (filed below)
			miss = append(miss, missing{r.height, r.hash})
		case e.digest:
			item := append(append(be32Bytes(r.height), r.hash...), v[:16]...)
			item = append(item, be32Bytes(uint32(len(v)-16))...)
			cseg.add(r.height, append(item, v[16:]...))
		default:
			w.add("cfilter", vU(uint64(r.height)), vH(r.hash), vX(v[:16]), vX(v[16:]))
		}
	})
	cseg.flush()
	for _, m := range miss {
		w.add("cfmissing", vU(uint64(m.height)), vH(m.hash))
	}
	sideFilter := func(_ rowSource, k, v []byte) bool {
		if len(v) < 16 {
			e.bad(cpath, k, "len", v)
			return false
		}
		return !main.has(k)
	}
	e.sortedRowsWhere(layHash, []rowSource{{path: cpath, b: cb}}, sideFilter, func(_ rowSource, k, v []byte) {
		w.add("sidecfilter", vH(k), vX(v[:16]), vX(v[16:]))
	})
	w.close()
}

// ---------------------------------------------------------------- [txs] [unpublished]

func (e *exporter) transactions() {
	w := e.out.section("txs")
	tpath := []string{"wtxmgr", "t"}
	e.sortedRows(layIncidence, []rowSource{{path: tpath, b: e.bucket(tpath...)}}, func(_ rowSource, k, v []byte) {
		// key: hash, BE32 height, block hash; value: BE64 received, tx
		// (txdb.go:487-526)
		if len(v) < 8 {
			e.bad(tpath, k, "len", v)
			return
		}
		w.add("tx", vH(k[:32]), vU(uint64(be32(k[32:36]))), vH(k[36:68]),
			vI(int64(be64(v))), vX(v[8:]))
	})
	mpath := []string{"wtxmgr", "m"}
	e.sortedRows(layHash, []rowSource{{path: mpath, b: e.bucket(mpath...)}}, func(_ rowSource, k, v []byte) {
		if len(v) < 8 {
			e.bad(mpath, k, "len", v)
			return
		}
		w.add("utx", vH(k), vI(int64(be64(v))), vX(v[8:]))
	})
	w.close()

	w = e.out.section("unpublished")
	upath := []string{"wtxmgr", "up"}
	e.sortedRows(layHash, []rowSource{{path: upath, b: e.bucket(upath...)}}, func(_ rowSource, k, v []byte) {
		// one nonzero byte means unpublished (txdb.go:1396-1420); which
		// nonzero byte is not observable (N10)
		if len(v) != 1 || v[0] == 0 {
			e.bad(upath, k, "value", v)
			return
		}
		w.add("unpublished", vH(k))
	})
	w.close()
}

// ---------------------------------------------------------------- [credits] [debits]

// creditFlags renders the flag byte shared by mined and unmined credits
// (txdb.go:687-798, 1422-1490): change 0x02, stake opcode tag bits 2-4,
// coinbase 0x20, expiry 0x40 (database version 9 and later).
func creditFlags(f byte) (change, optag, coinbase, expiry string) {
	return vB(f&0x02 != 0), vU(uint64(f>>2) & 7), vB(f&0x20 != 0), vB(f&0x40 != 0)
}

func (e *exporter) credits() {
	upath := []string{"wtxmgr", "u"}
	ub := e.bucket(upath...)
	cpath := []string{"wtxmgr", "c"}
	cb := e.bucket(cpath...)
	icpath := []string{"wtxmgr", "ic"}
	icb := e.bucket(icpath...)
	w := e.out.section("credits")
	e.sortedRows(layCredit, []rowSource{{cpath, cb, 0}, {icpath, icb, 1}}, func(src rowSource, k, v []byte) {
		// key: tx hash, BE32 height, block hash, BE32 index (txdb.go:687-720)
		inv := src.tag == 1
		if len(v) < 9 {
			e.bad(src.path, k, "len", v)
			return
		}
		change, optag, coinbase, expiry := creditFlags(v[8])
		spent := v[8]&1 != 0
		sptx, spheight, spblock, spindex := absent, absent, absent, absent
		if spent {
			if len(v) < 81 {
				e.bad(src.path, k, "len", v)
			} else {
				sptx, spheight = vH(v[9:41]), vU(uint64(be32(v[41:45])))
				spblock, spindex = vH(v[45:77]), vU(uint64(be32(v[77:81])))
			}
		}
		stype, soff, slen, acct := absent, absent, absent, absent
		if len(v) >= 94 {
			stype = vU(uint64(v[81] & 0x7f))
			soff, slen = vU(uint64(be32(v[82:86]))), vU(uint64(be32(v[86:90])))
			if v[81]&0x80 != 0 {
				acct = vU(uint64(be32(v[90:94])))
			}
		}
		// The unspent index: u[outpoint] = height ‖ block hash
		// (txdb.go:1084-1103).
		utxo := false
		if ub != nil && !inv {
			op := append(append([]byte(nil), k[:32]...), k[68:72]...)
			utxo = bytes.Equal(ub.Get(op), k[32:68])
		}
		w.add("credit", vH(k[:32]), vU(uint64(be32(k[32:36]))), vH(k[36:68]),
			vU(uint64(be32(k[68:72]))), vB(inv), vI(int64(be64(v))), change, optag, coinbase,
			expiry, vB(spent), sptx, spheight, spblock, spindex, stype, soff, slen, acct, vB(utxo))
	})
	w.close()

	// A u row folds into the mined credit its value names, when that
	// credit row exists and decodes; any other u row is an orphan.
	e.each(upath, ub, func(k, v []byte) {
		if len(k) == 36 && len(v) == 36 && cb != nil {
			ck := make([]byte, 0, 72)
			ck = append(append(append(ck, k[:32]...), v...), k[32:36]...)
			if cv := cb.Get(ck); len(cv) >= 9 {
				return
			}
		}
		if len(k) != 36 || len(v) != 36 {
			e.bad(upath, k, "layout", v)
			return
		}
		e.orphan("utxo", fmt.Sprintf("%s:%d", vH(k[:32]), be32(k[32:36])),
			fmt.Sprintf("%d:%s", be32(v[:4]), vH(v[4:36])))
	})
}

func (e *exporter) debits() {
	dpath := []string{"wtxmgr", "d"}
	idpath := []string{"wtxmgr", "id"}
	w := e.out.section("debits")
	srcs := []rowSource{{dpath, e.bucket(dpath...), 0}, {idpath, e.bucket(idpath...), 1}}
	e.sortedRows(layCredit, srcs, func(src rowSource, k, v []byte) {
		// value: BE64 amount, credit key (txdb.go:1166-1199)
		if len(v) < 80 {
			e.bad(src.path, k, "len", v)
			return
		}
		w.add("debit", vH(k[:32]), vU(uint64(be32(k[32:36]))), vH(k[36:68]),
			vU(uint64(be32(k[68:72]))), vB(src.tag == 1), vI(int64(be64(v))),
			vH(v[8:40]), vU(uint64(be32(v[40:44]))), vH(v[44:76]), vU(uint64(be32(v[76:80]))))
	})
	w.close()
}

// ---------------------------------------------------------------- unmined

func (e *exporter) unmined() {
	w := e.out.section("ucredits")
	mcpath := []string{"wtxmgr", "mc"}
	e.sortedRows(layOutpoint, []rowSource{{path: mcpath, b: e.bucket(mcpath...)}}, func(_ rowSource, k, v []byte) {
		// 22 bytes, or 9 in the legacy form (txdb.go:1422-1565)
		if len(v) < 9 {
			e.bad(mcpath, k, "len", v)
			return
		}
		change, optag, coinbase, expiry := creditFlags(v[8])
		stype, soff, slen, acct := absent, absent, absent, absent
		if len(v) >= 22 {
			stype = vU(uint64(v[9] & 0x7f))
			soff, slen = vU(uint64(be32(v[10:14]))), vU(uint64(be32(v[14:18])))
			if v[9]&0x80 != 0 {
				acct = vU(uint64(be32(v[18:22])))
			}
		}
		w.add("ucredit", vH(k[:32]), vU(uint64(be32(k[32:36]))), vI(int64(be64(v))),
			change, optag, coinbase, expiry, vB(v[8]&1 != 0), stype, soff, slen, acct)
	})
	w.close()

	w = e.out.section("uinputs")
	mipath := []string{"wtxmgr", "mi"}
	e.sortedRows(layOutpoint, []rowSource{{path: mipath, b: e.bucket(mipath...)}}, func(_ rowSource, k, v []byte) {
		// value: the spending unmined transaction's hash (txdb.go:1678-1713)
		if len(v) < 32 {
			e.bad(mipath, k, "len", v)
			return
		}
		w.add("uinput", vH(k[:32]), vU(uint64(be32(k[32:36]))), vH(v[:32]))
	})
	w.close()
}

// ---------------------------------------------------------------- stake

func (e *exporter) ticketsAndCommitments() {
	w := e.out.section("tickets")
	tpath := []string{"wtxmgr", "tix"}
	e.sortedRows(layHash, []rowSource{{path: tpath, b: e.bucket(tpath...)}}, func(_ rowSource, k, v []byte) {
		// value: BE32 picked height as int32 (txdb.go:1715-1746)
		picked := absent
		if len(v) >= 4 {
			picked = vI(int64(int32(be32(v))))
		} else {
			e.bad(tpath, k, "len", v)
		}
		w.add("ticket", vH(k), picked)
	})
	w.close()

	cupath := []string{"wtxmgr", "cmu"}
	cub := e.bucket(cupath...)
	cpath := []string{"wtxmgr", "cmt"}
	cmt := e.bucket(cpath...)
	w = e.out.section("commitments")
	e.sortedRows(layOutpoint, []rowSource{{path: cpath, b: cmt}}, func(_ rowSource, k, v []byte) {
		// value: BE64 amount, BE32 account (txdb.go:2007-2089)
		if len(v) < 8 {
			e.bad(cpath, k, "len", v)
			return
		}
		acct := absent
		if len(v) >= 12 {
			acct = vU(uint64(be32(v[8:12])))
		} else {
			e.bad(cpath, k, "len", v)
		}
		unspent, ums := false, absent
		if cub != nil {
			if uv := cub.Get(k); uv != nil {
				unspent = true
				if len(uv) >= 1 {
					ums = vB(uv[0]&1 != 0)
				} else {
					e.bad(cupath, k, "len", uv)
				}
			}
		}
		w.add("commitment", vH(k[:32]), vU(uint64(be32(k[32:36]))),
			vI(int64(be64(v))), acct, vB(unspent), ums)
	})
	w.close()
	e.each(cupath, cub, func(k, v []byte) {
		if len(k) == 36 && cmt != nil && len(cmt.Get(k)) >= 8 {
			return
		}
		if len(k) != 36 {
			e.bad(cupath, k, "key", v)
			return
		}
		e.orphan("cmtunspent", fmt.Sprintf("%s:%d", vH(k[:32]), be32(k[32:36])), vX(v))
	})
}

func (e *exporter) multisig() {
	mupath := []string{"wtxmgr", "mu"}
	mub := e.bucket(mupath...)
	path := []string{"wtxmgr", "ms"}
	ms := e.bucket(path...)
	w := e.out.section("multisig")
	e.sortedRows(layOutpoint, []rowSource{{path: path, b: ms}}, func(_ rowSource, k, v []byte) {
		// exactly 135 bytes (txdb.go:1748-1846); only bits 0 and 1 of the
		// flags byte are read (N11)
		if len(v) != 135 {
			e.bad(path, k, "len", v)
			return
		}
		spent := v[22]&1 != 0
		sptx, spindex := absent, absent
		if spent {
			sptx, spindex = vH(v[67:99]), vU(uint64(be32(v[99:103])))
		}
		// The value of a mu entry is never read (N10).
		utxo := mub != nil && mub.Get(k) != nil
		w.add("multisig", vH(k[:32]), vU(uint64(be32(k[32:36]))), vX(v[:20]),
			vU(uint64(v[20])), vU(uint64(v[21])), vB(spent), vB(v[22]&2 != 0), vH(v[23:55]),
			vU(uint64(be32(v[55:59]))), vI(int64(be64(v[59:67]))), sptx, spindex, vH(v[103:135]), vB(utxo))
	})
	w.close()
	e.each(mupath, mub, func(k, v []byte) {
		if len(k) == 36 && ms != nil && len(ms.Get(k)) == 135 {
			return
		}
		if len(k) != 36 {
			e.bad(mupath, k, "key", v)
			return
		}
		e.orphan("multisigutxo", fmt.Sprintf("%s:%d", vH(k[:32]), be32(k[32:36])), vX(v))
	})
}
