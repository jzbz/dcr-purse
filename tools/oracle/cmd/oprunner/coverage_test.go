// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

package main

import (
	"encoding/json"
	"go/ast"
	"go/build"
	"go/parser"
	"go/token"
	"os/exec"
	"path/filepath"
	"sort"
	"strings"
	"testing"
)

// udbImportPath is the package whose entry points the vocabulary covers.
const udbImportPath = "decred.org/dcrwallet/v5/wallet/udb"

// udbPin is the dcrwallet commit the oracle builds against (PARITY.md).
const udbPin = "54b85b336077"

// notInVocabulary names the exported entry points of wallet/udb at the pin
// that no operation's "calls" names, each with the reason (the package
// comment's "Coverage" table and its "Not covered" list).
var notInVocabulary = map[string]string{
	"Initialize":          "run by setup for a seed's wallet (script.go), not an operation",
	"InitializeWatchOnly": "run by setup for an extended public key's wallet (script.go), not an operation",
	"Upgrade":             "run inside Initialize and InitializeWatchOnly; dcr-purse writes the upgraded state directly and reads no older database (ADR-0013)",
	"Migrate":             "converts a pre-unified (two-file) wallet; dcr-purse reads no dcrwallet database (ADR-0013)",
	"UseLogger":           "logging only (crate txstore, choice 5)",
}

// udbEntryPoints returns the exported package functions of wallet/udb and
// the exported methods of *Manager and *Store (as "Manager.Name" and
// "Store.Name"), from the package's non-test files for this platform.
func udbEntryPoints(t *testing.T) []string {
	t.Helper()
	out, err := exec.Command("go", "list", "-f", "{{.Dir}}\t{{.Module.Version}}", udbImportPath).Output()
	if err != nil {
		t.Fatalf("go list %s: %v", udbImportPath, err)
	}
	dir, version, ok := strings.Cut(strings.TrimSpace(string(out)), "\t")
	if !ok {
		t.Fatalf("go list %s: unexpected output %q", udbImportPath, out)
	}
	if !strings.HasSuffix(version, "-"+udbPin) {
		t.Fatalf("wallet/udb is at %s, not at the pin %s", version, udbPin)
	}
	pkg, err := build.ImportDir(dir, 0)
	if err != nil {
		t.Fatalf("import %s: %v", dir, err)
	}
	fset := token.NewFileSet()
	var names []string
	for _, name := range pkg.GoFiles {
		f, err := parser.ParseFile(fset, filepath.Join(dir, name), nil, parser.SkipObjectResolution)
		if err != nil {
			t.Fatalf("parse %s: %v", name, err)
		}
		for _, d := range f.Decls {
			fn, ok := d.(*ast.FuncDecl)
			if !ok || !fn.Name.IsExported() {
				continue
			}
			if fn.Recv == nil {
				names = append(names, fn.Name.Name)
				continue
			}
			typ := fn.Recv.List[0].Type
			if star, ok := typ.(*ast.StarExpr); ok {
				typ = star.X
			}
			if id, ok := typ.(*ast.Ident); ok && (id.Name == "Manager" || id.Name == "Store") {
				names = append(names, id.Name+"."+fn.Name.Name)
			}
		}
	}
	sort.Strings(names)
	return names
}

// TestVocabularyCoversEveryEntryPoint checks that every exported method of
// *Manager and *Store and every exported package function of wallet/udb at
// the pin is named in some operation's "calls" (as the vocabulary command
// reports it), or listed in notInVocabulary with a reason; and that no
// listed name is also covered or gone. A new entry point at a pin move, or
// an operation that stops naming one, fails it.
func TestVocabularyCoversEveryEntryPoint(t *testing.T) {
	v, err := cmdVocabulary(nil)
	if err != nil {
		t.Fatal(err)
	}
	b, err := json.Marshal(v)
	if err != nil {
		t.Fatal(err)
	}
	var vocab struct {
		Ops []struct {
			Op    string `json:"op"`
			Calls string `json:"calls"`
		} `json:"ops"`
	}
	if err := json.Unmarshal(b, &vocab); err != nil {
		t.Fatal(err)
	}
	named := make(map[string][]string)
	for _, op := range vocab.Ops {
		for _, c := range strings.Split(op.Calls, ",") {
			c = strings.TrimSpace(c)
			named[c] = append(named[c], op.Op)
		}
	}
	entries := udbEntryPoints(t)
	if len(entries) == 0 {
		t.Fatal("no entry points found")
	}
	seen := make(map[string]bool)
	methods := 0
	for _, e := range entries {
		seen[e] = true
		if strings.Contains(e, ".") {
			methods++
		}
		why, listed := notInVocabulary[e]
		switch ops := named[e]; {
		case len(ops) > 0 && listed:
			t.Errorf("%s is named by %v and listed as not covered (%s)", e, ops, why)
		case len(ops) == 0 && !listed:
			t.Errorf("%s: no operation of the vocabulary names it and no reason is listed", e)
		}
	}
	for e, why := range notInVocabulary {
		if !seen[e] {
			t.Errorf("%s (%s) is listed but is not an exported entry point at the pin", e, why)
		}
	}
	t.Logf("%d entry points of wallet/udb (%d methods of *Manager and *Store), %d listed as not covered",
		len(entries), methods, len(notInVocabulary))
}
