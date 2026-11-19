// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

// This file is the interception, not a copy. It replaces prng.go and
// prng_arc4random.go of github.com/decred/dcrd/crypto/rand v1.0.1, which key
// a ChaCha20 stream from the operating system at package initialization and
// re-key it after 4 MiB or 20 s (prng.go:20-23, 67-110), with the seeded
// stream of docs/specs/23 section 5.6.2:
//
//  1. ChaCha20 under a 32-byte key set by Seed, a zero 12-byte nonce, and
//     no re-keying;
//  2. Read XORs the key stream into its argument, as upstream's does
//     (prng.go:85-110: p.cipher.XORKeyStream(s, s)), so a caller passing a
//     non-zero buffer gets buffer XOR stream;
//  3. the derived-value functions are upstream's, unchanged (default.go and
//     uniform.go in this directory are verbatim copies, which
//     cmd/txauthor's copies_test.go enforces).
//
// It is the same file as cmd/oprunner/internal/rand/prng.go: a package under
// another command's internal/ cannot be imported from here.
//
// The exported API is upstream's plus Seed and Drawn. Until Seed is called
// the stream runs under the all-zero key.

package rand

import (
	"sync"

	"golang.org/x/crypto/chacha20"
)

// PRNG is a deterministic ChaCha20 stream with upstream's method set.
// PRNG methods are not safe for concurrent access.
type PRNG struct {
	cipher *chacha20.Cipher
	drawn  uint64
}

func newPRNG(key *[chacha20.KeySize]byte) *PRNG {
	var nonce [chacha20.NonceSize]byte
	// Never errors with correct key and nonce sizes.
	c, err := chacha20.NewUnauthenticatedCipher(key[:], nonce[:])
	if err != nil {
		panic(err)
	}
	return &PRNG{cipher: c}
}

// NewPRNG returns a PRNG keyed by the next 32 bytes of the global stream.
func NewPRNG() (*PRNG, error) {
	var key [chacha20.KeySize]byte
	Read(key[:])
	return newPRNG(&key), nil
}

// Read XORs len(s) bytes of the key stream into s. Read never errors.
func (p *PRNG) Read(s []byte) (n int, err error) {
	p.cipher.XORKeyStream(s, s)
	p.drawn += uint64(len(s))
	return len(s), nil
}

type lockingPRNG struct {
	*PRNG
	sync.Mutex
}

func init() {
	globalRand = &lockingPRNG{PRNG: newPRNG(new([chacha20.KeySize]byte))}
}

func (p *lockingPRNG) Read(s []byte) (n int, err error) {
	p.Lock()
	defer p.Unlock()

	return p.PRNG.Read(s)
}

// Seed restarts the global stream under key.
func Seed(key [32]byte) {
	globalRand.Lock()
	defer globalRand.Unlock()

	globalRand.PRNG = newPRNG(&key)
}

// Drawn returns the number of bytes read from the global stream since the
// last Seed.
func Drawn() uint64 {
	globalRand.Lock()
	defer globalRand.Unlock()

	return globalRand.PRNG.drawn
}
