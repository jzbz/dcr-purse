module github.com/jzbz/dcr-purse/tools/oracle

go 1.26.0

// dcrwallet's daemon builds with Go 1.25's GODEBUG defaults (its debug.go:
// `//go:debug default=go1.25`), so the oracle runs with the same ones.
godebug default=go1.25

require (
	decred.org/cspp/v2 v2.4.0
	decred.org/dcrwallet/v5 v5.0.1-0.20261105142009-54b85b336077
	github.com/decred/dcrd/blockchain/stake/v5 v5.0.3
	github.com/decred/dcrd/blockchain/standalone/v2 v2.3.0
	github.com/decred/dcrd/chaincfg/chainhash v1.0.5
	github.com/decred/dcrd/chaincfg/v3 v3.3.0
	github.com/decred/dcrd/crypto/rand v1.0.1
	github.com/decred/dcrd/crypto/ripemd160 v1.0.2
	github.com/decred/dcrd/dcrec v1.0.1
	github.com/decred/dcrd/dcrec/secp256k1/v4 v4.4.1
	github.com/decred/dcrd/dcrutil/v4 v4.0.3
	github.com/decred/dcrd/gcs/v4 v4.1.1
	github.com/decred/dcrd/hdkeychain/v3 v3.1.3
	github.com/decred/dcrd/mixing v0.7.5
	github.com/decred/dcrd/txscript/v4 v4.1.2
	github.com/decred/dcrd/wire v1.8.0
	github.com/decred/slog v1.2.0
	go.etcd.io/bbolt v1.3.12
	golang.org/x/crypto v0.57.0
	golang.org/x/net v0.60.0
	google.golang.org/grpc v1.83.2
)

require (
	github.com/agl/ed25519 v0.0.0-20170116200512-5312a6153412 // indirect
	github.com/companyzero/sntrup4591761 v0.0.0-20220309191932-9e0f3af2f07a // indirect
	github.com/dchest/siphash v1.2.3 // indirect
	github.com/decred/base58 v1.0.6 // indirect
	github.com/decred/dcrd/container/lru v1.0.0 // indirect
	github.com/decred/dcrd/crypto/blake256 v1.1.0 // indirect
	github.com/decred/dcrd/database/v3 v3.0.3 // indirect
	github.com/decred/dcrd/dcrec/edwards/v2 v2.0.4 // indirect
	github.com/decred/dcrd/dcrjson/v4 v4.2.0 // indirect
	github.com/decred/dcrd/rpc/jsonrpc/types/v4 v4.4.0 // indirect
	github.com/decred/vspd/client/v4 v4.0.2 // indirect
	github.com/decred/vspd/types/v3 v3.0.0 // indirect
	github.com/gorilla/websocket v1.5.1 // indirect
	github.com/jrick/bitset v1.0.0 // indirect
	github.com/jrick/logrotate v1.0.0 // indirect
	github.com/jrick/wsrpc/v2 v2.3.8 // indirect
	github.com/klauspost/cpuid/v2 v2.2.8 // indirect
	golang.org/x/sync v0.23.0 // indirect
	golang.org/x/sys v0.48.0 // indirect
	golang.org/x/text v0.42.0 // indirect
	google.golang.org/genproto/googleapis/rpc v0.0.0-20260526163538-3dc84a4a5aaa // indirect
	google.golang.org/protobuf v1.36.11 // indirect
	lukechampine.com/blake3 v1.3.0 // indirect
)
