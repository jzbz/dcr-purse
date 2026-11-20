// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

package main

import (
	"fmt"
	"io"
	"os"
	"path/filepath"
	"regexp"
	"sort"
	"strings"
)

// proseLine finds line references written in words ("line 12", "lines
// 3-9"), which the grammar does not accept.
var proseLine = regexp.MustCompile(`\b[Ll]ines? [0-9]+([-–][0-9]+)?\b`)

// WriteSurvey writes how the documents' citations parse: every form and
// resolution with counts, the share parsed, and every unparsed form.
func (a *Analysis) WriteSurvey(w io.Writer, examples int) error {
	p := func(format string, args ...any) { fmt.Fprintf(w, format, args...) }
	p("# Citation survey\n\n")
	p("Paths resolved against the upstream tree at %s. %d documents (README.md and docs/; code comments are not surveyed).\n\n", revLine(a.From), len(a.Docs))

	type key struct {
		form   Form
		target Target
		how    How
	}
	type tally struct{ cites, items int }
	counts := map[key]*tally{}
	var keys []key
	parsed, parsedItems, mentions := 0, 0, 0
	byTarget := map[string]int{}
	for i := range a.Results {
		r := &a.Results[i]
		if r.Group == GroupCode {
			continue
		}
		if r.Form == FormMention {
			mentions++
			continue
		}
		k := key{r.Form, r.Target, r.How}
		if counts[k] == nil {
			counts[k] = &tally{}
			keys = append(keys, k)
		}
		counts[k].cites++
		counts[k].items += len(r.Items)
		parsed++
		parsedItems += len(r.Items)
		label := r.Target.String()
		if r.Target == TargetUpstream {
			label += " (" + map[bool]string{true: "mapped", false: "bare-root"}[r.Mapped()] + ")"
		}
		byTarget[label]++
	}
	sort.Slice(keys, func(i, j int) bool {
		if counts[keys[i]].cites != counts[keys[j]].cites {
			return counts[keys[i]].cites > counts[keys[j]].cites
		}
		return fmt.Sprint(keys[i]) < fmt.Sprint(keys[j])
	})
	prose := 0
	var proseEx []string
	for _, d := range a.Docs {
		data, err := os.ReadFile(filepath.Join(a.Repo, filepath.FromSlash(d)))
		if err != nil {
			return err
		}
		for ln, line := range strings.Split(string(data), "\n") {
			for _, m := range proseLine.FindAllString(line, -1) {
				prose++
				if len(proseEx) < examples {
					proseEx = append(proseEx, fmt.Sprintf("`%s:%d` %q", d, ln+1, m))
				}
			}
		}
	}
	denom := parsed + len(a.Unparsed) + prose
	p("## Share\n\n")
	p("- Line citations parsed: %d (%d items), plus %d path mentions without a line.\n", parsed, parsedItems, mentions)
	p("- Line-citation-like text not parsed: %d (below), and %d line references in words (\"line N\", \"lines N-M\"), which the grammar does not accept and which are not all citations.\n", len(a.Unparsed), prose)
	if denom > 0 {
		p("- Share parsed: %d of %d, %.1f%% (%.1f%% leaving out the references in words).\n", parsed, denom,
			100*float64(parsed)/float64(denom), 100*float64(parsed)/float64(parsed+len(a.Unparsed)))
	}
	var tk []string
	for k := range byTarget {
		tk = append(tk, k)
	}
	sort.Strings(tk)
	var parts []string
	for _, k := range tk {
		parts = append(parts, fmt.Sprintf("%s %d", k, byTarget[k]))
	}
	p("- Parsed line citations by target: %s.\n\n", strings.Join(parts, ", "))

	p("## Parsed forms\n\n| Form | Target | Path | Citations | Items |\n|---|---|---|---|---|\n")
	for _, k := range keys {
		p("| %s | %s | %s | %d | %d |\n", mdEscape(k.form.String()), k.target, k.how, counts[k].cites, counts[k].items)
	}
	p("\n")

	p("## Unparsed forms\n\n")
	reasons := map[string][]Unparsed{}
	var rk []string
	for _, u := range a.Unparsed {
		if reasons[u.Reason] == nil {
			rk = append(rk, u.Reason)
		}
		reasons[u.Reason] = append(reasons[u.Reason], u)
	}
	sort.Slice(rk, func(i, j int) bool { return len(reasons[rk[i]]) > len(reasons[rk[j]]) })
	if len(rk) == 0 {
		p("None.\n")
	}
	for _, r := range rk {
		p("- %s: %d\n", r, len(reasons[r]))
		for i, u := range reasons[r] {
			if i == examples {
				break
			}
			p("  - `%s:%d` `%s`\n", u.Doc, u.Line, strings.ReplaceAll(u.Raw, "`", "'"))
		}
	}
	p("- line references in words: %d\n", prose)
	for _, e := range proseEx {
		p("  - %s\n", e)
	}
	p("\n")

	p("## Unresolved line citations by written path\n\n")
	type un struct {
		written string
		n       int
		hints   []string
	}
	um := map[string]*un{}
	noCtx := 0
	for i := range a.Results {
		r := &a.Results[i]
		if r.Group == GroupCode || r.Target != TargetUnresolved || len(r.Items) == 0 {
			continue
		}
		if r.Path == "" {
			noCtx++
			continue
		}
		u := um[r.Path]
		if u == nil {
			u = &un{written: r.Path, hints: r.Hints}
			um[r.Path] = u
		}
		u.n++
	}
	var ul []*un
	for _, u := range um {
		ul = append(ul, u)
	}
	sort.Slice(ul, func(i, j int) bool {
		if ul[i].n != ul[j].n {
			return ul[i].n > ul[j].n
		}
		return ul[i].written < ul[j].written
	})
	p("%d continuations or numeric spans had no path earlier in their paragraph. Written paths that resolved to nothing (bare names of files outside the upstream root, dependency paths without module@version, other repositories):\n\n", noCtx)
	for i, u := range ul {
		if i == 4*examples {
			p("- ... %d more written paths\n", len(ul)-i)
			break
		}
		h := ""
		if len(u.hints) > 0 {
			hs := u.hints
			if len(hs) > 4 {
				hs = append(hs[:4:4], fmt.Sprintf("and %d more", len(u.hints)-4))
			}
			h = " (upstream files of that name: " + strings.Join(hs, ", ") + ")"
		}
		p("- `%s`: %d%s\n", u.written, u.n, h)
	}
	return nil
}

// Proposal is a directive the survey proposes for one bare name in one
// document.
type Proposal struct {
	Doc   string
	Name  string
	Items int      // line items written with the name (continuations included)
	Value string   // the proposed path, "" when there is none
	Why   string   // why there is none, or the candidates
	Cands []string // the document's full paths with that base name
}

// Proposals finds, per document, the bare names that resolve to nothing or
// only by the bare-root rule, and proposes a directive for each name the
// document itself writes in full exactly once (as an upstream file, a
// dependency file or an unresolved relative path).  An upstream proposal
// is dropped when a cited line lies past the end of the file.
func (a *Analysis) Proposals() ([]Proposal, error) {
	type agg struct {
		items   int
		maxLine int
	}
	byDoc := map[string]map[string]*agg{}
	full := map[string]map[string]map[string]bool{} // doc -> base -> full paths
	for i := range a.Results {
		r := &a.Results[i]
		if r.Group == GroupCode {
			continue
		}
		if full[r.Doc] == nil {
			full[r.Doc] = map[string]map[string]bool{}
			byDoc[r.Doc] = map[string]*agg{}
		}
		fp := ""
		switch {
		case r.Target == TargetUpstream && (r.How == HowExplicit || r.How == HowInherited):
			fp = r.Path
		case r.Target == TargetDependency && r.How == HowExplicit:
			fp = r.Module + "@" + r.Version + "/" + r.Path
		case r.Target == TargetUnresolved && r.How == HowBare && strings.Contains(r.Path, "/") && r.Form != FormContinuation && r.Form != FormNumericSpan:
			fp = r.Path
		}
		if fp != "" {
			b := baseName(fp)
			if full[r.Doc][b] == nil {
				full[r.Doc][b] = map[string]bool{}
			}
			full[r.Doc][b][fp] = true
		}
		bare := (r.Target == TargetUnresolved && r.How == HowBare) || r.How == HowBareRoot
		if !bare || r.Path == "" || strings.Contains(r.Path, "/") || len(r.Items) == 0 {
			continue
		}
		g := byDoc[r.Doc][r.Path]
		if g == nil {
			g = &agg{}
			byDoc[r.Doc][r.Path] = g
		}
		g.items += len(r.Items)
		for _, it := range r.Items {
			if it.End > g.maxLine {
				g.maxLine = it.End
			}
		}
	}
	var out []Proposal
	var need []string
	for _, d := range a.Docs {
		for name := range byDoc[d] {
			for fp := range full[d][name] {
				if !strings.Contains(fp, "@") {
					need = append(need, fp)
				}
			}
		}
	}
	counts, err := Git{Dir: a.Upstream}.LineCounts(a.From.Commit, need)
	if err != nil {
		return nil, err
	}
	for _, d := range a.Docs {
		if docGroup(d) != GroupRewrite {
			continue
		}
		var names []string
		for n := range byDoc[d] {
			names = append(names, n)
		}
		sort.Strings(names)
		for _, n := range names {
			g := byDoc[d][n]
			pr := Proposal{Doc: d, Name: n, Items: g.items}
			for fp := range full[d][n] {
				pr.Cands = append(pr.Cands, fp)
			}
			sort.Strings(pr.Cands)
			switch {
			case len(pr.Cands) == 0:
				pr.Why = "the document writes no full path of that name"
			case len(pr.Cands) > 1:
				pr.Why = "several full paths of that name: " + strings.Join(pr.Cands, ", ")
			default:
				c := pr.Cands[0]
				lines, isUp := counts[c]
				switch {
				case strings.Contains(c, "@"):
					pr.Value = c
				case !isUp:
					pr.Why = "the one full path of that name, " + c + ", is not an upstream file"
				case g.maxLine > lines:
					pr.Why = fmt.Sprintf("a cited line (%d) lies past the end of %s (%d lines)", g.maxLine, c, lines)
				default:
					pr.Value = c
				}
			}
			out = append(out, pr)
		}
	}
	return out, nil
}

// WriteProposals writes the proposed directives, one line per document.
func (a *Analysis) WriteProposals(w io.Writer) error {
	props, err := a.Proposals()
	if err != nil {
		return err
	}
	p := func(format string, args ...any) { fmt.Fprintf(w, format, args...) }
	p("## Proposed directives for bare names\n\n")
	p("A bare name is rewritten only when the document maps it with a `<!-- pinmove: name=path -->` directive. Proposed below where the document writes exactly one full path of that name and every cited line exists in it. Each must be checked by reading before it is added.\n\n")
	covered, open := 0, 0
	byDoc := map[string][]Proposal{}
	for _, pr := range props {
		byDoc[pr.Doc] = append(byDoc[pr.Doc], pr)
	}
	for _, d := range a.Docs {
		var kv []string
		var rest []Proposal
		n := 0
		for _, pr := range byDoc[d] {
			if pr.Value != "" {
				kv = append(kv, pr.Name+"="+pr.Value)
				n += pr.Items
			} else {
				rest = append(rest, pr)
			}
		}
		if len(kv) == 0 && len(rest) == 0 {
			continue
		}
		covered += n
		p("### `%s`\n\n", d)
		if len(kv) > 0 {
			p("Covers %d items:\n\n    <!-- pinmove: %s -->\n\n", n, strings.Join(kv, " "))
		}
		for _, pr := range rest {
			open += pr.Items
			p("- `%s` (%d items): %s\n", pr.Name, pr.Items, pr.Why)
		}
		if len(rest) > 0 {
			p("\n")
		}
	}
	p("Proposed directives cover %d items; %d items of bare names have no proposal.\n", covered, open)
	return nil
}
