// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

// Command snacl is the oracle for crates/purse-snacl: dcrwallet's key sealing
// behind the linejson protocol. It links
//
//   - internal/snaclcopy, a verbatim copy of wallet/internal/snacl (an
//     internal package Go will not let this module import), for the scrypt
//     master keys and the XSalsa20-Poly1305 crypto-key boxes;
//   - decred.org/dcrwallet/v5/kdf itself, for the Argon2id parameters;
//   - sealcopy.go, a verbatim copy of wallet/udb's unexported argon2idKey,
//     seal and unseal, for the per-account XChaCha20-Poly1305 blobs;
//   - wallet/udb's public API, to write whole wallet databases.
//
// Every command answers {"result": outcome}, where outcome is one of
//
//	{"ok": {...}}
//	{"err": {"text": daemon, "text_lib": library, "kind": n, "op": s}}
//	{"panic": message}
//
// text is the error rendered with the daemon's errors.Separator ":: " (as
// dcrwallet's main sets it), text_lib with the package default ":\n\t"; kind
// is the numeric errors.Kind and op the errors.Op of the outermost
// *errors.Error (0 and "" for an error of another type). A panic is
// recovered and its value reported, so the Rust side can compare its error
// for inputs on which Go panics. Byte strings are lower-case hex.
//
// # Randomness
//
// snacl and seal draw from github.com/decred/dcrd/crypto/rand, a userspace
// ChaCha20 generator seeded from crypto/rand once, at package
// initialization, and again only every 20 seconds or 4 MiB. Replacing
// crypto/rand.Reader per request (as cmd/seed does) therefore cannot steer
// it. Every draw these functions make appears verbatim in their output,
// though: the salt in the marshalled parameters, the nonce at the front of a
// sealed blob, the generated key itself. The Rust side replays those bytes
// through its injectable source and must reproduce the output byte for byte
// and consume exactly them. kdf.NewArgon2idParams takes its reader as an
// argument, so argon2-new-params injects the request's bytes directly.
//
// # Commands
//
//	info                                 -> {num_cpu, go}
//	new-secret-key {pass, n, r, p}       -> {params, key}            snacl.NewSecretKey, Marshal
//	marshal {salt, digest, n, r, p}      -> {params}                 SecretKey.Marshal
//	unmarshal {data}                     -> {salt, digest, n, r, p}  SecretKey.Unmarshal
//	derive-key {params, pass}            -> {key}                    Unmarshal, DeriveKey
//	                                        (key_after: sk.Key after DeriveKey, on success and failure)
//	generate-crypto-key                  -> {key}                    snacl.GenerateCryptoKey
//	crypto-encrypt {key, plaintext}      -> {sealed}                 CryptoKey.Encrypt
//	secret-encrypt {key, plaintext}      -> {sealed}                 SecretKey.Encrypt
//	crypto-decrypt {key, sealed}         -> {plaintext}              CryptoKey.Decrypt
//	secret-decrypt {key, sealed}         -> {plaintext}              SecretKey.Decrypt
//	argon2-new-params {stream}           -> {params, served, num_cpu} kdf.NewArgon2idParams on a reader
//	                                        serving stream, then failing with "injected reader: exhausted";
//	                                        num_cpu is runtime.NumCPU(), which sets Threads
//	argon2-marshal {salt, time, memory, threads} -> {data}           Argon2idParams.MarshalBinary
//	argon2-unmarshal {data}              -> {salt, time, memory, threads}
//	argon2-derive {params, pass}         -> {key}                    UnmarshalBinary, kdf.DeriveKey(.., 32)
//	seal {key, plaintext}                -> {sealed}                 udb seal
//	unseal {key, sealed}                 -> {plaintext}              udb unseal
//	account-seal {params, pass, plaintext} -> {key, sealed}          argon2idKey, seal
//	account-unseal {params, pass, sealed}  -> {key, plaintext}       argon2idKey, unseal
//	make-db {...}                        -> see makeDB
//	open-db {...}                        -> see openDB
//
// Every request must finish within requestTimeout; the oracle exits if one
// does not (see withWatchdog).
//
// The salt drawn by a new-secret-key that fails is not visible; the Rust side
// replays any 32 bytes for it (Go draws the salt before running scrypt).
package main

import (
	"context"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"os"
	"runtime"
	"time"

	"decred.org/dcrwallet/v5/errors"
	"decred.org/dcrwallet/v5/kdf"
	_ "decred.org/dcrwallet/v5/wallet/drivers/bdb"
	"decred.org/dcrwallet/v5/wallet/udb"
	"decred.org/dcrwallet/v5/wallet/walletdb"
	"github.com/decred/dcrd/chaincfg/v3"
	"github.com/decred/dcrd/dcrec"
	"github.com/decred/dcrd/dcrutil/v4"
	"github.com/decred/dcrd/hdkeychain/v3"
	"github.com/jzbz/dcr-purse/tools/oracle/internal/linejson"
	snacl "github.com/jzbz/dcr-purse/tools/oracle/internal/snaclcopy"
)

const daemonSeparator = ":: "
const librarySeparator = ":\n\t"

type hexBytes []byte

func (h *hexBytes) UnmarshalJSON(b []byte) error {
	var s string
	if err := json.Unmarshal(b, &s); err != nil {
		return err
	}
	d, err := hex.DecodeString(s)
	if err != nil {
		return err
	}
	*h = d
	return nil
}

func hx(b []byte) string { return hex.EncodeToString(b) }

func ok(v map[string]any) map[string]any { return map[string]any{"ok": v} }

// errOutcome renders err with both separators and records the outermost
// *errors.Error's kind and op. extra members are merged in.
func errOutcome(err error, extra map[string]any) map[string]any {
	errors.Separator = librarySeparator
	lib := err.Error()
	errors.Separator = daemonSeparator
	text := err.Error()
	kind, op := 0, ""
	if e, isE := err.(*errors.Error); isE {
		kind, op = int(e.Kind), string(e.Op)
	}
	o := map[string]any{"text": text, "text_lib": lib, "kind": kind, "op": op}
	for k, v := range extra {
		o[k] = v
	}
	return map[string]any{"err": o}
}

// guard runs f, turning a panic into a {"panic": message} outcome.
func guard(f func() map[string]any) (out map[string]any) {
	defer func() {
		if r := recover(); r != nil {
			out = map[string]any{"panic": fmt.Sprint(r)}
		}
	}()
	return f()
}

func decode[T any](raw json.RawMessage) (T, error) {
	var args T
	if len(raw) == 0 {
		return args, nil
	}
	err := json.Unmarshal(raw, &args)
	return args, err
}

func handler[T any](f func(T) map[string]any) linejson.Handler {
	return func(raw json.RawMessage) (any, error) {
		args, err := decode[T](raw)
		if err != nil {
			return nil, err
		}
		return guard(func() map[string]any { return f(args) }), nil
	}
}

func cryptoKey(b []byte) (*snacl.CryptoKey, error) {
	if len(b) != snacl.KeySize {
		return nil, fmt.Errorf("key must be %d bytes, not %d", snacl.KeySize, len(b))
	}
	var k snacl.CryptoKey
	copy(k[:], b)
	return &k, nil
}

type keyArgs struct {
	Key       hexBytes `json:"key"`
	Plaintext hexBytes `json:"plaintext"`
	Sealed    hexBytes `json:"sealed"`
}

type paramArgs struct {
	Params    hexBytes `json:"params"`
	Pass      hexBytes `json:"pass"`
	Plaintext hexBytes `json:"plaintext"`
	Sealed    hexBytes `json:"sealed"`
}

// exhaustingReader serves its bytes, then fails.
type exhaustingReader struct {
	data   []byte
	served int
}

func (r *exhaustingReader) Read(p []byte) (int, error) {
	n := copy(p, r.data)
	r.data = r.data[n:]
	r.served += n
	if n < len(p) {
		return n, errors.New("injected reader: exhausted")
	}
	return n, nil
}

// requestTimeout bounds one request. Every request the tests send finishes
// in seconds (the heaviest are dcrwallet's real 256 MiB Argon2id cost and
// make-db); one that runs for longer means its inputs are wrong, such as a
// cost field built with a broken encoder, on which Go would otherwise hash
// for hours.
const requestTimeout = 120 * time.Second

// withWatchdog makes the oracle exit, naming the command and its arguments
// on stderr, when h runs for longer than requestTimeout. The Rust side then
// reads EOF and fails on that command instead of waiting.
func withWatchdog(name string, h linejson.Handler) linejson.Handler {
	return func(raw json.RawMessage) (any, error) {
		t := time.AfterFunc(requestTimeout, func() {
			fmt.Fprintf(os.Stderr, "snacl oracle: %s %s did not finish within %v; exiting\n",
				name, raw, requestTimeout)
			os.Exit(2)
		})
		defer t.Stop()
		return h(raw)
	}
}

func argon2Params(data []byte) (*kdf.Argon2idParams, error) {
	p := new(kdf.Argon2idParams)
	return p, p.UnmarshalBinary(data)
}

func main() {
	errors.Separator = daemonSeparator
	handlers := map[string]linejson.Handler{
		"info": func(json.RawMessage) (any, error) {
			return map[string]any{"num_cpu": runtime.NumCPU(), "go": runtime.Version()}, nil
		},
		"new-secret-key": handler(func(a struct {
			Pass hexBytes `json:"pass"`
			N    int      `json:"n"`
			R    int      `json:"r"`
			P    int      `json:"p"`
		}) map[string]any {
			pass := []byte(a.Pass)
			sk, err := snacl.NewSecretKey(&pass, a.N, a.R, a.P)
			if err != nil {
				return errOutcome(err, nil)
			}
			return ok(map[string]any{"params": hx(sk.Marshal()), "key": hx(sk.Key[:])})
		}),
		"marshal": handler(func(a struct {
			Salt   hexBytes `json:"salt"`
			Digest hexBytes `json:"digest"`
			N      int      `json:"n"`
			R      int      `json:"r"`
			P      int      `json:"p"`
		}) map[string]any {
			var sk snacl.SecretKey
			copy(sk.Parameters.Salt[:], a.Salt)
			copy(sk.Parameters.Digest[:], a.Digest)
			sk.Parameters.N, sk.Parameters.R, sk.Parameters.P = a.N, a.R, a.P
			return ok(map[string]any{"params": hx(sk.Marshal())})
		}),
		"unmarshal": handler(func(a struct {
			Data hexBytes `json:"data"`
		}) map[string]any {
			var sk snacl.SecretKey
			if err := sk.Unmarshal(a.Data); err != nil {
				return errOutcome(err, nil)
			}
			p := &sk.Parameters
			return ok(map[string]any{"salt": hx(p.Salt[:]), "digest": hx(p.Digest[:]),
				"n": p.N, "r": p.R, "p": p.P})
		}),
		"derive-key": handler(func(a paramArgs) map[string]any {
			var sk snacl.SecretKey
			if err := sk.Unmarshal(a.Params); err != nil {
				return errOutcome(err, nil)
			}
			pass := []byte(a.Pass)
			if err := sk.DeriveKey(&pass); err != nil {
				return errOutcome(err, map[string]any{"key_after": hx(sk.Key[:])})
			}
			return ok(map[string]any{"key": hx(sk.Key[:]), "key_after": hx(sk.Key[:])})
		}),
		"generate-crypto-key": handler(func(struct{}) map[string]any {
			k, err := snacl.GenerateCryptoKey()
			if err != nil {
				return errOutcome(err, nil)
			}
			return ok(map[string]any{"key": hx(k[:])})
		}),
		"crypto-encrypt": handler(func(a keyArgs) map[string]any {
			k, err := cryptoKey(a.Key)
			if err != nil {
				panic(err)
			}
			sealed, err := k.Encrypt(a.Plaintext)
			if err != nil {
				return errOutcome(err, nil)
			}
			return ok(map[string]any{"sealed": hx(sealed)})
		}),
		"secret-encrypt": handler(func(a keyArgs) map[string]any {
			k, err := cryptoKey(a.Key)
			if err != nil {
				panic(err)
			}
			sk := snacl.SecretKey{Key: k}
			sealed, err := sk.Encrypt(a.Plaintext)
			if err != nil {
				return errOutcome(err, nil)
			}
			return ok(map[string]any{"sealed": hx(sealed)})
		}),
		"crypto-decrypt": handler(func(a keyArgs) map[string]any {
			k, err := cryptoKey(a.Key)
			if err != nil {
				panic(err)
			}
			pt, err := k.Decrypt(a.Sealed)
			if err != nil {
				return errOutcome(err, nil)
			}
			return ok(map[string]any{"plaintext": hx(pt)})
		}),
		"secret-decrypt": handler(func(a keyArgs) map[string]any {
			k, err := cryptoKey(a.Key)
			if err != nil {
				panic(err)
			}
			sk := snacl.SecretKey{Key: k}
			pt, err := sk.Decrypt(a.Sealed)
			if err != nil {
				return errOutcome(err, nil)
			}
			return ok(map[string]any{"plaintext": hx(pt)})
		}),
		"argon2-new-params": handler(func(a struct {
			Stream hexBytes `json:"stream"`
		}) map[string]any {
			r := &exhaustingReader{data: a.Stream}
			p, err := kdf.NewArgon2idParams(r)
			ncpu := runtime.NumCPU()
			if err != nil {
				return errOutcome(err, map[string]any{"served": r.served, "num_cpu": ncpu})
			}
			b, _ := p.MarshalBinary()
			return ok(map[string]any{"params": hx(b), "served": r.served, "num_cpu": ncpu})
		}),
		"argon2-marshal": handler(func(a struct {
			Salt    hexBytes `json:"salt"`
			Time    uint32   `json:"time"`
			Memory  uint32   `json:"memory"`
			Threads uint8    `json:"threads"`
		}) map[string]any {
			p := kdf.Argon2idParams{Time: a.Time, Memory: a.Memory, Threads: a.Threads}
			copy(p.Salt[:], a.Salt)
			b, err := p.MarshalBinary()
			if err != nil {
				return errOutcome(err, nil)
			}
			return ok(map[string]any{"data": hx(b)})
		}),
		"argon2-unmarshal": handler(func(a struct {
			Data hexBytes `json:"data"`
		}) map[string]any {
			p, err := argon2Params(a.Data)
			if err != nil {
				return errOutcome(err, nil)
			}
			return ok(map[string]any{"salt": hx(p.Salt[:]), "time": p.Time,
				"memory": p.Memory, "threads": p.Threads})
		}),
		"argon2-derive": handler(func(a paramArgs) map[string]any {
			p, err := argon2Params(a.Params)
			if err != nil {
				return errOutcome(err, nil)
			}
			return ok(map[string]any{"key": hx(kdf.DeriveKey(a.Pass, p, 32))})
		}),
		"seal": handler(func(a keyArgs) map[string]any {
			sealed, err := seal(keyType(a.Key), a.Plaintext)
			if err != nil {
				return errOutcome(err, nil)
			}
			return ok(map[string]any{"sealed": hx(sealed)})
		}),
		"unseal": handler(func(a keyArgs) map[string]any {
			pt, err := unseal(keyType(a.Key), a.Sealed)
			if err != nil {
				return errOutcome(err, nil)
			}
			return ok(map[string]any{"plaintext": hx(pt)})
		}),
		"account-seal": handler(func(a paramArgs) map[string]any {
			p, err := argon2Params(a.Params)
			if err != nil {
				return errOutcome(err, nil)
			}
			key := argon2idKey(a.Pass, p)
			sealed, err := seal(key, a.Plaintext)
			if err != nil {
				return errOutcome(err, map[string]any{"key": hx(key)})
			}
			return ok(map[string]any{"key": hx(key), "sealed": hx(sealed)})
		}),
		"account-unseal": handler(func(a paramArgs) map[string]any {
			p, err := argon2Params(a.Params)
			if err != nil {
				return errOutcome(err, nil)
			}
			key := argon2idKey(a.Pass, p)
			pt, err := unseal(key, a.Sealed)
			if err != nil {
				return errOutcome(err, map[string]any{"key": hx(key)})
			}
			return ok(map[string]any{"key": hx(key), "plaintext": hx(pt)})
		}),
		"make-db": handler(makeDB),
		"open-db": handler(openDB),
	}
	for name, h := range handlers {
		handlers[name] = withWatchdog(name, h)
	}
	linejson.Serve(handlers)
}

type makeDBArgs struct {
	// Path is where the database is created; it must not exist.
	Path string `json:"path"`
	// Net is "mainnet", "testnet3", "simnet" or "regnet".
	Net        string   `json:"net"`
	Seed       hexBytes `json:"seed"`
	PubPass    hexBytes `json:"pub_pass"`
	PrivPass   hexBytes `json:"priv_pass"`
	WrongPass  hexBytes `json:"wrong_pass"`
	AcctPass   hexBytes `json:"acct_pass"`
	VotingSeed hexBytes `json:"voting_seed"`
	VotingPass hexBytes `json:"voting_pass"`
	// ImportKey is a 32-byte secp256k1 private key imported as a WIF.
	ImportKey hexBytes `json:"import_key"`
	// Script is imported as a script address.
	Script hexBytes `json:"script"`
}

func netParams(name string) (*chaincfg.Params, error) {
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

// makeDB writes a wallet database through udb's public API: Initialize (the
// version 1 layout plus every upgrade, so the current version), then, with
// the manager unlocked, a second BIP0044 account given its own passphrase by
// SetAccountPassphrase, an imported voting account (ImportVotingAccount, also
// sealed under its own passphrase), an imported private key and an imported
// script. It then closes the database and reopens it to record how the real
// API answers right and wrong passphrases.
//
// Result: {db_version, accounts: {"0": xpriv, "<unique>": xpriv, "<voting>": xpriv},
// unique_accounts: [unique, voting], voting_account, import_pubkey, import_privkey,
// script_hash160, open_wrong_pub, unlock_wrong_priv, unlock_account_wrong,
// unlock_voting_wrong, unlock_account_ok, unlock_voting_ok}. The *_wrong
// members are error outcomes (the "err" object), the *_ok members booleans.
func makeDB(a makeDBArgs) map[string]any {
	ctx := context.Background()
	params, err := netParams(a.Net)
	if err != nil {
		return errOutcome(err, nil)
	}
	if _, err := os.Stat(a.Path); err == nil {
		return errOutcome(fmt.Errorf("%s exists", a.Path), nil)
	}
	db, err := walletdb.Create("bdb", a.Path)
	if err != nil {
		return errOutcome(err, nil)
	}
	closed := false
	defer func() {
		if !closed {
			db.Close()
		}
	}()
	if err := udb.Initialize(ctx, db, params, a.Seed, a.PubPass, a.PrivPass); err != nil {
		return errOutcome(err, nil)
	}
	amgr, _, err := udb.Open(ctx, db, params, a.PubPass)
	if err != nil {
		return errOutcome(err, nil)
	}
	accounts := map[string]string{}
	var acct1, votingAccount uint32
	var importPub []byte
	// ImportPrivateKey zeroes the private key bytes the WIF holds, which
	// alias the request's, so they are recorded first.
	importPriv := hx(a.ImportKey)
	err = walletdb.Update(ctx, db, func(tx walletdb.ReadWriteTx) error {
		ns := tx.ReadWriteBucket([]byte("waddrmgr"))
		if err := amgr.Unlock(ns, a.PrivPass); err != nil {
			return err
		}
		var err error
		acct1, err = amgr.NewAccount(ns, "unique")
		if err != nil {
			return err
		}
		for _, acct := range []uint32{0, acct1} {
			xpriv, err := amgr.AccountExtendedPrivKey(tx, acct)
			if err != nil {
				return err
			}
			accounts[fmt.Sprint(acct)] = xpriv.String()
		}
		if err := amgr.SetAccountPassphrase(tx, acct1, a.AcctPass); err != nil {
			return err
		}
		master, err := hdkeychain.NewMaster(a.VotingSeed, params)
		if err != nil {
			return err
		}
		votingAccount, err = amgr.ImportVotingAccount(tx, master, a.VotingPass, "voting")
		if err != nil {
			return err
		}
		accounts[fmt.Sprint(votingAccount)] = master.String()
		wif, err := dcrutil.NewWIF(a.ImportKey, params.PrivateKeyID, dcrec.STEcdsaSecp256k1)
		if err != nil {
			return err
		}
		importPub = wif.PubKey()
		if _, err := amgr.ImportPrivateKey(ns, wif); err != nil {
			return err
		}
		if _, err := amgr.ImportScript(ns, a.Script); err != nil {
			return err
		}
		return amgr.Lock()
	})
	if err != nil {
		return errOutcome(err, nil)
	}
	dbVersion, err := dbVersionOf(ctx, db)
	if err != nil {
		return errOutcome(err, nil)
	}
	db.Close()
	closed = true

	// Reopen and record the API's answers.
	db, err = walletdb.Open("bdb", a.Path)
	if err != nil {
		return errOutcome(err, nil)
	}
	defer db.Close()
	result := map[string]any{
		"db_version":      dbVersion,
		"accounts":        accounts,
		"unique_accounts": []uint32{acct1, votingAccount},
		"voting_account":  votingAccount,
		"import_pubkey":   hx(importPub),
		"import_privkey":  importPriv,
		"script_hash160":  hx(dcrutil.Hash160(a.Script)),
	}
	if _, err := passphraseAnswers(ctx, db, params, a, acct1, votingAccount, result); err != nil {
		return errOutcome(err, nil)
	}
	return ok(result)
}

// passphraseAnswers records in result how udb's API answers right and wrong
// passphrases on an open database: udb.Open with the wrong public passphrase
// (open_wrong_pub), then, opened with the right one, Manager.Unlock with the
// wrong private passphrase and UnlockAccount with the wrong and the right
// account passphrases. It returns the manager, with both accounts unlocked.
func passphraseAnswers(ctx context.Context, db walletdb.DB, params *chaincfg.Params,
	a makeDBArgs, acct, voting uint32, result map[string]any) (*udb.Manager, error) {

	inner := func(err error) any {
		if err == nil {
			return nil
		}
		return errOutcome(err, nil)["err"]
	}
	_, _, wrongPubErr := udb.Open(ctx, db, params, a.WrongPass)
	result["open_wrong_pub"] = inner(wrongPubErr)
	amgr, _, err := udb.Open(ctx, db, params, a.PubPass)
	if err != nil {
		return nil, err
	}
	err = walletdb.View(ctx, db, func(tx walletdb.ReadTx) error {
		ns := tx.ReadBucket([]byte("waddrmgr"))
		result["unlock_wrong_priv"] = inner(amgr.Unlock(ns, a.WrongPass))
		result["unlock_account_wrong"] = inner(amgr.UnlockAccount(tx, acct, a.WrongPass))
		result["unlock_voting_wrong"] = inner(amgr.UnlockAccount(tx, voting, a.WrongPass))
		result["unlock_account_ok"] = amgr.UnlockAccount(tx, acct, a.AcctPass) == nil
		result["unlock_voting_ok"] = amgr.UnlockAccount(tx, voting, a.VotingPass) == nil
		return nil
	})
	return amgr, err
}

// dbVersionOf reads meta/ver, the database version.
func dbVersionOf(ctx context.Context, db walletdb.DB) (uint32, error) {
	var version uint32
	err := walletdb.View(ctx, db, func(tx walletdb.ReadTx) error {
		v := tx.ReadBucket([]byte("meta")).Get([]byte("ver"))
		if len(v) != 4 {
			return fmt.Errorf("meta/ver is %d bytes", len(v))
		}
		version = uint32(v[0])<<24 | uint32(v[1])<<16 | uint32(v[2])<<8 | uint32(v[3])
		return nil
	})
	return version, err
}

// openDB opens an existing database that make-db wrote (a copy: opening it
// may write to it) through udb's public API and answers with make-db's
// result shape, every member read from the database rather than recorded at
// creation: the version; the accounts named "unique" and "voting"; account
// 0's extended private key after Manager.Unlock with the private
// passphrase, and the other two's after UnlockAccount with their own; the
// imported key's public key (decrypted with the public crypto key) and
// private key (Manager.PrivateKey); the imported script's hash160; and
// passphraseAnswers. So a frozen database is checked against Go without
// being regenerated. Only the members path, net and the five passphrases of
// the request are read.
func openDB(a makeDBArgs) map[string]any {
	ctx := context.Background()
	params, err := netParams(a.Net)
	if err != nil {
		return errOutcome(err, nil)
	}
	db, err := walletdb.Open("bdb", a.Path)
	if err != nil {
		return errOutcome(err, nil)
	}
	defer db.Close()
	dbVersion, err := dbVersionOf(ctx, db)
	if err != nil {
		return errOutcome(err, nil)
	}
	amgr, _, err := udb.Open(ctx, db, params, a.PubPass)
	if err != nil {
		return errOutcome(err, nil)
	}
	var acct, voting uint32
	err = walletdb.View(ctx, db, func(tx walletdb.ReadTx) error {
		ns := tx.ReadBucket([]byte("waddrmgr"))
		var err error
		if acct, err = amgr.LookupAccount(ns, "unique"); err != nil {
			return err
		}
		voting, err = amgr.LookupAccount(ns, "voting")
		return err
	})
	if err != nil {
		return errOutcome(err, nil)
	}
	result := map[string]any{
		"db_version":      dbVersion,
		"unique_accounts": []uint32{acct, voting},
		"voting_account":  voting,
	}
	amgr, err = passphraseAnswers(ctx, db, params, a, acct, voting, result)
	if err != nil {
		return errOutcome(err, nil)
	}
	accounts := map[string]string{}
	var importPub, importPriv, scriptHash []byte
	err = walletdb.View(ctx, db, func(tx walletdb.ReadTx) error {
		ns := tx.ReadBucket([]byte("waddrmgr"))
		if err := amgr.Unlock(ns, a.PrivPass); err != nil {
			return err
		}
		for _, n := range []uint32{0, acct, voting} {
			xpriv, err := amgr.AccountExtendedPrivKey(tx, n)
			if err != nil {
				return err
			}
			accounts[fmt.Sprint(n)] = xpriv.String()
		}
		// ForEachAccountAddress holds the manager's lock while it calls
		// back, and PrivateKey takes it, so the key is fetched afterwards.
		var imported udb.ManagedPubKeyAddress
		err := amgr.ForEachAccountAddress(ns, udb.ImportedAddrAccount, func(ma udb.ManagedAddress) error {
			switch ma := ma.(type) {
			case udb.ManagedPubKeyAddress:
				if imported != nil {
					return fmt.Errorf("more than one imported key")
				}
				imported = ma
			case udb.ManagedScriptAddress:
				if scriptHash != nil {
					return fmt.Errorf("more than one imported script")
				}
				scriptHash = ma.AddrHash()
			default:
				return fmt.Errorf("imported address of type %T", ma)
			}
			return nil
		})
		if err != nil {
			return err
		}
		if imported == nil {
			return fmt.Errorf("no imported key")
		}
		importPub = imported.PubKey()
		key, done, err := amgr.PrivateKey(ns, imported.Address())
		if err != nil {
			return err
		}
		importPriv = key.Serialize()
		done()
		return nil
	})
	if err != nil {
		return errOutcome(err, nil)
	}
	if err := amgr.Lock(); err != nil {
		return errOutcome(err, nil)
	}
	result["accounts"] = accounts
	result["import_pubkey"] = hx(importPub)
	result["import_privkey"] = hx(importPriv)
	result["script_hash160"] = hx(scriptHash)
	return ok(result)
}
