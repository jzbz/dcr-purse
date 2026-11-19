// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

package main

// TestCopiesAreVerbatim enforces the copy technique of docs/specs/23
// section 5.5: every file under internal/ that comes from upstream must
// equal the upstream file of the module version go.mod pins, as found in
// the module cache, after exactly the rewrites listed in copySpecs and a
// gofmt pass (which only re-sorts the rewritten import lines). Nothing
// else in a copied package may differ, and a copied package may hold no
// file the table does not name.
//
// The rewrites are the interception and nothing more: imports of
// dcrd/crypto/rand go to the seeded stream in internal/rand, imports of
// dcrwallet's internal and kdf packages go to their copies (the internal
// ones cannot be imported from outside dcrwallet's module, and the copies
// must link the seeded stream), time.Now() goes to seam.Now() and
// runtime.NumCPU() to seam.NumCPU(). Each rewrite carries the number of
// times it must apply. The counts cover only the files a rewrite names,
// so TestCopiesDrawOnlyThroughTheSeams also scans every copied file
// (after the rewrites) for a clock read, a randomness source or a
// processor count left in place: a pin move, or a regeneration, that adds
// one anywhere in the copies fails there instead of escaping the seams.
//
// What neither test can see is the unmodified dependencies the copies
// call. At the pin they were audited by hand (every non-test file of each
// package the copies import, searched for time.Now, time.Since,
// time.Until, crypto/rand, math/rand, dcrd/crypto/rand and
// runtime.NumCPU); the sources found, none of which udb or the runner
// reaches:
//
//	wallet/txauthor  author.go:172 RandomizeOutputPosition (udb uses only
//	                 the InputDetail type)
//	dcrec/secp256k1  privkey.go:76 GeneratePrivateKey (v4.4.1)
//	hdkeychain       extendedkey.go:630 GenerateSeed (v3.1.3; scripts
//	                 supply seeds)
//	txscript         sigcache.go:130 createShortTxHashKey, from NewSigCache
//	                 (v4.1.2)
//	wire             common.go:1012 RandomUint64; time.Now in
//	                 NewBlockHeader (blockheader.go:205), NewNetAddressIPPort
//	                 (netaddress.go:72), NewNetAddressV2IPPort
//	                 (netaddressv2.go:81), NewMsgVersion (msgversion.go:243)
//	                 (v1.8.0; the runner builds headers itself)
//	slog             log.go:286, 320 time.Now for log lines (v1.2.0; udb's
//	                 logger is slog.Disabled, wallet/udb/log.go:13)
//
// A pin move repeats that audit.
//
// With PURSE_REGEN_COPIES=1 the test writes the copies instead of
// checking them (after a pin move: bump go.mod, regenerate, review the
// counts, run the test again).

import (
	"bytes"
	"go/ast"
	"go/format"
	"go/parser"
	"go/token"
	"os"
	"os/exec"
	"path/filepath"
	"sort"
	"strconv"
	"strings"
	"testing"
)

const (
	selfPath  = "github.com/jzbz/dcr-purse/tools/oracle/cmd/oprunner/internal/"
	seamPath  = selfPath + "seam"
	seamQuote = "\t\"" + seamPath + "\"\n"
)

type rewrite struct {
	// files limits the rewrite to these file names (all files if empty).
	files []string
	old   string
	new   string
	// want is the exact number of replacements over the package.
	want int
}

type copySpec struct {
	module string // module path as go.mod requires it
	dir    string // package directory inside the module
	dst    string // directory under internal/
	// files to copy; nil copies every non-test .go file of dir.
	files []string
	// own names files of dst that are not copies (the interception).
	own      []string
	rewrites []rewrite
}

var (
	importSeam = rewrite{old: "\nimport (\n", new: "\nimport (\n" + seamQuote}
)

func importRewrite(from, to string, want int) rewrite {
	return rewrite{old: `"` + from + `"`, new: `"` + selfPath + to + `"`, want: want}
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
		dir:    "internal/compat",
		dst:    "compat",
	},
	{
		module: "decred.org/dcrwallet/v5",
		dir:    "kdf",
		dst:    "kdf",
		rewrites: []rewrite{
			{old: "runtime.NumCPU()", new: "seam.NumCPU()", want: 1},
			withFiles(importSeam, 1, "kdf.go"),
		},
	},
	{
		module: "decred.org/dcrwallet/v5",
		dir:    "wallet/internal/snacl",
		dst:    "snacl",
		rewrites: []rewrite{
			importRewrite("github.com/decred/dcrd/crypto/rand", "rand", 1),
		},
	},
	{
		module: "decred.org/dcrwallet/v5",
		dir:    "wallet/udb",
		dst:    "udb",
		rewrites: []rewrite{
			importRewrite("github.com/decred/dcrd/crypto/rand", "rand", 2),
			importRewrite("decred.org/dcrwallet/v5/wallet/internal/snacl", "snacl", 2),
			importRewrite("decred.org/dcrwallet/v5/internal/compat", "compat", 3),
			importRewrite("decred.org/dcrwallet/v5/kdf", "kdf", 2),
			{files: []string{"addressdb.go", "txdb.go"}, old: "time.Now()", new: "seam.Now()", want: 5},
			withFiles(importSeam, 2, "addressdb.go", "txdb.go"),
			// addressdb.go reads the clock and nothing else of package
			// time, so its time import goes with the rewrite.
			{files: []string{"addressdb.go"}, old: "\t\"time\"\n", new: "", want: 1},
		},
	},
}

func withFiles(r rewrite, want int, files ...string) rewrite {
	r.files = files
	r.want = want
	return r
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

func applies(r rewrite, file string) bool {
	if len(r.files) == 0 {
		return true
	}
	for _, f := range r.files {
		if f == file {
			return true
		}
	}
	return false
}

// expected returns the copy of each upstream file as it must appear under
// internal/.
func expected(t *testing.T, spec copySpec) map[string][]byte {
	t.Helper()
	src := filepath.Join(moduleDir(t, spec.module), filepath.FromSlash(spec.dir))
	files := spec.files
	if files == nil {
		entries, err := os.ReadDir(src)
		if err != nil {
			t.Fatal(err)
		}
		for _, e := range entries {
			n := e.Name()
			if !e.IsDir() && strings.HasSuffix(n, ".go") && !strings.HasSuffix(n, "_test.go") {
				files = append(files, n)
			}
		}
	}
	counts := make([]int, len(spec.rewrites))
	out := make(map[string][]byte)
	for _, name := range files {
		b, err := os.ReadFile(filepath.Join(src, name))
		if err != nil {
			t.Fatal(err)
		}
		changed := false
		for i, r := range spec.rewrites {
			if !applies(r, name) {
				continue
			}
			n := bytes.Count(b, []byte(r.old))
			if r.old == importSeam.old {
				// One import of the seam per file, and only where the
				// file was otherwise rewritten to use it.
				n = min(n, 1)
				if n == 1 {
					b = bytes.Replace(b, []byte(r.old), []byte(r.new), 1)
				}
			} else {
				b = bytes.ReplaceAll(b, []byte(r.old), []byte(r.new))
			}
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
		var have []string
		for _, e := range entries {
			if !e.IsDir() && strings.HasSuffix(e.Name(), ".go") {
				have = append(have, e.Name())
			}
		}
		allowed := make(map[string]bool)
		for name := range want {
			allowed[name] = true
		}
		for _, name := range spec.own {
			allowed[name] = true
		}
		sort.Strings(have)
		for _, name := range have {
			if !allowed[name] {
				t.Errorf("%s/%s is neither a checked copy nor a listed interception file", dst, name)
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

// copiedDirs are the directories under internal/ that hold copies.
var copiedDirs = []string{"udb", "snacl", "compat", "kdf", "rand"}

// allowedImports lists, per copied directory and file, the imports of a
// randomness package that are not draws from the operating system: the
// copy of dcrd/crypto/rand imports the standard crypto/rand only for
// rand.Int with its own seeded generator as the reader (uniform.go:235).
var allowedImports = map[string]string{
	"rand/uniform.go": "crypto/rand",
}

// TestCopiesDrawOnlyThroughTheSeams fails on any clock read, randomness
// source or processor count left in the copies: a reference to time.Now,
// time.Since or time.Until; an import of crypto/rand, math/rand,
// math/rand/v2 or an unrewritten github.com/decred/dcrd/crypto/rand; a
// reference to runtime.NumCPU. The seams (seam.Now, seam.NumCPU and the
// seeded stream in internal/rand) are then the only sources the copies
// can reach.
func TestCopiesDrawOnlyThroughTheSeams(t *testing.T) {
	banned := map[string]map[string]bool{
		"time":    {"Now": true, "Since": true, "Until": true},
		"runtime": {"NumCPU": true},
	}
	bannedImports := map[string]bool{
		"crypto/rand": true, "math/rand": true, "math/rand/v2": true,
		"github.com/decred/dcrd/crypto/rand": true,
	}
	for _, dir := range copiedDirs {
		entries, err := os.ReadDir(filepath.Join("internal", dir))
		if err != nil {
			t.Fatal(err)
		}
		for _, e := range entries {
			name := e.Name()
			if e.IsDir() || !strings.HasSuffix(name, ".go") {
				continue
			}
			rel := dir + "/" + name
			fset := token.NewFileSet()
			f, err := parser.ParseFile(fset, filepath.Join("internal", dir, name), nil, 0)
			if err != nil {
				t.Fatal(err)
			}
			// Local names of the imports of package time and runtime.
			local := make(map[string]string)
			for _, imp := range f.Imports {
				path, _ := strconv.Unquote(imp.Path.Value)
				if bannedImports[path] && allowedImports[rel] != path {
					t.Errorf("%s imports %s: a randomness source outside the seeded stream",
						rel, path)
				}
				if banned[path] != nil {
					n := path
					if imp.Name != nil {
						n = imp.Name.Name
					}
					local[n] = path
				}
			}
			ast.Inspect(f, func(n ast.Node) bool {
				sel, ok := n.(*ast.SelectorExpr)
				if !ok {
					return true
				}
				id, ok := sel.X.(*ast.Ident)
				if !ok {
					return true
				}
				if path, ok := local[id.Name]; ok && banned[path][sel.Sel.Name] {
					t.Errorf("%s: %s.%s at %v escapes the seams", rel, path, sel.Sel.Name,
						fset.Position(sel.Pos()))
				}
				return true
			})
		}
	}
}
