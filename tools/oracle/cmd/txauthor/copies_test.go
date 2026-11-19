// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

package main

// TestCopiesAreVerbatim enforces the copy technique of docs/specs/23
// section 5.5 for this command, as cmd/oprunner's copies_test.go does for
// its own: every file under internal/ that comes from upstream must equal
// the upstream file of the module version go.mod pins, as found in the
// module cache, after exactly the listed rewrite and a gofmt pass (which
// only re-sorts the rewritten import line), and a copied package may hold
// no file the table does not name.
//
// txauthor's RandomizeOutputPosition draws from the package global of
// github.com/decred/dcrd/crypto/rand, which has no seam; the copy of
// author.go here imports the seeded stream in internal/rand instead, so
// the oracle can draw what Go draws from a key the Rust test chooses.
// Everything else the oracle answers (NewUnsignedTransaction,
// AddAllInputScripts) runs the unmodified package.
//
// With PURSE_REGEN_COPIES=1 the test writes the copies instead of checking
// them (after a pin move: bump go.mod, regenerate, review the counts, run
// the test again).

import (
	"bytes"
	"go/format"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"testing"
)

const selfPath = "github.com/jzbz/dcr-purse/tools/oracle/cmd/txauthor/internal/"

type rewrite struct {
	old, new string
	want     int // exact number of replacements over the package
}

type copySpec struct {
	module   string   // module path as go.mod requires it
	dir      string   // package directory inside the module
	dst      string   // directory under internal/
	files    []string // files to copy
	own      []string // files of dst that are not copies (the interception)
	rewrites []rewrite
}

var copySpecs = []copySpec{
	{
		module: "github.com/decred/dcrd/crypto/rand",
		dir:    ".",
		dst:    "rand",
		files:  []string{"default.go", "doc.go", "uniform.go"},
		own:    []string{"prng.go"},
	},
	{
		module: "decred.org/dcrwallet/v5",
		dir:    "wallet/txauthor",
		dst:    "txauthor",
		files:  []string{"author.go"},
		rewrites: []rewrite{{
			old:  `"github.com/decred/dcrd/crypto/rand"`,
			new:  `"` + selfPath + `rand"`,
			want: 1,
		}},
	},
}

func moduleDir(t *testing.T, module string) string {
	t.Helper()
	out, err := exec.Command("go", "list", "-m", "-f", "{{.Dir}}", module).Output()
	if err != nil {
		t.Fatalf("go list -m %s: %v", module, err)
	}
	dir := strings.TrimSpace(string(out))
	if dir == "" {
		t.Fatalf("go list -m %s printed no module directory (module not downloaded?)", module)
	}
	return dir
}

func expected(t *testing.T, spec copySpec) map[string][]byte {
	t.Helper()
	src := filepath.Join(moduleDir(t, spec.module), filepath.FromSlash(spec.dir))
	counts := make([]int, len(spec.rewrites))
	out := make(map[string][]byte)
	for _, name := range spec.files {
		b, err := os.ReadFile(filepath.Join(src, name))
		if err != nil {
			t.Fatal(err)
		}
		changed := false
		for i, r := range spec.rewrites {
			n := bytes.Count(b, []byte(r.old))
			b = bytes.ReplaceAll(b, []byte(r.old), []byte(r.new))
			counts[i] += n
			changed = changed || n > 0
		}
		if changed {
			formatted, err := format.Source(b)
			if err != nil {
				t.Fatalf("%s/%s after rewriting: %v", spec.dir, name, err)
			}
			b = formatted
		}
		out[name] = b
	}
	for i, r := range spec.rewrites {
		if counts[i] != r.want {
			t.Errorf("%s: rewrite %q applies %d times, want %d (an upstream change to review)",
				spec.dir, r.old, counts[i], r.want)
		}
	}
	return out
}

func TestCopiesAreVerbatim(t *testing.T) {
	regen := os.Getenv("PURSE_REGEN_COPIES") == "1"
	for _, spec := range copySpecs {
		want := expected(t, spec)
		dst := filepath.Join("internal", spec.dst)
		if regen {
			for name, b := range want {
				if err := os.WriteFile(filepath.Join(dst, name), b, 0o644); err != nil {
					t.Fatal(err)
				}
			}
		}
		entries, err := os.ReadDir(dst)
		if err != nil {
			t.Fatal(err)
		}
		allowed := make(map[string]bool)
		for name := range want {
			allowed[name] = true
		}
		for _, name := range spec.own {
			allowed[name] = true
		}
		for _, e := range entries {
			if !e.IsDir() && strings.HasSuffix(e.Name(), ".go") && !allowed[e.Name()] {
				t.Errorf("%s/%s is neither a checked copy nor a listed interception file", dst, e.Name())
			}
		}
		for name, b := range want {
			local, err := os.ReadFile(filepath.Join(dst, name))
			if err != nil {
				t.Errorf("%s/%s: %v", dst, name, err)
				continue
			}
			// A checkout with core.autocrlf may hold CRLF; the module
			// cache always holds LF.
			local = bytes.ReplaceAll(local, []byte("\r\n"), []byte("\n"))
			if !bytes.Equal(local, b) {
				t.Errorf("%s/%s differs from %s/%s beyond the listed rewrites",
					dst, name, spec.dir, name)
			}
		}
	}
}

// TestSeededStreamMatchesOprunner keeps this command's interception file
// equal, below its header comment, to cmd/oprunner's: both are the seeded
// stream of docs/specs/23 section 5.6.2, and the Rust side models one
// stream.
func TestSeededStreamMatchesOprunner(t *testing.T) {
	body := func(path string) []byte {
		b, err := os.ReadFile(path)
		if err != nil {
			t.Fatal(err)
		}
		i := bytes.Index(b, []byte("\npackage rand\n"))
		if i < 0 {
			t.Fatalf("%s: no package clause", path)
		}
		return bytes.ReplaceAll(b[i:], []byte("\r\n"), []byte("\n"))
	}
	if !bytes.Equal(body("internal/rand/prng.go"), body("../oprunner/internal/rand/prng.go")) {
		t.Error("internal/rand/prng.go differs from ../oprunner/internal/rand/prng.go below the header")
	}
}

// TestSeededStream pins the stream the Rust side models: ChaCha20 under the
// key with a zero nonce, read from the start.
func TestSeededStream(t *testing.T) {
	var key [32]byte
	for i := range key {
		key[i] = byte(i)
	}
	seedAndDraw := func() (uint64, uint64) {
		seed(key)
		return uint64(randomizeDraw(5)), drawn()
	}
	r1, d1 := seedAndDraw()
	r2, d2 := seedAndDraw()
	if r1 != r2 || d1 != d2 || d1 != 8 {
		t.Fatalf("reseeding does not restart the stream: (%d, %d) then (%d, %d)", r1, d1, r2, d2)
	}
}
