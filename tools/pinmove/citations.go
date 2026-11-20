// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

package main

// The citation grammar.  The documents cite source as follows; the parser
// accepts exactly these forms and records everything else that looks like a
// line citation as unparsed, with a reason, so that a survey can list it.
//
//	citation     = path ":" items
//	path         = [ module "@" version "/" ] segment { "/" segment }
//	               (the last segment ends in "." ext, ext a known extension)
//	items        = item { "," [ " " ] item }
//	item         = N [ ( "-" | "–" ) M ]     (1 <= N <= M)
//	continuation = ":" items                 (after a space, "(", "`", ",",
//	                                          "[", ";" or the line start)
//	numeric span = "`" items "`"             (directly after a code span that
//	                                          held a citation, separated by
//	                                          ", ", "; ", " and ", ", and "
//	                                          or "–"; unless after "–",
//	                                          followed by the line's end,
//	                                          punctuation, "and" or "or",
//	                                          or "(`")
//
// A path is resolved against the upstream tree at the old revision and the
// local repository:
//
//   - module@version/path names a dependency (goX.Y.Z/src/... the Go
//     toolchain); it moves only with a go.mod requirement.
//   - A path with a slash that is a file upstream is an upstream citation; a
//     file of this repository is local; both is ambiguous, and so is a bare
//     name that is a file at the upstream root and here ("README.md:77").
//     An ambiguous citation is classified against the upstream file and
//     listed, never rewritten.
//   - A bare name or relative path the document maps with a directive (see
//     directiveRe) takes the directive's path.
//   - A bare name, or a relative path that is not a file upstream, that
//     directly follows a citation or path mention in a list (see the list
//     rule below) is completed from that path's directory when the
//     directory holds the file ("inherited"): "`wallet/udb/txmined.go:12`,
//     `txdb.go:5`".  No completion is taken for a name that is also a
//     file at the upstream root, and none into a dependency module whose
//     copy in the Go module cache lacks the file.
//   - A bare name that is not completed so and names a file at the upstream
//     root is "root" when it is a Go file (or its name contains
//     "dcrwallet") and no other upstream file has that base name: the
//     specs' convention says paths are relative to the upstream root, and
//     no other upstream file could be meant.  Generic names (README.md,
//     SECURITY.md, go.mod) are left out because other repositories' files
//     of the same name are cited too.  When another upstream file shares
//     the name (config.go, log.go, params.go) it is "bare-root": authors
//     also wrote bare names of files in other directories, so these are
//     classified and listed, never rewritten.
//   - Anything else is unresolved, with the upstream files of the same base
//     name as hints.
//
// The list rule: a continuation takes the path of the citation or path
// mention it directly follows in a list, with only punctuation, code-span
// delimiters, a line break and one of "and", "or", "at", "line(s)" between
// them (a mention counts, so that "`spv/sync.go` at `:500`" attaches to
// spv/sync.go).  A continuation that opens a parenthesis and follows no
// citation so takes the section's source file instead (the one path the
// section heading names, or else the one path of the section's first
// "Source:" line: "| `ReadTx` (`:21-29`) |"; a line holding only
// "`path`:" heads what follows with that path instead, until the next
// heading), but only when its paragraph
// names no other file: after prose, authors meant the paragraph's last
// file in some places and the section's file in others.  Otherwise it is
// unresolved; a port written ":9110" after prose is never taken for a
// line.  A numeric span takes the path of the citation in the code span
// before it; after an en dash it closes the range its predecessor opened:
// "`spv/sync.go:205`–`206`" is the range 205-206.  A numeric span after
// ", ", "; " or "and" is taken only when what follows it ends the item
// (spanFollowOK): in "`wallet/wallet.go:2328`, `4941` atoms are the
// limit" the span is a value, not a line, and is recorded as unparsed.
//
// Every resolution is checked against the file's length at the old
// revision: an item past the end is reported as invalid and never
// rewritten, which also catches a misattributed citation whose lines do
// not exist in the file it was attributed to.

import (
	"path"
	"regexp"
	"sort"
	"strings"
)

// Target says what a citation's path names.
type Target uint8

// Targets.
const (
	TargetUnresolved Target = iota
	TargetUpstream
	TargetDependency
	TargetToolchain
	TargetLocal
	TargetAmbiguous
)

var targetNames = [...]string{"unresolved", "upstream", "dependency", "toolchain", "local", "ambiguous"}

func (t Target) String() string { return targetNames[t] }

// MarshalText writes the target's name, for the JSON report.
func (t Target) MarshalText() ([]byte, error) { return []byte(t.String()), nil }

// How says how the parser obtained a citation's path.
type How uint8

// Ways a path is obtained.
const (
	HowExplicit  How = iota // written in full
	HowInherited            // completed from the path it follows in a list
	HowBareRoot             // a bare name of a file at the upstream root
	HowBare                 // a bare name that resolved to nothing
	HowSection              // a continuation given the section's source file
	HowDirective            // a name the document's pinmove directive maps
	HowRoot                 // a bare name of an upstream root file no other upstream file shares
)

var howNames = [...]string{"explicit", "inherited", "bare-root", "bare", "section", "directive", "root"}

func (h How) String() string { return howNames[h] }

// MarshalText writes the name, for the JSON report.
func (h How) MarshalText() ([]byte, error) { return []byte(h.String()), nil }

// Form is the written form of a citation.
type Form uint8

// Forms.
const (
	FormPath         Form = iota // path:items
	FormContinuation             // :items
	FormNumericSpan              // `items` after a citation's code span
	FormMention                  // a path with no line number
)

var formNames = [...]string{"path:line", ":line", "`line` span", "mention"}

func (f Form) String() string { return formNames[f] }

// MarshalText writes the form's name, for the JSON report.
func (f Form) MarshalText() ([]byte, error) { return []byte(f.String()), nil }

// Item is one line or range of a citation.  The offsets are byte offsets
// into the document of the digits of Start and End; EndOff is -1 for a
// single line.
type Item struct {
	Start, End         int
	StartOff, StartLen int
	EndOff, EndLen     int
}

// Citation is one citation or path mention in a document.
type Citation struct {
	Doc     string
	Line    int // 1-based line of the document
	Off     int // byte offset of the first byte
	Raw     string
	Written string // the path as written ("" for a continuation)
	Path    string // upstream-, module- or repository-relative path
	Module  string
	Version string
	Target  Target
	How     How
	Form    Form
	Items   []Item
	Section string   // nearest heading above
	Hints   []string // upstream files of the same base name, when unresolved
	Note    string
}

// Mapped reports whether the citation's lines are mapped through the
// upstream diff and so may be rewritten: an upstream citation written in
// full, completed from the path it follows in a list, given its
// section's source file, declared by a directive, or a bare name of a root
// file whose base name no other upstream file has.  An ambiguous citation
// is classified (classified) but never mapped so.
func (c *Citation) Mapped() bool {
	return c.Target == TargetUpstream && (c.How == HowExplicit || c.How == HowInherited ||
		c.How == HowSection || c.How == HowDirective || c.How == HowRoot)
}

// classified reports whether the citation's lines are mapped through the
// upstream diff: every upstream citation, and every citation of a file
// that is both upstream and in this repository (ambiguous), which is
// classified so that a reviewer sees what the range does to the upstream
// file, and never rewritten.
func (c *Citation) classified() bool {
	return c.Target == TargetUpstream || c.Target == TargetAmbiguous
}

// Unparsed is text that looks like a line citation and is not one the
// grammar accepts.
type Unparsed struct {
	Doc    string
	Line   int
	Raw    string
	Reason string
}

// Resolver resolves paths against the upstream tree and this repository.
type Resolver struct {
	upstream map[string]bool
	base     map[string][]string
	local    func(string) bool
	// dep reports whether a file exists in a dependency module at a
	// version; known is false when that cannot be told (SetDepChecker).
	dep func(module, version, file string) (exists, known bool)
}

// SetDepChecker sets how list completion tells whether a file exists in a
// dependency module: a bare name is completed into a module only when the
// checker does not know the file to be absent.
func (r *Resolver) SetDepChecker(f func(module, version, file string) (exists, known bool)) {
	r.dep = f
}

// depAbsent reports whether the checker knows a dependency file to be
// absent.
func (r *Resolver) depAbsent(module, version, file string) bool {
	if r.dep == nil {
		return false
	}
	exists, known := r.dep(module, version, file)
	return known && !exists
}

// NewResolver indexes the upstream files.  local reports whether a
// repository-relative path names a file of this repository.
func NewResolver(upstreamFiles []string, local func(string) bool) *Resolver {
	r := &Resolver{upstream: make(map[string]bool, len(upstreamFiles)), base: make(map[string][]string), local: local}
	for _, f := range upstreamFiles {
		r.upstream[f] = true
		b := path.Base(f)
		r.base[b] = append(r.base[b], f)
	}
	for _, v := range r.base {
		sort.Strings(v)
	}
	if r.local == nil {
		r.local = func(string) bool { return false }
	}
	return r
}

// Upstream reports whether p is a file of the upstream tree.
func (r *Resolver) Upstream(p string) bool { return r.upstream[p] }

var knownExt = map[string]bool{
	"go": true, "proto": true, "mod": true, "sum": true, "md": true, "rs": true,
	"toml": true, "yml": true, "yaml": true, "json": true, "jsonl": true,
	"sh": true, "conf": true, "txt": true, "nix": true, "py": true, "c": true,
	"h": true, "s": true, "ps1": true, "service": true, "xml": true,
	"html": true, "js": true, "ts": true, "cfg": true, "ini": true,
	"sql": true, "csv": true, "tsv": true, "lock": true,
}

var toolchainSeg = regexp.MustCompile(`^go1\.[0-9]+(\.[0-9]+)?((rc|beta)[0-9]+)?$`)

func isPathChar(c byte) bool {
	return c >= 'a' && c <= 'z' || c >= 'A' && c <= 'Z' || c >= '0' && c <= '9' ||
		c == '_' || c == '.' || c == '-' || c == '+' || c == '~' || c == '@' || c == '/'
}

func isDigit(c byte) bool { return c >= '0' && c <= '9' }

func isAlnum(c byte) bool {
	return c >= 'a' && c <= 'z' || c >= 'A' && c <= 'Z' || c >= '0' && c <= '9'
}

// extOf returns the extension of the last segment of p, or "".
func extOf(p string) string {
	b := path.Base(p)
	i := strings.LastIndexByte(b, '.')
	if i <= 0 || i == len(b)-1 {
		return ""
	}
	return b[i+1:]
}

// ctxPath is a path a later citation can take.
type ctxPath struct {
	set     bool
	target  Target
	how     How
	path    string
	module  string
	version string
	hints   []string
}

const enDash = "–"

// numAt reads a decimal number at s[i:], returning it and the end; ok is
// false when there is no digit, the number has a leading zero or it is
// implausibly long.
func numAt(s string, i int) (n, end int, ok bool) {
	j := i
	for j < len(s) && isDigit(s[j]) {
		j++
	}
	// No leading zero: a line number is written plainly, and a rewrite
	// must reproduce the text it replaces.
	if j == i || j-i > 7 || (s[i] == '0' && j-i > 1) {
		return 0, i, false
	}
	for k := i; k < j; k++ {
		n = n*10 + int(s[k]-'0')
	}
	return n, j, true
}

// termOK reports whether the byte at s[i] may follow an item.  strict is
// set for an item written after ", ", where prose could follow.
func termOK(s string, i int, strict bool) bool {
	if i >= len(s) {
		return true
	}
	c := s[i]
	if strict {
		switch c {
		case ',', '`', ')', ';', ']', '|', '*':
			return true
		case '.':
			return i+1 >= len(s) || !isAlnum(s[i+1])
		}
		return false
	}
	switch {
	case isAlnum(c) || c == '_' || c == '/' || c == '-' || c == '@':
		return false
	case (c == '.' || c == ':') && i+1 < len(s) && isDigit(s[i+1]):
		return false
	case strings.HasPrefix(s[i:], enDash):
		return false
	}
	return true
}

// parseItems parses items at s[i:] (offset base in the document).  It
// returns the items and the end of the last one; reason says why nothing
// was parsed when items is empty.
func parseItems(s string, i, base int) (items []Item, end int, reason string) {
	p := i
	strict := false
	for {
		start, e, ok := numAt(s, p)
		if !ok {
			break
		}
		it := Item{Start: start, End: start, StartOff: base + p, StartLen: e - p, EndOff: -1}
		q := e
		dash := 0
		if q < len(s) && s[q] == '-' {
			dash = 1
		} else if strings.HasPrefix(s[q:], enDash) {
			dash = len(enDash)
		}
		if dash > 0 {
			end2, e2, ok2 := numAt(s, q+dash)
			if !ok2 {
				if len(items) == 0 {
					return nil, i, "range without an end"
				}
				break
			}
			it.End = end2
			it.EndOff = base + q + dash
			it.EndLen = e2 - q - dash
			q = e2
		}
		if !termOK(s, q, strict) {
			if len(items) == 0 {
				return nil, i, "line number followed by other text"
			}
			break
		}
		if it.Start == 0 || it.End < it.Start {
			if len(items) == 0 {
				return nil, i, "line zero or a range that ends before it starts"
			}
			break
		}
		items = append(items, it)
		end = q
		// Another item?
		if q < len(s) && s[q] == ',' {
			r := q + 1
			strict = false
			if r < len(s) && s[r] == ' ' {
				r++
				strict = true
			}
			if r < len(s) && isDigit(s[r]) {
				p = r
				continue
			}
		}
		break
	}
	if len(items) == 0 {
		return nil, i, "no line number"
	}
	return items, end, ""
}

var unitStart = regexp.MustCompile(`^(#|\||>|[-*+] |[0-9]+[.)] )`)

// separators allowed between a citation's code span and a numeric span.
var spanSeps = map[string]bool{", ": true, "; ": true, " and ": true, ", and ": true, enDash: true}

// listGap matches the text allowed between a citation (or a path mention)
// and a continuation that belongs to it: list punctuation, code-span
// delimiters, a line break and at most one connecting word.
var listGap = regexp.MustCompile("^[\\s,;`(]*((and|or|at|lines?)[\\s`(]+)?$")

// directiveRe matches a document's declarations for bare names:
//
//	<!-- pinmove: methods.go=internal/rpc/jsonrpc/methods.go client.go=dcrd/mixing@v0.7.5/mixclient/client.go helper.go=- -->
//
// Each maps a name as written (a bare name, or a relative path) to an
// upstream path, to module@version/path, or to "-" (never upstream).  They
// apply to the whole document, after a path written in full and before
// completion from a list and the root rules.
var directiveRe = regexp.MustCompile(`<!--\s*pinmove:([^>]*)-->`)

type directive struct {
	target  Target
	path    string
	module  string
	version string
}

// parseDirectives reads a document's directives.
func (p *parser) parseDirectives() {
	p.directives = map[string]directive{}
	for _, m := range directiveRe.FindAllStringSubmatchIndex(p.s, -1) {
		line := 1 + strings.Count(p.s[:m[0]], "\n")
		for _, kv := range strings.Fields(p.s[m[2]:m[3]]) {
			k, v, ok := strings.Cut(kv, "=")
			if !ok || k == "" || v == "" {
				p.unparsedAt(line, kv, "directive: not name=path")
				continue
			}
			var d directive
			switch {
			case v == "-":
				d.target = TargetUnresolved
			case strings.Contains(v, "@"):
				at := strings.IndexByte(v, '@')
				ver, rest, ok := strings.Cut(v[at+1:], "/")
				if !ok {
					p.unparsedAt(line, kv, "directive: module@version without a path")
					continue
				}
				d = directive{target: TargetDependency, module: v[:at], version: ver, path: rest}
			case p.r.upstream[v]:
				d = directive{target: TargetUpstream, path: v}
			default:
				p.unparsedAt(line, kv, "directive: not an upstream file at the old revision")
				continue
			}
			p.directives[path.Clean(k)] = d
		}
	}
}

// ParseDoc finds the citations and path mentions in a Markdown document.
func ParseDoc(doc string, data []byte, r *Resolver) ([]Citation, []Unparsed) {
	s := string(data)
	p := parser{doc: doc, r: r, s: s, lastEnd: -1}
	p.parseDirectives()
	inFence := false
	lineNo := 0
	for lo := 0; lo < len(s); {
		hi := strings.IndexByte(s[lo:], '\n')
		next := len(s)
		if hi < 0 {
			hi = len(s)
		} else {
			hi += lo
			next = hi + 1
		}
		text := s[lo:hi]
		text = strings.TrimSuffix(text, "\r")
		lineNo++
		trimmed := strings.TrimLeft(text, " \t")
		first := len(p.cites)
		switch {
		case strings.HasPrefix(trimmed, "```") || strings.HasPrefix(trimmed, "~~~"):
			inFence = !inFence
			p.newUnit()
		case trimmed == "":
			p.newUnit()
		case directiveRe.MatchString(text):
			p.newUnit()
		case inFence:
			p.newUnit()
			p.scanLine(text, lo, lineNo)
			p.newUnit()
		case strings.HasPrefix(trimmed, "#"):
			p.newUnit()
			p.section = strings.TrimSpace(strings.TrimLeft(trimmed, "#"))
			p.sectionFile = ctxPath{}
			p.sourceSeen = false
			p.scanLine(text, lo, lineNo)
			p.sectionFile = p.single(p.cites[first:], 0)
			p.sourceSeen = p.sectionFile.set
			p.newUnit()
		default:
			if unitStart.MatchString(trimmed) {
				p.newUnit()
			}
			p.scanLine(text, lo, lineNo)
			if sub, ok := p.subHeading(trimmed, p.cites[first:]); ok {
				// A line holding only "`path`:" heads what follows as a
				// heading would, until the next heading: its
				// parenthesized ":N" rows mean that file, not the file the
				// section heading named.  A path that resolves to no file
				// leaves no source file at all.
				p.sectionFile = sub
				p.sourceSeen = true
			} else if i := strings.Index(text, "Source:"); i >= 0 && !p.sourceSeen {
				p.sourceSeen = true
				p.sectionFile = p.single(p.cites[first:], lo+i)
			}
		}
		lo = next
	}
	return p.cites, p.unparsed
}

// subHeading reports whether a line is a file sub-heading, a path mention
// alone followed by a colon ("`internal/rpc/rpcserver/log.go`:", also in
// bold), and returns the path it names (unset when the path resolves to no
// one file).
func (p *parser) subHeading(trimmed string, cites []Citation) (ctxPath, bool) {
	if len(cites) != 1 || cites[0].Form != FormMention {
		return ctxPath{}, false
	}
	t, ok := strings.CutSuffix(strings.TrimRight(trimmed, " \t\r"), ":")
	if !ok {
		return ctxPath{}, false
	}
	t = strings.Trim(t, "*_")
	t = strings.TrimSuffix(strings.TrimPrefix(t, "`"), "`")
	if t != cites[0].Written {
		return ctxPath{}, false
	}
	switch cites[0].Target {
	case TargetUnresolved, TargetAmbiguous:
		return ctxPath{}, true
	}
	return ctxOf(&cites[0]), true
}

// single returns the one distinct path the citations at or after offset
// off name, or an unset context when they name none or several.
func (p *parser) single(cites []Citation, off int) ctxPath {
	var out ctxPath
	for i := range cites {
		c := &cites[i]
		if c.Off < off || c.Form == FormContinuation || c.Form == FormNumericSpan {
			continue
		}
		key := c.Module + "@" + c.Version + "/" + c.Path
		if out.set && out.module+"@"+out.version+"/"+out.path != key {
			return ctxPath{}
		}
		out = ctxOf(c)
	}
	if out.path == "" {
		return ctxPath{}
	}
	return out
}

type parser struct {
	doc        string
	r          *Resolver
	s          string
	directives map[string]directive
	section    string
	// The section's source file: the one path its heading names, or else
	// the one path of its first "Source:" line.
	sectionFile ctxPath
	sourceSeen  bool
	// The last citation or mention with a written path in the paragraph.
	unitPath ctxPath
	// The last citation or mention in the paragraph: its path and the
	// document offset just after it.
	last    ctxPath
	lastEnd int
	inCode  bool
	// The open code span's delimiter length and whether it holds a
	// citation; a span may continue onto the next line of a paragraph.
	spanDelim   int
	spanHasCite bool
	cites       []Citation
	unparsed    []Unparsed
}

func (p *parser) newUnit() {
	p.last = ctxPath{}
	p.unitPath = ctxPath{}
	p.lastEnd = -1
	p.inCode = false
}

func (p *parser) unparsedAt(line int, raw, reason string) {
	p.unparsed = append(p.unparsed, Unparsed{Doc: p.doc, Line: line, Raw: raw, Reason: reason})
}

// add records a citation and makes it the last one, ending at end.
func (p *parser) add(c Citation, end int) {
	p.cites = append(p.cites, c)
	p.last = ctxOf(&c)
	p.lastEnd = end
	if c.Form == FormPath || c.Form == FormMention {
		p.unitPath = p.last
	}
	if p.inCode && c.Form != FormMention {
		p.spanHasCite = true
	}
}

// scanLine scans one line; lo is the line's offset in the document.
func (p *parser) scanLine(text string, lo, lineNo int) {
	lastCiteSpanEnd := -1 // index just after the closing backticks of a code span holding a citation
	for i := 0; i < len(text); {
		c := text[i]
		switch {
		case c == '`':
			k := i
			for k < len(text) && text[k] == '`' {
				k++
			}
			run := k - i
			if p.inCode {
				if run == p.spanDelim {
					p.inCode = false
					if p.spanHasCite {
						lastCiteSpanEnd = k
					} else {
						lastCiteSpanEnd = -1
					}
				}
				i = k
				continue
			}
			// Opening a span.  A numeric span is consumed whole here.
			if cl := strings.Index(text[k:], strings.Repeat("`", run)); cl >= 0 {
				content := text[k : k+cl]
				if p.numericSpan(text, lo, lineNo, i, k, content, lastCiteSpanEnd) {
					i = k + cl + run
					lastCiteSpanEnd = i
					p.lastEnd = lo + i
					continue
				}
			}
			p.inCode = true
			p.spanDelim = run
			p.spanHasCite = false
			i = k
			continue
		case c == ':' && i+1 < len(text) && isDigit(text[i+1]) && (i == 0 || strings.IndexByte(" (`,[;\t", text[i-1]) >= 0):
			items, end, reason := parseItems(text, i+1, lo)
			if len(items) == 0 {
				p.unparsedAt(lineNo, snippet(text, i, i+12), "continuation: "+reason)
				i++
				continue
			}
			p.add(p.continuation(lineNo, lo+i, text[i:end], items), lo+end)
			i = end
			continue
		case isPathChar(c):
			j := i
			for j < len(text) && isPathChar(text[j]) {
				j++
			}
			tok := text[i:j]
			// Trailing dots end sentences; leading punctuation is not part
			// of a path.
			tokStart := i
			for len(tok) > 0 && tok[len(tok)-1] == '.' {
				tok = tok[:len(tok)-1]
			}
			for len(tok) > 0 && strings.IndexByte(".-+~", tok[0]) >= 0 && !strings.HasPrefix(tok, "./") && !strings.HasPrefix(tok, "../") {
				tok = tok[1:]
				tokStart++
			}
			ext := extOf(tok)
			followed := j < len(text) && text[j] == ':' && j+1 < len(text) && isDigit(text[j+1]) && len(tok) == j-tokStart
			if ext == "" || !knownExt[ext] {
				if followed && ext != "" && !isDigit(ext[0]) && strings.Contains(tok, "/") {
					p.unparsedAt(lineNo, snippet(text, tokStart, j+8), "unknown extension ."+ext)
				}
				i = j
				if followed {
					// Skip the number so it is not read as a continuation.
					_, e, _ := numAt(text, j+1)
					i = e
				}
				continue
			}
			if followed {
				items, end, reason := parseItems(text, j+1, lo)
				if len(items) == 0 {
					p.unparsedAt(lineNo, snippet(text, tokStart, j+12), reason)
					_, e, _ := numAt(text, j+1)
					i = e
					continue
				}
				p.add(p.resolve(tok, FormPath, lineNo, lo+tokStart, text[tokStart:end], items), lo+end)
				i = end
				continue
			}
			p.add(p.resolve(tok, FormMention, lineNo, lo+tokStart, tok, nil), lo+tokStart+len(tok))
			i = j
			continue
		}
		i++
	}
}

func snippet(s string, i, j int) string {
	if j > len(s) {
		j = len(s)
	}
	if i < 0 {
		i = 0
	}
	return s[i:j]
}

// numericSpan handles a code span whose content is only line numbers.  It
// returns true when the span was taken as a citation.
func (p *parser) numericSpan(text string, lo, lineNo, open, contentStart int, content string, lastCiteSpanEnd int) bool {
	if content == "" || !isDigit(content[0]) {
		return false
	}
	items, end, _ := parseItems(content, 0, lo+contentStart)
	if len(items) == 0 || end != len(content) {
		return false
	}
	sep := ""
	if lastCiteSpanEnd >= 0 && lastCiteSpanEnd <= open {
		sep = text[lastCiteSpanEnd:open]
	}
	if lastCiteSpanEnd < 0 || !spanSeps[sep] || !p.last.set {
		if strings.ContainsAny(content, ",-") || strings.Contains(content, enDash) {
			p.unparsedAt(lineNo, "`"+content+"`", "line numbers without a path")
		}
		return false
	}
	if sep != enDash && !spanFollowOK(text, contentStart+len(content)+(contentStart-open)) {
		p.unparsedAt(lineNo, "`"+content+"`", "numeric span followed by a word: a value, or a line?")
		return false
	}
	if sep == enDash {
		// Closes the range opened by the previous citation's last item.
		prev := &p.cites[len(p.cites)-1]
		if len(items) != 1 || items[0].Start != items[0].End || len(prev.Items) == 0 {
			p.unparsedAt(lineNo, "`"+content+"`", "en-dash span that is not a range end")
			return false
		}
		last := &prev.Items[len(prev.Items)-1]
		if last.End != last.Start || items[0].Start < last.Start {
			p.unparsedAt(lineNo, "`"+content+"`", "en-dash span that is not a range end")
			return false
		}
		last.End = items[0].Start
		last.EndOff = items[0].StartOff
		last.EndLen = items[0].StartLen
		prev.Raw += enDash + "`" + content + "`"
		return true
	}
	c := Citation{Doc: p.doc, Line: lineNo, Off: lo + open, Raw: "`" + content + "`", Form: FormNumericSpan, Items: items, Section: p.section}
	fill(&c, p.last)
	p.add(c, lo+open)
	return true
}

// spanFollowOK reports whether the text after a numeric span (from text[i])
// ends the item: the line's end, punctuation that closes or separates
// (",", ";", ":", ")", "]", "|", "*", or "." not followed by a letter or
// digit), the words "and" or "or" before the line's end or another code
// span, or a parenthesized code span ("(`Name`)").  A word after it
// ("`4941` atoms") makes it a value.
func spanFollowOK(text string, i int) bool {
	for i < len(text) && text[i] == ' ' {
		i++
	}
	if i >= len(text) {
		return true
	}
	switch text[i] {
	case ',', ';', ':', ')', ']', '|', '*':
		return true
	case '.':
		return i+1 >= len(text) || !isAlnum(text[i+1])
	case '(':
		return strings.HasPrefix(text[i+1:], "`")
	}
	for _, w := range []string{"and", "or"} {
		if rest, ok := strings.CutPrefix(text[i:], w); ok {
			t := strings.TrimLeft(rest, " ")
			if t == "" || (len(t) < len(rest) && strings.HasPrefix(t, "`")) {
				return true
			}
		}
	}
	return false
}

func ctxOf(c *Citation) ctxPath {
	return ctxPath{set: true, target: c.Target, how: c.How, path: c.Path, module: c.Module, version: c.Version, hints: c.Hints}
}

func fill(c *Citation, x ctxPath) {
	c.Target, c.How, c.Path, c.Module, c.Version, c.Hints = x.target, x.how, x.path, x.module, x.version, x.hints
}

func sameFile(a, b ctxPath) bool {
	return a.path == b.path && a.module == b.module && a.version == b.version
}

// parenthesized reports whether the text before off ends in "(" or "(`".
func parenthesized(s string, off int) bool {
	t := strings.TrimSuffix(s[:off], "`")
	return strings.HasSuffix(t, "(")
}

// continuation resolves ":items": the path of the citation or mention it
// directly follows in a list, else, when it opens a parenthesis, the
// section's source file.  A port written as ":9110" outside parentheses
// is therefore never taken for a line.
func (p *parser) continuation(lineNo, off int, raw string, items []Item) Citation {
	c := Citation{Doc: p.doc, Line: lineNo, Off: off, Raw: raw, Form: FormContinuation, Items: items, Section: p.section}
	switch {
	case p.last.set && p.lastEnd >= 0 && p.lastEnd <= off && listGap.MatchString(p.s[p.lastEnd:off]):
		fill(&c, p.last)
	case p.sectionFile.set && parenthesized(p.s, off) && (!p.unitPath.set || sameFile(p.unitPath, p.sectionFile)):
		fill(&c, p.sectionFile)
		if c.How == HowExplicit || c.How == HowInherited {
			c.How = HowSection
		}
		c.Note = "path from the section's source file"
	case p.sectionFile.set && parenthesized(p.s, off):
		// Authors wrote both conventions: after prose, ":N" means the
		// paragraph's last file in some places and the section's file in
		// others, so a paragraph naming another file leaves it open.
		c.Target, c.How = TargetUnresolved, HowBare
		c.Note = "after prose in a paragraph naming " + p.unitPath.path + ", in a section naming " + p.sectionFile.path
	default:
		c.Target, c.How = TargetUnresolved, HowBare
		c.Note = "not in a list after a citation, and not in parentheses in a section that names one source file"
	}
	return c
}

// resolve resolves a written path.
func (p *parser) resolve(tok string, form Form, lineNo, off int, raw string, items []Item) Citation {
	c := Citation{Doc: p.doc, Line: lineNo, Off: off, Raw: raw, Written: tok, Form: form, Items: items, Section: p.section}
	r := p.r
	if at := strings.IndexByte(tok, '@'); at > 0 {
		rest := tok[at+1:]
		if sl := strings.IndexByte(rest, '/'); sl > 0 {
			c.Module, c.Version, c.Path = tok[:at], rest[:sl], rest[sl+1:]
			c.Target = TargetDependency
			if c.Module == "golang.org/toolchain" {
				c.Target = TargetToolchain
			}
			return c
		}
		c.Target, c.How = TargetUnresolved, HowBare
		c.Note = "module@version with no path"
		return c
	}
	if first, rest, ok := strings.Cut(tok, "/"); ok && toolchainSeg.MatchString(first) {
		c.Target, c.Module, c.Version, c.Path = TargetToolchain, "go", first, rest
		return c
	}
	clean := path.Clean(tok)
	if strings.HasPrefix(clean, "../") || strings.HasPrefix(clean, "/") {
		c.Target, c.How, c.Path = TargetUnresolved, HowBare, tok
		c.Note = "path outside the repository root"
		return c
	}
	hasSlash := strings.Contains(clean, "/")
	up, loc := r.upstream[clean], r.local(clean)
	if hasSlash {
		switch {
		case up && loc:
			c.Target, c.Path = TargetAmbiguous, clean
			return c
		case up:
			c.Target, c.Path = TargetUpstream, clean
			return c
		case loc:
			c.Target, c.Path = TargetLocal, clean
			return c
		}
	}
	if d, ok := p.directives[clean]; ok {
		c.How = HowDirective
		c.Target, c.Path, c.Module, c.Version = d.target, d.path, d.module, d.version
		if d.target == TargetUnresolved {
			c.Path = clean
			c.Note = "declared not upstream by a directive"
		}
		return c
	}
	// Complete from the citation or mention this one directly follows in a
	// list.  An upstream completion is not taken for a name that is also a
	// file at the upstream root, which the convention says a bare name is.
	if p.last.set && p.last.path != "" && p.lastEnd >= 0 && p.lastEnd <= off && listGap.MatchString(p.s[p.lastEnd:off]) {
		dir := path.Dir(p.last.path)
		cand := path.Join(dir, clean)
		switch p.last.target {
		case TargetUpstream:
			if dir != "." && r.upstream[cand] && !up {
				c.Target, c.How, c.Path = TargetUpstream, HowInherited, cand
				return c
			}
		case TargetDependency, TargetToolchain:
			// As for an upstream completion, a name that is a file at the
			// upstream root is not taken into the module ("bbolt@v1.3.12/
			// db.go:646-657; dcrwallet.go:176-189" names dcrwallet's own
			// file), nor a name the module is known not to have.
			if !up && !r.depAbsent(p.last.module, p.last.version, cand) {
				c.Target, c.How, c.Path, c.Module, c.Version = p.last.target, HowInherited, cand, p.last.module, p.last.version
				return c
			}
		case TargetLocal:
			if r.local(cand) {
				c.Target, c.How, c.Path = TargetLocal, HowInherited, cand
				return c
			}
		}
	}
	if !hasSlash {
		switch {
		case up && loc:
			c.Target, c.How, c.Path = TargetAmbiguous, HowBareRoot, clean
			return c
		case up && len(r.base[clean]) == 1 && (extOf(clean) == "go" || strings.Contains(clean, "dcrwallet")):
			c.Target, c.How, c.Path = TargetUpstream, HowRoot, clean
			return c
		case up:
			c.Target, c.How, c.Path = TargetUpstream, HowBareRoot, clean
			c.Hints = r.base[clean]
			return c
		case loc:
			c.Target, c.Path = TargetLocal, clean
			return c
		}
	}
	c.Target, c.How, c.Path = TargetUnresolved, HowBare, clean
	c.Hints = r.base[path.Base(clean)]
	return c
}
