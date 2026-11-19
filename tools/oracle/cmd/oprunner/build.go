// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

package main

// Fixture construction: transactions (wrapped in udb.NewTxRecordFromMsgTx
// or udb.NewTxRecord, as every Go caller does before handing a transaction
// to the store) and block headers with their version 2 filters (handed to
// Store.ExtendMainChain). The store checks no signature and no proof of
// work, so neither is made.

import (
	"bytes"
	"crypto/sha256"
	"encoding/binary"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"time"

	"github.com/decred/dcrd/chaincfg/chainhash"
	"github.com/decred/dcrd/gcs/v4"
	"github.com/decred/dcrd/gcs/v4/blockcf2"
	"github.com/decred/dcrd/txscript/v4/stdaddr"
	"github.com/decred/dcrd/wire"
	"github.com/jzbz/dcr-purse/tools/oracle/cmd/oprunner/internal/seam"
	"github.com/jzbz/dcr-purse/tools/oracle/cmd/oprunner/internal/udb"
)

type inputArg struct {
	// Prev is an outpoint {tx, index, tree?}, or "coinbase" / "stakebase"
	// for the null previous outpoint (all-zero hash, index 0xffffffff).
	Prev     json.RawMessage `json:"prev"`
	Value    int64           `json:"value"`
	Sequence *uint32         `json:"sequence"`
	Script   string          `json:"script"`
	Tree     *int8           `json:"tree"`
}

type outputArg struct {
	Value int64 `json:"value"`
	// Exactly one of the following.
	Pay        json.RawMessage `json:"pay"`
	Script     *string         `json:"script"`
	Commitment *struct {
		Pay    json.RawMessage `json:"pay"`
		Amount int64           `json:"amount"`
		Limits *uint16         `json:"limits"`
	} `json:"commitment"`
	VoteBlock json.RawMessage `json:"voteblock"`
	VoteBits  *string         `json:"votebits"`
	OpReturn  *string         `json:"opreturn"`
	// Tag prefixes a "pay" script with a stake opcode.
	Tag string `json:"tag"`
}

var stakeTags = map[string]byte{
	"sstx": 0xba, "ssgen": 0xbb, "ssrtx": 0xbc, "sstxchange": 0xbd,
	"tgen": 0xc3,
}

// pushData returns a minimal canonical push of b (len(b) <= 75 here).
func pushData(b []byte) []byte {
	if len(b) > 75 {
		panic("push too long")
	}
	return append([]byte{byte(len(b))}, b...)
}

func (r *runner) buildOutput(c *opCtx, o *outputArg, what string) (*wire.TxOut, error) {
	var script []byte
	n := 0
	if o.Pay != nil {
		n++
		addr, err := r.address(c, o.Pay, what)
		if err != nil {
			return nil, err
		}
		_, s := addr.PaymentScript()
		if o.Tag != "" {
			op, ok := stakeTags[o.Tag]
			if !ok {
				return nil, scriptErrorf("%s: unknown tag %q", what, o.Tag)
			}
			s = append([]byte{op}, s...)
		}
		script = s
	} else if o.Tag != "" {
		return nil, scriptErrorf("%s: tag without pay", what)
	}
	if o.Script != nil {
		n++
		s, err := hex.DecodeString(*o.Script)
		if err != nil {
			return nil, scriptErrorf("%s: script is not hex", what)
		}
		script = s
	}
	if o.Commitment != nil {
		n++
		addr, err := r.address(c, o.Commitment.Pay, what)
		if err != nil {
			return nil, err
		}
		var h160 []byte
		p2sh := false
		switch a := addr.(type) {
		case *stdaddr.AddressPubKeyHashEcdsaSecp256k1V0:
			h160 = a.Hash160()[:]
		case *stdaddr.AddressScriptHashV0:
			h160 = a.Hash160()[:]
			p2sh = true
		default:
			return nil, scriptErrorf("%s: commitment address must be P2PKH or P2SH", what)
		}
		payload := append([]byte{}, h160...)
		var amt [8]byte
		binary.LittleEndian.PutUint64(amt[:], uint64(o.Commitment.Amount))
		if p2sh {
			amt[7] |= 0x80
		}
		payload = append(payload, amt[:]...)
		limits := uint16(0x5800)
		if o.Commitment.Limits != nil {
			limits = *o.Commitment.Limits
		}
		payload = binary.LittleEndian.AppendUint16(payload, limits)
		script = append([]byte{0x6a}, pushData(payload)...)
	}
	if o.VoteBlock != nil {
		n++
		h, err := r.blockHash(c, o.VoteBlock, what)
		if err != nil {
			return nil, err
		}
		meta, err := r.blockMeta(c, &h, what)
		if err != nil {
			return nil, err
		}
		payload := append([]byte{}, h[:]...)
		payload = binary.LittleEndian.AppendUint32(payload, uint32(meta.Height))
		script = append([]byte{0x6a}, pushData(payload)...)
	}
	if o.VoteBits != nil {
		n++
		b, err := hex.DecodeString(*o.VoteBits)
		if err != nil || len(b) < 2 || len(b) > 75 {
			return nil, scriptErrorf("%s: votebits must be 2 to 75 bytes of hex", what)
		}
		script = append([]byte{0x6a}, pushData(b)...)
	}
	if o.OpReturn != nil {
		n++
		b, err := hex.DecodeString(*o.OpReturn)
		if err != nil || len(b) > 75 {
			return nil, scriptErrorf("%s: opreturn must be at most 75 bytes of hex", what)
		}
		script = append([]byte{0x6a}, pushData(b)...)
	}
	if n != 1 {
		return nil, scriptErrorf("%s: an output needs exactly one of pay, script, commitment, voteblock, votebits, opreturn", what)
	}
	return wire.NewTxOut(o.Value, script), nil
}

// opTxBuild builds a transaction and records it under a label with
// udb.NewTxRecordFromMsgTx. The received time is "received" or, as a Go
// caller's time.Now(), the injected clock.
func opTxBuild(r *runner, c *opCtx, a *opArgs) (any, error) {
	label, err := a.str("label")
	if err != nil {
		return nil, err
	}
	if _, dup := r.txs[label]; dup {
		return nil, scriptErrorf("tx.build: label %q already used", label)
	}
	tx := wire.NewMsgTx()
	ver, err := a.optU32("version", 1)
	if err != nil {
		return nil, err
	}
	tx.Version = uint16(ver)
	if tx.LockTime, err = a.optU32("locktime", 0); err != nil {
		return nil, err
	}
	if tx.Expiry, err = a.optU32("expiry", 0); err != nil {
		return nil, err
	}
	var ins []inputArg
	var outs []outputArg
	if a.has("inputs") {
		if err := a.decode("inputs", &ins); err != nil {
			return nil, err
		}
	}
	if a.has("outputs") {
		if err := a.decode("outputs", &outs); err != nil {
			return nil, err
		}
	}
	// Every argument is read before outputs resolve their references (a
	// derived address of an account that does not exist is the
	// operation's result, and must not hide a malformed argument).
	received := seam.Unix()
	if received, err = a.optInt64("received", received); err != nil {
		return nil, err
	}
	for i := range ins {
		in := &ins[i]
		what := fmt.Sprintf("tx.build %s input %d", label, i)
		var op wire.OutPoint
		switch string(in.Prev) {
		case `"coinbase"`, `"stakebase"`:
			op = wire.OutPoint{Index: wire.MaxPrevOutIndex}
		default:
			if op, err = r.outpoint(in.Prev, what); err != nil {
				return nil, err
			}
		}
		if in.Tree != nil {
			op.Tree = *in.Tree
		}
		script, err := hex.DecodeString(in.Script)
		if err != nil {
			return nil, scriptErrorf("%s: script is not hex", what)
		}
		txIn := wire.NewTxIn(&op, in.Value, script)
		if in.Sequence != nil {
			txIn.Sequence = *in.Sequence
		}
		tx.AddTxIn(txIn)
	}
	for i := range outs {
		out, err := r.buildOutput(c, &outs[i], fmt.Sprintf("tx.build %s output %d", label, i))
		if err != nil {
			return nil, err
		}
		tx.AddTxOut(out)
	}
	rec, err := udb.NewTxRecordFromMsgTx(tx, time.Unix(received, 0))
	if err != nil {
		return nil, err
	}
	r.txs[label] = rec
	return map[string]any{"hash": rec.Hash.String(), "type": txTypeName(rec.TxType),
		"tx": hexStr(rec.SerializedTx)}, nil
}

// opTxRaw records serialized transaction bytes under a label with
// udb.NewTxRecord.
func opTxRaw(r *runner, c *opCtx, a *opArgs) (any, error) {
	label, err := a.str("label")
	if err != nil {
		return nil, err
	}
	if _, dup := r.txs[label]; dup {
		return nil, scriptErrorf("tx.raw: label %q already used", label)
	}
	b, err := a.hexBytes("tx")
	if err != nil {
		return nil, err
	}
	received, err := a.optInt64("received", seam.Unix())
	if err != nil {
		return nil, err
	}
	rec, err := udb.NewTxRecord(b, time.Unix(received, 0))
	if err != nil {
		return nil, err
	}
	r.txs[label] = rec
	return map[string]any{"hash": rec.Hash.String(), "type": txTypeName(rec.TxType)}, nil
}

// detHash is SHA-256 of the ASCII text "<kind>/<height>/<counter>".
func detHash(kind string, height, counter uint32) chainhash.Hash {
	return chainhash.Hash(sha256.Sum256([]byte(fmt.Sprintf("%s/%d/%d", kind, height, counter))))
}

// buildHeader makes the header ExtendMainChain gets: by default the child
// of the current tip at its height plus one, timestamped chain_time +
// 300 s per height, version 7, bits the network's proof-of-work limit,
// nonce the number of extend calls so far in the run (counting this one),
// and merkle and stake roots detHash("merkle" or "stake", height, nonce),
// so that every extend call makes a distinct block.
func (r *runner) buildHeader(c *opCtx, a *opArgs) (*wire.BlockHeader, *gcs.FilterV2, error) {
	// Every argument is read before the parent is looked up, so that a
	// parent the store does not hold (the operation's result) never hides
	// a malformed argument (a script fault).
	parent, height := r.store.MainChainTip(c.rtx)
	hasParent := false
	if raw, ok := a.raw("parent"); ok {
		h, err := r.blockHash(c, raw, a.op+" parent")
		if err != nil {
			return nil, nil, err
		}
		parent, hasParent = h, true
	}
	explicitHeight := a.has("height")
	var h32 uint32
	if explicitHeight {
		var err error
		if h32, err = a.u32("height"); err != nil {
			return nil, nil, err
		}
	}
	voteBits, err := a.optU32("votebits", 1)
	if err != nil {
		return nil, nil, err
	}
	stakeVer, err := a.optU32("stakeversion", 0)
	if err != nil {
		return nil, nil, err
	}
	hasTime := a.has("time")
	var ts int64
	if hasTime {
		if ts, err = a.int64v("time"); err != nil {
			return nil, nil, err
		}
	}
	var labels []string
	if a.has("filter") {
		if err := a.decode("filter", &labels); err != nil {
			return nil, nil, err
		}
		for _, l := range labels {
			if _, ok := r.txs[l]; !ok {
				return nil, nil, scriptErrorf("%s: unknown transaction label %q", a.op, l)
			}
		}
	}
	r.nonce++
	if hasParent {
		ph, err := r.store.GetBlockHeader(c.rtx, &parent)
		if err != nil {
			return nil, nil, resolveErr(a.op+" parent", "GetBlockHeader", err)
		}
		height = int32(ph.Height)
	}
	if !explicitHeight {
		h32 = uint32(height + 1)
	}
	if !hasTime {
		ts = r.chainTime + int64(h32)*300
	}
	header := &wire.BlockHeader{
		Version:      7,
		PrevBlock:    parent,
		MerkleRoot:   detHash("merkle", h32, r.nonce),
		StakeRoot:    detHash("stake", h32, r.nonce),
		VoteBits:     uint16(voteBits),
		Bits:         r.params.PowLimitBits,
		Height:       h32,
		Timestamp:    time.Unix(ts, 0),
		Nonce:        r.nonce,
		StakeVersion: stakeVer,
	}
	// The filter matches the output scripts of the listed transactions.
	var data [][]byte
	for _, l := range labels {
		for _, out := range r.txs[l].MsgTx.TxOut {
			data = append(data, out.PkScript)
		}
	}
	f, err := gcs.NewFilterV2(blockcf2.B, blockcf2.M, blockcf2.Key(&header.MerkleRoot), data)
	if err != nil {
		return nil, nil, err
	}
	return header, f, nil
}

func headerBytes(h *wire.BlockHeader) []byte {
	var buf bytes.Buffer
	_ = h.Serialize(&buf)
	return buf.Bytes()
}
