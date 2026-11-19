// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

package main

import (
	"context"
	"crypto/tls"
	"crypto/x509"
	"encoding/json"
	"errors"
	"net"
	"os"
	"sync"

	pb "decred.org/dcrwallet/v5/rpc/walletrpc"
	"google.golang.org/grpc"
	"google.golang.org/grpc/credentials"
)

// The reference server: dcrwallet's gRPC server as startRPCServers builds it
// (rpcserver.go:265-347) with the VersionService implementation of
// internal/rpc/rpcserver (server.go:322-329). The TLS configuration is
// dcrwallet's: the key pair, MinVersion TLS 1.2, ClientCAs from the client
// CA file and ClientAuth RequireAndVerifyClientCert, handed to
// credentials.NewTLS, with no other server option but the two interceptors,
// which here only check service readiness the way dcrwallet's do for a
// service that is always ready (VersionService) and so are left out.

type versionServer struct {
	pb.UnimplementedVersionServiceServer
}

func (*versionServer) Version(ctx context.Context, req *pb.VersionRequest) (*pb.VersionResponse, error) {
	return &pb.VersionResponse{
		VersionString: "10.0.0",
		Major:         10,
		Minor:         0,
		Patch:         0,
	}, nil
}

var (
	refMu     sync.Mutex
	refServer *grpc.Server
)

type serverArgs struct {
	Cert     string `json:"cert"`
	Key      string `json:"key"`
	ClientCA string `json:"clientca"`
}

func cmdServerStart(raw json.RawMessage) (any, error) {
	var a serverArgs
	if err := json.Unmarshal(raw, &a); err != nil {
		return nil, err
	}
	refMu.Lock()
	defer refMu.Unlock()
	if refServer != nil {
		return nil, errors.New("reference server already running")
	}
	keyPair, err := tls.LoadX509KeyPair(a.Cert, a.Key)
	if err != nil {
		return nil, err
	}
	tlsConfig := &tls.Config{
		Certificates: []tls.Certificate{keyPair},
		MinVersion:   tls.VersionTLS12,
		ClientCAs:    x509.NewCertPool(),
	}
	cafile, err := os.ReadFile(a.ClientCA)
	if err != nil {
		return nil, err
	}
	if !tlsConfig.ClientCAs.AppendCertsFromPEM(cafile) {
		return nil, errors.New("no certificates added from CA file")
	}
	tlsConfig = tlsConfig.Clone()
	tlsConfig.ClientAuth = tls.RequireAndVerifyClientCert
	lis, err := net.Listen("tcp4", "127.0.0.1:0")
	if err != nil {
		return nil, err
	}
	server := grpc.NewServer(grpc.Creds(credentials.NewTLS(tlsConfig)))
	pb.RegisterVersionServiceServer(server, &versionServer{})
	go func() { _ = server.Serve(lis) }()
	refServer = server
	return map[string]string{"addr": lis.Addr().String()}, nil
}

func cmdServerStop(json.RawMessage) (any, error) {
	refMu.Lock()
	defer refMu.Unlock()
	if refServer != nil {
		refServer.Stop()
		refServer = nil
	}
	return map[string]bool{"stopped": true}, nil
}
