// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

package main

import (
	"strings"
	"testing"
)

// visible returns the non-blank lines of commentText's output, trimmed.
func visible(ext, src string) []string {
	var out []string
	for _, l := range strings.Split(string(commentText(ext, []byte(src))), "\n") {
		if t := strings.TrimSpace(l); t != "" {
			out = append(out, t)
		}
	}
	return out
}

func TestCommentText(t *testing.T) {
	cases := []struct {
		name, ext, src string
		want           []string
	}{
		{"rust line and doc comments", "rs", "//! Crate doc a.go:1\n/// Item doc\nfn f() {} // trailing\n", []string{"Crate doc a.go:1", "Item doc", "trailing"}},
		{"rust strings hide markers", "rs", "let s = \"// not a comment\"; let e = \"a\\\"// still\"; // yes\n", []string{"yes"}},
		{"rust raw strings", "rs", "let r = r#\"he said \"// no\" \"#; // after\nlet b = br\"//x\";\n", []string{"after"}},
		{"rust multi-line string", "rs", "let s = \"line one\n// inside the string\n\"; // out\n", []string{"out"}},
		{"rust char and lifetime", "rs", "fn f<'a>(x: &'a str) -> char { '\"' } // c1\nlet q = '\\''; // c2\nlet u = 'é'; // c3\n", []string{"c1", "c2", "c3"}},
		{"rust nested block", "rs", "/* outer /* inner */ still outer */ code /* b */\n", []string{"outer    inner    still outer            b"}},
		{"block comment stars", "rs", "/**\n * Doc a.go:5\n */\n", []string{"Doc a.go:5"}},
		{"go raw string", "go", "var s = `// not\n// still not`\n// yes\n", []string{"yes"}},
		{"go rune and string", "go", "var c = '\"' // r1\nvar s = \"//\" // r2\n", []string{"r1", "r2"}},
		{"go block comments do not nest", "go", "/* a /* b */ c */\n", []string{"a /* b"}},
		{"identifier ending in r", "rs", "let fr = 1; for r in x {} // ok\n", []string{"ok"}},
	}
	for _, tc := range cases {
		got := visible(tc.ext, tc.src)
		if strings.Join(got, "|") != strings.Join(tc.want, "|") {
			t.Errorf("%s: got %q, want %q", tc.name, got, tc.want)
		}
	}
	// Offsets are the source's.
	src := "fn f() {}\n    // see wallet/a.go:12\n"
	out := commentText("rs", []byte(src))
	if len(out) != len(src) || strings.Index(string(out), "wallet/a.go:12") != strings.Index(src, "wallet/a.go:12") {
		t.Errorf("offsets moved: %q", out)
	}
}

// FuzzCommentText: the output has the input's length and line breaks, and
// every byte it keeps is the input's byte at that offset.
func FuzzCommentText(f *testing.F) {
	for _, s := range []string{
		"//! a.go:1\nfn f<'a>() -> char { '\"' } /* x /* y */ */\n",
		"let r = br##\"// \"# \"##; // z\n",
		"var s = `//`\n/* a\n * b */\n",
	} {
		f.Add(s)
	}
	f.Fuzz(func(t *testing.T, src string) {
		for _, ext := range []string{"rs", "go"} {
			out := commentText(ext, []byte(src))
			if len(out) != len(src) {
				t.Fatalf("%s: length %d, want %d", ext, len(out), len(src))
			}
			for i := range out {
				if (src[i] == '\n') != (out[i] == '\n') || (out[i] != ' ' && out[i] != src[i]) {
					t.Fatalf("%s: byte %d is %q, input %q", ext, i, out[i], src[i])
				}
			}
		}
	})
}
