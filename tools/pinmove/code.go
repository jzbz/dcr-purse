// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

package main

import (
	"io/fs"
	"os"
	"path"
	"path/filepath"
	"sort"
	"strings"
	"unicode/utf8"
)

// The code also cites upstream source, in comments: a Rust test names the
// dcrwallet lines it reproduces, an oracle command the function it calls.
// ADR-0016 item 5.8 rewrites citations in documents only, so these are a
// group of their own: classified like the documents' citations, listed by
// report and cite, never rewritten.  Only comments are read (string
// literals hold test input, not citations), from the Rust and Go files
// under codeRoots, leaving out the frozen files' directories and the
// oracle's copies of upstream source, whose comments are upstream's own
// and move with the copy (item 5.4).

// codeRoots are the directories whose Rust and Go comments are read.
// tools/pinmove is not among them: its tests cite synthetic repositories.
var codeRoots = []string{"crates", "fuzz", "tools/oracle"}

// FindCode lists the Rust and Go files under codeRoots, sorted, leaving
// out directories named testdata or target, every tests/data directory,
// and the paths skip reports (repository-relative, slash-separated).
func FindCode(repo string, skip func(string) bool) ([]string, error) {
	var out []string
	for _, r := range codeRoots {
		root := filepath.Join(repo, filepath.FromSlash(r))
		err := filepath.WalkDir(root, func(p string, d fs.DirEntry, err error) error {
			if err != nil {
				return err
			}
			rel, err := filepath.Rel(repo, p)
			if err != nil {
				return err
			}
			rel = filepath.ToSlash(rel)
			if d.IsDir() {
				switch {
				case d.Name() == "testdata", d.Name() == "target", strings.HasSuffix(rel, "/tests/data"):
					return filepath.SkipDir
				case skip != nil && skip(rel):
					return filepath.SkipDir
				}
				return nil
			}
			if !d.Type().IsRegular() {
				return nil
			}
			if ext := extOf(rel); (ext == "rs" || ext == "go") && (skip == nil || !skip(rel)) {
				out = append(out, rel)
			}
			return nil
		})
		if err != nil && !os.IsNotExist(err) {
			return nil, err
		}
	}
	sort.Strings(out)
	return out, nil
}

// copySkipper reports whether a repository-relative path is one of the
// oracle's copies of upstream source in t (a directory or a file).
func copySkipper(t Table) func(string) bool {
	return func(p string) bool {
		rest, ok := strings.CutPrefix(p, "tools/oracle/")
		if !ok {
			return false
		}
		for _, c := range t.Copies {
			if rest == c.Oracle || strings.HasPrefix(rest, c.Oracle+"/") {
				return true
			}
		}
		return false
	}
}

// commentText returns src with everything but the text of its comments
// replaced by spaces, line breaks kept, so that byte offsets and line
// numbers are those of the source.  The comment markers ("//", "///",
// "//!", "/*", "*/" and a block comment line's leading "*") are blanked
// too, leaving the text the Markdown parser reads.  ext is "rs" or "go":
// Rust block comments nest, Rust has raw strings (r#"..."#) and character
// literals that a lifetime ('a) is told apart from, Go has raw strings in
// backquotes and rune literals.
func commentText(ext string, src []byte) []byte {
	out := make([]byte, len(src))
	for i := range out {
		if src[i] == '\n' {
			out[i] = '\n'
		} else {
			out[i] = ' '
		}
	}
	rust := ext == "rs"
	n := len(src)
	// skipString skips a string or character literal opened at i (the
	// quote), with backslash escapes, and returns the index after it.
	skipQuoted := func(i int, q byte) int {
		for j := i + 1; j < n; j++ {
			switch src[j] {
			case '\\':
				j++
			case q:
				return j + 1
			case '\n':
				if q == '\'' || (!rust && q == '"') {
					// An unterminated rune literal, or a Go string,
					// ends at the line: never swallow the file.
					return j
				}
			}
		}
		return n
	}
	identByte := func(c byte) bool { return isAlnum(c) || c == '_' }
	for i := 0; i < n; {
		c := src[i]
		switch {
		case c == '/' && i+1 < n && src[i+1] == '/':
			j := i + 2
			for j < n && (src[j] == '/' || src[j] == '!') {
				j++
			}
			for j < n && src[j] != '\n' {
				out[j] = src[j]
				j++
			}
			i = j
		case c == '/' && i+1 < n && src[i+1] == '*':
			depth := 1
			j := i + 2
			for j < n && (src[j] == '*' || src[j] == '!') && !(src[j] == '*' && j+1 < n && src[j+1] == '/') {
				j++
			}
			lineStart := false
			for j < n && depth > 0 {
				switch {
				case src[j] == '*' && j+1 < n && src[j+1] == '/':
					depth--
					j += 2
					continue
				case rust && src[j] == '/' && j+1 < n && src[j+1] == '*':
					depth++
					j += 2
					continue
				case src[j] == '\n':
					lineStart = true
					j++
					continue
				case lineStart && (src[j] == ' ' || src[j] == '\t'):
					j++
					continue
				case lineStart && src[j] == '*':
					// A block comment line's leading "*".
					lineStart = false
					j++
					continue
				}
				lineStart = false
				out[j] = src[j]
				j++
			}
			i = j
		case c == '"':
			i = skipQuoted(i, '"')
		case !rust && c == '`':
			j := i + 1
			for j < n && src[j] != '`' {
				j++
			}
			i = min(j+1, n)
		case !rust && c == '\'':
			i = skipQuoted(i, '\'')
		case rust && c == '\'':
			// A character literal ('x', '\n', '\u{1F600}') or a lifetime
			// ('a, 'static): a literal has its closing quote after one
			// character or an escape.
			if i+1 < n && src[i+1] == '\\' {
				i = skipQuoted(i, '\'')
				continue
			}
			_, sz := utf8.DecodeRune(src[min(i+1, n):])
			if sz > 0 && i+1+sz < n && src[i+1+sz] == '\'' {
				i += 2 + sz
				continue
			}
			i++
		case rust && (c == 'r' || c == 'b') && (i == 0 || !identByte(src[i-1])):
			// r"...", r#"..."#, br#"..."#: no escapes, closed by a quote
			// and as many hashes as opened it.
			j := i + 1
			if c == 'b' {
				if j >= n || src[j] != 'r' {
					i++
					continue
				}
				j++
			}
			hashes := 0
			for j < n && src[j] == '#' {
				hashes++
				j++
			}
			if j >= n || src[j] != '"' {
				i++
				continue
			}
			end := []byte("\"" + strings.Repeat("#", hashes))
			k := indexFrom(src, j+1, end)
			if k < 0 {
				i = n
				continue
			}
			i = k + len(end)
		default:
			i++
		}
	}
	return out
}

// indexFrom returns the index of sep in s at or after from, or -1.
func indexFrom(s []byte, from int, sep []byte) int {
	if from > len(s) {
		return -1
	}
	if k := strings.Index(string(s[from:]), string(sep)); k >= 0 {
		return from + k
	}
	return -1
}

// isCodeFile reports whether a repository-relative path is a source file
// whose comments are read (a Rust or Go file).
func isCodeFile(p string) bool {
	switch path.Ext(p) {
	case ".rs", ".go":
		return true
	}
	return false
}
