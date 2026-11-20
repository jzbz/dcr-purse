// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

// Command pinmove is the tooling ADR-0016 (upstream tracking) calls for
// before the first move of the dcrwallet parity pin.  It reads a dcrwallet
// clone with git plumbing only and never fetches, commits or writes to it.
//
//	pinmove report  -upstream DIR [-from REV] -to REV [-json FILE] [-o FILE]
//	pinmove cite    -upstream DIR [-from REV] -to REV [-w] [-allow-dirty]
//	pinmove batteries -upstream DIR [-from REV] [-to REV] [-oracle DIR] [-json FILE] [-o FILE]
//	pinmove survey  -upstream DIR [-from REV]
//
// report writes the review list of ADR-0016 items 3 and 5.1: the upstream
// commits in the range, the files and hunks changed, every citation in
// README.md, the Markdown files under docs/ and the comments of the Rust
// and Go files under crates/, fuzz/ and tools/oracle (the oracle's copies
// of upstream source left out) that resolves to an upstream file,
// classified as same, shifted, touched, gone or invalid (the rest
// counted), the QUIRKS entries citing changed files, the per-document
// review list and the go.mod delta, replace directives included.  cite
// rewrites the shifted citations in the documents of ADR-0016 item 5.8
// (with -w; without it, only lists), marks each document it covers with
// the new revision so that a rerun skips it, and lists the rest for a
// human, the citations in code comments among them; it never edits
// docs/adr/ or any code.  batteries lists every frozen
// file under crates/*/tests/data, how it names the pin, and which
// regenerators the range requires, by the packages each oracle command
// links (`go list -deps` in tools/oracle); report includes the same.
// survey reports how the documents' citations parse under the grammar in
// citations.go.
//
// -from defaults to DCRWALLET_PARITY_COMMIT in crates/purse-testutil, so
// cite, which ADR-0016 item 5.8 runs after item 5.3 has bumped that
// constant, is given the old pin as -from.  An empty range (-from and -to
// one commit) is refused by cite and flagged by report.
// -repo defaults to the nearest directory above the working directory
// holding Cargo.toml and docs/.
package main

import (
	"errors"
	"flag"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"regexp"
)

func main() {
	if err := run(os.Args[1:], os.Stdout, os.Stderr); err != nil {
		fmt.Fprintln(os.Stderr, "pinmove:", err)
		os.Exit(1)
	}
}

const usage = `usage:
  pinmove report  -upstream DIR [-repo DIR] [-from REV] -to REV [-o FILE] [-json FILE]
  pinmove cite    -upstream DIR [-repo DIR] [-from REV] -to REV [-w] [-allow-dirty]
  pinmove batteries -upstream DIR [-repo DIR] [-from REV] [-to REV] [-oracle DIR] [-o FILE] [-json FILE]
  pinmove survey  -upstream DIR [-repo DIR] [-from REV] [-examples N]
`

func run(args []string, stdout, stderr io.Writer) error {
	if len(args) == 0 {
		fmt.Fprint(stderr, usage)
		return errors.New("no command")
	}
	cmd := args[0]
	fs := flag.NewFlagSet(cmd, flag.ContinueOnError)
	fs.SetOutput(stderr)
	repo := fs.String("repo", "", "dcr-purse repository root (default: found above the working directory)")
	upstream := fs.String("upstream", "", "dcrwallet git clone to read (required)")
	from := fs.String("from", "", "old revision (default: DCRWALLET_PARITY_COMMIT)")
	to := fs.String("to", "", "new revision")
	var out, jsonOut, oracle *string
	var write, allowDirty *bool
	var examples *int
	switch cmd {
	case "report", "batteries":
		out = fs.String("o", "", "write the Markdown report to this file instead of standard output")
		jsonOut = fs.String("json", "", "also write the report as JSON to this file")
		oracle = fs.String("oracle", "", "the oracle's Go module (default: <repo>/tools/oracle)")
	case "cite":
		write = fs.Bool("w", false, "rewrite the documents (default: list only)")
		allowDirty = fs.Bool("allow-dirty", false, "rewrite documents that have uncommitted changes")
	case "survey":
		examples = fs.Int("examples", 5, "examples listed per unparsed form")
	case "-h", "-help", "--help", "help":
		fmt.Fprint(stdout, usage)
		return nil
	default:
		fmt.Fprint(stderr, usage)
		return fmt.Errorf("unknown command %q", cmd)
	}
	if err := fs.Parse(args[1:]); err != nil {
		return err
	}
	if fs.NArg() != 0 {
		return fmt.Errorf("unexpected arguments: %v", fs.Args())
	}
	if *upstream == "" {
		return errors.New("-upstream is required")
	}
	root := *repo
	if root == "" {
		var err error
		if root, err = findRepo(); err != nil {
			return err
		}
	}
	if *from == "" {
		c, err := parityCommit(root)
		if err != nil {
			return err
		}
		*from = c
	}
	if cmd == "survey" || (cmd == "batteries" && *to == "") {
		*to = *from
	}
	if *to == "" {
		return errors.New("-to is required")
	}
	a, err := Analyze(root, *upstream, *from, *to, nil)
	if err != nil {
		return err
	}
	if oracle != nil {
		dir := *oracle
		if dir == "" {
			dir = filepath.Join(root, "tools", "oracle")
		}
		if st, err := os.Stat(filepath.Join(dir, "go.mod")); err == nil && st.Mode().IsRegular() {
			if a.Batteries, err = a.ComputeBatteries(RepoTable, dir, &GoLister{Dir: dir}); err != nil {
				return err
			}
		} else if cmd == "batteries" {
			return fmt.Errorf("no Go module at %s (pass -oracle)", dir)
		}
	}
	if cmd == "report" && a.Empty() {
		fmt.Fprintln(stderr, "pinmove: warning:", a.EmptyWarning())
	}
	switch cmd {
	case "report", "batteries":
		w := stdout
		if *out != "" {
			f, err := os.Create(*out)
			if err != nil {
				return err
			}
			defer f.Close()
			w = f
		}
		write := a.WriteReport
		writeJSON := a.WriteJSON
		if cmd == "batteries" {
			write = a.WriteBatteriesReport
			writeJSON = a.WriteBatteriesJSON
		}
		if err := write(w); err != nil {
			return err
		}
		if *jsonOut != "" {
			f, err := os.Create(*jsonOut)
			if err != nil {
				return err
			}
			defer f.Close()
			if err := writeJSON(f); err != nil {
				return err
			}
		}
		return nil
	case "cite":
		return a.Cite(stdout, *write, *allowDirty)
	default:
		if err := a.WriteSurvey(stdout, *examples); err != nil {
			return err
		}
		fmt.Fprintln(stdout)
		return a.WriteProposals(stdout)
	}
}

// findRepo walks up from the working directory to a directory holding
// Cargo.toml and docs/.
func findRepo() (string, error) {
	dir, err := os.Getwd()
	if err != nil {
		return "", err
	}
	for {
		st1, err1 := os.Stat(filepath.Join(dir, "Cargo.toml"))
		st2, err2 := os.Stat(filepath.Join(dir, "docs"))
		if err1 == nil && err2 == nil && st1.Mode().IsRegular() && st2.IsDir() {
			return dir, nil
		}
		parent := filepath.Dir(dir)
		if parent == dir {
			return "", errors.New("no repository root (Cargo.toml and docs/) above the working directory; pass -repo")
		}
		dir = parent
	}
}

var parityRe = regexp.MustCompile(`pub const DCRWALLET_PARITY_COMMIT: &str = "([0-9a-f]{40})";`)

// parityCommit reads DCRWALLET_PARITY_COMMIT from purse-testutil.
func parityCommit(repo string) (string, error) {
	fn := filepath.Join(repo, "crates", "purse-testutil", "src", "lib.rs")
	data, err := os.ReadFile(fn)
	if err != nil {
		return "", fmt.Errorf("reading the parity pin: %v (pass -from)", err)
	}
	m := parityRe.FindSubmatch(data)
	if m == nil {
		return "", fmt.Errorf("no DCRWALLET_PARITY_COMMIT in %s (pass -from)", fn)
	}
	return string(m[1]), nil
}
