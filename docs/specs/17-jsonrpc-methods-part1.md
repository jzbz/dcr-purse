# 17 — JSON-RPC methods, part 1 (`internal/rpc/jsonrpc/methods.go` lines 1-2797)

## 1. Header

**Scope** (dcrwallet, line counts from `wc -l`):

| File | Lines | Covered here |
|---|---|---|
| `internal/rpc/jsonrpc/methods.go` | 5863 | lines 1-2797: every declaration that starts in that range (48 request handlers and 14 shared helpers/tables). Lines 2798-5863 belong to spec 18. |
| `internal/rpc/jsonrpc/errors.go` | 111 | read in full; fixed error values used by the handlers (owned by spec 16) |
| `internal/rpc/jsonrpc/marshaling.go` | 48 | read in full; used by `getaddressesbyaccount` (owned by spec 16) |
| `rpc/jsonrpc/types/methods.go` | 1336 | read in full; command structs for the handlers in scope |
| `rpc/jsonrpc/types/results.go` | 416 | read in full; result structs for the handlers in scope |
| `internal/rpchelp/helpdescs_en_US.go` | 1044 | read in full; cross-checked against behaviour (section 10.2) |
| `internal/rpchelp/methods.go` | 142 | read in full; documented result type per method |
| `internal/rpc/jsonrpc/rpcserverhelp.go` | 117 | usage strings only, cross-checked |

Supporting code read to pin what a client observes (error kinds and texts,
iteration order, defaults): `errors/errors.go`, `dcrwallet.go:36-39`,
`rpcserver.go:235-246`, `log.go:46-61`, `internal/loader/loader.go:364-404`,
`wallet/network.go:59-71`, `wallet/wallet.go` (the wallet methods each
handler calls, cited inline), `wallet/addresses.go:30-253`, `418-444`,
`504-640`, `747-912`, `915-942`, `wallet/chainntfns.go:324-436`,
`wallet/createtx.go:74-160`, `769-825`, `wallet/multisig.go:21-74`,
`wallet/notifications.go:89-181`, `430-442`, `wallet/tickets.go:1-93`,
`wallet/unstable.go:26-44`, `wallet/rescan.go:386-406`,
`wallet/discovery.go:716-795`, `wallet/walletdb/interface.go:203-255`,
`wallet/udb/addressdb.go:269-273`, `698-770`,
`wallet/udb/addressmanager.go:31-92`, `1297-1329`, `2069-2077`,
`wallet/udb/txmined.go:163-171`, `889-951`, `2240-2345`, `2533-2553`,
`wallet/udb/txdb.go:473-481`, `1992-2005`, `wallet/udb/cfilters.go:17-66`,
`wallet/udb/txquery.go:181-196`, `chain/sync.go:111-132`,
`chain/backend.go:82`, `spv/sync.go:200-223`, `742-773`,
`spv/backend.go:67-111`, `p2p/peering.go:45-52`, `317-336`, `752-771`,
`1504-1563`, `version/version.go:21-27`, `internal/compat` (`IsEitherCoinBaseTx`).

**Parity pin**

| Component | Version |
|---|---|
| dcrwallet | master `54b85b33`, version 2.2.0-pre, module `decred.org/dcrwallet/v5` |
| `github.com/decred/dcrd/dcrjson/v4` | v4.2.0 (`jsonrpcerr.go`, `cmdparse.go`, `helpers.go`) |
| `github.com/decred/dcrd/rpc/jsonrpc/types/v4` | v4.4.0 (`chainsvrcmds.go`, `chainsvrresults.go`: the command and result types reused by the wallet) |
| `github.com/decred/dcrd/dcrutil/v4` | v4.0.3 (`amount.go`, `const.go`, `wif.go`) |
| `github.com/decred/dcrd/chaincfg/chainhash` | v1.0.5 (`hash.go`) |
| `github.com/decred/dcrd/txscript/v4` | v4.1.2 (`stdaddr/address.go`, `stdaddr/addressv0.go`, `stdaddr/error.go`, `stdscript/script.go`, `stdscript/scriptv0.go`) |
| `github.com/decred/dcrd/wire` | v1.8.0 (`msgtx.go`, `protocol.go`) |
| `github.com/decred/dcrd/hdkeychain/v3` | v3.1.3 (error values only) |
| `github.com/decred/dcrd/gcs/v4` | v4.1.1 (`gcs.go`, `blockcf2/blockcf.go` constants) |
| `github.com/decred/dcrd/blockchain/standalone/v2`, `blockchain/stake/v5` | v2.3.0, v5.0.3 (function contracts only, through the dcroxide ports) |
| Go toolchain | `go 1.26.0` in `go.mod`; `encoding/json`, `fmt`, `encoding/hex` and `strconv` behaviour cited from the standard library |

**Summary**

This spec is the per-method contract for the first 48 JSON-RPC handlers of
the wallet (alphabetically `abandontransaction` through `gettxout`, in
source order) plus the helpers they share: address decoding, the verbose
transaction/vin/vout builders, the difficulty ratio and the confirmation
count. Each handler is a function `(server, context, parsed command) ->
(result value, error)`; transport, parameter decoding, the dispatch table
and `convertError` are specified in spec 16 and only summarised here.
Three methods (`getblock`, `getblockheader`, `gettxout`) are forwarded
verbatim to dcrd when the wallet syncs through dcrd RPC and are answered
from wallet data (or SPV peers) otherwise; four more (`getinfo`,
`getpeerinfo`, `getstakeinfo`, `gettickets`) change their content by
backend. Amounts are always JSON numbers produced by `Amount.ToCoin()`
(float64 division by 1e8) and Go float formatting. There is no upstream
test for any handler; this spec, plus a Go oracle shim, is the only pin.
The source has a number of client-visible defects that a drop-in
replacement has to reproduce or consciously fix: `addmultisigaddress`
returns `{}` instead of an address, `getreceivedbyaccount` indexes results
by database key order, `gettransaction` never fills `type`, and others
listed in section 10.

---

## 2. Role in the system

`methods.go` is the body of the JSON-RPC server: a static table mapping
method names to handler functions, and the handler functions themselves.

Callers:

- `Server.handlerClosure` (`internal/rpc/jsonrpc/server.go`, spec 16) calls
  `lazyApplyHandler(s, ctx, request)` (`methods.go:229`) for every request
  that is not `authenticate` or `stop`, then runs the returned closure on
  the HTTP handler goroutine (POST) or on a fresh goroutine (websocket).
- The closure looks the method up in `handlers`; a hit parses the
  positional parameters into the registered command struct
  (`dcrjson.ParseParams`) and calls the handler; a miss forwards the
  request to dcrd when the wallet is synced through dcrd RPC.

Callees:

- `loader.Loader` (`s.walletLoader`): `LoadedWallet()` and
  `NetworkBackend()` (`internal/loader/loader.go:364-404`).
- `wallet.Wallet`: the methods listed per handler in section 5. Each
  wallet method opens its own database transaction (specs 06-08); a handler
  that calls several wallet methods therefore reads several independent
  snapshots.
- The network backend: `*chain.Syncer` (dcrd RPC mode, spec 15) for
  passthrough calls via `Syncer.RPC().Call`, or `*spv.Syncer` (SPV mode,
  spec 14) for `Blocks`, `GetRemotePeers` and `Synced`.
- dcrd libraries for address, script, transaction and header handling
  (`stdaddr`, `stdscript`, `txscript`, `stake`, `standalone`, `wire`,
  `dcrutil`, `hdkeychain`, `chainhash`).
- `s.cfg.Loggers` (`debuglevel`), `s.activeNet` (network parameters), `s.wg`
  and `s.httpServer.BaseContext` (background rescans started by the
  `import*` methods).

Handlers in this range do not use the mixing, VSP or ticket-buyer options
of `Options`; those are consumed by handlers in spec 18.

---

## 3. Public surface inventory

Nothing in `methods.go` is exported except through the `Server` type; every
item below is package-private but carries observable behaviour.

### 3.1 Constants and tables

| Name | Kind | Location | Contract |
|---|---|---|---|
| `jsonrpcSemverString`, `jsonrpcSemverMajor`, `jsonrpcSemverMinor`, `jsonrpcSemverPatch` | const | `methods.go:55-60` | `"11.0.0"`, 11, 0, 0. Reported only by `version` (spec 16 section 4.8, handler in spec 18). |
| `sstxCommitmentString` | const | `methods.go:65` | `"sstxcommitment"`: the `type` string of odd-indexed ticket outputs in verbose vout lists. |
| `scriptVersionAssumed` | const | `methods.go:69` | `0`: script version passed to `stdscript` where the wallet does not store one (`getmultisigoutinfo`, `gettxout`). |
| `handlers` | `map[string]handler` | `methods.go:85-200` | 103 implemented methods plus 8 dead entries (`backupwallet`, `getwalletinfo`, `importwallet`, `listaddressgroupings` -> `unimplemented`; `dumpwallet`, `encryptwallet`, `move`, `setaccount` -> `unsupported`; all `noHelp: true`). Full table in spec 16 section 4.9. Lookup is by exact, case-sensitive method name. |
| `lazyHandler` | type | `methods.go:223` | `func() (any, *dcrjson.RPCError)`. |

The `handlers` literal (`methods.go:86-188`) is declared in this range but
55 of its 103 live entries point at functions that start after line 2797.
The 48 entries of section 3.3 are owned here; the other 55, in source
order of the literal with the handler function and its declaration line,
are owned by spec 18:

`accountunlocked` -> `accountUnlocked` (5733); `getcoinjoinsbyacct` ->
`getcoinjoinsbyacct` (5834); `getvotechoices` -> `getVoteChoices` (2798);
`getwalletfee` -> `getWalletFee` (2848); `help` -> `help` (2882);
`listaccounts` -> `listAccounts` (2941); `listaddresstransactions` ->
`listAddressTransactions` (3230); `listalltransactions` ->
`listAllTransactions` (3265); `listlockunspent` -> `listLockUnspent`
(2975); `listreceivedbyaccount` -> `listReceivedByAccount` (3002);
`listreceivedbyaddress` -> `listReceivedByAddress` (3044);
`listsinceblock` -> `listSinceBlock` (3136); `listtransactions` ->
`listTransactions` (3193); `listunspent` -> `listUnspent` (3281);
`lockaccount` -> `lockAccount` (5785); `lockunspent` -> `lockUnspent`
(3327); `mixaccount` -> `mixAccount` (5649); `mixoutput` -> `mixOutput`
(5613); `purchaseticket` -> `purchaseTicket` (3356);
`processunmanagedticket` -> `processUnmanagedTicket` (3522);
`redeemmultisigout` -> `redeemMultiSigOut` (4071); `redeemmultisigouts` ->
`redeemMultiSigOuts` (4169); `rescanwallet` -> `rescanWallet` (4223);
`sendfrom` -> `sendFrom` (4493); `sendfromtreasury` -> `sendFromTreasury`
(4742); `sendmany` -> `sendMany` (4539); `sendrawtransaction` ->
`sendRawTransaction` (4674); `sendtoaddress` -> `sendToAddress` (4584);
`sendtomultisig` -> `sendToMultiSig` (4627); `sendtotreasury` ->
`sendToTreasury` (4719); `setaccountpassphrase` -> `setAccountPassphrase`
(5715); `setdisapprovepercent` -> `setDisapprovePercent` (3855);
`settreasurypolicy` -> `setTreasuryPolicy` (3877); `settspendpolicy` ->
`setTSpendPolicy` (4008); `settxfee` -> `setTxFee` (4753); `setvotechoice`
-> `setVoteChoice` (4780); `signmessage` -> `signMessage` (4868);
`signrawtransaction` -> `signRawTransaction` (4896); `signrawtransactions`
-> `signRawTransactions` (5123); `spendoutputs` -> `spendOutputs` (4322);
`sweepaccount` -> `sweepAccount` (5241); `ticketinfo` -> `ticketInfo`
(4402); `treasurypolicy` -> `treasuryPolicy` (3792); `tspendpolicy` ->
`tspendPolicy` (3938); `unlockaccount` -> `unlockAccount` (5767);
`validateaddress` -> `validateAddress` (5306); `validatepredcp0005cf` ->
`validatePreDCP0005CF` (5392); `verifymessage` -> `verifyMessage` (5404);
`version` -> `version` (5438); `walletinfo` -> `walletInfo` (5468);
`walletislocked` -> `walletIsLocked` (5546); `walletlock` -> `walletLock`
(5558); `walletpassphrase` -> `walletPassphrase` (5572);
`walletpassphrasechange` -> `walletPassphraseChange` (5595);
`walletpubpassphrasechange` -> `walletPubPassphraseChange` (5700).

The literal is not in alphabetical order (`getblock` follows
`getblockheader`, `help` precedes `getcfilterv2`); the order is not
observable because the map is only ever indexed by name at run time. The
one place that iterates it is the test helper `serverMethods`
(`rpchelp_test.go:27-35`), which collects the names whose entry has
`noHelp == false`. The entry type `handler` (`server.go:80-83`) has two
fields: `fn func(*Server, context.Context, any) (any, error)` and `noHelp
bool`. `authenticate` and `stop` have no entry: the transport handles
them before dispatch (spec 16).

Imports of `methods.go` that no declaration in lines 1-2797 uses
(`encoding/base64`, `encoding/binary`, `sync`, `rpc/client/dcrd`,
`wallet/txrules`, `wallet/txsizes`, `dcrd/crypto/rand`, `dcrd/dcrec`,
`dcrd/dcrec/secp256k1/v4`, `txscript/v4/sign`, `golang.org/x/sync/errgroup`)
serve spec 18 handlers only; nothing in this range signs, draws
randomness or runs an error group.

### 3.2 Shared helpers

| Name | Signature | Location | Contract |
|---|---|---|---|
| `confirms` | `(txHeight, curHeight int32) int32` | `methods.go:75-82` | `0` if `txHeight == -1` or `txHeight > curHeight`; otherwise `curHeight - txHeight + 1`. Identical to `wallet.confirms` (`wallet/wallet.go:4411`). |
| `unimplemented` | `(*Server, context.Context, any) (any, error)` | `methods.go:204-209` | Always `{-1, "Method unimplemented"}`. Unreachable (section 10.1 Q1). |
| `unsupported` | same | `methods.go:213-218` | Always `{-1, "Request unsupported by dcrwallet"}`. Unreachable. |
| `lazyApplyHandler` | `(s *Server, ctx, request *dcrjson.Request) lazyHandler` | `methods.go:229-279` | Dispatch and dcrd passthrough. Section 5.1; full detail in spec 16 section 5.5. |
| `makeResponse` | `(id, result any, err error) dcrjson.Response` | `methods.go:284-306` | Builds a `dcrjson.Response` without the `jsonrpc` member; used only by three websocket paths in `server.go` (spec 16 section 4.2). |
| `walletPubKeys` | `(ctx, w *wallet.Wallet, keys []string) ([][]byte, error)` | `methods.go:393-426` | Resolves each string to a 33-byte compressed secp256k1 public key. Section 5.2.3. |
| `createTxRawResult` | `(chainParams, mtx *wire.MsgTx, blkIdx uint32, blkHeader *wire.BlockHeader, confirmations int64, isTreasuryEnabled bool) (*dcrdtypes.TxRawResult, error)` | `methods.go:1507-1534` | Verbose transaction object for `getblock` with `verbosetx`. Section 4.6. |
| `createVinList` | `(mtx *wire.MsgTx, isTreasuryEnabled bool) []dcrdtypes.Vin` | `methods.go:1538-1615` | Section 4.7. |
| `createVoutList` | `(mtx *wire.MsgTx, chainParams, filterAddrMap map[string]struct{}) []dcrdtypes.Vout` | `methods.go:1619-1704` | Section 4.8. The only caller passes a nil filter. |
| `difficultyRatio` | `(bits uint32, params *chaincfg.Params) float64` | `methods.go:1708-1724` | Section 5.2.4. |
| `decodeAddress` | `(s string, params *chaincfg.Params) (stdaddr.Address, error)` | `methods.go:1829-1851` | Section 5.2.1. |
| `decodeStakeAddress` | `(s string, params *chaincfg.Params) (stdaddr.StakeAddress, error)` | `methods.go:1853-1863` | Section 5.2.2. Defined here, called only from spec 18 handlers. |

`makeScriptChangeSource` (`methods.go:5220-5231`, outside this range) is
called by `fundrawtransaction`; its contract is restated in section 5.3.13.

Package-level identifiers defined in other files of the package and used
by code in this range (owned by spec 16; listed so nothing a handler
depends on is implicit):

| Name | Location | Used here for |
|---|---|---|
| `convertError(err) *dcrjson.RPCError` | `errors.go:16-49` | `lazyApplyHandler`, `makeResponse`: `*dcrjson.RPCError` unchanged; `*wsrpc.Error` -> same code and message; otherwise code by wallet error kind (section 5 conventions) and message `err.Error()` |
| `rpcError(code, err)` | `errors.go:51-56` | RPC error with message `err.Error()` |
| `rpcErrorf(code, format, args...)` | `errors.go:58-63` | RPC error with message `fmt.Sprintf(format, args...)` |
| `errUnloadedWallet`, `errRPCClientNotConnected`, `errNoNetwork`, `errAccountNotFound`, `errAddressNotInWallet`, `errNotImportedAccount`, `errWalletUnlockNeeded`, `errReservedAccountName` | `errors.go:66-110` | the fixed errors of section 8.2. They are shared pointer values: every reply carries the same code and text. |
| `errNeedPositiveAmount` (`-8` `amount must be positive`) | `errors.go:97-100` | **not used in this range** (first use is `methods.go:3571`, spec 18) |
| `remoteAddr(ctx) string` | `context.go:15-21` | the `<remote addr>` of the cancellation log line; `<unknown>` when the context carries no address |
| `idPointer(id any) *any` | `server.go:308-313` | `makeResponse`: nil for a nil id, otherwise a pointer to the id |
| `knownAddressMarshaler`, `addressStringsMarshaler` | `marshaling.go:38-48` | `getaddressesbyaccount`: hand-written JSON array writer (`[`, each string between `"` with **no escaping**, `,` separators, `]`); a nil or empty list writes `[]` |
| `log` | `log.go` | subsystem `RPCS` logger (section 8.5) |
| `handler` | `server.go:80-83` | entry type of `handlers` |

### 3.3 Handlers

All have the signature `func (s *Server) name(ctx context.Context, icmd
any) (any, error)`. "W" = requires a loaded wallet (`errUnloadedWallet`
otherwise). "N" = requires a network backend. Backend column: "same" = no
difference between dcrd RPC and SPV; otherwise see the cited section.

| Method | Handler | Lines | W | N | Backend | Result (JSON) | Section |
|---|---|---|---|---|---|---|---|
| `abandontransaction` | `abandonTransaction` | 310-324 | yes | no | same | `null` | 5.3.1 |
| `accountaddressindex` | `accountAddressIndex` | 328-355 | yes | no | same | number (uint32) | 5.3.2 |
| `accountsyncaddressindex` | `accountSyncAddressIndex` | 362-389 | yes | no | same (filter load if any backend) | `null` | 5.3.3 |
| `addmultisigaddress` | `addMultiSigAddress` | 430-457 | yes | no | same | `{}` (sic) | 5.3.4 |
| `addtransaction` | `addTransaction` | 459-479 | yes | no | same | `null` | 5.3.5 |
| `auditreuse` | `auditReuse` | 483-562 | yes | no | same | object: address -> array of outpoint strings | 5.3.6 |
| `consolidate` | `consolidate` | 566-603 | yes | yes (inside wallet) | same | string (tx hash) | 5.3.7 |
| `createmultisig` | `createMultiSig` | 607-632 | yes | no | same | `CreateMultiSigResult` | 5.3.8 |
| `createrawtransaction` | `createRawTransaction` | 635-742 | no | no | same | string (hex tx) | 5.3.9 |
| `createsignature` | `createSignature` | 747-796 | yes | no | same | `CreateSignatureResult` | 5.3.10 |
| `debuglevel` | `debugLevel` | 798-813 | no | no | same | string | 5.3.11 |
| `disapprovepercent` | `disapprovePercent` | 816-822 | yes | no | same | number (uint32) | 5.3.12 |
| `discoverusage` | `discoverUsage` | 824-853 | yes | yes | differs inside the wallet (spec 09) | `null` | 5.3.12 |
| `dumpprivkey` | `dumpPrivKey` | 858-878 | yes | no | same | string (WIF) | 5.3.12 |
| `fundrawtransaction` | `fundRawTransaction` | 880-971 | yes | no | same | `FundRawTransactionResult` | 5.3.13 |
| `getaddressesbyaccount` | `getAddressesByAccount` | 976-1042 | yes | no | same | array of strings | 5.3.14 |
| `getbalance` | `getBalance` | 1047-1161 | yes | no | same | `GetBalanceResult` | 5.3.15 |
| `getbestblock` | `getBestBlock` | 1165-1177 | yes | no | same | `GetBestBlockResult` | 5.3.16 |
| `getbestblockhash` | `getBestBlockHash` | 1181-1189 | yes | no | same | string | 5.3.16 |
| `getblockcount` | `getBlockCount` | 1193-1201 | yes | no | same | number (int32) | 5.3.16 |
| `getblockhash` | `getBlockHash` | 1205-1219 | yes | no | same | string | 5.3.16 |
| `getblockheader` | `getBlockHeader` | 1222-1337 | yes | yes | **passthrough on dcrd**, wallet data on SPV | string or `GetBlockHeaderVerboseResult` | 5.3.17 |
| `getblock` | `getBlock` | 1340-1505 | yes | yes | **passthrough on dcrd**, peers on SPV | string or `GetBlockVerboseResult` | 5.3.18 |
| `syncstatus` | `syncStatus` | 1727-1761 | yes | yes | `Synced` differs | `SyncStatusResult` | 5.3.19 |
| `getcurrentnet` | `getCurrentNet` | 1764-1766 | no | no | same | number (uint32) | 5.3.20 |
| `getinfo` | `getInfo` | 1770-1827 | yes | no | **dcrd fields merged on dcrd** | `InfoResult` | 5.3.21 |
| `getaccount` | `getAccount` | 1867-1888 | yes | no | same | string | 5.3.22 |
| `getaccountaddress` | `getAccountAddress` | 1896-1920 | yes | no | same | string | 5.3.22 |
| `getunconfirmedbalance` | `getUnconfirmedBalance` | 1924-1952 | yes | no | same | number | 5.3.23 |
| `getcfilterv2` | `getCFilterV2` | 1955-1977 | yes | no | same (never forwarded) | `GetCFilterV2Result` (wallet shape) | 5.3.24 |
| `importcfiltersv2` | `importCFiltersV2` | 1981-2004 | yes | no | same | `null` | 5.3.24 |
| `importprivkey` | `importPrivKey` | 2008-2067 | yes | if rescan | same | `null` | 5.3.25 |
| `importpubkey` | `importPubKey` | 2073-2125 | yes | if rescan | same | `null` | 5.3.25 |
| `importscript` | `importScript` | 2128-2177 | yes | if rescan | same | `null` | 5.3.25 |
| `importxpub` | `importXpub` | 2179-2192 | yes | no | same | `null` | 5.3.26 |
| `createnewaccount` | `createNewAccount` | 2197-2218 | yes | no | same | `null` | 5.3.27 |
| `renameaccount` | `renameAccount` | 2222-2245 | yes | no | same | `null` | 5.3.27 |
| `getmultisigoutinfo` | `getMultisigOutInfo` | 2249-2302 | yes | no | same | `GetMultisigOutInfoResult` | 5.3.28 |
| `getnewaddress` | `getNewAddress` | 2307-2346 | yes | no | same | string | 5.3.29 |
| `getrawchangeaddress` | `getRawChangeAddress` | 2353-2379 | yes | no | same | string | 5.3.29 |
| `getreceivedbyaccount` | `getReceivedByAccount` | 2383-2420 | yes | no | same | number | 5.3.30 |
| `getreceivedbyaddress` | `getReceivedByAddress` | 2424-2450 | yes | no | same | number | 5.3.30 |
| `getmasterpubkey` | `getMasterPubkey` | 2454-2486 | yes | no | same | string (xpub) | 5.3.31 |
| `getpeerinfo` | `getPeerInfo` | 2490-2539 | yes | yes | **dcrd call filtered / SPV peers** | array of `GetPeerInfoResult` | 5.3.32 |
| `getstakeinfo` | `getStakeInfo` | 2543-2591 | yes | no | **precise on dcrd, estimate on SPV** | `GetStakeInfoResult` | 5.3.33 |
| `gettickets` | `getTickets` | 2595-2617 | yes | no | **live set only on dcrd** | `GetTicketsResult` | 5.3.34 |
| `gettransaction` | `getTransaction` | 2621-2710 | yes | no | same | `GetTransactionResult` | 5.3.35 |
| `gettxout` | `getTxOut` | 2719-2794 | yes | yes | **passthrough on dcrd**, wallet UTXOs on SPV | `null` or `GetTxOutResult` | 5.3.36 |

`getVoteChoices` begins at line 2798 and belongs to spec 18.

---

## 4. Data formats

Nothing in this scope is persisted. The transmitted structures are the
JSON-RPC parameters and results. The envelope, the positional parameter
decoding rules and the command struct table are in spec 16 sections 4.1 to
4.6; the rules a handler author needs are restated in 4.1 and 4.2 below.

### 4.1 Parameter conventions (from spec 16, restated)

- Parameters are positional; parameter *i* is decoded into struct field *i*
  of the command struct. `json` tags on top-level command fields have no
  wire effect.
- A pointer field is optional. An omitted trailing optional parameter
  receives its `jsonrpcdefault` value if the field has one, otherwise nil.
- An explicit JSON `null` for an optional parameter leaves the pointer
  **nil and suppresses the default**. Handlers then apply their own nil
  handling, which in two cases differs from the registered default
  (`getblock`, `getblockheader`: section 10.1 Q9).
- An explicit `null` for a required parameter leaves the Go zero value.
- Any decode failure (wrong count, wrong type, out-of-range integer) is
  answered `-32600 Invalid request` before the handler runs.
- Integer parameters typed `int` are 64-bit. Several handlers narrow them
  with an unchecked Go conversion (two's-complement truncation); each
  occurrence is called out in section 5.3 and collected in section 10.1 Q7.
- Members of nested JSON objects (`TransactionInput`,
  `FundRawTransactionOptions`) are matched to struct fields by `json` tag,
  **case-insensitively**, unknown members are ignored and duplicate members
  keep the last value (Go `encoding/json` object decoding).

### 4.2 Scalar encodings used in results

| Kind | Encoding |
|---|---|
| Amount | JSON number: `float64(atoms) / 1e8` (`dcrutil.Amount.ToCoin`, `dcrd/dcrutil/v4@v4.0.3/amount.go:93-100`), formatted by Go `encoding/json`: shortest decimal that round-trips the float64; fixed notation when `1e-6 <= abs < 1e21`, otherwise exponent notation with the exponent's leading zero removed (`1e-08` is written `1e-8`). One atom is `1e-8`; 99 atoms is `9.9e-7`; 100 atoms is `0.000001`. Integral values print without a fraction (`2`, not `2.0`). Negative zero prints `-0` (reachable through `gettransaction` details, section 5.3.35). |
| Amount parameter | JSON number in DCR converted with `dcrutil.NewAmount` (`amount.go:76-89`): `round(f * 1e8)` where `round` adds 0.5 (or subtracts for negatives) and truncates toward zero through a Go `float64 -> int64` conversion. Errors only for NaN/Inf, which JSON cannot carry. Out-of-int64-range products produce an implementation-defined integer in Go (section 10.1 Q8). |
| Hash | 64 lowercase hex characters, byte-reversed (`chainhash.Hash.String`). |
| Hash parameter | `chainhash.NewHashFromStr` (`dcrd/chaincfg/chainhash@v1.0.5/hash.go:87-129`): at most 64 characters else error `max hash string length is 64 bytes`; an odd length is left-padded with one `0`; a short string is the low-order bytes of the hash (zero-extended), so `""` is the all-zero hash and `"1"` is hash `00…01`; upper- and lower-case hex are both accepted; a non-hex character gives the Go text `encoding/hex: invalid byte: U+0067 'g'` (code point of the offending byte). |
| Script, key, serialized tx/block/header | lowercase hex (`hex.EncodeToString`), no prefix. |
| Outpoint string | `<hash>:<index>` with decimal index and no tree (`wire.OutPoint.String`, `dcrd/wire@v1.8.0/msgtx.go:290-302`). |
| Address | `Address.String()` of the `stdaddr` type (base58check with the network's 2-byte prefix). |
| `bits` | lowercase hex of the uint32 without leading zeros (`strconv.FormatInt(int64(bits), 16)`). |
| Booleans, integers | plain JSON; integer widths follow the Go field type. |
| Strings | Go `encoding/json` string escaping with HTML escaping on (`<`, `>`, `&` become `\u003c`, `\u003e`, `\u0026`). |
| Nil slice vs empty slice | A nil Go slice encodes as `null`, an empty non-nil slice as `[]`; with `omitempty` both are omitted. Every array below states which it is. |
| Nil map vs empty map | `auditreuse` returns a non-nil map: `{}` when empty. Map keys are emitted in ascending byte order. |

### 4.3 Result structs defined by the wallet (`rpc/jsonrpc/types/results.go`)

Members are listed in emission order (Go struct order). "oe" = `omitempty`
(omitted for `0`, `0.0`, `false`, `""`, nil pointer, nil or empty slice).

**`FundRawTransactionResult`** (`results.go:9-12`)

| # | JSON name | Go type | Notes |
|---|---|---|---|
| 1 | `hex` | string | serialized funded transaction |
| 2 | `fee` | float64 | absolute fee in DCR |

**`GetAccountBalanceResult`** (`results.go:15-24`)

| # | JSON name | Go type |
|---|---|---|
| 1 | `accountname` | string |
| 2 | `immaturecoinbaserewards` | float64 |
| 3 | `immaturestakegeneration` | float64 |
| 4 | `lockedbytickets` | float64 |
| 5 | `spendable` | float64 |
| 6 | `total` | float64 |
| 7 | `unconfirmed` | float64 |
| 8 | `votingauthority` | float64 |

**`GetBalanceResult`** (`results.go:27-37`)

| # | JSON name | Go type | oe |
|---|---|---|---|
| 1 | `balances` | `[]GetAccountBalanceResult` | no |
| 2 | `blockhash` | string | no |
| 3 | `totalimmaturecoinbaserewards` | float64 | oe |
| 4 | `totalimmaturestakegeneration` | float64 | oe |
| 5 | `totallockedbytickets` | float64 | oe |
| 6 | `totalspendable` | float64 | oe |
| 7 | `cumulativetotal` | float64 | oe |
| 8 | `totalunconfirmed` | float64 | oe |
| 9 | `totalvotingauthority` | float64 | oe |

**`GetMultisigOutInfoResult`** (`results.go:41-54`), no member is `omitempty`

| # | JSON name | Go type |
|---|---|---|
| 1 | `address` | string |
| 2 | `redeemscript` | string |
| 3 | `m` | uint8 |
| 4 | `n` | uint8 |
| 5 | `pubkeys` | `[]string` |
| 6 | `txhash` | string |
| 7 | `blockheight` | uint32 |
| 8 | `blockhash` | string |
| 9 | `spent` | bool |
| 10 | `spentby` | string |
| 11 | `spentbyindex` | uint32 |
| 12 | `amount` | float64 |

**`CreateMultiSigResult`** (`results.go:58-61`): `address` string,
`redeemScript` string (capital S).

**`CreateSignatureResult`** (`results.go:65-68`): `signature` string,
`publickey` string.

**`GetPeerInfoResult`** (`results.go:71-80`), no `omitempty`

| # | JSON name | Go type |
|---|---|---|
| 1 | `id` | int32 |
| 2 | `addr` | string |
| 3 | `addrlocal` | string |
| 4 | `services` | string |
| 5 | `version` | uint32 |
| 6 | `subver` | string |
| 7 | `startingheight` | int64 |
| 8 | `banscore` | int32 |

**`GetStakeInfoResult`** (`results.go:84-104`)

| # | JSON name | Go type | oe |
|---|---|---|---|
| 1 | `blockheight` | int64 | no |
| 2 | `difficulty` | float64 | no |
| 3 | `totalsubsidy` | float64 | no |
| 4 | `ownmempooltix` | uint32 | no |
| 5 | `immature` | uint32 | no |
| 6 | `unspent` | uint32 | no |
| 7 | `voted` | uint32 | no |
| 8 | `revoked` | uint32 | no |
| 9 | `unspentexpired` | uint32 | no |
| 10 | `poolsize` | uint32 | oe |
| 11 | `allmempooltix` | uint32 | oe |
| 12 | `live` | uint32 | oe |
| 13 | `proportionlive` | float64 | oe |
| 14 | `missed` | uint32 | oe |
| 15 | `proportionmissed` | float64 | oe |
| 16 | `expired` | uint32 | oe |

**`GetTicketsResult`** (`results.go:108-110`): `hashes` `[]string`.

**`GetTransactionDetailsResult`** (`results.go:117-125`)

| # | JSON name | Go type | oe |
|---|---|---|---|
| 1 | `account` | string | no |
| 2 | `address` | string | oe |
| 3 | `amount` | float64 | no |
| 4 | `category` | string | no |
| 5 | `involveswatchonly` | bool | oe |
| 6 | `fee` | `*float64` | oe (nil pointer only; a pointer to `0` is emitted as `0`) |
| 7 | `vout` | uint32 | no |

**`GetTransactionResult`** (`results.go:128-143`)

| # | JSON name | Go type | oe |
|---|---|---|---|
| 1 | `amount` | float64 | no |
| 2 | `fee` | float64 | oe |
| 3 | `confirmations` | int64 | no |
| 4 | `blockhash` | string | no |
| 5 | `blockindex` | int64 | no |
| 6 | `blocktime` | int64 | no |
| 7 | `txid` | string | no |
| 8 | `walletconflicts` | `[]string` | no |
| 9 | `time` | int64 | no |
| 10 | `timereceived` | int64 | no |
| 11 | `details` | `[]GetTransactionDetailsResult` | no |
| 12 | `hex` | string | no |
| 13 | `type` | string | no |
| 14 | `ticketstatus` | string | oe |

**`GetCFilterV2Result`** (`results.go:146-150`): `blockhash` string,
`filter` string, `key` string. This is the wallet's own type; it is not
dcrd's `GetCFilterV2Result` (`blockhash`, `data`, `proofindex`,
`proofhashes`).

**`SyncStatusResult`** (`results.go:167-171`): `synced` bool,
`initialblockdownload` bool, `headersfetchprogress` **float32** (formatted
as the shortest decimal that round-trips a float32, with float32
thresholds for exponent notation).

**`InfoResult`** (`results.go:175-192`), no `omitempty`

| # | JSON name | Go type |
|---|---|---|
| 1 | `version` | int32 |
| 2 | `protocolversion` | int32 |
| 3 | `walletversion` | int32 |
| 4 | `balance` | float64 |
| 5 | `blocks` | int32 |
| 6 | `timeoffset` | int64 |
| 7 | `connections` | int32 |
| 8 | `proxy` | string |
| 9 | `difficulty` | float64 |
| 10 | `testnet` | bool |
| 11 | `keypoololdest` | int64 |
| 12 | `keypoolsize` | int32 |
| 13 | `unlocked_until` | int64 |
| 14 | `paytxfee` | float64 |
| 15 | `relayfee` | float64 |
| 16 | `errors` | string |

### 4.4 Result structs reused from dcrd (`dcrd/rpc/jsonrpc/types/v4@v4.4.0/chainsvrresults.go`)

**`GetBestBlockResult`** (`:114-117`): `hash` string, `height` int64.

**`GetBlockHeaderVerboseResult`** (`:139-165`)

| # | JSON name | Go type | oe |
|---|---|---|---|
| 1 | `hash` | string | no |
| 2 | `powhash` | string | no |
| 3 | `confirmations` | int64 | no |
| 4 | `version` | int32 | no |
| 5 | `merkleroot` | string | no |
| 6 | `stakeroot` | string | no |
| 7 | `votebits` | uint16 | no |
| 8 | `finalstate` | string | no |
| 9 | `voters` | uint16 | no |
| 10 | `freshstake` | uint8 | no |
| 11 | `revocations` | uint8 | no |
| 12 | `poolsize` | uint32 | no |
| 13 | `bits` | string | no |
| 14 | `sbits` | float64 | no |
| 15 | `height` | uint32 | no |
| 16 | `size` | uint32 | no |
| 17 | `time` | int64 | no |
| 18 | `mediantime` | int64 | no |
| 19 | `nonce` | uint32 | no |
| 20 | `extradata` | string | no |
| 21 | `stakeversion` | uint32 | no |
| 22 | `difficulty` | float64 | no |
| 23 | `chainwork` | string | no |
| 24 | `previousblockhash` | string | oe |
| 25 | `nextblockhash` | string | oe |

**`GetBlockVerboseResult`** (`:63-93`)

| # | JSON name | Go type | oe |
|---|---|---|---|
| 1 | `hash` | string | no |
| 2 | `powhash` | string | no |
| 3 | `confirmations` | int64 | no |
| 4 | `size` | int32 | no |
| 5 | `height` | int64 | no |
| 6 | `version` | int32 | no |
| 7 | `merkleroot` | string | no |
| 8 | `stakeroot` | string | no |
| 9 | `tx` | `[]string` | oe |
| 10 | `rawtx` | `[]TxRawResult` | oe |
| 11 | `stx` | `[]string` | oe |
| 12 | `rawstx` | `[]TxRawResult` | oe |
| 13 | `time` | int64 | no |
| 14 | `mediantime` | int64 | no |
| 15 | `nonce` | uint32 | no |
| 16 | `votebits` | uint16 | no |
| 17 | `finalstate` | string | no |
| 18 | `voters` | uint16 | no |
| 19 | `freshstake` | uint8 | no |
| 20 | `revocations` | uint8 | no |
| 21 | `poolsize` | uint32 | no |
| 22 | `bits` | string | no |
| 23 | `sbits` | float64 | no |
| 24 | `extradata` | string | no |
| 25 | `stakeversion` | uint32 | no |
| 26 | `difficulty` | float64 | no |
| 27 | `chainwork` | string | no |
| 28 | `previousblockhash` | string | no |
| 29 | `nextblockhash` | string | oe |

**`GetTxOutResult`** (`:388-394`): `bestblock` string, `confirmations`
int64, `value` float64, `scriptPubKey` `ScriptPubKeyResult`, `coinbase`
bool. No `omitempty`.

**`ScriptPubKeyResult`** (`:576-584`)

| # | JSON name | Go type | oe |
|---|---|---|---|
| 1 | `asm` | string | no |
| 2 | `hex` | string | oe |
| 3 | `reqSigs` | int32 | oe |
| 4 | `type` | string | no |
| 5 | `addresses` | `[]string` | oe |
| 6 | `commitamt` | `*float64` | oe |
| 7 | `version` | uint16 | no |

**`InfoChainResult`** (`:199-211`) is decode-only: the wallet unmarshals
dcrd's `getinfo` reply into it and copies seven members (section 5.3.21).

### 4.5 `ScriptSig`, `Vout`

`ScriptSig` (`:589-592`): `asm` string, `hex` string.

`Vout` (`:718-723`): `value` float64, `n` uint32, `version` uint16,
`scriptPubKey` `ScriptPubKeyResult`. No `omitempty`.

### 4.6 `TxRawResult` as built by `createTxRawResult` (`methods.go:1507-1534`)

| # | JSON name | Go type | oe | Value |
|---|---|---|---|---|
| 1 | `hex` | string | no | full serialization (`MsgTx.Serialize`, prefix and witness) in hex |
| 2 | `txid` | string | no | `mtx.CachedTxHash().String()` |
| 3 | `version` | int32 | no | `int32(mtx.Version)` (the 16-bit version only, not the serialization type) |
| 4 | `locktime` | uint32 | no | `mtx.LockTime` |
| 5 | `expiry` | uint32 | no | `mtx.Expiry` |
| 6 | `vin` | `[]Vin` | no | section 4.7 |
| 7 | `vout` | `[]Vout` | no | section 4.8 with a nil filter |
| 8 | `blockhash` | string | oe | `blkHeader.BlockHash().String()` (recomputed per transaction) |
| 9 | `blockheight` | int64 | oe | `int64(blkHeader.Height)`; omitted for the genesis block |
| 10 | `blockindex` | uint32 | oe | index of the transaction within **its own tree** (regular or stake); omitted for index 0 |
| 11 | `confirmations` | int64 | oe | the block's confirmation count as computed by `getblock`; `-1` for a side-chain block (emitted), never 0 |
| 12 | `time` | int64 | oe | `blkHeader.Timestamp.Unix()` |
| 13 | `blocktime` | int64 | oe | same value as `time` |

`createTxRawResult` can fail only if `mtx.Serialize` fails, which cannot
happen for an in-memory transaction; the caller would then answer `-32603`
`Could not create transaction: <err>` or `Could not create stake
transaction: <err>` (`methods.go:1484-1499`).

### 4.7 `Vin` list as built by `createVinList` (`methods.go:1538-1615`)

The list has exactly `len(mtx.TxIn)` entries. Each entry is emitted by
`Vin.MarshalJSON` (`chainsvrresults.go:627-714`, pointer receiver; it is
invoked because the entries are slice elements), which selects one of five
shapes by inspecting the entry in this order:

| Order | Condition on the entry | Emitted members, in order |
|---|---|---|
| 1 | `coinbase` string non-empty | `coinbase`, `sequence`, `amountin`, `blockheight`, `blockindex` |
| 2 | `stakebase` string non-empty | `stakebase`, `sequence`, `amountin`, `blockheight`, `blockindex` |
| 3 | `treasurybase` true | `treasurybase` (always `true`), `sequence`, `amountin`, `blockheight`, `blockindex` |
| 4 | `treasuryspend` string non-empty | `treasuryspend`, `sequence`, `amountin`, `blockheight`, `blockindex` |
| 5 | otherwise | `txid`, `vout`, `tree`, `sequence`, `amountin`, `blockheight`, `blockindex`, `scriptSig` (object `{asm, hex}` or `null`) |

Types: `sequence` uint32, `amountin` float64 (`Amount(txIn.ValueIn).ToCoin()`;
a null value-in of `-1` atoms prints `-1e-8`), `blockheight` uint32,
`blockindex` uint32, `vout` uint32, `tree` int8.

Population, in the order the Go checks them (`isTreasuryEnabled` is the
caller's flag):

1. If `isTreasuryEnabled` and `standalone.IsTreasuryBase(mtx)`: entry 0
   gets `treasurybase = true` plus `sequence`, `amountin`, `blockheight`,
   `blockindex` of input 0. Return. This check must precede the coinbase
   check because a treasurybase also satisfies `IsCoinBaseTx`.
2. If `standalone.IsCoinBaseTx(mtx, isTreasuryEnabled)`: entry 0 gets
   `coinbase = hex(sigScript of input 0)` plus the four common members.
   Return.
3. If `isTreasuryEnabled` and `stake.IsTSpend(mtx)`: entry 0 gets
   `treasuryspend = hex(sigScript of input 0)` plus the four common
   members. Return.
4. Otherwise let `isSSGen = stake.IsSSGen(mtx)`. For each input `i`:
   - if `isSSGen` and `i == 0`: `stakebase = hex(sigScript)` plus the four
     common members;
   - else: `txid`, `vout`, `tree` from the previous outpoint, the four
     common members, and `scriptSig = {asm: DisasmString(sigScript), hex:
     hex(sigScript)}`. The disassembly error is ignored (the string
     contains `[error]` inline).

Each of the three classifiers in steps 1 to 3 requires exactly one input,
so the list returned from those steps has one entry. If the signature
script hex in steps 2 to 4 is empty the marshaler falls through to shape 5
with empty `txid` and `"scriptSig":null`; consensus-valid coinbases, votes
and treasury spends always have a non-empty script.

### 4.8 `Vout` list as built by `createVoutList` (`methods.go:1619-1704`)

`txType = stake.DetermineTxType(mtx)`. For each output `i` (in order):

1. `asm = txscript.DisasmString(pkScript)`, error ignored.
2. If `txType == TxTypeSStx` and `i` is odd (a ticket commitment output):
   - `type = "sstxcommitment"`;
   - `stake.AddrFromSStxPkScrCommitment(pkScript, params)`: on success the
     address list is that single address; on failure log at warn
     `Failed to decode ticket commitment addr output for tx hash <hash>,
     output idx <i>` and leave the list empty;
   - `stake.AmountFromSStxPkScrCommitment(pkScript)`: on success
     `commitamt = amount.ToCoin()`; on failure log at warn `Failed to decode
     ticket commitment amt output for tx hash <hash>, output idx <i>` and
     omit `commitamt`;
   - `reqSigs` stays 0 (omitted).
3. Otherwise `(scriptType, addrs) = stdscript.ExtractAddrs(version,
   pkScript, params)`, `reqSigs = stdscript.DetermineRequiredSigs(version,
   pkScript)`, `type = scriptType.String()` (table below).
4. Encode every address with `String()`. With a nil or empty filter map
   every output passes; with a non-empty filter an output is kept only if
   at least one encoded address is a key of the map. (No caller in the
   wallet passes a filter.)
5. Emit `value = Amount(v.Value).ToCoin()`, `n = i`, `version =
   v.Version`, and `scriptPubKey = {asm, hex: hex(pkScript), reqSigs,
   type, addresses, commitamt}`.

`scriptPubKey.version` is **never assigned** by the wallet and is always
`0` regardless of the output's script version; the outer `version` member
carries the real value (section 10.1 Q15). `addresses` is an empty non-nil
slice when no address was extracted and is therefore omitted. `hex` is
omitted for an empty script.

`ScriptType.String()` values (`dcrd/txscript/v4@v4.1.2/stdscript/script.go:180-210`):

| Script type | String |
|---|---|
| non-standard | `nonstandard` |
| P2PK ECDSA secp256k1 | `pubkey` |
| P2PK Ed25519 | `pubkey-ed25519` |
| P2PK Schnorr secp256k1 | `pubkey-schnorr-secp256k1` |
| P2PKH ECDSA secp256k1 | `pubkeyhash` |
| P2PKH Ed25519 | `pubkeyhash-ed25519` |
| P2PKH Schnorr secp256k1 | `pubkeyhash-schnorr-secp256k1` |
| P2SH | `scripthash` |
| multisig | `multisig` |
| null data | `nulldata` |
| stake submission P2PKH / P2SH | `stakesubmission-pubkeyhash` / `stakesubmission-scripthash` |
| stake generation P2PKH / P2SH | `stakegen-pubkeyhash` / `stakegen-scripthash` |
| stake revocation P2PKH / P2SH | `stakerevoke-pubkeyhash` / `stakerevoke-scripthash` |
| stake change P2PKH / P2SH | `stakechange-pubkeyhash` / `stakechange-scripthash` |
| treasury add | `treasuryadd` |
| treasury generation P2PKH / P2SH | `treasurygen-pubkeyhash` / `treasurygen-scripthash` |

### 4.9 Parameter object types

`TransactionInput` (`dcrd/rpc/jsonrpc/types/v4@v4.4.0/chainsvrcmds.go:116-121`),
element of `createrawtransaction` parameter 1:

| JSON name | Go type | Notes |
|---|---|---|
| `amount` | float64 | optional on input (absent gives 0); `omitempty` matters only when marshalling |
| `txid` | string | |
| `vout` | uint32 | |
| `tree` | int8 | |

`FundRawTransactionOptions` (`rpc/jsonrpc/types/methods.go:165-169`),
`fundrawtransaction` parameter 3:

| JSON name | Go type |
|---|---|
| `changeaddress` | `*string` |
| `feerate` | `*float64` |
| `conf_target` | `*int32` |

---

## 5. Algorithms and state machines

Conventions used throughout this section:

- "RPC error `c` `m`" means the handler returns a `*dcrjson.RPCError`
  `{code: c, message: m}`, which reaches the client unchanged.
- "wallet error" means the handler returns a Go error that is not an
  `*RPCError`; it passes through `convertError` (spec 16 section 8.2): the
  code is chosen by the wallet error kind (`Bug` -32603, `Encoding` -8,
  `Locked` -13, `Passphrase` -14, `NoPeers` -9, `InsufficientBalance` -6,
  anything else -4) and the message is the Go `err.Error()` text. Section
  8.3 gives the text format. Where the text is fully determined by code in
  this scope it is quoted; otherwise the owning spec is cited.
- "wallet loaded check" means: `s.walletLoader.LoadedWallet()`; if no wallet
  is loaded return `errUnloadedWallet` = RPC error `-4` `request requires a
  wallet but wallet has not loaded yet` (`errors.go:67-70`).
- "account lookup of `name`" means: `w.AccountNumber(ctx, name)`
  (`wallet/wallet.go:2083`, one read transaction on the address-manager
  name index). If the error is of kind `NotExist` return
  `errAccountNotFound` = RPC error `-11` `account not found`; any other
  error is returned as a wallet error. Account names are matched exactly
  (case-sensitive, no trimming). Exceptions to this mapping are stated.
- `w.NetworkBackend()` (`wallet/network.go:61-71`) fails with wallet error
  kind `NoPeers` and text `wallet.NetworkBackend: Decred network is
  unreachable`, i.e. the client sees `-9` with that message.
  `s.walletLoader.NetworkBackend()` (`internal/loader/loader.go:397-404`)
  returns `(backend, ok)` with `ok == false` when there is no wallet or no
  backend.
- "dcrd backend" means the backend's dynamic type is `*chain.Syncer`; "SPV
  backend" means `*spv.Syncer`. These are the only two implementations.
- Each wallet call runs in its own database transaction unless stated.
  Handlers hold no lock of their own.

### 5.1 Dispatch (summary; owned by spec 16 section 5.5)

`lazyApplyHandler(s, ctx, request)` (`methods.go:229-279`):

1. Look `request.Method` up in `handlers`.
2. Not found: return a closure that
   1. takes `s.walletLoader.NetworkBackend()`; not ok -> RPC error `-9`
      `disconnected from consensus RPC`;
   2. requires a dcrd backend, else RPC error `-9` `RPC passthrough requires
      dcrd RPC synchronization`;
   3. calls `chainSyncer.RPC().Call(ctx, method, &raw, params...)` with the
      request's raw parameters;
   4. if `ctx.Err() != nil` after the call: logs warn `Canceled RPC method
      <method> invoked by <remote addr>: <err>`, where `<err>` is the
      error returned by the dcrd call (Go `%v`; `<nil>` if the call
      itself succeeded), and returns RPC error `-1` with message
      `ctx.Err().Error()`;
   5. on any other call error returns `convertError(err)`;
   6. otherwise returns dcrd's raw result.
3. Found: return a closure that
   1. runs `dcrjson.ParseParams(types.Method(method), params)`; any error
      -> `dcrjson.ErrRPCInvalidRequest` (`-32600`, `Invalid request`);
   2. calls the handler;
   3. on return, if `ctx.Err() != nil`, logs the same warn line with
      `<err>` = `ctx.Err()` (the reply is unaffected);
   4. maps a handler error with `convertError`, else returns the result.

The eight dead entries have no registered command type, so step 3.1 always
fails for them and `unimplemented`/`unsupported` never run.

Details of the closures that a port has to keep:

- The lookup in step 1 happens when `lazyApplyHandler` is called; the
  backend lookup, parameter parsing and handler call happen when the
  returned closure runs. A method name is therefore classified as
  "wallet method" or "passthrough" once, before any parameter is looked
  at: a wallet method with bad parameters is `-32600` and is never
  forwarded to dcrd, even if dcrd has a method of the same name with a
  different signature.
- Passthrough forwards `request.Params` element by element
  (`methods.go:243-246`): each raw JSON parameter becomes one positional
  argument of the dcrd call, re-encoded as it was received (compacted).
  A request without parameters forwards an empty parameter list. The
  method name is forwarded unchanged; the wallet does not check that dcrd
  knows it, so for an unknown method the client receives whatever error
  object dcrd answers, with dcrd's code and message (`convertError` copies
  both from the `*wsrpc.Error`).
- In the passthrough closure the cancellation test (step 2.4) comes
  **before** the error test: a request whose context ended is answered
  `-1` with `context canceled` or `context deadline exceeded` even when
  dcrd had already returned a result or an error object.
- In the wallet-method closure the cancellation log is written by a
  deferred function that is registered only after parameter parsing
  succeeded; a parse failure logs nothing. The handler's result or error
  is returned unchanged whether or not the context ended.
- The closure's error return is the concrete type `*dcrjson.RPCError`;
  `convertError` never returns nil for a non-nil error.

`makeResponse(id, result, err)` (`methods.go:284-306`), used only by the
websocket transport for the malformed-request reply, the `authenticate`
reply and the `stop` reply (`server.go:378`, `408`, `429`):

1. `idPtr = idPointer(id)`.
2. `err != nil`: return `{ID: idPtr, Error: convertError(err)}` with no
   result (`"result":null` on the wire).
3. `json.Marshal(result)`; on failure return `{ID: idPtr, Error: {-32603,
   "Unexpected error marshalling result"}}`. The three callers pass `nil`
   or a constant string, so this message is never produced.
4. Otherwise return `{ID: idPtr, Result: <marshalled bytes>}`.

The `Jsonrpc` field of the returned struct is never assigned. The struct
(`dcrd/dcrjson/v4@v4.2.0/jsonrpc.go:118-123`) has no `omitempty`, so these
three replies are encoded with `"jsonrpc":""`, member order `jsonrpc`,
`result`, `error`, `id`, a nil `Result` as `null` and a nil id pointer as
`"id":null` (spec 16 section 4.2). Regular method replies are not built by
`makeResponse`.

### 5.2 Shared helpers

#### 5.2.1 `decodeAddress(s, params)` (`methods.go:1829-1851`)

1. If the byte length of `s` is exactly 66 or 130, treat `s` as a hex
   public key:
   1. `hex.DecodeString(s)`; on failure return the Go error unwrapped. The
      client sees `-4` with the text `encoding/hex: invalid byte: U+XXXX
      'c'`.
   2. `stdaddr.NewAddressPubKeyEcdsaSecp256k1V0Raw(bytes, params)`
      (`stdaddr/addressv0.go:129-161`); on failure return the error
      unwrapped (`-4`): `failed to parse public key: <secp256k1 error>` when
      the bytes are not a valid curve point, or `serialized public key <hex>
      is not a valid format` when the key parses but is not compressed.
      Every 130-character (uncompressed or hybrid) key therefore fails.
   3. Return the pay-to-pubkey address (Go type
      `*stdaddr.AddressPubKeyEcdsaSecp256k1V0`).
2. Otherwise `stdaddr.DecodeAddress(s, params)`. On failure return RPC
   error `-5` with message

   ```
   invalid address <Q(s)>: decode failed: <BQ(err text)>
   ```

   where `Q` is Go `%q` (double-quoted, Go escape sequences) and `BQ` is Go
   `%#q` applied to the error text: the text wrapped in backquotes when it
   contains no backquote and no control character other than tab, otherwise
   the `%q` form. The `stdaddr` texts are
   (`stdaddr/address.go:445-457`, `stdaddr/addressv0.go:1108-1233`):
   - `address "<s>" is not a supported type` when `s` is not 35 or 53
     bytes long, contains a character outside the Decred base58 alphabet,
     or decodes to a 2-byte prefix that is not one of the network's five
     address identifiers (this is what an address of another network
     produces);
   - `failed to decode address "<s>": checksum error`;
   - `failed to decode address "<s>": invalid format: version and/or
     checksum bytes missing`;
   - `public key is <n> bytes vs required <m> bytes`, `address "<s>"
     decoded data is empty`, and the constructor errors for malformed
     payload lengths.

   Shape of a complete message for a 35-character base58 string `A` whose
   checksum does not verify (`A` contains no character needing an escape):
   ``invalid address "A": decode failed: `failed to decode address "A": checksum error` ``.
3. Return the decoded address. All seven version-0 address kinds are
   accepted here (P2PK and P2PKH for ECDSA, Ed25519 and Schnorr; P2SH);
   callers restrict further where needed.

Order matters: the length test comes first, so a 66- or 130-character
string is never tried as base58.

#### 5.2.2 `decodeStakeAddress(s, params)` (`methods.go:1853-1863`)

1. `decodeAddress(s, params)`; propagate its error.
2. If the address implements `stdaddr.StakeAddress` return it. Only
   `AddressPubKeyHashEcdsaSecp256k1V0` and `AddressScriptHashV0` do
   (interface assertions at `stdaddr/addressv0.go:515`, `906`; the
   `VotingRightsScript` methods at `593`, `994`).
3. Otherwise RPC error `-5` `invalid stake address <Q(s)>`.

#### 5.2.3 `walletPubKeys(ctx, w, keys)` (`methods.go:393-426`)

For each element, in order, stopping at the first error:

1. `decodeAddress(key, w.ChainParams())`; propagate its error.
2. If the result is a pay-to-pubkey ECDSA address, use its 33-byte
   serialized compressed public key and continue with the next element.
3. Otherwise `w.KnownAddress(ctx, addr)` (`wallet/addresses.go:223`). Kind
   `NotExist` -> `errAddressNotInWallet` = RPC error `-4` `address not
   found in wallet`; other errors are wallet errors.
4. If the known address implements `wallet.PubKeyHashAddress` (any P2PKH
   address managed by the wallet, HD or imported) use `PubKey()`.
5. Otherwise (a P2SH address known to the wallet) RPC error `-5` `address
   has no associated public key`.

The result is one 33-byte key per input, in input order. No private key is
needed and the wallet may be locked.

#### 5.2.4 `difficultyRatio(bits, params)` (`methods.go:1708-1724`)

1. `max = CompactToBig(params.PowLimitBits)`, `target = CompactToBig(bits)`.
2. Form the exact rational `max / target` and render it with
   `big.Rat.FloatString(8)`: decimal with exactly 8 fraction digits,
   rounded half away from zero.
3. Parse that string with `strconv.ParseFloat(_, 64)` and return it. A
   parse failure logs error `Cannot get difficulty: <err>` and returns 0.

A zero `target` makes `SetFrac` panic (division by zero); no header
accepted by the wallet has zero bits. The dcroxide function
`get_difficulty_ratio` (`dcroxide:crates/dcroxide-rpc/src/helpers.rs:223`)
implements the same computation for dcrd.

#### 5.2.5 Block-level values shared by `getblockheader` and `getblock`

Given a block hash `H`, its header `hdr` and the wallet `w`:

- **Confirmations and next hash.** `w.BlockInMainChain(ctx, H)`
  (`wallet/wallet.go:807`); an error is RPC error `-32603` `Error checking
  if block is in mainchain: <err>`. If the block is not in the wallet's
  main chain: `confirmations = -1`, no next hash. If it is: let `(_,
  best) = w.MainChainTip(ctx)`; if `hdr.Height < best`, look up the main
  chain block at `hdr.Height + 1` with `w.BlockInfo` (failure: RPC error
  `-32603` `Info not found for next block: <err>`) and use its hash as the
  next hash; `confirmations = confirms(hdr.Height, best)`.
- **Median time.** Collect `hdr.Timestamp` (Unix seconds) and the
  timestamps of up to 10 ancestors by following `PrevBlock` through
  `w.BlockHeader`, stopping at height 0; a failed lookup is RPC error
  `-32603` `Info not found for previous block: <err>`. Sort ascending;
  the median is element `len/2` (the upper middle for an even count).
  `getblockheader` performs one extra ancestor lookup after collecting the
  eleventh timestamp when that header's height is not 0
  (`methods.go:1288-1297`); `getblock` does not (`methods.go:1412-1421`).
  The extra lookup changes nothing unless it fails.
- **PoW hash.** `p = hdr.PowHashV1()`; if `p != H` then `p =
  hdr.PowHashV2()`. (Before DCP0011 the block hash equals the v1 PoW hash.)
- **Difficulty.** `difficultyRatio(hdr.Bits, w.ChainParams())`.
- **Chain work.** Always the empty string.

### 5.3 Handlers

#### 5.3.1 `abandontransaction` (`methods.go:310-324`)

Parameters: `hash` string (required).

1. Wallet loaded check.
2. `chainhash.NewHashFromStr(hash)`; failure -> RPC error `-22` with the
   hash error text (section 4.2).
3. `w.AbandonTransaction(ctx, hash)` (`wallet/wallet.go:5073-5092`), one
   read-write transaction: look up the transaction; if it is mined return
   kind `Invalid`; otherwise remove it and every unmined transaction that
   depends on it, then emit the removed-transaction notification.
4. Return `null`, or the wallet error:
   - unknown hash: `-4` `wallet.AbandonTransaction(<hash>): item does not
     exist`;
   - mined: `-4` `wallet.AbandonTransaction(<hash>): invalid operation:
     transaction <hash> is mined in main chain`.

#### 5.3.2 `accountaddressindex` (`methods.go:328-355`)

Parameters: `account` string, `branch` int (both required).

1. Wallet loaded check.
2. Account lookup of `account`.
3. `w.BIP0044BranchNextIndexes(ctx, account)` (`wallet/addresses.go:827-840`,
   in-memory, under the address-buffers mutex): for each branch,
   `lastUsed + 1 + cursor` in wrapping uint32 arithmetic (`lastUsed` is
   `0xFFFFFFFF` when no child has been used). An account without address
   buffers (the `imported` account) fails with kind `NotExist`: client sees
   `-4` `wallet.BIP0044BranchNextIndexes: item does not exist: account
   2147483647`.
4. Switch on `uint32(branch)` (truncating conversion): `0` -> external
   index, `1` -> internal index, anything else -> RPC error `-8` `invalid
   branch <branch>` (the untruncated parameter in decimal).

Result: a JSON number (uint32). The branch is validated after the account,
so an unknown account with a bad branch reports the account error.

#### 5.3.3 `accountsyncaddressindex` (`methods.go:362-389`)

Parameters: `account` string, `branch` int, `index` int (all required).

1. Wallet loaded check.
2. Account lookup of `account`.
3. `branch32 = uint32(branch)`, `index32 = uint32(index)` (truncating).
4. If `index32 >= 0x80000000`: RPC error `-8` `child index <index32>
   exceeds the maximum child index for an account` (the truncated unsigned
   value; `-1` reports `4294967295`).
5. `w.SyncLastReturnedAddress(ctx, account, branch32, index32)`
   (`wallet/addresses.go:845-912`):
   1. Under the address-buffers mutex: unknown account -> kind `NotExist`;
      branch other than 0 or 1 -> kind `Invalid`, client sees `-4`
      `wallet.SyncLastReturnedAddress: invalid operation: branch must be
      external (0) or internal (1)`. If some child has been used
      (`lastUsed != 0xFFFFFFFF`) and `index32 > lastUsed`, set `cursor =
      index32 - lastUsed`; this may raise or lower the cursor, and makes
      `index32 + 1` the next child handed out. Otherwise the cursor is
      untouched.
   2. One read-write transaction: extend the address manager's derived
      range to `index32` and record `index32` as the last returned child.
   3. If a network backend is attached and `index32 > lastUsed + gapLimit`,
      derive the addresses from `lastUsed + 1 + gapLimit` for `index32 -
      (lastUsed + gapLimit)` children and add them to the backend's
      transaction filter.
6. Return `null` or the wallet error.

The handler performs no comparison against the current index: the
"already beyond the passed index" error promised by the function comment
(`methods.go:357-361`) does not exist, and the inline comment claiming the
RPC "no longer changes the child indexes that new addresses are derived
from" (`methods.go:385-387`) is contradicted by the cursor assignment in
step 5.1 (section 10.2).

#### 5.3.4 `addmultisigaddress` (`methods.go:430-457`)

Parameters: `nrequired` int, `keys` array of strings, `account` string
(optional, no default).

1. If `account` is non-nil and not exactly `imported`: RPC error `-4`
   `imported addresses must belong to the imported account`. **This check
   precedes the wallet loaded check.**
2. Wallet loaded check.
3. `walletPubKeys(ctx, w, keys)` (5.2.3); propagate its error.
4. `stdscript.MultiSigScriptV0(nrequired, keys...)`
   (`stdscript/scriptv0.go:822-849`); failure is a wallet error (`-4`) with
   text `unable to generate multisig script with <n> required signatures`
   (negative threshold) or `unable to generate multisig script with <n>
   required signatures when there are only <k> public keys available`. The
   script is `<nrequired> <key>... <len(keys)> OP_CHECKMULTISIG` with
   minimal integer pushes. A threshold of 0 and an empty key list are
   accepted.
5. `w.ImportScript(ctx, script)` (`wallet/wallet.go:3780-3805`): stores the
   script in the imported account, adds its P2SH address to the backend's
   transaction filter if a backend is attached, logs info `Imported script
   with P2SH address <addr>`. Kind `Exist` (already imported) is ignored;
   any other error is returned.
6. Return the value of `stdaddr.NewAddressScriptHashV0(script,
   w.ChainParams())`.

Result: the Go value returned is a `*stdaddr.AddressScriptHashV0`, a struct
with only unexported fields and no JSON or text marshaler, so
`encoding/json` renders it as the empty object: the result is **`{}`**, not
the address string (section 10.1 Q2). No rescan is started.

#### 5.3.5 `addtransaction` (`methods.go:459-479`)

Parameters: `blockhash` string, `transaction` string (hex), both required.

1. Wallet loaded check.
2. `chainhash.NewHashFromStr(blockhash)`; failure -> RPC error `-22` with
   the hash error text.
3. Deserialize the transaction by streaming `transaction` through a hex
   decoder into `wire.MsgTx.Deserialize`; failure -> RPC error `-22` with
   the Go error text (for example `EOF` for an empty string, `unexpected
   EOF` for a truncated transaction, `encoding/hex: invalid byte: U+XXXX
   'c'`). Characters after the end of a complete transaction are never
   read and are not validated.
4. `w.AddTransaction(ctx, tx, &blockHash)` (`wallet/chainntfns.go:324-436`):
   - returns nil at once if the wallet published this transaction itself
     recently;
   - if the block hash is not in the wallet's main chain the transaction
     is processed as **unmined**: every input that spends a wallet UTXO
     must name the correct tree (else kind `ScriptFailure`, `input <i>
     spends <outpoint> from the wrong transaction tree`) and carry a valid
     signature script;
   - if the block is in the main chain the transaction is recorded as
     mined in that block without checking that the block contains it;
   - new outpoints are added to the backend's filter when a backend is
     attached.
5. Return `null` or the wallet error.

#### 5.3.6 `auditreuse` (`methods.go:483-562`)

Parameters: `since` int32 (optional, no default; nil means 0).

1. Wallet loaded check.
2. `reuse`: map address string -> list of outpoint strings; `inRange`: set
   of address strings.
3. `w.GetTransactions(ctx, f, nil, nil)` (`wallet/wallet.go:3248`): one
   read transaction visiting every wallet transaction, one batch per mined
   block that contains wallet transactions in ascending height, followed
   by one batch of all unmined transactions (header nil)
   (`wallet/udb/txquery.go:521-537`). For each transaction in each batch:
   1. Skip it entirely if its type is vote or revocation.
   2. For each wallet-owned output (`MyOutputs`, ascending output index):
      append `<txhash>:<index>` to `reuse[address]`; if the batch is
      unmined or its block height is `>= since`, add the address to
      `inRange`.
   3. If the transaction is a ticket purchase: deserialize it (failure
      aborts the request with that error) and for each odd output index
      `i` (commitments): decode the commitment address
      (`stake.AddrFromSStxPkScrCommitment`; failure aborts with that
      error); if `w.HaveAddress(ctx, addr)` is true, append `<txhash>:<i>`
      to `reuse[address]` and apply the same `inRange` rule.
4. On error from step 3 return it as a wallet error.
5. Delete every entry with one or fewer outpoints, and every entry whose
   address is not in `inRange`.
6. Return the map.

Result: JSON object, keys (addresses) in ascending byte order, each value
an array of outpoint strings in visit order (so ascending block height,
unmined last). Empty result is `{}`.

Notes: `since` filters addresses, not outpoints: an address with at least
one referencing output at or after `since` (or unmined) is reported with
all of its outpoints. `w.HaveAddress` opens a nested read transaction per
commitment while the outer read transaction is open. `MyOutputs[].Address`
is a nil interface when the output script yields no address or the address
lookup fails (`wallet/notifications.go:89-112`); step 3.2 then dereferences
nil (section 10.1 Q3).

#### 5.3.7 `consolidate` (`methods.go:566-603`)

Parameters: `inputs` int (required), `account` string (optional), `address`
string (optional). No registered defaults.

1. Wallet loaded check.
2. `account` nil -> account number 0; otherwise account lookup.
3. `address` nil or the empty string -> no explicit destination; otherwise
   `decodeAddress(address, w.ChainParams())`, propagating its error.
4. `w.Consolidate(ctx, inputs, account, changeAddr)`
   (`wallet/wallet.go:1539`, `wallet/createtx.go:769-825`, spec 10):
   requires a network backend (kind `NoPeers`, `-9`), uses minimum
   confirmations 1, fails with `-4` `wallet.Consolidate: too few outputs to
   consolidate` when at most one output is eligible, and when no address
   was given derives a new **internal-branch (change)** address of the
   account with the gap policy "ignore".
5. Return the transaction hash string.

#### 5.3.8 `createmultisig` (`methods.go:607-632`)

Parameters: `nrequired` int, `keys` array of strings (required).

1. Wallet loaded check.
2. `walletPubKeys` (5.2.3).
3. `stdscript.MultiSigScriptV0(nrequired, keys...)`; errors as in 5.3.4
   step 4.
4. `stdaddr.NewAddressScriptHashV0(script, params)`; propagate an error.
5. Return `CreateMultiSigResult{address: addr.String(), redeemScript:
   hex(script)}`.

Nothing is stored.

#### 5.3.9 `createrawtransaction` (`methods.go:635-742`)

Parameters: `inputs` array of `TransactionInput` (required), `amounts`
object address -> number (required), `locktime` int64 (optional), `expiry`
int64 (optional). No wallet is required; the network is `s.activeNet`.

1. If `expiry` is given and negative: RPC error `-8` `Expiry out of range`.
   There is no upper bound; the value is later truncated to uint32.
2. If `locktime` is given and negative or greater than `0xFFFFFFFF`: RPC
   error `-8` `Locktime out of range`.
3. Start a transaction with version 1, full serialization type.
4. For each input, in array order:
   1. `chainhash.NewHashFromStr(txid)`; failure -> RPC error `-8` with the
      hash error text.
   2. `tree` must be 0 or 1, else RPC error `-8` `Tx tree must be regular
      or stake`.
   3. `dcrutil.NewAmount(amount)`; failure -> RPC error `-8` `invalid coin
      amount` (not reachable from JSON).
   4. Amount negative -> RPC error `-8` `Positive input amount is required`.
      Zero (including an omitted `amount`) is accepted.
   5. Append an input: previous outpoint `(hash, vout, tree)`, `ValueIn =
      amount in atoms`, `BlockHeight = 0`, `BlockIndex = 0xFFFFFFFF`, empty
      signature script, `Sequence = 0xFFFFFFFF`, or `0xFFFFFFFE` when
      `locktime` is given and non-zero.
5. For each `(address, amount)` pair of `amounts`, **in Go map iteration
   order (unspecified, varies between calls)**:
   1. `stdaddr.DecodeAddress(address, s.activeNet)`; failure -> RPC error
      `-5` `Address <Q(address)>: <err text>` (no `decodeAddress`, so hex
      public keys are not accepted here).
   2. The address must be P2PKH-ECDSA-secp256k1 or P2SH; otherwise RPC
      error `-5` `Invalid type: <Go type>` where the Go type is one of
      `*stdaddr.AddressPubKeyEcdsaSecp256k1V0`,
      `*stdaddr.AddressPubKeyEd25519V0`,
      `*stdaddr.AddressPubKeySchnorrSecp256k1V0`,
      `*stdaddr.AddressPubKeyHashEd25519V0`,
      `*stdaddr.AddressPubKeyHashSchnorrSecp256k1V0`.
   3. `dcrutil.NewAmount(amount)`; failure -> RPC error `-32603` `New
      amount: invalid coin amount` (not reachable from JSON).
   4. If the amount is `<= 0` or `> 21e14` atoms: RPC error `-8` `Amount
      outside valid range: <amount>` where the amount is formatted by
      `Amount.String()`: shortest decimal DCR value in fixed notation
      followed by ` DCR` (for example `0 DCR`, `-1.5 DCR`, `21000001 DCR`).
   5. Append an output `{Value, Version, PkScript}` from the address's
      payment script.
6. If `locktime` is given set `LockTime = uint32(locktime)`; if `expiry` is
   given set `Expiry = uint32(expiry)` (truncating).
7. Return the hex of `MsgTx.Serialize` (prefix and witness; unsigned inputs
   carry the value-in, null fraud-proof fields and an empty script).

Because of step 5, the order of outputs in the result, and which error is
reported when several entries are invalid, are not deterministic in Go.
Duplicate addresses in the JSON object collapse to the last value before
the handler runs.

#### 5.3.10 `createsignature` (`methods.go:747-796`)

Parameters, in positional order: `address` string, `inputindex` int,
`hashtype` int, `previouspkscript` string (hex), `serializedtransaction`
string (hex). All required.

1. Wallet loaded check.
2. `hex.DecodeString(serializedtransaction)`; failure -> RPC error `-22`
   with the Go text (`encoding/hex: odd length hex string` or
   `encoding/hex: invalid byte: U+XXXX 'c'`).
3. `MsgTx.Deserialize`; failure -> RPC error `-22` with the Go text.
   Trailing bytes are ignored.
4. `inputindex < 0` -> RPC error `-8` `transaction input index cannot be
   negative`.
5. `inputindex >= len(tx.TxIn)` -> RPC error `-1` `transaction input
   <inputindex> does not exist`.
6. `decodeAddress(address, params)`; propagate.
7. `hashType = SigHashType(hashtype)` (Go conversion of the int to the
   8-bit type: only the low 8 bits are kept; no validation).
8. `hex.DecodeString(previouspkscript)`; failure -> RPC error `-22`.
9. `w.CreateSignature(ctx, &tx, inputindex, addr, hashType, script)`
   (`wallet/wallet.go:4914-4948`): loads the private key for the address
   through `udb.Manager.PrivateKey` (spec 02; a locked wallet or account
   is kind `Locked`, `-13`; a P2SH address is kind `Invalid`, `-4`, `no
   private key for P2SH address`; an unknown address is `-4`), then
   `sign.RawTxInSignature` with ECDSA secp256k1. The signature is the DER
   signature followed by the one-byte hash type.
10. Return `CreateSignatureResult{signature: hex(sig), publickey:
    hex(33-byte compressed key)}`.

#### 5.3.11 `debuglevel` (`methods.go:798-813`)

Parameters: `levelspec` string (required). No wallet required. Specified
in spec 16 section 5.7: `"show"` returns `Supported subsystems [AMGR CMGR
DCRW GRPC LODR MIXC MIXP PEER RPCS SYNC TKBY VSPC WLLT]` (Go `%v` of the
sorted subsystem names, `rpcserver.go:235-242`, `log.go:47-61`); otherwise
`Loggers.SetLevels(levelspec)`, RPC error `-8` `invalid debug level
<levelspec>: <err>` on failure, the string `Done.` on success.

#### 5.3.12 `disapprovepercent`, `discoverusage`, `dumpprivkey`

**`disapprovepercent`** (`methods.go:816-822`). No parameters. Wallet
loaded check; return `w.DisapprovePercent()` (atomic uint32 load,
`wallet/wallet.go:207`) as a JSON number.

**`discoverusage`** (`methods.go:824-853`). Parameters, all optional and
without registered defaults: `startblock` string, `discoveraccounts` bool,
`gaplimit` uint32.

1. Wallet loaded check.
2. `s.walletLoader.NetworkBackend()`; not ok -> RPC error `-9`
   `disconnected from network`.
3. Start block: the network's genesis hash, or `NewHashFromStr(startblock)`
   when given; failure -> RPC error `-8` `startblock: <hash error text>`.
4. `discoverAccounts = discoveraccounts != nil && *discoveraccounts`.
5. Gap limit: `w.GapLimit()` or the given value (0 is passed through).
6. `w.DiscoverActiveAddresses(ctx, n, &startBlock, discoverAccounts,
   gapLimit)` (`wallet/discovery.go:716`, spec 09). The call blocks until
   discovery finishes or the request context is cancelled. Account
   discovery needs the coin-type private key: a locked wallet fails with
   kind `Locked` (`-13`).
7. Return `null` or the wallet error.

**`dumpprivkey`** (`methods.go:858-878`). Parameter: `address` string.

1. Wallet loaded check.
2. `decodeAddress`; propagate.
3. `w.DumpWIFPrivateKey(ctx, addr)` (`wallet/wallet.go:3688-3701`). Kind
   `Locked` -> `errWalletUnlockNeeded` = RPC error `-13` `wallet or account
   locked; use walletpassphrase or unlockaccount first`. Other errors are
   wallet errors (`-4` for an unknown address or a P2SH address).
4. Return the WIF string (network private-key prefix, ECDSA secp256k1
   signature type).

#### 5.3.13 `fundrawtransaction` (`methods.go:880-971`)

Parameters: `hexstring` string, `fundaccount` string (required), `options`
object (optional, section 4.9).

1. Wallet loaded check.
2. Defaults: change address `""`, fee rate `w.RelayFee()` (atoms per kB),
   `confs = 1`.
3. If `options` is non-nil:
   - `changeaddress` non-nil -> use it (an empty string means none);
   - `feerate` non-nil -> `dcrutil.NewAmount(feerate)` (DCR per kB);
     failure is a wallet error `-4` `invalid coin amount`. Zero and
     negative rates are not rejected here;
   - `conf_target` non-nil -> `confs`; negative -> wallet error `-4` `confs
     must be non-negative`.
4. Deserialize `hexstring` through a streaming hex decoder; failure is a
   **wallet error `-4`** with the Go text (not `-22`).
5. If the transaction has any input: wallet error `-4` `transaction must
   not already have inputs`.
6. `w.AccountNumber(ctx, fundaccount)`; any error is returned **without**
   the `errAccountNotFound` mapping: an unknown account is `-4`
   `wallet.AccountNumber: item does not exist: no account "<name>"`.
7. If the change address is non-empty: `stdaddr.DecodeAddress(address,
   params)` (`makeScriptChangeSource`, `methods.go:5220-5231`); failure is
   a wallet error `-4` with the `stdaddr` text. The change source is the
   address's payment script and version; any of the seven address kinds is
   accepted.
8. `w.NewUnsignedTransaction(ctx, tx.TxOut, feeRate, account, confs,
   OutputSelectionAlgorithmDefault, changeSource, nil)`
   (`wallet/createtx.go:82-161`, spec 10). With no explicit change source
   and a change output being needed, the wallet derives a new internal
   address of the funding account and persists the advanced address index
   even though nothing is published (no address is derived when the
   authored transaction has no change output).
   Insufficient funds is kind `InsufficientBalance` (`-6`).
9. Replace the decoded transaction's inputs with the authored inputs. If
   the authored transaction has a change output (`ChangeIndex >= 0`),
   **append** that output after the caller's outputs (the caller's output
   order is preserved; the authored transaction's own output order is
   discarded).
10. `fee = authored.TotalInput - sum(outputs of the result)`.
11. Return `FundRawTransactionResult{hex: hex(serialize(tx)), fee:
    fee.ToCoin()}`.

The transaction's version, lock time and expiry are those of the input
hex. Selected outputs are not locked.

#### 5.3.14 `getaddressesbyaccount` (`methods.go:976-1042`)

Parameter: `account` string.

1. Wallet loaded check.
2. If `account == "imported"`: `w.ImportedAddresses(ctx, account)`
   (`wallet/addresses.go:915-942`); return the addresses' strings as a JSON
   array in the iteration order of the imported account's address index
   (ascending byte order of the index keys, spec 02). Errors are wallet
   errors.
3. Otherwise account lookup.
4. `w.AccountXpub(ctx, account)`; derive the external branch key (child 0)
   and the internal branch key (child 1); errors are wallet errors.
5. `(endExt, endInt) = w.BIP0044BranchNextIndexes(ctx, account)`.
6. For `i` in `0..endExt` on the external branch, then for `i` in
   `0..endInt` on the internal branch: derive child `i`; skip the index if
   derivation reports `hdkeychain.ErrInvalidChild`; otherwise append the
   P2PKH-ECDSA address of `Hash160(compressed pubkey)`.
7. Return the array.

Result: JSON array of strings written without escaping (spec 16 section
4.7); `[]` when empty, never `null`. The list covers every child below the
next index to be handed out (used and already-returned addresses), not the
gap-limit lookahead. The cost is one public derivation per address.

#### 5.3.15 `getbalance` (`methods.go:1047-1161`)

Parameters: `account` string (optional, no registered default), `minconf`
int (optional, registered default 1).

1. Wallet loaded check.
2. `minConf = 1`, or `int32(minconf)` when non-nil (truncating). Negative
   -> RPC error `-8` `minconf must be non-negative`.
3. `accountName = "*"`, or the given value.
4. `blockhash = w.MainChainTip(ctx)` hash (its own read transaction; a
   failed read silently yields the zero hash).
5. If `accountName == "*"`:
   1. `w.AccountBalances(ctx, minConf)` (`wallet/wallet.go:1927-1946`): one
      read transaction, one `Balances` per account in the iteration order
      of the account bucket. That order is the **byte order of the 4-byte
      little-endian account number** (`wallet/udb/addressdb.go:698-708`),
      not numeric order: account 0, then `0x80000000` (first imported-xpub
      account, if any), then 256, 512, … (if they exist), then 1,
      `0x80000001`, 257, …, with the `imported` account (`0x7FFFFFFF`,
      bytes `FF FF FF 7F`) last.
   2. For each entry, in that order: `w.AccountName(ctx, number)` (a
      separate read transaction each); kind `NotExist` -> RPC error
      `-32603` with the error text, other errors are wallet errors. Add
      each amount to the running integer totals and append a
      `GetAccountBalanceResult`.
   3. Set the seven totals from the integer sums.
6. Otherwise: account lookup; `w.AccountBalance(ctx, account, minConf)`
   (kind `NotExist` -> RPC error `-32603` with the text; other errors are
   wallet errors); `balances` is a one-element array whose `accountname`
   is the **requested name string**; the totals stay zero.
7. Return the `GetBalanceResult`.

Result notes: totals are `omitempty`, so they are absent in the
single-account form and absent individually in the `*` form whenever the
sum is zero. `balances` is always an array. The balance fields come from
`udb.Store.AccountBalance` (spec 04): `Total`, `Spendable`,
`ImmatureCoinbaseRewards`, `ImmatureStakeGeneration`, `LockedByTickets`,
`VotingAuthority`, `Unconfirmed`.

#### 5.3.16 `getbestblock`, `getbestblockhash`, `getblockcount`, `getblockhash`

All four require a loaded wallet and read only the wallet database; they
describe the wallet's own main-chain tip, never dcrd's, and are never
forwarded.

- **`getbestblock`** (`methods.go:1165-1177`): `{hash, height}` from
  `w.MainChainTip(ctx)`.
- **`getbestblockhash`** (`methods.go:1181-1189`): the tip hash string.
- **`getblockcount`** (`methods.go:1193-1201`): the tip height as a JSON
  number (int32).
- **`getblockhash`** (`methods.go:1205-1219`), parameter `index` int64:
  `height = int32(index)` (truncating), `w.BlockInfo(ctx, height)`
  (`wallet/wallet.go:2752-2791`); return the hash string. A height with no
  main-chain block is the wallet error `-4` `wallet.BlockInfo: item does
  not exist: no block at height <height> in main chain` (the truncated
  height).

#### 5.3.17 `getblockheader` (`methods.go:1222-1337`)

Parameters: `hash` string (required), `verbose` bool (optional, registered
default `true`).

1. Wallet loaded check.
2. `w.NetworkBackend()`; failure is the `-9` wallet error.
3. **dcrd backend:** call dcrd `getblockheader` with parameters `[hash,
   verbose]` (the string exactly as received; `verbose` as `true`, `false`
   or `null` when the client sent an explicit `null`) and return dcrd's
   raw result. A dcrd error object is returned with its code and message;
   a transport error is a wallet error.
4. **SPV backend:**
   1. `NewHashFromStr(hash)`; failure -> RPC error `-22`.
   2. `w.BlockHeader(ctx, hash)`; an unknown block is the wallet error `-4`
      `item does not exist: block header`.
   3. If `verbose` is nil or false: return the hex of the 180-byte
      serialized header. (A serialization failure would be RPC error
      `-32603` `Could not serialize block header: <err>`.)
   4. Otherwise compute the values of 5.2.5 and return
      `GetBlockHeaderVerboseResult`:

| Member | Value |
|---|---|
| `hash` | canonical hash string (not the input string) |
| `powhash` | 5.2.5 |
| `confirmations` | 5.2.5 (`-1` off the main chain) |
| `version` | header version (int32) |
| `merkleroot`, `stakeroot` | hash strings |
| `votebits`, `voters`, `freshstake`, `revocations`, `poolsize` | header fields |
| `finalstate` | hex of the 6 bytes |
| `bits` | hex without leading zeros |
| `sbits` | `Amount(SBits).ToCoin()` |
| `height`, `size` | header fields (uint32) |
| `time` | header timestamp, Unix seconds |
| `mediantime` | 5.2.5 |
| `nonce` | header field |
| `extradata` | hex of the 32 bytes |
| `stakeversion` | header field |
| `difficulty` | 5.2.5 |
| `chainwork` | `""` |
| `previousblockhash` | previous block hash string (the all-zero hash string for genesis; never omitted) |
| `nextblockhash` | 5.2.5; omitted when empty |

#### 5.3.18 `getblock` (`methods.go:1340-1505`)

Parameters: `hash` string (required), `verbose` bool (optional, default
`true`), `verbosetx` bool (optional, default `false`).

1. Wallet loaded check.
2. `w.NetworkBackend()`; failure is the `-9` wallet error.
3. **dcrd backend:** call dcrd `getblock` with `[hash, verbose, verbosetx]`
   and return the raw result (same rules as 5.3.17 step 3).
4. **SPV backend:**
   1. `NewHashFromStr(hash)`; failure -> RPC error `-22`.
   2. `n.Blocks(ctx, [hash])` (`spv/backend.go:95-111`, spec 14): fetch the
      block from any connected peer, retrying with another pick on every
      per-peer failure, until success or until the request context is
      done. A hash no peer serves therefore does not produce an error
      reply; the request ends when the client disconnects or the server
      shuts down (context error text as a wallet error). The block's
      merkle roots are verified against its header by the SPV layer.
   3. Zero blocks returned -> RPC error `-32603` `Network returned 0
      blocks` (defensive).
   4. If `verbose` is nil or false: return the hex of the wire-serialized
      block. (Failure: `-32603` `Could not serialize block: <err>`.)
   5. Compute the values of 5.2.5 from the block's header.
   6. `isTreasuryEnabled = block.Transactions[0].Version >= 3`.
   7. Build `GetBlockVerboseResult`:

| Member | Value |
|---|---|
| `hash` | **the `hash` parameter string exactly as sent** |
| `powhash`, `confirmations`, `mediantime`, `difficulty`, `nextblockhash` | 5.2.5 |
| `size` | `int32(header.Size)` |
| `height` | `int64(header.Height)` |
| `version`, `merkleroot`, `stakeroot`, `time`, `nonce`, `votebits`, `finalstate`, `voters`, `freshstake`, `revocations`, `poolsize`, `bits`, `extradata`, `stakeversion` | as in 5.3.17 |
| `sbits` | `float64(SBits) / 1e8` |
| `chainwork` | `""` |
| `previousblockhash` | previous hash string, always present |
| `tx`, `stx` | when `verbosetx` is nil or false: the transaction hash strings of the regular and stake trees. `stx` is omitted for a block without stake transactions. |
| `rawtx`, `rawstx` | when `verbosetx` is true: `createTxRawResult` (4.6) of every regular and stake transaction, with `blkIdx` the index inside the respective tree and the block's confirmations. `rawstx` is omitted when empty. |

The SPV path does not require the wallet to know the requested block: any
block a peer serves is accepted. For such a block `BlockInMainChain` is
false (`confirmations` `-1`, no `nextblockhash`), and the median-time walk
needs the wallet to hold the headers of up to 10 ancestors; the first
missing one ends the request with `-32603` `Info not found for previous
block: item does not exist: block header`.

#### 5.3.19 `syncstatus` (`methods.go:1727-1761`)

No parameters.

1. Wallet loaded check.
2. `w.NetworkBackend()`; failure is the `-9` wallet error.
3. `(tipHash, tipHeight) = w.MainChainTip(ctx)`; `w.BlockInfo(ctx,
   tipHash)`; failure is a wallet error.
4. `initialblockdownload = tipBlock.Timestamp < now - 24h`, with `now` the
   wall clock in Unix seconds and the block timestamp taken from the
   stored header.
5. `(synced, targetHeight) = n.Synced(ctx)`:
   - dcrd backend (`chain/sync.go:123-132`): `synced` is the syncer's
     atomic flag; `targetHeight` is the syncer's recorded target sync
     height while unsynced, and the wallet tip height once synced;
   - SPV backend (`spv/sync.go:200-215`): `synced` is the syncer's atomic
     flag; `targetHeight` is the largest height advertised in the version
     message of any connected peer while unsynced (0 with no peers), and
     the wallet tip height once synced.
6. `blocksToFetch = targetHeight - tipHeight` (int32). If `<= 0`,
   `headersfetchprogress = 1`. Otherwise `total = targetHeight -
   w.InitialHeight()` (the wallet's tip height when the wallet was opened,
   `wallet/wallet.go:786-788`) and `headersfetchprogress = 1 -
   float32(blocksToFetch) / float32(total)` in float32 arithmetic.
7. Return `SyncStatusResult{synced, initialblockdownload,
   headersfetchprogress}`.

If `total` is 0 while `blocksToFetch > 0` (the tip was reorganised below
the height at open) the quotient is `+Inf` and the result `-Inf`, which
`encoding/json` cannot encode: over HTTP POST the client receives HTTP 500
`500 Internal Server Error`, over websocket no reply at all (spec 16
sections 5.3 and 5.4.3). A negative `total` yields a value above 1.

#### 5.3.20 `getcurrentnet` (`methods.go:1764-1766`)

No parameters, no wallet required. Returns `s.activeNet.Net` as a JSON
number: `3652452601` (mainnet, `0xd9b400f9`), `2979310197` (testnet3,
`0xb194aa75`), `303307798` (simnet, `0x12141c16`)
(`dcrd/wire@v1.8.0/protocol.go:130-146`).

#### 5.3.21 `getinfo` (`methods.go:1770-1827`)

No parameters.

1. Wallet loaded check.
2. `(tipHash, tipHeight) = w.MainChainTip(ctx)`; `w.BlockHeader(ctx,
   tipHash)`; failure is a wallet error.
3. `w.AccountBalances(ctx, 1)`; sum `Spendable` over all accounts.
4. Build `InfoResult`:

| Member | Wallet value | Overwritten from dcrd `getinfo` |
|---|---|---|
| `version` | `version.Integer` = `1000000*major + 10000*minor + 100*patch` = `2020000` at the pin | yes |
| `protocolversion` | `p2p.Pver` = 11 (`p2p/peering.go:48`, `wire.BatchedCFiltersV2Version`) | yes |
| `walletversion` | `version.Integer` | no |
| `balance` | summed spendable balance, 1 confirmation, in DCR | no |
| `blocks` | wallet tip height | no |
| `timeoffset` | 0 | yes |
| `connections` | 0 | yes |
| `proxy` | `""` | yes |
| `difficulty` | `difficultyRatio(tipHeader.Bits)` | no |
| `testnet` | `params.Net == wire.TestNet3` | no |
| `keypoololdest` | 0 | no |
| `keypoolsize` | 0 | no |
| `unlocked_until` | 0 | no |
| `paytxfee` | `w.RelayFee().ToCoin()` (DCR per kB) | no |
| `relayfee` | 0 | yes |
| `errors` | `""` | yes |

5. `s.walletLoader.NetworkBackend()` (errors ignored). With a dcrd backend,
   call dcrd `getinfo` with no parameters, decode into `InfoChainResult`
   and copy the seven "yes" members; a call error fails the whole request
   (dcrd error object passed through, or wallet error).
6. Return the result.

In SPV mode `connections` is 0 regardless of the number of peers, and
`version`/`protocolversion` describe the wallet.

#### 5.3.22 `getaccount`, `getaccountaddress`

**`getaccount`** (`methods.go:1867-1888`). Parameter: `address` string.

1. Wallet loaded check.
2. `decodeAddress`; propagate.
3. `w.KnownAddress(ctx, addr)`; kind `NotExist` -> RPC error `-4` `address
   not found in wallet`; other errors are wallet errors.
4. Return the account name string.

**`getaccountaddress`** (`methods.go:1896-1920`). Parameter: `account`
string.

1. Wallet loaded check.
2. Account lookup.
3. `w.CurrentAddress(account)` (`wallet/wallet.go:1952-1973`): in memory,
   derive external child `lastUsed + 1 + cursor` and return its P2PKH
   address. Kind `NotExist` (account without address buffers, i.e.
   `imported`) -> RPC error `-32603` `wallet.CurrentAddress: item does not
   exist: no account 2147483647`. Other errors are wallet errors.
4. Return the address string.

No address is generated, persisted or added to the backend filter, and the
gap limit is not consulted: the method reports the address the next
`getnewaddress` would return (before any gap-policy wrap).

#### 5.3.23 `getunconfirmedbalance` (`methods.go:1924-1952`)

Parameter: `account` string (optional; nil means `default`).

1. Wallet loaded check.
2. Account lookup of the name.
3. `w.AccountBalance(ctx, account, 1)`; kind `NotExist` -> RPC error
   `-32603` with the text; other errors are wallet errors.
4. Return `(Total - Spendable).ToCoin()`.

The value is everything in the account that is not spendable at one
confirmation: unconfirmed outputs, but also immature coinbase and stake
returns and funds locked by tickets (section 10.2).

#### 5.3.24 `getcfilterv2`, `importcfiltersv2`

**`getcfilterv2`** (`methods.go:1955-1977`). Parameter: `blockhash` string.

1. `NewHashFromStr(blockhash)`; failure -> RPC error `-22`. **This
   precedes the wallet loaded check.**
2. Wallet loaded check.
3. `w.CFilterV2(ctx, hash)` (`wallet/wallet.go:833-842`,
   `wallet/udb/cfilters.go:17-30`): the stored 16-byte key and the stored
   filter bytes for the block. A block without a stored filter is the
   wallet error `-4` `item does not exist: no cfilter saved for block
   <hash>`.
4. Return `GetCFilterV2Result{blockhash: <the parameter string as sent>,
   filter: hex(filter bytes), key: hex(16-byte key)}`.

The filter bytes are the serialized version-2 GCS filter (varint item
count followed by the Golomb-coded data; an empty filter is the empty
string). The key is the stored 16-byte key; `importcfiltersv2` and
`blockcf2.Key` both define it as the first 16 bytes of the block's merkle
root. The method is answered from the wallet database in both backend modes and is
never forwarded to dcrd; its result shape differs from dcrd's method of
the same name.

**`importcfiltersv2`** (`methods.go:1981-2004`). Parameters: `startheight`
int32, `filters` array of hex strings (both required).

1. Wallet loaded check.
2. Hex-decode each filter in order; the first failure -> RPC error
   `-32602` `filter <i> is not a valid hex string` (zero-based index).
3. `w.ImportCFiltersV2(ctx, startheight, filterData)`
   (`wallet/wallet.go:5278-5287`, `wallet/udb/cfilters.go:34-66`): one
   read-write transaction; filter `i` is stored for the `i`-th main-chain
   block at or after height `startheight` (block records are contiguous,
   so height `startheight + i`), under that block's hash, together with a
   16-byte filter key taken from the first 16 bytes of the block header's
   merkle root. For each filter the block lookup comes first (failure:
   kind `NotExist`), then the blob must parse as a version-2 filter
   (`gcs.FromBytesV2`; an empty blob is accepted). Any failure rolls the whole
   import back and is returned as **RPC error `-32600`** with the wallet
   error text, for example `wallet.ImportCFiltersV2: item does not exist:
   block height of <h> unknown`.
4. Return `null`.

An empty `filters` array succeeds without writing anything.

#### 5.3.25 `importprivkey`, `importpubkey`, `importscript`

Common prefix (`methods.go:2015-2026`, `2080-2091`, `2135-2146`):

1. Wallet loaded check.
2. `rescan = true` unless the `rescan` parameter is non-nil; `scanFrom =
   0` unless `scanfrom` is non-nil, then `int32(scanfrom)` (truncating).
3. `(n, ok) = s.walletLoader.NetworkBackend()`. If `rescan && !ok`: RPC
   error `-9` `disconnected from network`.

Common suffix, "background rescan" (`methods.go:2054-2064`): if `rescan`,
add 1 to the server wait group and start a goroutine that runs
`w.RescanFromHeight(serverCtx, n, scanFrom)` (`wallet/rescan.go:386-406`)
with the **server's base context** (not the request context) and discards
its error; the handler returns without waiting. A `scanFrom` with no
main-chain block makes the rescan fail silently.

**`importprivkey`** (`methods.go:2008-2067`). Parameters: `privkey` string
(required), `label` string (optional), `rescan` bool (optional, default
`true`), `scanfrom` int (optional).

4. If `label` is non-nil and not exactly `imported`: RPC error `-4`
   `imported addresses must belong to the imported account`.
5. `dcrutil.DecodeWIF(privkey, params.PrivateKeyID)`
   (`dcrd/dcrutil/v4@v4.0.3/wif.go:101-162`); failure -> RPC error `-5`
   `WIF decode failed: <err>` with `<err>` one of: `malformed private key`
   (more than 54 characters, or not 39 bytes after base58 decoding);
   `checksum mismatch`; `WIF is not for the network identified by 0xNNNN`
   (the **expected** network's 2-byte private-key prefix in lowercase
   hex, for example `0x22de` on mainnet). The decoder also accepts
   Ed25519 and Schnorr-secp256k1 WIFs, and returns an empty key without
   error for an unknown signature-type byte; what the address manager does
   with those is outside this scope (section 13).
6. `w.ImportPrivateKey(ctx, wif)` (`wallet/wallet.go:3705-3738`): stores
   the key in the imported account, adds the address to the backend
   filter when a backend is attached, logs info `Imported payment address
   <addr>`, emits an account notification. Kind `Exist` -> return `null`
   **without rescanning**. Kind `Locked` -> RPC error `-13` `wallet or
   account locked; use walletpassphrase or unlockaccount first`. Other
   errors are wallet errors.
7. Background rescan; return `null`.

**`importpubkey`** (`methods.go:2073-2125`). Parameters: `pubkey` string
(hex), `label`, `rescan`, `scanfrom` as above.

4. Label check as above.
5. `hex.DecodeString(pubkey)`; failure -> RPC error `-22`.
6. `w.ImportPublicKey(ctx, pk)` (`wallet/wallet.go:3742-3775`,
   `wallet/udb/addressmanager.go:1297-1329`). Kind `Exist` -> `null`
   without rescanning. Other errors are wallet errors:
   - wallet has private keys (not watching-only): kind `Invalid`, `-4`,
     `wallet.ImportPublicKey: invalid operation: public keys may only be
     imported by watching-only wallets`;
   - length not 33: kind `Encoding`, **`-8`**, `wallet.ImportPublicKey:
     invalid encoding: invalid length for compressed pubkey`;
   - first byte not `0x02`/`0x03`: kind `Encoding`, `-8`,
     `wallet.ImportPublicKey: invalid encoding: invalid format byte for
     compressed pubkey`.
   The key is not checked to be on the curve at this layer.
7. Background rescan; return `null`.

**`importscript`** (`methods.go:2128-2177`). Parameters: `hex` string
(required), `rescan` bool (optional, default `true`), `scanfrom` int
(optional).

4. `hex.DecodeString(hex)`; failure -> RPC error `-22`.
5. Empty script -> RPC error `-8` `empty script`.
6. `w.ImportScript(ctx, script)` (5.3.4 step 5). Kind `Exist` -> `null`
   without rescanning; other errors are wallet errors.
7. Background rescan; return `null`.

In all three the network check (step 3) precedes input validation, so a
malformed key with `rescan` in effect and no backend reports `-9`.

#### 5.3.26 `importxpub` (`methods.go:2179-2192`)

Parameters: `name` string, `xpub` string (both required).

1. Wallet loaded check.
2. `hdkeychain.NewKeyFromString(xpub, params)`; failure is a wallet error
   `-4` with the hdkeychain text (`the provided serialized extended key
   length is invalid`, `bad extended key checksum`, `the provided
   serialized extended key is for the wrong network`, `unusable seed` for a
   private key outside the curve order, or the secp256k1 parse error for an
   invalid public key; `dcrd/hdkeychain/v3@v3.1.3/extendedkey.go:538-610`. The
   base58 decoder itself never reports an error: an undecodable string
   surfaces as the length error).
3. `w.ImportXpubAccount(ctx, name, key)` (`wallet/wallet.go:3893-3951`):
   rejects a private extended key (`-4` `wallet.ImportXpubAccount: extended
   key must be an xpub`); if a backend is attached, loads the first
   `gapLimit` external and internal addresses into its filter **before**
   the database write; creates the account (kind `Exist` `account name in
   use`, kind `Invalid` `exhausted possible imported accounts`); installs
   in-memory address buffers.
4. Return `null` or the wallet error.

No rescan is started.

#### 5.3.27 `createnewaccount`, `renameaccount`

**`createnewaccount`** (`methods.go:2197-2218`). Parameter: `account`
string.

1. Wallet loaded check.
2. `account == "*"` -> RPC error `-8` `account name is reserved by RPC
   server`.
3. `w.NextAccount(ctx, account)` (`wallet/wallet.go:2139-2239`). Kind
   `Locked` -> RPC error `-13` `creating new accounts requires an unlocked
   wallet`. Other errors are wallet errors, including:
   - `-4` `wallet.NextAccount: last <N> accounts have no transaction
     history` (`N` is the configured account gap limit, option
     `--accountgaplimit`); this check runs before the locked and
     watching-only checks;
   - kind `Exist`: `account named "<name>" already exists`;
   - kind `Invalid`: `accounts may not be named the empty string`,
     `reserved account name`;
   - kind `WatchingOnly`.
4. Return `null`. The new account number is discarded.

Side effects on success: account row and gap-limit address rows written,
in-memory address buffers installed, `2 * gapLimit` addresses loaded into
the backend filter when a backend is attached (a failure there fails the
request although the account already exists), account notification.

**`renameaccount`** (`methods.go:2222-2245`). Parameters: `oldaccount`,
`newaccount` strings.

1. Wallet loaded check.
2. `newaccount == "*"` -> RPC error `-8` `account name is reserved by RPC
   server`.
3. Account lookup of `oldaccount`.
4. `w.RenameAccount(ctx, account, newaccount)` (`wallet/wallet.go:2115-2132`);
   errors are wallet errors (kind `Invalid` `reserved account` for the
   imported account, kind `Exist` for a name in use, the name validation
   errors above).
5. Return `null`.

#### 5.3.28 `getmultisigoutinfo` (`methods.go:2249-2302`)

Parameters: `hash` string, `index` uint32 (both required).

1. Wallet loaded check.
2. `NewHashFromStr(hash)`; failure -> RPC error `-22`.
3. Outpoint `(hash, index, tree regular)`.
4. `w.FetchP2SHMultiSigOutput(ctx, &op)` (`wallet/multisig.go:21-74`): the
   stored multisig-output record and the redeem script of its script
   hash; errors are wallet errors.
5. `stdscript.ExtractAddrs(0, redeemScript, params)`; for every returned
   address that is a pay-to-pubkey ECDSA address, in script order, append
   the hex of its 33-byte compressed key.
6. Build `GetMultisigOutInfoResult`:

| Member | Value |
|---|---|
| `address` | P2SH address string |
| `redeemscript` | hex |
| `m`, `n` | stored thresholds |
| `pubkeys` | array from step 5; `[]` when empty, never `null` |
| `txhash` | outpoint hash string |
| `blockheight`, `blockhash` | containing block when the record has one; otherwise `0` and `""`. "No block" is the stored pair (zero hash, height -1) or (zero hash, height 0) (`wallet/common.go:27-32`). |
| `spent` | true when the record is marked spent |
| `spentby`, `spentbyindex` | spender hash string and input index when spent; `""` and `0` otherwise |
| `amount` | output amount in DCR |

#### 5.3.29 `getnewaddress`, `getrawchangeaddress`

**`getnewaddress`** (`methods.go:2307-2346`). Parameters: `account` string
(optional; nil means `default`), `gappolicy` string (optional).

1. Wallet loaded check.
2. Gap policy, validated **before** the account: nil or `""` -> wallet
   default; `"error"`, `"ignore"`, `"wrap"` -> that policy; anything else
   -> RPC error `-8` `unknown gap policy <Q(value)>`. The wallet default
   (no call option) is the "error" policy (`wallet/addresses.go:396`,
   `546`).
3. Account lookup.
4. `w.NewExternalAddress(ctx, account, opts...)`
   (`wallet/addresses.go:749-760`, spec 06): one read-write transaction
   that persists the returned child index. Errors are wallet errors:
   - gap limit reached under the "error" policy: kind `Policy`, `-4`,
     `wallet.NewExternalAddress: policy violation: generating next address
     violates the unused address gap limit policy`;
   - imported voting account: kind `Invalid`, `cannot use voting accounts
     for normal transactions`;
   - the `imported` account: kind `NotExist`, `account 2147483647`.
5. Return the address string.

**`getrawchangeaddress`** (`methods.go:2353-2379`). Parameter: `account`
string (optional; nil means `default`).

1. Wallet loaded check.
2. Account lookup.
3. `w.NewChangeAddress(ctx, account)` (`wallet/addresses.go:819-823`):
   internal branch, "wrap" gap policy; the `imported` account is replaced
   by account 0. Errors are wallet errors.
4. Return the address string.

#### 5.3.30 `getreceivedbyaccount`, `getreceivedbyaddress`

**`getreceivedbyaccount`** (`methods.go:2383-2420`). Parameters: `account`
string (required), `minconf` int (optional, default 1).

1. Wallet loaded check.
2. Account lookup.
3. If the account number is greater than `0x7FFFFFFF` (an imported-xpub
   account): return the number `0` (Go `0.0`).
4. `minConf = 1` or `int32(minconf)` (truncating; no sign check).
5. `results = w.TotalReceivedForAccounts(ctx, minConf)`
   (`wallet/wallet.go:4460-4522`): one entry per account **in account
   bucket iteration order** (the little-endian byte order described in
   5.3.15), each with the sum of all wallet credits (spent or not) whose
   first extracted address belongs to that account, over mined
   transactions up to height `stop = tip - minConf + 1`, or over all
   mined and unmined transactions when `minConf <= 0`. Because the range
   function treats any negative end height as "through the mempool"
   (`wallet/udb/txquery.go:388-396`, `521-537`), a `minConf` greater than
   `tip + 1` makes `stop` negative and **also** includes every mined and
   unmined transaction (section 10.1 Q37).
6. `index = int(accountNumber)`, or `len(results) - 1` for the `imported`
   account. Return `results[index].TotalReceived.ToCoin()`.

Step 6 assumes entry *k* is account *k*. That holds only while the wallet
has no BIP0044 account numbered above 255 (at most 256 accounts) and no
imported-xpub account (section 10.1 Q4).

**`getreceivedbyaddress`** (`methods.go:2424-2450`). Parameters: `address`
string (required), `minconf` int (optional, default 1).

1. Wallet loaded check.
2. `decodeAddress`; propagate.
3. `minConf` as above.
4. `w.TotalReceivedForAddr(ctx, addr, minConf)`
   (`wallet/wallet.go:4527-4568`): sum of wallet credits whose output
   script yields an address whose string equals `addr.String()`, same
   height window. Kind `NotExist` -> RPC error `-4` `address not found in
   wallet`; other errors are wallet errors.
5. Return the amount in DCR.

`TotalReceivedForAddr` never checks that the address belongs to the
wallet: an address the wallet has never seen returns `0`, and the
`NotExist` branch is effectively dead. A hex public key parameter is
compared as a pay-to-pubkey address string and so matches only P2PK
outputs.

#### 5.3.31 `getmasterpubkey` (`methods.go:2454-2486`)

Parameter: `account` string (optional; nil means account number 0 without
a name lookup).

1. Wallet loaded check.
2. Account lookup when a name is given.
3. `w.AccountXpub(ctx, account)`; errors are wallet errors.
4. Log at warn (subsystem `RPCS`): `Attention: Extended public keys must
   not be shared with or leaked to external parties, such as VSPs, in
   combination with any account private key; this reveals all private keys
   of this account`.
5. Return the extended public key string of the **account** (BIP0044
   account-level xpub, network HD public prefix).

#### 5.3.32 `getpeerinfo` (`methods.go:2490-2539`)

No parameters.

1. Wallet loaded check.
2. `w.NetworkBackend()`; failure is the `-9` wallet error.
3. **Not an SPV backend:** start with a nil list. With a dcrd backend, call
   dcrd `getpeerinfo` with no parameters and decode the reply into
   `[]*GetPeerInfoResult` (the wallet's 8-member type; all other members
   of dcrd's peer objects are dropped, and the wallet re-encodes the
   list); a call error fails the request. Return the list. dcrd's `[]`
   stays `[]`; a JSON `null` element would be re-emitted as `null`.
4. **SPV backend:** take a snapshot of the syncer's remote peers
   (`spv/sync.go:218-223`) and build one entry per peer:

| Member | Value |
|---|---|
| `id` | `int32(peer id)` (the id is a uint64 counter; truncating) |
| `addr` | remote address of the TCP connection (`net.Addr.String()`, `host:port`, IPv6 in brackets) |
| `addrlocal` | local address of the TCP connection |
| `services` | the peer's advertised service flags as a decimal number left-padded with zeros to 8 digits (`fmt.Sprintf("%08d", uint64)`) |
| `version` | negotiated protocol version |
| `subver` | the peer's user agent |
| `startingheight` | height from the peer's version message |
| `banscore` | `int32` of the peer's current ban score |

   Sort ascending by `id` and return. No peers -> `[]`.

#### 5.3.33 `getstakeinfo` (`methods.go:2543-2591`)

No parameters.

1. Wallet loaded check.
2. `s.walletLoader.NetworkBackend()` (absence tolerated).
3. dcrd backend: `w.StakeInfoPrecise(ctx, rpc)`
   (`wallet/wallet.go:4115-4246`); otherwise (SPV or no backend):
   `w.StakeInfo(ctx)` (`wallet/wallet.go:4008-4111`). Errors are wallet
   errors.
4. `proportionlive = Live / PoolSize` (float64) when `PoolSize > 0`, else
   0. `proportionmissed = Missed / (Voted + Missed)` when `Missed > 0`,
   else 0 (the sum is computed in uint32 before conversion).
5. Return `GetStakeInfoResult` with `difficulty = Sdiff.ToCoin()` and
   `totalsubsidy = TotalSubsidy.ToCoin()`.

What the two wallet functions fill, iterating every ticket in the store
for which the wallet has voting authority (the ticket's output 0 pays to a
hash160 known to the address manager):

| Field | `StakeInfo` (SPV / no backend) | `StakeInfoPrecise` (dcrd) |
|---|---|---|
| `blockheight` | wallet tip height | same |
| `difficulty` | next stake difficulty computed by the wallet when DCP0001 is active at the tip, else 0 | same |
| `ownmempooltix` | unmined tickets | same |
| `immature` | mined, not yet mature | same |
| `voted` | spent by a vote | same |
| `totalsubsidy` | sum of the votes' stakebase input values | same |
| `revoked` | spent by a revocation | same |
| `unspent` | mature and unspent | same |
| `unspentexpired` | unspent and past expiry by height | always 0 |
| `poolsize` | `PoolSize` of the wallet's tip header | same |
| `allmempooltix` | 0 (omitted) | dcrd mempool ticket count (`MempoolCount(ctx, "tickets")`, issued concurrently with the database read) |
| `live` | 0 (omitted) | equal to `unspent` |
| `missed` | 0 (omitted) | revoked tickets whose revocation was mined fewer than `TicketExpiryBlocks + TicketMaturity` blocks after the ticket |
| `expired` | 0 (omitted) | the other revoked tickets |
| `proportionlive`, `proportionmissed` | 0 (omitted) | step 4 |

Maturity and expiry use the wallet's helpers
(`wallet/wallet.go:4435-4447`): mature when `tip - height >
TicketMaturity`; expired when `tip - height > TicketMaturity +
TicketExpiry`. A spender that is neither vote nor revocation aborts with
kind `IO` (`ticket spender <hash> is neither vote nor revocation`).

#### 5.3.34 `gettickets` (`methods.go:2595-2617`)

Parameter: `includeimmature` bool (**required**).

1. Wallet loaded check.
2. `s.walletLoader.NetworkBackend()` (absence tolerated); `rpc` is the
   backend if it implements `wallet.LiveTicketQuerier`
   (`ExistsLiveTickets`). `*chain.Syncer` does (`chain/backend.go:82`);
   `*spv.Syncer` does not, so `rpc` is nil in SPV mode and without a
   backend.
3. `w.LiveTicketHashes(ctx, rpc, includeimmature)`
   (`wallet/tickets.go:28-93`): one read transaction over every ticket in
   the store (not filtered by ownership or by spent state), in store
   iteration order:
   - skip tickets past expiry by height;
   - a ticket that is unmined or not yet mature is appended to the result
     only when `includeimmature` is true;
   - every other ticket is a live candidate.
   With `rpc == nil` the candidates are **dropped**. Otherwise, if there
   are candidates, ask dcrd which are in the live ticket pool
   (`existslivetickets`) and append those, in candidate order.
4. Return `GetTicketsResult{hashes}`: immature tickets first, then live
   tickets; `"hashes":[]` when empty, never `null`.

#### 5.3.35 `gettransaction` (`methods.go:2621-2710`)

Parameters: `txid` string (required), `includewatchonly` bool (optional,
default `false`; **ignored**).

1. Wallet loaded check.
2. `NewHashFromStr(txid)`; failure -> RPC error `-22`.
3. `wallet.UnstableAPI(w).TxDetails(ctx, hash)` (`wallet/unstable.go:29-44`).
   Kind `NotExist` -> RPC error `-5` `no information for transaction`;
   other errors are wallet errors.
4. `(_, tipHeight) = w.MainChainTip(ctx)` (separate read).
5. Serialize the stored transaction to hex.
6. Initialise the result: `txid` = **the parameter string as sent**; `hex`;
   `time` = `timereceived` = the record's received time in Unix seconds;
   `walletconflicts` = `[]`.
7. If the record is mined (`Block.Height != -1`): `blockhash` = block hash
   string, `blocktime` = the block's time in Unix seconds, `confirmations`
   = `confirms(Block.Height, tipHeight)`. Otherwise these stay `""`, `0`,
   `0`.
8. `debitTotal` = sum of the record's debits; `creditTotal` = sum of its
   credits (including change). `amount = (creditTotal -
   debitTotal).ToCoin()`.
9. If the number of debits equals the number of inputs (every input is the
   wallet's): `fee = (-(debitTotal - sum of all output values)).ToCoin()`,
   i.e. the fee as a **non-positive** number. Otherwise `fee` is 0. `fee`
   is omitted when 0.
10. `w.ListTransactionDetails(ctx, hash)` (`wallet/wallet.go:2697-2720`, a
    third read transaction); errors are wallet errors. Copy each entry to
    a `GetTransactionDetailsResult` (`account`, `address`, `amount`,
    `category`, `involveswatchonly`, `fee`, `vout`).
11. Return the result. `blockindex` is always `0`, `type` is always `""`,
    `ticketstatus` is never set (omitted).

The details list (`wallet.listTransactions`, `wallet/wallet.go:2394-2520`)
is: all "receive-side" entries in output order, then all "send" entries in
output order. For each output `i` of the transaction:

- if `i` is a credit marked as change, the output contributes nothing;
- `address` is the single extracted address of the output script, or
  empty (omitted) when the script yields zero or several addresses;
- if the transaction has at least one debit, a send entry: `category`
  `"send"`, `amount = -value` (an output of value 0, such as a ticket
  commitment, yields **`-0`**), `fee` = pointer to the transaction's
  non-positive fee (present even when 0; `0` when not every input is a
  debit), `account` `""`;
- if `i` is a (non-change) credit, a receive entry: `account` = name of
  the account owning the address (empty if unknown), `amount = value`,
  no `fee`, `category` = `"generate"` for a coinbase that has matured
  (`tip - height + 1 > CoinbaseMaturity`), `"immature"` for a coinbase
  that has not, otherwise `"receive"`.

`involveswatchonly` is never set. An output can therefore appear zero, one
or two times.

#### 5.3.36 `gettxout` (`methods.go:2719-2794`)

Parameters: `txid` string, `vout` uint32, `tree` int8 (required),
`includemempool` bool (optional, default `true`).

1. Wallet loaded check.
2. `w.NetworkBackend()`; failure is the `-9` wallet error.
3. **dcrd backend:** call dcrd `gettxout` with `[txid, vout, tree,
   includemempool]` and return the raw result (`null` for a spent or
   unknown output). Same error rules as 5.3.17 step 3.
4. **SPV backend:**
   1. `NewHashFromStr(txid)`; failure -> RPC error `-22`.
   2. `tree` not 0 or 1 -> RPC error `-8` `Tx tree must be regular or
      stake`.
   3. `includeMempool = true` unless the parameter is non-nil.
   4. `w.UnspentOutput(ctx, outpoint, includeMempool)`
      (`wallet/wallet.go:3323`, `wallet/udb/txmined.go:2537-2553`): if
      `includeMempool` and an unmined credit exists for `(hash, index)`,
      use it; else if a mined unspent credit exists, use it; else kind
      `NotExist`. The lookup key does not include the tree, so a wrong
      `tree` value still finds the output. A mined output spent only by an
      unmined transaction is still reported. Kind `NotExist` -> result
      `null`; other errors are wallet errors.
   5. `asm = DisasmString(pkScript)`; `(type, addrs) = ExtractAddrs(0,
      pkScript, s.activeNet)`; `reqSigs = DetermineRequiredSigs(0,
      pkScript)`.
   6. `(bestHash, bestHeight) = w.MainChainTip(ctx)`; `confirmations =
      confirms(height, bestHeight)` for a mined output, `0` for an unmined
      one.
   7. Return `GetTxOutResult`:

| Member | Value |
|---|---|
| `bestblock` | the wallet's current tip hash |
| `confirmations` | step 6 |
| `value` | amount in DCR |
| `scriptPubKey.asm` | disassembly |
| `scriptPubKey.hex` | script hex |
| `scriptPubKey.reqSigs` | omitted when 0 |
| `scriptPubKey.type` | `ScriptType.String()` (4.8); ticket outputs report their stake script type because version 0 extraction is used directly |
| `scriptPubKey.addresses` | address strings; omitted when none |
| `scriptPubKey.version` | always `0` |
| `coinbase` | the credit's coinbase flag (always false for an unmined credit) |

Only outputs the wallet tracks as credits can be answered in SPV mode;
every other outpoint returns `null`.

---

## 6. Constants, limits and defaults

| Name | Value | Source | What depends on it |
|---|---|---|---|
| `jsonrpcSemverString` / major / minor / patch | `"11.0.0"` / 11 / 0 / 0 | `methods.go:55-60` | `version` result (`dcrwalletjsonrpcapi`) |
| `sstxCommitmentString` | `"sstxcommitment"` | `methods.go:65` | `scriptPubKey.type` of odd ticket outputs |
| `scriptVersionAssumed` | 0 | `methods.go:69` | script parsing in `getmultisigoutinfo`, `gettxout` |
| Default `minconf` | 1 | `methods.go:1054`, `2403`, `2436`; registered default `1` (`types/methods.go:231`, `307`, `325`) | `getbalance`, `getreceivedbyaccount`, `getreceivedbyaddress` |
| Fixed confirmations for derived balances | 1 | `methods.go:1782`, `1942` | `getinfo` balance, `getunconfirmedbalance` |
| Default account name | `"default"` | `methods.go:1931`, `2329`, `2360` | `getunconfirmedbalance`, `getnewaddress`, `getrawchangeaddress` |
| Default account number | 0 (`udb.DefaultAccountNum`) | `methods.go:573`, `2463`; `wallet/udb/addressmanager.go:53` | `consolidate`, `getmasterpubkey` |
| All-accounts wildcard | `"*"` | `methods.go:1062`, `1072`, `2206`, `2231` | `getbalance` default; rejected as a new account name |
| Imported account name / number | `"imported"` / `0x7FFFFFFF` | `wallet/udb/addressmanager.go:47-50`; `methods.go:433`, `983`, `2031`, `2093`, `2399`, `2416` | `addmultisigaddress`, `getaddressesbyaccount`, `import*`, `getreceivedbyaccount` |
| External / internal branch | 0 / 1 | `wallet/udb/addressmanager.go:80-85` | `accountaddressindex`, `accountsyncaddressindex`, `getaddressesbyaccount` |
| Hardened key start | `0x80000000` | `hdkeychain`; `methods.go:380` | upper bound (exclusive) for `accountsyncaddressindex` index |
| Hex public key lengths | 66 or 130 characters | `methods.go:1831` | `decodeAddress` pubkey path |
| Median-time window | 11 headers | `methods.go:1287`, `1411` | `mediantime` |
| Initial-block-download threshold | tip older than 24 h | `methods.go:1742-1743` | `syncstatus.initialblockdownload` |
| Difficulty precision | 8 fraction digits | `methods.go:1717` | `difficulty` |
| `wire.MaxTxInSequenceNum` | `0xFFFFFFFF` | `dcrd/wire@v1.8.0/msgtx.go:32`; `methods.go:646`, `678` | `createrawtransaction` locktime bound; input sequence (`-1` when locktime non-zero) |
| `dcrutil.MaxAmount` | `21e14` atoms | `dcrd/dcrutil/v4@v4.0.3/const.go:16`; `methods.go:712` | `createrawtransaction` output range |
| `dcrutil.AtomsPerCoin` | `1e8` | `const.go:13`; `methods.go:1434` | all amount conversions |
| Transaction trees | regular 0, stake 1 | `dcrd/wire@v1.8.0/msgtx.go:64-68` | `createrawtransaction`, `gettxout` validation; `getmultisigoutinfo` always uses 0 |
| `wire.TxVersionTreasury` | 3 | `msgtx.go:28`; `methods.go:1464` | treasury detection in `getblock` verbose transactions |
| `createrawtransaction` tx version | 1 (`wire.TxVersion`) | `msgtx.go:20`, `1164-1171` | serialized result |
| `fundrawtransaction` default confirmations | 1 | `methods.go:890` | input selection |
| `fundrawtransaction` default fee rate | wallet relay fee (`w.RelayFee()`) | `methods.go:889` | fee |
| `import*` defaults | `rescan = true`, `scanfrom = 0` | `methods.go:2015-2022`, `2080-2087`, `2135-2142` | background rescan |
| `getblock` / `getblockheader` `verbose` default | `true` (registered) | `dcrd/rpc/jsonrpc/types/v4@v4.4.0/chainsvrcmds.go:359`, `410` | result form when the parameter is omitted |
| `getblock` `verbosetx` default | `false` | `chainsvrcmds.go:360` | |
| `gettxout` `includemempool` default | `true` | `chainsvrcmds.go:742`; `methods.go:2748` | |
| `gettransaction` `includewatchonly` default | `false` (unused) | `types/methods.go:366` | none |
| `version.Integer` | 2020000 (2.2.0) | `version/version.go:21-27` | `getinfo.version`, `getinfo.walletversion` |
| `p2p.Pver` | 11 | `p2p/peering.go:48` | `getinfo.protocolversion` (SPV/offline) |
| Peer services format | `%08d` of the uint64 flags | `methods.go:2520` | `getpeerinfo.services` (SPV) |
| GCS key size | 16 bytes | `dcrd/gcs/v4@v4.1.1/gcs.go:69` | `getcfilterv2.key` (32 hex characters) |
| Block header size | 180 bytes | `wire` | non-verbose `getblockheader` (360 hex characters) |

RPC error codes used in this scope (`dcrd/dcrjson/v4@v4.2.0/jsonrpcerr.go`):

| Code | dcrjson name(s) used here |
|---|---|
| -1 | `ErrRPCMisc`, `ErrRPCUnimplemented` |
| -4 | `ErrRPCWallet` |
| -5 | `ErrRPCInvalidAddressOrKey`, `ErrRPCNoTxInfo` |
| -6 | `ErrRPCWalletInsufficientFunds` (via `convertError`) |
| -8 | `ErrRPCInvalidParameter` |
| -9 | `ErrRPCClientNotConnected` |
| -11 | `ErrRPCWalletInvalidAccountName` |
| -13 | `ErrRPCWalletUnlockNeeded` |
| -14 | `ErrRPCWalletPassphraseIncorrect` (via `convertError`) |
| -22 | `ErrRPCDecodeHexString`, `ErrRPCDeserialization` (same value) |
| -32600 | `ErrRPCInvalidRequest.Code` |
| -32602 | `ErrRPCInvalidParams.Code` |
| -32603 | `ErrRPCInternal.Code` |

---

## 7. Concurrency and lifecycle

- **Execution context.** A handler runs to completion on the goroutine
  that received the request: the `net/http` handler goroutine for POST
  (bounded by `MaxPOSTClients`), or one new goroutine per request for
  websocket clients (unbounded per connection). Handlers are re-entrant;
  none takes a server-level lock.
- **Wallet lookup.** `LoadedWallet()` and the loader's `NetworkBackend()`
  each take the loader mutex briefly. The wallet pointer is captured once
  per request; a wallet unloaded mid-request is not detected by the
  handler.
- **Database transactions.** Every wallet method called by a handler opens
  and closes its own bbolt transaction (`walletdb.View`/`Update`,
  `wallet/walletdb/interface.go:203-255`). bbolt allows one writer and
  many readers; write handlers (`abandontransaction`,
  `accountsyncaddressindex`, `addmultisigaddress`, `addtransaction`,
  `consolidate`, `createnewaccount`, `renameaccount`, `import*`,
  `importcfiltersv2`, `getnewaddress`, `getrawchangeaddress`,
  `fundrawtransaction` when it derives change) serialize on the writer
  lock. Multi-read handlers are not atomic: `getbalance` (tip hash, then
  balances, then one read per account name), `getinfo`, `getblockheader`
  and `getblock` (main-chain test, tip, next block, up to 11 header reads),
  `gettransaction` (details, tip, list details), `gettxout`, `syncstatus`.
  A block connected between the reads can make the parts of one reply
  disagree (for example `confirmations` versus `bestblock`).
- **`MainChainTip` cannot fail.** It discards the transaction error
  (`wallet/wallet.go:792-802`); on failure the handler proceeds with the
  zero hash and height 0.
- **Wallet mutexes reached from handlers** (owned by the wallet specs):
  the address-buffers mutex (`accountaddressindex`,
  `accountsyncaddressindex`, `getaccountaddress`, `getaddressesbyaccount`,
  `getnewaddress`, `getrawchangeaddress`, `createnewaccount`,
  `importxpub`), the locked-outpoints mutex (`fundrawtransaction`,
  `consolidate`, `addtransaction`), the relay-fee mutex (`getinfo`,
  `fundrawtransaction`), the recently-published mutex (`addtransaction`).
  In `nextAddress` the database write transaction is opened before the
  address-buffers mutex is taken (`wallet/addresses.go:513-552`).
- **Background goroutines.** `importprivkey`, `importpubkey` and
  `importscript` each start at most one goroutine per request, registered
  in the server wait group `s.wg` and driven by the server base context
  (`s.httpServer.BaseContext(nil)`, the process context passed to
  `NewServer`). The reply is sent before the rescan starts doing work.
  Server shutdown waits for these goroutines (spec 16 section 5.8); they
  end early only through cancellation of the base context. Concurrent
  import requests start concurrent rescans. `getstakeinfo` on a dcrd
  backend runs one extra goroutine for the mempool count and joins it
  before replying (inside `wallet.StakeInfoPrecise`, not in `methods.go`).
  The three `go` statements of the import handlers (`methods.go:2059`,
  `2117`, `2169`) are the only goroutine launches in lines 1-2797; the range
  declares no channel, mutex, timer or ticker of its own. `s.wg.Add(1)`
  is called on the handler goroutine before the launch and
  `s.httpServer.BaseContext(nil)` is evaluated inside the new goroutine,
  after the handler may already have returned.
- **Wall clock.** `syncstatus` is the only handler in this range that
  reads the clock (`time.Now()`, `methods.go:1742`); no handler sleeps,
  retries or applies a timeout of its own. The only retry loop reachable
  from this range is the SPV block fetch inside `getblock` (5.3.18).
- **Cancellation.** The request context is passed to every wallet and dcrd
  call. It is cancelled when the HTTP client disconnects, when the
  websocket session ends, or at shutdown. Long-running handlers
  (`discoverusage`, SPV `getblock`, `consolidate`, `auditreuse`,
  `getreceivedby*`) end only through completion or that cancellation;
  there is no per-request timeout. After a cancelled request the
  dispatcher logs `Canceled RPC method <method> invoked by <addr>: <err>`
  at warn.
- **Passthrough calls** (`getblock`, `getblockheader`, `gettxout`,
  `getinfo`, `getpeerinfo`, `getstakeinfo`, `gettickets`) share the
  wallet's single dcrd websocket connection with the syncer; replies are
  matched by request id (spec 15).
- **Ordering guarantees visible to clients.** None across requests. Within
  a request: the order of validation steps given in section 5.3 determines
  which error is reported; result arrays are ordered as stated per method.
- **Panics.** A Go panic inside a handler on the POST path is recovered by
  `net/http` (the connection is closed without a reply); on the websocket
  path it terminates the process. Reachable panics in this scope are
  listed in section 10.1 (Q3, the conditional Q38, and the zero-target
  case of `difficultyRatio`). `getreceivedbyaccount` indexes
  `results[account]` without a bounds check (`methods.go:2419`); the index
  is in range because every account the name lookup can return is also
  enumerated by `TotalReceivedForAccounts` in the same database state,
  but the two run in separate read transactions.

---

## 8. Errors

### 8.1 How an error reaches the client

A handler returns either an `*dcrjson.RPCError` (sent as is) or any other
Go error (mapped by `convertError`, spec 16 section 8.2). The reply is
`{"jsonrpc":…,"result":null,"error":{"code":c,"message":m},"id":…}` with
HTTP status 200. A dcrd error object received on a passthrough call
(`*wsrpc.Error`) is forwarded with its code and message.

### 8.2 Fixed RPC errors raised in this scope

| Code | Message | Raised by |
|---|---|---|
| -4 | `request requires a wallet but wallet has not loaded yet` | every handler marked W in 3.3 |
| -4 | `address not found in wallet` | `walletPubKeys` (so `addmultisigaddress`, `createmultisig`), `getaccount`, `getreceivedbyaddress` (dead branch) |
| -4 | `imported addresses must belong to the imported account` | `addmultisigaddress`, `importprivkey`, `importpubkey` |
| -11 | `account not found` | account lookup in `accountaddressindex`, `accountsyncaddressindex`, `consolidate`, `getaddressesbyaccount`, `getbalance`, `getaccountaddress`, `getunconfirmedbalance`, `renameaccount`, `getnewaddress`, `getrawchangeaddress`, `getreceivedbyaccount`, `getmasterpubkey` |
| -13 | `wallet or account locked; use walletpassphrase or unlockaccount first` | `dumpprivkey`, `importprivkey` |
| -13 | `creating new accounts requires an unlocked wallet` | `createnewaccount` |
| -8 | `account name is reserved by RPC server` | `createnewaccount`, `renameaccount` |
| -9 | `disconnected from network` | `discoverusage`, `import*` with rescan |
| -9 | `disconnected from consensus RPC` | passthrough of unknown methods |
| -9 | `RPC passthrough requires dcrd RPC synchronization` | passthrough of unknown methods |
| -1 | `Method unimplemented`, `Request unsupported by dcrwallet` | unreachable |
| -32600 | `Invalid request` | parameter decoding |
| -8 | `invalid branch <n>` | `accountaddressindex` |
| -8 | `child index <n> exceeds the maximum child index for an account` | `accountsyncaddressindex` |
| -5 | `address has no associated public key` | `walletPubKeys` |
| -5 | `invalid address <Q>: decode failed: <BQ>` | `decodeAddress` |
| -5 | `invalid stake address <Q>` | `decodeStakeAddress` |
| -8 | `Expiry out of range`, `Locktime out of range`, `Tx tree must be regular or stake`, `Positive input amount is required`, `Amount outside valid range: <amount>` | `createrawtransaction` (`Tx tree…` also SPV `gettxout`) |
| -8 | `<hash error text>` | `createrawtransaction` input txid |
| -5 | `Address <Q>: <err>`, `Invalid type: <Go type>` | `createrawtransaction` |
| -32603 | `New amount: <err>` | `createrawtransaction` (unreachable) |
| -22 | `<hash error text>` | `abandontransaction`, `addtransaction`, SPV `getblockheader`, SPV `getblock`, `getcfilterv2`, `getmultisigoutinfo`, `gettransaction`, SPV `gettxout` |
| -22 | `<hex or deserialization error text>` | `addtransaction`, `createsignature`, `importpubkey`, `importscript` |
| -8 | `transaction input index cannot be negative` | `createsignature` |
| -1 | `transaction input <n> does not exist` | `createsignature` |
| -8 | `invalid debug level <spec>: <err>` | `debuglevel` |
| -8 | `startblock: <hash error text>` | `discoverusage` |
| -8 | `minconf must be non-negative` | `getbalance` |
| -32603 | `<wallet error text>` | `getbalance`, `getunconfirmedbalance`, `getaccountaddress` when a just-resolved account is reported missing |
| -32603 | `Could not serialize block header: <err>`, `Error checking if block is in mainchain: <err>`, `Info not found for next block: <err>`, `Info not found for previous block: <err>` | SPV `getblockheader`, SPV `getblock` |
| -32603 | `Network returned 0 blocks`, `Could not serialize block: <err>`, `Could not create transaction: <err>`, `Could not create stake transaction: <err>` | SPV `getblock` |
| -32602 | `filter <i> is not a valid hex string` | `importcfiltersv2` |
| -32600 | `<wallet error text>` | `importcfiltersv2` import failure |
| -5 | `WIF decode failed: <err>` | `importprivkey` |
| -8 | `empty script` | `importscript` |
| -8 | `unknown gap policy <Q>` | `getnewaddress` |
| -5 | `no information for transaction` | `gettransaction` |
| -1 | `context canceled` / `context deadline exceeded` (`ctx.Err().Error()`) | passthrough of unknown methods when the request context ended (`methods.go:248-254`) |
| -32603 | `Unexpected error marshalling result` | `makeResponse` (`methods.go:296-299`); not produced by any caller |
| -8 | `invalid coin amount` | `createrawtransaction` input amount (`methods.go:666-669`); needs NaN or infinity, which JSON cannot carry |
| -4 | `confs must be non-negative`, `transaction must not already have inputs` | `fundrawtransaction` (`methods.go:907`, `924`); plain Go errors, so the code comes from the `convertError` default |
| -4 | `<Go hex / deserialization error text>` | `fundrawtransaction` transaction decoding (`methods.go:913-916`); hex public key path of `decodeAddress` (`methods.go:1832-1840`); `importxpub` key parsing (`methods.go:2186-2189`); `addmultisigaddress`/`createmultisig` script construction (`methods.go:446-449`, `618-621`) |

Every `*dcrjson.RPCError` construction in lines 1-2797 appears in the
table above (inventory by grep: 4 struct literals, in `unimplemented`,
`unsupported`, the passthrough cancellation and `makeResponse`; one use
of `dcrjson.ErrRPCInvalidRequest`; 22 `rpcError` calls; 36 `rpcErrorf`
calls; and the eight shared error variables, of which
`errUnloadedWallet` is returned at 45 sites). Everything else
a handler returns is a wallet or library error mapped by `convertError`.

### 8.3 Wallet error text

The message of a `convertError`-mapped wallet error is `(*errors.Error).Error()`
(`errors/errors.go:247-290`): starting at the outermost error, write the
operation name if set and different from the previously written one; then
`": "` and the kind description if the kind is set and different from the
previously written one; then, if the wrapped error is another
`*errors.Error`, write the separator and continue with it, otherwise write
`": "` and the wrapped error's text. The separator is `":: "` in the
daemon (`dcrwallet.go:36-39` overrides the package default `":\n\t"`).
`errors.E` collapses a wrapped `*Error` that adds no distinct operation or
kind (`errors/errors.go:187-236`), which is why most messages have the
single-level form `<op>: <kind>: <detail>`.

Kind descriptions (`errors/errors.go:77-124`) that appear in this scope:
`invalid operation`, `item already exists`, `item does not exist`,
`invalid encoding`, `wallet locked`, `watching only wallet`, `insufficient
balance`, `transaction script fails to execute`, `policy violation`,
`Decred network is unreachable`, `I/O error`, `internal wallet error`.

Operation names used by the wallet methods called here (each is the
leading token of the message): `wallet.AbandonTransaction(<hash>)`,
`wallet.AccountNumber`, `wallet.AccountName`,
`wallet.BIP0044BranchNextIndexes`, `wallet.SyncLastReturnedAddress`,
`wallet.KnownAddress`, `wallet.ImportScript`, `wallet.AddTransaction`,
`wallet.GetTransactions`, `wallet.HaveAddress`, `wallet.Consolidate`,
`wallet.CreateSignature`, `wallet.DiscoverActiveAddresses`,
`wallet.DumpWIFPrivateKey`, `wallet.LoadPrivateKey`,
`wallet.NewUnsignedTransaction`, `wallet.ImportedAddresses("imported")`,
`wallet.AccountXpub`, `wallet.CalculateAccountBalance` (sic, for
`AccountBalance`), `wallet.AccountBalances`, `wallet.BlockInfo`,
`wallet.NetworkBackend`, `wallet.BlockInMainChain`, `wallet.CurrentAddress`,
`wallet.ImportCFiltersV2`, `wallet.ImportPrivateKey`,
`wallet.ImportPublicKey`, `wallet.ImportXpubAccount`, `wallet.NextAccount`,
`wallet.RenameAccount`, `wallet.FetchP2SHMultiSigOutput`,
`wallet.NewExternalAddress`, `wallet.NewChangeAddress`,
`wallet.TotalReceivedForAccounts`, `wallet.TotalReceivedForAddr`,
`wallet.StakeInfo`, `wallet.StakeInfoPrecise`, `wallet.LiveTicketHashes`,
`wallet.TxDetails`, `wallet.ListTransactionDetails`. `w.BlockHeader`,
`w.CFilterV2` and `w.UnspentOutput` add no operation name.

### 8.4 Errors that are not JSON-RPC errors

- A result that cannot be marshalled (`syncstatus` with a non-finite
  progress): HTTP 500 on POST, silence on websocket, plus the error log
  `Unable to marshal response to client <addr>: <err>` (spec 16).
- A handler panic: connection reset on POST; process abort on websocket.

### 8.5 Log lines emitted in this scope (subsystem `RPCS`)

| Level | Text | Source |
|---|---|---|
| warn | `Canceled RPC method <method> invoked by <addr>: <err>` | `methods.go:249`, `270` |
| warn | `Failed to decode ticket commitment addr output for tx hash <hash>, output idx <i>` | `methods.go:1640-1642` |
| warn | `Failed to decode ticket commitment amt output for tx hash <hash>, output idx <i>` | `methods.go:1648-1650` |
| error | `Cannot get difficulty: <err>` | `methods.go:1720` |
| warn | `Attention: Extended public keys must not be shared with or leaked to external parties, such as VSPs, in combination with any account private key; this reveals all private keys of this account` | `methods.go:2480-2483` |

Wallet-subsystem lines triggered by handlers (`Imported script with P2SH
address <addr>`, `Imported payment address <addr>`, discovery and rescan
progress) are specified with the wallet.

---

## 9. Compatibility-critical behaviours

Each item is something an existing client (dcrctl, Decrediton, exchange
integrations, scripts, test harnesses) can observe.

1. **Method names and positional parameter order** of the 48 methods are
   exactly those of section 5.3; `createsignature` takes `address,
   inputindex, hashtype, previouspkscript, serializedtransaction` in that
   order; `gettickets` requires its boolean.
2. **Result member names, order, presence and `omitempty` behaviour** of
   every struct in sections 4.3 to 4.8, including the mixed-case names
   `redeemScript` (`createmultisig`), `scriptPubKey`, `scriptSig`,
   `reqSigs`, and `unlocked_until`.
3. **Amount encoding**: float64 division by 1e8 and Go float formatting
   (section 4.2), including exponent notation below `0.000001` and `-0`.
4. **Backend split**: `getblock`, `getblockheader` and `gettxout` return
   dcrd's bytes (compacted, HTML-escaped) when synced through dcrd RPC and
   the wallet-built objects of 5.3.17, 5.3.18 and 5.3.36 in SPV mode. The
   SPV objects differ from dcrd's in `chainwork` (`""`), `getblock.hash`
   (echo of the input), `scriptPubKey.version` (0) and `gettxout`
   coverage (wallet outputs only).
5. **Never-forwarded chain methods**: `getbestblock`, `getbestblockhash`,
   `getblockcount`, `getblockhash`, `getcfilterv2`, `getcurrentnet`,
   `createrawtransaction`, `debuglevel` are answered by the wallet alone
   even when dcrd is attached, and describe the wallet's chain view.
   `getinfo` and `getpeerinfo` are also wallet handlers (never the generic
   passthrough) but query dcrd in RPC mode as described in items 6 and 7.
   `getcfilterv2` has the wallet's own result shape (`blockhash`,
   `filter`, `key`).
6. **`getinfo` merge rule**: seven members come from dcrd in RPC mode
   (`version`, `protocolversion`, `timeoffset`, `connections`, `proxy`,
   `relayfee`, `errors`); the rest always come from the wallet; a dcrd
   failure fails the call.
7. **`getpeerinfo`**: 8-member objects in both modes (dcrd's extra members
   are stripped), SPV list sorted by `id`, `services` as an 8-digit
   zero-padded decimal string.
8. **`getbalance`**: default account `*`; `balances` order is account
   bucket key order (little-endian byte order, `imported` last); totals
   are omitted when zero and always absent in the single-account form;
   `blockhash` is the wallet tip.
9. **Error codes and messages** of section 8.2, in the validation order of
   section 5.3. Clients match on codes `-4` (wallet not loaded, address
   not in wallet), `-11` (account not found), `-13` (unlock needed), `-9`
   (no network), `-5` (bad address/key, unknown transaction), `-22` (bad
   hex/hash), `-8` (bad parameter).
10. **`decodeAddress` acceptance**: a 66-character hex compressed secp256k1
    public key is accepted wherever an address is (`getaccount`,
    `dumpprivkey`, `createsignature`, `getreceivedbyaddress`, multisig key
    lists, `consolidate`), base58 addresses of all seven version-0 kinds
    decode, and the error text format of 5.2.1.
11. **Hash parameter leniency**: short, odd-length and upper-case hash
    strings are accepted (section 4.2).
12. **`import*` semantics**: `label` must be absent or `imported`; a
    duplicate import is a silent success that skips the rescan; a
    successful import returns before the rescan finishes; the rescan
    survives the client disconnecting; `rescan` in effect without a
    network backend is `-9` before any input validation.
13. **`createnewaccount` / `renameaccount`** reject the name `*` with `-8`
    and return `null` on success.
14. **`getnewaddress` gap policy** strings `""`, `error`, `ignore`, `wrap`;
    default policy "error"; `getrawchangeaddress` always wraps and maps
    the imported account to account 0.
15. **`createrawtransaction` output**: version-1 transaction, inputs in
    array order with value-in set from `amount`, sequence `0xFFFFFFFE`
    when a non-zero locktime is given, only P2PKH-ECDSA and P2SH
    destinations, hex of the full serialization.
16. **`fundrawtransaction` output**: caller outputs kept in order with the
    change output appended last; `fee` in DCR; option key `conf_target`.
17. **`gettransaction`**: `fee` non-positive and omitted when zero;
    `details` ordered receives then sends; categories `send`, `receive`,
    `generate`, `immature`; `walletconflicts` is `[]`; `txid` echoes the
    request; `blockindex` 0; `type` `""`.
18. **`getstakeinfo` / `gettickets`** backend-dependent content (5.3.33,
    5.3.34), including which members are omitted in SPV mode.
19. **`auditreuse`** object shape, `since` semantics and the exclusion of
    votes and revocations.
20. **`getaddressesbyaccount`** ordering (external ascending, then internal
    ascending; imported in index order) and `[]` for no addresses.
21. **Verbose transaction objects** (4.6 to 4.8): the five `vin` shapes,
    `sstxcommitment` with `commitamt`, script type names, omission rules.
22. **`addmultisigaddress`** imports the script and its P2SH address into
    the imported account (the side effect other tools rely on) regardless
    of the malformed result.
23. **Explicit `null` parameters** behave as in section 4.1, including the
    non-verbose result for `getblock <hash> null` in SPV mode.
24. **Null results**: methods documented as returning nothing return JSON
    `null`; SPV `gettxout` returns `null` for unknown or spent outputs.
25. **Unknown-method passthrough**: any method name absent from the
    `handlers` table is forwarded to dcrd with its raw parameters when the
    wallet syncs through dcrd RPC, and dcrd's result bytes or error object
    are returned; without a backend it is `-9` `disconnected from
    consensus RPC`, in SPV mode `-9` `RPC passthrough requires dcrd RPC
    synchronization`. Tools such as dcrctl rely on this to reach dcrd
    methods through the wallet port. A name that is in the table is never
    forwarded, whatever its parameters (the eight dead names included:
    they always answer `-32600`).
26. **Default account selection** (Q39): by the name `default` in
    `getunconfirmedbalance`, `getnewaddress`, `getrawchangeaddress`; by
    account number 0 in `consolidate` and `getmasterpubkey`; all accounts
    (`*`) in `getbalance`.
27. **`getaddressesbyaccount` raw string emission**: the array is written
    without JSON string escaping (base58 strings need none); the result
    bytes are `["a","b"]` with no spaces.
28. **`gettransaction` has no `generated` member** and its `details`
    entries have exactly the seven members of
    `GetTransactionDetailsResult`, not the richer `listtransactions`
    entry shape.
29. **`getreceivedbyaccount` returns `0` for imported-xpub accounts**
    (account number above `0x7FFFFFFF`) without reading any transaction.

---

## 10. Quirks

### 10.1 Bugs, surprises, runtime-dependent behaviour

**Q1. Dead handler entries.** `unimplemented` and `unsupported` can never
run: the eight methods have no registered command type, so parameter
parsing fails first and the client sees `-32600 Invalid request`
(`methods.go:192-199`, `263-266`).

**Q2. `addmultisigaddress` returns `{}`.** The handler returns the
`*stdaddr.AddressScriptHashV0` itself (`methods.go:456`); the type has no
exported field and no marshaler (`dcrd/txscript/v4@v4.1.2/stdaddr/addressv0.go:898-901`),
so the JSON result is the empty object. The help documents a string
(`helpdescs_en_US.go:40`, `rpchelp/methods.go:34`). `createmultisig`
(`methods.go:628-631`) calls `String()` and is correct.

**Q3. `auditreuse` nil dereference.** `out.Address.String()`
(`methods.go:509`) is called on a nil interface when a wallet credit's
script yields no address or its address-manager lookup fails
(`wallet/notifications.go:96-105`). On POST the connection is dropped; on
websocket the process aborts.

**Q4. `getreceivedbyaccount` indexes by iteration position.**
`results[int(account)]` (`methods.go:2415-2419`) assumes
`TotalReceivedForAccounts` returns accounts in numeric order. The order is
that of 4-byte little-endian keys (`wallet/udb/addressdb.go:698-708`):
with an imported-xpub account (number `0x80000000`, key `00 00 00 80`)
position 1 is that account, and once account number 256 exists (257
BIP0044 accounts) position 1 is account 256. The wrong account's total is returned without error. The `imported`
account uses `len(results) - 1`, which stays correct because its key `FF
FF FF 7F` sorts last.

**Q5. `getbalance "*"` ordering** is the same key order (5.3.15); it is
numeric only for wallets with no account numbered above 255 and no
imported xpub account.

**Q6. Map iteration in `createrawtransaction`.** Outputs are appended in Go
map iteration order (`methods.go:685`), which is deliberately randomised
by the runtime: the same request can yield different serializations and,
with several invalid entries, different errors.

**Q7. Unchecked integer narrowing.** `accountaddressindex.branch` and
`accountsyncaddressindex.branch`/`index` (`int` to `uint32`,
`methods.go:347`, `377-378`); `getbalance`, `getreceivedby*` `minconf`
(`int` to `int32`, `methods.go:1056`, `2405`, `2438`); `getblockhash.index`
(`int64` to `int32`, `methods.go:1212`); `import*` `scanfrom` (`int` to
`int32`); `createrawtransaction.expiry` (`int64` to `uint32`,
`methods.go:732`); `createsignature.hashtype` (`int` to 8 bits,
`methods.go:780`); `getpeerinfo.id` (`uint64` to `int32`,
`methods.go:2517`). Example: `accountaddressindex "default" 4294967296`
returns the external index; `getblockhash 4294967296` returns the genesis
hash.

**Q8. Float to integer conversion.** `dcrutil.NewAmount` converts
`f*1e8 ± 0.5` with a Go `float64 -> int64` conversion; for magnitudes
beyond int64 the Go result is implementation-defined (on amd64 it is
`-2^63`). `createrawtransaction` then reports `Amount outside valid range`
with whatever value resulted. Rust's saturating `as` differs for these
inputs.

**Q9. Explicit `null` for `verbose`.** The registered default is `true`,
but the SPV code treats a nil pointer as false (`methods.go:1252`,
`1377`): `getblock <hash>` returns the object, `getblock <hash> null`
returns hex. In dcrd mode the `null` is forwarded and dcrd decides.

**Q10. Echoed input strings.** `getblock.hash` (SPV), `gettransaction.txid`
and `getcfilterv2.blockhash` repeat the parameter text (`methods.go:1436`,
`2653`, `1973`), so an upper-case or shortened hash is returned as typed,
whereas SPV `getblockheader.hash` is canonical (`methods.go:1311`).

**Q11. `gettransaction` unset members.** `type` is never assigned and is
always `""`; `ticketstatus` and `blockindex` are never assigned;
`includewatchonly` is ignored (`methods.go:2652-2659`).

**Q12. `gettickets` in SPV mode** lists only immature and unmined tickets,
and only when asked to; mature unspent tickets are dropped because their
liveness cannot be checked (`wallet/tickets.go:70-73`). With
`includeimmature=false` the SPV result is always `{"hashes":[]}`.

**Q13. SPV `getblock` never fails for an unknown hash**; it retries peers
until the request context ends (`spv/backend.go:95-111`).

**Q14. `syncstatus` can be unanswerable** (non-finite float32, 5.3.19) and
can report a progress outside `[0, 1]`.

**Q15. `scriptPubKey.version` is always 0** in wallet-built vouts and in
SPV `gettxout`; `createVoutList` assigns only the outer `version`
(`methods.go:1686-1698`). dcrd's own `createVoutList` sets both
(`dcroxide:crates/dcroxide-rpc/src/txresults.rs:292-306` mirrors dcrd).

**Q16. Negative zero.** `gettransaction` details for a sending transaction
report `"amount":-0` for zero-value outputs
(`wallet/wallet.go:2471`, `2507`).

**Q17. Inconsistent error codes for the same failure class.**
`fundrawtransaction` reports a transaction that fails to deserialize, a
bad option and an unknown account all as `-4` (`methods.go:899-930`),
where `addtransaction` uses `-22` and other methods `-11`;
`createrawtransaction` reports a bad txid as `-8` where other methods use
`-22` (`methods.go:656`); `importcfiltersv2` uses the JSON-RPC protocol
codes `-32602` and `-32600` for application errors (`methods.go:1994`,
`2000`); the hex-public-key path of `decodeAddress` yields `-4` while the
base58 path yields `-5`; `importpubkey` with a wrong-length key yields
`-8` through the `Encoding` kind.

**Q18. Check ordering.** `addmultisigaddress` validates `account` and
`getcfilterv2` validates the hash before the wallet-loaded check
(`methods.go:433-440`, `1957-1965`); `createrawtransaction`, `debuglevel`
and `getcurrentnet` never need a wallet. The `import*` methods test the
network before validating their input (`methods.go:2023-2026`).

**Q19. `addtransaction` does not verify inclusion.** A block hash unknown
to the wallet or off its main chain silently turns the request into "add
as unmined" (`wallet/chainntfns.go:365-380`); a main-chain block hash is
trusted. Characters after a complete transaction in the hex string are
ignored.

**Q20. Local data needs a backend.** `getblockheader`, `gettxout`,
`syncstatus` and `getpeerinfo` fail with `-9` `wallet.NetworkBackend:
Decred network is unreachable` when no syncer is attached, although the
SPV code paths read only the wallet database.

**Q21. `getinfo` in SPV mode** reports `connections: 0` whatever the peer
count, and `walletversion` is the software version integer, not a
database version.

**Q22. `getstakeinfo` asymmetries.** `poolsize` is filled in SPV mode
despite the struct comment "Not available to SPV wallets"
(`types/results.go:96`); `unspentexpired` is never set in dcrd mode;
`live` equals `unspent` in dcrd mode (no liveness query is made); an
unmined revocation counts as `missed`.

**Q23. `getreceivedbyaddress`** never reports an unknown address; the
`errAddressNotInWallet` branch (`methods.go:2443-2445`) is dead.

**Q24. `getaccountaddress`** is a pure computation; it does not create or
persist an address although the source comment and help say it may
(`methods.go:1890-1895`).

**Q25. `consolidate`** sends to a new internal (change) address when no
address is given (`wallet/createtx.go:819-822`); an empty `address` string
is treated as absent.

**Q26. Import rescans.** Errors of the background rescan are discarded
(`methods.go:2062`); a duplicate import skips the rescan even when the
caller supplied a new `scanfrom`.

**Q27. `fundrawtransaction` side effects.** A wallet-derived change address
advances the account's internal address index permanently; non-positive
fee rates are passed to the wallet unvalidated; the usage string names the
option `conftarget`, which the decoder ignores as an unknown member.

**Q28. SPV `gettxout`** ignores `tree` for the lookup, reports the wallet
tip as `bestblock`, and reports `coinbase: false` for every unmined
output.

**Q29. `getmultisigoutinfo`** cannot distinguish "unmined" from a block at
height 0 with a zero hash (`wallet/common.go:27-32`) and always queries
the regular tree.

**Q30. `getblockheader` does one more ancestor lookup than `getblock`**
(5.2.5).

**Q31. `omitempty` in verbose transactions** drops `blockindex` for the
first transaction of each tree and `blockheight` for genesis (4.6).

**Q32. Vin shape fall-through** for an empty coinbase/stakebase/treasury
spend script (4.7).

**Q33. Go-specific text in messages.** Error messages embed Go library
texts (`encoding/hex: invalid byte: U+0067 'g'`, `encoding/hex: odd length
hex string`, `EOF`, `unexpected EOF`), Go quoting (`%q`, `%#q`) and Go type
names (`*stdaddr.AddressPubKeyEd25519V0`).

**Q34. `getmasterpubkey`** returns an account-level extended public key and
logs a warning on every call.

**Q35. `getpeerinfo` help type.** The documented result type is a single
object (`rpchelp/methods.go:63`); the method returns an array.

**Q36. Dead code.** The `filterAddrMap` parameter of `createVoutList` is
always nil; `makeResponse`'s marshal-failure branch is unreachable; the
serialization-failure returns of `createrawtransaction`
(`methods.go:737-740`), `fundrawtransaction` (`962-965`) and
`gettransaction` (`2645-2648`) cannot trigger when writing to an in-memory
builder;
`confirms` duplicates `wallet.confirms`; the `NotExist` handling after
`AccountBalance`/`AccountName`/`CurrentAddress` can trigger only if the
account disappears between two reads (accounts are never deleted) or, for
`CurrentAddress`, for the imported account.

**Q37. Large `minconf` counts everything.** In `getreceivedbyaccount` and
`getreceivedbyaddress`, `minconf > tip height + 1` yields a negative stop
height, which the transaction range interprets as "include unmined": the
result is the zero-confirmation total rather than 0
(`wallet/wallet.go:4486-4492`, `4540-4544`;
`wallet/udb/txquery.go:391-396`, `534`).

**Q38. Unguarded `Transactions[0]` in SPV `getblock`.** The verbose path
evaluates `blk.Transactions[0].Version` (`methods.go:1464`) without
checking that the regular tree is non-empty; it runs after the non-verbose
return and before the `verbosetx` test, so it is reached for every verbose
request. A block with no regular transaction would panic with an index
out of range (section 7, panics). The SPV layer only checks that the
transaction trees match the header's merkle commitments
(`spv/backend.go:67-91`), so reaching this needs a peer to serve a block
whose header commits to an empty regular tree; no consensus-valid block
does. UNVERIFIED whether the p2p decoding layer rejects such a block
earlier (settled by reading `p2p/peering.go` block receipt and
`validate.MerkleRoots`, owned by spec 14).

**Q39. Defaults by name versus by number.** `getunconfirmedbalance`,
`getnewaddress` and `getrawchangeaddress` default to the account **named**
`default` (`methods.go:1931`, `2329`, `2360`); `consolidate` and
`getmasterpubkey` default to account **number** 0 (`methods.go:573`,
`2463`). Account 0 can be renamed (only the imported account is reserved,
`wallet/udb/addressmanager.go:91-99`, `2343-2350`); after `renameaccount
default x` the first three answer `-11` `account not found` when called
without an account, while the last two keep working.

**Q40. Stale comments and TODOs in the source** (none changes behaviour;
recorded so they are not mistaken for requirements):

| Location | Comment | Reality |
|---|---|---|
| `methods.go:281-283` | `makeResponse` doc comment ends mid-sentence ("but must be") | no missing behaviour |
| `methods.go:357-361`, `385-387` | `accountSyncAddressIndex` error and "no longer changes" claims | section 5.3.3 |
| `methods.go:564-565` | `consolidate` "returning the txHash and error" | returns the hash string |
| `methods.go:1890-1895` | `getAccountAddress` "can fail if the keypool runs out (and will return dcrjson.ErrRPCWalletKeypoolRanOut)" | that code (`-12`) is never produced anywhere in this range; there is no keypool |
| `methods.go:2030` | "Yes, Label is the account name." | `importprivkey.label` is compared with `imported` |
| `methods.go:2194-2196` | `createNewAccount` "creating and returning a new account" | returns `null` |
| `methods.go:2351-2352` | "bitcoind allows specifying the account as an optional parameter, but ignores the parameter" | the wallet does not ignore it |
| `methods.go:2408-2410` | `TODO: This is more inefficient that it could be…` | `getreceivedbyaccount` computes the totals of all accounts and returns one |
| `methods.go:2489` | `getPeerInfo` "views the data on remote peers when in spv mode" | also answers in dcrd mode |
| `methods.go:2593-2594` | `getTickets` "tickets currently owned by wallet" | every unexpired ticket in the store (5.3.34) |
| `methods.go:2633` | "returns nil details when not found" | `TxDetails` returns kind `NotExist` |
| `methods.go:2650-2651`, `2658` | `TODO: Add a "generated" field to this result type` and a commented-out `Generated:` assignment | `gettransaction` has no `generated` member; a port must not add one |
| `methods.go:2715-2718` | `getTxOut` "(nil, nil) is returned if the transaction output … is spent by another transaction already in the main chain" | matches 5.3.36 |
| `methods.go:1333`, `1458` | "unset because wallet is not equipped to easily calculate the cummulative chainwork" | `chainwork` is `""` |

**Q41. `getpeerinfo` with a backend that is neither syncer type** would
return the nil slice, i.e. JSON `null` (`methods.go:2502-2509`). No such
backend exists at the pin; a port with a third backend kind has to choose.

**Q42. `gettransaction` conversions of `ListTransactionDetails`.** The
handler copies only seven members of each `ListTransactionsResult`
(`methods.go:2698-2706`); the `confirmations`, `generated`, `blockhash`,
`blocktime`, `txid`, `walletconflicts`, `time`, `timereceived` and
`txtype` members that the wallet function fills per entry are dropped, so
the per-transaction type computed there never reaches the client (Q11).

### 10.2 Help text versus behaviour

Help descriptions are in `internal/rpchelp/helpdescs_en_US.go`; usage lines
are generated into `rpcserverhelp.go`. The texts must be reproduced byte
for byte (spec 16); the table records where they are wrong so nobody
"fixes" behaviour to match them.

| Method | Help says | Code does |
|---|---|---|
| `accountaddressindex` | "Get the current address index" | returns the next index to be handed out (`lastUsed + 1 + cursor`) |
| `accountsyncaddressindex` | source comments promise an error when the pool is already beyond the index and "no longer changes the child indexes" | no such error; the in-memory cursor is reassigned (5.3.3) |
| `addmultisigaddress` | result: "The imported pay-to-script-hash address" (string); `account`: "DEPRECATED -- Unused" | result `{}`; `account` is validated and must be `imported` when present |
| `addtransaction` | "Manually record a transaction mined in a main chain block" | inclusion is not verified; unknown block means unmined (Q19) |
| `auditreuse` | `--result0--value`: "Reused address"; `--result0--key`: "Array of outpoints…" | key and value descriptions are swapped; `since` filters addresses, not outpoints |
| `consolidate` | `address` default "obtained via getnewaddress from the account's address pool" | default is a new internal (change) address |
| `createrawtransaction` | `expiry`: "a non-zero value when the transaction expiry" (garbled) | sets the expiry height |
| `debuglevel` | subsystems "CMGR, DCRW, GRPC, LODR, MIXC, MIXP, PEER, RPCS, SYNC, TKBY, VSPC, and WLLT" | `show` also lists `AMGR` (`log.go:56`) |
| `fundrawtransaction` | usage: `{"changeaddress":…,"feerate":…,"conftarget":…}` | the key is `conf_target` |
| `getaccountaddress` | "A new address is generated for the account if the most recently generated address has been seen" | nothing is generated (Q24) |
| `getbalance` | synopsis "Calculates and returns the balance of all accounts." | also serves a single account |
| `getblockheader` (SPV) | `chainwork`: "(not set in SPV mode)"; `hash`: "(same as provided)" | `chainwork` is present as `""`; `hash` is canonicalised |
| `getblock` (SPV) | `chainwork`: "The total number of hashes expected…" | always `""` |
| `getinfo` | `walletversion`: "The version of the address manager database" | software version integer |
| `getmasterpubkey` | "Requests the master pubkey from the wallet" | account extended public key |
| `getmultisigoutinfo` | `index`: "Index of input."; `hash`: "Input hash to check." | output index and transaction hash of the multisig output |
| `getpeerinfo` | "Returns data on remote peers when in spv mode."; single-object result type | works in dcrd mode too; array result |
| `getstakeinfo` | `difficulty`: "Current stake difficulty"; `live`: "Number of mature, active tickets" | next required stake difficulty computed by the wallet; `live` is not verified against the pool |
| `gettransaction` | category `"recv"`; `type`: "(regular, ticket, vote, or revocation)"; `fee`: "…or 0 if 'txid' is not a sent transaction"; `amount`: "The total amount this transaction credits to the wallet" | category `"receive"`; `type` always `""`; `fee` negative and omitted when zero; `amount` is credits minus debits |
| `gettxout` (SPV) | `bestblock`: "The block hash that contains the transaction output" | wallet tip hash; `null` result for spent/unknown outputs is undocumented |
| `getunconfirmedbalance` | "the unspent output value of all unmined transaction outputs" | `total - spendable` at one confirmation (includes immature and ticket-locked funds) |
| `importcfiltersv2` | "Does not perform validation on the filters" | each blob must parse as a version-2 filter |
| `importpubkey` | no restriction mentioned | fails unless the wallet is watching-only |
| `accountaddressindex` | documented result type `int` (`rpchelp/methods.go:31`) | Go `uint32`; the JSON number is never negative |
| `createrawtransaction` | `transactioninput-tree`: "The tree to generate transaction for" | the tree of the referenced previous output |
| `getaddressesbyaccount` | "All addresses controlled by 'account'" | children 0 up to (not including) the next index to hand out on each branch; gap-limit lookahead addresses are not listed |
| `getblock`, `getblockheader` | only the verbose object is a documented result (`rpchelp/methods.go:55-56`) | a hex string when `verbose` is false (or explicit `null` in SPV mode) |
| `getinfo` | `balance`: "The balance of all accounts calculated with one block confirmation"; documented result type `InfoWalletResult` (`rpchelp/methods.go:59`) | sum of the **spendable** balances only; `InfoWalletResult` is a type alias of `InfoResult` (`types/results.go:194-195`), same members |
| `getmultisigoutinfo` | `spentbyindex`: "Index of spending tx." | input index within the spending transaction |
| `getreceivedbyaccount` | no exception mentioned | always `0` for imported-xpub accounts |
| `getstakeinfo` | `revoked`: "Number of missed tickets that were missed and then revoked" | every ticket spent by a revocation, expired ones included |
| `gettickets` | "the hashes of the tickets currently owned by wallet" | every unexpired ticket recorded in the transaction store, without an ownership test; live filtering only with dcrd (5.3.34) |
| `gettransaction` details | `account`: "DEPRECATED -- Unset"; `amount`: "The amount of a received output"; `address`: "…or the empty string if the output is nonstandard…" | `account` carries the owning account name on receive-side entries (empty on `send` entries); `amount` is the negated output value on `send` entries; an empty `address` is omitted, and is also what a multi-address output produces |
| `gettransaction` | `walletconflicts`: "Unset"; `blockindex`: "Unset" | both are always emitted, as `[]` and `0` |
| `getblockverboseresult-stakeroot`, `-stx`, `-rawstx` | `stakeroot` and `stx`: "The block's sstx hashes the were included"; `rawstx`: "The block's raw sstx hashes the were included" | stake merkle root, stake transaction hashes, stake transaction objects |

The description map also contains entries for methods that do not exist
(`getbalancetomaintain`, `getticketmaxprice`; `helpdescs_en_US.go:179-180`,
`430-431`).

---

## 11. Upstream tests

There is **no test that calls any handler** in this scope.

| Test | File | What it pins for this scope |
|---|---|---|
| `TestRPCMethodHelpGeneration` | `internal/rpc/jsonrpc/rpchelp_test.go:39` | every method in `rpchelp.Methods` has complete help descriptions for its command and result types (so the result types named in `rpchelp/methods.go` are structurally valid) |
| `TestRPCMethodUsageGeneration` | `internal/rpc/jsonrpc/rpchelp_test.go:78` | a one-line usage can be generated for every method, and the joined usages equal the generated `requestUsages` string (otherwise "run 'go generate'") |
| `TestThrottle` | `internal/rpc/jsonrpc/rpcserver_test.go:15` | transport only |
| `TestWalletSvrCmds` | `rpc/jsonrpc/types/methods_test.go:23` | marshal/unmarshal of command structs. In scope: `addmultisigaddress` (2 cases), `createmultisig`, `createnewaccount`, `dumpprivkey`, `getaccount`, `getaccountaddress`, `getaddressesbyaccount`, `getbalance` (3), `getnewaddress` (2), `getrawchangeaddress` (2), `getreceivedbyaccount` (2), `getreceivedbyaddress` (2), `gettransaction` (2), `importprivkey` (4), `importpubkey`, `renameaccount` |
| `TestWalletSvrWsCmds` | `rpc/jsonrpc/types/walletsvrwscmds_test.go:22` | `getunconfirmedbalance` (2 cases) |
| `TestChainSvrCustomResults` | `dcrd/rpc/jsonrpc/types/v4@v4.4.0/chainsvrresults_test.go:16` | the custom `Vin` marshaller shapes (4.7) |

These are all the test files under `internal/rpc/jsonrpc` and
`rpc/jsonrpc/types` at the pin (`rpchelp_test.go`, `rpcserver_test.go`,
`methods_test.go`, `walletsvrwscmds_test.go`); `internal/rpchelp` has
none. Both help tests obtain the method set from `serverMethods()`
(`rpchelp_test.go:27-35`), i.e. the keys of `handlers` with `noHelp ==
false`, delete from it every name in `rpchelp.Methods`, and (the help
test only, `rpchelp_test.go:70-72`) fail with `Missing '<locale>' help for
method '<name>'` for each name left over. A name listed in
`rpchelp.Methods` but absent from `handlers` is not detected. This is the
only upstream check tying the 103 live method names of section 3.1 to the
help tables. Neither `makeResponse`, `lazyApplyHandler`,
`confirms`, `decodeAddress`, `difficultyRatio` nor the vin/vout builders
has a unit test.

Vectors to port directly: the command marshal/unmarshal cases (they pin
parameter order, optionality and defaults) and the `Vin` cases.

Vectors to generate through a Go oracle shim (a small Go program linking
the pinned dcrwallet that opens a fixture wallet database with no backend,
calls handlers through `lazyApplyHandler`-equivalent dispatch and prints
request/response pairs):

- every error path of section 5.3 that does not need a backend (bad
  hashes, bad hex, bad addresses in both `decodeAddress` branches, unknown
  accounts, reserved names, gap policy strings, `createrawtransaction`
  range checks, `createsignature` index checks);
- `createrawtransaction` with single-output maps (deterministic) across
  locktime/expiry combinations;
- amount formatting of `getbalance`, `getreceivedby*`, `gettransaction`
  (including `-0` and sub-microcoin values) on a fixture with known
  transactions;
- `getbalance "*"` ordering with at least 257 accounts and with an
  imported xpub account, and `getreceivedbyaccount` on the same fixture
  (Q4, Q5);
- `addmultisigaddress` (`{}`) and `createmultisig`;
- `auditreuse`, `getaddressesbyaccount`, `getmultisigoutinfo`,
  `getstakeinfo` (SPV form), `gettickets` (nil querier) on a fixture with
  tickets;
- SPV-form `getblockheader` (hex and verbose, main chain, side chain,
  genesis, heights below and above 11) and `gettxout` by installing a stub
  backend that is not a `*chain.Syncer`;
- `getblock` verbose and `verbosetx` objects for blocks containing a
  coinbase, treasurybase, treasury spend, ticket, vote and revocation,
  using a stub backend that serves fixture blocks;
- `difficultyRatio` for the network limits and a sample of mainnet bits.

Untested upstream and needing fixtures with a live or mocked dcrd:
passthrough behaviour, `getinfo` merge, `getpeerinfo` filtering,
`getstakeinfo` precise form, `gettickets` live filtering, `discoverusage`,
`consolidate`, `fundrawtransaction`, the import rescans.

---

## 12. Rust porting notes

### 12.1 Suggested boundary

One module tree inside the JSON-RPC server crate (spec 16 proposes
`purse-jsonrpc`), with handlers grouped by concern rather than one file:

| Module | Contents |
|---|---|
| `handlers::helpers` | `confirms`, `decode_address`, `decode_stake_address`, `wallet_pub_keys`, `difficulty_ratio`, block-level values of 5.2.5, hash parsing with Go-compatible error text, amount conversion |
| `handlers::txresults` | `create_tx_raw_result`, `create_vin_list`, `create_vout_list` (wallet variants) |
| `handlers::accounts` | `accountaddressindex`, `accountsyncaddressindex`, `createnewaccount`, `renameaccount`, `getaccount`, `getaccountaddress`, `getaddressesbyaccount`, `getnewaddress`, `getrawchangeaddress`, `getmasterpubkey`, `importxpub` |
| `handlers::balances` | `getbalance`, `getunconfirmedbalance`, `getreceivedbyaccount`, `getreceivedbyaddress`, `getinfo` |
| `handlers::chain` | `getbestblock`, `getbestblockhash`, `getblockcount`, `getblockhash`, `getblockheader`, `getblock`, `gettxout`, `getcfilterv2`, `importcfiltersv2`, `syncstatus`, `getcurrentnet`, `getpeerinfo`, `discoverusage` |
| `handlers::transactions` | `abandontransaction`, `addtransaction`, `auditreuse`, `consolidate`, `createrawtransaction`, `createsignature`, `fundrawtransaction`, `gettransaction` |
| `handlers::keys` | `addmultisigaddress`, `createmultisig`, `dumpprivkey`, `importprivkey`, `importpubkey`, `importscript`, `getmultisigoutinfo` |
| `handlers::staking` | `disapprovepercent`, `getstakeinfo`, `gettickets` |

Handlers should be written against a wallet trait and a backend enum
(`DcrdRpc`, `Spv`, `None`) so the oracle vectors can run with stubs.

### 12.2 Reuse (crates inspected)

| Need | Candidate | Notes |
|---|---|---|
| Command registry, positional decoding, Go-compatible JSON encoding (struct order, `omitempty`, float32/float64 formatting, HTML escaping, `%q` quoting) | `dcroxide-dcrjson` (`dcroxide:crates/dcroxide-dcrjson/src/lib.rs`, `gojson.rs`: `encode`, `try_encode`, `format_float_json`, `format_float_json32`, `go_quote`, `append_json_string`) | Results are described as `GoType`/`GoValue` trees; `try_encode` reports the unsupported-value case needed for `syncstatus`. A `%#q` (backquote) helper was not found and has to be added. |
| dcrd result descriptors (`GetBlockVerboseResult`, `GetBlockHeaderVerboseResult`, `TxRawResult`, `GetBestBlockResult`, `InfoChainResult`) and the `Vin` marshaller | `dcroxide-rpctypes` (`dcroxide:crates/dcroxide-rpctypes/src/chainsvrresults.rs`) | The wallet result structs of 4.3 need new descriptors. |
| `createVinList`, `createVoutList`, `createTxRawResult`, `to_coin` | `dcroxide-rpc` (`dcroxide:crates/dcroxide-rpc/src/txresults.rs:24`, `142`, `230`, `318`) | Same algorithm for vins. The vout builder sets `scriptPubKey.version` to the output's version and logs lower-case messages; the wallet variant must emit 0 and the wallet's log text (Q15). `create_tx_raw_result` has a different signature (dcrd passes hashes and height separately). Copy and adapt rather than depend on the node crate. |
| Difficulty ratio | `dcroxide-rpc` `get_difficulty_ratio` (`dcroxide:crates/dcroxide-rpc/src/helpers.rs:223`) | Identical computation; returns 0 for a zero target where Go panics. |
| Address decoding and types, script type names, `ExtractAddrs`, `DetermineRequiredSigs`, `MultiSigScriptV0`, disassembly | `dcroxide-txscript` (`stdaddr.rs:762`, `778`; `stdscript.rs:833`, `842`, `1157`; `script.rs:117`) | Error texts must match the Go ones quoted in 5.2.1 and 5.3.4; verify against the oracle. |
| Stake/treasury classification, commitment decoding | `dcroxide-stake` (`lib.rs:392`, `417`, `1117`, `1214`; `treasury.rs:290`), `dcroxide-standalone` (`tx.rs:40`, `107`; `pow.rs:33`) | |
| Transactions, headers, PoW hashes | `dcroxide-wire` (`msgtx.rs`, `blockheader.rs:158`, `164`) | |
| Hash parsing with the Go leniency | `dcroxide-chainhash` (`FromStr`), plus the Go error text helpers in `dcroxide-rpc` (`helpers.rs:57`, `72`: `go_hex_invalid_byte_error`, `go_hash_decode_error`) | |
| HD keys, WIF, simple address/amount helpers | `dcr-rs` (`dcr-rs:src/hd.rs`, `src/address.rs`, `src/amount.rs`) | `dcr-rs` addresses cover P2PKH/P2SH only and its amount module formats strings; the handlers need the full `stdaddr` set and float semantics, so prefer the dcroxide crates here and use `dcr-rs` only if the wallet core already depends on it for key derivation. |

### 12.3 Hazards

- **Float semantics.** Compute amounts as `atoms as f64 / 1e8` and format
  with the Go-compatible formatter; do not use decimal arithmetic. Sum in
  integer atoms before converting, exactly where the Go does.
  `headersfetchprogress` must be computed in `f32`.
- **Integer conversions.** Reproduce the truncations of Q7 with wrapping
  casts (`as`), and decide the out-of-range float case of Q8 explicitly.
- **Go strings in errors** (Q33): hex and I/O error texts, `%q`/`%#q`
  quoting, `%T` type names, `%v` of amounts. Keep a table of Go texts
  rather than using Rust library messages.
- **Iteration order.** Account order is bbolt key order of little-endian
  u32 keys (Q4, Q5); the `auditreuse` object is key-sorted; SPV peers are
  sorted by id; `createrawtransaction` outputs need a chosen order (Q6).
- **Nil versus empty** per member, as stated in section 4.
- **Echoing raw parameter strings** (Q10) requires keeping the original
  text alongside the parsed hash.
- **Non-atomic reads.** Using one snapshot for a whole handler is a
  behavioural improvement that cannot be distinguished by a client except
  under races; it is safe, but the `MainChainTip`-cannot-fail behaviour
  should be kept so the same requests succeed.
- **Background rescans** must outlive the request and be joined at
  shutdown.
- **Panics** (Q3) must not become process aborts by accident nor silently
  change into success; see decisions.

### 12.4 Size estimate

**L.** 48 handlers and about 2,800 lines of Go, most of them thin, but
with roughly 110 distinct error exits, 20 result shapes, three
backend-dependent methods with full SPV implementations, and exact
text/number formatting throughout. The helpers and result descriptors can
be adapted from dcroxide in days; the bulk of the effort is the oracle
harness and fixture wallet needed to pin behaviour that upstream never
tested, and the dependency on wallet-core methods (specs 06 to 11) being
available behind a trait.

---

## 13. Open questions

1. **Reproduce or fix the defects?** `addmultisigaddress` returning `{}`
   (Q2), `getreceivedbyaccount` mis-indexing (Q4), `gettransaction.type`
   empty (Q11), the `auditreuse` nil dereference (Q3) and the
   non-finite `syncstatus` (Q14). A byte-compatible port reproduces Q2,
   Q4 and Q11; Q3 and Q14 are crashes with no useful behaviour to
   preserve. Needs a project-level rule.
2. **`createrawtransaction` output order** (Q6): Go is random. Candidates:
   JSON document order (what callers expect) or sorted by address. Either
   is within Go's observable behaviour.
3. **Exactness of error messages built from Go library errors** (Q33):
   byte-identical (requires a Go-text table and oracle coverage) or
   code-identical with best-effort text.
4. **Float-to-integer overflow in amounts** (Q8): replicate amd64 Go
   (`i64::MIN`) or saturate.
5. **UNVERIFIED: exact dcrd handling of forwarded `null` optional
   parameters** for `getblock`, `getblockheader`, `gettxout`. The wallet
   forwards `null` verbatim; what dcrd answers is defined by dcrd (and by
   dcroxide), not by this scope. Settled by a dcrd oracle run.
6. **UNVERIFIED: the complete list of error kinds and texts produced
   inside wallet-core calls** (`Consolidate`, `NewUnsignedTransaction`,
   `DiscoverActiveAddresses`, `FetchP2SHMultiSigOutput`,
   `ImportedAddresses`, the address manager's private-key path). This spec
   quotes those read during the deep dive; the owning specs (02, 04, 06 to
   10) are authoritative and should be cross-checked with the oracle.
7. **UNVERIFIED: `importprivkey` with a non-ECDSA or unknown-type WIF.**
   `dcrutil.DecodeWIF` succeeds for Ed25519 and Schnorr keys and returns
   an empty key for an unknown signature-type byte
   (`dcrd/dcrutil/v4@v4.0.3/wif.go:137-160`); the outcome depends on
   `udb.Manager.ImportPrivateKey` (spec 02) and should be pinned with the
   oracle.
8. **SPV `getblock` for unknown hashes** (Q13): keep the unbounded retry or
   add a timeout. A timeout changes behaviour only for requests that
   currently never complete.
9. **Websocket panic policy**: Go aborts the process on a handler panic
   over websocket and drops the connection over POST. The Rust server
   needs a single documented policy (suggested: catch, log, reply
   `-32603`), which is a deliberate divergence.
10. **UNVERIFIED: can an SPV peer deliver a block with an empty regular
    tree to `getblock`?** (Q38). If yes, the Go panics at
    `methods.go:1464`; the port needs a defined answer (suggested:
    `-32603`). Settled by the block-receipt path in spec 14.
11. **Third backend kind** (Q41): if dcr-purse ever has a backend that is
    neither the dcrd-RPC syncer nor the SPV syncer, `getpeerinfo` (JSON
    `null` in Go), `getblock` (Go would call `Blocks` on it),
    `getblockheader` and `gettxout` (wallet-data path) need explicit
    rules.
