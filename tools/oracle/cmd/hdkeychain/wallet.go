// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

package main

import (
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"runtime"
	"strings"
	"sync"

	"decred.org/dcrwallet/v5/wallet/udb"
	"github.com/decred/dcrd/chaincfg/v3"
	"github.com/decred/dcrd/hdkeychain/v3"
)

// The wallet command derives, for each seed, what a dcrwallet created from
// that seed holds and shows: the account extended public keys of both coin
// types and the first address of both branches of each account, along the
// code paths dcrwallet itself takes.
//
//   - udb.HDKeysFromSeed (called by createAddressManager when a wallet is
//     created or restored) gives the legacy and SLIP0044 coin type keys and
//     account 0 of each, account 0 derived from the coin type key held in
//     memory, which hdkeychain may have stored stripped of leading zeros.
//   - Account 1 is derived as udb.Manager.NewAccount does (reached through
//     Wallet.NextAccount): from the coin type key re-parsed from its
//     serialized string (which pads the key back to 32 bytes), with
//     deriveAccountKey's Child(account + HardenedKeyStart).
//   - The account extended public key is the private account key neutered
//     and serialized, as stored; addresses come from it re-parsed (as loaded
//     from the database), through deriveBranches (Child of the external and
//     internal branch) and deriveChildAddress (Child, then compat.HD2Address,
//     copied verbatim in compat.go).
//
// One line per item:
//
//	C|<coin type>|<coin type key neutered String()>
//	A|<coin type>|<account>|<account xpub>|<branch 0 address 0>|<branch 1 address 0>
//	E|<coin type>|<account>|<error>       an account whose derivation failed
//	E|<error>                             HDKeysFromSeed failed (no other lines)
//
// for the legacy coin type, then the SLIP0044 coin type, each with its C line
// then accounts 0 and 1. An error is its Error() text; none that
// HDKeysFromSeed returns nests one dcrwallet error in another, so the text
// does not depend on errors.Separator.
//
// Request: {"net": "mainnet"|"testnet3"|"simnet"|"regnet", "digest": bool,
// "seeds": [hex]}. Answer: per seed, its lines, or with digest the hex
// SHA-256 of its lines, each followed by "\n".

// walletAccounts are the accounts derived for each coin type.
var walletAccounts = []uint32{0, 1}

func chainParams(name string) (*chaincfg.Params, error) {
	switch name {
	case "mainnet":
		return chaincfg.MainNetParams(), nil
	case "testnet3":
		return chaincfg.TestNet3Params(), nil
	case "simnet":
		return chaincfg.SimNetParams(), nil
	case "regnet":
		return chaincfg.RegNetParams(), nil
	}
	return nil, fmt.Errorf("unknown network %q", name)
}

// accountLine derives the account's extended public key and the first
// address of both branches from the account's private key, as the wallet
// stores and then loads it.
func accountLine(coinType, account uint32, acctPriv *hdkeychain.ExtendedKey,
	params *chaincfg.Params) string {

	fail := func(err error) string {
		return fmt.Sprintf("E|%d|%d|%s", coinType, account, err)
	}
	xpubString := acctPriv.Neuter().String()
	xpub, err := hdkeychain.NewKeyFromString(xpubString, params)
	if err != nil {
		return fail(err)
	}
	// wallet/addresses.go deriveBranches.
	extKey, err := xpub.Child(udb.ExternalBranch)
	if err != nil {
		return fail(err)
	}
	intKey, err := xpub.Child(udb.InternalBranch)
	if err != nil {
		return fail(err)
	}
	addrs := make([]string, 0, 2)
	for _, branch := range []*hdkeychain.ExtendedKey{extKey, intKey} {
		// wallet/addresses.go deriveChildAddress(branch, 0, params).
		child, err := branch.Child(0)
		if err != nil {
			return fail(err)
		}
		addr, err := HD2Address(child, params)
		if err != nil {
			return fail(err)
		}
		addrs = append(addrs, addr.String())
	}
	return fmt.Sprintf("A|%d|%d|%s|%s|%s", coinType, account, xpubString, addrs[0], addrs[1])
}

// walletLines is the answer for one seed.
func walletLines(seed []byte, params *chaincfg.Params) []string {
	ctLegacy, ctSLIP0044, acctLegacy, acctSLIP0044, err := udb.HDKeysFromSeed(seed, params)
	if err != nil {
		return []string{"E|" + err.Error()}
	}
	legacy, slip0044 := udb.CoinTypes(params)
	var lines []string
	for _, c := range []struct {
		coinType uint32
		ctKey    *hdkeychain.ExtendedKey
		acct0    *hdkeychain.ExtendedKey
	}{{legacy, ctLegacy, acctLegacy}, {slip0044, ctSLIP0044, acctSLIP0044}} {
		lines = append(lines, fmt.Sprintf("C|%d|%s", c.coinType, c.ctKey.Neuter().String()))
		for _, account := range walletAccounts {
			acctPriv := c.acct0
			if account != 0 {
				// udb.Manager.NewAccount: the stored coin type key, re-parsed.
				reparsed, err := hdkeychain.NewKeyFromString(c.ctKey.String(), params)
				if err != nil {
					lines = append(lines, fmt.Sprintf("E|%d|%d|%s", c.coinType, account, err))
					continue
				}
				// deriveAccountKey.
				acctPriv, err = reparsed.Child(account + hdkeychain.HardenedKeyStart)
				if err != nil {
					lines = append(lines, fmt.Sprintf("E|%d|%d|%s", c.coinType, account, err))
					continue
				}
			}
			lines = append(lines, accountLine(c.coinType, account, acctPriv, params))
		}
	}
	return lines
}

func walletDigest(lines []string) string {
	h := sha256.New()
	for _, l := range lines {
		h.Write([]byte(l))
		h.Write([]byte{'\n'})
	}
	return hex.EncodeToString(h.Sum(nil))
}

func wallet(raw json.RawMessage) (any, error) {
	var args struct {
		Net    string   `json:"net"`
		Digest bool     `json:"digest"`
		Seeds  []string `json:"seeds"`
	}
	if err := json.Unmarshal(raw, &args); err != nil {
		return nil, err
	}
	params, err := chainParams(args.Net)
	if err != nil {
		return nil, err
	}
	seeds := make([][]byte, len(args.Seeds))
	for i, s := range args.Seeds {
		if seeds[i], err = hex.DecodeString(s); err != nil {
			return nil, fmt.Errorf("seed %d: %v", i, err)
		}
	}

	results := make([]any, len(seeds))
	next := make(chan int)
	var wg sync.WaitGroup
	for w := 0; w < runtime.GOMAXPROCS(0); w++ {
		wg.Add(1)
		go func() {
			defer wg.Done()
			for i := range next {
				lines := walletLines(seeds[i], params)
				for _, l := range lines {
					if strings.ContainsRune(l, '\n') {
						panic(fmt.Sprintf("line with a newline: %q", l))
					}
				}
				if args.Digest {
					results[i] = walletDigest(lines)
				} else {
					results[i] = lines
				}
			}
		}()
	}
	for i := range seeds {
		next <- i
	}
	close(next)
	wg.Wait()
	return results, nil
}
