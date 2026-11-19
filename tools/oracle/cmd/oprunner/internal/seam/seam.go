// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

// Package seam holds the injected clock and host values the copied
// dcrwallet packages under ../ read in place of the real ones. The copies
// differ from upstream only where a call was rewritten to come here (the
// oprunner test enforces that):
//
//   - time.Now() in wallet/udb/addressdb.go (address add times, the address
//     manager creation date) and wallet/udb/txdb.go (the transaction store
//     creation date) becomes Now();
//   - runtime.NumCPU() in kdf/kdf.go (the Argon2id thread count written
//     into every per-account passphrase's parameters) becomes NumCPU().
//
// The values are process-global, as the runner executes one script at a
// time on one goroutine.
package seam

import (
	"sync"
	"time"
)

var (
	mu     sync.Mutex
	now    int64
	reads  uint64
	numCPU = 1
)

// Now returns the injected time, whole seconds, in UTC.
func Now() time.Time {
	mu.Lock()
	defer mu.Unlock()
	reads++
	return time.Unix(now, 0).UTC()
}

// SetNow sets the injected time to the Unix second t.
func SetNow(t int64) {
	mu.Lock()
	defer mu.Unlock()
	now = t
}

// Unix returns the injected time as a Unix second, without counting a read.
func Unix() int64 {
	mu.Lock()
	defer mu.Unlock()
	return now
}

// Reads returns how many times Now has been called since ResetReads.
func Reads() uint64 {
	mu.Lock()
	defer mu.Unlock()
	return reads
}

// ResetReads zeroes the read counter.
func ResetReads() {
	mu.Lock()
	defer mu.Unlock()
	reads = 0
}

// NumCPU returns the injected processor count.
func NumCPU() int {
	mu.Lock()
	defer mu.Unlock()
	return numCPU
}

// SetNumCPU sets the injected processor count.
func SetNumCPU(n int) {
	mu.Lock()
	defer mu.Unlock()
	numCPU = n
}
