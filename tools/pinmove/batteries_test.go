// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

package main

import (
	"bytes"
	"encoding/json"
	"os"
	"os/exec"
	"path"
	"path/filepath"
	"regexp"
	"strings"
	"testing"
)

// fakeLister answers Deps from a table.
type fakeLister struct {
	module string
	deps   map[string][]Pkg
}

func (f fakeLister) Module() (string, error) { return f.module, nil }

func (f fakeLister) Deps(cmd, tags string) ([]Pkg, error) {
	return f.deps[cmd+tags], nil
}

// batteryFixture builds an upstream with packages and a fixture database,
// and a dcr-purse with frozen files.
//
// From A to B: pkg/p/p.go changes, only the test file of pkg/q changes,
// testdata/fix.db.gz changes, and go.mod moves example.org/dep.
func batteryFixture(t *testing.T) (up, repo, a string) {
	isolateGit(t)
	up, repo = t.TempDir(), t.TempDir()
	gitT(t, up, "init", "-q")
	writeT(t, up, "go.mod", "module example.org/up\n\ngo 1.26.0\n\nrequire (\n\texample.org/dep v1.0.0\n\texample.org/other v0.1.0\n)\n")
	writeT(t, up, "pkg/p/p.go", "package p\n\nconst P = 1\n")
	writeT(t, up, "pkg/q/q.go", "package q\n\nconst Q = 1\n")
	writeT(t, up, "pkg/q/q_test.go", "package q\n")
	writeT(t, up, "testdata/fix.db.gz", "old fixture\n")
	gitT(t, up, "add", "-A")
	gitT(t, up, "commit", "-q", "-m", "A")
	a = gitT(t, up, "rev-parse", "HEAD")
	writeT(t, up, "go.mod", "module example.org/up\n\ngo 1.26.0\n\nrequire (\n\texample.org/dep v1.1.0\n\texample.org/other v0.1.0\n)\n")
	writeT(t, up, "pkg/p/p.go", "package p\n\nconst P = 2\n")
	writeT(t, up, "pkg/q/q_test.go", "package q\n\n// changed\n")
	writeT(t, up, "testdata/fix.db.gz", "new fixture\n")
	gitT(t, up, "commit", "-q", "-am", "B")
	gitT(t, up, "tag", "B")

	writeT(t, repo, "Cargo.toml", "[workspace]\n")
	d := "crates/c1/tests/data/"
	writeT(t, repo, d+"a.txt", "# vectors\n# dcrwallet commit: "+a+"\nbody\n")
	writeT(t, repo, d+"b.txt", "# vectors at dcrwallet "+a[:12]+"\n# also dcrwallet 0123456789ab\nbody\n")
	writeT(t, repo, d+"copy/fix.db.gz", "old fixture\n")
	writeT(t, repo, d+"copy/README.md", "Copied at `"+a[:8]+"`.\n")
	writeT(t, repo, d+"self.txt", "computed by the crate\n")
	writeT(t, repo, d+"mod.hex", "00\n")
	writeT(t, repo, d+"dep.txt", "# at the parity pin\n")
	writeT(t, repo, d+"cc.txt", "# nothing\n")
	writeT(t, repo, "crates/c2/tests/data/stray.txt", "not in the table\n")
	return up, repo, a
}

var batteryTable = Table{
	Batteries: []Battery{
		{Crate: "c1", Files: []string{"a.txt"}, Kind: KindGoGenerated, Commands: []string{"cp"}, PinCheck: "a test", Regenerate: "regen-a"},
		{Crate: "c1", Files: []string{"b.txt"}, Kind: KindGoGenerated, Commands: []string{"cq"}, Regenerate: "regen-b"},
		{Crate: "c1", Files: []string{"copy/*.db.gz"}, Kind: KindUpstreamCopy, Inputs: []string{"testdata/*.db.gz"}, Regenerate: "copy"},
		{Crate: "c1", Files: []string{"copy/README.md"}, Kind: KindDoc},
		{Crate: "c1", Files: []string{"self.txt"}, Kind: KindSelf},
		{Crate: "c1", Files: []string{"mod.hex"}, Kind: KindModuleCopy, Modules: []string{"example.org/dep"}, Regenerate: "copy module"},
		{Crate: "c1", Files: []string{"dep.txt"}, Kind: KindGoGenerated, Commands: []string{"cd"}, Regenerate: "regen-d"},
		{Crate: "c1", Files: []string{"cc.txt", "missing*.txt"}, Kind: KindGoGenerated, Commands: []string{"cc"}, Regenerate: "regen-c"},
		{Crate: "c1", Files: []string{"a.txt"}, Kind: KindSelf}, // a second battery naming a.txt
	},
	Copies: []Copy{
		{Oracle: "internal/copyp/p.go", Package: "internal/copyp", Upstream: []string{"pkg/p/p.go"}, Regenerate: "recopy p"},
		{Oracle: "internal/copyq", Package: "internal/copyq", Upstream: []string{"pkg/q/"}, Regenerate: "recopy q"},
		{Oracle: "internal/rand", Package: "internal/rand", Module: "example.org/dep", Upstream: []string{"r.go"}, Regenerate: "recopy rand"},
		{Oracle: "up.go.mod", Upstream: []string{"go.mod"}, Regenerate: "recopy go.mod"},
	},
}

func TestBatteriesSynthetic(t *testing.T) {
	up, repo, a := batteryFixture(t)
	an, err := Analyze(repo, up, a, "B", nil)
	if err != nil {
		t.Fatal(err)
	}
	ver := "v0.0.0-20260102030405-" + a[:12]
	std := Pkg{ImportPath: "fmt", Standard: true}
	l := fakeLister{module: "example.org/oracle", deps: map[string][]Pkg{
		"cp": {std, {ImportPath: "example.org/up/pkg/p", Module: "example.org/up", Version: ver}},
		"cq": {std, {ImportPath: "example.org/up/pkg/q", Module: "example.org/up", Version: ver},
			{ImportPath: "example.org/oracle/internal/copyq", Module: "example.org/oracle"}},
		"cd": {{ImportPath: "example.org/other/x", Module: "example.org/other", Version: "v0.1.0"},
			{ImportPath: "example.org/dep/y", Module: "example.org/dep", Version: "v1.2.0"}},
		"cc": {{ImportPath: "example.org/oracle/internal/copyp", Module: "example.org/oracle"},
			{ImportPath: "example.org/oracle/internal/rand", Module: "example.org/oracle"},
			{ImportPath: "example.org/dep", Module: "example.org/dep", Version: "v1.0.0"}},
	}}
	b, err := an.ComputeBatteries(batteryTable, "oracle", l)
	if err != nil {
		t.Fatal(err)
	}
	an.Batteries = b
	if !b.DcrwalletOK || b.Dcrwallet != ver {
		t.Errorf("oracle version: %q ok=%v", b.Dcrwallet, b.DcrwalletOK)
	}
	if strings.Join(b.Uncovered, " ") != "crates/c2/tests/data/stray.txt" {
		t.Errorf("uncovered: %v", b.Uncovered)
	}
	if strings.Join(b.Ambiguous, " ") != "crates/c1/tests/data/a.txt" {
		t.Errorf("ambiguous: %v", b.Ambiguous)
	}
	if strings.Join(b.Unmatched, " ") != "crates/c1/tests/data/missing*.txt" {
		t.Errorf("unmatched: %v", b.Unmatched)
	}
	type want struct {
		group   int
		verdict string
		pin     string
		reasons []string
	}
	wants := []want{
		{1, VerdictRegenerate, PinFull, []string{"package `pkg/p` changed (`pkg/p/p.go`)"}},
		// Only a test file of pkg/q changed: nothing linked moved.
		{2, VerdictPinOnly, PinAbbrev, nil},
		{2, VerdictRegenerate, PinReadme, []string{"input `testdata/fix.db.gz` changed (M)"}},
		{0, VerdictDoc, PinShort, nil},
		{3, VerdictUnchanged, pinNone, nil},
		{3, VerdictRegenerate, pinNone, []string{"module `example.org/dep` moves v1.0.0 -> v1.1.0"}},
		// Names no commit but says "parity pin": tied by its header.  It
		// links example.org/dep at a version of the oracle's own choosing.
		{2, VerdictRegenerate, PinPhrase, []string{"module `example.org/dep` moves v1.0.0 -> v1.1.0 (the oracle links v1.2.0)"}},
		// Tied to the commit through the copy of pkg/p/p.go it links.
		{2, VerdictRegenerate, pinNone, []string{"copied file `pkg/p/p.go` changed (M)", "module `example.org/dep` moves v1.0.0 -> v1.1.0"}},
		{3, VerdictUnchanged, PinFull, nil},
	}
	if len(b.Results) != len(wants) {
		t.Fatalf("%d results", len(b.Results))
	}
	for i, w := range wants {
		r := b.Results[i]
		if r.Group != w.group || r.Verdict != w.verdict || strings.Join(r.Reasons, "|") != strings.Join(w.reasons, "|") {
			t.Errorf("battery %d (%s): group %d verdict %s reasons %q; want %d %s %q", i, batteryName(r), r.Group, r.Verdict, r.Reasons, w.group, w.verdict, w.reasons)
		}
		if len(r.Matched) == 0 || r.Matched[0].Pin != w.pin {
			t.Errorf("battery %d: files %+v, want pin %q", i, r.Matched, w.pin)
		}
	}
	if o := b.Results[1].Matched[0].Other; strings.Join(o, " ") != "0123456789ab" {
		t.Errorf("other commits: %v", o)
	}
	if got := b.Results[1].Upstream; strings.Join(got, " ") != "pkg/q/" {
		t.Errorf("upstream of b.txt: %v", got)
	}
	copies := map[string]string{}
	for _, c := range b.Copies {
		copies[c.Oracle] = c.Verdict + " " + strings.Join(c.UsedBy, ",") + " " + strings.Join(c.Changed, "|")
	}
	for k, v := range map[string]string{
		"internal/copyp/p.go": "regenerate cc `pkg/p/p.go` changed (M)",
		"internal/copyq":      "does-not-move cq ",
		"internal/rand":       "regenerate cc module `example.org/dep` moves v1.0.0 -> v1.1.0",
		"up.go.mod":           "regenerate  `go.mod` changed (M)",
	} {
		if copies[k] != v {
			t.Errorf("copy %s: %q, want %q", k, copies[k], v)
		}
	}
	vc := b.VerdictCounts()
	if vc[VerdictRegenerate] != 5 || vc[VerdictPinOnly] != 1 || vc[VerdictUnchanged] != 2 || vc[VerdictDoc] != 1 {
		t.Errorf("verdict counts: %v", vc)
	}

	var md, br, js bytes.Buffer
	if err := an.WriteReport(&md); err != nil {
		t.Fatal(err)
	}
	for _, s := range []string{
		"- Frozen files: 5 to regenerate because something they depend on changed, 1 pin-only, 2 that do not move; 3 oracle copies of upstream source to regenerate.",
		"## Frozen files and regenerators",
		"**Frozen files no battery names**",
		"- c1: a.txt: package `pkg/p` changed (`pkg/p/p.go`). Regenerate with `regen-a`.",
	} {
		if !strings.Contains(md.String(), s) {
			t.Errorf("report lacks %q:\n%s", s, md.String())
		}
	}
	if err := an.WriteBatteriesReport(&br); err != nil {
		t.Fatal(err)
	}
	for _, s := range []string{
		"### Every frozen file",
		"| `crates/c1/tests/data/b.txt` | c1: b.txt | 12 digits | `0123456789ab` |",
		"### Files without a checked pin (ADR-0016, answer 8)",
		"- `crates/c1/tests/data/b.txt` (12 digits)",
	} {
		if !strings.Contains(br.String(), s) {
			t.Errorf("batteries report lacks %q:\n%s", s, br.String())
		}
	}
	if err := an.WriteBatteriesJSON(&js); err != nil {
		t.Fatal(err)
	}
	var back struct {
		Batteries struct {
			Batteries []struct{ Verdict string }
		}
	}
	if err := json.Unmarshal(js.Bytes(), &back); err != nil || len(back.Batteries.Batteries) != len(wants) {
		t.Errorf("JSON: %v %+v", err, back)
	}
}

func TestPinForm(t *testing.T) {
	c := "54b85b336077587eaf20112dc695355ce06fc76e"
	for _, tc := range []struct{ head, form string }{
		{"# dcrwallet commit: " + c + "\n", PinFull},
		{"# as linked by dcrwallet 54b85b336077),\n", PinAbbrev},
		{"pin `54b85b33`\n", PinShort},
		{"x54b85b33y\n", pinNone},
		{"at the Parity Pin\n", PinPhrase},
		{"nothing\n", pinNone},
	} {
		if got, _ := pinForm([]byte(tc.head), c); got != tc.form {
			t.Errorf("%q: %q, want %q", tc.head, got, tc.form)
		}
	}
	// A 40-digit hash on a line that mentions neither dcrwallet nor a
	// commit is data, not a commit.
	_, other := pinForm([]byte("hash ffffffffffffffffffffffffffffffffffffffff\ndcrwallet commit eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee\n"), c)
	if strings.Join(other, " ") != "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee" {
		t.Errorf("other: %v", other)
	}
	if pseudoCommit("v5.0.1-0.20261105142009-54b85b336077") != "54b85b336077" || pseudoCommit("v5.1.2") != "" {
		t.Error("pseudoCommit")
	}
}

// TestGoLister runs the real `go list -deps` on a small oracle module that
// replaces its upstream with a local directory, offline.
func TestGoLister(t *testing.T) {
	if _, err := exec.LookPath("go"); err != nil {
		t.Skip("go not found")
	}
	t.Setenv("GOPROXY", "off")
	t.Setenv("GOTOOLCHAIN", "local")
	t.Setenv("GO111MODULE", "on")
	t.Setenv("GOFLAGS", "")
	root := t.TempDir()
	writeT(t, root, "up/go.mod", "module example.org/up\n\ngo 1.21\n")
	writeT(t, root, "up/pkg/p/p.go", "package p\n\nconst P = 1\n")
	writeT(t, root, "oracle/go.mod", "module example.org/oracle\n\ngo 1.21\n\nrequire example.org/up v0.0.0\n\nreplace example.org/up => ../up\n")
	writeT(t, root, "oracle/internal/copyp/p.go", "package copyp\n\nconst P = 1\n")
	writeT(t, root, "oracle/cmd/x/main.go", "package main\n\nimport (\n\t\"fmt\"\n\n\t\"example.org/oracle/internal/copyp\"\n\t\"example.org/up/pkg/p\"\n)\n\nfunc main() { fmt.Println(p.P, copyp.P) }\n")
	writeT(t, root, "oracle/cmd/x/tagged.go", "//go:build special\n\npackage main\n\nimport _ \"example.org/up/pkg/p\"\n")
	l := &GoLister{Dir: filepath.Join(root, "oracle")}
	mod, err := l.Module()
	if err != nil || mod != "example.org/oracle" {
		t.Fatalf("module %q: %v", mod, err)
	}
	pkgs, err := l.Deps("x", "special")
	if err != nil {
		t.Fatal(err)
	}
	got := map[string]Pkg{}
	for _, p := range pkgs {
		got[p.ImportPath] = p
	}
	if p := got["example.org/up/pkg/p"]; p.Module != "example.org/up" || p.Version != "v0.0.0" {
		t.Errorf("upstream package: %+v", p)
	}
	if p := got["example.org/oracle/internal/copyp"]; p.Module != "example.org/oracle" {
		t.Errorf("oracle package: %+v", p)
	}
	if p := got["fmt"]; !p.Standard {
		t.Errorf("fmt: %+v", p)
	}
	if _, err := l.Deps("missing", ""); err == nil {
		t.Error("a missing command was listed")
	}
}

// repoRoot is the dcr-purse checkout tools/pinmove sits in.
func repoRoot(t *testing.T) string {
	root, err := filepath.Abs(filepath.Join("..", ".."))
	if err != nil {
		t.Fatal(err)
	}
	if _, err := os.Stat(filepath.Join(root, "crates", "purse-testutil", "src", "lib.rs")); err != nil {
		t.Skip("not inside the dcr-purse repository")
	}
	return root
}

var headerCmdRe = regexp.MustCompile(`tools/oracle/cmd/([a-z0-9]+)`)

// TestRepoTable holds RepoTable to the repository (see its comment).
func TestRepoTable(t *testing.T) {
	root := repoRoot(t)
	pin, err := parityCommit(root)
	if err != nil {
		t.Fatal(err)
	}
	a := &Analysis{Repo: root, From: Rev{Commit: pin}, Files: map[string]*FileDiff{}}
	b, err := a.ComputeBatteries(RepoTable, "", nil)
	if err != nil {
		t.Fatal(err)
	}
	if len(b.Uncovered) > 0 {
		t.Errorf("frozen files missing from RepoTable: %v", b.Uncovered)
	}
	if len(b.Ambiguous) > 0 {
		t.Errorf("frozen files two batteries name: %v", b.Ambiguous)
	}
	if len(b.Unmatched) > 0 {
		t.Errorf("RepoTable patterns matching no file: %v", b.Unmatched)
	}
	for _, r := range b.Results {
		cmds := map[string]bool{}
		for _, c := range r.Commands {
			cmds[c] = true
			if st, err := os.Stat(filepath.Join(root, "tools", "oracle", "cmd", c)); err != nil || !st.IsDir() {
				t.Errorf("%s: no oracle command %s", batteryName(r), c)
			}
		}
		if (r.Kind == KindGoGenerated) != (len(r.Commands) > 0) {
			t.Errorf("%s: kind %s with commands %v", batteryName(r), r.Kind, r.Commands)
		}
		if r.Kind != KindSelf && r.Kind != KindDoc && r.Regenerate == "" {
			t.Errorf("%s: no regenerator", batteryName(r))
		}
		for _, f := range r.Matched {
			if r.PinCheck != "" && f.Pin != PinFull {
				t.Errorf("%s: a test is said to check the pin, but the file names it as %q", f.Path, f.Pin)
			}
			if f.Binary || r.Kind == KindDoc {
				continue
			}
			head, err := readHead(filepath.Join(root, filepath.FromSlash(f.Path)))
			if err != nil {
				t.Fatal(err)
			}
			for _, m := range headerCmdRe.FindAllSubmatch(head, -1) {
				if !cmds[string(m[1])] {
					t.Errorf("%s names tools/oracle/cmd/%s, which its battery does not list", f.Path, m[1])
				}
			}
			if len(f.Other) > 0 {
				t.Errorf("%s names commits other than the pin: %v", f.Path, f.Other)
			}
		}
	}
	for _, c := range RepoTable.Copies {
		if _, err := os.Stat(filepath.Join(root, "tools", "oracle", filepath.FromSlash(c.Oracle))); err != nil {
			t.Errorf("copy %s: %v", c.Oracle, err)
		}
		if c.Package != "" {
			if st, err := os.Stat(filepath.Join(root, "tools", "oracle", filepath.FromSlash(c.Package))); err != nil || !st.IsDir() {
				t.Errorf("copy %s: no package directory %s", c.Oracle, c.Package)
			}
			if !strings.HasPrefix(c.Oracle, c.Package) {
				t.Errorf("copy %s lies outside its package %s", c.Oracle, c.Package)
			}
		}
		for _, u := range c.Upstream {
			if u == "" || path.IsAbs(u) {
				t.Errorf("copy %s: bad upstream path %q", c.Oracle, u)
			}
		}
	}
}
