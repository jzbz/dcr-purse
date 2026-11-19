# 19 — gRPC API: transport, VersionService and WalletService

## 1. Header

**Scope** (dcrwallet, line counts from `wc -l`):

| File | Lines | Covered here |
|---|---|---|
| `rpc/api.proto` | 1341 | file-level declarations; `VersionService` and `WalletService` (`:7-92`); every message those two services use, including the shared messages `TransactionDetails`, `BlockDetails`, `DetachedBlockDetails` (`:137-181`). Messages of the other nine services belong to the sibling gRPC spec. |
| `rpc/documentation/api.md` | 3070 | preamble and `VersionService` (`:1-82`), `WalletService` and its shared messages (`:409-2257`, `:2466-2706`) |
| `rpc/documentation/serverchanges.md` | 94 | all |
| `internal/rpc/rpcserver/server.go` | 4312 | plumbing (`:1-360`), every `walletServer` method (`:361-2509`, `:3727-3786`, `:3848-4312`), `scProto` (`:3664-3698`), every `StartXxxService` function and `checkReady` method (`:2512-2529`, `:2568-2586`, `:2688-2692`, `:3207-3209`, `:3242-3258`, `:3519-3521`, `:3550-3552`, `:3789-3805`) |
| `internal/rpc/rpcserver/log.go` | 81 | all |
| `rpcserver.go` (root, package `main`) | 512 | gRPC parts: `:248-347` (server construction), `:397-448` (interceptors). TLS key/cert generation and JSON-RPC wiring are specified in spec 16 and only referenced here. |

Supporting code read to pin observable behaviour: `errors/errors.go` (all),
`dcrwallet.go:36-39`, `:57-66`, `:373-411`, `log.go:38`, `:54`,
`internal/loggers/loggers.go:52`, `internal/netparams/params.go`,
`internal/loader/loader.go:79-104`, `config.go:868-956`, `ipc.go:149-177`,
`wallet/notifications.go` (all), `wallet/wallet.go:1712-1849` (unlock and
lock), `wallet/rescan.go:212-443`, `wallet/utxos.go:22-87`,
`wallet/udb/txquery.go:388-538`, `:640-725`, `wallet/udb/txmined.go:3098-3345`,
and the signatures of every `*wallet.Wallet` method the handlers call.
`rpc/walletrpc/api_grpc.pb.go` and `api.pb.go` (generated) were consulted for
method paths and generated names, and the TLS set-up portions of
`rpc/documentation/clientusage.md` (460 lines, otherwise out of scope) for
the client side of the handshake.

**Parity pin**

| Component | Version |
|---|---|
| dcrwallet | master `54b85b33`, version 2.2.0-pre, module `decred.org/dcrwallet/v5` |
| `google.golang.org/grpc` | v1.83.2 (`server.go`, `status/status.go`, `internal/status/status.go`, `credentials/tls.go`, `grpclog/`, `internal/transport/defaults.go` read) |
| `google.golang.org/protobuf` | v1.36.11 (`internal/impl/codec_gen.go`, `codec_message.go` read) |
| generated code | `protoc-gen-go` v1.36.10, `protoc-gen-go-grpc` v1.5.1, `protoc` v6.33.0 (`rpc/walletrpc/api.pb.go:1-5`, `api_grpc.pb.go:1-5`) |
| `github.com/decred/dcrd/chaincfg/chainhash` | v1.0.5 |
| `github.com/decred/dcrd/dcrutil/v4` | v4.0.3 |
| `github.com/decred/dcrd/hdkeychain/v3` | v3.1.3 |
| `github.com/decred/dcrd/txscript/v4` | v4.1.2 |
| `github.com/decred/dcrd/gcs/v4` | v4.1.1 |
| `github.com/decred/dcrd/wire` | v1.8.0 |

**Summary**

dcrwallet serves a gRPC API (proto3 package `walletrpc`, eleven services) over
HTTP/2 with mandatory mutual TLS. The API's own semantic version is the
constant `10.0.0`, reported by `VersionService.Version`; the prose
documentation still says 7.13.x. Two interceptors log each call and reject
calls to a service whose dependencies are not yet attached with
`FailedPrecondition`. `WalletService` has 65 RPCs (57 unary, 7
server-streaming, 1 bidirectional) and becomes ready when a wallet is loaded.
Handlers are thin: they validate a few request fields, optionally unlock the
wallet for the duration of the call, invoke `*wallet.Wallet` methods and copy
results into response messages. Wallet errors are converted to gRPC status
codes by a small kind-to-code table whose message is the full nested error
string; roughly a dozen handlers skip that table and return raw errors, which
the gRPC library reports as `Unknown`. The documentation disagrees with the
code in many places (error codes, ignored fields, undocumented RPCs); the code
is normative and every disagreement found is listed in section 10.

## 2. Role in the system

The gRPC server is one of the wallet's two client-facing RPC surfaces (the
other is JSON-RPC, spec 16). Its consumers are programs written against
stubs generated from `rpc/api.proto` (the project brief names Decrediton);
the Go example in `rpc/documentation/clientusage.md:66-93` shows the
intended client set-up: the server's `rpc.cert` as the only trusted root and
a client key pair whose certificate is listed in `clients.pem`.

Callers of this subsystem:

- `main.startRPCServers` (`rpcserver.go:248`) builds the `grpc.Server`,
  registers all services and starts the wallet-independent ones.
- `main.run` (`dcrwallet.go:385-399`) registers a loader callback that starts
  the five wallet-dependent services when a wallet is created or opened, and
  defers `Stop()`.
- `main` logging setup (`log.go:38`) installs the gRPC library logger.

What the handlers call:

- `*wallet.Wallet` (specs 06 to 08, 10, 11): every data access and mutation.
- `wallet.NetworkBackend`, obtained per call from `Wallet.NetworkBackend()`
  (`wallet/network.go:61`): passed back into wallet methods that publish,
  rescan or discover. Two concrete types are inspected: `*chain.Syncer`
  (dcrd JSON-RPC mode, spec 15) and `*spv.Syncer` (SPV mode, spec 14).
- `wallet.NotificationServer` (`wallet/notifications.go`): the three
  notification streams.
- `*wallet.VSPClient` (spec 11): VSP fee processing RPCs.
- Decred primitives: `stdaddr`, `stdscript`, `chainhash`, `wire`,
  `hdkeychain`, `dcrutil`, `txscript`, `gcs`.

The subsystem owns no persistent state. Its in-memory state is the set of
singleton service objects and their `ready` flags.

## 3. Public surface inventory

### 3.1 Package `rpcserver` (`internal/rpc/rpcserver`)

Exported:

| Name | Signature | Contract |
|---|---|---|
| `RegisterServices` | `func(server *grpc.Server)` (`server.go:274`) | Registers the eleven singleton service implementations, in the order Version, Wallet, WalletLoader, Seed, AccountMixer, TicketBuyer, Agenda, Voting, MessageVerification, DecodeMessage, Network. Registration does not make a service ready. |
| `ServiceReady` | `func(service string) error` (`:303`) | `nil` when the named service (`walletrpc.<Service>`) is ready. Unknown name: `Unimplemented`, `service <name> not found`. Known but not ready: `FailedPrecondition`, `service <name> is not ready`. A service whose type has no `checkReady` method is always ready. |
| `StartWalletService` | `func(wallet *wallet.Wallet)` (`:332`) | Under the write lock, panics with `service already started` if already ready; otherwise sets `ready = true` and stores the wallet. |
| `StartWalletLoaderService` | `func(loader *loader.Loader, activeNet *netparams.Params)` (`:2512`) | Stores both fields, no lock, no ready flag. |
| `StartAccountMixerService` | `func(wallet *wallet.Wallet)` (`:2518`) | Same shape as `StartWalletService`. |
| `StartTicketBuyerService` | `func(wallet *wallet.Wallet)` (`:2575`) | Same shape. |
| `StartAgendaService` | `func(activeNet *chaincfg.Params)` (`:3207`) | Stores the params. |
| `StartVotingService` | `func(wallet *wallet.Wallet)` (`:3242`) | Same shape as `StartWalletService`. |
| `StartMessageVerificationService` | `func(chainParams *chaincfg.Params)` (`:3519`) | Stores the params. |
| `StartDecodeMessageService` | `func(chainParams *chaincfg.Params)` (`:3550`) | Stores the params. |
| `StartNetworkService` | `func(wallet *wallet.Wallet)` (`:3789`) | Same shape as `StartWalletService`. |
| `UseLogger` | `func(l slog.Logger)` (`log.go:27`) | Installs `logger{l}` as the gRPC library's (v1) logger. |

Unexported, with observable behaviour:

| Name | Location | Contract |
|---|---|---|
| `semverString`, `semverMajor`, `semverMinor`, `semverPatch` | `server.go:63-68` | `"10.0.0"`, 10, 0, 0. |
| `scriptVersionAssumed` | `:72` | 0; script version used by `ImportScript`. |
| `translateError` | `:80` | `status.Errorf(errorCode(err), "%s", err.Error())`. Section 5.3. |
| `errorCode` | `:85` | Kind-to-code table. Section 5.3. |
| `decodeAddress` | `:141` | `stdaddr.DecodeAddress(a, params)`; failure is `InvalidArgument`, `invalid address <a>: <err>`. |
| `decodeHashes` | `:149` | Converts each element with `chainhash.NewHash`; first failure is `InvalidArgument`, `hash (hex <lowercase hex of the element>): <err>`. |
| `versionServer`, `walletServer` | `:163`, `:168` | Service implementations. `walletServer` holds `ready bool`, `readyMtx sync.RWMutex`, `wallet *wallet.Wallet`. |
| embedded `pb.UnimplementedVersionServiceServer`, `pb.UnimplementedWalletServiceServer` | `:164`, `:175` | Generated fallback implementations embedded by value in the two service structs (each of the other nine structs embeds its own). A proto method with no handler method would answer `Unimplemented`, `method <Name> not implemented` (`rpc/walletrpc/api_grpc.pb.go:1071-1073`). All 65 `WalletService` methods and `Version` are overridden (checked by diffing the `rpc` lines of `api.proto:21-92` against the `(*walletServer)` methods), so the fallback is never reached at the pin. |
| `loaderServer`, `seedServer`, `accountMixerServer`, `ticketbuyerServer`, `agendaServer`, `votingServer`, `messageVerificationServer`, `decodeMessageServer`, `networkServer` | `:180-254` | Sibling spec. Listed because `RegisterServices` and `serviceMap` name them. `accountMixerServer`, `ticketbuyerServer`, `votingServer` and `networkServer` carry the same `ready`/`readyMtx`/`wallet` triple and a `checkReady` method; the others have no ready gate. |
| the eleven singletons | `:258-270` | Package-level values; one per service for the life of the process. |
| `serviceMap` | `:288-300` | Map from `walletrpc.<Service>` to the singleton, used by `ServiceReady`. Keys: `VersionService`, `WalletService`, `WalletLoaderService`, `SeedService`, `AccountMixerService`, `TicketBuyerService`, `AgendaService`, `VotingService`, `MessageVerificationService`, `DecodeMessageService`, `NetworkService`, each prefixed `walletrpc.`. |
| `(*walletServer).checkReady` | `:344` | Returns `ready` under the read lock. |
| `(*walletServer).requireNetworkBackend` | `:352` | `wallet.NetworkBackend()`; any error becomes `FailedPrecondition`, `wallet is not associated with a consensus server RPC client`. |
| `zero` | `:491` | Overwrites a byte slice with zeros. |
| `scriptChangeSource`, `makeScriptChangeSource` | `:936-960` | Change source paying a decoded address; `Script()` returns `(script, version, nil)`, `ScriptSize()` returns `len(script)`. |
| `sumOutputValues` | `:962` | Sum of `TxOut.Value`. |
| `decodeDestination` | `:1142` | Section 5.4.5. |
| `txChangeSource`, `makeTxChangeSource` | `:1166-1190` | Change source from an `OutputDestination`. |
| `blockRangeReq`, `decodeBlockRange` | `:1323-1386` | Section 5.4.4. |
| `marshalTransactionInputs`, `marshalTransactionOutputs`, `marshalTxType`, `marshalTransactionDetails`, `marshalTransactionDetailsSlice`, `marshalTicketDetails`, `marshalGetTicketBlockDetails`, `marshalBlock`, `marshalBlocks`, `marshalDetachedBlock`, `marshalDetachedBlocks`, `marshalHeaderHashes`, `marshalHashes`, `marshalVSPTrackedTickets` | `:2212-2386`, `:4254` | Section 5.4.6. |
| `scProto` | `:3664` | Script type to `ScriptType` enum number. Section 4.7. |
| `logger` and its methods, `stripGrpcPrefix`, `stripGrpcPrefixArgs` | `log.go:32-81` | Section 5.2.3. |

### 3.2 Package `main` (`rpcserver.go`, `dcrwallet.go`)

| Name | Location | Contract |
|---|---|---|
| `startRPCServers` | `rpcserver.go:248` | Builds both RPC servers. gRPC part in section 5.1. |
| `serviceName` | `:399` | From `/pkg.Service/Method` returns `pkg.Service` (drops the first byte, keeps everything before the next `/`). |
| `interceptStreaming` | `:406` | Section 5.2.2. |
| `interceptUnary` | `:432` | Section 5.2.1. |
| `done` | `dcrwallet.go:59` | Non-blocking test of `ctx.Done()`. |
| `listenFunc`, `makeListeners` | `rpcserver.go:450`, `:455` | Shared with JSON-RPC and specified in spec 16 section 5.1.5. For gRPC it is called with plain `net.Listen`: each normalized address is classified by its host (empty host: both families; an IPv6 zone suffix `%...` is stripped before parsing; a host that is not an IP literal is skipped with warn `` `<host>` is not a valid IP address ``; an address that does not split into host and port is skipped with error `` `<addr>` is not a normalized listener address ``), then all `tcp4` listeners are opened before all `tcp6` listeners, each failure logged as warn `Can't listen on <addr>: <err>` and skipped. |
| `grpcListenerEventServer`, `newGRPCListenerEventServer`, `grpcListenerEvent` | `ipc.go:149-177` | Pipe notifier for bound gRPC addresses (message type `grpclistener`, payload the UTF-8 address string; framing in spec 21). `notify` on the nil notifier (no `--rpclistenerevents`) does nothing. |

### 3.3 RPC inventory

`VersionService` (`api.proto:7-9`): one unary RPC, `Version`.

`WalletService` (`api.proto:21-92`). Kinds: U unary, SS server-streaming,
BIDI bidirectional streaming. The full method path is
`/walletrpc.WalletService/<Name>` (`rpc/walletrpc/api_grpc.pb.go:124-188`).
"Doc" is whether `rpc/documentation/api.md` has a section for the method.

| # | RPC | Kind | Request → Response | Doc | Section |
|---|---|---|---|---|---|
| 1 | `Ping` | U | `PingRequest` → `PingResponse` | yes | 5.6.1 |
| 2 | `Network` | U | `NetworkRequest` → `NetworkResponse` | yes | 5.6.2 |
| 3 | `CoinType` | U | `CoinTypeRequest` → `CoinTypeResponse` | yes | 5.6.3 |
| 4 | `AccountNumber` | U | `AccountNumberRequest` → `AccountNumberResponse` | yes | 5.6.4 |
| 5 | `Accounts` | U | `AccountsRequest` → `AccountsResponse` | yes | 5.6.5 |
| 6 | `Address` | U | `AddressRequest` → `AddressResponse` | yes | 5.6.6 |
| 7 | `Balance` | U | `BalanceRequest` → `BalanceResponse` | yes | 5.6.7 |
| 8 | `GetAccountExtendedPubKey` | U | `GetAccountExtendedPubKeyRequest` → `...Response` | yes | 5.6.8 |
| 9 | `GetAccountExtendedPrivKey` | U | `GetAccountExtendedPrivKeyRequest` → `...Response` | yes | 5.6.9 |
| 10 | `GetTransaction` | U | `GetTransactionRequest` → `GetTransactionResponse` | yes | 5.6.10 |
| 11 | `GetTransactions` | SS | `GetTransactionsRequest` → stream `GetTransactionsResponse` | yes | 5.6.11 |
| 12 | `GetTicket` | U | `GetTicketRequest` → `GetTicketsResponse` | yes | 5.6.12 |
| 13 | `GetTickets` | SS | `GetTicketsRequest` → stream `GetTicketsResponse` | yes | 5.6.13 |
| 14 | `TicketPrice` | U | `TicketPriceRequest` → `TicketPriceResponse` | yes | 5.6.14 |
| 15 | `StakeInfo` | U | `StakeInfoRequest` → `StakeInfoResponse` | yes | 5.6.15 |
| 16 | `BlockInfo` | U | `BlockInfoRequest` → `BlockInfoResponse` | yes | 5.6.16 |
| 17 | `BestBlock` | U | `BestBlockRequest` → `BestBlockResponse` | yes | 5.6.17 |
| 18 | `Spender` | U | `SpenderRequest` → `SpenderResponse` | no | 5.6.18 |
| 19 | `GetCFilters` | SS | `GetCFiltersRequest` → stream `GetCFiltersResponse` | yes | 5.6.19 |
| 20 | `GetPeerInfo` | U | `GetPeerInfoRequest` → `GetPeerInfoResponse` | no | 5.6.20 |
| 21 | `DumpPrivateKey` | U | `DumpPrivateKeyRequest` → `DumpPrivateKeyResponse` | yes | 5.6.21 |
| 22 | `BirthBlock` | U | `BirthBlockRequest` → `BirthBlockResponse` | yes | 5.6.22 |
| 23 | `TransactionNotifications` | SS | `TransactionNotificationsRequest` → stream `...Response` | yes | 5.6.23 |
| 24 | `AccountNotifications` | SS | `AccountNotificationsRequest` → stream `...Response` | yes | 5.6.24 |
| 25 | `ConfirmationNotifications` | BIDI | stream `ConfirmationNotificationsRequest` → stream `...Response` | yes | 5.6.25 |
| 26 | `ChangePassphrase` | U | `ChangePassphraseRequest` → `ChangePassphraseResponse` | yes | 5.6.26 |
| 27 | `RenameAccount` | U | `RenameAccountRequest` → `RenameAccountResponse` | yes | 5.6.27 |
| 28 | `Rescan` | SS | `RescanRequest` → stream `RescanResponse` | yes | 5.6.28 |
| 29 | `NextAccount` | U | `NextAccountRequest` → `NextAccountResponse` | yes | 5.6.29 |
| 30 | `NextAddress` | U | `NextAddressRequest` → `NextAddressResponse` | yes | 5.6.30 |
| 31 | `ImportPrivateKey` | U | `ImportPrivateKeyRequest` → `ImportPrivateKeyResponse` | yes | 5.6.31 |
| 32 | `ImportExtendedPublicKey` | U | `ImportExtendedPublicKeyRequest` → `...Response` | no | 5.6.32 |
| 33 | `ImportVotingAccountFromSeed` | U | `ImportVotingAccountFromSeedRequest` → `...Response` | yes | 5.6.33 |
| 34 | `ImportScript` | U | `ImportScriptRequest` → `ImportScriptResponse` | yes | 5.6.34 |
| 35 | `FundTransaction` | U | `FundTransactionRequest` → `FundTransactionResponse` | yes | 5.6.35 |
| 36 | `UnspentOutputs` | SS | `UnspentOutputsRequest` → stream `UnspentOutputResponse` | yes | 5.6.36 |
| 37 | `ConstructTransaction` | U | `ConstructTransactionRequest` → `...Response` | yes | 5.6.37 |
| 38 | `SignTransaction` | U | `SignTransactionRequest` → `SignTransactionResponse` | yes | 5.6.38 |
| 39 | `SignTransactions` | U | `SignTransactionsRequest` → `SignTransactionsResponse` | yes | 5.6.39 |
| 40 | `CreateSignature` | U | `CreateSignatureRequest` → `CreateSignatureResponse` | yes | 5.6.40 |
| 41 | `PublishTransaction` | U | `PublishTransactionRequest` → `PublishTransactionResponse` | yes | 5.6.41 |
| 42 | `PublishUnminedTransactions` | U | `PublishUnminedTransactionsRequest` → `...Response` | yes | 5.6.42 |
| 43 | `PurchaseTickets` | U | `PurchaseTicketsRequest` → `PurchaseTicketsResponse` | yes | 5.6.43 |
| 44 | `LoadActiveDataFilters` | U | `LoadActiveDataFiltersRequest` → `...Response` | yes | 5.6.44 |
| 45 | `SignMessage` | U | `SignMessageRequest` → `SignMessageResponse` | yes | 5.6.45 |
| 46 | `SignMessages` | U | `SignMessagesRequest` → `SignMessagesResponse` | yes | 5.6.46 |
| 47 | `ValidateAddress` | U | `ValidateAddressRequest` → `ValidateAddressResponse` | yes | 5.6.47 |
| 48 | `CommittedTickets` | U | `CommittedTicketsRequest` → `CommittedTicketsResponse` | yes | 5.6.48 |
| 49 | `SweepAccount` | U | `SweepAccountRequest` → `SweepAccountResponse` | yes | 5.6.49 |
| 50 | `AbandonTransaction` | U | `AbandonTransactionRequest` → `AbandonTransactionResponse` | yes | 5.6.50 |
| 51 | `SignHashes` | U | `SignHashesRequest` → `SignHashesResponse` | yes | 5.6.51 |
| 52 | `GetCoinjoinOutputspByAcct` | U | `GetCoinjoinOutputspByAcctRequest` → `...Response` | no | 5.6.52 |
| 53 | `SetAccountPassphrase` | U | `SetAccountPassphraseRequest` → `...Response` | no | 5.6.53 |
| 54 | `UnlockAccount` | U | `UnlockAccountRequest` → `UnlockAccountResponse` | yes | 5.6.54 |
| 55 | `LockAccount` | U | `LockAccountRequest` → `LockAccountResponse` | yes | 5.6.55 |
| 56 | `UnlockWallet` | U | `UnlockWalletRequest` → `UnlockWalletResponse` | yes | 5.6.56 |
| 57 | `LockWallet` | U | `LockWalletRequest` → `LockWalletResponse` | yes | 5.6.57 |
| 58 | `AccountUnlocked` | U | `AccountUnlockedRequest` → `AccountUnlockedResponse` | no | 5.6.58 |
| 59 | `SyncVSPFailedTickets` | U | `SyncVSPTicketsRequest` → `SyncVSPTicketsResponse` | no | 5.6.59 |
| 60 | `GetVSPTicketsByFeeStatus` | U | `GetVSPTicketsByFeeStatusRequest` → `...Response` | no | 5.6.60 |
| 61 | `ProcessManagedTickets` | U | `ProcessManagedTicketsRequest` → `...Response` | no | 5.6.61 |
| 62 | `ProcessUnmanagedTickets` | U | `ProcessUnmanagedTicketsRequest` → `...Response` | no | 5.6.62 |
| 63 | `SetVspdVoteChoices` | U | `SetVspdVoteChoicesRequest` → `...Response` | no | 5.6.63 |
| 64 | `GetTrackedVSPTickets` | U | `GetTrackedVSPTicketsRequest` → `...Response` | yes | 5.6.64 |
| 65 | `DiscoverUsage` | U | `DiscoverUsageRequest` → `DiscoverUsageResponse` | no | 5.6.65 |

Note the request/response type names that do not follow the method name:
`GetTicket` returns `GetTicketsResponse`; `UnspentOutputs` streams
`UnspentOutputResponse` (singular); `SyncVSPFailedTickets` uses
`SyncVSPTicketsRequest`/`SyncVSPTicketsResponse`.

## 4. Data formats

### 4.1 Transport

| Layer | Value | Source |
|---|---|---|
| TCP listeners | `--grpclisten` addresses; default `localhost` resolved addresses on the network's gRPC port (mainnet 9111, testnet3 19111, simnet 19558); one listener per address, IPv4 (`tcp4`) listeners created before IPv6 (`tcp6`) | `config.go:874-881`, `:902-903`, `internal/netparams/params.go:25`, `:34`, `:43`, `rpcserver.go:455-512` |
| TLS | Mandatory. Server certificate and key from `--rpccert`/`--rpckey` (spec 16 section 5.1). Minimum version TLS 1.2. Client certificate required and verified against the client CA pool (`tls.RequireAndVerifyClientCert`). | `rpcserver.go:272-276`, `:321-322` |
| ALPN | `h2` is appended to the server's protocol list by the gRPC credentials; a client that negotiates no ALPN protocol is rejected (`GRPC_ENFORCE_ALPN_ENABLED` defaults to true) | `grpc@v1.83.2/credentials/tls.go:176-184`, `:239` |
| HTTP/2 | Standard gRPC-over-HTTP/2: request `:method POST`, `:path /walletrpc.<Service>/<Method>`, `content-type: application/grpc` (optionally `+proto`), `te: trailers` | gRPC protocol; paths from `rpc/walletrpc/api_grpc.pb.go:22`, `:124-188` |
| Message framing | Each message: 1 byte compressed flag (0), 4 bytes big-endian length, then the protobuf encoding | gRPC protocol |
| Status | Trailers `grpc-status: <decimal code>` and `grpc-message: <percent-encoded UTF-8>`. No status in this subsystem carries details, so `grpc-status-details-bin` is never sent. | section 5.3 |
| Max received message | 4 MiB (4,194,304 bytes); the library rejects larger requests with `ResourceExhausted` | `grpc@v1.83.2/server.go:61`, `:190` |
| Max sent message | 2^31 − 1 bytes | `grpc@v1.83.2/server.go:62` |
| Other server options | None. Only `grpc.Creds`, `grpc.StreamInterceptor`, `grpc.UnaryInterceptor` are passed. No keepalive policy, stream limit, reflection, health or channelz service is configured. Library defaults apply: connection handshake timeout 120 s, server keepalive ping after 2 h idle with 20 s timeout, minimum client ping interval 5 min, initial HTTP/2 window 65,535, max header list 16 MiB. | `rpcserver.go:328-332`, `grpc@v1.83.2/server.go:192`, `internal/transport/defaults.go:28-48` |
| Client keepalive enforcement | Library default policy (no `KeepaliveEnforcementPolicy` option is passed): a client HTTP/2 PING is a strike when it arrives less than 2 h after the previous PING while the connection has no active stream, or less than 5 min after the previous PING while it has at least one. The strike counter is reset whenever the server has written header or data frames since the last PING. On the third strike (`pingStrikes > 2`) the server sends GOAWAY with error code `ENHANCE_YOUR_CALM` and debug data `too_many_pings` and closes the connection. Every PING is acknowledged before the check. | `grpc@v1.83.2/internal/transport/http2_server.go:880-931`, `internal/transport/defaults.go:38-40` |
| Compressed requests | A request whose `grpc-encoding` is neither absent nor `identity` and names a compressor that is not registered is answered `Unimplemented`, `grpc: Decompressor is not installed for grpc-encoding "<name>"`. Which names are registered is the UNVERIFIED item in the next row. | `grpc@v1.83.2/server.go:1360-1369`, `:1686-1694` |
| Compression | dcrwallet's own packages import no gRPC compressor. UNVERIFIED whether a transitive dependency registers gzip; settle by listing the imports of the built binary for `google.golang.org/grpc/encoding/gzip`. | grep of the repository |

A request for a service or method that is not registered never reaches the
interceptors: the library answers `Unimplemented` with `unknown service
<service>` or `unknown method <method> for service <service>`
(`grpc@v1.83.2/server.go:1866-1875`).

### 4.2 `api.proto` file-level declarations

```
syntax = "proto3";
package walletrpc;
option go_package = "decred.org/dcrwallet/v5/rpc/walletrpc";
```

There are no imports and no other options. The proto package `walletrpc` is
part of every method path and of every fully-qualified message name; it must
not change. Services are declared in this order: `VersionService`,
`WalletService`, `WalletLoaderService`, `AccountMixerService`,
`TicketBuyerService`, `SeedService`, `AgendaService`, `VotingService`,
`MessageVerificationService`, `NetworkService`, and (after the messages, at
`:1003`) `DecodeMessageService`.

The Rust port should vendor `rpc/api.proto` byte for byte and generate from
it. The tables below are the normative inventory for the two services in
scope; where they disagree with the vendored file, the file wins.

### 4.3 Encoding rules that affect byte parity

1. All fields have proto3 implicit presence. A scalar field equal to its zero
   value (0, `false`, empty string, empty bytes) is not emitted.
2. Fields are emitted in ascending field-number order
   (`protobuf@v1.36.11/internal/impl/codec_message.go:158-160`). One message
   in scope declares fields out of numeric order: `BlockDetails` declares
   `approves_parent = 5` before `transactions = 4` (`api.proto:167-174`). The
   wire order is 1, 2, 3, 4, 5, 6.
3. Repeated scalar numeric fields are packed (proto3 default); the only one
   in scope is `unsigned_input_indexes`.
4. A message-typed field is emitted when the Go pointer is non-nil, even if
   the message is empty (two bytes: tag, length 0). It is omitted when nil.
   Handlers rely on this distinction in three places:
   - `GetTicketsResponse.ticket.spender` is always a non-nil
     `TransactionDetails`, empty when the ticket is unspent
     (`server.go:2299-2302`), so field 2 of `TicketDetails` is always present.
   - `GetTicketsResponse.block` is nil (absent) for unmined tickets
     (`:2310-2313`).
   - `GetTransactionsResponse.mined_transactions` is set only in messages
     for a mined block (`:1425-1433`).
5. A repeated message field built with `make([]*T, n)` is emitted with one
   element per entry; an empty or nil slice emits nothing. Nil and empty
   slices are indistinguishable on the wire.
6. On decode, a `bytes` field that is absent or present with length 0 yields
   a nil slice (`protobuf@v1.36.11/internal/impl/codec_gen.go:5489-5500`).
   Every `!= nil` test on a request `bytes` field in the handlers is therefore
   equivalent to "length > 0".
7. `int32`/`int64`/enum fields use plain varints (a negative value occupies
   10 bytes); `sint32` fields use zigzag. The `sint32` fields in scope are
   `starting_block_height` in `GetTransactionsRequest`, `GetTicketsRequest`
   and `GetCFiltersRequest`, and `ending_block_height` in `GetCFiltersRequest`
   only.
8. Enum fields are open: an unknown numeric value decodes without error and
   reaches the handler.
9. `string` fields must be valid UTF-8 on decode; an invalid one makes the
   library reject the request before the handler runs
   (`protobuf@v1.36.11/internal/impl/codec_gen.go:5053-5067`,
   `consumeStringValidateUTF8`), with status `Internal`, `grpc: error
   unmarshalling request: <err>` (`grpc@v1.83.2/server.go:1417`).

10. Decoding is lenient in the standard proto3 ways, and a hand-written
    decoder must match them because existing clients are generated by
    other protobuf libraries and from other revisions of `api.proto`: a
    field number the message does not declare is skipped, not rejected
    (this is what makes the reserved field 3 of `SignTransactionRequest`
    and fields added by newer clients harmless); a non-repeated scalar that
    appears more than once takes the last value; repeated fields accumulate
    across occurrences; a repeated numeric field is accepted both packed
    and unpacked; a sub-message that appears more than once is merged. A
    known field number with the wrong wire type, a truncated varint or
    length, or a length that overruns the buffer fails the whole request as
    in item 9 (`Internal`, `grpc: error unmarshalling request: <err>`).
11. No handler in scope inspects unknown fields or presence bits, and no
    message in scope uses `optional`, `oneof`, `map` or well-known types
    (`api.proto` has no imports).

Byte strings carried inside `bytes` fields that have their own format:

| Field | Format |
|---|---|
| every `transaction`, `unsigned_transaction`, `serialized_transaction`, `signed_transaction`, `spender_transaction`, `tickets[i]`, `split_tx` | Decred wire transaction as written by `wire.MsgTx.Serialize` and read by `Deserialize` (the serialization type is whatever the transaction's own version field selects; responses built by the wallet are full serializations with prefix and witness) |
| `BlockInfoResponse.block_header` | the stored 180-byte wire block header |
| `GetCFiltersResponse.filter` | `gcs.FilterV2.Bytes()`: the element count N as a wire varint followed by the Golomb-coded filter data (`dcrd/gcs/v4@v4.1.1/gcs.go:238-265`). A filter stored as zero bytes is relayed as the empty byte string, in which case field 2 is absent from the message (`:539-565`). The B and M parameters are not transmitted. |
| `GetCFiltersResponse.key` | the 16-byte filter key, always present |
| hashes | 32 bytes, internal byte order (section 4.5) |

### 4.4 `VersionService` messages (`api.proto:11-19`)

| Message | Fields |
|---|---|
| `VersionRequest` | none |
| `VersionResponse` | 1 `version_string` string; 2 `major` uint32; 3 `minor` uint32; 4 `patch` uint32; 5 `prerelease` string; 6 `build_metadata` string |

### 4.5 Shared messages (`api.proto:137-186`)

| Message | Fields |
|---|---|
| `TransactionDetails` | 1 `hash` bytes; 2 `transaction` bytes; 3 `debits` repeated `Input`; 4 `credits` repeated `Output`; 5 `fee` int64; 6 `timestamp` int64; 7 `transaction_type` `TransactionType` |
| `TransactionDetails.Input` | 1 `index` uint32; 2 `previous_account` uint32; 3 `previous_amount` int64 |
| `TransactionDetails.Output` | 1 `index` uint32; 2 `account` uint32; 3 `internal` bool; 4 `amount` int64; 5 `address` string; 6 `output_script` bytes |
| `BlockDetails` | 1 `hash` bytes; 2 `height` int32; 3 `timestamp` int64; 4 `transactions` repeated `TransactionDetails`; 5 `approves_parent` bool; 6 `prev_block` bytes |
| `DetachedBlockDetails` | 1 `hash` bytes; 2 `height` int32; 3 `prev_block` bytes; 4 `timestamp` int64 |
| `AccountBalance` | 1 `account` uint32; 2 `total_balance` int64. Declared but used by no RPC. |

All hashes carried in `bytes` fields are the 32 raw bytes of
`chainhash.Hash` in its internal (wire) byte order, not the reversed hex
string order.

### 4.6 `WalletService` messages

Requests and responses with no fields are listed once here and not repeated:
`PingRequest`, `PingResponse`, `NetworkRequest`, `CoinTypeRequest`,
`AccountsRequest`, `RenameAccountResponse`, `ImportPrivateKeyResponse`,
`ImportExtendedPublicKeyResponse`, `BirthBlockRequest`, `TicketPriceRequest`,
`StakeInfoRequest`, `ChangePassphraseResponse`,
`PublishUnminedTransactionsRequest`, `PublishUnminedTransactionsResponse`,
`LoadActiveDataFiltersRequest`, `LoadActiveDataFiltersResponse`,
`TransactionNotificationsRequest`, `AccountNotificationsRequest`,
`BestBlockRequest`, `AbandonTransactionResponse`,
`GetCoinjoinOutputspByAcctRequest`, `SetAccountPassphraseResponse`,
`UnlockAccountResponse`, `LockAccountResponse`, `UnlockWalletResponse`,
`LockWalletRequest`, `LockWalletResponse`, `GetPeerInfoRequest`,
`SyncVSPTicketsResponse`, `ProcessManagedTicketsResponse`,
`ProcessUnmanagedTicketsResponse`, `SetVspdVoteChoicesResponse`,
`GetTrackedVSPTicketsRequest`, `DiscoverUsageResponse`.

| Message (`api.proto` line) | Fields |
|---|---|
| `NetworkResponse` (192) | 1 `active_network` uint32 |
| `CoinTypeResponse` (197) | 1 `coin_type` uint32 |
| `AccountNumberRequest` (201) | 1 `account_name` string |
| `AccountNumberResponse` (204) | 1 `account_number` uint32 |
| `AccountsResponse` (209) | 1 `accounts` repeated `Account`; 2 `current_block_hash` bytes; 3 `current_block_height` int32 |
| `AccountsResponse.Account` (210) | 1 `account_number` uint32; 2 `account_name` string; 3 `total_balance` int64; 4 `external_key_count` uint32; 5 `internal_key_count` uint32; 6 `imported_key_count` uint32; 7 `account_encrypted` bool; 8 `account_unlocked` bool |
| `AddressRequest` (225) | 1 `account` uint32; 2 `kind` `Kind`; 3 `index` uint32 |
| `AddressResponse` (234) | 1 `address` string; 2 `public_key` string |
| `RenameAccountRequest` (240) | 1 `account_number` uint32; 2 `new_name` string |
| `RescanRequest` (246) | 1 `begin_height` int32; 2 `begin_hash` bytes |
| `RescanResponse` (250) | 1 `rescanned_through` int32 |
| `NextAccountRequest` (254) | 1 `passphrase` bytes; 2 `account_name` string |
| `NextAccountResponse` (258) | 1 `account_number` uint32 |
| `NextAddressRequest` (262) | 1 `account` uint32; 2 `kind` `Kind`; 3 `gap_policy` `GapPolicy` |
| `NextAddressResponse` (277) | 1 `address` string; 2 `public_key` string |
| `ImportPrivateKeyRequest` (282) | 1 `passphrase` bytes; 2 `account` uint32; 3 `private_key_wif` string; 4 `rescan` bool; 5 `scan_from` int32 |
| `ImportExtendedPublicKeyRequest` (292) | 1 `xpub` string; 2 `account_name` string; 3 `rescan` bool; 4 `scan_from` int32 |
| `DumpPrivateKeyRequest` (301) | 1 `address` string |
| `DumpPrivateKeyResponse` (304) | 1 `private_key_wif` string |
| `BirthBlockResponse` (310) | 1 `hash` bytes; 2 `height` uint32 |
| `ImportVotingAccountFromSeedRequest` (315) | 1 `seed` bytes; 2 `name` string; 3 `passphrase` bytes; 4 `rescan` bool; 5 `scan_from` int32; 6 `discover_usage` bool; 7 `discover_from` int32; 8 `discover_gap_limit` int32 |
| `ImportVotingAccountFromSeedResponse` (325) | 1 `account` uint32 |
| `ImportScriptRequest` (329) | 1 `passphrase` bytes; 2 `script` bytes; 3 `rescan` bool; 4 `scan_from` int32; 5 `require_redeemable` bool |
| `ImportScriptResponse` (336) | 1 `p2sh_address` string; 2 `redeemable` bool |
| `BalanceRequest` (341) | 1 `account_number` uint32; 2 `required_confirmations` int32 |
| `BalanceResponse` (345) | 1 `total` int64; 2 `spendable` int64; 3 `immature_reward` int64; 4 `immature_stake_generation` int64; 5 `locked_by_tickets` int64; 6 `voting_authority` int64; 7 `unconfirmed` int64 |
| `GetTransactionRequest` (355) | 1 `transaction_hash` bytes |
| `GetTransactionResponse` (358) | 1 `transaction` `TransactionDetails`; 2 `confirmations` int32; 3 `block_hash` bytes |
| `GetTransactionsRequest` (364) | 1 `starting_block_hash` bytes; 2 `starting_block_height` sint32; 3 `ending_block_hash` bytes; 4 `ending_block_height` int32; 5 `minimum_recent_transactions` int32; 6 `target_transaction_count` int32 |
| `GetTransactionsResponse` (391) | 1 `mined_transactions` `BlockDetails`; 2 `unmined_transactions` repeated `TransactionDetails` |
| `GetTicketRequest` (396) | 1 `ticket_hash` bytes |
| `GetTicketsRequest` (399) | 1 `starting_block_hash` bytes; 2 `starting_block_height` sint32; 3 `ending_block_hash` bytes; 4 `ending_block_height` int32; 5 `target_ticket_count` int32 |
| `GetTicketsResponse` (406) | 1 `ticket` `TicketDetails`; 2 `block` `BlockDetails` (the nested one); 3 `vsp_host` string |
| `GetTicketsResponse.TicketDetails` (407) | 1 `ticket` `TransactionDetails`; 2 `spender` `TransactionDetails`; 3 `ticket_status` `TicketStatus` |
| `GetTicketsResponse.BlockDetails` (422) | 1 `hash` bytes; 2 `height` int32; 3 `timestamp` int64 |
| `TicketPriceResponse` (434) | 1 `ticket_price` int64; 2 `height` int32 |
| `StakeInfoResponse` (441) | 1 `pool_size` uint32; 2 `all_mempool_tix` uint32; 3 `own_mempool_tix` uint32; 4 `immature` uint32; 5 `live` uint32; 6 `voted` uint32; 7 `missed` uint32; 8 `revoked` uint32; 9 `expired` uint32; 10 `total_subsidy` int64; 11 `unspent` uint32 |
| `BlockInfoRequest` (455) | 1 `block_hash` bytes; 2 `block_height` int32 |
| `BlockInfoResponse` (459) | 1 `block_hash` bytes; 2 `block_height` int32; 3 `confirmations` int32; 4 `timestamp` int64; 5 `block_header` bytes; 6 `stake_invalidated` bool; 7 `approves_parent` bool |
| `ChangePassphraseRequest` (469) | 1 `key` `Key`; 2 `old_passphrase` bytes; 3 `new_passphrase` bytes |
| `FundTransactionRequest` (480) | 1 `account` uint32; 2 `target_amount` int64; 3 `required_confirmations` int32; 4 `include_immature_coinbases` bool; 5 `include_change_script` bool |
| `FundTransactionResponse` (487) | 1 `selected_outputs` repeated `PreviousOutput`; 2 `total_amount` int64; 3 `change_pk_script` bytes |
| `FundTransactionResponse.PreviousOutput` (488) | 1 `transaction_hash` bytes; 2 `output_index` uint32; 3 `amount` int64; 4 `pk_script` bytes; 5 `receive_time` int64; 6 `from_coinbase` bool; 7 `tree` int32 |
| `UnspentOutputsRequest` (502) | 1 `account` uint32; 2 `target_amount` int64; 3 `required_confirmations` int32; 4 `include_immature_coinbases` bool |
| `UnspentOutputResponse` (508) | 1 `transaction_hash` bytes; 2 `output_index` uint32; 3 `amount` int64; 4 `pk_script` bytes; 5 `receive_time` int64; 6 `from_coinbase` bool; 7 `tree` int32; 8 `amount_sum` int64 |
| `ConstructTransactionRequest` (519) | 1 `source_account` uint32; 2 `required_confirmations` int32; 3 `fee_per_kb` int32; 4 `output_selection_algorithm` `OutputSelectionAlgorithm`; 5 `non_change_outputs` repeated `Output`; 6 `change_destination` `OutputDestination` |
| `ConstructTransactionRequest.OutputDestination` (520) | 1 `address` string; 2 `script` bytes; 3 `script_version` uint32 |
| `ConstructTransactionRequest.Output` (526) | 1 `destination` `OutputDestination`; 2 `amount` int64 |
| `ConstructTransactionResponse` (541) | 1 `unsigned_transaction` bytes; 2 `total_previous_output_amount` int64; 3 `total_output_amount` int64; 4 `estimated_signed_size` uint32; 5 `change_index` int32 |
| `SignTransactionRequest` (549) | 1 `passphrase` bytes; 2 `serialized_transaction` bytes; 4 `additional_scripts` repeated `AdditionalScript`. Field number 3 and the name `input_indexes` are `reserved`. |
| `SignTransactionRequest.AdditionalScript` (552) | 1 `transaction_hash` bytes; 2 `output_index` uint32; 3 `tree` int32; 4 `pk_script` bytes |
| `SignTransactionResponse` (568) | 1 `transaction` bytes; 2 `unsigned_input_indexes` repeated uint32 (packed) |
| `SignTransactionsRequest` (573) | 1 `passphrase` bytes; 2 `transactions` repeated `UnsignedTransaction`; 3 `additional_scripts` repeated `AdditionalScript` |
| `SignTransactionsRequest.AdditionalScript` (574) | 1 `transaction_hash` bytes; 2 `output_index` uint32; 3 `tree` int32; 4 `pk_script` bytes |
| `SignTransactionsRequest.UnsignedTransaction` (580) | 1 `serialized_transaction` bytes |
| `SignTransactionsResponse` (588) | 1 `transactions` repeated `SignedTransaction` |
| `SignTransactionsResponse.SignedTransaction` (589) | 1 `transaction` bytes; 2 `unsigned_input_indexes` repeated uint32 (packed) |
| `CreateSignatureRequest` (596) | 1 `passphrase` bytes; 2 `address` string; 3 `serialized_transaction` bytes; 4 `input_index` uint32; 5 `hash_type` `SigHashType`; 6 `previous_pk_script` bytes |
| `CreateSignatureResponse` (612) | 1 `signature` bytes; 2 `public_key` bytes |
| `PublishTransactionRequest` (617) | 1 `signed_transaction` bytes |
| `PublishTransactionResponse` (620) | 1 `transaction_hash` bytes |
| `PurchaseTicketsRequest` (627) | 1 `passphrase` bytes; 2 `account` uint32; 3 `required_confirmations` uint32; 4 `num_tickets` uint32; 5 `expiry` uint32; 6 `tx_fee` int64; 7 `ticket_fee` int64; 8 `dont_sign_tx` bool; 9 `enable_mixing` bool; 10 `mixed_account` uint32; 11 `mixed_account_branch` uint32; 12 `mixed_split_account` uint32; 13 `change_account` uint32; 14 `vsp_host` string; 15 `vsp_pubkey` string; 16 `use_voting_account` bool; 17 `voting_account` uint32 |
| `PurchaseTicketsResponse` (646) | 1 `ticket_hashes` repeated bytes; 2 `tickets` repeated bytes; 3 `split_tx` bytes |
| `SignMessageRequest` (655) | 1 `address` string; 2 `message` string; 3 `passphrase` bytes |
| `SignMessageResponse` (660) | 1 `signature` bytes |
| `SignMessagesRequest` (664) | 1 `passphrase` bytes; 2 `messages` repeated `Message` |
| `SignMessagesRequest.Message` (665) | 1 `address` string; 2 `message` string |
| `SignMessagesResponse` (672) | 1 `replies` repeated `SignReply` |
| `SignMessagesResponse.SignReply` (673) | 1 `signature` bytes; 2 `error` string |
| `TransactionNotificationsResponse` (681) | 1 `attached_blocks` repeated `BlockDetails`; 2 `detached_blocks` repeated bytes; 3 `unmined_transactions` repeated `TransactionDetails`; 4 `unmined_transaction_hashes` repeated bytes; 5 `detached_block_headers` repeated `DetachedBlockDetails` |
| `AccountNotificationsResponse` (706) | 1 `account_number` uint32; 2 `account_name` string; 3 `external_key_count` uint32; 4 `internal_key_count` uint32; 5 `imported_key_count` uint32 |
| `ConfirmationNotificationsRequest` (714) | 1 `tx_hashes` repeated bytes; 2 `stop_after` int32 |
| `ConfirmationNotificationsResponse` (718) | 1 `confirmations` repeated `TransactionConfirmations` |
| `ConfirmationNotificationsResponse.TransactionConfirmations` (719) | 1 `tx_hash` bytes; 2 `confirmations` int32; 3 `block_hash` bytes; 4 `block_height` int32 |
| `ValidateAddressRequest` (1067) | 1 `address` string |
| `ValidateAddressResponse` (1071) | 1 `is_valid` bool; 2 `is_mine` bool; 3 `account_number` uint32; 4 `pub_key_addr` string; 5 `pub_key` bytes; 6 `is_script` bool; 7 `pk_script_addrs` repeated string; 8 `script_type` `ScriptType`; 9 `pay_to_addr_script` bytes; 10 `sigs_required` uint32; 11 `is_internal` bool; 12 `index` uint32 |
| `CommittedTicketsRequest` (1102) | 1 `tickets` repeated bytes |
| `CommittedTicketsResponse` (1123) | 1 `ticketAddresses` repeated `TicketAddress` (the field name is camelCase in the proto) |
| `CommittedTicketsResponse.TicketAddress` (1124) | 1 `ticket` bytes; 2 `address` string |
| `GetAccountExtendedPubKeyRequest` (1106) | 1 `account_number` uint32 |
| `GetAccountExtendedPubKeyResponse` (1110) | 1 `acc_extended_pub_key` string |
| `GetAccountExtendedPrivKeyRequest` (1114) | 1 `account_number` uint32; 2 `passphrase` bytes |
| `GetAccountExtendedPrivKeyResponse` (1119) | 1 `acc_extended_priv_key` string |
| `BestBlockResponse` (1134) | 1 `height` uint32; 2 `hash` bytes |
| `SweepAccountRequest` (1139) | 1 `source_account` string; 2 `destination_address` string; 3 `required_confirmations` uint32; 4 `fee_per_kb` double |
| `SweepAccountResponse` (1146) | 1 `unsigned_transaction` bytes; 2 `total_previous_output_amount` int64; 3 `total_output_amount` int64; 4 `estimated_signed_size` uint32 |
| `AbandonTransactionRequest` (1153) | 1 `transaction_hash` bytes |
| `SignHashesRequest` (1159) | 1 `passphrase` bytes; 2 `address` string; 3 `hashes` repeated bytes |
| `SignHashesResponse` (1165) | 1 `public_key` bytes; 2 `signatures` repeated bytes |
| `SpenderRequest` (1170) | 1 `transaction_hash` bytes; 2 `index` uint32 |
| `SpenderResponse` (1174) | 1 `spender_transaction` bytes; 2 `input_index` uint32 |
| `GetCFiltersRequest` (1179) | 1 `starting_block_hash` bytes; 2 `starting_block_height` sint32; 3 `ending_block_hash` bytes; 4 `ending_block_height` sint32 |
| `GetCFiltersResponse` (1186) | 1 `key` bytes; 2 `filter` bytes; 3 `block_hash` bytes |
| `coinjoinTxsSumByAcct` (1203) | 1 `account_number` uint32; 2 `coinjoin_txs_sum` int32. The message name starts with a lower-case letter; its full name is `walletrpc.coinjoinTxsSumByAcct`. |
| `GetCoinjoinOutputspByAcctResponse` (1208) | 1 `data` repeated `coinjoinTxsSumByAcct` |
| `SetAccountPassphraseRequest` (1212) | 1 `account_passphrase` bytes; 2 `new_account_passphrase` bytes; 3 `account_number` uint32; 4 `wallet_passphrase` bytes |
| `UnlockAccountRequest` (1220) | 1 `passphrase` bytes; 2 `account_number` uint32 |
| `LockAccountRequest` (1225) | 1 `account_number` uint32 |
| `AccountUnlockedRequest` (1235) | 1 `account_number` uint32 |
| `AccountUnlockedResponse` (1239) | 1 `unlocked` bool |
| `UnlockWalletRequest` (1243) | 1 `passphrase` bytes |
| `GetPeerInfoResponse` (1252) | 1 `peer_info` repeated `PeerInfo` |
| `GetPeerInfoResponse.PeerInfo` (1253) | 1 `id` int32; 2 `addr` string; 3 `addr_local` string; 4 `services` string; 5 `version` uint32; 6 `sub_ver` string; 7 `starting_height` int64; 8 `ban_score` int32 |
| `SyncVSPTicketsRequest` (1266) | 1 `vsp_host` string; 2 `vsp_pubkey` string; 3 `account` uint32; 4 `change_account` uint32 |
| `GetVSPTicketsByFeeStatusRequest` (1275) | 3 `fee_status` `FeeStatus` (the only field; its number is 3) |
| `GetVSPTicketsByFeeStatusResponse` (1285) | 1 `tickets_hashes` repeated bytes |
| `ProcessManagedTicketsRequest` (1289) | 1 `vsp_host` string; 2 `vsp_pubkey` string; 3 `fee_account` uint32; 4 `change_account` uint32 |
| `ProcessUnmanagedTicketsRequest` (1298) | 1 `vsp_host` string; 2 `vsp_pubkey` string; 3 `fee_account` uint32; 4 `change_account` uint32 |
| `SetVspdVoteChoicesRequest` (1307) | 1 `vsp_host` string; 2 `vsp_pubkey` string; 3 `fee_account` uint32; 4 `change_account` uint32 |
| `GetTrackedVSPTicketsResponse` (1318) | 1 `vsps` repeated `VSP` |
| `GetTrackedVSPTicketsResponse.Ticket` (1319) | 1 `ticket_hash` bytes; 2 `voting_address` string; 3 `commitment_address` string; 4 `state` uint32; 5 `fee` int64; 6 `fee_hash` bytes |
| `GetTrackedVSPTicketsResponse.VSP` (1327) | 1 `host` string; 2 `tickets` repeated `Ticket` |
| `DiscoverUsageRequest` (1335) | 1 `discover_accounts` bool; 2 `gap_limit` uint32; 3 `starting_block_hash` bytes |

### 4.7 Enums

| Enum (`api.proto` line) | Values |
|---|---|
| `TransactionDetails.TransactionType` (157) | `REGULAR` 0, `TICKET_PURCHASE` 1, `VOTE` 2, `REVOCATION` 3, `COINBASE` 4 (declared in the order 0, 4, 1, 2, 3) |
| `AddressRequest.Kind` (227), `NextAddressRequest.Kind` (264) | `BIP0044_EXTERNAL` 0, `BIP0044_INTERNAL` 1 |
| `NextAddressRequest.GapPolicy` (269) | `GAP_POLICY_UNSPECIFIED` 0, `GAP_POLICY_ERROR` 1, `GAP_POLICY_IGNORE` 2, `GAP_POLICY_WRAP` 3 |
| `GetTicketsResponse.TicketDetails.TicketStatus` (410) | `UNKNOWN` 0, `UNMINED` 1, `IMMATURE` 2, `LIVE` 3, `VOTED` 4, `MISSED` 5, `EXPIRED` 6, `REVOKED` 7 |
| `ChangePassphraseRequest.Key` (470) | `PRIVATE` 0, `PUBLIC` 1 |
| `ConstructTransactionRequest.OutputSelectionAlgorithm` (530) | `UNSPECIFIED` 0, `ALL` 1 |
| `CreateSignatureRequest.SigHashType` (601) | `SIGHASH_OLD` 0, `SIGHASH_ALL` 1, `SIGHASH_NONE` 2, `SIGHASH_SINGLE` 3, `SIGHASH_ALLVALUE` 4, `SIGHASH_ANYONECANPAY` 128 (written `0x80`) |
| `ValidateAddressResponse.ScriptType` (1079) | `NonStandardTy` 0, `PubKeyTy` 1, `PubKeyHashTy` 2, `ScriptHashTy` 3, `MultiSigTy` 4, `NullDataTy` 5, `StakeSubmissionTy` 6, `StakeGenTy` 7, `StakeRevocationTy` 8, `StakeSubChangeTy` 9, `PubkeyAltTy` 10, `PubkeyHashAltTy` 11, `TreasuryGenTy` 12, `TreasuryAddTy` 13 |
| `GetVSPTicketsByFeeStatusRequest.FeeStatus` (1276) | `VSP_FEE_PROCESS_STARTED` 0, `VSP_FEE_PROCESS_PAID` 1, `VSP_FEE_PROCESS_ERRORED` 2, `VSP_FEE_PROCESS_CONFIRMED` 3 |

Mappings from wallet values to enum numbers:

| Mapping | Rule | Source |
|---|---|---|
| wallet `TransactionType` → `TransactionType` | `TransactionTypeCoinbase` → 4; `TransactionTypeTicketPurchase` → 1; `TransactionTypeVote` → 2; `TransactionTypeRevocation` → 3; anything else → 0. The wallet's own numbering (Regular 0, Coinbase 1, TicketPurchase 2, Vote 3, Revocation 4, `wallet/notifications.go:408-427`) differs; map by name. | `server.go:2245-2258` |
| wallet `TicketStatus` → `TicketStatus` | `Expired` → 6; `Immature` → 2; `Voted` → 4; `Revoked` → 7; `Unmined` → 1; `Missed` → 5; `Unknown` → 0; every other value, which is `Live` and `Unspent` (the SPV-only status, `wallet/wallet.go:2876-2879`), → 3 `LIVE` | `server.go:2281-2298` |
| `stdscript.ScriptType` → `ScriptType` (`scProto`) | `STNonStandard` → 0; `STPubKeyEcdsaSecp256k1` → 1; `STPubKeyHashEcdsaSecp256k1` → 2; `STScriptHash` → 3; `STMultiSig` → 4; `STNullData` → 5; `STStakeSubmissionPubKeyHash`, `STStakeSubmissionScriptHash` → 6; `STStakeGenPubKeyHash`, `STStakeGenScriptHash` → 7; `STStakeRevocationPubKeyHash`, `STStakeRevocationScriptHash` → 8; `STStakeChangePubKeyHash`, `STStakeChangeScriptHash` → 9; `STPubKeyEd25519`, `STPubKeySchnorrSecp256k1` → 10; `STPubKeyHashEd25519`, `STPubKeyHashSchnorrSecp256k1` → 11; `STTreasuryGenPubKeyHash`, `STTreasuryGenScriptHash` → 12; `STTreasuryAdd` → 13; anything else → 0 | `server.go:3664-3698` |
| `FeeStatus` → `udb.FeeStatus` | identity on 0..3 (`VSPFeeProcessStarted` 0, `Paid` 1, `Errored` 2, `Confirmed` 3) | `server.go:4051-4063`, `wallet/udb/vsp.go:25-36` |

## 5. Algorithms and state machines

Notation used throughout this section:

- **S(code, "text")** — the handler returns `status.Errorf(code, ...)` with
  exactly that message. `<x>` marks an interpolated value.
- **T(e)** — the handler returns `translateError(e)`: the code comes from the
  table in 5.3.1 and the message is `e.Error()`.
- **RAW(e)** — the handler returns `e` unchanged and the gRPC library
  converts it (5.3.3). For a wallet error this is always `Unknown` unless the
  error wraps a context error.
- "The wallet" is the `*wallet.Wallet` stored by `StartWalletService`.
- `ctx` is the RPC's context: cancelled when the client cancels, the client
  deadline passes, the connection drops, the handler returns, or the server
  stops.

### 5.1 Server construction and service start-up

#### 5.1.1 `startRPCServers`, gRPC part (`rpcserver.go:248-347`)

The TLS key pair, the client CA pool and `--issueclientcert` handling are
shared with JSON-RPC and specified in spec 16 section 5.1. The gRPC-specific
steps, in execution order:

1. If `--noservertls` is set: log info `Server TLS is disabled.  Only
   JSON-RPC may be used` (two spaces after the period). No gRPC server is
   created; skip to step 8.
2. Build the base `tls.Config`: `Certificates = [keyPair]`, `MinVersion =
   TLS 1.2`, `ClientCAs` = a new empty pool (`:272-276`).
3. `clientCAsExist` = whether the `--clientcafile` path exists (default
   `<appdata>/clients.pem`, `config.go:80`, `:460-461`). If it exists, read
   it (a read error aborts start-up) and append its PEM certificates to the
   pool; if none were appended, warn `No certificates added from CA file
   <path>` (`:277-287`).
4. If `--issueclientcert`: generate a CA and client certificate, add the CA
   certificate to the pool and emit the `issuedclientcertificate` pipe
   message (`:291-308`; spec 16).
5. `clientCAsExist = clientCAsExist || --issueclientcert` (`:315`). Existence
   of the file is enough; a file with no usable certificate still counts.
6. If there are gRPC listen addresses and `clientCAsExist` is false: warn
   `gRPC server is configured with listeners, but no trusted client
   certificates exist (looked in <path>)` and create no gRPC server
   (`:316-319`).
7. If there are gRPC listen addresses and `clientCAsExist` is true
   (`:320-347`):
   1. Clone the base TLS config and set `ClientAuth =
      RequireAndVerifyClientCert` on the clone (the base config keeps the
      JSON-RPC setting).
   2. `listeners = makeListeners(cfg.GRPCListeners, net.Listen)`: plain TCP
      listeners; TLS is applied by the gRPC credentials, not by
      `tls.Listen`. Address classification and ordering are those of
      `makeListeners` (section 3.2; spec 16 section 5.1.5). Addresses that
      fail to bind are logged (`Can't listen on <addr>: <err>`, warn) and
      skipped. If no listener results, return the
      error `failed to create listeners for RPC server`, which aborts
      start-up.
   3. `grpc.NewServer(grpc.Creds(credentials.NewTLS(tlsConfig)),
      grpc.StreamInterceptor(interceptStreaming),
      grpc.UnaryInterceptor(interceptUnary))`.
   4. `rpcserver.RegisterServices(server)`.
   5. `StartWalletLoaderService(walletLoader, activeNet)`,
      `StartAgendaService(activeNet.Params)`,
      `StartDecodeMessageService(activeNet.Params)`,
      `StartMessageVerificationService(activeNet.Params)`, in that order.
   6. For each listener start a goroutine that: takes `laddr =
      lis.Addr().String()`; sends the `grpclistener` pipe message carrying
      `laddr` when `--rpclistenerevents` is set (`ipc.go:149-177`); logs info
      `gRPC server listening on <laddr>` (subsystem `DCRW`); calls
      `server.Serve(lis)`; when it returns, logs trace `Finished serving
      gRPC: <err>`.
8. After the JSON-RPC server is handled: if neither server exists, return
   the error `no suitable RPC services can be started` (`:390-392`).

With no gRPC listen addresses (`--nogrpc`, `config.go:880-881`) steps 6 and 7
are both skipped silently.

#### 5.1.2 Attaching the wallet (`dcrwallet.go:385-399`)

If a gRPC server was created, `run` registers one loader callback that calls,
in order, `StartWalletService(w)`, `StartTicketBuyerService(w)`,
`StartAccountMixerService(w)`, `StartNetworkService(w)`,
`StartVotingService(w)`. `Loader.RunAfterLoad` runs the callback immediately
if a wallet is already loaded, otherwise once, on the first create or open;
the loader then discards its callbacks (`internal/loader/loader.go:79-104`).
Consequences are in section 10 (Q1).

### 5.2 Interceptors and logging

#### 5.2.1 `interceptUnary` (`rpcserver.go:432-448`)

1. `p, ok = peer.FromContext(ctx)`. If `ok`, log debug (subsystem `GRPC`):
   `Unary method <FullMethod> invoked by <p.Addr.String()>`.
2. `err = rpcserver.ServiceReady(serviceName(FullMethod))`. If non-nil,
   return `(nil, err)`. Nothing further is logged.
3. `resp, err = handler(ctx, req)`.
4. If `err != nil` and `ok`, log error (`GRPC`): `Unary method <FullMethod>
   invoked by <addr> errored: <err>`. `<err>` is the handler's error value
   formatted with `%v`; for a status error that is `rpc error: code = <Code>
   desc = <message>` (`grpc@v1.83.2/internal/status/status.go:201-203`).
5. Return `(resp, err)`.

#### 5.2.2 `interceptStreaming` (`rpcserver.go:406-430`)

1. `p, ok = peer.FromContext(ss.Context())`. If `ok`, log debug: `Streaming
   method <FullMethod> invoked by <addr>`.
2. `ServiceReady` as above; a non-nil error is returned without logging.
3. `err = handler(srv, ss)`.
4. If `err != nil` and `ok`: the log level is error, except that it is debug
   when `status.Code(err) == codes.Canceled` and the stream context is
   already done. Message: `Streaming method <FullMethod> invoked by <addr>
   errored: <err>`.
5. Return `err`.

`FullMethod` is `/walletrpc.<Service>/<Method>`. `ServiceReady` outcomes are
in section 3.1; for the two services in scope: `VersionService` is always
ready; `WalletService` is ready once `StartWalletService` has run.

#### 5.2.3 gRPC library logger (`log.go`)

`UseLogger(loggers.GrpcLog)` is called while logging is initialised
(`log.go:38`). The adapter implements the library's legacy `Logger`
interface, so every library log call, whatever its severity (info, warning
or error), arrives through `Print`, `Printf` or `Println`
(`grpc@v1.83.2/grpclog/internal/logger.go:33-87`) and the library's verbosity
check always returns true.

| Adapter method | Action |
|---|---|
| `Print(args...)`, `Println(args...)` | `Info(args...)` on the `GRPC` logger after stripping a leading `grpc: ` from the first argument if it is a string |
| `Printf(format, args...)` | `Infof(format', args...)` where `format'` is `format` with a leading `grpc: ` removed |
| `Fatal`, `Fatalln` | `Critical(args...)` with the same prefix strip, then `os.Exit(1)` |
| `Fatalf` | `Criticalf(format', args...)`, then `os.Exit(1)` |

### 5.3 Error translation

#### 5.3.1 `errorCode` (`server.go:85-136`)

Evaluated top to bottom; the first match wins.

1. Find the first error in `err`'s chain (outermost first, following
   `Unwrap`) that yields a `Kind` other than `Other` (`errors.As(err,
   &kind)`; `errors/errors.go:136-149`, `:311-323`). If found, map it:

   | `errors.Kind` | gRPC code |
   |---|---|
   | `Invalid` | `InvalidArgument` (3) |
   | `Permission` | `PermissionDenied` (7) |
   | `Exist` | `AlreadyExists` (6) |
   | `NotExist` | `NotFound` (5) |
   | `Crypto` | `DataLoss` (15) |
   | `Locked` | `FailedPrecondition` (9) |
   | `Passphrase` | `InvalidArgument` (3) |
   | `Seed` | `InvalidArgument` (3) |
   | `WatchingOnly` | `Unimplemented` (12) |
   | `InsufficientBalance` | `ResourceExhausted` (8) |
   | `NoPeers` | `Unavailable` (14) |
   | `Bug`, `IO`, `Encoding`, `ScriptFailure`, `Policy`, `DoubleSpend`, `Protocol` | no result: these `case` arms are empty and Go does not fall through. Continue with step 2. |
   | `Consensus`, `Deployment` | not listed in the switch. Continue with step 2. |

2. If `errors.Is(err, hdkeychain.ErrInvalidSeedLen)`: `InvalidArgument`.
3. If `errors.Is(err, context.Canceled)`: `Canceled` (1).
4. If the chain contains an error with a `GRPCStatus()` method whose code is
   neither `OK` nor `Unknown`: that code.
5. Otherwise `Unknown` (2).

`context.DeadlineExceeded` has no rule and ends at `Unknown` (or at a code
from step 1 if a kind wraps it).

#### 5.3.2 `translateError` message (`server.go:80-83`)

The message is `err.Error()` verbatim. For a wallet `*errors.Error` that is
the nested rendering defined by `errors/errors.go:247-290`, with the
separator between nested wallet errors set to `":: "` by the main package
(`dcrwallet.go:36-39`; the package default `":\n\t"` is never in effect in
the daemon). Rendering rule, repeated per nesting level: the `Op` if
non-empty and different from the last printed `Op`; then, if the `Kind` is
non-zero and different from the last printed kind, `": "` (only when
something was printed at this level) and the kind's string; then, if the
wrapped error is another `*errors.Error`, the separator (only when something
was printed at this level) and continue with it; otherwise `": "` (same
condition) and the wrapped error's string. An error that renders nothing is
`unclassified error`.

Kind strings (`errors/errors.go:77-124`): `Bug` `internal wallet error`;
`Invalid` `invalid operation`; `Permission` `permission denied`; `IO` `I/O
error`; `Exist` `item already exists`; `NotExist` `item does not exist`;
`Encoding` `invalid encoding`; `Crypto` `encryption/decryption error`;
`Locked` `wallet locked`; `Passphrase` `invalid passphrase`; `Seed` `invalid
seed`; `WatchingOnly` `watching only wallet`; `InsufficientBalance`
`insufficient balance`; `ScriptFailure` `transaction script fails to
execute`; `Policy` `policy violation`; `Consensus` `consensus violation`;
`DoubleSpend` `double spend`; `Protocol` `protocol violation`; `NoPeers`
`Decred network is unreachable`; `Deployment` `inactive deployment`.

Example: `wallet.NetworkBackend()` with no backend is `errors.E(op,
errors.NoPeers)` and renders `wallet.NetworkBackend: Decred network is
unreachable`.

When the argument of `translateError` is itself a gRPC status error (this
happens in `SweepAccount`, `ConstructTransaction` and on `Send` failures),
rule 4 keeps its code, but the message becomes the status error's string, so
the client sees a nested message: `rpc error: code = InvalidArgument desc =
<original message>`.

#### 5.3.3 Errors returned without translation (RAW)

When a handler returns an error that is not a status error, the library
(`grpc@v1.83.2/server.go:1443-1451`, `:1738-1745`,
`status/status.go:96-127`, `:151-162`) converts it:

1. If the error wraps an error with `GRPCStatus()`: that status's code, with
   the message replaced by the full `err.Error()`.
2. Else if `errors.Is(err, context.DeadlineExceeded)`: `DeadlineExceeded`.
3. Else if `errors.Is(err, context.Canceled)`: `Canceled`.
4. Else `Unknown`.

The message is `err.Error()`. Wallet error kinds play no part.

### 5.4 Common procedures

#### 5.4.1 Temporary unlock (P-UNLOCK)

Used by handlers that accept a `passphrase` field. Unless stated otherwise in
the handler's steps:

1. If `len(passphrase) == 0`, do nothing: the operation runs against the
   wallet's current lock state and fails in the wallet with kind `Locked`
   (`FailedPrecondition`) if keys are needed and unavailable.
2. Otherwise create `lock`, a channel of capacity 1, and register a deferred
   send of one value on it for when the handler returns.
3. Call `wallet.Unlock(ctx, passphrase, lock)` (`wallet/wallet.go:1712`). On
   error return T(err): wrong passphrase is kind `Passphrase`
   (`InvalidArgument`), a watching-only wallet is kind `WatchingOnly`
   (`Unimplemented`).

Observable effects of `Unlock` with a timeout channel
(`wallet/wallet.go:1712-1839`):

- Wallet locked before the call: it is unlocked, and locked again when the
  handler returns (the deferred send fires the timeout). Wallet log lines:
  `The wallet has been temporarily unlocked`, then `The wallet has been
  locked due to timeout.`
- Wallet already unlocked with no timeout (for example by `UnlockWallet`):
  the passphrase is still verified; the channel is drained and ignored; the
  wallet stays unlocked after the handler returns.
- Wallet already unlocked with a timeout (for example JSON-RPC
  `walletpassphrase`): the earlier timeout is cancelled and replaced by this
  call's channel, so the wallet is locked when the handler returns.
- Wrong passphrase: the wallet is locked, whatever its previous state
  (`:1722-1727`).

Request passphrase buffers are overwritten with zeros by some handlers
before returning; this is not observable to clients and is listed per
handler only where the order matters.

#### 5.4.2 Network backend requirement (P-BACKEND)

`requireNetworkBackend()` (`server.go:352-359`): call
`wallet.NetworkBackend()`; on any error return S(`FailedPrecondition`,
`wallet is not associated with a consensus server RPC client`). The backend
is set while a syncer (SPV or dcrd RPC) is running.

Five handlers call `wallet.NetworkBackend()` directly instead and treat its
error differently: `PurchaseTickets` and `GetPeerInfo` return it RAW
(`Unknown`, `wallet.NetworkBackend: Decred network is unreachable`);
`StakeInfo`, `GetTicket` and `GetTickets` ignore it.

#### 5.4.3 Address and hash decoding

- `decodeAddress(a, params)`: `stdaddr.DecodeAddress`; failure is
  S(`InvalidArgument`, `invalid address <a>: <err>`).
- `decodeHashes(in)`: for each element in order, `chainhash.NewHash`;
  failure is S(`InvalidArgument`, `hash (hex <hex>): <err>`) where `<hex>` is
  the lower-case hex of the offending element and `<err>` is `invalid hash
  length of <n>, want 32` (`dcrd/chaincfg/chainhash@v1.0.5/hash.go:51-56`).
- Every other single-hash field is checked with `chainhash.NewHash` and a
  per-handler message.

#### 5.4.4 `decodeBlockRange` (`server.go:1346-1386`)

Input: `starting_block_hash`, `starting_block_height`, `ending_block_hash`,
`ending_block_height`. Output: optional start and end block identifiers.

1. Start: if hash non-empty and height ≠ 0: S(`InvalidArgument`, `starting
   block hash and height may not be specified simultaneously`). Else if hash
   non-empty: `chainhash.NewHash`; failure S(`InvalidArgument`, `invalid
   starting block hash: invalid hash length of <n>, want 32`); identifier by
   hash. Else if height ≠ 0: identifier by height (the value is passed
   through unchanged, including negative values). Else none.
2. End: the same with `ending` in both messages (`ending block hash and
   height may not be specified simultaneously`, `invalid ending block hash:
   ...`).

The wallet resolves identifiers inside its database transaction
(`wallet/wallet.go:3040-3081`): no start → height 0; no end → −1; an
identifier by hash → that block header's height (an unknown hash is a wallet
error, returned as T(err)). Height 0 cannot be requested explicitly as an
identifier, but it is the default start. Section 10 (Q17) describes what
negative and inverted ranges do.

#### 5.4.5 `decodeDestination` (`server.go:1142-1164`)

Input: an optional `OutputDestination`. Output: `(pkScript, version)`.

1. If the destination is absent: S(`InvalidArgument`, `unknown or missing
   output destination`).
2. If `address` is non-empty: `decodeAddress`; on success the result is the
   address's payment script and script version.
3. Else if `script` is non-empty: if `script_version > 65535`
   S(`InvalidArgument`, `script_version overflows uint16`); otherwise the
   result is `(script, uint16(script_version))`.
4. Else: S(`InvalidArgument`, `unknown or missing output destination`).

#### 5.4.6 Marshalling helpers (`server.go:2212-2386`)

`TransactionDetails` from a wallet `TransactionSummary`
(`wallet/notifications.go:114-181`, `:393-401`):

| Field | Value |
|---|---|
| `hash` | the 32-byte transaction hash |
| `transaction` | the serialized transaction |
| `debits` | one `Input` per wallet-owned input, in the summary's order: `index`, `previous_account`, `previous_amount` (atoms) |
| `credits` | one `Output` per wallet-owned output: `index`, `account`, `internal`, `amount` (atoms), `address` (the address string, or empty when the summary has no address), `output_script` |
| `fee` | the summary's fee: inputs minus outputs when every input is a wallet debit, else 0 |
| `timestamp` | Unix seconds: the earlier of the receive time and, when mined, the block time |
| `transaction_type` | per section 4.7 |

`BlockDetails` from a wallet `Block` with a header: `hash` =
`header.BlockHash()`; `height` = `int32(header.Height)`; `timestamp` =
`header.Timestamp.Unix()`; `approves_parent` = `header.VoteBits & 0x0001 !=
0`; `transactions` = the block's summaries; `prev_block` =
`header.PrevBlock`. With a nil header (not reachable from the callers in this
spec): `hash` absent, `height` −1, `transactions` only.

`DetachedBlockDetails` from a header: `hash`, `height`, `timestamp` (Unix
seconds), `prev_block`.

`GetTicketsResponse.BlockDetails` from a header: absent when the header is
nil; else `hash`, `height`, `timestamp`.

`TicketDetails` from a wallet `TicketSummary`: `ticket` = the ticket's
`TransactionDetails`; `spender` = the spender's `TransactionDetails`, or an
empty (but present) message when there is no spender; `ticket_status` per
section 4.7.

### 5.5 `VersionService.Version` (`server.go:322-329`)

No validation, no wallet access, no error path. Response: `version_string =
"10.0.0"`, `major = 10`, `minor = 0`, `patch = 0`. `prerelease` and
`build_metadata` are never set. Encoded response bytes (hex): `0a 06 31 30 2e
30 2e 30 10 0a` (field 1 length 6 `10.0.0`, field 2 varint 10; fields 3 to 6
are zero and omitted).

### 5.6 `WalletService` RPCs

Every RPC passes the interceptor (5.2) before its handler; when the service
is not ready the result is S(`FailedPrecondition`, `service
walletrpc.WalletService is not ready`) and the handler does not run. The
position of request decoding relative to the interceptor differs by RPC
kind: for a unary RPC the generated stub decodes the request before it calls
the interceptor (`rpc/walletrpc/api_grpc.pb.go:1287-1303`), so an oversized
or undecodable request fails with the library's `ResourceExhausted` or
`Internal` status without any interceptor log line and even when the service
is not ready; for a streaming RPC the interceptor (log line and readiness
check) runs first and the request is read afterwards (inside the generated
handler for server-streaming RPCs, `:1467-1473`; by the handler's own `Recv`
calls for `ConfirmationNotifications`), so a not-ready service answers
`FailedPrecondition`
whatever the request bytes are and a decode failure is logged by
`interceptStreaming` as a handler error. Steps below are in execution order;
the first failing step ends the RPC. "Wallet call" names the
`*wallet.Wallet` method whose behaviour (database transactions, locks, error
kinds) is specified by the wallet specs; this section fixes only the order of
calls, the arguments, the error wrapping and the response construction.

#### 5.6.1 `Ping` (`server.go:361-363`)

Returns an empty `PingResponse`. No wallet access.

#### 5.6.2 `Network` (`server.go:365-369`)

`active_network = uint32(wallet.ChainParams().Net)`: mainnet `0xd9b400f9`,
testnet3 `0xb194aa75`, simnet `0x12141c16` (`dcrd/wire@v1.8.0/protocol.go:131-146`).
No error path.

#### 5.6.3 `CoinType` (`server.go:371-377`)

1. Wallet call `CoinType(ctx)` (`wallet/wallet.go:1070`); error → T(err). A
   watching-only wallet yields kind `WatchingOnly` → `Unimplemented`.
2. `coin_type` = the result.

#### 5.6.4 `AccountNumber` (`server.go:379-388`)

1. Wallet call `AccountNumber(ctx, account_name)`; error → T(err) (unknown
   name is kind `NotExist` → `NotFound`).
2. `account_number` = the result.

#### 5.6.5 `Accounts` (`server.go:390-415`)

1. Wallet call `Accounts(ctx)` (`wallet/wallet.go:3371`); error → T(err).
2. For each account in the wallet's order (the address manager's
   `ForEachAccount` order): `account_number`, `account_name`, `total_balance`
   (atoms), `external_key_count = LastReturnedExternalIndex + 1`,
   `internal_key_count = LastReturnedInternalIndex + 1` (uint32 arithmetic:
   the "nothing returned yet" sentinel `0xFFFFFFFF` wraps to 0),
   `imported_key_count`, `account_encrypted`, `account_unlocked`.
3. `current_block_hash` = the main chain tip hash read in the same database
   transaction; `current_block_height` = its height.

#### 5.6.6 `Address` (`server.go:581-611`)

1. `kind` 0 → branch 0; 1 → branch 1; any other value → S(`InvalidArgument`,
   `kind=<decimal value>`).
2. Wallet call `AddressAtIdx(ctx, account, branch, index)`
   (`wallet/addresses.go:673`); error → T(err). Unknown account is kind
   `NotExist` → `NotFound`, message `wallet.AddressAtIdx: item does not
   exist: account <n>`. `index ≥ 2^31` is an unclassified error → `Unknown`,
   message `wallet.AddressAtIdx: child out of range`. The wallet logs info
   `Returning address (account=<a> branch=<b> child=<c>)`.
3. `address` = the address string. If the address is a wallet P2PKH address
   (it exposes a public key), `public_key` =
   `NewAddressPubKeyEcdsaSecp256k1V0Raw(pubKey, params).String()`
   (constructor error → T(err)); otherwise empty.

The call derives the address from the account's branch xpub without
recording it as returned or used.

#### 5.6.7 `Balance` (`server.go:869-891`)

1. Wallet call `AccountBalance(ctx, account_number, required_confirmations)`
   (`wallet/wallet.go:1911`; its `Op` string is
   `wallet.CalculateAccountBalance`); error → T(err). The handler does not
   validate `required_confirmations`.
2. Response, all in atoms: `total` ← `Total`; `spendable` ← `Spendable`;
   `immature_reward` ← `ImmatureCoinbaseRewards`; `immature_stake_generation`
   ← `ImmatureStakeGeneration`; `locked_by_tickets` ← `LockedByTickets`;
   `voting_authority` ← `VotingAuthority`; `unconfirmed` ← `Unconfirmed`.

#### 5.6.8 `GetAccountExtendedPubKey` (`server.go:1267-1276`)

1. Wallet call `AccountXpub(ctx, account_number)`; error → RAW(err), so an
   unknown account is `Unknown`, not `NotFound`.
2. `acc_extended_pub_key` = the extended key's string form.

#### 5.6.9 `GetAccountExtendedPrivKey` (`server.go:1278-1301`)

1. If `passphrase` is non-empty: `wallet.Unlock(ctx, passphrase, lock)`;
   error → T(err). Only after a successful unlock is the relock (and
   zeroing of the passphrase) deferred.
2. Wallet call `AccountXpriv(ctx, account_number)`; error → T(err).
3. `acc_extended_priv_key` = the extended private key's string form.

#### 5.6.10 `GetTransaction` (`server.go:1303-1321`)

1. `transaction_hash` must be 32 bytes, else S(`InvalidArgument`,
   `transaction_hash has invalid length`).
2. Wallet call `TransactionSummary(ctx, hash)` (`wallet/wallet.go:2795`);
   error → T(err). The error's `Op` is `wallet.TransactionSummary(<hash
   string>)`.
3. `transaction` = `TransactionDetails` (5.4.6); `confirmations` = the
   confirmation count (0 when unmined); `block_hash` = the containing block
   hash when `confirmations > 0`, otherwise absent.

#### 5.6.11 `GetTransactions` (server-streaming; `server.go:1391-1451`)

1. `decodeBlockRange` (5.4.4); its status errors are returned as is.
2. If `minimum_recent_transactions ≠ 0`: if an end block was specified,
   S(`InvalidArgument`, `ending block and minimum number of recent
   transactions may not be specified simultaneously`); if the value is
   negative, S(`InvalidArgument`, `minimum number of recent transactions may
   not be negative`). The value is otherwise ignored.
3. If `target_transaction_count < 0`: S(`InvalidArgument`, `maximum
   transaction count may not be negative`).
4. Wallet call `GetTransactions(ctx, rangeFn, start, end)`
   (`wallet/wallet.go:3248`). The whole iteration, including every `Send`,
   runs inside one read-only database transaction. The wallet calls
   `rangeFn` once per block that has at least one wallet transaction, and
   once for the set of unmined transactions (a `Block` with a nil header).
   `rangeFn(block)`:
   1. Mined block: message with only `mined_transactions =
      marshalBlock(block)`. Unmined set: message with only
      `unmined_transactions`.
   2. `txCount += len(block.Transactions)`.
   3. If `ctx` is done: stop, returning `ctx.Err()` (the message is not
      sent).
   4. `Send` the message. Stop if `Send` failed, or if
      `target_transaction_count > 0` and `txCount ≥ target`.
5. A non-nil error from the wallet call (including the `Send` or context
   error, wrapped with `Op` `wallet.GetTransactions`) → T(err). Otherwise the
   stream ends with status OK.

Order of messages: with defaults, mined blocks by ascending height followed
by one message of unmined transactions (when any exist). See Q17 for
non-default ranges. The target count is checked after a whole block, so the
total may exceed the target.

#### 5.6.12 `GetTicket` (`server.go:1453-1488`)

1. `chainhash.NewHash(ticket_hash)`; failure S(`InvalidArgument`, `invalid
   hash length of <n>, want 32`).
2. `n, _ = wallet.NetworkBackend()` (error ignored).
3. If `n` is a `*chain.Syncer` (dcrd RPC mode): wallet call
   `GetTicketInfoPrecise(ctx, syncer.RPC(), hash)`; otherwise
   `GetTicketInfo(ctx, hash)`. Error → T(err).
4. Wallet call `VSPHostForTicket(ctx, hash)` (`wallet/wallet.go:5670`). A
   `NotExist` error is ignored (host is empty); any other error → RAW(err).
5. Response: `ticket` = `TicketDetails` (5.4.6), `block` = block details of
   the ticket's block (absent when unmined), `vsp_host`.

#### 5.6.13 `GetTickets` (server-streaming; `server.go:1490-1549`)

1. `decodeBlockRange`.
2. If `target_ticket_count < 0`: S(`InvalidArgument`, `target ticket count
   may not be negative`).
3. `n, _ = wallet.NetworkBackend()`. If `n` is a `*chain.Syncer`: wallet
   call `GetTicketsPrecise(ctx, syncer.RPC(), rangeFn, start, end)`; else
   `GetTickets(ctx, rangeFn, start, end)`. One read-only database
   transaction spans the iteration. The wallet calls `rangeFn(tickets,
   header)` once per block containing at least one wallet ticket (`header`
   nil for unmined tickets). `rangeFn`:
   1. Build one response with `block` from the header.
   2. For each ticket in order: set `ticket`; `VSPHostForTicket(ctx, hash)`
      (a `NotExist` error is ignored, any other error stops the iteration);
      set `vsp_host`; `Send`. A `Send` error stops the iteration.
   3. `ticketCount += len(tickets)`.
   4. If `ctx` is done: stop with `ctx.Err()`. Otherwise stop when
      `target_ticket_count > 0` and `ticketCount ≥ target`.
4. Wallet error → T(err); else OK.

One message per ticket; all tickets of a block are sent before the target
is checked.

#### 5.6.14 `TicketPrice` (`server.go:893-904`)

1. Wallet call `NextStakeDifficulty(ctx)` (`wallet/difficulty.go:792`);
   error → T(err). When DCP0001 is not active at the tip the error has kind
   `Deployment`, which maps to `Unknown`.
2. `_, tipHeight = wallet.MainChainTip(ctx)` (a second, separate database
   read).
3. `ticket_price` = the difficulty in atoms; `height` = `tipHeight`.

#### 5.6.15 `StakeInfo` (`server.go:906-932`)

1. `n, _ = wallet.NetworkBackend()`. If `n` is a `*chain.Syncer`: wallet
   call `StakeInfoPrecise(ctx, syncer.RPC())`; else `StakeInfo(ctx)`. Error
   → T(err).
2. Response from `StakeInfoData` (`wallet/wallet.go:3954-3971`): `pool_size`,
   `all_mempool_tix`, `own_mempool_tix`, `immature`, `live`, `voted`,
   `missed`, `revoked`, `expired`, `total_subsidy` (atoms), `unspent`. The
   data's `BlockHeight`, `Sdiff` and `UnspentExpired` are not exposed.

#### 5.6.16 `BlockInfo` (`server.go:1020-1055`)

1. If `block_hash` non-empty and `block_height ≠ 0`: S(`InvalidArgument`,
   `block hash and height must not be set together`). Else if `block_hash`
   non-empty: must be 32 bytes, else S(`InvalidArgument`, `block hash has
   invalid length`); identify by hash. Else identify by `block_height` (0
   selects the genesis block; negative values go to the wallet unchanged).
2. Wallet call `BlockInfo(ctx, id)` (`wallet/wallet.go:2752`); error →
   T(err).
3. Deserialize the stored header; failure S(`Internal`, `failed to
   deserialize saved block header: <err>`).
4. Response: `block_hash`; `block_height` (from the header);
   `confirmations` (0 when not in the main chain); `timestamp` (header time,
   Unix seconds); `block_header` (the 180-byte serialized header);
   `stake_invalidated`; `approves_parent = VoteBits & 0x0001 != 0`.

#### 5.6.17 `BestBlock` (`server.go:3727-3734`)

`hash, height = wallet.MainChainTip(ctx)`; `hash` = 32 bytes; `height` =
`uint32(height)`. No error path.

#### 5.6.18 `Spender` (`server.go:2147-2177`)

1. `chainhash.NewHash(transaction_hash)`; failure S(`InvalidArgument`,
   `invalid transaction hash: invalid hash length of <n>, want 32`).
2. Outpoint = `{hash, index, tree 0}`. Wallet call `Spender(ctx, &out)`
   (`wallet/wallet.go:3308`, `wallet/udb/txquery.go:640-725`; the lookup
   ignores the tree).
3. On error the handler tests `errors.Is(errors.NotExist, err)` and
   `errors.Is(errors.Invalid, err)` with the arguments reversed. Both tests
   are false for every error the wallet returns (the wallet returns
   `*errors.Error` values, never a bare kind), so the intended messages
   `output is unspent` and `output is not relevant to the wallet` are never
   produced. The result is always T(err): `NotFound` with a message ending
   `item does not exist: credit is unspent` (or `missing unmined spending
   tx`, `recorded spending tx does not spend credit`), `InvalidArgument`
   with `invalid operation: output is not a credit`, or `Unknown` for kind
   `Bug`.
4. Response: `spender_transaction` = the serialized spending transaction;
   `input_index` = the index of the spending input.

#### 5.6.19 `GetCFilters` (server-streaming; `server.go:2179-2210`)

1. `decodeBlockRange`.
2. Wallet call `RangeCFiltersV2(ctx, start, end, rangeFn)`
   (`wallet/wallet.go:855`), one read-only database transaction. For each
   main chain block in range, `rangeFn(blockHash, key, filter)`:
   1. `Send` `{key: the 16-byte filter key, filter: filter.Bytes(),
      block_hash}`. A `Send` error stops the iteration.
   2. If `ctx` is done, stop with `ctx.Err()`.
3. Wallet error → T(err); else OK.

Unlike `GetTransactions`, the message is sent before the cancellation check.
The `filter` bytes are specified in section 4.3 (N varint plus filter data;
absent for an empty filter). The handler relays what the wallet stored
without validating it.

#### 5.6.20 `GetPeerInfo` (`server.go:3982-4047`)

1. `n, err = wallet.NetworkBackend()`; error → RAW(err) (`Unknown`).
2. If `n` is not a `*spv.Syncer`:
   1. If `n` is a `*chain.Syncer`: call dcrd JSON-RPC `getpeerinfo` with no
      parameters and decode the result as an array of objects with keys
      `id` (int32), `addr`, `addrlocal`, `services` (string), `version`
      (uint32), `subver`, `startingheight` (int64), `banscore` (int32).
      Error → RAW(err).
   2. Copy each element, in dcrd's order, to a `PeerInfo`.
3. If `n` is a `*spv.Syncer`: for each remote peer: `id =
   int32(peer.ID())`; `addr = RemoteAddr().String()`; `addr_local =
   LocalAddr().String()`; `services` = the service flags as an unsigned
   decimal zero-padded to at least 8 digits (`%08d`); `version` = the
   negotiated protocol version; `sub_ver` = the peer's user agent;
   `starting_height = int64(InitialHeight())`; `ban_score =
   int32(BanScore())`. Sort ascending by `id`.

#### 5.6.21 `DumpPrivateKey` (`server.go:613-626`)

1. `decodeAddress(address)`.
2. Wallet call `DumpWIFPrivateKey(ctx, addr)` (`wallet/wallet.go:3688`,
   which loads the key through the address manager); error → T(err). The
   kinds for a locked wallet, an unknown address and a watching-only wallet
   are the address manager's (spec 02); by the table in 5.3.1 they surface
   as `FailedPrecondition`, `NotFound` and `Unimplemented` when the manager
   reports `Locked`, `NotExist` and `WatchingOnly`.
3. `private_key_wif` = the WIF string.

#### 5.6.22 `BirthBlock` (`server.go:628-648`)

1. Wallet call `BirthState(ctx)`; error → S(`Internal`, `failed to get birth
   state: <err>`).
2. If no birth state is stored: S(`NotFound`, `birth block not set`). If one
   is stored with `SetFromTime` or `SetFromHeight` true (the block has not
   been resolved yet): S(`NotFound`, `birth block is pending...`).
3. Response: `hash` (32 bytes), `height`.

#### 5.6.23 `TransactionNotifications` (server-streaming; `server.go:2388-2414`)

1. Register a client with `wallet.NtfnServer.TransactionNotifications()`:
   an unbuffered channel appended to the server's client list
   (`wallet/notifications.go:488-497`). Deregistration (`Done`) is deferred.
2. Loop: wait for either a notification or `ctx` done.
   - Notification `v`: `Send` one message with `attached_blocks =
     marshalBlocks(v.AttachedBlocks)`; `detached_blocks` = the block hashes
     of `v.DetachedBlocks`; `unmined_transactions`;
     `unmined_transaction_hashes`; `detached_block_headers` =
     `marshalDetachedBlocks(v.DetachedBlocks)` (same order as
     `detached_blocks`). `Send` error → T(err).
   - `ctx` done: return OK.

Nothing is sent when the stream opens. The wallet's `NewBalances` are not
exposed. Two notification shapes exist (`wallet/notifications.go:225-356`):

- Unmined transaction accepted: `unmined_transactions` holds exactly that
  transaction, `unmined_transaction_hashes` holds every currently unmined
  hash; the block fields are empty.
- Main chain change: `attached_blocks` in connection order (ascending
  height), each with its wallet transactions; `detached_blocks` and
  `detached_block_headers` in disconnection order (descending height);
  `unmined_transaction_hashes` is the full unmined set after the change;
  `unmined_transactions` is empty.

A notification that the wallet fails to build is dropped for every client
and only logged by the wallet (error level, wallet subsystem): `Cannot fetch
unmined transaction hashes: <err>` or `Cannot determine balances for
relevant accounts: <err>` for the unmined shape
(`wallet/notifications.go:243`, `:250`), `Failed to construct attached
blocks notification: <err>` for the main-chain shape (`:344`). The stream
stays open and the client is not told that a notification was lost.
Balances are computed (and can fail) even though this RPC does not expose
them.

Delivery, ordering and backpressure are in section 7.3.

#### 5.6.24 `AccountNotifications` (server-streaming; `server.go:2416-2442`)

Same structure with `wallet.NtfnServer.AccountNotifications()`. Each
notification becomes one message: `account_number`, `account_name`,
`external_key_count`, `internal_key_count`, `imported_key_count`. The counts
come from the wallet (`wallet/notifications.go:600-627`) and are not the
same quantity as in `Accounts`: for BIP0044 accounts they are
`min(2^31, LastUsedIndex + gapLimit)` in uint32 arithmetic (an unused branch
has last-used index `0xFFFFFFFF`, giving `gapLimit − 1`); for the imported
account they are 0. This applies to `external_key_count` and
`internal_key_count` only: the test is `account_number ≤ 2^31 − 2`
(`udb.MaxAccountNum`, `wallet/udb/addressmanager.go:36`), and
`imported_key_count` is always the account's stored imported key count.

The wallet emits an account notification from exactly four operations
(`wallet/wallet.go:2130`, `:2236`, `:3734`, `:3771`): `RenameAccount`,
`NextAccount`, `ImportPrivateKey` and `ImportPublicKey` (the last has no
gRPC entry point). Deriving addresses (`NextAddress`), importing a script,
an extended public key or a voting account, and setting, unlocking or
locking account passphrases emit nothing. So, despite `api.md:2065-2066`
("account property changes, such as name and key counts"), a client does
not see key counts advance through this stream except as a side effect of
those four operations. When no account client is registered the
notification is not built (`wallet/notifications.go:603-607`).

#### 5.6.25 `ConfirmationNotifications` (bidirectional; `server.go:2444-2509`)

1. `c = wallet.NtfnServer.ConfirmationNotifications(ctx)`: registers a
   client that is removed when `ctx` is done.
2. Create `errOut`, an error channel of capacity 2.
3. Start the **request goroutine**, looping:
   1. `req, err = Recv()`; on error send it to `errOut` and exit. A client
      half-close arrives here as `io.EOF`.
   2. `decodeHashes(req.tx_hashes)`; on error send it to `errOut` and exit.
   3. If `req.stop_after < 0`: send S(`InvalidArgument`, `stop_after must be
      non-negative`) to `errOut` and exit.
   4. `c.Watch(txHashes, req.stop_after)` (blocks until its result has been
      taken by the response goroutine or `ctx` is done).
4. Start the **response goroutine**, looping:
   1. `n, err = c.Recv()`; on error send it to `errOut` and exit (`Recv`
      returns `context.Canceled` once `ctx` is done).
   2. Build one `ConfirmationNotificationsResponse` with one entry per
      element of `n`, in order: `tx_hash`; `confirmations`; `block_hash`
      (absent when nil); `block_height`.
   3. `Send`; on error send it to `errOut` and exit.
5. The handler waits for `ctx` done (return OK) or the first value on
   `errOut`: if it is or wraps `context.Canceled` return OK; if it is a
   status error return it unchanged; otherwise T(err). In particular a
   client half-close ends the RPC with `Unknown`, message `EOF`.

`Watch(txHashes, stopAfter)` (`wallet/notifications.go:820-899`):

1. An empty hash list returns immediately and produces no response.
2. In one read-only database transaction, with index `i` starting at 0 and
   the tip height read once, process `txHashes[i]` until `i` reaches the
   (shrinking) length:
   - `confs` = −1 if the transaction is unknown to the wallet (kind
     `NotExist` from `TxBlockHeight`); −1 if its block is a main chain block
     that the next block stake-invalidated (`BlockInMainChain`'s second
     result, `wallet/udb/txmined.go:936-951`; checked only when `0 < height
     < tipHeight`); otherwise the confirmation count (0 for unmined).
   - Append `{tx_hash, confirmations: confs, block_hash: nil, block_height:
     −1}`; when `confs > 0`, set `block_height` to the transaction's block
     height and `block_hash` to the main chain block hash at that height.
   - If `confs ≥ stopAfter` or `confs == −1`: remove the hash from the list
     by moving the last element into position `i` (do not advance `i`).
     Otherwise advance `i`.
3. A database error replaces the result with that error.
4. Deliver the result to the response goroutine (or give up when `ctx` is
   done), then record every hash still in the list as watched with this
   `stopAfter` (a later request for the same hash overwrites its
   `stopAfter`).

The response to a request therefore has exactly one entry per requested
hash (duplicates included), in the order produced by step 2: the first
hash, then, whenever an entry is finished immediately, the hash that was
last in the remaining list. With `stop_after = 0` every known transaction is
reported once and never watched.

On every main chain tip change the wallet calls `process(newHeight)` for
each registered client (`wallet/notifications.go:912-988`): for every watched
hash it computes an entry by the same rules, unwatches those with `confs ≥
stopAfter` or `confs == −1`, and delivers one result. The entries are in Go
map iteration order (unspecified). A result is delivered even when nothing
is watched, so a connected client receives an empty
`ConfirmationNotificationsResponse` (zero bytes of payload) on every tip
change.

#### 5.6.26 `ChangePassphrase` (`server.go:1551-1583`)

1. `key = PRIVATE` (0): wallet call `ChangePrivatePassphrase(ctx, old,
   new)`.
2. `key = PUBLIC` (1): an empty `old_passphrase` is replaced by the bytes of
   `public` (`wallet.InsecurePubPassphrase`, `wallet/wallet.go:66`), and
   likewise an empty `new_passphrase`; wallet call
   `ChangePublicPassphrase(ctx, old, new)`.
3. Any other `key`: S(`InvalidArgument`, `Unknown key type (<decimal>)`).
4. Wallet error → T(err). Response is empty.

#### 5.6.27 `RenameAccount` (`server.go:417-426`)

Wallet call `RenameAccount(ctx, account_number, new_name)`; error → T(err).
On success the wallet emits an account notification (5.6.24). Response is
empty.

#### 5.6.28 `Rescan` (server-streaming; `server.go:442-489`)

1. P-BACKEND.
2. Validation, first match wins: `begin_hash` non-empty and `begin_height ≠
   0` → S(`InvalidArgument`, `begin hash and height must not be set
   together`); `begin_height < 0` → S(`InvalidArgument`, `begin height must
   be non-negative`); `begin_hash` non-empty → must be 32 bytes, else
   S(`InvalidArgument`, `block hash has invalid length`), identify by hash;
   otherwise identify by `begin_height`.
3. Wallet call `BlockInfo(ctx, id)`; error → T(err). Only the resulting
   height is used.
4. Create `progress`, a channel of capacity 1, and start a goroutine running
   `wallet.RescanProgressFromHeight(ctx, n, height, progress)`
   (`wallet/rescan.go:419`), which closes the channel when it returns.
5. For each value received: if it carries an error → T(err) (the error has
   `Op` `wallet.RescanProgressFromHeight`; a cancelled rescan yields
   `Canceled` with message `wallet.RescanProgressFromHeight: context
   canceled`). Otherwise `Send` `{rescanned_through: ScannedThrough}`; a
   `Send` error → T(err).
6. When the channel closes: if `ctx` is done, S(`Canceled`, `rescan
   canceled`); otherwise OK.

The wallet rescans the main chain from the start height in batches of up to
2000 blocks (`wallet/rescan.go:21`) and reports once per completed batch
with `rescanned_through` = the height of the batch's last block. The genesis
block is skipped but counted.

#### 5.6.29 `NextAccount` (`server.go:497-523`)

1. If `account_name` is empty: S(`InvalidArgument`, `account name may not be
   empty`).
2. P-UNLOCK.
3. Wallet call `NextAccount(ctx, account_name)`; error → T(err).
4. `account_number` = the new account.

#### 5.6.30 `NextAddress` (`server.go:525-579`)

1. `gap_policy`: 0 → no option (the wallet's configured default policy); 1
   → error policy; 2 → ignore policy; 3 → wrap policy; any other value →
   S(`InvalidArgument`, `gap_policy=<decimal>`).
2. `kind`: 0 → wallet call `NewExternalAddress(ctx, account, opts...)`; 1 →
   `NewInternalAddress(ctx, account, opts...)`; error → T(err); any other
   value → S(`InvalidArgument`, `kind=<decimal>`).
3. Response as in `Address` step 3.

The gap policy is validated before the kind.

#### 5.6.31 `ImportPrivateKey` (`server.go:650-704`)

1. `dcrutil.DecodeWIF(private_key_wif, params.PrivateKeyID)`; failure
   S(`InvalidArgument`, `Invalid WIF-encoded private key: <err>`).
2. P-UNLOCK.
3. If `account ≠ 2147483647` (`udb.ImportedAddrAccount`,
   `wallet/udb/addressmanager.go:47`): S(`InvalidArgument`, `Only the
   imported account accepts private key imports`).
4. If `scan_from < 0`: S(`InvalidArgument`, `Attempted to scan from a
   negative block height`).
5. If `scan_from > 0` and `rescan` is true: S(`InvalidArgument`, `Passed a
   rescan height without rescan set`). (The condition is inverted relative
   to the message; see Q10.)
6. P-BACKEND.
7. Wallet call `ImportPrivateKey(ctx, wif)`; error → T(err).
8. If `rescan`: start a detached goroutine running
   `wallet.RescanFromHeight(context.Background(), n, scan_from)`; its result
   is discarded and it is not tied to the RPC.
9. Empty response.

#### 5.6.32 `ImportExtendedPublicKey` (`server.go:706-729`)

1. `hdkeychain.NewKeyFromString(xpub, params)`; error → RAW(err)
   (`Unknown`).
2. P-BACKEND.
3. Wallet call `ImportXpubAccount(ctx, account_name, xpub)`; error → T(err).
4. If `rescan`: detached `RescanFromHeight(context.Background(), n,
   scan_from)`. `scan_from` is not validated.
5. Empty response.

#### 5.6.33 `ImportVotingAccountFromSeed` (`server.go:786-867`)

1. `len(seed)` must be in [16, 64], else S(`InvalidArgument`, `invalid seed
   length`).
2. `scan_from < 0` → S(`InvalidArgument`, `Attempted to scan from a negative
   block height`).
3. `scan_from > 0` and `rescan` → S(`InvalidArgument`, `Passed a rescan
   height without rescan set`).
4. `discover_from < 0` → S(`InvalidArgument`, `Attempted to discover usage
   from a negative block height`).
5. `discover_from > 0` and `discover_usage` → S(`InvalidArgument`, `Passed a
   discover usage height without discover usage set`).
6. If `discover_usage` or `rescan`: P-BACKEND, but its failure is rewrapped
   as S(`Unknown`, `Unable to retrieve network backend. Error: rpc error:
   code = FailedPrecondition desc = wallet is not associated with a
   consensus server RPC client`).
7. Wallet call `VotingXprivFromSeed(seed)`; error → T(err).
8. Wallet call `ImportVotingAccount(ctx, xpriv, passphrase, name)`; error →
   T(err). `passphrase` is the new account's own passphrase; the wallet is
   not unlocked with it.
9. If `discover_usage`: start block = the network's genesis hash (because of
   step 5 `discover_from` is always 0 here, so the lookup by height at
   `:842-849` is unreachable); gap limit = `discover_gap_limit` converted to
   uint32 if non-zero, else the wallet's gap limit; wallet call
   `DiscoverActiveAddresses(ctx, n, &start, false, gapLimit)`; error →
   RAW(err). The account has already been imported at this point.
10. If `rescan`: detached `RescanFromHeight(context.Background(), n,
    scan_from)`.
11. `account` = the new account number.

`discover_gap_limit` is `int32` and is not range-checked: a negative value
is reinterpreted as a uint32 (−1 becomes 4,294,967,295) and passed to the
wallet as the gap limit (`server.go:851-854`); `api.md:1230` documents
`InvalidArgument` for it. `seed` and `passphrase` are zeroed on every exit
path (`:789-792`).

#### 5.6.34 `ImportScript` (`server.go:731-784`)

`passphrase` is ignored.

1. Analyse `script` as a version 0 script: its type and the addresses it
   pays (`stdscript.ExtractAddrs`), and the number of required signatures
   (`stdscript.DetermineRequiredSigs`).
2. For each extracted address, wallet call `HaveAddress(ctx, a)`; error →
   T(err). Count the addresses the wallet has.
3. `redeemable` = the script is multisig and the count ≥ required
   signatures.
4. If not redeemable and `require_redeemable`: S(`FailedPrecondition`, `The
   script is not redeemable by the wallet`).
5. `scan_from < 0` → S(`InvalidArgument`, `Attempted to scan from a negative
   block height`).
6. `scan_from > 0` and `rescan` → S(`InvalidArgument`, `Passed a rescan
   height without rescan set`).
7. P-BACKEND.
8. Wallet call `ImportScript(ctx, script)`. A kind `Exist` error is
   tolerated; any other error → T(err).
9. If the import succeeded (not `Exist`) and `rescan`: detached
   `RescanFromHeight(context.Background(), n, scan_from)`.
10. `p2sh_address` = `NewAddressScriptHashV0(script, params).String()`
    (error → T(err)); `redeemable`.

The script is always analysed as version 0 (`scriptVersionAssumed`; the
source carries a TODO to make the version a request field,
`server.go:734-735`). The own-address counter and the required-signature
count are both `uint16`; the source notes that it does not reject the
degenerate case of no addresses and zero required signatures (`:738`), so a
script classified as multisig with zero required signatures is reported
redeemable. The handler does not reject an empty `script`: it is
non-standard, hence not redeemable, and goes on to the wallet import.

#### 5.6.35 `FundTransaction` (`server.go:1098-1140`)

1. Wallet call `SelectInputs(ctx, target_amount, account,
   required_confirmations)` (`wallet/utxos.go:22`). An error of kind
   `InsufficientBalance` is ignored; any other error → T(err) (unknown
   account: `NotFound`, message `wallet.SelectInputs: item does not exist:
   account not found`).
2. For each selected input, in the wallet's order: wallet call
   `OutputInfo(ctx, &outpoint)` (a separate database read per input); error
   → T(err). Build `PreviousOutput`: `transaction_hash`, `output_index`,
   `tree = int32(outpoint.Tree)`, `amount`, `pk_script` (the previous output
   script returned by the selection), `receive_time` (Unix seconds),
   `from_coinbase`.
3. If `include_change_script` and the selected total exceeds
   `target_amount`: wallet call `NewChangeAddress(ctx, account)`; error →
   T(err); `change_pk_script` = its payment script.
4. `total_amount` = the selected total.

`include_immature_coinbases` is ignored. `target_amount = 0` selects every
eligible output; a negative target selects none
(`wallet/udb/txmined.go:3100-3101`). The selection order is randomised by the
wallet (Q20).

#### 5.6.36 `UnspentOutputs` (server-streaming; `server.go:1057-1096`)

1. Wallet call `SelectInputs(ctx, target_amount, account,
   required_confirmations)`, with the same error handling as
   `FundTransaction` step 1.
2. For each selected input in order: if `ctx` is done S(`Canceled`,
   `unspentoutputs cancelled`). Otherwise `OutputInfo` (error → T(err)),
   build the message with the same fields as `PreviousOutput`, add `amount`
   to a running sum and set `amount_sum` to it, `Send` (error → T(err)).
3. OK.

The selection is complete before the first message is sent; no database
transaction is held across sends. `include_immature_coinbases` is ignored.

#### 5.6.37 `ConstructTransaction` (`server.go:1192-1265`)

1. If `non_change_outputs` is empty and `change_destination` is absent:
   S(`InvalidArgument`, `non_change_outputs and change_destination may not
   both be empty or null`).
2. For each output in order: `decodeDestination(output.destination)` (its
   status error is returned as is); append a transaction output `{value:
   amount, version, pkScript}`. Amounts are not validated here.
3. `output_selection_algorithm`: 0 → the wallet's default algorithm; 1 →
   all outputs; any other value → S(`InvalidArgument`, `unknown output
   selection algorithm`).
4. Fee rate: `fee_per_kb` atoms per kB if non-zero, else 10,000
   (`txrules.DefaultRelayFeePerKb`, `wallet/txrules/rules.go:38`). The
   wallet's configured relay fee is not used.
5. If `change_destination` is present: `decodeDestination` it; a failure is
   passed through T(), giving `InvalidArgument` with the nested message
   `rpc error: code = InvalidArgument desc = <original>` (5.3.2). The change
   source returns that script and version. If absent, the wallet derives an
   internal change address when change is needed.
6. Wallet call `NewUnsignedTransaction(ctx, outputs, feeRate,
   source_account, required_confirmations, algo, changeSource, nil)`
   (`wallet/createtx.go:82`); error → T(err) (insufficient funds:
   `ResourceExhausted`).
7. If the result has a change output (`ChangeIndex ≥ 0`), swap it with the
   output at a uniformly random index in [0, number of outputs), which may
   be its own index (`RandomizeChangePosition`,
   `wallet/txauthor/author.go:171-181`). The displaced output takes the
   change output's former position; all other outputs keep the request
   order.
8. Response: `unsigned_transaction` = the serialized transaction;
   `total_previous_output_amount` = total input value;
   `total_output_amount` = sum of output values; `estimated_signed_size`;
   `change_index` (−1 when there is no change output).

#### 5.6.38 `SignTransaction` (`server.go:1585-1645`)

1. Deserialize `serialized_transaction`; failure S(`InvalidArgument`, `Bytes
   do not represent a valid raw transaction: <err>`).
2. P-UNLOCK.
3. For each additional script: `transaction_hash` must be 32 bytes, else
   S(`InvalidArgument`, `Invalid transaction hash length for script
   <script>, expected 32 got <n>`), where `<script>` is Go's `%v` rendering
   of the protobuf message (not byte-stable; Q22). Record `{hash,
   output_index, int8(tree)} → pk_script`; later duplicates overwrite
   earlier ones.
4. Wallet call `SignTransaction(ctx, &tx, SigHashAll, additionalScripts,
   nil, nil)` (`wallet/wallet.go:4773`); error → T(err).
5. Response: `transaction` = the serialized transaction after signing;
   `unsigned_input_indexes` = the input index of every signature error the
   wallet reported, in the wallet's order.

Inputs that could not be signed do not fail the RPC.

#### 5.6.39 `SignTransactions` (`server.go:1647-1712`)

1. P-UNLOCK.
2. Additional scripts as in `SignTransaction` step 3 (shared by all
   transactions).
3. For each transaction in order: deserialize (failure aborts the whole RPC
   with the same `InvalidArgument` message); sign with `SigHashAll` (error →
   T(err)); append `{transaction, unsigned_input_indexes}`.
4. Response `transactions` has one entry per request transaction, in order.

#### 5.6.40 `CreateSignature` (`server.go:1714-1754`)

1. Deserialize `serialized_transaction`; failure as above.
2. If `input_index ≥` the number of inputs: S(`InvalidArgument`,
   `transaction input <input_index> does not exist`).
3. P-UNLOCK.
4. `decodeAddress(address)`.
5. `hashType = byte(hash_type)` (the enum's numeric value truncated to 8
   bits; no validation). Wallet call `CreateSignature(ctx, &tx, input_index,
   addr, hashType, previous_pk_script)`; error → T(err).
6. Response: `signature`, `public_key` (serialized compressed).

#### 5.6.41 `PublishTransaction` (`server.go:1756-1788`)

1. P-BACKEND.
2. Deserialize `signed_transaction`; failure S(`InvalidArgument`, `Bytes do
   not represent a valid raw transaction: <err>`).
3. Unless the wallet allows high fees (`--allowhighfees`):
   `txrules.TxPaysHighFees(tx)`; an error (an input with negative `ValueIn`,
   kind `Invalid`) → T(err); a true result → T(errors.E(Policy, "high
   fees")), which is `Unknown` with message `policy violation: high fees`.
4. Wallet call `PublishTransaction(ctx, tx, n)`; error → T(err).
5. `transaction_hash` = the published transaction's hash.

#### 5.6.42 `PublishUnminedTransactions` (`server.go:428-440`)

P-BACKEND; wallet call `PublishUnminedTransactions(ctx, n)`; error → T(err);
empty response.

#### 5.6.43 `PurchaseTickets` (`server.go:1791-1940`)

1. `n, err = wallet.NetworkBackend()`; error → RAW(err).
2. If `vsp_host` or `vsp_pubkey` is non-empty: `wallet.VSP({URL: vsp_host,
   PubKey: vsp_pubkey, Policy: {MaxFee: wallet.VSPMaxFee(), FeeAcct:
   account, ChangeAcct: change_account}})`; failure S(`InvalidArgument`,
   `VSP client failed to start. Error: <err>`). VSP clients are cached by
   URL (`wallet/vspclientloader.go:12-26`): if a client for `vsp_host`
   exists it is returned as is and the pubkey and policy in this request are
   not applied.
3. If `num_tickets == 0`: S(`InvalidArgument`, `Zero or negative number of
   tickets given`).
4. If `tx_fee < 0`: S(`InvalidArgument`, `Negative fees per KB given`).
   `tx_fee` is not used otherwise; `ticket_fee` is not read at all.
5. If `enable_mixing`:
   1. `wallet.AccountName(ctx, mixed_account)`; error → S(`InvalidArgument`,
      `Mixing requested, but error on mixed account: <err>`).
   2. `mixed_account_branch` must be 0 or 1, else S(`InvalidArgument`,
      `MixedAccountBranch should be 0 or 1.`).
   3. `wallet.AccountName(ctx, mixed_split_account)`; error →
      S(`InvalidArgument`, `Mixing requested, but error on
      mixedSplitAccount: <err>`).
   4. `wallet.AccountName(ctx, change_account)`; error →
      S(`InvalidArgument`, `Mixing requested, but error on changeAccount:
      <err>`).
6. Build the wallet request: `Count = num_tickets`; `SourceAccount =
   account`; `MinConf = int32(required_confirmations)`; `Expiry =
   int32(expiry)`; `DontSignTx = dont_sign_tx`; `Mixing = enable_mixing`;
   `MixedAccount`, `MixedAccountBranch`, `MixedSplitAccount` = the request
   values when mixing is enabled, else 0; `ChangeAccount = change_account`
   (always); `VSPClient` = the client from step 2 or none. `VotingAccount`:
   `voting_account` when `use_voting_account`; else `mixed_account` when
   mixing; else `account`.
7. If `dont_sign_tx` is false: P-UNLOCK.
8. Wallet call `PurchaseTickets(ctx, n, request)` (`wallet/wallet.go:1585`);
   error → RAW(err). Every wallet failure, including insufficient balance
   and a locked wallet, is therefore `Unknown`.
9. Response: `ticket_hashes` = each ticket hash (32 bytes); `tickets` =
   each ticket transaction serialized (signed or not); `split_tx` = the
   serialized split transaction. A serialization failure of a ticket is
   RAW; of the split transaction S(`Unknown`, `Error Serializing split tx:
   <err>`).

#### 5.6.44 `LoadActiveDataFilters` (`server.go:1942-1956`)

P-BACKEND; wallet call `LoadActiveDataFilters(ctx, n, false)`; error →
T(err); empty response.

#### 5.6.45 `SignMessage` (`server.go:2018-2036`, helper `:1994-2016`)

1. P-UNLOCK.
2. `signMessage(address, message)`:
   1. `decodeAddress`.
   2. The address must be a secp256k1 P2PK or P2PKH version 0 address, else
      S(`InvalidArgument`, `address must be secp256k1 P2PK or P2PKH`).
   3. Wallet call `SignMessage(ctx, message, addr)`; error → T(err).
3. `signature` = the result.

#### 5.6.46 `SignMessages` (`server.go:2038-2068`)

1. P-UNLOCK.
2. For each message in order run `signMessage`. Append `{signature, error}`
   where `error` is empty on success and otherwise the error's string. Since
   `signMessage` returns status errors, the string has the form `rpc error:
   code = <Code> desc = <message>`.
3. The RPC itself succeeds; `replies` has one entry per message.

#### 5.6.47 `ValidateAddress` (`server.go:2070-2145`)

1. `decodeAddress(address)`; on failure return an empty response
   (`is_valid` false) with status OK.
2. `is_valid = true`. From the address's payment script and version:
   `script_type = scProto(type)`; `pay_to_addr_script` = the payment script.
   If the address is a public-key address: `pub_key` = its serialized
   public key and `pub_key_addr` = the address string itself. If the script
   type is script-hash: `is_script = true`.
3. Wallet call `KnownAddress(ctx, addr)`. Kind `NotExist` → return the
   response built so far. Any other error → RAW(err).
4. Wallet call `AccountNumber(ctx, ka.AccountName())`; error → RAW(err).
5. `is_mine = true`; `account_number`.
6. If the known address is a P2PKH address: `pub_key` = its compressed
   public key; `pub_key_addr` = the pubkey-address string for that key
   (constructor error → RAW). If it is a P2SH address with a known redeem
   script: `pk_script_addrs` = the addresses extracted from the redeem
   script, in script order; `script_type` is replaced by the redeem script's
   type; if that type is multisig, `sigs_required` = the required signature
   count.
7. If the known address is a BIP0044 address: `is_internal` = branch is 1;
   `index` = the child index.

#### 5.6.48 `CommittedTickets` (`server.go:1958-1992`)

1. `decodeHashes(tickets)`.
2. Wallet call `CommittedTickets(ctx, hashes)`; error → T(err).
3. If the two returned lists differ in length: S(`Internal`, `impossible
   condition: ticket and address count unequal`).
4. `ticketAddresses` = one `{ticket, address}` per returned pair, in the
   wallet's order.

#### 5.6.49 `SweepAccount` (`server.go:969-1018`)

1. Fee rate starts as the wallet's configured relay fee
   (`wallet.RelayFee()`).
2. If `fee_per_kb < 0`: S(`InvalidArgument`, `fee per kb argument cannot be
   negative`).
3. If `fee_per_kb > 0`: `dcrutil.NewAmount(fee_per_kb)`; `fee_per_kb` is in
   DCR per kB and is converted to atoms by multiplying by 1e8 and rounding
   half up. +Infinity fails with S(`InvalidArgument`, `invalid coin
   amount`). NaN satisfies neither comparison and leaves the default.
4. Wallet call `AccountNumber(ctx, source_account)` (the account is given by
   name); error → T(err).
5. `decodeAddress(destination_address)`; its error is passed through T(),
   giving `InvalidArgument` with the nested message `rpc error: code =
   InvalidArgument desc = invalid address <a>: <err>`.
6. Wallet call `NewUnsignedTransaction(ctx, nil, feeRate, account,
   int32(required_confirmations), OutputSelectionAlgorithmAll,
   changeSource, nil)` where the change source pays the destination; error
   → T(err).
7. Response: `unsigned_transaction`, `total_previous_output_amount`,
   `total_output_amount`, `estimated_signed_size`.

#### 5.6.50 `AbandonTransaction` (`server.go:3774-3786`)

1. `chainhash.NewHash(transaction_hash)`; failure S(`InvalidArgument`,
   `invalid transaction hash: invalid hash length of <n>, want 32`).
2. Wallet call `AbandonTransaction(ctx, hash)`; error → T(err).
3. Empty response.

#### 5.6.51 `SignHashes` (`server.go:3736-3772`)

1. P-UNLOCK.
2. `decodeAddress(address)`.
3. Address type check as in `signMessage` step 2.2.
4. Wallet call `SignHashes(ctx, hashes, addr)`; error → T(err).
5. `public_key`, `signatures` (one per hash, same order).

#### 5.6.52 `GetCoinjoinOutputspByAcct` (`server.go:3848-3866`)

1. Wallet call `GetCoinjoinTxsSumbByAcct(ctx)` returning a map from account
   number to count; error → T(err).
2. `data` = one `{account_number, coinjoin_txs_sum: int32(count)}` per map
   entry, in Go map iteration order (unspecified).

#### 5.6.53 `SetAccountPassphrase` (`server.go:3868-3910`)

1. Wallet call `AccountName(ctx, account_number)`: kind `NotExist` →
   S(`NotFound`, `account not found`); other error → RAW(err).
2. Wallet call `AccountHasPassphrase(ctx, account_number)`; error →
   RAW(err).
3. If the account has its own passphrase and `account_passphrase` is
   non-empty: `wallet.UnlockAccount(ctx, account_number,
   account_passphrase)`; error → T(err); defer `LockAccount` (its error is
   discarded).
   Otherwise, if `wallet_passphrase` is non-empty: P-UNLOCK with it.
   Otherwise no unlock.
4. Wallet call `SetAccountPassphrase(ctx, account_number,
   new_account_passphrase)`; error → T(err). An empty new passphrase
   re-encrypts the account with the wallet's global passphrase
   (`wallet/wallet.go:1746-1755`).
5. Empty response.

#### 5.6.54 `UnlockAccount` (`server.go:3912-3926`)

1. `AccountName` check: `NotExist` → S(`NotFound`, `account not found`);
   other error → RAW(err).
2. Wallet call `UnlockAccount(ctx, account_number, passphrase)`; error →
   T(err).
3. Empty response.

#### 5.6.55 `LockAccount` (`server.go:3928-3942`)

1. `AccountName` check: `NotExist` → S(`NotFound`, `account not found`);
   other error → T(err).
2. Wallet call `LockAccount(ctx, account_number)`; error → T(err).
3. Empty response.

#### 5.6.56 `UnlockWallet` (`server.go:3962-3971`)

Wallet call `Unlock(ctx, passphrase, nil)`: unlocks with no timeout,
cancelling any existing timeout; error → T(err). Empty response.

#### 5.6.57 `LockWallet` (`server.go:3973-3978`)

`wallet.Lock()`. No error path. Empty response.

#### 5.6.58 `AccountUnlocked` (`server.go:3944-3960`)

1. `AccountName` check as in `LockAccount`.
2. Wallet call `AccountUnlocked(ctx, account_number)`; error → T(err). An
   account without its own passphrase is kind `Invalid` → `InvalidArgument`,
   message `wallet.AccountUnlocked: invalid operation: account is not
   individually encrypted`.
3. `unlocked`.

#### 5.6.59 `SyncVSPFailedTickets` (`server.go:4079-4118`)

1. Wallet call `GetVSPTicketsByFeeStatus(ctx, 2)` (errored); error →
   RAW(err).
2. `wallet.VSP({URL: vsp_host, PubKey: vsp_pubkey, Policy: {MaxFee:
   VSPMaxFee(), FeeAcct: account, ChangeAcct: change_account}})`; failure
   S(`InvalidArgument`, `VSP client failed to start. Error: <err>`).
3. For each ticket hash: `wallet.NewVSPTicket(ctx, &hash)` then
   `vspClient.Process(ctx, ticket, new empty transaction)`. Errors from
   either are silently skipped.
4. Empty response, status OK.

#### 5.6.60 `GetVSPTicketsByFeeStatus` (`server.go:4049-4077`)

1. `fee_status` 0..3 maps to the same number; any other value →
   S(`InvalidArgument`, `fee status=<decimal>`).
2. Wallet call `GetVSPTicketsByFeeStatus(ctx, status)`; error → RAW(err).
3. `tickets_hashes` = the hashes, in the wallet's order, which is Go map
   iteration order (`wallet/wallet.go:5625-5630`).

#### 5.6.61 `ProcessManagedTickets` (`server.go:4120-4148`)

1. `wallet.VSP(...)` with `FeeAcct: fee_account`, `ChangeAcct:
   change_account`; failure S(`InvalidArgument`, `VSP client failed to
   start. Error: <err>`).
2. Wallet call `ProcessedTickets(ctx)`; error → S(`Unknown`,
   `ProcessedTickets failed. Error: <err>`).
3. `vspClient.ProcessManagedTickets(ctx, tickets)`; error → S(`Unknown`,
   `ProcessManagedTickets failed. Error: <err>`).
4. Empty response.

#### 5.6.62 `ProcessUnmanagedTickets` (`server.go:4150-4175`)

1. `wallet.VSP(...)` as above.
2. Wallet call `UnprocessedTickets(ctx)`; error → S(`Unknown`, `failed to
   retrieve unmanaged tickets. Error: <err>`).
3. `vspClient.ProcessUnprocessedTickets(ctx, tickets)` (no error result).
4. Empty response.

#### 5.6.63 `SetVspdVoteChoices` (`server.go:4177-4252`)

1. `wallet.VSP(...)` as above.
2. Treasury key choices: for each entry of `wallet.TreasuryKeyPolicies()`,
   key = lower-case hex of the Pi key, value = `yes`, `no`, or `abstain`
   for any other policy.
3. TSpend choices: for each transaction of `wallet.GetAllTSpends(ctx)`, key
   = the transaction hash string, value = `yes`, `no`, or the empty string
   for any other policy returned by `wallet.TSpendPolicy(&hash, nil)`.
4. `wallet.ForUnspentUnexpiredTickets(ctx, f)` where `f(hash)`:
   `AgendaChoices(ctx, hash)`; `VSPHostForTicket(ctx, hash)`; if the host
   equals `vsp_host`: `NewVSPTicket(ctx, hash)` then
   `vspClient.SetVoteChoice(ctx, ticket, choices, tspendChoices,
   treasuryChoices)`. Any error from `f` is collected and the iteration
   continues (`wallet/wallet.go:5705-5747`).
5. If any error was collected: S(`Unknown`, `ForUnspentUnexpiredTickets
   failed. Error: <joined errors, newline-separated>`). Because
   `VSPHostForTicket` fails with `NotExist` for a ticket without VSP
   records, the RPC reports failure whenever the wallet holds such a
   ticket, even though the choices of matching tickets were submitted.
6. Otherwise empty response.

#### 5.6.64 `GetTrackedVSPTickets` (`server.go:4269-4284`, `:4254-4267`)

For each entry of `wallet.AllVSPs()` (map from host to client, Go map
order): `{host, tickets}` where `tickets` = one entry per
`vspClient.TrackedTickets()` element: `ticket_hash`, `commitment_address`
and `voting_address` (address strings), `state = uint32(State)`, `fee`
(atoms), `fee_hash` (32 bytes, all zero when no fee transaction exists). No
error path.

#### 5.6.65 `DiscoverUsage` (`server.go:4286-4312`)

1. P-BACKEND, failure rewrapped as S(`Unknown`, `Unable to retrieve network
   backend. Error: rpc error: code = FailedPrecondition desc = wallet is not
   associated with a consensus server RPC client`).
2. Start block = the genesis hash, or `starting_block_hash` when non-empty;
   an invalid length is S(`Unknown`, `Invalid starting block hash provided.
   Error: invalid hash length of <n>, want 32`).
3. Gap limit = `gap_limit` if non-zero, else the wallet's gap limit.
4. Wallet call `DiscoverActiveAddresses(ctx, n, &start, discover_accounts,
   gapLimit)`; error → RAW(err).
5. Empty response.

## 6. Constants, limits and defaults

| Name | Value | Source | What depends on it |
|---|---|---|---|
| `semverString` | `"10.0.0"` | `server.go:64` | `VersionResponse.version_string`; clients gate features on it |
| `semverMajor`, `semverMinor`, `semverPatch` | 10, 0, 0 | `server.go:65-67` | `VersionResponse` |
| `scriptVersionAssumed` | 0 | `server.go:72` | `ImportScript` analysis |
| proto package | `walletrpc` | `api.proto:3` | method paths, `serviceMap` keys |
| service name keys | `walletrpc.VersionService`, `walletrpc.WalletService` (and nine more) | `server.go:288-300` | readiness gate |
| gRPC default port | mainnet 9111, testnet3 19111, simnet 19558 | `internal/netparams/params.go:25`, `:34`, `:43` | default listeners |
| TLS minimum version | TLS 1.2 | `rpcserver.go:274` | handshake |
| client authentication | `RequireAndVerifyClientCert` | `rpcserver.go:322` | every gRPC connection |
| client CA file default | `<appdata>/clients.pem` | `config.go:80`, `:460-461` | whether gRPC starts at all |
| max received message | 4,194,304 bytes | `grpc@v1.83.2/server.go:61` | large `SignTransactions`, `PublishTransaction`, `ImportScript` requests |
| wallet error separator | `":: "` | `dcrwallet.go:38` | every T() message |
| `errOut` capacity | 2 | `server.go:2446` | `ConfirmationNotifications` goroutine exit |
| rescan progress channel capacity | 1 | `server.go:469` | `Rescan` |
| unlock timeout channel capacity | 1 | e.g. `server.go:507` | P-UNLOCK deferred send never blocks |
| `maxBlocksPerRescan` | 2000 | `wallet/rescan.go:21` | `RescanResponse` cadence |
| `txrules.DefaultRelayFeePerKb` | 10,000 atoms/kB | `wallet/txrules/rules.go:38` | `ConstructTransaction` default fee |
| high fee threshold | fee above the fee for the transaction's size at 1000 × `DefaultRelayFeePerKb` | `wallet/txrules/rules.go:118-145` | `PublishTransaction` policy check |
| `udb.ImportedAddrAccount` | 2,147,483,647 (2^31 − 1) | `wallet/udb/addressmanager.go:47` | `ImportPrivateKey` account check |
| `hdkeychain.MinSeedBytes`, `MaxSeedBytes` | 16, 64 | `dcrd/hdkeychain/v3@v3.1.3/extendedkey.go:40`, `:44` | `ImportVotingAccountFromSeed` |
| `wallet.InsecurePubPassphrase` | `"public"` | `wallet/wallet.go:66` | `ChangePassphrase` with `PUBLIC` |
| `chainhash.HashSize` | 32 | `dcrd/chaincfg/chainhash@v1.0.5/hash.go:14` | every hash field |
| `gcs.KeySize` | 16 | `dcrd/gcs/v4@v4.1.1/gcs.go:69` | `GetCFiltersResponse.key` |
| `dcrutil.BlockValid` | `0x0001` | `dcrd/dcrutil/v4@v4.0.3/bitflags.go:17` | `approves_parent` |
| branch numbers | external 0, internal 1 | `wallet/udb/addressmanager.go:80`, `:85` | `Address`, `ValidateAddress.is_internal` |
| peer services format | `%08d` of the unsigned flags | `server.go:4033` | `GetPeerInfo` in SPV mode |
| log subsystem | `GRPC` | `internal/loggers/loggers.go:52`, `log.go:54` | interceptor and library logs; `--debuglevel` |
| pipe message type | `grpclistener` | `ipc.go:159` | parent processes reading the listener address |
| keepalive enforcement | 5 min minimum PING interval with active streams, 2 h without; more than 2 strikes closes the connection | `grpc@v1.83.2/internal/transport/defaults.go:40`, `internal/transport/http2_server.go:881-882` | clients that send HTTP/2 keepalive PINGs (Q38) |
| server keepalive | PING after 2 h without activity, 20 s to answer | `grpc@v1.83.2/internal/transport/defaults.go:38-39` | idle connections through NAT or proxies |
| `udb.MaxAccountNum` | 2,147,483,646 (2^31 − 2) | `wallet/udb/addressmanager.go:36` | which accounts get BIP0044 key counts in `AccountNotifications` |
| seed and passphrase zeroing | request buffers overwritten in `NextAccount`, `ImportPrivateKey`, `ImportVotingAccountFromSeed`, `GetAccountExtendedPrivKey`, `ChangePassphrase`, `SignTransaction`, `SignTransactions`, `CreateSignature`, `SetAccountPassphrase`, `UnlockWallet`; not in `PurchaseTickets`, `SignMessage`, `SignMessages`, `SignHashes`, `UnlockAccount` | `server.go:500`, `:653`, `:789-792`, `:1283`, `:1554-1557`, `:1599`, `:1649`, `:1717`, `:3890`, `:3896`, `:3965` | nothing observable; hygiene only |

## 7. Concurrency and lifecycle

### 7.1 Goroutines

| Goroutine | Started by | Ends when |
|---|---|---|
| one `Serve` loop per listener | `startRPCServers` (`rpcserver.go:338-346`) | `Stop()` |
| per connection and per RPC | the gRPC library; each RPC handler runs on its own goroutine, with no stream worker pool and no configured stream limit | connection close, handler return |
| request and response goroutines | `ConfirmationNotifications` (`server.go:2447`, `:2466`) | first error; the response goroutine exits when `ctx` is done, the request goroutine when `Recv` fails after the stream ends |
| client cleanup | `NotificationServer.ConfirmationNotifications` (`wallet/notifications.go:768-782`) | `ctx` done |
| rescan worker | `Rescan` (`server.go:470`) | rescan finishes, fails or `ctx` is cancelled |
| detached rescans | `ImportPrivateKey`, `ImportExtendedPublicKey`, `ImportScript`, `ImportVotingAccountFromSeed` | rescan completes; uses `context.Background()`, so it outlives the RPC and is not cancelled by client disconnect |
| drain and deregister | `Done()` of a transaction or account notification client: one goroutine drains the channel, one removes it from the server and closes it (`wallet/notifications.go:502-523`, `:652-671`) | deregistration |

### 7.2 Locks owned by this subsystem

Only `readyMtx`, in the five wallet-dependent services (Wallet, AccountMixer,
TicketBuyer, Voting, Network): write-locked by their `StartXxxService`,
read-locked by `checkReady`. The other four `StartXxxService` functions
(`WalletLoader`, `Agenda`, `MessageVerification`, `DecodeMessage`) store
their fields without any lock. Handlers read `s.wallet` without the lock, after the
interceptor has observed `ready`; the field is written once, before `ready`
becomes observable to other goroutines (same critical section).

Handlers do not serialise against each other. Two concurrent RPCs that
both use P-UNLOCK on a locked wallet interact through the wallet's
passphrase timeout (`wallet/wallet.go:1804-1832`): the second call's
`Unlock` cancels the first call's timeout and installs its own. The first
handler's deferred send is then only drained and locks nothing; the wallet
is locked when the second handler returns, even if the first is still
running (its remaining key operations then fail with kind `Locked`), and
stays unlocked until then if the first returns earlier.

### 7.3 Notification delivery

`TransactionNotifications` and `AccountNotifications`:

- The client channel is unbuffered. The wallet delivers to every registered
  client in registration-list order while holding the notification server
  mutex (`wallet/notifications.go:258-260`, `:351-355`, `:624-626`). The
  handler receives the value and then calls `Send`, which blocks under
  HTTP/2 flow control when the client does not read.
- No notification is dropped or coalesced on account of a slow client. A
  client that stops reading blocks the wallet goroutine that is notifying
  (block connection, transaction acceptance, account changes) and every
  other notification client behind it, until the client reads or
  disconnects.
- Each client receives notifications in the order the wallet produced them.
  There is no ordering guarantee between different streams.
- Notifications produced before the stream was registered are not replayed.
- When no transaction-notification client is registered the wallet skips
  building notifications entirely.
- A notification whose construction fails in the wallet is dropped for all
  clients (5.6.23, Q37); this is the only way a notification is lost.
- On disconnect the handler returns, `Done` drains and closes the channel.

`ConfirmationNotifications`: results are handed over an unbuffered channel;
`Watch` blocks the request goroutine, and tip-change processing blocks the
wallet's tip-change notifier (which waits for all confirmation clients)
until the response goroutine takes the result or `ctx` is done.

### 7.4 Streaming and database transactions

`GetTransactions`, `GetTickets` and `GetCFilters` call `Send` from inside
the wallet's read-only database transaction; a slow client keeps that
transaction open for the duration of the stream. `UnspentOutputs` and
`Rescan` do not hold a transaction across sends.

### 7.5 Cancellation

- Unary handlers pass `ctx` to the wallet; wallet calls that honour it
  return errors wrapping `context.Canceled`, which T() maps to `Canceled`.
- `GetTransactions` checks `ctx` before each send; `GetTickets` after each
  block; `GetCFilters` after each send; `UnspentOutputs` before each output
  (S(`Canceled`, `unspentoutputs cancelled`)).
- Notification streams return OK when `ctx` is done.
- `Rescan` returns T(err) with a `Canceled` code when the rescan observes
  the cancellation, or S(`Canceled`, `rescan canceled`) when the progress
  channel closes without an error value while `ctx` is done.
- Detached rescans are not cancellable through the API.

### 7.6 Start-up and shutdown ordering

1. Start: `startRPCServers` creates the server, registers services, starts
   the four wallet-independent services, then begins serving. From the
   first accepted connection, `VersionService`, `WalletLoaderService`,
   `SeedService`, `AgendaService`, `MessageVerificationService` and
   `DecodeMessageService` answer; the wallet-dependent services answer
   `FailedPrecondition`.
2. Wallet load: the loader callback starts `WalletService`,
   `TicketBuyerService`, `AccountMixerService`, `NetworkService`,
   `VotingService`, in that order, on the goroutine that loaded the wallet.
3. Shutdown (`dcrwallet.go:395-399`, deferred in `run`): warn `Stopping gRPC
   server...`, call `Stop()` (not `GracefulStop`: listeners and all
   connections are closed immediately and active RPC contexts are
   cancelled; the call does not wait for handlers to return), info `gRPC
   server shutdown`. The JSON-RPC server's deferred stop runs before this
   one (spec 16 section 5.8).

## 8. Errors

### 8.1 Status codes produced

| Code | Produced by |
|---|---|
| `OK` | success; notification streams on client cancellation |
| `Canceled` (1) | T() of an error wrapping `context.Canceled`; `Rescan`, `UnspentOutputs` explicit messages; RAW context errors |
| `Unknown` (2) | T() fallback (kinds `Bug`, `IO`, `Encoding`, `ScriptFailure`, `Policy`, `DoubleSpend`, `Protocol`, `Consensus`, `Deployment`, unclassified errors); every RAW wallet error; explicit uses in `ImportVotingAccountFromSeed`, `DiscoverUsage`, `PurchaseTickets`, `ProcessManagedTickets`, `ProcessUnmanagedTickets`, `SetVspdVoteChoices`; `ConfirmationNotifications` on client half-close |
| `InvalidArgument` (3) | explicit validation failures; kinds `Invalid`, `Passphrase`, `Seed`; `hdkeychain.ErrInvalidSeedLen` |
| `DeadlineExceeded` (4) | RAW (an error wrapping `context.DeadlineExceeded` returned untranslated); T() of a `Send` status error carrying that code (5.3.1 rule 4); the library itself when the client deadline passes. T() of a bare or wallet-wrapped `context.DeadlineExceeded` is `Unknown`. |
| `NotFound` (5) | kind `NotExist`; `BirthBlock`; the `account not found` checks |
| `AlreadyExists` (6) | kind `Exist` |
| `PermissionDenied` (7) | kind `Permission` |
| `ResourceExhausted` (8) | kind `InsufficientBalance`; the library for oversized requests |
| `FailedPrecondition` (9) | service not ready; P-BACKEND; kind `Locked`; `ImportScript` with `require_redeemable` |
| `Unimplemented` (12) | kind `WatchingOnly`; `ServiceReady` for an unknown service name; the library for unknown services and methods |
| `Internal` (13) | `BlockInfo` header decode; `BirthBlock` state read; `CommittedTickets` sanity check; the library for malformed requests |
| `Unavailable` (14) | kind `NoPeers` |
| `DataLoss` (15) | kind `Crypto` |

`Aborted`, `OutOfRange` and `Unauthenticated` are never produced by the
handlers' own code. Rule 4 of 5.3.1 passes through whatever code a status
error returned by the library's `Send` carries, so the set of codes on
streaming RPCs whose `Send` fails is the library's, not this table's.
Authentication failure is a TLS handshake failure, not a gRPC
status.

### 8.2 Handlers that return untranslated (RAW) errors

| Handler | Call whose error is RAW |
|---|---|
| `GetAccountExtendedPubKey` | `AccountXpub` |
| `GetTicket` | `VSPHostForTicket` (non-`NotExist`) |
| `ImportExtendedPublicKey` | `hdkeychain.NewKeyFromString` |
| `ImportVotingAccountFromSeed` | `DiscoverActiveAddresses` |
| `PurchaseTickets` | `NetworkBackend`, `PurchaseTickets`, ticket serialization |
| `ValidateAddress` | `KnownAddress` (non-`NotExist`), `AccountNumber`, pubkey address construction |
| `SetAccountPassphrase` | `AccountName` (non-`NotExist`), `AccountHasPassphrase` |
| `UnlockAccount` | `AccountName` (non-`NotExist`) |
| `GetPeerInfo` | `NetworkBackend`, dcrd `getpeerinfo` |
| `GetVSPTicketsByFeeStatus` | `GetVSPTicketsByFeeStatus` |
| `SyncVSPFailedTickets` | `GetVSPTicketsByFeeStatus` |
| `DiscoverUsage` | `DiscoverActiveAddresses` |

### 8.3 Log lines

| Level | Subsystem | Text |
|---|---|---|
| debug | `GRPC` | `Unary method <path> invoked by <addr>` |
| error | `GRPC` | `Unary method <path> invoked by <addr> errored: <err>` |
| debug | `GRPC` | `Streaming method <path> invoked by <addr>` |
| error, or debug for a client-cancelled stream | `GRPC` | `Streaming method <path> invoked by <addr> errored: <err>` |
| info | `GRPC` | every gRPC library log line, with a leading `grpc: ` removed |
| critical, then exit status 1 | `GRPC` | gRPC library fatal log lines |
| info | `DCRW` | `gRPC server listening on <addr>`; `gRPC server shutdown`; `Server TLS is disabled.  Only JSON-RPC may be used` |
| warn | `DCRW` | `gRPC server is configured with listeners, but no trusted client certificates exist (looked in <path>)`; `No certificates added from CA file <path>`; `Stopping gRPC server...` |
| trace | `DCRW` | `Finished serving gRPC: <err>` |

Start-up failures (`failed to create listeners for RPC server`, `no suitable
RPC services can be started`) are logged by `run` as `Unable to create RPC
servers: <err>` and end the process with a non-zero exit (spec 16).

A panic inside a handler is not recovered by the interceptors or the
library and terminates the process. `StartWalletService` panics with
`service already started` if called twice.

## 9. Compatibility-critical behaviours

1. Method paths `/walletrpc.VersionService/Version` and
   `/walletrpc.WalletService/<Name>` for the 65 names in section 3.3,
   including the misspelt `GetCoinjoinOutputspByAcct`.
2. Every message's field numbers and wire types exactly as in section 4,
   including `sint32` for the starting heights (and both heights of
   `GetCFiltersRequest`), field number 3 for
   `GetVSPTicketsByFeeStatusRequest.fee_status`, the reserved field 3 of
   `SignTransactionRequest`, and the message name `coinjoinTxsSumByAcct`.
3. Enum numbers as in section 4.7, in particular `COINBASE = 4` and the
   wallet-to-proto mappings (ticket status `Unspent` reported as `LIVE`).
4. `Version` returns `10.0.0` / 10 / 0 / 0 with empty `prerelease` and
   `build_metadata`.
5. Mutual TLS: a gRPC connection without a client certificate chaining to a
   certificate in the client CA file (or to the CA issued through
   `--issueclientcert`) fails in the TLS handshake. Certificates of every
   key type `--tlscurve` can produce (Ed25519 default, P-256, P-384, P-521)
   must be accepted on both sides. The gRPC server does not start without a
   client CA source, and never starts with `--noservertls`.
6. The readiness gate: `FailedPrecondition`, `service walletrpc.WalletService
   is not ready` before a wallet is loaded; `VersionService` always
   answers. This is the only way a client can tell "no wallet loaded" from
   other failures.
7. The `grpclistener` pipe message per listener and the log line `gRPC
   server listening on <addr>` (the only ways to learn the bound address
   when the configured port is 0).
8. Hash fields carry 32 raw bytes in internal byte order; a wrong length is
   `InvalidArgument` with the per-handler messages of section 5.6.
9. The status code for each failure as specified in section 5.6, including
   the cases where the code is `Unknown` because the error is returned RAW
   or the kind has no mapping. The codes a client is most likely to branch
   on: `ResourceExhausted` for insufficient funds in `ConstructTransaction`,
   `InvalidArgument` for a wrong passphrase, `FailedPrecondition` for a
   locked wallet or missing backend, `NotFound` for an unknown transaction
   or block, `AlreadyExists` for kind `Exist`.
10. Status messages: the fixed strings of section 5.6 and, for T() errors,
    the wallet error rendering with separator `":: "`. The message is the
    only detail a client receives (no status details are attached).
11. Presence rules of section 4.3 item 4: `spender` always present in
    `TicketDetails`; `block` absent for unmined tickets; `block_hash` absent
    in `GetTransactionResponse` and `TransactionConfirmations` when
    unmined.
12. `GetTransactions`: one message per block with at least one wallet
    transaction, ascending by height for default ranges, then one message of
    unmined transactions; `target_transaction_count` applied after whole
    blocks.
13. `GetTickets`: exactly one ticket per message; the block's tickets are
    all sent before the target is checked.
14. `TransactionNotifications`: no initial message; one message per wallet
    notification, lossless and ordered; `detached_blocks[i]` is the hash of
    `detached_block_headers[i]`; every attached block is listed, including
    blocks without wallet transactions.
15. `ConfirmationNotifications`: one response per non-empty request with one
    entry per requested hash; one response per main chain tip change (also
    when empty); `confirmations = −1` and `block_height = −1` conventions;
    `stop_after` semantics; the client must not half-close.
16. Temporary unlock semantics (5.4.1): a wallet that was locked is locked
    again when the RPC returns; a wallet unlocked by `UnlockWallet` stays
    unlocked; a wrong passphrase locks the wallet.
17. `UnlockWallet` unlocks without a timeout; `LockWallet` always succeeds.
18. `Accounts` key counts are last-returned index + 1;
    `AccountNotifications` key counts are last-used index + gap limit.
19. `ConstructTransaction`: default fee rate 10,000 atoms/kB when
    `fee_per_kb` is 0; change output swapped into a random position;
    `change_index = −1` without change. `SweepAccount.fee_per_kb` is in DCR
    per kB and the default is the wallet's relay fee.
20. `PurchaseTickets`: voting account selection rule; `tickets` and
    `split_tx` are always populated; `tx_fee` and `ticket_fee` have no
    effect beyond the sign check on `tx_fee`.
21. `ImportScript` succeeds (returning the P2SH address) when the script is
    already imported, without starting a rescan.
22. `ValidateAddress` returns OK with `is_valid = false` for undecodable or
    wrong-network addresses.
23. `SignMessages` returns OK with per-message `error` strings.
24. `GetPeerInfo` in dcrd mode relays dcrd's `getpeerinfo` fields; in SPV
    mode sorts by peer id and formats services with `%08d`.
25. Import RPCs start their rescans in the background and return
    immediately.
26. `Stop()` on shutdown: open streams end with a transport error, not a
    clean status.
27. Forward and backward compatibility of the message decoder: unknown
    field numbers are skipped, repeated numeric fields are accepted packed
    or unpacked, a repeated scalar takes its last value (section 4.3 item
    10). Clients built from older or newer revisions of `api.proto`, and
    by other protobuf libraries, rely on this.
28. `VersionService.Version` keeps its request and response shape forever
    (`api.md:79-81`); clients call it first to decide whether they can talk
    to the wallet, and they do so before any wallet is loaded.
29. `AccountNotifications` fires only for rename, new account and key
    import (5.6.24); `TransactionNotifications` leaves
    `unmined_transactions` empty in main-chain messages, so a client must
    use `unmined_transaction_hashes` to learn what a reorganization
    returned to the unmined set.
30. `GetCFiltersResponse.filter` is the N-varint-prefixed filter
    serialization and is absent for an empty filter (section 4.3).
31. Byte-identical status messages for the library-generated failures a
    client can provoke before a handler runs: `unknown service <s>`,
    `unknown method <m> for service <s>`, `grpc: error unmarshalling
    request: <err>`, the oversized-message `ResourceExhausted`, and
    `grpc: Decompressor is not installed for grpc-encoding "<name>"`
    (sections 4.1, 4.3). The `<err>` text of the unmarshalling failure is
    protobuf-go's and cannot be matched exactly by another codec; match the
    code and the prefix.
32. `ImportVotingAccountFromSeed` with a name that already exists returns
    `AlreadyExists`: the in-repository example client branches on exactly
    that code to treat the account as already imported
    (`rpc/grpc_example/main.go:130-134`). The code comes from the wallet's
    error kind through T(), so the port's address manager must report kind
    `Exist` for a duplicate account name here.

## 10. Quirks

Each item is observable behaviour a faithful port reproduces unless the
project decides otherwise (section 13).

**Q1. Services bind to the first wallet forever.** `ready` is never
cleared and the loader discards its callbacks after the first load
(`internal/loader/loader.go:81-89`). After `WalletLoaderService.CloseWallet`,
`WalletService` still passes the readiness gate and its handlers run
against the closed wallet (database errors). Opening a wallet again does not
re-attach the services. If the callback could run twice the process would
panic (`service already started`, `server.go:336-338`).

**Q2. Version numbers disagree.** Code reports 10.0.0
(`server.go:64-67`); `rpc/documentation/api.md:3` says `Version: 7.13.x`.
`prerelease` and `build_metadata` are documented but never set.

**Q3. Empty `case` arms in `errorCode`.** The switch at `server.go:88-118`
lists `Bug`, `IO`, `Encoding`, `ScriptFailure`, `Policy`, `DoubleSpend` and
`Protocol` with no body. The layout suggests they were meant to share the
code of the next arm (`Bug` → `InvalidArgument`, `IO` → `AlreadyExists`,
`Encoding` → `DataLoss`, the last four → `Unavailable`), but Go does not fall through, so all seven map
to `Unknown` (unless a later rule matches). A high-fee rejection in
`PublishTransaction` is therefore `Unknown`.

**Q4. Untranslated errors.** The handlers in section 8.2 return raw errors.
Visible consequences: `GetAccountExtendedPubKey` for an unknown account is
`Unknown` (documented `NotFound`, `api.md:688`); every `PurchaseTickets`
wallet failure is `Unknown` (documented `FailedPrecondition` for
insufficient balance, `api.md:1817`); `ValidateAddress` database errors are
`Unknown`.

**Q5. Nested status strings.** Passing a status error through T() or `%v`
embeds `rpc error: code = <Code> desc = ...` in the message:
`SweepAccount` invalid destination (`server.go:991-994`),
`ConstructTransaction` invalid change destination (`:1234-1237`),
`ImportVotingAccountFromSeed` and `DiscoverUsage` backend failures (`:826`,
`:4289`), and `SignMessages` per-message errors (`:2058`).

**Q6. `Aborted` is documented and never produced.** `api.md` lists
`Aborted: The wallet database is closed` for about twenty methods. No code
path returns `Aborted`.

**Q7. `Spender` special cases are dead.** `errors.Is(errors.NotExist, err)`
(`server.go:2156`, `:2159`) has its arguments reversed; see 5.6.18. The
codes happen to match the intent, the messages do not.

**Q8. `ConfirmationNotifications` oddities.** Client half-close ends the
RPC with `Unknown`/`EOF` (`server.go:2449-2452`, `:2504-2507`). An empty
response is sent on every tip change even with nothing watched
(`wallet/notifications.go:912-988`). Tip-change entries are in map order.
A watched transaction whose block is stake-invalidated by the next block
reports −1 and is unwatched, the same value as "unknown to the wallet".

**Q9. Temporary unlock can lock a wallet unlocked elsewhere.** If the
wallet was unlocked with a timeout (JSON-RPC `walletpassphrase`), any gRPC
call carrying a passphrase replaces that timeout and the wallet is locked
when the call returns (`wallet/wallet.go:1804-1832`). A wrong passphrase in
any call locks an unlocked wallet (`:1722-1727`).

**Q10. Inverted rescan-height checks.** `ImportPrivateKey` (`server.go:684`),
`ImportScript` (`:760`) and `ImportVotingAccountFromSeed` (`:804`, `:814`)
reject `scan_from > 0` (or `discover_from > 0`) exactly when the
corresponding flag is set, with a message saying the flag is not set. A
rescan or discovery from a non-zero height is therefore impossible through
these RPCs, a non-zero height without the flag is silently accepted, and
the height lookup at `:842-849` is unreachable.

**Q11. Import RPCs need a network backend even without a rescan**
(`server.go:689`, `:714`, `:765`). `ImportPrivateKey` unlocks the wallet
before validating the account and scan arguments, so a request that fails
validation still has the unlock side effects. `api.md:1139` documents
`NotFound` for a missing account; the code returns `InvalidArgument` for
any account other than the imported account.

**Q12. `ImportScript` ignores `passphrase`** (documented as deprecated,
`api.md:1151-1152`) and treats an already-imported script as success.

**Q13. `PurchaseTickets` documentation is stale.** `api.md:1746` documents
a `spend_limit` field that does not exist; `mixing_enabled` is
`enable_mixing`; `unsigned_tickets` is `tickets`; `split_tx` is `bytes`,
not repeated. `tx_fee` is range-checked and then unused
(`server.go:1829-1834`); `ticket_fee` is never read. `num_tickets` is
unsigned, so only 0 triggers `Zero or negative number of tickets given`.
The VSP client cache keyed by URL means a second request with a different
pubkey or accounts for the same host silently reuses the first
configuration. The request passphrase is not zeroed.

**Q14. `FundTransaction` and `UnspentOutputs` ignore
`include_immature_coinbases`** and do not reject negative `target_amount`
or `required_confirmations` (documented `InvalidArgument`,
`api.md:1308-1310`, `:1372-1374`). The `InsufficientBalance` tolerance is
never exercised because the wallet's input source returns what it found
without that error (`wallet/udb/txmined.go:3100-3345`). Both handlers
dereference the returned input detail unconditionally after tolerating
that kind (`server.go:1067`, `:1109`), so the tolerance is only safe as
long as the wallet never pairs an `InsufficientBalance` error with a nil
result.

**Q15. Two fee units.** `ConstructTransactionRequest.fee_per_kb` is `int32`
atoms per kB (negative values are passed to the wallet unchecked);
`SweepAccountRequest.fee_per_kb` is `double` DCR per kB. Their defaults
differ (constant 10,000 atoms/kB versus the wallet's configured relay fee).

**Q16. `CreateSignature.hash_type` is truncated to 8 bits** with no
validation (`server.go:1747`).

**Q17. Block range semantics differ from both descriptions.** The proto
comment (`api.proto:367-369`) says a negative starting height means "the
last N blocks"; `api.md:769-773` says it is subtracted from the tip. The
code passes heights through unchanged (`server.go:1365-1366`, `:1381-1382`)
and the store (`wallet/udb/txquery.go:388-421`, `:521-538`, `:727-761`)
behaves as follows, with start defaulting to 0 and end to −1:

| Resolved start, end | `GetTransactions` / `GetTickets` | `GetCFilters` |
|---|---|---|
| start ≥ 0, end < 0 | blocks ascending from start to the tip, then unmined | blocks ascending from start to the tip |
| 0 ≤ start < end | blocks ascending, start to end inclusive; no unmined | same |
| start ≥ end ≥ 0 | blocks descending from start (or the highest block at or below it) down to end inclusive; no unmined | same |
| start < 0, end ≥ 0 | unmined first, then blocks descending from the tip down to end | blocks descending from the tip down to end |
| start < 0, end < 0 | unmined only | nothing |

An explicit height 0 cannot be told apart from "unset". `GetCFiltersRequest`
declares `ending_block_height` as `sint32` (`api.proto:1183`) while
`api.md:2495` documents `int32`; the other two requests use `int32`.

**Q18. `minimum_recent_transactions`** is validated and then ignored
(`server.go:1388-1412`, the source's own `BUGS` comment). The same comment
notes wrong codes for unknown blocks: an unknown hash or height surfaces as
the wallet's error through T().

**Q19. Ticket results.** `TicketStatusUnspent` (SPV mode) is reported as
`LIVE`. Precision depends on the backend type (`*chain.Syncer` uses dcrd to
distinguish missed/expired). `api.md:881` names the response field
`tickets`; the proto field is `ticket`. `vsp_host` is undocumented.
`GetTicket` for a hash that is not a ticket fails with `Unknown`.

**Q20. Unspecified orderings exposed to clients.**
`FundTransaction`/`UnspentOutputs` output order comes from a randomised
selection (`wallet/udb/txmined.go:3104-3172`); `ConstructTransaction`
randomises the change position; `GetCoinjoinOutputspByAcct`,
`GetTrackedVSPTickets` (both the VSP list and each ticket list),
`GetVSPTicketsByFeeStatus` and `ConfirmationNotifications` tip updates are
in Go map iteration order. None of these can be compared byte for byte
against the Go implementation; compare as sets.

**Q21. Two definitions of "key count".** `Accounts` reports last-returned
index + 1 (`server.go:403-404`); `AccountNotifications` reports
`min(2^31, last-used index + gap limit)` with uint32 wraparound for unused
branches (`wallet/notifications.go:618-623`). `api.md` describes both as
"number of derived keys". `Accounts` also returns `account_encrypted` and
`account_unlocked`, which `api.md:547-560` omits.

**Q22. A status message embeds a protobuf text rendering.** `Invalid
transaction hash length for script <script>, expected 32 got <n>`
(`server.go:1613-1615`, `:1668-1670`) formats the `AdditionalScript`
message with `%v`. protobuf-go's text output is deliberately unstable
across builds (it varies whitespace), so the exact bytes cannot be pinned.

**Q23. `StakeInfo` and `BestBlock`/`TicketPrice` have no backend
requirement**; `TicketPrice` reads the difficulty and the tip height in two
separate database transactions, so `height` may be one block ahead of the
block the price was computed for.

**Q24. `ValidateAddress.pub_key_addr`** is the input address string itself
for public-key addresses that the wallet does not own, and the derived
pubkey-address for owned P2PKH addresses. `api.md:1927` describes
`pay_to_addr_script` as "the redeem script"; it is the address's payment
script.

**Q25. `Rescan` details.** `begin_hash` is undocumented (`api.md:1007-1009`).
A hash is resolved to a height and the rescan runs over the main chain from
that height, even if the hash names a side chain block. Cancellation
produces either of two `Canceled` messages (7.5). If the handler exits on a
`Send` error while a progress value is still buffered, the rescan goroutine
blocks forever on its final send to the capacity-1 channel
(`wallet/rescan.go:435-442`); not externally visible.

**Q26. `SetAccountPassphrase` relocks the account.** When it unlocks the
account with `account_passphrase`, the deferred `LockAccount` runs
regardless of the account's state before the call, and its error is
assigned to a variable that is not returned (`server.go:3889-3892`).

**Q27. VSP RPC error handling.** `SyncVSPFailedTickets` swallows every
per-ticket error and always returns OK once the client is created.
`SetVspdVoteChoices` returns `Unknown` whenever any unspent ticket lacks
VSP records (5.6.63). `ProcessUnmanagedTickets` has no failure signal for
the processing itself.

**Q28. Naming irregularities fixed by compatibility.** Method
`GetCoinjoinOutputspByAcct`; message `coinjoinTxsSumByAcct` (lower-case
first letter); field `ticketAddresses` (camelCase); wallet method
`GetCoinjoinTxsSumbByAcct`; enum values of
`ValidateAddressResponse.ScriptType` in CamelCase; `AccountBalance` message
unused; `SIGHASH_OLD = 0`.

**Q29. gRPC library logging.** All library output is logged at info
(5.2.3); library fatal conditions exit the process with status 1 without
running the wallet's shutdown path.

**Q30. Interceptor logging.** Every unary error, including client mistakes
such as `InvalidArgument`, is logged at error level with the peer address.
Readiness failures are not logged at all. The debug line is logged before
the readiness check.

**Q31. Silent absence of gRPC.** Without `clients.pem` and
`--issueclientcert` the gRPC server is not started and only a warning is
logged; the process continues with JSON-RPC alone.

**Q32. Further documentation disagreements.**

| RPC | `api.md` | Code |
|---|---|---|
| `Balance` | `InvalidArgument` when `required_confirmations` is negative (`:618`) | no check in the handler |
| `NextAccount` | no mention of the empty-name rule | `InvalidArgument`, `account name may not be empty` |
| `NextAddress` | no `InvalidArgument` listed | unknown `kind` or `gap_policy` is `InvalidArgument` |
| `ImportVotingAccountFromSeed` | response named `ImportPrivateKeyResponse` (`:1215`); "Passphrase not supplied", "Duplicate name" as `InvalidArgument` | response is `ImportVotingAccountFromSeedResponse`; those conditions come from the wallet through T() (a duplicate name is kind `Exist` → `AlreadyExists` if the address manager reports it so; see spec 02) |
| `SignTransactions` | request field `unsigned_transaction` (`:1518`); `SignedTransaction` has only `transaction` | field is `transactions`; `SignedTransaction` also has `unsigned_input_indexes` |
| `SignMessages` | response named `SignMessageResponse` (`:1881`) | `SignMessagesResponse` |
| `SignHashes` | response field `signature` (`:1681`) | `signatures` |
| `PublishUnminedTransactions` | "Expected errors: None" | `FailedPrecondition` without a backend, plus wallet errors |
| `TicketPrice`, `StakeInfo` | "Expected errors: None" | wallet errors through T() |
| `LockAccount`, `UnlockAccount` | `FailedPrecondition` when the account has no own passphrase (`:2571`, `:2591`) | whatever kind the address manager returns through T(); `AccountUnlocked` uses kind `Invalid` → `InvalidArgument` |
| `DumpPrivateKey` | `InvalidArgument` for a watching-only wallet (`:2688`) | kind `WatchingOnly` maps to `Unimplemented` |
| `Address` | field `pulic_key` (`:2655`); "the address's pkh" | field `public_key`, a pubkey-address string |
| `CommittedTickets` | nested message `TicketAddresses` | `TicketAddress` |
| `BestBlock` | "block height and height" | hash and height |
| `GetTickets` | `REVOKED` etc. only | also `vsp_host` |
| method list (`:417-466`) | omits `CoinType`, `GetAccountExtendedPubKey`, `GetAccountExtendedPrivKey` | implemented and documented further down |
| `CoinType` | expected error `WatchingOnly` (`:508`), which is not a gRPC code | kind `WatchingOnly` → `Unimplemented` |
| `Accounts` | accounts "ordered by increasing account numbers" (`:543`) | the handler copies the wallet's order (open question 6) |
| `GetTransactions` | no `minimum_recent_transactions` field; only the hash-length `InvalidArgument` listed (`:762-816`) | field 5 exists and is validated (5.6.11 step 2); negative `target_transaction_count` and the hash-plus-height combinations are also `InvalidArgument` |
| `GetTransactionsRequest.minimum_recent_transactions` (proto comment, `api.proto:382-385`) | "Cannot be used when the ending block hash is specified" | rejected when an ending hash or an ending height is given; otherwise ignored |
| `ImportPrivateKey` | no `scan_from` field (`:1113-1122`) | field 5 exists and is validated |
| `ImportVotingAccountFromSeed` | `passphrase` is "the wallet's private passphrase" (`:1200`), while the method summary says the account is "locked with the supplied password" (`:1191`); negative gap limit is `InvalidArgument` (`:1230`) | the passphrase encrypts the new account and does not unlock the wallet (5.6.33 step 8); a negative `discover_gap_limit` wraps to a large uint32 |
| `SignTransactionRequest` (proto comment, `api.proto:560-565`) | describes the removed `input_indexes` field and says the RPC "immediately errors" when an input cannot be signed | every input is attempted; failures are reported in `unsigned_input_indexes` with status OK |
| `SetAccountPassphraseRequest.wallet_passphrase` (proto comment, `api.proto:1216`) | "used only in case account pass not yet set" | used whenever the account has no own passphrase or `account_passphrase` is empty (5.6.53 step 3) |
| `PublishTransaction` | `InvalidArgument` when the transaction "is missing input scripts" (`:1624-1625`) | the handler only deserializes; anything else is a wallet or network error through T() |
| `PurchaseTickets` | `tx_fee` 0 means "the global value for a transaction fee will be used" (`:1759-1761`); stale pool-address and pool-fee errors (`:1806-1813`); `tickets`/`split_tx` returned "when dont_sign_tx is true" (`:1792-1794`) | `tx_fee` has no effect at all; there are no pool fields; both response fields are always populated |
| `ValidateAddress` | "For watch-only wallets [`account_number`] will always be 0" (`:1914-1915`); no expected errors listed | no watching-only special case in the handler; database errors are RAW `Unknown` |
| `TransactionNotifications` | on a reorganization, transactions that are not mined in the new chain "are included" in `unmined_transactions` (`:2040-2042`; same claim in `api.proto:691-695`); `detached_blocks` are blocks "with wallet transactions" (`api.proto:686-688`) | `unmined_transactions` is deliberately left empty in main-chain notifications (`wallet/notifications.go:318-323`); every detached block is listed whether or not it held wallet transactions |
| `AccountNotifications` | notifies "account property changes, such as name and key counts" (`:2065-2066`) | emitted by four wallet operations only (5.6.24) |
| twelve RPCs | no section at all: `Spender`, `GetPeerInfo`, `ImportExtendedPublicKey`, `GetCoinjoinOutputspByAcct`, `SetAccountPassphrase`, `AccountUnlocked`, `SyncVSPFailedTickets`, `GetVSPTicketsByFeeStatus`, `ProcessManagedTickets`, `ProcessUnmanagedTickets`, `SetVspdVoteChoices`, `DiscoverUsage` | implemented |

`serverchanges.md:76` shows the obsolete `--go_out=plugins=grpc` command and
`:88` links a `../rpcserver/` directory that no longer exists; the actual
generator invocation is `rpc/regen.sh` (`--go_out`, `--go-grpc_out`,
`paths=source_relative`), and the implementation lives in
`internal/rpc/rpcserver`.

**Q33. Dead code.** `NextAddress` re-checks `err` after both branches
already returned on error (`server.go:559-561`). `minRecentTxs` is assigned
and discarded (`:1412`). `marshalBlock`'s nil-header branch (`:2326-2332`)
is unreachable from current callers. `CommittedTickets` returns a non-nil
empty response alongside an error (`:1964`), which the library discards.

**Q34. Client examples predate mandatory client certificates.** The Go
example in `rpc/documentation/clientusage.md:66-93` presents a client key
pair, but the C++, C#, Node.js and Python examples in the same file
(`:115-460`) trust `rpc.cert` and present no client certificate; against the
server as wired in `rpcserver.go:321-322` their TLS handshake fails. The
same examples set `GRPC_SSL_CIPHER_SUITES=HIGH+ECDSA` and state that ECDSA
certificates are required, while the default `--tlscurve` is Ed25519
(`internal/cfgutil/curve.go:29`). Clients configured this way exist in the
field; whether they complete a handshake with an Ed25519 server certificate
is open question 4.

**Q35. The documents' own compatibility rules.** `api.md:16-19` declares
itself "the authoritative source" and any divergence "an implementation
error", yet its version line and much of its content are stale (Q2, Q32);
for a drop-in replacement the Go code is what clients observe.
`serverchanges.md:31-45` states the evolution rules the API has followed
and a port must keep following: the API version obeys Semantic Versioning
2.0; unless the major version is bumped, message fields are never removed
or changed and new fields are only appended; `required` is never used, and
a field that must be set is checked by the handler, which "must return an
appropriate error". `api.md:79-81` marks `VersionService` "Stable: This
service will never introduce a breaking change even in future major
versions" and recommends it as the runtime compatibility check;
`api.md:1688-1689` marks `SignHashes` "Unstable". No other RPC in scope
carries a stability statement. The service list at `api.md:34-43` omits
`AccountMixerService`.

**Q36. Notes left in the source.** `Balance` carries `TODO: Spendable
currently includes multisig outputs that may not actually be spendable
without additional keys` (`server.go:879-880`): `spendable` is the wallet's
figure, unfiltered. `ImportScript` carries the script-version TODO and the
zero-signature NOTE (5.6.34). `translateError`'s comment says the table "is
by no means complete" and that every handler not returning a
`status.Errorf` error "should return this result instead"
(`server.go:74-79`); the RAW handlers of section 8.2 violate that rule.
`GetTransactionsRequest.minimum_recent_transactions` carries `TODO: remove
until spec adds it back in some way` (`api.proto:385`). The comment block
before `scProto` (`server.go:3646-3662`) quotes a `ScriptClass` enum with
names (`NON_STANDARD`, `PUB_KEY`, ... `TGEN`, `TADD`) that belong to
`DecodedTransaction` (sibling spec); `ValidateAddressResponse.ScriptType`
uses the same numbers with the CamelCase names of section 4.7.

**Q37. Lost transaction notifications are silent.** A database error while
building a transaction notification drops it for all clients with only a
wallet log line (5.6.23); the next notification carries the then-current
`unmined_transaction_hashes`, but attached and detached blocks of the lost
one are never resent.

**Q38. Keepalive pings can terminate a client.** With the library's
default enforcement policy (section 4.1) a client that sends HTTP/2 PINGs
more often than every 5 minutes while only long-lived notification streams
are open and idle, or at all within 2 hours on a connection with no open
stream, is disconnected with GOAWAY `too_many_pings` after its third
strike. Clients written against the Go server are necessarily tuned to
this; a port that never enforces it is more permissive, which existing
clients cannot observe.

## 11. Upstream tests

There are none for this scope. `internal/rpc/rpcserver` has no `_test.go`
file and no test in the repository imports `rpc/walletrpc` or the
`rpcserver` package.

Adjacent tests:

| File | Pins |
|---|---|
| `x509_test.go` (root) | `generateAuthority`, `marshalPrivateKey`, `createSignedClientCert` for Ed25519, P-256, P-384, P-521: a client certificate signed by the generated CA authenticates against an HTTPS test server requiring client certificates (`TestClientCert`); `TestUntrustedClientCert` covers a client certificate the server does not trust. Covers the `--issueclientcert` path shared with gRPC (spec 16). |
| `errors/errors_test.go` | Three tests: `TestCollapse` (`errors.E` collapsing), `TestIs` (`Is` matching on wrapped error, op and kind), `TestDoubleWrappedErrorWithKind` (`Is` against a bare `Kind` through two wrappings). `As` and the string rendering of `Error()` (including the separator), on which T() depends, are not tested upstream. |

Not tests, but the only in-repository clients of this API:
`rpc/grpc_example/main.go` and the snippets in
`rpc/documentation/clientusage.md`. The example is a manual program against
a testnet wallet on `localhost:19111` (mutual TLS with `client.pem` and
`client-key.pem`); it is not run by `run_tests.sh`. It encodes a few
expectations that double as an informal acceptance script for the port:

| Call (`rpc/grpc_example/main.go`) | Expectation |
|---|---|
| `Balance` account 0, 1 confirmation (`:64-71`) | succeeds |
| `ImportScript` with only `script` set (`:100-114`) | succeeds, including on a re-run when the script is already imported |
| `ImportVotingAccountFromSeed` with a 32-byte seed, a name and an account passphrase (`:125-148`) | succeeds, or fails with exactly `AlreadyExists` on a re-run, after which the account is located by name through `Accounts` |
| `NextAddress` with `GAP_POLICY_IGNORE` on the imported voting account, either branch (`:155-177`) | fails (any error) |
| `ValidateAddress` for every address obtained above (`:184-194`) | succeeds |
| `Address` for account 0 at branch 0 index 0, branch 1 index 0, and branch 2 (`:199-217`, `:295-308`) | the first two succeed; branch 2 fails (any error; the handler returns `InvalidArgument`, `kind=2`) |
| `UnlockAccount` with the account passphrase, then `DumpPrivateKey` for an address of that account (`:220-228`) | both succeed |

The generated package `rpc/walletrpc` has no test file. `rpcserver.go`'s
`serviceName`, `interceptUnary`, `interceptStreaming` and `makeListeners`
have no test (spec 16 notes the same for `makeListeners`).

Untested upstream, to be covered by the port:

1. Message encodings. Build golden vectors by encoding each response
   message with protobuf-go through a Go oracle shim (a small Go program
   linked against `rpc/walletrpc` at the pin that reads a JSON description
   and writes the bytes). At minimum: `VersionResponse`, `BlockDetails`
   (field order), `GetTicketsResponse` with and without spender and block,
   `SignTransactionResponse` (packed indexes), messages with negative
   `int32` and `sint32` values.
2. `errorCode`: one vector per `errors.Kind`, plus nested kinds, wrapped
   `context.Canceled`, `hdkeychain.ErrInvalidSeedLen`, wrapped status errors
   and the string rendering with `":: "`. Regenerate through the oracle shim
   by calling `translateError` (requires a shim inside the dcrwallet module,
   since the function is unexported).
3. Validation order and messages of every handler in 5.6: drive a Go
   dcrwallet on simnet (with dcrd or in SPV mode) and the Rust port with the
   same request set against copies of the same `wallet.db`; compare status
   code, message and, where ordering is defined, response bytes.
4. `decodeBlockRange` and the range table of Q17.
5. Notification streams: scripted block connection, reorganisation and
   unmined-transaction scenarios on simnet, comparing message sequences.
6. `ConfirmationNotifications`: request permutations (duplicates, unknown
   hashes, `stop_after` 0 and 1), tip-change responses, half-close.
7. TLS: connection with no client certificate, with an unrelated
   certificate, and with each supported key type.
8. Decoder leniency (section 4.3 item 10): requests with unknown fields,
   unpacked repeated numerics, duplicated scalars, and each malformed shape,
   compared against the Go server's status.
9. `AccountNotifications` trigger set (5.6.24) and the dropped-notification
   behaviour of `TransactionNotifications` under an injected database
   error.
10. Library-level rejections: unknown service and method, oversized
    request, unsupported `grpc-encoding`, PING flood (Q38).

## 12. Rust porting notes

### 12.1 Suggested boundaries

| Crate / module | Mirrors | Contents |
|---|---|---|
| `purse-walletrpc` | `rpc/api.proto`, `rpc/walletrpc` | The vendored `api.proto`; message types with encode/decode for all eleven services; service and method path constants. No I/O. |
| `purse-grpc` | the parts of `google.golang.org/grpc` the wallet uses | Server-side gRPC over HTTP/2: framing, unary/server-streaming/bidi dispatch, status trailers with percent-encoded messages, 4 MiB receive limit, per-RPC cancellation context, interceptor hook. |
| `purse-rpcserver::grpc` (module in the daemon's RPC crate) | `internal/rpc/rpcserver` | Service singletons with ready gates, `ServiceReady`, error translation, helpers of 5.4, the `VersionService` and `WalletService` handlers (the sibling spec adds the rest). |
| daemon wiring | `rpcserver.go`, `dcrwallet.go` | TLS config, listeners, interceptors, loader callback, shutdown. |

### 12.2 Reuse

Only crates that were opened for this spec are named.

- `dcroxide-certgen` (`dcroxide:crates/dcroxide-certgen/src/certgen.rs:218`,
  `:305`): `new_tls_cert_pair` and `new_ed25519_tls_cert_pair` mirror dcrd's
  `certgen`, which `cfg.TLSCurve.CertGen` calls
  (`internal/cfgutil/curve.go:116-124`).
- `dcroxide-tlscrypto`: the rustls crypto provider (Ed25519, P-256/384/521
  signing and verification) used by dcroxide's RPC server; needed for the
  same certificate types here.
- `dcroxide-node` contains a Go-compatible client certificate verifier
  (`rpcrun::GoClientCertVerifier`, referenced from
  `dcroxide:crates/dcroxide-node/Cargo.toml:74-79`). It lives in the node
  binary crate, so it would have to be extracted to be shared.
- `dcroxide-rpc` holds the JSON-RPC request handling
  (`dcroxide:crates/dcroxide-rpc/src/http.rs:1-8`); the live HTTP/1.1 shell
  (listeners, header reading and writing) is hand-written in
  `dcroxide:crates/dcroxide-node/src/rpcrun.rs`. Neither has HTTP/2 support
  and nothing in them serves gRPC.
- `dcroxide-wire`, `dcroxide-chainhash`, `dcroxide-txscript`, `dcroxide-gcs`,
  `dcroxide-chaincfg` exist as crates and are the natural sources for
  transaction (de)serialization, hashes, address and script classification,
  filters and network parameters. Their APIs were not read for this spec.
- `dcr-rs` (`dcr-rs:src/`) has `address.rs`, `hd.rs`, `tx.rs`, `amount.rs`
  modules; not evaluated here.

### 12.3 Hazards

1. **HTTP/2.** dcroxide has no async runtime and no HTTP/2 implementation
   (`dcroxide:docs/adr/0005-concurrency-model.md`). gRPC requires HTTP/2
   with flow control, trailers and concurrent streams. Either adopt an
   existing stack (which brings an async runtime) or write a synchronous
   HTTP/2 server. This is the largest single cost in the subsystem and a
   project-level decision.
2. **Field order.** Emit fields in ascending field number, not declaration
   order (`BlockDetails`). Whatever protobuf library is used must be
   checked for this; a hand-written encoder makes it explicit.
3. **Presence of empty sub-messages** (4.3 item 4). Model `spender` as
   always-present and `block` as optional.
4. **Client certificate verification.** Go accepts a client certificate
   that is itself in the CA pool (self-signed leaf used as its own trust
   anchor) and certificates carrying CA basic constraints; common Rust
   verifiers reject both. Certificates in `clients.pem` are typically
   self-signed certificates generated by the client application.
5. **Backpressure.** The notification streams must block the wallet rather
   than buffer or drop (7.3). An unbounded queue would change memory
   behaviour; a bounded dropping queue would lose notifications.
6. **Error rendering.** T() messages are the wallet error string; the port's
   error type must render identically (`Op`, kind string, `":: "`
   separator, de-duplication of repeated ops and kinds).
7. **`errorCode` empty arms** (Q3) must be reproduced as `Unknown`, and RAW
   handlers (8.2) must bypass the table.
8. **Unlock interplay** (5.4.1, 7.2): the temporary unlock is a wallet-wide
   state change, not a per-call capability.
9. **Zeroing.** Go zeroes some passphrase buffers and not others; zero all
   of them in Rust (not observable).
10. **Map-ordered outputs** (Q20): choose a deterministic order in Rust and
    compare as sets in differential tests.
11. **Panics.** Go terminates the process on a handler panic; decide the
    Rust behaviour (abort versus isolating the RPC).
12. **Detached rescans** must not be tied to the RPC's cancellation.
13. **`grpc-message` encoding.** Wallet error strings may contain bytes
    outside printable ASCII (addresses and hashes do not, but error text
    from dependencies can); percent-encode per the gRPC HTTP/2 spec.
14. **Decoder leniency.** A strict hand-written protobuf decoder (rejecting
    unknown fields, unpacked repeated numerics or duplicate scalars) breaks
    clients that the Go server accepts (section 4.3 item 10).
15. **Keepalive.** grpc-go clients configured with keepalive expect PING
    acknowledgements; the server-side strike policy (Q38) may be relaxed
    but PING ACK and the server's own 2 h idle PING should exist so that
    dead connections holding notification streams are detected.
16. **Unimplemented fallback.** The Go structs embed generated
    `Unimplemented*Server` types; the port should have an equivalent
    default (`Unimplemented`, `method <Name> not implemented`) so that a
    method added to the proto before its handler exists behaves the same.

### 12.4 Size estimate

**L**, excluding the HTTP/2 transport. The Go in scope is about 2,700 lines
of handlers and plumbing plus 1,341 lines of proto; the handlers are thin,
but there are 65 of them, each with its own validation order and error
strings, three notification streams with blocking semantics, and a message
codec for about 155 message types (about 220 in the whole file). If the project hand-writes HTTP/2 and
gRPC framing instead of adopting a stack, the transport alone is XL and
should be planned as its own work item.

## 13. Open questions

1. UNVERIFIED: whether any transitive dependency registers a gRPC
   compressor (4.1). Settle by inspecting the import graph of the Go binary
   at the pin.
2. UNVERIFIED: exact error kinds and messages produced inside the wallet
   for each handler (unknown account, locked wallet, duplicate name and so
   on). This spec fixes the translation rule; the kinds are owned by specs
   02, 04, 06 to 08, 10 and 11 and must be cross-checked when those are
   implemented.
3. UNVERIFIED: the precise HTTP/2 frames grpc-go emits for error-only
   responses (trailers-only versus headers plus trailers) and whether any
   client depends on the difference. Settle with a packet capture of the Go
   server.
4. UNVERIFIED: which TLS 1.2/1.3 cipher suites and curves clients in the
   field require. Go's defaults apply on the server; test against the
   client implementations the project intends to support (the non-Go
   examples in `rpc/documentation/clientusage.md` use gRPC Core and Node).
5. UNVERIFIED: whether Go's verifier and the Rust verifier agree on every
   certificate shape found in real `clients.pem` files (hazard 4). Settle
   with a corpus of client certificates from each supported client and
   from `--issueclientcert`.
6. The address manager's iteration order behind `Accounts` (5.6.5) and the
   ordering of `CommittedTickets` results were taken as "the wallet's
   order" and not traced here.

7. UNVERIFIED: `api.md:2154-2156` says `BlockDetails.transactions` are
   "sorted by their block index in increasing order". The handler copies
   the wallet's slice order; the order the wallet produces for
   `GetTransactions` and for attached-block notifications was not traced
   here; it belongs to the transaction store and wallet core specs.
8. UNVERIFIED: whether the transaction store's input source
   (`wallet/udb/txmined.go:3100-3345`) can pair an `InsufficientBalance`
   error with a nil input detail (Q14). Settle against the transaction
   store spec; if it can, `FundTransaction` and `UnspentOutputs` panic in
   Go.

### Decisions needed

1. HTTP/2 and gRPC transport: adopt an async stack or hand-write a
   synchronous one (12.3 hazard 1).
2. Protobuf codec: generated code from a third-party library or
   hand-written encoders with tag-ordered output.
3. Q1: reproduce "services stay bound to the first wallet after
   `CloseWallet`" or reset the ready gates on close.
4. Q3 and Q4: reproduce the `Unknown` codes exactly (default for a drop-in
   replacement) or map the unmapped kinds and RAW errors properly.
5. Q7: keep the wallet-rendered `Spender` messages or emit the intended
   fixed strings.
6. Q8: keep `Unknown`/`EOF` on half-close and the empty per-tip responses.
7. Q10: keep the inverted rescan-height checks.
8. Q20: the deterministic orders to use where Go exposes map order.
9. Q22: the text to use for the unstable protobuf rendering.
10. Handler panic policy (12.3 hazard 11).
11. Whether the reported API version stays `10.0.0` for the first release.
