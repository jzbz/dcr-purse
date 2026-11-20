// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

package main

import (
	"fmt"
	"io/fs"
	"os"
	"path/filepath"
	"sort"
	"strings"
)

// Group says what a document is to a pin move (ADR-0016 item 5.8).
type Group string

// Document groups.
const (
	// GroupRewrite: the specs, QUIRKS.md, PARITY.md, SKEW.md, the brief
	// and the README, whose shifted citations cite rewrites.
	GroupRewrite Group = "rewrite"
	// GroupADR: dated records; their citations stay at the pin in force on
	// their date.
	GroupADR Group = "adr"
	// GroupDated: docs/UPSTREAM.md, dated read-ahead notes, never
	// rewritten.
	GroupDated Group = "dated"
	// GroupOther: every other document under docs/; listed, not rewritten.
	GroupOther Group = "other"
	// GroupCode: comments in the Rust and Go files under codeRoots
	// (code.go); listed, never rewritten.
	GroupCode Group = "code"
)

// docGroups are the groups of documents, every group but GroupCode.
var docGroups = []Group{GroupRewrite, GroupADR, GroupDated, GroupOther}

// docGroup classifies a repository-relative document path.
func docGroup(p string) Group {
	switch {
	case isCodeFile(p):
		return GroupCode
	case strings.HasPrefix(p, "docs/adr/"):
		return GroupADR
	case p == "docs/UPSTREAM.md":
		return GroupDated
	case p == "README.md", p == "docs/QUIRKS.md", p == "docs/PARITY.md", p == "docs/SKEW.md",
		p == "docs/dcr-purse-project-brief.md":
		return GroupRewrite
	case strings.HasPrefix(p, "docs/specs/") && !strings.Contains(strings.TrimPrefix(p, "docs/specs/"), "/"):
		return GroupRewrite
	}
	return GroupOther
}

// FindDocs lists README.md and every Markdown file under docs/, sorted.
func FindDocs(repo string) ([]string, error) {
	var docs []string
	if st, err := os.Stat(filepath.Join(repo, "README.md")); err == nil && st.Mode().IsRegular() {
		docs = append(docs, "README.md")
	}
	root := filepath.Join(repo, "docs")
	err := filepath.WalkDir(root, func(p string, d fs.DirEntry, err error) error {
		if err != nil {
			return err
		}
		if d.Type().IsRegular() && strings.HasSuffix(p, ".md") {
			rel, err := filepath.Rel(repo, p)
			if err != nil {
				return err
			}
			docs = append(docs, filepath.ToSlash(rel))
		}
		return nil
	})
	if err != nil && !os.IsNotExist(err) {
		return nil, err
	}
	sort.Strings(docs)
	return docs, nil
}

// localChecker reports whether a repository-relative path is a file.
func localChecker(repo string) func(string) bool {
	return func(p string) bool {
		if p == "" || strings.HasPrefix(p, "/") || strings.HasPrefix(p, "../") || p == ".." {
			return false
		}
		st, err := os.Stat(filepath.Join(repo, filepath.FromSlash(p)))
		return err == nil && st.Mode().IsRegular()
	}
}

// modCacheDir locates the Go module cache without running go: GOMODCACHE,
// else GOPATH's first entry (or its default) plus pkg/mod.  It returns ""
// when none is found.
func modCacheDir() string {
	if d := os.Getenv("GOMODCACHE"); d != "" {
		return d
	}
	gp := os.Getenv("GOPATH")
	if gp == "" {
		home, err := os.UserHomeDir()
		if err != nil {
			return ""
		}
		gp = filepath.Join(home, "go")
	}
	gp = filepath.SplitList(gp)[0]
	return filepath.Join(gp, "pkg", "mod")
}

// escapeModPath applies the module cache's case encoding: each upper-case
// letter becomes "!" and its lower case.
func escapeModPath(s string) string {
	var b strings.Builder
	for _, c := range s {
		if c >= 'A' && c <= 'Z' {
			b.WriteByte('!')
			c += 'a' - 'A'
		}
		b.WriteRune(c)
	}
	return b.String()
}

// modCacheChecker tells whether a file exists in a dependency module, by
// the module's extracted copy in the module cache.  A module written in
// short form ("bbolt", "cspp/v2") is matched against the old revision's
// go.mod; the answer is unknown when the module or its copy at that
// version is not found, so a report run without a populated cache
// completes names as it would without the check.
func modCacheChecker(cache string, mod GoMod) func(module, version, file string) (bool, bool) {
	return func(module, version, file string) (bool, bool) {
		if cache == "" || module == "" || version == "" {
			return false, false
		}
		full := mod.MatchModule(module)
		if full == "" {
			return false, false
		}
		dir := filepath.Join(cache, filepath.FromSlash(escapeModPath(full)+"@"+escapeModPath(version)))
		if st, err := os.Stat(dir); err != nil || !st.IsDir() {
			return false, false
		}
		st, err := os.Stat(filepath.Join(dir, filepath.FromSlash(file)))
		return err == nil && st.Mode().IsRegular(), true
	}
}

// Empty reports whether the range is empty: the two revisions are one
// commit.
func (a *Analysis) Empty() bool { return a.From.Commit == a.To.Commit }

// EmptyWarning says why an empty range is most likely a mistake: run after
// ADR-0016 item 5.3 bumps DCRWALLET_PARITY_COMMIT, -from defaults to the
// new pin, and every citation then looks unchanged.
func (a *Analysis) EmptyWarning() string {
	return fmt.Sprintf("The range is empty: the old and the new revision are both %s, so nothing in it moves any citation. "+
		"If DCRWALLET_PARITY_COMMIT has already been bumped (ADR-0016 item 5.3), -from defaults to the new pin: pass the old pin as -from.",
		a.From.Short())
}

// Rev is one end of a range.
type Rev struct {
	Spec   string `json:"spec"`
	Commit string `json:"commit"`
	Date   string `json:"date"`
	Tags   string `json:"tags,omitempty"`
}

// Short is the eight-digit abbreviation the documents use.
func (r Rev) Short() string { return r.Commit[:8] }

// Result is one citation with what the move does to it.
type Result struct {
	Citation
	Group Group     `json:"group"`
	Maps  []Mapping `json:"maps,omitempty"` // one per item, for upstream citations
	// For a dependency citation: the module it resolves to in the old
	// go.mod and what happens to it.
	DepModule string `json:"depModule,omitempty"`
	DepStatus string `json:"depStatus,omitempty"`
}

// Analysis is everything report and cite read.
type Analysis struct {
	Repo      string
	Upstream  string
	From, To  Rev
	Commits   []LogEntry
	Backports map[string]string // commit -> commit of the same subject on the other side
	Files     map[string]*FileDiff
	Docs      []string
	// Code lists the source files whose comments were read (FindCode).
	Code       []string
	Results    []Result
	Unparsed   []Unparsed
	OldMod     GoMod
	NewMod     GoMod
	ModChanges []ModChange
	Replaces   []string
	// Batteries is the frozen-file analysis, when the oracle's module was
	// found (ComputeBatteries).
	Batteries *Batteries
}

// Analyze reads the upstream range and every document.
func Analyze(repo, upstream, from, to string, docs []string) (*Analysis, error) {
	g := Git{Dir: upstream}
	a := &Analysis{Repo: repo, Upstream: upstream}
	var err error
	for _, r := range []struct {
		spec string
		dst  *Rev
	}{{from, &a.From}, {to, &a.To}} {
		c, err := g.Commit(r.spec)
		if err != nil {
			return nil, err
		}
		date, err := g.CommitDate(c)
		if err != nil {
			return nil, err
		}
		*r.dst = Rev{Spec: r.spec, Commit: c, Date: date, Tags: g.Describe(c)}
	}
	files, err := g.Files(a.From.Commit)
	if err != nil {
		return nil, err
	}
	if a.Files, err = g.Diff(a.From.Commit, a.To.Commit); err != nil {
		return nil, err
	}
	if a.Commits, err = g.Log(a.From.Commit, a.To.Commit); err != nil {
		return nil, err
	}
	a.Backports = matchBackports(a.Commits)
	for i, spec := range []string{a.From.Commit, a.To.Commit} {
		data, ok, err := g.Show(spec, "go.mod")
		if err != nil {
			return nil, err
		}
		if ok {
			if i == 0 {
				a.OldMod = ParseGoMod(data)
			} else {
				a.NewMod = ParseGoMod(data)
			}
		}
	}
	a.ModChanges, a.Replaces = ModDelta(a.OldMod, a.NewMod)

	if docs == nil {
		if docs, err = FindDocs(repo); err != nil {
			return nil, err
		}
	}
	a.Docs = docs
	res := NewResolver(files, localChecker(repo))
	res.SetDepChecker(modCacheChecker(modCacheDir(), a.OldMod))
	var cites []Citation
	for _, d := range docs {
		data, err := os.ReadFile(filepath.Join(repo, filepath.FromSlash(d)))
		if err != nil {
			return nil, err
		}
		c, u := ParseDoc(d, data, res)
		cites = append(cites, c...)
		a.Unparsed = append(a.Unparsed, u...)
	}
	// Comments in the code: their unparsed text is not kept, since the
	// survey is of the documents.
	if a.Code, err = FindCode(repo, copySkipper(RepoTable)); err != nil {
		return nil, err
	}
	for _, f := range a.Code {
		data, err := os.ReadFile(filepath.Join(repo, filepath.FromSlash(f)))
		if err != nil {
			return nil, err
		}
		c, _ := ParseDoc(f, commentText(extOf(f), data), res)
		cites = append(cites, c...)
	}

	// Line counts at the old revision of every cited upstream file.
	need := map[string]bool{}
	for i := range cites {
		if cites[i].classified() && len(cites[i].Items) > 0 {
			need[cites[i].Path] = true
		}
	}
	paths := make([]string, 0, len(need))
	for p := range need {
		paths = append(paths, p)
	}
	sort.Strings(paths)
	oldBlobs, err := g.Blobs(a.From.Commit, paths)
	if err != nil {
		return nil, err
	}

	a.Results = make([]Result, len(cites))
	// The files whose cited lines are checked for a changed declaration:
	// those the range modifies with hunks to map through.
	review := map[string]bool{}
	for i := range cites {
		r := Result{Citation: cites[i], Group: docGroup(cites[i].Doc)}
		switch {
		case r.classified():
			fd := a.Files[r.Path]
			n := lineCount(oldBlobs[r.Path])
			for _, it := range r.Items {
				m := classify(fd, n, it.Start, it.End)
				if (m.Class == Shifted || m.Class == Same) && fd != nil && fd.Status == 'M' && len(fd.Hunks) > 0 {
					review[r.Path] = true
				}
				r.Maps = append(r.Maps, m)
			}
		case r.Target == TargetDependency:
			r.DepModule, r.DepStatus = a.depStatus(r.Module, r.Version)
		}
		a.Results[i] = r
	}
	if err := a.reviewDecls(g, oldBlobs, review); err != nil {
		return nil, err
	}
	return a, nil
}

// reviewDecls compares the enclosing declaration of every shifted and
// every same item in the given files at the two revisions (decl.go) and
// records a reason on each whose declaration differs.
func (a *Analysis) reviewDecls(g Git, oldBlobs map[string][]byte, files map[string]bool) error {
	if len(files) == 0 {
		return nil
	}
	paths := make([]string, 0, len(files))
	for p := range files {
		paths = append(paths, p)
	}
	sort.Strings(paths)
	newBlobs, err := g.Blobs(a.To.Commit, paths)
	if err != nil {
		return err
	}
	oldIdx := make(map[string][]decl, len(paths))
	newIdx := make(map[string][]decl, len(paths))
	for _, p := range paths {
		oldIdx[p] = declIndex(p, oldBlobs[p])
		newIdx[p] = declIndex(p, newBlobs[p])
	}
	for i := range a.Results {
		r := &a.Results[i]
		if !r.classified() || !files[r.Path] {
			continue
		}
		hunks := a.Files[r.Path].Hunks
		for j := range r.Maps {
			if m := &r.Maps[j]; m.Class == Shifted || m.Class == Same {
				m.Review = declReview(oldIdx[r.Path], newIdx[r.Path], hunks, *m, r.Items[j].Start, r.Items[j].End)
			}
		}
	}
	return nil
}

// Dependency statuses.
const (
	DepUnchanged   = "unchanged"
	DepMoves       = "moves"
	DepOtherVer    = "version-differs-from-pin"
	DepNotRequired = "not-required-by-upstream"
)

// depStatus classifies a dependency citation by the effective version of
// its module at the two revisions (GoMod.Effective): a replace directive
// decides what dcrwallet builds as much as a require line does, and
// dcrwallet has changed replaces with the require lines left as they were
// (183083d6 added replaces of three dcrd modules; 92529a4b and 99be2039
// removed them).  A citation whose module's effective version changes
// "moves", whatever version it names, since the text it supports is about
// the module the pin builds; one that names another version than the pin
// builds, of a module that does not move, differs from the pin.
func (a *Analysis) depStatus(module, version string) (string, string) {
	mod := a.OldMod.MatchModule(module)
	if mod == "" {
		return "", DepNotRequired
	}
	oldV, _ := a.OldMod.Effective(mod)
	newV, _ := a.NewMod.Effective(mod)
	switch {
	case newV != oldV:
		return mod, DepMoves
	case version != oldV:
		return mod, DepOtherVer
	}
	return mod, DepUnchanged
}

// normalSubject strips a release-branch prefix such as "[release-v2.1] ".
func normalSubject(s string) string {
	s = strings.TrimSpace(s)
	if strings.HasPrefix(s, "[") {
		if i := strings.Index(s, "]"); i > 0 {
			s = strings.TrimSpace(s[i+1:])
		}
	}
	return s
}

// matchBackports pairs commits on opposite sides of the range whose
// subjects match once a release-branch prefix is stripped.
func matchBackports(commits []LogEntry) map[string]string {
	bySide := map[string]map[string]string{"+": {}, "-": {}}
	for _, c := range commits {
		bySide[c.Side][normalSubject(c.Subject)] = c.Hash
	}
	out := map[string]string{}
	for _, c := range commits {
		other := "-"
		if c.Side == "-" {
			other = "+"
		}
		if h, ok := bySide[other][normalSubject(c.Subject)]; ok {
			out[c.Hash] = h
		}
	}
	return out
}

// Counts tallies classes.
type Counts struct {
	Same, Shifted, Touched, Gone, Invalid int
}

func (c *Counts) add(cl Class) {
	switch cl {
	case Same:
		c.Same++
	case Shifted:
		c.Shifted++
	case Touched:
		c.Touched++
	case Gone:
		c.Gone++
	case Invalid:
		c.Invalid++
	}
}

// Total is the number of items counted.
func (c Counts) Total() int { return c.Same + c.Shifted + c.Touched + c.Gone + c.Invalid }

// DocSummary is one document's tally.
type DocSummary struct {
	Doc      string
	Group    Group
	Mapped   Counts // items of upstream citations cite maps
	BareRoot Counts // items of bare-root citations (listed, never rewritten)
	// Items of citations naming a file both upstream and in this
	// repository ("README.md:77"; listed, never rewritten).
	Ambiguous Counts
	// Same and shifted items whose enclosing declaration differs at the
	// new revision (Mapping.Review), of every kind above.
	DeclReview int
	// Upstream files the document cites (mapped, bare-root with a line, or
	// mentioned by a path that names one file) that the range changes.
	ChangedFiles []string
	Commits      []string // commits in the range touching those files
	Unresolved   int      // line citations whose path did not resolve
	DepMoves     int      // dependency citations whose requirement moves
}

// Summaries tallies per document, in document order, then per code file
// (group code).
func (a *Analysis) Summaries() []DocSummary {
	all := append(append([]string{}, a.Docs...), a.Code...)
	idx := map[string]int{}
	out := make([]DocSummary, len(all))
	for i, d := range all {
		idx[d] = i
		out[i] = DocSummary{Doc: d, Group: docGroup(d)}
	}
	files := make([]map[string]bool, len(all))
	for i := range files {
		files[i] = map[string]bool{}
	}
	for i := range a.Results {
		r := &a.Results[i]
		s := &out[idx[r.Doc]]
		switch {
		case r.Target == TargetUpstream:
			// A bare name shared by several upstream files and written
			// without a line ("the oracle's `go.mod`") says nothing about
			// which file is meant, so it does not put the document on the
			// review list.
			if a.Files[r.Path] != nil && !(r.Form == FormMention && r.How == HowBareRoot) {
				files[idx[r.Doc]][r.Path] = true
			}
			for _, m := range r.Maps {
				if r.Mapped() {
					s.Mapped.add(m.Class)
				} else {
					s.BareRoot.add(m.Class)
				}
				if m.Review != "" {
					s.DeclReview++
				}
			}
		case r.Target == TargetAmbiguous:
			// Only a line citation says which file is meant often enough
			// to put the document on the review list.
			if a.Files[r.Path] != nil && len(r.Items) > 0 {
				files[idx[r.Doc]][r.Path] = true
			}
			for _, m := range r.Maps {
				s.Ambiguous.add(m.Class)
				if m.Review != "" {
					s.DeclReview++
				}
			}
		case r.Target == TargetUnresolved && len(r.Items) > 0:
			s.Unresolved++
		case r.Target == TargetDependency && r.DepStatus == DepMoves:
			s.DepMoves++
		}
	}
	for i := range out {
		for f := range files[i] {
			out[i].ChangedFiles = append(out[i].ChangedFiles, f)
		}
		sort.Strings(out[i].ChangedFiles)
		for _, c := range a.Commits {
			for _, f := range c.Files {
				if files[i][f] {
					out[i].Commits = append(out[i].Commits, c.Hash[:8])
					break
				}
			}
		}
	}
	return out
}

// Totals tallies the classified items of a set of groups.
type Totals struct {
	Mapped    Counts `json:"mapped"`    // upstream citations cite maps
	BareRoot  Counts `json:"bareRoot"`  // bare names of root files other upstream files share
	Ambiguous Counts `json:"ambiguous"` // files both upstream and in this repository
	// DeclSame and DeclShifted count the same and shifted items, of every
	// kind above, whose enclosing declaration differs (Mapping.Review).
	DeclSame    int `json:"declSame"`
	DeclShifted int `json:"declShifted"`
}

// Total tallies the classified items of the given groups (every group
// when none is given).
func (a *Analysis) Total(groups ...Group) Totals {
	want := map[Group]bool{}
	for _, g := range groups {
		want[g] = true
	}
	var t Totals
	for i := range a.Results {
		r := &a.Results[i]
		if !r.classified() || (len(want) > 0 && !want[r.Group]) {
			continue
		}
		for _, m := range r.Maps {
			switch {
			case r.Target == TargetAmbiguous:
				t.Ambiguous.add(m.Class)
			case r.Mapped():
				t.Mapped.add(m.Class)
			default:
				t.BareRoot.add(m.Class)
			}
			if m.Review != "" {
				if m.Class == Same {
					t.DeclSame++
				} else {
					t.DeclShifted++
				}
			}
		}
	}
	return t
}

// itemText writes an item as N or N-M.
func itemText(it Item) string {
	if it.Start == it.End {
		return fmt.Sprint(it.Start)
	}
	return fmt.Sprintf("%d-%d", it.Start, it.End)
}
