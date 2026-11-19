// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

package walletexport

// Sections [wallet], [masterkeys], [cryptokeys], [cointypes], [accounts],
// [accountnames] and [addresses]: raw rows of the waddrmgr namespace decoded
// per wallet/udb/addressdb.go, with plaintext fields (xpubs, address
// strings, hash160s, imported public keys) taken from udb's public API.

import (
	"crypto/sha256"
	"encoding/binary"
	"fmt"
	"strconv"

	"decred.org/dcrwallet/v5/wallet/udb"
	"github.com/decred/dcrd/txscript/v4/stdaddr"
	bolt "go.etcd.io/bbolt"
)

const (
	maxAccountNum       = 1<<31 - 2 // udb.MaxAccountNum
	importedAddrAccount = 1<<31 - 1 // udb.ImportedAddrAccount
)

// addrInfo is what udb's API says about one address row.
type addrInfo struct {
	id      []byte
	addr    string
	pubkey  []byte          // imported keys only
	address stdaddr.Address // imported keys only (for [privcheck])
}

func le32(b []byte) uint32 { return binary.LittleEndian.Uint32(b) }

func le32Bytes(v uint32) []byte {
	b := make([]byte, 4)
	binary.LittleEndian.PutUint32(b, v)
	return b
}

// lenPrefixed splits <LE32 len><bytes> off the front of b
// (addressdb.go:99-137, 1190-1295).
func lenPrefixed(b []byte) (field, rest []byte, ok bool) {
	if len(b) < 4 {
		return nil, nil, false
	}
	n := uint64(le32(b))
	if uint64(len(b)-4) < n {
		return nil, nil, false
	}
	return b[4 : 4+n], b[4+n:], true
}

// decryptPublic opens a blob sealed under the public crypto key through
// Manager.Decrypt (addressmanager.go:2622). It returns absent without the
// public passphrase or for an empty blob, and records a bad row when the
// blob does not open.
func (e *exporter) decryptPublic(path []string, key, blob []byte) string {
	if e.mgr == nil || len(blob) == 0 {
		return absent
	}
	var plain []byte
	err := recovered(func() error {
		var err error
		plain, err = e.mgr.Decrypt(udb.CKTPublic, blob)
		return err
	})
	if err != nil {
		e.bad(path, key, "decrypt", blob)
		return absent
	}
	return vS(plain)
}

func (e *exporter) addressManager() error {
	ns := e.bucket("waddrmgr")
	if ns == nil {
		return fmt.Errorf("no waddrmgr bucket")
	}
	mainPath := []string{"waddrmgr", "main"}
	main := e.bucket(mainPath...)
	metaPath := []string{"waddrmgr", "meta"}
	meta := e.bucket(metaPath...)
	if main == nil || meta == nil {
		return fmt.Errorf("waddrmgr lacks main or meta")
	}
	get := func(path []string, b *bolt.Bucket, key string) []byte {
		v := b.Get([]byte(key))
		if v != nil {
			e.claims.claim(path, []byte(key))
		}
		return v
	}

	// [wallet] (addressdb.go:489-499, 711-743, 1683-1686; txdb.go:163-185, 2180-2185)
	watch := absent
	if v := get(mainPath, main, "watchonly"); len(v) == 1 {
		watch = vB(v[0] != 0)
	} else {
		e.bad(mainPath, []byte("watchonly"), "len", v)
	}
	lastAcct := absent
	if v := get(metaPath, meta, "lastaccount"); len(v) == 4 {
		lastAcct = vU(uint64(le32(v)))
	} else {
		e.bad(metaPath, []byte("lastaccount"), "len", v)
	}
	lastImp := absent
	switch v := get(metaPath, meta, "lastimportedaccount"); {
	case len(v) == 0:
		lastImp = vU(importedAddrAccount)
	case len(v) == 4 && le32(v) > maxAccountNum:
		lastImp = vU(uint64(le32(v)))
	default:
		e.bad(metaPath, []byte("lastimportedaccount"), "len", v)
	}
	mgrCreated := absent
	if v := get(mainPath, main, "mgrcreated"); len(v) == 8 {
		mgrCreated = vU(binary.LittleEndian.Uint64(v))
	} else if v != nil {
		e.bad(mainPath, []byte("mgrcreated"), "len", v)
	}
	txCreated, minedBal := absent, absent
	txPath := []string{"wtxmgr"}
	if txns := e.bucket("wtxmgr"); txns != nil {
		if v := get(txPath, txns, "date"); len(v) == 8 {
			txCreated = vI(int64(binary.BigEndian.Uint64(v)))
		} else if v != nil {
			e.bad(txPath, []byte("date"), "len", v)
		}
		if v := get(txPath, txns, "bal"); len(v) == 8 {
			minedBal = vI(int64(binary.BigEndian.Uint64(v)))
		} else {
			e.bad(txPath, []byte("bal"), "len", v)
		}
	}
	e.sec("wallet").add("wallet", watch, lastAcct, lastImp, mgrCreated, txCreated, minedBal)

	// [masterkeys] (addressdb.go:318-341; snacl Marshal: salt, digest, LE64 N, R, P)
	for _, which := range []string{"pub", "priv"} {
		key := "m" + which
		v := get(mainPath, main, key)
		if v == nil {
			continue
		}
		if len(v) != 88 {
			e.bad(mainPath, []byte(key), "len", v)
			continue
		}
		e.sec("masterkeys").add("masterkey", which, vX(v[:32]), vX(v[32:64]),
			vU(binary.LittleEndian.Uint64(v[64:72])),
			vU(binary.LittleEndian.Uint64(v[72:80])),
			vU(binary.LittleEndian.Uint64(v[80:88])))
	}

	// [cryptokeys] (addressdb.go:443-466)
	for _, which := range []string{"pub", "priv"} {
		key := "c" + which
		if v := get(mainPath, main, key); v != nil {
			e.sec("cryptokeys").add("cryptokey", which, vX(v))
		}
	}

	// [cointypes] (addressdb.go:367-441)
	for _, ct := range []struct{ which, pub, priv string }{
		{"legacy", "ctpub", "ctpriv"},
		{"slip0044", "ctpub-slip0044", "ctpriv-slip0044"},
	} {
		pub := get(mainPath, main, ct.pub)
		priv := get(mainPath, main, ct.priv)
		if pub == nil && priv == nil {
			continue
		}
		encpub, encpriv := absent, absent
		if pub != nil {
			encpub = vX(pub)
		}
		if priv != nil {
			encpriv = vX(priv)
		}
		e.sec("cointypes").add("cointype", ct.which, encpub, encpriv,
			e.decryptPublic(mainPath, []byte(ct.pub), pub))
	}
	if v := get(mainPath, main, "slip0044acct0"); v != nil {
		e.slip0044Acct0(mainPath, v)
	}

	if err := e.accounts(); err != nil {
		return err
	}
	for _, name := range []string{"wallet", "masterkeys", "cryptokeys", "cointypes", "accounts", "accountnames"} {
		e.flush(name)
	}
	e.addresses()
	return nil
}

// slip0044Acct0 decodes the parked SLIP0044 account 0 row in the version-1
// account layout, as UpgradeToSLIP0044CoinType reads it
// (addressmanager.go:783-796; addressdb.go:518-608): type 0, then
// <len pub><pub><len priv><priv><LE32 next ext><LE32 next int><len name><name>.
func (e *exporter) slip0044Acct0(path []string, v []byte) {
	key := []byte("slip0044acct0")
	if len(v) < 5 || v[0] != 0 {
		e.bad(path, key, "type", v)
		return
	}
	raw, rest, ok := lenPrefixed(v[1:])
	if !ok || len(rest) != 0 {
		e.bad(path, key, "layout", v)
		return
	}
	pub, r, ok1 := lenPrefixed(raw)
	priv, r, ok2 := lenPrefixed(r)
	if !ok1 || !ok2 || len(r) < 12 {
		e.bad(path, key, "layout", v)
		return
	}
	next0, next1 := le32(r[0:4]), le32(r[4:8])
	name, r, ok3 := lenPrefixed(r[8:])
	if !ok3 || len(r) != 0 {
		e.bad(path, key, "layout", v)
		return
	}
	e.sec("cointypes").add("slip0044acct0", vX(pub), vX(priv), vU(uint64(next0)), vU(uint64(next1)),
		vS(name), e.decryptPublic(path, key, pub))
	e.slip0044Pub, e.slip0044Priv = pub, priv
}

func (e *exporter) accounts() error {
	acctPath := []string{"waddrmgr", "acct"}
	varsPath := []string{"waddrmgr", "acctvars"}
	ididxPath := []string{"waddrmgr", "acctididx"}
	nameidxPath := []string{"waddrmgr", "acctnameidx"}
	acct := e.bucket(acctPath...)
	vars := e.bucket(varsPath...)
	ididx := e.bucket(ididxPath...)
	nameidx := e.bucket(nameidxPath...)
	if acct == nil || vars == nil || ididx == nil || nameidx == nil {
		return fmt.Errorf("waddrmgr lacks acct, acctvars, acctididx or acctnameidx")
	}

	rows := make(map[uint32]bool)
	e.each(acctPath, acct, func(k, v []byte) {
		if len(k) != 4 {
			e.bad(acctPath, k, "key", v)
			return
		}
		num := le32(k)
		// <type><LE32 rdlen><rawData> (addressdb.go:518-537)
		if len(v) < 5 {
			e.bad(acctPath, k, "len", v)
			return
		}
		typ := v[0]
		// Bytes after rawData are ignored by the reader.
		raw, _, ok := lenPrefixed(v[1:])
		if !ok {
			e.bad(acctPath, k, "layout", v)
			return
		}
		// fetchDBAccount accepts types 1 and 2 only at version 26
		// (addressdb.go:803-851).
		if typ != 1 && typ != 2 {
			e.bad(acctPath, k, "type", v)
			return
		}
		// <len pub><pub><len priv><priv>, nothing left (addressdb.go:99-137)
		pub, r, ok1 := lenPrefixed(raw)
		priv, r, ok2 := lenPrefixed(r)
		if !ok1 || !ok2 || len(r) != 0 {
			e.bad(acctPath, k, "layout", v)
			return
		}
		rows[num] = true
		e.acctHasPriv[num] = len(priv) > 0

		vb := vars.Bucket(k)
		vpath := append(append([]string(nil), varsPath...), string(k))
		name, ext, intr, extret, intret := absent, absent, absent, absent, absent
		kdfSalt, kdfTime, kdfMem, kdfThreads := absent, absent, absent, absent
		if vb == nil {
			e.bad(varsPath, k, "missing", nil)
		} else {
			e.claims.claim(varsPath, k)
			u32 := func(key string) string {
				val := vb.Get([]byte(key))
				if val != nil {
					e.claims.claim(vpath, []byte(key))
				}
				if len(val) != 4 {
					e.bad(vpath, []byte(key), "len", val)
					return absent
				}
				return vU(uint64(le32(val)))
			}
			ext, intr = u32("extused"), u32("intused")
			extret, intret = u32("extret"), u32("intret")
			nv := vb.Get([]byte("name"))
			if nv != nil {
				e.claims.claim(vpath, []byte("name"))
			}
			name = vS(nv)
			if kv := vb.Get([]byte("kdf-params")); kv != nil {
				e.claims.claim(vpath, []byte("kdf-params"))
				e.acctKDF[num] = true
				// salt(16) LE32 time, LE32 memory, threads (kdf/kdf.go:56-65)
				if len(kv) != 25 {
					e.bad(vpath, []byte("kdf-params"), "len", kv)
				} else {
					kdfSalt = vX(kv[:16])
					kdfTime = vU(uint64(le32(kv[16:20])))
					kdfMem = vU(uint64(le32(kv[20:24])))
					kdfThreads = vU(uint64(kv[24]))
				}
			}
		}
		idname := absent
		if iv := ididx.Get(k); iv != nil {
			e.claims.claim(ididxPath, k)
			if n, rest, ok := lenPrefixed(iv); ok && len(rest) == 0 {
				idname = vS(n)
			} else {
				e.bad(ididxPath, k, "layout", iv)
			}
		}
		e.sec("accounts").add("account", vU(uint64(num)), vU(uint64(typ)), name, idname,
			vX(pub), vX(priv), kdfSalt, kdfTime, kdfMem, kdfThreads, ext, intr, extret, intret,
			e.decryptPublic(acctPath, k, pub))
	})
	e.acctNums = sortedU32(rows)

	// acctvars buckets and acctididx entries with no account row.
	c := vars.Cursor()
	for k, v := c.First(); k != nil; k, v = c.Next() {
		if v != nil || (len(k) == 4 && rows[le32(k)]) {
			continue
		}
		e.claims.claim(varsPath, k)
		vpath := append(append([]string(nil), varsPath...), string(k))
		e.each(vpath, vars.Bucket(k), func(_, _ []byte) {})
		e.orphan("acctvars", accountRef(k), "")
	}
	e.each(ididxPath, ididx, func(k, v []byte) {
		if len(k) == 4 && rows[le32(k)] {
			return
		}
		e.orphan("acctidname", accountRef(k), string(v))
	})

	// [accountnames] (addressdb.go:761-770)
	e.each(nameidxPath, nameidx, func(k, v []byte) {
		name, rest, ok := lenPrefixed(k)
		if !ok || len(rest) != 0 {
			e.bad(nameidxPath, k, "key", v)
			return
		}
		num := absent
		if len(v) == 4 {
			num = vU(uint64(le32(v)))
		} else {
			e.bad(nameidxPath, k, "len", v)
		}
		e.sec("accountnames").add("nameindex", vS(name), num)
	})
	return nil
}

// accountRef renders an account-number key for an orphan ref: the LE32
// number in decimal when the key has 4 bytes, otherwise "0x" and the key in
// hex (which no decimal number can be mistaken for).
func accountRef(k []byte) string {
	if len(k) == 4 {
		return strconv.FormatUint(uint64(le32(k)), 10)
	}
	return fmt.Sprintf("0x%x", k)
}

// apiAddresses asks udb for every address of every account
// (Manager.ForEachAccountAddress, addressmanager.go:2428) and indexes the
// answers by bucket key, SHA-256 of the hash160 (addressdb.go:1337-1350).
func (e *exporter) apiAddresses() {
	if e.mgr == nil {
		return
	}
	ns := e.dbtx.ReadBucket([]byte("waddrmgr"))
	for _, acct := range e.acctNums {
		err := recovered(func() error {
			return e.mgr.ForEachAccountAddress(ns, acct, func(ma udb.ManagedAddress) error {
				info := addrInfo{
					id:   append([]byte(nil), ma.AddrHash()...),
					addr: ma.Address().String(),
				}
				if pk, ok := ma.(udb.ManagedPubKeyAddress); ok && ma.Imported() {
					info.pubkey = append([]byte(nil), pk.PubKey()...)
					info.address = ma.Address()
				}
				key := sha256.Sum256(info.id)
				e.keyAddrs[string(key[:])] = info
				return nil
			})
		})
		if err != nil {
			e.bad([]string{"waddrmgr", "addracctidx"}, le32Bytes(acct), "api", nil)
		}
	}
}

// addrRow is a decoded waddrmgr/addr row (addressdb.go:1111-1295).
type addrRow struct {
	typName                                         string
	account                                         uint32
	added                                           uint64
	branch, index, encpub, encpriv, enchash, script string
	hasPriv                                         bool
}

// decodeAddrRow decodes an address row as fetchAddressByHash does, or
// returns the [bad] reason: type(1) LE32 account, LE64 addTime, unused
// byte, LE32 rdlen, rawData (addressdb.go:1111-1152).
func decodeAddrRow(v []byte) (addrRow, string) {
	if len(v) < 18 {
		return addrRow{}, "len"
	}
	r := addrRow{account: le32(v[1:5]), added: binary.LittleEndian.Uint64(v[5:13])}
	raw, _, ok := lenPrefixed(v[14:])
	if !ok {
		return addrRow{}, "layout"
	}
	r.branch, r.index, r.encpub, r.encpriv, r.enchash, r.script = absent, absent, absent, absent, absent, absent
	switch v[0] {
	case 0: // chained: LE32 branch, LE32 index (addressdb.go:1156-1188)
		if len(raw) != 8 {
			return addrRow{}, "layout"
		}
		r.typName = "chain"
		r.branch, r.index = vU(uint64(le32(raw[0:4]))), vU(uint64(le32(raw[4:8])))
	case 1: // imported key (addressdb.go:1190-1240)
		p, rest, ok1 := lenPrefixed(raw)
		q, _, ok2 := lenPrefixed(rest)
		if !ok1 || !ok2 {
			return addrRow{}, "layout"
		}
		r.typName = "key"
		r.encpub, r.encpriv = vX(p), vX(q)
		r.hasPriv = len(q) > 0
	case 2: // imported script (addressdb.go:1241-1295)
		h, rest, ok1 := lenPrefixed(raw)
		sc, _, ok2 := lenPrefixed(rest)
		if !ok1 || !ok2 {
			return addrRow{}, "layout"
		}
		r.typName = "script"
		r.enchash, r.script = vX(h), vX(sc)
	default:
		return addrRow{}, "type"
	}
	return r, ""
}

// addresses streams [addresses] in bucket order, which is the record
// order (the key is rendered as x, whose order is byte order), folding the
// flat and nested addracctidx entries; a second pass over addracctidx
// turns entries that fold into no row into orphans.
func (e *exporter) addresses() {
	addrPath := []string{"waddrmgr", "addr"}
	idxPath := []string{"waddrmgr", "addracctidx"}
	addr := e.bucket(addrPath...)
	idx := e.bucket(idxPath...)
	w := e.out.section("addresses")
	defer w.close()
	if addr == nil || idx == nil {
		e.bad([]string{"waddrmgr"}, []byte("addr"), "missing", nil)
		return
	}
	e.apiAddresses()

	e.each(addrPath, addr, func(k, v []byte) {
		r, reason := decodeAddrRow(v)
		if reason != "" {
			e.bad(addrPath, k, reason, v)
			return
		}
		if r.hasPriv {
			e.privKeyAddrs = append(e.privKeyAddrs, string(k))
		}
		id, addrStr, pubkey := absent, absent, absent
		if info, ok := e.keyAddrs[string(k)]; ok {
			id, addrStr = vX(info.id), vS([]byte(info.addr))
			if r.typName == "key" {
				pubkey = vX(info.pubkey)
			}
		} else if e.mgr != nil {
			e.bad(addrPath, k, "unreached", nil)
		}
		idxAcct := absent
		if iv := idx.Get(k); iv != nil {
			if len(iv) == 4 {
				idxAcct = vU(uint64(le32(iv)))
			} else {
				e.bad(idxPath, k, "len", iv)
			}
		}
		// The value of a nested set entry is never read (N10).
		inset := false
		if nb := idx.Bucket(le32Bytes(r.account)); nb != nil && nb.Get(k) != nil {
			inset = true
		}
		w.add("address", vX(k), id, addrStr, r.typName, vU(uint64(r.account)), idxAcct,
			vB(inset), r.branch, r.index, r.encpub, r.encpriv, r.enchash, r.script, pubkey, vU(r.added))
	})

	// rowAccount reports whether k names an address row that decodes, and
	// its account: only such rows fold index entries.
	rowAccount := func(k []byte) (uint32, bool) {
		v := addr.Get(k)
		if v == nil {
			return 0, false
		}
		r, reason := decodeAddrRow(v)
		return r.account, reason == ""
	}

	// Index entries that point at no row (or, nested, at a row of another
	// account). Every entry of addracctidx and of its nested buckets is
	// consumed: folded, an orphan or a [bad] record.
	e.claims.claimAll(idxPath)
	c := idx.Cursor()
	for k, v := c.First(); k != nil; k, v = c.Next() {
		if v != nil {
			if _, ok := rowAccount(k); ok {
				continue
			}
			if len(k) != 32 || len(v) != 4 {
				e.bad(idxPath, k, "layout", v)
				continue
			}
			e.orphan("addracct", fmt.Sprintf("%x", k), strconv.FormatUint(uint64(le32(v)), 10))
			continue
		}
		e.claims.claim(idxPath, k)
		npath := append(append([]string(nil), idxPath...), string(k))
		nb := idx.Bucket(k)
		if len(k) != 4 {
			e.each(npath, nb, func(kk, vv []byte) { e.bad(npath, kk, "key", vv) })
			continue
		}
		acct := le32(k)
		e.each(npath, nb, func(kk, _ []byte) {
			if a, ok := rowAccount(kk); ok && a == acct {
				return
			}
			e.orphan("acctaddrs", fmt.Sprintf("%d:%x", acct, kk), "")
		})
	}
}
