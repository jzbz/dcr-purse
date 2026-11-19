// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

// Command sntrup is the oracle for crates/sntrup4591761: the KEM
// github.com/companyzero/sntrup4591761 at the version dcrwallet links
// (v0.0.0-20220309191932-9e0f3af2f07a, through github.com/decred/dcrd/mixing
// v0.7.5), and its exported subpackages zx, rq and r3.
//
// Byte strings travel as lower-case hex; polynomials as JSON arrays of
// integers in coefficient order (index k multiplies x^k).
//
// # The KEM
//
// Randomness is injected: "rand" is the whole byte stream the operation may
// read, served by a bytes.Reader in place of the io.Reader Go's callers
// pass. "consumed" is how many of those bytes the operation read. When the
// stream runs out, the operation's error is returned as Go returns it
// ("EOF" or "unexpected EOF", the bytes.Reader's through io.ReadFull), with
// kind "rand" and the bytes consumed in the text after a tab.
//
//	keygen        {"rand"} -> {"pk", "sk", "consumed"}: GenerateKey
//	encapsulate   {"rand", "pk"} -> {"ct", "key", "consumed"}: Encapsulate
//	decapsulate   {"ct", "sk"} -> {"key", "ok"}: Decapsulate; ok is Go's
//	              int (1 or 0), key is returned whatever ok is
//
// pk, sk and ct must be exactly PublicKeySize, PrivateKeySize and
// CiphertextSize bytes; any bytes of those lengths are passed through.
//
// # The subpackages
//
//	zx.random-small   {"rand"} -> {"f", "consumed"}: zx.RandomSmall
//	zx.random-tsmall  {"rand"} -> {"f", "consumed"}: zx.RandomTSmall
//	zx.decode         {"bytes"} (191) -> [int8]: zx.Decode
//	rq.decode         {"bytes"} (1218) -> [int16]: rq.Decode
//	rq.decode-rounded {"bytes"} (1015) -> [int16]: rq.DecodeRounded
//	rq.mult           {"f": [int16], "g": [int8]} -> [int16]: rq.Mult
//	r3.mult           {"f": [int8], "g": [int8]} -> [int8]: r3.Mult
//	r3.reciprocal     {"s": [int8]} -> {"r": [int8], "ret": int}: r3.Reciprocal
//	rq.reciprocal3    {"s": [int8]} -> {"r": [int16], "ret": int}: rq.Reciprocal3
//
// The polynomial arguments are any values of the Go types, not only the
// ones the KEM produces.
//
// # Timing
//
//	bench {"rand", "reps"} -> {"keygen", "encapsulate", "decapsulate":
//	      [ns], "pk", "ct", "key", "ok", "go"}
//
// Runs reps rounds of GenerateKey, Encapsulate to the new key and
// Decapsulate of the new ciphertext, each round reading "rand" from the
// start through a fresh bytes.Reader, and returns each operation's wall
// time per round in nanoseconds, the last round's outputs (for the caller
// to check against its own) and runtime.Version(). Three untimed rounds run
// first.
package main

import (
	"bytes"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"io"
	"runtime"
	"time"

	"github.com/companyzero/sntrup4591761"
	"github.com/companyzero/sntrup4591761/r3"
	"github.com/companyzero/sntrup4591761/rq"
	"github.com/companyzero/sntrup4591761/zx"
	"github.com/jzbz/dcr-purse/tools/oracle/internal/linejson"
)

const p = 761

// fixed decodes hex of exactly n bytes.
func fixed(name, s string, n int) ([]byte, error) {
	b, err := hex.DecodeString(s)
	if err != nil {
		return nil, fmt.Errorf("%s: %v", name, err)
	}
	if len(b) != n {
		return nil, fmt.Errorf("%s: %d bytes, want %d", name, len(b), n)
	}
	return b, nil
}

// stream is the injected reader and a count of what it served.
type stream struct {
	r   *bytes.Reader
	len int
}

func newStream(s string) (*stream, error) {
	b, err := hex.DecodeString(s)
	if err != nil {
		return nil, fmt.Errorf("rand: %v", err)
	}
	return &stream{r: bytes.NewReader(b), len: len(b)}, nil
}

func (s *stream) consumed() int { return s.len - s.r.Len() }

func (s *stream) fail(err error) error {
	return &linejson.Error{Kind: "rand", Text: fmt.Sprintf("%v\t%d", err, s.consumed())}
}

func poly8(name string, v []int8) (*[p]int8, error) {
	if len(v) != p {
		return nil, fmt.Errorf("%s: %d coefficients, want %d", name, len(v), p)
	}
	out := new([p]int8)
	copy(out[:], v)
	return out, nil
}

func poly16(name string, v []int16) (*[p]int16, error) {
	if len(v) != p {
		return nil, fmt.Errorf("%s: %d coefficients, want %d", name, len(v), p)
	}
	out := new([p]int16)
	copy(out[:], v)
	return out, nil
}

// ints8 and ints16 widen a polynomial to []int for encoding/json.
func ints8(v []int8) []int {
	out := make([]int, len(v))
	for i, x := range v {
		out[i] = int(x)
	}
	return out
}

func ints16(v []int16) []int {
	out := make([]int, len(v))
	for i, x := range v {
		out[i] = int(x)
	}
	return out
}

type randArgs struct {
	Rand string `json:"rand"`
}

type bytesArgs struct {
	Bytes string `json:"bytes"`
}

func main() {
	handlers := map[string]linejson.Handler{
		"keygen": func(raw json.RawMessage) (any, error) {
			var args randArgs
			if err := json.Unmarshal(raw, &args); err != nil {
				return nil, err
			}
			s, err := newStream(args.Rand)
			if err != nil {
				return nil, err
			}
			pk, sk, err := sntrup4591761.GenerateKey(s.r)
			if err != nil {
				return nil, s.fail(err)
			}
			return map[string]any{
				"pk":       hex.EncodeToString(pk[:]),
				"sk":       hex.EncodeToString(sk[:]),
				"consumed": s.consumed(),
			}, nil
		},
		"encapsulate": func(raw json.RawMessage) (any, error) {
			var args struct {
				Rand string `json:"rand"`
				PK   string `json:"pk"`
			}
			if err := json.Unmarshal(raw, &args); err != nil {
				return nil, err
			}
			b, err := fixed("pk", args.PK, sntrup4591761.PublicKeySize)
			if err != nil {
				return nil, err
			}
			pk := new(sntrup4591761.PublicKey)
			copy(pk[:], b)
			s, err := newStream(args.Rand)
			if err != nil {
				return nil, err
			}
			ct, key, err := sntrup4591761.Encapsulate(s.r, pk)
			if err != nil {
				return nil, s.fail(err)
			}
			return map[string]any{
				"ct":       hex.EncodeToString(ct[:]),
				"key":      hex.EncodeToString(key[:]),
				"consumed": s.consumed(),
			}, nil
		},
		"decapsulate": func(raw json.RawMessage) (any, error) {
			var args struct {
				CT string `json:"ct"`
				SK string `json:"sk"`
			}
			if err := json.Unmarshal(raw, &args); err != nil {
				return nil, err
			}
			cb, err := fixed("ct", args.CT, sntrup4591761.CiphertextSize)
			if err != nil {
				return nil, err
			}
			sb, err := fixed("sk", args.SK, sntrup4591761.PrivateKeySize)
			if err != nil {
				return nil, err
			}
			ct := new(sntrup4591761.Ciphertext)
			copy(ct[:], cb)
			sk := new(sntrup4591761.PrivateKey)
			copy(sk[:], sb)
			key, ok := sntrup4591761.Decapsulate(ct, sk)
			return map[string]any{"key": hex.EncodeToString(key[:]), "ok": ok}, nil
		},
		"zx.random-small": func(raw json.RawMessage) (any, error) {
			return randomPoly(raw, zx.RandomSmall)
		},
		"zx.random-tsmall": func(raw json.RawMessage) (any, error) {
			return randomPoly(raw, zx.RandomTSmall)
		},
		"zx.decode": func(raw json.RawMessage) (any, error) {
			var args bytesArgs
			if err := json.Unmarshal(raw, &args); err != nil {
				return nil, err
			}
			b, err := fixed("bytes", args.Bytes, 191)
			if err != nil {
				return nil, err
			}
			return ints8(zx.Decode(b)[:]), nil
		},
		"rq.decode": func(raw json.RawMessage) (any, error) {
			var args bytesArgs
			if err := json.Unmarshal(raw, &args); err != nil {
				return nil, err
			}
			b, err := fixed("bytes", args.Bytes, 1218)
			if err != nil {
				return nil, err
			}
			return ints16(rq.Decode(b)[:]), nil
		},
		"rq.decode-rounded": func(raw json.RawMessage) (any, error) {
			var args bytesArgs
			if err := json.Unmarshal(raw, &args); err != nil {
				return nil, err
			}
			b, err := fixed("bytes", args.Bytes, 1015)
			if err != nil {
				return nil, err
			}
			return ints16(rq.DecodeRounded(b)[:]), nil
		},
		"rq.mult": func(raw json.RawMessage) (any, error) {
			var args struct {
				F []int16 `json:"f"`
				G []int8  `json:"g"`
			}
			if err := json.Unmarshal(raw, &args); err != nil {
				return nil, err
			}
			f, err := poly16("f", args.F)
			if err != nil {
				return nil, err
			}
			g, err := poly8("g", args.G)
			if err != nil {
				return nil, err
			}
			h := new([p]int16)
			rq.Mult(h, f, g)
			return ints16(h[:]), nil
		},
		"r3.mult": func(raw json.RawMessage) (any, error) {
			var args struct {
				F []int8 `json:"f"`
				G []int8 `json:"g"`
			}
			if err := json.Unmarshal(raw, &args); err != nil {
				return nil, err
			}
			f, err := poly8("f", args.F)
			if err != nil {
				return nil, err
			}
			g, err := poly8("g", args.G)
			if err != nil {
				return nil, err
			}
			h := new([p]int8)
			r3.Mult(h, f, g)
			return ints8(h[:]), nil
		},
		"r3.reciprocal": func(raw json.RawMessage) (any, error) {
			var args struct {
				S []int8 `json:"s"`
			}
			if err := json.Unmarshal(raw, &args); err != nil {
				return nil, err
			}
			s, err := poly8("s", args.S)
			if err != nil {
				return nil, err
			}
			r := new([p]int8)
			ret := r3.Reciprocal(r, s)
			return map[string]any{"r": ints8(r[:]), "ret": ret}, nil
		},
		"rq.reciprocal3": func(raw json.RawMessage) (any, error) {
			var args struct {
				S []int8 `json:"s"`
			}
			if err := json.Unmarshal(raw, &args); err != nil {
				return nil, err
			}
			s, err := poly8("s", args.S)
			if err != nil {
				return nil, err
			}
			r := new([p]int16)
			ret := rq.Reciprocal3(r, s)
			return map[string]any{"r": ints16(r[:]), "ret": ret}, nil
		},
		"bench": bench,
	}
	linejson.Serve(handlers)
}

// bench times the three KEM operations; see the package documentation.
func bench(raw json.RawMessage) (any, error) {
	var args struct {
		Rand string `json:"rand"`
		Reps int    `json:"reps"`
	}
	if err := json.Unmarshal(raw, &args); err != nil {
		return nil, err
	}
	if args.Reps < 1 || args.Reps > 100000 {
		return nil, fmt.Errorf("reps: %d, want 1..100000", args.Reps)
	}
	stream, err := hex.DecodeString(args.Rand)
	if err != nil {
		return nil, fmt.Errorf("rand: %v", err)
	}
	var (
		pk  *sntrup4591761.PublicKey
		ct  *sntrup4591761.Ciphertext
		key *sntrup4591761.SharedKey
		ok  int
	)
	kg := make([]int64, 0, args.Reps)
	en := make([]int64, 0, args.Reps)
	de := make([]int64, 0, args.Reps)
	for i := -3; i < args.Reps; i++ {
		r := bytes.NewReader(stream)
		t0 := time.Now()
		var sk *sntrup4591761.PrivateKey
		pk, sk, err = sntrup4591761.GenerateKey(r)
		t1 := time.Now()
		if err != nil {
			return nil, fmt.Errorf("keygen: %v", err)
		}
		ct, _, err = sntrup4591761.Encapsulate(r, pk)
		t2 := time.Now()
		if err != nil {
			return nil, fmt.Errorf("encapsulate: %v", err)
		}
		key, ok = sntrup4591761.Decapsulate(ct, sk)
		t3 := time.Now()
		if i >= 0 {
			kg = append(kg, t1.Sub(t0).Nanoseconds())
			en = append(en, t2.Sub(t1).Nanoseconds())
			de = append(de, t3.Sub(t2).Nanoseconds())
		}
	}
	return map[string]any{
		"keygen":      kg,
		"encapsulate": en,
		"decapsulate": de,
		"pk":          hex.EncodeToString(pk[:]),
		"ct":          hex.EncodeToString(ct[:]),
		"key":         hex.EncodeToString(key[:]),
		"ok":          ok,
		"go":          runtime.Version(),
	}, nil
}

// randomPoly serves zx.RandomSmall and zx.RandomTSmall from {"rand"}.
func randomPoly(raw json.RawMessage, gen func(*[p]int8, io.Reader) error) (any, error) {
	var args randArgs
	if err := json.Unmarshal(raw, &args); err != nil {
		return nil, err
	}
	s, err := newStream(args.Rand)
	if err != nil {
		return nil, err
	}
	f := new([p]int8)
	if err := gen(f, s.r); err != nil {
		return nil, s.fail(err)
	}
	return map[string]any{"f": ints8(f[:]), "consumed": s.consumed()}, nil
}
