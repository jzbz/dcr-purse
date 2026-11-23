// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

package main

// Operations on udb.Manager and the address manager's package functions.
// Each runs the Go call with the arguments a Go caller passes: a Manager
// method taking a bucket gets the waddrmgr bucket of the operation's
// transaction, one taking a transaction gets the transaction.

import (
	"github.com/decred/dcrd/dcrec"
	"github.com/decred/dcrd/dcrec/secp256k1/v4"
	"github.com/decred/dcrd/dcrutil/v4"
	"github.com/decred/dcrd/txscript/v4/stdaddr"
	"github.com/jzbz/dcr-purse/tools/oracle/cmd/oprunner/internal/udb"
)

func cryptoKeyType(a *opArgs) (udb.CryptoKeyType, error) {
	s, err := a.str("key")
	if err != nil {
		return 0, err
	}
	switch s {
	case "private":
		return udb.CKTPrivate, nil
	case "public":
		return udb.CKTPublic, nil
	}
	return 0, scriptErrorf("%s: key must be private or public", a.op)
}

func acctProps(p *udb.AccountProperties) map[string]any {
	return map[string]any{
		"account":      p.AccountNumber,
		"name":         p.AccountName,
		"type":         p.AccountType,
		"extused":      p.LastUsedExternalIndex,
		"intused":      p.LastUsedInternalIndex,
		"extret":       p.LastReturnedExternalIndex,
		"intret":       p.LastReturnedInternalIndex,
		"importedkeys": p.ImportedKeyCount,
		"encrypted":    p.AccountEncrypted,
		"unlocked":     p.AccountUnlocked,
	}
}

// addrLabel reads an import operation's optional "label" (before the
// call, so that a failed import still consumes it).
func (r *runner) addrLabel(a *opArgs) (string, error) {
	l, err := a.optStr("label", "")
	if err != nil {
		return "", err
	}
	if _, dup := r.addrs[l]; dup && l != "" {
		return "", scriptErrorf("%s: address label %q already used", a.op, l)
	}
	return l, nil
}

// labelAddress stores the address of a successful import under label.
func (r *runner) labelAddress(label string, m udb.ManagedAddress) {
	if label != "" {
		r.addrs[label] = m.Address()
	}
}

func init() {
	for name, def := range mgrOps {
		ops[name] = def
	}
}

var mgrOps = map[string]opDef{
	"mgr.watching_only": {noTx, "Manager.WatchingOnly", func(r *runner, c *opCtx, a *opArgs) (any, error) {
		return r.mgr.WatchingOnly(), nil
	}},
	"mgr.is_locked": {noTx, "Manager.IsLocked", func(r *runner, c *opCtx, a *opArgs) (any, error) {
		return r.mgr.IsLocked(), nil
	}},
	"mgr.lock": {noTx, "Manager.Lock", func(r *runner, c *opCtx, a *opArgs) (any, error) {
		return nil, r.mgr.Lock()
	}},
	"mgr.unlocked_with_passphrase": {noTx, "Manager.UnlockedWithPassphrase", func(r *runner, c *opCtx, a *opArgs) (any, error) {
		p, err := a.passphrase("pass")
		if err != nil {
			return nil, err
		}
		return nil, r.mgr.UnlockedWithPassphrase(p)
	}},
	"mgr.chain_params": {noTx, "Manager.ChainParams", func(r *runner, c *opCtx, a *opArgs) (any, error) {
		return r.mgr.ChainParams().Name, nil
	}},
	"mgr.encrypt": {noTx, "Manager.Encrypt", func(r *runner, c *opCtx, a *opArgs) (any, error) {
		kt, err := cryptoKeyType(a)
		if err != nil {
			return nil, err
		}
		in, err := a.hexBytes("data")
		if err != nil {
			return nil, err
		}
		out, err := r.mgr.Encrypt(kt, in)
		if err != nil {
			return nil, err
		}
		return map[string]any{"sealed~r": hexStr(out)}, nil
	}},
	"mgr.decrypt": {noTx, "Manager.Decrypt", func(r *runner, c *opCtx, a *opArgs) (any, error) {
		kt, err := cryptoKeyType(a)
		if err != nil {
			return nil, err
		}
		in, err := a.hexBytes("data")
		if err != nil {
			return nil, err
		}
		out, err := r.mgr.Decrypt(kt, in)
		if err != nil {
			return nil, err
		}
		return hexStr(out), nil
	}},
	"mgr.unlock": {readTx, "Manager.Unlock", func(r *runner, c *opCtx, a *opArgs) (any, error) {
		p, err := a.passphrase("pass")
		if err != nil {
			return nil, err
		}
		return nil, r.mgr.Unlock(c.addrmgr(), p)
	}},
	"mgr.unlock_account": {readTx, "Manager.UnlockAccount", func(r *runner, c *opCtx, a *opArgs) (any, error) {
		acct, err := a.u32("account")
		if err != nil {
			return nil, err
		}
		p, err := a.passphrase("pass")
		if err != nil {
			return nil, err
		}
		return nil, r.mgr.UnlockAccount(c.rtx, acct, p)
	}},
	"mgr.lock_account": {readTx, "Manager.LockAccount", func(r *runner, c *opCtx, a *opArgs) (any, error) {
		acct, err := a.u32("account")
		if err != nil {
			return nil, err
		}
		return nil, r.mgr.LockAccount(c.rtx, acct)
	}},
	"mgr.set_account_passphrase": {writeTx, "Manager.SetAccountPassphrase", func(r *runner, c *opCtx, a *opArgs) (any, error) {
		acct, err := a.u32("account")
		if err != nil {
			return nil, err
		}
		p, err := a.passphrase("pass")
		if err != nil {
			return nil, err
		}
		if err := r.mgr.SetAccountPassphrase(c.wtx, acct, p); err != nil {
			return nil, err
		}
		if len(p) == 0 {
			delete(r.acctPass, acct)
		} else {
			r.acctPass[acct] = string(p)
		}
		return nil, nil
	}},
	"mgr.account_has_passphrase": {readTx, "Manager.AccountHasPassphrase", func(r *runner, c *opCtx, a *opArgs) (any, error) {
		acct, err := a.u32("account")
		if err != nil {
			return nil, err
		}
		has, unlocked := r.mgr.AccountHasPassphrase(c.rtx, acct)
		return map[string]any{"has": has, "unlocked": unlocked}, nil
	}},
	"mgr.change_passphrase": {writeTx, "Manager.ChangePassphrase", func(r *runner, c *opCtx, a *opArgs) (any, error) {
		old, err := a.passphrase("old")
		if err != nil {
			return nil, err
		}
		nw, err := a.passphrase("new")
		if err != nil {
			return nil, err
		}
		private, err := a.boolean("private")
		if err != nil {
			return nil, err
		}
		if err := r.mgr.ChangePassphrase(c.addrmgrW(), old, nw, private); err != nil {
			return nil, err
		}
		if private {
			s := string(nw)
			r.privPass = &s
		} else {
			r.pubPass = string(nw)
		}
		return nil, nil
	}},
	"mgr.convert_to_watching_only": {writeTx, "Manager.ConvertToWatchingOnly", func(r *runner, c *opCtx, a *opArgs) (any, error) {
		if err := r.mgr.ConvertToWatchingOnly(c.addrmgrW()); err != nil {
			return nil, err
		}
		r.privPass = nil
		r.acctPass = make(map[uint32]string)
		return nil, nil
	}},
	"mgr.account_properties": {readTx, "Manager.AccountProperties", func(r *runner, c *opCtx, a *opArgs) (any, error) {
		acct, err := a.u32("account")
		if err != nil {
			return nil, err
		}
		p, err := r.mgr.AccountProperties(c.addrmgr(), acct)
		if err != nil {
			return nil, err
		}
		return acctProps(p), nil
	}},
	"mgr.account_extended_pub_key": {readTx, "Manager.AccountExtendedPubKey", func(r *runner, c *opCtx, a *opArgs) (any, error) {
		acct, err := a.u32("account")
		if err != nil {
			return nil, err
		}
		k, err := r.mgr.AccountExtendedPubKey(c.rtx, acct)
		if err != nil {
			return nil, err
		}
		return k.String(), nil
	}},
	"mgr.account_extended_priv_key": {readTx, "Manager.AccountExtendedPrivKey", func(r *runner, c *opCtx, a *opArgs) (any, error) {
		acct, err := a.u32("account")
		if err != nil {
			return nil, err
		}
		k, err := r.mgr.AccountExtendedPrivKey(c.rtx, acct)
		if err != nil {
			return nil, err
		}
		return k.String(), nil
	}},
	"mgr.account_branch_extended_pub_key": {readTx, "Manager.AccountBranchExtendedPubKey", func(r *runner, c *opCtx, a *opArgs) (any, error) {
		acct, err := a.u32("account")
		if err != nil {
			return nil, err
		}
		branch, err := a.u32("branch")
		if err != nil {
			return nil, err
		}
		k, err := r.mgr.AccountBranchExtendedPubKey(c.rtx, acct, branch)
		if err != nil {
			return nil, err
		}
		return k.String(), nil
	}},
	"mgr.coin_type_priv_key": {readTx, "Manager.CoinTypePrivKey", func(r *runner, c *opCtx, a *opArgs) (any, error) {
		k, err := r.mgr.CoinTypePrivKey(c.rtx)
		if err != nil {
			return nil, err
		}
		return k.String(), nil
	}},
	"mgr.coin_type": {readTx, "Manager.CoinType", func(r *runner, c *opCtx, a *opArgs) (any, error) {
		return r.mgr.CoinType(c.rtx)
	}},
	"mgr.upgrade_to_slip0044_coin_type": {writeTx, "Manager.UpgradeToSLIP0044CoinType", func(r *runner, c *opCtx, a *opArgs) (any, error) {
		// dcr-purse's upgrade drops account 0's own passphrase, where Go
		// keeps its Argon2id parameters over a key they no longer seal
		// (QK-0767), and reports the drop; Go's call returns only an
		// error. The runner states the same report from Go's state: whether
		// account 0 had parameters before the call, read from the bucket
		// without the Manager, so its cache is not touched.
		had := account0HasKDF(c)
		if err := r.mgr.UpgradeToSLIP0044CoinType(c.wtx); err != nil {
			return nil, err
		}
		if had {
			return map[string]any{"passphrase_dropped": true}, nil
		}
		return nil, nil
	}},
	"mgr.address": {readTx, "Manager.Address", func(r *runner, c *opCtx, a *opArgs) (any, error) {
		addr, err := r.argAddress(c, a, "addr")
		if err != nil {
			return nil, err
		}
		m, err := r.mgr.Address(c.addrmgr(), addr)
		if err != nil {
			return nil, err
		}
		return managedAddressValue(m), nil
	}},
	"mgr.addr_account": {readTx, "Manager.AddrAccount", func(r *runner, c *opCtx, a *opArgs) (any, error) {
		addr, err := r.argAddress(c, a, "addr")
		if err != nil {
			return nil, err
		}
		return r.mgr.AddrAccount(c.addrmgr(), addr)
	}},
	"mgr.exists_hash160": {readTx, "Manager.ExistsHash160", func(r *runner, c *opCtx, a *opArgs) (any, error) {
		h, err := a.hexBytes("hash160")
		if err != nil {
			return nil, err
		}
		return r.mgr.ExistsHash160(c.addrmgr(), h), nil
	}},
	"mgr.import_private_key": {writeTx, "dcrutil.NewWIF, Manager.ImportPrivateKey", func(r *runner, c *opCtx, a *opArgs) (any, error) {
		key, err := a.hexBytes("key")
		if err != nil {
			return nil, err
		}
		if len(key) != secp256k1.PrivKeyBytesLen {
			return nil, scriptErrorf("%s: key must be 32 bytes", a.op)
		}
		wif, err := dcrutil.NewWIF(key, r.params.PrivateKeyID, dcrec.STEcdsaSecp256k1)
		if err != nil {
			return nil, scriptErrorf("%s: NewWIF: %v", a.op, err)
		}
		label, err := r.addrLabel(a)
		if err != nil {
			return nil, err
		}
		m, err := r.mgr.ImportPrivateKey(c.addrmgrW(), wif)
		if err != nil {
			return nil, err
		}
		r.labelAddress(label, m)
		return managedAddressValue(m), nil
	}},
	"mgr.import_public_key": {writeTx, "Manager.ImportPublicKey", func(r *runner, c *opCtx, a *opArgs) (any, error) {
		pk, err := a.hexBytes("pubkey")
		if err != nil {
			return nil, err
		}
		label, err := r.addrLabel(a)
		if err != nil {
			return nil, err
		}
		m, err := r.mgr.ImportPublicKey(c.addrmgrW(), pk)
		if err != nil {
			return nil, err
		}
		r.labelAddress(label, m)
		return managedAddressValue(m), nil
	}},
	"mgr.import_script": {writeTx, "Manager.ImportScript", func(r *runner, c *opCtx, a *opArgs) (any, error) {
		s, err := a.hexBytes("script")
		if err != nil {
			return nil, err
		}
		label, err := r.addrLabel(a)
		if err != nil {
			return nil, err
		}
		m, err := r.mgr.ImportScript(c.addrmgrW(), s)
		if err != nil {
			return nil, err
		}
		r.labelAddress(label, m)
		return managedAddressValue(m), nil
	}},
	"mgr.import_xpub_account": {writeTx, "Manager.ImportXpubAccount", func(r *runner, c *opCtx, a *opArgs) (any, error) {
		name, err := a.str("name")
		if err != nil {
			return nil, err
		}
		raw, ok := a.raw("xpub")
		if !ok {
			return nil, scriptErrorf("%s: missing argument \"xpub\"", a.op)
		}
		xpub, err := r.keySpec(raw, false)
		if err != nil {
			return nil, err
		}
		return nil, r.mgr.ImportXpubAccount(c.addrmgrW(), name, xpub)
	}},
	"mgr.import_voting_account": {writeTx, "Manager.ImportVotingAccount", func(r *runner, c *opCtx, a *opArgs) (any, error) {
		name, err := a.str("name")
		if err != nil {
			return nil, err
		}
		p, err := a.passphrase("pass")
		if err != nil {
			return nil, err
		}
		raw, ok := a.raw("xpriv")
		if !ok {
			return nil, scriptErrorf("%s: missing argument \"xpriv\"", a.op)
		}
		key, err := r.keySpec(raw, true)
		if err != nil {
			return nil, err
		}
		acct, err := r.mgr.ImportVotingAccount(c.wtx, key, p, name)
		if err != nil {
			return nil, err
		}
		if len(p) != 0 {
			r.acctPass[acct] = string(p)
		}
		return acct, nil
	}},
	"mgr.lookup_account": {readTx, "Manager.LookupAccount", func(r *runner, c *opCtx, a *opArgs) (any, error) {
		name, err := a.str("name")
		if err != nil {
			return nil, err
		}
		return r.mgr.LookupAccount(c.addrmgr(), name)
	}},
	"mgr.mark_used": {writeTx, "Manager.MarkUsed", func(r *runner, c *opCtx, a *opArgs) (any, error) {
		addr, err := r.argAddress(c, a, "addr")
		if err != nil {
			return nil, err
		}
		return nil, r.mgr.MarkUsed(c.wtx, addr)
	}},
	"mgr.mark_used_child_index": {writeTx, "Manager.MarkUsedChildIndex", func(r *runner, c *opCtx, a *opArgs) (any, error) {
		acct, branch, child, err := acctBranchChild(a)
		if err != nil {
			return nil, err
		}
		return nil, r.mgr.MarkUsedChildIndex(c.wtx, acct, branch, child)
	}},
	"mgr.mark_returned_child_index": {writeTx, "Manager.MarkReturnedChildIndex", func(r *runner, c *opCtx, a *opArgs) (any, error) {
		acct, branch, child, err := acctBranchChild(a)
		if err != nil {
			return nil, err
		}
		return nil, r.mgr.MarkReturnedChildIndex(c.wtx, acct, branch, child)
	}},
	"mgr.sync_account_to_addr_index": {writeTx, "Manager.SyncAccountToAddrIndex", func(r *runner, c *opCtx, a *opArgs) (any, error) {
		acct, err := a.u32("account")
		if err != nil {
			return nil, err
		}
		idx, err := a.u32("index")
		if err != nil {
			return nil, err
		}
		branch, err := a.u32("branch")
		if err != nil {
			return nil, err
		}
		return nil, r.mgr.SyncAccountToAddrIndex(c.addrmgrW(), acct, idx, branch)
	}},
	"mgr.record_derived_address": {writeTx, "Manager.RecordDerivedAddress", func(r *runner, c *opCtx, a *opArgs) (any, error) {
		acct, branch, child, err := acctBranchChild(a)
		if err != nil {
			return nil, err
		}
		var pk []byte
		if a.has("pubkey") {
			if pk, err = a.hexBytes("pubkey"); err != nil {
				return nil, err
			}
		} else {
			// The public key of the derived child, as a Go caller
			// (wallet discovery) passes it.
			xpub, err := r.mgr.AccountBranchExtendedPubKey(c.rtx, acct, branch)
			if err != nil {
				return nil, err
			}
			k, err := xpub.Child(child)
			if err != nil {
				return nil, err
			}
			pk = k.SerializedPubKey()
		}
		return nil, r.mgr.RecordDerivedAddress(c.wtx, acct, branch, child, pk)
	}},
	"udb.validate_account_name": {noTx, "ValidateAccountName", func(r *runner, c *opCtx, a *opArgs) (any, error) {
		name, err := a.str("name")
		if err != nil {
			return nil, err
		}
		return nil, udb.ValidateAccountName(name)
	}},
	"mgr.new_account": {writeTx, "Manager.NewAccount", func(r *runner, c *opCtx, a *opArgs) (any, error) {
		name, err := a.str("name")
		if err != nil {
			return nil, err
		}
		return r.mgr.NewAccount(c.addrmgrW(), name)
	}},
	"mgr.rename_account": {writeTx, "Manager.RenameAccount", func(r *runner, c *opCtx, a *opArgs) (any, error) {
		acct, err := a.u32("account")
		if err != nil {
			return nil, err
		}
		name, err := a.str("name")
		if err != nil {
			return nil, err
		}
		return nil, r.mgr.RenameAccount(c.addrmgrW(), acct, name)
	}},
	"mgr.account_name": {readTx, "Manager.AccountName", func(r *runner, c *opCtx, a *opArgs) (any, error) {
		acct, err := a.u32("account")
		if err != nil {
			return nil, err
		}
		return r.mgr.AccountName(c.addrmgr(), acct)
	}},
	"mgr.for_each_account": {readTx, "Manager.ForEachAccount", func(r *runner, c *opCtx, a *opArgs) (any, error) {
		out := []any{}
		err := r.mgr.ForEachAccount(c.addrmgr(), func(acct uint32) error {
			out = append(out, acct)
			return nil
		})
		return out, err
	}},
	"mgr.last_account": {readTx, "Manager.LastAccount", func(r *runner, c *opCtx, a *opArgs) (any, error) {
		return r.mgr.LastAccount(c.addrmgr())
	}},
	"mgr.last_imported_account": {readTx, "Manager.LastImportedAccount", func(r *runner, c *opCtx, a *opArgs) (any, error) {
		return r.mgr.LastImportedAccount(c.rtx)
	}},
	"mgr.for_each_account_address": {readTx, "Manager.ForEachAccountAddress", func(r *runner, c *opCtx, a *opArgs) (any, error) {
		acct, err := a.u32("account")
		if err != nil {
			return nil, err
		}
		out := []any{}
		err = r.mgr.ForEachAccountAddress(c.addrmgr(), acct, func(m udb.ManagedAddress) error {
			out = append(out, managedAddressValue(m))
			return nil
		})
		return out, err
	}},
	"mgr.for_each_active_account_address": {readTx, "Manager.ForEachActiveAccountAddress", func(r *runner, c *opCtx, a *opArgs) (any, error) {
		acct, err := a.u32("account")
		if err != nil {
			return nil, err
		}
		out := []any{}
		err = r.mgr.ForEachActiveAccountAddress(c.addrmgr(), acct, func(m udb.ManagedAddress) error {
			out = append(out, managedAddressValue(m))
			return nil
		})
		return out, err
	}},
	"mgr.for_each_active_address": {readTx, "Manager.ForEachActiveAddress", func(r *runner, c *opCtx, a *opArgs) (any, error) {
		out := []any{}
		err := r.mgr.ForEachActiveAddress(c.addrmgr(), func(addr stdaddr.Address) error {
			out = append(out, addr.String())
			return nil
		})
		return out, err
	}},
	"mgr.private_key": {readTx, "Manager.PrivateKey", func(r *runner, c *opCtx, a *opArgs) (any, error) {
		addr, err := r.argAddress(c, a, "addr")
		if err != nil {
			return nil, err
		}
		key, done, err := r.mgr.PrivateKey(c.addrmgr(), addr)
		if err != nil {
			return nil, err
		}
		defer done()
		return map[string]any{
			"key":    hexStr(key.Serialize()),
			"pubkey": hexStr(key.PubKey().SerializeCompressed()),
		}, nil
	}},
	"mgr.have_private_key": {readTx, "Manager.HavePrivateKey", func(r *runner, c *opCtx, a *opArgs) (any, error) {
		addr, err := r.argAddress(c, a, "addr")
		if err != nil {
			return nil, err
		}
		return r.mgr.HavePrivateKey(c.addrmgr(), addr)
	}},
	"mgr.redeem_script": {readTx, "Manager.RedeemScript", func(r *runner, c *opCtx, a *opArgs) (any, error) {
		addr, err := r.argAddress(c, a, "addr")
		if err != nil {
			return nil, err
		}
		s, err := r.mgr.RedeemScript(c.addrmgr(), addr)
		if err != nil {
			return nil, err
		}
		return hexStr(s), nil
	}},
	"udb.coin_types": {noTx, "CoinTypes", func(r *runner, c *opCtx, a *opArgs) (any, error) {
		legacy, slip := udb.CoinTypes(r.params)
		return map[string]any{"legacy": legacy, "slip0044": slip}, nil
	}},
	"udb.hd_keys_from_seed": {noTx, "HDKeysFromSeed", func(r *runner, c *opCtx, a *opArgs) (any, error) {
		seed, err := a.hexBytes("seed")
		if err != nil {
			return nil, err
		}
		ctl, cts, al, as, err := udb.HDKeysFromSeed(seed, r.params)
		if err != nil {
			return nil, err
		}
		return map[string]any{
			"cointype_legacy": ctl.String(), "cointype_slip0044": cts.String(),
			"account0_legacy": al.String(), "account0_slip0044": as.String(),
		}, nil
	}},
	"udb.is_imported_voting": {noTx, "IsImportedVoting", func(r *runner, c *opCtx, a *opArgs) (any, error) {
		t, err := a.ranged("type", 0, 255)
		if err != nil {
			return nil, err
		}
		return udb.IsImportedVoting(uint8(t)), nil
	}},
}

// account0HasKDF reports whether account 0's variables hold Argon2id
// parameters (udb's acctVarsBucketName/<account>/acctVarKDF,
// wallet/udb/addressdb.go:179, :1037), the mark of an account with its own
// passphrase.
func account0HasKDF(c *opCtx) bool {
	vars := c.addrmgr().NestedReadBucket([]byte("acctvars"))
	if vars == nil {
		return false
	}
	acct := vars.NestedReadBucket([]byte{0, 0, 0, 0})
	return acct != nil && acct.Get([]byte("kdf-params")) != nil
}

func acctBranchChild(a *opArgs) (acct, branch, child uint32, err error) {
	if acct, err = a.u32("account"); err != nil {
		return
	}
	if branch, err = a.u32("branch"); err != nil {
		return
	}
	child, err = a.u32("child")
	return
}
