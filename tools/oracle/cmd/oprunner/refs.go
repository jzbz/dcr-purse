// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

package main

// Arguments that name things (transactions, blocks, outpoints, addresses,
// extended keys) and the canonical encoding of results.

import (
	"bytes"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"strconv"
	"strings"
	"time"

	"github.com/decred/dcrd/blockchain/stake/v5"
	"github.com/decred/dcrd/chaincfg/chainhash"
	"github.com/decred/dcrd/dcrutil/v4"
	"github.com/decred/dcrd/hdkeychain/v3"
	"github.com/decred/dcrd/txscript/v4/stdaddr"
	"github.com/decred/dcrd/wire"
	"github.com/jzbz/dcr-purse/tools/oracle/cmd/oprunner/internal/udb"
)

// hashRef resolves a hash given as 64 hex digits in display order, or as
// a transaction label, or as a block label.
func (r *runner) hashRef(raw json.RawMessage, what string) (chainhash.Hash, error) {
	var s string
	if err := json.Unmarshal(raw, &s); err != nil {
		return chainhash.Hash{}, scriptErrorf("%s must be a string", what)
	}
	if len(s) == 64 {
		if h, err := chainhash.NewHashFromStr(s); err == nil {
			return *h, nil
		}
	}
	if rec, ok := r.txs[s]; ok {
		return rec.Hash, nil
	}
	if h, ok := r.blocks[s]; ok {
		return h, nil
	}
	return chainhash.Hash{}, scriptErrorf("%s: unknown label %q", what, s)
}

func (r *runner) argHash(a *opArgs, key string) (chainhash.Hash, error) {
	raw, ok := a.raw(key)
	if !ok {
		return chainhash.Hash{}, scriptErrorf("%s: missing argument %q", a.op, key)
	}
	return r.hashRef(raw, a.op+" "+key)
}

// argTx resolves a transaction label to the record tx.build made.
func (r *runner) argTx(a *opArgs, key string) (*udb.TxRecord, error) {
	s, err := a.str(key)
	if err != nil {
		return nil, err
	}
	rec, ok := r.txs[s]
	if !ok {
		return nil, scriptErrorf("%s: unknown transaction label %q", a.op, s)
	}
	return rec, nil
}

// argBlockMeta resolves an optional block reference to the BlockMeta a Go
// caller would build from the block's header (nil when absent or null:
// an unmined record). "tip" names the current main chain tip. An object
// {hash, height, time, votebits} is the BlockMeta itself, as a Go caller
// builds it from a block notification whether or not the store holds the
// header.
func (r *runner) argBlockMeta(c *opCtx, a *opArgs, key string) (*udb.BlockMeta, error) {
	raw, ok := a.raw(key)
	if !ok || string(raw) == "null" {
		return nil, nil
	}
	what := a.op + " " + key
	if t := bytes.TrimSpace(raw); len(t) > 0 && t[0] == '{' {
		var o struct {
			Hash     json.RawMessage `json:"hash"`
			Height   *int32          `json:"height"`
			Time     *int64          `json:"time"`
			VoteBits *uint16         `json:"votebits"`
		}
		dec := json.NewDecoder(bytes.NewReader(raw))
		dec.DisallowUnknownFields()
		if err := dec.Decode(&o); err != nil || o.Hash == nil || o.Height == nil ||
			o.Time == nil || o.VoteBits == nil {
			return nil, scriptErrorf("%s: a block meta is {hash, height, time, votebits}", what)
		}
		h, err := r.hashRef(o.Hash, what)
		if err != nil {
			return nil, err
		}
		return &udb.BlockMeta{
			Block:    udb.Block{Hash: h, Height: *o.Height},
			Time:     time.Unix(*o.Time, 0),
			VoteBits: *o.VoteBits,
		}, nil
	}
	h, err := r.blockHash(c, raw, what)
	if err != nil {
		return nil, err
	}
	return r.blockMeta(c, &h, what)
}

func (r *runner) blockHash(c *opCtx, raw json.RawMessage, what string) (chainhash.Hash, error) {
	if string(raw) == `"tip"` {
		h, _ := r.store.MainChainTip(c.rtx)
		return h, nil
	}
	return r.hashRef(raw, what)
}

func (r *runner) argBlockHash(c *opCtx, a *opArgs, key string) (chainhash.Hash, error) {
	raw, ok := a.raw(key)
	if !ok {
		return chainhash.Hash{}, scriptErrorf("%s: missing argument %q", a.op, key)
	}
	return r.blockHash(c, raw, a.op+" "+key)
}

// resolveErr is a udb error met while resolving a reference: an operation
// result (never a script fault, since the script is well formed and the
// wallet's state decides), with udb's kind kept for errors.As and
// errors.Is and a text that names the resolution step:
// "<what>: <Go entry point>: <udb's text>".
func resolveErr(what, entry string, err error) error {
	return fmt.Errorf("%s: %s: %w", what, entry, err)
}

// blockMeta builds the BlockMeta a Go caller builds from the stored header
// of block h. A header the store does not hold is the operation's error
// (kind NotExist from Store.GetBlockHeader).
func (r *runner) blockMeta(c *opCtx, h *chainhash.Hash, what string) (*udb.BlockMeta, error) {
	header, err := r.store.GetBlockHeader(c.rtx, h)
	if err != nil {
		return nil, resolveErr(what, "GetBlockHeader", err)
	}
	return &udb.BlockMeta{
		Block:    udb.Block{Hash: *h, Height: int32(header.Height)},
		Time:     header.Timestamp,
		VoteBits: header.VoteBits,
	}, nil
}

type outpointArg struct {
	Tx    json.RawMessage `json:"tx"`
	Index uint32          `json:"index"`
	Tree  int8            `json:"tree"`
}

func (r *runner) outpoint(raw json.RawMessage, what string) (wire.OutPoint, error) {
	var o outpointArg
	dec := json.NewDecoder(bytes.NewReader(raw))
	dec.DisallowUnknownFields()
	if err := dec.Decode(&o); err != nil || o.Tx == nil {
		return wire.OutPoint{}, scriptErrorf("%s: an outpoint is {tx, index, tree?}", what)
	}
	h, err := r.hashRef(o.Tx, what)
	if err != nil {
		return wire.OutPoint{}, err
	}
	return wire.OutPoint{Hash: h, Index: o.Index, Tree: o.Tree}, nil
}

func (r *runner) argOutpoint(a *opArgs, key string) (wire.OutPoint, error) {
	raw, ok := a.raw(key)
	if !ok {
		return wire.OutPoint{}, scriptErrorf("%s: missing argument %q", a.op, key)
	}
	return r.outpoint(raw, a.op+" "+key)
}

// address resolves an address reference: an encoded address; {"ref":
// label} for an address an import operation labelled; {"acct", "branch",
// "index"} for the P2PKH address of a derived key, derived through
// Manager.AccountBranchExtendedPubKey in the operation's transaction (its
// error, an account that does not exist for example, and an invalid child
// are the operation's result, not script faults); {"p2sh": script hex}.
func (r *runner) address(c *opCtx, raw json.RawMessage, what string) (stdaddr.Address, error) {
	var s string
	if json.Unmarshal(raw, &s) == nil {
		addr, err := stdaddr.DecodeAddress(s, r.params)
		if err != nil {
			return nil, scriptErrorf("%s: %v", what, err)
		}
		return addr, nil
	}
	var o struct {
		Ref    *string `json:"ref"`
		Acct   *uint32 `json:"acct"`
		Branch *uint32 `json:"branch"`
		Index  *uint32 `json:"index"`
		P2SH   *string `json:"p2sh"`
	}
	dec := json.NewDecoder(bytes.NewReader(raw))
	dec.DisallowUnknownFields()
	if err := dec.Decode(&o); err != nil {
		return nil, scriptErrorf("%s: bad address reference: %v", what, err)
	}
	switch {
	case o.Ref != nil:
		addr, ok := r.addrs[*o.Ref]
		if !ok {
			return nil, scriptErrorf("%s: unknown address label %q", what, *o.Ref)
		}
		return addr, nil
	case o.Acct != nil && o.Branch != nil && o.Index != nil:
		if c == nil || c.rtx == nil {
			return nil, scriptErrorf("%s: a derived address needs a transaction", what)
		}
		xpub, err := r.mgr.AccountBranchExtendedPubKey(c.rtx, *o.Acct, *o.Branch)
		if err != nil {
			return nil, resolveErr(what, "AccountBranchExtendedPubKey", err)
		}
		child, err := xpub.Child(*o.Index)
		if err != nil {
			return nil, resolveErr(what, fmt.Sprintf("Child(%d)", *o.Index), err)
		}
		addr, err := stdaddr.NewAddressPubKeyHashEcdsaSecp256k1V0(
			dcrutil.Hash160(child.SerializedPubKey()), r.params)
		if err != nil {
			return nil, resolveErr(what, "NewAddressPubKeyHashEcdsaSecp256k1V0", err)
		}
		return addr, nil
	case o.P2SH != nil:
		script, err := hex.DecodeString(*o.P2SH)
		if err != nil {
			return nil, scriptErrorf("%s: p2sh is not hex", what)
		}
		addr, err := stdaddr.NewAddressScriptHashV0(script, r.params)
		if err != nil {
			return nil, scriptErrorf("%s: %v", what, err)
		}
		return addr, nil
	}
	return nil, scriptErrorf("%s: bad address reference %s", what, raw)
}

func (r *runner) argAddress(c *opCtx, a *opArgs, key string) (stdaddr.Address, error) {
	raw, ok := a.raw(key)
	if !ok {
		return nil, scriptErrorf("%s: missing argument %q", a.op, key)
	}
	return r.address(c, raw, a.op+" "+key)
}

// keySpec resolves an extended key: an encoded xpub or xpriv string, or
// {"seed": hex, "path": ["44'", "1'", "0'", ...]} (a master key from the
// seed on the script's network, then each child; a trailing ' marks a
// hardened index). With public set the key is neutered.
func (r *runner) keySpec(raw json.RawMessage, private bool) (*hdkeychain.ExtendedKey, error) {
	var s string
	var key *hdkeychain.ExtendedKey
	if json.Unmarshal(raw, &s) == nil {
		k, err := hdkeychain.NewKeyFromString(s, r.params)
		if err != nil {
			return nil, scriptErrorf("extended key: %v", err)
		}
		key = k
	} else {
		var o struct {
			Seed string   `json:"seed"`
			Path []string `json:"path"`
		}
		dec := json.NewDecoder(bytes.NewReader(raw))
		dec.DisallowUnknownFields()
		if err := dec.Decode(&o); err != nil {
			return nil, scriptErrorf("bad key spec: %v", err)
		}
		seed, err := hex.DecodeString(o.Seed)
		if err != nil {
			return nil, scriptErrorf("key spec seed is not hex")
		}
		k, err := hdkeychain.NewMaster(seed, r.params)
		if err != nil {
			return nil, scriptErrorf("key spec: NewMaster: %v", err)
		}
		for _, p := range o.Path {
			hard := strings.HasSuffix(p, "'")
			n, err := strconv.ParseUint(strings.TrimSuffix(p, "'"), 10, 31)
			if err != nil {
				return nil, scriptErrorf("key spec path element %q", p)
			}
			idx := uint32(n)
			if hard {
				idx += hdkeychain.HardenedKeyStart
			}
			if k, err = k.Child(idx); err != nil {
				return nil, scriptErrorf("key spec child %q: %v", p, err)
			}
		}
		key = k
	}
	if !private {
		key = key.Neuter()
	}
	return key, nil
}

// Result encoding. Keys ending in "~c" hold values derived from the
// injected clock, "~r" from the injected randomness, "~cr" from both;
// everything else must not depend on either (ADR-0002).

func hashStr(h *chainhash.Hash) string { return h.String() }
func hexStr(b []byte) string           { return hex.EncodeToString(b) }

func outpointValue(op *wire.OutPoint) map[string]any {
	return map[string]any{"hash": op.Hash.String(), "index": op.Index, "tree": op.Tree}
}

func blockValue(b *udb.Block) map[string]any {
	return map[string]any{"hash": b.Hash.String(), "height": b.Height}
}

func blockMetaValue(b *udb.BlockMeta) map[string]any {
	return map[string]any{
		"hash": b.Hash.String(), "height": b.Height,
		"time": b.Time.Unix(), "votebits": b.VoteBits,
	}
}

var txTypeNames = map[stake.TxType]string{
	stake.TxTypeRegular:      "regular",
	stake.TxTypeSStx:         "ticket",
	stake.TxTypeSSGen:        "vote",
	stake.TxTypeSSRtx:        "revocation",
	stake.TxTypeTAdd:         "tadd",
	stake.TxTypeTSpend:       "tspend",
	stake.TxTypeTreasuryBase: "treasurybase",
}

func txTypeName(t stake.TxType) string {
	if n, ok := txTypeNames[t]; ok {
		return n
	}
	return "type" + strconv.Itoa(int(t))
}

func serializeTx(tx *wire.MsgTx) string {
	var buf bytes.Buffer
	buf.Grow(tx.SerializeSize())
	if err := tx.Serialize(&buf); err != nil {
		return "!" + err.Error()
	}
	return hexStr(buf.Bytes())
}

func txRecordValue(rec *udb.TxRecord) map[string]any {
	raw := hexStr(rec.SerializedTx)
	if rec.SerializedTx == nil {
		raw = serializeTx(&rec.MsgTx)
	}
	return map[string]any{
		"hash":        rec.Hash.String(),
		"tx":          raw,
		"received~c":  rec.Received.Unix(),
		"type":        txTypeName(rec.TxType),
		"unpublished": rec.Unpublished,
	}
}

func txDetailsValue(d *udb.TxDetails) map[string]any {
	if d == nil {
		return nil
	}
	v := txRecordValue(&d.TxRecord)
	v["block"] = blockMetaValue(&d.Block)
	v["height"] = d.Height()
	credits := make([]any, 0, len(d.Credits))
	for _, c := range d.Credits {
		credits = append(credits, map[string]any{
			"index": c.Index, "amount": int64(c.Amount), "spent": c.Spent,
			"change": c.Change, "opcode": c.OpCode, "coinbase": c.IsCoinbase,
			"expiry": c.HasExpiry,
		})
	}
	debits := make([]any, 0, len(d.Debits))
	for _, db := range d.Debits {
		debits = append(debits, map[string]any{"index": db.Index, "amount": int64(db.Amount)})
	}
	v["credits"] = credits
	v["debits"] = debits
	return v
}

func creditValue(c *udb.Credit) map[string]any {
	if c == nil {
		return nil
	}
	return map[string]any{
		"outpoint":   outpointValue(&c.OutPoint),
		"block":      blockMetaValue(&c.BlockMeta),
		"amount":     int64(c.Amount),
		"pkscript":   hexStr(c.PkScript),
		"received~c": c.Received.Unix(),
		"coinbase":   c.FromCoinBase,
		"expiry":     c.HasExpiry,
	}
}

func balancesValue(b *udb.Balances) map[string]any {
	return map[string]any{
		"account":                 b.Account,
		"immaturecoinbaserewards": int64(b.ImmatureCoinbaseRewards),
		"immaturestakegeneration": int64(b.ImmatureStakeGeneration),
		"lockedbytickets":         int64(b.LockedByTickets),
		"spendable":               int64(b.Spendable),
		"total":                   int64(b.Total),
		"votingauthority":         int64(b.VotingAuthority),
		"unconfirmed":             int64(b.Unconfirmed),
	}
}

func managedAddressValue(m udb.ManagedAddress) map[string]any {
	v := map[string]any{
		"addr":     m.Address().String(),
		"account":  m.Account(),
		"addrhash": hexStr(m.AddrHash()),
		"imported": m.Imported(),
		"internal": m.Internal(),
		"multisig": m.Multisig(),
	}
	switch m := m.(type) {
	case udb.ManagedPubKeyAddress:
		v["kind"] = "pubkey"
		v["pubkey"] = hexStr(m.PubKey())
		v["index"] = m.Index()
	case udb.ManagedScriptAddress:
		v["kind"] = "script"
		ver, script := m.RedeemScript()
		v["scriptversion"] = ver
		v["script"] = hexStr(script)
	default:
		v["kind"] = "other"
	}
	return v
}

func hashList(hs []chainhash.Hash) []any {
	out := make([]any, 0, len(hs))
	for i := range hs {
		out = append(out, hs[i].String())
	}
	return out
}

func hashPtrList(hs []*chainhash.Hash) []any {
	out := make([]any, 0, len(hs))
	for _, h := range hs {
		out = append(out, h.String())
	}
	return out
}
