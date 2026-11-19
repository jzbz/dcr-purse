// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

// Command walletgen writes dcrwallet databases at schema version 26 for the
// logical-export checks (docs/adr/0001-logical-export-format.md), every
// byte written by dcrwallet's own wallet/udb at the parity pin except the
// fault injections of the mutate command. It speaks the linejson protocol.
//
// Commands:
//
//	upgrade {gz, out, network?, pubpass?}
//	        Decompresses the gzipped fixture gz (one of upstream's
//	        wallet/udb/testdata/v*.db.gz) to out, which must not exist, and
//	        runs udb.Upgrade on the copy, as upstream's TestUpgrades does
//	        (network testnet3 and public passphrase "public" by default,
//	        wallet/udb/upgrades_test.go). Returns {from, to}.
//	create  {path, network, seed, pubpass, privpass, scenario, time,
//	        chain_time?}
//	        Creates path (which must not exist) with udb.Initialize from the
//	        hex seed and the passphrases, then applies the scenario: "empty"
//	        (nothing more) or "full" (see scenario.go). time is the Unix time
//	        the injected wallet clock starts at (transaction received times
//	        and the birthday time derive from it); chain_time, by default
//	        equal to time, is the base of the block timestamps, which are
//	        chain data rather than clock readings. Returns {accounts,
//	        blocks, tip, txs}.
//	reopen  {path, network, pubpass}
//	        Opens path as dcrwallet does at start-up (walletdb.Open with the
//	        bdb driver, then udb.Open) and closes it. Returns {}.
//	mutate  {path, ops: [{op, path, key, value}]}
//	        Fault injection for the export's anomaly sections ([orphans],
//	        [bad], [vsp] future IDs, [residue], [privcheck] verdicts):
//	        applies raw bbolt writes, in one transaction, to a database
//	        this command set created. op is "put" (key and value), "delete"
//	        (key), "bucket" (create the nested bucket key if missing) or
//	        "deletebucket" (key); path is the bucket path, every component
//	        and the key and value in hex. These are the only bytes of the
//	        generated databases that udb did not write. Returns {applied}.
//
// What cannot be injected. udb draws salts, nonces and generated keys from
// github.com/decred/dcrd/crypto/rand, a generator seeded from the operating
// system at package initialization (see cmd/snacl), and stamps address rows
// and the two creation dates with time.Now() inside udb
// (wallet/udb/addressdb.go:1360, 1375, 1390, 1683; txdb.go:2181). Neither has
// a seam in udb's public API, so two runs with equal arguments differ in
// exactly those fields, which the export format marks volatile. The Argon2id
// thread count is runtime.NumCPU() (kdf/kdf.go:31), so it depends on the
// host.
package main

import (
	"compress/gzip"
	"context"
	"encoding/binary"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"os"
	"time"

	"decred.org/dcrwallet/v5/wallet/udb"
	"decred.org/dcrwallet/v5/wallet/walletdb"
	"github.com/jzbz/dcr-purse/tools/oracle/internal/linejson"
	bolt "go.etcd.io/bbolt"

	_ "decred.org/dcrwallet/v5/wallet/drivers/bdb"
	"github.com/decred/dcrd/chaincfg/v3"
)

func main() {
	linejson.Serve(map[string]linejson.Handler{
		"upgrade": cmdUpgrade,
		"create":  cmdCreate,
		"reopen":  cmdReopen,
		"mutate":  cmdMutate,
	})
}

func unmarshal(args json.RawMessage, v any) error {
	if len(args) == 0 {
		return errors.New("missing args")
	}
	return json.Unmarshal(args, v)
}

func netParams(name string) (*chaincfg.Params, error) {
	switch name {
	case "mainnet":
		return chaincfg.MainNetParams(), nil
	case "testnet3", "":
		return chaincfg.TestNet3Params(), nil
	case "simnet":
		return chaincfg.SimNetParams(), nil
	case "regnet":
		return chaincfg.RegNetParams(), nil
	}
	return nil, fmt.Errorf("unknown network %q", name)
}

func dbVersion(ctx context.Context, db walletdb.DB) (uint32, error) {
	var v uint32
	err := walletdb.View(ctx, db, func(tx walletdb.ReadTx) error {
		meta := tx.ReadBucket([]byte("meta"))
		if meta == nil {
			return nil // pre-unified
		}
		b := meta.Get([]byte("ver"))
		if len(b) != 4 {
			return fmt.Errorf("meta/ver has %d bytes", len(b))
		}
		v = binary.BigEndian.Uint32(b)
		return nil
	})
	return v, err
}

type upgradeArgs struct {
	GZ      string  `json:"gz"`
	Out     string  `json:"out"`
	Network string  `json:"network"`
	PubPass *string `json:"pubpass"`
}

func cmdUpgrade(raw json.RawMessage) (any, error) {
	var args upgradeArgs
	if err := unmarshal(raw, &args); err != nil {
		return nil, err
	}
	params, err := netParams(args.Network)
	if err != nil {
		return nil, err
	}
	pub := []byte("public")
	if args.PubPass != nil {
		pub = []byte(*args.PubPass)
	}
	if _, err := os.Stat(args.Out); err == nil {
		return nil, fmt.Errorf("%s already exists", args.Out)
	}
	in, err := os.Open(args.GZ)
	if err != nil {
		return nil, err
	}
	defer in.Close()
	zr, err := gzip.NewReader(in)
	if err != nil {
		return nil, err
	}
	out, err := os.OpenFile(args.Out, os.O_CREATE|os.O_EXCL|os.O_WRONLY, 0600)
	if err != nil {
		return nil, err
	}
	if _, err := io.Copy(out, zr); err != nil {
		out.Close()
		return nil, err
	}
	if err := out.Close(); err != nil {
		return nil, err
	}

	ctx := context.Background()
	db, err := walletdb.Open("bdb", args.Out)
	if err != nil {
		return nil, err
	}
	defer db.Close()
	from, err := dbVersion(ctx, db)
	if err != nil {
		return nil, err
	}
	if err := udb.Upgrade(ctx, db, pub, params); err != nil {
		return nil, fmt.Errorf("udb.Upgrade: %w", err)
	}
	to, err := dbVersion(ctx, db)
	if err != nil {
		return nil, err
	}
	return map[string]uint32{"from": from, "to": to}, nil
}

type createArgs struct {
	Path     string `json:"path"`
	Network  string `json:"network"`
	Seed     string `json:"seed"`
	PubPass  string `json:"pubpass"`
	PrivPass string `json:"privpass"`
	Scenario string `json:"scenario"`
	Time     int64  `json:"time"`
	// ChainTime is the timestamp base of the scenario's blocks (default:
	// Time). Block timestamps are chain data, not wallet clock readings.
	ChainTime int64 `json:"chain_time"`
}

type createResult struct {
	Accounts []uint32 `json:"accounts"`
	Blocks   int32    `json:"blocks"`
	Tip      string   `json:"tip"`
	Txs      int      `json:"txs"`
}

func cmdCreate(raw json.RawMessage) (any, error) {
	var args createArgs
	if err := unmarshal(raw, &args); err != nil {
		return nil, err
	}
	params, err := netParams(args.Network)
	if err != nil {
		return nil, err
	}
	seed, err := hex.DecodeString(args.Seed)
	if err != nil {
		return nil, fmt.Errorf("seed: %w", err)
	}
	if _, err := os.Stat(args.Path); err == nil {
		return nil, fmt.Errorf("%s already exists", args.Path)
	}
	ctx := context.Background()
	db, err := walletdb.Create("bdb", args.Path)
	if err != nil {
		return nil, err
	}
	defer db.Close()
	pub, priv := []byte(args.PubPass), []byte(args.PrivPass)
	if err := udb.Initialize(ctx, db, params, seed, pub, priv); err != nil {
		return nil, fmt.Errorf("udb.Initialize: %w", err)
	}
	mgr, store, err := udb.Open(ctx, db, params, pub)
	if err != nil {
		return nil, fmt.Errorf("udb.Open: %w", err)
	}
	defer mgr.Close()

	s := &scenario{
		ctx: ctx, db: db, params: params, mgr: mgr, store: store,
		priv: priv, clock: time.Unix(args.Time, 0), chainClock: time.Unix(args.Time, 0),
	}
	if args.ChainTime != 0 {
		s.chainClock = time.Unix(args.ChainTime, 0)
	}
	switch args.Scenario {
	case "", "empty":
	case "full":
		if err := s.full(); err != nil {
			return nil, err
		}
	default:
		return nil, fmt.Errorf("unknown scenario %q", args.Scenario)
	}

	var res createResult
	err = walletdb.View(ctx, db, func(tx walletdb.ReadTx) error {
		ns := tx.ReadBucket([]byte("waddrmgr"))
		if err := mgr.ForEachAccount(ns, func(a uint32) error {
			res.Accounts = append(res.Accounts, a)
			return nil
		}); err != nil {
			return err
		}
		h, height := store.MainChainTip(tx)
		res.Tip, res.Blocks = h.String(), height
		return nil
	})
	res.Txs = s.txs
	return res, err
}

type reopenArgs struct {
	Path    string `json:"path"`
	Network string `json:"network"`
	PubPass string `json:"pubpass"`
}

func cmdReopen(raw json.RawMessage) (any, error) {
	var args reopenArgs
	if err := unmarshal(raw, &args); err != nil {
		return nil, err
	}
	params, err := netParams(args.Network)
	if err != nil {
		return nil, err
	}
	ctx := context.Background()
	db, err := walletdb.Open("bdb", args.Path)
	if err != nil {
		return nil, err
	}
	defer db.Close()
	mgr, _, err := udb.Open(ctx, db, params, []byte(args.PubPass))
	if err != nil {
		return nil, err
	}
	return struct{}{}, mgr.Close()
}

type mutateOp struct {
	Op    string   `json:"op"`
	Path  []string `json:"path"`
	Key   string   `json:"key"`
	Value string   `json:"value"`
}

type mutateArgs struct {
	Path string     `json:"path"`
	Ops  []mutateOp `json:"ops"`
}

func cmdMutate(raw json.RawMessage) (any, error) {
	var args mutateArgs
	if err := unmarshal(raw, &args); err != nil {
		return nil, err
	}
	if _, err := os.Stat(args.Path); err != nil {
		return nil, err
	}
	db, err := bolt.Open(args.Path, 0600, &bolt.Options{Timeout: 2 * time.Second})
	if err != nil {
		return nil, err
	}
	defer db.Close()
	err = db.Update(func(tx *bolt.Tx) error {
		for i, op := range args.Ops {
			if len(op.Path) == 0 {
				return fmt.Errorf("op %d: empty bucket path", i)
			}
			var b *bolt.Bucket
			for j, c := range op.Path {
				name, err := hex.DecodeString(c)
				if err != nil {
					return fmt.Errorf("op %d: path: %w", i, err)
				}
				if j == 0 {
					b = tx.Bucket(name)
				} else {
					b = b.Bucket(name)
				}
				if b == nil {
					return fmt.Errorf("op %d: no bucket %s", i, c)
				}
			}
			key, err := hex.DecodeString(op.Key)
			if err != nil || len(key) == 0 {
				return fmt.Errorf("op %d: bad key %q", i, op.Key)
			}
			switch op.Op {
			case "put":
				value, err := hex.DecodeString(op.Value)
				if err != nil {
					return fmt.Errorf("op %d: value: %w", i, err)
				}
				err = b.Put(key, value)
				if err != nil {
					return fmt.Errorf("op %d: %w", i, err)
				}
			case "delete":
				if err := b.Delete(key); err != nil {
					return fmt.Errorf("op %d: %w", i, err)
				}
			case "bucket":
				if _, err := b.CreateBucketIfNotExists(key); err != nil {
					return fmt.Errorf("op %d: %w", i, err)
				}
			case "deletebucket":
				if err := b.DeleteBucket(key); err != nil {
					return fmt.Errorf("op %d: %w", i, err)
				}
			default:
				return fmt.Errorf("op %d: unknown op %q", i, op.Op)
			}
		}
		return nil
	})
	if err != nil {
		return nil, err
	}
	return map[string]int{"applied": len(args.Ops)}, nil
}
