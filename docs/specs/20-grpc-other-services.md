# 20 — gRPC API: loader, account mixer, ticket buyer, seed, agenda, voting, message verification, network and decode services

## 1. Header

**Scope** (dcrwallet, line counts from `wc -l`):

| File | Lines | Covered here |
|---|---|---|
| `rpc/api.proto` | 1341 | Services `WalletLoaderService`, `AccountMixerService`, `TicketBuyerService`, `SeedService`, `AgendaService`, `VotingService`, `MessageVerificationService`, `NetworkService`, `DecodeMessageService` (`:94-135`, `:1003-1005`) and their messages (`:728-1065`, `:1192-1199`). `VersionService` and `WalletService` belong to the sibling gRPC spec. |
| `internal/rpc/rpcserver/server.go` | 4312 | `:1-320` (constants, error translation, server types, registration, readiness), `:491-495` (`zero`), `:2245-2258` (`marshalTxType`, shared), `:2511-3725` (all nine services), `:3788-3846` (`networkServer`). The `walletServer` methods belong to the sibling spec. |
| `internal/rpc/rpcserver/log.go` | 81 | all |
| `internal/loader/loader.go` | 415 | all |
| `internal/loader/doc.go`, `log.go` | 11, 20 | all |
| `rpc/documentation/api.md` | 3070 | `:1-408` (preamble, `VersionService`, `WalletLoaderService`), `:2259-2462` (`DecodedTransaction`, sync notification messages), `:2707-3070` (the remaining eight services) |
| `rpc/documentation/clientusage.md` | 460 | all |
| `rpc/documentation/serverchanges.md`, `README.md` | 94, 16 | all |
| `rpc/grpc_example/main.go` | 308 | all |
| `rpc/client/dcrwallet/client.go` | 63 | all; `methods.go` (982) and `marshaling.go` (234) inventoried by function list only (this package is a JSON-RPC client, not gRPC; see 3.7) |
| `walletseed/seed.go` | 102 | all (driven by `SeedService`; owned by no other spec) |
| `pgpwordlist/pgpwordlist.go`, `wordlist.go` | 60, 544 | all (same reason) |

Supporting code read to pin observable behaviour: `rpcserver.go:248-447`
(gRPC server construction and interceptors), `dcrwallet.go:78-431`
(start-up ordering), `dcrwallet.go:531-600` (main-package sync loops, for
comparison), `errors/errors.go` (all), `wallet/wallet.go:268-314`,
`:400-770`, `:1712-1849`, `:2036-2063`, `:2752-2790`, `:5292-5325`,
`wallet/rescan.go:462-512`, `wallet/network.go:61-71`,
`wallet/vspclientloader.go`, `wallet/vsp.go:58-94`,
`wallet/udb/txmined.go:327-363`, `wallet/internal/bdb/db.go:18-36`,
`:316-323`, `chain/sync.go:62-108`, `:136-215`, `:516-600`,
`spv/sync.go:110-316`, `ticketbuyer/tb.go` (all),
`internal/cfgutil/normalization.go`, `internal/netparams/params.go`.

**Parity pin**

| Component | Version |
|---|---|
| dcrwallet | master `54b85b33`, version 2.2.0-pre, module `decred.org/dcrwallet/v5` |
| `google.golang.org/grpc` | v1.83.2 (`server.go` error conversion and defaults, `status/status.go`, `grpclog/logger.go`) |
| `google.golang.org/protobuf` | v1.36.11 (`internal/impl/codec_gen.go`, `codec_tables.go`: proto3 bytes decoding) |
| generated stubs | `protoc-gen-go` v1.36.10, `protoc-gen-go-grpc` v1.5.1, `protoc` v6.33.0 (`rpc/walletrpc/api.pb.go:1-5`, `api_grpc.pb.go:1-5`) |
| `github.com/decred/dcrd/hdkeychain/v3` | v3.1.3 (seed length constants) |
| `github.com/decred/dcrd/blockchain/stake/v5` | v5.0.3 (`staketx.go`: commitment helpers, treasury vote constants) |
| `github.com/decred/dcrd/txscript/v4` | v4.1.2 (`stdaddr`, `stdscript` type enumeration) |
| `github.com/decred/dcrd/chaincfg/v3` | v3.3.0 (deployments) |
| `github.com/decred/dcrd/chaincfg/chainhash` | v1.0.5 (`NewHash` error text) |
| `github.com/decred/dcrd/dcrec/secp256k1/v4` | v4.4.1 (`ecdsa.RecoverCompact` error text) |
| `github.com/decred/dcrd/addrmgr/v3` | v3.0.0 (`New`) |

**Summary**

The wallet exposes eleven gRPC services from one `grpc.Server`. This spec
covers the nine that are not `VersionService` or `WalletService`. Five of
them (`WalletLoaderService`, `SeedService`, and the parameter-only
`AgendaService`, `MessageVerificationService`, `DecodeMessageService`) work
without a loaded wallet; four (`AccountMixerService`, `TicketBuyerService`,
`VotingService`, `NetworkService`) are gated by a readiness flag that is set
once, when the loader first loads a wallet, and never cleared. The loader
service creates, opens and closes `wallet.db` through `internal/loader` and
runs the dcrd-RPC or SPV syncer for the lifetime of a server-streaming RPC,
turning syncer callbacks into a stream of `SyncNotificationType` messages.
The mixer and ticket buyer RPCs run a `ticketbuyer.TB` for the lifetime of a
stream that never carries a message. Several behaviours are bugs a
drop-in replacement has to reproduce or consciously fix: the birthday state
written by `CreateWallet` ignores `birth_time`, `DecodeRawTransaction` never
populates `commitment_amount`, a wallet reopened over gRPC is not rebound to
the wallet-dependent services, and the passphrase "lock on exit" of the
mixer and ticket buyer RPCs has no effect. The published `api.md` is stale
(it says API version 7.13.x; the code reports 10.0.0) and disagrees with the
code on many status codes.

---

## 2. Role in the system

```
gRPC client (a GUI front end such as Decrediton, or any custom client)
   │  HTTP/2 + TLS 1.2+, mutual TLS (client certificate required)
   ▼
grpc.Server  (rpcserver.go:328-332; one instance, all listeners)
   │  interceptUnary / interceptStreaming  → rpcserver.ServiceReady
   ▼
rpcserver singletons (server.go:258-270)
   ├─ loaderServer ──────────► internal/loader.Loader ─► wallet.CreateDB/OpenDB,
   │      │                                              wallet.Create/CreateWatchOnly/Open
   │      ├─ RpcSync ────────► chain.NewSyncer(...).Run        (spec 15)
   │      └─ SpvSync ────────► p2p.NewLocalPeer, spv.NewSyncer(...).Run (spec 14)
   ├─ accountMixerServer ────► ticketbuyer.New(...).Run  (mix only)   (specs 11, 13)
   ├─ ticketbuyerServer ─────► wallet.VSP, ticketbuyer.New(...).Run   (spec 11)
   ├─ seedServer ────────────► dcrd crypto/rand, walletseed, pgpwordlist
   ├─ agendaServer ──────────► wallet.CurrentAgendas (chaincfg deployments)
   ├─ votingServer ──────────► wallet.AgendaChoices/SetAgendaChoices,
   │                           TSpendPolicy/SetTSpendPolicy, TreasuryKeyPolicies/
   │                           SetTreasuryKeyPolicy                    (spec 11)
   ├─ messageVerificationServer ► stdaddr.DecodeAddress, wallet.VerifyMessage
   ├─ decodeMessageServer ───► wire.MsgTx.Deserialize, txscript.DisasmString,
   │                           stdscript.ExtractAddrs, stake helpers
   └─ networkServer ─────────► wallet.NetworkBackend().Blocks, wallet.BlockInfo
```

Callers: any gRPC client holding a certificate trusted by the wallet's client
CA bundle. The services are registered only when the gRPC server is started
(`rpcserver.go:320-346`); when it is not (TLS disabled, `--nogrpc`, or no
client CA), none of this subsystem exists at run time, except
`internal/loader`, which the main package and the JSON-RPC server always use
(`dcrwallet.go:168-172`, `rpcserver.go:383`).

`internal/loader.Loader` is the single owner of the open `wallet.db` handle
and the `*wallet.Wallet`. It is used by: `main.run` (initial open, final
unload), `walletsetup.go` (`--create`), the JSON-RPC server (per-request
`LoadedWallet`), and `loaderServer`.

---

## 3. Public surface inventory

### 3.1 Package `rpcserver` (`internal/rpc/rpcserver/server.go`) — items in scope

| Name | Kind / signature | Contract |
|---|---|---|
| `semverString`, `semverMajor`, `semverMinor`, `semverPatch` | consts `"10.0.0"`, `10`, `0`, `0` (`:63-68`) | gRPC API version reported by `VersionService.Version` (sibling spec). Listed because `api.md:3` claims `7.13.x`. |
| `scriptVersionAssumed` | const `0` (`:72`) | Unused by the services in this spec. |
| `translateError` | `func(err error) error` (`:80`) | `status.Errorf(errorCode(err), "%s", err.Error())`. Section 5.3. |
| `errorCode` | `func(err error) codes.Code` (`:85`) | Maps wallet error kinds to gRPC codes. Section 5.3. |
| `decodeAddress` | `func(a string, params *chaincfg.Params) (stdaddr.Address, error)` (`:141`) | `stdaddr.DecodeAddress`; on failure `InvalidArgument` `invalid address <a>: <err>`. |
| `decodeHashes` | `func(in [][]byte) ([]*chainhash.Hash, error)` (`:149`) | Not used by the services in this spec. |
| `zero` | `func(b []byte)` (`:491`) | Overwrites every byte of `b` with 0. |
| `loaderServer` | struct `{loader *loader.Loader; activeNet *netparams.Params}` (`:180`) | No readiness flag: always callable once registered. |
| `seedServer` | struct `{}` (`:189`) | Stateless. |
| `accountMixerServer` | struct `{ready bool; readyMtx sync.RWMutex; wallet *wallet.Wallet}` (`:195`) | Readiness-gated. |
| `ticketbuyerServer` | same shape (`:207`) | Readiness-gated. |
| `agendaServer` | struct `{activeNet *chaincfg.Params}` (`:217`) | No readiness flag. |
| `votingServer` | struct `{ready; readyMtx; wallet}` (`:222`) | Readiness-gated. |
| `messageVerificationServer` | struct `{chainParams *chaincfg.Params}` (`:234`) | No readiness flag. |
| `decodeMessageServer` | struct `{chainParams *chaincfg.Params}` (`:239`) | No readiness flag. |
| `networkServer` | struct `{ready; readyMtx; wallet}` (`:246`) | Readiness-gated. |
| `versionService`, `walletService`, `loaderService`, `seedService`, `accountMixerService`, `ticketBuyerService`, `agendaService`, `votingService`, `messageVerificationService`, `decodeMessageService`, `networkService` | package-level singleton vars (`:258-270`) | One instance per process; every listener serves the same instances. |
| `RegisterServices` | `func(server *grpc.Server)` (`:274`) | Registers all eleven services, in the order of 5.1. |
| `serviceMap` | `map[string]any` (`:288`) | Fully-qualified service name → singleton. Keys in 5.1. |
| `ServiceReady` | `func(service string) error` (`:303`) | `nil` when ready. Unknown name: `Unimplemented` `service <name> not found`. Not ready: `FailedPrecondition` `service <name> is not ready`. |
| `StartWalletLoaderService` | `func(loader *loader.Loader, activeNet *netparams.Params)` (`:2512`) | Sets the two fields. No lock, no double-start guard. |
| `StartAccountMixerService` | `func(wallet *wallet.Wallet)` (`:2518`) | Sets `ready`, `wallet` under `readyMtx`; panics `service already started` on a second call. |
| `StartTicketBuyerService` | `func(wallet *wallet.Wallet)` (`:2575`) | Same. |
| `StartVotingService` | `func(wallet *wallet.Wallet)` (`:3242`) | Same. |
| `StartNetworkService` | `func(wallet *wallet.Wallet)` (`:3789`) | Same. |
| `StartAgendaService` | `func(activeNet *chaincfg.Params)` (`:3207`) | Sets the field. |
| `StartMessageVerificationService` | `func(chainParams *chaincfg.Params)` (`:3519`) | Sets the field. |
| `StartDecodeMessageService` | `func(chainParams *chaincfg.Params)` (`:3550`) | Sets the field. |
| `(*accountMixerServer).checkReady`, `(*ticketbuyerServer).checkReady`, `(*votingServer).checkReady`, `(*networkServer).checkReady` | `func() bool` (`:2568`, `:2688`, `:3254`, `:3801`) | Read `ready` under `readyMtx.RLock`. Presence of this method is what makes a service readiness-gated (5.1). |
| `isLoopback` | `func(addr string) bool` (`:2817`) | Section 5.5.7 step 5. |
| `marshalDecodedTxInputs` | `func(mtx *wire.MsgTx) []*pb.DecodedTransaction_Input` (`:3554`) | Section 5.12. |
| `marshalDecodedTxOutputs` | `func(mtx *wire.MsgTx, chainParams *chaincfg.Params) []*pb.DecodedTransaction_Output` (`:3579`) | Section 5.12. |
| `scProto` | `func(class stdscript.ScriptType) int32` (`:3664`) | `stdscript.ScriptType` → `ScriptClass` number. Table in 4.4. |
| `marshalTxType` | `func(wallet.TransactionType) pb.TransactionDetails_TransactionType` (`:2245`) | Shared with `WalletService`; used here by `DecodeRawTransaction`. Table in 4.4. |
| RPC handler methods | 22 methods listed in 3.3 | Section 5. |
| `pb.Unimplemented<Service>Server` | embedded by value in each of the eleven server structs (`:164`, `:175`, `:183`, `:190`, `:202`, `:214`, `:219`, `:229`, `:236`, `:241`, `:253`) | Generated forward-compatibility stub. A method present in `api.proto` but not implemented by the struct would answer `Unimplemented`, `method <Method> not implemented` (`rpc/walletrpc/api_grpc.pb.go:66`). Every method of the nine services in scope is implemented, so the stub is never reached. |
| `readyChecker` | interface `{ checkReady() bool }`, declared inside `ServiceReady` (`:308-310`) | The type assertion that decides whether a service is gated. |
| `versionServer`, `(*versionServer).Version`, `walletServer`, `StartWalletService`, `(*walletServer).checkReady`, `(*walletServer).requireNetworkBackend`, `decodeHashes`, and every other `walletServer` method and `marshal*` helper of `server.go` | (`:163`, `:322`, `:168`, `:332`, `:344`, `:352`, `:149`, `:361-2509`, `:3727-3786`, `:3848-4312`) | Owned by the sibling gRPC spec (19). Named here only so that the file's inventory is complete across the two specs. |

`internal/rpc/rpcserver/log.go`:

| Name | Signature | Contract |
|---|---|---|
| `UseLogger` | `func(l slog.Logger)` (`:27`) | `grpclog.SetLogger(logger{l})`. Called once from `log.go:38` (package `main`) with the `GRPC` subsystem logger. |
| `logger` | struct embedding `slog.Logger` (`:32`) | Implements the deprecated `grpclog.Logger` (v1) interface. |
| `stripGrpcPrefix` | `func(string) string` (`:38`) | `strings.TrimPrefix(s, "grpc: ")`. |
| `stripGrpcPrefixArgs` | `func([]any) []any` (`:45`) | Strips the prefix from `args[0]` when it is a string; mutates and returns the same slice. |
| `Fatal`, `Fatalf`, `Fatalln` | (`:56-69`) | Log at Critical, then `os.Exit(1)`. |
| `Print`, `Printf`, `Println` | (`:71-81`) | Log at Info. |

grpc-go wraps a v1 logger so that its Info, Warning and Error output all go
through `Print*` (`grpc@v1.83.2/grpclog/logger.go:33`), so every grpc-go
library log line appears at level INF under subsystem `GRPC`, and verbosity
checks (`V(n)`) are false.

### 3.2 Package `loader` (`internal/loader`)

| Name | Signature | Contract |
|---|---|---|
| `walletDbName` | const `"wallet.db"` (`loader.go:22`) | File name inside the network directory. |
| `driver` | const `"bdb"` (`:23`) | walletdb driver name (bbolt; spec 01). |
| `Loader` | struct (`:33-54`) | Fields: `callbacks []func(*wallet.Wallet)`, `chainParams`, `dbDirPath string`, `wallet *wallet.Wallet`, `db wallet.DB`, the wallet options `votingEnabled bool`, `gapLimit uint32`, `watchLast uint32`, `accountGapLimit int`, `disableCoinTypeUpgrades bool`, `mixingEnabled bool`, `allowHighFees bool`, `manualTickets bool`, `relayFee dcrutil.Amount`, `vspMaxFee dcrutil.Amount`, `mixSplitLimit int`, `dialer wallet.DialFunc`, and `mu sync.Mutex`. "Safe for concurrent access". |
| `NewLoader` | `func(chainParams *chaincfg.Params, dbDirPath string, votingEnabled bool, gapLimit uint32, watchLast uint32, allowHighFees bool, relayFee dcrutil.Amount, vspMaxFee dcrutil.Amount, accountGapLimit int, disableCoinTypeUpgrades bool, mixingEnabled bool, manualTickets bool, mixSplitLimit int, dialer wallet.DialFunc) *Loader` (`:57`) | Stores the arguments. No I/O. |
| `(*Loader).onLoaded` | `func(w *wallet.Wallet, db wallet.DB)` (`:81`), unexported | With `mu` held: runs every callback in registration order, sets `wallet`, `db`, then sets `callbacks = nil`. |
| `(*Loader).RunAfterLoad` | `func(fn func(*wallet.Wallet))` (`:94`) | If a wallet is loaded, calls `fn(w)` immediately on the caller's goroutine after releasing `mu`; otherwise appends `fn` to `callbacks`. |
| `(*Loader).CreateWatchingOnlyWallet` | `func(ctx, extendedPubKey string, pubPass []byte) (*wallet.Wallet, error)` (`:108`) | Section 5.4.2. |
| `(*Loader).CreateNewWallet` | `func(ctx, pubPassphrase, privPassphrase, seed []byte) (*wallet.Wallet, error)` (`:198`) | Section 5.4.1. |
| `(*Loader).OpenExistingWallet` | `func(ctx, pubPassphrase []byte) (*wallet.Wallet, error)` (`:288`) | Section 5.4.3. |
| `(*Loader).DbDirPath` | `func() string` (`:345`) | Returns `dbDirPath` without locking. |
| `(*Loader).WalletExists` | `func() (bool, error)` (`:351`) | `os.Stat(<dbDirPath>/wallet.db)`; not-exist → `(false, nil)`; other stat error → `errors.E("loader.WalletExists", err)`. No lock. |
| `(*Loader).LoadedWallet` | `func() (*wallet.Wallet, bool)` (`:364`) | Returns the pointer and `w != nil`, under `mu`. |
| `(*Loader).UnloadWallet` | `func() error` (`:375`) | Section 5.4.4. |
| `(*Loader).NetworkBackend` | `func() (wallet.NetworkBackend, bool)` (`:397`) | Under `mu`: the loaded wallet's backend or nil; `ok = n != nil`. Used by the JSON-RPC server. |
| `fileExists` | `func(filePath string) (bool, error)` (`:406`) | `os.Stat`; `IsNotExist` → `(false, nil)`. |
| `log`, `UseLogger` | `log.go:13`, `:18` | Package logger, `slog.Disabled` until `UseLogger`; main wires subsystem `LODR` (`log.go:31` in package `main`). |

### 3.3 gRPC services and methods in scope

Package name `walletrpc` (`rpc/api.proto:3`). The HTTP/2 `:path` of every
method is `/walletrpc.<Service>/<Method>`.

| Service | Method | Kind | Request → Response | Handler (`server.go`) |
|---|---|---|---|---|
| `WalletLoaderService` | `WalletExists` | unary | `WalletExistsRequest` → `WalletExistsResponse` | `:2793` |
| | `CreateWallet` | unary | `CreateWalletRequest` → `CreateWalletResponse` | `:2694` |
| | `CreateWatchingOnlyWallet` | unary | `CreateWatchingOnlyWalletRequest` → `CreateWatchingOnlyWalletResponse` | `:2757` |
| | `OpenWallet` | unary | `OpenWalletRequest` → `OpenWalletResponse` | `:2774` |
| | `CloseWallet` | unary | `CloseWalletRequest` → `CloseWalletResponse` | `:2803` |
| | `SpvSync` | server streaming | `SpvSyncRequest` → stream `SpvSyncResponse` | `:2984` |
| | `RpcSync` | server streaming | `RpcSyncRequest` → stream `RpcSyncResponse` | `:2832` |
| | `RescanPoint` | unary | `RescanPointRequest` → `RescanPointResponse` | `:3160` |
| `AccountMixerService` | `RunAccountMixer` | server streaming | `RunAccountMixerRequest` → stream `RunAccountMixerResponse` | `:2531` |
| `TicketBuyerService` | `RunTicketBuyer` | server streaming | `RunTicketBuyerRequest` → stream `RunTicketBuyerResponse` | `:2588` |
| `SeedService` | `GenerateRandomSeed` | unary | `GenerateRandomSeedRequest` → `GenerateRandomSeedResponse` | `:3177` |
| | `DecodeSeed` | unary | `DecodeSeedRequest` → `DecodeSeedResponse` | `:3199` |
| `AgendaService` | `Agendas` | unary | `AgendasRequest` → `AgendasResponse` | `:3211` |
| `VotingService` | `VoteChoices` | unary | `VoteChoicesRequest` → `VoteChoicesResponse` | `:3260` |
| | `SetVoteChoices` | unary | `SetVoteChoicesRequest` → `SetVoteChoicesResponse` | `:3302` |
| | `TSpendPolicies` | unary | `TSpendPoliciesRequest` → `TSpendPoliciesResponse` | `:3330` |
| | `SetTSpendPolicy` | unary | `SetTSpendPolicyRequest` → `SetTSpendPolicyResponse` | `:3400` |
| | `TreasuryPolicies` | unary | `TreasuryPoliciesRequest` → `TreasuryPoliciesResponse` | `:3449` |
| | `SetTreasuryPolicy` | unary | `SetTreasuryPolicyRequest` → `SetTreasuryPolicyResponse` | `:3482` |
| `MessageVerificationService` | `VerifyMessage` | unary | `VerifyMessageRequest` → `VerifyMessageResponse` | `:3523` |
| `NetworkService` | `GetRawBlock` | unary | `GetRawBlockRequest` → `GetRawBlockResponse` | `:3807` |
| `DecodeMessageService` | `DecodeRawTransaction` | unary | `DecodeRawTransactionRequest` → `DecodeRawTransactionResponse` | `:3699` |

Messages defined in `api.proto` that no RPC references (dead definitions that
still appear in the generated descriptor and must stay in the `.proto` for
descriptor compatibility): `StartConsensusRpcRequest/Response` (`:760-766`),
`DiscoverAddressesRequest/Response` (`:768-773`),
`FetchMissingCFiltersRequest/Response` (`:775-776`),
`SubscribeToBlockNotificationsRequest/Response` (`:778-779`),
`FetchHeadersRequest/Response` (`:781-788`).

No gRPC reflection service and no gRPC health service are registered
(`rpcserver.go:328-337` registers only `rpcserver.RegisterServices`).

### 3.4 Package `walletseed` (`walletseed/seed.go`)

| Name | Signature | Contract |
|---|---|---|
| `GenerateRandomSeed` | `func(size uint) ([]byte, error)` (`:21`) | Not used by the gRPC service (used by `--create`). `size >= 255` or outside `[16, 64]` → `errors.E(op, errors.Invalid, hdkeychain.ErrInvalidSeedLen)`; otherwise `hdkeychain.GenerateSeed(uint8(size))`. |
| `checksumByte` | `func(data []byte) byte` (`:38`), unexported | First byte of `SHA-256(SHA-256(data))`. |
| `EncodeMnemonicSlice` | `func(seed []byte) []string` (`:44`) | `len(seed)+1` words: one per byte, then one for the checksum byte at position `len(seed)`. |
| `EncodeMnemonic` | `func(seed []byte) string` (`:55`) | Same words joined by single U+0020 spaces. For an empty seed the result is `" "` followed by the checksum word (a leading space), because the separator before the checksum word is unconditional. |
| `DecodeUserInput` | `func(input string) ([]byte, error)` (`:71`) | Section 5.8.2. |

### 3.5 Package `pgpwordlist`

| Name | Signature | Contract |
|---|---|---|
| `wordList` | `[]string`, 512 entries (`wordlist.go:23`, data `:33-544`) | `strings.Split` of a newline-separated constant. Index `2*b` is the even-position word for byte `b`, `2*b+1` the odd-position word. |
| `wordIndexes` | `map[string]uint16` (`wordlist.go:25-31`) | Lower-cased word → index. No two words collide case-insensitively. |
| `ByteToMnemonic` | `func(b byte, index int) string` (`pgpwordlist.go:26`) | `wordList[2*b + (index % 2 != 0 ? 1 : 0)]`. Words are returned with the capitalisation of the list (50 entries contain upper-case letters, e.g. `Istanbul`, `Pluto`). |
| `DecodeMnemonics` | `func(words []string) ([]byte, error)` (`pgpwordlist.go:36`) | Section 5.8.2 step 4. |

### 3.6 Package `main` glue (owned by spec 16; restated where this subsystem depends on it)

| Name | Location | Contract |
|---|---|---|
| `startRPCServers` | `rpcserver.go:248` | Builds the `grpc.Server`, registers services, starts the four wallet-independent services. Section 5.1. |
| `serviceName` | `rpcserver.go:399` | `/pkg.Service/Method` → `pkg.Service`. |
| `interceptStreaming`, `interceptUnary` | `rpcserver.go:406`, `:432` | Logging and readiness gate. Section 5.2. |
| gRPC `RunAfterLoad` callback | `dcrwallet.go:388-394` | Starts the five wallet-dependent services, in the order `Wallet`, `TicketBuyer`, `AccountMixer`, `Network`, `Voting`. |

### 3.7 Client-side artefacts

| Artefact | What it is |
|---|---|
| `rpc/walletrpc/api.pb.go`, `api_grpc.pb.go` | Generated Go bindings, committed. Package `walletrpc`, import path `decred.org/dcrwallet/v5/rpc/walletrpc`. Regenerated by `rpc/regen.sh` (`protoc -I. api.proto --go_out=walletrpc --go-grpc_out=walletrpc --go_opt=paths=source_relative --go-grpc_opt=paths=source_relative`). `serverchanges.md:76` still shows the obsolete `--go_out=plugins=grpc:walletrpc` invocation. |
| `rpc/grpc_example/main.go` | Example Go client. Reads the server certificate from `<dcrwallet appdata>/rpc.cert` as the only root CA, loads a client key pair from `client.pem` / `client-key.pem` in the working directory (the certificate "must be part of ~/.dcrwallet/clients.pem", `:26`), dials `localhost:19111` (testnet gRPC port) and calls `WalletService.Balance`, `DecodeMessageService.DecodeRawTransaction` for seven hard-coded testnet transactions (`:236-252`), `ImportScript`, `ImportVotingAccountFromSeed`, `Accounts`, `NextAddress`, `ValidateAddress`, `Address`, `UnlockAccount`, `DumpPrivateKey`. Output is `prototext` multi-line. |
| `rpc/documentation/clientusage.md` | Go, C++, C#, Node.js and Python examples, all calling `WalletService.Balance` on `localhost:19111`. Only the Go example supplies a client certificate; the other four pass only the server certificate and therefore cannot connect to the server as actually configured (10.Q21). The non-Go examples set `GRPC_SSL_CIPHER_SUITES=HIGH+ECDSA` before loading gRPC Core. |
| `rpc/client/dcrwallet` | A typed **JSON-RPC** client (not gRPC): `Client{Caller, net}` over a `Caller` interface (`Call(ctx, method, res, args...)`), with `RawRequestCaller` adapting a `RawRequester`. It does not dial; the caller supplies a transport (for example a `wsrpc` client). 79 `Client` methods wrapping JSON-RPC commands (`methods.go`). Out of scope for a daemon replacement; it is a Go library other programs import. Its full inventory (`Caller`, `RawRequester`, `RawRequestCaller`, `Client`, `NewClient`, `GapPolicy` and `SigHashType` constants, `SignatureErrors`, the marshaling helpers and its two test files) is owned by spec 22. |
| `rpc/regen-docker.sh`, `rpc/Dockerfile` | Reproducible stub generation: a pinned `golang` 1.24.10-alpine3.22 image (by digest), `protoc` release 33.0 (`Dockerfile:1-17`), running `./regen.sh` in `/build/rpc`. `regen.sh` puts `rpc/tools/bin` first on `PATH` for `protoc-gen-go` and `protoc-gen-go-grpc`. Not needed at run time; relevant only if the Rust project regenerates a Go oracle client. |

How a client is expected to connect, as shown by the only complete example
(`rpc/grpc_example/main.go:39-62`):

1. Build a root pool containing only the wallet's `rpc.cert` (read from
   `dcrutil.AppDataDir("dcrwallet", false)`; failure to parse →
   `no certificates found in <path>`).
2. Load the client key pair with `tls.LoadX509KeyPair("client.pem",
   "client-key.pem")`; the certificate must already be in the wallet's
   client CA file.
3. `grpc.NewClient("localhost:19111",
   grpc.WithTransportCredentials(credentials.NewTLS(&tls.Config{Certificates,
   RootCAs})))`. No per-RPC credentials, no metadata, no explicit authority
   override: the server certificate must therefore be valid for the dialled
   host name (`localhost`).
4. One `*grpc.ClientConn` is shared by per-service stubs
   (`pb.NewWalletServiceClient(conn)`, `pb.NewDecodeMessageServiceClient(conn)`);
   all eleven services are multiplexed on one HTTP/2 connection.
5. The example exits with status 1 after printing the error on any failure,
   and 0 otherwise (`:30-36`).

---

## 4. Data formats

### 4.1 Transport

- gRPC over HTTP/2, TLS only. There is no plaintext gRPC listener
  (`rpcserver.go:264-347`: the gRPC server is created only inside the
  `!cfg.DisableServerTLS` branch).
- Server TLS config: the wallet's RPC key pair, `MinVersion` TLS 1.2,
  `ClientAuth = RequireAndVerifyClientCert` with the client CA pool
  (`rpcserver.go:272-276`, `:321-322`). A client without a certificate
  chaining to the pool fails the TLS handshake; there is no application-level
  authentication (no username/password, no per-RPC metadata).
- Client CA pool: PEM certificates from `--clientcafile` (default
  `<appdata>/clients.pem`) and/or the ephemeral authority generated by
  `--issueclientcert`. If neither exists the gRPC server is not started
  (spec 16, section 5.1).
- Default listen ports: mainnet 9111, testnet3 19111, simnet 19558
  (`internal/netparams/params.go:25`, `:34`, `:43`).
- Server options: only `Creds`, `StreamInterceptor`, `UnaryInterceptor`
  (`rpcserver.go:328-332`). Everything else is the grpc-go default:
  maximum received message 4 MiB (`grpc@v1.83.2/server.go:61`), maximum sent
  message `math.MaxInt32` (`:62`), no compression required, default
  keepalive parameters and enforcement policy.
- Message framing is the standard gRPC length-prefixed framing (1-byte
  compressed flag, 4-byte big-endian length, protobuf payload). Content type
  `application/grpc`.

### 4.2 Protobuf conventions

- `syntax = "proto3"`, package `walletrpc`, `option go_package =
  "decred.org/dcrwallet/v5/rpc/walletrpc"` (`api.proto:1-5`).
- No field uses `optional`, `oneof`, `map` or `required`. Every scalar has
  implicit presence: a field equal to its default (0, false, empty string,
  empty bytes) is not emitted, and a field absent on the wire reads as the
  default.
- **Bytes fields never distinguish "absent" from "present and empty".** The
  Go runtime decodes a zero-length proto3 `bytes` field to a nil slice
  (`protobuf@v1.36.11/internal/impl/codec_gen.go:5489-5500`, selected by
  `codec_tables.go:200`, `:270-278`). The handlers mix `x != nil`,
  `x == nil` and `len(x) != 0` tests on request bytes fields
  (`server.go:3363`, `:3390`, `:3413`, `:3814`); for values that arrived
  over the wire all three are equivalent to "length is non-zero".
- Message-typed fields have presence. A sub-message that the handler sets is
  emitted even when all of its fields are default (as tag + length 0).
- Repeated message fields are emitted one length-delimited record per
  element, in slice order.
- Go emits fields in ascending field-number order. Every message in scope
  declares its fields in ascending order, so declaration order and
  field-number order coincide.
- Unknown fields in requests are ignored by the handlers.

Tag byte = `(field_number << 3) | wire_type`; wire type 0 = varint, 2 =
length-delimited. `sint32` uses ZigZag varint. `int32`/`int64`/enum negative
values are encoded as 10-byte two's-complement varints.

### 4.3 Messages

Columns: field number, name, proto type, tag byte (hex), notes.

#### WalletLoaderService

`WalletExistsRequest {}` — empty.

`WalletExistsResponse`

| # | Name | Type | Tag |
|---|---|---|---|
| 1 | `exists` | bool | `08` |

`CreateWalletRequest` (`api.proto:728-736`)

| # | Name | Type | Tag | Notes |
|---|---|---|---|---|
| 1 | `public_passphrase` | bytes | `0a` | empty → `"public"` |
| 2 | `private_passphrase` | bytes | `12` | zeroed on return |
| 3 | `seed` | bytes | `1a` | zeroed on return |
| 4 | `set_birth_time` | bool | `20` | |
| 5 | `birth_time` | int64 | `28` | Unix seconds |
| 6 | `set_birth_height` | bool | `30` | |
| 7 | `birth_height` | uint32 | `38` | |

`CreateWalletResponse {}` — empty.

`CreateWatchingOnlyWalletRequest`

| # | Name | Type | Tag |
|---|---|---|---|
| 1 | `extended_pub_key` | string | `0a` |
| 2 | `public_passphrase` | bytes | `12` |

`CreateWatchingOnlyWalletResponse {}` — empty.

`OpenWalletRequest`: 1 `public_passphrase` bytes (`0a`).

`OpenWalletResponse`: 1 `watching_only` bool (`08`).

`CloseWalletRequest {}`, `CloseWalletResponse {}` — empty.

`RescanPointRequest {}` — empty.

`RescanPointResponse`: 1 `rescan_point_hash` bytes (`0a`) — 32 bytes in
internal (wire) byte order, or absent.

`RpcSyncRequest` (`api.proto:827-834`)

| # | Name | Type | Tag |
|---|---|---|---|
| 1 | `network_address` | string | `0a` |
| 2 | `username` | string | `12` |
| 3 | `password` | bytes | `1a` |
| 4 | `certificate` | bytes | `22` |
| 5 | `discover_accounts` | bool | `28` |
| 6 | `private_passphrase` | bytes | `32` |

`SpvSyncRequest` (`api.proto:844-848`)

| # | Name | Type | Tag |
|---|---|---|---|
| 1 | `discover_accounts` | bool | `08` |
| 2 | `private_passphrase` | bytes | `12` |
| 3 | `spv_connect` | repeated string | `1a` |

`RpcSyncResponse` and `SpvSyncResponse` (identical layout, `api.proto:835-842`,
`:849-856`)

| # | Name | Type | Tag |
|---|---|---|---|
| 1 | `synced` | bool | `08` |
| 2 | `notification_type` | `SyncNotificationType` | `10` |
| 3 | `fetch_headers` | `FetchHeadersNotification` | `1a` |
| 4 | `fetch_missing_cfilters` | `FetchMissingCFiltersNotification` | `22` |
| 5 | `rescan_progress` | `RescanProgressNotification` | `2a` |
| 6 | `peer_information` | `PeerNotification` | `32` |

`FetchHeadersNotification`: 1 `fetched_headers_count` int32 (`08`),
2 `last_header_time` int64 (`10`).

`FetchMissingCFiltersNotification`: 1 `fetched_cfilters_start_height` int32
(`08`), 2 `fetched_cfilters_end_height` int32 (`10`).

`RescanProgressNotification`: 1 `rescanned_through` int32 (`08`).

`PeerNotification`: 1 `peer_count` int32 (`08`), 2 `address` string (`12`).

#### AccountMixerService, TicketBuyerService

`RunAccountMixerRequest` (`api.proto:896-901`)

| # | Name | Type | Tag |
|---|---|---|---|
| 1 | `passphrase` | bytes | `0a` |
| 2 | `mixed_account` | uint32 | `10` |
| 3 | `mixed_account_branch` | uint32 | `18` |
| 4 | `change_account` | uint32 | `20` |

`RunAccountMixerResponse {}` — empty; never sent.

`RunTicketBuyerRequest` (`api.proto:879-892`)

| # | Name | Type | Tag |
|---|---|---|---|
| 1 | `passphrase` | bytes | `0a` |
| 2 | `account` | uint32 | `10` |
| 3 | `voting_account` | uint32 | `18` |
| 4 | `balance_to_maintain` | int64 | `20` |
| 5 | `vsp_host` | string | `2a` |
| 6 | `vsp_pubkey` | string | `32` |
| 7 | `limit` | int32 | `38` |
| 8 | `enable_mixing` | bool | `40` |
| 9 | `mixed_account` | uint32 | `48` |
| 10 | `mixed_account_branch` | uint32 | `50` |
| 11 | `mixed_split_account` | uint32 | `58` |
| 12 | `change_account` | uint32 | `60` |

`RunTicketBuyerResponse {}` — empty; never sent.

#### SeedService

`GenerateRandomSeedRequest`: 1 `seed_length` uint32 (`08`).

`GenerateRandomSeedResponse`: 1 `seed_bytes` bytes (`0a`), 2 `seed_hex`
string (`12`, lower-case hex), 3 `seed_mnemonic` string (`1a`).

`DecodeSeedRequest`: 1 `user_input` string (`0a`).

`DecodeSeedResponse`: 1 `decoded_seed` bytes (`0a`).

#### AgendaService

`AgendasRequest {}` — empty.

`AgendasResponse` (`api.proto:906-924`): 1 `version` uint32 (`08`),
2 `agendas` repeated `Agenda` (`12`).

`AgendasResponse.Agenda`

| # | Name | Type | Tag |
|---|---|---|---|
| 1 | `id` | string | `0a` |
| 2 | `description` | string | `12` |
| 3 | `mask` | uint32 | `18` |
| 4 | `choices` | repeated `Choice` | `22` |
| 5 | `start_time` | int64 | `28` |
| 6 | `expire_time` | int64 | `30` |

`AgendasResponse.Choice`

| # | Name | Type | Tag |
|---|---|---|---|
| 1 | `id` | string | `0a` |
| 2 | `description` | string | `12` |
| 3 | `bits` | uint32 | `18` |
| 4 | `is_abstain` | bool | `20` |
| 5 | `is_no` | bool | `28` |

#### VotingService

`VoteChoicesRequest`: 1 `ticket_hash` bytes (`0a`).

`VoteChoicesResponse`: 1 `version` uint32 (`08`), 2 `choices` repeated
`Choice` (`12`), 3 `votebits` uint32 (`18`).

`VoteChoicesResponse.Choice`: 1 `agenda_id` string (`0a`),
2 `agenda_description` string (`12`), 3 `choice_id` string (`1a`),
4 `choice_description` string (`22`).

`SetVoteChoicesRequest`: 1 `choices` repeated `Choice` (`0a`),
2 `ticket_hash` bytes (`12`).

`SetVoteChoicesRequest.Choice`: 1 `agenda_id` string (`0a`), 2 `choice_id`
string (`12`).

`SetVoteChoicesResponse`: 1 `votebits` uint32 (`08`).

`TSpendPoliciesRequest`: 1 `hash` bytes (`0a`), 2 `ticket_hash` bytes (`12`).

`TSpendPoliciesResponse`: 1 `policies` repeated `Policy` (`0a`).

`TSpendPoliciesResponse.Policy`: 1 `hash` bytes (`0a`), 2 `policy` string
(`12`), 3 `ticket_hash` bytes (`1a`).

`SetTSpendPolicyRequest`: 1 `hash` bytes (`0a`), 2 `policy` string (`12`),
3 `ticket_hash` bytes (`1a`).

`SetTSpendPolicyResponse {}` — empty.

`TreasuryPoliciesRequest {}` — empty.

`TreasuryPoliciesResponse`: 1 `policies` repeated `Policy` (`0a`).

`TreasuryPoliciesResponse.Policy`: 1 `key` bytes (`0a`), 2 `policy` string
(`12`), 3 `ticket_hash` bytes (`1a`).

`SetTreasuryPolicyRequest`: 1 `key` bytes (`0a`), 2 `policy` string (`12`),
3 `ticket_hash` bytes (`1a`).

`SetTreasuryPolicyResponse {}` — empty.

All hashes in this service are 32 raw bytes in internal byte order (the
reverse of the hex string a block explorer shows).

#### MessageVerificationService

`VerifyMessageRequest`: 1 `address` string (`0a`), 2 `message` string
(`12`), 3 `signature` bytes (`1a`, 65-byte compact signature).

`VerifyMessageResponse`: 1 `valid` bool (`08`).

#### NetworkService

`GetRawBlockRequest` (`api.proto:1192-1195`): 1 `block_hash` bytes (`0a`),
2 `block_height` **sint32** (`10`, ZigZag).

`GetRawBlockResponse`: 1 `block` bytes (`0a`) — wire-serialized `MsgBlock`.

#### DecodeMessageService

`DecodeRawTransactionRequest`: 1 `serialized_transaction` bytes (`0a`).

`DecodeRawTransactionResponse`: 1 `transaction` `DecodedTransaction` (`0a`).

`DecodedTransaction` (`api.proto:1007-1058`)

| # | Name | Type | Tag |
|---|---|---|---|
| 1 | `transaction_hash` | bytes | `0a` |
| 2 | `version` | int32 | `10` |
| 3 | `lock_time` | uint32 | `18` |
| 4 | `expiry` | uint32 | `20` |
| 5 | `transaction_type` | `TransactionDetails.TransactionType` | `28` |
| 6 | `inputs` | repeated `Input` | `32` |
| 7 | `outputs` | repeated `Output` | `3a` |

`DecodedTransaction.Input`

| # | Name | Type | Tag |
|---|---|---|---|
| 1 | `previous_transaction_hash` | bytes | `0a` |
| 2 | `previous_transaction_index` | uint32 | `10` |
| 3 | `tree` | `TreeType` | `18` |
| 4 | `sequence` | uint32 | `20` |
| 5 | `amount_in` | int64 | `28` |
| 6 | `block_height` | uint32 | `30` |
| 7 | `block_index` | uint32 | `38` |
| 8 | `signature_script` | bytes | `42` |
| 9 | `signature_script_asm` | string | `4a` |

`DecodedTransaction.Output`

| # | Name | Type | Tag |
|---|---|---|---|
| 1 | `value` | int64 | `08` |
| 2 | `index` | uint32 | `10` |
| 3 | `version` | int32 | `18` |
| 4 | `script` | bytes | `22` |
| 5 | `script_asm` | string | `2a` |
| 6 | `required_signatures` | int32 | `30` |
| 7 | `script_class` | `ScriptClass` | `38` |
| 8 | `addresses` | repeated string | `42` |
| 9 | `commitment_amount` | int64 | `48` |

#### Unreferenced messages (kept for descriptor compatibility)

These are referenced by no RPC (3.3) but are part of the file descriptor a
reflection-free client compiles in; their definitions (`api.proto:760-788`):

`StartConsensusRpcRequest`: 1 `network_address` string (`0a`), 2 `username`
string (`12`), 3 `password` bytes (`1a`), 4 `certificate` bytes (`22`).
`StartConsensusRpcResponse {}`.

`DiscoverAddressesRequest`: 1 `discover_accounts` bool (`08`),
2 `private_passphrase` bytes (`12`), 3 `starting_block_hash` bytes (`1a`).
`DiscoverAddressesResponse {}`.

`FetchMissingCFiltersRequest {}`, `FetchMissingCFiltersResponse {}`,
`SubscribeToBlockNotificationsRequest {}`,
`SubscribeToBlockNotificationsResponse {}`, `FetchHeadersRequest {}` — empty.

`FetchHeadersResponse`: 1 `fetched_headers_count` uint32 (`08`),
2 `first_new_block_hash` bytes (`12`), 3 `first_new_block_height` int32
(`18`), 4 `main_chain_tip_block_hash` bytes (`22`),
5 `main_chain_tip_block_height` int32 (`28`).

Declaration order in `api.proto` is not grouped by service:
`DecodeMessageService` is declared at `:1003`, after the message definitions
of the other services, while the other eight services in scope are declared
at `:94-135`. Order of declaration determines the index of each service and
message in the serialized file descriptor embedded in the Go stubs; it has no
effect on the wire.

### 4.4 Enums

`SyncNotificationType` (`api.proto:790-806`)

| Value | Name | Sent by | Payload sub-message |
|---|---|---|---|
| 0 | `SYNCED` | both | none; `synced = true` |
| 1 | `UNSYNCED` | both | none; `synced = false` |
| 2 | `PEER_CONNECTED` | SpvSync only | `peer_information` |
| 3 | `PEER_DISCONNECTED` | SpvSync only | `peer_information` |
| 4 | `FETCHED_MISSING_CFILTERS_STARTED` | both | none |
| 5 | `FETCHED_MISSING_CFILTERS_PROGRESS` | both | `fetch_missing_cfilters` |
| 6 | `FETCHED_MISSING_CFILTERS_FINISHED` | both | none |
| 7 | `FETCHED_HEADERS_STARTED` | both | none |
| 8 | `FETCHED_HEADERS_PROGRESS` | both | `fetch_headers` |
| 9 | `FETCHED_HEADERS_FINISHED` | both | none |
| 10 | `DISCOVER_ADDRESSES_STARTED` | both | none |
| 11 | `DISCOVER_ADDRESSES_FINISHED` | both | none |
| 12 | `RESCAN_STARTED` | both | none |
| 13 | `RESCAN_PROGRESS` | both | `rescan_progress` |
| 14 | `RESCAN_FINISHED` | both | none |

`DecodedTransaction.Input.TreeType` (`api.proto:1011-1015`): `REGULAR = 0`,
`UNKNOWN = -1`, `STAKE = 1`. The handler casts the outpoint's `int8` tree
value directly (`server.go:3566`), so any other tree byte in the input is
passed through as that (unnamed) enum number.

`DecodedTransaction.Output.ScriptClass` (`api.proto:1031-1046`) and the
mapping from `stdscript.ScriptType` (`server.go:3664-3697`):

| Value | Name | `stdscript` types mapped to it |
|---|---|---|
| 0 | `NON_STANDARD` | `STNonStandard`, and any type not listed below |
| 1 | `PUB_KEY` | `STPubKeyEcdsaSecp256k1` |
| 2 | `PUB_KEY_HASH` | `STPubKeyHashEcdsaSecp256k1` |
| 3 | `SCRIPT_HASH` | `STScriptHash` |
| 4 | `MULTI_SIG` | `STMultiSig` |
| 5 | `NULL_DATA` | `STNullData` |
| 6 | `STAKE_SUBMISSION` | `STStakeSubmissionPubKeyHash`, `STStakeSubmissionScriptHash`; also forced for every odd-index output of a ticket (5.12) |
| 7 | `STAKE_GEN` | `STStakeGenPubKeyHash`, `STStakeGenScriptHash` |
| 8 | `STAKE_REVOCATION` | `STStakeRevocationPubKeyHash`, `STStakeRevocationScriptHash` |
| 9 | `STAKE_SUB_CHANGE` | `STStakeChangePubKeyHash`, `STStakeChangeScriptHash` |
| 10 | `PUB_KEY_ALT` | `STPubKeyEd25519`, `STPubKeySchnorrSecp256k1` |
| 11 | `PUB_KEY_HASH_ALT` | `STPubKeyHashEd25519`, `STPubKeyHashSchnorrSecp256k1` |
| 12 | `TGEN` | `STTreasuryGenPubKeyHash`, `STTreasuryGenScriptHash` |
| 13 | `TADD` | `STTreasuryAdd` |

`TransactionDetails.TransactionType` (`api.proto:157-163`, shared with the
sibling spec) and `marshalTxType` (`server.go:2245-2258`):

| Value | Name | `wallet.TransactionType` |
|---|---|---|
| 0 | `REGULAR` | `TransactionTypeRegular` and any other value |
| 4 | `COINBASE` | `TransactionTypeCoinbase` |
| 1 | `TICKET_PURCHASE` | `TransactionTypeTicketPurchase` |
| 2 | `VOTE` | `TransactionTypeVote` |
| 3 | `REVOCATION` | `TransactionTypeRevocation` |

### 4.5 Wire examples (payload bytes after the 5-byte gRPC frame header)

| Message | Bytes (hex) |
|---|---|
| `SpvSyncResponse` for `Synced(true)` | `08 01` (`notification_type` 0 is omitted) |
| `SpvSyncResponse` for `Synced(false)` | `10 01` |
| `FETCHED_HEADERS_STARTED` | `10 07` |
| `FETCHED_HEADERS_PROGRESS`, count 2000, time 1700000000 | `10 08 1a 09 08 d0 0f 10 80 e2 cf aa 06` |
| `FETCHED_MISSING_CFILTERS_PROGRESS`, start 0, end 0 | `10 05 22 00` (sub-message present, empty) |
| `RESCAN_PROGRESS`, through 0 | `10 0d 2a 00` |
| `PEER_CONNECTED`, count 3, address `1.2.3.4:9108` | `10 02 32 10 08 03 12 0c 31 2e 32 2e 33 2e 34 3a 39 31 30 38` |
| `WalletExistsResponse{exists: false}` | (zero bytes) |
| `DecodedTransaction.Input.tree = UNKNOWN (-1)` | `18 ff ff ff ff ff ff ff ff ff 01` |

### 4.6 Status encoding

Every failure is a gRPC status with a code and a UTF-8 message and no
details (`status.Errorf` / `status.Error` only; `grpc-status-details-bin` is
never set). Errors a handler returns that are not gRPC statuses are converted
by grpc-go: a wrapped `context.DeadlineExceeded` → `DeadlineExceeded`, a
wrapped `context.Canceled` → `Canceled`, anything else → `Unknown`, with
`err.Error()` as the message (`grpc@v1.83.2/server.go:1443-1450`,
`:1738-1744`, `status/status.go:151-162`). Section 8 lists every status.

Wallet error strings embedded in status messages are produced by
`errors.(*Error).Error()` with the process-wide separator `":: "`
(`dcrwallet.go:38`; the package default `":\n\t"` at `errors/errors.go:27`
is overridden at init). Format (`errors/errors.go:247-290`): for each nested
`*Error`, the `Op` if non-empty and different from the last printed `Op`,
then `": "` and the `Kind` string if non-zero and different from the last
printed `Kind`; between nested `*Error` levels the separator; before a
non-`*Error` leaf `": "` and the leaf's text. An `*Error` with nothing to
print renders as `unclassified error`.

### 4.7 Persisted data touched

This subsystem defines no database format of its own. It writes through
wallet APIs owned by other specs:

| Data | Written by | Format owner |
|---|---|---|
| `<appdata>/<netdir>/wallet.db` creation / open / close | `Loader` (`loader.go:134`, `:224`, `:299`, `:385`); directories created with mode `0700` (`:122`, `:153`, `:212`, `:243`) | spec 01 (bbolt), specs 02/03/05 (contents) |
| Birthday state: key `birthstate` (10 ASCII bytes; Go variable `rootBirthState`, `wallet/udb/txdb.go:163`) in the top-level `wtxmgr` bucket (`wallet/udb/migration.go:19`), 45 bytes: `[0]` options (bit 0 = set from height, bit 1 = set from time), `[1:33]` block hash, `[33:37]` height, `[37:45]` `Time.Unix()` as uint64; integers big-endian (`wallet/udb/txmined.go:341-363`; `byteOrder` at `wallet/udb/txdb.go:123`) | `CreateWallet` via `wallet.SetBirthState` | spec 03/05 |
| Default and per-ticket agenda preferences | `SetVoteChoices` via `udb.SetDefaultAgendaPreference` / `SetTicketAgendaPreference` | spec 05 |
| TSpend and treasury-key policies (wallet-wide and per-ticket) | `SetTSpendPolicy`, `SetTreasuryPolicy` via `udb.SetTSpendPolicy` / `SetVSPTSpendPolicy` / `SetTreasuryKeyPolicy` / `SetVSPTreasuryKeyPolicy` | spec 05 |
| `<dbDirPath>/peers.json` | SPV address manager created by `SpvSync` (`server.go:3015`, `dcrd/addrmgr/v3@v3.0.0/addrmanager.go:25`, `:1220`) | spec 14 |

`<netdir>` is `chainParams.Name`, except `testnet3` for testnet3 and
`testnet2` for net `0x48e7a065` (`walletsetup.go:34-44`).

The exact birthday bytes `CreateWallet` produces are given in 5.5.2.

### 4.8 Seed mnemonic format (PGP word list)

- Word table: the 512 lines of `pgpwordlist/wordlist.go:33-544`, in file
  order, are indexes 0..511. Line 33 carries the constant's opening
  (`` const alternatingWords = `aardvark ``) and line 544 its closing
  backtick (`` Yucatan` ``). The table must be copied verbatim, including
  capitalisation. Index 0 = `aardvark`, 1 = `adroitness`, 2 = `absurd`,
  3 = `adviser`, …, 510 = `Zulu`, 511 = `Yucatan`.
- Encoding of byte `b` at zero-based position `i`: word index
  `2*b + (i mod 2)`.
- A seed of `n` bytes encodes to `n + 1` words: the `n` seed bytes at
  positions `0..n-1`, then the checksum byte
  `SHA-256(SHA-256(seed))[0]` at position `n`. Words are separated by a
  single space; no leading or trailing space for `n >= 1`.
- Hex form: `2n` lower-case hexadecimal digits, no prefix.

Vectors (`walletseed/seed_test.go:14-45`): the 20-byte seed
`E58294F2E9A227486E8B061B31CC528FD7FA3F19` encodes to `topmost Istanbul
Pluto vagabond treadmill Pacific brackish dictator goldfish Medusa afflict
bravado chatter revolver Dupont midsummer stopwatch whimsical cowbell
bottomless fracture`; 31 zero bytes encode to `aardvark adroitness` repeated
15 times, then `aardvark insurgent`.

---

## 5. Algorithms and state machines

### 5.1 Registration and the readiness gate

`startRPCServers` (`rpcserver.go:320-346`), reached only when server TLS is
enabled, at least one gRPC listen address is configured and a client CA
exists:

1. Clone the TLS config; set `ClientAuth = RequireAndVerifyClientCert`.
2. Open the listeners (`makeListeners`; IPv4 addresses first, then IPv6). No
   listener → error `failed to create listeners for RPC server`.
3. `grpc.NewServer(Creds(TLS), StreamInterceptor(interceptStreaming),
   UnaryInterceptor(interceptUnary))`.
4. `rpcserver.RegisterServices(server)` registers, in this order:
   `VersionService`, `WalletService`, `WalletLoaderService`, `SeedService`,
   `AccountMixerService`, `TicketBuyerService`, `AgendaService`,
   `VotingService`, `MessageVerificationService`, `DecodeMessageService`,
   `NetworkService` (`server.go:274-286`).
5. `StartWalletLoaderService(walletLoader, activeNet)`,
   `StartAgendaService(activeNet.Params)`,
   `StartDecodeMessageService(activeNet.Params)`,
   `StartMessageVerificationService(activeNet.Params)`.
6. One goroutine per listener: emit the `grpclistener` pipe event, log
   `gRPC server listening on <addr>`, `server.Serve(lis)`.

Back in `run` (`dcrwallet.go:385-394`), if the gRPC server exists:
`loader.RunAfterLoad(func(w) { StartWalletService(w);
StartTicketBuyerService(w); StartAccountMixerService(w);
StartNetworkService(w); StartVotingService(w) })`. If a wallet is already
loaded (no `--noinitialload`) the callback runs immediately; otherwise it runs
inside the first successful `CreateWallet`, `CreateWatchingOnlyWallet` or
`OpenWallet`, on that RPC's goroutine, with the loader mutex held, before the
RPC returns.

Readiness (`ServiceReady`, `server.go:303-320`): looked up by fully-qualified
name in `serviceMap`:

| Key | Gated |
|---|---|
| `walletrpc.VersionService` | no |
| `walletrpc.WalletService` | yes |
| `walletrpc.WalletLoaderService` | no |
| `walletrpc.SeedService` | no |
| `walletrpc.AccountMixerService` | yes |
| `walletrpc.TicketBuyerService` | yes |
| `walletrpc.AgendaService` | no |
| `walletrpc.VotingService` | yes |
| `walletrpc.MessageVerificationService` | no |
| `walletrpc.DecodeMessageService` | no |
| `walletrpc.NetworkService` | yes |

A service is gated iff its singleton has a `checkReady() bool` method. A
gated service that is not ready fails every call with `FailedPrecondition`
and the message `service walletrpc.<Name> is not ready`. The flag goes from
false to true exactly once and is never reset; `CloseWallet` does not unset
it (10.Q4).

A request for a service or method that is not registered never reaches the
interceptors; grpc-go answers `Unimplemented` with `unknown service <svc>`
or `unknown method <m> for service <svc>`
(`grpc@v1.83.2/server.go:1866-1875`). The `service <name> not found` branch
of `ServiceReady` is therefore unreachable.

### 5.2 Interceptors (`rpcserver.go:406-447`)

Unary:

1. If the peer address is available from the context: log at DBG, subsystem
   `GRPC`: `Unary method <full method> invoked by <peer addr>`.
2. `err = ServiceReady(serviceName(fullMethod))`; non-nil → return it. This
   failure is **not** logged.
3. Call the handler.
4. If the handler failed and the peer address was available: log at ERR
   `Unary method <full method> invoked by <peer addr> errored: <err>`.
5. Return the handler's response and error unchanged.

Streaming: the same, with `Streaming method …` texts, except step 4 logs at
DBG instead of ERR when `status.Code(err) == codes.Canceled` and the stream
context is done.

`<err>` in the log is the Go error's `Error()`; for a gRPC status error this
is `rpc error: code = <Code> desc = <message>`.

### 5.3 Error translation (`server.go:80-136`)

`translateError(err)` = `status.Errorf(errorCode(err), "%s", err.Error())`.

`errorCode(err)`, first match wins:

1. If a wallet `errors.Kind` can be extracted from the chain (`errors.As`;
   the outermost `*Error` with a non-zero kind wins, and `errors.E` promotes
   an inner kind outward when the outer has none, `errors/errors.go:187-236`,
   `:311-323`):

   | Kind | Code |
   |---|---|
   | `Invalid` | `InvalidArgument` |
   | `Permission` | `PermissionDenied` |
   | `Exist` | `AlreadyExists` |
   | `NotExist` | `NotFound` |
   | `Crypto` | `DataLoss` |
   | `Locked` | `FailedPrecondition` |
   | `Passphrase` | `InvalidArgument` |
   | `Seed` | `InvalidArgument` |
   | `WatchingOnly` | `Unimplemented` |
   | `InsufficientBalance` | `ResourceExhausted` |
   | `NoPeers` | `Unavailable` |
   | `Bug`, `IO`, `Encoding`, `ScriptFailure`, `Policy`, `DoubleSpend`, `Protocol` | fall through to step 2 (Go `switch` cases do not fall into the next case, so the empty cases at `:89`, `:94`, `:99`, `:112-115` select nothing) |
   | `Consensus`, `Deployment` | not listed; fall through to step 2 |

2. `errors.Is(err, hdkeychain.ErrInvalidSeedLen)` → `InvalidArgument`.
3. `errors.Is(err, context.Canceled)` → `Canceled`.
4. If the chain contains an error with a `GRPCStatus()` method whose code is
   neither `OK` nor `Unknown` → that code.
5. `Unknown`.

The message is always the full wallet error string (4.6), including
operation names such as `wallet.Unlock`.

### 5.4 Loader operations

All four operations take `l.mu` for their whole duration (acquired after the
`defer l.mu.Unlock()` statement is registered), except that
`OpenExistingWallet` releases it around the database open.

#### 5.4.1 `CreateNewWallet(ctx, pubPassphrase, privPassphrase, seed)` (`loader.go:198-282`)

Op string `loader.CreateNewWallet`. Every error below is
`errors.E(op, …)` wrapping the cause unless a kind is shown.

1. If a wallet is loaded → kind `Exist`, text `wallet already opened`.
2. `os.Stat(dbDirPath)`:
   - not exist → `os.MkdirAll(dbDirPath, 0700)`; failure → error;
   - other stat error → error;
   - exists but is not a directory → `"<dbDirPath>" is not a directory`
     (Go `%q` quoting; **no kind**, so it maps to `Unknown`).
3. `dbPath = <dbDirPath>/wallet.db`. Stat it: error → error; exists → kind
   `Exist`, text `wallet DB exists`.
4. From here, if the function returns an error, `os.Remove(dbPath)` is
   attempted (result ignored). The database handle is not closed first
   (10.Q8).
5. `os.MkdirAll(dbDirPath, 0700)` again.
6. `wallet.CreateDB("bdb", dbPath)`.
7. `wallet.Create(ctx, db, pubPassphrase, privPassphrase, seed,
   chainParams)`. Relevant failures: seed length outside `[16, 64]` →
   `hdkeychain.ErrInvalidSeedLen` (no kind; `wallet/wallet.go:5304-5306`);
   a **nil** seed makes `wallet.Create` generate a random 32-byte seed
   instead (`wallet/wallet.go:5297-5303`), which is what the loader's doc
   comment "The seed is optional" refers to; this branch is unreachable
   through gRPC because the handler rejects an empty seed first (5.5.2
   step 2);
   empty private passphrase → kind `Invalid`, `private passphrase may not be
   empty` (`wallet/udb/addressmanager.go:2903-2904`).
8. `wallet.Open(ctx, cfg)` with `cfg = {DB, PubPassphrase, VotingEnabled,
   GapLimit, WatchLast, AccountGapLimit, DisableCoinTypeUpgrades,
   MixingEnabled, ManualTickets, AllowHighFees, RelayFee, VSPMaxFee, Params,
   Dialer}`. **`MixSplitLimit` is not set** (it is 0) in this path only
   (`loader.go:259-274`; compare `:182`, `:331`) (10.Q7).
9. `onLoaded(w, db)`; return `w`.

#### 5.4.2 `CreateWatchingOnlyWallet(ctx, extendedPubKey, pubPass)` (`loader.go:108-193`)

Op `loader.CreateWatchingOnlyWallet`. Same steps as 5.4.1 with these
differences:

- step 1 text: `wallet already loaded` (kind `Exist`);
- step 2 not-a-directory error carries kind `Invalid`;
- step 3 text: `wallet already exists` (kind `Exist`);
- step 7 is `wallet.CreateWatchOnly(ctx, db, extendedPubKey, pubPass,
  chainParams)`;
- step 8 config includes `MixSplitLimit`.

#### 5.4.3 `OpenExistingWallet(ctx, pubPassphrase)` (`loader.go:288-342`)

Op `loader.OpenExistingWallet`.

1. If a wallet is loaded → kind `Exist`, text `wallet already opened`.
2. **Release `l.mu`**, call `wallet.OpenDB("bdb", <dbDirPath>/wallet.db)`,
   re-acquire `l.mu`. The bbolt open takes an exclusive file lock with no
   timeout (`wallet/internal/bdb/db.go:321`, options `nil`), so a second
   concurrent open blocks here, without the mutex, until the first handle is
   closed (10.Q9). A missing file fails fast with kind `NotExist`, text
   `missing database file` (`db.go:317-319`).
3. On error: log at ERR (subsystem `LODR`) `Failed to open database: <err>`
   and return `errors.E(op, err)`.
4. From here, if the function returns an error the database is closed.
5. `wallet.Open(ctx, cfg)` with the full config including `MixSplitLimit`.
   A wrong public passphrase surfaces as kind `Passphrase`. Database
   upgrades run inside `wallet.Open` (spec 05).
6. `onLoaded(w, db)`; return `w`. The "already loaded" condition is not
   re-checked after the mutex was re-acquired.

#### 5.4.4 `UnloadWallet()` (`loader.go:375-393`)

1. No wallet loaded → `errors.E("loader.UnloadWallet", errors.Invalid,
   "wallet is unopened")`.
2. `l.db.Close()`; error → `errors.E(op, err)` and the loader keeps its
   wallet and db references.
3. Set `l.wallet = nil`, `l.db = nil`.

Nothing else is stopped: no syncer is cancelled, the wallet is not locked,
its network backend is not cleared, running ticket buyers and mixers keep
running against a closed database. Closing a bbolt database waits for open
transactions to finish.

#### 5.4.5 Callbacks

`onLoaded` runs the registered callbacks synchronously, in order, then
discards them. A second load in the same process (after `UnloadWallet`)
therefore runs no callbacks; `RunAfterLoad` called while a wallet is loaded
invokes the function immediately and does not store it.

### 5.5 `WalletLoaderService`

#### 5.5.1 `WalletExists` (`server.go:2793-2801`)

1. `exists, err = loader.WalletExists()`.
2. Error → `translateError(err)` (a stat failure other than not-exist; kind
   none → `Unknown`).
3. Respond `{exists}`.

Works whether or not a wallet is loaded; reports only that a file named
`wallet.db` exists.

#### 5.5.2 `CreateWallet` (`server.go:2694-2755`)

On every return path `private_passphrase` and `seed` buffers are zeroed
(deferred).

1. `pubPassphrase = req.public_passphrase`; if empty, the 6 bytes `public`
   (`wallet.InsecurePubPassphrase`, `wallet/wallet.go:66`).
2. `len(seed) == 0` → `InvalidArgument`, `seed is a required parameter`.
3. `set_birth_time && set_birth_height` → `InvalidArgument`,
   `both set birth time and height cannot be set`.
4. If `set_birth_time`: `birthday = time.Unix(birth_time, 0)`; if
   `now - birthday < 24h` → `InvalidArgument`, `birth time cannot be in the
   future or too close (one day) to the present`. Otherwise
   `birthState = {SetFromTime: true, Time: birthday}`.
5. **If `set_birth_time`** (the condition is a copy of step 4's; the source
   evidently meant `set_birth_height`, `server.go:2730`):
   `birthState = {SetFromHeight: true, Height: birth_height}`, replacing the
   value from step 4.
6. If `birthState` is still nil: `birthState = {SetFromHeight: true}`
   (height 0, the genesis block).
7. `w, err = loader.CreateNewWallet(ctx, pubPassphrase, private_passphrase,
   seed)`; error → `translateError(err)`.
8. `w.SetBirthState(ctx, birthState)` (one read-write database transaction);
   error → `Internal`, `unable to set birthday state: <err>`. The wallet
   stays created and loaded in that case.
9. Respond with the empty message.

Net effect of steps 4-6 (10.Q1):

| Request | Validation | Birthday state written |
|---|---|---|
| neither flag | none | from height, height 0 |
| `set_birth_height` only | none | from height, height **0** (`birth_height` ignored) |
| `set_birth_time` only | `birth_time` at least 24 h in the past | from height, height = `birth_height` (normally 0); `birth_time` discarded |
| both | rejected | — |

The 45-byte value written (4.7) is therefore always: `01`, 32 zero bytes,
`birth_height` big-endian (or `00 00 00 00`), then
`ff ff ff f1 88 6e 09 00` (the Unix time of Go's zero `time.Time`,
-62135596800, as a big-endian uint64).

Passphrase and seed requirements enforced below the handler: seed length
16..64 bytes (else `InvalidArgument` via step 2 of 5.3, since
`ErrInvalidSeedLen` has no kind); private passphrase non-empty (else
`InvalidArgument`).

#### 5.5.3 `CreateWatchingOnlyWallet` (`server.go:2757-2772`)

1. Default the public passphrase as in 5.5.2 step 1.
2. `loader.CreateWatchingOnlyWallet(ctx, extended_pub_key, pubPassphrase)`;
   error → `translateError(err)`.
3. Respond with the empty message. No birthday state is written.

#### 5.5.4 `OpenWallet` (`server.go:2774-2791`)

1. Default the public passphrase as in 5.5.2 step 1.
2. `w, err = loader.OpenExistingWallet(ctx, pubPassphrase)`; error →
   `translateError(err)`.
3. Respond `{watching_only: w.WatchingOnly()}`.

The request context is passed to `wallet.Open`. UNVERIFIED that cancelling
it aborts a long database upgrade: `walletdb.Update` uses the context only
for trace regions and does not test it for cancellation
(`wallet/walletdb/interface.go:230-255`), and `udb.Upgrade` runs all
upgrades inside one such transaction (`wallet/udb/upgrades.go:1717-1719`).
Settle by tracing every use of `ctx` below `wallet.Open`.

#### 5.5.5 `CloseWallet` (`server.go:2803-2815`)

1. `err = loader.UnloadWallet()`.
2. `errors.Is(err, errors.Invalid)` → `FailedPrecondition`, `Wallet is not
   loaded`. (A database close error of kind `Invalid` is reported the same
   way.)
3. Other error → `translateError(err)`.
4. Respond with the empty message.

#### 5.5.6 `RescanPoint` (`server.go:3160-3175`)

1. `loader.LoadedWallet()`; none → `FailedPrecondition`, `Wallet has not
   been loaded`.
2. `wallet.RescanPoint(ctx)` (one read transaction; spec 09). Error →
   `FailedPrecondition`, `Rescan point failed to be requested <err>` (no
   colon before the error).
3. Non-nil → `{rescan_point_hash: <32 bytes>}`; nil → empty message.

#### 5.5.7 `RpcSync` (`server.go:2832-2982`)

`password` is zeroed on every return path (deferred first).

1. `loader.LoadedWallet()`; none → `FailedPrecondition`, `Wallet has not
   been loaded`.
2. `wallet.NetworkBackend()` succeeds (a backend is set, including the
   offline backend) → `FailedPrecondition`, `wallet is loaded and already
   synchronizing`.
3. `discover_accounts && len(private_passphrase) == 0` → `InvalidArgument`,
   `private passphrase is required for discovering accounts`.
4. If `discover_accounts`:
   1. create `lock`, a channel of capacity 1;
   2. define `lockWallet` = send one value on `lock`, then zero
      `private_passphrase`;
   3. register `lockWallet` to run when the handler returns (the deferred
      call captures the function value now, so the later "set to nil" in the
      `DiscoverAddressesFinished` callback of step 6 does not cancel it);
   4. `wallet.Unlock(streamCtx, private_passphrase, lock)`; error →
      `translateError(err)` (wrong passphrase: `InvalidArgument`;
      watching-only: `Unimplemented`).
5. `syncer = chain.NewSyncer(wallet, &RPCOptions{Address: network_address,
   DefaultPort: activeNet.JSONRPCClientPort, User: username,
   Pass: string(password), CA: certificate,
   Insecure: isLoopback(network_address) && len(certificate) == 0})`.
   `Dial`, `ClientCert`, `ClientKey` are unset: no proxy and no client
   certificate authentication to dcrd over this path. Default ports:
   mainnet `9109`, testnet3 `19109`, simnet `19556`.

   `isLoopback(addr)`: split host and port (`net.SplitHostPort`); if that
   succeeds use the host, else the whole string; return true if it equals
   `localhost` exactly (case-sensitive); else parse as an IP and return
   whether it is a loopback address (`127.0.0.0/8`, `::1`); unparsable →
   false. A bracketed IPv6 literal without a port (`[::1]`) is therefore not
   loopback.

   `NewSyncer` samples `!wallet.Locked()` once to decide whether account
   discovery runs (`chain/sync.go:82`), so discovery happens whenever the
   wallet is unlocked at this moment, whether or not `discover_accounts` was
   set (10.Q12).
6. Install callbacks; each builds one `RpcSyncResponse` and calls
   `svr.Send`, **ignoring the send error**:

   | Callback | Response |
   |---|---|
   | `Synced(sync)` | `synced = sync`; `notification_type = SYNCED` if true else `UNSYNCED` |
   | `FetchMissingCFiltersStarted` | type 4 |
   | `FetchMissingCFiltersProgress(start, end)` | type 5, `fetch_missing_cfilters = {start, end}` |
   | `FetchMissingCFiltersFinished` | type 6 |
   | `FetchHeadersStarted` | type 7 |
   | `FetchHeadersProgress(n, t)` | type 8, `fetch_headers = {fetched_headers_count: n, last_header_time: t}` |
   | `FetchHeadersFinished` | type 9 |
   | `DiscoverAddressesStarted` | type 10 |
   | `DiscoverAddressesFinished` | type 11; before sending, if `lockWallet` is non-nil call it and set it to nil |
   | `RescanStarted` | type 12 |
   | `RescanProgress(h)` | type 13, `rescan_progress = {rescanned_through: h}` |
   | `RescanFinished` | type 14 |

   `synced` is false in every response except `Synced(true)`.
7. `err = syncer.Run(streamCtx)`; blocks until the syncer stops.
8. `err != nil`: if the stream context is done → `Canceled`, `Wallet
   synchronization canceled: <err>`; else → `Unknown`, `Wallet
   synchronization stopped: <err>`. `<err>` is prefixed by the syncer with
   op `rpcsyncer.Run` (`chain/sync.go:517-522`). `translateError` is not
   used, so an unreachable dcrd, a bad address, bad credentials and a network
   mismatch are all `Unknown`.
9. `err == nil` → the stream ends with status `OK`.

Values and order of notifications are produced by the syncer (spec 15). As
implemented (`chain/sync.go:657-666`, `:229-395`, `:674`): missing-cfilters
started / progress* / finished; fetch-headers started / progress* /
finished, where progress carries the **number of headers newly added** in
that batch and the last header's timestamp in **Unix seconds**; then, only
if a rescan point exists, discover-addresses started / finished and rescan
started / progress* / finished; then `SYNCED`. `UNSYNCED` is sent when the
syncer exits after having been synced. There is no retry loop: one
connection attempt, one run.

Wallet locking with `discover_accounts` (see `wallet.Unlock`,
`wallet/wallet.go:1712-1839`): if the wallet was locked, the unlock installs
a timeout goroutine that locks the wallet when a value arrives on `lock`.
`lockWallet` is called at `DiscoverAddressesFinished` (first occurrence
only) and again when the handler returns; the second send lands in the
channel buffer and is never read. If the wallet was already unlocked without
a timeout before the RPC, the `lock` channel is merely drained and the
wallet is not locked. If no rescan is needed, `DiscoverAddressesFinished`
never fires and the wallet stays unlocked until the RPC ends.

#### 5.5.8 `SpvSync` (`server.go:2984-3158`)

Steps 1 to 4 are identical to `RpcSync` steps 1 to 4 (same codes and
messages). Then:

5. `addr = [::1]:0` (TCP); `amgr = addrmgr.New(loader.DbDirPath())`;
   `lp = p2p.NewLocalPeer(wallet.ChainParams(), addr, amgr)`. No dial
   function is set (no proxy) and transaction relay is not disabled; compare
   the main-package loop, which applies `--proxy` and `--spvdisablerelaytx`
   (`dcrwallet.go:536-538`) (10.Q13).
6. Build the notification set: the twelve callbacks of 5.5.7 step 6 with
   `SpvSyncResponse`, plus:

   | Callback | Response |
   |---|---|
   | `PeerConnected(count, addr)` | type 2, `peer_information = {peer_count: count, address: addr}` |
   | `PeerDisconnected(count, addr)` | type 3, same payload |

   `MempoolTxs` and `TipChanged` are not set.
7. `syncer = spv.NewSyncer(wallet, lp)`; `syncer.SetNotifications(ntfns)`.
   As with the RPC syncer, account discovery is decided by `!wallet.Locked()`
   at construction (`spv/sync.go:144`).
8. If `spv_connect` is non-empty: normalize each entry in order with
   `cfgutil.NormalizeAddress(entry, activeNet.Params.DefaultPort)`; the first
   failure → `FailedPrecondition`, `SPV Connect address invalid: <err>`
   (the wallet has already been unlocked by step 4; if it was locked
   before the RPC, the deferred `lockWallet` then re-locks it). On success
   `syncer.SetPersistentPeers(list)`. Duplicates are not removed.

   `NormalizeAddress(addr, port)` (`internal/cfgutil/normalization.go:12-27`):
   if `net.SplitHostPort(addr)` succeeds return `JoinHostPort(host, port)`;
   otherwise `JoinHostPort(addr, defaultPort)` and, if that splits, return
   it; otherwise return the **original** split error. Neither host nor port
   content is validated.
9. `err = syncer.Run(streamCtx)`.
10. `err != nil`: wraps `context.Canceled` → `Canceled`, `SPV
    synchronization canceled: <err>`; wraps `context.DeadlineExceeded` →
    `DeadlineExceeded`, `SPV synchronization deadline exceeded: <err>`;
    otherwise `translateError(err)`.
11. `err == nil` → status `OK`.

Notification values come from the SPV syncer (spec 14). As implemented
(`spv/sync.go:417-434`, `:554-566`, `:1833`, `:1865-1906`): peer
notifications at any time, with `peer_count` = number of connected remotes
after the change and `address` = the remote address string; `UNSYNCED` when
the last peer disconnects after the wallet was synced; fetch-headers
progress carries the **height of the last header** (not a count) and its
timestamp in Unix seconds; when no rescan point exists the sequence after
fetch-headers finished is just `SYNCED`.

#### 5.5.9 Network backend lifecycle as seen by `RpcSync` / `SpvSync`

The "already synchronizing" precondition (step 2 of 5.5.7 and 5.5.8) reads
`wallet.NetworkBackend()` (`wallet/network.go:61-71`), which is set and
cleared by the syncers, not by the handlers:

- `chain.Syncer.Run` sets the backend only **after** it has normalized the
  address, dialled dcrd, checked `getcurrentnet` and checked the JSON-RPC API
  version (`chain/sync.go:536-611`), and clears it with a deferred
  `SetNetworkBackend(nil)` when `Run` returns.
- `spv.Syncer.Run` sets it after reading the tip and rescan point, starting
  the address manager, DNS seeding and launching its handler goroutines
  (`spv/sync.go:334-397`), and clears it the same way.

Consequences a client can observe:

1. After an `RpcSync` or `SpvSync` stream ends for any reason, the backend is
   nil again and a new `RpcSync` or `SpvSync` is accepted. This is the only
   retry mechanism: the client re-issues the RPC.
2. The check is not atomic with the set. Two sync RPCs issued close together
   both pass step 2 (for `RpcSync` the window lasts for the whole connection
   set-up, including a slow or hanging dial). Both syncers then run against
   the same wallet; the later `SetNetworkBackend` overwrites the earlier, and
   whichever `Run` returns first clears the backend although the other is
   still running, after which a third sync RPC is accepted as well
   (10.Q37).
3. When the wallet was started **without** `--noinitialload`, the
   main-package loop owns synchronization. It builds a new syncer per
   attempt and sleeps 5 s between failed attempts (`dcrwallet.go:531-600`),
   so the backend is nil while dcrd is unreachable or between SPV attempts,
   and a gRPC `RpcSync` / `SpvSync` issued in such a window is accepted and
   runs concurrently with the main loop's next attempt. While the main loop
   is connected, the RPCs fail with `wallet is loaded and already
   synchronizing`. With `--offline` the offline backend is set permanently
   and both RPCs always fail that way.
4. `RpcSync` failures that occur before the backend is set, all reported as
   `Unknown` `Wallet synchronization stopped: rpcsyncer.Run: …`
   (`chain/sync.go:536-606`): address normalization failure (wallet kind
   `Invalid`, still `Unknown` because `translateError` is not used), the
   `wsrpc.Dial` error (for example connection refused, a TLS failure, or a
   WebSocket handshake rejected for bad credentials), a failed `getcurrentnet` or `version` call,
   `mismatched networks`, and `advertised API version <have> incompatible
   with required version <want>` (spec 15 owns the exact texts).
5. `SpvSync` constructs the address manager and local peer (step 5) before
   it validates `spv_connect` (step 8). `addrmgr.New` and `p2p.NewLocalPeer`
   perform no I/O (`dcrd/addrmgr/v3@v3.0.0/addrmanager.go:1218-1227`);
   the address manager is started and stopped (which is where `peers.json`
   is loaded and saved, spec 14) only inside `syncer.Run`
   (`spv/sync.go:360-366`), so a request rejected
   for a bad `spv_connect` entry touches no file.

### 5.6 `AccountMixerService.RunAccountMixer` (`server.go:2531-2566`)

Gated by readiness (5.1).

1. `tb = ticketbuyer.New(wallet, Config{Mixing: true,
   MixedAccountBranch: req.mixed_account_branch,
   MixedAccount: req.mixed_account, ChangeAccount: req.change_account,
   BuyTickets: false, MixChange: true})`. No field of the request is
   validated: accounts need not exist and the branch may be any value.
2. If `len(passphrase) > 0`:
   1. `lock` = channel of capacity 1; `lockWallet` = send on `lock`, zero
      `passphrase`;
   2. `wallet.Unlock(streamCtx, passphrase, lock)`; error →
      `translateError(err)`;
   3. register `lockWallet` to run on return.
3. `err = tb.Run(streamCtx, passphrase)` (spec 11 section 5.13; spec 13 for
   `MixAccount`). With `BuyTickets` false, each main-chain tip notification
   with at least one attached block runs only `wallet.MixAccount(ctx,
   change_account, mixed_account, mixed_account_branch)` in a goroutine;
   mixing errors are logged, not returned. The buyer's own gating still
   applies before the mix is started (`ticketbuyer/tb.go:93-140`): the
   notification is skipped while a rescan point exists, when the tip header
   cannot be read, and when `height + 2` equals the start of the next
   stake-difficulty window.
4. `err != nil`: stream context done → `Canceled`, `AccountMixer instance
   canceled, account number: <mixed_account>`; otherwise → `Unknown`,
   `AccountMixer instance errored: <err>`.
5. `err == nil` → `OK` (not reachable: `Run` only returns on context
   cancellation or a fatal passphrase error).

No `RunAccountMixerResponse` is ever sent. Because grpc-go sends response
headers lazily, the client receives nothing at all (not even headers) until
the RPC ends; the terminal status then arrives as a trailers-only response.

`tb.Run` itself calls `wallet.Unlock(ctx, passphrase, nil)` when the
passphrase is non-empty (`ticketbuyer/tb.go:64-69`). That second unlock
replaces the timeout installed in step 2 with "no timeout"; the earlier
timeout goroutine is cancelled and only drains `lock`. Consequently the
deferred `lockWallet` does **not** lock the wallet: after the RPC ends the
wallet remains unlocked without a time limit (10.Q5). With an empty
passphrase the wallet's lock state is untouched; whether mixing then
succeeds depends on the wallet already being unlocked (spec 13).

### 5.7 `TicketBuyerService.RunTicketBuyer` (`server.go:2588-2686`)

Gated by readiness. Validation order is exactly:

1. If `vsp_host != "" || vsp_pubkey != ""`:
   `vspClient, err = wallet.VSP(VSPClientConfig{URL: vsp_host,
   PubKey: vsp_pubkey, Policy: {MaxFee: wallet.VSPMaxFee(),
   FeeAcct: account, ChangeAcct: account}})`. Error → `InvalidArgument`,
   `VSP client failed to start. Error: <err>`. Causes
   (`wallet/vsp.go:58-74`): `vsp url can not be null`, `vsp pubkey can not be
   null`, a URL parse error, a base64 (standard alphabet, padded) decode
   error of the public key. `wallet.VSP` caches clients by URL string
   (`wallet/vspclientloader.go:12-26`): if a client for `vsp_host` already
   exists it is returned unchanged and the supplied public key and policy
   are ignored.
2. `balance_to_maintain < 0` → `InvalidArgument`, `Negative balance to
   maintain given`.
3. If `enable_mixing`, in order:
   1. `wallet.AccountName(ctx, mixed_account)` fails → `InvalidArgument`,
      `Mixing requested, but error on mixed account: <err>`;
   2. `mixed_account_branch` not 0 or 1 → `InvalidArgument`,
      `MixedAccountBranch should be 0 or 1.`;
   3. `wallet.AccountName(ctx, mixed_split_account)` fails →
      `InvalidArgument`, `Mixing requested, but error on mixedSplitAccount:
      <err>`;
   4. `wallet.AccountName(ctx, change_account)` fails → `InvalidArgument`,
      `Mixing requested, but error on changeAccount: <err>`.

   `account` and `voting_account` are never validated.
4. `tb = ticketbuyer.New(wallet, Config{BuyTickets: true, Account: account,
   VotingAccount: voting_account, Maintain: balance_to_maintain,
   VSP: vspClient, Mixing: enable_mixing, MixedAccount, MixChange,
   ChangeAccount: change_account, MixedAccountBranch, TicketSplitAccount,
   Limit: int(limit)})`, where, when `enable_mixing` is false,
   `MixedAccount = MixedAccountBranch = TicketSplitAccount = 0` and
   `MixChange = false` regardless of the request, and `ChangeAccount` is
   still the request's `change_account`. When true, `MixChange = true` and
   the three values come from the request.
5. Passphrase handling identical to 5.6 step 2 (same quirk).
6. `err = tb.Run(streamCtx, passphrase)`.
7. `err != nil`: stream context done → `Canceled`, `TicketBuyer instance
   canceled, account number: <account>`; otherwise → `Unknown`,
   `TicketBuyer instance errored: <err>` (reached when a purchase attempt
   finds the passphrase no longer valid, `ticketbuyer/tb.go:190-195`, or
   when the initial unlock inside `Run` fails).

No `RunTicketBuyerResponse` is ever sent (same header behaviour as 5.6).
Multiple concurrent `RunTicketBuyer` / `RunAccountMixer` streams are
allowed; nothing prevents two buyers on the same account.

### 5.8 `SeedService`

#### 5.8.1 `GenerateRandomSeed` (`server.go:3177-3197`)

1. `size = seed_length`; if 0, 32 (`hdkeychain.RecommendedSeedLen`).
2. `size < 16 || size > 64` → `InvalidArgument`, `invalid seed length`.
3. Fill `size` bytes from the CSPRNG (`dcrd/crypto/rand.Read`).
4. Respond `{seed_bytes: seed, seed_hex: lower-case hex,
   seed_mnemonic: walletseed.EncodeMnemonic(seed)}` (4.8).

The seed is not stored or zeroed by the server.

#### 5.8.2 `DecodeSeed` (`server.go:3199-3205`, `walletseed/seed.go:71-102`)

`seed, err = walletseed.DecodeUserInput(user_input)`; error →
`InvalidArgument` with message `err.Error()`; else `{decoded_seed: seed}`.

`DecodeUserInput(input)`, op `walletseed.DecodeUserInput`, every error has
kind `Encoding`:

1. `words = strings.Split(strings.TrimSpace(input), " ")` — trim Unicode
   white space at both ends, then split on single U+0020 only. An empty or
   all-space input yields one empty word.
2. One word → hexadecimal: decode with `encoding/hex` (upper or lower case
   digits). Failure → the hex error (`encoding/hex: invalid byte: U+XXXX
   'c'` or `encoding/hex: odd length hex string`).
3. More than one word → mnemonic:
   1. `decoded, err = pgpwordlist.DecodeMnemonics(words)` (step 4); error →
      wrapped.
   2. Fewer than 2 decoded bytes → no seed (falls to step 5 with length 0).
   3. `SHA-256(SHA-256(decoded[:n-1]))[0] != decoded[n-1]` → `checksum
      mismatch`.
   4. `seed = decoded[:n-1]`.
4. `DecodeMnemonics(words)` (op `pgpwordlist.DecodeMnemonics`, kind
   `Encoding`): for each element in order: trim Unicode white space; skip if
   empty (so runs of spaces between words are tolerated); look up the
   lower-cased word (Unicode lower-casing); not found → `word <w> is not in
   the PGP word list` (with `<w>` as trimmed, original case); found at list
   index `k` but `k mod 2 != idx mod 2`, where `idx` counts accepted words
   from 0 → `word <w> is not valid at position <idx>, check for missing
   words`; else append byte `k / 2`.
5. `len(seed) < 16 || len(seed) > 64` → `hdkeychain.ErrInvalidSeedLen`,
   text `seed length must be between 128 and 512 bits`.

Resulting status messages (derived from 4.6; pin with the oracle):

| Condition | Message |
|---|---|
| bad hex digit `g` | `walletseed.DecodeUserInput: invalid encoding: encoding/hex: invalid byte: U+0067 'g'` |
| odd hex length | `walletseed.DecodeUserInput: invalid encoding: encoding/hex: odd length hex string` |
| unknown word `foo` | `walletseed.DecodeUserInput: invalid encoding:: pgpwordlist.DecodeMnemonics: word foo is not in the PGP word list` |
| misplaced word | `walletseed.DecodeUserInput: invalid encoding:: pgpwordlist.DecodeMnemonics: word <w> is not valid at position <idx>, check for missing words` |
| checksum | `walletseed.DecodeUserInput: invalid encoding: checksum mismatch` |
| length (including empty input) | `walletseed.DecodeUserInput: invalid encoding: seed length must be between 128 and 512 bits` |

Words separated by tabs or newlines are not split and fail as unknown words.

### 5.9 `AgendaService.Agendas` (`server.go:3211-3239`)

1. `version, deployments = wallet.CurrentAgendas(activeNet)`
   (`wallet/wallet.go:291-314`): `version` is 11 for mainnet, 12 for
   testnet3, 12 for simnet, 6 for net `0x48e7a065`, 1 otherwise;
   `deployments = params.Deployments[version]` (absent → none).
2. Respond `version` and, for each deployment in slice order, an `Agenda`:
   `id = Vote.Id`, `description = Vote.Description`, `mask = Vote.Mask`
   (16-bit widened), `start_time = StartTime`, `expire_time = ExpireTime`
   (uint64 reinterpreted as int64), and for each choice in slice order
   `{id, description, bits (16-bit widened), is_abstain, is_no}`.

Never fails. Does not use the wallet. `ForcedChoiceID` is not exposed. At
the pin, mainnet version 11 and testnet3/simnet version 12 each contain one
agenda, `maxtreasuryspend` ("Change maximum treasury expenditure policy as
defined in DCP0013", mask `0x0006`, choices `abstain` 0x0000, `no` 0x0002,
`yes` 0x0004; mainnet start 1762992000, expire 1826064000;
`dcrd/chaincfg/v3@v3.3.0/mainnetparams.go:486-514`).

### 5.10 `VotingService`

Gated by readiness. All six handlers use the wallet captured when the service
was started.

#### 5.10.1 `VoteChoices` (`server.go:3260-3300`)

1. If `ticket_hash` is non-empty: `chainhash.NewHash`; wrong length →
   `InvalidArgument`, `invalid hash length of <n>, want 32`.
2. `version, agendas = wallet.CurrentAgendas(wallet.ChainParams())`.
3. `choices, voteBits, err = wallet.AgendaChoices(ctx, ticketHash)`
   (`wallet/wallet.go:412-468`; one read transaction). Error →
   `translateError(err)`; a ticket the wallet does not own gives `NotFound`,
   `item does not exist: ticket not found`. If the ticket is owned but has
   no saved per-ticket preference, the wallet-wide defaults are returned.
   With no agendas for the network: empty map and `voteBits = 0`; otherwise
   `voteBits` starts at 1 (bit 0 set) and ORs in the bits of each saved
   choice.
4. Respond `version`, `votebits = voteBits`, and one `Choice` per agenda in
   deployment order: `agenda_id`, `agenda_description`,
   `choice_id = choices[agenda_id]` (`abstain` when nothing is saved),
   `choice_description` = the description of the agenda choice whose id
   equals `choice_id`, or empty if none matches.

#### 5.10.2 `SetVoteChoices` (`server.go:3302-3323`)

1. Parse `ticket_hash` as in 5.10.1 step 1.
2. Build a map `agenda_id → choice_id` from `choices` in request order; a
   repeated agenda keeps the last entry.
3. `voteBits, err = wallet.SetAgendaChoices(ctx, ticketHash, map)`
   (`wallet/wallet.go:476-564`):
   1. no agendas for the network → error text `no agendas to set for this
      network` (no kind → `Unknown`);
   2. with a ticket hash: one read transaction to check ownership; not owned
      → `NotFound`, `item does not exist: ticket not found`;
   3. one read-write transaction that iterates the map **in Go map order
      (random)**: unknown agenda → kind `Invalid`, `no agenda with ID
      "<id>"`; unknown choice → kind `Invalid`, `agenda "<id>" has no choice
      ID "<choice>"` (both `%q`-quoted); otherwise persist the default or
      per-ticket preference. Any error rolls the whole transaction back.
   4. without a ticket hash: under `stakeSettingsLock`, for each applied
      choice clear the agenda's mask bits in the cached default vote bits and
      OR in the choice's bits; the result is returned. With a ticket hash:
      the returned bits are the ticket's vote bits as re-read inside the
      transaction after the last write; if the request's `choices` is empty
      the result is 0.
   Errors from 3.2-3.3 are wrapped with op `wallet.SetAgendaChoices`
   (`InvalidArgument`, message `wallet.SetAgendaChoices: invalid operation:
   no agenda with ID "<id>"`).
4. Error → `translateError(err)`; else `{votebits}`.

When several entries are invalid, which one is reported depends on map
iteration order (10.Q16).

#### 5.10.3 `TSpendPolicies` (`server.go:3330-3396`)

1. Non-empty `ticket_hash` → `chainhash.NewHash`; failure →
   `InvalidArgument`, `invalid hash length of <n>, want 32`.
2. If `hash` is non-empty:
   1. `chainhash.NewHash(hash)`; failure → same error form.
   2. `p = wallet.TSpendPolicy(hash, ticketHash)`
      (`wallet/wallet.go:591-628`): under `stakeSettingsLock`, first match
      of: per-ticket tspend policy (only with a ticket hash), wallet-wide
      tspend policy, then, if the tspend is in the wallet's tspend cache,
      the policy for its treasury key (bytes `[66:99]` of the first input's
      signature script), per-ticket first, then wallet-wide; else invalid.
   3. Respond one `Policy`: `hash` = request bytes, `policy` = `yes`
      (`TreasuryVoteYes`, 1), `no` (`TreasuryVoteNo`, 2) or **`abstain`**
      (anything else), `ticket_hash` = request bytes (absent if empty).
3. Otherwise: `tspends = wallet.GetAllTSpends(ctx)`
   (`wallet/wallet.go:274-289`): iterates the tspend cache **in Go map order
   (random)**, deleting and skipping entries whose expiry is below the
   current tip height. For each: `policy` = `yes`, `no` or **the empty
   string** (not `abstain`); `hash` = the tspend's transaction hash;
   `ticket_hash` = request bytes. The per-ticket lookup is used when a ticket
   hash was given.

Never returns a wallet error. Ticket ownership is not checked.

#### 5.10.4 `SetTSpendPolicy` (`server.go:3400-3446`)

1. `len(hash) != 32` → `InvalidArgument`, `invalid tspend hash length,
   expected 32 got <n>`.
2. If `ticket_hash` is non-empty and its length is not 32 →
   `InvalidArgument`, `invalid ticket hash length, expected 32 got <n>`.
3. `policy`: `abstain`, `invalid` or empty → invalid (0); `yes` → 1; `no` →
   2; anything else → `InvalidArgument`, `unknown policy "<policy>"` (Go
   `%q`). Matching is exact and case-sensitive.
4. `err = wallet.SetTSpendPolicy(ctx, hash, policy, ticketHash)`
   (`wallet/wallet.go:716-761`): under `stakeSettingsLock`, one read-write
   transaction writing the wallet-wide or per-ticket policy, then the cache
   update (an invalid policy deletes the cache entry).
5. Error → **returned as is, not translated** (`server.go:3441`): grpc-go
   reports `Unknown` (or `Canceled` / `DeadlineExceeded` if it wraps a
   context error) with the raw error text (10.Q15).
6. Respond with the empty message.

The tspend need not be known to the wallet and the ticket need not be owned.

#### 5.10.5 `TreasuryPolicies` (`server.go:3449-3475`)

1. `policies = wallet.TreasuryKeyPolicies()` (`wallet/wallet.go:640-661`):
   under `stakeSettingsLock`, all wallet-wide key policies, then all
   per-ticket key policies, each group **in Go map order (random)**.
2. For each: `key` = the key bytes, `policy` = `yes`, `no` or empty string,
   `ticket_hash` = the ticket hash for per-ticket entries, absent otherwise.

Never fails. Entries with an invalid/abstain policy are not stored in the
cache, so the empty-string case does not normally occur.

#### 5.10.6 `SetTreasuryPolicy` (`server.go:3482-3516`)

1. Non-empty `ticket_hash` → `chainhash.NewHash`; failure →
   `InvalidArgument`, `invalid hash length of <n>, want 32`.
2. `len(key) != 33` → `InvalidArgument`, `treasury key must be 33 bytes`.
   The key is not otherwise validated.
3. `policy` parsed as in 5.10.4 step 3 (same message).
4. `wallet.SetTreasuryKeyPolicy(ctx, key, policy, ticketHash)`
   (`wallet/wallet.go:666-711`); error → `translateError(err)`.
5. Respond with the empty message.

The handler comment says the preference is also sent to a configured VSP;
the gRPC code does not do that (10.Q17).

### 5.11 `MessageVerificationService.VerifyMessage` (`server.go:3523-3547`)

1. `addr = decodeAddress(address, chainParams)`; failure →
   `InvalidArgument`, `invalid address <address>: <err>`.
2. If the decoded address is not a version-0 secp256k1 ECDSA
   pay-to-pubkey-hash address → `InvalidArgument`, `address must be
   secp256k1 pay-to-pubkey-hash`. (Pay-to-pubkey addresses are rejected,
   contrary to `api.md:2983`.)
3. `valid, err = wallet.VerifyMessage(message, addr, signature,
   chainParams)` (`wallet/wallet.go:2036-2063`):
   1. digest = BLAKE-256 of `varstring("Decred Signed Message:\n") ||
      varstring(message)` (wire var-string: compact-size length then bytes);
   2. `ecdsa.RecoverCompact(signature, digest)`; failure → error with op
      `wallet.VerifyMessage`;
   3. hash160 of the recovered key serialized compressed or uncompressed
      according to the signature's header byte;
   4. `valid` = the P2PKH address string of that hash equals the request
      address's string.
4. Error → `translateError(err)`. Recovery errors carry no wallet kind, so a
   malformed signature is `Unknown`, e.g. `wallet.VerifyMessage: malformed
   signature: wrong size: 64 != 65`, not `valid = false` (10.Q18).
5. Respond `{valid}`.

Needs no wallet.

### 5.12 `DecodeMessageService.DecodeRawTransaction` (`server.go:3699-3725`)

1. `wire.MsgTx.Deserialize` over `serialized_transaction`; failure →
   `InvalidArgument`, `Could not decode Tx: <err>`. Bytes following a
   complete transaction are ignored. The serialization type in the upper 16
   bits of the version field is honoured (fields not present in a
   no-witness or witness-only serialization decode as zero).
2. Respond `transaction`:
   - `transaction_hash` = `mtx.TxHash()`, 32 bytes, internal order;
   - `transaction_type` = `marshalTxType(wallet.TxTransactionType(mtx))`:
     coinbase (regular or treasury-enabled form) → `COINBASE`; else ticket →
     `TICKET_PURCHASE`; else vote → `VOTE`; else revocation → `REVOCATION`;
     else `REGULAR` (`wallet/notifications.go:430-442`). The coinbase test
     is `compat.IsEitherCoinBaseTx` (`internal/compat/compat.go:19-27`),
     which accepts `standalone.IsCoinBaseTx(tx, false)`: exactly one input
     whose previous outpoint has a zero hash and index `0xffffffff`, with no
     version check and no treasury-spend exclusion
     (`dcrd/blockchain/standalone/v2@v2.3.0/tx.go:49-103`). A treasurybase
     and a **treasury spend** (single null-outpoint input) are therefore both
     `COINBASE`; a treasury add (ordinary inputs) is `REGULAR`;
   - `version` = the 16-bit transaction version, `lock_time`, `expiry`;
   - `inputs`, `outputs` as below.

Inputs (`marshalDecodedTxInputs`), one per input in order:
`previous_transaction_hash` (32 bytes), `previous_transaction_index`,
`tree` (the outpoint tree byte as a signed number), `sequence`,
`amount_in` (`ValueIn`), `block_height`, `block_index`,
`signature_script`, `signature_script_asm` =
`txscript.DisasmString(signature_script)` with its error discarded (the
string contains `[error]` where parsing failed).

Outputs (`marshalDecodedTxOutputs`), with `txType =
stake.DetermineTxType(mtx)` computed once; for output `i`:

1. `script_asm = DisasmString(pkScript)` (error discarded).
2. If `txType` is a ticket purchase and `i` is odd (a commitment output):
   - `script_class = STAKE_SUBMISSION` (6), which the source itself calls
     "Questionable" (`server.go:3598-3601`);
   - `addresses` = one element: the commitment address
     (`stake.AddrFromSStxPkScrCommitment`: P2SH if bit 7 of script byte 29
     is set, else P2PKH, from script bytes `[2:22]`), or on failure the
     literal `[error] failed to decode ticket commitment addr output for tx
     hash <txhash>, output idx <i>`;
   - `required_signatures = 0`;
   - `stake.AmountFromSStxPkScrCommitment` is called, but the result is
     stored only **when it returns an error** (`server.go:3612-3615`), in
     which case the amount is 0. `commitment_amount` is therefore always 0
     (10.Q2).
3. Otherwise: `(class, addrs) = stdscript.ExtractAddrs(version, pkScript,
   chainParams)`; `required_signatures =
   stdscript.DetermineRequiredSigs(version, pkScript)`;
   `addresses` = each address's string in order;
   `script_class = scProto(class)` (4.4).
4. `index = i`, `value`, `version` (16-bit script version), `script` =
   `pkScript`.

Needs no wallet. Addresses are encoded for the wallet's active network.

### 5.13 `NetworkService.GetRawBlock` (`server.go:3807-3846`)

Gated by readiness.

1. `n, err = wallet.NetworkBackend()`; error → `translateError(err)`:
   `Unavailable`, `wallet.NetworkBackend: Decred network is unreachable`.
2. If `block_hash` is empty: `info, err = wallet.BlockInfo(ctx,
   NewBlockIdentifierFromHeight(block_height))` (one read transaction; main
   chain lookup); error → `translateError(err)`; use `info.Hash`. An empty
   request therefore fetches the block at height 0.
3. Else `chainhash.NewHash(block_hash)`; failure → `InvalidArgument`,
   `invalid blockhash: invalid hash length of <n>, want 32`. `block_height`
   is ignored.
4. `blocks, err = n.Blocks(ctx, [hash])`; error → `translateError(err)`.
5. Zero blocks → `Internal`, `network returned 0 blocks`.
6. `raw, err = blocks[0].Bytes()` (wire serialization); error →
   `translateError(err)`.
7. Respond `{block: raw}`.

The block is not stored. With the offline backend step 1 succeeds and step 4
fails with whatever the offline backend returns (spec 06/15).

### 5.14 Start-up and shutdown ordering (`dcrwallet.go:78-431`)

1. Config, logging, pipes, profiling.
2. `loader = NewLoader(...)` with `dbDir = networkDir(appdata, params)`.
3. Deferred: on return (not on panic) `loader.UnloadWallet()`; logs
   `Failed to close wallet: <err>` (ERR) unless the error is kind `Invalid`,
   or `Closed wallet` (INF) on success.
4. Unless `--noinitialload`: open the wallet (`OpenExistingWallet` in a
   goroutine, raced against shutdown), unlock / prompt, set up the VSP
   client, start the config-driven ticket buyer / mixer.
5. `startRPCServers` (5.1). Failure → `Unable to create RPC servers: <err>`.
6. If the gRPC server exists: register the service-start callback (5.1);
   defer `Stop()` (logs `Stopping gRPC server...` WRN, `gRPC server
   shutdown` INF).
7. JSON-RPC shutdown plumbing.
8. Unless `--noinitialload`: `RunAfterLoad` the sync loop (offline backend,
   `spvLoop` or `rpcSyncLoop`), which runs on the main goroutine's call
   because the wallet is already loaded.
9. Wait for shutdown.

Shutdown (deferred calls run in reverse order): JSON-RPC stop, gRPC
`Stop()`, wait for the config-driven ticket buyer, unload the wallet, close
the log rotator. `grpc.Server.Stop()` closes listeners and connections and
cancels every stream context immediately, so in-flight `RpcSync`, `SpvSync`,
`RunTicketBuyer` and `RunAccountMixer` handlers see a cancelled context and
return their `Canceled` statuses (which no client receives).

With `--noinitialload`, nothing syncs until a client calls `RpcSync` or
`SpvSync`; the expected client sequence is `WalletExists` → `CreateWallet`
/ `OpenWallet` → `RpcSync` or `SpvSync` (kept open) → wallet-dependent
services.

---

## 6. Constants, limits and defaults

| Name | Value | Source | What depends on it |
|---|---|---|---|
| gRPC API version | `10.0.0` (major 10, minor 0, patch 0) | `server.go:63-68` | `VersionService.Version`; clients' compatibility checks |
| protobuf package | `walletrpc` | `api.proto:3` | every method path and `serviceMap` key |
| `walletDbName` | `wallet.db` | `loader.go:22` | file created / opened / stat'ed |
| `driver` | `bdb` | `loader.go:23` | walletdb driver lookup |
| directory mode | `0700` | `loader.go:122`, `:153`, `:212`, `:243` | network directory creation |
| database file mode | `0600` | `wallet/internal/bdb/db.go:321` | `wallet.db` creation |
| `wallet.InsecurePubPassphrase` | `public` | `wallet/wallet.go:66` | default for empty `public_passphrase` |
| minimum birthday age | 24 hours before now | `server.go:2720` | `CreateWallet` `birth_time` validation |
| `hdkeychain.RecommendedSeedLen` | 32 | `dcrd/hdkeychain/v3@v3.1.3/extendedkey.go:30` | `GenerateRandomSeed` default |
| `hdkeychain.MinSeedBytes` / `MaxSeedBytes` | 16 / 64 | `extendedkey.go:40`, `:44` | `GenerateRandomSeed`, `DecodeSeed`, `CreateWallet` |
| `ErrInvalidSeedLen` text | `seed length must be between 128 and 512 bits` | `extendedkey.go:79-80` | status messages |
| PGP word list size | 512 words | `pgpwordlist/wordlist.go:33-544` | mnemonics |
| mnemonic checksum | 1 byte, `SHA-256(SHA-256(seed))[0]` | `walletseed/seed.go:38-41` | mnemonics |
| vote version | mainnet 11, testnet3 12, simnet 12, testnet2 6, other 1 | `wallet/wallet.go:291-304` | `Agendas`, `VoteChoices`, `SetVoteChoices` |
| treasury vote values | invalid 0, yes 1, no 2 | `dcrd/blockchain/stake/v5@v5.0.3/staketx.go:828-836` | policy strings |
| policy strings accepted | `abstain`, `invalid`, empty, `yes`, `no` | `server.go:3427-3437`, `:3497-3507` | `SetTSpendPolicy`, `SetTreasuryPolicy` |
| policy strings returned | `yes`, `no`, `abstain` (single-hash query only), empty | `server.go:3347-3354`, `:3379-3384`, `:3456-3461` | `TSpendPolicies`, `TreasuryPolicies` |
| treasury key length | 33 (`secp256k1.PubKeyBytesLenCompressed`) | `server.go:3492` | `SetTreasuryPolicy` |
| hash length | 32 (`chainhash.HashSize`) | `server.go:3401`, `:3414` | all hash fields |
| pi-key offset in tspend | signature script bytes `[66:99]` of input 0 | `wallet/wallet.go:615` | `TSpendPolicies` |
| compact signature size | 65 | `dcrd/dcrec/secp256k1/v4@v4.4.1/ecdsa/signature.go:741` | `VerifyMessage` |
| message prefix | `Decred Signed Message:\n` | `wallet/wallet.go:2041` | `VerifyMessage` |
| `SStxPKHMinOutSize` | 32 | `dcrd/blockchain/stake/v5@v5.0.3/staketx.go:81` | commitment address extraction |
| dcrd RPC default ports | mainnet 9109, testnet3 19109, simnet 19556 | `internal/netparams/params.go:23`, `:32`, `:41` | `RpcSync` `network_address` without port |
| SPV default port | `chainParams.DefaultPort` | `server.go:3139` | `spv_connect` entries without port |
| SPV local address | `[::1]:0` | `server.go:3014` | local peer advertisement |
| lock channel capacity | 1 | `server.go:2543`, `:2663`, `:2853`, `:3003` | passphrase timeout handling |
| gRPC ports | mainnet 9111, testnet3 19111, simnet 19558 | `internal/netparams/params.go:25`, `:34`, `:43` | listeners |
| max received gRPC message | 4 MiB (4194304) | `grpc@v1.83.2/server.go:61` | large requests rejected with `ResourceExhausted` by the transport |
| max sent gRPC message | 2^31 - 1 | `grpc@v1.83.2/server.go:62` | `GetRawBlock`, `DecodeRawTransaction` responses |
| TLS minimum version | 1.2 | `rpcserver.go:274` | handshake |
| server keepalive (grpc-go defaults) | ping after 2 h idle, 20 s ping timeout; client pings permitted at most every 5 min | `grpc@v1.83.2/internal/transport/defaults.go:38-40` | long-lived streams, client keepalive settings |
| `errors.Separator` | `:: ` | `dcrwallet.go:38` | every status message built from a nested wallet error |
| logger subsystems | `GRPC` (interceptors, grpc-go internals), `LODR` (loader) | `internal/loggers/loggers.go:47`, `:52` | log output |

---

## 7. Concurrency and lifecycle

**Goroutines**

- One per listener, running `server.Serve` (`rpcserver.go:338-346`).
- grpc-go runs every RPC handler on its own goroutine (no worker pool is
  configured). Handlers of long-lived streams (`RpcSync`, `SpvSync`,
  `RunTicketBuyer`, `RunAccountMixer`) occupy theirs for the stream's
  lifetime.
- Syncer and ticket-buyer internals (specs 11, 14, 15) spawn their own.
- `wallet.Unlock` spawns one timeout goroutine per unlock with a non-nil
  timeout channel.

**Locks**

| Lock | Protects | Notes |
|---|---|---|
| `Loader.mu` | `callbacks`, `wallet`, `db` | Held for the whole of create / open / unload, including database initialisation and upgrades, except during `wallet.OpenDB` in `OpenExistingWallet`. `onLoaded` callbacks run with it held, so a callback must not call back into the loader. `DbDirPath` and `WalletExists` do not take it. |
| `<service>.readyMtx` (RWMutex) | `ready`, `wallet` of each gated service | Write-locked once in `Start…Service`; read-locked by `checkReady`. Handlers then read `wallet` without the lock (safe because it is written once before `ready`). |
| `wallet.stakeSettingsLock` | vote bits and treasury policy caches | Held across the database transaction in `SetTSpendPolicy` / `SetTreasuryKeyPolicy`; taken after the transaction in `SetAgendaChoices`. |
| none | `loaderService.loader/activeNet`, `agendaService.activeNet`, `messageVerificationService.chainParams`, `decodeMessageService.chainParams` | Written once in `startRPCServers` before `Serve` starts. |

Lock order where two are held: `Loader.mu` → `<service>.readyMtx` (via the
`RunAfterLoad` callback). `stakeSettingsLock` → database write transaction
(policy setters).

**Contexts**

- Unary handlers pass the request context into wallet calls.
- Streaming handlers use `svr.Context()`. Cancellation by the client,
  connection loss, or `grpc.Server.Stop()` cancels it; the syncer / ticket
  buyer then winds down and the handler returns.
- There is no server-side timeout on any RPC.

**Stream sends**

- Sync notifications are sent from syncer goroutines through
  `svr.Send`, synchronously inside the syncer callback
  (`chain/sync.go:86-88`: callbacks "block the syncer from continuing").
  HTTP/2 flow control therefore back-pressures the syncer: a client that
  stops reading eventually stalls synchronisation. Notifications are never
  dropped or coalesced by this layer, and send errors are ignored (the
  syncer keeps running until the context is cancelled).
- Ordering observed by a client is the order in which the syncer invoked the
  callbacks. SPV peer notifications originate from per-peer goroutines and
  interleave with the sync-progress notifications.
- Response headers are sent with the first message; a stream that ends
  without any message is a trailers-only response.

**Unsynchronized handler state**

- The "already synchronizing" test and the syncer's later
  `SetNetworkBackend` are separate critical sections of
  `wallet.networkBackendMu`; nothing serializes two sync RPCs (5.5.9).
- `lockWallet` in `RpcSync` / `SpvSync` is a captured local variable written
  (`lockWallet = nil`) from the syncer goroutine that invokes
  `DiscoverAddressesFinished` and read by nothing else afterwards (the
  deferred call captured the original function value), so there is no data
  race, but the passphrase buffer is zeroed from the syncer goroutine while
  the handler goroutine is blocked in `Run`.
- No goroutine is launched by any handler in scope or by `internal/loader`;
  all concurrency comes from grpc-go, the syncers, the ticket buyer and
  `wallet.Unlock`.

**Lifecycle of the gated services**

`ready` and `wallet` are set once per process. After `CloseWallet`, the gated
services (and `WalletService`) keep the pointer to the closed wallet; after a
subsequent `OpenWallet` or `CreateWallet` they still hold the **old** wallet
because the loader discarded its callbacks (10.Q3). Only `loaderServer`
(`RpcSync`, `SpvSync`, `RescanPoint`) and the JSON-RPC server, which ask the
loader on every call, see the new wallet.

**Shutdown**: 5.14.

---

## 8. Errors

### 8.1 Status inventory by RPC

`T(err)` = `translateError(err)` (5.3). Codes are gRPC canonical codes.

| RPC | Condition | Code | Message |
|---|---|---|---|
| any gated service | wallet never loaded | `FailedPrecondition` | `service walletrpc.<Service> is not ready` |
| any | unknown method / service | `Unimplemented` | grpc-go: `unknown method <m> for service <s>` / `unknown service <s>` |
| any | method path without a second `/` | `Unimplemented` | grpc-go: `malformed method name: "<path>"` (`grpc@v1.83.2/server.go:1787`) |
| any | request larger than 4 MiB | `ResourceExhausted` | grpc-go: `grpc: received message larger than max (<n> vs. 4194304)` |
| `WalletExists` | stat failure | `T` (`Unknown`) | `loader.WalletExists: <os error>` |
| `CreateWallet` | no seed | `InvalidArgument` | `seed is a required parameter` |
| | both birth flags | `InvalidArgument` | `both set birth time and height cannot be set` |
| | birth time < 24 h old | `InvalidArgument` | `birth time cannot be in the future or too close (one day) to the present` |
| | wallet loaded | `AlreadyExists` | `loader.CreateNewWallet: item already exists: wallet already opened` |
| | file exists | `AlreadyExists` | `loader.CreateNewWallet: item already exists: wallet DB exists` |
| | seed length | `InvalidArgument` | `loader.CreateNewWallet:: wallet.Create: seed length must be between 128 and 512 bits` |
| | empty private passphrase | `InvalidArgument` | `loader.CreateNewWallet: invalid operation:: wallet.Create: private passphrase may not be empty` |
| | other loader failure | `T` | wallet error string |
| | birthday write failure | `Internal` | `unable to set birthday state: <err>` |
| `CreateWatchingOnlyWallet` | wallet loaded | `AlreadyExists` | `loader.CreateWatchingOnlyWallet: item already exists: wallet already loaded` |
| | file exists | `AlreadyExists` | `loader.CreateWatchingOnlyWallet: item already exists: wallet already exists` |
| | bad xpub etc. | `T` | wallet error string |
| `OpenWallet` | wallet loaded | `AlreadyExists` | `loader.OpenExistingWallet: item already exists: wallet already opened` |
| | no file | `NotFound` | `loader.OpenExistingWallet: item does not exist:: wallet.OpenDB: missing database file` |
| | wrong public passphrase | `InvalidArgument` | wallet error string of kind `Passphrase` |
| | other | `T` | wallet error string |
| `CloseWallet` | not loaded (or close error of kind `Invalid`) | `FailedPrecondition` | `Wallet is not loaded` |
| | close failure | `T` | `loader.UnloadWallet: <err>` |
| `RescanPoint` | not loaded | `FailedPrecondition` | `Wallet has not been loaded` |
| | lookup failure | `FailedPrecondition` | `Rescan point failed to be requested <err>` |
| `RpcSync`, `SpvSync` | not loaded | `FailedPrecondition` | `Wallet has not been loaded` |
| | backend already set | `FailedPrecondition` | `wallet is loaded and already synchronizing` |
| | discover without passphrase | `InvalidArgument` | `private passphrase is required for discovering accounts` |
| | unlock failure | `T` (`InvalidArgument` for a wrong passphrase, `Unimplemented` for watching-only) | `wallet.Unlock: …` |
| `RpcSync` | syncer error, context done | `Canceled` | `Wallet synchronization canceled: <err>` |
| | syncer error otherwise | `Unknown` | `Wallet synchronization stopped: <err>`; `<err>` begins `rpcsyncer.Run: `; causes before the backend is set are listed in 5.5.9 item 4 |
| `SpvSync` | bad `spv_connect` entry | `FailedPrecondition` | `SPV Connect address invalid: <err>` |
| | syncer error wraps `context.Canceled` | `Canceled` | `SPV synchronization canceled: <err>` |
| | wraps `context.DeadlineExceeded` | `DeadlineExceeded` | `SPV synchronization deadline exceeded: <err>` |
| | other syncer error | `T` | wallet error string |
| `RunAccountMixer` | unlock failure | `T` | `wallet.Unlock: …` |
| | run error, context done | `Canceled` | `AccountMixer instance canceled, account number: <mixed_account>` |
| | run error otherwise | `Unknown` | `AccountMixer instance errored: <err>` |
| `RunTicketBuyer` | VSP client | `InvalidArgument` | `VSP client failed to start. Error: <err>` |
| | negative maintain | `InvalidArgument` | `Negative balance to maintain given` |
| | mixed account | `InvalidArgument` | `Mixing requested, but error on mixed account: <err>` |
| | branch | `InvalidArgument` | `MixedAccountBranch should be 0 or 1.` |
| | split account | `InvalidArgument` | `Mixing requested, but error on mixedSplitAccount: <err>` |
| | change account | `InvalidArgument` | `Mixing requested, but error on changeAccount: <err>` |
| | unlock failure | `T` | `wallet.Unlock: …` |
| | run error, context done | `Canceled` | `TicketBuyer instance canceled, account number: <account>` |
| | run error otherwise | `Unknown` | `TicketBuyer instance errored: <err>` |
| `GenerateRandomSeed` | length not 0 and not in 16..64 | `InvalidArgument` | `invalid seed length` |
| `DecodeSeed` | any decode failure | `InvalidArgument` | table in 5.8.2 |
| `Agendas` | — | — | never fails |
| `VoteChoices`, `SetVoteChoices`, `TSpendPolicies`, `SetTreasuryPolicy` | bad hash length | `InvalidArgument` | `invalid hash length of <n>, want 32` |
| `VoteChoices`, `SetVoteChoices` | ticket not owned | `NotFound` | `item does not exist: ticket not found` |
| `SetVoteChoices` | no agendas on this network | `Unknown` | `no agendas to set for this network` |
| | unknown agenda | `InvalidArgument` | `wallet.SetAgendaChoices: invalid operation: no agenda with ID "<id>"` |
| | unknown choice | `InvalidArgument` | `wallet.SetAgendaChoices: invalid operation: agenda "<id>" has no choice ID "<choice>"` |
| `SetTSpendPolicy` | tspend hash length | `InvalidArgument` | `invalid tspend hash length, expected 32 got <n>` |
| | ticket hash length | `InvalidArgument` | `invalid ticket hash length, expected 32 got <n>` |
| | database error | `Unknown` (untranslated) | raw error text |
| `SetTSpendPolicy`, `SetTreasuryPolicy` | bad policy | `InvalidArgument` | `unknown policy "<policy>"` |
| `SetTreasuryPolicy` | key length | `InvalidArgument` | `treasury key must be 33 bytes` |
| | database error | `T` | wallet error string |
| `VerifyMessage` | undecodable address | `InvalidArgument` | `invalid address <a>: <stdaddr error>` |
| | not P2PKH-ECDSA | `InvalidArgument` | `address must be secp256k1 pay-to-pubkey-hash` |
| | signature recovery failure | `Unknown` | `wallet.VerifyMessage: <ecdsa error>` |
| `DecodeRawTransaction` | undecodable | `InvalidArgument` | `Could not decode Tx: <wire error>` |
| `GetRawBlock` | no network backend | `Unavailable` | `wallet.NetworkBackend: Decred network is unreachable` |
| | bad hash | `InvalidArgument` | `invalid blockhash: invalid hash length of <n>, want 32` |
| | height not in main chain | `T` | `wallet.BlockInfo: …` |
| | backend failure | `T` | backend error string |
| | empty result | `Internal` | `network returned 0 blocks` |

Messages that embed a nested wallet error string (those containing `::`)
were derived by applying `errors.E` collapsing and `Error()` formatting to
the call chains read; they are to be confirmed by the oracle shim (13.1).

`ecdsa.RecoverCompact` error texts: `malformed signature: wrong size: <n>
!= 65`; `invalid signature: public key recovery code <c> is not in the valid
range [<min>, <max>]`; `invalid signature: R >= group order`; `invalid
signature: R is 0`; `invalid signature: S >= group order`; `invalid
signature: S is 0`; `invalid signature: signature R + N >= P`; `invalid
signature: not for a valid curve point`; `invalid signature: recovered
pubkey is the point at infinity`
(`dcrd/dcrec/secp256k1/v4@v4.4.1/ecdsa/signature.go:862-995`).

### 8.2 Log lines

| Level | Subsystem | Text |
|---|---|---|
| DBG | `GRPC` | `Unary method <m> invoked by <addr>`; `Streaming method <m> invoked by <addr>`; `Streaming method <m> invoked by <addr> errored: <err>` (cancelled streams) |
| ERR | `GRPC` | `Unary method <m> invoked by <addr> errored: <err>`; `Streaming method <m> invoked by <addr> errored: <err>` |
| INF | `GRPC` | grpc-go internal messages, with a leading `grpc: ` removed |
| CRT | `GRPC` | grpc-go fatal messages, followed by process exit status 1 |
| ERR | `LODR` | `Failed to open database: <err>` |
| INF / ERR | `DCRW` | `gRPC server listening on <addr>`, `Closed wallet`, `Failed to close wallet: <err>` (spec 16) |

The wallet, syncer, ticket buyer and VSP subsystems log under their own tags
(other specs), for example `The wallet has been temporarily unlocked` and
`The wallet has been locked due to timeout.` from `wallet.Unlock`.

### 8.3 Panics

`Start…Service` panics with `service already started` if called twice for a
gated service. With the loader discarding callbacks after the first load this
cannot happen through gRPC.

---

## 9. Compatibility-critical behaviours

1. Method paths `/walletrpc.<Service>/<Method>` for the 22 methods of 3.3,
   their unary / server-streaming kinds, and the field numbers and types of
   4.3-4.4, including `sint32` for `GetRawBlockRequest.block_height` and the
   negative enum value `TreeType.UNKNOWN = -1`.
2. Mutual TLS is the only authentication; TLS 1.2 minimum; no plaintext
   mode; the gRPC server is absent when no client CA is available.
3. Service readiness: before the first wallet load, calls to
   `AccountMixerService`, `TicketBuyerService`, `VotingService`,
   `NetworkService` (and `WalletService`) fail with `FailedPrecondition`
   `service walletrpc.<Name> is not ready`; `WalletLoaderService`,
   `SeedService`, `AgendaService`, `MessageVerificationService`,
   `DecodeMessageService` (and `VersionService`) always answer. Clients that
   start the wallet with `--noinitialload`, the mode the loader service
   exists for, rely on this split.
4. `CreateWallet`, `CreateWatchingOnlyWallet` and `OpenWallet` substitute
   `public` for an empty public passphrase, so wallets created by the Go
   wallet with the default open under the replacement and vice versa.
5. `CreateWallet` requires a seed of 16..64 bytes and a non-empty private
   passphrase, creates `<netdir>/wallet.db` (directory mode 0700), and
   leaves the wallet loaded. On failure after the file was created the file
   is removed.
6. The birthday state written by `CreateWallet` (5.5.2 table). A replacement
   that "fixes" the flag handling changes which blocks a restored wallet
   scans; this needs an explicit decision (13.D1).
7. `OpenWallet` returns `watching_only`; errors: `AlreadyExists` when
   loaded, `NotFound` when the file is missing, `InvalidArgument` for a
   wrong public passphrase.
8. `CloseWallet` returns `FailedPrecondition` `Wallet is not loaded` when
   nothing is loaded.
9. `RpcSync` / `SpvSync` preconditions and their exact codes and messages
   (8.1), in the order: wallet loaded, not already syncing, passphrase
   present when discovering, unlock.
10. Sync notification stream: one message per syncer callback, in callback
    order, never dropped; `synced` true only on `SYNCED`; payload
    sub-messages present exactly on the three progress types and the two
    peer types; empty sub-messages still present on the wire (4.5).
    `notification_type` is the only discriminator a client has.
11. `fetch_headers.last_header_time` is Unix **seconds**;
    `fetched_headers_count` is a per-batch count under `RpcSync` and a
    block height under `SpvSync`.
12. `RpcSync` connects without TLS exactly when the address is `localhost`
    or a loopback IP (with or without port) **and** no certificate was
    supplied; default dcrd port per network; HTTP basic auth only when
    `username` is non-empty.
13. `SpvSync` with `spv_connect`: entries are normalized with the network's
    default P2P port and used as persistent peers instead of DNS-seeded
    discovery; an invalid entry is `FailedPrecondition`.
14. `SpvSync` stores peer addresses in `<netdir>/peers.json`.
15. With `discover_accounts`, the wallet is unlocked for the discovery phase
    and locked again when address discovery finishes or the RPC ends; the
    passphrase buffer is wiped.
16. `RunTicketBuyer` and `RunAccountMixer` never send a message; they end
    only by cancellation (`Canceled` with the account number in the text) or
    a fatal error (`Unknown`). Validation order and messages of 5.7.
17. After `RunTicketBuyer` / `RunAccountMixer` with a passphrase ends, the
    wallet remains unlocked (10.Q5). Clients may have come to depend on
    either reading; decision 13.D2.
18. `GenerateRandomSeed`: default 32 bytes, range 16..64, hex lower-case,
    mnemonic per 4.8. `DecodeSeed`: accepts hex (either case) or a PGP-word
    mnemonic with checksum, case-insensitive words, tolerant of repeated
    spaces; same seeds as every existing Decred wallet backup.
19. `Agendas` and `VoteChoices` report the vote version and deployments of
    the pinned chain parameters, in deployment order; `VoteChoices` reports
    `abstain` for agendas without a saved choice and `votebits` with bit 0
    set.
20. `SetVoteChoices` persists default or per-ticket preferences in the
    existing database buckets and returns the resulting vote bits; unknown
    agenda / choice → `InvalidArgument`; unowned ticket → `NotFound`.
21. Treasury policies: accepted input strings, returned strings (`abstain`
    only for a single-hash query, empty string otherwise), 33-byte key
    check, per-ticket variants keyed by ticket hash, persistence through the
    existing buckets.
22. `VerifyMessage` accepts only P2PKH-ECDSA addresses of the active
    network, uses the `Decred Signed Message:\n` prefix and BLAKE-256, and
    reports malformed signatures as an error status, not `valid = false`.
23. `DecodeRawTransaction` output field semantics of 5.12, including
    `script_class = STAKE_SUBMISSION` and a single commitment address for
    odd ticket outputs, `commitment_amount` always 0, `[error]` markers in
    disassembly, and `transaction_type` mapping.
24. `GetRawBlock` returns the wire-serialized block; selects by hash when
    `block_hash` is non-empty, else by main-chain height; `Unavailable`
    without a network backend.
25. Status messages are part of the observable surface (clients display
    them and some match on substrings). The wallet error-string format with
    separator `:: ` and operation prefixes must be reproduced where a
    message embeds a wallet error.
26. Hashes are raw 32-byte values in internal byte order in every field of
    these services; no hex strings.
27. Request secrets (`private_passphrase`, `seed`, `password`,
    `passphrase`) are wiped from the request buffer when the handler is done
    with them.
28. Maximum inbound message size 4 MiB.
29. `RpcSync` / `SpvSync` may be issued again after a previous sync stream
    has ended (the network backend is cleared when the syncer returns);
    front ends implement reconnection by re-issuing the RPC, since the
    gRPC-started syncers never retry by themselves (5.5.9).
30. The ten unreferenced loader messages (4.3) stay in the protobuf
    descriptor with their field numbers; clients generated from the
    upstream `api.proto` reference these types even though no RPC uses them.
31. No service in scope implements gRPC reflection, health checking or
    per-RPC authentication metadata; a client that probes for them gets
    `Unimplemented` `unknown service …`.

---

## 10. Quirks

**Q1. `CreateWallet` birthday flags are crossed.** `server.go:2730` tests
`req.SetBirthTime` where `req.SetBirthHeight` was meant. Result: 5.5.2
table. `birth_time` is validated and then discarded; `birth_height` is
honoured only together with `set_birth_time`; `set_birth_height` alone
writes height 0. The stored `Time` is Go's zero time.

**Q2. `commitment_amount` is never populated.** `server.go:3612-3615`
assigns the commitment amount only when `AmountFromSStxPkScrCommitment`
returns an error (inverted test), and in that case the amount is 0. For a
well-formed ticket the field is always absent.

**Q3. A wallet reopened over gRPC is invisible to the wallet-dependent
services.** `Loader.onLoaded` sets `callbacks = nil` (`loader.go:88`), and
the gRPC service-start callback is registered once (`dcrwallet.go:388`).
After `CloseWallet` + `OpenWallet` (or `CreateWallet`), `WalletService`,
`TicketBuyerService`, `AccountMixerService`, `NetworkService` and
`VotingService` still reference the first, closed wallet; only the loader
service's own RPCs and JSON-RPC use the new one.

**Q4. Readiness is never revoked.** After `CloseWallet`, gated services
remain "ready" and fail inside the wallet with database-closed errors
instead of `FailedPrecondition … is not ready`. `api.md:244-246` describes
this ("will remain running but any operations that require the database
will be unusable").

**Q5. The lock-on-exit of `RunTicketBuyer` / `RunAccountMixer` is
ineffective.** The handler unlocks with a timeout channel
(`server.go:2550`, `:2670`), then `tb.Run` unlocks again with a nil timeout
(`ticketbuyer/tb.go:65`), which cancels the first timeout
(`wallet/wallet.go:1804-1832`). The deferred `lockWallet` only feeds the
drained channel. The wallet stays unlocked after the RPC. The passphrase
buffer is still zeroed.

**Q6. `CloseWallet` stops nothing.** `UnloadWallet` only closes the
database (`loader.go:385`); its comment ("stops the loaded wallet") and
`api.md:243` ("cleanly stop all wallet operations") overstate. A running
`RpcSync` / `SpvSync` stream is not ended by `CloseWallet`; it fails later
on database access.

**Q7. `MixSplitLimit` is dropped for wallets created with
`CreateNewWallet`.** The config literal at `loader.go:259-274` omits it, so
until the process restarts and reopens the wallet the mix split limit is 0.
`CreateWatchingOnlyWallet` and `OpenExistingWallet` pass it.

**Q8. Failed creation removes the file without closing the database.**
`loader.go:146-150`, `:236-240` call `os.Remove(dbPath)` while the bbolt
handle from `CreateDB` is still open and never close it. On Unix the unlink
succeeds and the descriptor leaks until exit. Windows behaviour is
UNVERIFIED (removing an open file normally fails there, which would leave a
partial `wallet.db` behind and make the next `CreateWallet` fail with
`wallet DB exists`).

**Q9. Concurrent `OpenWallet` calls.** `OpenExistingWallet` drops the mutex
around `wallet.OpenDB` (`loader.go:300-302`) and does not re-check
`l.wallet` afterwards. bbolt's open has no timeout, so a second concurrent
call blocks on the file lock indefinitely (the first holds it for as long as
the wallet is open); if the first is later closed, the blocked call proceeds
and loads the wallet. A blocked RPC is not released by client cancellation.

**Q10. Ticket commitment outputs are classed `STAKE_SUBMISSION`.**
`server.go:3598-3601`; the example client notes the same
(`rpc/grpc_example/main.go:248`). The JSON-RPC decoder reports these as
`sstxcommitment` (`internal/rpc/jsonrpc/methods.go:65`).

**Q11. `FetchHeadersNotification` means different things per syncer and the
documentation is wrong about units.** `RpcSync`: count of headers newly
inserted in the batch (`chain/sync.go:294`); `SpvSync`: height of the last
header (`spv/sync.go:265`). `api.md:2425` says `last_header_time` is in
nanoseconds; both syncers send Unix seconds.

**Q12. `discover_accounts` does not directly control account discovery.**
Both syncers decide from `!wallet.Locked()` at construction
(`chain/sync.go:82`, `spv/sync.go:144`). The flag only causes the handler to
unlock the wallet first. A wallet already unlocked by another RPC discovers
accounts even with `discover_accounts = false`; with the flag false and the
wallet locked, only address discovery within known accounts runs.

**Q13. Syncers started over gRPC ignore most network configuration.**
Compared with `spvLoop` / `rpcSyncLoop` (`dcrwallet.go:531-600`), the gRPC
paths set no dial function (`p2p.NewLocalPeer` defaults to a plain
`net.Dialer`, `p2p/peering.go:169-172`; `chain.RPCOptions.Dial` is nil), so
`--proxy` / Tor settings are bypassed for both SPV peers and the dcrd
connection. They also do not apply `--spvdisablerelaytx`, ignore
`--spvconnect`, `--rpcconnect`, `--cafile`, `--dcrdauthtype` with
`--dcrdclientcert` / `--dcrdclientkey`, and `--noclienttls`, and do not
retry:
the first syncer error ends the RPC and the client must call again.

**Q14. Status codes differ from `api.md`.**

| RPC / condition | `api.md` | Code |
|---|---|---|
| `CreateWallet`, `CreateWatchingOnlyWallet`, `OpenWallet` while a wallet is open | `FailedPrecondition` (`:175`, `:205`, `:233`) | `AlreadyExists` |
| `RpcSync` bad address / bad credentials | `InvalidArgument` (`:390-398`) | `Unknown` |
| `RpcSync` dcrd unreachable | `Unavailable` (`:400`) | `Unknown` |
| `RpcSync` / `SpvSync` wrong private passphrase | `FailedPrecondition` (`:322-323`, `:406-407`) and `InvalidArgument` (`:326`) | `InvalidArgument` |
| `SpvSync` discover without passphrase | not listed | `InvalidArgument` |
| `RunTicketBuyer` "invalid pool address" | `InvalidArgument` (`:2794`) | no such check exists |
| `VerifyMessage` P2PK address | accepted (`:2983`, `:2996`) | `InvalidArgument` |
| `VoteChoices` unowned ticket | "ExpectedErrors: None" (`:2933`) | `NotFound` |
| `CreateWallet` `birth_time` | "at least a day ahead of current time" (`:161-163`), "not 24 hours ahead of current time" (`:184`) | must be at least a day **before** now |

**Q15. `SetTSpendPolicy` returns the wallet error untranslated**
(`server.go:3439-3442`), unlike `SetTreasuryPolicy` (`:3509-3512`). Any
failure is `Unknown` with the raw text. The trailing `return resp, err`
(`:3445`, `:3515`) always has `err == nil`.

**Q16. Go map iteration order is observable in three places.**
(a) `SetAgendaChoices` iterates the choice map (`wallet/wallet.go:505`): with
more than one invalid entry, which one is reported is random. The returned
vote bits are unaffected (the final database state is the same in any
order).
(b) `TSpendPolicies` without a hash lists tspends in map order
(`wallet/wallet.go:280`). (c) `TreasuryPolicies` lists wallet-wide policies
then per-ticket policies, each in map order (`wallet/wallet.go:645-659`).
A port should treat the order of (b) and (c) as unspecified and may sort.

**Q17. `SetTreasuryPolicy` comment vs code.** The comment
(`server.go:3480-3481`) says the preference is forwarded to a configured
VSP. The gRPC handler has no VSP configuration and forwards nothing (the
JSON-RPC `settreasurypolicy` handler does,
`internal/rpc/jsonrpc/methods.go:3924-3928`). The same holds for `SetVoteChoices`
and `SetTSpendPolicy` over gRPC; clients use
`WalletService.SetVspdVoteChoices` for that.

**Q18. `VerifyMessage` error for malformed signatures.** A signature of the
wrong length or with an invalid recovery code yields status `Unknown`
rather than `valid = false` (`server.go:3542-3545`); only a well-formed
signature by a different key yields `valid = false`.

**Q19. `abstain` vs empty policy string.** `TSpendPolicies` with a hash
returns `abstain` for an unset policy (`server.go:3352-3353`); without a
hash, and in `TreasuryPolicies`, an unset policy is the empty string
(`:3378-3384`, `:3455-3461`).

**Q20. `api.md` is stale.** Header says `Version: 7.13.x` (`:3`); the server
reports `10.0.0`. The service index (`:34-43`) omits `AccountMixerService`.
`VotingService` documents only `VoteChoices` and `SetVoteChoices`
(`:2895-2896`); `TSpendPolicies`, `SetTSpendPolicy`, `TreasuryPolicies`,
`SetTreasuryPolicy` are undocumented. `RpcSyncRequest` documents a
non-existent `spv_connect` (`:361-363`); `RpcSyncResponse` documents a
non-existent `syncing_status` (`:384-386`) and omits `peer_information`.
`RunTicketBuyerRequest` documents only fields 1-4 (`:2778-2786`).
`AccountMixerService` is described as launching "the ticket buyer"
(`:2800-2801`). `RescanPointResponse` field is called `rescan_point`
(`:269`; actual `rescan_point_hash`). `CreateWatchingOnlyWalletRequest`
field is called `extended_public_key` (`:194`; actual `extended_pub_key`).
`OpenWalletResponse.watching_only` is undocumented. Typos:
`CreateWalletReponse`, `RescanProgressNotifiction`. `GetRawBlock` is
documented correctly.

**Q21. `clientusage.md` and `serverchanges.md` are stale.** The C++, C#,
Node.js and Python examples present no client certificate and use
deprecated client libraries (`grpc.load`, Python 2 beta API); they cannot
complete a handshake with a server that requires client certificates.
`serverchanges.md:76` shows a `protoc` invocation for the removed
`plugins=grpc` generator and links to `../rpcserver/` (`:88`), which does
not exist (the package is `internal/rpc/rpcserver`).

**Q22. Streams withhold headers until the first message.** grpc-go behaviour
(no explicit `SendHeader`). `RunTicketBuyer` / `RunAccountMixer` clients see
no response headers for the whole run; `RpcSync` / `SpvSync` clients see them
with the first notification. Send errors are discarded
(`_ = svr.Send(resp)`).

**Q23. Dead protobuf messages.** Ten messages from a removed multi-step
loader API remain in `api.proto` (3.3). `FetchHeadersResponse` is unrelated
to `FetchHeadersNotification`.

**Q24. Inconsistent loader errors.** The not-a-directory error has kind
`Invalid` in `CreateWatchingOnlyWallet` (`loader.go:130`) and no kind in
`CreateNewWallet` (`:220`), giving `InvalidArgument` vs `Unknown`. The
"already loaded" / "file exists" texts differ between the three entry points
(`wallet already loaded` / `wallet already opened`; `wallet already exists`
/ `wallet DB exists`). Paths and identifiers in several messages use Go
`%q` quoting.

**Q25. `isLoopback` edge cases.** `LOCALHOST`, `localhost.`, a bracketed
IPv6 literal without port, and host names that resolve to loopback are not
loopback; `127.x.y.z` of any value is. For a non-loopback address with an
empty certificate the syncer dials `wss://` with the system root pool
(spec 15).

**Q26. `errorCode` empty `case` arms.** The layout of `server.go:88-118`
reads like C-style fall-through (`IO` → `AlreadyExists`, `Encoding` →
`DataLoss`, `ScriptFailure`/`Policy`/`DoubleSpend`/`Protocol` →
`Unavailable`, `Bug` → `InvalidArgument`), but Go cases do not fall through:
those kinds yield `Unknown`. The implemented behaviour is authoritative.

**Q27. `GetRawBlock` defaults.** An empty request returns the genesis block.
A negative `block_height` (representable because the field is `sint32`)
reaches the main-chain lookup and fails there.

**Q28. `CloseWallet` conflates errors.** Any `UnloadWallet` error of kind
`Invalid` (including a bbolt "database not open" conversion,
`wallet/internal/bdb/db.go:26-27`) is reported as `Wallet is not loaded`.

**Q29. `RescanPoint` uses `FailedPrecondition` for every failure**,
including database errors, with a message lacking a separator before the
error.

**Q30. Secret hygiene is partial.** `RpcSync` copies `password` into a Go
string for the RPC options before zeroing the request buffer
(`server.go:2869`); public passphrases are never zeroed; `seed_bytes` of
`GenerateRandomSeed` is not zeroed. The zeroing acts on the decoded request
message, not on transport buffers.

**Q31. `RunTicketBuyer` VSP handling.** The VSP client is validated before
the balance; clients are cached by URL so a second call with a different
public key for the same URL silently reuses the first key; the fee and
change account policy of the first call persists as well. The maximum fee
comes from the wallet's start-up configuration, not the request.

**Q32. `RunAccountMixer` validates nothing.** A non-existent account or a
branch other than 0/1 is accepted; failures appear only in the log
(`Account mixing failed: …`) on each new block.

**Q33. `DecodeRawTransaction` ignores trailing bytes** and classifies
treasurybases and treasury spends as `COINBASE` (both have a single
null-outpoint input and the pre-treasury coinbase test is tried first,
`internal/compat/compat.go:19-27`) and treasury adds as `REGULAR`.

**Q34. Zero-length vs absent bytes.** The handlers' `!= nil` tests
(`server.go:3363`, `:3390`, `:3413`, `:3814`) behave as length tests only
because of the Go protobuf runtime's decoding of proto3 bytes. An in-process
Go caller could pass a non-nil empty slice and hit different branches; no
wire client can.

**Q35. `EncodeMnemonic` on an empty seed** yields a leading space
(`walletseed/seed.go:55-68`); unreachable through gRPC (minimum 16 bytes).

**Q36. Readiness failures are not logged**, while handler failures are
(5.2).

**Q37. The "already synchronizing" guard is neither atomic nor permanent.**
`server.go:2843-2846` and `:2993-2996` test the wallet's network backend,
which the syncer sets only once it is connected (`chain/sync.go:610`,
`spv/sync.go:396`) and clears on return. Concurrent sync RPCs can both pass;
the first syncer to exit clears the backend while the other still runs; and
with the main-package sync loop active the guard is open during its 5 s
retry sleeps (5.5.9). A port that takes a lock for the lifetime of the
stream changes which requests are rejected.

**Q38. Loader doc comments disagree with the code.**
(a) `onLoaded` (`loader.go:79-80`) says it "prevents loader from loading any
additional wallets"; it does not: after `UnloadWallet` another wallet can be
created or opened (the `UnloadWallet` comment itself says "The Loader may be
reused", `:373-374`). What `onLoaded` actually prevents is callbacks running
for later loads (Q3).
(b) `OpenExistingWallet` (`:284-287`) describes a `canConsolePrompt`
parameter that does not exist.
(c) `UnloadWallet` (`:371-373`) says it "stops the loaded wallet" (Q6) and
refers to a non-existent `LoadExistingWallet`.
(d) `RunAfterLoad` (`:91-93`) says functions are "executed in a single
goroutine in the order they are added"; a function added after a wallet is
loaded runs immediately on the caller's goroutine, outside the mutex, and
may therefore run concurrently with anything else.
(e) `CreateNewWallet` (`:195-197`) says a nil seed generates a random one;
true of `wallet.Create`, but the generated seed is never returned to the
caller, so such a wallet could not be backed up. No caller passes nil.
(f) `internal/loader/doc.go` says the package manages "services like ticket
buyer"; it manages none.

**Q39. Further `api.md` disagreements** (in addition to Q14 and Q20).

| Item | `api.md` | Code |
|---|---|---|
| Preamble | "authoritative source … Any divergence from this document is an implementation error" (`:16-17`) | The code is what existing clients run against; treat the document as informative only. |
| `WalletExists` errors | "None" (`:122`) | `Unknown` on a stat failure other than not-exist. |
| `RunTicketBuyer`, `RunAccountMixer` not loaded | `FailedPrecondition`: "Wallet has not been loaded" (`:2792`, `:2827`) | Code is `FailedPrecondition` but the message is `service walletrpc.<Service> is not ready` (from the interceptor); after `CloseWallet` there is no such failure at all (Q4). |
| `RunTicketBuyer` VSP / mixing errors | not listed | six `InvalidArgument` conditions (5.7). |
| `SetVoteChoices` errors | only `InvalidArgument` (`:2959-2962`) | also `NotFound` (unowned ticket) and `Unknown` (`no agendas to set for this network`). |
| `VerifyMessage` errors | only `InvalidArgument` (`:2994-2997`) | malformed signature → `Unknown` (Q18). |
| `GetRawBlock` errors | only `Unavailable` (`:3068-3070`) | also `InvalidArgument`, `NotFound`/other from the height lookup, `Internal`. |
| `RescanPoint` | "`FailedPrecondition`: The wallet or consensus RPC server has not been opened" (`:273`) | No consensus-server condition exists; every database error is also `FailedPrecondition` (Q29). |
| `SpvSync` errors | `FailedPrecondition` for "The string provided to spv_connect is not a valid address or port" (`:322-324`) | Matches in code; but host and port **content** is never validated (5.5.8 step 8), only that the string splits as `host:port`. |
| `RpcSync` `certificate` | TLS disabled for `localhost`, `127.0.0.1`, `::1` (`:349-351`) | Any `127.0.0.0/8` address; `::1` only as `[::1]:port` or bare `::1` (Q25). |
| `OpenWallet` text | "creating the wallet no longer automatically synchronizes" (`:219-220`, copied from `CreateWallet` `:141-142`) | Applies to opening. |
| `AgendaService` / `VoteChoices` "latest stake version supported" | (`:2832`, `:2842-2843`, `:2849`, `:2903`, `:2913`) | The version is a per-network constant in `wallet.CurrentAgendas`, not derived from the chain parameters' highest deployment (5.9). |

**Q40. `serviceName` assumes a well-formed method path.**
`rpcserver.go:399-404` slices `method[1:]` and then up to the next `/`
without checking for `-1`; a path without a second slash would panic. grpc-go
rejects such paths before invoking interceptors, answering `Unimplemented`
`malformed method name: "<path>"` (Go `%q`;
`grpc@v1.83.2/server.go:1787`, `:1830`), so the panic is unreachable, but a
Rust interceptor written against raw paths needs its own guard and the same
status.

**Q41. `Unknown` code used as an explicit status.** `RunAccountMixer`,
`RunTicketBuyer` and `RpcSync` construct `codes.Unknown` statuses directly
(`server.go:2562`, `:2682`, `:2978`) with a handler-authored prefix, where
other handlers use `translateError`. Wallet kinds inside `<err>` (for
example `Locked`, `Passphrase`, `NoPeers`) are therefore not mapped for these
three RPCs.

---

## 11. Upstream tests

There are **no tests** for `internal/rpc/rpcserver` or `internal/loader`.

| File | Pins | Port? |
|---|---|---|
| `walletseed/seed_test.go` | Four seed/mnemonic vectors (20, 20, 31 and 31 bytes) through `EncodeMnemonicSlice`, `EncodeMnemonic`, `DecodeUserInput` (mnemonic and hex). | Port verbatim. All four vector lengths (20, 20, 31, 31 bytes) are within the 16..64 range the gRPC path accepts. |
| `pgpwordlist/pgpwordlist_test.go` | Two 20-byte vectors through `DecodeMnemonics` and `ByteToMnemonic` (no checksum word). | Port verbatim. |
| `errors/errors_test.go` | `TestCollapse`, `TestIs`, `TestDoubleWrappedErrorWithKind`: collapsing and matching rules that determine status messages and codes. | Port with the errors crate (owned elsewhere); required before any status-message vector can pass. |
| `rpc/client/dcrwallet/marshaling_test.go` (`TestMarshalOutpointsToString`), `methods_test.go` (`TestSignRawTransactionErrors`) | JSON-RPC client marshaling and `SignatureErrors` formatting. | Not applicable to the daemon (spec 22). |
| `rpc/grpc_example/main.go:236-252` | Not a test, but seven testnet transactions (vote, P2PKH+P2PK, treasurybase with tadd, treasury spend with tgen, coinbase, legacy stake-pool ticket, solo ticket) exercised through `DecodeRawTransaction`. | Use as inputs; generate expected `DecodedTransaction` bytes with the oracle. |

Untested upstream and needing oracle-generated vectors:

1. Protobuf encoding of every response in 4.3 (golden bytes per message,
   including empty sub-messages and the negative enum).
2. `errorCode` for each error kind and for nested / collapsed errors;
   status-message strings of 8.1.
3. `CreateWallet` birthday bytes for the four flag combinations (read the
   value of key `birthstate` in bucket `wtxmgr` back from the created
   database).
4. `DecodeRawTransaction` for every `stdscript.ScriptType`, ticket
   commitment outputs (P2PKH and P2SH flavours), malformed scripts
   (`[error]` disassembly), no-witness and witness-only serializations,
   trailing bytes, empty input.
5. `DecodeSeed` error strings and white-space handling; mnemonic
   capitalisation.
6. `isLoopback` and `NormalizeAddress` tables.
7. `Agendas` / `VoteChoices` output for mainnet, testnet3, simnet.
8. Voting policy round trips, including per-ticket variants and the
   `abstain` / empty-string difference.
9. Loader state machine: create / open / close / reopen sequences and their
   status codes, including the stale-wallet behaviour of Q3.
10. Sync notification sequences for a fresh wallet and for an up-to-date
    wallet, under both syncers (integration test against a simnet dcrd and
    SPV peers), compared message-for-message with the Go wallet.
11. Wallet lock state after `RpcSync` with `discover_accounts` and after
    `RunTicketBuyer` / `RunAccountMixer` end.

---

## 12. Rust porting notes

### 12.1 Suggested boundaries

| Crate / module | Mirrors | Contents |
|---|---|---|
| `purse-walletrpc` | `rpc/walletrpc` | Message types and codecs for `api.proto` (shared with the sibling gRPC spec). Field tables of 4.3; golden-byte tests. |
| `purse-grpc` | `internal/rpc/rpcserver`, gRPC parts of `rpcserver.go` | HTTP/2 + gRPC framing server, interceptor equivalents (logging, readiness gate), status mapping (`error_code`), the service singletons with one-shot readiness, and the handlers. One module per service: `loader`, `mixer`, `ticketbuyer`, `seed`, `agenda`, `voting`, `msgverify`, `decode`, `network`. |
| `purse-loader` | `internal/loader` | `Loader` with the mutex discipline of 5.4, `run_after_load`, the three load paths and unload. Used by main, JSON-RPC and gRPC. |
| `purse-walletseed` (or a module of the wallet crate) | `walletseed`, `pgpwordlist` | Word table (copied verbatim), encode / decode, checksum. |

### 12.2 Reuse (crates looked at for this spec)

| Need | Candidate |
|---|---|
| Transaction decode, `TxHash`, block serialization | `dcroxide:crates/dcroxide-wire` (`MsgTx`, `MsgBlock`) |
| Script disassembly | `dcroxide:crates/dcroxide-txscript/src/script.rs:117` (`disasm_string`, returns the string and the error, matching the "ignore the error" usage) |
| Script classification, address extraction, required signatures | `dcroxide:crates/dcroxide-txscript/src/stdscript.rs:797-1157` (`determine_required_sigs`, `extract_addrs`) |
| Address decoding and types | `dcroxide:crates/dcroxide-txscript/src/stdaddr.rs:762` (`decode_address`) |
| Ticket commitment helpers, tx type | `dcroxide:crates/dcroxide-stake/src/lib.rs:392`, `:417`, `:1214`, `:1240` |
| Coinbase test | `dcroxide:crates/dcroxide-standalone/src/tx.rs:40` (`is_coin_base_tx`) |
| Compact signature recovery | `dcroxide:crates/dcroxide-dcrec/src/secp256k1/ecdsa.rs:435` (`recover_compact`); check that its error texts match 8.1 |
| Deployments / vote definitions | `dcroxide:crates/dcroxide-chaincfg/src/lib.rs:59-176` (`Choice`, `Vote`, `ConsensusDeployment`, `deployments`); confirm the tables equal chaincfg v3.3.0 (mainnet v11, testnet3 v12, simnet v12) |
| TLS | `dcroxide:crates/dcroxide-tlscrypto` (rustls provider) and the node's `rpctls.rs`; client-certificate verification must be added for the gRPC listener |
| SPV address manager | `dcroxide:crates/dcroxide-addrmgr` |
| HD seeds | `dcr-rs:src/hd.rs` implements BIP32 with BIP39 mnemonics (`from_phrase`, `:249`); it has **no** PGP word list, so the seed mnemonic code is new |

dcroxide has no HTTP/2, gRPC or protobuf code and no async runtime
dependency (its RPC server is an in-tree HTTP/1.1 + WebSocket stack over
rustls). The gRPC transport is therefore new work and a dependency decision
(13.D3).

### 12.3 Hazards

1. **Header timing.** grpc-go sends response headers with the first message
   and uses trailers-only responses for immediate errors. A typical Rust
   gRPC server sends headers as soon as a streaming handler returns its
   stream. To match, validate and run the pre-stream steps before producing
   the stream, and for `RunTicketBuyer` / `RunAccountMixer` never emit
   headers before the terminal status.
2. **Presence of empty sub-messages.** Progress and peer payloads must be
   encoded even when all fields are zero (`22 00`, `2a 00`, `32 00`).
3. **Back-pressure, not buffering.** Notifications must block the syncer
   when the client is slow; an unbounded queue would change memory
   behaviour, a bounded lossy queue would drop notifications.
4. **Go `%q` and `%v` formatting** in status messages (`unknown policy
   "…"`, `no agenda with ID "…"`, `"<path>" is not a directory`): Go's
   quoting rules differ from Rust's `{:?}` for non-ASCII and control
   characters.
5. **Error strings from lower layers** (`encoding/hex`, `net.SplitHostPort`,
   `wire` deserialization, `os` errors, base64, URL parsing) leak into
   status messages. Byte-exact parity for these is a large surface; decide
   the tolerance (13.D4).
6. **Map-order behaviours** (Q16): choose a deterministic order and document
   it; do not try to imitate randomness.
7. **The stale-wallet bug (Q3) and the unlock quirk (Q5)** are easy to "fix"
   accidentally by structuring the code sensibly. Whatever is decided must
   be deliberate and tested.
8. **Unicode handling in `DecodeSeed`**: Go `TrimSpace` / `ToLower` are
   Unicode-aware; splitting is on U+0020 only.
9. **`time.Unix` / zero-time arithmetic** for the birthday bytes: the
   stored time is the constant `0xFFFFFFF1886E0900`, not 0.
10. **Loader mutex release during open** (Q9) and the "remove without close"
    path (Q8) interact with file locking differently per platform.
11. **4 MiB inbound limit** and the transport's error text for exceeding it.
12. **Zeroing secrets** requires owning the decoded request buffers; a codec
    that hands out borrowed slices of the HTTP/2 frame buffer changes what
    is wiped.

### 12.4 Size estimate

**L.** The handlers themselves are small (about 1,300 lines of Go for nine
services plus 415 for the loader), and most heavy lifting is in subsystems
owned by other specs. What makes it large is the absence of any reusable
gRPC stack in the sister projects (HTTP/2 server with mutual TLS, gRPC
framing, protobuf codec for the 222 message definitions of `api.proto`,
nested ones included, across both gRPC specs),
the complete lack of upstream tests (every vector must come from an oracle),
and the integration surface (loader lifecycle, two syncers, ticket buyer,
wallet locking). If the transport and message crate are counted under the
sibling gRPC spec, the remainder here is **M**.

---

## 13. Open questions

### 13.1 Unverified

1. The nested wallet-error strings in 8.1 and 5.8.2 were derived by reading
   `errors.E` / `Error()`, not observed. Settle with a Go oracle shim that
   calls each handler against a scratch wallet and records code + message.
2. Windows behaviour of the failed-create cleanup (Q8) and of a blocked
   concurrent open (Q9). Settle by running the Go wallet on Windows.
3. Whether any shipping client (Decrediton in particular) depends on (a) the
   wallet staying unlocked after `RunTicketBuyer` / `RunAccountMixer`, (b)
   header timing on the long-lived streams, (c) the `CloseWallet` →
   `OpenWallet` sequence within one process (Q3), (d) the order of
   `TreasuryPolicies` / `TSpendPolicies` results, (e) substrings of status
   messages. Settle by reading the Decrediton sources at its current
   release.
4. grpc-go's keepalive defaults apply (server ping after 2 h idle, 20 s
   ping timeout, clients may ping at most once per 5 minutes and, by
   default, not without active streams;
   `grpc@v1.83.2/internal/transport/defaults.go:31-40`). Whether
   Decrediton's channel options stay inside these limits was not checked; a
   replacement transport with different enforcement could change which
   clients get disconnected with `GOAWAY`. Settle by reading Decrediton's
   channel options.
5. The transport-generated statuses (oversize message,
   `grpc@v1.83.2/rpc_util.go:798`; unknown method / service,
   `server.go:1866-1875`) were read but not exercised; other
   transport-level statuses (malformed frames, unsupported compression,
   deadline parsing) were not enumerated.
6. Behaviour of `GetRawBlock` under the offline network backend (error kind
   and text) was not traced.
7. The `wire` deserialization error texts for `DecodeRawTransaction`
   (`Could not decode Tx: …`) were not enumerated; they come from
   `dcrd/wire@v1.8.0` and should be compared with dcroxide-wire's.

8. Behaviour of two syncers running concurrently against one wallet (5.5.9
   item 2, Q37) was not traced beyond the backend pointer; what the wallet
   does with interleaved header and rescan writes belongs to specs 09, 14
   and 15. Settle by issuing two `RpcSync` calls against a Go wallet whose
   dcrd endpoint accepts TCP but delays the WebSocket upgrade.
### 13.2 Decisions needed

- **D1. `CreateWallet` birthday bug (Q1):** reproduce (bit-compatible
  database contents and rescan range) or fix (honour `birth_time` /
  `birth_height` as documented). Fixing changes the bytes written to
  `wallet.db` for the same request.
- **D2. Lock state after ticket buyer / mixer RPCs (Q5):** reproduce
  (wallet stays unlocked) or lock as the code evidently intended.
- **D3. gRPC transport stack:** adopt an existing HTTP/2 + gRPC + protobuf
  stack (which brings an async runtime the sister project has avoided) or
  write a minimal in-tree HTTP/2 server and protobuf codec (only unary and
  server-streaming calls, proto3 scalars, bytes, strings, enums, nested and
  repeated messages are needed; the sibling spec adds one bidirectional
  stream).
- **D4. Fidelity of status messages** that embed Go standard-library or
  third-party error text: byte-exact, or exact code plus exact
  wallet-authored prefix with best-effort tail.
- **D5. Stale wallet after reopen (Q3) and never-revoked readiness (Q4):**
  reproduce, or rebind services on every load and reject calls while no
  wallet is loaded.
- **D6. Deterministic ordering** for `TSpendPolicies` / `TreasuryPolicies`
  (Q16): pick and document (for example ascending by hash / key, wallet-wide
  before per-ticket).
- **D7. Proxy bypass for gRPC-started syncers (Q13):** reproduce (traffic
  leaves outside the configured proxy) or apply the configured dialer. This
  is a privacy-relevant divergence either way.
- **D8. `commitment_amount` (Q2) and `STAKE_SUBMISSION` for commitment
  outputs (Q10):** reproduce or fix; clients may already compensate.
- **D10. Sync-RPC exclusion (Q37):** reproduce the non-atomic guard, or
  hold an exclusive "sync in progress" state from the precondition check to
  the end of the stream. The latter rejects requests the Go wallet accepts
  only in race windows.
- **D9. Whether to ship corrected API documentation** (`api.md` at the true
  version 10.0.0) with the replacement, given Q14, Q20 and Q21.
