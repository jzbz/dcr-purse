// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

package main

import (
	"bytes"
	"encoding/hex"
	"encoding/json"
	"os"
	"path/filepath"
	"testing"

	"github.com/decred/dcrd/chaincfg/v3"
	"github.com/decred/dcrd/dcrec/secp256k1/v4"
	"github.com/decred/dcrd/dcrutil/v4"
	"github.com/decred/dcrd/hdkeychain/v3"
	"github.com/decred/dcrd/txscript/v4/stdaddr"
)

// purse-udb's address manager is compared with this runner
// (crates/purse-udb/tests/oracle.rs). These tests pin the runner's own
// answers independently of the copied udb under internal/, so that it
// cannot drift on the paths the Rust differential relies on without a
// test here failing: a run is deterministic (creation, opening, the
// wallet lock, a reopen); the account 0 keys it reports after creation are
// the ones dcrd's hdkeychain derives on its own from the seed; and the
// address operations (SyncAccountToAddrIndex, Address, PrivateKey of a
// derived address and of an imported key, MarkUsed and the counters it
// moves, NewAccount's key) answer as dcrd's hdkeychain, stdaddr and
// secp256k1 compute them (TestAddrmgrAddressesAreDcrds). The other
// operations the differential compares (the coin type upgrade, imported
// xpub accounts and scripts, renames, lookups, Encrypt and Decrypt) are
// pinned only by udb itself and the frozen vectors, except the report the
// runner adds to the coin type upgrade for dcr-purse's
// (TestUpgradeReportsADroppedAccountPassphrase).

const addrmgrSliceScript = `{
  "format": "purse-opscript 1",
  "name": "address manager: creation, opening, the lock",
  "setup": {"network": "simnet", "seed": "b46bc6502a30beb92f0aebc776403c3dbf11bfb68305967c36dac9ef8d641567",
            "pubpass": "public", "privpass": "private",
            "rand": "0606060606060606060606060606060606060606060606060606060606060606", "clock": 1794990000},
  "ops": [
    {"op": "mgr.account_extended_pub_key", "account": 0},
    {"op": "mgr.account_extended_priv_key", "account": 0},
    {"op": "mgr.unlock", "pass": "private"},
    {"op": "mgr.account_extended_priv_key", "account": 0},
    {"op": "mgr.unlock", "pass": "wrong"},
    {"op": "mgr.is_locked"},
    {"op": "rt.reopen"},
    {"op": "mgr.coin_type"},
    {"op": "mgr.for_each_account"}
  ]
}`

func runAddrmgrSlice(t *testing.T) (*runResult, []byte, []byte) {
	t.Helper()
	var s Script
	if err := json.Unmarshal([]byte(addrmgrSliceScript), &s); err != nil {
		t.Fatal(err)
	}
	dir := filepath.Join(t.TempDir(), "run")
	res, err := run(&s, dir)
	if err != nil {
		t.Fatalf("run: %v", err)
	}
	results, err := os.ReadFile(filepath.Join(dir, "results.jsonl"))
	if err != nil {
		t.Fatal(err)
	}
	export, err := os.ReadFile(filepath.Join(dir, "final.pe"))
	if err != nil {
		t.Fatal(err)
	}
	return res, results, export
}

func TestAddrmgrSliceIsDeterministic(t *testing.T) {
	_, r1, e1 := runAddrmgrSlice(t)
	_, r2, e2 := runAddrmgrSlice(t)
	if !bytes.Equal(r1, r2) {
		t.Fatalf("results differ between two runs:\n%s\n%s", r1, r2)
	}
	if !bytes.Equal(e1, e2) {
		t.Fatal("final exports differ between two runs")
	}
}

func TestAddrmgrSliceAccount0KeysAreHdkeychains(t *testing.T) {
	res, _, _ := runAddrmgrSlice(t)
	seed, err := hex.DecodeString("b46bc6502a30beb92f0aebc776403c3dbf11bfb68305967c36dac9ef8d641567")
	if err != nil {
		t.Fatal(err)
	}
	params := chaincfg.SimNetParams()
	// m/44'/115'/0', each step with Child, as udb derives account 0 of
	// the legacy coin type (wallet/udb/addressmanager.go:2664-2706).
	key, err := hdkeychain.NewMaster(seed, params)
	if err != nil {
		t.Fatal(err)
	}
	for _, i := range []uint32{44, params.LegacyCoinType, 0} {
		if key, err = key.Child(i + hdkeychain.HardenedKeyStart); err != nil {
			t.Fatal(err)
		}
	}
	want := map[int]any{
		0: key.Neuter().String(),
		1: map[string]any{"kind": "Locked", "text": "wallet locked: unable to access account extended privkey"},
		3: key.String(),
		4: map[string]any{"kind": "Passphrase", "text": "invalid passphrase"},
		5: true,
		7: params.LegacyCoinType,
		8: []uint32{0, 2147483647},
	}
	for i, w := range want {
		rec := res.Results[i]
		got := rec.OK
		if rec.Err != nil {
			got = rec.Err
		}
		gb, err := marshalCanonical(got)
		if err != nil {
			t.Fatal(err)
		}
		wb, err := marshalCanonical(w)
		if err != nil {
			t.Fatal(err)
		}
		if !bytes.Equal(gb, wb) {
			t.Errorf("op %d (%s): got %s, want %s", i, rec.Op, gb, wb)
		}
	}
}

// The address operations purse-udb compares with this runner: their
// answers are pinned against dcrd's own hdkeychain, stdaddr and secp256k1,
// independently of the copied udb.
const addrmgrAddressScript = `{
  "format": "purse-opscript 1",
  "name": "address manager addresses",
  "setup": {"network": "simnet", "seed": "b46bc6502a30beb92f0aebc776403c3dbf11bfb68305967c36dac9ef8d641567",
            "pubpass": "public", "privpass": "private",
            "rand": "0707070707070707070707070707070707070707070707070707070707070707", "clock": 1794990000},
  "ops": [
    {"op": "mgr.sync_account_to_addr_index", "account": 0, "index": 2, "branch": 1},
    {"op": "mgr.address", "addr": {"acct": 0, "branch": 1, "index": 2}},
    {"op": "mgr.unlock", "pass": "private"},
    {"op": "mgr.private_key", "addr": {"acct": 0, "branch": 1, "index": 1}},
    {"op": "mgr.import_private_key", "key": "0000000000000000000000000000000000000000000000000000000000000001", "label": "g"},
    {"op": "mgr.private_key", "addr": {"ref": "g"}},
    {"op": "mgr.mark_used", "addr": {"acct": 0, "branch": 1, "index": 1}},
    {"op": "mgr.account_properties", "account": 0},
    {"op": "mgr.new_account", "name": "second"},
    {"op": "mgr.account_extended_pub_key", "account": 1}
  ]
}`

func TestAddrmgrAddressesAreDcrds(t *testing.T) {
	var s Script
	if err := json.Unmarshal([]byte(addrmgrAddressScript), &s); err != nil {
		t.Fatal(err)
	}
	res, err := run(&s, filepath.Join(t.TempDir(), "run"))
	if err != nil {
		t.Fatalf("run: %v", err)
	}
	seed, _ := hex.DecodeString("b46bc6502a30beb92f0aebc776403c3dbf11bfb68305967c36dac9ef8d641567")
	params := chaincfg.SimNetParams()
	// The legacy coin type key, as udb stores it (each step with Child),
	// then account 0 and its internal branch.
	coin, err := hdkeychain.NewMaster(seed, params)
	if err != nil {
		t.Fatal(err)
	}
	for _, i := range []uint32{44, params.LegacyCoinType} {
		if coin, err = coin.Child(i + hdkeychain.HardenedKeyStart); err != nil {
			t.Fatal(err)
		}
	}
	acct0, err := coin.Child(hdkeychain.HardenedKeyStart)
	if err != nil {
		t.Fatal(err)
	}
	internal, err := acct0.Child(1)
	if err != nil {
		t.Fatal(err)
	}
	child := func(i uint32) *hdkeychain.ExtendedKey {
		k, err := internal.Child(i)
		if err != nil {
			t.Fatal(err)
		}
		return k
	}
	c2 := child(2).SerializedPubKey()
	addr2, err := stdaddr.NewAddressPubKeyHashEcdsaSecp256k1V0(dcrutil.Hash160(c2), params)
	if err != nil {
		t.Fatal(err)
	}
	c1priv, err := child(1).SerializedPrivKey()
	if err != nil {
		t.Fatal(err)
	}
	k1 := secp256k1.PrivKeyFromBytes(c1priv)
	g := secp256k1.PrivKeyFromBytes([]byte{1})
	// NewAccount derives account 1 from the coin type key parsed from its
	// string (QK-0028).
	parsed, err := hdkeychain.NewKeyFromString(coin.String(), params)
	if err != nil {
		t.Fatal(err)
	}
	acct1, err := parsed.Child(1 + hdkeychain.HardenedKeyStart)
	if err != nil {
		t.Fatal(err)
	}
	want := map[int]any{
		1: map[string]any{"addr": addr2.String(), "account": 0, "addrhash": hex.EncodeToString(dcrutil.Hash160(c2)),
			"imported": false, "internal": true, "multisig": false, "kind": "pubkey",
			"pubkey": hex.EncodeToString(c2), "index": 2},
		3: map[string]any{"key": hex.EncodeToString(k1.Serialize()), "pubkey": hex.EncodeToString(k1.PubKey().SerializeCompressed())},
		5: map[string]any{"key": hex.EncodeToString(g.Serialize()), "pubkey": hex.EncodeToString(g.PubKey().SerializeCompressed())},
		7: map[string]any{"account": 0, "name": "default", "type": 1, "extused": uint32(0xFFFFFFFF), "intused": 1,
			"extret": uint32(0xFFFFFFFF), "intret": 1, "importedkeys": 0, "encrypted": false, "unlocked": false},
		8: 1,
		9: acct1.Neuter().String(),
	}
	for i, w := range want {
		rec := res.Results[i]
		got := rec.OK
		if rec.Err != nil {
			got = rec.Err
		}
		gb, err := marshalCanonical(got)
		if err != nil {
			t.Fatal(err)
		}
		wb, err := marshalCanonical(w)
		if err != nil {
			t.Fatal(err)
		}
		if !bytes.Equal(gb, wb) {
			t.Errorf("op %d (%s): got %s, want %s", i, rec.Op, gb, wb)
		}
	}
}

// TestUpgradeReportsADroppedAccountPassphrase pins the report the runner
// adds to mgr.upgrade_to_slip0044_coin_type for dcr-purse's (QK-0767):
// {"passphrase_dropped": true} exactly when account 0 has Argon2id
// parameters before the call, read from the bucket, and null otherwise,
// as for Go's call. Go's upgrade itself keeps the parameters, which the
// record of mgr.account_has_passphrase after it shows.
func TestUpgradeReportsADroppedAccountPassphrase(t *testing.T) {
	const script = `{
  "format": "purse-opscript 1",
  "name": "the upgrade's report",
  "setup": {"network": "simnet", "seed": "000102030405060708090a0b0c0d0e0f",
            "pubpass": "public", "privpass": "private",
            "rand": "6666666666666666666666666666666666666666666666666666666666666666", "clock": 1794990000},
  "ops": [
    {"op": "mgr.unlock", "pass": "private"},
    {"op": "mgr.set_account_passphrase", "account": 0, "pass": "zero"},
    {"op": "mgr.upgrade_to_slip0044_coin_type"},
    {"op": "mgr.account_has_passphrase", "account": 0},
    {"op": "mgr.upgrade_to_slip0044_coin_type"}
  ]
}`
	for _, removed := range []bool{false, true} {
		var s Script
		if err := json.Unmarshal([]byte(script), &s); err != nil {
			t.Fatal(err)
		}
		if removed {
			// Set and removed again: no parameters at the upgrade.
			extra := json.RawMessage(`{"op": "mgr.set_account_passphrase", "account": 0, "pass": ""}`)
			s.Ops = append(s.Ops[:2], append([]json.RawMessage{extra}, s.Ops[2:]...)...)
		}
		res, err := run(&s, filepath.Join(t.TempDir(), "run"))
		if err != nil {
			t.Fatalf("run: %v", err)
		}
		at := 2
		if removed {
			at = 3
		}
		up, err := json.Marshal(res.Results[at].OK)
		if err != nil {
			t.Fatal(err)
		}
		want := `{"passphrase_dropped":true}`
		if removed {
			want = "null"
		}
		if res.Results[at].Err != nil || string(up) != want {
			t.Errorf("removed %v: upgrade: ok %s, err %v; want ok %s", removed, up, res.Results[at].Err, want)
		}
		has, err := json.Marshal(res.Results[at+1].OK)
		if err != nil {
			t.Fatal(err)
		}
		wantHas := `{"has":true,"unlocked":true}`
		if removed {
			wantHas = `{"has":false,"unlocked":false}`
		}
		if string(has) != wantHas {
			t.Errorf("removed %v: AccountHasPassphrase after the upgrade: %s, want %s", removed, has, wantHas)
		}
		if res.Results[at+2].Err == nil {
			t.Errorf("removed %v: a second upgrade succeeded", removed)
		}
	}
}
