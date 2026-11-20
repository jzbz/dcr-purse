// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

package main

import (
	"bytes"
	"fmt"
	"io"
	"os"
	"os/exec"
	"path/filepath"
	"regexp"
	"sort"
	"strconv"
	"strings"
)

// edit replaces the bytes [off, off+n) of a document.
type edit struct {
	off, n int
	text   string
}

// DocEdits is what cite does to one document.
type DocEdits struct {
	Doc      string
	Rewrites int // items rewritten
	Skipped  string
	// Done is set when the document's marker names the new revision: its
	// numbers are already the new ones, so nothing is listed for it.
	Done  bool
	edits []edit
}

// citedMarkerRe matches the marker cite -w leaves in every document it
// covers: the upstream commit the document's citations are at.  A rerun
// over a range already applied, or a range that does not start where the
// document's citations are, then skips the document instead of shifting
// shifted numbers again.  A document without a marker is taken to be at
// the old revision of the range (the pin, before any move).
var citedMarkerRe = regexp.MustCompile(`<!--\s*pinmove-cited-at:\s*([0-9a-f]{40})\s*-->`)

// citedMarker returns the marker line for a commit.
func citedMarker(commit string) string {
	return "<!-- pinmove-cited-at: " + commit + " -->"
}

// setCitedMarker replaces the document's marker, or appends one.
func setCitedMarker(data []byte, commit string) []byte {
	if loc := citedMarkerRe.FindIndex(data); loc != nil {
		out := append([]byte{}, data[:loc[0]]...)
		out = append(out, citedMarker(commit)...)
		return append(out, data[loc[1]:]...)
	}
	out := append([]byte{}, data...)
	if len(out) > 0 && out[len(out)-1] != '\n' {
		out = append(out, '\n')
	}
	return append(out, "\n"+citedMarker(commit)+"\n"...)
}

// CitePlan computes the rewrites of every shifted item of a mapped upstream
// citation in the documents of group rewrite, except those whose enclosing
// declaration differs (Mapping.Review).  A document is skipped, with the
// reason, when its marker (citedMarkerRe) names another commit than the
// old revision, when it does not name the old revision's eight-digit
// abbreviation (cite runs before the move updates the text naming the pin,
// ADR-0016 item 5.8 before 5.12), or when it already names the new one.
func (a *Analysis) CitePlan() []*DocEdits {
	by := map[string]*DocEdits{}
	var order []string
	for _, d := range a.Docs {
		if docGroup(d) != GroupRewrite {
			continue
		}
		by[d] = &DocEdits{Doc: d}
		order = append(order, d)
		data, err := os.ReadFile(filepath.Join(a.Repo, filepath.FromSlash(d)))
		var at string
		if m := citedMarkerRe.FindSubmatch(data); m != nil {
			at = string(m[1])
		}
		switch {
		case err != nil:
			by[d].Skipped = err.Error()
		case at != "" && at == a.To.Commit && a.To.Commit != a.From.Commit:
			by[d].Skipped = "already rewritten for the new revision " + a.To.Short() + " (its pinmove-cited-at marker)"
			by[d].Done = true
		case at != "" && at != a.From.Commit:
			by[d].Skipped = "its citations are at " + at[:8] + " (its pinmove-cited-at marker), not at the old revision " + a.From.Short()
		case !bytes.Contains(data, []byte(a.From.Short())):
			by[d].Skipped = "does not name the old revision " + a.From.Short()
		case a.To.Commit != a.From.Commit && bytes.Contains(data, []byte(a.To.Short())):
			by[d].Skipped = "already names the new revision " + a.To.Short()
		}
	}
	for i := range a.Results {
		r := &a.Results[i]
		de := by[r.Doc]
		if de == nil || de.Skipped != "" || !r.Mapped() {
			continue
		}
		for j, m := range r.Maps {
			if m.Class != Shifted || m.Review != "" {
				continue
			}
			it := r.Items[j]
			de.edits = append(de.edits, edit{it.StartOff, it.StartLen, strconv.Itoa(m.NewStart)})
			if it.EndOff >= 0 {
				de.edits = append(de.edits, edit{it.EndOff, it.EndLen, strconv.Itoa(m.NewEnd)})
			}
			de.Rewrites++
		}
	}
	out := make([]*DocEdits, 0, len(order))
	for _, d := range order {
		out = append(out, by[d])
	}
	return out
}

// apply returns data with the edits applied.
func applyEdits(data []byte, edits []edit) ([]byte, error) {
	sort.Slice(edits, func(i, j int) bool { return edits[i].off < edits[j].off })
	var b bytes.Buffer
	prev := 0
	for _, e := range edits {
		if e.off < prev || e.off+e.n > len(data) {
			return nil, fmt.Errorf("overlapping or out-of-range edit at byte %d", e.off)
		}
		for k := e.off; k < e.off+e.n; k++ {
			if !isDigit(data[k]) {
				return nil, fmt.Errorf("edit at byte %d does not replace a number", e.off)
			}
		}
		b.Write(data[prev:e.off])
		b.WriteString(e.text)
		prev = e.off + e.n
	}
	b.Write(data[prev:])
	return b.Bytes(), nil
}

// dirtyDocs lists the documents with uncommitted changes in the repository
// (git status), so that cite -w never mixes its rewrite with other edits
// and the rewrite can be reviewed as a diff of its own.
func dirtyDocs(repo string, docs []string) ([]string, error) {
	if len(docs) == 0 {
		return nil, nil
	}
	args := append([]string{"-C", repo, "status", "--porcelain=v1", "-z", "--untracked-files=all", "--"}, docs...)
	cmd := exec.Command("git", args...)
	var out, errb bytes.Buffer
	cmd.Stdout, cmd.Stderr = &out, &errb
	if err := cmd.Run(); err != nil {
		return nil, fmt.Errorf("git status in %s: %v: %s", repo, err, strings.TrimSpace(errb.String()))
	}
	var dirty []string
	for _, rec := range strings.Split(out.String(), "\x00") {
		if len(rec) > 3 {
			dirty = append(dirty, rec[3:])
		}
	}
	return dirty, nil
}

// Cite plans and, when write is set, applies the rewrites, then writes the
// list a human works through: every touched, gone and invalid item, the
// bare-root and upstream-and-local items the range moves, the items whose
// enclosing declaration differs and the unresolved line citations, in the
// documents cite covers, and the citations in code comments the range
// moves, which are never rewritten.
//
// An empty range is refused: it rewrites nothing and lists nothing, which
// reads as nothing to do, and the likely cause is a run after
// DCRWALLET_PARITY_COMMIT was bumped with -from left to its default.
func (a *Analysis) Cite(w io.Writer, write, allowDirty bool) error {
	if a.Empty() {
		return fmt.Errorf("refusing an empty range: %s", a.EmptyWarning())
	}
	plan := a.CitePlan()
	if write && !allowDirty {
		var docs []string
		for _, de := range plan {
			if de.Skipped == "" {
				docs = append(docs, de.Doc)
			}
		}
		dirty, err := dirtyDocs(a.Repo, docs)
		if err != nil {
			return err
		}
		if len(dirty) > 0 {
			return fmt.Errorf("documents with uncommitted changes (commit them, or pass -allow-dirty): %s", strings.Join(dirty, ", "))
		}
	}
	p := func(format string, args ...any) { fmt.Fprintf(w, format, args...) }
	verb := "Would rewrite"
	if write {
		verb = "Rewrote"
	}
	p("# Citations %s..%s\n\n", a.From.Short(), a.To.Short())
	total, marked := 0, 0
	for _, de := range plan {
		if de.Skipped != "" {
			p("- `%s`: skipped, %s\n", de.Doc, de.Skipped)
			continue
		}
		marked++
		if write {
			fn := filepath.Join(a.Repo, filepath.FromSlash(de.Doc))
			data, err := os.ReadFile(fn)
			if err != nil {
				return err
			}
			out, err := applyEdits(data, de.edits)
			if err != nil {
				return fmt.Errorf("%s: %v", de.Doc, err)
			}
			out = setCitedMarker(out, a.To.Commit)
			st, err := os.Stat(fn)
			if err != nil {
				return err
			}
			if err := os.WriteFile(fn, out, st.Mode().Perm()); err != nil {
				return err
			}
		}
		if de.Rewrites > 0 {
			p("- `%s`: %s %d shifted items\n", de.Doc, strings.ToLower(verb[:1])+verb[1:], de.Rewrites)
		}
		total += de.Rewrites
	}
	p("\n%s %d shifted items in total. ADRs, docs/UPSTREAM.md and the other documents are never rewritten.", verb, total)
	if marked > 0 {
		mverb := "would get"
		if write {
			mverb = "got"
		}
		p(" Each of the %d documents covered %s the marker `%s`, so that a rerun skips it.", marked, mverb, citedMarker(a.To.Commit))
	}
	p("\n\n")

	inScope := func(r *Result) bool {
		for _, de := range plan {
			if de.Doc == r.Doc {
				return !de.Done
			}
		}
		return false
	}
	a.writeItems(w, "Touched: re-read each and correct the text it supports", func(r *Result, m Mapping) bool {
		return inScope(r) && r.Mapped() && m.Class == Touched
	})
	a.writeItems(w, "Gone: re-locate each", func(r *Result, m Mapping) bool {
		return inScope(r) && r.Mapped() && m.Class == Gone
	})
	a.writeItems(w, "Invalid already at the old revision: correct each", func(r *Result, m Mapping) bool {
		return inScope(r) && r.Target != TargetAmbiguous && m.Class == Invalid
	})
	a.writeItems(w, "Bare-root citations the range moves: not rewritten; check which file each means", func(r *Result, m Mapping) bool {
		return inScope(r) && r.Target == TargetUpstream && !r.Mapped() && (movedClass(m) || m.Review != "")
	})
	a.writeItems(w, "Citations of a file both upstream and in this repository that the range moves upstream: not rewritten; check which file each means", func(r *Result, m Mapping) bool {
		return inScope(r) && r.Target == TargetAmbiguous && (movedClass(m) || m.Review != "")
	})
	rewritten := func(r *Result, m Mapping) bool {
		if !r.Mapped() || m.Class != Shifted || m.Review != "" {
			return false
		}
		for _, de := range plan {
			if de.Doc == r.Doc {
				return de.Skipped == ""
			}
		}
		return false
	}
	a.writeItems(w, "Shifted, but the enclosing declaration differs: not rewritten; re-locate each", func(r *Result, m Mapping) bool {
		return inScope(r) && r.Mapped() && m.Class == Shifted && m.Review != ""
	})
	a.writeItems(w, "Unchanged numbers, but the enclosing declaration differs: re-read each", func(r *Result, m Mapping) bool {
		return inScope(r) && r.Mapped() && m.Class == Same && m.Review != ""
	})
	a.writeItems(w, "Rewritten by the root rule: check that each bare name means the upstream root file", func(r *Result, m Mapping) bool {
		return r.How == HowRoot && rewritten(r, m)
	})
	a.writeItems(w, "Rewritten with a path from the section or sub-heading: check that each means that file", func(r *Result, m Mapping) bool {
		return r.How == HowSection && rewritten(r, m)
	})
	a.writeItems(w, "Rewritten with a path completed from a list: check that each means that file", func(r *Result, m Mapping) bool {
		return r.How == HowInherited && rewritten(r, m)
	})
	a.writeItems(w, "Rewritten numeric code spans: check that each is a line, not a value", func(r *Result, m Mapping) bool {
		return r.Form == FormNumericSpan && rewritten(r, m)
	})
	a.writeItems(w, "Shifted items in skipped documents: not rewritten", func(r *Result, m Mapping) bool {
		if !r.Mapped() || m.Class != Shifted {
			return false
		}
		for _, de := range plan {
			if de.Doc == r.Doc {
				return de.Skipped != "" && !de.Done
			}
		}
		return false
	})
	n := 0
	for i := range a.Results {
		r := &a.Results[i]
		if inScope(r) && r.Target == TargetUnresolved && len(r.Items) > 0 {
			n++
		}
	}
	p("## Unresolved line citations\n\n%d line citations in these documents did not resolve to a file (see `pinmove survey`); none is rewritten.\n\n", n)
	a.writeCodeItems(w)
	return nil
}
