// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

package main

import (
	"bytes"
	"encoding/json"
	"os"
	"os/exec"
	"path/filepath"
	"testing"
)

// hd2AddressFunc returns the text of the HD2Address function declaration in
// src: from its "func" line through the first line that is a lone "}".
func hd2AddressFunc(t *testing.T, name string, src []byte) []byte {
	t.Helper()
	src = bytes.ReplaceAll(src, []byte("\r\n"), []byte("\n"))
	const head = "\nfunc HD2Address("
	if bytes.Count(src, []byte(head)) != 1 {
		t.Fatalf("%s does not declare HD2Address exactly once", name)
	}
	start := bytes.Index(src, []byte(head)) + 1
	end := bytes.Index(src[start:], []byte("\n}\n"))
	if end < 0 {
		t.Fatalf("%s: HD2Address has no closing brace line", name)
	}
	return src[start : start+end+3]
}

// TestHD2AddressIsVerbatim fails unless compat.go's HD2Address is
// byte-identical to internal/compat/compat.go's in the dcrwallet module
// version go.mod pins, as found in the module cache (downloaded first if it
// is not there yet).
func TestHD2AddressIsVerbatim(t *testing.T) {
	path := filepath.Join(moduleDir(t, "decred.org/dcrwallet/v5"), "internal", "compat", "compat.go")
	upstream, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	local, err := os.ReadFile("compat.go")
	if err != nil {
		t.Fatal(err)
	}
	want := hd2AddressFunc(t, path, upstream)
	got := hd2AddressFunc(t, "compat.go", local)
	if !bytes.Equal(got, want) {
		t.Fatalf("compat.go's HD2Address differs from %s:\n%s\nwant:\n%s", path, got, want)
	}
}

// moduleDir returns the module cache directory of the version of module path
// that go.mod selects, as internal/snaclcopy's test finds it.
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
