// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

package walletexport

// A read-only walletdb.DB over a bbolt handle opened with ReadOnly, so that
// udb.Open and udb's read API run on the very transaction the raw readers
// use, without the write lock dcrwallet's own bdb driver takes
// (wallet/internal/bdb/db.go: bolt.Open(path, 0600, nil)). udb.Open itself
// only runs a View (wallet/udb/open.go:21-54).

import (
	"errors"
	"io"

	"decred.org/dcrwallet/v5/wallet/walletdb"
	bolt "go.etcd.io/bbolt"
)

type roDB struct{ db *bolt.DB }

var errReadOnly = errors.New("walletexport: the database is open read-only")

func (d roDB) BeginReadTx() (walletdb.ReadTx, error) {
	tx, err := d.db.Begin(false)
	if err != nil {
		return nil, err
	}
	return roTx{tx}, nil
}

func (d roDB) BeginReadWriteTx() (walletdb.ReadWriteTx, error) { return nil, errReadOnly }

func (d roDB) Copy(w io.Writer) error {
	return d.db.View(func(tx *bolt.Tx) error {
		_, err := tx.WriteTo(w)
		return err
	})
}

func (d roDB) Close() error { return d.db.Close() }

type roTx struct{ tx *bolt.Tx }

func (t roTx) ReadBucket(key []byte) walletdb.ReadBucket {
	b := t.tx.Bucket(key)
	if b == nil {
		return nil
	}
	return roBucket{b}
}

func (t roTx) Rollback() error { return t.tx.Rollback() }

type roBucket struct{ b *bolt.Bucket }

func (b roBucket) NestedReadBucket(key []byte) walletdb.ReadBucket {
	n := b.b.Bucket(key)
	if n == nil {
		return nil
	}
	return roBucket{n}
}

func (b roBucket) ForEach(f func(k, v []byte) error) error { return b.b.ForEach(f) }
func (b roBucket) Get(key []byte) []byte                   { return b.b.Get(key) }
func (b roBucket) KeyN() int                               { return b.b.Stats().KeyN }
func (b roBucket) ReadCursor() walletdb.ReadCursor         { return roCursor{b.b.Cursor()} }

type roCursor struct{ c *bolt.Cursor }

func (c roCursor) First() ([]byte, []byte)        { return c.c.First() }
func (c roCursor) Last() ([]byte, []byte)         { return c.c.Last() }
func (c roCursor) Next() ([]byte, []byte)         { return c.c.Next() }
func (c roCursor) Prev() ([]byte, []byte)         { return c.c.Prev() }
func (c roCursor) Seek(k []byte) ([]byte, []byte) { return c.c.Seek(k) }
func (c roCursor) Close()                         {}
