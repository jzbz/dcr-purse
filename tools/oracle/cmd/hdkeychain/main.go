// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

// Command hdkeychain is the oracle for crates/purse-hdkeychain: it answers
// with github.com/decred/dcrd/hdkeychain/v3 at the version dcrwallet links.
//
// Every key is reported as one canonical text line, which the Rust tests
// build the same way from their own keys:
//
//	K|<private 0/1>|<depth>|<child num>|<parent fp>|<String()>|<Neuter().String()>|<pubkey hex>|<privkey hex, or ! and the error>|<PrivKeyFromBytes(privkey).Serialize() hex, or ! and the error>
//	E|<error text>
//
// Commands:
//
//	consts         the package's constants and error texts, and the HD
//	               version bytes of chaincfg's four networks
//	derive         {"net": {"priv": hex, "pub": hex}, "digest": bool,
//	                "jobs": [{"seed": hex} or {"key": string}, "paths": [path]}]}
//	               For each job, the root key (NewMaster or NewKeyFromString)
//	               then each path from it. A path is "/"-separated steps;
//	               a step is an optional decimal index (derived with Child,
//	               or ChildBIP32Std with flag s) followed by flags applied in
//	               order: r re-parses the key from its String(), n neuters
//	               it, z zeroes it. One line per root and per step; a path
//	               stops at its first error. With digest, each job's answer
//	               is the hex SHA-256 of its lines, each followed by "\n",
//	               with "#\n" after every path; otherwise the lines.
//	parse          {"net": ..., "keys": [string]}: one line per key.
//	generate_seed  {"lengths": [int]}: "len=<n>" or "E|<error>" per length.
//	wallet         {"net": name, "digest": bool, "seeds": [hex]}: the account
//	               extended public keys and first addresses dcrwallet derives
//	               from each seed (see wallet.go).
package main

import (
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"runtime"
	"strconv"
	"strings"
	"sync"

	"github.com/decred/dcrd/chaincfg/v3"
	"github.com/decred/dcrd/dcrec/secp256k1/v4"
	"github.com/decred/dcrd/hdkeychain/v3"
	"github.com/jzbz/dcr-purse/tools/oracle/internal/linejson"
)

type netParams struct {
	priv, pub [4]byte
}

func (p *netParams) HDPrivKeyVersion() [4]byte { return p.priv }
func (p *netParams) HDPubKeyVersion() [4]byte  { return p.pub }

type netArg struct {
	Priv string `json:"priv"`
	Pub  string `json:"pub"`
}

func (a netArg) params() (*netParams, error) {
	var p netParams
	for _, f := range []struct {
		s   string
		dst *[4]byte
	}{{a.Priv, &p.priv}, {a.Pub, &p.pub}} {
		b, err := hex.DecodeString(f.s)
		if err != nil || len(b) != 4 {
			return nil, fmt.Errorf("bad version %q", f.s)
		}
		copy(f.dst[:], b)
	}
	return &p, nil
}

func keyLine(k *hdkeychain.ExtendedKey) string {
	priv := 0
	if k.IsPrivate() {
		priv = 1
	}
	// The stored private key, and the 32-byte private key Go code makes of
	// it with secp256k1.PrivKeyFromBytes (left-padded, reduced mod n).
	privStr, paddedStr := "", ""
	if pk, err := k.SerializedPrivKey(); err != nil {
		privStr = "!" + err.Error()
		paddedStr = privStr
	} else {
		privStr = hex.EncodeToString(pk)
		paddedStr = hex.EncodeToString(secp256k1.PrivKeyFromBytes(pk).Serialize())
	}
	return fmt.Sprintf("K|%d|%d|%d|%d|%s|%s|%s|%s|%s", priv, k.Depth(), k.ChildNum(),
		k.ParentFingerprint(), k.String(), k.Neuter().String(),
		hex.EncodeToString(k.SerializedPubKey()), privStr, paddedStr)
}

func errLine(err error) string { return "E|" + err.Error() }

type step struct {
	hasIndex bool
	index    uint32
	flags    string
}

func parsePath(path string) ([]step, error) {
	if path == "" {
		return nil, nil
	}
	var steps []step
	for _, tok := range strings.Split(path, "/") {
		digits := strings.TrimRight(tok, "srnz")
		flags := tok[len(digits):]
		var s step
		if digits != "" {
			v, err := strconv.ParseUint(digits, 10, 32)
			if err != nil {
				return nil, fmt.Errorf("bad step %q", tok)
			}
			s.hasIndex, s.index = true, uint32(v)
		}
		s.flags = flags
		steps = append(steps, s)
	}
	return steps, nil
}

// runPath derives along steps from a fresh copy of the root and returns one
// line per step.
func runPath(root func() (*hdkeychain.ExtendedKey, error), steps []step,
	net *netParams) []string {

	k, err := root()
	if err != nil {
		return []string{errLine(err)}
	}
	lines := []string{keyLine(k)}
	for _, s := range steps {
		if s.hasIndex {
			if strings.ContainsRune(s.flags, 's') {
				k, err = k.ChildBIP32Std(s.index)
			} else {
				k, err = k.Child(s.index)
			}
			if err != nil {
				return append(lines, errLine(err))
			}
		}
		for _, f := range s.flags {
			switch f {
			case 'r':
				k, err = hdkeychain.NewKeyFromString(k.String(), net)
				if err != nil {
					return append(lines, errLine(err))
				}
			case 'n':
				k = k.Neuter()
			case 'z':
				k.Zero()
			}
		}
		lines = append(lines, keyLine(k))
	}
	return lines
}

type job struct {
	Seed  *string  `json:"seed"`
	Key   *string  `json:"key"`
	Paths []string `json:"paths"`
}

type deriveArgs struct {
	Net    netArg `json:"net"`
	Digest bool   `json:"digest"`
	Jobs   []job  `json:"jobs"`
}

func runJob(j job, net *netParams) ([][]string, error) {
	var root func() (*hdkeychain.ExtendedKey, error)
	switch {
	case j.Seed != nil:
		seed, err := hex.DecodeString(*j.Seed)
		if err != nil {
			return nil, fmt.Errorf("bad seed hex: %v", err)
		}
		root = func() (*hdkeychain.ExtendedKey, error) {
			return hdkeychain.NewMaster(seed, net)
		}
	case j.Key != nil:
		key := *j.Key
		root = func() (*hdkeychain.ExtendedKey, error) {
			return hdkeychain.NewKeyFromString(key, net)
		}
	default:
		return nil, errors.New("job has neither seed nor key")
	}
	out := make([][]string, 0, len(j.Paths))
	for _, p := range j.Paths {
		steps, err := parsePath(p)
		if err != nil {
			return nil, err
		}
		out = append(out, runPath(root, steps, net))
	}
	return out, nil
}

func digest(paths [][]string) string {
	h := sha256.New()
	for _, lines := range paths {
		for _, l := range lines {
			h.Write([]byte(l))
			h.Write([]byte{'\n'})
		}
		h.Write([]byte("#\n"))
	}
	return hex.EncodeToString(h.Sum(nil))
}

func derive(raw json.RawMessage) (any, error) {
	var args deriveArgs
	if err := json.Unmarshal(raw, &args); err != nil {
		return nil, err
	}
	net, err := args.Net.params()
	if err != nil {
		return nil, err
	}

	results := make([]any, len(args.Jobs))
	errs := make([]error, len(args.Jobs))
	next := make(chan int)
	var wg sync.WaitGroup
	for w := 0; w < runtime.GOMAXPROCS(0); w++ {
		wg.Add(1)
		go func() {
			defer wg.Done()
			for i := range next {
				paths, err := runJob(args.Jobs[i], net)
				if err != nil {
					errs[i] = err
					continue
				}
				if args.Digest {
					results[i] = digest(paths)
				} else {
					results[i] = paths
				}
			}
		}()
	}
	for i := range args.Jobs {
		next <- i
	}
	close(next)
	wg.Wait()
	for i, err := range errs {
		if err != nil {
			return nil, fmt.Errorf("job %d: %v", i, err)
		}
	}
	return results, nil
}

func parse(raw json.RawMessage) (any, error) {
	var args struct {
		Net  netArg   `json:"net"`
		Keys []string `json:"keys"`
	}
	if err := json.Unmarshal(raw, &args); err != nil {
		return nil, err
	}
	net, err := args.Net.params()
	if err != nil {
		return nil, err
	}
	out := make([]string, len(args.Keys))
	for i, s := range args.Keys {
		k, err := hdkeychain.NewKeyFromString(s, net)
		if err != nil {
			out[i] = errLine(err)
		} else {
			out[i] = keyLine(k)
		}
	}
	return out, nil
}

func generateSeed(raw json.RawMessage) (any, error) {
	var args struct {
		Lengths []int `json:"lengths"`
	}
	if err := json.Unmarshal(raw, &args); err != nil {
		return nil, err
	}
	out := make([]string, len(args.Lengths))
	for i, n := range args.Lengths {
		if n < 0 || n > 255 {
			return nil, fmt.Errorf("length %d is not a uint8", n)
		}
		seed, err := hdkeychain.GenerateSeed(uint8(n))
		if err != nil {
			out[i] = errLine(err)
		} else {
			out[i] = fmt.Sprintf("len=%d", len(seed))
		}
	}
	return out, nil
}

func consts(json.RawMessage) (any, error) {
	nets := map[string]any{}
	for name, p := range map[string]*chaincfg.Params{
		"mainnet":  chaincfg.MainNetParams(),
		"testnet3": chaincfg.TestNet3Params(),
		"simnet":   chaincfg.SimNetParams(),
		"regnet":   chaincfg.RegNetParams(),
	} {
		priv, pub := p.HDPrivKeyVersion(), p.HDPubKeyVersion()
		nets[name] = map[string]string{
			"priv": hex.EncodeToString(priv[:]),
			"pub":  hex.EncodeToString(pub[:]),
		}
	}
	return map[string]any{
		"nets":                    nets,
		"RecommendedSeedLen":      hdkeychain.RecommendedSeedLen,
		"HardenedKeyStart":        uint32(hdkeychain.HardenedKeyStart),
		"MinSeedBytes":            hdkeychain.MinSeedBytes,
		"MaxSeedBytes":            hdkeychain.MaxSeedBytes,
		"ErrDeriveHardFromPublic": hdkeychain.ErrDeriveHardFromPublic.Error(),
		"ErrNotPrivExtKey":        hdkeychain.ErrNotPrivExtKey.Error(),
		"ErrInvalidChild":         hdkeychain.ErrInvalidChild.Error(),
		"ErrUnusableSeed":         hdkeychain.ErrUnusableSeed.Error(),
		"ErrInvalidSeedLen":       hdkeychain.ErrInvalidSeedLen.Error(),
		"ErrBadChecksum":          hdkeychain.ErrBadChecksum.Error(),
		"ErrInvalidKeyLen":        hdkeychain.ErrInvalidKeyLen.Error(),
		"ErrWrongNetwork":         hdkeychain.ErrWrongNetwork.Error(),
	}, nil
}

func main() {
	linejson.Serve(map[string]linejson.Handler{
		"consts":        consts,
		"derive":        derive,
		"parse":         parse,
		"generate_seed": generateSeed,
		"wallet":        wallet,
	})
}
