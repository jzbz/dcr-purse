// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

// Command grpcclient is the grpc-go oracle for the gRPC transport spikes
// (decision D5). It is an unmodified grpc-go v1.83.2 client, the same
// library dcrwallet links, plus a reference server built the way dcrwallet
// builds its own (rpcserver.go), so that a Rust server's observable
// behaviour can be compared with grpc-go's. It speaks the linejson protocol
// (one JSON request per line on stdin, one response per line on stdout).
//
// Commands:
//
//	pki          {dir, server_key?, client_key?}
//	             Writes a server key pair and the client-side material into
//	             dir (see pki.go for the file list) and returns the paths.
//	             Key types: "Ed25519" (dcrwallet's default --tlscurve),
//	             "P-256", "P-384", "P-521", "RSA-2048".
//	server.start {cert, key, clientca}
//	             Starts the reference server on 127.0.0.1:0 and returns
//	             {addr}. TLS and server options are dcrwallet's.
//	server.stop  {}
//	version      {addr, rootca, cert?, key?, n?, total?, concurrency?,
//	             timeout_ms?, insecure?, tls_max?}
//	             Dials addr with grpc-go and calls VersionService.Version n
//	             times sequentially, then total times with the given
//	             concurrency on the same connection. Returns the first
//	             reply, the negotiated TLS parameters, the response headers
//	             and trailers of the first call, latency percentiles and
//	             throughput, or the gRPC status of the first failure.
//	call         {addr, rootca, cert, key, method, body_hex?, metadata?,
//	             timeout_ms?}
//	             One unary call to an arbitrary method with a raw request
//	             body (default empty). Returns the status code and message,
//	             the headers and trailers, and the reply bytes.
//	http1        {addr, rootca, cert, key}
//	             One HTTP/1.1 request over TLS (ALPN http/1.1) with Go's
//	             net/http client.
//	raw          {addr, rootca, cert?, key?, alpn?, preface?, steps,
//	             read_ms?, max_frames?}
//	             A TLS connection that writes the client preface (unless
//	             preface is false) and then each step's bytes (hex, repeated
//	             repeat times), and reports every frame the server sends
//	             until it closes, read_ms passes or max_frames are read.
//	burst        {addr, rootca, cert, key, streams, path?, timeout_ms?}
//	             Opens streams concurrent unary streams on one HTTP/2
//	             connection, written back to back with x/net's framer, and
//	             counts how each ended.
//	bigcalls     {addr, rootca, cert, key, total, concurrency, size,
//	             timeout_ms?}
//	             total Version calls whose request is size bytes (an
//	             unknown field the server skips), concurrency at a time on
//	             one grpc-go connection; returns the status codes counted.
//	cancels      {addr, rootca, cert, key, calls, method, call_ms,
//	             after_wait_ms?}
//	             calls concurrent calls to method with a deadline of
//	             call_ms, which grpc-go cancels (RST_STREAM CANCEL), then a
//	             Version call; reports the codes and whether that call used
//	             the same connection (no GOAWAY forced a reconnect).
//	metacall     {addr, rootca, cert, key, meta_key, size}
//	             One Version call carrying a metadata value of size bytes.
//	slowread     {addr, rootca, cert, key, calls, rcvbuf}
//	             In the background, an authenticated connection with a
//	             small receive buffer that writes calls Version requests and
//	             never reads; returns {id, written} once the server stops
//	             taking requests.
//	slowread.stop {id}
//	             How far that connection got and whether the server closed
//	             it; closes it.
package main

import (
	"github.com/jzbz/dcr-purse/tools/oracle/internal/linejson"
)

func main() {
	linejson.Serve(map[string]linejson.Handler{
		"pki":           cmdPKI,
		"server.start":  cmdServerStart,
		"server.stop":   cmdServerStop,
		"version":       cmdVersion,
		"call":          cmdCall,
		"http1":         cmdHTTP1,
		"raw":           cmdRaw,
		"burst":         cmdBurst,
		"bigcalls":      cmdBigcalls,
		"cancels":       cmdCancels,
		"metacall":      cmdMetacall,
		"slowread":      cmdSlowread,
		"slowread.stop": cmdSlowreadStop,
	})
}
