// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

package walletexport

// Sections [derived] and [privcheck]: answers of udb's public API, never
// rows. Every call is guarded by recover, so a Go panic becomes a "failed"
// record instead of ending the oracle.

import (
	"bytes"
	"errors"
	"sort"
	"strconv"

	dcrerrors "decred.org/dcrwallet/v5/errors"
	"decred.org/dcrwallet/v5/wallet/udb"
	"github.com/decred/dcrd/hdkeychain/v3"
)

func (e *exporter) derived() {
	d := e.sec("derived")
	failed := make(map[string]bool)
	ns := e.dbtx.ReadBucket([]byte("waddrmgr"))

	// Manager.CoinType (addressmanager.go:714-730): an error, such as
	// WatchingOnly for a wallet created from an xpub, renders as absent.
	coin := absent
	if err := recovered(func() error {
		if ct, err := e.mgr.CoinType(e.dbtx); err == nil {
			coin = vU(uint64(ct))
		}
		return nil
	}); err != nil {
		failed["coin"] = true
	} else {
		d.add("coin", coin)
	}

	// Store.MainChainTip (txmined.go:183)
	var tipHeight int32
	if err := recovered(func() error {
		_, tipHeight = e.store.MainChainTip(e.dbtx)
		return nil
	}); err != nil {
		failed["tipheight"] = true
	} else {
		d.add("tipheight", vI(int64(tipHeight)))
	}

	// Store.IsMissingMainChainCFilters (txmined.go:400)
	if err := recovered(func() error {
		d.add("missingcf", vB(e.store.IsMissingMainChainCFilters(e.dbtx)))
		return nil
	}); err != nil {
		failed["missingcf"] = true
	}

	// Store.UnspentOutputCount (txmined.go:2351)
	if err := recovered(func() error {
		d.add("utxocount", vU(uint64(e.store.UnspentOutputCount(e.dbtx))))
		return nil
	}); err != nil {
		failed["utxocount"] = true
	}

	// Manager.AccountProperties (addressmanager.go:550-607) for every
	// account row. AccountUnlocked is memory-only state and is not exported.
	for _, acct := range e.acctNums {
		err := recovered(func() error {
			p, err := e.mgr.AccountProperties(ns, acct)
			if err != nil {
				return err
			}
			d.add("acctprops", vU(uint64(acct)), vS([]byte(p.AccountName)), vU(uint64(p.AccountType)),
				vU(uint64(p.LastUsedExternalIndex)), vU(uint64(p.LastUsedInternalIndex)),
				vU(uint64(p.LastReturnedExternalIndex)), vU(uint64(p.LastReturnedInternalIndex)),
				vU(uint64(p.ImportedKeyCount)), vB(p.AccountEncrypted))
			return nil
		})
		if err != nil {
			failed["acctprops"] = true
		}
	}

	// Store.AccountBalances (txmined.go:3580) at minconf 0 and 1, every key
	// of the returned map (sorted: the map's order is random).
	for _, minconf := range []int32{0, 1} {
		err := recovered(func() error {
			bals, err := e.store.AccountBalances(e.dbtx, minconf)
			if err != nil {
				return err
			}
			accts := make([]uint32, 0, len(bals))
			for a := range bals {
				accts = append(accts, a)
			}
			sort.Slice(accts, func(i, j int) bool { return accts[i] < accts[j] })
			for _, a := range accts {
				b := bals[a]
				d.add("balance", vU(uint64(a)), vI(int64(minconf)),
					vI(int64(b.ImmatureCoinbaseRewards)), vI(int64(b.ImmatureStakeGeneration)),
					vI(int64(b.LockedByTickets)), vI(int64(b.Spendable)), vI(int64(b.Total)),
					vI(int64(b.VotingAuthority)), vI(int64(b.Unconfirmed)))
			}
			return nil
		})
		if err != nil {
			failed["balance"] = true
		}
	}

	// Store.UnspentTickets (txmined.go:2656) at the tip, immature included.
	if !failed["tipheight"] {
		err := recovered(func() error {
			hashes, err := e.store.UnspentTickets(e.dbtx, tipHeight, true)
			if err != nil {
				return err
			}
			for i := range hashes {
				d.add("unspentticket", vH(hashes[i][:]))
			}
			return nil
		})
		if err != nil {
			failed["unspentticket"] = true
		}
	}

	for item := range failed {
		d.add("failed", item)
	}
}

// privcheck proves that sealed secrets open to the keys their public halves
// name, without writing any secret or any digest of one (ADR-0001 6.27).
func (e *exporter) privcheck(privPass []byte, acctPass map[string]string) {
	p := e.sec("privcheck")
	ns := e.dbtx.ReadBucket([]byte("waddrmgr"))
	verdict := func(ok bool) string {
		if ok {
			return "yes"
		}
		return "no"
	}

	unlocked := false
	err := recovered(func() error { return e.mgr.Unlock(ns, privPass) })
	switch {
	case err == nil:
		unlocked = true
		p.add("mkeyok", "yes")
	case errors.Is(err, dcrerrors.Passphrase):
		p.add("mkeyok", "no")
	default:
		p.add("mkeyok", "error")
	}
	defer func() { _ = recovered(e.mgr.Lock) }()

	// One verdict per stored coin-type private key, legacy and SLIP0044,
	// each against its own public half: CoinTypePrivKey would only reach
	// the key in use (addressdb.go:367-392), and both private keys are
	// sealed at creation (addressmanager.go:2961-3007). The parked
	// SLIP0044 account 0 row, which UpgradeToSLIP0044CoinType installs as
	// account 0 (addressmanager.go:749-861), gets its own verdict.
	main := e.tx.Bucket([]byte("waddrmgr")).Bucket([]byte("main"))
	sealedPair := func(pub, priv []byte) string {
		if !unlocked {
			return "nopass"
		}
		ok := "error"
		_ = recovered(func() error {
			plainPub, err := e.mgr.Decrypt(udb.CKTPublic, pub)
			if err != nil {
				return err
			}
			plainPriv, err := e.mgr.Decrypt(udb.CKTPrivate, priv)
			if err != nil {
				return err
			}
			k, err := hdkeychain.NewKeyFromString(string(plainPriv), e.params)
			for i := range plainPriv {
				plainPriv[i] = 0
			}
			if err != nil {
				return err
			}
			ok = verdict(k.Neuter().String() == string(plainPub))
			k.Zero()
			return nil
		})
		return ok
	}
	for _, ct := range []struct{ which, pub, priv string }{
		{"legacy", "ctpub", "ctpriv"},
		{"slip0044", "ctpub-slip0044", "ctpriv-slip0044"},
	} {
		priv := main.Get([]byte(ct.priv))
		if priv == nil {
			continue
		}
		p.add("ctok", ct.which, sealedPair(main.Get([]byte(ct.pub)), priv))
	}
	if main.Get([]byte("slip0044acct0")) != nil {
		ok := "error"
		if e.slip0044Pub != nil {
			ok = sealedPair(e.slip0044Pub, e.slip0044Priv)
		}
		p.add("slip0044acct0ok", ok)
	}

	for _, acct := range e.acctNums {
		if !e.acctHasPriv[acct] {
			continue
		}
		via := "wallet"
		ready := unlocked
		if e.acctKDF[acct] {
			via = "account"
			pass, ok := acctPass[strconv.FormatUint(uint64(acct), 10)]
			ready = false
			if ok {
				err := recovered(func() error { return e.mgr.UnlockAccount(e.dbtx, acct, []byte(pass)) })
				if err != nil {
					p.add("acctok", vU(uint64(acct)), "no", via)
					continue
				}
				ready = true
			}
		}
		if !ready {
			p.add("acctok", vU(uint64(acct)), "nopass", via)
			continue
		}
		ok := "error"
		_ = recovered(func() error {
			xpriv, err := e.mgr.AccountExtendedPrivKey(e.dbtx, acct)
			if err != nil {
				return err
			}
			xpub, err := e.mgr.AccountExtendedPubKey(e.dbtx, acct)
			if err != nil {
				return err
			}
			ok = verdict(xpriv.Neuter().String() == xpub.String())
			return nil
		})
		p.add("acctok", vU(uint64(acct)), ok, via)
		if e.acctKDF[acct] {
			_ = recovered(func() error { return e.mgr.LockAccount(e.dbtx, acct) })
		}
	}

	for _, key := range e.privKeyAddrs {
		info, found := e.keyAddrs[key]
		if !found || !unlocked {
			p.add("addrok", vX([]byte(key)), "nopass")
			continue
		}
		ok := "error"
		_ = recovered(func() error {
			priv, done, err := e.mgr.PrivateKey(ns, info.address)
			if err != nil {
				return err
			}
			defer done()
			pub := priv.PubKey()
			ok = verdict(bytes.Equal(pub.SerializeCompressed(), info.pubkey) ||
				bytes.Equal(pub.SerializeUncompressed(), info.pubkey))
			return nil
		})
		p.add("addrok", vX([]byte(key)), ok)
	}
}
