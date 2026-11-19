// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

package main

// The wallet session: address issuance through dcrwallet's wallet package,
// which holds the gap-limit policy that udb lacks (wallet/addresses.go
// nextAddress, :505-649). The wallet package links the unmodified udb, so
// its Manager is a second one over the same database, caching state of its
// own: wallet.open closes the copied udb's Manager first, and only the
// session's operations, rt.checkpoint and the clock run until wallet.close
// opens the copied udb again. The address buffers that carry the gap
// state (the cursor past the last used child) are memory only and live
// for one session.
//
// Address issuance draws no randomness and reads no clock in the wallet
// package; its only database write is SyncAccountToAddrIndex
// (addresses.go:468-501), whose unmodified udb stamps each new address row
// with time.Now() (wallet/udb/addressdb.go:1360). The session's database
// handle is an interposer (seamDB) that rewrites that one field to the
// injected clock as the row is put, which is what the copied udb's
// seam.Now() gives, so sessions stay byte-for-byte deterministic. Opening
// the wallet (wallet.Open: udb.Upgrade, udb.Open) draws the passphrase
// hasher keys of its Manager from the real crypto/rand; they stay in
// memory and never reach a result or the database.

import (
	"bytes"
	"encoding/binary"
	"errors"

	"decred.org/dcrwallet/v5/wallet"
	"decred.org/dcrwallet/v5/wallet/walletdb"
	"github.com/jzbz/dcr-purse/tools/oracle/cmd/oprunner/internal/seam"
	"github.com/jzbz/dcr-purse/tools/oracle/cmd/oprunner/internal/udb"
)

// seamDriver is the walletdb driver name under which wallet.OpenDB gets
// the runner's own database handle, wrapped in seamDB.
const seamDriver = "oprunner-seam"

func init() {
	err := walletdb.RegisterDriver(walletdb.Driver{
		DbType: seamDriver,
		Create: func(...any) (walletdb.DB, error) {
			return nil, errors.New("oprunner-seam opens an existing handle only")
		},
		Open: func(args ...any) (walletdb.DB, error) {
			if len(args) != 1 {
				return nil, errors.New("oprunner-seam takes one walletdb.DB")
			}
			db, ok := args[0].(walletdb.DB)
			if !ok {
				return nil, errors.New("oprunner-seam takes one walletdb.DB")
			}
			return seamDB{db}, nil
		},
	})
	if err != nil {
		panic(err)
	}
}

var (
	waddrmgrKey = []byte("waddrmgr") // wallet/udb/migration.go:18
	addrBucket  = []byte("addr")     // wallet/udb/addressdb.go:180
)

// The address row (serializeAddressRow, wallet/udb/addressdb.go:1136-1152)
// and its key (the SHA-256 of the address ID, putAddress, :1337-1351).
const (
	addrKeyLen   = 32
	addrRowHead  = 18 // type, account, add time, unused byte, raw-data length
	addTimeStart = 5
	addTimeEnd   = 13
)

var (
	_ walletdb.DB          = seamDB{}
	_ walletdb.ReadWriteTx = seamTx{}
)

// seamDB passes everything to the runner's handle except that read-write
// transactions are wrapped. Close does not close the runner's handle.
type seamDB struct{ walletdb.DB }

func (d seamDB) BeginReadWriteTx() (walletdb.ReadWriteTx, error) {
	tx, err := d.DB.BeginReadWriteTx()
	if err != nil {
		return nil, err
	}
	return seamTx{tx}, nil
}

func (seamDB) Close() error { return nil }

type seamTx struct{ walletdb.ReadWriteTx }

func (t seamTx) ReadWriteBucket(key []byte) walletdb.ReadWriteBucket {
	b := t.ReadWriteTx.ReadWriteBucket(key)
	if b == nil || !bytes.Equal(key, waddrmgrKey) {
		return b
	}
	return mgrBucket{b}
}

type mgrBucket struct{ walletdb.ReadWriteBucket }

func (b mgrBucket) NestedReadWriteBucket(key []byte) walletdb.ReadWriteBucket {
	n := b.ReadWriteBucket.NestedReadWriteBucket(key)
	if n == nil || !bytes.Equal(key, addrBucket) {
		return n
	}
	return addrRows{n}
}

// addrRows rewrites the add time of every address row put.
type addrRows struct{ walletdb.ReadWriteBucket }

func (b addrRows) Put(key, value []byte) error {
	return b.ReadWriteBucket.Put(key, stampAddTime(key, value, seam.Now().Unix()))
}

// stampAddTime returns value with its add time set to unix when key and
// value have the shape of an address row (serializeAddressRow,
// wallet/udb/addressdb.go:1136-1152: a 32-byte key; 1 byte type 0 to 2,
// 4 bytes account, 8 bytes add time, 1 unused byte, 4 bytes raw-data
// length, then that many bytes), and value unchanged otherwise.
func stampAddTime(key, value []byte, unix int64) []byte {
	if len(key) != addrKeyLen || len(value) < addrRowHead || value[0] > 2 ||
		uint64(len(value)) != uint64(addrRowHead)+uint64(binary.LittleEndian.Uint32(value[14:18])) {
		return value
	}
	out := bytes.Clone(value)
	binary.LittleEndian.PutUint64(out[addTimeStart:addTimeEnd], uint64(unix))
	return out
}

// walletOpen runs wallet.Open over the runner's database handle (through
// seamDB) with the script's gap limit and no network backend, then closes
// the copied udb's Manager. An error from wallet.Open is the operation's
// result and leaves the copied udb open.
func (r *runner) walletOpen(a *opArgs) error {
	gap, err := a.optU32("gap_limit", wallet.DefaultGapLimit)
	if err != nil {
		return err
	}
	db, err := wallet.OpenDB(seamDriver, r.db)
	if err != nil {
		return err
	}
	w, err := wallet.Open(r.ctx, &wallet.Config{
		DB:            db,
		PubPassphrase: []byte(r.pubPass),
		GapLimit:      gap,
		Params:        r.params,
	})
	if err != nil {
		return err
	}
	if err := r.mgr.Close(); err != nil {
		return &runnerFault{"wallet.open: Manager.Close of the copied udb: " + err.Error()}
	}
	r.mgr, r.store = nil, nil
	r.session = w
	return nil
}

// walletClose ends the session and opens the copied udb on the same handle,
// as rt.reopen does without reopening the file: the copied Manager starts
// locked, with every cache rebuilt from the database.
func (r *runner) walletClose() error {
	r.session = nil
	var err error
	r.mgr, r.store, err = udb.Open(r.ctx, r.db, r.params, []byte(r.pubPass))
	return err
}

func gapPolicy(a *opArgs) ([]wallet.NextAddressCallOption, error) {
	if _, ok := a.m["policy"]; !ok {
		// No option: Go's zero policy, which is the error policy.
		return nil, nil
	}
	p, err := a.str("policy")
	if err != nil {
		return nil, err
	}
	switch p {
	case "error":
		return []wallet.NextAddressCallOption{wallet.WithGapPolicyError()}, nil
	case "ignore":
		return []wallet.NextAddressCallOption{wallet.WithGapPolicyIgnore()}, nil
	case "wrap":
		return []wallet.NextAddressCallOption{wallet.WithGapPolicyWrap()}, nil
	}
	return nil, scriptErrorf("%s: policy must be error, ignore or wrap", a.op)
}

func newAddress(internal bool) func(r *runner, c *opCtx, a *opArgs) (any, error) {
	return func(r *runner, c *opCtx, a *opArgs) (any, error) {
		acct, err := a.u32("account")
		if err != nil {
			return nil, err
		}
		opts, err := gapPolicy(a)
		if err != nil {
			return nil, err
		}
		next := r.session.NewExternalAddress
		if internal {
			next = r.session.NewInternalAddress
		}
		addr, err := next(r.ctx, acct, opts...)
		if err != nil {
			return nil, err
		}
		v := map[string]any{"addr": addr.String()}
		if ka, ok := addr.(wallet.KnownAddress); ok {
			v["accountname"] = ka.AccountName()
		}
		if bip, ok := addr.(wallet.BIP0044Address); ok {
			account, branch, child := bip.Path()
			v["path"] = map[string]any{"account": account, "branch": branch, "child": child}
		}
		return v, nil
	}
}

func init() {
	for name, def := range walletOps {
		ops[name] = def
	}
}

var walletOps = map[string]opDef{
	"wallet.new_external_address": {walletSession, "Wallet.NewExternalAddress", newAddress(false)},
	"wallet.new_internal_address": {walletSession, "Wallet.NewInternalAddress", newAddress(true)},
	"wallet.branch_next_indexes": {walletSession, "Wallet.BIP0044BranchNextIndexes", func(r *runner, c *opCtx, a *opArgs) (any, error) {
		acct, err := a.u32("account")
		if err != nil {
			return nil, err
		}
		ext, in, err := r.session.BIP0044BranchNextIndexes(r.ctx, acct)
		if err != nil {
			return nil, err
		}
		return map[string]any{"external": ext, "internal": in}, nil
	}},
}
