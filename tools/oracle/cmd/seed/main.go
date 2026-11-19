// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

// Command seed is the oracle for crates/purse-seed: dcrwallet's own
// pgpwordlist and walletseed packages behind the linejson protocol.
//
// Commands:
//
//	wordlist                                -> [512]string, ByteToMnemonic(b, i) for b in 0..255, i in 0..1
//	encode {"seed": hex}                    -> {"mnemonic": EncodeMnemonic, "words": EncodeMnemonicSlice}
//	decode-mnemonics {"words": [string]}    -> {"decoded": hex}
//	decode-user-input {"input": string}     -> {"seed": hex}
//	generate-random-seed {"size": uint}     -> {"len": n}
//	generate-random-seed-injected {"size": uint, "start": byte}
//	                                        -> {"seed": hex, "served": n} or
//	                                           {"error": text, "kind": name, "served": n}
//
// generate-random-seed-injected replaces crypto/rand.Reader with a reader
// serving start, start+1, ... (mod 256) for the one call, so the bytes
// GenerateRandomSeed returns and how many it read can be compared; crypto/rand.Read
// reads through a replaced Reader (go1.27 crypto/rand/rand.go, Read).
//
// The decode commands take an optional "separator" member that sets
// errors.Separator while the error is rendered; the default is the daemon's
// ":: " (dcrwallet.go sets it in init). Errors carry the outermost
// errors.Kind's constant name as the linejson kind.
package main

import (
	"crypto/rand"
	"encoding/hex"
	"encoding/json"
	"fmt"

	"decred.org/dcrwallet/v5/errors"
	"decred.org/dcrwallet/v5/pgpwordlist"
	"decred.org/dcrwallet/v5/walletseed"
	"github.com/jzbz/dcr-purse/tools/oracle/internal/linejson"
)

const daemonSeparator = ":: "

func kindName(k errors.Kind) string {
	switch k {
	case errors.Other:
		return "Other"
	case errors.Invalid:
		return "Invalid"
	case errors.Encoding:
		return "Encoding"
	default:
		return fmt.Sprintf("Kind(%d)", int(k))
	}
}

// wrap renders err with the requested separator and attaches its kind.
func wrap(err error, separator *string) error {
	sep := daemonSeparator
	if separator != nil {
		sep = *separator
	}
	errors.Separator = sep
	text := err.Error()
	errors.Separator = daemonSeparator
	kind := "Other"
	var e *errors.Error
	if errors.As(err, &e) {
		kind = kindName(e.Kind)
	}
	return &linejson.Error{Kind: kind, Text: text}
}

// countingReader serves consecutive byte values and counts what it served.
type countingReader struct {
	next   byte
	served int
}

func (r *countingReader) Read(p []byte) (int, error) {
	for i := range p {
		p[i] = r.next
		r.next++
	}
	r.served += len(p)
	return len(p), nil
}

func main() {
	errors.Separator = daemonSeparator
	linejson.Serve(map[string]linejson.Handler{
		"wordlist": func(json.RawMessage) (any, error) {
			words := make([]string, 0, 512)
			for b := 0; b < 256; b++ {
				for i := 0; i < 2; i++ {
					words = append(words, pgpwordlist.ByteToMnemonic(byte(b), i))
				}
			}
			return words, nil
		},
		"encode": func(raw json.RawMessage) (any, error) {
			var args struct {
				Seed string `json:"seed"`
			}
			if err := json.Unmarshal(raw, &args); err != nil {
				return nil, err
			}
			seed, err := hex.DecodeString(args.Seed)
			if err != nil {
				return nil, err
			}
			return map[string]any{
				"mnemonic": walletseed.EncodeMnemonic(seed),
				"words":    walletseed.EncodeMnemonicSlice(seed),
			}, nil
		},
		"decode-mnemonics": func(raw json.RawMessage) (any, error) {
			var args struct {
				Words     []string `json:"words"`
				Separator *string  `json:"separator"`
			}
			if err := json.Unmarshal(raw, &args); err != nil {
				return nil, err
			}
			decoded, err := pgpwordlist.DecodeMnemonics(args.Words)
			if err != nil {
				return nil, wrap(err, args.Separator)
			}
			return map[string]any{"decoded": hex.EncodeToString(decoded)}, nil
		},
		"decode-user-input": func(raw json.RawMessage) (any, error) {
			var args struct {
				Input     string  `json:"input"`
				Separator *string `json:"separator"`
			}
			if err := json.Unmarshal(raw, &args); err != nil {
				return nil, err
			}
			seed, err := walletseed.DecodeUserInput(args.Input)
			if err != nil {
				return nil, wrap(err, args.Separator)
			}
			return map[string]any{"seed": hex.EncodeToString(seed)}, nil
		},
		"generate-random-seed": func(raw json.RawMessage) (any, error) {
			var args struct {
				Size uint `json:"size"`
			}
			if err := json.Unmarshal(raw, &args); err != nil {
				return nil, err
			}
			seed, err := walletseed.GenerateRandomSeed(args.Size)
			if err != nil {
				return nil, wrap(err, nil)
			}
			return map[string]any{"len": len(seed)}, nil
		},
		"generate-random-seed-injected": func(raw json.RawMessage) (any, error) {
			var args struct {
				Size  uint `json:"size"`
				Start byte `json:"start"`
			}
			if err := json.Unmarshal(raw, &args); err != nil {
				return nil, err
			}
			reader := &countingReader{next: args.Start}
			saved := rand.Reader
			rand.Reader = reader
			seed, err := walletseed.GenerateRandomSeed(args.Size)
			rand.Reader = saved
			if err != nil {
				e := wrap(err, nil).(*linejson.Error)
				return map[string]any{"error": e.Text, "kind": e.Kind, "served": reader.served}, nil
			}
			return map[string]any{"seed": hex.EncodeToString(seed), "served": reader.served}, nil
		},
	})
}
