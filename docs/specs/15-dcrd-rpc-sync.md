# 15 — RPC mode: syncing through a dcrd node over JSON-RPC websocket

## Header

**Scope (files read in full, with line counts)**

| File | Lines | Content |
|---|---:|---|
| `chain/sync.go` | 966 | `Syncer`, `RPCOptions`, `Callbacks`, `Run`, header catch-up, notification handling |
| `chain/backend.go` | 106 | `wallet.NetworkBackend` and optional-interface methods of `Syncer` |
| `chain/semver.go` | 30 | dcrd JSON-RPC API version compatibility check |
| `chain/log.go` | 20 | package logger hook |
| `chain/semver_test.go` | 88 | tests |
| `rpc/client/dcrd/calls.go` | 543 | typed wrappers for every dcrd JSON-RPC method |
| `rpc/client/dcrd/json.go` | 330 | custom JSON encoders/decoders |
| `rpc/client/dcrd/notifications.go` | 115 | notification parameter decoders |
| `rpc/client/dcrd/codes.go` | 9 | dcrd error code constant |
| `rpc/client/dcrd/json_test.go` | 379 | tests |
| `dcrwallet.go` | 636 | of which RPC-mode wiring: backend selection `:417-443`, `rpcSyncLoop` `:557-600`, `readCAFile` `:602-619`, `readClientCertKey` `:621-636` |
| `jrick/wsrpc/v2@v2.3.8/rpc.go` | 528 | JSON-RPC 2.0 websocket client |

Read as supporting context (owned by other specs, cited where the RPC syncer depends on them): `config.go:120-141,691-737,784-842,986-1026`, `internal/netparams/params.go`, `internal/cfgutil/normalization.go`, `wallet/network.go`, `wallet/rescan.go`, `wallet/sidechains.go`, `wallet/chainntfns.go:84-440,680-933`, `wallet/wallet.go:239-270,792-830,891-1200,1291-1540,5172-5186,5545-5552`, `wallet/discovery.go:26-31,536-714,716-979`, `wallet/checkpoints.go`, `wallet/difficulty.go:833-900`, `validate/validate.go`, `deployments/deployments.go:75-160`, `errors/errors.go`, `internal/rpc/jsonrpc/errors.go:16-49`, the `*chain.Syncer` call sites in `internal/rpc/jsonrpc/methods.go` and `internal/rpc/rpcserver/server.go`, `gorilla/websocket@v1.5.1/{client.go,json.go,conn.go}`, `dcrd/rpc/jsonrpc/types/v4@v4.4.0/{chainsvrcmds.go,chainsvrresults.go,chainsvrwscmds.go,chainsvrwsntfns.go,chainsvrwsresults.go}`.

**Parity pin**

| Component | Version |
|---|---|
| dcrwallet | master `54b85b33` (2.2.0-pre, module `decred.org/dcrwallet/v5`) |
| `github.com/jrick/wsrpc/v2` | v2.3.8 |
| `github.com/gorilla/websocket` | v1.5.1 (selected by dcrwallet `go.mod:33`; wsrpc itself asks for v1.5.0) |
| `github.com/decred/dcrd/rpc/jsonrpc/types/v4` | v4.4.0 |
| `github.com/decred/dcrd/dcrjson/v4` | v4.2.0 |
| `github.com/decred/dcrd/wire` | v1.8.0 |
| `github.com/decred/dcrd/gcs/v4` | v4.1.1 |
| `github.com/decred/dcrd/chaincfg/chainhash` | v1.0.5 |
| `github.com/decred/dcrd/dcrutil/v4` | v4.0.3 |
| `github.com/decred/dcrd/mixing` | v0.7.5 |
| `github.com/decred/dcrd/blockchain/stake/v5` | v5.0.3 |
| `github.com/jrick/bitset` | v1.0.0 |
| `golang.org/x/sync` (errgroup) | v0.23.0 |
| `github.com/decred/dcrd/crypto/blake256` | v1.1.0 (the syncer's shared mix-message hasher) |
| `github.com/decred/dcrd/blockchain/standalone/v2` | v2.3.0 (`VerifyInclusionProof`, used by `validate.CFilterV2HeaderCommitment`) |
| dcrd server behaviour cross-checked against | dcroxide `crates/dcroxide-rpc` (port of dcrd master `46e9df48`, JSON-RPC API 8.3.0) |

**Summary**

In RPC mode the wallet keeps one websocket connection (`ws://host:port/ws` or `wss://host:port/ws`) to a trusted dcrd node and speaks JSON-RPC 2.0 request framing over it. `chain.Syncer.Run` dials, checks the network magic (`getcurrentnet`) and the dcrd JSON-RPC API semver (`version`, must be compatible with 8.3.0), registers itself as the wallet's `NetworkBackend`, waits for dcrd to finish its own sync, subscribes to winning-ticket notifications (voting wallets only), fetches missing cfilters, pulls headers with `getheaders` plus one `getcfilterv2` per header until dcrd returns nothing new, discovers addresses with `existsaddresses`, loads the server-side transaction filter with `loadtxfilter`, rescans with `rescan`, and then subscribes to block, tspend and (mixing wallets only) mix-message notifications. Notifications are processed strictly in arrival order by a single logical worker; a `blockconnected` whose parent is unknown triggers a header re-sync. Reorganizations are handled through the in-memory `SidechainForest`; `blockdisconnected` and `reorganization` notifications are ignored. When `Run` returns with an error the main package logs it, sleeps 5 s and starts a fresh `Syncer`. Unlike SPV mode, dcrd is trusted for transaction relevance, ticket liveness, address usage and agenda deployment status; header proof of work, stake difficulty and cfilter commitments are still verified locally during header catch-up.

---

## 2. Role in the system

**What it does.** Implements the "RPC" network backend: the wallet's only source of chain data when neither `--spv` nor `--offline` is set (`dcrwallet.go:434-441`). It owns the dcrd connection lifecycle, the initial sync state machine, live notification handling, and exposes dcrd to the rest of the wallet through `wallet.NetworkBackend` plus three optional capability interfaces (`deployments.Querier`, `wallet.LiveTicketQuerier`, `usedAddressesQuerier`).

**Who calls it.**

| Caller | Entry | Citation |
|---|---|---|
| main package, default mode | `rpcSyncLoop(ctx, w)` via `loader.RunAfterLoad`; builds `chain.RPCOptions` from config and loops `chain.NewSyncer(...).Run(ctx)` | `dcrwallet.go:422-442,562-600` |
| gRPC `WalletLoaderService.RpcSync` | builds its own `chain.RPCOptions` from the request, sets `chain.Callbacks` that stream `RpcSyncResponse` messages, calls `Run` once | `internal/rpc/rpcserver/server.go:2832-2981` |
| JSON-RPC server | type-asserts the wallet's backend to `*chain.Syncer` and calls `Syncer.RPC().Call(...)` for passthrough and for precise ticket/stake info | `internal/rpc/jsonrpc/methods.go:231-258,1234-1238,1352-1356,1811-1826,2501-2508,2552-2553,2603,2731-2735,2887-2933,4996-5026,5441-5446,5479-5489` |
| gRPC server | `StakeInfo`, `GetTicket`, `GetTickets`, `GetPeerInfo` use `Syncer.RPC()` when the backend is a `*chain.Syncer` | `internal/rpc/rpcserver/server.go:906-914,1462-1472,1538-1543,3999-4004` |
| wallet package | every use of `w.NetworkBackend()` (publish, load filter, rescan, cfilters, blocks, mix messages, `WrapContext`, `Synced`), and the optional interfaces `deployments.Querier`, `wallet.LiveTicketQuerier`, `usedAddressesQuerier` | `wallet/network.go:35-57`, `deployments/deployments.go:82-86`, `wallet/tickets.go:18-23`, `wallet/discovery.go:29-31` |

**What it calls.**

- `wsrpc.Dial` / `(*wsrpc.Client).Call` — the transport (gorilla/websocket underneath).
- `rpc/client/dcrd` — typed method wrappers and JSON codecs.
- Wallet methods: `ChainParams`, `Locked`, `MainChainTip`, `RescanPoint`, `BlockHeader`, `BlockLocators`, `BlockInMainChain`, `EvaluateBestChain`, `ValidateHeaderChainDifficulties`, `ChainSwitch`, `DiscoverActiveAddresses`, `GapLimit`, `LoadActiveDataFilters`, `RescanProgressFromHeight`, `FetchMissingCFiltersWithProgress`, `PublishUnminedTransactions`, `AddTSpend`, `AddTransaction`, `VoteOnOwnedTickets`, `VotingEnabled`, `VoteBits`, `MixingEnabled`, `ManualTickets`, `AcceptMixMessageBySource`, `SetNetworkBackend`, `Run`.
- `validate.CFilterV2HeaderCommitment`, `wallet.BadCheckpoint`, `wallet.NewBlockNode`, `wallet.SidechainForest`.
- `stake.IsSStx` (to drop unmined tickets in manual-ticket mode).

---

## 3. Public surface inventory

### 3.1 Package `chain`

| Item | Signature | Contract | Citation |
|---|---|---|---|
| `requiredAPIVersion` | `var semver{Major: 8, Minor: 3, Patch: 0}` | Minimum dcrd JSON-RPC API version; see §5.3. | `chain/sync.go:33` |
| `Syncer` | struct | Fields: `walletSynced atomic.Bool`; `targetSyncHeight atomic.Int32`; `wallet *wallet.Wallet`; `opts *RPCOptions`; `rpc *dcrd.RPC`; `notifier *notifier`; `blake256Hasher hash.Hash` + `blake256HasherMu sync.Mutex`; `discoverAccts atomic.Bool`; `sidechains wallet.SidechainForest` + `sidechainsMu sync.Mutex`; `cb *Callbacks`; `done chan struct{}`. Single-use: one `Run` per value. | `chain/sync.go:37-58` |
| `RPCOptions` | struct `{Address, DefaultPort, User, Pass string; Dial func(ctx, network, address string) (net.Conn, error); CA, ClientCert, ClientKey []byte; Insecure bool}` | Connection settings; see §5.1. | `chain/sync.go:62-72` |
| `NewSyncer` | `func NewSyncer(w *wallet.Wallet, r *RPCOptions) *Syncer` | Allocates the BLAKE-256 hasher and `done`; sets `discoverAccts = !w.Locked()` at construction time. | `chain/sync.go:75-84` |
| `Callbacks` | struct of 12 optional funcs | `Synced(synced bool)`, `FetchMissingCFiltersStarted()`, `FetchMissingCFiltersProgress(startCFiltersHeight, endCFiltersHeight int32)`, `FetchMissingCFiltersFinished()`, `FetchHeadersStarted()`, `FetchHeadersProgress(lastHeaderHeight int32, lastHeaderTime int64)`, `FetchHeadersFinished()`, `DiscoverAddressesStarted()`, `DiscoverAddressesFinished()`, `RescanStarted()`, `RescanProgress(rescannedThrough int32)`, `RescanFinished()`. Called synchronously on the syncing goroutine; a blocking callback blocks sync. Each is invoked only when `cb != nil` and the member is non-nil. | `chain/sync.go:89-102,136-215` |
| `(*Syncer).SetCallbacks` | `func (s *Syncer) SetCallbacks(cb *Callbacks)` | Plain assignment, unsynchronized; must precede `Run`. | `chain/sync.go:106-108` |
| `(*Syncer).RPC` | `func (s *Syncer) RPC() *dcrd.RPC` | Returns the client; `nil` until `Run` has dialed. | `chain/sync.go:111-113` |
| `(*Syncer).DisableDiscoverAccounts` | `func (s *Syncer) DisableDiscoverAccounts()` | `discoverAccts.Store(false)`. No caller in the tree at this pin. | `chain/sync.go:117-119` |
| `(*Syncer).Synced` | `func (s *Syncer) Synced(ctx context.Context) (bool, int32)` | Returns `walletSynced`; height is `targetSyncHeight` when unsynced, otherwise the wallet main-chain tip height (DB read). | `chain/sync.go:123-132` |
| `(*Syncer).Run` | `func (s *Syncer) Run(ctx context.Context) (err error)` | Full lifecycle, §5.2. Any non-nil return is wrapped with op `"rpcsyncer.Run"`. | `chain/sync.go:516-758` |
| `(*Syncer).synced` / `unsynced` | unexported | Compare-and-swap `walletSynced`; fire `Callbacks.Synced(true/false)` only on an actual transition. | `chain/sync.go:136-150` |
| 11 callback wrappers | `fetchMissingCfiltersStart/Progress/Finished`, `fetchHeadersStart/Progress/Finished`, `discoverAddressesStart/Finished`, `rescanStart/Progress/Finished` | Nil-safe invocations of the corresponding `Callbacks` member (the twelfth member, `Synced`, is invoked by `synced`/`unsynced` above). | `chain/sync.go:152-215` |
| `(*Syncer).getHeaders` | `func (s *Syncer) getHeaders(ctx) error` | Header + cfilter catch-up, address discovery, filter load, rescan; §5.5. | `chain/sync.go:219-398` |
| `(*Syncer).getMissingHeaders` | `func (s *Syncer) getMissingHeaders(ctx) error` | Per-block catch-up after a gap; §5.7. | `chain/sync.go:406-452` |
| `normalizeAddress` | `func normalizeAddress(addr, defaultPort string) (string, error)` | Appends the default port when missing; §5.1 step 1. | `chain/sync.go:454-465` |
| `(*Syncer).waitRPCSync` | `func (s *Syncer) waitRPCSync(ctx, minHeight int64) error` | Polls `getblockchaininfo` until dcrd is caught up; §5.4. | `chain/sync.go:470-503` |
| `hashStop` | `var hashStop chainhash.Hash` | All-zero stop hash for `getheaders`. | `chain/sync.go:507` |
| `notifier` | struct `{closed atomic.Bool; syncer *Syncer; ctx context.Context; done chan struct{}; connectingBlocks bool}` | Implements `wsrpc.Notifier` and `io.Closer`. | `chain/sync.go:760-815` |
| `(*notifier).Notify` | `func (n *notifier) Notify(method string, params json.RawMessage) error` | Dispatch table, §5.6. | `chain/sync.go:768-808` |
| `(*notifier).Close` | `func (n *notifier) Close() error` | Closes `done` once. | `chain/sync.go:810-815` |
| `winningTickets`, `blockConnected`, `handleBlockConnected`, `relevantTxAccepted`, `storeTSpend`, `mixMessage` | methods on `*Syncer` | Notification handlers, §5.6. | `chain/sync.go:817-966` |
| `(*Syncer).Blocks` | `(ctx, []*chainhash.Hash) ([]*wire.MsgBlock, error)` | → `dcrd.RPC.Blocks`. | `chain/backend.go:21-23` |
| `filterProof` | type alias of `struct{Filter *gcs.FilterV2; ProofIndex uint32; Proof []chainhash.Hash}` | Identical anonymous struct as `wallet.FilterProof`. | `chain/backend.go:25-29` |
| `(*Syncer).CFiltersV2` | `(ctx, []*chainhash.Hash) ([]filterProof, error)` | → `dcrd.RPC.CFiltersV2`. | `chain/backend.go:32-34` |
| `(*Syncer).PublishTransactions` | `(ctx, ...*wire.MsgTx) error` | → `dcrd.RPC.PublishTransactions`. | `chain/backend.go:37-39` |
| `(*Syncer).PublishMixMessages` | `(ctx, ...mixing.Message) error` | → `dcrd.RPC.PublishMixMessages`. | `chain/backend.go:43-45` |
| `(*Syncer).LoadTxFilter` | `(ctx, reload bool, addrs []stdaddr.Address, outpoints []wire.OutPoint) error` | → `dcrd.RPC.LoadTxFilter`. | `chain/backend.go:48-50` |
| `(*Syncer).Rescan` | `(ctx, blocks []chainhash.Hash, save func(*chainhash.Hash, []*wire.MsgTx) error) error` | → `dcrd.RPC.Rescan`. | `chain/backend.go:53-55` |
| `(*Syncer).StakeDifficulty` | `(ctx) (dcrutil.Amount, error)` | → `dcrd.RPC.StakeDifficulty` (see quirk Q1). | `chain/backend.go:58-60` |
| `(*Syncer).Deployments` | `(ctx) (map[string]dcrdtypes.AgendaInfo, error)` | `getblockchaininfo`, returns the `deployments` member. Satisfies `deployments.Querier`. | `chain/backend.go:63-69` |
| `(*Syncer).GetTxOut` | `(ctx, txHash *chainhash.Hash, index uint32, tree int8, includeMempool bool) (*dcrdtypes.GetTxOutResult, error)` | → `dcrd.RPC.GetTxOut`. The Go comment says it "fulfills the LiveTicketQuerier interface", but `wallet.LiveTicketQuerier` declares only `ExistsLiveTickets` (`wallet/tickets.go:18-23`); the only caller is JSON-RPC `signrawtransaction`, directly on the `*chain.Syncer` (`internal/rpc/jsonrpc/methods.go:5016`). | `chain/backend.go:72-74` |
| `(*Syncer).GetConfirmationHeight` | `(ctx, txHash *chainhash.Hash) (int32, error)` | → `dcrd.RPC.GetConfirmationHeight`. Same comment-vs-interface disagreement as `GetTxOut`; no caller in the tree at this pin. | `chain/backend.go:77-79` |
| `(*Syncer).ExistsLiveTickets` | `(ctx, []*chainhash.Hash) (bitset.Bytes, error)` | → `dcrd.RPC.ExistsLiveTickets`. The single method of `wallet.LiveTicketQuerier`. | `chain/backend.go:82-84` |
| `(*Syncer).UsedAddresses` | `(ctx, []stdaddr.Address) (bitset.Bytes, error)` | → `dcrd.RPC.UsedAddresses`. `usedAddressesQuerier`; its presence switches address discovery to the dcrd exists-address index. | `chain/backend.go:87-89` |
| `(*Syncer).WrapContext` | `(ctx) (context.Context, context.CancelFunc)` | Child context cancelled when `s.done` closes (i.e. when `Run` returns) or when the returned cancel is called. Spawns one goroutine per call. | `chain/backend.go:94-106` |
| `log`, `UseLogger` | `var log = slog.Disabled`; `func UseLogger(slog.Logger)` | Package logger. The daemon wires it to the `SYNC` subsystem (shared with `spv`). | `chain/log.go:13-20`, `log.go:35` |
| `semver` | `struct{Major, Minor, Patch uint32}` with JSON keys `major`, `minor`, `patch` | Decoded from dcrd's `version` reply. | `chain/semver.go:9-13` |
| `semverCompatible` | `func semverCompatible(required, actual semver) bool` | §5.3. | `chain/semver.go:15-26` |
| `(semver).String` | `"%d.%d.%d"` | Used in the incompatibility error text. | `chain/semver.go:28-30` |

#### 3.1.1 Interface conformance of `*chain.Syncer`

There is no compile-time assertion in `chain/`; conformance is established by the `SetNetworkBackend(s)` call and by run-time type assertions in the wallet.

| Interface | Declared at | Methods | Supplied by |
|---|---|---|---|
| `wallet.NetworkBackend` | `wallet/network.go:35-57` | `Blocks`, `CFiltersV2`, `PublishTransactions`, `PublishMixMessages`, `LoadTxFilter`, `Rescan`, `StakeDifficulty`, `Synced`, `WrapContext` (nine, all required) | `chain/backend.go:21-60,94-106`, `chain/sync.go:123-132` |
| `deployments.Querier` | `deployments/deployments.go:82-86` | `Deployments` | `chain/backend.go:63-69`; asserted at `wallet/chainntfns.go:709`, `wallet/createtx.go:1285`, `wallet/wallet.go:1636` |
| `wallet.LiveTicketQuerier` | `wallet/tickets.go:18-23` | `ExistsLiveTickets` | `chain/backend.go:82-84`; asserted at `internal/rpc/jsonrpc/methods.go:2603` (a failed assertion yields a nil querier, which the wallet treats as SPV) |
| `usedAddressesQuerier` (unexported, `wallet`) | `wallet/discovery.go:29-31` | `UsedAddresses` | `chain/backend.go:87-89`; asserted at `wallet/discovery.go:765,830` |
| `wsrpc.Notifier` + `io.Closer` | `wsrpc/v2@v2.3.8/rpc.go:59-61` | `Notify`, `Close` | `*chain.notifier`, `chain/sync.go:768-815` |
| `dcrd.Caller` | `rpc/client/dcrd/calls.go:31-38` | `Call` | `*wsrpc.Client`; `*dcrd.RPC` embeds it, so `Syncer.RPC().Call(...)` reaches the websocket client directly |

`wallet.Caller` (`wallet/network.go:83-90`) is a second, textually identical declaration of the `Call` interface; nothing in the tree refers to it. `wallet.FilterProof` (`wallet/network.go:26-30`), `chain.filterProof` and `dcrd.filterProof` are three aliases of one anonymous struct type so that the three packages agree on the `CFiltersV2` signature without an import cycle.

`WrapContext` is called by the wallet at `wallet/wallet.go:1596` (ticket purchase) and `wallet/mixing.go:271`; both defer the returned cancel. The effect is that a ticket purchase or a mix in progress is cancelled as soon as `Run` returns (connection lost), not when the next call on the dead backend fails.

`*dcrd.RPC` values obtained from `Syncer.RPC()` are also passed into the wallet package by the RPC servers: `wallet.GetTicketInfoPrecise`, `wallet.GetTicketsPrecise` and `wallet.StakeInfoPrecise` take a `*dcrd.RPC` (`wallet/wallet.go:2947,3102,4115`) and use `ExistsLiveTickets` (through `LiveTicketHashes`/`makeTicketSummary`) and `MempoolCount("tickets")` (`wallet/wallet.go:4121`).

### 3.2 Package `rpc/client/dcrd`

| Item | Signature | Contract | Citation |
|---|---|---|---|
| `Caller` | `interface{ Call(ctx, method string, res any, args ...any) error }` | Positional-parameter JSON-RPC call; `res == nil` discards the result. | `rpc/client/dcrd/calls.go:31-38` |
| `RPC` | `struct{ Caller }` | Thin typed wrapper; all methods below. | `rpc/client/dcrd/calls.go:42-44` |
| `New` | `func New(caller Caller) *RPC` | | `rpc/client/dcrd/calls.go:47-49` |
| `hashSliceToStrings` | `([]*chainhash.Hash) []string` | Reversed-hex strings, always a non-nil slice. | `:51-57` |
| `addrSliceToStrings` | `([]stdaddr.Address) []string` | `Address.String()` for each, always a non-nil slice (so JSON `[]`, never `null`). | `:59-65` |
| `exists` | `(ctx, r, method string, res *bitset.Bytes, param json.RawMessage) error` | Shared body of `exists*` calls returning a hex bitset. | `:69-81` |
| `ExistsLiveTicket` | `(ctx, *chainhash.Hash) (bool, error)` | `existsliveticket`. No caller in tree. | `:85-93` |
| `UsedAddresses` | `(ctx, []stdaddr.Address) (bitset.Bytes, error)` | `existsaddresses`, batched 400 000 per request. | `:98-127` |
| `ExistsLiveTickets` | `(ctx, []*chainhash.Hash) (bitset.Bytes, error)` | `existslivetickets`. | `:131-143` |
| `MempoolCount` | `(ctx, kind string) (int, error)` | `getrawmempool false <kind>`, returns the array length. | `:153-163` |
| `getRawTransaction` | `(ctx, hash string) (*wire.MsgTx, error)` | `getrawtransaction <hash>` (non-verbose). | `:166-170` |
| `GetMempoolTSpends` | `(ctx) ([]*wire.MsgTx, error)` | `getrawmempool false "tspend"` then one `getrawtransaction` per hash, sequentially. | `:173-190` |
| `PublishTransaction` | `(ctx, *wire.MsgTx) error` | `sendrawtransaction`, op `dcrd.PublishTransaction`. No caller in tree. | `:195-198` |
| `publishTransaction` | `(ctx, op, *wire.MsgTx) error` | Duplicate-tx error (−40) is success. | `:200-217` |
| `PublishTransactions` | `(ctx, ...*wire.MsgTx) error` | Sequential; every tx attempted; first error returned. | `:224-241` |
| `publishMixMessage` | `(ctx, op, mixing.Message) error` | `sendrawmixmessage <command> <hex>`. | `:243-250` |
| `PublishMixMessages` | `(ctx, ...mixing.Message) error` | Sequential; every message attempted; first error returned. | `:254-268` |
| `MixMessage` | `(ctx, *chainhash.Hash) (mixing.Message, error)` | `getmixmessage`. | `:271-305` |
| `MixPairRequests` | `(ctx) ([]*wire.MsgMixPairReq, error)` | `getmixpairrequests`. No caller in tree. | `:309-318` |
| `Blocks` | `(ctx, []*chainhash.Hash) ([]*wire.MsgBlock, error)` | One concurrent `getblock <hash> false` per hash. | `:321-337` |
| `CFilterV2` | `(ctx, *chainhash.Hash) (*gcs.FilterV2, uint32, []chainhash.Hash, error)` | `getcfilterv2`. | `:341-352` |
| `filterProof` | alias, same anonymous struct as `wallet.FilterProof` | | `:356-360` |
| `CFiltersV2` | `(ctx, []*chainhash.Hash) ([]filterProof, error)` | One concurrent `getcfilterv2` per hash; returns the (possibly partial) slice together with the error. | `:365-389` |
| `Headers` | `(ctx, locators []*chainhash.Hash, hashStop *chainhash.Hash) ([]*wire.BlockHeader, error)` | `getheaders`. | `:393-406` |
| `LoadTxFilter` | `(ctx, reload bool, []stdaddr.Address, []wire.OutPoint) error` | `loadtxfilter`. | `:411-433` |
| `Rescan` | `(ctx, []chainhash.Hash, save) error` | `rescan`; `save` invoked once per block that dcrd reports. | `:438-471` |
| `StakeDifficulty` | `(ctx) (dcrutil.Amount, error)` | `getstakedifficulty` (see Q1). | `:475-490` |
| `GetBlockchainInfo` | `(ctx) (*dcrdtypes.GetBlockChainInfoResult, error)` | `getblockchaininfo`. | `:493-502` |
| `GetTxOut` | `(ctx, *chainhash.Hash, uint32, int8, bool) (*dcrdtypes.GetTxOutResult, error)` | `gettxout`; `(nil, nil)` when dcrd answers `null`. | `:509-518` |
| `GetConfirmationHeight` | `(ctx, *chainhash.Hash) (int32, error)` | `getrawtransaction <hash> 1`, field `blockheight`; `-1` on error. Reached only through `Syncer.GetConfirmationHeight`, which has no caller in the tree. | `:525-535` |
| `(*RPC).String` | `() string` | The caller's `String()` when it has one (the websocket URL for `wsrpc.Client`), else `"rpc"`. | `:538-543` |
| `codeDuplicateTx` | `const = -40` | dcrd `ErrRPCDuplicateTx`. | `rpc/client/dcrd/codes.go:7-9` |
| `deserializer` | `interface{ Deserialize(io.Reader) error }` | | `rpc/client/dcrd/json.go:21-23` |
| `unmarshalFunc`, `unhex` | `func unhex(msg deserializer) json.Unmarshaler` | JSON string → streaming hex decode → `msg.Deserialize`. | `json.go:25-44` |
| `cfilterV2` | `struct{ Filter *gcs.FilterV2 }` with `Deserialize` and `UnmarshalJSON` | Hex string → `gcs.FromBytesV2(19, 784931, bytes)`. | `json.go:52-78` |
| `cfilterV2Reply` | `struct{ Filter *cfilterV2 "data"; ProofIndex uint32 "proofindex"; ProofHashes *hashes "proofhashes" }`, `proofHashes()` | `getcfilterv2` result. | `json.go:82-99` |
| `hash` | `struct{ Hash *chainhash.Hash }` + `UnmarshalJSON` | Exactly 64 hex digits, byte-reversed. | `json.go:102-120` |
| `hashes` | `struct{ Hashes []*chainhash.Hash }` + `UnmarshalJSON`, `MarshalJSON` | JSON array of reversed-hex strings; nil ⇄ `null`. | `json.go:124-174` |
| `hashesContiguous` | `struct{ Hashes []chainhash.Hash }` + same two methods | Same encoding over a value slice. | `json.go:176-226` |
| `headers` | `struct{ Headers []*wire.BlockHeader }` + `UnmarshalJSON` | Array of hex-serialized 180-byte headers. | `json.go:230-253` |
| `transactions` | `struct{ Transactions []*wire.MsgTx }` + `UnmarshalJSON` | Array of hex-serialized transactions. | `json.go:255-281` |
| `msgPtr`, `messageArray`, `makeMessageArray` | generic array-of-hex-wire-message decoder with protocol version | Used only by `MixPairRequests`. | `json.go:283-321` |
| `buffer` | `struct{ Buffer bytes.Buffer }` + `Deserialize` | Collects hex-decoded bytes. | `json.go:323-330` |
| `unmarshalArray` | `func unmarshalArray(j json.RawMessage, params ...any) error` | Positional decode of a JSON array into caller-supplied pointers; §4.5. | `rpc/client/dcrd/notifications.go:16-22` |
| `WinningTickets` | `(params) (block *chainhash.Hash, height int32, winners []*chainhash.Hash, err error)` | | `notifications.go:26-42` |
| `BlockConnected` | `(params) (header *wire.BlockHeader, relevant []*wire.MsgTx, err error)` | | `notifications.go:46-57` |
| `RelevantTxAccepted` | `(params) (*wire.MsgTx, error)` | | `notifications.go:61-67` |
| `TSpend` | `(params) (*wire.MsgTx, error)` | | `notifications.go:70-76` |
| `MixMessage` | `(params) (mixing.Message, error)` | | `notifications.go:80-115` |

### 3.3 Package `wsrpc` (as used)

| Item | Signature / value | Contract | Citation |
|---|---|---|---|
| `writeWait` | `10 * time.Second` | Write deadline for requests, pings and the close frame. | `wsrpc/v2@v2.3.8/rpc.go:33` |
| `Error` | `struct{ Code int64 "code"; Message string "message"; Data json.RawMessage "data,omitempty" }`; `Error()` returns `Message` only | JSON-RPC error object. | `rpc.go:36-42` |
| `Notifier` | `interface{ Notify(method string, params json.RawMessage) error }` | Never called concurrently; called in receive order; a non-nil return closes the client; if it also implements `io.Closer`, `Close` is called when the client's error is set. | `rpc.go:59-61,278-280` |
| `Call` (interface), `call` | async call handle | `Result()`, `Done()`. Only used internally by `Client.Call`. | `rpc.go:63-96` |
| `request` | `struct{ JSONRPC string "jsonrpc"; Method string "method"; Params []interface{} "params,omitempty"; ID uint32 "id" }` | Outgoing frame, §4.1. | `rpc.go:98-105` |
| `Client` | struct | One websocket, a call map keyed by id, an unbuffered `send` channel, final-error state. | `rpc.go:108-121` |
| `Option`, `DialFunc` | functional options | | `rpc.go:133-137` |
| `WithDial(dial)` | sets the TCP dial function | | `rpc.go:140-144` |
| `WithBasicAuth(user, pass)` | adds header `Authorization: Basic base64(user ":" pass)` (standard alphabet, padded) | | `rpc.go:148-155` |
| `WithTLSConfig(cfg)` | TLS client config for `wss` | | `rpc.go:159-163` |
| `WithNotifier(n)` | notification sink; without one, notifications are dropped | | `rpc.go:168-172` |
| `WithPingPeriod(d)` | sets `pingPeriod = d` and `pongWait = 10·d/9` (default period 60 s) | Not used by dcrwallet. | `rpc.go:179-184` |
| `WithoutPongDeadline()` | sets `pongWait = 0` | Pings are still sent; no read deadline is ever set. Used by dcrwallet. | `rpc.go:189-193` |
| `Dial(ctx, addr, opts...)` | `(*Client, error)` | Websocket handshake; starts the reader and writer goroutines. | `rpc.go:197-249` |
| `(*Client).String` | dialed URL | Appears in the "Fetched … header(s) … from %s" log line. | `rpc.go:252-254` |
| `(*Client).Close` | sends close frame 1000 with empty reason, then closes the socket | | `rpc.go:258-268` |
| `(*Client).Call` | `(ctx, method, result, args...) error` | Blocks for the response, the context, or client failure. | `rpc.go:430-447` |
| `(*Client).Go` | async variant | Used only via `Call`. | `rpc.go:454-517` |
| `(*Client).Done` / `Err` | channel closed / blocking getter for the final error | | `rpc.go:520-528` |
| `options` (unexported) | `struct{ tls *tls.Config; header http.Header; dial DialFunc; notify Notifier; pongWait, pingPeriod time.Duration }` | Accumulator for `Option` functions. `header` is nil unless `WithBasicAuth` ran; `WithBasicAuth` uses `Header.Add`, so applying it twice would send two `Authorization` values (dcrwallet applies it at most once). | `rpc.go:123-130,148-155` |
| `Client` fields | `atomicSeq uint32; addr string; ws *websocket.Conn; pongWait, pingPeriod time.Duration; notify Notifier; calls map[uint32]*call; callMu sync.Mutex; send chan *request; errMu sync.Mutex; errc chan struct{}; err error` | `calls == nil` is the "client has failed" marker read under `callMu`. | `rpc.go:108-121` |
| `call` fields, `finalize` | `finalized uint32; result interface{}; err error; done chan Call` | `finalize` panics with `wsrpc: double call finalization` if called twice; `Result` panics with `wsrpc: Result called before RPC completed`; `Go` panics with `wsrpc: done channel is unbuffered` for an unbuffered caller-supplied channel. None is reachable through `Client.Call`, which always supplies a channel of capacity 1; a port needs no equivalent. | `rpc.go:63-85,455-460` |
| `request.ctx` | unexported, not serialized | Carried only for execution-trace logging. | `rpc.go:104,333-336` |
| `WithPingPeriod(0)` | — | A zero period disables pings and the pong handler altogether (`pingTicker` stays nil). Not used by dcrwallet. | `rpc.go:226,300-304` |
| `cmd/wsrpc`, `cmd/wsrpc-agent` | command-line tools shipped in the module | Not imported by dcrwallet; out of scope. | `wsrpc/v2@v2.3.8/cmd/` |
| pong deadline (default, i.e. without `WithoutPongDeadline`) | `pongWait = 10·60 s/9` = 66.666… s | At dial time the socket read deadline is set to now + `pingPeriod` + `pongWait` (≈ 126.67 s); every received pong moves it to now + `pongWait`. A read that hits the deadline fails with a timeout error, which becomes the client's final error. The daemon's syncer never takes this path (it always passes `WithoutPongDeadline`), but `cmd/sweepaccount` does (next row). | `rpc.go:201-202,227-243` |
| other in-tree users of the wsrpc client | `cmd/repaircfilters`, `cmd/sweepaccount` | Both are clients of **dcrwallet's** JSON-RPC server, not of dcrd, and are outside this scope; listed because they fix which option combinations the transport must support. `repaircfilters`: `WithBasicAuth`, `WithoutPongDeadline`, `WithTLSConfig{RootCAs}` (no minimum version, no suite list), URL `"wss://" + rpcconnect + "/ws"`, no notifier. `sweepaccount`: `WithTLSConfig{RootCAs}` then `WithBasicAuth`, the URL taken verbatim from its `rpcconnect` option, default ping period **and default pong deadline**, no notifier (so notifications are dropped). | `cmd/repaircfilters/main.go:169-183`, `cmd/sweepaccount/main.go:308-325` |

### 3.4 Main package wiring

| Item | Contract | Citation |
|---|---|---|
| backend selection | After wallet load: `--offline` → `wallet.OfflineNetworkBackend{}`; `--spv` → `spvLoop`; otherwise `rpcSyncLoop`. Skipped entirely under `--noinitialload`. | `dcrwallet.go:417-443` |
| `rpcSyncLoop(ctx, w)` | Reads the CA and client cert/key once, then loops forever creating a `Syncer` and calling `Run`; §5.1/§5.9. | `dcrwallet.go:562-600` |
| `readCAFile()` | Returns the bytes of `cfg.CAFile` unless `--noclienttls`; on read failure logs a warning and returns `nil`. | `dcrwallet.go:602-619` |
| `readClientCertKey()` | Returns `(nil, nil)` unless `dcrdauthtype=clientcert`; otherwise the bytes of `dcrdclientcert` and `dcrdclientkey`, each replaced by `nil` (with a warning) on read failure. | `dcrwallet.go:621-636` |

---

## 4. Data formats

The RPC syncer persists nothing of its own. All database writes happen inside the wallet methods it calls (`ChainSwitch`, `AddTransaction`, `saveRescanned`, `InsertMissingCFilters`, …), specified elsewhere. Everything in this section is wire data exchanged with dcrd.

### 4.1 Transport

#### 4.1.1 URL

`"ws://" + hostport + "/ws"` when `RPCOptions.Insecure`, otherwise `"wss://" + hostport + "/ws"` (`chain/sync.go:541-545`). `hostport` is `normalizeAddress(Address, DefaultPort)` (§5.1).

#### 4.1.2 Opening handshake (client → dcrd)

Produced by gorilla's `Dialer.DialContext` with `EnableCompression: true`, no subprotocols, no cookie jar, no HTTP proxy function, no handshake timeout (`wsrpc/v2@v2.3.8/rpc.go:206-211`, `gorilla/websocket@v1.5.1/client.go:192-244`). Go's `http.Request.Write` emits the request line, `Host`, a default `User-Agent`, then the remaining headers sorted by header name. The exact bytes are:

```
GET /ws HTTP/1.1\r\n
Host: <hostport>\r\n
User-Agent: Go-http-client/1.1\r\n
Authorization: Basic <base64(user ":" pass)>\r\n        (only when RPCOptions.User != "")
Connection: Upgrade\r\n
Sec-WebSocket-Extensions: permessage-deflate; server_no_context_takeover; client_no_context_takeover\r\n
Sec-WebSocket-Key: <base64 of 16 random bytes>\r\n
Sec-WebSocket-Version: 13\r\n
Upgrade: websocket\r\n
\r\n
```

The response is accepted only when the status is 101, `Upgrade` contains the token `websocket`, `Connection` contains the token `upgrade` (case-insensitive token match), and `Sec-Websocket-Accept` equals base64(SHA-1(key ‖ `258EAFA5-E914-47DA-95CA-C5AB0DC85B11`)); otherwise the dial fails with the error text `websocket: bad handshake` (`client.go:402-413`). An HTTP 401 from dcrd therefore surfaces as `websocket: bad handshake`, not as an authentication-specific error.

If the server's reply lists the `permessage-deflate` extension it must carry both `server_no_context_takeover` and `client_no_context_takeover`, or the dial fails (`websocket: invalid compression negotiation`); when accepted, every data frame the client writes is compressed and RSV1-marked frames are inflated on read (`client.go:415-427`, `conn.go:539-541,839-843`). dcrd does not negotiate the extension (dcroxide's frame layer rejects RSV1: `dcroxide:crates/dcroxide-node/src/wsframe.rs:231`), so in practice all frames are uncompressed.

#### 4.1.3 Frames

- Client → server requests: one masked text **message** per JSON-RPC request (gorilla `WriteJSON`, `json.go:23-34`). The payload is the JSON encoding followed by a single `\n` byte (Go `json.Encoder.Encode`). The message is a single frame only while the payload is at most 4096 bytes (gorilla's default write buffer; wsrpc sets no `WriteBufferSize`); a longer payload is fragmented into a text frame with FIN = 0 carrying the first 4096 payload bytes, continuation frames of 4096 payload bytes each, and a final continuation frame (FIN = 1) with the remainder (1–4096 bytes), each frame with its own mask key (`gorilla/websocket@v1.5.1/conn.go:38,305,571-697`). Large `loadtxfilter`, `rescan`, `existsaddresses`, `existslivetickets` and `sendrawtransaction` requests are therefore fragmented; dcrd reassembles them.
- Client → server ping: a ping control frame with empty payload every 60 s (`rpc.go:201,244,345-352`). The ticker starts at dial time; pings have priority over queued requests (`rpc.go:306-318`).
- Client → server close: close control frame, status 1000, empty reason, written with a 10 s deadline, immediately followed by closing the TCP/TLS connection without waiting for the peer's close (`rpc.go:258-268`).
- Server → client: each text (or binary) message, reassembled from its fragments, is decoded as exactly one JSON value (`json.go:49-60`); gorilla's default handlers answer server pings with pongs and echo server close frames. No read limit is set. No read deadline is ever set because dcrwallet passes `WithoutPongDeadline()` (`chain/sync.go:551`, `rpc.go:227-243`).
- Control frames from the server, as handled by gorilla's default handlers (wsrpc replaces only the pong handler): a **ping** is answered with a pong carrying the same application data, written with a 1 s deadline; a failure to write that pong because a close was already sent or because of a network error is ignored (`gorilla/websocket@v1.5.1/conn.go:35,1187-1200`). A **pong** invokes wsrpc's handler, which with `pongWait == 0` does nothing (`rpc.go:227-236`). A **close** frame is echoed with the received status code and an empty reason (also with the 1 s deadline), and the pending read then fails with a `*websocket.CloseError` carrying the peer's code and text (`conn.go:1162-1173`); that error becomes the client's final error (§8.1).
- An empty data message (zero payload bytes) fails JSON decoding with `unexpected EOF` (gorilla converts `io.EOF` to `io.ErrUnexpectedEOF`, `gorilla/websocket@v1.5.1/json.go:54-58`) and kills the connection. Bytes following the first JSON value inside one message are not examined.

#### 4.1.4 Request object

Field order and names are fixed by the struct (`rpc.go:98-105`):

```
{"jsonrpc":"2.0","method":"<method>","params":[<p0>,<p1>,…],"id":<n>}\n
```

- `params` is **omitted entirely** when the call has no arguments (`omitempty` on an empty slice): `{"jsonrpc":"2.0","method":"getcurrentnet","id":1}\n`.
- `id` is a JSON number from a per-connection `uint32` counter: first request is `1`, incremented by 1 for every call that passes the pre-send context check, in call order under concurrency (atomic add). Value `0` is never used; on wraparound the counter is advanced a second time (`rpc.go:471-474`).
- Parameter values are encoded by Go `encoding/json` with default HTML escaping: inside strings the characters `<`, `>`, `&`, U+2028 and U+2029 are written as the six-character escapes `\u003c`, `\u003e`, `\u0026`, `\u2028`, `\u2029` respectively. This matters only for passthrough parameters supplied by wallet RPC clients; everything the syncer generates is hex, base58 or ASCII method data. `json.RawMessage` parameters are compacted (insignificant whitespace removed) and HTML-escaped the same way.

#### 4.1.5 Response and notification objects

Every incoming message is decoded into (`rpc.go:366-374`):

| JSON key | Go type | Notes |
|---|---|---|
| `result` | raw JSON | absent → nil; present, including the literal `null` → non-nil raw bytes |
| `error` | `*Error` (`code` int64, `message` string, `data` raw, optional) | `null` or absent → nil |
| `id` | uint32 | `null` or absent → 0; a non-number or out-of-range id is a decode error that kills the connection |
| `method` | string | |
| `params` | raw JSON | |

Unknown keys (e.g. `jsonrpc`) are ignored; key matching follows Go's rules (exact match preferred, then case-insensitive).

Classification (`rpc.go:384-424`):

1. **Notification** iff `method != ""` and `result` absent and `error` nil and `id == 0`. dcrd marshals notifications as `{"jsonrpc":"1.0","method":"<m>","params":[…],"id":null}` (`dcroxide:crates/dcroxide-rpc/src/websocket.rs:801-812`). The pair `(method, params)` is queued for the notifier; if no notifier is configured it is dropped.
2. Otherwise a **response**: the pending call with that id is removed from the call map. If no call is registered under the id, the client fails with the error `wsrpc: unknown response ID` and the connection is torn down. This includes any server message that carries an `error` or a `result` together with a `null`/absent id (for example a JSON-RPC parse-error reply, which by convention has `"id":null`): it decodes to id 0, no call is ever registered under 0, and the connection is killed. It also includes a message with an empty/absent `method` and no `result`/`error` at all (e.g. `{}`). If `error` is non-nil the call fails with that `*wsrpc.Error`. Else, if the caller supplied a result target, `result` is decoded into it (decode errors become the call's error; an absent `result` key yields `EOF`). If the caller passed a nil target the result is not looked at.

Result decoding rules that follow from `json.NewDecoder(result).Decode(target)` (`rpc.go:419-421`) and that every typed wrapper inherits:

| Case | Behaviour |
|---|---|
| `result` is the literal `null` | No error. A pointer target is set to nil (`GetBlockchainInfo`, `GetTxOut`, `MixMessage`, see Q9); a struct, number, string, bool or slice target is left at its initial value (so `getcurrentnet` → 0 → `mismatched networks`; `version` → 0.0.0 → incompatible; `getrawmempool` → empty list). A target with a custom `UnmarshalJSON` (`unhex`, `messageArray`) **is** invoked with `null` and applies its own rule (`unhex`: `not a string`). |
| `result` has the wrong JSON type for the target | The call fails with Go's `encoding/json` error (`json: cannot unmarshal <kind> into Go value of type <T>`), not wrapped in any kind; the connection stays up. |
| number out of range for the target (`int32` heights, `uint32` semver parts, negative into unsigned) | same `encoding/json` type error |
| object keys | matched exactly first, then case-insensitively; unknown keys ignored; duplicate keys: the last one wins |
| bytes after the first JSON value in `result` | cannot occur (`result` is one raw value extracted by the outer decoder) |
| target is nil | `result` is never inspected, whatever it contains |

A server message carrying a non-empty `method` together with a **non-zero** `id` (a server-to-client request, which dcrd never sends) is not a notification: it is treated as a response to that id. If a call with that id is pending it is completed — with `EOF` when the caller wanted a result (the `result` key is absent), with success when it did not; otherwise the client dies with `wsrpc: unknown response ID`.

### 4.2 dcrd methods called: request parameters

`H(x)` denotes the 64-character lower-case hex string of a 32-byte hash with bytes reversed (`chainhash.Hash.String`). All calls are positional.

| # | Method | Params (JSON, in order) | Issued by | Citation |
|---:|---|---|---|---|
| 1 | `getcurrentnet` | none | `Run` | `chain/sync.go:588` |
| 2 | `version` | none | `Run`; JSON-RPC `version` | `chain/sync.go:600` |
| 3 | `getblockchaininfo` | none | `waitRPCSync`, `getHeaders` (when unsynced), `Syncer.Deployments` | `calls.go:497` |
| 4 | `notifywinningtickets` | none | `Run`, only if voting enabled | `chain/sync.go:646` |
| 5 | `getheaders` | `[ [H(loc0),H(loc1),…] , H(stop) ]`; first element is `null` if the locator slice is nil; `stop` is always the zero hash → `"0000…0000"` (64 zeros) | `getHeaders`, `getMissingHeaders` | `calls.go:401`, `json.go:149-174` |
| 6 | `getcfilterv2` | `[ H(block) ]` | `CFilterV2`, `CFiltersV2` | `calls.go:345,373` |
| 7 | `existsaddresses` | `[ ["addr0","addr1",…] ]` — the array is pre-marshalled and passed as raw JSON; at most 400 000 addresses per request | `UsedAddresses` | `calls.go:104-124` |
| 8 | `loadtxfilter` | `[ reload(bool), ["addr",…], [{"hash":H(txid),"index":<uint32>,"tree":<int8>},…] ]`; both arrays are always arrays (`[]` when empty, never `null`); object key order is `hash`,`index`,`tree` | `LoadTxFilter` | `calls.go:411-433` |
| 9 | `rescan` | `[ [H(b0),H(b1),…] ]` (`null` if the slice is nil); hashes must form a parent→child chain; the wallet sends at most 2000 per request | `Rescan` | `calls.go:447`, `json.go:201-226`, `wallet/rescan.go:21,231-236` |
| 10 | `notifyblocks` | none | `Run` | `chain/sync.go:677` |
| 11 | `rebroadcastwinners` | none | `Run` (unconditionally, even when voting is disabled) | `chain/sync.go:683` |
| 12 | `sendrawtransaction` | `[ "<tx hex>" ]` — lower-case hex of `MsgTx.Serialize` (full serialization incl. witness); `allowhighfees` omitted | `publishTransaction` | `calls.go:200-207` |
| 13 | `getrawmempool` | `[ false, "<kind>" ]`; kinds used: `"tspend"` (`GetMempoolTSpends`), `"tickets"` (`StakeInfoPrecise`) | `GetMempoolTSpends`, `MempoolCount` | `calls.go:158,176`, `wallet/wallet.go:4121` |
| 14 | `getrawtransaction` | `[ "<txid string>" ]` (non-verbose) | `getRawTransaction` | `calls.go:168` |
| 15 | `getrawtransaction` | `[ H(txid), 1 ]` (verbose) | `GetConfirmationHeight` (no caller in the tree; never issued at this pin) | `calls.go:530` |
| 16 | `notifytspend` | none | `Run` | `chain/sync.go:708` |
| 17 | `notifymixmessages` | none | `Run`, only if mixing enabled | `chain/sync.go:738` |
| 18 | `sendrawmixmessage` | `[ "<command>", "<hex>" ]` — `command` is the wire command string of the message, hex is `BtcEncode(pver = 10)` | `publishMixMessage` | `calls.go:243-250` |
| 19 | `getmixmessage` | `[ H(msg) ]` | `MixMessage` | `calls.go:275` |
| 20 | `getmixpairrequests` | none | `MixPairRequests` (unused) | `calls.go:313` |
| 21 | `getblock` | `[ H(block), false ]` | `Blocks` | `calls.go:329` |
| 22 | `existslivetickets` | `[ [H(t0),H(t1),…] ]` (pre-marshalled raw JSON; `[]` when empty — the call is still made) | `ExistsLiveTickets` | `calls.go:131-143` |
| 23 | `existsliveticket` | `[ H(ticket) ]` | `ExistsLiveTicket` (unused) | `calls.go:88` |
| 24 | `getstakedifficulty` | none | `StakeDifficulty` | `calls.go:481` |
| 25 | `gettxout` | `[ H(txid), <index uint32>, <tree int8>, <includeMempool bool> ]` | `GetTxOut` (JSON-RPC `signrawtransaction`, one concurrent call per input not supplied by the client, `includeMempool = true`) | `calls.go:513`, `internal/rpc/jsonrpc/methods.go:4996-5026` |

Methods reached through `Syncer.RPC().Call` from the wallet's own RPC servers (owned by the JSON-RPC/gRPC specs; listed so the transport supports them):

| Method | Params | Result target | Citation |
|---|---|---|---|
| any method not handled by the wallet (passthrough) | the client's raw params, one raw JSON value each | raw JSON | `internal/rpc/jsonrpc/methods.go:231-258` |
| `getblockheader` | `[hash string, verbose *bool]` (a nil pointer would encode as `null`; the command types carry `jsonrpcdefault` tags — `true` here, `true`/`false` for `getblock`, `true` for `gettxout` — which dcrjson fills in when the client omits the parameter, so the default is sent explicitly, `types/v4@v4.4.0/chainsvrcmds.go:357-361,408-411,738-743`, re-declared for the wallet in `rpc/jsonrpc/types/methods.go:1324-1330`) | raw JSON | `methods.go:1234-1238` |
| `getblock` | `[hash, verbose *bool, verbosetx *bool]` | raw JSON | `methods.go:1352-1356` |
| `getinfo` | none | `InfoChainResult` | `methods.go:1811-1814` |
| `getpeerinfo` | none | `[]GetPeerInfoResult` (JSON-RPC) or a local struct (gRPC) | `methods.go:2503-2504`, `rpcserver/server.go:3999-4000` |
| `gettxout` | `[txid, vout, tree, includeMempool *bool]` | raw JSON | `methods.go:2731-2735` |
| `help` | none, or `[command]` | string | `methods.go:2897-2931` |
| `version` | none | `map[string]VersionResult` | `methods.go:5441-5442` |
| `ping` | none | discarded | `methods.go:5480` |

### 4.3 dcrd methods called: result decoding

| Method | Expected `result` | Decoding | Citation |
|---|---|---|---|
| `getcurrentnet` | JSON number | into `uint32` (`wire.CurrencyNet`). MainNet `0xd9b400f9` = 3652452601, TestNet3 `0xb194aa75` = 2979310197, SimNet `0x12141c16` = 303307798. | `chain/sync.go:587-594`, `wire@v1.8.0/protocol.go:131-146` |
| `version` | object keyed by component name | only key `dcrdjsonrpcapi` is read, and from it only `major`, `minor`, `patch` (uint32). Absent key → 0.0.0. | `chain/sync.go:597-607` |
| `getblockchaininfo` | object | `GetBlockChainInfoResult`: `chain` string, `blocks` int64, `headers` int64, `syncheight` int64, `bestblockhash` string, `difficulty` uint32, `difficultyratio` float64, `verificationprogress` float64, `chainwork` string, `initialblockdownload` bool, `maxblocksize` int64, `deployments` map name → `{status string, since int64 (optional), starttime uint64, expiretime uint64}`. The syncer reads `blocks`, `headers`, `initialblockdownload`, `deployments`. | `types/v4@v4.4.0/chainsvrresults.go:106-134` |
| `getheaders` | `{"headers":["<hex>",…]}` | `headers` codec §4.4.5. Result order is the server's (ascending height). | `calls.go:396-405` |
| `getcfilterv2` | `{"blockhash":…,"data":"<hex>","proofindex":<n>,"proofhashes":["<H>",…]}` | `data` → `cfilterV2` (§4.4.2); `proofindex` → uint32; `proofhashes` → `hashes` (§4.4.3) then copied into `[]chainhash.Hash` (nil when the key is absent or `null`). `blockhash` is ignored. | `json.go:82-99`, `calls.go:344-351` |
| `existsaddresses`, `existslivetickets` | JSON string, hex | hex-decoded (either case) into a byte bitset: item *i* is bit `i & 7` (LSB first) of byte `i >> 3`. Batches are concatenated. | `calls.go:69-81,110-126`, `bitset@v1.0.0/bitset.go:151-153` |
| `existsliveticket` | bool | | `calls.go:87-92` |
| `getrawmempool` | array of strings | only the length (`MempoolCount`) or the strings themselves (`GetMempoolTSpends`, passed back verbatim as the `getrawtransaction` parameter) | `calls.go:157-189` |
| `getrawtransaction` (non-verbose) | JSON string, hex | `unhex` → `MsgTx.Deserialize` | `calls.go:166-170` |
| `getrawtransaction` (verbose) | object | only `blockheight` (int32) is read; absent → 0 | `calls.go:527-534` |
| `getblock` (verbose=false) | JSON string, hex | `unhex` → `MsgBlock.Deserialize` | `calls.go:328-329` |
| `rescan` | `{"discovereddata":[{"hash":"<H>","transactions":["<hex>",…]},…]}` | for each entry in order: `hash` via `chainhash.NewHashFromStr`, each tx via streaming hex → `MsgTx.Deserialize`; then `save(hash, txs)`. dcrd lists only blocks that contained at least one match (`dcroxide:crates/dcroxide-rpc/src/websocket.rs:643-648`). | `calls.go:441-470` |
| `getmixmessage` | `{"type":"<command>","message":"<hex>"}` | command → concrete type (table §4.5.5), hex → `BtcDecode(pver 10)` | `calls.go:274-304` |
| `getmixpairrequests` | array of hex strings | `messageArray[MsgMixPairReq]` with pver 10 | `calls.go:312-317`, `json.go:299-321` |
| `getstakedifficulty` | object | only key `nextstakedifficulty` (float64) is read; dcrd's keys are `current` and `next`, so the value is always 0 (Q1) | `calls.go:478-489`, `types/v4@v4.4.0/chainsvrresults.go:339-342` |
| `gettxout` | object or `null` | `*GetTxOutResult` (`bestblock`, `confirmations`, `value`, `scriptPubKey`{…}, `coinbase`); `null` → nil pointer with nil error | `calls.go:512-517` |
| `loadtxfilter`, `notify*`, `rebroadcastwinners`, `sendrawtransaction`, `sendrawmixmessage`, `ping` | anything | ignored (nil target) | |

Full shapes of the dcrd result types that the wallet decodes into complete structs (and that the RPC servers re-emit or merge), field order as declared:

| Type | Fields (JSON key: Go type) | Citation |
|---|---|---|
| `VersionResult` (value type of the `version` reply map; the syncer reads only the three numbers) | `versionstring`: string, `major`: uint32, `minor`: uint32, `patch`: uint32, `prerelease`: string, `buildmetadata`: string | `types/v4@v4.4.0/chainsvrresults.go:565-572` |
| `AgendaInfo` | `status`: string, `since`: int64 (`omitempty`), `starttime`: uint64, `expiretime`: uint64 | `chainsvrresults.go:106-111` |
| `GetTxOutResult` | `bestblock`: string, `confirmations`: int64, `value`: float64, `scriptPubKey`: `ScriptPubKeyResult`, `coinbase`: bool | `chainsvrresults.go:388-394` |
| `ScriptPubKeyResult` | `asm`: string, `hex`: string (`omitempty`), `reqSigs`: int32 (`omitempty`), `type`: string, `addresses`: []string (`omitempty`), `commitamt`: *float64 (`omitempty`), `version`: uint16 | `chainsvrresults.go:576-584` |
| `GetMixMessageResult` | `type`: string, `message`: string | `chainsvrresults.go:238-241` |

`rescan` reply details (`calls.go:441-470`): `discovereddata` absent or `null` → no `save` call at all; an entry whose `transactions` is absent, `null` or `[]` still produces one `save(hash, txs)` call with a **non-nil empty** list; the `hash` string goes through `chainhash.NewHashFromStr` (short strings accepted and zero-extended, §4.4.3); transaction strings are standard-JSON-decoded first and then streamed through hex into `MsgTx.Deserialize` (trailing hex ignored). Decoding and saving are interleaved entry by entry, so a malformed later entry leaves the earlier entries already saved.

### 4.4 Custom JSON codecs (`rpc/client/dcrd/json.go`)

All "hex" below is Go `encoding/hex`: both `a-f` and `A-F` are accepted on input; output is lower-case.

#### 4.4.1 `unhex(msg)` — hex-encoded wire message in a JSON string

Input is the raw JSON token `j`.

1. If `len(j) < 2` or `j[0] != '"'` or `j[len(j)-1] != '"'` → error kind `Encoding`, message `not a string`.
2. The bytes strictly between the quotes are fed, **without JSON unescaping**, to a streaming hex decoder which is handed to `msg.Deserialize`.
3. Any error from `Deserialize` (including hex errors surfaced through the reader) → kind `Encoding` wrapping that error.

Consequences of the streaming design: bytes following a complete message are never read, so trailing hex (valid or not) after a complete header/transaction/block is silently ignored; an empty string `""` fails inside `Deserialize` with an EOF-class error (`json.go:32-44`).

#### 4.4.2 `cfilterV2` — version 2 committed filter

`UnmarshalJSON(j)` (`json.go:65-78`):

1. If `len(j)` is odd, or `j[0] != '"'`, or the last byte is not `'"'` → kind `Encoding`, message `cfilterv2 is not an even hex string`.
2. Hex-decode `j[1:len(j)-1]` into `len(j)/2 - 1` bytes; error → kind `Encoding`.
3. `gcs.FromBytesV2(B = 19, M = 784931, bytes)`: empty input yields an empty filter (N = 0); otherwise the leading Bitcoin-style varint is N and the remainder is the Golomb-coded data (`gcs/v4@v4.1.1/gcs.go:539-566`, `gcs/v4@v4.1.1/blockcf2/blockcf.go:33-38`). A varint read failure is returned **unwrapped** (not kind `Encoding`).

`Deserialize(r)` reads everything from `r` and applies step 3 (`json.go:56-63`); it is not used on any live path.

A JSON `null` or absent `data` key leaves `cfilterV2Reply.Filter` nil; `CFilterV2`/`CFiltersV2` then dereference it (Q9).

#### 4.4.3 `hash`, `hashes`, `hashesContiguous`

- `hash.UnmarshalJSON(j)` (`json.go:106-120`): requires `len(j) == 66` with leading and trailing `"` → else kind `Encoding`, message `not a 32-byte hash hex string`. Hex-decodes the 64 digits into 32 bytes and reverses them in place. Used for the `winningtickets` block hash and ticket hashes.
- `hashes.UnmarshalJSON(j)` / `hashesContiguous.UnmarshalJSON(j)` (`json.go:128-147,180-199`): the exact bytes `null` → nil slice. Otherwise decode as a JSON array of strings (standard JSON string decoding; a JSON syntax/type error is returned unwrapped) and convert each with `chainhash.NewHashFromStr`; failure → kind `Encoding`. `NewHashFromStr` accepts 0–64 hex digits, left-pads odd lengths with `0`, and zero-extends short values (`chainhash@v1.0.5/hash.go:98-129`); longer strings fail with `ErrHashStrSize`.
- `MarshalJSON` (both, `json.go:149-174,201-226`): nil slice → `null`. Otherwise `[` + for each hash `"` + 64 lower-case hex digits of the byte-reversed hash + `"`, separated by `,` + `]`, with no whitespace. A non-nil empty slice → `[]`.

#### 4.4.4 `transactions`

`UnmarshalJSON(j)` (`json.go:259-281`): `null` → nil slice. Otherwise decode as an array of raw JSON values; each element must be a string token (`len ≥ 2`, quotes at both ends) else kind `Encoding`/`not a string`; its inner bytes go through streaming hex into `MsgTx.Deserialize`; failure → kind `Encoding`. An empty array yields a **non-nil empty** slice. For `blockconnected` the nil/empty distinction is erased again by `handleBlockConnected`, which replaces a nil list by an empty one (§5.6.2 step 7); the distinction that matters to `ChainSwitch` is between that path (always non-nil) and header catch-up (always nil).

#### 4.4.5 `headers`

`UnmarshalJSON(j)` (`json.go:234-253`): `null` → nil slice. Otherwise decode as an array of JSON strings, each streaming-hex-decoded into `BlockHeader.Deserialize` (180 bytes consumed; anything after is ignored); failure → kind `Encoding`.

#### 4.4.6 `messageArray[T]`

Same as `transactions` but each element is decoded with `BtcDecode(reader, pver)` for the configured protocol version (`json.go:299-321`).

### 4.5 Notifications

#### 4.5.1 Positional decoding (`unmarshalArray`)

`unmarshalArray(params, t0, t1, …)` calls `json.Unmarshal(params, &[]any{t0, t1, …})` (`notifications.go:16-22`). Go reuses the existing slice elements, so element *i* of the JSON array is decoded into the value pointed to by `t_i`. Any decode error is wrapped with kind `Encoding`. The exact acceptance rules a port must decide whether to reproduce:

| Input | Go behaviour |
|---|---|
| `params` absent from the message (empty raw bytes) | error (`unexpected end of JSON input`), kind `Encoding` |
| `params` is `null` | no error; all targets keep their initial (zero) values |
| `params` is an object, string, number | error, kind `Encoding` |
| array shorter than the target list | no error; missing targets keep their initial values |
| array longer than the target list | no error; extra elements are decoded generically and discarded |
| element is `null` | no error; that target keeps its initial value and its custom `UnmarshalJSON` is not invoked |

#### 4.5.2 `winningtickets`

Params: `[ "<H(block)>", <height>, { "<k0>": "<H(ticket0)>", "<k1>": "<H(ticket1)>", … } ]`. dcrd keys the object by decimal index `"0"`, `"1"`, … (`dcroxide:crates/dcroxide-rpc/src/websocket.rs:1148-1152`); the wallet ignores keys.

Decoding (`notifications.go:26-42`): element 0 via `hash` (§4.4.3, strict 64 digits); element 1 into `int32`; element 2 into `map[string]*hash`. `winners` is the list of map values in **Go map iteration order (unspecified, randomized)**. Duplicate keys: the last one wins.

#### 4.5.3 `blockconnected`

Params: `[ "<header hex>", ["<tx hex>",…] | null ]`. Element 0 → `unhex(BlockHeader)`; element 1 → `transactions`. dcrd sends `null` for element 1 when no transaction of the block matched the client's filter (stake-tree matches first, then regular-tree, each in block order) (`dcroxide:crates/dcroxide-rpc/src/websocket.rs:932-988`). (`notifications.go:46-57`)

#### 4.5.4 `relevanttxaccepted` and `tspend`

Params: `[ "<tx hex>" ]` → `unhex(MsgTx)` (`notifications.go:61-76`). `relevanttxaccepted` is sent by dcrd, without any explicit subscription, to every websocket client whose loaded filter matches a transaction accepted to the mempool (`dcroxide:crates/dcroxide-rpc/src/websocket.rs:1285-1345`).

#### 4.5.5 `mixmessage`

Params: `[ "<command>", "<payload hex>" ]`. Element 0 → string; element 1 → `unhex(buffer)` (all hex decoded into memory). The command selects the concrete message type, then `BtcDecode(payload, pver = wire.MixVersion = 10)` (`notifications.go:80-115`, `wire@v1.8.0/protocol.go:56`, `wire@v1.8.0/message.go:56-63`):

| Command string | Message type |
|---|---|
| `mixpairreq` | `MsgMixPairReq` |
| `mixkeyxchg` | `MsgMixKeyExchange` |
| `mixcphrtxt` | `MsgMixCiphertexts` |
| `mixslotres` | `MsgMixSlotReserve` |
| `mixdcnet` | `MsgMixDCNet` |
| `mixconfirm` | `MsgMixConfirm` |
| `mixfactpoly` | `MsgMixFactoredPoly` |
| `mixsecrets` | `MsgMixSecrets` |

Any other command → error text `unrecognized mixing message command string` (notification path) or `unrecognized mixing message type` wrapped with op `dcrd.MixMessage` (`getmixmessage` path, `calls.go:298-301`). Decode errors are returned unwrapped together with the partially decoded message.

#### 4.5.6 Notifications received but ignored

Every other method name (`blockdisconnected`, `reorganization`, `newtickets`, `work`, `txaccepted`, `txacceptedverbose`, …) falls through the dispatch switch and is dropped without logging (`chain/sync.go:773-807`).

---

## 5. Algorithms and state machines

### 5.1 Connection parameters

#### 5.1.1 From configuration to `RPCOptions` (daemon path)

The CA file, the client certificate/key and the dial function are obtained once per process before the reconnect loop (`dcrwallet.go:563-568`); a fresh `RPCOptions` value is then built from them and from the (unchanging) configuration at the top of every loop iteration (`dcrwallet.go:570-584`):

| `RPCOptions` field | Source | Notes |
|---|---|---|
| `Address` | `cfg.RPCConnect` (`-c`, `--rpcconnect`) | Config loading defaults it to `localhost:<JSONRPCClientPort>` and normalizes it with the default port (`config.go:784-796`). |
| `DefaultPort` | `activeNet.JSONRPCClientPort` | mainnet `9109`, testnet3 `19109`, simnet `19556` (`internal/netparams/params.go:20-45`). |
| `User`, `Pass` | `cfg.DcrdUsername`, `cfg.DcrdPassword`, each falling back to `--username` / `--password` when empty (`config.go:986-991`) | Both forced to `""` when a client certificate was read (`len(clientCert) != 0`). |
| `Dial` | `cfg.dial`; replaced by a plain `net.Dialer.DialContext` when `--nodcrdproxy` | `cfg.dial` is nil unless `--proxy` is set (`config.go:691-728`). With a proxy it dials directly (no proxy) when the host is exactly `localhost` or a literal IP inside 127.0.0.0/8, ::1/128, 10.0.0.0/8, 172.16.0.0/12, 192.168.0.0/16 or fc00::/7, and through the SOCKS5 proxy otherwise; proxy failures are reported as `proxy dial <network> <address>: <err>`. |
| `CA` | `readCAFile()` | Bytes of `cfg.CAFile`, or nil when `--noclienttls` or when the file cannot be read (warning `Cannot open CA file: <err>` on the `DCRW` logger; info `Chain server RPC TLS is disabled` when TLS is off). Default path resolution: `<appdata>/dcrd.cert`; if that file does not exist and the RPC host is `localhost`, `127.0.0.1` or `::1`, and dcrd's own `rpc.cert` exists in dcrd's default app-data directory, that path is used instead (`config.go:817-841`). |
| `ClientCert`, `ClientKey` | `readClientCertKey()` | Only when `--dcrdauthtype=clientcert`; unreadable files give nil with warnings `Cannot open dcrd RPC client certificate: <err>` / `Cannot open dcrd RPC client key: <err>`. Config loading already refuses to start when either file is missing or when combined with `--noclienttls` (`config.go:993-1020`). |
| `Insecure` | `cfg.DisableClientTLS` (`--noclienttls`) | Config loading only allows it when the RPC host is `localhost`, `127.0.0.1` or `::1` (`config.go:797-815`). |

The files are read **once**; a certificate rotated on disk is not picked up by later reconnects.

Every command-line / `dcrwallet.conf` option that influences the RPC-mode connection (struct tags at `config.go:94,120-146`; parsing, defaults and validation are owned by the configuration spec):

| Option (long name; short) | Type | Effect on RPC mode |
|---|---|---|
| `rpcconnect` (`-c`) | string | dcrd `host[:port]` |
| `cafile` | path | CA / self-signed certificate used to verify dcrd |
| `noclienttls` | bool | plain `ws://`; localhost only |
| `dcrdusername`, `dcrdpassword` | string | Basic credentials; fall back to `username` / `password` |
| `dcrdauthtype` | `basic` \| `clientcert` | selects Basic header or TLS client certificate; any other value is a startup error `unknown dcrd authtype "<v>"` (`config.go:1020-1025`) |
| `dcrdclientcert`, `dcrdclientkey` | path | PEM client certificate and key for `clientcert` |
| `proxy`, `proxyuser`, `proxypass` | string | SOCKS5 proxy and its credentials for the websocket TCP connection (subject to the bypass rules above) |
| `torisolation`, `circuitlimit` | bool, int (default 32) | with `torisolation` the proxied dial goes through a `socks.NewPool(proxy, circuitlimit)` pool instead of a single proxy dialer (`config.go:691-705`) |
| `nodcrdproxy` | bool | dial dcrd directly even when `proxy` is set |
| `spv`, `offline`, `noinitialload` | bool | any of them prevents `rpcSyncLoop` from being started by the main package |

Execution context of the loop (`dcrwallet.go:414-443`, `internal/loader/loader.go:91-104`): `rpcSyncLoop` runs inside the `loader.RunAfterLoad` callback. If the wallet is already loaded when the callback is registered the loop runs synchronously on the main goroutine (which is why it "blocks until cancelled"); otherwise it runs on the goroutine that later creates or opens the wallet. Inside the same callback, and **before** the loop starts, the VSP client's `ProcessManagedTickets` is run when a VSP is configured with `--vsp.sync`; its errors are only logged (`Getting VSP tickets failed: <err>`, `Adding tickets to VSP client failed: <err>` on the `DCRW` logger).

What decides `discoverAccts` on the daemon path (`dcrwallet.go:231-248,468-529`; the prompt itself is owned by the startup spec). `NewSyncer` samples `!w.Locked()`, so account discovery happens only if the wallet was unlocked before the loop started. With `--noinitialload` unset the main package unlocks it in exactly these cases, before the RPC servers and the sync loop are started:

1. `--pass` is non-empty: `w.Unlock(ctx, pass, nil)` (no timeout); a wrong passphrase logs `Incorrect passphrase in pass config setting.` (DCRW error) and aborts startup.
2. Otherwise `startPromptPass`: nothing for a watching-only wallet; otherwise the passphrase is requested on the terminal when `--promptpass` is set, when `--enableticketbuyer` is set, or when `w.NeedsAccountsSync` reports true (first run after a restore from seed). On simnet an unlock with the well-known simulation passphrase is tried first on every round of the prompt loop. The loop repeats until an unlock succeeds or the prompt is cancelled.

In every other case the wallet is locked when the first `Syncer` is created, `discoverAccts` is false, and `DiscoverActiveAddresses` is called with `discoverAccts = false` (address discovery only, within the accounts the wallet already has). Later reconnects re-sample the lock state (Q22).

Before registering the callback the main package returns `ctx.Err()` if shutdown was already requested (`dcrwallet.go:418-420`); after registering it, it blocks on `<-ctx.Done()` (`dcrwallet.go:447-448`). Since `rpcSyncLoop` normally runs synchronously inside `RunAfterLoad`, that wait is reached only after the loop has returned.

#### 5.1.2 gRPC `RpcSync` path

`Address = req.NetworkAddress`, `DefaultPort = JSONRPCClientPort`, `User = req.Username`, `Pass = string(req.Password)`, `CA = req.Certificate`, `Insecure = isLoopback(req.NetworkAddress) && len(req.Certificate) == 0`; `Dial`, `ClientCert`, `ClientKey` are unset, so the configured proxy is never used and client-certificate auth is unavailable (`internal/rpc/rpcserver/server.go:2865-2872`). `isLoopback` strips an optional port and accepts the literal `localhost` or any loopback IP (`server.go:2817-2830`). `Run` is called exactly once; there is no retry loop.

Steps of `RpcSync` around the syncer, in order (`internal/rpc/rpcserver/server.go:2832-2981`; the protobuf message layout is owned by the gRPC spec):

1. `req.Password` is zeroed on return.
2. No wallet loaded → `FailedPrecondition`, `Wallet has not been loaded`.
3. `wallet.NetworkBackend()` succeeds (a backend is already set) → `FailedPrecondition`, `wallet is loaded and already synchronizing`.
4. `req.DiscoverAccounts` with an empty private passphrase → `InvalidArgument`, `private passphrase is required for discovering accounts`.
5. If `req.DiscoverAccounts`: unlock the wallet with the passphrase and a lock-timeout channel of capacity 1; an unlock failure is returned through `translateError`. A `lockWallet` closure (sends on that channel, zeroes the passphrase) is deferred and is also invoked — once — from the `DiscoverAddressesFinished` callback, so the wallet is relocked as soon as the first discovery completes.
6. `chain.NewSyncer` (hence `discoverAccts = !w.Locked()`, evaluated after the unlock of step 5), `SetCallbacks`, `Run(stream context)`.
7. Result mapping per §8.2.

Each callback sends one `RpcSyncResponse` on the stream; send errors are ignored:

| Callback | `notification_type` | Other fields set |
|---|---|---|
| `Synced(true)` / `Synced(false)` | `SYNCED` / `UNSYNCED` | `synced` = the flag |
| `FetchMissingCFiltersStarted` / `Finished` | `FETCHED_MISSING_CFILTERS_STARTED` / `_FINISHED` | — |
| `FetchMissingCFiltersProgress(a, b)` | `FETCHED_MISSING_CFILTERS_PROGRESS` | `fetch_missing_cfilters{fetched_cfilters_start_height = a, fetched_cfilters_end_height = b}` |
| `FetchHeadersStarted` / `Finished` | `FETCHED_HEADERS_STARTED` / `_FINISHED` | — |
| `FetchHeadersProgress(n, t)` | `FETCHED_HEADERS_PROGRESS` | `fetch_headers{fetched_headers_count = n, last_header_time = t}` |
| `DiscoverAddressesStarted` / `Finished` | `DISCOVER_ADDRESSES_STARTED` / `_FINISHED` | — (`Finished` relocks first, then sends) |
| `RescanStarted` / `Finished` | `RESCAN_STARTED` / `_FINISHED` | — |
| `RescanProgress(h)` | `RESCAN_PROGRESS` | `rescan_progress{rescanned_through = h}` |

The daemon path (`rpcSyncLoop`) never calls `SetCallbacks`; all callbacks are no-ops there.

#### 5.1.3 `Run` connection steps (`chain/sync.go:537-584`)

1. `addr = normalizeAddress(opts.Address, opts.DefaultPort)`: if `net.SplitHostPort(addr)` succeeds the result is `JoinHostPort(host, port)`; otherwise `JoinHostPort(addr, defaultPort)` is tried and, if that still does not split, the **original** split error is returned. A failure returns `errors.E(errors.Invalid, err)`.
2. URL per §4.1.1.
3. Options, in this order:
   1. `WithBasicAuth(User, Pass)` only if `User != ""` (a password without a user sends no header).
   2. `WithNotifier(s.notifier)`.
   3. `WithoutPongDeadline()`.
   4. `WithDial(opts.Dial)` only if non-nil.
   5. `WithTLSConfig(tc)` only if `len(opts.CA) != 0 && !opts.Insecure`, where `tc` is:

      | Setting | Value |
      |---|---|
      | Minimum version | TLS 1.2 |
      | TLS 1.2 cipher suites (source order; Go's `crypto/tls` ignores the configured order and offers the enabled set in its own preference order) | `ECDHE-ECDSA-CHACHA20-POLY1305`, `ECDHE-RSA-CHACHA20-POLY1305`, `ECDHE-ECDSA-AES256-GCM-SHA384`, `ECDHE-RSA-AES256-GCM-SHA384`, `ECDHE-ECDSA-AES128-GCM-SHA256`, `ECDHE-RSA-AES128-GCM-SHA256` |
      | TLS 1.3 | enabled with library defaults |
      | Root CAs | a fresh pool containing every certificate parsed from the PEM in `opts.CA`; parse failures are ignored (an unusable PEM gives an empty pool and every handshake fails verification) |
      | Server name | not set by the syncer; gorilla sets it to the URL host with the `:port` suffix cut off (`gorilla/websocket@v1.5.1/client.go:125-130,348-351`). For an IPv6 literal the square brackets are kept (`[::1]:9109` → `[::1]`). After the handshake gorilla additionally calls `tlsConn.VerifyHostname(ServerName)` (`gorilla/websocket@v1.5.1/tls_handshake.go:8-18`) |
      | Client certificate | if `len(opts.ClientCert) != 0`: `tls.X509KeyPair(ClientCert, ClientKey)`; a parse error is returned from `Run` as-is |
      | ALPN | `NextProtos` is left empty by the syncer and gorilla does not fill it in, so the ClientHello carries no ALPN extension (`gorilla/websocket@v1.5.1/client.go:345-366`) |

   When TLS is in use (`wss`) but no TLS config was supplied (empty `CA`), gorilla uses `{MinVersion: TLS 1.2}` with the system root store. The client certificate is configured only inside the CA branch.
4. `wsrpc.Dial(ctx, url, opts...)`; any error is returned (wrapped by the deferred op). `ctx` cancels the dial; there is no handshake timeout of its own. gorilla calls the dial function as `dial(ctx, "tcp", "<host>:<port>")` with the host exactly as it appears in the URL (a host name is not resolved first, so with a SOCKS proxy the proxy resolves it); when `opts.Dial` is nil a zero-value `net.Dialer` is used (no connect timeout, no keep-alive override). For `wss` gorilla performs the TLS handshake itself on the returned connection; a custom dialer is never expected to return a TLS connection (`gorilla/websocket@v1.5.1/client.go:253-286,344-366`). The caller context carries no deadline in either caller, so no socket deadline is installed during the handshake.
5. `defer wsClient.Close()`; `s.rpc = dcrd.New(wsClient)`.

### 5.2 `Syncer.Run` — full sequence

`Run` may be called once per `Syncer`. Deferred actions are listed in §7.3. Steps, in execution order (`chain/sync.go:516-758`):

1. Create the notification context `ntfnCtx` (derived from `context.Background()`, **not** from the caller's context) and the notifier `{syncer, ctx: ntfnCtx, done: make(chan struct{})}`.
2. Connect (§5.1.3).
3. **Network check.** `getcurrentnet` → `uint32`; if it differs from `ChainParams().Net`, return `errors.E("mismatched networks")`.
4. **API version check.** `version`; read `dcrdjsonrpcapi`; if `!semverCompatible(requiredAPIVersion, v)` return the error `advertised API version <v> incompatible with required version 8.3.0` (§5.3).
5. `wallet.SetNetworkBackend(s)`; deferred `SetNetworkBackend(nil)`. From here on other wallet code can issue calls through the syncer.
6. Read `MainChainTip` and `RescanPoint` (error → return). Log `Headers synced through block <hash> height <h>`. If a rescan point exists, read its header (error → return) and log `Transactions synced through block <header.PrevBlock> height <header.Height-1>`; otherwise log the same line with the tip hash and height.
7. `waitRPCSync(ctx, tipHeight)` (§5.4).
8. Derive a cancellable context and an errgroup context from it; all later calls use the errgroup context.
9. If `wallet.VotingEnabled()`: call `notifywinningtickets` (error → return); log `Wallet voting enabled: vote bits = <%#04x of Bits>, extended vote bits = <hex of ExtendedBits>` and `Please ensure your wallet remains unlocked so it may vote`.
10. **Missing cfilters.** `fetchMissingCfiltersStart()`; start `wallet.FetchMissingCFiltersWithProgress(ctx, s, progress)` on a goroutine with a channel of capacity 1; for each item: `Err != nil` → return it; else `fetchMissingCfiltersProgress(BlockHeightStart, BlockHeightEnd)`. After the channel closes: `fetchMissingCfiltersFinished()`. (The wallet function requests `getcfilterv2` for up to 2000 main-chain blocks at a time through `Syncer.CFiltersV2`, `wallet/wallet.go:1291-1418`.)
11. `getHeaders(ctx)` (§5.5); error → return. On success the wallet is marked synced (`Callbacks.Synced(true)`).
12. Register `defer s.unsynced()`.
13. `notifyblocks` (error → return).
14. `rebroadcastwinners` (error → return).
15. `wallet.PublishUnminedTransactions(ctx, s)`: one `sendrawtransaction` per unmined, published transaction in dependency order; an error is only logged: `Could not publish one or more unmined transactions: <err>` (warning).
16. `GetMempoolTSpends` (error → return); `wallet.AddTSpend(tx)` for each, ignoring its error; trace log `TSpends in mempool: <n>`.
17. `notifytspend` (error → return).
18. Register the notifier-drain defer (§7.3, order 2).
19. Start goroutine A on the errgroup: `wallet.Run(walletCtx)` where `walletCtx` derives from `context.Background()` (runs the mixing client when mixing is enabled; returns nil immediately otherwise).
20. If `wallet.MixingEnabled()`: `notifymixmessages` (error → return).
21. Log `Blockchain sync completed, wallet ready for general usage.`
22. Start goroutine B on the errgroup: wait for either the errgroup context to be done (→ `ctx.Err()`) or `wsClient.Done()` (→ `wsClient.Err()`); then cancel `walletCtx`; return that error.
23. Return `g.Wait()` (first non-nil error among A and B).

`wallet.Run` (step 19) constructs a **new** mixing client and stores it in the wallet on every call (`wallet/wallet.go:5538-5552`), i.e. once per successful connection; the previous connection's client has already returned because its `walletCtx` was cancelled. With mixing disabled goroutine A returns nil at once and the errgroup then depends on goroutine B alone.

Subscription ordering that a port must keep: `notifywinningtickets` precedes header sync; `loadtxfilter` (inside `getHeaders`) precedes `notifyblocks`; `notifyblocks` precedes `rebroadcastwinners`; `notifytspend` follows the mempool tspend load; `notifymixmessages` follows the start of `wallet.Run`.

### 5.3 API version compatibility (`chain/semver.go:15-26`)

`semverCompatible(required, actual)`:

1. `required.Major != actual.Major` → false.
2. `required.Minor > actual.Minor` → false.
3. `required.Minor == actual.Minor && required.Patch > actual.Patch` → false.
4. Otherwise true.

With `required = 8.3.0`: any `8.m.p` with `m > 3`, or `8.3.p` for any `p`, is accepted; `8.2.x`, `7.x`, `9.x` are rejected.

### 5.4 `waitRPCSync(ctx, minHeight)` (`chain/sync.go:470-503`)

`minHeight` starts as the wallet's main-chain tip height. Loop:

1. `getblockchaininfo`; error → return it.
2. If `info.headers > minHeight` then `minHeight = info.headers`.
3. If `info.blocks >= minHeight` and (network is simnet **or** `initialblockdownload == false`) → return nil.
4. Log (info) `Waiting for dcrd instance to catch up to minimum block height <minHeight> (<blocks> blocks, <headers> headers, chain synced=<!initialblockdownload>)`.
5. Sleep 1 s if `minHeight - info.blocks < 100`, else 10 s; context cancellation during the sleep returns the context error.

### 5.5 `getHeaders` — header catch-up, discovery, filter load, rescan (`chain/sync.go:219-398`)

1. `locators = wallet.BlockLocators(ctx, nil)`; error → return. (The wallet returns the tip hash twice followed by the usual back-off list; see Q14.)
2. `startedSynced = walletSynced.Load()`.
3. `fetchHeadersStart()`.
4. Loop:
   1. If the context is done → return its error.
   2. If `!startedSynced`: `getblockchaininfo` (error → return); `targetSyncHeight = int32(info.headers)`.
   3. `headers = getheaders(locators, zero stop hash)`; error → return.
   4. If `len(headers) == 0` → leave the loop.
   5. For every header **concurrently** (one task per header, no concurrency limit; up to 2000 simultaneous requests): compute the block hash; `getcfilterv2`; `validate.CFilterV2HeaderCommitment(net, header, filter, proofIndex, proof)`; build `BlockNode(header, hash, filter, RelevantTxs = nil)`; if `wallet.BadCheckpoint(net, hash, height)` mark the node invalid. Wait for all tasks; the first error is returned (remaining tasks still run to completion).
   6. For every node in header order: skip it if `wallet.BlockInMainChain(hash)`; otherwise, under `sidechainsMu`, `sidechains.AddBlockNode(node)` and count it in `added` when it was not a duplicate.
   7. `fetchHeadersProgress(added, unix time of the last returned header)`.
   8. Log (info) `Fetched <added> new header(s) ending at height <last header height> from <websocket URL>`.
   9. If `added == 0` → leave the loop (dcrd is behind or equal to the wallet; nothing is reorganized out).
   10. Under `sidechainsMu`: `bestChain = wallet.EvaluateBestChain(&sidechains)`; error → return. If empty → `continue` (the locators are **not** refreshed; the next round re-fetches the same headers, adds nothing and exits at step 9).
   11. `wallet.ValidateHeaderChainDifficulties(ctx, bestChain, 0)`; error → return (the returned invalid sub-chain is discarded; no node is marked invalid).
   12. Under `sidechainsMu`: `prevChain = wallet.ChainSwitch(ctx, &sidechains, bestChain)`; error → return.
   13. If `prevChain` is non-empty: log `Reorganize from <old tip hash> to <new tip hash> (total <len(prevChain)> block(s) reorged)` and, under `sidechainsMu`, re-add every removed node to the forest.
   14. Log `Connected block <hash>, height <h>` when exactly one block was attached, else `Connected <n> blocks, new tip block <hash>, height <h>, date <header timestamp>`.
   15. `locators = wallet.BlockLocators(ctx, nil)`; error → return.
5. `fetchHeadersFinished()`.
6. `rescanPoint = wallet.RescanPoint(ctx)`; error → return.
7. If `rescanPoint != nil`:
   1. `discoverAccts = s.discoverAccts.Load()`.
   2. `discoverAddressesStart()`; `wallet.DiscoverActiveAddresses(ctx, s, rescanPoint, discoverAccts, wallet.GapLimit())`; error → return; `discoverAddressesFinished()`.
   3. `s.discoverAccts.Store(false)` — account discovery runs at most once per `Syncer`.
   4. `wallet.LoadActiveDataFilters(ctx, s, reload = true)`; error → return.
   5. `rescanStart()`; read the rescan point's header (error → return); start `wallet.RescanProgressFromHeight(ctx, s, header.Height, progress)` on a goroutine with a channel of capacity 1; for each item: `Err != nil` → return it; else `rescanProgress(ScannedThrough)`. Then `rescanFinished()`.
8. Else: `wallet.LoadActiveDataFilters(ctx, s, reload = true)`; error → return.
9. `s.synced()`.

Header nodes carry `RelevantTxs == nil`, so `ChainSwitch` does not advance the wallet's processed-transactions marker during header catch-up; that is what makes `RescanPoint` non-nil afterwards and forces steps 7.1–7.5 whenever at least one header was attached.

Database transactions: each wallet call above opens its own view/update transaction; `getHeaders` itself holds none. Lock order: `sidechainsMu` is taken before any wallet lock or database transaction and is never held across the `getheaders`/`getcfilterv2` calls, but **is** held across `ChainSwitch`. `ChainSwitch` can itself issue `loadtxfilter` calls (§5.11); in the header catch-up path it does not, because the attached nodes carry no transactions (no outpoints to watch) and `watchHDAddrs(firstWatch = false)` returns immediately while a rescan point exists (`wallet/wallet.go:891-900`). In the notification path (§5.6.2) the calls are made with `sidechainsMu` held.

#### 5.5.1 `validate.CFilterV2HeaderCommitment(net, header, filter, leafIndex, proof)` (`validate/validate.go:69-102`)

Called once per header in `getHeaders` and once per block in `handleBlockConnected`, always with the `proofindex` and `proofhashes` from the `getcfilterv2` reply.

1. If `net` is TestNet3 and `header.Height < 323328`, or `net` is MainNet and `header.Height < 431488` → return nil without looking at the filter or the proof (pre-DCP0005 filters are validated in bulk elsewhere in the wallet). On **every other network, including simnet, the proof is checked from height 0**.
2. `filterHash = filter.Hash()`: all-zero for an empty filter (N = 0), otherwise BLAKE-256 of the serialized filter bytes exactly as received in `data` (varint N followed by the Golomb-coded set) (`gcs/v4@v4.1.1/gcs.go:403-411`).
3. `root = header.StakeRoot` (post-DCP0005 this field is the header commitment root).
4. `VerifyInclusionProof(root, filterHash, leafIndex, proof)` (`blockchain/standalone/v2@v2.3.0/inclusionproof.go:162-207`): fail if `len(proof) > 32` or `leafIndex > 2^len(proof) − 1`; otherwise start with `acc = filterHash` and for each proof hash `p` in order: `acc = BLAKE-256(p ‖ acc)` when the low bit of `leafIndex` is 1, else `BLAKE-256(acc ‖ p)`; then `leafIndex >>= 1`. Succeeds iff `acc == root`. An empty proof therefore requires `leafIndex == 0` and `filterHash == root`.
5. Failure → `errors.E(Op("validate.CFilterV2HeaderCommitment(<block hash>)"), errors.Consensus, "invalid header inclusion proof for cfilterv2")`.

The wallet does not compare `leafIndex` against a fixed commitment position; any index/proof pair that hashes to the root is accepted.

### 5.6 Notification processing

#### 5.6.1 Dispatch (`chain/sync.go:768-808`)

`Notify(method, params)` runs handlers with `ntfnCtx`. Return value semantics: a non-nil return makes wsrpc record it as the client's final error and close the connection.

| Method | Handler | On handler error |
|---|---|---|
| `winningtickets` | `winningTickets` | log at error level: `winningtickets: <err>`; return nil |
| `blockconnected` | `blockConnected` | see below |
| `relevanttxaccepted` | `relevantTxAccepted` | log at error level `relevanttxaccepted: <err>`; return nil |
| `tspend` | `storeTSpend` | log at error level `tspend: <err>`; return nil |
| `mixmessage` | `mixMessage` | log at error level `mixmessage: <err>`; return nil |
| anything else | — | return nil |

The logged value is `errors.E(Op(method), err)` rendered with the daemon's nested-error separator `":: "` (`dcrwallet.go:36-39`).

`blockconnected` error policy, using the notifier's `connectingBlocks` flag (initially false, per connection):

- handler returned nil → `connectingBlocks = true`; return nil.
- handler failed and `connectingBlocks == false` → log (error) `Failed to connect block: blockconnected: <err>`; return nil (the connection survives).
- handler failed and `connectingBlocks == true` → return the wrapped error; the connection is closed and `Run` returns it.

#### 5.6.2 `blockconnected` → `handleBlockConnected(ctx, header, relevantTxs, isProcessingMissingBlock = false)` (`chain/sync.go:825-912`)

1. `blockHash = header.BlockHash()`. If `wallet.BlockInMainChain(blockHash)` → return nil.
2. Under `sidechainsMu`: `prevInMainChain = wallet.BlockInMainChain(header.PrevBlock)`; `prevInSideChain = sidechains.HasSideChainBlock(header.PrevBlock)`. Unlock.
3. If neither:
   - when called for a missing block (`isProcessingMissingBlock`): return the error `broken assumption: received missing block  without its parent in mainchain or sidechain` (two spaces after "block");
   - otherwise log (info) `Received header for block <hash> (height <h>) when parent <prev hash> not in main or side chain. Re-requesting missing headers.` and return `getMissingHeaders(ctx)` (§5.7). The triggering block itself is not processed further here; it is expected to be among the headers that `getMissingHeaders` fetches.
4. `getcfilterv2(blockHash)`; error → return.
5. `validate.CFilterV2HeaderCommitment`; error → return.
6. Lock `sidechainsMu` until return.
7. If `relevantTxs == nil` replace it by an empty non-nil list (a non-nil list tells `ChainSwitch` that the block's wallet transactions are fully known, so the processed-transactions marker may advance).
8. Build `BlockNode(header, hash, filter, relevantTxs)`; mark invalid if `BadCheckpoint`; `sidechains.AddBlockNode(node)`.
9. `bestChain = wallet.EvaluateBestChain(&sidechains)`; error → return.
10. If `bestChain` is non-empty: `prevChain = wallet.ChainSwitch(&sidechains, bestChain)` (error → return); if `prevChain` is non-empty log the `Reorganize from … to … (total N block(s) reorged)` line and re-add the removed nodes to the forest; then for every node of `bestChain` log `Connected block <hash>, height <h>, <n> wallet transaction(s)`.
11. Else log `Observed sidechain or orphan block <hash> (height <h>)`.

Differences from the header catch-up path: no `ValidateHeaderChainDifficulties` call (proof of work and stake difficulty of notified blocks are not checked), and no explicit rescan (the transactions come from dcrd's filter match).

#### 5.6.3 `winningtickets` (`chain/sync.go:817-823`)

Decode (§4.5.2) and call `wallet.VoteOnOwnedTickets(ctx, winners, blockHash, height)`. With voting enabled and `height >= StakeValidationHeight - 1`, that function issues, in order: `getblockchaininfo` once per deployment query (DCP0010 then DCP0012 on mainnet/testnet3; DCP0012 only on simnet) **before** checking ticket ownership; then one `sendrawtransaction` per created vote; then `loadtxfilter false [] [outpoints]` if any outputs must be watched; then possibly further `loadtxfilter` calls for newly watched HD addresses (`wallet/chainntfns.go:698-924`, `deployments/deployments.go:92-158`).

#### 5.6.4 `relevanttxaccepted` (`chain/sync.go:914-923`)

Decode the transaction. If `wallet.ManualTickets()` and the transaction is a ticket purchase (`stake.IsSStx`) → return nil without recording. Otherwise `wallet.AddTransaction(ctx, tx, nil)` (unmined).

#### 5.6.5 `tspend` (`chain/sync.go:925-931`)

Decode and `wallet.AddTSpend(*tx)`; a tspend already cached produces the error `tspend already cached`, logged as `tspend: tspend already cached` at error level.

#### 5.6.6 `mixmessage` (`chain/sync.go:933-966`)

1. If `!wallet.MixingEnabled()` → debug log `Ignoring mixmessage notification: mixing disabled`; return nil.
2. Decode (§4.5.5); error → return.
3. Under `blake256HasherMu`: `msg.WriteHash(hasher)` (computes and caches the message hash with the syncer's shared BLAKE-256 state).
4. `err = wallet.AcceptMixMessageBySource(msg, mixpool.ZeroSource)`.
5. If `err` is (wraps) `*mixpool.MissingOwnPRError`:
   - if the message is not a `MsgMixKeyExchange`, or its `Run != 0` → return `err`;
   - else `pr = getmixmessage(e.MissingPR)`; if that succeeded: hash `pr` under the mutex and return `wallet.AcceptMixMessageBySource(pr, ZeroSource)`; if it failed return that error. The original key-exchange message is **not** re-submitted (the mixpool has kept it as an orphan).
6. Otherwise return `err`.

`msg.WriteHash(hasher)` (e.g. `wire@v1.8.0/msgmixpairreq.go:256-265`; every mix message type has the same shape): reset the hasher, write the 64-byte signature, then the remainder of the message serialized at protocol version 10 without the signature, and store the 32-byte BLAKE-256 sum in the message's private hash field, which `Hash()` returns afterwards. The mixpool identifies messages by that hash (`mixing@v0.7.5/mixpool/mixpool.go`, `AcceptMessage` calls `msg.Hash()`), so it must be computed before acceptance. The syncer keeps one hasher per `Syncer` and serializes its use with `blake256HasherMu`; within `chain/` the only users are the two call sites in `mixMessage`, and notification handlers never run concurrently, so the mutex is uncontended. A hasher whose `Size()` is not 32 makes `WriteHash` panic; not reachable.

### 5.7 `getMissingHeaders` (`chain/sync.go:406-452`)

Runs on the notification worker, with `ntfnCtx`.

1. `locators = wallet.BlockLocators(ctx, nil)`; `headers = getheaders(locators, zero)`; errors → return.
2. `len(headers) == 0` → return nil.
3. `len(headers) == 2000` (`wire.MaxBlockHeadersPerMsg`) → log (warning) `Too many (2000) missing headers. Performing full header sync.` and return `getHeaders(ctx)` — the full §5.5 procedure including callbacks, discovery, filter reload and rescan, but with `startedSynced == true` (no `targetSyncHeight` updates).
4. Otherwise, for each header in order:
   1. `rescan([blockHash])`, collecting all returned transactions (the list stays nil when dcrd reports no match); error → return.
   2. `handleBlockConnected(ctx, header, txs, isProcessingMissingBlock = true)`; error → return.

Blocks are rescanned one at a time because each connected block can extend the server-side filter (new addresses/outpoints) that the next block must be matched against.

### 5.8 Method wrapper behaviour (`rpc/client/dcrd/calls.go`)

| Wrapper | Algorithm | Errors |
|---|---|---|
| `UsedAddresses` | Split the addresses into consecutive chunks of 400 000 (a multiple of 8, compile-time asserted); for each chunk marshal the address strings and call `existsaddresses`; append the returned bytes. Zero addresses → no call, nil result. | marshal or call failure → op `dcrd.UsedAddresses` around op `existsaddresses`; hex failure adds kind `Encoding` |
| `ExistsLiveTickets` | one call with all hashes | op `dcrd.ExistsLiveTickets` around op `existslivetickets` |
| `GetMempoolTSpends` | `getrawmempool false "tspend"`, then sequential `getrawtransaction` per returned hash string | op `dcrd.GetMempoolTSpends`; aborts on the first failure |
| `publishTransaction` | serialize to lower-case hex; `sendrawtransaction`; if the error is (wraps) a `*wsrpc.Error` with `Code == -40` → success | serialization → op + kind `Encoding`; other → op |
| `PublishTransactions` | call `publishTransaction` for every tx in argument order, never concurrently; remember the first error; keep going | op `dcrd.PublishTransactions` |
| `PublishMixMessages` | `sendrawmixmessage` for every message in order; remember the first error; keep going. There is no duplicate-message exemption. | serialization (`BtcEncode`) failure → op `dcrd.PublishMixMessages` + kind `Encoding`, and no call is made for that message; a call failure is returned by the inner helper **without** an op and wrapped once by the outer function (contrast `publishTransaction`, which wraps with the op itself and is then wrapped again by `PublishTransactions`; the `errors` package collapses the repeated op) |
| `Blocks` | one task per hash, unbounded concurrency; results in input order | first error, op `dcrd.Blocks` |
| `CFiltersV2` | one task per hash, unbounded concurrency; results in input order; on error returns the partially filled slice **and** the first error | per-task op `dcrd.CFiltersV2(<hash>)` |
| `CFilterV2` | single call | op `dcrd.CFilterV2(<hash>)` |
| `Headers` | single call | op `dcrd.Headers` |
| `LoadTxFilter` | single call | op `dcrd.LoadTxFilter` |
| `Rescan` | single call; then per reported block decode and invoke `save`; stops at the first failure | call → op `dcrd.Rescan`; decode → op + kind `Encoding`; `save` errors returned as-is |
| `StakeDifficulty` | single call; `dcrutil.NewAmount(float)` (round-half-away of `f * 1e8`; NaN/±Inf → `invalid coin amount`) | op `dcrd.StakeDifficulty` |
| `GetBlockchainInfo` | single call | op `dcrd.GetBlockchainInfo` |
| `GetTxOut` | single call | op `dcrd.GetTxOut` |
| `GetConfirmationHeight` | single call; returns `-1` with the error | op `dcrd.GetRawTransaction` (sic) |
| `MixMessage` | single call then type switch and decode | call → op `dcrd.MixMessage`; unknown type → op + `unrecognized mixing message type`; decode error unwrapped |
| `MempoolCount` | single call; `kind` is passed through unvalidated. The doc comment lists `"all"`, `"regular"`, `"tickets"`, `"votes"`, `"revocations"`; the only value used in the tree is `"tickets"` (`GetMempoolTSpends` separately uses `"tspend"`, which the comment does not list) | op `dcrd.MempoolCount` |
| `getRawTransaction` | single call; returns the (possibly partially decoded) transaction together with the error, unwrapped; the only caller wraps it | none of its own |
| `ExistsLiveTicket` | single call | op `dcrd.ExistsLiveTicket` |
| `MixPairRequests` | single call; result decoded by `messageArray[MsgMixPairReq]` at pver 10 (`null` → nil slice) | call or decode failure → op `dcrd.MixPairRequests` (element errors carry kind `Encoding`) |
| `New`, `(*RPC).String` | no I/O | — |

Per-wrapper partial-result behaviour on failure: `Blocks` returns a nil slice (each task allocates its own `MsgBlock` before calling, so a failed element may be partially decoded but is never exposed); `CFiltersV2` returns the slice with zero-valued entries (`Filter == nil`) at every index whose request failed; `GetMempoolTSpends` returns nil even when earlier transactions decoded; `UsedAddresses` returns nil even when earlier batches succeeded; `MixMessage` returns the partially decoded message together with a decode error.

### 5.9 Reconnect loop (`dcrwallet.go:569-599`)

```
loop:
    syncer = chain.NewSyncer(w, rpcOptions)         # fresh Syncer, fresh sidechain forest, discoverAccts = !w.Locked()
    err = syncer.Run(ctx)
    if err != nil:
        if errors.Is(err, context.Canceled) or ctx.Err() != nil:
            SYNC info  "RPC synchronization stopped"
            return
        SYNC error "RPC synchronization stopped: <err>"
        wait 5 s (return immediately if ctx is cancelled)
    # err == nil: loop again immediately (not reachable in practice, see Q13)
```

There is no backoff growth, no jitter and no attempt limit. Every failure — dial error, TLS error, bad credentials, wrong network, incompatible API version, any error during sync, a `blockconnected` failure after the first success, loss of the connection — takes the same path. Each attempt redoes the complete `Run` sequence, including `loadtxfilter` with `reload = true`.

### 5.10 wsrpc client internals

#### 5.10.1 `Dial` (`rpc.go:197-249`)

Defaults `pingPeriod = 60 s`, `pongWait = 10·pingPeriod/9`; options applied in order (dcrwallet's `WithoutPongDeadline` zeroes `pongWait`). After the handshake: a pong handler is installed (a no-op when `pongWait == 0`), a 60 s ticker is created, and two goroutines start: the reader `in` and the writer `out`.

#### 5.10.2 `Call` / `Go` (`rpc.go:430-517`)

1. If the context is already done: the call completes immediately with the context error; **no id is consumed**.
2. `id = ++seq` (skipping 0).
3. Under the call mutex: if the client has already failed (call map is nil) complete with the client's final error; else register the call under `id`.
4. Hand the request to the writer over an unbuffered channel, racing against context cancellation and client failure. If cancellation or failure wins, the call is unregistered and completed with that error (nothing is sent).
5. Wait for completion or context cancellation.
6. Whatever happened, if the context is done when `Call` returns, the returned error is the context error (it overrides a successfully received result).

A call abandoned after its request was written stays registered; its late response is consumed silently. Requests from concurrent callers are written in the order the writer receives them; responses are matched by id and may arrive in any order.

#### 5.10.3 Writer `out` (`rpc.go:294-343`)

Loop: if a ping tick is pending send the ping first; then wait for the first of client failure (exit), ping tick (send ping), or a request (set a 10 s write deadline, write the JSON text frame). Any write error — including a JSON marshalling error of the parameters — becomes the client's final error and the loop exits. On exit the socket is closed.

#### 5.10.4 Reader `in` (`rpc.go:354-425`)

Loop: read one message as JSON (§4.1.5). Read or decode error → final error, exit. Notifications are queued by spawning one goroutine per notification, chained by a pair of channels so that goroutine *k+1* starts its `Notify` only after goroutine *k* finished; the reader itself never waits for `Notify`, so responses to calls issued from inside a notification handler are delivered while the handler runs. The queue is unbounded. A queued notification that observes the client's failure before its turn is dropped. If `Notify` returns an error it becomes the final error and the socket is closed.

The drop is not deterministic. Each queued goroutine waits in a two-way `select` on "client failed" and "my predecessor finished" (`rpc.go:390-394`). A goroutine whose predecessor finishes after the client has failed finds both channels ready, and Go's `select` then picks one pseudo-randomly: the notification is either dropped or delivered. A delivered one closes its successor's gate when it returns, so the same coin toss repeats down the queue; a dropped one returns without closing the gate, and from then on every later goroutine can only take the "client failed" branch. Consequently, after a connection failure (including the failure caused by a `blockconnected` handler error, §5.6.1) an arbitrary-length prefix of the already queued notifications may still be handed to `Notify`, running with `ntfnCtx` possibly already cancelled and with `Syncer.rpc` calls failing immediately with the client's final error.

#### 5.10.5 `setErr` (`rpc.go:270-292`)

First caller wins: stores the error, closes the done channel, calls `Close()` on the notifier if it implements `io.Closer` (for `chain.notifier` this closes `notifier.done`), then completes every registered call with the error and sets the call map to nil. Later callers are ignored.

#### 5.10.6 `Close` (`rpc.go:258-268`)

`Close` does not call `setErr`. It writes the close frame (10 s deadline) and closes the socket; the reader then fails with a "use of closed network connection"-class error (or the writer fails first), and **that** error becomes the final error. `Close` returns the close-frame write error if there was one, else the socket-close error. `Run` can call it twice (deferred action 2 and deferred action 6 of §7.3, and the writer goroutine closes the socket a third time on exit); the repeated calls fail harmlessly and their errors are discarded. `Close` is safe to call concurrently with reads and writes.

### 5.11 RPC traffic generated by wallet operations in RPC mode

For orientation (the algorithms belong to the wallet specs); this is what dcrd observes.

| Wallet operation | Calls, in order | Citation |
|---|---|---|
| `LoadActiveDataFilters(reload = true)` | `loadtxfilter true [] []`; then per HD account (BIP0044 accounts and imported-xpub accounts, visited in Go map iteration order) and branch (external, then internal), address chunks of 256 child indexes as `loadtxfilter false [addrs] []`, up to `NumCPU` chunks in flight per branch; then imported addresses in chunks of 256; then unspent outpoints plus output 0 (stake tree) of every unspent ticket in chunks of 64 as `loadtxfilter false [] [outpoints]` | `wallet/wallet.go:1003-1038,1101-1200` |
| `rescan` (wallet) | one `rescan` per batch of at most 2000 consecutive main-chain block hashes (genesis excluded), sequentially | `wallet/rescan.go:21,212-356` |
| `DiscoverActiveAddresses` with a backend that has `UsedAddresses` | binary searches issuing `existsaddresses` with `gapLimit` addresses per request, accounts and branches in parallel | `wallet/discovery.go:545-714,765-769,830-834` |
| `fetchMissingCFilters` | `getcfilterv2` × up to 2000 in parallel per round | `wallet/wallet.go:1291-1418` |
| `ChainSwitch` | after the DB update: `loadtxfilter false [addrs] []` for newly watched HD addresses (skipped entirely while a rescan point exists, `wallet/wallet.go:891-900`; the same holds for the HD-address step after publishing in the next row and after voting in §5.6.3), then `loadtxfilter false [] [outpoints]` for outputs created by the attached blocks' wallet transactions (failure only logged) | `wallet/chainntfns.go:293-305` |
| transaction publishing | `sendrawtransaction`, then `loadtxfilter false [addrs] []` for newly watched HD addresses, then `loadtxfilter false [] [outpoints]` | `wallet/createtx.go:331-365` |
| address generation | `loadtxfilter false [addrs] []` | `wallet/addresses.go:587-600,893-908` |

---

## 6. Constants, limits and defaults

| Name | Value | Source | What depends on it |
|---|---|---|---|
| `requiredAPIVersion` | 8.3.0 | `chain/sync.go:33` | Minimum dcrd JSON-RPC API; dcrd master advertises exactly 8.3.0 (`dcroxide:crates/dcroxide-rpc/src/handlers.rs:3657-3660`) |
| Version reply key | `dcrdjsonrpcapi` | `chain/sync.go:598` | API check |
| Websocket path | `/ws` | `chain/sync.go:542-544` | URL |
| JSON-RPC version string sent | `"2.0"` | `wsrpc/v2@v2.3.8/rpc.go:487` | request envelope |
| Default dcrd RPC port | mainnet `9109`, testnet3 `19109`, simnet `19556` | `internal/netparams/params.go:20-45` | `RPCOptions.DefaultPort`, default `--rpcconnect` |
| Default `--rpcconnect` | `localhost:<port>` | `config.go:784-786` | daemon |
| Default CA file name | `dcrd.cert` in the wallet app-data dir (fallback: dcrd's `rpc.cert`) | `config.go:42,73,817-841` | TLS trust |
| Default client cert / key | `dcrd-client.cert`, `dcrd-client.key` in the wallet app-data dir | `config.go:78-79,463-468` | `dcrdauthtype=clientcert` |
| Auth types | `basic` (default), `clientcert` | `config.go:37-38,50` | `--dcrdauthtype` |
| TLS minimum version | 1.2 | `chain/sync.go:559` | handshake |
| TLS 1.2 cipher suites | six ECDHE AEAD suites (§5.1.3) | `chain/sync.go:560-567` | handshake |
| Ping period | 60 s | `wsrpc rpc.go:201` | keep-alive pings |
| Pong deadline | none (`WithoutPongDeadline`) | `chain/sync.go:551` | a silent peer is never timed out by reads |
| `writeWait` | 10 s | `wsrpc rpc.go:33` | request, ping and close-frame write deadline |
| Pong write deadline (reply to a server ping) | 1 s | `gorilla/websocket@v1.5.1/conn.go:35` | gorilla default ping handler |
| Websocket write buffer | 4096 bytes | `gorilla/websocket@v1.5.1/conn.go:38` | fragmentation threshold of outgoing requests (§4.1.3) |
| Websocket read limit | none | `wsrpc rpc.go:206-224` (never calls `SetReadLimit`) | incoming message size is unbounded |
| Handshake timeout | none | `wsrpc rpc.go:206-210` | only the caller context bounds the dial |
| `Run` option slice capacity | 5 | `chain/sync.go:546` | upper bound on wsrpc options (auth, notifier, no-pong-deadline, dial, TLS) |
| Progress channel capacity | 1 | `chain/sync.go:378,658` | missing-cfilter and rescan progress channels |
| `defaultCircuitLimit` | 32 | `config.go:61` | SOCKS pool size for the dcrd dial under `--torisolation` |
| Reconnect delay | 5 s, fixed | `dcrwallet.go:596` | `rpcSyncLoop` |
| dcrd-sync poll interval | 10 s; 1 s when fewer than 100 blocks remain | `chain/sync.go:492-495` | `waitRPCSync` |
| `wire.MaxBlockHeadersPerMsg` | 2000 | `wire@v1.8.0/msgheaders.go:15` | threshold for falling back to a full header sync in `getMissingHeaders` |
| `maxBlocksPerRescan` | 2000 | `wallet/rescan.go:21` | block hashes per `rescan` request |
| Missing-cfilter span | 2000 | `wallet/wallet.go:1310` | `getcfilterv2` requests per round |
| `existsaddresses` batch | 400 000 addresses | `rpc/client/dcrd/calls.go:104` | keeps a request under dcrd's 16 MB websocket read limit |
| Outpoint filter batch | 64 | `wallet/wallet.go:1112` | `loadtxfilter` outpoint chunk |
| Address filter batch | 256 | `wallet/wallet.go:1013,1138` | `loadtxfilter` address chunk |
| `codeDuplicateTx` | −40 | `rpc/client/dcrd/codes.go:8` | `sendrawtransaction` duplicate is success |
| cfilter parameters | B = 19, M = 784931 | `gcs/v4@v4.1.1/blockcf2/blockcf.go:33-38` | `getcfilterv2` decoding |
| `wire.MixVersion` | 10 | `wire@v1.8.0/protocol.go:56` | mix message (de)serialization pver |
| DCP0005 activation heights | mainnet 431488, testnet3 323328 | `validate/validate.go:18-26` | cfilter commitment proofs are not checked below these heights |
| Testnet3 checkpoint | height 962928, hash `0000004fd1b267fd39111d456ff557137824538e6f6776168600e56002e23b93` | `wallet/checkpoints.go:22-33` | nodes violating it are marked invalid |
| Mixpool source id | `mixpool.ZeroSource` (0) | `chain/sync.go:947,960` | source attribution of dcrd-relayed mix messages |
| Logger subsystem | `SYNC` | `log.go:35`, `internal/loggers/loggers.go:50` | all `chain` package log lines and the two `RPC synchronization stopped` lines |
| Error separator (daemon) | `":: "` | `dcrwallet.go:38` | rendering of nested errors in log lines |

---

## 7. Concurrency and lifecycle

### 7.1 Goroutines

| Goroutine | Started by | Ends when |
|---|---|---|
| wsrpc reader (`in`) | `wsrpc.Dial` | read/decode error or unknown response id |
| wsrpc writer (`out`) | `wsrpc.Dial` | client failure or write error; closes the socket on exit |
| one per queued notification | reader | after its `Notify` returns, or when the client has failed before its turn |
| missing-cfilter fetcher | `Run` step 10 | wallet function returns; closes the progress channel |
| per-header cfilter task (≤ 2000) | `getHeaders` step 4.5 | its call returns |
| rescan driver | `getHeaders` step 7.5 | wallet function returns; closes the progress channel |
| errgroup A: `wallet.Run(walletCtx)` | `Run` step 19 | `walletCtx` cancelled (or immediately when mixing is disabled) |
| errgroup B: connection watcher | `Run` step 22 | errgroup context done or client failed |
| `WrapContext` helper | each `WrapContext` call | `Syncer.done` closed or the child context cancelled |
| per-hash task | `dcrd.RPC.Blocks`, `dcrd.RPC.CFiltersV2` | its call returns |

### 7.2 Shared state

| State | Protection |
|---|---|
| `Syncer.sidechains` | `sidechainsMu`. Held across `EvaluateBestChain` + `ChainSwitch` (which performs a DB update and RPC round trips). Acquired before any wallet-internal lock. |
| `Syncer.blake256Hasher` | `blake256HasherMu` |
| `walletSynced`, `targetSyncHeight`, `discoverAccts`, `notifier.closed` | atomics |
| `Syncer.cb`, `Syncer.rpc`, `Syncer.notifier` | unsynchronized; written before/at the start of `Run` |
| `notifier.connectingBlocks` | touched only by `Notify`, which is never concurrent with itself |
| wsrpc call map | `callMu` |
| wsrpc final error | `errMu` plus the done channel |
| request id counter | atomic |

### 7.3 Deferred actions of `Run`, in the order they execute on return

Only the defers registered before the return point run.

| Order | Registered at | Action |
|---:|---|---|
| 1 | `sync.go:729` | cancel `walletCtx` |
| 2 | `sync.go:713-724` | cancel `ntfnCtx`; if the errgroup context is already done, `wsClient.Close()`; then **block until `notifier.done` is closed** |
| 3 | `sync.go:674` | `s.unsynced()` → `Callbacks.Synced(false)` if it was synced |
| 4 | `sync.go:638-643` | cancel the run context; `g.Wait()`; if `Run` was returning nil, adopt the errgroup error |
| 5 | `sync.go:611` | `wallet.SetNetworkBackend(nil)` |
| 6 | `sync.go:583` | `wsClient.Close()` (errors ignored) |
| 7 | `sync.go:531` | cancel `ntfnCtx` |
| 8 | `sync.go:524-526` | `close(s.done)` → every `WrapContext` child is cancelled |
| 9 | `sync.go:517-522` | wrap a non-nil error as `errors.E("rpcsyncer.Run", err)` |

### 7.4 Contexts

| Context | Parent | Cancelled by | Used for |
|---|---|---|---|
| caller `ctx` | — | process shutdown / gRPC stream end | dial, steps 3–7 |
| run context (errgroup) | caller `ctx` | caller, deferred cancel, or either errgroup goroutine returning an error | steps 9–20 and all wallet calls on the main path |
| `ntfnCtx` | background | `Run`'s defers only | every notification handler, including the header re-sync it may trigger |
| `walletCtx` | background | watcher goroutine B, or `Run`'s defer | `wallet.Run` (mixing client), so that it is stopped only after the connection watcher has observed the shutdown |

### 7.5 Ordering guarantees observable by callers

1. Notifications are handled one at a time in the order dcrd sent them. Responses to calls are not ordered relative to notifications.
2. Notification handlers may run concurrently with the main `Run` goroutine during initial sync (`winningtickets` after step 9, `relevanttxaccepted` once the filter is loaded). `blockconnected` cannot arrive before step 13, i.e. after `getHeaders` has returned.
3. `Callbacks` order on a first sync: `FetchMissingCFiltersStarted`, `…Progress`*, `…Finished`, `FetchHeadersStarted`, `FetchHeadersProgress`*, `FetchHeadersFinished`, then (if a rescan point exists) `DiscoverAddressesStarted`, `DiscoverAddressesFinished`, `RescanStarted`, `RescanProgress`*, `RescanFinished`, then `Synced(true)`. A later full header re-sync from the notification worker repeats the header/discovery/rescan callbacks without a second `Synced(true)`.
4. `Synced(true)` precedes `notifyblocks`, the unmined-transaction rebroadcast and the `Blockchain sync completed` log line.
5. On exit (defer order of §7.3): when `Run` got past step 18, the socket is closed first (deferred action 2 calls `wsClient.Close()` because the errgroup context is done, unless the connection had already failed; a failure at step 20 is the exception, see Q16) and `notifier.done` awaited, then `Synced(false)` → backend unset → `WrapContext` children cancelled. When `Run` fails between steps 12 and 18: `Synced(false)` → backend unset → socket closed → `WrapContext` children cancelled. Before step 12 there is no `Synced(false)`.
6. The wallet's network backend is non-nil exactly between step 5 of `Run` and deferred action 5; between reconnect attempts (≥ 5 s) the wallet has no backend and backend-dependent operations fail with kind `NoPeers` (`wallet/network.go:61-71`).

7. `Syncer.RPC()` is non-nil for as long as the syncer is the wallet's registered backend (it is assigned right after the dial, before `SetNetworkBackend`), so the RPC servers' `chainSyncer.RPC().Call(...)` never dereferences nil. A caller that obtained the backend just before `Run` returned keeps a usable `*Syncer` whose calls fail immediately with the dead client's final error (§5.10.2 step 3); its `Synced()` then reports `(false, <last targetSyncHeight>)`.

### 7.6 Shutdown

Process shutdown cancels the caller context: watcher B returns `context.Canceled`, cancels `walletCtx`, `wallet.Run` returns, `g.Wait()` returns, the errgroup context is done, deferred action 2 closes the websocket (close frame 1000) which fails the reader, which closes `notifier.done`, and the remaining defers run. `Run` returns `rpcsyncer.Run: context canceled`; `rpcSyncLoop` logs `RPC synchronization stopped` (info) and returns.

---

## 8. Errors

### 8.1 Produced in scope

| Origin | Kind | Text (after any op prefix) |
|---|---|---|
| `normalizeAddress` failure | `Invalid` | Go `net.SplitHostPort` error text for the original address |
| `tls.X509KeyPair` | — | Go `crypto/tls` error text |
| `wsrpc.Dial` | — | dial/TLS/handshake error; `websocket: bad handshake` for any non-101 reply |
| network check | — | `mismatched networks` |
| API check | — | `advertised API version <a.b.c> incompatible with required version 8.3.0` |
| dcrd JSON-RPC error | — | `*wsrpc.Error{Code, Message, Data}`; text is `Message` |
| wsrpc | — | `wsrpc: unknown response ID`; read/write errors of the websocket |
| `unhex`, `transactions`, `messageArray` | `Encoding` | `not a string`, or the wire/hex error |
| `cfilterV2` | `Encoding` | `cfilterv2 is not an even hex string`, or the hex error; gcs errors unwrapped |
| `hash` | `Encoding` | `not a 32-byte hash hex string`, or the hex error |
| `hashes`, `hashesContiguous`, `headers` | `Encoding` (element errors); JSON syntax errors unwrapped | |
| `unmarshalArray` | `Encoding` | Go `encoding/json` error text |
| `MixMessage` (notification) | — | `unrecognized mixing message command string` |
| `dcrd.RPC.MixMessage` | — | `unrecognized mixing message type` |
| `dcrd.RPC.StakeDifficulty` | — | `invalid coin amount` (NaN/Inf only) |
| `handleBlockConnected` | — | `broken assumption: received missing block  without its parent in mainchain or sidechain` |
| `validate.CFilterV2HeaderCommitment` | `Consensus` | op `validate.CFilterV2HeaderCommitment(<block hash>)`, `invalid header inclusion proof for cfilterv2` |
| `publishTransaction`, `publishMixMessage` serialization | `Encoding` | the wire encoder's error text |
| `exists` (bitset hex) | `Encoding` | Go `encoding/hex` error text, under op `<method>` |
| `Rescan` reply decoding | `Encoding` | `chainhash.NewHashFromStr` or wire/hex error text, under op `dcrd.Rescan` |
| peer closed the websocket | — | gorilla `*CloseError`: `websocket: close <code>[ (<name>)][: <reason>]`, where the name is `normal` (1000), `going away` (1001), `protocol error` (1002), `unsupported data` (1003), `no status` (1005), `abnormal closure` (1006), `invalid payload data` (1007), `policy violation` (1008), `message too big` (1009), `mandatory extension missing` (1010), `internal server error` (1011), `TLS handshake error` (1015), and absent for other codes (`gorilla/websocket@v1.5.1/conn.go:113-145`) |
| TCP connection dropped without a close frame | — | `websocket: close 1006 (abnormal closure): unexpected EOF` (`conn.go:178,377-381`) — the text an operator sees after dcrd is killed or restarted |
| empty or truncated JSON message | — | `unexpected EOF` (`gorilla/websocket@v1.5.1/json.go:54-58`) |
| handshake reply offered compression without both no-context-takeover parameters | — | `websocket: invalid compression negotiation` |
| write after the local close frame | — | `websocket: close sent` (`conn.go:87`) |
| JSON-RPC result missing from a response whose caller wanted one | — | `EOF` (Go `json.Decoder` on empty input, `wsrpc rpc.go:419-421`) |

Operation prefixes are listed in §5.8; `Run` adds `rpcsyncer.Run`. Rendering follows the `errors` package rules: an op and a kind are joined by `": "`, nested `*errors.Error` values by the separator (`":: "` in the daemon), and an identical op/kind is collapsed (`errors/errors.go`).

### 8.2 How they surface

| Surface | Behaviour | Citation |
|---|---|---|
| Daemon log | `SYNC` error `RPC synchronization stopped: rpcsyncer.Run: <…>`, then a retry after 5 s. The process never exits because of an RPC sync failure. | `dcrwallet.go:586-598` |
| gRPC `RpcSync` | `codes.Canceled`, message `Wallet synchronization canceled: <err>` when the stream context is done; otherwise `codes.Unknown`, `Wallet synchronization stopped: <err>` | `internal/rpc/rpcserver/server.go:2973-2979` |
| JSON-RPC server | `convertError` uses a **type switch**, not unwrapping: an error that is itself a `*wsrpc.Error` (raw passthrough, `getblockheader`, `getblock`, `gettxout`, `getinfo`, `getpeerinfo`, `help`, `version` paths) is returned to the client with dcrd's code and message unchanged; a `*wsrpc.Error` wrapped by any op becomes code −4 (`ErrRPCWallet`) with the full rendered text, unless a kind maps it elsewhere (`Encoding` → −8, `NoPeers` → −9, `Bug` → internal error, …) | `internal/rpc/jsonrpc/errors.go:16-49` |
| JSON-RPC passthrough with a cancelled request context | code −1 (`ErrRPCMisc`), message = context error text | `internal/rpc/jsonrpc/methods.go:247-254` |
| JSON-RPC passthrough without an RPC syncer | no backend: code −9 `disconnected from consensus RPC`; SPV backend: code −9 `RPC passthrough requires dcrd RPC synchronization` | `methods.go:233-241`, `errors.go:72-75` |
| Notification handlers | logged at error level on `SYNC`; never returned to any client, except the `blockconnected` rule in §5.6.1 which terminates the connection | `chain/sync.go:773-807` |
| No backend set | `wallet.NetworkBackend()` → kind `NoPeers` (`Decred network is unreachable`); JSON-RPC −9, gRPC `Unavailable` | `wallet/network.go:61-71` |
| JSON-RPC `walletinfo` | `ping` through the syncer; a failure is logged as a warning `Ping failed on connected daemon client: <err>` and reported as `daemonconnected = false`, not as an RPC error; a cancelled request context is returned as the error | `internal/rpc/jsonrpc/methods.go:5479-5489` |

### 8.3 Log lines

Every log statement in scope. `chain` logs on the `SYNC` subsystem; the main package's `log` is the `DCRW` subsystem. `%v` of a hash is its reversed-hex string.

| Level | Subsystem | Format | Citation |
|---|---|---|---|
| info | SYNC | `Headers synced through block <hash> height <h>` | `chain/sync.go:618` |
| info | SYNC | `Transactions synced through block <hash> height <h>` | `chain/sync.go:626,628` |
| info | SYNC | `Waiting for dcrd instance to catch up to minimum block height <n> (<blocks> blocks, <headers> headers, chain synced=<bool>)` | `chain/sync.go:486-488` |
| info | SYNC | `Wallet voting enabled: vote bits = <%#04x>, extended vote bits = <%x>` — Go pads the digits to four after the `0x` prefix, so vote bits 1 print as `0x0001`; the extended bits are lower-case hex of the byte string | `chain/sync.go:651-652` |
| info | SYNC | `Please ensure your wallet remains unlocked so it may vote` | `chain/sync.go:653` |
| info | SYNC | `Fetched <added> new header(s) ending at height <h> from <url>` — `<url>` is the full dialed URL, e.g. `wss://localhost:9109/ws` | `chain/sync.go:296-297` |
| info | SYNC | `Reorganize from <old tip> to <new tip> (total <n> block(s) reorged)` | `chain/sync.go:332-333,897-898` |
| info | SYNC | `Connected block <hash>, height <h>` | `chain/sync.go:342` |
| info | SYNC | `Connected <n> blocks, new tip block <hash>, height <h>, date <time>` (Q20) | `chain/sync.go:344-345` |
| info | SYNC | `Connected block <hash>, height <h>, <n> wallet transaction(s)` | `chain/sync.go:904-905` |
| info | SYNC | `Observed sidechain or orphan block <hash> (height <h>)` | `chain/sync.go:908` |
| info | SYNC | `Received header for block <hash> (height <h>) when parent <hash> not in main or side chain. Re-requesting missing headers.` | `chain/sync.go:853-856` |
| warn | SYNC | `Too many (2000) missing headers. Performing full header sync.` | `chain/sync.go:421-422` |
| warn | SYNC | `Could not publish one or more unmined transactions: <err>` | `chain/sync.go:694` |
| trace | SYNC | `TSpends in mempool: <n>` | `chain/sync.go:705` |
| info | SYNC | `Blockchain sync completed, wallet ready for general usage.` | `chain/sync.go:744` |
| error | SYNC | `<method>: <err>` for `winningtickets`, `relevanttxaccepted`, `tspend`, `mixmessage` handler failures (the logged value is the error itself, no format string) | `chain/sync.go:777,794,799,804` |
| error | SYNC | `Failed to connect block: blockconnected: <err>` | `chain/sync.go:787` |
| debug | SYNC | `Ignoring mixmessage notification: mixing disabled` | `chain/sync.go:935` |
| info | SYNC | `RPC synchronization stopped` | `dcrwallet.go:589` |
| error | SYNC | `RPC synchronization stopped: <err>` | `dcrwallet.go:592` |
| warn | DCRW | `Cannot open CA file: <err>` | `dcrwallet.go:609` |
| info | DCRW | `Chain server RPC TLS is disabled` | `dcrwallet.go:615` |
| warn | DCRW | `Cannot open dcrd RPC client certificate: <err>` | `dcrwallet.go:627` |
| warn | DCRW | `Cannot open dcrd RPC client key: <err>` | `dcrwallet.go:632` |

`rpc/client/dcrd` and wsrpc log nothing. wsrpc and `notifier.Notify` emit Go execution-tracer tasks and log events only (Q37).

---

## 9. Compatibility-critical behaviours

1. **Endpoint and envelope.** Connect to `ws[s]://host:port/ws`; send JSON-RPC requests with `"jsonrpc":"2.0"`, positional `params` (omitted when empty), numeric `id` ≥ 1; treat any message with a non-empty `method`, no `result`, no `error` and a null/absent id as a notification. dcrd's websocket handler depends on this framing.
2. **Authentication.** HTTP Basic credentials go in the `Authorization` header of the upgrade request (never an `authenticate` JSON-RPC call); with `dcrdauthtype=clientcert` no `Authorization` header is sent and the TLS client certificate is presented instead.
3. **TLS trust.** The dcrd certificate is verified against the configured CA file only (not the system roots) whenever a CA is configured; the server name is the configured host. Plain `ws://` only for `--noclienttls` (daemon: localhost only) or gRPC loopback-without-certificate.
4. **Proxy rules.** The configured SOCKS5 proxy is bypassed for `localhost` and private/loopback literal IPs, and entirely with `--nodcrdproxy`; the gRPC `RpcSync` path never uses the proxy.
5. **Network and API gates.** `getcurrentnet` must equal the wallet's network magic; `version.dcrdjsonrpcapi` must satisfy §5.3 against 8.3.0. Error texts are visible in logs and gRPC status messages.
6. **Exact method set and parameter shapes** in §4.2, including: reversed-hex hashes; `loadtxfilter` always passing two arrays; `getheaders` with a 64-zero stop hash; `rescan` with a contiguous hash array of at most 2000; `getrawmempool false "<kind>"`; `sendrawmixmessage <command> <hex>`.
7. **Subscription sequence** in §5.2: `notifywinningtickets` (voting only) → cfilters → headers → `loadtxfilter` reload → rescan → `notifyblocks` → `rebroadcastwinners` → rebroadcast unmined → mempool tspends → `notifytspend` → `notifymixmessages` (mixing only).
8. **Filter completeness.** dcrd's per-connection filter is the only source of relevant transactions after sync. Every new address and outpoint the wallet starts caring about must be added with `loadtxfilter false …` on the same connection, and the whole filter must be rebuilt (`reload = true`) on every (re)connection and every full header re-sync.
9. **Duplicate transaction tolerance.** `sendrawtransaction` error −40 is success; all transactions in a batch are attempted in order even after a failure.
10. **Header catch-up verification.** During `getHeaders`, each header's cfilter commitment proof (post-DCP0005), proof of work, difficulty and stake difficulty are verified locally; a violation aborts the sync attempt (`Run` returns the error). The testnet3 checkpoint is enforced differently: a header whose hash violates it is not an error — its node is marked invalid before being added to the sidechain forest, stays there, and is never returned as (part of) a best chain (`wallet/sidechains.go:77-84,196-216,429-433`).
11. **Block notification semantics.** A `blockconnected` with a `null` or empty transaction list still lets `ChainSwitch` advance the processed-transactions marker (the list is normalized to empty, non-nil); `ChainSwitch` itself withholds the advance when there is a gap between the current rescan point and the first attached block (`wallet/chainntfns.go:212-239`). A block whose parent is unknown triggers `getheaders`-based recovery with per-block `rescan`, or a full re-sync at 2000 headers.
12. **Voting latency and content.** `winningtickets` leads directly to vote creation and `sendrawtransaction`; the deployment status used for the vote's subsidy calculation comes from `getblockchaininfo.deployments` (`VoteIDChangeSubsidySplit`, `VoteIDChangeSubsidySplitR2`) queried at notification time.
13. **Manual-ticket mode.** Unmined ticket purchases reported by `relevanttxaccepted` are dropped when `--manualtickets` is set.
14. **Mixing relay.** With mixing enabled every `mixmessage` is fed to the wallet mixpool with source id 0; a key exchange (run 0) whose own pair request is missing triggers one `getmixmessage` for that pair request. Outgoing mix messages go through `sendrawmixmessage`.
15. **Sync status reporting.** `Synced()` semantics (target = dcrd's `headers` while unsynced, wallet tip after) feed JSON-RPC `syncstatus` and others; the gRPC `RpcSync` stream depends on the callback order of §7.5 item 3 and on `FetchHeadersProgress` carrying the *count of newly added headers in the batch* and the last header's Unix time.
16. **Reconnect cadence.** Fixed 5 s delay, unlimited attempts, a full re-sync sequence each time; the wallet has no network backend in between.
17. **RPC passthrough.** Any JSON-RPC method the wallet does not implement is forwarded verbatim (method name and raw params) over the same websocket and dcrd's result or error is returned unchanged; `getinfo`, `getpeerinfo`, `getblock`, `getblockheader`, `gettxout`, `help`, `version`, `walletinfo` (ping) merge or forward dcrd data when in RPC mode.
18. **Capability interfaces.** Only the RPC backend provides deployment queries (`deployments.Querier`), live-ticket queries (`wallet.LiveTicketQuerier`, i.e. `existslivetickets` only) and exists-address discovery (`usedAddressesQuerier`). Wallet behaviour (ticket status precision, discovery algorithm, DCP0010/DCP0012 vote subsidy) switches on their presence. `gettxout` is not part of any capability interface: it is called on the concrete `*chain.Syncer` by JSON-RPC `signrawtransaction`; verbose `getrawtransaction` (`GetConfirmationHeight`) is never issued.
19. **Log lines.** The `SYNC` messages quoted in §5 are what operators and Decrediton-style launchers read; keep the texts. The complete list with levels is §8.3.
20. **Keep-alive.** An empty-payload ping every 60 s from dial time, pongs to server pings echoing the payload, and no liveness timeout of the client's own: a silent dcrd is never dropped by the wallet; only a write failure (10 s deadline), a read error or a close frame ends the connection.
21. **One connection, no batching.** All traffic — sync, wallet-initiated calls, RPC passthrough, notifications — shares a single websocket; requests are never batched and never use named parameters; up to 2000 requests may be outstanding at once (Q15). dcrd's per-connection state (loaded filter, notification registrations) is tied to that connection, so nothing may be sent over a second connection.
22. **gRPC `RpcSync` stream.** Preconditions, error codes/messages, the relock after the first `DiscoverAddressesFinished`, and the callback → `RpcSyncResponse` mapping of §5.1.2.
23. **Backend cancellation.** `WrapContext` children are cancelled when `Run` returns; in-flight ticket purchases and mixes abort at that moment (§3.1.1).
24. **Passive behaviour toward unknown input.** Unknown notification methods are ignored silently; unknown keys in results are ignored; unknown components in the `version` reply are ignored. A stricter port would break against a newer dcrd that adds notifications or fields within API major version 8.

---

## 10. Quirks

| # | Quirk | Citation |
|---|---|---|
| Q1 | `StakeDifficulty` decodes the key `nextstakedifficulty`, but dcrd's `getstakedifficulty` result uses `current` and `next`. The decoded value is therefore always 0 and the call returns `(0, nil)`. The interface comment claims the method "will always error" for non-RPC backends; the RPC backend silently returns a zero price instead. Reached only when DCP0001 is not active at the tip (mainnet below height 149248), so effectively dead. | `rpc/client/dcrd/calls.go:478-489`, `types/v4@v4.4.0/chainsvrresults.go:339-342`, `wallet/createtx.go:1209-1212`, `wallet/network.go:43-46` |
| Q2 | `rebroadcastwinners` is issued even when voting is disabled; `notifywinningtickets` only when enabled. | `chain/sync.go:645-647,683` |
| Q3 | Blocks connected by dcrd between the end of `getHeaders` and the `notifyblocks` call are not fetched until the next `blockconnected` arrives with an unknown parent. The wallet can report synced while one or more blocks behind for up to a block interval. | `chain/sync.go:670-680,848-858` |
| Q4 | The first `blockconnected` failures on a connection are logged and swallowed; after one success any failure closes the connection and restarts sync. | `chain/sync.go:779-790` |
| Q5 | Blocks delivered by notification or by `getMissingHeaders` skip `ValidateHeaderChainDifficulties`; only the bulk header path validates proof of work and stake difficulty. | `chain/sync.go:319,885-891` |
| Q6 | `blockdisconnected` and `reorganization` are never processed. Reorganizations are inferred solely from the `blockconnected` notifications dcrd sends for the new chain's blocks: each is added to the sidechain forest and the wallet switches only once `EvaluateBestChain` finds a chain with strictly more work than its current one. | `chain/sync.go:773-807,883-891`, `wallet/sidechains.go:417-465` |
| Q7 | The winner list from `winningtickets` is in Go map iteration order (random per run); votes for several owned winners are therefore published in random order. | `rpc/client/dcrd/notifications.go:31-41` |
| Q8 | Positional notification decoding is lenient: short arrays, `null` elements, `null` params and extra elements are accepted and leave targets zero-valued (§4.5.1). E.g. `relevanttxaccepted [null]` yields an empty transaction handed to `AddTransaction`. | `notifications.go:16-22` |
| Q9 | Several replies are dereferenced without nil checks and would crash the Go process on a malformed server reply: `getcfilterv2` without a `data` string; a `null` ticket value in `winningtickets`; `getblockchaininfo` or `getmixmessage` answering `null`; `getheaders` answering `{"headers":null}` (Go's decoder sets the pre-allocated `*headers` field to nil without calling its `UnmarshalJSON`, and `res.Headers.Headers` is then dereferenced; a `null` result or an absent `headers` key is harmless and yields no headers). A port should return an error instead. | `calls.go:351,379-383,281,396-405,497-501`, `notifications.go:38-40`, `chain/sync.go:241,478` |
| Q10 | Hex-in-JSON decoders stream: bytes after a complete header/transaction/block/mix message are ignored, upper-case hex is accepted, and JSON escapes inside the string are not interpreted by `unhex`, `transactions` and `messageArray` (they slice the raw token); `headers`, `hashes`/`hashesContiguous` and the `rescan` reply go through standard JSON string decoding first, so escapes are interpreted there. `hashes` accepts hash strings shorter than 64 digits; `hash` requires exactly 64. | `json.go:32-44,106-120,128-147,234-281` |
| Q11 | The client certificate is installed only inside the "CA present" branch. With `dcrdauthtype=clientcert` and an unreadable CA file the connection is attempted with no client certificate, no Basic credentials (they were cleared) and the system trust store. | `chain/sync.go:555-578`, `dcrwallet.go:579-584` |
| Q12 | A dcrd password without a username sends no `Authorization` header. | `chain/sync.go:547-549` |
| Q13 | `rpcSyncLoop` restarts immediately, without the 5 s delay, if `Run` returns nil. `Run` cannot return nil after a successful start (the watcher always yields a non-nil error), so this is unreachable. | `dcrwallet.go:586-598`, `chain/sync.go:746-757` |
| Q14 | `BlockLocators` lists the tip hash twice (index 0 and 1), so every `getheaders` request carries a duplicated first locator. | `wallet/wallet.go:1497-1536` |
| Q15 | Header catch-up issues up to 2000 `getcfilterv2` requests at once on the single connection (likewise `Blocks`, `CFiltersV2`), relying on dcrd to queue them. | `chain/sync.go:253-275`, `calls.go:324-336,368-388` |
| Q16 | If `notifymixmessages` fails with a JSON-RPC error while the connection is healthy, `Run`'s deferred wait on `notifier.done` can block until the connection is closed by some other event, because the errgroup context is usually not yet done when the defer checks it. | `chain/sync.go:713-742` |
| Q17 | `notifier.done` is closed when the client's final error is set, not when the last notification handler returns. The comment "Wait for notifications to finish before returning" overstates it: a handler can still be running (with a cancelled `ntfnCtx`) after `Run` has returned. | `chain/sync.go:722-723,810-815`, `wsrpc rpc.go:270-281` |
| Q18 | `wsrpc.Client.Call` returns the context error whenever the context is done at return time, even if the response had already arrived; a transaction accepted by dcrd can be reported as cancelled. | `wsrpc rpc.go:434-438` |
| Q19 | `errors.Opf("wallet.FetchMissingCFilters(%v)", n)` formats the backend with `%v`. `*chain.Syncer` has no `String` method, so the op contains Go's struct dump of the syncer (addresses, internal state). Not reproducible byte-for-byte; choose a stable rendering. | `wallet/wallet.go:1423-1431,1448-1456` |
| Q20 | `Connected %d blocks, … date %v` prints the header timestamp with Go's default `time.Time` format in the process's local time zone. | `chain/sync.go:344-345` |
| Q21 | `GetConfirmationHeight` reports its errors under the op `dcrd.GetRawTransaction`. | `calls.go:526` |
| Q22 | Account discovery is decided by `!w.Locked()` at `NewSyncer` time; `DisableDiscoverAccounts` has no caller. Consequences: gRPC `RpcSync` with `discover_accounts = false` still discovers accounts if the wallet happens to be unlocked; every reconnect creates a new `Syncer` and re-runs account discovery if the wallet is unlocked and a rescan point exists. | `chain/sync.go:82,117-119,360-367`, `internal/rpc/rpcserver/server.go:2848-2872` |
| Q23 | `targetSyncHeight` is first written inside the header loop; `Synced()` reports target 0 during `waitRPCSync` and the missing-cfilter phase. It is not updated during a re-sync that starts in the synced state. | `chain/sync.go:226-242` |
| Q24 | When `EvaluateBestChain` returns an empty chain the header loop continues without refreshing locators, repeating one identical `getheaders` and its cfilter fetches before exiting. | `chain/sync.go:315-317` |
| Q25 | If dcrd is behind the wallet (`getheaders` returns only known blocks, or nothing), the wallet keeps its extra blocks, marks itself synced and waits. | `chain/sync.go:299-307` |
| Q26 | Errors from `BlockInMainChain` are discarded at all three call sites (a DB error reads as "not in main chain"). | `chain/sync.go:283,836,845` |
| Q27 | `ExistsLiveTickets` with an empty list still performs the round trip; `UsedAddresses` with an empty list does not. | `calls.go:111,131-142` |
| Q28 | The notification queue is unbounded (one parked goroutine per pending notification). | `wsrpc rpc.go:386-404` |
| Q29 | A duplicate `tspend` notification is reported at error level (`tspend: tspend already cached`). The same duplicate during the initial mempool load is ignored silently. | `chain/sync.go:702-704,797-800`, `wallet/wallet.go:259-270` |
| Q30 | The CA file and client key pair are read once per process; reconnects reuse the bytes. | `dcrwallet.go:563-564` |
| Q31 | The client always offers `permessage-deflate`; dcrd never accepts it. | `wsrpc rpc.go:206-210` |
| Q32 | The `mixmessage` handler checks `MixingEnabled()` although the subscription is only made when mixing is enabled; the debug line is reachable only if dcrd sends unsolicited mix messages. | `chain/sync.go:934-937` |
| Q33 | `Callbacks.FetchHeadersProgress` declares its first parameter as `lastHeaderHeight`, but the syncer passes the number of headers newly added to the sidechain forest in that batch (the internal wrapper names it `fetchedHeadersCount`, and gRPC forwards it as `fetched_headers_count`). | `chain/sync.go:95,176-180,294`, `internal/rpc/rpcserver/server.go:2913-2922` |
| Q34 | `dcrd.RPC.PublishTransaction`, `ExistsLiveTicket`, `MixPairRequests`, `cfilterV2.Deserialize`, `Syncer.DisableDiscoverAccounts` and `Syncer.GetConfirmationHeight` (hence `dcrd.RPC.GetConfirmationHeight` and the verbose `getrawtransaction` call) have no callers in the tree. The comments on `Syncer.GetTxOut` and `Syncer.GetConfirmationHeight` claim they fulfil `wallet.LiveTicketQuerier`, which declares only `ExistsLiveTickets`. | `chain/backend.go:71-79`, `wallet/tickets.go:18-23` |
| Q35 | After the client has failed, already queued notifications are dropped or delivered by a pseudo-random `select` (§5.10.4); the number of notifications processed after a failure is not deterministic. A port should pick one behaviour (dropping everything queued is the simplest and loses nothing that the next full sync does not recover). | `wsrpc rpc.go:389-402` |
| Q36 | Any server message with a `null`/absent id that is not a pure notification (an error reply to an unparseable request, `{}`, a notification that also carries a non-null `error`) is treated as a response to id 0 and terminates the connection with `wsrpc: unknown response ID`. | `wsrpc rpc.go:385,407-415` |
| Q37 | `notifier.Notify` wraps every notification in a Go `runtime/trace` task named after the method, and wsrpc creates tasks `Dial`, `<method>` (per call), `ping`, `wsrpc.Client.in`, `wsrpc.Client.out`, a region `PongHandler`, and trace log events. These are visible only to the Go execution tracer; there is nothing to port. | `chain/sync.go:771-772`, `wsrpc rpc.go:198,228-232,275,295,346,355-358,431` |
| Q38 | Source annotations: `// TODO: consistent error wrapping` at the top of `calls.go` (the inconsistencies are those of §5.8: `publishMixMessage` and `getRawTransaction` return unwrapped call errors, `GetConfirmationHeight` uses another method's op, `MixMessage` returns decode errors unwrapped, `CFiltersV2` wraps per task but not the group error); `// XXX: this also loads active addresses.` above the `getHeaders` call in `Run` (true: `getHeaders` performs discovery, the filter reload and the rescan, §5.5 steps 6–9). | `rpc/client/dcrd/calls.go:5`, `chain/sync.go:669` |
| Q39 | `wallet.NetworkBackend` documents `StakeDifficulty` as "impossible to determine over the wire protocol, and will always error". That describes the SPV backend; the RPC backend returns `(0, nil)` (Q1). `Syncer.PublishMixMessages` is the one `NetworkBackend` method in `backend.go` whose comment does not say it is part of the interface. | `wallet/network.go:43-46`, `chain/backend.go:41-45` |
| Q40 | `MempoolCount`'s comment enumerates the accepted kinds without `"tspend"`, which `GetMempoolTSpends` passes to the same dcrd method. The comment also notes the call is wasteful (all hashes are transferred to count them). | `rpc/client/dcrd/calls.go:145-163,176` |
| Q41 | `Syncer.Synced` performs a wallet database read (`MainChainTip`) once synced and returns that tip height as the "target"; the tuple is therefore not a consistent snapshot (a block may connect between the atomic load and the read), and `MainChainTip`'s own failure mode (it ignores database errors) is inherited. | `chain/sync.go:123-132` |
| Q42 | `fetchHeadersProgress` receives `int32(added)` and `targetSyncHeight` receives `int32(info.Headers)` (an `int64` from JSON): both are plain truncating conversions. Unreachable with real chain heights; a port should use a checked or saturating conversion. | `chain/sync.go:241,294` |
| Q43 | In `getHeaders` the per-header tasks run on an `errgroup.Group` **without** a derived context: one failing `getcfilterv2` does not cancel the other outstanding requests; `g.Wait` returns the first error only after all tasks have finished. The same holds for `dcrd.RPC.Blocks` and `CFiltersV2`. | `chain/sync.go:253-279`, `rpc/client/dcrd/calls.go:325-336,369-388` |
| Q44 | The gRPC `RpcSync` path computes `Insecure` from the address and certificate only; a non-loopback address with an empty certificate gives `wss` with the **system** trust store (no `CA` → no custom TLS config, §5.1.3). The daemon path cannot reach that combination unless the CA file is unreadable (then it does, with a warning). | `internal/rpc/rpcserver/server.go:2865-2872`, `chain/sync.go:555`, `dcrwallet.go:606-612` |
| Q45 | `cfilterV2.UnmarshalJSON` indexes `j[0]` and `j[len(j)-1]` after only a parity test; it is safe solely because `encoding/json` never hands an empty token to an unmarshaler. The parity rule also means a JSON string holding an odd number of hex digits is reported as `cfilterv2 is not an even hex string`, while a number such as `12` (even length, no quotes) gets the same message and `123` likewise. | `rpc/client/dcrd/json.go:65-68` |
| Q46 | `WinningTickets` with a `null` or missing element 0 returns a nil block-hash pointer (and with a missing element 2 an empty winner list) without error; the nil pointer is handed to `VoteOnOwnedTickets`. A port should reject the notification. | `rpc/client/dcrd/notifications.go:31-41`, `chain/sync.go:817-823` |
| Q47 | `rescan`'s `save` callback receives a non-nil empty list for a block entry without transactions, whereas `getMissingHeaders` accumulates with `append` and so passes **nil** to `handleBlockConnected` when nothing matched; both are then normalized to "empty, non-nil" (§5.6.2 step 7). | `rpc/client/dcrd/calls.go:456-465`, `chain/sync.go:436-445` |
| Q48 | The commitment check accepts any `(proofindex, proofhashes)` pair that hashes to the stake root; it does not pin the cfilter's position in the commitment tree, and on networks other than mainnet/testnet3 it applies from genesis (§5.5.1). | `validate/validate.go:80-100` |
| Q49 | Mainnet and testnet3 headers below the DCP0005 heights get **no** per-header cfilter validation in `getHeaders`; the filter is stored as received. | `validate/validate.go:80-89` |
| Q50 | `sweepaccount` runs the wsrpc client with the default pong deadline (≈ 66.7 s after each pong, ≈ 126.7 s initially) whereas the daemon and `repaircfilters` disable it; the transport must implement both modes. | `cmd/sweepaccount/main.go:317-322`, `wsrpc rpc.go:201-243` |
| Q51 | Account discovery on the daemon path depends on a side effect of startup (`--pass`, `--promptpass`, ticket buyer, or first-run prompt having unlocked the wallet) rather than on any syncer option (§5.1.1). | `dcrwallet.go:240-248,468-529`, `chain/sync.go:82` |

---

## 11. Upstream tests

| File | Tests | What they pin |
|---|---|---|
| `chain/semver_test.go` | `TestSemverCompatible` (6 cases), `TestSemverToString` (2 cases) | equal versions compatible; different major, lower minor, lower patch at equal minor incompatible; `"%d.%d.%d"` rendering |
| `rpc/client/dcrd/json_test.go` | `TestUnmarshalHashes`, `TestUnmarshalHashesContiguous` | `null` → empty; malformed JSON → error; non-hash strings → kind `Encoding`; two mainnet-style hashes round-trip through `String()` |
| | `TestMarshalHashes`, `TestMarshalHashesContiguous` | nil → `null`; exact `["<h0>","<h1>"]` bytes |
| | `TestUnmarshalHeaders` | `null`, malformed JSON, non-hex strings (`Encoding`), two serialized headers decode to deep-equal structs |
| | `TestUnmarshalTransactions` | `null`, malformed JSON, non-string element (`Encoding`), non-hex strings (`Encoding`), two serialized transactions |
| | `TestUnmarshalHash` | 63- and 65-digit strings, missing quote at either end → `Encoding`; 64 `f` digits decode |

**Port directly:** all of the above (the inputs are literal).

**Untested upstream — generate vectors through a Go oracle shim:**

- Request bytes for every wrapper in §4.2: run each `dcrd.RPC` method against a recording `Caller` that marshals `(method, args)` through the wsrpc `request` struct with `json.Encoder`, capturing the exact frame including the trailing newline; cover nil/empty slices, 0/1/many elements, and `UsedAddresses` batching boundaries (399 999, 400 000, 400 001 — or a reduced batch constant in the shim).
- Result decoding for every wrapper against canned replies, including the error/kind and op-prefixed text for each failure in §5.8 and §8.1.
- `cfilterV2.UnmarshalJSON`, `unhex`, `messageArray`, `hash` edge cases (odd length, missing quotes, empty string, trailing bytes, upper-case hex).
- Notification decoders with the edge inputs of §4.5.1, and all eight mix message commands.
- `semverCompatible` across a small grid around 8.3.0.
- `normalizeAddress` (host only, host:port, IPv6 literal with and without brackets/port, empty string, malformed).
- wsrpc classification of incoming frames (§4.1.5) and id allocation, including the null-id cases of Q36.
- The log formats of §8.3 that depend on Go formatting verbs (`%#04x` vote bits, `%x` extended bits, `%v` of a `time.Time`, `%v` of a nested `*errors.Error` with the `":: "` separator).
- Error rendering for each op/kind combination of §5.8 as produced by `errors.E` (op collapsing in `PublishTransactions`, the op-less inner error of `PublishMixMessages`, `exists` double op).
- `validate.CFilterV2HeaderCommitment` (§5.5.1): the `validate` package contains only `validate.go` at this pin, so nothing upstream tests it. Generate vectors from real mainnet/testnet3 blocks above the DCP0005 heights (header, `getcfilterv2` reply) plus negatives: wrong index, proof longer than 32, truncated proof, empty filter, a pre-activation height with a garbage proof (must pass).
- Result-decoding rules of §4.1.5 (`null` into each target kind, wrong type, out-of-range numbers) for `getcurrentnet`, `version`, `getblockchaininfo`, `gettxout`, `getrawmempool`.
- `rescan` reply edge cases of §4.3 (absent/`null`/empty `discovereddata` and `transactions`, short hash strings, failure in a later entry after earlier `save` calls).
- Mix message hashing: `WriteHash` output for one message of each of the eight types.

Test data: both in-scope test files embed their inputs as literals; there is no `testdata` directory under `chain/` or `rpc/client/dcrd/`. The wsrpc module ships no `_test.go` files (its `cmd/` directory holds two command-line tools, not tests).

**Not covered by any upstream test:** `chain/sync.go` in its entirety (`Run`, `getHeaders`, `getMissingHeaders`, `waitRPCSync`, notification handling, shutdown), `chain/backend.go`, `rpc/client/dcrd/calls.go`, `rpc/client/dcrd/notifications.go`, the wsrpc module (it ships no tests), and `rpcSyncLoop`. The sidechain forest and chain-switch logic the syncer drives are covered by `wallet/sidechains_test.go` and `wallet/reorg_test.go` (owned by the wallet specs). End-to-end behaviour needs a harness with a real dcrd (or dcroxide) on simnet: sync from genesis, reconnect after dcrd restart, reorg via `invalidateblock`/competing miner, voting on `winningtickets`, mixing relay.

---

## 12. Rust porting notes

### 12.1 Suggested boundaries

| Crate / module | Mirrors | Contents |
|---|---|---|
| `purse-wsrpc` | `jrick/wsrpc/v2` + the client half of gorilla/websocket | websocket client handshake and framing (masking, ping/pong/close, optional permessage-deflate), JSON-RPC 2.0 request multiplexing by id, ordered notification delivery, final-error semantics, Basic-auth header, pluggable dialer and TLS |
| `purse-dcrd-client` | `rpc/client/dcrd` | `Caller` trait, typed method wrappers, request encoders and result/notification decoders, error ops |
| `purse-chain` (module `rpc_sync`) | `chain` | `Syncer`, `RpcOptions`, `Callbacks`, sync state machine, notifier, `NetworkBackend` + capability trait impls, semver check |
| daemon crate | `dcrwallet.go` | `rpc_sync_loop`, CA/client-cert loading, backend selection |

### 12.2 Reuse candidates (inspected)

| Need | Candidate | Notes |
|---|---|---|
| Block header, transaction, block, mix message (de)serialization; `MIX_VERSION`; command strings | `dcroxide-wire` (`blockheader.rs`, `msgtx.rs`, `msg_mix.rs`, `message.rs`) | `BlockHeader::from_bytes` and `MsgTx::from_bytes` return the consumed length (`dcroxide:crates/dcroxide-wire/src/blockheader.rs:106`, `msgtx.rs:360`), which makes the trailing-bytes leniency of Q10 a one-line choice |
| Hash type and reversed-hex codec | `dcroxide-chainhash` | check that its string decoder matches `NewHashFromStr` padding rules before using it for `hashes` |
| GCS v2 filters | `dcroxide-gcs` (`FilterV2::from_bytes`, `blockcf2::B`, `blockcf2::M`, `hash()`) | `dcroxide:crates/dcroxide-gcs/src/lib.rs:497,556`, `blockcf2.rs:14-18` |
| Inclusion proof verification | `dcroxide-standalone::verify_inclusion_proof` | `dcroxide:crates/dcroxide-standalone/src/inclusionproof.rs:119` |
| Go-compatible JSON string escaping and number formatting for request emission | `dcroxide-dcrjson::gojson` (`append_json_string`, float formatters) | the request envelope is small enough to emit by hand on top of these |
| dcrd result/command shapes | `dcroxide-rpctypes` | built around a runtime Go-type model for server-side emission; for client-side decoding plain typed structs are likely simpler. Useful as the authoritative field list |
| TLS | `dcroxide-tlscrypto` (rustls provider with Go's key-parsing rules) | cipher-suite restriction and custom root store map directly onto rustls configuration |
| Websocket framing, SOCKS5 dialer | `dcroxide-node/src/wsframe.rs`, `dcroxide-node/src/socks.rs` | server-side framing only (unmasked writes, masked reads) and both live inside the node binary crate; they would have to be extracted into a library crate and extended with the client role |
| Mixpool acceptance and `MissingOwnPR` error | `dcroxide-mixing` (`mixpool.rs`) | owned by the mixing spec |
| dcr-rs | — | provides transaction/address/HD primitives without networking; nothing in this scope needs it beyond what `dcroxide-wire` already supplies |

### 12.3 Hazards

1. **Re-entrancy.** Notification handlers issue calls on the same connection and wait for the replies; the reader must keep running while a handler is active. With dcroxide's thread-based model (ADR-0005) this means at least a reader thread, a writer (or a write lock with ping priority), and one notification worker thread fed by an unbounded FIFO.
2. **Mass concurrency.** Do not translate "one goroutine per header" into 2000 threads: pipeline the 2000 `getcfilterv2` requests on the socket and collect responses by id. Keep the observable behaviour (all requests outstanding at once, results in input order, first error wins).
3. **Cancellation.** Three independent cancellation scopes (§7.4) plus the "context error overrides result" rule. Model them explicitly; do not collapse `ntfnCtx` into the caller's scope or the shutdown ordering changes.
4. **nil versus empty.** `relevantTxs` nil/non-nil drives whether the processed-transactions marker advances; `hashes` nil/empty selects `null`/`[]` on the wire; `addrSliceToStrings` never yields `null`.
5. **Map order.** Winner order (Q7) is random in Go; pick a deterministic order (e.g. numeric key order) and document the divergence.
6. **Error text from Go libraries** (`net`, `crypto/tls`, `crypto/x509`, gorilla, `encoding/json`, `encoding/hex`) appears in log lines and gRPC status messages. Exact reproduction is impractical; decide which texts are contractual (suggested: only the texts authored in dcrwallet/wsrpc and listed in §8.1).
7. **Lenient decoding** (Q8–Q10): decide per case between Go-faithful leniency and strictness; strictness can only cause divergence against a non-conforming server.
8. **Panics** (Q9) must become errors.
9. **Request bytes.** Trailing `\n`, omitted `params`, HTML escaping, key order, and compaction of raw passthrough params are part of "byte-identical requests" if that is a goal.
10. **Handshake bytes.** `User-Agent: Go-http-client/1.1`, header order and the `permessage-deflate` offer are reproducible; the TLS ClientHello is not.

### 12.4 Size

**M–L.** In-scope Go is about 2,100 non-test lines plus 528 lines of wsrpc, but the port also has to supply the client half of gorilla/websocket and the TLS/SOCKS glue under the pure-Rust policy, and the state machine has intricate shutdown and re-entrancy semantics with no upstream tests. The typed client and codecs are S on their own; the transport is M; the syncer with its lifecycle and an integration harness against dcrd/dcroxide is M.

---

## 13. Open questions

1. **permessage-deflate.** Offer the extension (byte-identical handshake) and implement no-context-takeover deflate, offer it without being able to honour acceptance, or stop offering it? dcrd never accepts it, but a reverse proxy in front of dcrd could. UNVERIFIED against dcrd's Go source (only dcroxide's server was read); settle by reading dcrd `internal/rpcserver` websocket upgrader options.
2. **Handshake fidelity.** Is `User-Agent: Go-http-client/1.1` to be kept, or replaced by a dcr-purse identifier? Nothing in dcrd is known to depend on it.
3. **Winner ordering (Q7)** and **`FetchMissingCFilters(%v)` op text (Q19):** choose deterministic replacements.
4. **Leniency (Q8, Q10):** reproduce or tighten.
5. **Q1 (`getstakedifficulty` key):** reproduce the zero result, fix the key to `next`, or return an error as the interface comment describes. The path is unreachable on current networks.
6. **Q11 (client certificate without CA):** reproduce, or fail fast with a clear error.
7. **Q16/Q17 (shutdown waits):** reproduce the "wait for client error" semantics, or actually wait for the in-flight handler. Waiting is safer but changes timing only.
8. **Q3 (gap before `notifyblocks`):** keep, or subscribe before the final `getheaders` round. Subscribing earlier changes the observable call order.
9. **Q22 (account discovery on every reconnect):** keep, or remember completion across `Syncer` instances.
10. **Go library error texts** (hazard 6): confirm the proposed contract boundary.
11. **dcrd behaviour without credentials.** dcrd's handling of an unauthenticated websocket (expecting an `authenticate` request) was not read; the wallet never sends `authenticate`. UNVERIFIED how the resulting failure surfaces (expected: the first call fails or the socket is closed); settle with dcrd `internal/rpcserver` or a live test.
12. **`existsaddresses` availability.** RPC-mode discovery relies on dcrd's exists-address index; the error text when dcrd runs without it was not traced. UNVERIFIED; settle against dcrd/dcroxide `handleExistsAddresses`.
13. **Websocket crate policy.** Extract and extend dcroxide's `wsframe.rs`/`socks.rs` into shared library crates, or write a fresh client in `purse-wsrpc`?
14. **Q35 (notifications queued at failure time):** drop all, or deliver all before tearing down. Go does either, at random.
15. **Q36 (null-id error replies):** keep the connection-fatal behaviour, or log and ignore. UNVERIFIED whether dcrd ever sends such a reply to a well-formed client (settle by reading dcrd's websocket request parser for the error path taken on an oversized or malformed request).
16. **Q44 (system trust store fallback):** reproduce (requires a platform root-store dependency) or reject `wss` without a CA.
17. **TLS ClientHello shape.** No ALPN, SNI = configured host (Go omits SNI for IP literals — UNVERIFIED here, it is `crypto/tls` behaviour outside the read sources), six TLS 1.2 suites plus TLS 1.3 defaults. Decide whether anything beyond "dcrd accepts it" is required.
18. **Server ping handling under a stalled writer.** gorilla answers pings from the reader goroutine with a 1 s deadline while the writer goroutine may hold the write lock for a large request; the interleaving rules (control frames may be injected between fragments of a data message) were not traced in gorilla's `conn.go`. UNVERIFIED; settle by reading `Conn.WriteControl` and `messageWriter.flushFrame`.
