# 23 — Upstream test inventory and the differential-testing (oracle) plan

## 1. Header

**Scope.** Every `*_test.go` file and every `testdata` directory of dcrwallet at the
parity pin, read end to end, plus the dcroxide verification rig this plan is modelled
on.

dcrwallet test files (43 files, 10,932 lines; counts from `wc -l`):

| File | Lines | File | Lines |
| --- | ---: | --- | ---: |
| `chain/semver_test.go` | 88 | `wallet/txauthor/author_test.go` | 239 |
| `errors/errors_test.go` | 81 | `wallet/txrules/poolfees_test.go` | 93 |
| `internal/rpc/jsonrpc/rpchelp_test.go` | 111 | `wallet/txsizes/size_test.go` | 70 |
| `internal/rpc/jsonrpc/rpcserver_test.go` | 54 | `wallet/udb/addressmanager_test.go` | 1,274 |
| `pgpwordlist/pgpwordlist_test.go` | 66 | `wallet/udb/cointype_test.go` | 210 |
| `rpc/client/dcrd/json_test.go` | 379 | `wallet/udb/common_test.go` | 97 |
| `rpc/client/dcrwallet/marshaling_test.go` | 61 | `wallet/udb/internal_test.go` | 80 |
| `rpc/client/dcrwallet/methods_test.go` | 129 | `wallet/udb/stakevalidation_test.go` | 187 |
| `rpc/jsonrpc/types/methods_test.go` | 1,306 | `wallet/udb/txcommon_test.go` | 103 |
| `rpc/jsonrpc/types/walletsvrwscmds_test.go` | 192 | `wallet/udb/txexample_test.go` | 229 |
| `wallet/addresses_test.go` | 307 | `wallet/udb/txmined_test.go` | 80 |
| `wallet/discovery_test.go` | 101 | `wallet/udb/txquery_test.go` | 759 |
| `wallet/internal/bdb/driver_test.go` | 159 | `wallet/udb/tx_test.go` | 574 |
| `wallet/internal/bdb/interface_test.go` | 720 | `wallet/udb/upgrades_test.go` | 482 |
| `wallet/internal/snacl/snacl_test.go` | 118 | `wallet/udb/vsp_test.go` | 252 |
| `wallet/locking_test.go` | 150 | `wallet/walletdb/db_test.go` | 142 |
| `wallet/main_test.go` | 24 | `wallet/walletdb/example_test.go` | 175 |
| `wallet/mixing_test.go` | 128 | `wallet/walletdb/interface_test.go` | 709 |
| `wallet/network_test.go` | 40 | `wallet/wallet_test.go` | 231 |
| `wallet/reorg_test.go` | 238 | `walletseed/seed_test.go` | 88 |
| `wallet/setup_test.go` | 55 | `x509_test.go` | 211 |
| `wallet/sidechains_test.go` | 140 | | |

dcrwallet test data (one directory, `wallet/udb/testdata`, 16 files): eight gzip
fixtures `v1.db.gz`, `v2.db.gz`, `v3.db.gz`, `v4.db.gz`, `v5.db.gz`, `v7.db.gz`,
`v11.db.gz`, `v24.db.gz` and their eight generator programs `v1.db.go` (159 lines),
`v2.db.go` (114), `v3.db.go` (78), `v4.db.go` (125), `v5.db.go` (433), `v7.go` (214),
`v11.db.go` (469), `v24.db.go` (94).

Also read, as the model for the plan:

| Source | What was read |
| --- | --- |
| `dcroxide:docs/adr/0001-oracle-driven-differential-testing.md` | all 612 lines |
| `dcroxide:docs/dcroxide-project-brief.md` | section 7 (lines 700-742) and the phase headings |
| `dcroxide:tools/oracle/` | `main.go` protocol header and command table, `go.mod` |
| `dcroxide:tools/*` | headers and run scripts of `mixpoolgen`, `chainopsgen`, `helpgen`, `fullblockgen`, `pinbump`, `dcrdstat`; header comments of `disapprovalgen` (and its `run.sh`), `trsyboundarygen`, `utxoviewgen`, `mempoolvectors`, `profilergen`, `powerloss` (`shim.c`, `replay.py`), `dsample` (section 5.5.3) |
| `dcroxide:crates/dcroxide-testutil/src/lib.rs` | oracle spawn policy, `DcrdNode` harness, seeds |
| `dcroxide:fuzz/fuzz_targets/` | target list, two targets' headers |

**Parity pin.** dcrwallet master `54b85b33` (2.2.0-pre, module
`decred.org/dcrwallet/v5`, `go 1.26.0` in `go.mod:3`). Dependency versions read for this
spec, each the version `go.mod` pins unless noted:

| Module | Version | Why it was read |
| --- | --- | --- |
| `github.com/decred/dcrd/crypto/rand` | v1.0.1 | the only randomness source of dcrwallet; determinism seam |
| `github.com/decred/dcrd/blockchain/v5` | v5.1.1 | `chaingen`, used only by dcrwallet tests |
| `github.com/decred/dcrd/gcs/v4` | v4.1.1 | `blockcf2.B`, `blockcf2.M` and the `PrevScripter` interface used by test helpers |
| `github.com/decred/dcrd/mixing` | v0.7.5 | `MinPeers`, test hooks, test names |
| `github.com/decred/dcrd/chaincfg/v3` | v3.3.0 | simnet stake parameters for the acceptance harness |
| `github.com/decred/dcrtest/dcrdtest` | v1.0.1-0.20240404170936-a2529e936df1 | the only dcrtest package in the module cache; **not** a dcrwallet dependency |
| `github.com/decred/vspd` | pseudo-version at `a381bece711e` | `harness.sh`, `rpc/dcrwallet.go`, `rpc/version.go`; **not** a dcrwallet dependency (dcrwallet pins only `vspd/client/v4` v4.0.2 and `vspd/types/v3` v3.0.0) |
| dcrd repository | local checkout at `5dd4ca59` | `contrib/devtools/dcr_tmux_simnet_setup.sh` only; **not** a pin |

**Summary.**

- dcrwallet has 77 live top-level test functions, no benchmarks and no fuzz targets,
  in 10,932 lines of test code against 83,418 lines of non-test Go (64,674 once the
  18,744 generated lines of `rpc/walletrpc` are set aside). 1,163 of the test lines sit
  in three files excluded from the build by a build tag.
- Coverage is concentrated in the storage layer (`wallet/udb`, `wallet/walletdb`,
  `wallet/internal/bdb`) and in JSON-RPC command marshalling. There are no tests at all
  for SPV (`spv`, `p2p`), the gRPC server, the JSON-RPC method handlers, ticket
  purchase, voting, revocation, VSP, the mixing integration, the ticket buyer,
  configuration parsing, or the daemon lifecycle.
- The upstream vectors that exist are small and portable; they pin perhaps two percent
  of the observable surface. Everything else has to be pinned by vectors this project
  generates from dcrwallet's own code.
- The plan adopts dcroxide's rig unchanged in mechanism: one Go oracle binary over
  line-delimited JSON, pinned to the parity commit, built on demand, fail-not-skip in
  CI; kept overlay exporters for anything inside `internal/` packages or unexported;
  frozen vector files with the exporter kept in the tree.
- Three things are specific to a wallet and have no dcroxide precedent: byte-level
  database cross-compatibility with a file Go wrote (and will write again), a second
  public API (gRPC) that needs golden captures, and behaviour that is deliberately
  random (coin selection, change position, nonces, salts), which blocks naive
  differential testing and needs a seeded seam on both sides.

---

## 2. Role in the system

This document is not a subsystem of the wallet. It is the contract for how every other
specification in this set is verified.

- **Consumers.** Every implementation crate takes its test vectors, fixtures, oracle
  commands and acceptance scenarios from here. Specifications 01-16 each end with a
  list of "vectors to generate through a Go oracle shim"; this document defines the one
  rig those lists are served by, so that the project has a single oracle binary and a
  single set of conventions rather than sixteen.
- **Producers.** The only authority is the Go code at the pin. A vector is valid only
  if it was emitted by dcrwallet's own functions (or by a pinned dependency's), never
  transcribed from a reading of the source.
- **What it calls.** The Go toolchain (to build the oracle and the exporters), a
  dcrwallet checkout at the pin (for overlay exporters and for the reference binary), a
  dcrd binary at dcroxide's parity pin, a dcroxide binary, and optionally vspd, dcrctl
  and Decrediton for the acceptance matrix.
- **What calls it.** `cargo test` in every crate (function-level differential tests),
  CI jobs (interop, acceptance, fuzz), and the engineer moving the parity pin.

---

## 3. Public surface inventory

For a test plan the "surface" is: every upstream test function and what it pins
(3.1-3.3), every upstream helper and test seam (3.4), and every fixture (3.5).

### 3.1 Classification

| Code | Meaning |
| --- | --- |
| **P** | Portable as is. The inputs and expected outputs are literal data in the test; transcribe them into a Rust test. |
| **P+R** | Portable, but the test only checks a round trip or a property. Port it and add byte vectors regenerated through the oracle. |
| **R** | Regenerate. The expectations are computed by Go code at run time (or depend on Go internals); port the scenario and take the expected bytes from the oracle. |
| **F** | Binary fixture. Carry the file over unchanged and replay the scenario against it. |
| **G** | Go-internal. Pins a mechanism that has no counterpart in a Rust port (the driver registry, test seams, Go's TLS stack). Re-express the intent only where it is observable. |
| **D** | Dead. Excluded from the Go build by a build constraint; never runs upstream. |
| **H** | Helper only. No test function. |

### 3.2 Test functions by package

Line numbers are the first line of the function.

#### Root package `main` (3,039 source lines, 211 test lines)

| Test | Cite | Pins | Vectors | Class |
| --- | --- | --- | --- | --- |
| `TestClientCert` | `x509_test.go:48` | A client certificate made by `generateAuthority` + `createSignedClientCert` + `marshalPrivateKey` is accepted by a Go TLS 1.2+ server requiring and verifying client certificates, for Ed25519, P-256, P-384 and P-521 keys; an HTTP PUT body is echoed back. | none (keys are random) | G, re-express: the issued certificate must verify under Go's `crypto/tls` |
| `TestUntrustedClientCert` | `x509_test.go:123` | A client certificate signed by a different authority is refused; the client-side error ends in `tls: bad certificate` or `tls: unknown certificate authority` (connection resets are retried for up to 5 s). | none | G, re-express as an interop test against a Go client |

Nothing else in package `main` is tested: `config.go` (1,050 lines), `dcrwallet.go`
(636), `walletsetup.go` (388), `rpcserver.go` (512, apart from the three certificate
helpers above), `ipc.go` (203), `log.go`, signal handling.

#### `chain` (1,122 / 88)

| Test | Cite | Pins | Vectors | Class |
| --- | --- | --- | --- | --- |
| `TestSemverCompatible` | `chain/semver_test.go:8` | `semverCompatible(required, actual)` | 6 rows: equal versions are compatible (three rows); `1.2.3` vs `2.0.0`, `1.2.4` vs `1.0.2`, `1.3.6` vs `1.3.2` are not | P |
| `TestSemverToString` | `chain/semver_test.go:63` | `semver.String()` | `1.2.3`, `2.0.1` | P |

The syncer, notification handling and backend (`chain/sync.go`, `chain/backend.go`)
are untested.

#### `errors` (400 / 81)

| Test | Cite | Pins | Class |
| --- | --- | --- | --- |
| `TestCollapse` | `errors/errors_test.go:36` | `E(E(Op("abc")), Passphrase)` produces a single-level error equal to `E(Op("abc"), Passphrase)`. | P |
| `TestIs` | `errors/errors_test.go:49` | `Is` matches a wrapped base error, an `Op`, a `Kind`; does not match a different string, `Op` or `Kind`. | P |
| `TestDoubleWrappedErrorWithKind` | `errors/errors_test.go:72` | A kind survives a second wrap: `Is(E(E(Invalid, "abc"), "def"), Invalid)`. | P |

The text produced by `Error()` (observable in RPC error messages and logs) is not
tested.

#### `internal/rpc/jsonrpc` (6,843 / 165)

| Test | Cite | Pins | Class |
| --- | --- | --- | --- |
| `TestRPCMethodHelpGeneration` | `internal/rpc/jsonrpc/rpchelp_test.go:39` | For every locale in `rpchelp.HelpDescs`, `dcrjson.GenerateHelp` succeeds for each of the 103 entries of `rpchelp.Methods`; every handler without `noHelp` has an entry; the generated text equals the committed `rpcserverhelp.go`. | R: the committed help text is the vector; an exporter proves equality at the pin |
| `TestRPCMethodUsageGeneration` | `internal/rpc/jsonrpc/rpchelp_test.go:78` | `dcrjson.MethodUsageText` succeeds for every method and the joined usages equal the committed `requestUsages`. | R |
| `TestThrottle` | `internal/rpc/jsonrpc/rpcserver_test.go:15` | `throttledFn(1, h)`: of two concurrent requests one gets 200 and the other 429. | P |

None of the 111 entries of the `handlers` map (`internal/rpc/jsonrpc/methods.go:85`) is
exercised by a test.

#### `pgpwordlist` (609 / 66) and `walletseed` (102 / 88)

| Test | Cite | Pins | Vectors | Class |
| --- | --- | --- | --- | --- |
| `TestDecode` | `pgpwordlist/pgpwordlist_test.go:43` | `DecodeMnemonics` | 2 × 20 words → 20 bytes | P |
| `TestEncodeMnemonics` | `pgpwordlist/pgpwordlist_test.go:56` | `ByteToMnemonic(b, index)` | the same 40 byte/word pairs | P |
| `TestEncodeMnemonicSlice` | `walletseed/seed_test.go:46` | `EncodeMnemonicSlice` appends the checksum word | 4 vectors (section 4.2.2) | P |
| `TestEncodeMnemonic` | `walletseed/seed_test.go:55` | `EncodeMnemonic` (space-joined) | same 4 | P |
| `TestDecodeMnemonic` | `walletseed/seed_test.go:64` | `DecodeUserInput` on a mnemonic | same 4 | P |
| `TestDecodeHex` | `walletseed/seed_test.go:77` | `DecodeUserInput` on lower-case hex | same 4 | P |

Not tested: checksum mismatch, unknown words, wrong word parity, seed length bounds,
upper-case and whitespace handling, `GenerateRandomSeed`.

#### `rpc/client/dcrd` (997 / 379)

All seven tests are `t.Parallel()`.

| Test | Cite | Pins | Class |
| --- | --- | --- | --- |
| `TestUnmarshalHashes` | `rpc/client/dcrd/json_test.go:21` | `hashes.UnmarshalJSON`: `null` → zero hashes; truncated JSON → error; non-hash strings → kind `Encoding`; two 64-hex strings decode to hashes that print back identically. | P |
| `TestMarshalHashes` | `:70` | `hashes.MarshalJSON`: empty → `null`; two hashes → `["…","…"]` with no whitespace. | P |
| `TestUnmarshalHashesContiguous` | `:102` | Same as `TestUnmarshalHashes` for the contiguous-array form. | P |
| `TestMarshalHashesContiguous` | `:151` | Same as `TestMarshalHashes`. | P |
| `TestUnmarshalHeaders` | `:183` | `headers.UnmarshalJSON`: `null` → zero headers; truncated JSON → error (kind not checked); non-hex strings → `Encoding`; two hex block headers decode field-for-field. | P (headers are built in the test from field values; serialize them with the wire crate) |
| `TestUnmarshalTransactions` | `:255` | `transactions.UnmarshalJSON`: `null` → zero transactions; truncated JSON → error (kind not checked); `[1234]` → `Encoding`; non-hex → `Encoding`; two empty transactions (version 1 lock time 0; version 123 lock time 69420) decode. | P |
| `TestUnmarshalHash` | `:332` | `hash.UnmarshalJSON`: 63 and 65 hex digits → `Encoding`; missing opening or closing quote → `Encoding`; 64 `f` decodes. | P |

The client's call methods, the notification decoding and everything else in the
package are untested.

#### `rpc/client/dcrwallet` (1,279 / 190)

| Test | Cite | Pins | Class |
| --- | --- | --- | --- |
| `TestMarshalOutpointsToString` | `rpc/client/dcrwallet/marshaling_test.go:15` | `marshalOutpointsToString`: `[]` for none; `["<hash>:<index>"]` joined by commas, no whitespace. 3 vectors. | P (client library; needed only if the Rust project ships a client) |
| `TestSignRawTransactionErrors` | `rpc/client/dcrwallet/methods_test.go:30` | The four `SignRawTransaction*` client calls return a `SignatureErrors` value when the response carries `errors`, with the partially signed transaction and `complete=false`; and no error when `errors` is absent. | P as a response-shape vector for `signrawtransaction` (the JSON literal at `methods_test.go:47-66` is a server-shaped result) |

#### `rpc/jsonrpc/types` (1,825 / 1,498)

| Test | Cite | Pins | Vectors | Class |
| --- | --- | --- | --- | --- |
| `TestWalletSvrCmds` | `rpc/jsonrpc/types/methods_test.go:23` | For each case: `dcrjson.MarshalCmd("1.0", 1, staticCmd)` equals the literal JSON; `dcrjson.NewCmd(method, args…)` marshals to the same bytes; `dcrjson.ParseParams` of those params deep-equals the expected command struct, with defaults filled in for omitted optional parameters. | 79 cases over 39 methods | P |
| `TestWalletSvrWsCmds` | `rpc/jsonrpc/types/walletsvrwscmds_test.go:22` | Same three checks. | 7 cases over 4 methods (`getunconfirmedbalance`, `listaddresstransactions`, `listalltransactions`, `walletislocked`) | P |

Together the two tests touch 43 distinct methods of the 104 the package registers
(85 wallet methods, 18 dcrd methods re-implemented by the wallet, and `authenticate`;
`rpc/jsonrpc/types/methods.go:1189-1312`). No result type is marshalled by any test.
Defaults the cases pin include `minconf=1` (`getbalance`, `getreceivedby*`,
`listaccounts`, `listreceivedby*`), `includewatchonly=false`, `rescan=true`
(`importprivkey`), `count=10`/`from=0` (`listtransactions`), and
`targetconfirmations=1` (`listsinceblock`).

#### `wallet` (16,726 / 1,414)

| Test | Cite | Pins | Vectors | Class |
| --- | --- | --- | --- | --- |
| `TestAddresses` | `wallet/addresses_test.go:186` | On a fresh simnet wallet from a fixed 32-byte seed, the first five `NewInternalAddress` and first five `NewExternalAddress` results (address string, hash160, compressed public key, branch) and `KnownAddress(...).AccountName() == "default"`; repeated locked and unlocked, and after `ConvertToWatchingOnly`. | 10 addresses (section 4.2.3) | P |
| `TestAccountIndexes` | `wallet/addresses_test.go:219` | The in-memory address buffer (`lastUsed`, `cursor`) of account 0 after sequences of `NewExternalAddress`, `watchHDAddrs`, and marking child 10, 30, 31 used. 9 steps. | table at `:231-239` | R (reads unexported state; port the scenario, assert on the next address returned and on the persisted last-used/last-returned indexes) |
| `TestDiscoveryCursorPos` | `wallet/discovery_test.go:16` | `DiscoverActiveAddresses` does not reset an in-memory cursor of 9 when nothing is used; after child 4 is marked used and 0-9 marked returned, `lastUsed` becomes 4 and `cursor` becomes 4. Runs with `DisableCoinTypeUpgrades=true`. | 2 states | R |
| `TestLocking` | `wallet/locking_test.go:15` | Five sub-scenarios on one wallet: unlock without timeout then `Lock`; a wrong passphrase on an unlocked wallet returns kind `Passphrase` **and locks the wallet**; a timeout channel passed while unlocked without timeout is drained in the background and does not lock; a timeout channel locks after it fires; a second timeout replaces the first, and the first channel is still drained. | none | P (timing constants 100 ms are test-side) |
| `TestIsMixTx` | `wallet/mixing_test.go:21` | `PossibleCoinJoin(tx)`: mainnet transaction `ab70b9b3…78f3` → `(true, 268435456, 11)`; a coinbase → `(false, 0, 0)`. | 2 raw transactions (`mixing_test.go:77-127`) | P |
| `TestReorg` | `wallet/reorg_test.go:155` | `SidechainForest.AddBlockNode`, `EvaluateBestChain`, `ChainSwitch`, forest pruning and `BlockInMainChain` across: block one; a two-block extension attached together; a two-block side chain that does not win; a third side block that wins and reorganizes two blocks out. | blocks from `chaingen` at run time | R (chain fixture, section 5.9) |
| `TestPruneChainFromSRT` | `wallet/sidechains_test.go:43` | `sidechainRootedTree.pruneChain` on a four-branch tree: pruning the 5-block trunk leaves two trees, one with two tips; pruning again partially; pruning to nothing. | headers with random `ExtraData` | P (structure only) |
| `TestCoinbaseMatured` | `wallet/wallet_test.go:21` | `coinbaseMatured(mainnet, txHeight, tip)`; 9 rows; matured at `tip − tx ≥ 256`. | section 4.2.6 | P |
| `TestTicketMatured` | `wallet/wallet_test.go:48` | `ticketMatured`; 9 rows; **not** matured at exactly 256, matured at 257. | section 4.2.6 | P |
| `TestTicketExpired` | `wallet/wallet_test.go:75` | `ticketExpired`; 9 rows; not expired at exactly maturity + expiry, expired one block later. | section 4.2.6 | P |
| `TestVotingXprivFromSeed` | `wallet/wallet_test.go:103` | `votingXprivFromSeed(32 zero bytes, mainnet)` → one `dprv…` string; a 1-byte seed → kind `Invalid`. | section 4.2.4 | P |
| `TestSetBirthStateAndScan` | `wallet/wallet_test.go:147` | After attaching 9 blocks: birthday by height 6 → block 6; height 0 → genesis; time one minute in the future → tip; time one minute in the past → genesis; an empty state → error. | blocks from `chaingen` | R |

Nothing in `wallet/createtx.go`, `wallet/tickets.go`, `wallet/vsp*.go`,
`wallet/feepayment.go`, `wallet/mixing.go` (other than `PossibleCoinJoin`),
`wallet/rescan.go`, `wallet/notifications.go`, `wallet/chainntfns.go` transaction
processing, or the bulk of `wallet/wallet.go` is tested.

#### `wallet/internal/bdb` (417 / 879) and `wallet/walletdb` (425 / 1,026)

| Test | Cite | Pins | Class |
| --- | --- | --- | --- |
| `TestCreateOpenFail` (bdb) | `wallet/internal/bdb/driver_test.go:25` | Open of a missing file → kind `NotExist`; exact error strings for wrong argument count and type to `Open` and `Create` (section 8.1); `BeginReadTx` on a closed database → kind `Invalid`. | P |
| `TestPersistence` | `wallet/internal/bdb/driver_test.go:92` | Three key/value pairs in a top-level bucket survive close and reopen. | P |
| `TestInterface` (bdb) | `wallet/internal/bdb/interface_test.go:707` | Conformance suite over two top-level buckets (section 5.2.3). | P |
| `TestAddDuplicateDriver` | `wallet/walletdb/db_test.go:23` | Registering a second driver under an existing name → kind `Exist`, and the first driver stays in effect. | G |
| `TestCreateOpenFail` (walletdb) | `wallet/walletdb/db_test.go:65` | A driver's own error is returned by `Create`/`Open` such that `errors.Is` matches it. | G |
| `TestCreateOpenUnsupported` | `wallet/walletdb/db_test.go:110` | Unknown driver name → kind `Invalid`. | G |
| `TestInterface` (walletdb) | `wallet/walletdb/db_test.go:128` | Same suite as the bdb copy, slightly newer; `testCursorDelete` is defined but skipped (`interface_test.go:202`). | P |
| `ExampleCreate`, `ExampleDB_namespace`, `Example_basicUsage` | `wallet/walletdb/example_test.go:23,69,100` | Nothing: `//go:build disabled`, and written against an API (`db.Namespace`, `namespace.Update`) that no longer exists. | D |

#### `wallet/internal/snacl` (249 / 118)

Ten tests sharing package-level variables, so they pass only in source order
(`snacl_test.go:14-20`).

| Test | Cite | Pins | Class |
| --- | --- | --- | --- |
| `TestNewSecretKey` | `:22` | `NewSecretKey(password, 16384, 8, 1)` succeeds. | P+R |
| `TestMarshalSecretKey`, `TestUnmarshalSecretKey` | `:31`, `:35` | `Marshal` → `Unmarshal` → `DeriveKey(password)` reproduces the key. | P+R |
| `TestUnmarshalSecretKeyInvalid`, `TestDeriveKeyInvalid` | `:52`, `:113` | A wrong password → kind `Passphrase`. | P |
| `TestEncrypt`, `TestDecrypt` | `:66`, `:76` | Round trip of a 33-byte message. | P+R |
| `TestDecryptCorrupt` | `:89` | Adding 1 to the byte 15 from the end of the ciphertext makes `Decrypt` fail. | P |
| `TestZero` | `:98` | `Zero` clears the 32-byte key. | P |
| `TestDeriveKey` | `:107` | `DeriveKey` works again after `Zero`. | P |

No fixed ciphertext or marshalled-parameter vector exists: salts and nonces are random.

#### `wallet/txauthor` (234 / 239), `wallet/txrules` (281 / 93), `wallet/txsizes` (209 / 70)

| Test | Cite | Pins | Vectors | Class |
| --- | --- | --- | --- | --- |
| `TestNewUnsignedTransaction` | `wallet/txauthor/author_test.go:66` | `NewUnsignedTransaction` with an in-order input source, P2PKH-sized change, mainnet `MaxTxSize`: whether a change output exists, its amount, the number of inputs, and `InsufficientBalance`. | 13 cases (section 4.2.8) | P |
| `TestStakePoolTicketFee` | `wallet/txrules/poolfees_test.go:11` | `StakePoolTicketFee` on mainnet. | 7 rows (section 4.2.7) | P |
| `TestEstimateSerializeSize` | `wallet/txsizes/size_test.go:31` | `EstimateSerializeSize`. | 17 rows (section 4.2.5) | P |

Not tested: `IsDustAmount`, `IsDustOutput`, `CheckOutput`, `FeeForSerializeSize`
(directly), `PaysHighFees`, `TxPaysHighFees`, `ValidPoolFeeRate`,
`RandomizeOutputPosition`, `AddAllInputScripts`, the other three `txsizes` functions.

#### `wallet/udb` (15,269 / 4,327, of which 988 dead)

| Test | Cite | Pins | Vectors | Class |
| --- | --- | --- | --- | --- |
| `TestManagerImports` | `wallet/udb/addressmanager_test.go:433` | `ImportPrivateKey` of one testnet WIF and `ImportScript` of one 1-of-1 multisig script: address string, hash160, public key, account `ImportedAddrAccount`, `Imported()==true`, `Internal()==false`; lookups via `Address` unlocked and locked. (The second pass with `create=false` returns immediately.) | section 4.2.9 | P |
| `TestImportVotingAccount` | `:474` | `ImportVotingAccount(xpriv, pass, name)`: account numbers `ImportedAddrAccount+1…+3` in order; duplicate name → `Exist`; empty passphrase → `Passphrase`; empty name → `Invalid`; the account type is `importedVoting` and the stored xpriv decrypts to the input after `UnlockAccount`. | one `tprv…` key | P+R |
| `TestImportAccount` | `:606` | `importAccount(actBIP0044, xpriv, name)`: account numbers `ImportedAddrAccount+1` and `+2` in order; duplicate name → `Exist`; importing after an address of the same key is already indexed → `Exist`; empty name → `Invalid`. | same key | P+R |
| `TestManagerEncryptDecrypt` | `:1070` | `Encrypt`/`Decrypt` with an invalid key type fail; with `CKTPrivate` while locked → `Locked`; a failing private crypto key → `Crypto`; round trip under `CKTPublic` and `CKTPrivate`. | none | P (the failing-key case is G) |
| `TestChangePassphrase` | `:1092` | Public change with a failing key generator → `Crypto`; wrong old public passphrase → `Passphrase`; public change and change back; wrong old private passphrase → `Passphrase`; private change, unlock with the new one, change back while unlocked (stays unlocked). | none | P |
| `TestManagerWatchingOnly` | `:1146` | `testManagerAPI` (locking semantics, `NewAccount`, `LookupAccount`, `LastAccount`, `ForEachAccount`, `RenameAccount`) on a fresh manager, then after `ConvertToWatchingOnly`, then after reopen. Watching-only: `Lock`/`Unlock` → `WatchingOnly`. | account order `[0, 1, 2147483647]` | P |
| `TestManager` | `:1211` | `testManagerAPI` in create mode and again after reopen. **Skipped under `-short`**, which is how upstream CI runs. | same | P |
| `TestCoinTypes` | `wallet/udb/cointype_test.go:19` | `CoinTypes(params)`. | mainnet (20, 42), testnet3 (11, 1), simnet (115, 1) | P |
| `TestCoinTypeUpgrade` | `wallet/udb/cointype_test.go:59` | A new database starts on the legacy coin type; `UpgradeToSLIP0044CoinType` switches the coin-type key and the account 0 xpub to the SLIP0044 derivation; address 0/0 of accounts 0 and 1 are indexed after `SyncAccountToAddrIndex`; a second upgrade → `Invalid`. | derived from the fixed seed with `hdkeychain` | P+R |
| `TestStakeInvalidationOfTip` | `wallet/udb/stakevalidation_test.go:42` | A credit mined in block 2 that spends a credit mined in block 1: balance and unspent set before and after block 3 (vote bits 0) disapproves block 2. | constructed in test | P+R (dump the buckets through the oracle) |
| `TestSetBirthState` | `wallet/udb/txmined_test.go:32` | `SetBirthState` / `BirthState` round trip of random values. | none | P+R |
| `TestInsertsCreditsDebitsRollbacks` | `wallet/udb/tx_test.go:20` | Nine steps of unmined insert, credit, duplicate insert, mined insert, rollback, re-insert, mined double spend, rollback, unmined debit: spendable balance, unconfirmed balance, unspent set, unmined set after each. | constructed in test | P+R |
| `TestCoinbases` | `wallet/udb/tx_test.go:353` | Immature coinbase reward across heights 1, 7, 13, 16, 17 on testnet3 (maturity 16), after a spend, and after two rollbacks. | constructed in test | P+R |
| `TestUpgrades` | `wallet/udb/upgrades_test.go:42` | `Upgrade` of seven fixtures to version 26, with per-fixture checks (section 5.2.2). | 7 of the 8 `.db.gz` files | F |
| `TestVSPSerializeVSPHost`, `TestVSPSerializeVSPPubKey`, `TestVSPSerializeVSPTicket` | `wallet/udb/vsp_test.go:18,54,100` | Serialize/deserialize round trips; `Host` and `PubKey` of a `VSPTicket` are not part of its serialization. | none | R |
| `TestSetGetDeleteVSPTicket` | `wallet/udb/vsp_test.go:177` | Missing ticket → `NotExist`; set then get returns the full record **including** `Host` and `PubKey` (joined from the host and pubkey buckets); delete; delete of a missing ticket is not an error. | one record | P+R |
| `TestStoreQueries`, `TestPreviousPkScripts` | `wallet/udb/txquery_test.go:239,552` | Nothing: `//go:build ignore`; written against a pre-`walletdb` API (`s.InsertTx`, `testStore()`). | — | D (design notes only) |
| `ExampleStore_Rollback`, `Example_basicUsage` | `wallet/udb/txexample_test.go:19,107` | Nothing: `//go:build ignore`. | — | D |

### 3.3 Totals

| Quantity | Value |
| --- | ---: |
| Test files | 43 |
| Files with no test function (helpers) | 6 (`wallet/main_test.go`, `wallet/setup_test.go`, `wallet/network_test.go`, `wallet/udb/common_test.go`, `wallet/udb/internal_test.go`, `wallet/udb/txcommon_test.go`) |
| Files excluded by build tag | 3 (`wallet/walletdb/example_test.go`, `wallet/udb/txexample_test.go`, `wallet/udb/txquery_test.go`; 1,163 lines) |
| Live top-level `Test…` functions | 77 |
| `TestMain` functions | 2 (`wallet/main_test.go:17`, `wallet/udb/addressmanager_test.go:1251`) |
| Benchmarks, fuzz targets, runnable examples | 0 |
| Packages with Go source | 42 |
| Packages with at least one live test | 17 |

### 3.4 Test helpers, seams and hooks

| Symbol | Cite | Contract |
| --- | --- | --- |
| `logFlag` / `TestMain` | `wallet/main_test.go:15-24` | `-log` enables the `WLLT` logger on stderr for package `wallet` tests. |
| `testPrivPass` | `wallet/setup_test.go:18` | `"private"`. |
| `basicWalletConfig` | `wallet/setup_test.go:20-26` | Public passphrase `"public"` (`InsecurePubPassphrase`, `wallet/wallet.go:66`), gap limit 20, relay fee 1e5 atoms/kB, simnet, mixing enabled. |
| `testWallet(ctx, t, cfg, seed)` | `wallet/setup_test.go:28` | Creates a bdb file in `t.TempDir()`, `Create` then `Open`; a `nil` seed lets `Create` generate one. |
| `mockNetwork` | `wallet/network_test.go:20-40` | A `NetworkBackend` returning zero values; `Synced` returns `(false, 0)`. |
| `seed` (wallet and udb) | `wallet/addresses_test.go:46`, `wallet/udb/common_test.go:21` | The same 32 bytes: `b46bc6502a30beb92f0aebc776403c3dbf11bfb68305967c36dac9ef8d641567`. |
| `tg`, `tw`, `gblock`, `maketg`, `chaingenPrevScripter` | `wallet/reorg_test.go:20-58` | Wrap a `chaingen.Generator`; build `BlockNode`s with a version 2 filter computed by `blockcf2.Regular` assuming every spent script is P2SH of `OP_TRUE`. |
| `genTestBlockNodes` | `wallet/sidechains_test.go:17` | Chains of headers with random `ExtraData`, heights from 1 or from the parent. |
| `pubPassphrase`, `privPassphrase`, `pubPassphrase2`, `privPassphrase2` | `wallet/udb/common_test.go:30-33` | Four passphrases: `pubPassphrase` is 29 bytes (its Go literal contains the escape `\n`, one newline byte); the other three are 30 bytes. |
| `createEmptyDB`, `cloneDB` | `wallet/udb/common_test.go:47,70` | One template database is initialized on testnet3 and upgraded in `TestMain`; each test copies the file to a name in the package directory and opens it. |
| `pubPass` | `wallet/udb/upgrades_test.go:40` | `"public"`, the passphrase of the fixtures. |
| `tempDB` | `wallet/udb/txcommon_test.go:22` | Empty bdb file in `t.TempDir()`. |
| `blockGenerator`, `makeHeaderData`, `emptyFilters`, `makeBlockMeta`, `decodeHash` | `wallet/udb/txcommon_test.go:39-103` | Headers chained from the testnet3 genesis hash with only `PrevBlock`, `VoteBits`, `Height` set; empty version 2 filters. |
| `insertMainChainHeaders` | `wallet/udb/stakevalidation_test.go:20` | Calls `Store.ExtendMainChain` per header. |
| `newCoinBase`, `spendOutput` | `wallet/udb/tx_test.go:325,339` | Minimal transactions with zero-length scripts. |
| `TstLatestMgrVersion` | `wallet/udb/internal_test.go:22` | Pointer to `latestMgrVersion` (6, `wallet/udb/addressdb.go:21`). Unused by any test. |
| `TstRunWithReplacedNewSecretKey` | `wallet/udb/internal_test.go:27` | Swaps the package variable `newSecretKey` (`wallet/udb/addressmanager.go:240`) for one returning kind `Crypto`. |
| `TstCheckPublicPassphrase` | `wallet/udb/internal_test.go:40` | Derives the public master key with a candidate passphrase. |
| `failingCryptoKey`, `TstRunWithFailingCryptoKeyPriv` | `wallet/udb/internal_test.go:49-77` | Replaces `Manager.cryptoKeyPriv` with a key whose `Encrypt`/`Decrypt` fail. |
| `TstDefaultAccountName` | `wallet/udb/internal_test.go:80` | `"default"`. |
| mixing client hooks | `dcrd/mixing@v0.7.5/mixclient/testhooks.go:7-16`, `dcrd/mixing@v0.7.5/mixclient/client.go:410,767` | `testHooks` map on the client with stages `before run`, `before CT publish`, `before SR publish`, `before DC publish`; set only from in-package tests. Not reachable from dcrwallet. |

The remaining test-file declarations, one line each, so that the inventory is complete
(every `func`, `type`, `const` and `var` of the 43 files is either in section 3.2, in
the table above, or here):

| Symbol | Cite | Contract |
| --- | --- | --- |
| `depth(err)`, `eq(e0, e1)` | `errors/errors_test.go:12-34` | `depth` counts nested `*Error` wrappers; `eq` compares `Op`, `Kind` and `Err` by `==` (pointer identity for the wrapped error). `TestCollapse` requires depth 1 and field equality. |
| `serverMethods()` | `internal/rpc/jsonrpc/rpchelp_test.go:27-35` | The set of keys of the unexported `handlers` map whose entry does not have `noHelp`. Both help tests delete each `rpchelp.Methods` entry from it; a leftover is reported as `Missing '<locale>' help for method '<m>'` (the usage test builds the set but never checks leftovers, `:91-110`). |
| `keygen`, `ed25519Keygen()`, `ecKeygen(curve)` | `x509_test.go:26-46` | Key factories returning `(pub, priv, name)`: Ed25519 from a 32-byte random seed, ECDSA from `ecdsa.GenerateKey(curve, rand.Reader())`; names `ed25519`, `P-256`, `P-384`, `P-521` (Go's `Params().Name`). |
| `echo`, `testClientCert(t, pub, priv, name)` | `x509_test.go:62-121` | HTTP handler copying the request body to the response; one certificate round trip (section 5.2.10). |
| `expectedAddr`, `testContext` (wallet) | `wallet/addresses_test.go:19-32` | Vector row `{address, addressHash, branch, pubKey}`; context `{t, account, watchingOnly}`. |
| `hexToBytes` (wallet and udb) | `wallet/addresses_test.go:36`, `wallet/udb/common_test.go:38` | `hex.DecodeString` that panics on error. |
| `defaultAccount`, `defaultAccountName`, `waddrmgrBucketKey` (wallet tests) | `wallet/addresses_test.go:53-56` | `0`, `"default"`, `"waddrmgr"`. The non-test code has the same bucket name as `waddrmgrNamespaceKey` (`wallet/wallet.go:77`); the tests use both spellings for the one bucket. |
| `expectedInternalAddrs`, `expectedExternalAddrs` | `wallet/addresses_test.go:58-118` | The ten rows of section 4.2.3. |
| `newAddressFunc`, `testKnownAddresses`, `testAddresses` | `wallet/addresses_test.go:120-217` | Method-expression type for `(*Wallet).NewInternalAddress` / `NewExternalAddress`; the per-wallet check loop; the four combinations per lock state (section 5.2.11). |
| `accountIndexes`, `nextAddresses(n)`, `watchFutureAddresses`, `useAddress(child)` | `wallet/addresses_test.go:262-307` | Step functions of `TestAccountIndexes` (section 4.2.13). |
| `testUnlock`, `testLockOnBadPassphrase`, `testNoNilTimeoutReplacement`, `testNonNilTimeoutLock`, `testTimeoutReplacement` | `wallet/locking_test.go:33-150` | The five sub-scenarios of `TestLocking`, run in this order on one wallet with `w.Lock()` after each (section 5.2.9). |
| `MsgTxFromHex`, `mix0Hex`, `coinbaseTxHex` | `wallet/mixing_test.go:11-19, 75-127` | Hex decoder using `MsgTx.Deserialize`; the two raw transactions of `TestIsMixTx`. |
| `mockNetwork` methods `Blocks`, `CFiltersV2`, `PublishTransactions`, `PublishMixMessages`, `LoadTxFilter`, `Rescan`, `StakeDifficulty`, `Synced`, `WrapContext` | `wallet/network_test.go:22-40` | Every method returns zero values and a nil error; `WrapContext` returns the same context and a no-op cancel. This is the complete `NetworkBackend` method set at the pin; a scripted backend for `wallet.script` implements the same nine methods. |
| `(*tg).createBlockOne`, `(*tg).nextBlock`, `(*tg).blockHashByName`, `mustAddBlockNode`, `(*tw).evaluateBestChain`, `(*tw).assertNoBetterChain`, `(*tw).chainSwitch`, `(*tw).expectBlockInMainChain`, `assertSidechainTree`, `chaingenPrevScripter.PrevScript` | `wallet/reorg_test.go:47-153` | Section 5.2.6. `PrevScript` always returns script version 0, `true`, and the 22-byte sequence `a9 f5a8302ee8695bf836258b8f2b57b38a0be14e47 87`: `OP_HASH160`, the 20 hash bytes, `OP_EQUAL`, **without** the `0x14` push opcode a real P2SH script has (`:50-56`). For a block that spends such an output the filter entry would differ from the one a node computes from the real 23-byte script. Every test block is generated with no spend and no ticket spends (`nextBlock(name, nil, nil)`), so whether the helper's bytes ever enter a filter was not established (UNVERIFIED; settle by having `chain.gen` log `PrevScript` calls). The `chain.gen` fixture must reproduce the helper's bytes, not correct them, to stay equal to what the Go test feeds the wallet. |
| `AuthorTestChangeSource` (`Script`, `ScriptSize`), `p2pkhOutputs`, `makeInputSource` | `wallet/txauthor/author_test.go:20-64` | Change source returning 25 zero bytes, version 0, size 25; outputs with 36-byte zero scripts; an input source that appends unspents in order until the running total reaches the target, each input built by `wire.NewTxIn(&wire.OutPoint{}, value, nil)` with a 108-byte script size and a nil previous script, and that keeps its accumulated state across calls. |
| `p2pkhScriptSize`, `p2shScriptSize`, `makeScriptSizes`, `makeInts` | `wallet/txsizes/size_test.go:10-29` | 25 and 23; slice builders for section 4.2.5. |
| `testContext`, `expectedAddr` (udb) | `wallet/udb/addressmanager_test.go:27-49` | Context `{t, db, manager, account, create, unlocked, watchingOnly}`; vector row `{address, addressHash, internal, compressed, imported, pubKey, privKey, privKeyWIF, script}`. `compressed`, `privKey` and `privKeyWIF` are filled in but never compared. |
| `testNamePrefix`, `testManagedPubKeyAddress`, `testManagedScriptAddress`, `testAddress` | `wallet/udb/addressmanager_test.go:53-142` | Message prefix `Create account #N` / `Open account #N`; `testAddress` compares `Account()`, `Address().String()`, `AddrHash()`, `Internal()`, `Imported()`, then `PubKey()` for a `ManagedPubKeyAddress` or `AddrHash() == Hash160(script)` for a `ManagedScriptAddress`. |
| `testLocking`, `testImportPrivateKey`, `testImportScript`, `testChangePassphrase`, `testNewAccount`, `testLookupAccount`, `testRenameAccount`, `testForEachAccount`, `testEncryptDecryptErrors`, `testEncryptDecrypt`, `testManagerAPI` | `wallet/udb/addressmanager_test.go:147-1141` | Section 5.2.8. |
| `deriveChildAddress`, `equalExtKeys` (udb) | `wallet/udb/cointype_test.go:43-57` | `account.Child(branch).Child(child)` then `compat.HD2Address`; extended keys compared by their base58 strings. |
| `randomBytes`, `randomHash` | `wallet/udb/txmined_test.go:17-30` | Random inputs of `TestSetBirthState`. |
| `makeBlockGenerator`, `(*blockGenerator).generate(voteBits)`, `makeHeaderDataSlice` | `wallet/udb/txcommon_test.go:44-77` | Header `n` has `PrevBlock` = hash of header `n−1` (testnet3 genesis for the first), `Height` = `n`, the given `VoteBits`, every other field zero (timestamp included). `BlockHeaderData.BlockHash` is the header's real hash. |
| `emptyFilters(n)` | `wallet/udb/txcommon_test.go:79-85` | `gcs.FromBytesV2(blockcf2.B, blockcf2.M, nil)`: B = 19, M = 784931 (`dcrd/gcs/v4@v4.1.1/blockcf2/blockcf.go:33, 38`), no data. |
| `makeBlockMeta`, `decodeHash` | `wallet/udb/txcommon_test.go:87-103` | `BlockMeta{Hash, Height}` with a zero `Time`; `chainhash.NewHashFromStr` that panics. |
| `dbUpgradeTests`, `verifyV2Upgrade` … `verifyV12Upgrade` | `wallet/udb/upgrades_test.go:24-38, 86-482` | The fixture table and the seven verifiers of section 5.2.2. |
| `dbType`, `testContext`, `errSubTestFail`, `rollbackValues`, `testGetValues`, `testPutValues`, `testDeleteValues`, `testCursorDelete`, `testNestedReadWriteBucket`, `testReadWriteBucketInterface`, `testManualTxInterface`, `testNamespaceAndTxInterfaces`, `testAdditionalErrors`, `testInterface` | `wallet/walletdb/interface_test.go:26-709`, `wallet/internal/bdb/interface_test.go:34-705`, `wallet/walletdb/db_test.go:19`, `wallet/internal/bdb/driver_test.go:21` | The conformance suite of section 5.2.3 (the bdb copy has no `testCursorDelete`). `dbType` is `"bdb"`. |
| `caller` (`Call`), `zeroTx`, `mainnetParams` | `rpc/client/dcrwallet/methods_test.go:17-28` | A stub transport that unmarshals one canned JSON response into the result regardless of method and arguments; `zeroTx` is the JSON string `"010000000000000000000000000000"` (a 15-byte transaction: version 1, no inputs, no outputs, lock time 0, expiry 0, no witnesses). |
| `mnemonicTests` | `walletseed/seed_test.go:14-44` | The four `{mnemonic, seed}` rows of section 4.2.2, shared by the four seed tests. |
| `password`, `message`, `key`, `params`, `blob` | `wallet/internal/snacl/snacl_test.go:14-20` | Shared state of the ten snacl tests (section 4.2.14). |
| `queryState`, `newQueryState`, `deepCopy`, `deepCopyTxDetails`, `(*queryState).compare`, `equalTxDetails`, `equalTxs`, `timeNow`, `stripSerializedTx`; `exampleNum`, `exampleLoadDB` | `wallet/udb/txquery_test.go:22-237`, `wallet/walletdb/example_test.go:52` | Dead with their files (class D). `txquery_test.go:224` also declares a second `makeBlockMeta(height int32)`, which collides with the live helper of the same name in `txcommon_test.go:87`, so the file could not join the build even without its tag. |

Functions of the fixture generators (`wallet/udb/testdata/*.go`; none is built):

| Symbol | In | Contract |
| --- | --- | --- |
| `main`, `setup`, `compress` | all eight | `setup` creates `<name>.db` in the working directory with `walletdb.Create("bdb", …)`, calls `udb.Initialize` and populates it; `compress` gzips the file to `<name>.db.gz` with Go's default compression level and removes the uncompressed file (`v1.db.go:141-159`). |
| `ticketPurchaseHex` | `v2.db.go:40` | The serialized testnet2 ticket inserted into the legacy stake manager. |
| `privKey`, `ticketFeeLimits`, `votebits`, `subsidies`, `createUnsignedTicketPurchase`, `newVoteScript`, `createUnsignedVote`, `createUnsignedRevocation` | `v5.db.go:45-57, 263-433` | Imported private key `00…01`; fee-limit constant `0x5800`; vote bits 1; unsigned ticket, vote and revocation builders of that era. |
| `privKey`, `epoch` | `v7.go:36-39` | The same private key; a zero `time.Time` used as every record's received time and block time. |
| `pay2ssgen`, `pay2ssrtx`, `pay2sstx`, `pay2sstxChange`, `sstxAddrPush`, `dummyTxIn`, `stakeBaseTxIn`, `ticketSpendTxIn` | `v11.db.go:84-137` | Script and input builders for the six ticket scenarios. |

### 3.5 Test data inventory

All eight fixtures are gzip streams of a bbolt file with magic `0xED0CDAED`, bolt
format version 2 and page size 4096 (read from the meta page of each).

| Fixture | gzip bytes | bolt file bytes | DB version | Created on | Seed | Contents (from the generator) | Used by |
| --- | ---: | ---: | ---: | --- | --- | --- | --- |
| `v1.db.gz` | 127,077 | 524,288 | 1 | testnet2 | random | Accounts 0-8; per account both branches synced to N addresses with a given set of used child indexes (`v1.db.go:75-88`) | `verifyV2Upgrade` |
| `v2.db.gz` | 2,702 | 65,536 | 2 | testnet2 | random | One testnet2 ticket inserted into the legacy `wstakemgr` namespace with vote bits 1 and extended bits `00000004` (`v2.db.go:85-93`) | `verifyV3Upgrade` |
| `v3.db.gz` | 2,371 | 65,536 | 3 | testnet2 | random | Freshly initialized, nothing else (`v3.db.go:47-58`) | `verifyV4Upgrade` |
| `v4.db.gz` | 5,041 | 65,536 | 4 | testnet2 | random | Accounts 1-8 with last-used external/internal children (0,0), (9,9), (5,15), (19,20), (20,19), (29,30), (30,29), (2³¹−1, 2³¹−1) (`v4.db.go:75-102`) | `verifyV5Upgrade` |
| `v5.db.gz` | 3,751 | 65,536 | 5 | testnet2 | random | Private key `00…01` imported; 7 fake headers; 6 tickets: unmined; mined; mined + unmined vote; mined + mined vote; mined + unmined revocation; mined + mined revocation (`v5.db.go:138-237`) | `verifyV6Upgrade` |
| `v7.db.gz` | 3,931 | 65,536 | 7 | testnet2 | random | One block; 3 mined `sstxchange` credits with expiry 1000; 3 unmined transactions with expiry 1000; 3 mined regular transactions with no expiry (`v7.go:127-190`) | `verifyV8Upgrade` |
| `v11.db.gz` | 10,401 | 131,072 | 11 | testnet3 | 32 zero bytes | Accounts 1-12; six tickets, each voting to an odd account and committing 1000 atoms to the next account plus a 100-atom pool fee to a foreign address, in the six mined/unmined ticket/vote/revocation combinations (`v11.db.go:384-445`) | `verifyV12Upgrade` |
| `v24.db.gz` | 3,358 | 65,536 | 24 | testnet3 | 32 zero bytes | Freshly initialized (`v24.db.go:63-74`) | **nothing** |

All fixtures use public passphrase `public` and private passphrase `private`.

Generator programs are `package main` files inside `testdata`, so the Go tool ignores
them. Each states that it must be compiled at the commit that introduced it
(`v1.db.go:5-7`); they import long-gone paths (`github.com/decred/dcrwallet/wallet/udb`,
`github.com/decred/dcrutil/hdkeychain`, `decred.org/dcrwallet/v3/...`). They are
documentation of the fixture contents, not runnable tools.

---

## 4. Data formats

### 4.1 Upstream fixture format

A fixture is `gzip(bbolt file)`. `TestUpgrades` reads it with `compress/gzip`, copies the
decompressed bytes to `<tempdir>/test<i>.db` and opens that with
`walletdb.Open("bdb", path)` (`wallet/udb/upgrades_test.go:51-73`). The bbolt file
format is specified in spec 01. Facts the port's fixture loader can assert before use:

| Offset in decompressed file | Width | Value |
| --- | --- | --- |
| 16 | 4, little endian | `0xED0CDAED` (bolt magic) |
| 20 | 4, little endian | 2 (bolt format version) |
| 24 | 4, little endian | 4096 (page size) |

The fixtures must be carried over byte for byte. They cannot be regenerated: six of
the eight were created from a random seed (`walletseed.GenerateRandomSeed`,
`v1.db.go:53`), and every one contains random scrypt salts and secretbox nonces.

### 4.2 Vectors embedded in upstream tests

These are literal data. They are reproduced here so the port can be started without
the Go tree; the Go file remains the authority.

#### 4.2.1 PGP word list (`pgpwordlist/pgpwordlist_test.go:25-41`)

| Bytes (hex) | Words (even index → two-syllable list, odd index → three-syllable list) |
| --- | --- |
| `e58294f2e9a227486e8b061b31cc528fd7fa3f19` | `topmost Istanbul Pluto vagabond treadmill Pacific brackish dictator goldfish Medusa afflict bravado chatter revolver Dupont midsummer stopwatch whimsical cowbell bottomless` |
| `d1d464c004f00fb5c9a4c8d8e433e7fb7ff56256` | `stairway souvenir flytrap recipe adrift upcoming artist positive spearhead Pandora spaniel stupendous tonic concurrent transit Wichita lockup visitor flagpole escapade` |

Capitalization is part of the expected output (`Istanbul`, `Pluto`, `Pacific`,
`Medusa`, `Dupont`, `Pandora`, `Wichita`).

#### 4.2.2 Seed mnemonics (`walletseed/seed_test.go:14-44`)

The mnemonic is the word encoding of `seed || checksum byte`. Four vectors:

| Seed (hex) | Bytes | Checksum word (last word) |
| --- | ---: | --- |
| `e58294f2e9a227486e8b061b31cc528fd7fa3f19` | 20 | `fracture` |
| `d1d464c004f00fb5c9a4c8d8e433e7fb7ff56256` | 20 | `merit` |
| `e34cd132128c1929ec96865ced5c4d0bf40a5d021fcef58d27dbfee371d210` | 31 | `recipe` |
| 31 zero bytes | 31 | `insurgent` |

Full word strings for the third and fourth:

- `tissue disbelief stairway component atlas megaton bedlamp certify tumor monument necklace fascinate tunnel fascinate dreadful armistice upshot Apollo exceed aftermath billiard sardonic vapor microscope brackish suspicious woodlark torpedo hamlet sensation assume recipe`
- `aardvark adroitness` repeated 15 times, then `aardvark insurgent`.

The first two are the 4.2.1 words followed by the checksum word. `DecodeUserInput`
must accept each mnemonic and each seed's lower-case hex and return the seed.

#### 4.2.3 First addresses of a fixed-seed simnet wallet (`wallet/addresses_test.go:44-118`)

Seed `b46bc6502a30beb92f0aebc776403c3dbf11bfb68305967c36dac9ef8d641567`, simnet,
account 0, in the order returned by successive calls on a newly created and opened
wallet.

Internal branch (`NewInternalAddress`, branch 1):

| # | Address | Hash160 | Compressed public key |
| --- | --- | --- | --- |
| 1 | `SsrFKd8aX4KHabWSQfbmEaDv3BJCpSH2ySj` | `f032b89ec075ab2847e2ec186ad000be16cf354b` | `03d1ad44eeac8eb59e9598f7e530a1cbe2c1684c0aa5f45ab24d33d38a2102dd1a` |
| 2 | `SsW4roiFKWkbbhiAeEV5byet1pLKAP4xRks` | `12d5a8e19b9a070d6d5e6e425b593c2c137285e3` | `02cbcf5c1aa84bf8e6d04412d867eccbaa6cc12ebb792f3f1eaf4d2887f8e884f3` |
| 3 | `SscaK4A6V94dawc6ZBRCGUxPjdf7um1GJgD` | `5a38638f09937214b07481c656d0c9c73020f8bf` | `0392735a0eee9026425556ef5c5ae23ad3e54598132a1ca0d74dbcac7bfe31bfa4` |
| 4 | `Ssm4BeTKgwKGTqNR63WiGtP1FJaKCRJsN1S` | `b73edb8f32957800e2e3b9424c3b659acac51b7f` | `037c1e500884c6c3cb044390b52525d324fd67c031fdd9a47d742d0323fe5de73f` |
| 5 | `SssBoVxTkCUb6xs7vph3BHdPmki3weVvRsF` | `fa8073fcb670ba7312a1ef0d908cfb05c59b70b9` | `0327540e546f9cfac45f51699e2656732727507971060641ead554d78eeea88aa6` |

External branch (`NewExternalAddress`, branch 0):

| # | Address | Hash160 | Compressed public key |
| --- | --- | --- | --- |
| 1 | `SsfPTmZmaXGkXfcNGjftRPmoGGCqtNPCHKx` | `791376f67fb3becf392b071d25d7c99c82139ee3` | `031634efb3e83c834a82cdc898000f85215a09dc742d5b3b82ace7221ca1bb0938` |
| 2 | `SsXhSHBiaEan7Ls36bvhLspZ3LC1NKzuwQz` | `24b8b3d89f987bf3cd80a8c16d9368d683217fa4` | `0280beb72c6ef42ce3133fd6d340fd5bedcfccaded5a6eabb6d2430e3958bf7c85` |
| 3 | `SspSfaWDNwc9TA31Q9iR2jot2eV1hk2ix6U` | `dc67b3d95adb1789efe4aa73607d8a8c57eee2bb` | `03b120e0e073a12a1957680251a1562c5c6e30e547797fb5411107eac19699f601` |
| 4 | `SsrSYTB9MQQ1czAfPmWE66ZFqv7NrwzqfQT` | `f252015c8e0059c21cae623704d8588d12ca5c74` | `0350822c9bd61f524f4d68fa605e850c34c5e8ccc9b5cf278782131c1e21dd261b` |
| 5 | `SsqBcGBre8SZrG61cd5M5e2GaMJbK2CMdEa` | `e486d22d1244becac5a30b38dda1c8c4c1b3bdeb` | `031c494068c9c454bef7de35fa4717f21c07dec4471bd8500650b133d57e49a81d` |

The test does not assert the BIP0044 coin type or the child indexes these correspond
to (only the branch, via `Path()`). Which coin type (115 legacy or 1 SLIP0044) and
which child indexes produce them is to be established by the oracle command
`hd.derive` (section 5.5) and recorded next to the vector. UNVERIFIED here.

A second, external vector: the dcrd simnet setup script pairs seed
`b280922d2cffda44648346412c5ec97f429938105003730414f10b01e1402eac` with mining
address `SsXciQNTo3HuV5tX3yy4hXndRWgLMRVC7Ah` and transfer address
`SsWKp7wtdTZYabYFYSc9cnxhwFEjA5g4pFc` (dcrd repository,
`contrib/devtools/dcr_tmux_simnet_setup.sh:31-33`, local checkout, not a pin). Their
derivation paths are likewise to be established by the oracle.

#### 4.2.4 Voting extended private key (`wallet/wallet_test.go:103-145`)

| Input | Network | Output |
| --- | --- | --- |
| 32 zero bytes | mainnet | `dprv3oaf1h1hLxLZNHn6Gn9AaUaMJxhpHAj2KGMMHTi2AnoiHdRhCciKbwf3dB6zpPEq8ffdT4NZ7gjtrZBQhSWDm2RVXbphmpdPnbq299ddB8a` |
| 1 zero byte | mainnet | error of kind `Invalid` |

#### 4.2.5 Serialize-size estimates (`wallet/txsizes/size_test.go:31-59`)

`EstimateSerializeSize(inputScriptSizes, outputs, changeScriptSize)`. Constants:
`RedeemP2PKHSigScriptSize` = 108 (`wallet/txsizes/size.go:28`), `P2PKHPkScriptSize` =
25 (`:66`), `P2PKHOutputSize` = 36 (`:108`), `RedeemP2PKHInputSize` = 166 (`:55`).
A change script size of 0 means no change output.

| # | Inputs (each 108) | Output script lengths | Change script | Expected |
| ---: | --- | --- | ---: | ---: |
| 0 | 1 | none | 0 | 181 |
| 1 | 1 | 25 | 0 | 217 |
| 2 | 1 | none | 25 | 217 |
| 3 | 1 | 25 | 25 | 253 |
| 4 | 1 | 23 | 0 | 215 |
| 5 | 1 | 23 | 25 | 251 |
| 6 | 2 | none | 0 | 347 |
| 7 | 2 | 25 | 0 | 383 |
| 8 | 2 | none | 25 | 383 |
| 9 | 2 | 25 | 25 | 419 |
| 10 | 2 | 23 | 0 | 381 |
| 11 | 2 | 23 | 25 | 417 |
| 12 | 1 | 252 × 25 | 0 | 9253 |
| 13 | 1 | 253 × 25 | 0 | 9253 + 36 + 2 = 9291 |
| 14 | 1 | 252 × 25 | 25 | 9291 |
| 15 | 252 | none | 0 | 41847 |
| 16 | 253 | none | 0 | 41847 + 166 + 4 = 42017 |

Row 16's `+4` is the test's own note: the input count is encoded twice (prefix and
witness), so crossing 252 grows both compact integers from 1 to 3 bytes.

#### 4.2.6 Maturity predicates (`wallet/wallet_test.go:21-100`)

Mainnet parameters; `m` is `CoinbaseMaturity` = `TicketMaturity` = 256
(`dcrd/chaincfg/v3@v3.3.0/mainnetparams.go:553-555`); `e` = `TicketMaturity +
TicketExpiry`. Each row is `(txHeight, tipHeight) → result`.

| `coinbaseMatured` | `ticketMatured` | `ticketExpired` |
| --- | --- | --- |
| (0, 0) → false | (0, 0) → false | (0, 0) → false |
| (0, m−1) → false | (0, m−1) → false | (0, e−1) → false |
| (0, m) → **true** | (0, m) → **false** | (0, e) → **false** |
| (0, m+1) → true | (0, m+1) → true | (0, e+1) → true |
| (1, 0) → false | (1, 0) → false | (1, 0) → false |
| (m, 0) → false | (m, 0) → false | (e, 0) → false |
| (0, −1) → false | (0, −1) → false | (0, −1) → false |
| (−1, m) → false | (−1, m) → false | (−1, e) → false |
| (−1, 2³¹−1) → false | (−1, 2³¹−1) → false | (−1, 2³¹−1) → false |

The definitions are `tx ≥ 0 && tip − tx + 1 > CoinbaseMaturity`,
`tx ≥ 0 && tip − tx > TicketMaturity`, and `tx ≥ 0 && tip − tx > TicketMaturity +
TicketExpiry` in 32-bit signed arithmetic (`wallet/wallet.go:4422-4447`). The last row
exists because `2³¹−1 − (−1)` would wrap without the `tx ≥ 0` guard.

#### 4.2.7 Stake pool ticket fee (`wallet/txrules/poolfees_test.go:11-93`)

`StakePoolTicketFee(stakeDiff, relayFee, height, poolFeePercent, mainnet, dcp0010,
dcp0012)`, amounts in atoms.

| # | Stake difficulty | Relay fee | Height | Pool fee % | DCP0010 | DCP0012 | Expected |
| ---: | ---: | ---: | ---: | ---: | --- | --- | ---: |
| 0 | 1,000,000,000 | 1,000,000 | 25,000 | 1.00 | no | no | 1,500,463 |
| 1 | 2,000,000,000 | 1,000,000 | 25,000 | 1.00 | no | no | 1,621,221 |
| 2 | 500,000,000 | 5,000,000 | 50,000 | 2.59 | no | no | 3,310,616 |
| 3 | 1,500,000,000 | 5,000,000 | 50,000 | 2.59 | no | no | 3,956,376 |
| 4 | 1,500,000,000 | 5,000,000 | 50,000 | 2.59 | yes | no | 9,023,823 |
| 5 | 1,500,000,000 | 5,000,000 | 50,000 | 2.59 | no | yes | 9,784,185 |
| 6 | 1,500,000,000 | 5,000,000 | 50,000 | 2.59 | yes | yes | 9,784,185 |

#### 4.2.8 Unsigned transaction authoring (`wallet/txauthor/author_test.go:66-195`)

Setup: inputs are offered in order, each with a 108-byte signature-script estimate;
the change source returns a 25-byte zero script, version 0; `maxTxSize` is mainnet's
393,216. **The test's output scripts are 36 bytes long** (`p2pkhOutputs` allocates
`txsizes.P2PKHOutputSize` bytes for the script, `author_test.go:34`), not 25.

Let `fee(r, n, outs)` = `FeeForSerializeSize(r, EstimateSerializeSize(n × [108], outs,
25))`, where `outs` are the test outputs with their 36-byte scripts, and
`FeeForSerializeSize(r, size)` = `r × size / 1000` in 64-bit signed integer
arithmetic, then raised to `r` when the quotient is 0 and `r > 0`, then replaced by
`MaxAmount` when negative or above `MaxAmount` (`wallet/txrules/rules.go:94-106`).
Neither adjustment is reached by these 13 cases (row 12 has `r = 0`, giving fee 0).

| # | Unspent (atoms) | Outputs (atoms) | Relay fee / kB | Expected |
| ---: | --- | --- | ---: | --- |
| 0 | 1e8 | 1e8 | 1,000 | error kind `InsufficientBalance` |
| 1 | 1e8 | 1e6 | 1,000 | 1 input; change = 1e8 − 1e6 − fee(1000, 1, [1e6]) |
| 2 | 1e8 | 1e6 | 10,000 | 1 input; change = 1e8 − 1e6 − fee(10000, 1, [1e6]) |
| 3 | 1e8 | 1e6, 1e6, 1e6 | 10,000 | 1 input; change = 1e8 − 3e6 − fee(10000, 1, 3 outputs) |
| 4 | 1e8 | 1e6, 1e6, 1e6 | 2,550 | 1 input; change = 1e8 − 3e6 − fee(2550, 1, 3 outputs) |
| 5 | 1e8 | 1e8 − 602 − fee(1000, 1, 1 output) | 1,000 | 1 input; no change (602 is dust) |
| 6 | 1e8 | 1e8 − 603 − fee(1000, 1, 1 output) | 1,000 | 1 input; change = 603 |
| 7 | 1e8 | 1e8 − 1537 − fee(2550, 1, 1 output) | 2,550 | 1 input; no change |
| 8 | 1e8 | 1e8 − 1538 − fee(2550, 1, 1 output) | 2,550 | 1 input; change = 1538 |
| 9 | 1e8, 1e8 | 1e8 − 603 − fee(1000, 1, 1 output) | 1,000 | 1 input; change = 603 |
| 10 | 1e8, 1e8 | 1e8 − 545 − fee(1000, 1, 1 output) | 1,000 | 1 input; no change (a second input is not pulled in to rescue dust change) |
| 11 | 1e8, 1e8 | 1e8 | 1,000 | 2 inputs; change = 1e8 − fee(1000, 2, [1e8]) |
| 12 | 1e8 | 1e8 | 0 | 1 input; no change |

The dust boundary pinned by rows 5-8: for a 25-byte change script, 603 atoms is the
smallest non-dust amount at 1,000 atoms/kB and 1,538 at 2,550 atoms/kB. The absolute
fee values are to be emitted by the oracle (`tx.author`), not computed by hand.

#### 4.2.9 Address manager imports (`wallet/udb/addressmanager_test.go:231-360, 481-483`)

Testnet3.

| Item | Value |
| --- | --- |
| WIF | `PtWUqkS3apLoZUevFtG3Bwt6uyX8LQfYttycGkt2XCzgxquPATQgG` |
| → address | `TsSYVKf24LcrxyWHBqj4oBcU542PcjH1iA2` |
| → hash160 | `10b601a41d2320527c95eb4cdae2c75b45ae45e1` |
| → public key | `03df8852b90ce8da7de6bcbacd26b78534ad9e46dc1b62a01dcf43f5837d7f9f5e` |
| → private key (listed, not asserted) | `ac4cb1a53c4f04a71fffbff26d4500c8a95443936deefd1b6ed89727a6858e08` |
| Script | `51210373c717acda38b5aa4c00c33932e059cdbc11deceb5f00490a9101704cc444c5151ae` |
| → P2SH address | `TcsXPUraiDWZoeQBEbw7T7LSgrvD7dar9DA` |
| → hash160 | `db7e6d507e3e291a5ab2fac10107f4479c1f4f9c` |
| Account xpriv for import tests | `tprvZUo1ZuEfLLFWg9ktEaXHf3HKq2EdfwY5pXFZ2rbg6HFzeaoJDp1qFZnuuuN6iUAG9EyNF4sH4RmJb395XWYpdqQoXRoKkV88HZwgq95KfiK` |

#### 4.2.10 Small literal vectors

| Subject | Vector | Cite |
| --- | --- | --- |
| Coin types | mainnet (legacy 20, SLIP0044 42); testnet3 (11, 1); simnet (115, 1) | `wallet/udb/cointype_test.go:26-28` |
| Possible coinjoin | mainnet transaction `ab70b9b3fc88feb7be0c1a1b9ba47cac9dea10f158911fd9cfaf3af3f80878f3` (hex in the test) → mix, denomination 268,435,456, 11 mixed outputs | `wallet/mixing_test.go:24-48, 76-121` |
| Outpoint list marshalling | `[]` → `[]`; one → `["679c5086…94ec:0"]`; two → `["679c5086…94ec:69","7bc19eb0…bfd1:1001"]` | `rpc/client/dcrwallet/marshaling_test.go:18-41` |
| Throttle | threshold 1, two concurrent requests → `{200: 1, 429: 1}`; the 429 body is `429 Too Many Requests` followed by a newline (written by `http.Error`; the test checks only the status codes) | `internal/rpc/jsonrpc/rpcserver_test.go:15-53`, `internal/rpc/jsonrpc/server.go:296` |
| Account iteration order | `[0, 1, 2147483647]` after creating one extra account | `wallet/udb/addressmanager_test.go:952` |
| Reserved account names | empty → `Invalid`; `imported` → `Invalid` | `wallet/udb/addressmanager_test.go:862-871` |
| Upgrade expectations | section 5.2.2 | `wallet/udb/upgrades_test.go:86-482` |

#### 4.2.11 JSON-RPC command cases (`rpc/jsonrpc/types/methods_test.go:23-1306`, `walletsvrwscmds_test.go:22-192`)

Every case has the envelope `{"jsonrpc":"1.0","method":"<method>","params":<params>,"id":1}`
with no whitespace and the members in that order; the method is the first word of the
case name. For each case three things must hold (`methods_test.go:1242-1305`):

1. `MarshalCmd("1.0", 1, <typed constructor result>)` equals the envelope byte for
   byte.
2. `NewCmd(method, args…)` with untyped arguments, marshalled the same way, equals the
   same bytes.
3. `ParseParams(method, params)` of those bytes deep-equals the "parsed" column. In
   that column a bare value on an optional (pointer) field is a non-nil pointer to that
   value, `nil` is a nil pointer, a field not written is the zero value (nil for
   pointers), and `&[]T{}` is a non-nil pointer to an empty slice, which is distinct
   from `nil`.

Cases 0-78 are in `methods_test.go`, 79-85 in `walletsvrwscmds_test.go`.

| # | Case name | Params | Parsed | Cite |
| ---: | --- | --- | --- | --- |
| 0 | `addmultisigaddress` | `[2,["031234","035678"]]` | `AddMultisigAddressCmd{NRequired: 2, Keys: []string{"031234", "035678"}, Account: nil}` | `methods_test.go:35` |
| 1 | `addmultisigaddress optional` | `[2,["031234","035678"],"test"]` | `AddMultisigAddressCmd{NRequired: 2, Keys: []string{"031234", "035678"}, Account: "test"}` | `methods_test.go:51` |
| 2 | `createmultisig` | `[2,["031234","035678"]]` | `CreateMultisigCmd{NRequired: 2, Keys: []string{"031234", "035678"}}` | `methods_test.go:67` |
| 3 | `createnewaccount` | `["acct"]` | `CreateNewAccountCmd{Account: "acct"}` | `methods_test.go:82` |
| 4 | `dumpprivkey` | `["1Address"]` | `DumpPrivKeyCmd{Address: "1Address"}` | `methods_test.go:95` |
| 5 | `getaccount` | `["1Address"]` | `GetAccountCmd{Address: "1Address"}` | `methods_test.go:108` |
| 6 | `getaccountaddress` | `["acct"]` | `GetAccountAddressCmd{Account: "acct"}` | `methods_test.go:121` |
| 7 | `getaddressesbyaccount` | `["acct"]` | `GetAddressesByAccountCmd{Account: "acct"}` | `methods_test.go:134` |
| 8 | `getbalance` | `[]` | `GetBalanceCmd{Account: nil, MinConf: 1}` | `methods_test.go:147` |
| 9 | `getbalance optional1` | `["acct"]` | `GetBalanceCmd{Account: "acct", MinConf: 1}` | `methods_test.go:161` |
| 10 | `getbalance optional2` | `["acct",6]` | `GetBalanceCmd{Account: "acct", MinConf: 6}` | `methods_test.go:175` |
| 11 | `getnewaddress` | `[]` | `GetNewAddressCmd{Account: nil, GapPolicy: nil}` | `methods_test.go:189` |
| 12 | `getnewaddress optional` | `["acct","ignore"]` | `GetNewAddressCmd{Account: "acct", GapPolicy: "ignore"}` | `methods_test.go:203` |
| 13 | `getrawchangeaddress` | `[]` | `GetRawChangeAddressCmd{Account: nil}` | `methods_test.go:217` |
| 14 | `getrawchangeaddress optional` | `["acct"]` | `GetRawChangeAddressCmd{Account: "acct"}` | `methods_test.go:230` |
| 15 | `getreceivedbyaccount` | `["acct"]` | `GetReceivedByAccountCmd{Account: "acct", MinConf: 1}` | `methods_test.go:243` |
| 16 | `getreceivedbyaccount optional` | `["acct",6]` | `GetReceivedByAccountCmd{Account: "acct", MinConf: 6}` | `methods_test.go:257` |
| 17 | `getreceivedbyaddress` | `["1Address"]` | `GetReceivedByAddressCmd{Address: "1Address", MinConf: 1}` | `methods_test.go:271` |
| 18 | `getreceivedbyaddress optional` | `["1Address",6]` | `GetReceivedByAddressCmd{Address: "1Address", MinConf: 6}` | `methods_test.go:285` |
| 19 | `gettransaction` | `["123"]` | `GetTransactionCmd{Txid: "123", IncludeWatchOnly: false}` | `methods_test.go:299` |
| 20 | `gettransaction optional` | `["123",true]` | `GetTransactionCmd{Txid: "123", IncludeWatchOnly: true}` | `methods_test.go:313` |
| 21 | `importprivkey` | `["abc"]` | `ImportPrivKeyCmd{PrivKey: "abc", Label: nil, Rescan: true}` | `methods_test.go:327` |
| 22 | `importprivkey optional1` | `["abc","label"]` | `ImportPrivKeyCmd{PrivKey: "abc", Label: "label", Rescan: true}` | `methods_test.go:342` |
| 23 | `importprivkey optional2` | `["abc","label",false]` | `ImportPrivKeyCmd{PrivKey: "abc", Label: "label", Rescan: false}` | `methods_test.go:357` |
| 24 | `importprivkey optional3` | `["abc","label",false,12345]` | `ImportPrivKeyCmd{PrivKey: "abc", Label: "label", Rescan: false, ScanFrom: 12345}` | `methods_test.go:372` |
| 25 | `importpubkey` | `["abc","label",false,12345]` | `ImportPubKeyCmd{PubKey: "abc", Label: "label", Rescan: false, ScanFrom: 12345}` | `methods_test.go:388` |
| 26 | `listaccounts` | `[]` | `ListAccountsCmd{MinConf: 1}` | `methods_test.go:404` |
| 27 | `listaccounts optional` | `[6]` | `ListAccountsCmd{MinConf: 6}` | `methods_test.go:417` |
| 28 | `listlockunspent` | `[]` | `ListLockUnspentCmd{}` | `methods_test.go:430` |
| 29 | `listreceivedbyaccount` | `[]` | `ListReceivedByAccountCmd{MinConf: 1, IncludeEmpty: false, IncludeWatchOnly: false}` | `methods_test.go:441` |
| 30 | `listreceivedbyaccount optional1` | `[6]` | `ListReceivedByAccountCmd{MinConf: 6, IncludeEmpty: false, IncludeWatchOnly: false}` | `methods_test.go:456` |
| 31 | `listreceivedbyaccount optional2` | `[6,true]` | `ListReceivedByAccountCmd{MinConf: 6, IncludeEmpty: true, IncludeWatchOnly: false}` | `methods_test.go:471` |
| 32 | `listreceivedbyaccount optional3` | `[6,true,false]` | `ListReceivedByAccountCmd{MinConf: 6, IncludeEmpty: true, IncludeWatchOnly: false}` | `methods_test.go:486` |
| 33 | `listreceivedbyaddress` | `[]` | `ListReceivedByAddressCmd{MinConf: 1, IncludeEmpty: false, IncludeWatchOnly: false}` | `methods_test.go:501` |
| 34 | `listreceivedbyaddress optional1` | `[6]` | `ListReceivedByAddressCmd{MinConf: 6, IncludeEmpty: false, IncludeWatchOnly: false}` | `methods_test.go:516` |
| 35 | `listreceivedbyaddress optional2` | `[6,true]` | `ListReceivedByAddressCmd{MinConf: 6, IncludeEmpty: true, IncludeWatchOnly: false}` | `methods_test.go:531` |
| 36 | `listreceivedbyaddress optional3` | `[6,true,false]` | `ListReceivedByAddressCmd{MinConf: 6, IncludeEmpty: true, IncludeWatchOnly: false}` | `methods_test.go:546` |
| 37 | `listsinceblock` | `[]` | `ListSinceBlockCmd{BlockHash: nil, TargetConfirmations: 1, IncludeWatchOnly: false}` | `methods_test.go:561` |
| 38 | `listsinceblock optional1` | `["123"]` | `ListSinceBlockCmd{BlockHash: "123", TargetConfirmations: 1, IncludeWatchOnly: false}` | `methods_test.go:576` |
| 39 | `listsinceblock optional2` | `["123",6]` | `ListSinceBlockCmd{BlockHash: "123", TargetConfirmations: 6, IncludeWatchOnly: false}` | `methods_test.go:591` |
| 40 | `listsinceblock optional3` | `["123",6,true]` | `ListSinceBlockCmd{BlockHash: "123", TargetConfirmations: 6, IncludeWatchOnly: true}` | `methods_test.go:606` |
| 41 | `listtransactions` | `[]` | `ListTransactionsCmd{Account: nil, Count: 10, From: 0, IncludeWatchOnly: false}` | `methods_test.go:621` |
| 42 | `listtransactions optional1` | `["acct"]` | `ListTransactionsCmd{Account: "acct", Count: 10, From: 0, IncludeWatchOnly: false}` | `methods_test.go:637` |
| 43 | `listtransactions optional2` | `["acct",20]` | `ListTransactionsCmd{Account: "acct", Count: 20, From: 0, IncludeWatchOnly: false}` | `methods_test.go:653` |
| 44 | `listtransactions optional3` | `["acct",20,1]` | `ListTransactionsCmd{Account: "acct", Count: 20, From: 1, IncludeWatchOnly: false}` | `methods_test.go:669` |
| 45 | `listtransactions optional4` | `["acct",20,1,true]` | `ListTransactionsCmd{Account: "acct", Count: 20, From: 1, IncludeWatchOnly: true}` | `methods_test.go:686` |
| 46 | `listunspent` | `[]` | `ListUnspentCmd{MinConf: 1, MaxConf: 9999999, Addresses: nil}` | `methods_test.go:703` |
| 47 | `listunspent optional1` | `[6]` | `ListUnspentCmd{MinConf: 6, MaxConf: 9999999, Addresses: nil}` | `methods_test.go:718` |
| 48 | `listunspent optional2` | `[6,100]` | `ListUnspentCmd{MinConf: 6, MaxConf: 100, Addresses: nil}` | `methods_test.go:733` |
| 49 | `listunspent optional3` | `[6,100,["1Address","1Address2"]]` | `ListUnspentCmd{MinConf: 6, MaxConf: 100, Addresses: &[]string{"1Address", "1Address2"}}` | `methods_test.go:748` |
| 50 | `lockunspent` | `[true,[{"txid":"123","vout":1,"tree":0}]]` | `LockUnspentCmd{Unlock: true, Transactions: []TransactionInput{ {Txid: "123", Vout: 1} }}` | `methods_test.go:764` |
| 51 | `renameaccount` | `["oldacct","newacct"]` | `RenameAccountCmd{OldAccount: "oldacct", NewAccount: "newacct"}` | `methods_test.go:783` |
| 52 | `sendfrom` | `["from","1Address",0.5]` | `SendFromCmd{FromAccount: "from", ToAddress: "1Address", Amount: 0.5, MinConf: 1, Comment: nil, CommentTo: nil}` | `methods_test.go:797` |
| 53 | `sendfrom optional1` | `["from","1Address",0.5,6]` | `SendFromCmd{FromAccount: "from", ToAddress: "1Address", Amount: 0.5, MinConf: 6, Comment: nil, CommentTo: nil}` | `methods_test.go:815` |
| 54 | `sendfrom optional2` | `["from","1Address",0.5,6,"comment"]` | `SendFromCmd{FromAccount: "from", ToAddress: "1Address", Amount: 0.5, MinConf: 6, Comment: "comment", CommentTo: nil}` | `methods_test.go:833` |
| 55 | `sendfrom optional3` | `["from","1Address",0.5,6,"comment","commentto"]` | `SendFromCmd{FromAccount: "from", ToAddress: "1Address", Amount: 0.5, MinConf: 6, Comment: "comment", CommentTo: "commentto"}` | `methods_test.go:852` |
| 56 | `sendmany` | `["from",{"1Address":0.5}]` | `SendManyCmd{FromAccount: "from", Amounts: map[string]float64{"1Address": 0.5}, MinConf: 1, Comment: nil}` | `methods_test.go:871` |
| 57 | `sendmany optional1` | `["from",{"1Address":0.5},6]` | `SendManyCmd{FromAccount: "from", Amounts: map[string]float64{"1Address": 0.5}, MinConf: 6, Comment: nil}` | `methods_test.go:888` |
| 58 | `sendmany optional2` | `["from",{"1Address":0.5},6,"comment"]` | `SendManyCmd{FromAccount: "from", Amounts: map[string]float64{"1Address": 0.5}, MinConf: 6, Comment: "comment"}` | `methods_test.go:905` |
| 59 | `sendtoaddress` | `["1Address",0.5]` | `SendToAddressCmd{Address: "1Address", Amount: 0.5, Comment: nil, CommentTo: nil}` | `methods_test.go:922` |
| 60 | `sendtoaddress optional1` | `["1Address",0.5,"comment","commentto"]` | `SendToAddressCmd{Address: "1Address", Amount: 0.5, Comment: "comment", CommentTo: "commentto"}` | `methods_test.go:938` |
| 61 | `sendfromtreasury` | `["key",{"1Address":0.5}]` | `SendFromTreasuryCmd{Key: "key", Amounts: map[string]float64{"1Address": 0.5}}` | `methods_test.go:955` |
| 62 | `sendtotreasury` | `[0.5]` | `SendToTreasuryCmd{Amount: 0.5}` | `methods_test.go:970` |
| 63 | `settspendpolicy` | `["hash","policy"]` | `SetTSpendPolicyCmd{Hash: "hash", Policy: "policy", Ticket: nil}` | `methods_test.go:983` |
| 64 | `settspendpolicy optional` | `["hash","policy","ticket"]` | `SetTSpendPolicyCmd{Hash: "hash", Policy: "policy", Ticket: "ticket"}` | `methods_test.go:998` |
| 65 | `settreasurypolicy` | `["key","policy"]` | `SetTreasuryPolicyCmd{Key: "key", Policy: "policy", Ticket: nil}` | `methods_test.go:1013` |
| 66 | `settreasurypolicy optional` | `["key","policy","ticket"]` | `SetTreasuryPolicyCmd{Key: "key", Policy: "policy", Ticket: "ticket"}` | `methods_test.go:1028` |
| 67 | `settxfee` | `[0.0001]` | `SetTxFeeCmd{Amount: 0.0001}` | `methods_test.go:1043` |
| 68 | `signmessage` | `["1Address","message"]` | `SignMessageCmd{Address: "1Address", Message: "message"}` | `methods_test.go:1056` |
| 69 | `signrawtransaction` | `["001122"]` | `SignRawTransactionCmd{RawTx: "001122", Inputs: nil, PrivKeys: nil, Flags: "ALL"}` | `methods_test.go:1070` |
| 70 | `signrawtransaction optional1` | `["001122",[{"txid":"123","vout":1,"tree":0,"scriptPubKey":"00","redeemScript":"01"}]]` | `SignRawTransactionCmd{RawTx: "001122", Inputs: &[]RawTxInput{ { Txid: "123", Vout: 1, ScriptPubKey: "00", RedeemScript: "01" } }, PrivKeys: nil, Flags: "ALL"}` | `methods_test.go:1086` |
| 71 | `signrawtransaction optional2` | `["001122",[],["abc"]]` | `SignRawTransactionCmd{RawTx: "001122", Inputs: &[]RawTxInput{}, PrivKeys: &[]string{"abc"}, Flags: "ALL"}` | `methods_test.go:1118` |
| 72 | `signrawtransaction optional3` | `["001122",[],[],"ALL"]` | `SignRawTransactionCmd{RawTx: "001122", Inputs: &[]RawTxInput{}, PrivKeys: &[]string{}, Flags: "ALL"}` | `methods_test.go:1136` |
| 73 | `sweepaccount - optionals provided` | `["default","DsUZxxoHJSty8DCfwfartwTYbuhmVct7tJu",6,0.05]` | `SweepAccountCmd{SourceAccount: "default", DestinationAddress: "DsUZxxoHJSty8DCfwfartwTYbuhmVct7tJu", RequiredConfirmations: 6, FeePerKb: 0.05}` | `methods_test.go:1155` |
| 74 | `sweepaccount - optionals omitted` | `["default","DsUZxxoHJSty8DCfwfartwTYbuhmVct7tJu"]` | `SweepAccountCmd{SourceAccount: "default", DestinationAddress: "DsUZxxoHJSty8DCfwfartwTYbuhmVct7tJu"}` | `methods_test.go:1173` |
| 75 | `walletlock` | `[]` | `WalletLockCmd{}` | `methods_test.go:1187` |
| 76 | `walletpassphrase` | `["pass",60]` | `WalletPassphraseCmd{Passphrase: "pass", Timeout: 60}` | `methods_test.go:1198` |
| 77 | `walletpassphrasechange` | `["old","new"]` | `WalletPassphraseChangeCmd{OldPassphrase: "old", NewPassphrase: "new"}` | `methods_test.go:1212` |
| 78 | `walletpubpassphrasechange` | `["old","new"]` | `WalletPubPassphraseChangeCmd{OldPassphrase: "old", NewPassphrase: "new"}` | `methods_test.go:1226` |
| 79 | `getunconfirmedbalance` | `[]` | `GetUnconfirmedBalanceCmd{Account: nil}` | `walletsvrwscmds_test.go:34` |
| 80 | `getunconfirmedbalance optional1` | `["acct"]` | `GetUnconfirmedBalanceCmd{Account: "acct"}` | `walletsvrwscmds_test.go:47` |
| 81 | `listaddresstransactions` | `[["1Address"]]` | `ListAddressTransactionsCmd{Addresses: []string{"1Address"}, Account: nil}` | `walletsvrwscmds_test.go:60` |
| 82 | `listaddresstransactions optional1` | `[["1Address"],"acct"]` | `ListAddressTransactionsCmd{Addresses: []string{"1Address"}, Account: "acct"}` | `walletsvrwscmds_test.go:74` |
| 83 | `listalltransactions` | `[]` | `ListAllTransactionsCmd{Account: nil}` | `walletsvrwscmds_test.go:89` |
| 84 | `listalltransactions optional` | `["acct"]` | `ListAllTransactionsCmd{Account: "acct"}` | `walletsvrwscmds_test.go:102` |
| 85 | `walletislocked` | `[]` | `WalletIsLockedCmd{}` | `walletsvrwscmds_test.go:115` |

What the cases pin beyond the obvious:

- Defaults applied by `ParseParams` when a trailing optional parameter is omitted:
  `minconf` 1 (`getbalance`, `getreceivedbyaccount`, `getreceivedbyaddress`,
  `listaccounts`, `listreceivedbyaccount`, `listreceivedbyaddress`, `listunspent`,
  `sendfrom`, `sendmany`); `includewatchonly` false (`gettransaction`,
  `listreceivedby*`, `listsinceblock`, `listtransactions`); `includeempty` false;
  `rescan` true (`importprivkey`); `count` 10 and `from` 0 (`listtransactions`);
  `targetconfirmations` 1 (`listsinceblock`); `maxconf` 9999999 (`listunspent`);
  `flags` `"ALL"` (`signrawtransaction`). Optional parameters without a default stay
  nil (`account`, `gappolicy`, `label`, `scanfrom`, `comment`, `commentto`, `ticket`,
  `addresses`, `inputs`, `privkeys`, `requiredconfirmations`, `feeperkb`).
- The marshalled form omits trailing optional parameters that are nil. No case has a
  nil optional followed by a non-nil one, so how a gap would be written is not pinned
  here; cases 71 and 72 fill the gap with an explicit empty array `[]`.
- `lockunspent` and `signrawtransaction` inputs marshal the transaction input as
  `{"txid":…,"vout":…,"tree":0…}`: the `tree` member is always present, and in case 70
  the member order is `txid`, `vout`, `tree`, `scriptPubKey`, `redeemScript`.
- Amounts are JSON numbers written in Go's shortest float form: `0.5`, `0.0001`,
  `0.05`. `sendmany` and `sendfromtreasury` carry a one-entry object
  `{"1Address":0.5}` (a single entry, so map order is not exercised).
- Cases 32 and 36 pass `false` explicitly for `includewatchonly`; the parsed value is
  a non-nil pointer to `false`, indistinguishable from the default.
- No case supplies a wrong type, too few or too many parameters; the dcrjson error
  codes and texts for those are not pinned upstream (`rpc.cmd` and `diff_rpc_cmd`
  cover them).

#### 4.2.12 dcrd result decoders (`rpc/client/dcrd/json_test.go:21-379`)

The same inputs are applied to `hashes` (a slice of hash pointers) and
`hashesContiguous` (a slice of hash values).

| Decoder | Input | Result |
| --- | --- | --- |
| `hashes`, `hashesContiguous`, `headers`, `transactions` | `null` | no error, zero elements |
| the same four | `["` (two bytes) | an error; its kind is not checked |
| `hashes`, `hashesContiguous` | `["Invalid Hash 1","Invalid Hash 2"]` | kind `Encoding` |
| `hashes`, `hashesContiguous` | `["b751872e37415e433250d6c8af37441632724ed920b71072b0fc5f9604472633","d41988665c526f7d484c14e1c2a0973f6b0556ed5a0e28c4584977e21ef6d270"]` | two hashes whose `String()` (the byte-reversed hex form) equals the input strings, in order |
| `headers` | `["Invalid Header 1","Invalid Header 2"]` | kind `Encoding` |
| `headers` | the hex of two serialized 180-byte headers, built from: header 0 `{Version 6, Timestamp 1533513600, Bits 1, SBits 20000000, Nonce 0x18aea41a, StakeVersion 6}`; header 1 `{Version 1, Timestamp 1454954400, Bits 0x1b01ffff, SBits 200000000, Nonce 0, StakeVersion 3}`; every other field zero | two headers equal field for field (`json_test.go:212-252`) |
| `transactions` | `[1234]` | kind `Encoding` |
| `transactions` | `["Invalid Transaction 1","Invalid Transaction 2"]` | kind `Encoding` |
| `transactions` | the hex of two serialized transactions with no inputs and no outputs: `{Version 1, LockTime 0}` and `{Version 123, LockTime 69420}`, full serialization type, expiry 0 | two transactions equal to the originals; the decoded input and output lists are empty, non-nil slices (`:291-329`) |
| `hash` | a JSON string of 63 `f` characters; of 65 | kind `Encoding` |
| `hash` | `"` + 65 `f` with no closing quote; 65 `f` + `"` with no opening quote | kind `Encoding` |
| `hash` | a JSON string of 64 `f` characters | the all-`ff` hash |

Encoders: a zero-value `hashes` or `hashesContiguous` marshals to the four bytes
`null`; the two hashes above marshal to `["b751…2633","d419…d270"]` with no whitespace
(`:70-100, 151-181`). The decoders are reused across inputs in one test, so a
successful decode must replace, not append to, what an earlier call left
(`:27-58`).

#### 4.2.13 Address buffer states (`wallet/addresses_test.go:219-260`, `wallet/discovery_test.go:16-101`)

Each external or internal branch of an account has an in-memory buffer with two
unsigned 32-bit fields: `lastUsed` (the highest child index known to be used;
`0xFFFFFFFF` when none) and `cursor` (how many addresses past `lastUsed` have been
handed out). The next child index returned is `lastUsed + 1 + cursor` in wrapping
32-bit arithmetic (`wallet/addresses.go:449-452, 606`).

`TestAccountIndexes`: simnet wallet from a random seed, gap limit 20, a `mockNetwork`
backend set, account 0. After each step, the external and internal buffers must be:

| # | Step | External (`lastUsed`, `cursor`) | Internal |
| ---: | --- | --- | --- |
| 0 | none (fresh wallet) | (none, 0) | (none, 0) |
| 1 | `NewExternalAddress` × 1 | (none, 1) | (none, 0) |
| 2 | `NewExternalAddress` × 19 (all 20 calls succeed; the cursor reaches the gap limit) | (none, 20) | (none, 0) |
| 3 | `watchHDAddrs(ctx, false, backend)` | (none, 20) | (none, 0) |
| 4 | mark external child 10 used, then `watchHDAddrs` | (10, 9) | (none, 0) |
| 5 | `NewExternalAddress` × 1 | (10, 10) | (none, 0) |
| 6 | `NewExternalAddress` × 10 | (10, 20) | (none, 0) |
| 7 | mark external child 30 used, then `watchHDAddrs` | (30, 0) | (none, 0) |
| 8 | mark external child 31 used, then `watchHDAddrs` | (31, 0) | (none, 0) |

"Mark child N used" derives the address from the branch extended public key held in
the buffer, loads it with `Manager.Address` inside a read-write transaction, and calls
the unexported `markUsedAddress("", dbtx, ma)` (`addresses_test.go:285-306`). Step 4
keeps the next index at 20 (10 + 1 + 9): marking an address used never moves the next
address backwards. Steps 7 and 8 mark addresses at and beyond the last one handed out
(children 30 and 31 when 0-30 have been returned), which requires those addresses to
exist in the database already; they do because the wallet derives and watches
addresses ahead of the cursor.

`TestDiscoveryCursorPos`: simnet wallet from a random seed with
`DisableCoinTypeUpgrades = true`, account 0, external branch.

| # | Action | `lastUsed` | `cursor` |
| ---: | --- | --- | ---: |
| 0 | set the in-memory `cursor` to 9 directly (nothing persisted) | none | 9 |
| 1 | `DiscoverActiveAddresses(ctx, &mockNetwork{}, &genesisHash, false, gapLimit)` | none | 9 |
| 2 | in one read-write transaction: `MarkReturnedChildIndex(dbtx, 0, 0, 9)`, then mark child 4 used; then discovery again with the same arguments | 4 | 4 |

The next index is 9 before and after (`0xFFFFFFFF + 1 + 9` and `4 + 1 + 4`). The
discovery calls pass `discoverAccounts = false` and the genesis hash as the starting
block; with a backend that returns no blocks or filters the wallet "only records the
genesis block" (`discovery_test.go:44`).

#### 4.2.14 Further literal inputs

| Subject | Values | Cite |
| --- | --- | --- |
| Semver compatibility | compatible: (1.2.3, 1.2.3), (450.378.210, 450.378.210), (78.94.80, 78.94.80); not compatible, as (required, actual): (1.2.3, 2.0.0), (1.2.4, 1.0.2), (1.3.6, 1.3.2) | `chain/semver_test.go:15-51` |
| `errors` matching | `e = E(errors.New("base error"), Op("operation"), Permission)`: `Is(e, base)` true; `Is(e, E("different error"))` false; `Is(e, E(Op("operation")))` true; `Is(e, E(Op("different operation")))` false; `Is(e, E(Permission))` true; `Is(e, E(Invalid))` false | `errors/errors_test.go:49-70` |
| snacl inputs | password `sikrit`; message `this is a secret message of sorts` (33 bytes); wrong passwords `wrong password` and `bogus`; parameters `DefaultN`, `DefaultR`, `DefaultP`; corruption is `blob[len-15] += 1` (wrapping byte addition) | `wallet/internal/snacl/snacl_test.go:15-16, 24, 59, 90, 114` |
| Manager encryption plaintext | `this is a plaintext`, under `CKTPublic` then `CKTPrivate`; invalid key type `0xff`; empty plaintext for the error cases | `wallet/udb/addressmanager_test.go:977-1061` |
| VSP ticket record | ticket hash `0a472047bfcf1a19580412536570e351d2ac3f30638491c1c1268d6ada514858`; `FeeHash` `679c508625503e5cd00ba8d35badc40ce040df8986c11321758db831b78f94ec`; `FeeTxStatus` 123; `VSPHostID` 321; `Host` `example.com`; `PubKey` the 14 bytes `not-a-real-key` | `wallet/udb/vsp_test.go:186-195` |
| VSP serialization round trips | `VSPHost`: `my host`, and an empty host; `VSPPubKey`: (ID 69, `my public key`), (ID 0, `my public key`), (ID 69, empty key); `VSPTicket`: all fields set, zero fee hash, zero status, zero host ID | `wallet/udb/vsp_test.go:21-140` |
| `signrawtransaction` result with errors | `{"hex":"010000000000000000000000000000","complete":false,"errors":[{"txid":"bfc0e650ad0cc0dd5fa88b6bc84beb5ea4a675b4353671532796171ed319341b","vout":0,"scriptSig":"0123456789abcdef","sequence":123,"error":"bad things happened"},{"txid":"79dd1f6a5b7fa43407d47ff203259efe9a78453605ffd07b4db1703fed339066","vout":1,"scriptSig":"abcdef0123456789","sequence":456,"error":"other things happened"}]}` (whitespace in the test literal omitted here); and `{"hex":"010000000000000000000000000000","complete":true}` | `rpc/client/dcrwallet/methods_test.go:26, 47-66, 112-115` |
| Outpoint list hashes | `679c508625503e5cd00ba8d35badc40ce040df8986c11321758db831b78f94ec` with index 0 and 69; `7bc19eb0bf3a57be73d6879b6c411404b14b0156353dd47c5e0456768704bfd1` with index 1001 | `rpc/client/dcrwallet/marshaling_test.go:18-41` |
| Conformance suite keys | bucket checks: `bucketkey1`-`bucketkey3`; manual transactions: `umtxkey1`-`umtxkey3`; managed transactions: `mtxkey1`-`mtxkey3`; persistence test: `ns1key1`-`ns1key3` in top-level bucket `ns1`; values `foo1`-`foo3` throughout; nested bucket name `testbucket`; top-level buckets `ns1`, `ns2`, `ns3` | `wallet/walletdb/interface_test.go:143-147, 208, 434-438, 505-509, 630, 696-701`, `wallet/internal/bdb/driver_test.go:106-111` |
| Tickets of the v5 fixture | `7bc19eb0bf3a57be73d6879b6c411404b14b0156353dd47c5e0456768704bfd1`, `a6abeb0127c347b5f38ebc2401134b324612d5b1ad9a9b8bdf6a91521842b7b1`, `1107757fa4f238803192c617c7b60bf35bdc57bc0fc94b408c71239ff9eaeb98`, `3fd00cda28c4d148e0cd38e1d646ba1365116b3ddd9a49aca4483bef80513ff9`, `f4bdebefaa174470182960046fa53f554108b8ea09a86de5306a14c3a0124566`, `bca8c2649860585f10b27d774b354ea7b80007e9ad79c090ea05596d63995cf5` (display form, as `chainhash.NewHashFromStr` reads them) | `wallet/udb/upgrades_test.go:256-261` |
| Client certificate round trip | request `PUT` with body `balls` (trusted case) or `test` (untrusted case); server TLS config: minimum version TLS 1.2, `RequireAndVerifyClientCert`, a client CA pool holding only the authority certificate | `x509_test.go:88-102, 155-178` |

### 4.3 Oracle wire protocol (proposed; identical to dcroxide's)

One JSON object per line on stdin, one per line on stdout, in order
(`dcroxide:tools/oracle/main.go:24-36`):

| Direction | Field | Type | Meaning |
| --- | --- | --- | --- |
| request | `cmd` | string | command name, dotted by family (section 5.5) |
| request | `data` | string | hex of the command's single binary argument; layouts are per command, big-endian length prefixes |
| request | `id` | unsigned 64-bit, optional | echoed in the response |
| response | `result` | string | hex or text, per command |
| response | `error` | string | Go error text, on failure |
| response | `kind` | string | the dcrwallet `errors.Kind` name (or a dependency's error-kind name) when the failure carries one |
| response | further fields | per command | named in the command's doc comment |

A line that does not decode, or exceeds 64 MiB, is answered with an error without an
id and the next line is served. A panic ends the process after the preceding answers
are flushed. Commands whose natural argument is structured (a database script, an RPC
type name with a JSON body) carry a JSON document hex-encoded in `data`, so the
framing stays one-argument.

### 4.4 Vector file format (proposed)

Frozen vectors are line-oriented UTF-8 text under `crates/<crate>/tests/data/`,
`<subject>_vectors.txt`, as in dcroxide:

- `#` starts a comment line. The first comment block names the exporter, the dcrwallet
  commit, the Go release, and the command that regenerates the file.
- Every other line is `<row-kind> <field> <field> …`, fields separated by single
  spaces, binary fields in lower-case hex, an empty binary field written as `-`.
- A file whose content depends on a clock or a random stream records that input as its
  first row (`now <unix seconds>`, `seed <hex>`), as dcroxide's full block battery
  records its generation time (`dcroxide:tools/fullblockgen/main.go:25-27`).

### 4.5 Canonical database dump (proposed)

The logical content of a wallet database, independent of page layout. Produced by the
oracle command `db.dump` (using `go.etcd.io/bbolt` v1.3.12 directly, because the
`walletdb` interface has no way to enumerate top-level buckets) and by the Rust
implementation, and compared as text.

```
B <depth> <hex bucket name>        entering a bucket; depth 0 is top level
K <hex key> <hex value>            a key/value pair of the current bucket
E                                  leaving the current bucket
```

Rules: buckets and keys are visited in bbolt cursor order (ascending byte order of
the key; a nested bucket sorts by its name among the keys of its parent); an empty
value is `-`; nothing else is emitted. Bucket sequence numbers are not dumped
(dcrwallet never calls `NextSequence`).

Two derived forms:

- **Masked dump.** Fields that hold wall-clock time or random bytes are replaced by
  `*` of the same length, by rule, before comparison (section 5.6.4).
- **Plaintext dump.** For a database whose passphrases are known, encrypted values
  are replaced by `P:<hex plaintext>` using the oracle command `db.decrypt`, so that
  two databases with different nonces compare equal when their secrets are equal.

### 4.6 Session capture formats (proposed)

All captures are directories with a `manifest.json` (dcrwallet commit and Go release,
network, wallet fixture name, backend, command line) and one numbered record file per
exchange.

| Interface | Record content | Compared as |
| --- | --- | --- |
| JSON-RPC over HTTP POST | request body bytes; response status line, the headers the wallet sets, body bytes | exact bytes after the masks of section 5.8.4 |
| JSON-RPC over websocket | each text frame in order with a direction marker, including notifications | exact bytes per frame; notification order within a block is part of the record |
| gRPC | service/method, request message bytes, each response message's bytes in order, final status code and message | decoded field-by-field, and additionally byte-equal where the message has no masked field (`rpc/api.proto` has no `map`, `oneof` or `optional` field, so encodings are canonical in field-number order) |
| Wallet → dcrd websocket (RPC mode) | every request the wallet sends and every response/notification it receives | request multiset and per-method parameter bytes; ordering constraints are listed per scenario (spec 15) |
| Wallet ↔ peer (SPV) | every wire message in each direction, as framed bytes | message types and payload bytes after masking nonces and timestamps (spec 14) |
| Wallet → VSP HTTP | method, path, request body, the `VSP-Client-Signature` header | exact bytes for a given ticket, key and clock value: the request bodies carry a `Timestamp` taken from `time.Now().Unix()` (`wallet/feepayment.go:253, 375, 521`), so the clock is pinned (section 5.6.3) or the field and the signature over it are masked. That the signature is deterministic for a fixed body is UNVERIFIED here (settle in spec 11 by reading the wallet's message-signing path); server responses are fixtures |
| Pipe IPC (`--pipetx`) | each framed message: 1-byte version, 1-byte type length, type, 4-byte little-endian payload length, payload (`ipc.go:18-23`) | exact bytes except bound addresses |
| stdout/stderr/log file | lines | after stripping the timestamp prefix |

---

## 5. Algorithms and state machines

Sections 5.1-5.2 describe what the Go suite does, in the order it does it. Sections
5.3-5.13 are the procedures of the proposed rig.

### 5.1 How the upstream suite runs

1. The only entry point is `run_tests.sh`: `go test -short -vet=all "$@" ./...`
   (`run_tests.sh:9`). CI runs it on `ubuntu-latest` under Go 1.26 and Go 1.27 after
   `go build ./...` (`.github/workflows/go.yml:8-24`). There is no race-detector run,
   no lint job, no integration job and no other operating system.
2. Because of `-short`, `TestManager` is skipped in CI
   (`wallet/udb/addressmanager_test.go:1212-1214`). Its create-mode half is also run
   by `TestManagerWatchingOnly`; its second half (the API checks on a reopened,
   non-watching manager) runs nowhere in CI.
3. Package `wallet/udb` has a `TestMain` (`addressmanager_test.go:1251-1274`):
   1. create a directory `udb-*` under the system temporary directory;
   2. set `emptyDbPath` to `<dir>/empty.kv`;
   3. `createEmptyDB`: `walletdb.Create("bdb", emptyDbPath)`, then
      `Initialize(ctx, db, TestNet3Params, seed, pubPassphrase, privPassphrase)`, then
      `Upgrade(ctx, db, pubPassphrase, TestNet3Params)`, then close
      (`common_test.go:47-66`);
   4. run the tests; remove the directory; exit with the tests' code.
4. Each udb test that needs a database calls `cloneDB(ctx, t, name)`
   (`common_test.go:70-97`): read the whole template file, write it to `name` (a
   relative path, so the package directory) with mode 0644, `walletdb.Open`, then
   `udb.Open(ctx, db, TestNet3Params, pubPassphrase)`; cleanup closes and removes the
   copy. Three tests share the name `mgr_watching_only.kv` (`TestManagerWatchingOnly`,
   `TestManager`, `TestSetBirthState`); they are not parallel, so they do not collide.
5. Package `wallet` has a `TestMain` that only parses `-log` (`main_test.go:17-24`).
   Each wallet test builds its own database through `testWallet`.
6. `wallet/internal/snacl` tests depend on source order (section 3.2).
7. `wallet/internal/bdb` and `wallet/walletdb` tests create `noexist.db` (never
   created), `createfail.db`, `persistencetest.db`, `interfacetest.db` and
   `dupdrivertest.db` in the package directory and remove them.

### 5.2 Upstream scenarios worth stating exactly

#### 5.2.1 Transaction store step sequence (`wallet/udb/tx_test.go:20-323`)

Setup: three headers b1, b2, b3 (vote bits "valid") chained from the testnet3 genesis
hash and inserted with empty filters. `tx1` has one output of 2e8 and no inputs;
`sTx1` spends `tx1:0`; `tx2` has one output of 3e8; `sTx2` spends `tx2:0`. Each step
runs in its own read-write transaction and is followed by four checks at
`minConf = 1`, account 0: `AccountBalance.Spendable`, `AccountBalance.Unconfirmed`,
the set of `UnspentOutputs`, the set of `UnminedTxs`.

| Step | Operation | Spendable | Unconfirmed | Unspent | Unmined |
| --- | --- | ---: | ---: | --- | --- |
| new store | none | 0 | 0 | ∅ | ∅ |
| txout insert | `InsertMemPoolTx(tx1)`, `AddCredit(tx1, nil, 0, false, 0)` | 0 | 2e8 | tx1:0 | tx1 |
| insert duplicate unconfirmed | `InsertMemPoolTx(tx1)` again (no error) | 0 | 2e8 | tx1:0 | tx1 |
| confirmed txout insert | `InsertMinedTx(tx1, b1)` | 2e8 | 0 | tx1:0 | ∅ |
| rollback confirmed credit | `Rollback(1)` | 0 | 2e8 | tx1:0 | tx1 |
| insert duplicate confirmed | re-insert the three headers, `InsertMinedTx(tx1, b1)` (its error is discarded by the test, `:200-201`) | 2e8 | 0 | tx1:0 | ∅ |
| insert confirmed double spend | `InsertMinedTx(sTx1, b2)` twice (no error) | 0 | 0 | ∅ | ∅ |
| rollback after spending tx | `Rollback(2)` | 0 | 0 | ∅ | sTx1 |
| insert unconfirmed debit | `InsertMemPoolTx(sTx2)` | 0 | 0 | ∅ | sTx1, sTx2 |

`sTx1` has an output but no credit is ever added for it, which is why the unspent set
is empty after it is mined. `sTx2` spends an output (`tx2:0`) that is not in the store.

#### 5.2.2 Upgrade test (`wallet/udb/upgrades_test.go:42-482`)

For each of seven fixtures, in parallel sub-tests: decompress to a temporary file, open,
`Upgrade(ctx, db, "public", TestNet3Params)`, then verify. All seven must reach version
26 without error. Six of them were created on testnet2 and are upgraded with testnet3
parameters.

| Fixture | Verified after upgrade |
| --- | --- |
| `v1.db.gz` | `udb.Open` succeeds. Last account number is 8. For accounts 0-8 the address-pool meta keys for both branches are gone. `AccountProperties` last-used external and internal index are equal and are, by account: none (`0xFFFFFFFF`), 18, 19, 19, 25, 29, 29, 185, 199. The used-address bucket no longer exists. |
| `v2.db.gz` | `udb.Open` succeeds. The top-level agenda preferences bucket exists. (The original per-ticket rewrite of this step was removed with the legacy stake pool code, `:162-166`.) |
| `v3.db.gz` | The `waddrmgr` main bucket has no seed key. |
| `v4.db.gz` | Reading each account row with the version-5 row reader (`fetchDBAccount(ns, acct, 5)`): last-used and last-returned child are equal per branch and are (ext, int): account 0 (none, none); 1 (0, 0); 2 (9, 9); 3 (5, 15); 4 (19, 20); 5 (20, 19); 6 (29, 30); 7 (30, 29); 8 (2³¹−1, 2³¹−1); imported account (0, 0). |
| `v5.db.gz` | The tickets bucket holds exactly six hashes (`7bc19eb0…bfd1`, `a6abeb01…b7b1`, `1107757f…eb98`, `3fd00cda…3ff9`, `f4bdebef…4566`, `bca8c264…5cf5`, full values at `:256-261`), each with picked height −1. The unmined-inputs bucket has no entry for the stakebase outpoint (zero hash, index `0xFFFFFFFF`). |
| `v7.db.gz` | Every mined credit and every unmined credit value has the has-expiry flag set. The transaction records bucket holds three records with a non-zero expiry and three with expiry 0. |
| `v11.db.gz` | `udb.Open` succeeds. The two ticket-commitment buckets exist. `AccountBalances(minConf 0)`: accounts 1 and 3 have voting authority 1100 and nothing else; accounts 2 and 4 have locked-by-tickets 1000 and total 1000; accounts 5, 7, 9, 11 have no balance entry; accounts 6 and 8 have total 1300 and immature stake generation 1300; accounts 10 and 12 have total 700 and immature stake generation 700. Spendable and unconfirmed are 0 everywhere. |

Upgrade steps with no dedicated fixture or assertion: 6→7 (noted as
backwards-compatible, `:33`), 8→9 (the comment at `:35` says the v7 fixture's
check still applies), 9→10 (a `TODO`, `:36`), 10→11 (not mentioned at all), and every
step from 12→13 through 25→26. Those later steps do execute on all
seven fixtures, but only their not failing is observed.

#### 5.2.3 Database conformance suite (`wallet/walletdb/interface_test.go`)

For each of two top-level buckets `ns1`, `ns2`:

1. Create the top-level bucket in an `Update`.
2. Manual transactions on it: a read transaction sees the bucket; a read-write
   transaction that runs the bucket checks and puts three keys, rolled back, leaves
   nothing; the same committed leaves the three keys; a further read-write transaction
   deletes them.
3. Bucket checks (`testReadWriteBucketInterface`, run on the bucket and, recursively,
   on a nested bucket to depth two): put three keys (`bucketkey1`-`bucketkey3`), get
   them, `ForEach` sees exactly them, delete them, get returns nil, put them again
   (they stay in the bucket; `interface_test.go:198`); `CreateBucket("testbucket")`; creating it again
   → kind `Exist`; `CreateBucketIfNotExists` returns it; `NestedReadWriteBucket`
   returns it; `DeleteNestedBucket` removes it; deleting again → kind `NotExist`;
   `CreateBucketIfNotExists` creates it; delete it.
4. Managed transactions: an `Update` whose function returns an error rolls back and
   returns that same error; a successful `Update` persists; a later `View` reads it.
5. Delete the top-level bucket.

Then on `ns3`: `CreateBucket(nil)`, `DeleteNestedBucket(nil)` and `Put(nil, nil)` →
kind `Invalid`; `Rollback` after `Rollback` → `Invalid`; `Commit` after `Rollback` →
`Invalid`.

Differences between the two copies of the suite. The bdb copy
(`wallet/internal/bdb/interface_test.go`) omits the "put them again" part of step 3,
so its nested-bucket checks run on an otherwise empty bucket; it compares values with
`bytes.Equal`, which treats a nil and an empty slice alike. The walletdb copy compares
with `reflect.DeepEqual` (`wallet/walletdb/interface_test.go:59, 167`), which does not:
`Get` of a key that was deleted, rolled back or never written must return nil rather
than an empty slice. The suite never stores an empty value, so what `Get` returns for
a key stored with an empty value is not pinned. `ns3` is created by the error checks and never
deleted (`:630-658`). Manual transactions are checked in the order: read transaction
(bucket visible, then `Rollback`), read-write with rollback, read-write with commit,
delete (`:440-470`); a read transaction is ended with `Rollback`, and its `Rollback`
returns nil.

#### 5.2.4 Coinbase maturity sequence (`wallet/udb/tx_test.go:353-574`)

Setup on a clone of the testnet3 template (coinbase maturity 16): 19 headers from the
generator of section 3.4, all with vote bits "valid" (only 17 are inserted; 18 empty
filters are made). The coinbase has one input whose previous outpoint is the zero
hash with index `0xFFFFFFFF` and tree 0, no scripts, and three outputs of 20e8, 10e8
and 30e8 atoms with empty scripts; its record is created with a zero received time.
The whole sequence runs inside **one** read-write transaction. After each step the
test reads `AccountBalance(dbtx, minConf 0, account 0)`.

| # | Operation | `ImmatureCoinbaseRewards` | Spendable in the test table (not asserted, quirk 7) |
| ---: | --- | ---: | ---: |
| 1 | insert header 1; `InsertMinedTx(coinbase, block 1)`; `AddCredit(coinbase, block 1 meta, index 0, change false, account 0)`; the same for index 2 | 50e8 | 0 |
| 2 | insert headers 2-7 | 50e8 | 0 |
| 3 | insert headers 8-13 | 50e8 | 0 |
| 4 | insert headers 14-16 | 50e8 | 0 |
| 5 | insert header 17 | 0 | 50e8 |
| 6 | `InsertMinedTx(spenderA, block 17)`, where `spenderA` spends coinbase output 0 (tree 0) and has outputs 5e8 and 15e8; no credit is added for it; its received time is the wall clock | 0 | 30e8 |
| 7 | `Rollback(dbtx, 17)` | 30e8 | 0 |
| 8 | `Rollback(dbtx, 1)` | 0 | 0 |

The coinbase mined at height 1 is immature through tip height 16 and mature at tip
height 17. Step 7 removes block 17; the value 30e8 is what remains if `spenderA`
moves back to the unmined set (as rollbacks do in section 5.2.1) and still spends
output 0, leaving only output 2 as immature reward. Output 1
(10e8) never has a credit.

#### 5.2.5 Stake invalidation of the tip (`wallet/udb/stakevalidation_test.go:42-187`)

Three headers from the same generator: 1 and 2 with vote bits "valid"
(`dcrutil.BlockValid`), 3 with vote bits 0. `block1Tx` has no inputs and one output
of 2e8; `block2Tx` has one input spending `block1Tx:0` (tree 0) and one output of
1e8. Both are zero-value `wire.MsgTx` otherwise (version 0, empty scripts) and both
records have a zero received time. Everything runs in one read-write transaction.

| # | Operation | Check |
| ---: | --- | --- |
| 1 | `InsertMemPoolTx(block1Tx)`, `AddCredit(block1Tx, nil, 0, false, 0)`, `InsertMemPoolTx(block2Tx)`, `AddCredit(block2Tx, nil, 0, false, 0)` | `AccountBalance(minConf 0, account 0).Total` = 1e8 |
| 2 | insert headers 1 and 2; `InsertMinedTx(block1Tx, block 1)`; `InsertMinedTx(block2Tx, block 2)` | `AccountBalance(minConf 1, account 0).Total` = 1e8; `UnspentOutputs` has exactly one credit: `block2Tx`, amount 1e8 |
| 3 | insert header 3 (vote bits 0, which disapproves block 2's regular tree) | `AccountBalance(minConf 1, account 0).Total` = 2e8; `UnspentOutputs` has exactly one credit: `block1Tx`, amount 2e8 |

Step 3 changes the answers without any transaction being inserted or removed: the
disapproval alone makes `block2Tx` (and its spend of `block1Tx:0`) not count.

#### 5.2.6 Sidechain forest, chain switch and birthday (`wallet/reorg_test.go:155-238`, `wallet/sidechains_test.go:43-140`, `wallet/wallet_test.go:147-231`)

`TestReorg`, on a simnet wallet with an empty `SidechainForest`. A block node is
`{Header, Hash, FilterV2}`. `chainSwitch` in the test re-adds every node of the
returned previous chain to the forest and requires `MainChainTip` to equal the last
node of the new chain.

1. Generate `block-one` (`CreateBlockOne(name, 0)`), add its node. `EvaluateBestChain`
   returns a chain of length 1 ending in it. `ChainSwitch` to it. The forest must then
   hold no tree, and `EvaluateBestChain` must return an empty chain.
2. Generate `2a` and `3a` on top, add both nodes. The forest holds one tree rooted at
   `2a` with the single tip `3a`. `EvaluateBestChain` returns length 2 ending in `3a`;
   switch; the forest is empty; no better chain.
3. Set the generator tip back to `block-one`; generate `2b` and `3b`; add both. One
   tree rooted at `2b` with tip `3b`. `EvaluateBestChain` returns an empty chain: a
   side chain of equal length does not win.
4. Generate `4b`, add it. One tree rooted at `2b` with tip `4b`. `EvaluateBestChain`
   returns length 3 ending in `4b`; switch.
5. After the switch (and the re-adding of the two removed nodes) the forest holds
   exactly one tree, rooted at `2a` with tip `3a`; no better chain;
   `BlockInMainChain` returns `(have, invalidated)` = (false, false) for `2a` and `3a`
   and (true, false) for `2b`, `3b`, `4b`.

`AddBlockNode` must return true for every node added in steps 1-4 (the re-adds in
`chainSwitch` ignore the result).

`TestPruneChainFromSRT`. Nodes carry only `Height`, `PrevBlock` and 32 random
`ExtraData` bytes. Tree: `a → b0 → c0 → d0 → e0` (chain0, heights 1-5); `a → b1 → c1
→ d1 → e1` (chain1); `a → b2 → c2 → d2 → e2` (chain2); `c2 → d3 → e3` (chain3). The
tree is rooted at `a`; every other node must attach (`maybeAttachNode` returns true)
in the order chain0 without its root, chain1, chain2, chain3.

| Call | Result |
| --- | --- |
| `pruneChain(chain0)` on the full tree | two trees, in either order: root `b1` with children {`c1`, `d1`, `e1`} and tips {`e1`}; root `b2` with children {`c2`, `d2`, `e2`, `d3`, `e3`} and tips {`e2`, `e3`} |
| `pruneChain([b1, c1])` on the first | one tree: root `d1`, children {`e1`}, tips {`e1`} |
| `pruneChain([d1, e1])` on that | no trees |

The children map excludes the root and is keyed by hash; identity of the node objects
is asserted, which a port expresses as equality of hashes.

`TestSetBirthStateAndScan`. Nine blocks `1a`-`9a` are generated directly from the
generator's genesis tip with `NextBlock` (there is no `CreateBlockOne` call here),
added, and switched to in one chain of length 9. `before` is the wall clock minus one
minute, taken before the blocks are generated. Five sub-tests run in order on the same
wallet; each calls `SetBirthStateAndScan(ctx, state)` and then reads `BirthState`:

| Sub-test | State passed | Expected `BirthState().Hash` |
| --- | --- | --- |
| `ok middle` | `SetFromHeight`, `Height` 6 | hash of `6a` |
| `ok genesis` | `SetFromHeight`, `Height` 0 | simnet genesis hash |
| `ok from now` | `SetFromTime`, `Time` = now + 1 minute | hash of `9a` (the tip) |
| `ok from before` | `SetFromTime`, `Time` = `before` | simnet genesis hash |
| `nothing to set` | zero value | an error (kind not checked); state not read |

No network backend is set on the wallet, so the "scan" half is exercised only as far
as it goes without one.

#### 5.2.7 Account import cases (`wallet/udb/addressmanager_test.go:474-717`)

Both tests use the `tprv…` key of section 4.2.9 on a clone of the testnet3 template,
one read-write transaction per case, in this order. The `watchingOnly` flag of the
first case only changes the test's message prefix; the manager is never watching-only
here.

`TestImportVotingAccount`: `ImportVotingAccount(dbtx, xpriv, passphrase, name)`.

| # | Manager state | Name | Passphrase | Result |
| ---: | --- | --- | --- | --- |
| 0 | locked | `account 1` | `abc` | account number `ImportedAddrAccount + 1` (2³¹) |
| 1 | locked | `account 2` | `abc` | `ImportedAddrAccount + 2` |
| 2 | unlocked with the private passphrase | `account 3` | `abc` | `ImportedAddrAccount + 3` |
| 3 | locked | `account 1` | `abc` | kind `Exist` |
| 4 | locked | `account 4` | empty | kind `Passphrase` |
| 5 | locked | empty | `abc` | kind `Invalid` |

After each success: `UnlockAccount(dbtx, number, "abc")` succeeds; the loaded account
has type `importedVoting`; its private account key's string form equals the imported
key's. Import succeeds while the manager is locked: the account is protected by its
own passphrase. In case 2 the manager is locked again by a deferred
`Lock` before the transaction commits.

`TestImportAccount`: the unexported `importAccount(dbtx, actBIP0044, xpriv, name)`,
the manager unlocked for every case and locked again afterwards.

| # | Before the call | Name | Result |
| ---: | --- | --- | --- |
| 0 | — | `account 1` | `ImportedAddrAccount + 1` |
| 1 | — | `account 2` | `ImportedAddrAccount + 2` (the same key under a second name is accepted) |
| 2 | — | `account 1` | kind `Exist` |
| 3 | `SyncAccountToAddrIndex(ns, ImportedAddrAccount + 1, 0, ExternalBranch)` | `account 3` | kind `Exist` (an address of this key is now indexed) |
| 4 | — | empty | kind `Invalid` |

After each success the loaded account's private key string equals the imported key's.
The error kind is read with a direct type assertion on the returned error
(`err.(*errors.Error).Kind`), so the outermost error must carry the kind itself.

#### 5.2.8 Manager API passes (`wallet/udb/addressmanager_test.go:147-211, 817-972, 1120-1249`)

`testManagerAPI(tc)` runs five helpers inside one read-write transaction:

1. `testLocking` (requires the manager locked on entry):
   1. `IsLocked()` is true.
   2. `Lock()` on a locked manager → kind `Locked` (`WatchingOnly` on a watching-only
      manager).
   3. `Unlock(privPassphrase)` → nil, `IsLocked()` false (watching-only: kind
      `WatchingOnly`, still locked).
   4. `Unlock(privPassphrase)` again while unlocked → nil, still unlocked.
   5. `Unlock("invalidpassphrase")` → kind `Passphrase` **and the manager is locked
      afterwards** (watching-only: `WatchingOnly`, locked).
2. `testNewAccount`, only when `tc.create`:
   1. `NewAccount("test")` while locked → kind `Locked`.
   2. `Unlock`; `NewAccount("acct-create")` → account 1.
   3. `NewAccount("acct-create")` again → `Exist`; `NewAccount("")` → `Invalid`;
      `NewAccount("imported")` → `Invalid`. The manager is left unlocked.
3. `testLookupAccount`: `LookupAccount("default")` = 0; `LookupAccount("imported")` =
   `ImportedAddrAccount`; `LookupAccount("non existent account")` → `NotExist`;
   `LastAccount()` = 1.
4. `testForEachAccount`: `ForEachAccount` yields exactly `[0, 1, 2147483647]` in that
   order.
5. `testRenameAccount` on account 1: read its name `N`; `RenameAccount(1, N +
   "-renamed")` succeeds; `AccountName(1)` returns the new name; renaming to the same
   new name again → `Exist`; `LookupAccount(N)` → `NotExist`.

Passes, each on the same database:

| Test | Pass | Manager | `create` | Account 1's name after the pass |
| --- | --- | --- | --- | --- |
| `TestManagerWatchingOnly` | 1 | fresh clone | yes | `acct-create-renamed` |
| | 2 | closed, reopened, then `ConvertToWatchingOnly` | no | `acct-create-renamed-renamed` |
| | 3 | opened again (the pass-2 manager is still open) | no | `acct-create-renamed-renamed-renamed` |
| `TestManager` (not run in CI) | 1 | fresh clone | yes | `acct-create-renamed` |
| | 2 | closed and reopened | no | `acct-create-renamed-renamed` |

What the later passes establish: an account created and renamed before a close is
found after reopening with its number, name and position in the iteration order;
`LastAccount` stays 1; on a watching-only manager account lookup, iteration and rename
all work, and rename is a write.

`TestManagerImports` runs `testImportPrivateKey` and `testImportScript` in one
transaction with `create` set: unlock, import, compare the returned managed address
(section 4.2.9: account `ImportedAddrAccount`, not internal, imported), look the
address up again through `Manager.Address` (P2PKH address from the hash160; P2SH
address from the script) while unlocked, `Lock`, look it up again while locked. Its
second pass (`create` false) returns before doing anything.

`TestManagerEncryptDecrypt`, on a locked manager: `Encrypt` and `Decrypt` with key
type `0xff` and an empty input → an error (kind not checked); with `CKTPrivate` →
kind `Locked`; after `Unlock`, `Encrypt(CKTPrivate, empty)` with the private crypto
key swapped for one whose `Encrypt` returns the plain error `failed to encrypt` →
kind `Crypto`; `Lock`; then unlock again and round-trip the 19-byte plaintext under
`CKTPublic` and `CKTPrivate`; `Lock`.

`TestChangePassphrase`, in one transaction, with `ChangePassphrase(ns, old, new,
private)`:

| # | Call | Result |
| ---: | --- | --- |
| 1 | public, `pubPassphrase` → `pubPassphrase2`, with the secret-key constructor replaced by one that fails | kind `Crypto` |
| 2 | public, `bogus` → `pubPassphrase2` | kind `Passphrase` |
| 3 | public, `pubPassphrase` → `pubPassphrase2` | nil; the public master key now derives from `pubPassphrase2` |
| 4 | public, `pubPassphrase2` → `pubPassphrase` | nil |
| 5 | private, `bogus` → `privPassphrase2` | kind `Passphrase` |
| 6 | private, `privPassphrase` → `privPassphrase2` (manager locked) | nil; `Unlock(privPassphrase2)` succeeds |
| 7 | private, `privPassphrase2` → `privPassphrase` (manager unlocked) | nil; the manager is still unlocked |

Step 1 must leave the stored public key parameters untouched, since step 3 succeeds
with the original passphrase.

`TestCoinTypeUpgrade` is the only udb test that does not use the template: it calls
`Initialize` on an empty file with public passphrase `public` and the udb private
passphrase, opens without calling `Upgrade`, and inside one transaction: `Unlock`;
`CoinType` = 11 and `CoinTypePrivKey` equals the key derived from the seed for coin
type 11; `UpgradeToSLIP0044CoinType`; `CoinType` = 1 and the key equals the one for
coin type 1; `AccountExtendedPubKey(0)` equals the neutered SLIP0044 account 0 key;
`SyncAccountToAddrIndex(ns, 0, 1, 0)` and then `ExistsHash160` of the SLIP0044 account
0, branch 0, child 0 address; `NewAccount("account-1")`; the same xpub and address
checks for account 1; a second `UpgradeToSLIP0044CoinType` → kind `Invalid`.

#### 5.2.9 Wallet locking sequence (`wallet/locking_test.go:15-150`)

One simnet wallet, private passphrase `private`. `w.Lock()` is called after each
sub-scenario. `Unlock(ctx, passphrase, timeout)` takes a receive-only channel of
`time.Time`, or nil for no timeout.

| # | Steps | Required |
| ---: | --- | --- |
| 1 | `Locked()`; `Unlock(pass, nil)`; `Locked()`; `Lock()` from another goroutine | true; nil; false; `Lock` returns within 100 ms and `Locked()` is true |
| 2 | `Unlock(pass, nil)`; `Unlock("incorrect", nil)`; `Locked()`; then the same three again | nil; kind `Passphrase`; **true** (both rounds) |
| 3 | `Unlock(pass, nil)`; `Unlock(pass, ch)`; send one value on the unbuffered `ch` | nil; nil; the send is accepted within 100 ms; `Locked()` is false afterwards |
| 4 | `Unlock(pass, ch)` on a locked wallet; send on `ch`; wait 100 ms | nil; `Locked()` is true |
| 5 | `Unlock(pass, ch1)`; `Unlock(pass, ch2)`; send on `ch2`; wait 100 ms; non-blocking send on `ch1` | nil; nil; `Locked()` is true; the send on `ch1` succeeds immediately (a reader is already waiting on it) |

Row 3: a timeout offered while the wallet is unlocked with no timeout is ignored for
locking but its channel is still read once, so a caller's timer send never blocks.
Row 5: a replaced timeout channel keeps a background reader; that reader must not
lock the wallet when it fires (the test cannot observe this because the wallet is
already locked).

#### 5.2.10 Client certificate round trip (`x509_test.go:48-211`)

For each of Ed25519, P-256, P-384, P-521:

1. `generateAuthority(pub, priv)` → an authority with `Cert` (parsed certificate) and
   `PrivateKey`.
2. `marshalPrivateKey(ca.PrivateKey)` → a PEM key block.
3. `createSignedClientCert(pub, ca.PrivateKey, ca.Cert)` → a PEM certificate block.
   The client certificate is issued for the **authority's own public key**: the same
   key pair is the authority key and the client key.
4. `tls.X509KeyPair(certBlock, keyBlock)` must succeed.
5. A TLS server (minimum TLS 1.2, client certificates required and verified against a
   pool holding only `ca.Cert`) must accept a `PUT` carrying that key pair and echo
   the body.

`TestUntrustedClientCert` builds two Ed25519 authorities, trusts the first on the
server and presents a certificate issued by the second. The request must fail. An
error ending in `reset by peer` or containing `connection was forcibly closed` is
retried in a new goroutine; an error ending in `tls: bad certificate` or `tls: unknown
certificate authority` passes; anything else fails; no verdict within 5 s fails.

#### 5.2.11 Known-address passes (`wallet/addresses_test.go:122-217`)

`TestAddresses` runs eight passes, each on a **new** wallet created from the fixed
seed: for `unlock` in (false, true), then for (internal, external), then for
watching-only in (false, true). Within a pass: optionally `Unlock(private, nil)`;
optionally `ConvertToWatchingOnly` on the manager inside a read-write transaction;
then five times: call the address function for account 0, look the result up with
`KnownAddress`, and require `AccountName()` = `default`, `String()` = the expected
address, and through the `BIP0044Address` interface `PubKeyHash()`, the branch from
`Path()` and `PubKey()`. So the same five addresses per branch come out of a locked
wallet, an unlocked wallet, and a wallet converted to watching-only (converted after
unlocking, in the unlocked passes), and address generation needs no private key.

### 5.3 Verification layers

dcroxide defines full coverage as the conjunction of seven layers with CI gates
(`dcroxide:docs/dcroxide-project-brief.md:702-721`). The wallet keeps the list and
replaces the consensus battery with a database battery and adds API captures.

| Layer | Content for dcr-purse | Gate |
| --- | --- | --- |
| 1. Ported vectors | Everything classed P, P+R or F in section 3, plus the dependency suites the sibling specs name (bbolt, hdkeychain, mixing, sntrup4591761, addrmgr, dcrjson, vspd client and types, wsrpc). A ledger (`docs/PARITY.md`) records per upstream test: ported, regenerated, or skipped with a reason. | 100 % of ledger rows resolved |
| 2. Property tests | Round-trip laws for every database row codec, every RPC type, every wire and mixing message; model-based tests of the address buffers and the transaction store against a simple in-memory model. | per crate |
| 3. Fuzzing | Section 5.12. | crash-free nightly |
| 4. Differential (oracle) | Function-level comparison with dcrwallet's own code through `tools/oracle` (section 5.4) and overlay exporters (section 5.5.3). | fails rather than skips in CI |
| 5. Database battery | Section 5.10: scripted state differentials, fixture upgrades, alternating-writer tests, historical versions. | every row of the battery |
| 6. Recorded sessions | Section 5.8: JSON-RPC, gRPC, dcrd-facing, P2P and VSP captures replayed against the port. | byte equality after declared masks |
| 7. Integration and ecosystem acceptance | Section 5.11: simnet matrix with dcrd and dcroxide, vspd, Go mixing peers, dcrctl, Decrediton. | every cell green or recorded as a divergence |
| 8. Robustness and performance | Power-loss simulation on the database writer (dcroxide has a harness, `dcroxide:crates/dcroxide-testutil/src/powerloss.rs:47`), lock-order and shutdown tests, sync and rescan benchmarks against the Go wallet. | regression thresholds |

### 5.4 Oracle rig

#### 5.4.1 Shape

`tools/oracle` is a standalone Go module producing one binary, `dcrwallet-oracle`,
that links dcrwallet's packages and serves the protocol of section 4.3. This is
dcroxide's decision verbatim (`dcroxide:docs/adr/0001-oracle-driven-differential-testing.md:28-41`);
the reasons given there (no cgo coupling, live differential capability, works on all
target operating systems) apply unchanged.

#### 5.4.2 Pin rule

1. `tools/oracle/go.mod` requires `decred.org/dcrwallet/v5` at the pseudo-version of
   `54b85b33`. dcrwallet's `go.mod` has no `replace` directive, so every dependency
   resolves to a published tag and the oracle links what the dcrwallet binary links.
2. The oracle's `go.mod` must not raise any module above the version dcrwallet's
   `go.mod` selects. A unit test compares the oracle's resolved module list
   (`runtime/debug.ReadBuildInfo`, reported by `build.info`) against a table copied
   from dcrwallet's `go.mod`, and against the commit named in the Rust harness
   constant, as dcroxide's `the_dcrd_pin_matches_ci_and_the_oracle` does
   (`dcroxide:docs/adr/0001-oracle-driven-differential-testing.md:94-99`).
3. Versions move only when the parity pin moves.
4. **Version skew with dcroxide.** dcroxide's crates are parity ports of dcrd modules
   at dcrd master `46e9df48`; dcrwallet links older tags of the same modules. Compared
   directory by directory in the module cache (license files ignored):

   | Module | dcrwallet links | dcroxide's oracle links | Difference seen |
   | --- | --- | --- | --- |
   | `txscript/v4` | v4.1.2 | v4.1.3 pseudo-version | `error.go`, `opcode.go`: comment lines only |
   | `gcs/v4` | v4.1.1 | v4.1.2 pseudo-version | `blockcf2/blockcf.go`: comment lines only |
   | `crypto/rand` | v1.0.1 | v1.0.2 pseudo-version | `default.go`, `uniform.go`: comment lines only |
   | `crypto/blake256` | v1.1.0 | v1.1.1 pseudo-version | `hasher224.go`, `hasher256.go`: comments |
   | `blockchain/standalone/v2` | v2.3.0 | v2.3.1 pseudo-version | `subsidy.go`: a guard around a cache insertion |
   | `chaincfg/v3` | v3.3.0 | v3.3.1 pseudo-version | `mainnetparams.go`: one additional mainnet DNS seeder |
   | `dcrec/secp256k1/v4` | v4.4.1 | v4.4.2 pseudo-version | code changes in `curve.go`, `field.go`, `modnscalar.go`, `pubkey.go`, `ecdsa/signature.go`, `schnorr/signature.go`; new files `common.go`, `field64.go`, `field64_generic.go`, `field64_amd64.go`, `field64_amd64.s` (assembly) and a new `internal/` directory; `ellipticadaptor.go`, `error.go`, `nonce.go`, `doc.go` differ in comments only |
   | `blockchain/stake/v5` | v5.0.3 | v5.0.4 pseudo-version | a test file only |
   | `wire`, `chainhash`, `dcrutil/v4`, `base58` | v1.8.0, v1.0.5, v4.0.3, v1.0.6 | the same | none |

   Whether the secp256k1 changes alter any output was not established. The wallet
   oracle therefore re-runs, against the versions dcrwallet links, the differential
   tests of every dcroxide crate the wallet reuses (section 12.2). The seeder list is a
   real difference: SPV mode on mainnet must use the list of `chaincfg/v3` v3.3.0.

#### 5.4.3 Build and spawn policy (Rust side)

1. A test asks `purse-testutil` for an oracle by name. The oracle is one binary per
   area (`tools/oracle/cmd/<name>`), so a prebuilt set is named by directory: if the
   environment variable `PURSE_ORACLE_DIR` is set, `purse-oracle-<name>` in that
   directory is used, and its absence is a failure.
2. Otherwise, if `go` is on the path, `tools/oracle/cmd/<name>` is built into
   `<target-dir>/<profile>/oracle/` under a name that carries a hash of its sources,
   and reused while they are unchanged. A build that fails is a failure, never a skip.
3. Otherwise the test prints a skip notice and returns, unless `PURSE_REQUIRE_ORACLE`
   is set, in which case it panics. CI sets it, so differential coverage cannot
   silently disappear (`dcroxide:crates/dcroxide-testutil/src/lib.rs:231-283`).
4. Under `PURSE_REQUIRE_ORACLE` the oracle's reported Go release (its
   `build.info` command) must equal the project's reference Go, go1.27.4,
   decided 2026-11-18 (section 13 item 1) (`dcroxide:crates/dcroxide-testutil/src/lib.rs:207-221`).
   dcrwallet's repository names a Go
   release only in its CI matrix (1.26 and 1.27) and in `rpc/Dockerfile`, which is the
   protobuf regeneration image (golang 1.24.10), not a release recipe.

#### 5.4.4 Two builds

| Build | Randomness | Clock | Used for |
| --- | --- | --- | --- |
| `dcrwallet-oracle` | real | real | pure functions; comparisons made on masked or decrypted dumps |
| `dcrwallet-oracle-det` | seeded stream (section 5.6.2) | settable (section 5.6.3) | byte-exact database and transaction comparisons |

The deterministic build is the same source built against a dcrwallet **checkout** at
the pin (a `replace decred.org/dcrwallet/v5 => <checkout>` passed with `-modfile`, plus
`-overlay`), so that the clock seam can be applied to dcrwallet's own files without
writing into the checkout. Whether `-overlay` may also replace files inside the module
cache was not checked; using a checkout avoids the question.

### 5.5 What the oracle and the exporters must provide

#### 5.5.1 Reach

A standalone module can import only dcrwallet's public packages: `assets`, `chain`,
`deployments`, `errors`, `kdf`, `lru`, `p2p`, `pgpwordlist`, `rpc/client/dcrd`,
`rpc/client/dcrwallet`, `rpc/jsonrpc/types`, `rpc/walletrpc`, `spv`, `ticketbuyer`,
`validate`, `version`, `wallet`, `wallet/drivers/bdb`, `wallet/txauthor`,
`wallet/txrules`, `wallet/txsizes`, `wallet/udb`, `wallet/walletdb`, `walletseed`.

It cannot import `internal/cfgutil`, `internal/compat`, `internal/loader`,
`internal/loggers`, `internal/netparams`, `internal/prompt`, `internal/rpchelp`,
`internal/rpc/jsonrpc`, `internal/rpc/rpcserver`, `wallet/internal/bdb`,
`wallet/internal/snacl`, or package `main`, and it cannot call unexported functions
(every row codec of `wallet/udb`, the JSON codecs of `rpc/client/dcrd`, the address
buffers of `wallet`). Those are reached by overlay exporters (5.5.3) or through the
running binary (5.8).

#### 5.5.2 Oracle command families

Each family lists the Go entry points it wraps. Field layouts are fixed in the doc
comment of each handler when it is written; sibling specs list the cases to run.

| Command | Wraps | Result |
| --- | --- | --- |
| `build.info` | `runtime.Version`, `debug.ReadBuildInfo` | Go release; module path and version of dcrwallet and every dependency |
| `rand.seed` (det build only) | the seeded stream | resets the stream to a 32-byte key |
| `clock.set` (det build only) | the clock seam | sets the Unix time returned to seamed call sites |
| `seed.encode`, `seed.decode` | `walletseed.EncodeMnemonic`, `walletseed.DecodeUserInput` | words; seed bytes or kind + text |
| `pgp.word`, `pgp.decode` | `pgpwordlist.ByteToMnemonic`, `pgpwordlist.DecodeMnemonics` | word; bytes or error text |
| `hd.derive` | `hdkeychain.NewMaster`, `Child`, `Neuter`, `String`; `udb.CoinTypes`; `stdaddr` | for a seed, network and path: xpriv, xpub, compressed public key, P2PKH address, hash160; both coin types |
| `kdf.derive` | `kdf.Argon2idParams.UnmarshalBinary`, `kdf.DeriveKey` | key bytes; the marshalled parameters |
| `err.text` | `errors.E` over a described list of arguments | `Error()` text, kind name, `Is` verdicts |
| `txsizes.*` | the four exported functions of `wallet/txsizes` | integer |
| `txrules.*` | `IsDustAmount`, `IsDustOutput`, `CheckOutput`, `FeeForSerializeSize`, `PaysHighFees`, `TxPaysHighFees`, `ValidPoolFeeRate`, `StakePoolTicketFee`, `StakeSubScriptType` | value or kind |
| `tx.author` | `txauthor.NewUnsignedTransaction` with a scripted input source and change source; optionally `RandomizeChangePosition` (det build) | transaction hex, change index, estimated signed size, total input, or kind |
| `tx.sign` | `txauthor.AddAllInputScripts` with supplied keys and scripts | signed transaction hex or error |
| `rpc.cmd` | `dcrjson.ParseParams`, `dcrjson.MarshalCmd`, `dcrjson.MethodUsageText` over `rpc/jsonrpc/types` | canonical dump of the parsed command; re-marshalled bytes; usage text; or dcrjson error code and message |
| `rpc.result` | `encoding/json.Marshal` of a named result type populated from a field script | exact JSON bytes; run for every result type, with zero values, populated values, nil and empty slices and maps |
| `grpc.msg` | `proto.Marshal` / `proto.Unmarshal` of a named `walletrpc` message populated from a field script | wire bytes and the decoded field dump |
| `validate.*` | the four functions of `validate` | nil or error text |
| `deploy.active` | `deployments.HardcodedDeployment.Active` and the two `…Active` helpers with a stub querier | bool |
| `db.dump`, `db.check`, `db.pages`, `db.apply` | bbolt v1.3.12 directly (spec 01, section 11.3) | canonical dump (4.5); `Tx.Check` findings and the freelist; per-page type/count/overflow; results of a scripted series of transactions |
| `db.decrypt` | `udb.Open` + `Manager.Unlock` + `Manager.Decrypt` on the values a rule file names | plaintext dump (4.5) |
| `udb.script` | `walletdb.Create/Open`, `udb.Initialize`, `InitializeWatchOnly`, `Upgrade`, `Open`, then any exported `Manager` or `Store` method and the exported VSP, treasury-policy and birthday functions, in the order of a JSON script | per step: return values or kind + text; at the end, the dump |
| `wallet.script` | `wallet.CreateDB`, `wallet.Create`, `wallet.CreateWatchOnly`, `wallet.Open`, a scripted `NetworkBackend`, then exported `Wallet` methods | per step: return values or kind + text; notifications emitted; at the end, the dump |
| `chain.gen` | `chaingen.Generator` driven by a script (`CreateBlockOne`, `NextBlock`, `SetTip`) and `blockcf2.Regular` | block hex, block hash, version 2 filter hex per block |
| `mix.*` | the exported functions of `dcrd/mixing` (spec 12, section 11) | bytes |
| `x509.verify` | Go's `crypto/x509` and `crypto/tls` on a certificate chain produced by the port | verdict; negotiated parameters (dcroxide's oracle has the same, `dcroxide:tools/oracle/main.go:1612`) |

"Derive addresses for a seed" is `hd.derive` plus a `wallet.script` that calls
`NewExternalAddress`/`NewInternalAddress`. "Build a transaction from a fixed UTXO set
with a fixed RNG" is `tx.author` (pure) and a `wallet.script` in the det build (full
path through coin selection). "Dump a wallet.db bucket by bucket" is `db.dump`.
"Serialize every RPC result type" is `rpc.result` and `grpc.msg`.

#### 5.5.3 Overlay exporters

For anything section 5.5.1 puts out of reach, the exporter is a `_test.go` file kept
under `tools/<name>gen/` that runs **inside** the dcrwallet package through
`go test -overlay`, writes a vector file, and never modifies the checkout. This is
dcroxide's mechanism (`dcroxide:tools/mixpoolgen/run.sh:9-14, 49-58`,
`dcroxide:tools/chainopsgen/run.sh:47-56`):

1. `run.sh <dcrwallet checkout> <output file>` resolves both paths.
2. It writes an overlay JSON mapping `<pkg>/purse_export_test.go` to the kept exporter
   and, where a seam is needed, mapping a source file to a `sed`-rewritten copy made
   on the spot from the checkout's own file.
3. It runs `go test -count=1 -tags purse_export -overlay <json> -run '^TestExport…$'
   <pkg>` with the output path in an environment variable.
4. The exporter fails to compile unless the seamed copy is in place (the copy declares
   a constant the exporter names), so it can never write vectors on the real clock or
   real randomness by accident (`dcroxide:tools/mixpoolgen/export_test.go:16-19`).

| Exporter | Runs in package | Emits |
| --- | --- | --- |
| `udbrowgen` | `wallet/udb` | byte vectors for every unexported row codec: address rows, account rows (each DB version's layout), tx records, credits, debits, block records, ticket records, VSP rows, treasury policy rows, birthday state; key encodings |
| `snaclgen` | `wallet/internal/snacl` | marshalled parameters, derived key, ciphertext for fixed password, salt and nonce (seamed randomness) |
| `bdbgen` | `wallet/internal/bdb` | error-kind mapping for every bbolt error; conformance trace |
| `helpgen` | `internal/rpc/jsonrpc` | every method's help text and usage line, read from `helpDescsEnUS()` and `requestUsages`, plus the handler table (method, `noHelp`) |
| `jsonrpcgen` | `internal/rpc/jsonrpc` | handler-level vectors: for a scripted wallet, each handler's returned value marshalled to JSON and each error's code and message |
| `grpcgen` | `internal/rpc/rpcserver` | handler-level vectors for unary RPCs over a scripted wallet: response message bytes, status code and message |
| `dcrdjsongen` | `rpc/client/dcrd` | encode/decode vectors for the unexported `hash`, `hashes`, `hashesContiguous`, `headers`, `transactions` codecs, including error kinds |
| `configgen` | package `main` | `loadConfig` results and error/usage text for a table of argument lists and config files; the `-h` text at 80 columns and at a fixed wider width (dcroxide's `tools/helpgen` shows why the width is part of the vector, `dcroxide:tools/helpgen/main.go:16-24`) |
| `cfgutilgen` | `internal/cfgutil` | flag-value parsing (`AmountFlag`, `CurveFlag`, `ExplicitString`), address normalization |
| `promptgen` | `internal/prompt` | prompt text and accepted answers for the `--create` dialogue |
| `addrbufgen` | `wallet` | the address-buffer scenarios of `TestAccountIndexes` and `TestDiscoveryCursorPos` with the unexported state they read |
| `walletgen` | `wallet` | scenarios needing unexported entry points (vote and revocation construction, ticket purchase with a stub VSP, mixing change handling) under the seeded stream and settable clock |

dcroxide's other tools, and what each is a model for here. All were read at their
header comments; none is reusable as code, since each runs inside a dcrd package.

| dcroxide tool | What it does there | Wallet counterpart |
| --- | --- | --- |
| `tools/mixpoolgen`, `tools/chainopsgen` | overlay exporter with a `sed`-made seam, kept beside its `run.sh` | every exporter of the table above |
| `tools/disapprovalgen`, `tools/trsyboundarygen`, `tools/utxoviewgen` | exporters inside dcrd's `internal/blockchain` that replay a scenario and write every intermediate state as rows; `disapprovalgen` also seams `chaingen` (fixed block-one time, fixed random stream, one solver thread, `dcroxide:tools/disapprovalgen/export_test.go:22-28`) | `udbrowgen` and `walletgen` scenario rows; the `chaingen` seam of section 5.9. `utxoviewgen` takes the non-derived rows of the file it regenerates as its input (`dcroxide:tools/utxoviewgen/export_test.go:12-15`), a pattern that suits the stake-invalidation and coinbase scenarios of sections 5.2.4-5.2.5 |
| `tools/mempoolvectors` | a session exporter using the package's own test harness, with the session clock and every random nonce pinned to the values the frozen file already carries, so a re-run at an unchanged pin is byte-identical (`dcroxide:tools/mempoolvectors/vectors_export_test.go:22-27`). It has no `run.sh`: it is copied into the checkout by hand (`:10-15`) | the rule that a seam's inputs are rows of the vector file (section 4.4); the wallet's exporters all use the `run.sh` overlay form instead of the copy-in form |
| `tools/profilergen` | an exporter inside dcrd's package `main`, one row per call, error texts in hex, with host-dependent outcomes folded into one verdict so that the file does not depend on the machine (`dcroxide:tools/profilergen/export_test.go:11-18`) | `configgen` (package `main` of dcrwallet): listener and path results that depend on the host are normalized the same way |
| `tools/helpgen` | prints the `-h` text at a fixed width | `configgen`'s help rows |
| `tools/fullblockgen` | dumps a generated battery with its generation time as the first row | `chain.gen` |
| `tools/powerloss` | an `LD_PRELOAD` shim that logs every write, truncate and file creation with the bytes it replaced, and a replayer that undoes whatever was not followed by a successful `fsync`/`fdatasync` of its file, giving the on-disk state a power cut would leave; engine-independent (`dcroxide:tools/powerloss/shim.c:3-14`, `dcroxide:tools/powerloss/replay.py:3-25`). Its header lists what it does not model (directory durability, torn or reordered writes) | usable **as is** on both the Go wallet and the port, because it works at the libc boundary: layer 8's power-loss test kills each during a scripted write load, replays the log, and requires `db.check` clean and `udb.Open` successful on the result. Linux only |
| `tools/dcrdstat` | measures payload bytes per bucket of dcrd's database | `db.pages` plus the canonical dump give the same per-bucket accounting for bbolt |
| `tools/dsample` | samples per-thread scheduler states for performance attribution | performance work only; no vector role |
| `tools/pinbump` | maps upstream changes to crates | section 5.13 |

#### 5.5.4 The frozen-battery rule

Adopted from dcroxide from the first vector on
(`dcroxide:docs/adr/0001-oracle-driven-differential-testing.md:169-185`): every frozen
vector file has its exporter in the tree; a file comes due for regeneration when a pin
move changes the package it was dumped in (or the reference Go changes texts it
pins); a due file is regenerated by its tool, never re-expected by hand; before a tool
is run at a new pin it must reproduce the old file byte for byte at the old pin.
dcroxide recorded this rule only after losing the exporters of most of its early
batteries (`:160-167`); the wallet project should not repeat that.

### 5.6 Nondeterminism and how to pin it

#### 5.6.1 Inventory

All randomness in dcrwallet's own packages comes from
`github.com/decred/dcrd/crypto/rand`: no non-test file imports `math/rand` or the
standard `crypto/rand`. One dependency reads the operating system source directly:
`hdkeychain.GenerateSeed` (`dcrd/hdkeychain/v3@v3.1.3/extendedkey.go:15, 630`), which
has four callers: `walletseed.GenerateRandomSeed` (`walletseed/seed.go:29`), the
`--createtemp` seed (`walletsetup.go:278`), the `--create` dialogue when the user
declines to enter a seed (`internal/prompt/prompt.go:271`), and `wallet.Create` when
called with a nil seed (`wallet/wallet.go:5298`). The module replacement of section
5.6.2 does not reach it, so deterministic scenarios always supply the seed. The
Argon2id salt is drawn inside `kdf.NewArgon2idParams` from the reader its caller
passes (`kdf/kdf.go:30-39`); the two callers listed below pass the `crypto/rand`
reader. Call sites outside tests:

| Site | What is random | Persisted or sent? |
| --- | --- | --- |
| `wallet/internal/snacl/snacl.go:38` | 24-byte secretbox nonce per `Encrypt` | yes, in every encrypted database value |
| `wallet/internal/snacl/snacl.go:77` | 32-byte crypto key | yes (encrypted) |
| `wallet/internal/snacl/snacl.go:237` | scrypt salt | yes |
| `wallet/udb/addressmanager.go:173` | XChaCha20-Poly1305 nonce in `seal` | yes |
| `wallet/udb/addressmanager.go:1745, 2196` | Argon2id parameters: the random salt, and (not random but machine-dependent) the thread count `min(runtime.NumCPU(), 256)` (`kdf/kdf.go:31-35`) | yes |
| `wallet/udb/addressmanager.go:512, 1080, 1736, 2783` | keys of in-memory BLAKE2b passphrase hashers | no |
| `wallet/udb/txmined.go:2362, 2386, 2410` | random start point and direction in the unspent bucket (`randomUTXO`) | affects which outputs are selected |
| `wallet/udb/txmined.go:3169` | shuffle of remaining keys during input selection | affects selection order |
| `wallet/createtx.go:1957` | shuffle of the unspent list | affects selection |
| `wallet/createtx.go:1492` | per-ticket publish delay, 20 s + [0, 40 s) | timing |
| `wallet/txauthor/author.go:172` | change output position | yes, in the transaction |
| `wallet/chainntfns.go:779` | whether to disapprove a block (non-mainnet, `disapprovepercent`) | yes, in the vote |
| `wallet/feepayment.go:210` | VSP fee payment scheduling jitter | timing |
| `internal/rpc/jsonrpc/methods.go:3687` | 24 random bytes in an `OP_RETURN` payload | yes, in the transaction |
| `internal/rpc/jsonrpc/methods.go:4363-4364` | shuffle of inputs and outputs (`spendoutputs`) | yes |
| `internal/rpc/rpcserver/server.go:3189` | the seed returned by the gRPC `SeedService.GenerateRandomSeed` (a direct `rand.Read`, so the seeded stream does reach it) | returned to the client |
| `internal/prompt/prompt.go:271`, `wallet/wallet.go:5298`, `walletseed/seed.go:29` | a new wallet seed from `hdkeychain.GenerateSeed` (operating system source, not the seeded stream) | yes |
| `rpcserver.go:120, 156, 205, 218, 292` | certificate serial numbers and key pairs | written to `rpc.cert`/`rpc.key` |
| `p2p/peering.go:236, 946` | version and ping nonces | sent |
| `p2p/peering.go:505`, `spv/sync.go:623, 695` | inventory trickle and request jitter | timing |
| `walletsetup.go:278` | seed of a `--createtemp` wallet (`hdkeychain.GenerateSeed`) | yes |
| the mixing client | spec 12 | sent |
| `dcrd/blockchain/v5@v5.1.1/chaingen/generator.go:448, 564, 674, 837, 2068, 3169` (test-only dependency; used by the chain fixtures of section 5.9) | random `OP_RETURN` data, the proof-of-work nonce found first among `runtime.NumCPU()` goroutines, block one's wall-clock timestamp | yes, in every generated block |

Wall-clock values that are persisted or returned:

| Site | Field |
| --- | --- |
| `wallet/udb/addressdb.go:1360, 1375, 1390` | `addTime` of every address row |
| `wallet/udb/addressdb.go:1683` | address manager creation date |
| `wallet/udb/txdb.go:2181` | transaction store creation date |
| callers of `udb.NewTxRecordFromMsgTx(tx, time.Now())` (`wallet/chainntfns.go:71, 346, 874`, `wallet/createtx.go:253, 309, 511, 1461, 1576, 1661`, `wallet/rescan.go:196`) and of `udb.NewTxRecord(bytes, time.Now())` (`wallet/wallet.go:5134`) | received time of each transaction record (returned as `time`/`timereceived` by the list RPCs) |
| `wallet/feepayment.go:253, 375, 521` | `Timestamp` field of the VSP request bodies (`time.Now().Unix()`); sent and signed, not persisted |
| `rpcserver.go` (4 uses) | certificate validity window |

Other sources: Go map iteration order wherever a handler ranges over a map without
sorting (each sibling spec lists its own cases); goroutine scheduling (order of
notifications from different sources, order of concurrent database transactions,
order in which goroutines draw from the one global random stream).

#### 5.6.2 The randomness seam

**Go side.** `crypto/rand` v1.0.1 exposes no way to seed its global generator: it is a
ChaCha20 stream keyed from the operating system, re-keyed after 4 MiB or 20 s
(`dcrd/crypto/rand@v1.0.1/prng.go:20-23, 67-110`; on OpenBSD with Go 1.24 or later
the build tag selects `prng_arc4random.go` instead, whose `Read` is the standard
`crypto/rand.Read` and overwrites the buffer). The det build replaces the module
(`replace github.com/decred/dcrd/crypto/rand => ./detrand`) with a package of the same
exported API in which:

1. the stream is ChaCha20 with a 32-byte key set by `rand.seed`, a zero 12-byte nonce,
   and no re-keying;
2. `Read(s)` XORs the key stream **into** `s`, as upstream does
   (`prng.go:85-110`: `p.cipher.XORKeyStream(s, s)`), so a caller passing a non-zero
   buffer gets buffer XOR stream;
3. the derived-value functions are copied unchanged from `uniform.go`.

**Rust side.** One `purse-rand` crate is the only source of randomness in the
workspace. In production it is seeded from the operating system. Under a test-only
constructor it takes a 32-byte key and produces the same stream as `detrand`. For the
two sides to agree on a seeded stream, the derived-value functions must consume bytes
exactly as Go does:

| Function | Bytes consumed | Rule (`dcrd/crypto/rand@v1.0.1/uniform.go`) |
| --- | --- | --- |
| `Uint32()` | 4 | little endian (`:47-51`) |
| `Uint64()` | 8 | little endian (`:54-58`) |
| `Uint32N(n)` | 8 per attempt | power of two: low bits of one `Uint64`; otherwise the high word of the 96-bit product of one `Uint64` and `n`, redrawing while the low 64 bits of the product are below `2⁶⁴ mod n` — implemented with 32-bit halves (`:61-97`) |
| `Uint64N(n)` | 8 per attempt | power of two: mask; otherwise `hi, lo = x × n` (128-bit); if `lo < n`, redraw while `lo < (−n mod n)`; return `hi` (`:102-148`). On a 32-bit platform a value fitting 32 bits goes through `Uint32N` instead (`:103-105`). |
| `Int32N`, `Int64N`, `IntN`, `UintN`, `Duration` | `Int32N` as `Uint32N`; the others as `Uint64N` | casts; `Int32N`, `Int64N`, `IntN` and `Duration` panic on `n ≤ 0`, `UintN` does not (`:159-209`) |
| `Shuffle(n, swap)` | per step as `Uint64N` | for `i = n−1` down to 1: `j = Uint64N(i+1)`; `swap(i, j)` (`:214-229`) |
| `ShuffleSlice` | as `Shuffle` | swaps slice elements (`default.go:144-148`) |
| `BigInt(max)` | as Go's `crypto/rand.Int` | delegated to the standard library with the stream as reader (`uniform.go:233-237`) |

The seeded stream gives byte equality only on paths where the order of draws is
fixed: single-goroutine scripted calls (`udb.script`, `wallet.script` without a
running syncer, `tx.author`). It does not make a running daemon deterministic,
because several goroutines draw from the one stream. Whether the port commits to
drawing in Go's order on those paths is a decision (section 13).

#### 5.6.3 The clock seam

Go side: the overlay rewrites `time.Now()` to a package-level `purseNow()` in the files
of the scripted paths (`wallet/udb/addressdb.go`, `wallet/udb/txdb.go`, and the wallet
files a scenario touches), as dcroxide's mixpool exporter does with `sed`
(`dcroxide:tools/mixpoolgen/run.sh:42-47`). Rust side: every production call site takes
the time from one clock object that tests can set. Persisted times are whole seconds
(`uint64(time.Now().Unix())`), so the seam value is a Unix second.

#### 5.6.4 Mask rules for database dumps

When the plain oracle is used, these fields are masked before a dump comparison. The
list is by origin; the exact key and offset of each field is in specs 02 and 03.

| Field | Origin |
| --- | --- |
| Address row `addTime` (8 bytes) | `wallet/udb/addressdb.go:1360-1390` |
| Manager creation date (8 bytes) | `wallet/udb/addressdb.go:1683` |
| Transaction store creation date (8 bytes) | `wallet/udb/txdb.go:2181` |
| Master key parameters: the scrypt salt | `wallet/internal/snacl/snacl.go:237` |
| Every snacl ciphertext: 24-byte nonce and the sealed bytes | `wallet/internal/snacl/snacl.go:35-45` |
| Every `seal` output: nonce and sealed bytes | `wallet/udb/addressmanager.go:170-180` |
| Argon2id parameter blobs: the salt; the thread-count byte when the two sides run on machines with different CPU counts (`kdf/kdf.go:31-35`) | `wallet/udb/addressmanager.go:1745, 2196` |
| Transaction record received time, when the scenario does not fix it | callers of `NewTxRecordFromMsgTx` |

Masking hides the plaintext of encrypted values. The plaintext dump (`db.decrypt`)
closes that gap for databases whose passphrases the test knows.

#### 5.6.5 Map order and scheduling

- Where Go's output order is map order (random per run), no golden can pin it and no
  client can depend on it. The capture tool runs each such request several times; a
  response whose byte order varies across Go runs is recorded as an unordered
  collection, and the port's output is compared as a set. A response that never
  varies is compared in order.
- Where order comes from goroutine scheduling (notification interleaving), captures
  record the partial order that held in every run, and the scenario states the
  constraint (for example: a transaction notification for a block's transactions
  precedes the next block's attach notification).

### 5.7 Database differential procedure

For one scenario (a `udb.script` or `wallet.script`):

1. Set the seed and clock on both sides (det build), or neither (plain build).
2. Go: run the script on a new file; collect per-step results and the final dump.
3. Rust: run the same script through the port's equivalent entry points on a new
   file; collect the same.
4. Compare per-step results: return values, and for errors the kind name and the
   full `Error()` text.
5. Compare dumps: exact (det build) or masked plus plaintext (plain build).
6. Hand the Rust-written file to the Go oracle: `db.check` must report nothing, and
   `db.dump` of it must equal the Rust dump.
7. Hand the Go-written file to the port: it must open, and its dump must equal Go's.

Steps 6 and 7 make every state differential also a file-format test in both
directions.

### 5.8 Recorded sessions

#### 5.8.1 Why capture at all

No upstream test calls a JSON-RPC handler or a gRPC method. Field order, omitted
fields, number formatting, error codes and messages, and notification order exist
only in the running binary. Handler-level exporters (5.5.3) give exactness per call;
captures give the transport framing, authentication and streaming behaviour around
them.

#### 5.8.2 Deterministic read-only captures (frozen database)

1. Build a wallet database with the Go binary on simnet through a scripted scenario
   (section 5.11.2), stop it, and commit the `wallet.db` as a fixture together with
   the chain fixture it was synced against.
2. Start Go dcrwallet with `--offline` (the wallet then uses
   `wallet.OfflineNetworkBackend`, `dcrwallet.go:435-436`), fixed `--username`,
   `--password`, `--pass`, on the fixture copy.
3. Drive every read-only JSON-RPC method and every read-only gRPC method with a fixed
   request list covering each optional parameter's presence and absence and each
   documented error; record requests and responses (section 4.6).
4. Replay: start the port the same way on another copy of the fixture, send the same
   requests, compare.

Nothing in this procedure touches the network, the clock (beyond fields the masks
name) or randomness, so the comparison is byte-exact and the capture never needs to
be re-recorded unless the pin or the fixture changes.

#### 5.8.3 Stateful captures

For methods that change state (address generation, account creation, policy setting,
locking, imports, signing with given inputs) the script runs against a fresh copy of
the fixture per test, with responses compared exactly and the resulting database
compared by section 5.7's dump rules.

For methods whose result depends on randomness or on a live backend (sending,
ticket purchase, mixing, rescans against a node) no golden is byte-comparable. They
are covered three ways: handler-level exporters under the det build; structural
comparison of the live response (same fields, same types, plausible values, the
transaction decodes and has the expected shape and fee); and the acceptance matrix.

#### 5.8.4 Masks for API captures

A mask is declared per method and field, never globally. The initial list is the
fields fed by the sources of section 5.6.1: generated seeds, transaction hashes of
randomly constructed transactions, certificate material, peer nonces, bound listener
addresses, elapsed-time and timestamp fields that read the clock at call time. Each
mask is added only after the capture tool shows the field varying between two Go
runs; the mask table lives beside the captures and is reviewed like code.

#### 5.8.5 Backend-facing captures

- **RPC mode.** A recording websocket proxy between Go dcrwallet and dcrd stores the
  conversation for a scripted scenario. The replay harness plays dcrd's side from the
  recording to the port and checks the port's requests against the recorded ones
  (method, parameter bytes; order where spec 15 says order is fixed).
- **SPV mode.** A scripted peer (built on dcroxide's wire and peer crates) serves a
  chain fixture; the messages Go dcrwallet sends are recorded and the port's are
  compared (spec 14).
- **VSP.** A stub HTTP server returns recorded vspd responses; the port's request
  path, body and `VSP-Client-Signature` header are compared with Go's for the same
  ticket and keys.

### 5.9 Chain fixtures

`TestReorg` and `TestSetBirthStateAndScan` build blocks with `chaingen` at run time.
`chaingen` is not reproducible: block one takes its timestamp from `time.Now()`
(`dcrd/blockchain/v5@v5.1.1/chaingen/generator.go:3167-3170`) and coinbase scripts
carry a random extra nonce (`generator.go:564`). So:

1. `chain.gen` runs a script once and writes a fixture: `now <unix>` first, then per
   block the name, hash, serialized block and version 2 filter (computed with the
   same previous-script rule as `chaingenPrevScripter`, `wallet/reorg_test.go:45-58`).
2. Fixtures are committed. Both the Go oracle (`wallet.script`) and the port consume
   the same fixture, so their databases can be compared.
3. Named fixtures to create first: `reorg` (block one, 2a, 3a, 2b, 3b, 4b as in
   `TestReorg`), `linear9` (nine blocks as in `TestSetBirthStateAndScan`), and one
   simnet chain past stake validation height 144 with tickets and votes
   (`dcrd/chaincfg/v3@v3.3.0/simnetparams.go:572-573`) for stake scenarios.

`chaingen` has three more sources of run-to-run variation than the two named above,
all in `dcrd/blockchain/v5@v5.1.1/chaingen/generator.go`:

| Line | Source | Reached by the wallet fixtures? |
| --- | --- | --- |
| `:448` | `UniqueOpReturnScript`: a random 64-bit value | only if a script asks for it |
| `:564` | coinbase extra nonce: a random 64-bit value at bytes 28-36 of the 36-byte `OP_RETURN` data, after the little-endian block height at bytes 0-4 | every block |
| `:674` | treasurybase `OP_RETURN`: a random 64-bit value after the height | blocks generated with the treasury agenda active |
| `:837` | treasury spend payload: 24 random bytes | only treasury-spend scripts |
| `:2068-2085` | proof-of-work search: the nonce range is split over `runtime.NumCPU()` goroutines and the first solution reported is taken | every block: the header nonce, and so the block hash, depends on scheduling and on the core count |
| `:3169` | block one's timestamp is the wall clock | every chain |

All four random sites draw from `dcrd/crypto/rand`, so the det build's stream
replacement (section 5.6.2) covers them, but the wall clock and the solver race are
not covered by it. For `chain.gen` to satisfy the frozen-battery rule (reproduce its
old file byte for byte at the old pin, section 5.5.4) it therefore runs against an
overlaid copy of `generator.go`, derived from the module's own file on every run, in
which `time.Now()` is replaced by a fixed block-one time and `runtime.NumCPU()` by 1,
exactly as dcroxide's exporters do (`dcroxide:tools/disapprovalgen/run.sh:45-57`:
`sed` on `time\.Now()`, `rand\.Uint64()`, `rand\.Read(` and `runtime\.NumCPU()`, with
a check that each pattern occurs the expected number of times, which holds at
v5.1.1: one `time.Now()` and one `runtime.NumCPU()`). Without that overlay the
fixture file is a one-off recording and has to be treated as such: frozen, never
regenerated, and exempt from the reproduce-first rule.

For daemon-level tests the chain fixture is instead a dcrd simnet data directory
produced by the acceptance harness and archived.

### 5.10 Database cross-compatibility battery

#### 5.10.1 Alternating writers

For each scenario S of a list covering every bucket (account creation, address
generation on both branches, imports of each kind, mined and unmined transactions,
tickets, votes, revocations, rollbacks, VSP records, treasury policies, birthday,
passphrase changes, watching-only conversion, coin-type upgrade):

1. Go creates the database and runs the first half of S.
2. Rust opens the same file, verifies it can read everything Go wrote (queries
   equal), and runs the second half.
3. Go reopens the file: `db.check` clean; `udb.Open` succeeds; queries equal the
   expected final state; Go then performs one more write and closes.
4. Rust reopens once more and verifies.

The same is run with the roles reversed (Rust creates). At the binary level the same
ping-pong is run with the two daemons on one `--appdata` directory, never
concurrently: Go creates and syncs on simnet; Rust opens, syncs further, sends and
buys a ticket; Go reopens, syncs, votes.

Go opens the file with `bolt.Open(path, 0600, nil)`
(`wallet/internal/bdb/db.go:321`), that is, default options: any file the port writes
must be valid for bbolt v1.3.12 with its default freelist handling (spec 01).

#### 5.10.2 Upstream fixtures

Port `TestUpgrades` exactly (section 5.2.2) on the seven fixtures, and add: for each
fixture, the dump after the port's upgrade must equal the dump after Go's upgrade
(`udb.script`: open, `Upgrade`, dump), under the masks. This turns the seven fixtures
from seven spot checks into seven full-state comparisons of all 25 upgrade steps.
`v24.db.gz`, unused upstream, joins the list.

#### 5.10.3 Historical versions

The version constants run from 1 to 26 (`wallet/udb/upgrades.go:31-213`). Fixtures
exist for 1, 2, 3, 4, 5, 7, 11 and 24. Read from the tags of the local dcrwallet
repository, the value of `DBVersion` at each application release is:

| Application release tags | `DBVersion` |
| --- | ---: |
| v1.0.0, v1.0.1 | 3 |
| v1.0.3, v1.0.4 | 4 |
| v1.0.5 | 5 |
| v1.0.7, v1.0.8, v1.1.0 | 6 |
| v1.1.2 | 7 |
| v1.2.0 | 9 |
| v1.2.1, v1.2.2, release-v1.3.0, release-v1.4.0, release-v1.4.1 | 11 |
| release-v1.5.0, release-v1.5.1 | 13 |
| release-v1.6.0 … release-v1.6.3 | 22 |
| release-v1.7.0 … release-v1.8.1 | 25 |
| release-v2.0.0 … release-v2.1.7 | 26 |

Tags v0.0.3 through v0.8.2 have no `wallet/udb/upgrades.go`; those releases predate
the unified database (spec 05 covers `NeedsMigration`/`Migrate`). So the versions a
user's database can be at when it last ran a tagged release are 3, 4, 5, 6, 7, 9, 11,
13, 22, 25 and 26. Version 21 was the latest version at the release-candidate tags
`release-v1.6.0-rc1` and `v1.6.0-rc1`, and version 12 at three module tags
(`rpc/jsonrpc/types/v1.1.0`-`v1.3.0`, `spv/v2.1.2`); versions 1, 2, 8, 10, 14-20, 23
and 24 were the latest version only on untagged commits. (The repository also carries tags `v1.5.0`
through `v5.1.2` without the `release-` prefix; from 1.7.0 on they report different
values from the same-numbered `release-` tags and were not used for this table.)

Procedure:

1. For each released version without a fixture (6, 9, 13, 22, 25, 26) build that
   release's dcrwallet, create a wallet from a fixed seed on simnet or testnet3, run a
   short scripted history against a node of the same era or by direct `udb` calls in a
   generator compiled at that tag (the method the upstream generators use), and
   archive the database. This requires old Go toolchains and old dependency trees; it
   is a one-time job whose outputs are committed.
2. For intermediate versions, either accept the transitive coverage that the released
   fixtures give (every database upgraded from version N passes through all later
   steps), or build fixtures at the commit that introduced each version. Decision in
   section 13.
3. For every fixture: Go upgrades a copy, the port upgrades a copy, dumps must be
   equal, and the port must then pass the alternating-writer test starting from its
   upgraded copy.
4. A database at a version above 26 must be refused by the port with the error Go
   gives (spec 05).

### 5.11 Simnet acceptance

#### 5.11.1 Harness

- `purse-testutil` spawns processes itself, as dcroxide's `DcrdNode` does
  (`dcroxide:crates/dcroxide-testutil/src/lib.rs:594-640`): dcrd or dcroxide on simnet
  with a temporary data directory, then wallets. The dcrd binary must be built from
  dcroxide's parity commit; the harness checks the reported revision
  (`dcroxide:crates/dcroxide-testutil/src/lib.rs:570-581`). The Go dcrwallet binary must
  report the wallet parity commit in the same way.
- Wallets are created without a terminal by piping the creation answers to
  `--create`, the method of dcrd's simnet script (`y`, `n`, `y`, then the seed hex;
  dcrd repository `contrib/devtools/dcr_tmux_simnet_setup.sh:34-38, 134`), or through
  the gRPC `WalletLoaderService.CreateWallet`, which takes the seed and passphrases as
  request fields (`rpc/api.proto:728-736`) when the daemon is started with
  `--noinitialload`.
- Listener addresses are learned from the pipe IPC (`--pipetx` with
  `--rpclistenerevents`, `config.go:165-167`), not by parsing logs.
- Blocks are mined through the node's `generate` RPC. Simnet stake parameters the
  scenarios rely on: coinbase maturity 16, ticket maturity 16, ticket expiry 384,
  ticket pool size 64, 5 tickets per block, stake enabled at height 32, stake
  validation at height 144, minimum ticket price 20,000 atoms
  (`dcrd/chaincfg/v3@v3.3.0/simnetparams.go:559-573`).
- The Go harness `dcrdtest` exists for dcrd: `New(t, params, handlers, extraArgs)`
  starts a node from a binary set with `SetPathToDCRD` or built with `go build`
  (`dcrtest/dcrdtest@v1.0.1-0.20240404170936-a2529e936df1/rpc_harness.go:76, 101`,
  `builder.go:17-36`), with an in-memory wallet (`memwallet.go:84`), a
  `VotingWallet` that buys tickets and votes with one hard-coded key so that a chain
  can be driven past stake validation without any wallet under test
  (`votingwallet.go:94, 144, 317`), and `AdjustedSimnetMiner` (`simnet_miner.go:113`).
  Its `go.mod` requires dcrd v1.8.0 by default (`go.mod:8`), so the binary must be set
  explicitly. Use it only where a Go driver is needed anyway; the module cache holds
  no dcrwallet harness package from dcrtest (UNVERIFIED whether upstream publishes
  one; settle by listing the dcrtest repository).

#### 5.11.2 Scenario catalogue

| Id | Scenario | Passes when |
| --- | --- | --- |
| A1 | Create from seed; restore from seed with discovery; watching-only from xpub | same addresses, balances and discovered accounts as the Go wallet on the same chain |
| A2 | Receive; `sendtoaddress`, `sendmany`, `sendfrom`, `sweepaccount`, `consolidate`, raw transaction create/fund/sign/send | transactions accepted and mined; balances agree with a Go wallet restored from the same seed afterwards |
| A3 | Reorganization of 1-3 blocks containing wallet transactions, including one that double-spends | balances and transaction states agree with the Go wallet restored from the same seed |
| A4 | Solo staking: manual `purchaseticket`, then the ticket buyer (`--enableticketbuyer`), voting (`--enablevoting`), reward maturity | votes appear in blocks; the chain advances past height 144 with the port as the only voter |
| A5 | Missed and expired tickets; revocation as spec 11 defines it | revocations mined; balances correct |
| A6 | Vote choices, per-ticket choices, treasury spend policy, treasury key policy, `disapprovepercent` | votes carry the configured bits |
| A7 | VSP client against a real vspd on simnet: purchase with fee payment, status, vote-choice update, fee-payment recovery after restart | vspd reports the ticket paid and confirmed; a voting wallet behind vspd votes it |
| A8 | The port as vspd's voting wallet | vspd's calls succeed: `version`, `getcurrentnet`, `walletinfo`, `importprivkey`, `addtransaction`, `setvotechoice`, `getblockcount`, `ticketinfo`, `rescanwallet`, `settreasurypolicy`, `settspendpolicy` (read from `vspd` at `a381bece`, `rpc/dcrwallet.go:140-246`), and its version check passes (JSON-RPC API 11.x; dcrwallet at the pin reports `11.0.0`, `internal/rpc/jsonrpc/methods.go:56-59`) |
| A9 | Mixing: `--mixing` account mixer with Go dcrwallet peers; mixed ticket purchase; `mixoutput`, `mixaccount` | a mix completes containing outputs of the port and of Go wallets; at least 4 participants are required (`dcrd/mixing@v0.7.5/client.go:9`), so one port instance plus three Go instances is the minimum, and the reverse (three port instances plus one Go) is run too |
| A10 | Mixing disruption: a peer that stops responding mid-run | the port blames and re-runs as the Go peers do (function-level coverage is in spec 12; here only that a mixed session with Go peers recovers) |
| A11 | Treasury: `sendtotreasury`, `sendfromtreasury` with simnet treasury keys | transactions accepted |
| A12 | Multisig: `createmultisig`, `sendtomultisig`, `redeemmultisigout(s)`, `importscript` | spends mined |
| A13 | Key and account management: `importprivkey`, `importpubkey`, `importxpub`, voting account import, account passphrases, `walletpassphrasechange`, rescans, birthday | results agree with Go |
| A14 | Lifecycle: start with and without an existing wallet, `--noinitialload` and the loader service, certificate generation, `--issueclientcert` and client-certificate authentication, pipe IPC, signals, shutdown during sync and during a rescan | same files created, same exit codes, same IPC messages |
| A15 | `dcrctl --wallet` sweep: every wallet command dcrctl lists | each command returns what it returns against Go dcrwallet on a twin wallet (read-only commands byte-equal) |
| A16 | Decrediton with the port's binary substituted | wallet creation, sync in both modes, send, staking, mixing and settings screens work |
| A17 | Database ping-pong at the binary level (5.10.1) | both daemons continue from each other's file |
| A18 | Offline signing workflow of `docs/offline_wallets.md` | a watching-only port and an offline Go wallet (and the reverse) complete a spend |

#### 5.11.3 Matrix

Wallet under test × backend × sync mode. The Go wallet column is the control: a
scenario that fails with Go dcrwallet is a harness bug, not a finding.

| Backend | Sync | Go dcrwallet (control) | dcr-purse |
| --- | --- | --- | --- |
| dcrd at dcroxide's pin | RPC (`--rpcconnect`) | A1-A15, A17, A18 | A1-A18 |
| dcrd | SPV (`--spv --spvconnect`) | A1-A6, A9, A13 | A1-A7, A9, A10, A13, A16 |
| dcroxide | RPC | A1-A6, A9 | A1-A9, A11-A15 |
| dcroxide | SPV | A1-A4, A9 | A1-A7, A9, A13 |
| mixed: wallet's peers are one dcrd and one dcroxide | SPV | A3 | A3, A9 |

Scenarios that need an RPC backend by construction (A8, A11 as far as the node's
treasury RPCs are involved) are listed only in RPC rows. Decrediton (A16) is run in
both of its modes against dcrd; which flags and gRPC calls Decrediton depends on was
not read (UNVERIFIED; settle by reading Decrediton's wallet launcher and client code,
or by recording a session with the Go wallet behind a logging gRPC proxy).

### 5.12 Fuzz targets

Crash-freedom targets (malformed input must produce an error, never a panic or an
out-of-bounds read), one per decoder that reads bytes the wallet does not control or
that may be corrupt:

| Target | Input |
| --- | --- |
| `bolt_open` | a whole database file: meta pages, freelist, branch and leaf pages |
| `bolt_page` | one page with an arbitrary header |
| `udb_row_decode` | first byte selects a row decoder of `waddrmgr` or `wtxmgr` (address row, account row per version, tx record, credit, debit, block record, ticket record, VSP rows, policy rows, birthday), rest is the value |
| `snacl_unmarshal`, `kdf_params_unmarshal` | marshalled key parameters |
| `seed_decode` | text for `DecodeUserInput` (hex and mnemonic) |
| `config_file`, `config_args` | a `dcrwallet.conf`; an argument vector |
| `jsonrpc_request`, `jsonrpc_params` | an HTTP body; `(method, params)` for every registered command |
| `ws_frame` | websocket frames from a client |
| `grpc_message` | `(method, message bytes)` |
| `dcrd_json` | results and notifications arriving from dcrd |
| `p2p_message` | framed wire messages from a peer, including mixing messages |
| `mix_message` | each mixing message type's payload |
| `vsp_response` | HTTP response head and JSON body, with and without a valid server signature |
| `ipc_frame` | pipe messages on `--piperx` |
| `address_decode`, `wif_decode`, `xkey_decode` | strings supplied to RPCs |
| `script_classify` | output scripts fed to the wallet's script classification and to `PossibleCoinJoin` |

Differential targets (random input to both implementations through the oracle; any
difference is a finding), following dcroxide's pattern of a fuzz target that drives
the oracle (`dcroxide:fuzz/fuzz_targets/dcrec_diff_ecdsa.rs:1-11`):

| Target | Compared |
| --- | --- |
| `diff_seed` | `seed.decode` / `seed.encode` |
| `diff_hd` | `hd.derive` over random seeds and paths (including hardened and invalid children) |
| `diff_txsizes`, `diff_txrules` | size and fee functions |
| `diff_txauthor` | `tx.author` over random UTXO sets, outputs and fee rates under a random seed |
| `diff_rpc_cmd` | `rpc.cmd` over random JSON parameter arrays for each method: parsed result or error code and message |
| `diff_rpc_result`, `diff_grpc_msg` | marshalled bytes over random field values |
| `diff_db_ops` | random scripts of bucket operations through `db.apply`, comparing results, dumps and `db.check` (the model-checking tests of bbolt, re-expressed) |
| `diff_udb_ops` | random valid sequences of store operations through `udb.script` |
| `diff_rand` | the derived-value functions of section 5.6.2 over random seeds and bounds |
| `diff_mix` | the mixing primitives (spec 12) |

### 5.13 Moving the pin

1. Run the pin-bump report (the dcroxide tool lists upstream files changed between
   two commits and maps them to crates through the parity ledger,
   `dcroxide:tools/pinbump/main.go:1-31`); upstream `_test.go` changes get their own
   section, because a new or corrected upstream expectation is what a copied vector
   goes stale on.
2. Move `tools/oracle/go.mod`, the harness constant and CI's commit together; the
   pin-agreement test fails otherwise.
3. Re-run every exporter whose package changed; each must first reproduce its file at
   the old pin. Review the diffs.
4. Re-record captures whose handlers changed.
5. Re-run the database battery; a new `DBVersion` adds an upgrade step, a fixture at
   the previous version, and a row in section 5.10.3.

---

## 6. Constants, limits and defaults

| Name | Value | Source | What depends on it |
| --- | --- | --- | --- |
| Parity pin | dcrwallet `54b85b33`, version 2.2.0-pre | `version/version.go:21-33` | oracle `go.mod`, harness constant, CI |
| Go directive | `go 1.26.0` | `go.mod:3` | lowest Go that builds the oracle |
| Upstream CI Go releases | 1.26, 1.27 | `.github/workflows/go.yml:12` | candidates for the reference Go |
| Upstream test command | `go test -short -vet=all ./...` | `run_tests.sh:9` | which upstream tests are known to pass |
| Test files / lines | 43 / 10,932 | section 1 | parity ledger size |
| Dead test lines | 1,163 (3 files) | section 3.3 | not ported |
| Non-test Go lines | 83,418 (18,744 generated in `rpc/walletrpc`) | `wc -l` at the pin | coverage ratio |
| JSON-RPC handlers | 111 entries, 8 of them `noHelp` stubs | `internal/rpc/jsonrpc/methods.go:85-200` | capture request list |
| Help methods | 103 | `internal/rpchelp/methods.go` | `helpgen` rows |
| Registered command types | 104 | `rpc/jsonrpc/types/methods.go:1189-1312` | `rpc.cmd` coverage |
| Methods touched by upstream type tests | 43 | section 3.2 | the other 61 need oracle vectors |
| gRPC services / RPCs / streaming RPCs / messages | 11 / 88 / 12 / 195 top-level (222 counting nested message types) | `rpc/api.proto` | `grpc.msg` and capture coverage |
| JSON-RPC API version | 11.0.0 | `internal/rpc/jsonrpc/methods.go:56-59` | vspd's and dcrctl's version checks |
| gRPC API version | 10.0.0 | `internal/rpc/rpcserver/server.go:64-67` | any client that checks `VersionService` |
| Required dcrd JSON-RPC API | 8.3.0, same major, minor/patch at least | `chain/sync.go:33`, `chain/semver.go:15-26` | backend rows of the matrix |
| Current database version | 26 | `wallet/udb/upgrades.go:208-213` | upgrade battery |
| Address manager version constant | 6 | `wallet/udb/addressdb.go:21` | `TstLatestMgrVersion` |
| Fixture versions | 1, 2, 3, 4, 5, 7, 11, 24 | `wallet/udb/testdata` | section 5.10 |
| Fixture passphrases | public `public`, private `private` | `wallet/udb/testdata/v1.db.go:28-29` | fixture replay |
| udb test passphrases | four strings, 29 bytes (`pubPassphrase`) and 30 bytes (the other three) | `wallet/udb/common_test.go:30-33` | ported udb tests |
| Wallet test passphrases | public `public`, private `private` | `wallet/setup_test.go:18-21`, `wallet/wallet.go:66` | ported wallet tests |
| Simulation wallet passphrase | `password` | `wallet/wallet.go:72` | `--createtemp` scenarios |
| Test seed | `b46bc650…641567` (32 bytes) | `wallet/addresses_test.go:46-51` | address vectors |
| Test wallet gap limit / relay fee | 20 / 100,000 atoms per kB | `wallet/setup_test.go:22-23` | address-buffer scenarios |
| snacl defaults | N = 16384, r = 8, p = 1 | `wallet/internal/snacl/snacl.go:25-27` | `snaclgen` |
| Seed length bounds | 16 to 64 bytes, recommended 32 | `dcrd/hdkeychain/v3@v3.1.3/extendedkey.go:30-44` | `seed.decode` cases |
| Mixing minimum peers | 4 | `dcrd/mixing@v0.7.5/client.go:9` | A9 needs three Go peers |
| Simnet stake parameters | maturity 16/16, expiry 384, pool 64, 5 per block, SVH 144 | `dcrd/chaincfg/v3@v3.3.0/simnetparams.go:559-573` | acceptance scripts |
| Mainnet maturities | 256 / 256 | `dcrd/chaincfg/v3@v3.3.0/mainnetparams.go:553-555` | section 4.2.6 |
| Mainnet max transaction size | 393,216 | `dcrd/chaincfg/v3@v3.3.0/mainnetparams.go:104` | section 4.2.8 |
| bolt magic / format version / page size of fixtures | `0xED0CDAED` / 2 / 4096 | fixture meta pages | fixture loader |
| Random stream re-key limits (upstream) | 4 MiB or 20 s | `dcrd/crypto/rand@v1.0.1/prng.go:20-23` | removed in `detrand` |
| Oracle maximum request line | 64 MiB | `dcroxide:tools/oracle/main.go:29-31` | protocol |
| Upstream timing constants in tests | 100 ms waits; 5 s TLS timeout | `wallet/locking_test.go:50, 102, 120, 141`, `x509_test.go:175` | ported tests use the port's own clock control instead |
| Throttle test threshold | 1 | `internal/rpc/jsonrpc/rpcserver_test.go:16` | ported test |
| dcrdtest default dcrd | v1.8.0 | `dcrtest/dcrdtest@v1.0.1-0.20240404170936-a2529e936df1/go.mod:8` | must be overridden |
| vspd minimum wallet versions (at `a381bece`) | dcrwallet 2.1.6; JSON-RPC API 11.0 | `vspd` `rpc/version.go:15-20` | A8 |

---

## 7. Concurrency and lifecycle

### 7.1 Upstream tests

- `t.Parallel()` is used in: all seven `rpc/client/dcrd` tests; `TestMarshalOutpointsToString`
  and its sub-tests; both `rpc/jsonrpc/types` tests; `TestIsMixTx` and its sub-tests;
  `TestReorg`; `TestCoinTypes`, `TestCoinTypeUpgrade`; the `TestUpgrades` sub-tests;
  the four VSP tests; `TestCoinbaseMatured`, `TestTicketMatured`, `TestTicketExpired`,
  `TestSetBirthStateAndScan`. Everything else is sequential within its package.
- Package-level mutable state shared between tests: the snacl variables; the udb
  template database path; the udb seam variable `newSecretKey`; the walletdb driver
  registry (`TestCreateOpenFail` in `wallet/walletdb` registers a driver named
  `createopenfail` that stays registered for the rest of the process).
- Timing-dependent tests: `TestLocking` (sleeps of 100 ms to let a background lock
  happen), `TestThrottle` (relies on the first request blocking in the handler while
  the second arrives), `TestUntrustedClientCert` (retries on connection reset, gives
  up after 5 s).
- No test starts a syncer, a network listener other than `httptest` servers, or more
  than one wallet.
- Goroutines launched by test code, three sites in all: the two concurrent `GET`
  requests of `TestThrottle` (`internal/rpc/jsonrpc/rpcserver_test.go:31`; the handler
  blocks on a channel that the test closes after the **first** response arrives, so
  the first response is necessarily the 429 and the 200 follows); the request retry
  loop of `TestUntrustedClientCert` (`x509_test.go:177`; a new goroutine per attempt,
  results on a channel of capacity 2); and the `Lock` call of `testUnlock`
  (`wallet/locking_test.go:46`). `TestThrottle` never closes its server.
- Database transactions held by tests: `TestCoinbases`,
  `TestStakeInvalidationOfTip`, `TestCoinTypeUpgrade`, `TestChangePassphrase` and each
  `testManagerAPI` pass run their whole scenario inside a single read-write
  transaction, so every read in them sees uncommitted writes of the same transaction.
  `TestInsertsCreditsDebitsRollbacks` commits after every step. A port of these tests
  keeps the same boundaries, since read-your-own-writes inside one transaction and
  visibility after commit are both being exercised.
- `t.Fatal` is called from inside transaction closures in several udb tests
  (`wallet/udb/tx_test.go:263`, `wallet/udb/cointype_test.go:116`), which unwinds
  without rollback or commit. Go-internal; a port returns the failure through the
  closure.
- Sub-tests that share state and must run in order: the five of
  `TestSetBirthStateAndScan` (one wallet), the twelve `acct=N` sub-tests of
  `verifyV12Upgrade` (one read transaction, `wallet/udb/upgrades_test.go:473-475`).

### 7.2 Rig

- **Oracle process.** One oracle process per Rust test process, spawned lazily, killed
  on drop (`dcroxide:crates/dcroxide-testutil/src/lib.rs:565-570`). Requests are
  answered in order; a client that pipelines uses `id` to match. Stateful commands
  (`udb.script`, `wallet.script`, `db.apply`) carry their whole script in one request
  and own their temporary files for the duration, so no state crosses requests and
  tests can share one process.
- **Seed and clock are process-global** in the det build (they replace package
  globals in Go). A test that sets them holds an exclusive lock on the oracle handle
  for its whole scenario, or uses a private oracle process.
- **Files.** Every test works in its own temporary directory. A database file is
  never open in two processes at once: bbolt takes an exclusive file lock on open
  (spec 01), so the alternating-writer procedure waits for one process to exit before
  starting the other.
- **Daemons.** The harness owns process lifetimes: start node, wait for its bound
  address events; start wallet, wait for its listener events; stop in reverse order
  with the same signal a user would send, and assert the exit code. Ports are taken
  as ephemeral and learned from the IPC events, never chosen from a fixed range.
- **Ordering guarantees the rig relies on.** None beyond request/response order on
  one connection. Anything asynchronous (notifications, sync progress streams, mix
  completion) is awaited by predicate with a timeout, and a timeout is a failure with
  the daemon logs attached.
- **CI layout.** Function-level differential tests run in the ordinary test job on
  every supported operating system. Interop and acceptance run in their own jobs with
  prebuilt, cached Go binaries keyed by commit and Go release, as dcroxide caches its
  dcrd build (`dcroxide:docs/adr/0001-oracle-driven-differential-testing.md:331-333`).
  Fuzzing runs nightly with a kept corpus.

---

## 8. Errors

### 8.1 Error strings and kinds pinned by upstream tests

Exact strings (compared with `err.Error()`, so no operation prefix may be added;
`wallet/internal/bdb/driver_test.go:35-71`):

| Call | Text |
| --- | --- |
| `walletdb.Open("bdb", 1, 2, 3)` | `invalid arguments to bdb.Open -- expected database path` |
| `walletdb.Open("bdb", 1)` | `first argument to bdb.Open is invalid -- expected database path string` |
| `walletdb.Create("bdb", 1, 2, 3)` | `invalid arguments to bdb.Create -- expected database path` |
| `walletdb.Create("bdb", 1)` | `first argument to bdb.Create is invalid -- expected database path string` |

These concern Go's variadic driver interface. A Rust API with a typed path has no
such error; they are recorded so the decision to drop them is explicit.

Kinds:

| Kind | Pinned by |
| --- | --- |
| `NotExist` | opening a missing database file; `DeleteNestedBucket` of a missing bucket; `LookupAccount` of an unknown name, and of an old name after rename; `GetVSPTicket` of an unknown ticket |
| `Exist` | `CreateBucket` twice; duplicate driver; `NewAccount`, `RenameAccount`, `ImportVotingAccount`, `importAccount` with a taken name; `importAccount` when an address of the key is already indexed |
| `Invalid` | use of a closed database or transaction; empty bucket name or key; unknown driver; `NewAccount("")`, `NewAccount("imported")`; import with an empty account name; second SLIP0044 upgrade; a 1-byte seed for the voting xpriv |
| `Passphrase` | wrong private or public passphrase (snacl `DeriveKey`, `Manager.Unlock`, `ChangePassphrase`, `Wallet.Unlock`); `ImportVotingAccount` with an empty passphrase |
| `Locked` | `Manager.Lock` when already locked; `NewAccount` while locked; `Encrypt`/`Decrypt` with the private key type while locked |
| `WatchingOnly` | `Lock` and `Unlock` (with a right or a wrong passphrase) on a watching-only manager, which also reports `IsLocked() == true` throughout. The test helpers contain `WatchingOnly` expectations for `NewAccount` and for a private `ChangePassphrase` too, but no test reaches them (section 10, quirk 28). |
| `Crypto` | key generation failure during a passphrase change; a failing private crypto key in `Encrypt` |
| `Encoding` | malformed hashes, headers and transactions from dcrd |
| `InsufficientBalance` | `NewUnsignedTransaction` when inputs do not cover outputs plus fee |

How kinds map to JSON-RPC error codes and gRPC status codes is in spec 16 and the
gRPC specification; no upstream test pins that mapping, so it is pinned by `jsonrpcgen`
and `grpcgen` vectors.

Further error facts the tests rely on:

| Fact | Cite |
| --- | --- |
| Opening a missing database returns kind `NotExist` with the text `missing database file` and does **not** create the file; the check is made only for `Open`. `Create` on a path that already exists does not fail: it opens the file (a zero-length file is initialized by bbolt). Every wallet and udb test helper relies on this, since each creates an empty file with `os.CreateTemp` first and then calls `walletdb.Create` on it. | `wallet/internal/bdb/db.go:316-323`, `wallet/setup_test.go:29-34`, `wallet/udb/txcommon_test.go:23-28` |
| `Manager.Unlock` with a wrong passphrase on an **unlocked** manager returns `Passphrase` and leaves the manager locked; a correct passphrase on an unlocked manager returns nil. | `wallet/udb/addressmanager_test.go:186-210` |
| Kinds in `TestImportVotingAccount` and `TestImportAccount` are read by asserting the returned error to `*errors.Error` and taking its `Kind` field, not through `errors.Is`: the outermost error value must itself carry `Exist`, `Passphrase` or `Invalid`. | `wallet/udb/addressmanager_test.go:560-561, 686-687` |
| `TestCollapse` compares the wrapped error by identity and requires nesting depth 1: `E(E(Op("abc")), Passphrase)` yields one `Error{Op: "abc", Kind: Passphrase, Err: nil}`, not an error wrapping another. | `errors/errors_test.go:36-47` |
| `walletdb.Update` returns the closure's error such that `errors.Is(err, thatError)` holds for a plain `fmt.Errorf` value, and `walletdb.Create`/`Open` do the same for a driver's error. Whether the value is returned unwrapped or wrapped is not distinguished by the tests. | `wallet/walletdb/interface_test.go:514-533`, `wallet/walletdb/db_test.go:91-105` |
| A second `Rollback`, or a `Commit` after `Rollback`, on a read-write transaction returns kind `Invalid`; the first `Rollback` returns nil. | `wallet/walletdb/interface_test.go:668-684` |
| `BirthState` has no error return: it is read inside a `View` and returns a pointer (the test dereferences it without a nil check after a preceding `SetBirthState`). `SetBirthStateAndScan` with a state that sets neither flag returns an error whose kind no test checks. | `wallet/udb/txmined_test.go:62-66`, `wallet/wallet_test.go:205-217` |
| Error texts that exist only in test doubles and are never observable from the daemon: `failed to encrypt`, `failed to decrypt` (the failing crypto key); `duplicate driver allowed for database type [bdb]`; `failed to create or open database for database type [createopenfail]`; `sub test failure`; `force rollback`. | `wallet/udb/internal_test.go:57, 64`, `wallet/walletdb/db_test.go:36-37, 70-71`, `wallet/walletdb/interface_test.go:27, 514` |
| Test failure messages that name a regeneration step: `Generated help texts are out of date: run 'go generate'` and `Generated help usages are out of date: run 'go generate'`. A stale committed help file is therefore an upstream test failure, which is what licenses treating the committed text as the vector. | `internal/rpc/jsonrpc/rpchelp_test.go:44, 83` |

### 8.2 Rig failure policy

| Situation | Outcome |
| --- | --- |
| No Go toolchain and no prebuilt oracle, local run | differential tests print a skip notice |
| The same in CI (`PURSE_REQUIRE_ORACLE`) | failure |
| Oracle built with a Go release other than the reference, in CI | failure |
| Oracle module list disagrees with the pin table | failure of the pin-agreement test |
| Go binary for interop reports a different commit | the harness refuses to run |
| A differential test finds a difference | failure; the minimized input becomes a permanent regression vector; the difference is either fixed or recorded in the quirks ledger with the decision (`dcroxide:docs/dcroxide-project-brief.md:710`) |
| A capture differs outside declared masks | failure; adding a mask requires evidence that Go itself varies in that field |
| An exporter cannot reproduce its old file at the old pin | the regeneration stops until the adaptation is found and recorded |
| The control column of the acceptance matrix fails | harness defect; the cell is not counted for the port |

---

## 9. Compatibility-critical behaviours

What the rig must be able to demonstrate before the port is called a drop-in
replacement. Items 1-16 restate what upstream tests pin; items 17-30 are obligations
that no upstream test covers and that exist only through this plan; items 31-40 are
further behaviours that upstream tests do pin, found on a second pass.

1. The PGP word encoding and the seed mnemonic (words, capitalization, checksum word)
   of section 4.2.1-4.2.2; a user's written seed must restore the same wallet.
2. `DecodeUserInput` accepts both a mnemonic and hex for the same seed.
3. The ten addresses of section 4.2.3 for the fixed seed on simnet, locked, unlocked
   and watching-only.
4. The voting extended private key of section 4.2.4.
5. Legacy and SLIP0044 coin types per network, and the effects of the coin-type
   upgrade on the stored keys (`TestCoinTypeUpgrade`).
6. A wrong passphrase given to `Unlock` on an unlocked wallet locks it; the timeout
   replacement rules of `TestLocking`.
7. The maturity and expiry boundaries of section 4.2.6, including the extra block for
   tickets.
8. The size estimates of section 4.2.5 and the dust and change rules of section
   4.2.8; they decide the fee every transaction pays.
9. The pool-fee amounts of section 4.2.7; a VSP rejects a fee that is too low.
10. `PossibleCoinJoin` on the two reference transactions.
11. Account numbering and iteration order (`[0, 1, …, 2³¹−1]`, imported accounts from
    `2³¹` up in order of import), reserved names, and the error kinds of section 8.1.
12. A `VSPTicket` read returns host and public key joined from their own buckets.
13. JSON-RPC command parsing and marshalling for the 86 literal cases, including
    defaults for omitted optional parameters.
14. Help and usage text equal to the committed generated text for all 103 methods.
15. `null` decodes to an empty list in the dcrd result decoders, and an empty list of
    hashes encodes as `null`.
16. A database at each of the seven fixture versions upgrades to version 26 with the
    states of section 5.2.2.
17. A `wallet.db` last written by Go dcrwallet at version 26 opens in place and every
    query returns what Go returns.
18. A `wallet.db` written by the port opens in Go dcrwallet `54b85b33`, passes
    bbolt's consistency check, and Go can continue writing to it.
19. Upgrading from every database version that a tagged release wrote (3, 4, 5, 6, 7,
    9, 11, 13, 22, 25) yields the same logical database as Go's upgrade.
20. Every JSON-RPC result's bytes (field order, omitted fields, number formatting,
    nil versus empty collections) and every error's code and message, for all 103
    documented methods and the 8 stubs.
21. Every gRPC response message's fields and every status code and message, for the
    88 RPCs, including the order and content of the 12 streams.
22. The requests the wallet sends to dcrd in RPC mode and the messages it sends to
    peers in SPV mode are ones dcrd and dcroxide accept, in an order they accept.
23. The DNS seeders and other network parameters are those of `chaincfg/v3` v3.3.0,
    which is what dcrwallet links, not those of a later dcrd.
24. VSP requests (paths, JSON bodies, signature header) are accepted by vspd, and the
    wallet accepts vspd's signed responses.
25. vspd can use the port as a voting wallet (the eleven calls of scenario A8) and its
    version check on the `version` result passes.
26. A mix that includes the port and Go wallets completes, in either majority.
27. A client certificate issued by the port (`--issueclientcert`) and the server
    certificate it generates are accepted by Go's TLS stack, which dcrctl, vspd and Go
    gRPC clients use.
28. Command-line flags, the configuration file, the `-h` text and the pipe IPC
    messages are the same, so that Decrediton and scripts start the port unchanged.
29. Log lines that tools or users match on (subsystem tags, the creation dialogue,
    the "wallet created" lines) are the same after the timestamp.
30. Nothing in items 17-29 may depend on which backend (dcrd or dcroxide) or which
    sync mode is in use, except where Go dcrwallet itself differs between them.
31. The remaining defaults of section 4.2.11 that clients observe through omitted
    parameters: `listunspent` `maxconf` 9999999, `signrawtransaction` flags `ALL`,
    `listtransactions` count 10 and from 0, `importprivkey` rescan true; and the
    marshalled forms with the always-present `tree` member of transaction inputs.
32. The address-buffer rule of section 4.2.13: the next child index is `lastUsed + 1 +
    cursor`; marking an earlier address used, or running discovery, never moves the
    next address backwards and never skips addresses; all 20 addresses of a gap-limit
    window can be handed out without an error.
33. A wrong passphrase locks an unlocked address manager as well as an unlocked wallet
    (section 5.2.8, step 1.5), and a second correct `Unlock` is accepted.
34. A voting account can be imported while the wallet is locked; importing the same
    extended key twice under two names is accepted until one of its addresses is
    indexed (section 5.2.7).
35. Account 1 created, renamed and persisted is found again after reopen and after
    conversion to watching-only, and a watching-only manager can still rename accounts
    (section 5.2.8).
36. A block whose vote bits disapprove its parent changes balances and the unspent set
    with no transaction inserted or removed (section 5.2.5); a coinbase mined at
    height `h` on a network with maturity 16 is immature while the tip is at most
    `h + 15` (section 5.2.4).
37. `Create` of the database driver on an existing zero-length file succeeds and
    initializes it; `Open` of a missing file fails with kind `NotExist` and creates
    nothing (section 8.1). Tools that pre-create the file (the upstream test helpers
    do) keep working.
38. Reading a key that is absent returns "no value", distinguishable from an empty
    value (section 5.2.3).
39. The sidechain rules of section 5.2.6: a side chain of equal length does not
    replace the main chain, one block longer does; blocks removed by a reorganization
    are no longer reported as in the main chain and are not reported as invalidated.
40. The certificate helpers accept Ed25519, P-256, P-384 and P-521 keys, and the
    client certificate verifies under an authority made from the same key pair
    (section 5.2.10).

---

## 10. Quirks

1. **CI never runs `TestManager`.** `run_tests.sh` passes `-short`
   (`run_tests.sh:9`) and the test skips itself under it
   (`wallet/udb/addressmanager_test.go:1212-1214`).
2. **1,163 test lines are dead.** `wallet/udb/txquery_test.go` and
   `wallet/udb/txexample_test.go` carry `//go:build ignore`
   (`txquery_test.go:6`, `txexample_test.go:6`); `wallet/walletdb/example_test.go`
   carries `//go:build disabled` (`example_test.go:7`). All three use APIs that no
   longer exist (`s.InsertTx`, `testStore()`, `db.Namespace`). The two "examples" in
   `txexample_test.go` take a `*testing.T` parameter, so they would not be valid
   examples even if built.
3. **`v24.db.gz` is unused.** No entry of `dbUpgradeTests` names it
   (`wallet/udb/upgrades_test.go:24-38`).
4. **`v7.go` cannot compile as written.** It calls `errors.Errorf`
   (`wallet/udb/testdata/v7.go:133, 154, 160, 178`) without importing an `errors`
   package (`:11-31`).
5. **Fixtures are not reproducible** and six of them cross networks: created with
   `chaincfg.TestNet2Params` from a random seed (`v1.db.go:32, 53`), upgraded by the
   test with `TestNet3Params` (`upgrades_test.go:75`).
6. **`v11.db.go` copies an empty buffer** into `SerializedHeader`: the header is
   never serialized into `buf` (`v11.db.go:352-364`). Harmless there because
   `ExtendMainChain` is given the header itself.
7. **`TestCoinbases` never checks the spendable amount.** The second condition
   repeats the first (`wallet/udb/tx_test.go:416-424`), so every `spendable` value in
   the test's tables is unverified upstream. The port should assert both and take the
   expected spendable values from the oracle.
8. **`TestInsertsCreditsDebitsRollbacks` discards an error** in the "insert duplicate
   confirmed" step (`tx_test.go:200-201`) and reads the "unconfirmed" balance with the
   same `minConf = 1` call as the spendable one (`:267-282`).
9. **snacl tests are order-dependent** through package variables
   (`wallet/internal/snacl/snacl_test.go:14-20`): run in isolation, the later tests
   would operate on nil package variables.
10. **Tests write into the package directory**: the bdb, walletdb and udb tests create
    their files by relative name (section 5.1, items 4 and 7).
11. **Cursor deletion during iteration is untested.** `testCursorDelete` exists but is
    skipped with a reference to a bolt defect (`wallet/walletdb/interface_test.go:99-101,
    202-205`).
12. **The txauthor test's "P2PKH outputs" carry 36-byte scripts**
    (`wallet/txauthor/author_test.go:34`): the output size constant is used as a script
    length. The expectations are self-consistent because they are computed from the
    same outputs.
13. **A failed `Unlock` locks an unlocked wallet**
    (`wallet/locking_test.go:61-85`).
14. **Ticket maturity is one block later than coinbase maturity** for the same
    parameter value, with the comment "dcrd off-by-one"
    (`wallet/wallet_test.go:58`, `wallet/wallet.go:4435-4440`).
15. **A known address-buffer defect is worked around in a test.**
    `TestDiscoveryCursorPos` sets `DisableCoinTypeUpgrades` because "the buffers
    record off-by-ones after the upgrade" (`wallet/discovery_test.go:20-22`), with the
    upgrade call commented out (`:26-33`). The behaviour after a coin-type upgrade is
    therefore untested and admitted to be off; see spec 02's quirk on the SLIP0044
    upgrade.
16. **The upgrade test reads version-5 rows from a version-26 database.**
    `verifyV5Upgrade` calls `fetchDBAccount(ns, acct, 5)` after the full upgrade
    (`wallet/udb/upgrades_test.go:215-218`) and expects the last-used and
    last-returned fields there. Whether later upgrades leave those row fields intact
    or the reader ignores what changed is for specs 02 and 05; the fixture replay pins
    the outcome either way.
17. **`VSPTicket` serialization omits two of its fields** by design
    (`wallet/udb/vsp_test.go:164-173`) while `GetVSPTicket` returns them
    (`:216-227`).
18. **`TstLatestMgrVersion` is exported for tests and used by none**
    (`wallet/udb/internal_test.go:22`).
19. **`TestSetBirthState` uses random inputs**, including independently random
    `SetFromHeight` and `SetFromTime` flags (`wallet/udb/txmined_test.go:43-49`), so
    it can store a state with both set or neither.
20. **`TestUntrustedClientCert` accepts two different TLS error strings and retries on
    resets** (`x509_test.go:192-202`): what the client sees depends on Go's TLS
    version and timing.
21. **Re-importing the same account key is allowed** until one of its addresses is
    indexed: `TestImportAccount` imports the same `tprv` twice under different names
    successfully and gets `Exist` only after `SyncAccountToAddrIndex` on the first
    copy (`wallet/udb/addressmanager_test.go:627-647, 673-678`).
22. **The upstream random stream XORs into the caller's buffer**
    (`dcrd/crypto/rand@v1.0.1/prng.go:100, 105`). Callers pass zeroed buffers, but a
    replacement stream that overwrote instead would differ for any caller that does
    not.
23. **`chaingen` output differs on every run** (section 5.9), so the two wallet tests
    built on it assert only relations between blocks, never bytes.
24. **Type tests cover 43 of 104 commands and no result type** (section 3.2).
25. **The dcrtest harness defaults to dcrd v1.8.0** (`dcrdtest` `go.mod:8`), far
    behind the node the wallet at the pin requires (JSON-RPC API 8.3.0); used without
    `SetPathToDCRD` it would start a node the wallet refuses.
26. **Tag families disagree.** `v1.7.0` reports database version 22 while
    `release-v1.7.0` reports 25 (section 5.10.3); only the `release-` family was used.
27. **Module versions differ from dcroxide's parity pin.** Seven dcrd modules differ
    in non-test source (section 5.4.2), `dcrec/secp256k1/v4` in code rather than
    comments.
28. **Two helper branches are never reached.** `testNewAccount` checks that
    `NewAccount` returns `WatchingOnly` on a watching-only manager
    (`wallet/udb/addressmanager_test.go:822-828`), but returns early unless
    `tc.create` is set (`:818-820`), and no caller sets `create` together with
    `watchingOnly`. `testChangePassphrase` has watching-only branches (`:768-781`)
    but its only caller is `TestChangePassphrase`, with `watchingOnly` false
    (`:1100-1107`). What the watching-only passes of `TestManagerWatchingOnly` do
    reach: `Lock`/`Unlock` errors, account lookups, `ForEachAccount`, and
    `RenameAccount`, which succeeds on a watching-only manager.
29. **A shared test configuration is mutated.** `testWallet` stores the opened
    database handle in the caller's `Config` (`wallet/setup_test.go:47`);
    `TestAddresses` and `TestLocking` pass the package-level `basicWalletConfig`
    (`wallet/addresses_test.go:123`, `wallet/locking_test.go:18`), so the global
    keeps a handle to a closed database between tests. The parallel tests copy the
    struct first (`wallet/reorg_test.go:159`).
30. **`chaingen` blocks depend on the core count and on scheduling**, not only on the
    clock and the random stream: the nonce search runs on `runtime.NumCPU()`
    goroutines and keeps the first solution reported
    (`dcrd/blockchain/v5@v5.1.1/chaingen/generator.go:2068-2085`). A seeded random
    stream alone does not make `chain.gen` reproducible (section 5.9).
31. **The test filter helper's "P2SH script" is malformed.** `chaingenPrevScripter`
    returns 22 bytes with no push opcode before the hash
    (`wallet/reorg_test.go:50-56`), although its comment calls it the p2sh `OP_TRUE`
    script.
32. **Store tests record a zero received time.** `NewTxRecordFromMsgTx(tx,
    time.Time{})` is used throughout (`wallet/udb/tx_test.go:37, 55, 61`,
    `wallet/udb/stakevalidation_test.go:69, 73`), and the v5, v7 and v11 generators
    do the same with a zero `epoch` (`wallet/udb/testdata/v5.db.go:52, 161`,
    `v7.go:36, 136`, `v11.db.go:64, 288`). Go's zero time has `Unix()` =
    −62,135,596,800, which the record writer stores as `uint64(rec.Received.Unix())`
    (`wallet/udb/txdb.go:524`), that is `0xFFFFFFF1886E0900`. (What the old releases
    that wrote the fixtures stored for it was not checked; the fixture dumps show
    it.) A port's clock type must
    be able to round-trip that value when it replays these scenarios or reads those
    fixtures; a type that clamps to the Unix epoch produces different bytes.
33. **Fixture block hashes are not header hashes.** The v5 and v7 generators insert
    headers under invented hashes of the form 31 zero bytes followed by the height
    (`chainhash.Hash{31: byte(height)}`, `wallet/udb/testdata/v5.db.go:127-135`,
    `v7.go:111`), with `PrevBlock` chained through those invented hashes. A port that
    re-hashes stored headers and compares against their keys while opening or
    upgrading would reject fixtures that Go accepts. The v11 generator uses real
    header hashes (`v11.db.go:360-362`).
34. **`TestCoinbases` makes 19 headers and 18 filters and uses 17 of each**
    (`wallet/udb/tx_test.go:368-380`).
35. **`TestImportVotingAccount` and `TestImportAccount` have a case named "ok watching
    only" that is not watching-only.** The flag only reaches the test context
    (`wallet/udb/addressmanager_test.go:495-499, 532`); the manager is an ordinary
    one, and `TestImportAccount` even unlocks it in that case (`:662`).
36. **Two of the command cases named "optional3" do not exercise a third value.**
    `listreceivedbyaccount optional3` and `listreceivedbyaddress optional3` pass
    `false` for `includewatchonly`, which is also the default
    (`rpc/jsonrpc/types/methods_test.go:486-499, 546-559`).
37. **`verifyV2Upgrade` carries an unused column and a copied message.** The
    `totalAddrs` values (20, 20, 20, 30, 30, 30, 200, 200) are never compared, and the
    internal-branch check reports "external address pool bucket still exists"
    (`wallet/udb/upgrades_test.go:96-127`).
38. **`TestSetBirthState` compares `time.Time` values with `!=`**
    (`wallet/udb/txmined_test.go:72`), which compares Go's internal representation
    (wall, monotonic reading, location pointer), so `BirthState` must rebuild the time
    exactly as `time.Unix(seconds, 0)` does. Go-internal; a port compares the Unix
    second.
39. **The dcrd decoder tests compare with `reflect.DeepEqual`** against headers built
    with `time.Unix` and transactions built with empty, non-nil input and output
    slices (`rpc/client/dcrd/json_test.go:244, 291-296, 321`). The nil-versus-empty
    distinction is Go-internal.
40. **The two copies of the database conformance suite have drifted** (section
    5.2.3): only the walletdb copy distinguishes nil from empty and re-puts the keys
    before the nested-bucket checks. The bdb copy says so itself: "Test must be
    updated for API changes" (`wallet/internal/bdb/interface_test.go:14`).
41. **`TestChangePassphrase`'s comment for step 4 says "Change the private passphrase
    back"** while the call changes the public one
    (`wallet/udb/addressmanager_test.go:756-757`).
42. **`TestRPCMethodUsageGeneration` builds the set of documented handlers and never
    checks it** (`internal/rpc/jsonrpc/rpchelp_test.go:91-94`); only the help test
    reports a handler without a help entry. Both tests stop comparing text after the
    first mismatch (the `needsGenerate` flag), so a failure does not say which method
    is stale.
43. **The client certificate tests issue the client certificate to the authority's
    own key** (`x509_test.go:67-77`); a certificate for a key different from the
    authority's is not tested. `TestClientCert` also assigns the key pair to one
    `s.Client()` transport and sends with a second `s.Client()` call (`:98-107`),
    which works only because `httptest.Server.Client` returns the same client each
    time.
44. **`TestThrottle` leaks its server** (no `Close`,
    `internal/rpc/jsonrpc/rpcserver_test.go:19-23`) and asserts only the multiset of
    status codes.
45. **`TestDiscoveryCursorPos` fetches the address manager bucket read-only inside a
    read-write transaction** and under the non-test name of the key
    (`dbtx.ReadBucket(waddrmgrNamespaceKey)`, `wallet/discovery_test.go:72`), then
    passes the transaction itself to the write calls. Its comment says "0-9 have been
    returned" for a cursor of 9, which by the rule of section 4.2.13 means children
    0-8 have been returned and 9 is next.
46. **`dcroxide`'s `mempoolvectors` exporter has no `run.sh`** and must be copied into
    the checkout (`dcroxide:tools/mempoolvectors/vectors_export_test.go:10-15`),
    unlike the overlay exporters. The wallet rig should not copy that form: it writes
    into the upstream tree.

---

## 11. Upstream tests

### 11.1 Coverage by package

Lines are non-test / test. "Live" excludes build-tag-disabled files.

| Package | Lines | Tested | Not tested |
| --- | --- | --- | --- |
| `.` (main) | 3,039 / 211 | client certificate issuance | config, startup, wallet setup dialogue, IPC, RPC server wiring, signals |
| `chain` | 1,122 / 88 | version comparison | RPC syncer, notifications, backend |
| `cmd/movefunds`, `cmd/repaircfilters`, `cmd/sweepaccount`, `cmd/treasurykey` | 1,051 / 0 | — | everything |
| `deployments` | 157 / 0 | — | everything |
| `errors` | 400 / 81 | collapse and matching | message formatting |
| `internal/cfgutil`, `internal/loader`, `internal/prompt`, `internal/loggers`, `internal/netparams`, `internal/compat` | 1,425 / 0 | — | everything |
| `internal/rpc/jsonrpc` | 6,843 / 165 | help generation, the throttle wrapper | all 111 handlers, server transport, authentication, websocket |
| `internal/rpc/rpcserver` | 4,393 / 0 | — | the entire gRPC server |
| `internal/rpchelp` | 1,286 / 0 | indirectly by the help test | — |
| `assets`, `kdf`, `lru`, `validate`, `version` | 507 / 0 | — | everything |
| `rpc/grpc_example`, `rpc/rpc_example` | 431 / 0 | — | example programs |
| `p2p` | 2,262 / 0 | — | everything |
| `spv` | 2,761 / 0 | — | everything |
| `ticketbuyer` | 359 / 0 | — | everything |
| `pgpwordlist` | 609 / 66 | two vectors each way | error paths |
| `walletseed` | 102 / 88 | four vectors each way | error paths, generation |
| `rpc/client/dcrd` | 997 / 379 | five JSON codecs | the client |
| `rpc/client/dcrwallet` | 1,279 / 190 | one marshaller, one result shape | the client |
| `rpc/jsonrpc/types` | 1,825 / 1,498 | 43 commands | 61 commands, every result type |
| `rpc/walletrpc` | 18,744 / 0 | — | generated code |
| `wallet` | 16,726 / 1,414 | address derivation and buffers, locking, sidechain forest and chain switch, birthday, three maturity predicates, coinjoin heuristic, voting xpriv | transaction creation, coin selection, tickets, voting, revocation, VSP, mixing, rescan, discovery against data, notifications, multisig, balances, listings |
| `wallet/internal/bdb` | 417 / 879 | conformance, persistence, argument errors | `Copy`, cursors beyond `ForEach`, `KeyN` |
| `wallet/walletdb` | 425 / 851 live | registry, conformance | cursor deletion |
| `wallet/internal/snacl` | 249 / 118 | round trips | byte formats |
| `wallet/txauthor` | 234 / 239 | `NewUnsignedTransaction` | change randomization, signing |
| `wallet/txrules` | 281 / 93 | pool fee | dust, fee, high-fee checks |
| `wallet/txsizes` | 209 / 70 | one of four functions | the rest |
| `wallet/udb` | 15,269 / 3,339 live | manager API, imports, passphrases, coin-type upgrade, seven upgrade fixtures, two store scenarios, stake invalidation of the tip, VSP rows, birthday | most of the transaction store (tickets, votes, multisig, queries, range iteration, unmined pruning, balances beyond the simple cases), treasury policies, agenda preferences, upgrade steps 13-26 beyond not failing, migration from the pre-unified layout |

### 11.2 Port ledger summary

| Class | Tests | Action |
| --- | ---: | --- |
| P | 49 | transcribe |
| P+R | 13 | transcribe, add oracle byte vectors |
| R | 9 | port the scenario, take expectations from an exporter or the oracle |
| F | 1 (7 fixtures, plus the unused eighth) | carry fixtures, replay, add full-dump comparison |
| G | 5 | re-express what is observable; record the rest as not applicable |
| D | 7 functions in 3 files | not ported; `txquery_test.go` is useful as a list of query cases to regenerate (spec 07 says the same) |

(The counts are of the 77 live functions plus the dead ones; a function listed with
two classes in section 3.2 is counted under the first.)

### 11.3 Dependency suites

Not in this spec's scope but part of layer 1; sizes at the versions dcrwallet pins
(non-test / test lines):

| Module | Lines | Owner spec |
| --- | --- | --- |
| `go.etcd.io/bbolt` v1.3.12 | 9,500 / 10,038 | 01 |
| `dcrd/hdkeychain/v3` v3.1.3 | 732 / 1,330 | 02 |
| `dcrd/mixing` v0.7.5 | 7,336 / 1,489 | 12 |
| `decred.org/cspp/v2` v2.4.0 | 5,067 / 1,378 | 12 |
| `companyzero/sntrup4591761` | 1,155 / 132 (+ one `testdata` directory) | 12 |
| `dcrd/addrmgr/v3` v3.0.0 | 1,919 / 1,697 | 14 |
| `dcrd/connmgr/v3` v3.1.4 | 1,377 / 1,263 | 14 |
| `dcrd/gcs/v4` v4.1.1 | 1,376 / 1,531 | 09, 14 (dcroxide has a port) |
| `dcrd/dcrjson/v4` v4.2.0 | 2,478 / 2,539 | 16 (dcroxide has a port) |
| `dcrd/rpc/jsonrpc/types/v4` v4.4.0 | 2,540 / 1,699 | 15, 16 (dcroxide has a port) |
| `vspd/client/v4` v4.0.2, `vspd/types/v3` v3.0.0 | 495 / 330 | 11 |
| `dcrd/certgen` v1.2.0 | 365 / 125 | 16 (dcroxide has a port) |
| `jrick/wsrpc/v2` v2.3.8 | 1,005 / 0 | 15 |

`wsrpc`, the websocket JSON-RPC client the wallet uses to talk to dcrd, has no tests.

### 11.4 Vectors to regenerate

Each sibling spec lists its own; this is the index of which tool serves them.

| Spec | Subject | Tool |
| --- | --- | --- |
| 01 | page layouts, scripted transactions, check output | `db.apply`, `db.dump`, `db.check`, `db.pages`, `bdbgen` |
| 02 | manager rows, key hierarchy, encryption formats, account kinds | `udbrowgen`, `snaclgen`, `kdf.derive`, `hd.derive`, `udb.script` |
| 03, 04 | store rows, scripted store sequences, query results, stake invalidation post-states | `udbrowgen`, `udb.script` |
| 05 | every upgrade step's post-state; fixtures at missing versions | `udb.script` on fixtures; historical fixture builds |
| 06-08 | wallet methods over fixture databases; float formatting; notification order | `wallet.script`, `walletgen`, captures |
| 09 | chain switch, rescan, discovery | `chain.gen` fixtures + `wallet.script`, `addrbufgen` |
| 10 | sizes, fees, dust, authoring, signing | `txsizes.*`, `txrules.*`, `tx.author`, `tx.sign`, `walletgen` |
| 11 | ticket, vote, revocation construction; VSP requests | `walletgen`, VSP capture |
| 12, 13 | mixing primitives and wallet integration | `mix.*`, `walletgen`, scenario A9 |
| 14 | wire conversation, address manager file | P2P capture; dcroxide's oracle for wire framing |
| 15 | dcrd conversation | backend capture, `dcrdjsongen` |
| 16 | command parsing, results, help, errors | `rpc.cmd`, `rpc.result`, `helpgen`, `jsonrpcgen`, captures |
| gRPC, config, lifecycle | messages, handler results, flags, help, IPC | `grpc.msg`, `grpcgen`, `configgen`, `cfgutilgen`, `promptgen`, captures |

### 11.5 Untested upstream and still hard to test here

- Concurrent access patterns inside the daemon (a rescan while a block arrives while
  a ticket is purchased): only the acceptance matrix and soak runs reach them.
- Cross-process database locking and behaviour on Windows.
- Power loss between a database write and its sync.
- Long-running behaviour: reconnection and backoff over hours, mix sessions over
  many epochs, fee-payment retries over the days a VSP allows.
- Mainnet- and testnet-only paths (DNS seeding, checkpoints, real difficulty).
- Decrediton, until its dependencies are read.

---

## 12. Rust porting notes

### 12.1 Deliverables

| Item | Kind | Mirrors | Contents |
| --- | --- | --- | --- |
| `tools/oracle` | Go module | `dcroxide:tools/oracle` | `dcrwallet-oracle`, the commands of section 5.5.2; `detrand/` replacement package; unit tests for every handler |
| `tools/<name>gen` (twelve) | Go test files + `run.sh` | `dcroxide:tools/mixpoolgen`, `chainopsgen`, `helpgen` | overlay exporters of section 5.5.3 |
| `tools/capture` | Go program | — | records JSON-RPC and gRPC sessions using dcrwallet's own client packages (`rpc/client/dcrwallet`, `rpc/walletrpc`) and a recording websocket proxy for the dcrd side |
| `tools/pinbump` | Go program | `dcroxide:tools/pinbump` | reuse; point it at the wallet's parity ledger |
| `purse-testutil` | Rust crate | `dcroxide-testutil` | oracle client, node and wallet process harness, dump comparator, capture replayer, chain-fixture loader, seeded test RNG |
| `purse-rand` | Rust crate | `dcrd/crypto/rand` | the stream and the derived-value functions of section 5.6.2, with a test-only seeded constructor |
| clock abstraction | module in the wallet core crate | — | the only reader of wall-clock time |
| `fuzz/` | cargo-fuzz workspace | `dcroxide:fuzz` | targets of section 5.12 |
| `docs/PARITY.md`, `docs/QUIRKS.md`, ADR "oracle rig" | documents | dcroxide's | per-test ledger; quirk decisions; the pin and reference-Go rules |

### 12.2 Reuse from dcroxide (looked at)

- `dcroxide-testutil`: `Oracle` (spawn, `call`, kill on drop), the skip-or-fail
  policy functions, `SplitMix64` with its seed environment variable, `TempDir`, hex
  helpers, and `DcrdNode` (`dcroxide:crates/dcroxide-testutil/src/lib.rs:67, 231, 430,
  594, 786`). The oracle client can be reused nearly as is with a different binary and
  environment variable names. `DcrdNode` starts dcrd with `--norpc`
  (`dcroxide:crates/dcroxide-testutil/src/lib.rs:637`); the wallet harness needs a variant with RPC enabled, a mining
  address and certificates.
- The power-loss and gate storage backends
  (`dcroxide:crates/dcroxide-testutil/src/powerloss.rs:47, 166`,
  `dcroxide:crates/dcroxide-testutil/src/gate.rs:65-73`) implement or wrap a
  `redb::StorageBackend`. The wallet's file is bbolt-format, so the idea carries over
  but the code does not.
- `tools/oracle` of dcroxide already serves hashing, signatures, wire framing, script
  execution, standard addresses and scripts, stake transaction analysis and GCS
  filters. The wallet reuses those commands for the dcroxide crates it links, but
  rebuilt against the module versions dcrwallet pins (section 5.4.2); the simplest
  arrangement is one handler source tree compiled into both oracles.
- The fuzz workspace layout and the oracle-driving differential target pattern
  (`dcroxide:fuzz/fuzz_targets/dcrec_diff_ecdsa.rs`).

### 12.3 Reuse from dcr-rs (looked at)

`dcr-rs:src/hd.rs` implements Decred HD derivation with both the legacy
(zero-stripping) and strict child derivations (`derive_child` at `:280`,
`derive_child_bip32_std` at `:292`), extended-key base58 (`:468, 503`) and P2PKH
addresses (`:458`). It is built on the `secp256k1` crate's context type (`:258`). It is
useful as an independent second implementation to cross-check `hd.derive` vectors;
whether it may be a production dependency is for the address-manager spec, since
dcroxide's policy is pure Rust (`dcroxide:docs/adr/0012-pure-rust.md`).

### 12.4 Hazards

- **Building the rig late.** dcroxide's ADR records the cost of vectors whose
  exporters were scratch code. Build `tools/oracle`, `db.dump` and `udb.script`
  before the first storage crate.
- **Reading masks as licence.** Every mask hides bytes from comparison. The plaintext
  dump and the det build exist so that masked bytes are still compared somewhere.
- **Seed-order coupling.** If the port adopts Go's draw order to get byte equality,
  refactoring Rust code can break tests without changing behaviour. Keep the coupling
  to the storage and transaction-construction paths and document each draw.
- **Go standard library behaviour in vectors.** JSON number formatting, HTML escaping
  in JSON strings, base64, TLS and HTTP error texts come from the Go release, not from
  dcrwallet. The reference-Go rule exists for that
  (`dcroxide:docs/adr/0001-oracle-driven-differential-testing.md:259-281`).
- **Old toolchains for historical fixtures.** Releases back to v1.0.0 need Go
  versions and dependency trees of their time; fetching and building them is the
  slowest single task in this plan and can fail for reasons unrelated to the port.
- **Acceptance flakiness.** Mixing needs four live participants and epochs; VSP flows
  need confirmations. Each scenario needs explicit predicates and generous timeouts,
  and the control column to tell a harness defect from a finding.

### 12.5 Size

**XL** for the rig as a whole, comparable to a large implementation crate, because
upstream supplies so little:

| Part | Size | Reasoning |
| --- | --- | --- |
| Porting upstream tests and fixtures | S | about 60 portable functions, mostly tables |
| `tools/oracle` with pure-function commands and `db.*` | M | dcroxide's oracle is 2,710 lines for a comparable command count |
| `udb.script`, `wallet.script`, det build | L | a scripting layer over two large APIs plus the seams |
| Overlay exporters | L | twelve tools; the handler-level ones need a scripted wallet behind them |
| Captures and replayers | L | five interfaces, masks, a proxy, fixtures |
| Database battery including historical fixtures | L | mostly waiting on old builds and on the storage crates |
| Acceptance harness and matrix | L | process orchestration for node, wallets, vspd, mixing peers on three operating systems |
| Fuzz workspace | M | about thirty targets, a third of them differential |

---

## 13. Open questions

1. **Reference Go release.** Decided (project owner, 2026-11-18): go1.27.4, the
   latest release at the time, pinned and moved by a deliberate bump, not tracked.
   It is `REFERENCE_GO` in `purse-testutil` and `GO_VERSION` in the CI workflow, and
   the vectors frozen so far were generated with it. dcrwallet has no release build
   recipe in its repository; the options had been dcroxide's reference (go1.27.3,
   `dcroxide:crates/dcroxide-testutil/src/lib.rs:207`) or the Go that built the
   published dcrwallet binary of the target release.
2. **Seeded-stream parity.** Does the port commit to consuming random bytes in Go's
   order and sizes on storage and transaction-construction paths (enabling byte-exact
   comparison under the det build), or are those paths compared only through masks,
   plaintext dumps and structural checks? Decision.
3. **Page-level identity.** Is a database written by the port required to be
   byte-identical to what bbolt would have written for the same operations, or only
   logically equal and valid for bbolt? This plan assumes the latter; spec 01 owns
   the decision.
4. **Historical floor.** Which database versions get dedicated fixtures: only those a
   tagged release wrote, or all 26; and is the pre-unified migration (releases before
   v1.0.0) in scope? Decision.
5. **Address vector paths.** Which coin type and child indexes produce the addresses
   of section 4.2.3 and the two simnet addresses from dcrd's script. UNVERIFIED;
   settled by the first `hd.derive` run.
6. **dcrtest wallet harness.** Whether the dcrtest repository publishes a dcrwallet
   harness package compatible with the pin. UNVERIFIED; the module cache holds only
   `dcrdtest`. Settled by listing the repository.
7. **Decrediton's dependencies.** Flags, IPC messages and gRPC calls it uses were not
   read. Settled by reading its launcher and client code or by recording a session.
8. **dcrctl's wallet command list.** Not read; scenario A15 enumerates it from the
   dcrctl binary at run time.
9. **vspd version.** Which vspd release pairs with `vspd/client/v4` v4.0.2 for
   scenario A7 and A8; the copy read here (`a381bece`) is not a pin.
10. **secp256k1 skew.** Whether v4.4.1 and dcroxide's pinned v4.4.2 pseudo-version
    differ in any output. Settled by running dcroxide's signature differential suite
    against an oracle built with v4.4.1.
11. **`-overlay` on module-cache files.** Not checked; the plan uses a checkout.
12. **gRPC byte equality.** Whether the Rust protobuf encoder chosen produces the same
    bytes as Go's for every message (expected for a schema without maps), or whether
    comparison stays at field level. Settled by `diff_grpc_msg`.
13. **Upstream test defects.** Whether to report `TestCoinbases`' missing assertion
    and the non-compiling `v7.go` upstream. They do not affect the port.
14. **Supported operating systems for the acceptance matrix.** dcroxide tests on
    Linux (two architectures), macOS and Windows
    (`dcroxide:docs/adr/0001-oracle-driven-differential-testing.md:315-318`); the wallet's
    database locking and IPC differ per platform, so the same set is proposed.
    Decision.
15. **Reproducible chain fixtures.** Whether `chain.gen` runs against a seamed copy of
    `chaingen` (fixed block-one time, seeded stream, one solver goroutine; section
    5.9), making chain fixtures regenerable under the frozen-battery rule, or whether
    chain fixtures are one-off recordings exempt from it. Decision; the seamed form is
    recommended because dcroxide already carries the technique.
16. **Zero received time.** Whether the port's record type can carry the Go zero time
    (Unix second −62,135,596,800) that upstream store tests and three fixtures write
    (quirk 32). Settled by the first `udbrowgen` vector for a transaction record with
    that time and by reading spec 03's row layout.
17. **Synthetic block hashes in fixtures.** Whether any open, upgrade or consistency
    path of the port recomputes header hashes (quirk 33). Settled by the fixture
    upgrade battery of section 5.10.2, which fails on the v5 and v7 fixtures if one
    does.
18. **`PrevScript` in the reorg tests.** Whether `blockcf2.Regular` consults the
    helper for any block the wallet tests generate (section 3.4). UNVERIFIED; settled
    by logging in `chain.gen`.
19. **Empty values in the key/value layer.** No upstream test stores an empty value;
    what `Get` returns for one (nil or empty) is unpinned upstream and must come from
    `db.apply` vectors against bbolt v1.3.12 (spec 01).
