// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

package linejson

import (
	"encoding/json"
	"runtime"
	"strings"
	"testing"
)

// TestReadBuildInfo checks the shape "build.info" answers with: the Go
// release, and a non-null module list with a path and version for every
// module and no replaced module.
func TestReadBuildInfo(t *testing.T) {
	info := ReadBuildInfo()
	if info.Go != runtime.Version() {
		t.Fatalf("go = %q, want %q", info.Go, runtime.Version())
	}
	if info.Deps == nil {
		t.Fatal("deps is nil; build.info must answer a list")
	}
	for _, d := range info.Deps {
		if d.Path == "" || d.Version == "" {
			t.Errorf("module without path or version: %+v", d)
		}
		if d.Replace != nil {
			t.Errorf("%s is replaced by %+v; the oracle must link published modules", d.Path, *d.Replace)
		}
	}
	b, err := json.Marshal(info)
	if err != nil {
		t.Fatal(err)
	}
	if !strings.HasPrefix(string(b), `{"go":`) || !strings.Contains(string(b), `"deps":[`) {
		t.Fatalf("unexpected encoding: %s", b)
	}
}
