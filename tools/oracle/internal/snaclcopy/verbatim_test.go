// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

package snaclcopy

import (
	"bytes"
	"encoding/json"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"testing"
)

// TestCopyIsVerbatim fails unless snacl.go is byte-identical, apart from its
// package clause, to wallet/internal/snacl/snacl.go of the dcrwallet module
// version go.mod pins, as found in the module cache (downloaded first if it
// is not there yet).
func TestCopyIsVerbatim(t *testing.T) {
	dir := moduleDir(t, "decred.org/dcrwallet/v5")
	upstream, err := os.ReadFile(filepath.Join(dir, "wallet", "internal", "snacl", "snacl.go"))
	if err != nil {
		t.Fatal(err)
	}
	local, err := os.ReadFile("snacl.go")
	if err != nil {
		t.Fatal(err)
	}
	// Normalize a checkout's line endings so the comparison holds on
	// Windows with core.autocrlf; the module cache always holds LF.
	local = bytes.ReplaceAll(local, []byte("\r\n"), []byte("\n"))

	const upstreamClause = "\npackage snacl\n"
	const localClause = "\npackage snaclcopy\n"
	if bytes.Count(upstream, []byte(upstreamClause)) != 1 {
		t.Fatalf("upstream snacl.go does not have exactly one %q line", strings.TrimSpace(upstreamClause))
	}
	if bytes.Count(local, []byte(localClause)) != 1 {
		t.Fatalf("snacl.go does not have exactly one %q line", strings.TrimSpace(localClause))
	}
	want := bytes.Replace(upstream, []byte(upstreamClause), []byte(localClause), 1)
	if !bytes.Equal(local, want) {
		t.Fatalf("snacl.go differs from %s beyond the package clause",
			filepath.Join(dir, "wallet", "internal", "snacl", "snacl.go"))
	}
}

// moduleDir returns the module cache directory of the version of module path
// that go.mod selects. It asks "go mod download -json", which downloads and
// extracts the module first when the cache does not hold it yet (as on a
// fresh machine or CI runner) and changes neither go.mod nor go.sum, whose
// sums for it are already recorded; it fails the test with Go's own message
// when that is impossible (no network and an empty cache, say).
func moduleDir(t *testing.T, path string) string {
	t.Helper()
	cmd := exec.Command("go", "mod", "download", "-json", path)
	var stderr bytes.Buffer
	cmd.Stderr = &stderr
	out, runErr := cmd.Output()
	var info struct {
		Version string
		Dir     string
		Error   string
	}
	if err := json.Unmarshal(out, &info); err != nil {
		t.Fatalf("go mod download -json %s: %v (output %q, stderr %q, decode: %v)",
			path, runErr, out, stderr.String(), err)
	}
	if info.Error != "" {
		t.Fatalf("go mod download %s: %s", path, info.Error)
	}
	if runErr != nil {
		t.Fatalf("go mod download %s: %v: %s", path, runErr, stderr.String())
	}
	if info.Dir == "" {
		t.Fatalf("go mod download %s@%s reported no module directory", path, info.Version)
	}
	return info.Dir
}
