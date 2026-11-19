// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

package deps

import (
	"bufio"
	"bytes"
	"crypto/sha256"
	"encoding/base64"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// dcrwalletModule is the module the oracle pins at the parity commit.
const dcrwalletModule = "decred.org/dcrwallet/v5"

// oracleRoot is tools/oracle, relative to this package's directory, which
// is where `go test` runs the test.
var oracleRoot = filepath.Join("..", "..")

// readLF reads a file and normalizes CRLF line endings to LF, so that a
// checkout with core.autocrlf on Windows compares as the LF original does.
func readLF(t *testing.T, name string) []byte {
	t.Helper()
	b, err := os.ReadFile(filepath.Join(oracleRoot, name))
	if err != nil {
		t.Fatal(err)
	}
	return bytes.ReplaceAll(b, []byte("\r\n"), []byte("\n"))
}

// goModHash is the h1: hash go.sum records for a module's go.mod file:
// golang.org/x/mod/sumdb/dirhash.Hash1 over the single file "go.mod", that
// is base64(SHA-256("<hex SHA-256 of the file>  go.mod\n")).
func goModHash(contents []byte) string {
	fileSum := sha256.Sum256(contents)
	summary := fmt.Sprintf("%x  %s\n", fileSum, "go.mod")
	h := sha256.Sum256([]byte(summary))
	return "h1:" + base64.StdEncoding.EncodeToString(h[:])
}

// requiredVersion returns the version go.mod requires of module, from a
// require line inside or outside a require block.
func requiredVersion(goMod []byte, module string) (string, error) {
	sc := bufio.NewScanner(bytes.NewReader(goMod))
	inBlock := false
	for sc.Scan() {
		line := strings.TrimSpace(sc.Text())
		if i := strings.Index(line, "//"); i >= 0 {
			line = strings.TrimSpace(line[:i])
		}
		switch {
		case line == "require (":
			inBlock = true
			continue
		case inBlock && line == ")":
			inBlock = false
			continue
		case strings.HasPrefix(line, "require "):
			line = strings.TrimSpace(strings.TrimPrefix(line, "require "))
		case !inBlock:
			continue
		}
		f := strings.Fields(line)
		if len(f) == 2 && f[0] == module {
			return f[1], nil
		}
	}
	if err := sc.Err(); err != nil {
		return "", err
	}
	return "", fmt.Errorf("go.mod does not require %s", module)
}

// TestDcrwalletGoModIsVerbatim fails unless dcrwallet.go.mod is the go.mod of
// the dcrwallet module version the oracle's go.mod requires: its h1: hash
// must equal the "<module> <version>/go.mod h1:" line of go.sum (line 4 at
// the parity pin 54b85b33), which the go command verified against the
// checksum database when the module was added. The Rust checks of ADR-0009
// read dcrwallet's requirements from this copy, so it may not drift.
func TestDcrwalletGoModIsVerbatim(t *testing.T) {
	version, err := requiredVersion(readLF(t, "go.mod"), dcrwalletModule)
	if err != nil {
		t.Fatal(err)
	}
	prefix := dcrwalletModule + " " + version + "/go.mod "
	var want string
	lineNo := 0
	for i, line := range strings.Split(string(readLF(t, "go.sum")), "\n") {
		if strings.HasPrefix(line, prefix) {
			if want != "" {
				t.Fatalf("go.sum has more than one %q line", strings.TrimSpace(prefix))
			}
			want = strings.TrimPrefix(line, prefix)
			lineNo = i + 1
		}
	}
	if want == "" {
		t.Fatalf("go.sum has no %q line", strings.TrimSpace(prefix))
	}
	got := goModHash(readLF(t, "dcrwallet.go.mod"))
	if got != want {
		t.Fatalf("dcrwallet.go.mod hashes to %s; go.sum:%d records %s for %s %s: "+
			"the copy is not dcrwallet's go.mod at the pin", got, lineNo, want, dcrwalletModule, version)
	}
	t.Logf("dcrwallet.go.mod matches go.sum:%d (%s %s/go.mod %s)", lineNo, dcrwalletModule, version, got)
}
