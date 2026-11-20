// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

package main

import (
	"bytes"
	"encoding/json"
	"fmt"
	"math/rand"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"testing"
)

// The tests below build small git repositories in temporary directories:
// a synthetic upstream with fixed revisions, and a synthetic dcr-purse
// whose documents cite it.  Commits there are unsigned and use no user or
// system git configuration.

func isolateGit(t *testing.T) {
	t.Helper()
	if _, err := exec.LookPath("git"); err != nil {
		t.Skip("git not found")
	}
	t.Setenv("GIT_CONFIG_GLOBAL", os.DevNull)
	t.Setenv("GIT_CONFIG_NOSYSTEM", "1")
	t.Setenv("GIT_AUTHOR_NAME", "test")
	t.Setenv("GIT_AUTHOR_EMAIL", "test@example.invalid")
	t.Setenv("GIT_COMMITTER_NAME", "test")
	t.Setenv("GIT_COMMITTER_EMAIL", "test@example.invalid")
	t.Setenv("GIT_AUTHOR_DATE", "2026-01-02T03:04:05Z")
	t.Setenv("GIT_COMMITTER_DATE", "2026-01-02T03:04:05Z")
}

func gitT(t *testing.T, dir string, args ...string) string {
	t.Helper()
	full := append([]string{"-C", dir, "-c", "commit.gpgsign=false", "-c", "tag.gpgsign=false", "-c", "init.defaultBranch=master"}, args...)
	out, err := exec.Command("git", full...).CombinedOutput()
	if err != nil {
		t.Fatalf("git %v: %v\n%s", args, err, out)
	}
	return strings.TrimSpace(string(out))
}

func writeT(t *testing.T, dir, name, content string) {
	t.Helper()
	fn := filepath.Join(dir, filepath.FromSlash(name))
	if err := os.MkdirAll(filepath.Dir(fn), 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(fn, []byte(content), 0o644); err != nil {
		t.Fatal(err)
	}
}

func readT(t *testing.T, dir, name string) string {
	t.Helper()
	b, err := os.ReadFile(filepath.Join(dir, filepath.FromSlash(name)))
	if err != nil {
		t.Fatal(err)
	}
	return string(b)
}

// lines returns n numbered lines with a prefix, so every line is unique.
func lines(prefix string, n int) []string {
	out := make([]string, n)
	for i := range out {
		out[i] = fmt.Sprintf("%s line %d", prefix, i+1)
	}
	return out
}

func join(ls []string) string { return strings.Join(ls, "\n") + "\n" }

// insertAfter inserts ins after 1-based line n.
func insertAfter(ls []string, n int, ins ...string) []string {
	out := append([]string{}, ls[:n]...)
	out = append(out, ins...)
	return append(out, ls[n:]...)
}

type fixture struct {
	up, docs string
	a, b     string // the old and new upstream commits
}

// newFixture builds the upstream with revisions A and B and the documents.
//
// From A to B (two commits): in a.go, two lines are inserted after line 5,
// line 20 is replaced, and a line is inserted after line 30; b.go is
// deleted; c.go is renamed to d.go; go.mod moves a requirement.
func newFixture(t *testing.T) fixture {
	isolateGit(t)
	f := fixture{up: t.TempDir(), docs: t.TempDir()}
	gitT(t, f.up, "init", "-q")
	a := lines("a", 60)
	writeT(t, f.up, "a.go", join(a))
	writeT(t, f.up, "b.go", join(lines("b", 30)))
	writeT(t, f.up, "c.go", join(lines("c", 40)))
	writeT(t, f.up, "sub/x.go", join(lines("x", 40)))
	writeT(t, f.up, "go.mod", "module example.org/up\n\ngo 1.26.0\n\nrequire (\n\texample.org/dep/mod v1.0.0\n\texample.org/other v0.1.0 // indirect\n)\n")
	gitT(t, f.up, "add", "-A")
	gitT(t, f.up, "commit", "-q", "-m", "A")
	f.a = gitT(t, f.up, "rev-parse", "HEAD")

	a2 := insertAfter(a, 5, "new 1", "new 2")
	a2[21] = "changed line 20" // old line 20, now line 22
	a2 = insertAfter(a2, 32, "inserted after old 30")
	writeT(t, f.up, "a.go", join(a2))
	gitT(t, f.up, "commit", "-q", "-am", "Edit a.go")
	gitT(t, f.up, "rm", "-q", "b.go")
	gitT(t, f.up, "mv", "c.go", "d.go")
	writeT(t, f.up, "go.mod", "module example.org/up\n\ngo 1.26.0\n\nrequire (\n\texample.org/dep/mod v1.1.0\n\texample.org/other v0.1.0 // indirect\n)\n")
	gitT(t, f.up, "commit", "-q", "-am", "[release-v2.1] Remove b.go, rename c.go")
	gitT(t, f.up, "tag", "B")
	f.b = gitT(t, f.up, "rev-parse", "HEAD")

	short := f.a[:8]
	writeT(t, f.docs, "Cargo.toml", "[workspace]\n")
	writeT(t, f.docs, "crates/purse-testutil/src/lib.rs", "pub const DCRWALLET_PARITY_COMMIT: &str = \""+f.a+"\";\n")
	writeT(t, f.docs, "docs/specs/01-a.md", "# 01 at `"+short+"`\n\n"+
		"- same `a.go:3`, `sub/x.go:7`\n"+
		"- shifted `a.go:10-12` and `a.go:25-30`\n"+
		"- touched `a.go:20` and `a.go:28-33`\n"+
		"- gone `b.go:4`, `c.go:2`\n"+
		"- invalid `a.go:61`\n"+
		"- dependency `dep/mod@v1.0.0/f.go:3`\n"+
		"- list Source: a.go:40, :41-42, sub/x.go:1\n")
	writeT(t, f.docs, "docs/QUIRKS.md", "# Quirks at `"+short+"`\n\n## QK-0001 — x\n\nSpec: 01. Source: a.go:40-41\n\nText (`:50`).\n")
	writeT(t, f.docs, "docs/adr/0001-x.md", "# ADR at `"+short+"`\n\n`a.go:10-12`\n")
	writeT(t, f.docs, "docs/specs/02-b.md", "# 02, naming no pin\n\n`a.go:10`\n")
	writeT(t, f.docs, "README.md", "Pin `"+short+"`; see `a.go:11`.\n")
	gitT(t, f.docs, "init", "-q")
	gitT(t, f.docs, "add", "-A")
	gitT(t, f.docs, "commit", "-q", "-m", "docs")
	return f
}

func TestAnalyzeSynthetic(t *testing.T) {
	f := newFixture(t)
	a, err := Analyze(f.docs, f.up, f.a, "B", nil)
	if err != nil {
		t.Fatal(err)
	}
	if len(a.Commits) != 2 || a.Commits[0].Side != "+" || len(a.Backports) != 0 {
		t.Errorf("commits: %+v", a.Commits)
	}
	if len(a.ModChanges) != 1 || a.ModChanges[0] != (ModChange{Module: "example.org/dep/mod", Old: "v1.0.0", New: "v1.1.0"}) {
		t.Errorf("go.mod delta: %+v", a.ModChanges)
	}
	if fd := a.Files["c.go"]; fd == nil || fd.Status != 'D' || fd.Renamed != "d.go" {
		t.Errorf("c.go: %+v", fd)
	}
	got := map[string]string{}
	for _, r := range a.Results {
		for j, m := range r.Maps {
			key := fmt.Sprintf("%s %s:%s", r.Doc, r.Path, itemText(r.Items[j]))
			v := m.Class.String()
			if m.Class == Shifted || m.Class == Same {
				v += " " + itemText(Item{Start: m.NewStart, End: m.NewEnd})
			}
			got[key] = v
		}
		if r.Target == TargetDependency && r.DepStatus != DepMoves {
			t.Errorf("dependency %s: %s", r.Raw, r.DepStatus)
		}
	}
	want := map[string]string{
		"docs/specs/01-a.md a.go:3":     "same 3",
		"docs/specs/01-a.md sub/x.go:7": "same 7",
		"docs/specs/01-a.md a.go:10-12": "shifted 12-14",
		"docs/specs/01-a.md a.go:25-30": "shifted 27-32", // the insertion after 30 lies outside
		"docs/specs/01-a.md a.go:20":    "touched",
		"docs/specs/01-a.md a.go:28-33": "touched", // an insertion inside, every line unchanged
		"docs/specs/01-a.md b.go:4":     "gone",
		"docs/specs/01-a.md c.go:2":     "gone",
		"docs/specs/01-a.md a.go:61":    "invalid",
		"docs/specs/01-a.md a.go:40":    "shifted 43",
		"docs/specs/01-a.md a.go:41-42": "shifted 44-45",
		"docs/specs/01-a.md sub/x.go:1": "same 1",
		"docs/QUIRKS.md a.go:40-41":     "shifted 43-44",
		"docs/QUIRKS.md a.go:50":        "shifted 53",
		"docs/adr/0001-x.md a.go:10-12": "shifted 12-14",
		"docs/specs/02-b.md a.go:10":    "shifted 12",
		"README.md a.go:11":             "shifted 13",
	}
	for k, v := range want {
		if got[k] != v {
			t.Errorf("%s: got %q, want %q", k, got[k], v)
		}
	}
	if len(got) != len(want) {
		t.Errorf("got %d items, want %d: %v", len(got), len(want), got)
	}

	var md, js bytes.Buffer
	if err := a.WriteReport(&md); err != nil {
		t.Fatal(err)
	}
	for _, s := range []string{
		"Commits: 2 (2 reachable from the new revision only",
		"`a.go:28-33` item 28-33 of `a.go`: hunks -30,0 +33,1",
		"rename detection pairs it with `d.go`",
		"the file has 60 lines at the old revision",
		"`example.org/dep/mod`: v1.0.0 -> v1.1.0",
		"- QK-0001: `a.go`",
		"| `docs/specs/01-a.md` | rewrite | 3 | 4 | 2 | 2 | 1 |",
		"## Text naming the old revision",
		"- `docs/QUIRKS.md`: lines 1\n",
		"- `docs/specs/01-a.md`: lines 1\n",
	} {
		if !strings.Contains(md.String(), s) {
			t.Errorf("report lacks %q:\n%s", s, md.String())
		}
	}
	if strings.Contains(md.String(), "`docs/adr/0001-x.md`: lines") {
		t.Error("the report lists an ADR's text naming the pin")
	}
	if err := a.WriteJSON(&js); err != nil {
		t.Fatal(err)
	}
	var back map[string]any
	if err := json.Unmarshal(js.Bytes(), &back); err != nil {
		t.Fatal(err)
	}
}

func TestCiteSynthetic(t *testing.T) {
	f := newFixture(t)
	a, err := Analyze(f.docs, f.up, f.a, "B", nil)
	if err != nil {
		t.Fatal(err)
	}
	before := map[string]string{}
	for _, d := range []string{"docs/specs/01-a.md", "docs/QUIRKS.md", "docs/adr/0001-x.md", "docs/specs/02-b.md", "README.md"} {
		before[d] = readT(t, f.docs, d)
	}
	var dry bytes.Buffer
	if err := a.Cite(&dry, false, false); err != nil {
		t.Fatal(err)
	}
	for d, s := range before {
		if readT(t, f.docs, d) != s {
			t.Errorf("a dry run changed %s", d)
		}
	}
	var out bytes.Buffer
	if err := a.Cite(&out, true, false); err != nil {
		t.Fatal(err)
	}
	short := f.a[:8]
	wantSpec := "# 01 at `" + short + "`\n\n" +
		"- same `a.go:3`, `sub/x.go:7`\n" +
		"- shifted `a.go:12-14` and `a.go:27-32`\n" +
		"- touched `a.go:20` and `a.go:28-33`\n" +
		"- gone `b.go:4`, `c.go:2`\n" +
		"- invalid `a.go:61`\n" +
		"- dependency `dep/mod@v1.0.0/f.go:3`\n" +
		"- list Source: a.go:43, :44-45, sub/x.go:1\n" +
		"\n" + citedMarker(f.b) + "\n"
	if got := readT(t, f.docs, "docs/specs/01-a.md"); got != wantSpec {
		t.Errorf("spec rewritten as:\n%s\nwant:\n%s", got, wantSpec)
	}
	if got, want := readT(t, f.docs, "docs/QUIRKS.md"), strings.NewReplacer("a.go:40-41", "a.go:43-44", "`:50`", "`:53`").Replace(before["docs/QUIRKS.md"])+"\n"+citedMarker(f.b)+"\n"; got != want {
		t.Errorf("QUIRKS rewritten as:\n%s", got)
	}
	if got := readT(t, f.docs, "README.md"); got != "Pin `"+short+"`; see `a.go:13`.\n\n"+citedMarker(f.b)+"\n" {
		t.Errorf("README rewritten as %q", got)
	}
	// ADRs are never rewritten; a document naming no pin is skipped.
	for _, d := range []string{"docs/adr/0001-x.md", "docs/specs/02-b.md"} {
		if readT(t, f.docs, d) != before[d] {
			t.Errorf("%s was rewritten", d)
		}
	}
	for _, s := range []string{
		"`docs/specs/02-b.md`: skipped, does not name the old revision",
		"Rewrote 7 shifted items in total",
		"## Touched: re-read each",
		"`docs/specs/01-a.md:5` `a.go:28-33` item 28-33",
		"## Shifted items in skipped documents",
	} {
		if !strings.Contains(out.String(), s) {
			t.Errorf("cite output lacks %q:\n%s", s, out.String())
		}
	}
	// A rerun changes nothing, before and after the first run's result is
	// committed: every document it rewrote carries the marker naming B.
	after := map[string]string{}
	for d := range before {
		after[d] = readT(t, f.docs, d)
	}
	rerun := func(when string) {
		t.Helper()
		a2, err := Analyze(f.docs, f.up, f.a, "B", nil)
		if err != nil {
			t.Fatal(err)
		}
		var out bytes.Buffer
		if err := a2.Cite(&out, true, false); err != nil {
			t.Fatalf("rerun %s: %v", when, err)
		}
		for d, s := range after {
			if got := readT(t, f.docs, d); got != s {
				t.Errorf("rerun %s changed %s:\n%s", when, d, got)
			}
		}
		// Nothing is listed for a document already rewritten: its numbers
		// are the new ones.
		if strings.Contains(out.String(), "- `docs/specs/01-a.md:") {
			t.Errorf("rerun %s lists items of a rewritten document:\n%s", when, out.String())
		}
		for _, s := range []string{
			"`docs/specs/01-a.md`: skipped, already rewritten for the new revision " + f.b[:8],
			"Rewrote 0 shifted items in total",
		} {
			if !strings.Contains(out.String(), s) {
				t.Errorf("rerun %s: output lacks %q:\n%s", when, s, out.String())
			}
		}
	}
	rerun("uncommitted")
	gitT(t, f.docs, "commit", "-q", "-am", "cite")
	rerun("committed")

	// A range that does not start where the citations are is refused
	// document by document: from A to B's parent, the marker names B.
	a3, err := Analyze(f.docs, f.up, f.a, "B~1", nil)
	if err != nil {
		t.Fatal(err)
	}
	var out3 bytes.Buffer
	if err := a3.Cite(&out3, true, false); err != nil {
		t.Fatal(err)
	}
	if want := "`docs/QUIRKS.md`: skipped, its citations are at " + f.b[:8]; !strings.Contains(out3.String(), want) {
		t.Errorf("output lacks %q:\n%s", want, out3.String())
	}
	for d, s := range after {
		if readT(t, f.docs, d) != s {
			t.Errorf("a range from the wrong revision changed %s", d)
		}
	}

	// Other edits in a covered document still stop a write.
	writeT(t, f.docs, "docs/specs/01-a.md", before["docs/specs/01-a.md"])
	a4, err := Analyze(f.docs, f.up, f.a, "B", nil)
	if err != nil {
		t.Fatal(err)
	}
	if err := a4.Cite(&bytes.Buffer{}, true, false); err == nil || !strings.Contains(err.Error(), "uncommitted changes") {
		t.Errorf("a dirty document: %v", err)
	}
}

// upstreamRepo builds a synthetic upstream from a list of revisions, each a
// map of path to content (a "-> target" content makes a symbolic link),
// and returns the commits.
func upstreamRepo(t *testing.T, revs ...map[string]string) (string, []string) {
	t.Helper()
	isolateGit(t)
	dir := t.TempDir()
	gitT(t, dir, "init", "-q")
	var commits []string
	for i, rev := range revs {
		for name, content := range rev {
			fn := filepath.Join(dir, filepath.FromSlash(name))
			_ = os.Remove(fn)
			if target, ok := strings.CutPrefix(content, "-> "); ok {
				if err := os.Symlink(target, fn); err != nil {
					t.Skipf("symbolic links: %v", err)
				}
				continue
			}
			writeT(t, dir, name, content)
		}
		gitT(t, dir, "add", "-A")
		gitT(t, dir, "commit", "-q", "--allow-empty", "-m", fmt.Sprint("rev ", i))
		commits = append(commits, gitT(t, dir, "rev-parse", "HEAD"))
	}
	return dir, commits
}

// docsRepo writes documents (each naming the pin) into a new repository.
func docsRepo(t *testing.T, docs map[string]string) string {
	t.Helper()
	dir := t.TempDir()
	writeT(t, dir, "Cargo.toml", "[workspace]\n")
	for name, content := range docs {
		writeT(t, dir, name, content)
	}
	gitT(t, dir, "init", "-q")
	gitT(t, dir, "add", "-A")
	gitT(t, dir, "commit", "-q", "-m", "docs")
	return dir
}

// TestReplaceMovesDependency: a change of a replace directive alone moves
// the module a build uses, as dcrwallet's 183083d6, 92529a4b and 99be2039
// did with their require lines unchanged.
func TestReplaceMovesDependency(t *testing.T) {
	const req = "module example.org/up\n\ngo 1.26.0\n\nrequire (\n\texample.org/dep v1.0.0\n\texample.org/still v1.0.0\n)\n"
	up, c := upstreamRepo(t,
		map[string]string{"go.mod": req, "a.go": "package up\n"},
		map[string]string{"go.mod": req + "\nreplace example.org/dep => example.org/dep v1.5.0\n"},
		map[string]string{"go.mod": req + "\nreplace example.org/dep v1.0.0 => ../dep\n"},
		map[string]string{"go.mod": req},
	)
	doc := func(ver string) string {
		return "# at `" + c[0][:8] + "`\n\n`dep@" + ver + "/x.go:5`, `still@v1.0.0/y.go:7`\n"
	}
	cases := []struct {
		from, to     int
		cited        string
		want         string
		wantReplaces int
	}{
		{0, 1, "v1.0.0", DepMoves, 1}, // a replace added, require unchanged
		{1, 3, "v1.5.0", DepMoves, 1}, // the replace removed; the pin builds v1.5.0
		{1, 3, "v1.0.0", DepMoves, 1}, // cites the require line, not what is built
		{0, 2, "v1.0.0", DepMoves, 1}, // a local directory counts as a move
		{0, 3, "v1.0.0", DepUnchanged, 0},
		{1, 1, "v1.0.0", DepOtherVer, 0}, // the pin builds v1.5.0
		{1, 1, "v1.5.0", DepUnchanged, 0},
	}
	for _, tc := range cases {
		docs := docsRepo(t, map[string]string{"docs/specs/01-x.md": doc(tc.cited)})
		a, err := Analyze(docs, up, c[tc.from], c[tc.to], nil)
		if err != nil {
			t.Fatal(err)
		}
		got := map[string]string{}
		for _, r := range a.Results {
			if r.Target == TargetDependency && len(r.Items) > 0 {
				got[r.DepModule] = r.DepStatus
			}
		}
		if got["example.org/dep"] != tc.want || got["example.org/still"] != DepUnchanged {
			t.Errorf("%d..%d citing %s: %v, want dep %s", tc.from, tc.to, tc.cited, got, tc.want)
		}
		if len(a.Replaces) != tc.wantReplaces {
			t.Errorf("%d..%d: replaces %v", tc.from, tc.to, a.Replaces)
		}
		if tc.want != DepMoves {
			continue
		}
		if len(a.ModChanges) != 1 || a.ModChanges[0].Module != "example.org/dep" || !a.ModChanges[0].Moves() {
			t.Errorf("%d..%d: changes %+v", tc.from, tc.to, a.ModChanges)
		}
		var md bytes.Buffer
		if err := a.WriteReport(&md); err != nil {
			t.Fatal(err)
		}
		if !strings.Contains(md.String(), "`dep@"+tc.cited+"/x.go:5`: `example.org/dep` ") {
			t.Errorf("%d..%d: the report does not list the citation:\n%s", tc.from, tc.to, md.String())
		}
	}
}

// TestTypeChange: a file replaced by a symbolic link is gone, never mapped
// through the link's one-line diff.
func TestTypeChange(t *testing.T) {
	up, c := upstreamRepo(t,
		map[string]string{"a.go": join(lines("a", 30)), "link.go": join(lines("l", 30))},
		map[string]string{"link.go": "-> a.go"},
	)
	files, err := Git{Dir: up}.Diff(c[0], c[1])
	if err != nil {
		t.Fatal(err)
	}
	if fd := files["link.go"]; fd == nil || fd.Status != 'D' || !fd.TypeChange {
		t.Fatalf("link.go: %+v", fd)
	}
	docs := docsRepo(t, map[string]string{"docs/specs/01-x.md": "# at `" + c[0][:8] + "`\n\n`link.go:5-6`\n"})
	a, err := Analyze(docs, up, c[0], c[1], nil)
	if err != nil {
		t.Fatal(err)
	}
	var out bytes.Buffer
	if err := a.Cite(&out, true, false); err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(out.String(), "`link.go:5-6` item 5-6 of `link.go`: file replaced by an object of another type") {
		t.Errorf("cite output:\n%s", out.String())
	}
	if got := readT(t, docs, "docs/specs/01-x.md"); !strings.Contains(got, "`link.go:5-6`") {
		t.Errorf("rewritten: %q", got)
	}
}

// TestShiftAcrossDeclarations: the Myers diff aligns the end of A with the
// identical end of an appended B; the shifted item is listed, not
// rewritten.  A changed signature of the same function is not flagged.
func TestShiftAcrossDeclarations(t *testing.T) {
	up, c := upstreamRepo(t,
		map[string]string{"sub/w.go": "package w\n\nfunc A() error {\n\tx()\n\treturn nil\n}\n",
			"sub/s.go": "package w\n\nfunc S(a int) {\n\tp()\n\tq()\n}\n"},
		map[string]string{"sub/w.go": "package w\n\nfunc A() error {\n\ty()\n\treturn errFoo\n}\n\nfunc B() error {\n\tx()\n\treturn nil\n}\n",
			"sub/s.go": "package w\n\n// S does p and q.\nfunc S(a, b int) {\n\tp()\n\tq()\n}\n"},
	)
	spec := "# at `" + c[0][:8] + "`\n\n`A` returns nil at its end (`sub/w.go:5`); its body calls `x` (`sub/w.go:4-5`). `S` calls `q` (`sub/s.go:5`).\n"
	docs := docsRepo(t, map[string]string{"docs/specs/01-x.md": spec})
	a, err := Analyze(docs, up, c[0], c[1], nil)
	if err != nil {
		t.Fatal(err)
	}
	got := map[string]Mapping{}
	for _, r := range a.Results {
		for j, m := range r.Maps {
			got[r.Path+":"+itemText(r.Items[j])] = m
		}
	}
	for _, k := range []string{"sub/w.go:5", "sub/w.go:4-5"} {
		if m := got[k]; m.Class != Shifted || !strings.Contains(m.Review, "`func A`") || !strings.Contains(m.Review, "`func B`") {
			t.Errorf("%s: %+v", k, m)
		}
	}
	if m := got["sub/s.go:5"]; m.Class != Shifted || m.NewStart != 6 || m.Review != "" {
		t.Errorf("sub/s.go:5: %+v", m)
	}
	var out bytes.Buffer
	if err := a.Cite(&out, true, false); err != nil {
		t.Fatal(err)
	}
	want := strings.Replace(spec, "`sub/s.go:5`", "`sub/s.go:6`", 1) + "\n" + citedMarker(c[1]) + "\n"
	if got := readT(t, docs, "docs/specs/01-x.md"); got != want {
		t.Errorf("rewritten as:\n%s\nwant:\n%s", got, want)
	}
	if !strings.Contains(out.String(), "## Shifted, but the enclosing declaration differs") ||
		!strings.Contains(out.String(), "`sub/w.go:4-5` item 4-5 of `sub/w.go`: would become 9-10, but line 4 is in `func A` opened at line 3 (line 3 at the new revision), line 9 in `func B` opened at line 8") {
		t.Errorf("cite output:\n%s", out.String())
	}
}

// TestReviewList: the review list leaves out docs/UPSTREAM.md and a bare
// mention of a name several upstream files share ("the `go.mod` delta").
func TestReviewList(t *testing.T) {
	f := newFixture(t)
	writeT(t, f.docs, "docs/UPSTREAM.md", "# Notes\n\n`a.go:10` and `go.mod` at `"+f.a[:8]+"`\n")
	writeT(t, f.docs, "README.md", "Pin `"+f.a[:8]+"`; the `go.mod` delta.\n")
	writeT(t, f.docs, "docs/specs/03-c.md", "# 03\n\nThe `sub/go.mod` is not upstream; `go.mod:3` is.\n")
	a, err := Analyze(f.docs, f.up, f.a, "B", nil)
	if err != nil {
		t.Fatal(err)
	}
	var md bytes.Buffer
	if err := a.WriteReport(&md); err != nil {
		t.Fatal(err)
	}
	list := md.String()[strings.Index(md.String(), "## Review list by document"):]
	for _, d := range []string{"docs/UPSTREAM.md", "README.md"} {
		if strings.Contains(list, "\n- `"+d+"`") {
			t.Errorf("the review list names %s:\n%s", d, list)
		}
	}
	for _, d := range []string{"docs/specs/01-a.md", "docs/specs/03-c.md"} {
		if !strings.Contains(list, "\n- `"+d+"`") {
			t.Errorf("the review list lacks %s:\n%s", d, list)
		}
	}
}

func TestRunCommands(t *testing.T) {
	f := newFixture(t)
	dir := t.TempDir()
	var out, errb bytes.Buffer
	js := filepath.Join(dir, "r.json")
	// -from defaults to DCRWALLET_PARITY_COMMIT.
	if err := run([]string{"report", "-repo", f.docs, "-upstream", f.up, "-to", "B", "-json", js}, &out, &errb); err != nil {
		t.Fatalf("report: %v\n%s", err, errb.String())
	}
	if !strings.HasPrefix(out.String(), "# Upstream range "+f.a[:8]+".."+f.b[:8]) {
		t.Errorf("report: %.200s", out.String())
	}
	if _, err := os.Stat(js); err != nil {
		t.Error(err)
	}
	if !strings.Contains(out.String(), "- Frozen files: not computed (no oracle module).") {
		t.Errorf("report without an oracle module: %.600s", out.String())
	}
	if err := run([]string{"batteries", "-repo", f.docs, "-upstream", f.up}, &out, &errb); err == nil || !strings.Contains(err.Error(), "no Go module") {
		t.Errorf("batteries without an oracle module: %v", err)
	}
	out.Reset()
	if err := run([]string{"survey", "-repo", f.docs, "-upstream", f.up}, &out, &errb); err != nil {
		t.Fatalf("survey: %v", err)
	}
	if !strings.Contains(out.String(), "## Share") {
		t.Errorf("survey: %.300s", out.String())
	}
	out.Reset()
	if err := run([]string{"cite", "-repo", f.docs, "-upstream", f.up, "-to", "B"}, &out, &errb); err != nil {
		t.Fatalf("cite: %v", err)
	}
	if !strings.Contains(out.String(), "Would rewrite 7 shifted items") {
		t.Errorf("cite: %.300s", out.String())
	}
	if err := run([]string{"report", "-repo", f.docs, "-upstream", f.up, "-to", "nonexistent"}, &out, &errb); err == nil {
		t.Error("a missing revision was accepted")
	}
	if err := run([]string{"bogus"}, &out, &errb); err == nil {
		t.Error("an unknown command was accepted")
	}
}

// TestClassifyAgainstGit edits a file at random, commits both revisions,
// and checks classify on the diff git produces against the truth the edit
// knows: every line is unique and new lines are fresh, so the unchanged
// lines are exactly the longest common subsequence and any correct diff
// pairs them as the edit did.
func TestClassifyAgainstGit(t *testing.T) {
	isolateGit(t)
	dir := t.TempDir()
	gitT(t, dir, "init", "-q")
	rng := rand.New(rand.NewSource(1))
	fresh := 0
	for iter := 0; iter < 25; iter++ {
		const n = 80
		old := lines(fmt.Sprintf("it%d", iter), n)
		// prov[i] is the old line (1-based) at new position i+1, or 0.
		cur := append([]string{}, old...)
		prov := make([]int, n)
		for i := range prov {
			prov[i] = i + 1
		}
		for op := 0; op < 1+rng.Intn(6); op++ {
			pos := rng.Intn(len(cur) + 1)
			switch rng.Intn(3) {
			case 0: // insert
				k := 1 + rng.Intn(3)
				var ins []string
				var insp []int
				for j := 0; j < k; j++ {
					fresh++
					ins = append(ins, fmt.Sprintf("fresh %d", fresh))
					insp = append(insp, 0)
				}
				cur = append(cur[:pos], append(ins, cur[pos:]...)...)
				prov = append(prov[:pos], append(insp, prov[pos:]...)...)
			case 1: // delete
				if pos == len(cur) {
					continue
				}
				k := min(1+rng.Intn(3), len(cur)-pos)
				cur = append(cur[:pos], cur[pos+k:]...)
				prov = append(prov[:pos], prov[pos+k:]...)
			default: // replace
				if pos == len(cur) {
					continue
				}
				fresh++
				cur[pos] = fmt.Sprintf("fresh %d", fresh)
				prov[pos] = 0
			}
		}
		writeT(t, dir, "f.go", join(old))
		gitT(t, dir, "add", "f.go")
		gitT(t, dir, "commit", "-q", "--allow-empty", "-m", "old")
		from := gitT(t, dir, "rev-parse", "HEAD")
		writeT(t, dir, "f.go", join(cur))
		gitT(t, dir, "commit", "-q", "--allow-empty", "-am", "new")
		to := gitT(t, dir, "rev-parse", "HEAD")
		files, err := Git{Dir: dir}.Diff(from, to)
		if err != nil {
			t.Fatal(err)
		}
		where := map[int]int{} // old line -> new line
		for i, p := range prov {
			if p > 0 {
				where[p] = i + 1
			}
		}
		for s := 1; s <= n; s++ {
			for e := s; e <= n && e < s+12; e++ {
				want := Same
				for l := s; l <= e; l++ {
					if where[l] == 0 || (l > s && where[l] != where[l-1]+1) {
						want = Touched
						break
					}
				}
				if want == Same && where[s] != s {
					want = Shifted
				}
				m := classify(files["f.go"], n, s, e)
				if m.Class != want || (want != Touched && (m.NewStart != where[s] || m.NewEnd != where[e])) {
					t.Fatalf("iteration %d, %d-%d: got %v %d-%d, want %v %d-%d", iter, s, e, m.Class, m.NewStart, m.NewEnd, want, where[s], where[e])
				}
			}
		}
	}
}

// TestSameNameDeclarations: a shifted item the diff moves into another
// declaration of the same name (a second `func init`, a second `var (`
// block) is listed, not rewritten, and an item at unchanged numbers whose
// declaration was replaced is listed for re-reading.  In w.go and v.go the
// first declaration keeps a line (setup(), p = 0) that pairs its opening
// line with the new first declaration, while the cited lines pair with
// the second; an appended declaration that repeats the whole old one is
// paired with it whole (the diff inserts the new first one before it), and
// its lines are then rewritten, being still the old declaration's.
func TestSameNameDeclarations(t *testing.T) {
	up, c := upstreamRepo(t,
		map[string]string{
			"sub/w.go": "package w\n\nfunc init() {\n\tsetup()\n\tx()\n\tregister(a)\n}\n",
			"sub/v.go": "package w\n\nvar (\n\tp = 0\n\ta = 1\n\tb = 2\n)\n",
			"sub/u.go": "package w\n\nfunc init() {\n\tx()\n\tregister(a)\n}\n",
			"sub/s.go": "package w\n\nfunc A() error {\n\tx()\n\treturn nil\n}\n",
		},
		map[string]string{
			"sub/w.go": "package w\n\nfunc init() {\n\tsetup()\n\ty()\n\tregister(c)\n}\n\nfunc init() {\n\tx()\n\tregister(a)\n}\n",
			"sub/v.go": "package w\n\nvar (\n\tp = 0\n\tq = 1\n\tr = 2\n)\n\nvar (\n\ta = 1\n\tb = 2\n)\n",
			"sub/u.go": "package w\n\nfunc init() {\n\ty()\n\tregister(c)\n}\n\nfunc init() {\n\tx()\n\tregister(a)\n}\n",
			"sub/s.go": "package w\n\nfunc B() error {\n\tx()\n\treturn nil\n}\n\nfunc A() error {\n\ty()\n\treturn errFoo\n}\n",
		},
	)
	spec := "# at `" + c[0][:8] + "`\n\nThe init registers a (`sub/w.go:5-6`); vars (`sub/v.go:5-6`); init (`sub/u.go:4-5`). `A` calls `x` and returns nil (`sub/s.go:4-5`).\n"
	docs := docsRepo(t, map[string]string{"docs/specs/01-x.md": spec})
	a, err := Analyze(docs, up, c[0], c[1], nil)
	if err != nil {
		t.Fatal(err)
	}
	got := map[string]Mapping{}
	for _, r := range a.Results {
		for j, m := range r.Maps {
			got[r.Path+":"+itemText(r.Items[j])] = m
		}
	}
	for _, k := range []string{"sub/w.go:5-6", "sub/v.go:5-6"} {
		if m := got[k]; m.Class != Shifted || m.NewStart != 10 || !strings.Contains(m.Review, "opened at line 9") {
			t.Errorf("%s: %+v", k, m)
		}
	}
	if m := got["sub/u.go:4-5"]; m.Class != Shifted || m.NewStart != 9 || m.Review != "" {
		t.Errorf("sub/u.go:4-5: %+v", m)
	}
	if m := got["sub/s.go:4-5"]; m.Class != Same || !strings.Contains(m.Review, "`func A`") || !strings.Contains(m.Review, "`func B`") {
		t.Errorf("sub/s.go:4-5: %+v", m)
	}
	if tot := a.Total(docGroups...); tot.DeclShifted != 2 || tot.DeclSame != 1 {
		t.Errorf("totals: %+v", tot)
	}
	var md bytes.Buffer
	if err := a.WriteReport(&md); err != nil {
		t.Fatal(err)
	}
	for _, s := range []string{
		"- Items whose enclosing declaration differs at the new revision (listed, never rewritten): 2 shifted, 1 at unchanged numbers.",
		"## Citations at unchanged numbers whose enclosing declaration differs (re-read each)\n\n- `docs/specs/01-x.md:3` `sub/s.go:4-5` item 4-5 of `sub/s.go`: unchanged numbers, but line 4 is in `func A`, line 4 in `func B`",
	} {
		if !strings.Contains(md.String(), s) {
			t.Errorf("report lacks %q:\n%s", s, md.String())
		}
	}
	var out bytes.Buffer
	if err := a.Cite(&out, true, false); err != nil {
		t.Fatal(err)
	}
	if got, want := readT(t, docs, "docs/specs/01-x.md"), strings.Replace(spec, "sub/u.go:4-5", "sub/u.go:9-10", 1)+"\n"+citedMarker(c[1])+"\n"; got != want {
		t.Errorf("rewritten as:\n%s\nwant:\n%s", got, want)
	}
	for _, s := range []string{
		"`sub/w.go:5-6` item 5-6 of `sub/w.go`: would become 10-11, but line 5 is in `func init` opened at line 3 (line 3 at the new revision), line 10 in `func init` opened at line 9",
		"`sub/v.go:5-6` item 5-6 of `sub/v.go`: would become 10-11, but line 5 is in `var (` opened at line 3 (line 3 at the new revision), line 10 in `var (` opened at line 9",
		"## Unchanged numbers, but the enclosing declaration differs: re-read each\n\n- `docs/specs/01-x.md:3` `sub/s.go:4-5`",
	} {
		if !strings.Contains(out.String(), s) {
			t.Errorf("cite output lacks %q:\n%s", s, out.String())
		}
	}
}

// TestAmbiguousCitations: a name that is a file both upstream and in this
// repository ("README.md:3") is classified against the upstream file,
// counted, listed by report (and in the QUIRKS re-check list) and by cite,
// and never rewritten.
func TestAmbiguousCitations(t *testing.T) {
	readme := join(lines("readme", 10))
	up, c := upstreamRepo(t,
		map[string]string{"README.md": readme, "docs/x.md": join(lines("x", 10)), "a.go": join(lines("a", 10))},
		map[string]string{"README.md": strings.Replace(readme, "readme line 3\n", "readme line three\n", 1),
			"docs/x.md": "new\n" + join(lines("x", 10))},
	)
	short := c[0][:8]
	quirks := "# Quirks at `" + short + "`\n\n## QK-0001 — x\n\nUpstream says so (`README.md:3`); `docs/x.md:5`; `README.md:40`.\n"
	docs := docsRepo(t, map[string]string{
		"docs/QUIRKS.md": quirks,
		"docs/x.md":      "local\n",
		"README.md":      "local readme\n",
	})
	a, err := Analyze(docs, up, c[0], c[1], nil)
	if err != nil {
		t.Fatal(err)
	}
	got := map[string]string{}
	for _, r := range a.Results {
		for j, m := range r.Maps {
			if r.Target != TargetAmbiguous {
				t.Errorf("%s: target %v", r.Raw, r.Target)
			}
			got[r.Path+":"+itemText(r.Items[j])] = m.Class.String()
		}
	}
	want := map[string]string{"README.md:3": "touched", "docs/x.md:5": "shifted", "README.md:40": "invalid"}
	if fmt.Sprint(got) != fmt.Sprint(want) {
		t.Errorf("classes %v, want %v", got, want)
	}
	if amb := a.Total(docGroups...).Ambiguous; amb != (Counts{Shifted: 1, Touched: 1, Invalid: 1}) {
		t.Errorf("ambiguous totals: %+v", amb)
	}
	var md bytes.Buffer
	if err := a.WriteReport(&md); err != nil {
		t.Fatal(err)
	}
	for _, s := range []string{
		"(`README.md:77`, `go.mod:3`; classified against the upstream file, listed, never rewritten): 0 same, 1 shifted, 1 touched, 0 gone, 1 invalid.",
		"`docs/QUIRKS.md:5` `README.md:3` item 3 of `README.md`: hunks -3,1 +3,1 (`README.md` is also a file of this repository)",
		"`docs/QUIRKS.md:5` `docs/x.md:5` item 5 of `docs/x.md`: would become 6",
		"- QK-0001: `README.md`, `docs/x.md` (items: 0 same, 1 shifted, 1 touched, 0 gone)",
	} {
		if !strings.Contains(md.String(), s) {
			t.Errorf("report lacks %q:\n%s", s, md.String())
		}
	}
	// An invalid item most likely means the local file: counted, not
	// listed as wrong.
	if strings.Contains(md.String(), "`README.md:40` item") {
		t.Errorf("report lists the invalid ambiguous item:\n%s", md.String())
	}
	var out bytes.Buffer
	if err := a.Cite(&out, true, false); err != nil {
		t.Fatal(err)
	}
	if got := readT(t, docs, "docs/QUIRKS.md"); got != quirks+"\n"+citedMarker(c[1])+"\n" {
		t.Errorf("rewritten as:\n%s", got)
	}
	if !strings.Contains(out.String(), "## Citations of a file both upstream and in this repository that the range moves upstream: not rewritten; check which file each means\n\n- `docs/QUIRKS.md:5` `README.md:3`") {
		t.Errorf("cite output:\n%s", out.String())
	}
}

// TestCodeComments: citations in the comments of Rust and Go files under
// crates/, fuzz/ and tools/oracle are classified and listed, never
// rewritten; string literals, the oracle's copies of upstream source and
// frozen files' directories are not read.
func TestCodeComments(t *testing.T) {
	a0 := lines("a", 60)
	up, c := upstreamRepo(t,
		map[string]string{"wallet/a.go": join(a0)},
		map[string]string{"wallet/a.go": join(insertAfter(a0, 5, "new 1", "new 2"))},
	)
	short := c[0][:8]
	docs := docsRepo(t, map[string]string{
		"docs/specs/01-x.md": "# at `" + short + "`\n\n`wallet/a.go:10`\n",
		"crates/k/src/lib.rs": "//! Ports `wallet/a.go:10-12`.\n" +
			"fn f() -> &'static str {\n" +
			"    let s = \"wallet/a.go:20\"; // see wallet/a.go:3\n" +
			"    let r = r#\"// wallet/a.go:21\"#;\n" +
			"    let c = '\"'; /* block wallet/a.go:30\n" +
			"     * and wallet/a.go:31 */\n" +
			"    s\n}\n",
		"tools/oracle/cmd/x/main.go":               "package main\n\n// Calls wallet/a.go:40.\nvar s = `wallet/a.go:41`\n",
		"tools/oracle/internal/snaclcopy/snacl.go": "package snaclcopy\n\n// wallet/a.go:42\n",
		"crates/k/tests/data/x.go":                 "// wallet/a.go:43\n",
		"fuzz/fuzz_targets/t.rs":                   "// wallet/a.go:44 and wallet/a.go:99\n",
	})
	a, err := Analyze(docs, up, c[0], c[1], nil)
	if err != nil {
		t.Fatal(err)
	}
	if want := "crates/k/src/lib.rs fuzz/fuzz_targets/t.rs tools/oracle/cmd/x/main.go"; strings.Join(a.Code, " ") != want {
		t.Errorf("code files %v, want %s", a.Code, want)
	}
	got := map[string]string{}
	for _, r := range a.Results {
		if r.Group != GroupCode {
			continue
		}
		for j, m := range r.Maps {
			got[fmt.Sprintf("%s:%d %s", r.Doc, r.Line, itemText(r.Items[j]))] = m.Class.String()
		}
	}
	want := map[string]string{
		"crates/k/src/lib.rs:1 10-12":     "shifted",
		"crates/k/src/lib.rs:3 3":         "same",
		"crates/k/src/lib.rs:5 30":        "shifted",
		"crates/k/src/lib.rs:6 31":        "shifted",
		"tools/oracle/cmd/x/main.go:3 40": "shifted",
		"fuzz/fuzz_targets/t.rs:1 44":     "shifted",
		"fuzz/fuzz_targets/t.rs:1 99":     "invalid",
	}
	if fmt.Sprint(got) != fmt.Sprint(want) {
		t.Errorf("code citations\n got %v\nwant %v", got, want)
	}
	var md bytes.Buffer
	if err := a.WriteReport(&md); err != nil {
		t.Fatal(err)
	}
	for _, s := range []string{
		"- Upstream line citations in code comments (3 Rust and Go files under `crates`, `fuzz`, `tools/oracle`, the oracle's copies of upstream source left out; listed, never rewritten): 1 same, 5 shifted, 0 touched, 0 gone, 1 invalid; 0 whose enclosing declaration differs.",
		"- `crates/k/src/lib.rs:1` `wallet/a.go:10-12` item 10-12 of `wallet/a.go`: would become 12-14",
		"- `fuzz/fuzz_targets/t.rs:1` `wallet/a.go:99` item 99 of `wallet/a.go`: the file has 60 lines at the old revision",
	} {
		if !strings.Contains(md.String(), s) {
			t.Errorf("report lacks %q:\n%s", s, md.String())
		}
	}
	if strings.Contains(md.String(), "| `crates/k/src/lib.rs` |") {
		t.Error("the per-document table lists a code file")
	}
	before := readT(t, docs, "crates/k/src/lib.rs")
	var out bytes.Buffer
	if err := a.Cite(&out, true, false); err != nil {
		t.Fatal(err)
	}
	if readT(t, docs, "crates/k/src/lib.rs") != before {
		t.Error("cite rewrote a code comment")
	}
	if !strings.Contains(readT(t, docs, "docs/specs/01-x.md"), "`wallet/a.go:12`") {
		t.Error("cite did not rewrite the spec")
	}
	if !strings.Contains(out.String(), "## Citations in code comments the range shifts, touches or removes (never rewritten; correct each by hand)\n\n- `crates/k/src/lib.rs:1` `wallet/a.go:10-12`") {
		t.Errorf("cite output:\n%s", out.String())
	}
}

// TestEmptyRange: after ADR-0016 item 5.3 bumps DCRWALLET_PARITY_COMMIT,
// -from defaults to the new pin; cite then refuses the empty range and
// report says it is empty, instead of reporting nothing to do.
func TestEmptyRange(t *testing.T) {
	f := newFixture(t)
	writeT(t, f.docs, "crates/purse-testutil/src/lib.rs", "pub const DCRWALLET_PARITY_COMMIT: &str = \""+f.b+"\";\n")
	var out, errb bytes.Buffer
	err := run([]string{"cite", "-repo", f.docs, "-upstream", f.up, "-to", "B", "-w", "-allow-dirty"}, &out, &errb)
	if err == nil || !strings.Contains(err.Error(), "pass the old pin as -from") {
		t.Errorf("cite over an empty range: %v\n%s", err, out.String())
	}
	if err := run([]string{"report", "-repo", f.docs, "-upstream", f.up, "-to", "B"}, &out, &errb); err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(out.String(), "**The range is empty") || !strings.Contains(errb.String(), "warning: The range is empty") {
		t.Errorf("report over an empty range:\n%s\n%s", out.String(), errb.String())
	}
	// With the old pin as -from, cite runs.
	out.Reset()
	if err := run([]string{"cite", "-repo", f.docs, "-upstream", f.up, "-from", f.a, "-to", "B"}, &out, &errb); err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(out.String(), "Would rewrite 7 shifted items") {
		t.Errorf("cite with -from: %.400s", out.String())
	}
	// batteries describes the pin itself with -from and -to one commit.
	if err := run([]string{"batteries", "-repo", f.docs, "-upstream", f.up}, &out, &errb); err == nil || !strings.Contains(err.Error(), "no Go module") {
		t.Errorf("batteries: %v", err)
	}
}
