// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

package main

import (
	"bytes"
	"encoding/binary"
	"testing"

	"decred.org/dcrwallet/v5/errors"
	upstreamudb "decred.org/dcrwallet/v5/wallet/udb"
	"github.com/jzbz/dcr-purse/tools/oracle/cmd/oprunner/internal/udb"
)

// A panic in the runner's own code must end the run as a runner fault,
// never pass for a dcrwallet panic (kind "Panic").
func TestRunnerPanicIsARunnerFault(t *testing.T) {
	err := recovered(func() error {
		var details *udb.TicketDetails
		_ = details.Ticket // the nil dereference the ticket_details handler had
		return nil
	})
	if _, ok := err.(*runnerFault); !ok {
		t.Fatalf("a nil dereference in the runner gave %T %v, want *runnerFault", err, err)
	}
	if !fatal(err) {
		t.Fatal("a runner fault must end the run")
	}
	if v := errorValue(err); v["kind"] == "Panic" {
		t.Fatal("a runner fault encoded with kind Panic")
	}
}

// A panic inside the copied udb, even one that surfaces in a dependency
// udb calls, is dcrwallet behaviour: an operation result of kind "Panic".
func TestUpstreamPanicIsAnOperationResult(t *testing.T) {
	err := recovered(func() error {
		udb.ExtractBlockHeaderHeight(nil) // slices a nil header
		return nil
	})
	p, ok := err.(*panicError)
	if !ok {
		t.Fatalf("a panic inside udb gave %T %v, want *panicError", err, err)
	}
	if fatal(err) || errorValue(err)["kind"] != "Panic" || p.text == "" {
		t.Fatalf("a panic inside udb must be an operation result of kind Panic: %v", errorValue(err))
	}
}

// A panic inside dcrwallet's unmodified packages (the wallet session's
// wallet package and the udb under it) is dcrwallet behaviour too.
func TestUnmodifiedDcrwalletPanicIsAnOperationResult(t *testing.T) {
	err := recovered(func() error {
		var s *upstreamudb.Store
		_, _ = s.TxDetails(nil, nil) // dereferences its nil bucket inside udb
		return nil
	})
	if _, ok := err.(*panicError); !ok {
		t.Fatalf("a panic inside dcrwallet's udb gave %T %v, want *panicError", err, err)
	}
}

// A runner callback that udb runs is runner code: its panic is the
// runner's, though udb frames sit further out.
func TestPanicInRunnerCallbackIsARunnerFault(t *testing.T) {
	err := recovered(func() error {
		return udbCallsBack(func() { panic("callback") })
	})
	if _, ok := err.(*runnerFault); !ok {
		t.Fatalf("a panic in a runner callback gave %T %v, want *runnerFault", err, err)
	}
}

// udbCallsBack runs f the way udb runs a caller's callback, with no copied
// udb frame between the panic and the runner's frame.
func udbCallsBack(f func()) error { f(); return nil }

// Error texts are rendered with the daemon's separator (dcrwallet.go:38).
func TestDaemonSeparator(t *testing.T) {
	if errors.Separator != ":: " {
		t.Fatalf("errors.Separator is %q, want the daemon's \":: \"", errors.Separator)
	}
	err := errors.E(errors.Op("outer"), errors.E(errors.Op("inner"), errors.Invalid, "detail"))
	v := errorValue(err)
	if v["text"] != "outer: invalid operation:: inner: detail" || v["kind"] != "Invalid" {
		t.Fatalf("nested error encoded as %v", v)
	}
}

func TestTolerated(t *testing.T) {
	exist := errors.E(errors.Op("x"), errors.Exist, "already")
	wrapped := resolveErr("op arg", "Entry", exist)
	if !tolerated(exist, []errors.Kind{errors.NotExist, errors.Exist}) {
		t.Fatal("Exist not tolerated")
	}
	if !tolerated(wrapped, []errors.Kind{errors.Exist}) {
		t.Fatal("a resolution error lost its kind")
	}
	if tolerated(exist, []errors.Kind{errors.NotExist}) {
		t.Fatal("Exist tolerated as NotExist")
	}
	if tolerated(&panicError{"x"}, []errors.Kind{errors.Exist, errors.Invalid}) {
		t.Fatal("a panic tolerated")
	}
	canon := &canonError{kind: "NotExist", text: "t", err: errors.E(errors.NotExist)}
	if !tolerated(canon, []errors.Kind{errors.NotExist}) {
		t.Fatal("a canonicalized error lost its kind")
	}
}

// The wallet session's interposer rewrites exactly the add time of an
// address row (wallet/udb/addressdb.go:1136-1152) and nothing else.
func TestStampAddTime(t *testing.T) {
	raw := []byte{0, 0, 0, 0, 7, 0, 0, 0} // a chained address's branch and index
	row := make([]byte, 18+len(raw))
	row[0] = 0                                         // adtChain
	binary.LittleEndian.PutUint32(row[1:5], 3)         // account
	binary.LittleEndian.PutUint64(row[5:13], 12345678) // the real clock
	binary.LittleEndian.PutUint32(row[14:18], uint32(len(raw)))
	copy(row[18:], raw)
	key := bytes.Repeat([]byte{0xaa}, 32)

	got := stampAddTime(key, row, 1794990000)
	if binary.LittleEndian.Uint64(got[5:13]) != 1794990000 {
		t.Fatal("add time not rewritten")
	}
	if !bytes.Equal(got[:5], row[:5]) || !bytes.Equal(got[13:], row[13:]) {
		t.Fatal("bytes other than the add time changed")
	}
	if binary.LittleEndian.Uint64(row[5:13]) != 12345678 {
		t.Fatal("the caller's buffer was modified")
	}
	// Anything not shaped like an address row passes unchanged.
	for _, c := range []struct{ k, v []byte }{
		{key[:20], row},
		{key, row[:17]},
		{key, append(append([]byte{}, row...), 0)},
		{key, append([]byte{9}, row[1:]...)},
	} {
		if !bytes.Equal(stampAddTime(c.k, c.v, 1), c.v) {
			t.Fatalf("a non-row value was rewritten: %x", c.v)
		}
	}
}
