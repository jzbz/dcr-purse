// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

package main

import (
	"fmt"
	"strings"
	"testing"
)

func TestParseHunk(t *testing.T) {
	for _, tc := range []struct {
		line string
		want Hunk
	}{
		{"@@ -10,0 +11,3 @@", Hunk{10, 0, 11, 3}},
		{"@@ -5 +5 @@ func f() {", Hunk{5, 1, 5, 1}},
		{"@@ -7,2 +6,0 @@", Hunk{7, 2, 6, 0}},
		{"@@ -0,0 +1,4 @@", Hunk{0, 0, 1, 4}},
	} {
		got, err := parseHunk(tc.line)
		if err != nil || got != tc.want {
			t.Errorf("parseHunk(%q) = %v, %v; want %v", tc.line, got, err, tc.want)
		}
	}
	if _, err := parseHunk("@@ -x +1 @@"); err == nil {
		t.Error("a malformed hunk header parsed")
	}
}

func TestClassify(t *testing.T) {
	// Old file of 100 lines.  Hunks: 3 lines inserted after line 10; line
	// 20 replaced by two lines; lines 30-31 deleted; one line inserted
	// after line 50.
	fd := &FileDiff{Path: "f.go", Status: 'M', Hunks: []Hunk{
		{10, 0, 11, 3},
		{20, 1, 23, 2},
		{30, 2, 33, 0},
		{50, 0, 51, 1},
	}}
	cases := []struct {
		start, end int
		want       Class
		ns, ne     int
	}{
		{1, 10, Same, 1, 10},      // before every hunk, ending where the insertion goes
		{5, 5, Same, 5, 5},        //
		{11, 11, Shifted, 14, 14}, // after the insertion: +3
		{10, 11, Touched, 0, 0},   // the insertion falls inside the range
		{12, 19, Shifted, 15, 22}, //
		{19, 21, Touched, 0, 0},   // line 20 changed
		{20, 20, Touched, 0, 0},   //
		{21, 29, Shifted, 25, 33}, // +3 +1
		{29, 30, Touched, 0, 0},   // line 30 deleted
		{32, 50, Shifted, 34, 52}, // +3 +1 -2
		{50, 51, Touched, 0, 0},   // insertion after 50, inside 50-51
		{51, 100, Shifted, 54, 103},
		{100, 101, Invalid, 0, 0}, // past the end of the old file
	}
	for _, tc := range cases {
		m := classify(fd, 100, tc.start, tc.end)
		if m.Class != tc.want || (tc.want != Touched && tc.want != Invalid && (m.NewStart != tc.ns || m.NewEnd != tc.ne)) {
			t.Errorf("%d-%d: got %v %d-%d, want %v %d-%d", tc.start, tc.end, m.Class, m.NewStart, m.NewEnd, tc.want, tc.ns, tc.ne)
		}
		if m.Class == Touched && len(m.Hunks) == 0 {
			t.Errorf("%d-%d: touched without the hunks that touch it", tc.start, tc.end)
		}
	}
	if m := classify(nil, 100, 3, 4); m.Class != Same || m.NewStart != 3 {
		t.Errorf("unchanged file: %+v", m)
	}
	if m := classify(&FileDiff{Status: 'D', Renamed: "g.go"}, 100, 3, 4); m.Class != Gone || m.RenamedTo != "g.go" {
		t.Errorf("deleted file: %+v", m)
	}
	if m := classify(&FileDiff{Status: 'M', Binary: true}, 100, 3, 4); m.Class != Touched {
		t.Errorf("binary file: %+v", m)
	}
	// An insertion before line 1.
	if m := classify(&FileDiff{Status: 'M', Hunks: []Hunk{{0, 0, 1, 2}}}, 10, 1, 3); m.Class != Shifted || m.NewStart != 3 {
		t.Errorf("insertion at the top: %+v", m)
	}
}

func TestParsePatch(t *testing.T) {
	patch := `diff --git a/a.go b/a.go
index 1111111..2222222 100644
--- a/a.go
+++ b/a.go
@@ -3,0 +4,2 @@ func a() {
+x
+y
@@ -9 +11 @@
--- a/not-a-header
+changed
diff --git a/gone.go b/gone.go
deleted file mode 100644
index 3333333..0000000
--- a/gone.go
+++ /dev/null
@@ -1,2 +0,0 @@
-p
-q
diff --git a/new.go b/new.go
new file mode 100644
index 0000000..4444444
--- /dev/null
+++ b/new.go
@@ -0,0 +1 @@
+n
diff --git a/img.png b/img.png
index 5555555..6666666 100644
Binary files a/img.png and b/img.png differ
`
	files, err := parsePatch([]byte(patch))
	if err != nil {
		t.Fatal(err)
	}
	if fd := files["a.go"]; fd == nil || fd.Status != 'M' || len(fd.Hunks) != 2 || fd.Hunks[1] != (Hunk{9, 1, 11, 1}) {
		t.Errorf("a.go: %+v", fd)
	}
	if fd := files["gone.go"]; fd == nil || fd.Status != 'D' {
		t.Errorf("gone.go: %+v", fd)
	}
	if fd := files["new.go"]; fd == nil || fd.Status != 'A' {
		t.Errorf("new.go: %+v", fd)
	}
	if fd := files["img.png"]; fd == nil || !fd.Binary {
		t.Errorf("img.png: %+v", fd)
	}
}

// TestParsePatchTypeChange: git prints a type change as a deletion and an
// addition of one path; the deletion is kept, marked.
func TestParsePatchTypeChange(t *testing.T) {
	patch := `diff --git a/link.go b/link.go
deleted file mode 100644
index 1111111..0000000
--- a/link.go
+++ /dev/null
@@ -1,30 +0,0 @@
-x
diff --git a/link.go b/link.go
new file mode 120000
index 0000000..2222222
--- /dev/null
+++ b/link.go
@@ -0,0 +1 @@
+a.go
`
	files, err := parsePatch([]byte(patch))
	if err != nil {
		t.Fatal(err)
	}
	fd := files["link.go"]
	if fd == nil || fd.Status != 'D' || !fd.TypeChange || len(fd.Hunks) != 1 || fd.Hunks[0] != (Hunk{1, 30, 0, 0}) {
		t.Fatalf("link.go: %+v", fd)
	}
	if m := classify(fd, 30, 5, 6); m.Class != Gone || !m.TypeChange {
		t.Errorf("classify: %+v", m)
	}
	// An added file is never mapped through.
	if m := classify(&FileDiff{Status: 'A', Hunks: []Hunk{{0, 0, 1, 1}}}, 30, 5, 6); m.Class != Gone {
		t.Errorf("classify of an added file: %+v", m)
	}
}

func TestDeclIndex(t *testing.T) {
	src := "package w\n\nimport (\n\t\"x\"\n)\n\n// A does.\nfunc (w *Wallet[T]) A(\n\tn int,\n) error {\n\treturn nil\n}\n\ntype S struct {\n\tf int\n}\nvar v = 1\n"
	got := declIndex("w.go", []byte(src))
	want := "package@1|package@1|import (@3|import (@3|import (@3|@0|@0|func (Wallet).A@8|func (Wallet).A@8|func (Wallet).A@8|func (Wallet).A@8|func (Wallet).A@8|@0|type S@14|type S@14|type S@14|var v@17"
	if s := declString(got); s != want {
		t.Errorf("got  %q\nwant %q", s, want)
	}
	proto := "service S {\n  rpc Ping (A) returns (B);\n  rpc Pong (A) returns (B);\n}\nmessage M {\n  int32 x = 1;\n}\n"
	got = declIndex("api.proto", []byte(proto))
	want = "service S@1|rpc Ping@2|rpc Pong@3|rpc Pong@3|message M@5|message M@5|message M@5"
	if s := declString(got); s != want {
		t.Errorf("proto: got %q", s)
	}
	if declIndex("x.md", []byte("a\n")) != nil {
		t.Error("a Markdown file is indexed")
	}
}

func declString(ds []decl) string {
	var parts []string
	for _, d := range ds {
		parts = append(parts, fmt.Sprintf("%s@%d", d.Name, d.Open))
	}
	return strings.Join(parts, "|")
}

// TestDeclReview: declarations are compared by position.  A line the diff
// pairs with a line of another declaration of the same name is flagged; a
// changed signature of the same declaration is not.
func TestDeclReview(t *testing.T) {
	idx := func(src string) []decl { return declIndex("w.go", []byte(src)) }
	cases := []struct {
		name       string
		old, new   string
		hunks      []Hunk
		start, end int
		m          Mapping
		flagged    bool
	}{
		{
			// The first init's body replaced, a second init appended with
			// the old body: the cited lines are found in the second.
			name:  "second init",
			old:   "package w\n\nfunc init() {\n\tx()\n\tregister(a)\n}\n",
			new:   "package w\n\nfunc init() {\n\ty()\n\tregister(c)\n}\n\nfunc init() {\n\tx()\n\tregister(a)\n}\n",
			hunks: []Hunk{{4, 2, 4, 2}, {6, 0, 7, 5}},
			start: 4, end: 5, m: Mapping{Class: Shifted, NewStart: 9, NewEnd: 10}, flagged: true,
		},
		{
			name:  "second var block",
			old:   "package w\n\nvar (\n\ta = 1\n\tb = 2\n)\n",
			new:   "package w\n\nvar (\n\tq = 1\n\tr = 2\n)\n\nvar (\n\ta = 1\n\tb = 2\n)\n",
			hunks: []Hunk{{4, 2, 4, 2}, {6, 0, 7, 5}},
			start: 4, end: 5, m: Mapping{Class: Shifted, NewStart: 9, NewEnd: 10}, flagged: true,
		},
		{
			// A line inserted above inside the same function.
			name:  "same function",
			old:   "package w\n\nfunc A() {\n\tx()\n\ty()\n}\n",
			new:   "package w\n\nfunc A() {\n\tw()\n\tx()\n\ty()\n}\n",
			hunks: []Hunk{{3, 0, 4, 1}},
			start: 5, end: 5, m: Mapping{Class: Shifted, NewStart: 6, NewEnd: 6},
		},
		{
			name:  "changed signature",
			old:   "package w\n\nfunc S(a int) {\n\tp()\n\tq()\n}\n",
			new:   "package w\n\n// S does p and q.\nfunc S(a, b int) {\n\tp()\n\tq()\n}\n",
			hunks: []Hunk{{3, 1, 3, 2}},
			start: 5, end: 5, m: Mapping{Class: Shifted, NewStart: 6, NewEnd: 6},
		},
		{
			// The declaration around unchanged numbers replaced: Same.
			name:  "same numbers, other function",
			old:   "package w\n\nfunc A() error {\n\tx()\n\treturn nil\n}\n",
			new:   "package w\n\nfunc B() error {\n\tx()\n\treturn nil\n}\n\nfunc A() error {\n\ty()\n\treturn errFoo\n}\n",
			hunks: []Hunk{{3, 1, 3, 1}, {6, 0, 7, 5}},
			start: 4, end: 5, m: Mapping{Class: Same, NewStart: 4, NewEnd: 5}, flagged: true,
		},
		{
			name:  "outside every declaration",
			old:   "package w\n\nfunc A() {\n}\n\n// note\n",
			new:   "package w\n\n// new\nfunc A() {\n}\n\n// note\n",
			hunks: []Hunk{{2, 0, 3, 1}},
			start: 6, end: 6, m: Mapping{Class: Shifted, NewStart: 7, NewEnd: 7},
		},
	}
	for _, tc := range cases {
		got := declReview(idx(tc.old), idx(tc.new), tc.hunks, tc.m, tc.start, tc.end)
		if (got != "") != tc.flagged {
			t.Errorf("%s: review %q, want flagged %v", tc.name, got, tc.flagged)
		}
	}
}
