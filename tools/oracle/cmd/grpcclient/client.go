// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

package main

import (
	"context"
	"crypto/tls"
	"crypto/x509"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net/http"
	"os"
	"sort"
	"sync"
	"time"

	pb "decred.org/dcrwallet/v5/rpc/walletrpc"
	"google.golang.org/grpc"
	"google.golang.org/grpc/credentials"
	"google.golang.org/grpc/credentials/insecure"
	"google.golang.org/grpc/metadata"
	"google.golang.org/grpc/peer"
	"google.golang.org/grpc/status"
)

type dialArgs struct {
	Addr      string `json:"addr"`
	RootCA    string `json:"rootca"`
	Cert      string `json:"cert"`
	Key       string `json:"key"`
	Insecure  bool   `json:"insecure"`
	TLSMax    string `json:"tls_max"`
	TimeoutMS int    `json:"timeout_ms"`
}

// clientTLS is the client side of the handshake as dcrwallet's
// clientusage.md sets it up: the server's rpc.cert as the only root, and a
// client key pair when one is given.
func clientTLS(a *dialArgs, alpn []string) (*tls.Config, error) {
	pool := x509.NewCertPool()
	pemBytes, err := os.ReadFile(a.RootCA)
	if err != nil {
		return nil, err
	}
	if !pool.AppendCertsFromPEM(pemBytes) {
		return nil, errors.New("no root certificate")
	}
	cfg := &tls.Config{RootCAs: pool, NextProtos: alpn}
	if a.Cert != "" {
		kp, err := tls.LoadX509KeyPair(a.Cert, a.Key)
		if err != nil {
			return nil, err
		}
		cfg.Certificates = []tls.Certificate{kp}
	}
	switch a.TLSMax {
	case "", "1.3":
	case "1.2":
		cfg.MaxVersion = tls.VersionTLS12
	default:
		return nil, fmt.Errorf("unknown tls_max %q", a.TLSMax)
	}
	return cfg, nil
}

func dial(a *dialArgs) (*grpc.ClientConn, error) {
	var creds credentials.TransportCredentials
	if a.Insecure {
		creds = insecure.NewCredentials()
	} else {
		cfg, err := clientTLS(a, nil)
		if err != nil {
			return nil, err
		}
		creds = credentials.NewTLS(cfg)
	}
	return grpc.NewClient(a.Addr, grpc.WithTransportCredentials(creds))
}

func timeout(ms int) time.Duration {
	if ms <= 0 {
		return 10 * time.Second
	}
	return time.Duration(ms) * time.Millisecond
}

type statusResult struct {
	Code     uint32 `json:"code"`
	CodeName string `json:"code_name"`
	Message  string `json:"message"`
}

func statusOf(err error) *statusResult {
	st := status.Convert(err)
	return &statusResult{Code: uint32(st.Code()), CodeName: st.Code().String(), Message: st.Message()}
}

func mdMap(md metadata.MD) map[string][]string {
	out := map[string][]string{}
	for k, v := range md {
		out[k] = v
	}
	return out
}

type tlsResult struct {
	Version            string `json:"version"`
	CipherSuite        string `json:"cipher_suite"`
	Curve              string `json:"curve"`
	NegotiatedProtocol string `json:"negotiated_protocol"`
	DidResume          bool   `json:"did_resume"`
}

func tlsOf(p *peer.Peer) *tlsResult {
	if p == nil || p.AuthInfo == nil {
		return nil
	}
	info, ok := p.AuthInfo.(credentials.TLSInfo)
	if !ok {
		return nil
	}
	st := info.State
	return &tlsResult{
		Version:            tls.VersionName(st.Version),
		CipherSuite:        tls.CipherSuiteName(st.CipherSuite),
		Curve:              st.CurveID.String(),
		NegotiatedProtocol: st.NegotiatedProtocol,
		DidResume:          st.DidResume,
	}
}

type versionArgs struct {
	dialArgs
	N           int `json:"n"`
	Total       int `json:"total"`
	Concurrency int `json:"concurrency"`
}

type latency struct {
	Calls      int     `json:"calls"`
	Errors     int     `json:"errors"`
	P50us      int64   `json:"p50_us"`
	P90us      int64   `json:"p90_us"`
	P99us      int64   `json:"p99_us"`
	MaxUs      int64   `json:"max_us"`
	ElapsedMS  float64 `json:"elapsed_ms"`
	CallsPerS  float64 `json:"calls_per_s"`
	Concurrent int     `json:"concurrency"`
}

func summarize(d []time.Duration, errs int, elapsed time.Duration, conc int) *latency {
	sort.Slice(d, func(i, j int) bool { return d[i] < d[j] })
	pct := func(p float64) int64 {
		if len(d) == 0 {
			return 0
		}
		i := int(p * float64(len(d)-1))
		return d[i].Microseconds()
	}
	l := &latency{Calls: len(d), Errors: errs, ElapsedMS: float64(elapsed.Microseconds()) / 1000, Concurrent: conc}
	l.P50us, l.P90us, l.P99us = pct(0.50), pct(0.90), pct(0.99)
	if len(d) > 0 {
		l.MaxUs = d[len(d)-1].Microseconds()
	}
	if elapsed > 0 {
		l.CallsPerS = float64(len(d)) / elapsed.Seconds()
	}
	return l
}

func cmdVersion(raw json.RawMessage) (any, error) {
	var a versionArgs
	if err := json.Unmarshal(raw, &a); err != nil {
		return nil, err
	}
	if a.N <= 0 {
		a.N = 1
	}
	conn, err := dial(&a.dialArgs)
	if err != nil {
		return nil, err
	}
	defer conn.Close()
	client := pb.NewVersionServiceClient(conn)
	to := timeout(a.TimeoutMS)

	out := map[string]any{}
	var hdr, trl metadata.MD
	var p peer.Peer
	ctx, cancel := context.WithTimeout(context.Background(), to)
	reply, err := client.Version(ctx, &pb.VersionRequest{}, grpc.Header(&hdr), grpc.Trailer(&trl), grpc.Peer(&p))
	cancel()
	if err != nil {
		out["status"] = statusOf(err)
		out["tls"] = tlsOf(&p)
		return out, nil
	}
	b, _ := json.Marshal(reply)
	out["reply"] = json.RawMessage(b)
	out["headers"] = mdMap(hdr)
	out["trailers"] = mdMap(trl)
	out["tls"] = tlsOf(&p)

	// Sequential calls.
	var seq []time.Duration
	seqErrs := 0
	start := time.Now()
	for i := 0; i < a.N; i++ {
		t0 := time.Now()
		ctx, cancel := context.WithTimeout(context.Background(), to)
		_, err := client.Version(ctx, &pb.VersionRequest{})
		cancel()
		if err != nil {
			seqErrs++
			if _, ok := out["status"]; !ok {
				out["status"] = statusOf(err)
			}
			continue
		}
		seq = append(seq, time.Since(t0))
	}
	out["sequential"] = summarize(seq, seqErrs, time.Since(start), 1)

	// Concurrent calls on the same connection.
	if a.Total > 0 && a.Concurrency > 0 {
		var mu sync.Mutex
		var all []time.Duration
		errs := 0
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
					t0 := time.Now()
					ctx, cancel := context.WithTimeout(context.Background(), to)
					_, err := client.Version(ctx, &pb.VersionRequest{})
					cancel()
					d := time.Since(t0)
					mu.Lock()
					if err != nil {
						errs++
						if firstErr == nil {
							firstErr = err
						}
					} else {
						all = append(all, d)
					}
					mu.Unlock()
				}
			}()
		}
		wg.Wait()
		out["concurrent"] = summarize(all, errs, time.Since(start), a.Concurrency)
		if firstErr != nil {
			out["concurrent_status"] = statusOf(firstErr)
		}
	}
	return out, nil
}

// rawCodec sends and receives message bytes as they are, so a call can carry
// a malformed request. Its name is "proto", so the content-type stays
// application/grpc as with the generated client.
type rawCodec struct{}

func (rawCodec) Marshal(v any) ([]byte, error) { return *(v.(*[]byte)), nil }
func (rawCodec) Unmarshal(data []byte, v any) error {
	*(v.(*[]byte)) = append([]byte(nil), data...)
	return nil
}
func (rawCodec) Name() string { return "proto" }

type callArgs struct {
	dialArgs
	Method   string            `json:"method"`
	BodyHex  string            `json:"body_hex"`
	Metadata map[string]string `json:"metadata"`
}

func cmdCall(raw json.RawMessage) (any, error) {
	var a callArgs
	if err := json.Unmarshal(raw, &a); err != nil {
		return nil, err
	}
	body, err := hex.DecodeString(a.BodyHex)
	if err != nil {
		return nil, err
	}
	conn, err := dial(&a.dialArgs)
	if err != nil {
		return nil, err
	}
	defer conn.Close()
	ctx, cancel := context.WithTimeout(context.Background(), timeout(a.TimeoutMS))
	defer cancel()
	for k, v := range a.Metadata {
		ctx = metadata.AppendToOutgoingContext(ctx, k, v)
	}
	var hdr, trl metadata.MD
	var reply []byte
	err = conn.Invoke(ctx, a.Method, &body, &reply, grpc.ForceCodec(rawCodec{}),
		grpc.Header(&hdr), grpc.Trailer(&trl))
	out := map[string]any{
		"headers":   mdMap(hdr),
		"trailers":  mdMap(trl),
		"reply_hex": hex.EncodeToString(reply),
	}
	if err != nil {
		out["status"] = statusOf(err)
	} else {
		out["status"] = &statusResult{Code: 0, CodeName: "OK"}
	}
	return out, nil
}

func cmdHTTP1(raw json.RawMessage) (any, error) {
	var a dialArgs
	if err := json.Unmarshal(raw, &a); err != nil {
		return nil, err
	}
	cfg, err := clientTLS(&a, []string{"http/1.1"})
	if err != nil {
		return nil, err
	}
	tr := &http.Transport{TLSClientConfig: cfg, ForceAttemptHTTP2: false}
	client := &http.Client{Transport: tr, Timeout: timeout(a.TimeoutMS)}
	resp, err := client.Post("https://"+a.Addr+"/walletrpc.VersionService/Version",
		"application/grpc", nil)
	if err != nil {
		return map[string]any{"error": err.Error()}, nil
	}
	defer resp.Body.Close()
	body, _ := io.ReadAll(io.LimitReader(resp.Body, 4096))
	return map[string]any{"status": resp.StatusCode, "proto": resp.Proto, "body": string(body)}, nil
}
