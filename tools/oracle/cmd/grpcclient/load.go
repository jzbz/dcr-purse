// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

package main

import (
	"bytes"
	"context"
	"crypto/tls"
	"encoding/binary"
	"encoding/json"
	"errors"
	"net"
	"strings"
	"sync"
	"sync/atomic"
	"syscall"
	"time"

	pb "decred.org/dcrwallet/v5/rpc/walletrpc"
	"golang.org/x/net/http2"
	"golang.org/x/net/http2/hpack"
	"google.golang.org/grpc"
	"google.golang.org/grpc/metadata"
	"google.golang.org/grpc/peer"
)

// versionBody is a VersionRequest of exactly size bytes: one unknown
// length-delimited field (number 15) of zeros, which a proto3 decoder
// skips, so the server answers it as an ordinary Version call.
func versionBody(size int) []byte {
	if size <= 0 {
		return nil
	}
	for n := size - 2; n >= 0; n-- {
		var v [binary.MaxVarintLen64]byte
		k := binary.PutUvarint(v[:], uint64(n))
		if 1+k+n == size {
			b := make([]byte, 0, size)
			b = append(b, 0x7a)
			b = append(b, v[:k]...)
			return append(b, make([]byte, n)...)
		}
	}
	return nil
}

type bigcallsArgs struct {
	dialArgs
	Total       int `json:"total"`
	Concurrency int `json:"concurrency"`
	Size        int `json:"size"`
}

// cmdBigcalls makes total Version calls of size bytes each, concurrency at
// a time, on one grpc-go connection, and counts how they ended.
func cmdBigcalls(raw json.RawMessage) (any, error) {
	var a bigcallsArgs
	if err := json.Unmarshal(raw, &a); err != nil {
		return nil, err
	}
	conn, err := dial(&a.dialArgs)
	if err != nil {
		return nil, err
	}
	defer conn.Close()
	body := versionBody(a.Size)
	codes := map[string]int{}
	var mu sync.Mutex
	var firstErr error
	next := make(chan struct{}, a.Total)
	for i := 0; i < a.Total; i++ {
		next <- struct{}{}
	}
	close(next)
	var wg sync.WaitGroup
	start := time.Now()
	for w := 0; w < a.Concurrency; w++ {
		wg.Add(1)
		go func() {
			defer wg.Done()
			for range next {
				ctx, cancel := context.WithTimeout(context.Background(), timeout(a.TimeoutMS))
				var reply []byte
				err := conn.Invoke(ctx, "/walletrpc.VersionService/Version", &body, &reply,
					grpc.ForceCodec(rawCodec{}))
				cancel()
				mu.Lock()
				if err != nil {
					codes[statusOf(err).CodeName]++
					if firstErr == nil {
						firstErr = err
					}
				} else {
					codes["OK"]++
				}
				mu.Unlock()
			}
		}()
	}
	wg.Wait()
	out := map[string]any{
		"codes":      codes,
		"elapsed_ms": float64(time.Since(start).Microseconds()) / 1000,
	}
	if firstErr != nil {
		out["first_error"] = statusOf(firstErr)
	}
	return out, nil
}

type cancelsArgs struct {
	dialArgs
	Calls     int    `json:"calls"`
	Method    string `json:"method"`
	CallMS    int    `json:"call_ms"`
	AfterWait int    `json:"after_wait_ms"`
}

// cmdCancels makes calls concurrent calls to method on one connection, each
// with a deadline of call_ms, so that grpc-go cancels each one with
// RST_STREAM CANCEL while the server is still working on it. It then makes
// one Version call and reports whether it went over the same connection
// (local address unchanged) or grpc-go had to reconnect, which is what a
// server-side GOAWAY would force.
func cmdCancels(raw json.RawMessage) (any, error) {
	var a cancelsArgs
	if err := json.Unmarshal(raw, &a); err != nil {
		return nil, err
	}
	conn, err := dial(&a.dialArgs)
	if err != nil {
		return nil, err
	}
	defer conn.Close()
	client := pb.NewVersionServiceClient(conn)
	localOf := func() (string, error) {
		var p peer.Peer
		ctx, cancel := context.WithTimeout(context.Background(), timeout(a.TimeoutMS))
		defer cancel()
		if _, err := client.Version(ctx, &pb.VersionRequest{}, grpc.Peer(&p)); err != nil {
			return "", err
		}
		if p.LocalAddr == nil {
			return "", errors.New("no local address")
		}
		return p.LocalAddr.String(), nil
	}
	before, err := localOf()
	if err != nil {
		return map[string]any{"before_error": statusOf(err)}, nil
	}
	codes := map[string]int{}
	var mu sync.Mutex
	var wg sync.WaitGroup
	body := []byte{}
	for i := 0; i < a.Calls; i++ {
		wg.Add(1)
		go func() {
			defer wg.Done()
			ctx, cancel := context.WithTimeout(context.Background(), time.Duration(a.CallMS)*time.Millisecond)
			var reply []byte
			err := conn.Invoke(ctx, a.Method, &body, &reply, grpc.ForceCodec(rawCodec{}))
			cancel()
			mu.Lock()
			if err != nil {
				codes[statusOf(err).CodeName]++
			} else {
				codes["OK"]++
			}
			mu.Unlock()
		}()
	}
	wg.Wait()
	time.Sleep(time.Duration(a.AfterWait) * time.Millisecond)
	after, err := localOf()
	out := map[string]any{"codes": codes, "same_connection": err == nil && after == before}
	if err != nil {
		out["after_error"] = statusOf(err)
	}
	return out, nil
}

type metacallArgs struct {
	dialArgs
	MetaKey string `json:"meta_key"`
	Size    int    `json:"size"`
}

// cmdMetacall makes one Version call carrying one metadata value of size
// bytes, which grpc-go sends as a header block split into 16 KiB frames.
func cmdMetacall(raw json.RawMessage) (any, error) {
	var a metacallArgs
	if err := json.Unmarshal(raw, &a); err != nil {
		return nil, err
	}
	conn, err := dial(&a.dialArgs)
	if err != nil {
		return nil, err
	}
	defer conn.Close()
	ctx, cancel := context.WithTimeout(context.Background(), timeout(a.TimeoutMS))
	defer cancel()
	ctx = metadata.AppendToOutgoingContext(ctx, a.MetaKey, strings.Repeat("a", a.Size))
	_, err = pb.NewVersionServiceClient(conn).Version(ctx, &pb.VersionRequest{})
	if err != nil {
		return map[string]any{"status": statusOf(err)}, nil
	}
	return map[string]any{"status": &statusResult{Code: 0, CodeName: "OK"}}, nil
}

type slowreadArgs struct {
	dialArgs
	Calls  int `json:"calls"`
	RcvBuf int `json:"rcvbuf"`
}

type slowreader struct {
	conn    net.Conn
	written atomic.Int64
	done    chan struct{}
	err     atomic.Value
}

var (
	slowMu      sync.Mutex
	slowReaders = map[int]*slowreader{}
	slowNext    int
)

// cmdSlowread starts, in the background, an authenticated HTTP/2
// connection with a small receive buffer that opens every window as wide
// as it goes, writes calls complete Version requests as fast as the server
// takes them, and never reads. It returns once the server has stopped
// taking requests for 500 ms or all were written, with an id for
// slowread.stop.
func cmdSlowread(raw json.RawMessage) (any, error) {
	var a slowreadArgs
	if err := json.Unmarshal(raw, &a); err != nil {
		return nil, err
	}
	cfg, err := clientTLS(&a.dialArgs, []string{"h2"})
	if err != nil {
		return nil, err
	}
	d := &net.Dialer{Timeout: timeout(a.TimeoutMS)}
	if a.RcvBuf > 0 {
		d.Control = func(network, address string, c syscall.RawConn) error {
			var serr error
			err := c.Control(func(fd uintptr) {
				serr = syscall.SetsockoptInt(int(fd), syscall.SOL_SOCKET, syscall.SO_RCVBUF, a.RcvBuf)
			})
			if err != nil {
				return err
			}
			return serr
		}
	}
	rawConn, err := d.Dial("tcp", a.Addr)
	if err != nil {
		return map[string]any{"dial_error": err.Error()}, nil
	}
	if host, _, err := net.SplitHostPort(a.Addr); err == nil {
		cfg.ServerName = host
	}
	tc := tls.Client(rawConn, cfg)
	if err := tc.Handshake(); err != nil {
		rawConn.Close()
		return map[string]any{"dial_error": err.Error()}, nil
	}
	sr := &slowreader{conn: tc, done: make(chan struct{})}
	go func() {
		defer close(sr.done)
		var buf bytes.Buffer
		fr := http2.NewFramer(&buf, nil)
		buf.WriteString(http2.ClientPreface)
		_ = fr.WriteSettings(http2.Setting{ID: http2.SettingInitialWindowSize, Val: 1<<31 - 1})
		_ = fr.WriteWindowUpdate(0, 1<<31-1-65535)
		var hbuf bytes.Buffer
		enc := hpack.NewEncoder(&hbuf)
		flush := func() bool {
			if _, err := tc.Write(buf.Bytes()); err != nil {
				sr.err.Store(err.Error())
				return false
			}
			buf.Reset()
			return true
		}
		for i := 0; i < a.Calls; i++ {
			id := uint32(2*i + 1)
			hbuf.Reset()
			for _, f := range [][2]string{
				{":method", "POST"}, {":scheme", "https"}, {":path", "/walletrpc.VersionService/Version"},
				{":authority", a.Addr}, {"content-type", "application/grpc"}, {"te", "trailers"},
			} {
				_ = enc.WriteField(hpack.HeaderField{Name: f[0], Value: f[1]})
			}
			_ = fr.WriteHeaders(http2.HeadersFrameParam{StreamID: id, BlockFragment: hbuf.Bytes(), EndHeaders: true})
			_ = fr.WriteData(id, true, []byte{0, 0, 0, 0, 0})
			if buf.Len() > 16<<10 {
				if !flush() {
					return
				}
			}
			sr.written.Store(int64(i + 1))
		}
		flush()
	}()
	// Wait until the writes stop advancing or finish.
	last := int64(-1)
	for {
		select {
		case <-sr.done:
			goto started
		case <-time.After(500 * time.Millisecond):
		}
		n := sr.written.Load()
		if n == last {
			goto started
		}
		last = n
	}
started:
	slowMu.Lock()
	slowNext++
	id := slowNext
	slowReaders[id] = sr
	slowMu.Unlock()
	return map[string]any{"id": id, "written": sr.written.Load()}, nil
}

type slowstopArgs struct {
	ID int `json:"id"`
}

// cmdSlowreadStop reports how far a slow reader got and whether the server
// has closed its connection (a read returns at once), then closes it.
func cmdSlowreadStop(raw json.RawMessage) (any, error) {
	var a slowstopArgs
	if err := json.Unmarshal(raw, &a); err != nil {
		return nil, err
	}
	slowMu.Lock()
	sr := slowReaders[a.ID]
	delete(slowReaders, a.ID)
	slowMu.Unlock()
	if sr == nil {
		return nil, errors.New("no such slow reader")
	}
	out := map[string]any{"written": sr.written.Load()}
	if e, ok := sr.err.Load().(string); ok {
		out["write_error"] = e
	}
	// Drain what the server sent; a closed connection ends in EOF or a
	// reset rather than a timeout.
	_ = sr.conn.SetReadDeadline(time.Now().Add(2 * time.Second))
	buf := make([]byte, 1<<16)
	end := "open"
	for {
		if _, err := sr.conn.Read(buf); err != nil {
			var ne net.Error
			if errors.As(err, &ne) && ne.Timeout() {
				end = "open"
			} else {
				end = "closed: " + err.Error()
			}
			break
		}
	}
	out["end"] = end
	sr.conn.Close()
	return out, nil
}
