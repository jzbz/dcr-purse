# 14 — SPV mode: the p2p peer implementation and the SPV syncer

## Header

**Scope (dcrwallet, read in full)**

| File | Lines | Contents |
|---|---|---|
| `p2p/peering.go` | 2247 | `LocalPeer`, `RemotePeer`, handshake, message pump, request/response matching, seeding |
| `p2p/log.go` | 15 | package logger hook |
| `spv/sync.go` | 1980 | `Syncer`, peer management, initial sync pipeline, announcements, inv/getdata relay |
| `spv/backend.go` | 652 | `wallet.NetworkBackend` implementation, header/cfilter batch fetchers |
| `spv/rescan.go` | 115 | in-memory transaction filter matching |
| `spv/log.go` | 14 | package logger hook |
| `lru/cache.go` | 67 | `lru.Cache[T]` |
| `lru/map.go` | 101 | `lru.Map[K,V]` |

**Dependencies read as used by the scope**

| Module | Version | What was read |
|---|---|---|
| `github.com/decred/dcrd/addrmgr/v3` | v3.0.0 | all non-test files (`addrmanager.go` 1227, `knownaddress.go` 117, `netaddress.go` 186, `network.go` 272, `error.go` 53) |
| `github.com/decred/dcrd/connmgr/v3` | v3.1.4 | `dynamicbanscore.go` (147), `tor.go` (137, used by config only) |
| `github.com/decred/dcrd/wire` | v1.8.0 | `protocol.go`, `message.go`, `msgversion.go`, `netaddress.go`, `invvect.go`, and the encode/decode of every message the wallet sends or handles |
| `github.com/decred/dcrd/gcs/v4` | v4.1.1 | `gcs.go` (`FromBytesV2`, `MatchAny`, `Hash`), `blockcf2/blockcf.go` (`B`, `M`, `Key`, `Entries`) |
| `github.com/decred/dcrd/mixing` | v0.7.5 | `message.go` (`Message` interface), `mixpool` (`Source`, `MissingOwnPRError`) |
| `github.com/decred/dcrd/chaincfg/v3` | v3.3.0 | seeders, default ports, `PiKeyExists` |
| `github.com/decred/dcrd/crypto/rand` | v1.0.1 | `Duration`, `Uint64` |
| `github.com/decred/go-socks` | v1.1.0 | `ProxiedAddr`, `proxiedConn` |
| `github.com/decred/dcrd/blockchain/standalone/v2` | v2.3.0 | merkle root and inclusion-proof entry points (signatures only) |

**Parity pin:** dcrwallet master `54b85b33` (2.2.0-pre, module `decred.org/dcrwallet/v5`, `go 1.26.0`).

**Summary**

SPV mode replaces the dcrd JSON-RPC backend with a wallet-owned p2p client. `p2p.LocalPeer`
dials outbound-only TCP connections, performs a minimal version/verack handshake at protocol
version 11, and exposes each connection as a `p2p.RemotePeer` with synchronous request methods
(headers, blocks, cfilters, transactions, mix messages, init state) matched to replies by hash.
`spv.Syncer` owns up to 8 such peers (or a fixed `--spvconnect` list), runs a three-stage
header → cfilter → chain-switch pipeline for initial sync, then switches every peer to
`sendheaders` and processes pushed headers one announcement at a time. Transactions and mix
messages are relayed by inv/getdata with per-peer "inv sent" LRUs. Peers are discovered from
HTTPS seeders and `addr` messages and persisted by dcrd's address manager in `peers.json`.
There is no ban list: misbehaviour disconnects the peer and a per-address reconnect backoff
applies. There are no tests for `p2p`, `spv` or `lru` upstream.

---

## 1. Role in the system

### 1.1 What it does

- Provides the `wallet.NetworkBackend` used by the wallet when started with `--spv` (CLI) or
  when a gRPC client calls `WalletLoaderService.SpvSync`.
- Maintains the wallet main chain (headers + committed filters) from untrusted peers,
  verifying proof of work / stake difficulty (wallet code), header-committed cfilter inclusion
  proofs, and block merkle roots.
- Finds wallet-relevant transactions by matching version-2 committed filters against the
  wallet's watched scripts and downloading only matching blocks.
- Announces wallet transactions and StakeShuffle mix messages; fetches unconfirmed
  transactions and mix messages announced by peers.

### 1.2 Who calls it

| Caller | Entry point | Citation |
|---|---|---|
| CLI daemon, `--spv` | `spvLoop`: `addrmgr.New`, `p2p.NewLocalPeer`, `SetDialFunc`, `SetDisableRelayTx`, `spv.NewSyncer`, `SetPersistentPeers`, `Run` in a restart loop | `dcrwallet.go:531-556` |
| gRPC `SpvSync` | builds one `LocalPeer` + `Syncer` with a `Notifications` struct that streams `SpvSyncResponse` | `internal/rpc/rpcserver/server.go:2984-3157` |
| JSON-RPC `getpeerinfo`, gRPC `GetPeerInfo` | `Syncer.GetRemotePeers`, then `RemotePeer` accessors | `internal/rpc/jsonrpc/methods.go:2490-2537`, `internal/rpc/rpcserver/server.go:3982-4046` |
| JSON-RPC `walletinfo` | type switch on `*spv.Syncer`; `len(GetRemotePeers()) > 0` is `daemonconnected`, and `spv` is true | `internal/rpc/jsonrpc/methods.go:5474-5479` |
| JSON-RPC `syncstatus` | `NetworkBackend.Synced` | `internal/rpc/jsonrpc/methods.go:1745` |
| JSON-RPC `getinfo` | `p2p.Pver` reported as `protocolversion` | `internal/rpc/jsonrpc/methods.go:1793` |
| Wallet internals | every `NetworkBackend` method (publishing, rescans, cfilter fetch, address discovery, mixing) | `wallet/network.go:35-57` |
| Logging setup | `spv.UseLogger(SyncLog)`, `p2p.UseLogger(PeerLog)`, `addrmgr.UseLogger(AmgrLog)` | `log.go:36-40` |

### 1.3 What it calls

| Callee | Use |
|---|---|
| `wallet.Wallet` | `MainChainTip`, `RescanPoint`, `BlockHeader`, `BlockInMainChain`, `BlockLocators`, `ValidateHeaderChainDifficulties`, `EvaluateBestChain`, `ChainSwitch`, `FetchMissingCFiltersWithProgress`, `DiscoverActiveAddresses`, `LoadActiveDataFilters`, `RescanProgressFromHeight`, `CFilterV2`, `GetTransactionsByHashes`, `DetermineRelevantTxs`, `AddTransaction`, `UnminedTransactions`, `IsTSpendCached`, `AddTSpend`, `MixingEnabled`, `MixMessage`, `AcceptMixMessageBySource`, `ManualTickets`, `GapLimit`, `Locked`, `ChainParams`, `SetNetworkBackend`, `Run` |
| `wallet.SidechainForest`, `wallet.BlockNode`, `wallet.BadCheckpoint`, `wallet.RescanFilter` | in-memory fork tracking, checkpoint marking, precise tx matching |
| `validate` | `MerkleRoots`, `DCP0005MerkleRoot`, `CFilterV2HeaderCommitment` |
| dcrd `addrmgr` | address book, `peers.json` |
| dcrd `connmgr` | `DynamicBanScore` only |
| dcrd `wire` | message framing and codecs |
| dcrd `gcs`, `gcs/blockcf2` | filter decode and matching |
| dcrd `stake`, `txscript`, `schnorr` | tspend recognition and signature check |
| Go `net/http` | HTTPS seeders |

The wallet-side functions are specified in the wallet chain/sync specs; this document states
only the contract the syncer relies on.

---

## 2. Public surface inventory

### 2.1 Package `p2p`

#### Constants and variables

| Name | Value | Contract | Citation |
|---|---|---|---|
| `uaName` | `"dcrwallet"` | user agent name | `p2p/peering.go:38` |
| `uaVersion` | `version.String()` evaluated at package init | user agent version, e.g. `2.2.0-pre+54b85b336` (build metadata is the first 9 characters of the VCS revision when built from git) | `p2p/peering.go:41`, `version/version.go` |
| `minPver` | 9 (`wire.RemoveRejectVersion`) | lowest remote protocol version accepted | `p2p/peering.go:45` |
| `Pver` (exported) | 11 (`wire.BatchedCFiltersV2Version`) | protocol version advertised; also JSON-RPC `getinfo.protocolversion` | `p2p/peering.go:48` |
| `stallTimeout` | 30 s | per-request reply deadline | `p2p/peering.go:52` |
| `minInvTrickleTimeout` / `maxInvTrickleTimeout` | 100 ms / 500 ms | inv trickle period bounds | `p2p/peering.go:57-60` |
| `banThreshold` | 100 | ban score above which the peer is disconnected | `p2p/peering.go:62` |
| `invLRUSize` | 5000 | limit of per-peer inv LRUs (effective capacity 5001, see §9 Q-01) | `p2p/peering.go:64` |
| `ErrDisconnected` | `errors.New("peer has been disconnected")` | reason substituted for a nil `Disconnect` reason | `p2p/peering.go:721` |
| `ErrNotFound` | `errors.E(errors.NotExist, "transaction not found")` | returned by `Transactions`/`MixMessages` when a `notfound` closes a request | `p2p/peering.go:1567` |
| `MaskGetData`, `MaskInv` | `1<<0`, `1<<1` of type `MessageMask uint64` | selects which unsolicited messages are forwarded to `Receive*` | `p2p/peering.go:870-876` |
| `inMsgPool` | `sync.Pool` of `*inMsg` | allocation reuse only | `p2p/peering.go:778-792` |
| `blake256Hasher`, `blake256Mu` | one global BLAKE-256 hasher under a mutex | used to compute hashes of received mix messages | `p2p/peering.go:1254-1265` |

#### Types

| Type | Contract | Citation |
|---|---|---|
| `DialFunc func(ctx, net, addr string) (net.Conn, error)` | dialer used for peers and seeders | `p2p/peering.go:187` |
| `MessageMask uint64` | bitmask, see above | `p2p/peering.go:870` |
| `LocalPeer` | process-side endpoint: dial function, address manager, chain params, handled-message mask, required peer height, relay flag, table of live peers by id, unbuffered fan-in channels for getdata / inv / announced headers | `p2p/peering.go:145-165` |
| `RemotePeer` | one live outbound connection and its request tables | `p2p/peering.go:81-141` |
| `msgAck` (unexported) | outbound queue item: message + optional ack channel signalled after the write | `p2p/peering.go:66-69` |
| `blockRequest` (unexported) | in-flight block request: hash, `ready` channel, result block or error | `p2p/peering.go:71-76` |
| `msgReader`, `msgWriter` (unexported) | thin wrappers over `wire.ReadMessage` / `wire.WriteMessage` | `p2p/peering.go:436-447`, `484-500` |
| `inMsg` (unexported) | (peer, message) pair passed over the fan-in channels | `p2p/peering.go:773-776` |
| `filterProof` (alias) | `struct{ Filter *gcs.FilterV2; ProofIndex uint32; Proof []chainhash.Hash }`, identical to `wallet.FilterProof` | `p2p/peering.go:1767-1771` |

`LocalPeer` fields `receivedHeaders`, `receivedInitState` and `extaddr` are allocated but never
read (`p2p/peering.go:152,155,157`). `RemotePeer.invsRecv` is allocated and exposed but never
written (`p2p/peering.go:127,343`). `msgReader.rawMsg` is stored but never read.

#### `LocalPeer` functions and methods

| Signature | Contract | Citation |
|---|---|---|
| `NewLocalPeer(params *chaincfg.Params, extaddr *net.TCPAddr, amgr *addrmgr.AddrManager) *LocalPeer` | default dial is `net.Dialer.DialContext`; `extaddr` is stored and unused (callers pass `[::1]:0`) | `p2p/peering.go:169-184` |
| `SetDialFunc(dial DialFunc)` | replaces the dialer for peers and seeders; not synchronized | `p2p/peering.go:190-192` |
| `SetDisableRelayTx(bool)` | sets the relay flag sent in `version` and enforced on received `inv`; must precede use | `p2p/peering.go:197-199` |
| `RequirePeerHeight(int32)` | atomically stores the minimum `LastBlock` a peer must advertise | `p2p/peering.go:247-249` |
| `ConnectOutbound(ctx, addr string, reqSvcs wire.ServiceFlag) (*RemotePeer, error)` | resolve, dial, handshake, start service goroutines, enforce services and height, mark address good, optionally `getaddr`; §4.2 | `p2p/peering.go:255-308` |
| `AddrManager() *addrmgr.AddrManager` | accessor | `p2p/peering.go:311` |
| `SeedPeers(ctx, services wire.ServiceFlag)` | blocking HTTPS seeding; §4.1 | `p2p/peering.go:347-434` |
| `AddHandledMessages(mask)`, `RemoveHandledMessages(mask)` | CAS loop OR / AND-NOT on the mask | `p2p/peering.go:880-900` |
| `ReceiveGetData(ctx) (*RemotePeer, *wire.MsgGetData, error)` | blocks for the next forwarded `getdata`; `ctx.Err()` on cancel | `p2p/peering.go:908-917` |
| `ReceiveInv(ctx) (*RemotePeer, *wire.MsgInv, error)` | blocks for the next forwarded `inv` | `p2p/peering.go:921-930` |
| `ReceiveHeadersAnnouncement(ctx) (*RemotePeer, []*wire.BlockHeader, error)` | blocks for the next `headers` message received from a peer in sendheaders mode | `p2p/peering.go:934-943` |
| `newMsgVersion(pver, c)` (unexported) | builds the outgoing `version`; §3.2 | `p2p/peering.go:230-242` |
| `connectOutbound(ctx, addr, na)` (unexported) | dial + handshake + id assignment; §4.2 | `p2p/peering.go:618-658` |
| `serveUntilError(ctx, rp)` (unexported) | runs the per-peer goroutines; §4.4 | `p2p/peering.go:660-716` |
| `messageIsMasked(m)` (unexported) | mask test | `p2p/peering.go:902-904` |

#### `RemotePeer` accessors

| Signature | Returns | Citation |
|---|---|---|
| `NA() *addrmgr.NetAddress` | address object created by `ConnectOutbound` (its `Services` is overwritten with the advertised services after handshake) | `p2p/peering.go:314`, `654-655` |
| `UA() string` | remote user agent | `p2p/peering.go:317` |
| `ID() uint64` | per-`LocalPeer` id, starting at 1, never 0; also satisfies `mixpool.Source` | `p2p/peering.go:320`, `641-652` |
| `InitialHeight() int32` | `LastBlock` from the remote `version` | `p2p/peering.go:324` |
| `LastHeight() int32` | highest last-header height seen in any accepted `headers` message; 0 until then | `p2p/peering.go:328-333` |
| `Services() wire.ServiceFlag` | advertised services | `p2p/peering.go:336` |
| `InvsSent() *lru.Cache[chainhash.Hash]` | hashes this side has announced to the peer | `p2p/peering.go:339` |
| `InvsRecv() *lru.Cache[chainhash.Hash]` | always empty | `p2p/peering.go:343` |
| `Pver() uint32` | negotiated version `min(11, remote)` | `p2p/peering.go:767` |
| `BanScore() uint32` | current dynamic ban score | `p2p/peering.go:762-764` |
| `RemoteAddr() net.Addr`, `LocalAddr() net.Addr` | from the connection (a SOCKS connection returns `*socks.ProxiedAddr`) | `p2p/peering.go:752-759` |
| `String() string` | `RemoteAddr().String()` captured at handshake | `p2p/peering.go:769-771` |
| `Err() error` | blocks until disconnected, then the first disconnect reason | `p2p/peering.go:741-744` |
| `Done() <-chan struct{}` | closed on disconnect | `p2p/peering.go:747-749` |
| `SendHeadersSent() bool` | whether sendheaders mode is set | `p2p/peering.go:2166-2171` |

#### `RemotePeer` operations

| Signature | Contract | Citation |
|---|---|---|
| `Disconnect(reason error)` | idempotent (atomic CAS); logs, closes the socket, records the first reason (`ErrDisconnected` if nil), closes `errc` | `p2p/peering.go:725-737` |
| `Addrs(ctx) error` | queue `getaddr`; replies are handled asynchronously; §4.6 | `p2p/peering.go:1388-1408` |
| `Block(ctx, hash) (*wire.MsgBlock, error)` | `Blocks` with one hash; wraps errors with op `remotepeer(<addr>).Block(<hash>)` | `p2p/peering.go:1411-1421` |
| `Blocks(ctx, hashes) ([]*wire.MsgBlock, error)` | one `getdata`; joins in-flight requests for the same hash; §4.7 | `p2p/peering.go:1506-1563` |
| `Transactions(ctx, hashes) ([]*wire.MsgTx, error)` | one `getdata`; nil entries + `ErrNotFound` on `notfound`; §4.8 | `p2p/peering.go:1575-1637` |
| `MixMessages(ctx, hashes) ([]mixing.Message, error)` | as `Transactions` with `InvTypeMix`; §4.8 and Q-07 | `p2p/peering.go:1645-1707` |
| `CFilterV2(ctx, hash) (*gcs.FilterV2, uint32, []chainhash.Hash, error)` | one `getcfilterv2`; §4.9 | `p2p/peering.go:1715-1763` |
| `CFiltersV2(ctx, hashes) ([]filterProof, error)` | many pipelined `getcfilterv2`; §4.9 | `p2p/peering.go:1776-1882` |
| `BatchedCFiltersV2(ctx, start, end) ([]filterProof, error)` | one `getcfsv2`; §4.9 | `p2p/peering.go:1886-1940` |
| `SendHeaders(ctx) error` | switch the peer to pushed headers; §4.10 | `p2p/peering.go:1949-1986` |
| `Headers(ctx, locators, hashStop) ([]*wire.BlockHeader, error)` | synchronous `getheaders`; §4.10 | `p2p/peering.go:1992-2036` |
| `HeadersAsync(ctx, locators, hashStop) error` | `getheaders` whose reply arrives as an announcement; §4.10 | `p2p/peering.go:2043-2077` |
| `PublishTransactions(ctx, txs...) error` | append tx invs to the trickle queue; always nil | `p2p/peering.go:2081-2097` |
| `PublishMixMessages(ctx, msgs...) error` | append mix invs to the trickle queue; Protocol error below pver 10. No caller in the tree | `p2p/peering.go:2101-2121` |
| `SendMessage(ctx, msg) error` | queue any message on the normal queue, waiting at most 30 s; does not observe peer disconnection | `p2p/peering.go:2126-2135` |
| `ReceivedOrphanHeader() error` | add 10 to the transient ban score; Protocol error above 100 | `p2p/peering.go:2154-2162` |
| `GetInitState(ctx, msg) (*wire.MsgInitState, error)` | synchronous `getinitstate`; §4.11 | `p2p/peering.go:2174-2210` |

#### Unexported functions carrying observable behaviour

| Function | Contract | Citation |
|---|---|---|
| `isCGNAT(ip)` | true for IPv4 100.64.0.0/10 | `p2p/peering.go:201-206` |
| `newNetAddress(addr, services)` | builds the `AddrYou` wire address; hides non-public addresses; §3.2 | `p2p/peering.go:208-228` |
| `handshake(ctx, lp, na, c)` | §4.3 | `p2p/peering.go:544-616` |
| `readMessages`, `writeMessages`, `sendInventory`, `sendWaitingInventory`, `pingPong`, `pong` | §4.4, §4.5 | `p2p/peering.go:449-542`, `794-862`, `945-964` |
| `received*` handlers (addr, block, cfilterv2, cfiltersv2, headers, notfound, tx, mix, getdata, initstate, getminingstate, getinitstate, pong, inv) | §4.5 | `p2p/peering.go:966-1381`, `2227-2247` |
| `addRequested*` / `deleteRequested*` | request-table bookkeeping | `p2p/peering.go:1007-1093`, `1219-1341` |
| `requestBlocks(reqs)` | sends the block `getdata` and resolves every request; §4.7 | `p2p/peering.go:1425-1502` |
| `sendMessageAck(ctx, msg)` | queue and wait until the socket write finished | `p2p/peering.go:2139-2150` |
| `writeMixMsgHash(msg)` | compute and cache the hash of a received mix message | `p2p/peering.go:1259-1265` |
| `invVecContainsTxOrMix` | relay-disabled check | `p2p/peering.go:2214-2224` |
| `UseLogger(l slog.Logger)` (exported) | sets the package logger (subsystem `PEER`) | `p2p/log.go:13-15` |

### 2.2 Package `spv`

#### Constants and variables

| Name | Value | Citation |
|---|---|---|
| `reqSvcs` | `wire.SFNodeNetwork` (1) | `spv/sync.go:37` |
| `targetPeerCount` | 8 | `spv/sync.go:40` |
| `minVersion` | 10 (`wire.MixVersion`) | `spv/sync.go:44` |
| `minVersionTarget` | 3 | `spv/sync.go:45` |
| `errBreaksMinVersionTarget` | `"peer uses too low version to satisfy minimum target version count"` | `spv/sync.go:508-509` |
| `hashStop` | zero hash | `spv/sync.go:1611` |
| `errCfilterWatchdogTriggered` | `"getCFilters watchdog triggered"` | `spv/backend.go:145` |

#### Types

| Type | Contract | Citation |
|---|---|---|
| `Syncer` | SPV synchronizer; implements `wallet.NetworkBackend` (`var _ wallet.NetworkBackend = (*Syncer)(nil)`) | `spv/sync.go:55-106`, `spv/backend.go:28` |
| `Notifications` | optional callbacks: `Synced(bool)`, `PeerConnected(count int32, addr string)`, `PeerDisconnected(count int32, addr string)`, `FetchMissingCFiltersStarted()`, `FetchMissingCFiltersProgress(start, end int32)`, `FetchMissingCFiltersFinished()`, `FetchHeadersStarted()`, `FetchHeadersProgress(lastHeaderHeight int32, lastHeaderTime int64)`, `FetchHeadersFinished()`, `DiscoverAddressesStarted()`, `DiscoverAddressesFinished()`, `RescanStarted()`, `RescanProgress(rescannedThrough int32)`, `RescanFinished()`, `MempoolTxs([]*wire.MsgTx)`, `TipChanged(tip *wire.BlockHeader, reorgDepth int32, txs []*wire.MsgTx)`. Each is skipped when the struct or the field is nil | `spv/sync.go:110-138`, `225-318` |
| `backoff` (unexported) | `{duration time.Duration; time time.Time}` per-address reconnect state | `spv/sync.go:48-51` |
| `headersBatch` (unexported) | `{done bool; nodes, bestChain []*wallet.BlockNode; rp *p2p.RemotePeer}` | `spv/backend.go:265-270` |
| `filterProof` (alias) | same anonymous struct as `wallet.FilterProof` | `spv/backend.go:115-119` |

#### Exported functions and methods

| Signature | Contract | Citation |
|---|---|---|
| `NewSyncer(w *wallet.Wallet, lp *p2p.LocalPeer) *Syncer` | `discoverAccounts = !w.Locked()`; `seenTxs`/`seenMixMsgs` LRUs of limit 2000; empty rescan filter | `spv/sync.go:141-156` |
| `(*Syncer) SetPersistentPeers(peers []string)` | non-empty list disables seeding and candidate discovery | `spv/sync.go:160-162` |
| `(*Syncer) SetNotifications(*Notifications)` | | `spv/sync.go:166-168` |
| `(*Syncer) DisableDiscoverAccounts()` | effective only before the discovery step of `Run`. No caller in the tree | `spv/sync.go:172-174` |
| `(*Syncer) Synced(ctx) (bool, int32)` | synced flag; target height is the wallet tip height when synced, otherwise the maximum `InitialHeight` over connected peers (0 with none) | `spv/sync.go:200-215` |
| `(*Syncer) GetRemotePeers() map[string]*p2p.RemotePeer` | copy of the connected-peer map keyed by the dialed address string | `spv/sync.go:218-223` |
| `(*Syncer) Run(ctx) error` | runs until failure or cancellation; one call per `Syncer`; §4.12 | `spv/sync.go:336-466` |
| `(*Syncer) GetInitState(ctx, rp) error` | fetch and cache unseen treasury spends from one peer; §4.19 | `spv/sync.go:1016-1058` |
| `(*Syncer) Blocks(ctx, hashes) ([]*wire.MsgBlock, error)` | `NetworkBackend`; §4.21 | `spv/backend.go:95-111` |
| `(*Syncer) CFiltersV2(ctx, hashes) ([]filterProof, error)` | `NetworkBackend`; §4.21 | `spv/backend.go:124-141` |
| `(*Syncer) String() string` | `"spv.Syncer"` | `spv/backend.go:404-409` |
| `(*Syncer) LoadTxFilter(ctx, reload bool, addrs []stdaddr.Address, outpoints []wire.OutPoint) error` | `NetworkBackend`; §4.22 | `spv/backend.go:417-433` |
| `(*Syncer) PublishTransactions(ctx, txs...) error` | `NetworkBackend`; §4.23 | `spv/backend.go:437-475` |
| `(*Syncer) PublishMixMessages(ctx, msgs...) error` | `NetworkBackend`; §4.23 | `spv/backend.go:479-520` |
| `(*Syncer) Rescan(ctx, blockHashes []chainhash.Hash, save func(*chainhash.Hash, []*wire.MsgTx) error) error` | `NetworkBackend`; §4.24 | `spv/backend.go:523-624` |
| `(*Syncer) StakeDifficulty(ctx) (dcrutil.Amount, error)` | always `0, errors.E(errors.Invalid, "stake difficulty is not queryable over wire protocol")` | `spv/backend.go:633-635` |
| `(*Syncer) WrapContext(ctx) (context.Context, context.CancelFunc)` | child context cancelled when `Run` returns | `spv/backend.go:640-652` |
| `UseLogger(logger slog.Logger)` | sets the package logger (subsystem `SYNC`, shared with the dcrd-RPC syncer) | `spv/log.go:12-14` |

#### Unexported functions carrying observable behaviour

| Function | Section | Citation |
|---|---|---|
| `synced`, `unsynced` | §4.12 | `spv/sync.go:178-196` |
| notification helpers (`peerConnected` … `tipChanged`) | §4.26 | `spv/sync.go:226-318` |
| `setRequiredHeight` | §4.13 | `spv/sync.go:323-329` |
| `peerCandidate`, `connectAndRunPeer`, `breaksMinVersionTarget`, `nextBackoff`, `backoffJitter`, `backoff`, `setBackoff`, `connectToPersistent`, `connectToCandidates` | §4.13 | `spv/sync.go:471-714` |
| `forRemotes`, `waitForRemote` | §4.14 | `spv/sync.go:716-773` |
| `receiveGetData`, `receiveInv`, `receiveHeadersAnnouncements` | §4.17, §4.18, §4.16 | `spv/sync.go:779-951`, `1264-1288` |
| `verifyTSpendSignature`, `checkTSpend` | §4.19 | `spv/sync.go:955-1012` |
| `handleBlockInvs`, `handleTxInvs`, `handleMixInvs` | §4.18 | `spv/sync.go:1060-1259` |
| `scanChain`, `handleBlockAnnouncements`, `disconnectStragglers` | §4.16 | `spv/sync.go:1293-1607` |
| `initialSyncHeaders`, `initialSyncRescan`, `peerStartup`, `handleMempool` | §4.15, §4.20, §4.25 | `spv/sync.go:1616-1980` |
| `pickAny`, `pickForGetHeaders`, `pickForGetCfilters`, `blocksFromPeer`, `cfiltersV2FromNodes`, `getHeaders` | §4.14, §4.15, §4.21 | `spv/backend.go:35-402` |
| `rescanCheckTransactions`, `rescanBlock`, `filterRelevant` | §4.24 | `spv/rescan.go:20-115` |

### 2.3 Package `lru`

| Signature | Contract | Citation |
|---|---|---|
| `type Cache[T comparable]` | mutex-guarded set with recency list; zero value invalid | `lru/cache.go:17-22` |
| `NewCache[T](limit int) Cache[T]` | returns by value (callers must not copy after use) | `lru/cache.go:25-31` |
| `(*Cache[T]) Add(item T)` | present: move to front. Absent: if `len > limit` evict the back item, then push front. Holds up to `limit+1` items | `lru/cache.go:36-59` |
| `(*Cache[T]) Contains(v T) bool` | membership; does not touch recency | `lru/cache.go:62-67` |
| `type Map[K comparable, V any]` | same structure with values | `lru/map.go:22-27` |
| `NewMap[K,V](limit int) Map[K,V]` | | `lru/map.go:30-36` |
| `(*Map) Add(key, value)` | present: move to front **without replacing the stored value**. Absent: same eviction rule as `Cache.Add` | `lru/map.go:41-65` |
| `(*Map) Get(key) (V, bool)` | returns value and moves to front | `lru/map.go:70-82` |
| `(*Map) Hit(key) bool` | `Get` discarding the value | `lru/map.go:89-92` |
| `(*Map) Contains(key) bool` | membership without recency change (the doc comment says "It does modify the priority"; the code does not) | `lru/map.go:96-101` |

`lru.Map` has no user anywhere in dcrwallet at the pin; `lru.Cache` is used only by `p2p` and
`spv`.

### 2.4 dcrd `addrmgr` v3.0.0 API as used

| Call | Used from | Behaviour relied on |
|---|---|---|
| `addrmgr.New(dataDir)` | `dcrwallet.go:534`, `rpcserver/server.go:3015` | `peersFile = dataDir/peers.json`; fresh random 32-byte key; empty buckets |
| `Start()` | `spv/sync.go:360` | first call only: load `peers.json`, start the 10-minute dump loop |
| `Stop() error` | `spv/sync.go:362` | first call only: stop the loop and write `peers.json` once more; later calls log a warning and return nil |
| `AddAddresses(addrs, src)` | seeding, `addr` handling | adds routable addresses to "new" buckets |
| `GetAddress() *KnownAddress` | `spv/sync.go:475` | weighted random pick; nil when empty |
| `KnownAddress.NetAddress()`, `.LastAttempt()` | `spv/sync.go:479,492` | |
| `NetAddress.Key()` / `.String()` | map keys, dial string | `ip:port` via `net.JoinHostPort` (IPv6 bracketed) |
| `Attempt(na)`, `Connected(na)`, `Good(na)` | `p2p/peering.go:620,630,297` | return `ErrAddressNotFound` for addresses not in the manager; the wallet ignores every return value |
| `NeedMoreAddresses() bool` | `p2p/peering.go:299` | `nTried + nNew < 1000` |
| `NewNetAddressFromIPPort(ip, port, services)` | several | timestamp = now truncated to seconds; type derived from the bytes |
| `UseLogger` | `log.go:40` | subsystem `AMGR` |

Not used by the wallet: `AddressCache`, `SetServices`, local-address functions,
`IsExternalAddrCandidate`, `EncodeHost` (only indirectly through deserialization).

The internal algorithms behind these calls (admission, bucket hashing, expiry, selection
weights, promotion to "tried", save/load lifecycle) are specified in §4.27; the ban score
arithmetic behind §2.5 is in §4.28.

### 2.5 dcrd `connmgr` v3.1.4 API as used

Only `connmgr.DynamicBanScore` (`Increase(persistent, transient uint32) uint32`, `Int() uint32`)
from `p2p`, `connmgr.TorLookupIP` from `config.go:731` (configuration spec), and
`connmgr.UseLogger(CmgrLog)` from `log.go:41` (subsystem `CMGR`). The connection
manager itself (`ConnManager`, `ConnReq`, seeding helpers) is not used.

---

## 3. Data formats

Nothing in scope is written to `wallet.db`. The persisted artefact is `peers.json`; everything
else is transmitted.

### 3.1 Wire message framing

All peer traffic uses dcrd `wire` v1.8.0 framing (`dcrd/wire@v1.8.0/message.go:243-316`,
`362-495`). The codecs themselves are supplied by the wire crate; the layout is restated here
because the wallet's behaviour on malformed frames is observable.

| Offset | Size | Field | Encoding |
|---|---|---|---|
| 0 | 4 | network magic | uint32 LE: mainnet `0xd9b400f9`, testnet3 `0xb194aa75`, simnet `0x12141c16`, regnet `0xdab500fa` (`dcrd/wire@v1.8.0/protocol.go:131-146`) |
| 4 | 12 | command | ASCII, NUL padded |
| 16 | 4 | payload length | uint32 LE |
| 20 | 4 | checksum | first 4 bytes of BLAKE-256(payload) |
| 24 | n | payload | per message |

Read-side rejections, each of which ends the peer's read loop and therefore disconnects the
peer (§4.4): payload length > 32 MiB; magic mismatch; non-ASCII command; **unknown command**
(`dcrd/wire@v1.8.0/message.go:233-236`); payload longer than the type's maximum at the
negotiated version; short read; checksum mismatch; payload decode error; trailing bytes.
`reject` is not a known command in wire v1.8.0, so a received `reject` disconnects the peer.

All messages after the handshake are encoded and decoded at the **negotiated** protocol version
`rp.pver` (`p2p/peering.go:453`, `795`). The remote `version` message is decoded at version 11
(`p2p/peering.go:585`).

### 3.2 The `version` message as sent

Built by `newMsgVersion` (`p2p/peering.go:230-242`) from `wire.NewMsgVersion`
(`dcrd/wire@v1.8.0/msgversion.go:235-250`) and encoded by `MsgVersion.BtcEncode`
(`dcrd/wire@v1.8.0/msgversion.go:161-199`).

| Offset | Size | Field | Value sent by the wallet |
|---|---|---|---|
| 0 | 4 | ProtocolVersion | int32 LE = 11 |
| 4 | 8 | Services | uint64 LE = 0 |
| 12 | 8 | Timestamp | int64 LE, current Unix seconds |
| 20 | 8 | AddrYou.Services | uint64 LE = 0 |
| 28 | 16 | AddrYou.IP | remote IP as 16 bytes (IPv4 as `::ffff:a.b.c.d`), or 16 zero bytes when hidden |
| 44 | 2 | AddrYou.Port | uint16 **big endian**, remote port, or 0 when hidden |
| 46 | 8 | AddrMe.Services | 0 |
| 54 | 16 | AddrMe.IP | 16 zero bytes |
| 70 | 2 | AddrMe.Port | 0 |
| 72 | 8 | Nonce | uint64 LE, uniformly random per connection |
| 80 | 1+L | UserAgent | varint length + ASCII |
| 81+L | 4 | LastBlock | int32 LE = 0 |
| 85+L | 1 | relay | `0x01`, or `0x00` when `--spvdisablerelaytx` is set |

Field notes:

- **UserAgent** is `"/dcrwire:1.0.0/" + "dcrwallet:" + uaVersion + "/"`, i.e.
  `/dcrwire:1.0.0/dcrwallet:2.2.0-pre+<rev9>/` for a git build of the pin and
  `/dcrwire:1.0.0/dcrwallet:2.2.0-pre/` without VCS build info
  (`dcrd/wire@v1.8.0/msgversion.go:22`, `295-310`; `version/version.go`). A drop-in
  replacement must emit the same `dcrwallet:<semver>` token with its own version string in the
  same grammar (`MAJOR.MINOR.PATCH[-pre][+build]`, characters limited to `[0-9A-Za-z.-]`).
  L is 45 with a 9-character revision (payload 131 bytes) and 35 without (payload 121 bytes).
  The error return of `AddUserAgent` is ignored.
- **AddrYou** is derived from `c.RemoteAddr()` by `newNetAddress`
  (`p2p/peering.go:208-228`): for `*net.TCPAddr` its IP and port; for `*socks.ProxiedAddr`
  `net.ParseIP(Host)` (nil for a hostname) and `Port`; any other address type is an error
  `newNetAddress: unsupported address type %T` that fails the handshake. The IP and port are
  then replaced by nil/0 when the IP is loopback, RFC 1918 / RFC 4193 private
  (`net.IP.IsPrivate`), not a global unicast address, or in 100.64.0.0/10.
- **AddrMe** is a zero `wire.NetAddress`; the wallet never reveals a local address.
- **Services** 0: the wallet advertises no services. **LastBlock** is always 0 regardless of
  the wallet tip.
- **Nonce** is not remembered; there is no self-connection detection.

### 3.3 Messages sent and handled

"Sent" lists every message the wallet can put on the wire; "Handled" lists what happens to
each message type the wire package can decode. Payload layouts are those of wire v1.8.0.

#### Sent

| Command | When | Payload as sent |
|---|---|---|
| `version` | first write on every connection | §3.2 |
| `verack` | after the remote `version` is accepted | empty |
| `getaddr` | once after a successful connect when the address manager holds < 1000 addresses | empty |
| `ping` | every 2 minutes after the previous ping exchange completed | 8-byte random nonce LE; priority queue |
| `pong` | reply to `ping` | echoed nonce; priority queue |
| `getheaders` | initial sync (synchronous); after `sendheaders` at peer startup and on orphan announcements (asynchronous) | uint32 LE `ProtocolVersion` = negotiated pver; varint count; locator hashes (§3.4); 32-byte zero `HashStop` |
| `sendheaders` | once per peer after initial sync | empty |
| `getdata` | blocks (type 2), transactions (type 1), mix messages (type 4) | varint count; per entry uint32 LE type + 32-byte hash. One message per request call; never mixes types |
| `getcfilterv2` | one per block hash | 32-byte block hash |
| `getcfsv2` | per batch of ≤ 100 consecutive blocks, only to peers with pver ≥ 11 | 32-byte start hash, 32-byte end hash |
| `getinitstate` | once per peer after initial sync | `01 07 "tspends"` (varint count 1, var-string `tspends`) |
| `inv` | trickled wallet tx announcements at peer startup; immediate tx and mix announcements from `PublishTransactions` / `PublishMixMessages` | varint count; entries of type 1 (tx) or 4 (mix) |
| `tx` | reply to `getdata` for a hash previously announced to that peer | serialized transaction |
| `mixpairreq`, `mixkeyxchg`, `mixcphrtxt`, `mixslotres`, `mixfactpoly`, `mixdcnet`, `mixconfirm`, `mixsecrets` | reply to `getdata` type 4 for a hash previously announced to that peer | the mix message |
| `notfound` | reply to `getdata` for everything not served | varint count; the unserved inv vectors (original type preserved, except mixpool misses which are rebuilt as type 4) |
| `miningstate` | reply to `getminings` | `01 00 00 00` version 1, `00 00 00 00` height 0, `00` blocks, `00` votes (10 bytes) |
| `initstate` | reply to `getinitstate` | `00 00 00` (no blocks, votes, tspends) |

Never sent: `addr`, `addrv2`, `getblocks`, `block`, `headers`, `mempool`, `feefilter`,
`cfilterv2`, `cfiltersv2`, `reject`, and the retired v1 cfilter family.

#### Handled (after the handshake)

| Received | Action | Citation |
|---|---|---|
| `version` | read loop returns Protocol error `received unexpected version message`; disconnect | `p2p/peering.go:798-801` |
| `verack` | ignored (never waited for) | `p2p/peering.go:606`, `803-849` |
| `addr` | every entry converted with `addrmgr.NewNetAddressFromIPPort(IP, Port, Services)`, timestamp copied, then `AddAddresses(list, peer.na)` | `p2p/peering.go:973-981` |
| `block` | resolves the matching block request; unrequested → disconnect | `p2p/peering.go:983-1005` |
| `cfilterv2` | resolves the request keyed by `BlockHash`; unrequested → disconnect | `p2p/peering.go:1052-1070` |
| `cfiltersv2` | empty → disconnect; else resolves the request keyed by `CFilters[0].BlockHash`; unrequested → disconnect | `p2p/peering.go:1016-1041` |
| `notfound` | closes matching tx requests; any other hash → disconnect | `p2p/peering.go:1194-1217` |
| `tx` | resolves the request keyed by tx hash; unrequested → disconnect | `p2p/peering.go:1235-1252` |
| `getdata` | ban-score accounting, then forwarded to the syncer when `MaskGetData` is set | `p2p/peering.go:1302-1324` |
| `headers` | validated, then delivered to the pending `Headers` call or, in sendheaders mode, to `ReceiveHeadersAnnouncement` | `p2p/peering.go:1095-1192` |
| `inv` | relay-disabled check, then forwarded when `MaskInv` is set | `p2p/peering.go:2227-2247` |
| `getminings` | reply with empty `miningstate` | `p2p/peering.go:1363-1371` |
| `getinitstate` | reply with empty `initstate` | `p2p/peering.go:1373-1381` |
| `initstate` | resolves the pending `GetInitState`; unrequested → disconnect | `p2p/peering.go:1343-1361` |
| `ping` | reply `pong` on the priority queue | `p2p/peering.go:855-862` |
| `pong` | delivered to the ping waiter (buffer of 1) | `p2p/peering.go:966-971` |
| 8 mix message types | resolves the request keyed by the message hash; unrequested → disconnect | `p2p/peering.go:1283-1300` |
| `getaddr`, `getblocks`, `getheaders`, `mempool`, `miningstate`, `sendheaders`, `feefilter`, `getcfilterv2`, `getcfsv2`, v1 cfilter family | decoded and silently dropped. Exception: the wire decoders gate by the negotiated version, so `getcfsv2` and `cfiltersv2` below pver 11 and any mix message below pver 10 fail to decode → read error → disconnect (`dcrd/wire@v1.8.0/msggetcfsv2.go:31`, `msgcfiltersv2.go:76`, `msgmixpairreq.go:114,401`) | `p2p/peering.go:803-849` |
| `addrv2` | cannot be decoded below pver 12 (max payload 0) → read error → disconnect | `dcrd/wire@v1.8.0/msgaddrv2.go:233-236` |
| `reject` or any unknown command | read error → disconnect | `dcrd/wire@v1.8.0/message.go:233-236` |

### 3.4 Block locators in `getheaders`

Locators come from `wallet.BlockLocators(ctx, sidechain)` (`wallet/wallet.go:1482-1535`). The
construction is wire-observable and is restated here; ownership is the wallet chain spec.

1. `hash, height` = last node of `sidechain` if non-empty, else the wallet main-chain tip.
2. `locators = [hash]`.
3. `step = 1`. While `height >= 0`: append the hash at `height` (from `sidechain` when
   `height >= sidechain[0].Height`, else the main-chain hash at that height); `height -= step`;
   if `len(locators) > 10`, `step *= 2`.
4. Return `locators` and the starting height.

Consequences a peer can see: the first two locators are always the same hash; entries 1..10
are consecutive heights `h, h-1, …, h-9`; then `h-10, h-12, h-16, h-24, h-40, …` with the gap
doubling; the genesis block is included only when the descent lands on height 0 exactly.
`HashStop` is always the zero hash. The `ProtocolVersion` field of `getheaders` is the
negotiated version, not 11.

### 3.5 HTTPS seeder request and response

Request (`p2p/peering.go:348-383`):

- `GET https://<seeder>/api/addrs?services=<n>` with `n` the decimal service flags; the
  syncer always passes `SFNodeNetwork`, so the query is `services=1`. No other query
  parameters.
- Seeder hosts (`dcrd/chaincfg/v3@v3.3.0/mainnetparams.go:604-608`,
  `testnetparams.go:575-579`): mainnet `mainnet-seed-1.decred.org`,
  `mainnet-seed-2.decred.org`, `mainnet-seed.jholdstock.uk`; testnet3
  `testnet-seed-1.decred.org`, `testnet-seed-2.decred.org`, `testnet-seed.jholdstock.uk`;
  simnet and regnet none.
- The HTTP client is a Go `http.Client` whose `Transport` has only `DialContext = lp.dial`
  set: default TLS verification against system roots, HTTP/1.1 only, no environment proxy,
  Go's default request headers (`User-Agent: Go-http-client/1.1`, `Accept-Encoding: gzip` with
  transparent decompression). The seeder host name is passed to the dial function unresolved,
  so with `--proxy` it is resolved by the proxy.
- Per-request timeout 1 minute (context deadline covering connect, headers and body).

Response handling (`p2p/peering.go:384-433`):

- The HTTP status code is **not** checked.
- At most 4096 bytes of body are read.
- The body is parsed as a **stream of concatenated JSON objects** (not an array), each
  `{"host": "<ip>:<port>", "services": <uint64>}`. Unknown fields are ignored. At most 16
  objects are decoded per seeder.
- `host` must split as host:port, the host must be an IP literal, and the port must parse as
  a 16-bit unsigned decimal; otherwise the entry is skipped with a warning.
- The `services` value in the response is ignored; each address is recorded with the
  `services` argument of `SeedPeers` (1).
- The decoded struct is reused between objects without being cleared, so a later object
  lacking `host` inherits the previous object's value.

### 3.6 `peers.json`

**Location.** CLI: `<appdata>/<netname>/peers.json` where `<netname>` is the chain params
name (`mainnet`, `testnet3`, `simnet`) (`dcrwallet.go:533-534`). gRPC `SpvSync`:
`<loader DbDirPath>/peers.json` (`internal/rpc/rpcserver/server.go:3015`), which is the same
network directory when the loader is configured by the daemon. Temp file `peers.json.new` in
the same directory, written then renamed over the target
(`dcrd/addrmgr/v3@v3.0.0/addrmanager.go:467-486`).

**Encoding.** One JSON object followed by `\n`, produced by Go `encoding/json` from
`serializedAddrManager` (`dcrd/addrmgr/v3@v3.0.0/addrmanager.go:102-119`), fields in this
order:

| Key | JSON type | Contents |
|---|---|---|
| `Version` | number | `1` |
| `Key` | array of 32 numbers (0-255) | bucket-hash key (a Go byte **array**, hence not base64) |
| `Addresses` | array of objects | one per known address, in Go map iteration order |
| `NewBuckets` | array of exactly 1024 arrays of strings | address keys per new bucket (order = map iteration); empty buckets are `[]` |
| `TriedBuckets` | array of exactly 64 arrays of strings | address keys per tried bucket, in slice order |

Each `Addresses` element, fields in this order:

| Key | JSON type | Contents |
|---|---|---|
| `Addr` | string | address key `ip:port` (`net.JoinHostPort`: IPv6 as `[addr]:port`) |
| `Src` | string | key of the source address |
| `Attempts` | number | failed-attempt counter |
| `TimeStamp` | number | address timestamp, Unix seconds |
| `LastAttempt` | number | Unix seconds; a never-attempted address stores Go's zero time, `-62135596800` |
| `LastSuccess` | number | Unix seconds; never-succeeded is `-62135596800` |

**Loading** (`dcrd/addrmgr/v3@v3.0.0/addrmanager.go:493-601`):

1. Missing file → empty manager, no error.
2. Decode one JSON value; `Version != 1` → error.
3. Copy `Key`. For each address, parse `Addr` and `Src` with `net.SplitHostPort` + IP literal
   parse (non-IP hosts fail); create the known address with `Attempts`, `LastAttempt`,
   `LastSuccess`. **`TimeStamp` is not applied**: the address timestamp becomes the load time,
   and services become `SFNodeNetwork`.
4. For each new bucket entry: must exist in the address list; increments its reference count
   (and the new-address total on first reference).
5. For each tried bucket entry: must exist; marks it tried and appends to the bucket.
6. Every address must be referenced by a new bucket xor be tried.
7. Any error: log `Failed to parse file <path>: <err>`, delete the file, reset to an empty
   manager with a fresh random key. The new/tried counters are **not** reset by `reset()`
   (see Q-20).
8. Success logs `Loaded <n> addresses from file '<path>'` at info level.

**Saving** happens every 10 minutes and once at `Stop`, only when something changed since the
last save (`dcrd/addrmgr/v3@v3.0.0/addrmanager.go:401-488`). Service flags are not persisted.

### 3.7 Peer fields exposed to RPC clients

`getpeerinfo` (JSON-RPC and gRPC) projects each connected peer as follows; the JSON field
names and ordering belong to the RPC specs.

| Field | Source | Notes |
|---|---|---|
| id | `int32(rp.ID())` | 64-bit counter truncated |
| addr | `rp.RemoteAddr().String()` | with a SOCKS proxy this is `Host:Port` from `socks.ProxiedAddr` (IPv6 not bracketed) |
| addrlocal | `rp.LocalAddr().String()` | with a SOCKS proxy, the proxy's reported bound address when present |
| services | `fmt.Sprintf("%08d", uint64(rp.Services()))` | zero-padded decimal, e.g. `00000001` |
| version | `rp.Pver()` | negotiated version |
| subver | `rp.UA()` | |
| startingheight | `int64(rp.InitialHeight())` | |
| banscore | `int32(rp.BanScore())` | |

Results are sorted by id ascending (`internal/rpc/jsonrpc/methods.go:2528-2537`).

---

## 4. Algorithms and state machines

Conventions: "disconnect(E)" means `rp.Disconnect(E)`; the first reason passed wins and is what
`rp.Err()` returns. "Stall" means the 30-second `stallTimeout`. "Queue" means sending on the
peer's unbuffered normal-priority channel `rp.out` (the write pump receives it); "priority
queue" is `rp.outPrio`. No step in §4.1-§4.11 opens a wallet database transaction.

### 4.1 `LocalPeer.SeedPeers(ctx, services)`

`p2p/peering.go:347-434`. Called by `Syncer.Run` before any peer goroutine starts, and blocks.

1. Build the URL template (§3.5) and an `http.Client` using `lp.dial`.
2. For each seeder host: derive a context with a 1-minute timeout; create the GET request
   (on failure log error `Bad seeder request: <err>` and continue); start a goroutine that
   performs the request and sends the response (or nil after logging warning
   `Failed to seed addresses from <host>: <err>`) on an unbuffered channel.
3. Receive exactly `len(seeders)` results, in completion order. For each non-nil response:
   decode up to 16 objects from the first 4096 body bytes. Stop at EOF; on a decode error log
   warning `Invalid seeder <host> API response: <err>` and stop. Per object apply the checks
   of §3.5 with warnings `Invalid host in seeder <host> API: <err>`,
   `Invalid IP address "<h>" in seeder <host> API host field`,
   `Invalid port "<p>" in seeder <host> API host field`. Log debug
   `Discovered peer <host:port> from seeder <host>` for each accepted address.
4. Close the body. If at least one address was accepted, call
   `amgr.AddAddresses(list, list[0])` — the source address is the first address of that
   seeder's list.
5. On return all per-request contexts are cancelled.

With no seeders (simnet, regnet) the function returns immediately.

### 4.2 `LocalPeer.ConnectOutbound(ctx, addr, reqSvcs)`

`p2p/peering.go:255-308`, `618-658`.

1. Log debug `Attempting connection to peer <addr>`.
2. `net.ResolveTCPAddr("tcp", addr)` using the system resolver (not the proxy); return its
   error unwrapped on failure.
3. `na = addrmgr.NewNetAddressFromIPPort(ip, port, SFNodeNetwork)`; `na.Timestamp = now`.
4. `connectOutbound`:
   1. `amgr.Attempt(na)` (result ignored; a no-op error for addresses the manager does not
      know, e.g. persistent peers).
   2. Dial `lp.dial(ctx10s, "tcp", addr)` with a 10-second timeout, passing the **original
      address string**. Return the error unwrapped on failure.
   3. `amgr.Connected(na)` (result ignored).
   4. `handshake` (§4.3) under a 5-second context. On failure return the error; the socket is
      not explicitly closed (Q-12).
   5. Under `rpMu`: increment `rpIDCounter` until the value is non-zero and unused; assign it
      as `rp.id`; insert into `rpByID`.
   6. `na.Services = rp.services` (local object only; the address manager's copy is not
      updated).
5. On any error from step 4 return `errors.E(Op("localpeer.ConnectOutbound(<addr>)"), err)`.
6. Start `serveUntilError(ctx, rp)` in the background (§4.4).
7. If `rp.services & reqSvcs != reqSvcs`: build
   `errors.E(Op("localpeer.ConnectOutbound(<raddr>)"), errors.Errorf("missing required service flags %v", reqSvcs &^ rp.services))`
   (the flag set prints via `wire.ServiceFlag.String`, e.g. `SFNodeNetwork`), disconnect with
   it, return it.
8. If `rp.initHeight < lp.requireHeight`: error
   `errors.E(Op("localpeer.ConnectOutbound(<raddr>)"), "peer is not synced")`, disconnect,
   return.
9. `amgr.Good(na)` (result ignored).
10. If `amgr.NeedMoreAddresses()`: `rp.Addrs(ctx)`; on error disconnect with it and return it.
11. Return `rp`.

### 4.3 Handshake

`p2p/peering.go:544-616`. Runs under the 5-second handshake context; op `p2p.handshake`.

1. Create the `RemotePeer` with `pver = 11`, `raddr = c.RemoteAddr()`, `pongs` channel of
   capacity 1, empty request maps, two inv LRUs with limit 5000.
2. Build the `version` message (§3.2). Failure → `errors.E(op, err)`.
3. Write it at pver 11. The write runs in a helper goroutine; a context expiry returns
   `ctx.Err()`. Failure → `errors.E(op, errors.IO, err)`.
4. Set a read deadline of now + 3 s (failure → IO error). Read one message decoded at pver
   11. Failure → `errors.E(op, errors.IO, err)`.
5. If it is not `version` → `errors.E(op, errors.Protocol, "first received message was not the version message")`.
6. Record `initHeight = LastBlock`, `services`, `ua`. Clear the read deadline.
7. If remote `ProtocolVersion` (as uint32) < 9 →
   `errors.E(op, errors.Protocol, "remote peer has pver lower than minimum required")`.
8. `rp.pver = min(11, remote ProtocolVersion)`.
9. Write `verack` at the negotiated version. Failure → IO error.
10. Create the two unbuffered outbound channels and return.

The remote `verack` is never awaited; it is consumed and ignored by the read loop. The remote
nonce, timestamp, addresses and relay flag are ignored. A peer that sends anything before its
`version` fails step 5.

### 4.4 Per-peer service goroutines

`serveUntilError(ctx, rp)` (`p2p/peering.go:660-716`) runs five tasks in an error group whose
context `gctx` derives from the caller's context (the syncer's group context):

| Task | Behaviour |
|---|---|
| closer | waits for `gctx` to end, then `Disconnect(gctx.Err())` and closes the socket |
| reader | `readMessages(gctx)` (below); a returned error is logged at debug as `remotepeer(<addr>).readMessages: <err>` unless `gctx` already ended |
| writer | `writeMessages(gctx)`; same logging with `writeMessages` |
| trickler | `sendInventory(gctx)`; same logging with `sendInventory` |
| pinger | loop: wait 2 minutes (or `gctx`), then `pingPong` with a 15-second deadline |

When the group finishes with an error, `Disconnect(err)`. Finally log debug
`Disconnected from outbound peer <addr>` and remove the peer from `rpByID`.

Any task error cancels `gctx`, which makes the closer disconnect and close the socket.
`Disconnect` called from elsewhere closes the socket, which makes the reader fail, which
cancels `gctx`. The reason recorded is whichever `Disconnect` call ran first.

**Reader** (`p2p/peering.go:794-853`). Loop: read one message at `rp.pver`; log debug
`<command> <- <addr>`; a `version` message returns
`errors.E(errors.Protocol, "received unexpected version message")`; otherwise **spawn a new
goroutine** that dispatches the message (§4.5) and continue reading. A read error ends the
loop and is returned. Because every message is dispatched on its own goroutine, handlers for
consecutive messages are not ordered relative to each other.

**Writer** (`p2p/peering.go:449-482`). An inner goroutine loops forever: take a message from
the priority queue if one is ready, otherwise wait on both queues; log debug
`<command> -> <addr>`; write it at `rp.pver`; signal the ack channel if present; on a write
error report it and stop. The outer function returns the write error or `ctx.Err()`.
A message that cannot be encoded at the negotiated version (for example a mix message below
version 10) is a write error and therefore disconnects the peer.

**Trickler** (`p2p/peering.go:502-542`). A timer fires after a uniformly random duration in
[100 ms, 500 ms) and is re-armed with a new random duration each time. On each tick, under
`waitingInvsMu`: send the queued inv vectors in `inv` messages of at most 50 000 entries each
using `sendMessageAck` (queue with a 30-second timeout, then wait for the write to complete);
on success truncate the queue. Nothing is sent when the queue is empty. A send error is
returned and disconnects the peer; the queue is left intact. "Success" here means only that
the write pump took the message and signalled the ack: the pump signals the ack **before**
it examines the write result (`p2p/peering.go:466-473`), so `sendMessageAck` returns nil for
a failed socket write as well and the queue is truncated; the failure surfaces through the
writer task instead. The ack channel has capacity 1, so the pump never blocks on it.
`RemotePeer.PublishTransactions`/`PublishMixMessages` block on `waitingInvsMu` while a
trickle send is in progress.

**Pinger** (`p2p/peering.go:700-711`, `945-964`). `pingPong`: pick a random 64-bit nonce;
put `ping` on the priority queue; wait for a value on `rp.pongs` or the 15-second deadline.
Deadline → disconnect(`errors.E(errors.IO, "ping timeout")`). A pong whose nonce differs →
disconnect(`errors.E(errors.Protocol, "pong contains nonmatching nonce")`). The 15 seconds
also cover the time spent queueing the `ping`; if the deadline passes before the ping could be
queued, `pingPong` returns without disconnecting. The next 2-minute wait starts after
`pingPong` returns.

### 4.5 Inbound message handlers

All in `p2p/peering.go`. Op strings use the remote address text.

**`ping`** (`855-862`): put `pong(nonce)` on the priority queue, giving up after 5 minutes or
when the peer context ends.

**`pong`** (`966-971`): send to `rp.pongs` (capacity 1) or give up when the context ends. An
unsolicited pong occupies the buffer and is consumed by the next ping cycle, where its nonce
mismatches and the peer is disconnected (Q-09).

**`addr`** (`973-981`): §3.3.

**`block`** (`983-1005`): under `requestedBlocksMu` look up the header hash. Absent →
disconnect(`errors.E(Op("remotepeer(<addr>).receivedBlock(<hash>)"), errors.Protocol, "received unrequested block")`).
Present and unresolved → store the block and close `ready`. Already resolved → ignore.

**`cfilterv2`** (`1052-1070`): log debug `Received cfilter for block <hash>`. Look up
`BlockHash` in the single-filter request table. Absent →
disconnect(Protocol, `received unrequested cfilter`, op `remotepeer(<addr>).receivedCFilterV2(<hash>)`).
Present → delete the entry and deliver the message.

**`cfiltersv2`** (`1016-1041`): zero filters →
disconnect(Protocol, `received empty cfiltersv2 message`). Look up `CFilters[0].BlockHash`
in the batched request table. Absent → disconnect(Protocol, `received unrequested many cfilter`).
Present → delete and deliver.

**`headers`** (`1095-1192`), under `requestedHeadersMu`, checks in this order; each failure
is disconnect(`errors.E(Op("remotepeer(<addr>).receivedHeaders"), errors.Protocol, <text>)`):

1. Neither sendheaders mode nor a pending synchronous request → `received unrequested headers`.
2. `0 < len < 2000` and the last header's height < `uint32(rp.initHeight)` →
   `peer sent too few headers`. (Applies to announcements too.)
3. A synchronous request recorded locators and `len > 0`: the first header's `PrevBlock` must
   equal one of them → else `peer sent headers that do not connect to block locators`.
   (`HeadersAsync` records no locators, so its replies skip this check.)
4. For each header after the first: `PrevBlock` equals the previous header's hash and
   `Height == previous + 1` → else `received out-of-sequence headers`.
5. If the last header's height > 0, raise `rp.lastHeight` to it if greater.
6. Sendheaders mode: release the lock and deliver `(rp, msg)` to the local peer's
   announcement channel (blocking until the syncer receives it or the context ends).
   Otherwise: take and clear the pending request (channel and locators), release the lock,
   deliver.

An empty `headers` message is still subject to check 1 (unrequested → disconnect); it passes
checks 2-4, skips step 5, and is delivered by step 6 (an empty announcement, or an empty reply
that completes the pending `Headers` call).

**`notfound`** (`1194-1217`): for each inv vector, **regardless of its type**, remove the hash
from the transaction request table; if it was present close its channel. If any vector's hash
was not a pending transaction request, after processing all vectors
disconnect(`errors.E(errors.Protocol, "received notfound for unrequested hash")`). The op
string is built with `errors.Errorf` instead of `errors.Opf` and is overwritten, so the error
text is exactly `protocol violation: received notfound for unrequested hash`. `notfound` for a
requested block or a requested mix message therefore disconnects the peer (Q-07).

**`tx`** (`1235-1252`): look up and remove `TxHash()`. Absent →
disconnect(Protocol, `received unrequested tx`, op `remotepeer(<addr>).receivedTx(<hash>)`).
Present → deliver.

**mix messages** (`1283-1300`): compute the message hash with the shared BLAKE-256 hasher
(`WriteHash` then `Hash`), look up and remove it. Absent →
disconnect(Protocol, `received unrequested mix msg`, op `remotepeer(<addr>).receivedMixMsg(<hash>)`).
Present → deliver.

**`getdata`** (`1302-1324`): `delta = len(InvList) * 100 / 50000` (integer division: 0 below
500 entries, 100 at 50 000). `score = banScore.Increase(0, delta)`. If `score > 100`: log
warning `<addr>: ban score reached threshold` and
disconnect(`errors.E(errors.Protocol, "ban score reached")`). Otherwise, if `MaskGetData` is
set, send `(rp, msg)` on the getdata channel with a plain blocking send.

**`inv`** (`2227-2247`): if relay is disabled and any vector is a tx or mix type →
disconnect(Protocol, `received tx in msginv when tx relaying is disabled`, op
`remotepeer(<addr>).receivedInv`). Else if `MaskInv` is set, forward (or give up when the
context ends).

**`getminings`**, **`getinitstate`** (`1363-1381`): queue the empty reply (§3.3) unless the
context ends or the peer disconnects first.

**`initstate`** (`1343-1361`): take and clear the pending request. None →
disconnect(Protocol, `received unrequested init state`, op `remotepeer(<addr>).receivedInitState`).
Else deliver.

### 4.6 `RemotePeer.Addrs`

`p2p/peering.go:1388-1408`. Queue `getaddr` under a 30-second timeout. Timeout →
disconnect and return `errors.E(Op("remotepeer(<addr>).Addrs"), errors.IO, "peer appears stalled")`;
outer cancellation → `ctx.Err()`; peer already gone → its disconnect reason. No reply is
awaited.

### 4.7 `RemotePeer.Blocks`

`p2p/peering.go:1425-1563`.

1. Under `requestedBlocksMu`, for each hash reuse the existing in-flight request or create
   and register a new one.
2. If any are new, start `requestBlocks(new)` in a goroutine:
   1. Build one `getdata` with a type-2 vector per new hash. An `AddInvVect` failure (more
      than 50 000) fails all new requests with that error.
   2. Start a 30-second stall timer. Queue the message; if the timer fires first →
      disconnect(`errors.E(errors.IO, "peer appears stalled")`) and fail all; if the peer
      disconnects first → fail all with its reason.
   3. For each new request **in request order**, wait for it to resolve, for the stall timer,
      or for peer disconnect. Each resolution re-arms the timer to 30 s. The timer therefore
      bounds the gap between consecutive requested blocks in the order requested, not arrival
      of any block.
   4. Finally mark any unresolved request with the outcome and remove all of them from the
      table.
3. Wait, in input order, for every request (new or joined). A request error returns
   `errors.E(Op("remotepeer(<addr>).Blocks"), err)`; context cancellation returns
   `errors.E(op, ctx.Err())` (the background fetch continues).
4. Wait for `requestBlocks` to finish, then return the blocks in input order.

### 4.8 `RemotePeer.Transactions` and `MixMessages`

`p2p/peering.go:1575-1707`. Identical apart from the inv type (1 vs 4), the request table and
the "already requested" text.

1. For each hash in order: add a vector to one `getdata` (failure → `errors.E(op, err)`);
   register a capacity-1 result channel. If the hash is already registered for this peer
   (including a duplicate within the same call), unregister the hashes registered so far and
   return `errors.E(op, errors.Errorf("tx %v is already being requested from this peer", h))`
   (mix: `mix msg %v is already being requested from this peer`). Ops:
   `remotepeer(<addr>).Transactions`, `remotepeer(<addr>).MixMessages`.
2. Queue the `getdata`. Context end → unregister all, return `ctx.Err()`. (Peer disconnection
   is not observed at this step.)
3. Start one 30-second stall timer for the **whole batch** (never re-armed). For each hash in
   order wait for: its channel (a value, or closure by `notfound` → nil entry and
   `notfound = true`); the stall timer → unregister the remaining hashes, disconnect and
   return `errors.E(op, errors.IO, "peer appears stalled")`; peer disconnect → its reason;
   context end → return `ctx.Err()` and unregister the remaining hashes when the stall timer
   later fires.
4. Return the slice, with `ErrNotFound` if any entry was closed. For mix messages closure
   never happens (Q-07).

### 4.9 Committed filter requests

**`CFilterV2`** (`p2p/peering.go:1715-1763`):

1. `rp.pver < 7` → `errors.E(op, errors.Protocol, errors.Errorf("protocol version %v is too low to fetch cfiltersv2 from this peer", pver))` (unreachable with `minPver` 9). Op `remotepeer(<addr>).CFilterV2(<hash>)`.
2. Register the hash; duplicate →
   `errors.E(op, errors.Invalid, "cfilterv2 is already being requested from this peer for this block")`.
3. Start the stall timer; queue `getcfilterv2`; wait for the reply. Stall → unregister,
   disconnect, `errors.E(op, errors.IO, "peer appears stalled")`. Peer gone → its reason.
   Context end → `ctx.Err()`, unregistering when the timer fires.
4. Decode `Data` with `gcs.FromBytesV2(B=19, M=784931, data)`; a decode error is returned as
   `errors.E(op, err)` without disconnecting. Return the filter, `ProofIndex`, `ProofHashes`.

**`CFiltersV2`** (`p2p/peering.go:1776-1882`) — many pipelined single requests:

1. A sender goroutine walks the hashes in order: register (duplicate → push the Invalid error
   above to an error channel and stop), record the current time, queue `getcfilterv2`, and
   publish `(time, channel)` to the receiver. It stops on cancellation or peer disconnect and
   closes the publish channel on exit.
2. The receiver handles hashes in order. For hash `i` it first waits for the sender's record,
   then re-arms the stall timer to `30 s − (now − send time)`, then waits for the reply and
   decodes it as above. While waiting it also reacts to: context end → cleanup, `ctx.Err()`;
   stall → cleanup, disconnect, `errors.E(Op("remotepeer(<addr>).CFiltersV2(<hash i>)"), errors.IO, "peer appears stalled")`;
   peer gone → cleanup, its reason; sender error → cleanup, that error; decode error →
   cleanup, `errors.E(op, err)`.
3. Cleanup cancels the sender, drains its channel, and unregisters hashes `i..end`.
4. Return one `filterProof` per hash in input order.

Early sender exit (`p2p/peering.go:1792`, `1837-1862`): when the sender stops before
publishing every record (duplicate request, cancellation, peer disconnect) it closes the
publish channel. In the receiver's `select` the closed channel is then ready alongside the
error/disconnect/cancel case and Go chooses among ready cases at random. If the closed
channel is chosen, the receiver obtains a zero record, re-arms the stall timer with a negative
duration (it fires at once), and — because both the record's reply channel and the publish
channel variable are now nil — **leaves the wait for hash `i` without a filter** and moves to
hash `i+1`, where the same choice repeats with the fired stall timer as a third candidate. The
call therefore ends in one of: the expected error; a `peer appears stalled` disconnect; or, if
the closed channel wins for every remaining hash, a **nil error with zero-valued
`filterProof` entries (nil `Filter`)** from that hash on. Callers pass these filters to
`validate.CFilterV2HeaderCommitment`, which calls `filter.Hash()` for post-DCP0005 heights.
Established by reading; UNVERIFIED at runtime (a Go test calling `CFiltersV2` on an already
disconnected peer would settle the frequency and the nil-filter consequence). A port must
return the error in these cases.

Replies are matched by block hash, so the peer may answer in any order, but the receiver
consumes them in request order and the stall deadline for request `i` is 30 s after the
sender started queueing it (the time is taken before the queue send).

**`BatchedCFiltersV2(start, end)`** (`p2p/peering.go:1886-1940`):

1. Same pver check as `CFilterV2`; op text is `remotepeer(<addr>).CFilterV2(<start>)`.
2. Register `start` in the batched table (duplicate → the same Invalid error text).
3. Stall timer; queue `getcfsv2(start, end)`; wait. Stall → unregister, disconnect, IO
   `peer appears stalled`. Peer gone → reason. Context end → unregister, `ctx.Err()`.
4. Decode every returned filter; return one `filterProof` per returned entry. The method does
   not check the count, the block hashes after the first, or their order; the caller checks
   the count (§4.15) and the inclusion proofs bind each filter to its header.

### 4.10 Header requests

**`SendHeaders`** (`p2p/peering.go:1949-1986`): pver < 3 → Protocol error
`protocol version %v is too low to receive block header announcements` (unreachable). Set the
sendheaders flag under the lock; if it was already set return nil. Queue `sendheaders` with
the stall timer: stall → disconnect, `errors.E(Op("remotepeer(<addr>).SendHeaders"), errors.IO, "peer appears stalled")`;
peer gone → reason; context end → clear the flag and return `ctx.Err()`.

**`Headers(locators, hashStop)`** (`p2p/peering.go:1992-2036`), op `remotepeer(<addr>).Headers`:

1. Sendheaders mode → `errors.E(op, errors.Invalid, "synchronous getheaders after sendheaders is unsupported")`.
2. Another request pending → `errors.E(op, errors.Invalid, "headers are already being requested from this peer")`.
3. Record the reply channel and the locator list. Stall timer; queue `getheaders`; wait for
   the reply. Stall → disconnect, IO `peer appears stalled`; peer gone → reason; context end
   → `ctx.Err()`, clearing the pending request when the timer fires.
4. Return the headers (already validated by §4.5).

**`HeadersAsync(locators, hashStop)`** (`p2p/peering.go:2043-2077`): not in sendheaders mode →
`errors.E(op, errors.Invalid, "asynchronous getheaders before sendheaders is unsupported")`
(op text is also `remotepeer(<addr>).Headers`). Queue `getheaders` with the stall timer
(stall → disconnect + IO error; peer gone → reason; context end → `ctx.Err()`). Return as soon
as it is queued; the reply is delivered as an announcement.

### 4.11 `RemotePeer.GetInitState`

`p2p/peering.go:2174-2210`. One pending request per peer; a second concurrent call returns
`errors.E(Op("remotepeer(<addr>).GetInitState"), errors.Invalid, "init state is already being requested from this peer")`.
Stall timer; queue the caller's `getinitstate`; wait for `initstate`. Stall → disconnect, IO
`peer appears stalled` (the pending slot is not cleared); peer gone → reason; context end →
clear the slot, `ctx.Err()`.

### 4.12 `Syncer.Run`

`spv/sync.go:336-466`. One call per `Syncer`.

1. Defer `close(s.done)` (runs last; cancels every `WrapContext` child).
2. `tipHash, tipHeight = wallet.MainChainTip`; `setRequiredHeight(tipHeight)`.
3. `rescanPoint = wallet.RescanPoint` (error → return).
4. Log info `Headers synced through block <tipHash> height <tipHeight>`.
5. If a rescan point exists, load its header (error → return) and log info
   `Transactions synced through block <header.PrevBlock> height <header.Height-1>`; otherwise
   log the same line with the tip hash and height.
6. `AddrManager().Start()`; defer `Stop()` (a non-nil error logs
   `Failed to cleanly stop address manager: <err>` at error level).
7. If there are no persistent peers: `lp.SeedPeers(ctx, SFNodeNetwork)` (blocking, §4.1).
8. Create an error group `g` whose context `gctx` derives from **the background context**
   (not `ctx`), made additionally cancellable. `s.teardown` = set `Run`'s return error to
   `errors.E(errors.NoPeers)` and cancel `gctx`.
9. Start in `g`: `receiveGetData`, `receiveInv`, `receiveHeadersAnnouncements`. Then
   `lp.AddHandledMessages(MaskGetData | MaskInv)`.
10. Start in `g`: one `connectToPersistent(gctx, addr)` per persistent peer, or a single
    `connectToCandidates(gctx)`.
11. Start in `g`: `handleMempool`.
12. `wallet.SetNetworkBackend(s)`; defer `wallet.SetNetworkBackend(nil)`.
13. Create `walletCtx` from the background context; a helper goroutine cancels it when either
    `ctx` or `gctx` ends.
14. Start in `g` the **initial sync** task, all under `walletCtx`:
    1. Start `wallet.FetchMissingCFiltersWithProgress(walletCtx, s, progress)` in a goroutine.
       Log debug `Fetching missing CFilters...`; notify `FetchMissingCFiltersStarted`; for
       each progress item: an error returns it; otherwise notify
       `FetchMissingCFiltersProgress(start, end)`. Then notify
       `FetchMissingCFiltersFinished`; log debug `Fetched all missing cfilters`.
    2. Notify `FetchHeadersStarted`; log debug `Fetching headers and CFilters...`;
       `initialSyncHeaders` (§4.15); error → return it. Notify `FetchHeadersFinished`.
    3. `initialSyncRescan` (§4.20); error → return it.
    4. `close(s.initialSyncDone)`; return nil.
15. Start in `g`: `wallet.Run(walletCtx)` (runs the mixing client when mixing is enabled; a
    non-nil error returns it). After it returns nil, wait for `walletCtx` to end and return
    `walletCtx.Err()`. This task is what turns cancellation of the outer `ctx` into
    cancellation of `gctx`.
16. `g.Wait()`, then `if err == nil { err = groupErr }` on `Run`'s **named return variable**
    `err` (`spv/sync.go:462-465`). That variable is also written, without synchronization, by
    `teardown` (step 8) and by the initial sync task, which assigns the results of
    `initialSyncHeaders` and `initialSyncRescan` to it (`spv/sync.go:430`, `437`; a
    `FetchMissingCFilters` progress error is returned without being assigned). The result is
    therefore: the NoPeers error if `teardown` ran; else a non-nil error returned by
    `initialSyncHeaders`/`initialSyncRescan` even when another task failed first (for example
    `wallet.Run` failing during initial sync makes `Run` return the initial sync's
    `context canceled`, not the `wallet.Run` error); else the group's first error.
17. Deferred, in order: clear the wallet's network backend; stop the address manager; close
    `done`.

**Synced flag** (`spv/sync.go:178-196`). `synced()`: compare-and-swap false→true; on change
notify `Synced(true)`. `unsynced()`: compare-and-swap true→false; on change notify
`Synced(false)` and call `teardown()`. `unsynced()` is called only when the connected-peer
count drops to zero (§4.13). Consequently: losing all peers **after** the wallet became synced
ends `Run` with a `NoPeers` error (text `Decred network is unreachable`); losing all peers
before that does not.

**Restart loop** (`dcrwallet.go:531-556`). The CLI creates the address manager once, then
loops: new `LocalPeer` (same manager), `SetDialFunc(cfg.dial)`,
`SetDisableRelayTx(cfg.SPVDisableRelayTx)`, new `Syncer`, `SetPersistentPeers` if
`--spvconnect` is non-empty, `Run`. A nil error or a cancelled process context logs info
`SPV synchronization stopped` and returns; otherwise log error
`SPV synchronization stopped: <err>`, wait 5 seconds, repeat. Because the same `AddrManager`
is reused and its `Start`/`Stop` are effective once, see Q-02.

**gRPC driver** (`internal/rpc/rpcserver/server.go:2984-3157`). `WalletLoaderService.SpvSync`
runs one `Syncer` for the lifetime of the server-streaming call. The request/response
protobuf encoding belongs to the gRPC spec; the SPV-specific sequence is:

1. No wallet loaded → `FailedPrecondition` `Wallet has not been loaded`.
2. `wallet.NetworkBackend()` succeeds (a syncer of any kind is already attached) →
   `FailedPrecondition` `wallet is loaded and already synchronizing`.
3. `discover_accounts` set with an empty `private_passphrase` → `InvalidArgument`
   `private passphrase is required for discovering accounts`.
4. If `discover_accounts`: `wallet.Unlock(streamCtx, passphrase, lock)` with a one-slot lock
   channel; failure → the generic error translation. A `lockWallet` closure (send on the lock
   channel, zero the passphrase bytes) is deferred for handler exit.
5. A **new** address manager is created for every call from the loader's database directory,
   then `NewLocalPeer(params, [::1]:0, amgr)` with no dial function and no relay flag (Q-03).
   Because the manager is per call, Q-02 does not apply on this path: `peers.json` is loaded
   at the start of each call and written at its end.
6. `NewSyncer` is created **after** the unlock, so its `discoverAccounts = !wallet.Locked()`
   is true exactly when the wallet is unlocked at that moment — including a wallet that was
   already unlocked by other means while `discover_accounts` is false.
7. `SetNotifications` with the callbacks below; `SetPersistentPeers` when `spv_connect` is
   non-empty (§5).
8. `Run(streamCtx)`; error mapping per §7.2.

| Callback | `notification_type` | Other fields set |
|---|---|---|
| `Synced(true)` / `Synced(false)` | `SYNCED` / `UNSYNCED` | `synced` |
| `PeerConnected` / `PeerDisconnected` | `PEER_CONNECTED` / `PEER_DISCONNECTED` | `peer_information{peer_count, address}` |
| `FetchMissingCFiltersStarted` / `Progress` / `Finished` | `FETCHED_MISSING_CFILTERS_STARTED` / `_PROGRESS` / `_FINISHED` | progress: `fetch_missing_cfilters{fetched_cfilters_start_height, fetched_cfilters_end_height}` |
| `FetchHeadersStarted` / `Progress` / `Finished` | `FETCHED_HEADERS_STARTED` / `_PROGRESS` / `_FINISHED` | progress: `fetch_headers{fetched_headers_count, last_header_time}` |
| `DiscoverAddressesStarted` / `Finished` | `DISCOVER_ADDRESSES_STARTED` / `_FINISHED` | — |
| `RescanStarted` / `Progress` / `Finished` | `RESCAN_STARTED` / `_PROGRESS` / `_FINISHED` | progress: `rescan_progress{rescanned_through}` |

Every callback sends one `SpvSyncResponse` and ignores the send error. The
`DiscoverAddressesFinished` callback relocks the wallet (first time only) **before** sending
its message; the deferred `lockWallet` runs again at handler exit. `MempoolTxs` and
`TipChanged` are not set.

### 4.13 Peer acquisition

**Required height** (`spv/sync.go:323-329`): `lp.RequirePeerHeight(tip > 6 ? tip − 6 : tip)`.
Set at `Run` start and after every chain switch.

**`peerCandidate(svcs)`** (`spv/sync.go:471-506`): up to 100 tries. Each try:
`amgr.GetAddress()`; nil → stop. Under `remotesMu`, skip the address when any of:

- its key is already in `connectingRemotes` or `remotes`;
- `tries < 30` and its last attempt was less than 10 minutes ago;
- `tries < 50` and its known services lack `svcs`.

Otherwise mark the key in `connectingRemotes` and return the address. Exhaustion returns the
error `no addresses`.

**`connectToCandidates`** (`spv/sync.go:664-714`): a semaphore of 8 slots. Loop: return
`ctx.Err()` when cancelled; acquire a slot; `peerCandidate(SFNodeNetwork)`; on error wait 5
seconds, release the slot, retry. Otherwise start a goroutine:

1. `raddr = na.String()`; `b = backoffs[raddr]`.
2. First attempt for this address (`b.time` zero): `jitter = rand[0, 200 ms)`, `delay = jitter`.
   Otherwise `jitter = backoffJitter()` and `delay = (b.time − now) + jitter`.
3. `connected = connectAndRunPeer(ctx, delay, raddr, false)` (blocks for the peer's lifetime).
4. Connected → delete the backoff entry. Not connected →
   `b.duration = nextBackoff(b.duration + jitter)`, `b.time = now + b.duration`, store.
5. Release the slot.

At most 8 candidate goroutines exist, so connecting plus connected peers never exceed 8.

**`connectToPersistent(raddr)`** (`spv/sync.go:645-662`): `d = 0`; loop:
`jitter = backoffJitter()`; `connected = connectAndRunPeer(ctx, d/5 + jitter, raddr, true)`;
return `ctx.Err()` if cancelled; `d = connected ? 0 : d + jitter`; `d = nextBackoff(d)`. One
such loop per configured address; there is no cap on persistent peers.

**Backoff functions** (`spv/sync.go:615-624`): `nextBackoff(d) = min(5 s + d·95/100, 90 s)`
(integer nanosecond arithmetic); `backoffJitter() = −200 ms + rand[0, 400 ms)`.
Starting from 0 the sequence is 5 s, 9.75 s, 14.2625 s, … (fixed point 100 s, capped at 90 s
after about 45 consecutive failures; the source comment says "roughly 45m"); persistent peers
sleep one fifth of it (1 s … 18 s).

Delay edge cases (`spv/sync.go:514-521`, `650-651`, `694-700`): `connectAndRunPeer` sleeps
only when the delay is strictly positive. A candidate whose stored `b.time` already lies in
the past yields a negative `time.Until` and is dialled at once. The first attempt of a
persistent peer uses `0/5 + jitter` with jitter in [−200 ms, +200 ms), so it is dialled
immediately about half the time and after up to 200 ms otherwise. For a candidate the
`connectingRemotes` entry is written under `NetAddress.Key()` and removed under
`NetAddress.String()`; the two are the same string (§4.27). Candidate backoff entries live in
`Syncer.backoffs` for the life of that `Syncer` only and are deleted on a successful
connection (`setBackoff` deletes whenever the stored duration would be zero).

**`connectAndRunPeer(ctx, backoff, raddr, persistent)`** (`spv/sync.go:513-589`):

1. If `backoff > 0`: log trace `connectAndRunPeer(<raddr>): sleeping backoff <d>` and sleep
   (return false on cancellation).
2. `rp, err = lp.ConnectOutbound(ctx, raddr, SFNodeNetwork)`. On error: remove `raddr` from
   `connectingRemotes`; unless the error is a cancellation log warning
   `Peering attempt failed: <err>`; return false.
3. Under `remotesMu`: remove from `connectingRemotes`. If not persistent and
   `breaksMinVersionTarget(rp)`: log debug
   `Disconnecting <raddr>: peer uses too low version to satisfy minimum target version count`,
   disconnect with that error, return false. Else `remotes[raddr] = rp`, `n = len(remotes)`,
   and wake any `waitForRemote` waiter by closing and clearing `remoteAvailable`.
4. Log info `New peer <raddr> <user agent> version=<pver> <services>`; notify
   `PeerConnected(n, raddr)`; the function result is now true.
5. On exit (deferred): remove from `remotes`; notify `PeerDisconnected(n, raddr)` with the new
   count; if the count is 0 call `unsynced()`.
6. `peerStartup(ctx, rp)` (§4.20). On error: cancellation logs info `Lost peer <raddr>`, any
   other error logs warning `Unable to complete startup sync with peer <raddr>: <err>`;
   disconnect with the error; return.
7. Block on `rp.Err()`. Cancellation logs info `Lost peer <raddr>`; otherwise warning
   `Lost peer <raddr>: <err>`.

**`breaksMinVersionTarget(rp)`** (`spv/sync.go:591-609`): false if `rp.Pver() >= 10`; false
if fewer than 5 peers are currently connected; otherwise true unless at least 3 connected
peers have pver ≥ 10. Effect: with 5 or more peers connected, a version-9 candidate is
refused until 3 mixing-capable peers are present.

### 4.14 Peer selection

**`forRemotes(f)`** (`spv/sync.go:716-729`): under `remotesMu`; no peers →
`errors.E(errors.NoPeers)`; otherwise call `f` for each peer in map iteration order, stopping
at the first error. The mutex is held for the whole iteration, including any blocking work
done by `f`.

**`waitForRemote(ctx, isEligible, waitEligible)`** (`spv/sync.go:742-773`): loop under
`remotesMu`. No peers: create/reuse the `remoteAvailable` channel, unlock, wait for it to
close (or return `ctx.Err()`), repeat. Peers present: return the first peer in map iteration
order for which `isEligible` is true; if none, return `(nil, nil)` when `waitEligible` is
false, else unlock and repeat immediately (a busy loop until an eligible peer appears or the
peer set empties; the context is not checked on this path — Q-10).

Go map iteration order is randomized, so selection among eligible peers is effectively random.

**Eligibility predicates** (`spv/backend.go:35-61`):

| Predicate | True when |
|---|---|
| `pickAny` | always |
| `pickForGetHeaders(tipHeight)` | `rp.InitialHeight() > tipHeight && rp.LastHeight() < rp.InitialHeight()` |
| `pickForGetCfilters(lastHeaderHeight)` | `rp.InitialHeight() >= lastHeaderHeight − 6 \|\| rp.LastHeight() >= lastHeaderHeight` |

### 4.15 Initial sync: headers and committed filters

The initial sync runs once per `Run`, before any peer is switched to sendheaders mode
(`peerStartup` blocks on `initialSyncDone`). Connected peers are used only as request targets
through `waitForRemote`.

#### 4.15.1 Missing cfilters

`wallet.FetchMissingCFiltersWithProgress` (wallet spec) calls `Syncer.CFiltersV2` (§4.21) for
runs of main-chain blocks that lack stored filters, validates each against the stored header
with `validate.CFilterV2HeaderCommitment`, and stores them. For a wallet with complete
filters it reports nothing.

#### 4.15.2 `initialSyncHeaders` pipeline

`spv/sync.go:1616-1846`. Three tasks in an error group connected by two unbuffered channels;
any task error ends the whole initial sync (and `Run`).

A shared **invalidation signal** (`invalidateBatch`) is a channel that is closed and replaced
under a mutex; stage 1 holds the current channel and, when it observes it closed, drops its
cached batch so the next locators come from the wallet main chain only.

**Stage 1 — fetch headers.** Loop:

1. `likely = previous batch's bestChain` (nil for the first batch or after invalidation).
2. `batch = getHeaders(ctx, likely)` (§4.15.3); error → return.
3. If invalidated meanwhile: refresh the signal, forget the batch, continue.
4. Send the batch to stage 2 (also reacting to invalidation as in 3, and to cancellation).
5. If `batch.done`, return nil.

**Stage 2 — fetch cfilters.** Loop:

1. Receive a batch. If `done`, forward it to stage 3 and return nil.
2. Under `sidechainMu`, find the first node of `batch.bestChain` whose `FilterV2` is nil;
   `missing` = that node and everything after it.
3. `cfiltersV2FromNodes(ctx, missing)` (§4.15.4). On `errCfilterWatchdogTriggered`: log debug
   `Unable to fetch missing cfilters from <peer>: <err>`, invalidate, continue with the next
   batch. Any other error → return.
4. If `missing` is non-empty log debug
   `Fetched <n> new cfilters(s) ending at height <h>`.
5. Send the batch to stage 3.

**Stage 3 — chain switch.** Record the start time. Loop while not cancelled:

1. Receive a batch. If `done`: log debug `Initial sync completed in <duration rounded to 1 s>`
   and return nil.
2. Lock `sidechainMu`. `chain = batch.bestChain`.
3. Trim what is already attached: `tipIndex = walletTipHeight − chain[0].Height`; if
   `0 < tipIndex < len(chain) − 1` and `chain[tipIndex].Hash == walletTipHash`, log trace
   `Updating bestChain to tipIndex <i> from <h0> to <h1>` and use `chain[tipIndex+1:]`.
4. `prev, err = wallet.ChainSwitch(ctx, &sidechains, chain)`. On error: unlock,
   `batch.rp.Disconnect(err)`, invalidate, continue.
5. If `prev` is non-empty: log info
   `Reorganize from <prev tip hash> to <new tip hash> (total <n> block(s) reorged)` and add
   every removed node back into the sidechain forest.
6. Log info `Connected block <hash>, height <h>` for a single block, or
   `Connected <n> blocks, new tip <hash>, height <h>, date <header timestamp>` otherwise.
7. Notify `FetchHeadersProgress(tip height, tip header Unix time)`. Unlock.
8. `setRequiredHeight(tip height)`; `disconnectStragglers(tip height)` (§4.16.3).

During initial sync the nodes carry no `RelevantTxs`, so `ChainSwitch` does not advance the
wallet's processed-transactions marker; transactions are found later by the rescan (§4.20).

#### 4.15.3 `getHeaders(ctx, likelyBestChain)`

`spv/backend.go:277-402`. Loop until a batch is produced:

1. `tipHeight` = wallet main-chain tip height.
2. `rp = waitForRemote(ctx, pickForGetHeaders(tipHeight), waitEligible=false)`. Error →
   return it. `rp == nil` (peers exist but none advertises unfetched headers above the wallet
   tip) → return `{done: true}`: **initial header sync is complete**.
3. Log trace `Attempting next batch of headers from <peer>`.
4. `locators, locatorHeight = wallet.BlockLocators(ctx, likelyBestChain)` (§3.4); error →
   return.
5. `headers = rp.Headers(ctx, locators, zero hash)`. Error → log debug
   `Unable to fetch headers from <peer>: <err>` and retry from 1.
6. Empty reply: if `rp.LastHeight() < rp.InitialHeight()` and
   `locatorHeight < rp.InitialHeight()` → disconnect with
   `errors.E(errors.Protocol, "peer did not provide headers through advertised height")`
   and retry; otherwise log trace
   `Skipping to next batch due to len(headers) == 0 from <peer>` and retry.
7. Build one `BlockNode` per header. The hash of header `i` is taken from header `i+1`'s
   `PrevBlock` (already verified by the p2p layer); only the last header is hashed. Mark the
   node invalid when `wallet.BadCheckpoint(net, hash, height)` (the only checkpoint is
   testnet3 height 962928 =
   `0000004fd1b267fd39111d456ff557137824538e6f6776168600e56002e23b93`,
   `wallet/checkpoints.go:22-34`).
8. Lock `sidechainMu`. `full = sidechains.FullSideChain(nodes)`; error → unlock and return
   it (fatal).
9. `wallet.ValidateHeaderChainDifficulties(ctx, full, 0)`. Error → unlock, disconnect the
   peer with it, log warning
   `Disconnecting from <peer> due to header validation error: <err>` (unless cancelled),
   retry.
10. For each node not already in the wallet main chain, `sidechains.AddBlockNode`; count the
    additions.
11. `best = wallet.EvaluateBestChain(ctx, &sidechains)`. Error → unlock, disconnect with it,
    retry. Empty → unlock, retry.
12. Log debug `Fetched <added> new header(s) ending at height <last height> from <peer>`.
    Unlock. Return `{nodes, bestChain: best, rp}`.

Each `getheaders` reply carries at most 2000 headers (`wire.MaxBlockHeadersPerMsg`); because
stage 1 requests the next batch using locators built from the not-yet-attached `bestChain`,
up to three batches are in flight through the pipeline at once.

#### 4.15.4 `cfiltersV2FromNodes(ctx, nodes)`

`spv/backend.go:149-262`.

1. Empty input → nil.
2. More than 100 nodes: split into consecutive chunks of 100 (the last may be shorter), run
   each chunk through this function concurrently in an error group, return the first error.
3. For one chunk (≤ 100 nodes): create a watchdog context with a 2-minute timeout. Loop while
   the outer context is live:
   1. `rp = waitForRemote(watchdogCtx, pickForGetCfilters(last node height), waitEligible=true)`.
   2. If the watchdog has expired and the outer context has not: log warning
      `Batch of CFilters ending on block <hash> at height <h> not received within 2m0s. Clearing sidechain forest to retry with different headers`;
      under `sidechainMu` `sidechains.PruneAll()`; return `errCfilterWatchdogTriggered`.
   3. Other `waitForRemote` error → return it.
   4. If `rp.Pver() >= 11`: `rp.BatchedCFiltersV2(ctx, first hash, last hash)`; when it
      succeeds with a count different from the chunk size, disconnect with
      `errors.E(errors.Protocol, "peer returned unexpected number of filters (got <g>, want <w>)")`
      and retry. If `rp.Pver() < 11`: `rp.CFiltersV2(ctx, all hashes)` instead.
   5. Request error → log trace `Unable to fetch cfilter batch for from <peer>: <err>` and
      retry immediately (no delay, peer not disconnected by this function).
   6. Validate each filter with
      `validate.CFilterV2HeaderCommitment(net, node.Header, filter, proofIndex, proof)`.
      On failure: message
      `CFilter for block <hash> (height <h>) received from <peer> failed validation: <err>`,
      logged at warning; disconnect with `errors.E(errors.Protocol, message)`; retry.
   7. Under `sidechainMu` assign every node's `FilterV2`. Log trace
      `Fetched <n> new cfilters(s) ending at height <h> from <peer> (request took <d truncated to ms>)`.
      Return nil.
4. Outer context ended → `ctx.Err()`.

#### 4.15.5 Header-commitment proof verification

`validate.CFilterV2HeaderCommitment(net, header, filter, leafIndex, proof)`
(`validate/validate.go:70-103`):

- testnet3 with `header.Height < 323328`, or mainnet with `header.Height < 431488`: accept
  without checking (pre-DCP0005 filters are authenticated in bulk by the wallet elsewhere).
- Otherwise require
  `standalone.VerifyInclusionProof(root = header.StakeRoot, leaf = filter.Hash(), leafIndex, proof)`;
  failure is `errors.E(Op("validate.CFilterV2HeaderCommitment(<block hash>)"), errors.Consensus, "invalid header inclusion proof for cfilterv2")`.
- `filter.Hash()` is BLAKE-256 of the serialized filter (varint N followed by the Golomb
  data), or the zero hash for an empty serialization.
- On every other network (simnet, regnet) the proof is always checked.

### 4.16 Steady state: pushed headers

#### 4.16.1 `receiveHeadersAnnouncements`

`spv/sync.go:1264-1288`. Loop on `lp.ReceiveHeadersAnnouncement`; for each announcement start a
goroutine running `handleBlockAnnouncements(ctx, rp, headers, nil)`. On a non-nil error (and a
live context): if the error is of kind Protocol or Consensus, log warning
`Disconnecting peer <peer>: <err>` and disconnect with it; otherwise log warning
`Failed to handle headers announced by <peer>: <err>`.

#### 4.16.2 `handleBlockAnnouncements(ctx, rp, headers, bmap)`

`spv/sync.go:1381-1584`. Any returned error is wrapped with op
`spv.handleBlockAnnouncements(<remote addr>)` unless the context ended.

1. No headers → nil.
2. If the first header's height is below the wallet tip by more than 256 →
   `errors.E(errors.Protocol, "peer announced old header")`.
3. Lock `sidechainMu` for steps 4-17.
4. **Orphan check.** If the first header's parent is neither in the wallet main chain nor in
   the sidechain forest: `rp.ReceivedOrphanHeader()` (error → return it, a Protocol error);
   `locators = wallet.BlockLocators(ctx, nil)`; `rp.HeadersAsync(ctx, locators, zero hash)`;
   return nil. The announced headers are discarded.
5. Build nodes: iterate the headers in order; while no node has been kept yet, skip headers
   already in the main chain or the forest; keep all the rest.
6. Nothing kept → return nil.
7. `full = sidechains.FullSideChain(newBlocks)`; error → return.
8. `wallet.ValidateHeaderChainDifficulties(ctx, full, 0)`; error → return (Consensus kind
   errors disconnect the peer via §4.16.1; other kinds only log).
9. Add every new node to the forest. (No checkpoint marking is applied on this path.)
10. `best = wallet.EvaluateBestChain`; error → return; empty → return nil (go to step 19).
11. `filters = rp.CFiltersV2(ctx, hashes of every node in best)` — individual
    `getcfilterv2` requests to the **announcing peer**, for the whole best chain. Error →
    return it (`ctx.Err()` if cancelled).
12. Validate each with `CFilterV2HeaderCommitment`; error → return (Consensus → disconnect).
    Assign `FilterV2`.
13. `rpt = wallet.RescanPoint`; error → return. If nil (wallet transactions are current):
    `matching = scanChain(ctx, rp, best, bmap)` (§4.16.4; error → return) and set each node's
    `RelevantTxs` to its matches, using an **empty non-nil** list when there are none (this
    tells `ChainSwitch` to advance the processed-transactions marker).
14. `prev = wallet.ChainSwitch(ctx, &sidechains, best)`; error → return.
15. If `prev` is non-empty: log info
    `Reorganize from <prev tip> to <new tip> (total <n> block(s) reorged)` and re-add the
    removed nodes to the forest.
16. `setRequiredHeight(tip height)`; `disconnectStragglers(tip height)`.
17. Notify `TipChanged(tip header, len(prev), all matching txs)` (transactions concatenated
    in map iteration order).
18. Unlock. For each node of `best` log info
    `Connected block <hash>, height <h>, <n> wallet transaction(s)`.
19. For each kept new node not now in the main chain log info
    `Received sidechain or orphan block <hash>, height <h>` (a `BlockInMainChain` error is
    returned).

Announcements from all peers are serialized by `sidechainMu`, which is held across the
network round trips of steps 4, 11 and 13.

#### 4.16.3 `disconnectStragglers(height)`

`spv/sync.go:1588-1607`. For every connected peer: `peerHeight = max(LastHeight, InitialHeight)`;
if `height − peerHeight > 6`, disconnect with
`errors.E(errors.Policy, "disconnecting from straggler peer (peer height <p>, tip height <t>)")`.

#### 4.16.4 `scanChain(ctx, rp, chain, bmap)`

`spv/sync.go:1293-1375`.

1. Snapshot `filterData` under `filterMu`.
2. `fetched[i]` = block from `bmap` when present.
3. For every node without a block, test
   `node.FilterV2.N() != 0 && node.FilterV2.MatchAny(blockcf2.Key(&header.MerkleRoot), filterData)`
   on up to `NumCPU` worker goroutines; collect matching hashes and indexes.
4. If any matched: `blocksFromPeer(ctx, rp, matches)` (§4.21; error → return) and place the
   blocks.
5. Return `ctx.Err()` if cancelled.
6. For every node with a block (matched or supplied by `bmap`): `rescanBlock(block)` (§4.24)
   and record the result under the node hash (possibly an empty list).

The filter key is the first 16 bytes of the header's merkle root
(`dcrd/gcs/v4@v4.1.1/blockcf2/blockcf.go:96-100`).

### 4.17 Serving `getdata`

`receiveGetData` (`spv/sync.go:779-888`) loops on `lp.ReceiveGetData`, handling each message in
its own goroutine:

1. Classify each inv vector:
   - hash not in the peer's `InvsSent` LRU → notfound;
   - type tx → candidate transaction;
   - type mix → candidate mix message when `wallet.MixingEnabled()`, else notfound;
   - any other type (including block) → notfound.
2. Transactions: `wallet.GetTransactionsByHashes`. An error other than NotExist logs warning
   `Failed to look up transactions for getdata reply to peer <addr>: <err>` and **abandons
   the whole reply** (nothing sent). Hashes the wallet lacks are looked up in the syncer
   mempool (§4.25); still missing → notfound.
3. Mix messages: `wallet.MixMessage(hash)`; error → notfound (rebuilt as a type-4 vector).
4. Send every found transaction, then every found mix message, each as its own message via
   `rp.SendMessage`; a failure logs warning
   `Failed to send getdata reply to peer <addr>: <err>` and continues. Stop silently if the
   context ended.
5. If there are notfound vectors, send one `notfound` listing them in the order they were
   classified (unannounced and unsupported first in request order, then wallet/mempool
   misses, then mixpool misses). Failure logs warning
   `Failed to send notfound reply to peer <addr>: <err>`.

Reply order on the wire is therefore: wallet-store transactions in request order, then
mempool-only transactions, then mix messages, then `notfound`.

### 4.18 Handling `inv`

`receiveInv` (`spv/sync.go:892-951`) loops on `lp.ReceiveInv`. It partitions the vectors into
block, tx and mix hashes (mix only when `wallet.MixingEnabled()`; other types ignored) and
starts one goroutine per non-empty class.

**Blocks — `handleBlockInvs`** (`spv/sync.go:1060-1087`): if the peer has not been sent
`sendheaders`, log debug `Ignoring block invs from <peer> before sendheaders is sent` and stop.
Otherwise `blocksFromPeer(ctx, rp, hashes)` (error wrapped with op
`spv.handleBlockInvs(<peer>)`), take each block's header, and run
`handleBlockAnnouncements(ctx, rp, headers, bmap)` with the fetched blocks. Errors are treated
as in §4.16.1 with the non-disconnecting warning text
`Failed to handle blocks inventoried by <peer>: <err>`.

**Transactions — `handleTxInvs`** (`spv/sync.go:1094-1177`):

1. `wallet.RescanPoint`; error → log warning (op `spv.handleTxInvs(<addr>)`) and stop. A
   non-nil rescan point → stop: transaction invs are ignored while a rescan is needed or
   running.
2. Drop hashes present in `seenTxs`. Nothing left → stop.
3. `rp.Transactions(ctx, unseen)`. On NotExist drop the nil entries and their hashes. Any
   other error → log warning (unless cancelled) and stop.
4. Add the fetched hashes to `seenTxs`.
5. For each transaction: if `checkTSpend` (§4.19) → `wallet.AddTSpend`.
6. `relevant = filterRelevant(txs)` (§4.24). For each: skip tickets when
   `wallet.ManualTickets()`; `wallet.AddTransaction(ctx, tx, nil)`. On error log warning; if
   the error kind is ScriptFailure and the peer advertises `SFNodeNetwork`, disconnect with
   the error and stop (without notifying).
7. Notify `MempoolTxs(relevant)` (called even when the list is empty, and including skipped
   manual tickets).

**Mix messages — `handleMixInvs(ctx, rp, hashes, onlyByID)`** (`spv/sync.go:1182-1259`):

1. Drop hashes present in `seenMixMsgs`; nothing left → stop.
2. `rp.MixMessages(ctx, unseen)`; NotExist handling as above; other error → warning (op
   `spv.handleMixInvs(<addr>)`) and stop.
3. Add fetched hashes to `seenMixMsgs`.
4. For each message: when `onlyByID` is non-empty skip messages whose 33-byte identity is not
   in it. `wallet.AcceptMixMessageBySource(msg, rp)`. If the error is a
   `*mixpool.MissingOwnPRError`: log debug `Will request unknown PR from <identity hex>`,
   remember the missing pair-request hash and the key-exchange identity. Any other error →
   warning.
5. If any pair requests are missing, recurse once per level with the missing hashes (in map
   iteration order) and the remembered identities as `onlyByID`.

### 4.19 Treasury spends

**`Syncer.GetInitState(ctx, rp)`** (`spv/sync.go:1016-1058`): send `getinitstate` requesting
only `tspends`; collect hashes not `wallet.IsTSpendCached`; none → nil. Fetch them with
`rp.Transactions`; on NotExist drop the nil entries; other error → return. For each
transaction passing `checkTSpend`, `wallet.AddTSpend`.

**`checkTSpend(ctx, tx)`** (`spv/sync.go:983-1012`) is true only when all hold:

1. `stake.CheckTSpend(tx)` succeeds, yielding the signature and Pi public key;
2. wallet tip height ≤ `tx.Expiry` (compared as uint32);
3. `chainParams.PiKeyExists(pubKey)`;
4. the Schnorr signature verifies against
   `txscript.CalcSignatureHash(nil, SigHashAll, tx, 0, nil)` with
   `schnorr.ParseSignature` / `schnorr.ParsePubKey`.

### 4.20 Initial rescan and per-peer startup

**`initialSyncRescan`** (`spv/sync.go:1850-1908`):

1. `rescanPoint = wallet.RescanPoint`; error → return.
2. Nil: log debug `Skipping rescanning due to rescanPoint == nil`;
   `wallet.LoadActiveDataFilters(ctx, s, true)`; `synced()`; return.
3. Otherwise log debug
   `Starting address discovery (discoverAccounts=<b>, gapLimit=<n>, rescanPoint=<hash>)`;
   notify `DiscoverAddressesStarted`;
   `wallet.DiscoverActiveAddresses(ctx, s, rescanPoint, discoverAccounts, gapLimit)`;
   notify `DiscoverAddressesFinished`.
4. `wallet.LoadActiveDataFilters(ctx, s, true)`.
5. Load the rescan point header; start `wallet.RescanProgressFromHeight(ctx, s, height, progress)`
   in a goroutine; notify `RescanStarted`; for each progress item an error returns it,
   otherwise notify `RescanProgress(ScannedThrough)`; notify `RescanFinished`.
6. Log debug `Wallet considered synced`; `synced()`.

**`peerStartup(ctx, rp)`** (`spv/sync.go:1912-1959`), run for every peer as soon as it is
registered:

1. Wait for `initialSyncDone`; return `ctx.Err()` on cancellation or the peer's reason if it
   disconnects first.
2. If `rp.Pver() >= 8` (always): `GetInitState`; an error logs
   `Failed to get init state: <err>` at error level and is otherwise ignored.
3. `wallet.UnminedTransactions`; error → log error
   `Cannot load unmined transactions for resending: <err>` and **return nil** (the peer stays
   connected but is never sent `sendheaders`).
4. If any: `rp.PublishTransactions(txs...)` — queues trickled tx invs and records the hashes
   in `InvsSent`.
5. `rp.SendHeaders(ctx)`; error → return.
6. Log debug `Requesting updated headers from peer <peer>`;
   `locators = wallet.BlockLocators(ctx, nil)`; `rp.HeadersAsync(ctx, locators, zero hash)`;
   errors → return.

There is no periodic rebroadcast in SPV mode: unmined wallet transactions are re-announced
only to each newly started peer (step 4) and when a client calls the gRPC
`PublishUnminedTransactions` method (wallet spec).

### 4.21 Block and cfilter fetching for the wallet

**`blocksFromPeer(ctx, rp, hashes)`** (`spv/backend.go:67-92`): `rp.Blocks`; for every block
require `validate.MerkleRoots(block)` or, failing that, `validate.DCP0005MerkleRoot(block)`;
if both fail disconnect the peer with the second error and return it.

- `MerkleRoots`: `header.MerkleRoot == CalcTxTreeMerkleRoot(Transactions)` and
  `header.StakeRoot == CalcTxTreeMerkleRoot(STransactions)`; errors
  `invalid regular merkle root` / `invalid stake merkle root`, kind Consensus, op
  `validate.MerkleRoots(<hash>)`.
- `DCP0005MerkleRoot`: `header.MerkleRoot == CalcCombinedTxTreeMerkleRoot(Transactions, STransactions)`;
  error `invalid combined merkle root`, kind Consensus, op `validate.DCP0005MerkleRoot(<hash>)`.

The block's header hash already equals the requested hash because the p2p layer matches
replies by header hash.

**`Syncer.Blocks`** (`spv/backend.go:95-111`): loop: return `ctx.Err()` if cancelled;
`waitForRemote(pickAny, true)`; `blocksFromPeer`; on error log debug
`Unable to fetch blocks from <peer>: <err>` and retry with another pick; else return.

**`Syncer.CFiltersV2`** (`spv/backend.go:124-141`): same loop with `rp.CFiltersV2`; error log
`Error while fetching cfilters from <peer>: <err>` (debug). The proofs are returned to the
caller unverified; the wallet verifies them.

Neither method bounds its retries; with no peers it blocks until one connects or the context
ends.

### 4.22 `LoadTxFilter(ctx, reload, addrs, outpoints)`

`spv/backend.go:417-433`. Under `filterMu`: when `reload` is true, replace the precise filter
with an empty `wallet.RescanFilter` and clear `filterData`. For each address add it to the
precise filter and append its payment script to `filterData` (the script version returned by
`PaymentScript` is discarded; empty scripts are skipped by `Entries.AddRegularPkScript`). For each outpoint add it to the precise filter's
unspent set. Always returns nil.

`filterData` holds only **address scripts**; version-2 block filters do not commit to spent
outpoints, so a spend of a watched outpoint is found only because the address that owns it is
also loaded (stated in the source comment at `spv/backend.go:414-416`). `filterData` grows
monotonically between reloads and may contain duplicates.

### 4.23 Publishing

**`Syncer.PublishTransactions(ctx, txs...)`** (`spv/backend.go:437-475`):

1. `relevant = wallet.DetermineRelevantTxs(txs...)`; error → return.
2. For each transaction in input order: if not wallet-relevant, store it in the syncer
   mempool keyed by hash; when newly stored, hand the hash to the mempool evictor (blocking;
   context end → `ctx.Err()`). Append a type-1 inv vector (failure → Protocol error).
3. `forRemotes`: for each connected peer add every hash to its `InvsSent` and
   `rp.SendMessage(ctx, inv)` — one **immediate, untrickled** `inv` carrying all hashes. The
   first peer error stops the iteration and is returned; with no peers the result is
   `errors.E(errors.NoPeers)`.

**`Syncer.PublishMixMessages(ctx, msgs...)`** (`spv/backend.go:479-520`), op
`spv.PublishMixMessages`:

1. Build one `inv` with a type-4 vector per message (hash = `msg.Hash()`). For every
   key-exchange message remember `ke.SeenPRs[ke.Pos]` — the sender's own pair-request hash.
2. `forRemotes`: skip peers with pver < 10; for the others add every inv hash **and every
   remembered own-PR hash** to `InvsSent` (so the peer may `getdata` the pair request it
   learns of through the key exchange), then `SendMessage`.
3. `forRemotes` error → `errors.E(op, err)`. No peer with pver ≥ 10 →
   `errors.E(op, errors.Protocol, "no connected peers support the mixing protocol version")`.

### 4.24 Rescan and transaction matching

**`Syncer.Rescan(ctx, blockHashes, save)`** (`spv/backend.go:523-624`):

1. For each hash load `(key, filter)` with `wallet.CFilterV2` (a read of the stored filter;
   error → return).
2. Snapshot `filterData`.
3. On `NumCPU` workers test `filter.MatchAny(key, filterData)` for every block; collect
   matching hashes and indexes.
4. If any matched, loop: return `ctx.Err()` if cancelled; `waitForRemote(pickAny, true)`;
   `blocksFromPeer(ctx, rp, matches)`; on error retry with another pick (silently); else
   place the blocks and stop looping.
5. In input order, for every block fetched: return `ctx.Err()` if cancelled;
   `matched = rescanBlock(block)`; if non-empty call `save(&blockHashes[i], matched)` and
   return its error.

**`rescanBlock(block)`** (`spv/rescan.go:20-86`), under `filterMu`: process the **stake tree
first, then the regular tree**. For each transaction at index `i`:

- type = regular for the regular tree, else `stake.DetermineTxType(tx)`;
- inputs considered: none when `i == 0` and the type is regular (this includes the coinbase,
  and also the first stake-tree transaction if it is not a recognised stake type); all but
  the first for votes; all otherwise. A transaction matches if any considered input's
  previous outpoint is in the unspent set;
- outputs: for each output, extract addresses with `stdscript.ExtractAddrs(version, script, params)`;
  for each address in the precise filter, add the outpoint `(txhash, index, tree)` to the
  unspent set if absent and mark the transaction matched.

A matched transaction is appended once. Spent outpoints are never removed from the unspent
set.

**`filterRelevant(txs)`** (`spv/rescan.go:90-115`), under `filterMu`: keep a transaction when
any input's previous outpoint is in the unspent set or any output address is in the precise
filter. The filter is not modified. The result reuses the input slice's storage.

### 4.25 Syncer mempool

`handleMempool` (`spv/sync.go:1963-1980`): for each hash received from `PublishTransactions`
start a timer goroutine that deletes the entry after 60 minutes (or exits on cancellation).
The mempool holds only transactions published by this wallet process that the wallet database
does not consider relevant (for example transactions sent through `sendrawtransaction`), so
they can be served to peers that request them.

### 4.26 Notification order

For one `Run` the callbacks fire in this order (peer events interleave freely):

1. `FetchMissingCFiltersStarted`, zero or more `FetchMissingCFiltersProgress`,
   `FetchMissingCFiltersFinished`.
2. `FetchHeadersStarted`, one `FetchHeadersProgress` per successful chain switch,
   `FetchHeadersFinished` (omitted if header sync fails).
3. Only when a rescan point exists: `DiscoverAddressesStarted`, `DiscoverAddressesFinished`,
   `RescanStarted`, zero or more `RescanProgress`, `RescanFinished`.
4. `Synced(true)`.
5. Afterwards: `TipChanged` per chain switch from announcements, and `Synced(false)` once if
   the peer count reaches zero (immediately followed by `Run` ending).

`MempoolTxs` fires at most once per tx inv, at step 7 of `handleTxInvs` (§4.18): only when no
rescan is pending, at least one announced hash was unseen, the fetch succeeded, and no
ScriptFailure disconnect cut the handler short; for a wallet that
starts with no rescan point this includes the period before `Synced(true)`.

`PeerConnected(count, addr)` and `PeerDisconnected(count, addr)` report the connected-peer
count after the change and the dialed address string.

The CLI sets no callbacks. The gRPC `SpvSync` handler sets all but `MempoolTxs` and
`TipChanged`, and maps `FetchHeadersProgress`'s first argument (the tip **height**) to the
protobuf field `fetched_headers_count` (`internal/rpc/rpcserver/server.go:3077-3085`).

### 4.27 Address manager behaviour relied on (addrmgr v3.0.0)

The wallet never touches the address book directly; everything below happens inside the calls
of §2.4, each under the manager's single mutex. It is restated because it decides which
addresses `peerCandidate` is offered and what `peers.json` contains.

**State** (`dcrd/addrmgr/v3@v3.0.0/addrmanager.go:29-99`, `knownaddress.go:16-43`): a 32-byte
random `key`; `addrIndex` (address key → known address); 1024 "new" buckets (maps keyed by
address key); 64 "tried" buckets (ordered lists); counters `nNew`, `nTried`; a dirty flag
`addrChanged`. A known address holds: the net address (treated as immutable and replaced by a
modified copy), the source address, `attempts`, `lastattempt`, `lastsuccess`, `tried`, and
`refs` (the number of new buckets holding it).

**Net addresses** (`netaddress.go:59-69`, `86-113`, `175-186`).
`NewNetAddressFromIPPort(ip, port, services)`: type is IPv4 when `ip.To4() != nil`, IPv6 when
the slice is 16 bytes long, otherwise unknown (the error is discarded); the stored bytes are 4
bytes for IPv4 and 16 for IPv6; timestamp = now truncated to whole seconds. `Key()` (and
`String()`, which is identical) is `net.JoinHostPort(<ip text>, <decimal port>)`; for an
unknown type the host part is the literal text
`unsupported IP type <n>, <bytes as string>, <bytes as hex>`.

**Routability** (`network.go:211-228`). An address is routable when it is non-nil, not
unspecified, not `255.255.255.255`, and in none of: 10/8, 172.16/12, 192.168/16 (RFC 1918);
198.18/15 (RFC 2544); 169.254/16 (RFC 3927); fe80::/64 (RFC 4862); 2001:db8::/32 (RFC 3849);
2001:10::/28 (RFC 4843); 192.0.2/24, 198.51.100/24, 203.0.113/24 (RFC 5737); 100.64/10
(RFC 6598); loopback or 0.0.0.0/8 ("local"); fc00::/7 (RFC 4193).

**Group key** (`network.go:232-272`), used only for bucket hashing: `local` for local
addresses; `unroutable` for non-routable ones; for IPv4 the /16 network as dotted text; for
IPv6 in `::ffff:0:0:0/96` (RFC 6145) or `64:ff9b::/96` (RFC 6052) the /16 of the embedded
IPv4 (bytes 12-15); for `2002::/16` (RFC 3964) the /16 of bytes 2-5; for `2001::/32`
(RFC 4380, Teredo) the /16 of bytes 12-15 each XORed with `0xff`; for `2001:470::/32` the /36;
any other IPv6 the /32.

**Bucket hashing** (`addrmanager.go:360-397`). `H` is BLAKE-256, `LE64(x)` the first 8 bytes
of `x` as a little-endian uint64, `le8(n)` the 8-byte little-endian encoding of `n`:

- new bucket for `(addr, src)`:
  `n = LE64(H(key ‖ group(addr) ‖ group(src))) mod 64`;
  bucket = `LE64(H(key ‖ group(src) ‖ le8(n))) mod 1024`.
- tried bucket for `addr`:
  `n = LE64(H(key ‖ Key(addr))) mod 8`;
  bucket = `LE64(H(key ‖ group(addr) ‖ le8(n))) mod 64`.

The Go loader does not check that the bucket an address is listed in matches these hashes, so
bucket placement is not needed for file compatibility; it matters only for parity of eviction
and selection behaviour.

**`AddAddresses(addrs, src)`** → `addOrUpdateAddress` per address
(`addrmanager.go:223-298`, `645-652`):

1. Non-routable address → ignored. (Consequence: loopback and private-range peers, which is
   what simnet/regnet setups and LAN peers use, can never enter the book; they are reachable
   only through `--spvconnect`.)
2. Already known: if the offered timestamp is later than the stored one, or the offered
   services are not a subset of the stored ones, replace the stored net address by a copy
   with the offered timestamp and the union of services. Then stop if the address is tried,
   or if `refs == 8`; otherwise continue with probability `1 / (2·refs)` and stop otherwise.
   The stored source address is never updated.
3. Not known: store a copy with `src` as its source; `nNew += 1`; mark dirty.
4. `bucket = newBucket(addr, src)` using the `src` of **this** call. Already in that bucket
   → stop.
5. If the bucket holds **more than** 64 entries, run `expireNew(bucket)` (so a new bucket can
   hold 65).
6. `refs += 1`; insert; mark dirty.

**`expireNew(bucket)`** (`addrmanager.go:302-340`): remove every entry for which `isBad()` is
true, then additionally remove the remaining entry with the earliest timestamp (on equal
timestamps the one visited later in map iteration). Each removal decrements `refs`; at zero
the address leaves `addrIndex` and `nNew -= 1`.

**`isBad()`** (`knownaddress.go:87-117`), first matching rule wins: attempted within the last
minute → false; timestamp more than 10 minutes in the future → true; timestamp older than 30
days → true; never succeeded and `attempts >= 3` → true; no success within the last 7 days
and `attempts >= 5` → true; otherwise false.

**`chance()`** (`knownaddress.go:63-79`): `0.01` when the address was **never attempted** or
was attempted less than 10 minutes ago; otherwise `max(1.5^(−attempts), 0.01)`.

**`GetAddress()`** (`addrmanager.go:766-824`):

1. `nNew + nTried == 0` → nil.
2. Use the tried table when `nTried > 0` and (`nNew == 0` or a fair coin says so); otherwise
   the new table.
3. `factor = 1.0`. Loop: pick a uniformly random bucket of the table; if it is empty, pick
   again; pick a uniformly random entry of the bucket (for a new bucket, the n-th entry in map
   iteration order); draw `r` uniformly from `[0, 2^30)`; return the entry when
   `r < factor · chance() · 2^30`; otherwise `factor *= 1.2` and repeat.

The result may be an address the wallet is already connected to; `peerCandidate` filters
those (§4.13). There is no service, type or group filter in this version.

**`Attempt(addr)`** (`addrmanager.go:834-852`): `attempts += 1`; `lastattempt = now`. Does not
mark dirty.

**`Connected(addr)`** (`addrmanager.go:858-880`): if the stored timestamp is more than 20
minutes old, replace the net address by a copy with timestamp = now. Does not mark dirty.

**`Good(addr)`** (`addrmanager.go:887-981`):

1. `lastsuccess = lastattempt = now`; `attempts = 0`.
2. Already tried → done.
3. Remove the address from every new bucket (each removal: `refs -= 1`, mark dirty),
   remembering the first bucket index it was found in; `nNew -= 1`. Found in none → error
   `<addr> is not marked as a new address` (kind `ErrAddressNotFound`; `nNew` stays
   decremented).
4. `bucket = triedBucket(addr)`. Fewer than 256 entries → append, `tried = true`,
   `nTried += 1`, mark dirty, done.
5. Otherwise evict the entry of that tried bucket with the earliest timestamp (the first such
   on ties): it takes the promoted address's slot; the evicted address becomes new again
   (`tried = false`, `refs += 1`, `nNew += 1`) in `newBucket(evicted, its source)`, or, when
   that bucket already holds 64 or more entries, in the bucket remembered in step 3.

`Attempt`, `Connected` and `Good` return an `ErrAddressNotFound` error
(`address <addr> not found`) for an address that is not in `addrIndex`; the wallet discards
it (§4.2).

**`NeedMoreAddresses()`** (`addrmanager.go:665-670`): `nNew + nTried < 1000`.

**Lifecycle** (`addrmanager.go:401-417`, `609-639`, `723-741`, `1218-1227`). `New` calls
`reset()`, which draws the random key and sets the dirty flag, so the first save always
writes a file even when nothing was learned. `Start` (first call only) loads the file and
starts a task that saves every 10 minutes; `Stop` (first call only) logs
`Address manager shutting down`, ends that task, and the task performs one final save before
`Stop` returns. A save is skipped when the dirty flag is clear, and the flag is cleared only
after a successful rename. Because `Attempt` and `Connected` do not set the flag, attempt
counters and refreshed timestamps reach the file only when some other change (an added
address, a promotion, an expiry) triggers a save.

**Saving** (`addrmanager.go:421-488`): the temp file is created with Go's `os.Create`
(mode `0666` before umask, truncating any leftover), written, closed and renamed; there is no
`fsync`. On an encode failure the temp file is left behind and its handle is not closed.

**What the wallet feeds it.** Seeder results: source = the first address of that seeder's
list (which is therefore its own source), services = 1, timestamp = time of parsing.
`addr` gossip: source = the announcing peer's address object, services and timestamp copied
from each wire entry without the future-timestamp or age adjustments a full node applies
(`p2p/peering.go:973-981`). Outbound attempts: `Attempt` before dialling, `Connected` after
the TCP connect, `Good` after the handshake and the service and height checks pass.

### 4.28 Dynamic ban score (connmgr v3.1.4)

`dcrd/connmgr/v3@v3.1.4/dynamicbanscore.go:16-147`. State per peer: `lastUnix` (int64
seconds, initially 0), `transient` (float64), `persistent` (uint32); all under one mutex. The
wallet only ever passes `persistent = 0`.

`decay(dt) = exp(−dt · ln 2 / 60)` for `dt` in whole seconds (values for `dt < 64` come from a
table computed with the same expression).

`Increase(persistent, transient)` at wall-clock second `now`:

1. `persistent += persistent argument`; `dt = now − lastUnix`.
2. If the transient argument is > 0: if `dt > 1800` set `transient = 0`; else if
   `transient > 1` and `dt > 0` set `transient *= decay(dt)`; then
   `transient += argument`; `lastUnix = now`.
3. Return `persistent + uint32(transient)` (truncation toward zero).

`Int()` at second `now`: `dt = now − lastUnix`; return `persistent` when `transient < 1`,
`dt < 0` or `dt > 1800`; otherwise `persistent + uint32(transient · decay(dt))`.

Consequences in the wallet: a `getdata` of fewer than 500 entries adds nothing and does not
update `lastUnix`; the disconnect test is `score > 100` on the value returned by `Increase`,
so a single maximal `getdata` (increment 100) is tolerated and a later increment disconnects
only when the decayed remainder plus that increment truncates to 101 or more;
`getpeerinfo.banscore` reports `Int()`, which decays between calls. `Increase` and `Int` read
the wall clock at one-second resolution.

## 5. Constants, limits and defaults

| Name | Value | Source | Depends on it |
|---|---|---|---|
| `Pver` | 11 | `p2p/peering.go:48` | advertised version; `getinfo.protocolversion`; no `addrv2` |
| `minPver` | 9 | `p2p/peering.go:45` | handshake rejection |
| `uaName` | `dcrwallet` | `p2p/peering.go:38` | user agent |
| wire default UA prefix | `/dcrwire:1.0.0/` | `dcrd/wire@v1.8.0/msgversion.go:22` | user agent |
| `reqSvcs` | `SFNodeNetwork` = 1 | `spv/sync.go:37` | peer acceptance, candidate filter, seeder query |
| Dial timeout | 10 s | `p2p/peering.go:623` | connect |
| Handshake timeout | 5 s | `p2p/peering.go:633` | version write + read + verack write |
| Version read deadline | 3 s | `p2p/peering.go:581` | handshake |
| `stallTimeout` | 30 s | `p2p/peering.go:52` | every request; `SendMessage`; `sendMessageAck` |
| Ping interval | 2 min after previous exchange | `p2p/peering.go:705` | keepalive |
| Pong deadline | 15 s | `p2p/peering.go:706` | `ping timeout` disconnect |
| Pong reply queue timeout | 5 min | `p2p/peering.go:856` | |
| Inv trickle period | uniform [100 ms, 500 ms) | `p2p/peering.go:57-60`, `503-506` | per-peer queued tx invs |
| `invLRUSize` | 5000 (holds 5001) | `p2p/peering.go:64` | `InvsSent` window that authorises `getdata` |
| `banThreshold` | 100 (disconnect when score > 100) | `p2p/peering.go:62` | getdata flood, orphan headers |
| getdata ban increment | `len·100/50000` transient | `p2p/peering.go:1312` | |
| Orphan-header ban increment | 10 transient | `p2p/peering.go:2156` | 11 quick orphans disconnect |
| Ban score half-life / lifetime | 60 s / 1800 s | `dcrd/connmgr/v3@v3.1.4/dynamicbanscore.go:18,25` | decay |
| `wire.MaxInvPerMsg` | 50 000 | `dcrd/wire@v1.8.0/invvect.go:18` | inv/getdata chunking |
| `wire.MaxBlockHeadersPerMsg` | 2000 | `dcrd/wire@v1.8.0/msgheaders.go:15` | header batch; "too few headers" rule |
| `wire.MaxBlockLocatorsPerMsg` | 500 | `dcrd/wire@v1.8.0/msggetblocks.go:17` | upper bound never approached |
| `wire.MaxCFiltersV2PerBatch` | 100 | `dcrd/wire@v1.8.0/msgcfiltersv2.go:21` | `cfilterBatchSize` |
| `wire.MaxCFilterDataSize` | 262 144 bytes | `dcrd/wire@v1.8.0/msgcfilter.go:19` | decode limit |
| `wire.MaxHeaderProofHashes` | 32 | `dcrd/wire@v1.8.0/msgcfilterv2.go:18` | decode limit |
| `wire.MaxMessagePayload` | 32 MiB | `dcrd/wire@v1.8.0/message.go:27` | framing |
| cfilter params | B = 19, M = 784931 | `dcrd/gcs/v4@v4.1.1/blockcf2/blockcf.go:33,38` | filter decode/match |
| cfilter watchdog | 2 min | `spv/backend.go:185` | forest reset during initial sync |
| DCP0005 heights | mainnet 431488, testnet3 323328 | `validate/validate.go:19-26` | proof verification gate |
| `targetPeerCount` | 8 | `spv/sync.go:40` | candidate slots |
| `minVersion` / `minVersionTarget` | 10 / 3 | `spv/sync.go:44-45` | `breaksMinVersionTarget` |
| Candidate tries | 100; recency rule for first 30; service rule for first 50 | `spv/sync.go:474-495` | selection |
| Candidate recency window | 10 min since last attempt | `spv/sync.go:492` | selection |
| No-candidate retry delay | 5 s | `spv/sync.go:683` | |
| First-attempt jitter | uniform [0, 200 ms) | `spv/sync.go:695` | |
| Backoff | `min(5 s + 0.95·d, 90 s)`; jitter uniform [−200 ms, +200 ms) | `spv/sync.go:615-624` | reconnect pacing |
| Persistent-peer backoff scale | 1/5 | `spv/sync.go:651` | |
| Required-height slack | 6 blocks | `spv/sync.go:325-327` | handshake rejection `peer is not synced` |
| `stragglerLimit` | 6 blocks | `spv/sync.go:1589` | straggler disconnect |
| `maxAnnHeaderTipDelta` | 256 blocks | `spv/sync.go:1400` | `peer announced old header` |
| cfilter-peer slack | 6 blocks | `spv/backend.go:59` | `pickForGetCfilters` |
| `seenTxs`, `seenMixMsgs` | limit 2000 (hold 2001) | `spv/sync.go:148-149` | duplicate-fetch suppression |
| `mempoolEvictionTimeout` | 60 min | `spv/sync.go:1964` | non-wallet tx serving window |
| Seeder request timeout | 1 min | `p2p/peering.go:368` | startup delay bound |
| Seeder body limit / entries | 4096 bytes / 16 | `p2p/peering.go:395-398` | |
| SPV restart delay (CLI) | 5 s | `dcrwallet.go:552` | |
| addrmgr `needAddressThreshold` | 1000 | `dcrd/addrmgr/v3@v3.0.0/addrmanager.go:157` | `getaddr` after connect |
| addrmgr `dumpAddressInterval` | 10 min | `…/addrmanager.go:161` | `peers.json` writes |
| addrmgr buckets | new 1024 × 64, tried 64 × 256 | `…/addrmanager.go:165-177` | `peers.json` shape |
| addrmgr spread | `triedBucketsPerGroup` 8, `newBucketsPerGroup` 64, `newBucketsPerAddress` 8 | `…/addrmanager.go:181-189` | bucket placement |
| addrmgr badness | `numMissingDays` 30, `numRetries` 3, `maxFailures` 5, `minBadDays` 7 | `…/addrmanager.go:193-205` | expiry |
| addrmgr `serialisationVersion` | 1 | `…/addrmanager.go:218` | `peers.json` |
| addrmgr effective new-bucket capacity | 65 (`len > 64` test) | `…/addrmanager.go:286` | `peers.json` bucket lengths (Q-44) |
| addrmgr selection weight | `minChance` 0.01; recent-attempt window 10 min; decay base 1.5 per failed attempt | `dcrd/addrmgr/v3@v3.0.0/knownaddress.go:68-78` | `GetAddress` |
| addrmgr selection loop | acceptance scale `2^30`; factor growth 1.2 per rejected draw | `…/addrmanager.go:776-822` | `GetAddress` |
| addrmgr `isBad` windows | 1 min since last attempt; +10 min future timestamp | `dcrd/addrmgr/v3@v3.0.0/knownaddress.go:94-100` | new-bucket expiry |
| addrmgr `Connected` timestamp refresh | only when older than 20 min | `…/addrmanager.go:868-871` | address timestamps |
| addrmgr known-address re-add probability | `1/(2·refs)` while `refs < 8` | `…/addrmanager.go:255-264` | spread over new buckets |
| addrmgr temp file suffix | `.new` | `…/addrmanager.go:468` | `peers.json.new` |
| Ban score decay table | 64 precomputed factors (`dt` 0…63 s) | `dcrd/connmgr/v3@v3.1.4/dynamicbanscore.go:29-49` | none (same values as the formula) |
| Announcement fan-in channels | unbuffered (`receivedGetData`, `receivedInv`, `announcedHeaders`) | `p2p/peering.go:173-177` | handler tasks block until the syncer receives |
| Per-request reply channels | capacity 1 (tx, mix, cfilter, batched cfilter, headers, init state, pong) | `p2p/peering.go:559`, `1586`, `1656`, `1725`, `1896`, `2000`, `2177` | a reply never blocks its handler |
| `CFiltersV2` send-record channel | capacity = number of hashes | `p2p/peering.go:1790` | sender never blocks on bookkeeping |
| `mempoolAdds` channel | unbuffered | `spv/sync.go:151` | `PublishTransactions` blocks until `handleMempool` takes the hash |
| Missing-cfilter / rescan progress channels | capacity 1 | `spv/sync.go:413`, `1891` | wallet progress reporting |
| Filter-match workers | at most `NumCPU` (`scanChain`, spawned on demand); exactly `NumCPU` (`Rescan`, job channel capacity `NumCPU`) | `spv/sync.go:1316-1347`; `spv/backend.go:549-568` | CPU use only |
| addrmgr `peersFilename` | `peers.json` | `…/addrmanager.go:25` | |
| Default p2p ports | mainnet 9108, testnet3 19108, simnet 18555, regnet 18655 | `dcrd/chaincfg/v3@v3.3.0/*params.go` | `--spvconnect` normalization |
| Testnet3 checkpoint | height 962928 | `wallet/checkpoints.go:22-34` | `BadCheckpoint` |

Configuration surface (owned by the configuration spec; listed for completeness):

| Option | Effect here | Citation |
|---|---|---|
| `--spv` | selects `spvLoop`; incompatible with `--offline` and `--enablevoting` | `config.go:144`, `844-854`; `dcrwallet.go:437-438` |
| `--spvconnect` (repeatable) | persistent peers; each normalized with the network default port (not de-duplicated); requires `--spv` | `config.go:145`, `855-865` |
| `--spvdisablerelaytx` | relay byte 0 in `version`; tx/mix invs from peers become a protocol violation | `config.go:146`; `dcrwallet.go:538` |
| `--proxy`, `--proxyuser`, `--proxypass`, `--torisolation`, `--circuitlimit` | `cfg.dial`: SOCKS5 for everything except `localhost`, loopback and RFC 1918 / RFC 4193 literals; failures wrapped as `proxy dial <net> <addr>: <err>` | `config.go:670-728` |
| gRPC `SpvSyncRequest.spv_connect` | persistent peers, each normalized with the default port; error `SPV Connect address invalid: <err>` (FailedPrecondition) | `internal/rpc/rpcserver/server.go:3136-3146` |

---

## 6. Concurrency and lifecycle

### 6.1 Tasks

Per `Syncer.Run`:

| Task | Lifetime | Notes |
|---|---|---|
| `receiveGetData` | group | one sub-task per `getdata`; waits for them on exit |
| `receiveInv` | group | up to three sub-tasks per `inv`; waits for them on exit |
| `receiveHeadersAnnouncements` | group | one untracked sub-task per announcement |
| `connectToCandidates` or N × `connectToPersistent` | group | candidate mode: ≤ 8 `connectAndRunPeer` sub-tasks |
| `handleMempool` | group | one timer sub-task per stored tx |
| initial sync | group | ends with nil after closing `initialSyncDone` |
| `wallet.Run` wrapper | group | always ends with an error once `walletCtx` ends |
| walletCtx canceller | detached | |
| address manager dump loop | from `Start` to `Stop` | first `Run` only (Q-02) |

Per connected peer: closer, reader (plus one short-lived task per received message), writer
(plus an inner task that can outlive the peer while idle), trickler, pinger; plus the
`connectAndRunPeer` task that owns it.

### 6.2 Shared state and locks

| Lock | Protects | Held across blocking I/O? |
|---|---|---|
| `LocalPeer.rpMu` | `rpByID`, `rpIDCounter` | no |
| `LocalPeer.mask`, `requireHeight` | atomics | — |
| `RemotePeer.requestedBlocksMu` | block request table and each request's fields | no |
| `RemotePeer.requestedTxsMu`, `requestedMixMsgsMu` | request tables | no |
| `RemotePeer.requestedCFiltersV2`, `requestedManyCFiltersV2` | concurrent maps | — |
| `RemotePeer.requestedHeadersMu` | pending headers request, locators, sendheaders flag | no |
| `RemotePeer.requestedInitStateMu` | pending init-state request | no |
| `RemotePeer.lastHeightMu` | `lastHeight` | no |
| `RemotePeer.waitingInvsMu` | trickle queue | **yes**: the trickler sends while holding it |
| `RemotePeer.closed` | atomic; `err` is written once before `errc` closes | — |
| `Syncer.remotesMu` | `remotes`, `connectingRemotes`, `remoteAvailable` | **yes**: `forRemotes` callbacks (`SendMessage` up to 30 s per peer) |
| `Syncer.filterMu` | precise filter, `filterData` | no (CPU only) |
| `Syncer.sidechainMu` | sidechain forest and the `FilterV2` fields of its nodes | **yes**: all of `handleBlockAnnouncements`; wallet DB transactions in `getHeaders` and stage 3 |
| `Syncer.backoffsMu` | `backoffs` | no |
| `Syncer.walletSynced` | atomic | — |
| `Syncer.mempool` | concurrent map | — |
| `lru.Cache.mu` | each cache | no |
| global `blake256Mu` | shared mix-message hasher | no |

Observed acquisition orders (a port must not invert them): `sidechainMu` → wallet database
transaction → (`remotesMu` | `filterMu`); `remotesMu` → per-peer locks and LRU locks;
`waitingInvsMu` → outbound queue. `filterMu` and `remotesMu` are never held together.
`ChainSwitch` calls back into `Syncer.LoadTxFilter` (taking `filterMu`) while the caller holds
`sidechainMu`.

### 6.3 Database transaction boundaries

The syncer opens no transaction itself. Each wallet call it makes is one self-contained view
or update transaction: `RescanPoint`, `BlockHeader`, `BlockInMainChain`, `BlockLocators`,
`ValidateHeaderChainDifficulties`, `EvaluateBestChain`, `CFilterV2`, `GetTransactionsByHashes`,
`DetermineRelevantTxs`, `UnminedTransactions` are read-only; `ChainSwitch` and
`AddTransaction` are single update transactions. Consequently the steps of
`handleBlockAnnouncements` and `getHeaders` are **not** atomic with respect to the database;
consistency relies on `sidechainMu` serializing all chain-changing paths of the syncer.

### 6.4 Cancellation and shutdown

- Outer context cancelled → `walletCtx` cancelled → the `wallet.Run` wrapper (and the initial
  sync task if still running) returns → the group context is cancelled → every group task
  returns → every peer's `serveUntilError` context (a child of the group context) ends →
  closer disconnects with `context.Canceled` and closes the socket.
- A group task failing has the same effect minus the first step.
- `unsynced()` teardown cancels the group context directly and forces the `NoPeers` result.
- `Run` returns only after all group tasks (including all `connectAndRunPeer` sub-tasks via
  the candidate wait group, and the getdata/inv sub-tasks) have finished. Announcement
  handler sub-tasks and mempool timers are not awaited.
- After `Run` returns: the wallet has no network backend (`NetworkBackend()` yields a NoPeers
  error), `peers.json` has been written (first run only), and every `WrapContext` child is
  cancelled.

### 6.5 Ordering guarantees visible to callers

- Requests on one peer are written in the order they were queued; `ping`/`pong` overtake
  queued messages.
- Handlers for two messages received from the same peer may run in either order (§4.4).
- Chain switches are totally ordered (by `sidechainMu`); `TipChanged` is delivered inside the
  lock, in chain order.
- `PeerConnected` is emitted after the peer is registered and before `peerStartup` runs.
  Initial-sync requests select peers through `waitForRemote`, so they can be sent to a peer as
  soon as it is registered, possibly before the notification is delivered.

---

## 7. Errors

### 7.1 Error values produced

Kinds refer to the wallet `errors.Kind` enumeration; an `errors.E` value formats as
`<op>: <kind text>: <message>` (kind text: Protocol = `protocol violation`, IO = `I/O error`,
Invalid = `invalid operation`, Consensus = `consensus violation`, Policy = `policy violation`,
NotExist = `item does not exist`, NoPeers = `Decred network is unreachable`). That form holds
for a single (non-nested) value. When an `errors.E` value wraps another one with a different
op, the two are joined by `errors.Separator` (default `":\n\t"`; the daemon sets it to `":: "`
at `dcrwallet.go:38`), the inner kind is promoted to the outer value and printed once, after
the outer op: for example a handshake read failure surfaces from `ConnectOutbound` as
`localpeer.ConnectOutbound(<addr>): I/O error:: p2p.handshake: <transport error>`
(`errors/errors.go:187-290`).

| Message | Kind | Op | Produced by | Peer disconnected |
|---|---|---|---|---|
| `first received message was not the version message` | Protocol | `p2p.handshake` | handshake | connection abandoned |
| `remote peer has pver lower than minimum required` | Protocol | `p2p.handshake` | handshake | connection abandoned |
| (transport error) | IO | `p2p.handshake` | handshake read/write | connection abandoned |
| `newNetAddress: unsupported address type %T` | — | `p2p.handshake` | version build | connection abandoned |
| `missing required service flags <flags>` | — | `localpeer.ConnectOutbound(<raddr>)` | services check | yes |
| `peer is not synced` | — | `localpeer.ConnectOutbound(<raddr>)` | height check | yes |
| `received unexpected version message` | Protocol | — | reader | yes |
| `received unrequested block` | Protocol | `remotepeer(<a>).receivedBlock(<h>)` | block handler | yes |
| `received empty cfiltersv2 message` | Protocol | `remotepeer(<a>).receivedCFilterV2(<zero hash>)` | batched cfilter handler | yes |
| `received unrequested many cfilter` | Protocol | `remotepeer(<a>).receivedCFilterV2(<h>)` | batched cfilter handler | yes |
| `received unrequested cfilter` | Protocol | `remotepeer(<a>).receivedCFilterV2(<h>)` | cfilter handler | yes |
| `received unrequested headers` | Protocol | `remotepeer(<a>).receivedHeaders` | headers handler | yes |
| `peer sent too few headers` | Protocol | same | headers handler | yes |
| `peer sent headers that do not connect to block locators` | Protocol | same | headers handler | yes |
| `received out-of-sequence headers` | Protocol | same | headers handler | yes |
| `received notfound for unrequested hash` | Protocol | (none) | notfound handler | yes |
| `received unrequested tx` | Protocol | `remotepeer(<a>).receivedTx(<h>)` | tx handler | yes |
| `received unrequested mix msg` | Protocol | `remotepeer(<a>).receivedMixMsg(<h>)` | mix handler | yes |
| `ban score reached` | Protocol | — | getdata handler | yes |
| `ban score reached due to orphan header` | Protocol | — | `ReceivedOrphanHeader` | by caller |
| `received unrequested init state` | Protocol | `remotepeer(<a>).receivedInitState` | initstate handler | yes |
| `received tx in msginv when tx relaying is disabled` | Protocol | `remotepeer(<a>).receivedInv` | inv handler | yes |
| `ping timeout` | IO | — | pinger | yes |
| `pong contains nonmatching nonce` | Protocol | — | pinger | yes |
| `peer appears stalled` | IO | request op (none for blocks) | every request method | yes |
| `tx <h> is already being requested from this peer` | — | `remotepeer(<a>).Transactions` | `Transactions` | no |
| `mix msg <h> is already being requested from this peer` | — | `remotepeer(<a>).MixMessages` | `MixMessages` | no |
| `cfilterv2 is already being requested from this peer for this block` | Invalid | cfilter op | cfilter methods | no |
| `protocol version <v> is too low to fetch cfiltersv2 from this peer` | Protocol | cfilter op | cfilter methods | no |
| `protocol version <v> is too low to receive block header announcements` | Protocol | `remotepeer(<a>).SendHeaders` | `SendHeaders` | no |
| `protocol version <v> is too low to publish mix messages` | Protocol | `remotepeer(<a>).PublishMixMessages` | peer-level publish | no |
| `synchronous getheaders after sendheaders is unsupported` | Invalid | `remotepeer(<a>).Headers` | `Headers` | no |
| `headers are already being requested from this peer` | Invalid | `remotepeer(<a>).Headers` | `Headers` | no |
| `asynchronous getheaders before sendheaders is unsupported` | Invalid | `remotepeer(<a>).Headers` | `HeadersAsync` | no |
| `init state is already being requested from this peer` | Invalid | `remotepeer(<a>).GetInitState` | `GetInitState` | no |
| `transaction not found` (`ErrNotFound`) | NotExist | — | `Transactions` | no |
| `peer has been disconnected` (`ErrDisconnected`) | — | — | nil disconnect reason | — |
| `no addresses` | — | — | `peerCandidate` | — |
| `peer uses too low version to satisfy minimum target version count` | — | — | `connectAndRunPeer` | yes |
| `peer did not provide headers through advertised height` | Protocol | — | `getHeaders` | yes |
| `peer returned unexpected number of filters (got <g>, want <w>)` | Protocol | — | `cfiltersV2FromNodes` | yes |
| `CFilter for block <h> (height <n>) received from <peer> failed validation: <err>` | Protocol | — | `cfiltersV2FromNodes` | yes |
| `getCFilters watchdog triggered` | — | — | `cfiltersV2FromNodes` | no (internal) |
| `peer announced old header` | Protocol | `spv.handleBlockAnnouncements(<a>)` | announcements | by caller |
| `disconnecting from straggler peer (peer height <p>, tip height <t>)` | Policy | — | `disconnectStragglers` | yes |
| `invalid regular merkle root` / `invalid stake merkle root` / `invalid combined merkle root` | Consensus | `validate.*(<h>)` | `blocksFromPeer` | yes |
| `invalid header inclusion proof for cfilterv2` | Consensus | `validate.CFilterV2HeaderCommitment(<h>)` | proof check | by caller |
| `stake difficulty is not queryable over wire protocol` | Invalid | — | `StakeDifficulty` | — |
| `no connected peers support the mixing protocol version` | Protocol | `spv.PublishMixMessages` | publish | — |
| (no message) | NoPeers | — | `forRemotes` with no peers; `Run` teardown | — |

### 7.2 How errors surface

| Situation | Surface |
|---|---|
| Peer-level errors | log lines of §7.3; never reach RPC clients directly |
| `Run` returns non-nil (CLI) | error log `SPV synchronization stopped: <err>`; retry after 5 s; the process does not exit |
| `Run` returns (gRPC `SpvSync`) | `Canceled` `SPV synchronization canceled: <err>`; `DeadlineExceeded` `SPV synchronization deadline exceeded: <err>`; otherwise the generic translation (NoPeers → `Unavailable`) |
| `NetworkBackend` method errors | propagate to the wallet operation and from there to the RPC caller; NoPeers maps to JSON-RPC code −9 (`ErrRPCClientNotConnected`) and gRPC `Unavailable` (`internal/rpc/jsonrpc/errors.go:28-44`, `internal/rpc/rpcserver/server.go:116-117`) |
| Wallet has no backend (between `Run` invocations) | `wallet.NetworkBackend()` returns `errors.E(Op("wallet.NetworkBackend"), errors.NoPeers)` |

### 7.3 Log output

Subsystems: `PEER` (p2p), `SYNC` (spv), `AMGR` (addrmgr). Exact texts are given inline in §4.
Info-level and above (visible at the default level) for quick reference:

| Level | Subsystem | Text |
|---|---|---|
| info | SYNC | `Headers synced through block <hash> height <n>` |
| info | SYNC | `Transactions synced through block <hash> height <n>` |
| info | SYNC | `New peer <addr> <ua> version=<pver> <services>` |
| info | SYNC | `Lost peer <addr>` |
| warn | SYNC | `Lost peer <addr>: <err>` |
| warn | SYNC | `Peering attempt failed: <err>` |
| warn | SYNC | `Unable to complete startup sync with peer <addr>: <err>` |
| info | SYNC | `Connected block <hash>, height <n>` / `Connected <k> blocks, new tip <hash>, height <n>, date <time>` / `Connected block <hash>, height <n>, <k> wallet transaction(s)` |
| info | SYNC | `Reorganize from <hash> to <hash> (total <n> block(s) reorged)` |
| info | SYNC | `Received sidechain or orphan block <hash>, height <n>` |
| warn | SYNC | `Disconnecting peer <addr>: <err>` |
| warn | SYNC | `Disconnecting from <addr> due to header validation error: <err>` |
| warn | SYNC | `Failed to handle headers announced by <addr>: <err>` / `Failed to handle blocks inventoried by <addr>: <err>` |
| warn | SYNC | `Batch of CFilters ending on block <hash> at height <n> not received within 2m0s. Clearing sidechain forest to retry with different headers` |
| warn | SYNC | `CFilter for block <hash> (height <n>) received from <addr> failed validation: <err>` |
| warn | SYNC | `Failed to look up transactions for getdata reply to peer <addr>: <err>` / `Failed to send getdata reply to peer <addr>: <err>` / `Failed to send notfound reply to peer <addr>: <err>` |
| error | SYNC | `Failed to get init state: <err>` |
| error | SYNC | `Cannot load unmined transactions for resending: <err>` / `Failed to resent one or more unmined transactions: <err>` |
| error | SYNC | `Failed to cleanly stop address manager: <err>` |
| info / error | SYNC | `SPV synchronization stopped` / `SPV synchronization stopped: <err>` |
| warn | PEER | `Failed to seed addresses from <host>: <err>` and the seeder parse warnings of §4.1 |
| warn | PEER | `<addr>: ban score reached threshold` |
| info | AMGR | `Loaded <n> addresses from file '<path>'`, `Address manager shutting down` |
| warn | AMGR | `Address manager is already in the process of shutting down` |
| error | AMGR | `Failed to parse file <path>: <err>` and the save-failure errors |
| warn | AMGR | `Failed to remove corrupt peers file <path>: <err>` |
| error | AMGR | save failures, each aborting that save: `Error opening file <path>.new: <err>`, `Failed to encode file <path>.new: <err>`, `Error closing file <path>.new: <err>`, `Error writing file <path>: <err>` |
| error | PEER | `Bad seeder request: <err>` |

`<err>` in the AMGR parse line is one of (`dcrd/addrmgr/v3@v3.0.0/addrmanager.go:512-600`):
`<path> error opening file: <os err>`; `error reading <path>: <json err>`;
`unknown version <n> in serialized addrmanager`;
`failed to deserialize netaddress <key>: <err>` (for `Addr` or `Src`; the inner error is a
`net.SplitHostPort` or `strconv.ParseUint` error, or `failed to deserialize address <key>`
for a non-IP host); `new buckets contains <key> but none in address list`;
`tried buckets contains <key> but none in address list`;
`address <key> after serialisation with no references`;
`address <key> after serialisation which is both new and tried`.

Debug and trace lines not quoted elsewhere: PEER debug `Disconnecting <addr>` on every first
`Disconnect` (`p2p/peering.go:730`); AMGR trace `Starting address manager`,
`Address handler done`, `new bucket is full, expiring old`, `expiring bad address <key>`,
`expiring oldest address <key>`, `Added new address <key> for a total of <n> addresses`,
`Selected <key> from tried bucket`, `Selected <key> from new bucket`,
`Replacing <old key> with <new key> in tried`
(`dcrd/addrmgr/v3@v3.0.0/addrmanager.go:287-330`, `416`, `615`, `792`, `818`, `975`). The
`p2p` and `spv` package loggers default to a disabled logger until `UseLogger` is called
(`p2p/log.go:9`, `spv/log.go:9`).

The `date` in the multi-block line is Go's default `time.Time` formatting of the header
timestamp (local time zone, e.g. `2026-11-05 09:20:09 -0500 EST`).

---

## 8. Compatibility-critical behaviours

Each item names the external party that depends on it.

1. **Protocol version 11, minimum 9** (dcrd peers). Advertise exactly 11 so dcrd never sends
   `addrv2` (version 12) and does offer batched cfilters; reject peers below 9; encode and
   decode everything after the handshake at `min(11, remote)`.
2. **`version` contents** (dcrd peers, network crawlers): services 0, `LastBlock` 0, zero
   `AddrMe`, `AddrYou` hidden for non-public addresses, relay byte from
   `--spvdisablerelaytx`, user agent `/dcrwire:1.0.0/dcrwallet:<version>/` (§3.2). How dcrd
   and crawlers act on these values is outside this scope (UNVERIFIED); they are what every
   deployed dcrwallet sends, so they are reproduced as is.
3. **Handshake sequencing** (dcrd): send `version` first without waiting for the remote's;
   send `verack` after reading the remote `version`; tolerate the remote `verack` arriving at
   any later time; never send a second `version`.
4. **Request discipline** (dcrd): the complete set of requests the wallet issues is
   `getheaders` with the §3.4 locators and zero stop hash, `getcfilterv2`, `getcfsv2` with
   start/end spanning ≤ 100 consecutive blocks on one chain, `getdata` for blocks/txs/mix,
   `getinitstate` with the single type `tspends`, `getaddr` at most once per connection.
5. **`sendheaders` after initial sync** (dcrd): each peer is sent `sendheaders` exactly once,
   followed immediately by a `getheaders` from the wallet tip; from then on new blocks arrive
   as `headers`, and the wallet must accept `headers` at any time from that peer.
6. **Keepalive** (remote peers' idle handling): answer every `ping` with `pong` carrying the
   same nonce, ahead of queued traffic; send a `ping` about every 2 minutes.
7. **Replies to peers' state queries**: an empty `initstate` to `getinitstate` and an empty
   `miningstate` (version 1, height 0) to `getminings`.
8. **`getdata` service contract** (dcrd and other SPV wallets): serve a `tx` or mix message
   only if its hash was announced to that peer and is still in that peer's 5001-entry
   `InvsSent` window; answer everything else with one `notfound`; send data messages before
   the `notfound`. Own pair-request hashes referenced by an announced key exchange count as
   announced (§4.23).
9. **Inventory announcements** (dcrd mempool/mixpool relay): wallet-initiated publishes send
   one immediate `inv` per peer listing all hashes; startup re-announcement of unmined wallet
   transactions goes through the 100-500 ms trickle queue; mix invs go only to peers with
   pver ≥ 10; transactions are never pushed unsolicited.
10. **Header acceptance rules** (dcrd, hostile peers): the four checks of §4.5 including the
    "fewer than 2000 headers must reach the advertised height" rule and locator-parent rule;
    proof-of-work and stake-difficulty validation through the wallet before anything is
    stored; testnet3 checkpoint rejection during initial sync.
11. **cfilter authentication**: every version-2 filter used for a post-DCP0005 block must
    carry a valid inclusion proof against the header's stake root, with filter parameters
    B = 19, M = 784931 and key = first 16 bytes of the header merkle root; activation heights
    431488 (mainnet) and 323328 (testnet3) gate the check exactly.
12. **Block authentication**: every downloaded block must match either the split merkle
    roots or the DCP0005 combined root before its transactions are used.
13. **Transaction matching semantics** (existing `wallet.db` contents): the set of
    transactions handed to the wallet for a block is exactly what §4.24 produces (stake tree
    first, coinbase inputs and vote stakebase skipped, outputs matched by extracted address,
    newly found outputs added to the watched outpoints during the same pass). The stored
    transaction history depends on it.
14. **Processed-transactions marker**: nodes passed to `ChainSwitch` from announcements carry
    a non-nil (possibly empty) relevant-transaction list only when no rescan is pending;
    initial-sync nodes carry nil. This decides when the database's rescan point advances.
15. **`peers.json`** (operators, downgrade/upgrade in place): same path, same JSON schema and
    version 1, key as a 32-element number array, 1024 new buckets and 64 tried buckets,
    `-62135596800` for unset times, temp file `peers.json.new`, atomic rename; a file written
    by dcrwallet must load, and a file written by the replacement must load in dcrwallet
    (which rejects non-IP address keys and deletes the file on any inconsistency).
16. **Seeder protocol** (seeder operators): `GET /api/addrs?services=1` over HTTPS on the three
    hard-coded hosts per network; tolerate a body of concatenated JSON objects; honour the
    4096-byte and 16-entry limits so behaviour under large responses is unchanged.
17. **Peer counts** (network load, privacy): at most 8 discovered peers including in-flight
    attempts; exactly one connection loop per `--spvconnect` entry and no discovery or seeding
    when any is given.
18. **`--proxy` routing** (privacy): every peer connection and every seeder request made by
    the CLI daemon goes through the configured dial function; loopback and private literals
    bypass the proxy.
19. **RPC-visible peer data** (dcrctl, Decrediton): `getpeerinfo` fields of §3.7 including
    the `%08d` services string, negotiated version, id ordering; `walletinfo.daemonconnected`
    = at least one connected peer; `walletinfo.spv` true; `syncstatus` computed from
    `Synced()` with the target height rule of §2.2; `getinfo.protocolversion` = 11.
20. **gRPC `SpvSync` stream** (Decrediton): notification sequence of §4.26, peer count
    semantics, `Synced(false)` followed by stream termination with `Unavailable` when the
    last peer is lost after sync, `Canceled` when the client cancels.
21. **Wallet-visible backend contract**: `Blocks` and `CFiltersV2` block until served or
    cancelled; `StakeDifficulty` always fails with the Invalid error; `PublishTransactions`
    with no peers fails with NoPeers (JSON-RPC −9); `PublishMixMessages` fails with the
    Protocol error when no peer supports mixing; `WrapContext` children are cancelled when the
    syncer stops.
22. **Mix message relay** (other StakeShuffle participants): fetch every unseen announced mix
    message, feed it to the mixpool with the peer as source, and fetch missing pair requests
    referenced by orphan key exchanges from the same peer, accepting only messages from the
    identities that triggered the fetch.
23. **Treasury spends** (voting wallets fed by SPV for policy display): request `tspends`
    init state from each peer, accept only version-3 treasury spends signed by a known Pi key
    whose expiry has not passed.
24. **Log lines** (operators, log scrapers): the info/warn/error texts of §7.3 under the
    `SYNC`, `PEER` and `AMGR` subsystem tags.
25. **`peers.json` structural invariants** (Go dcrwallet loading a file written by the
    replacement): every address listed in `Addresses` must appear in at least one new bucket
    or in exactly one tried bucket and never in both; every bucket entry must be listed in
    `Addresses`; `Addr` and `Src` must be IP-literal `host:port` keys; a new bucket may hold
    up to 65 keys and an address may sit in up to 8 new buckets. Violations make Go delete
    the file (§3.6). Bucket placement itself is not verified on load (§4.27).
26. **Address book admission** (contents of `peers.json`, peer selection): only routable
    addresses are stored; never-attempted addresses carry the minimum selection weight; an
    address is promoted to "tried" only after a handshake that also passed the service and
    height checks (§4.2 step 9, §4.27).
27. **gRPC `SpvSync` preconditions and notification types** (Decrediton): the three
    precondition errors and their status codes, the unlock-for-discovery sequence with the
    relock at `DISCOVER_ADDRESSES_FINISHED`, and the callback → `notification_type` mapping
    of §4.12; a second `SpvSync` (or any sync) while one is attached must fail with
    `FailedPrecondition`.
28. **Ban score visibility** (dcrctl `getpeerinfo`): `banscore` is the decaying value of
    §4.28 computed at call time with one-second resolution and truncation toward zero.

---

## 9. Quirks

**Q-01 — LRU holds `limit + 1` entries.** Eviction tests `len(m) > limit` before inserting, so
an eviction happens only when the set already holds `limit + 1`. `InvsSent` therefore covers
5001 hashes and `seenTxs`/`seenMixMsgs` 2001. `lru/cache.go:47-54`, `lru/map.go:52-59`.

**Q-02 — the address manager works for the first `Run` only.** `spvLoop` creates one
`AddrManager` and reuses it for every restart. `Start` and `Stop` are guarded by counters that
are never reset: on the second and later runs `Start` does nothing (no reload, no dump loop)
and `Stop` logs `Address manager is already in the process of shutting down` and writes
nothing. After the first syncer restart, discovered addresses are kept in memory but
`peers.json` is never written again until the process restarts. `dcrwallet.go:534-545`;
`dcrd/addrmgr/v3@v3.0.0/addrmanager.go:609-639`. Reproduce or fix deliberately (§12).

**Q-03 — gRPC `SpvSync` ignores the proxy and the relay flag.** The handler builds its
`LocalPeer` without `SetDialFunc` or `SetDisableRelayTx`, so peers and seeders are dialled
directly even when `--proxy` is configured. `internal/rpc/rpcserver/server.go:3014-3016`
versus `dcrwallet.go:536-538`.

**Q-04 — no ban list.** A misbehaving peer is disconnected and nothing else: its address stays
in the address manager and is eligible again subject to the per-address backoff, which is
keyed by address string and lost when `Run` restarts. `connmgr.DynamicBanScore` is used only
for oversized `getdata` and orphan header announcements. `p2p/peering.go:1312-1318`,
`2154-2162`; `spv/sync.go:626-643`.

**Q-05 — any undecodable or unknown message disconnects.** Unlike dcrd's peer, the read loop
treats every `wire.ReadMessage` error as fatal, including unknown commands and `reject`.
`p2p/peering.go:794-853`.

**Q-06 — the remote `verack` is never required.** A peer that never sends `verack` is fully
usable. The remote nonce is not compared with the local one. `p2p/peering.go:606`.

**Q-07 — `notfound` for a mix message or a block disconnects the peer.** The handler consults
only the transaction request table. `MixMessages`' documentation promises `ErrNotFound` with
nil entries, which cannot occur; the syncer's NotExist branch for mix messages is dead code.
The inv type of a `notfound` entry is ignored, so a block-typed `notfound` carrying a pending
transaction hash resolves that transaction request. `p2p/peering.go:1194-1217`,
`1639-1644`; `spv/sync.go:1199-1210`.

**Q-08 — lost op in the notfound error.** `errors.Errorf` is used where `errors.Opf` was
intended; the formatted op is discarded. `p2p/peering.go:1210-1211`.

**Q-09 — an unsolicited `pong` kills the connection later.** It sits in the one-slot buffer
until the next ping cycle and fails the nonce comparison. A second unsolicited pong parks a
goroutine until the peer ends. `p2p/peering.go:945-971`.

**Q-10 — `waitForRemote` can spin.** With `waitEligible` true, peers connected and none
eligible, the loop re-locks and re-scans without sleeping and without checking the context,
so the cfilter watchdog cannot fire until the peer set changes. `spv/sync.go:742-773`,
`spv/backend.go:192-205`.

**Q-11 — handlers run concurrently per message.** Two `headers` announcements from one peer
can reach the syncer in either order; a `tx` and the `notfound` after it are unordered.
`p2p/peering.go:802-850`.

**Q-12 — failed handshakes leak the socket; persistent peer names are resolved locally.** The
connection is not closed when the handshake fails (it is reclaimed by the Go runtime's
finalizer). `ConnectOutbound` resolves the address with the system resolver before dialling,
so a host name given to `--spvconnect` is looked up outside the proxy, and then looked up again
by the proxy when the original string is dialled. `p2p/peering.go:260`, `624-638`.

**Q-13 — block locators repeat the tip and may omit genesis.** §3.4.
`wallet/wallet.go:1510-1532`.

**Q-14 — stall timers differ per request type.** Transactions and mix messages: one 30 s
budget for the whole batch. Blocks: 30 s per block, counted in request order. Pipelined
cfilters: 30 s from the moment each request was queued. Batched cfilters, headers, init
state: 30 s from the call (including time spent waiting for the write queue).
`p2p/peering.go:1457-1497`, `1605-1631`, `1813-1862`.

**Q-15 — batched cfilter replies are loosely matched.** Only the first entry's block hash is
used to find the request; the count and the remaining hashes are not checked at the p2p layer
and the op strings say `CFilterV2`. Correctness comes from the count check and the per-header
inclusion proofs in the syncer. For pre-DCP0005 heights on mainnet/testnet3 no proof is
checked on any path (single or batched), so filters for those blocks are accepted
unauthenticated during sync; the wallet's separate bulk check (`validatepredcp0005cf`,
`validate.PreDCP0005CFilterHash`) is outside this scope. `p2p/peering.go:1016-1041`, `1886-1940`; `spv/backend.go:215-247`.

**Q-16 — the checkpoint is enforced only during initial sync.** `BadCheckpoint` marking is
applied in `getHeaders` but not in `handleBlockAnnouncements`. `spv/backend.go:340-342`;
`spv/sync.go:1465-1486`.

**Q-17 — initial sync can finish after a failed last chain switch.** When stage 3 fails to
attach the final real batch it disconnects the source peer and invalidates, but if stage 1
has already emitted the `done` batch the pipeline completes normally and the rescan proceeds
from the older tip; the missing blocks arrive later through announcements.
`spv/sync.go:1667-1711`, `1786-1817`.

**Q-18 — repeated `getheaders` to a peer that has nothing new.** An empty reply from a peer
whose advertised height is still above the (lagging) wallet tip leaves it eligible, so stage 1
can ask it again each round trip until stage 3 catches up. `spv/backend.go:306-324`.

**Q-19 — announcement handling refetches filters and blocks the world.** Every announcement
that produces a better chain requests a filter for every node of that chain from the
announcing peer, one `getcfilterv2` each, while holding `sidechainMu`.
`spv/sync.go:1497-1520`.

**Q-20 — address manager load path.** (a) `TimeStamp` is written but not restored; loaded
addresses look freshly seen. (b) Service flags are not persisted; loaded addresses are assumed
`SFNodeNetwork`. (c) A failed load deletes the file and calls `reset()`, which does not zero
the new/tried counters; in v3.0.0 `GetAddress` picks buckets blindly and retries while the
chosen bucket is empty, so stale counters over empty buckets make `GetAddress` loop while
holding the manager mutex. `dcrd/addrmgr/v3@v3.0.0/addrmanager.go:536-556`, `723-741`,
`766-824`. dcroxide records (a) and the counter half of (c) as its QK-0004 and QK-0023
(`dcroxide:docs/QUIRKS.md`).

**Q-21 — seeder client laxity.** HTTP status ignored; decode target reused across objects;
source address is the first returned address; the `services` field of the response is unused;
a request-construction failure would leave the collector waiting forever (unreachable with
valid host names). `p2p/peering.go:366-433`.

**Q-22 — `forRemotes` holds the peer-map lock while sending.** `PublishTransactions` and
`PublishMixMessages` call `SendMessage` (up to 30 s each) under `remotesMu`; during that time
`getpeerinfo`, `walletinfo`, peer registration and peer selection block. Iteration stops at
the first failing peer, so peers later in the (random) iteration order receive nothing and the
publish reports failure even though some peers were told. `spv/sync.go:716-729`;
`spv/backend.go:469-474`.

**Q-23 — `SendMessage` does not notice a dead peer.** It never selects on the peer's
disconnect channel. What happens depends on the inner write loop of Q-38: while that loop is
still parked (the peer was idle when it disconnected), it receives the first message queued
after the disconnect, so that `SendMessage` returns **nil** at once; the write to the closed
socket then fails and the loop exits. Once the loop has exited, every further `SendMessage`
waits the full 30 s and returns `context deadline exceeded`. `p2p/peering.go:455-474`,
`2126-2135`.

**Q-24 — `fetched_headers_count` carries a height.** §4.26.

**Q-25 — a database error in `peerStartup` strands the peer.** It returns nil after logging,
so the peer stays connected without `sendheaders`. `spv/sync.go:1930-1934`.

**Q-26 — "too few headers" also polices announcements.** A sendheaders-mode `headers` message
with fewer than 2000 headers whose last height is below the height the peer advertised at
handshake disconnects the peer (for example a peer announcing a shorter competing branch). A
negative advertised height wraps to a large unsigned value in this comparison.
`p2p/peering.go:1112-1121`.

**Q-27 — `--spvconnect` entries are not de-duplicated.** Two identical entries run two loops
sharing one map key; the second registration replaces the first and the first exit removes the
entry. `config.go:860-865`; `spv/sync.go:546`, `560`.

**Q-28 — persistent peers still gossip addresses.** `getaddr` is sent to persistent peers when
the manager holds fewer than 1000 addresses, and the replies are stored and saved, although
nothing will dial them in persistent mode. `Attempt`/`Connected`/`Good` fail silently for
persistent peers because they are not in the manager. `p2p/peering.go:297-305`, `620-630`.

**Q-29 — dead code and stale comments.** `RemotePeer.InvsRecv` (never populated);
`LocalPeer.receivedHeaders`, `receivedInitState`, `extaddr`; `msgReader.rawMsg`;
`RemotePeer.PublishMixMessages`; `Syncer.DisableDiscoverAccounts`; all of `lru.Map`; the
pver guards in `CFilterV2`, `SendHeaders` and `peerStartup` (always satisfied with
`minPver` 9). Comments in `spv/backend.go` refer to a `wallet.Peer` interface that no longer
exists; the only interface is `wallet.NetworkBackend`. `spv/backend.go:94,121,405,436`.

**Q-30 — `lru.Map.Add` keeps the old value** for an existing key, and `Map.Contains`'s comment
contradicts its code. `lru/map.go:45-49`, `94-101`.

**Q-31 — `MempoolTxs` over-reports.** It receives every filter-relevant transaction, including
tickets skipped in manual-ticket mode and transactions whose `AddTransaction` failed, and is
called with an empty list when nothing matched. `spv/sync.go:1153-1176`.

**Q-32 — duplicate tx hashes in one `inv` drop the whole inv.** `Transactions` rejects a
repeated hash as "already being requested"; `handleTxInvs` logs and gives up.
`p2p/peering.go:1587-1593`; `spv/sync.go:1108-1137`.

**Q-33 — Go-runtime-dependent behaviour.** (a) Map iteration order randomizes peer selection
(`waitForRemote`, `forRemotes`), the order of transactions in `TipChanged`, the order of
missing pair-request hashes in the follow-up `getdata`, and the element order inside
`peers.json`. (b) `select` among simultaneously ready cases is random; in `CFiltersV2` an early sender
exit (duplicate request, cancellation, peer disconnect) can surface as the expected error, as
a stall disconnect, or as a nil-error result with nil filters (§4.9).
(c) The `if !timer.Stop() { <-timer.C }` idiom is the pre-Go-1.23 drain pattern; under the
timer semantics selected by `go 1.26.0` in `go.mod`, `Stop` returns true whenever the value
has not been received, so the drain branch is not taken on these paths (UNVERIFIED at
runtime; no externally observable effect either way). (d) Worker completion order determines the order of block hashes
in filter-match `getdata` messages. `spv/sync.go:310-318`, `722`, `760`, `1254-1256`,
`1311-1349`; `p2p/peering.go:1837-1862`.

**Q-34 — the syncer's task group ignores the caller's context.** It derives from the
background context; outer cancellation reaches it only through the `wallet.Run` wrapper task
returning `walletCtx.Err()`. `spv/sync.go:374-375`, `401-408`, `447-459`.

**Q-35 — required height for short chains.** With a tip height ≤ 6 the required peer height is
the tip height itself, not `tip − 6`. `spv/sync.go:323-329`.

**Q-36 — `ConnectOutbound` starts serving before policy checks.** The service goroutines run
(and can already process inbound messages) before the services and height checks disconnect
the peer; a peer failing them is never marked good, so its address accrues attempts and
backoff. `p2p/peering.go:275-294`.

**Q-37 — ping cadence drifts.** The 2-minute wait restarts after each exchange completes, so
the period is 2 minutes plus the round trip. `p2p/peering.go:700-711`.

**Q-38 — writer goroutine can outlive the peer.** The inner write loop exits only on a write
error; an idle disconnected peer leaves it parked forever. `p2p/peering.go:451-475`.

**Q-39 — `filterData` only grows.** Scripts are appended on every `LoadTxFilter(reload=false)`
without de-duplication; spent outpoints are never removed from the precise filter.
`spv/backend.go:423-430`; `spv/rescan.go:60-67`.

**Q-40 — block invs after `sendheaders` are processed without filter checks.** Every
inventoried block is downloaded in full and rescanned regardless of filter match.
`spv/sync.go:1072-1086`, `1302-1309`, `1366-1373`.

**Q-41 — `sendMessageAck` cannot report a write failure.** The ack is signalled whether or
not the socket write succeeded, so the trickler treats a failed `inv` write as sent and drops
the queued vectors (§4.4). `p2p/peering.go:466-473`, `2139-2150`.

**Q-42 — "DNS seeding" in comments is HTTPS seeding.** The doc comment of `SetPersistentPeers`
and the comment in `Run` speak of DNS seeding; the code performs the HTTPS `/api/addrs`
requests of §3.5 and no DNS seed lookups. `spv/sync.go:158-159`, `368-371`;
`p2p/peering.go:345-434`. Likewise `NewLocalPeer`'s comment says the peer "is externally
reachable … through extaddr" while the wallet never listens (`p2p/peering.go:167-168`), and
`MessageMask`'s comment warns that not receiving masked messages leaks goroutines
(`p2p/peering.go:864-869`): with the mask set and no `ReceiveGetData` consumer, each `getdata`
handler blocks forever on a plain channel send (`p2p/peering.go:1321-1323`).

**Q-43 — never-attempted addresses have the minimum selection weight.** `chance()` returns
0.01 both for an address attempted in the last 10 minutes and for one that was never
attempted (zero `lastattempt`, which includes every freshly seeded or gossiped address and
every loaded address whose stored `LastAttempt` is `-62135596800`). `GetAddress` therefore
needs many rounds of its `factor *= 1.2` loop before returning anything from a fresh book,
and an address that failed once more than 10 minutes ago (weight 0.67) is strongly preferred
over an untried one. `dcrd/addrmgr/v3@v3.0.0/knownaddress.go:63-79`, `addrmanager.go:780-822`.

**Q-44 — a new bucket holds 65 addresses.** The expiry test is `len > 64` before insertion,
the same off-by-one as Q-01; `peers.json` written by Go can therefore list 65 keys in one
`NewBuckets` element. `dcrd/addrmgr/v3@v3.0.0/addrmanager.go:285-289`.

**Q-45 — discovery cannot find non-routable peers.** `AddAddresses` drops loopback, RFC 1918,
CGNAT and the other ranges of §4.27 silently, and simnet/regnet have no seeders, so on those
networks (and for LAN-only peers on any network) the candidate loop logs nothing and retries
`no addresses` every 5 seconds forever; `--spvconnect` is the only way to peer.
`dcrd/addrmgr/v3@v3.0.0/addrmanager.go:224-228`; `spv/sync.go:678-686`.

**Q-46 — counters after a failed promotion.** `Good` decrements `nNew` before it discovers
that the address was in no new bucket, and returns an error the wallet ignores; the counter
then under-reports. An untried address always sits in at least one new bucket, so this is not
reachable through the wallet's own call sequence; a port need not reproduce it.
`dcrd/addrmgr/v3@v3.0.0/addrmanager.go:913-933`.

**Q-47 — unreachable or unused code beyond Q-29.** The error branch after
`rp.PublishTransactions` in `peerStartup` (the method always returns nil), with its log line
`Failed to resent one or more unmined transactions: <err>` (`spv/sync.go:1936-1940`); the
constant `op = "spv.Rescan"` in `Syncer.Rescan`, declared and never used, so `Rescan` errors
carry no op of their own (`spv/backend.go:524`); the `s.rescanFilter == nil` test in
`LoadTxFilter` (the filter is set in `NewSyncer`) (`spv/backend.go:419`); the
`blockMatches[i] != nil` skip in `Rescan` (the slice is freshly allocated)
(`spv/backend.go:569-573`); the `b == nil` skip in `blocksFromPeer` (`Blocks` returns no nil
entry on success) (`spv/backend.go:73-75`); `LocalPeer.RemoveHandledMessages` (no caller);
`RemotePeer.Block`, `RemotePeer.CFilterV2`, `RemotePeer.NA` and `RemotePeer.InvsRecv` (no
caller in the tree).

**Q-48 — TODO comments in scope** (none marks behaviour that exists): reject duplicate
`version` messages properly (`p2p/peering.go:799`; today the read loop returns a Protocol
error); split block `getdata` into batches above 50 000 hashes (`p2p/peering.go:1445-1446`;
today `AddInvVect` fails the whole request); replace the in-memory precise rescan filter with
database lookups (`spv/sync.go:77-78`); compare candidate addresses by network block instead
of exact address (`spv/sync.go:488`); remove double-spent unmined transactions when
re-announcing fails (`spv/sync.go:1938`); track in-flight blocks and cfilters across
`NetworkBackend` calls and cache results (`spv/backend.go:30-33`). There are no `XXX`, `BUG`
or `Deprecated` markers in `p2p/`, `spv/` or `lru/`.

**Q-49 — stage-2 watchdog log names the wrong peer.** The debug line
`Unable to fetch missing cfilters from <peer>: getCFilters watchdog triggered` prints the peer
that supplied the **headers** of the batch, not the peers that failed to serve filters.
`spv/sync.go:1749-1753`.

**Q-50 — `getHeaders` ignores a database error.** The `BlockInMainChain` error is discarded
when deciding which fetched nodes to add to the forest; on error the node is treated as not
in the main chain and added. `spv/backend.go:366-374`.

**Q-51 — concurrent requests for one transaction from one peer collide.** `GetInitState`
(tspend fetch during `peerStartup`) and `handleTxInvs` both call `rp.Transactions` on the
same peer; if both ask for the same hash at once the later caller fails with
`tx <h> is already being requested from this peer` and gives up (logged as
`Failed to get init state: …` or as the `spv.handleTxInvs` warning). `p2p/peering.go:1587-1593`;
`spv/sync.go:1036`, `1118`.

**Q-52 — `discoverAccounts` is sampled at construction.** `NewSyncer` reads `!w.Locked()`
once; unlocking or locking the wallet afterwards does not change whether account discovery
runs (only `DisableDiscoverAccounts`, which has no caller, could). `spv/sync.go:141-144`,
`1870-1874`.

---

## 10. Upstream tests

### 10.1 In scope

There are **no test files** in `p2p/`, `spv/` or `lru/` at the pin, and no test data. Nothing
in the dcrwallet repository exercises the handshake, the request methods, the sync pipeline,
`getdata` serving, or the LRU.

### 10.2 Adjacent upstream tests worth porting or mirroring

| Test file | Pins | Action |
|---|---|---|
| `dcrd/addrmgr/v3@v3.0.0/addrmanager_test.go` (1005 lines) | add/attempt/connected/good transitions, bucket overflow, `peers.json` round trip and corrupt-file handling, `GetAddress` | port as vectors against whichever address manager is used; regenerate `peers.json` fixtures through a Go oracle built against **v3.0.0** |
| `dcrd/addrmgr/v3@v3.0.0/knownaddress_test.go`, `netaddress_test.go`, `network_test.go` | `chance`, `isBad`, key formatting, routability and group keys | port |
| `dcrd/connmgr/v3@v3.1.4/dynamicbanscore_test.go` | decay arithmetic | already mirrored by `dcroxide:crates/dcroxide-connmgr/tests/banscore_vectors.rs` |
| `wallet/sidechains_test.go`, `wallet/reorg_test.go` | forest and chain-switch behaviour the syncer depends on | owned by the wallet chain spec |
| dcrd `wire` tests | message codecs | covered by the wire crate |
| `dcrd/addrmgr/v3@v3.0.0/error_test.go`, `log_test.go` | error kind strings and `errors.Is`/`As` behaviour; logger hook | port only if the error kinds are exposed; the wallet discards every addrmgr error |
| `dcrd/connmgr/v3@v3.1.4/connmanager_test.go`, `error_test.go` | connection manager, not used by the wallet | not applicable |

Untested upstream and specific to this scope (in addition to §10.1): the bucket hash
functions with a fixed key (`addrmanager_test.go:500-503` replaces
`getNewBucket`/`getTriedBucket` with stubs for the tried-eviction test), `chance()` for
never-attempted addresses, the
`Good` eviction path's choice of fallback bucket, the ack-before-result write pump, every
stall-timer rule of Q-14, `breaksMinVersionTarget`, the backoff arithmetic, and the gRPC
`SpvSync` sequencing. UNVERIFIED: whether any other case in `addrmanager_test.go` pins real
bucket indices for a fixed key (only the stub override was located by search; reading the
test file end to end settles whether new Go-oracle vectors are needed for the hashes).

### 10.3 Vectors to create (no upstream equivalent)

1. **Handshake transcript**: byte-exact `version` payloads for (public IPv4, private IPv4,
   IPv6, CGNAT, SOCKS host name) × (relay on/off), generated by a Go shim calling
   `newMsgVersion` logic with a fixed nonce and timestamp.
2. **Block locators**: locator lists for tip heights 0, 1, 9, 10, 11, 100, 1 000 000 with and
   without a sidechain suffix (Go shim over `blockLocators` with a stub store).
3. **Headers acceptance**: accepted/rejected cases for each rule of §4.5 with the exact error
   text.
4. **`getdata` reply ordering** for mixed announced/unannounced, wallet/mempool/mixpool hits
   and misses.
5. **Backoff sequence**: `nextBackoff` iterates from 0 to the cap (integer nanoseconds).
6. **Ban score**: `len·100/50000` increments and decay crossing the threshold.
7. **Filter matching**: `rescanBlock` and `filterRelevant` outputs for blocks containing
   coinbase, votes, tickets, revocations and treasury transactions.
8. **`peers.json`**: fixtures written by the Go address manager v3.0.0, loaded and re-saved.
9. **Seeder parsing**: concatenated-object bodies including truncation at 4096 bytes, more
   than 16 entries, missing fields, non-IP hosts, non-200 status.
10. **End-to-end**: a scripted fake peer (or a simnet dcrd) driving initial sync, reorg via
    announcements, straggler disconnect, cfilter watchdog, and loss of all peers after sync.
    A differential run of Go dcrwallet and the port against the same simnet dcrd, comparing
    the resulting `wallet.db` transaction set and the message transcript, is the strongest
    available oracle.

---

## 11. Rust porting notes

### 11.1 Suggested boundaries

| Crate / module | Mirrors | Contents |
|---|---|---|
| `purse-p2p` | `p2p/` | `LocalPeer`, `RemotePeer`, handshake, read/write pumps, request tables, trickle queue, ping, seeding client. Async (the daemon is I/O bound and every request has a timeout and cancellation). Expose the same request methods so the syncer logic maps one-to-one |
| `purse-spv` | `spv/` | `Syncer`, peer acquisition, initial-sync pipeline, announcement handling, relay, `NetworkBackend` impl, filter matching |
| `purse-lru` (or a module of `purse-p2p`) | `lru/` | the 60-line set with the `limit + 1` capacity; `Map` only if another subsystem turns out to need it |
| address manager | dcrd `addrmgr` v3.0.0 | see reuse notes |

A trait for the transport (dial function) and for the clock/RNG makes the timers (trickle,
backoff jitter, stall, ping) testable; the Go code has no such seam, which is why it has no
tests.

### 11.2 Reuse (crates inspected)

| Need | Candidate | Notes |
|---|---|---|
| Message framing and codecs | `dcroxide-wire` | `read_message(buf, pver, net)` / `write_message` are sans-I/O and already reproduce dcrd's validation order (`dcroxide:crates/dcroxide-wire/src/message.rs:949`). Its `PROTOCOL_VERSION` is 12 (`dcroxide:crates/dcroxide-wire/src/protocol.rs:16`); the wallet must pass 11/negotiated explicitly and must keep its own `Pver` constant |
| Address book + `peers.json` | `dcroxide-addrmgr` | mirrors dcrd **master**, not v3.0.0: adds Tor v3 addresses, typed selection filters and bucket statistics (`dcroxide:crates/dcroxide-addrmgr/src/lib.rs:1-16`, `manager.rs:924`). Serialization version is the same 1 and field names match (`manager.rs:61`, `341-370`). Needs a parity review against v3.0.0 for `GetAddress` (v3.0.0 has no filter and no bucket statistics), and a decision on Q-20(c). Its HTTPS seeder (`seed.rs`) implements dcrd's seeder client (status check, extra query parameters) and is **not** the wallet's `SeedPeers` |
| Ban score | `dcroxide-connmgr::DynamicBanScore` | explicit-clock API (`increase_at`, `int_at`) (`dcroxide:crates/dcroxide-connmgr/src/banscore.rs:41-98`) |
| Filters | `dcroxide-gcs` | `FilterV2::from_bytes(b, m, d)`, `matches_any`, `n`, `hash`; `blockcf2::key`; `blockcf2::Entries` (`dcroxide:crates/dcroxide-gcs/src/lib.rs:477-560`, `blockcf2.rs:56-90`) |
| Merkle roots, inclusion proofs | `dcroxide-standalone` | `calc_tx_tree_merkle_root`, `calc_combined_tx_tree_merkle_root`, `verify_inclusion_proof` (`dcroxide:crates/dcroxide-standalone/src/merkle.rs:77,96`, `inclusionproof.rs:119`) |
| Chain parameters, seeders | `dcroxide-chaincfg` | `seeders` field present (`dcroxide:crates/dcroxide-chaincfg/src/lib.rs:227`) |
| Mixpool source identity | `dcroxide-mixing` | the mixpool port; `RemotePeer::id()` is the source id |
| Peer state machine | `dcroxide-peer` | **not a fit**: it ports dcrd's `peer` package (inbound + outbound, verack-gated, self-connection nonces, dcrd's stall handler). Useful as a reference only |
| LRU | `dcroxide-containers::lru` | different semantics (exact limit, TTL support); not a drop-in for Q-01 |
| dcr-rs | — | no networking; nothing in this scope maps to it (modules are address, airgap, amount, blake256, hashing, hd, network, sighash, sign, tx) |

### 11.3 Hazards

- **Version skew**: dcrwallet pins `addrmgr` v3.0.0 and advertises pver 11 while dcroxide
  tracks dcrd master (pver 12, `addrv2`). Reusing dcroxide crates must not silently raise the
  wallet's protocol version or start writing Tor v3 keys into `peers.json` (dcrwallet's loader
  would delete such a file).
- **Unordered handlers** (Q-11): a naive sequential handler loop is compatible but changes
  timing; in particular a handler that blocks (announcement delivery waits for the syncer)
  must not stall `pong` replies or request replies. Keep dispatch non-blocking per message.
- **Locks across awaits**: `sidechainMu` and `remotesMu` are held across network I/O in Go.
  Use async-aware mutexes or restructure, preserving the serialization of chain switches.
- **Busy loops and leaks** (Q-10, Q-18, Q-38) should not be reproduced; replacing them with
  proper waits is not externally observable except as lower CPU use.
- **Go default HTTP behaviour** for seeders (HTTP/1.1, `User-Agent: Go-http-client/1.1`,
  gzip) is observable by seeder operators; pick a TLS/HTTP stack deliberately (§12).
- **Timer semantics**: implement each stall rule of Q-14 as specified; do not unify them.
- **Local DNS resolution** (Q-12) has privacy consequences behind Tor; decide explicitly.
- **Mix message hashing**: received mix messages must have their hash computed before lookup;
  the wire crate exposes `Message::mix_hash` (`dcroxide:crates/dcroxide-wire/src/message.rs:170`).

### 11.4 Size estimate

**L.** About 5 000 lines of Go in scope, dense with concurrency: roughly 30 request/handler
state machines in `p2p`, a three-stage pipeline with invalidation, announcement handling with
reorgs, and relay logic. Codecs, filters, proofs, ban score and (with review) the address
manager are reusable, which keeps it below XL. The absence of upstream tests means the test
harness (fake peer + differential simnet runs) is a substantial part of the work and should be
scheduled as such.

---

## 12. Open questions

1. **Q-02 (address manager lifetime)**: reproduce the "saved only until the first syncer
   restart" behaviour, or create/stop the manager per run? The latter changes when
   `peers.json` is written but not its format.
2. **Q-03 (gRPC `SpvSync` bypasses the proxy)**: reproduce or route through the configured
   dialer? Reproducing leaks the client address for Decrediton-style launches that pass
   `--proxy`; fixing is a behaviour change visible only on the network.
3. **Q-12 (local DNS for `--spvconnect` host names)**: reproduce or resolve through the proxy.
4. **Q-07 (mix `notfound` disconnects)**: reproduce exactly, or implement the documented
   `ErrNotFound` behaviour? Other mixing participants are unaffected either way; only which
   dcrd peers the wallet stays connected to changes.
5. **Q-20(c)**: v3.0.0's `GetAddress` can spin on stale counters after a corrupt
   `peers.json`. UNVERIFIED at runtime (established by reading
   `dcrd/addrmgr/v3@v3.0.0/addrmanager.go:723-741,766-824`); a Go test that loads a file with
   a tried-bucket entry missing from the address list and then calls `GetAddress` would settle
   it. The port should not hang regardless.
6. **Address manager source**: adapt `dcroxide-addrmgr` (master semantics) or port v3.0.0
   verbatim. Needs a function-by-function diff of `addOrUpdateAddress`, `expireNew`, `Good`,
   `GetAddress` and `GroupKey` between v3.0.0 and the version dcroxide mirrors; not done here.
7. **Seeder HTTP fingerprint**: whether to imitate Go's client headers and HTTP/1.1.
   UNVERIFIED what the deployed seeders return for the body framing beyond what the client
   code accepts (concatenated objects); a capture of one live response would confirm.
8. **User agent build metadata**: the Go build appends `+<9-hex VCS revision>` when built from
   a git checkout and nothing for a module-proxy build. Decide what the Rust build emits.
9. **Unknown-command tolerance (Q-05)**: when dcrd adds a new message for pver ≤ 11 peers the
   Go wallet disconnects; keep that, since tolerating it would diverge observably only in
   staying connected.
10. **dcrd's reaction to wallet idiosyncrasies** is UNVERIFIED here because dcrd's server code
    is outside the scope: whether dcrd penalizes the duplicate first locator, a second
    `getheaders` while one is outstanding, or `LastBlock = 0`. The current network accepts all
    three from Go dcrwallet, so reproducing them is safe.
11. **`fetched_headers_count` semantics (Q-24)** must stay as is for Decrediton; confirm with
    the gRPC spec owner that the field is documented as a height there.
12. **Peer selection randomness**: Go's map iteration gives non-uniform but effectively random
    selection. The port needs an explicit policy (uniform random among eligible peers is
    proposed).
