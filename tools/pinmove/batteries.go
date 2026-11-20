// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

package main

// The batteries command (ADR-0016, Decision items 5.1, 5.6 and 6): which
// frozen files a move must regenerate, by package, for every Go-generated
// file whether or not it names the pin, and which files name the pin.
//
// A battery is a set of frozen files under crates/<crate>/tests/data
// written by one regenerator (or copied by hand).  The table (RepoTable)
// says which oracle commands a battery's regenerator runs and which
// upstream files it copies or reads; `go list -deps` says which packages
// each command links, at the version the oracle requires.  A Go-generated
// battery must be regenerated with a possible change of content when the
// range changes a non-test Go file of an upstream package its commands
// link (directly, or through one of the oracle's copies of upstream
// source), moves the go.mod requirement of a dependency module they link,
// or changes an upstream file it reads.  The closure at the old pin is
// enough: a package the new revision adds to a command's closure can only
// arrive through an import added to a package already in it, and that
// package has then changed.
//
// Every other battery tied to the pin is regenerated at each move all the
// same (ADR-0016, answer 8) and must then differ in the pin it names only
// (item 5.6); the verdict says so.

import (
	"bufio"
	"bytes"
	"encoding/json"
	"fmt"
	"io"
	"io/fs"
	"os"
	"os/exec"
	"path"
	"path/filepath"
	"regexp"
	"sort"
	"strings"
)

// Kind says how a frozen file came to be.
type Kind string

// Kinds of frozen files.
const (
	// KindGoGenerated: written by a regenerator from the Go oracle's
	// answers.
	KindGoGenerated Kind = "go-generated"
	// KindUpstreamCopy: copied from dcrwallet at the pin.
	KindUpstreamCopy Kind = "upstream-copy"
	// KindModuleCopy: copied from a dependency module at the version
	// dcrwallet requires; moves only with that requirement.
	KindModuleCopy Kind = "module-copy"
	// KindSelf: written by the crate or by hand; never moves with the pin.
	KindSelf Kind = "self"
	// KindDoc: a README describing the files beside it.
	KindDoc Kind = "doc"
)

// Battery is a set of frozen files with one way of making them.
type Battery struct {
	Crate string `json:"crate"`
	// Files are path.Match patterns relative to crates/<Crate>/tests/data.
	Files []string `json:"files"`
	Kind  Kind     `json:"kind"`
	// Commands are the oracle commands (tools/oracle/cmd/<name>) the
	// regenerator runs, built with Tags.
	Commands []string `json:"commands,omitempty"`
	Tags     string   `json:"tags,omitempty"`
	// Inputs are path.Match patterns of upstream files the battery copies
	// or reads.
	Inputs []string `json:"inputs,omitempty"`
	// Modules are the dependency modules a module copy comes from.
	Modules    []string `json:"modules,omitempty"`
	Regenerate string   `json:"regenerate,omitempty"`
	// PinCheck names the frozen test that requires the files to name
	// DCRWALLET_PARITY_COMMIT; empty if none does.
	PinCheck string `json:"pinCheck,omitempty"`
	Note     string `json:"note,omitempty"`
}

// Copy is one of the oracle's copies of upstream source.
type Copy struct {
	// Oracle is the copy's path under tools/oracle (a directory or file).
	Oracle string `json:"oracle"`
	// Package is the oracle package holding the copy, relative to the
	// oracle's module path; empty for a file no command links.
	Package string `json:"package,omitempty"`
	// Upstream lists the files copied: paths in dcrwallet (or, with
	// Module, in that module); a path ending in "/" is every non-test Go
	// file of that directory.
	Upstream   []string `json:"upstream"`
	Module     string   `json:"module,omitempty"`
	Regenerate string   `json:"regenerate"`
}

// Table is the repository's list of batteries and copies.
type Table struct {
	Batteries []Battery
	Copies    []Copy
}

// Pkg is one package of a command's dependency closure.
type Pkg struct {
	ImportPath string
	Module     string
	Version    string
	Standard   bool
}

// Lister lists what the oracle's commands link.
type Lister interface {
	// Module is the oracle's module path.
	Module() (string, error)
	// Deps is the dependency closure of tools/oracle/cmd/<cmd>.
	Deps(cmd, tags string) ([]Pkg, error)
}

// GoLister asks `go list` in the oracle's module.
type GoLister struct {
	Dir   string
	cache map[string][]Pkg
}

// Module reads the module path from go.mod.
func (l *GoLister) Module() (string, error) {
	data, err := os.ReadFile(filepath.Join(l.Dir, "go.mod"))
	if err != nil {
		return "", err
	}
	m := ParseGoMod(data)
	if m.Module == "" {
		return "", fmt.Errorf("%s/go.mod has no module directive", l.Dir)
	}
	return m.Module, nil
}

// Deps runs `go list -deps` for one command.  The module graph is read
// only (-mod=readonly), no workspace applies, and cgo is enabled so that
// a command's cgo files are listed (listing compiles nothing).
func (l *GoLister) Deps(cmd, tags string) ([]Pkg, error) {
	key := cmd + "\x00" + tags
	if p, ok := l.cache[key]; ok {
		return p, nil
	}
	args := []string{"list", "-deps", "-json=ImportPath,Module,Standard"}
	if tags != "" {
		args = append(args, "-tags", tags)
	}
	args = append(args, "./cmd/"+cmd)
	c := exec.Command("go", args...)
	c.Dir = l.Dir
	c.Env = append(os.Environ(), "GOFLAGS=-mod=readonly", "GOWORK=off", "CGO_ENABLED=1")
	var out, errb bytes.Buffer
	c.Stdout, c.Stderr = &out, &errb
	if err := c.Run(); err != nil {
		return nil, fmt.Errorf("go list -deps ./cmd/%s: %v: %s", cmd, err, strings.TrimSpace(errb.String()))
	}
	var pkgs []Pkg
	dec := json.NewDecoder(&out)
	for {
		var p struct {
			ImportPath string
			Standard   bool
			Module     *struct{ Path, Version string }
		}
		if err := dec.Decode(&p); err == io.EOF {
			break
		} else if err != nil {
			return nil, fmt.Errorf("go list -deps ./cmd/%s: %v", cmd, err)
		}
		q := Pkg{ImportPath: p.ImportPath, Standard: p.Standard}
		if p.Module != nil {
			q.Module, q.Version = p.Module.Path, p.Module.Version
		}
		pkgs = append(pkgs, q)
	}
	if l.cache == nil {
		l.cache = map[string][]Pkg{}
	}
	l.cache[key] = pkgs
	return pkgs, nil
}

// Pin forms a frozen file can name the pin in.
const (
	PinFull   = "full"     // the 40-digit commit
	PinAbbrev = "abbrev12" // its 12-digit abbreviation (as in a pseudo-version)
	PinShort  = "abbrev8"  // its 8-digit abbreviation (as the documents write it)
	PinPhrase = "phrase"   // "parity pin", no commit
	PinReadme = "readme"   // a binary file whose README.md names the commit
	pinNone   = ""         // names nothing
)

const (
	pinScanLen = 8192        // only a file's head is read: bodies are hex
	verdictSep = "; "        // between reasons
	maxListed  = 6           // changed files listed per package
	readmeName = "README.md" // the manifest a binary file may have beside it
)

// FileInfo is one frozen file.
type FileInfo struct {
	Path   string   `json:"path"`
	Binary bool     `json:"binary,omitempty"`
	Pin    string   `json:"pin,omitempty"`
	Other  []string `json:"otherCommits,omitempty"` // dcrwallet-like commits named that are not the pin
	Count  int      `json:"-"`                      // batteries matching it
}

// Verdicts.
const (
	// VerdictRegenerate: something the battery depends on changed; its
	// content may change.
	VerdictRegenerate = "regenerate"
	// VerdictPinOnly: regenerated (or re-copied) at every move; nothing it
	// depends on changed, so only the pin it names may differ.
	VerdictPinOnly = "pin-only"
	// VerdictUnchanged: not tied to the dcrwallet commit, and nothing it
	// depends on moved.
	VerdictUnchanged = "does-not-move"
	// VerdictDoc: a README.
	VerdictDoc = "document"
)

// BatteryResult is one battery under the range.
type BatteryResult struct {
	Battery
	// Group is ADR-0016's (Context, "Frozen files"): 1 Go-generated with
	// the pin checked by a test; 2 Go-generated or copied at the pin with
	// the pin not checked; 3 not tied to the dcrwallet commit; 0 a README.
	Group    int        `json:"group"`
	Matched  []FileInfo `json:"matched"`
	Upstream []string   `json:"upstream,omitempty"` // linked upstream package directories and copied files
	Linked   []string   `json:"linkedModules,omitempty"`
	Reasons  []string   `json:"reasons,omitempty"`
	Verdict  string     `json:"verdict"`
}

// CopyResult is one copy under the range.
type CopyResult struct {
	Copy
	UsedBy  []string `json:"usedBy,omitempty"` // commands linking it
	Changed []string `json:"changed,omitempty"`
	Verdict string   `json:"verdict"`
}

// Batteries is the batteries report.
type Batteries struct {
	OracleDir    string          `json:"oracleDir"`
	OracleModule string          `json:"oracleModule,omitempty"`
	Dcrwallet    string          `json:"dcrwalletVersion,omitempty"` // the version the oracle requires
	DcrwalletOK  bool            `json:"dcrwalletMatchesFrom"`
	ListError    string          `json:"listError,omitempty"`
	Results      []BatteryResult `json:"batteries"`
	Copies       []CopyResult    `json:"copies"`
	Uncovered    []string        `json:"uncovered,omitempty"` // frozen files no battery names
	Ambiguous    []string        `json:"ambiguous,omitempty"` // frozen files two batteries name
	Unmatched    []string        `json:"unmatched,omitempty"` // patterns matching no file
	Files        int             `json:"files"`
}

// FrozenFiles lists every file under crates/*/tests/data, repository
// relative and sorted.
func FrozenFiles(repo string) ([]string, error) {
	dirs, err := filepath.Glob(filepath.Join(repo, "crates", "*", "tests", "data"))
	if err != nil {
		return nil, err
	}
	var out []string
	for _, d := range dirs {
		err := filepath.WalkDir(d, func(p string, e fs.DirEntry, err error) error {
			if err != nil {
				return err
			}
			if e.Type().IsRegular() {
				rel, err := filepath.Rel(repo, p)
				if err != nil {
					return err
				}
				out = append(out, filepath.ToSlash(rel))
			}
			return nil
		})
		if err != nil {
			return nil, err
		}
	}
	sort.Strings(out)
	return out, nil
}

var (
	hex40Re     = regexp.MustCompile(`\b[0-9a-f]{40}\b`)
	dcrwallet12 = regexp.MustCompile(`dcrwallet[ ]+([0-9a-f]{12})\b`)
	phraseRe    = regexp.MustCompile(`(?i)parity pin`)
)

func isBinary(name string, head []byte) bool {
	switch path.Ext(name) {
	case ".gz", ".db", ".redb":
		return true
	}
	return bytes.IndexByte(head, 0) >= 0
}

// pinForm says how a file's head names the commit, and which other
// commits it names on a line that mentions dcrwallet or a commit (bodies
// are full of 40-digit hashes that are not commits).
func pinForm(head []byte, commit string) (string, []string) {
	var other []string
	seen := map[string]bool{}
	for _, line := range bytes.Split(head, []byte("\n")) {
		low := bytes.ToLower(line)
		if !bytes.Contains(low, []byte("dcrwallet")) && !bytes.Contains(low, []byte("commit")) {
			continue
		}
		for _, m := range hex40Re.FindAll(line, -1) {
			if s := string(m); s != commit && !seen[s] {
				seen[s] = true
				other = append(other, s)
			}
		}
		for _, m := range dcrwallet12.FindAllSubmatch(line, -1) {
			if s := string(m[1]); !strings.HasPrefix(commit, s) && !seen[s] {
				seen[s] = true
				other = append(other, s)
			}
		}
	}
	switch {
	case bytes.Contains(head, []byte(commit)):
		return PinFull, other
	case bytes.Contains(head, []byte(commit[:12])):
		return PinAbbrev, other
	case regexp.MustCompile(`\b` + regexp.QuoteMeta(commit[:8]) + `\b`).Match(head):
		return PinShort, other
	case phraseRe.Match(head):
		return PinPhrase, other
	}
	return pinNone, other
}

func readHead(fn string) ([]byte, error) {
	f, err := os.Open(fn)
	if err != nil {
		return nil, err
	}
	defer f.Close()
	buf := make([]byte, pinScanLen)
	n, err := io.ReadFull(bufio.NewReader(f), buf)
	if err != nil && err != io.ErrUnexpectedEOF && err != io.EOF {
		return nil, err
	}
	return buf[:n], nil
}

// scanFile reads a frozen file's head for the pin; a binary file takes
// the pin from a README.md beside it.
func scanFile(repo, rel, commit string) (FileInfo, error) {
	fi := FileInfo{Path: rel}
	head, err := readHead(filepath.Join(repo, filepath.FromSlash(rel)))
	if err != nil {
		return fi, err
	}
	if isBinary(rel, head) {
		fi.Binary = true
		rd, err := readHead(filepath.Join(repo, filepath.FromSlash(path.Join(path.Dir(rel), readmeName))))
		if err == nil {
			if f, _ := pinForm(rd, commit); f == PinFull || f == PinAbbrev || f == PinShort {
				fi.Pin = PinReadme
			}
		}
		return fi, nil
	}
	fi.Pin, fi.Other = pinForm(head, commit)
	return fi, nil
}

// changedPackages maps each upstream directory with a changed non-test Go
// file (outside testdata) to those files.
func changedPackages(files map[string]*FileDiff) map[string][]string {
	out := map[string][]string{}
	for p := range files {
		if !strings.HasSuffix(p, ".go") || strings.HasSuffix(p, "_test.go") {
			continue
		}
		if p == "testdata" || strings.HasPrefix(p, "testdata/") || strings.Contains(p, "/testdata/") {
			continue
		}
		d := path.Dir(p)
		out[d] = append(out[d], p)
	}
	for d := range out {
		sort.Strings(out[d])
	}
	return out
}

func listFiles(fs []string) string {
	if len(fs) > maxListed {
		return "`" + strings.Join(fs[:maxListed], "`, `") + fmt.Sprintf("` and %d more", len(fs)-maxListed)
	}
	return "`" + strings.Join(fs, "`, `") + "`"
}

// pseudoCommit returns the 12-digit commit of a pseudo-version, or "".
func pseudoCommit(v string) string {
	if i := strings.LastIndexByte(v, '-'); i >= 0 && len(v)-i-1 == 12 {
		s := v[i+1:]
		for _, c := range s {
			if (c < '0' || c > '9') && (c < 'a' || c > 'f') {
				return ""
			}
		}
		return s
	}
	return ""
}

// ComputeBatteries applies the range to the table.  A failure to list a
// command's packages is recorded, not returned: the rest of the report
// stands, and the affected batteries carry no package reasons.
func (a *Analysis) ComputeBatteries(t Table, oracleDir string, l Lister) (*Batteries, error) {
	b := &Batteries{OracleDir: oracleDir}
	files, err := FrozenFiles(a.Repo)
	if err != nil {
		return nil, err
	}
	b.Files = len(files)
	info := make(map[string]*FileInfo, len(files))
	for _, f := range files {
		fi, err := scanFile(a.Repo, f, a.From.Commit)
		if err != nil {
			return nil, err
		}
		info[f] = &fi
	}

	upMod := a.OldMod.Module
	oracleMod := ""
	if l != nil {
		if oracleMod, err = l.Module(); err != nil {
			b.ListError = err.Error()
			l = nil
		}
	}
	b.OracleModule = oracleMod
	copiesByPkg := map[string][]Copy{}
	for _, c := range t.Copies {
		if c.Package != "" {
			copiesByPkg[c.Package] = append(copiesByPkg[c.Package], c)
		}
	}
	changed := changedPackages(a.Files)
	moved := map[string]ModChange{}
	for _, m := range a.ModChanges {
		if m.Moves() {
			moved[m.Module] = m
		}
	}
	usedBy := map[string]map[string]bool{} // oracle package -> commands

	for _, bt := range t.Batteries {
		r := BatteryResult{Battery: bt}
		dataDir := path.Join("crates", bt.Crate, "tests", "data")
		for _, pat := range bt.Files {
			n := 0
			for _, f := range files {
				rel, ok := strings.CutPrefix(f, dataDir+"/")
				if !ok {
					continue
				}
				if m, _ := path.Match(pat, rel); m {
					n++
					info[f].Count++
					r.Matched = append(r.Matched, *info[f])
				}
			}
			if n == 0 {
				b.Unmatched = append(b.Unmatched, path.Join(dataDir, pat))
			}
		}

		// What the battery links.
		upstream := map[string]bool{}
		linked := map[string]string{}
		if l != nil {
			for _, cmd := range bt.Commands {
				pkgs, err := l.Deps(cmd, bt.Tags)
				if err != nil {
					if b.ListError == "" {
						b.ListError = err.Error()
					}
					continue
				}
				for _, p := range pkgs {
					switch {
					case p.Standard || p.Module == "":
					case p.Module == upMod && upMod != "":
						d := strings.TrimPrefix(strings.TrimPrefix(p.ImportPath, upMod), "/")
						if d == "" {
							d = "."
						}
						upstream[d+"/"] = true
						if b.Dcrwallet == "" {
							b.Dcrwallet = p.Version
						}
					case p.Module == oracleMod:
						rel := strings.TrimPrefix(strings.TrimPrefix(p.ImportPath, oracleMod), "/")
						if usedBy[rel] == nil {
							usedBy[rel] = map[string]bool{}
						}
						usedBy[rel][cmd] = true
						for _, c := range copiesByPkg[rel] {
							if c.Module != "" {
								continue // its module is linked through the copy's own listing
							}
							for _, u := range c.Upstream {
								upstream[u] = true
							}
						}
					default:
						linked[p.Module] = p.Version
					}
				}
			}
		}
		for u := range upstream {
			r.Upstream = append(r.Upstream, u)
		}
		sort.Strings(r.Upstream)
		for m, v := range linked {
			r.Linked = append(r.Linked, m+"@"+v)
		}
		sort.Strings(r.Linked)

		// Reasons.
		for _, u := range r.Upstream {
			if d, ok := strings.CutSuffix(u, "/"); ok {
				if fs := changed[d]; len(fs) > 0 {
					r.Reasons = append(r.Reasons, fmt.Sprintf("package `%s` changed (%s)", d, listFiles(fs)))
				}
			} else if fd := a.Files[u]; fd != nil && !upstream[path.Dir(u)+"/"] {
				r.Reasons = append(r.Reasons, fmt.Sprintf("copied file `%s` changed (%c)", u, fd.Status))
			}
		}
		for _, p := range a.changedPaths() {
			for _, in := range bt.Inputs {
				if m, _ := path.Match(in, p); m {
					r.Reasons = append(r.Reasons, fmt.Sprintf("input `%s` changed (%c)", p, a.Files[p].Status))
					break
				}
			}
		}
		var modReasons []string
		for m, v := range linked {
			if mc, ok := moved[m]; ok {
				s := fmt.Sprintf("module `%s` moves %s -> %s", m, orNone(mc.From()), orNone(mc.To()))
				if v != mc.From() {
					// The oracle's own go.mod selects another version, so
					// the move may not reach what the command links.
					s += fmt.Sprintf(" (the oracle links %s)", v)
				}
				modReasons = append(modReasons, s)
			}
		}
		for _, m := range bt.Modules {
			if mc, ok := moved[m]; ok {
				modReasons = append(modReasons, fmt.Sprintf("module `%s` moves %s -> %s", m, orNone(mc.From()), orNone(mc.To())))
			}
		}
		sort.Strings(modReasons)
		r.Reasons = append(r.Reasons, modReasons...)

		// Group and verdict.
		namesPin := false
		for _, f := range r.Matched {
			if f.Pin != pinNone {
				namesPin = true
			}
		}
		tied := bt.Kind == KindUpstreamCopy || namesPin || len(r.Upstream) > 0
		switch {
		case bt.Kind == KindDoc:
			r.Group = 0
		case bt.PinCheck != "":
			r.Group = 1
		case (bt.Kind == KindGoGenerated || bt.Kind == KindUpstreamCopy) && tied:
			r.Group = 2
		default:
			r.Group = 3
		}
		switch {
		case bt.Kind == KindDoc:
			r.Verdict = VerdictDoc
		case bt.Kind == KindSelf:
			r.Verdict = VerdictUnchanged
			r.Reasons = nil
		case len(r.Reasons) > 0:
			r.Verdict = VerdictRegenerate
		case r.Group == 1 || r.Group == 2:
			r.Verdict = VerdictPinOnly
		default:
			r.Verdict = VerdictUnchanged
		}
		b.Results = append(b.Results, r)
	}

	if b.Dcrwallet != "" {
		pc := pseudoCommit(b.Dcrwallet)
		if pc != "" {
			b.DcrwalletOK = strings.HasPrefix(a.From.Commit, pc)
		} else if c, err := (Git{Dir: a.Upstream}).Commit(b.Dcrwallet); err == nil {
			b.DcrwalletOK = c == a.From.Commit
		}
	}

	for _, c := range t.Copies {
		cr := CopyResult{Copy: c}
		for cmd := range usedBy[c.Package] {
			cr.UsedBy = append(cr.UsedBy, cmd)
		}
		sort.Strings(cr.UsedBy)
		if c.Package == "" {
			cr.UsedBy = nil
		}
		if c.Module != "" {
			if mc, ok := moved[c.Module]; ok {
				cr.Changed = append(cr.Changed, fmt.Sprintf("module `%s` moves %s -> %s", c.Module, orNone(mc.From()), orNone(mc.To())))
			}
		} else {
			for _, u := range c.Upstream {
				if d, ok := strings.CutSuffix(u, "/"); ok {
					if fs := changed[d]; len(fs) > 0 {
						cr.Changed = append(cr.Changed, fmt.Sprintf("package `%s` changed (%s)", d, listFiles(fs)))
					}
				} else if fd := a.Files[u]; fd != nil {
					cr.Changed = append(cr.Changed, fmt.Sprintf("`%s` changed (%c)", u, fd.Status))
				}
			}
		}
		if len(cr.Changed) > 0 {
			cr.Verdict = VerdictRegenerate
		} else {
			cr.Verdict = VerdictUnchanged
		}
		b.Copies = append(b.Copies, cr)
	}

	for _, f := range files {
		switch info[f].Count {
		case 0:
			b.Uncovered = append(b.Uncovered, f)
		case 1:
		default:
			b.Ambiguous = append(b.Ambiguous, f)
		}
	}
	return b, nil
}

// VerdictCounts tallies files (not batteries) by verdict.
func (b *Batteries) VerdictCounts() map[string]int {
	out := map[string]int{}
	for _, r := range b.Results {
		out[r.Verdict] += len(r.Matched)
	}
	return out
}

// CopiesChanged counts the copies the range requires regenerating.
func (b *Batteries) CopiesChanged() int {
	n := 0
	for _, c := range b.Copies {
		if c.Verdict == VerdictRegenerate {
			n++
		}
	}
	return n
}

// GroupCounts tallies files by group.
func (b *Batteries) GroupCounts() map[int]int {
	out := map[int]int{}
	for _, r := range b.Results {
		out[r.Group] += len(r.Matched)
	}
	return out
}

func pinText(f FileInfo) string {
	switch f.Pin {
	case PinFull:
		return "commit"
	case PinAbbrev:
		return "12 digits"
	case PinShort:
		return "8 digits"
	case PinPhrase:
		return "\"parity pin\" only"
	case PinReadme:
		return "README beside it"
	}
	return "no"
}

// batteryPins summarizes how a battery's files name the pin.
func batteryPins(r BatteryResult) string {
	seen := map[string]int{}
	var order []string
	for _, f := range r.Matched {
		t := pinText(f)
		if seen[t] == 0 {
			order = append(order, t)
		}
		seen[t]++
	}
	if len(order) == 1 {
		return order[0]
	}
	var parts []string
	for _, t := range order {
		parts = append(parts, fmt.Sprintf("%s (%d)", t, seen[t]))
	}
	return strings.Join(parts, ", ")
}

func batteryName(r BatteryResult) string {
	return r.Crate + ": " + strings.Join(r.Files, ", ")
}

// WriteBatteries writes the batteries section; detail lists every file.
func (a *Analysis) WriteBatteries(w io.Writer, b *Batteries, heading string, detail bool) {
	p := func(format string, args ...any) { fmt.Fprintf(w, format, args...) }
	p("%s Frozen files and regenerators\n\n", heading)
	if b == nil {
		p("Not computed.\n\n")
		return
	}
	p("Which batteries a move from `%s` to `%s` must regenerate (ADR-0016, Decision items 5.1, 5.6 and 6). ", a.From.Short(), a.To.Short())
	p("The packages each oracle command links come from `go list -deps` in `tools/oracle`, at the dcrwallet version the oracle requires.\n\n")
	if b.ListError != "" {
		p("- **The package lists are incomplete:** %s. Batteries whose commands could not be listed carry no package reasons.\n", b.ListError)
	}
	switch {
	case b.Dcrwallet == "":
		p("- The oracle's dcrwallet requirement was not seen in any listing.\n")
	case b.DcrwalletOK:
		p("- The oracle requires `%s`, the old revision.\n", b.Dcrwallet)
	default:
		p("- **The oracle requires `%s`, not the old revision `%s`;** the package lists are of the oracle's version.\n", b.Dcrwallet, a.From.Commit[:12])
	}
	vc, gc := b.VerdictCounts(), b.GroupCounts()
	p("- %d frozen files under `crates/*/tests/data`. By verdict: %d regenerate (something they depend on changed), %d pin-only (regenerated or re-copied at every move; only the pin may differ), %d do not move, %d READMEs.\n",
		b.Files, vc[VerdictRegenerate], vc[VerdictPinOnly], vc[VerdictUnchanged], vc[VerdictDoc])
	p("- By ADR-0016's groups: %d with the pin checked by a test (1), %d Go-generated or copied at the pin with the pin unchecked (2), %d not tied to the dcrwallet commit (3).\n",
		gc[1], gc[2], gc[3])
	if a.OldMod.Go != a.NewMod.Go {
		p("- The `go` directive of dcrwallet's go.mod changes (%s -> %s); the oracle sets its own `godebug` line, which a move re-reads.\n", orNone(a.OldMod.Go), orNone(a.NewMod.Go))
	}
	if len(b.Uncovered) > 0 {
		p("- **Frozen files no battery names** (enter them in `tools/pinmove/batteries_table.go`): %s.\n", listAll(b.Uncovered))
	}
	if len(b.Ambiguous) > 0 {
		p("- **Frozen files two batteries name:** %s.\n", listAll(b.Ambiguous))
	}
	if len(b.Unmatched) > 0 {
		p("- **Table patterns matching no file:** %s.\n", listAll(b.Unmatched))
	}
	p("\n| Battery | Files | Kind | Group | Names the pin | Commands | Verdict |\n|---|---|---|---|---|---|---|\n")
	for _, r := range b.Results {
		cmds := "-"
		if len(r.Commands) > 0 {
			cmds = "`" + strings.Join(r.Commands, "`, `") + "`"
			if r.Tags != "" {
				cmds += " (-tags " + r.Tags + ")"
			}
		}
		p("| %s | %d | %s | %s | %s | %s | %s |\n", mdEscape(batteryName(r)), len(r.Matched), r.Kind, groupText(r.Group),
			batteryPins(r), cmds, r.Verdict)
	}
	p("\n")

	n := 0
	for _, r := range b.Results {
		if r.Verdict != VerdictRegenerate {
			continue
		}
		if n == 0 {
			p("Regenerate, with what changed:\n\n")
		}
		n++
		p("- %s: %s. Regenerate with `%s`.\n", mdEscape(batteryName(r)), strings.Join(r.Reasons, verdictSep), r.Regenerate)
	}
	if n == 0 {
		p("Nothing any battery depends on changed.\n")
	}
	p("\n")

	p("Copies of upstream source in `tools/oracle` (item 5.4):\n\n| Copy | Upstream | Used by | Changed | Regenerate |\n|---|---|---|---|---|\n")
	for _, c := range b.Copies {
		up := "`" + strings.Join(c.Upstream, "`, `") + "`"
		if c.Module != "" {
			up = "`" + c.Module + "`: " + up
		}
		used := "-"
		if len(c.UsedBy) > 0 {
			used = "`" + strings.Join(c.UsedBy, "`, `") + "`"
		}
		ch := "no"
		if len(c.Changed) > 0 {
			ch = strings.Join(c.Changed, verdictSep)
		}
		p("| `%s` | %s | %s | %s | %s |\n", c.Oracle, up, used, ch, mdEscape(c.Regenerate))
	}
	p("\n")

	if !detail {
		return
	}
	p("%s# Every frozen file\n\n| File | Battery | Names the pin | Other commits named |\n|---|---|---|---|\n", heading)
	for _, r := range b.Results {
		for _, f := range r.Matched {
			other := ""
			if len(f.Other) > 0 {
				other = "`" + strings.Join(f.Other, "`, `") + "`"
			}
			p("| `%s` | %s | %s | %s |\n", f.Path, mdEscape(batteryName(r)), pinText(f), other)
		}
	}
	p("\n%s# Linked upstream packages and modules by battery\n\n", heading)
	for _, r := range b.Results {
		if len(r.Upstream) == 0 && len(r.Linked) == 0 {
			continue
		}
		p("- %s: upstream %s; modules %s\n", mdEscape(batteryName(r)), orDash(r.Upstream), orDash(r.Linked))
	}
	p("\n%s# Files without a checked pin (ADR-0016, answer 8)\n\n", heading)
	p("Go-generated or copied at the pin, with no frozen test requiring the pin; the uniform rule of answer 8 gives each a header naming the commit, or a manifest beside binary files, and a frozen test requiring it to equal `DCRWALLET_PARITY_COMMIT`.\n\n")
	n = 0
	for _, r := range b.Results {
		if r.Group != 2 {
			continue
		}
		n++
		var fs []string
		for _, f := range r.Matched {
			fs = append(fs, fmt.Sprintf("`%s` (%s)", f.Path, pinText(f)))
		}
		note := ""
		if r.Note != "" {
			note = "; " + r.Note
		}
		p("- %s%s\n", strings.Join(fs, ", "), mdEscape(note))
	}
	if n == 0 {
		p("None.\n")
	}
	p("\n")
}

func groupText(g int) string {
	if g == 0 {
		return "-"
	}
	return fmt.Sprint(g)
}

func listAll(xs []string) string { return "`" + strings.Join(xs, "`, `") + "`" }

func orDash(xs []string) string {
	if len(xs) == 0 {
		return "-"
	}
	return listAll(xs)
}
