// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

// Command txrules is the oracle for crates/purse-txrules: it answers with
// dcrwallet's wallet/txsizes and wallet/txrules packages at the parity pin.
//
// A float64 travels as its IEEE-754 bits, 16 lower-case hex digits; byte
// strings are hex. Every command but consts, arch and bench takes a batch
// and answers with one element per input, in order. An element whose call
// panics in Go answers {"panic": the recovered value's text} instead, so a
// Go panic is pinned as one rather than ending the process.
//
// An output is {"value": int64, "version": u16, "script": hex}. A
// transaction is {"ser": SerType (0 full, 1 no witness, 2 only witness),
// "ins": [[value_in, signature script length]], "outs": [[value, pkScript
// length]]}: the sizes and sums these packages compute depend on nothing
// else, so the scripts are zero bytes of the given lengths.
//
// Commands:
//
//	arch            {"goarch": runtime.GOARCH, "goamd64": the GOAMD64 build
//	                setting, "" off amd64}. int64(math.Floor(x)) of a NaN or
//	                out-of-range x differs by architecture.
//	consts          the txsizes constants, DefaultRelayFeePerKb, MaxAmount.
//	size.estimate   {"cases": [[[input script sizes], [output script
//	                lengths], change script size]]} -> [EstimateSerializeSize]
//	size.scripts    {"cases": [[[input script sizes], [output script
//	                sizes], change script size]]}
//	                -> [EstimateSerializeSizeFromScriptSizes]
//	size.input      {"sizes": [int]} -> [EstimateInputSize]
//	size.output     {"sizes": [int]} -> [EstimateOutputSize]
//	fee             {"cases": [[relayFeePerKb, size]]} -> [FeeForSerializeSize]
//	dust.amount     {"cases": [[amount, script size, relayFeePerKb]]}
//	                -> [IsDustAmount]
//	dust.output     {"cases": [[output, relayFeePerKb]]} -> [IsDustOutput]
//	check.output    {"cases": [[output, relayFeePerKb]]}
//	                -> [null | error], an error being {"error": Error(),
//	                "kind": the kind's String(), "op": the op}
//	highfees        {"cases": [[totalInput, tx]]} -> [PaysHighFees]
//	highfees.tx     {"txs": [tx]} -> [{"high": bool} | error]
//	stake.subtype   {"types": [int]} -> [[type, converted]], each type as
//	                {"n": int, "name": String()}
//	poolfee.valid   {"bits": [hex]} -> [ValidPoolFeeRate]
//	poolfee.ticket  {"net": "mainnet"|"testnet3"|"simnet"|"regnet", "cases":
//	                [[stakeDiff, relayFee, height, poolFee bits, dcp0010,
//	                dcp0012]]} -> {"bound": the network the package's
//	                subsidy cache was built for, "fees": [StakePoolTicketFee]}.
//	                The package keeps one subsidy cache per process, built
//	                from the first call's parameters, so a process answers
//	                for the network it first served; "bound" says which.
//	poolfee.int     {"bits": [hex]} -> [int64(math.Floor(f * 100.0))], the
//	                percentage conversion StakePoolTicketFee starts with.
//	bench           {"millis": per-entry time budget} -> [{"name", "ns"}],
//	                testing.Benchmark's ns/op for each main entry point
//	                with the fixed inputs crates/purse-txrules/tests/bench.rs
//	                uses.
package main

import (
	"encoding/json"
	"fmt"
	"math"
	"runtime"
	"runtime/debug"
	"strconv"
	"testing"
	"time"

	"decred.org/dcrwallet/v5/errors"
	"decred.org/dcrwallet/v5/wallet/txrules"
	"decred.org/dcrwallet/v5/wallet/txsizes"
	"github.com/decred/dcrd/chaincfg/v3"
	"github.com/decred/dcrd/dcrutil/v4"
	"github.com/decred/dcrd/txscript/v4/stdscript"
	"github.com/decred/dcrd/wire"
	"github.com/jzbz/dcr-purse/tools/oracle/internal/linejson"
)

type output struct {
	Value   int64  `json:"value"`
	Version uint16 `json:"version"`
	Script  string `json:"script"`
}

func (o *output) txOut() (*wire.TxOut, error) {
	script, err := decodeHex(o.Script)
	if err != nil {
		return nil, err
	}
	return &wire.TxOut{Value: o.Value, Version: o.Version, PkScript: script}, nil
}

func decodeHex(s string) ([]byte, error) {
	b := make([]byte, len(s)/2)
	if len(s)%2 != 0 {
		return nil, fmt.Errorf("odd-length hex %q", s)
	}
	for i := range b {
		v, err := strconv.ParseUint(s[2*i:2*i+2], 16, 8)
		if err != nil {
			return nil, err
		}
		b[i] = byte(v)
	}
	return b, nil
}

type txShape struct {
	Ser  uint16     `json:"ser"`
	Ins  [][2]int64 `json:"ins"`
	Outs [][2]int64 `json:"outs"`
}

// maxScript bounds a script length the oracle allocates.
const maxScript = 1 << 20

func (t *txShape) msgTx() (*wire.MsgTx, error) {
	tx := wire.NewMsgTx()
	tx.SerType = wire.TxSerializeType(t.Ser)
	for _, in := range t.Ins {
		if in[1] < 0 || in[1] > maxScript {
			return nil, fmt.Errorf("signature script length %d", in[1])
		}
		tx.TxIn = append(tx.TxIn, &wire.TxIn{
			ValueIn:         in[0],
			SignatureScript: make([]byte, in[1]),
		})
	}
	for _, out := range t.Outs {
		if out[1] < 0 || out[1] > maxScript {
			return nil, fmt.Errorf("pkScript length %d", out[1])
		}
		tx.TxOut = append(tx.TxOut, &wire.TxOut{Value: out[0], PkScript: make([]byte, out[1])})
	}
	return tx, nil
}

func zeroOutputs(lengths []int) ([]*wire.TxOut, error) {
	outs := make([]*wire.TxOut, 0, len(lengths))
	for _, l := range lengths {
		if l < 0 || l > maxScript {
			return nil, fmt.Errorf("pkScript length %d", l)
		}
		outs = append(outs, &wire.TxOut{PkScript: make([]byte, l)})
	}
	return outs, nil
}

func bitsToFloat(s string) (float64, error) {
	u, err := strconv.ParseUint(s, 16, 64)
	if err != nil {
		return 0, err
	}
	return math.Float64frombits(u), nil
}

// guarded runs f, answering {"panic": text} if it panics.
func guarded(f func() any) (result any) {
	defer func() {
		if r := recover(); r != nil {
			result = map[string]any{"panic": fmt.Sprint(r)}
		}
	}()
	return f()
}

func errorValue(err error) any {
	if err == nil {
		return nil
	}
	v := map[string]any{"error": err.Error(), "kind": "", "op": ""}
	var e *errors.Error
	if errors.As(err, &e) {
		v["kind"] = e.Kind.String()
		v["op"] = string(e.Op)
	}
	return v
}

func params(net string) (*chaincfg.Params, error) {
	switch net {
	case "mainnet":
		return chaincfg.MainNetParams(), nil
	case "testnet3":
		return chaincfg.TestNet3Params(), nil
	case "simnet":
		return chaincfg.SimNetParams(), nil
	case "regnet":
		return chaincfg.RegNetParams(), nil
	}
	return nil, fmt.Errorf("unknown network %q", net)
}

// boundNet is the network of the first poolfee.ticket call, whose
// parameters txrules' process-wide subsidy cache was built from.
var boundNet string

func batch[T any](raw json.RawMessage, key string) ([]T, error) {
	var args map[string]json.RawMessage
	if err := json.Unmarshal(raw, &args); err != nil {
		return nil, err
	}
	var items []T
	if err := json.Unmarshal(args[key], &items); err != nil {
		return nil, fmt.Errorf("%s: %w", key, err)
	}
	return items, nil
}

func p2pkhScript() []byte {
	s := make([]byte, 25)
	s[0], s[1], s[2], s[23], s[24] = 0x76, 0xa9, 0x14, 0x88, 0xac
	for i := 3; i < 23; i++ {
		s[i] = byte(i)
	}
	return s
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
			return map[string]any{
				"redeem_p2pk_sig_script_size":   txsizes.RedeemP2PKSigScriptSize,
				"redeem_p2pkh_sig_script_size":  txsizes.RedeemP2PKHSigScriptSize,
				"redeem_p2sh_sig_script_size":   txsizes.RedeemP2SHSigScriptSize,
				"redeem_p2pkh_input_size":       txsizes.RedeemP2PKHInputSize,
				"p2pkh_pk_script_size":          txsizes.P2PKHPkScriptSize,
				"p2pkh_pk_treasury_script_size": txsizes.P2PKHPkTreasuryScriptSize,
				"p2sh_pk_script_size":           txsizes.P2SHPkScriptSize,
				"ticket_commitment_script_size": txsizes.TicketCommitmentScriptSize,
				"p2pkh_output_size":             txsizes.P2PKHOutputSize,
				"tspend_input_size":             txsizes.TSPENDInputSize,
				"default_relay_fee_per_kb":      int64(txrules.DefaultRelayFeePerKb),
				"max_amount":                    int64(dcrutil.MaxAmount),
			}, nil
		},
		"size.estimate": func(raw json.RawMessage) (any, error) {
			cases, err := batch[[3]json.RawMessage](raw, "cases")
			if err != nil {
				return nil, err
			}
			out := make([]any, 0, len(cases))
			for _, c := range cases {
				var ins, lens []int
				var change int
				if err := json.Unmarshal(c[0], &ins); err != nil {
					return nil, err
				}
				if err := json.Unmarshal(c[1], &lens); err != nil {
					return nil, err
				}
				if err := json.Unmarshal(c[2], &change); err != nil {
					return nil, err
				}
				outs, err := zeroOutputs(lens)
				if err != nil {
					return nil, err
				}
				out = append(out, guarded(func() any {
					return txsizes.EstimateSerializeSize(ins, outs, change)
				}))
			}
			return out, nil
		},
		"size.scripts": func(raw json.RawMessage) (any, error) {
			cases, err := batch[[3]json.RawMessage](raw, "cases")
			if err != nil {
				return nil, err
			}
			out := make([]any, 0, len(cases))
			for _, c := range cases {
				var ins, outs []int
				var change int
				if err := json.Unmarshal(c[0], &ins); err != nil {
					return nil, err
				}
				if err := json.Unmarshal(c[1], &outs); err != nil {
					return nil, err
				}
				if err := json.Unmarshal(c[2], &change); err != nil {
					return nil, err
				}
				out = append(out, guarded(func() any {
					return txsizes.EstimateSerializeSizeFromScriptSizes(ins, outs, change)
				}))
			}
			return out, nil
		},
		"size.input": func(raw json.RawMessage) (any, error) {
			sizes, err := batch[int](raw, "sizes")
			if err != nil {
				return nil, err
			}
			out := make([]int, 0, len(sizes))
			for _, s := range sizes {
				out = append(out, txsizes.EstimateInputSize(s))
			}
			return out, nil
		},
		"size.output": func(raw json.RawMessage) (any, error) {
			sizes, err := batch[int](raw, "sizes")
			if err != nil {
				return nil, err
			}
			out := make([]int, 0, len(sizes))
			for _, s := range sizes {
				out = append(out, txsizes.EstimateOutputSize(s))
			}
			return out, nil
		},
		"fee": func(raw json.RawMessage) (any, error) {
			cases, err := batch[[2]int64](raw, "cases")
			if err != nil {
				return nil, err
			}
			out := make([]int64, 0, len(cases))
			for _, c := range cases {
				out = append(out, int64(txrules.FeeForSerializeSize(dcrutil.Amount(c[0]), int(c[1]))))
			}
			return out, nil
		},
		"dust.amount": func(raw json.RawMessage) (any, error) {
			cases, err := batch[[3]int64](raw, "cases")
			if err != nil {
				return nil, err
			}
			out := make([]any, 0, len(cases))
			for _, c := range cases {
				out = append(out, guarded(func() any {
					return txrules.IsDustAmount(dcrutil.Amount(c[0]), int(c[1]), dcrutil.Amount(c[2]))
				}))
			}
			return out, nil
		},
		"dust.output": func(raw json.RawMessage) (any, error) {
			cases, err := batch[[2]json.RawMessage](raw, "cases")
			if err != nil {
				return nil, err
			}
			out := make([]any, 0, len(cases))
			for _, c := range cases {
				var o output
				var rate int64
				if err := json.Unmarshal(c[0], &o); err != nil {
					return nil, err
				}
				if err := json.Unmarshal(c[1], &rate); err != nil {
					return nil, err
				}
				txOut, err := o.txOut()
				if err != nil {
					return nil, err
				}
				out = append(out, guarded(func() any {
					return txrules.IsDustOutput(txOut, dcrutil.Amount(rate))
				}))
			}
			return out, nil
		},
		"check.output": func(raw json.RawMessage) (any, error) {
			cases, err := batch[[2]json.RawMessage](raw, "cases")
			if err != nil {
				return nil, err
			}
			out := make([]any, 0, len(cases))
			for _, c := range cases {
				var o output
				var rate int64
				if err := json.Unmarshal(c[0], &o); err != nil {
					return nil, err
				}
				if err := json.Unmarshal(c[1], &rate); err != nil {
					return nil, err
				}
				txOut, err := o.txOut()
				if err != nil {
					return nil, err
				}
				out = append(out, guarded(func() any {
					return errorValue(txrules.CheckOutput(txOut, dcrutil.Amount(rate)))
				}))
			}
			return out, nil
		},
		"highfees": func(raw json.RawMessage) (any, error) {
			cases, err := batch[[2]json.RawMessage](raw, "cases")
			if err != nil {
				return nil, err
			}
			out := make([]any, 0, len(cases))
			for _, c := range cases {
				var total int64
				var shape txShape
				if err := json.Unmarshal(c[0], &total); err != nil {
					return nil, err
				}
				if err := json.Unmarshal(c[1], &shape); err != nil {
					return nil, err
				}
				tx, err := shape.msgTx()
				if err != nil {
					return nil, err
				}
				out = append(out, guarded(func() any {
					return txrules.PaysHighFees(dcrutil.Amount(total), tx)
				}))
			}
			return out, nil
		},
		"highfees.tx": func(raw json.RawMessage) (any, error) {
			shapes, err := batch[txShape](raw, "txs")
			if err != nil {
				return nil, err
			}
			out := make([]any, 0, len(shapes))
			for i := range shapes {
				tx, err := shapes[i].msgTx()
				if err != nil {
					return nil, err
				}
				out = append(out, guarded(func() any {
					high, err := txrules.TxPaysHighFees(tx)
					if err != nil {
						return errorValue(err)
					}
					return map[string]any{"high": high}
				}))
			}
			return out, nil
		},
		"stake.subtype": func(raw json.RawMessage) (any, error) {
			types, err := batch[int](raw, "types")
			if err != nil {
				return nil, err
			}
			out := make([]any, 0, len(types))
			for _, n := range types {
				st := stdscript.ScriptType(n)
				sub, ok := txrules.StakeSubScriptType(st)
				out = append(out, []any{
					map[string]any{"n": int(st), "name": st.String()},
					map[string]any{"n": int(sub), "name": sub.String()},
					ok,
				})
			}
			return out, nil
		},
		"poolfee.valid": func(raw json.RawMessage) (any, error) {
			bits, err := batch[string](raw, "bits")
			if err != nil {
				return nil, err
			}
			out := make([]bool, 0, len(bits))
			for _, b := range bits {
				f, err := bitsToFloat(b)
				if err != nil {
					return nil, err
				}
				out = append(out, txrules.ValidPoolFeeRate(f))
			}
			return out, nil
		},
		"poolfee.int": func(raw json.RawMessage) (any, error) {
			bits, err := batch[string](raw, "bits")
			if err != nil {
				return nil, err
			}
			out := make([]int64, 0, len(bits))
			for _, b := range bits {
				f, err := bitsToFloat(b)
				if err != nil {
					return nil, err
				}
				// The same two steps as poolfees.go:43-44.
				poolFeeAbs := math.Floor(f * 100.0)
				out = append(out, int64(poolFeeAbs))
			}
			return out, nil
		},
		"poolfee.ticket": func(raw json.RawMessage) (any, error) {
			var args struct {
				Net   string               `json:"net"`
				Cases [][6]json.RawMessage `json:"cases"`
			}
			if err := json.Unmarshal(raw, &args); err != nil {
				return nil, err
			}
			p, err := params(args.Net)
			if err != nil {
				return nil, err
			}
			fees := make([]any, 0, len(args.Cases))
			for _, c := range args.Cases {
				var stakeDiff, relayFee int64
				var height int32
				var bits string
				var dcp0010, dcp0012 bool
				for i, dst := range []any{&stakeDiff, &relayFee, &height, &bits, &dcp0010, &dcp0012} {
					if err := json.Unmarshal(c[i], dst); err != nil {
						return nil, err
					}
				}
				poolFee, err := bitsToFloat(bits)
				if err != nil {
					return nil, err
				}
				if boundNet == "" {
					boundNet = args.Net
				}
				fees = append(fees, guarded(func() any {
					return int64(txrules.StakePoolTicketFee(dcrutil.Amount(stakeDiff),
						dcrutil.Amount(relayFee), height, poolFee, p, dcp0010, dcp0012))
				}))
			}
			return map[string]any{"bound": boundNet, "fees": fees}, nil
		},
		"bench": func(raw json.RawMessage) (any, error) {
			var args struct {
				Millis int `json:"millis"`
			}
			if err := json.Unmarshal(raw, &args); err != nil {
				return nil, err
			}
			return bench(time.Duration(args.Millis) * time.Millisecond), nil
		},
	})
}

// Sinks keep the benchmarked calls from being optimised away.
var (
	sinkInt    int
	sinkAmount dcrutil.Amount
	sinkBool   bool
	sinkErr    error
)

// bench times each main entry point with the inputs
// crates/purse-txrules/tests/bench.rs gives the Rust side.
func bench(budget time.Duration) []any {
	p2pkh := p2pkhScript()
	outs := []*wire.TxOut{
		{Value: 100_000_000, PkScript: p2pkh},
		{Value: 250_000_000, PkScript: p2pkh},
	}
	scriptSizes := []int{txsizes.RedeemP2PKHSigScriptSize, txsizes.RedeemP2PKHSigScriptSize}
	tx := wire.NewMsgTx()
	for range 2 {
		tx.TxIn = append(tx.TxIn, &wire.TxIn{
			ValueIn:         200_000_000,
			SignatureScript: make([]byte, txsizes.RedeemP2PKHSigScriptSize),
		})
	}
	tx.TxOut = outs
	mainnet := chaincfg.MainNetParams()
	// Builds the package's subsidy cache before timing, as a running
	// wallet has.
	txrules.StakePoolTicketFee(15e8, 0.05e8, 1_000_000, 2.59, mainnet, true, true)

	entries := []struct {
		name string
		f    func()
	}{
		{"EstimateSerializeSize(2 P2PKH in, 2 P2PKH out, change)", func() {
			sinkInt = txsizes.EstimateSerializeSize(scriptSizes, outs, txsizes.P2PKHPkScriptSize)
		}},
		{"EstimateSerializeSizeFromScriptSizes(2 in, 2 out, change)", func() {
			sinkInt = txsizes.EstimateSerializeSizeFromScriptSizes(scriptSizes,
				[]int{25, 25}, txsizes.P2PKHPkScriptSize)
		}},
		{"FeeForSerializeSize", func() {
			sinkAmount = txrules.FeeForSerializeSize(10_000, 419)
		}},
		{"IsDustOutput(P2PKH)", func() {
			sinkBool = txrules.IsDustOutput(outs[0], 10_000)
		}},
		{"CheckOutput(P2PKH)", func() {
			sinkErr = txrules.CheckOutput(outs[0], 10_000)
		}},
		{"PaysHighFees(2 in, 2 out)", func() {
			sinkBool = txrules.PaysHighFees(400_000_000, tx)
		}},
		{"TxPaysHighFees(2 in, 2 out)", func() {
			sinkBool, sinkErr = txrules.TxPaysHighFees(tx)
		}},
		{"StakePoolTicketFee(mainnet, height 1000000)", func() {
			sinkAmount = txrules.StakePoolTicketFee(15e8, 0.05e8, 1_000_000, 2.59, mainnet, true, true)
		}},
	}
	out := make([]any, 0, len(entries))
	for _, e := range entries {
		f := e.f
		start := time.Now()
		var best float64
		for time.Since(start) < budget || best == 0 {
			r := testing.Benchmark(func(b *testing.B) {
				for b.Loop() {
					f()
				}
			})
			ns := float64(r.T.Nanoseconds()) / float64(r.N)
			if best == 0 || ns < best {
				best = ns
			}
		}
		out = append(out, map[string]any{"name": e.name, "ns": best})
	}
	return out
}
