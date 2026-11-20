// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

package main

import (
	"bytes"
	"fmt"
	"path"
	"regexp"
	"strings"
)

// A cited line keeps its text, but the Myers diff may align identical
// lines of one declaration with those of another: when a change appends a
// function whose lines repeat the end of the one before it ("return nil",
// "}"), the old function's own lines can be the ones reported as changed
// and the cited lines found in the new function.  The text the citation
// supports then names a function the lines no longer belong to.  The same
// happens to an item the diff leaves at its old numbers, when the
// declaration around it was replaced by another.  So the enclosing
// declaration of every shifted and every same item in a file the range
// changes is compared at the two revisions, and an item whose declaration
// differs is listed for a human instead of rewritten.
//
// The declaration of a line is the nearest line at or above it, at column
// 0, that opens one (Go: func, with its receiver's type, type, var, const,
// import, package; protobuf: message, service, enum, extend, and rpc at
// any indentation), cleared by a line holding only a closing brace or
// parenthesis at column 0, so that the lines between two declarations
// belong to neither.  Declarations are compared by position, not by name:
// the old declaration's opening line is mapped through the diff, and it
// must be unchanged and be the opening line of the new declaration.  Names
// alone would pass a line moved into another declaration of the same name
// (a second `func init`, a second `var (` block).  Only when the opening
// line itself changed (a new signature, a comment joined to it) are the
// names compared instead, so that a changed signature of the same function
// is not a different declaration.  Other file types are not checked, and
// an item that stays inside one declaration while the diff pairs it with
// an identical line elsewhere in that declaration is not caught.

var (
	goFuncRe  = regexp.MustCompile(`^func\s*(?:\(([^)]*)\))?\s*([A-Za-z_][A-Za-z0-9_]*)`)
	goRecvRe  = regexp.MustCompile(`\*?\s*([A-Za-z_][A-Za-z0-9_]*)\s*(?:\[[^\]]*\])?\s*$`)
	goOtherRe = regexp.MustCompile(`^(type|var|const|import)\b\s*(\(|[A-Za-z_][A-Za-z0-9_]*)`)
	protoRe   = regexp.MustCompile(`^(message|service|enum|extend)\s+([A-Za-z_][A-Za-z0-9_.]*)`)
	protoRPC  = regexp.MustCompile(`^\s+rpc\s+([A-Za-z_][A-Za-z0-9_]*)`)
)

// declOpen returns the declaration a line opens, or "".
func declOpen(ext, line string) string {
	switch ext {
	case "go":
		if m := goFuncRe.FindStringSubmatch(line); m != nil {
			if m[1] != "" {
				if r := goRecvRe.FindStringSubmatch(m[1]); r != nil {
					return "func (" + r[1] + ")." + m[2]
				}
			}
			return "func " + m[2]
		}
		if m := goOtherRe.FindStringSubmatch(line); m != nil {
			return m[1] + " " + m[2]
		}
		if strings.HasPrefix(line, "package ") {
			return "package"
		}
	case "proto":
		if m := protoRe.FindStringSubmatch(line); m != nil {
			return m[1] + " " + m[2]
		}
		if m := protoRPC.FindStringSubmatch(line); m != nil {
			return "rpc " + m[1]
		}
	}
	return ""
}

// decl is the declaration a line belongs to: its name, and the 1-based
// line that opens it (0 for a line outside every declaration).
type decl struct {
	Name string
	Open int
}

// declIndex returns, per line (index 0 is line 1), the declaration the
// line belongs to, or nil when the file type is not checked.
func declIndex(file string, data []byte) []decl {
	ext := extOf(path.Base(file))
	if ext != "go" && ext != "proto" {
		return nil
	}
	ls := bytes.Split(data, []byte{'\n'})
	if len(ls) > 0 && len(ls[len(ls)-1]) == 0 {
		ls = ls[:len(ls)-1]
	}
	out := make([]decl, len(ls))
	var cur decl
	for i, b := range ls {
		line := strings.TrimSuffix(string(b), "\r")
		if d := declOpen(ext, line); d != "" {
			cur = decl{Name: d, Open: i + 1}
		}
		out[i] = cur
		// A line holding only a closing brace or parenthesis at column 0
		// ends the declaration after itself (not ") error {", which ends a
		// signature written over several lines).
		switch strings.TrimRight(line, " \t") {
		case "}", ")", "};":
			cur = decl{}
		}
	}
	return out
}

// declAt returns the declaration of 1-based line n, and false when n is
// outside the index.
func declAt(idx []decl, n int) (decl, bool) {
	if n < 1 || n > len(idx) {
		return decl{}, false
	}
	return idx[n-1], true
}

// declReview returns why a shifted or same item needs a human, or "": the
// declaration of its first or last line at the new revision is not the
// one it was in at the old revision.  hunks are the file's zero-context
// hunks, sorted by OldStart.
func declReview(oldIdx, newIdx []decl, hunks []Hunk, m Mapping, start, end int) string {
	if oldIdx == nil || newIdx == nil {
		return ""
	}
	for _, pr := range [][2]int{{start, m.NewStart}, {end, m.NewEnd}} {
		o, ok1 := declAt(oldIdx, pr[0])
		n, ok2 := declAt(newIdx, pr[1])
		if !ok1 || !ok2 {
			return fmt.Sprintf("line %d maps to %d, past the end of the new file", pr[0], pr[1])
		}
		if o.Open == 0 && n.Open == 0 {
			continue
		}
		if o.Open == 0 || n.Open == 0 {
			return fmt.Sprintf("line %d is in %s, line %d in %s", pr[0], declName(o), pr[1], declName(n))
		}
		mo, changed := mapLine(hunks, o.Open)
		switch {
		case !changed && mo != n.Open:
			return fmt.Sprintf("line %d is in %s opened at line %d (line %d at the new revision), line %d in %s opened at line %d",
				pr[0], declName(o), o.Open, mo, pr[1], declName(n), n.Open)
		case changed && o.Name != n.Name:
			return fmt.Sprintf("line %d is in %s, line %d in %s", pr[0], declName(o), pr[1], declName(n))
		}
	}
	return ""
}

func declName(d decl) string {
	if d.Name == "" {
		return "no declaration"
	}
	return "`" + d.Name + "`"
}
