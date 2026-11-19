# 22 — Auxiliary packages and command-line tools

## 1. Header

**Scope** (dcrwallet, line counts from `wc -l`):

| File | Lines | Covered here |
|---|---|---|
| `pgpwordlist/doc.go` | 5 | all |
| `pgpwordlist/pgpwordlist.go` | 60 | all |
| `pgpwordlist/wordlist.go` | 544 | all (512-word table reproduced in section 4.1) |
| `pgpwordlist/pgpwordlist_test.go` | 66 | tests |
| `walletseed/seed.go` | 102 | all |
| `walletseed/seed_test.go` | 88 | tests |
| `errors/errors.go` | 400 | all |
| `errors/errors_test.go` | 81 | tests |
| `kdf/kdf.go` | 71 | all (API and byte layout; the caller is the address manager, spec 02) |
| `lru/cache.go` | 67 | all |
| `lru/map.go` | 101 | all |
| `validate/validate.go` | 130 | inventory only (details belong to the sync specs 09/14/15) |
| `version/version.go` | 113 | inventory only (startup/RPC use belongs to specs 16 and the daemon spec) |
| `cmd/movefunds/main.go`, `config.json` | 239, 6 | all |
| `cmd/repaircfilters/main.go`, `README.md` | 250, 36 | all |
| `cmd/sweepaccount/main.go` | 493 | all |
| `cmd/treasurykey/treasurykey.go`, `README.md` | 69, 50 | all |
| `rpc/client/dcrwallet/client.go` | 63 | all |
| `rpc/client/dcrwallet/marshaling.go` | 234 | all |
| `rpc/client/dcrwallet/methods.go` | 982 | all |
| `rpc/client/dcrwallet/marshaling_test.go`, `methods_test.go` | 61, 129 | tests |
| `rpc/rpc_example/main.go` | 123 | all |
| `assets/assets.go` | 25 | all |
| `.github/workflows/go.yml` | 24 | all |
| `run_tests.sh` | 9 | all |
| `README.md`, `SECURITY.md` | 176, 14 | support policy |

Supporting code read to pin observable behaviour: `go.mod`, `flake.nix`,
`.gitignore`, `debug.go`, `docs/offline_wallets.md`, `dcrwallet.go:36-39`
(`errors.Separator` override), `internal/rpc/jsonrpc/errors.go` (kind to
JSON-RPC code), `internal/rpc/rpcserver/server.go:80-136, 3177-3205` (kind to
gRPC code, seed service), `internal/prompt/prompt.go:257-365`,
`walletsetup.go:270-290`, `wallet/txauthor/author.go:1-166`,
`wallet/txsizes/size.go:120-209`, `wallet/txrules/rules.go:37-106`,
`wallet/udb/addressmanager.go:140-158`, `wallet/udb/addressdb.go:1058-1106`,
`internal/netparams/params.go`, `rpc/jsonrpc/types/{methods,results}.go`
(types the tools and the typed client use).

**Parity pin**: dcrwallet `54b85b33` (master, version 2.2.0-pre, module
`decred.org/dcrwallet/v5`). Dependency versions read (all as pinned by
`go.mod`):

| Module | Version | Read for |
|---|---|---|
| `github.com/decred/dcrd/hdkeychain/v3` | v3.1.3 | seed length limits, `ErrInvalidSeedLen`, `GenerateSeed` |
| `github.com/decred/dcrd/dcrutil/v4` | v4.0.3 | `Amount`, `NewAmount`, `AppDataDir`, `WIF` |
| `github.com/decred/dcrd/dcrec` | v1.0.1 | signature type constants |
| `github.com/decred/dcrd/chaincfg/v3` | v3.3.0 | `PrivateKeyID`, `MaxTxSize` |
| `github.com/decred/dcrd/chaincfg/chainhash` | v1.0.5 | `HashB` |
| `github.com/decred/dcrd/wire` | v1.8.0 | `MsgTx`, `OutPoint.String` |
| `github.com/decred/dcrd/rpc/jsonrpc/types/v4` | v4.4.0 | `TransactionInput` |
| `github.com/decred/dcrd/rpcclient/v8` | v8.1.0 | `RawRequest` (rpc_example) |
| `github.com/jrick/wsrpc/v2` | v2.3.8 | websocket JSON-RPC client used by the tools (read in full) |
| `github.com/gorilla/websocket` | v1.5.1 | URL scheme check in `DialContext` |
| `github.com/jessevdk/go-flags` | v1.5.0 | `Parse` defaults and error printing |
| `golang.org/x/crypto` | v0.57.0 | `argon2.IDKey` |
| Go standard library | go1.27.4 | `errors.Is`/`errors.As`, `strings.ToLower`, `strings.TrimSpace`, `encoding/hex` error texts |

**Summary**

- `errors` is the wallet-wide error model: an `Error{Op, Kind, Err}` chain,
  21 `Kind` values with fixed strings, an `E()` constructor that promotes and
  collapses nested errors, and template matching. Its rendered strings are the
  `message` of JSON-RPC errors and gRPC statuses, so the 21 kind strings, the
  `Op` strings spread over the code base (154 constant declarations, 38
  format constants, 4 inline) and the separator (`":: "` in the daemon) are
  wire-visible.
- `pgpwordlist` and `walletseed` define the seed mnemonic: the 512-word PGP
  alternating word list, one word per byte with even/odd position parity, and
  one extra checksum word (first byte of double SHA-256).
- `kdf` is Argon2id (time 1, memory 256 MiB, threads = CPU count of the
  creating machine, stored in one byte) with a 25-byte persisted parameter
  record used for individually encrypted accounts.
- `lru` is a small mutex-guarded recency cache whose effective capacity is
  `limit+1`; it is not dcrd's `container/lru`.
- `validate` and `version` are listed for completeness.
- The four `cmd/` tools are not part of the daemon. At the pin `treasurykey`
  and `repaircfilters` work; `movefunds` panics before producing output and
  `sweepaccount` cannot connect (missing URL scheme) and would send a
  malformed `signrawtransaction` if it could.
- `rpc/client/dcrwallet` is a Go convenience client over JSON-RPC. It is
  not wire surface of the daemon, but it documents what request shapes
  existing Go clients send; four of its methods call two method names
  (`listreceivedbyacccount`, `addticket`) that the server does not register.
- CI runs `go build ./...` and `go test -short -vet=all ./...` on Go 1.26
  and 1.27, Linux only.

## 2. Role in the system

| Package | What it does | Called by | Calls |
|---|---|---|---|
| `errors` | Error construction, classification, formatting, matching | every package in the module (1,050 `errors.E` call sites outside tests) | Go `errors`, `fmt`, `runtime/debug` |
| `pgpwordlist` | Byte to word and word to byte mapping | `walletseed` only | `errors` |
| `walletseed` | Seed generation, mnemonic encode with checksum, user-input decode | `internal/prompt/prompt.go:276,350` (interactive `--create`), `walletsetup.go:287` (simnet seed file), gRPC `SeedService` (`internal/rpc/rpcserver/server.go:3194,3200`), `wallet/udb/testdata/*.go` generators | `pgpwordlist`, `hdkeychain`, SHA-256 |
| `kdf` | Argon2id parameters, marshal/unmarshal, key derivation | `wallet/udb/addressmanager.go:156,1745,2196`, `wallet/udb/addressdb.go:1058,1095` | `x/crypto/argon2` |
| `lru` | Recency sets and maps | `p2p/peering.go:126-127,563-564` (per-peer sent/received inventory, `lru.Cache`), `spv/sync.go:87-88,148-149` (seen transactions and mix messages, limit 2000). `lru.Map` has no caller at the pin. | `container/list`, `sync` |
| `validate` | Context-free merkle root, cfilter commitment and pre-DCP0005 cfilter-set checks | `spv/backend.go:82-84,235`, `spv/sync.go:1513`, `chain/sync.go:263,866`, `wallet/wallet.go:1381,5238-5263` | dcrd `blockchain/standalone`, `gcs`, `chainhash` |
| `version` | Version constants and semver string | `dcrwallet.go:89`, `config.go:412`, `p2p/peering.go:41`, `internal/rpc/jsonrpc/methods.go:1792-1794,5449-5454` | `runtime/debug` |
| `assets` | Source-tree relative path helper | no caller at the pin | `runtime`, `path/filepath` |
| `rpc/client/dcrwallet` | Typed JSON-RPC client for third-party Go programs | `rpc/rpc_example` only | a `Caller` supplied by the user |
| `cmd/*` | Stand-alone operator tools | humans | wallet JSON-RPC over websocket (`sweepaccount`, `repaircfilters`), local files (`movefunds`), nothing (`treasurykey`) |

None of the packages in this scope touches `wallet.db` directly. `kdf`
defines bytes that the address manager stores (section 4.3).

## 3. Public surface inventory

### 3.1 `pgpwordlist`

| Symbol | Signature | Contract |
|---|---|---|
| `ByteToMnemonic` | `func(b byte, index int) string` (`pgpwordlist/pgpwordlist.go:26-32`) | Returns `wordList[2*b + (index%2 != 0 ? 1 : 0)]`. `index` is the byte position in the sequence. Go `%` keeps the sign, so a negative odd index also selects the odd word. Never fails. |
| `DecodeMnemonics` | `func(words []string) ([]byte, error)` (`pgpwordlist/pgpwordlist.go:36-60`) | Decodes words to bytes, skipping words that are empty after whitespace trimming. Section 5.1. |
| `wordList` (unexported) | `[]string`, 512 entries (`pgpwordlist/wordlist.go:23`) | `strings.Split(alternatingWords, "\n")`. |
| `wordIndexes` (unexported) | `map[string]uint16` (`pgpwordlist/wordlist.go:25-31`) | Lower-cased word to list index, built in `init`. |
| `alternatingWords` (unexported) | string constant (`pgpwordlist/wordlist.go:33-544`) | The word list, newline separated, no trailing newline. |

### 3.2 `walletseed`

| Symbol | Signature | Contract |
|---|---|---|
| `GenerateRandomSeed` | `func(size uint) ([]byte, error)` (`walletseed/seed.go:21-34`) | Returns `size` random bytes; `size` must be in 16..64. Section 5.2. |
| `EncodeMnemonicSlice` | `func(seed []byte) []string` (`walletseed/seed.go:44-52`) | `len(seed)+1` words: one per byte, then the checksum word. No length validation. |
| `EncodeMnemonic` | `func(seed []byte) string` (`walletseed/seed.go:55-67`) | Same words joined with single spaces. For an empty seed returns `" "` followed by the checksum word (leading space, see quirks). |
| `DecodeUserInput` | `func(input string) ([]byte, error)` (`walletseed/seed.go:71-102`) | Decodes hex (one token) or mnemonic (two or more space-separated tokens) and enforces 16..64 bytes. Section 5.3. |
| `checksumByte` (unexported) | `func(data []byte) byte` (`walletseed/seed.go:38-41`) | `SHA256(SHA256(data))[0]`. |

### 3.3 `errors`

| Symbol | Signature | Contract |
|---|---|---|
| `Separator` | `var Separator = ":\n\t"` (`errors/errors.go:27`) | Inserted between nested `*Error` levels by `(*Error).Error`. The daemon sets it to `":: "` in `init` (`dcrwallet.go:36-39`). Process-global, not synchronised. |
| `Error` | `struct{ Op Op; Kind Kind; Err error; stack []byte }` (`errors/errors.go:32-38`) | One level of an error chain. All exported fields may be zero. |
| `Op` | `type Op string` (`errors/errors.go:42`) | Name of the operation, conventionally `"package.Function"` or `"package.Type.Method"`, sometimes with arguments. |
| `Opf` | `func(format string, a ...any) Op` (`errors/errors.go:45-47`) | `Op(fmt.Sprintf(format, a...))`. |
| `Kind` | `type Kind int` (`errors/errors.go:50`) | Error class; implements `error`. Values in section 4.2. |
| `Other` .. `Deployment` | 21 constants, values 0..20 (`errors/errors.go:53-75`) | Section 4.2. |
| `Kind.String` | `func (k Kind) String() string` (`errors/errors.go:77-124`) | Fixed string per kind; `"unknown error kind"` for any value outside 0..20. |
| `Kind.Error` | `func (k Kind) Error() string` (`errors/errors.go:126-128`) | Same as `String`. |
| `Kind.As` | `func (k Kind) As(target any) bool` (`errors/errors.go:136-149`) | `false` for `Other`. For `**Error` target assigns `&Error{Kind: k}`; for `*Kind` target assigns `k`; else `false`. |
| `New` | `func(text string) error` (`errors/errors.go:153-155`) | Standard library `errors.New`. |
| `Errorf` | `func(format string, args ...any) error` (`errors/errors.go:159-161`) | `fmt.Errorf` (so `%w` wrapping works). |
| `E` | `func(args ...any) error` (`errors/errors.go:187-236`) | Builds an `*Error`. Section 5.4. Panics with `"errors.E: no args"` when called with no arguments. |
| `WithStack` | `func(args ...any) error` (`errors/errors.go:241-245`) | `E(args...)` plus `debug.Stack()` stored in the unexported `stack` field. No caller at the pin. |
| `(*Error).Error` | `func (e *Error) Error() string` (`errors/errors.go:247-290`) | Formatting. Section 5.5. |
| `(*Error).Unwrap` | `func (e *Error) Unwrap() error` (`errors/errors.go:295-303`) | `e.Err` if non-nil; else `e.Kind` (as an `error`) if not `Other`; else nil. |
| `(*Error).As` | `func (e *Error) As(target any) bool` (`errors/errors.go:311-323`) | `**Error` target: assign `e`, true. `*Kind` target: assign and return true only if `e.Kind != Other`. |
| `(*Error).Is` | `func (e *Error) Is(target error) bool` (`errors/errors.go:330-338`) | `*Error` target: `match(target, e)`. `Kind` target: `e.Kind != Other && e.Kind == target`. Otherwise false. |
| `Is` | `func(err, target error) bool` (`errors/errors.go:341-343`) | Standard library `errors.Is`. |
| `As` | `func(err error, target any) bool` (`errors/errors.go:348-350`) | Standard library `errors.As`. |
| `Join` | `func(errs ...error) error` (`errors/errors.go:353-355`) | Standard library `errors.Join`. One caller: `wallet/wallet.go:5746`. |
| `Stacks` | `func(err error) [][]byte` (`errors/errors.go:390-400`) | Collects `stack` from each `*Error` reached by following `Err` while it is an `*Error`, outermost first. No caller at the pin. |
| `match` (unexported) | `func(err1, err2 error) bool` (`errors/errors.go:357-386`) | Template match. Section 5.6. |

### 3.4 `kdf`

| Symbol | Signature | Contract |
|---|---|---|
| `Argon2idParams` | `struct{ Salt [16]byte; Time uint32; Memory uint32; Threads uint8 }` (`kdf/kdf.go:18-23`) | Argon2id cost parameters and salt. `Memory` is in KiB. |
| `NewArgon2idParams` | `func(rand io.Reader) (*Argon2idParams, error)` (`kdf/kdf.go:30-39`) | `Time=1`, `Memory=262144`, `Threads=uint8(min(NumCPU, 256))`, 16-byte salt read with a single `rand.Read` call. Returns the params together with the read error (params non-nil even on error). |
| `MarshaledLen` | `const = 25` (`kdf/kdf.go:42`) | Length of the marshaled form. |
| `(*Argon2idParams).MarshalBinary` | `func() ([]byte, error)` (`kdf/kdf.go:46-53`) | 25 bytes, section 4.3. Never errors. |
| `(*Argon2idParams).UnmarshalBinary` | `func(data []byte) error` (`kdf/kdf.go:56-65`) | Requires exactly 25 bytes; otherwise the plain error `invalid marshaled Argon2id parameters` (no `Op`, no `Kind`). No range validation of the fields. |
| `DeriveKey` | `func(password []byte, p *Argon2idParams, len uint32) []byte` (`kdf/kdf.go:68-71`) | `argon2.IDKey(password, p.Salt[:], p.Time, p.Memory, p.Threads, len)`, then forces a Go garbage collection on return. Panics (inside `x/crypto`) if `Time < 1` or `Threads < 1` (`x/crypto@v0.57.0/argon2/argon2.go:106-111`). |

### 3.5 `lru`

| Symbol | Signature | Contract |
|---|---|---|
| `Cache[T comparable]` | struct (`lru/cache.go:17-22`) | Recency set. Zero value invalid. All methods take the internal mutex. |
| `NewCache[T]` | `func(limit int) Cache[T]` (`lru/cache.go:25-31`) | Returns the struct by value (contains a mutex; callers store it in a field and use it by pointer). |
| `(*Cache[T]).Add` | `func(item T)` (`lru/cache.go:36-59`) | Section 5.7. |
| `(*Cache[T]).Contains` | `func(v T) bool` (`lru/cache.go:62-67`) | Membership test; does not change recency. |
| `Map[K comparable, V any]` | struct (`lru/map.go:22-27`) | Recency map. Zero value invalid. |
| `NewMap[K, V]` | `func(limit int) Map[K, V]` (`lru/map.go:30-36`) | As `NewCache`. |
| `(*Map[K, V]).Add` | `func(key K, value V)` (`lru/map.go:41-65`) | Section 5.7. An existing key keeps its old value. |
| `(*Map[K, V]).Get` | `func(key K) (V, bool)` (`lru/map.go:70-82`) | Returns the value and marks it most recent; zero value and false when absent. |
| `(*Map[K, V]).Hit` | `func(key K) bool` (`lru/map.go:89-92`) | `Get` discarding the value. |
| `(*Map[K, V]).Contains` | `func(key K) bool` (`lru/map.go:96-101`) | Membership test; does not change recency (the doc comment says "It does modify", the code does not). |

### 3.6 `validate` (inventory only)

| Symbol | Signature | Contract |
|---|---|---|
| `DCP0005ActiveHeightTestNet3` | `const int32 = 323328` (`validate/validate.go:21`) | First testnet3 height with DCP0005 rules. |
| `DCP0005ActiveHeightMainNet` | `const int32 = 431488` (`validate/validate.go:25`) | First mainnet height with DCP0005 rules. |
| `MerkleRoots` | `func(block *wire.MsgBlock) error` (`validate/validate.go:31-48`) | Pre-DCP0005 check: `Header.MerkleRoot` equals the regular tree root and `Header.StakeRoot` equals the stake tree root. Errors: `Op` `validate.MerkleRoots(<block hash>)`, kind `Consensus`, text `invalid regular merkle root` or `invalid stake merkle root`. |
| `DCP0005MerkleRoot` | `func(block *wire.MsgBlock) error` (`validate/validate.go:56-67`) | `Header.MerkleRoot` equals the combined root. Error: `Op` `validate.DCP0005MerkleRoot(<block hash>)`, kind `Consensus`, text `invalid combined merkle root`. |
| `CFilterV2HeaderCommitment` | `func(net wire.CurrencyNet, header *wire.BlockHeader, filter *gcs.FilterV2, leafIndex uint32, proof []chainhash.Hash) error` (`validate/validate.go:71-103`) | Returns nil without checking for testnet3 headers below 323328 and mainnet headers below 431488. On every other network and height verifies the inclusion proof of `filter.Hash()` against `header.StakeRoot`. Error: `Op` `validate.CFilterV2HeaderCommitment(<block hash>)`, kind `Consensus`, text `invalid header inclusion proof for cfilterv2`. |
| `PreDCP0005CFilterHash` | `func(net wire.CurrencyNet, cfsethash *chainhash.Hash) error` (`validate/validate.go:107-130`) | Compares against the hard-coded target (testnet3 `619c08f5adda6f834212bbdaee3002fdc4efed731477af6c0fed490bbe2488d0`, mainnet `f95e09f9ded38f8d6c32e5158a1f286633881393659218c63f5ab0fc86b36c83`, both in `chainhash` string form, i.e. byte-reversed). Errors are plain (no `Op`/`Kind`): `unknown network <decimal net magic>` and `hash of provided cfilters <got> does not match expected hash <want>`. |

### 3.7 `version` (inventory only)

| Symbol | Value / signature | Contract |
|---|---|---|
| `Major`, `Minor`, `Patch` | `2`, `2`, `0` (`version/version.go:21-23`) | Application version. |
| `Integer` | `1000000*Major + 10000*Minor + 100*Patch` = `2020000` (`version/version.go:27`) | Fills the `Version` and `WalletVersion` fields of the JSON-RPC `getinfo` result (`internal/rpc/jsonrpc/methods.go:1792-1794`). |
| `PreRelease` | `var = "pre"` (`version/version.go:33`) | Link-time overridable (`-X decred.org/dcrwallet/v5/version.PreRelease=...`). |
| `BuildMetadata` | `var = ""` (`version/version.go:38`) | Link-time overridable. If empty at `init`, set to `vcsCommitID()`. |
| `String` | `func() string` (`version/version.go:71-94`) | `"<Major>.<Minor>.<Patch>"`, then `"-" + normalized PreRelease` if non-empty, then `"+" + normalized BuildMetadata` if non-empty. The emptiness test is made on the string **after** normalization, so a value consisting only of characters outside the alphabet is omitted together with its `-` or `+`. `Major`, `Minor`, `Patch` and `Integer` are untyped constants. At the pin without VCS info: `2.2.0-pre`. |
| `vcsCommitID` (unexported) | `func() string` (`version/version.go:46-67`) | From Go build info: empty if no build info or no `vcs` setting; otherwise `vcs.revision`, truncated to the first 9 characters when `vcs == "git"` and the revision is longer than 9. `vcs.modified` is not consulted. |
| `normalizeVerString` (unexported) | `func(str string) string` (`version/version.go:100-113`) | Drops every character not in `0-9A-Za-z-.` (`semverAlphabet`, `version/version.go:17`). |

### 3.8 `assets`

| Symbol | Signature | Contract |
|---|---|---|
| `Path` | `func(asset string) string` (`assets/assets.go:23-25`) | `filepath.Join(<directory of assets.go at compile time>, asset)`. Works only on the build host without `-trimpath`. No caller in the module at the pin. |

### 3.9 `rpc/client/dcrwallet`

| Symbol | Signature | Contract |
|---|---|---|
| `Caller` | `interface{ Call(ctx, method string, res any, args ...any) error }` (`rpc/client/dcrwallet/client.go:11-18`) | Positional-parameter JSON-RPC call; `res` is a pointer to decode into, or nil. `*wsrpc.Client` satisfies it. |
| `RawRequester` | `interface{ RawRequest(ctx, method string, params []json.RawMessage) (json.RawMessage, error) }` (`rpc/client/dcrwallet/client.go:22-24`) | Satisfied by dcrd `rpcclient.Client`. |
| `RawRequestCaller` | `func(req RawRequester) Caller` (`rpc/client/dcrwallet/client.go:27-52`) | Adapter: `json.Marshal` each argument, call `RawRequest`, `json.Unmarshal` the result when `res != nil`. |
| `Client` | `struct{ Caller; net *chaincfg.Params }` (`rpc/client/dcrwallet/client.go:55-58`) | Embeds the caller, so `Call` is also available for untyped use. |
| `NewClient` | `func(caller Caller, net *chaincfg.Params) *Client` (`rpc/client/dcrwallet/client.go:61-63`) | |
| `GapPolicy`, `GapPolicyError`, `GapPolicyIgnore`, `GapPolicyWrap` | `type GapPolicy string`; `"error"`, `"ignore"`, `"wrap"` (`rpc/client/dcrwallet/methods.go:314-322`) | Values for the second `getnewaddress` parameter. |
| `SigHashType` and six constants | `"ALL"`, `"NONE"`, `"SINGLE"`, `"ALL\|ANYONECANPAY"`, `"NONE\|ANYONECANPAY"`, `"SINGLE\|ANYONECANPAY"` (the pipe characters are literal) (`rpc/client/dcrwallet/methods.go:770-809`) | Values for the `signrawtransaction` flags parameter. `String()` returns the underlying string. |
| `SignatureErrors` | `type SignatureErrors []types.SignRawTransactionError` with `Error() string` (`rpc/client/dcrwallet/methods.go:813-834`) | Empty: `<nil>`. One element: that element's `Error` field. More: `multiple signature errors: ["e1"; "e2"; ...]`. |
| 79 `(*Client)` methods | section 4.8 | One JSON-RPC call each. |
| unexported marshal helpers | `rpc/client/dcrwallet/marshaling.go:18-234` | Section 4.8. |

### 3.10 Command-line programs

| Program | Entry | Summary |
|---|---|---|
| `movefunds` | `cmd/movefunds/main.go:122` | Reads `unspent.json` and `config.json` from the working directory, writes `sign.sh`. Section 5.8. |
| `repaircfilters` | `cmd/repaircfilters/main.go:94,245` | Feeds a binary file of pre-DCP0005 cfilters to the wallet with `importcfiltersv2`. Section 5.9. |
| `sweepaccount` | `cmd/sweepaccount/main.go:112,287` | Sweeps outputs of an account or address to an account or address through the wallet JSON-RPC server. Section 5.10. |
| `treasurykey` | `cmd/treasurykey/treasurykey.go:34` | Prints a fresh secp256k1 private key, its compressed public key and a Schnorr-typed WIF. Section 5.11. |
| `rpc_example` | `rpc/rpc_example/main.go:29` | Demonstration program for the typed client. Section 5.12. |

### 3.11 Unexported symbols not listed above

Every remaining declaration in scope, so that nothing is implicit. None is
reachable from outside its package; the "observable" column says where its
behaviour shows up.

| Package | Symbol | Declaration | Observable through |
|---|---|---|---|
| `pgpwordlist` | `init` | `pgpwordlist/wordlist.go:27-31` | Fills `wordIndexes` with `ToLower(word) -> index` for all 512 words. |
| `lru` | `mapListValue[K, V]` | `struct{ k K; v V }` (`lru/map.go:12-15`) | List element payload of `Map`; a pointer to it is stored in the list (`lru/map.go:56-64,77`). |
| `lru` | `Cache.mu/m/list/limit`, `Map.mu/m/list/limit` | `lru/cache.go:18-21`, `lru/map.go:23-26` | The Go map is pre-sized with `limit` (allocation hint only). |
| `version` | `init` | `version/version.go:40-44` | Sets `BuildMetadata = vcsCommitID()` only when the link-time value is empty. |
| `version` | `semverAlphabet` | `version/version.go:17` | Section 3.7. |
| `assets` | `basepath`, `init` | `assets/assets.go:8-13` | `basepath = Dir(file name reported by runtime.Caller(0))`. |
| `errors` | `Error.stack` | `[]byte` (`errors/errors.go:37`) | Only through `Stacks`; never rendered, never compared by `match`. |
| `rpc/client/dcrwallet` | `rawRequester`, `(*rawRequester).Call` | `rpc/client/dcrwallet/client.go:31-52` | The adapter returned by `RawRequestCaller`. A `json.Marshal` failure of any argument is returned before any request is sent; a `RawRequest` error is returned unchanged; with `res == nil` the response body is not parsed at all. |
| `rpc/client/dcrwallet` | `marshalJSONFunc`, `unmarshalJSONFunc` | function types implementing `json.Marshaler` (value receiver) and `json.Unmarshaler` (pointer receiver) (`rpc/client/dcrwallet/marshaling.go:18-22`) | Carrier for the closures below. |
| `rpc/client/dcrwallet` | `addressArrayMarshaler(n, s)`, `marshalAddresses`, `marshalTx`, `marshalOutpointsToString` | `marshaling.go:24-73` | Request encodings, section 4.8. |
| `rpc/client/dcrwallet` | `unmarshalOutpoints`, `unmarshalHash`, `unmarshalHashes`, `unmarshalAddress`, `unmarshalAddresses`, `unmarshalListAccounts`, `unmarshalHDKey`, `unmarshalAmount`, `unmarshalWIF`, `unmarshalTx` | `marshaling.go:75-234` | Result decodings and their failure modes, section 4.8. |
| `cmd/movefunds` | `params` (global, assigned in `main`), `configJSON`, `saneOutputValue`, `parseOutPoint`, `errNoInputValue`, `makeInputSource`, `main` | `cmd/movefunds/main.go:38-239` | Section 5.8. `makeInputSource` returns the input total as well as the source. |
| `cmd/sweepaccount` | `activeNet`, `walletDataDirectory`, `newlineBytes`, `fatalf`, `errContext`, `opts`, `normalizeAddress`, `walletPort`, `init`, `errNoInputValue`, `makeInputSource`, `destinationScriptSourceToAccount` (`Script`, `ScriptSize`), `destinationScriptSourceToAddress` (`Script`, `ScriptSize`), `main`, `sweep`, `promptSecret`, `saneOutputValue`, `parseOutPoint`, `pickNoun` | `cmd/sweepaccount/main.go:31-493` | Section 5.10. `errContext(err, ctx)` is `fmt.Errorf("%s: %v", ctx, err)` (no `%w`, so the cause is not unwrappable). `pickNoun(n, s, p)` returns `s` only for `n == 1` (so 0 and negative counts are plural). `walletPort` returns the empty string for any network other than mainnet, testnet3 and simnet. |
| `cmd/repaircfilters` | `walletDataDirectory`, `newlineBytes`, `opts`, `fatalf`, `errContext`, `normalizeAddress`, `walletPort`, `init`, `promptSecret`, `repair`, `main` | `cmd/repaircfilters/main.go:30-250` | Section 5.9. `fatalf`, `errContext`, `normalizeAddress`, `walletPort` and `promptSecret` are code-identical copies of the `sweepaccount` helpers (only comment wrapping differs). |
| `cmd/treasurykey` | `generateKeys(params)`, `main` | `cmd/treasurykey/treasurykey.go:14-69` | Section 5.11. |
| `rpc/rpc_example` | `host`, `user`, `pass`, `certFile`, `params`, `main`, `connectWallet`, `run`, `importScripts`, `validateAddrs` | `rpc/rpc_example/main.go:18-123` | Section 5.12. |

Both tools' `fatalf(format, args...)` write `Sprintf(format, args...)` and
then a single `\n` to standard error and call `os.Exit(1)`
(`cmd/sweepaccount/main.go:37-41`, `cmd/repaircfilters/main.go:50-54`).
`promptSecret(what)` prints `<what>: ` (no newline) to standard output,
reads one line without echo from file descriptor 0 with
`term.ReadPassword`, prints a newline to standard output whether or not the
read failed, and returns the bytes read as a string with no trimming
(`cmd/sweepaccount/main.go:465-474`, `cmd/repaircfilters/main.go:135-144`).
When standard input is not a terminal the read fails and the tool exits
with `failed to read RPC password: <err>`; there is no way to pipe the
password in other than `-P`.

## 4. Data formats

### 4.1 Seed mnemonic (PGP word list with checksum)

**Word list.** 512 words, index 0..511 (`pgpwordlist/wordlist.go:33-544`).
Index `2*b` is the word for byte value `b` at an even position, index
`2*b+1` the word for `b` at an odd position. Fifty words start with an
upper-case letter in the canonical spelling; output always uses the canonical
spelling. All words are ASCII letters only, 4 to 11 characters, and are
unique after lower-casing. The list below is in index order, eight per line
(row `r`, column `c` is index `8r+c`). SHA-256 of the 512 words joined with
`\n` (no trailing newline, exactly the Go string constant) is
`55c9067321ccd946013bfda1ba480ffa09d3b97e119eddc72ba9598a28ef70d3`
(computed from the constant while writing this spec; use it as a
transcription check).

```
aardvark adroitness absurd adviser accrue aftermath acme aggregate
adrift alkali adult almighty afflict amulet ahead amusement
aimless antenna Algol applicant allow Apollo alone armistice
ammo article ancient asteroid apple Atlantic artist atmosphere
assume autopsy Athens Babylon atlas backwater Aztec barbecue
baboon belowground backfield bifocals backward bodyguard banjo bookseller
beaming borderline bedlamp bottomless beehive Bradbury beeswax bravado
befriend Brazilian Belfast breakaway berserk Burlington billiard businessman
bison butterfat blackjack Camelot blockade candidate blowtorch cannonball
bluebird Capricorn bombast caravan bookshelf caretaker brackish celebrate
breadline cellulose breakup certify brickyard chambermaid briefcase Cherokee
Burbank Chicago button clergyman buzzard coherence cement combustion
chairlift commando chatter company checkup component chisel concurrent
choking confidence chopper conformist Christmas congregate clamshell consensus
classic consulting classroom corporate cleanup corrosion clockwork councilman
cobra crossover commence crucifix concert cumbersome cowbell customer
crackdown Dakota cranky decadence crowfoot December crucial decimal
crumpled designing crusade detector cubic detergent dashboard determine
deadbolt dictator deckhand dinosaur dogsled direction dragnet disable
drainage disbelief dreadful disruptive drifter distortion dropper document
drumbeat embezzle drunken enchanting Dupont enrollment dwelling enterprise
eating equation edict equipment egghead escapade eightball Eskimo
endorse everyday endow examine enlist existence erase exodus
escape fascinate exceed filament eyeglass finicky eyetooth forever
facial fortitude fallout frequency flagpole gadgetry flatfoot Galveston
flytrap getaway fracture glossary framework gossamer freedom graduate
frighten gravity gazelle guitarist Geiger hamburger glitter Hamilton
glucose handiwork goggles hazardous goldfish headwaters gremlin hemisphere
guidance hesitate hamlet hideaway highchair holiness hockey hurricane
indoors hydraulic indulge impartial inverse impetus involve inception
island indigo jawbone inertia keyboard infancy kickoff inferno
kiwi informant klaxon insincere locale insurgent lockup integrate
merit intention minnow inventive miser Istanbul Mohawk Jamaica
mural Jupiter music leprosy necklace letterhead Neptune liberty
newborn maritime nightbird matchmaker Oakland maverick obtuse Medusa
offload megaton optic microscope orca microwave payday midsummer
peachy millionaire pheasant miracle physique misnomer playhouse molasses
Pluto molecule preclude Montana prefer monument preshrunk mosquito
printer narrative prowler nebula pupil newsletter puppy Norwegian
python October quadrant Ohio quiver onlooker quota opulent
ragtime Orlando ratchet outfielder rebirth Pacific reform pandemic
regain Pandora reindeer paperweight rematch paragon repay paragraph
retouch paramount revenge passenger reward pedigree rhythm Pegasus
ribcage penetrate ringbolt perceptive robust performance rocker pharmacy
ruffled phonetic sailboat photograph sawdust pioneer scallion pocketful
scenic politeness scorecard positive Scotland potato seabird processor
select provincial sentence proximate shadow puberty shamrock publisher
showgirl pyramid skullcap quantity skydive racketeer slingshot rebellion
slowdown recipe snapline recover snapshot repellent snowcap replica
snowslide reproduce solo resistor southward responsive soybean retraction
spaniel retrieval spearhead retrospect spellbind revenue spheroid revival
spigot revolver spindle sandalwood spyglass sardonic stagehand Saturday
stagnate savagery stairway scavenger standard sensation stapler sociable
steamship souvenir sterling specialist stockman speculate stopwatch stethoscope
stormy stupendous sugar supportive surmount surrender suspense suspicious
sweatband sympathy swelter tambourine tactics telephone talon therapist
tapeworm tobacco tempest tolerance tiger tomorrow tissue torpedo
tonic tradition topmost travesty tracker trombonist transit truncated
trauma typewriter treadmill ultimate Trojan undaunted trouble underfoot
tumor unicorn tunnel unify tycoon universe uncut unravel
unearth upcoming unwind vacancy uproot vagabond upset vertigo
upshot Virginia vapor visitor village vocalist virus voyager
Vulcan warranty waffle Waterloo wallet whimsical watchword Wichita
wayside Wilmington willow Wyoming woodlark yesteryear Zulu Yucatan
```

**Encoding** (`walletseed/seed.go:44-67`). For a seed `s` of `n` bytes:

| Position | Word |
|---|---|
| `i` in `0..n-1` | `wordList[2*s[i] + (i mod 2)]` |
| `n` (checksum) | `wordList[2*c + (n mod 2)]` where `c = SHA256(SHA256(s))[0]` |

The string form joins the `n+1` words with a single ASCII space, no leading
or trailing space (for `n >= 1`) and no newline. A 32-byte seed gives 33
words; a 16-byte seed 17; a 64-byte seed 65.

**Decoding** accepts either form (section 5.3):

- hexadecimal: one token, upper or lower case digits, even length, no `0x`
  prefix;
- mnemonic: two or more tokens separated by single spaces (runs of spaces are
  tolerated because empty tokens are skipped), matched case-insensitively, the
  last word being the checksum.

Test vectors (from `walletseed/seed_test.go:14-44`):

| Seed (hex) | Checksum byte | Last word |
|---|---|---|
| `e58294f2e9a227486e8b061b31cc528fd7fa3f19` | `0x65` | `fracture` |
| `d1d464c004f00fb5c9a4c8d8e433e7fb7ff56256` | `0x80` | `merit` |
| `e34cd132128c1929ec96865ced5c4d0bf40a5d021fcef58d27dbfee371d210` (31 bytes) | `0xc0` | `recipe` |
| 31 bytes of `0x00` | `0x7e` | `insurgent` |

The two 31-byte vectors have an odd length, so their checksum word is taken
from the odd half of the list (index `2c+1`). The first 20 words of the two
20-byte vectors are also the complete vectors of
`pgpwordlist/pgpwordlist_test.go:25-41` (no checksum word at that layer).
The full mnemonic of the first vector is
`topmost Istanbul Pluto vagabond treadmill Pacific brackish dictator goldfish Medusa afflict bravado chatter revolver Dupont midsummer stopwatch whimsical cowbell bottomless fracture`;
of the third,
`tissue disbelief stairway component atlas megaton bedlamp certify tumor monument necklace fascinate tunnel fascinate dreadful armistice upshot Apollo exceed aftermath billiard sardonic vapor microscope brackish suspicious woodlark torpedo hamlet sensation assume recipe`
(`walletseed/seed_test.go:19,31`).

The checksum byte column was recomputed independently while writing this
spec and agrees with the upstream words.

### 4.2 Error kinds

`Kind` is a Go `int`. The numeric values are in-process only (never
persisted or transmitted); the strings are transmitted inside error
messages.

| Value | Constant | `String()` / `Error()` | JSON-RPC code (`internal/rpc/jsonrpc/errors.go:27-44`) | gRPC code (`internal/rpc/rpcserver/server.go:85-119`) |
|---|---|---|---|---|
| 0 | `Other` | `unclassified error` | n/a (never matched) | n/a |
| 1 | `Bug` | `internal wallet error` | `ErrRPCInternal.Code` | falls through |
| 2 | `Invalid` | `invalid operation` | `ErrRPCWallet` | `InvalidArgument` |
| 3 | `Permission` | `permission denied` | `ErrRPCWallet` | `PermissionDenied` |
| 4 | `IO` | `I/O error` | `ErrRPCWallet` | falls through |
| 5 | `Exist` | `item already exists` | `ErrRPCWallet` | `AlreadyExists` |
| 6 | `NotExist` | `item does not exist` | `ErrRPCWallet` | `NotFound` |
| 7 | `Encoding` | `invalid encoding` | `ErrRPCInvalidParameter` | falls through |
| 8 | `Crypto` | `encryption/decryption error` | `ErrRPCWallet` | `DataLoss` |
| 9 | `Locked` | `wallet locked` | `ErrRPCWalletUnlockNeeded` | `FailedPrecondition` |
| 10 | `Passphrase` | `invalid passphrase` | `ErrRPCWalletPassphraseIncorrect` | `InvalidArgument` |
| 11 | `Seed` | `invalid seed` | `ErrRPCWallet` | `InvalidArgument` |
| 12 | `WatchingOnly` | `watching only wallet` | `ErrRPCWallet` | `Unimplemented` |
| 13 | `InsufficientBalance` | `insufficient balance` | `ErrRPCWalletInsufficientFunds` | `ResourceExhausted` |
| 14 | `ScriptFailure` | `transaction script fails to execute` | `ErrRPCWallet` | falls through |
| 15 | `Policy` | `policy violation` | `ErrRPCWallet` | falls through |
| 16 | `Consensus` | `consensus violation` | `ErrRPCWallet` | not listed, falls through |
| 17 | `DoubleSpend` | `double spend` | `ErrRPCWallet` | falls through |
| 18 | `Protocol` | `protocol violation` | `ErrRPCWallet` | falls through |
| 19 | `NoPeers` | `Decred network is unreachable` | `ErrRPCClientNotConnected` | `Unavailable` |
| 20 | `Deployment` | `inactive deployment` | `ErrRPCWallet` | not listed, falls through |
| other | none | `unknown error kind` | `ErrRPCWallet` | falls through |

The two right-hand columns are a cross-reference; the JSON-RPC mapping is
owned by spec 16 and the gRPC mapping by the gRPC server spec. "Falls
through" means the gRPC `switch` case has an empty body (Go cases do not fall
into the next case), so the code after the switch decides
(`internal/rpc/rpcserver/server.go:120-135`): an error wrapping
`hdkeychain.ErrInvalidSeedLen` gives `InvalidArgument`, one wrapping
`context.Canceled` gives `Canceled`, one carrying a gRPC status with a code
other than `OK`/`Unknown` gives that code, anything else `Unknown`.

Usage counts at the pin (non-test `errors.<Kind>` references outside the
`errors` package), to size the port: `IO` 318, `Invalid` 142, `NotExist` 127,
`Crypto` 49, `Protocol` 33, `Exist` 30, `Encoding` 30, `Locked` 23,
`Passphrase` 20, `InsufficientBalance` 19, `Bug` 16, `WatchingOnly` 16,
`Consensus` 13, `Policy` 8, `Seed` 7, `NoPeers` 6, `ScriptFailure` 5,
`DoubleSpend` 4, `Deployment` 4, `Permission` 1, `Other` 0.

### 4.3 Argon2id parameter record (persisted)

`MarshalBinary` output, exactly 25 bytes (`kdf/kdf.go:42-53`):

| Offset | Size | Field | Encoding |
|---|---|---|---|
| 0 | 16 | `Salt` | raw bytes |
| 16 | 4 | `Time` | uint32 little-endian (Argon2 pass count) |
| 20 | 4 | `Memory` | uint32 little-endian, KiB |
| 24 | 1 | `Threads` | uint8 (Argon2 parallelism / lane count) |

The record is stored by the address manager as an account variable
(`wallet/udb/addressdb.go:1058-1068`; bucket and key names are in spec 02).
A stored value of any other length fails to load with
`invalid marshaled Argon2id parameters`. The derived key is
`Argon2id(version 0x13, password, salt = Salt, t = Time, m = Memory KiB,
p = Threads, no secret, no associated data, tag length = 32)`
(`x/crypto@v0.57.0/argon2/argon2.go:48,101-103`,
`wallet/udb/addressmanager.go:156-158`). `Threads` is an input to the
Argon2 hash, not just an execution hint, so the stored value must be used
as-is when re-deriving. As in every Argon2 implementation that follows the
reference, the effective memory is rounded down to a multiple of
`4*Threads` KiB with a floor of `8*Threads` KiB
(`x/crypto@v0.57.0/argon2/argon2.go:114-117`).

### 4.4 JSON-RPC request envelope emitted by the tools (wsrpc)

`sweepaccount` and `repaircfilters` use `wsrpc`
(`wsrpc/v2@v2.3.8/rpc.go:98-105,454-517`). Each call is one websocket text
message containing:

```
{"jsonrpc":"2.0","method":"<name>","params":[...],"id":<n>}
```

- Field order is `jsonrpc`, `method`, `params`, `id`.
- `params` is omitted entirely when the call has no arguments (`omitempty` on
  an empty slice). `listunspent` and `walletlock` from `sweepaccount` are sent
  without a `params` member.
- `id` is a uint32 counter starting at 1 for the first call on a connection,
  incremented per call, skipping 0 on wraparound.
- Arguments are encoded with Go `encoding/json` through a `json.Encoder`
  (`gorilla/websocket@v1.5.1/json.go:23-34`): HTML-escaping of `<`, `>`, `&`
  is on, and the message ends with a single `\n` after the closing brace.
- The response is matched by `id`; a response with a non-null `error` yields
  that error (`code`, `message`, optional `data`); otherwise `result` is
  decoded into the caller's result pointer if one was supplied. A response
  whose `id` is unknown terminates the client with
  `wsrpc: unknown response ID`.
- A frame with a non-empty `method`, **no** `result` member, a null or
  absent `error`, and `id` 0 or absent is treated as a notification
  (`wsrpc/v2@v2.3.8/rpc.go:385`); the tools install no notifier, so
  notifications are dropped. An explicit `"result":null` does not qualify:
  `result` is decoded into a `json.RawMessage`, which holds the four bytes
  `null` (non-nil) in that case, so such a frame is handled as a response
  with id 0 and terminates the client with `wsrpc: unknown response ID`.

Connection properties: HTTP header `Authorization: Basic base64(user:pass)`;
websocket permessage-deflate offered (`EnableCompression: true`); a ping
control frame every 60 s; by default the read deadline is extended to 66.67 s
(`10*60/9`) after each pong (`repaircfilters` disables the pong deadline with
`WithoutPongDeadline`); each request write has a 10 s deadline; `Close` sends
a normal-closure close frame.

### 4.5 `repaircfilters` input file

A concatenation of records, one per block starting at height 0
(`cmd/repaircfilters/README.md:7-18`, `cmd/repaircfilters/main.go:197-215`):

| Size | Field |
|---|---|
| 2 | filter length `n`, uint16 big-endian |
| `n` | version 2 committed filter bytes for that block |

There is no header, no count and no trailer. A zero-length record is valid
and is sent as an empty hex string.

### 4.6 `movefunds` files

All three are in the current working directory.

`unspent.json` (input): a JSON array of `listunspent` result objects
(`rpc/jsonrpc/types/results.go:265-277`). Fields consumed: `txid` (string),
`vout` (uint32), `tree` (int8), `amount` (float, DCR). Other members are
ignored.

`config.json` (input, `cmd/movefunds/main.go:41-46`):

| Member | Type | Meaning in the code |
|---|---|---|
| `txfee` | integer | converted with `NewAmount(float64(txfee))`, i.e. interpreted as **DCR**, and used both as the amount subtracted from the single output and as the per-kB relay fee |
| `sendtoaddress` | string | destination address |
| `network` | string | `"testnet"`, `"mainnet"` or `"simnet"` |
| `dcrctlargs` | string | inserted verbatim after `dcrctl ` in the script |

The shipped sample is
`{"txfee": 10000000, "sendtoaddress": "Ssgymtv6sjYPVEE6mwkbYJz1HzPicKpLCwc", "network": "simnet", "dcrctlargs": "--wallet"}`
(`cmd/movefunds/config.json:1-6`).

`sign.sh` (output, mode 0755, no shebang, no trailing newline,
`cmd/movefunds/main.go:209-232`):

```
dcrctl <dcrctlargs> signrawtransaction <tx hex> '[{"txid":"<hash>","vout":<index>,"tree":<tree>,"scriptpubkey":"<hex of the input signature script, empty>","redeemscript":""},...]' | jq -r .hex
```

One array element per transaction input, in input order, comma separated
without spaces.

### 4.7 `treasurykey` output

Three lines on standard output (`cmd/treasurykey/treasurykey.go:27-29`):

```
Private key: <64 lower-case hex digits>
Public  key: <66 lower-case hex digits, compressed secp256k1 point>
WIF        : <base58 string>
```

Note the two spaces in `Public  key` and the padding before the colon in the
third line. The WIF string is `base58(payload || checksum)` with
(`dcrutil/v4@v4.0.3/wif.go:167-181`):

| Offset | Size | Field |
|---|---|---|
| 0 | 2 | network `PrivateKeyID`: mainnet `22 de`, testnet3 `23 0e`, simnet `23 07`, regnet `22 fe` (`chaincfg/v3@v3.3.0/{mainnet,testnet,simnet,regnet}params.go`) |
| 2 | 1 | signature type `0x02` (`dcrec.STSchnorrSecp256k1`, `dcrec@v1.0.1`) |
| 3 | 32 | private key scalar, big-endian |
| 35 | 4 | first 4 bytes of a single BLAKE-256 hash of bytes 0..34 (`chainhash.HashB`) |

The decoded form is always 39 bytes and the base58 string at most 54
characters (`dcrutil/v4@v4.0.3/wif.go:113-124`).

**Documented example and its drift.** `cmd/treasurykey/README.md:31-34`
shows a mainnet run:

```
$ treasurykey -mainnet
Private key: 9bcf82e4b267585b3f0c54632d7e8eef2fc13407dfdbf7a80398085f5851baa0
Public  key: 03f31aa7013b3c3e8568eb6415a895d8662b5ddceff12a62bf77c01e3e451a332f
WIF        : PmQeT3VmoxDRjgsfwVJLV13ioNyxmd9XuQmQu86UPiQQAn5XkRfLN
```

The public key is the correct compressed point for that private key
(recomputed while writing this spec). The WIF, however, base58-decodes to
`22de 00 9bcf...baa0 60531a6b`: network id mainnet, **signature type `0x00`
(`STEcdsaSecp256k1`)**, not the `0x02` the code at the pin writes
(`cmd/treasurykey/treasurykey.go:21-22`). The README example was produced
by an earlier revision of the tool and is not a valid vector for the
current output; the first three bytes of a current mainnet WIF payload are
`22 de 02`. The trailing checksum `60531a6b` was not recomputed (it needs
BLAKE-256); it is usable as an ECDSA-typed WIF decoding vector only after a
Go oracle confirms it.

The README then imports the key with
`dcrctl --wallet importprivkey <WIF> imported false`
(`cmd/treasurykey/README.md:39`): label `imported` (the only label the
server accepts, `internal/rpc/jsonrpc/methods.go:2031-2033`) and
`rescan = false`. `dcrutil.DecodeWIF` accepts type bytes 0, 1 and 2; any
other type byte is **not** rejected and yields a WIF with scheme 0 and
empty key material (`dcrutil/v4@v4.0.3/wif.go:139-160`). How the address
manager treats a Schnorr-typed WIF on import is owned by the address-manager
and JSON-RPC specs.

### 4.8 Typed client: request and response shapes

Every method issues exactly one JSON-RPC call with positional parameters.
Encoding rules common to all methods (`rpc/client/dcrwallet/marshaling.go`):

- Addresses are sent as their string encoding. Address arrays are written as
  `["a","b"]` without JSON escaping and `[]` when empty or nil
  (`marshaling.go:24-52`).
- Amounts are sent as JSON numbers in DCR: `float64(atoms) / 1e8`
  (`Amount.ToCoin`).
- Transactions are sent as a JSON string holding the lower-case hex of the
  full wire serialization (`marshaling.go:54-63`) and decoded the same way
  (`marshaling.go:223-234`).
- Outpoint lists for `spendoutputs` are arrays of `"<txhash>:<index>"`
  strings, `[]` when empty (`marshaling.go:65-73`).
- Hash results are decoded from byte-reversed hex strings; amount results from
  JSON numbers with `NewAmount` (round half away from zero at 1e-8).
- Optional trailing parameters are simply not sent, except where noted.

| Go method | RPC method | Parameters sent | Result decoding |
|---|---|---|---|
| `GetTransaction(txHash)` | `gettransaction` | hash string | `types.GetTransactionResult` |
| `ListTransactions(account)` | `listtransactions` | account | `[]types.ListTransactionsResult` |
| `ListTransactionsCount(account, count)` | `listtransactions` | account, count | same |
| `ListTransactionsCountFrom(account, count, from)` | `listtransactions` | account, count, from | same |
| `ListUnspent()` | `listunspent` | none | `[]types.ListUnspentResult` |
| `ListUnspentMin(minConf)` | `listunspent` | minConf | same |
| `ListUnspentMinMax(minConf, maxConf)` | `listunspent` | minConf, maxConf | same |
| `ListUnspentMinMaxAddresses(minConf, maxConf, addrs)` | `listunspent` | minConf, maxConf, address array | same |
| `ListSinceBlock(blockHash)` | `listsinceblock` | hash string, or JSON `null` when the hash is nil | `types.ListSinceBlockResult` |
| `ListSinceBlockMinConf(blockHash, minConfirms)` | `listsinceblock` | hash string or `null`, minConfirms | same |
| `LockUnspent(unlock, ops)` | `lockunspent` | unlock bool, array of `{"txid":..,"vout":..,"tree":..}` (`amount` omitted because zero) | none |
| `ListLockUnspent()` | `listlockunspent` | none | array of `{txid, vout, tree}` objects to `[]*wire.OutPoint` |
| `SendToAddress(address, amount)` | `sendtoaddress` | address, amount | hash |
| `SpendOutputs(fromAccount, previousOutpoints, amounts)` | `spendoutputs` | account, outpoint string array, array of `{"address":..,"amount":..}` (JSON `null` if the slice is nil) | hash |
| `SendFrom(fromAccount, toAddress, amount)` | `sendfrom` | account, address, amount | hash |
| `SendFromMinConf(..., minConfirms)` | `sendfrom` | account, address, amount, minConfirms | hash |
| `SendMany(fromAccount, amounts)` | `sendmany` | account, object `{address: amount}` (keys sorted by `encoding/json`) | hash |
| `SendManyMinConf(..., minConfirms)` | `sendmany` | account, object, minConfirms | hash |
| `PurchaseTicket(fromAccount, minConf, numTickets, expiry, ticketChange, ticketFee)` | `purchaseticket` | account, then each non-nil optional in order; a nil optional followed by a later non-nil one is sent as JSON `null`; trailing nils are not sent. `ticketFee` is sent in DCR | array of hash strings |
| `AddMultisigAddress(requiredSigs, addresses, account)` | `addmultisigaddress` | n, address array, account | address |
| `CreateMultisig(requiredSigs, addresses)` | `createmultisig` | n, address array | `types.CreateMultiSigResult` |
| `CreateNewAccount(account)` | `createnewaccount` | account | none |
| `GetNewAddress(account)` | `getnewaddress` | account | address |
| `GetNewAddressGapPolicy(account, gapPolicy)` | `getnewaddress` | account, policy string | address |
| `GetRawChangeAddress(account, net)` | `getrawchangeaddress` | account (the `net` argument is unused) | address |
| `GetAccountAddress(account)` | `getaccountaddress` | account | address |
| `GetAccount(address)` | `getaccount` | address | string |
| `GetAddressesByAccount(account)` | `getaddressesbyaccount` | account | address array |
| `RenameAccount(old, new)` | `renameaccount` | old, new | none |
| `ValidateAddress(address)` | `validateaddress` | address | `types.ValidateAddressWalletResult` |
| `ListAccounts()` | `listaccounts` | none | object `{name: number}` to `map[string]Amount` |
| `ListAccountsMinConf(minConfirms)` | `listaccounts` | minConfirms | same |
| `GetMasterPubkey(account)` | `getmasterpubkey` | account | extended key string |
| `GetBalance(account)` | `getbalance` | account | `types.GetBalanceResult` |
| `GetBalanceMinConf(account, minConfirms)` | `getbalance` | account, minConfirms | same |
| `GetReceivedByAccount(account)` | `getreceivedbyaccount` | account | amount |
| `GetReceivedByAccountMinConf(account, minConfirms)` | `getreceivedbyaccount` | account, minConfirms | amount |
| `GetUnconfirmedBalance(account)` | `getunconfirmedbalance` | account | amount |
| `GetReceivedByAddress(address)` | `getreceivedbyaddress` | address | amount |
| `GetReceivedByAddressMinConf(address, minConfirms)` | `getreceivedbyaddress` | address, minConfirms | amount |
| `ListReceivedByAccount()` | `listreceivedbyacccount` (sic, three `c`) | none | `[]types.ListReceivedByAccountResult` |
| `ListReceivedByAccountMinConf(minConfirms)` | `listreceivedbyacccount` (sic) | minConfirms | same |
| `ListReceivedByAccountIncludeEmpty(minConfirms, includeEmpty)` | `listreceivedbyacccount` (sic) | minConfirms, includeEmpty | same |
| `ListReceivedByAddress()` | `listreceivedbyaddress` | none | `[]types.ListReceivedByAddressResult` |
| `ListReceivedByAddressMinConf(minConfirms)` | `listreceivedbyaddress` | minConfirms | same |
| `ListReceivedByAddressIncludeEmpty(minConfirms, includeEmpty)` | `listreceivedbyaddress` | minConfirms, includeEmpty | same |
| `AccountUnlocked(account)` | `accountunlocked` | account | `types.AccountUnlockedResult` |
| `LockAccount(account)` | `lockaccount` | account | none |
| `UnlockAccount(account, passphrase)` | `unlockaccount` | account, passphrase | none |
| `WalletLock()` | `walletlock` | none | none |
| `WalletPassphrase(passphrase, timeoutSecs)` | `walletpassphrase` | passphrase, timeout (int64) | none |
| `WalletPassphraseChange(old, new)` | `walletpassphrasechange` | old, new | none |
| `SignMessage(address, message)` | `signmessage` | address, message | string |
| `VerifyMessage(address, signature, message)` | `verifymessage` | address, signature, message | bool |
| `DumpPrivKey(address)` | `dumpprivkey` | address | WIF string decoded for the client's network |
| `ImportPrivKey(wif)` | `importprivkey` | WIF string | none |
| `ImportPrivKeyLabel(wif, label)` | `importprivkey` | WIF, label | none |
| `ImportPrivKeyRescan(wif, label, rescan)` | `importprivkey` | WIF, label, rescan | none |
| `ImportPrivKeyRescanFrom(wif, label, rescan, scanFrom)` | `importprivkey` | WIF, label, rescan, scanFrom | none |
| `ImportScript(script)` | `importscript` | hex string | none |
| `ImportScriptRescan(script, rescan)` | `importscript` | hex, rescan | none |
| `ImportScriptRescanFrom(script, rescan, scanFrom)` | `importscript` | hex, rescan, scanFrom | none |
| `AccountAddressIndex(account, branch)` | `accountaddressindex` | account, branch (uint32) | int |
| `AccountSyncAddressIndex(account, branch, index)` | `accountsyncaddressindex` | account, branch, index | none |
| `AddTicket(ticket)` | `addticket` | transaction hex | none |
| `FundRawTransaction(rawhex, fundAccount, options)` | `fundrawtransaction` | hex, account, object `{"changeaddress":..,"feerate":..,"conf_target":..}` with `null` for unset members | `types.FundRawTransactionResult` |
| `GetInfoWallet()` | `getinfo` | none | `types.InfoWalletResult` |
| `GetStakeInfo()` | `getstakeinfo` | none | `types.GetStakeInfoResult` |
| `GetTickets(includeImmature)` | `gettickets` | bool | object `{"hashes":[...]}` to hash list |
| `SetTxFee(fee)` | `settxfee` | amount | none |
| `GetVoteChoices()` | `getvotechoices` | none | `types.GetVoteChoicesResult` |
| `SetVoteChoice(agendaID, choiceID)` | `setvotechoice` | agenda id, choice id | none |
| `WalletInfo()` | `walletinfo` | none | `types.WalletInfoResult` |
| `ListAddressTransactions(addresses, account)` | `listaddresstransactions` | address array, account | `[]types.ListTransactionsResult` |
| `SignRawTransaction(tx)` | `signrawtransaction` | tx hex | `{hex, complete, errors}` |
| `SignRawTransaction2(tx, inputs)` | `signrawtransaction` | tx hex, inputs array (`null` if nil) | same |
| `SignRawTransaction3(tx, inputs, privKeysWIF)` | `signrawtransaction` | tx hex, inputs, WIF array (`null` if nil) | same |
| `SignRawTransaction4(tx, inputs, privKeysWIF, hashType)` | `signrawtransaction` | tx hex, inputs, WIF array, flags string | same |
| `SignRawSSGenTx(tx)` | `signrawtransaction` | tx hex, `null`, `null`, `"ssgen"` | `{hex, complete}` (errors ignored) |

For the four `SignRawTransaction*` methods, a non-empty `errors` array in a
successful response is returned as a `SignatureErrors` error **together with**
the decoded transaction and `complete` flag
(`rpc/client/dcrwallet/methods.go:847-861`).

**Types on the wire.** The structures the client marshals or unmarshals
verbatim (member order is the Go field order):

| Type | Members | Source |
|---|---|---|
| `dcrdtypes.TransactionInput` (`lockunspent` request, `listlockunspent` result) | `amount` (number, `omitempty`), `txid`, `vout` (uint32), `tree` (int8) | `dcrd/rpc/jsonrpc/types/v4@v4.4.0/chainsvrcmds.go:116-121` |
| `types.RawTxInput` (`signrawtransaction` inputs) | `txid`, `vout`, `tree`, `scriptPubKey`, `redeemScript` | `rpc/jsonrpc/types/methods.go:977-983` |
| `types.AddressAmountPair` (`spendoutputs`) | `address`, `amount` (number, DCR) | `rpc/jsonrpc/types/methods.go:1177-1180` |
| `types.FundRawTransactionOptions` | `changeaddress`, `feerate`, `conf_target`; all three pointers without `omitempty`, so unset members are sent as `null` | `rpc/jsonrpc/types/methods.go:165-169` |
| `types.SignRawTransactionError` (`errors` array) | `txid`, `vout`, `scriptSig`, `sequence`, `error` | `rpc/jsonrpc/types/results.go:303-309` |
| `types.ListUnspentResult` | `txid`, `vout`, `tree`, `txtype`, `address`, `account`, `scriptPubKey`, `redeemScript` (`omitempty`), `amount`, `confirmations` (int64), `spendable` | `rpc/jsonrpc/types/results.go:265-277` |

`ValidateAddress` decodes into `types.ValidateAddressWalletResult`, which
is a type alias of `types.ValidateAddressResult`
(`rpc/jsonrpc/types/results.go:392`).

**Result decoding failure modes** (`rpc/client/dcrwallet/marshaling.go:75-234`).
Each `unmarshal*` helper first decodes the JSON value into a Go `string`,
`[]string`, `float64` or `map[string]float64` and then converts; either
step's error is returned from the method unchanged, **alongside** whatever
partial value was already stored.

| Helper | Conversion | Notes |
|---|---|---|
| `unmarshalHash` | `chainhash.NewHashFromStr` | Accepts any hex string of 0 to 64 digits; shorter strings are left-padded with zeros and an odd digit count gets a leading `0` (`dcrd/chaincfg/chainhash@v1.0.5/hash.go:98-129`). A string longer than 64 fails with `ErrHashStrSize`. An empty string, and a JSON `null` result, decode to the all-zero hash with no error. |
| `unmarshalHashes` | same, per element | Allocates a non-nil slice; stops at the first bad element, leaving the elements decoded so far in the caller's slice. |
| `unmarshalOutpoints` | array of `TransactionInput` to `wire.NewOutPoint(hash, vout, tree)` | The `amount` member is ignored. |
| `unmarshalAddress`, `unmarshalAddresses` | `stdaddr.DecodeAddress(s, net)` with the network given to `NewClient` | An address of another network is a decode error. A `null` result is an error (empty string is not an address). |
| `unmarshalListAccounts` | `dcrutil.NewAmount` per member | Writes into the map created by the caller; iteration is over a Go map, so on a mid-way error which accounts are present is unspecified. |
| `unmarshalHDKey` | `hdkeychain.NewKeyFromString(s, net)` | |
| `unmarshalAmount` | `dcrutil.NewAmount(float64)` | `null` decodes to 0. |
| `unmarshalWIF` | `dcrutil.DecodeWIF(s, net.PrivateKeyID)` | |
| `unmarshalTx` | hex-decoding reader into `MsgTx.Deserialize` | A fresh `MsgTx` is allocated before decoding, so the caller receives a non-nil, possibly partly filled transaction on error. Trailing hex after a complete transaction is not detected. |

When one of these helpers is the top-level result target, a JSON `null`
result still invokes it (with the bytes `null`). When it is a struct member
typed as the `json.Unmarshaler` interface (`hex` in the `signrawtransaction`
results, `hashes` in `gettickets`), a `null` member instead sets the
interface to nil without calling the helper, and an absent member leaves it
untouched; in both cases the caller's variable stays nil
(`go1.27.4/src/encoding/json/decode.go:433-500`, `rpc/client/dcrwallet/methods.go:724-733,847-861`).

**Further request-side facts** (`rpc/client/dcrwallet/methods.go`):

- Go `int`, `int64` and `uint32` arguments are sent as JSON integers
  without a fraction or exponent; `bool` as `true`/`false`; account names,
  labels, passphrases and messages as JSON strings with Go's default
  escaping (HTML-unsafe characters `<`, `>`, `&` as `\u003c`, `\u003e`,
  `\u0026`, invalid UTF-8 replaced by U+FFFD).
- `LockUnspent` always sends an array for its second parameter: `[]` for a
  nil or empty `ops` (the slice is allocated with `make`), never `null`
  (`rpc/client/dcrwallet/methods.go:146-156`). `LockUnspent(true, nil)` is
  thus `lockunspent true []`, which the method comment documents as
  "unlock everything".
- Methods taking pointer arguments dereference them unconditionally:
  a nil `txHash` (`GetTransaction`), nil `*dcrutil.WIF` (`ImportPrivKey*`),
  nil `*dcrutil.Tx` (`AddTicket`), nil `*wire.MsgTx` (`SignRaw*`) or nil
  `stdaddr.Address` panics inside the client before any request is sent.
  Only `ListSinceBlock*` (hash) and `PurchaseTicket` (optionals) accept
  nil.
- `GetInfoWallet` is named so to avoid clashing with a dcrd client's
  `GetInfo`; it calls `getinfo` (`rpc/client/dcrwallet/methods.go:709-715`).
- `SigHashType` values are passed through `String()` unvalidated; any
  string can be cast to the type and is sent as is.

`GetTickets` ignores every member of the result other than `hashes`.
`SignRawSSGenTx` has no `errors` member in its result struct, so signature
errors reported by the server are dropped silently
(`rpc/client/dcrwallet/methods.go:972-982`). `SendMany`/`SendManyMinConf`
key the amounts object by `Address.String()`; two distinct `Address` values
with the same string collapse into one member and one of the two amounts
is lost (Go map iteration order decides which)
(`rpc/client/dcrwallet/methods.go:227-253`).

### 4.9 Version strings

`version.String()` forms at the pin: `2.2.0-pre` (no VCS stamp, for example
`go install pkg@version`), `2.2.0-pre+54b85b336` (built from a git checkout
of the pin with VCS stamping), `2.2.0` or `2.2.0+release` style for release
builds that override `PreRelease`/`BuildMetadata` at link time. The exact
`-ldflags` used for official releases are outside this repository
(UNVERIFIED; they live in the release tooling).

## 5. Algorithms and state machines

None of the library packages in this scope opens a database transaction or
performs I/O other than reading the system random source. The tools perform
network and file I/O as described per tool.

### 5.1 `pgpwordlist.DecodeMnemonics(words)`

`pgpwordlist/pgpwordlist.go:36-60`.

1. Allocate an output buffer of `len(words)` bytes; `idx = 0`.
2. For each `w` in order:
   1. `w = TrimSpace(w)` (Unicode white space at both ends). If empty,
      skip it; `idx` does not advance.
   2. Look up `ToLower(w)` in the index map. If absent, return
      `E("pgpwordlist.DecodeMnemonics", Encoding, "word <w> is not in the PGP word list")`,
      where `<w>` is the trimmed, original-case word.
   3. Let `b` be the list index. If `b mod 2 != idx mod 2`, return
      `E("pgpwordlist.DecodeMnemonics", Encoding, "word <w> is not valid at position <idx>, check for missing words")`.
      `<idx>` is the zero-based position among the non-empty words.
   4. `out[idx] = b / 2`; `idx++`.
3. Return `out[:idx]` (possibly empty), nil error.

There is no length limit and no checksum handling at this layer.

### 5.2 `walletseed.GenerateRandomSeed(size)`

`walletseed/seed.go:21-34`.

1. If `size >= 255`: return `E("walletseed.GenerateRandomSeed", Invalid, ErrInvalidSeedLen)`.
2. If `size < 16` or `size > 64`: same error.
3. `hdkeychain.GenerateSeed(uint8(size))`: read `size` bytes from the
   cryptographic random source (`dcrd/hdkeychain/v3@v3.1.3/extendedkey.go:623-636`).
   A read failure is returned as `E("walletseed.GenerateRandomSeed", err)`
   (kind `Other`).

`ErrInvalidSeedLen` renders as `seed length must be between 128 and 512 bits`
(`dcrd/hdkeychain/v3@v3.1.3/extendedkey.go:79-80`), so the full message is
`walletseed.GenerateRandomSeed: invalid operation: seed length must be between 128 and 512 bits`.

The daemon's own seed generation does not go through this function: the
interactive prompt and the simnet setup call `hdkeychain.GenerateSeed(32)`
directly (`internal/prompt/prompt.go:271`, `walletsetup.go:278`), and the gRPC
`GenerateRandomSeed` handler fills a buffer itself
(`internal/rpc/rpcserver/server.go:3177-3197`). At the pin
`GenerateRandomSeed` is called only by the `wallet/udb/testdata` generators.

### 5.3 `walletseed.DecodeUserInput(input)`

`walletseed/seed.go:71-102`. `op = "walletseed.DecodeUserInput"`.

1. `words = Split(TrimSpace(input), " ")`. The split is on the single ASCII
   space character only; tabs and newlines inside the input are not
   separators. `Split` of an empty string yields one empty token.
2. If `len(words) == 1`: `seed = hex.DecodeString(words[0])`. On error
   return `E(op, Encoding, err)`. Messages:
   - `walletseed.DecodeUserInput: invalid encoding: encoding/hex: odd length hex string`
   - `walletseed.DecodeUserInput: invalid encoding: encoding/hex: invalid byte: U+0067 'g'`
     (Go `%#U` formatting of the offending byte; an invalid byte is
     reported in preference to odd length).
3. If `len(words) > 1`:
   1. `decoded = pgpwordlist.DecodeMnemonics(words)`. On error return
      `E(op, Encoding, err)`. Because the inner error has a different `Op`
      and the same kind, the result is a two-level chain rendered as
      `walletseed.DecodeUserInput: invalid encoding<SEP>pgpwordlist.DecodeMnemonics: word <w> is not in the PGP word list`
      (or the position message), where `<SEP>` is `errors.Separator`:
      `":: "` inside the daemon, `":\n\t"` elsewhere.
   2. If `len(decoded) < 2`: leave `seed` nil and go to step 4 (which
      fails).
   3. If `checksumByte(decoded[:len-1]) != decoded[len-1]`: return
      `E(op, Encoding, "checksum mismatch")`, rendered
      `walletseed.DecodeUserInput: invalid encoding: checksum mismatch`.
   4. `seed = decoded[:len-1]`.
4. If `len(seed) < 16` or `len(seed) > 64`: return
   `E(op, Encoding, ErrInvalidSeedLen)`, rendered
   `walletseed.DecodeUserInput: invalid encoding: seed length must be between 128 and 512 bits`.
5. Return `seed`.

Consequences: an empty or all-white-space input fails at step 4 (empty hex
decodes to zero bytes); a single mnemonic word is treated as hex and fails
at step 2; valid mnemonics are 17 to 65 words.

How the result reaches clients:

- gRPC `SeedService.DecodeSeed` returns status `InvalidArgument` with the
  rendered error text as the message
  (`internal/rpc/rpcserver/server.go:3199-3205`).
- The interactive `--create` prompt calls `DecodeUserInput` only when the
  collapsed input has two or more tokens; a one-token input is hex-decoded
  by the prompt itself (after padding an odd-length string with a leading
  `0`), so its error text is the bare `encoding/hex` message without the
  `walletseed.DecodeUserInput: invalid encoding: ` prefix. In both cases the
  prompt prints `Input error: <text>`, then the
  `Invalid seed specified. ...` line, and re-prompts
  (`internal/prompt/prompt.go:337-363`). The prompt pre-processes the input
  (joins lines with spaces, collapses every run of Unicode white space to
  one space) and is specified with the daemon start-up spec.

### 5.4 `errors.E(args...)`

`errors/errors.go:187-236`.

1. If there are no arguments, panic with the string `errors.E: no args`.
2. Start with an empty `Error e` and `prev = nil`.
3. For each argument, in order, by dynamic type (first matching rule):
   | Argument type | Effect |
   |---|---|
   | `Op` | `e.Op = arg` |
   | `Kind` | `e.Kind = arg` |
   | `string` | `e.Err = New(arg)` |
   | `*Error` | `prev = arg`; if `e.Kind == Other` then `e.Kind = arg.Kind`; `e.Err = arg` |
   | any other `error` | `e.Err = arg` |
   | anything else (including an untyped nil, i.e. a nil `error` interface) | ignored |
   A later argument of the same category overwrites an earlier one
   (strings and errors share the `Err` slot).
4. If `prev != nil` and `e.Err` is still that same `prev` pointer:
   1. If `e.Op == ""`, `e.Op = prev.Op`.
   2. If `e.Kind == Other`, `e.Kind = prev.Kind`.
   3. Collapse: if `(prev.Op == "" || e.Op == prev.Op)` and
      `(prev.Kind == Other || e.Kind == prev.Kind)`, then `e.Err = prev.Err`
      and, if `e` has no stack, it takes `prev`'s stack. The inner level
      disappears from the chain.
5. Return `&e` (always a non-nil `*Error`, even when every field is zero).

Worked examples (separator shown as `<SEP>`):

| Call | Resulting chain | `Error()` |
|---|---|---|
| `E(Op("a"), Passphrase)` | `{a, Passphrase, nil}` | `a: invalid passphrase` |
| `E(IO, plainErr("x"))` | `{"", IO, x}` | `I/O error: x` |
| `E(Op("a"), E(IO, "disk"))` | collapsed: `{a, IO, "disk"}` | `a: I/O error: disk` |
| `E(Op("a"), E(Op("b"), IO, "disk"))` | `{a, IO, {b, IO, "disk"}}` | `a: I/O error<SEP>b: disk` |
| `E(Op("a"), Invalid, E(Op("b"), IO, "disk"))` | `{a, Invalid, {b, IO, "disk"}}` | `a: invalid operation<SEP>b: I/O error: disk` |
| `E(Op("a"), E(Op("a"), IO, "disk"))` | collapsed: `{a, IO, "disk"}` | `a: I/O error: disk` |
| `E(E(Invalid, "abc"), "def")` | `{"", Invalid, "def"}` (kind promoted, inner error replaced by the string) | `invalid operation: def` |
| `E("text")` | `{"", Other, "text"}` | `text` |
| `E(Op(""))` | all zero | `unclassified error` |

Additional consequences of the code as written:

- Passing `Other` explicitly is indistinguishable from passing no kind:
  `E(op, Other, inner)` still promotes `inner`'s kind in step 4.2, and the
  check in the `*Error` row (`e.Kind == Other`) treats an earlier explicit
  `Other` as unset (`errors/errors.go:204-206,221-223`).
- The `*Error` row copies the kind at the moment the argument is seen.
  `E(inner, Invalid)` therefore first takes `inner.Kind` and is then
  overwritten by `Invalid`; `E(Invalid, inner)` keeps `Invalid`. Both end
  with the same kind; only with two `*Error` arguments does order matter
  (Q2).
- A `Kind` value is matched by the `Kind` row, never by the `error` row,
  although `Kind` implements `error`; an `Op` is never treated as a string.
  An untyped string constant is a `string` (it becomes `Err`), so an `Op`
  must be declared with its type (`const op errors.Op = "..."`), which is
  the form every call site uses.
- `WithStack` overwrites any stack inherited through a collapse with the
  stack of its own call site (`errors/errors.go:241-245`). `Stacks(err)`
  type-asserts `err` itself without unwrapping, so it returns nil for an
  `*Error` that is wrapped by `fmt.Errorf("%w")` or `Join`, and it skips
  levels whose stack is nil (`errors/errors.go:390-400`).
- The package does not export `Unwrap`; callers that need it import the
  standard library. The exported surface is exactly the 9 functions, 7
  methods, 3 types (`Error`, `Op`, `Kind`), 21 constants and 1 variable of
  section 3.3.

### 5.5 `(*Error).Error()`

`errors/errors.go:247-290`.

1. `out = ""`, `lastOp = ""`, `lastKind = Other`, `e` = receiver.
2. Loop:
   1. `pad = false`.
   2. If `e.Op != ""` and `e.Op != lastOp`: append `e.Op`; `pad = true`;
      `lastOp = e.Op`.
   3. If `e.Kind != Other` and `e.Kind != lastKind`: if `pad`, append `": "`;
      append `e.Kind.String()`; `pad = true`; `lastKind = e.Kind`.
   4. If `e.Err == nil`: stop.
   5. If `e.Err` is an `*Error` (direct type test, no unwrapping): if `pad`,
      append `Separator`; `e = e.Err`; continue the loop.
   6. Otherwise: if `pad`, append `": "`; append `e.Err.Error()`; stop.
3. If `out` is empty return `unclassified error`, else `out`.

`lastOp`/`lastKind` track the most recently written values, not the parent's
values, so an `Op` or kind is suppressed only when it repeats the one written
immediately before it in the output.

### 5.6 Matching: `Is`, `As`, `match`

These are the standard library traversals (`go1.27.4/src/errors/wrap.go`)
driven by the methods on `*Error` and `Kind`.

**`Is(err, target)`**

1. If either is nil, return `err == target`.
2. Walk the chain starting at `err`. At each node `n`:
   1. If the target's dynamic type is comparable and `n == target` (Go
      interface equality: same dynamic type and equal value; for `*Error`
      that is pointer identity, for `Kind` value equality), return true.
   2. If `n` has an `Is` method and `n.Is(target)` is true, return true. For
      `*Error` nodes (`errors/errors.go:330-338`):
      - target is `*Error`: return `match(target, n)` (below);
      - target is `Kind`: return `n.Kind != Other && n.Kind == target`;
      - otherwise false.
   3. Move to `n.Unwrap()`. For `*Error` that is `Err` if non-nil, else the
      `Kind` value if not `Other`, else stop (false). For a `Join` result,
      recurse depth-first into each member in order.

**`match(template, e)`** (`errors/errors.go:357-386`): the template is the
first argument.

1. Both must be `*Error`, else false.
2. If `template.Op != ""` and differs from `e.Op`: false.
3. If `template.Kind != Other` and differs from `e.Kind`: false.
4. If `template.Err == nil`: true.
5. If `template.Err == e.Err` (interface equality): true.
6. If `template.Err` is an `*Error`: return `match(template.Err, e.Err)`.
7. Otherwise return `template.Err.Error() == e.Err.Error()` (string
   comparison; see quirk Q5 for the nil case).

So `Is(err, E(Op("x")))` asks "does any `*Error` in the chain have `Op` x",
`Is(err, E(Permission))` asks for the kind, and a template with several
fields set requires all of them on the same node.

**`As(err, &kind)`** (target type `*Kind`), the form used by the RPC error
mappers:

1. Walk the chain. At each node: if the node's dynamic type is `Kind`
   itself, assign it and return true (this happens when a bare `Kind` was
   used as an error or when `Unwrap` produced the kind of a leaf `*Error`).
   If the node is an `*Error` with `Kind != Other`, assign that kind and
   return true (`errors/errors.go:311-323`). Otherwise continue with
   `Unwrap()`.
2. The result is therefore the **outermost** non-`Other` kind in the chain.
   Because `E` promotes an inner kind to an outer level that has none, the
   outermost level normally carries the innermost classified kind unless an
   intermediate caller set its own.

**`As(err, &e)`** with `e *Error`: the first node whose dynamic type is
`*Error` is assigned. If the chain reaches a bare `Kind` (not `Other`),
`Kind.As` synthesises `&Error{Kind: k}` (`errors/errors.go:136-149`).

### 5.7 `lru` insertion

`Cache.Add(item)` (`lru/cache.go:36-59`) and `Map.Add(key, value)`
(`lru/map.go:41-65`), under the mutex:

1. If the key is present: move its list element to the front and return.
   `Map.Add` does **not** replace the stored value.
2. If `len(map) > limit` (strictly greater): remove the back element of the
   list and delete its key.
3. Push the new element at the front and record it in the map.

Because step 2 tests the size before insertion with `>`, the structure grows
to `limit+1` entries before the first eviction and then stays at `limit+1`.
`Contains` never reorders. `Map.Get`/`Hit` move the element to the front.
With `limit = 0` the structure holds one entry.

### 5.8 `movefunds`

`cmd/movefunds/main.go:122-239`. No flags; arguments are ignored. All
messages go to standard output. Except where stated, failures print a
message **without a trailing newline** and return from `main`, so the exit
status is 0.

1. Open `unspent.json`; on failure print
   `error opening unspent file unspent.json: <err>`.
2. JSON-decode it as an array of `listunspent` results; on failure print
   `error parsing unspent file: <err>`.
3. Build the input source (`cmd/movefunds/main.go:70-120`). For each entry in
   order:
   1. `amount = NewAmount(entry.amount)` (`round(f*1e8)` half away from
      zero; fails only for NaN/Inf, which JSON cannot carry). On failure
      record ``invalid amount `<v>` in listunspent result`` and stop
      iterating.
   2. If `amount == 0`, skip the entry.
   3. If `amount < 0` or `amount > 21e14`, record
      ``impossible output amount `<amount>` in listunspent result`` and stop.
   4. Add to the running total.
   5. Parse `txid` as a hash; on failure record
      `invalid data in listunspent result: <err>` and stop.
   6. Append an input for outpoint `(hash, vout, tree)` with `ValueIn =
      amount` and an empty signature script.
   If no error was recorded and the total is zero, record `no input value`.
   The source returns the full input set and the recorded error regardless
   of the requested target.
4. Open `config.json` (`error opening config file config.json: <err>`),
   decode it (`error parsing config file: <err>`).
5. Select parameters from `network`; otherwise print
   `unknown network specified: <network>`.
6. Decode `sendtoaddress` for that network; on failure print
   `failed to parse address <addr>: <err>`.
7. `fee = NewAmount(float64(txfee))`; on failure `invalid fee rate: <err>`.
8. Build one output paying `total - fee` to the address's payment script.
9. Call `txauthor.NewUnsignedTransaction(outputs, fee, source, nil,
   params.MaxTxSize)` (`cmd/movefunds/main.go:189`). **At the pin this call
   dereferences the nil change source on its first statements
   (`wallet/txauthor/author.go:97`) and the program terminates with a Go
   nil-pointer panic (stack trace on standard error, exit status 2).** Steps
   10 to 13 are therefore unreachable; they are listed for completeness.
10. On an authoring error print `failed to create unsigned transaction: <err>`.
11. If the serialized size exceeds `MaxTxSize` print
    `tx too big: got <n>, max <m>`.
12. Serialize the transaction; write `sign.sh` (section 4.6). Failures print
    `Failed to serialize tx:  <err>` or `Failed to write signing script:  <err>`
    (two spaces, with newline).
13. Print `Successfully wrote transaction to sign script.` and a newline.

Details of the unreachable tail that a corrected port needs:

- The single output's value is `total - fee` with no check that it is
  positive (`cmd/movefunds/main.go:182-188`); `total` is the sum computed
  in step 3 even when the input source recorded an error.
- Input-source errors recorded in step 3 are not reported at step 3. They
  surface only through the authoring call in step 9/10, i.e. after the
  configuration has been read and validated.
- The script text is assembled by string concatenation, not by a JSON
  encoder: `vout` and `tree` are written with `%v` (decimal), the hash in
  `chainhash` string form, and `dcrctlargs` unquoted
  (`cmd/movefunds/main.go:209-231`). The byte sequence after the array is
  `]' ` followed by `| jq -r .hex`.
- In the input-source messages, `<v>` of `invalid amount` is the JSON
  float printed with Go `%v` (shortest representation, exponent form for
  very large or small magnitudes), and `<amount>` of
  `impossible output amount` is `Amount.String()`, i.e. the DCR value
  followed by ` DCR` (for example ``impossible output amount `-1 DCR` in
  listunspent result``) (`cmd/movefunds/main.go:81-91`). The same holds in
  `sweepaccount` (`cmd/sweepaccount/main.go:195-206`).
- Each input is built with `wire.NewTxIn`: sequence `0xffffffff`,
  `ValueIn` = the output amount in atoms, block height `0x00000000` (null
  block height) and block index `0xffffffff`, empty signature script
  (`dcrd/wire@v1.8.0/msgtx.go:338-347`). The authored transaction has
  serialization type full, version 1, lock time 0 and expiry 0
  (`wallet/txauthor/author.go:28,126-133`). These decide the bytes of the
  hex in `sign.sh` and of the transaction `sweepaccount` would sign.
- The zero-amount skip happens before the outpoint is parsed, so a
  zero-valued entry with a malformed `txid` is ignored silently
  (`cmd/movefunds/main.go:85-101`).
- `unspent.json` and `config.json` are opened read-only and never closed
  explicitly; `sign.sh` is created or truncated with mode `0755` subject to
  the process umask.

**Intended workflow** (`docs/offline_wallets.md:21-99`; this is the only
user documentation of the tool and pins the RPC surface it relies on):

1. On the online machine with a watching-only wallet:
   `dcrctl --wallet listunspent > unspent.json`, then record the outputs of
   `dcrctl --wallet accountaddressindex <account> 0` and
   `dcrctl --wallet accountaddressindex <account> 1`.
2. Edit `unspent.json` to remove outputs that must not be spent; copy and
   edit `config.json`.
3. Run `movefunds` in that directory; carry `sign.sh` to the offline
   machine (which needs `jq`).
4. On the offline machine start the node with `--connect=127.0.0.1:12345`
   so that it idles at the genesis block, start the wallet, and run
   `dcrctl --wallet accountsyncaddressindex <account> 0 <index0>` and
   `dcrctl --wallet accountsyncaddressindex <account> 1 <index1>`.
5. `./sign.sh > rawtx.txt`; carry the hex back and run
   `dcrctl sendrawtransaction $(cat rawtx.txt)` (as written, against the
   node, not the wallet).

The document describes the tool as usable on "*nix machines" only.

### 5.9 `repaircfilters`

Flags (go-flags, `cmd/repaircfilters/main.go:35-48`):

| Flag | Type | Default | Description string |
|---|---|---|---|
| `--testnet` | bool | false | `Use the test decred network` |
| `-c`, `--connect` | string | `localhost` (see step 1) | `Hostname[:port] of wallet RPC server` |
| `-u`, `--rpcuser` | string | empty | `Wallet RPC username` |
| `-P`, `--rpcpass` | string | empty | `Wallet RPC password` |
| `--cafile` | string | `<appdata>/rpc.cert` (see step 1) | `Wallet RPC TLS certificate` |
| `--cfiltersfile` | string | empty | `Binary file with pre-dcp0005 filter data` |

`<appdata>` is `dcrutil.AppDataDir("dcrwallet", false)`: `~/.dcrwallet` on
POSIX systems, `~/Library/Application Support/Dcrwallet` on macOS,
`%LOCALAPPDATA%\Dcrwallet` on Windows (falling back to `%APPDATA%`),
`$home/dcrwallet` (no leading dot) on Plan 9, `.` if no home directory (on
Windows: neither variable) is known (`dcrutil/v4@v4.0.3/appdata.go:21-73`).

Start-up validation (`init`, `cmd/repaircfilters/main.go:94-133`). `fatalf`
writes the message and a newline to standard error and exits with status 1.

1. If the default certificate path does not exist, clear both the
   `--connect` and `--cafile` defaults (before parsing).
2. Parse flags. On any parse error, including `-h`/`--help`, exit with
   status 1 (go-flags has already printed the help text to standard output
   or the error to standard error, `go-flags@v1.5.0/parser.go:702-714`).
   Positional arguments are ignored.
3. Network: mainnet, or testnet3 with `--testnet`.
4. Empty `--connect`: `RPC hostname[:port] is required`.
5. Normalize the address (below) with the default port for the network
   (mainnet `9110`, testnet3 `19110`); failure:
   ``Invalid RPC network address `<input>`: <err>``.
6. Empty `--rpcuser`: `RPC username is required`.
7. Certificate file missing: ``RPC certificate file `<path>` not found``.
8. Empty `--cfiltersfile`: `CFilters file is required`.

Address normalization (`cmd/repaircfilters/main.go:63-78`, identical in
`sweepaccount`): if `SplitHostPort(addr)` succeeds, return
`JoinHostPort(host, port)`. Otherwise join the whole input with the default
port; if that splits cleanly return it, else return the **original** split
error.

Run (`repair`, `cmd/repaircfilters/main.go:146-243`):

1. Password: `-P` value, or prompt `Wallet RPC password: ` on standard
   output, read without echo from the standard input terminal, then print a
   newline. Failure: `failed to read RPC password: <err>`.
2. Open the cfilters file (`failed to open cfilters file: <err>`).
3. Read the certificate (`failed to read RPC certificate: <err>`); add it to
   an otherwise empty root pool (the parse result is not checked).
4. Dial `wss://<host:port>/ws` with basic auth and no pong deadline
   (`failed to connect to the wallet: <err>`).
5. `height = 0` (int32); 65,536-byte buffer; `readOffset = 0`; BLAKE-256
   hasher.
6. Loop:
   1. Read into `buf[readOffset:]`.
   2. Split `buf[:readOffset+n]` into complete records: while at least 2
      bytes remain, read the big-endian length; if the record is not
      complete, stop; otherwise hash the filter bytes (not the length
      prefix), append their hex to the batch and advance.
   3. If the batch is non-empty, call `importcfiltersv2` with parameters
      `[height, [hex, ...]]`, discarding the result. Failure:
      `failed to import cfilters: <err>` (for a server error, `<err>` is the
      JSON-RPC error message).
   4. `height += len(batch)`; move the unconsumed tail to the front of the
      buffer; `readOffset = len(tail)`.
   5. If the read reported end of file, leave the loop. An incomplete
      trailing record is dropped silently.
7. Print `Hash of cf data sent: <hash>` where `<hash>` is the BLAKE-256 of
   all filter bytes in `chainhash` string form (byte-reversed hex), then
   `Height: <height-1>`. For a complete, correct file the hash equals the
   `PreDCP0005CFilterHash` target for the network (section 3.6).
8. Exit status 0; on any error above, the message on standard error and
   status 1.

Batches are sent strictly sequentially, each waiting for its response; the
start height of a batch is the number of filters sent before it.

Additional facts:

- The source contains a fifth run-time message, `cfiltersfile read error: <err>`
  (`cmd/repaircfilters/main.go:193-195`). It is unreachable: the guard
  tests the outer `err`, which is nil whenever the loop is entered and
  becomes non-nil only on the `importcfiltersv2` failure that returns
  immediately (quirk Q23).
- The certificate is read after the cfilters file is opened and before
  dialling. The cfilters file and the websocket (normal closure frame) are
  closed by deferred calls on every return path of `repair`, success or
  error, before `main` prints the error and exits.
- `height` is a signed 32-bit counter with no overflow check; the
  `importcfiltersv2` start height is sent as a JSON integer and the filter
  list as a JSON array of lower-case hex strings.
- Expected input, per the README: one record for every block height below
  the DCP0005 activation height of the selected network (431488 records on
  mainnet, 323328 on testnet3; section 3.6), so a complete run prints
  `Height: 431487` or `Height: 323327`. The tool itself enforces neither
  the count nor the hash; the wallet's `importcfiltersv2` handler does the
  validation (JSON-RPC spec).
- `cmd/repaircfilters/README.md:20-21` names
  `https://github.com/matheusd/cfilterv2hashes` as a generator for the
  input file. The README's sample invocation is
  `go run . -c localhost:19110 -u USER -p PASSWORD --cafile ~/.dcrwallet/rpc.cert --cfiltersfile testnet-data.bin`
  (`cmd/repaircfilters/README.md:33-34`): it omits `--testnet` (so the
  tool keeps mainnet parameters while talking to the testnet port, which
  only affects the default port) and uses `-p`, which the tool does not
  define (Q23).

### 5.10 `sweepaccount`

Flags (go-flags, `cmd/sweepaccount/main.go:48-76`):

| Flag | Type | Default | Description string |
|---|---|---|---|
| `--testnet` | bool | false | `Use the test decred network` |
| `--simnet` | bool | false | `Use the simulation decred network` |
| `-c`, `--connect` | string | `localhost` | `Hostname[:port] of wallet RPC server` |
| `-u`, `--rpcuser` | string | empty | `Wallet RPC username` |
| `-P`, `--rpcpass` | string | empty | `Wallet RPC password` |
| `--cafile` | string | `<appdata>/rpc.cert` | `Wallet RPC TLS certificate` |
| `--feerate` | float64 | `0.0001` | `Transaction fee per kilobyte` |
| `--sourceacct` | string | empty | `Account to sweep outputs from` |
| `--sourceaddr` | string | empty | `Address to sweep outputs from` |
| `--destacct` | string | empty | `Account to send sweeped outputs to` |
| `--destaddr` | string | empty | `Address to send sweeped outputs to` |
| `--minconf` | int64 | `2` | `Required confirmations to include an output` |
| `--dryrun` | bool | false | `Do not actually send any transactions but output what would have happened` |

Start-up validation (`init`, `cmd/sweepaccount/main.go:112-173`), in this
order, each failing with `fatalf` (standard error, status 1):

1. Clear the `--connect` and `--cafile` defaults if the default certificate
   does not exist; parse flags (exit 1 on error or help).
2. `--testnet` and `--simnet` together:
   `Multiple decred networks may not be used simultaneously`.
3. Empty `--connect`: `RPC hostname[:port] is required`.
4. Normalization failure (default ports mainnet `9110`, testnet3 `19110`,
   simnet `19557`): ``Invalid RPC network address `<input>`: <err>``.
5. Empty user: `RPC username is required`.
6. Missing certificate: ``RPC certificate file `<path>` not found``.
7. `--feerate > 1`: ``Fee rate `<v>/kB` is exceptionally high``.
8. `--feerate < 1e-6`: ``Fee rate `<v>/kB` is exceptionally low``.
9. Neither source: `A source is required`.
10. Source account equal to destination account:
    `Source and destination accounts should not be equal`.
11. Neither destination: `A destination is required`.
12. Both destinations: `Destination must be either an account or an address`.
13. `--minconf < 0`: `Required confirmations must be non-negative`.

Both `--sourceacct` and `--sourceaddr` may be given; then both filters apply.

Notes on the checks (`cmd/sweepaccount/main.go:152-172`):

- Check 10 applies only when `--sourceacct` is non-empty. With only
  `--sourceaddr` given, no comparison with the destination account is made,
  so sweeping an address into the account that owns it is allowed.
- Checks 7 and 8 are plain float comparisons. `--feerate NaN` (accepted by
  the flag parser, which uses Go `strconv.ParseFloat`) passes both and fails
  later, after the passphrase prompt, with `invalid fee rate: <err>`.
  `+Inf` fails check 7 and `-Inf` check 8. The message prints the float
  with Go `%v`.
- `--destaddr` is not parsed or checked against the network at start-up
  (see the wire-level details below).
- The flag structure carries no `default:` tags; the defaults are the
  initial values of the Go struct, which go-flags still reports in the help
  text (section 5.10.1).

Run (`sweep`, `cmd/sweepaccount/main.go:295-463`), as written:

1. RPC password as in 5.9 step 1.
2. Read the certificate (`failed to read RPC certificate: <err>`). If it
   contains no parsable PEM certificate:
   `unparsable certificate authority: unparsable certificate authority`.
3. Dial with TLS and basic auth, passing the normalized `host:port` **without
   a scheme or path** (`cmd/sweepaccount/main.go:322`). **At the pin this
   fails for every address form the normalizer can produce**: the websocket
   dialer requires a `ws` or `wss` URL
   (`gorilla/websocket@v1.5.1/client.go:173-185`), so the program ends with
   `failed to create RPC client: malformed ws or wss URL` (or a URL parse
   error for inputs such as numeric IPv4 hosts) and status 1. The remaining
   steps describe the intended behaviour.
4. Call `listunspent` with no parameters. Keep entries that are `spendable`,
   have `confirmations >= --minconf`, match `--sourceacct` (if set) by
   `account` and `--sourceaddr` (if set) by `address`. Group them by
   `address`. Failure: `failed to fetch unspent outputs: <err>`.
5. For each group (Go map order, unspecified) print
   `Found <n> matching unspent <output|outputs> for address <addr>`.
6. If there is at least one group, prompt `Wallet private passphrase: `
   (`failed to read private passphrase: <err>`).
7. `feeRate = NewAmount(--feerate)` in atoms per kB (failure, NaN or
   infinity only: `invalid fee rate: <err>`).
8. For each group (map order again), one transaction:
   1. Build an input source over the group exactly as in 5.8 step 3.
   2. Author with `txauthor.NewUnsignedTransaction(nil, feeRate, source,
      changeSource, MaxTxSize)`: no fixed outputs, everything goes to the
      "change" output. The change source reports script size 25 and
      produces the script either by calling `getnewaddress <destacct>` and
      decoding the returned address, or by decoding `--destaddr`.
      An error other than `no input value` is reported as
      `Failed to create unsigned transaction: <err>`; either way the group
      is skipped.
   3. `walletpassphrase <passphrase> 60`; failure:
      `Failed to unlock wallet: <err>`, skip.
   4. `signrawtransaction` with the transaction as the only parameter, then
      `walletlock` unconditionally (its error is ignored). Failure of the
      former: `Failed to sign transaction: <err>`, skip. **The parameter is
      the Go transaction struct, not hex** (`cmd/sweepaccount/main.go:420`);
      `wire.MsgTx` has no JSON marshaller (`dcrd/wire@v1.8.0/msgtx.go:380-388`),
      so the wire form is a JSON object and a real server rejects it.
   5. If the result's `complete` is false: `Failed to sign every input`,
      skip.
   6. With `--dryrun` print `DRY RUN: not actually sending transaction`;
      otherwise `sendrawtransaction <hex> false` and take the returned hash
      string. Failure: `Failed to publish transaction: <err>`, skip.
   7. Print `Swept <amount> to destination with transaction <hash|DRYRUN>`
      where `<amount>` is the value of output 0 formatted as `Amount.String`
      (shortest decimal DCR value followed by ` DCR`).
   "Reported" errors go to standard error with a newline and increment an
   error counter.
9. `published = groups - errors`. If non-zero print
   `Swept <total> to destination across <published> <transaction|transactions>`;
   the noun is chosen from the **error** count. If `errors > 0` return
   `failed to publish <errors> <transaction|transactions>` (standard error,
   status 1).

Wire-level details of the intended run:

- `listunspent` result members consumed: `spendable`, `confirmations`
  (int64, compared with `--minconf`), `account`, `address` (filter and
  grouping key), `txid`, `vout`, `tree`, `amount`. Outputs with an empty
  `address` all fall into one group keyed by the empty string.
- `getnewaddress` is sent with exactly one parameter (the destination
  account name) and its result is decoded as a JSON string. It is called
  from inside the change source with `context.Background()`, once per
  group whose swept value minus fee is not dust (the authoring code asks
  for the script only then, `wallet/txauthor/author.go:136-142`), so a new
  address is consumed per transaction, including on a dry run and
  including groups that later fail to sign
  (`cmd/sweepaccount/main.go:245-261`).
- Authoring errors arrive wrapped by
  `errors.E("txauthor.NewUnsignedTransaction", err)`
  (`wallet/txauthor/author.go:93,103-105,138-140`), so the reported line
  is `Failed to create unsigned transaction: txauthor.NewUnsignedTransaction: <text>`
  for input-source and change-source failures, and
  `Failed to create unsigned transaction: invalid operation: signed tx size exceeds allowed maximum`
  (no `Op`) when the estimated size exceeds `MaxTxSize`
  (`wallet/txauthor/author.go:122-124`). The `no input value` sentinel is
  still recognised through the wrapper by `errors.Is`.
- Two further authoring failures exist. When the group's total value is
  below the fee, the line is
  `Failed to create unsigned transaction: txauthor.NewUnsignedTransaction: insufficient balance`
  (`wallet/txauthor/author.go:107-109`; the first comparison uses the fee
  for an assumed single P2PKH input plus the 25-byte change output, later
  iterations the fee recomputed from the empty redeem-script-size list of
  Q22(c)). A change script longer than 520 bytes would give
  `Failed to create unsigned transaction: invalid operation: script size exceed maximum bytes pushable to the stack`
  (`wallet/txauthor/author.go:142-145`); no standard address produces one.
- When both `--destacct` and `--destaddr` blocks could run only one does
  (start-up check 12); the code nevertheless evaluates them as two
  independent `if` statements, not `if/else`
  (`cmd/sweepaccount/main.go:388-403`).
- `walletpassphrase` parameters: passphrase string, then the JSON integer
  `60`. `walletlock` and `listunspent` carry no `params` member.
- The `signrawtransaction` result is decoded into
  `types.SignRawTransactionResult` (`hex`, `complete`, `errors`); only
  `hex` and `complete` are used, the `errors` array is ignored.
- `sendrawtransaction` parameters: the `hex` string returned by the signer
  and the JSON boolean `false` (allow high fees); the result is decoded as
  a JSON string and printed verbatim.
- The validation of `--destaddr` happens only when the change source is
  asked for a script, i.e. after the passphrase prompt; a bad address makes
  every group fail with `Failed to create unsigned transaction: <err>`.
- The deferred `rpcClient.Close()` runs on every return from `sweep` after
  a successful dial.

### 5.10.1 Help and parse-error output of the two go-flags tools

Derived by reading `go-flags@v1.5.0` (`parser.go:136-138,156-168,205-213,695-714`,
`help.go:25-45,170-250,280-420`, `option.go:304-310,507-540`,
`convert.go:66-104`, `optstyle_other.go:10-12,57-66`); not confirmed by
running the Go binaries.

Both tools call `flags.Parse(&opts)`, i.e. a parser named
`path.Base(os.Args[0])` with options `HelpFlag | PrintErrors |
PassDoubleDash`. Consequences:

- `-h` or `--help` prints the help text followed by one extra newline to
  **standard output**; the tool then exits with status 1 (Q28).
- Any other parse error is printed to **standard error** followed by a
  newline, then exit status 1. Texts: ``unknown flag `<name>'`` (opening
  backquote, closing apostrophe; `<name>` without dashes) and
  ``expected argument for flag `<flag>'`` where `<flag>` is `-c, --connect`
  style for options with both forms and `--cafile` style otherwise
  (`go-flags@v1.5.0/parser.go:540,562,607,644`, `option.go:196-217`).
- A bare `--` ends option parsing; everything after it is positional and
  ignored by the tools.
- On Windows go-flags additionally accepts `/c` and `/connect:value` forms
  and prints help in that style (`go-flags@v1.5.0/optstyle_windows.go:12-14,97-104`).

Help layout (non-Windows): a `Usage:` line, a blank line, the group title
`Application Options:`, one line per option in struct order, a blank line,
`Help Options:` and the `-h, --help` line. Each option line is two spaces,
then `-X, ` or four spaces, then `--name`, then `=` for every non-boolean
option; the description starts at column `maxLongNameLength + 10` (20 for
`sweepaccount`, 22 for `repaircfilters`) and is word-wrapped to the
terminal width with continuation lines indented to the same column. The
terminal width is read with `TIOCGWINSZ` on file descriptor 0 and defaults
to 80 when that fails or reports 0 (`go-flags@v1.5.0/termsize.go:9-15`,
`help.go:66-68`). A non-boolean option whose value at parse time differs
from the Go zero value has ` (default: <value>)` appended to its
description (strings verbatim, integers in decimal, floats in Go
`strconv.FormatFloat(v, 'g', -1, 64)` form); booleans never show a default.

`sweepaccount --help` at 80 columns when the default certificate file does
not exist (so `--connect` and `--cafile` have no default):

```
Usage:
  sweepaccount [OPTIONS]

Application Options:
      --testnet     Use the test decred network
      --simnet      Use the simulation decred network
  -c, --connect=    Hostname[:port] of wallet RPC server
  -u, --rpcuser=    Wallet RPC username
  -P, --rpcpass=    Wallet RPC password
      --cafile=     Wallet RPC TLS certificate
      --feerate=    Transaction fee per kilobyte (default: 0.0001)
      --sourceacct= Account to sweep outputs from
      --sourceaddr= Address to sweep outputs from
      --destacct=   Account to send sweeped outputs to
      --destaddr=   Address to send sweeped outputs to
      --minconf=    Required confirmations to include an output (default: 2)
      --dryrun      Do not actually send any transactions but output what would
                    have happened

Help Options:
  -h, --help        Show this help message

```

`repaircfilters --help` under the same conditions:

```
Usage:
  repaircfilters [OPTIONS]

Application Options:
      --testnet       Use the test decred network
  -c, --connect=      Hostname[:port] of wallet RPC server
  -u, --rpcuser=      Wallet RPC username
  -P, --rpcpass=      Wallet RPC password
      --cafile=       Wallet RPC TLS certificate
      --cfiltersfile= Binary file with pre-dcp0005 filter data

Help Options:
  -h, --help          Show this help message

```

When the default certificate exists, the `--connect` description becomes
`Hostname[:port] of wallet RPC server (default: localhost)` and the
`--cafile` description `Wallet RPC TLS certificate (default: <appdata>/rpc.cert)`
with the real path, wrapped by the rule above. The help text therefore
depends on the file system state of the machine it runs on.

### 5.11 `treasurykey`

`cmd/treasurykey/treasurykey.go:34-69`. Uses the Go standard `flag` package
(single or double dash accepted): `-mainnet`, `-simnet`, `-regnet`,
`-testnet`, all boolean, default false, usage strings
`use mainnet parameters`, `use simnet parameters`, `use regnet parameters`,
`use testnet parameters`.

1. Parse flags (standard `flag` behaviour: an unknown flag prints an error
   and usage to standard error and exits 2; `-h` prints usage and exits 0).
2. Count the set flags. If the count is not exactly one, print
   `One and only one flag must be selected` to standard output, print the
   flag usage to standard error, exit 1.
3. Generate a random secp256k1 private key: 32 bytes from the system random
   source, redrawn until the value is in `[1, N-1]`
   (`dcrd/dcrec/secp256k1/v4@v4.4.1/privkey.go:52-71`).
4. Build a WIF with the network's `PrivateKeyID` and signature type
   `STSchnorrSecp256k1`.
5. Print the three lines of section 4.7. Exit 0. An error in 3 or 4 is
   printed with `fmt.Println` to standard output and the status is 1.

The usage text written to standard error in step 2 (and for `-h`, and
after the error line of an undefined flag) is the Go `flag` default
(`go1.27.4/src/flag/flag.go:607-650,706-709`): flags in lexical order, a
tab after four spaces before each usage string, no default shown for
false booleans. `<argv0>` is `os.Args[0]` unmodified:

```
Usage of <argv0>:
  -mainnet
    <TAB>use mainnet parameters
  -regnet
    <TAB>use regnet parameters
  -simnet
    <TAB>use simnet parameters
  -testnet
    <TAB>use testnet parameters
```

(`<TAB>` stands for one tab character.) An undefined flag first prints
`flag provided but not defined: -<name>` to standard error
(`go1.27.4/src/flag/flag.go:1115`). Boolean flags accept `-mainnet=false`
and `-mainnet=true`; a flag set to false does not count. If several are
set the count check fails; the parameter selection order in the source
(mainnet, testnet, simnet, regnet; `cmd/treasurykey/treasurykey.go:43-58`)
is therefore unobservable. Positional arguments are ignored. No key is
generated when the count check fails.

### 5.12 `rpc_example`

`rpc/rpc_example/main.go:18-123`. No flags. Constants: host
`localhost:9113`, user `user`, password `pass`, certificate
`<appdata>/rpc.cert`, testnet3 parameters.

1. Read the certificate; create a dcrd `rpcclient` websocket connection to
   endpoint `ws` on the host; wrap it with `RawRequestCaller`.
2. For each of three hex scripts (`rpc/rpc_example/main.go:99-106`: an
   `OP_TGEN`-prefixed P2PKH script labelled `TSPEND` in the source comment,
   a 2-of-2 multisig, and the single byte `01`) call
   `importscript`.
3. Call `getnewaddress "default"` and append the result to the list of 12
   hard-coded addresses (`rpc/rpc_example/main.go:108-123`, the first being
   the invalid string `abcd`).
4. For each address call `validateaddress` and print the result re-marshalled
   with two-space indentation.
5. Any error is printed to standard output and the status is 1; otherwise 0.

Further details (`rpc/rpc_example/main.go:37-97`):

- The connection configuration is `rpcclient.ConnConfig{Host, Endpoint:
  "ws", User, Pass, Certificates}` with every other field zero: websocket
  mode over TLS, the certificate bytes used as the only trusted root, no
  notification handlers (`nil`). The client is never shut down.
- The scripts are imported with `ImportScript` (one parameter, no rescan
  argument). The source comment warns that they stay imported in the
  wallet permanently. The three scripts are, in order:
  `c376a914d17043c104a57393aa7353e1510e39eab811e3db88ac`,
  `522103d484eb60ad03549e731ae9045281f8ee14ff6ea11b697f32cde3d8a18992261b210342b0b9c0ecb53cb9761beb0d010bbf08b5049d2a4d3bea5d3a1d95eb664931cb52ae`,
  `01`.
- `validateaddress` is issued through the embedded `Caller` (`client.Call`)
  with the raw address string, not through the typed `ValidateAddress`
  (which needs a decoded address and could not send `abcd`). The result is
  decoded into `types.ValidateAddressResult` and printed with
  `json.MarshalIndent(res, "", "  ")`, so members tagged `omitempty` that
  are zero are absent from the output and the member order is the Go
  struct order (`rpc/jsonrpc/types/results.go:372-389`).
- The 12 hard-coded strings, in order, with the source's own labels:
  `abcd` (invalid address);
  `TkKkYvSrnu8orwhtedcJGkD7guarvZUbUAtjr4iKqD9Y8pNEf8iHu` (PubKeyEcdsaSecp256k1V0);
  `Tsp18L8qTcjzigYXrD5GSdwDmhVYBpKmfUL` (PubKeyHashEcdsaSecp256k1V0);
  `TkKnVfd6EvzEYAqiELWstkASHgVyYH8JK3gNvAxUX79C9CrnsV8W6` (PubKeyEd25519V0);
  `Tead9n1wLBgaUR7AZrrtt6WWeDfetbF3dpy` (PubKeyHashEd25519V0);
  `TkKpSQoKgxqfDPyXp3RTWk7ktTR69zn19vU1zHCdD18r9bMTvDKT3` (PubKeySchnorrSecp256k1V0);
  `TSs3jHQMbbZGPyftUh4cgaALzgDZhfXGtxn` (PubKeyHashSchnorrSecp256k1V0);
  `TcvVou7ooM4rJRWNeJYwehJ9fQq1HTc5pbK` (ScriptHashV0);
  `020000000000000000000000000000000000000000000000000000000000000001`
  (a raw compressed public key, labelled "PubKeyEcdsaSecp256k1V0 by pubkey");
  then the script-hash addresses of the three imported scripts:
  `TcfdqCrK2fiFJBZnGj5N6xs6rMsbQBsJBYf`, `TcrzaAVMbFURm1PpukWru8yE2uBTjvQePoa`,
  `TckSpBht36nMZgnDDjv7xaHUrgCyJpxQiLA`.
  These are a ready-made input set for a `validateaddress` parity test on
  testnet3 (every address kind, an invalid string, a bare public key and
  owned script addresses); the expected outputs are not in the repository
  and must come from a Go oracle run.
- `run` creates a cancellable context that is cancelled only on return;
  `main` calls `os.Exit(0)` explicitly on success. A hex-decoding failure
  of one of the three script constants would be returned like any other
  error (`rpc/rpc_example/main.go:29-35,56-68`). The global
  `validateAddrs` slice is appended to in place.
- The server answers `validateaddress` for an undecodable string with a
  result (`isvalid` false) rather than an error
  (`internal/rpc/jsonrpc/methods.go:5313-5319`), so the loop reaches all 13
  entries; any RPC error aborts the program at that entry.

### 5.13 `kdf` parameter generation and derivation

1. `NewArgon2idParams(rand)`: `ncpu = min(NumCPU(), 256)`; `Time = 1`;
   `Memory = 262144`; `Threads = uint8(ncpu)` (256 truncates to 0, quirk
   Q9); one `rand.Read` into the 16-byte salt.
2. `DeriveKey(password, p, len)`: Argon2id as in section 4.3; a garbage
   collection is forced when the function returns to release the 256 MiB
   working memory promptly.

## 6. Constants, limits and defaults

| Name | Value | Source | What depends on it |
|---|---|---|---|
| word list size | 512 words | `pgpwordlist/wordlist.go:33-544` | mnemonic encode/decode |
| checksum length | 1 byte, `SHA256(SHA256(seed))[0]` | `walletseed/seed.go:38-41` | last mnemonic word |
| `hdkeychain.MinSeedBytes` | 16 | `dcrd/hdkeychain/v3@v3.1.3/extendedkey.go:40` | seed acceptance |
| `hdkeychain.MaxSeedBytes` | 64 | `dcrd/hdkeychain/v3@v3.1.3/extendedkey.go:44` | seed acceptance |
| `hdkeychain.RecommendedSeedLen` | 32 | `dcrd/hdkeychain/v3@v3.1.3/extendedkey.go:30` | default generated seed (33 words) |
| `ErrInvalidSeedLen` text | `seed length must be between 128 and 512 bits` | `dcrd/hdkeychain/v3@v3.1.3/extendedkey.go:79-80` | error messages |
| `errors.Separator` (library default) | `":\n\t"` (colon, newline, tab) | `errors/errors.go:27` | tests, tools, any non-daemon embedding |
| `errors.Separator` (daemon) | `":: "` (colon, colon, space) | `dcrwallet.go:36-39` | every nested error message in logs and RPC responses |
| field separator inside one level | `": "` | `errors/errors.go:262,279` | error messages |
| empty-error text | `unclassified error` | `errors/errors.go:286-288` | error messages |
| `E` no-argument panic text | `errors.E: no args` | `errors/errors.go:189` | programming error only |
| number of kinds | 21 (0..20) | `errors/errors.go:53-75` | section 4.2 |
| `kdf.MarshaledLen` | 25 | `kdf/kdf.go:42` | stored record length |
| Argon2id salt length | 16 bytes | `kdf/kdf.go:19` | stored record |
| Argon2id default time | 1 | `kdf/kdf.go:33` | new unique-passphrase accounts |
| Argon2id default memory | 262144 KiB (256 MiB) | `kdf/kdf.go:34` | new unique-passphrase accounts; RAM needed to unlock |
| Argon2id default threads | `uint8(min(NumCPU, 256))` | `kdf/kdf.go:31,35` | new unique-passphrase accounts |
| derived key length | 32 bytes (chosen by the caller) | `wallet/udb/addressmanager.go:156-158` | account key encryption |
| `lru` effective capacity | `limit + 1` | `lru/cache.go:48`, `lru/map.go:53` | inventory and seen-message de-duplication |
| SPV seen-transaction / seen-mix-message limit | 2000 each | `spv/sync.go:148-149` | owned by spec 14 |
| `DCP0005ActiveHeightTestNet3` | 323328 | `validate/validate.go:21` | cfilter commitment checks, cfilter import range |
| `DCP0005ActiveHeightMainNet` | 431488 | `validate/validate.go:25` | same |
| pre-DCP0005 cfilter set hash, testnet3 | `619c08f5adda6f834212bbdaee3002fdc4efed731477af6c0fed490bbe2488d0` | `validate/validate.go:112` | `validatepredcp0005cf`, cfilter repair |
| pre-DCP0005 cfilter set hash, mainnet | `f95e09f9ded38f8d6c32e5158a1f286633881393659218c63f5ab0fc86b36c83` | `validate/validate.go:114` | same |
| `version.Major.Minor.Patch` | 2.2.0 | `version/version.go:21-23` | version string, `version` RPC |
| `version.Integer` | 2020000 | `version/version.go:27` | `getinfo` |
| `version.PreRelease` default | `pre` | `version/version.go:33` | version string |
| semver alphabet | `0-9A-Za-z-.` | `version/version.go:17` | version string normalization |
| VCS revision truncation | 9 characters (git only) | `version/version.go:63-65` | build metadata |
| tool default RPC host | `localhost` | `cmd/sweepaccount/main.go:65`, `cmd/repaircfilters/main.go:44` | tools |
| tool default RPC ports | mainnet 9110, testnet3 19110, simnet 19557 | `cmd/sweepaccount/main.go:98-109`, `cmd/repaircfilters/main.go:80-91` | tools (match `internal/netparams/params.go`) |
| tool default certificate | `<appdata>/rpc.cert` | `cmd/sweepaccount/main.go:68`, `cmd/repaircfilters/main.go:47` | tools |
| `sweepaccount` default fee rate | 0.0001 DCR/kB (`txrules.DefaultRelayFeePerKb` = 10000 atoms) | `cmd/sweepaccount/main.go:69`, `wallet/txrules/rules.go:38` | tools |
| `sweepaccount` fee rate bounds | must satisfy `1e-6 <= rate <= 1` DCR/kB | `cmd/sweepaccount/main.go:152-157` | tools |
| `sweepaccount` default `--minconf` | 2 | `cmd/sweepaccount/main.go:74` | tools |
| `sweepaccount` unlock timeout | 60 seconds | `cmd/sweepaccount/main.go:413` | `walletpassphrase` argument |
| `sweepaccount` change script size | 25 bytes | `cmd/sweepaccount/main.go:263-265,283-285` | fee estimate |
| `repaircfilters` buffer | 65536 bytes | `cmd/repaircfilters/main.go:188` | batch size, maximum record length |
| wsrpc write timeout | 10 s | `wsrpc/v2@v2.3.8/rpc.go:33` | tools |
| wsrpc ping period / pong wait | 60 s / 66.67 s | `wsrpc/v2@v2.3.8/rpc.go:201-202` | tools |
| `MaxTxSize` | mainnet 393216; testnet3, simnet, regnet 1000000 | `chaincfg/v3@v3.3.0/*params.go` | tool transaction size checks |
| `dcrutil.MaxAmount` | 21e14 atoms | `dcrutil/v4@v4.0.3/const.go:16` | tool input sanity check |
| WIF signature type written by `treasurykey` | 2 (`STSchnorrSecp256k1`) | `cmd/treasurykey/treasurykey.go:21-22`, `dcrec@v1.0.1` | treasury key import |
| CI Go versions | 1.26, 1.27 | `.github/workflows/go.yml:12` | support policy |
| `go.mod` language version | `go 1.26.0` | `go.mod:3` | minimum toolchain |
| `rpc_example` host | `localhost:9113` | `rpc/rpc_example/main.go:19` | example only |
| `rpc_example` credentials | user `user`, password `pass`, endpoint `ws`, network testnet3 | `rpc/rpc_example/main.go:20-26,44` | example only |
| `chainhash.MaxHashStringSize` | 64 hex digits; shorter strings are zero-padded | `dcrd/chaincfg/chainhash@v1.0.5/hash.go:98-129` | `txid` parsing in the tools and the typed client |
| WIF decoded length / maximum string length | 39 bytes / 54 characters | `dcrutil/v4@v4.0.3/wif.go:113-124` | `treasurykey` output, `DumpPrivKey` decoding |
| `sign.sh` file mode | `0755` | `cmd/movefunds/main.go:232` | `movefunds` output |
| `PurchaseTicket` parameter capacity | 1 required + up to 5 optional positions sent (slice capacity 9 is only an allocation hint) | `rpc/client/dcrwallet/methods.go:262-284` | typed client |
| `addressArrayMarshaler` buffer hint | `2 + n*38` bytes | `rpc/client/dcrwallet/marshaling.go:31` | allocation only, not observable |
| go-flags help width | terminal columns of file descriptor 0, default 80 | `go-flags@v1.5.0/termsize.go:9-15`, `help.go:66-68` | help text wrapping of the two flag-based tools |
| go-flags help description column | longest long name + 10 (20 `sweepaccount`, 22 `repaircfilters`) | `go-flags@v1.5.0/help.go:25-45,190` | help text layout |
| authored transaction version / lock time / expiry | 1 / 0 / 0 | `wallet/txauthor/author.go:28,126-133` | tool transactions |
| tool input sequence, block height, block index | `0xffffffff`, `0x00000000`, `0xffffffff` | `dcrd/wire@v1.8.0/msgtx.go:338-347` | tool transactions |
| `repaircfilters` wsrpc option slice capacity | 5 (3 used) | `cmd/repaircfilters/main.go:169-178` | allocation hint only |
| imported-account label | `imported` | `wallet/udb/addressmanager.go:50`, `cmd/treasurykey/README.md:39` | `importprivkey` of a treasury key |
| CI action pins | `actions/setup-go@b7ad1dad31e06c5925ef5d2fc7ad053ef454303e`, `actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1` | `.github/workflows/go.yml:15,19` | supply-chain policy (section 11) |
| Nix dev-shell packages | `go_1_25`, `protobuf` | `flake.nix:20-23` | contributor tooling only |

## 7. Concurrency and lifecycle

- **`errors`**. `Separator` is a plain package variable. It is written once,
  in `package main`'s `init`, before any goroutine exists, and read on every
  `Error()` call. `*Error` values are never mutated after `E`/`WithStack`
  return. `E` allocates one `Error` per call. There is no global registry.
- **`pgpwordlist`**. The index map is filled in `init` and read-only
  afterwards; lookups are safe from any goroutine.
- **`walletseed`**. Stateless. Randomness comes from the process-wide
  cryptographic source used by `hdkeychain`.
- **`kdf`**. `DeriveKey` blocks the calling goroutine for the full Argon2id
  computation, allocates `Memory` KiB, and runs `Threads` worker goroutines
  inside `x/crypto`. It is not cancellable. The forced garbage collection on
  return is a Go memory-footprint measure with no observable protocol
  effect.
- **`lru`**. One mutex per instance, held for the duration of each method.
  `Add` and `Contains` are safe to call concurrently; there is no iteration
  API, so callers never observe ordering other than through eviction.
- **`version`**. `BuildMetadata` is finalised in the package `init`;
  packages that capture `version.String()` in a package-level variable
  (`p2p/peering.go:41`) rely on Go initialising imported packages first.
- **`validate`**, **`assets`**. Stateless.
- **Typed client**. Holds no state besides the `Caller` and network
  parameters; concurrency safety is that of the supplied `Caller`.
- **Tools**. Single-threaded apart from the two goroutines `wsrpc` starts per
  connection (reader and writer/pinger). All calls use
  `context.Background()`: there are no per-call timeouts and no signal
  handling; an interrupt terminates the process without closing the
  websocket cleanly. `sweepaccount` processes groups sequentially and, within
  a group, strictly orders `walletpassphrase`, `signrawtransaction`,
  `walletlock`, `sendrawtransaction`. `repaircfilters` sends batches in file
  order and waits for each response before reading further. `sweepaccount`
  uses the default pong deadline: the connection is dropped when no pong
  arrives within 66.67 s of the previous pong (126.67 s after connecting if
  none ever arrives); only pongs extend the deadline, ordinary responses do
  not. `repaircfilters` disables the deadline.

## 8. Errors

### 8.1 Library packages

| Origin | `Op` | Kind | Leaf text |
|---|---|---|---|
| `pgpwordlist.DecodeMnemonics` | `pgpwordlist.DecodeMnemonics` | `Encoding` | `word <w> is not in the PGP word list` |
| `pgpwordlist.DecodeMnemonics` | same | `Encoding` | `word <w> is not valid at position <n>, check for missing words` |
| `walletseed.GenerateRandomSeed` | `walletseed.GenerateRandomSeed` | `Invalid` | `seed length must be between 128 and 512 bits` |
| `walletseed.GenerateRandomSeed` | same | `Other` | random source failure text |
| `walletseed.DecodeUserInput` | `walletseed.DecodeUserInput` | `Encoding` | hex error, nested mnemonic error, `checksum mismatch`, or the seed length text (section 5.3) |
| `kdf.UnmarshalBinary` | none | none | `invalid marshaled Argon2id parameters` |
| `validate.*` | section 3.6 | `Consensus` or none | section 3.6 |

`lru`, `version` and `assets` produce no errors. `kdf.DeriveKey` can panic
inside the Argon2 implementation on zero `Time` or `Threads`.

### 8.2 How errors surface

- **JSON-RPC**: the handler's error is converted by `convertError`
  (`internal/rpc/jsonrpc/errors.go:16-49`, spec 16): the code comes from the
  outermost non-`Other` kind (table in 4.2, default `ErrRPCWallet`), the
  message is `err.Error()`, i.e. the full rendered chain with `":: "`
  separators and every `Op` name.
- **gRPC**: `translateError` builds a status whose message is `err.Error()`
  and whose code comes from `errorCode`
  (`internal/rpc/rpcserver/server.go:80-136`). The seed service bypasses it
  and always uses `InvalidArgument` (`server.go:3185,3202`).
- **Logs**: errors are logged with `%v`, giving the same rendered text.
- **Interactive prompts**: `Input error: <rendered text>`.
- **Tools**: `fatalf` to standard error with exit status 1
  (`repaircfilters`, `sweepaccount`); standard output with exit status 0
  (`movefunds`); standard output with exit status 1 (`treasurykey`,
  `rpc_example`). A JSON-RPC error received by `wsrpc` renders as its
  `message` only (`wsrpc/v2@v2.3.8/rpc.go:42`); the code is not shown.

### 8.3 Typed client and tool-internal errors

- **Typed client.** The client adds no error wrapping, `Op` or `Kind` of
  its own. A method returns, unchanged, whichever comes first: the
  argument-marshalling error (`RawRequestCaller` only), the `Caller`'s
  error (transport or JSON-RPC error object), the `encoding/json` error
  from decoding the result, or the conversion error from the `unmarshal*`
  helper (`chainhash`, `stdaddr`, `hdkeychain`, `dcrutil`, `wire` or
  `encoding/hex` error values). The only error type the package defines is
  `SignatureErrors` (section 3.9). Methods return their (possibly partly
  filled or zero) result value together with a non-nil error; callers must
  test the error first.
- **Tool input sources.** `no input value` is a sentinel
  (`errNoInputValue`) compared with `errors.Is` in `sweepaccount`
  (`cmd/sweepaccount/main.go:406`); `movefunds` declares the same sentinel
  but never tests for it, so there it would be printed like any other
  authoring failure. The other three input-source messages are built with
  `fmt.Errorf` and are distinguishable only by text.
- **Tool exit statuses, complete list.** `repaircfilters` and
  `sweepaccount`: 0 on success, 1 for every validation, parse, help or
  run-time failure. `movefunds`: 0 for every handled outcome, 2 for the Go
  panic described in 5.8. `treasurykey`: 0 on success and for `-h`, 1 for
  a wrong flag count or a key-generation error, 2 for an undefined flag
  (Go `flag` package `ExitOnError`). `rpc_example`: 0 or 1.

## 9. Compatibility-critical behaviours

1. **Kind strings.** The 21 strings of section 4.2, byte for byte (note the
   capitalisation of `I/O error` and `Decred network is unreachable`), plus
   `unclassified error` for an all-empty error. They appear inside JSON-RPC
   `error.message` and gRPC status messages that Decrediton, dcrctl users and
   scripts read and sometimes match.
2. **Error rendering.** The algorithm of 5.5 with `": "` between fields of
   one level and `":: "` between levels in the daemon. Together with the
   promotion and collapse rules of `E` (5.4) this decides which `Op` names
   and kind strings appear, and in what order, in every wallet error message
   sent to clients. A port that keeps kinds but renders differently changes
   client-visible text for essentially every failing RPC.
3. **`Op` names are part of the message.** Every `const op errors.Op = "..."`
   and every `Opf` format in the code base is client-visible when its error
   reaches an RPC boundary. Each subsystem spec must list its `Op` strings;
   this spec defines only how they are combined.
4. **Kind selection for code mapping.** The JSON-RPC and gRPC code is derived
   from the outermost non-`Other` kind in the chain (5.6), including kinds
   promoted by `E` from wrapped errors and kinds reached through
   `Errorf("%w")` wrappers and `Join` trees.
5. **PGP word list and parity.** The exact 512 words in the exact order;
   output in canonical capitalisation; even positions use even indexes, odd
   positions odd indexes. Seeds written down by users of dcrwallet and
   Decrediton (33 words for the default 32-byte seed) must decode to the same
   bytes, and seeds shown by the replacement must be accepted by the Go
   implementation.
6. **Checksum word.** First byte of double SHA-256 over the seed bytes,
   encoded at position `n` with that position's parity.
7. **`DecodeUserInput` acceptance.** One token means hex (both cases, even
   length); two or more tokens mean mnemonic, case-insensitive, tolerant of
   repeated spaces and surrounding white space; result length 16..64 bytes;
   checksum mandatory for mnemonics and absent for hex. The four error
   message families of 5.3 are returned verbatim by gRPC
   `SeedService.DecodeSeed` with code `InvalidArgument`.
8. **`EncodeMnemonic` output.** Single spaces, no trailing newline; returned
   by gRPC `GenerateRandomSeed` (the `SeedMnemonic` response field) and written verbatim to the
   simnet `seed` file (`walletsetup.go:287-288`).
9. **Argon2id record.** The 25-byte layout of 4.3 inside existing wallet
   databases, and Argon2id version 0x13 with the stored time, memory and
   lane count, 32-byte output. A wallet whose account was encrypted on a
   64-thread machine stores `Threads = 64`; the replacement must derive with
   64 lanes regardless of the local CPU count. New records should use the Go
   defaults (time 1, memory 262144); any thread count from 1 to 255 is
   readable by the Go implementation, which always derives from the stored
   values.
10. **`lru` semantics** where a peer can observe them: capacity `limit+1`,
    no reordering on `Contains`, promotion on re-`Add`. These decide which
    inventory hashes the wallet considers already sent to or received from a
    peer (spec 14 owns the limits and call sites).
11. **`validate` constants.** The two activation heights, the two cfilter
    set hashes, the rule that networks other than mainnet and testnet3 always
    verify the commitment, and the error texts of 3.6 (they surface through
    sync errors and the `validatepredcp0005cf` RPC).
12. **Version reporting.** `Integer = 2020000` semantics
    (`1000000*major + 10000*minor + 100*patch`), the string grammar of 3.7
    and the normalisation alphabet; consumers are `getinfo`, the `version`
    RPC, the P2P user agent and the start-up log line. The values the
    replacement reports are a project decision (section 13), the format is
    not.
13. **Request shapes that existing clients send** (from the tools and the
    typed client; the server must accept all of them): requests with no
    `params` member; `"jsonrpc":"2.0"` with numeric ids; JSON `null` as a
    placeholder for an omitted optional positional parameter
    (`listsinceblock null <minconf>`, `purchaseticket` gaps,
    `signrawtransaction <hex> null null "ssgen"`); amounts as JSON numbers in
    DCR; `lockunspent` inputs without an `amount` member; object member names
    in a different letter case from the server's struct tags (for example
    `scriptpubkey`/`redeemscript` in `sign.sh` versus `scriptPubKey`/
    `redeemScript`), which Go's JSON decoder matches case-insensitively;
    `importcfiltersv2 <int32 height> [<hex>, ...]` in batches whose raw
    filter data totals up to 64 KiB (about 128 KiB of hex in one request); `walletpassphrase <string> 60`;
    `sendrawtransaction <hex> false`.
14. **If the tools are shipped** (section 13): flag names and short forms,
    the default certificate path and ports, message texts, output streams and
    exit statuses of 5.8 to 5.11; the `treasurykey` line format (operators
    paste the public key into consensus parameters and the WIF into
    `importprivkey`); the `repaircfilters` input format and its two result
    lines.
15. **WIF produced by `treasurykey`** must import with `importprivkey` on
    both implementations: 2-byte network id, type byte 2, 32-byte key, 4-byte
    BLAKE-256 checksum, base58.
16. **Documented operator command lines must keep working against the
    replacement** (they are the only published usage of these RPCs outside
    the help text): `dcrctl --wallet importprivkey <WIF> imported false`
    for a Schnorr-typed (type byte 2) WIF and for the ECDSA-typed (type
    byte 0) WIF shown in the README (`cmd/treasurykey/README.md:31-39`);
    `dcrctl --wallet listunspent` producing a JSON array that parses as
    `ListUnspentResult` objects; `accountaddressindex <account> <0|1>`
    returning a bare integer and `accountsyncaddressindex <account> <0|1>
    <index>` accepting it back; `dcrctl <args> signrawtransaction <hex>
    '<inputs json>'` returning an object with a `hex` member; and
    `sendrawtransaction <hex>` (`docs/offline_wallets.md:52-98`).
17. **Wallet daemon idling at genesis.** The offline-signing procedure
    depends on the wallet being able to sign with `signrawtransaction`
    while its node peer has no blocks beyond genesis and after address
    indexes were set by `accountsyncaddressindex`
    (`docs/offline_wallets.md:76-92`). Signing must not require a synced
    chain.
18. **`validateaddress` on garbage input returns a result, not an error**
    (`internal/rpc/jsonrpc/methods.go:5313-5319`); `rpc_example` relies on
    it for its first entry (`rpc/rpc_example/main.go:109`). Owned by the
    JSON-RPC spec; recorded here because this example is its only
    in-repository consumer.

## 10. Quirks

Q1. **The separator is set by `package main`, not by the library.** Library
users, unit tests and the `cmd/` tools render nested errors with
`":\n\t"`; the daemon renders them with `":: "` (`errors/errors.go:27`,
`dcrwallet.go:36-39`). Upstream tests that compare rendered multi-level
errors see the multi-line form.

Q2. **A kind can outlive the error it came from.** In `E`, an `*Error`
argument copies its kind into the new error immediately
(`errors/errors.go:202-207`). If a later string or error argument replaces
`Err`, the kind stays: `E(E(Invalid, "abc"), "def")` is `{Invalid, "def"}`
(pinned by `errors/errors_test.go:72-81`). With two `*Error` arguments the
first one's kind wins and the second one becomes `Err`.

Q3. **`E` silently ignores arguments of unknown type**, including a nil
`error` interface (`errors/errors.go:194-211`). `E(op, err)` with `err ==
nil` returns a non-nil error that renders as the `Op` alone. Callers must
check for nil before wrapping; the port must not turn this into "returns
nil".

Q4. **Typed-nil `*Error` arguments panic.** `E((*Error)(nil))` dereferences
the nil pointer when the kind is still unset (`errors/errors.go:204-205`),
and an `Error` whose `Err` holds a typed-nil `*Error` panics in `Error()`
(`errors/errors.go:271-276`). No call site at the pin does this.

Q5. **`match` can panic.** When the template's `Err` is a non-`*Error` error
and the candidate's `Err` is nil, step 7 calls `Error()` on a nil interface
(`errors/errors.go:385`). Example: `Is(E(Op("x"), Invalid), E("text"))`.
Step 5 uses Go interface equality, which panics when both sides hold the
same uncomparable dynamic type such as a slice-based error
(`errors/errors.go:376`). Neither occurs in the code base; a Rust port should
return false in both cases and record the divergence.

Q6. **`Kind.As` documentation versus behaviour.** The comment says `Other`
never matches, but the standard library assigns a `Kind` to a `*Kind` target
by type before calling the `As` method, so `As(errors.Other, &k)` is true
(`errors/errors.go:130-149`, `go1.27.4/src/errors/wrap.go`). `Other` is never
used as an error value at the pin (0 references outside the package).

Q7. **Leaf `*Error` values unwrap to their kind.** `Unwrap` returns the
`Kind` as an `error` when `Err` is nil (`errors/errors.go:295-303`), so the
chain of `E(op, NotExist)` is `*Error -> Kind`. `Is(err, NotExist)` succeeds
through either node.

Q8. **Suppression of repeated fields is relative to the last written
value**, and a level that contributes nothing still receives the separator
before it (`errors/errors.go:253-283`). Such chains cannot be built with `E`
(it collapses them) but can with struct literals; tests construct
`&errors.Error{Kind: ...}` templates directly
(`wallet/udb/addressmanager_test.go:515-646`).

Q9. **Argon2 thread count can be zero.** `uint8(min(NumCPU, 256))` is 0 on a
machine with 256 or more logical CPUs (`kdf/kdf.go:31,35`); deriving a key
with such parameters panics with `argon2: parallelism degree too low`
(`x/crypto@v0.57.0/argon2/argon2.go:109-111`). The intended cap was
presumably 255. A port should clamp to 255 and record the divergence, and
must still read any stored value.

Q10. **The KDF cost is machine dependent and unvalidated.** `Threads` is
whatever the creating machine had; `UnmarshalBinary` accepts any 25 bytes
(`kdf/kdf.go:56-65`), so a corrupted record with zero time or threads panics
at unlock instead of returning an error, and a record with a huge memory
value attempts the allocation.

Q11. **`NewArgon2idParams` issues a single `Read`** and returns the
parameters even when the read fails (`kdf/kdf.go:37-38`); a short read
without error would leave a partly zero salt. Both call sites
(`wallet/udb/addressmanager.go:1745,2196`) pass the dcrd `crypto/rand`
reader, which fills the buffer.

Q12. **`lru` holds `limit+1` entries** (`lru/cache.go:48`,
`lru/map.go:53`), `Map.Add` on an existing key keeps the old value
(`lru/map.go:46-50`), the type comments say "least-recently-updated" and
`Map.Contains` is documented as "It does modify the priority" while it does
not (`lru/map.go:94-101`). `lru.Map` is unused at the pin.

Q13. **Mnemonic tokens are split on the space character only**
(`walletseed/seed.go:73`). A mnemonic pasted with newlines or tabs between
words is either a single token (hex error) or contains tokens with embedded
white space, giving `word a<newline>b is not in the PGP word list`. The
interactive prompt avoids this by joining lines with spaces first; gRPC
callers get the raw behaviour.

Q14. **A one-word input is always treated as hex**, so `aardvark` yields an
`encoding/hex` error rather than a mnemonic error
(`walletseed/seed.go:76-82`).

Q15. **Case folding is Go's simple Unicode lower-casing**
(`pgpwordlist/pgpwordlist.go:46`, `go1.27.4/src/strings/strings.go`
`ToLower`): U+212A KELVIN SIGN folds to `k` and U+0130 folds to `i`, so
`İstanbul` matches `Istanbul`. Rust's `str::to_lowercase` maps U+0130 to two
code points and would not match. Use ASCII folding plus an explicit decision
on these two code points, or a per-character simple mapping.

Q16. **The seed-length error has two kinds.** `GenerateRandomSeed` wraps
`ErrInvalidSeedLen` as `Invalid`, `DecodeUserInput` as `Encoding`
(`walletseed/seed.go:24,27,99`). The first of the two range checks in
`GenerateRandomSeed` (`size >= 255`) is dead code given the second.

Q17. **`EncodeMnemonic` of an empty seed starts with a space**
(`walletseed/seed.go:63-65`); neither encoder validates the length.

Q18. **`version`**: `vcs.modified` is ignored, so a dirty tree reports the
clean revision; only git revisions are truncated; `go install pkg@version`
builds have no build metadata (`version/version.go:46-67`). The README still
tells users to install the latest release from the `v4` module path
(`README.md:100`).

Q19. **`validate.CFilterV2HeaderCommitment` skips nothing on simnet and
regnet** and skips everything below the activation height on mainnet and
testnet3 (`validate/validate.go:80-89`). `PreDCP0005CFilterHash` formats an
unknown network as a decimal number and returns plain errors without kind
(`validate/validate.go:107-130`).

Q20. **`assets.Path` is dead code** and only meaningful on the build host
(`assets/assets.go:10-25`).

Q21. **`movefunds` is broken at the pin.** It passes a nil `ChangeSource`,
which `NewUnsignedTransaction` dereferences unconditionally
(`cmd/movefunds/main.go:189`, `wallet/txauthor/author.go:97`). Independent
of that: handled errors are printed without newline and exit 0; `txfee` is
converted as DCR (the sample value is ten million DCR,
`cmd/movefunds/config.json:2`, `cmd/movefunds/main.go:173`); the generated
`scriptpubkey` member carries the (empty) signature script
(`cmd/movefunds/main.go:223-224`); `docs/offline_wallets.md:32-40` still
describes a GOPATH and `dep` based install.

Q22. **`sweepaccount` is broken at the pin** in three independent ways:
(a) it dials `host:port` with no `wss://` scheme and no `/ws` path
(`cmd/sweepaccount/main.go:322` versus `cmd/repaircfilters/main.go:177`);
(b) it passes a `*wire.MsgTx` to `signrawtransaction`, which serialises as a
JSON object instead of a hex string (`cmd/sweepaccount/main.go:420`);
(c) `redeemScriptSizes` is allocated but never filled
(`cmd/sweepaccount/main.go:189,231`), so the fee is computed for a
transaction with zero inputs: estimated size 51 bytes, fee
`feeRate*51/1000` (510 atoms at the default rate) regardless of the number
of inputs. Further oddities: the summary noun is chosen from the error
count; groups skipped for `no input value` are counted as published
(`cmd/sweepaccount/main.go:405-410,452`); if the swept amount minus the fee
is dust there is no output and `TxOut[0]` panics
(`cmd/sweepaccount/main.go:446`); the change script size is fixed at 25 even
for script-hash destinations; a dry run still unlocks the wallet and signs;
processing order follows Go map iteration. The wallet's own `sweepaccount`
JSON-RPC method supersedes the tool.

Q23. **`repaircfilters`**: the read-error check tests the wrong variable
(`err` instead of `ioErr`, `cmd/repaircfilters/main.go:193-195`), so non-EOF
read errors are ignored and can spin forever; record lengths 65534 and
65535 overflow the `uint16` arithmetic used for slicing (panic) or never fit
the buffer (spin) (`cmd/repaircfilters/main.go:204-211`); the README shows
`-p PASSWORD` but the flag is `-P` and says "Requires go 1.13"
(`cmd/repaircfilters/README.md:25,33`); giving `--cafile` alone is not enough
when the default certificate is absent because the `--connect` default was
cleared; help exits with status 1; an empty file prints `Height: -1`;
`walletPort` handles simnet although there is no `--simnet` flag.

Q24. **`treasurykey`**: the WIF checksum is a single BLAKE-256 although the
dcrutil comment describes double SHA-256
(`dcrutil/v4@v4.0.3/wif.go:94-95` versus `178-179`); the "one flag" message
goes to standard output while the usage text goes to standard error.

Q25. **Typed client drift.** `ListReceivedByAccount*` call
`listreceivedbyacccount`, and `AddTicket` calls `addticket`; neither name is
in the server's handler table (`rpc/client/dcrwallet/methods.go:492,505,516,694`,
`internal/rpc/jsonrpc/methods.go:85-200`), so both fail with a
method-not-found error. `PurchaseTicket` sends a boolean and a number in
positions where the server expects `comment` (string) and `dontsigntx`
(bool) (`rpc/client/dcrwallet/methods.go:258-287`,
`rpc/jsonrpc/types/methods.go:666-673`). `GetRawChangeAddress` has an unused
parameter. `VerifyMessage`'s comment claims an unlocked wallet is needed.

Q26. **`rpc_example` targets port 9113**, which is not a dcrwallet default
on any network (`rpc/rpc_example/main.go:19`).

Q27. **Tooling documentation drift.** `flake.nix:21` provides Go 1.25 while
`go.mod:3` requires 1.26.0; `debug.go:13-15` pins `GODEBUG` defaults to
go1.25 under a `go1.25` build tag; `README.md:77` says Go 1.26 or 1.27.

Q28. **go-flags help handling in the tools**: `-h` prints the help to
standard output and the tool then exits with status 1
(`go-flags@v1.5.0/parser.go:702-714`, `cmd/sweepaccount/main.go:120-123`).

Q29. **The `treasurykey` README example is stale.** Its WIF carries
signature type byte `0x00` (ECDSA) while the code writes `0x02` (Schnorr)
(`cmd/treasurykey/README.md:34`, `cmd/treasurykey/treasurykey.go:21-22`;
decoded in section 4.7). The README also says the public key "will be
hardcoded inside `dcrd` and `dcrwallet`" (`cmd/treasurykey/README.md:21-22`)
and recommends generating at least two keys. Unknown WIF type bytes are
accepted by `dcrutil.DecodeWIF` without error
(`dcrutil/v4@v4.0.3/wif.go:139-160`).

Q30. **Typed client returns data together with errors and has uneven
`null` handling.** A `null` hash result becomes the zero hash with a nil
error; a `null` amount becomes 0; a `null` or absent `hex` member leaves
the returned transaction nil with a nil error; a `null` address is an
error (section 4.8). `unmarshalListAccounts` fills the caller's map in Go
map order and stops at the first bad amount. `SignRawSSGenTx` discards
signature errors. `SendMany` can silently merge two entries whose
addresses stringify identically
(`rpc/client/dcrwallet/marshaling.go:95-234`,
`rpc/client/dcrwallet/methods.go:227-253,972-982`).

Q31. **Address arrays are written without JSON escaping**
(`rpc/client/dcrwallet/marshaling.go:24-46`). This is safe only because
every `stdaddr.Address.String()` is base58 or hex; a custom `Address`
implementation returning a quote or backslash would produce invalid JSON.
By contrast transaction hex and outpoint strings go through
`json.Marshal`.

Q32. **`sweepaccount` consumes a destination address per transaction even
on a dry run**, validates `--destaddr` only after prompting for the
passphrase, and reports authoring failures with the
`txauthor.NewUnsignedTransaction: ` prefix of the wallet error model
rendered with the library separator (section 5.10). Its `errContext`
helper flattens causes with `%v`, so nothing printed by either flag-based
tool is unwrappable or carries a JSON-RPC code.

Q33. **`repaircfilters` contains an unreachable error message**
(`cfiltersfile read error`, `cmd/repaircfilters/main.go:193-195`) and its
README example would talk to a testnet port with mainnet parameters
selected (harmless: the network only chooses the default port)
(`cmd/repaircfilters/README.md:33-34`).

Q34. **`docs/offline_wallets.md` is partly obsolete**: GOPATH/`dep`
install path `github.com/dcrwallet/cmd/movefunds`
(`docs/offline_wallets.md:33-40`), a tool that panics at the pin (Q21), and
a final `dcrctl sendrawtransaction` without `--wallet`
(`docs/offline_wallets.md:98`). The procedure itself (watch-only listing,
address-index synchronisation, offline `signrawtransaction`) is still the
documented cold-wallet flow and is what compatibility item 16 protects.

Q35. **CI action comments disagree in style, not substance**: the
`setup-go` pin is annotated `#7.0.0` and the `checkout` pin `#v7.0.1`
(`.github/workflows/go.yml:15,19`); only the commit hashes are
authoritative. Go is installed before the source is checked out, so
`setup-go` cannot read `go.mod` and the matrix value alone selects the
toolchain.

Q36. **README statements that are claims, not code** (to be confirmed or
refuted by the owning specs; recorded here because the README is in this
scope): SPV wallets cannot vote, cannot revoke tickets before expiry and
cannot determine exact live/missed ticket counts (`README.md:26-36`);
switching between dcrd-RPC and SPV mode needs only a restart
(`README.md:22-23`); the JSON-RPC server is enabled by default "as long as
a username and password are provided" (`README.md:47-48`); the gRPC server
is enabled by default, is disabled with `--nogrpc`, and its API "is not
stabilized" (`README.md:52-57`); unencrypted private keys "are never
written to disk" (`README.md:10-11`); all derived addresses use
`m/44'/<coin type>'/<account>'/<branch>/<address index>`
(`README.md:11-14`); "Unencrypted private keys are not supported"
(`README.md:10`); dcrd-RPC mode is "the first (and default)" mode and SPV
is selected with `--spv`, with explicit peers given by `--spvconnect`
(`README.md:16-22`); the JSON-RPC server is "inspired by the Bitcoin Core
rpc server" with complete compatibility "not guaranteed", accounts in
particular (`README.md:40-46`); only the gRPC server provides
notifications of wallet changes (`README.md:54-57`); a wallet is created
with `dcrwallet --create` (`README.md:122-126`); SPV mode downloads
headers and filters but not full blocks (`README.md:134-135`). The flag
names `--spv`, `--spvconnect`, `--nogrpc` and `--create` are the only
configuration options the README names.

Q37. **`version.String` comments disagree with the code.** The comments
say the pre-release and build-metadata strings are "not appended if it
contains invalid characters"; the code strips the invalid characters and
appends what is left, omitting the part only when nothing is left
(`version/version.go:75-91,100-113`). Example: `PreRelease = "rc_1"`
renders `2.2.0-rc1`. The `init` comment-free fallback also means a
link-time `BuildMetadata` always wins over the VCS revision, and that
`BuildMetadata` cannot be forced empty in a VCS-stamped build
(`version/version.go:40-44`).

Q38. **Server behaviour asserted only by typed-client doc comments**
(claims for the JSON-RPC spec to confirm or refute; none is enforced by
the client): `lockunspent true` with no outpoints unlocks all outputs, and
lock state is not persisted across restarts
(`rpc/client/dcrwallet/methods.go:129-145`); `walletpassphrase` with
timeout 0 unlocks without a time limit (`methods.go:588-590`);
`lockaccount` on an already locked account fails with the
`ErrRPCWalletUnlockNeeded` code and on an unencrypted account fails;
`unlockaccount` on an already unlocked account succeeds
(`methods.go:565-574`); the `unlocked` member of `accountunlocked` is
present only when `encrypted` is true (`methods.go:557-558`);
`listreceivedbyaddress` omits watching-only addresses by default
(`methods.go:521-522`); when `signrawtransaction` is given private keys,
only those keys are used (`methods.go:899-901`); the send methods need an
unlocked wallet (`methods.go:169-243`).

Q39. **Help output of the flag-based tools depends on the file system.**
Because the `--connect` and `--cafile` defaults are cleared before parsing
when `<appdata>/rpc.cert` is missing, and go-flags derives the displayed
default from the struct value at parse time, the `(default: ...)` suffixes
of those two options appear only on machines where the default
certificate exists (section 5.10.1, `cmd/sweepaccount/main.go:113-118`,
`go-flags@v1.5.0/option.go:507-540`). The flag descriptions contain the
upstream wording `sweeped` and lower-case `decred`; they are part of the
help text.

Q40. **`movefunds` and `sweepaccount` share copied helpers that have
drifted.** `makeInputSource` returns `(total, source)` in `movefunds` and
only `source` in `sweepaccount`; the bodies are otherwise identical,
including the never-filled `redeemScriptSizes`
(`cmd/movefunds/main.go:70-120`, `cmd/sweepaccount/main.go:185-235`).
`destinationScriptSourceToAddress.Script` returns the (nil) decode error
variable on success instead of a literal nil
(`cmd/sweepaccount/main.go:274-281`). The doc comments on both `Script`
methods call them `Source`.

## 11. Upstream tests

| Test file | What it pins | Port |
|---|---|---|
| `pgpwordlist/pgpwordlist_test.go:25-66` | Two 20-byte vectors: decode of 20 words to bytes, and `ByteToMnemonic` per position. No error cases. | Port directly. |
| `walletseed/seed_test.go:14-88` | Four vectors (20, 20, 31 and 31 bytes; the last all zero) through `EncodeMnemonicSlice`, `EncodeMnemonic`, `DecodeUserInput` (mnemonic) and `DecodeUserInput` (hex). The 31-byte vectors exercise an odd-length seed, where the checksum word uses the odd half of the list. | Port directly. |
| `errors/errors_test.go:36-47` | Collapse: `E(E(Op("abc")), Passphrase)` equals `E(Op("abc"), Passphrase)` field by field with depth 1. | Port directly. |
| `errors/errors_test.go:49-70` | `Is` against the base error, a different string template, matching and non-matching `Op` templates, matching and non-matching kind templates. | Port directly. |
| `errors/errors_test.go:72-81` | Kind survives a second wrap with a string (Q2). | Port directly. |
| `rpc/client/dcrwallet/marshaling_test.go:15-61` | `marshalOutpointsToString` for 0, 1 and 2 outpoints, including `[]` for empty. | Port only if the client is ported. |
| `rpc/client/dcrwallet/methods_test.go:30-129` | All four `SignRawTransaction*` variants return `SignatureErrors` plus the transaction when `errors` is present, and nil error when absent; uses the 15-byte empty transaction `010000000000000000000000000000`. | Port only if the client is ported. |

Test functions by file, for traceability when porting:
`pgpwordlist_test.go`: `TestDecode`, `TestEncodeMnemonics`.
`seed_test.go`: `TestEncodeMnemonicSlice`, `TestEncodeMnemonic`,
`TestDecodeMnemonic`, `TestDecodeHex`.
`errors_test.go`: helpers `depth` and `eq` (field-wise comparison using
interface equality on `Err`), `TestCollapse`, `TestIs`,
`TestDoubleWrappedErrorWithKind`.
`marshaling_test.go`: `TestMarshalOutpointsToString`.
`methods_test.go`: a stub `caller` whose `Call` ignores the method and
arguments and unmarshals a canned response into the result, and
`TestSignRawTransactionErrors`. The canned response carries two
`SignRawTransactionError` objects (txids
`bfc0e650ad0cc0dd5fa88b6bc84beb5ea4a675b4353671532796171ed319341b` and
`79dd1f6a5b7fa43407d47ff203259efe9a78453605ffd07b4db1703fed339066`, vout 0
and 1, `scriptSig` `0123456789abcdef` and `abcdef0123456789`, sequence 123
and 456, messages `bad things happened` and `other things happened`), which
pins the member names `txid`, `vout`, `scriptSig`, `sequence`, `error` of
the server's `errors` array (`rpc/client/dcrwallet/methods_test.go:31-70`).

No test in scope exercises any `cmd/` program, `rpc_example`, `assets`,
`kdf`, `lru`, `validate` or `version`; `rpc/client/dcrd` (out of this
scope, owned by the dcrd-RPC sync spec) has its own `json_test.go`.

Untested upstream: `Error()` rendering (no test compares a rendered string),
`As`, `Unwrap`, `Join`, `WithStack`/`Stacks`; every error path of
`DecodeMnemonics` and `DecodeUserInput`; `GenerateRandomSeed`; all of `kdf`,
`lru`, `validate`, `version`, `assets`; all four tools and `rpc_example`.

Vectors to generate through a Go oracle shim:

1. `errors`: a table of `E(...)` argument lists (covering each row of the
   table in 5.4, three-level chains, repeated `Op`s, kinds on inner levels
   only, `Errorf("%w")` wrappers, `Join`) with the rendered string under both
   separators, the result of `As(&kind)`, and `Is` against a set of
   templates.
2. `walletseed`: encode/decode round trips for lengths 16, 17, 31, 32, 33,
   64; every failure class of 5.3 with its exact message; inputs with mixed
   case, repeated spaces, leading/trailing white space, tabs and newlines,
   15- and 65-byte seeds in both forms, a two-word input, and the two
   non-ASCII folding cases of Q15.
3. `kdf`: `MarshalBinary`/`UnmarshalBinary` round trip and `DeriveKey`
   outputs for a few `(password, salt, time, memory, threads)` tuples with
   small memory (for example 64 KiB and thread counts 1, 2, 4, 64) plus one
   at the production parameters.
4. `lru`: an operation trace (adds, contains, gets) with the membership set
   after each step for limits 0, 1 and 3.
5. `version`: `String()` for combinations of pre-release and build metadata
   including characters outside the alphabet.
6. `treasurykey`: WIF strings for fixed private keys on all four networks
   (call `dcrutil.NewWIF` directly).

CI at the pin (`.github/workflows/go.yml`, `run_tests.sh`): on every push and
pull request, on `ubuntu-latest`, for Go 1.26 and 1.27: `go build ./...`,
then `go version` and `go test -short -vet=all ./...` under `set -ex`. There
is no lint job, no race detector run, no formatting check and no non-Linux
runner. `-short` skips exactly one test body
(`wallet/udb/addressmanager_test.go:1212`). `run_tests.sh` honours a `GO`
environment variable for the toolchain and forwards extra arguments to
`go test`. Three files are excluded from normal builds by tags
(`wallet/walletdb/example_test.go` `disabled`; `wallet/udb/txquery_test.go`
and `txexample_test.go` `ignore`).

Workflow details (`.github/workflows/go.yml:1-24`): workflow name
`Build and Test`; triggers `push` and `pull_request` with no branch or
path filter; token permissions limited to `contents: read`; one job `build`
displayed as `Go CI`; matrix `go: ["1.26", "1.27"]` with default fail-fast
behaviour; steps in order: set up Go (`actions/setup-go`, pinned by full
commit hash), check out source (`actions/checkout`, pinned by full commit
hash), `go build ./...`, `sh ./run_tests.sh`. Third-party actions are
pinned by commit hash rather than tag. There is no caching configuration
beyond the action defaults, no artifact upload, no release job, no
dependency-review or vulnerability scan, no protobuf regeneration check
and no build of the `cmd/` tools other than through `./...` (they are in
the main module, so `go build ./...` compiles them; having no test files
they are not vetted by `go test -vet=all`. Compilation is the only
assurance they receive, which is how `movefunds` and `sweepaccount` came
to be broken at run time while CI stays green).

`run_tests.sh` (`run_tests.sh:1-9`) is a POSIX `sh` script: `set -ex`
(abort on first failure, echo commands), `GO=${GO:-go}`, `${GO} version`,
`${GO} test -short -vet=all "$@" ./...`. Extra arguments are placed before
the package pattern, so flags such as `-race` or `-run` can be passed
through.

Other repository-level facts stated by the README: the project is a single
Go module (`README.md:90`); the documented install commands are
`go install decred.org/dcrwallet/v5@master` and
`go install decred.org/dcrwallet/v4@latest` (`README.md:94,100`); release
binaries ship in the `decred-release` CLI suite (`README.md:64-65`);
Windows users are told to use `cmd.exe` or PowerShell
(`README.md:119-120`); `dcrctl` is named as the RPC client for both
daemons (`README.md:139-141`); the licence is ISC (`README.md:176`); the
issue tracker is the GitHub one (`README.md:163-164`). `.gitignore` lists
the four tool binaries, `rpc/rpc_example/rpc_example`, `/dcrwallet`,
`*.key`, and `wallet/udb/*.kv` (temporary databases created by tests).

Stated support policy: build from source needs Go 1.26 or 1.27
(`README.md:77`); "Decred only supports the current and previous major
versions of Go" (`README.md:148-149`); binary releases are provided for
Windows, Linux and macOS (`README.md:61-65`); the Nix flake lists x86_64 and
aarch64 for Linux and Darwin (`flake.nix:8-13`); signal handling has a Unix
build-tagged file covering darwin, dragonfly, freebsd, linux, netbsd,
openbsd and solaris (`signal_unix.go:6`); security bugs must reproduce on
the latest production release or master and are reported through the bug
bounty site (`SECURITY.md:3-13`).

## 12. Rust porting notes

**Suggested boundaries**

| Crate / module | Contents | Reuse |
|---|---|---|
| `purse-errors` (leaf crate, no dependencies on other wallet crates) | `Kind`, `Op`, `Error`, the `E` construction rules, rendering with a process-wide separator, `is`/`as_kind` matching | New. dcroxide has no equivalent of this Upspin-style model (its crates define their own error types, for example `dcroxide:crates/dcroxide-standalone/src/error.rs`). |
| `purse-seed` | word list, `byte_to_mnemonic`, `decode_mnemonics`, `encode_mnemonic(_slice)`, `decode_user_input`, `generate_random_seed` | SHA-256 from the `sha2` crate already used by dcr-rs (`dcr-rs:Cargo.toml:63`). dcr-rs' `mnemonic` feature is BIP39 (`dcr-rs:src/hd.rs:75-76,235-249`), a different scheme; do not route Decred seeds through it. Seed bounds match `dcr-rs:src/hd.rs:90-92`. |
| `kdf` module inside the address-manager crate | 25-byte record, Argon2id derive | An Argon2 crate with explicit `p` (lanes), version 0x13 and raw 32-byte output; neither dcroxide nor dcr-rs depends on one today. |
| `lru` module in the P2P crate | `Cache<T>` and `Map<K,V>` with the `limit+1` behaviour | Not `dcroxide:crates/dcroxide-containers/src/lru.rs`: that is dcrd's `container/lru` (exact limit, TTL support, value replacement on `put`). Write the 60-line structure or wrap the dcroxide one with `limit+1` and a no-replace `add`; the former is simpler to audit. |
| `validate` module in the sync crate | four checks | `dcroxide:crates/dcroxide-standalone/src/merkle.rs:77,96` (`calc_tx_tree_merkle_root`, `calc_combined_tx_tree_merkle_root`), `dcroxide:crates/dcroxide-standalone/src/inclusionproof.rs:119` (`verify_inclusion_proof`), `dcroxide:crates/dcroxide-gcs/src/lib.rs:477` (`FilterV2`). |
| `version` module in the daemon crate | constants, `string()`, normalisation, build metadata from the build script | Same shape as `dcroxide:crates/dcroxide-node/src/version.rs:47,167`. |
| tool binaries (optional) | `treasurykey`, `repaircfilters`, and, if wanted, fixed `sweepaccount`/`movefunds` | `dcroxide:crates/dcroxide-node/src/config.rs:1189` (`app_data_dir`), `dcroxide:crates/dcroxide-node/src/bin/promptsecret.rs` (no-echo terminal read), `dcroxide:crates/dcroxide-base58/src/lib.rs:129` (base58), `dcroxide:crates/dcroxide-dcrec` (secp256k1), `dcroxide:crates/dcroxide-node/src/flags.rs` (go-flags emulation, written against v1.6.1 while dcrwallet pins v1.5.0; diff the two before reuse). No WIF encoder exists in dcroxide or dcr-rs; it is needed by the wallet proper anyway (`dumpprivkey`, `importprivkey`). |
| typed client (optional) | Rust equivalent of `rpc/client/dcrwallet` | Useful as the driver for RPC parity tests; not required for drop-in status. |

**Hazards**

- The error model is cross-cutting: its API shape decides how every other
  crate reports failures and how faithfully messages can be reproduced.
  Settle it first (section 13, decision D1). A workable representation is a
  struct `{op: Option<Op>, kind: Kind, source: Option<Source>}` where
  `Source` is either another wallet error or an opaque leaf with a
  pre-rendered string, with a constructor that applies steps 3 and 4 of 5.4
  explicitly. Leaf strings that originate in Go libraries (`encoding/hex`,
  bbolt, `net`, TLS, JSON) are part of messages too; each subsystem has to
  decide which of those it reproduces.
- `Separator` must be `":: "` in the daemon binary and available as the
  multi-line form for ported tests.
- Kind lookup must walk to the outermost classified level, not the leaf.
- Mnemonic decoding: do not use `split_whitespace`; split on `' '` after
  trimming and skip tokens that are empty after per-token trimming. Go's
  `TrimSpace` and Rust's `char::is_whitespace` both follow the Unicode
  `White_Space` property; differences can only come from the Unicode version
  each runtime ships.
- Argon2: verify the chosen crate against Go vectors with `p > 1` and with
  memory not divisible by `4p`; check whether the crate rejects parameter
  combinations that Go accepts (for example memory below `8p`, which Go
  silently raises to `8p`).
- Seed bytes, passphrases and derived keys should be zeroised; the Go code
  does not do so for seeds, so this is an improvement without observable
  effect.
- Amount formatting in tool output uses Go's shortest float formatting of
  `atoms/1e8`; `dcr-rs:src/amount.rs:11` (`format_amount`) produces the same
  text for amounts within the money supply (integer arithmetic, trailing
  zeros trimmed) but this was compared by reading, not by test.

**Size**: S to M overall. `errors` is about 150 lines of logic but needs the
largest vector set and early agreement; seed, kdf, lru, validate and version
are S each (the word list is data); `treasurykey` and `repaircfilters` are
S each; fixing and porting `sweepaccount`/`movefunds` and the 79-method
client would add an M and is optional.

## 13. Open questions

1. **D1: fidelity target for error messages.** Options: (a) reproduce `Op`
   names, kind strings and nesting exactly for every error that can reach an
   RPC client; (b) reproduce kind strings and codes exactly and treat `Op`
   prefixes and library leaf texts as best effort. Option (a) requires every
   subsystem spec to enumerate its `Op` strings and wrap sites; the counts
   are 154 `Op` constants (152 distinct), 38 `Opf` formats, 4 inline `Op`
   literals and 1,050 `E` call sites.
2. **D2: which tools ship.** `treasurykey` and `repaircfilters` work and are
   small. `movefunds` and `sweepaccount` do not work at the pin; porting
   them "faithfully" is meaningless. Choose between dropping them, or
   shipping corrected versions and documenting the divergence.
3. **D3: typed client.** Whether to provide a Rust client crate mirroring
   `rpc/client/dcrwallet` (useful for parity tests), and whether to carry the
   two wrong method names.
4. **D4: version identity.** What `version.String()`, `Integer` and the user
   agent report for the replacement (same numbers as the pin, or its own),
   and how build metadata is derived without Go build info.
5. **D5: Argon2 thread clamp.** Clamp new parameters to 255 (diverging from
   the Go truncation to 0 at 256 CPUs) or reproduce the defect.
6. **D6: Unicode case folding in mnemonics** (Q15): reproduce Go's simple
   folding for U+212A and U+0130, or accept ASCII only.
7. **D7: CI policy for the Rust project.** The Go project tests two
   toolchain versions on Linux only with vet enabled. Decide the Rust
   equivalents (MSRV and stable, clippy, platforms) and whether to add the
   Windows and macOS runners that upstream lacks although it ships binaries
   for them.
8. UNVERIFIED: the `-ldflags` used by official release builds for
   `PreRelease` and `BuildMetadata` (they live in the release tooling
   repository, not here). Reading the decred-release build scripts would
   settle it.
9. PARTLY SETTLED: the text go-flags v1.5.0 prints for `-h` and for parse
   errors in the two flag-based tools is given in section 5.10.1 from a
   full reading of `help.go` and the relevant parts of `parser.go`,
   `option.go` and `convert.go`. It has not been confirmed against a run
   of the Go binaries; remaining uncertainty is limited to word-wrapping
   of long default values (certificate paths) and the Windows option
   style. One run of each tool with `--help` would settle it.
10. UNVERIFIED: the exact error text `sweepaccount` prints at the dial step
    for each address form (it depends on Go's `net/url` parsing of a
    scheme-less `host:port`, which was not read); the failure itself is
    confirmed from the gorilla scheme check.
11. UNVERIFIED: whether any deployed third-party software depends on the
    multi-line (`":\n\t"`) rendering through the public `errors` package of
    the Go module. This is a Go API concern and outside the drop-in surface.
12. UNVERIFIED: the 4-byte checksum of the README's treasury WIF
    (`60531a6b` after base58 decoding) was not recomputed, and no
    Schnorr-typed (type byte 2) WIF vector exists in the repository. A Go
    oracle run of `dcrutil.NewWIF` for the README private key on all four
    networks with types 0 and 2 would supply both.
13. UNVERIFIED in this spec: what `importprivkey` does with the signature
    type of a Schnorr-typed WIF (whether the address manager stores it,
    ignores it or rejects it). `dcrutil.DecodeWIF` accepts it; the answer
    is in `wallet/udb` and belongs to the address-manager spec. It decides
    whether `treasurykey` output is importable as documented.
14. UNVERIFIED: the typed client's `null`-handling description in section
    4.8 was derived by reading `encoding/json` (`decode.go`, the v1
    decoder); it assumes the v1 decoder is the one in effect for the Go
    versions in CI. A three-line Go test would confirm it. Only relevant
    if the client is ported (D3).
15. **D8: status of the README claims in Q36.** Each owning spec (SPV,
    staking, JSON-RPC, gRPC, address manager) should state whether the
    claim holds at the pin, since the replacement's README will be read
    against the same expectations.
16. **D9: supply-chain policy.** Upstream pins CI actions by commit hash
    and limits the workflow token to read access; decide whether the Rust
    project adopts the same rules (and their cargo equivalents: locked
    builds, audited dependencies).
