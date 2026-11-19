// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

// Command txauthor is the oracle for crates/purse-txauthor: it answers with
// dcrwallet's wallet/txauthor package at the parity pin.
//
// Byte strings are hex. Every command but consts and bench takes a batch
// and answers with one element per input, in order. An element whose call
// panics in Go answers {"panic": the recovered value's text, ...} with
// whatever was observed before the panic, so a Go panic is pinned as one
// rather than ending the process. A bounds-check panic's text is cut after
// "out of range", as tools/oracle/cmd/oprunner cuts it (ADR-0002), because
// the indices the runtime prints can differ between architectures. Errors are rendered with dcrwallet's
// daemon separator ":: " (dcrwallet.go:36-39; ADR-0011) as {"error":
// Error(), "kind": the first *errors.Error's kind String() or "", "op": its
// op or ""}.
//
// An output is {"value": int64, "version": u16, "script": hex}. An error a
// source returns is described by {"op": string, "kind": int (-1 for none),
// "text": string, "bare": bool}: bare returns errors.Kind(kind) itself as
// the error; otherwise errors.E of the op, kind and text that are set, or
// errors.New(text) when neither op nor kind is.
//
// Commands:
//
//	consts     {"max_script_element_size", "max_tx_size": {network: n}}
//	author     {"cases": [{"outputs": [output], "rate": relayFeePerKb,
//	           "max": maxTxSize, "source": source, "change": change}]}
//	           -> [{"calls": [the target of each fetchInputs call],
//	           "script_calls": Script() calls, "size_calls": ScriptSize()
//	           calls, and either "tx": hex of Tx.Bytes(), "prev":
//	           [PrevScripts], "total": TotalInput, "change_index",
//	           "size": EstimatedSignedSerializeSize; or an error}]
//	           source: {"utxos": [[value, signature script size, pkScript
//	           length]], "mode": "cumulative" (default: the upstream
//	           test's makeInputSource, author_test.go:40-64) or "all"
//	           (every UTXO on the first call), "delta": added to every
//	           reported Amount (wrapping), "fail_at": the 1-based call that
//	           returns "error" instead (0: none), "error", "limit": calls
//	           answered before the source fails with "input source: call
//	           limit reached" (0: 64)}. UTXO i becomes wire.NewTxIn of
//	           OutPoint{Hash: i little-endian in the first 8 bytes, Index:
//	           i, Tree: i mod 2} with its value and a nil signature
//	           script; its pkScript is bytes byte(i+j), j < length.
//	           change: {"size": ScriptSize(), "script": hex, "version":
//	           u16, "error": an error Script() returns instead, or null}
//	randomize  {"cases": [{"key": hex32, "n": outputs, "calls": [[index,
//	           change]]}]} -> [{"results": [{"index": returned index,
//	           "drawn": bytes drawn so far, "order": output values after
//	           the call} | {"panic", "drawn", "order"}]}]. The stream is
//	           reseeded with key per case; n outputs carry the values 0 to
//	           n-1; each call is RandomizeOutputPosition(outputs, index), or
//	           with change set, RandomizeChangePosition on an AuthoredTx
//	           whose ChangeIndex is index (the result's "index" is then the
//	           new ChangeIndex). This uses the copy of author.go in
//	           internal/txauthor, which draws from the seeded stream of
//	           internal/rand.
//	sign       {"cases": [{"net": network, "tx": hex, "prev": [hex], "keys":
//	           {address: [private key hex, signature type, compressed]},
//	           "scripts": {address: hex}, "key_errors": {address: error},
//	           "script_errors": {address: error}}]} -> [{"tx": hex of the
//	           transaction afterwards (signed inputs stay signed when a
//	           later one fails), "log": [["key"|"script", address]] in
//	           lookup order, "error": null | error}]. An address with
//	           neither a key nor a key error answers errors.E(NotExist,
//	           "no key for <address>"), and likewise "no script for".
//	bench      {"millis": per-entry time budget} -> [{"name", "ns"}],
//	           testing.Benchmark's ns/op for each main entry point with the
//	           fixed inputs crates/purse-txauthor/tests/bench.rs uses.
//
// A sign case on which SignTxOutput's merge recurses without end (QK-0746)
// ends the process with "fatal error: stack overflow", as in dcrwallet. The
// goroutine stack limit is lowered from Go's default 1 GB to 16 MB
// (debug.SetMaxStack) so that it ends in seconds: each level of that
// recursion re-tokenizes both signature scripts, and at the 16 KB script
// limit the default took over two minutes per case. No answer the oracle
// gives comes near that depth; the limit changes only when the overflow is
// reported.
package main

import (
	"encoding/binary"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"runtime/debug"
	"strings"
	"testing"
	"time"

	"decred.org/dcrwallet/v5/errors"
	"decred.org/dcrwallet/v5/wallet/txauthor"
	"decred.org/dcrwallet/v5/wallet/txsizes"
	"github.com/decred/dcrd/chaincfg/chainhash"
	"github.com/decred/dcrd/chaincfg/v3"
	"github.com/decred/dcrd/dcrec"
	"github.com/decred/dcrd/dcrec/secp256k1/v4"
	"github.com/decred/dcrd/dcrutil/v4"
	"github.com/decred/dcrd/txscript/v4"
	"github.com/decred/dcrd/txscript/v4/stdaddr"
	"github.com/decred/dcrd/wire"
	seeded "github.com/jzbz/dcr-purse/tools/oracle/cmd/txauthor/internal/rand"
	copied "github.com/jzbz/dcr-purse/tools/oracle/cmd/txauthor/internal/txauthor"
	"github.com/jzbz/dcr-purse/tools/oracle/internal/linejson"
)

type output struct {
	Value   int64  `json:"value"`
	Version uint16 `json:"version"`
	Script  string `json:"script"`
}

func (o *output) txOut() (*wire.TxOut, error) {
	script, err := hex.DecodeString(o.Script)
	if err != nil {
		return nil, err
	}
	return &wire.TxOut{Value: o.Value, Version: o.Version, PkScript: script}, nil
}

type errSpec struct {
	Op   string `json:"op"`
	Kind int    `json:"kind"`
	Text string `json:"text"`
	Bare bool   `json:"bare"`
}

func (s *errSpec) build() error {
	if s.Bare {
		return errors.Kind(s.Kind)
	}
	var args []any
	if s.Op != "" {
		args = append(args, errors.Op(s.Op))
	}
	if s.Kind >= 0 {
		args = append(args, errors.Kind(s.Kind))
	}
	if len(args) == 0 {
		return errors.New(s.Text)
	}
	if s.Text != "" {
		args = append(args, s.Text)
	}
	return errors.E(args...)
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

// guarded runs f, adding {"panic": text} to the answer if it panics.
func guarded(answer map[string]any, f func()) {
	defer func() {
		if r := recover(); r != nil {
			answer["panic"] = canonPanic(fmt.Sprint(r))
		}
	}()
	f()
}

// boundsPanics are the runtime's bounds-check panics, whose printed indices
// can depend on the architecture the binary was compiled for (see
// tools/oracle/cmd/oprunner's canonPanic).
var boundsPanics = []string{
	"runtime error: slice bounds out of range",
	"runtime error: index out of range",
}

// canonPanic drops the indices from a bounds-check panic's text, so that an
// answer is the same on every architecture. Any other text is unchanged.
func canonPanic(text string) string {
	for _, prefix := range boundsPanics {
		if strings.HasPrefix(text, prefix) {
			return prefix
		}
	}
	return text
}

func batch[T any](raw json.RawMessage) ([]T, error) {
	var args struct {
		Cases []T `json:"cases"`
	}
	if err := json.Unmarshal(raw, &args); err != nil {
		return nil, err
	}
	return args.Cases, nil
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

// ---------------------------------------------------------------------
// author
// ---------------------------------------------------------------------

type sourceSpec struct {
	Utxos  [][3]int64 `json:"utxos"`
	Mode   string     `json:"mode"`
	Delta  int64      `json:"delta"`
	FailAt int        `json:"fail_at"`
	Error  *errSpec   `json:"error"`
	Limit  int        `json:"limit"`
}

// maxScript bounds a script length the oracle allocates.
const maxScript = 1 << 20

var errCallLimit = errors.New("input source: call limit reached")

func utxoHash(i int) chainhash.Hash {
	var h chainhash.Hash
	binary.LittleEndian.PutUint64(h[:8], uint64(i))
	return h
}

// inputSource returns the source the spec describes and the record of the
// targets it was called with.
func (s *sourceSpec) inputSource() (txauthor.InputSource, *[]int64, error) {
	for _, u := range s.Utxos {
		if u[2] < 0 || u[2] > maxScript {
			return nil, nil, fmt.Errorf("pkScript length %d", u[2])
		}
	}
	limit := s.Limit
	if limit == 0 {
		limit = 64
	}
	calls := new([]int64)
	var (
		total   dcrutil.Amount
		inputs  []*wire.TxIn
		scripts [][]byte
		sizes   []int
		next    int
	)
	add := func() {
		u := s.Utxos[next]
		op := wire.OutPoint{Hash: utxoHash(next), Index: uint32(next), Tree: int8(next % 2)}
		inputs = append(inputs, wire.NewTxIn(&op, u[0], nil))
		script := make([]byte, u[2])
		for j := range script {
			script[j] = byte(next + j)
		}
		scripts = append(scripts, script)
		sizes = append(sizes, int(u[1]))
		total += dcrutil.Amount(u[0])
		next++
	}
	f := func(target dcrutil.Amount) (*txauthor.InputDetail, error) {
		*calls = append(*calls, int64(target))
		n := len(*calls)
		if n == s.FailAt && s.Error != nil {
			return nil, s.Error.build()
		}
		if n > limit {
			return nil, errCallLimit
		}
		switch s.Mode {
		case "all":
			for next < len(s.Utxos) {
				add()
			}
		default:
			for total < target && next < len(s.Utxos) {
				add()
			}
		}
		return &txauthor.InputDetail{
			Amount:            total + dcrutil.Amount(s.Delta),
			Inputs:            inputs[:len(inputs):len(inputs)],
			Scripts:           scripts[:len(scripts):len(scripts)],
			RedeemScriptSizes: sizes[:len(sizes):len(sizes)],
		}, nil
	}
	return f, calls, nil
}

type changeSpec struct {
	Size      int      `json:"size"`
	ScriptHex string   `json:"script"`
	Version   uint16   `json:"version"`
	Error     *errSpec `json:"error"`

	script      []byte
	scriptCalls int
	sizeCalls   int
}

func (c *changeSpec) Script() ([]byte, uint16, error) {
	c.scriptCalls++
	if c.Error != nil {
		return nil, 0, c.Error.build()
	}
	return c.script, c.Version, nil
}

func (c *changeSpec) ScriptSize() int {
	c.sizeCalls++
	return c.Size
}

type authorCase struct {
	Outputs []output   `json:"outputs"`
	Rate    int64      `json:"rate"`
	Max     int        `json:"max"`
	Source  sourceSpec `json:"source"`
	Change  changeSpec `json:"change"`
}

func author(c *authorCase) (any, error) {
	outs := make([]*wire.TxOut, 0, len(c.Outputs))
	for i := range c.Outputs {
		o, err := c.Outputs[i].txOut()
		if err != nil {
			return nil, err
		}
		outs = append(outs, o)
	}
	script, err := hex.DecodeString(c.Change.ScriptHex)
	if err != nil {
		return nil, err
	}
	change := &c.Change
	change.script = script
	source, calls, err := c.Source.inputSource()
	if err != nil {
		return nil, err
	}
	answer := map[string]any{}
	guarded(answer, func() {
		atx, err := txauthor.NewUnsignedTransaction(outs, dcrutil.Amount(c.Rate), source, change, c.Max)
		if err != nil {
			answer["error"] = errorValue(err)
			return
		}
		b, err := atx.Tx.Bytes()
		if err != nil {
			answer["error"] = map[string]any{"bytes": err.Error()}
			return
		}
		prev := make([]string, 0, len(atx.PrevScripts))
		for _, s := range atx.PrevScripts {
			prev = append(prev, hex.EncodeToString(s))
		}
		answer["tx"] = hex.EncodeToString(b)
		answer["prev"] = prev
		answer["total"] = int64(atx.TotalInput)
		answer["change_index"] = atx.ChangeIndex
		answer["size"] = atx.EstimatedSignedSerializeSize
	})
	answer["calls"] = *calls
	answer["script_calls"] = change.scriptCalls
	answer["size_calls"] = change.sizeCalls
	return answer, nil
}

// ---------------------------------------------------------------------
// randomize
// ---------------------------------------------------------------------

type randomizeCase struct {
	Key      string               `json:"key"`
	N        int                  `json:"n"`
	RawCalls [][2]json.RawMessage `json:"calls"`
}

func seed(key [32]byte) { seeded.Seed(key) }

func drawn() uint64 { return seeded.Drawn() }

// randomizeDraw is RandomizeOutputPosition over n outputs at index 0.
func randomizeDraw(n int) int {
	outs := make([]*wire.TxOut, n)
	for i := range outs {
		outs[i] = &wire.TxOut{Value: int64(i)}
	}
	return copied.RandomizeOutputPosition(outs, 0)
}

func randomize(c *randomizeCase) (any, error) {
	keyBytes, err := hex.DecodeString(c.Key)
	if err != nil || len(keyBytes) != 32 {
		return nil, fmt.Errorf("key %q", c.Key)
	}
	if c.N < 0 || c.N > 1<<16 {
		return nil, fmt.Errorf("n %d", c.N)
	}
	var key [32]byte
	copy(key[:], keyBytes)
	seed(key)
	outs := make([]*wire.TxOut, c.N)
	for i := range outs {
		outs[i] = &wire.TxOut{Value: int64(i)}
	}
	order := func() []int64 {
		v := make([]int64, len(outs))
		for i, o := range outs {
			v[i] = o.Value
		}
		return v
	}
	results := make([]any, 0, len(c.RawCalls))
	for _, raw := range c.RawCalls {
		var index int
		var change bool
		if err := json.Unmarshal(raw[0], &index); err != nil {
			return nil, err
		}
		if err := json.Unmarshal(raw[1], &change); err != nil {
			return nil, err
		}
		r := map[string]any{}
		guarded(r, func() {
			if change {
				atx := &copied.AuthoredTx{Tx: &wire.MsgTx{TxOut: outs}, ChangeIndex: index}
				atx.RandomizeChangePosition()
				r["index"] = atx.ChangeIndex
			} else {
				r["index"] = copied.RandomizeOutputPosition(outs, index)
			}
		})
		r["drawn"] = drawn()
		r["order"] = order()
		results = append(results, r)
	}
	return map[string]any{"results": results}, nil
}

// ---------------------------------------------------------------------
// sign
// ---------------------------------------------------------------------

type keySpec [3]json.RawMessage

type signCase struct {
	Net          string             `json:"net"`
	Tx           string             `json:"tx"`
	Prev         []string           `json:"prev"`
	Keys         map[string]keySpec `json:"keys"`
	Scripts      map[string]string  `json:"scripts"`
	KeyErrors    map[string]errSpec `json:"key_errors"`
	ScriptErrors map[string]errSpec `json:"script_errors"`
}

type secrets struct {
	c      *signCase
	params *chaincfg.Params
	log    []any
}

func (s *secrets) ChainParams() *chaincfg.Params { return s.params }

func (s *secrets) GetKey(a stdaddr.Address) ([]byte, dcrec.SignatureType, bool, error) {
	addr := a.String()
	s.log = append(s.log, []string{"key", addr})
	if e, ok := s.c.KeyErrors[addr]; ok {
		return nil, 0, false, e.build()
	}
	k, ok := s.c.Keys[addr]
	if !ok {
		return nil, 0, false, errors.E(errors.NotExist, "no key for "+addr)
	}
	var privHex string
	var sigType int
	var compressed bool
	if err := json.Unmarshal(k[0], &privHex); err != nil {
		panic(err)
	}
	if err := json.Unmarshal(k[1], &sigType); err != nil {
		panic(err)
	}
	if err := json.Unmarshal(k[2], &compressed); err != nil {
		panic(err)
	}
	priv, err := hex.DecodeString(privHex)
	if err != nil {
		panic(err)
	}
	return priv, dcrec.SignatureType(sigType), compressed, nil
}

func (s *secrets) GetScript(a stdaddr.Address) ([]byte, error) {
	addr := a.String()
	s.log = append(s.log, []string{"script", addr})
	if e, ok := s.c.ScriptErrors[addr]; ok {
		return nil, e.build()
	}
	script, ok := s.c.Scripts[addr]
	if !ok {
		return nil, errors.E(errors.NotExist, "no script for "+addr)
	}
	b, err := hex.DecodeString(script)
	if err != nil {
		panic(err)
	}
	return b, nil
}

func sign(c *signCase) (any, error) {
	p, err := params(c.Net)
	if err != nil {
		return nil, err
	}
	txBytes, err := hex.DecodeString(c.Tx)
	if err != nil {
		return nil, err
	}
	var tx wire.MsgTx
	if err := tx.FromBytes(txBytes); err != nil {
		return nil, err
	}
	prev := make([][]byte, 0, len(c.Prev))
	for _, h := range c.Prev {
		b, err := hex.DecodeString(h)
		if err != nil {
			return nil, err
		}
		prev = append(prev, b)
	}
	s := &secrets{c: c, params: p, log: []any{}}
	answer := map[string]any{}
	guarded(answer, func() {
		answer["error"] = errorValue(txauthor.AddAllInputScripts(&tx, prev, s))
	})
	b, err := tx.Bytes()
	if err != nil {
		return nil, err
	}
	answer["tx"] = hex.EncodeToString(b)
	answer["log"] = s.log
	return answer, nil
}

func main() {
	// dcrwallet's daemon sets this separator at startup (dcrwallet.go:36-39).
	errors.Separator = ":: "
	// See the package comment: an endless merge recursion is reported in
	// seconds rather than minutes.
	debug.SetMaxStack(16 << 20)
	linejson.Serve(map[string]linejson.Handler{
		"consts": func(json.RawMessage) (any, error) {
			maxTx := map[string]int{}
			for _, net := range []string{"mainnet", "testnet3", "simnet", "regnet"} {
				p, _ := params(net)
				maxTx[net] = p.MaxTxSize
			}
			return map[string]any{
				"max_script_element_size": txscript.MaxScriptElementSize,
				"max_tx_size":             maxTx,
			}, nil
		},
		"author": func(raw json.RawMessage) (any, error) {
			cases, err := batch[authorCase](raw)
			if err != nil {
				return nil, err
			}
			out := make([]any, 0, len(cases))
			for i := range cases {
				a, err := author(&cases[i])
				if err != nil {
					return nil, fmt.Errorf("case %d: %w", i, err)
				}
				out = append(out, a)
			}
			return out, nil
		},
		"randomize": func(raw json.RawMessage) (any, error) {
			cases, err := batch[randomizeCase](raw)
			if err != nil {
				return nil, err
			}
			out := make([]any, 0, len(cases))
			for i := range cases {
				a, err := randomize(&cases[i])
				if err != nil {
					return nil, fmt.Errorf("case %d: %w", i, err)
				}
				out = append(out, a)
			}
			return out, nil
		},
		"sign": func(raw json.RawMessage) (any, error) {
			cases, err := batch[signCase](raw)
			if err != nil {
				return nil, err
			}
			out := make([]any, 0, len(cases))
			for i := range cases {
				a, err := sign(&cases[i])
				if err != nil {
					return nil, fmt.Errorf("case %d: %w", i, err)
				}
				out = append(out, a)
			}
			return out, nil
		},
		"bench": func(raw json.RawMessage) (any, error) {
			var args struct {
				Millis int `json:"millis"`
			}
			if err := json.Unmarshal(raw, &args); err != nil {
				return nil, err
			}
			return bench(time.Duration(args.Millis) * time.Millisecond)
		},
	})
}

// ---------------------------------------------------------------------
// bench
// ---------------------------------------------------------------------

// Sinks keep the benchmarked calls from being optimised away.
var (
	sinkTx  *txauthor.AuthoredTx
	sinkErr error
	sinkInt int
)

type benchChange struct{ script []byte }

func (c benchChange) Script() ([]byte, uint16, error) { return c.script, 0, nil }
func (c benchChange) ScriptSize() int                 { return len(c.script) }

func p2pkhScript(fill byte) []byte {
	s := make([]byte, 25)
	s[0], s[1], s[2], s[23], s[24] = 0x76, 0xa9, 0x14, 0x88, 0xac
	for i := 3; i < 23; i++ {
		s[i] = fill
	}
	return s
}

// bench times each main entry point with the inputs
// crates/purse-txauthor/tests/bench.rs gives the Rust side: authoring two
// P2PKH outputs (1 and 2.5 DCR) at 10000 atoms/kB from five 1 DCR P2PKH
// UTXOs offered cumulatively, with a P2PKH change source; randomizing the
// change position among three outputs; and signing a transaction of two
// P2PKH inputs (secp256k1 ECDSA, compressed keys 0x01.. and 0x02..
// repeated) on mainnet.
func bench(budget time.Duration) (any, error) {
	outs := []*wire.TxOut{
		{Value: 100_000_000, PkScript: p2pkhScript(0x11)},
		{Value: 250_000_000, PkScript: p2pkhScript(0x22)},
	}
	change := benchChange{p2pkhScript(0x33)}
	newSource := func() txauthor.InputSource {
		var total dcrutil.Amount
		var inputs []*wire.TxIn
		var scripts [][]byte
		var sizes []int
		next := 0
		return func(target dcrutil.Amount) (*txauthor.InputDetail, error) {
			for total < target && next < 5 {
				op := wire.OutPoint{Hash: utxoHash(next), Index: uint32(next)}
				inputs = append(inputs, wire.NewTxIn(&op, 100_000_000, nil))
				scripts = append(scripts, p2pkhScript(byte(next)))
				sizes = append(sizes, txsizes.RedeemP2PKHSigScriptSize)
				total += 100_000_000
				next++
			}
			return &txauthor.InputDetail{Amount: total, Inputs: inputs,
				Scripts: scripts, RedeemScriptSizes: sizes}, nil
		}
	}

	mainnet := chaincfg.MainNetParams()
	signC := &signCase{Keys: map[string]keySpec{}}
	var prevScripts [][]byte
	unsigned := wire.NewMsgTx()
	for i := 0; i < 2; i++ {
		var k [32]byte
		for j := range k {
			k[j] = byte(i + 1)
		}
		priv := secp256k1.PrivKeyFromBytes(k[:])
		pkh := stdaddr.Hash160(priv.PubKey().SerializeCompressed())
		addr, err := stdaddr.NewAddressPubKeyHashEcdsaSecp256k1V0(pkh, mainnet)
		if err != nil {
			return nil, err
		}
		_, script := addr.PaymentScript()
		prevScripts = append(prevScripts, script)
		privJSON, _ := json.Marshal(hex.EncodeToString(k[:]))
		signC.Keys[addr.String()] = keySpec{privJSON, json.RawMessage("0"), json.RawMessage("true")}
		op := wire.OutPoint{Hash: utxoHash(i), Index: uint32(i)}
		unsigned.AddTxIn(wire.NewTxIn(&op, 200_000_000, nil))
	}
	unsigned.AddTxOut(&wire.TxOut{Value: 300_000_000, PkScript: p2pkhScript(0x44)})
	secretsSrc := &secrets{c: signC, params: mainnet}

	entries := []struct {
		name string
		f    func()
	}{
		{"NewUnsignedTransaction(2 P2PKH out, 5 UTXOs, change)", func() {
			sinkTx, sinkErr = txauthor.NewUnsignedTransaction(outs, 10_000, newSource(), change,
				mainnet.MaxTxSize)
		}},
		{"RandomizeOutputPosition(3 outputs)", func() {
			o := []*wire.TxOut{outs[0], outs[1], outs[0]}
			sinkInt = txauthor.RandomizeOutputPosition(o, 2)
		}},
		{"AddAllInputScripts(2 P2PKH inputs)", func() {
			tx := unsigned.Copy()
			secretsSrc.log = secretsSrc.log[:0]
			sinkErr = txauthor.AddAllInputScripts(tx, prevScripts, secretsSrc)
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
	return out, nil
}
