// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

package main

import (
	"sort"
	"strings"
)

// GoMod is the part of a go.mod file the report reads.
type GoMod struct {
	Module   string
	Go       string
	Require  map[string]string // module path -> version
	Indirect map[string]bool
	Replace  map[string]string // "old[ version]" -> "new[ version]"
}

// ParseGoMod reads module, go, require and replace directives, in both the
// single-line and the block form.  Anything else is ignored.
func ParseGoMod(data []byte) GoMod {
	m := GoMod{Require: map[string]string{}, Indirect: map[string]bool{}, Replace: map[string]string{}}
	block := ""
	for _, raw := range strings.Split(string(data), "\n") {
		line := strings.TrimSpace(raw)
		comment := ""
		if i := strings.Index(line, "//"); i >= 0 {
			comment = strings.TrimSpace(line[i+2:])
			line = strings.TrimSpace(line[:i])
		}
		if line == "" {
			continue
		}
		if block != "" {
			if line == ")" {
				block = ""
				continue
			}
			m.directive(block, line, comment)
			continue
		}
		verb, rest, _ := strings.Cut(line, " ")
		rest = strings.TrimSpace(rest)
		if rest == "(" {
			block = verb
			continue
		}
		switch verb {
		case "module":
			m.Module = strings.Trim(rest, `"`)
		case "go":
			m.Go = rest
		default:
			m.directive(verb, rest, comment)
		}
	}
	return m
}

func (m *GoMod) directive(verb, args, comment string) {
	switch verb {
	case "require":
		f := strings.Fields(args)
		if len(f) == 2 {
			mod := strings.Trim(f[0], `"`)
			m.Require[mod] = f[1]
			if comment == "indirect" {
				m.Indirect[mod] = true
			}
		}
	case "replace":
		if l, r, ok := strings.Cut(args, "=>"); ok {
			m.Replace[strings.Join(strings.Fields(l), " ")] = strings.Join(strings.Fields(r), " ")
		}
	}
}

// ModChange is one module whose requirement or effective version differs
// between two go.mod files.  The effective version is what a build of the
// module itself uses: the target of a replace directive for the module
// (one naming the required version before one naming the bare path), or
// else the requirement.  OldBuilt and NewBuilt are set only where a
// replace makes the effective version differ from the requirement, so a
// change of a replace alone, with the require line as it was, still
// appears here.
type ModChange struct {
	Module   string `json:"module"`
	Old      string `json:"old,omitempty"` // "" when added
	New      string `json:"new,omitempty"` // "" when removed
	Indirect bool   `json:"indirect,omitempty"`
	OldBuilt string `json:"oldBuilt,omitempty"` // replace target at the old revision
	NewBuilt string `json:"newBuilt,omitempty"` // replace target at the new revision
}

// From is the effective version at the old revision.
func (m ModChange) From() string {
	if m.OldBuilt != "" {
		return m.OldBuilt
	}
	return m.Old
}

// To is the effective version at the new revision.
func (m ModChange) To() string {
	if m.NewBuilt != "" {
		return m.NewBuilt
	}
	return m.New
}

// Moves reports whether what a build uses changes.
func (m ModChange) Moves() bool { return m.From() != m.To() }

// String renders the change: the requirement, and the effective versions
// where a replace applies on either side.
func (m ModChange) String() string {
	s := orNone(m.Old) + " -> " + orNone(m.New)
	if m.OldBuilt != "" || m.NewBuilt != "" {
		s += " (built: " + orNone(m.From()) + " -> " + orNone(m.To()) + ")"
	}
	return s
}

// Effective returns the version a build of the module itself uses for a
// required module: the replace target for "mod version", else for "mod",
// else the requirement.  A replace by the same module path yields its
// version alone; one by another path yields "path version", and one by a
// local directory the directory, neither of which equals any version a
// citation can name.  replaced is set when a replace applied.  It returns
// "" when mod is not required.
func (m GoMod) Effective(mod string) (eff string, replaced bool) {
	req, ok := m.Require[mod]
	if !ok {
		return "", false
	}
	r, ok := m.Replace[mod+" "+req]
	if !ok {
		r, ok = m.Replace[mod]
	}
	if !ok {
		return req, false
	}
	if p, v, two := strings.Cut(r, " "); two && p == mod {
		return v, true
	}
	return r, true
}

// ModDelta lists the modules whose requirement or effective version
// changes, and the replace directive changes, between two go.mod files,
// sorted by module path.
func ModDelta(a, b GoMod) (changes []ModChange, replaces []string) {
	seen := map[string]bool{}
	for mod := range a.Require {
		seen[mod] = true
	}
	for mod := range b.Require {
		seen[mod] = true
	}
	for mod := range seen {
		c := ModChange{Module: mod, Old: a.Require[mod], New: b.Require[mod], Indirect: a.Indirect[mod] || b.Indirect[mod]}
		if e, ok := a.Effective(mod); ok && e != c.Old {
			c.OldBuilt = e
		}
		if e, ok := b.Effective(mod); ok && e != c.New {
			c.NewBuilt = e
		}
		if c.Old != c.New || c.Moves() {
			changes = append(changes, c)
		}
	}
	sort.Slice(changes, func(i, j int) bool { return changes[i].Module < changes[j].Module })
	keys := map[string]bool{}
	for k := range a.Replace {
		keys[k] = true
	}
	for k := range b.Replace {
		keys[k] = true
	}
	for k := range keys {
		if a.Replace[k] != b.Replace[k] {
			replaces = append(replaces, k+": "+orNone(a.Replace[k])+" -> "+orNone(b.Replace[k]))
		}
	}
	sort.Strings(replaces)
	return changes, replaces
}

func orNone(s string) string {
	if s == "" {
		return "(none)"
	}
	return s
}

// MatchModule finds the required module a citation's module names: the
// module itself or the only required module whose path ends in "/" plus
// the cited form ("dcrd/wire", "wire", "bbolt").  It returns "" when none
// or several match.
func (m GoMod) MatchModule(cited string) string {
	if _, ok := m.Require[cited]; ok {
		return cited
	}
	found := ""
	for mod := range m.Require {
		if strings.HasSuffix(mod, "/"+cited) {
			if found != "" {
				return ""
			}
			found = mod
		}
	}
	return found
}
