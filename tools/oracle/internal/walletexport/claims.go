// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

package walletexport

// The closed world (ADR-0001 principle P1): every entry of the database,
// bucket entries included, is consumed by exactly one rule. Producers claim
// what they consume; a final cursor walk over the whole tree sends dead
// items to [residue] and anything else to [unknown], which fails the export
// unless allow_unknown is set.

import (
	"bytes"
	"crypto/sha256"
	"encoding/hex"
	"fmt"
	"hash"
	"strconv"
	"strings"

	bolt "go.etcd.io/bbolt"
)

// claims records what the producers consumed. A bucket whose every
// key/value entry a producer consumed (through exporter.each, or by marking
// it after a pass of its own) is recorded once, as a whole; only the few
// entries consumed one by one (top-level and namespace keys, the account
// variables) are kept individually. Memory therefore grows with the number
// of buckets and individually claimed keys, never with the number of rows
// (ADR-0001 Consequences, memory).
type claims struct {
	full map[string]struct{}
	set  map[string]struct{}
}

func newClaims() *claims {
	return &claims{full: make(map[string]struct{}), set: make(map[string]struct{})}
}

func joinPath(path []string) string { return strings.Join(path, "\x00") }

func claimKey(path []string, key []byte) string {
	return joinPath(path) + "\x01" + string(key)
}

// claim marks key in the bucket at path (bucket names, raw) consumed.
func (c *claims) claim(path []string, key []byte) {
	c.set[claimKey(path, key)] = struct{}{}
}

// claimAll marks every key/value entry of the bucket at path consumed.
// Nested buckets inside it are not claimed by this.
func (c *claims) claimAll(path []string) {
	c.full[joinPath(path)] = struct{}{}
}

func (c *claims) claimed(path []string, key []byte) bool {
	_, ok := c.set[claimKey(path, key)]
	return ok
}

func (c *claims) isFull(path []string) bool {
	_, ok := c.full[joinPath(path)]
	return ok
}

// pathString renders a bucket path for [bad], [unknown] and [residue]: each
// component as itself when it is made of [a-z0-9-] (every bucket name udb
// uses), otherwise in hex.
func pathString(path []string) string {
	parts := make([]string, len(path))
	for i, p := range path {
		parts[i] = componentString([]byte(p))
	}
	return strings.Join(parts, "/")
}

func componentString(p []byte) string {
	if len(p) == 0 {
		return ""
	}
	for _, c := range p {
		if !(c >= 'a' && c <= 'z' || c >= '0' && c <= '9' || c == '-') {
			return hex.EncodeToString(p)
		}
	}
	return string(p)
}

// residueRule names a dead item: the bucket path it lives in and a
// predicate on the key. A matching bucket entry carries everything below it
// into the same item.
type residueRule struct {
	name string
	path []string
	key  func([]byte) bool
}

func exactKey(s string) func([]byte) bool {
	return func(k []byte) bool { return string(k) == s }
}

// residueRules is the dead-item list of ADR-0001 section 9.
var residueRules = []residueRule{
	{name: "wstakemgr", path: nil, key: exactKey("wstakemgr")},
	{name: "waddrmgr/main/mgrver", path: []string{"waddrmgr", "main"}, key: exactKey("mgrver")},
	{name: "waddrmgr/main/seed", path: []string{"waddrmgr", "main"}, key: exactKey("seed")},
	{name: "waddrmgr/main/cscript", path: []string{"waddrmgr", "main"}, key: exactKey("cscript")},
	{name: "waddrmgr/main/lastaddrs", path: []string{"waddrmgr", "main"}, key: exactKey("lastaddrs")},
	{name: "waddrmgr/usedaddrs", path: []string{"waddrmgr"}, key: exactKey("usedaddrs")},
	{name: "waddrmgr/sync", path: []string{"waddrmgr"}, key: exactKey("sync")},
	{name: "waddrmgr/meta/addrpool", path: []string{"waddrmgr", "meta"}, key: func(k []byte) bool {
		return bytes.HasPrefix(k, []byte("addrpoolext")) || bytes.HasPrefix(k, []byte("addrpoolint"))
	}},
	{name: "wtxmgr/vers", path: []string{"wtxmgr"}, key: exactKey("vers")},
	{name: "wtxmgr/sc", path: []string{"wtxmgr"}, key: exactKey("sc")},
}

type residueAcc struct {
	entries uint64
	hasher  hash.Hash
}

func samePath(a, b []string) bool {
	if len(a) != len(b) {
		return false
	}
	for i := range a {
		if a[i] != b[i] {
			return false
		}
	}
	return true
}

// dumpPath renders a path as tools/oracle/cmd/boltdump does ("/" then hex
// components joined by "/").
func dumpPath(path []string) string {
	if len(path) == 0 {
		return "/"
	}
	parts := make([]string, len(path))
	for i, p := range path {
		parts[i] = hex.EncodeToString([]byte(p))
	}
	return "/" + strings.Join(parts, "/")
}

// walkResult is what the closed-world walk found.
type walkResult struct {
	residue map[string]*residueAcc
	unknown [][3]string // path, key hex, value hex or "-"
	seqs    []string    // dump-format lines of buckets with a nonzero sequence
}

func (c *claims) walk(tx *bolt.Tx) *walkResult {
	res := &walkResult{residue: make(map[string]*residueAcc)}
	acc := func(name string) *residueAcc {
		a := res.residue[name]
		if a == nil {
			a = &residueAcc{hasher: sha256.New()}
			res.residue[name] = a
		}
		return a
	}
	var visit func(path []string, b *bolt.Bucket, under string)
	visit = func(path []string, b *bolt.Bucket, under string) {
		full := c.isFull(path)
		cur := b.Cursor()
		for k, v := cur.First(); k != nil; k, v = cur.Next() {
			isBucket := v == nil
			var child *bolt.Bucket
			if isBucket {
				child = b.Bucket(k)
			}
			line := func() string {
				if isBucket {
					return fmt.Sprintf("b %s %x %d\n", dumpPath(path), k, child.Sequence())
				}
				return fmt.Sprintf("k %s %x %x\n", dumpPath(path), k, v)
			}
			if isBucket && child.Sequence() != 0 {
				res.seqs = append(res.seqs, line())
			}
			rule := under
			if rule == "" && !(full && !isBucket) && !c.claimed(path, k) {
				for _, r := range residueRules {
					if samePath(r.path, path) && r.key(k) {
						rule = r.name
						break
					}
				}
				if rule == "" {
					val := "-"
					if !isBucket {
						val = vX(v)
					}
					res.unknown = append(res.unknown, [3]string{vS([]byte(pathString(path))), vX(k), val})
				}
			}
			if rule != "" {
				a := acc(rule)
				a.entries++
				a.hasher.Write([]byte(line()))
			}
			if isBucket {
				// A claimed bucket entry does not claim its contents:
				// they are walked and must be claimed one by one. The
				// contents of a dead bucket are dead too.
				visit(append(append([]string(nil), path...), string(k)), child, rule)
			}
		}
	}
	visit(nil, tx.Cursor().Bucket(), "")
	return res
}

// emit adds the walk's findings to [residue] and [unknown].
func (r *walkResult) emit(residue, unknown *sectionBuf) {
	for name, a := range r.residue {
		residue.add("residue", vS([]byte(name)), vU(a.entries), vX(a.hasher.Sum(nil)))
	}
	if len(r.seqs) > 0 {
		h := sha256.New()
		for _, l := range r.seqs {
			h.Write([]byte(l))
		}
		residue.add("residue", vS([]byte("bucket-sequences")), strconv.Itoa(len(r.seqs)), vX(h.Sum(nil)))
	}
	for _, u := range r.unknown {
		unknown.add("unknown", u[0], u[1], u[2])
	}
}
