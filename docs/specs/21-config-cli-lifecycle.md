# 21 — Process: configuration, CLI, startup/shutdown, wallet creation prompts, IPC, logging

## 1. Header

**Scope** (dcrwallet, line counts from `wc -l`):

| File | Lines | Covered here |
|---|---|---|
| `config.go` | 1050 | all |
| `sample-dcrwallet.conf` | 264 | all |
| `dcrwallet.go` | 636 | all |
| `walletsetup.go` | 388 | all |
| `ipc.go` | 203 | all |
| `log.go` | 96 | all |
| `signal.go` | 71 | all |
| `signal_unix.go` | 21 | all |
| `params.go` | 10 | all |
| `debug.go` | 17 | all |
| `testsolver.go` | 35 | all (called from `config.go`) |
| `internal/prompt/prompt.go` | 480 | all |
| `internal/loggers/loggers.go` | 91 | all |
| `internal/netparams/params.go` | 44 | all |
| `internal/cfgutil/address.go` | 59 | all |
| `internal/cfgutil/amount.go` | 44 | all |
| `internal/cfgutil/curve.go` | 123 | all |
| `internal/cfgutil/explicitflags.go` | 39 | all |
| `internal/cfgutil/file.go` | 22 | all |
| `internal/cfgutil/normalization.go` | 50 | all |
| `internal/loader/doc.go`, `loader.go`, `log.go` | 11, 415, 20 | all (lifecycle; the `wallet.Config` it builds is consumed by the wallet-core specs) |
| `version/version.go` | 113 | all |
| `internal/compat/compat.go` | 27 | all |
| `docs/README.md`, `docs/offline_wallets.md` | 2, 99 | all |
| `walletseed/seed.go`, `pgpwordlist/pgpwordlist.go` | 102, 60 | read in full because the seed prompt depends on them; the 512-entry word table in `pgpwordlist/wordlist.go` (544 lines) is data to copy verbatim |
| `rpcserver.go` | 512 | read in full for start order and IPC emission; its contract is spec 16, section 5.1 |

The only `*_test.go` files in the directories of this scope are
`walletseed/seed_test.go`, `pgpwordlist/pgpwordlist_test.go` and the
repository-root `x509_test.go` (package `main`, covers `rpcserver.go`; section
11).

**Parity pin**

| Component | Version read |
|---|---|
| dcrwallet | master `54b85b33`, version 2.2.0-pre, module `decred.org/dcrwallet/v5`, `go 1.26.0` |
| `github.com/jessevdk/go-flags` | v1.5.0 (`ini.go`, `parser.go`, `option.go`, `group.go`, `convert.go`, `help.go`, `error.go`, `multitag.go`, `optstyle_other.go`, `optstyle_windows.go`, `termsize.go`, `command.go:280-465`) |
| `github.com/decred/slog` | v1.2.0 (`log.go`) |
| `github.com/jrick/logrotate` | v1.0.0 (`rotator/rotator.go`) |
| `github.com/decred/dcrd/dcrutil/v4` | v4.0.3 (`appdata.go`, `amount.go`) |
| `github.com/decred/dcrd/chaincfg/v3` | v3.3.0 (network names, default P2P ports) |
| `github.com/decred/dcrd/wire` | v1.8.0 (`CurrencyNet` constants) |
| `github.com/decred/dcrd/hdkeychain/v3` | v3.1.3 (seed length constants) |
| `github.com/decred/dcrd/certgen` | v1.2.0 (`certgen.go`, key/cert PEM types only) |
| `github.com/decred/dcrd/connmgr/v3` | v3.1.4 (`tor.go:45`, signature only) |
| `github.com/decred/go-socks` | v1.1.0 (`socks/pool.go`) |
| `decred.org/cspp/v2` | v2.4.0 (`solverrpc/rpc.go`) |
| `golang.org/x/term` | v0.46.0 (`term_unix.go`, `terminal.go:1050-1084`) |

**Summary.** This subsystem is package `main` of the daemon: it turns argv and
`dcrwallet.conf` into one validated `config` value, initialises logging, runs
the interactive wallet-creation wizards, opens the wallet through the
`loader`, starts the RPC servers and the ticket buyer/mixer, selects exactly
one of three network backends (dcrd RPC, SPV, offline), and tears everything
down in a fixed order on SIGINT/SIGTERM/SIGHUP, a JSON-RPC `stop`, or the
closing of a parent-supplied pipe. Option parsing is go-flags v1.5.0 with an
INI file layer; 81 options in three groups (plus go-flags' built-in Help
Options group), no environment-variable options, no
hidden or deprecated options. The parent-process IPC is a 1-byte-versioned
framed message stream with three message types (`jsonrpclistener`,
`grpclistener`, `issuedclientcertificate`); there is **no** `--lifetimeevents`
option and no lifetime-event message in dcrwallet (that is dcrd only). Log
lines use decred/slog's fixed header and go to stdout and to a size-rotated,
gzip-archived `dcrwallet.log`. Exit status is 0 for clean shutdown, help,
version and successful creation, 1 for every configuration or startup failure.

## 2. Role in the system

- **What it does.** Owns process lifetime. `main` (`dcrwallet.go:45`) creates
  the shutdown-cancelled context, calls `run`, and maps the result to an exit
  status. `run` (`dcrwallet.go:78`) is the single place where services are
  wired together.
- **Who calls it.** The operating system (argv, signals, stdin/stdout/stderr),
  a supervising parent such as Decrediton (two anonymous pipes, see section
  4.2), and, indirectly, RPC clients: JSON-RPC `stop` reaches
  `requestShutdown` (`dcrwallet.go:402-406`), JSON-RPC `debuglevel` reaches
  `parseAndSetDebugLevels` through `rpcLoggers` (`rpcserver.go:233-246`), and
  the gRPC `WalletLoaderService` drives the `loader.Loader` built here.
- **What it calls.**
  - `internal/loader` to create/open/close `wallet.db` (bbolt driver `"bdb"`,
    spec 01) and build `wallet.Config` (specs 06-08).
  - `wallet.Wallet`: `Unlock`, `NeedsAccountsSync`, `WatchingOnly`,
    `AccountNumber`, `VSP`, `ProcessedTickets`, `SetNetworkBackend`,
    `UpgradeToSLIP0044CoinType`, `ImportXpubAccount`, `SetBirthState`.
  - `ticketbuyer.New(...).Run` (spec 11/13), `chain.NewSyncer(...).Run`
    (spec 15), `spv.NewSyncer(...).Run` with `p2p.NewLocalPeer` and
    `addrmgr.New` (spec 14).
  - `startRPCServers` (`rpcserver.go:248`, spec 16) and the gRPC
    `rpcserver.Start*Service` functions.
  - `solverrpc.StartSolver`/`Roots` to spawn and probe the external
    `csppsolver` process (spec 12 for its use during mixing).
- **Package-level state.** `cfg *config` (`dcrwallet.go:41-43`), `activeNet
  *netparams.Params` (`params.go:10`), `log` (`log.go:27`),
  `subsystemLoggers` (`log.go:47`), `shutdownRequestChannel`,
  `shutdownSignaled`, `signals` (`signal.go:15-24`), `outgoingPipeMessages`
  (`ipc.go:31`), and `errors.Separator = ":: "` set in `init`
  (`dcrwallet.go:36-39`), which makes every nested `errors.Error` render on
  one line throughout the process.

## 3. Public surface inventory

### 3.1 Package `main`

| Symbol | Signature / value | Contract |
|---|---|---|
| `main` | `func()` `dcrwallet.go:45` | `ctx := withShutdownCancel(Background)`; `go shutdownListener()`; `run(ctx)`; exit 1 iff the error is non-nil and not `context.Canceled`. |
| `run` | `func(ctx) error` `dcrwallet.go:78` | Section 5.3. |
| `done` | `func(ctx) bool` `dcrwallet.go:59` | Non-blocking test of `ctx.Done()`. |
| `zero` | `func([]byte)` `dcrwallet.go:68` | Overwrite with zero bytes. |
| `passPrompt` | `func(ctx, prefix string, confirm bool) ([]byte, error)` `dcrwallet.go:451` | `os.Stdout.Sync()`, run `prompt.PassPrompt` on a fresh `bufio.Reader(os.Stdin)` in a goroutine, return `ctx.Err()` if the context ends first (the goroutine is abandoned, still reading). |
| `startPromptPass` | `func(ctx, *wallet.Wallet) []byte` `dcrwallet.go:468` | Section 5.3.4. |
| `spvLoop` | `func(ctx, *wallet.Wallet)` `dcrwallet.go:531` | Section 5.5. |
| `rpcSyncLoop` | `func(ctx, *wallet.Wallet)` `dcrwallet.go:562` | Section 5.5. |
| `readCAFile` | `func() []byte` `dcrwallet.go:602` | Section 5.5. |
| `readClientCertKey` | `func() ([]byte, []byte)` `dcrwallet.go:621` | Section 5.5. |
| `config`, `ticketBuyerOptions`, `vspOptions` | structs `config.go:84-197` | Section 4.1. Unexported fields `dial`, `lookup`, `mixedAccount`, `mixedBranch` are derived (section 5.2). |
| `cleanAndExpandPath` | `func(string) string` `config.go:201` | Section 5.2.1. |
| `validLogLevel` | `func(string) bool` `config.go:254` | `slog.LevelFromString` succeeded. |
| `supportedSubsystems` | `func() []string` `config.go:261` | Keys of `subsystemLoggers`, sorted bytewise. |
| `parseAndSetDebugLevels` | `func(string) error` `config.go:276` | Section 5.2.2. Also the JSON-RPC `debuglevel` backend. |
| `loadConfig` | `func(ctx) (*config, []string, error)` `config.go:339` | Section 5.2. The returned remaining-args slice is ignored by `run`. |
| `networkDir` | `func(dataDir string, *chaincfg.Params) string` `walletsetup.go:34` | `Join(dataDir, name)` where name is `params.Name`, except `"testnet2"` for net `0x48e7a065` and `"testnet3"` for `wire.TestNet3`. |
| `displaySimnetMiningAddrs` | `func(seed []byte, imported bool) error` `walletsetup.go:49` | Section 5.4.5. |
| `createWallet` | `func(ctx, *config) error` `walletsetup.go:113` | Section 5.4.1. |
| `createSimulationWallet` | `func(ctx, *config) error` `walletsetup.go:270` | Section 5.4.3. |
| `promptHDPublicKey` | `func(*bufio.Reader) (string, error)` `walletsetup.go:315` | Print `Enter HD wallet public key: `, read one line, `TrimSpace`. Read error (including EOF) is returned. |
| `createWatchingOnlyWallet` | `func(ctx, *config) error` `walletsetup.go:328` | Section 5.4.2. |
| `checkCreateDir` | `func(path string) error` `walletsetup.go:371` | `Stat`; if absent `MkdirAll(path, 0700)`; errors `cannot create directory: %s`, `error checking directory: %s`, `path '%s' is not a directory`. |
| `pipeMessage` | interface `Type() string; PayloadSize() uint32; WritePayload(io.Writer) error` `ipc.go:25` | Section 4.2. |
| `outgoingPipeMessages` | `chan pipeMessage` (unbuffered) `ipc.go:31` | Single queue for all child-to-parent messages. Never closed. |
| `serviceControlPipeRx` | `func(fd uintptr)` `ipc.go:46` | Section 5.6. |
| `serviceControlPipeTx` | `func(fd uintptr)` `ipc.go:70` | Section 5.6. |
| `drainOutgoingPipeMessages` | `func()` `ipc.go:111` | Receive and discard forever. |
| `jsonrpcListenerEvent`, `grpcListenerEvent` | `string` types `ipc.go:124,155` | Types `"jsonrpclistener"`, `"grpclistener"`; payload is the address string bytes. |
| `issuedClientCertEvent` | `[]byte` type `ipc.go:179` | Type `"issuedclientcertificate"`; payload is the bytes. |
| `jsonrpcListenerEventServer`, `grpcListenerEventServer`, `issuedClientCertEventServer` | `chan<- pipeMessage` types with `notify` methods `ipc.go:135-203` | `notify` on a nil server is a no-op; otherwise a blocking send of one event. `issuedClientCertEventServer.notify(key, certChain...)` sends `key ‖ certChain[0] ‖ certChain[1] ‖ ...`. |
| `newJSONRPCListenerEventServer`, `newGRPCListenerEventServer`, `newIssuedClientCertEventServer` | `func(chan<- pipeMessage) <server type>` `ipc.go:137,168,190` | Type conversions of the channel; no state. Called from `startRPCServers` with `outgoingPipeMessages` (`rpcserver.go:251-254,306`). The two listener servers are created only under `--rpclistenerevents` and are otherwise nil (so `notify` is a no-op); the client-certificate server is created only under `--issueclientcert`. |
| `rpcLoggers` | `struct{}` with `Subsystems() []string`, `SetLevels(levelSpec string) error` `rpcserver.go:233-246` | The `jsonrpc.Options.Loggers` value (`rpcserver.go:381`). `Subsystems` returns the same sorted 13 tags as `supportedSubsystems`; `SetLevels` is `parseAndSetDebugLevels`. This is how the JSON-RPC `debuglevel` method (including `debuglevel show`) reaches this package. |
| `log` | `slog.Logger` = `loggers.MainLog` `log.go:27` | Subsystem `DCRW`. |
| `init` (`log.go:30-44`) | | Wires package loggers: `loader`→`LODR`; `wallet`, `wallet/udb`→`WLLT`; `ticketbuyer`→`TKBY`; `chain`, `spv`→`SYNC`; `p2p`→`PEER`; `internal/rpc/rpcserver`→`GRPC` (through the grpclog adapter, section 4.4); `internal/rpc/jsonrpc`→`RPCS`; dcrd `addrmgr`→`AMGR`; dcrd `connmgr`→`CMGR`; dcrd `mixing/mixpool`→`MIXP`. The mixing client line is commented out (`// XXX mixclient.UseLogger(loggers.MixcLog)`, `log.go:42`); `MIXC` is instead attached per client at `wallet/wallet.go:5540`. |
| `subsystemLoggers` | `map[string]slog.Logger` `log.go:47-61` | 13 entries, section 4.4. |
| `setLogLevel` | `func(subsystemID, logLevel string)` `log.go:66` | Unknown subsystem ignored; unparsable level silently becomes `info`. |
| `setLogLevels` | `func(logLevel string)` `log.go:81` | `setLogLevel` for every subsystem. |
| `fatalf` | `func(format, args...)` `log.go:91` | `log.Errorf`, `os.Stdout.Sync()`, `loggers.CloseLogRotator()`, `os.Exit(1)`. Only caller: profile server failure. |
| `shutdownRequestChannel` | `chan struct{}` (unbuffered) `signal.go:15` | Internal shutdown requests. |
| `shutdownSignaled` | `chan struct{}` `signal.go:20` | Closed once when shutdown starts. |
| `signals` | `[]os.Signal` `signal.go:24`, `signal_unix.go:15-21` | `{os.Interrupt}`; on darwin, dragonfly, freebsd, linux, netbsd, openbsd, solaris replaced by `{os.Interrupt, SIGTERM, SIGHUP}`. The replacement is selected by the build constraint `darwin \|\| dragonfly \|\| freebsd \|\| linux \|\| netbsd \|\| openbsd \|\| solaris` (`signal_unix.go:6`); every other target (windows, plan9, aix, js, wasip1) keeps `{os.Interrupt}` only, so SIGTERM and SIGHUP there take the Go runtime default (terminate without cleanup). |
| `withShutdownCancel` | `func(ctx) ctx` `signal.go:29` | Child context cancelled when `shutdownSignaled` closes. |
| `requestShutdown` | `func()` `signal.go:40` | Blocking send on `shutdownRequestChannel`. |
| `shutdownListener` | `func()` `signal.go:47` | Section 5.7. Never returns. |
| `activeNet` | `*netparams.Params` `params.go:10` | Defaults to `&netparams.MainNetParams`; reassigned by `loadConfig`. |
| `testStartedSolverWorks` | `func() error` `testsolver.go:18` | Section 5.2, step 22. |
| constants | `config.go:35-70` | Section 6. |
| default path variables | `config.go:72-82` | Section 4.1.3. |
| `debug.go` | `//go:build go1.25`, `//go:debug default=go1.25` | No code. Pins Go runtime GODEBUG defaults to the Go 1.25 set. |

### 3.2 Package `internal/cfgutil`

| Symbol | Signature | Contract |
|---|---|---|
| `ExplicitString` | `struct{ Value string; explicitlySet bool }` `explicitflags.go:14` | String option that remembers whether go-flags ever called `UnmarshalFlag` on it (command line **or** config file). |
| `NewExplicitString(default)` | `*ExplicitString` `:20` | `explicitlySet=false`. |
| `(*ExplicitString).ExplicitlySet()` | `bool` `:26` | |
| `(*ExplicitString).MarshalFlag()` | `(string, error)` `:29` | Returns `Value`. |
| `(*ExplicitString).UnmarshalFlag(v)` | `error` `:32` | `Value=v; explicitlySet=true`; never fails. No trimming. |
| `(*ExplicitString).String()` | `string` `:39` | `Value`. Used when the pointer is formatted with `%v`/`%s`. |
| `AmountFlag` | `struct{ dcrutil.Amount }` `amount.go:17` | DCR amount option. |
| `NewAmountFlag(a)` | `*AmountFlag` `:22` | |
| `(*AmountFlag).MarshalFlag()` | `(string, error)` `:27` | `Amount.String()`, for example `0.0001 DCR`. |
| `(*AmountFlag).UnmarshalFlag(v)` | `error` `:32` | Section 5.2.3. |
| `CurveID` | `int` `curve.go:19` | `CurveP256=0, CurveP384=1, CurveP521=2, Ed25519=3`; `PreferredCurve = Ed25519`. |
| `CurveFlag` | `struct{ curveID CurveID }` `:34` | |
| `NewCurveFlag(id)` | `*CurveFlag` `:39` | |
| `(*CurveFlag).ECDSACurve()` | `(elliptic.Curve, bool)` `:45` | P-256/P-384/P-521, else `(nil,false)`. |
| `(*CurveFlag).MarshalFlag()` | `(string, error)` `:59` | `P-256`, `P-384`, `P-521`, `Ed25519`; else `unknown curve ID %v`. |
| `(*CurveFlag).UnmarshalFlag(v)` | `error` `:76` | Exact, case-sensitive match of the four names; else `unrecognized curve %v`. |
| `(*CurveFlag).GenerateKeyPair(rand)` | `(pub, priv any, err)` `:92` | ECDSA key on the curve, or Ed25519 from a 32-byte seed read from `rand`; else `unknown curve ID`. |
| `(*CurveFlag).CertGen(org, validUntil, extraHosts)` | `(cert, key []byte, err)` `:115` | `certgen.NewTLSCertPair` or `certgen.NewEd25519TLSCertPair`. |
| `FileExists(path)` | `(bool, error)` `file.go:10` | `""` → `(false,nil)`; `Stat` ok → true; not-exist → false; other error returned. True for directories too. |
| `NormalizeAddress(addr, defaultPort)` | `(string, error)` `normalization.go:12` | Section 5.2.4. |
| `NormalizeAddresses(addrs, defaultPort)` | `([]string, error)` `:31` | Normalise each, drop later duplicates, keep first-seen order. Returns a non-nil empty slice for empty input. |
| `AddressFlag`, `NewAddressFlag`, `MarshalFlag`, `UnmarshalFlag`, `Address`, `StakeAddress` | `address.go:15-59` | **Dead code**: no option uses it at this pin. Stores the string; `Address` decodes with `stdaddr.DecodeAddress` (nil for empty); `StakeAddress` errors `address is not suitable for stake usage`. |

### 3.3 Package `internal/loggers`

| Symbol | Contract |
|---|---|
| `logWriter` (unexported) `loggers.go:18-27` | `Write(p)`: `os.Stdout.Write(p)`, then `logRotator.Write(p)` if a rotator exists; both errors ignored; always returns `(len(p), nil)`. |
| `backendLog` (unexported) `:40` | `slog.NewBackend(logWriter{})`. |
| `logRotator` (unexported) `:44` | `*rotator.Rotator`, nil until `InitLogRotator`. |
| `MainLog`, `LoaderLog`, `WalletLog`, `TkbyLog`, `SyncLog`, `PeerLog`, `GrpcLog`, `JsonrpcLog`, `CmgrLog`, `MixcLog`, `MixpLog`, `VspcLog`, `AmgrLog` `:46-58` | Tags `DCRW`, `LODR`, `WLLT`, `TKBY`, `SYNC`, `PEER`, `GRPC`, `RPCS`, `CMGR`, `MIXC`, `MIXP`, `VSPC`, `AMGR`. Each starts at level info. |
| `InitLogRotator(logFile string, logSize int64)` `:67` | `MkdirAll(dir(logFile), 0700)`; `rotator.New(logFile, logSize, false, 0)`. Either failure prints `failed to create log directory: %v\n` or `failed to create file rotator: %v\n` to stderr and exits 1. |
| `CloseLogRotator() error` `:85` | No-op if never initialised; else `Rotator.Close()` (closes the file, waits for pending compressions). |

### 3.4 Package `internal/netparams`

`Params` (`params.go:12`) embeds `*chaincfg.Params` and adds three decimal port
strings. Values in section 6.2. Exported variables `MainNetParams`,
`TestNet3Params`, `SimNetParams`.

### 3.5 Package `internal/prompt`

| Symbol | Signature | Contract |
|---|---|---|
| `ProvideSeed()` | `([]byte, error)` `prompt.go:29` | **Dead code.** Loops on `Enter existing wallet seed: `, lowercases and trims, hex-decodes, requires 16..64 bytes, else prints `Invalid seed specified.  Must be a hexadecimal value that is at least 128 bits and at most 512 bits\n`. |
| `ProvidePrivPassphrase()` | `([]byte, error)` `:56` | **Dead code.** Loops on `Enter the private passphrase of your wallet: ` using `term.ReadPassword`, prints `\n`, trims, repeats while empty. |
| `promptList` (unexported) | `(reader, prefix, validResponses, defaultEntry) (string, error)` `:77` | Section 5.4.6. |
| `promptListBool` (unexported) | `(reader, prefix, defaultEntry) (bool, error)` `:109` | `promptList` with `n/no/y/yes`; true for `y` or `yes`. |
| `PassPrompt` | `(reader *bufio.Reader, prefix string, confirm bool) ([]byte, error)` `:122` | Section 5.4.7. |
| `PrivatePass` | `(reader, configPass []byte) ([]byte, error)` `:177` | Section 5.4.8. |
| `PublicPass` | `(reader, privPass, defaultPubPassphrase, configPubPass []byte) ([]byte, error)` `:199` | Section 5.4.9. |
| `Seed` | `(reader) (seed []byte, imported bool, err error)` `:263` | Section 5.4.10. |
| `Birthday` | `(reader) (*time.Time, *uint32, error)` `:372` | Section 5.4.11. |
| `ImportedAccounts` | `(reader, *chaincfg.Params) (names []string, xpubs []*hdkeychain.ExtendedKey, err error)` `:412` | Section 5.4.12. |
| `collapseSpace` (unexported) | `(string) string` `:465` | Replace each run of `unicode.IsSpace` runes by one ASCII space. |

### 3.6 Package `internal/loader`

| Symbol | Signature | Contract |
|---|---|---|
| `Loader` | struct `loader.go:33` | One optional loaded wallet plus its DB handle, pending callbacks, and the option values copied into every `wallet.Config`. Guarded by one `sync.Mutex`. |
| `NewLoader` | `(chainParams, dbDirPath string, votingEnabled bool, gapLimit, watchLast uint32, allowHighFees bool, relayFee, vspMaxFee dcrutil.Amount, accountGapLimit int, disableCoinTypeUpgrades, mixingEnabled, manualTickets bool, mixSplitLimit int, dialer wallet.DialFunc) *Loader` `:57` | Stores arguments; no I/O. |
| `(*Loader).RunAfterLoad(fn)` | `:94` | If a wallet is loaded, call `fn(w)` **synchronously on the caller's goroutine** after releasing the mutex; else append to the callback list. |
| `(*Loader).CreateWatchingOnlyWallet(ctx, extendedPubKey, pubPass)` | `(*wallet.Wallet, error)` `:108` | Section 5.8. |
| `(*Loader).CreateNewWallet(ctx, pubPassphrase, privPassphrase, seed)` | `(*wallet.Wallet, error)` `:198` | Section 5.8. |
| `(*Loader).OpenExistingWallet(ctx, pubPassphrase)` | `(*wallet.Wallet, error)` `:288` | Section 5.8. |
| `(*Loader).DbDirPath()` | `string` `:345` | The network directory. No locking. |
| `(*Loader).WalletExists()` | `(bool, error)` `:351` | `Stat(dbDir/wallet.db)`; op `loader.WalletExists`. No locking. |
| `(*Loader).LoadedWallet()` | `(*wallet.Wallet, bool)` `:364` | |
| `(*Loader).UnloadWallet()` | `error` `:375` | Section 5.8. |
| `(*Loader).NetworkBackend()` | `(wallet.NetworkBackend, bool)` `:397` | Backend of the loaded wallet, `ok` iff non-nil. |
| `UseLogger(slog.Logger)` | `log.go:18` | Package logger, default `slog.Disabled`. |
| `walletDbName`, `driver` | `"wallet.db"`, `"bdb"` `loader.go:21-24` | |
| `(*Loader).onLoaded(w, db)` (unexported) | `loader.go:81` | Runs the queued callbacks in order, then stores `wallet` and `db` and sets the callback list to nil. Caller holds the mutex. |
| `fileExists(path)` (unexported) | `(bool, error)` `loader.go:406` | `os.Stat`; not-exist → `(false, nil)`; other error returned. Unlike `cfgutil.FileExists` it does not special-case the empty path. |
| `Loader` fields | `loader.go:33-54` | `callbacks []func(*wallet.Wallet)`, `chainParams`, `dbDirPath`, `wallet`, `db wallet.DB`, the twelve option copies (`votingEnabled`, `gapLimit`, `watchLast`, `accountGapLimit`, `disableCoinTypeUpgrades`, `mixingEnabled`, `allowHighFees`, `manualTickets`, `relayFee`, `vspMaxFee`, `mixSplitLimit`, `dialer`), `mu sync.Mutex`. The option copies are fixed at construction: nothing changes them afterwards, so every wallet opened through one `Loader` (including over gRPC under `--noinitialload`) gets the startup option values. |

The package imports `wallet/drivers/bdb` for its side effect of registering
the `"bdb"` walletdb driver (`internal/loader/loader.go:16`; `walletsetup.go:22`
does the same for package `main`).

### 3.7 Package `version`

| Symbol | Value / signature | Contract |
|---|---|---|
| `Major`, `Minor`, `Patch` | `2`, `2`, `0` `version.go:20-24` | |
| `Integer` | `1000000*Major + 10000*Minor + 100*Patch` = `2020000` `:27` | Reported by JSON-RPC `getinfo` (`internal/rpc/jsonrpc/methods.go:1792-1794`). |
| `PreRelease` | `var = "pre"` `:33` | Link-time overridable (`-X decred.org/dcrwallet/v5/version.PreRelease=...`). |
| `BuildMetadata` | `var = ""` `:38` | Link-time overridable; if empty at `init`, set to `vcsCommitID()`. |
| `vcsCommitID` (unexported) | `:46` | Section 5.9. |
| `String()` | `string` `:71` | Section 5.9. |
| `normalizeVerString` (unexported) | `:100` | Drop every rune not in `semverAlphabet`. |
| `semverAlphabet` | `0-9A-Za-z-.` `:17` | |

### 3.8 Package `internal/compat`

| Symbol | Contract |
|---|---|
| `HD2Address(k *hdkeychain.ExtendedKey, params) (*stdaddr.AddressPubKeyHashEcdsaSecp256k1V0, error)` `compat.go:10` | P2PKH (ECDSA secp256k1, script version 0) address of `Hash160(k.SerializedPubKey())`. |
| `IsEitherCoinBaseTx(tx *wire.MsgTx) bool` `compat.go:19` | `standalone.IsCoinBaseTx(tx, false) || standalone.IsCoinBaseTx(tx, true)`. |

Callers are in `wallet/` and `wallet/udb/`; nothing in package `main` uses them.

## 4. Data formats

### 4.1 Options

#### 4.1.1 Option table

Order is struct order (`config.go:84-197`), which is also help order. "INI
keys" lists every spelling the config file accepts for the option (section
4.1.4); the first is the long name, the second the Go field name, the third
the short name when there is one. All keys are case-sensitive. "Help default"
is the literal go-flags prints as `(default: X)`; a dash means no default
suffix is printed.

Group **Application Options** (INI section `[Application Options]`, or no
section header):

| # | Long | Short | Field (INI alt key) | Go type | Default | Help default | Description (exact) |
|---|---|---|---|---|---|---|---|
| 1 | `configfile` | `C` | `ConfigFile` | `*ExplicitString` | `<defaultAppData>/dcrwallet.conf` | that path | Path to configuration file |
| 2 | `version` | `V` | `ShowVersion` | bool | false | - | Display version information and exit |
| 3 | `create` | | `Create` | bool | false | - | Create new wallet |
| 4 | `createtemp` | | `CreateTemp` | bool | false | - | Create simulation wallet in nonstandard --appdata; private passphrase is 'password' |
| 5 | `createwatchingonly` | | `CreateWatchingOnly` | bool | false | - | Create watching wallet from account extended pubkey |
| 6 | `appdata` | `A` | `AppDataDir` | `*ExplicitString` | `<defaultAppData>` | that path | Application data directory for wallet config, databases and logs |
| 7 | `testnet` | | `TestNet` | bool | false | - | Use the test network |
| 8 | `simnet` | | `SimNet` | bool | false | - | Use the simulation test network |
| 9 | `noinitialload` | | `NoInitialLoad` | bool | false | - | Defer wallet creation/opening on startup and enable loading wallets over RPC |
| 10 | `debuglevel` | `d` | `DebugLevel` | string | `info` | `info` | Logging level {trace, debug, info, warn, error, critical} |
| 11 | `logdir` | | `LogDir` | `*ExplicitString` | `<defaultAppData>/logs` | that path | Directory to log output. |
| 12 | `logsize` | | `LogSize` | string | `10M` | `10M` | Maximum size of log file before it is rotated |
| 13 | `nofilelogging` | | `NoFileLogging` | bool | false | - | Disable file logging |
| 14 | `profile` | | `Profile` | `[]string` | nil | - | Enable HTTP profiling this interface/port |
| 15 | `memprofile` | | `MemProfile` | string | `""` | - | Write mem profile to the specified file |
| 16 | `cpuprofile` | | `CPUProfile` | string | `""` | - | Write cpu profile to the specified file |
| 17 | `walletpass` | | `WalletPass` | string | `public` | - (`default-mask:"-"`) | Public wallet password; required when created with one |
| 18 | `promptpass` | | `PromptPass` | bool | false | - | Prompt for private passphase from terminal and unlock without timeout |
| 19 | `pass` | | `Pass` | string | `""` | - | Unlock with private passphrase |
| 20 | `promptpublicpass` | | `PromptPublicPass` | bool | false | - | Prompt for public passphrase from terminal |
| 21 | `enableticketbuyer` | | `EnableTicketBuyer` | bool | false | - | Enable the automatic ticket buyer |
| 22 | `enablevoting` | | `EnableVoting` | bool | false | - | Automatically vote on winning tickets |
| 23 | `purchaseaccount` | | `PurchaseAccount` | string | `default` | `default` | Account to autobuy tickets from |
| 24 | `gaplimit` | | `GapLimit` | uint32 | 20 | `20` | Allowed unused address gap between used addresses of accounts |
| 25 | `watchlast` | | `WatchLast` | uint32 | 0 | - | Limit watched previous addresses of each HD account branch |
| 26 | `manualtickets` | | `ManualTickets` | bool | false | - | Do not discover new tickets through network synchronization |
| 27 | `allowhighfees` | | `AllowHighFees` | bool | false | - | Do not perform high fee checks |
| 28 | `txfee` | | `RelayFee` | `*AmountFlag` | 10000 atoms | `0.0001 DCR` | Transaction fee per kilobyte |
| 29 | `accountgaplimit` | | `AccountGapLimit` | int | 10 | `10` | Allowed gap of unused accounts |
| 30 | `disablecointypeupgrades` | | `DisableCoinTypeUpgrades` | bool | false | - | Never upgrade from legacy to SLIP0044 coin type keys |
| 31 | `rpcconnect` | `c` | `RPCConnect` | string | `""` (becomes `localhost:<port>`) | - | Network address of dcrd RPC server |
| 32 | `cafile` | | `CAFile` | `*ExplicitString` | `""` (derived, section 5.2 step 26) | - | dcrd RPC Certificate Authority |
| 33 | `clientcafile` | | `ClientCAFile` | `*ExplicitString` | `<defaultAppData>/clients.pem` | that path | Certficate Authority to verify TLS client certificates |
| 34 | `noclienttls` | | `DisableClientTLS` | bool | false | - | Disable TLS for dcrd RPC; only allowed when connecting to localhost |
| 35 | `dcrdusername` | | `DcrdUsername` | string | `""` | - | dcrd RPC username; overrides --username |
| 36 | `dcrdpassword` | | `DcrdPassword` | string | `""` | - (mask) | dcrd RPC password; overrides --password |
| 37 | `dcrdclientcert` | | `DcrdClientCert` | `*ExplicitString` | `<defaultAppData>/dcrd-client.cert` | that path | TLS client certificate to present to authenticate RPC connections to dcrd |
| 38 | `dcrdclientkey` | | `DcrdClientKey` | `*ExplicitString` | `<defaultAppData>/dcrd-client.key` | that path | Key for dcrd RPC client certificate |
| 39 | `dcrdauthtype` | | `DcrdAuthType` | string | `basic` | `basic` | Method for dcrd JSON-RPC client authentication (basic or clientcert) |
| 40 | `proxy` | | `Proxy` | string | `""` | - | Establish network connections and DNS lookups through a SOCKS5 proxy (e.g. 127.0.0.1:9050) |
| 41 | `proxyuser` | | `ProxyUser` | string | `""` | - | Proxy server username |
| 42 | `proxypass` | | `ProxyPass` | string | `""` | - (mask) | Proxy server password |
| 43 | `circuitlimit` | | `CircuitLimit` | int | 32 | `32` | Set maximum number of open Tor circuits; used only when --torisolation is enabled |
| 44 | `torisolation` | | `TorIsolation` | bool | false | - | Enable Tor stream isolation by randomizing user credentials for each connection |
| 45 | `nodcrdproxy` | | `NoDcrdProxy` | bool | false | - | Never use configured proxy to dial dcrd websocket connectons |
| 46 | `offline` | | `Offline` | bool | false | - | Do not sync the wallet |
| 47 | `spv` | | `SPV` | bool | false | - | Sync using simplified payment verification |
| 48 | `spvconnect` | | `SPVConnect` | `[]string` | nil | - | SPV sync only with specified peers; disables DNS seeding |
| 49 | `spvdisablerelaytx` | | `SPVDisableRelayTx` | bool | false | - | Disable receiving mempool transactions when in SPV mode |
| 50 | `rpccert` | | `RPCCert` | `*ExplicitString` | `<defaultAppData>/rpc.cert` | that path | RPC server TLS certificate |
| 51 | `rpckey` | | `RPCKey` | `*ExplicitString` | `<defaultAppData>/rpc.key` | that path | RPC server TLS key |
| 52 | `tlscurve` | | `TLSCurve` | `*CurveFlag` | Ed25519 | `Ed25519` | Curve to use when generating TLS keypairs |
| 53 | `onetimetlskey` | | `OneTimeTLSKey` | bool | false | - | Generate self-signed TLS keypairs each startup; only write certificate file |
| 54 | `noservertls` | | `DisableServerTLS` | bool | false | - | Disable TLS for the RPC servers; only allowed when binding to localhost |
| 55 | `grpclisten` | | `GRPCListeners` | `[]string` | nil (derived) | - | Listen for gRPC connections on this interface |
| 56 | `rpclisten` | | `LegacyRPCListeners` | `[]string` | nil (derived) | - | Listen for JSON-RPC connections on this interface |
| 57 | `nogrpc` | | `NoGRPC` | bool | false | - | Disable gRPC server |
| 58 | `nolegacyrpc` | | `NoLegacyRPC` | bool | false | - | Disable JSON-RPC server |
| 59 | `rpcmaxclients` | | `LegacyRPCMaxClients` | int64 | 10 | `10` | Max JSON-RPC HTTP POST clients |
| 60 | `rpcmaxwebsockets` | | `LegacyRPCMaxWebsockets` | int64 | 25 | `25` | Max JSON-RPC websocket clients |
| 61 | `username` | `u` | `Username` | string | `""` | - | JSON-RPC username and default dcrd RPC username |
| 62 | `password` | `P` | `Password` | string | `""` | - (mask) | JSON-RPC password and default dcrd RPC password |
| 63 | `jsonrpcauthtype` | | `JSONRPCAuthType` | string | `basic` | `basic` | Method for JSON-RPC client authentication (basic or clientcert) |
| 64 | `pipetx` | | `PipeTx` | `*uint` | nil | - | File descriptor or handle of write end pipe to enable child -> parent process communication |
| 65 | `piperx` | | `PipeRx` | `*uint` | nil | - | File descriptor or handle of read end pipe to enable parent -> child process communication |
| 66 | `rpclistenerevents` | | `RPCListenerEvents` | bool | false | - | Notify JSON-RPC and gRPC listener addresses over the TX pipe |
| 67 | `issueclientcert` | | `IssueClientCert` | bool | false | - | Notify a client cert and key over the TX pipe for RPC authentication |
| 68 | `mixing` | | `MixingEnabled` | bool | false | - | Enable creation of mixed transactions and participation in the peer-to-peer mixing network |
| 69 | `csppsolver` | | `CSPPSolver` | `*ExplicitString` | `csppsolver` | `csppsolver` | Path to CSPP solver executable (if not in PATH) |
| 70 | `mixedaccount` | | `MixedAccount` | string | `""` | - | Account/branch used to derive CoinShuffle++ mixed outputs and voting rewards |
| 71 | `ticketsplitaccount` | | `TicketSplitAccount` | string | `""` (becomes the mixed account name) | - | Account to derive fresh addresses from for mixed ticket splits; uses mixedaccount if unset |
| 72 | `changeaccount` | | `ChangeAccount` | string | `""` | - | Account to send change when ticket autobuying or mixing; required when mixing |
| 73 | `mixchange` | | `MixChange` | bool | false | - | Use CoinShuffle++ to mix change account outputs into mix account |
| 74 | `mixsplitlimit` | | `MixSplitLimit` | int | 10 | `10` | Maximum concurrent mixes for any change amount |

Group **Ticket Buyer Options** (namespace `ticketbuyer`, INI section `[Ticket
Buyer Options]`):

| # | Long | Field (INI alt key) | Go type | Default | Help default | Description |
|---|---|---|---|---|---|---|
| 75 | `ticketbuyer.balancetomaintainabsolute` | `BalanceToMaintainAbsolute` | `*AmountFlag` | 0 | `0 DCR` | Amount of funds to keep in wallet when purchasing tickets |
| 76 | `ticketbuyer.limit` | `Limit` | uint | 1 | `1` | Buy no more than specified number of tickets per block (0 for no limit) |
| 77 | `ticketbuyer.votingaccount` | `VotingAccount` | string | `""` | - | Account used to derive addresses specifying voting rights (mandatory if mixing is enabled) |

Group **VSP Options** (namespace `vsp`, INI section `[VSP Options]`):

| # | Long | Field (INI alt key) | Go type | Default | Help default | Description |
|---|---|---|---|---|---|---|
| 78 | `vsp.url` | `URL` | string | `""` | - | Base URL of the VSP server |
| 79 | `vsp.pubkey` | `PubKey` | string | `""` | - | Base64 encoded public key of the VSP server. Can be found in the footer of the VSP website |
| 80 | `vsp.sync` | `Sync` | bool | false | - | Sync tickets to VSP |
| 81 | `vsp.maxfee` | `MaxFee` | `*AmountFlag` | 20000000 atoms | `0.2 DCR` | Maximum VSP fee |

Group **Help Options** (added by go-flags, command line only): `-h`,
`--help`, "Show this help message"; on Windows additionally `/?`
(`go-flags@v1.5.0/optstyle_other.go:57`, `optstyle_windows.go:93`).

Facts that hold for every option:

- No option has an `env`, `default`, `choice`, `hidden`, `ini-name`, `no-ini`,
  `required`, `optional` or `value-name` tag. Defaults come only from the
  struct literal at `config.go:345-389`.
- No option is deprecated or hidden at this pin. Any name not in the table is
  an error on the command line (``unknown flag `name'``) and in the config
  file (`unknown option: name`), so a configuration file carried over from an
  older release that still contains a removed option stops startup.
- Integer conversion uses base 10 only (`go-flags@v1.5.0/convert.go:223,237`):
  `0x10`, `0o7`, and digit separators are rejected. Widths: `uint32` for
  options 24-25, platform `int` for 29, 43, 74, `int64` for 59-60, platform
  `uint` for 64, 65, 76.
- Booleans on the command line never take an argument: `--testnet=1` fails
  with ``bool flag `--testnet' cannot have an argument``
  (`parser.go:522-526`). In the config file a boolean takes an empty value
  (true) or anything `strconv.ParseBool` accepts: `1 t T TRUE true True 0 f F
  FALSE false False` (`convert.go:210-221`).

#### 4.1.2 Value syntax for the custom types

| Type | Accepted text | Error text (the `%s` tail of the go-flags message) |
|---|---|---|
| `*ExplicitString` | any string, stored verbatim | never fails |
| `*AmountFlag` | optional suffix ` DCR` removed, then `strconv.ParseFloat(_, 64)`, then `round(f * 1e8)` half away from zero | `strconv.ParseFloat: parsing "X": invalid syntax` (or `value out of range`); `invalid coin amount` for NaN and ±Inf |
| `*CurveFlag` | exactly `P-256`, `P-384`, `P-521`, `Ed25519` | `unrecognized curve X` |
| `*uint` | base-10 unsigned integer | `strconv.ParseUint: parsing "X": invalid syntax` |

On the command line a conversion failure is reported as
``invalid argument for flag `<opt>' (expected <GoType>): <tail>`` where
`<opt>` is `-C, --configfile` style for options with a short name and
`--name` otherwise, and `<GoType>` is the reflected type string: `string`,
`uint32`, `int`, `int64`, `uint`, `*uint`, `[]string`,
`*cfgutil.ExplicitString`, `*cfgutil.AmountFlag`, `*cfgutil.CurveFlag`
(`parser.go:574-598`). In the config file the same failure is
`<file>:<line>: <tail>` with no flag name (`ini.go:586-596`).

#### 4.1.3 Default directories and files

`defaultAppData = dcrutil.AppDataDir("dcrwallet", false)`
(`config.go:74`, `dcrd/dcrutil/v4@v4.0.3/appdata.go:21-74`):

| OS (`runtime.GOOS`) | Application data directory | Fallback |
|---|---|---|
| windows | `%LOCALAPPDATA%\Dcrwallet`; if `LOCALAPPDATA` is empty, `%APPDATA%\Dcrwallet` | `.` when both are empty |
| darwin | `$HOME/Library/Application Support/Dcrwallet` | `.` when the home directory is unknown |
| plan9 | `$home/dcrwallet` | `.` |
| everything else | `$HOME/.dcrwallet` | `.` |

"Home directory" is Go's `os.UserHomeDir()` (`$HOME` on Unix, `%USERPROFILE%`
on Windows, `$home` on Plan 9); an error yields the `.` fallback.

Paths computed once at package initialisation (`config.go:72-82`), before
argv is read:

| Variable | Value |
|---|---|
| `dcrdDefaultCAFile` | `AppDataDir("dcrd", false)/rpc.cert` (for example `~/.dcrd/rpc.cert`, `%LOCALAPPDATA%\Dcrd\rpc.cert`) |
| `defaultConfigFile` | `<defaultAppData>/dcrwallet.conf` |
| `defaultRPCKeyFile` | `<defaultAppData>/rpc.key` |
| `defaultRPCCertFile` | `<defaultAppData>/rpc.cert` |
| `defaultDcrdClientCertFile` | `<defaultAppData>/dcrd-client.cert` |
| `defaultDcrdClientKeyFile` | `<defaultAppData>/dcrd-client.key` |
| `defaultRPCClientCAFile` | `<defaultAppData>/clients.pem` |
| `defaultLogDir` | `<defaultAppData>/logs` |

Resulting on-disk layout for an application data directory `A` and network
directory name `N` (`mainnet`, `testnet3`, `simnet`):

| Path | Created by | Mode | Content |
|---|---|---|---|
| `A/dcrwallet.conf` | the operator (never written by the daemon) | - | section 4.1.4 |
| `A/N/` | `checkCreateDir` (creation flows) or `loader` | 0700 | network directory |
| `A/N/wallet.db` | bbolt driver | spec 01 | wallet database |
| `A/N/seed` | `--createtemp` only | 0644 | 33 PGP words separated by single spaces, no trailing newline |
| `A/N/peers.json` | dcrd `addrmgr` in SPV mode (directory passed at `dcrwallet.go:533-534`) | spec 14 | peer address book |
| `A/logs/N/dcrwallet.log` | `InitLogRotator` | dir 0700, file 0644 | section 4.3 |
| `A/logs/N/dcrwallet.log.<n>.gz` | log rotation | 0644 | gzip of a rotated log |
| `A/rpc.cert`, `A/rpc.key` | `generateRPCKeyPair` (spec 16) | 0600 | PEM |
| `A/clients.pem` | the operator | - | PEM CA bundle for client certificates |
| `A/dcrd-client.cert`, `A/dcrd-client.key` | the operator | - | PEM, used when `dcrdauthtype=clientcert` |
| `A/dcrd.cert` | the operator | - | copy of dcrd's RPC certificate; preferred CA file |
| `A/testnet/` | releases that predate testnet3 | - | only tested for existence, to print a warning |

The TLS and CA files live in `A`, not in `A/N`; they are shared by all
networks.

#### 4.1.4 Configuration file grammar

The file is parsed by `flags.NewIniParser(parser).ParseFile(path)`
(`config.go:428`, `go-flags@v1.5.0/ini.go:374-615`).

Lexical rules (`ini.go:374-477`):

1. The file is read line by line. A line terminator is `\n` or `\r\n`; a
   final line without a terminator is processed.
2. Each line is trimmed of leading and trailing Unicode white space.
3. Empty lines and lines whose first byte is `;` or `#` are ignored. There
   are no trailing comments: `testnet=1 ; x` gives the value `1 ; x`.
4. A line whose first byte is `[` is a section header. Its last byte must be
   `]`, otherwise the error is `<file>:<line>: malformed section header`. The
   name is the text between the brackets, trimmed; an empty name is
   `<file>:<line>: empty section name`. A repeated section name continues the
   earlier section.
5. Any other line must contain `=`; it is split at the first `=` into a key
   and a value, each trimmed. No `=` is
   `<file>:<line>: malformed key=value (<line text>)`.
6. If the value begins with `"`, it must be a valid Go interpreted string
   literal and is replaced by its unquoted value
   (`strconv.Unquote`); failure is `<file>:<line>: invalid syntax`. Values
   that do not begin with `"` are taken verbatim, including embedded quotes.

Semantic rules (`ini.go:479-615`, `group.go:96-110,147-176`):

7. Keys before any section header belong to the unnamed section, which is
   searched across every group (Application Options, then Ticket Buyer
   Options, then VSP Options).
8. A named section is matched case-insensitively against the group names
   `Application Options`, `Ticket Buyer Options`, `VSP Options`. Any other
   name fails with ``could not find option group `<name>'`` (no file or line
   prefix). `[Help Options]` is not a valid section.
9. `[Application Options]` also reaches the two nested groups, so
   `ticketbuyer.limit=2` and `vsp.url=...` are valid there. `[Ticket Buyer
   Options]` reaches only options 75-77 and `[VSP Options]` only 78-81.
10. A key matches an option when it equals, case-sensitively, the Go field
    name, or the namespaced long name, or the one-character short name, in
    that priority. Examples that are all accepted: `testnet=1`, `TestNet=1`,
    `C=/path` (ignored in effect), `ticketbuyer.limit=2`, `Limit=2`,
    `vsp.maxfee=0.1`, `MaxFee=0.1`. Rejected: `Testnet=1`, `limit=2`,
    `maxfee=0.1`. No match is `<file>:<line>: unknown option: <key>`. One
    exception: a line with an **empty key** (`=value`) matches the first
    option of the first group searched (`configfile` in the unnamed section
    and in `[Application Options]`, `ticketbuyer.balancetomaintainabsolute`
    in `[Ticket Buyer Options]`, `vsp.url` in `[VSP Options]`), because the
    absent `ini-name` tag compares equal to the empty key
    (`ini.go:523-525`, `group.go:153-156`).
11. A scalar option given more than once keeps the last value. A `[]string`
    option accumulates every occurrence in file order. An empty value for a
    `[]string` option appends the empty string (`rpclisten=` therefore means
    "all interfaces, default port").
12. Sections are applied in Go map iteration order, which is random
    (`ini.go:508`). This is observable only when the same option is assigned
    in two different sections, or when two sections both contain an error
    (which error is reported is then random).

`sample-dcrwallet.conf` is a commented template shipped with the source. It is
not read or written by the daemon. Its disagreements with the code are listed
in section 10.

Shape of the sample (264 lines): line 1 is `[Application Options]`; every
other non-blank line up to line 231 is a `;` comment; line 233 is `[Ticket
Buyer Options]` and line 248 is `[VSP Options]`, each followed only by
comments. As shipped it therefore sets nothing, and copying it verbatim to
`dcrwallet.conf` is a no-op. Example values it shows: `rpcconnect=localhost:9109`,
`cafile=~/.dcrwallet/dcrd.cert`, `appdata=~/.dcrwallet`, `txfee=0.0001`,
`gaplimit=20`, `accountgaplimit=10`, `proxy=127.0.0.1:9050`, `circuitlimit=32`,
`rpccert=~/.dcrwallet/rpc.cert`, `rpckey=~/.dcrwallet/rpc.key`,
`tlscurve=P-256`, the twelve `rpclisten=` forms of lines 139-162,
`profile=:6062`, `debuglevel=info`, `ticketbuyer.balancetomaintainabsolute=0`,
`ticketbuyer.limit=1`, `vsp.url=https://teststakepool.decred.org`,
`vsp.pubkey=ia9Ra2Drb+OHLqRyBsJnRKBd7TUG1IvrseC6robKzGo=`, `vsp.sync=1`,
`vsp.maxfee=0.2`. Booleans are shown as `=0`/`=1`.

The sample documents 46 of the 81 options. The 35 it does not mention as a
key (derived by matching every `long:` tag in `config.go:84-197` against the
commented keys): `configfile`, `version`, `create`, `createtemp`,
`createwatchingonly`, `noinitialload`, `logdir`, `logsize`, `nofilelogging`,
`memprofile`, `cpuprofile`, `walletpass`, `promptpass`, `promptpublicpass`,
`enableticketbuyer`, `purchaseaccount`, `watchlast`, `manualtickets`,
`allowhighfees`, `clientcafile`, `noclienttls`, `dcrdclientcert`,
`dcrdclientkey`, `dcrdauthtype`, `noservertls`, `grpclisten` (named in prose
at lines 132-134 only), `rpcmaxclients`, `rpcmaxwebsockets`,
`jsonrpcauthtype`, `pipetx`, `piperx`, `rpclistenerevents`,
`issueclientcert`, `csppsolver`, `mixsplitlimit`. All of them are nevertheless
valid config-file keys: go-flags makes no distinction between "command-line
only" and "file" options, so `create=1`, `version=1` or `pipetx=3` in the file
behave exactly like the flags (`version=1` in the file is ignored, because
only the pre-parse copy is tested at `config.go:410`; `create=1` in the file
runs the creation wizard on every start until the database exists and then
makes every start fail with "already exists").

#### 4.1.5 Command-line grammar

go-flags v1.5.0, parser options `HelpFlag | PrintErrors | PassDoubleDash`
(`config.go:394,418`, `parser.go:118`). POSIX build (`optstyle_other.go`):

- `--name`, `--name=value`, `--name value`; `-x`, `-xvalue`, `-x=value`,
  `-x value`; clustered short flags `-ab`.
- An argument is an option if it is `-` followed by a non-`-` byte, or `--`
  followed by a non-`-` byte (`optstyle_other.go:19-29`). A bare `-` and
  anything not starting with `-` is a positional argument.
- `--` ends option parsing; everything after it is positional.
- Positional arguments are collected and returned by `loadConfig` and then
  ignored by `run`. `dcrwallet foo bar` starts normally.
- A value supplied as a separate argument may not itself look like an option
  (``expected argument for flag `--pass', but got option `-x'``) unless the
  option's kind is a signed integer or float (also through a pointer or
  slice; here only the `int`/`int64` options 29, 43, 59, 60, 74; not the
  `*AmountFlag` options) and the value is `-` followed by a digit
  (`option.go:459-472,561-569`). Use `--pass=-x`, `--txfee=-1`.
- A value that begins with `"` is passed through `strconv.Unquote`
  (`parser.go:544-546`, `convert.go:351-357`); failure is reported as
  ``invalid argument for flag `--pass' (expected string): invalid syntax``.
- A repeated scalar option keeps the last value. For `[]string` options the
  first command-line occurrence **discards** all values that came from the
  config file, then occurrences accumulate (`option.go:242-251`,
  `parser.go:210-213`).
- Unknown option: ``unknown flag `name'``. Missing value:
  ``expected argument for flag `--name'``.
- If the environment variable `GO_FLAGS_COMPLETION` is non-empty the first
  parse prints shell completions and exits 0 (`parser.go:220-234`).

Windows build (`optstyle_windows.go`): additionally `/name`, `/name:value`,
`/x`; an argument starting with `/` and longer than one byte is an option
(so a positional path beginning with `/` is misread as an option); option
names in error messages and help use `/` and `:` (`/C, /configfile`).

Precedence, lowest to highest: built-in defaults, config file, command line.

#### 4.1.6 Help output

`-h`/`--help` during the first parse makes go-flags print the help text to
**stdout** followed by one extra newline, and `loadConfig` exits 0
(`config.go:396-400`, `parser.go:702-714`). On any other first-parse error
go-flags prints the error line to stderr and `loadConfig` then writes the
same help text to **stderr** (`config.go:401`).

Layout (`help.go:280-490`):

```
Usage:
  <name> [OPTIONS]

Application Options:
  -C, --configfile=                            <description> (default: <path>)
      --create                                 <description>
  ...

Ticket Buyer Options:
      --ticketbuyer.balancetomaintainabsolute= <description> (default: 0 DCR)
  ...

VSP Options:
  ...

Help Options:
  -h, --help                                   Show this help message
```

(Padding in this sketch is exact: descriptions begin at byte offset 47.)

- `<name>` is `path.Base(os.Args[0])` (slash-separated `path`, not
  `filepath`, so on Windows a backslash path is printed whole).
- Each option line starts with two spaces, then `-X, ` or four spaces, then
  `--long`, then `=` for every non-boolean option.
- The description column starts at byte offset 47: longest namespaced long
  name 37 (`ticketbuyer.balancetomaintainabsolute`) + 2 + 2 (short options
  exist) + 4 + 2 (`help.go:30-46,188`).
- Descriptions are suffixed with ` (default: <literal>)` when the "Help
  default" column above is not a dash, then wrapped at spaces to
  `columns - 47` bytes (minimum 10), continuation lines indented 47 spaces
  (`help.go:110-164`). `columns` is the terminal width of file descriptor 0
  from `TIOCGWINSZ`, or 80 when that fails or reports 0
  (`termsize.go:9-15`, `help.go:68-70`).

The exact bytes for 80 columns should be captured once from the Go binary and
kept as a golden file (section 11).

### 4.2 Parent-process pipe protocol

Two independent, unidirectional byte streams, each on a file descriptor
(Unix) or handle value (Windows) inherited from the parent and named by a
decimal number on the command line.

#### 4.2.1 Child to parent (`--pipetx`)

A concatenation of messages. Each message (`ipc.go:18-29,77-105`):

| Offset | Size | Field | Encoding |
|---|---|---|---|
| 0 | 1 | protocol version | `0x01` |
| 1 | 1 | type length `T` | `byte(len(type))` |
| 2 | `T` | type | ASCII, not NUL-terminated |
| 2+T | 4 | payload length `P` | unsigned 32-bit little endian |
| 6+T | `P` | payload | type-specific |

Each message is written as header, payload, then one flush of the buffered
writer, so a message normally arrives in one `write(2)` unless it exceeds the
4096-byte `bufio.Writer` buffer.

Message types:

| Type string | `T` | Enabled by | Payload | Emitted |
|---|---|---|---|---|
| `jsonrpclistener` | 15 | `--rpclistenerevents` | listener address as `net.Listener.Addr().String()`: `127.0.0.1:9110`, `[::1]:9110`; with port 0 the kernel-assigned port | once per JSON-RPC listener, synchronously, in listener order (all IPv4 listeners in configuration order, then all IPv6), after the JSON-RPC server is constructed (`rpcserver.go:384-386`) |
| `grpclistener` | 12 | `--rpclistenerevents` | listener address string | once per gRPC listener, from that listener's own goroutine, immediately before `Serve` (`rpcserver.go:338-346`); order among gRPC listeners and relative to `jsonrpclistener` messages is not deterministic |
| `issuedclientcertificate` | 23 | `--issueclientcert` and server TLS enabled | three PEM blocks concatenated with no separator: `PRIVATE KEY` (PKCS#8 of the client key), `CERTIFICATE` (client certificate), `CERTIFICATE` (ephemeral CA) (`ipc.go:194-203`, `rpcserver.go:291-308`) | once, before any listener is opened, therefore always before every listener message |

Example, `jsonrpclistener` for `127.0.0.1:9110` (hex):

```
01 0f 6a 73 6f 6e 72 70 63 6c 69 73 74 65 6e 65 72   ..jsonrpclistener
0e 00 00 00                                          payload length 14
31 32 37 2e 30 2e 30 2e 31 3a 39 31 31 30            127.0.0.1:9110
```

There is no message for "startup complete", "wallet opened" or "shutting
down". `--rpclistenerevents` and `--issueclientcert` without `--pipetx` are
accepted; the messages are generated and discarded.

#### 4.2.2 Parent to child (`--piperx`)

No messages are defined. The child reads and discards all bytes. End of file
or any read error requests process shutdown (section 5.6). The stream exists
so that the child stops when the parent dies or closes its end.

### 4.3 Log output

Every log record is one line, written to stdout and, unless
`--nofilelogging`, appended to the log file
(`slog@v1.2.0/log.go:217-250,282-340`):

```
YYYY-MM-DD hh:mm:ss.mmm [LVL] TAG: message\n
```

| Part | Format |
|---|---|
| date/time | local time zone, zero-padded, milliseconds truncated (not rounded) |
| `LVL` | `TRC`, `DBG`, `INF`, `WRN`, `ERR`, `CRT` |
| `TAG` | subsystem tag, section 4.4 |
| message | `Sprintf(format, args...)` for the `f` methods; `Sprintln(args...)` (space-separated operands) for the others |

The environment variable `LOGFLAGS`, read once at process start
(`slog@v1.2.0/log.go:72-88`), is a comma-separated list that alters the
header: `UTC` (timestamps in UTC), `nodatetime` (omit the timestamp and its
trailing space), `longfile` / `shortfile` (insert ` <file>:<line>` between the
tag and the colon; Go source locations, not reproducible in a port).

Log file rotation (`logrotate@v1.0.0/rotator/rotator.go`):

1. The file is opened `O_CREATE|O_APPEND|O_RDWR`, mode 0644; the running size
   starts at the existing file size.
2. Threshold in bytes = `1000 * logSizeKiB` (`rotator.go:75`), where
   `logSizeKiB` comes from `--logsize` (section 5.2 step 10). The default
   `10M` is 10,240 KiB and therefore 10,240,000 bytes.
3. After each write, if the running size is at or above the threshold and the
   write ended with `\n`, rotate: list `<file>.*`; take the highest integer
   `n` found as the last dot-separated component (or the one before a final
   `gz`); close the file; rename it to `<file>.<n+1>`; open a new `<file>`
   with `O_CREATE|O_RDWR`, mode 0644; reset the size to 0; start a goroutine
   that writes `<file>.<n+1>.gz` (gzip, default level, `O_EXCL`, mode 0644)
   and on success removes `<file>.<n+1>`.
4. `maxRolls` is 0: archives are never deleted.
5. A rotation error is returned from `Rotator.Write`, which `logWriter`
   ignores; the rotator may then hold a closed file and later file writes
   fail silently while stdout logging continues.

### 4.4 Logging subsystems

| Tag | Logger variable | Wired to (`log.go:30-44` unless noted) |
|---|---|---|
| `AMGR` | `AmgrLog` | dcrd `addrmgr` |
| `CMGR` | `CmgrLog` | dcrd `connmgr` |
| `DCRW` | `MainLog` | package `main` |
| `GRPC` | `GrpcLog` | `internal/rpc/rpcserver` and the gRPC interceptors |
| `LODR` | `LoaderLog` | `internal/loader` |
| `MIXC` | `MixcLog` | mixing client, set at `wallet/wallet.go:5540` |
| `MIXP` | `MixpLog` | dcrd `mixing/mixpool` |
| `PEER` | `PeerLog` | `p2p` |
| `RPCS` | `JsonrpcLog` | `internal/rpc/jsonrpc` |
| `SYNC` | `SyncLog` | `chain` and `spv` |
| `TKBY` | `TkbyLog` | `ticketbuyer` |
| `VSPC` | `VspcLog` | VSP client, passed at `wallet/vspclientloader.go:20` |
| `WLLT` | `WalletLog` | `wallet` and `wallet/udb` |

The table is in the sorted order printed by `--debuglevel=show`.

`GRPC` receives two kinds of records. The interceptors in `rpcserver.go:406-448`
log through `loggers.GrpcLog` directly (`Unary method %s invoked by %s`,
`Streaming method %s invoked by %s` at debug; `... errored: %v` at error, or
at debug for a client-cancelled stream). The gRPC library's own log output is
routed through an adapter installed by `rpcserver.UseLogger`
(`internal/rpc/rpcserver/log.go:27-81`, `grpc@v1.83.2/grpclog/logger.go:32-34`,
`grpc@v1.83.2/grpclog/internal/logger.go:39-81`): library Info, Warning and
Error records are all emitted at `INF`; a leading `grpc: ` is removed from the
format string (or from the first argument when it is a string); library Fatal
records are emitted at `CRT` and followed by `os.Exit(1)` without closing the
log rotator or running any deferred shutdown action. A port that does not use
grpc-go has no equivalent library chatter; the interceptor lines are the part
that is this daemon's own output (their exact texts belong to the gRPC spec).

Several log lines that concern the RPC servers are emitted under `DCRW`, not
`GRPC`/`RPCS`, because `rpcserver.go` is package `main`: `Generating TLS
certificates...`, `Done generating TLS certificates`,
`Server TLS is disabled.  Only JSON-RPC may be used` (two spaces), `gRPC server listening on <addr>`, the listener
warnings of section 5.3.5, and `Finished serving gRPC: <err>` (trace).

### 4.5 Seed encodings used by the prompts

- **Hex**: lower- or upper-case hexadecimal, 16 to 64 bytes.
- **Mnemonic** (`walletseed/seed.go:38-67`, `pgpwordlist/pgpwordlist.go:26-60`):
  one word per seed byte plus one checksum word. Byte `b` at position `i`
  (0-based) maps to `wordList[2*b + (i mod 2)]`, the 512-entry alternating PGP
  word list in `pgpwordlist/wordlist.go`. The checksum byte is the first byte
  of `SHA-256(SHA-256(seed))` and is encoded at position `len(seed)`. A
  32-byte seed is therefore 33 words. Decoding lower-cases each word, skips
  empty words, and fails with `word %v is not in the PGP word list` or
  `word %v is not valid at position %v, check for missing words` (word parity
  must equal position parity), then `checksum mismatch`, then (decoded
  seed outside 16..64 bytes, or fewer than two words decoded)
  `seed length must be between 128 and 512 bits`
  (`walletseed/seed.go:89-100`,
  `dcrd/hdkeychain/v3@v3.1.3/extendedkey.go:79-80`). These errors are
  wrapped by `errors.E` with kind `Encoding`. Because `errors.Separator` is
  already `":: "` during `--create` (set in `init`), the rendered texts are
  `walletseed.DecodeUserInput: invalid encoding:: pgpwordlist.DecodeMnemonics: word foo is not in the PGP word list`
  and `walletseed.DecodeUserInput: invalid encoding: checksum mismatch`.

## 5. Algorithms and state machines

Conventions for this section: "print to stderr" means `fmt.Fprintln(os.Stderr,
err)` (message plus `\n`) unless a different call is quoted; "fail" means
`loadConfig` returns the error, `run` returns it, and `main` exits with
status 1 without printing anything further. `funcName` is the literal
`loadConfig`. `usage` is the line `Use <appName> -h to show usage`, where
`appName` is `filepath.Base(os.Args[0])` with its `filepath.Ext` removed
(`config.go:406-409`).

### 5.1 Process entry (`dcrwallet.go:36-55`)

1. Package initialisers run: `errors.Separator = ":: "`; the default path
   variables of section 4.1.3; `log.go` wires the subsystem loggers; on Unix
   `signals` becomes `{SIGINT, SIGTERM, SIGHUP}`; `version.BuildMetadata` is
   filled from build info if empty; slog reads `LOGFLAGS`.
2. `ctx = withShutdownCancel(context.Background())`.
3. `go shutdownListener()`.
4. `err = run(ctx)`.
5. If `err != nil` and `!errors.Is(err, context.Canceled)`: `os.Exit(1)`.
   Otherwise return from `main` (status 0).

### 5.2 `loadConfig` (`config.go:339-1050`)

Steps run in exactly this order; each validation sees the effects of the
earlier ones.

1. **Defaults.** Build `cfg` from the literal at `config.go:345-389` (values
   in section 4.1.1), with `dial = (&net.Dialer{}).DialContext` and
   `lookup = net.LookupIP`.
2. **Pre-parse copy.** `preCfg := cfg` is a shallow copy. All options whose
   type is a pointer to a `cfgutil` value (1, 6, 11, 28, 32, 33, 37, 38, 50,
   51, 52, 69, 75, 81) are therefore the *same objects* in `preCfg` and
   `cfg`; values the pre-parse stores into them are already present in `cfg`
   before the config file is read. `[]string` and `*uint` options are
   independent.
3. **Pre-parse argv** into `preCfg` (`config.go:394-403`).
   - `-h`/`--help`: help on stdout, `os.Exit(0)`.
   - Any other error: go-flags has printed the message to stderr; write the
     help text to stderr; fail.
4. **Version.** If `preCfg.ShowVersion`: print to stdout
   `<appName> version <version.String()> (Go version <runtime.Version()> <GOOS>/<GOARCH>)\n`
   and `os.Exit(0)` (`config.go:410-414`).
5. **Locate the config file** (`config.go:419-427`).
   - `--configfile` given on the command line: `cleanAndExpandPath(value)`.
   - Otherwise, if `preCfg.AppDataDir.Value != defaultAppData`:
     `filepath.Join(<appdata value exactly as typed>, "dcrwallet.conf")`. The
     appdata value is **not** passed through `cleanAndExpandPath` here.
   - Otherwise `defaultConfigFile`.
6. **Parse the config file** into `cfg` with a second parser
   (`config.go:428-437`). If the error is not an `*os.PathError` (that is, a
   syntax error, unknown section, unknown option or bad value): print it to
   stderr, write help to stderr, fail. If it is an `*os.PathError` (file
   missing, permission denied, path is a directory): remember it as
   `configFileError` and continue.
7. **Parse argv again** into `cfg` (`config.go:440-447`). On error write help
   to stderr (unless the error is the help request) and fail.
8. **Rebase paths on `--appdata`** (`config.go:452-472`). Only if
   `cfg.AppDataDir.ExplicitlySet()` (command line or config file):
   `AppDataDir = cleanAndExpandPath(AppDataDir)`, then for each of `rpckey`,
   `rpccert`, `clientcafile`, `dcrdclientcert`, `dcrdclientkey`, `logdir`
   that was **not** explicitly set, replace its value with
   `Join(AppDataDir, <default basename>)` (`rpc.key`, `rpc.cert`,
   `clients.pem`, `dcrd-client.cert`, `dcrd-client.key`, `logs`).
9. **Network** (`config.go:476-491`). `--testnet` selects
   `netparams.TestNet3Params`, `--simnet` selects `netparams.SimNetParams`,
   neither leaves mainnet. Both: print to stderr and fail with
   `loadConfig: The testnet and simnet params can't be used together -- choose one`.
10. **File logging** (`config.go:493-535`), skipped entirely under
    `--nofilelogging` (the log size is then not validated):
    1. `LogDir = Join(cleanAndExpandPath(LogDir), activeNet.Params.Name)`.
    2. Find `units`, the byte index of the first character of `LogSize` that
       is not `0`-`9`. If there is none, or it is index 0, fail with
       `loadConfig: Invalid logsize: <LogSize> ` (note the trailing space
       before the newline).
    3. `n = strconv.ParseInt(LogSize[:units], 10, 32)`; failure (more than
       2147483647) is the same error.
    4. Suffix `LogSize[units:]`: `k`, `K`, `KiB` → `n`; `m`, `M`, `MiB` →
       `n << 10`; `g`, `G`, `GiB` → `n << 20`; anything else the same error.
       The result is a size in KiB.
    5. `loggers.InitLogRotator(Join(LogDir, "dcrwallet.log"), n)`.
11. **`--debuglevel=show`**: print to stdout
    `Supported subsystems [AMGR CMGR DCRW GRPC LODR MIXC MIXP PEER RPCS SYNC TKBY VSPC WLLT]\n`
    and `os.Exit(0)` (`config.go:538-541`). The log directory and log file
    have already been created by step 10.
12. **Debug levels**: `parseAndSetDebugLevels(cfg.DebugLevel)` (section
    5.2.2). On error print `loadConfig: <message>` to stderr, write help to
    stderr, fail.
13. **Missing config file** (`config.go:553-565`). If `configFileError` is set
    and `cfg.ConfigFile.ExplicitlySet()`: log it at error level
    (`[ERR] DCRW: open <path>: no such file or directory`) and fail. If it is
    set otherwise: log the same text at warn level and continue. A default
    installation without a config file therefore logs one `WRN` line on every
    start.
14. If `BalanceToMaintainAbsolute < 0`: fail with
    `loadConfig: balancetomaintainabsolute cannot be negative: <amount> DCR`.
15. If `--createtemp` and appdata was not explicitly set: print to stderr
    `Tried to create a temporary simulation wallet, but failed to specify data directory!`
    and `os.Exit(0)`.
16. If `--createtemp` without `--simnet`: print to stderr
    `Tried to create a temporary simulation wallet for network other than simnet!`
    and `os.Exit(0)`.
17. `netDir = networkDir(AppDataDir, activeNet.Params)`;
    `dbPath = Join(netDir, "wallet.db")`.
18. If `--createtemp` and `--create`: fail with
    `The flags --create and --createtemp can not be specified together. Use --help for more information.`
19. `dbFileExists, err = cfgutil.FileExists(dbPath)`; on error print and fail.
20. **Creation branch** (`config.go:608-666`):
    - `--createtemp`: if the database exists print to **stdout**
      `The wallet already exists. Loading this wallet instead.`;
      `checkCreateDir(netDir)` (print and fail on error); if the database did
      not exist run `createSimulationWallet` (on error print
      `Unable to create wallet: <err>` to stderr and fail). Startup then
      **continues**; the process does not exit.
    - else `--create` or `--createwatchingonly`: if the database exists fail
      with ``The wallet database file `<dbPath>` already exists.``;
      `checkCreateDir(netDir)`; `os.Stdout.Sync()`; run
      `createWatchingOnlyWallet` if `--createwatchingonly` is set (it wins
      when both are given), else `createWallet`. On error print
      `Unable to create wallet: <err>` to stderr and fail. On success
      `os.Exit(0)`.
    - else if the database does not exist and `--noinitialload` is not set:
      fail with
      `The wallet does not exist.  Run with the --create option to initialize and create it.`
      (two spaces after the first period).
21. **Proxy** (`config.go:668-737`), only when `--proxy` is non-empty. Build a
    go-socks `Proxy{Addr, Username, Password, TorIsolation}`. With
    `--torisolation` the dialer is `socks.NewPool(proxy,
    uint32(CircuitLimit)).DialContext`, otherwise `proxy.DialContext`.
    `cfg.dial(ctx, network, address)` becomes:
    1. `host` = host part of `address`, or `address` itself if it has no
       port.
    2. If `host == "localhost"`: dial directly.
    3. If `host` parses as an IP address inside `127.0.0.0/8`, `::1/128`,
       `10.0.0.0/8`, `172.16.0.0/12`, `192.168.0.0/16` or `fc00::/7`: dial
       directly.
    4. Otherwise dial through the proxy; an error is wrapped as
       `proxy dial <network> <address>: <err>`.
    `cfg.lookup` becomes a Tor SOCKS resolve through `connmgr.TorLookupIP`
    with error wrap `proxy lookup for <host>: <err>`; nothing reads
    `cfg.lookup` (section 10).
22. **Mixing solver** (`config.go:739-757`), only under `--mixing`:
    - `--csppsolver` explicitly set: `solverrpc.SolverProcess = value`
      (not path-expanded); the solver must work.
    - Otherwise `solverrpc.StartSolver()` (spawns the program named
      `csppsolver` found through `PATH`, connected by stdin/stdout): if that
      succeeds the solver must work; if it fails log
      `[WRN] DCRW: Unable to start csppsolver; must rely on other peers publishing results`
      and continue.
    - "Must work": `testStartedSolverWorks()` calls `StartSolver` (a no-op
      after the first call; the first result is sticky). If that returns an
      error (for example the program named by an explicit `--csppsolver`
      cannot be executed) the probe fails with that error unchanged
      (`testsolver.go:19-21`). Otherwise it calls
      `solverrpc.Roots([1, 2], mixing.F)`; of the possible `Roots` results
      only an `io.ErrUnexpectedEOF` or `net/rpc.ErrShutdown` counts as
      failure (`rpc failed: <err>`); any other result, including other
      errors, passes. On failure print to stderr and fail with
      `csppsolver process is not operating properly: <err>`.
23. **`--mixedaccount`** (`config.go:760-778`), when non-empty: split at the
    **last** `/`. No slash: fail with
    `--mixedaccount must have form 'accountname/branch'`. The part after the
    slash must be exactly `0` or `1`, else fail with
    `--mixedaccount branch must be 0 or 1`. Store `mixedAccount` (the part
    before) and `mixedBranch`.
24. If `--ticketsplitaccount` is empty set it to `mixedAccount` (possibly
    empty).
25. **`--rpcconnect`** (`config.go:784-805`): empty becomes
    `localhost:<JSONRPCClientPort>`; then `NormalizeAddress(value,
    JSONRPCClientPort)`. Error: `fmt.Fprintf(os.Stderr, "Invalid rpcconnect
    network address: %v\n", err)` and fail. `RPCHost` is the host part.
26. **dcrd TLS** (`config.go:806-842`):
    - `--noclienttls`: `RPCHost` must be exactly `localhost`, `127.0.0.1` or
      `::1`, else print
      `loadConfig: the --noclienttls option may not be used when connecting RPC to non localhost addresses: <rpcconnect>`
      then `usage`, and fail.
    - TLS enabled and `--cafile` not explicitly set: `CAFile =
      Join(AppDataDir, "dcrd.cert")`. If that file does not exist **and**
      `RPCHost` is one of the three localhost names **and**
      `dcrdDefaultCAFile` exists, `CAFile = dcrdDefaultCAFile`. An unexpected
      `Stat` error on either file is printed and fails.
27. **Mode conflicts** (`config.go:844-859`), each printed to stderr and
    failing:
    - `--spv` and `--offline`:
      `SPV and Offline mode cannot be specified at the same time`
    - `--spv` and `--enablevoting`:
      `SPV voting is not possible: disable --spv or --enablevoting`
    - `--spvconnect` without `--spv`: `--spvconnect requires --spv`
28. Normalise every `--spvconnect` entry with the network's default P2P port
    (9108, 19108, 18555). An error fails **without printing anything**.
29. **Default listeners** (`config.go:870-891`). `net.LookupHost("localhost")`
    (error fails silently). If no `--grpclisten` was given and `--nogrpc` is
    not set, the gRPC listeners become one `JoinHostPort(addr,
    GRPCServerPort)` per returned address, in the resolver's order; if
    `--nogrpc` is set the list becomes nil. The same for `--rpclisten` /
    `--nolegacyrpc` with `JSONRPCServerPort`.
30. **Normalise listeners** with `NormalizeAddresses` (default port added,
    duplicates removed). Errors:
    `Invalid network address in legacy RPC listeners: <err>\n` (JSON-RPC) and
    `Invalid network address in RPC listeners: <err>\n` (gRPC), both to
    stderr via `Fprintf`.
31. **Shared listeners** (`config.go:912-927`): if a normalised address
    appears in both lists and does not end in `:0`, fail with
    ``Address `<addr>` may not be used as a listener address for both RPC servers``.
32. **`--noservertls`** (`config.go:931-953`): every listener host of both
    servers must be exactly `localhost`, `127.0.0.1` or `::1` (an empty host
    is rejected). Errors, each followed by `usage`:
    `loadConfig: RPC listen interface '<addr>' is invalid: <err>` and
    `loadConfig: the --noservertls option may not be used when binding RPC to non localhost addresses: <addr>`.
33. **VSP** (`config.go:956-972`): if either `vsp.url` or `vsp.pubkey` is
    non-empty, fail with `vsp pubkey can not be null` when the key is empty,
    `vsp URL can not be null` when the URL is empty,
    `vsp max fee must be greater than zero` when `vsp.maxfee` is exactly 0.
34. **Expand paths** with `cleanAndExpandPath`: `cafile`, `rpccert`, `rpckey`,
    `dcrdclientcert`, `dcrdclientkey`, `clientcafile` (`config.go:975-980`).
    Not expanded: `csppsolver`, `cpuprofile`, `memprofile`.
35. **dcrd credentials**: empty `dcrdusername` takes `username`; empty
    `dcrdpassword` takes `password`.
36. **`--dcrdauthtype`** (`config.go:993-1025`): `basic` is accepted.
    `clientcert` fails with
    `dcrdauthtype=clientcert is incompatible with disableclienttls` under
    `--noclienttls`, with
    `dcrdclientcert "<path>" is required by dcrdauthtype=clientcert but does not exist`
    or the same text with `dcrdclientkey` when a file is missing (paths are
    Go-quoted with `%q`). Any other value: `unknown dcrd authtype "<value>"`
    then `usage`.
37. **`--jsonrpcauthtype`**: must be `basic` or `clientcert`, else
    `unknown JSON-RPC authtype "<value>"` then `usage`.
38. **Old testnet directory**: if `Join(AppDataDir, "testnet")` exists log
    `[WRN] DCRW: Wallet data from previous testnet found (<dir>) and can probably be removed.`
39. Return `(&cfg, remainingArgs, nil)`.

Details of individual steps that the list above does not spell out:

- **Step 3 and step 7 print parse errors themselves.** Both parsers are built
  with `flags.Default`, which includes `PrintErrors`; go-flags writes the
  error line to stderr (help text to stdout) before `Parse` returns. The INI
  parser of step 6 does not print; `loadConfig` prints its error with
  `fmt.Fprintln(os.Stderr, err)` (`config.go:432`).
- **Step 9 mutates the global `activeNet`** (`config.go:478,482`) before the
  conflict check, in the order testnet then simnet; with both flags the global
  is left at simnet when the error is returned (not observable, the process
  exits).
- **Step 10.2 scans runes, not bytes**, but returns the byte index
  (`config.go:501-506`); a leading `-` or `+` is a non-digit at index 0 and is
  therefore invalid. The first non-digit ends the number, so `10MiBx` is
  rejected by the suffix match, and `1e3K` is rejected (suffix `e3K`).
- **Step 14** formats the amount through `dcrutil.Amount.String()`, for
  example `loadConfig: balancetomaintainabsolute cannot be negative: -1 DCR`.
- **Step 25/26.** After normalisation `net.SplitHostPort(cfg.RPCConnect)` is
  called again to obtain `RPCHost`; an error there fails with **no output**
  (`config.go:802-805`; unreachable after a successful normalisation).
- **Step 30 runs JSON-RPC first, then gRPC** (`config.go:895-908`), so with a
  bad address in both lists only the "legacy RPC listeners" message appears.
- **Step 31** iterates the gRPC list in order and reports the first shared
  address (`config.go:917-926`).
- **Step 32** checks all JSON-RPC listeners in order, then all gRPC listeners
  (`config.go:932-933`), and stops at the first offender.
- **Step 33** is skipped entirely when both `vsp.url` and `vsp.pubkey` are
  empty: `vsp.sync` and `vsp.maxfee` are then accepted and unused (except
  that `vsp.maxfee` is always passed to the loader as `VSPMaxFee`). Only
  `vsp.maxfee == 0` is rejected; a negative fee passes. Neither the URL nor
  the base64 key is syntax-checked here; `w.VSP` does that in `run` (step
  10.4 of section 5.3).
- **Step 36** ignores the error result of `cfgutil.FileExists`
  (`config.go:1002,1011`): a `Stat` failure other than not-exist (for example
  permission denied on a parent directory) is reported as "does not exist".
  The certificate is checked before the key. The files are only tested for
  existence here; they are read in `rpcSyncLoop`.
- **Step 38** likewise ignores the `FileExists` error (`config.go:1041`).
- **No step validates** `--gaplimit`, `--accountgaplimit`, `--watchlast`,
  `--rpcmaxclients`, `--rpcmaxwebsockets`, `--mixsplitlimit`,
  `--circuitlimit`, `--ticketbuyer.limit`, `--txfee`, `--purchaseaccount`,
  `--changeaccount`, `--ticketsplitaccount`, `--ticketbuyer.votingaccount`,
  `--proxy`, `--profile`, `--cpuprofile`, `--memprofile` or `--csppsolver`
  beyond type conversion. Zero, negative (for the signed types) and
  nonsensical values reach the consumers unchanged. Nor are these
  combinations rejected: `--mixing` without `--mixedaccount` or
  `--changeaccount`; `--mixchange` without `--mixing`; `--enableticketbuyer`
  with `--offline` or on a watching-only wallet; `--promptpass` with
  `--pass`; `--torisolation`, `--proxyuser`, `--proxypass`, `--circuitlimit`
  or `--nodcrdproxy` without `--proxy` (silently unused);
  `--spvdisablerelaytx` without `--spv`; `--create` with `--noinitialload`
  (creation wins, the process exits); `--rpclistenerevents` /
  `--issueclientcert` without `--pipetx`. Account names are resolved only
  after the wallet is open (section 5.3 steps 10.4-10.5).

#### 5.2.1 `cleanAndExpandPath` (`config.go:201-251`)

1. Empty input returns empty.
2. `os.ExpandEnv`: replace `$NAME` and `${NAME}` with the environment value
   (empty when unset). `%NAME%` is not expanded on any platform.
3. If the result does not start with `~`, return `filepath.Clean(result)`.
4. Strip the `~`. The user name is the text up to the first path separator
   (`/`; on Windows either `\` or `/`); if there is no separator the user
   name is empty and the whole remainder is kept as the path.
5. Home directory: current user (`os/user.Current`) for an empty name,
   otherwise `os/user.Lookup(name)`. On lookup failure or an empty home
   directory use `.`.
6. Return `filepath.Join(home, rest)`.

The result is cleaned but not made absolute; a relative path stays relative
to the working directory.

#### 5.2.2 `parseAndSetDebugLevels` (`config.go:276-322`)

1. If the string contains neither `,` nor `=`: it must be a level name (see
   below), else error `The specified debug level [<s>] is invalid`. Set every
   subsystem to that level. Done.
2. Otherwise split on `,`. For each piece, in order:
   1. No `=` in the piece: error
      `The specified debug level contains an invalid subsystem/level pair [<piece>]`.
   2. Split on `=`; the subsystem is field 0 and the level is field 1 (extra
      fields are ignored).
   3. The subsystem must be one of the 13 tags, case-sensitive, else
      `The specified subsystem [<id>] is invalid -- supported subsystems [AMGR CMGR DCRW GRPC LODR MIXC MIXP PEER RPCS SYNC TKBY VSPC WLLT]`.
   4. The level must be valid, else
      `The specified debug level [<level>] is invalid`.
   5. Apply the level to that subsystem immediately.
3. Pieces are applied as they are validated; an error in a later piece leaves
   the earlier ones applied.

Level names (`slog@v1.2.0/log.go:110-129`), case-insensitive: `trace`/`trc`,
`debug`/`dbg`, `info`/`inf`, `warn`/`wrn`, `error`/`err`, `critical`/`crt`,
`off`.

#### 5.2.3 `AmountFlag.UnmarshalFlag` (`internal/cfgutil/amount.go:32-44`)

1. Remove one trailing ` DCR` if present.
2. `f = strconv.ParseFloat(value, 64)` (accepts decimal and exponent forms,
   `inf`, `nan`, hexadecimal floats); error returned as is.
3. NaN or ±Inf: `invalid coin amount`.
4. `atoms = int64(f*1e8 + 0.5)` for `f >= 0`, `int64(f*1e8 - 0.5)` for
   `f < 0` (truncation toward zero after the offset).

Negative values are accepted here; only `balancetomaintainabsolute` is
range-checked later.

#### 5.2.4 `NormalizeAddress` (`internal/cfgutil/normalization.go:12-27`)

1. `host, port, err = net.SplitHostPort(addr)`. If it succeeds return
   `net.JoinHostPort(host, port)` (which brackets a host containing `:`).
2. Otherwise `candidate = net.JoinHostPort(addr, defaultPort)`; if
   `SplitHostPort(candidate)` succeeds return `candidate`, else return the
   error from step 1.

Examples: `""` → `:9110`; `localhost` → `localhost:9110`; `::1` →
`[::1]:9110`; `[::1]` → `[[::1]]:9110` fails the second split, so the
original error `address [::1]: missing port in address` is returned;
`127.0.0.1:0` is unchanged. The host is not resolved or validated here.

### 5.3 `run` (`dcrwallet.go:78-449`)

1. `loadConfig(ctx)`; on error return it. Store the result in the global
   `cfg`. Defer `loggers.CloseLogRotator()` (deferred action D1).
2. Log `[INF] DCRW: Version <version.String()> (Go version <runtime.Version()> <GOOS>/<GOARCH>)`.
   Under `--nofilelogging` also `[INF] DCRW: File logging disabled`.
3. **Pipes.** If `--piperx` was given start `serviceControlPipeRx(fd)` in a
   goroutine. If `--pipetx` was given start `serviceControlPipeTx(fd)` in a
   goroutine, otherwise start `drainOutgoingPipeMessages()` in a goroutine.
4. **Profiling HTTP server**, if `--profile` has entries: return `ctx.Err()`
   if already cancelled; register a 303 redirect from `/` to `/debug/pprof`
   on Go's default HTTP mux (which also carries `net/http/pprof`); for each
   address start a goroutine that logs
   `[INF] DCRW: Starting profile server on <addr>` and calls
   `http.ListenAndServe(addr, nil)`; if that returns an error,
   `fatalf("Unable to run profiler: %v", err)` (exit 1). Addresses are not
   validated or normalised.
5. **CPU profile**, if `--cpuprofile` is non-empty: cancelled check; create
   the file (error: `[ERR] DCRW: Unable to create cpu profile: <err>`, return
   the error); start the Go CPU profiler; defer file close (D2) and profiler
   stop (D3).
6. **Heap profile**, if `--memprofile` is non-empty: cancelled check; create
   the file (error: `[ERR] DCRW: Unable to create mem profile: <err>`);
   defer writing the heap profile and closing the file (D4).
7. Cancelled check.
8. `dbDir = networkDir(AppDataDir, activeNet.Params)`; `loader =
   loader.NewLoader(params, dbDir, EnableVoting, GapLimit, WatchLast,
   AllowHighFees, RelayFee, VSPOpts.MaxFee, AccountGapLimit,
   DisableCoinTypeUpgrades, MixingEnabled, ManualTickets, MixSplitLimit,
   cfg.dial)`.
9. Defer the wallet unload (D5): if the function is unwinding because of a
   panic, re-panic without touching the database (closing bbolt inside a
   panicked transaction would deadlock); otherwise `loader.UnloadWallet()`;
   on success log `[INF] DCRW: Closed wallet`; on an error other than kind
   `Invalid` ("wallet is unopened") log
   `[ERR] DCRW: Failed to close wallet: <err>`.
10. **Initial load**, skipped under `--noinitialload` (steps 10.1-10.6):
    1. `walletPass = []byte(cfg.WalletPass)`; under `--promptpublicpass`
       replace it with `passPrompt(ctx, "Enter public wallet passphrase",
       false)` (error ignored). Cancelled check.
    2. In a goroutine: `w, err = loader.OpenExistingWallet(ctx, walletPass)`,
       zeroing `walletPass` afterwards. On error log
       `[ERR] DCRW: Failed to open wallet: <err>`; if the error has kind
       `Passphrase` and `cfg.WalletPass == "public"` also log
       `[INF] DCRW: Configure public passphrase with walletpass or promptpublicpass options.`
       The main goroutine waits for the result or for cancellation (returning
       `ctx.Err()` and abandoning the open) and returns the open error.
    3. **Private passphrase.** If `--pass` is non-empty:
       `w.Unlock(ctx, pass, nil)` (no relock timeout); on error log
       `[ERR] DCRW: Incorrect passphrase in pass config setting.` and return
       the error. Otherwise `passphrase = startPromptPass(ctx, w)` (5.3.4).
    4. **VSP client**, if `vsp.url` is non-empty:
       - `changeAccountName = cfg.ChangeAccount`; if empty and mixing is
         disabled, log
         `[WRN] DCRW: Change account not set, using purchase account "<purchaseaccount>"`
         and use the purchase account.
       - Resolve the change account then the purchase account with
         `w.AccountNumber`. Failures log
         `[WRN] DCRW: Failed to get account number for ticket change account "<name>": <err>`
         or `... ticket purchase account "<name>": <err>` and return the
         error.
       - `vspClient = w.VSP(wallet.VSPClientConfig{URL, PubKey,
         Policy{MaxFee: vsp.maxfee, FeeAcct: purchase, ChangeAcct: change}})`;
         on error log `[ERR] DCRW: VSP: <err>` and return it.
    5. **Ticket buyer / change mixer**, if `--mixchange` or
       `--enableticketbuyer`. Account names are resolved in this order, and
       resolution stops at the first failure:
       - under `--enableticketbuyer`: `purchaseaccount`; then, if mixing is
         enabled and `ticketbuyer.votingaccount` is empty, log (error level)
         and return `cannot run mixed ticketbuyer without --votingaccount`;
         `ticketbuyer.votingaccount` if set (else voting = purchase);
         `changeaccount` if set (else change = purchase);
       - if (`--enableticketbuyer` and `--mixing`) or `--mixchange`: the
         mixed account name, then `changeaccount` (again, overriding the
         previous value; an empty name fails);
       - if `--enableticketbuyer` and `--mixing`: `ticketsplitaccount`.
       On failure log `[ERR] DCRW: <flag>: account "<name>" does not exist`
       where `<flag>` is `purchaseaccount`, `ticketbuyer.votingaccount`,
       `changeaccount`, `mixedaccount` or `ticketsplitaccount`, and return
       the lookup error. The lookup error is examined only once, after all
       lookups (`dcrwallet.go:292-338`): the helper skips every lookup after
       the first failure and remembers the flag and name of the failing one.
       The "mixed ticketbuyer without --votingaccount" check sits between
       the `purchaseaccount` lookup and that examination, so with
       `--enableticketbuyer --mixing`, no voting account and a nonexistent
       purchase account the voting-account message is the one reported. The
       message always says "does not exist" whatever the lookup error was.
       With `--mixchange` (or `--enableticketbuyer --mixing`) and no
       `--mixedaccount`, the lookup of the empty name fails and the line is
       `[ERR] DCRW: mixedaccount: account "" does not exist`; likewise
       `changeaccount: account "" does not exist` when `--changeaccount` is
       missing. These two messages are the only enforcement of "required
       when mixing".
       Then build `ticketbuyer.Config{BuyTickets: EnableTicketBuyer, Account,
       Maintain: balancetomaintainabsolute, Limit: int(ticketbuyer.limit),
       VotingAccount, Mixing: MixingEnabled, MixChange, MixedAccount,
       MixedAccountBranch, TicketSplitAccount, ChangeAccount, VSP:
       vspClient}`, log `[INF] DCRW: Starting auto transaction creator`, and
       run `tb.Run(ctx, passphrase)` in a goroutine; when it returns with an
       error other than cancellation log
       `[ERR] DCRW: Transaction creator ended: <err>`. Defer waiting for that
       goroutine (D6).
11. Cancelled check.
12. **RPC servers**: `startRPCServers(ctx, loader)` (spec 16, section 5.1).
    On error log `[ERR] DCRW: Unable to create RPC servers: <err>` and return
    it. Inside that call, in order: the `issuedclientcertificate` pipe
    message; gRPC listeners bound and `WalletLoaderService`, `AgendaService`,
    `DecodeMessageService`, `MessageVerificationService` started; one
    goroutine per gRPC listener started, each of which sends its
    `grpclistener` message and then serves (so these messages are not
    ordered relative to what follows); JSON-RPC listeners bound and served;
    `jsonrpclistener` messages.
13. If a gRPC server exists: `loader.RunAfterLoad` a callback that starts, in
    order, the wallet, ticket-buyer, account-mixer, network and voting gRPC
    services. Defer (D7): log `[WRN] DCRW: Stopping gRPC server...`,
    `gRPCServer.Stop()`, log `[INF] DCRW: gRPC server shutdown`.
14. If a JSON-RPC server exists: start a goroutine that calls
    `requestShutdown()` for every value received from
    `jsonRPCServer.RequestProcessShutdown()` (the `stop` method). Defer (D8):
    log `[WRN] DCRW: Stopping JSON-RPC server...`, `jsonRPCServer.Stop()`,
    log `[INF] DCRW: JSON-RPC server shutdown`.
15. **Network backend**, skipped under `--noinitialload`: cancelled check;
    `loader.RunAfterLoad` a callback which, because the wallet is already
    loaded, runs **synchronously on the main goroutine**:
    1. If a VSP client exists and `vsp.sync` is set:
       `w.ProcessedTickets(ctx)` (error logged as
       `[ERR] DCRW: Getting VSP tickets failed: <err>`, not fatal), then
       `vspClient.ProcessManagedTickets(ctx, tickets)` (error logged as
       `[ERR] DCRW: Adding tickets to VSP client failed: <err>`).
    2. `--offline`: `w.SetNetworkBackend(wallet.OfflineNetworkBackend{})` and
       return. `--spv`: `spvLoop(ctx, w)`. Otherwise `rpcSyncLoop(ctx, w)`.
       The two loops return only after cancellation.
16. `<-ctx.Done()`; return `ctx.Err()`.

#### 5.3.1 Deferred actions at shutdown

When `run` returns (normally after cancellation), deferred actions execute in
this order:

1. D8: stop the JSON-RPC server (close listeners, disconnect clients, wait).
2. D7: stop the gRPC server (`Stop`, not `GracefulStop`).
3. D6: wait for the ticket buyer / mixer goroutine to return.
4. D5: unload the wallet (close `wallet.db`).
5. D4: write the heap profile.
6. D3, D2: stop the CPU profiler, close its file.
7. D1: close the log rotator (waits for in-flight gzip jobs).

An early return runs only the deferred actions registered so far.

#### 5.3.2 Sync mode selection

| Condition (evaluated in this order) | Backend |
|---|---|
| `--noinitialload` | none started by `main`; a gRPC client starts one (`RpcSync`/`SpvSync`, gRPC spec) |
| `--offline` | `wallet.OfflineNetworkBackend{}` |
| `--spv` | SPV (`spvLoop`) |
| otherwise | dcrd JSON-RPC over websocket (`rpcSyncLoop`) |

`--spv` with `--offline`, `--spv` with `--enablevoting`, and `--spvconnect`
without `--spv` are rejected earlier (5.2 step 27).

#### 5.3.3 Order in which services start (initial-load mode)

1. Logging (during `loadConfig`).
2. Pipe reader and writer goroutines.
3. Profiling server, CPU profile.
4. Wallet database open and wallet load (`LODR`/`WLLT` log lines).
5. Wallet unlock (config passphrase or prompt).
6. VSP client.
7. Ticket buyer / change mixer goroutine.
8. gRPC server and its loader-independent services, then JSON-RPC server.
9. Wallet-bound gRPC services.
10. VSP ticket sync, then the network syncer.

RPC clients can therefore connect before the wallet has a network backend,
and the wallet is already unlocked (when a passphrase was supplied) before
any RPC listener exists.

#### 5.3.4 `startPromptPass` (`dcrwallet.go:468-529`)

1. `promptPass = cfg.PromptPass`.
2. Watching-only wallet: return nil (never prompts).
3. `needSync, err = w.NeedsAccountsSync(ctx)`. On error log
   `[ERR] DCRW: Error determining whether an accounts sync is necessary: <err>`.
   If it succeeded and is true, print to stdout exactly:

   ```
   *** ATTENTION ***
   Since this is your first time running we need to sync accounts. Please enter
   the private wallet passphrase. This will complete syncing of the wallet
   accounts and then leave your wallet unlocked. You may relock wallet after by
   calling 'walletlock' through the RPC.
   *****************
   ```

   and set `promptPass = true`.
4. `--enableticketbuyer` sets `promptPass = true`.
5. If `promptPass` is false return nil.
6. Loop:
   1. On simnet, try `w.Unlock(ctx, []byte("password"), nil)`; on success
      return `"password"`.
   2. `passphrase, err = passPrompt(ctx, "Enter private passphrase", false)`
      (prompt text `Enter private passphrase: `). On error (cancellation or
      read error) return nil.
   3. `w.Unlock(ctx, passphrase, nil)`. On failure print to stdout
      `Incorrect password entered. Please try again.` and repeat; on success
      return the passphrase.

A wallet unlocked here stays unlocked until an RPC locks it.

#### 5.3.5 Which RPC servers start (`rpcserver.go:248-395`)

The full contract of `startRPCServers` is spec 16; the option
interdependencies are summarised here because they decide whether the
process starts at all.

| Condition, evaluated in this order | Effect |
|---|---|
| `--noservertls` | Log `[INF] DCRW: Server TLS is disabled.  Only JSON-RPC may be used`. No TLS key pair is opened, no client certificate is issued even under `--issueclientcert`, and the gRPC server is **never** started, whatever `--grpclisten` says. |
| TLS enabled | Open or generate the key pair (`openRPCKeyPair`, spec 16); failure is returned. With `--onetimetlskey` and an existing key file the error is ``one time TLS keys are enabled, but TLS key `<rpckey>` already exists`` (`rpcserver.go:54-57`). |
| `--clientcafile` path exists | Read it (a read error is returned and is fatal); if it contains no certificate log `[WRN] DCRW: No certificates added from CA file <path>`. |
| `--issueclientcert` (TLS enabled) | Generate the ephemeral CA and client pair, trust the CA, send `issuedclientcertificate`. |
| gRPC listeners non-empty, and neither a client CA file exists nor `--issueclientcert` | Log `[WRN] DCRW: gRPC server is configured with listeners, but no trusted client certificates exist (looked in <clientcafile>)`; gRPC not started. |
| gRPC listeners non-empty, and a client CA file exists or `--issueclientcert` | Bind the listeners; if none could be bound return `failed to create listeners for RPC server`; otherwise start gRPC (client certificates always required). Each serving goroutine logs `[INF] DCRW: gRPC server listening on <addr>`. |
| TLS enabled, JSON-RPC listeners non-empty, `jsonrpcauthtype=clientcert`, no trusted client certificates | Log `[WRN] DCRW: JSON-RPC TLS server is configured with listeners and client cert auth, but no trusted client certificates exist (looked in <clientcafile>)`; JSON-RPC not started. |
| else `jsonrpcauthtype=basic` and (`--username` empty or `--password` empty) | Log `[INF] DCRW: JSON-RPC server disabled (basic auth requires username and password, and client cert authentication is not enabled)`; JSON-RPC not started. |
| else JSON-RPC listeners non-empty | Bind; if none could be bound return `failed to create listeners for JSON-RPC server`; otherwise start JSON-RPC (basic credentials passed only for `basic`). |
| neither server started | Return `no suitable RPC services can be started`; `run` logs `[ERR] DCRW: Unable to create RPC servers: no suitable RPC services can be started` and the process exits 1 after closing the wallet. |

Consequences:

1. **A default installation does not stay up.** With no `username`/`password`,
   no `clients.pem` and no `--issueclientcert`, startup generates
   `rpc.cert`/`rpc.key`, logs the gRPC warning and the JSON-RPC "disabled"
   line, then fails with "no suitable RPC services can be started" (exit 1).
   The same happens with `--nogrpc --nolegacyrpc`, and with `--noservertls`
   when basic credentials are missing.
2. `--nogrpc` and `--nolegacyrpc` work by emptying the listener lists
   (section 5.2 step 29); the table above then simply finds no listeners.
3. **Hostnames are not bindable.** Listener binding (`makeListeners`,
   `rpcserver.go:455-512`) never resolves names: a listener whose host is not
   empty and not an IP literal (after removing an IPv6 `%zone`) is skipped
   with ``[WRN] DCRW: `<host>` is not a valid IP address``. `--rpclisten=localhost`
   therefore passes every `loadConfig` check (including `--noservertls`,
   which explicitly allows the name `localhost`) and then produces no
   listener; if it is the only one the server fails with "failed to create
   listeners". An empty host (`rpclisten=` or `:9110`) is bound twice, as
   `tcp4` and as `tcp6`, and yields two listener events. A bind failure for
   one address logs `[WRN] DCRW: Can't listen on <addr>: <err>` and is not
   fatal while at least one listener of that server binds.
4. The values handed to the JSON-RPC server from this configuration are
   `Username`, `Password`, `MaxPOSTClients` (`rpcmaxclients`),
   `MaxWebsocketClients` (`rpcmaxwebsockets`), `MixingEnabled`, `MixAccount`
   and `MixBranch` (parsed `--mixedaccount`), `MixChangeAccount`
   (`--changeaccount`), `VSPHost` (`vsp.url`), `VSPPubKey`,
   `TicketSplitAccount`, `Dial` (`cfg.dial`) and `Loggers`
   (`rpcserver.go:368-382`). These apply under `--noinitialload` too, unlike
   the options listed in quirk 31.

### 5.4 Wallet creation

All prompts are written to stdout with no trailing newline unless shown.
Answers are read from stdin. Yes/no prompts are rendered by `promptList`
(5.4.6) as `<question> (n/no/y/yes) [no]: `.

#### 5.4.1 `--create` (`walletsetup.go:113-266`)

1. Build a `loader` exactly as in 5.3 step 8.
2. In a goroutine, on one `bufio.Reader` over stdin:
   1. `privPass = prompt.PrivatePass(reader, []byte(cfg.Pass))` (5.4.8).
   2. `pubPass = prompt.PublicPass(reader, privPass, []byte("public"),
      []byte(cfg.WalletPass))` (5.4.9).
   3. `seed, imported = prompt.Seed(reader)` (5.4.10).
   4. If `imported`: `prompt.Birthday(reader)` (5.4.11); an `io.EOF` error is
      treated as "no". A date gives birthday state `{Time: date, SetFromTime:
      true}`; a block number gives `{Height: n, SetFromHeight: true}`;
      neither gives `{SetFromHeight: true}` (height 0, the genesis block).
      If not imported: `{Time: now - 24h, SetFromTime: true}`.
   5. `prompt.ImportedAccounts(reader, params)` (5.4.12); an `io.EOF` error
      is treated as "no accounts".
   Any other error from these steps ends the goroutine with that error.
3. The main goroutine waits for the goroutine or for cancellation. On
   cancellation it returns `context.Canceled` (the prompt goroutine is
   abandoned); `loadConfig` prints `Unable to create wallet: context canceled`
   and the process exits with status **0** (5.1 step 5).
4. Print `Creating the wallet...\n`.
5. `w = loader.CreateNewWallet(ctx, pubPass, privPass, seed)`.
6. If not imported: `w.UpgradeToSLIP0044CoinType(ctx)`.
7. For each imported account, in entry order: `w.ImportXpubAccount(ctx, name,
   xpub)`.
8. Under `--simnet`: `displaySimnetMiningAddrs(seed, imported)` (5.4.5).
9. `w.SetBirthState(ctx, birthState)`.
10. `loader.UnloadWallet()`.
11. Print `The wallet has been created successfully.\n`.

An error in steps 6-10 is returned with `wallet.db` left on disk; only
`CreateNewWallet` itself removes a partially created database.

#### 5.4.2 `--createwatchingonly` (`walletsetup.go:328-367`)

Runs on the main goroutine and does not observe context cancellation while
prompting.

1. Print `Enter HD wallet public key: `; read a line; trim.
2. `pubPass = prompt.PublicPass(reader, []byte{}, []byte("public"),
   []byte(cfg.WalletPass))`.
3. Print `Creating the wallet...\n`.
4. `db = wallet.CreateDB("bdb", Join(netDir, "wallet.db"))`.
5. `wallet.CreateWatchOnly(ctx, db, pubKeyString, pubPass, params)`. On error
   `os.Remove(dbPath)` is attempted while the database is still open (a
   removal error is printed to stdout) and the creation error is returned.
6. Print `The watching only wallet has been created successfully.\n`; the
   database is closed as the function returns (after the message).

No birthday is recorded and no accounts are imported.

#### 5.4.3 `--createtemp` (`walletsetup.go:270-312`)

1. Private passphrase `password`; public passphrase `public`.
2. `seed = hdkeychain.GenerateSeed(32)`.
3. Write the mnemonic (33 words, single spaces, no newline) to
   `Join(netDir, "seed")`, mode 0644.
4. Print `Creating the wallet...\n`.
5. `wallet.CreateDB("bdb", dbPath)`; `wallet.Create(ctx, db, pubPass,
   privPass, seed, params)`.
6. Print `The wallet has been created successfully.\n`; the database is
   closed as the function returns (after the message). On an error in step 5
   the database file and the `seed` file are left in place.

No SLIP0044 upgrade and no birthday are applied here. Startup continues with
the new wallet; `startPromptPass` unlocks it automatically with `password`
whenever a prompt would otherwise be needed (5.3.4 step 6.1).

#### 5.4.4 Stdout transcript of a default `--create`

With a terminal on stdin, no configured passphrases, and Enter pressed at the
optional questions (`<pw>` is not echoed):

```
Enter the private passphrase for your new wallet: <pw>
Confirm passphrase: <pw>
Do you want to add an additional layer of encryption for public data? (n/no/y/yes) [no]: 
Do you have an existing wallet seed you want to use? (n/no/y/yes) [no]: 
Your wallet generation seed is:
w1 w2 w3 w4 w5 w6 
w7 w8 w9 w10 w11 w12 
w13 w14 w15 w16 w17 w18 
w19 w20 w21 w22 w23 w24 
w25 w26 w27 w28 w29 w30 
w31 w32 w33 

Hex: <64 lower-case hex digits>
IMPORTANT: Keep the seed in a safe place as you
will NOT be able to restore your wallet without it.
Please keep in mind that anyone who has access
to the seed can also restore your wallet thereby
giving them access to all your funds, so it is
imperative that you keep it in a secure location.
Once you have stored the seed in a safe and secure location, enter "OK" to continue: OK
Do you have an additional account to import from an extended public key? (enter account name, or 'no') [no]: 
Creating the wallet...
The wallet has been created successfully.
```

Every word is followed by one space, including the last on each row. The row
`w31 w32 w33 ` is followed by `\n\nHex: ...` so there is one empty line before
`Hex:`. `LODR`/`WLLT` log lines may be interleaved on stdout between
`Creating the wallet...` and the final line.

#### 5.4.5 `displaySimnetMiningAddrs` (`walletsetup.go:49-106`)

Derive the legacy and SLIP0044 coin-type account-0 keys from the seed with
`udb.HDKeysFromSeed(seed, simnet params)` (spec 02). Print:

```
<empty line>
NOTE: only start the wallet after at least 2 blocks (i.e. blocks at heights 1
and 2) have been mined in the backing dcrd node, otherwise account and address
discovery may not work correctly.
<empty line>
Mining address for the SLIP0044 coin type: <address>
Mining address for the legacy coin type: <address>
<empty line>
```

The `legacy` line is printed only when the seed was supplied by the user. Each
address is the P2PKH address of the public key at external branch 0, index 0
of the respective account key.

#### 5.4.6 `promptList` (`internal/prompt/prompt.go:77-104`)

1. Prompt text: `<prefix> (<valid joined by "/">) [<default>]: ` when a
   default exists, else `<prefix> (<valid joined by "/">): `.
2. Print the prompt; read through `\n`. A read error, including EOF and
   including EOF after an unterminated final line, is returned.
3. Lower-case and trim the reply; empty means the default.
4. If the reply is one of the valid responses return it; otherwise go to 2
   with no message.

#### 5.4.7 `PassPrompt` (`internal/prompt/prompt.go:122-173`)

1. Print `<prefix>: `.
2. Read the passphrase:
   - stdin is a terminal: `term.ReadPassword(fd)`, which disables echo,
     reads bytes one at a time until `\n`, applies backspace (`\b`) by
     deleting the previous byte, ignores `\r` (on Windows the line ends
     at `\r` and `\n` is ignored), and restores the terminal
     state on return (`x/term@v0.46.0/term_unix.go:74-91`,
     `terminal.go:1050-1084`). This reads the descriptor directly, not the
     `bufio.Reader`.
   - otherwise: `reader.ReadBytes('\n')`; `io.EOF` is not an error.
3. A read error is returned.
4. Print `\n`.
5. `bytes.TrimSpace`. Empty: go to 1.
6. If `confirm` is false return the passphrase.
7. Print `Confirm passphrase: `; read the same way; print `\n`; trim.
8. If the two differ print `The entered passphrases do not match\n` and go to
   1; otherwise return the passphrase.

Passphrases are raw bytes after trimming leading and trailing white space;
there is no Unicode normalisation and no length limit.

The terminal test is made on **stdin's descriptor**, independently of the
`reader` argument, and separately for the passphrase and for the
confirmation (`internal/prompt/prompt.go:129-130,153`). The trim is
`bytes.TrimSpace` (Unicode white space, so a trailing `\r` from CRLF input is
removed). A passphrase configured through `--pass` or `--walletpass` and
accepted at the "use the existing configured" questions is used **untrimmed**
(`internal/prompt/prompt.go:185-187,222-224`).

`passPrompt` in package `main` (`dcrwallet.go:451-464`), used for the
startup prompts, wraps stdin in a **new** `bufio.Reader` on every call. With
stdin not a terminal the reader may read ahead up to 4096 bytes, and that
read-ahead is discarded when the call returns. Piping
`"<public>\n<private>\n"` in one write to a start with `--promptpublicpass`
and `--promptpass` therefore delivers the public passphrase and then leaves
the private prompt at end of input (where it spins, quirk 14) unless the two
lines arrive in separate reads. Each retry after
`Incorrect password entered. Please try again.` also starts a new reader.

#### 5.4.8 `PrivatePass` (`internal/prompt/prompt.go:177-190`)

If `--pass` is non-empty ask
`Use the existing configured private passphrase for wallet encryption?`
(default no); on yes return the configured value. Otherwise
`PassPrompt(reader, "Enter the private passphrase for your new wallet", true)`.

#### 5.4.9 `PublicPass` (`internal/prompt/prompt.go:199-255`)

1. Ask
   `Do you want to add an additional layer of encryption for public data?`
   (default no). On no return the default `public`, regardless of
   `--walletpass`.
2. If `--walletpass` is non-empty and differs from `public`, ask
   `Use the existing configured public passphrase for encryption of public data?`
   (default no); on yes return the configured value.
3. Loop: `PassPrompt(reader, "Enter the public passphrase for your new
   wallet", true)`. If it equals the private passphrase ask
   `Are you sure want to use the same passphrase for public and private data?`
   (default no); on no repeat the loop.
4. Print
   `NOTE: Use the --walletpass option to configure your public passphrase.\n`
   and return the passphrase.

#### 5.4.10 `Seed` (`internal/prompt/prompt.go:263-369`)

1. Ask `Do you have an existing wallet seed you want to use?` (default no).
2. **No**: generate 32 random bytes; print the block shown in 5.4.4 (33
   words, a newline after every sixth word, then `\n\nHex: %x\n`, then the
   two warning paragraphs). Loop: print
   `Once you have stored the seed in a safe and secure location, enter "OK" to continue: `,
   read a line (error returned), trim white space, trim surrounding `"`
   characters, accept when it equals `OK` case-insensitively. Return
   `(seed, false)`.
3. **Yes**: loop:
   1. Print
      `Enter existing wallet seed (follow seed words with additional blank line): `.
   2. Read lines with a `bufio.Scanner` layered on the reader. Stop at an
      empty line or at end of input. If the **first** line decodes as hex,
      take it alone and stop without waiting for a blank line. Otherwise
      append each line to the accumulated text with a leading space.
   3. Trim, collapse white-space runs to single spaces, and count words.
   4. One word: left-pad with `0` to even length and hex-decode. More than
      one: `walletseed.DecodeUserInput` (section 4.5). On a decode error
      print `Input error: <err>\n`.
   5. On a decode error or a length outside 16..64 bytes print
      `Invalid seed specified.  Must be a word seed (usually 33 words) using the PGP wordlist or hexadecimal value that is at least 128 bits and at most 512 bits\n`
      and repeat from 3.1.
   6. Print `\nSeed input successful. \nHex: %x\n` (a space precedes the
      second newline) and return `(seed, true)`.

Details of the existing-seed loop (`internal/prompt/prompt.go:312-368`):

- The scanner is `bufio.NewScanner(reader)` with the default line splitter
  (strips one trailing `\n` and one `\r` before it) and the default 64 KiB
  token limit; a longer line ends the scan like end of input. A new scanner
  is created on every retry, so read-ahead held by the previous one is lost
  between attempts as well (quirk 15).
- The first-line shortcut applies `hex.DecodeString` to the **raw** line: it
  succeeds only for an even number of hex digits (either case) with no
  surrounding white space. An odd-length hex string, or one with leading or
  trailing blanks, is not recognised there and is accumulated like words; it
  then needs the terminating blank line, after which the one-word path of
  step 4 trims it and left-pads it with `0`.
- "Blank line" means a line of length zero after the splitter; a line of
  spaces is not blank and is appended (it disappears in the collapse).
- Only the first line is tested for hex. A hex seed typed on the second or a
  later line, after a non-hex first line, is treated as a mnemonic word.
- With no input before the blank line (or at end of input) the accumulated
  text is empty: the one-word path hex-decodes the empty string to a
  zero-length seed without error, no `Input error` line is printed, and only
  the `Invalid seed specified.` line appears.
- The one-word error text is Go's `encoding/hex` message, for example
  `Input error: encoding/hex: invalid byte: U+0078 'x'`. A valid hex string
  of the wrong length prints only the `Invalid seed specified.` line.
- In the multi-word path a successfully decoded mnemonic of the wrong length
  is reported by `DecodeUserInput` itself, so both lines appear:
  `Input error: walletseed.DecodeUserInput: invalid encoding: seed length must be between 128 and 512 bits`
  followed by the `Invalid seed specified.` line.
- The generated-seed branch reads the `OK` confirmation with
  `reader.ReadString('\n')`; any other reply, including an empty line,
  silently repeats the prompt. `"ok"`, `OK`, `Ok` and `"OK"` are accepted
  (white space is trimmed first, then every leading and trailing `"`).

#### 5.4.11 `Birthday` (`internal/prompt/prompt.go:372-408`)

Loop:

1. Print
   `Do you have a wallet birthday we should rescan from? (enter date as YYYY-MM-DD, or a block number, or 'no') [no]: `.
2. Read a line (error returned to the caller, which maps EOF to "no"); trim.
3. Case-insensitively: empty, `n`, `no` → return nothing. `y`, `yes` → repeat.
4. If the reply parses as an unsigned base-10 integer that fits 32 bits:
   print `Using birthday block <n>.\n` and return the block number.
5. Parse as `YYYY-MM-DD` (UTC midnight). On failure print
   `Unable to parse date: <err>\n` and repeat.
6. If less than 24 hours have elapsed since that instant print
   `Birthday cannot be in the future or too close (one day) to the present.\n`
   and repeat.
7. Print `Using birthday time <t>.\n`, where `<t>` is Go's default time
   format, for example `2021-03-04 00:00:00 +0000 UTC`, and return the date.

#### 5.4.12 `ImportedAccounts` (`internal/prompt/prompt.go:412-461`)

The set of taken names starts as `{"default"}`. Loop:

1. Print
   `Do you have an additional account to import from an extended public key? (enter account name, or 'no') [no]: `.
2. Read a line (error returned; the caller maps EOF to "none"); trim.
3. Case-insensitively: empty, `n`, `no` → return the collected lists. `y`,
   `yes` → repeat. Anything else is the account name, case preserved.
4. Name already taken: print `Account "<name>" is already defined\n`; repeat.
5. Print `Enter extended public key for account "<name>": `; read a line. EOF
   here becomes `io.ErrUnexpectedEOF` (`unexpected EOF`), which is fatal.
6. `hdkeychain.NewKeyFromString(reply, params)`; on error print
   `Failed to decode extended key: <err>\n` and repeat.
7. A private key: print `Extended key is a private key (not neutered)\n` and
   repeat.
8. Print `Importing account "<name>" from extended public key\n`; record the
   name and key; mark the name taken; repeat.

Names are Go-quoted (`%q`) in these messages. Reserved names such as
`imported` are not rejected here; they fail later in `ImportXpubAccount`.

Every error return of `ImportedAccounts` is `(nil, nil, err)`
(`internal/prompt/prompt.go:421,443`), and `createWallet` turns an `io.EOF`
into success (`walletsetup.go:207-212`). **Accounts accepted before the end
of input are therefore discarded**: a script that supplies
`name\n<xpub>\n` and then closes stdin without a final `no` line sees
`Importing account "name" from extended public key` on stdout, but the
wallet is created with no imported account. The name comparison in step 4 is
exact and case-sensitive (`Default` is not taken); only the `y`/`yes`/`n`/`no`
test is case-insensitive, so an account cannot be named `y`, `yes`, `n` or
`no` in any letter case. The key decode error texts come from
`hdkeychain.NewKeyFromString` and include its wrong-network error for a key
of another network.

`Birthday` (5.4.11) similarly cannot express the dates' time zone: the date
is midnight UTC. Its parse error is Go's `time.Parse` text, for example
`Unable to parse date: parsing time "soon" as "2006-01-02": cannot parse "soon" as "2006"`.
A reply of `0` is a block number (genesis); a number above 4294967295 falls
through to the date parser and is reported as an unparsable date.

### 5.5 Network sync loops

#### 5.5.1 `spvLoop` (`dcrwallet.go:531-555`)

1. Local address `[::1]:0`; `amgr = addrmgr.New(Join(AppDataDir,
   chainParams.Name))` created once and reused across restarts.
2. Loop:
   1. `lp = p2p.NewLocalPeer(params, addr, amgr)`; `lp.SetDialFunc(cfg.dial)`;
      `lp.SetDisableRelayTx(cfg.SPVDisableRelayTx)`.
   2. `syncer = spv.NewSyncer(w, lp)`; if `--spvconnect` has entries,
      `syncer.SetPersistentPeers(entries)`.
   3. `err = syncer.Run(ctx)`.
   4. If `err == nil` or the context is done: log
      `[INF] SYNC: SPV synchronization stopped` and return.
   5. Log `[ERR] SYNC: SPV synchronization stopped: <err>`; wait 5 seconds or
      until cancellation (then return without a further log line).

#### 5.5.2 `rpcSyncLoop` (`dcrwallet.go:562-600`)

1. Before the loop, once:
   - `certs = readCAFile()`: under `--noclienttls` log
     `[INF] DCRW: Chain server RPC TLS is disabled` and use nil; otherwise
     read the CA file; on error log `[WRN] DCRW: Cannot open CA file: <err>`
     and use nil.
   - `clientCert, clientKey = readClientCertKey()`: nil unless
     `dcrdauthtype=clientcert`; read both files; failures log
     `[WRN] DCRW: Cannot open dcrd RPC client certificate: <err>` /
     `[WRN] DCRW: Cannot open dcrd RPC client key: <err>` and leave that
     value nil.
   - `dial = cfg.dial`, or a plain direct dialer under `--nodcrdproxy`.
2. Loop:
   1. `chain.RPCOptions{Address: rpcconnect, DefaultPort:
      JSONRPCClientPort, User: dcrdusername, Pass: dcrdpassword, Dial, CA:
      certs, Insecure: noclienttls}`; if a client certificate was read, clear
      `User` and `Pass` and set `ClientCert`/`ClientKey`.
   2. `err = chain.NewSyncer(w, opts).Run(ctx)`.
   3. If `err != nil`: if it is `context.Canceled` or the context is done log
      `[INF] SYNC: RPC synchronization stopped` and return; otherwise log
      `[ERR] SYNC: RPC synchronization stopped: <err>` and wait 5 seconds or
      until cancellation.
   4. If `err == nil` the loop repeats immediately, with no log line and no
      delay.

The CA file and client certificate are read once per process; replacing them
on disk has no effect until restart.

### 5.6 Pipe goroutines (`ipc.go:46-114`)

**Reader.** Wrap the descriptor as a file named `|<fd>`. Repeatedly discard
up to 1024 buffered bytes. On EOF stop silently. On any other error log
`[ERR] DCRW: Failed to read from pipe: <err>` (Go renders a bad descriptor as
`read |<fd>: bad file descriptor`) and stop. Then send one value on
`shutdownRequestChannel`, which makes `shutdownListener` log
`Shutdown requested.  Shutting down...`.

**Writer.** Wrap the descriptor as a file named `|<fd>` behind a 4096-byte
buffered writer. For each message received from `outgoingPipeMessages`:
build the header of section 4.2.1, write header, write payload, flush. On the
first write or flush error leave the loop, log
`[ERR] DCRW: Failed to write to pipe: <err>`, and continue as a drain so that
senders never block. The loop has no other exit: the channel is never closed.

Senders block until the writer (or drain) goroutine takes the message; the
channel has no buffer. Both goroutines start after `loadConfig`, so neither
pipe is touched during `--create`, `--createwatchingonly`, `-h` or `-V`.

### 5.7 Signals and shutdown requests (`signal.go`)

`shutdownListener`:

1. Register for the platform's signal set (section 3.1, `signals`) on a
   channel of capacity 1.
2. Wait for the first of:
   - a signal: log `[INF] DCRW: Received signal (<name>).  Shutting down...`
     where `<name>` is Go's signal string: `interrupt`, `terminated`,
     `hangup`;
   - a value on `shutdownRequestChannel` (JSON-RPC `stop`, pipe closed): log
     `[INF] DCRW: Shutdown requested.  Shutting down...`.
3. Close `shutdownSignaled`. Every context made by `withShutdownCancel` is
   cancelled.
4. Forever: for each further signal or request log
   `[INF] DCRW: Shutdown signaled.  Already shutting down...`.

There is no forced exit on a second signal and no shutdown timeout.

### 5.8 Loader (`internal/loader/loader.go`)

All three load methods hold the loader mutex for their whole duration, with
one exception noted below, and finish by calling `onLoaded`, which, **still
holding the mutex**, runs every queued callback in registration order on the
calling goroutine, stores the wallet and database, and discards the callback
list. A callback therefore runs exactly once: those queued before the first
load run for the first wallet only, and a wallet loaded after an
`UnloadWallet` triggers none of them (only callbacks registered through
`RunAfterLoad` while no wallet was loaded, which are queued for that next
load).

`OpenExistingWallet(ctx, pubPassphrase)`:

1. Wallet already loaded: `errors.E("loader.OpenExistingWallet", Exist,
   "wallet already opened")`.
2. Release the mutex, `wallet.OpenDB("bdb", Join(dbDir, "wallet.db"))`,
   re-acquire the mutex. On error log `[ERR] LODR: Failed to open database:
   <err>` and return it wrapped with the op.
3. `wallet.Open(ctx, &wallet.Config{DB, PubPassphrase, VotingEnabled,
   GapLimit, WatchLast, AccountGapLimit, DisableCoinTypeUpgrades,
   MixingEnabled, ManualTickets, AllowHighFees, RelayFee, VSPMaxFee,
   MixSplitLimit, Params, Dialer})`. On error close the database and return
   the error wrapped with the op.
4. `onLoaded`.

`CreateNewWallet(ctx, pubPassphrase, privPassphrase, seed)`:

1. Wallet already loaded: kind `Exist`, `wallet already opened`.
2. Ensure the network directory exists (`MkdirAll`, 0700); a non-directory
   gives `"<dir>" is not a directory`.
3. `wallet.db` already present: kind `Exist`, `wallet DB exists`.
4. From here on, any error removes `wallet.db` (best effort).
5. `wallet.CreateDB("bdb", path)`; `wallet.Create(ctx, db, pubPassphrase,
   privPassphrase, seed, params)`; `wallet.Open` with the same configuration
   as above **except that `MixSplitLimit` is not set** (it is 0).
6. `onLoaded`.

`CreateWatchingOnlyWallet(ctx, extendedPubKey, pubPass)`: as
`CreateNewWallet`, with messages `wallet already loaded` / `wallet already
exists`, the non-directory error carrying kind `Invalid`, `wallet.CreateWatchOnly`
in place of `wallet.Create`, and `MixSplitLimit` set.

`UnloadWallet()`: no wallet loaded gives kind `Invalid`, `wallet is unopened`.
Otherwise close the database; on success forget the wallet and database. The
wallet's goroutines are not stopped by the loader; the process relies on
context cancellation for that.

Error strings render as `<op>: <kind text>: <message>`, for example
`loader.OpenExistingWallet: item already exists: wallet already opened`.

### 5.9 Version string (`version/version.go`)

1. At `init`, if `BuildMetadata` is empty: read the Go build info; if it
   records a VCS, take `vcs.revision`; when the VCS is `git` and the revision
   is longer than 9 characters, truncate it to 9. No build info or no VCS
   leaves it empty. A dirty tree (`vcs.modified`) is not reflected.
2. `String()`: `"<Major>.<Minor>.<Patch>"`; append `-` and `PreRelease` with
   every character outside `[0-9A-Za-z.-]` removed, if the result is
   non-empty; append `+` and the similarly filtered `BuildMetadata` if
   non-empty.

At this pin a binary built from a git checkout reports
`2.2.0-pre+54b85b336`; a build without VCS stamping reports `2.2.0-pre`.

The string appears in: the `-V` output, the first `DCRW` log line, the
JSON-RPC `version` result (`versionstring`, with `prerelease` and
`buildmetadata` carrying the raw variables), and the P2P user agent
(`dcrwallet` plus this version, `p2p/peering.go:38-41,238`).

## 6. Constants, limits and defaults

### 6.1 Process constants

| Name | Value | Source | What depends on it |
|---|---|---|---|
| `authTypeBasic` | `"basic"` | `config.go:37` | `--dcrdauthtype`, `--jsonrpcauthtype` |
| `authTypeClientCert` | `"clientcert"` | `config.go:38` | same |
| `defaultCAFilename` | `"dcrd.cert"` | `config.go:42` | CA copy looked up in the application data directory |
| `defaultConfigFilename` | `"dcrwallet.conf"` | `config.go:43` | config discovery |
| `defaultLogLevel` | `"info"` | `config.go:44` | `--debuglevel` default |
| `defaultLogDirname` | `"logs"` | `config.go:45` | log directory |
| `defaultLogFilename` | `"dcrwallet.log"` | `config.go:46` | log file |
| `defaultLogSize` | `"10M"` (10,240 KiB → threshold 10,240,000 bytes) | `config.go:47`, `logrotate@v1.0.0/rotator/rotator.go:75` | rotation |
| `defaultRPCMaxClients` | 10 | `config.go:48` | JSON-RPC POST client cap |
| `defaultRPCMaxWebsockets` | 25 | `config.go:49` | JSON-RPC websocket client cap |
| `defaultAuthType` | `"basic"` | `config.go:50` | both auth-type options |
| `defaultEnableTicketBuyer`, `defaultEnableVoting`, `defaultPromptPass`, `defaultPromptPublicPass`, `defaultAllowHighFees`, `defaultDisableCoinTypeUpgrades` | false | `config.go:51-60` | option defaults |
| `defaultPurchaseAccount` | `"default"` | `config.go:53` | ticket purchase and VSP fee account |
| `defaultPass` | `""` | `config.go:55` | |
| `defaultGapLimit` | 20 (`wallet.DefaultGapLimit`) | `config.go:57`, `wallet/addresses.go:385` | address discovery |
| `defaultAccountGapLimit` | 10 (`wallet.DefaultAccountGapLimit`) | `config.go:59`, `wallet/addresses.go:389` | account discovery |
| `defaultCircuitLimit` | 32 | `config.go:61` | go-socks pool size under `--torisolation` |
| `defaultMixSplitLimit` | 10 | `config.go:62` | concurrent mixes per amount |
| `defaultVSPMaxFee` | 20,000,000 atoms (0.2 DCR) | `config.go:63` | VSP fee ceiling |
| `defaultBalanceToMaintainAbsolute` | 0 | `config.go:66` | ticket buyer |
| `defaultTicketbuyerLimit` | 1 | `config.go:67` | tickets per block |
| `walletDbName` | `"wallet.db"` | `config.go:69`, `internal/loader/loader.go:22` | database file name |
| loader `driver` | `"bdb"` | `internal/loader/loader.go:23` | walletdb driver |
| default relay fee | 10,000 atoms/kB (`txrules.DefaultRelayFeePerKb`, 0.0001 DCR) | `config.go:373`, `wallet/txrules/rules.go:38` | `--txfee` |
| default public passphrase | `"public"` (`wallet.InsecurePubPassphrase`) | `config.go:351`, `wallet/wallet.go:66` | `--walletpass`, creation |
| simulation private passphrase | `"password"` (`wallet.SimulationPassphrase`) | `wallet/wallet.go:72` | `--createtemp`, simnet auto-unlock |
| default TLS curve | Ed25519 | `config.go:363`, `internal/cfgutil/curve.go:29` | generated `rpc.cert`/`rpc.key` and issued client certificates |
| solver program | `"csppsolver"` | `cspp/v2@v2.4.0/solverrpc/rpc.go:15` | `--csppsolver` default |
| pipe protocol version | 1 | `ipc.go:78` | every outgoing pipe message |
| pipe header capacity | 1+1+255+4 bytes | `ipc.go:75` | type string limited to 255 bytes by the 1-byte length |
| pipe read chunk | 1024 bytes | `ipc.go:50` | none observable |
| syncer restart delay | 5 s | `dcrwallet.go:552,596` | reconnect cadence to dcrd / SPV restart |
| new-wallet birthday offset | 24 h before creation time | `walletsetup.go:194` | initial rescan start |
| birthday minimum age | 24 h | `internal/prompt/prompt.go:401` | prompt validation |
| seed length generated | 32 bytes (`hdkeychain.RecommendedSeedLen`) | `internal/prompt/prompt.go:271`, `walletsetup.go:278` | 33-word mnemonic |
| seed length accepted | 16..64 bytes (`MinSeedBytes`, `MaxSeedBytes`) | `dcrd/hdkeychain/v3@v3.1.3/extendedkey.go:38-44` | seed prompt |
| words per row | 6 | `internal/prompt/prompt.go:282` | seed display |
| directory mode | 0700 | `walletsetup.go:375`, `internal/loader/loader.go:122,153`, `internal/loggers/loggers.go:69` | network dir, log dir |
| log file mode | 0644 | `logrotate@v1.0.0/rotator/rotator.go:63,198,222` | log and archives |
| seed file mode | 0644 | `walletsetup.go:288` | `--createtemp` |
| proxy bypass networks | `127.0.0.0/8`, `::1/128`, `10.0.0.0/8`, `172.16.0.0/12`, `192.168.0.0/16`, `fc00::/7` | `config.go:675-689` | direct dialing despite `--proxy` |
| localhost names | `localhost`, `127.0.0.1`, `::1` | `config.go:797-801` | `--noclienttls`, `--noservertls`, dcrd certificate fallback |
| version | 2.2.0, pre-release `pre`, integer 2020000 | `version/version.go:20-33` | RPC results, user agent, logs |
| GODEBUG baseline | `default=go1.25` | `debug.go:15` | Go runtime behaviour only |
| stdin line-reader buffer | 4096 bytes (`bufio.NewReader` default) | `walletsetup.go:130,330`, `dcrwallet.go:455` | amount of piped input a prompt reader may consume beyond the line it returns (sections 5.4.7, 5.4.10) |
| seed scanner token limit | 65536 bytes (`bufio.MaxScanTokenSize`) | `internal/prompt/prompt.go:320` | longest seed input line |
| `--logsize` number range | 1..2147483647 before the unit shift (`ParseInt(_, 10, 32)`); 0 is accepted | `config.go:518` | rotation threshold |
| profile redirect | HTTP 303 from `/` to `/debug/pprof` | `dcrwallet.go:113-114` | `--profile` servers |
| pipe file names | `\|<fd>` (decimal) | `ipc.go:47,73` | text of pipe read/write errors in the log |
| help description column | byte 47 | derived, section 4.1.6 | help layout |
| fallback terminal width | 80 | `go-flags@v1.5.0/help.go:68-70` | help wrapping |

### 6.2 Per-network values

| Network | Selected by | `Params.Name` (log subdirectory, SPV address-book directory) | Network directory (`networkDir`) | dcrd RPC port (`JSONRPCClientPort`) | JSON-RPC server port | gRPC server port | P2P default port (`--spvconnect`) |
|---|---|---|---|---|---|---|---|
| mainnet | default | `mainnet` | `mainnet` | 9109 | 9110 | 9111 | 9108 |
| testnet3 | `--testnet` | `testnet3` | `testnet3` | 19109 | 19110 | 19111 | 19108 |
| simnet | `--simnet` | `simnet` | `simnet` | 19556 | 19557 | 19558 | 18555 |

Sources: `internal/netparams/params.go:21-44`,
`dcrd/chaincfg/v3@v3.3.0/{mainnet,testnet,simnet}params.go`,
`walletsetup.go:34-44`. There is no regnet option.

## 7. Concurrency and lifecycle

### 7.1 Goroutines owned by package `main`

| Goroutine | Started | Ends |
|---|---|---|
| `shutdownListener` | `main`, before `run` | never |
| context watcher in `withShutdownCancel` | `main` | when `shutdownSignaled` closes |
| create-wallet prompt goroutine | `createWallet` | when prompts finish; abandoned on cancellation |
| `serviceControlPipeRx` | `run` step 3, if `--piperx` | at EOF/error, after requesting shutdown |
| `serviceControlPipeTx` or `drainOutgoingPipeMessages` | `run` step 3 | never |
| profile HTTP servers | `run` step 4, one per address | never (or `fatalf`) |
| wallet-open goroutine | `run` step 10.2 | when the open returns (even if `run` already returned on cancellation) |
| `passPrompt` goroutine | each prompt | when the read returns; abandoned on cancellation |
| ticket buyer / mixer | `run` step 10.5 | when `tb.Run` returns; `run` waits for it (D6) |
| JSON-RPC stop forwarder | `run` step 14 | never |
| gRPC `Serve`, one per listener | `startRPCServers` | when the server stops |
| network syncer | `run` step 15 | runs on the **main goroutine**, returns on cancellation |

### 7.2 Channels and locks

- `shutdownRequestChannel` is unbuffered; `requestShutdown` blocks until
  `shutdownListener` receives, which it always eventually does.
- `shutdownSignaled` is closed exactly once.
- `outgoingPipeMessages` is unbuffered and always has exactly one receiver
  after `run` step 3. `startRPCServers` runs after step 3, so its sends cannot
  deadlock.
- `Loader.mu` serialises load/unload; callbacks run under it during the
  first load (section 5.8). `RunAfterLoad` on an already-loaded wallet runs
  the callback outside the mutex.
- slog's backend mutex makes each log line one atomic `Write` to stdout and
  to the file; relative order across subsystems is the order of acquisition.

### 7.3 Ordering guarantees visible outside the process

1. `issuedclientcertificate` precedes every listener message.
2. A `jsonrpclistener`/`grpclistener` message is sent only after the listener
   is bound; a parent that waits for these messages can connect immediately.
3. In initial-load mode the wallet is open (and unlocked when a passphrase
   was configured) before any RPC listener exists; the network backend is
   attached only after the listeners are up, so early RPC calls may observe
   "no network backend" conditions (spec 16).
4. On shutdown, RPC servers stop before the wallet database is closed, and
   the log file is closed last.
5. The context is cancelled before any deferred action runs; long-running
   wallet operations see cancellation first.
6. Nothing bounds shutdown time. A component that ignores cancellation blocks
   exit indefinitely; further signals only log
   `Shutdown signaled.  Already shutting down...`.
7. **Shutdown while the wallet is being opened.** `run` returns as soon as
   the context is cancelled (`dcrwallet.go:223-225`) while the open goroutine
   continues. The deferred unload then takes the loader mutex. If the open
   goroutine is inside `wallet.Open` (it holds the mutex,
   `internal/loader/loader.go:291-342`), the unload waits for the open
   (including any database upgrade) to finish, then closes the database and
   logs `Closed wallet`. If the open goroutine is inside `wallet.OpenDB` (the
   one window in which the mutex is released, `loader.go:300-302`), the
   unload finds no wallet, returns kind `Invalid` (not logged), and `main`
   returns while the open goroutine is still running; the database is closed
   only if `wallet.Open` then fails (for example on the cancelled context)
   before the process image is torn down.
8. **`fatalf` and library-fatal exits skip all of the above.** A profile
   server that cannot bind (`dcrwallet.go:118-121`) or a grpc-go fatal log
   record (section 4.4) calls `os.Exit(1)` from its own goroutine at whatever
   point startup or operation has reached: RPC servers are not stopped and
   the wallet database is not closed. `fatalf` closes the log rotator first;
   the gRPC adapter does not.

## 8. Errors

### 8.1 Exit status

| Status | When |
|---|---|
| 0 | `-h`/`--help`; `-V`/`--version`; `--debuglevel=show`; successful `--create` / `--createwatchingonly`; `--createtemp` rejected for a missing `--appdata` or missing `--simnet` (message on stderr); shell-completion mode; normal shutdown by signal, JSON-RPC `stop` or `--piperx` EOF; cancellation at any point where `run` returns `context.Canceled`, including Ctrl-C during the `--create` prompts |
| 1 | every `loadConfig` failure (section 5.2); log directory or log file cannot be created; wallet open failure; wrong `--pass`; VSP or account-lookup failure at startup; no RPC server can be started (section 5.3.5: includes the default configuration with no RPC credentials and no client CA), a TLS key pair or client CA file that cannot be opened, `--onetimetlskey` with an existing key file; profile server failure (`fatalf`); CPU/heap profile file cannot be created; a fatal log record from the gRPC library (`internal/rpc/rpcserver/log.go:56-69`) |
| 2 | Go runtime: unrecovered panic (after printing the panic and goroutine traces to stderr), fatal runtime errors |
| signal | Go runtime default for unhandled signals: `SIGQUIT` dumps goroutines and exits 2; a write to a closed stdout/stderr pipe raises `SIGPIPE` and kills the process |

### 8.2 Where startup errors are reported

| Phase | Channel |
|---|---|
| go-flags parse errors | stderr (message line), followed by the full help text on stderr |
| config-file syntax errors | stderr (message line), followed by help on stderr |
| validation errors in `loadConfig` | stderr, one line; some followed by the `usage` line (5.2 steps 26, 32, 36, 37); steps 28 and 29 fail with no output at all |
| missing explicit config file | log line at `ERR` (stdout and log file), nothing on stderr |
| failures in `run` | log line at `ERR`/`WRN` from `DCRW` (stdout and log file) |

Log output always goes to **stdout**, never stderr.

### 8.3 Error kinds

`loadConfig` errors are plain formatted errors with no `errors.Kind`. The
loader returns `errors.Error` values with ops `loader.CreateWatchingOnlyWallet`,
`loader.CreateNewWallet`, `loader.OpenExistingWallet`, `loader.WalletExists`,
`loader.UnloadWallet` and kinds `Exist` (already loaded / already exists),
`Invalid` (unload without a wallet; watching-only path is not a directory),
or the kind of the wrapped wallet/database error (for example `Passphrase`
for a wrong public passphrase). gRPC and JSON-RPC translate these kinds to
their own codes (spec 16 and the gRPC spec). `run` distinguishes only
`errors.Passphrase` (extra hint line) and `errors.Invalid` (suppressing the
close-failure log).

## 9. Compatibility-critical behaviours

1. **Database location.** `wallet.db` is at
   `<appdata>/<mainnet|testnet3|simnet>/wallet.db` with the per-OS default
   `<appdata>` of section 4.1.3. An existing installation must be found
   without any new option.
2. **Option surface.** All 81 options with the names, short names, types,
   defaults and group namespaces of section 4.1.1; unknown names are fatal on
   both the command line and in the config file.
3. **Config file grammar.** Section 4.1.4 in full, including: field-name and
   short-name keys, case-insensitive section names, all options reachable
   with no section header, `[Application Options]` reaching the nested
   groups, `;`/`#` comments only at line start, Go-unquoting of values that
   start with `"`, boolean spellings, and the `<file>:<line>: <msg>` error
   form.
4. **Precedence.** Defaults < file < command line; scalars last-wins;
   `[]string` options given on the command line replace the file's values.
5. **Path derivation.** `--appdata` rebases `rpc.cert`, `rpc.key`,
   `clients.pem`, `dcrd-client.cert`, `dcrd-client.key` and `logs` unless set
   explicitly; `~`, `~user`, `$VAR`, `${VAR}` expansion; the config file is
   looked up under the *unexpanded* `--appdata`.
6. **dcrd CA selection.** `<appdata>/dcrd.cert` if present, else dcrd's own
   `rpc.cert` in dcrd's default application data directory when connecting to
   `localhost`/`127.0.0.1`/`::1`, else the (missing) copy path. This is what
   lets a default dcrd + wallet pair on one machine connect with no
   configuration.
7. **Ports.** Section 6.2; default listeners are every address
   `localhost` resolves to.
8. **dcrd credentials.** `dcrdusername`/`dcrdpassword` fall back to
   `username`/`password`; with a client certificate the user and password are
   sent empty.
9. **Pipe protocol.** Framing and the three message types of section 4.2,
   byte for byte; `issuedclientcertificate` first; listener messages only
   after bind; payload address strings in Go's `host:port` / `[v6]:port`
   form with the real bound port; `--piperx` EOF triggers clean shutdown;
   absence of `--pipetx` must not block startup.
10. **Exit status** per section 8.1, in particular 0 after a clean stop and 1
    for configuration errors (a supervisor distinguishes crash from stop this
    way).
11. **Signals.** SIGINT, SIGTERM and SIGHUP all start a clean shutdown on
    Unix; only the console interrupt on Windows.
12. **JSON-RPC `stop`** must trigger the same shutdown path, and **`debuglevel`**
    must use the grammar of section 5.2.2 including the exact error texts
    (they are returned to the RPC client).
13. **Version reporting.** `2.2.0-pre` plus optional `+metadata` in `-V`, the
    startup log line, the JSON-RPC `version` and `getinfo` results (integer
    2020000) and the P2P user agent component `dcrwallet:<version>/`
    (`dcrd/wire@v1.8.0/msgversion.go:298-303`, spec 14).
14. **Log format and files.** Header layout of section 4.3, subsystem tags
    of section 4.4, file `logs/<net>/dcrwallet.log`, archives
    `dcrwallet.log.<n>.gz`, logging to stdout. Operators' tooling and
    Decrediton's log views read these.
15. **Creation wizard.** Prompt texts, accepted answers and defaults of
    section 5.4, so that scripted creation (piped stdin) used by simnet
    harnesses keeps working: the answer order is private passphrase (twice),
    public-encryption yes/no, existing-seed yes/no, then `OK` or the seed,
    then the optional birthday and account prompts, with EOF at the optional
    prompts meaning "no".
16. **Seed encodings.** 33-word PGP mnemonic with double-SHA256 checksum
    byte and hex, both accepted on input and both printed on creation.
17. **Creation side effects.** New random seeds are upgraded to the SLIP0044
    coin type at creation and get a birthday of now − 24 h; restored seeds
    are not upgraded at creation and default to a genesis birthday.
18. **`--createtemp`.** Requires explicit `--appdata` and `--simnet`, writes
    `<netdir>/seed`, uses passphrases `password`/`public`, continues into
    normal operation, and auto-unlocks on simnet. The Decred simnet harnesses
    rely on this.
19. **Startup unlock.** `--pass` unlocks without a timeout before RPC is
    available; a wrong `--pass` is fatal.
20. **Sync mode.** dcrd RPC by default, `--spv`, `--offline`; restart of a
    failed syncer after 5 s; `--nodcrdproxy` bypasses the proxy for dcrd
    only; proxy bypass for loopback and private addresses.
21. **Loader semantics** used by the gRPC `WalletLoaderService`: one wallet
    at a time, `Exist`/`Invalid` errors, removal of a half-created database,
    and callbacks firing once.
22. **Amount options.** Decimal DCR with optional ` DCR` suffix, rounded half
    away from zero to atoms.
23. **TLS file handling** (shared with spec 16): `rpc.cert`/`rpc.key`
    locations and modes, `--onetimetlskey`, default curve Ed25519.
24. **RPC server enablement rules** of section 5.3.5. Launchers depend on
    them: `--issueclientcert` (with `--pipetx`) is what makes the gRPC server
    start with no `clients.pem` on disk; JSON-RPC needs both `username` and
    `password` for `basic`; `--noservertls` means JSON-RPC only; a process
    with no startable server exits 1 rather than idling. The warning and
    info lines of that table are what an operator greps for when a server is
    missing.
25. **`debuglevel` over JSON-RPC** shares the subsystem list and the setter
    with the command line (`rpcserver.go:233-246`): `debuglevel show` returns
    the string `Supported subsystems [AMGR CMGR DCRW GRPC LODR MIXC MIXP PEER RPCS SYNC TKBY VSPC WLLT]`.
26. **Every option is also a config-file key**, including the ones the
    sample file omits and the "action" flags (`create`, `createtemp`,
    `createwatchingonly`, `pipetx`, `piperx`, `noinitialload`); existing
    `dcrwallet.conf` files that set, for example, `noinitialload=1` or
    `logsize=` must keep working.
27. **Startup prompt texts on stdout**: `Enter public wallet passphrase: `,
    `Enter private passphrase: `, the six-line `*** ATTENTION ***` block and
    `Incorrect password entered. Please try again.` (sections 5.3 step 10.1
    and 5.3.4). Wrappers that drive `--promptpass` match on these.

## 10. Quirks

Each item states what the Go does and where; "port" notes say whether to
reproduce.

1. **No `--lifetimeevents`.** dcrwallet has no lifetime-event option or
   message (`config.go:164-168`, `ipc.go`); only dcrd has. A parent cannot
   learn "startup complete" from the pipe; the listener events are the only
   readiness signal.
2. **Config file found under the unexpanded `--appdata`.**
   `--appdata='~/w'` reads `./~/w/dcrwallet.conf` relative to the working
   directory but uses the expanded directory for everything else
   (`config.go:423-426` versus `:453`).
3. **`appdata=` inside the config file** rebases the derived paths
   (`ExplicitlySet` is true for file-set values) although the file itself was
   located with the previous value (`internal/cfgutil/explicitflags.go:32-36`,
   `config.go:452`).
4. **Booleans cannot be switched off on the command line.** There is no
   `--no-x` form and `--x=false` is an error; a boolean enabled in the file
   can only be disabled by editing the file.
5. **Command-line lists replace file lists** rather than append
   (`go-flags@v1.5.0/option.go:245-251`).
6. **Leading double quote triggers Go unquoting** in both the file and argv,
   so a password such as `"abc` is a syntax error and `"a\tb"` becomes
   `a<TAB>b` (`ini.go:452-464`, `parser.go:544-546`).
7. **Random section order** when applying the config file
   (`go-flags@v1.5.0/ini.go:508`). Port: apply sections in file order, which
   is one of the orders Go can produce.
8. **Positional arguments are silently ignored** (`dcrwallet.go:81`).
9. **`GO_FLAGS_COMPLETION`** turns the process into a completion helper
   (`go-flags@v1.5.0/parser.go:220-234`). Port: may be dropped.
10. **`--createtemp` misuse exits 0** with only a stderr message
    (`config.go:577-589`).
11. **`--createwatchingonly` works without `--create`** and takes priority
    over it; `docs/offline_wallets.md` documents
    `dcrwallet --create --createwatchingonly` (`config.go:631,649-653`).
12. **Ctrl-C during `--create` exits 0** after printing
    `Unable to create wallet: context canceled`; the abandoned prompt
    goroutine may be inside `term.ReadPassword`, whose terminal-restore runs
    only on return, so the terminal can be left with echo disabled (follows
    from `walletsetup.go:214-221` and `x/term@v0.46.0/term_unix.go:74-91`; not
    exercised by running the binary).
13. **`--createwatchingonly` cannot be interrupted while prompting**: the
    signal is consumed by `shutdownListener`, which only logs, and the
    prompts run on the main goroutine without watching the context
    (`walletsetup.go:328-341`).
14. **Prompt loops spin at end of input.** With stdin not a terminal and at
    EOF, `PassPrompt` reads an empty passphrase without error and loops
    forever printing the prompt (`internal/prompt/prompt.go:133-145`); the
    existing-seed loop does the same (`:312-363`). Port: either reproduce or
    fail with EOF; a decision is required (section 13).
15. **The existing-seed prompt swallows following piped answers.** It reads
    through a fresh `bufio.Scanner` over the shared reader
    (`internal/prompt/prompt.go:320`); whatever the scanner buffered beyond
    the seed is lost, so with fully piped input the birthday and
    account-import prompts see EOF and take their defaults. With a new seed
    there is no scanner and later answers are read normally. This depends on
    how the input bytes are chunked by the OS.
16. **Public passphrase "no" ignores `--walletpass`**: the wallet is created
    with `public` (`internal/prompt/prompt.go:210-212`).
17. **Partial creation is not rolled back** after `CreateNewWallet` returns:
    a failure in the SLIP0044 upgrade, an account import (for example a
    reserved account name), or the birthday write leaves `wallet.db` in place
    (`walletsetup.go:229-262`).
18. **Watching-only cleanup removes the file while it is open**
    (`walletsetup.go:356-363`); this fails on Windows and the failure is
    printed to stdout.
19. **`--logsize` oddities** (`config.go:500-534`,
    `logrotate@v1.0.0/rotator/rotator.go:75`): a bare number is invalid;
    the error text ends in a space; `K` means 1000 bytes per unit after the
    binary multiplication (`10M` = 10,240,000 bytes, `1G` = 1,048,576,000
    bytes); `0K` rotates after every line; the option is not validated under
    `--nofilelogging`.
20. **Log directory is created before early exits** that follow step 10:
    `--debuglevel=show`, all validation failures after it, and `--create`.
21. **Missing default config file logs a warning on every start**; a config
    path that is a directory is treated the same way (`config.go:429-437,
    563-565`).
22. **`--debuglevel` applies partially on error and ignores extra `=`
    fields** (`config.go:294-319`). The help text and sample file list six
    levels; `off` and the three-letter abbreviations are also accepted.
23. **`setLogLevel` maps an invalid level to `info`** (`log.go:73-75`);
    unreachable through validated paths.
24. **Solver probe is lenient**: apart from a failure to start the process,
    only two specific error values fail it, and
    `solverrpc` replaces those with `ErrSolverProcessExited` once it has
    observed the child exit, so a solver that dies immediately can pass
    depending on timing (`testsolver.go:18-34`,
    `cspp/v2@v2.4.0/solverrpc/rpc.go:83-162`). The first `StartSolver`
    outcome is cached for the life of the process.
25. **`--csppsolver` is not path-expanded** (`config.go:741-743`).
26. **`cfg.lookup` is dead**: assigned at `config.go:357,730` and never read.
    Also dead: `cfgutil.AddressFlag`, `prompt.ProvideSeed`,
    `prompt.ProvidePrivPassphrase`, the `testnet2` branch of `networkDir`,
    and the inner condition at `config.go:554`.
27. **Listener validation is textual.** `--noclienttls`/`--noservertls`
    accept only the three literal names (not `127.0.0.2`, not `LOCALHOST`,
    not an empty host); the both-servers check compares strings, so
    `localhost:9110` and `127.0.0.1:9110` are not detected as the same
    listener (`config.go:797-953`).
28. **`--rpclisten=[::1]` is rejected** (`missing port in address`) while
    `::1` and `[::1]:9110` are accepted (section 5.2.4).
29. **`--nogrpc` / `--nolegacyrpc` silently drop explicit listeners**
    (`config.go:880-891`).
30. **Default listener order follows `net.LookupHost("localhost")`**, which
    depends on the resolver and hosts file; IPv4 listeners are nevertheless
    bound first (spec 16).
31. **`--noinitialload` silently ignores** `--pass`, `--promptpass`,
    `--promptpublicpass`, `--enableticketbuyer`, `--mixchange`, the VSP
    options, `--spv`, `--offline` and the dcrd connection options
    (`dcrwallet.go:194,417`).
32. **`--promptpublicpass` errors are ignored** (`dcrwallet.go:197`).
33. **`--mixchange` without a passphrase source** starts the mixer with a nil
    passphrase (`dcrwallet.go:193,249,359`); consequences are in spec 13.
34. **`rpcSyncLoop` busy-loops if the syncer returns nil**, and `spvLoop`
    stops syncing permanently if its syncer returns nil
    (`dcrwallet.go:544-547,586-598`). Whether either syncer can return nil is
    for specs 14 and 15.
35. **CA and client certificate files are read once**; a dcrd certificate
    that appears after wallet start is not picked up (`dcrwallet.go:563-564`).
36. **`loader.CreateNewWallet` omits `MixSplitLimit`**
    (`internal/loader/loader.go:259-274`), unlike the other two load paths.
    Visible only for wallets created over gRPC or by `--create` (which
    unloads immediately).
37. **`OpenExistingWallet` drops the mutex around `OpenDB`** and does not
    re-check for a loaded wallet afterwards (`internal/loader/loader.go:299-302`).
38. **Loader callbacks fire once per process** and under the mutex
    (`internal/loader/loader.go:81-89`): after a gRPC unload and reload the
    wallet-bound gRPC services are not re-pointed by the loader.
39. **`--issueclientcert` without `--pipetx`** generates and trusts an
    ephemeral CA whose client key is discarded (`rpcserver.go:291-308`,
    `dcrwallet.go:101-105`).
40. **gRPC listener events are unordered** relative to each other and to the
    JSON-RPC ones (`rpcserver.go:338-346`).
41. **Amount parsing accepts negatives, `inf`/`nan` spellings and hex
    floats**; out-of-range conversion to `int64` is platform-defined in Go
    (`internal/cfgutil/amount.go:32-44`,
    `dcrd/dcrutil/v4@v4.0.3/amount.go:61-91`). `--txfee=-1` is accepted by
    `loadConfig`.
42. **`--circuitlimit` is cast to `uint32`** (negative wraps; 0 makes every
    proxied dial fail with `maximum number of connections reached`)
    (`config.go:703`, `go-socks@v1.1.0/socks/pool.go:39-45`).
43. **`--profile` values are not validated**; an empty `profile=` in the file
    listens on the default HTTP port (`dcrwallet.go:108-123`).
44. **`appName`** strips the last dot-suffix of the executable name
    (`dcrwallet-2.2` becomes `dcrwallet-2`) (`config.go:407-408`); the help
    header uses the unstripped base name.
45. **Go-runtime content in output**: the `-V` line and the startup log line
    embed `runtime.Version()` and `GOOS/GOARCH`; `--profile`, `--cpuprofile`
    and `--memprofile` produce Go pprof data; `LOGFLAGS=longfile|shortfile`
    prints Go source locations.
46. **Sample configuration disagreements** (`sample-dcrwallet.conf`):
    - line 175 documents `legacyrpclisten=`, which is not an option (the key
      is `rpclisten` or `LegacyRPCListeners`); uncommenting it is fatal;
    - line 114 shows `tlscurve=P-256`; the default is Ed25519;
    - lines 27-28 name the network directories `mainnet` and `testnet`; the
      testnet directory is `testnet3`;
    - lines 65-67 claim a proxy disables RPC listening unless `rpclisten` is
      set; no such behaviour exists;
    - lines 168-174 describe listener defaults of an older design;
    - the `[Ticket Buyer Options]` and `[VSP Options]` headers are not
      required for any key: both the dotted keys shown and the bare
      field-name keys (`Limit`, `MaxFee`, ...) work in every section that
      reaches the group, that is with no header, under
      `[Application Options]`, and under the group's own header
      (`go-flags@v1.5.0/group.go:147-176`, section 4.1.4 rules 7-10).
47. **`docs/offline_wallets.md` is stale** (GOPATH paths, `dep`); it is the
    only content under `docs/` besides a one-line index.
48. **`zero(walletPass)`** clears only the byte copy; the configured strings
    stay in memory (`dcrwallet.go:195,209`).
49. **Shallow pre-parse copy** shares pointer-typed option values between the
    two parse passes (`config.go:393`). No externally visible effect was
    found; a port may use two independent passes.
50. **A default installation exits 1.** Without RPC credentials or client
    certificates neither RPC server can start and `run` fails with
    `no suitable RPC services can be started` after having opened (and
    possibly prompted to unlock) the wallet (`rpcserver.go:316-319,355-357,390-392`;
    section 5.3.5).
51. **`localhost` is accepted as a listener host and then cannot be bound.**
    `loadConfig` normalises and whitelists the name, `makeListeners` skips
    every non-literal host with a warning (`config.go:797-801,943`,
    `rpcserver.go:484-487`). The default listeners avoid this only because
    step 29 resolves `localhost` to literals first.
52. **`--noservertls` silently disables gRPC and `--issueclientcert`**
    (`rpcserver.go:264-266`); `--grpclisten` values are validated by
    `loadConfig` and then unused.
53. **Startup prompts lose piped read-ahead**: every `passPrompt` call makes
    a new `bufio.Reader` over stdin (`dcrwallet.go:455`; section 5.4.7).
54. **Imported accounts are dropped at end of input**: `ImportedAccounts`
    returns nil slices with the EOF that `createWallet` then forgives
    (`internal/prompt/prompt.go:419-422`, `walletsetup.go:207-212`; section
    5.4.12). The `Importing account ...` confirmation has already been
    printed.
55. **Yes/no words cannot be account names** at the import prompt, and the
    duplicate-name test knows only `default` plus names entered in the same
    session (`internal/prompt/prompt.go:413-436`).
56. **Action flags work from the config file**: `create=1`,
    `createwatchingonly=1`, `createtemp=1`, `pipetx=N`, `piperx=N` in
    `dcrwallet.conf` behave like the command-line flags, while `version=1`
    and `configfile=...` in the file are inert (`config.go:410,419-427`;
    section 4.1.4).
57. **Unchecked option combinations and ranges**: listed at the end of
    section 5.2. Notable: `--mixing` does not require `--mixedaccount` or
    `--changeaccount` at configuration time (the help text says "required
    when mixing"); the requirement surfaces only as an account-lookup failure
    for the empty name when the ticket buyer or change mixer starts
    (`dcrwallet.go:326-338`), and not at all otherwise.
58. **Account-lookup failures always say "does not exist"** regardless of
    the underlying error, and the voting-account check pre-empts a failed
    purchase-account lookup (`dcrwallet.go:308-338`).
59. **VSP account failures are logged at `WRN` but are fatal**
    (`dcrwallet.go:259-272`), whereas the ticket-buyer account failures are
    logged at `ERR`.
60. **Shutdown during wallet open** either waits for the open to complete or
    exits with the database handle still open, depending on which side of
    the `OpenDB` call the open goroutine is on (section 7.3 item 7;
    `dcrwallet.go:207-230`, `internal/loader/loader.go:299-302`).
61. **Hard exits bypass cleanup**: profile-server bind failure (`fatalf`,
    racing with the rest of startup because the listen happens in a
    goroutine) and grpc-go fatal records (`dcrwallet.go:116-122`,
    `internal/rpc/rpcserver/log.go:56-69`).
62. **gRPC library log levels are flattened to `INF`**, with a `grpc: `
    prefix stripped (`internal/rpc/rpcserver/log.go:36-81`); `--debuglevel
    GRPC=warn` therefore hides library errors too.
63. **Go standard-library error texts appear verbatim in user-visible
    output**: `os.PathError` texts (`open <path>: no such file or directory`)
    in the config-file warning and the CA-file warnings; `strconv` texts in
    option conversion errors; `encoding/hex` and `time.Parse` texts in the
    creation prompts; `net` texts (`address <x>: missing port in address`,
    `too many colons in address`) in the address errors; `read |<fd>: ...` /
    `write |<fd>: ...` in the pipe errors. A port has to reproduce these
    strings (dcroxide's `gostd` module does this for the ones dcrd needs) or
    record each as a deliberate divergence.
64. **Stale or wrong comments** (code wins):
    - `config.go:336-338` and `walletsetup.go:110-112` describe a returned
      bool and passphrase that no longer exist;
    - `config.go:583-584` says "simnet or testnet"; only simnet is allowed;
    - `internal/loader/loader.go:284-287` documents a `canConsolePrompt`
      parameter that does not exist; `:371-374` says `UnloadWallet` "stops
      the loaded wallet" (it only closes the database) and names a
      `LoadExistingWallet` method that does not exist; `:195-197` says a nil
      seed is replaced by a random one (that is `wallet.Create`'s business,
      not the loader's);
    - `internal/loggers/loggers.go:33-35,61-65` refer to `initLogRotator`
      and say loggers must not be used before the rotator exists; in fact
      `logWriter` tolerates a nil rotator and logging before
      `InitLogRotator`, or under `--nofilelogging`, goes to stdout only;
    - `log.go:63-65,78-80` say loggers are "dynamically created as needed";
      the set is fixed;
    - `signal.go:18-19` names `withShutdownChannel` (the function is
      `withShutdownCancel`);
    - `ipc.go:22` says the type string is "no longer than 255 bytes"; the
      length is written as `byte(len(type))` with no check, which would wrap
      for a longer type (all three types are shorter).
65. **Open TODO/XXX markers in scope**: `dcrwallet.go:232-240`
    (`TODO(jrick)`: the startup private-passphrase prompt should be replaced
    by an RPC-triggered accounts rescan; until then it is disabled under
    `--noinitialload`) and `log.go:42` (`XXX`: the mixing client has no
    package-level `UseLogger`, so `MIXC` is attached elsewhere). There are no
    `BUG`, `FIXME` or deprecation markers in the files of this scope.

## 11. Upstream tests

| File | What it pins | Action |
|---|---|---|
| `walletseed/seed_test.go` | Four mnemonic/seed vectors (20-byte and 31-byte seeds), `EncodeMnemonicSlice`, `DecodeUserInput` round trip | Port the vectors verbatim. |
| `pgpwordlist/pgpwordlist_test.go` | Word list decode cases | Port verbatim. |
| `x509_test.go` | Client-certificate issuance for Ed25519/P-256/P-384/P-521 (`rpcserver.go`) | Spec 16. |

Nothing else in this scope has tests: there is no `config_test.go`, no test
for `ipc.go`, `signal.go`, `internal/prompt`, `internal/cfgutil`,
`internal/loader`, `internal/loggers` or `version`.

Vectors to generate with a Go oracle shim (a small Go program linking the
pinned packages, or the real `dcrwallet` binary driven as a black box):

1. **Config pipeline**: for a matrix of argv and config-file contents,
   capture exit status, stdout, stderr, and the resulting effective values
   (the JSON-RPC and gRPC listener sets, CA path, log path, db path). Cover
   every error of section 5.2, every INI rule of section 4.1.4, every type
   conversion error, and the Windows option style.
2. **Help text** at 80 columns and at one narrow and one wide width.
3. **`--debuglevel`** accept/reject matrix including `show`.
4. **`cleanAndExpandPath`** and **`NormalizeAddress(es)`** tables.
5. **`AmountFlag`** parse/format table (rounding edges such as `0.000000005`,
   `1e-8`, `0.1 DCR`, negative values).
6. **Pipe transcripts**: bytes read from `--pipetx` for the combinations of
   `--rpclistenerevents`, `--issueclientcert`, TLS on/off, one and two
   listeners; shutdown on `--piperx` close with exit status.
7. **Creation transcripts**: stdout bytes for scripted stdin through every
   branch of section 5.4 (new seed, hex seed, word seed, bad seeds, birthday
   forms, account import, EOF placements), run with `--simnet` for the mining
   address block. The seed is random, so compare with the seed lines masked,
   or supply seeds.
8. **Log lines**: startup and shutdown sequences for the three sync modes
   with timestamps masked; rotation behaviour with a small `--logsize`.
9. **Version strings** for the build-metadata cases.
10. **RPC server enablement matrix** (section 5.3.5): for the combinations
    of `username`/`password` present or absent, `clients.pem` present or
    absent, `--issueclientcert`, `--noservertls`, `--nogrpc`,
    `--nolegacyrpc`, `--jsonrpcauthtype`, and a hostname listener, capture
    the `DCRW` log lines, the pipe messages and the exit status.
11. **Startup prompt transcripts**: `--promptpublicpass` and `--promptpass`
    with piped stdin delivered in one write and in separate writes (section
    5.4.7), a wrong passphrase followed by a right one, and the simnet
    auto-unlock.
12. **Account-import end-of-input cases** (section 5.4.12): with and without
    the closing `no` line, checking the accounts of the created wallet.

Test files elsewhere in the repository that touch this scope indirectly:
`errors/errors_test.go` (rendering of nested errors with a separator, which
this scope sets to `":: "`) and `internal/rpc/jsonrpc/rpcserver_test.go` /
`rpchelp_test.go` (JSON-RPC server, spec 16). `run_tests.sh` runs
`go test -short -vet=all ./...`; there is no integration harness in the
repository that starts the binary.

## 12. Rust porting notes

### 12.1 Suggested boundaries

| Crate / module | Mirrors | Contents |
|---|---|---|
| `goflags` (shared library crate) | go-flags v1.5.0 | Option registry (groups, namespaces, field names, kinds, custom value parsers with Go type names), argv scanner, INI reader, help renderer, exact error texts. Parameterised by registry and by a small version-quirk set. |
| `gostd` (shared) | Go standard-library behaviours | `filepath.Clean/Join/Base/Ext`, `os.ExpandEnv`, `net.SplitHostPort/JoinHostPort`, `strconv` parse/quote/unquote with Go error texts, Go errno strings, owner-only directory and file creation. |
| `purse-config` | `config.go`, `internal/cfgutil`, `internal/netparams`, `params.go` | The 81-option registry, `load_config` pipeline with injected environment (argv, env, filesystem, localhost resolution, terminal width), the effective `Config`. |
| `purse-logging` | `log.go`, `internal/loggers`, slog, logrotate v1.0.0 | Subsystem registry, level control, line rendering, stdout+file writer, rotator. |
| `purse-ipc` | `ipc.go` | Message framing, writer/drain task, reader task, descriptor adoption. |
| `purse-prompt` | `internal/prompt`, `walletseed`, `pgpwordlist` | Prompts over injected reader/writer/terminal, seed codecs. |
| `purse-loader` | `internal/loader` | Loader state machine over the wallet crate. |
| `purse-version` | `version` | Version constants and string. |
| binary `dcr-purse` (installable as `dcrwallet`) | `dcrwallet.go`, `walletsetup.go`, `signal*.go` | `main`, `run`, creation flows, signal handling, sync loops. |

### 12.2 What dcroxide already has, and what has to change

dcroxide solved the same go-flags/INI problem inside one crate,
`dcroxide:crates/dcroxide-node`, not as a reusable library:

| dcroxide source (looked at) | What it is | Reuse for the wallet |
|---|---|---|
| `dcroxide:crates/dcroxide-node/src/flags.rs` (2188 lines) | Port of go-flags **v1.6.1**: `OptKind`, `OptSpec`, a fixed `OPTIONS: [OptSpec; 86]` for dcrd, argv scanner (`scan_args_in`), INI parser (`parse_ini`), help renderer (`render_help_with`, `wrap_text`), env defaults | The algorithms carry over. It must be generalised: the registry is a dcrd-specific constant; `OptKind` has no `Int64`, no pointer-to-uint, no custom-unmarshaler kinds (needed for `*cfgutil.ExplicitString`, `*cfgutil.AmountFlag`, `*cfgutil.CurveFlag` and their names in marshal errors); there is one option group (plus the Windows service group) where the wallet needs nested groups with namespaces and per-group INI sections; "explicitly set" tracking is absent. Version differences to honour: integer parsing is base 10 in v1.5.0 but base 0 in v1.6.1 (`go-flags@v1.5.0/convert.go:223,237`); v1.6.1's `AllowBoolValues` and command-level `PassAfterNonOption` do not exist in v1.5.0. The INI section-order choice (file order) is the same decision this spec recommends. |
| `dcroxide:crates/dcroxide-node/src/gostd.rs` (1330 lines, private module) | `filepath_clean`, `filepath_join`, `expand_env`, `split_host_port`, `join_host_port`, `go_parse_int/uint/float/bool`, `go_unquote`, `go_quote`, `go_errno_string`, `go_mkdir_all_owner_only` | Directly needed; has to be exported or moved to a shared crate. `go_parse_int` is base-0 and needs a base-10 entry point. |
| `dcroxide:crates/dcroxide-node/src/config.rs` | `clean_and_expand_path` (same algorithm as `config.go:201-251`), `app_data_dir` (port of `dcrutil.AppDataDir`), `file_exists`, the `ConfigEnv` injection pattern, `load_config` staging | `clean_and_expand_path` and `app_data_dir` are reusable as written (pass `"dcrwallet"`). The pipeline itself is dcrd's and is rewritten. dcrd's `normalize_addresses` handles interface names and is **not** the wallet's `NormalizeAddress`. |
| `dcroxide:crates/dcroxide-node/src/logsubsys.rs`, `logging.rs` | slog level names, `--debuglevel` grammar, line renderer, process-wide local offset | Reusable with the wallet's 13 tags. The fixed-offset decision documented there (no DST change while running) applies equally. |
| `dcroxide:crates/dcroxide-node/src/logrotate.rs` | Port of logrotate **v1.1.2** | Usable: with gzip and `maxRolls = 0` the behaviour matches v1.0.0 except that v1.0.0 compresses rolls concurrently without a mutex, which is not observable in file contents. |
| `dcroxide:crates/dcroxide-node/src/ipc.rs`, `pipeserve.rs` | Same frame format; dcrd message set (`lifetimeevent`, `p2plistenaddr`, `rpclistenaddr`); inherited-descriptor adoption without `unsafe` (Linux `pidfd_getfd`, path re-open elsewhere, Windows handle duplication via `dcroxide:crates/dcroxide-winsvc`) | Reuse the adoption and reader/writer loops; replace the message enum with the wallet's three types and drop the lifetime/bound-address gates. |
| `dcroxide:crates/dcroxide-node/src/socks.rs` | go-socks `Proxy.DialContext` and `TorLookupIP` | Reusable; add the `socks.NewPool` connection cap used under `--torisolation`. |
| `dcroxide:crates/dcroxide-node/src/secretfile.rs` | Owner-only file and directory creation | Reusable for 0700 directories and 0600 key files. |
| `dcroxide:crates/dcroxide-node/src/profiler.rs` | Precedent: profiling flags validated but not served | Same decision is available for `--profile`/`--cpuprofile`/`--memprofile`. |
| `dcroxide:crates/dcroxide-node/src/version.rs` | Version string handling without Go build info | Same shape; wallet needs its own constants and a build-metadata policy. |
| `dcroxide:crates/dcroxide-certgen` | `certgen` port | Used by spec 16. |
| `dcroxide:crates/dcroxide-node/src/bin/dcroxide.rs` signal handling | `ctrlc` crate with the `termination` feature | Insufficient here: the wallet logs the signal's name (`interrupt`, `terminated`, `hangup`), and `ctrlc` does not report which signal arrived. |

dcr-rs (looked at `dcr-rs:src/amount.rs`, `dcr-rs:src/hd.rs`): `format_amount`
produces the same text as `dcrutil.Amount.String()` for the default values
(`0 DCR`, `0.0001 DCR`, `0.2 DCR`) but formats from integers, whereas Go
formats `float64(atoms)/1e8` with shortest round-trip digits; they can differ
for amounts that are not exactly representable. Its mnemonic support is BIP39
and is unrelated to the PGP word list used here; nothing in dcr-rs supplies
the PGP list.

### 12.3 Hazards

1. **Stdout semantics.** Go ignores stdout write errors in `logWriter` and is
   killed by `SIGPIPE` when stdout is a closed pipe. Rust ignores `SIGPIPE`
   by default and `println!` panics on `EPIPE`. Write log and prompt output
   with explicit error handling and decide the closed-stdout behaviour.
2. **Argument and path bytes.** Go strings are byte strings; argv and paths
   need not be UTF-8. dcroxide chose to reject non-UTF-8 argv
   (`dcroxide:crates/dcroxide-node/src/flags.rs`, `args_after_program`).
3. **Float parsing and rounding** for amounts must match Go's
   `ParseFloat` and the `+0.5` truncation; do not parse decimals exactly.
4. **Terminal handling.** Password entry must read the terminal descriptor
   directly with echo off and must not consume look-ahead from the line
   reader; non-terminal stdin takes the line-reader path. Restore the
   terminal on cancellation (a deliberate improvement over quirk 12).
5. **Blocking prompts and cancellation.** Go abandons goroutines blocked on
   stdin. A Rust port needs a thread it can abandon at process exit, or
   non-blocking stdin.
6. **Synchronous `RunAfterLoad`.** The syncer runs on the caller's task when
   the wallet is already loaded; shutdown ordering depends on it returning
   before the deferred actions run.
7. **Panic exit status.** Go exits 2 on panic and deliberately skips closing
   the database. Rust's default is 101 with unwinding; choose abort or a
   handler that preserves "do not touch the database after a panic".
8. **Local time.** Timestamps are local time; see the dcroxide note on
   resolving the offset once per process.
9. **Localhost resolution** decides the default listener set; inject it for
   tests and keep Go's "IPv4 listeners first" bind order (spec 16).
10. **External solver.** The Go spawns `csppsolver` and speaks Go `net/rpc`
    (gob) over its stdio. The port either implements that client, or embeds
    a solver and keeps `--csppsolver` as an accepted but unused option
    (section 13).
11. **Windows.** Option style `/x` and `/name:value`, help with `/` and `:`,
    handles instead of descriptors for the pipes, backslash-aware `~`
    expansion, no SIGTERM/SIGHUP.

### 12.4 Size

**L.** The logic in scope is about 4,400 lines of Go, but observable
behaviour is dominated by library semantics that have to be re-implemented:
go-flags (about 3,500 lines read), slog and the rotator, Go path/number/quote
formatting, terminal password entry, and descriptor inheritance on three
operating systems. dcroxide provides working models for most of these, which
removes the research risk; the work is generalising them, building the
81-option registry and the 39-step pipeline, and producing oracle vectors for
a surface that has no upstream tests.

## 13. Open questions

1. **Decrediton's exact invocation** (which of `--pipetx`, `--piperx`,
   `--rpclistenerevents`, `--issueclientcert`, `--noinitialload`,
   `--onetimetlskey`, port-0 listeners it passes, and whether it parses
   stdout) was not verified: Decrediton's source is not among the pinned
   inputs. Settle by reading Decrediton's wallet launcher at the release that
   pairs with dcrwallet 2.2 and by capturing a live launch.
2. **Release build metadata.** `version.BuildMetadata` and `PreRelease` are
   set at link time for official releases (`version/version.go:29-38`); the
   values used by the release tooling are not in this repository. Settle from
   the decred-release build scripts. Decide what the port reports: the
   upstream version it is compatible with, its own, or both.
3. **Go runtime fields in `-V` and the startup line.** UNVERIFIED whether any
   consumer parses `(Go version goX.Y.Z os/arch)`. Decide the replacement
   text.
4. **Profiling options.** Decide between accepting-and-ignoring (dcroxide's
   choice), serving a Rust-appropriate profile endpoint, or rejecting.
5. **Prompt behaviour at EOF** (quirk 14): reproduce the infinite loop or
   fail fast. Scripted callers that rely on EOF only do so at the optional
   prompts, which already handle it.
6. **Exit status 0 on cancelled creation and on `--createtemp` misuse**
   (quirks 10, 12): reproduce or correct.
7. **Windows console events.** Go's `os/signal` documentation states that
   close, logoff and shutdown console events are delivered as `SIGTERM` once
   `Notify` has been called; the wallet registers only `os.Interrupt` on
   Windows. UNVERIFIED what the pinned Go runtime does with those events in
   this configuration. Settle with a Windows test of the Go binary.
8. **`csppsolver`**: external process with Go `net/rpc` framing, or built in.
   If external, the gob wire format of `Solver.Roots`/`Solver.RootFactors`
   becomes part of spec 12's contract.
9. **Whether `chain.Syncer.Run` or `spv.Syncer.Run` can return nil** (quirk
   34) is for specs 15 and 14.
10. **`MixSplitLimit` omission in `CreateNewWallet`** (quirk 36): reproduce
    or fix; depends on what a zero limit does in the mixing integration
    (spec 13).
11. **Binary and directory naming.** A drop-in replacement must answer to
    `dcrwallet`, read `dcrwallet.conf` and use the `Dcrwallet`/`.dcrwallet`
    directory. dcroxide chose its own names for dcrd's equivalents; the
    wallet cannot if it is to open existing installations in place. Confirm.
12. **Help text parity**: byte-exact (requires the terminal-width probe and
    wrap algorithm) or semantically equivalent.

