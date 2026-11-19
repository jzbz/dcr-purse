// Copyright (c) 2026 The dcr-purse developers
// Copyright (c) 2014-2016 The btcsuite developers
// Copyright (c) 2015-2024 The Decred developers
// Use of this source code is governed by an ISC
// license that can be found in the LICENSE file.

package main

// The per-account passphrase scheme of dcrwallet's wallet/udb (Argon2id key,
// XChaCha20-Poly1305 seal), whose functions are unexported there. Everything
// below the marker line is a verbatim copy of a contiguous range of
// wallet/udb/addressmanager.go at the version go.mod pins;
// sealcopy_test.go fails unless it still is.

import (
	"decred.org/dcrwallet/v5/errors"
	"decred.org/dcrwallet/v5/kdf"
	"github.com/decred/dcrd/crypto/rand"
	"golang.org/x/crypto/chacha20poly1305"
)

// --- verbatim from wallet/udb/addressmanager.go ---
func argon2idKey(password []byte, k *kdf.Argon2idParams) keyType {
	return keyType(kdf.DeriveKey(password, k, 32))
}

const (
	xchacha20NonceSize        = 24
	poly1305TagSize           = 16
	xchacha20poly1305Overhead = xchacha20NonceSize + poly1305TagSize
)

// improves type safety for seal and unseal funcs with a new type for
// argon2id-derived keys.
type keyType []byte

func seal(key keyType, plaintext []byte) ([]byte, error) {
	sealedLen := len(plaintext) + xchacha20poly1305Overhead
	nonce := make([]byte, xchacha20NonceSize, sealedLen)
	rand.Read(nonce)

	aead, err := chacha20poly1305.NewX(key)
	if err != nil {
		// wrong key len; this is always a programming mistake
		// (bad type conversion to keyType).
		panic(err)
	}
	sealed := aead.Seal(nonce, nonce, plaintext, nil)
	return sealed, nil
}

func unseal(key keyType, ciphertext []byte) ([]byte, error) {
	aead, err := chacha20poly1305.NewX(key)
	if err != nil {
		// wrong key len; this is always a programming mistake
		// (bad type conversion to keyType).
		panic(err)
	}
	if len(ciphertext) < xchacha20poly1305Overhead {
		e := errors.Errorf("ciphertext too short (len %d) "+
			"to encode nonce and MAC tag", len(ciphertext))
		return nil, errors.E(errors.Crypto, e)
	}
	nonce := ciphertext[:xchacha20NonceSize]
	ciphertext = ciphertext[xchacha20NonceSize:]
	plaintext, err := aead.Open(nil, nonce, ciphertext, nil)
	if err != nil {
		// technically the ciphertext may have been tampered with, but
		// to improve UX we report authentication failures as incorrect
		// passphrases.
		return nil, errors.E(errors.Passphrase)
	}
	return plaintext, nil
}
