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
	"strings"
	"testing"
)

// TestSealCopyIsVerbatim fails unless everything below the marker line of
// sealcopy.go appears, byte for byte, in wallet/udb/addressmanager.go of the
// dcrwallet module version go.mod pins, and covers the functions the oracle
// relies on.
func TestSealCopyIsVerbatim(t *testing.T) {
	dir := moduleDir(t, "decred.org/dcrwallet/v5")
	path := filepath.Join(dir, "wallet", "udb", "addressmanager.go")
	upstream, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	local, err := os.ReadFile("sealcopy.go")
	if err != nil {
		t.Fatal(err)
	}
	local = bytes.ReplaceAll(local, []byte("\r\n"), []byte("\n"))

	const marker = "// --- verbatim from wallet/udb/addressmanager.go ---\n"
	i := bytes.Index(local, []byte(marker))
	if i < 0 || bytes.Count(local, []byte(marker)) != 1 {
		t.Fatalf("sealcopy.go does not have exactly one marker line %q", strings.TrimSpace(marker))
	}
	block := local[i+len(marker):]
	if !bytes.Contains(upstream, block) {
		t.Fatalf("the block below the marker in sealcopy.go is not a verbatim range of %s", path)
	}
	for _, decl := range []string{
		"func argon2idKey(password []byte, k *kdf.Argon2idParams) keyType {",
		"xchacha20NonceSize        = 24",
		"type keyType []byte",
		"func seal(key keyType, plaintext []byte) ([]byte, error) {",
		"func unseal(key keyType, ciphertext []byte) ([]byte, error) {",
	} {
		if !bytes.Contains(block, []byte(decl)) {
			t.Fatalf("the verbatim block lacks %q", decl)
		}
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
