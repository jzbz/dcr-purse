# 16 — JSON-RPC server: transport, authentication, command/result types, help text, error codes

## 1. Header

**Scope** (dcrwallet, line counts from `wc -l`):

| File | Lines | Covered here |
|---|---|---|
| `internal/rpc/jsonrpc/server.go` | 625 | all |
| `internal/rpc/jsonrpc/errors.go` | 111 | all |
| `internal/rpc/jsonrpc/config.go` | 42 | all |
| `internal/rpc/jsonrpc/context.go` | 21 | all |
| `internal/rpc/jsonrpc/marshaling.go` | 48 | all |
| `internal/rpc/jsonrpc/log.go` | 16 | all |
| `internal/rpc/jsonrpc/rpcserverhelp.go` | 117 (93,520 bytes, generated) | all |
| `internal/rpc/jsonrpc/rpchelp_test.go`, `rpcserver_test.go` | 111, 54 | tests |
| `internal/rpc/jsonrpc/methods.go` | 5863 | only the dispatch layer: lines 54-60 (API version), 84-306 (handler table, `lazyApplyHandler`, `makeResponse`), 798-815 (`debugLevel`), 1764-1766 (`getCurrentNet`), 2857-2937 (`help`), 5438-5462 (`version`), plus a scan of every handler for network-backend and lock handling. Per-method semantics belong to sibling specs. |
| `internal/rpchelp/genrpcserverhelp.go` | 100 | all |
| `internal/rpchelp/helpdescs_en_US.go` | 1044 | all |
| `internal/rpchelp/methods.go` | 142 | all |
| `rpc/jsonrpc/types/doc.go` | 73 | all |
| `rpc/jsonrpc/types/methods.go` | 1336 | all |
| `rpc/jsonrpc/types/results.go` | 416 | all |
| `rpc/jsonrpc/types/methods_test.go`, `walletsvrwscmds_test.go` | 1306, 192 | tests |
| `rpcserver.go` (root, package `main`) | 512 | all |

Supporting code read to pin observable behaviour: `ipc.go:20-215` (pipe
messages), `config.go` (RPC option names and defaults only),
`internal/cfgutil/curve.go`, `internal/loader/loader.go:364-404`,
`wallet/network.go:59-71`, `errors/errors.go`, `version/version.go`,
`dcrwallet.go:373-411`.

**Parity pin**

| Component | Version |
|---|---|
| dcrwallet | master `54b85b33`, version 2.2.0-pre, module `decred.org/dcrwallet/v5` |
| `github.com/decred/dcrd/dcrjson/v4` | v4.2.0 (all non-test files read) |
| `github.com/decred/dcrd/rpc/jsonrpc/types/v4` | v4.4.0 (the 19 command types and the result types the wallet reuses) |
| `github.com/decred/dcrd/certgen` | v1.2.0 |
| `github.com/gorilla/websocket` | v1.5.1 (`server.go` upgrade path, `conn.go` limits) |
| `github.com/jrick/wsrpc/v2` | v2.3.8 (`Error` type only) |
| Go toolchain | `go 1.26.0` in `go.mod`; `net/http` and `encoding/json` behaviour cited from the Go standard library |

**Summary**

The JSON-RPC server is a Go `net/http` server with two routes on every
listener: `/` (one JSON-RPC request per HTTP POST, connection closed after
the reply) and `/ws` (gorilla websocket, many concurrent requests per
connection). TLS is on by default with a self-generated certificate;
authentication is HTTP Basic (compared by SHA-256 of the whole
`Authorization` header value) or TLS client certificates. The API version is
the constant `11.0.0`, reported by the `version` method under the key
`dcrwalletjsonrpcapi`. There are 103 implemented methods, 8 dead
"unimplemented/unsupported" table entries, one websocket-only method
(`authenticate`) and one transport-level method (`stop`); any other method
name is forwarded verbatim to dcrd when the wallet syncs through dcrd RPC.
Batched requests are not supported. Every parameter-parsing failure is
reported as `-32600 Invalid request`. Help text is a static, pre-generated
string table (103 entries, 81,739 bytes of text) plus a 103-line usage
string; both must be embedded byte-for-byte.

---

## 2. Role in the system

The subsystem exposes the wallet to `dcrctl --wallet`, Decrediton's
JSON-RPC consumers, exchange integrations and scripts that speak the
Bitcoin-Core-style JSON-RPC 1.0/2.0 dialect.

Callers and callees:

- `main.startRPCServers` (`rpcserver.go:248`) is called once from
  `dcrwallet.go:380` after the loader exists and before the wallet is
  necessarily open. It builds the TLS configuration, the listeners and both
  the gRPC server and the JSON-RPC server. The gRPC parts of `rpcserver.go`
  are described here only where they share TLS state with JSON-RPC; the gRPC
  API itself is a sibling spec.
- `jsonrpc.NewServer` (`internal/rpc/jsonrpc/server.go:93`) receives the
  process context, the `Options`, the chain parameters, the wallet
  `loader.Loader` and the already-open listeners. It starts serving
  immediately.
- Every handler obtains the wallet lazily through
  `walletLoader.LoadedWallet()` and the network backend through
  `walletLoader.NetworkBackend()` or `wallet.NetworkBackend()`; the server
  therefore runs before a wallet is opened and survives the backend
  changing between dcrd RPC sync (`*chain.Syncer`), SPV (`*spv.Syncer`),
  offline (`wallet.OfflineNetworkBackend`) and none.
- Request and response envelopes, parameter parsing, usage and help
  generation come from `dcrjson/v4`. Command structs are registered with
  dcrjson in `rpc/jsonrpc/types/methods.go:1182-1313` under the Go type
  `types.Method`.
- The process is asked to shut down through
  `Server.RequestProcessShutdown()` (a channel read by `dcrwallet.go:403`).
- Logging goes to the `RPCS` subsystem logger (`log.go:55`,
  `internal/loggers/loggers.go:53`); gRPC interceptors log to `GRPC`.

---

## 3. Public surface inventory

### 3.1 Package `jsonrpc` (`internal/rpc/jsonrpc`)

| Item | Kind | Signature / value | Contract |
|---|---|---|---|
| `Options` | struct | `Username, Password string; MaxPOSTClients, MaxWebsocketClients int64; MixingEnabled bool; MixAccount string; MixBranch uint32; MixChangeAccount string; TicketSplitAccount string; VSPHost, VSPPubKey string; Dial func(ctx, network, addr) (net.Conn, error); Loggers Loggers` (`config.go:13-31`) | Copied by value into the server. Empty `Username` or `Password` disables Basic auth entirely. The mixing/VSP/Dial fields are read by method handlers (sibling specs). |
| `Loggers` | interface | `Subsystems() []string`; `SetLevels(levelSpec string) error` (`config.go:34-42`) | Backing for `debuglevel`. Implemented by `main.rpcLoggers`. |
| `Server` | struct | fields `httpServer http.Server`, `walletLoader`, `listeners`, `authsha *[32]byte`, `upgrader websocket.Upgrader`, `cfg Options`, `wg`, `quit chan struct{}`, `quitMtx`, `requestShutdownChan chan struct{}` (cap 1), `activeNet *chaincfg.Params` (`server.go:62-78`) | One per process. |
| `NewServer` | func | `NewServer(ctx context.Context, opts *Options, activeNet *chaincfg.Params, walletLoader *loader.Loader, listeners []net.Listener) *Server` (`server.go:93`) | Registers `/` and `/ws`, starts one serving goroutine per listener, returns without blocking. `ctx` is the base context of every request. |
| `(*Server).Stop` | method | `Stop()` (`server.go:216`) | Idempotent. Closes listeners, closes `quit`, waits for the server wait group. |
| `(*Server).RequestProcessShutdown` | method | `RequestProcessShutdown() <-chan struct{}` (`server.go:623`) | Receives one value per accepted `stop` request. |
| `UseLogger` | func | `UseLogger(logger slog.Logger)` (`log.go:14`) | Sets the package logger; default is disabled. |
| `handler` | struct | `fn func(*Server, context.Context, any) (any, error); noHelp bool` (`server.go:80-83`) | Entry type of the method table. |
| `handlers` | var | `map[string]handler` (`methods.go:85-200`) | The method table, section 4.9. |
| `unimplemented`, `unsupported` | funcs | `(*Server, context.Context, any) (any, error)` (`methods.go:204`, `213`) | Return `-1 "Method unimplemented"` and `-1 "Request unsupported by dcrwallet"`. Unreachable; see Quirk Q1. |
| `lazyHandler` | type | `func() (any, *dcrjson.RPCError)` (`methods.go:223`) | Deferred execution of one request. |
| `lazyApplyHandler` | func | `(s *Server, ctx, request *dcrjson.Request) lazyHandler` (`methods.go:229`) | Dispatch: table handler or dcrd passthrough. Section 5.5. |
| `makeResponse` | func | `(id, result any, err error) dcrjson.Response` (`methods.go:284`) | Builds a response struct with an empty `Jsonrpc` field; used only by three websocket paths. |
| `jsonrpcSemverString/Major/Minor/Patch` | consts | `"11.0.0"`, 11, 0, 0 (`methods.go:55-60`) | API version, section 4.8. |
| `websocketClient` | struct | `conn *websocket.Conn; authenticated bool; allRequests, responses chan []byte (unbuffered); cancel func(); quit chan struct{}; wg sync.WaitGroup` (`server.go:30-38`) | Per-connection state. |
| `newWebsocketClient`, `(*websocketClient).send` | funcs | `server.go:40`, `51` | `send` blocks until the sender goroutine takes the message or `quit` closes (then returns `"websocket client disconnected"`). |
| `jsonAuthFail` | func | `(w http.ResponseWriter)` (`server.go:86`) | 401 with `WWW-Authenticate: Basic realm="dcrwallet RPC"`. |
| `httpBasicAuth` | func | `(username, password string) []byte` (`server.go:186`) | `"Basic " + base64std(username + ":" + password)`. |
| `(*Server).serve` | method | `(lis net.Listener)` (`server.go:204`) | Goroutine running `httpServer.Serve(lis)`. |
| `(*Server).handlerClosure` | method | `(ctx, request) lazyHandler` (`server.go:249`) | Logs `RPC method %q invoked by %v` at debug and calls `lazyApplyHandler`. |
| `errNoAuth` | var | `errors.E("missing Authorization header")` (`server.go:256`) | Sentinel distinguishing "no header" from "wrong header". |
| `(*Server).checkAuthHeader` | method | `(r *http.Request) error` (`server.go:262`) | Section 5.2. |
| `throttledFn`, `throttled` | funcs | `(threshold int64, f) http.Handler` (`server.go:281`, `287`) | Concurrency cap answering 429. |
| `idPointer` | func | `(id any) *any` (`server.go:308`) | `nil` for a nil id. |
| `(*Server).invalidAuth` | method | `(req *dcrjson.Request) bool` (`server.go:318`) | Validates a websocket `authenticate` request. |
| `websocketClientRead/Respond/Send`, `websocketClientRPC` | methods | `server.go:338`, `354`, `473`, `507` | Websocket lifecycle, section 5.4. |
| `maxRequestSize` | const | `1024 * 1024 * 4` (`server.go:531`) | POST body cap. |
| `(*Server).postClientRPC` | method | `(w, r)` (`server.go:534`) | POST lifecycle, section 5.3. |
| `(*Server).requestProcessShutdown` | method | `server.go:617` | Blocking send on the cap-1 channel. |
| `contextKey`, `withRemoteAddr`, `remoteAddr` | type/funcs | `context.go:9-21` | Stores the peer address under key `contextKey("remote-addr")`; `remoteAddr` returns `"<unknown>"` when absent. |
| `convertError`, `rpcError`, `rpcErrorf` | funcs | `errors.go:16`, `51`, `58` | Error mapping, section 8. |
| `errUnloadedWallet` … `errReservedAccountName` | vars | `errors.go:66-111` | Fixed errors, section 8.3. |
| `marshalJSONFunc`, `addressArrayMarshaler`, `knownAddressMarshaler`, `addressStringsMarshaler` | type/funcs | `marshaling.go:10-48` | Streaming JSON array-of-strings marshalers, section 4.7. |
| `helpDescsEnUS`, `localeHelpDescs`, `requestUsages` | generated | `rpcserverhelp.go:5`, `113`, `117` | Static help table, section 4.10. |
| `helpDescs`, `helpDescsMu` | vars | `methods.go:2875-2876` | Lazily built help map and its mutex. |
| `(*Server).help`, `(*Server).version`, `(*Server).debugLevel`, `(*Server).getCurrentNet` | methods | `methods.go:2882`, `5438`, `798`, `1764` | Transport-adjacent handlers specified here (sections 5.6-5.8). |

### 3.2 Package `rpchelp` (`internal/rpchelp`)

| Item | Kind | Contract |
|---|---|---|
| `Methods` | `[]struct{ Method string; ResultTypes []any }` (`methods.go:26-133`) | 103 entries; order defines the order of `requestUsages`. Result types drive the "Result" block of each help text. |
| `HelpDescs` | `[]struct{ Locale, GoLocale string; Descs map[string]string }` (`methods.go:136-142`) | One entry: `{"en_US", "EnUS", helpDescsEnUS}`. |
| `returnsBool`, `returnsNumber`, `returnsString`, `returnsStringArray`, `returnsLTRArray` | vars (`methods.go:17-21`) | Each is a one-element `[]any` result-type list: `[]any{(*bool)(nil)}`, `[]any{(*float64)(nil)}`, `[]any{(*string)(nil)}`, `[]any{(*[]string)(nil)}`, `[]any{(*[]types.ListTransactionsResult)(nil)}`. `help` uses `append(returnsString, returnsString[0])` (two `*string` entries, `methods.go:75`). |
| `helpDescsEnUS` | `map[string]string` (`helpdescs_en_US.go:10-1044`) | Description keys, section 4.10.3. |
| generator `main` | `genrpcserverhelp.go` (build tag `generate`) | Writes `rpcserverhelp.go`; section 5.9. |
| generator `init` | `genrpcserverhelp.go:21-23` | `log.SetFlags(log.Lshortfile \| log.LstdFlags)`; affects only the generator's own fatal messages. |
| generator `outputFile` | package-level `*os.File` (`genrpcserverhelp.go:25-31`) | `os.Create("rpcserverhelp.go")` in the current directory at package initialisation, i.e. before `main` runs; the file is truncated even if generation then fails. |
| generator `writefln` | `(format string, args ...any)` (`:33-42`) | `fmt.Fprintf` to the output file followed by one `\n`; any write error is fatal. |
| generator `writeLocaleHelp` | `(locale, goLocale string, descs map[string]string)` (`:44-58`) | Emits `func helpDescs<GoLocale>() map[string]string { return map[string]string{ ... } }` with one `%q: %q,` line per `rpchelp.Methods` entry, in `Methods` order; a `GenerateHelp` error is fatal. |
| generator `writeLocales` | `()` (`:60-66`) | Emits `var localeHelpDescs = map[string]func() map[string]string{ "<Locale>": helpDescs<GoLocale>, }`. |
| generator `writeUsage` | `()` (`:68-79`) | Emits `var requestUsages = %q` of the `MethodUsageText` strings joined with `\n`. |

Build tags: `internal/rpchelp/methods.go` and `helpdescs_en_US.go` carry
`//go:build !generate`; `genrpcserverhelp.go` carries `//go:build generate`
and is `package main` in the same directory. The generator is therefore run
as a single file (`go run ../../rpchelp/genrpcserverhelp.go jsonrpc`) and
imports the `rpchelp` package normally.

### 3.3 Package `types` (`rpc/jsonrpc/types`)

- `type Method string` (`methods.go:17`): the registration key type. dcrd's
  own types package registers the same method strings under a different Go
  type, so the two registries do not collide and the wallet only ever parses
  with wallet registrations.
- 85 wallet command structs, 19 newtypes of dcrd command structs
  (`methods.go:1316-1336`), helper structs `FundRawTransactionOptions`,
  `RawTxInput`, `AddressAmountPair`, and `CreateUnsignedTicketResult`
  (declared in `methods.go:689`). All are tabulated in section 4.4.
- 60 constructors named `New<Foo>Cmd`. Each only copies its arguments into
  the struct; none validates. They are client conveniences and carry no
  server behaviour. Four have signatures that do not cover the struct:
  `NewSendToTreasuryCmd(amount float64, comment, commentTo *string)` ignores
  both strings (`methods.go:853`); `NewListUnspentCmd(minConf, maxConf
  *int, addresses *[]string)` cannot set `Account` (`methods.go:641`);
  `NewPurchaseTicketCmd` cannot set `DontSignTx` (`methods.go:676`);
  `NewListLockUnspentCmd()` cannot set `Account` (`methods.go:507`).
  Commands without a constructor: `abandontransaction`, `accountunlocked`,
  `addtransaction`, `auditreuse`, `createsignature`, `disapprovepercent`,
  `discoverusage`, `getcoinjoinsbyacct`, `importcfiltersv2`, `importxpub`,
  `lockaccount`, `mixaccount`, `mixoutput`, `processunmanagedticket`,
  `rescanwallet`, `setaccountpassphrase`, `setdisapprovepercent`,
  `spendoutputs`, `syncstatus`, `ticketinfo`, `treasurypolicy`,
  `tspendpolicy`, `unlockaccount`, `validatepredcp0005cf`,
  `walletpubpassphrasechange`, and all 19 dcrd newtypes.
- Result structs and the `ListTransactionsTxType` string constants
  (`results.go`), tabulated in section 4.6.
- `init()` (`methods.go:1182`) registers 104 methods with
  `dcrjson.MustRegister`: 103 with flags `0` and `authenticate` with
  `dcrjson.UFWebsocketOnly`. Registration panics on any violation, so a
  malformed command struct is a start-up failure, never a runtime error.
  Registration order inside `init` is: the 85 wallet commands in the
  alphabetical-ish order of `methods.go:1189-1275`, then the 18 dcrd
  newtypes (`1281-1300`), then `authenticate` (`1307-1312`). Order has no
  observable effect (the registry is a map; `dcrjson.RegisteredMethods`
  sorts).
- `doc.go` (73 lines) is package documentation only, copied from dcrd's
  types package. Two statements in it are wrong for this package and must
  not be taken as contract: it lists "Notifications (type `<Foo>Ntfn`)"
  although the package defines no notification type, and it tells readers
  to "use UnmarshalCmd", a function that does not exist in dcrjson v4.2.0
  (the function is `ParseParams`) (`rpc/jsonrpc/types/doc.go:22`, `32-34`).

#### 3.3.1 Constructors (`rpc/jsonrpc/types/methods.go`)

All 60, with the line of the declaration. Each returns a pointer to the
command struct named by dropping the `New` prefix, and assigns its
parameters to the struct fields in declaration order unless noted. Pointer
parameters are stored as given (nil stays nil; **constructors never apply
`jsonrpcdefault` values**, those are applied only by `ParseParams`).

| Constructor | Line |
|---|---|
| `NewAccountAddressIndexCmd(acct string, branch int)` | 34 |
| `NewAccountSyncAddressIndexCmd(acct string, branch int, idx int)` | 51 |
| `NewAddMultisigAddressCmd(nRequired int, keys []string, account *string)` | 72 |
| `NewConsolidateCmd(inputs int, acct *string, addr *string)` | 109 |
| `NewCreateMultisigCmd(nRequired int, keys []string)` | 121 |
| `NewCreateNewAccountCmd(account string)` | 144 |
| `NewDumpPrivKeyCmd(address string)` | 157 |
| `NewFundRawTransactionCmd(hexString string, fundAccount string, options *FundRawTransactionOptions)` | 181 |
| `NewGetAccountCmd(address string)` | 196 |
| `NewGetAccountAddressCmd(account string)` | 209 |
| `NewGetAddressesByAccountCmd(account string)` | 222 |
| `NewGetBalanceCmd(account *string, minConf *int)` | 239 |
| `NewGetMasterPubkeyCmd(acct *string)` | 253 |
| `NewGetMultisigOutInfoCmd(hash string, index uint32)` | 266 |
| `NewGetNewAddressCmd(account *string, gapPolicy *string)` | 281 |
| `NewGetRawChangeAddressCmd(account *string)` | 298 |
| `NewGetReceivedByAccountCmd(account string, minConf *int)` | 315 |
| `NewGetReceivedByAddressCmd(address string, minConf *int)` | 333 |
| `NewGetStakeInfoCmd()` | 346 |
| `NewGetTicketsCmd(includeImmature bool)` | 359 |
| `NewGetTransactionCmd(txHash string, includeWatchOnly *bool)` | 374 |
| `NewGetUnconfirmedBalanceCmd(account *string)` | 391 |
| `NewGetVoteChoicesCmd(tickethash *string)` | 405 |
| `NewGetWalletFeeCmd()` | 416 |
| `NewImportPrivKeyCmd(privKey string, label *string, rescan *bool, scanFrom *int)` | 433 |
| `NewImportPubKeyCmd(pubKey string, label *string, rescan *bool, scanFrom *int)` | 455 |
| `NewImportScriptCmd(hex string, rescan *bool, scanFrom *int)` | 473 |
| `NewListAccountsCmd(minConf *int)` | 494 |
| `NewListLockUnspentCmd()` (leaves `Account` nil) | 507 |
| `NewListReceivedByAccountCmd(minConf *int, includeEmpty, includeWatchOnly *bool)` | 523 |
| `NewListReceivedByAddressCmd(minConf *int, includeEmpty, includeWatchOnly *bool)` | 543 |
| `NewListAddressTransactionsCmd(addresses []string, account *string)` | 563 |
| `NewListAllTransactionsCmd(account *string)` | 580 |
| `NewListSinceBlockCmd(blockHash *string, targetConfirms *int, includeWatchOnly *bool)` | 598 |
| `NewListTransactionsCmd(account *string, count, from *int, includeWatchOnly *bool)` | 619 |
| `NewListUnspentCmd(minConf, maxConf *int, addresses *[]string)` (leaves `Account` nil) | 641 |
| `NewLockUnspentCmd(unlock bool, transactions []dcrdtypes.TransactionInput)` | 657 |
| `NewPurchaseTicketCmd(fromAccount string, minConf *int, numTickets *int, expiry *int, comment *string)` (leaves `DontSignTx` nil) | 676 |
| `NewRedeemMultiSigOutCmd(hash string, index uint32, tree int8, address *string)` | 704 |
| `NewRedeemMultiSigOutsCmd(from string, to *string, number *int)` | 723 |
| `NewRenameAccountCmd(oldAccount, newAccount string)` | 740 |
| `NewSendFromCmd(fromAccount, toAddress string, amount float64, minConf *int, comment, commentTo *string)` | 767 |
| `NewSendManyCmd(fromAccount string, amounts map[string]float64, minConf *int, comment *string)` | 791 |
| `NewSendToAddressCmd(address string, amount float64, comment, commentTo *string)` | 813 |
| `NewSendToMultiSigCmd(fromaccount string, amount float64, pubkeys []string, nrequired *int, minConf *int, comment *string)` | 834 |
| `NewSendToTreasuryCmd(amount float64, comment, commentTo *string)` (both strings ignored) | 853 |
| `NewSendFromTreasuryCmd(pubkey string, amounts map[string]float64)` (`pubkey` is stored in `Key`) | 867 |
| `NewSetTreasuryPolicyCmd(key string, policy string, ticket *string)` | 901 |
| `NewSetTSpendPolicyCmd(hash string, policy string, ticket *string)` | 926 |
| `NewSetTxFeeCmd(amount float64)` | 941 |
| `NewSetVoteChoiceCmd(agendaID, choiceID string, tickethash *string)` | 956 |
| `NewSignMessageCmd(address, message string)` | 968 |
| `NewSignRawTransactionCmd(hexEncodedTx string, inputs *[]RawTxInput, privKeys *[]string, flags *string)` | 998 |
| `NewSignRawTransactionsCmd(hexEncodedTxs []string, send *bool)` | 1015 |
| `NewSweepAccountCmd(sourceAccount string, destinationAddress string, requiredConfs *uint32, feePerKb *float64)` | 1032 |
| `NewWalletInfoCmd()` | 1050 |
| `NewWalletIsLockedCmd()` | 1059 |
| `NewWalletLockCmd()` | 1068 |
| `NewWalletPassphraseCmd(passphrase string, timeout int64)` | 1080 |
| `NewWalletPassphraseChangeCmd(oldPassphrase, newPassphrase string)` | 1095 |

Go-level field types that differ from the simplified notation of 4.4: the
optional slice parameters are pointers to slices (`ListUnspentCmd.Addresses
*[]string`, `SignRawTransactionCmd.Inputs *[]RawTxInput`,
`SignRawTransactionCmd.PrivKeys *[]string`). This matters to help
generation (5.9, Quirk Q33) and to `MarshalCmd` (4.3.1), not to the wire.

### 3.4 Package `main`, `rpcserver.go`

| Item | Signature | Contract |
|---|---|---|
| `openRPCKeyPair` | `() (tls.Certificate, error)` (`:42`) | Load or generate the server key pair; section 5.1.2. |
| `generateRPCKeyPair` | `(writeKey bool) (tls.Certificate, error)` (`:70`) | Generate and write `rpc.cert` (and `rpc.key` if `writeKey`). |
| `randomX509SerialNumber` | `() *big.Int` (`:118`) | Uniform random in `[0, 2^128)`. |
| `endOfTime` | `time.Date(2049,12,31,23,59,59,0,UTC)` (`:125`) | Cap for certificate `NotAfter`. |
| `ClientCA` | `struct{ CertBlock []byte; Cert *x509.Certificate; PrivateKey any }` (`:127`) | Ephemeral client CA. |
| `generateAuthority` | `(pub, priv any) (*ClientCA, error)` (`:133`) | Self-signed client CA; section 5.1.4. |
| `marshalPrivateKey` | `(key any) ([]byte, error)` (`:180`) | PKCS#8 DER in a `PRIVATE KEY` PEM block. |
| `createSignedClientCert` | `(pub, caPriv any, ca *x509.Certificate) ([]byte, error)` (`:193`) | Client certificate signed by the ephemeral CA. |
| `generateClientKeyPair` | `(caPriv any, ca *x509.Certificate) (cert, key []byte, err error)` (`:217`) | New key on `cfg.TLSCurve` plus its certificate. |
| `rpcLoggers` | `struct{}` with `Subsystems() []string` (sorted keys of `subsystemLoggers`) and `SetLevels(string) error` (`parseAndSetDebugLevels`) (`:233-246`) | Implements `jsonrpc.Loggers`. |
| `startRPCServers` | `(ctx, walletLoader) (*grpc.Server, *jsonrpc.Server, error)` (`:248`) | Section 5.1. |
| `serviceName`, `interceptStreaming`, `interceptUnary` | `:399`, `:406`, `:432` | gRPC interceptors (logging and service-ready gate). Owned by the gRPC spec; listed for completeness. |
| `listenFunc` | `func(net string, laddr string) (net.Listener, error)` (`:450`) | `net.Listen` or a closure over `tls.Listen`. |
| `makeListeners` | `(normalizedListenAddrs []string, listen listenFunc) []net.Listener` (`:455`) | Section 5.1.5. |

### 3.5 `dcrjson/v4` surface the server depends on

| Item | Source | Contract as used |
|---|---|---|
| `Request{Jsonrpc string; Method string; Params []json.RawMessage; ID interface{}}` | `dcrd/dcrjson/v4@v4.2.0/jsonrpc.go:70-75` | Decode target for every request. |
| `Response{Jsonrpc string; Result json.RawMessage; Error *RPCError; ID *interface{}}` | `jsonrpc.go:118-123` | Encode shape of every response; no `omitempty` anywhere. |
| `RPCError{Code RPCErrorCode \`json:"code,omitempty"\`; Message string \`json:"message,omitempty"\`}` | `jsonrpc.go:21-24` | Both members are omitted when zero/empty. `Error()` formats `"%d: %s"`. |
| `MarshalResponse(rpcVersion, id, result, rpcErr)` | `jsonrpc.go:152-166` | Section 4.2. |
| `NewResponse`, `IsValidIDType` | `jsonrpc.go:130-147`, `51-62` | ID type gate. |
| `ParseParams(method, params)` | `cmdparse.go:111-163` | Section 5.5.2. |
| `MustRegister/Register`, `UsageFlag`, `UFWebsocketOnly` (value `1<<1`), `UFNotification` (`1<<2`) | `register.go` | Registration rules, section 4.3. |
| `MethodUsageText`, `GenerateHelp` | `cmdinfo.go:225`, `help.go:508` | Build-time only (generator and tests). |
| `NewCmd`, `MarshalCmd` | `cmdparse.go:513`, `40` | Client side (dcrctl); exercised by upstream tests, not by the server. |
| Error code constants and `ErrRPC*` variables | `jsonrpcerr.go` | Section 8. |
| `jsonerr.go` | value-typed `RPCError` variables (`ErrInvalidRequest` …) | One use in dcrwallet (`methods.go:3857`), with a surprising result: section 8.1 and Quirk Q27. |
| `parse.go` `EncodeConcatenatedHashes`/`DecodeConcatenatedHashes`, `helpers.go` pointer helpers (`Bool`, `Int`, `Uint`, `Uint16`, `Int32`, `Uint32`, `Int64`, `Uint64`, `Float64`, `String`) | | Not used by the server paths in scope. The one use in the package is `dcrjson.Float64` in a per-method handler (`internal/rpc/jsonrpc/methods.go:1697`). |
| `NewRequest(rpcVersion, id, method, params []interface{}) (*Request, error)` | `jsonrpc.go:85-112` | Client side. Version other than `"1.0"`/`"2.0"` becomes `"1.0"`; invalid id type gives `ErrInvalidType` `the id of type '<T>' is invalid`; each parameter is `json.Marshal`led into a `RawMessage`; with zero parameters `Params` is an empty non-nil slice, so the request carries `"params":[]`. |
| `NewRPCError(code, message) *RPCError` | `jsonrpc.go:37-42` | Plain constructor. Not used by dcrwallet. |
| `RPCErrorCode` | `jsonrpc.go:17` | `int`. |
| `ErrorKind`, `Error{Err error; Description string}`, `makeError` | `error.go:12-91` | Error type of every dcrjson failure. `Error.Error()` returns `Description` only; `Unwrap()` returns the kind, so `errors.Is(err, dcrjson.ErrNumParams)` works. Kinds (each a string equal to its own name): `ErrDuplicateMethod`, `ErrInvalidUsageFlags`, `ErrInvalidType`, `ErrEmbeddedType`, `ErrUnexportedField`, `ErrUnsupportedFieldType`, `ErrNonOptionalField`, `ErrNonOptionalDefault`, `ErrMismatchedDefault`, `ErrUnregisteredMethod`, `ErrMissingDescription`, `ErrNumParams`. |
| `CmdMethod(cmd) (string, error)` | `cmdinfo.go:17-29` | Reverse lookup from a registered command pointer type to its method string; `ErrUnregisteredMethod` `<T> is not registered` otherwise. Not used by the server. |
| `MethodUsageFlags(method) (UsageFlag, error)` | `cmdinfo.go:34-45` | Registered flags; `ErrUnregisteredMethod` `"<method>" is not registered`. Not used by the server (Quirk Q16). |
| `RegisteredMethods(methodType) []string` | `register.go:298-313` | Sorted method strings registered under the Go type of the argument (e.g. `types.Method("")`). Not used by the server. |
| `UsageFlag.String()` | `register.go:48-73` | Clears bit 0, then `0x0` for zero, otherwise the set flag names joined with `\|` followed by `\|0x<hex>` for unknown bits. Appears only in the `ErrInvalidUsageFlags` text. |
| `methodInfo{maxParams, numReqParams, numOptParams int; defaults map[int]reflect.Value; flags UsageFlag; usage string}` | `register.go:77-84` | Per-method registry record; `usage` caches the one-line usage after the first `MethodUsageText` call. |
| `methodToConcreteType`, `methodToInfo`, `concreteTypeToMethod`, `registerLock` | `register.go:86-91` | Process-global registry maps keyed by the typed method value (so `types.Method("help")` and dcrd's `Method("help")` are distinct keys) and by command pointer type, guarded by one RWMutex. |
| `EncodeConcatenatedHashes(hashSlice []chainhash.Hash) string` | `parse.go:17-25` | Lower-case hex of the hashes' 32 raw bytes each, concatenated in slice order, **not** byte-reversed; empty slice gives `""`. Not referenced anywhere in dcrwallet's non-test code. |
| `DecodeConcatenatedHashes(hashes string) ([]chainhash.Hash, error)` | `parse.go:38-58` | Inverse. Length not a multiple of 64 characters: `*RPCError{-8, "Hashes is not evenly divisible by the hash size"}`. A 64-character group that is not hex: `*RPCError{-22, "Parameter contains invalid hexadecimal encoding: " + <the 64 characters>}`. Empty string gives an empty non-nil slice. Not referenced by dcrwallet. |
| unexported helpers, `register.go` | `baseKindString` (`:96`), `isAcceptableKind` (`:109`), `usageFlagStrings` map (`:42-45`: `UFWebsocketOnly` → `"UFWebsocketOnly"`, `UFNotification` → `"UFNotification"`), the unnamed retired flag `1<<0` ("was UFWalletOnly") and `highestUsageFlagBit` (`1<<3`, one past the last real flag) (`:22-39`) | Implement registration checks 4-9 of 4.3.1 and the `<type> (<stars><kind>)` rendering used in checks 5, 6 and 9 (one `*` per pointer level, then the base kind name). |
| unexported helpers, `cmdparse.go` | `makeParams` (`:18-34`), `checkNumParams` (`:73-89`), `populateDefaults` (`:97-107`), `isNumeric` (`:167`), `typesMaybeCompatible` (`:182`), `baseType` (`:224`), `assignField` (`:238-485`) | Covered by 4.3 and 4.3.1 (`MarshalCmd` truncation, arity check, default application, `NewCmd` conversions). `baseType` strips every pointer level and reports how many were stripped. |
| unexported helpers, `cmdinfo.go` | `subStructUsage` (`:55`), `subArrayUsage` (`:102`), `fieldUsage` (`:132`), `methodUsageText` (`:181`) | One-line usage rules of 5.9. |
| unexported helpers, `help.go` | `baseHelpDescs` (`:19`), `reflectTypeToJSONType` (`:50`), `resultStructHelp` (`:83`), `reflectTypeToJSONExample` (`:144`), `resultTypeHelp` (`:253`), `argTypeHelp` (`:286`), `argHelp` (`:329`), `methodHelp` (`:397`), `isValidResultType` (`:446`) | Help generation rules of 5.9.1. |


---

## 4. Data formats

Nothing in this subsystem is persisted to `wallet.db`. The persisted
artefacts are the TLS files (4.13). Everything else is wire format.

### 4.1 Request envelope

A request is one JSON value decoded with Go `encoding/json` into
`dcrjson.Request` (`dcrd/dcrjson/v4@v4.2.0/jsonrpc.go:70-75`):

| JSON key | Go type | Notes |
|---|---|---|
| `jsonrpc` | `string` | Echoed in the response when exactly `"1.0"` or `"2.0"`; any other string (including absent/empty) is answered as `"1.0"`. A non-string value is a decode error. |
| `method` | `string` | Looked up case-sensitively. Non-string is a decode error. |
| `params` | `[]json.RawMessage` | JSON array of positional parameters, `null`, or absent (both give zero parameters). A JSON object (named parameters) is a decode error. Each element is kept as raw bytes. |
| `id` | `interface{}` | Any JSON value. Numbers decode to `float64`. |

Decode rules inherited from Go that a port must reproduce:

1. Object keys are matched to fields case-insensitively (`"Method"`,
   `"METHOD"` and `"method"` are the same key); when a key repeats, the
   last occurrence wins. Unknown keys are ignored.
2. Leading/trailing JSON whitespace is allowed; any other trailing data is
   a syntax error. An empty body is a syntax error.
3. A *syntax* error leaves the request struct zeroed, and so does a
   top-level value that is not an object (a batch array, a string, a
   number). A *member type* error (wrong JSON type for `jsonrpc`, `method`
   or `params`) does not stop decoding: the remaining well-typed members are still stored and the
   first such error is returned afterwards. The invalid-request reply
   therefore echoes `id` and `jsonrpc` whenever those members were
   well-typed in an otherwise mistyped object, and uses `null` / `"1.0"`
   otherwise (see 5.3 step 5 and 5.4.3 step 2). Example:
   `{"jsonrpc":"2.0","id":7,"method":"getinfo","params":{}}` is answered
   `{"jsonrpc":"2.0","result":null,"error":{"code":-32600,"message":"Invalid request"},"id":7}`.
4. There is no notion of a notification: a request with `"id":null` or no
   `id` is executed and answered with `"id":null`
   (`server.go:557-610`; contrast the dcrjson package comment at
   `dcrd/dcrjson/v4@v4.2.0/doc.go:37-40`).
5. Batches (a top-level JSON array) are not supported on either transport;
   they hit the type-error path and receive a single `-32600` reply.

### 4.2 Response envelope

Every reply produced through `dcrjson.MarshalResponse`
(`jsonrpc.go:152-166`) is a single JSON object with exactly these members
in this order and no insignificant whitespace:

```
{"jsonrpc":<version>,"result":<result>,"error":<error>,"id":<id>}
```

| Member | Encoding |
|---|---|
| `jsonrpc` | `"2.0"` if the request said `"2.0"`, `"1.0"` if it said `"1.0"`, otherwise `"1.0"`. |
| `result` | `json.Marshal(result)`; always present. `null` when the handler returned a nil interface or when there is an error. |
| `error` | `null` on success, otherwise `{"code":<int>,"message":<string>}`. Because of `omitempty` on both members, code `0` is omitted and an empty message is omitted (`{}` in the degenerate case). |
| `id` | The request id re-encoded: string as string, number via Go `float64` formatting (integers print without a fraction; magnitudes at or above 1e21 or below 1e-6 print in exponent form; integers above 2^53 lose precision), `null` for null/absent. |

`NewResponse` rejects ids that are not string, number or null
(`jsonrpc.go:135-138`, `IsValidIDType` at `jsonrpc.go:51-62`): a boolean,
array or object id makes `MarshalResponse` fail with the Go error
`the id of type '<T>' is invalid` (`<T>` is `bool`, `[]interface {}` or
`map[string]interface {}`). The handler has already run when this is
detected. See 5.3 step 9 and 5.4.3 step 6 for what the client then
observes.

Three websocket code paths bypass `MarshalResponse` and marshal a
`dcrjson.Response` built by `makeResponse` (`methods.go:284-306`) directly.
Those replies have **`"jsonrpc":""`** regardless of the request, and they do
not apply the id type check:

| Path | Exact bytes (id shown as `1`) |
|---|---|
| successful `authenticate` | `{"jsonrpc":"","result":null,"error":null,"id":1}` |
| `stop` over websocket | `{"jsonrpc":"","result":"dcrwallet stopping.","error":null,"id":1}` |
| undecodable message from an authenticated client | `{"jsonrpc":"","result":null,"error":{"code":-32600,"message":"Invalid request"},"id":null}` (id is whatever was decoded, usually `null`) |

`makeResponse` additionally has an unreachable branch that would answer
`{"code":-32603,"message":"Unexpected error marshalling result"}` if the
result could not be marshalled (`methods.go:292-301`); none of its three
callers passes such a value.

JSON text encoding is Go `encoding/json` with HTML escaping on: inside
strings `<`, `>`, `&` are written as the six-character escapes `\u003c`,
`\u003e`, `\u0026`; U+2028 and U+2029 as `\u2028`, `\u2029`; each invalid
UTF-8 byte as `\ufffd`; map keys are
emitted in sorted byte order; struct members in declaration order; floats
in Go's shortest round-trip form. This is visible to clients (for example
the `help debuglevel` text contains `<subsystem>=<level>`, which is sent as
`\u003csubsystem\u003e=\u003clevel\u003e`).
Results that are already `json.RawMessage` (dcrd passthrough) are compacted
and HTML-escaped the same way when embedded.

### 4.3 Command registration and positional parameter decoding

Registration rules enforced by `dcrjson.Register`
(`dcrd/dcrjson/v4@v4.2.0/register.go:162-284`): the command is a pointer to
a struct; fields are exported, not embedded, and not chan/func/complex/
interface/pointer-to-pointer; a pointer field is an optional parameter; all
fields after the first optional one must be optional; a
`jsonrpcdefault:"<json>"` tag is allowed only on optional fields and must
unmarshal into the pointee type. For each method dcrjson records
`maxParams` (field count), `numReqParams`, `numOptParams`, the default
values by field index, and the usage flags.

Wire consequences:

- Parameters are positional. Parameter *i* is decoded into struct field
  *i*. The Go field name, lower-cased, is the parameter's display name in
  usage and help; **`json:"..."` tags on top-level command fields have no
  effect on the wire** (they would only matter if the struct were marshalled
  as an object, which never happens). JSON tags do matter inside nested
  object parameters (4.5).
- Parameter count must satisfy `numReqParams <= n <= maxParams`
  (`cmdparse.go:73-89`).
- Each supplied parameter is decoded with `json.Unmarshal` into the field
  (`cmdparse.go:132-153`). After the supplied parameters, each remaining
  optional field that has a registered default is set to a fresh copy of the
  default (`cmdparse.go:97-107`); optional fields without a default stay
  nil.
- **An explicit JSON `null` is accepted for every parameter.** For an
  optional parameter it leaves the pointer nil *and suppresses the default*
  (defaults are applied only to positions beyond the supplied count). For a
  required parameter it silently leaves the Go zero value (`""`, `0`,
  `false`, nil slice/map). This follows from how `encoding/json` treats
  `null` when unmarshalling through an interface holding a pointer
  (`cmdparse.go:135-136`).

Go type to accepted JSON, as decoded by `encoding/json`:

| Go field type | Accepted JSON | Rejected (decode error) |
|---|---|---|
| `string` | string, `null` | number, bool, array, object |
| `bool` | `true`, `false`, `null` | `0`/`1`, `"true"` |
| `int` (64-bit), `int64`, `int32`, `int8` | integer literal within range, `null` | fraction or exponent (`1.0`, `1e3`), out-of-range, string |
| `uint32` | non-negative integer literal within range, `null` | negative, fraction/exponent, string |
| `float64` | any JSON number within float64 range, `null` | string |
| `[]string` | array of strings (elements may be `null`, giving `""`), `null` (nil slice) | other |
| `[]T` struct elements | array of objects, `null` | other |
| `map[string]float64` | object with number values, `null` (nil map) | other |
| struct (`FundRawTransactionOptions`) | object, `null` | other |
| `*T` | as `T`; `null` gives nil | as `T` |

Any failure at this stage (unregistered method, wrong count, undecodable
parameter) is reported to the client as `-32600 Invalid request`; the
descriptive dcrjson error text (for example `wrong number of params
(expected between 1 and 2, received 3)` or `parameter #2 'minconf' must be
type int (got string)`) is discarded by the server
(`methods.go:263-266`). Those texts are still observable through `dcrctl`,
which builds commands client-side with `dcrjson.NewCmd`, so a Rust
`dcrctl`-equivalent needs them but the wallet daemon does not.

#### 4.3.1 dcrjson error texts and client-side command handling

None of the texts below reaches a JSON-RPC client of the wallet daemon
(server-side they are discarded or are init-time panics). They are part of
the contract of a Rust `dcrjson`/`dcrctl` equivalent and of the ported type
tests. `<q>` denotes Go `%q` quoting.

`Register` checks, in this order (`dcrd/dcrjson/v4@v4.2.0/register.go:162-284`):

| # | Condition | Kind | Description |
|---|---|---|---|
| 1 | method value is not of string kind | `ErrInvalidType` | `method <q> is not a string type` |
| 2 | method already registered | `ErrDuplicateMethod` | `method <q> is already registered for type <T>` (`<T>` is the Go type of the method value itself, format `%[1]T`, e.g. `types.Method`; not the command type) |
| 3 | command pointer type already registered | `ErrDuplicateMethod` | `param type <T> is already registered for method <q>` |
| 4 | flag bits outside `UFWebsocketOnly\|UFNotification` (bit 0 is tolerated) | `ErrInvalidUsageFlags` | `invalid usage flags specified for method <method>: <flags>` |
| 5 | not a pointer | `ErrInvalidType` | `type must be *struct not '<type> (<kind>)'` |
| 6 | pointer to non-struct | `ErrInvalidType` | `type must be *struct not '<type> (*<kind>)'` |
| 7 | embedded field | `ErrEmbeddedType` | `embedded fields are not supported (field name: <q>)` |
| 8 | unexported field | `ErrUnexportedField` | `unexported fields are not supported (field name: <q>)` |
| 9 | field (after one pointer dereference) of kind chan, complex64/128, func, pointer or interface | `ErrUnsupportedFieldType` | `unsupported field type '<type> (<stars><kind>)' (field name <q>)` |
| 10 | required field after an optional one | `ErrNonOptionalField` | `all fields after the first optional field must also be optional (field name <q>)` |
| 11 | `jsonrpcdefault` on a required field | `ErrNonOptionalDefault` | `required fields must not have a default specified (field name <q>)` |
| 12 | `jsonrpcdefault` does not `json.Unmarshal` into the pointee type | `ErrMismatchedDefault` | `default value of <q> is the wrong type (field name <q>)` |

Checks 7-12 run per field in declaration order. An empty `jsonrpcdefault`
tag is the same as no tag.

`ParseParams` errors (`cmdparse.go:111-163`):

| Condition | Kind | Description |
|---|---|---|
| method not registered under the given typed key | `ErrUnregisteredMethod` | `<q> is not registered` |
| count outside range, required == max | `ErrNumParams` | `wrong number of params (expected <n>, received <m>)` |
| count outside range otherwise | `ErrNumParams` | `wrong number of params (expected between <min> and <max>, received <m>)` |
| parameter has the wrong JSON type (`*json.UnmarshalTypeError`) | `ErrInvalidType` | `parameter #<i> '<lowerfield>' must be type <GoType> (got <jsonkind>)`, `<i>` 1-based, `<jsonkind>` the `encoding/json` description (`string`, `number`, `bool`, `array`, `object`, or `number <literal>` for an out-of-range or fractional number) |
| any other unmarshal error (syntax) | `ErrInvalidType` | `parameter #<i> '<lowerfield>' failed to unmarshal: <err>` |

The count check precedes every parameter decode; parameters are decoded
left to right and the first failure wins.

`MarshalCmd(rpcVersion, id, cmd)` (`cmdparse.go:40-69`), used by `dcrctl`
and by the upstream type tests:

1. Command type not registered: `ErrUnregisteredMethod` `<T> is not
   registered`. Nil command pointer: `ErrInvalidType` `the specified
   command is nil`.
2. Parameters are the struct fields in order, **stopping at the first nil
   pointer field**: a later non-nil optional field is silently dropped
   (`makeParams`, `cmdparse.go:18-34`). Non-nil pointer fields are
   marshalled through the pointer (so the pointee value is emitted).
3. The request is `NewRequest(rpcVersion, id, method, params)` marshalled
   with `encoding/json`: member order `jsonrpc`, `method`, `params`, `id`;
   `params` is `[]` when there are none. Example:
   `{"jsonrpc":"1.0","method":"getbalance","params":[],"id":1}`.

`NewCmd(method, args...)` (`cmdparse.go:513-552`), used by `dcrctl` to turn
command-line words into a command:

1. Unregistered method: `ErrUnregisteredMethod` `<q> is not registered`.
   Argument count is checked with the same `ErrNumParams` texts as
   `ParseParams`.
2. Each argument is assigned to field *i* by `assignField`
   (`cmdparse.go:238-485`). **`NewCmd` does not apply `jsonrpcdefault`
   values**; omitted optional fields stay nil.
3. `assignField` rules, with both types reduced to their base type by
   stripping pointers (`<dst>`/`<src>` are the base types; `<i>` is
   1-based):
   - Compatibility gate (`typesMaybeCompatible`, `cmdparse.go:182-220`):
     identical base types; numeric to numeric; or a string source into a
     numeric, bool, string, array, slice, struct or map destination.
     Otherwise `ErrInvalidType` `parameter #<i> '<field>' must be type
     <dst> (got <src>)`.
   - Same base type: assigned directly, allocating or dereferencing
     pointers as needed.
   - Signed/unsigned integer source into an integer destination:
     range-checked; failure is `parameter #<i> '<field>' overflows
     destination type <dst>` (a negative value into an unsigned
     destination, or an unsigned value above 2^63-1 into a signed
     destination, is an overflow). Integer source into a float or other
     non-integer destination: `... must be type <dst> (got <src>)`.
   - Float source: destination must be float32/float64 (else `must be
     type`); overflow-checked.
   - String source: bool via `strconv.ParseBool` (accepts `1`, `t`, `T`,
     `TRUE`, `true`, `True`, `0`, `f`, `F`, `FALSE`, `false`, `False`);
     signed integers via `strconv.ParseInt(s, 0, 0)` and unsigned via
     `strconv.ParseUint(s, 0, 0)` (base prefixes `0x`, `0o`, `0b`, leading
     `0` octal, and `_` separators are accepted), then range-checked
     against the destination width; floats via `strconv.ParseFloat(s,
     64)`; parse failure is `parameter #<i> '<field>' must parse to a
     <dst>`, range failure the `overflows destination type` text. A string
     into an array, slice, struct or map destination is `json.Unmarshal`led;
     failure is `parameter #<i> '<field>' must be valid JSON which
     unmarshals to a <dst>`.

`MethodUsageText` / `GenerateHelp` on an unregistered method return
`ErrUnregisteredMethod` with description `<q> is not registered`
(`cmdinfo.go:232-235`) and `<Go-syntax value> is not registered` (`%#v`,
`help.go:515-518`) respectively.

### 4.4 Command structs

Notation: `name: Type` is required; `name?: Type` is optional (Go pointer);
`= v` is the `jsonrpcdefault`. Names are the lower-cased Go field names, as
shown in usage/help. All citations are `rpc/jsonrpc/types/methods.go`
unless marked `dcrd` (`dcrd/rpc/jsonrpc/types/v4@v4.4.0/chainsvrcmds.go`
or `chainsvrwscmds.go`).

Wallet commands (85):

| Method | Struct (line) | Positional parameters |
|---|---|---|
| `abandontransaction` | `AbandonTransactionCmd` (21) | `hash: string` |
| `accountaddressindex` | `AccountAddressIndexCmd` (28) | `account: string`, `branch: int` |
| `accountsyncaddressindex` | `AccountSyncAddressIndexCmd` (44) | `account: string`, `branch: int`, `index: int` |
| `accountunlocked` | `AccountUnlockedCmd` (1156) | `account: string` |
| `addmultisigaddress` | `AddMultisigAddressCmd` (61) | `nrequired: int`, `keys: []string`, `account?: string` |
| `addtransaction` | `AddTransactionCmd` (85) | `blockhash: string`, `transaction: string` |
| `auditreuse` | `AuditReuseCmd` (95) | `since?: int32` |
| `consolidate` | `ConsolidateCmd` (102) | `inputs: int`, `account?: string`, `address?: string` |
| `createmultisig` | `CreateMultisigCmd` (114) | `nrequired: int`, `keys: []string` |
| `createnewaccount` | `CreateNewAccountCmd` (138) | `account: string` |
| `createsignature` | `CreateSignatureCmd` (129) | `address: string`, `inputindex: int`, `hashtype: int`, `previouspkscript: string`, `serializedtransaction: string` |
| `disapprovepercent` | `DisapprovePercentCmd` (876) | none |
| `discoverusage` | `DiscoverUsageCmd` (1111) | `startblock?: string`, `discoveraccounts?: bool`, `gaplimit?: uint32` |
| `dumpprivkey` | `DumpPrivKeyCmd` (151) | `address: string` |
| `fundrawtransaction` | `FundRawTransactionCmd` (173) | `hexstring: string`, `fundaccount: string`, `options?: FundRawTransactionOptions` |
| `getaccount` | `GetAccountCmd` (190) | `address: string` |
| `getaccountaddress` | `GetAccountAddressCmd` (203) | `account: string` |
| `getaddressesbyaccount` | `GetAddressesByAccountCmd` (216) | `account: string` |
| `getbalance` | `GetBalanceCmd` (229) | `account?: string`, `minconf?: int = 1` |
| `getcoinjoinsbyacct` | `GetCoinjoinsByAcctCmd` (1166) | none |
| `getmasterpubkey` | `GetMasterPubkeyCmd` (248) | `account?: string` |
| `getmultisigoutinfo` | `GetMultisigOutInfoCmd` (260) | `hash: string`, `index: uint32` |
| `getnewaddress` | `GetNewAddressCmd` (271) | `account?: string`, `gappolicy?: string` |
| `getrawchangeaddress` | `GetRawChangeAddressCmd` (289) | `account?: string` |
| `getreceivedbyaccount` | `GetReceivedByAccountCmd` (305) | `account: string`, `minconf?: int = 1` |
| `getreceivedbyaddress` | `GetReceivedByAddressCmd` (323) | `address: string`, `minconf?: int = 1` |
| `getstakeinfo` | `GetStakeInfoCmd` (342) | none |
| `gettickets` | `GetTicketsCmd` (353) | `includeimmature: bool` (required) |
| `gettransaction` | `GetTransactionCmd` (364) | `txid: string`, `includewatchonly?: bool = false` |
| `getunconfirmedbalance` | `GetUnconfirmedBalanceCmd` (382) | `account?: string` |
| `getvotechoices` | `GetVoteChoicesCmd` (399) | `tickethash?: string` |
| `getwalletfee` | `GetWalletFeeCmd` (412) | none |
| `importcfiltersv2` | `ImportCFiltersV2Cmd` (1121) | `startheight: int32`, `filters: []string` |
| `importprivkey` | `ImportPrivKeyCmd` (421) | `privkey: string`, `label?: string`, `rescan?: bool = true`, `scanfrom?: int` |
| `importpubkey` | `ImportPubKeyCmd` (443) | `pubkey: string`, `label?: string`, `rescan?: bool = true`, `scanfrom?: int` |
| `importscript` | `ImportScriptCmd` (466) | `hex: string`, `rescan?: bool = true`, `scanfrom?: int` |
| `importxpub` | `ImportXpubCmd` (479) | `name: string`, `xpub: string` |
| `listaccounts` | `ListAccountsCmd` (485) | `minconf?: int = 1` |
| `listaddresstransactions` | `ListAddressTransactionsCmd` (553) | `addresses: []string`, `account?: string` |
| `listalltransactions` | `ListAllTransactionsCmd` (571) | `account?: string` |
| `listlockunspent` | `ListLockUnspentCmd` (501) | `account?: string` |
| `listreceivedbyaccount` | `ListReceivedByAccountCmd` (512) | `minconf?: int = 1`, `includeempty?: bool = false`, `includewatchonly?: bool = false` |
| `listreceivedbyaddress` | `ListReceivedByAddressCmd` (532) | `minconf?: int = 1`, `includeempty?: bool = false`, `includewatchonly?: bool = false` |
| `listsinceblock` | `ListSinceBlockCmd` (587) | `blockhash?: string`, `targetconfirmations?: int = 1`, `includewatchonly?: bool = false` |
| `listtransactions` | `ListTransactionsCmd` (607) | `account?: string`, `count?: int = 10`, `from?: int = 0`, `includewatchonly?: bool = false` |
| `listunspent` | `ListUnspentCmd` (629) | `minconf?: int = 1`, `maxconf?: int = 9999999`, `addresses?: []string`, `account?: string` |
| `lockaccount` | `LockAccountCmd` (1151) | `account: string` |
| `lockunspent` | `LockUnspentCmd` (650) | `unlock: bool`, `transactions: []TransactionInput` |
| `mixaccount` | `MixAccountCmd` (1103) | none |
| `mixoutput` | `MixOutputCmd` (1106) | `outpoint: string` |
| `processunmanagedticket` | `ProcessUnmanagedTicketCmd` (1161) | `tickethash: string` |
| `purchaseticket` | `PurchaseTicketCmd` (666) | `fromaccount: string`, `minconf?: int = 1`, `numtickets?: int = 1`, `expiry?: int`, `comment?: string`, `dontsigntx?: bool` |
| `redeemmultisigout` | `RedeemMultiSigOutCmd` (696) | `hash: string`, `index: uint32`, `tree: int8`, `address?: string` |
| `redeemmultisigouts` | `RedeemMultiSigOutsCmd` (716) | `fromscraddress: string`, `toaddress?: string`, `number?: int` |
| `renameaccount` | `RenameAccountCmd` (733) | `oldaccount: string`, `newaccount: string` |
| `rescanwallet` | `RescanWalletCmd` (748) | `beginheight?: int = 0` |
| `sendfrom` | `SendFromCmd` (753) | `fromaccount: string`, `toaddress: string`, `amount: float64`, `minconf?: int = 1`, `comment?: string`, `commentto?: string` |
| `sendfromtreasury` | `SendFromTreasuryCmd` (860) | `key: string`, `amounts: map[string]float64` |
| `sendmany` | `SendManyCmd` (779) | `fromaccount: string`, `amounts: map[string]float64` (usage tag `{"address":amount,...}`), `minconf?: int = 1`, `comment?: string` |
| `sendtoaddress` | `SendToAddressCmd` (801) | `address: string`, `amount: float64`, `comment?: string`, `commentto?: string` |
| `sendtomultisig` | `SendToMultiSigCmd` (824) | `fromaccount: string`, `amount: float64`, `pubkeys: []string`, `nrequired?: int = 1`, `minconf?: int = 1`, `comment?: string` |
| `sendtotreasury` | `SendToTreasuryCmd` (847) | `amount: float64` |
| `setaccountpassphrase` | `SetAccountPassphraseCmd` (1139) | `account: string`, `passphrase: string` |
| `setdisapprovepercent` | `SetDisapprovePercentCmd` (880) | `percent: uint32` |
| `settreasurypolicy` | `SetTreasuryPolicyCmd` (893) | `key: string`, `policy: string`, `ticket?: string` |
| `settspendpolicy` | `SetTSpendPolicyCmd` (918) | `hash: string`, `policy: string`, `ticket?: string` |
| `settxfee` | `SetTxFeeCmd` (935) | `amount: float64` |
| `setvotechoice` | `SetVoteChoiceCmd` (948) | `agendaid: string`, `choiceid: string`, `tickethash?: string` |
| `signmessage` | `SignMessageCmd` (961) | `address: string`, `message: string` |
| `signrawtransaction` | `SignRawTransactionCmd` (986) | `rawtx: string`, `inputs?: []RawTxInput`, `privkeys?: []string`, `flags?: string = "ALL"` |
| `signrawtransactions` | `SignRawTransactionsCmd` (1008) | `rawtxs: []string`, `send?: bool = true` |
| `spendoutputs` | `SpendOutputsCmd` (1169) | `account: string`, `previousoutpoints: []string`, `outputs: []AddressAmountPair` |
| `sweepaccount` | `SweepAccountCmd` (1024) | `sourceaccount: string`, `destinationaddress: string`, `requiredconfirmations?: uint32`, `feeperkb?: float64` |
| `syncstatus` | `SyncStatusCmd` (1042) | none |
| `ticketinfo` | `TicketInfoCmd` (1127) | `startheight?: int32 = 0` |
| `treasurypolicy` | `TreasuryPolicyCmd` (886) | `key?: string`, `ticket?: string` |
| `tspendpolicy` | `TSpendPolicyCmd` (911) | `hash?: string`, `ticket?: string` |
| `unlockaccount` | `UnlockAccountCmd` (1145) | `account: string`, `passphrase: string` |
| `validatepredcp0005cf` | `ValidatePreDCP0005CFCmd` (1118) | none |
| `walletinfo` | `WalletInfoCmd` (1045) | none |
| `walletislocked` | `WalletIsLockedCmd` (1055) | none |
| `walletlock` | `WalletLockCmd` (1064) | none |
| `walletpassphrase` | `WalletPassphraseCmd` (1073) | `passphrase: string`, `timeout: int64` |
| `walletpassphrasechange` | `WalletPassphraseChangeCmd` (1088) | `oldpassphrase: string`, `newpassphrase: string` |
| `walletpubpassphrasechange` | `WalletPubPassphraseChangeCmd` (1132) | `oldpassphrase: string`, `newpassphrase: string` |

dcrd command shapes re-registered by the wallet as newtypes
(`methods.go:1316-1336`, registered at `1281-1312`):

| Method | Wallet newtype of dcrd struct (dcrd line) | Positional parameters |
|---|---|---|
| `authenticate` | `AuthenticateCmd` (`chainsvrwscmds.go:14`) | `username: string`, `passphrase: string`; flag `UFWebsocketOnly` |
| `createrawtransaction` | `CreateRawTransactionCmd` (124) | `inputs: []TransactionInput`, `amounts: map[string]float64` (usage tag `{"address":amount,...}`), `locktime?: int64`, `expiry?: int64` |
| `debuglevel` | `DebugLevelCmd` (148) | `levelspec: string` |
| `getbestblock` | `GetBestBlockCmd` (339) | none |
| `getbestblockhash` | `GetBestBlockHashCmd` (348) | none |
| `getblockcount` | `GetBlockCountCmd` (386) | none |
| `getblockhash` | `GetBlockHashCmd` (395) | `index: int64` |
| `getblockheader` | `GetBlockHeaderCmd` (408) | `hash: string`, `verbose?: bool = true` |
| `getblock` | `GetBlockCmd` (357) | `hash: string`, `verbose?: bool = true`, `verbosetx?: bool = false` |
| `getcfilterv2` | `GetCFilterV2Cmd` (438) | `blockhash: string` |
| `getcurrentnet` | `GetCurrentNetCmd` (478) | none |
| `getinfo` | `GetInfoCmd` (514) | none |
| `getpeerinfo` | `GetPeerInfoCmd` (614) | none |
| `gettxout` | `GetTxOutCmd` (738) | `txid: string`, `vout: uint32`, `tree: int8`, `includemempool?: bool = true` |
| `help` | `HelpCmd` (843) | `command?: string` |
| `sendrawtransaction` | `SendRawTransactionCmd` (939) | `hextx: string`, `allowhighfees?: bool = false` |
| `validateaddress` | `ValidateAddressCmd` (1091) | `address: string` |
| `verifymessage` | `VerifyMessageCmd` (1122) | `address: string`, `signature: string`, `message: string` |
| `version` | `VersionCmd` (1139) | none |

Top-level `json` tags that exist on command fields but have no wire effect
(listed so nobody mistakes them for named parameters): `AbandonTransactionCmd.Hash`,
`AccountAddressIndexCmd.*`, `AccountSyncAddressIndexCmd.*`,
`AddTransactionCmd.*`, `AuditReuseCmd.Since`, `ConsolidateCmd.Inputs`,
`ImportXpubCmd.*`, `MixOutputCmd.Outpoint`, `DiscoverUsageCmd.*`,
`ImportCFiltersV2Cmd.*`, `TicketInfoCmd.StartHeight`.

### 4.5 Nested parameter object types

These are decoded from JSON objects, so their `json` tags are the wire
keys. Go matches object keys to these names case-insensitively and ignores
unknown keys; a faithful port must accept, for example, `scriptpubkey` for
`scriptPubKey`.

| Type | Source | Members in order (JSON key: Go type) |
|---|---|---|
| `FundRawTransactionOptions` | `methods.go:165-169` | `changeaddress: *string`, `feerate: *float64`, `conf_target: *int32` |
| `RawTxInput` | `methods.go:977-983` | `txid: string`, `vout: uint32`, `tree: int8`, `scriptPubKey: string`, `redeemScript: string` |
| `AddressAmountPair` | `methods.go:1177-1180` | `address: string`, `amount: float64` |
| `TransactionInput` (dcrd) | `dcrd/rpc/jsonrpc/types/v4@v4.4.0/chainsvrcmds.go:116-121` | `amount: float64` (omitempty), `txid: string`, `vout: uint32`, `tree: int8` |

`TransactionInput` is also a result element (`listlockunspent`), where
`amount` is omitted when zero.

### 4.6 Result structs (`rpc/jsonrpc/types/results.go`)

Members are listed in declaration order, which is the JSON output order.
`(oe)` marks `omitempty`: the member is omitted when it is the Go zero
value (`0`, `0.0`, `""`, `false`, nil pointer, nil or empty slice). Members
without `(oe)` are always present; a nil slice without `(oe)` is emitted as
`null`, an empty non-nil slice as `[]`. Whether a handler produces nil or
empty is method semantics and is fixed in the per-method specs; this spec
fixes only names, order, types and tags. Numeric widths matter for range
and formatting: all `float64` members use Go float formatting, integer
members print as plain decimal.

| Struct (line) | Members |
|---|---|
| `FundRawTransactionResult` (9) | `hex: string`, `fee: float64` |
| `GetAccountBalanceResult` (15) | `accountname: string`, `immaturecoinbaserewards: float64`, `immaturestakegeneration: float64`, `lockedbytickets: float64`, `spendable: float64`, `total: float64`, `unconfirmed: float64`, `votingauthority: float64` |
| `GetBalanceResult` (27) | `balances: []GetAccountBalanceResult`, `blockhash: string`, `totalimmaturecoinbaserewards: float64 (oe)`, `totalimmaturestakegeneration: float64 (oe)`, `totallockedbytickets: float64 (oe)`, `totalspendable: float64 (oe)`, `cumulativetotal: float64 (oe)`, `totalunconfirmed: float64 (oe)`, `totalvotingauthority: float64 (oe)` |
| `GetMultisigOutInfoResult` (41) | `address: string`, `redeemscript: string`, `m: uint8`, `n: uint8`, `pubkeys: []string`, `txhash: string`, `blockheight: uint32`, `blockhash: string`, `spent: bool`, `spentby: string`, `spentbyindex: uint32`, `amount: float64` |
| `CreateMultiSigResult` (58) | `address: string`, `redeemScript: string` |
| `CreateSignatureResult` (65) | `signature: string`, `publickey: string` |
| `GetPeerInfoResult` (71) | `id: int32`, `addr: string`, `addrlocal: string`, `services: string`, `version: uint32`, `subver: string`, `startingheight: int64`, `banscore: int32` |
| `GetStakeInfoResult` (84) | `blockheight: int64`, `difficulty: float64`, `totalsubsidy: float64`, `ownmempooltix: uint32`, `immature: uint32`, `unspent: uint32`, `voted: uint32`, `revoked: uint32`, `unspentexpired: uint32`, `poolsize: uint32 (oe)`, `allmempooltix: uint32 (oe)`, `live: uint32 (oe)`, `proportionlive: float64 (oe)`, `missed: uint32 (oe)`, `proportionmissed: float64 (oe)`, `expired: uint32 (oe)` |
| `GetTicketsResult` (108) | `hashes: []string` |
| `GetTransactionDetailsResult` (117) | `account: string`, `address: string (oe)`, `amount: float64`, `category: string`, `involveswatchonly: bool (oe)`, `fee: *float64 (oe)`, `vout: uint32` |
| `GetTransactionResult` (128) | `amount: float64`, `fee: float64 (oe)`, `confirmations: int64`, `blockhash: string`, `blockindex: int64`, `blocktime: int64`, `txid: string`, `walletconflicts: []string`, `time: int64`, `timereceived: int64`, `details: []GetTransactionDetailsResult`, `hex: string`, `type: string`, `ticketstatus: string (oe)` |
| `GetCFilterV2Result` (146) | `blockhash: string`, `filter: string`, `key: string` |
| `VoteChoice` (153) | `agendaid: string`, `agendadescription: string (oe)`, `choiceid: string`, `choicedescription: string (oe)` |
| `GetVoteChoicesResult` (161) | `version: uint32`, `choices: []VoteChoice` |
| `SyncStatusResult` (167) | `synced: bool`, `initialblockdownload: bool`, `headersfetchprogress: float32` (formatted with 32-bit shortest round-trip, e.g. `0.1`, not `0.10000000149011612`) |
| `InfoResult` (175), alias `InfoWalletResult` (195) | `version: int32`, `protocolversion: int32`, `walletversion: int32`, `balance: float64`, `blocks: int32`, `timeoffset: int64`, `connections: int32`, `proxy: string`, `difficulty: float64`, `testnet: bool`, `keypoololdest: int64`, `keypoolsize: int32`, `unlocked_until: int64`, `paytxfee: float64`, `relayfee: float64`, `errors: string` |
| `ListTransactionsResult` (216) | `account: string`, `address: string (oe)`, `amount: float64`, `blockhash: string (oe)`, `blockindex: *int64 (oe)`, `blocktime: int64 (oe)`, `category: string`, `confirmations: int64`, `fee: *float64 (oe)`, `generated: bool (oe)`, `involveswatchonly: bool (oe)`, `time: int64`, `timereceived: int64`, `txid: string`, `txtype: *ListTransactionsTxType (oe)`, `vout: uint32`, `walletconflicts: []string`, `comment: string (oe)`, `otheraccount: string (oe)` |
| `ListReceivedByAccountResult` (240) | `account: string`, `amount: float64`, `confirmations: uint64` |
| `ListReceivedByAddressResult` (248) | `account: string`, `address: string`, `amount: float64`, `confirmations: uint64`, `txids: []string (oe)`, `involvesWatchonly: bool (oe)` |
| `ListSinceBlockResult` (258) | `transactions: []ListTransactionsResult`, `lastblock: string` |
| `ListUnspentResult` (265) | `txid: string`, `vout: uint32`, `tree: int8`, `txtype: int`, `address: string`, `account: string`, `scriptPubKey: string`, `redeemScript: string (oe)`, `amount: float64`, `confirmations: int64`, `spendable: bool` |
| `RedeemMultiSigOutResult` (281) | `hex: string`, `complete: bool`, `errors: []SignRawTransactionError (oe)` |
| `RedeemMultiSigOutsResult` (289) | `results: []RedeemMultiSigOutResult` |
| `SendToMultiSigResult` (295) | `txhash: string`, `address: string`, `redeemscript: string` |
| `SignRawTransactionError` (303) | `txid: string`, `vout: uint32`, `scriptSig: string`, `sequence: uint32`, `error: string` |
| `SignRawTransactionResult` (313) | `hex: string`, `complete: bool`, `errors: []SignRawTransactionError (oe)` |
| `SignedTransaction` (321) | `signingresult: SignRawTransactionResult`, `sent: bool`, `txhash: *string (oe)` |
| `SignRawTransactionsResult` (329) | `results: []SignedTransaction` |
| `SweepAccountResult` (335) | `unsignedtransaction: string`, `totalpreviousoutputamount: float64`, `totaloutputamount: float64`, `estimatedsignedsize: uint32` |
| `TicketInfoResult` (343) | `hash: string`, `cost: float64`, `votingaddress: string`, `status: string`, `blockhash: string (oe)`, `blockheight: int32`, `vote: string (oe)`, `revocation: string (oe)`, `choices: []VoteChoice (oe)`, `vsphost: string (oe)` |
| `TreasuryPolicyResult` (357) | `key: string`, `policy: string`, `ticket: string (oe)` |
| `TSpendPolicyResult` (364) | `hash: string`, `policy: string`, `ticket: string (oe)` |
| `ValidateAddressResult` (372), alias `ValidateAddressWalletResult` (392) | `isvalid: bool`, `address: string (oe)`, `ismine: bool (oe)`, `iswatchonly: bool (oe)`, `isscript: bool (oe)`, `pubkeyaddr: string (oe)`, `pubkey: string (oe)`, `iscompressed: bool (oe)`, `account: string (oe)`, `addresses: []string (oe)`, `hex: string (oe)`, `script: string (oe)`, `sigsrequired: int32 (oe)`, `accountn: *uint32 (oe)`, `branch: *uint32 (oe)`, `index: *uint32 (oe)` |
| `WalletInfoResult` (395) | `daemonconnected: bool`, `spv: bool`, `unlocked: bool`, `cointype: uint32 (oe)`, `txfee: float64`, `votebits: uint16`, `votebitsextended: string`, `voteversion: uint32`, `voting: bool`, `vsp: string`, `manualtickets: bool`, `birthhash: string`, `birthheight: uint32` |
| `AccountUnlockedResult` (413) | `encrypted: bool`, `unlocked: *bool (oe)` |
| `CreateUnsignedTicketResult` (`methods.go:689`) | `unsignedtickets: []string`, `splittx: string` |

`ListTransactionsTxType` is a string type with constants `LTTTRegular =
"regular"`, `LTTTTicket = "ticket"`, `LTTTVote = "vote"`, `LTTTRevocation =
"revocation"` (`results.go:199-213`).

Mixed-case keys that must not be lower-cased: `redeemScript`
(`CreateMultiSigResult`, `ListUnspentResult`), `scriptPubKey`
(`ListUnspentResult`), `scriptSig` (`SignRawTransactionError`),
`involvesWatchonly` (`ListReceivedByAddressResult`; contrast
`involveswatchonly` elsewhere), `unlocked_until` (`InfoResult`). Note the
two spellings `redeemscript` (`GetMultisigOutInfoResult`,
`SendToMultiSigResult`) and `redeemScript`.

`omitempty` on a pointer omits only a nil pointer: `fee`, `blockindex`,
`txtype`, `txhash`, `unlocked`, `accountn`, `branch`, `index` are emitted
when non-nil even if they point at zero/false.

dcrd result types the wallet returns from its own handlers
(`dcrd/rpc/jsonrpc/types/v4@v4.4.0/chainsvrresults.go`; mirrored in
`dcroxide:crates/dcroxide-rpctypes/src/chainsvrresults.rs`):

| Type (line) | Members |
|---|---|
| `GetBestBlockResult` (114) | `hash: string`, `height: int64` |
| `VersionResult` (565) | `versionstring: string`, `major: uint32`, `minor: uint32`, `patch: uint32`, `prerelease: string`, `buildmetadata: string` |
| `GetBlockHeaderVerboseResult` (139) | `hash`, `powhash`, `confirmations: int64`, `version: int32`, `merkleroot`, `stakeroot`, `votebits: uint16`, `finalstate`, `voters: uint16`, `freshstake: uint8`, `revocations: uint8`, `poolsize: uint32`, `bits`, `sbits: float64`, `height: uint32`, `size: uint32`, `time: int64`, `mediantime: int64`, `nonce: uint32`, `extradata`, `stakeversion: uint32`, `difficulty: float64`, `chainwork`, `previousblockhash (oe)`, `nextblockhash (oe)` (untyped members are `string`) |
| `GetBlockVerboseResult` (63) | `hash`, `powhash`, `confirmations: int64`, `size: int32`, `height: int64`, `version: int32`, `merkleroot`, `stakeroot`, `tx: []string (oe)`, `rawtx: []TxRawResult (oe)`, `stx: []string (oe)`, `rawstx: []TxRawResult (oe)`, `time: int64`, `mediantime: int64`, `nonce: uint32`, `votebits: uint16`, `finalstate`, `voters: uint16`, `freshstake: uint8`, `revocations: uint8`, `poolsize: uint32`, `bits`, `sbits: float64`, `extradata`, `stakeversion: uint32`, `difficulty: float64`, `chainwork`, `previousblockhash`, `nextblockhash (oe)` |
| `TxRawResult` (321) | `hex`, `txid`, `version: int32`, `locktime: uint32`, `expiry: uint32`, `vin: []Vin`, `vout: []Vout`, `blockhash (oe)`, `blockheight: int64 (oe)`, `blockindex: uint32 (oe)`, `confirmations: int64 (oe)`, `time: int64 (oe)`, `blocktime: int64 (oe)` |
| `Vin` (596) | Custom `MarshalJSON` (626-716) emitting one of five shapes chosen in this order: coinbase (`coinbase`, `sequence`, `amountin`, `blockheight`, `blockindex`), stakebase (`stakebase`, …same four), treasurybase (`treasurybase: true`, …), treasury spend (`treasuryspend`, …), regular (`txid`, `vout`, `tree`, `sequence`, `amountin`, `blockheight`, `blockindex`, `scriptSig`) |
| `ScriptSig` (589) | `asm`, `hex` |
| `Vout` (718) | `value: float64`, `n: uint32`, `version: uint16`, `scriptPubKey: ScriptPubKeyResult` |
| `ScriptPubKeyResult` (576) | `asm`, `hex (oe)`, `reqSigs: int32 (oe)`, `type`, `addresses: []string (oe)`, `commitamt: *float64 (oe)`, `version: uint16` |
| `GetTxOutResult` (388) | `bestblock`, `confirmations: int64`, `value: float64`, `scriptPubKey: ScriptPubKeyResult`, `coinbase: bool` |
| `InfoChainResult` (199) | decode-only (dcrd `getinfo` reply): `version`, `protocolversion`, `blocks`, `timeoffset`, `connections`, `proxy`, `difficulty`, `testnet`, `relayfee`, `errors`, `txindex` |

### 4.7 Address-array marshalers (`marshaling.go`)

`addressArrayMarshaler(n, s)` returns a `json.Marshaler` whose output is
`[` + the `n` strings each wrapped in `"` and separated by `,` + `]`, with
no whitespace and **no escaping of the strings** (`marshaling.go:14-36`).
`n == 0` yields `[]` (never `null`). `knownAddressMarshaler` renders
`wallet.KnownAddress.String()`; `addressStringsMarshaler` renders the given
strings. `encoding/json` validates and compacts marshaler output, so a
string containing `"` or `\` would make the whole response fail to marshal;
base58 address strings cannot contain such characters. The buffer
pre-allocation (`2 + n*38` bytes) is an optimisation only.

### 4.8 API version reporting

| Constant | Value | Source |
|---|---|---|
| `jsonrpcSemverString` | `"11.0.0"` | `internal/rpc/jsonrpc/methods.go:56` |
| `jsonrpcSemverMajor`, `Minor`, `Patch` | `11`, `0`, `0` | `methods.go:57-59` |

The only place the version is reported is the `version` method
(`methods.go:5438-5462`). The result is a JSON object (Go
`map[string]VersionResult`, so keys are sorted) built as follows:

1. Start with an empty map.
2. If the network backend is a dcrd RPC syncer, call dcrd `version` with no
   parameters and unmarshal the reply into the map (dcrd supplies `dcrd`
   and `dcrdjsonrpcapi`). A call error fails the request through
   `convertError`.
3. Set key `dcrwallet` to `{versionstring: version.String(), major:
   version.Major, minor: version.Minor, patch: version.Patch, prerelease:
   version.PreRelease, buildmetadata: version.BuildMetadata}`. At the pin:
   major 2, minor 2, patch 0, prerelease `"pre"`; build metadata is the
   first 9 characters of the VCS revision when the binary was built from a
   git checkout, else `""`; `versionstring` is `2.2.0-pre` optionally
   followed by `+<buildmetadata>` (`version/version.go`).
4. Set key `dcrwalletjsonrpcapi` to
   `{"versionstring":"11.0.0","major":11,"minor":0,"patch":0,"prerelease":"","buildmetadata":""}`.

Step 3 and 4 overwrite same-named keys returned by dcrd. No wallet needs to
be loaded for `version`. The gRPC API has its own version (sibling spec).

### 4.9 Method table

The dispatch table is `handlers` (`methods.go:85-200`): 103 implemented
entries and 8 `noHelp` entries. Two further method names are intercepted by
the transport before the table (`authenticate`, `stop`). Everything else is
dcrd passthrough (5.5.1).

Column legend:

- **Wallet**: `yes` = fails with `errUnloadedWallet` (`-4`, "request
  requires a wallet but wallet has not loaded yet") when no wallet is
  loaded; `no` = works without a wallet; `late` = some parameter or
  configuration checks run before the wallet check.
- **Backend** (what the handler itself does with the network backend; the
  wallet layer may use it further):
  - `-` not touched by the handler.
  - `req(L)` requires a backend via the loader; without one fails with
    `errNoNetwork` (`-9`, "disconnected from network").
  - `req(W)` requires a backend via `wallet.NetworkBackend()`; without one
    the wallet error `wallet.NetworkBackend: Decred network is unreachable`
    (kind `NoPeers`) is converted to code `-9`.
  - `fwd` when the backend is a dcrd RPC syncer the request is forwarded to
    dcrd under the same method name with the parsed (defaulted) arguments
    and dcrd's raw `result` is returned; under SPV the handler answers from
    wallet data.
  - `opt` uses dcrd RPC for extra data when the backend is a dcrd RPC
    syncer, otherwise proceeds without it.
- **Unlock**: `E` = the handler explicitly maps a wallet `Locked` error to
  code `-13`; `K` = the handler invokes a wallet operation that needs
  private keys and relies on `convertError` (wallet kind `Locked` gives
  code `-13` with the wallet's own error text); `-` = no private-key use
  in the handler. `K`/`-` are classifications from the wallet call made by
  the handler; the per-method specs are authoritative for the exact
  condition.

| Method | Handler (`(*Server).`) | Wallet | Backend | Unlock |
|---|---|---|---|---|
| `abandontransaction` | `abandonTransaction` | yes | - | - |
| `accountaddressindex` | `accountAddressIndex` | yes | - | - |
| `accountsyncaddressindex` | `accountSyncAddressIndex` | yes | - | - |
| `accountunlocked` | `accountUnlocked` | yes | - | - |
| `addmultisigaddress` | `addMultiSigAddress` | late (`account` other than the imported account is rejected with `errNotImportedAccount` first, `methods.go:432-435`) | - | - |
| `addtransaction` | `addTransaction` | yes | - | - |
| `auditreuse` | `auditReuse` | yes | - | - |
| `consolidate` | `consolidate` | yes | - | K |
| `createmultisig` | `createMultiSig` | yes | - | - |
| `createnewaccount` | `createNewAccount` | yes | - | E (`-13`, "creating new accounts requires an unlocked wallet", `methods.go:2212-2213`) |
| `createrawtransaction` | `createRawTransaction` | no | - | - |
| `createsignature` | `createSignature` | yes | - | K |
| `debuglevel` | `debugLevel` | no | - | - |
| `disapprovepercent` | `disapprovePercent` | yes | - | - |
| `discoverusage` | `discoverUsage` | yes | req(L) | K when `discoveraccounts` is true |
| `dumpprivkey` | `dumpPrivKey` | yes | - | E |
| `fundrawtransaction` | `fundRawTransaction` | yes | - | - |
| `getaccount` | `getAccount` | yes | - | - |
| `getaccountaddress` | `getAccountAddress` | yes | - | - |
| `getaddressesbyaccount` | `getAddressesByAccount` | yes | - | - |
| `getbalance` | `getBalance` | yes | - | - |
| `getbestblock` | `getBestBlock` | yes | - | - |
| `getbestblockhash` | `getBestBlockHash` | yes | - | - |
| `getblockcount` | `getBlockCount` | yes | - | - |
| `getblockhash` | `getBlockHash` | yes | - | - |
| `getblockheader` | `getBlockHeader` | yes | req(W), fwd | - |
| `getblock` | `getBlock` | yes | req(W), fwd | - |
| `getcoinjoinsbyacct` | `getcoinjoinsbyacct` | yes | - | - |
| `getcurrentnet` | `getCurrentNet` | no | - | - |
| `getinfo` | `getInfo` | yes | opt (dcrd `getinfo`) | - |
| `getmasterpubkey` | `getMasterPubkey` | yes | - | - |
| `getmultisigoutinfo` | `getMultisigOutInfo` | yes | - | - |
| `getnewaddress` | `getNewAddress` | yes | - | - |
| `getpeerinfo` | `getPeerInfo` | yes | req(W); SPV: local peers; dcrd: calls dcrd `getpeerinfo` and re-encodes through the wallet's 8-member `GetPeerInfoResult`; any other backend: `null` | - |
| `getrawchangeaddress` | `getRawChangeAddress` | yes | - | - |
| `getreceivedbyaccount` | `getReceivedByAccount` | yes | - | - |
| `getreceivedbyaddress` | `getReceivedByAddress` | yes | - | - |
| `getstakeinfo` | `getStakeInfo` | yes | opt (precise stake info through dcrd) | - |
| `gettickets` | `getTickets` | yes | opt (live-ticket query through dcrd) | - |
| `gettransaction` | `getTransaction` | yes | - | - |
| `gettxout` | `getTxOut` | yes | req(W), fwd | - |
| `getunconfirmedbalance` | `getUnconfirmedBalance` | yes | - | - |
| `getvotechoices` | `getVoteChoices` | yes | - | - |
| `getwalletfee` | `getWalletFee` | yes | - | - |
| `help` | `help` | no | opt (dcrd `help`) | - |
| `getcfilterv2` | `getCFilterV2` | late | - | - |
| `importcfiltersv2` | `importCFiltersV2` | yes | - | - |
| `importprivkey` | `importPrivKey` | yes | req(L) only when `rescan` is true | E |
| `importpubkey` | `importPubKey` | yes | req(L) only when `rescan` is true | - |
| `importscript` | `importScript` | yes | req(L) only when `rescan` is true | - |
| `importxpub` | `importXpub` | yes | - | - |
| `listaccounts` | `listAccounts` | yes | - | - |
| `listaddresstransactions` | `listAddressTransactions` | yes | - | - |
| `listalltransactions` | `listAllTransactions` | yes | - | - |
| `listlockunspent` | `listLockUnspent` | yes | - | - |
| `listreceivedbyaccount` | `listReceivedByAccount` | yes | - | - |
| `listreceivedbyaddress` | `listReceivedByAddress` | yes | - | - |
| `listsinceblock` | `listSinceBlock` | yes | - | - |
| `listtransactions` | `listTransactions` | yes | - | - |
| `listunspent` | `listUnspent` | yes | - | - |
| `lockaccount` | `lockAccount` | yes | - | - |
| `lockunspent` | `lockUnspent` | yes | - | - |
| `mixaccount` | `mixAccount` | late (mixing-enabled check first) | - | K |
| `mixoutput` | `mixOutput` | late (mixing-enabled check first) | - | K |
| `purchaseticket` | `purchaseTicket` | yes | req(W) | K |
| `processunmanagedticket` | `processUnmanagedTicket` | late (hash and `VSPHost` checks first) | - | - |
| `redeemmultisigout` | `redeemMultiSigOut` | yes | opt (via `signRawTransaction`) | K |
| `redeemmultisigouts` | `redeemMultiSigOuts` | yes | opt (via `signRawTransaction`) | K |
| `renameaccount` | `renameAccount` | yes | - | - |
| `rescanwallet` | `rescanWallet` | yes | req(L) | - |
| `sendfrom` | `sendFrom` | yes | - | E |
| `sendfromtreasury` | `sendFromTreasury` | yes | req(L) at publish time | K |
| `sendmany` | `sendMany` | yes | - | E |
| `sendrawtransaction` | `sendRawTransaction` | yes | req(W) | - |
| `sendtoaddress` | `sendToAddress` | yes | - | E |
| `sendtomultisig` | `sendToMultiSig` | yes | - | K |
| `sendtotreasury` | `sendToTreasury` | yes | - | E |
| `setaccountpassphrase` | `setAccountPassphrase` | yes | - | K |
| `setdisapprovepercent` | `setDisapprovePercent` | late (mainnet rejection first) | - | - |
| `settreasurypolicy` | `setTreasuryPolicy` | yes | - | - |
| `settspendpolicy` | `setTSpendPolicy` | yes | - | - |
| `settxfee` | `setTxFee` | yes | - | - |
| `setvotechoice` | `setVoteChoice` | yes | - | - |
| `signmessage` | `signMessage` | yes | - | E |
| `signrawtransaction` | `signRawTransaction` | yes | opt (dcrd `gettxout` for unknown inputs) | K |
| `signrawtransactions` | `signRawTransactions` | yes, but only through the per-element call to `signRawTransaction`; the handler itself has no wallet check, so an empty `rawtxs` array never fails with `errUnloadedWallet` (`send` false: `{"results":[]}`; `send` true: backend check only) (`methods.go:5123-5203`) | opt, plus req(L) when `send` is true | K |
| `spendoutputs` | `spendOutputs` | yes | req(W) | K |
| `sweepaccount` | `sweepAccount` | yes | - | - |
| `syncstatus` | `syncStatus` | yes | req(W) | - |
| `ticketinfo` | `ticketInfo` | yes | - | - |
| `treasurypolicy` | `treasuryPolicy` | yes | - | - |
| `tspendpolicy` | `tspendPolicy` | yes | - | - |
| `unlockaccount` | `unlockAccount` | yes | - | - |
| `validateaddress` | `validateAddress` | yes | - | - |
| `validatepredcp0005cf` | `validatePreDCP0005CF` | yes | - | - |
| `verifymessage` | `verifyMessage` | no | - | - |
| `version` | `version` | no | opt (dcrd `version`) | - |
| `walletinfo` | `walletInfo` | yes | opt (dcrd `ping`) | - |
| `walletislocked` | `walletIsLocked` | yes | - | - |
| `walletlock` | `walletLock` | yes | - | - |
| `walletpassphrase` | `walletPassphrase` | yes | - | - |
| `walletpassphrasechange` | `walletPassphraseChange` | yes | - | - |
| `walletpubpassphrasechange` | `walletPubPassphraseChange` | yes | - | - |

Non-table and dead entries:

| Method | Where handled | Behaviour |
|---|---|---|
| `authenticate` | websocket loop (`server.go:394-419`); POST (`server.go:585-589`) | Websocket: section 5.4.3 step 3. POST: request dropped, empty HTTP 200. |
| `stop` | websocket loop (`server.go:427-441`); POST (`server.go:590-593`) | Replies then requests process shutdown. Websocket result string `dcrwallet stopping.`; POST result string `dcrwallet stopping` (no full stop). No parameter parsing is performed; any `params` are ignored. Requires authentication only. |
| `backupwallet`, `getwalletinfo`, `importwallet`, `listaddressgroupings` | `handlers`, `fn: unimplemented`, `noHelp` (`methods.go:192-195`) | Intended reply `-1 "Method unimplemented"`. Actual reply `-32600 "Invalid request"` (Quirk Q1). Never forwarded to dcrd. |
| `dumpwallet`, `encryptwallet`, `move`, `setaccount` | `handlers`, `fn: unsupported`, `noHelp` (`methods.go:196-199`) | Intended reply `-1 "Request unsupported by dcrwallet"`. Actual reply `-32600 "Invalid request"` (Quirk Q1). |
| any other name | `lazyApplyHandler` passthrough (`methods.go:231-260`) | Section 5.5.1. |

### 4.10 Help data

#### 4.10.1 Generated file layout (`internal/rpc/jsonrpc/rpcserverhelp.go`)

- `func helpDescsEnUS() map[string]string`: 103 entries keyed by method
  name; each value is the complete help text of the method (total 81,739
  bytes of text). Built on first use, not at init.
- `var localeHelpDescs = map[string]func() map[string]string{"en_US":
  helpDescsEnUS}`.
- `var requestUsages string`: the 103 one-line usages joined with `\n`, no
  trailing newline (4,154 bytes).
- The file starts with the line `// AUTOGENERATED by
  internal/rpchelp/genrpcserverhelp.go; do not edit.`, a blank line and
  `package jsonrpc` (`rpcserverhelp.go:1-3`). The package name is the
  generator's first command-line argument; with no argument it would be
  `main` (`genrpcserverhelp.go:84-87`). Every key and value is a Go
  interpreted string literal produced by `%q` (so `\n`, `\t`, `\"` escapes;
  non-ASCII printable characters are kept literally), and the file is
  post-processed by `gofmt -w`, which aligns the map values. An extraction
  tool must unescape Go string literals, not JSON strings.

SHA-256 of the file at the pin:
`6107f68df16fcd0383e5d95aff20bd8e9671f7ebb86a1977ad50aa3686aaa9df`.
The Rust port must embed these 104 strings verbatim; the simplest faithful
route is to extract them from the Go file with a one-off tool and commit
them as data, with a test that re-derives them through the dcrjson help
generator port (`dcroxide:crates/dcroxide-dcrjson/src/help.rs`,
`usage.rs`).

#### 4.10.2 `requestUsages` (exact text; the `help` result with no argument)

```
abandontransaction "hash"
accountaddressindex "account" branch
accountsyncaddressindex "account" branch index
accountunlocked "account"
addmultisigaddress nrequired ["key",...] ("account")
addtransaction "blockhash" "transaction"
auditreuse (since)
consolidate inputs ("account" "address")
createmultisig nrequired ["key",...]
createnewaccount "account"
createrawtransaction [{"amount":n.nnn,"txid":"value","vout":n,"tree":n},...] {"address":amount,...} (locktime expiry)
createsignature "address" inputindex hashtype "previouspkscript" "serializedtransaction"
debuglevel "levelspec"
disapprovepercent
discoverusage ("startblock" discoveraccounts gaplimit)
dumpprivkey "address"
fundrawtransaction "hexstring" "fundaccount" ({"changeaddress":changeaddress,"feerate":feerate,"conftarget":conftarget})
getaccount "address"
getaccountaddress "account"
getaddressesbyaccount "account"
getbalance ("account" minconf=1)
getbestblock
getbestblockhash
getblockcount
getblockhash index
getblockheader "hash" (verbose=true)
getblock "hash" (verbose=true verbosetx=false)
getcoinjoinsbyacct
getcurrentnet
getinfo
getmasterpubkey ("account")
getmultisigoutinfo "hash" index
getnewaddress ("account" "gappolicy")
getpeerinfo
getrawchangeaddress ("account")
getreceivedbyaccount "account" (minconf=1)
getreceivedbyaddress "address" (minconf=1)
getstakeinfo
gettickets includeimmature
gettransaction "txid" (includewatchonly=false)
gettxout "txid" vout tree (includemempool=true)
getunconfirmedbalance ("account")
getvotechoices ("tickethash")
getwalletfee
getcfilterv2 "blockhash"
help ("command")
importcfiltersv2 startheight ["filter",...]
importprivkey "privkey" ("label" rescan=true scanfrom)
importpubkey "pubkey" ("label" rescan=true scanfrom)
importscript "hex" (rescan=true scanfrom)
importxpub "name" "xpub"
listaccounts (minconf=1)
listaddresstransactions ["address",...] ("account")
listalltransactions ("account")
listlockunspent ("account")
listreceivedbyaccount (minconf=1 includeempty=false includewatchonly=false)
listreceivedbyaddress (minconf=1 includeempty=false includewatchonly=false)
listsinceblock ("blockhash" targetconfirmations=1 includewatchonly=false)
listtransactions ("account" count=10 from=0 includewatchonly=false)
listunspent (minconf=1 maxconf=9999999 ["address",...] "account")
lockaccount "account"
lockunspent unlock [{"amount":n.nnn,"txid":"value","vout":n,"tree":n},...]
mixaccount
mixoutput "outpoint"
processunmanagedticket "tickethash"
purchaseticket "fromaccount" (minconf=1 numtickets=1 expiry "comment" dontsigntx)
redeemmultisigout "hash" index tree ("address")
redeemmultisigouts "fromscraddress" ("toaddress" number)
renameaccount "oldaccount" "newaccount"
rescanwallet (beginheight=0)
sendfrom "fromaccount" "toaddress" amount (minconf=1 "comment" "commentto")
sendfromtreasury "key" amounts
sendmany "fromaccount" {"address":amount,...} (minconf=1 "comment")
sendrawtransaction "hextx" (allowhighfees=false)
sendtoaddress "address" amount ("comment" "commentto")
sendtomultisig "fromaccount" amount ["pubkey",...] (nrequired=1 minconf=1 "comment")
sendtotreasury amount
setaccountpassphrase "account" "passphrase"
setdisapprovepercent percent
settreasurypolicy "key" "policy" ("ticket")
settspendpolicy "hash" "policy" ("ticket")
settxfee amount
setvotechoice "agendaid" "choiceid" ("tickethash")
signmessage "address" "message"
signrawtransaction "rawtx" ([{"txid":"value","vout":n,"tree":n,"scriptpubkey":"value","redeemscript":"value"},...] ["privkey",...] flags="ALL")
signrawtransactions ["rawtx",...] (send=true)
spendoutputs "account" ["previousoutpoint",...] [{"address":"value","amount":n.nnn},...]
sweepaccount "sourceaccount" "destinationaddress" (requiredconfirmations feeperkb)
syncstatus
ticketinfo (startheight=0)
treasurypolicy ("key" "ticket")
tspendpolicy ("hash" "ticket")
unlockaccount "account" "passphrase"
validateaddress "address"
validatepredcp0005cf
verifymessage "address" "signature" "message"
version
walletinfo
walletislocked
walletlock
walletpassphrase "passphrase" timeout
walletpassphrasechange "oldpassphrase" "newpassphrase"
walletpubpassphrasechange "oldpassphrase" "newpassphrase"
```

The order is the order of `rpchelp.Methods`
(`internal/rpchelp/methods.go:30-132`): alphabetical except that `getblock`
follows `getblockheader` and `getcfilterv2` sits between `getwalletfee` and
`help`.

#### 4.10.3 Description map (`internal/rpchelp/helpdescs_en_US.go`)

`helpDescsEnUS` holds 720 keys. Key grammar (dcrjson,
`dcrd/dcrjson/v4@v4.2.0/help.go:471-507`):

| Key form | Meaning |
|---|---|
| `<method>--synopsis` | Synopsis paragraph (may contain `\n`). |
| `<method>-<lowerfieldname>` | Description of a command argument (lower-cased Go field name). |
| `<method>-<lowerfieldname>--key` / `--value` / `--desc` | Map-typed argument: example key, example value, description. |
| `<lowertypename>-<jsonname>` | Description of a member of a result or nested argument struct; the type name is the lower-cased Go struct name, the member name is the JSON tag name (case preserved, e.g. `listunspentresult-scriptPubKey`). |
| `<method>--condition<N>` | Condition label for result N; used only when the method has more than one result type. |
| `<method>--result<N>` | Description of a primitive result N; unused when result N is a struct. |
| `<method>--result<N>--key` / `--value` / `--desc` | Map-typed result. |

Fallback labels come from `baseHelpDescs`
(`help.go:19-42`): `Arguments`, `None`, `Result`, `Nothing`, `default`,
`optional`, `required`, `numeric`, `string`, `boolean`, `array of `,
`object`, `value`, example values `value`, `true|false`, `data`,
`unknown`. Their keys and exact values:

| Key | Value |
|---|---|
| `help-arguments` | `Arguments` |
| `help-arguments-none` | `None` |
| `help-result` | `Result` |
| `help-result-nothing` | `Nothing` |
| `help-default` | `default` |
| `help-optional` | `optional` |
| `help-required` | `required` |
| `json-type-numeric` | `numeric` |
| `json-type-string` | `string` |
| `json-type-bool` | `boolean` |
| `json-type-array` | `array of ` (trailing space) |
| `json-type-object` | `object` |
| `json-type-value` | `value` |
| `json-example-string` | `value` |
| `json-example-bool` | `true\|false` |
| `json-example-map-data` | `data` (never looked up by `help.go`) |
| `json-example-unknown` | `unknown` |

Lookup order is the caller's description map first, then `baseHelpDescs`
(`help.go:545-555`), so a description map may override a label. The wallet
map defines none of these keys.

Structure of `internal/rpchelp/helpdescs_en_US.go`: one `map[string]string`
literal, 720 distinct keys, grouped under `// <Type> help.` comments in
roughly alphabetical method order with each result type's keys following
the first method that returns it. Key groups (text before the first `-`)
and their key counts, in file order; a group is either a method name or a
lower-cased Go type name:

`abandontransaction` 2, `accountaddressindex` 4, `accountsyncaddressindex`
4, `accountunlocked` 2, `accountunlockedresult` 2, `addmultisigaddress` 5,
`addtransaction` 3, `auditreuse` 5, `consolidate` 5, `createmultisig` 3,
`createmultisigresult` 2, `createsignatureresult` 2, `createnewaccount` 2,
`createrawtransaction` 9, `createsignature` 6, `debuglevel` 6,
`disapprovepercent` 2, `discoverusage` 4, `dumpprivkey` 3,
`fundrawtransaction` 4, `fundrawtransactionoptions` 3,
`fundrawtransactionresult` 2, `getaccountaddress` 3, `getaccount` 3,
`getaddressesbyaccount` 3, `getbalance` 3, `getbalanceresult` 9,
`getaccountbalanceresult` 8, `getbalancetomaintain` 2, `getbestblock` 1,
`getbestblockhash` 2, `getbestblockresult` 2, `getblockcount` 2,
`getblockhash` 3, `getblockheader` 6, `getblockheaderverboseresult` 25,
`getblock` 7, `getblockverboseresult` 29, `txrawresult` 13, `vin` 12,
`scriptsig` 2, `vout` 4, `scriptpubkeyresult` 7, `getcfilterv2` 2,
`getcfilterv2result` 3, `syncstatus` 1, `syncstatusresult` 3,
`getcurrentnet` 2, `getinfo` 1, `getmasterpubkey` 3, `getmultisigoutinfo`
3, `getmultisigoutinforesult` 12, `getnewaddress` 4, `getpeerinfo` 1,
`getpeerinforesult` 8, `getrawchangeaddress` 3, `getreceivedbyaccount` 4,
`getreceivedbyaddress` 4, `getstakeinfo` 1, `getstakeinforesult` 16,
`getticketmaxprice` 2, `gettickets` 2, `getticketsresult` 1,
`gettransaction` 3, `gettransactiondetailsresult` 7,
`gettransactionresult` 14, `getunconfirmedbalance` 3, `getvotechoices` 2,
`getvotechoicesresult` 2, `getwalletfee` 2, `help` 6, `gettxout` 5,
`gettxoutresult` 5, `importcfiltersv2` 3, `importprivkey` 5,
`importpubkey` 5, `importscript` 4, `importxpub` 3, `inforesult` 16,
`listaccounts` 5, `listaddresstransactions` 3, `listalltransactions` 2,
`listlockunspent` 2, `listreceivedbyaccount` 4,
`listreceivedbyaccountresult` 3, `listreceivedbyaddress` 4,
`listreceivedbyaddressresult` 6, `listsinceblock` 8,
`listsinceblockresult` 2, `listtransactions` 5, `listtransactionsresult`
19, `listunspent` 5, `listunspentresult` 11, `lockaccount` 2,
`lockunspent` 4, `mixaccount` 2, `mixoutput` 2, `purchaseticket` 9,
`processunmanagedticket` 2, `redeemmultisigout` 5,
`redeemmultisigoutresult` 3, `redeemmultisigouts` 4, `renameaccount` 3,
`rescanwallet` 2, `sendfrom` 8, `sendfromtreasury` 7, `sendmany` 9,
`sendrawtransaction` 4, `sendtoaddress` 6, `sendtomultisig` 8,
`sendtotreasury` 3, `setaccountpassphrase` 3, `setbalancetomaintain` 3,
`setdisapprovepercent` 2, `setgenerate` 3, `getcoinjoinsbyacct` 4,
`setticketmaxprice` 2, `settreasurypolicy` 4, `settspendpolicy` 4,
`settxfee` 3, `setvotechoice` 4, `signmessage` 4, `signrawtransaction` 5,
`signrawtransactionerror` 5, `signrawtransactionresult` 3,
`signrawtransactions` 3, `signrawtransactionsresult` 1,
`signedtransaction` 3, `spendoutputs` 5, `addressamountpair` 2,
`sweepaccount` 5, `sweepaccountresult` 4, `ticketinfo` 3,
`ticketinforesult` 10, `transactioninput` 4, `treasurypolicy` 7,
`treasurypolicyresult` 3, `tspendpolicy` 7, `tspendpolicyresult` 3,
`unlockaccount` 3, `validateaddress` 2, `validateaddressresult` 16,
`validatepredcp0005cf` 2, `verifymessage` 5, `version` 4, `votechoice` 4,
`walletinfo` 1, `walletinforesult` 13, `walletislocked` 2, `walletlock` 1,
`walletpassphrasechange` 3, `walletpassphrase` 3,
`walletpubpassphrasechange` 3.

That is 153 groups. One group is not contiguous in the file: the
`getbalanceresult` keys are `getbalanceresult-balances`
(`helpdescs_en_US.go:160`), then the eight `getaccountbalanceresult-*`
keys (`161-168`), then the remaining eight `getbalanceresult-*` keys
(`169-176`). Type-name groups present: the wallet result structs of
4.6 that appear in a help result (not `CreateUnsignedTicketResult`,
`RedeemMultiSigOutsResult`, `SendToMultiSigResult`, which no help entry
references), the nested argument types `fundrawtransactionoptions`,
`addressamountpair` and dcrd's `transactioninput`, and the dcrd result
types `getbestblockresult`, `getblockheaderverboseresult`,
`getblockverboseresult`, `txrawresult`, `vin`, `scriptsig`, `vout`,
`scriptpubkeyresult`, `gettxoutresult`. There is **no `rawtxinput` group**:
the `signrawtransaction` `inputs` argument is never expanded (Quirk Q33).
`InfoWalletResult` and `ValidateAddressWalletResult` are Go type aliases,
so their keys use the underlying names `inforesult` and
`validateaddressresult`. The exact description strings are data to be
extracted from the Go file together with the generated texts if the
regeneration cross-check of Decision D7 is implemented; they are not
otherwise observable.

Keys present in the map but not consumed by any of the 103 generated texts
(dead, safe to drop only if the texts are embedded rather than
regenerated): `debuglevel--condition0`, `debuglevel--condition1`,
`debuglevel--result1`, `getbalancetomaintain--synopsis`,
`getbalancetomaintain--result0`, `getblockheader--condition0`,
`getblockheader--condition1`, `getblockheader--result0`,
`getblock--condition0`, `getblock--condition1`, `getblock--result0`,
`getticketmaxprice--synopsis`, `getticketmaxprice--result0`,
`listsinceblock--condition0`, `listsinceblock--condition1`,
`listsinceblock--result0`, `listsinceblock--result1`, `mixaccount-account`,
`purchaseticket-nosplittransaction`, `setbalancetomaintain--synopsis`,
`setbalancetomaintain-balance`, `setbalancetomaintain--result0`,
`setgenerate--synopsis`, `setgenerate-generate`,
`setgenerate-genproclimit`, `setticketmaxprice--synopsis`,
`setticketmaxprice-max`, `ticketinfo--result0`, `treasurypolicy--result0`,
`treasurypolicy--result1`, `tspendpolicy--result0`,
`tspendpolicy--result1`. (Derived by checking which description values
occur in the generated texts and from the single-result rule; a few keys
with very short values such as `Unset` cannot be distinguished this way.)

#### 4.10.4 Result types per method (`internal/rpchelp/methods.go:30-132`)

Only needed to regenerate help. `nil` means "Result: Nothing".

| Result types | Methods |
|---|---|
| `nil` | `abandontransaction`, `accountsyncaddressindex`, `addtransaction`, `createnewaccount`, `discoverusage`, `importcfiltersv2`, `importprivkey`, `importpubkey`, `importscript`, `importxpub`, `lockaccount`, `mixaccount`, `mixoutput`, `processunmanagedticket`, `renameaccount`, `rescanwallet`, `setaccountpassphrase`, `setdisapprovepercent`, `settreasurypolicy`, `settspendpolicy`, `setvotechoice`, `unlockaccount`, `walletlock`, `walletpassphrase`, `walletpassphrasechange`, `walletpubpassphrasechange` |
| `*string` | `addmultisigaddress`, `consolidate`, `createrawtransaction`, `debuglevel`, `dumpprivkey`, `getaccount`, `getaccountaddress`, `getbestblockhash`, `getblockhash`, `getmasterpubkey`, `getnewaddress`, `getrawchangeaddress`, `purchaseticket`, `sendfrom`, `sendfromtreasury`, `sendmany`, `sendrawtransaction`, `sendtoaddress`, `sendtomultisig`, `sendtotreasury`, `signmessage`, `spendoutputs` |
| `*string`, `*string` | `help` |
| `*float64` | `getblockcount`, `getreceivedbyaccount`, `getreceivedbyaddress`, `getunconfirmedbalance`, `getwalletfee` |
| `*bool` | `lockunspent`, `settxfee`, `validatepredcp0005cf`, `verifymessage`, `walletislocked` |
| `*int` | `accountaddressindex` |
| `*uint32` | `disapprovepercent`, `getcurrentnet` |
| `*[]string` | `getaddressesbyaccount` |
| `*map[string][]string` | `auditreuse` |
| `*map[string]uint32` | `getcoinjoinsbyacct` |
| `*map[string]float64` | `listaccounts` |
| `*map[string]dcrdtypes.VersionResult` | `version` |
| `*[]types.ListTransactionsResult` | `listaddresstransactions`, `listalltransactions`, `listtransactions` |
| `*[]dcrdtypes.TransactionInput` | `listlockunspent` |
| `*[]types.ListReceivedByAccountResult` | `listreceivedbyaccount` |
| `*[]types.ListReceivedByAddressResult` | `listreceivedbyaddress` |
| `*[]types.TicketInfoResult` | `ticketinfo` |
| `*[]types.TreasuryPolicyResult`, `*types.TreasuryPolicyResult` | `treasurypolicy` |
| `*[]types.TSpendPolicyResult`, `*types.TSpendPolicyResult` | `tspendpolicy` |
| single struct pointer | `accountunlocked` (`AccountUnlockedResult`), `createmultisig` (`CreateMultiSigResult`), `createsignature` (`CreateSignatureResult`), `fundrawtransaction` (`FundRawTransactionResult`), `getbalance` (`GetBalanceResult`), `getbestblock` (dcrd `GetBestBlockResult`), `getblockheader` (dcrd `GetBlockHeaderVerboseResult`), `getblock` (dcrd `GetBlockVerboseResult`), `getinfo` (`InfoWalletResult`), `getmultisigoutinfo` (`GetMultisigOutInfoResult`), `getpeerinfo` (`GetPeerInfoResult`), `getstakeinfo` (`GetStakeInfoResult`), `gettickets` (`GetTicketsResult`), `gettransaction` (`GetTransactionResult`), `gettxout` (dcrd `GetTxOutResult`), `getvotechoices` (`GetVoteChoicesResult`), `getcfilterv2` (`GetCFilterV2Result`), `listsinceblock` (`ListSinceBlockResult`), `listunspent` (`ListUnspentResult`), `redeemmultisigout` and `redeemmultisigouts` (both `RedeemMultiSigOutResult`), `signrawtransaction` (`SignRawTransactionResult`), `signrawtransactions` (`SignRawTransactionsResult`), `sweepaccount` (`SweepAccountResult`), `syncstatus` (`SyncStatusResult`), `validateaddress` (`ValidateAddressWalletResult`), `walletinfo` (`WalletInfoResult`) |

The help result types are documentation only and several disagree with
what handlers return (section 10, Q13).

### 4.11 HTTP wire details

Routes on every JSON-RPC listener (Go `http.ServeMux`,
`server.go:129`, `146`):

| Pattern | Handler | Notes |
|---|---|---|
| `/` | POST handler behind `throttled(MaxPOSTClients)` | Matches every path except exactly `/ws`. **The HTTP method is not checked**: GET, PUT etc. are processed the same way (an empty body yields the invalid-request reply). ServeMux redirects non-canonical paths (`//`, `/./`, `/a/../b`; not for `CONNECT`) to the cleaned path before the handler runs. The redirect status is Go-release dependent: the Go 1.27 standard library source uses 307 Temporary Redirect (`net/http/server.go`, `ServeMux.findHandler`), older releases used 301 Moved Permanently. UNVERIFIED which status the Go 1.26 toolchain named in `go.mod` emits; settle by sending `POST //` to the reference binary. |
| `/ws` | websocket handler behind `throttled(MaxWebsocketClients)` | Must be an HTTP/1.1 GET upgrade. |

HTTP-level responses. `http.Error` sets `Content-Type: text/plain;
charset=utf-8` and `X-Content-Type-Options: nosniff` and appends `\n` to
the body:

| Condition | Status | Extra headers | Body |
|---|---|---|---|
| Concurrency threshold reached (either route) | 429 | none | `429 Too Many Requests\n` |
| Auth failure on `/` | 401 | `WWW-Authenticate: Basic realm="dcrwallet RPC"`, `Connection: close` | `401 Unauthorized\n` |
| Wrong `Authorization` on `/ws` (a missing header is not a failure there) | 401 | `WWW-Authenticate: Basic realm="dcrwallet RPC"` | `401 Unauthorized\n` |
| Body larger than 4 MiB on `/` | 413 | `Connection: close` | `413 Request Too Large\n` |
| Other body read error on `/` | 500 | `Connection: close` | `Request read failed\n` |
| Response marshalling failure on `/` (unsupported id type, unmarshalable result) | 500 | `Connection: close` | `500 Internal Server Error\n` |
| `authenticate` sent to `/` | 200 | `Connection: close`, `Content-Type: application/json`, `Content-Length: 0` | empty |
| Any executed request on `/`, success or JSON-RPC error | 200 | `Connection: close`, `Content-Type: application/json` | the response object, no trailing newline |
| Websocket handshake rejected by gorilla (`websocket@v1.5.1/server.go:78-87`, `126-161`): checks in order `Connection: upgrade` token, `Upgrade: websocket` token, method GET, `Sec-Websocket-Version: 13`, valid 16-byte base64 `Sec-WebSocket-Key` | 400 (405 for the method check) | `Sec-Websocket-Version: 13` | `Bad Request\n` (`Method Not Allowed\n`) |
| Websocket handshake accepted | 101 | `Upgrade: websocket`, `Connection: Upgrade`, `Sec-WebSocket-Accept: <key>` | none |

Connection persistence: the `/` handler sets `Connection: close` (and
`Request.Close`) before authentication, so every reply it produces closes
the connection. The 429 reply is written by the throttle wrapper before
that handler runs, and the `/ws` 401 and gorilla's handshake rejections
never set it, so those replies leave an HTTP/1.1 keep-alive connection
open for another request (subject to Go's rules for unread request bodies
and to the 10 s read timeout, which also acts as the idle timeout because
`IdleTimeout` is unset). Derived from `server.go:129-173`, `287-302` and
Go `net/http` semantics; not probed live.

JSON-RPC errors never change the HTTP status: it is 200 whenever a JSON
body is written. Body framing of the 200 reply is Go `net/http` default
behaviour (a `Content-Length` header for small bodies, chunked transfer
encoding for larger ones; the connection is closed afterwards either way).

Websocket framing (gorilla v1.5.1 defaults; `server.go:116-119` configures
only `CheckOrigin`): no subprotocol, no compression, no origin
restriction, no message size limit, automatic pong replies to pings,
no server-initiated pings. Incoming text and binary messages are treated
alike (`server.go:340`); outgoing responses are always single text
messages (`server.go:489`).

Websocket protocol-level behaviour inherited from gorilla v1.5.1 (the
wallet sets no handlers and no limits, so every default below is
observable by a client; `websocket@v1.5.1/server.go:126-260`,
`conn.go:113-147`, `424-480`, `773-796`, `808-1009`, `1162-1200`):

1. **Handshake reply bytes.** The 101 response is written directly to the
   hijacked connection, not through `net/http`, so it carries no `Date`
   or other header. Exactly:
   `HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: <accept>\r\n\r\n`,
   `<accept>` = base64(SHA-1(key + `258EAFA5-E914-47DA-95CA-C5AB0DC85B11`)).
   `Sec-WebSocket-Protocol` and `Sec-WebSocket-Extensions` request headers
   are ignored (no subprotocol is ever selected; `permessage-deflate` is
   never negotiated).
2. **Handshake checks** compare header tokens case-insensitively and accept
   comma-separated token lists (`Connection: keep-alive, Upgrade`). The
   `Sec-WebSocket-Key` must be standard base64 decoding to exactly 16
   bytes.
3. **Bytes pipelined behind the handshake request** (data already buffered
   when the connection is hijacked): gorilla closes the TCP connection
   without any HTTP reply and returns `websocket: client sent data before
   handshake is complete`, which the wallet logs through the `Cannot
   websocket upgrade client` warning. This is the one path on which the
   server closes a `/ws` connection itself.
4. **Inbound frames.** Client frames must be masked; RSV1-3 must be zero;
   control frames must be final with a payload of at most 125 bytes;
   opcodes other than continuation (0), text (1), binary (2), close (8),
   ping (9), pong (10) are invalid; a data frame may not start while a
   fragmented message is open and a continuation may not arrive without
   one. On any violation gorilla sends a close frame with status 1002 and
   reason text listing the violations joined by `, ` (`RSV1 set`, `RSV2
   set`, `RSV3 set`, `len > 125 for control`, `FIN not set on control`,
   `data before FIN`, `continuation after FIN`, `bad opcode <n>`, `bad
   MASK`; truncated to 125 bytes of payload) and the read fails with
   `websocket: <reason>`, which ends the reader (5.4.2) with the receive
   warning.
5. **Fragmented messages** are reassembled; one JSON-RPC request is one
   complete message of any size (no read limit is set). Text payloads are
   not checked for UTF-8 validity by gorilla; they go straight to
   `json.Unmarshal`.
6. **Ping** from the client: a pong with the same application data is
   written from the reader goroutine with a 1 s write wait; a network
   error or an already-sent close is ignored. **Pong** from the client is
   ignored.
7. **Close** from the client: a status code that is not one of 1000, 1001,
   1002, 1003, 1007, 1008, 1009, 1010, 1011, 1012, 1013 or in 3000-4999
   is a protocol error (`bad close code <n>`), as is a non-UTF-8 reason
   (`invalid utf8 payload in close frame`); both are answered as in item
   4. Otherwise gorilla **echoes a close frame** carrying the same status
   code and an empty reason (an empty close payload is echoed as an empty
   close payload), with a 1 s write wait, and the read then fails with
   `*CloseError`, whose text is `websocket: close <code>` + a name for
   well-known codes (` (normal)` for 1000, ` (going away)` for 1001, `
   (no status)` for a payload-less close, …) + `: <reason>` when the reason
   is not empty. That text appears in the receive warning of 5.4.2.
8. **After any close frame has been written** (items 4 and 7) every later
   data write on that connection fails with `websocket: close sent`, so a
   reply that completes afterwards produces the warning `Failed websocket
   send to client <addr>: websocket: close sent` and ends the sender.
9. **Outbound data frames.** Each response is exactly one unfragmented,
   unmasked text frame (FIN set, opcode 1) whatever its size
   (`conn.go:775-786`). The read and write buffers are the 4096-byte
   `net/http` buffers reused from the hijacked connection; buffer sizes
   have no wire effect.
10. The server still does not close the TCP connection after a close
    handshake (5.4.5); the socket is released when the client closes its
    end.

### 4.12 Pipe (IPC) messages emitted by this subsystem

Framing (`ipc.go:20-29`, `70-109`): 1 byte protocol version (`0x01`), 1 byte
type length *n*, *n* bytes ASCII type, 4 bytes payload length (little
endian), payload. Messages are written only when `--pipetx` is set;
otherwise they are drained and discarded (`dcrwallet.go:101-105`).

| Type | When | Payload |
|---|---|---|
| `jsonrpclistener` | `--rpclistenerevents`; one per JSON-RPC listener, after `NewServer` returns, in listener order (`rpcserver.go:384-386`) | `net.Listener.Addr().String()`, e.g. `127.0.0.1:9110` or `[::1]:9110` |
| `grpclistener` | `--rpclistenerevents`; one per gRPC listener, from each listener's serving goroutine (`rpcserver.go:339-342`) | listener address string |
| `issuedclientcertificate` | `--issueclientcert`, before any listener is opened (`rpcserver.go:306-307`) | concatenation of three PEM blocks: client private key (`PRIVATE KEY`, PKCS#8), client certificate, CA certificate (`ipc.go:194-203`) |

### 4.13 TLS files

| File | Default path | Written by | Format | Mode |
|---|---|---|---|---|
| RPC certificate (`--rpccert`) | `<appdata>/rpc.cert` | `generateRPCKeyPair` | one PEM `CERTIFICATE` block | 0600 |
| RPC key (`--rpckey`) | `<appdata>/rpc.key` | `generateRPCKeyPair` when `writeKey` | ECDSA curves: PEM `EC PRIVATE KEY` (SEC 1); Ed25519: PEM `PRIVATE KEY` (PKCS#8) (`dcrd/certgen@v1.2.0/certgen.go:154-160`, `certgen_ed25519.go`) | 0600 |
| Client CA bundle (`--clientcafile`) | `<appdata>/clients.pem` | never written by the wallet; read only | concatenated PEM `CERTIFICATE` blocks | n/a |

Parent directories of the cert and key are created with mode 0700
(`rpcserver.go:74-83`). Path defaults and `--appdata` rebasing are
`config.go:76-80`, `454-461`, `974-979`.

---

## 5. Algorithms and state machines

### 5.1 Server start-up (`startRPCServers`, `rpcserver.go:248-395`)

Executed once, on the main goroutine. Without `--noinitialload` the wallet
has already been opened (and, where configured, unlocked) by this point
(`dcrwallet.go:194-230` precede the call at `dcrwallet.go:380`); with
`--noinitialload` no wallet is loaded yet. In both cases no network backend
is attached yet: syncing starts after the servers (`dcrwallet.go:414-440`).

1. If `--rpclistenerevents`, bind the two listener-event notifiers to the
   outgoing pipe channel; otherwise they are nil and `notify` is a no-op.
2. If `--noservertls`:
   1. Log (info, `DCRW` logger) `Server TLS is disabled.  Only JSON-RPC may
      be used` (two spaces after the full stop).
   2. JSON-RPC listeners will be plain `net.Listen`. No gRPC server is
      created and `--issueclientcert` and `--clientcafile` are ignored.
3. Otherwise (TLS enabled):
   1. `openRPCKeyPair()` (5.1.2); on error return it.
   2. Build `tls.Config{Certificates: [keyPair], MinVersion: TLS 1.2,
      ClientCAs: empty pool}`. Cipher suites, curves and TLS 1.3 are Go
      defaults.
   3. If the client CA file exists (stat errors other than not-exist count
      as "does not exist"), read it (read error is returned) and append its
      certificates to `ClientCAs`; if none parse, warn `No certificates
      added from CA file <path>`.
   4. If `--jsonrpcauthtype=clientcert`, set `ClientAuth =
      RequireAndVerifyClientCert` on this config.
   5. If `--issueclientcert`: generate a key pair on `--tlscurve`, build an
      ephemeral CA (5.1.4), issue one client certificate, add the CA
      certificate to `ClientCAs`, and emit the `issuedclientcertificate`
      pipe message.
   6. JSON-RPC listeners will be `tls.Listen(network, addr, tlsConfig)`.
   7. `clientCAsExist = clientCAsExist || IssueClientCert`.
   8. gRPC: if there are gRPC listen addresses and no client CAs, warn
      `gRPC server is configured with listeners, but no trusted client
      certificates exist (looked in <path>)` and do not start gRPC. If
      there are gRPC listen addresses and client CAs exist, clone the TLS
      config, force `RequireAndVerifyClientCert` on the clone, open plain
      TCP listeners with `makeListeners` (error `failed to create listeners
      for RPC server` if none could be opened), create and start the gRPC
      server (sibling spec).
4. JSON-RPC start decision, first matching branch wins:
   1. TLS enabled, at least one JSON-RPC listen address, auth type
      `clientcert`, and no client CAs: warn `JSON-RPC TLS server is
      configured with listeners and client cert auth, but no trusted client
      certificates exist (looked in <path>)`; the JSON-RPC server is not
      started.
   2. Auth type `basic` and (`--username` empty or `--password` empty):
      info `JSON-RPC server disabled (basic auth requires username and
      password, and client cert authentication is not enabled)`; not
      started.
   3. At least one JSON-RPC listen address: open listeners with
      `makeListeners(cfg.LegacyRPCListeners, jsonrpcListen)`; if none could
      be opened return `failed to create listeners for JSON-RPC server`.
      Build `jsonrpc.Options` (username/password only when auth type is
      `basic`, else both empty; `MaxPOSTClients = --rpcmaxclients`;
      `MaxWebsocketClients = --rpcmaxwebsockets`; mixing, VSP, dial and
      logger fields from config), call `jsonrpc.NewServer`, then emit one
      `jsonrpclistener` pipe message per listener.
5. If neither server was created return `no suitable RPC services can be
   started`. The caller logs `Unable to create RPC servers: <err>` and the
   process exits with an error (`dcrwallet.go:381-384`).

Configuration preconditions enforced earlier in `config.go` and relied on
here: listen addresses are normalised and default to every address
`localhost` resolves to on the network's JSON-RPC port (mainnet 9110,
testnet3 19110, simnet 19557; gRPC 9111/19111/19558;
`internal/netparams/params.go:24-43`, `config.go:870-907`);
`--nolegacyrpc` clears the JSON-RPC listeners; `--noservertls` is rejected
unless every listener host is `localhost`, `127.0.0.1` or `::1`
(`config.go:931-951`); `--jsonrpcauthtype` must be `basic` or `clientcert`
else `unknown JSON-RPC authtype "<v>"` (`config.go:1027-1033`).

#### 5.1.1 Configuration inputs and the `Options` mapping

Options read by `startRPCServers` and its helpers (long flag name is also
the `dcrwallet.conf` key; parsing and validation belong to the
configuration spec, the values below are what this subsystem relies on):

| Flag | Config field | Type / default | Use here |
|---|---|---|---|
| `--rpclisten` | `LegacyRPCListeners` | repeated string; default every `localhost` address on the network's JSON-RPC port | JSON-RPC listen addresses (normalised `host:port`) |
| `--nolegacyrpc` | `NoLegacyRPC` | bool | clears `LegacyRPCListeners` (`config.go:889-891`) |
| `--grpclisten` | `GRPCListeners` | repeated string; default every `localhost` address on the gRPC port | gRPC listen addresses |
| `--nogrpc` | `NoGRPC` | bool | clears `GRPCListeners` (`config.go:880-882`) |
| `--rpcmaxclients` | `LegacyRPCMaxClients` | int64, 10 | `Options.MaxPOSTClients` |
| `--rpcmaxwebsockets` | `LegacyRPCMaxWebsockets` | int64, 25 | `Options.MaxWebsocketClients` |
| `-u`, `--username` | `Username` | string, empty | Basic auth user; also the default dcrd RPC user (`config.go:986-988`) |
| `-P`, `--password` | `Password` | string, empty (masked in help output) | Basic auth password; also the default dcrd RPC password |
| `--jsonrpcauthtype` | `JSONRPCAuthType` | `basic` (default) or `clientcert` | auth mode |
| `--rpccert` | `RPCCert` | path, `<appdata>/rpc.cert` | server certificate |
| `--rpckey` | `RPCKey` | path, `<appdata>/rpc.key` | server key |
| `--clientcafile` | `ClientCAFile` | path, `<appdata>/clients.pem` | trusted client CAs for JSON-RPC `clientcert` auth and for gRPC |
| `--tlscurve` | `TLSCurve` | `P-256`, `P-384`, `P-521`, `Ed25519` (default); anything else is `unrecognized curve <v>` (`internal/cfgutil/curve.go:76-90`) | all key generation |
| `--onetimetlskey` | `OneTimeTLSKey` | bool | 5.1.2 |
| `--noservertls` | `DisableServerTLS` | bool | plain TCP JSON-RPC, no gRPC |
| `--rpclistenerevents` | `RPCListenerEvents` | bool | listener pipe messages |
| `--issueclientcert` | `IssueClientCert` | bool | ephemeral client certificate over the pipe |

Listener rules enforced by configuration loading that shape what this
subsystem sees: after normalisation the same `host:port` may not appear in
both listener lists unless the port is `0`; the error is ``Address
`<addr>` may not be used as a listener address for both RPC servers``
(`config.go:910-927`). Listener normalisation errors are reported on
stderr as `Invalid network address in legacy RPC listeners: <err>` and
`Invalid network address in RPC listeners: <err>` (`config.go:895-907`).
The three path options are passed through `cleanAndExpandPath`
(environment-variable and `~` expansion, `filepath.Clean`) but are **not
made absolute** (`config.go:201-233`, `976-980`); see Quirk Q32.

`jsonrpc.Options` as built at `rpcserver.go:364-382`:

| `Options` field | Source |
|---|---|
| `Username`, `Password` | `cfg.Username`, `cfg.Password` when `JSONRPCAuthType == "basic"`, otherwise both `""` |
| `MaxPOSTClients` | `cfg.LegacyRPCMaxClients` |
| `MaxWebsocketClients` | `cfg.LegacyRPCMaxWebsockets` |
| `MixingEnabled` | `cfg.MixingEnabled` |
| `MixAccount` | `cfg.mixedAccount` (derived, unexported config field) |
| `MixBranch` | `cfg.mixedBranch` (derived, unexported config field) |
| `MixChangeAccount` | `cfg.ChangeAccount` |
| `TicketSplitAccount` | `cfg.TicketSplitAccount` |
| `VSPHost` | `cfg.VSPOpts.URL` |
| `VSPPubKey` | `cfg.VSPOpts.PubKey` |
| `Dial` | `cfg.dial` (the proxy-aware dialer) |
| `Loggers` | `rpcLoggers{}` |

The thresholds are not validated: a zero or negative `--rpcmaxclients` /
`--rpcmaxwebsockets` makes every request on that route fail with 429
(`current-1 >= threshold` is true for the first client, `server.go:294`).

`TLSCurve.GenerateKeyPair(rand)` (`internal/cfgutil/curve.go:92-113`):
ECDSA curves use `ecdsa.GenerateKey`; Ed25519 reads a 32-byte seed from the
supplied reader and derives the key from it. `TLSCurve.CertGen`
(`curve.go:115-123`) selects the certgen function. Both return `unknown
curve ID` for an unrecognised internal curve value (unreachable through
the flag parser). The random source everywhere in `rpcserver.go` is
`dcrd/crypto/rand` (`rand.Reader()`, `rand.BigInt`).

#### 5.1.2 `openRPCKeyPair` (`rpcserver.go:42-65`)

Let `keyExists = !os.IsNotExist(stat(rpckey))` (any stat error other than
not-exist counts as "exists").

| `--onetimetlskey` | `keyExists` | Action |
|---|---|---|
| true | true | Error `one time TLS keys are enabled, but TLS key \`<rpckey>\` already exists` |
| true | false | Generate; write certificate only; key stays in memory |
| false | false | Generate; write certificate and key (overwrites an existing certificate) |
| false | true | `tls.LoadX509KeyPair(rpccert, rpckey)`; a missing certificate is an error |

#### 5.1.3 `generateRPCKeyPair` (`rpcserver.go:70-116`)

1. Log info `Generating TLS certificates...`.
2. `MkdirAll` the directories of the cert and key paths, mode 0700.
3. Generate with `cfg.TLSCurve.CertGen("dcrwallet autogenerated cert",
   now + 10*365*24h, nil)` (`internal/cfgutil/curve.go:115-123`), which
   calls `certgen.NewTLSCertPair` for P-256/P-384/P-521 or
   `certgen.NewEd25519TLSCertPair` for Ed25519 (the default,
   `curve.go:29`). Certificate contents (`dcrd/certgen@v1.2.0/certgen.go:40-166`):
   - serial: uniform random below 2^128;
   - subject: `O=dcrwallet autogenerated cert`, `CN=<hostname>` (IDNA
     ASCII form if the hostname is not ASCII);
   - `NotBefore = now - 24h`; `NotAfter = min(now + 10 years of 365 days,
     2049-12-31T23:59:59Z)`;
   - key usage: key encipherment, digital signature, cert sign; basic
     constraints valid with `CA:TRUE`;
   - DNS SANs: hostname, plus `localhost` if the hostname is not
     `localhost`; IP SANs: `127.0.0.1`, `::1`, then every local interface
     address not already present, in interface enumeration order;
   - self-signed with the generated key.
4. Parse the pair with `tls.X509KeyPair`.
5. Write the certificate file (0600). If `writeKey`, write the key file
   (0600); if that write fails, remove the certificate file (warn `Cannot
   remove written certificates: <err>` if removal fails) and return the
   write error.
6. Log info `Done generating TLS certificates`.

The certificate bytes are not reproducible (random key and serial, wall
clock, host interfaces); only the structure above is contractual.
`dcroxide:crates/dcroxide-certgen/src/certgen.rs` already implements both
generators.

#### 5.1.4 Ephemeral client CA and client certificate (`rpcserver.go:118-231`)

`generateAuthority(pub, priv)`:

1. `validUntil = min(now + 10*365*24h, endOfTime)`; error `valid until
   date <t> already elapsed` if it is before now.
2. Template: random 128-bit serial; subject `CN=dcrwallet`, `O=dcrwallet`,
   `OU=dcrwallet certificate authority`; `NotBefore = now - 24h`;
   `NotAfter = validUntil`; key usage cert sign; basic constraints valid,
   `CA:TRUE`.
3. Self-sign, PEM-encode (`CERTIFICATE`), re-parse.

`generateClientKeyPair(caPriv, ca)`:

1. New key pair on `--tlscurve`.
2. Private key: PKCS#8 DER in a PEM `PRIVATE KEY` block (for all curves,
   unlike the server key).
3. Certificate signed by the CA: random 128-bit serial; `NotBefore = now -
   24h`; `NotAfter = ca.NotAfter`; subject `CN=dcrwallet`, `O=dcrwallet`,
   `OU=dcrwallet client certificate`; no key usage, no extended key usage,
   no SANs.

Neither key nor certificate is written to disk; they exist only in the
pipe message. The CA private key is discarded after issuing the single
client certificate.

#### 5.1.5 `makeListeners` (`rpcserver.go:455-512`)

1. For each normalised `host:port`: split; on failure log error
   `` `<addr>` is not a normalized listener address `` and skip.
2. Empty host (or `*` on plan9): add the address to both the IPv4 and IPv6
   lists.
3. Otherwise strip any `%zone` suffix and parse the host as an IP. Not an
   IP: warn `` `<host>` is not a valid IP address `` and skip (host names
   are never resolved here). IPv4 goes to the IPv4 list, anything else to
   the IPv6 list.
4. Listen on every IPv4 address with network `tcp4`, then every IPv6
   address with `tcp6`, in input order. A failed listen logs warn `Can't
   listen on <addr>: <err>` and is skipped.
5. Return the successfully opened listeners (IPv4 first, then IPv6).

For JSON-RPC with TLS the `listen` function is `tls.Listen`, so the
listener yields `*tls.Conn` connections. `net/http` detects this and runs
the TLS handshake explicitly on the connection goroutine right after
accept, under its own read and write deadline equal to the smallest
non-zero server timeout, here `ReadTimeout` = 10 s (Go `net/http/server.go`,
`conn.serve` and `Server.tlsHandshakeTimeout`; read in the Go 1.27 source).

### 5.2 Authentication

State at construction (`server.go:124-127`): if both `Username` and
`Password` are non-empty, `authsha = SHA-256("Basic " +
base64std(Username + ":" + Password))` (standard alphabet, with padding);
otherwise `authsha` is nil and Basic auth is disabled.

`checkAuthHeader(r)` (`server.go:262-277`):

1. `authsha == nil`: return nil (authenticated).
2. No `Authorization` header: return `errNoAuth`.
3. Compare, in constant time, `SHA-256(first Authorization header value)`
   with `authsha`. Mismatch: return `invalid Authorization header`.

The comparison is over the entire header value, so the client must send
exactly `Basic <base64>`: scheme spelled `Basic`, one space, canonical
padded standard base64 of `user:pass`. `basic ...`, extra whitespace or
unpadded base64 are rejected. Only the first `Authorization` header is
considered.

Resulting policy:

| Mode | `/` (POST) | `/ws` |
|---|---|---|
| `basic` | Header required and must match; else 401 | Matching header: pre-authenticated. No header: upgraded unauthenticated; first message must be a valid `authenticate`. Wrong header: 401, no upgrade. |
| `clientcert` (TLS on) | No HTTP auth; the TLS layer requires a client certificate chaining to `ClientCAs` | Same; connection is pre-authenticated |
| `clientcert` with `--noservertls` | **No authentication at all** (Quirk Q5) | same |

Every 401 logs warn `Failed authentication attempt from client
<remote-addr>` (`server.go:136`, `157`).

Websocket `authenticate` validation (`invalidAuth`, `server.go:318-336`):
parse params as `AuthenticateCmd` (exactly two parameters); invalid if
parsing fails, if Basic auth is disabled, or if `SHA-256("Basic " +
base64std(username + ":" + passphrase))` differs from `authsha`
(constant-time compare).

### 5.3 HTTP POST request lifecycle (`server.go:129-144`, `534-615`)

1. Throttle (`server.go:287-302`): atomically increment the POST counter;
   decrement on return. If the pre-increment value is `>= MaxPOSTClients`,
   log warn `Reached threshold of <N> concurrent active clients`, reply
   429 and stop. The check precedes authentication.
2. Set response headers `Connection: close` and `Content-Type:
   application/json`; mark the request to close the connection.
3. `checkAuthHeader`; on any error log and reply 401 (4.11).
4. Add to the server wait group for the duration of the request. Read the
   whole body through `http.MaxBytesReader(4 MiB)`. Over-limit: warn
   `Request from client <addr> exceeds maximum size`, reply 413. Other
   read error: warn `Failed to read request from client <addr>`, reply 500
   `Request read failed`.
5. `json.Unmarshal` the body into `dcrjson.Request`. On error reply (HTTP
   200) `MarshalResponse(req.Jsonrpc, req.ID, nil, ErrRPCInvalidRequest)`
   using whatever was decoded (4.1 rule 3). If that marshal fails (id of
   unsupported type): error log `Unable to marshal response to client
   <addr>: <err>` and 500. A failed write logs warn `Cannot write invalid
   request response to client <addr>: <err>`. Nothing is logged for the
   decode failure itself.
6. Open a runtime trace task named after the method (no observable
   effect).
7. Switch on `req.Method`:
   - `authenticate`: warn `Invalid RPC method authenticate invoked by HTTP
     POST client <addr>`; return without writing (empty 200).
   - `stop`: debug `RPC method stop invoked by <addr>`; result is the
     string `dcrwallet stopping`; remember to shut down.
   - otherwise: debug `RPC method "<method>" invoked by <addr>`
     (`server.go:250`; `%q` quoting), then run the dispatch closure
     synchronously (5.5).
8. `MarshalResponse(req.Jsonrpc, req.ID, result, rpcErr)`.
9. If marshalling fails: error log `Unable to marshal response to client
   <addr>: <err>`, reply 500 `500 Internal Server Error`. The method has
   already executed.
10. Write the body. A failed write logs warn `Failed to write response to
    client <addr>: <err>`.
11. If the method was `stop`, send on the shutdown channel (blocking, cap
    1) after the response has been written to the HTTP layer.

One request per connection: the connection is closed after the reply.
Request context: derived from the process context (`BaseContext`,
`server.go:103-105`) through `net/http`'s per-connection and per-request
contexts, plus the remote address value; it is cancelled when the client
disconnects, when the handler returns, on process shutdown, **and when the
connection read deadline fires** (section 7.3, Open question O1).

### 5.4 Websocket lifecycle

#### 5.4.1 Upgrade (`server.go:146-173`)

1. Throttle with `MaxWebsocketClients`; the counter stays incremented for
   as long as the handler is blocked in step 6, i.e. for the life of the
   client session.
2. `checkAuthHeader`: nil gives `authenticated = true`; `errNoAuth` gives
   `authenticated = false`; any other error logs warn `Failed
   authentication attempt from client <addr>` and replies 401.
3. `upgrader.Upgrade(w, r, nil)`. On failure gorilla has already replied;
   log warn `Cannot websocket upgrade client <addr>: <err>` and return.
4. Build the session context: request context + remote address value +
   cancel function.
5. Log info `New websocket client <addr>`. Clear the read deadline
   inherited from the HTTP server (warn `Cannot remove read deadline:
   <err>` on failure).
6. Start the reader goroutine (not tracked by the server wait group), add
   2 to the server wait group, start the responder and sender goroutines,
   then block until the session's `quit` channel closes.

#### 5.4.2 Reader (`websocketClientRead`, `server.go:338-352`)

Loop: read one message (text or binary). On success send the bytes on the
unbuffered `allRequests` channel (blocks until the responder takes it). On
any read error: unless the error is `io.EOF` or `io.ErrUnexpectedEOF`, log
warn `Websocket receive failed from client <addr>: <err>` (a normal close
frame is reported by gorilla as a `CloseError` and therefore *is* logged);
close `allRequests`; cancel the session context; exit.

#### 5.4.3 Responder (`websocketClientRespond`, `server.go:354-471`)

Loop selecting on `allRequests` and the server `quit` channel. "End the
session" below means leaving the loop.

1. `allRequests` closed: end the session. Server `quit` closed: end the
   session.
2. `json.Unmarshal` into `dcrjson.Request`. On error log warn `Failed
   unmarshal of JSON-RPC request object from client <addr>`; if the client
   is not authenticated end the session without replying; otherwise send
   the `makeResponse(req.ID, nil, ErrRPCInvalidRequest)` bytes (4.2) and
   continue.
3. `method == "authenticate"`: debug `RPC method authenticate invoked by
   <addr>`. If already authenticated: warn `Multiple authentication
   attempts from <addr>`, end the session, no reply. If `invalidAuth`:
   warn `Failed authentication attempt from <addr>`, end the session, no
   reply. Otherwise mark authenticated, send
   `{"jsonrpc":"","result":null,"error":null,"id":<id>}`, continue.
4. Not authenticated: end the session without replying or logging.
5. `method == "stop"`: debug `RPC method stop invoked by <addr>`; send the
   `dcrwallet stopping.` reply (4.2); send on the shutdown channel; end
   the session.
6. Otherwise: copy the request, open a trace task, log debug `RPC method
   "<method>" invoked by <addr>`, build the dispatch closure, and run it
   in a **new goroutine** tracked by the session wait group. The goroutine
   executes the closure, marshals with `MarshalResponse(req.Jsonrpc,
   req.ID, result, err)` and sends the bytes; on a marshal failure it logs
   error `Unable to marshal response to client <addr>: <err>` and sends
   nothing. The responder immediately continues with the next message.

After the loop: wait for all in-flight request goroutines, close the
`responses` channel, release one server wait-group count.

Consequences: requests on one websocket run concurrently and replies are
emitted in completion order, not request order; clients must correlate by
`id`. There is no per-connection cap on in-flight requests.

#### 5.4.4 Sender (`websocketClientSend`, `server.go:473-503`)

Loop selecting on `responses` and the server `quit` channel. For each
response: set a write deadline of now + 2 s (warn `Cannot set write
deadline on client <addr>: <err>` on failure) and write one text message;
on write failure warn `Failed websocket send to client <addr>: <err>` and
leave the loop. Leave the loop when `responses` is closed or the server
quits. Then close the session `quit` channel and log info `Disconnected
websocket client <addr>`.

#### 5.4.5 Session termination

When the session `quit` channel closes, the HTTP handler returns, which
decrements the throttle counter and cancels the request context (hence the
session context and every in-flight request). **The server never closes
the websocket's TCP connection and never initiates a close frame**
(`server.go` contains no `conn.Close()` and no close-message write); see
Quirk Q3. The only close frames a client can receive are the ones gorilla
writes on its own from inside the read path: the echo of a
client-initiated close and the status-1002 close after a protocol
violation (4.11, websocket items 4 and 7). If the client is
still connected the reader goroutine remains blocked in a read (or, after
one more client message, blocked forever on `allRequests`). That describes
sessions ended by the responder. When the session ends because the sender
failed while the responder is still running, the responder keeps consuming
and dispatching requests; see Quirk Q28.

### 5.5 Dispatch (`lazyApplyHandler`, `methods.go:229-279`)

Input: server, request context, decoded request. Output: a closure
returning `(result, *RPCError)`.

#### 5.5.1 Method not in the table: dcrd passthrough

1. `walletLoader.NetworkBackend()`; if there is no wallet or no backend:
   `-9 "disconnected from consensus RPC"`.
2. If the backend is not the dcrd RPC syncer (SPV, offline): `-9 "RPC
   passthrough requires dcrd RPC synchronization"`.
3. Call dcrd over the wallet's own dcrd websocket connection with the same
   method name and the request's raw parameters, positionally, unmodified.
4. If the request context is done after the call: warn `Canceled RPC
   method <method> invoked by <addr>: <err>` (here `<err>` is the error
   returned by the dcrd call, not `ctx.Err()`; wsrpc v2.3.8 `rpc.go:434-438`
   substitutes the context error whenever the context is done, so the two
   coincide) and reply `-1` with message
   `context canceled` or `context deadline exceeded` (`ctx.Err()` text).
   This check does not look at the call's own outcome: a dcrd call that
   completed successfully just before the context was cancelled is still
   answered with `-1` and its result is discarded (`methods.go:248-254`).
5. If the call failed: `convertError(err)`. A dcrd JSON-RPC error
   (`*wsrpc.Error`) passes through with the same `code` and `message`
   (`errors.go:20-24`); its `data` member is dropped.
6. Otherwise the result is dcrd's raw `result` JSON.

No authentication, parameter or flag checks are applied: websocket-only
dcrd methods and notification registrations (`notifyblocks`, `session`,
`rescan`, …) are forwarded and act on the wallet's own dcrd connection
(`methods.go:2884-2886` records this as a TODO for `help`).

#### 5.5.2 Method in the table

1. `dcrjson.ParseParams(types.Method(method), params)`. Any error:
   `ErrRPCInvalidRequest` (`-32600`, `Invalid request`). This is also the
   outcome for the eight `noHelp` entries, which have no registration.
2. Call the handler with the server, the context and the parsed command.
3. On return, if the context is done log warn `Canceled RPC method
   <method> invoked by <addr>: <err>` (this does not alter the reply).
4. Handler error: `convertError(err)` (section 8.2). Otherwise the result
   value is returned and marshalled by the transport.

A handler returning `(nil, nil)` yields `"result":null,"error":null`.

### 5.6 `help` (`methods.go:2882-2937`)

Works without a wallet. Let `rpc` be the dcrd RPC client if the backend is
a dcrd RPC syncer, else nil.

1. `command` absent, `null` or `""`:
   1. `usages = requestUsages`.
   2. If `rpc != nil`: call dcrd `help` with no parameters, expecting a
      string; a call error fails the request (`convertError`). If the
      string is non-empty the result is
      `"Chain server usage:\n\n" + dcrdUsage + "\n\n" + "Wallet server
      usage (overrides chain requests):\n\n" + requestUsages`.
   3. Return `usages`.
2. Otherwise, under `helpDescsMu`, build the en_US map on first use and
   look up `command`. Found: return the text.
3. Not found and `rpc != nil`: call dcrd `help` with `command`; a call
   error fails the request (dcrd's error code and message pass through).
   A non-empty string is returned as is.
4. Otherwise: `-8` with message `no help for method "<command>"` (Go `%q`
   quoting).

The locale is hard-coded to `en_US` (`methods.go:2914-2917`).

`helpDescsMu` is taken at step 2 and released only when the handler
returns (`methods.go:2910-2911`, deferred unlock), so the dcrd call of
step 3 runs **with the mutex held**: `help <unknown>` requests are
serialised behind one another's dcrd round trips, and any `help <command>`
request (known or not) waits for an in-flight unknown-command lookup. The
no-argument form (step 1) never takes the mutex. The built map is kept for
the life of the process.

### 5.7 `debuglevel`, `getcurrentnet`

`debuglevel` (`methods.go:798-815`): `levelspec == "show"` returns the
string `Supported subsystems ` followed by Go's `%v` rendering of the
sorted subsystem list (the keys of `subsystemLoggers`, `log.go:47-61`,
sorted bytewise): `Supported subsystems [AMGR CMGR DCRW GRPC LODR MIXC
MIXP PEER RPCS SYNC TKBY VSPC WLLT]`. Otherwise
`Loggers.SetLevels(levelspec)`; on error `-8` with message `invalid debug
level <levelspec>: <err>`; on success the string `Done.`. The subsystem
set and the level-spec grammar belong to the logging/config spec.

`getcurrentnet` (`methods.go:1764-1766`): returns the network's wire magic
as a JSON number (`activeNet.Net`, a `uint32`).

### 5.8 Shutdown (`Stop`, `server.go:216-240`; `dcrwallet.go:401-411`)

1. On process shutdown the main goroutine logs warn `Stopping JSON-RPC
   server...`, calls `Stop()`, then logs info `JSON-RPC server shutdown`.
   The gRPC server is stopped after it (deferred earlier, so it runs
   later): warn `Stopping gRPC server...`, `Stop()`, info `gRPC server
   shutdown`.
2. `Stop()` under `quitMtx`: if `quit` is already closed, return. Close
   each listener (error log `` Cannot close listener `<addr>`: <err> ``),
   close `quit`, release the mutex, wait for the wait group (listener
   goroutines, POST requests in progress, websocket responder and sender
   goroutines).
3. `http.Server.Shutdown`/`Close` is never called: idle or in-flight POST
   connections are not closed by `Stop`; in-flight handlers are expected
   to end because the process context (their base context) is already
   cancelled.

A `stop` RPC only signals: `dcrwallet.go:402-406` loops over the shutdown
channel and calls `requestShutdown()` for each value.

### 5.9 Help generation (build time; `genrpcserverhelp.go`, dcrjson `help.go`, `cmdinfo.go`)

Run by `go generate` from `internal/rpc/jsonrpc`
(`methods.go:2872-2873`: `go run ../../rpchelp/genrpcserverhelp.go
jsonrpc`, then `gofmt -w rpcserverhelp.go`). Output file layout is 4.10.1.

One-line usage (`cmdinfo.go:132-220`): method name; then each required
field's usage separated by spaces; then, if any, ` (` + optional field
usages separated by spaces + `)`. Field usage: a `jsonrpcusage` tag
verbatim if present; string: `"name"` (with default: `name="default"`);
array/slice: `["singular",...]` for strings, `[{...},...]` for structs,
`[singular,...]` otherwise, where the singular form strips a trailing
`ies`→`y`, `es`, or `s`; struct: `{"name":value,...}` using lower-cased Go
field names (not JSON tags) with value `n` (integer), `n.nnn` (float),
`"value"` (string), nested forms, or the bare field name for anything else
(pointers included); other kinds: `name` (with default: `name=<default>`).

Full help (`help.go:397-442`):

```
<usage>\n
\n
<synopsis>\n
\n
Arguments:\n
<argument table | "None\n">
\n
Result:\n            (or one "Result (<condition>):" block per result type)
<result text>\n
```

Argument rows are `N.<TAB>name<TAB>(type, required|optional[,
default=V])<TAB>description`, followed by an expanded object example for
struct, map and array-of-struct arguments; result objects are rendered one
member per line as `"name": example,<TAB>(type)<TAB>description` with one
space of indentation per nesting level. Both blocks are aligned with a Go
`text/tabwriter` configured `(minwidth 0, tabwidth 4, padding 1, padchar
' ', flags 0)` (`help.go:270`, `386`), which is what produces the trailing
spaces visible on some lines of the generated text (for example after a
closing `}`). Missing description keys make generation fail with
`ErrMissingDescription`.

Because of the tabwriter alignment and trailing-space subtleties, embed
the generated strings; regenerate only as a cross-check.

#### 5.9.1 Exact generation rules (`dcrd/dcrjson/v4@v4.2.0/help.go`)

Needed only for the regeneration cross-check. `xT(key)` is the description
lookup (caller map, then `baseHelpDescs`, else the key itself is
substituted and remembered as missing). `T` below is a Go type after
removing one pointer level where stated.

`GenerateHelp(method, descs, resultTypes...)` (`help.go:508-564`):

1. Unregistered method: `ErrUnregisteredMethod`.
2. Each non-nil result type must be a pointer (`result #<i> (<kind>) is not
   a pointer`) to a numeric, string, struct, array, slice, bool or map
   (`result #<i> (<kind>) is not an allowed type`); both `ErrInvalidType`,
   `<i>` 0-based.
3. Build the text with `methodHelp`. If any lookup missed, return the text
   **and** `ErrMissingDescription` whose description is the last missing
   key.

JSON type name of `T` (`reflectTypeToJSONType`, `help.go:50-75`): any
integer or float kind `numeric`; string `string`; bool `boolean`;
array/slice `array of ` + element type name; struct or map `object`;
anything else (including a pointer that was not removed by the caller)
`value`.

Example lines of `T` at indent level `L` under description key `K`
(`reflectTypeToJSONExample`, `help.go:144-249`; one leading pointer is
removed first). Returns lines and a "complex" flag:

| Kind | Lines | Complex |
|---|---|---|
| float32/64 | `n.nnn` | no |
| other numeric | `n` | no |
| string | `"value"` | no |
| bool | `true\|false` | no |
| struct | the member lines of `resultStructHelp(T, L+1)`; at `L == 0` preceded by a line `{`; followed by a closing line of `L` spaces + `}` (+ `,` when `L > 0`) + two TAB characters | yes |
| array/slice of a non-complex element | `[<element example>,...]` | no |
| array/slice of a complex element | the element's lines with, at `L == 0`, the first line replaced by `[{` and the last by `},...]`; at `L > 0` only the last line replaced by `L` spaces + `},...],` + two TABs | yes |
| map | at `L == 0` a line `{`; then `L+1` spaces + `"<xT(K--key)>": <xT(K--value)>, (<type name>) <xT(K--desc)>` (the key through `%q`, single spaces, no TABs); then `L+1` spaces + `...`; then `L` spaces + `}` | yes |
| anything else | `unknown` | no |

Struct member lines (`resultStructHelp`, `help.go:83-136`), for each field
in declaration order at indent `I` spaces: the member name is the part of
the `json` tag before the first comma, or the lower-cased Go field name
when there is no tag; the field type has one pointer level removed; the
description key is `<lower-cased Go type name>-<member name>`.

- Non-complex field: `<I spaces>"<name>": <example>,<TAB>(<type name>)<TAB><xT(key)>`.
- Complex field: `<I spaces>"<name>": <brace><TAB>(<type name>)<TAB><xT(key)>`
  with `<brace>` `[{` for arrays/slices and `{` otherwise, followed by the
  field's example lines. (For a map-typed struct member this would look up
  `<key>--key` etc.; no type used by the wallet has one.)

Result or argument block text (`resultTypeHelp`, `help.go:253-280`):
non-complex: `<example> (<type name>) <xT(K)>` on one line; complex: the
example lines joined with `\n` (no newline after the last) and passed
through the tabwriter, which turns the TAB-separated cells into
space-aligned columns and leaves the trailing spaces produced by the
two-TAB closing lines.

Argument table (`argHelp`, `help.go:329-392`): empty string for a command
with no fields. Otherwise, per field *i* (1-based), the row
`<i>.<TAB><lowerfield><TAB>(<details>)<TAB><xT(method-lowerfield)>` where
`<details>` (`argTypeHelp`, `help.go:286-326`) is the JSON type name of the
de-pointered field, then `, optional` or `, required`, then for an optional
field with a default `, default=<V>` (`<V>` is the default formatted with
`%v`, wrapped in double quotes when it is a string). After the row:

- de-pointered field is a struct: append the result block for it under key
  `<method>-<lowerfield>` (member keys are `<lowertypename>-<json name>`);
- a map: append the result block under key `<method>-<lowerfield>` (keys
  `--key`, `--value`, `--desc`);
- an array or slice: append the result block **only if `Field.Type.Elem()`
  is a struct**, evaluated on the declared field type, not the
  de-pointered one. For a required `[]Struct` field this is the element
  type and the block is emitted; for an optional `*[]Struct` field the
  element of the pointer is the slice itself, so nothing is emitted (Quirk
  Q33).

All rows and blocks are written, each followed by `\n`, through one
tabwriter.

Method text (`methodHelp`, `help.go:397-442`):

1. `<one-line usage>\n\n<xT(method--synopsis)>\n`.
2. `\nArguments:\n<argument table>` or, with no fields,
   `\nArguments:\nNone\n`.
3. Result text *i* (0-based) for each entry of `resultTypes`: `Nothing`
   for an untyped nil entry, else the result block of the pointee type
   under key `<method>--result<i>`.
4. More than one result: for each, `\nResult (<xT(method--condition<i>)>):\n<text>\n`.
   Exactly one: `\nResult:\n<text>\n`. None (the `nil` slice used for the
   "returns nothing" methods): `\nResult:\nNothing\n`.

Go `text/tabwriter` behaviour relied on (`Init(out, 0, 4, 1, ' ', 0)`): a
cell is text terminated by a TAB; text after the last TAB of a line is not
a cell and is not padded; a column block is a run of consecutive lines
that all have a cell in that column, and its width is the widest cell plus
one padding space; widths are measured in runes.
`dcroxide:crates/dcroxide-dcrjson/src/help.rs` carries a port; its
`TabWriter` lives in `dcroxide:crates/dcroxide-dcrjson/src/tabwriter.rs`.

---

## 6. Constants, limits and defaults

| Name | Value | Source | What depends on it |
|---|---|---|---|
| `jsonrpcSemverString` / `Major` / `Minor` / `Patch` | `"11.0.0"` / 11 / 0 / 0 | `internal/rpc/jsonrpc/methods.go:55-60` | `version` result key `dcrwalletjsonrpcapi`; clients gate features on the major. |
| `rpcAuthTimeoutSeconds` | 10 | `server.go:95` | `http.Server.ReadTimeout`: the TLS handshake must complete within 10 s of accept, and the request header and body must arrive within a further 10 s counted from the start of request reading; also bounds POST handler context lifetime (7.3). |
| `maxRequestSize` | 4,194,304 bytes (4 MiB) | `server.go:531` | POST body cap; 413 above it. No cap on websocket message size. |
| websocket write deadline | 2 s per response | `server.go:475` | A client that does not read for 2 s loses the session. |
| `requestShutdownChan` capacity | 1 | `server.go:121` | `stop` handling blocks if a previous stop has not been consumed. |
| Basic auth realm | `dcrwallet RPC` | `server.go:87` | `WWW-Authenticate` header. |
| Stop result strings | `dcrwallet stopping` (POST), `dcrwallet stopping.` (websocket) | `server.go:593`, `430` | Client-visible. |
| `defaultRPCMaxClients` (`--rpcmaxclients`) | 10 | `config.go:48`, `364` | `MaxPOSTClients` threshold. |
| `defaultRPCMaxWebsockets` (`--rpcmaxwebsockets`) | 25 | `config.go:49`, `365` | `MaxWebsocketClients` threshold. |
| `defaultAuthType` (`--jsonrpcauthtype`) | `basic`; alternatives `clientcert` | `config.go:36-37`, `50`, `366` | Auth mode. |
| JSON-RPC default ports | mainnet 9110, testnet3 19110, simnet 19557 | `internal/netparams/params.go:24`, `33`, `42` | Default `--rpclisten`. |
| gRPC default ports | mainnet 9111, testnet3 19111, simnet 19558 | `params.go:25`, `34`, `43` | Shared start-up path. |
| Default cert / key / client CA paths | `<appdata>/rpc.cert`, `rpc.key`, `clients.pem` | `config.go:76-80`, `454-461` | TLS files. |
| Default TLS curve (`--tlscurve`) | `Ed25519`; accepted names `P-256`, `P-384`, `P-521`, `Ed25519` | `internal/cfgutil/curve.go:29`, `76-90` | Server key, client CA and client key generation. |
| Certificate organisation | `dcrwallet autogenerated cert` | `rpcserver.go:86` | Generated `rpc.cert` subject. |
| Certificate validity | now - 24 h to now + 3650 days, capped at 2049-12-31T23:59:59Z | `rpcserver.go:87`, `125`; `dcrd/certgen@v1.2.0/certgen.go:52-55`, `130-131` | Generated certificates. |
| Serial number range | `[0, 2^128)` | `rpcserver.go:118-122`; `certgen.go:57-58` | Generated certificates. |
| Cert/key file mode, directory mode | 0600, 0700 | `rpcserver.go:76-103` | Files on disk. |
| TLS minimum version | TLS 1.2 | `rpcserver.go:274` | Client compatibility. |
| Client CA / client cert subjects | `CN=dcrwallet, O=dcrwallet, OU=dcrwallet certificate authority` / `OU=dcrwallet client certificate` | `rpcserver.go:145-149`, `199-203` | `--issueclientcert`. |
| Context key for peer address | `contextKey("remote-addr")`, fallback `<unknown>` | `context.go:12-18` | Log text. |
| Help locale | `en_US` (`EnUS`) | `internal/rpchelp/methods.go:141`; `methods.go:2917` | Help text selection. |
| tabwriter parameters | minwidth 0, tabwidth 4, padding 1, pad `' '`, flags 0 | `dcrd/dcrjson/v4@v4.2.0/help.go:270`, `386` | Help text alignment. |
| `UFWebsocketOnly`, `UFNotification` | `1<<1`, `1<<2` (bit 0 retired) | `dcrd/dcrjson/v4@v4.2.0/register.go:22-39` | Registration flags; `authenticate` carries `UFWebsocketOnly`, though the server never consults the flag. |
| Address marshaler size hint | `2 + n*(3+35)` bytes | `marshaling.go:21` | Allocation only. |
| `sstxCommitmentString`, `scriptVersionAssumed` | `"sstxcommitment"`, 0 | `methods.go:62-70` | Used by verbose transaction results (sibling specs). |
| Subsystem tag | `RPCS` (JSON-RPC), `GRPC`, `DCRW` (start-up messages) | `log.go:27`, `54-55` | Log lines. |

---

## 7. Concurrency and lifecycle

### 7.1 Goroutines

| Goroutine | Started | Ends | In `Server.wg` |
|---|---|---|---|
| Listener serve loop, one per listener | `serve` (`server.go:204-212`), from `NewServer` | `listener.Close()` in `Stop` makes `Serve` return; logs trace `Finished serving RPC: <err>` | yes |
| `net/http` connection goroutine, one per accepted connection | `http.Server.Serve` | connection end | no |
| POST request | runs on the connection goroutine | handler return | yes (added after auth succeeds, `server.go:141`) |
| Websocket reader | `websocketClientRPC` (`server.go:520`) | read error only | no (deliberately, `server.go:516-519`) |
| Websocket responder | `server.go:523` | client disconnect, auth failure, `stop`, or server quit; then waits for in-flight requests | yes |
| Websocket sender | `server.go:524` | `responses` closed, write failure, or server quit | yes |
| Websocket request, one per request | `server.go:448` | handler return and send (or send abandoned when `quit` closes) | no; in the per-session `wsc.wg` |
| Shutdown-request relay | `dcrwallet.go:402` | never (process exit) | no |
| gRPC listener serve loop, one per gRPC listener | `rpcserver.go:339-345` | `grpc.Server.Stop()`; logs trace `Finished serving gRPC: <err>` | no (untracked) |

Each gRPC serve goroutine first emits its `grpclistener` pipe message, then
logs info `gRPC server listening on <addr>`, then serves; the pipe-message
order across gRPC listeners is therefore scheduling-dependent, unlike the
`jsonrpclistener` messages, which are sent synchronously in listener order
after `NewServer` returns. The JSON-RPC `Listening on <addr>` log lines are
likewise emitted from the per-listener goroutines in no fixed order
(`server.go:206-208`).

The websocket upgrade handler itself runs on the `net/http` connection
goroutine and blocks until the session ends.

### 7.2 Channels and locks

| Object | Type | Use |
|---|---|---|
| `Server.quit` | `chan struct{}` | Closed once by `Stop`; observed by websocket responder and sender loops. POST handlers do not observe it. |
| `Server.quitMtx` | mutex | Makes `Stop` idempotent. |
| `Server.requestShutdownChan` | `chan struct{}`, cap 1 | One value per `stop` request. |
| `websocketClient.allRequests` | unbuffered `chan []byte` | Reader to responder; closed by the reader on read error. |
| `websocketClient.responses` | unbuffered `chan []byte` | Request goroutines and responder to sender; closed by the responder after all in-flight requests finish. |
| `websocketClient.quit` | `chan struct{}` | Closed by the sender on exit; unblocks pending `send` calls and the upgrade handler. |
| `helpDescsMu` | mutex | Guards lazy construction of the help map. |
| throttle counters | `atomic.Int64`, one per route | Concurrency caps. |
| dcrjson `registerLock` | RWMutex | Registry reads during `ParseParams`. |

There is no lock ordering concern inside this subsystem: no code path
holds two of these at once. Handlers acquire wallet locks on their own.

Ordering guarantees visible to clients:

- POST: strictly one request and one response per connection.
- Websocket: requests are *started* in arrival order but run concurrently;
  responses are written one at a time by the single sender in completion
  order. `authenticate` and `stop` are handled inline by the responder, so
  their replies are queued in order relative to each other but may
  interleave with replies of earlier, still-running requests.
- A websocket `stop` reply is handed to the sender before the shutdown
  signal is raised; a POST `stop` reply is written before the signal is
  raised.

### 7.3 Timeouts, deadlines and cancellation

- `http.Server.ReadTimeout = 10 s` is the only HTTP timeout set
  (`server.go:109`). No `ReadHeaderTimeout`, `WriteTimeout`, `IdleTimeout`
  or `MaxHeaderBytes` (Go defaults: header timeout falls back to
  `ReadTimeout`; 1 MiB header limit; no write timeout).
- Websocket connections have their read deadline cleared after the upgrade
  (`server.go:512`; gorilla also clears both deadlines on hijack) and get a
  2 s write deadline per outgoing message.
- **POST handler context and the read deadline.** `net/http` leaves the
  connection read deadline at (request start + `ReadTimeout`) while the
  handler runs and performs a background read on the connection to detect
  client disconnects. When that deadline fires the background read fails
  and `net/http` cancels the connection context (Go `net/http/server.go`,
  `connReader.backgroundRead` → `handleReadErrorLocked` → `cancelCtx`).
  By this analysis every HTTP POST handler still running about 10 s after
  the server began reading the request sees its context cancelled; the
  warn line `Canceled RPC method <m> invoked by <addr>: context canceled`
  (`methods.go:269-271`) is the observable trace, and the reply is whatever
  the handler returns once cancelled (typically code `-4` with a `context
  canceled` message, or `-1` on the passthrough path). Websocket requests
  are not affected. This is derived from the Go standard library source,
  not from a live run: see Open question O1.
- Request contexts are also cancelled by client disconnect (POST: detected
  by the same background read; websocket: reader error calls `cancel`), by
  handler/session return, and by process shutdown (base context).
- No handler-level timeout exists; a websocket request may run
  indefinitely.

### 7.4 Start-up and shutdown ordering

Start: config → loader → wallet open (skipped with `--noinitialload`,
`dcrwallet.go:194-230`) → `startRPCServers` (TLS material, gRPC listeners
and server, JSON-RPC listeners and server, pipe notifications) → network
sync start (`dcrwallet.go:414-440`). With `--noinitialload` JSON-RPC accepts
requests before a wallet is loaded; such requests fail with
`errUnloadedWallet` unless the method needs no wallet. In every mode
requests can arrive before a network backend is attached.

Stop: JSON-RPC `Stop()` then gRPC `Stop()` (5.8). Listener close order is
the listener slice order (IPv4 listeners first).

---

## 8. Errors

### 8.1 JSON-RPC error codes (`dcrd/dcrjson/v4@v4.2.0/jsonrpcerr.go`)

Codes the wallet's own code can emit are marked **W**. Any code can also
arrive from dcrd through passthrough.

| Code | dcrjson name(s) | Standard message (for the predefined `*RPCError` values) | W |
|---|---|---|---|
| -32700 | `ErrRPCParse` | `Parse error` | |
| -32600 | `ErrRPCInvalidRequest` | `Invalid request` | **W** undecodable request, any parameter parsing failure, the 8 dead table entries; code reused with a custom message by `importcfiltersv2` (`methods.go:2000`) |
| -32601 | `ErrRPCMethodNotFound` | `Method not found` | never produced by the wallet (unknown methods are forwarded) |
| -32602 | `ErrRPCInvalidParams`, `ErrRPCRawTxString` | `Invalid parameters` | **W** (one handler use) |
| -32603 | `ErrRPCInternal` | `Internal error` | **W** code only, with handler-specific messages; also wallet error kind `Bug` |
| -1 | `ErrRPCMisc`, `ErrRPCOutOfRange`, `ErrRPCNoWallet`, `ErrRPCUnimplemented` | | **W** passthrough cancellation, misc handler errors |
| -2 | `ErrRPCForbiddenBySafeMode` | | |
| -3 | `ErrRPCType` | | |
| -4 | `ErrRPCWallet` | | **W** default for every unclassified error |
| -5 | `ErrRPCInvalidAddressOrKey`, `ErrRPCBlockNotFound`, `ErrRPCBlockCount`, `ErrRPCBestBlockHash`, `ErrRPCDifficulty`, `ErrRPCNoTxInfo`, `ErrRPCNoNewestBlockInfo`, `ErrRPCInvalidTxVout`, `ErrRPCNoTreasury`, `ErrRPCNoMixMsgInfo` | | **W** |
| -6 | `ErrRPCWalletInsufficientFunds` | | **W** |
| -7 | `ErrRPCOutOfMemory` | | |
| -8 | `ErrRPCInvalidParameter` | | **W** |
| -9 | `ErrRPCClientNotConnected` | | **W** |
| -10 | `ErrRPCClientInInitialDownload` | | |
| -11 | `ErrRPCWalletInvalidAccountName` | | **W** |
| -12 | `ErrRPCWalletKeypoolRanOut` | | **W** (one handler use) |
| -13 | `ErrRPCWalletUnlockNeeded` | | **W** |
| -14 | `ErrRPCWalletPassphraseIncorrect` | | **W** |
| -15 | `ErrRPCWalletWrongEncState` | | |
| -16 | `ErrRPCWalletEncryptionFailed` | | |
| -17 | `ErrRPCWalletAlreadyUnlocked` | | |
| -20 | `ErrRPCDatabase` | | |
| -22 | `ErrRPCDeserialization`, `ErrRPCDecodeHexString` | | **W** |
| -25 | `ErrRPCVerify` | | |
| -26 | `ErrRPCInvalidState`, `ErrRPCProfilerState` | | |
| -27 | `ErrRPCCancel` | | |
| -40 | `ErrRPCDuplicateTx` | | |
| -50 | `ErrRPCReconsiderFailure` | | |

Messages are free text chosen at each call site; only the five `-326xx`/
`-32700` values have fixed messages in dcrjson. `jsonerr.go` additionally
declares *value-typed* errors (`dcrjson.RPCError`, not pointers) with stock
messages (`ErrInvalidRequest`, `ErrWallet` "Unspecified problem with
wallet", …). dcrwallet references exactly one of them,
`dcrjson.ErrInvalidRequest`, in `setdisapprovepercent` on mainnet
(`methods.go:3856-3858`); because it is not a pointer, `convertError` does
not recognise it and the client receives code `-4` with the message
`-32600: Invalid request` (Quirk Q27).

Complete list of the `jsonerr.go` value-typed variables
(`dcrd/dcrjson/v4@v4.2.0/jsonerr.go:13-176`). Only `ErrInvalidRequest` is
referenced by dcrwallet; the rest matter only to a full `dcrjson` port.
Note the stock message of `ErrInvalidParams` is `Invalid parameters` and
that these are separate objects from the pointer-typed `ErrRPC*` values of
`jsonrpcerr.go`.

| Variable | Code | Message |
|---|---|---|
| `ErrInvalidRequest` | -32600 | `Invalid request` |
| `ErrMethodNotFound` | -32601 | `Method not found` |
| `ErrInvalidParams` | -32602 | `Invalid parameters` |
| `ErrInternal` | -32603 | `Internal error` |
| `ErrParse` | -32700 | `Parse error` |
| `ErrMisc` | -1 | `Miscellaneous error` |
| `ErrForbiddenBySafeMode` | -2 | `Server is in safe mode, and command is not allowed in safe mode` |
| `ErrType` | -3 | `Unexpected type was passed as parameter` |
| `ErrInvalidAddressOrKey` | -5 | `Invalid address or key` |
| `ErrOutOfMemory` | -7 | `Ran out of memory during operation` |
| `ErrInvalidParameter` | -8 | `Invalid, missing or duplicate parameter` |
| `ErrDatabase` | -20 | `Database error` |
| `ErrDeserialization` | -22 | `Error parsing or validating structure in raw format` |
| `ErrClientNotConnected` | -9 | `node is not connected` |
| `ErrClientInInitialDownload` | -10 | `node is downloading blocks...` |
| `ErrWallet` | -4 | `Unspecified problem with wallet` |
| `ErrWalletInsufficientFunds` | -6 | `Not enough funds in wallet or account` |
| `ErrWalletInvalidAccountName` | -11 | `Invalid account name` |
| `ErrWalletKeypoolRanOut` | -12 | `Keypool ran out, call keypoolrefill first` |
| `ErrWalletUnlockNeeded` | -13 | `Enter the wallet passphrase with walletpassphrase first` |
| `ErrWalletPassphraseIncorrect` | -14 | `The wallet passphrase entered was incorrect` |
| `ErrWalletWrongEncState` | -15 | `Command given in wrong wallet encryption state` |
| `ErrWalletEncryptionFailed` | -16 | `Failed to encrypt the wallet` |
| `ErrWalletAlreadyUnlocked` | -17 | `Wallet is already unlocked` |
| `ErrBlockNotFound` | -5 | `Block not found` |
| `ErrBlockCount` | -5 | `Error getting block count` |
| `ErrBestBlockHash` | -5 | `Error getting best block hash` |
| `ErrDifficulty` | -5 | `Error getting difficulty` |
| `ErrOutOfRange` | -1 | `Block number out of range` |
| `ErrNoTxInfo` | -5 | `No information available about transaction` |
| `ErrNoNewestBlockInfo` | -5 | `No information about newest block` |
| `ErrInvalidTxVout` | -5 | `Output index number (vout) does not exist for transaction.` |
| `ErrRawTxString` | -32602 | `Raw tx is not a string` |
| `ErrDecodeHexString` | -22 | `Unable to decode hex string` |
| `ErrNoWallet` | -1 | `This implementation does not implement wallet commands` |
| `ErrUnimplemented` | -1 | `Command unimplemented` |

### 8.2 `convertError` (`errors.go:16-49`)

Applied to every handler error and to passthrough call errors:

1. `*dcrjson.RPCError`: returned unchanged (identity, no wrapping
   inspected; a wrapped `*RPCError` or a non-pointer `dcrjson.RPCError`
   value is *not* recognised and falls to step 3, where its `Error()` text
   `"<code>: <message>"` becomes the message).
2. `*wsrpc.Error` (an error object received from dcrd): `{code: err.Code,
   message: err.Message}`.
3. Otherwise the code defaults to `-4` and is refined by the wallet error
   kind found by `errors.As(err, &kind)`, i.e. the kind of the outermost
   `*errors.Error` in the chain whose kind is not `Other`:

| Wallet kind | Code |
|---|---|
| `Bug` | -32603 |
| `Encoding` | -8 |
| `Locked` | -13 |
| `Passphrase` | -14 |
| `NoPeers` | -9 |
| `InsufficientBalance` | -6 |
| any other kind, or no kind | -4 |

   The message is `err.Error()` verbatim, i.e. the wallet error chain text
   (operation names, kind descriptions such as `wallet locked`, `invalid
   passphrase`, `Decred network is unreachable`, joined by `: `; see
   `errors/errors.go:77-124`, `247-290`). These strings are client-visible
   and must match the wallet-core error formatting spec.

`rpcError(code, err)` and `rpcErrorf(code, format, args...)`
(`errors.go:51-63`) build `{code, message}` from an error's text or a
`fmt.Sprintf`.

### 8.3 Fixed error values (`errors.go:66-111`)

| Variable | Code | Message |
|---|---|---|
| `errUnloadedWallet` | -4 | `request requires a wallet but wallet has not loaded yet` |
| `errRPCClientNotConnected` | -9 | `disconnected from consensus RPC` |
| `errNoNetwork` | -9 | `disconnected from network` |
| `errAccountNotFound` | -11 | `account not found` |
| `errAddressNotInWallet` | -4 | `address not found in wallet` |
| `errNotImportedAccount` | -4 | `imported addresses must belong to the imported account` |
| `errNeedPositiveAmount` | -8 | `amount must be positive` |
| `errWalletUnlockNeeded` | -13 | `wallet or account locked; use walletpassphrase or unlockaccount first` |
| `errReservedAccountName` | -8 | `account name is reserved by RPC server` |

Dispatch-layer errors:

| Situation | Code | Message |
|---|---|---|
| Passthrough, no wallet or backend | -9 | `disconnected from consensus RPC` |
| Passthrough under SPV/offline | -9 | `RPC passthrough requires dcrd RPC synchronization` |
| Passthrough, context done | -1 | `context canceled` / `context deadline exceeded` |
| Parameter parsing failure | -32600 | `Invalid request` |
| `unimplemented` (unreachable) | -1 | `Method unimplemented` |
| `unsupported` (unreachable) | -1 | `Request unsupported by dcrwallet` |
| `help` for an unknown command | -8 | `no help for method "<name>"` |
| `debuglevel` bad spec | -8 | `invalid debug level <spec>: <err>` |
| `makeResponse` marshal failure (unreachable) | -32603 | `Unexpected error marshalling result` |

### 8.4 Non-RPC errors

| Error | Surface |
|---|---|
| `one time TLS keys are enabled, but TLS key \`<path>\` already exists` | start-up failure; logged as `Unable to create RPC servers: …`; process exits non-zero |
| `failed to create listeners for RPC server` / `failed to create listeners for JSON-RPC server` | same |
| `no suitable RPC services can be started` | same |
| `valid until date <t> already elapsed`, `failed to marshal private key: …`, `failed to encode private key: …`, `failed to encode certificate: …` | `--issueclientcert` failures; same surface |
| file read/write, key generation and `tls.LoadX509KeyPair` errors | same, with the OS/Go error text |
| `unknown curve ID` (`internal/cfgutil/curve.go:112`, `122`) | key or certificate generation with an invalid internal curve value; unreachable through the flag parser |
| `os.MkdirAll` failure for the certificate or key directory, `tls.X509KeyPair` failure on the freshly generated pair, `os.WriteFile` failure (`rpcserver.go:76-110`) | start-up failure, OS/Go error text; a failed key write first removes the certificate file |
| client CA file exists but cannot be read (`rpcserver.go:279-282`) | start-up failure with the OS error; a file that reads but contains no certificate is only a warning |
| panic in the websocket responder if a `makeResponse` reply cannot be marshalled (`server.go:384-386`, `411-413`, `433-435`) | process crash; unreachable, since the id came out of `encoding/json` and the results are `nil` or a string |
| panic inside a table handler or the passthrough closure on the **POST** route | recovered by `net/http` on the connection goroutine (Go `net/http/server.go`, `conn.serve`): the connection is closed without any HTTP response, the throttle slot and the wait-group count are released by their deferred calls, the process keeps running, and `http: panic serving <remote-addr>: <value>` plus a stack trace is written to stderr (8.5) |
| panic inside a request on the **websocket** route (`server.go:448-459`) | not recovered: the request runs on its own goroutine, so the Go runtime terminates the whole process with the panic trace on stderr and exit status 2 (Quirk Q39) |
| `missing Authorization header`, `invalid Authorization header` | internal only; clients see 401 |
| `websocket client disconnected` | internal only |
| dcrjson registration errors (`ErrDuplicateMethod`, `ErrInvalidType`, …) | panic at process init; cannot occur with the shipped types |

### 8.5 Log lines (subsystem `RPCS` unless noted)

| Level | Text |
|---|---|
| info | `Listening on <addr>` |
| trace | `Finished serving RPC: <err>` |
| warn | `Failed authentication attempt from client <addr>` (HTTP header auth, both routes) |
| warn | `Cannot websocket upgrade client <addr>: <err>` |
| error | `` Cannot close listener `<addr>`: <err> `` |
| debug | `RPC method "<method>" invoked by <addr>` |
| warn | `Reached threshold of <N> concurrent active clients` |
| warn | `Websocket receive failed from client <addr>: <err>` |
| warn | `Failed unmarshal of JSON-RPC request object from client <addr>` |
| debug | `RPC method authenticate invoked by <addr>` |
| warn | `Multiple authentication attempts from <addr>` |
| warn | `Failed authentication attempt from <addr>` (websocket `authenticate`) |
| debug | `RPC method stop invoked by <addr>` |
| error | `Unable to marshal response to client <addr>: <err>` |
| warn | `Cannot set write deadline on client <addr>: <err>` |
| warn | `Failed websocket send to client <addr>: <err>` |
| info | `Disconnected websocket client <addr>` |
| info | `New websocket client <addr>` |
| warn | `Cannot remove read deadline: <err>` |
| warn | `Request from client <addr> exceeds maximum size` |
| warn | `Failed to read request from client <addr>` |
| warn | `Cannot write invalid request response to client <addr>: <err>` |
| warn | `Invalid RPC method authenticate invoked by HTTP POST client <addr>` |
| warn | `Failed to write response to client <addr>: <err>` |
| warn | `Canceled RPC method <method> invoked by <addr>: <err>` |
| info (`DCRW`) | `Generating TLS certificates...`, `Done generating TLS certificates`, `Server TLS is disabled.  Only JSON-RPC may be used`, `JSON-RPC server disabled (basic auth requires username and password, and client cert authentication is not enabled)`, `gRPC server listening on <addr>`, `JSON-RPC server shutdown`, `gRPC server shutdown` |
| warn (`DCRW`) | `Cannot remove written certificates: <err>`, `No certificates added from CA file <path>`, the two "no trusted client certificates exist" messages (5.1), `` `<host>` is not a valid IP address ``, `Can't listen on <addr>: <err>`, `Stopping JSON-RPC server...`, `Stopping gRPC server...` |
| error (`DCRW`) | `` `<addr>` is not a normalized listener address ``, `Unable to create RPC servers: <err>` |
| trace (`DCRW`) | `Finished serving gRPC: <err>` |

`<addr>` is Go's `http.Request.RemoteAddr` (`ip:port`, IPv6 in brackets).
Request parameters, passphrases included, are never logged by this layer.

Lines that bypass the wallet's logger. `http.Server.ErrorLog` is not set
(`server.go:97-110`), so `net/http` reports its own connection-level
problems through Go's process-wide standard `log` logger, which dcrwallet
never redirects (no `log.SetOutput` anywhere in the non-test tree): they
go to **stderr only**, never to the log file, with the standard-logger
prefix `YYYY/MM/DD HH:MM:SS ` in local time and no subsystem or level tag
(Go `net/http/server.go`, `Server.logf`). The ones a deployment will see:

| Text | When |
|---|---|
| `http: TLS handshake error from <remote-addr>: <err>` | A TLS handshake on a JSON-RPC listener fails: wrong protocol, handshake not finished within 10 s, and in `clientcert` mode every client that presents no certificate or an untrusted one (`tls: client didn't provide a certificate`, `tls: failed to verify certificate: ...`). There is no `RPCS` log line for a rejected client certificate. |
| `http: panic serving <remote-addr>: <value>` followed by a goroutine stack | A POST handler panicked (8.4). |
| `http: Accept error: <err>; retrying in <delay>` | Temporary `accept` failures (for example file-descriptor exhaustion); `net/http` retries with a back-off from 5 ms doubling to 1 s. |

gorilla likewise uses the standard logger for `websocket: failed to close
network connection: <err>` (`websocket@v1.5.1/server.go:186-190`, `254-280`).

---

## 9. Compatibility-critical behaviours

1. **Routes and transports.** `/ws` is the websocket endpoint; every other
   path is the POST endpoint. Which external clients use which route
   (`dcrctl --wallet`, Decrediton, wsrpc-based clients) is UNVERIFIED: no
   client source was read for this spec; both routes must work.
2. **Basic auth is an exact string match** of the `Authorization` header
   against `Basic ` + padded standard base64 of `user:pass`; failures are
   401 with `WWW-Authenticate: Basic realm="dcrwallet RPC"`.
3. **Websocket auth alternatives**: header at upgrade, or first message
   `authenticate [username, passphrase]` answered with
   `{"jsonrpc":"","result":null,"error":null,"id":<id>}`. An unauthenticated
   client's non-`authenticate` request is never answered.
4. **Response envelope**: member order `jsonrpc`, `result`, `error`, `id`;
   `"error":null` on success and `"result":null` on error (both always
   present); `jsonrpc` echoes `1.0`/`2.0` and defaults to `1.0`; HTTP status
   200 for JSON-RPC errors.
5. **Go JSON text rules** in every response: HTML-escaped `<`, `>`, `&`,
   sorted map keys, Go float formatting (including `float32` for
   `headersfetchprogress`), struct member order, `omitempty` rules,
   `null` for nil slices/maps/pointers without `omitempty`.
6. **Positional parameters only**, with the exact arity ranges, types,
   optionality and defaults of section 4.4. `null` for an optional
   parameter means "absent without default"; `null` for a required
   parameter means the zero value.
7. **All parameter errors are `-32600 Invalid request`** (not `-32602`),
   and so are the eight bitcoind-compat names in 4.9.
8. **Unknown methods are forwarded to dcrd** when syncing through dcrd RPC,
   returning dcrd's result bytes or dcrd's error code and message; without
   dcrd they fail with `-9` and the two messages in 8.3. A chain query such
   as `getrawtransaction` sent to the wallet port is therefore answered by
   dcrd.
9. **`getblock`, `getblockheader`, `gettxout` forward to dcrd** when
   available (so their output is dcrd's, including members the wallet's SPV
   implementation does not fill).
10. **Error code mapping** of 8.2 and the fixed messages of 8.3; in
    particular `-13` for locked wallet, `-14` for bad passphrase, `-6` for
    insufficient balance, `-9` for no network, `-4` otherwise, with the
    wallet error text as the message.
11. **`version` result** keys `dcrwallet` and `dcrwalletjsonrpcapi`
    (`11.0.0`), merged over dcrd's `dcrd` and `dcrdjsonrpcapi` when
    available.
12. **`help` output**: `requestUsages` verbatim (with the `Chain server
    usage:` / `Wallet server usage (overrides chain requests):` wrapper
    when dcrd is attached) and the 103 per-method texts verbatim.
13. **`stop`**: accepted on both transports, result strings differ by a
    trailing full stop, reply precedes shutdown.
14. **Result struct shapes** of section 4.6: key spelling and case, order,
    `omitempty`.
15. **TLS behaviour**: self-generated `rpc.cert`/`rpc.key` at the default
    paths with the SAN set of 5.1.3 so that existing clients configured
    with the certificate keep working; an existing key pair in either PEM
    private-key format must load unchanged (ECDSA SEC 1 `EC PRIVATE KEY`
    from older installs, PKCS#8 `PRIVATE KEY` for Ed25519, and anything
    else Go's `tls.LoadX509KeyPair` accepts, e.g. PKCS#1 RSA supplied by
    the operator); TLS 1.2 minimum; client-certificate auth against
    `clients.pem` when `--jsonrpcauthtype=clientcert`.
16. **Pipe messages** `jsonrpclistener`, `grpclistener`,
    `issuedclientcertificate` with the framing and payloads of 4.12
    (the pipe protocol exists for parent processes that launch the wallet
    with `--pipetx`, `--rpclistenerevents` and `--issueclientcert`;
    Decrediton's actual use of these was not checked for this spec).
17. **Start-up outcomes**: the conditions under which the JSON-RPC server
    silently does not start (5.1 step 4) and the fatal error when neither
    server starts.
18. **Limits**: 4 MiB POST body (413), `--rpcmaxclients` /
    `--rpcmaxwebsockets` concurrency caps answered with 429 before
    authentication.
19. **Concurrent websocket requests** with out-of-order replies; clients
    that multiplex calls over one websocket must not be serialised behind
    a long request.
20. **Works before a wallet is loaded** (the `--noinitialload` case), with `-4 request requires a wallet
    but wallet has not loaded yet` for wallet-dependent methods and normal
    replies for `help`, `version`, `getcurrentnet`, `debuglevel`,
    `verifymessage`, `createrawtransaction`.
21. **Log line texts** in 8.5 for operators and tooling that scrape logs
    (lower priority than wire behaviour).
22. **`help <command>` fallback chain**: wallet text first, then dcrd's
    detailed help for names the wallet does not document (dcrd's error
    passes through), then `-8 no help for method "<command>"`; with
    `command` equal to `""` or `null` behaving as absent (5.6).
23. **Listener set**: one listener per normalised address, IPv4 (`tcp4`)
    before IPv6 (`tcp6`), host names never resolved at listen time, a
    listener that fails to bind skipped with a warning as long as at least
    one binds; `jsonrpclistener` pipe messages in that same order. Parents
    that start the wallet on port `0` rely on the reported addresses
    (`ipc.go:116-120`, `148-151`).
24. **Client-side request shape** for a Rust `dcrctl` equivalent or test
    harness: `MarshalCmd` output `{"jsonrpc":…,"method":…,"params":[…],"id":…}`
    with `params` always an array, optional parameters truncated at the
    first nil, and `NewCmd` string-to-type conversions and error texts of
    4.3.1.
25. **Client-certificate mode accepts any certificate chaining to a CA in
    `--clientcafile`** (or to the ephemeral CA of `--issueclientcert`);
    there is no per-client identity, no revocation check and no HTTP auth
    on top. A websocket `authenticate` message in this mode ends the
    session (the connection is pre-authenticated, so it is a "multiple
    authentication" attempt).
26. **Keep-alive asymmetry**: every reply from the `/` handler closes the
    connection; 429 replies and `/ws` handshake failures do not (4.11).
27. **Websocket protocol behaviour of 4.11**: one unfragmented text frame
    per response, pong replies to client pings, a close frame echoed with
    the client's status code when the client closes, status 1002 on
    protocol violations, no size limit and no compression. Which
    client libraries rely on the close echo was not checked for this
    spec; a standards-conforming client waits for it to complete its
    closing handshake.
28. **Client-certificate tooling**: the PEM triple delivered in the
    `issuedclientcertificate` pipe message must load with a standard TLS
    stack as a key pair (PKCS#8 key first, then the client certificate)
    whose chain verifies against the third block, for every `--tlscurve`
    value; a client certificate from any other CA must be rejected during
    the handshake (pinned by the two upstream tests in `x509_test.go`,
    section 11).

---

## 10. Quirks

Q1. **`unimplemented`/`unsupported` handlers are unreachable.** The eight
`noHelp` names are in `handlers` but have no dcrjson registration, so
`ParseParams` fails first and the reply is `-32600 Invalid request`
(`methods.go:192-199`, `262-266`; registration list
`rpc/jsonrpc/types/methods.go:1189-1312`). They are also never forwarded
to dcrd. Reproduce the observable reply; the two handler bodies are dead
code.

Q2. **Parameter errors lose their diagnostics.** Every `ParseParams` error
collapses to `-32600 Invalid request` (`methods.go:264-266`) instead of
`-32602` with the dcrjson description. Clients cannot tell a wrong arity
from a wrong type from a malformed envelope.

Q3. **The server never closes a websocket.** "Disconnect immediately"
comments (`server.go:375`, `422`) only stop the responder; no close frame
is sent and the TCP connection is left open (there is no `conn.Close()` in
`server.go`). A client that fails authentication simply stops receiving
replies. If it sends one more message the reader goroutine blocks forever
on `allRequests` and the socket leaks until process exit. The throttle
slot is released because the HTTP handler returns. A port must decide
whether to reproduce the silent hang or close the connection (Decision D2).

Q4. **Websocket special replies carry `"jsonrpc":""`** (`authenticate`,
`stop`, invalid-request; `methods.go:284-306`, `server.go:378`, `408`,
`429`), while every other reply echoes the request version. They also skip
the id-type check, so a boolean/array/object id is echoed there but causes
a marshal failure (no reply on websocket, HTTP 500 on POST) everywhere
else.

Q5. **`--noservertls` with `--jsonrpcauthtype=clientcert` gives an
unauthenticated server.** The client-CA check at `rpcserver.go:350-354` is
skipped when TLS is off, the username/password are withheld because the
auth type is not `basic` (`rpcserver.go:364-367`), so `authsha` is nil and
`checkAuthHeader` accepts everything. Exposure is limited to localhost by
the config rule for `--noservertls`. Reproduce or reject at start-up
(Decision D3).

Q6. **POST `authenticate` gets an empty HTTP 200** with `Content-Type:
application/json` and no body (`server.go:585-589`).

Q7. **POST accepts any HTTP method** and any path other than `/ws`
(`server.go:129`).

Q8. **Stop strings differ**: `dcrwallet stopping` vs `dcrwallet stopping.`
(`server.go:593`, `430`).

Q9. **Requests with unsupported id types execute but cannot be answered**
(`jsonrpc.go:135-138` via `server.go:599-605`, `452-458`). A
state-changing call with `"id":true` takes effect and returns HTTP 500 or
nothing.

Q10. **`null` handling in parameters** (4.3): `null` for a defaulted
optional parameter yields nil, not the default; handlers therefore
nil-check even "defaulted" parameters. `null` for a required parameter is
accepted as the zero value.

Q11. **Usage strings use lower-cased Go field names, not JSON keys**, so
they misdocument object keys: `fundrawtransaction` shows `"conftarget"`
where the wire key is `conf_target`; `signrawtransaction` shows
`"scriptpubkey"`/`"redeemscript"` for `scriptPubKey`/`redeemScript` (the
latter still work because Go matches keys case-insensitively; `conftarget`
does **not** match `conf_target`). `sendfromtreasury` shows a bare
`amounts` because, unlike `sendmany`, the field has no `jsonrpcusage` tag
(`rpc/jsonrpc/types/methods.go:862` vs `781`).

Q12. **Help text vs code.** `debuglevel` help lists 12 subsystems; the
running daemon has 13 (`AMGR` is missing from the text,
`internal/rpchelp/helpdescs_en_US.go:105` vs `log.go:47-61`).
`mixaccount` help says "Mix all outputs of an account." and the
description map has `mixaccount-account`, but the command takes no
parameters. `purchaseticket-fromaccount` says `(default="default")` for a
required parameter. `purchaseticket-nosplittransaction`, the
`getbalancetomaintain`, `getticketmaxprice`, `setbalancetomaintain`,
`setgenerate`, `setticketmaxprice` families and several
`--condition`/`--result` keys are dead (4.10.3). `gettickets
includeimmature` is a required boolean although the help style suggests a
flag. The "CreateMultisigResult help" comment is duplicated above the
`createsignatureresult` keys (`helpdescs_en_US.go:70`).

Q13. **Help result types disagree with handlers** (documentation only):
`purchaseticket` documents a string but returns an array of hashes or a
`CreateUnsignedTicketResult` object (`methods.go:3491`, `3514-3517`);
`sendtomultisig` documents a string but returns a `SendToMultiSigResult`
object (`methods.go:4660-4664`); `redeemmultisigouts` documents a single
`RedeemMultiSigOutResult` but returns `{"results":[…]}`
(`methods.go:4218`); `listunspent` and `getpeerinfo` document a single
object but return arrays; `getblockcount`, `getreceivedby*`,
`getunconfirmedbalance`, `getwalletfee` are documented as `n.nnn`;
`getcoinjoinsbyacct` documents `uint32` values but the handler builds
`map[string]int` (`methods.go:5849`); `debuglevel` documents only `Done.`
although `show` returns the subsystem list.

Q14. **`getpeerinfo` narrows dcrd's reply**: under dcrd sync the wallet
unmarshals dcrd's peer list into its own 8-member struct and re-emits it,
dropping dcrd-only members and always emitting `addrlocal` (dcrd omits it
when empty) (`methods.go:2500-2509`; `results.go:71-80`). Under an offline
backend the result is `null`.

Q15. **Passthrough is unrestricted**: any dcrd method, including
websocket-only and notification-registration methods, is forwarded over
the wallet's own dcrd connection (`methods.go:231-259`). `help` with no
argument concatenates dcrd's usage (which includes those methods) in front
of the wallet's.

Q16. **`UFWebsocketOnly` is never consulted.** `authenticate` is rejected
on POST by an explicit string comparison, not by the registration flag
(`server.go:585`).

Q17. **Go runtime dependencies** a port must emulate or consciously drop:
case-insensitive JSON key matching and last-duplicate-wins for the request
envelope and nested parameter objects; continuation after member type
errors (4.1 rule 3); `float64` round-trip of numeric ids; HTML escaping;
sorted map keys; `%q`/`%v` formatting in messages (`no help for method
"x"`, `Supported subsystems [...]`); `net/http` details (path-cleaning
redirect and its release-dependent status, 4.11; `Content-Length` vs chunked, 1 MiB header cap, read-deadline
context cancellation); `text/tabwriter` alignment in help; interface
enumeration order in certificate SANs; TLS cipher-suite/curve defaults.

Q18. **HTTP/2 is never negotiated.** The TLS listener is created with
`tls.Listen` on a config without ALPN protocols and passed to
`http.Server.Serve`, so only HTTP/1.1 is spoken (derived from Go
`net/http` semantics; not probed live).

Q19. **A normal websocket close is logged as a warning** because gorilla
reports it as `*websocket.CloseError`, which is neither `io.EOF` nor
`io.ErrUnexpectedEOF` (`server.go:342-345`).

Q20. **`stop` skips parameter validation** on both transports and, on
websocket, still requires prior authentication but nothing else.

Q21. **Throttle before auth.** Unauthenticated clients consume
concurrency slots and can cause 429 for legitimate ones; websocket
sessions hold a slot for their lifetime even when idle.

Q22. **No server-side keepalive** on websockets: no pings, no idle
timeout; a half-open connection is noticed only on the next write (2 s
deadline).

Q23. **`Server.Stop` does not shut the HTTP server down** (`server.go:216-240`):
it closes listeners and waits; in-flight POST handlers are not interrupted
by `Stop` itself, only by the already-cancelled base context.

Q24. **Doc vs code, sample config**: `sample-dcrwallet.conf` mentions a
`legacyrpclisten` option (`sample-dcrwallet.conf:175`) that does not
exist; the flag is `rpclisten` (`config.go:155`). There is no
`--jsonrpcclientcafile` option either: client-certificate authentication
is configured with `--clientcafile` and `--jsonrpcauthtype`
(`config.go:122`, `162`), and the one CA file is shared with gRPC.

Q25. **Dead keys and tags**: top-level `json` tags on command structs
(4.4), all `jsonerr.go` value errors except `ErrInvalidRequest`,
`makeResponse`'s marshal-failure branch, the `noHelp` handlers.

Q26. **`convertError` uses type switches, not `errors.As`, for RPC error
types** (`errors.go:17-25`): an `*RPCError` wrapped by `fmt.Errorf("%w")`
or by the wallet `errors.E` would be re-coded as `-4` with the wrapped
text.

Q27. **`setdisapprovepercent` on mainnet answers `-4 "-32600: Invalid
request"`.** The handler returns the value-typed `dcrjson.ErrInvalidRequest`
(`methods.go:3856-3858`); `convertError` matches only the pointer type, so
the error is treated as an unclassified wallet error and its `Error()`
string (`"%d: %s"`, `dcrd/dcrjson/v4@v4.2.0/jsonrpc.go:31-33`) becomes the
message. This is the only such site in the package; `importcfiltersv2`
separately uses code `-32600` with a custom message (`methods.go:2000`)
and code `-32602` (`methods.go:1994`).

Q28. **A websocket session outlives its sender.** When a write fails (or
the 2 s write deadline passes) the sender exits and closes the session
`quit` channel, the upgrade handler returns, the throttle slot is freed and
`net/http` cancels the request context, but nothing stops the reader or the
responder (`server.go:473-503`, `507-527`). While the client keeps sending,
the responder keeps dispatching each request on its own goroutine with an
already-cancelled context and discards the reply (`_ = wsc.send(mresp)`,
`server.go:457`; `send` returns `websocket client disconnected` at once).
Handlers that do not consult the context before acting still take effect.
`authenticate`, `stop` and the invalid-request path leave the loop on the
failed `send`; for `stop` this happens **before** `requestProcessShutdown`,
so a `stop` on such a session does not shut the process down
(`server.go:436-441`). The responder's server wait-group count is held
until the client disconnects or `Stop` runs. Derived from the code and Go
`net/http` (the per-request context is cancelled when the handler returns,
hijacked or not); not probed live.

Q29. **Passthrough reports cancellation even after success**
(`methods.go:248-254`): the context is examined after the dcrd call
returns, regardless of whether the call produced a result.

Q30. **`help <command>` holds a global mutex across a dcrd round trip**
(`methods.go:2910-2933`); see 5.6.

Q31. **`MarshalCmd` truncates at the first nil optional field**
(`dcrd/dcrjson/v4@v4.2.0/cmdparse.go:18-34`): a command with a nil optional
followed by a non-nil one is sent without the later value. Together with
positional parameters this means a client cannot set a later optional
parameter while leaving an earlier one at its default except by sending an
explicit value or a JSON `null` built by hand (and `null` suppresses the
default, Q10). Client-side only.

Q32. **Relative TLS paths without a directory component cannot be
generated.** `generateRPCKeyPair` calls `os.MkdirAll(dir, 0700)` on the
directory part returned by `filepath.Split` (`rpcserver.go:74-83`). The
paths are cleaned but not made absolute, so `--rpccert=rpc.cert` yields the
directory `""` and, by reading of Go's `os.MkdirAll`, an error of the form
`mkdir : no such file or directory`, failing start-up. UNVERIFIED on a
running binary (Open question O8). Paths with any directory component,
relative or absolute, work.

Q33. **Optional array-of-object arguments are not expanded in help.** The
argument-table code tests `Field.Type.Elem().Kind() == Struct` on the
declared type (`dcrd/dcrjson/v4@v4.2.0/help.go:372-379`). For
`signrawtransaction`'s `inputs` (`*[]RawTxInput`) the element of the
pointer is a slice, so the help shows only the row `2. inputs (array of
object, optional) ...` with no member listing, and no `rawtxinput-*`
description keys exist. Required `[]Struct` arguments (`lockunspent`,
`createrawtransaction`, `spendoutputs`) and the optional struct
`fundrawtransaction` `options` are expanded. The one-line usage is not
affected (it de-pointers first, `cmdinfo.go:139-143`).

Q34. **`doc.go` of the types package describes things that do not exist**
(notification types, `UnmarshalCmd`); see 3.3.

Q35. **The generator truncates its output before generating.**
`genrpcserverhelp.go:25-31` creates `rpcserverhelp.go` in a package-level
initialiser; a failing generation run leaves a truncated file. Build-time
only.

Q36. **Throttle thresholds are unvalidated**: zero or negative
`--rpcmaxclients` / `--rpcmaxwebsockets` disable the route with 429 for
every request (5.1.1).

Q37. **The `/` handler joins the server wait group only after
authentication** (`server.go:135-142`), and the websocket reader is never
in it; `Stop` therefore does not wait for requests still being
authenticated or throttled, nor for readers.

Q38. **`Canceled RPC method` logs differ between the two dispatch paths**:
the table path logs `ctx.Err()`, the passthrough path logs the dcrd call
error (possibly `<nil>` if the call succeeded) (`methods.go:249`, `270`);
both format the method with `%v` (unquoted), unlike the `invoked by` debug
line, which uses `%q`.

Q39. **A handler panic is survivable over POST and fatal over websocket.**
POST handlers run on the `net/http` connection goroutine, whose deferred
`recover` logs `http: panic serving ...` and drops the connection; the
same handler invoked through `/ws` runs on a bare goroutine
(`server.go:448-459`) and an unrecovered panic there ends the process.
The three `panic(err)` calls in the responder (`server.go:385`, `412`,
`434`) are in the same unrecovered position. A Rust port has to choose
what a handler panic does on each transport (abort the process on both,
or isolate on both); matching Go exactly means isolating only POST.

Q40. **`net/http` diagnostics go to stderr, not to the wallet log**
(8.5): TLS handshake failures, including every rejected client
certificate in `clientcert` mode, are invisible in `dcrwallet.log` and
carry a different timestamp format from the wallet's own lines.

Q41. **Close handshake without teardown.** When a client sends a close
frame gorilla echoes it, but the wallet never closes the socket (Q3), and
after the echo the connection is write-dead: replies of requests still in
flight fail with `websocket: close sent` and are logged as send failures
(4.11, websocket item 8). A client that half-closes and waits for
outstanding replies never receives them.

Q42. **The start-up "JSON-RPC server disabled" line is logged even when
there is nothing to disable.** The branch at `rpcserver.go:355-357` tests
only the auth type and the credentials, before the listener count, so
with `--nolegacyrpc` (no JSON-RPC listeners) and no username/password the
info line `JSON-RPC server disabled (basic auth requires username and
password, and client cert authentication is not enabled)` is still
printed; with credentials set, or with auth type `clientcert` and client
CAs present, an empty listener list skips the JSON-RPC server silently.
Likewise the `clientcert` warning of 5.1 step 4.1 is printed only when
TLS is on and at least one JSON-RPC listen address is configured.

---

## 11. Upstream tests

| Test | File | What it pins | Port |
|---|---|---|---|
| `TestWalletSvrCmds` | `rpc/jsonrpc/types/methods_test.go` (79 cases) | For 39 wallet methods: `MarshalCmd` output of the static constructor and of `dcrjson.NewCmd` equals a literal JSON-RPC 1.0 request; `ParseParams` of that request equals an expected struct, including applied defaults and nil optionals. Covers `addmultisigaddress`, `createmultisig`, `createnewaccount`, `dumpprivkey`, `getaccount`, `getaccountaddress`, `getaddressesbyaccount`, `getbalance`, `getnewaddress`, `getrawchangeaddress`, `getreceivedbyaccount`, `getreceivedbyaddress`, `gettransaction`, `importprivkey`, `importpubkey`, `listaccounts`, `listlockunspent`, `listreceivedbyaccount`, `listreceivedbyaddress`, `listsinceblock`, `listtransactions`, `listunspent`, `lockunspent`, `renameaccount`, `sendfrom`, `sendmany`, `sendtoaddress`, `sendfromtreasury`, `sendtotreasury`, `settspendpolicy`, `settreasurypolicy`, `settxfee`, `signmessage`, `signrawtransaction`, `sweepaccount`, `walletlock`, `walletpassphrase`, `walletpassphrasechange`, `walletpubpassphrasechange`. | Port all vectors verbatim; they are literal strings. |
| `TestWalletSvrWsCmds` | `rpc/jsonrpc/types/walletsvrwscmds_test.go` (7 cases) | Same harness for `getunconfirmedbalance`, `listaddresstransactions`, `listalltransactions`, `walletislocked`. | Port verbatim. |
| `TestRPCMethodHelpGeneration` | `internal/rpc/jsonrpc/rpchelp_test.go:39` | Every non-`noHelp` handler appears in `rpchelp.Methods`; `GenerateHelp` succeeds for each (no missing description key); the generated text equals the committed `rpcserverhelp.go`. | Port as: handler table ⊆ help table; regenerated text equals embedded text. |
| `TestRPCMethodUsageGeneration` | `rpchelp_test.go:78` | `MethodUsageText` succeeds for each method and the joined usages equal `requestUsages`. | Port likewise. |
| `TestClientCert` | `x509_test.go` (root, package `main`) | For Ed25519, P-256, P-384 and P-521: `generateAuthority(pub, priv)`, `marshalPrivateKey(ca.PrivateKey)` and `createSignedClientCert(pub, ca.PrivateKey, ca.Cert)` produce a PEM pair that `tls.X509KeyPair` accepts and that authenticates against a TLS 1.2+ server configured with `RequireAndVerifyClientCert` and the CA in `ClientCAs` (an HTTP PUT is echoed back). The test uses the **same** key for the CA and the client certificate, as `startRPCServers` does not (it generates a second key in `generateClientKeyPair`). | Port as an integration test of the client-CA and client-certificate templates of 5.1.4 against the Rust TLS stack, for all four curves. |
| `TestUntrustedClientCert` | `x509_test.go` | A client certificate issued by a different (Ed25519) authority is rejected by the server during the handshake; the client-side error must end in `tls: bad certificate` or `tls: unknown certificate authority` (connection resets are retried, 5 s overall timeout). | Port: a certificate from an untrusted CA must fail the handshake with a TLS alert, not at the HTTP layer. |
| `TestThrottle` | `internal/rpc/jsonrpc/rpcserver_test.go:15` | With threshold 1 and two concurrent requests, one gets 200 and the other 429. | Port. |
| dcrjson tests | `dcrd/dcrjson/v4@v4.2.0/*_test.go` | Registration rules, `NewCmd` conversions and error texts, `ParseParams`, usage and help generation, response marshalling. | dcroxide carries vector tests for its dcrjson port (`dcroxide:crates/dcroxide-dcrjson/tests/dcrjson_vectors.rs` and siblings); their coverage was not audited for this spec. |

Test mechanics worth keeping when porting:

- Both type tests run four checks per case (`methods_test.go`,
  `walletsvrwscmds_test.go`): marshal the statically constructed command
  with `MarshalCmd("1.0", testID, cmd)` (`testID` is the integer `1`) and
  compare bytes with the literal; build the same command with
  `dcrjson.NewCmd` and marshal it, same comparison; `json.Unmarshal` those
  bytes into `dcrjson.Request`; `ParseParams` it and `reflect.DeepEqual`
  against the expected struct. Because constructors and `NewCmd` do not
  apply defaults while `ParseParams` does, the expected "unmarshalled"
  struct differs from the constructed one exactly in the defaulted fields.
- `TestRPCMethodHelpGeneration` reports an out-of-date generated file
  (`Generated help texts are out of date: run 'go generate'`) only when no
  other failure occurred; a missing description is reported as `Cannot
  generate '<locale>' help for method '<m>': missing description for
  '<key>'`, and a handler without a `rpchelp.Methods` entry as `Missing
  '<locale>' help for method '<m>'` (`rpchelp_test.go:39-76`).
  `serverMethods()` (`rpchelp_test.go:27-35`) is the set of non-`noHelp`
  handler names. The reverse inclusion (a `Methods` entry without a
  handler) is not tested; at the pin the two sets are equal (103 names).
- dcrjson's own test files at the pin: `cmdinfo_test.go`,
  `cmdparse_test.go`, `error_test.go`, `example_test.go`,
  `help_test.go`, `helpers_test.go`, `jsonrpc_test.go`, `parse_test.go`,
  `register_test.go`.
- Root `rpcserver.go` is tested only through `x509_test.go` (the two
  client-certificate tests above, which exercise `generateAuthority`,
  `marshalPrivateKey`, `createSignedClientCert` and, indirectly,
  `randomX509SerialNumber` and `endOfTime`). `openRPCKeyPair`,
  `generateRPCKeyPair`, `generateClientKeyPair`, `startRPCServers`,
  `makeListeners` and the gRPC interceptors have no test. No test exists
  for `internal/rpchelp` itself, for `errors.go`, `context.go`,
  `marshaling.go`, or for the JSON-RPC `server.go` beyond `TestThrottle`.

Untested upstream, so vectors must come from a Go oracle shim or a live
Go dcrwallet:

- The whole HTTP and websocket lifecycle: auth outcomes, status codes and
  bodies, the invalid-request replies (including the echoed id rule), POST
  `authenticate`, the websocket special replies with `"jsonrpc":""`,
  unauthenticated-client behaviour, `stop`.
- `lazyApplyHandler`: passthrough errors, `-32600` for parse failures and
  for the dead entries, `convertError` mapping.
- Parameter decoding for the 61 registered commands that the two type
  tests do not touch, and explicit-`null` semantics for every command (no
  upstream case passes `null`).
- Result struct marshalling (no upstream test marshals any result type):
  generate golden JSON for each struct with zero values, fully populated
  values, nil vs empty slices and pointer-to-zero values through a Go shim.
- `help` composition with and without dcrd; `version`; `debuglevel show`.
- TLS start-up matrix (5.1.2), certificate structure, client-cert issuance
  and pipe messages.
- `makeListeners` address classification.
- Websocket protocol behaviour of 4.11 (close echo, protocol-error close,
  ping/pong, single-frame replies) and the panic asymmetry of Q39.
- The `ReadTimeout` context cancellation (O1).

---

## 12. Rust porting notes

### 12.1 Suggested boundaries

| Crate / module | Mirrors | Contents |
|---|---|---|
| `purse-rpctypes` | `rpc/jsonrpc/types` | Command descriptors (field order, kinds, optionality, defaults, usage tags) registered into a `dcroxide-dcrjson` `Registry` under the method-type name `types.Method`; typed command structs produced from `parse_params`; result structs with Go-faithful JSON emission; the type tests. |
| `purse-jsonrpc::transport` | `internal/rpc/jsonrpc/server.go`, `context.go` | HTTP/1.1 POST and websocket front-end, auth, throttle, session state machine, shutdown signalling. |
| `purse-jsonrpc::dispatch` | `methods.go:84-306`, `errors.go` | Method table, passthrough, `convert_error`, fixed errors. Handlers live in sibling modules. |
| `purse-jsonrpc::help` | `rpcserverhelp.go`, `internal/rpchelp` | Embedded usage and help strings; description map and result-type table kept only for the regeneration test. |
| daemon `rpcserver` module | root `rpcserver.go` | TLS key-pair open/generate, client CA handling, client-cert issuance, listener construction, start decision chain, pipe notifications. |

### 12.2 Reuse (crates inspected for this spec)

- `dcroxide:crates/dcroxide-dcrjson`: `Registry`, `parse_params`,
  `new_cmd`, `marshal_cmd`, `marshal_response`, `RpcId`, `RPCError` and
  code constants, `gojson` (Go-compatible JSON encoder/decoder), usage and
  help generators with a `TabWriter`. It makes Go reflection data explicit
  as `GoType` trees, which is exactly what the 104 wallet registrations
  need. Confirm that `parse_params` reproduces the `null` rules of 4.3
  (Open question O4).
- `dcroxide:crates/dcroxide-rpctypes`: the dcrd command and result
  definitions (`chainsvrcmds.rs`, `chainsvrresults.rs`, `chainsvrwscmds.rs`)
  for the 19 newtyped commands and the dcrd result shapes in 4.6,
  including `Vin`'s custom marshalling.
- `dcroxide:crates/dcroxide-certgen`: `new_tls_cert_pair` and
  `new_ed25519_tls_cert_pair` (byte-structure parity with dcrd `certgen`),
  plus DER/PEM/X.509 helpers usable for the client CA and client
  certificate of 5.1.4 (those two templates are wallet-specific and need
  new code).
- `dcroxide:crates/dcroxide-tlscrypto`: the rustls crypto provider and Go
  private-key parsing rules used by dcroxide for `LoadX509KeyPair`
  parity.
- `dcroxide:crates/dcroxide-rpc/src/http.rs`: Go-faithful request
  unmarshalling and Basic-auth helpers written for dcrd's server. dcrd
  authenticates with an HMAC and supports batches, so the wallet needs its
  own policy layer, but the envelope decoder and base64 helper are
  reusable.
- `dcroxide:crates/dcroxide-node/src/rpcrun.rs`, `rpctls.rs`,
  `websocket.rs`, `wsframe.rs`: dcroxide's thread-per-connection HTTP/1.1
  shell, TLS listener set-up and websocket framing live in the node crate,
  not a library crate; extracting them into a shared crate would let the
  wallet reuse them (only file headers were inspected; suitability must be
  confirmed by whoever does the extraction).
- `dcr-rs`: nothing applicable to this subsystem (its modules are
  `address`, `amount`, `blake256`, `hashing`, `hd`, `network`, `sighash`,
  `sign`, `tx`, `airgap`).

### 12.3 Hazards

1. Using a general-purpose HTTP framework changes observable details:
   method checking, path normalisation, header casing/order, error bodies,
   `Connection: close`, read-deadline semantics. The dcroxide approach of a
   small hand-written HTTP/1.1 server is the safer route to parity.
2. `serde`-style strictness is wrong here: the envelope and nested objects
   need Go's case-insensitive key matching, tolerance for unknown keys,
   `null` acceptance, and continue-after-type-error behaviour.
3. Result emission must go through the Go-compatible encoder; `serde_json`
   differs in float formatting, HTML escaping and map ordering.
4. Websocket concurrency: each request needs its own task with responses
   funnelled through one writer; back-pressure is an unbuffered hand-off
   and a 2 s write deadline.
5. The session-termination behaviour (Q3) is easy to "fix" accidentally;
   make the choice explicit.
6. The help table must be embedded as bytes; do not reflow or normalise
   whitespace (lines have trailing spaces).
7. Start-up decision chain (5.1 step 4) has security-relevant corner
   cases (Q5).
8. Passthrough needs a raw-call entry point on the dcrd RPC client that
   takes raw JSON parameters and returns the raw result plus dcrd's error
   object unmodified.
9. Zeroisation: `authenticate`, `walletpassphrase` and similar requests
   carry secrets in the raw request buffer and parsed command; Go does not
   scrub them. A port may scrub without affecting parity.

### 12.4 Size estimate

**M** for this spec's own scope (transport, dispatch, types, help data,
TLS start-up), assuming the dcroxide crates above are reused: roughly
1.5-2.5k lines of new Rust plus generated/embedded data and a Go oracle
shim for result-struct goldens. It becomes **L** if a shared HTTP/websocket
shell has to be extracted from `dcroxide-node` first. The 103 handlers
themselves are out of scope and dominate the JSON-RPC effort overall.

---

## 13. Open questions

O1. **`ReadTimeout` versus long POST requests** (7.3). Source analysis of
Go `net/http` says a POST handler's context is cancelled about 10 s after
the request starts, which would abort long calls such as `rescanwallet`,
`discoverusage`, `purchaseticket` or `importprivkey` with rescan when
issued through `dcrctl` (HTTP POST). UNVERIFIED on a running daemon.
Settle by timing a deliberately slow POST call against Go dcrwallet at the
pin (for example `rescanwallet` on a wallet with enough history) and
recording whether the `Canceled RPC method` warning appears and what the
client receives. The answer determines whether the Rust server must
implement the same 10 s cancellation.

O2. **HTTP/2 and exact response framing** (Q18, 4.11). Derived, not probed.
Settle with a TLS probe (ALPN offer of `h2`) and by capturing replies below
and above 2 KiB.

O3. **Which wallet operations return `Locked`** for the `K` rows of 4.9.
The classification comes from the wallet method each handler calls;
the wallet-core and per-method specs must confirm each one (notably
`setaccountpassphrase`, `sendtomultisig`, `sendfromtreasury`,
`discoverusage`, and the mixing calls).

O4. **`dcroxide-dcrjson::parse_params` and `null`.** Confirm (by test, not
by reading) that it leaves optional fields nil without applying defaults
and accepts `null` for required fields, as Go does (4.3).

O5. **`getpeerinfo` under an offline backend** returns `null`
(`methods.go:2500-2509`); confirm against the SPV/sync spec that
`OfflineNetworkBackend` is the only third backend type.

O6. **Version identity** reported under the `dcrwallet` key of `version`
(Decision D1).

O7. **`http.Error` header set** for the text replies depends on the Go
release (newer releases also strip certain pre-set headers); capture the
exact header lists from the Go binary built with the pinned toolchain if
header-level parity is wanted.

O8. **Relative certificate/key path with no directory component** (Quirk
Q32). Settle by starting the Go binary with `--rpccert=rpc.cert
--rpckey=rpc.key` from an empty working directory and recording the
error.

O9. **Keep-alive after 429 and after `/ws` handshake failures** (4.11,
item 26 of section 9). Settle by sending two requests on one connection
to a throttled server and observing whether the second is read.

O10. **Sender-failure behaviour of a websocket session** (Quirk Q28).
Settle with a client that stops reading until a server write times out and
then sends `walletlock` and `stop`, observing wallet state and process
lifetime.

O11. **dcrd-side shape of passthrough parameters.** The wallet hands the
raw parameter values to its dcrd client unchanged (5.5.1 step 3). The
client's `RPC` type embeds the caller directly (`rpc/client/dcrd/calls.go:42-44`),
and the wsrpc request struct declares
`Params []interface{}` with `json:"params,omitempty"`
(`wsrpc/v2@v2.3.8/rpc.go:98-102`), so a request with no parameters reaches
dcrd with the `params` member **absent** (neither `[]` nor `null`). The
rest of the dcrd-side envelope (id allocation, `jsonrpc` value) is outside
this scope and was not read for it.

### Decisions needed

D1. What `version` reports for key `dcrwallet` (keep `2.2.0-pre` + build
metadata of the parity pin, or the Rust project's own version) while
keeping `dcrwalletjsonrpcapi` at `11.0.0`.

D2. Websocket session termination: reproduce Go's "stop answering, leave
the socket open" (Q3) or close the connection with a close frame.

D3. `--noservertls` + `--jsonrpcauthtype=clientcert`: reproduce the
unauthenticated server (Q5) or refuse to start.

D4. Reproduce `-32600` for the eight dead method names (Q1) and for all
parameter errors (Q2), or return the evidently intended `-1` / `-32602`.
Parity says reproduce.

D5. Whether to reproduce the 10 s POST cancellation if O1 confirms it.

D6. Whether id types rejected by dcrjson (bool/array/object) keep the
"execute, then fail to answer" behaviour (Q9) or are rejected before
execution.

D7. Whether help strings are embedded as extracted data (recommended) or
regenerated at build time from ported descriptions.

D8. Whether a websocket session whose sender has failed keeps executing
requests (Q28) or is torn down. Tearing it down is not observable to a
well-behaved client.

D9. Whether zero/negative client limits keep meaning "reject everything"
(Q36) or are rejected at configuration time.
