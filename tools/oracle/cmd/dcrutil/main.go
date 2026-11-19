// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

// Command dcrutil is the oracle for crates/purse-dcrutil: it answers with
// github.com/decred/dcrd/dcrutil/v4 and its txsort package at the version
// dcrwallet links (v4.0.3).
//
// A float64 travels as its IEEE-754 bits, 16 lower-case hex digits, so that
// NaN payloads, signed zeros, subnormals and every boundary value arrive and
// return exactly. Byte strings are hex. Every command but consts and arch
// takes a batch and answers with one element per input, in order.
//
// Commands:
//
//	arch          {"goarch": runtime.GOARCH, "goamd64": the GOAMD64 build
//	              setting, "" off amd64}. NewAmount and MulF64 differ by
//	              architecture (see crates/purse-dcrutil/src/amount.rs).
//	consts        the package constants, the unit names of the named units,
//	              the package's error texts and chaincfg's PrivateKeyID of
//	              the four networks.
//	amount.new    {"bits": [hex]} -> [{"atoms": int64} | {"error": text}]
//	amount.mulf64 {"cases": [[atoms, hex]]} -> [int64]
//	amount.coin   {"atoms": [int64]} -> [{"to_coin": hex, "string": text}]
//	amount.format {"cases": [[atoms, unit]]}
//	              -> [{"to_unit": hex, "format": text}]
//	amount.unit   {"units": [int64]} -> [AmountUnit(u).String()]
//	wif.new       {"cases": [{"priv": hex, "net": hex, "scheme": int64}]}
//	              -> [wif | {"error": text}]
//	wif.decode    {"cases": [{"wif": text, "net": hex}]}
//	              -> [wif | {"error": text}]
//	              where wif is {"string": String(), "priv": hex, "pub": hex,
//	              "dsa": DSA(), "net": hex of the first two decoded bytes}
//	txsort.hex    {"txs": [hex]} -> [{"is_sorted": IsSorted, "sorted": Sort
//	              serialized, "in_place": InPlaceSort serialized,
//	              "unmodified": whether Sort left its input unchanged,
//	              "hash": TxHash before, "sorted_hash": TxHash of Sort}]
//	              or {"error": text} for a transaction that does not
//	              deserialize.
//	txsort.fields {"txs": [tx]} -> [{"is_sorted": bool, "inputs": [in],
//	              "outputs": [out], "sort_agrees": bool}]
//	              where tx is {"inputs": [in], "outputs": [out]}, in is
//	              {"hash": hex of the 32 bytes in memory order, "index": u32,
//	              "tree": int8, "sequence": u32, "value_in": int64,
//	              "block_height": u32, "block_index": u32, "script": hex},
//	              out is {"value": int64, "version": u16, "script": hex}.
//	              The answer is InPlaceSort's order; sort_agrees says
//	              whether Sort's copy came out field for field the same.
//	              Trees travel as int8, so negative trees, which do not
//	              survive serialization, are covered too.
package main

import (
	"bytes"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"math"
	"runtime"
	"runtime/debug"
	"strconv"

	"github.com/decred/dcrd/chaincfg/chainhash"
	"github.com/decred/dcrd/chaincfg/v3"
	"github.com/decred/dcrd/dcrec"
	"github.com/decred/dcrd/dcrutil/v4"
	"github.com/decred/dcrd/dcrutil/v4/txsort"
	"github.com/decred/dcrd/wire"
	"github.com/jzbz/dcr-purse/tools/oracle/internal/linejson"
)

func bitsToFloat(s string) (float64, error) {
	u, err := strconv.ParseUint(s, 16, 64)
	if err != nil {
		return 0, err
	}
	return math.Float64frombits(u), nil
}

func floatToBits(f float64) string {
	return fmt.Sprintf("%016x", math.Float64bits(f))
}

func decodeNet(s string) ([2]byte, error) {
	var net [2]byte
	b, err := hex.DecodeString(s)
	if err != nil {
		return net, err
	}
	if len(b) != 2 {
		return net, fmt.Errorf("net must be 2 bytes, got %d", len(b))
	}
	copy(net[:], b)
	return net, nil
}

func wifResult(w *dcrutil.WIF, net [2]byte) map[string]any {
	return map[string]any{
		"string": w.String(),
		"priv":   hex.EncodeToString(w.PrivKey()),
		"pub":    hex.EncodeToString(w.PubKey()),
		"dsa":    int64(w.DSA()),
		"net":    hex.EncodeToString(net[:]),
	}
}

type txIn struct {
	Hash        string `json:"hash"`
	Index       uint32 `json:"index"`
	Tree        int8   `json:"tree"`
	Sequence    uint32 `json:"sequence"`
	ValueIn     int64  `json:"value_in"`
	BlockHeight uint32 `json:"block_height"`
	BlockIndex  uint32 `json:"block_index"`
	Script      string `json:"script"`
}

type txOut struct {
	Value   int64  `json:"value"`
	Version uint16 `json:"version"`
	Script  string `json:"script"`
}

type txFields struct {
	Inputs  []txIn  `json:"inputs"`
	Outputs []txOut `json:"outputs"`
}

func buildTx(f *txFields) (*wire.MsgTx, error) {
	tx := wire.NewMsgTx()
	for _, in := range f.Inputs {
		h, err := hex.DecodeString(in.Hash)
		if err != nil {
			return nil, err
		}
		if len(h) != chainhash.HashSize {
			return nil, fmt.Errorf("hash must be 32 bytes, got %d", len(h))
		}
		script, err := hex.DecodeString(in.Script)
		if err != nil {
			return nil, err
		}
		ti := &wire.TxIn{
			Sequence:        in.Sequence,
			ValueIn:         in.ValueIn,
			BlockHeight:     in.BlockHeight,
			BlockIndex:      in.BlockIndex,
			SignatureScript: script,
		}
		copy(ti.PreviousOutPoint.Hash[:], h)
		ti.PreviousOutPoint.Index = in.Index
		ti.PreviousOutPoint.Tree = in.Tree
		tx.TxIn = append(tx.TxIn, ti)
	}
	for _, out := range f.Outputs {
		script, err := hex.DecodeString(out.Script)
		if err != nil {
			return nil, err
		}
		tx.TxOut = append(tx.TxOut, &wire.TxOut{
			Value:    out.Value,
			Version:  out.Version,
			PkScript: script,
		})
	}
	return tx, nil
}

func txToFields(tx *wire.MsgTx) txFields {
	f := txFields{Inputs: []txIn{}, Outputs: []txOut{}}
	for _, in := range tx.TxIn {
		f.Inputs = append(f.Inputs, txIn{
			Hash:        hex.EncodeToString(in.PreviousOutPoint.Hash[:]),
			Index:       in.PreviousOutPoint.Index,
			Tree:        in.PreviousOutPoint.Tree,
			Sequence:    in.Sequence,
			ValueIn:     in.ValueIn,
			BlockHeight: in.BlockHeight,
			BlockIndex:  in.BlockIndex,
			Script:      hex.EncodeToString(in.SignatureScript),
		})
	}
	for _, out := range tx.TxOut {
		f.Outputs = append(f.Outputs, txOut{
			Value:   out.Value,
			Version: out.Version,
			Script:  hex.EncodeToString(out.PkScript),
		})
	}
	return f
}

func serialize(tx *wire.MsgTx) (string, error) {
	b, err := tx.Bytes()
	if err != nil {
		return "", err
	}
	return hex.EncodeToString(b), nil
}

func main() {
	linejson.Serve(map[string]linejson.Handler{
		"arch": func(json.RawMessage) (any, error) {
			goamd64 := ""
			if info, ok := debug.ReadBuildInfo(); ok {
				for _, s := range info.Settings {
					if s.Key == "GOAMD64" {
						goamd64 = s.Value
					}
				}
			}
			return map[string]any{"goarch": runtime.GOARCH, "goamd64": goamd64}, nil
		},
		"consts": func(json.RawMessage) (any, error) {
			units := []dcrutil.AmountUnit{
				dcrutil.AmountMegaCoin, dcrutil.AmountKiloCoin, dcrutil.AmountCoin,
				dcrutil.AmountMilliCoin, dcrutil.AmountMicroCoin, dcrutil.AmountAtom,
			}
			unitValues := make([]int64, 0, len(units))
			unitNames := make([]string, 0, len(units))
			for _, u := range units {
				unitValues = append(unitValues, int64(u))
				unitNames = append(unitNames, u.String())
			}
			_, amountErr := dcrutil.NewAmount(math.NaN())
			nets := map[string]string{}
			for _, p := range []*chaincfg.Params{
				chaincfg.MainNetParams(), chaincfg.TestNet3Params(),
				chaincfg.SimNetParams(), chaincfg.RegNetParams(),
			} {
				nets[p.Name] = hex.EncodeToString(p.PrivateKeyID[:])
			}
			return map[string]any{
				"atoms_per_cent":       int64(dcrutil.AtomsPerCent),
				"atoms_per_coin":       int64(dcrutil.AtomsPerCoin),
				"max_amount":           int64(dcrutil.MaxAmount),
				"unit_values":          unitValues,
				"unit_names":           unitNames,
				"signature_types":      []int64{int64(dcrec.STEcdsaSecp256k1), int64(dcrec.STEd25519), int64(dcrec.STSchnorrSecp256k1)},
				"err_amount":           amountErr.Error(),
				"err_malformed":        dcrutil.ErrMalformedPrivateKey.Error(),
				"err_checksum":         dcrutil.ErrChecksumMismatch.Error(),
				"private_key_ids":      nets,
				"err_wrong_net_sample": dcrutil.ErrWrongWIFNetwork([2]byte{0x22, 0xde}).Error(),
			}, nil
		},
		"amount.new": func(raw json.RawMessage) (any, error) {
			var args struct {
				Bits []string `json:"bits"`
			}
			if err := json.Unmarshal(raw, &args); err != nil {
				return nil, err
			}
			out := make([]any, 0, len(args.Bits))
			for _, s := range args.Bits {
				f, err := bitsToFloat(s)
				if err != nil {
					return nil, err
				}
				a, err := dcrutil.NewAmount(f)
				if err != nil {
					out = append(out, map[string]any{"error": err.Error()})
				} else {
					out = append(out, map[string]any{"atoms": int64(a)})
				}
			}
			return out, nil
		},
		"amount.mulf64": func(raw json.RawMessage) (any, error) {
			var args struct {
				Cases [][2]json.RawMessage `json:"cases"`
			}
			if err := json.Unmarshal(raw, &args); err != nil {
				return nil, err
			}
			out := make([]int64, 0, len(args.Cases))
			for _, c := range args.Cases {
				var atoms int64
				var bits string
				if err := json.Unmarshal(c[0], &atoms); err != nil {
					return nil, err
				}
				if err := json.Unmarshal(c[1], &bits); err != nil {
					return nil, err
				}
				f, err := bitsToFloat(bits)
				if err != nil {
					return nil, err
				}
				out = append(out, int64(dcrutil.Amount(atoms).MulF64(f)))
			}
			return out, nil
		},
		"amount.coin": func(raw json.RawMessage) (any, error) {
			var args struct {
				Atoms []int64 `json:"atoms"`
			}
			if err := json.Unmarshal(raw, &args); err != nil {
				return nil, err
			}
			out := make([]any, 0, len(args.Atoms))
			for _, a := range args.Atoms {
				amt := dcrutil.Amount(a)
				out = append(out, map[string]any{
					"to_coin": floatToBits(amt.ToCoin()),
					"string":  amt.String(),
				})
			}
			return out, nil
		},
		"amount.format": func(raw json.RawMessage) (any, error) {
			var args struct {
				Cases [][2]int64 `json:"cases"`
			}
			if err := json.Unmarshal(raw, &args); err != nil {
				return nil, err
			}
			out := make([]any, 0, len(args.Cases))
			for _, c := range args.Cases {
				amt := dcrutil.Amount(c[0])
				u := dcrutil.AmountUnit(c[1])
				out = append(out, map[string]any{
					"to_unit": floatToBits(amt.ToUnit(u)),
					"format":  amt.Format(u),
				})
			}
			return out, nil
		},
		"amount.unit": func(raw json.RawMessage) (any, error) {
			var args struct {
				Units []int64 `json:"units"`
			}
			if err := json.Unmarshal(raw, &args); err != nil {
				return nil, err
			}
			out := make([]string, 0, len(args.Units))
			for _, u := range args.Units {
				out = append(out, dcrutil.AmountUnit(u).String())
			}
			return out, nil
		},
		"wif.new": func(raw json.RawMessage) (any, error) {
			var args struct {
				Cases []struct {
					Priv   string `json:"priv"`
					Net    string `json:"net"`
					Scheme int64  `json:"scheme"`
				} `json:"cases"`
			}
			if err := json.Unmarshal(raw, &args); err != nil {
				return nil, err
			}
			out := make([]any, 0, len(args.Cases))
			for _, c := range args.Cases {
				priv, err := hex.DecodeString(c.Priv)
				if err != nil {
					return nil, err
				}
				net, err := decodeNet(c.Net)
				if err != nil {
					return nil, err
				}
				w, err := dcrutil.NewWIF(priv, net, dcrec.SignatureType(c.Scheme))
				if err != nil {
					out = append(out, map[string]any{"error": err.Error()})
					continue
				}
				out = append(out, wifResult(w, net))
			}
			return out, nil
		},
		"wif.decode": func(raw json.RawMessage) (any, error) {
			var args struct {
				Cases []struct {
					WIF string `json:"wif"`
					Net string `json:"net"`
				} `json:"cases"`
			}
			if err := json.Unmarshal(raw, &args); err != nil {
				return nil, err
			}
			out := make([]any, 0, len(args.Cases))
			for _, c := range args.Cases {
				net, err := decodeNet(c.Net)
				if err != nil {
					return nil, err
				}
				w, err := dcrutil.DecodeWIF(c.WIF, net)
				if err != nil {
					out = append(out, map[string]any{"error": err.Error()})
					continue
				}
				out = append(out, wifResult(w, net))
			}
			return out, nil
		},
		"txsort.hex": func(raw json.RawMessage) (any, error) {
			var args struct {
				Txs []string `json:"txs"`
			}
			if err := json.Unmarshal(raw, &args); err != nil {
				return nil, err
			}
			out := make([]any, 0, len(args.Txs))
			for _, s := range args.Txs {
				b, err := hex.DecodeString(s)
				if err != nil {
					return nil, err
				}
				var tx wire.MsgTx
				if err := tx.Deserialize(bytes.NewReader(b)); err != nil {
					out = append(out, map[string]any{"error": err.Error()})
					continue
				}
				isSorted := txsort.IsSorted(&tx)
				before := tx.TxHash()
				sorted := txsort.Sort(&tx)
				sortedHex, err := serialize(sorted)
				if err != nil {
					return nil, err
				}
				origHex, err := serialize(&tx)
				if err != nil {
					return nil, err
				}
				txsort.InPlaceSort(&tx)
				inPlaceHex, err := serialize(&tx)
				if err != nil {
					return nil, err
				}
				out = append(out, map[string]any{
					"is_sorted":   isSorted,
					"sorted":      sortedHex,
					"in_place":    inPlaceHex,
					"unmodified":  origHex == hex.EncodeToString(b),
					"hash":        before.String(),
					"sorted_hash": sorted.TxHash().String(),
				})
			}
			return out, nil
		},
		"txsort.fields": func(raw json.RawMessage) (any, error) {
			var args struct {
				Txs []txFields `json:"txs"`
			}
			if err := json.Unmarshal(raw, &args); err != nil {
				return nil, err
			}
			out := make([]any, 0, len(args.Txs))
			for i := range args.Txs {
				tx, err := buildTx(&args.Txs[i])
				if err != nil {
					return nil, err
				}
				isSorted := txsort.IsSorted(tx)
				sorted := txToFields(txsort.Sort(tx))
				txsort.InPlaceSort(tx)
				inPlace := txToFields(tx)
				a, err := json.Marshal(sorted)
				if err != nil {
					return nil, err
				}
				b, err := json.Marshal(inPlace)
				if err != nil {
					return nil, err
				}
				out = append(out, map[string]any{
					"is_sorted":   isSorted,
					"inputs":      inPlace.Inputs,
					"outputs":     inPlace.Outputs,
					"sort_agrees": bytes.Equal(a, b),
				})
			}
			return out, nil
		},
	})
}
