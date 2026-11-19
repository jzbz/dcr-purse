// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

package main

// The "full" scenario: a short script of udb operations that fills as many
// sections of the logical export as udb's public API allows without a chain
// backend. Transactions are built by hand (no signatures: the store never
// checks them) and recorded the way wallet.processTransactionRecord records
// them (wallet/chainntfns.go): InsertMinedTx or InsertMemPoolTx, then
// AddCredit, AddTicketCommitment or AddMultisigOut per relevant output, and
// RedeemTicketCommitments for a vote.

import (
	"context"
	"crypto/sha256"
	"encoding/binary"
	"fmt"
	"time"

	"decred.org/dcrwallet/v5/wallet/udb"
	"decred.org/dcrwallet/v5/wallet/walletdb"
	"github.com/decred/dcrd/blockchain/stake/v5"
	"github.com/decred/dcrd/chaincfg/chainhash"
	"github.com/decred/dcrd/chaincfg/v3"
	"github.com/decred/dcrd/dcrec"
	"github.com/decred/dcrd/dcrec/secp256k1/v4"
	"github.com/decred/dcrd/dcrutil/v4"
	"github.com/decred/dcrd/gcs/v4"
	"github.com/decred/dcrd/gcs/v4/blockcf2"
	"github.com/decred/dcrd/hdkeychain/v3"
	"github.com/decred/dcrd/txscript/v4/stdaddr"
	"github.com/decred/dcrd/wire"
)

const coin = 1e8

type scenario struct {
	ctx    context.Context
	db     walletdb.DB
	params *chaincfg.Params
	mgr    *udb.Manager
	store  *udb.Store
	priv   []byte
	clock  time.Time
	// chainClock is the base of block timestamps.
	chainClock time.Time
	nonce      uint32
	txs        int
}

// Passphrases and names the scenario uses; the tests name them too.
const (
	votingPass  = "voting-pass"
	acct1Pass   = "account-one-pass"
	vspHost     = "https://vsp.example.org"
	agendaID    = "reverttreasurypolicy"
	agendaID2   = "maxblocksize"
	agendaVer   = 10
	ticketAgVer = 11
)

func (s *scenario) update(f func(tx walletdb.ReadWriteTx) error) error {
	return walletdb.Update(s.ctx, s.db, f)
}

func addrmgrNs(tx walletdb.ReadWriteTx) walletdb.ReadWriteBucket {
	return tx.ReadWriteBucket([]byte("waddrmgr"))
}

func txmgrNs(tx walletdb.ReadWriteTx) walletdb.ReadWriteBucket {
	return tx.ReadWriteBucket([]byte("wtxmgr"))
}

// detBytes returns n bytes derived from a label, for keys, hashes and
// prevouts that only need to be fixed and distinct.
func detBytes(label string, n int) []byte {
	var out []byte
	for i := 0; len(out) < n; i++ {
		h := sha256.Sum256([]byte(fmt.Sprintf("%s/%d", label, i)))
		out = append(out, h[:]...)
	}
	return out[:n]
}

func detHash(label string) chainhash.Hash {
	var h chainhash.Hash
	copy(h[:], detBytes(label, 32))
	return h
}

func p2pkh(hash160 []byte) []byte {
	s := []byte{0x76, 0xa9, 0x14}
	s = append(s, hash160...)
	return append(s, 0x88, 0xac)
}

// stakeTagged prefixes a P2PKH script with a stake opcode (OP_SSTX 0xba,
// OP_SSGEN 0xbb, OP_SSTXCHANGE 0xbd).
func stakeTagged(op byte, hash160 []byte) []byte {
	return append([]byte{op}, p2pkh(hash160)...)
}

// walletHash160 is the hash160 of the wallet's address at
// account/branch/index.
func (s *scenario) walletHash160(acct, branch, index uint32) ([]byte, error) {
	var h []byte
	err := walletdb.View(s.ctx, s.db, func(tx walletdb.ReadTx) error {
		xpub, err := s.mgr.AccountBranchExtendedPubKey(tx, acct, branch)
		if err != nil {
			return err
		}
		child, err := xpub.Child(index)
		if err != nil {
			return err
		}
		h = dcrutil.Hash160(child.SerializedPubKey())
		return nil
	})
	return h, err
}

func (s *scenario) mustHash160(acct, branch, index uint32) []byte {
	h, err := s.walletHash160(acct, branch, index)
	if err != nil {
		panic(err)
	}
	return h
}

// extend connects one block with the given vote bits (bit 0 approves the
// parent's regular tree) whose filter covers the outputs of txs, and
// returns its metadata.
func (s *scenario) extend(voteBits uint16, txs ...*wire.MsgTx) (*udb.BlockMeta, error) {
	var meta *udb.BlockMeta
	err := s.update(func(tx walletdb.ReadWriteTx) error {
		tip, height := s.store.MainChainTip(tx)
		s.nonce++
		h := uint32(height + 1)
		header := &wire.BlockHeader{
			Version:    7,
			PrevBlock:  tip,
			MerkleRoot: detHash(fmt.Sprintf("merkle/%d/%d", h, s.nonce)),
			StakeRoot:  detHash(fmt.Sprintf("stake/%d/%d", h, s.nonce)),
			VoteBits:   voteBits,
			Bits:       s.params.PowLimitBits,
			Height:     h,
			Timestamp:  s.chainClock.Add(time.Duration(h) * 5 * time.Minute),
			Nonce:      s.nonce,
		}
		var data [][]byte
		for _, t := range txs {
			for _, out := range t.TxOut {
				data = append(data, out.PkScript)
			}
		}
		f, err := gcs.NewFilterV2(blockcf2.B, blockcf2.M, blockcf2.Key(&header.MerkleRoot), data)
		if err != nil {
			return err
		}
		hash := header.BlockHash()
		if err := s.store.ExtendMainChain(txmgrNs(tx), header, &hash, f); err != nil {
			return err
		}
		meta = &udb.BlockMeta{
			Block:    udb.Block{Hash: hash, Height: int32(h)},
			Time:     header.Timestamp,
			VoteBits: voteBits,
		}
		return nil
	})
	return meta, err
}

func (s *scenario) record(msg *wire.MsgTx, at time.Duration) (*udb.TxRecord, error) {
	rec, err := udb.NewTxRecordFromMsgTx(msg, s.clock.Add(at))
	if err != nil {
		return nil, err
	}
	s.txs++
	return rec, nil
}

type creditSpec struct {
	index   uint32
	change  bool
	account uint32
}

// mined records rec in block and adds its credits.
func (s *scenario) mined(rec *udb.TxRecord, block *udb.BlockMeta, credits ...creditSpec) error {
	return s.update(func(tx walletdb.ReadWriteTx) error {
		if err := s.store.InsertMinedTx(tx, rec, &block.Hash); err != nil {
			return fmt.Errorf("InsertMinedTx %v: %w", &rec.Hash, err)
		}
		for _, c := range credits {
			if err := s.store.AddCredit(tx, rec, block, c.index, c.change, c.account); err != nil {
				return fmt.Errorf("AddCredit %v:%d: %w", &rec.Hash, c.index, err)
			}
		}
		return nil
	})
}

func newTx() *wire.MsgTx {
	t := wire.NewMsgTx()
	t.Version = 1
	return t
}

func spend(t *wire.MsgTx, hash chainhash.Hash, index uint32, tree int8, value int64) {
	t.AddTxIn(wire.NewTxIn(wire.NewOutPoint(&hash, index, tree), value, nil))
}

func (s *scenario) full() error {
	// Unlock, for the account and key operations that need the private
	// key.
	if err := walletdb.View(s.ctx, s.db, func(tx walletdb.ReadTx) error {
		return s.mgr.Unlock(tx.ReadBucket([]byte("waddrmgr")), s.priv)
	}); err != nil {
		return fmt.Errorf("Unlock: %w", err)
	}

	// Accounts: two BIP0044 accounts (one renamed), an imported xpub
	// account, an imported voting account, a per-account passphrase.
	other, err := hdkeychain.NewMaster(detBytes("other-seed", 32), s.params)
	if err != nil {
		return err
	}
	otherAcct, err := other.Child(hdkeychain.HardenedKeyStart + 44)
	if err != nil {
		return err
	}
	voting, err := hdkeychain.NewMaster(detBytes("voting-seed", 32), s.params)
	if err != nil {
		return err
	}
	err = s.update(func(tx walletdb.ReadWriteTx) error {
		ns := addrmgrNs(tx)
		if _, err := s.mgr.NewAccount(ns, "second"); err != nil {
			return fmt.Errorf("NewAccount: %w", err)
		}
		if _, err := s.mgr.NewAccount(ns, "third"); err != nil {
			return fmt.Errorf("NewAccount: %w", err)
		}
		if err := s.mgr.RenameAccount(ns, 2, "renamed third"); err != nil {
			return fmt.Errorf("RenameAccount: %w", err)
		}
		if err := s.mgr.ImportXpubAccount(ns, "watched", otherAcct.Neuter()); err != nil {
			return fmt.Errorf("ImportXpubAccount: %w", err)
		}
		if _, err := s.mgr.ImportVotingAccount(tx, voting, []byte(votingPass), "voter"); err != nil {
			return fmt.Errorf("ImportVotingAccount: %w", err)
		}
		if err := s.mgr.SetAccountPassphrase(tx, 1, []byte(acct1Pass)); err != nil {
			return fmt.Errorf("SetAccountPassphrase: %w", err)
		}
		return nil
	})
	if err != nil {
		return err
	}

	// Addresses: both branches, the counters, an imported private key and
	// an imported 2-of-2 script (ImportPublicKey is refused outside
	// watching-only wallets, addressmanager.go:1297).
	key1 := secp256k1.PrivKeyFromBytes(detBytes("imported-key", 32))
	key2 := secp256k1.PrivKeyFromBytes(detBytes("imported-pubkey", 32))
	msScript := []byte{0x52, 0x21}
	msScript = append(msScript, key1.PubKey().SerializeCompressed()...)
	msScript = append(msScript, 0x21)
	msScript = append(msScript, key2.PubKey().SerializeCompressed()...)
	msScript = append(msScript, 0x52, 0xae)
	err = s.update(func(tx walletdb.ReadWriteTx) error {
		ns := addrmgrNs(tx)
		for _, a := range []struct{ acct, idx, branch uint32 }{
			{0, 5, 0}, {0, 2, 1}, {1, 1, 0}, {1, 0, 1},
		} {
			if err := s.mgr.SyncAccountToAddrIndex(ns, a.acct, a.idx, a.branch); err != nil {
				return fmt.Errorf("SyncAccountToAddrIndex: %w", err)
			}
		}
		if err := s.mgr.MarkUsedChildIndex(tx, 0, 0, 2); err != nil {
			return fmt.Errorf("MarkUsedChildIndex: %w", err)
		}
		if err := s.mgr.MarkUsedChildIndex(tx, 0, 1, 1); err != nil {
			return fmt.Errorf("MarkUsedChildIndex: %w", err)
		}
		if err := s.mgr.MarkReturnedChildIndex(tx, 0, 0, 4); err != nil {
			return fmt.Errorf("MarkReturnedChildIndex: %w", err)
		}
		wif, err := dcrutil.NewWIF(key1.Serialize(), s.params.PrivateKeyID, dcrec.STEcdsaSecp256k1)
		if err != nil {
			return err
		}
		if _, err := s.mgr.ImportPrivateKey(ns, wif); err != nil {
			return fmt.Errorf("ImportPrivateKey: %w", err)
		}
		if _, err := s.mgr.ImportScript(ns, msScript); err != nil {
			return fmt.Errorf("ImportScript: %w", err)
		}
		return nil
	})
	if err != nil {
		return err
	}
	p2sh, err := stdaddr.NewAddressScriptHashV0(msScript, s.params)
	if err != nil {
		return err
	}
	_, p2shScript := p2sh.PaymentScript()

	foreign := detBytes("foreign-hash160", 20)
	h00, h01, h02 := s.mustHash160(0, 0, 0), s.mustHash160(0, 0, 1), s.mustHash160(0, 0, 2)
	h03, h04, h05 := s.mustHash160(0, 0, 3), s.mustHash160(0, 0, 4), s.mustHash160(0, 0, 5)
	h0i0, h0i1 := s.mustHash160(0, 1, 0), s.mustHash160(0, 1, 1)
	h10, h11 := s.mustHash160(1, 0, 0), s.mustHash160(1, 0, 1)

	// Block 1: a coinbase paying account 0.
	cb := newTx()
	spend(cb, chainhash.Hash{}, wire.MaxPrevOutIndex, wire.TxTreeRegular, 50*coin)
	cb.AddTxOut(wire.NewTxOut(50*coin, p2pkh(h04)))
	b1, err := s.extend(1, cb)
	if err != nil {
		return err
	}
	cbRec, err := s.record(cb, 1*time.Minute)
	if err != nil {
		return err
	}
	if err := s.mined(cbRec, b1, creditSpec{0, false, 0}); err != nil {
		return err
	}

	// Block 2: A pays accounts 0 and 1 and two multisig outputs.
	a := newTx()
	spend(a, detHash("prevout-a"), 0, wire.TxTreeRegular, 80*coin)
	a.AddTxOut(wire.NewTxOut(50*coin, p2pkh(h00)))
	a.AddTxOut(wire.NewTxOut(20*coin, p2pkh(h10)))
	a.AddTxOut(wire.NewTxOut(5*coin, p2shScript))
	a.AddTxOut(wire.NewTxOut(3*coin, p2shScript))
	b2, err := s.extend(1, a)
	if err != nil {
		return err
	}
	aRec, err := s.record(a, 2*time.Minute)
	if err != nil {
		return err
	}
	if err := s.mined(aRec, b2, creditSpec{0, false, 0}, creditSpec{1, false, 1}); err != nil {
		return err
	}
	if err := s.update(func(tx walletdb.ReadWriteTx) error {
		for _, i := range []uint32{2, 3} {
			if err := s.store.AddMultisigOut(tx, aRec, b2, i); err != nil {
				return fmt.Errorf("AddMultisigOut: %w", err)
			}
		}
		return nil
	}); err != nil {
		return err
	}

	// Block 3: B spends A:0, paying out with change to account 0.
	b := newTx()
	spend(b, aRec.Hash, 0, wire.TxTreeRegular, 50*coin)
	b.AddTxOut(wire.NewTxOut(10*coin, p2pkh(foreign)))
	b.AddTxOut(wire.NewTxOut(39*coin+99990000, p2pkh(h0i0)))
	b3, err := s.extend(1, b)
	if err != nil {
		return err
	}
	bRec, err := s.record(b, 3*time.Minute)
	if err != nil {
		return err
	}
	if err := s.mined(bRec, b3, creditSpec{1, true, 0}); err != nil {
		return err
	}

	// Block 4: ticket T spends B:1, committing to account 0.
	const ticketPrice = 39*coin + 90000000
	t := newTx()
	spend(t, bRec.Hash, 1, wire.TxTreeRegular, 39*coin+99990000)
	t.AddTxOut(wire.NewTxOut(ticketPrice, stakeTagged(0xba, h01)))
	commit := []byte{0x6a, 0x1e}
	commit = append(commit, h02...)
	var amt [8]byte
	binary.LittleEndian.PutUint64(amt[:], 39*coin+99990000)
	commit = append(commit, amt[:]...)
	commit = append(commit, 0x00, 0x58)
	t.AddTxOut(wire.NewTxOut(0, commit))
	t.AddTxOut(wire.NewTxOut(0, stakeTagged(0xbd, foreign)))
	if stake.DetermineTxType(t) != stake.TxTypeSStx {
		return fmt.Errorf("scenario ticket is not a ticket: %v", stake.CheckSStx(t))
	}
	b4, err := s.extend(1, t)
	if err != nil {
		return err
	}
	tRec, err := s.record(t, 4*time.Minute)
	if err != nil {
		return err
	}
	if err := s.mined(tRec, b4, creditSpec{0, false, 0}); err != nil {
		return err
	}
	if err := s.update(func(tx walletdb.ReadWriteTx) error {
		return s.store.AddTicketCommitment(txmgrNs(tx), tRec, 1, 0)
	}); err != nil {
		return fmt.Errorf("AddTicketCommitment: %w", err)
	}

	// Block 5: vote V spends T:0 and pays the commitment address.
	v := newTx()
	spend(v, chainhash.Hash{}, wire.MaxPrevOutIndex, wire.TxTreeRegular, 1*coin)
	spend(v, tRec.Hash, 0, wire.TxTreeStake, ticketPrice)
	ref := []byte{0x6a, 0x24}
	ref = append(ref, b4.Hash[:]...)
	var hb [4]byte
	binary.LittleEndian.PutUint32(hb[:], uint32(b4.Height))
	ref = append(ref, hb[:]...)
	v.AddTxOut(wire.NewTxOut(0, ref))
	v.AddTxOut(wire.NewTxOut(0, []byte{0x6a, 0x02, 0x01, 0x00}))
	v.AddTxOut(wire.NewTxOut(ticketPrice+1*coin, stakeTagged(0xbb, h02)))
	if stake.DetermineTxType(v) != stake.TxTypeSSGen {
		return fmt.Errorf("scenario vote is not a vote: %v", stake.CheckSSGen(v))
	}
	b5, err := s.extend(1, v)
	if err != nil {
		return err
	}
	vRec, err := s.record(v, 5*time.Minute)
	if err != nil {
		return err
	}
	if err := s.mined(vRec, b5, creditSpec{2, false, 0}); err != nil {
		return err
	}
	if err := s.update(func(tx walletdb.ReadWriteTx) error {
		return s.store.RedeemTicketCommitments(txmgrNs(tx), vRec, b5)
	}); err != nil {
		return fmt.Errorf("RedeemTicketCommitments: %w", err)
	}

	// Block 6: C spends A:1 within account 1; block 7 disapproves block 6,
	// which moves C's credits and debits to the stake-invalidated buckets.
	c := newTx()
	spend(c, aRec.Hash, 1, wire.TxTreeRegular, 20*coin)
	c.AddTxOut(wire.NewTxOut(19*coin+99990000, p2pkh(h11)))
	b6, err := s.extend(1, c)
	if err != nil {
		return err
	}
	cRec, err := s.record(c, 6*time.Minute)
	if err != nil {
		return err
	}
	if err := s.mined(cRec, b6, creditSpec{0, true, 1}); err != nil {
		return err
	}
	if _, err := s.extend(0); err != nil {
		return err
	}

	// Block 8: E pays account 0; G spends multisig output A:3.
	e := newTx()
	spend(e, detHash("prevout-e"), 1, wire.TxTreeRegular, 31*coin)
	e.AddTxOut(wire.NewTxOut(30*coin, p2pkh(h03)))
	g := newTx()
	spend(g, aRec.Hash, 3, wire.TxTreeRegular, 3*coin)
	g.AddTxOut(wire.NewTxOut(2*coin+99990000, p2pkh(foreign)))
	// T2, a second ticket in block 8, stays live: its commitment remains
	// in the unspent index.
	t2 := newTx()
	spend(t2, detHash("prevout-t2"), 0, wire.TxTreeRegular, 20*coin)
	t2.AddTxOut(wire.NewTxOut(19*coin, stakeTagged(0xba, h01)))
	commit2 := []byte{0x6a, 0x1e}
	commit2 = append(commit2, h03...)
	binary.LittleEndian.PutUint64(amt[:], 20*coin)
	commit2 = append(commit2, amt[:]...)
	commit2 = append(commit2, 0x00, 0x58)
	t2.AddTxOut(wire.NewTxOut(0, commit2))
	t2.AddTxOut(wire.NewTxOut(0, stakeTagged(0xbd, foreign)))
	if stake.DetermineTxType(t2) != stake.TxTypeSStx {
		return fmt.Errorf("scenario ticket 2 is not a ticket: %v", stake.CheckSStx(t2))
	}
	b8, err := s.extend(1, e, g, t2)
	if err != nil {
		return err
	}
	eRec, err := s.record(e, 8*time.Minute)
	if err != nil {
		return err
	}
	if err := s.mined(eRec, b8, creditSpec{0, false, 0}); err != nil {
		return err
	}
	gRec, err := s.record(g, 8*time.Minute)
	if err != nil {
		return err
	}
	if err := s.mined(gRec, b8); err != nil {
		return err
	}
	t2Rec, err := s.record(t2, 8*time.Minute)
	if err != nil {
		return err
	}
	if err := s.mined(t2Rec, b8, creditSpec{0, false, 0}); err != nil {
		return err
	}
	if err := s.update(func(tx walletdb.ReadWriteTx) error {
		return s.store.AddTicketCommitment(txmgrNs(tx), t2Rec, 1, 0)
	}); err != nil {
		return fmt.Errorf("AddTicketCommitment: %w", err)
	}
	if err := s.update(func(tx walletdb.ReadWriteTx) error {
		op := wire.OutPoint{Hash: aRec.Hash, Index: 3}
		return s.store.SpendMultisigOut(txmgrNs(tx), &op, gRec.Hash, 0)
	}); err != nil {
		return fmt.Errorf("SpendMultisigOut: %w", err)
	}

	// Blocks 9 and 10, then a reorganization of block 10: its header and
	// filter stay behind as side-chain rows.
	if _, err := s.extend(1); err != nil {
		return err
	}
	if _, err := s.extend(1); err != nil {
		return err
	}
	if err := s.update(func(tx walletdb.ReadWriteTx) error {
		_, err := s.store.Rollback(tx, 10)
		return err
	}); err != nil {
		return fmt.Errorf("Rollback: %w", err)
	}
	if _, err := s.extend(1); err != nil {
		return err
	}

	// Unmined: D spends E:0 with change to account 0, saved unpublished
	// and with an expiry; F spends D's change and stays published.
	d := newTx()
	d.Expiry = 1000
	spend(d, eRec.Hash, 0, wire.TxTreeRegular, 30*coin)
	d.AddTxOut(wire.NewTxOut(12*coin, p2pkh(foreign)))
	d.AddTxOut(wire.NewTxOut(17*coin+99990000, p2pkh(h0i1)))
	dRec, err := s.record(d, 20*time.Minute)
	if err != nil {
		return err
	}
	f := newTx()
	spend(f, dRec.Hash, 1, wire.TxTreeRegular, 17*coin+99990000)
	f.AddTxOut(wire.NewTxOut(17*coin+99980000, p2pkh(h05)))
	fRec, err := s.record(f, 21*time.Minute)
	if err != nil {
		return err
	}
	if err := s.update(func(tx walletdb.ReadWriteTx) error {
		if err := s.store.InsertMemPoolTx(tx, dRec); err != nil {
			return fmt.Errorf("InsertMemPoolTx: %w", err)
		}
		if err := s.store.AddCredit(tx, dRec, nil, 1, true, 0); err != nil {
			return fmt.Errorf("AddCredit unmined: %w", err)
		}
		if err := s.store.SetPublished(tx, &dRec.Hash, false); err != nil {
			return fmt.Errorf("SetPublished: %w", err)
		}
		if err := s.store.InsertMemPoolTx(tx, fRec); err != nil {
			return fmt.Errorf("InsertMemPoolTx: %w", err)
		}
		return s.store.AddCredit(tx, fRec, nil, 0, false, 0)
	}); err != nil {
		return err
	}

	// Preferences, policies, a VSP ticket, the birthday and the rescan
	// marker.
	tspend := detHash("tspend")
	pikey := key2.PubKey().SerializeCompressed()
	return s.update(func(tx walletdb.ReadWriteTx) error {
		if err := udb.SetDefaultAgendaPreference(tx, agendaVer, agendaID, "no"); err != nil {
			return err
		}
		if err := udb.SetDefaultAgendaPreference(tx, agendaVer, agendaID2, "yes"); err != nil {
			return err
		}
		if err := udb.SetTicketAgendaPreference(tx, &tRec.Hash, ticketAgVer, agendaID2, "no"); err != nil {
			return err
		}
		if err := udb.SetTSpendPolicy(tx, &tspend, stake.TreasuryVoteYes); err != nil {
			return err
		}
		if err := udb.SetVSPTSpendPolicy(tx, &tRec.Hash, &tspend, stake.TreasuryVoteNo); err != nil {
			return err
		}
		if err := udb.SetTreasuryKeyPolicy(tx, pikey, stake.TreasuryVoteYes); err != nil {
			return err
		}
		if err := udb.SetVSPTreasuryKeyPolicy(tx, &tRec.Hash, pikey, stake.TreasuryVoteNo); err != nil {
			return err
		}
		if err := udb.SetVSPTicket(tx, &t2Rec.Hash, &udb.VSPTicket{
			FeeHash:     detHash("vsp-fee"),
			FeeTxStatus: uint32(udb.VSPFeeProcessConfirmed),
			Host:        vspHost,
			PubKey:      detBytes("vsp-pubkey", 32),
		}); err != nil {
			return err
		}
		if err := udb.SetBirthState(tx, &udb.BirthdayState{
			Hash:   b1.Hash,
			Height: uint32(b1.Height),
			Time:   s.clock,
		}); err != nil {
			return err
		}
		tip, _ := s.store.MainChainTip(tx)
		return s.store.UpdateProcessedTxsBlockMarker(tx, &tip)
	})
}
