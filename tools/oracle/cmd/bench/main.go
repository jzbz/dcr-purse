// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

// Command bench is the Go side of crates/purse-bench (ADR-0014, section 4):
// the same workloads on the same inputs, through the packages dcrwallet
// links at the parity pin (dcrd hdkeychain v3.1.3, txscript/v4 stdaddr,
// decred/base58, dcrutil/v4, dcrwallet's kdf and, through the verbatim copy
// internal/snaclcopy, wallet/internal/snacl; the mixing root solver
// decred.org/cspp/v2 over FLINT when built with the tag "flint").
//
// It is not a linejson oracle: it runs every selected workload once and
// prints tab-separated lines, the format purse-bench's driver reads:
//
//	META    go  <key>  <value>
//	CHECK   go  <name> <value>     an output of the workload, which must
//	                               equal the Rust side's
//	RESULT  go  <name> <median ns/op> <min> <max> <ops/rep> <reps> <every rep>
//	SKIP    go  <name> <reason>
//
// Flags: -reps (repetitions per workload), -ms (target milliseconds per
// repetition of a calibrated workload), -only (comma-separated substrings
// of the workload names to run), -smoke (one short repetition each and the
// reduced sizes listed in purse-bench's documentation: a check that the
// workloads run and agree, not a measurement).
//
// Inputs are fixed: the seed is the bytes 0, 1, ..., 31, and every other
// input comes from SplitMix64 with a seed per workload, the same generator
// purse-testutil implements. The workload list, its names and its inputs
// are kept equal to crates/purse-bench/src/workloads.rs by hand; the
// driver refuses a session in which a CHECK differs between the sides.
package main

import (
	"bytes"
	"crypto/hmac"
	"crypto/sha256"
	"crypto/sha512"
	"encoding/binary"
	"encoding/hex"
	"flag"
	"fmt"
	"runtime"
	"runtime/debug"
	"sort"
	"strings"
	"time"

	"decred.org/dcrwallet/v5/kdf"
	"github.com/decred/base58"
	"github.com/decred/dcrd/chaincfg/v3"
	"github.com/decred/dcrd/dcrutil/v4"
	"github.com/decred/dcrd/hdkeychain/v3"
	"github.com/decred/dcrd/txscript/v4/stdaddr"
	"github.com/jzbz/dcr-purse/tools/oracle/internal/snaclcopy"
	"golang.org/x/crypto/nacl/secretbox"
	"golang.org/x/crypto/scrypt"
)

// sink keeps every workload's result alive.
var sink uint64

// config is the harness configuration from the flags.
type config struct {
	reps  int
	ms    float64
	only  []string
	smoke bool
}

var cfg config

func selected(name string) bool {
	if len(cfg.only) == 0 {
		return true
	}
	for _, s := range cfg.only {
		if strings.Contains(name, s) {
			return true
		}
	}
	return false
}

// anySelected reports whether any of the workloads names runs: a group
// whose workloads are all left out skips its setup and its checks.
func anySelected(names ...string) bool {
	for _, n := range names {
		if selected(n) {
			return true
		}
	}
	return false
}

// bench times f(n), which runs n operations: it doubles n until one call
// takes 25 ms (the warm-up and calibration), then times cfg.reps calls of
// the size that takes about cfg.ms.
func bench(name string, f func(n uint64) uint64) {
	if !selected(name) {
		return
	}
	iters := uint64(1)
	if !cfg.smoke {
		n := uint64(1)
		var t float64
		for {
			s := time.Now()
			sink ^= f(n)
			t = float64(time.Since(s).Nanoseconds())
			if t >= 25e6 || n >= 1<<40 {
				break
			}
			n *= 2
		}
		iters = uint64(cfg.ms * 1e6 / (t / float64(n)))
		if iters < 1 {
			iters = 1
		}
	} else {
		sink ^= f(1)
	}
	v := make([]float64, 0, cfg.reps)
	for r := 0; r < cfg.reps; r++ {
		s := time.Now()
		sink ^= f(iters)
		v = append(v, float64(time.Since(s).Nanoseconds())/float64(iters))
	}
	report(name, v, iters)
}

// benchOnce times whole operations too slow to calibrate: one warm-up
// call, then cfg.reps timed calls of one operation each.
func benchOnce(name string, f func() uint64) {
	if !selected(name) {
		return
	}
	sink ^= f()
	v := make([]float64, 0, cfg.reps)
	for r := 0; r < cfg.reps; r++ {
		s := time.Now()
		sink ^= f()
		v = append(v, float64(time.Since(s).Nanoseconds()))
	}
	report(name, v, 1)
}

func report(name string, v []float64, iters uint64) {
	s := append([]float64(nil), v...)
	sort.Float64s(s)
	var med float64
	if len(s)%2 == 1 {
		med = s[len(s)/2]
	} else {
		med = (s[len(s)/2-1] + s[len(s)/2]) / 2
	}
	all := make([]string, len(v))
	for i, x := range v {
		all[i] = fmt.Sprintf("%.1f", x)
	}
	fmt.Printf("RESULT\tgo\t%s\t%.1f\t%.1f\t%.1f\t%d\t%d\t%s\n",
		name, med, s[0], s[len(s)-1], iters, len(v), strings.Join(all, ","))
}

func check(name string, v any) {
	if selected(name) || selectedPrefix(name) {
		fmt.Printf("CHECK\tgo\t%s\t%v\n", name, v)
	}
}

// selectedPrefix reports whether any selected workload shares the group
// (the part before the first dot) of a check's name, so that a run limited
// with -only still prints the checks of its group.
func selectedPrefix(name string) bool {
	group, _, _ := strings.Cut(name, ".")
	for _, s := range cfg.only {
		if strings.HasPrefix(s, group+".") || s == group {
			return true
		}
	}
	return false
}

func skip(name, why string) {
	if selected(name) {
		fmt.Printf("SKIP\tgo\t%s\t%s\n", name, why)
	}
}

func meta(k string, v any) { fmt.Printf("META\tgo\t%s\t%v\n", k, v) }

func must(err error) {
	if err != nil {
		panic(err)
	}
}

// splitMix is SplitMix64, as purse-testutil's SplitMix64.
type splitMix uint64

func (s *splitMix) next() uint64 {
	*s += 0x9e3779b97f4a7c15
	z := uint64(*s)
	z = (z ^ (z >> 30)) * 0xbf58476d1ce4e5b9
	z = (z ^ (z >> 27)) * 0x94d049bb133111eb
	return z ^ (z >> 31)
}

func (s *splitMix) fill(b []byte) {
	for i := 0; i < len(b); i += 8 {
		var x [8]byte
		binary.LittleEndian.PutUint64(x[:], s.next())
		copy(b[i:], x[:])
	}
}

func (s *splitMix) bytes(n int) []byte {
	b := make([]byte, n)
	s.fill(b)
	return b
}

func sha256Lines(lines []string) string {
	h := sha256.New()
	for _, l := range lines {
		h.Write([]byte(l))
		h.Write([]byte{'\n'})
	}
	return hex.EncodeToString(h.Sum(nil))
}

func main() {
	reps := flag.Int("reps", 5, "repetitions per workload")
	ms := flag.Float64("ms", 200, "target milliseconds per repetition of a calibrated workload")
	only := flag.String("only", "", "comma-separated substrings of the workload names to run")
	smoke := flag.Bool("smoke", false, "one short repetition each, reduced sizes")
	flag.Parse()
	cfg = config{reps: *reps, ms: *ms, smoke: *smoke}
	if *only != "" {
		cfg.only = strings.Split(*only, ",")
	}
	if cfg.smoke {
		cfg.reps = 1
	}
	if cfg.reps < 1 {
		cfg.reps = 1
	}

	meta("version", runtime.Version())
	meta("goos", runtime.GOOS+"/"+runtime.GOARCH)
	meta("gomaxprocs", runtime.GOMAXPROCS(0))
	meta("num_cpu", runtime.NumCPU())
	if bi, ok := debug.ReadBuildInfo(); ok {
		for _, d := range bi.Deps {
			switch d.Path {
			case "decred.org/dcrwallet/v5", "github.com/decred/dcrd/hdkeychain/v3",
				"github.com/decred/dcrd/dcrec/secp256k1/v4", "github.com/decred/dcrd/txscript/v4",
				"github.com/decred/base58", "golang.org/x/crypto", "decred.org/cspp/v2":
				meta("module", d.Path+"@"+d.Version)
			}
		}
	}
	mode := "measure"
	if cfg.smoke {
		mode = "smoke"
	}
	meta("mode", mode)
	meta("reps", cfg.reps)
	meta("ms", cfg.ms)

	hd()
	primitives()
	snaclWorkloads()
	kdfWorkloads()
	amounts()
	mix()
	meta("sink", sink&1)
}

// sizes the smoke mode reduces.
func addrCount() uint32 {
	if cfg.smoke {
		return 200
	}
	return 20000
}

const hardened = hdkeychain.HardenedKeyStart

func p2pkh(pub []byte, params *chaincfg.Params) stdaddr.Address {
	a, err := stdaddr.NewAddressPubKeyHashEcdsaSecp256k1V0(stdaddr.Hash160(pub), params)
	must(err)
	return a
}

func hd() {
	params := chaincfg.MainNetParams()
	seed := make([]byte, 32)
	for i := range seed {
		seed[i] = byte(i)
	}
	master, err := hdkeychain.NewMaster(seed, params)
	must(err)
	acct := master
	for _, i := range []uint32{hardened + 44, hardened + 42, hardened} {
		acct, err = acct.Child(i)
		must(err)
	}
	acctPub := acct.Neuter()
	acctPubString := acctPub.String()
	branchPub, err := acctPub.Child(0)
	must(err)
	branchPriv, err := acct.Child(0)
	must(err)

	check("hd.master_pub", hex.EncodeToString(master.SerializedPubKey()))
	check("hd.acct_xpub", acctPubString)
	h7, err := acct.Child(hardened + 7)
	must(err)
	check("hd.hardened7_pub", hex.EncodeToString(h7.SerializedPubKey()))
	p7, err := branchPriv.Child(7)
	must(err)
	check("hd.priv7_pub", hex.EncodeToString(p7.SerializedPubKey()))
	c7, err := branchPub.Child(7)
	must(err)
	check("hd.pub7_pub", hex.EncodeToString(c7.SerializedPubKey()))
	_, script := p2pkh(c7.SerializedPubKey(), params).PaymentScript()
	check("hd.pub7_script", hex.EncodeToString(script))

	var ctr uint32
	next := func() uint32 {
		ctr = (ctr + 1) & 0x7fffffff
		return ctr
	}
	bench("hd.master", func(n uint64) uint64 {
		var x uint64
		for i := uint64(0); i < n; i++ {
			m, err := hdkeychain.NewMaster(seed, params)
			must(err)
			x ^= uint64(m.SerializedPubKey()[1])
		}
		return x
	})
	bench("hd.hardened_child", func(n uint64) uint64 {
		var x uint64
		for i := uint64(0); i < n; i++ {
			c, err := acct.Child(hardened + next())
			must(err)
			x ^= uint64(c.SerializedPubKey()[1])
		}
		return x
	})
	bench("hd.priv_child", func(n uint64) uint64 {
		var x uint64
		for i := uint64(0); i < n; i++ {
			c, err := branchPriv.Child(next())
			must(err)
			x ^= uint64(c.SerializedPubKey()[1])
		}
		return x
	})
	bench("hd.pub_child", func(n uint64) uint64 {
		var x uint64
		for i := uint64(0); i < n; i++ {
			c, err := branchPub.Child(next())
			must(err)
			x ^= uint64(c.SerializedPubKey()[1])
		}
		return x
	})
	bench("hd.address", func(n uint64) uint64 {
		var x uint64
		for i := uint64(0); i < n; i++ {
			c, err := branchPub.Child(next())
			must(err)
			x ^= uint64(p2pkh(c.SerializedPubKey(), params).String()[3])
		}
		return x
	})
	bench("hd.script", func(n uint64) uint64 {
		var x uint64
		for i := uint64(0); i < n; i++ {
			c, err := branchPub.Child(next())
			must(err)
			_, s := p2pkh(c.SerializedPubKey(), params).PaymentScript()
			x ^= uint64(s[5])
		}
		return x
	})

	// The 20,000-address workload: the first addresses of a branch, as
	// dcrwallet derives them for discovery and for the address pool, on one
	// thread.
	count := addrCount()
	addrs := func() []string {
		v := make([]string, 0, count)
		for i := uint32(0); i < count; i++ {
			c, err := branchPub.Child(i)
			must(err)
			v = append(v, p2pkh(c.SerializedPubKey(), params).String())
		}
		return v
	}
	if selected("hd.addr20k") {
		a := addrs()
		check("hd.addr20k_first", a[0])
		check("hd.addr20k_last", a[len(a)-1])
		check("hd.addr20k_sha256", sha256Lines(a))
	}
	benchOnce("hd.addr20k", func() uint64 { return uint64(len(addrs())) })

	// The extended-key workloads cycle through the first keys of the
	// public branch, each a different string, so that neither side's
	// branch predictor replays one string's base58 work.
	if !anySelected("hd.xpub_string", "hd.parse_xpub") {
		return
	}
	xcount := xpubSetSize()
	mask := uint64(xcount - 1)
	keys := make([]*hdkeychain.ExtendedKey, xcount)
	strs := make([]string, xcount)
	parsed := make([]string, xcount)
	for i := range keys {
		keys[i], err = branchPub.Child(uint32(i))
		must(err)
		strs[i] = keys[i].String()
	}
	for i, s := range strs {
		k, err := hdkeychain.NewKeyFromString(s, params)
		must(err)
		if !bytes.Equal(k.SerializedPubKey(), keys[i].SerializedPubKey()) {
			panic("hd: " + s + " does not parse to its key")
		}
		parsed[i] = hex.EncodeToString(k.SerializedPubKey())
	}
	check("hd.xpub_set_count", xcount)
	check("hd.xpub_set_sha256", sha256Lines(strs))
	check("hd.xpub_set_parsed_sha256", sha256Lines(parsed))
	bench("hd.xpub_string", func(n uint64) uint64 {
		var x uint64
		for i := uint64(0); i < n; i++ {
			x ^= uint64(keys[i&mask].String()[5])
		}
		return x
	})
	bench("hd.parse_xpub", func(n uint64) uint64 {
		var x uint64
		for i := uint64(0); i < n; i++ {
			k, err := hdkeychain.NewKeyFromString(strs[i&mask], params)
			must(err)
			x ^= uint64(k.SerializedPubKey()[1])
		}
		return x
	})
}

func primitives() {
	params := chaincfg.MainNetParams()
	rng := splitMix(0x7072_696d)
	key := rng.bytes(32)
	msg := rng.bytes(37)
	pub := append([]byte{0x02}, rng.bytes(32)...)
	payload := rng.bytes(20)
	ver := params.PubKeyHashAddrID

	mac := hmac.New(sha512.New, key)
	mac.Write(msg)
	check("crypto.hmac_sha512", hex.EncodeToString(mac.Sum(nil)))
	check("crypto.hash160", hex.EncodeToString(stdaddr.Hash160(pub)))
	encoded := base58.CheckEncode(payload, ver)
	check("crypto.base58check", encoded)

	bench("crypto.hmac_sha512", func(n uint64) uint64 {
		var x uint64
		for i := uint64(0); i < n; i++ {
			msg[36] = byte(i)
			m := hmac.New(sha512.New, key)
			m.Write(msg)
			x ^= uint64(m.Sum(nil)[0])
		}
		return x
	})
	bench("crypto.hash160", func(n uint64) uint64 {
		var x uint64
		for i := uint64(0); i < n; i++ {
			pub[32] = byte(i)
			x ^= uint64(stdaddr.Hash160(pub)[0])
		}
		return x
	})
	// The criterion's rows cycle through addressSetSize different
	// addresses (encode: their hashes; decode: their strings).
	if anySelected("crypto.base58check_encode", "crypto.base58check_decode") {
		hashes, strs := addressSet(ver)
		mask := uint64(addressSetSize - 1)
		decoded := make([]string, len(strs))
		for i, s := range strs {
			p, v, err := base58.CheckDecode(s)
			must(err)
			if !bytes.Equal(p, hashes[i][:]) || v != ver {
				panic("crypto: " + s + " does not decode to its hash")
			}
			decoded[i] = hex.EncodeToString(v[:]) + hex.EncodeToString(p)
		}
		check("crypto.base58check_set_count", addressSetSize)
		check("crypto.base58check_set_sha256", sha256Lines(strs))
		check("crypto.base58check_set_decoded_sha256", sha256Lines(decoded))
		bench("crypto.base58check_encode", func(n uint64) uint64 {
			var x uint64
			for i := uint64(0); i < n; i++ {
				x ^= uint64(base58.CheckEncode(hashes[i&mask][:], ver)[5])
			}
			return x
		})
		bench("crypto.base58check_decode", func(n uint64) uint64 {
			var x uint64
			for i := uint64(0); i < n; i++ {
				p, _, err := base58.CheckDecode(strs[i&mask])
				must(err)
				x ^= uint64(p[0])
			}
			return x
		})
	}
	// The decode row as it was until 2026-11-21: one string decoded again
	// and again. Held to no criterion; kept so that a session shows what
	// the repetition does to each side.
	bench("crypto.base58check_decode_one", func(n uint64) uint64 {
		var x uint64
		for i := uint64(0); i < n; i++ {
			p, _, err := base58.CheckDecode(encoded)
			must(err)
			x ^= uint64(p[0])
		}
		return x
	})
}

// addressSetSize is the number of different addresses the base58check
// workloads cycle through: 16,384, as dcroxide's own base58 benchmark draws,
// far more strings than a branch predictor can learn. Not reduced in smoke
// mode, as on the Rust side.
const addressSetSize = 16384

// xpubSetSize is the number of different extended public keys hd.xpub_string
// and hd.parse_xpub cycle through: 16,384 (256 in smoke mode).
func xpubSetSize() int {
	if cfg.smoke {
		return 256
	}
	return 16384
}

// addressSet is the base58check workloads' inputs: addressSetSize 20-byte
// hashes drawn from SplitMix64 with a seed of their own, and each hash's
// base58check string with the version ver, as purse-bench's address_set.
func addressSet(ver [2]byte) ([][20]byte, []string) {
	rng := splitMix(0x6235_3861)
	hashes := make([][20]byte, addressSetSize)
	strs := make([]string, addressSetSize)
	for i := range hashes {
		rng.fill(hashes[i][:])
		strs[i] = base58.CheckEncode(hashes[i][:], ver)
	}
	return hashes, strs
}

func snaclN() int {
	if cfg.smoke {
		return 1024
	}
	return snaclcopy.DefaultN
}

func snaclWorkloads() {
	// The setup derives a key at full cost: skipped when nothing here runs.
	if !anySelected("snacl.derive", "snacl.seal_32", "snacl.open_32", "snacl.seal_111", "snacl.open_111") {
		return
	}
	rng := splitMix(0x736e_6163)
	pass := []byte("bench passphrase")
	salt := rng.bytes(32)
	n := snaclN()

	// A master key's stored parameters, built as NewSecretKey would with
	// this salt (its own salt comes from dcrd's crypto/rand, which cannot be
	// injected).
	key, err := scrypt.Key(pass, salt, n, snaclcopy.DefaultR, snaclcopy.DefaultP, snaclcopy.KeySize)
	must(err)
	digest := sha256.Sum256(key)
	marshalled := make([]byte, 0, 88)
	marshalled = append(marshalled, salt...)
	marshalled = append(marshalled, digest[:]...)
	marshalled = binary.LittleEndian.AppendUint64(marshalled, uint64(n))
	marshalled = binary.LittleEndian.AppendUint64(marshalled, snaclcopy.DefaultR)
	marshalled = binary.LittleEndian.AppendUint64(marshalled, snaclcopy.DefaultP)
	check("snacl.derive_key", hex.EncodeToString(key))
	benchOnce("snacl.derive", func() uint64 {
		var sk snaclcopy.SecretKey
		must(sk.Unmarshal(marshalled))
		p := append([]byte(nil), pass...)
		must(sk.DeriveKey(&p))
		return uint64(sk.Key[0])
	})

	var ck snaclcopy.CryptoKey
	copy(ck[:], rng.bytes(32))
	for _, size := range []int{32, 111} {
		plain := rng.bytes(size)
		var nonce [snaclcopy.NonceSize]byte
		rng.fill(nonce[:])
		// A box as CryptoKey.Encrypt writes it, with a fixed nonce.
		sealed := secretbox.Seal(append([]byte(nil), nonce[:]...), plain, &nonce, (*[32]byte)(&ck))
		check(fmt.Sprintf("snacl.sealed_%d", size), hex.EncodeToString(sealed))
		opened, err := ck.Decrypt(sealed)
		must(err)
		if string(opened) != string(plain) {
			panic("snacl: box does not open")
		}
		bench(fmt.Sprintf("snacl.seal_%d", size), func(n uint64) uint64 {
			var x uint64
			for i := uint64(0); i < n; i++ {
				plain[0] = byte(i)
				s, err := ck.Encrypt(plain)
				must(err)
				x ^= uint64(s[30])
			}
			return x
		})
		bench(fmt.Sprintf("snacl.open_%d", size), func(n uint64) uint64 {
			var x uint64
			for i := uint64(0); i < n; i++ {
				p, err := ck.Decrypt(sealed)
				must(err)
				x ^= uint64(p[0])
			}
			return x
		})
	}
}

func argonMemory() uint32 {
	if cfg.smoke {
		return 1024
	}
	return 256 * 1024
}

func kdfWorkloads() {
	rng := splitMix(0x6b64_6600)
	pass := []byte("bench passphrase")
	for _, threads := range []uint8{1, 4} {
		p := &kdf.Argon2idParams{Time: 1, Memory: argonMemory(), Threads: threads}
		rng.fill(p.Salt[:])
		name := fmt.Sprintf("kdf.argon2id_t%d", threads)
		if selected(name) {
			check(name+"_key", hex.EncodeToString(kdf.DeriveKey(pass, p, 32)))
		}
		benchOnce(name, func() uint64 { return uint64(kdf.DeriveKey(pass, p, 32)[0]) })
	}
}

// amountInputs are 1,024 amounts in atoms of every magnitude from one atom
// to the money supply, a quarter of them negative.
func amountInputs() []dcrutil.Amount {
	rng := splitMix(0x616d_6f75)
	v := make([]dcrutil.Amount, 1024)
	for i := range v {
		digits := 1 + rng.next()%16
		m := uint64(1)
		for j := uint64(0); j < digits; j++ {
			m *= 10
		}
		a := int64(rng.next() % m)
		if rng.next()%4 == 0 {
			a = -a
		}
		v[i] = dcrutil.Amount(a)
	}
	return v
}

func amounts() {
	in := amountInputs()
	s := make([]string, len(in))
	for i, a := range in {
		s[i] = a.String()
	}
	check("dcrutil.amount_strings", sha256Lines(s))
	bench("dcrutil.amount_string", func(n uint64) uint64 {
		var x uint64
		for i := uint64(0); i < n; i++ {
			x ^= uint64(len(in[i%1024].String()))
		}
		return x
	})
}
