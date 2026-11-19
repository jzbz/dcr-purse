# 13 — Wallet-side mixing integration: account mixer, mixed accounts, change handling

## 1. Header

**Scope** (dcrwallet files; line counts are whole-file, only the mixing paths of the larger files are in scope):

| File | Lines | What is in scope |
| --- | --- | --- |
| `wallet/mixing.go` | 597 | Entire file |
| `wallet/wallet.go` | 5828 | Mixing fields of `Wallet`/`Config`, `PurchaseTicketsRequest` mixing fields, `PurchaseTickets` mixing guards, `Open` mixpool construction, `MixingEnabled`, `newMixClient`, `Run`, `GetCoinjoinTxsSumbByAcct`, the locked-outpoint set as mixing uses it |
| `wallet/createtx.go` | 2176 | `mixedSplit`, the mixing branches of `purchaseTickets`, `findEligibleOutputsAmount`, `p2pkhSizedScript` |
| `wallet/addresses.go` | 1038 | `nextAddress`, `accountBranchChildUpdates`, gap policies, `p2PKHChangeSource` as mixing uses them |
| `wallet/network.go` | 142 | `NetworkBackend.PublishMixMessages`, `WrapContext`, `OfflineNetworkBackend` |
| `wallet/chainntfns.go` | 933 | `expireMixMessages` call site, outpoint unlock in `processTransactionRecord` |
| `ticketbuyer/tb.go` | 340 | Entire file (mixing configuration, `mixChange`, mixed buying fan-out) |
| `spv/sync.go`, `spv/backend.go` | 1980, 652 | Mix inventory handling, `PublishMixMessages`, `wallet.Run` hosting, minimum-version peer target |
| `chain/sync.go`, `chain/backend.go` | 966, 106 | `notifymixmessages`, `mixmessage` notification, `PublishMixMessages`, `wallet.Run` hosting |
| `p2p/peering.go` | 2247 | Mix message request/receive/inventory paths |
| `rpc/client/dcrd/calls.go`, `notifications.go` | 543, 115 | `sendrawmixmessage`, `getmixmessage`, `getmixpairrequests`, `mixmessage` decoding |
| `internal/rpc/jsonrpc/methods.go`, `config.go` | 5863, 42 | `mixaccount`, `mixoutput`, `getcoinjoinsbyacct`, mixing branches of `purchaseticket`, `sendPairs`, `sendAmountToTreasury` (semantics only) |
| `internal/rpc/rpcserver/server.go` | 4312 | `AccountMixerService.RunAccountMixer`, mixing branches of `PurchaseTickets` and `RunTicketBuyer`, `GetCoinjoinOutputspByAcct` (semantics only) |
| `config.go`, `dcrwallet.go`, `testsolver.go`, `log.go`, `internal/loader/loader.go` | 1050, 636, 35, 96, 415 | Mixing options, their validation, start-up wiring, loggers |

**Parity pin:** dcrwallet `54b85b33` (2.2.0-pre, module `decred.org/dcrwallet/v5`). Dependencies read at the versions `go.mod` pins: `github.com/decred/dcrd/mixing v0.7.5` (`mixpool/mixpool.go`, `mixpool/errors.go`, `mixpool/limits.go`, `mixclient/client.go` entry points, `mixclient/coinjoin.go`, `mixclient/errors.go`, `expiry.go`, `limits.go`, `message.go`), `github.com/decred/dcrd/wire v1.8.0` (`protocol.go`, `invvect.go`, `msgmixpairreq.go`), `github.com/decred/dcrd/chaincfg/v3 v3.3.0`, `github.com/decred/dcrd/rpc/jsonrpc/types/v4 v4.4.0`, `github.com/decred/dcrd/crypto/rand v1.0.1`, `decred.org/cspp/v2 v2.4.0` (`solverrpc/rpc.go`).

**Summary.** The wallet owns no mixing cryptography. It supplies the mix client (spec 12) with five callbacks (best block, mixpool handle, message submission, input signing, transaction publication), builds a `CoinJoin` descriptor per mix request (inputs with UTXO ownership proofs, an optional change output, a mixed denomination, a mixed-output count, a fresh-address generator), and blocks until the client reports the outcome. Two request kinds exist: mixing one wallet output into at most four outputs of a power-of-four "split point" denomination (`MixOutput`, fanned out by `MixAccount`), and a mixed ticket split whose denomination is the per-ticket funding amount (`mixedSplit`). The wallet keeps one in-memory mixpool (dcrd `mixing/mixpool`), fed by the network backend (wire `inv`/`getdata` in SPV mode, `mixmessage` notifications in dcrd-RPC mode) and drained to the network through `NetworkBackend.PublishMixMessages`. Nothing mixing-specific is persisted; the only database effects are address records for generated addresses and the coinjoin transaction itself. Concurrency is bounded per denomination by ten non-blocking semaphores of capacity `--mixsplitlimit`. The account mixer (gRPC `AccountMixerService`, `--mixchange`, JSON-RPC `mixaccount`) is the ticket buyer loop with buying switched off: on every new main-chain tip it calls `MixAccount` on the change account.

---

## 2. Role in the system

### 2.1 What it does

1. Holds the wallet's mixpool and mix client and exposes them to the network syncers.
2. Turns "mix this output" and "fund these tickets through a mix" into `mixclient.CoinJoin` values and runs them through `mixclient.Client.Dicemix`.
3. Chooses the mixed denomination, the number of mixed outputs, the fee and whether a change output is included.
4. Generates fresh mixed-output addresses from a configured account and branch, ignoring the gap limit, each time the client asks.
5. Signs the wallet's own coinjoin inputs when the client asks, and records and publishes the finished coinjoin.
6. Reserves (locks) the outpoints a pending mix spends so no other transaction author selects them.
7. Classifies transactions as probable coinjoins (`PossibleCoinJoin`) for account discovery heuristics.

### 2.2 Callers

| Caller | Entry point |
| --- | --- |
| Ticket buyer loop (`ticketbuyer/tb.go:201-211`) | `Wallet.MixAccount` once per attached tip |
| Ticket buyer loop (`ticketbuyer/tb.go:305`) | `Wallet.PurchaseTickets` with `Mixing: true` → `mixedSplit` |
| JSON-RPC `mixaccount`, `mixoutput`, `purchaseticket`, `getcoinjoinsbyacct` | `Wallet.MixAccount`, `Wallet.MixOutput`, `Wallet.PurchaseTickets`, `Wallet.GetCoinjoinTxsSumbByAcct` |
| gRPC `AccountMixerService.RunAccountMixer`, `TicketBuyerService.RunTicketBuyer`, `WalletService.PurchaseTickets`, `WalletService.GetCoinjoinOutputspByAcct` | a `ticketbuyer.TB`, `Wallet.PurchaseTickets`, `Wallet.GetCoinjoinTxsSumbByAcct` |
| SPV syncer | `Wallet.Run`, `Wallet.MixingEnabled`, `Wallet.MixMessage`, `Wallet.AcceptMixMessageBySource` |
| dcrd-RPC syncer | `Wallet.Run`, `Wallet.MixingEnabled`, `Wallet.AcceptMixMessageBySource` |
| `Wallet.ChainSwitch` (`wallet/chainntfns.go:310`) | `Wallet.expireMixMessages` |
| `mixclient.Client` (spec 12) | the five `mixclient.Wallet` methods on `mixingWallet`, and the `GenFunc` closure |
| `mixpool.Pool` | `mixpoolBlockchain.CurrentTip`, `mixpoolBlockchain.ChainParams` |

### 2.3 Callees

| Callee | Use |
| --- | --- |
| `mixclient.NewClient`, `Client.SetLogger`, `Client.Run`, `Client.Dicemix`, `Client.ExpireMessages` | client lifecycle and one call per mix request |
| `mixclient.NewCoinJoin`, `CoinJoin.AddInput`, `CoinJoin.Tx`, `CoinJoin.MixedIndices` | request descriptor and result |
| `mixpool.NewPool`, `Pool.AcceptMessage`, `Pool.Message`, `Pool.RemoveMessage` | message pool |
| `mixing.MaxExpiry` | pair request expiry |
| `NetworkBackend.PublishMixMessages`, `PublishTransactions`, `WrapContext`, `LoadTxFilter` | network I/O |
| Address manager (`nextAddress`, `Manager.PrivateKey`, `Manager.AddrAccount`, `SyncAccountToAddrIndex`) | fresh addresses, keys, account lookup |
| Transaction store (`TxDetails`, `RandomUTXO`, `UnspentOutputs`, `UnspentOutputCount`, `MakeInputSource`, `RangeTransactions`) | UTXO lookup and selection |
| `txauthor.NewUnsignedTransaction` | input selection and change sizing for mixed ticket splits |
| `Wallet.PublishTransaction`, `Wallet.NextStakeDifficulty`, `Wallet.MainChainTip`, `Wallet.RelayFee` | publication, ticket price, tip, fee rate |
| `solverrpc.StartSolver`, `solverrpc.Roots` (`decred.org/cspp/v2`) | start-up probe of the root solver subprocess |

---

## 3. Public surface inventory

### 3.1 Package `wallet` — `wallet/mixing.go`

| Symbol | Signature | Contract |
| --- | --- | --- |
| `(*Wallet).MixMessage` | `(query *chainhash.Hash) (mixing.Message, error)` | Returns `w.mixpool.Message(query)`: a pair request or any accepted non-orphan message with that hash, else an error with text `message not found`. Dereferences a nil pool when mixing is disabled; callers must check `MixingEnabled` first. `wallet/mixing.go:34-36` |
| `mixpoolBlockchain` | `type mixpoolBlockchain Wallet` | Adapter implementing `mixpool.BlockChain`. Does not implement `mixpool.UtxoFetcher`, so the wallet's pool never validates pair request UTXOs. `wallet/mixing.go:38` |
| `(*mixpoolBlockchain).CurrentTip` | `() (chainhash.Hash, int64)` | `MainChainTip(context.Background())`, height widened to `int64`. `wallet/mixing.go:40-45` |
| `(*mixpoolBlockchain).ChainParams` | `() *chaincfg.Params` | The wallet's chain parameters. `wallet/mixing.go:47-49` |
| `(*Wallet).makeGen` | `(ctx, account, branch uint32) mixclient.GenFunc` | Returns a closure that derives `mcount` fresh addresses and returns their HASH160s. §5.4. `wallet/mixing.go:51-75` |
| `mixingWallet` | `type mixingWallet Wallet` | Adapter implementing `mixclient.Wallet`. `wallet/mixing.go:78` |
| `(*mixingWallet).BestBlock` | `() (uint32, chainhash.Hash)` | Tip height (as `uint32`) and hash. `wallet/mixing.go:81-85` |
| `(*mixingWallet).Mixpool` | `() *mixpool.Pool` | The wallet's pool. `wallet/mixing.go:92-95` |
| `(*mixingWallet).SubmitMixMessage` | `(ctx, msg mixing.Message) error` | Accepts into the local pool with `mixpool.ZeroSource`, then publishes through the current network backend; removes the message from the pool again on any error. §5.6. `wallet/mixing.go:99-118` |
| `(*mixingWallet).SignInput` | `(tx *wire.MsgTx, index int, prevScript []byte) error` | Sets `tx.TxIn[index].SignatureScript` to a P2PKH signature script. §5.7. `wallet/mixing.go:121-153` |
| `(*mixingWallet).PublishTransaction` | `(ctx, tx *wire.MsgTx) error` | `Wallet.PublishTransaction(ctx, tx, currentBackend)`; logs `Failed to publish mix transaction: %v` at error level on failure and returns the error. `wallet/mixing.go:157-169` |
| `(*Wallet).dicemixExpiry` | `(ctx) uint32` | `mixing.MaxExpiry(tipHeight, params) - 2`. §5.3. `wallet/mixing.go:171-174` |
| `(*Wallet).addCoinJoinInput` | `(ctx, cj *mixclient.CoinJoin, input *wire.TxIn, prevScript []byte, prevScriptVersion uint16) error` | Looks up the private key controlling `prevScript` and calls `cj.AddInput`. §5.5. `wallet/mixing.go:180-211` |
| `splitPoints` | `[10]dcrutil.Amount` | Standard mixed denominations, largest first. §6. `wallet/mixing.go:214-225` |
| `smallestMixChange` | `func(feeRate dcrutil.Amount) dcrutil.Amount` | Minimum change worth creating: fee of a one-input one-output P2PKH transaction plus the smallest split point. `wallet/mixing.go:227-234` |
| `mixSemaphores` | `struct{ splitSems [10]chan struct{} }` | One buffered channel per split point. `wallet/mixing.go:236-238` |
| `newMixSemaphores` | `func(n int) mixSemaphores` | Each channel is created with capacity `n`. `n == 0` yields unbuffered channels (every acquire fails); `n < 0` panics in `make`. `wallet/mixing.go:240-246` |
| `errNoSplitDenomination` | `errors.New("no suitable split denomination")` | Returned (wrapped) by `MixOutput` when no denomination fits. `wallet/mixing.go:249` |
| `errThrottledMixRequest` | `errors.New("throttled mix request for split denomination")` | Returned bare by `MixOutput` when the denomination's semaphore is full. `wallet/mixing.go:250` |
| `(*Wallet).MixOutput` | `(ctx, output *wire.OutPoint, changeAccount, mixAccount, mixBranch uint32) error` | Mixes one wallet output. Blocks until the mix completes or fails. §5.1. `wallet/mixing.go:256-441` |
| `(*Wallet).MixAccount` | `(ctx, changeAccount, mixAccount, mixBranch uint32) error` | Selects eligible outputs of `changeAccount` and runs `MixOutput` on each concurrently. Blocks until all return. §5.2. `wallet/mixing.go:449-512` |
| `PossibleCoinJoin` | `func(tx *wire.MsgTx) (isMix bool, mixDenom int64, mixCount uint32)` | Heuristic coinjoin classifier. §5.11. `wallet/mixing.go:517-557` |
| `(*Wallet).AcceptMixMessage` | `(msg mixing.Message) error` | Deprecated. `mixpool.AcceptMessage(msg, mixpool.ZeroSource)`, discarding the accepted list. No in-tree caller. `wallet/mixing.go:563-570` |
| `(*Wallet).AcceptMixMessageBySource` | `(msg mixing.Message, source mixpool.Source) error` | `mixpool.AcceptMessage(msg, source)`, discarding the accepted list. `wallet/mixing.go:574-581` |
| `(*Wallet).expireMixMessages` | `(height uint32)` | Forwards to `mixClient.ExpireMessages(height)` if a client has been stored; otherwise no-op. `wallet/mixing.go:583-589` |
| `(*Wallet).dicemix` | `(ctx, cj *mixclient.CoinJoin) error` | `mixClient.Dicemix(ctx, cj)`; if no client has been stored returns `errors.E(errors.Invalid, "mixing client is not running")`. `wallet/mixing.go:591-597` |

### 3.2 Package `wallet` — other files

| Symbol | Signature | Contract |
| --- | --- | --- |
| `Wallet.mixingEnabled` | `bool` | From `Config.MixingEnabled`; immutable after `Open`. `wallet/wallet.go:159`, `:5419` |
| `Wallet.mixpool` | `*mixpool.Pool` | Non-nil only when `mixingEnabled`. Created at the end of `Open`. `wallet/wallet.go:160`, `:5525-5527` |
| `Wallet.mixSems` | `mixSemaphores` | `newMixSemaphores(cfg.MixSplitLimit)`. `wallet/wallet.go:161`, `:5418` |
| `Wallet.mixClient` | `atomic.Pointer[mixclient.Client]` | Set by each `Run`; never cleared. `wallet/wallet.go:162`, `:5548` |
| `Config.MixSplitLimit` | `int` | Semaphore capacity per split point. `wallet/wallet.go:193` |
| `Config.MixingEnabled` | `bool` | Enables pool, client and mixing requests. `wallet/wallet.go:195` |
| `(*Wallet).MixingEnabled` | `() bool` | Returns the flag. `wallet/wallet.go:5534-5536` |
| `newMixClient` | `func(w *Wallet) *mixclient.Client` | `mixclient.NewClient((*mixingWallet)(w))` then `SetLogger(loggers.MixcLog)`. `wallet/wallet.go:5538-5542` |
| `(*Wallet).Run` | `(ctx) error` | When mixing is enabled: build a new client, store it, return `client.Run(ctx)`. Otherwise return nil immediately. `wallet/wallet.go:5545-5552` |
| `PurchaseTicketsRequest.Mixing`, `.MixedAccount`, `.MixedAccountBranch`, `.MixedSplitAccount`, `.ChangeAccount` | `bool`, `uint32` ×4 | Mixed split buying parameters. §5.8. `wallet/wallet.go:1558-1563` |
| `(*Wallet).PurchaseTickets` (mixing guards) | — | Rejects `Mixing` when the wallet has mixing disabled; refuses the VSP-fee UTXO split when `Mixing && MixedAccount == SourceAccount`. `wallet/wallet.go:1590-1594`, `:1618-1620` |
| `(*Wallet).mixedSplit` | `(ctx, req *PurchaseTicketsRequest, neededPerTicket dcrutil.Amount) (tx *wire.MsgTx, outIndexes []int, err error)` | Funds `req.Count` ticket-sized outputs through a mix. §5.8. `wallet/createtx.go:1000-1093` |
| `p2pkhSizedScript` | `make([]byte, 25)` | Placeholder script (25 zero bytes) used only for size estimation of the mixed outputs. `wallet/createtx.go:994` |
| `(*Wallet).findEligibleOutputsAmount` | `(dbtx, account uint32, minconf int32, amount dcrutil.Amount, currentHeight int32, minAmount dcrutil.Amount, maxResults int) ([]Input, error)` | UTXO selection with random sampling. §5.9. Caller must hold `lockedOutpointMu`. `wallet/createtx.go:1829-1987` |
| `(*Wallet).getCoinjoinTxsSumbByAcct`, `GetCoinjoinTxsSumbByAcct` | `(ctx) (map[uint32]int, error)` | Per-account count of wallet-owned outputs at the mixed denomination of mined probable coinjoins. §5.12. `wallet/wallet.go:5556-5609` |
| `outpoint` | `struct{ hash chainhash.Hash; index uint32 }` | Key of the locked-outpoint set; the tree is not part of the key. `wallet/wallet.go:94-97` |
| `Wallet.lockedOutpoints`, `lockedOutpointMu` | `map[outpoint]struct{}`, `sync.Mutex` | In-memory reservation set shared with every transaction author and with `lockunspent`. `wallet/wallet.go:137-138` |
| `NetworkBackend.PublishMixMessages` | `(ctx, msgs ...mixing.Message) error` | Announces or submits mixing messages. `wallet/network.go:39` |
| `NetworkBackend.WrapContext` | `(ctx) (context.Context, context.CancelFunc)` | Derived context cancelled when the backend stops. `wallet/network.go:56` |
| `OfflineNetworkBackend.PublishMixMessages` | — | Always returns `operation not supported in offline mode`. `wallet/network.go:110-112` |
| `WithGapPolicyIgnore`, `gapPolicyIgnore` | call option | Address generation continues past the gap limit, loading new addresses into the backend filter in batches. `wallet/addresses.go:432-434`, `:578-599` |
| `accountBranchChildUpdates`, `.UpdateDB` | `[]accountBranchChild` | Deferred address-record writes. §5.4. `wallet/addresses.go:466-502` |
| `p2PKHChangeSource` | `txauthor.ChangeSource` | Change script generator used by `mixedSplit` with `gapPolicyIgnore`. `wallet/addresses.go:944-966` |

### 3.3 Package `ticketbuyer`

| Symbol | Signature | Contract |
| --- | --- | --- |
| `Config` | struct | `BuyTickets bool`, `Account uint32`, `VotingAccount uint32`, `Maintain dcrutil.Amount`, `Limit int`, `Mixing bool`, `MixedAccount uint32`, `MixedAccountBranch uint32`, `TicketSplitAccount uint32`, `ChangeAccount uint32`, `MixChange bool`, `VSP *wallet.VSPClient`. `ticketbuyer/tb.go:18-43` |
| `TB` | struct | `wallet`, `cfg`, `mu`. `ticketbuyer/tb.go:48-53` |
| `New` | `func(w *wallet.Wallet, cfg Config) *TB` | Constructor. `ticketbuyer/tb.go:56-58` |
| `(*TB).Run` | `(ctx, passphrase []byte) error` | The loop of §5.10. Returns `ctx.Err()` on cancellation or an `errors.Passphrase` error. `ticketbuyer/tb.go:63-214` |
| `(*TB).buy` | `(ctx, passphrase, sdiff, expiry, cfg *Config) error` | One purchase attempt. `ticketbuyer/tb.go:216-312` |
| `(*TB).AccessConfig` | `(f func(cfg *Config))` | Runs `f` on the live config under the mutex. `ticketbuyer/tb.go:318-322` |
| `(*TB).mixChange` | `(ctx, cfg *Config) error` | Returns nil unless both `MixChange` and `Mixing`; otherwise `wallet.MixAccount(ctx, cfg.ChangeAccount, cfg.MixedAccount, cfg.MixedAccountBranch)` inside a `ticketbuyer.mixChange` trace task. `ticketbuyer/tb.go:324-340` |
| `log`, `UseLogger` | package logger, `func UseLogger(logger slog.Logger)` | Package-level logger, wired to the `TKBY` subsystem by the process. `ticketbuyer/log.go:14-17`, `log.go:51` |

`TB.buy` is specified step by step in §5.10.1. The package has no other declarations (mechanical inventory of `ticketbuyer/tb.go`: two types, five functions/methods, no constants or package variables besides the logger).

### 3.4 Network backends

| Symbol | Contract |
| --- | --- |
| `spv.minVersion = wire.MixVersion` (10), `spv.minVersionTarget = 3`, `spv.targetPeerCount = 8` | Peer admission rule of §5.14.5. `spv/sync.go:39-46` |
| `spv.Syncer.seenMixMsgs` | `lru.Cache[chainhash.Hash]` of capacity 2000: hashes of mix messages already fetched and processed. `spv/sync.go:88`, `:149` |
| `(*spv.Syncer).PublishMixMessages(ctx, msgs...)` | Sends one `inv` (type `InvTypeMix`) to every connected peer with negotiated protocol version ≥ 10. §5.14.1. `spv/backend.go:479-520` |
| `(*spv.Syncer).handleMixInvs(ctx, rp, hashes, onlyByID)` | Fetches unseen announced mix messages and feeds the pool. §5.14.3. `spv/sync.go:1182-1259` |
| `(*spv.Syncer).receiveGetData`, `receiveInv` | Serve and consume mix inventory. §5.14.2, §5.14.3. `spv/sync.go:779-888`, `:892-951` |
| `(*spv.Syncer).WrapContext`, `(*chain.Syncer).WrapContext` | Child context cancelled when the syncer's `done` channel closes; one helper goroutine per call. `spv/backend.go:640-652`, `chain/backend.go:94-106` |
| `(*chain.Syncer).PublishMixMessages(ctx, msgs...)` | Delegates to `dcrd.RPC.PublishMixMessages`. `chain/backend.go:43-45` |
| `(*chain.Syncer).mixMessage(ctx, params)` | Handles the `mixmessage` notification. §5.15.2. `chain/sync.go:933-966` |
| `(*dcrd.RPC).PublishMixMessages(ctx, msgs...)` | One `sendrawmixmessage` call per message, sequentially; all are attempted; the first error is returned wrapped with op `dcrd.PublishMixMessages`. `rpc/client/dcrd/calls.go:243-268` |
| `(*dcrd.RPC).MixMessage(ctx, hash)` | `getmixmessage`; decodes by the result's `type`. The returned message is not hashed. `rpc/client/dcrd/calls.go:271-305` |
| `(*dcrd.RPC).MixPairRequests(ctx)` | `getmixpairrequests`; decodes an array of hex strings. No in-tree caller. `rpc/client/dcrd/calls.go:309-318` |
| `dcrd.MixMessage(params)` | Decodes `mixmessage` notification parameters. `rpc/client/dcrd/notifications.go:80-115` |
| `(*p2p.RemotePeer).MixMessages(ctx, hashes)` | One `getdata` for all hashes; returns when every message arrived. §5.14.3. `p2p/peering.go:1645-1707` |
| `(*p2p.RemotePeer).PublishMixMessages(ctx, msgs...)` | Queues mix inventory vectors on the peer's trickle list. No in-tree caller (the SPV syncer sends its `inv` directly). `p2p/peering.go:2101-2121` |
| `(*p2p.RemotePeer).receivedMixMsg`, `addRequestedMixMsg`, `deleteRequestedMixMsg`, `writeMixMsgHash` | Request bookkeeping and hashing of received mix messages. `p2p/peering.go:1259-1301` |
| `p2p.invVecContainsTxOrMix` | True when an `inv` carries any `InvTypeTx` or `InvTypeMix` vector. `p2p/peering.go:2214-2224` |
| `(*p2p.RemotePeer).ID() uint64` | Makes `*RemotePeer` a `mixpool.Source`. IDs start at 1, so they never collide with `mixpool.ZeroSource`. `p2p/peering.go:320`, `:644-647` |
| `p2p.RemotePeer.requestedMixMsgs`, `requestedMixMsgsMu` | `map[chainhash.Hash]chan<- mixing.Message` and its mutex; created empty with the peer. One entry per mix hash currently requested from that peer. `p2p/peering.go:112-113`, `:562` |
| `p2p.blake256Hasher`, `p2p.blake256Mu` | Package-global BLAKE-256 hasher and mutex used by `writeMixMsgHash` for every peer. `p2p/peering.go:1254-1265` |
| `p2p` read loop dispatch | Each of `*wire.MsgMixPairReq`, `MsgMixKeyExchange`, `MsgMixCiphertexts`, `MsgMixSlotReserve`, `MsgMixDCNet`, `MsgMixConfirm`, `MsgMixFactoredPoly`, `MsgMixSecrets` is routed to `receivedMixMsg`. `p2p/peering.go:833-848` |
| `(*dcrd.RPC).publishMixMessage(ctx, op, msg)` | Unexported helper behind `PublishMixMessages`: hex-encodes `msg.BtcEncode(…, wire.MixVersion)` and calls `sendrawmixmessage` with `(msg.Command(), hex)`; an encode failure is `errors.E(op, errors.Encoding, err)`; the RPC error is returned unwrapped (the caller adds the op). `rpc/client/dcrd/calls.go:243-250` |
| `dcrd.messageArray[T]`, `makeMessageArray` | JSON decoder used by `MixPairRequests`: JSON `null` → nil slice; otherwise an array whose every element must be a JSON string (else kind `Encoding`, `not a string`), each hex-decoded with `BtcDecode` at the given protocol version (failure → kind `Encoding`). `rpc/client/dcrd/json.go:293-321` |
| Error ops of the dcrd client | `dcrd.PublishMixMessages`, `dcrd.MixMessage`, `dcrd.MixPairRequests` (`rpc/client/dcrd/calls.go:255`, `:272`, `:310`). `MixMessage` wraps the RPC error with its op, returns `errors.E(op, "unrecognized mixing message type")` for an unknown `type`, and returns the `BtcDecode` error unwrapped together with the partially decoded message. |

### 3.5 RPC servers (semantics; wire formats are owned by the JSON-RPC and gRPC specs)

| Method | Contract |
| --- | --- |
| JSON-RPC `mixaccount` | No parameters. §5.16.1. `internal/rpc/jsonrpc/methods.go:5649-5677` |
| JSON-RPC `mixoutput "outpoint"` | One string parameter `txhash:index`. §5.16.2. `internal/rpc/jsonrpc/methods.go:5613-5647` |
| JSON-RPC `getcoinjoinsbyacct` | No parameters; object mapping account name to count. §5.16.3. `internal/rpc/jsonrpc/methods.go:5834-5863` |
| JSON-RPC `purchaseticket` (mixing branch) | §5.16.4. `internal/rpc/jsonrpc/methods.go:3403-3474` |
| JSON-RPC `sendPairs`, `sendAmountToTreasury` (mixing branch) | Change redirection when spending from the mixed account. §5.16.5. `internal/rpc/jsonrpc/methods.go:3592-3605`, `:3628-3642` |
| `jsonrpc.Options.MixingEnabled`, `.MixAccount`, `.MixBranch`, `.MixChangeAccount`, `.TicketSplitAccount` | Server-wide mixing configuration, copied from the process config. `internal/rpc/jsonrpc/config.go:20-24`, `rpcserver.go:373-379` |
| gRPC `AccountMixerService.RunAccountMixer` | Server-streaming; §5.17.1. `internal/rpc/rpcserver/server.go:2531-2566` |
| gRPC `StartAccountMixerService(w)` | Marks the service ready and binds the wallet; panics with `service already started` on a second call. `internal/rpc/rpcserver/server.go:2518-2528` |
| gRPC `WalletService.PurchaseTickets`, `TicketBuyerService.RunTicketBuyer` (mixing branch) | §5.17.2, §5.17.3. `internal/rpc/rpcserver/server.go:1838-1894`, `:2615-2659` |
| gRPC `WalletService.GetCoinjoinOutputspByAcct` | §5.17.4. `internal/rpc/rpcserver/server.go:3848-3866` |
| `rpcserver.accountMixerServer` | struct `{ready bool; readyMtx sync.RWMutex; wallet *wallet.Wallet; pb.UnimplementedAccountMixerServiceServer}`; one package-global instance `accountMixerService`, registered with the gRPC server at construction (`pb.RegisterAccountMixerServiceServer`) and listed under the name `walletrpc.AccountMixerService` in the readiness table. `internal/rpc/rpcserver/server.go:193-203`, `:263`, `:279`, `:293` |
| `(*accountMixerServer).checkReady` | `() bool`; returns `ready` under the read lock; consulted through `ServiceReady("walletrpc.AccountMixerService")` before every call. `internal/rpc/rpcserver/server.go:2568-2572`, `:302-320` |
| `types.MixAccountCmd` | `struct{}`; registered as method `mixaccount`. `rpc/jsonrpc/types/methods.go:1103`, `:1238` |
| `types.MixOutputCmd` | `struct{ Outpoint string \`json:"outpoint"\` }`; one required positional string; registered as `mixoutput`. `rpc/jsonrpc/types/methods.go:1106-1108`, `:1239` |
| `types.GetCoinjoinsByAcctCmd` | `struct{}`; registered as `getcoinjoinsbyacct`. `rpc/jsonrpc/types/methods.go:1166`, `:1209` |
| JSON-RPC handler table entries | `"getcoinjoinsbyacct": {fn: (*Server).getcoinjoinsbyacct}`, `"mixaccount": {fn: (*Server).mixAccount}`, `"mixoutput": {fn: (*Server).mixOutput}`; none carries the `noHelp` flag, so all three appear in `help` output. `internal/rpc/jsonrpc/methods.go:113`, `:148-149` |
| `jsonrpc.parseOutpoint` | `func(s string) (*wire.OutPoint, error)`; §5.16.2. `internal/rpc/jsonrpc/methods.go:5679-5696` |
| Help text | Usage lines `getcoinjoinsbyacct`, `mixaccount`, `mixoutput "outpoint"`; full strings in §5.16.6. `internal/rpc/jsonrpc/rpcserverhelp.go:34`, `:69-70`, `:117`; `internal/rpchelp/helpdescs_en_US.go:677-683`, `:811-815`; `internal/rpchelp/methods.go:57`, `:92-93` |

### 3.6 Process configuration

| Symbol | Contract |
| --- | --- |
| `config.MixingEnabled` (`--mixing`), `CSPPSolver` (`--csppsolver`), `MixedAccount` (`--mixedaccount`), `TicketSplitAccount` (`--ticketsplitaccount`), `ChangeAccount` (`--changeaccount`), `MixChange` (`--mixchange`), `MixSplitLimit` (`--mixsplitlimit`) | §6.2. `config.go:170-179` |
| `config.mixedAccount`, `config.mixedBranch` | Parsed halves of `--mixedaccount`. `config.go:174-175`, `:759-778` |
| `testStartedSolverWorks` | Starts the solver subprocess and performs one `Roots` call. `testsolver.go:18-35` |
| Loggers `MIXC`, `MIXP` | Mix client and mixpool subsystems. `log.go:42-43`, `:58-59`; `internal/loggers/loggers.go:55-56`. The mixpool logger is installed process-wide at init with `mixpool.UseLogger(loggers.MixpLog)`. The mix client has no package-level logger hook in use: the line `// XXX mixclient.UseLogger(loggers.MixcLog)` is commented out (`log.go:42`) and the logger is attached per client instance by `newMixClient` (`wallet/wallet.go:5540`). Both subsystem tags are accepted by `--debuglevel` and the JSON-RPC `debuglevel` method, whose help text lists them (`internal/rpchelp/helpdescs_en_US.go:105`) |
| `defaultMixSplitLimit` | Constant `10`; default of `--mixsplitlimit`. `config.go:62`, `:377` |
| `loader.Loader.mixingEnabled`, `.mixSplitLimit`; `loader.NewLoader(…, mixingEnabled bool, manualTickets bool, mixSplitLimit int, dialer)` | The loader carries both values from the process config (`dcrwallet.go:170-172` for the running wallet, `walletsetup.go:117-119` for `--create`) into `wallet.Config`. `MixingEnabled` is copied by all three constructors; `MixSplitLimit` only by `CreateWatchingOnlyWallet` and `OpenExistingWallet` (§10, Q1). `internal/loader/loader.go:45`, `:50`, `:57-77`, `:177`, `:182`, `:267`, `:326`, `:331` |
| `rpcserver.StartAccountMixerService` call site | Registered through `loader.RunAfterLoad` together with the wallet, ticket buyer, network and voting services, only when a gRPC server was created. Order: `StartWalletService`, `StartTicketBuyerService`, `StartAccountMixerService`, `StartNetworkService`, `StartVotingService`. `dcrwallet.go:385-394` |

### 3.7 Dependency surface the wallet relies on (contract only; behaviour is specified in spec 12)

| Symbol | Contract as the wallet observes it |
| --- | --- |
| `mixclient.Wallet` interface | `BestBlock() (uint32, chainhash.Hash)`, `Mixpool() *mixpool.Pool`, `SubmitMixMessage(ctx, mixing.Message) error`, `SignInput(tx *wire.MsgTx, index int, prevScript []byte) error`, `PublishTransaction(ctx, *wire.MsgTx) error`. `dcrd/mixing@v0.7.5/mixclient/client.go:126-146` |
| `mixclient.GenFunc` | `func(count uint32) (wire.MixVect, error)`; `wire.MixVect` is `[][20]byte`. The client rejects a result whose length differs from the pair request's message count with `gen returned wrong message count`. `dcrd/mixing@v0.7.5/mixclient/coinjoin.go:62`, `mixclient/client.go:827-833` |
| `mixclient.NewCoinJoin(gen, change *wire.TxOut, mixValue int64, prExpiry uint32, mcount uint32)` | Creates the descriptor; `change` may be nil. `dcrd/mixing@v0.7.5/mixclient/coinjoin.go:66-78` |
| `(*CoinJoin).AddInput(input *wire.TxIn, prevScript []byte, prevScriptVersion uint16, privKey)` | Appends a `wire.MixPairReqUTXO{OutPoint, Script: nil, PubKey: 33-byte compressed key, Signature: UTXO proof over prExpiry, Opcode}` and adds `input.ValueIn` to the input value. `Opcode` is `prevScript[0]` when that byte is `OP_SSGEN`, `OP_SSRTX` or `OP_TGEN`, else 0. `prevScriptVersion` is ignored. Indexes `prevScript[0]` without a length check. `dcrd/mixing@v0.7.5/mixclient/coinjoin.go:86-117` |
| `(*CoinJoin).Tx()`, `MixedIndices()` | The coinjoin transaction as this local peer assembled it, and the output indexes of this peer's mixed outputs within it. `dcrd/mixing@v0.7.5/mixclient/coinjoin.go:288-295` |
| `mixclient.NewClient(w)` | Calls `solverrpc.StartSolver()` (sets the can-solve-roots pair request flag on success), reads `w.BestBlock()` once and `w.Mixpool().Epoch()`. `dcrd/mixing@v0.7.5/mixclient/client.go:414-454` |
| `(*Client).Run(ctx)` | Blocks until `ctx` is cancelled and every running session finished; normally returns `mixclient.ErrStopping` (`mixing client is stopping`). `dcrd/mixing@v0.7.5/mixclient/client.go:486-530`, `:697-715` |
| `(*Client).Dicemix(ctx, cj)` | Blocks until the client is warmed up, then until the mix succeeds (nil), fails, or `ctx` is cancelled. `dcrd/mixing@v0.7.5/mixclient/client.go:1006-1091` |
| `(*Client).ExpireMessages(height)` | Records the height; the actual pool expiry runs at the next epoch tick. `dcrd/mixing@v0.7.5/mixclient/client.go:1097-1106` |
| `mixpool.NewPool(blockchain)` | Epoch 10 minutes, or 3 minutes when `ChainParams().Net == wire.TestNet3`; fee rate fixed at 10000 atoms/kB. `dcrd/mixing@v0.7.5/mixpool/mixpool.go:269-305`, `:31` |
| `(*Pool).AcceptMessage(msg, src)` | Requires the message hash to be set (`WriteHash` called); returns `nil, nil` for duplicates. Returns `*mixpool.MissingOwnPRError{MissingPR}` for a key exchange whose own pair request is unknown (the key exchange is kept as an orphan). `dcrd/mixing@v0.7.5/mixpool/mixpool.go:1155-1342`, `:1697-1802` |
| `(*Pool).Message(hash)` | Pair requests and accepted entries only (not orphans, not the recently-removed cache). Error text `message not found`. `dcrd/mixing@v0.7.5/mixpool/mixpool.go:313-325`, `mixpool/errors.go:134-136` |
| `(*Pool).RemoveMessage(msg)` | Removes a message by hash. `dcrd/mixing@v0.7.5/mixpool/mixpool.go:609` |
| `mixpool.Source`, `mixpool.Uint64Source`, `mixpool.ZeroSource` | `Source` is `interface{ ID() uint64 }`; `ZeroSource` is the constant `Uint64Source(0)`. Used only to attribute orphans for eviction. `dcrd/mixing@v0.7.5/mixpool/mixpool.go:111-128`, `:1067-1109` |
| `mixing.MaxExpiry(tipHeight, params)` | `tipHeight + uint32(60 min / params.TargetTimePerBlock) + 1`. `dcrd/mixing@v0.7.5/expiry.go:15-18` |
| `solverrpc.SolverProcess`, `StartSolver`, `Roots` | Process name/path (default `csppsolver`), one-shot process start, one RPC round trip. `cspp/v2@v2.4.0/solverrpc/rpc.go:15`, `:94-97`, `:138-168` |

---

## 4. Data formats

### 4.1 Persisted state

This subsystem defines no bucket, key or value of its own. No mixing session, pair request, mixpool message, semaphore or locked outpoint is written to `wallet.db`. A wallet that restarts mid-mix forgets the mix entirely.

Database effects that mixing causes through other subsystems:

| Effect | Written by | When |
| --- | --- | --- |
| Address records for every generated mixed-output address and mix change address, and for every lower child index of the same branch not yet recorded | `Manager.SyncAccountToAddrIndex(ns, account, child, branch)` via `accountBranchChildUpdates.UpdateDB` (`wallet/addresses.go:468-502`, `wallet/udb/addressmanager.go:1985-2056`) | In its own read-write transaction, immediately after generation and before the mix runs (`wallet/mixing.go:71`, `:412`); for mixed ticket splits, after the mix succeeds (`wallet/createtx.go:1009-1015`) |
| The coinjoin transaction as an unmined transaction with its credits and debits | `Wallet.PublishTransaction` → `processTransactionRecord` (`wallet/wallet.go:5108-5147`) | When the mix client publishes the finished coinjoin, only if `isRelevantTx` is true |

`SyncAccountToAddrIndex` does not modify the account's "last returned" or "last used" child index properties (`wallet/udb/addressmanager.go:2058-2060`). The address manager spec owns the byte layout of the address records.

### 4.2 In-memory state

| State | Type | Lifetime |
| --- | --- | --- |
| Mixpool | `*mixpool.Pool` | `Open` to process exit; never recreated, so it survives syncer restarts |
| Mix client | `*mixclient.Client` | Replaced by every `Wallet.Run`; the previous, stopped client stays referenced until then |
| Split semaphores | 10 × `chan struct{}` of capacity `MixSplitLimit` | `Open` to process exit |
| Locked outpoints | `map[outpoint]struct{}` | `Open` to process exit; entries added and removed per §5.13 |
| Address cursors | `addressBuffer.cursor` per account branch | `Open` to process exit; advanced by every generated address and not rolled back on failure (`wallet/addresses.go:449-452`) |
| SPV seen-mix cache | LRU of 2000 hashes | Per `spv.Syncer` |
| Per-peer requested mix messages | `map[chainhash.Hash]chan<- mixing.Message` | Per `p2p.RemotePeer` |
| Per-peer sent inventory | LRU of 5000 hashes (`p2p/peering.go:64`, `:563`) | Per `p2p.RemotePeer` |

### 4.3 Structures handed to the mix client

`mixclient.NewCoinJoin(gen, change, mixValue, expires, mcount)` arguments as the wallet fills them:

| Field | `MixOutput` | `mixedSplit` |
| --- | --- | --- |
| `gen` | `makeGen(ctx, mixAccount, mixBranch)` | `makeGen(ctx, req.MixedSplitAccount, req.MixedAccountBranch)` |
| `change` | nil, or `wire.TxOut{Value: changeValue, Version: 0, PkScript: 25-byte P2PKH script of a fresh internal-branch address of changeAccount}` | nil, or the change output `txauthor` produced (fresh internal-branch address of `req.ChangeAccount`) |
| `mixValue` | the chosen split point, atoms | `neededPerTicket`, atoms |
| `prExpiry` | `dicemixExpiry(ctx)` | `dicemixExpiry(ctx)` |
| `mcount` | 1 to 4 | `uint32(req.Count)` |

`CoinJoin.AddInput` arguments:

| Field | `MixOutput` | `mixedSplit` |
| --- | --- | --- |
| `input` | `wire.TxIn{PreviousOutPoint: *output, ValueIn: amount}`; all other fields zero, so `Sequence` is 0 and `PreviousOutPoint.Tree` is whatever the caller's `*wire.OutPoint` carries: always 0 from JSON-RPC `mixoutput`; the stored credit's tree (0 regular, 1 stake) from `MixAccount` | each `atx.Tx.TxIn[i]` produced by the input source |
| `prevScript` | `txDetails.MsgTx.TxOut[index].PkScript` | `atx.PrevScripts[i]` |
| `prevScriptVersion` | `txDetails.MsgTx.TxOut[index].Version` | constant 0 (`wallet/createtx.go:1078`) |
| `privKey` | key for the single P2PKH address extracted from `prevScript` at script version 0 | same |

Only `input.PreviousOutPoint` and `input.ValueIn` are consumed by the client (`dcrd/mixing@v0.7.5/mixclient/coinjoin.go:105-114`). The resulting pair request carries `InputValue` = sum of `ValueIn`, `MixAmount` = `mixValue`, `MessageCount` = `mcount`, `Expiry` = `prExpiry`, script class `P2PKH-secp256k1-v0`, and the change output verbatim (`dcrd/mixing@v0.7.5/mixclient/client.go:1035-1038`). The pair request's byte layout is in spec 12 and the wire spec.

### 4.4 Generated mixed outputs

`GenFunc` returns `mcount` 20-byte values, each the HASH160 of the compressed public key of a freshly derived BIP0044 child of `account'/branch` (`wallet/mixing.go:65-69`). The client turns each into a version-0 script `76 a9 14 <20 bytes> 88 ac` (`dcrd/mixing@v0.7.5/mixclient/coinjoin.go:155-165`).

### 4.5 dcrd JSON-RPC exchanges (dcrd-RPC mode)

| Direction | Method | Parameters | Result |
| --- | --- | --- | --- |
| wallet → dcrd | `notifymixmessages` | none | none; sent once per sync session when mixing is enabled (`chain/sync.go:737-742`) |
| dcrd → wallet | notification `mixmessage` | `[command string, hex string]` | — |
| wallet → dcrd | `sendrawmixmessage` | `[command string, hex string]` | none |
| wallet → dcrd | `getmixmessage` | `[hash string]` (`chainhash.Hash.String()`, byte-reversed hex) | object `{"type": command, "message": hex}` (`dcrd/rpc/jsonrpc/types/v4@v4.4.0/chainsvrresults.go:238-241`) |
| wallet → dcrd | `getmixpairrequests` | none | array of hex strings (client method exists, unused) |

`command` is the wire command string: `mixpairreq`, `mixkeyxchg`, `mixcphrtxt`, `mixslotres`, `mixfactpoly`, `mixdcnet`, `mixconfirm`, `mixsecrets` (`dcrd/wire@v1.8.0/message.go:56-63`). `hex` is the lowercase hex of the message payload as produced by `BtcEncode(w, wire.MixVersion)` — the message body without the 24-byte wire header — encoded and decoded at protocol version 10 regardless of any peer version (`rpc/client/dcrd/calls.go:243-250`, `notifications.go:113`). An unknown command yields the error `unrecognized mixing message command string` (notification) or `unrecognized mixing message type` (`getmixmessage`).

The dcrd JSON-RPC API version required by the syncer is `8.3.0` under semver compatibility (`chain/sync.go:33`, `:604-607`).

### 4.6 Wire protocol exchanges (SPV mode)

| Message | Use |
| --- | --- |
| `inv` with `InvVect{Type: InvTypeMix (4), Hash}` | Announce a mix message, both directions |
| `getdata` with `InvTypeMix` vectors | Request announced mix messages, both directions |
| `mixpairreq`, `mixkeyxchg`, `mixcphrtxt`, `mixslotres`, `mixfactpoly`, `mixdcnet`, `mixconfirm`, `mixsecrets` | Reply to `getdata`, both directions |
| `notfound` | Wallet → peer for mix hashes it did not announce or no longer holds. Peer → wallet for mix hashes is treated as a protocol violation (§10, Q5) |

The local peer advertises protocol version `wire.BatchedCFiltersV2Version` = 11 (`p2p/peering.go:48`) and requires peers to be at least `wire.RemoveRejectVersion` = 9 (`p2p/peering.go:45`). Mix messages are exchanged only on connections whose negotiated version is ≥ `wire.MixVersion` = 10.

### 4.7 RPC fields that carry mixing configuration

Field names, types and numbers as declared in `rpc/api.proto` (the gRPC spec owns the full messages):

| Message | Field | Type | Number |
| --- | --- | --- | --- |
| `PurchaseTicketsRequest` | `enable_mixing` | bool | 9 |
| | `mixed_account` | uint32 | 10 |
| | `mixed_account_branch` | uint32 | 11 |
| | `mixed_split_account` | uint32 | 12 |
| | `change_account` | uint32 | 13 |
| `RunTicketBuyerRequest` | `limit` | int32 | 7 |
| | `enable_mixing` | bool | 8 |
| | `mixed_account` | uint32 | 9 |
| | `mixed_account_branch` | uint32 | 10 |
| | `mixed_split_account` | uint32 | 11 |
| | `change_account` | uint32 | 12 |
| `RunAccountMixerRequest` | `passphrase` | bytes | 1 |
| | `mixed_account` | uint32 | 2 |
| | `mixed_account_branch` | uint32 | 3 |
| | `change_account` | uint32 | 4 |
| `RunAccountMixerResponse` | (no fields) | | |
| `GetCoinjoinOutputspByAcctRequest` | (no fields) | | |
| `coinjoinTxsSumByAcct` | `account_number` | uint32 | 1 |
| | `coinjoin_txs_sum` | int32 | 2 |
| `GetCoinjoinOutputspByAcctResponse` | `data` | repeated `coinjoinTxsSumByAcct` | 1 |

Sources: `rpc/api.proto:78`, `:104-106`, `:627-642`, `:879-903`, `:1201-1210`. Proto3 defaults apply: an unset `change_account` or `mixed_account` is account 0, an unset `limit` is 0.

JSON-RPC: `mixaccount` takes no parameters; `mixoutput` takes one positional string (`outpoint`); `getcoinjoinsbyacct` takes none and returns a JSON object whose keys are account names and whose values are JSON integers (Go `map[string]int`, marshalled by `encoding/json` with keys in sorted order). `mixaccount` and `mixoutput` return JSON `null`.

---

## 5. Algorithms and state machines

Notation: `est(ins, outs, change)` is `txsizes.EstimateSerializeSizeFromScriptSizes` (`wallet/txsizes/size.go:161-186`, with `EstimateInputSize` and `EstimateOutputSize` at `:198-209`): `12 + 2·varintSize(len(ins)) + varintSize(len(outs) + (change>0 ? 1 : 0)) + Σ(57 + varintSize(s) + s for s in ins) + Σ(10 + varintSize(s) + s for s in outs) + (change>0 ? 10 + varintSize(change) + change : 0)`. For one 108-byte signature script input and `n` 25-byte outputs this is `181 + 36·n` without change and `217 + 36·n` with a 25-byte change output (valid while the output count is below 253). `fee(rate, size)` is `txrules.FeeForSerializeSize` (`wallet/txrules/rules.go:94-106`): `rate·size/1000` with truncating integer division; if the result is 0 and `rate > 0` the result is `rate`; if the result is negative or above 21,000,000 DCR it is 21,000,000 DCR.

### 5.1 `MixOutput(ctx, output, changeAccount, mixAccount, mixBranch)`

Source: `wallet/mixing.go:256-441`. `op` is the string `wallet.MixOutput(<txhash>:<index>)` (`wire.OutPoint.String` form).

1. If mixing is disabled, return `errors.E(op, errors.Invalid, "wallet mixing support is disabled")`.
2. `nb, err := w.NetworkBackend()`. On error return it unwrapped (`wallet.NetworkBackend: Decred network is unreachable`, kind `NoPeers`).
3. `ctx, cancel := nb.WrapContext(ctx)`; `cancel` is deferred. Every later step uses the wrapped context.
4. `sdiff, err := w.NextStakeDifficulty(ctx)` (one read transaction). On error return `errors.E(op, err)`. This fails with kind `Deployment` when DCP0001 is not active at the tip.
5. Lock `lockedOutpointMu`.
6. If `{output.Hash, output.Index}` is in `lockedOutpoints`: unlock, return `errors.E(op, errors.Errorf("output %v already locked", output))`.
7. In one read transaction: `txDetails, err := w.txStore.TxDetails(txmgrNs, &output.Hash)`. On error return it (kind `NotExist` when the wallet does not know the transaction). If `output.Index >= len(txDetails.MsgTx.TxOut)` return `errors.E(op, errors.Invalid, errors.Errorf("index out of range: index:%d txouts:%d", index, len))`. Otherwise capture `prevScript`, `prevScriptVersion` and `amount` from that output.
8. If step 7 failed: unlock the mutex, return `errors.E(op, err)`.
9. Insert `{output.Hash, output.Index}` into `lockedOutpoints`; unlock the mutex. Defer removal of the same key (under the mutex) for every exit path from here on.
10. `feeRate := w.RelayFee()`; `smallest := smallestMixChange(feeRate)` = `fee(feeRate, 217) + 262144`.
11. Denomination selection. For `i` from 0 to 9, with `mixValue = splitPoints[i]` and `last = (i == 9)`:
    1. If `!last && mixValue >= sdiff`, skip to the next `i`.
    2. `count = min(uint32(amount / mixValue), 4)`.
    3. While `count > 0`:
       1. `remValue = amount - count·mixValue`. (A negative `remValue` decrements `count` and retries; unreachable given step 2.)
       2. `feeC = fee(feeRate, est([108], [25]×count, 25))`; `changeValue = remValue - feeC`.
       3. If `last`, set `changeValue = 0`.
       4. If `changeValue <= 0`: `feeN = fee(feeRate, est([108], [25]×count, 0))`; if `remValue < feeN`, decrement `count` and retry step 3; otherwise `changeValue = 0`.
       5. If `changeValue < smallest`, set `changeValue = 0`.
       6. Selection is complete: stop both loops with this `i`, `mixValue`, `count`, `changeValue`.
12. If `count == 0` after the loops: return `errors.E(op, errors.Errorf("output %v (%v): %w", output, amount, errNoSplitDenomination))`. `amount` formats as `dcrutil.Amount` (`<n> DCR`).
13. Semaphore acquire, non-blocking, on `mixSems.splitSems[i]`:
    - if the context is done: return `errors.E(op, ctx.Err())`;
    - if a slot is free: take it, defer its release;
    - otherwise return the bare `errThrottledMixRequest` (no `op`).
    When both the context is done and a slot is free, Go picks one of the two at random.
14. If `changeValue > 0`: derive the next internal-branch (`udb.InternalBranch`, 1) address of `changeAccount` with `nextAddress(ctx, op, &updates, nil, "", changeAccount, 1, WithGapPolicyIgnore())`; on error return `errors.E(op, err)`. Build `change = &wire.TxOut{Value: changeValue, PkScript: script, Version: version}` from `addr.PaymentScript()`. Then `updates.UpdateDB(ctx, w, nil)` in its own read-write transaction; on error return it unwrapped.
15. `gen := w.makeGen(ctx, mixAccount, mixBranch)`; `expires := w.dicemixExpiry(ctx)`; `cj := mixclient.NewCoinJoin(gen, change, int64(mixValue), expires, count)`.
16. `input := &wire.TxIn{PreviousOutPoint: *output, ValueIn: int64(amount)}`; `w.addCoinJoinInput(ctx, cj, input, prevScript, prevScriptVersion)`; on error return `errors.E(op, err)`.
17. Log at info level: `Mixing output <txhash>:<index> (<amount>)`.
18. `w.dicemix(ctx, cj)`; on error return `errors.E(op, err)`.
19. Log at info level: `Completed CoinShuffle++ mix of output <txhash>:<index> in transaction <hash of cj.Tx()>`. Return nil.

Properties of step 11:

- The remainder not returned as change is the fee. At the smallest denomination the whole remainder is fee, however large.
- A positive change below `smallest` is dropped and becomes fee; this adds at most `smallest - 1` atoms to the fee.
- The implied fee is exactly `feeC` when change is kept, matching the fee the mixpool requires at its fixed 10000 atoms/kB rate when `feeRate` is 10000 (`dcrd/mixing@v0.7.5/mixpool/mixpool.go:1857-1874`, `:1897-1911`).
- No check is made that the output is unspent, confirmed, owned by `changeAccount`, or owned by the wallet at all; those failures surface later (key lookup in step 16, or rejection of the pair request by the network).

Worked values at `feeRate = 10000` atoms/kB:

| `count` | size with change | fee with change | size without | fee without |
| --- | --- | --- | --- | --- |
| 1 | 253 | 2530 | 217 | 2170 |
| 2 | 289 | 2890 | 253 | 2530 |
| 3 | 325 | 3250 | 289 | 2890 |
| 4 | 361 | 3610 | 325 | 3250 |

`smallestMixChange(10000)` = 2170 + 262144 = 264314 atoms.

### 5.2 `MixAccount(ctx, changeAccount, mixAccount, mixBranch)`

Source: `wallet/mixing.go:449-512`. `op` is `wallet.MixAccount`.

1. If mixing is disabled, return `errors.E(op, errors.Invalid, "wallet mixing support is disabled")`.
2. `_, tipHeight := w.MainChainTip(ctx)` (one read transaction).
3. Lock `lockedOutpointMu`. In one read transaction call `findEligibleOutputsAmount(dbtx, changeAccount, minconf = 2, targetAmount = 0, tipHeight, minAmount = 262144, maxResults = cap(mixSems.splitSems[0]) × 10)`. Unlock. On error return `errors.E(op, err)`. The selected outputs are not locked by this step.
4. If no outputs were selected: log at debug level `Account <changeAccount> has no outputs eligible for mixing`, return nil.
5. For each selected credit start one goroutine (an `errgroup.Group` without a derived context) running `MixOutput(ctx, &credit.OutPoint, changeAccount, mixAccount, mixBranch)`:
   - on nil, increment an atomic success counter;
   - if the error `Is` `errNoSplitDenomination`: log debug `Unable to mix output for account <changeAccount>: <err>`, treat as nil;
   - if the error `Is` `errThrottledMixRequest`: log debug `Temporarily skipped output <outpoint> during account <changeAccount> mix: <err>`, treat as nil;
   - any other error is returned to the group.
6. Wait for every goroutine. Log debug `Mixed <success> of <selected> selected outputs of account <changeAccount>`.
7. If the group reported an error (the first one returned by any goroutine), return `errors.E(op, err)`; else nil.

A failing output does not cancel the others; the call returns only after the slowest `MixOutput` returns, which can be many minutes.

With `MixSplitLimit = 10` (default), `maxResults` is 100. With `MixSplitLimit = 0`, `maxResults` is 0, which means "no limit" to `findEligibleOutputsAmount`, and every `MixOutput` is then throttled at step 13.

### 5.3 `dicemixExpiry`

`mixing.MaxExpiry(uint32(tipHeight), params) - 2` = `tipHeight + uint32(60 min / TargetTimePerBlock) + 1 - 2` (`wallet/mixing.go:171-174`, `dcrd/mixing@v0.7.5/expiry.go:15-18`).

| Network | `TargetTimePerBlock` | Pair request expiry |
| --- | --- | --- |
| mainnet | 5 min | tip + 11 |
| testnet3 | 2 min | tip + 29 |
| simnet, regnet | 1 s | tip + 3599 |

The tip is read in its own read transaction at the time the `CoinJoin` is built, after the semaphore slot is taken and before `Dicemix` waits for warm-up and for its pre-publication delay, so the expiry can be several blocks stale by the time the pair request is published. The mixpool rejects a pair request whose expiry is ≤ the tip height or greater than `MaxExpiry(tip)` at acceptance time (`dcrd/mixing@v0.7.5/mixpool/mixpool.go:1424-1433`).

### 5.4 `makeGen` and address cursor handling

Source: `wallet/mixing.go:51-75`, `wallet/addresses.go:505-649`.

The closure returned by `makeGen(ctx, account, branch)` captures the request context. Each invocation `gen(mcount)`:

1. Allocates the result (`mcount` entries) and an `accountBranchChildUpdates` slice.
2. For each of the `mcount` entries calls `w.nextAddress(ctx, "gen", &updates, nil, "", account, branch, WithGapPolicyIgnore())`. On error returns `(nil, err)`; address cursors already advanced in this invocation stay advanced and their records are not written.
3. Requires the address to implement `stdaddr.Hash160er`; otherwise returns `errors.E("address does not have Hash160 method")` (unreachable for the P2PKH addresses `nextAddress` produces).
4. Stores the 20-byte HASH160.
5. Calls `updates.UpdateDB(ctx, w, nil)`: one new read-write transaction calling `Manager.SyncAccountToAddrIndex(ns, account, child, branch)` for each generated child in generation order; commit on success, rollback on error. Returns `(gen, err)` — on a database error the generated vector is returned together with the error.

`nextAddress` in this mode (updates slice supplied, no transaction supplied):

1. Locks `addressBuffersMu` for the whole call.
2. Looks up the account's buffers; unknown account → `errors.E(op, errors.NotExist, errors.Errorf("account %d", account))`. The imported-keys account has no buffers and fails here.
3. Selects the branch: 0 external, 1 internal; anything else → `errors.E(op, errors.Invalid, "branch must be external (0) or internal (1)")`.
4. Loop:
   1. If the context is done, return `ctx.Err()`.
   2. If `cursor >= gapLimit` and `cursor % gapLimit == 0` and a network backend is set: derive `gapLimit` addresses starting at child `lastUsed + 1 + cursor` and call `LoadTxFilter(ctx, false, addrs, nil)`; a failure of either returns the error. If no backend is set, or the cursor is not a multiple of the gap limit, nothing is loaded.
   3. `childIndex = lastUsed + 1 + cursor`. If `childIndex >= 2^31` return `errors.E(op, errors.Errorf("account %d branch %d exhausted", account, branch))`.
   4. Derive the child public key. On `hdkeychain.ErrInvalidChild` increment the cursor and repeat the loop.
   5. Append `{account, branch, childIndex}` to the updates slice, increment the cursor, log at info level `Returning address (account=<a> branch=<b> child=<c>)`, return the address.

`lastUsed` is `^uint32(0)` for a branch with no used address, so the first child is 0 by wraparound.

When the client calls `gen`:

- once inside `Dicemix`, before the pair request is submitted (`dcrd/mixing@v0.7.5/mixclient/client.go:1052`);
- again for every local peer at the start of a rerun that requires fresh messages, that is after secrets were revealed for blame assignment (`dcrd/mixing@v0.7.5/mixclient/client.go:1518-1525`, `:2577`).

Consequences an implementation must reproduce:

- A mix request consumes `mcount` addresses of the mixed account/branch up front, plus `mcount` more per blame rerun, whether or not the mix ever succeeds.
- The generated addresses are recorded in the database before the mix runs. Their records are the only durable trace of a failed mix.
- The gap limit is ignored: a long run of failed mixes can push the cursor arbitrarily far past the last used child. Each time the cursor crosses a multiple of the gap limit, the next `gapLimit` addresses are added to the backend's transaction filter.
- The cursor is in memory only. After a restart the cursor is rebuilt as `LastReturnedIndex - LastUsedIndex` from the account properties (`wallet/wallet.go:5471-5480`), and generating mix addresses does not advance `LastReturnedIndex` (§4.1). Addresses generated for mixes that never appeared on chain are generated again after a restart.
- The first `gen` call runs on the goroutine of the `MixOutput`/`mixedSplit` caller, inside `Dicemix`, after the warm-up wait and the pre-publication delay (`dcrd/mixing@v0.7.5/mixclient/client.go:1007-1052`); a request cancelled during either wait consumes no mixed-output addresses. Rerun calls run on a mix client goroutine. Every call uses the context of the `MixOutput`/`mixedSplit` call that created the generator. If that call has returned (context cancelled) while the client is still running a session for the peer, `nextAddress` returns the context error and the client fails that peer.

### 5.5 `addCoinJoinInput`

Source: `wallet/mixing.go:180-211`.

1. `stdscript.ExtractAddrs(0, prevScript, params)` — script version 0 is assumed regardless of `prevScriptVersion`.
2. Require exactly one address of type `*stdaddr.AddressPubKeyHashEcdsaSecp256k1V0`; otherwise `errors.E(errors.Invalid, "previous output is not P2PKH")`. Stake-tagged P2PKH scripts (`OP_SSTX`, `OP_SSGEN`, `OP_SSRTX`, `OP_SSTXCHANGE`, `OP_TGEN` prefixes) satisfy this (`dcrd/txscript/v4@v4.1.2/stdscript/addressv0.go:105-164`).
3. In one read transaction, `w.manager.PrivateKey(addrmgrNs, addr)`. On error (wallet or account locked: kind `Locked`; watching-only; unknown address) release the key if one was returned and return the error.
4. `cj.AddInput(input, prevScript, prevScriptVersion, privKey)`; release the key (`privKeyDone()`); return the `AddInput` error.

### 5.6 `SubmitMixMessage`

Source: `wallet/mixing.go:99-118`.

1. `w.mixpool.AcceptMessage(msg, mixpool.ZeroSource)`. On error return it (the deferred cleanup still calls `RemoveMessage`).
2. Read the current network backend under `networkBackendMu`. If nil, return the bare kind `errors.NoPeers`.
3. Return `n.PublishMixMessages(ctx, msg)`.
4. On any non-nil return, `w.mixpool.RemoveMessage(msg)` runs.

The message must already be signed and hashed by the client. `ctx` carries the client's stage deadline (`dcrd/mixing@v0.7.5/mixclient/client.go:786-790`). The wallet's own messages therefore pass through the same acceptance rules as network messages, including for pair requests: change not dust and version 0 P2PKH/P2SH, mix amount not dust, input value covering outputs, expiry window, fee between the required minimum and 10000 times it at 10000 atoms/kB (`dcrd/mixing@v0.7.5/mixpool/mixpool.go:1388-1445`). UTXO existence and ownership proofs are not checked locally because the wallet supplies no `UtxoFetcher` (`:1463`).

### 5.7 `SignInput`

Source: `wallet/mixing.go:121-153`. Runs in one read transaction with `context.Background()`.

1. Extract addresses from `prevScript` at script version 0; require exactly one `AddressPubKeyHashEcdsaSecp256k1V0`, else `errors.E(errors.Invalid, "previous output is not P2PKH")`.
2. `PrivateKey(addrmgrNs, addr)`; on error return it. The key is released when the transaction closure returns.
3. `sign.SignatureScript(tx, index, prevScript, txscript.SigHashAll, privKey.Serialize(), dcrec.STEcdsaSecp256k1, true)` (compressed public key). On error return `errors.E(errors.Op("sign.SignatureScript"), err)`.
4. Assign the result to `tx.TxIn[index].SignatureScript`.

The wallet must be unlocked (or the owning account unlocked) at the moment the client reaches the confirmation stage, which is minutes after the request started.

### 5.8 Mixed ticket split: `mixedSplit` and the mixing branches of `purchaseTickets`

`PurchaseTickets` guards (`wallet/wallet.go:1590-1620`):

1. `req.Mixing && !w.mixingEnabled` → `errors.E("wallet.PurchaseTickets", errors.Invalid, "wallet mixing support is disabled")`.
2. If the first `purchaseTickets` attempt fails with the internal "paying VSP fee requires UTXO split" condition (`errVSPFeeRequiresUTXOSplit`) and `req.DontSignTx` is false (with `DontSignTx` the first attempt's response and error are returned as they are, `wallet/wallet.go:1600-1602`), and `req.Mixing && req.MixedAccount == req.SourceAccount`, return `errors.E(op, errors.InsufficientBalance)` instead of creating the split.

`purchaseTickets` computes `neededPerTicket = ticketPrice + fee(relayFee, 298)`, where 298 is `est([108], [26, 32, 26], 0)` (`wallet/createtx.go:1217-1242`); at the default relay fee this is `ticketPrice + 2980` atoms. It then loops (`wallet/createtx.go:1433-1454`): call `mixedSplit` when `req.Mixing`, else `individualSplit`; if the error is `InsufficientBalance` and `req.Count > 1`, decrement `req.Count`, release the first reserved VSP-fee credit set, and retry; any other error returns `errors.E(op, err)`.

`mixedSplit(ctx, req, neededPerTicket)` (`wallet/createtx.go:1000-1093`):

1. Build `req.Count` placeholder outputs `wire.TxOut{Value: neededPerTicket, Version: 0, PkScript: 25 zero bytes}`.
2. `relayFee := w.RelayFee()`.
3. Lock `lockedOutpointMu`. In one read transaction:
   1. `inputSource := w.txStore.MakeInputSource(dbtx, req.SourceAccount, req.MinConf, tipHeight, ignoreInput)` where `ignoreInput` reports membership in `lockedOutpoints`.
   2. `changeSource := &p2PKHChangeSource{updates: &changeSourceUpdates, account: req.ChangeAccount, wallet: w, ctx: ctx, gapPolicy: gapPolicyIgnore}`.
   3. `atx, err := txauthor.NewUnsignedTransaction(mixOut, relayFee, inputSource.SelectInputs, changeSource, params.MaxTxSize)`. On error return it (kind `InsufficientBalance` when funds are short).
   4. Insert every selected input's `{hash, index}` into `lockedOutpoints` and remember them for unlocking.
4. Unlock the mutex. On error from step 3 return.
5. For each selected input log at info level `Selected input <outpoint> (<amount>) for ticket purchase split transaction`.
6. `change` = `atx.Tx.TxOut[atx.ChangeIndex]` when `atx.ChangeIndex >= 0`, else nil. If `change.Value < smallestMixChange(relayFee)`, set `change = nil` (the amount becomes fee; the change address already taken from `req.ChangeAccount` stays consumed).
7. `gen := w.makeGen(ctx, req.MixedSplitAccount, req.MixedAccountBranch)`; `expires := w.dicemixExpiry(ctx)`; `cj := mixclient.NewCoinJoin(gen, change, int64(neededPerTicket), expires, uint32(req.Count))`.
8. For each selected input `i`: `w.addCoinJoinInput(ctx, cj, in, atx.PrevScripts[i], 0)`; on error return it.
9. `w.dicemix(ctx, cj)`; on error return it.
10. Log at info level `Completed CoinShuffle++ mix of ticket split transaction <hash>`. Return `cj.Tx()`, `cj.MixedIndices()`, nil.

Deferred actions, in execution order on return: remove every input locked in step 3.4 from `lockedOutpoints` (always); then, only when the result error is nil, `changeSourceUpdates.UpdateDB(ctx, w, nil)` whose error becomes the function's error.

`txauthor.NewUnsignedTransaction` calls the change source only when the leftover after the fee is non-zero and not dust (`wallet/txauthor/author.go:136-153`). The change address is an internal-branch address of `req.ChangeAccount` (or of account 0 when `req.ChangeAccount` is the imported-keys account, `wallet/addresses.go:805-812`); imported voting accounts are rejected with `cannot use voting accounts for normal transactions`.

After `mixedSplit` returns, `purchaseTickets` (`wallet/createtx.go:1456-1620`):

1. Unless `req.DontSignTx`: records the returned transaction (`NewTxRecordFromMsgTx`, `processTransactionRecord` under `lockedOutpointMu` in one read-write transaction), marks its hash recently published, and calls `n.PublishTransactions(ctx, splitTx)`. The mix client already recorded and published the same transaction through `mixingWallet.PublishTransaction`; this second insert finds the unmined record and the second publication is a duplicate (§10, Q3).
2. When `req.Mixing`: for each split output draws a trickle time `now + 20 s + rand.Duration(40 s)` (uniform in `[20 s, 60 s)`, CSPRNG) and sorts the times ascending.
3. For each index in `cj.MixedIndices()`, builds a ticket spending that output. The voting address comes from `req.VotingAccount` branch 1; the commitment (reward) address comes from `req.MixedAccount` branch `req.MixedAccountBranch` when mixing, otherwise from `req.SourceAccount` branch 1; both use `WithGapPolicyIgnore` and are derived with `nextAddress` opening its own read-write transaction, no deferred updates (`wallet/createtx.go:1187-1190`, `:1518-1535`).
4. Publishes ticket `i` only after waiting until the `i`-th trickle time; cancellation while waiting returns the partial response with `errors.E(op, ctx.Err())` (`wallet/createtx.go:1603-1620`).

Additional facts of this path: for each split output index the wallet logs at info level `Split output is <txhash>:<index>` (`wallet/createtx.go:1511`), the outpoint being built from `splitTx.TxHash()` with tree 0; the response's `SplitTx` is set to the returned transaction before the record/publish step (`wallet/createtx.go:1456-1457`); the trickle wait uses a fresh timer per ticket and consumes the sorted times front to back, and without mixing only a context check precedes each publication (`wallet/createtx.go:1603-1620`); with `req.DontSignTx` the function returns before the publication loop, so no trickle delay is applied (`wallet/createtx.go:1599-1601`).

`req.DontSignTx` does not prevent the mix: `mixedSplit` ignores it, so the coinjoin is still signed, recorded and published by the mix client.

### 5.9 `findEligibleOutputsAmount`

Source: `wallet/createtx.go:1829-1987`. The caller holds `lockedOutpointMu` and a read transaction.

Skip predicate for a credit (first match wins):

1. Its `{hash, index}` is in the per-call `seen` set (random phase only).
2. Its `{hash, index}` is in `lockedOutpoints`.
3. It is not `confirmed(minconf, height, currentHeight)`: confirmations are 0 for unmined or `height > currentHeight`, else `currentHeight - height + 1`; require `>= minconf`.
4. `minAmount != 0` and its amount is below `minAmount`.
5. `stdscript.ExtractAddrs(0, pkScript, params)` does not return exactly one address.
6. Script class maturity: stake generation, stake revocation, treasury add and treasury generation classes (P2PKH and P2SH forms) require `height >= 0 && currentHeight - height + 1 > CoinbaseMaturity`; stake change classes require `> SStxChangeMaturity`; plain secp256k1 P2PKH requires coinbase maturity only when the credit is from a coinbase; every other class is skipped. The treasury-add case (`wallet/createtx.go:1879`) is dead: `ExtractAddrs` returns no address for `STTreasuryAdd` (`dcrd/txscript/v4@v4.1.2/stdscript/addressv0.go:156`), so predicate 5 already skips it.
7. `Manager.AddrAccount` of the single address fails or differs from `account`.

Selection:

1. Random phase. `maxTries = UnspentOutputCount / 2` when `(amount != 0 || maxResults != 0) && minconf > 0`, else 0 (the count is logged at debug level as `Unspent bucket k/v count: <n>`). Up to `maxTries` times: `RandomUTXO(dbtx, minconf, currentHeight)`; an error is returned as is; a nil credit ends the phase without counting as a try; a skipped credit consumes a try; otherwise add it to `seen`, append `Input{OutPoint, PrevOut{Value, Version: 0, PkScript}}`, add its amount to the running total, and return immediately when `amount != 0 && total >= amount` or `maxResults != 0 && len == maxResults`.
2. If the try counter is above zero when the phase ends without returning, log debug `Abandoned random UTXO selection attempts after <n> tries`.
3. Full phase. Discard everything selected so far. Read all unspent credits (`UnspentOutputs`; an error is returned as is), shuffle them (`rand.ShuffleSlice`, CSPRNG), and apply the same predicate (without `seen`) and the same early-return conditions.
4. If `amount != 0` and the total is below `amount`, return the bare kind `errors.InsufficientBalance`. Otherwise return what was selected.

For `MixAccount` (`amount = 0`, `minAmount = 262144`, `maxResults = 10 × MixSplitLimit`, `minconf = 2`) the result is up to `maxResults` eligible credits in random order, or all eligible credits when fewer exist.

### 5.10 Ticket buyer loop as account mixer

Source: `ticketbuyer/tb.go:63-214`.

1. If `passphrase` is non-empty, `wallet.Unlock(ctx, passphrase, nil)`; on error return it.
2. Subscribe to main-tip-changed notifications; derive a cancellable context.
3. For each notification with at least one attached block:
   1. Skip (debug log) if `RescanPoint` errors or is non-nil (transactions not synced through the tip).
   2. Load the header of the last attached block; on error log it and skip.
   3. If `height + 2 >= nextIntervalStart`: cancel all outstanding purchase contexts; recompute `nextIntervalStart = (height / StakeDiffWindowSize + 1) × StakeDiffWindowSize`; if `height + 2 == nextIntervalStart` log debug `Skipping purchase: next sdiff interval starts soon` and skip the whole iteration (including the change mix); set `expiry = nextIntervalStart`, plus one window when `height + 1 == nextIntervalStart`.
   4. Copy the config under the mutex. If `cfg.Mixing`: `multiple = cfg.Limit` and the copy's `Limit` becomes 1; otherwise `multiple = 1`.
   5. Create a purchase context (cancelled by step 3.3 of a later iteration) and, when `cfg.BuyTickets`, start `multiple` goroutines each calling `buy` with it. Errors of kind `InsufficientBalance`, `context.Canceled` and `context.DeadlineExceeded` are silent; others log `Ticket purchasing failed: <err>`; a `Passphrase` error additionally ends `Run` with that error.
   6. Start one goroutine calling `tb.mixChange(ctx, &cfg)` with the loop context (not the purchase context). `context.Canceled` is silent; any other error logs `Account mixing failed: <err>` at error level.
4. On context cancellation return the recorded fatal error if any, else `ctx.Err()`.

`buy` under mixing (`ticketbuyer/tb.go:216-312`): `minconf` is 2 instead of 1; the request carries `Mixing`, `MixedAccount`, `MixedAccountBranch`, `MixedSplitAccount = cfg.TicketSplitAccount`, `ChangeAccount`; with the per-goroutine `Limit` of 1 each goroutine buys at most one ticket, so a block's purchases are `multiple` independent single-ticket mix requests at the same denomination.

Additional details of `Run` (`ticketbuyer/tb.go:63-214`):

- Log lines: debug `Skipping autobuyer actions: RescanPoint err: <err>`; debug `Skipping autobuyer actions: transactions are not synced`; the header lookup error is logged bare at error level (`log.Error(err)`); error `Calculating sdiff failed: <err>`.
- Only the last attached block of a notification is considered; notifications with no attached blocks are ignored.
- `nextIntervalStart` starts at 0, so the first processed tip always enters step 3.3.
- The stake difficulty is computed lazily and at most once per tip (`sync.OnceValue`) by `wallet.NextStakeDifficultyAfterHeader(loopCtx, tipHeader)`, shared by all purchase goroutines of that tip. On error it logs the line above and yields 0; every purchase goroutine that sees 0 returns without calling `buy` and without logging. It is not computed at all when `BuyTickets` is false (pure account mixer).
- One purchase context is created per processed tip, whether or not `BuyTickets` is set, and its cancel function is appended to a slice that is only drained at step 3.3; a pure account mixer therefore accumulates one live child context per block until the next window boundary.
- Goroutines started by `Run` are not awaited when `Run` returns; they end through context cancellation.
- The config copy taken in step 3.4 is shared by pointer between the purchase goroutines and the change-mix goroutine of that tip; neither writes to it.

#### 5.10.1 `TB.buy(ctx, passphrase, sdiff, expiry, cfg)`

Source: `ticketbuyer/tb.go:216-312`. Runs inside a `ticketbuyer.buy` runtime trace task.

1. `n, err := w.NetworkBackend()`; on error return it (kind `NoPeers`; this is not one of the silenced errors, so `Run` logs `Ticket purchasing failed: …` for every purchase goroutine on every tip while no backend is set).
2. `ctx, cancel := n.WrapContext(ctx)`; `cancel` deferred.
3. If `passphrase` is non-empty: `w.Unlock(ctx, passphrase, nil)`; on error return it. A `Passphrase`-kind error ends `Run` (§5.10 step 3.5). This re-unlock happens on every purchase attempt, never for the change mix.
4. `minconf` = 2 when `cfg.Mixing`, else 1.
5. Ticket count `buy`:
   1. If `cfg.Maintain != 0`: `bal := w.AccountBalance(ctx, cfg.Account, minconf)` (error returned); if `bal.Spendable < Maintain` log debug `Skipping purchase: low available balance` and return nil; `buy = int((Spendable - Maintain) / sdiff)` (truncating); if 0 log the same line and return nil; clamp to `int(params.MaxFreshStakePerBlock)`.
   2. Else `buy = int(params.MaxFreshStakePerBlock)`.
   3. If `cfg.Limit == 0 && cfg.Mixing`: `buy = 1`. Else if `cfg.Limit > 0 && buy > cfg.Limit`: `buy = cfg.Limit`. From `Run`, `cfg.Limit` is always 1 under mixing, so `buy` is 1.
6. Build `wallet.PurchaseTicketsRequest{Count: buy, SourceAccount: cfg.Account, VotingAccount: cfg.VotingAccount, MinConf: minconf, Expiry: expiry, Mixing: cfg.Mixing, MixedAccount: cfg.MixedAccount, MixedAccountBranch: cfg.MixedAccountBranch, MixedSplitAccount: cfg.TicketSplitAccount, ChangeAccount: cfg.ChangeAccount, VSPClient: tb.cfg.VSP}`. The VSP client is read from the live `tb.cfg` without taking `tb.mu`, not from the per-tip copy (`ticketbuyer/tb.go:302`).
7. `tix, err := w.PurchaseTickets(ctx, n, req)`. For every hash in a non-nil response (also when `err` is non-nil, which happens when trickled publication is interrupted) log at info level `Purchased ticket <hash> at stake difficulty <sdiff>` (`sdiff` formatted as `dcrutil.Amount`). Return `err`.

The `multiple` purchase goroutines of one tip evaluate the balance in step 5 independently; `tb.go` contains no coordination between them. Without mixing, `ChangeAccount` is still passed and is where the non-mixed split transaction sends its change (`wallet/createtx.go:1126`); the process start-up code sets it to the purchase account when `--changeaccount` is empty (§5.18).

Change mixes are started on every tip without waiting for earlier ones. Overlap is safe because outputs being mixed are locked and therefore not selected again, and because the split semaphores bound concurrency.

### 5.11 `PossibleCoinJoin(tx)`

Source: `wallet/mixing.go:517-557`.

1. If the transaction has fewer than 3 outputs or fewer than 3 inputs, return `(false, 0, 0)`.
2. Walk the outputs in order. Count occurrences of each `PkScript` (byte-exact); on the second occurrence of any script return `(false, 0, 0)`. Skip zero-valued outputs; otherwise count occurrences of the value.
3. For each distinct value with a count of at least 3: if the value is greater than the current `mixDenom`, set `mixDenom` and `mixCount` to it and its count; then if `len(outputs) - count > len(inputs)` return `(false, 0, 0)`.
4. Return `isMix = mixCount >= uint32(len(outputs) / 2)` (integer division) together with `mixDenom` and `mixCount`.

The result does not depend on map iteration order: any qualifying value that fails the step 3 test forces `(false, 0, 0)`, and otherwise `mixDenom` is the largest qualifying value. When `isMix` is false at step 4, `mixDenom` and `mixCount` are still returned non-zero if any value qualified.

### 5.12 `GetCoinjoinTxsSumbByAcct`

Source: `wallet/wallet.go:5556-5609`. One read transaction.

1. Read the tip height from the transaction store.
2. `RangeTransactions(ctx, txmgrNs, 0, tipHeight, f)`: mined transactions from height 0 through the tip; unmined transactions are not visited.
3. For each transaction of type regular with `PossibleCoinJoin` true: for each output whose value equals `mixDenom`, extract addresses at the output's own script version; if exactly one address and `Manager.AddrAccount` succeeds, increment that account's counter.
4. Return the map. Errors are wrapped twice, as two nested `*errors.Error` values with different ops; in the dcrwallet process (nested-error separator `:: `, §8.2) the text is `wallet.GetCoinjoinTxsSumbByAcct:: wallet.getCoinjoinTxsSumbByAcct: …`.

The values are output counts, not amounts and not transaction counts.

### 5.13 Outpoint locking during mixes

| Event | Effect on `lockedOutpoints` | Source |
| --- | --- | --- |
| `MixOutput` passes its checks | inserts the one outpoint | `wallet/mixing.go:310` |
| `MixOutput` returns (any path after insertion) | removes it | `wallet/mixing.go:313-317` |
| `mixedSplit` selects inputs | inserts every selected input | `wallet/createtx.go:1051-1055` |
| `mixedSplit` returns | removes them | `wallet/createtx.go:1017-1025` |
| Any non-vote, non-revocation transaction is recorded (`processTransactionRecord`), including the finished coinjoin | removes each of its inputs | `wallet/chainntfns.go:464-473` |
| JSON-RPC `lockunspent` with unlock and no outputs | replaces the whole set with an empty one, releasing reservations of in-flight mixes | `wallet/wallet.go:4278-4282` |

`MixAccount` reads the set while selecting (holding the mutex) but does not insert; each `MixOutput` inserts its own outpoint later. Two overlapping `MixAccount` calls can therefore select the same output; the later `MixOutput` fails with `output … already locked`, which `MixAccount` does not suppress.

The key excludes the tree. Lock acquisition order where both are needed: `lockedOutpointMu` first, then the database transaction (`wallet/mixing.go:279-311`, `:460-476`; `wallet/createtx.go:1031-1058`).

### 5.14 SPV mode

#### 5.14.1 `spv.Syncer.PublishMixMessages` (`spv/backend.go:479-520`)

1. Build one `inv` with an `InvTypeMix` vector per message, using `msg.Hash()`. `AddInvVect` failure → `errors.E("spv.PublishMixMessages", errors.Protocol, err)`.
2. For each message that is a key exchange, remember `ke.SeenPRs[ke.Pos]` (the sender's own pair request hash).
3. Under `remotesMu`, for each connected peer (map order): skip peers with `Pver() < 10`; otherwise count it, add every inventory hash and every remembered pair request hash to the peer's `InvsSent` cache, and `rp.SendMessage(ctx, inv)` (enqueue on the peer's output channel with a 30 s timeout; no trickle batching). The first error stops the iteration and is returned wrapped with the op.
4. No connected peers at all → `errors.E(op, errors.E(errors.NoPeers))`. Zero mix-capable peers → `errors.E(op, errors.Protocol, "no connected peers support the mixing protocol version")`.

#### 5.14.2 Serving `getdata` (`spv/sync.go:779-888`, `p2p/peering.go:1302-1325`)

1. Each received `getdata` increases the peer's ban score by `len(InvList) × 100 / 50000`; above 100 the peer is disconnected with `ban score reached`.
2. One goroutine per message. For each vector: if the hash is not in the peer's `InvsSent` cache → `notfound`. `InvTypeTx` → transaction lookup. `InvTypeMix` → `notfound` when mixing is disabled, else queued for mix lookup. Other types → `notfound`.
3. For each queued mix hash: `wallet.MixMessage(hash)`; an error turns it into a `notfound` vector of type `InvTypeMix`.
4. Send found transactions, then found mix messages (each as its own wire message, in request order), then one `notfound` listing everything missing. Send errors are logged at warn level (`Failed to send getdata reply to peer <addr>: <err>` for transactions and mix messages, `Failed to send notfound reply to peer <addr>: <err>` for the `notfound`) and do not stop the remaining sends; context cancellation ends the goroutine silently. A transaction lookup error other than `NotExist` logs `Failed to look up transactions for getdata reply to peer <addr>: <err>` at warn level and ends the goroutine without sending anything, mix messages included (`spv/sync.go:818-823`).

Only messages this wallet announced to that peer (its own messages, plus its own pair requests referenced by its key exchanges) are ever served; the wallet does not relay third-party mix messages.

#### 5.14.3 Consuming `inv` (`spv/sync.go:892-951`, `:1182-1259`; `p2p/peering.go:1645-1707`, `:1283-1301`, `:2227-2247`)

1. `p2p`: if the local peer was configured with relay disabled and the `inv` contains any transaction or mix vector, disconnect the peer with `received tx in msginv when tx relaying is disabled` (kind `Protocol`).
2. Syncer: collect `InvTypeMix` hashes only when mixing is enabled; start one goroutine `handleMixInvs(ctx, rp, hashes, nil)` per `inv`.
3. `handleMixInvs`:
   1. Drop hashes present in `seenMixMsgs`. If none remain, return.
   2. `rp.MixMessages(ctx, unseen)`: one `getdata` with all hashes; a hash already being requested from this peer fails the call with `mix msg <hash> is already being requested from this peer`; a 30 s stall timer covering the whole batch disconnects the peer with `peer appears stalled` (kind `IO`).
   3. A `NotExist` error keeps the non-nil results (this branch is unreachable, §10 Q5). Any other error is logged at warn level with op `spv.handleMixInvs(<addr>)` unless the context is done, and the call returns.
   4. Add every fetched hash to `seenMixMsgs`.
   5. For each message, in order: when `onlyByID` is non-empty, skip messages whose 33-byte identity is not in it. Call `wallet.AcceptMixMessageBySource(msg, rp)`. If the error is a `*mixpool.MissingOwnPRError`: log debug `Will request unknown PR from <identity hex>`, record `MissingPR` and the key exchange's identity. Any other error is logged at warn level.
   6. If any pair requests were recorded as missing, recurse once per level: `handleMixInvs(ctx, rp, missingPRHashes, identities)`, which fetches them from the same peer and accepts only those authored by the recorded identities.
4. `p2p` on receiving any of the eight mix message types: compute and store the message hash (`WriteHash` with a package-global BLAKE-256 hasher under a mutex); look up and delete the pending request for that hash; if none, disconnect the peer with `received unrequested mix msg` (kind `Protocol`); otherwise deliver the message to the waiting `MixMessages` call.

Mixpool rule violations by a peer are logged only; the SPV syncer never bans or disconnects a peer for them.

Details of `RemotePeer.MixMessages` (`p2p/peering.go:1645-1707`) that a peer or the syncer can observe:

1. Registration happens hash by hash before anything is sent. One buffered (capacity 1) channel is registered per hash. If a hash is already registered — by a concurrent call, or because the same hash appears twice in this call's list (a peer announcing one mix hash twice in a single `inv`) — the registrations made so far by this call (`hashes[:i]`) are removed and the call fails with op `remotepeer(<addr>).MixMessages` and text `mix msg <hash> is already being requested from this peer`; no `getdata` is sent. `AddInvVect` failure (more than the wire maximum of inventory vectors) returns the wrapped error without removing earlier registrations.
2. The `getdata` is enqueued on the peer's output channel with no deadline other than the context; if the context ends first all registrations are removed and `ctx.Err()` is returned.
3. A single 30 s timer is started after the enqueue and covers the whole batch. Replies are awaited strictly in request order (`cs[0]`, then `cs[1]`, …); replies arriving out of order are parked in their buffered channels.
4. Context cancellation during the wait returns `ctx.Err()` immediately but leaves the remaining registrations in place until the 30 s timer fires (a helper goroutine removes them then). A late reply arriving in that window is therefore accepted silently rather than treated as unrequested.
5. Stall: the remaining registrations are removed, the peer is disconnected with kind `IO`, `peer appears stalled`, and that error is returned.
6. Peer failure (`rp.errc` closed): the timer is stopped and the peer's terminal error is returned; registrations are not removed (the peer object is dead).
7. The result slice is index-aligned with the request.

Details of `handleMixInvs` (`spv/sync.go:1182-1259`):

- Hashes are added to `seenMixMsgs` only after a successful fetch. A failed fetch (stall, duplicate request, peer error) leaves them unseen, so a later `inv` from any peer triggers another fetch.
- Every fetched hash is marked seen whether or not the pool accepts the message and whether or not the `onlyByID` filter skips it.
- The missing-pair-request recursion goes through the same `seenMixMsgs` filter: a pair request hash that was fetched before (and rejected or since removed from the pool) is not fetched again, and the orphan key exchange stays an orphan.
- The message identity used for the `onlyByID` filter is `msg.Pub()` converted to a 33-byte array.
- The key exchange is obtained with an unchecked type assertion on the message that produced `MissingOwnPRError`; the pool returns that error only for key exchanges.
- `receiveInv` and `receiveGetData` track their handler goroutines in a wait group and wait for all of them before returning their own error (`spv/sync.go:780-786`, `:893-898`).

#### 5.14.4 `wallet.Run` hosting (`spv/sync.go:396-465`)

`walletCtx` is a background-derived context cancelled when either the caller's context or the syncer's handler group context ends. One group goroutine calls `s.wallet.Run(walletCtx)`; on a nil return (mixing disabled) it waits for `walletCtx` and returns its error. The mix client therefore starts as soon as the syncer starts, concurrently with the initial header and filter sync, and before the wallet is synced.

#### 5.14.5 Minimum-version peer target (`spv/sync.go:536-545`, `:591-608`)

A newly connected non-persistent peer with protocol version below 10 is disconnected (debug log, error `peer uses too low version to satisfy …`) when there are already at least 5 (`targetPeerCount - minVersionTarget`) connected peers and fewer than 3 of them have version ≥ 10. The rule applies whether or not mixing is enabled.

### 5.15 dcrd-RPC mode

#### 5.15.1 Start-up (`chain/sync.go:726-757`)

After the initial sync steps (headers, `notifyblocks`, `rebroadcastwinners`, unmined rebroadcast, tspends, `notifytspend`):

1. Create `walletCtx` from the background context; start a group goroutine running `s.wallet.Run(walletCtx)`.
2. If mixing is enabled, call `notifymixmessages` with no parameters; an error ends the sync session.
3. Log `Blockchain sync completed, wallet ready for general usage.`
4. A final group goroutine waits for the session's group context (an `errgroup.WithContext` child of the caller's context, `chain/sync.go:637`, so it also ends when any group goroutine returns an error) or the websocket client to end, cancels `walletCtx`, and returns that error.

#### 5.15.2 `mixmessage` notification (`chain/sync.go:801-805`, `:933-966`)

1. If mixing is disabled: log debug `Ignoring mixmessage notification: mixing disabled`, return nil.
2. Decode with `dcrd.MixMessage(params)`; on error return it.
3. Hash the message (`WriteHash` with the syncer's BLAKE-256 hasher under its mutex).
4. `wallet.AcceptMixMessageBySource(msg, mixpool.ZeroSource)`.
5. If the error is a `*mixpool.MissingOwnPRError`: if the message is not a key exchange or its `Run` is non-zero, return the error. Otherwise `s.rpc.MixMessage(ctx, &e.MissingPR)` (`getmixmessage`); on success hash the pair request and accept it with `ZeroSource`, returning that result; on failure return the RPC error.
6. Otherwise return the acceptance error.

A non-nil return is logged at error level as `errors.E("mixmessage", err)` by the notification dispatcher (`mixmessage: <err>` for a plain error such as a mixpool rule error; `mixmessage:: <op>: …` when the error is itself a wallet `*errors.Error` with its own op, §8.2) and does not end the session. Notifications are handled one at a time in arrival order by the notifier.

#### 5.15.3 Publication (`rpc/client/dcrd/calls.go:243-268`)

For each message, serialise with `BtcEncode` at protocol version 10 into lowercase hex and call `sendrawmixmessage` with `[msg.Command(), hex]`. A serialisation failure is kind `Encoding`. All messages are attempted sequentially; the first error is returned as `errors.E("dcrd.PublishMixMessages", firstErr)`. The mix client submits one message per call.

### 5.16 JSON-RPC methods (semantics)

Common: all use the server-wide options `MixingEnabled`, `MixAccount` (account name), `MixBranch`, `MixChangeAccount` (account name), `TicketSplitAccount` (account name) fixed at server start from `--mixing`, `--mixedaccount`, `--changeaccount`, `--ticketsplitaccount`.

#### 5.16.1 `mixaccount`

1. `!MixingEnabled` → error `Mixing is not configured` (plain error; code `-4`, `ErrRPCWallet`).
2. No loaded wallet → `request requires a wallet but wallet has not loaded yet` (`-4`).
3. Resolve `MixAccount` then `MixChangeAccount` by name; `NotExist` → `account not found` (`ErrRPCWalletInvalidAccountName`); other errors pass through.
4. `w.MixAccount(ctx, changeAccount, mixAccount, MixBranch)`; return `null` result and the error. The call blocks for the whole mix.

#### 5.16.2 `mixoutput "outpoint"`

Steps 1–2 as above. Then parse the outpoint: length ≥ 66, byte 64 is `:`, first 64 characters a hash string, remainder a base-10 `uint32`; failures are `parseOutpoint: bad len`, `parseOutpoint: bad separator`, or the wrapped parse error. The resulting `wire.OutPoint` has tree 0. Resolve accounts as in `mixaccount`, then `w.MixOutput(ctx, outpoint, changeAccount, mixAccount, MixBranch)`; return `null` and the error. A throttled request returns `throttled mix request for split denomination` to the client.

#### 5.16.3 `getcoinjoinsbyacct`

`w.GetCoinjoinTxsSumbByAcct`, then each account number is resolved to its name; the result is a JSON object `{accountName: count}`. A `NotExist` during name resolution is returned with the internal-error code.

Further details (`internal/rpc/jsonrpc/methods.go:5834-5863`): no `MixingEnabled` check is made, so the method works on a wallet without `--mixing`; an unloaded wallet yields `request requires a wallet but wallet has not loaded yet`; an error from `GetCoinjoinTxsSumbByAcct` of kind `Passphrase` is mapped to `ErrRPCWalletPassphraseIncorrect` with message `incorrect passphrase` (unreachable in practice: the call performs no decryption), any other error passes through; accounts with no qualifying output are absent from the object (the object is `{}` for a wallet with no coinjoins).

#### 5.16.4 `purchaseticket` with mixing enabled

When `MixingEnabled`: `MixedAccount` = number of `MixAccount`, `MixedAccountBranch` = `MixBranch` (must be 0 or 1), `MixedSplitAccount` = number of `TicketSplitAccount`, `ChangeAccount` = number of `MixChangeAccount`; resolution failures are `ErrRPCInvalidParameter` with messages `Mixing enabled, but error on mixed account: <err>`, `MixedAccountBranch should be 0 or 1.`, `Mixing enabled, but error on mixedSplitAccount: <err>`, `Mixing enabled, but error on changeAccount: <err>`. `Mixing` is set from `MixingEnabled`, and the voting account is the mixed account instead of the source account. Without mixing the change account is the source account.

#### 5.16.5 Sends from the mixed account

`sendPairs` (used by `sendfrom`, `sendmany`, `sendtoaddress`): when `MixingEnabled` and both `MixAccount` and `MixChangeAccount` are non-empty and the spending account equals the mixed account, the change account becomes `MixChangeAccount`. `sendAmountToTreasury`: the same redirection when `MixingEnabled`, without the non-empty checks (an empty `MixAccount` name then fails the account lookup). In all other cases change returns to the spending account. Account lookup errors in either function are returned as they are (not mapped to `account not found`).

Additional `purchaseticket` facts (`internal/rpc/jsonrpc/methods.go:3386-3476`): the four mixing lookups run in the order mixed account, branch check, split account, change account, and the first failure wins; they run before the VSP client is created; when `--vsp.url` is configured the VSP policy is `{FeeAcct: source account, ChangeAcct: change account}`, so under mixing VSP fee change also goes to `--changeaccount`; `numtickets` is passed through unchecked against `mixing.MaxMcount` (a mixed request for more than 16 tickets fails at pool acceptance, §10 Q18).

#### 5.16.6 Help text

The generated help strings are observable through `help` and are, byte for byte (`internal/rpc/jsonrpc/rpcserverhelp.go:34`, `:69-70`):

- `mixaccount`: `"mixaccount\n\nMix all outputs of an account.\n\nArguments:\nNone\n\nResult:\nNothing\n"`
- `mixoutput`: `"mixoutput \"outpoint\"\n\nMix a specific output.\n\nArguments:\n1. outpoint (string, required) Outpoint (in form \"txhash:index\") to mix\n\nResult:\nNothing\n"`
- `getcoinjoinsbyacct`: `"getcoinjoinsbyacct\n\nGet coinjoin outputs by account.\n\nArguments:\nNone\n\nResult:\n{\n \"Accounts name\": Coinjoin outputs sum., (object) Return a map of account's name and its coinjoin outputs sum.\n ...\n}\n"`

The help generator declares the `getcoinjoinsbyacct` result type as `map[string]uint32` (`internal/rpchelp/methods.go:57`) while the handler returns `map[string]int`; the JSON is the same for all reachable values.

### 5.17 gRPC methods (semantics)

#### 5.17.1 `AccountMixerService.RunAccountMixer(RunAccountMixerRequest) → stream RunAccountMixerResponse`

Request fields: `passphrase` (bytes, 1), `mixed_account` (uint32, 2), `mixed_account_branch` (uint32, 3), `change_account` (uint32, 4). The response message is empty and is never sent.

1. Build `ticketbuyer.New(wallet, Config{Mixing: true, MixedAccount: req.MixedAccount, MixedAccountBranch: req.MixedAccountBranch, ChangeAccount: req.ChangeAccount, BuyTickets: false, MixChange: true})`. No account or branch validation is performed and the wallet's mixing flag is not checked.
2. If the passphrase is non-empty: `wallet.Unlock(streamCtx, passphrase, lock)`; on error return the translated error. On return from the handler a value is sent on `lock` (relocking per the unlock spec) and the passphrase bytes are zeroed.
3. `tb.Run(streamCtx, req.Passphrase)` (§5.10).
4. On a non-nil result: if the stream context is done return status `Canceled` with message `AccountMixer instance canceled, account number: <mixed_account>`; otherwise status `Unknown` with message `AccountMixer instance errored: <err>`.

The stream stays open until the client cancels it. Per-block mixing failures (including "wallet mixing support is disabled") are only logged.

The service reports not ready (`FailedPrecondition`, `service walletrpc.AccountMixerService is not ready`) until `StartAccountMixerService` runs after a wallet is loaded.

#### 5.17.2 `WalletService.PurchaseTickets`

When `enable_mixing`: `mixed_account`, `mixed_split_account` and `change_account` must name existing accounts and `mixed_account_branch` must be 0 or 1; failures are `InvalidArgument` with `Mixing requested, but error on mixed account: <err>`, `MixedAccountBranch should be 0 or 1.`, `Mixing requested, but error on mixedSplitAccount: <err>`, `Mixing requested, but error on changeAccount: <err>`. The request's `Mixing` is `enable_mixing`. When `use_voting_account` is false the voting account is the mixed account under mixing, else the source account.

Further details (`internal/rpc/rpcserver/server.go:1791-1911`):

1. Order of checks: network backend (`wallet.NetworkBackend()` error returned untranslated), VSP client creation, `num_tickets < 1` (`InvalidArgument`, `Zero or negative number of tickets given`), `tx_fee < 0` (`InvalidArgument`, `Negative fees per KB given`), then the four mixing validations in the order mixed account, branch, split account, change account.
2. `change_account` is validated only when `enable_mixing` is set, but it is always copied into `PurchaseTicketsRequest.ChangeAccount` and, when a VSP is given, into the VSP policy `ChangeAcct`. A non-mixed request that leaves `change_account` unset therefore sends the split transaction's change and the VSP fee change to account 0, not to the purchasing account (the JSON-RPC method defaults to the purchasing account instead).
3. Account existence is tested with `wallet.AccountName(ctx, number)`; the `<err>` in the message is that call's error text.
4. The wallet is unlocked with the request passphrase only when `dont_sign_tx` is false and the passphrase is non-empty; a mixed request with `dont_sign_tx` set and a locked wallet fails in `addCoinJoinInput` with kind `Locked` (the mix itself is not suppressed by `dont_sign_tx`, §10 Q4).

#### 5.17.3 `TicketBuyerService.RunTicketBuyer`

Same four validations and messages as §5.17.2. `ticketbuyer.Config{BuyTickets: true, Mixing: enable_mixing, MixChange: enable_mixing, MixedAccount, MixedAccountBranch, TicketSplitAccount: mixed_split_account, ChangeAccount: change_account, Limit: int(limit), …}`. Change mixing is therefore always on when mixed buying is requested over gRPC.

Further details (`internal/rpc/rpcserver/server.go:2588-2686`):

1. Order of checks: VSP client creation (when `vsp_host` or `vsp_pubkey` is non-empty; failure is `InvalidArgument`, `VSP client failed to start. Error: <err>`), `balance_to_maintain < 0` (`InvalidArgument`, `Negative balance to maintain given`), then the mixing validations.
2. The VSP policy is `{FeeAcct: account, ChangeAcct: account}` regardless of mixing and of `change_account`.
3. `voting_account` is passed through as given. Unlike `PurchaseTickets` and JSON-RPC `purchaseticket` it is not replaced by the mixed account under mixing; an unset value is account 0.
4. `change_account` is copied into the config whether or not mixing is enabled (unset = account 0), and is the change account of non-mixed split transactions too.
5. `limit` is converted with `int(req.Limit)`; under mixing it becomes the number of concurrent single-ticket purchases per block, and 0 buys nothing (§10 Q7).
6. Passphrase handling and stream termination mirror `RunAccountMixer`, with the strings `TicketBuyer instance canceled, account number: <account>` (`Canceled`) and `TicketBuyer instance errored: <err>` (`Unknown`). No response message is ever sent.

#### 5.17.4 `WalletService.GetCoinjoinOutputspByAcct`

Returns one `coinjoinTxsSumByAcct{account_number, coinjoin_txs_sum = int32(count)}` per map entry, in Go map iteration order (unspecified). The request message is empty; the count is narrowed with `int32(count)` without a range check; a wallet error is passed through `translateError`; with no qualifying outputs `data` is empty. No mixing-enabled check is made.

### 5.18 Process start-up (`config.go:739-782`, `dcrwallet.go:249-365`)

Configuration loading:

1. If `--mixing`:
   - `--csppsolver` given explicitly: set `solverrpc.SolverProcess` to it; the solver must work.
   - else try `solverrpc.StartSolver()` (runs `csppsolver` from `PATH`): success → the solver must work; failure → log warn `Unable to start csppsolver; must rely on other peers publishing results` and continue.
2. If the solver must work: `testStartedSolverWorks()` starts the process (idempotent) and calls `Roots([1, 2], F)`; a start error aborts start-up with `csppsolver process is not operating properly: <err>` on stderr; a `Roots` error that `Is` `io.ErrUnexpectedEOF` or `rpc.ErrShutdown` aborts with `csppsolver process is not operating properly: rpc failed: <err>` (`testsolver.go:30-32`). Any other `Roots` error (including `solver process exited`) is accepted.
3. Parse `--mixedaccount`, only when it is non-empty (an empty value leaves the parsed name empty and the branch 0, with no error): split at the last `/`; no slash → `--mixedaccount must have form 'accountname/branch'`; branch text other than `0` or `1` → `--mixedaccount branch must be 0 or 1`. Both abort start-up.
4. If `--ticketsplitaccount` is empty it takes the mixed account name.

After the wallet is opened (not under `--noinitialload`):

1. The private passphrase is prompted for (or taken from `--pass`) when `--enableticketbuyer` is set, an accounts sync is needed, or `--promptpass` is set; `--mixchange` alone does not prompt.
2. Only when `--vsp.url` is set — VSP change account: `--changeaccount`, or the purchase account when empty and mixing is disabled (warn log `Change account not set, using purchase account "<name>"`); with mixing enabled an empty change account fails the account lookup and aborts start-up.
3. If `--mixchange` or `--enableticketbuyer`:
   - with the ticket buyer: resolve `--purchaseaccount`; if `--mixing` and no `--ticketbuyer.votingaccount`, fail with `cannot run mixed ticketbuyer without --votingaccount`; voting account = `--ticketbuyer.votingaccount` or the purchase account; change account = `--changeaccount` or the purchase account;
   - if (ticket buyer and mixing) or `--mixchange`: resolve `--mixedaccount` (name part) and `--changeaccount` (mandatory here);
   - if ticket buyer and mixing: resolve `--ticketsplitaccount`;
   - the first failing lookup aborts start-up after logging `<flag>: account "<name>" does not exist` (whatever the underlying error);
   - start `ticketbuyer.New(w, Config{BuyTickets: --enableticketbuyer, Account, Maintain, Limit: --ticketbuyer.limit, VotingAccount, Mixing: --mixing, MixChange: --mixchange, MixedAccount, MixedAccountBranch, TicketSplitAccount, ChangeAccount, VSP})` and log `Starting auto transaction creator`. When its `Run` ends with an error other than cancellation, log `Transaction creator ended: <err>`.

Further start-up details (`dcrwallet.go:241-366`):

- With `--pass` set, the wallet is unlocked at start-up with it; a failure logs `Incorrect passphrase in pass config setting.` at error level and aborts. The same passphrase bytes are handed to `tb.Run`, which unlocks again (§5.10 step 1).
- VSP block log lines (warn level): `Failed to get account number for ticket change account "<name>": <err>`, `Failed to get account number for ticket purchase account "<name>": <err>`; a VSP client construction failure logs `VSP: <err>` at error level. The VSP policy is `{MaxFee: --vsp.maxfee, FeeAcct: purchase account, ChangeAcct: change account}`.
- The `cannot run mixed ticketbuyer without --votingaccount` error is logged at error level and returned; note the flag is spelled `--ticketbuyer.votingaccount` on the command line.
- The lookup helper stops at the first failure: later lookups are skipped, and the flag names used in the log line are `purchaseaccount`, `ticketbuyer.votingaccount`, `changeaccount`, `mixedaccount`, `ticketsplitaccount` (without leading dashes).
- With the ticket buyer and mixing both enabled, `--changeaccount` is looked up twice when non-empty (once in the ticket buyer branch, once in the mixing branch); when empty, the ticket buyer branch substitutes the purchase account and the mixing branch then fails on the empty name.
- `MixedAccountBranch` is always taken from the parsed `--mixedaccount` (0 when the option is empty).
- The ticket buyer goroutine is started before the RPC servers and before the syncer; the main function waits for it to finish (a deferred receive on its done channel) before returning, so process shutdown waits for `tb.Run` to observe cancellation.
- The whole block, including the account mixer, is skipped under `--noinitialload`; the gRPC `AccountMixerService` is then the only way to run the loop.

---

## 6. Constants, limits and defaults

### 6.1 Constants

| Name | Value | Source | What depends on it |
| --- | --- | --- | --- |
| `splitPoints[0]` | 2^36 = 68,719,476,736 atoms (687.19476736 DCR) | `wallet/mixing.go:215` | Mixed denomination |
| `splitPoints[1]` | 2^34 = 17,179,869,184 (171.79869184) | `wallet/mixing.go:216` | Mixed denomination |
| `splitPoints[2]` | 2^32 = 4,294,967,296 (42.94967296) | `wallet/mixing.go:217` | Mixed denomination |
| `splitPoints[3]` | 2^30 = 1,073,741,824 (10.73741824) | `wallet/mixing.go:218` | Mixed denomination |
| `splitPoints[4]` | 2^28 = 268,435,456 (2.68435456) | `wallet/mixing.go:219` | Mixed denomination |
| `splitPoints[5]` | 2^26 = 67,108,864 (0.67108864) | `wallet/mixing.go:220` | Mixed denomination |
| `splitPoints[6]` | 2^24 = 16,777,216 (0.16777216) | `wallet/mixing.go:221` | Mixed denomination |
| `splitPoints[7]` | 2^22 = 4,194,304 (0.04194304) | `wallet/mixing.go:222` | Mixed denomination |
| `splitPoints[8]` | 2^20 = 1,048,576 (0.01048576) | `wallet/mixing.go:223` | Mixed denomination |
| `splitPoints[9]` | 2^18 = 262,144 (0.00262144) | `wallet/mixing.go:224` | Smallest denomination; no change; `MixAccount` minimum output amount; base of `smallestMixChange` |
| Maximum mixed outputs per `MixOutput` | 4 | `wallet/mixing.go:342` | `mcount` of the pair request |
| `MixAccount` `minconf` | 2 | `wallet/mixing.go:464` | Output eligibility |
| `MixAccount` `maxResults` | `10 × MixSplitLimit` | `wallet/mixing.go:467` | Outputs attempted per call |
| Expiry margin | `mixing.MaxExpiry(tip) - 2` | `wallet/mixing.go:173` | Pair request `Expiry` |
| `P2PKHv0Len` | 25 | `wallet/mixing.go:352` | Size estimation |
| `txsizes.RedeemP2PKHSigScriptSize` | 108 | `wallet/txsizes/size.go:28` | Size estimation |
| `txsizes.P2PKHPkScriptSize` | 25 | `wallet/txsizes/size.go:66` | Size estimation |
| `smallestMixChange(rate)` | `fee(rate, 217) + 262144`; 264,314 atoms at 10000 atoms/kB | `wallet/mixing.go:227-234` | Whether a change output is created (`MixOutput`, `mixedSplit`) |
| Mixed ticket split denomination | `ticketPrice + fee(relayFee, 298)`; `ticketPrice + 2980` atoms at 10000 atoms/kB | `wallet/createtx.go:1235-1242` | Pairing between ticket buyers |
| Ticket trickle delay | uniform `[20 s, 60 s)` per ticket, sorted | `wallet/createtx.go:1492` | Publication time of mixed tickets |
| Ticket buyer `minconf` when mixing | 2 (otherwise 1) | `ticketbuyer/tb.go:252-255` | Input eligibility for mixed splits |
| `mixpool.ZeroSource` | `Uint64Source(0)` | `dcrd/mixing@v0.7.5/mixpool/mixpool.go:128` | Source of own and dcrd-RPC messages |
| Mixpool fee rate | 10000 atoms/kB (`0.0001e8`) | `dcrd/mixing@v0.7.5/mixpool/mixpool.go:31` | Minimum and maximum (×10000) pair request fee accepted locally and by peers |
| Mixpool dust limit for a 25-byte script | value < 6030 atoms | `dcrd/mixing@v0.7.5/mixpool/mixpool.go:1842-1855` | Change and mix amount acceptance |
| `mixing.MaxMcount` | 16 | `dcrd/mixing@v0.7.5/limits.go:30` | Upper bound on `req.Count` for a mixed split |
| `mixing.MaxMixAmount` | 21,000,000 DCR / 4 | `dcrd/mixing@v0.7.5/limits.go:41` | Upper bound on the mixed denomination |
| `mixing.MinPeers` | 4 | `dcrd/mixing@v0.7.5/client.go:9` | Minimum session size (spec 12) |
| Mix epoch | 10 min; 3 min on testnet3 | `dcrd/mixing@v0.7.5/mixpool/mixpool.go:280`, `:290-292` | Earliest completion of any mix; warm-up |
| `wire.MixVersion` | 10 | `dcrd/wire@v1.8.0/protocol.go:56` | Peer eligibility; message encoding version for dcrd RPC hex |
| `wire.InvTypeMix` | 4 | `dcrd/wire@v1.8.0/invvect.go:35` | Inventory vectors |
| `p2p.Pver` | 11 | `p2p/peering.go:48` | Advertised protocol version |
| `p2p.minPver` | 9 | `p2p/peering.go:45` | Lowest accepted peer version |
| `p2p.stallTimeout` | 30 s | `p2p/peering.go:52` | `MixMessages` batch deadline; `SendMessage` enqueue deadline |
| `p2p.invLRUSize` | 5000 | `p2p/peering.go:64` | Per-peer sent-inventory cache (gates `getdata` service) |
| `p2p.banThreshold` | 100 | `p2p/peering.go:62` | `getdata` ban score |
| `spv` seen-mix LRU | 2000 | `spv/sync.go:149` | Deduplication of fetched mix messages |
| `spv.targetPeerCount`, `minVersion`, `minVersionTarget` | 8, 10, 3 | `spv/sync.go:39-46` | Peer admission |
| `chain.requiredAPIVersion` | 8.3.0 | `chain/sync.go:33` | dcrd JSON-RPC compatibility |
| `defaultMixSplitLimit` | 10 | `config.go:62` | Default `--mixsplitlimit`; with it `MixAccount` attempts at most 100 outputs per call and at most 10 concurrent mixes per denomination |
| Ticket buyer per-request ticket count under mixing | 1 (`cfg.Limit` forced to 1; `--ticketbuyer.limit` becomes the fan-out) | `ticketbuyer/tb.go:167-171`, `:282-286` | `mcount` of mixed ticket split pair requests from the automatic buyer |
| `params.MaxFreshStakePerBlock` | chain parameter | `ticketbuyer/tb.go:275-281` | Upper bound of tickets per `buy` call without mixing |
| JSON-RPC `mixoutput` minimum argument length | 66 characters (64 hex + `:` + ≥ 1 digit) | `internal/rpc/jsonrpc/methods.go:5681-5686` | Outpoint parsing |
| `MixMessages` reply channel capacity | 1 per requested hash | `p2p/peering.go:1656` | Out-of-order replies do not block the peer's read loop |
| Solver probe coefficients | `[1, 2]` over `mixing.F` | `testsolver.go:25-29` | Start-up solver check |
| `solverrpc.SolverProcess` default | `csppsolver` | `cspp/v2@v2.4.0/solverrpc/rpc.go:15` | Solver subprocess name |

### 6.2 Configuration options

| Option | Type | Default | Behaviour |
| --- | --- | --- | --- |
| `--mixing` | bool | false | Sets `wallet.Config.MixingEnabled`, `jsonrpc.Options.MixingEnabled`, `ticketbuyer.Config.Mixing`. Description: `Enable creation of mixed transactions and participation in the peer-to-peer mixing network` |
| `--csppsolver` | string, records whether explicitly set | `csppsolver` | Path or name of the solver executable. Description: `Path to CSPP solver executable (if not in PATH)` |
| `--mixedaccount` | string `name/branch` | empty | Account and branch (0 or 1) for mixed outputs and, under mixed buying, ticket commitment addresses. Description: `Account/branch used to derive CoinShuffle++ mixed outputs and voting rewards` |
| `--ticketsplitaccount` | string | the name part of `--mixedaccount` | Account whose `--mixedaccount` branch supplies mixed ticket split outputs. Description: `Account to derive fresh addresses from for mixed ticket splits; uses mixedaccount if unset` |
| `--changeaccount` | string | empty | Source account of `--mixchange` mixes; destination of mix change, mixed split change, and change of sends from the mixed account. Description: `Account to send change when ticket autobuying or mixing; required when mixing` |
| `--mixchange` | bool | false | Starts the account mixer loop. Description: `Use CoinShuffle++ to mix change account outputs into mix account` |
| `--mixsplitlimit` | int | 10 | Capacity of each split-point semaphore. Not validated. Description: `Maximum concurrent mixes for any change amount` |
| `--ticketbuyer.limit` | uint | 1 | Under mixing: number of concurrent single-ticket mixed purchases per block |
| `--ticketbuyer.votingaccount` | string | empty | Mandatory for the mixed ticket buyer |
| `--spvdisablerelaytx` | bool | false | Incompatible with mixing in SPV mode (§10, Q11); not validated against `--mixing` |
| `--debuglevel` subsystems | — | — | `MIXC` (mix client), `MIXP` (mixpool) |

Option names, types and descriptions are from `config.go:146`, `:170-179`, `:185-190`; defaults from `config.go:62`, `:67`, `:377-378`. The sample configuration file documents `mixing`, `mixedaccount`, `ticketsplitaccount`, `changeaccount` and `mixchange` (`sample-dcrwallet.conf:88-102`); `csppsolver` and `mixsplitlimit` are absent from it.

### 6.3 Per-account constraints

| Constraint | Enforced by |
| --- | --- |
| The mixed account, split account and change account must be BIP0044 accounts with address buffers (default, named, or imported-xpub accounts); the imported-keys account cannot generate addresses | `nextAddress` (`wallet/addresses.go:553-556`) at generation time; `mixedSplit` change silently substitutes account 0 for the imported-keys account |
| The mixed branch must be 0 or 1 | `--mixedaccount` parsing; JSON-RPC `purchaseticket`; gRPC `PurchaseTickets`/`RunTicketBuyer`; for gRPC `RunAccountMixer` only at generation time inside `nextAddress` |
| Mix change always goes to the internal branch (1) of the change account | `wallet/mixing.go:401-402`, `wallet/addresses.go:812` |
| `MixAccount` only selects outputs owned by the change account; `MixOutput` accepts any outpoint of a transaction the wallet knows | `findEligibleOutputsAmount`; `MixOutput` step 7 |
| Mixed buying from the mixed account itself never creates the VSP-fee UTXO split | `wallet/wallet.go:1618-1620` |
| Mixed buying: commitment address from the mixed account/branch; voting address from the voting account (the mixed account for JSON-RPC and for gRPC without `use_voting_account`) | `wallet/createtx.go:1526-1535`; §5.16.4; §5.17.2 |
| Sends from the mixed account return change to the change account (JSON-RPC only) | §5.16.5 |
| Imported voting accounts cannot receive mixed split change | `wallet/addresses.go:800-803` |
| Keys must be available: the wallet, or the account owning each input when individually encrypted, must be unlocked when inputs are added and again when the coinjoin is signed | `Manager.PrivateKey` in §5.5, §5.7 |

---

## 7. Concurrency and lifecycle

### 7.1 Goroutines

| Goroutine | Started by | Ends when |
| --- | --- | --- |
| `Wallet.Run` → `mixclient.Client.Run` (which itself starts an epoch ticker and `runtime.NumCPU()` peer workers, spec 12) | SPV syncer group (`spv/sync.go:447-459`); dcrd-RPC syncer group (`chain/sync.go:730-734`) | `walletCtx` is cancelled and all running sessions have finished |
| One per selected output | `MixAccount` (`wallet/mixing.go:483-504`) | its `MixOutput` returns |
| `multiple` purchase goroutines and one change-mix goroutine per attached tip | `TB.Run` (`ticketbuyer/tb.go:198-211`) | their call returns |
| One per `WrapContext` call | `MixOutput`, `PurchaseTickets`, `TB.buy` | the returned cancel function is called or the syncer's `done` closes |
| One per received `inv` containing mix vectors; one per `getdata` | `spv.Syncer.receiveInv`, `receiveGetData` | handler returns |
| One per received wire message | `p2p.RemotePeer.readMessages` (`p2p/peering.go:794-852`) | handler returns |

### 7.2 Locks

| Lock | Protects | Notes |
| --- | --- | --- |
| `Wallet.lockedOutpointMu` | `lockedOutpoints` | Taken before any database transaction it spans. Held across the selection read transaction in `MixOutput`, `MixAccount`, `mixedSplit` |
| `Wallet.addressBuffersMu` | address cursors | Held for the whole `nextAddress` call, including the `LoadTxFilter` network call in the gap-ignore path |
| `Wallet.networkBackendMu` | `networkBackend` | Read-and-release in `SubmitMixMessage`, `mixingWallet.PublishTransaction` |
| `Wallet.mixClient` | — | Atomic pointer; no lock |
| `mixSems.splitSems[i]` | concurrency per split point | Non-blocking acquire; released when `MixOutput` returns |
| `spv.Syncer.remotesMu` | peer map | Held while enqueuing the `inv` on every peer in `PublishMixMessages` (each enqueue may wait up to 30 s) |
| `p2p.RemotePeer.requestedMixMsgsMu` | pending mix requests | |
| `p2p.blake256Mu`, `chain.Syncer.blake256HasherMu` | shared BLAKE-256 hasher instances | Hashers are reused; `WriteHash` resets them |

The address derivation in `MixOutput` step 14 and in `gen` opens its database write transaction after `nextAddress` returns, never while holding `addressBuffersMu`.

### 7.3 Lifecycle

1. `wallet.Open` creates the mixpool (when enabled) and the semaphores. No client exists yet; `mixedSplit` (whose caller supplies the backend) fails at `dicemix` with `mixing client is not running`, and `MixOutput` fails earlier at `NetworkBackend()` with `NoPeers` while no syncer has set a backend, reaching the `dicemix` failure only in the window between the syncer setting the backend and `wallet.Run` storing the client.
2. A syncer starts and calls `wallet.Run`. A new client is created and stored. `mixclient.NewClient` starts the solver subprocess if it is not already running.
3. Warm-up: `Dicemix` blocks until the client has passed its first epoch boundary plus one stage timeout plus 2 s (`dcrd/mixing@v0.7.5/mixclient/client.go:877-907`, `:1007-1011`): up to about 10 min 32 s on mainnet, 3 min 12 s on testnet3. Requests arriving earlier wait (they have already locked their outpoints, taken their semaphore slot and taken their change address; the mixed-output addresses are generated only after the warm-up and the pre-publication delay, §5.4).
4. Steady state: each request waits for the pre-publication delay, publishes its pair request, and completes at an epoch boundary when at least `MinPeers` compatible pair requests exist (spec 12).
5. Tip changes: `ChainSwitch` calls `expireMixMessages(newTipHeight)` after the database update and before the tip-changed notification is delivered (`wallet/chainntfns.go:310-313`). Expired pair requests fail their waiting `Dicemix` call at the next epoch tick.
6. Syncer stop: `walletCtx` is cancelled; the client stops accepting new sessions, finishes running ones, and `Run` returns `mixclient.ErrStopping`. The syncer's `Run` does not return, and the network backend is not cleared, until then. Pending `Dicemix` callers whose pairing never formed receive `ErrStopping`; callers inside `MixOutput`/`PurchaseTickets` are also cancelled through the wrapped context when the syncer's `done` channel closes.
7. A later syncer start repeats from step 2 with a new client and the same mixpool.

In SPV mode the handler group's context is not derived from the caller's context (`spv/sync.go:374-375`); shutdown of the handlers is triggered by the first group goroutine to return an error, which after cancellation is the initial-sync goroutine or the `wallet.Run` goroutine. `Syncer.Run` can therefore return `mixing client is stopping` instead of a context error; the process-level loop treats any error as a clean stop when its own context is done (`dcrwallet.go:543-547`).

### 7.4 Ordering guarantees

- `MixOutput` locks its outpoint before any address is generated and before any network activity. `mixedSplit` derives its change address (inside `txauthor.NewUnsignedTransaction`) before inserting the selected inputs into `lockedOutpoints`, both within one hold of `lockedOutpointMu` (`wallet/createtx.go:1031-1058`); its mixed-output addresses are generated after the inputs are locked. Address generation past the gap limit can itself call `LoadTxFilter` on the backend (§5.4).
- Change and mixed-output address records are committed before the pair request is published (`MixOutput`). For `mixedSplit` the change address record is committed only after success.
- The finished coinjoin is recorded in the wallet before it is sent to the network; if sending fails it is abandoned (`wallet/wallet.go:5148-5157`).
- `MixOutput` returns success only after the coinjoin was recorded and published.
- Mixed tickets are published in the order of their sorted trickle times, never before `now + 20 s`.

---

## 8. Errors

### 8.1 Errors produced in scope

| Condition | Error (kind, text) | Origin |
| --- | --- | --- |
| Mixing request on a wallet opened without mixing | `Invalid`, `wallet mixing support is disabled`, op `wallet.MixOutput(<op>)`, `wallet.MixAccount` or `wallet.PurchaseTickets` | `wallet/mixing.go:262-265`, `:454-457`; `wallet/wallet.go:1591-1594` |
| No network backend | `NoPeers` (`Decred network is unreachable`), op `wallet.NetworkBackend` | `wallet/network.go:61-71` |
| Output already reserved | `output <hash>:<index> already locked` | `wallet/mixing.go:282-283` |
| Output index beyond the transaction | `Invalid`, `index out of range: index:<i> txouts:<n>` | `wallet/mixing.go:296-298` |
| Transaction unknown to the wallet | `NotExist` | `wallet/udb/txquery.go:193` |
| No denomination fits | `output <hash>:<index> (<amount>): no suitable split denomination` | `wallet/mixing.go:385-386` |
| Semaphore full | `throttled mix request for split denomination` (no op) | `wallet/mixing.go:394` |
| Context ended at the semaphore | `context canceled` / `context deadline exceeded` wrapped with op | `wallet/mixing.go:390` |
| Input script not single-address P2PKH | `Invalid`, `previous output is not P2PKH` | `wallet/mixing.go:133`, `:137`, `:187`, `:191` |
| Key unavailable | `Locked`, `WatchingOnly` or `NotExist` from the address manager | `wallet/mixing.go:139`, `:198` |
| Signing failure | op `sign.SignatureScript` | `wallet/mixing.go:148` |
| Client not started | `Invalid`, `mixing client is not running` | `wallet/mixing.go:594` |
| Address not HASH160-capable | `address does not have Hash160 method` | `wallet/mixing.go:67` |
| Unknown account / bad branch / exhausted branch | `NotExist` `account <n>`; `Invalid` `branch must be external (0) or internal (1)`; `account <n> branch <b> exhausted` | `wallet/addresses.go:555`, `:565`, `:608` |
| Mixed buy from the mixed account needing a fee split | `InsufficientBalance`, op `wallet.PurchaseTickets` | `wallet/wallet.go:1618-1620` |
| Offline backend | `operation not supported in offline mode` | `wallet/network.go:92`, `:110-112` |
| SPV: no peers / no mix-capable peers | `NoPeers`; `Protocol`, `no connected peers support the mixing protocol version`; op `spv.PublishMixMessages` | `spv/backend.go:512-518` |
| SPV: unrequested mix message | `Protocol`, `received unrequested mix msg` (peer disconnected) | `p2p/peering.go:1290-1294` |
| SPV: duplicate concurrent request | `mix msg <hash> is already being requested from this peer` | `p2p/peering.go:1662` |
| SPV: stalled peer | `IO`, `peer appears stalled` (peer disconnected) | `p2p/peering.go:1690-1693` |
| SPV: mix or transaction inventory while relay is disabled | `Protocol`, `received tx in msginv when tx relaying is disabled` (peer disconnected) | `p2p/peering.go:2231-2236` |
| Peer protocol version below 10 on the unused per-peer publish helper | `Protocol`, `protocol version <v> is too low to publish mix messages` | `p2p/peering.go:2104-2109` |
| dcrd RPC: undecodable notification / unknown command | `Encoding`; `unrecognized mixing message command string`; `unrecognized mixing message type` | `rpc/client/dcrd/notifications.go:86-111`, `calls.go:298-300` |
| dcrd RPC: publication | op `dcrd.PublishMixMessages` wrapping the dcrd error (a `*wsrpc.Error` carrying dcrd's code and message) | `rpc/client/dcrd/calls.go:254-268` |
| JSON-RPC: mixing off | `Mixing is not configured` | `internal/rpc/jsonrpc/methods.go:5616`, `:5651` |
| JSON-RPC: wallet not loaded | `request requires a wallet but wallet has not loaded yet` (`ErrRPCWallet`) | `internal/rpc/jsonrpc/errors.go:67-70` |
| JSON-RPC: `--mixedaccount`/`--changeaccount` name unknown in `mixaccount`/`mixoutput` | `account not found` (`ErrRPCWalletInvalidAccountName`) | `internal/rpc/jsonrpc/errors.go:82-85`; `methods.go:5628-5641`, `:5658-5671` |
| JSON-RPC `purchaseticket` under mixing | `ErrRPCInvalidParameter`: `Mixing enabled, but error on mixed account: <err>`; `MixedAccountBranch should be 0 or 1.`; `Mixing enabled, but error on mixedSplitAccount: <err>`; `Mixing enabled, but error on changeAccount: <err>` | `internal/rpc/jsonrpc/methods.go:3411-3431` |
| JSON-RPC `getcoinjoinsbyacct` | `ErrRPCWalletPassphraseIncorrect`, `incorrect passphrase` (kind `Passphrase`); `ErrRPCInternal` wrapping a `NotExist` account-name lookup | `internal/rpc/jsonrpc/methods.go:5843-5856` |
| gRPC `PurchaseTickets`/`RunTicketBuyer` under mixing | `InvalidArgument`: `Mixing requested, but error on mixed account: <err>`; `MixedAccountBranch should be 0 or 1.`; `Mixing requested, but error on mixedSplitAccount: <err>`; `Mixing requested, but error on changeAccount: <err>` | `internal/rpc/rpcserver/server.go:1843-1865`, `:2621-2644` |
| gRPC stream end | `Canceled`: `AccountMixer instance canceled, account number: <n>` / `TicketBuyer instance canceled, account number: <n>`; `Unknown`: `AccountMixer instance errored: <err>` / `TicketBuyer instance errored: <err>` | `internal/rpc/rpcserver/server.go:2559-2562`, `:2679-2682` |
| gRPC service not started | `FailedPrecondition`, `service walletrpc.AccountMixerService is not ready`; `StartAccountMixerService` twice panics with `service already started` | `internal/rpc/rpcserver/server.go:317`, `:2522-2524` |
| Ticket buyer loop (log only) | `Calculating sdiff failed: <err>`; `Ticket purchasing failed: <err>`; `Account mixing failed: <err>` (all error level) | `ticketbuyer/tb.go:156`, `:188`, `:208` |
| Start-up account lookups (log + abort) | `<flag>: account "<name>" does not exist`; `Failed to get account number for ticket change account "<name>": <err>`; `Incorrect passphrase in pass config setting.` | `dcrwallet.go:245`, `:261-263`, `:336` |
| SPV: too many inventory vectors | `Protocol`, op `spv.PublishMixMessages`, wrapping the wire `AddInvVect` error | `spv/backend.go:490-493` |
| dcrd RPC: message encode failure | `Encoding`, op `dcrd.PublishMixMessages` | `rpc/client/dcrd/calls.go:245-248` |
| dcrd RPC: `getmixpairrequests` element not a string / undecodable | `Encoding`, `not a string`; `Encoding` wrapping the decode error; op `dcrd.MixPairRequests` | `rpc/client/dcrd/json.go:311-318`, `calls.go:313-316` |
| dcrd-RPC mode: `notifymixmessages` rejected | the RPC error, returned unwrapped from the syncer's `Run` (ends the sync session) | `chain/sync.go:738-741` |
| JSON-RPC: outpoint syntax | `parseOutpoint: bad len`, `parseOutpoint: bad separator`, or the wrapped hash/integer parse error | `internal/rpc/jsonrpc/methods.go:5679-5696` |
| Config | `--mixedaccount must have form 'accountname/branch'`; `--mixedaccount branch must be 0 or 1`; `csppsolver process is not operating properly: <err>`; `cannot run mixed ticketbuyer without --votingaccount` | `config.go:753`, `:763`, `:774`; `dcrwallet.go:311` |

Errors from the mix client and mixpool pass through unchanged apart from the op wrapper: `mixing client is stopping`, `mixing pair request <hash> by <id> expired`, `not enough peers required to mix`, `unable to participate in reformed session referencing unknown PRs`, `gen returned wrong message count`, and mixpool rule errors such as `change output is dust`, `mix output is dust`, `not enough input value, or too low fee`, `too high fee`, `message has expired`, `expiry is too far into future`, `message count <n> exceeds max 16` (`dcrd/mixing@v0.7.5/mixclient/errors.go`, `mixclient/client.go:57-65`, `mixpool/errors.go`, `mixpool/limits.go`).

### 8.2 Surfacing

| Surface | Mapping |
| --- | --- |
| JSON-RPC | The error string is the wallet error's `Error()` (`errors/errors.go:247-290`): within one `*errors.Error` the op, the kind string and a non-`*Error` cause are joined by `: `; each step into a nested `*errors.Error` is joined by `errors.Separator`, which the dcrwallet process sets to `:: ` at init (`dcrwallet.go:36-39`; the package default is `":\n\t"`, `errors/errors.go:27`). An op or kind equal to the one last written is not repeated, and `errors.E` collapses a wrapped `*Error` that has no op or kind differing from the wrapper (`errors/errors.go:213-233`). Kind strings are those of `errors/errors.go:77-124` (for example `Invalid` → `invalid operation`, `NoPeers` → `Decred network is unreachable`, `Locked` → `wallet locked`, `InsufficientBalance` → `insufficient balance`). The code is chosen by the outermost non-zero kind in the error chain (`errors/errors.go:311-323`): `Bug` → internal error, `Encoding` → invalid parameter, `Locked` → wallet unlock needed, `Passphrase` → passphrase incorrect, `NoPeers` → client not connected, `InsufficientBalance` → insufficient funds; everything else, including `Invalid` and plain errors, → `ErrRPCWallet` (-4). A `*wsrpc.Error` from dcrd keeps dcrd's code and message only when it is the returned error itself, not when wrapped (`internal/rpc/jsonrpc/errors.go:16-49`) |
| gRPC `RunAccountMixer`, `RunTicketBuyer` | Only the final loop error is surfaced (`Canceled` or `Unknown`); per-block failures are logged |
| gRPC unary | `translateError`: `Invalid` → `InvalidArgument`, `NotExist` → `NotFound`, `Locked` → `FailedPrecondition`, `InsufficientBalance` → `ResourceExhausted`, `NoPeers` → `Unavailable`, cancellation → `Canceled` (`internal/rpc/rpcserver/server.go:80-125`). `PurchaseTickets` returns the wallet error untranslated (`internal/rpc/rpcserver/server.go:1908-1911`) |
| Logs (subsystem) | `WLLT`: `Mixing output …`, `Completed CoinShuffle++ mix …`, `Selected input …`, `Failed to publish mix transaction: …`, `Returning address …`, debug lines of §5.2. `TKBY`: `Account mixing failed: …`, `Ticket purchasing failed: …`. `SYNC`: SPV/RPC warnings and `mixmessage: …` errors. `PEER`: per-message debug lines of the wire peer. `MIXC`: mix client debug/error lines. `MIXP`: mixpool accept/reject debug lines. `DCRW`: start-up warnings and errors of §5.18 |
| Exit | Configuration errors of §5.18 abort start-up through the standard configuration-error path; account lookup failures return the error from the main run function |

---

## 9. Compatibility-critical behaviours

Each item names what depends on it.

1. **Split-point denominations.** The ten values of §6.1, exactly. Pairing with other wallets requires an identical mix amount; a different table produces pair requests no one else pairs with. (Other mixing participants.)
2. **Denomination selection.** Largest split point strictly below the next ticket price that fits the output (the smallest split point is exempt from the ticket-price test), at most four mixed outputs, the count reduced until the fee is payable. Changing this moves the wallet into different pairing pools and changes observable transaction shapes. (Other participants; chain analysis parity.)
3. **Mixed ticket split denomination.** `ticketPrice + fee(relayFee, 298)`. Ticket buyers pair only when this is identical, so the 298-byte ticket size estimate, the fee function (truncating division, minimum of one fee-rate unit) and the next-stake-difficulty calculation must match dcrwallet bit for bit. (Other dcrwallet and Decrediton ticket buyers.)
4. **Fee and change sizing.** The size estimate of §5 and the rule that change below `smallestMixChange(relayFee)` is dropped and that no change exists at the smallest split point. The resulting pair request must pass every full node's mixpool: fee ≥ `10000 × size / 1000` and ≤ 10000 times that, change ≥ 6030 atoms. (dcrd mixpool acceptance; peers may ban on `ErrLowInput`, `ErrHighFee`, `ErrChangeDust`.)
5. **Pair request expiry.** `MaxExpiry(tip) - 2`. A larger value is rejected by nodes whose tip equals or trails the wallet's; a smaller one shortens the mixing window. (dcrd mixpool acceptance.)
6. **Mixed output and change scripts.** Version-0 P2PKH scripts of fresh BIP0044 children: mixed outputs from the mixed (or split) account at the configured branch; change from branch 1 of the change account. (Existing `wallet.db`: the outputs are recognised only through the address records; Decrediton account views.)
7. **Address records written before the mix.** `SyncAccountToAddrIndex` for each generated child, in its own committed transaction, before the pair request is published (after success for mixed split change). A Go dcrwallet opening the same database later must find these records. (Existing `wallet.db`.)
8. **Address consumption.** `mcount` addresses per `gen` call, a new set per blame rerun, gap limit ignored, cursor never rolled back, "last returned" index not advanced. Observable through `accountaddressindex`/`NextAddress` results, the next address handed to a user, and the addresses added to the dcrd transaction filter (`loadtxfilter`) in batches of the gap limit. (JSON-RPC and gRPC clients; dcrd.)
9. **Outpoint reservation.** Inputs of a pending mix appear in the locked-outpoint set (`listlockunspent`, gRPC equivalents) from request start until the request returns or the coinjoin is recorded; `lockunspent` reset clears them. (dcrctl, Decrediton.)
10. **dcrd JSON-RPC usage.** `notifymixmessages` once per session and only when mixing is enabled; `sendrawmixmessage [command, hex]` per message with the payload encoded at protocol version 10; `getmixmessage` for a key exchange's missing own pair request only when the key exchange has run 0; messages hashed before pool acceptance. (dcrd, dcroxide.)
11. **SPV inventory behaviour.** Immediate (untrickled) `inv` of type 4 to every peer at version ≥ 10; `getdata` served only for hashes previously announced to that peer (including the own pair request hash recorded when a key exchange is announced), `notfound` for everything else; announced mix messages fetched with one `getdata` per `inv`; orphan key exchanges trigger a fetch of the missing pair request from the same peer; an unrequested mix message disconnects the peer. (dcrd and dcroxide peers, including their ban scoring.)
12. **Advertised protocol version 11 and the minimum-version peer rule** (§5.14.5). (Peers.)
13. **Pair request UTXO descriptors.** Outpoint including tree as stored for the credit, 33-byte compressed public key, ownership proof over the expiry, opcode `OP_SSGEN`/`OP_SSRTX`/`OP_TGEN` or 0 from the first script byte, no redeem script. (dcrd mixpool UTXO validation.)
14. **Coinjoin recording and publication.** Recorded as an unmined wallet transaction, then published; abandoned if publication fails; a duplicate-transaction reply (`-40`) from dcrd is success. For mixed splits the same transaction is recorded and published a second time by the ticket purchase code. (Existing `wallet.db`; dcrd.)
15. **Mixed ticket trickle.** Each mixed ticket is published at an independent uniformly random time 20–60 s after the split, in ascending order. (Chain analysis parity; client timing expectations.)
16. **JSON-RPC surface.** `mixaccount` and `mixoutput` block until completion, return `null`, and fail with the exact strings of §8; `getcoinjoinsbyacct` returns name → output count; `purchaseticket` under `--mixing` votes from the mixed account and takes change/split accounts from the process options; sends from the mixed account return change to the change account. (dcrctl, scripts.)
17. **gRPC surface.** `RunAccountMixer` never sends a response message, validates nothing, and ends only with `Canceled` (`AccountMixer instance canceled, account number: <n>`) or `Unknown` (`AccountMixer instance errored: <err>`); `RunTicketBuyer` with `enable_mixing` always mixes change; the `InvalidArgument` strings of §5.17.2; `GetCoinjoinOutputspByAcct` field semantics. (Decrediton.)
18. **Option names, defaults and failure strings** of §6.2 and §5.18, including that `--csppsolver` set explicitly makes a non-working solver fatal while an absent default solver is only a warning. (Existing `dcrwallet.conf` files, launch scripts, Decrediton's launcher.)
19. **`PossibleCoinJoin`** exactly as in §5.11, since its output feeds `getcoinjoinsbyacct`/`GetCoinjoinOutputspByAcct`, which Decrediton uses to guess the mixed account on restore. (Decrediton.)
20. **Throttling.** Per-denomination, non-blocking; the throttled error is returned to direct callers and swallowed by `MixAccount`. (JSON-RPC `mixoutput` callers.)
21. **Eligibility.** `MixAccount` mixes only outputs of the change account with ≥ 2 confirmations and ≥ 262144 atoms, at most `10 × mixsplitlimit` per call; mixed ticket buying uses ≥ 2 confirmations. (Wallet behaviour visible on chain.)
22. **Lifecycle coupling.** No mixing before a syncer has started; mixes wait for the client warm-up; syncer shutdown waits for running sessions. (Process shutdown timing; client expectations.)
23. **Log lines** at info level (`Mixing output …`, `Completed CoinShuffle++ mix of output … in transaction …`, `Completed CoinShuffle++ mix of ticket split transaction …`, `Selected input … for ticket purchase split transaction`, `Starting auto transaction creator`), if log compatibility is in the project's parity definition. (Log scrapers, Decrediton log view.)
24. **gRPC change-account defaults.** `PurchaseTickets` and `RunTicketBuyer` copy `change_account` into the request even without mixing, so an unset field means account 0 for split change (and, for `PurchaseTickets`, for VSP fee change); `RunTicketBuyer` never substitutes the mixed account for an unset `voting_account`. Clients written against dcrwallet set these fields explicitly and depend on them being honoured as given. (Decrediton, other gRPC clients.)
25. **Mixed ticket buyer shape.** Under mixing each tip starts `limit` independent requests of exactly one ticket each, with `minconf` 2, each its own pair request with `mcount` 1 at the ticket denomination; a single request for `limit` tickets would produce a different pair request (different `mcount` and input set) and therefore different on-chain shapes and pairing behaviour. (Other mixing participants; chain analysis parity.)
26. **Proto field numbers and JSON-RPC parameter shape** of §4.7, including the misspelled method and message names. (Decrediton, dcrctl.)
27. **Help strings** of §5.16.6, if `help` output parity is in the project's parity definition (upstream pins it with `TestRPCMethodHelpGeneration`). (dcrctl users, scripts.)
28. **Readiness and stream semantics of `AccountMixerService`.** `FailedPrecondition` until a wallet is loaded; afterwards any number of concurrent `RunAccountMixer` streams may run, each with its own loop (no single-instance guard exists), and each ends only by client cancellation or a passphrase error. (Decrediton.)

---

## 10. Quirks

- **Q1 — Newly created wallets cannot mix outputs until restart.** `Loader.CreateNewWallet` builds its `wallet.Config` without `MixSplitLimit` (`internal/loader/loader.go:259-274`), unlike `CreateWatchingOnlyWallet` and `OpenExistingWallet` (`:169-185`, `:318-334`). The semaphores are then unbuffered: every `MixOutput` returns `errThrottledMixRequest` and `MixAccount` selects every eligible output (`maxResults = 0`) and mixes none. Mixed ticket splits are unaffected (they do not use the semaphores). A wallet created over gRPC and used in the same process is in this state.
- **Q2 — `--mixsplitlimit` is not validated.** 0 gives the Q1 behaviour; a negative value panics in `make(chan struct{}, n)` when the wallet is opened (`wallet/mixing.go:240-246`).
- **Q3 — The mixed split transaction is recorded and published twice, and may be returned partially signed.** The mix client records and publishes the coinjoin (`wallet/mixing.go:157-169`); `purchaseTickets` then records and publishes `cj.Tx()` again (`wallet/createtx.go:1460-1482`). When several local requests of this wallet take part in the same session — the normal case for a mixed ticket buyer with `limit > 1` — only the first local peer's `CoinJoin` receives the other participants' signatures (`dcrd/mixing@v0.7.5/mixclient/client.go:2100-2137`); every other local request's `cj.Tx()` carries signature scripts only on its own inputs. That copy is what `mixedSplit` returns, what is passed to `PublishTransactions`, and what `PurchaseTicketsResponse.SplitTx` exposes to gRPC and JSON-RPC (`dontsigntx`) clients. It is harmless on the network because the hash is identical: the second insert finds the existing unmined record (`wallet/udb/txunmined.go:31-34`), dcrd answers duplicate (`-40`, treated as success, `rpc/client/dcrd/calls.go:207-213`), and in SPV mode `getdata` is answered from the stored copy. UNVERIFIED: dcrd's reply when the coinjoin has already been mined before the second publication (a non-duplicate rejection would fail the purchase); settle by testing against dcrd.
- **Q4 — `DontSignTx` does not stop a mixed split.** `mixedSplit` ignores it (`wallet/createtx.go:999`); the coinjoin is signed, recorded and published by the client, then unsigned tickets are returned.
- **Q5 — A `notfound` for a mix message disconnects the peer.** `receivedNotFound` consults only the pending transaction requests (`p2p/peering.go:1194-1217`); a `notfound` vector for a mix hash requested through `MixMessages` is "unrequested" and the peer is disconnected with `received notfound for unrequested hash`. The waiting `MixMessages` call then returns the peer's error. The `NotExist` handling in `handleMixInvs` (`spv/sync.go:1199-1210`) and the `ErrNotFound` return of `MixMessages` (`p2p/peering.go:1703-1705`) are dead code. Full nodes do answer `notfound` for mix messages they have dropped, so this path is live. `ErrNotFound`'s text is `transaction not found` even for mix messages.
- **Q6 — Unused surface.** `RemotePeer.PublishMixMessages` (trickled inventory, `p2p/peering.go:2101-2121`), `RPC.MixPairRequests` (`rpc/client/dcrd/calls.go:309-318`) and the deprecated `Wallet.AcceptMixMessage` have no in-tree callers. No initial mixpool synchronisation exists in either mode: the wallet learns only of messages announced after it connected, plus pair requests fetched for orphan key exchanges.
- **Q7 — Mixed ticket buyer with limit 0 buys nothing.** Under mixing, `multiple = cfg.Limit` (`ticketbuyer/tb.go:167-171`, `:198`), so `--ticketbuyer.limit=0` (documented as "0 for no limit") and a gRPC `RunTicketBuyer` with `limit = 0` start zero purchase goroutines. The `limit == 0 && mixing` branch in `buy` (`:282-283`) is unreachable from `Run`.
- **Q8 — `--mixchange` without `--mixing`.** The loop is started and requires `--mixedaccount` and `--changeaccount` to resolve, but `mixChange` returns immediately (`ticketbuyer/tb.go:332-334`).
- **Q9 — `--mixchange` alone does not obtain the passphrase.** Only `--enableticketbuyer`, `--promptpass`, `--pass` or a needed accounts sync do (`dcrwallet.go:241-250`, `:468-498`). With a locked wallet every block logs `Account mixing failed: …` (kind `Locked`) until the wallet is unlocked over RPC.
- **Q10 — `RunAccountMixer` validates nothing.** Unknown accounts, a branch other than 0/1, or a wallet opened without mixing produce a per-block error log and an open, silent stream (`internal/rpc/rpcserver/server.go:2531-2566`).
- **Q11 — `--spvdisablerelaytx` breaks SPV mixing.** With relay disabled the local peer disconnects any peer whose `inv` contains a mix vector (`p2p/peering.go:2230-2236`), and the version message asks peers not to relay. No configuration check rejects the combination with `--mixing`.
- **Q12 — Help text for a parameter that does not exist.** The help table carries `mixaccount-account` ("Account to mix") although `mixaccount` takes no parameters (`internal/rpchelp/helpdescs_en_US.go:678-679`, `rpc/jsonrpc/types/methods.go:1103`).
- **Q13 — `PossibleCoinJoin` details.** Duplicate-script rejection applies to zero-valued outputs too (the check precedes the zero-value skip); `mixDenom`/`mixCount` are returned non-zero with `isMix == false` when the half-of-outputs test fails; the function is a heuristic with false positives by its own comment (`wallet/mixing.go:514-557`).
- **Q14 — Misspelled API names are the API.** `GetCoinjoinTxsSumbByAcct`, gRPC `GetCoinjoinOutputspByAcct`, message `coinjoinTxsSumByAcct` (lower-case first letter in the proto, `rpc/api.proto:1203`), JSON-RPC `getcoinjoinsbyacct` (described in a comment as `getcoinjoinsbyaccount`). The gRPC response order is Go map iteration order.
- **Q15 — `MixOutput` trusts its caller.** It does not check that the output is unspent, mature, confirmed or owned; JSON-RPC `mixoutput` always supplies tree 0, so a stake-tree output named over RPC yields a pair request and coinjoin input with the wrong tree. UNVERIFIED: the exact dcrd rejection for the wrong tree; settle by submitting such a request to dcrd.
- **Q16 — Stake-change outputs are selected but cannot be proven.** `findEligibleOutputsAmount` admits matured `OP_SSTXCHANGE` P2PKH outputs (`wallet/createtx.go:1883-1886`) and `addCoinJoinInput` accepts them, but `CoinJoin.AddInput` assigns opcode 0 to them (`dcrd/mixing@v0.7.5/mixclient/coinjoin.go:99-103`), and a mixpool with a UTXO fetcher extracts the key hash with the plain P2PKH extractor for opcode 0 (`dcrd/mixing@v0.7.5/mixpool/mixpool.go:1485-1502`), which fails the ownership proof. UNVERIFIED end to end (derived from the pinned mixpool source, not from a running dcrd); settle with an integration test mixing a non-zero stake-change output of the change account.
- **Q17 — P2SH stake outputs pass selection.** The class filter admits P2SH stake generation/revocation/change and treasury classes; if such an output's address resolves to the change account, `MixOutput` fails with `previous output is not P2PKH`, which `MixAccount` does not suppress and returns as its error.
- **Q18 — Integer conversions.** `uint32(amount / mixValue)` truncates for quotients ≥ 2^32 (`wallet/mixing.go:342`); `uint32(height)` conversions assume non-negative heights; `uint32(req.Count)` is passed as `mcount` unchecked, so counts above 16 fail at pool acceptance rather than earlier.
- **Q19 — Nondeterminism from the Go runtime.** Random choice between a done context and a free semaphore slot (`wallet/mixing.go:388-395`); which error `MixAccount` returns when several outputs fail (first to finish); peer iteration order in `PublishMixMessages` and therefore which peer's send error is reported; order of missing pair request hashes requested (`spv/sync.go:1253-1256`); UTXO selection randomness (CSPRNG).
- **Q20 — Mix addresses are not durable as "returned".** See §5.4: after a restart, addresses generated for mixes that never reached the chain are generated again; within a process the cursor advances even when generation later fails or the mix is abandoned (`wallet/addresses.go:449-452`).
- **Q21 — Filter loading in the gap-ignore path depends on a backend being present at the exact call.** If no backend is set when the cursor crosses a multiple of the gap limit, that batch is not loaded then (`wallet/addresses.go:584-590`).
- **Q22 — Nil backend passed to `Wallet.PublishTransaction`.** `mixingWallet.PublishTransaction` passes the current backend without a nil check (`wallet/mixing.go:160-164`); a nil interface would panic at `n.PublishTransactions`. The SPV syncer clears the backend only after `wallet.Run` has returned (`spv/sync.go:396-397`, `:462`). The dcrd-RPC syncer does the same on its normal exit (`chain/sync.go:610-611`, `:757`), but when `notifymixmessages` fails it returns without waiting for the `wallet.Run` goroutine (`chain/sync.go:738-741`), clearing the backend while the just-created client is still stopping; that client has not warmed up, so no coinjoin can be published in that window. The nil case is therefore unreachable in the in-tree wiring.
- **Q23 — Stale client pointer.** `Wallet.mixClient` keeps the stopped client between syncer runs. A `Dicemix` on it either fails at publication (`NoPeers`) or, if that client never warmed up, blocks until the caller's context ends. `MixOutput` normally fails earlier at `NetworkBackend()`.
- **Q24 — `SubmitMixMessage` cleanup removes on every failure**, including when the pool reported the message as a duplicate and only the network publication failed (`wallet/mixing.go:99-118`).
- **Q25 — Unwrapped errors.** `MixOutput` returns the `NetworkBackend` error, the change-address database error and the throttle error without its op; everything else is wrapped.
- **Q26 — `--changeaccount` is described as "required when mixing" but is not checked at configuration time.** With `--mixing` and no `--changeaccount`, JSON-RPC `mixaccount`/`mixoutput` fail per call with `account not found`; `purchaseticket` fails with `Mixing enabled, but error on changeAccount: …`.
- **Q27 — `sendtotreasury` under `--mixing` without `--mixedaccount` always fails**, because `sendAmountToTreasury` resolves the empty mixed account name unconditionally (`internal/rpc/jsonrpc/methods.go:3630-3634`), while `sendPairs` guards on non-empty names.
- **Q28 — The change mix is not cancelled at stake-difficulty window boundaries**; purchases are. The change mix uses the loop context (`ticketbuyer/tb.go:201-202`), and is skipped entirely on the one tip where `height + 2 == nextIntervalStart`.
- **Q29 — `expireMixMessages` does not expire anything immediately**; it records the height for the client's next epoch tick (`dcrd/mixing@v0.7.5/mixclient/client.go:1097-1106`).
- **Q30 — Epoch selection is by network identity**: 3 minutes only for testnet3; simnet and regnet use the 10-minute mainnet epoch (`dcrd/mixing@v0.7.5/mixpool/mixpool.go:280-292`), so integration tests on simnet need at least one 10-minute boundary per mix unless the client's test hooks are used.
- **Q31 — Terminology.** Logs, options and comments say "CSPP"/"CoinShuffle++"; the protocol implemented by the pinned mix client is the peer-to-peer one of spec 12. The strings are observable and stay as they are.
- **Q32 — `mixingWallet.BestBlock` and `mixpoolBlockchain.CurrentTip` ignore read errors**: `MainChainTip` discards the view error and returns zero values (`wallet/wallet.go:792-802`).
- **Q33 — gRPC documentation disagrees with the proto and the code.** `rpc/documentation/api.md:1769` names the `PurchaseTicketsRequest` field `mixing_enabled`; the proto field is `enable_mixing` (`rpc/api.proto:636`). The `RunTicketBuyerRequest` documentation (`api.md:2778-2796`) omits `vsp_host`, `vsp_pubkey`, `limit` and all five mixing fields. The `AccountMixerService` section describes the service as providing "the ability to launch the ticket buyer" and lists only `FailedPrecondition` as an expected error (`api.md:2798-2827`), while the handler also returns `Canceled`, `Unknown` and translated unlock errors. `GetCoinjoinOutputspByAcct` is not documented at all. The proto and the code win.
- **Q34 — gRPC change account defaults to account 0 without mixing.** See §5.17.2 item 2 and §5.17.3 item 4 (`internal/rpc/rpcserver/server.go:1813`, `:1841`, `:1881`, `:2618`, `:2656`). JSON-RPC `purchaseticket` defaults to the purchasing account (`internal/rpc/jsonrpc/methods.go:3407-3409`).
- **Q35 — `RunTicketBuyer` does not default the voting account to the mixed account**, unlike `PurchaseTickets` and `purchaseticket` (`internal/rpc/rpcserver/server.go:2650` versus `:1888-1894`). The process-level ticket buyer instead refuses to start a mixed buyer without `--ticketbuyer.votingaccount` (`dcrwallet.go:310-314`). Three entry points, three rules.
- **Q36 — The start-up error names a flag that does not exist.** `cannot run mixed ticketbuyer without --votingaccount`; the option is `--ticketbuyer.votingaccount` (`dcrwallet.go:311`, `config.go:185-190`).
- **Q37 — Sample configuration describes `changeaccount` differently from the option.** `sample-dcrwallet.conf:98` says "Account used to derive unmixed CoinJoin outputs in CoinShuffle++ protocol."; the option description says "Account to send change when ticket autobuying or mixing; required when mixing" (`config.go:177`). The sample file's `mixing` comment reads "Enable the mixing client (required for any mixing)." versus the option's "Enable creation of mixed transactions and participation in the peer-to-peer mixing network".
- **Q38 — Doc comments that overstate.** `Wallet.MixingEnabled` is documented as reporting that the wallet "is running the mixing client" but returns only the configuration flag (`wallet/wallet.go:5532-5536`). `mixaccount`'s synopsis says "Mix all outputs of an account." although each call mixes at most `10 × mixsplitlimit` randomly chosen outputs and silently skips throttled ones (`internal/rpchelp/helpdescs_en_US.go:678`, §5.2). `MixOutput`'s comment says outputs are mixed into sizes "under the current ticket price", which does not hold for the smallest split point (§5.1 step 11.1). `RemotePeer.PublishMixMessages` is commented as advertising "transaction hashes of txs" (`p2p/peering.go:2099-2100`). `receiveGetData`'s comment says "Only transactions may be queried by a peer." although mix messages are served (`spv/sync.go:776-778`). The help-description block for `getcoinjoinsbyacct` is headed `// SetMixedAccountCmd help.` (`internal/rpchelp/helpdescs_en_US.go:811`).
- **Q39 — Commented-out logger hook.** `// XXX mixclient.UseLogger(loggers.MixcLog)` (`log.go:42`): the mix client logger is set per instance instead; a client created outside `newMixClient` would log nowhere.
- **Q40 — Cancelled `MixMessages` calls leave their registrations for up to 30 s.** See §5.14.3: a reply arriving after cancellation but before the stall timer fires is accepted silently; once the timer has fired the same reply disconnects the peer as unrequested (`p2p/peering.go:1678-1685`). The helper goroutine blocks on the timer channel for the full remaining time.
- **Q41 — A repeated hash in one `inv` aborts the whole fetch.** `handleMixInvs` does not deduplicate; `MixMessages` then fails with `… is already being requested from this peer` and none of the announced messages is fetched or marked seen (`spv/sync.go:1188-1198`, `p2p/peering.go:1657-1663`).
- **Q42 — `seenMixMsgs` can suppress the fetch of a missing own pair request.** The recursive call filters through the same cache, so a pair request hash already fetched once (and rejected or evicted) is not requested again and the key exchange stays orphaned (`spv/sync.go:1188-1196`, `:1252-1257`).
- **Q43 — `TB.buy` reads the VSP client from the live config without the mutex** (`ticketbuyer/tb.go:302`), while every other field comes from the per-tip copy; a concurrent `AccessConfig` writer races with it.
- **Q44 — No backend makes every tip log purchase failures.** `buy` returns the `NoPeers` error from `NetworkBackend()`, which is not in the silenced set, so `Ticket purchasing failed: wallet.NetworkBackend: Decred network is unreachable` is logged per purchase goroutine per tip; the change mix logs `Account mixing failed: …` for the same reason whenever at least one output is eligible, since `MixAccount` itself does not check the backend and each `MixOutput` returns the error (`ticketbuyer/tb.go:182-189`, `:224-227`). Tip notifications normally imply a backend, so this is seen mainly around syncer restarts.
- **Q45 — No single-instance guard on the account mixer.** Several `RunAccountMixer` streams, a `--mixchange` loop and JSON-RPC `mixaccount` calls can run concurrently against the same change account; they contend through the outpoint locks (`output … already locked`, logged as `Account mixing failed`) and the split semaphores (§5.13).
- **Q46 — The purchase-context slice grows in a pure account mixer.** One cancel function per tip is retained until the next stake-difficulty window boundary even when `BuyTickets` is false (`ticketbuyer/tb.go:173-174`, `:122-127`).
- **Q47 — `mixoutput` parses loosely.** `parseOutpoint` accepts any index text `strconv.ParseUint(…, 10, 32)` accepts (leading zeros allowed, sign rejected) and never sets the tree; the hash is parsed with `chainhash.NewHashFromStr` on exactly 64 characters (`internal/rpc/jsonrpc/methods.go:5679-5696`).
- **Q48 — `dcrd.RPC.MixMessage` returns a non-nil message with a decode error** (`rpc/client/dcrd/calls.go:303-304`); the only caller checks the error first (`chain/sync.go:954-962`). `dcrd.MixMessage` (notification decoder) does the same (`rpc/client/dcrd/notifications.go:113-114`).

---

## 11. Upstream tests

| File | What it pins | Port / regenerate |
| --- | --- | --- |
| `wallet/mixing_test.go` (`TestIsMixTx`) | `PossibleCoinJoin` on two vectors: mainnet transaction `ab70b9b3…78f3` (7 inputs, 18 outputs) → `(true, 268435456, 11)`; a coinbase → `(false, 0, 0)` | Port both hex vectors verbatim |
| `wallet/network_test.go` | `mockNetwork` implementing `PublishMixMessages` as a no-op | Port as the test double |
| `wallet/setup_test.go` | `basicWalletConfig` opens test wallets with `MixingEnabled: true` and `MixSplitLimit` zero | Mirror in the test harness |
| `dcrd/mixing@v0.7.5/mixclient/client_test.go` (`TestHonest`, `TestCTDisruption`, `TestCTLength`, `TestSRDisruption`, `TestDCDisruption`) | Client behaviour against a test wallet implementing `mixclient.Wallet`; shows the expected call pattern of the five callbacks | Owned by spec 12; reuse its test wallet as the model for the Rust trait |
| `dcrd/mixing@v0.7.5/mixpool/mixpool_test.go`, `orphans_test.go` | Pool acceptance and orphan handling | Owned by spec 12 / dcroxide-mixing |
| `internal/rpc/jsonrpc/rpchelp_test.go` (`TestRPCMethodHelpGeneration`, `TestRPCMethodUsageGeneration`) | That the checked-in help and usage strings (including the `mixaccount`, `mixoutput`, `getcoinjoinsbyacct` entries of §5.16.6) equal what the generator produces from `internal/rpchelp` | Port the three strings verbatim as golden values |
| `rpc/client/dcrd/json_test.go` | Hash, header and transaction array decoders only; the `messageArray` decoder used by `MixPairRequests` and the `mixmessage` notification decoder have no test | Add Rust tests from oracle output |
| `chain/semver_test.go` | Semver compatibility rule behind the `8.3.0` dcrd API requirement | Port |

Test files that exist in the scope's packages but contain nothing about mixing: `wallet/addresses_test.go` (`TestAddresses`, `TestAccountIndexes`: known-address derivation and account index handling through `NewInternalAddress`/`NewExternalAddress`, not the gap-ignore path mixing uses), `wallet/locking_test.go` (`TestLocking`: wallet passphrase unlock with nil and non-nil timeout channels and their replacement, which is the behaviour open question 9 depends on), `internal/rpc/jsonrpc/rpcserver_test.go` (`TestThrottle`). Packages with no test files at all: `ticketbuyer`, `spv`, `p2p`, `internal/rpc/rpcserver`, and the root package's `config.go`/`testsolver.go` mixing code. `wallet/mixing.go` other than `PossibleCoinJoin`, `mixedSplit`, `findEligibleOutputsAmount` and every network path of §5.14–§5.15 are untested upstream.

Untested upstream, to be covered by vectors generated through a Go oracle shim built from the pinned sources:

1. Denomination selection: a pure function `(amount, sdiff, feeRate) → (index, count, changeValue | no-split)` extracted from `MixOutput` steps 10–12. Generate a dense table: every split point ± fee boundaries, `sdiff` on both sides of each split point, fee rates 1, 1000, 10000, 100000, 1e6, and amounts around `smallestMixChange`.
2. `smallestMixChange(feeRate)` and `fee(rate, size)` including the zero and overflow clamps.
3. `PossibleCoinJoin` over a corpus of mainnet coinjoins and non-coinjoins, including transactions with duplicate scripts, zero-valued outputs, and two qualifying denominations.
4. `findEligibleOutputsAmount` skip predicate over synthetic credits of every script class and maturity edge.
5. `dicemixExpiry` for each network.
6. `--mixedaccount` parsing (names containing `/`, empty branch, branch `2`, no slash).
7. dcrd JSON-RPC parameter forms for `sendrawmixmessage`, `getmixmessage`, `mixmessage` for each of the eight message types (round-trip of the hex payload at protocol version 10).
8. `GetCoinjoinTxsSumbByAcct` over a fixture wallet.
9. SPV behaviours with a scripted peer: inventory gating of `getdata`, own pair request service for key exchanges, orphan key exchange fetch, unrequested message disconnect, `notfound` disconnect (Q5), relay-disabled disconnect.
10. Ticket buyer fan-out and the per-tip change mix, with a stub wallet.
11. End-to-end: two or more wallets (Go and Rust mixed) plus enough peers to reach `MinPeers` on a private network, for both `MixAccount` and mixed ticket purchases; this is the only test that proves pairing compatibility (items 1–3 of §9).

---

## 12. Rust porting notes

### 12.1 Suggested boundaries

| Module | Contents | Depends on |
| --- | --- | --- |
| `wallet::mixing` (in the wallet core crate) | `MixOutput`, `MixAccount`, `PossibleCoinJoin`, split points, `smallest_mix_change`, semaphores, the `mixclient::Wallet` and `mixpool::BlockChain` adapters, `make_gen`, `add_coinjoin_input`, `dicemix_expiry`, accept/submit helpers | mix client crate (spec 12), mixpool, address manager, tx store, network backend trait |
| `wallet::createtx` (ticket purchase module) | `mixed_split` and the trickle logic | `wallet::mixing`, tx authoring |
| `ticketbuyer` crate/module | The loop of §5.10 | wallet core |
| Network backend trait | `publish_mix_messages`, `wrap_context` equivalent (a cancellation token tied to the backend) | — |
| SPV syncer | §5.14 | p2p crate, wallet core |
| dcrd-RPC syncer and client | §5.15 | JSON-RPC client |
| Config/startup | §5.18, §6.2 | — |

Pure functions worth isolating for differential testing: denomination selection, `possible_coinjoin`, `smallest_mix_change`, the eligibility predicate, `--mixedaccount` parsing.

### 12.2 Reuse

| Need | Candidate (looked at) | Notes |
| --- | --- | --- |
| Mixpool | `dcroxide:crates/dcroxide-mixing/src/mixpool.rs` (`Pool<B: MixBlockChain>`, `accept_message`, `message`, `remove_message`, `expire_messages`, `receive`, observer functions) | It mirrors dcrd master's mixpool, which is ahead of `mixing@v0.7.5` (for example `MAX_KES_PER_IDENTITY`, the mempool-spent hook, `misbehaving_block`). The wallet pin is v0.7.5. The differences must be audited before reuse (§13). Its `receive` is a synchronous query, so the blocking wait-with-deadline the mix client needs has to be layered on top (spec 12). `MixBlockChain` matches the two-method `BlockChain` interface the wallet implements |
| `max_expiry`, limits, script class, `PR_FLAG_CAN_SOLVE_ROOTS` | `dcroxide:crates/dcroxide-mixing/src/lib.rs:42-86` | Direct reuse |
| Message signing, session IDs, DC-net math, PRNG, UTXO proofs | `dcroxide:crates/dcroxide-mixing/src/{signatures,sid,dcnet,prng,utxoproof}.rs` | For spec 12. The crate documents that the hybrid key agreement (`keyagreement.go`) and the whole `mixclient` are not ported |
| Wire messages and inventory types | `dcroxide:crates/dcroxide-wire/src/msg_mix.rs`, `invvect.rs`, `protocol.rs` | All eight mix messages at protocol 12; the wallet speaks version 11 and encodes dcrd RPC payloads at version 10 |
| P2PKH script recognition, address extraction, signature scripts | `dcroxide:crates/dcroxide-txscript/src/{stdscript,stdaddr,sign}.rs` | `ExtractAddrs` class set and stake-tagged P2PKH handling must match `stdscript` at txscript v4.1.2 |
| CSPRNG helpers (durations, shuffles) | `dcroxide:crates/dcroxide-crypto/src/rand.rs` | Check it offers uniform duration and slice shuffle equivalents |
| LRU caches | `dcroxide:crates/dcroxide-containers/src/lru.rs` | For the seen-mix and sent-inventory caches |
| P2PKH input signing | `dcr-rs:src/sign.rs` (`sign_p2pkh_input`) | Alternative to the dcroxide txscript signer; pick one signer for the whole wallet |
| dcrd-side RPC for tests | `dcroxide:crates/dcroxide-rpc` implements `sendrawmixmessage`, `getmixmessage`, `notifymixmessages` | Lets integration tests run against dcroxide as well as dcrd |

### 12.3 Hazards

1. **Blocking calls with long lifetimes.** `MixOutput` holds a semaphore slot, an outpoint lock and a generated address set for many minutes. In async Rust these must be guard types released on every exit path, including cancellation.
2. **Callback re-entrancy.** `SignInput`, `PublishTransaction` and rerun calls of `gen` are invoked from client tasks while the requesting call is parked in `Dicemix`; the first `gen` call runs on the requesting task itself, inside `Dicemix`. They open database transactions and take `addressBuffersMu`/`lockedOutpointMu`; the lock order of §7.2 must hold, and none may be held by the parked caller.
3. **Request-scoped generator context.** `gen` uses the request's context; cancelling the request must make later `gen` calls fail rather than derive addresses.
4. **Non-blocking semaphore semantics.** `tokio::sync::Semaphore::try_acquire` matches; a blocking acquire would change `MixAccount` behaviour.
5. **Capacity zero.** Reproduce Q1/Q2 deliberately or diverge deliberately (§13).
6. **Hash-before-accept.** Messages decoded from dcrd RPC or from the wire must have their hash computed before entering the pool; the pool rejects unhashed messages.
7. **Two publications of the split transaction** and the partially signed return value (Q3).
8. **Solver subprocess.** `--csppsolver` names an external executable speaking Go `net/rpc` (gob encoding) on stdin/stdout (`cspp/v2@v2.4.0/solverrpc/rpc.go:55-80`). Spec 12 owns root solving; this spec only needs the start-up probe semantics preserved (§5.18).
9. **Integer and rounding parity** in fee and size arithmetic; use `i64` atoms with truncating division and the same clamps.
10. **Epoch length on simnet** (Q30) for test planning.

### 12.4 Size estimate

**M.** The wallet-side glue is about 600 lines of Go plus roughly 300 lines spread over the ticket purchase path, 200 in the ticket buyer, 250 in the SPV syncer and peer, 120 in the dcrd-RPC syncer and client, and 150 in configuration and RPC handlers. The logic is simple; the cost is in the lifecycle (guards, cancellation, callback re-entrancy), the dependence on five other subsystems (address manager, tx store, tx authoring, both syncers), and the integration tests needed to prove pairing compatibility. It cannot be completed before spec 12's client exists, but the pure functions and the network plumbing can be built and tested first.

---

## 13. Open questions

1. **Mixpool version.** dcroxide's pool follows dcrd master; dcrwallet pins `mixing v0.7.5`. Decide whether dcr-purse uses the dcroxide pool as is (accepting acceptance-rule differences that only affect which messages the wallet's local pool keeps) or carries a v0.7.5-exact pool. Needs a rule-by-rule diff of `mixpool.go`/`observer.go` between v0.7.5 and the dcrd revision dcroxide mirrors.
2. **Q1/Q2 (`MixSplitLimit` zero or negative).** Reproduce the upstream bug for created wallets, or pass the configured limit and validate the option? Diverging changes no stored data and no wire behaviour.
3. **Q5 (`notfound` disconnect).** Reproduce (peers see a disconnect) or handle `notfound` for mix requests properly? The latter is what the surrounding code intends.
4. **Q3.** Keep the second record/publish of the split transaction and the possibly partially signed `SplitTx` in responses, or return the fully signed stored transaction? What dcrd answers for an already-mined duplicate is UNVERIFIED.
5. **Q7.** Keep "limit 0 buys nothing under mixing"?
6. **Q15/Q16.** Should `MixOutput` validate ownership, spent state and tree, and should stake-change outputs be excluded from `MixAccount` selection? Both effects on dcrd are UNVERIFIED and need an integration test.
7. **Solver.** Whether dcr-purse spawns the Go `csppsolver` binary (requires a gob `net/rpc` client), ships a native solver, or both; and what `--csppsolver` means in each case. Coordinated with spec 12.
8. **Log parity** for the `MIXC`/`MIXP` subsystems: the pinned client and pool emit debug lines whose text is specified by spec 12; dcroxide's pool logs nothing.
9. **`RunAccountMixer` relock semantics** depend on how `Unlock` with a timeout channel followed by `Unlock` with a nil channel behaves (the handler does both); owned by the wallet unlock spec, to be cross-checked there.

