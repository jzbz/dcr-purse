// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

package main

import (
	"fmt"
	"strings"
	"testing"
)

// upstreamFiles is the synthetic upstream tree the grammar tests resolve
// against.
var upstreamFiles = []string{
	"config.go", "log.go", "dcrwallet.go", "README.md", "SECURITY.md", "sample-dcrwallet.conf",
	"wallet/wallet.go", "wallet/log.go", "wallet/createtx.go",
	"wallet/udb/txmined.go", "wallet/udb/txdb.go", "wallet/udb/log.go",
	"internal/rpc/jsonrpc/methods.go", "internal/rpc/jsonrpc/config.go",
	"internal/rpchelp/methods.go", "spv/sync.go", "testsolver.go", "rpcserver.go",
	"internal/rpc/rpcserver/server.go", "internal/rpc/rpcserver/log.go",
}

var localFiles = map[string]bool{"README.md": true, "crates/purse-testutil/src/lib.rs": true, "docs/adr/0016-upstream-tracking.md": true}

func testResolver() *Resolver {
	return NewResolver(upstreamFiles, func(p string) bool { return localFiles[p] })
}

// show renders citations compactly: form, target, how, path and items.
func show(cs []Citation) []string {
	var out []string
	for _, c := range cs {
		var items []string
		for _, it := range c.Items {
			items = append(items, itemText(it))
		}
		p := c.Path
		if c.Module != "" {
			p = c.Module + "@" + c.Version + "/" + c.Path
		}
		out = append(out, fmt.Sprintf("%s %s %s %s [%s]", c.Form, c.Target, c.How, p, strings.Join(items, " ")))
	}
	return out
}

func TestGrammar(t *testing.T) {
	cases := []struct {
		name string
		doc  string
		want []string
	}{
		{"single", "See `wallet/wallet.go:120`.", []string{"path:line upstream explicit wallet/wallet.go [120]"}},
		{"range", "(`wallet/wallet.go:120-130`)", []string{"path:line upstream explicit wallet/wallet.go [120-130]"}},
		{"en dash range", "wallet/wallet.go:120–130 here", []string{"path:line upstream explicit wallet/wallet.go [120-130]"}},
		{"comma list", "Source: wallet/udb/txmined.go:550,570,645-650", []string{"path:line upstream explicit wallet/udb/txmined.go [550 570 645-650]"}},
		{"comma space list in code", "`wallet/wallet.go:487-526, 1351-1355`", []string{"path:line upstream explicit wallet/wallet.go [487-526 1351-1355]"}},
		{"comma space then prose", "wallet/wallet.go:12, 3 lines later", []string{"path:line upstream explicit wallet/wallet.go [12]"}},
		{"two paths", "Source: wallet/wallet.go:5, spv/sync.go:7", []string{
			"path:line upstream explicit wallet/wallet.go [5]", "path:line upstream explicit spv/sync.go [7]"}},
		{"continuations", "Source: wallet/wallet.go:280, :505,\n :645-651; :700.", []string{
			"path:line upstream explicit wallet/wallet.go [280]",
			":line upstream explicit wallet/wallet.go [505]",
			":line upstream explicit wallet/wallet.go [645-651]",
			":line upstream explicit wallet/wallet.go [700]"}},
		{"continuation after mention", "the loop in `spv/sync.go` at `:500`", []string{
			"mention upstream explicit spv/sync.go []", ":line upstream explicit spv/sync.go [500]"}},
		{"continuation after prose, no section", "In `wallet/wallet.go:5` the call (`:9`) returns.", []string{
			"path:line upstream explicit wallet/wallet.go [5]", ":line unresolved bare  [9]"}},
		{"numeric spans", "| `wallet/udb/txmined.go:392`, `686`; `700-710` |", []string{
			"path:line upstream explicit wallet/udb/txmined.go [392]",
			"`line` span upstream explicit wallet/udb/txmined.go [686]",
			"`line` span upstream explicit wallet/udb/txmined.go [700-710]"}},
		{"en dash spans", "`spv/sync.go:205`–`206` and (`:2352`–`2356`)", []string{
			"path:line upstream explicit spv/sync.go [205-206]",
			":line upstream explicit spv/sync.go [2352-2356]"}},
		{"numeric span without citation", "candidate path (`3277-3309`)", nil},
		{"dependency", "`dcrd/wire@v1.8.0/msgtx.go:31`, `msgblock.go:7` and `:9`", []string{
			"path:line dependency explicit dcrd/wire@v1.8.0/msgtx.go [31]",
			"path:line dependency inherited dcrd/wire@v1.8.0/msgblock.go [7]",
			":line dependency inherited dcrd/wire@v1.8.0/msgblock.go [9]"}},
		{"toolchain", "go1.27.4/src/flag/flag.go:607", []string{"path:line toolchain explicit go@go1.27.4/src/flag/flag.go [607]"}},
		{"inherited upstream", "`wallet/udb/txmined.go:12`, `txdb.go:5`", []string{
			"path:line upstream explicit wallet/udb/txmined.go [12]",
			"path:line upstream inherited wallet/udb/txdb.go [5]"}},
		{"no inheritance of a root name", "`wallet/udb/txmined.go:12`, `log.go:5`", []string{
			"path:line upstream explicit wallet/udb/txmined.go [12]",
			"path:line upstream bare-root log.go [5]"}},
		{"no inheritance after prose", "`wallet/udb/txmined.go:12` calls `txdb.go:5`", []string{
			"path:line upstream explicit wallet/udb/txmined.go [12]",
			"path:line unresolved bare txdb.go [5]"}},
		{"root rule", "`dcrwallet.go:36-39`", []string{"path:line upstream root dcrwallet.go [36-39]"}},
		{"root rule for the sample config", "`sample-dcrwallet.conf:12`", []string{"path:line upstream root sample-dcrwallet.conf [12]"}},
		{"shared root name", "`config.go:739-758`", []string{"path:line upstream bare-root config.go [739-758]"}},
		{"generic root name", "`SECURITY.md:3`", []string{"path:line upstream bare-root SECURITY.md [3]"}},
		{"local and upstream", "`README.md:8`", []string{"path:line ambiguous bare-root README.md [8]"}},
		{"local", "`crates/purse-testutil/src/lib.rs:31`", []string{"path:line local explicit crates/purse-testutil/src/lib.rs [31]"}},
		{"unresolved bare", "`methods.go:3741`", []string{"path:line unresolved bare methods.go [3741]"}},
		{"time is not a continuation", "at 12:30 and ratio 3:1", nil},
		{"port after prose", "Examples: `localhost` -> `localhost:9110`; `\"\"` -> `:9110`", []string{":line unresolved bare  [9110]"}},
		{"url is not a path", "https://example.com:443/x and 127.0.0.1:9108", nil},
		{"line and column", "wallet/wallet.go:12:5", nil},
		{"date after comma", "wallet/wallet.go:12, 2024-05-16", []string{"path:line upstream explicit wallet/wallet.go [12]"}},
		// A numeric span followed by a word is a value, not a line.
		{"numeric span then a word", "A2 `wallet/wallet.go:2328`, `4941` atoms are the limit.", []string{
			"path:line upstream explicit wallet/wallet.go [2328]"}},
		// A code span between ends the chain ("(`ListUnspent`), `4048`").
		{"numeric span then punctuation, and, or a code annotation", "`wallet/wallet.go:2906`, `3525`, `3531` (`ListUnspent`), `4048`. And `spv/sync.go:5` and `7` and `spv/sync.go:9`; `spv/sync.go:20`, `22` or\n`30`", []string{
			"path:line upstream explicit wallet/wallet.go [2906]",
			"`line` span upstream explicit wallet/wallet.go [3525]",
			"`line` span upstream explicit wallet/wallet.go [3531]",
			"path:line upstream explicit spv/sync.go [5]",
			"`line` span upstream explicit spv/sync.go [7]",
			"path:line upstream explicit spv/sync.go [9]",
			"path:line upstream explicit spv/sync.go [20]",
			"`line` span upstream explicit spv/sync.go [22]"}},
		{"en-dash span then a word", "`spv/sync.go:205`–`206` documents it", []string{
			"path:line upstream explicit spv/sync.go [205-206]"}},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			cs, _ := ParseDoc("docs/specs/01-x.md", []byte(tc.doc), testResolver())
			got := show(cs)
			if strings.Join(got, "\n") != strings.Join(tc.want, "\n") {
				t.Errorf("doc %q\n got:\n  %s\nwant:\n  %s", tc.doc, strings.Join(got, "\n  "), strings.Join(tc.want, "\n  "))
			}
		})
	}
}

func TestSectionSourceFile(t *testing.T) {
	doc := `### 4.1 Agenda preferences

Source: ` + "`wallet/udb/txmined.go`" + `.

| Item | Encoding |
|---|---|
| Default | bucket (` + "`wallet/udb/txmined.go:19`" + `). |
| Per-ticket | bucket (` + "`:20`" + `), created on first write (` + "`:40`" + `). |
| Key | order (` + "`wallet/udb/txdb.go:123`" + `) then the ID (` + "`:23`" + `). |

### 4.2 Prune hook (` + "`wallet/createtx.go:241-274`" + `)

The hook (` + "`:250`" + `) and the port ` + "`:9110`" + `.
`
	cs, un := ParseDoc("docs/specs/06-x.md", []byte(doc), testResolver())
	var got []string
	for _, s := range show(cs) {
		if !strings.HasPrefix(s, "mention") {
			got = append(got, s)
		}
	}
	want := []string{
		"path:line upstream explicit wallet/udb/txmined.go [19]",
		":line upstream section wallet/udb/txmined.go [20]",
		":line upstream section wallet/udb/txmined.go [40]",
		"path:line upstream explicit wallet/udb/txdb.go [123]",
		// After prose in a row naming txdb.go, in a section naming
		// txmined.go: left open.
		":line unresolved bare  [23]",
		"path:line upstream explicit wallet/createtx.go [241-274]",
		":line upstream section wallet/createtx.go [250]",
		":line unresolved bare  [9110]",
	}
	if strings.Join(got, "\n") != strings.Join(want, "\n") {
		t.Errorf("got:\n  %s\nwant:\n  %s", strings.Join(got, "\n  "), strings.Join(want, "\n  "))
	}
	if len(un) != 0 {
		t.Errorf("unparsed: %v", un)
	}
}

// TestSubHeading: spec 20's shape.  The section heading names server.go;
// a line holding only "`.../log.go`:" heads the table after it, whose
// parenthesized rows mean log.go.  A sub-heading naming no file leaves the
// rows unresolved.
func TestSubHeading(t *testing.T) {
	doc := "### 3.1 Inventory of `internal/rpc/rpcserver/server.go`\n\n" +
		"| Name | Contract |\n|---|---|\n| `zero` | (`:491`) |\n\n" +
		"`internal/rpc/rpcserver/log.go`:\n\n" +
		"| Name | Signature |\n|---|---|\n| `UseLogger` | `func(l slog.Logger)` (`:27`) |\n| `Fatal` | (`:56-69`) |\n\n" +
		"**`nowhere/x.go`**:\n\n| `y` | (`:5`) |\n\n" +
		"### 3.2 Next (`internal/rpc/rpcserver/server.go`)\n\n| `z` | (`:7`) |\n"
	cs, _ := ParseDoc("docs/specs/20-x.md", []byte(doc), testResolver())
	var got []string
	for _, s := range show(cs) {
		if !strings.HasPrefix(s, "mention") {
			got = append(got, s)
		}
	}
	want := []string{
		":line upstream section internal/rpc/rpcserver/server.go [491]",
		":line upstream section internal/rpc/rpcserver/log.go [27]",
		":line upstream section internal/rpc/rpcserver/log.go [56-69]",
		":line unresolved bare  [5]",
		":line upstream section internal/rpc/rpcserver/server.go [7]",
	}
	if strings.Join(got, "\n") != strings.Join(want, "\n") {
		t.Errorf("got:\n  %s\nwant:\n  %s", strings.Join(got, "\n  "), strings.Join(want, "\n  "))
	}
}

// TestDependencyCompletion: QK-0017's, QK-0326's and QK-0545's shapes.  A
// bare name after a dependency citation is not taken into the module when
// it is a file at the upstream root, nor when the module is known not to
// have it.
func TestDependencyCompletion(t *testing.T) {
	r := testResolver()
	r.SetDepChecker(func(module, version, file string) (bool, bool) {
		switch module + "@" + version {
		case "bbolt@v1.3.12":
			return file == "tx.go", true
		}
		return false, false
	})
	doc := "`bbolt@v1.3.12/db.go:646-657`; `dcrwallet.go:176-189`\n\n" +
		"`cspp/v2@v2.4.0/solverrpc/rpc.go:1`; `testsolver.go:18-35`; `config.go:739-758`\n\n" +
		"`grpc@v1.83.2/grpclog/internal/logger.go:3`; `rpcserver.go:406-448`\n\n" +
		"`bbolt@v1.3.12/db.go:1`, `tx.go:5`, `nothere.go:6`\n\n" +
		"`dcrd/wire@v1.8.0/msgtx.go:31`, `msgblock.go:7`\n"
	cs, _ := ParseDoc("docs/QUIRKS.md", []byte(doc), r)
	want := []string{
		"path:line dependency explicit bbolt@v1.3.12/db.go [646-657]",
		"path:line upstream root dcrwallet.go [176-189]",
		"path:line dependency explicit cspp/v2@v2.4.0/solverrpc/rpc.go [1]",
		"path:line upstream root testsolver.go [18-35]",
		"path:line upstream bare-root config.go [739-758]",
		"path:line dependency explicit grpc@v1.83.2/grpclog/internal/logger.go [3]",
		"path:line upstream root rpcserver.go [406-448]",
		"path:line dependency explicit bbolt@v1.3.12/db.go [1]",
		"path:line dependency inherited bbolt@v1.3.12/tx.go [5]",
		"path:line unresolved bare nothere.go [6]",
		"path:line dependency explicit dcrd/wire@v1.8.0/msgtx.go [31]",
		// The module is not in the cache: completed as before.
		"path:line dependency inherited dcrd/wire@v1.8.0/msgblock.go [7]",
	}
	if got := show(cs); strings.Join(got, "\n") != strings.Join(want, "\n") {
		t.Errorf("got:\n  %s\nwant:\n  %s", strings.Join(got, "\n  "), strings.Join(want, "\n  "))
	}
}

func TestModCacheChecker(t *testing.T) {
	cache := t.TempDir()
	writeT(t, cache, "go.etcd.io/bbolt@v1.3.12/db.go", "package bbolt\n")
	writeT(t, cache, "github.com/!burnt!sushi/toml@v1.0.0/decode.go", "package toml\n")
	mod := ParseGoMod([]byte("module x\n\nrequire (\n\tgo.etcd.io/bbolt v1.3.12\n\tgithub.com/BurntSushi/toml v1.0.0\n)\n"))
	check := modCacheChecker(cache, mod)
	for _, tc := range []struct {
		module, version, file string
		exists, known         bool
	}{
		{"bbolt", "v1.3.12", "db.go", true, true},
		{"bbolt", "v1.3.12", "dcrwallet.go", false, true},
		{"bbolt", "v1.3.11", "db.go", false, false},    // not in the cache
		{"grpc", "v1.83.2", "server.go", false, false}, // not required
		{"BurntSushi/toml", "v1.0.0", "decode.go", true, true},
	} {
		e, k := check(tc.module, tc.version, tc.file)
		if e != tc.exists || k != tc.known {
			t.Errorf("%s@%s/%s: %v %v", tc.module, tc.version, tc.file, e, k)
		}
	}
}

func TestDirectives(t *testing.T) {
	doc := "<!-- pinmove: methods.go=internal/rpc/jsonrpc/methods.go client.go=dcrd/mixing@v0.7.5/mixclient/client.go helper.go=- bad.go=nowhere/bad.go -->\n\n" +
		"`methods.go:3741`, `client.go:12`, `helper.go:3`, `bad.go:4`\n"
	cs, un := ParseDoc("docs/specs/17-x.md", []byte(doc), testResolver())
	want := []string{
		"path:line upstream directive internal/rpc/jsonrpc/methods.go [3741]",
		"path:line dependency directive dcrd/mixing@v0.7.5/mixclient/client.go [12]",
		"path:line unresolved directive helper.go [3]",
		"path:line unresolved bare bad.go [4]",
	}
	if got := show(cs); strings.Join(got, "\n") != strings.Join(want, "\n") {
		t.Errorf("got:\n  %s\nwant:\n  %s", strings.Join(got, "\n  "), strings.Join(want, "\n  "))
	}
	if len(un) != 1 || !strings.Contains(un[0].Reason, "not an upstream file") {
		t.Errorf("unparsed: %v", un)
	}
	if !cs[0].Mapped() || cs[1].Mapped() || cs[2].Mapped() {
		t.Error("only the upstream directive is mapped")
	}
}

// TestNumericSpanValue: a value after a citation is recorded as unparsed,
// so the survey shows it, and never becomes an item cite rewrites.
func TestNumericSpanValue(t *testing.T) {
	_, un := ParseDoc("d.md", []byte("`wallet/wallet.go:2328`, `4941` atoms\n"), testResolver())
	if len(un) != 1 || un[0].Raw != "`4941`" || !strings.Contains(un[0].Reason, "followed by a word") {
		t.Errorf("unparsed: %+v", un)
	}
}

func TestUnparsedReasons(t *testing.T) {
	doc := "`wallet/wallet.go:474–`, `:606–` and `wallet/wallet.go:0`, (`:25-…`) and `spv/sync.go:9-3`\n"
	_, un := ParseDoc("docs/specs/07-x.md", []byte(doc), testResolver())
	var got []string
	for _, u := range un {
		got = append(got, u.Reason)
	}
	want := []string{
		"range without an end",
		"continuation: range without an end",
		"line zero or a range that ends before it starts",
		"continuation: range without an end",
		"line zero or a range that ends before it starts",
	}
	if strings.Join(got, "|") != strings.Join(want, "|") {
		t.Errorf("got %q, want %q", got, want)
	}
}

// TestItemOffsets checks that every item's offsets point at its digits, in
// every form, including a range closed by an en-dash span.
func TestItemOffsets(t *testing.T) {
	doc := "Source: wallet/wallet.go:280-290, :505,\n :645–651; (`spv/sync.go:205`–`206`, `7`)\r\n| `wallet/udb/txmined.go:1,2, 3` |\n"
	cs, _ := ParseDoc("d.md", []byte(doc), testResolver())
	n := 0
	for _, c := range cs {
		for _, it := range c.Items {
			if got := doc[it.StartOff : it.StartOff+it.StartLen]; got != fmt.Sprint(it.Start) {
				t.Errorf("%s: start offset reads %q, want %d", c.Raw, got, it.Start)
			}
			if it.EndOff >= 0 {
				if got := doc[it.EndOff : it.EndOff+it.EndLen]; got != fmt.Sprint(it.End) {
					t.Errorf("%s: end offset reads %q, want %d", c.Raw, got, it.End)
				}
			} else if it.Start != it.End {
				t.Errorf("%s: a range without an end offset", c.Raw)
			}
			n++
		}
	}
	if n != 8 {
		t.Errorf("parsed %d items, want 8: %v", n, show(cs))
	}
}

func FuzzParseDoc(f *testing.F) {
	for _, s := range []string{
		"Source: wallet/wallet.go:280, :505,\n :645-651; :700.",
		"| `wallet/udb/txmined.go:392`, `686`; `700-710` |",
		"`spv/sync.go:205`–`206` and (`:2352`–`2356`)",
		"<!-- pinmove: methods.go=internal/rpc/jsonrpc/methods.go -->\n`methods.go:1`",
		"### H (`wallet/createtx.go:1`)\n\n(`:2`)",
	} {
		f.Add(s)
	}
	r := testResolver()
	f.Fuzz(func(t *testing.T, doc string) {
		cs, _ := ParseDoc("d.md", []byte(doc), r)
		for _, c := range cs {
			for _, it := range c.Items {
				if it.Start < 1 || it.End < it.Start {
					t.Fatalf("bad item %+v", it)
				}
				if doc[it.StartOff:it.StartOff+it.StartLen] != fmt.Sprint(it.Start) {
					t.Fatalf("start offset of %+v", it)
				}
				if it.EndOff >= 0 && doc[it.EndOff:it.EndOff+it.EndLen] != fmt.Sprint(it.End) {
					t.Fatalf("end offset of %+v", it)
				}
			}
		}
	})
}
