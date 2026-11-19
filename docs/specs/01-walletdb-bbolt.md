# 01 — Database layer: walletdb interface, the bdb driver, and the bbolt on-disk file format

## 1. Header

**Scope (dcrwallet, line counts from the parity pin):**

| File | Lines | Notes |
| --- | --- | --- |
| `wallet/walletdb/interface.go` | 323 | transaction/bucket/cursor interfaces, `View`, `Update`, driver registry |
| `wallet/walletdb/doc.go` | 102 | package documentation (stale, see section 10) |
| `wallet/walletdb/db_test.go` | 142 | registry tests |
| `wallet/walletdb/interface_test.go` | 709 | interface conformance test |
| `wallet/walletdb/example_test.go` | 175 | disabled by `//go:build disabled` |
| `wallet/internal/bdb/db.go` | 323 | bbolt adapter, error conversion, `openDB` |
| `wallet/internal/bdb/driver.go` | 68 | `"bdb"` driver registration and argument parsing |
| `wallet/internal/bdb/doc.go` | 26 | package documentation |
| `wallet/internal/bdb/driver_test.go` | 159 | driver tests |
| `wallet/internal/bdb/interface_test.go` | 720 | copy of the conformance test |
| `wallet/drivers/bdb/driver.go` | 16 | blank import that registers the driver |
| `wallet/db.go` | 51 | opaque `wallet.DB`, `OpenDB`, `CreateDB` |
| `internal/loader/loader.go` | 415 | database create/open/close paths only |
| `walletsetup.go` | 388 | database create paths only (`networkDir`, `createWallet`, `createSimulationWallet`, `createWatchingOnlyWallet`, `checkCreateDir`) |
| `config.go` (lines 69, 590-665) | — | database path, existence check, creation dispatch |
| `dcrwallet.go` (lines 163-225) | — | loader construction, open, deferred unload |

**Scope (dependency, read in full):** `go.etcd.io/bbolt@v1.3.12` non-test sources:
`db.go` (1397), `tx.go` (844), `bucket.go` (799), `node.go` (610), `cursor.go` (429),
`freelist.go` (410), `freelist_hmap.go` (178), `page.go` (212), `tx_check.go` (226),
`compact.go` (119), `errors.go` (78), `unsafe.go` (27), `bolt_unix.go` (87),
`bolt_linux.go` (10), `boltsync_unix.go` (9), `bolt_windows.go` (117),
`bolt_openbsd.go` (16), `bolt_unix_solaris.go` (88), `bolt_unix_aix.go` (91),
`mlock_unix.go` (37), `mlock_windows.go` (11), and the per-architecture constant files
`bolt_<arch>.go`.

**Parity pin:** dcrwallet `54b85b33` (2.2.0-pre, module `decred.org/dcrwallet/v5`);
`go.etcd.io/bbolt v1.3.12` (`go.mod:38`). The `chaincfg/v3 v3.3.0` and `wire v1.8.0`
modules were consulted only for network names.

**Summary.**

- The wallet stores everything in one file, `<appdata>/<network>/wallet.db`, a bbolt
  (BoltDB) B+tree file. dcrwallet reaches it only through the `walletdb` interfaces;
  the single registered driver, `"bdb"`, is a thin cast-through adapter over bbolt.
- dcrwallet opens the file with `bolt.Open(path, 0600, nil)`: all bbolt options are the
  defaults (array freelist, freelist synced to disk, fsync on, no lock timeout, OS page
  size for new files, 0.5 fill percent). There is no read-only open anywhere.
- The file is a sequence of fixed-size pages: two alternating meta pages, a freelist
  page, and copy-on-write branch/leaf pages. All integers are host-endian
  (little-endian on every platform dcrwallet ships for). Buckets nest; small buckets
  are stored inline in their parent's leaf.
- A drop-in replacement must (a) read any existing `wallet.db` whatever its page size,
  (b) write files that bbolt v1.3.12 accepts without panicking or misreading, including
  a correct freelist, and (c) take the same advisory file lock so that it and a Go
  dcrwallet exclude each other.
- Byte-identical output is not achievable in general (bbolt's own page assignment
  depends on Go map iteration order) and is not required; section 9 separates the
  cross-compatibility requirements from the byte-layout policies.
- The transaction API has observable semantics the upper layers depend on: snapshot
  isolation for readers, one serialized writer, nil-versus-empty values, cursor
  behaviour at the ends and under mutation, `KeyN` counting committed pages only, and
  the mapping of bbolt errors onto wallet error kinds.

---

## 2. Role in the system

`walletdb` is the storage abstraction of the wallet. Every persistent structure
specified elsewhere (address manager, transaction store, stake store, VSP data,
agenda preferences, metadata/versioning) lives in buckets of a single bbolt file and
is read or written inside a `walletdb` transaction.

Callers:

- `wallet/udb` (all bucket layouts, migrations and upgrades) and `wallet` itself:
  165 call sites of `walletdb.View`/`walletdb.Update` outside tests, plus three manual
  `BeginReadTx`/`BeginReadWriteTx` sites (`wallet/addresses.go:476`,
  `wallet/addresses.go:527`, `wallet/createtx.go:197`).
- `wallet.Create`, `wallet.CreateWatchOnly`, `wallet.Open` receive a `wallet.DB`
  (opaque wrapper) and unwrap it with the unexported `internal()` method
  (`wallet/wallet.go:5308`, `:5318`, `:5343`).
- `internal/loader` creates/opens/closes the database file for the daemon and for the
  gRPC `WalletLoaderService`; `walletsetup.go` creates it for `--createtemp` and
  `--createwatchingonly`.

Callees: `go.etcd.io/bbolt` only. bbolt in turn uses the operating system for
`open`, `flock` (or `LockFileEx`, or `fcntl` locks on Solaris/AIX), `mmap`, `pwrite`,
`ftruncate`, `fsync`/`fdatasync`.

Layering:

```
wallet, wallet/udb            (bucket layouts; other specs)
   |  walletdb.View / Update / Begin*Tx
wallet/walletdb               (interfaces, managed-transaction helpers, driver registry)
   |  Driver{DbType:"bdb"}
wallet/internal/bdb           (adapter: type conversions + error-kind mapping)
   |  bolt.Open / Tx / Bucket / Cursor
go.etcd.io/bbolt v1.3.12      (B+tree, MVCC, freelist, file format)
   |
wallet.db
```

---

## 3. Public surface inventory

### 3.1 `wallet/walletdb` (`wallet/walletdb/interface.go`)

| Item | Signature | Contract |
| --- | --- | --- |
| `ReadTx` (`:21-29`) | `ReadBucket(key []byte) ReadBucket`; `Rollback() error` | Read-only transaction. `ReadBucket` opens a top-level bucket; returns nil when the key is absent or is not a bucket. `Rollback` ends the transaction. |
| `ReadWriteTx` (`:33-52`) | embeds `ReadTx`; `ReadWriteBucket(key) ReadWriteBucket`; `CreateTopLevelBucket(key) (ReadWriteBucket, error)`; `DeleteTopLevelBucket(key) error`; `Commit() error` | Read-write transaction. `CreateTopLevelBucket` fails if the bucket exists (doc comment says "if it does not exist"; the bdb implementation calls bbolt `CreateBucket`, which errors with `Exist`). `DeleteTopLevelBucket` errors if absent or not a bucket. |
| `ReadBucket` (`:56-87`) | `NestedReadBucket(key) ReadBucket`; `ForEach(func(k, v []byte) error) error`; `Get(key) []byte`; `KeyN() int`; `ReadCursor() ReadCursor` | `ForEach` visits keys in ascending byte order; nested buckets are reported with `v == nil`. `Get` returns nil for an absent key and for a nested-bucket key. Returned slices are valid only during the transaction. `KeyN`: see section 5.12. |
| `ReadWriteBucket` (`:91-128`) | embeds `ReadBucket`; `NestedReadWriteBucket(key) ReadWriteBucket`; `CreateBucket(key) (ReadWriteBucket, error)`; `CreateBucketIfNotExists(key) (ReadWriteBucket, error)`; `DeleteNestedBucket(key) error`; `Put(key, value) error`; `Delete(key) error`; `ReadWriteCursor() ReadWriteCursor` | Mutating operations; error kinds in section 8. `Delete` of an absent key is not an error. |
| `ReadCursor` (`:133-158`) | `First()`, `Last()`, `Next()`, `Prev()`, `Seek(seek []byte)` each returning `(key, value []byte)`; `Close()` | Ordered iteration. End of iteration is `key == nil`. Nested buckets yield `value == nil`. `Close` is a no-op in bdb. |
| `ReadWriteCursor` (`:164-171`) | embeds `ReadCursor`; `Delete() error` | Deletes the pair under the cursor; errors with `Invalid` when the cursor is on a nested bucket. |
| `BucketIsEmpty` (`:175-180`) | `func(bucket ReadBucket) bool` | Opens a cursor, calls `First`, closes it, returns `k == nil && v == nil`. |
| `DB` (`:184-197`) | `BeginReadTx() (ReadTx, error)`; `BeginReadWriteTx() (ReadWriteTx, error)`; `Copy(w io.Writer) error`; `Close() error` | Database handle. `Copy` has no caller in dcrwallet outside the interface itself. |
| `View` (`:203-223`) | `func(ctx, db DB, f func(tx ReadTx) error) error` | Managed read transaction; section 5.13. |
| `Update` (`:230-255`) | `func(ctx, db DB, f func(tx ReadWriteTx) error) (err error)` | Managed read-write transaction; section 5.13. |
| `Driver` (`:259-271`) | `struct{ DbType string; Create func(args ...any) (DB, error); Open func(args ...any) (DB, error) }` | Backend descriptor. |
| `drivers` (`:274`) | `map[string]*Driver` (unexported) | Registry; not mutex-protected; written only from package `init` functions. |
| `RegisterDriver` (`:279-287`) | `func(driver Driver) error` | Errors with op `walletdb.RegisterDriver`, kind `Exist`, text `driver "<name>" is already registered` (`%q` formatting) when the type is taken; otherwise stores a copy. |
| `SupportedDrivers` (`:291-297`) | `func() []string` | Registered type names in Go map iteration order (unordered). |
| `Create` (`:302-310`) | `func(dbType string, args ...any) (DB, error)` | Unknown type: op `walletdb.Create`, kind `Invalid`, text `driver "<name>" is not registered`. Otherwise returns `drv.Create(args...)` unwrapped. |
| `Open` (`:315-323`) | `func(dbType string, args ...any) (DB, error)` | Same with op `walletdb.Open` and `drv.Open`. |

### 3.2 `wallet/internal/bdb` (`wallet/internal/bdb/db.go`, `driver.go`)

All types are unexported; behaviour is observable through the interfaces above.

| Item | Contract |
| --- | --- |
| `convertErr(err) error` (`db.go:18-36`) | nil stays nil. Otherwise wraps with a kind chosen by identity comparison (`switch err`, not `errors.Is`) against bbolt sentinels; table in section 8.1. |
| `transaction{boltTx *bolt.Tx}` (`db.go:41-43`) | One type implements both `ReadTx` and `ReadWriteTx`. |
| `(*transaction).ReadBucket` / `ReadWriteBucket` (`db.go:45-55`) | `boltTx.Bucket(key)`; nil pointer is converted to a nil interface. `ReadBucket` delegates to `ReadWriteBucket`. |
| `(*transaction).CreateTopLevelBucket` (`db.go:57-63`) | `boltTx.CreateBucket(key)`; error through `convertErr`. |
| `(*transaction).DeleteTopLevelBucket` (`db.go:65-71`) | `boltTx.DeleteBucket(key)`; error through `convertErr`. |
| `(*transaction).Commit` / `Rollback` (`db.go:77-87`) | `convertErr(boltTx.Commit())` / `convertErr(boltTx.Rollback())`. |
| `bucket` = `bolt.Bucket` (`db.go:91`) | Pointer-cast adapter. Methods: `NestedReadWriteBucket` (`:100-107`), `NestedReadBucket` (`:109-111`), `CreateBucket` (`:118-124`), `CreateBucketIfNotExists` (`:131-137`), `DeleteNestedBucket` (`:142-144`), `ForEach` (`:155-157`, callback errors pass through `convertErr`), `Put` (`:163-165`), `Get` (`:175-177`), `Delete` (`:183-185`), `KeyN` (`:190-192`, `Stats().KeyN`), `ReadCursor` (`:194-196`), `ReadWriteCursor` (`:202-204`). |
| `cursor` = `bolt.Cursor` (`db.go:213`) | `Delete` (`:219-221`), `First`/`Last`/`Next`/`Prev`/`Seek` (`:226-257`) pass straight through; `Close` (`:262`) does nothing. |
| `db` = `bolt.DB` (`db.go:267`) | `beginTx(writable)` (`:272-278`) calls `bolt.DB.Begin`; `BeginReadTx` (`:280`), `BeginReadWriteTx` (`:284`); `Copy` (`:292-296`) runs `tx.Copy(w)` inside bbolt's managed `View`; `Close` (`:301-303`). |
| `fileExists(name) bool` (`db.go:306-313`) | `os.Stat`; false only when the error is "not exist". Any other stat error (for example permission denied) reports true. |
| `openDB(dbPath, create) (walletdb.DB, error)` (`db.go:316-323`) | If `!create` and `!fileExists(dbPath)`: error kind `NotExist`, text `missing database file`, no op. Otherwise `bolt.Open(dbPath, 0600, nil)` and `convertErr`. Note the returned `DB` interface is non-nil (it wraps a nil `*bolt.DB`) even when the error is non-nil. |
| `dbType = "bdb"` (`driver.go:15-17`) | Driver name. |
| `parseArgs(funcName, args...) (string, error)` (`driver.go:20-33`) | Exactly one argument of type `string`. Errors are plain `fmt` errors with no kind: `invalid arguments to bdb.<Open\|Create> -- expected database path` (wrong count) and `first argument to bdb.<Open\|Create> is invalid -- expected database path string` (wrong type). |
| `openDBDriver` / `createDBDriver` (`driver.go:37-55`) | `parseArgs` then `openDB(path, false)` / `openDB(path, true)`. |
| `init` (`driver.go:57-68`) | Registers `Driver{DbType:"bdb", Create:createDBDriver, Open:openDBDriver}`; panics with `Failed to register database driver 'bdb': <err>` on failure. |
| compile-time assertions (`db.go:94`, `:270`) | `var _ walletdb.ReadWriteBucket = (*bucket)(nil)`, `var _ walletdb.DB = (*db)(nil)`. No behaviour. |
| `doc.go` (26 lines) | Package comment only: the driver type is `"bdb"` and `Open`/`Create` take the database path as their single string argument. |

Adapter details that are observable and easy to miss:

- **`ForEach` wraps the callback's error.** `bucket.ForEach` returns
  `convertErr(boltBucket.ForEach(fn))` (`db.go:155-157`). An error returned by the
  callback therefore comes back as a *new* `*errors.Error` value: a plain error `e`
  becomes `&errors.Error{Err: e}` (no op, kind `Other`; `Error()` text identical to
  `e`'s), and an `*errors.Error` is collapsed into a copy with the same op, kind and
  cause (`errors/errors.go:187-232`). The result is never pointer-identical (`==`) to
  what the callback returned; callers must match with `errors.Is`/`errors.As`. A
  callback error that happens to be identical to a bbolt sentinel receives that
  sentinel's kind. The cursor methods and `Get` return bbolt's results unwrapped.
- `transaction.ReadBucket`, `bucket.NestedReadBucket` and `bucket.ReadCursor` call
  the read-write variants and convert the result to the read interface; a nil
  bucket stays a nil interface in both conversions (`db.go:45-55`, `:100-111`).
- `CreateBucket`/`CreateBucketIfNotExists`/`CreateTopLevelBucket` return a nil
  interface together with the converted error (`db.go:57-63`, `:118-137`).
- `KeyN`, `Get`, `ReadBucket`, `NestedReadWriteBucket` and the cursor constructors
  have no error result; on a finished transaction the first four do not return an
  error but panic or fault (section 5.7). Constructing a cursor on a finished
  transaction succeeds (`bbolt@v1.3.12/bucket.go:82-91`); its moves then panic.

### 3.3 `wallet/drivers/bdb` (`wallet/drivers/bdb/driver.go`)

A blank import of `wallet/internal/bdb` (`:16`). Importing it makes `"bdb"` available
to `wallet.OpenDB`/`wallet.CreateDB`. `internal/loader/loader.go:16` and
`walletsetup.go:22` import it.

### 3.4 `wallet/db.go`

| Item | Contract |
| --- | --- |
| `DB` interface (`:15-22`) | `io.Closer` plus unexported `internal() walletdb.DB`. Opaque to other packages. |
| `opaqueDB{walletdb.DB}` (`:24-28`) | The only implementation. |
| `OpenDB(driver string, args ...any) (DB, error)` (`:32-39`) | `walletdb.Open`; error wrapped with op `wallet.OpenDB`. |
| `CreateDB(driver string, args ...any) (DB, error)` (`:44-51`) | `walletdb.Create`; error wrapped with op `wallet.CreateDB`. |

### 3.5 `internal/loader` — database-related surface (`internal/loader/loader.go`)

| Item | Contract |
| --- | --- |
| `walletDbName = "wallet.db"`, `driver = "bdb"` (`:21-24`) | File name inside the network directory; driver name. `config.go:69` defines the same file name for package `main`. |
| `Loader` (`:33-54`), `NewLoader` (`:57-77`) | Holds `dbDirPath`, the loaded `*wallet.Wallet`, its `wallet.DB`, wallet options, and a mutex. |
| `CreateWatchingOnlyWallet` (`:108-193`), `CreateNewWallet` (`:198-282`) | Create the directory and database, initialise, open. Section 5.15. |
| `OpenExistingWallet` (`:288-342`) | Opens the database (mutex released during the open), then `wallet.Open`. Section 5.15. |
| `NewLoader` parameters (`:57-77`) | Positional, in this order: `chainParams`, `dbDirPath`, `votingEnabled`, `gapLimit`, `watchLast`, `allowHighFees`, `relayFee`, `vspMaxFee`, `accountGapLimit`, `disableCoinTypeUpgrades`, `mixingEnabled`, `manualTickets`, `mixSplitLimit`, `dialer`. Stored unchanged and copied into the `wallet.Config` of each load as `DB`, `PubPassphrase`, `VotingEnabled`, `GapLimit`, `WatchLast`, `AccountGapLimit`, `DisableCoinTypeUpgrades`, `MixingEnabled`, `ManualTickets`, `AllowHighFees`, `RelayFee`, `VSPMaxFee`, `MixSplitLimit` (not in `CreateNewWallet`, Q15), `Params`, `Dialer` (`:169-185`, `:259-274`, `:318-334`). Both construction sites pass the same configuration values (`dcrwallet.go:168-172`, `walletsetup.go:115-119`). The loader does no validation of any of them. |
| `DbDirPath` (`:345-347`) | Returns `dbDirPath`. |
| `WalletExists` (`:351-359`) | Whether `<dbDirPath>/wallet.db` exists; stat errors other than not-exist are returned with op `loader.WalletExists`. |
| `LoadedWallet` (`:364-369`), `RunAfterLoad` (`:94-104`), `NetworkBackend` (`:397-404`) | Not database related; listed for completeness. |
| `UnloadWallet` (`:375-393`) | Closes the database. Section 5.15. |
| `fileExists` (`:406-415`) | `(bool, error)` variant: not-exist → `(false, nil)`, other errors returned. |
| `onLoaded(w, db)` (`:81-89`, unexported) | Runs every registered callback in registration order on the calling goroutine with `mu` held, then records `wallet` and `db`, then sets `callbacks = nil`. The callbacks are **not** restored by `UnloadWallet`. |
| `RunAfterLoad(fn)` (`:94-104`) | If a wallet is loaded: unlock `mu`, then call `fn(w)` synchronously. Otherwise append `fn` to `callbacks`. |
| `log`, `UseLogger(logger)` (`internal/loader/log.go:13-20`) | Package logger, `slog.Disabled` until the daemon installs the `LODR` subsystem logger (`log.go:31`, `internal/loggers/loggers.go:47`). The only log call in the package is `Failed to open database: <err>` at error level (`loader.go:305`). |
| `doc.go` (11 lines) | Package comment only. |

### 3.6 `walletsetup.go` / `config.go` — database-related surface

| Item | Contract |
| --- | --- |
| `networkDir(dataDir, chainParams) string` (`walletsetup.go:34-44`) | `filepath.Join(dataDir, netname)`; `netname` is `chainParams.Name`, overridden to `testnet2` for net `0x48e7a065` and `testnet3` for `wire.TestNet3`. Result: `mainnet`, `testnet3`, `simnet` (names from `dcrd/chaincfg/v3@v3.3.0/mainnetparams.go:86`, `testnetparams.go:81`, `simnetparams.go:90`). |
| `checkCreateDir(path) error` (`walletsetup.go:371-388`) | Stat; if missing `os.MkdirAll(path, 0700)`; errors `cannot create directory: <err>`, `error checking directory: <err>`, `path '<path>' is not a directory`. |
| `createWallet` (`walletsetup.go:113-266`) | Interactive creation through `Loader.CreateNewWallet`, ending with `loader.UnloadWallet()`. |
| `createSimulationWallet` (`walletsetup.go:270-312`) | `wallet.CreateDB("bdb", dbPath)` directly, `defer db.Close()`. |
| `createWatchingOnlyWallet` (`walletsetup.go:328-367`) | `wallet.CreateDB("bdb", dbPath)` directly, `defer db.Close()`; on `wallet.CreateWatchOnly` failure removes the file (before the deferred close runs) and prints the removal error, if any, to stdout. |
| `walletDbName = "wallet.db"` (`config.go:69`) | Package `main`'s own copy of the file name, used by `config.go:593`, `walletsetup.go:294`, `:346`; must stay equal to the loader's (`internal/loader/loader.go:22`). |
| `displaySimnetMiningAddrs(seed, imported)` (`walletsetup.go:49-106`), `promptHDPublicKey(reader)` (`walletsetup.go:315-324`) | Not database related; listed for completeness. The first prints simnet mining addresses after `--create` when `--simnet` is set (`walletsetup.go:248-249`); the second prints the prompt `Enter HD wallet public key: ` (no newline) and returns the line read from stdin with surrounding white space trimmed. Both belong to the wallet-creation wizard spec. |
| `--createtemp` preconditions (`config.go:575-589`, messages at `:578`, `:586`) | Checked before the path is computed: without an explicitly set `--appdata`, stderr `Tried to create a temporary simulation wallet, but failed to specify data directory!` and `os.Exit(0)`; without `--simnet`, stderr `Tried to create a temporary simulation wallet for network other than simnet!` and `os.Exit(0)`. Exit status 0 in both cases, and no database file is touched. |
| Path and existence check (`config.go:592-606`) | `dbPath = filepath.Join(networkDir(appdata, params), "wallet.db")`; existence via `cfgutil.FileExists` (empty path → false). |
| Creation dispatch (`config.go:608-665`) | `--createtemp` together with `--create`: error `The flags --create and --createtemp can not be specified together. Use --help for more information.` (checked first, `config.go:595-600`). `--createtemp`: if the file exists prints `The wallet already exists. Loading this wallet instead.` to stdout; `checkCreateDir` runs in both cases; `createSimulationWallet` runs only when the file did not exist; config loading then continues (no exit). `--create`/`--createwatchingonly`: error `The wallet database file `<path>` already exists.` if present; else `checkCreateDir`, run the wizard, `os.Exit(0)`. Neither flag and no file and not `--noinitialload`: error `The wallet does not exist.  Run with the --create option to initialize and create it.` Every one of these errors is printed to stderr on its own line and returned through `loadConfigError`; a stat failure other than not-exist in the existence check is printed and returned the same way (`config.go:602-606`). A failure of any of the three creation functions is printed as `Unable to create wallet: <err>` (stderr) and returned (`config.go:626-629`, `:654-657`). `os.Stdout.Sync()` is called before the `--create`/`--createwatchingonly` wizard starts (`config.go:648`). |

### 3.7 bbolt API reachable from dcrwallet

Only the following bbolt entry points are reachable through the bdb adapter. Everything
else in bbolt (`Batch`, `Update`, `Sync`, `Stats`, `Info`, `Tx.ForEach`, `Tx.OnCommit`,
`Tx.Check`, `Tx.CopyFile`, `Tx.WriteTo` flags, `Bucket.Sequence`/`SetSequence`/
`NextSequence`, `Bucket.FillPercent`, `Compact`, read-only mode, `NoSync`,
`NoFreelistSync`, the hashmap freelist, `Mlock`, `InitialMmapSize`, `PageSize`,
`StrictMode`) is unreachable and need not be exposed, but several of them define file
states a reader must tolerate (section 4).

| bbolt call | Reached from |
| --- | --- |
| `bolt.Open(path, 0600, nil)` | `openDB` |
| `(*DB).Begin(writable)` | `beginTx` |
| `(*DB).View` + `(*Tx).Copy` | `db.Copy` |
| `(*DB).Close` | `db.Close` |
| `(*Tx).Bucket`, `CreateBucket`, `DeleteBucket`, `Commit`, `Rollback` | `transaction` methods |
| `(*Bucket).Bucket`, `CreateBucket`, `CreateBucketIfNotExists`, `DeleteBucket`, `ForEach`, `Put`, `Get`, `Delete`, `Stats`, `Cursor` | `bucket` methods |
| `(*Cursor).First`, `Last`, `Next`, `Prev`, `Seek`, `Delete` | `cursor` methods |

Options in effect (`bbolt@v1.3.12/db.go:197-230`, `:1290-1294`): `Timeout = 0` (wait
forever for the file lock), `NoGrowSync = false`, `NoFreelistSync = false`,
`FreelistType = "array"`, `ReadOnly = false`, `MmapFlags = 0`, `InitialMmapSize = 0`,
`PageSize = 0` (OS page size for a new file), `NoSync = false`, `OpenFile = os.OpenFile`,
`Mlock = false`; `PreLoadFreelist` is forced true for a writable open;
`MaxBatchSize = 1000`, `MaxBatchDelay = 10ms`, `AllocSize = 16 MiB`;
`StrictMode = false`. Every bucket uses `FillPercent = 0.5`
(`bbolt@v1.3.12/bucket.go:26`, `:56`) because the adapter never changes it.

### 3.8 bbolt v1.3.12 surface that dcrwallet cannot reach (complete list)

Listed so that nothing in the dependency is silently unaccounted for. None of it has
to exist in the port; the right-hand column states what, if anything, it implies for
a file the port must read.

| Item (`bbolt@v1.3.12/`) | What it is | Consequence for the port |
| --- | --- | --- |
| `DB.Path`, `DB.GoString` (`bolt.DB{path:"…"}`), `DB.String` (`DB<"…">`), `DB.IsReadOnly`, `DB.Info`, `DB.Stats`, `Stats.Sub` (`db.go:180-192`, `:1063-1074`, `:1183`, `:1296-1330`) | accessors and counters | none |
| `DB.Update`, `DB.Batch` (+ `batch`, `call`, `panicked`, `safelyCall`), `DB.Sync` (`db.go:869-1059`) | managed write transactions, batching goroutine/timer, manual sync. `DB.View` **is** reached, by `Copy` (section 4.12). | none |
| Public fields `DB.StrictMode`, `NoSync`, `NoFreelistSync`, `FreelistType`, `NoGrowSync`, `PreLoadFreelist`, `MmapFlags`, `MaxBatchSize`, `MaxBatchDelay`, `AllocSize`, `Mlock` (`db.go:59-138`) and `Options` (`db.go:1222-1286`) | tuning; dcrwallet leaves every one at its default | a file last written by another tool with `NoFreelistSync` has `meta.freelist = pgidNoFreelist` (section 4.4); a file created with `Options.PageSize` has an arbitrary page size (section 5.3). `FreelistType` (`"array"`/`"hashmap"`, `freelist_hmap.go`) changes only the in-memory allocator — the hashmap allocator does not guarantee lowest-id-first — and has no on-disk representation. |
| `Tx.ID`, `Tx.DB`, `Tx.Size`, `Tx.Writable`, `Tx.Cursor`, `Tx.Stats`, `Tx.CreateBucketIfNotExists`, `Tx.ForEach`, `Tx.OnCommit`, `Tx.WriteTo`, `Tx.CopyFile`, `Tx.Page`, `Tx.WriteFlag`, `Tx.Check`, `Tx.CheckWithOptions`, `WithKVStringer`, `KVStringer`, `HexKVStringer` (`tx.go`, `tx_check.go`) | transaction accessors, copy-to-file, page info, consistency check | `Tx.Copy` (reached) is `Tx.WriteTo` with `WriteFlag == 0`. `Tx.Check` is the reference for the port's own checker (section 5.18). |
| `Bucket.Tx`, `Bucket.Root`, `Bucket.Writable`, `Bucket.Sequence`, `Bucket.SetSequence`, `Bucket.NextSequence`, `Bucket.ForEachBucket` (used internally by `DeleteBucket`), `Bucket.FillPercent`, `BucketStats` fields other than `KeyN`, `BucketStats.Add` (`bucket.go`) | sequence counter, fill tuning, statistics | bucket `sequence` is never changed by dcrwallet; it must be carried over unchanged whenever a bucket header is rewritten (section 4.8). |
| `Cursor.Bucket` (`cursor.go:25`) | accessor | none |
| `TxStats` and its `Get*`/`Inc*` methods, `PageInfo` (`tx.go:638-844`, `page.go:144-150`) | statistics | none |
| `Compact`, `walk`, `walkBucket` (`compact.go`) | copy one database into another | a compacted wallet file is an ordinary file; non-default `FillPercent` used by such tools only changes page fill. |
| `mlock`/`munlock`/`mrelock` (`mlock_unix.go`, `mlock_windows.go`, `db.go:570-596`) | memory locking | none |
| `page.typ`, `page.hexdump`, commented-out `node.dump`, `compareKeys` (`page.go:38-49`, `:99-102`, `node.go:561-590`) | debugging helpers; `typ` strings are `branch`, `leaf`, `meta`, `freelist`, `unknown<%02x>` and appear in panic messages | none |
| `ErrDatabaseOpen`, `ErrTimeout`, `ErrDatabaseReadOnly`, `ErrFreePagesNotLoaded` (`errors.go`) | sentinels with no reachable producer | section 8.1 |
| `cmd/bbolt`, `internal/guts_cli`, `internal/surgeon`, `internal/btesting`, `tests/` | command-line tool and test helpers | the `bbolt` tool (`check`, `pages`, `page`, `page-item`, `dump`, `buckets`, `keys`, `get`, `info`, `stats`, `compact`, `surgery`, `bench`; `cmd/bbolt/main.go:111-138`) is a convenient external oracle for files written by the port. |

---

## 4. Data formats

### 4.1 Conventions

- **Byte order.** bbolt writes its structures by casting memory, so every multi-byte
  integer is in the byte order of the machine that wrote the file
  (`bbolt@v1.3.12/db.go:1077-1085`, `page.go:52-54`). Every architecture dcrwallet
  releases for is little-endian; this spec states all layouts as **little-endian** and
  the Rust implementation must read and write little-endian explicitly. A file written
  on a big-endian host has magic bytes `ED 0C DA ED` at meta offset 0 and must be
  rejected exactly as bbolt on a little-endian host rejects it (magic mismatch →
  `ErrInvalid`).
- **Page.** The file is an array of pages of `pageSize` bytes; page `id` starts at file
  offset `id * pageSize`. `pageSize` is recorded in the meta pages (section 4.3) and is
  whatever the creating machine's OS page size was (4096 on x86-64 Linux and Windows,
  16384 on Apple Silicon macOS, kernel-dependent on Linux arm64 — platform facts, not
  from the bbolt source). A reader must take it from the file, never assume 4096.
- **Page id** (`pgid`) and **transaction id** (`txid`) are `u64`.
- **Alignment.** Page headers and element headers are 8-byte aligned in the file.
  Bucket values (section 4.8) are *not* aligned; bbolt copies them when misaligned
  (`bucket.go:126-134`). A Rust reader decodes fields from byte slices and has no
  alignment concern.
- **Padding.** Bytes of a page not covered by the structures below are zero when
  written by bbolt (buffers come from a zeroed pool or `make`,
  `db.go:1112-1119`, `tx.go:529-535`). Readers must not depend on that. A Rust writer
  must zero-fill, so that no heap contents reach the wallet file.

### 4.2 Page header (16 bytes)

`bbolt@v1.3.12/page.go:30-35`. Present at the start of every page (and of the embedded
page of an inline bucket).

| Offset | Size | Field | Meaning |
| --- | --- | --- | --- |
| 0 | 8 | `id` | Page id of this page. Must equal its position (`fileOffset / pageSize`). 0 for an inline-bucket page. |
| 8 | 2 | `flags` | Exactly one of: `0x01` branch, `0x02` leaf, `0x04` meta, `0x10` freelist (`page.go:17-22`). |
| 10 | 2 | `count` | Number of elements (branch/leaf), number of page ids (freelist, with the `0xFFFF` escape of section 4.4), 0 for meta. |
| 12 | 4 | `overflow` | Number of additional contiguous pages that belong to this logical page. The logical page occupies `overflow + 1` physical pages; only the first has a header. |

bbolt asserts on every branch/leaf page it loads through a transaction that
`id` equals the requested id and that `flags` is exactly one of the four values
(`page.go:56-64`, `tx.go:570-583`); a violation panics the Go process.

### 4.3 Meta pages (pages 0 and 1)

Page header (`flags = 0x04`, `count = 0`, `overflow = 0`, `id` = 0 or 1) followed at
page offset 16 by the 64-byte meta structure (`bbolt@v1.3.12/db.go:1337-1347`):

| Page offset | Meta offset | Size | Field | Value / meaning |
| --- | --- | --- | --- | --- |
| 16 | 0 | 4 | `magic` | `0xED0CDAED` (bytes `ED DA 0C ED`) (`db.go:23`) |
| 20 | 4 | 4 | `version` | `2` (`db.go:20`) |
| 24 | 8 | 4 | `pageSize` | page size in bytes |
| 28 | 12 | 4 | `flags` | always 0; never read |
| 32 | 16 | 8 | `root.root` | page id of the root bucket's root page |
| 40 | 24 | 8 | `root.sequence` | root bucket sequence; always 0 in dcrwallet files; must be preserved |
| 48 | 32 | 8 | `freelist` | page id of the freelist page, or `0xFFFFFFFFFFFFFFFF` (`pgidNoFreelist`, `db.go:25`) when the freelist is not persisted |
| 56 | 40 | 8 | `pgid` | high-water mark: one past the highest page id ever allocated. Pages `[0, pgid)` are in use or free; nothing at or beyond it is referenced. |
| 64 | 48 | 8 | `txid` | id of the transaction that wrote this meta |
| 72 | 56 | 8 | `checksum` | FNV-1a 64-bit hash of meta bytes `[0, 56)` (`db.go:1386-1390`) |

Remaining bytes of the page are zero.

**Checksum.** FNV-1a, 64-bit: `h = 0xcbf29ce484222325`; for each byte `b`:
`h ^= b; h = h * 0x100000001b3 mod 2^64`. Stored little-endian. Test vectors (the two
metas bbolt writes when it initialises a new file; `root.root = 3`, `freelist = 2`,
`pgid = 4`):

| pageSize | txid | meta bytes `[0,56)` (hex) | checksum value | stored bytes |
| --- | --- | --- | --- | --- |
| 4096 | 0 | `edda0ced 02000000 00100000 00000000 0300000000000000 0000000000000000 0200000000000000 0400000000000000 0000000000000000` | `0x07516e114689fdee` | `ee fd 89 46 11 6e 51 07` |
| 4096 | 1 | same with txid `0100000000000000` | `0x264c351a5179480f` | `0f 48 79 51 1a 35 4c 26` |
| 16384 | 0 | pageSize `00400000` | `0x90f347e30dddf37e` | `7e f3 dd 0d e3 47 f3 90` |
| 16384 | 1 | pageSize `00400000`, txid 1 | `0xafee0eec18cd3d9f` | `9f 3d cd 18 ec 0e ee af` |

(Computed from the algorithm; the algorithm was cross-checked against the meta pages
of `wallet/udb/testdata/v24.db.gz`, whose meta 0 — `root=8, freelist=9, pgid=10,
txid=2`, pageSize 4096 — carries checksum `0x9b9bed68c635cd62`.)

**Validation** (`db.go:1350-1359`), in this order: `magic` mismatch → `ErrInvalid`;
`version != 2` → `ErrVersionMismatch`; checksum mismatch → `ErrChecksum`. The page
header of a meta page is not validated on open.

**What is not validated.** `pageSize`, `flags`, `root`, `freelist`, `pgid` and the
16-byte page header in front of the meta are accepted as found. In particular bbolt
does not check that `pageSize` is a power of two, non-zero, or equal in both meta
pages, nor that it matches the offset at which meta 1 was found. The page size used
for the whole session is the one returned by the probe of section 5.3 (meta 0's if
meta 0 validates, whichever meta is newer). A writer must store the same `pageSize`
in both metas; a Rust reader must reject a `pageSize` that is zero, not a power of
two, smaller than 1024 or inconsistent with the file (kind `IO`) instead of
proceeding as bbolt would.

**Which meta is current** (`db.go:1088-1109`): the one with the larger `txid` if it
validates (on equal `txid` meta 0 is tried first), otherwise the other one if it
validates; if neither validates bbolt panics
(open already refused such a file). **Which meta a commit writes**
(`db.go:1375-1376`): page `txid mod 2`. A transaction's `txid` is the current meta's
`txid + 1` (`tx.go:60-63`), so commits alternate between the two meta pages and the
other page always holds the previous committed state.

### 4.4 Freelist page

Page header with `flags = 0x10`, then an array of `u64` page ids
(`bbolt@v1.3.12/freelist.go:273-304`, `:319-344`).

| Case | `count` field | Body at page offset 16 |
| --- | --- | --- |
| `n == 0` ids | 0 | nothing |
| `0 < n < 0xFFFF` | `n` | `n` page ids, ascending |
| `n >= 0xFFFF` | `0xFFFF` | first `u64` = `n`, then `n` page ids, ascending |

The page occupies `overflow + 1` pages; bbolt allocates
`(16 + 8*n') / pageSize + 1` pages for it (integer division), where `n' = n`, or
`n + 1` when `n >= 0xFFFF`, and `n` is counted *before* the freelist page itself is
allocated (`freelist.go:68-75`, `tx.go:240-255`). Trailing space is zero.

Content: the union of (a) pages free for allocation and (b) pages freed by
transactions but still "pending" because a reader might need them, including the
pages freed by the committing transaction itself and the *previous* freelist page
(`freelist.go:98-105`, `tx.go:173-176`). The new freelist page is never listed in
itself. After a crash or reopen every listed page is simply free.

On load bbolt sorts the ids (`freelist.go:296-302`), so a reader must not rely on
order, but a writer must emit ascending order.

bbolt does not validate the listed ids when loading (no range check against
`meta.pgid`, no duplicate check, no check against reachability). The defects surface
later: an id `<= 1` panics at the first allocation that scans it
(`invalid page allocation: N`, `freelist.go:116-118`); a duplicate or reachable id is
handed out and overwritten. With the `0xFFFF` escape, a leading count that does not
fit a signed 64-bit integer panics (`leading element count N overflows int`,
`freelist.go:284-286`), and bbolt trusts the count without checking it against the
page's `overflow`. The Rust reader must bound the count by
`(overflow + 1) * pageSize` and reject out-of-range ids.

The freelist page format is the same for both in-memory freelist types (`array`,
`hashmap`); nothing in the file records which one wrote it.

`meta.freelist == 0xFFFFFFFFFFFFFFFF` means no freelist page exists (written by bbolt
only under `NoFreelistSync`, which dcrwallet never sets). bbolt v1.3.12 then rebuilds
the freelist on open by walking every bucket and treating every page in `[2, pgid)`
that is not reachable as free (`db.go:1187-1219`), and — when opened writable with
freelist sync on, as dcrwallet does — immediately commits an empty write transaction
to persist it (`db.go:302-313`).

### 4.5 Branch page

Page header with `flags = 0x01`, `count = n >= 1`, followed by `n` element headers of
16 bytes each (`bbolt@v1.3.12/page.go:111-115`), followed by the key bytes.

| Element offset | Size | Field | Meaning |
| --- | --- | --- | --- |
| 0 | 4 | `pos` | Byte offset from the start of *this element header* to its key |
| 4 | 4 | `ksize` | Key length |
| 8 | 8 | `pgid` | Page id of the child page |

Element `i` is at page offset `16 + 16*i`. Its key is at
`16 + 16*i + pos` for `ksize` bytes (`page.go:118-120`). As written by bbolt
(`node.go:193-246`), keys are packed contiguously after the element array in element
order, so `pos_i = 16*(n - i) + sum(ksize_j for j < i)`.

Semantics: elements are sorted by key, strictly ascending. Element `i`'s key is the
smallest key in child `i`'s subtree at the time the child was written. Lookup of key
`K` descends into the last element whose key is `<= K`, or element 0 if `K` is smaller
than every key (`cursor.go:321-342`).

### 4.6 Leaf page

Page header with `flags = 0x02`, `count = n >= 0`, followed by `n` element headers of
16 bytes each (`bbolt@v1.3.12/page.go:123-128`), followed by key and value bytes.

| Element offset | Size | Field | Meaning |
| --- | --- | --- | --- |
| 0 | 4 | `flags` | `0x00` plain key/value; `0x01` (`bucketLeafFlag`, `page.go:24-26`) the value is a nested bucket (section 4.8). Only bit 0 is ever tested. |
| 4 | 4 | `pos` | Byte offset from the start of *this element header* to its key |
| 8 | 4 | `ksize` | Key length: 1..32768 for plain keys (`Put` enforces `MaxKeySize`); a nested-bucket name has no upper bound (`CreateBucket` checks only for empty, Q20), so a reader must accept any non-zero `u32` |
| 12 | 4 | `vsize` | Value length (0..2^31-2 accepted by `Put`). bbolt slices keys and values through a `[maxAllocSize]byte` array pointer (`unsafe.go:15-27`, `page.go:131-142`, `node.go:221-222`), so on 64-bit hosts `pos + ksize + vsize` must be `<= 0x7FFFFFFF` to be readable and `ksize + vsize <= 0x7FFFFFFF` to be written; beyond that Go faults with a slice-bounds runtime panic (`0xFFFFFFF` on 32-bit hosts) |

Element `i` is at page offset `16 + 16*i`; key at `16 + 16*i + pos`; value
immediately after the key (`page.go:131-142`). As written by bbolt, data is packed
contiguously in element order: `pos_i = 16*(n - i) + sum(ksize_j + vsize_j for j < i)`.
(Verified against every leaf of `wallet/udb/testdata/v1.db.gz`.)

Elements are sorted by key, strictly ascending; keys are unique within a bucket.
A leaf with `count = 0` is valid only as the root page of a bucket (an empty bucket)
— bbolt's cursor nevertheless skips empty leaves wherever it meets them
(`cursor.go:47-51`, `:236-240`).

### 4.7 Overflow pages

A node whose serialized size exceeds one page is written to `ceil(size / pageSize)`
contiguous pages (`node.go:367-368`). The first page carries the header with
`overflow = pages - 1`; the element array and data simply continue across the page
boundaries with no further headers. `pos` offsets are relative to the element header
and may exceed `pageSize`. Freeing such a page frees `id .. id + overflow`
(`freelist.go:172-181`).

A leaf holds one element larger than a page whenever a single key/value pair is
larger than a page; bbolt never splits a pair across separate logical pages.

### 4.8 Bucket values and inline buckets

A nested bucket is a leaf element with `flags & 0x01 != 0`. Its key is the bucket
name; its value starts with the 16-byte bucket header (`bucket.go:49-52`):

| Value offset | Size | Field | Meaning |
| --- | --- | --- | --- |
| 0 | 8 | `root` | Page id of the bucket's root page; **0 means the bucket is inline** |
| 8 | 8 | `sequence` | Per-bucket counter (`NextSequence`); dcrwallet never uses it, always 0; must be preserved on rewrite |

- **Regular bucket** (`root != 0`): the value is exactly 16 bytes. The bucket's tree is
  rooted at page `root` (leaf or branch).
- **Inline bucket** (`root == 0`): the value is `16 + pageBytes` long; at value offset
  16 is an embedded leaf page in the format of sections 4.2 and 4.6: header
  `id = 0`, `flags = 0x02`, `count`, `overflow = 0`, then elements and data, with `pos`
  relative to each element header as usual (`bucket.go:145-148`, `:638-652`). An empty
  inline bucket is the 32 bytes
  `00×16  00×8  02 00  00 00  00 00 00 00`.
- An inline bucket never contains nested buckets (`bucket.go:192-195`, `:616-630`).
  bbolt's consistency walk and its freelist reconstruction both skip inline buckets
  entirely (`tx_check.go:77-80`), so a nested regular bucket inside an inline bucket
  would have its pages treated as free. A writer must never produce one.

**Inlining rule** applied by bbolt at commit to every bucket it has materialised
(`bucket.go:608-635`): the bucket is written inline iff its tree is a single leaf
node, no element is a bucket, and
`16 + sum(16 + len(key) + len(value) over elements) <= pageSize / 4` (integer
division: 1024 for 4096-byte pages, 4096 for 16384-byte pages). Otherwise it is
written as a regular bucket. The same bucket can therefore flip between the two forms
from one commit to the next as it grows and shrinks.

### 4.9 Root bucket and the top-level namespace

`meta.root` is the header of the root bucket. The root bucket is never inline
(`db.go:613`: root page 3 at creation; `tx.go:164-171`: the root is spilled without
the inlining test). Its leaf elements are the top-level buckets
(`CreateTopLevelBucket`); walletdb offers no way to store a plain key/value pair in
the root bucket, so every root-level element has `flags = 0x01`. The names and
contents of those buckets belong to the `wallet/udb` specifications. Observed in
`wallet/udb/testdata/v24.db.gz` (database version 24): `agendaprefs`, `meta`,
`ticketsagendaprefs`, `treasurypolicy`, `tspendpolicy`, `vsp`, `vsphost`,
`vsppubkey`, `vsptreasurypolicy`, `vsptspendpolicy`, `waddrmgr`, `wstakemgr`,
`wtxmgr`; most small ones are inline.

### 4.10 Image of a newly created file

`bbolt@v1.3.12/db.go:599-641`. Four pages are written with one `pwrite` at offset 0
and then `fdatasync`ed; the file is exactly `4 * pageSize` bytes:

| Page | Header | Body |
| --- | --- | --- |
| 0 | `id=0, flags=0x04, count=0, overflow=0` | meta: magic, version 2, pageSize, flags 0, `root.root=3`, `root.sequence=0`, `freelist=2`, `pgid=4`, `txid=0`, checksum |
| 1 | `id=1, flags=0x04` | identical meta except `txid=1` (and its checksum) |
| 2 | `id=2, flags=0x10, count=0` | empty freelist |
| 3 | `id=3, flags=0x02, count=0` | empty leaf (root bucket) |

The first write transaction therefore has `txid = 2` and writes meta page 0.

### 4.11 Key ordering

Keys compare as unsigned byte strings, lexicographically, shorter-is-smaller on a
common prefix (Go `bytes.Compare`; `node.go:126`, `cursor.go:351-362`). There is one
key space per bucket shared by plain keys and nested-bucket names.

### 4.12 `Copy` stream

`DB.Copy(w)` (`db.go` adapter `:292-296`; `bbolt@v1.3.12/tx.go:346-420`) writes, inside
a read transaction:

1. One page: header `id=0, flags=0x04`, meta = the transaction's meta snapshot with a
   recomputed checksum.
2. One page: header `id=1, flags=0x04`, the same meta with `txid` decremented by one
   and the checksum recomputed.
3. The raw file bytes from offset `2 * pageSize` to `meta.pgid * pageSize`.

Total `meta.pgid * pageSize` bytes: a valid database file truncated to the high-water
mark. Errors writing the meta pages are `meta 0 copy: <err>` / `meta 1 copy: <err>`.

The data of step 3 is read from the database **file descriptor**
(`io.NewSectionReader(file, 2*pageSize, …)` fed to `io.CopyN`), not from the memory
map; the two meta pages are each written with a single `w.Write` of `pageSize`
bytes. The read transaction is held for the whole copy, so a commit that needs a
remap and `Close` wait for it (section 7.2); other commits proceed and cannot
disturb the copy, because no page visible to the snapshot is rewritten. `Copy` on a
closed handle returns `database not open` (kind `Invalid`) from the `Begin` inside
bbolt's `View` (`bbolt@v1.3.12/db.go:900-904`). `Tx.Copy` carries a `Deprecated`
marker upstream in favour of `WriteTo` (`tx.go:335-342`); dcrwallet still calls it.
No dcrwallet code calls `DB.Copy`; the test-data generators copy the closed file
with `io.Copy` instead (`wallet/udb/testdata/v24.db.go:76-89`).

### 4.13 File-validity invariants

A file is valid for bbolt v1.3.12 (and must be produced as such by the Rust writer)
when all of the following hold. Violations marked *panic* crash a Go dcrwallet;
violations marked *corrupt* silently lose or damage data.

1. File length `>= 2 * pageSize` (`db.go:439-441`, error `file size too small`) and
   `>= meta.pgid * pageSize` (pages are read through a memory map; reading past the end
   of file faults).
2. At least one meta page validates; the newer valid one describes a consistent tree.
3. `meta.root.root < meta.pgid`; `meta.freelist < meta.pgid` or equals
   `pgidNoFreelist` (`db.go:1367-1373`: bbolt enforces this by *panic* on every meta
   it writes; it does not check a meta it reads, where a violation instead leads to
   reads beyond the high-water mark).
4. Every branch/leaf page reachable from the root has `id` equal to its position and
   `flags` exactly `0x01` or `0x02` (*panic*).
5. The freelist page has `flags & 0x10` (*panic*: `invalid freelist page`), and lists
   only pages in `[2, pgid)` that are not reachable from the current meta (*corrupt* if
   it lists a reachable page: it will be overwritten; *panic* `page N already freed`
   if a listed page is later freed again). Unreachable pages missing from the list are
   leaked, not harmful.
6. No page is referenced twice; `overflow` is exact (it drives freeing).
7. Keys within a page are strictly ascending; a branch element's key is `<=` every key
   in its subtree and `>` every key in the preceding subtree (*corrupt*: lookups miss).
   For a file that Go will write again the branch key must be **equal** to the first
   key of the child page it points to: bbolt finds a rewritten (or removed) child's
   entry in the parent by the child's first key as read from the child page
   (`node.go:81-84`, `:181-184`, `:382-389`, `:452`, `:484`, `:500`); when no parent
   entry has exactly that key it inserts a second entry and leaves the old one
   pointing at the freed page (*corrupt*).
8. No zero-length key anywhere (*panic* `read: zero-length inode key` when bbolt
   materialises the page, `node.go:178`).
9. Branch pages have `count >= 1`. Non-root pages should not be empty.
10. Bucket values are at least 16 bytes; inline bucket values at least 32 bytes with an
    embedded leaf header; inline buckets contain no buckets.
11. Element counts fit `u16`: a node with `>= 0xFFFF` elements cannot be written
    (*panic* `inode overflow`, `node.go:203-205`).

---

## 5. Algorithms and state machines

Citations in this section without a module prefix are to `bbolt@v1.3.12/`.

### 5.1 Opening the database file (`bolt.Open`, `db.go:197-317`)

Both the driver's `Open` and `Create` end here; the only difference is the existence
pre-check in `openDB` (section 3.2).

1. Open the file with `os.OpenFile(path, O_RDWR|O_CREATE, 0600)` (the Go standard
   library adds `O_CLOEXEC`). The file is created if absent, **also on the "open existing" path**
   if it disappears between the pre-check and this call. Mode 0600 is subject to the
   umask and applies only at creation; an existing file's mode is untouched. Error:
   the OS error (`*PathError`) unchanged.
2. Take the advisory lock (section 5.2) — exclusive, no timeout. On error close the
   file and return the error.
3. `pageSize` := OS page size (provisional).
4. `fstat`. If the size is 0, initialise the file (section 4.10: one `pwrite` of four
   pages, then `fdatasync`). Otherwise determine the page size from the file
   (section 5.3); failure → `ErrInvalid` (`invalid database`).
5. Memory-map the file (section 5.4) and validate both meta pages; only if **both**
   fail is the error of meta 0 returned (`ErrInvalid`, `ErrVersionMismatch` or
   `ErrChecksum`, `db.go:499-506`). A file shorter than `2 * pageSize` fails with the
   plain error `file size too small`.
6. Load the freelist (section 5.5).
7. If the current meta has `freelist == pgidNoFreelist`: begin a write transaction and
   commit it immediately so that a freelist page is written (`db.go:302-313`).
8. Return the handle.

Other error texts of the open path, all returned unchanged by `bolt.Open` and
therefore kind `Other` after `convertErr`: `mmap stat error: <err>` (`db.go:438`);
the raw `mmap(2)` error, or `madvise: <err>` when `madvise` fails with anything but
`ENOSYS` (`bolt_unix.go:55-65`); on Windows `truncate: <err>`,
`CreateFileMapping: <err>`, `MapViewOfFile: <err>`, `CloseHandle: <err>`
(`bolt_windows.go:69-93`); `unmap error: <err>` (on Windows
`unmap error: UnmapViewOfFile: <err>`, `bolt_windows.go:111-113`) and, when a failed map cannot be
undone, `<err>; rollback unmap also failed: <err>` (`db.go:480-486`, `:526-528`);
the raw `fstat`, `pwrite` or sync error from file initialisation.

`Close` (`db.go:659-700`): a second call returns nil. Otherwise it unmaps, unlocks
(writable handles only) and closes the descriptor, attempting all three even when an
earlier one fails, and returns only the **first** failure: `unmap error: <err>`,
`bolt.Close(): funlock error: <err>` or `db file close: <err>` (kind `Other`). The
handle counts as closed afterwards whatever the result, so
`Loader.UnloadWallet`'s "state left loaded" after a close error (section 5.15)
leaves a loader holding a dead handle.

On any failure after step 1 the handle is closed (unmap, unlock, close) before
returning. An existing zero-length file is silently initialised as a new database
(step 4). Opening and closing without a write transaction leaves the file bytes
unchanged, except in the case of step 7 and, on Windows, the extension of the file
to the map size at step 5 (section 5.4).

### 5.2 File locking (`bolt_unix.go:16-50`, `bolt_windows.go:19-60`, `bolt_unix_solaris.go`, `bolt_unix_aix.go`)

| Platform | Lock call | Unlock |
| --- | --- | --- |
| Linux, macOS, BSDs (everything except Windows, Plan 9, Solaris, AIX) | `flock(fd, LOCK_EX \| LOCK_NB)` on the database file itself (`LOCK_SH \| LOCK_NB` for a read-only open) | `flock(fd, LOCK_UN)` |
| Windows | `LockFileEx(handle, LOCKFILE_FAIL_IMMEDIATELY \| LOCKFILE_EXCLUSIVE_LOCK, 0, 1, 0, &Overlapped{Offset: 0xFFFFFFFF, OffsetHigh: 0xFFFFFFFF})` — one byte at offset `2^64 - 1` (shared: without the exclusive flag) | `UnlockFileEx` with the same range |
| Solaris, AIX | `fcntl(F_SETLK)` whole-file `F_WRLCK` (`F_RDLCK` shared) | `fcntl(F_SETLK, F_UNLCK)` |

Retry loop: attempt the non-blocking lock; on success return; on any error other than
"would block" (`EWOULDBLOCK` / `ERROR_LOCK_VIOLATION` / `EAGAIN`) return it; if a
timeout `T != 0` was given and `elapsed > T - 50ms` return `ErrTimeout` (`timeout`);
sleep 50 ms (`flockRetryTimeout`, `db.go:44`) and retry. dcrwallet passes `T = 0`:
**the open blocks indefinitely, polling every 50 ms, with no log output**, while
another process (or another handle in the same process — `flock` locks belong to the
open file description) holds the lock.

There is no separate lock file. The lock is released by `Close` (explicit unlock for
writable opens, then file close) or by process exit.

### 5.3 Page-size determination for an existing file (`db.go:322-407`)

1. Read `0x1000` bytes at offset 0. If the full 4096 bytes were read, mark "meta 0
   readable"; if the meta at buffer offset 16 validates, return its `pageSize`.
2. For `i = 0..14`: `pos = 1024 << i`; stop if `pos >= fileSize - 1024`; read up to
   4096 bytes at `pos` (a short read is accepted when it ends exactly at end of file);
   mark "meta 1 readable"; if the meta at buffer offset 16 validates, return its
   `pageSize`. (This finds meta page 1 for any power-of-two page size from 1 KiB to
   16 MiB when meta 0 is damaged.)
3. If either read succeeded, return the provisional OS page size.
4. Otherwise `ErrInvalid`.

Consequences: a non-empty file of at most 2048 bytes fails with `invalid database`
(the read of step 1 is short and step 2 stops at `i = 0` because
`1024 >= fileSize - 1024`). A file of 2049..4095 bytes passes step 2 (the short read
at offset 1024 ends at end of file) and, like any longer file with two bad metas,
reaches the map step with the provisional OS page size; it fails there with
`file size too small` when shorter than two such pages (2049..8191 bytes with
4096-byte pages), otherwise with meta 0's validation error.

### 5.4 Memory map sizing and remapping (`db.go:432-568`)

bbolt maps the file read-only and shared (`PROT_READ`, `MAP_SHARED`,
`madvise(MADV_RANDOM)`), reads pages through the map, and writes with `pwrite`.

Map size for a requested minimum `size = max(fileSize, minsz)` (`mmapSize`):

1. For `i = 15..30`: if `size <= 2^i` return `2^i` (32 KiB minimum, doubling to 1 GiB).
2. If `size > maxMapSize` → error `mmap too large` (`maxMapSize` = `0xFFFFFFFFFFFF` on
   amd64/arm64/riscv64/ppc64/s390x/loong64, `0x7FFFFFFF` on 386/arm/ppc,
   `0x8000000000` on mips64, `0x40000000` on mips).
3. Round `size` up to a multiple of 1 GiB (`maxMmapStep`), then up to a multiple of
   `pageSize`, capped at `maxMapSize`.

The map may extend past the end of the file. A remap happens during commit when a
page must be allocated beyond the current map (section 5.9.4); it first takes the map
lock exclusively, and therefore waits for every open read transaction to finish, then
copies the writer's in-memory nodes off the old map (`db.go:461-464`), unmaps and
maps again (`db.go:432-476`).

**Windows only** (`bolt_windows.go:64-74`): for a writable open, mapping first
truncates (extends) the file to the map size, so the file length always equals the
map size (32 KiB, 64 KiB, … 1 GiB, then 1 GiB steps).

A Rust implementation is free not to memory-map at all; none of this is visible in
the file except the Windows file length and the growth rule of section 5.9.6, which
uses the map size.

### 5.5 Freelist load (`db.go:412-428`, `freelist.go:273-310`)

Once per open: if `meta.freelist == pgidNoFreelist`, rebuild by scanning (every page
in `[2, pgid)` not reachable from the root bucket through regular buckets and their
branch/leaf pages, including overflow pages, is free). Otherwise read the freelist
page (section 4.4), panic if the flag is missing, copy and sort the ids. All listed
ids become immediately allocatable; nothing is pending after an open.

The rebuild scan (`db.go:1187-1219`, `tx_check.go:75-114`) runs inside a read
transaction and reuses the consistency checker's bucket walk with an empty "freed"
set. Any finding of that walk — a page id above the high-water mark, a page
referenced twice, a reachable page that is neither branch nor leaf, keys out of
order within a page or against the parent's separator keys (section 5.18) — makes
the helper goroutine panic with
`freepages: failed to get all reachable pages (<finding>)`; because the panic is
raised on another goroutine it cannot be recovered by the caller and terminates the
process. So a file with
`freelist = pgidNoFreelist` and any such defect crashes a Go dcrwallet at open,
whereas the same defect in a file with a synced freelist goes unnoticed. Pages 0
and 1 and the (absent) freelist page are not part of the result; the result is every
id in `[2, pgid)` not marked reachable, ascending.

### 5.6 Beginning transactions (`db.go:719-830`, `tx.go:46-64`)

**Read transaction** (`beginTx`):

1. Lock `metalock`; take `mmaplock` shared (held until the transaction ends).
2. If the handle is closed: release both, return `ErrDatabaseNotOpen`. If unmapped:
   `ErrInvalidMapping`.
3. Snapshot: copy the current meta (section 4.3) into the transaction; the root
   bucket header is `meta.root`. Register the transaction in the handle's list of open
   readers. Unlock `metalock`.

The transaction sees exactly the committed state of `meta.txid` for its whole life
(snapshot isolation), regardless of later commits.

**Write transaction** (`beginRWTx`):

1. If the handle is read-only: `ErrDatabaseReadOnly`.
2. Lock `rwlock` (held until commit/rollback): **one writer at a time; a second
   `BeginReadWriteTx` blocks**.
3. Lock `metalock`; closed → unlock `rwlock`, `ErrDatabaseNotOpen`; unmapped →
   `ErrInvalidMapping`.
4. Snapshot the current meta and set `txid = meta.txid + 1`.
5. Release pending free pages that no open reader can still see (section 5.11).
6. Unlock `metalock`.

### 5.7 Reads

All reads resolve a page id through `pageNode` (`bucket.go:727-749`): for an inline
bucket the materialised root node if any, else the embedded page; otherwise the
bucket's materialised node for that page id if one exists (write transactions only),
else the page itself (a not-yet-written dirty page if the transaction has one, else
the committed page).

**Bucket lookup** (`Bucket.Bucket`, `bucket.go:96-151`): in a write transaction a
per-parent cache keyed by name returns the same child object on repeated lookups.
Otherwise seek the name; return nil unless the found key equals the name exactly and
the element has the bucket flag; build the child from the 16-byte header (copied in a
write transaction), and if `root == 0` point it at the embedded page.

**`Get`** (`bucket.go:262-275`): seek; return nil if the element is a bucket; return
nil if the found key differs; else the value. A stored zero-length value read from a
page is a non-nil empty slice; see section 10 Q6 for values written earlier in the same
transaction.

**Seek** (`cursor.go:156-163`, `:280-364`): starting at the bucket root, at each
branch descend into the last element whose key is `<= target`, or element 0 when
every key is greater than the target. At the leaf select the first element whose key
is `>= target`; the index may equal `count` (one past the end).

**Cursor.** The cursor is a stack of `(page-or-node, index)` references from the
bucket root to a leaf (`cursor.go:19-22`, `:409-413`).

| Method | Behaviour |
| --- | --- |
| `First` (`:32-58`) | Descend along index 0 to a leaf. If that leaf is empty, advance with the `Next` procedure. Return the element, or `(nil, nil)` if the bucket is empty. |
| `Last` (`:63-87`) | Descend along the last index. While the leaf is empty and the stack is deeper than one, step back with the `Prev` procedure. Return the element or `(nil, nil)`. |
| `Next` (`:92-99`, `:212-244`) | From the top of the stack downward, find the deepest level whose index can advance, advance it, drop the levels below, descend along index 0; if the leaf reached is empty repeat. If no level can advance return `(nil, nil)` **and leave the cursor where it was** (on the last element). |
| `Prev` (`:104-111`, `:248-277`) | From the top of the stack: if the index is `> 0` decrement it and stop; otherwise, if this is the only level, reposition with `First` and return `(nil, nil)` (the cursor is left **on the first element**); otherwise pop the level and continue. Then descend along the last index and return the element. |
| `Seek(k)` (`:117-133`) | Seek as above; if the leaf index is past the end, advance with `Next`. Returns the first element with key `>= k`, or `(nil, nil)` when none; in the latter case the leaf index stays one past the end, so a following `Prev` returns the last element of the bucket. `wallet/udb/txdb.go:400-413` depends on this. |
| `Delete` (`:137-152`) | `ErrTxClosed` / `ErrTxNotWritable`; `ErrIncompatibleValue` if the current element is a bucket; otherwise remove the current key from the leaf's materialised node (section 5.8). Does not move the cursor. |

Every method returning a pair returns `value == nil` for a nested bucket. `First`,
`Last`, `Next`, `Prev` and `Seek` panic (`assertion failed: tx closed`) on a finished
transaction.

Cursor edge cases (all follow from the code cited in the table):

| Situation | Result |
| --- | --- |
| `Next` or `Prev` on a cursor that was never positioned (empty stack) | `(nil, nil)`; the cursor stays unpositioned. |
| `Delete` on a never-positioned cursor (writable transaction) | Go runtime panic, index out of range (`cursor.go:368`). |
| `Delete` when the cursor is past the end of its leaf (after a `Seek` that returned nil), or on an empty bucket after `First`/`Last` | The current key is nil; the leaf node is materialised, nothing is removed, the node is **not** marked unbalanced, nil is returned (`cursor.go:144-149`, `node.go:144-151`). The bucket is still rewritten at commit because a node was materialised. |
| `Seek(nil)` / `Seek([]byte{})` | Positions on the first element (every key is `>=` the empty key). |
| `Last` on an empty bucket | `(nil, nil)`; the root reference has index `-1`. A following `Next` returns `(nil, nil)` (`cursor.go:63-87`, `:216-229`). |
| `First` on an empty bucket, then `Prev` | `(nil, nil)` both times. |
| Internal seek used by `Get`, `Put`, `Delete`, bucket lookup, `CreateBucket`, `DeleteBucket` (`cursor.go:156-163`) | Unlike the public `Seek` it does **not** step to the next leaf when the leaf index is past the end, and does not assert that the transaction is open. It reports "no key here", which every caller treats as "absent". |

Use after the transaction has finished: the public cursor moves panic as stated
above; `Cursor.Delete`, `Put`, `Delete`, `CreateBucket`, `DeleteBucket` and `ForEach`
return `ErrTxClosed`; `Get`, bucket lookups (`ReadBucket`, `NestedRead[Write]Bucket`)
and `KeyN` have no such guard and dereference the cleared transaction — for `Get`
and bucket lookups a nil pointer panic for a regular bucket and a read of possibly
unmapped memory for an inline one; for `KeyN` a nil pointer panic in both cases,
because `Stats` reads `tx.db.pageSize` first (`tx.go:328-332`, `bucket.go:96-119`,
`:262-275`, `:420-423`).

`ForEach` (`bucket.go:391-402`): `ErrTxClosed` on a finished transaction; otherwise a
fresh cursor from `First` through `Next`, calling the function for each element (value
nil for buckets) and returning the first error the function returns.

### 5.8 Mutations inside a write transaction

Mutations never touch pages. They operate on **nodes**: in-memory copies of pages
(`node.go:11-21`) holding a sorted list of `{flags, pgid, key, value}` entries. A node
is materialised from its page the first time a mutation needs it, together with every
ancestor up to the bucket root (`cursor.go:387-406`, `bucket.go:665-695`), and is
cached per bucket by page id. Nothing is split, merged or allocated until commit.
A cursor positioned before a leaf was materialised keeps iterating the *page*
snapshot of that leaf; a cursor positioned afterwards iterates the *live node*
(section 10 Q9).

| Operation | Steps and errors (in the order checked) |
| --- | --- |
| `Put(key, value)` (`bucket.go:281-313`) | finished tx → `ErrTxClosed`; read-only tx → `ErrTxNotWritable`; `len(key) == 0` → `ErrKeyRequired`; `len(key) > 32768` → `ErrKeyTooLarge`; `len(value) > 2^31 - 2` → `ErrValueTooLarge`; copy the key; seek; existing key that is a bucket → `ErrIncompatibleValue`; insert or overwrite in the leaf node with flags 0. The key is copied, **the value slice is retained by reference** until commit. |
| `Delete(key)` (`bucket.go:318-343`) | `ErrTxClosed`; `ErrTxNotWritable`; seek; key absent → nil (no node is materialised, the bucket is not rewritten; exception: an empty `key` on an **empty** bucket equals the nil "no key here" result of the seek, so the root node is materialised, nothing is removed, and the bucket is rewritten at commit); key is a bucket → `ErrIncompatibleValue`; remove from the leaf node and mark the node unbalanced. |
| `CreateBucket(name)` (`bucket.go:156-198`) | `ErrTxClosed`; `ErrTxNotWritable`; empty name → `ErrBucketNameRequired`; seek; existing bucket → `ErrBucketExists`; existing plain key → `ErrIncompatibleValue`; insert an element with the bucket flag whose value is the 32-byte empty inline bucket; stop treating the parent as inline for the rest of the transaction; return the child via bucket lookup. There is **no length check on the name**. |
| `CreateBucketIfNotExists(name)` (`bucket.go:203-211`) | `CreateBucket`; on `ErrBucketExists` return the existing bucket; other errors returned. |
| `DeleteBucket(name)` (`bucket.go:215-257`) | `ErrTxClosed`; `ErrTxNotWritable`; seek; found key differs → `ErrBucketNotFound`; not a bucket → `ErrIncompatibleValue`; recursively delete every nested bucket of the child (an inner failure is returned as the plain error `delete bucket: <err>`); drop the cached child; add every page of the child's tree (with overflow) to this transaction's pending-free set; remove the element from the parent leaf node. |
| cursor `Delete` | see section 5.7. |

Bucket handles inside a write transaction (`bucket.go:96-119`, `:245-251`):

- Looking up the same child twice returns the same object (per-parent cache keyed
  by name), so state such as materialised nodes is shared by all handles to it. In a
  read transaction every lookup builds a fresh, independent handle.
- `DeleteBucket` removes the child from the cache and clears the child's node cache.
  A handle to the deleted bucket (or to one of its descendants, which are deleted
  the same way) obtained earlier is dead. For a formerly regular bucket every
  read and mutation faults with a nil dereference (its `root` was reset to 0 and it
  has no inline page). For a formerly inline bucket reads keep returning the old
  contents and `Put`/`Delete` of an existing key/`CreateBucket` panic with
  `assertion failed: nodes map expected` (`bucket.go:666`). Creating a bucket of the
  same name afterwards yields a new, unrelated handle.
- A bucket created and deleted within the same transaction frees no pages (it never
  had any).
- Keys handed to `Put`/`CreateBucket` are copied immediately
  (`bucket.go:168`, `:297`), so later mutation of the caller's key buffer is
  harmless; values are not (Q8). A remap copies every materialised key and value off
  the map and off the caller's buffer (`node.go:523-551`), but a remap can happen
  only inside `Commit` (`db.go:1128-1135` is the only remap site after open), so a
  change the caller makes to a value buffer between `Put` and `Commit` always
  reaches the file.
- Node-level invariants enforced by panic during mutation: `put: zero-length old
  key`, `put: zero-length new key`, `pgId (N) above high water mark (M)`
  (`node.go:117-123`).

`DeleteBucket` enumerates the child's nested buckets with a cursor over the child
(`ForEachBucket`, `bucket.go:404-417`) while each recursive `DeleteBucket` removes
the visited entry from the child's leaf node. Whether that removal is visible to
the enumerating cursor follows the rule of Q9: if the leaf holding the nested-bucket
entries was still a page when the cursor reached it, the cursor walks the page
snapshot and visits every nested bucket; if that leaf had already been materialised
as a node earlier in the same transaction (a `Put`, `Delete`, `CreateBucket` or
`DeleteBucket` directly in the child that touched that leaf), the removal shifts the
following entry into the cursor's index and the next step skips it, so every second
nested bucket in that leaf is not recursed into. A skipped nested bucket's pages are
then never added to the pending-free set: they become unreachable but unlisted
(leaked, section 4.13 item 5), and the file stays valid. (Derived from
`bucket.go:234-254`, `cursor.go:212-244`, `node.go:144-158`; no upstream test
exercises it — UNVERIFIED by execution, section 13 item 13.)

`DeleteBucket` with an empty name follows the same steps: on an **empty** parent the
seek finds nothing, the nil found key equals the empty name, the flag test fails, and
the result is `ErrIncompatibleValue`; on a non-empty parent it is `ErrBucketNotFound`
(section 10 Q5).

### 5.9 Commit (`tx.go:143-255`)

`Commit` on a finished transaction → `ErrTxClosed`; on a read transaction →
`ErrTxNotWritable`. Otherwise, in order:

#### 5.9.1 Rebalance (`bucket.go:655-662`, `node.go:409-508`)

For every bucket opened in the transaction, depth-first from the root bucket, and for
every materialised node of each (in Go map iteration order, i.e. unspecified), run
`rebalance` on nodes marked unbalanced (nodes that lost an element):

1. Clear the mark. If `size(node) > pageSize / 4` **and** the node has more than
   `minKeys` elements (1 for a leaf, 2 for a branch), stop.
   `size(node) = 16 + sum(16 + len(key) + len(value))` (branch entries have no value).
2. Root node: if it is a branch with exactly one child, replace it by that child
   (take the child's entries and leaf flag, re-parent the grandchildren, free the
   child's page). Stop.
3. Node with no elements: remove its entry from the parent, drop it, free its page,
   rebalance the parent. Stop.
4. Otherwise merge with a sibling **unconditionally**: if the node is its parent's
   first child, append the *next* sibling's elements to this node and remove the
   sibling (entry removed from the parent, page freed); otherwise append this node's
   elements to the *previous* sibling and remove this node. Then rebalance the parent.

The sibling is materialised if necessary. A merged node that is now too large is
split again in the next step.

#### 5.9.2 Spill buckets (`bucket.go:548-604`)

`spill(bucket)`:

1. For each child bucket opened in this transaction (Go map iteration order):
   - if the child is inlineable (section 4.8): free all of its pages (if it was
     regular), and its new value is the 16-byte header (with `root = 0`, sequence
     preserved) followed by its root node serialised as an embedded leaf page;
   - else `spill(child)` recursively; its new value is the 16-byte header with the
     child's (possibly new) root page id;
   - if the child has no materialised root node (it was only read), skip it;
   - otherwise seek the child's name in this bucket (panic if it is not found or not
     a bucket) and overwrite the element's value in the leaf node — which materialises
     this bucket's path to that leaf.
2. If this bucket has no materialised root node, stop.
3. `spill(rootNode)` (next section); the bucket's `root` becomes the page id of the
   top node afterwards.

The root bucket is spilled with this procedure without the inlining test; then
`meta.root.root` is set to its root page id.

#### 5.9.3 Spill nodes, splitting (`node.go:339-405`, `:250-335`)

`spill(node)`:

1. If already spilled, stop.
2. Sort the materialised children by their first key and spill each (the list can grow
   while iterating).
3. Split this node into a list of nodes (below). For each resulting node, in order:
   a. if it was read from a page, add that page (with its overflow) to the
      transaction's pending-free set;
   b. allocate `ceil(size(node) / pageSize)` contiguous pages (section 5.9.4);
   c. serialise the node into them (sections 4.5/4.6); mark it spilled;
   d. if it has a parent, replace the parent's entry for this node — found under the
      node's *previous* first key — with `{key: current first key, pgid: new page}`
      (inserting a new entry for nodes created by the split).
4. If the split created a new root (a parent with no page id yet), spill that parent.

Split (`split`/`splitTwo`/`splitIndex`), applied repeatedly to the remainder:

1. Do not split if the node has `<= 4` elements (`2 * minKeysPerPage`) or if
   `size(node) < pageSize` (strictly less; `sizeLessThan`, `node.go:52-62`).
2. `threshold = floor(pageSize * fillPercent)` with `fillPercent` clamped to
   `[0.1, 1.0]`; always `pageSize / 2` in dcrwallet.
3. Choose the split index: `sz = 16`; for `i = 0 .. len - 3` (i.e.
   `i < len - minKeysPerPage`): set `index = i`; let
   `el = 16 + len(key_i) + len(value_i)`; if `i >= 2` and `sz + el > threshold`, stop;
   else `sz += el`. (If the loop runs out, `index = len - 3`.)
4. The first node keeps elements `[0, index)`; a new sibling under the same parent
   (creating a parent if the node was a root) takes `[index, len)` and is itself
   subjected to step 1.

Effect with 4096-byte pages: a leaf that reaches 4096 serialised bytes is cut into
pieces of at most 2048 bytes (at least two elements each; a piece exceeds 2048 only
when its first two elements alone do), except the last piece,
which keeps whatever remains once the remainder is below 4096 bytes or has four or
fewer elements (in the latter case it can exceed a page and is written with overflow
pages). (Observed in
`wallet/udb/testdata/v1.db.gz`: runs of equal-sized leaves of 1976–2044 used bytes
followed by a larger tail leaf of 2760, 2772, 2956, 3271, 3494 or 3544 bytes.) Every materialised node — every node on the path from a
bucket root to any modified leaf — is rewritten to newly allocated pages; the old
pages become pending-free. Nothing is modified in place.

#### 5.9.4 Page allocation (`db.go:1112-1141`, `freelist.go:109-149`)

To allocate `n` contiguous pages for transaction `txid`:

1. Scan the free ids in ascending order for the first run of `n` consecutive ids
   (first fit, lowest address). If found, remove the run from the free list and
   return its first id.
2. Otherwise take `id = meta.pgid` (the transaction's high-water mark), ensure the
   map covers `(id + n + 1) * pageSize` bytes (remap if
   `(id + n + 1) * pageSize >= mapSize`; failure → `mmap allocate error: <err>`),
   and advance `meta.pgid` by `n`.

The page buffer is zero-filled, its header gets `id` and `overflow = n - 1`. Pages
freed by the committing transaction are *pending*, not free, so they cannot be reused
within the same commit. Allocation happens only inside `Commit`.

#### 5.9.5 Freelist commit (`tx.go:173-185`, `:240-255`)

1. If `meta.freelist != pgidNoFreelist`, add the current freelist page (with
   overflow) to the pending-free set of this transaction.
2. Compute `pages = (16 + 8*n') / pageSize + 1` with `n` = free + pending ids at this
   moment (section 4.4) and allocate that many pages as in 5.9.4.
3. Write the freelist page: all free ids and all pending ids of all transactions,
   merged ascending. Set `meta.freelist` to its id.

A commit therefore always writes a new freelist page and a meta page, and advances
`txid`, even when the transaction changed nothing.

#### 5.9.6 File growth (`tx.go:187-197`, `db.go:1144-1181`)

If the high-water mark moved up during this commit, call
`grow((meta.pgid + 1) * pageSize)`:

1. If the requested size `<=` the file size recorded by this handle (`filesz`), stop.
   `filesz` is **not** initialised from the real file size on open; it is `4 * pageSize`
   after creating a file and 0 after opening an existing one, and is updated only
   here.
2. If `mapSize <= 16 MiB` (`AllocSize`), the new size is `mapSize`; otherwise the new
   size is the requested size plus 16 MiB.
3. Non-Windows: `ftruncate(newSize)` (error `file resize error: <err>`). All
   platforms: full `fsync` of the file (error `file sync error: <err>`). Record
   `filesz = newSize`.

Resulting file lengths on non-Windows: `4 * pageSize` at creation; then the map size
— 32 KiB, 64 KiB, … doubling up to 16 MiB; beyond that
`(pgid + 1) * pageSize + 16 MiB` each time the high-water mark outgrows the recorded
size. A file that only ever grew by these rules is never shrunk. Because `filesz`
starts at 0 (step 1) and `ftruncate` sets the length unconditionally, a file that is
longer than the computed size while the map exceeds 16 MiB — for example a file of
64 MiB or more last written on Windows, where the length equals the map size — is
truncated **down** to
`(pgid + 1) * pageSize + 16 MiB` by the first growing commit of a non-Windows
session; no referenced page is lost, since the new length exceeds
`pgid * pageSize`. (Observed: `v24.db` is 65536 bytes with `pgid = 13`;
`v1.db` is 524288 bytes with `pgid = 109`.)

#### 5.9.7 Write data pages (`tx.go:470-539`)

Write every dirty page (all node pages and the freelist page) with one positional
write per logical page (`pwrite` of `(overflow + 1) * pageSize` bytes at
`id * pageSize`; chunked at `maxAllocSize - 1` bytes), in ascending page-id order.
Then `fdatasync` (`bolt_linux.go:8-10`: `fdatasync(2)` on Linux;
`boltsync_unix.go:7-9`, `bolt_windows.go:14-16`: Go `File.Sync` on macOS, the other
BSDs and Windows — `F_FULLFSYNC` and `FlushFileBuffers` respectively, stated from
knowledge of the Go standard library, section 13 item 10; `bolt_openbsd.go:11-16`:
`msync` of the map on OpenBSD).

#### 5.9.8 Write the meta page (`tx.go:542-566`, `db.go:1367-1383`)

Build a zeroed page: header `id = txid mod 2`, `flags = 0x04`; meta = the
transaction's meta (new root, freelist, pgid, txid) with a fresh checksum. Panic if
`root.root >= pgid` or (`freelist >= pgid` and not `pgidNoFreelist`). Write it with
one `pwrite` at `id * pageSize` under `metalock`, then `fdatasync` again.

Durability point: the commit is durable when the second sync returns. A crash before
the meta write leaves the previous meta current; a torn meta write fails its checksum
and the other meta (previous commit) is used.

#### 5.9.9 Finish

Release the writer lock, update statistics, mark the transaction finished, run
commit handlers (none in dcrwallet). Return nil.

**Error exits.** A failure in spill, freelist allocation, growth, page write or meta
write rolls the transaction back with a freelist reload from disk (section 5.10) and
returns the underlying error unchanged (kind `Other` after `convertErr`). The
transaction is finished after a failed commit: a following `Rollback` or `Commit`
returns `ErrTxClosed` (pinned by `tests/failpoint/db_failpoint_test.go`
`TestFailpoint_LackOfDiskSpace`), and the writer lock has been released. The
freelist reload is skipped when the failure left the file unmapped (`tx.go:286-297`);
the handle then answers every later `Begin` with `ErrInvalidMapping`. Data pages
already written by the failed commit stay in the file but are unreferenced: the meta
page was not written, so the previous state remains current.

A `Commit` on a **read** transaction returns `ErrTxNotWritable` and leaves the
transaction open; it must still be rolled back. (Reachable only through the type
assertion of Q24, since `walletdb.ReadTx` has no `Commit`.)

### 5.10 Rollback (`tx.go:259-333`)

`Rollback` on a finished transaction → `ErrTxClosed`. Read transaction: unregister
from the open-reader list and release the shared map lock. Write transaction:
discard the pending-free set of this `txid` (section 5.11), release the writer lock.
No file bytes change (allocation happens only in commit). The internal rollback used
after a failed commit additionally reloads the free list from the on-disk freelist
page (or rescans when there is none), minus ids still pending for other transactions.

### 5.11 Freelist bookkeeping (`freelist.go`, `db.go:806-824`)

State: `ids` (sorted free page ids), `pending[txid]` (page ids freed by write
transaction `txid`, each tagged with the txid that allocated it, 0 if unknown,
`txid - 1` for a freelist page), `allocs[pgid]` (which transaction allocated a run
**from the free list**; a run taken at the high-water mark is not recorded,
`db.go:1123-1140`, `freelist.go:142`),
and a set of all free-or-pending ids used to detect double frees.

- **free(txid, page)**: panic if `page.id <= 1` (`cannot free page 0 or 1`) or if any
  id of the run is already free or pending (`page N already freed`); append
  `id .. id + overflow` to `pending[txid]`.
- **release at write-begin** (`freePages`): let `minid` be the smallest snapshot
  `txid` among open read transactions, or `2^64 - 1` when there are none. Move every
  `pending[t]` with `t <= minid - 1` into `ids`. Then, for each gap between
  consecutive reader snapshot txids (and above the last), move pending pages that were
  both allocated and freed by transactions inside that gap into `ids`
  (`releaseRange`). With no open readers, everything pending — including pages freed
  by the immediately preceding commit — becomes reusable by the new write
  transaction.
- **rollback(txid)**: drop `pending[txid]`; pages it had both allocated and freed
  return to `ids`; restore `allocs` entries for the others.

Details of the bookkeeping that affect which page ids are reused, and when
(`freelist.go:153-264`):

- `allocs` is empty after an open and is filled only by allocations served from the
  free list, so every page freed in a session has allocating txid 0 ("unknown")
  unless it was allocated from the free list earlier in the same session (pages
  allocated at the high-water mark stay "unknown"; a freelist page with no `allocs`
  entry gets `txid - 1`). Pages with
  allocating txid 0 are released only by the first rule (`pending[t]` with
  `t <= minid - 1`), never by the gap rule.
- The gap rule skips a `pending[t]` entry whose last gap scan started at the same
  `begin` txid (`lastReleaseBegin`), an optimisation that can postpone a release by
  one write transaction but never makes one unsafe.
- `rollback(txid)` removes the transaction's pending ids from the free-or-pending
  set. Ids whose allocating txid is 0 simply become in-use again; ids allocated by
  another transaction get their `allocs` entry back; ids allocated by `txid` itself
  go to `ids`. All `allocs` entries owned by `txid` are dropped.
- The array allocator panics on a free id `<= 1` (`invalid page allocation: N`).
- The free-or-pending set is rebuilt from `ids` plus all pending lists whenever
  `ids` is replaced wholesale (open, reload).

Safety property a port must keep: a page reachable from the meta of any open read
transaction, or from the most recently committed meta, is never overwritten.

### 5.12 `KeyN` (`db.go` adapter `:190-192`; `bucket.go:420-499`)

`KeyN()` returns `Bucket.Stats().KeyN`: the number of leaf elements in the bucket
**plus, recursively, `KeyN` of every nested regular-bucket child and inline child**
(a nested bucket's own entry counts as one element of its parent). It is computed by
walking **pages** — the embedded page of an inline bucket, or the page tree from
`root` — not the materialised nodes, so inside a write transaction it does not
reflect that transaction's uncommitted puts and deletes. Callers
(`wallet/udb/txmined.go:2353`, `:3073`) apply it to a flat bucket.

### 5.13 `walletdb.View` and `walletdb.Update` (`wallet/walletdb/interface.go:203-255`)

`View(ctx, db, f)`:

1. `db.BeginReadTx()`; on error return it.
2. Run `f(tx)`.
3. Always `tx.Rollback()` afterwards, also when `f` panics (the panic propagates).
4. Return `f`'s error. A rollback error is never returned (section 10 Q1).

`Update(ctx, db, f)`:

1. `db.BeginReadWriteTx()`; on error return it.
2. Run `f(tx)`.
3. If `f` panicked or returned an error: `tx.Rollback()` (its error ignored); return
   `f`'s error (or keep panicking).
4. Otherwise `tx.Commit()`; return the commit error, if any.

Both open Go execution-trace regions (`db.View`/`db.ReadTx`, `db.Update`/
`db.ReadWriteTx`); `ctx` is used for nothing else — in particular **cancellation of
`ctx` does not interrupt or abort a transaction**. The transactions are bbolt
*unmanaged* transactions, so calling `Commit`/`Rollback` inside `f` does not panic;
the helper's own call afterwards then fails with `tx closed` (kind `Invalid`), which
`Update` returns and `View` drops.

### 5.14 Driver registry

`RegisterDriver`, `Create`, `Open`, `SupportedDrivers` are single map operations
(section 3.1). Registration happens once, from `bdb`'s `init`.

### 5.15 Loader paths (`internal/loader/loader.go`)

All three entry points hold `Loader.mu` for their whole duration, with one exception
noted below.

**`CreateNewWallet(ctx, pubPass, privPass, seed)`** (`:198-282`), op
`loader.CreateNewWallet`:

1. Wallet already loaded → kind `Exist`, `wallet already opened`.
2. Stat `dbDirPath`: missing → `os.MkdirAll(dbDirPath, 0700)`; other stat error →
   wrapped; exists but not a directory → `"<path>" is not a directory` (`%q`), no
   kind.
3. `dbPath = dbDirPath/wallet.db`. Stat error → wrapped; exists → kind `Exist`,
   `wallet DB exists`.
4. Arm a deferred `os.Remove(dbPath)` that runs if the function returns an error
   (removal errors ignored; **the database handle is not closed first**).
5. `os.MkdirAll(dbDirPath, 0700)` again; `wallet.CreateDB("bdb", dbPath)`.
6. `wallet.Create(ctx, db, pubPass, privPass, seed, params)`; then
   `wallet.Open(ctx, cfg)`; then record wallet and db and run load callbacks.

**`CreateWatchingOnlyWallet(ctx, xpub, pubPass)`** (`:108-193`), op
`loader.CreateWatchingOnlyWallet`: identical shape with these textual differences:
already loaded → `wallet already loaded`; not a directory → kind `Invalid`; file
exists → `wallet already exists`; initialisation is `wallet.CreateWatchOnly`.

**`OpenExistingWallet(ctx, pubPass)`** (`:288-342`), op `loader.OpenExistingWallet`:

1. Wallet already loaded → kind `Exist`, `wallet already opened`.
2. **Unlock `mu`**, `wallet.OpenDB("bdb", dbDirPath/wallet.db)`, **re-lock `mu`**
   (the open can block forever on the file lock; other loader methods stay usable).
3. On error: log at error level `Failed to open database: <err>` and return the
   wrapped error.
4. Arm a deferred `db.Close()` that runs if the function returns an error.
5. `wallet.Open(ctx, cfg)`; record wallet and db; run load callbacks.

**`UnloadWallet()`** (`:375-393`), op `loader.UnloadWallet`: no wallet → kind
`Invalid`, `wallet is unopened`; `db.Close()` (error wrapped, state left loaded);
clear wallet and db. It does not stop the wallet's other activity.

Load callbacks (`onLoaded`, `:81-89`) run synchronously, in registration order,
with `mu` held, before `wallet`/`db` are recorded; a callback must therefore not call
back into the loader. The callback list is discarded after the first load.

`OpenExistingWallet` does not re-check `l.wallet` after re-acquiring `mu` in step 2.
Two concurrent calls both pass step 1; the second blocks on the file lock inside
`wallet.OpenDB` (the first call's handle holds it) for as long as the first wallet
stays loaded, and proceeds — opening a second `wallet.Wallet` and overwriting
`l.wallet`/`l.db` — as soon as `UnloadWallet` closes the first handle.

The daemon (`dcrwallet.go:163-225`) builds the loader with
`dbDir = networkDir(appdata, params)`, opens via `OpenExistingWallet` in a goroutine
(so a signal can abandon a blocked open), and on shutdown calls `UnloadWallet`,
logging `Closed wallet` at info level on success and `Failed to close wallet: <err>`
at error level on failures other than "unopened". When unwinding from a panic the
unload is **skipped** on purpose (a panic inside a transaction would deadlock
`Close`). If the daemon returns because its context was cancelled while the open
goroutine is still blocked on the file lock, the deferred `UnloadWallet` finds no
wallet (kind `Invalid`, not logged), and the blocked goroutine is abandoned to
process exit.

### 5.16 Command-line creation paths (`walletsetup.go`, `config.go:590-665`)

- `--create`: `checkCreateDir(netDir)` (0700), then `createWallet` →
  `Loader.CreateNewWallet` → post-creation wallet calls → `Loader.UnloadWallet()`;
  prints `Creating the wallet...` before and `The wallet has been created
  successfully.` after, to stdout.
- `--createwatchingonly`: `checkCreateDir`, prompt, `wallet.CreateDB("bdb", dbPath)`,
  `wallet.CreateWatchOnly`; on failure `os.Remove(dbPath)` while the handle is still
  open; the handle is closed by a deferred `db.Close()`. Prints `Creating the
  wallet...` and `The watching only wallet has been created successfully.`
- `--createtemp`: `checkCreateDir`, writes `<netDir>/seed` with mode 0644, then
  `wallet.CreateDB("bdb", dbPath)`, `wallet.Create`, deferred `db.Close()`. A failure
  does not remove the file. The seed file holds the mnemonic encoding of a fresh
  random seed (`walletseed.EncodeMnemonic`, no trailing newline added) and is written
  **before** the database exists, so a failed database creation leaves the seed file
  behind (`walletsetup.go:278-291`). Prints `Creating the wallet...` after the seed
  file is written and `The wallet has been created successfully.` at the end
  (`walletsetup.go:295`, `:310`). The passphrases are `wallet.SimulationPassphrase`
  (private) and `wallet.InsecurePubPassphrase` (public).
- In all three paths the process exits (or continues, for `--createtemp`) only after
  the database handle has been closed, so the file lock is released before a
  following start opens the wallet.

### 5.17 Read-only opens (bbolt capability, unused by dcrwallet)

dcrwallet never sets `Options.ReadOnly`. For tools that do (for example the `bbolt`
command-line utility inspecting a wallet file), bbolt v1.3.12 behaves as follows
(`db.go:218-225`, `:247`, `:294-300`, `:680-685`, `:771-775`): the file is opened
`O_RDONLY` (still with `O_CREATE`); the lock is **shared** (`LOCK_SH | LOCK_NB`, same
retry loop), so any number of read-only openers coexist with each other but not with
a writable opener — while a wallet (Go or Rust) holds the exclusive lock, a read-only
opener waits, and vice versa; the freelist is not loaded unless `PreLoadFreelist` is
set; `Begin(true)` returns `ErrDatabaseReadOnly`; no growth, truncation or freelist
flush happens; `Close` does not unlock explicitly (closing the descriptor drops the
lock). The Rust implementation's exclusive lock must therefore be the same lock type
that these shared locks conflict with (section 5.2).

### 5.18 Consistency check (`Tx.Check`, `tx_check.go`; unreachable from dcrwallet)

The reference for the port's own `check` and for the oracle comparison of
section 11.3. Findings are reported one by one, in this order; the check does not
stop at the first.

1. Load the freelist if it is not loaded. Take all free and pending ids; for each id
   seen twice report `page N: already freed`.
2. Mark pages 0 and 1 reachable; if `meta.freelist != pgidNoFreelist` mark the
   freelist page and its `overflow` continuation pages reachable.
3. Walk buckets starting from the root bucket. An inline bucket (`root == 0`) is
   skipped entirely. For a regular bucket, visit every page of its tree depth-first
   (page, then children in element order) and for each logical page:
   - if `id > meta.pgid` (strictly greater) report
     `page N: out of bounds: <pgid> (stack: [ids])`;
   - for each of its `overflow + 1` physical pages, if already marked reachable
     report `page N: multiple references (stack: [ids])`; mark it;
   - if the page id is in the freed set report `page N: reachable freed`; otherwise
     if the page is neither branch nor leaf report
     `page N: invalid type: <typ> (stack: [ids])`.
4. For the same bucket, check key order over the tree: within a leaf and within a
   branch keys must be strictly ascending (`key[i]=(hex)K on <leaf|branch> page(N)
   needs to be > (found <) than previous element (hex)P. Stack: [ids]`, or
   `(found =)`); the first key of a child must be `>=` the parent's separator key for
   it (`the first key[0]=(hex)K on <type> page(N) needs to be >= the key in the
   ancestor (P). Stack: [ids]`); every key in a child must be `<` the parent's next
   separator (`key[i]=(hex)K on <type> page(N) needs to be < than key of the next
   element in ancestor (hex)P. Pages stack: [ids]`; `P` in this message is the
   *previous* key, not the separator that was violated, `tx_check.go:187-190`). For
   a branch element the page number `N` printed is the child's page id
   (`tx_check.go:141`). A page of another type gives
   `unexpected page type for pgId:N`. Keys are rendered in lowercase hex.
5. Recurse into every nested bucket of the bucket (elements with the bucket flag, in
   key order).
6. For every id in `[0, meta.pgid)` that is neither reachable nor freed report
   `page N: unreachable unfreed`.

The check neither inspects inline buckets nor verifies element offsets, sizes,
`count`, or that bucket values are well-formed; the Rust checker should add those.

---

## 6. Constants, limits and defaults

| Name | Value | Source | What depends on it |
| --- | --- | --- | --- |
| database file name | `wallet.db` | `internal/loader/loader.go:22`, `config.go:69` | file location |
| driver name | `bdb` | `wallet/internal/bdb/driver.go:16`, `internal/loader/loader.go:23` | registry lookups |
| file create mode | `0600` | `wallet/internal/bdb/db.go:321` | permissions of a new `wallet.db` |
| directory create mode | `0700` | `internal/loader/loader.go:122,153,212,243`, `walletsetup.go:375` | network directory |
| simulation seed file | name `seed` in the network directory, mode `0644` | `walletsetup.go:288` | `--createtemp` |
| trace region names | `db.View`, `db.ReadTx`, `db.Update`, `db.ReadWriteTx` | `wallet/walletdb/interface.go:204`, `:211`, `:231`, `:238` | Go execution traces only (Q27) |
| Windows map protection | `PAGE_READONLY` mapping, `FILE_MAP_READ` view of the whole mapping | `bolt_windows.go:77-83` | none in the file |
| Unix map protection | `PROT_READ`, `MAP_SHARED`, `MADV_RANDOM` | `bolt_unix.go:55-61` | none in the file |
| `magic` | `0xED0CDAED` | `bbolt@v1.3.12/db.go:23` | meta validation |
| `version` | `2` | `db.go:20` | meta validation |
| `pgidNoFreelist` | `0xFFFFFFFFFFFFFFFF` | `db.go:25` | unsynced-freelist marker |
| default page size | OS page size (`os.Getpagesize()`) | `db.go:41`, `:255-258` | page size of new files; inline and split thresholds scale with it |
| `pageHeaderSize` | 16 | `page.go:10`, `:30-35` | all layouts |
| `branchPageElementSize` | 16 | `page.go:14`, `:111-115` | branch layout, node size |
| `leafPageElementSize` | 16 | `page.go:15`, `:123-128` | leaf layout, node size |
| `bucketHeaderSize` | 16 | `bucket.go:17`, `:49-52` | bucket values |
| meta structure size | 64 (checksum over first 56) | `db.go:1337-1347`, `:1386-1390` | meta layout |
| page flags | branch `0x01`, leaf `0x02`, meta `0x04`, freelist `0x10` | `page.go:17-22` | page typing |
| `bucketLeafFlag` | `0x01` | `page.go:24-26` | nested-bucket elements |
| `minKeysPerPage` | 2 | `page.go:12` | split: no split at `<= 4` elements; each side keeps `>= 2` |
| node `minKeys` | leaf 1, branch 2 | `node.go:32-37` | rebalance trigger |
| `DefaultFillPercent` | 0.5 | `bucket.go:26` | split threshold `pageSize/2` |
| `minFillPercent` / `maxFillPercent` | 0.1 / 1.0 | `bucket.go:19-22` | clamp (unreachable in dcrwallet) |
| rebalance threshold | `pageSize / 4` (node size must exceed it) | `node.go:419-422` | merge on delete |
| inline bucket limit | serialized size `<= pageSize / 4` | `bucket.go:618-635` | inline vs regular bucket |
| `MaxKeySize` | 32768 | `bucket.go:11` | `Put` → `ErrKeyTooLarge` |
| `MaxValueSize` | `2^31 - 2` | `bucket.go:14` | `Put` → `ErrValueTooLarge` |
| max elements per node | `< 0xFFFF` | `node.go:203-205` | panic otherwise |
| freelist count escape | `0xFFFF` | `freelist.go:68-75`, `:277-287`, `:325-341` | freelist page layout |
| minimum map size | `2^15` (32 KiB) | `db.go:536-542` | first growth target of a new file |
| `maxMmapStep` | `2^30` (1 GiB) | `db.go:17` | map growth above 1 GiB |
| `maxMapSize` | `0xFFFFFFFFFFFF` (amd64, arm64, riscv64, ppc64, ppc64le, s390x, loong64); `0x7FFFFFFF` (386, arm, ppc); `0x8000000000` (mips64); `0x40000000` (mips) | `bolt_<arch>.go` | `mmap too large` |
| `maxAllocSize` | `0x7FFFFFFF` (64-bit), `0xFFFFFFF` (32-bit) | `bolt_<arch>.go` | write chunking (`tx.go:487-510`) |
| `DefaultAllocSize` | 16 MiB | `db.go:37` | file growth step above 16 MiB |
| `flockRetryTimeout` | 50 ms | `db.go:44` | lock polling interval |
| lock timeout | 0 (wait forever) | `db.go:1290-1294`, `wallet/internal/bdb/db.go:321` | second opener blocks |
| `DefaultMaxBatchSize` / `DefaultMaxBatchDelay` | 1000 / 10 ms | `db.go:35-36` | unused by dcrwallet |
| `IgnoreNoSync` | true only on OpenBSD | `db.go:31` | unused (`NoSync` is false) |
| initial file | 4 pages; `root=3`, `freelist=2`, `pgid=4`, txids 0 and 1 | `db.go:599-641` | image of a new file |
| page-size probe range | offsets `1024 << i`, `i = 0..14`; read size 4096 | `db.go:359-407` | recovery when meta 0 is bad |
| "no open reader" txid | `0xFFFFFFFFFFFFFFFF` | `db.go:810`, `:822` | upper bound of the pending-release scan |
| first data page id | 2 (pages 0 and 1 are metas; freeing or allocating an id `<= 1` panics) | `freelist.go:116-118`, `:154-156`, `db.go:1213` | freelist contents, rebuild scan range |
| loader log subsystem | `LODR` | `internal/loggers/loggers.go:47` | tag of `Failed to open database: …` |
| lock byte (Windows) | offset `0xFFFFFFFF_FFFFFFFF`, length 1 | `bolt_windows.go:31-35`, `:55-59` | mutual exclusion with Go on Windows |

---

## 7. Concurrency and lifecycle

### 7.1 Locks inside bbolt (`bbolt@v1.3.12/db.go:165-168`)

| Lock | Kind | Held by | Purpose |
| --- | --- | --- | --- |
| `rwlock` | mutex | a write transaction from begin to commit/rollback; `Close` | single writer |
| `metalock` | mutex | briefly at transaction begin/end, during the meta `pwrite`, `Close` | meta page and reader list |
| `mmaplock` | RW mutex | shared: every read transaction for its lifetime; exclusive: remap and `Close` | map stability |
| `statlock` | RW mutex | statistics | none observable |

Acquisition order: `rwlock` → `metalock` → `mmaplock` (`db.go:646-656`, `:726-735`,
`:777-784`).

### 7.2 Guarantees observable by callers

1. Any number of read transactions run concurrently with each other and with one
   write transaction. A reader never observes a partial or later commit.
2. Write transactions are serialized; `BeginReadWriteTx` blocks until the previous
   writer commits or rolls back. Ordering among waiting writers is that of a Go
   `sync.Mutex` (not strictly FIFO).
3. A commit that must enlarge the memory map waits for all read transactions to end,
   and new read transactions wait behind it. A goroutine that holds a read
   transaction and then commits a growing write transaction deadlocks itself. This is
   a liveness property of the Go implementation only; a port without a shared map
   need not reproduce the stall, but must not introduce a deadlock the Go code does
   not have.
4. `Close` takes all three locks: it blocks until the write transaction and every
   read transaction have finished (`db.go:643-657`). After `Close`,
   `Begin*` returns `ErrDatabaseNotOpen` (kind `Invalid`); a second `Close` returns
   nil.
5. Transactions, buckets and cursors are not safe for use from several goroutines at
   once; one goroutine per transaction.
6. Byte slices returned by `Get`, `ForEach` and cursors alias the memory map (or the
   transaction's node memory) and are valid only until the transaction ends; writing
   to one faults. Using a bucket or cursor after its transaction ended is undefined
   behaviour in Go (possible fault). The Rust API must make both impossible by
   construction (lifetimes) rather than reproduce them.
7. There are no background goroutines, timers or channels in the paths dcrwallet
   uses. (`Batch` and `Tx.Check` start goroutines; neither is reachable. The
   freelist rebuild scan, `db.go:1187-1219`, starts one short-lived goroutine that
   panics on a reported inconsistency; it runs only for a file whose meta has
   `freelist = pgidNoFreelist`.)
8. Context cancellation does not reach the database layer (section 5.13).

### 7.3 Process-level lifecycle

1. Start: the file lock is acquired at open, before any wallet data is read. A second
   process on the same file blocks in open (section 5.2).
2. Run: the handle stays open, and the lock held, for as long as the wallet is
   loaded.
3. Stop: `Loader.UnloadWallet` → `Close` → unmap, unlock, close. Callers must have
   ended all transactions first or `Close` blocks. After a panic the daemon skips the
   close and relies on process exit to drop the lock (`dcrwallet.go:176-189`).
4. gRPC `WalletLoaderService.CloseWallet` followed by `OpenWallet` reuses the same
   `Loader` (`UnloadWallet` resets it).

---

## 8. Errors

### 8.1 bbolt error → wallet error kind (`wallet/internal/bdb/db.go:18-36`)

`convertErr` returns `errors.E(kind, err)`: an `*errors.Error` with the kind below and
the bbolt error as its cause (if the cause is already an `*errors.Error`, e.g. from a
`ForEach` callback, the wrapper collapses into an equivalent copy). Matching is by
identity with the sentinel value.

| bbolt error | Text | Kind |
| --- | --- | --- |
| `ErrInvalid` | `invalid database` | `IO` |
| `ErrDatabaseNotOpen` | `database not open` | `Invalid` |
| `ErrTxNotWritable` | `tx not writable` | `Invalid` |
| `ErrTxClosed` | `tx closed` | `Invalid` |
| `ErrBucketNameRequired` | `bucket name required` | `Invalid` |
| `ErrKeyRequired` | `key required` | `Invalid` |
| `ErrKeyTooLarge` | `key too large` | `Invalid` |
| `ErrValueTooLarge` | `value too large` | `Invalid` |
| `ErrIncompatibleValue` | `incompatible value` | `Invalid` |
| `ErrBucketNotFound` | `bucket not found` | `NotExist` |
| `ErrBucketExists` | `bucket already exists` | `Exist` |
| `ErrVersionMismatch` | `version mismatch` | `Other` (unclassified) |
| `ErrChecksum` | `checksum error` | `Other` |
| `ErrTimeout` | `timeout` | `Other` (unreachable: no timeout is set) |
| `ErrInvalidMapping` | `database isn't correctly mapped` | `Other` |
| `ErrDatabaseReadOnly` | `database is in read-only mode` | `Other` (unreachable) |
| `ErrFreePagesNotLoaded` | `free pages are not pre-loaded` | `Other` (unreachable) |
| `ErrDatabaseOpen` | `database already open` | `Other` (never returned by v1.3.12) |
| any other error (OS errors, `file size too small`, `mmap too large`, `mmap stat error: …`, `madvise: …`, `truncate: …`, `CreateFileMapping: …`, `MapViewOfFile: …`, `CloseHandle: …`, `unmap error: …` (Windows: `unmap error: UnmapViewOfFile: …`), `…; rollback unmap also failed: …`, `mmap allocate error: …`, `file resize error: …`, `file sync error: …`, `mlock/munlock error: …` (unreachable), `bolt.Close(): funlock error: …`, `db file close: …`, `delete bucket: …`, `meta N copy: …`, a short-read error from the data copy of `Copy`) | as produced | `Other` |
| an error returned by a `ForEach` callback | as produced | the callback error's own kind if it is an `*errors.Error`, else `Other`; always a new wrapper value (section 3.2) |

Kind strings as rendered (`errors/errors.go:77-123`): `Invalid` → `invalid operation`,
`IO` → `I/O error`, `Exist` → `item already exists`, `NotExist` →
`item does not exist`; `Other` contributes no text.

### 8.2 Errors produced by this layer itself

| Site | Op | Kind | Text |
| --- | --- | --- | --- |
| `openDB` (`db.go:317-319`) | — | `NotExist` | `missing database file` |
| `parseArgs` (`driver.go:21-30`) | — | none (plain error) | `invalid arguments to bdb.Open -- expected database path`; `first argument to bdb.Open is invalid -- expected database path string` (and the same with `Create`) |
| `walletdb.RegisterDriver` | `walletdb.RegisterDriver` | `Exist` | `driver "bdb" is already registered` |
| `walletdb.Create` / `Open` | `walletdb.Create` / `walletdb.Open` | `Invalid` | `driver "<name>" is not registered` |
| `wallet.OpenDB` / `CreateDB` | `wallet.OpenDB` / `wallet.CreateDB` | inherited | wraps the above |
| loader | `loader.CreateNewWallet` etc. | see section 5.15 | see section 5.15 |

### 8.3 Rendering and surfacing

- Error strings join nested fields with `errors.Separator`, which the daemon sets to
  `":: "` at start (`dcrwallet.go:38`; the package default is `":\n\t"`). Example, a
  missing file opened through the loader: the error returned is
  `loader.OpenExistingWallet: item does not exist:: wallet.OpenDB: missing database file`
  and the log line is `Failed to open database: wallet.OpenDB: item does not exist: missing database file`.
- The daemon then logs `Failed to open wallet: <err>` and exits with that error
  (`dcrwallet.go:206-225`); config-time failures (`config.go:590-665`) print to stderr
  and fail config loading. Exit-code mapping belongs to the daemon/config spec.
- gRPC: `WalletLoaderService` passes loader errors through `translateError`
  (defined at `internal/rpc/rpcserver/server.go:80-83`; loader call sites
  `:2774-2815`); `CloseWallet` maps kind `Invalid`
  from `UnloadWallet` to `FailedPrecondition` with message `Wallet is not loaded`. The
  kind → status mapping belongs to the gRPC spec.
- Panics: bbolt reports internal inconsistencies by panicking
  (`assertion failed: …`, `page N already freed`, `invalid freelist page: …`,
  `bolt.DB.meta(): invalid meta pages`, `inode overflow: …`, `pgid (N) above high water
  mark (M)`, `misplaced bucket header: …`). Complete list of panic texts in the
  reachable code, for triage of crash reports from Go wallets and as the catalogue of
  conditions the port must classify (section 13 item 7):

  | Text | Raised when | Source |
  | --- | --- | --- |
  | `assertion failed: Page expected to be: A, but self identifies as B` | a page's `id` field differs from its position | `page.go:57` |
  | `assertion failed: page N: has unexpected type/flags: X` | page flags are not exactly one of the four types | `page.go:59-63` |
  | `invalid page type: N: X` | a seek reaches a page that is neither branch nor leaf | `cursor.go:282-284` |
  | `assertion failed: tx closed` | public cursor move on a finished transaction | `cursor.go:33`, `:64`, `:93`, `:105`, `:118` |
  | `assertion failed: accessing a node with a zero-length cursor stack`, `… expected branch node`, `… expected leaf node` | cursor/node bookkeeping broken | `cursor.go:388-404` |
  | `assertion failed: nodes map expected` | mutation through a read-only or dead bucket handle | `bucket.go:666` |
  | `inline bucket non-zero page access(2): N != 0` | page lookup in an inline bucket with a non-zero id | `bucket.go:731-733` |
  | `misplaced bucket header: K -> K2`, `unexpected bucket header flag: X` | at spill, a dirty child bucket's entry is missing from, or not a bucket in, its parent | `bucket.go:577-582` |
  | `pgid (N) above high water mark (M)` | a spilled node or bucket root got an id `>= meta.pgid` | `bucket.go:598-600`, `node.go:374-376` |
  | `pgId (N) above high water mark (M)`, `put: zero-length old key`, `put: zero-length new key`, `assertion failed: put: zero-length inode key` | node insert invariants | `node.go:117-140` |
  | `assertion failed: read: zero-length inode key`, `… read: zero-length node key` | a page with an empty key is materialised | `node.go:178`, `:184` |
  | `assertion failed: node cannot be written into a not empty page`, `… write: zero-length inode key`, `… write: circular dependency occurred`, `inode overflow: N (pgid=P)` | node serialisation | `node.go:194-237` |
  | `assertion failed: spill: zero-length node key`, `… parent must have at least 2 children`, `… dereference: zero-length node key on existing node`, `… dereference: zero-length inode key` | spill/rebalance/remap invariants | `node.go:390`, `:460`, `:528`, `:537` |
  | `invalid childAt(N) on a leaf node` | child lookup on a leaf | `node.go:74-76` |
  | `cannot free page 0 or 1: N`, `page N already freed`, `invalid page allocation: N` | freelist misuse or a corrupt freelist | `freelist.go:154-156`, `:174-176`, `:116-118` |
  | `invalid freelist page: N, page type is T`, `leading element count N overflows int` | `meta.freelist` points at a non-freelist page; absurd count | `freelist.go:274-286` |
  | `mergepgids bad len A < B + C` | freelist page buffer smaller than the id count | `page.go:175-177` |
  | `bolt.DB.meta(): invalid meta pages` | both metas invalid after open (file changed underneath) | `db.go:1108` |
  | `root bucket pgid (N) above high water mark (M)`, `freelist pgid (N) above high water mark (M)` | meta about to be written is inconsistent | `db.go:1368-1373` |
  | `freepages: failed to open read only tx`, `freepages: failed to rollback tx`, `freepages: failed to get all reachable pages (E)` | freelist rebuild scan (section 5.5) | `db.go:1187-1206` |
  | `assertion failed: managed tx commit not allowed`, `… managed tx rollback not allowed` | `Commit`/`Rollback` inside bbolt's own `View`/`Update`; reachable only from within `Copy`'s closure, which does neither | `tx.go:144`, `:260` |
  | `check fail: …` | `StrictMode` only (unreachable) | `tx.go:207-219` |
  | Go runtime faults (nil dereference, index out of range, SIGSEGV/SIGBUS on the map) | use after transaction end, never-positioned cursor `Delete`, element offsets or page ids pointing outside the file | sections 5.7, 7.2 |

  In dcrwallet a panic terminates the
  process. A port should turn file-corruption conditions into errors of kind `IO` and
  keep true internal-invariant violations as panics; the choice is recorded in
  section 13.

---

## 9. Compatibility-critical behaviours

### 9.1 Required for cross-compatibility (a file valid to both implementations)

1. **Location and name.** `<appdata>/<mainnet|testnet3|simnet>/wallet.db`; directory
   created 0700, file created 0600 (umask applies). No sidecar files (no lock file,
   no WAL, no journal). The Rust wallet must not create any.
2. **Lock.** Take the same advisory lock on the same file as bbolt (section 5.2:
   `flock` `LOCK_EX` on Unix; `LockFileEx` on the single byte at offset `2^64-1` on
   Windows), hold it while the database is open, and wait for it the way bbolt does
   when it is held (block, no timeout). Otherwise a Go dcrwallet, Decrediton's bundled
   dcrwallet, or a second instance can open the file concurrently and corrupt it.
   Rust's `std::fs::File::lock` must not be used on Windows: in the Rust 1.99.1
   standard library (`library/std/src/sys/fs/windows.rs:409-424`, `:458-469`) it is
   `LockFileEx` at offset 0 with length `0xFFFFFFFF_FFFFFFFF`, i.e. bytes
   `[0, 2^64 - 1)`, which does not include the byte at offset `2^64 - 1` that bbolt
   locks (and it would lock the data range bbolt leaves unlocked). On Linux, macOS
   and the BSDs the same release uses blocking `flock(fd, LOCK_EX)`
   (`library/std/src/sys/fs/unix.rs:1321-1343`), the right lock type but without
   bbolt's 50 ms polling; re-check both against the toolchain dcr-purse pins.
3. **Page size from the file.** Read `pageSize` from the meta page (section 5.3),
   including the meta-1 probe. Files created on macOS arm64 or 16K/64K-page Linux
   kernels have 16384- or 65536-byte pages and must open on any platform; bbolt opens
   foreign page sizes too.
4. **Meta pages.** Exact layout, magic, version 2, FNV-1a checksum (section 4.3).
   Select the newer valid meta; write commit `txid` to page `txid mod 2` with
   `txid = previous + 1`; never touch the other meta page in the same commit.
5. **Commit ordering.** All data pages and the freelist page written and synced
   before the meta page is written; meta page synced before the commit is reported
   successful (sections 5.9.7–5.9.8).
6. **Copy-on-write.** Never overwrite a page reachable from the last committed meta
   (or from any live reader snapshot). Pages freed by commit `T` may be reused from
   commit `T + 1` on, not earlier.
7. **Freelist.** Persist a freelist page in the format of section 4.4 at every commit
   and point `meta.freelist` at it; list exactly the unreachable pages in `[2, pgid)`
   (free plus pending), sorted ascending, with the `0xFFFF` count escape. Read both
   forms, and accept `meta.freelist = pgidNoFreelist` by rebuilding (a file last
   written by a `NoFreelistSync` tool). Do not write `pgidNoFreelist`: it forces
   a full scan plus an extra commit on the next Go open (which then persists a
   freelist page again), and boltdb/bbolt versions that
   predate the marker would dereference it as a page id (the latter UNVERIFIED,
   section 13 item 9).
8. **Page headers.** Every page written has `id` equal to its position, exactly one
   type flag, the right `count`, and exact `overflow`. Go panics otherwise
   (section 4.2).
9. **Branch and leaf layouts** exactly as sections 4.5–4.7: 16-byte elements, `pos`
   relative to the element header, keys strictly ascending bytewise, no empty keys,
   branch keys equal to the first key of the child subtree.
10. **Buckets.** 16-byte header `{root, sequence}`; inline form when `root == 0`;
    never a bucket inside an inline bucket; the root bucket never inline; `sequence`
    preserved (section 4.8–4.9).
11. **High-water mark and file length.** `meta.pgid` covers every referenced page;
    file length `>= meta.pgid * pageSize` (bbolt keeps it `>= (pgid+1) * pageSize`
    after growth) and `>= 2 * pageSize`.
12. **Little-endian** encoding of every integer.
13. **API semantics the upper layers rely on** (sections 5.7, 5.8, 5.12, 5.13):
    ascending byte order of iteration; `value == nil` for nested buckets in `ForEach`
    and cursors; `Get` returns nil for a bucket key; `Seek` past the end followed by
    `Prev` yields the last element; `Next` at the end and `Prev` at the start leave
    the cursor in place and return nil; `Delete` of a missing key succeeds;
    `CreateBucket` on an existing bucket is `Exist`; `DeleteNestedBucket`/
    `DeleteTopLevelBucket` on a missing bucket is `NotExist`; the error-kind table of
    section 8.1; snapshot isolation; single writer; `Update` commits only when the
    closure returns nil; `KeyN` semantics.
14. **Zero-length and garbage files.** A zero-length `wallet.db` is initialised as an
    empty database at open; a non-database file fails with kind `IO`
    (`invalid database`), `version mismatch`, `checksum error` or
    `file size too small` as in sections 5.1 and 5.3.
15. **No write on read.** Opening, reading and closing must not modify the file
    (except the freelist flush of section 5.1 step 7; Go on Windows additionally
    extends the file length to the map size on every writable open, section 5.4), so
    that backups and file-hash-based tooling see an unchanged file.

16. **Tolerate leaked pages.** A file written by Go can contain pages in
    `[2, pgid)` that are neither reachable nor on the freelist (section 5.8,
    `DeleteBucket`, Q29). The Rust reader and its checker must accept such a file
    (bbolt's own `Tx.Check` reports them as `unreachable unfreed` but nothing in the
    open or commit path looks for them) and must never treat "unlisted" as "free".
    Reclaiming them is optional and safe only by a full reachability scan.

### 9.2 Matters only for byte-identical files (policy, not validity)

These determine *which* valid file a commit produces. bbolt reads any choice
correctly. Reproducing them keeps file size, growth and performance characteristics
identical to Go and makes differential testing sharper; where bbolt itself is
non-deterministic, byte identity is impossible.

| Property | Go behaviour | Deterministic in Go? |
| --- | --- | --- |
| Split point / fill | section 5.9.3, threshold `pageSize/2` | yes |
| Rebalance (merge) rule | section 5.9.1 | rule yes; order of node visits is Go map order |
| Inline threshold | `<= pageSize/4`, no sub-buckets | yes |
| Which nodes are rewritten | every node on the path of a touched leaf; `Put` of an identical value still rewrites | yes |
| Page id assignment | first-fit lowest free run, else high-water mark; order of allocation follows spill order | **no** — sibling buckets are spilled in Go map iteration order (`bucket.go:550`) |
| Freelist page size | `(16 + 8n)/pageSize + 1` pages with `n` taken before its own allocation | yes |
| Element data packing | contiguous, element order | yes |
| Zero padding | unused bytes zero | yes |
| File length | section 5.9.6 (map-size doubling to 16 MiB, then +16 MiB) ; Windows: map size | yes |
| A commit with no changes | still rewrites freelist + meta, `txid + 1` | yes |
| Page size of a new file | OS page size | per platform |
| `fsync` vs `fdatasync` and number of syncs | section 5.9.6–5.9.8 | per platform |

Recommendation: implement every deterministic row exactly, pick a fixed order
(ascending bucket name) where Go uses map order, and state in the project's parity
notes that page numbering can differ from any particular Go run.

---

## 10. Quirks

Each item is something a faithful port either reproduces or consciously diverges
from.

**Q1. `View` never returns a rollback error; the assignment is dead code.**
`wallet/walletdb/interface.go:215-220` assigns the rollback error to `err` only
`if err != nil`, where `err` is the (nil) result of `BeginReadTx`, and the function
has no named result. `View` returns exactly what `f` returned.

**Q2. Package documentation describes a removed API.**
`wallet/walletdb/doc.go` talks about `Namespace`, `Tx.RootBucket`, `Begin`, and says
that manually committing or rolling back a managed transaction panics. None of that
exists: transactions passed to `View`/`Update` are ordinary transactions and a manual
`Commit`/`Rollback` inside the closure makes the helper's own call fail with
`tx closed` (returned by `Update`, dropped by `View`). `example_test.go` uses the
removed API and is excluded by `//go:build disabled`. The code wins.

**Q3. `CreateTopLevelBucket` fails when the bucket exists.** The interface comment
(`interface.go:40-42`) reads as create-if-missing; the implementation
(`wallet/internal/bdb/db.go:57-63`) is bbolt `CreateBucket` → kind `Exist`.

**Q4. "Open" can create, "Create" can open.** Both drivers call the same
`bolt.Open` with `O_CREATE` (`wallet/internal/bdb/db.go:316-323`). `Create` on an existing database
opens it without error; the loader and `config.go` do the existence checks. `Open`
creates the file if it vanishes after the pre-check, and initialises an existing
zero-length file as an empty database. The pre-check `fileExists` treats every stat
error except not-exist as "exists".

**Q5. `DeleteNestedBucket` with an empty key depends on the bucket's contents.**
Kind `Invalid` (`incompatible value`) when the parent bucket is empty, kind
`NotExist` (`bucket not found`) when it is not (`bbolt@v1.3.12/bucket.go:222-231`,
`cursor.go:367-373`). The conformance test asserts `Invalid` on a freshly created,
empty bucket (`wallet/walletdb/interface_test.go:647`).

**Q6. nil versus empty values.** Go callers can distinguish them and some do
(`for k, v := c.First(); v != nil; …` in `wallet/udb/upgrades.go:550`, `:570`, `:645`,
`:683`, `:739`, `:787`, `:1166`, `:1511`, `:1604`).
From a committed page a zero-length value is a non-nil empty slice
(`page.go:138-142`); a nested bucket and end-of-iteration give nil. A value written
earlier in the same write transaction is returned as the very slice that was passed
to `Put`, so `Put(k, nil)` followed by `Get(k)` in the same transaction returns nil
(indistinguishable from "absent"), but returns non-nil empty after commit. The Rust
API should return `Some(&[])` for a present empty value in both cases and the
`wallet/udb` port must audit every nil test against that.

**Q7. `KeyN` is not "keys in this bucket as this transaction sees it".** It counts
committed pages, recursively through nested buckets, ignoring the transaction's own
pending changes (section 5.12). For a bucket created in the current transaction that
has had a nested bucket created in it, the walk starts at page 0 (a meta page) and
reports 0.

**Q8. `Put` keeps the caller's value slice.** bbolt copies the key but stores the
value by reference until commit (`bucket.go:277-313`); a Go caller that reuses the
buffer before commit changes what is written. A port should copy and the `wallet/udb`
port should check for buffer reuse between `Put` and commit.

**Q9. Cursors under mutation.** A cursor whose current leaf was not yet materialised
as a node keeps reading the page snapshot, so puts and deletes made through the
bucket are invisible to it until it moves to another leaf; a cursor on an
already-materialised leaf reads the live node, where a delete at or before its index
makes the following `Next` skip an element and an insert before its index makes it
repeat one (`cursor.go:212-244`, `:367-406`, `bucket.go:727-749`). `Cursor.Delete`
followed by `Next` therefore skips or does not skip depending on earlier writes in
the transaction — the reason the cursor-delete conformance test is disabled
(`wallet/walletdb/interface_test.go:99-120`, `:202-205`, citing boltdb issue 620).
dcrwallet does not call `ReadWriteCursor.Delete` (the only uses are commented out,
`wallet/udb/txdb.go:444`, `:1664`), but code that mutates a bucket inside `ForEach` or
a cursor loop inherits these semantics.

**Q10. Rebalance merges unconditionally.** The comment says "if both this node and
the target node are too small then merge them"; the code merges with the sibling
whatever its size (`node.go:462-507`) and relies on the split at spill time.

**Q11. Page numbering depends on Go map iteration order.** Dirty sibling buckets are
spilled, and nodes rebalanced, in map order (`bucket.go:550`, `:656-661`), so two runs
of the same Go program can assign different page ids. Logical content is identical.

**Q12. Recorded file size starts at 0 after opening an existing file**
(`bbolt@v1.3.12/db.go:149`, `:638`, `:1144-1181`), so the first growing commit of
each session re-runs `ftruncate` and a full `fsync`. The target is normally the
current size or larger, but a file longer than the computed size is cut down to it
(section 5.9.6).

**Q13. Empty commits write.** A write transaction that changes nothing still frees
and rewrites the freelist page, writes a meta page and increments `txid`
(section 5.9.5).

**Q14. A failed wallet creation unlinks an open database.** The loader's deferred
`os.Remove(dbPath)` (`internal/loader/loader.go:146-150`, `:236-240`) runs without
closing the handle: the file descriptor and lock leak until process exit; on Windows
the removal fails (file open and mapped; Windows behaviour stated from platform
knowledge, UNVERIFIED against the Go runtime source) and the error is discarded,
leaving a partial `wallet.db` behind. `createWatchingOnlyWallet` (`walletsetup.go:356-363`) has
the same order (remove, then deferred close).

**Q15. `CreateNewWallet` drops `MixSplitLimit`.** Its `wallet.Config` omits the field
that the other two loader paths set (`loader.go:259-274` versus `:169-185`,
`:318-334`), so a wallet used directly after creation runs with a zero split limit
until reopened. (Recorded here because it is in a scoped file; the consequence
belongs to the mixing spec.)

**Q16. Inconsistent loader messages and kinds.** "wallet already opened" versus
"wallet already loaded"; "wallet DB exists" versus "wallet already exists"; the
not-a-directory error has kind `Invalid` only in the watching-only path
(`loader.go:115`, `:130`, `:140`, `:205`, `:220`, `:230`).

**Q17. Stale doc comments in the loader.** `UnloadWallet` says it "stops the loaded
wallet" and mentions `LoadExistingWallet`; it only closes the database
(`loader.go:371-393`). `OpenExistingWallet`'s comment describes a `canConsolePrompt`
parameter that does not exist (`loader.go:284-288`).

**Q18. `Close` blocks while any transaction is open** and the daemon deliberately
skips it when unwinding from a panic (`dcrwallet.go:176-189`).

**Q19. Error classification is by identity.** `convertErr` uses `switch err`; a
wrapped bbolt error such as `delete bucket: bucket not found` gets no kind
(`wallet/internal/bdb/db.go:23-34`, `bbolt@v1.3.12/bucket.go:235-240`).

**Q20. No bucket-name length limit.** bbolt's comment promises an error "if the
bucket name is too long"; `CreateBucket` checks only for an empty name
(`bucket.go:153-163`).

**Q21. `SupportedDrivers` order is unspecified** (map iteration,
`interface.go:291-297`). With one driver this is unobservable.

**Q22. `openDB` returns a non-nil interface on failure** (a typed nil pointer,
`wallet/internal/bdb/db.go:321-322`). Callers check the error first.

**Q23. A second open blocks silently and forever**, also within one process
(section 5.2). The gRPC `OpenWallet` call simply does not return while another
process holds the wallet.

**Q24. Read handles are secretly writable types.** `ReadBucket` and `ReadCursor`
return the read-write implementation (`wallet/internal/bdb/db.go:45-47`, `:109-111`,
`:194-196`); a type
assertion to `ReadWriteBucket` succeeds and writes then fail with kind `Invalid`
(`tx not writable`). Rust should not reproduce the escape hatch.

**Q25. End-of-range cursor moves do not move the cursor.** After `Next` returns nil
the cursor is still on the last element (a following `Prev` returns the
second-to-last); after `Prev` returns nil it is on the first element (a following
`Next` returns the second) (`cursor.go:225-229`, `:257-265`).

**Q26. `Copy` output always has the newer meta on page 0**, whatever the parity of
`txid` (`tx.go:382-405`). Still a valid file.

**Q27. Go-runtime dependence.** New-file page size is `os.Getpagesize()`; all
integers are host-endian; execution-trace regions `db.View`, `db.ReadTx`,
`db.Update`, `db.ReadWriteTx` exist only in Go traces (`interface.go:204`, `:211`,
`:231`, `:238`) and have no counterpart.

**Q28. Short garbage files give different errors by length.** `1..2048` bytes →
`invalid database` (kind `IO`); `2049` bytes up to one byte short of two OS pages
(`2049..8191` with 4096-byte pages) → `file size too small`; two OS pages or more
with bad metas → meta 0's validation error (`invalid database`, `version mismatch`
or `checksum error`) (sections 5.1, 5.3). Only `ErrInvalid` (`invalid database`)
carries kind `IO`; the others are unclassified.

**Q29. `DeleteBucket` can leak the pages of nested buckets.** When the leaf that
holds the nested-bucket entries of the bucket being deleted was already
materialised in the transaction, the recursive deletion skips every second entry
and the skipped buckets' pages are never freed (section 5.8;
`bbolt@v1.3.12/bucket.go:234-254`, `cursor.go:212-244`). The file stays valid; it
only grows. A port that frees every nested bucket produces a different freelist
from Go's for the same script — a section 9.2 difference, not a validity one.
Derived by reading; UNVERIFIED by execution.

**Q30. Upstream TODOs that describe real limits.** The meta-1 probe stops at a
16 MiB page size ("should we support larger page size?", `db.go:390`); the freelist
rebuild frees pages even though the walk reported corruption, were it not for the
panic of section 5.5 (`db.go:1210`); a meta with `freelist = pgidNoFreelist` is
written without complaint even when freelist sync is on (`db.go:1371`). The cursor's
branch search carries a "bit hacky" marker (`cursor.go:304`, `:327`): the `exact`
flag is set by any probe of the binary search that compares equal, which is
equivalent to "an element equal to the key exists" only because keys are unique
and sorted. None of these changes what a valid file is.

**Q31. `Tx.Copy` is deprecated upstream** (`tx.go:335-342`) and `db.Copy` is its
only use here; the walletdb `Copy` has no caller in dcrwallet (section 4.12).

**Q32. `--createtemp` refusals exit with status 0** after writing to stderr
(`config.go:575-589`), unlike every other creation-path failure, which is returned
as a configuration error.

---

## 11. Upstream tests

### 11.1 dcrwallet

| File | What it pins | Port? |
| --- | --- | --- |
| `wallet/walletdb/db_test.go` | `TestAddDuplicateDriver`: duplicate registration is kind `Exist` and does not replace the driver. `TestCreateOpenFail`: driver errors are returned unwrapped. `TestCreateOpenUnsupported`: unknown driver is kind `Invalid`. `TestInterface`: runs the conformance suite on a fresh `bdb` file. | Port the kinds; the registry itself may be dropped (section 12). |
| `wallet/walletdb/interface_test.go` | `testInterface` over two namespaces: put/get/`ForEach`/delete, nested buckets two levels deep, `CreateBucket` twice → `Exist`, `CreateBucketIfNotExists`, `DeleteNestedBucket` → then `NotExist`, manual transactions with rollback and commit, managed `Update` with forced rollback, `View`; `testAdditionalErrors`: `CreateBucket(nil)`, `DeleteNestedBucket(nil)` (on an empty bucket), `Put(nil, nil)` → `Invalid`; double `Rollback` and `Commit` after `Rollback` → `Invalid`. Cursor-delete test present but skipped. | Port in full. |
| `wallet/internal/bdb/driver_test.go` | `TestCreateOpenFail`: missing file → `NotExist`; exact argument-error strings; `BeginReadTx` on a closed database → `Invalid`. `TestPersistence`: values survive close and reopen. | Port in full. |
| `wallet/internal/bdb/interface_test.go` | Older copy of the conformance suite (header says it must be updated). | Covered by the walletdb copy. |
| `wallet/walletdb/example_test.go` | Disabled (`//go:build disabled`). | No. |
| `wallet/udb/testdata/v1.db.gz` … `v24.db.gz` (+ generators `*.db.go`, `v7.go`) | Real bbolt files written by Go dcrwallet at database versions 1, 2, 3, 4, 5, 7, 11, 24; page size 4096. Used by the upgrade tests of `wallet/udb`. | Use as read fixtures for the Rust engine: open, run the consistency check, dump, compare with a Go dump. `v1.db` has branch pages and 99 leaves; `v24.db` exercises inline buckets. |

Not covered by dcrwallet tests: `Copy`, `KeyN`, cursors (`Seek`/`Prev`/`Last`),
locking, loader create/open/unload, error paths of file open, concurrent readers and
writer.

### 11.2 bbolt v1.3.12 (the format oracle)

| File | What it pins | Port / regenerate |
| --- | --- | --- |
| `node_test.go` | `put` ordering; leaf page read/write byte layout; split at fill threshold (`TestNode_split`: 5 × 40-byte elements, page 100 → 2 + 3), no split at min keys or when it fits. | Port as unit vectors. |
| `freelist_test.go` | `free` with overflow, `release`, `releaseRange` table, array allocation (first fit), rollback, freelist page read/write, id merging. | Port (array type only; hashmap type not needed). |
| `page_test.go` | type names, `pgids.merge`. | Port merge. |
| `db_test.go` | open/create, `ErrInvalid`/`ErrVersionMismatch`/`ErrChecksum` on corrupted metas, page size from meta 1 (`TestOpen_ReadPageSize_FromMeta1_*`), `file size too small`, file size stability across reopen (`TestOpen_Size`), big page size, freelist recovery after `NoFreelistSync` (`TestOpen_RecoverFreeList`), begin on closed database, `Close` with pending transactions, `WriteTo` under concurrency, consistency after many commits. | Port; regenerate the corrupted-file cases through the oracle. |
| `tx_test.go` | closed/readonly transaction errors, bucket create/delete errors, `CopyFile`, rollback, `releaseRange` with interleaved readers, truncate-before-write growth steps (`TestTx_TruncateBeforeWrite`). | Port. |
| `bucket_test.go` (47 tests) | get/put/delete semantics incl. `ErrIncompatibleValue`, large values and overflow, nested and inline buckets, `DeleteBucket` freeing nested pages, stats (`KeyN`, inline accounting), sequence. | Port the reachable API; use stats tests for `KeyN`. |
| `cursor_test.go` (16 tests) | seek on exact/missing/large keys, empty buckets, iteration forward/backward, cursor delete, empty pages left by deletes, restart after end. | Port in full; these define section 5.7. |
| `simulation_test.go`, `quick_test.go`, `concurrent_test.go` | randomised model checking with concurrent readers/writers. | Re-implement as a differential fuzzer against the Go oracle. |
| `tx_check.go` (library code) | `Tx.Check`: double-free, unreachable-unfreed, multiple references, key order. | Implement an equivalent `check` in Rust and also run the Go one on Rust-written files. |
| `unix_test.go`, `manydbs_test.go`, `allocate_test.go` | mlock, many databases, page stats. | Not needed. |
| `db_whitebox_test.go` | `TestOpenWithPreLoadFreelist` (freelist loaded or not per `ReadOnly`/`PreLoadFreelist`), `TestMethodPage` (`Tx.Page` → `ErrFreePagesNotLoaded`). | Not needed (unreachable options); useful only if a read-only open is offered (section 13 item 12). |
| `simulation_no_freelist_sync_test.go` | the simulation suite under `NoFreelistSync` (13 size combinations). | Port as the test of the `pgidNoFreelist` rebuild path of section 5.5 (files produced by the Go oracle, opened by Rust). |
| `tx_stats_test.go`, `TestTxStats_*`/`TestDBStats_Sub` in `tx_test.go`/`db_test.go` | statistics counters. | Not needed. |
| `internal/tests/tx_check_test.go` | `TestTx_RecursivelyCheckPages_MisplacedPage`, `…_CorruptedLeaf`: exact `Tx.Check` findings on files damaged with `internal/surgeon` (page copied over another; leaf keys out of order). | Port against the Rust checker; regenerate the damaged files through the oracle. |
| `internal/surgeon/*_test.go`, `cmd/bbolt/*_test.go` | page-surgery helpers and the command-line tool. | Not ported; `surgeon` is the reference for building corrupted fixtures. |
| `tests/failpoint/db_failpoint_test.go` | `TestFailpoint_MapFail` (a failing map makes `Open` return the error), `TestFailpoint_UnmapFail_DbClose` (an `Open` that fails in unmap still releases the file lock: a following `Open` with a 30 s lock timeout succeeds), `TestFailpoint_ResizeFileFail`, `TestFailpoint_LackOfDiskSpace` (failed commit → `ErrTxClosed` afterwards, database still usable), `TestIssue72` (a caller mutating the key buffer during `Put` cannot corrupt the tree because the key is copied), mlock cases. | Port the commit-failure and close-failure cases with Rust-side fault injection. |
| `tests/robustness/powerfailure_test.go`, `tests/dmflakey` | power-failure recovery on ext4/XFS through a device-mapper flakey target (root, Linux only). | Not portable as is; the model for the crash-injection tests of section 12.4 item 2. |

The fault-injection points compiled into bbolt (gofail markers, inactive in a normal
build) name the places a port's own fault injection should cover: `mapError`
(`db.go:472`), `unmapError` (`db.go:524`), `resizeFileError` (`db.go:1162`),
`lackOfDiskSpace` (`tx.go:190`, before growth), `beforeSyncDataPages` (`tx.go:515`),
`beforeSyncMetaPage` (`tx.go:556`), `beforeBucketPut` (`bucket.go:308`), and the
mlock pair `munlockError`/`mlockError` (`db.go:571`, `:580`).

Untested upstream and here: cross-process lock contention, Windows locking, power
loss between the data sync and the meta sync, opening a file whose page size differs
from the host's while also writing to it, big-endian files.

### 11.3 Oracle shim

A small Go program built against `go.etcd.io/bbolt v1.3.12` and
`decred.org/dcrwallet/v5/wallet/walletdb` at the pin, driven over stdin/stdout, with
these commands: (1) `dump <file>`: canonical listing of every bucket path, key and
value, plus per-bucket `KeyN`; (2) `check <file>`: `Tx.Check` results and the
freelist; (3) `apply <file> <script>`: execute a script of transactions
(put/delete/create/delete-bucket/commit/rollback) and report every returned error
kind and cursor result; (4) `pages <file>`: per-page type/count/overflow. The Rust
test-suite runs the same scripts and compares dumps, error kinds, cursor traces and
`check` output, and alternates writers (Rust commit, Go commit, Rust commit …) on one
file.

---

## 12. Rust porting notes

### 12.1 Suggested crate and module boundary

| Crate | Mirrors | Contents |
| --- | --- | --- |
| `purse-bolt` | `go.etcd.io/bbolt` (subset) | File open/lock/init, page codec, meta, freelist (array), node/split/rebalance/spill, cursor, read and write transactions, commit, `copy`, `check`. No wallet types. |
| `purse-walletdb` | `wallet/walletdb` + `wallet/internal/bdb` + `wallet/db.go` | The transaction/bucket/cursor handle types the wallet uses, `view`/`update`, error-kind mapping (section 8.1), `open`/`create` with the existence pre-check. |
| daemon/loader crate | `internal/loader`, `walletsetup.go` | Paths, directory creation, create/open/unload sequencing and messages. |

The driver registry (`RegisterDriver`, `SupportedDrivers`, the `"bdb"` name) is
internal plumbing with a single driver and no external observer; it can be replaced
by direct calls provided the error kinds and texts of section 8 that can reach a
user are kept.

### 12.2 Reuse from existing code (only what was inspected)

- `dcroxide:crates/dcroxide-database` is backed by `redb`
  (`dcroxide:crates/dcroxide-database/Cargo.toml`, ADR-0004 in
  `dcroxide:docs/adr/0004-storage-backend.md`): it deliberately does not share dcrd's
  on-disk format and **cannot supply the engine** for `wallet.db`. Its
  `begin`/`view`/`update` shape (`dcroxide:crates/dcroxide-database/src/lib.rs:3329`,
  `:3364`, `:3374`) is a usable model for the handle API — with the difference that
  its `view`/`update` closures panic on a manual commit or rollback, which walletdb's
  do not (section 5.13) — and
  `create_dir_all_owner_only` (`dcroxide:crates/dcroxide-database/src/lib.rs:130`) is
  exactly the `MkdirAll(…, 0700)` behaviour needed by the loader.
- The dcroxide workspace forbids `unsafe` (`dcroxide:Cargo.toml:147`,
  `unsafe_code = "forbid"`) and has a pure-Rust rule
  (`dcroxide:docs/adr/0012-pure-rust.md`). If dcr-purse adopts the same lints, the
  engine cannot memory-map the file. That is acceptable: use positional reads and
  writes (`FileExt::read_at`/`write_at`, `seek_read`/`seek_write` on Windows) with an
  in-process cache of immutable pages. Copy-on-write guarantees a page visible to a
  snapshot is never rewritten while the snapshot lives, so cached pages need no
  invalidation other than on reuse of a page id by a later commit.
- `rustix` is already a dcroxide dependency
  (`dcroxide:crates/dcroxide-node/Cargo.toml:120`) and provides safe wrappers; whether
  its `fs` feature (for `flock`, `fdatasync`, `ftruncate`) is acceptable to the
  dependency ledger is a project decision. The Windows lock needs `LockFileEx` with
  an explicit `OVERLAPPED` offset, i.e. a `windows-sys` call or a safe wrapper crate.
- `dcr-rs` (`dcr-rs:src/`: address, airgap, amount, blake256, hashing, hd, network,
  sighash, sign, tx) has nothing for this layer.
- FNV-1a 64 is ten lines; no dependency needed.

### 12.3 Existing bbolt-compatible Rust crates

No such crate is present in the dcroxide or dcr-rs trees. From general knowledge
only, all **UNVERIFIED** (settle by reading each crate's source and running the
section 11.3 differential tests before any adoption):

- `bbolt-rs`: a Rust port of etcd's bbolt that states file compatibility as a goal.
  Maintenance status, platform coverage (Windows locking), and its use of `unsafe`
  and memory mapping are unverified.
- `nut`: an older port of BoltDB to Rust; believed unmaintained.
- `jammdb`: BoltDB-inspired, memory-mapped; believed **not** file-compatible.
- `redb`, `sled`, LMDB bindings: different formats; not applicable.

Given that this file holds the wallet's keys, that the workspace policy forbids
`unsafe`, and that the required subset is small and fully specified here, the
recommended plan is an in-house engine verified differentially against Go bbolt.

### 12.4 Hazards

1. **Freelist correctness is the main corruption risk.** A page wrongly listed as
   free is overwritten by the next Go (or Rust) commit. Run the `check` equivalent
   after every commit in debug and test builds.
2. **Crash consistency.** Order of writes and syncs (section 5.9.7–5.9.8); `sync_data`
   for the two commit syncs and `sync_all` after `set_len`. Test with fault injection
   (drop writes after the data sync; tear the meta page).
3. **Readers during writes.** Pending-free accounting must consider every live
   snapshot (section 5.11). In a design without mmap there is no remap stall, but page
   reuse rules are unchanged.
4. **Locking primitives.** `flock` (not POSIX `fcntl` record locks, not OFD locks) on
   Unix; the exact byte range on Windows. Waiting must be interruptible by process
   shutdown.
5. **Page size.** Never a constant; thresholds (`/2`, `/4`) derive from the file's
   page size. New files use the host page size.
6. **Lifetimes.** Returned keys/values borrow from the transaction; a write
   transaction's cursors must cope with nodes being modified (Q9). Decide the exact
   in-transaction cursor semantics before porting `wallet/udb`.
7. **Value aliasing and nil/empty** (Q6, Q8).
8. **Large values.** Overflow pages make a logical page span many physical pages;
   element offsets are `u32`; keep arithmetic in `u64`/`usize` with checks.
9. **Untrusted file contents.** bbolt trusts the file and panics or faults on
   garbage. The Rust reader must bounds-check every offset, count and page id and
   return an error (kind `IO`) instead.
10. **Windows file removal** semantics in the failed-creation path (Q14): close
    before remove, or reproduce the leftover file — a decision.
11. **Secrets in freed pages.** bbolt never scrubs freed pages or the unused tail of
    rewritten pages' predecessors; old ciphertext and public data linger in the file.
    Matching Go means not scrubbing either; scrubbing is allowed by the format.

### 12.5 Size estimate

**L.** The reachable bbolt subset is about 4,000 lines of Go (`db.go`, `tx.go`,
`bucket.go`, `node.go`, `cursor.go`, `freelist.go`, `page.go`) and the dcrwallet
layers above it about 800. The algorithms are compact and fully specified, but the
tolerance for error is zero, three platforms' locking and sync calls are involved,
and most of the effort is the oracle shim, differential fuzzing, crash-injection
tests and cross-implementation alternation tests. Expect roughly 4–6k lines of Rust
plus a test corpus of comparable size.

---

## 13. Open questions

1. **Engine: in-house or third-party?** Section 12.3; the named crates are
   UNVERIFIED. Recommended: in-house.
2. **`unsafe` policy for dcr-purse.** If inherited from dcroxide, no mmap and a
   decision on how the Windows `LockFileEx` call is made (audited exception versus a
   wrapper crate).
3. **Cursor semantics under mutation (Q9).** Reproduce bbolt's page-bound versus
   node-bound behaviour exactly, or define a cleaner rule and audit every
   iterate-and-mutate site in `wallet/udb` against it? Needs input from the
   `wallet/udb` spec owners.
4. **nil/empty values and `KeyN` (Q6, Q7).** Reproduce the Go quirks at the API or
   fix them and audit callers?
5. **Lock wait.** Reproduce the silent indefinite wait (strict parity) or add a log
   line / bounded wait? Any change is user-visible behaviour on a second instance.
6. **Spill order where Go uses map order.** Proposed: ascending bucket name. Confirms
   that byte-identical files are a non-goal.
7. **Corruption handling.** Which bbolt panics become errors of kind `IO`
   (section 8.3), and with what messages.
8. **Rust `std::fs::File::lock`.** On Windows it locks bytes `[0, 2^64 - 1)`
   (Rust 1.99.1 `library/std/src/sys/fs/windows.rs:409-424`), not bbolt's byte at
   `2^64 - 1`, so it cannot be used there (section 9.1 item 2); on Unix it is
   `flock(LOCK_EX)` (`library/std/src/sys/fs/unix.rs:1321-1343`). Re-check against
   the toolchain dcr-purse pins.
9. **Compatibility with dcrwallet releases older than the pin.** Which bbolt/boltdb
   versions those link was not checked (only v1.3.12 was read). The claim in
   section 9.1 item 7 that older versions cannot read `pgidNoFreelist` is UNVERIFIED;
   it is moot if the freelist is always synced, as recommended.
10. **macOS and Windows sync calls.** Go's `File.Sync` semantics on those platforms
    (`F_FULLFSYNC`, `FlushFileBuffers`) are stated from knowledge of the Go standard
    library, not from the sources read for this spec; confirm against the Go version
    used to build the reference binary and against Rust's `sync_data`/`sync_all`.
11. **Failed-creation cleanup (Q14) and `MixSplitLimit` (Q15):** reproduce or fix?
12. **Read-only tooling.** dcrwallet never opens read-only; should dcr-purse offer a
    read-only open (shared lock, no freelist load) for diagnostics? Not required for
    parity.
13. **`DeleteBucket` page leak (Q29).** Derived from the source, not run. Settle by
    an oracle script: create a regular bucket with several regular nested buckets in
    one leaf, commit; then, in one transaction, `Put` a key into the outer bucket
    and call `DeleteNestedBucket` on the outer bucket; compare `Tx.Check` output (`unreachable unfreed`) and decide whether
    the port reproduces the leak or frees the pages.
