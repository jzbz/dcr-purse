# 12 — StakeShuffle mixing client protocol (dcrd mixing module as linked by dcrwallet)

## Header

**Scope** (every file below was read end to end at the pinned version):

| Module | File | Lines |
|---|---|---|
| `dcrd/mixing@v0.7.5` (root) | `client.go` | 9 |
| | `dcnet.go` | 177 |
| | `errors.go` | 28 |
| | `expiry.go` | 18 |
| | `field.go` | 21 |
| | `flags.go` | 12 |
| | `keyagreement.go` | 237 |
| | `limits.go` | 42 |
| | `message.go` | 41 |
| | `scriptclass.go` | 16 |
| | `sid.go` | 122 |
| | `signatures.go` | 123 |
| | `vec.go` | 78 |
| `dcrd/mixing@v0.7.5/mixclient` | `client.go` | 2606 |
| | `blame.go` | 493 |
| | `coinjoin.go` | 295 |
| | `limits.go` | 95 |
| | `errors.go` | 26 |
| | `testhooks.go` | 16 |
| `dcrd/mixing@v0.7.5/utxoproof` | `utxoproof.go` | 82 |
| `dcrd/mixing@v0.7.5/internal/chacha20prng` | `prng.go` | 53 |
| `dcrd/wire@v1.8.0` | `msgmixpairreq.go` 523, `msgmixkeyexchange.go` 252, `msgmixciphertexts.go` 253, `msgmixslotreserve.go` 346, `msgmixfactoredpoly.go` 280, `msgmixdcnet.go` 336, `msgmixconfirm.go` 248, `msgmixsecrets.go` 376, `mixvect.go` 29 | 2643 |
| `cspp/v2@v2.4.0` | `solverrpc/rpc.go` 168, `cmd/csppsolver/solver.go` 90, `solver/solver.go` 126 | 384 |
| `sntrup4591761@v0.0.0-20220309191932-9e0f3af2f07a` | `sntrup4591761.go` 164, `zx/random.go` 98, `zx/encoding.go` 46, `r3/r3.go` 96, `r3/mod3/mod3.go` 50, `r3/vector/vector.go` 41, `rq/rq.go` 102, `rq/modq/modq.go` 77, `rq/vector/vector.go` 41, `rq/encoding.go` 231 | 946 |
| dcrwallet | `testsolver.go` 35, `config.go:739-758` (solver checks), `wallet/mixing.go:77-175,560-597` and `wallet/wallet.go:5525-5552` (the `mixclient.Wallet` adapter, read for the caller contract only) | — |

Read for the contract the client depends on, but specified elsewhere: `dcrd/mixing@v0.7.5/mixpool/mixpool.go` (`Receive`, `AcceptMessage`, `CompatiblePRs`, `ReceiveKEsByPairing`, removal and expiry functions), `mixpool/observer.go`, `mixpool/limits.go`, `dcrd/dcrutil/v4@v4.0.3/txsort/txsort.go`, `dcrd/dcrec/secp256k1/v4@v4.4.1` (`privkey.go`, `ecdh.go`, `schnorr/signature.go`), `dcrd/crypto/rand@v1.0.1`.

**Parity pin:** dcrwallet `54b85b33` (2.2.0-pre, `decred.org/dcrwallet/v5`). Dependency versions from its `go.mod`: `github.com/decred/dcrd/mixing v0.7.5`, `github.com/decred/dcrd/wire v1.8.0`, `decred.org/cspp/v2 v2.4.0`, `github.com/companyzero/sntrup4591761 v0.0.0-20220309191932-9e0f3af2f07a`, `github.com/decred/dcrd/dcrec/secp256k1/v4 v4.4.1`, `github.com/decred/dcrd/crypto/blake256 v1.1.0`, `github.com/decred/dcrd/crypto/rand v1.0.1`, `github.com/decred/dcrd/dcrutil/v4 v4.0.3`, `github.com/decred/dcrd/txscript/v4 v4.1.2`. In citations the sntrup module is abbreviated `sntrup4591761@9e0f3af2`.

**Summary.** The mixing client is the wallet-side half of Decred's peer-to-peer StakeShuffle (CoinShuffle++ with DiceMix Light) protocol. A wallet submits one pair request (PR) per coinjoin contribution; at every epoch boundary (10 minutes on mainnet, 3 minutes on testnet3) all peers that hold the same set of compatible PRs derive the same session ID and peer order, then run six broadcast stages — key exchange (KE), ciphertexts (CT), slot reservation (SR), factored polynomial (FP, by solver-capable peers only), XOR DC-net (DC) and confirmation (CM) — each bounded by a stage timeout (30 s mainnet, 10 s testnet3). Pairwise shared keys come from a hybrid of secp256k1 ECDH and the sntrup4591761 KEM, all derived from a per-peer ChaCha20 PRNG whose seed is committed to in the KE so that it can later be revealed (RS, "secrets") for blame assignment. A successful run yields a deterministic, sorted coinjoin transaction that every peer signs and one local peer publishes. Failure paths exclude peers that timed out or provably misbehaved and re-form the session with a new session ID; the run number on the wire is always 0. There is no persisted state: nothing in scope touches `wallet.db`. Root finding for the slot-reservation polynomial is delegated to an external `csppsolver` process spoken to with Go `net/rpc` (gob) over stdin/stdout. Contrary to the name sometimes used for the key agreement, no X25519 is involved at this pin: the ECDH half is secp256k1 (`dcrd/mixing@v0.7.5/keyagreement.go:76-80`).

---

## 2. Role in the system

**What it does.** Given a `CoinJoin` description (inputs with ownership proofs, an optional change output, a mixed output amount, a message count and an address generator), `Client.Dicemix` blocks until the described outputs have been created in a published coinjoin transaction with other network participants, or until it fails.

**Who calls it** (dcrwallet side; the wallet-mixing specification owns these callers):

- `Wallet.Run` creates one `mixclient.Client` per wallet when mixing is enabled, sets its logger to the `MIXC` subsystem, stores it in an atomic pointer and blocks in `Client.Run(ctx)` (`wallet/wallet.go:5538-5552`).
- `Wallet.dicemix` → `Client.Dicemix` for each output being mixed (`wallet/mixing.go:591-597`, `wallet/mixing.go:420-434`) and for mixed ticket-split transactions (`wallet/createtx.go:1076-1092`).
- `Wallet.expireMixMessages(height)` → `Client.ExpireMessages` with the new tip height when the main chain is extended or reorganized (`wallet/chainntfns.go:310`, `wallet/mixing.go:583-589`).
- `loadConfig` configures and probes the solver process before the wallet starts (`config.go:739-758`, `testsolver.go:18-35`).

**What it calls.**

- The `mixclient.Wallet` interface (five methods, section 3.2), implemented by `mixingWallet` in `wallet/mixing.go:77-169`: best block, the wallet's `*mixpool.Pool`, submit-and-broadcast of a mix message, input signing, transaction publication.
- `mixpool.Pool` directly for reads, waits and removals: `Epoch`, `Observer`, `MixPRs`, `CompatiblePRs`, `Receive`, `ReceiveKEsByPairing`, `HaveMessage`, `ExpireMessages`, `RemoveConfirmedSessions`, `RemoveUnresponsiveDuringEpoch`, `RemoveSession`; and `Observer.CheckPrevEpoch`, `Observer.ExcludePRs`. The client never calls `AcceptMessage` itself; all publishing goes through `Wallet.SubmitMixMessage`.
- `solverrpc.StartSolver` and `solverrpc.Roots` (external process).
- The caller-supplied `GenFunc` to obtain fresh HASH160 mixed-output messages (dcrwallet: next addresses of the mixed account/branch, `wallet/mixing.go:51-75`).
- Cryptographic primitives: BLAKE-256, EC-Schnorr-DCRv0 over secp256k1, secp256k1 ECDH, sntrup4591761, ChaCha20, SHA-512 (inside sntrup), the txscript engine (signature validation of the merged coinjoin), `txsort`.

**Network-visible products.** Eight wire message types (`mixpairreq`, `mixkeyxchg`, `mixcphrtxt`, `mixslotres`, `mixfactpoly`, `mixdcnet`, `mixconfirm`, `mixsecrets`) and one regular transaction. Everything a remote peer or a dcrd mixpool validates about those is specified here.

---

## 3. Public surface inventory

Notation: `id` = 33-byte compressed secp256k1 public key; `Hash` = 32-byte BLAKE-256 digest; `F` = 2^127 − 1.

### 3.1 Package `mixing` (root)

| Item | Signature | Contract |
|---|---|---|
| `MinPeers` | `const = 4` | Minimum peers for a run. Declared not API-stable (`client.go:7-9`). |
| `MaxPeers` | `const = 64` | Maximum peers in a session; also bounds referenced previous messages of non-KE messages (`limits.go:22-26`). |
| `MaxMcount` | `const = 16` | Maximum mixed messages one peer contributes (`limits.go:28-30`). |
| `MaxMtot` | `const = 1024` | Maximum mixed messages in a session (`limits.go:32-34`). |
| `MaxMixTxSerializeSize` | `const = 100000` | Maximum mix transaction size (`limits.go:36-38`). |
| `MaxMixAmount` | `const int64 = 21000000_00000000 / MinPeers` = 525 000 000 000 000 atoms | Maximum mixed output value (`limits.go:40-41`). |
| `Msize` | `const = 20` | Size of a mixed message (HASH160) (`vec.go:14-16`). |
| `PRFlagCanSolveRoots` | `const byte = 1` | Bit 0 of the PR `Flags` byte (`flags.go:7-12`). |
| `ScriptClass`, `ScriptClassP2PKHv0` | `type string`; `"P2PKH-secp256k1-v0"` | Only script class in use (`scriptclass.go:10-16`). |
| `F` | `var *big.Int` = `0x7fffffffffffffffffffffffffffffff` | Field prime 2^127 − 1 (`field.go:11-16`). |
| `InField` | `func(x *big.Int) bool` | `0 <= x < F` (`field.go:19-21`). |
| `Vec` | `type [][20]byte` | Vector of messages. Methods: `Equals(other) bool` (same length and data), `String()` (`"[" + hex elements separated by one space + "]"`), `Xor(src1, src2)` (element-wise XOR into receiver; panics `"dcnet: vectors do not share identical dimensions"` on length mismatch) (`vec.go:19-68`). |
| `XorVectors` | `func(vs []Vec) Vec` | XOR of all vectors; result length `len(vs[0])`; panics on empty input or mismatched lengths (`vec.go:72-78`). |
| `randVec` (unexported) | `func(n uint32, prng) Vec` | `n` consecutive 20-byte reads from the PRNG (`vec.go:21-27`). |
| `SRMixPads` | `func(kp [][]byte, my uint32) []*big.Int` | Exponential DC-net pads, section 5.9.2 (`dcnet.go:17-43`). |
| `SRMix` | `func(m *big.Int, pads []*big.Int) []*big.Int` | `mix[i] = m^(i+1) + pads[i] mod F` (`dcnet.go:48-57`). |
| `IntVectorsFromBytes` | `func([][][]byte) [][]*big.Int` | Big-endian unsigned parse of every element, no range check (`dcnet.go:61-70`). |
| `IntVectorsToBytes` | `func([][]*big.Int) [][][]byte` | Minimal big-endian bytes of the absolute value; zero is the empty slice (`dcnet.go:74-83`). |
| `AddVectors` | `func(vs ...[]*big.Int) []*big.Int` | Column sums mod F. Output length is `len(vs)` (the number of vectors), and column `i` reads `vs[j][i]` for every `j` (`dcnet.go:88-98`). |
| `Coefficients` | `func(S []*big.Int) []*big.Int` | Power sums → polynomial coefficients, `len(S)+1` outputs, section 5.9.3 (`dcnet.go:116-133`). |
| `IsRoot` | `func(m *big.Int, a []*big.Int) bool` | `Σ a[i]·m^i ≡ 0 (mod F)` (`dcnet.go:137-147`). |
| `DCMixPads` | `func(kp []wire.MixVect, my uint32) Vec` | XOR of every `kp[i]`, `i != my`; result length `len(kp)` (`dcnet.go:151-160`). |
| `DCMix` | `func(pads Vec, m []byte, slot uint32) Vec` | Copy of `pads` with `m` XORed into element `slot`; panics `"m is not len Msize"` if `len(m) != 20` (`dcnet.go:165-177`). |
| `PQPublicKey`, `PQPrivateKey`, `PQCiphertext`, `PQSharedKey` | `[1218]byte`, `[1600]byte`, `[1047]byte`, `[32]byte` | Aliases of the sntrup4591761 types (`keyagreement.go:22-27`). |
| `KX` | struct `{ECDHPublicKey, ECDHPrivateKey, PQPublicKey *[1218]byte, PQPrivateKey *[1600]byte, PQCleartexts [][32]byte}` | One peer's key-exchange keys (`keyagreement.go:46-52`). |
| `NewKX` | `func(csprng io.Reader) (*KX, error)` | secp256k1 key then sntrup key pair, both from the reader, in that order (`keyagreement.go:56-74`). |
| `(*KX).Encapsulate` | `func(prng io.Reader, pubkeys []*PQPublicKey, my int) ([]PQCiphertext, error)` | One sntrup encapsulation per public key in slice order (own key included); fills `PQCleartexts`. `my` is unused (`keyagreement.go:96-110`). |
| `RevealedKeys` | struct `{ECDHPublicKeys []*secp256k1.PublicKey, Ciphertexts []PQCiphertext, MyIndex uint32}` | Every peer's ECDH key and the ciphertext each peer created for `MyIndex` (`keyagreement.go:114-118`). |
| `SharedSecrets` | struct `{SRSecrets [][][]byte, DCSecrets [][]wire.MixVect}` | Indexed `[own message][global message]` (`keyagreement.go:122-125`). |
| `(*KX).SharedSecrets` | `func(k *RevealedKeys, sid []byte, run uint32, mcounts []uint32) (SharedSecrets, error)` | Section 5.9.1. `run` is unused. Errors: count mismatches (`fmt.Errorf`), `*DecapsulateError` (`keyagreement.go:130-237`). |
| `DecapsulateError` | struct `{SubmittingIndex uint32}`; `Error()` = `"decapsulate failure of ciphertext by peer %d"` | (`errors.go:20-28`). |
| `errInvalidPROrder`, `errInvalidSessionID` (unexported) | `"invalid pair request order"`, `"invalid session ID"` | Returned by `ValidateSession` (`errors.go:12-16`). |
| `MaxExpiry` | `func(tipHeight uint32, params) uint32` | `tipHeight + uint32(60 min / TargetTimePerBlock) + 1` (mainnet tip+13, testnet3 tip+31, simnet/regnet tip+3601) (`expiry.go:15-18`). dcrwallet uses `MaxExpiry − 2` for its PRs (`wallet/mixing.go:171-174`). |
| `Message` | interface: `wire.Message` + `Pub() []byte`, `Sig() []byte`, `WriteHash(hash.Hash)`, `Hash() Hash`, `WriteSignedData(hash.Hash)`, `PrevMsgs() []Hash`, `Sid() []byte`, `GetRun() uint32` | Implemented by all eight wire mix messages (`message.go:23-41`). |
| `Signed` | interface: `Pub`, `Sig`, `Sid`, `GetRun`, `Command() string`, `WriteSignedData` | Input of the signing functions (`signatures.go:20-27`). |
| `SignMessage` | `func(m Signed, priv) error` | Computes the signature and copies it into `m.Sig()` (`signatures.go:31-39`). |
| `VerifySignedMessage` | `func(m Signed) bool` | Section 4.3 (`signatures.go:43-58`). |
| `VerifySignature` | `func(pub, sig, sigHash []byte, command string, sid []byte, run uint32) bool` | Verification from a precomputed signed-data hash (`signatures.go:65-68`). |
| `SortPRsForSession` | `func(prs []*wire.MsgMixPairReq, epoch uint64) [32]byte` | In-place sort into session order; returns the session ID. Section 4.5 (`sid.go:38-61`). |
| `ValidateSession` | `func(ke *wire.MsgMixKeyExchange) error` | Checks `SeenPRs` order for the sid, and for run 0 that the sid derives from them (`sid.go:93-122`). Used by mixpool on received KEs. |
| `deriveSessionID` (unexported) | `func(seenPRs []Hash, epoch uint64) [32]byte` | Section 4.5 (`sid.go:19-33`). |

### 3.2 Package `mixclient`

Exported:

| Item | Signature | Contract |
|---|---|---|
| `MinPeers` | `const = mixing.MinPeers` | (`client.go:39`). |
| `Wallet` | interface | `BestBlock() (uint32, Hash)`; `Mixpool() *mixpool.Pool`; `SubmitMixMessage(ctx, mixing.Message) error` (accept into the local mixpool, then broadcast); `SignInput(tx *wire.MsgTx, index int, prevScript []byte) error` (sets `tx.TxIn[index].SignatureScript`); `PublishTransaction(ctx, tx) error` (`client.go:126-146`). |
| `Client` | struct (opaque) | One per wallet. |
| `NewClient` | `func(w Wallet) *Client` | Starts the solver (once per process), reads best height, selects network timing from the mixpool epoch. Does not start goroutines (`client.go:414-454`). |
| `(*Client).SetLogger` | `func(l slog.Logger)` | Not synchronized; must precede `Run` (`client.go:456-458`). |
| `(*Client).Run` | `func(ctx) error` | Runs the epoch ticker and worker pool; blocks until shutdown completes; returns `ErrStopping` after a clean stop (section 7) (`client.go:486-522`). |
| `(*Client).Dicemix` | `func(ctx, cj *CoinJoin) error` | Section 5.2. Returns nil only after the coinjoin containing `cj`'s outputs was published (`client.go:1006-1092`). |
| `(*Client).ExpireMessages` | `func(height uint32)` | Records `height`; expiry is applied at the next epoch tick (`client.go:1097-1106`). |
| `CoinJoin` | struct (opaque) | Per-request coinjoin state (section 5.8). |
| `GenFunc` | `func(count uint32) (wire.MixVect, error)` | Returns `count` fresh HASH160s (`coinjoin.go:62`). |
| `NewCoinJoin` | `func(gen GenFunc, change *wire.TxOut, mixValue int64, prExpiry uint32, mcount uint32) *CoinJoin` | Empty transaction from `wire.NewMsgTx()`: version 1, lock time 0, expiry 0 (`coinjoin.go:66-78`). |
| `(*CoinJoin).AddInput` | `func(input *wire.TxIn, prevScript []byte, prevScriptVersion uint16, privKey) error` | Appends a PR UTXO with ownership proof; records `prevScript`; adds `input.ValueIn` to the input value. `prevScriptVersion` is unused. Does not retain the key (`coinjoin.go:86-117`). |
| `(*CoinJoin).Tx` | `func() *wire.MsgTx` | The (shared, mutable) coinjoin transaction (`coinjoin.go:288-290`). |
| `(*CoinJoin).MixedIndices` | `func() []int` | Output indexes of this request's mixed outputs, in `gen` order, valid after a successful confirm (`coinjoin.go:293-295`). |
| `ErrTooFewPeers` | `"not enough peers required to mix"` | Carried inside an alternate-session result (`errors.go:10`). |
| `ErrUnknownPRs` | `"unable to participate in reformed session referencing unknown PRs"` | Likewise (`errors.go:12`). |
| `ErrStopping` | `"mixing client is stopping"` | Returned by `Run`, and sent to pending `Dicemix` callers at shutdown (`errors.go:14`). |

Unexported items with observable behaviour:

| Item | Value / signature | Contract |
|---|---|---|
| `pairingVersion`, `pairingVersionTestnet` | `byte 3`, `byte 4` | PR `PairingFlags` (`client.go:41-42`). |
| `timeoutDuration`, `maxJitter`, `msgJitter`, `peerJitter` | 30 s, 3 s, 300 ms, 2.7 s | Mainnet (and every non-testnet3 network) timing (`client.go:44-48`). |
| `timeoutDurationTestnet`, `maxJitterTestnet`, `peerJitterTestnet` | 10 s, 1 s, 700 ms | Testnet3 timing (`client.go:50-52`). |
| `expiredPRErr` | `"mixing pair request %v by %x expired"` (PR hash, identity) | Result sent to a pending `Dicemix` whose PR left the mixpool (`client.go:57-59`). |
| `errOnlyKEsBroadcasted` | `"session ended without mix occurring"` | Internal run result (`client.go:62`). |
| `errTriggeredBlame` | `"blame required"` | Internal run result; a local peer detected misbehaviour (`client.go:63`). |
| `errNoActiveLocalPeers` | `"no active local peers"` | Internal (`client.go:64`). |
| `errIncompletePairing` | `"incomplete pairing"` | Internal (`client.go:1412`). |
| `errBlameFailed` | `"blame failed"` | Internal (`blame.go:24`). |
| `errMissingGen` | `"coinjoin is missing gen output"` | Internal; triggers blame (`coinjoin.go:26`). |
| `errSignedWrongTx` | `"peer signed incorrect mix transaction"` | Internal; peer excluded (`coinjoin.go:31`). |
| `errExceedsStandardSize` | `"tx size would exceed standardness rules"` | Internal size-limit result (`limits.go:14`). |
| `testPeerBlamedError` | `Error()` = `"peer removed during blame assignment"` | Sent to a *local* peer that was blamed (`errors.go:20-26`, `client.go:2559-2562`). |
| `blamedIdentities` | `[]identity`; `Error()` = `"blamed peers [" + hex ids separated by space + "]"` | Internal error type carrying excluded peers (`blame.go:34-51`). |
| `sizeLimited` | `Error()` = `"session exceeds mixing limits"` | Internal (`client.go:2342-2350`). |
| `alternateSession` | `Error()` = `"recreated alternate session"`; `Unwrap()` returns its `err` | Internal (`client.go:2352-2364`). |
| `deadlines` | six `time.Time` fields | Section 5.5 (`client.go:148-169`). |
| `peer`, `peerRunState`, `pairedSessions`, `pendingPairing`, `sessionRun` | structs | Section 7. |
| `blameTimedOut` | `func(*sessionRun, int) error` | Peers lacking the stage's message (`client.go:85-117`). |
| `constTimeSlotSearch` | `func(secret *big.Int, roots []*big.Int) int` | Index of `secret` among `roots` comparing 16-byte left-padded encodings in constant time; −1 if absent; the last match wins (`client.go:2587-2606`). |
| `constantTimeOutputSearch` | `func(tx, value int64, scriptVer uint16, scripts [][]byte) ([]int, error)` | Section 5.8.4 (`coinjoin.go:213-243`). |
| `estimateInputSize`, `estimateOutputSize`, `estimateP2PKHv0SerializeSize`, `coinjoinSize.join` | | Section 5.6.4 (`limits.go:25-95`). `coinjoinSize` is `{currentInputs int; currentOutputScriptSizes []int}`; `join(contributedInputs, mcount, change)` is transactional: on `errExceedsStandardSize` both fields keep their previous values. `estimatedRedeemP2PKHv0InputSize` is the package variable holding `estimateInputSize(108)` = 166 (`limits.go:21`). |
| `mixing.F` initialization | package `init` | `F` is assigned in `init()` from the hex literal `7fffffffffffffffffffffffffffffff`; there is no error path (`dcrd/mixing@v0.7.5/field.go:14-16`). |
| `hasSRMixDimensions` | `func(mix [][][]byte, mcount, mtot uint32) bool` | Outer length `mcount`, every inner length `mtot` (`client.go:1489-1499`). |
| test hooks | `hookBeforeRun`, `hookBeforePeerCTPublish`, `hookBeforePeerSRPublish`, `hookBeforePeerDCPublish`; `testWaiting`, `testTickC` channels | Test-only seams: replace the epoch timer with injected ticks, skip `prDelay`, and let a test mutate a peer's message before it is signed (`testhooks.go:7-16`, `client.go:408-410,702-723,736-739,767-771`). A port needs equivalent seams to reuse the upstream test design. |
| `identity` | `type identity = [33]byte` | Alias for a compressed secp256k1 key, used as map key everywhere (`coinjoin.go:35`). |
| `msize` | `const = 20` | Length a mixed message must have for `addMixedMessage` to add an output; other lengths are silently ignored (`coinjoin.go:20,174-177`). |
| `fieldLen` | `var uint = len(F.Bytes())` = 16 | Padding width of `constTimeSlotSearch` (`client.go:2583`). |
| stage-timeout selectors | `ctTimeout = 1`, `srTimeout = 2`, `dcTimeout = 3`, `cmTimeout = 4` | Argument of `blameTimedOut`; any other value blames nobody and returns `errBlameFailed` (`client.go:67-72,85-117`). |
| message mask bits | `msgKE = 1`, `msgCT = 2`, `msgSR = 4`, `msgDC = 8`, `msgFP = 16`, `msgCM = 32`, `msgRS = 64` | Argument of `sendLocalPeerMsgs`. The bit order (DC before FP) differs from the send order (FP before DC), which is fixed by the code of section 5.13 (`client.go:74-83,608-635`). |
| `hook`, `hookFunc` | `type hook string`; `func(*Client, *pairedSessions, *sessionRun, *peer)`; names `"before run"`, `"before CT publish"`, `"before SR publish"`, `"before DC publish"` | Keys and callbacks of `Client.testHooks` (`testhooks.go:7-16`). `hookBeforeRun` fires at the top of every `pairSession` loop iteration with the newest run and a nil peer (`client.go:1267`); the three publish hooks fire inside the per-peer worker closure immediately after the CT/SR/DC message object is stored on the peer and before it is signed (`client.go:1767,1854,1953`). There is no hook for KE, FP, CM or RS. |
| `(*Client).testTick` | `func(t time.Time)` | Blocking send of an injected epoch time on `testTickC` (`client.go:763-765`). |
| `(*Client).testHook` | `func(stage hook, ps, s, p)` | Calls the registered hook, if any (`client.go:767-771`). |
| `(*Client).log`, `logf`, `logerrf`; `(*sessionRun).logf`, `logerrf` | | Debug/error logging helpers. Each is a no-op when the logger is nil, so a client on which `SetLogger` was never called logs nothing. The `sessionRun` forms prepend `sid=%x/%d ` (`client.go:351-365,460-482`). |
| `(*Client).stop` | `func()` | Compare-and-swap of `atomicStopping` 0→1, then closes `stopping`; idempotent (`client.go:526-530`). |
| `(*Client).peerWorker`, `forLocalPeers`, `queueWork` | | Worker pool, section 7.3 (`client.go:367-371,532-578`). |
| `delayedMsg` | struct `{sendTime, deadline time.Time; m mixing.Message; p *peer}` | Queue element of `sendLocalPeerMsgs` (`client.go:580-585`). |
| `(*peer).isRemoteOrCanceled` | `func() bool` | True for a remote placeholder or when the `Dicemix` caller's context is done (non-blocking read of `done`) (`client.go:233-244`). |
| `(*peer).sendRes` | `func(err error)` | Non-blocking send on the capacity-1 result channel; a second result is dropped (`client.go:247-254`). |
| `(*peer).cloneLocalPeer` | `func(freshGen bool) *peer` | Section 7.2. Panics `"cloneLocalPeer: remote peer"` on a remote placeholder (`client.go:260-277`). |
| `newRemotePeer` | `func(pr) *peer` | `{id: &pr.Identity, pr, remote: true}`; no client, result channel, keys or `CoinJoin` (`client.go:279-285`). |
| `generateSecp256k1` (mixclient) | `func() (pub, priv, error)` | Identity key from the system CSPRNG (`client.go:287-296`). The root package has a same-named function taking a reader, which substitutes the system CSPRNG for a nil reader (`keyagreement.go:29-42`). |
| `(*peer).msgJitter` | `func() time.Duration` | `p.jitter + U[0, 300 ms)`; a fresh random term on every call (`client.go:727-729`). |
| `(*peer).signAndHash`, `submit`, `signAndSubmit` | | Sign (section 4.3), compute the message hash under the shared hasher mutex, and call `Wallet.SubmitMixMessage` with a fresh background context carrying only the given deadline. `signAndSubmit(nil)` is a no-op (`client.go:773-801`). |
| `(*peer).genDicemixKeys` | `func() error` | Clears the peer's KE, then section 5.2 step 7 (`client.go:803-836`). |
| `(*Client).newPendingPairing`, `newPairedSessions` | | Constructors; `newPairedSessions` substitutes an empty map for a nil peer map and leaves `runs` nil (`client.go:838-855`). |
| `(*Client).removeUnresponsiveDuringEpoch`, `epochTicker`, `waitForEpoch`, `prDelay`, `expireMessages`, `donePairing`, `pairSession`, `completePairing`, `run`, `solvesRoots`, `roots`, `validateMergedCoinjoin`, `alternateSession`, `blame`; `excludeBlamed` | | Sections 5.3–5.12. `solvesRoots` is the atomic test of bit 0 of `atomicPRFlags` (`client.go:2148-2151`). |
| `CoinJoin` fields | `txHash`, `genFunc`, `tx`, `change`, `prevScripts`, `peerPRs`, `contributed`, `prUTXOs`, `myInputs`, `gens`, `genScripts`, `mixedIndices`, `mixValue`, `inputValue`, `prExpiry`, `mcount` | (`coinjoin.go:39-59`). `peerPRs` (identity → PR, refilled by `resetUnmixed`) and `gens` (the last generated messages) are written but never read. |
| `(*CoinJoin).resetUnmixed`, `gen`, `addMixedMessage`, `sort`, `confirm`, `mergeSignatures` | | Section 5.8 (`coinjoin.go:121-285`). |
| root package helpers | `xor(a, b []byte)` (in-place `a ^= b` over `len(a)`), `sortPRs` (`sort.Interface` that swaps the PRs and their XORed hashes together), `sign`, `verify`, `schnorrHash`, `zeroSID`, `tag`, `(*KX).ecdhSharedKey`, `(*KX).pqSharedKey` | Implement sections 4.3, 4.5 and 4.9 (`sid.go:63-87`, `signatures.go:17,70-123`, `keyagreement.go:76-88`). `pqSharedKey` turns a failed decapsulation into the error text `sntrup4591761: decapsulate failure`, which `SharedSecrets` replaces with `DecapsulateError`. |
| `utxoproof` constants | `tag = "mixpr-utxoproof"`, `secp256k1P2PKH = "P2PKH(EC-Schnorr-DCRv0)"`, `sep = ","` | (`utxoproof/utxoproof.go:17-26`). The strings must not contain a comma. |

### 3.3 Package `utxoproof`

| Item | Signature | Contract |
|---|---|---|
| `Secp256k1KeyPair` | struct `{Pub []byte, Priv *secp256k1.PrivateKey}` | `Pub` is the 33-byte compressed key (`utxoproof.go:32-35`). |
| `(*Secp256k1KeyPair).SignUtxoProof` | `func(expires uint32) ([]byte, error)` | 64-byte EC-Schnorr-DCRv0 signature, section 4.6 (`utxoproof.go:41-57`). |
| `ValidateSecp256k1P2PKH` | `func(pubkey, proof []byte, expires uint32) bool` | Verification; false on unparsable key or signature (`utxoproof.go:62-82`). |

### 3.4 Package `internal/chacha20prng`

| Item | Signature | Contract |
|---|---|---|
| `SeedSize` | `const = 32` | |
| `New` | `func(seed []byte, run uint32) *Reader` | ChaCha20 (IETF, 32-byte key, 12-byte nonce, block counter starting at 0); nonce = `u32le(run) ‖ 8 zero bytes`; panics `"chacha20prng: bad seed length N"` otherwise (`prng.go:25-35`). |
| `(*Reader).Read` | `func(b []byte) (int, error)` | Fills `b` with the next `len(b)` keystream bytes; never fails (`prng.go:38-46`). |
| `(*Reader).Next` | `func(n int) []byte` | Next `n` keystream bytes; shares the stream position with `Read` (`prng.go:49-53`). |

### 3.5 Package `cspp/v2/solverrpc` and the `csppsolver` program

| Item | Signature | Contract |
|---|---|---|
| `SolverProcess` | `var string = "csppsolver"` | Executable name or path; resolved through `PATH` when it has no path separator; must be set before the first `StartSolver` (`solverrpc/rpc.go:15`). |
| `ErrSolverProcessExited` | `"solver process exited"` | The child was started and has since exited (`solverrpc/rpc.go:19`). |
| `StartSolver` | `func() error` | Starts the child at most once per process (`sync.Once`); the first outcome is returned forever after (`solverrpc/rpc.go:94-97`). |
| `Roots` | `func(a []*big.Int, F *big.Int) ([]*big.Int, error)` | RPC `Solver.Roots`; error `"repeated roots"` (with method `RepeatedRoot()`) when any factor has exponent ≠ 1 (`solverrpc/rpc.go:138-168`). |
| `RootFactors` | `func(a, F) ([]*big.Int, []int, error)` | RPC `Solver.RootFactors`; not used by the client (`solverrpc/rpc.go:102-129`). |
| `csppsolver` | program | Serves `net/rpc` on stdin/stdout; ignores SIGINT, SIGTERM, SIGHUP; exits when stdin closes (`cmd/csppsolver/solver.go:76-90`). |
| `solverProcess` (unexported) | struct `{cmd, stdin, stdout}` with `Read` (child stdout), `Write` (child stdin), `Close` | The `io.ReadWriteCloser` handed to `rpc.NewClient` (`solverrpc/rpc.go:21-46`); `Close` is described in section 5.11.2. |
| `once`, `onceErr`, `client`, `cmdDone` (unexported package variables) | `sync.Once`, `error`, `*rpc.Client`, `chan struct{}` | Process-wide solver state: start guard, recorded start outcome, the single RPC client, and a channel closed by the exit watcher when the child terminates (`solverrpc/rpc.go:48-53`). |
| `startSolver` (unexported) | `func()` | `exec.Command(SolverProcess)` with no arguments, stdin pipe, stdout pipe, `Start`; any of the three failing is recorded in `onceErr` and nothing else is set up. On success creates the RPC client and launches the exit watcher (`cmd.Wait()`, then close `cmdDone`). Stderr is left unset (discarded); environment and working directory are inherited (`solverrpc/rpc.go:55-81`). |
| `checkExit` (unexported) | `func() error` | Non-blocking test of `cmdDone`: `ErrSolverProcessExited` when closed, otherwise nil (`solverrpc/rpc.go:83-90`). |
| `repeatedRoot` (unexported, in `solverrpc` and again in `solver`) | `type repeatedRoot big.Int`; `Error()` = `repeated roots`; `RepeatedRoot() *big.Int` | Error value returned by `Roots` when the reply carries a repeated root (`solverrpc/rpc.go:131-134`, `solver/solver.go:105-108`). `cmd/csppsolver` declares an interface of the same name with the one method `RepeatedRoot() *big.Int`; it is unused (`cmd/csppsolver/solver.go:55-57`). |
| `csppsolver` types | `stdio` (`Read` = `os.Stdin.Read`, `Write` = `os.Stdout.Write`, `Close` closes stdin then stdout and returns the first error); `Solver` (RPC receiver, methods `RootFactors(Args, *Result) error` and `Roots(Args, *Result) error`); `Args{A []*big.Int; F *big.Int}`; `Result{Roots []*big.Int; Exponents []int; RepeatedRoot *big.Int}` | (`cmd/csppsolver/solver.go:13-74`). `Solver.Roots` fills `Roots` **and** `Exponents` on success (the client's reply struct has no `Exponents` field and gob drops it), or only `RepeatedRoot` (the first factor in FLINT order whose exponent is not 1) with a nil RPC error. |
| `solver` package internals | `const base = 16`; `factorExp(fac, i)`, `factorPoly(fac, i)` (pointer arithmetic into FLINT's `fmpz_mod_poly_factor_struct`); cgo link line `-lflint -lmpfr -lgmp -lm` with include/library paths under `/usr/local` | (`solver/solver.go:3-36`). Relevant only if the FLINT-based server is rebuilt; no behaviour beyond section 5.11.2. |

### 3.6 Package `sntrup4591761`

| Item | Signature | Contract |
|---|---|---|
| `PublicKeySize`, `PrivateKeySize`, `CiphertextSize`, `SharedKeySize` | 1218, 1600, 1047, 32 | (`sntrup4591761.go:22-27`). |
| `GenerateKey` | `func(s io.Reader) (*PublicKey, *PrivateKey, error)` | Section 5.10.4. |
| `Encapsulate` | `func(s io.Reader, pk *PublicKey) (*Ciphertext, *SharedKey, error)` | Section 5.10.5. |
| `Decapsulate` | `func(c *Ciphertext, sk *PrivateKey) (*SharedKey, int)` | Second result is 1 on success, 0 on failure. Section 5.10.6. |
| `PrivateKey`, `PublicKey`, `Ciphertext`, `SharedKey` | aliases of `[1600]byte`, `[1218]byte`, `[1047]byte`, `[32]byte` | (`sntrup4591761.go:29-34`). |
| `deriveKey` (unexported) | `func(f, g, gr *[761]int8) (*PublicKey, *PrivateKey)` | Deterministic half of `GenerateKey` (steps 3–4 of section 5.10.4); the entry point the upstream Sage test drives (`sntrup4591761.go:37-52`). |
| `createCipher` (unexported) | `func(r *[761]int8, pk *PublicKey) (*Ciphertext, *SharedKey)` | Deterministic half of `Encapsulate` (section 5.10.5 after `RandomTSmall`) (`sntrup4591761.go:82-103`). |
| `zx.RandomSmall`, `zx.RandomTSmall`; unexported `random32`, `minmax`, `sort` | `func(*[761]int8, io.Reader) error` | Section 5.10.2. Every 4-byte read is `io.ReadFull`; a reader error (short read included) is returned unchanged by `RandomSmall`/`RandomTSmall`, and from there by `GenerateKey`, `Encapsulate`, `NewKX` and `KX.Encapsulate`. The ChaCha20 reader never fails, so these paths are reachable only with another reader (`zx/random.go:15-98`). |
| `zx.Encode`, `zx.Decode` | `func(*[761]int8) *[191]byte`, `func([]byte) *[761]int8` | Section 5.10.3. |
| `rq.Encode`, `rq.Decode`, `rq.EncodeRounded`, `rq.DecodeRounded` | | Section 5.10.3. |
| `rq.Reciprocal3`, `rq.Round3`, `rq.Mult`; `r3.Reciprocal`, `r3.Mult` | | Section 5.10.4. Both `rq` and `r3` carry private helpers `swapInt(x, y *int, mask int)` (conditional swap, mask −1 or 0) and `smallerMask(x, y int) int` = `(x − y) >> 31` (−1 when `y > x`, else 0; the operands are small non-negative loop counters, so the result is the same for 32- and 64-bit `int`) (`r3/r3.go:16-27`, `rq/rq.go:16-27`). |
| `mod3` scalar helpers | `Freeze(int32) int8`; `Product(a, b) = a·b`; `Reciprocal(a) = a`; `Quotient(a, b) = a·b`; `MinusProduct(a, b, c) = Freeze(a − b·c)`; `PlusProduct(a, b, c) = Freeze(a + b·c)`; `Sum(a, b) = Freeze(a + b)`; `MaskSet(x) = −x·x` as `int` (−1 for ±1, 0 for 0) | (`r3/mod3/mod3.go:10-50`). `Product` and `Quotient` do not reduce; their operands are in {−1, 0, 1}. |
| `modq` scalar helpers | `Freeze(int32) int16`; `Product(a, b) = Freeze(a·b)`; `Square(a) = Freeze(a·a)`; `Reciprocal(a) = a^4589` by the fixed chain a2, a3, a4, a8, a16, a32, a35, a70, a140, a143, a286, a572, a1144, a1147, a2294, a4588, a4589; `Quotient(a, b) = Product(a, Reciprocal(b))`; `MinusProduct`, `PlusProduct`, `Sum` as for `mod3` with `Freeze` of `modq`; `MaskSet(x) = (−int32(uint16(x))) >> 30` (−1 for non-zero, 0 for zero) | (`rq/modq/modq.go:10-77`). All products are formed in 32-bit signed arithmetic before `Freeze`. |
| `r3/vector`, `rq/vector` | `Swap(x, y, n, mask)` (element-wise conditional swap of the first `n` elements; the mask is truncated to the element width), `Product(z, n, x, c)` (`z[i] = Product(x[i], c)` of the scalar package: unreduced in `r3`, frozen in `rq`), `MinusProduct(z, n, x, y, c)` (`z[i] = Freeze(x[i] − y[i]·c)`), `Shift(z, n)` (`z[i] = z[i−1]` for `i = n−1 … 1`, `z[0] = 0`) | Building blocks of the two reciprocal routines (`r3/vector/vector.go:15-41`, `rq/vector/vector.go:15-41`). |
| `examples/keygen`, `examples/encap`, `examples/decap` | command-line programs | Not linked by dcrwallet; no behaviour to port. |

### 3.7 dcrwallet solver glue

| Item | Location | Contract |
|---|---|---|
| `--csppsolver` | `config.go:172,378` | `ExplicitString`, default `"csppsolver"`, description `Path to CSPP solver executable (if not in PATH)`. |
| solver start-up checks | `config.go:739-758` | Section 5.11.3. |
| `testStartedSolverWorks` | `testsolver.go:18-35` | Calls `Roots([1, 2], F)` and fails only when the error is `io.ErrUnexpectedEOF` or `rpc.ErrShutdown`. |

### 3.8 Wire messages (`dcrd/wire@v1.8.0`)

Message types `MsgMixPairReq`, `MsgMixKeyExchange`, `MsgMixCiphertexts`, `MsgMixSlotReserve`, `MsgMixFactoredPoly`, `MsgMixDCNet`, `MsgMixConfirm`, `MsgMixSecrets`, with command strings `CmdMixPairReq` … `CmdMixSecrets` = `mixpairreq`, `mixkeyxchg`, `mixcphrtxt`, `mixslotres`, `mixfactpoly`, `mixdcnet`, `mixconfirm`, `mixsecrets` (`dcrd/wire@v1.8.0/message.go:56-63`). All eight are announced with inventory type `InvTypeMix = 4` (`MSG_MIX`, `invvect.go:35,44`). Unexported codec helpers: `writeMixVects`/`readMixVects` (the DC matrix, `msgmixdcnet.go:173-248`), `writeMixVect`/`readMixVect` (the RS message vector, `msgmixsecrets.go:36-100`), and a per-message `writeMessageNoSignature` shared by encoding, hashing and signing; when its writer is a hasher the structural checks of section 8.5 are skipped (except the RS checks, Quirk Q15c, and the PR script-class checks, Quirk Q26).

Constructors `NewMsgMixPairReq`, `NewMsgMixKeyExchange`, `NewMsgMixCiphertexts`, `NewMsgMixSlotReserve`, `NewMsgMixFactoredPoly`, `NewMsgMixDCNet`, `NewMsgMixConfirm`, `NewMsgMixSecrets`; methods `BtcEncode`, `BtcDecode`, `Command`, `MaxPayloadLength`, `SerializeSize`, `Hash`, `WriteHash`, `WriteSignedData`, `Pub`, `Sig`, `PrevMsgs`, `Sid`, `GetRun` on each; `MsgMixPairReq.Pairing`, `MsgMixPairReq.Expires`, `MsgMixSecrets.Commitment`; type `MixVect` (`[][20]byte`); type `MixPairReqUTXO`. Byte layouts are in section 4.2; the constants are in section 6.

Accessor and constructor behaviour that the client and the signing code depend on (identical shape in all eight files; line numbers are those of the PR unless stated):

- `Hash()` returns the stored hash field; it is the zero hash until `WriteHash` has been called, and decoding does not set it (`msgmixpairreq.go:241-248`). `WriteHash(h)` calls `h.Reset()`, writes the 64-byte signature and the body at protocol version 10, and stores `h.Sum` in the message (`msgmixpairreq.go:256-265`). `WriteSignedData(h)` does **not** reset the hasher: it appends `varstring(cmd + "-sig")` and the body to whatever the hasher already holds (`msgmixpairreq.go:387-390`); `mixing.sign` and `VerifySignedMessage` always pass a fresh hasher.
- `Pub()` returns the 33 identity bytes and `Sig()` the 64 signature bytes, both as slices aliasing the message's own arrays; `SignMessage` writes the signature through the `Sig()` slice (`dcrd/mixing@v0.7.5/signatures.go:31-39`).
- `MaxPayloadLength(pver)` returns 0 when `pver < 10`, otherwise the constant of section 6.
- `Command()` returns the command string of the type.

| Message | `PrevMsgs()` | `Sid()` | `GetRun()` | `SerializeSize()` |
|---|---|---|---|---|
| PR | nil | nil | 0 | `109 + varstring(ScriptClass) + 18 + varint(nUTXO) + Σ_UTXO (37 + varbytes(Script) + varbytes(PubKey) + varbytes(Signature) + 1) + 1 + [8 + 2 + varbytes(change script)] + 2` (`msgmixpairreq.go:411-448`) |
| KE | `SeenPRs` | `SessionID[:]` | `Run` | `1428 + varint(n) + 32n` (`msgmixkeyexchange.go:198-203`) |
| CT | `SeenKeyExchanges` | `SessionID[:]` | `Run` | `133 + varint(len(Ciphertexts)) + 1047·len(Ciphertexts) + 32·len(SeenKeyExchanges)` (`msgmixciphertexts.go:203-209`) |
| SR | `SeenCiphertexts` | `SessionID[:]` | `Run` | `133 + varint(mcount) + varint(kpcount) + Σ varbytes(value) + varint(n) + 32n`, with `kpcount = len(DCMix[0])`, or 0 when `DCMix` is empty (`msgmixslotreserve.go:278-302`) |
| FP | `SeenSlotReserves` | `SessionID[:]` | `Run` | `133 + varint(len(Roots)) + Σ varbytes(root) + varint(n) + 32n` (`msgmixfactoredpoly.go:223-236`) |
| DC | `SeenSlotReserves` | `SessionID[:]` | `Run` | `133 + varint(x) + [varint(y) + 1 + 20·Σ len(row)] + varint(n) + 32n`, the bracket only when `x ≠ 0` (`msgmixdcnet.go:274-292`) |
| CM | `SeenDCNets` | `SessionID[:]` | `Run` | `133 + Mix.SerializeSize() + varint(n) + 32n` (`msgmixconfirm.go:194-200`) |
| RS | `SeenSecrets` | `SessionID[:]` | `Run` | `165 + varint(k) + Σ varbytes(SR message) + varint(m) + [1 + 20m] + varint(n) + 32n`, the bracket only when `m ≠ 0` (`msgmixsecrets.go:304-325`) |

`n` is the length of the message's `Seen*` array; 133 = 64 + 33 + 32 + 4.

Constructors store their arguments without copying or validation, except: `NewMsgMixPairReq` validates the script class and the UTXO count (section 8.5) and is the only one that can fail; `NewMsgMixConfirm` substitutes `wire.NewMsgTx()` for a nil transaction and stores the transaction **by value** (a shallow struct copy: the `TxIn`/`TxOut` slices are shared with the argument, which is why the client passes `Tx().Copy()`) (`msgmixconfirm.go:234-248`); `NewMsgMixSecrets` takes no `SeenSecrets` argument and initializes the field to an empty non-nil slice (`msgmixsecrets.go:364-376`). `MsgMixSecrets.Commitment(h)` operates on a shallow copy of the message, so the receiver's signature, `SeenSecrets` and stored hash are left untouched (`msgmixsecrets.go:203-209`). `MsgMixPairReq.Expires()` returns `Expiry`; `MsgMixPairReq.Pairing()` can only fail on a writer error, which a memory buffer never produces (`msgmixpairreq.go:83-108,461-463`). `MixVect.String()` prints `[` + lowercase hex of each 20-byte element separated by one space + `]`, the same format as `mixing.Vec.String()` (`mixvect.go:17-29`).

---

## 4. Data formats

Nothing in scope is persisted. All formats below are transmitted (P2P `mix*` messages, or the same bytes hex-encoded as the second parameter of dcrd's `sendrawmixmessage` JSON-RPC, whose first parameter is the command string, `rpc/client/dcrd/calls.go:243-250`), fed to a hash, or exchanged with the solver process.

### 4.1 Wire encoding conventions

- Fixed-width integers are little-endian (`dcrd/wire@v1.8.0/common.go:42-44`). Byte arrays are written verbatim.
- `varint` is the wire compact size: 1 byte for values < 0xfd; `0xfd` + u16; `0xfe` + u32; `0xff` + u64.
- `varbytes` = `varint(len) ‖ bytes`; `varstring` likewise over the string bytes.
- An outpoint is `hash[32] ‖ index u32 ‖ tree u8` (37 bytes) (`dcrd/wire@v1.8.0/msgtx.go:1201-1218`). A `TxOut` is `value i64 ‖ version u16 ‖ varbytes(pkScript)` (`msgtx.go:1325-1337`).
- Every mix message is `signature[64] ‖ body`. Encoding or decoding with a protocol version below `MixVersion` (10) fails with `ErrMsgInvalidForPVer`. Hashes and signatures are always computed over the encoding at protocol version 10.
- `Run` is always 0 in every message the client produces, and a mixpool rejects any message whose run is non-zero with `"nonzero reruns are unsupported"` (`dcrd/mixing@v0.7.5/mixpool/mixpool.go:1188-1190`). A re-formed session is distinguished by its session ID, never by `Run`.

### 4.2 Message bodies

In each table the offset column is omitted where a preceding field is variable-length.

**`mixpairreq` (PR)** — `dcrd/wire@v1.8.0/msgmixpairreq.go:59-78,273-382`

| Field | Encoding | Client value |
|---|---|---|
| Signature | `[64]` | section 4.3 |
| Identity | `[33]` | fresh compressed secp256k1 key per `Dicemix` call |
| Expiry | u32 | `cj.prExpiry` (block height) |
| MixAmount | i64 | `cj.mixValue`; decode rejects negative |
| ScriptClass | varstring, ≤ 32 bytes, every byte in 0x20..0x7e | `"P2PKH-secp256k1-v0"` (18 bytes) |
| TxVersion | u16 | `cj.tx.Version` = 1 |
| LockTime | u32 | `cj.tx.LockTime` = 0 |
| MessageCount | u32 | `cj.mcount` |
| InputValue | i64 | sum of `ValueIn` of added inputs; decode rejects negative |
| UTXO count | varint ≤ 512 | |
| per UTXO: OutPoint | 37 bytes | previous outpoint |
| per UTXO: Script | varbytes ≤ 16384 | empty (P2SH only; never set) |
| per UTXO: PubKey | varbytes ≤ 33 | 33-byte compressed key controlling the output |
| per UTXO: Signature | varbytes ≤ 64 | 64-byte UTXO proof, section 4.6 |
| per UTXO: Opcode | u8 | 0, or the first byte of the previous script when it is `OP_SSGEN` (0xbb), `OP_SSRTX` (0xbc) or `OP_TGEN` (0xc3) |
| HasChange | u8, 0 or 1 (other values: decode error `"invalid change TxOut encoding"`) | 1 iff a change output was given |
| Change | `TxOut`, present iff HasChange = 1 | |
| Flags | u8 | bit 0 = `PRFlagCanSolveRoots` |
| PairingFlags | u8 | 3 (4 on testnet3) |

**`mixkeyxchg` (KE)** — `msgmixkeyexchange.go:29-45,140-169`

| Offset | Field | Encoding | Client value |
|---|---|---|---|
| 0 | Signature | `[64]` | |
| 64 | Identity | `[33]` | |
| 97 | SessionID | `[32]` | section 4.5 |
| 129 | Epoch | u64 (little-endian) | Unix seconds of the epoch at which the pairing started; unchanged across re-formed sessions of the same epoch |
| 137 | Run | u32 | 0 |
| 141 | Pos | u32 | sender's index in the session peer order |
| 145 | ECDH | `[33]` | compressed secp256k1 ECDH public key |
| 178 | PQPK | `[1218]` | sntrup4591761 public key |
| 1396 | Commitment | `[32]` | section 4.7 |
| 1428 | SeenPRs count | varint ≤ 512 | n |
| | SeenPRs | n × `[32]` | PR hashes in session peer order |

**`mixcphrtxt` (CT)** — `msgmixciphertexts.go:18-32,135-174`

| Field | Encoding | Client value |
|---|---|---|
| Signature, Identity, SessionID, Run | `[64]`, `[33]`, `[32]`, u32 | |
| count | varint ≤ 512; a single count for both arrays (encoding fails if their lengths differ) | n |
| Ciphertexts | count × `[1047]` | element i is the encapsulation to peer i's PQ key (own position included) |
| SeenKeyExchanges | count × `[32]` | KE hashes in session peer order |

**`mixslotres` (SR)** — `msgmixslotreserve.go:30-42,177-249`

| Field | Encoding | Client value |
|---|---|---|
| Signature, Identity, SessionID, Run | as above | |
| mcount | varint, 1..1024 | sender's message count |
| kpcount | varint, 1..512 | mtot (inner dimension of every row) |
| DCMix | mcount × kpcount × `varbytes ≤ 32` | row i, column j = minimal big-endian bytes of `srMix[i][j]` (zero is a zero-length value) |
| SeenCiphertexts count | varint ≤ 512 | n |
| SeenCiphertexts | n × `[32]` | CT hashes in session peer order |

**`mixfactpoly` (FP)** — `msgmixfactoredpoly.go:17-31,144-197`

| Field | Encoding | Client value |
|---|---|---|
| Signature, Identity, SessionID, Run | as above | |
| Roots count | varint ≤ 1024 | mtot |
| Roots | count × `varbytes ≤ 32` | roots sorted ascending numerically, minimal big-endian bytes |
| SeenSlotReserves count | varint ≤ 512 | n |
| SeenSlotReserves | n × `[32]` | SR hashes in session peer order |

**`mixdcnet` (DC)** — `msgmixdcnet.go:16-30,130-248`

| Field | Encoding | Client value |
|---|---|---|
| Signature, Identity, SessionID, Run | as above | |
| x | varint ≤ 1024 | sender's message count |
| y | varint ≤ 1024; present only when x ≠ 0 | mtot |
| msize | varint, must equal 20; present only when x ≠ 0 | 20 |
| DCNet | x × y × `[20]` | row i = DC-net vector of own message i |
| SeenSlotReserves count | varint ≤ 512 | n |
| SeenSlotReserves | n × `[32]` | SR hashes in session peer order |

**`mixconfirm` (CM)** — `msgmixconfirm.go:21-35,133-168`

| Field | Encoding | Client value |
|---|---|---|
| Signature, Identity, SessionID, Run | as above | |
| Mix | a complete `MsgTx` in its own serialization (full: prefix and witness) | copy of the sorted coinjoin with only the sender's inputs signed |
| SeenDCNets count | varint ≤ 512 | n |
| SeenDCNets | n × `[32]` | DC hashes in session peer order |

**`mixsecrets` (RS)** — `msgmixsecrets.go:20-34,233-275`

| Field | Encoding | Client value |
|---|---|---|
| Signature, Identity, SessionID, Run | as above | |
| Seed | `[32]` | run PRNG seed |
| SlotReserveMsgs count | varint ≤ 1024 | own message count |
| SlotReserveMsgs | count × `varbytes ≤ 32` | unpadded SR messages, minimal big-endian bytes |
| DCNetMsgs n | varint ≤ 1024 | own message count |
| DCNetMsgs msize | varint = 20; present only when n ≠ 0 | 20 |
| DCNetMsgs | n × `[20]` | unpadded DC messages (the HASH160s) |
| SeenSecrets count | varint ≤ 512 | 0 for a peer that initiates the reveal; otherwise the RS hashes observed when responding |
| SeenSecrets | count × `[32]` | |

The wire limits above (512 peers, 1024 messages) are the decoder's. The mixing module applies the lower limits of section 6 when accepting messages into a mixpool (`dcrd/mixing@v0.7.5/mixpool/limits.go:14-119`).

### 4.3 Message hash, signed data and signature

For a message with command string `cmd` and body `B` (everything after the signature, encoded at protocol version 10):

1. **Message hash** `= BLAKE-256(signature ‖ B)`. It identifies the message in inventories and in the `Seen*` arrays of the next stage. The Go code stores it in the message when `WriteHash` is called; it is never computed implicitly (`msgmixpairreq.go:256-265`).
2. **Signed-data hash** `sigHash = BLAKE-256(varstring(cmd + "-sig") ‖ B)` (`msgmixpairreq.go:387-390`). The tag strings are `mixpairreq-sig`, `mixkeyxchg-sig`, `mixcphrtxt-sig`, `mixslotres-sig`, `mixfactpoly-sig`, `mixdcnet-sig`, `mixconfirm-sig`, `mixsecrets-sig`.
3. **Schnorr message** `m = BLAKE-256("decred-mix-signature" ‖ "," ‖ cmd ‖ "," ‖ hex(sid) ‖ "," ‖ dec(run) ‖ "," ‖ hex(sigHash))`, where `hex` is lowercase with no prefix, `dec` is the minimal decimal ASCII form, `sid` is the 32-byte session ID and `run` the message's run. When the message has no 32-byte session ID (the PR), `sid` is 32 zero bytes and `run` is 0 (`dcrd/mixing@v0.7.5/signatures.go:72-91,110-123`).
4. **Signature** = EC-Schnorr-DCRv0 over secp256k1 of `m` with the identity private key (deterministic RFC 6979 nonce as implemented by `dcrec/secp256k1/v4/schnorr.Sign`), serialized as `r[32] ‖ s[32]`.

Verification (`signatures.go:93-108`) rejects an identity that is not exactly 33 bytes or does not parse, and a signature that does not parse.

The client signs first and hashes second (`mixclient/client.go:773-784`): the hash covers the signature.

### 4.4 Pairing ID

`Pairing() = MixAmount i64 ‖ varstring(ScriptClass) ‖ TxVersion u16 ‖ LockTime u32 ‖ PairingFlags u8` (`msgmixpairreq.go:83-108`). For the client's PRs this is 34 bytes: `amount[8] ‖ 0x12 ‖ "P2PKH-secp256k1-v0" ‖ 01 00 ‖ 00 00 00 00 ‖ 03` (last byte `04` on testnet3). Two PRs can be mixed together only when their pairing IDs are byte-identical. The pairing ID is a local grouping key; it is never transmitted.

### 4.5 Session ID and peer order

For a set of PRs and an epoch (Unix seconds):

1. Sort the PR hashes ascending by unsigned byte-wise comparison.
2. `sid = BLAKE-256("decred-mix-session" ‖ u64be(epoch) ‖ h_0 ‖ … ‖ h_{n-1})` (`dcrd/mixing@v0.7.5/sid.go:19-33`). The epoch is **big-endian** here, unlike the KE's wire field.
3. The session peer order is the PRs sorted ascending by `hash XOR sid` (byte-wise XOR, unsigned byte-wise comparison) (`sid.go:52-58`). A peer's index in this order is its *unmixed position* (`myVk`, KE `Pos`).

`SortPRsForSession` reads each PR's stored hash (`Hash()`), so every PR must have had `WriteHash` applied first; it reorders the caller's slice in place (the client passes the slice returned by `CompatiblePRs`, the alternate-session list, the size-limited list or the post-blame list). Both sorts are unstable Go sorts; with distinct PR hashes the result is unique. `ValidateSession` tests the XORed `SeenPRs` with a non-strict "is sorted" check (adjacent equal values pass), then, for `Run == 0` only, re-sorts a copy of `SeenPRs` by plain hash and requires `deriveSessionID(sorted, ke.Epoch) == ke.SessionID`; an empty `SeenPRs` passes the order check and fails or passes the derivation check like any other list (`sid.go:38-61,93-122`).

### 4.6 UTXO ownership proof

`proofHash = BLAKE-256("mixpr-utxoproof,P2PKH(EC-Schnorr-DCRv0)," ‖ u32be(expiry) ‖ pubkey[33])`; the proof is the 64-byte EC-Schnorr-DCRv0 signature of `proofHash` by the key that controls the output (`dcrd/mixing@v0.7.5/utxoproof/utxoproof.go:17-57`). `expiry` is the PR's `Expiry`, so a proof is bound to one expiry height. There is no separator after the expiry.

### 4.7 Secrets commitment

`Commitment = message hash of the RS with Signature set to 64 zero bytes and SeenSecrets set to empty` = `BLAKE-256(0^64 ‖ Identity ‖ SessionID ‖ Run ‖ Seed ‖ SlotReserveMsgs… ‖ DCNetMsgs… ‖ varint(0))` (`msgmixsecrets.go:203-209`). It therefore binds the identity, the session ID, the seed and the unpadded SR and DC messages, and is unaffected by the `SeenSecrets` filled in later.

### 4.8 Run PRNG stream

Each local peer owns a 32-byte seed `S` drawn from the system CSPRNG. The run PRNG is the ChaCha20 keystream with key `S`, nonce twelve zero bytes, initial block counter 0 (`mixclient/client.go:806-808`, `internal/chacha20prng/prng.go:25-35`). It is consumed strictly in this order, and every other peer re-derives the same values from the revealed seed during blame assignment:

| Order | Bytes | Use |
|---|---|---|
| 1 | 32 per attempt | ECDH private scalar, big-endian. An attempt is rejected, and the next 32 bytes used, when the value is 0 or ≥ the group order (`dcrec/secp256k1/v4@v4.4.1/privkey.go:52-71`). |
| 2 | 3044 per attempt (761 × 4) | sntrup `g` candidate; repeated until `g` is invertible in R3 (section 5.10.4). |
| 3 | 3044 | sntrup `f`. |
| 4 | 3044 per peer, in session peer order | Encapsulation randomness for the ciphertext to each peer (own position included). |

The unpadded SR messages are deliberately **not** taken from this stream; they come from the system CSPRNG as uniform integers in `[0, F)` (`mixclient/client.go:817-824`).

### 4.9 Pairwise keys, pair PRNG, pads

For local message `(P, i)` (peer position P, own message index i) and remote message `(Q, j)`:

- `K_ecdh = BLAKE-256(X)`, `X` = 32-byte big-endian x-coordinate of `d_P · ECDH_Q` (`keyagreement.go:76-80`).
- `K_in` = sntrup key decapsulated from the ciphertext Q addressed to P; `K_out` = sntrup key P encapsulated to Q.
- `K(P,Q) = K_ecdh ⊕ K_in ⊕ K_out` for `Q ≠ P`; `K(P,P) = K_ecdh ⊕ K_in` (`keyagreement.go:172-191`).
- `pairSeed = BLAKE-256(sid[32] ‖ K[32] ‖ u32le(seedCounter))`, pair PRNG = ChaCha20 with key `pairSeed`, nonce `u32le(nonceCounter) ‖ 0^8`, counter 0 (`keyagreement.go:193-226`).
  - `Q > P`: `seedCounter = i`, `nonceCounter = j`. `Q < P`: `seedCounter = j`, `nonceCounter = i`. (The seed counter is the message index of the lower-positioned peer.)
  - `Q = P` (two messages of the same peer): `seedCounter = max(i, j)`, `nonceCounter = min(i, j)`.
- From the pair PRNG: bytes 0..31 = `srSecret`; the following `20·mtot` bytes = the DC pad vector (`mtot` elements of 20 bytes) (`keyagreement.go:228-229`).
- SR pad contribution for column `c` (0-based): `BLAKE-256(srSecret ‖ u64le(c + 1))` read as a 256-bit big-endian integer (`dcnet.go:24-33`).

### 4.10 Field elements

Field elements are transmitted as the minimal big-endian byte string of the canonical residue in `[0, F)` (Go `big.Int.Bytes()`): at most 16 bytes, no leading zero bytes, and the value 0 is zero-length. Receivers parse up to 32 bytes as an unsigned big-endian integer and reduce; a non-minimal or non-canonical encoding from a remote peer changes that peer's message hash but not the arithmetic.

### 4.11 Coinjoin transaction

- Version 1, lock time 0, expiry 0, full serialization type (`wire.NewMsgTx`, `coinjoin.go:66-78`).
- Inputs: one per PR UTXO, `PreviousOutPoint` from the PR, `Sequence = 0xffffffff`, `ValueIn = −1`, `BlockHeight = 0`, `BlockIndex = 0xffffffff`, empty signature script until signed (`coinjoin.go:133-140`, `dcrd/wire@v1.8.0/msgtx.go:338-347`). The witness value fields keep these null values in the CM messages and in the published transaction.
- Outputs: every PR's change output verbatim (value, version, script), and `mtot` mixed outputs each `value = mixValue`, `version = 0`, `pkScript = 76 a9 14 <20-byte message> 88 ac` (`coinjoin.go:174-188`).
- Order: section 5.8.3.

### 4.12 sntrup4591761 byte layouts

| Object | Size | Layout |
|---|---|---|
| Public key | 1218 | `rq.Encode(h)` |
| Private key | 1600 | `zx.Encode(f)` [0..191) ‖ `zx.Encode(g⁻¹ in R3)` [191..382) ‖ public key [382..1600) |
| Ciphertext | 1047 | confirmation `SHA-512(zx.Encode(r))[0..32)` ‖ `rq.EncodeRounded(c)` [32..1047) |
| Shared key | 32 | `SHA-512(zx.Encode(r))[32..64)` |

The encoders are specified in section 5.10.3.

### 4.13 Solver RPC

Transport: the child's stdin (requests) and stdout (responses), one Go `net/rpc` connection with the default `encoding/gob` codec (`solverrpc/rpc.go:72-76`; Go standard library `net/rpc/client.go` `NewClient`). The child's stderr is not connected. Each call is a gob-encoded header followed by a gob-encoded body on one continuous gob stream per direction:

| Direction | Header (gob struct) | Body (gob struct) |
|---|---|---|
| request | `Request{ServiceMethod string; Seq uint64}` | `{A []*big.Int; F *big.Int}` (anonymous struct on the client; `main.Args` on the server; gob matches by field name) |
| response | `Response{ServiceMethod string; Seq uint64; Error string}` | `{Roots []*big.Int; Exponents []int; RepeatedRoot *big.Int}` (server `main.Result`); the client's reply struct for `Solver.Roots` has only `Roots` and `RepeatedRoot`, for `Solver.RootFactors` only `Roots` and `Exponents` |

`ServiceMethod` is `"Solver.Roots"` or `"Solver.RootFactors"`. `A` holds the polynomial coefficients in ascending degree order (`A[k]` multiplies `x^k`), `F` the modulus. A `big.Int` travels as a gob `GobEncoder` byte string: one byte `(1 << 1) | sign` (i.e. `0x02` for non-negative values) followed by the big-endian magnitude (Go `math/big/intmarsh.go` `GobEncode`). gob omits zero-valued fields, so an absent `RepeatedRoot` decodes as nil. The exact gob type-descriptor bytes were not captured in this study; see Open questions.

---

## 5. Algorithms and state machines

Symbols: `E` = epoch duration; `T` = stage timeout; `n` = number of PRs (peers) in a session run; `mcount_k` = `MessageCount` of the PR at position k; `mtot = Σ mcount_k`; `start_k = Σ_{j<k} mcount_j` (a peer's first global message index). "Peer order" always means the session peer order of section 4.5.

There are no database transactions anywhere in this subsystem. The only wallet database access is indirect: the `GenFunc` (address derivation) and `SignInput` (private key lookup) callbacks.

### 5.1 Client construction and network parameters

`NewClient(w)` (`mixclient/client.go:414-454`):

1. `prFlags = 0`. Call `solverrpc.StartSolver()`; if it returns nil, set bit 0 (`PRFlagCanSolveRoots`).
2. `height, _ = w.BestBlock()`; `epoch = w.Mixpool().Epoch()`.
3. Select the parameter set by the epoch value alone:

| Parameter | `epoch == 3 min` (testnet3) | otherwise (mainnet, simnet, regnet) |
|---|---|---|
| `E` | 3 min | 10 min (`mixpool/mixpool.go:280,289-292`) |
| `stageTimeout` T | 10 s | 30 s |
| `maxJitter` | 1 s | 3 s |
| `peerJitter` | 700 ms | 2.7 s |
| `pairingVersion` | 4 | 3 |

4. Create the `warming` channel (open), the work queue (buffer = number of CPUs), the `stopping` channel, an empty pending-pairings map and one shared BLAKE-256 hasher guarded by its own mutex.

### 5.2 `Dicemix(ctx, cj)` — submitting a request

(`mixclient/client.go:1006-1092`)

1. Wait until `warming` is closed (section 5.4) or `ctx` is done (return `ctx.Err()`).
2. Generate the peer identity: a secp256k1 key pair from the system CSPRNG. `id` = compressed public key.
3. Create the local peer: `jitter` = uniform in `[0, peerJitter)`, fixed for the peer's lifetime; result channel of capacity 1; `done = ctx.Done()`.
4. `prDelay` (section 5.3). On `ctx` cancellation return `ctx.Err()`.
5. Read the current PR flags atomically. Build the PR with `NewMsgMixPairReq(id, cj.prExpiry, cj.mixValue, "P2PKH-secp256k1-v0", cj.tx.Version, cj.tx.LockTime, cj.mcount, cj.inputValue, cj.prUTXOs, cj.change, flags, pairingVersion)`. Constructor errors (script class, more than 512 UTXOs) are returned.
6. Sign and hash the PR (section 4.3). Compute its pairing ID.
7. `genDicemixKeys` (`client.go:803-836`): new 32-byte seed from the system CSPRNG; run PRNG; `NewKX(prng)`; `mcount` SR messages, each uniform in `[0, F)` from the system CSPRNG; `dcMsg = cj.gen()` (calls the `GenFunc` with `cj.mcount`, and records the 25-byte P2PKH scripts of the results in the `CoinJoin`); error `"gen returned wrong message count"` if the generator returned a different number of messages. Errors are returned.
8. Log (debug) `Created local peer id=%x PR=%s`.
9. Under `c.mu`: add the peer to `pendingPairings[pairingID].localPeers[id]`, creating the pairing entry if absent.
10. `SubmitMixMessage(background context with deadline now + T, pr)`. On error: under `c.mu` remove the peer again (and the pairing entry if now empty) and return the error.
11. Block until a result arrives on the peer's result channel (return it; nil means the coinjoin was published) or `ctx` is done (return `ctx.Err()`).

A cancelled request is not removed from the client's maps at step 11. Its peer is skipped by every later local-peer operation (so it stops sending messages and other participants see it time out) and is garbage-collected when its PR leaves the mixpool.

The identity key, PR, PR expiry and UTXO proofs are fixed at steps 2–6 and reused for every epoch in which the request stays pending. A PR is never re-issued by the client; when it expires the caller receives `expiredPRErr` and must start over.

### 5.3 PR submission timing (`prDelay`)

(`client.go:735-761`; skipped entirely when the test tick channel is set)

1. `now` = current UTC time; `epoch = truncate(now, E) + E` (the next epoch boundary; section 5.4).
2. `sendBefore = epoch − T − maxJitter`; `sendAfter = epoch + T`.
3. `wait = 0`. If `now ≥ sendBefore`: `wait = sendAfter − now` and `sendBefore += E`.
4. `wait += peer.jitter + U[0, 300 ms) + U[0, sendBefore − now)` where each `U[0, x)` is an independent uniform duration from the system CSPRNG (nanosecond resolution).
5. Sleep `wait`; return `ctx.Err()` if cancelled first.

Effect: a request arriving outside the window `[epoch − T − maxJitter, epoch]` publishes its PR at a uniformly random time before the next window opens; a request arriving inside the window waits until `T` after the boundary and then a further uniform delay (see Quirk Q7 for the resulting overshoot).

### 5.4 Epochs and the epoch ticker

**Epoch boundaries.** `waitForEpoch` computes `epoch = truncate(now_UTC, E) + E` and sleeps until then (`client.go:697-725`). Go's `Truncate` rounds down to a multiple of `E` counted from the zero time (year 1); because the Unix epoch is itself a whole multiple of both 3 and 10 minutes from that origin, this equals `floor(unix_seconds / E_seconds) · E_seconds + E_seconds`. Epoch values are therefore whole multiples of `E` in Unix time, and the value in seconds is what is hashed into the session ID and sent in the KE. A receiving mixpool rejects a KE whose epoch is not a whole minute, is more than 5 s in the future, or more than 20 minutes in the past (`mixpool/mixpool.go:33-34,1669-1695`).

If the `stopping` channel is closed while waiting, `waitForEpoch` waits for all in-flight session goroutines (`runWG`) and returns `ErrStopping`. If its context ends first it returns the computed epoch together with the context error.

**Test mode** (`client.go:702-723,887-904`; active when `testWaiting` is non-nil, which only the upstream tests arrange): `waitForEpoch` starts no timer; it first offers one value on `testWaiting` (signalling the test that the ticker is parked), then waits for a time on `testTickC` and returns that time truncated to a multiple of `E` as the epoch. The warm-up wait of ticker step 3 likewise offers on `testWaiting` and then waits for one injected tick instead of sleeping `T + 2 s` (the timer is still armed and also ends the wait). `prDelay` returns immediately when `testTickC` is non-nil. A test therefore drives each epoch with the pair "receive from `testWaiting`, call `testTick(t)`", and the first two ticks are consumed by the first-epoch wait and the warm-up.

**Ticker** (`epochTicker`, `client.go:877-1003`), one goroutine for the life of `Run`:

1. `prevPRs = mixpool.MixPRs()` (snapshot of all unexpired PRs at start).
2. `firstEpoch = waitForEpoch()`.
3. Sleep `T + 2 s`.
4. Under `c.mu`: `removeUnresponsiveDuringEpoch(prevPRs, firstEpoch)`; close `warming`. From here `Dicemix` calls proceed.
5. Loop forever:
   1. `epoch = waitForEpoch()`; on error return it.
   2. Log (debug) `Epoch tick`.
   3. `observer.CheckPrevEpoch(prevEpoch)`; on error return it (this ends the ticker; see section 8.2 for what happens to `Run`). `prevEpoch = epoch`.
   4. `pairingWG.Wait()`: wait until every session started at the previous tick has either completed pairing (section 5.7 step 9) or ended.
   5. Lock `c.mu`.
   6. `mixpool.RemoveConfirmedSessions()`; then `expireMessages()`: `mixpool.ExpireMessages(c.height)` (the height last given to `ExpireMessages`), and for every pending local peer whose PR hash is no longer in the mixpool: remove it from the pending pairing and send it `expiredPRErr`; delete emptied pairings.
   7. For every pending pairing (map order):
      1. `prs = mixpool.CompatiblePRs(pairing)` (all PRs with that pairing ID, minus lower-expiry double spends, sorted by hash); then `prs = observer.ExcludePRs(prs)` (drops PRs spending an output with ≥ 2 misbehaviour strikes).
      2. `localPeers` = a fresh map holding a clone (`cloneLocalPeer(false)`, section 7.2) of each pending local peer whose identity owns one of `prs`.
      3. Log (debug) `Have %d compatible/%d local PRs waiting for pairing %x`.
      4. If `len(prs) > 512` (`wire.MaxMixPeers`): build the list "PRs of `localPeers` (map order), then all other `prs` in order" and keep the first 512.
      5. `pairingWG.Add(1)`, `runWG.Add(1)`; start `pairSession(ctx, newPairedSessions(pairing, localPeers), prs, epoch)` in a new goroutine that calls `runWG.Done()` when it returns.
   8. Unlock `c.mu`.

A session is started for a pairing even when it has fewer than `MinPeers` PRs, so that KEs are still broadcast (other nodes use KEs to discover PRs they are missing).

Consequences of this structure that a port must keep:

- No session is ever started at the first epoch boundary after `Run` begins: that boundary only starts the warm-up. The earliest session start is the **second** boundary, so a freshly started wallet begins its first KE stage between `E` and `2E` after start-up.
- The height used by loop step 6 is the one captured from `Wallet.BestBlock()` in `NewClient` until the wallet first calls `ExpireMessages` (`client.go:421,443,1097-1109`). If no block arrives, every tick re-applies expiry at that initial height.
- A pending peer whose request was cancelled is still cloned at loop step 7.2 and still counted as a local PR (the log line and the "no more active local peers" test of section 5.6.2 count it); it is merely skipped by every per-peer operation. A pairing whose only local peers are cancelled therefore starts a session goroutine each epoch that ends immediately with `errNoActiveLocalPeers` at the KE step, without sending anything, until the PR leaves the mixpool.
- `pairingWG.Wait()` at loop step 4 is called with `c.mu` **not** held; the lock is taken only afterwards. A previous-epoch session that has not completed pairing releases the wait group only when its `pairSession` returns (the deferred `donePairing`, `client.go:1133`), which happens after its context expires at the epoch boundary and the loop of section 5.6.2 has run out of retries or hit a non-actionable error; the new sessions' `begin` (the boundary) can therefore already be in the past when they start.

`removeUnresponsiveDuringEpoch(prs, epoch)` (`client.go:857-875`): group `prs` by pairing ID; for each group (map order) fetch `ReceiveKEsByPairing(pairing, epoch)`; if fewer than `MinPeers` KEs exist, **return** (abandoning the remaining groups); otherwise call `mixpool.RemoveUnresponsiveDuringEpoch(group, epoch)`, which removes each PR whose identity sent no KE for that epoch.

### 5.5 Stage deadlines

`deadlines.start(begin, T)` sets six absolute deadlines (`client.go:157-169`):

| Deadline | Value | Meaning |
|---|---|---|
| `recvKE` | `begin + T` | all KEs received |
| `recvCT` | `begin + 2T` | all CTs received |
| `recvSR` | `begin + 3T` | all SRs received, and roots obtained |
| `recvDC` | `begin + 4T` | all DCs received |
| `recvDCRS` | `begin + 5T` | end of the window in which a peer may reveal secrets after the DC stage |
| `recvCM` | `begin + 6T` | all CMs received |

`begin` is the epoch boundary for the first session attempt of an epoch, and `time.Now()` at the moment a session is re-formed (or retried) within the same epoch. The whole pairing attempt, including every re-formed session, runs under a context whose deadline is the next epoch boundary (`client.go:1138-1141`).

Each receive returns as soon as messages from the expected number of distinct identities are present, so stages normally complete early. The one exception is the `recvDCRS` wait, which always lasts until its deadline unless a secrets message arrives (section 5.7 step 26). A run that succeeds therefore publishes its CMs no earlier than `begin + 5T` (150 s after the epoch boundary on mainnet, 50 s on testnet3).

### 5.6 Session pairing (`pairSession`)

One goroutine per pending pairing per epoch (`client.go:1132-1410`). Inputs: the `pairedSessions` object `ps` (pairing ID, the cloned local peers), the candidate PR list, the epoch.

#### 5.6.1 Set-up

1. `unixEpoch = epoch.Unix()`; derive a context with deadline `epoch + E`.
2. `ps.epoch = epoch`; `ps.deadlines.start(epoch, T)`.
3. `sid = SortPRsForSession(prs, unixEpoch)` (sorts `prs` into peer order).
4. `ps.runs = [ {sid, prs, freshGen: false} ]`; this is the "new run".
5. `altsesCounts` = empty map from session ID to count.

#### 5.6.2 Loop

Repeat:

1. **Materialize a new run** (only when the previous iteration created one, or on the first iteration): set its index (`idx` = position in `ps.runs`); for each PR at position i in peer order:
   - look the identity up in the `localPeers` of `ps.runs[idx − 1]` (or in `ps.localPeers` when `idx == 0`); this is the run appended immediately before, which is not necessarily the run that just executed when pairing completed on an earlier attempt (section 5.6.6) (`client.go:1234-1238`); if found, the run's peer is `cloneLocalPeer(run.freshGen)` of it and is added to the run's `localPeers`; otherwise it is a remote peer placeholder (`remote = true`, holding only the PR and identity);
   - set `myVk = i`, `myStart = start_i`; append to `run.peers` and `run.mcounts`.
   - `run.mtot = mtot`. Log (debug) `created session for pairid=%x from %d total %d local PRs %s` (pairing, n, local count, PR hash list).
   - If the run has no local peers: log `no more active local peers` and **return**.
2. `r, err = run(ctx, ps)` (section 5.7). `revealedSecrets = false`.
3. Dispatch on `err`:

| Result of the run | Action |
|---|---|
| `errOnlyKEsBroadcasted` | return. |
| `*alternateSession` with `err` set (`ErrTooFewPeers` or `ErrUnknownPRs`) | log `Unable to recreate session: %v`; `altsesCounts[zero sid]++`; if the count exceeds 2: log `Aborting alternate session forming`, return. Otherwise restart the deadlines at `now` and repeat the loop **on the same run** (its KEs are re-submitted, which is a no-op for the mixpool, and pairing completion is awaited again). |
| `*alternateSession`, `sid ≠ r.sid` | log `Recreating as session %x (pairid=%x)`; append its `unresponsive` PRs to the epoch's unresponsive list; `altsesCounts[sid]++`; abort as above when > 2; otherwise re-form with `{sid, prs}` from the alternate session, `freshGen = false`, without requiring peer agreement. |
| `*alternateSession`, `sid = r.sid` | log `Alternate session matches current run (pairid=%x)`; set `ps.peerAgreement = true`, `ps.peerAgreementRunIdx = r.idx`; `altsesCounts[sid]++`; abort when > 2; otherwise "re-form" with the same sid (see step 4). |
| `*sizeLimited` | if fewer than `MinPeers` PRs were kept: log `Aborting session with too few remaining peers`, return. Otherwise log `Recreating as session %x due to exceeded mixing limits (pairid=%x)` and re-form with the kept PRs and their sid, `freshGen = false`, requiring peer agreement. |
| `errTriggeredBlame` or `mixpool.ErrSecretsRevealed` (anywhere in the joined error) | `revealedSecrets = true`; run blame assignment (section 5.12). If its result is not a `blamedIdentities`: log `Aborting session for failed blame assignment: %v`, return. Otherwise continue with the blamed set below. |
| `errNoActiveLocalPeers` | return. |
| `blamedIdentities` (returned by the run itself, or produced by blame assignment) | log `Identified %d blamed peers %x`; if `n − len(blamed) < MinPeers`: log `Aborting session with too few remaining peers`, return. Otherwise re-form with `excludeBlamed` (section 5.6.5), `freshGen = revealedSecrets`, requiring peer agreement. |
| any other non-nil error | log at **error** level `Run error: %v`; return. |
| nil | `mixedSession = r`; return. |

   The rows are tested in the order listed and the first match wins (a Go `switch` over `errors.Is`/`errors.As`, `client.go:1276-1360`), except that the `blamedIdentities` row is a separate test evaluated after the switch (`client.go:1362`): it applies when blame assignment produced a list, or when no earlier row matched and the run's error is (or wraps) a `blamedIdentities`. Because several per-peer errors can be joined into one run error, an error that contains both `errTriggeredBlame` and a context error (one local peer triggered blame, another could not be queued) is handled as the blame row. `revealedSecrets` is reset to false at the top of every iteration, before the dispatch.

4. **Re-forming** with a candidate run `{sid, prs, freshGen}`:
   - If `sid` equals the last run's sid: log `recreated session matches previous try; not creating new run states` and do not append (if `ps.peerAgreementRunIdx ≥ len(ps.runs)`, reset it to the last run's index and log `reverting incremented peer agreement index`).
   - Otherwise append the candidate to `ps.runs`; it is materialized at step 1 of the next iteration.
   - `ps.deadlines.start(now, T)`.
   - If peer agreement is required: `ps.peerAgreementRunIdx = len(ps.runs) − 1`.

All session-scoped log lines are prefixed `sid=%x/%d ` with the run's session ID and run index (`client.go:351-365`).

#### 5.6.3 Exit handler

Runs on every return path (`client.go:1149-1203`), before the context is cancelled and before `pairingWG` is released:

1. `removeUnresponsiveDuringEpoch(unresponsive, unixEpoch)` for the PRs collected from alternate-session results.
2. If the session mixed: remove every identity of the mixed run from `ps.localPeers`; start a detached task that sleeps 10 s, logs `removing mixed session completed with transaction %v`, and calls `mixpool.RemoveSession(sid)` (which also removes the run's PRs).
3. If `ps.localPeers` is now empty, stop here (no pending entry is created). Otherwise, for every peer still in `ps.localPeers` whose PR is still in the mixpool: create `cloneLocalPeer(revealedSecrets)`; when `revealedSecrets` is true additionally `genDicemixKeys()` (on failure send the error to the request and drop it).
4. Under `c.mu`: (re)create the pending pairing entry — even when step 3 produced no clone, leaving an empty entry that the next expiry sweep deletes — and insert each such clone whose PR is still in the mixpool (checked a second time).

`revealedSecrets` here is the value from the **last** loop iteration only. Peers whose PR is no longer in the mixpool are dropped without any result being sent (Quirk Q10).

#### 5.6.4 Size limiting (first agreement only)

Performed inside the run immediately after pairing completes, only when `ps.peerAgreement` was false (`client.go:1641-1694`):

1. Set `ps.peerAgreement = true`, `ps.peerAgreementRunIdx = run.idx`.
2. Walk the run's PRs in peer order with counters `npeers = 0`, `mtot = 0` and a size estimator. A PR is **excluded** when `npeers + 1 > 64`, or `mtot + MessageCount > 1024`, or adding its inputs/outputs makes the estimated transaction size `≥ 100000`. Otherwise it is counted (`npeers++`, `mtot += MessageCount`, estimator updated). An excluded PR does not stop the walk; later PRs are still tested.
3. Size estimate (`mixclient/limits.go:16-95`): `12 + 2·varintSize(inputs) + varintSize(outputs) + 166·inputs + Σ_outputs (10 + varintSize(scriptLen) + scriptLen)`, where each mixed output has `scriptLen = 25` (36 bytes per output), a change output uses its actual script length, and 166 = 32 + 4 + 1 + 8 + 4 + 4 + 1 + 108 + 4 (a 108-byte signature script: 1 + 73 + 1 + 33).
4. If nothing was excluded, continue the run. Otherwise build `kept` by walking the PRs in peer order: append each PR that is not the next excluded PR; when the next excluded PR is met, drop it and advance; **stop the walk as soon as the last excluded PR has been met**. PRs positioned after the last excluded PR are therefore not in `kept` even though step 2 accepted them (Quirk Q4; this exact result is required for session agreement).
5. `sid = SortPRsForSession(kept, unixEpoch)`; return `sizeLimited{prs: kept, sid, excluded}`.

Because the walk is gated on `ps.peerAgreement` being false, it never runs for a pairing whose agreement flag was already set by the "alternate session matches current run" branch of section 5.6.2 (`client.go:1298-1299,1641`).

#### 5.6.5 `excludeBlamed`

(`client.go:2550-2581`) Take the previous run's peers in peer order, dropping those whose identity is blamed; a dropped **local** peer is sent `testPeerBlamedError`. The remaining PRs are re-sorted with `SortPRsForSession(prs, epoch)` (same epoch value), giving the new sid. The new run's `freshGen` is true exactly when secrets were revealed.

#### 5.6.6 Pairing completion (`completePairing`)

(`client.go:1416-1487`) Considers `ps.runs[ps.peerAgreementRunIdx:]`, oldest first:

1. For each such run, ask the mixpool for the session's KEs without waiting. If exactly `len(run.prs)` KE messages are present, the run is complete: record them as `run.kes` and return that run.
2. Otherwise start a waiter that blocks until KEs from `len(run.prs)` distinct identities are present or `recvKE` passes, then applies the same exact-count test.
3. Return the first waiter that succeeds. If every waiter fails, return the last failure (`errIncompletePairing`, `unknown session …`, or `ErrSecretsRevealed`). If the session context ends first, return its error.

The completed run can be an earlier attempt than the newest one; the run state machine then continues on that earlier run (log `replacing previous session attempt %x/%d`).

#### 5.6.7 Alternate session formation

Invoked when pairing did not complete and peer agreement has never been reached (`client.go:2366-2548`). Input: the pairing, the epoch, the current run's PRs.

1. `kes = mixpool.ReceiveKEsByPairing(pairing, unixEpoch)`: for every identity, its most recently accepted KE, provided that KE's epoch matches and the identity's PR has this pairing.
2. Keep one KE per identity, only for identities that own one of the current run's PRs (a KE whose PR is not among them is logged `missing PR by %x, but have their KE %s` and ignored).
3. `unresponsive` = the run's PRs (in peer order) whose identity has no kept KE; log `Identity %x (PR %s) is unresponsive` for each.
4. Count sightings: for every kept KE and every hash in its `SeenPRs`, skipping the sender's own PR hash and skipping hashes of unresponsive PRs, `prCounts[hash]++`. Hashes of PRs unknown to this wallet are counted too.
5. `totalPRs = len(prCounts)`; `needed = totalPRs / 2` (integer division). `remaining` = all hashes with `prCounts ≥ needed`.
6. Loop:
   1. If `len(remaining) < MinPeers`: return `alternateSession{unresponsive, err: ErrTooFewPeers}`.
   2. Sort `remaining` by ascending count, ties by ascending hash bytes.
   3. If the lowest count equals `len(remaining) − 1` (every remaining PR is seen by every other remaining peer): map the hashes to PRs; if any hash is unknown locally return `alternateSession{unresponsive, err: ErrUnknownPRs}`; otherwise `sid = SortPRsForSession(prs, unixEpoch)` and return `alternateSession{prs, sid, unresponsive}`.
   4. Otherwise log `Removing PR %s, was only seen %d times`, drop the first element, and recompute the counts over the survivors only: every survivor starts at 0; for each survivor whose PR and kept KE are known, each `SeenPRs` hash other than its own that is still a survivor is incremented.

The result is always returned to `pairSession` as an error value (section 5.6.2).

### 5.7 The run state machine (`run`)

(`client.go:1501-2146`) Operates on the newest run of `ps` (switching to an earlier one at step 6 if that one completes pairing). "Each active local peer" means every peer of the run that is neither remote nor cancelled; the work is dispatched to the worker pool and the step fails with `errNoActiveLocalPeers` when there is none (section 7.3). "Send X" means the scheduled submission of section 5.13 with the stated deadline; send errors are logged and otherwise ignored unless stated.

**KE stage**

1. For each active local peer: if `run.freshGen`, run `genDicemixKeys` (new seed, keys, SR messages and DC messages). Then, if the peer has no KE yet:
   - `rs = RS(id, sid, run 0, seed, [minimal bytes of each SR message], dcMsg)`; `commitment` per section 4.7;
   - `ke = KE(id, sid, unixEpoch, run 0, pos = myVk, ECDH public key, PQ public key, commitment, seenPRs = PR hashes in peer order)`;
   - store `ke`, clear any CT/SR/FP/DC/CM, store `rs`.
   A failure returns the error. Then `run.freshGen = false`.
2. Send KE (deadline `recvKE`).
3. If `n < MinPeers`: log `pairing %x: minimum peer requirement unmet`; return `errOnlyKEsBroadcasted`.
4. If no PR of the run has flag bit 0 set: log `pairing %x: no solver available`; return `errOnlyKEsBroadcasted`.
5. `completePairing` (section 5.6.6). On failure: if peer agreement was never reached return the alternate-session result (section 5.6.7); otherwise return the error.
6. If the completed run is not the newest, continue on it. Log `received all %d KEs`.
7. For **every** local peer of the pairing (`ps.localPeers`, not only this run's): `coinjoin.resetUnmixed(prs)` (section 5.8.2).
8. If peer agreement was not yet reached: mark it and apply size limiting (section 5.6.4), possibly returning `sizeLimited`.
9. Record each received KE on its peer (by identity). Under `c.mu`, remove this run's local peers from the pending pairing (deleting it when empty). Release the pairing wait group (once per `pairedSessions`).
10. Sort the KEs into peer order; `seenKEs` = their hashes. If none of the KEs belongs to a local peer of this run, return the error `excluded all local peers`.
11. Parse every KE's ECDH key as a secp256k1 public key. Any that fail are blamed: log `blaming %x during run (invalid ECDH pubkeys)` and return the `blamedIdentities`.

**CT stage**

12. For each active local peer without a CT: `pqct = kx.Encapsulate(prng, [PQ public keys in peer order])` (n encapsulations, own key included, consuming the run PRNG); `ct = CT(id, sid, 0, pqct, seenKEs)`. Errors are logged only.
13. Send CT (deadline `recvCT`).
14. Receive CTs (expect n, deadline `recvCT`); record each on its peer. A receive error (section 5.14) is returned. If the number received is not n: log `received %d CTs for %d peers; rerunning` and return `blameTimedOut(CT)` — the identities of all peers without a CT, or `errBlameFailed` if there are none. Sort into peer order; `seenCTs`.

**SR stage**

15. For each active local peer without an SR:
    - If any CT does not carry exactly n ciphertexts, add that CT's identity to a shared blamed set and stop processing this peer.
    - `revealed = {all parsed ECDH keys, Ciphertexts[k] = cts[k].Ciphertexts[myVk], MyIndex = myVk}`.
    - `shared = kx.SharedSecrets(revealed, sid, 0, mcounts)` (section 5.9.1). On error set `triggeredBlame` and fail the step with `errTriggeredBlame`.
    - For each own message i: `srMix[i] = SRMix(srMsg[i], SRMixPads(shared.SRSecrets[i], myStart + i))`.
    - `sr = SR(id, sid, 0, minimal bytes of srMix, seenCTs)`.
16. If the blamed set is non-empty: log `blaming %x during run (wrong ciphertext count)`; return it.
17. Send SR (deadline `recvSR`). A peer with `triggeredBlame` sends its RS here (it has no SR). Then, if step 15 failed for any peer, return that error.
18. Receive SRs (expect n, deadline `recvSR`); record; errors returned; on a short count log `received %d SRs for %d peers; rerunning` and return `blameTimedOut(SR)`. Sort; `seenSRs`.
19. Every SR must have exactly `mcount_k` rows of exactly `mtot` values; offenders are blamed (`blaming %x during run (wrong SR DC-mix dimensions)`, return). Concatenate all rows in peer order (`mtot` rows of `mtot` values), parse as integers, `powerSums = AddVectors(rows)`, `coeffs = Coefficients(powerSums)` (section 5.9.3).
20. `roots = roots(ctx bounded by recvSR, seenSRs, run, coeffs)` (section 5.11.1); errors returned. Store them on the run.

**DC stage**

21. For each active local peer without a DC: for each own SR message find its slot with `constTimeSlotSearch(srMsg[i], roots)`; if any is missing set `triggeredBlame` and fail with `errTriggeredBlame`. Otherwise `dcNet[i] = DCMix(DCMixPads(shared.DCSecrets[i], myStart + i), dcMsg[i], slot_i)` and `dc = DC(id, sid, 0, dcNet, seenSRs)`.
22. Send FP and DC (deadline `recvDC`); per peer the FP precedes the DC. A peer with `triggeredBlame` sends its FP (if any) and RS. If step 21 failed, log `DC-net error: %v` and return the error.
23. Receive DCs (expect n, deadline `recvDC`); record; errors returned; short count → log `received %d DCs for %d peers; rerunning`, return `blameTimedOut(DC)`. Sort; `seenDCs`.
24. Every DC must have exactly `mcount_k` vectors of exactly `mtot` elements; offenders are blamed (`blaming %x during run (wrong DC-net dimensions)`, return). `mixed = XorVectors(all vectors in peer order)`: `mtot` 20-byte messages, element s being the message that reserved slot s.

**CM stage**

25. For each active local peer without a CM: add one mixed output per element of `mixed` (in slot order) to its coinjoin, sort it, and `confirm` (section 5.8.4). `errMissingGen` → log `missing message; blaming and rerunning`, set `triggeredBlame`, fail with `errTriggeredBlame`. Any other error is sent to the request's result channel and fails the step. Otherwise `cm = CM(id, sid, 0, copy of the coinjoin transaction, seenDCs)`. On failure log `confirm error: %v` and return.
26. Wait for a secrets message: receive RSs (expect 1, deadline `recvDCRS`). A receive error is returned. If at least one RS is present return `mixpool.ErrSecretsRevealed`. In the normal case this step blocks until `recvDCRS`.
27. Send CM (deadline `recvCM`).
28. Receive CMs (expect n, deadline `recvCM`); record. An error other than `ErrSecretsRevealed` is returned; `ErrSecretsRevealed` is ignored at this point. Short count → log `received %d CMs for %d peers; rerunning`, return `blameTimedOut(CM)`. Sort.
29. Let `cj` be the coinjoin of the first non-remote peer in peer order (cancelled requests included). Let `lowestJitter` be computed over the run's non-remote peers in peer order: it starts at 0, and a peer's jitter replaces it whenever it is currently 0 or greater than that peer's jitter (the minimum jitter, with 0 treated as "unset").
30. For each CM in peer order: `cj.mergeSignatures(cm)` (section 5.8.5); failures are blamed (`blaming %x during run (confirmed wrong coinjoin)`, return).
31. `validateMergedCoinjoin` (section 5.8.6); a failure returns the `blamedIdentities` of the PRs owning the failing inputs.
32. Sleep `lowestJitter + U[0, 300 ms)`. `wallet.PublishTransaction(background context, cj.tx)`; an error is returned (and the session ends with `Run error`).
33. Send `nil` to the result channel of every active local peer. Record `run.cj = cj`. Return success.

`blameTimedOut` logs `blaming %x during run (%s timeout)` with the stage name `CT`, `SR`, `DC` or `CM` (`client.go:85-117`).

Details of the run that follow from the code and affect exclusion sets:

- **Recording before testing.** At every receive (steps 14, 18, 23, 28) the returned messages are stored on their peers before the receive error and the count are examined, and messages from identities that are not in the run's PR list are ignored for recording but still counted in `len(received)` (`client.go:1786-1799`). The count test is on the number of messages returned, the timeout blame on which peers have no stored message.
- **A secrets message stands in for a stage message in the count.** The mixpool counts an RS from an identity toward the expected number of distinct senders (section 5.14). At the CT, SR and DC stages the receive then returns `ErrSecretsRevealed` and the run goes to blame assignment. At the CM stage (step 28) that error is ignored, so a peer that publishes an RS instead of a CM after the `recvDCRS` window makes the receive return as soon as the other `n − 1` CMs are in; it is then excluded by `blameTimedOut(CM)` without any reveal, and the re-formed session reuses every remaining peer's keys, SR messages and mixed addresses (`freshGen` false). A peer that publishes both a CM and an RS at that point has no effect: the run completes.
- **Per-peer skip conditions.** Steps 1, 12, 15, 21 and 25 each skip a local peer that already holds that stage's message (`p.ke`, `p.ct`, `p.sr`, `p.dc`, `p.cm` non-nil). This matters only when the same run object is executed again (the retry and same-sid cases of section 5.6.2), in which case the stored message is re-signed and re-submitted unchanged (in practice only the KE, since those cases end before pairing completes).
- **KE `Pos`** is taken from the identity-to-index map of the run that was newest when the KE step executed (`client.go:1513-1516,1546`); it equals `myVk`.
- **Step 7 precedes step 8**: every local `CoinJoin` of the pairing has already been reset to the full (pre-size-limit) PR list when `sizeLimited` is returned; it is reset again when the re-formed run completes pairing.
- **Step 25 mutates shared transactions concurrently only across distinct requests.** Each local peer appends the `mtot` mixed outputs to, sorts and signs its own `CoinJoin`; no two peers of one run share a `CoinJoin`.
- **Step 32's sleep is unconditional and uninterruptible** (`time.Sleep`), and the publish that follows ignores the session deadline.

### 5.8 Coinjoin assembly

One `CoinJoin` object exists per `Dicemix` request and is shared by every clone of that request's local peer.

#### 5.8.1 Construction and inputs

`NewCoinJoin` stores the generator, change output, mixed value, PR expiry and message count, and creates an empty version-1 transaction. `AddInput(input, prevScript, _, privKey)` (`coinjoin.go:86-117`):

1. `pub` = compressed public key of `privKey`; `proof = SignUtxoProof(prExpiry)` (section 4.6).
2. `opcode = prevScript[0]` if that byte is `OP_SSGEN`, `OP_SSRTX` or `OP_TGEN`; otherwise 0.
3. Append `{OutPoint: input.PreviousOutPoint, Script: nil, PubKey: pub, Signature: proof, Opcode: opcode}` to the PR UTXO list.
4. Record `prevScripts[outpoint] = prevScript`; `inputValue += input.ValueIn`.

`AddInput` does not add anything to the transaction; inputs enter it only through `resetUnmixed`.

#### 5.8.2 `resetUnmixed(prs)`

(`coinjoin.go:121-145`) Clear the transaction's inputs and outputs and the bookkeeping maps, then for each PR in the given (peer) order: for each of its UTXOs append an input (section 4.11) and record `contributed[outpoint] = PR identity`; then, if the PR has a change output, append that output object.

#### 5.8.3 Deterministic ordering (`sort`)

After the `mtot` mixed outputs have been appended, the transaction is sorted in place with `txsort.InPlaceSort` (`coinjoin.go:195-207`, `dcrd/dcrutil/v4@v4.0.3/txsort/txsort.go:39-60,85-98,112-115`):

- **Inputs** ascending by: previous outpoint tree; then previous transaction hash compared after reversing its 32 bytes (i.e. as the big-endian number that the usual hex display shows); then, for equal hashes, output index.
- **Outputs** ascending by: value; then script version; then script bytes (lexicographic, a proper prefix sorting first).

Elements that compare equal are byte-identical in everything the comparison and the transaction hash cover, so the unstable sort is deterministic in effect. After sorting, `myInputs` = the indexes of inputs whose outpoint is in `prevScripts`, and `txHash = tx.TxHash()` (the prefix hash, which excludes signature scripts).

#### 5.8.4 `confirm`

(`coinjoin.go:213-264`)

1. Candidate outputs: those with `Value == mixValue`, `Version == 0` and script length equal to the length of the first generated script (25).
2. For each generated script (in `gen` order) find the index of an identical candidate script using constant-time comparison over all candidates; the last identical candidate wins; record −1 if none.
3. If any script was not found return `errMissingGen`.
4. For each index in `myInputs`: `wallet.SignInput(tx, index, prevScripts[outpoint])`; return the first error.
5. `mixedIndices` = the indexes from step 2.

dcrwallet's `SignInput` produces a standard P2PKH signature script with `SigHashAll` and an ECDSA secp256k1 signature using the compressed public key (`wallet/mixing.go:121-153`).

#### 5.8.5 `mergeSignatures(cm)`

(`coinjoin.go:271-285`) If `cm.Mix.TxHash() != txHash` return `errSignedWrongTx`. Otherwise, for each input index i of `cm.Mix` whose outpoint was contributed by `cm.Identity`, copy that input's signature script into input i of the local transaction. Signature scripts supplied for inputs the sender did not contribute are ignored.

#### 5.8.6 `validateMergedCoinjoin`

(`client.go:2260-2340`) For each input of the merged transaction:

1. Look up the PR UTXO for its outpoint; if absent, mark the input failed.
2. Build the previous script from the UTXO: `76 a9 14 HASH160(PubKey) 88 ac` when `Opcode == 0`; the same script prefixed by the opcode byte when `Opcode` is `OP_SSGEN`, `OP_SSRTX` or `OP_TGEN`; no script for any other opcode. `HASH160 = RIPEMD-160(BLAKE-256(x))`.
3. Execute the script engine on (previous script, transaction, input index) with script version 0 and flags `DiscourageUpgradableNops | VerifyCleanStack | VerifyCheckLockTimeVerify | VerifyCheckSequenceVerify | VerifyTreasury`, no signature cache. An engine construction error or execution error marks the input failed.

The three failure cases log, at debug level and without the `sid=` prefix: `warn: prev pubkey for %v not found during tx sig validate` (outpoint); `warn: blaming peer for error creating script engine: %v`; `warn: coinjoin %s input %d prev outpoint %v script fails to execute: %v` (transaction hash, input index, outpoint, error). Each blamed PR logs `warn: blaming %x for failed script execute` (identity) (`client.go:2300-2333`). The previous script is built in one reused 26-byte buffer `[opcode, 76, a9, 14, <20 bytes>, 88, ac]`: the untagged form is bytes 1..26, the tagged form is all 26 bytes. For any other opcode value the script passed to the engine is empty (nil), and whatever the engine then reports (construction or execution error) marks the input failed.

If no input failed, succeed. Otherwise return, in PR (peer) order, the identity of every PR that has at least one failed input. The previous output's real script and amount are never fetched: validation relies on the public key the PR declared. Whether that key matches the on-chain output is checked only by a mixpool that has a UTXO fetcher (dcrd) when it accepts the PR; the wallet's own mixpool is created without one (`wallet/mixing.go:38-49`).

### 5.9 DC-net mathematics

All arithmetic on field elements is modulo `F = 2^127 − 1` with canonical results in `[0, F)`.

#### 5.9.1 `SharedSecrets`

(`keyagreement.go:130-237`) Inputs: own `KX` (with `PQCleartexts` already filled by `Encapsulate`), `RevealedKeys{ECDHPublicKeys[n], Ciphertexts[n], MyIndex}`, `sid`, `mcounts[n]`.

1. If `len(ECDHPublicKeys) != n` return `ECDH public key count (%d) must match peer count (%d)`; if `len(Ciphertexts) != n` return `ciphertext count (%d) must match peer count (%d)`.
2. `mcount = mcounts[MyIndex]`. Allocate `SRSecrets[mcount][mtot]` (byte strings, initially absent) and `DCSecrets[mcount][mtot]` (vectors, initially absent).
3. For each own message `i` in `0..mcount`, walk the global message index `m` from 0 over peers `Q = 0..n-1`:
   1. If `Q == MyIndex` and `mcount == 1`: `m += 1`; continue (no self key is derived).
   2. `K = BLAKE-256(ecdh_x(d, ECDHPublicKeys[Q]))`.
   3. Decapsulate `Ciphertexts[Q]` with the own PQ private key; on failure return `DecapsulateError{SubmittingIndex: Q}`. XOR the result into `K`.
   4. If `Q != MyIndex`, XOR `PQCleartexts[Q]` into `K`.
   5. For each message `j` in `0..mcounts[Q]`:
      - if `Q == MyIndex` and `j == i`: `m += 1`; continue;
      - choose `(seedCounter, nonceCounter)` per section 4.9; derive the pair PRNG;
      - `SRSecrets[i][m]` = its first 32 bytes; `DCSecrets[i][m]` = the next `mtot` 20-byte elements;
      - `m += 1`.

The entry at the message's own global index (`myStart + i`) is left absent in both tables; the pad functions skip that index.

#### 5.9.2 Slot reservation pads and mix

`SRMixPads(kp, my)` with `kp = SRSecrets[i]`, `my = myStart + i` (`dcnet.go:17-43`): for each column `c` in `0..mtot`:

```
pad[c] = Σ over g in 0..mtot, g != my of  sign(g) · int_be(BLAKE-256(kp[g] ‖ u64le(c+1)))     (mod F)
sign(g) = +1 if my > g,  −1 if my < g
```

`SRMix(m, pads)`: `mix[c] = m^(c+1) + pad[c] (mod F)` for `c` in `0..mtot` (`dcnet.go:48-57`).

Because both ends of every message pair derive the same `srSecret` and apply opposite signs, the pads of all `mtot` vectors cancel column-wise: the column sums are the power sums `S_c = Σ_messages m^(c+1)`.

#### 5.9.3 Power sums to polynomial

`Coefficients(S)` with `S[0..mtot)` returns `a[0..mtot]` (`dcnet.go:116-133`):

```
a[mtot] = F − 1                                   (−1 mod F)
for i = 0 .. mtot−1:
    a[mtot−1−i] = −(i+1)^(−1) · Σ_{j=0..i} a[mtot−i+j] · S[j]      (mod F)
```

`g(x) = Σ a[k]·x^k` has exactly the SR messages as its roots when all peers were honest and the messages are distinct. The roots, sorted ascending as integers, define the slots: the message equal to root s owns slot s.

#### 5.9.4 Slot lookup

`constTimeSlotSearch(secret, roots)` left-pads the secret and each root to 16 bytes and compares in constant time, returning the highest matching index or −1 (`client.go:2587-2606`).

#### 5.9.5 XOR DC-net

`DCMixPads(kp, my)` with `kp = DCSecrets[i]`: the XOR of `kp[g]` over all `g != my`, a vector of `mtot` elements (`dcnet.go:151-160`). `DCMix(pads, msg, slot)`: the pads with the 20-byte `msg` XORed into element `slot` (`dcnet.go:165-177`). The XOR of all `mtot` published vectors cancels every pad and leaves, at element s, the message of the peer that reserved slot s (`vec.go:72-78`).

### 5.10 sntrup4591761

This is Streamlined NTRU Prime 4591^761 of the 2017 NTRU Prime submission; the Go files describe themselves as a port of the public-domain reference C implementation (the exact paper revision is UNVERIFIED, and the randomness routine of section 5.10.2 does not match a plain signed sort on every toolchain) — not the later NIST-round `sntrup761`; key, ciphertext and shared-key derivations differ. Because keys and ciphertexts must be reproducible from the run PRNG by every peer, the implementation must match the Go code bit for bit, including how many PRNG bytes each step consumes.

#### 5.10.1 Parameters and representations

- `p = 761`, `q = 4591`, weight `w = 286`. `R = Z[x]/(x^761 − x − 1)`.
- A *small* element has coefficients in {−1, 0, 1}, stored as signed 8-bit. A *t-small* element is small with exactly 286 non-zero coefficients.
- Elements of `R/3` use representatives {−1, 0, 1}; elements of `R/q` use representatives in [−2295, 2295], stored as signed 16-bit.
- Coefficient index k multiplies `x^k`.

Reductions, on signed 32-bit integers with arithmetic right shifts (`r3/mod3/mod3.go:10-14`, `rq/modq/modq.go:10-14`):

```
freeze3(a):  a −= 3·((10923·a) >> 15);   a −= 3·((89478485·a + 134217728) >> 28);   return a as i8
freezeq(a):  a −= 4591·((228·a) >> 20);  a −= 4591·((58470·a + 134217728) >> 28);   return a as i16
```

Inverse modulo q: `recipq(a) = a^4589 mod q`, computed with `freezeq` after every multiplication (`modq.go:27-46`). Modulo 3 the inverse of ±1 is itself and products are plain integer products.

#### 5.10.2 Randomness (`zx/random.go`)

- `random32`: read 4 bytes; interpret little-endian as a signed 32-bit value.
- `RandomSmall(g)` (3044 bytes): for each of the 761 coefficients in index order, `g[i] = (((r & 0x3fffffff) · 3) >> 30) − 1` computed on the unsigned 32-bit value `r`.
- `RandomTSmall(f)` (3044 bytes): read 761 values `r[i]` in index order; for `i < 286` clear bit 0 (`r[i] &= −2`); for `i ≥ 286` clear bit 1 and set bit 0 (`r[i] = (r[i] & −3) | 1`); sort all 761 values ascending **as unsigned 32-bit integers**; `f[i] = (r[i] & 3) − 1`.

  **Toolchain dependence of the sort (Quirk Q40).** The Go code is a fixed compare-exchange network whose comparison is `crypto/subtle.ConstantTimeLessOrEq(int(uint32(y)), int(uint32(x)))` (`zx/random.go:29-59`), i.e. it is called with values outside that function's documented domain (`0 … 2^31−1`). What the comparison computes therefore depends on the Go standard library and the platform:

  | Build | Comparison performed by `minmax(x, y)` (exchange when true) | Result of `sort` |
  |---|---|---|
  | Go ≥ 1.26, 64-bit `int` (what the parity pin produces: dcrwallet's `go.mod` requires `go 1.26.0`) | `y ≤ x` on the zero-extended values (`crypto/internal/constanttime.LessOrEq` is a plain `x <= y`) | ascending unsigned order; any correct sort gives the same result |
  | Go ≥ 1.26, 32-bit `int` | `y ≤ x` as signed 32-bit integers | ascending signed order |
  | Go ≤ 1.25, any platform | bit 31 of `(y − x − 1)` in wrapping signed 32-bit arithmetic (`crypto/internal/fips140/subtle.ConstantTimeLessOrEq`), i.e. exchange iff `(y − x) mod 2^32` is 0 or greater than `2^31` | not a total order; the output depends on the exact network below |

  The network, for `n = 761` (so `top = 512`): `top = 1; while top < n − top: top *= 2`; then for `p = top, top/2, …, 1`: (a) for `i` in `0..n−p` with `i & p == 0`: `minmax(x[i], x[i+p])`; (b) for `q = top, top/2, …` while `q > p`: for `i` in `0..n−q` with `i & p == 0`: `minmax(x[i+p], x[i+q])`. `minmax(a, b)` leaves the smaller value in `a`.

  Because `f` (key generation) and `r` (every encapsulation) come from this routine, wallets built with different toolchain generations or `int` widths derive different KE keys and ciphertexts from the same revealed seed, and would blame each other in section 5.12 (steps 8.5 and 9). Which variant deployed Go wallets use depends on the toolchain each release was built with, which was not established in this study (UNVERIFIED; see Open questions). The standard-library behaviours were read from the Go 1.25.14 and 1.26.0 sources.

#### 5.10.3 Encodings

`zx.Encode` (small element → 191 bytes, `zx/encoding.go:10-25`): for `i` in `0..190`, byte `i` = `(f[4i]+1) | (f[4i+1]+1) << 2 | (f[4i+2]+1) << 4 | (f[4i+3]+1) << 6`; byte 190 = `f[760] + 1`. `zx.Decode` reverses it taking two bits per coefficient minus 1 (`zx/encoding.go:28-46`); it does not validate.

`rq.Encode` (element of R/q → 1218 bytes, `rq/encoding.go:14-60`), signed 32-bit arithmetic, for `i` in `0..152`:

```
f0 =  f[5i]   + 2295
f1 = (f[5i+1] + 2295) · 3
f2 = (f[5i+2] + 2295) · 9
f3 = (f[5i+3] + 2295) · 27
f4 = (f[5i+4] + 2295) · 81
f0 += f1 << 11
c[8i]   = low8(f0); f0 >>= 8
c[8i+1] = low8(f0); f0 >>= 8
f0 += f2 << 6
c[8i+2] = low8(f0); f0 >>= 8
c[8i+3] = low8(f0); f0 >>= 8
f0 += f3 << 1
c[8i+4] = low8(f0); f0 >>= 8
f0 += f4 << 4
c[8i+5] = low8(f0); f0 >>= 8
c[8i+6] = low8(f0); f0 >>= 8
c[8i+7] = low8(f0)
```

then `f0 = f[760] + 2295; c[1216] = low8(f0); c[1217] = low8(f0 >> 8)`.

`rq.Decode` (1218 bytes → element, `rq/encoding.go:63-127`), unsigned 32-bit wrapping arithmetic, for `i` in `0..152` with `c0..c7 = c[8i..8i+7]`:

```
c6 += c7 << 8
f4 = (103564·c6 + 405·(c5+1)) >> 19
c5 += c6 << 8
c5 −= (f4·81) << 4
c4 += c5 << 8
f3 = (9709·(c4+2)) >> 19
c4 −= (f3·27) << 1
c3 += c4 << 8
f2 = (233017·c3 + 910·(c2+2)) >> 19
c2 += c3 << 8
c2 −= (f2·9) << 6
c1 += c2 << 8
f1 = (21845·(c1+2) + 85·c0) >> 19
c1 −= (f1·3) << 3
c0 += c1 << 8
f0 = c0
f[5i+k] = freezeq((fk + 4591 − 2295) as i32)      for k = 0..4
```

then `f[760] = freezeq((c[1216] + (c[1217] << 8) + 4591 − 2295) as i32)`.

`rq.EncodeRounded` (rounded element → 1015 bytes, `rq/encoding.go:130-177`), signed 32-bit, for `i` in `0..253`:

```
f0 = (21846·(f[3i]   + 2295)) >> 16
f1 = (21846·(f[3i+1] + 2295)) >> 16
f2 = (21846·(f[3i+2] + 2295)) >> 16
f2 ·= 3;  f1 += f2 << 9;  f1 ·= 3;  f0 += f1 << 9
c[4i] = low8(f0); f0 >>= 8;  c[4i+1] = low8(f0); f0 >>= 8;  c[4i+2] = low8(f0); f0 >>= 8;  c[4i+3] = low8(f0)
```

then for the last two coefficients: `f0 = (21846·(f[759]+2295)) >> 16; f1 = (21846·(f[760]+2295)) >> 16; f1 ·= 3; f0 += f1 << 9; c[1012] = low8(f0); f0 >>= 8; c[1013] = low8(f0); f0 >>= 8; c[1014] = low8(f0)`.

`rq.DecodeRounded` (1015 bytes → element, `rq/encoding.go:180-231`), unsigned 32-bit wrapping, for `i` in `0..253` with `c0..c3 = c[4i..4i+3]`:

```
f2 = (14913081·c3 + 58254·c2 + 228·(c1+2)) >> 21
c2 += c3 << 8
c2 −= (f2·9) << 2
f1 = (89478485·c2 + 349525·c1 + 1365·(c0+1)) >> 21
c1 += c2 << 8
c1 −= (f1·3) << 1
c0 += c1 << 8
f0 = c0
f[3i+k] = freezeq((fk·3 + 4591 − 2295) as i32)     for k = 0..2
```

then with `c0, c1, c2 = c[1012..1014]`: `f1 = (89478485·c2 + 349525·c1 + 1365·(c0+1)) >> 21; c1 += c2 << 8; c1 −= (f1·3) << 1; c0 += c1 << 8; f0 = c0; f[759] = freezeq(f0·3 + 2296); f[760] = freezeq(f1·3 + 2296)`.

#### 5.10.4 Ring operations and key generation

**Multiplication** `rq.Mult(h, f ∈ R/q, g small)` (`rq/rq.go:79-102`): schoolbook product into 1521 coefficients, each accumulated as `acc = freezeq(acc + f[j]·g[i−j])`; then for `i = 1520` down to `761`: `fg[i−761] = freezeq(fg[i−761] + fg[i])` and `fg[i−760] = freezeq(fg[i−760] + fg[i])`; the result is the low 761 coefficients. `r3.Mult` is identical with `freeze3` and small inputs (`r3/r3.go:73-96`).

**Rounding** `Round3(h, f)`: `h[i] = ((21846·(f[i] + 2295) + 32768) >> 16)·3 − 2295` (`rq/rq.go:72-76`).

**Inverse in R/3** `r3.Reciprocal(r, s)` (`r3/r3.go:31-70`), a constant-time extended division:

```
f = 762 zeros; f[0] = −1; f[1] = −1; f[761] = 1          (the modulus)
g = 762 zeros; g[0..761) = s
d = 761; e = 761
u = 1524 zeros; v = 1524 zeros; v[0] = 1
repeat 1523 times:
    c = g[761] · f[761]
    g[k] = freeze3(g[k] − f[k]·c) for all k;   shift g up one position (g[k] = g[k−1], g[0] = 0)
    v[k] = freeze3(v[k] − u[k]·c) for all k;   shift v up one position
    e −= 1
    if e < d and g[761] != 0:  swap(e, d); swap(f, g); swap(u, v)
r[k] = u[761 + k] · f[761]     for k in 0..761
return 0 if d == 0, else −1 (s is not invertible)
```

**Inverse of 3·s in R/q** `rq.Reciprocal3(r, s)` (`rq/rq.go:31-70`): the same procedure over 16-bit values with `g[k] = 3·s[k]`, `c = freezeq(g[761] · recipq(f[761]))`, `freezeq` in place of `freeze3`, and the final scaling `r[k] = freezeq(u[761+k] · recipq(f[761]))`. Its return value is not checked by key generation.

**`GenerateKey(s)`** (`sntrup4591761.go:55-79`):

1. Repeat: `RandomSmall(g, s)`; `gr = r3.Reciprocal(g)`; until the reciprocal reports success.
2. `RandomTSmall(f, s)`.
3. `f3r = rq.Reciprocal3(f)`; `h = rq.Mult(f3r, g)`; `pk = rq.Encode(h)`.
4. `sk = zx.Encode(f) ‖ zx.Encode(gr) ‖ pk`.

#### 5.10.5 `Encapsulate(s, pk)`

(`sntrup4591761.go:82-117`) `RandomTSmall(r, s)`; `h = rq.Decode(pk)`; `c = Round3(rq.Mult(h, r))`; `H = SHA-512(zx.Encode(r))`; ciphertext = `H[0..32) ‖ rq.EncodeRounded(c)`; shared key = `H[32..64)`.

#### 5.10.6 `Decapsulate(cstr, sk)`

(`sntrup4591761.go:121-164`)

1. `f = zx.Decode(sk[0..191))`; `c = rq.DecodeRounded(cstr[32..))`; `t = rq.Mult(c, f)`.
2. `t3[i] = freeze3(freezeq(3·t[i]))`.
3. `gr = zx.Decode(sk[191..382))`; `r = r3.Mult(t3, gr)`.
4. `ok` = (number of non-zero coefficients of `r` is 286).
5. `h = rq.Decode(sk[382..))`; `hr = Round3(rq.Mult(h, r))`; `ok &=` (`hr[i] == c[i]` for all i).
6. `H = SHA-512(zx.Encode(r))`; `ok &=` (`H[0..32) == cstr[0..32)`).
7. Return `H[32..64)` and `ok` (1 or 0). All checks are constant-time. The mixing code treats `ok != 1` as `sntrup4591761: decapsulate failure` (`keyagreement.go:82-88`).

### 5.11 Root solving

#### 5.11.1 `Client.roots`

(`client.go:2157-2258`) Input: coefficients `a[0..mtot]`.

**When the client advertises solving** (PR flag bit 0 currently set):

1. `roots, err = solverrpc.Roots(a, F)`.
2. If `err` is `ErrSolverProcessExited`: clear flag bit 0 for all future PRs and fall through to the non-solving path for this run.
3. If `err != nil` (including `repeated roots`) or the number of roots is not `mtot`: set `triggeredBlame` on every active local peer and return `errTriggeredBlame`.
4. Sort the roots ascending. For every active local peer without an FP, create `FP(id, sid, 0, [minimal bytes of each root], seenSRs)`. Return the roots. (Each local solver-capable peer in the session publishes its own FP, sent together with its DC.)

**Otherwise** wait for a usable FP from any peer:

1. Receive FPs (expect 1 at first, deadline `min(recvSR, now + T)`). A receive error (unknown session, or `ErrSecretsRevealed`) is returned.
2. For each received FP whose identity has not been examined before: require exactly `mtot` roots; parse each as an unsigned big-endian integer and require `InField` and `IsRoot(root, a)`; drop exact duplicates; require the de-duplicated list to be in ascending order and to still have `mtot` entries. The first FP passing all checks supplies the roots. `SeenSlotReserves` is not examined.
3. If none qualified, expect one more FP than is currently present and repeat from step 1.

The loop has no exit on deadline: once `recvSR` has passed, the receive returns immediately and the loop spins until an FP qualifies, a secrets message appears, or the session is removed from the mixpool (Quirk Q5).

#### 5.11.2 The solver process

- Start: execute `SolverProcess` with no arguments, with pipes on stdin and stdout; create the RPC client on them; a watcher marks the process exited when it terminates (`solverrpc/rpc.go:55-81`). Start-up happens once per OS process; a failed start is permanent for that process.
- Each `Roots` call first returns the start error if any, then `ErrSolverProcessExited` if the child has exited, then performs the RPC. If the RPC fails and the child has exited, the error is replaced by `ErrSolverProcessExited` (`solverrpc/rpc.go:138-168`).
- Server semantics (`cmd/csppsolver/solver.go:59-74`, `solver/solver.go:41-103`): with fewer than two coefficients the call fails `too few coefficients`. Otherwise the polynomial is factored over `Z/F` with FLINT (`fmpz_mod_poly_factor`); for **each** irreducible factor, of any degree, the value `−(constant coefficient) mod F` is reported as a root together with the factor's exponent. `Solver.Roots` returns `RepeatedRoot` (and no roots) if any exponent is not 1; the client turns that into the error `repeated roots`.
- Consequence: the reply has exactly `mtot` roots only when the polynomial splits into `mtot` distinct linear factors. The client's count check in 5.11.1 step 3 is what detects a polynomial that does not split; the client does not verify the returned values.
- The order of returned roots is FLINT's factor order; the client always sorts.
- Other server failures: `failed to read fmpz` when a factor's constant coefficient cannot be parsed back from FLINT's base-16 string (`solver/solver.go:92-95`). A server-side error travels in the `net/rpc` response header's `Error` string and surfaces from `client.Call` as an `rpc.ServerError` with that text; the mixing client treats every non-nil error other than `ErrSolverProcessExited` identically (section 5.11.1 step 3).
- Coefficients and the modulus cross the cgo boundary as base-16 strings (`solver/solver.go:26,45-67`). The client's coefficients are already canonical residues; how FLINT treats a coefficient outside `[0, F)` was not examined.
- Concurrency: there is one `rpc.Client` for the process. Sessions of different pairings can call `Roots` at the same time; `net/rpc` multiplexes the calls on the single connection by sequence number and the Go server (`rpc.Server.ServeConn`) runs each request in its own goroutine, so replies can arrive out of order. A port that keeps the child process must match requests to replies by `Seq`.
- `solverProcess.Close` (invoked by the RPC client when the connection fails) closes the child's stdin, then stdout, then waits for the child, returning the first error of the three (`solverrpc/rpc.go:35-46`). Nothing in dcrwallet closes the client deliberately.
- The server registers the receiver type `Solver` under its type name and serves exactly one connection; when that connection ends `main` returns and the process exits with status 0 (`cmd/csppsolver/solver.go:76-90`). A registration failure panics.
- The `cspp/v2/solver` package also exports in-process `RootFactors` and `Roots` with the same semantics (`solver/solver.go:41-126`); dcrwallet does not link them (they need cgo and FLINT).
- The child ignores interrupt, terminate and hang-up signals and ends when its stdin is closed, i.e. when the wallet process exits.

#### 5.11.3 dcrwallet start-up checks

(`config.go:739-758`, `testsolver.go:18-35`) During configuration loading, only when `--mixing` is set:

1. If `--csppsolver` was given explicitly: `solverrpc.SolverProcess = value`; the solver must work.
2. Otherwise call `StartSolver()` with the default name `csppsolver`: on success the solver must work; on failure log the warning `Unable to start csppsolver; must rely on other peers publishing results` and continue without a solver.
3. If the solver must work: `StartSolver()`, then `Roots([1, 2], F)`. If the start fails, or the call's error is `io.ErrUnexpectedEOF` or `rpc.ErrShutdown` (reported as `rpc failed: <err>`), print `csppsolver process is not operating properly: <err>` to stderr and fail configuration loading. Any other result, including other errors, passes.

`NewClient` later calls `StartSolver()` again (a no-op returning the recorded outcome) to decide the PR flag.

### 5.12 Blame assignment (`blame`)

(`mixclient/blame.go:53-493`) Entered when a run ended with `errTriggeredBlame` or `ErrSecretsRevealed`. `peers` are the run's peers with the messages recorded on them during the run.

**Reveal phase**

1. Log `running blame assignment`. `deadline = now + T`.
2. Send with an empty mask (deadline as above): only local peers with `triggeredBlame` send, and they send their RS (with empty `SeenSecrets`). A send error is returned.
3. Receive RSs (expect 1) under the session context; the result's error is ignored. `rsHashes` = hashes of the RSs present.
4. For each active local peer **without** `triggeredBlame`: set its RS's `SeenSecrets = rsHashes`.
5. Send RS for all active local peers (deadline as above). A send error is returned. (For a peer that already sent in step 2 this is an identical message: the mixpool treats it as a duplicate, but `SubmitMixMessage` still hands it to the network backend a second time.)
6. Receive RSs (expect n, timeout `T`); record each on its peer. If the number of RSs is not n: log `received %d RSs for %d peers; blaming unresponsive peers`; blame every peer without an RS (`blaming %x for RS timeout`); return.
7. Sort the RSs into peer order.

**Verification phase** (holding `c.mu` until return). Each check that blames anyone returns the accumulated `blamedIdentities` at the end of its pass; later passes run only if earlier ones blamed nobody.

8. *Per peer, in peer order* (a peer that fails a check is blamed and skipped for the rest of this pass):
   1. No KE recorded → `blaming %x for missing messages`.
   2. `RS.Commitment() != KE.Commitment` → `blaming %x for false commitment, got %x want %x`.
   3. Any revealed SR message not in `[0, F)` → `blaming %x for SR message outside field`.
   4. Rebuild the run PRNG from the revealed seed and `kx = NewKX(prng)`; an error → `blaming %x for bad KX`.
   5. KE's ECDH key ≠ `kx`'s compressed ECDH public key, or KE's PQ key ≠ `kx`'s → `blaming %x for KE public keys not derived from their PRNG`.
   6. Append the KE's ECDH and PQ public keys to the session key lists; record `start_k`.
   7. Number of revealed SR messages or DC messages ≠ `mcount_k` → `blaming %x for bad message count`.
   8. Adopt the revealed SR messages (as integers) and DC messages as the peer's messages.
9. *Ciphertexts.* For each peer, `recovered[k] = kx_k.Encapsulate(prng_k, all PQ keys in peer order)` (continuing each peer's rebuilt PRNG directly after its key generation); an error blames the peer. Then for each peer: no CT recorded → `blaming %x for missing messages`; ciphertext count differs → `blaming %x for different ciphertexts count %d != %d`; any ciphertext differs → `blaming %x for different ciphertexts`.
10. *Shared SR messages.* Any SR message value revealed by more than one (peer, message) → every owner is blamed (`blaming %x for shared SR message`); a peer that repeats a value among its own messages is listed once per occurrence.
11. *SR mix.* For each peer: no SR recorded → `missing messages`; wrong SR dimensions → `blaming %x for wrong SR DC-mix dimensions`; compute `SharedSecrets` from the session ECDH keys and `Ciphertexts[q] = recovered[q][k]`: a `DecapsulateError` blames **this** peer (`blaming %x for unrecoverable secrets`), any other error aborts blame with that error; recompute each `SRMix` (with `my = start_k + j`) and compare every value with the published SR → mismatch `blaming %x for bad SR mix`.
12. If the run never obtained roots: log at error level `Blame failed: unknown cause of root solving error` and end the verification phase with no finding.
13. *DC mix.* Map each root's minimal bytes to its slot. For each peer: no DC recorded → `missing messages`; DC vector count ≠ `mcount_k` → `blaming %x for missing DC mix vectors`; for each own message, look its SR message up among the roots (absence aborts blame with `blame failed: no slot for message %v`), recompute `DCMix` and compare with the published vector → mismatch `blaming %x for bad DC mix`.
14. End of verification with no finding.

Details of the verification phase that affect the blamed set:

- The SR comparison of step 11 is `recomputed.Cmp(big-endian integer of the published bytes) != 0` (`blame.go:390-396`). The published value is **not** reduced modulo `F` and is not required to be minimal: leading zero bytes are accepted, but a published value `≥ F` that is congruent to the correct one is blamed as `bad SR mix`, although the run itself reduced it and computed correct power sums (section 4.10).
- The DC comparison of step 13 compares the whole recomputed vector with the whole published vector `DCNet[j]` (`Vec.Equals`: same length and same bytes), so a vector of the wrong length is `bad DC mix`. The root-to-slot map is keyed by each root's minimal bytes; were two roots equal, the higher index would win.
- Step 8 overwrites, on **every** peer object of the run including local ones, the PRNG (rebuilt from the revealed seed), the KX (regenerated from it), `srMsg` (parsed from the RS) and `dcMsg` (taken from the RS); step 9 advances each rebuilt PRNG by one encapsulation pass and replaces the KX's `PQCleartexts`; step 11 overwrites `srKP`/`dcKP`. For an honest local peer these equal the values it already held. The run's local peers are only cloned from afterwards with `freshGen = true`, which discards all of it.
- In step 8 the per-peer `start_k` list and the ECDH/PQ key lists are appended only for peers that pass sub-steps 1–5, so they are misaligned with peer indexes whenever an earlier peer was blamed; this is harmless because the pass then returns before they are used.
- Step 6 records each RS on its peer **before** the count test, and the count test compares the number of RSs returned by the mixpool with `len(peers)`.
- The logging of the blamed list (`blamed peers […]`) uses `Client.log` with the list as its only argument and happens after the fallback has run.
- Step 3's receive error is discarded. If it fails or returns nothing (session context already done, or the session vanished from the mixpool), `rsHashes` is empty and every non-triggering local peer publishes an RS with empty `SeenSecrets`, which other participants' fallback rule treats as a false failure accusation whenever their verification phase finds no culprit. `rsHashes` holds whatever RSs are in the pool at that moment, in mixpool (map) order, including those of the wallet's own triggering peers; the order is visible on the wire in `SeenSecrets` but nothing evaluates it.
- Step 4 assigns the **same** `rsHashes` slice to every non-triggering local peer's RS, and it does so through the worker pool with the result ignored; a peer whose RS was cleared by an earlier failed submission (section 5.13) is skipped.
- Step 5 returns `errNoActiveLocalPeers` (and blame ends with "failed blame assignment") when no active local peer holds an RS; step 2 cannot return that error because its mask is empty.
- The ordering of the three deferred actions at return is: release `c.mu`, then the fallback, then the blamed-list log (`blame.go:65-69,138-192`). The fallback is registered only after the RS-count test, so the RS-timeout exit of step 6 and the send-error exits of steps 2 and 5 never run it.
- Step 8 contains a seed-length test between sub-steps 2 and 3 (`blaming %x for bad seed size in RS message`, `blame.go:218-223`) that can never fire because the seed is a fixed 32-byte array, and a re-parse of the KE's ECDH key after sub-step 5 (log format `blaming %x for unparsable pubkey`, called without its argument, `blame.go:258-263`) that can never fail because the bytes have just been found equal to a freshly serialized valid key.
- Blame can be entered before the run recorded any remote KE: when peer agreement already exists and pairing completion fails with `ErrSecretsRevealed` (section 5.7 step 5), the run returns before step 9, so only local peers have a KE stored. If all n RSs then arrive, step 8.1 blames **every remote peer** for `missing messages`; if they do not, step 6 blames the peers without an RS. Likewise a reveal that happens before the CT, SR or DC of some peer was recorded blames that peer for `missing messages` in steps 9, 11 or 13 if it passed the earlier passes. Derived from the code (`client.go:1601-1610,1696-1700`, `blame.go:200-205`); untested upstream.
- Step 9's encapsulation error path blames without logging (`blame.go:290-294`).

**Fallback** (only when the verification phase ended with no finding and no error): blame every peer whose RS (a) references a hash that is not the hash of one of the session's received RSs — `blaming %x for referencing unknown RS` — or (b) references no other peer's RS (self-references do not count) — `blaming %x for false failure accusation`. If that blames nobody the result is `errBlameFailed`.

Whenever `blame` returns with a non-empty blamed list (including the RS-timeout exit of step 6), that list is logged at debug level, without the `sid=` prefix, as the error text `blamed peers [<hex id> <hex id> …]` (`blame.go:36-38,65-69`).

### 5.13 Message send scheduling (`sendLocalPeerMsgs`)

(`client.go:590-692`) Inputs: submit deadline, run, message mask (any of KE, CT, SR, DC, FP, CM, RS).

1. `now` = current time. For each active local peer in peer order: `sendTime = now + peer.jitter + U[0, 300 ms)`; the peer's effective mask is the given mask plus RS when the peer has `triggeredBlame`; queue, in the fixed order KE, CT, SR, FP, DC, CM, RS, each selected message the peer currently holds. All of one peer's messages share its `sendTime`.
2. If nothing was queued and the given mask was non-empty, return `errNoActiveLocalPeers`.
3. Stable-sort the queue by `sendTime`.
4. For each queued message: wait until its `sendTime`, or, if the session context is done, record the error `session cancelled: <ctx error>` once and skip the message; every later message goes through the same two-way wait, so it is normally skipped as well, but a message whose `sendTime` has already passed can still be sent because Go's `select` picks randomly between a ready context and a ready timer (`client.go:677-684`; Go-runtime-dependent). Then sign the message, compute its hash (under the shared hasher mutex), and call `wallet.SubmitMixMessage` with a context whose deadline is the given deadline (not derived from the session context). On a signing or submission error, clear that message on the peer and collect the error.
5. Return all collected errors joined.

A message cleared at step 4 makes the peer look, to the timeout logic, like a peer that never sent it.

### 5.14 Mixpool behaviour the client relies on

(Specified in full in the mixpool specification; summarized because the state machine's meaning depends on it.)

- `Receive(ctx, r)` (`mixpool/mixpool.go:926-1058`): fails immediately with `unknown session %x` when no message of that session ID is in the pool. Otherwise it blocks until messages of the requested type — or secrets messages, which always count — from at least `capacity` distinct identities are present, or until `ctx` is done; **it returns nil in both cases**, appending every matching message currently in the session (unsorted). If the request did not ask for secrets messages and the session contains one, the returned error is `ErrSecretsRevealed` (the requested messages are still appended).
- `AcceptMessage` returns nil for a message already in the pool; accepts at most one message of each type per identity per session; stores non-KE messages whose sender's KE is not yet known as orphans; requires the message hash to have been computed; requires `Run == 0`.
- `SubmitMixMessage` in dcrwallet accepts the message into the wallet's mixpool, then publishes it through the network backend; if either fails the message is removed from the mixpool again and the error returned; with no backend the error is `errors.NoPeers` (`wallet/mixing.go:99-118`).
- `RemoveSession(sid)` removes the session's messages and, for a completed session, its PRs. `RemoveConfirmedSessions()` removes every session in which the number of CMs equals the number of PRs, with its PRs.
- `Received` (`mixpool/mixpool.go:893-910`) has one slice per message type. The slice to fill is selected by being non-nil, and the expected count is its **capacity** (`cap`), not its length; exactly one slice may have non-zero capacity, otherwise `Receive` fails with `mixpool: exactly one Received slice must have non-zero capacity` (`mixpool/mixpool.go:967-969`). The client reuses one `Received` across the stages of a run, setting the previous stage's slice to nil and allocating the next with capacity n (or 1 for the RS wait and the first FP wait); results are appended, so a reused slice must be truncated first (the FP loop does this, `client.go:2210`).
- `ErrSecretsRevealed` has the text `secrets revealed by peer` (`mixpool/errors.go:118`); it appears in the client's `Aborting session for failed blame assignment: %v` and `Run error: %v` lines when it escapes.
- The `mixclient.Wallet` adapter in dcrwallet (owned by the wallet-mixing specification; listed here because its results flow through the client unchanged): `SubmitMixMessage` calls `AcceptMessage(msg, mixpool.ZeroSource)` and on any failure of acceptance or publication calls `mixpool.RemoveMessage(msg)`; `SignInput` returns `previous output is not P2PKH` (kind `Invalid`) unless the previous script yields exactly one secp256k1 ECDSA P2PKH address, and otherwise the address-manager or signing error (wrapped with op `sign.SignatureScript`); `PublishTransaction` logs `Failed to publish mix transaction: %v` at error level on the wallet's own logger and returns the error; `Wallet.dicemix` returns `mixing client is not running` (kind `Invalid`) when no client has been stored yet (`wallet/mixing.go:99-169,591-597`).

---

## 6. Constants, limits and defaults

| Name | Value | Source | What depends on it |
|---|---|---|---|
| `MinPeers` | 4 | `dcrd/mixing@v0.7.5/client.go:9` | Run viability; alternate-session minimum; abort after blame/size limiting |
| `MaxPeers` | 64 | `limits.go:26` | Size limiting; mixpool caps on CT/SR/DC/CM/FP/RS reference counts |
| `MaxMcount` | 16 | `limits.go:30` | Mixpool cap on PR `MessageCount`, SR/DC rows, RS messages |
| `MaxMtot` | 1024 | `limits.go:34` | Size limiting; mixpool caps on SR/DC columns and FP roots |
| `MaxMixTxSerializeSize` | 100000 | `limits.go:38` | Size limiting (estimate must be strictly below); mixpool cap on CM transaction size |
| `MaxMixAmount` | 525 000 000 000 000 | `limits.go:41` | Mixpool cap on PR `MixAmount` |
| `Msize` / `wire.MixMsgSize` | 20 | `vec.go:16`, `wire@v1.8.0/mixvect.go:12` | Mixed message size |
| `F` | 2^127 − 1 | `field.go:15` | Slot reservation field |
| field element length | 16 bytes | `mixclient/client.go:2583` | Constant-time slot search padding |
| `PRFlagCanSolveRoots` | 0x01 | `flags.go:11` | PR `Flags` |
| `ScriptClassP2PKHv0` | `P2PKH-secp256k1-v0` | `scriptclass.go:15` | PR `ScriptClass`, pairing ID |
| `pairingVersion` | 3 (testnet3: 4) | `mixclient/client.go:41-42` | PR `PairingFlags`, pairing ID |
| stage timeout `T` | 30 s (testnet3: 10 s) | `client.go:45,50` | All deadlines; PR submit deadline; blame waits; warm-up |
| `maxJitter` | `T/10`: 3 s (1 s) | `client.go:46,51` | PR timing window |
| `msgJitter` | 300 ms | `client.go:47` | Per-send random delay; publish delay |
| `peerJitter` | `maxJitter − msgJitter`: 2.7 s (700 ms) | `client.go:48,52` | Per-peer fixed delay |
| epoch `E` | 10 min (testnet3: 3 min) | `mixpool/mixpool.go:280,289-292` | Session cadence, session ID, pairing deadline |
| warm-up | first epoch boundary + `T` + 2 s | `client.go:883-908` | Earliest time `Dicemix` proceeds |
| completed-session removal delay | 10 s | `client.go:1161-1166` | Mixpool retention of a mixed session |
| alternate-session attempt limit | more than 2 per session ID aborts | `client.go:1306-1310` | Session re-formation |
| worker pool size / work queue buffer | number of CPUs | `client.go:445,504-508` | Parallelism of per-peer work |
| tag: signature | `decred-mix-signature` | `signatures.go:17` | Section 4.3 |
| tag: session | `decred-mix-session` | `sid.go:23` | Section 4.5 |
| tag: UTXO proof | `mixpr-utxoproof` + `,` + `P2PKH(EC-Schnorr-DCRv0)` + `,` | `utxoproof/utxoproof.go:18-24` | Section 4.6 |
| chacha20 seed size | 32 | `internal/chacha20prng/prng.go:15` | PRNGs, RS `Seed` |
| sntrup sizes | pk 1218, sk 1600, ct 1047, key 32 | `sntrup4591761@9e0f3af2/sntrup4591761.go:22-27` | KE `PQPK`, CT ciphertexts |
| sntrup parameters | p 761, q 4591, weight 286 | `zx/random.go:64-98`, `rq/rq.go:6` | KEM |
| `redeemP2PKHv0SigScriptSize` | 108 | `mixclient/limits.go:17` | Size estimate |
| `p2pkhv0PkScriptSize` | 25 | `mixclient/limits.go:18` | Size estimate |
| estimated input size | 166 | `mixclient/limits.go:21-35` | Size estimate |
| `MaxExpiry` | tip + 60 min / block time + 1 | `expiry.go:15-18` | Upper bound for PR expiry; dcrwallet uses this − 2 |
| `wire.MixVersion` | 10 | `wire@v1.8.0/protocol.go:56` | Encoding protocol version for all hashes |
| `wire.MaxMixPeers` | 512 | `msgmixkeyexchange.go:22` | Wire cap on `Seen*` counts, CT count, SR `kpcount`; initial PR set trim |
| `wire.MaxMixMcount` | 1024 | `msgmixslotreserve.go:21` | Wire cap on SR rows, DC dimensions, FP roots, RS messages |
| `wire.MaxMixFieldValLen` | 32 | `msgmixslotreserve.go:25` | Wire cap on a field value's byte length |
| `wire.MaxMixPairReqScriptClassLen` | 32 | `msgmixpairreq.go:19` | PR |
| `wire.MaxMixPairReqUTXOs` | 512 | `msgmixpairreq.go:25` | PR |
| `wire.MaxMixPairReqUTXOScriptLen` / `PubKeyLen` / `SignatureLen` | 16384 / 33 / 64 | `msgmixpairreq.go:29-37` | PR UTXO |
| max payload: PR, KE, CT, SR, FP, DC, CM, RS | 8476848, 17815, 552584, 17318030, 49291, 20988047, 1016520, 70831 | each message's `MaxPayloadLength` | P2P framing |
| `SolverProcess` default | `csppsolver` | `cspp/v2@v2.4.0/solverrpc/rpc.go:15` | Solver discovery through `PATH` |
| mixpool `earlyKEDuration` / `lateKEDuration` | 5 s / 20 min | `mixpool/mixpool.go:33-34` | When a KE's stated epoch is acceptable |
| mixpool `strikeLimit` | 2 | `mixpool/observer.go:18` | `ExcludePRs` |
| mixpool messages per identity | 896 (7 × 2 × 64) | `mixpool/mixpool.go:46-51` | Upper bound on a peer's total accepted messages |
| script flags for merged coinjoin validation | `DiscourageUpgradableNops`, `VerifyCleanStack`, `VerifyCheckLockTimeVerify`, `VerifyCheckSequenceVerify`, `VerifyTreasury` | `mixclient/client.go:2291-2296` | Section 5.8.6 |
| opcodes | `OP_DUP` 0x76, `OP_HASH160` 0xa9, `OP_DATA_20` 0x14, `OP_EQUALVERIFY` 0x88, `OP_CHECKSIG` 0xac, `OP_SSGEN` 0xbb, `OP_SSRTX` 0xbc, `OP_TGEN` 0xc3 | `dcrd/txscript/v4@v4.1.2/opcode.go` | Scripts in sections 4.11 and 5.8 |

---

## 7. Concurrency and lifecycle

### 7.1 State

| Object | Lifetime | Contents |
|---|---|---|
| `Client` | wallet process | atomics `prFlags`, `stopping` flag; `pendingPairings: map[pairing bytes → pendingPairing]` and `height`, both guarded by `mu`; `warming` channel; work queue; `pairingWG`, `runWG`; shared BLAKE-256 hasher with its own mutex; timing parameters; `stopping` channel. |
| `pendingPairing` | while any local request with that pairing ID waits | `localPeers: map[id → peer]`, pairing ID. |
| `pairedSessions` | one pairing × one epoch | clone map of local peers, pairing ID, `runs` (append-only list of `sessionRun`), epoch, deadlines, `peerAgreement` flag and `peerAgreementRunIdx`, a once-guard for releasing `pairingWG`. |
| `sessionRun` | one session ID | sid, index, `mtot`, `freshGen`, `localPeers`, `prs`, `kes`, `peers` (peer order), `mcounts`, `roots`, the finished coinjoin, logger. |
| `peer` | see 7.2 | client, fixed `jitter`, result channel (capacity 1), identity key pair, PR, `CoinJoin` pointer, `remote` flag, `done` channel, and the per-run state: seed, PRNG, KX, SR messages, DC messages, own KE/CT/SR/FP/DC/CM/RS, `myVk`, `myStart`, shared secrets, computed mixes, `triggeredBlame`. |

### 7.2 Peer cloning

`cloneLocalPeer(freshGen)` (`client.go:260-277`) copies the peer (sharing the result channel, `done`, identity keys, PR and `CoinJoin` pointer) and resets all per-run state. When `freshGen` is false it carries over the seed, the **same PRNG object** (with its advanced stream position), the KX, the SR messages and the DC messages; when true it carries nothing and the caller generates new ones.

Clones are made: by the ticker for each epoch's `pairedSessions` (non-fresh); at the start of every run from the previous run's clone (fresh only after a reveal); and by the exit handler when returning peers to the pending set. Consequently a request keeps the same ECDH/PQ keys, SR messages and mixed-output addresses across epochs and across re-formed sessions until a secrets reveal forces regeneration, at which point the previously generated addresses are abandoned and the `GenFunc` is called again.

### 7.3 Tasks

| Task | Count | Role |
|---|---|---|
| `Run` stopper | 1 | Waits for the caller's context, then marks the client stopping (close `stopping` once). |
| Epoch ticker | 1 | Section 5.4. |
| Peer workers | number of CPUs | Execute per-peer closures taken from the work queue and post each result on that item's channel. `forLocalPeers` enqueues one item per active local peer (in peer order), or records the context error for an item if the context ends before it can be queued, then collects all results and joins the errors; with no active local peer it returns `errNoActiveLocalPeers` (`client.go:532-578`). |
| `pairSession` | one per pending pairing per epoch | Sections 5.6–5.7. Sessions of different pairings run concurrently; a session of the previous epoch may still be running its later stages when the next epoch's session for the same pairing starts (they hold disjoint sets of pending peers, except as noted in Quirk Q14). |
| KE waiters | up to one per candidate run during pairing completion | Section 5.6.6; all are cancelled and joined before `completePairing` returns. |
| Delayed session removal | one per mixed session | Sleeps 10 s, then removes the session from the mixpool. Not tracked by any wait group. |
| Solver exit watcher | 1 per process | Marks the solver as exited. |

The three long-lived task kinds run in an error group rooted in a **background** context, not the caller's: cancelling the context passed to `Run` does not cancel running sessions.

### 7.4 Locking

- `Client.mu` guards `pendingPairings`, `height` and the replacement of `warming`. It is held: by `Dicemix` for insertion/removal; by `ExpireMessages`; by the ticker from loop step 5 to loop step 8 of section 5.4 in every iteration (including its calls into the mixpool); by the run when removing paired peers from the pending set; by the exit handler when re-queuing; by blame assignment for its whole verification phase (`blame.go:191-192`); by `Run` at start and at exit.
- The mixpool has its own lock. The client calls mixpool methods both with and without `Client.mu` held; the mixpool never calls back into the client, so the order is always client → mixpool.
- The shared hasher mutex is taken only for the duration of one hash computation and never while taking another lock.
- `prFlags` is an atomic read at PR creation and an atomic clear when the solver process is found dead.
- Per-run peer state is touched only by the owning `pairSession` task and by workers executing closures for it; the closures for one step run concurrently for different peers and must not share mutable state without their own lock (the SR step guards its shared blamed set with a local mutex, `client.go:1810-1811`).

### 7.5 Start-up and shutdown ordering

1. `Run` (re)opens `warming` if it was closed, then starts the stopper, the ticker and the workers.
2. `Dicemix` calls block on `warming` until the ticker has passed the first epoch boundary plus `T + 2 s`.
3. When the caller's context ends: `stopping` is closed. The ticker notices while it is waiting for an epoch boundary, or the next time it starts such a wait (it does not check during the warm-up sleep or while starting sessions); it then waits for **all running sessions to finish** (`runWG`) and returns `ErrStopping`. That cancels the error-group context, the workers exit, and `Run` sends `ErrStopping` to every local peer still pending and returns `ErrStopping`.
4. No new session is started after `stopping` is closed. Sessions already running continue through all their stages, bounded by their own deadline (the next epoch boundary); shutdown can therefore take up to one epoch.
5. A stopped client cannot be run again: the stopping state is never reset.

### 7.6 Ordering guarantees visible to callers and peers

- A `Dicemix` call receives at most one result; later results are dropped (the channel write is non-blocking).
- For one local peer, stage messages are submitted in protocol order KE, CT, SR, (FP,) DC, CM; an RS is submitted only after a failure. Within one send step a peer's messages go out together after `jitter + U[0, 300 ms)`.
- A peer never sends a stage message before it has received that run's complete set of the previous stage's messages from all n peers; its `Seen*` array lists exactly those n hashes in peer order.
- `nil` is delivered to `Dicemix` only after `PublishTransaction` returned nil.
- At most one session per pairing is *pairing* at any time (the ticker waits for the previous epoch's pairing phase before starting the next).

---

## 8. Errors

### 8.1 Returned by `Dicemix`

| Error | Text | When |
|---|---|---|
| context error | `context canceled` / `context deadline exceeded` | Caller's context ended while warming, delaying, or waiting. |
| wire constructor error | e.g. `too many input UTXOs [len %d, max %d]` | PR could not be built. |
| generator error | wallet-defined, or `gen returned wrong message count` | `genDicemixKeys` failed (at submission, or when regenerating after a reveal). |
| submission error | mixpool rule error (e.g. fee, dust, expiry, UTXO proof), network backend error, or the wallet's no-peers error | PR was rejected locally or could not be broadcast. |
| `expiredPRErr` | `mixing pair request <hash> by <id hex> expired` | PR left the mixpool while the request was pending (expiry, or removal for any other reason). |
| `ErrStopping` | `mixing client is stopping` | Client shut down while the request was pending. |
| `testPeerBlamedError` | `peer removed during blame assignment` | The local peer itself was blamed and excluded. |
| signing error | wallet-defined | `SignInput` failed at confirmation. |
| nil | | Coinjoin published. |

How the wallet maps these to RPC/gRPC results and log lines belongs to the wallet-mixing specification. Not every failure produces a result: a session that ends with `Run error`, an aborted re-formation, or `errOnlyKEsBroadcasted` silently returns its local peers to the pending set for the next epoch.

### 8.2 Returned by `Run`

`ErrStopping` after a clean shutdown; an error from `Observer.CheckPrevEpoch` if the observer fails (its only error source is `MsgMixPairReq.Pairing` failing on a stored PR, `mixpool/observer.go:160-163,296-299,309-313`). In the latter case the ticker returns immediately and the error group cancels its context — the workers exit and sessions still running see a cancelled context — but `Run` itself does not return until the caller's context is also done, because the stopper task of section 7.3 only returns then and `errgroup.Wait` waits for it (`mixclient/client.go:495-509`). Until then the client is inert: no further epochs are processed and pending `Dicemix` calls keep waiting. (Quirk Q18 describes a nil return path that is unreachable in practice.)

### 8.3 Internal results

Listed in section 3.2. They never reach callers except through log lines.

### 8.4 Log output

All client log lines go to the logger given to `SetLogger` (dcrwallet: subsystem `MIXC`) at **debug** level, except `Run error: %v` and `Blame failed: unknown cause of root solving error`, which are **error** level. Formats are quoted throughout section 5; `%x` applied to a list of identities prints `[` + hex identities separated by one space + `]`; `%s`/`%v` of a hash prints the byte-reversed hex form. Joined errors print one error per line. The mixpool logs separately (subsystem `MIXP`). Start-up solver messages are in section 5.11.3.

### 8.5 Wire-level errors of the mix messages

All are `wire.MessageError` values built by `messageError(op, code, text)`, where `op` is `"<Type>.BtcDecode"`, `"<Type>.BtcEncode"` or `"NewMsgMixPairReq"`. Decode errors are what a peer or dcrd reports for a malformed message; encode errors reach the client through `SubmitMixMessage` (section 5.13 step 4 then clears the message on the peer). Read errors of the underlying stream (short data) are returned unchanged. Encode-side checks are skipped when the writer is a hasher, except where noted.

| Message | Direction | Condition | Code | Text |
|---|---|---|---|---|
| all eight | both | protocol version < 10 | `ErrMsgInvalidForPVer` | `%s message invalid for protocol version %d` (command, version) |
| KE, CT, SR, FP, DC, CM, RS | decode | `Seen*` count > 512 | `ErrTooManyPrevMixMsgs` | `too many previous referenced messages [count %v, max %v]` |
| KE, CT, SR, FP, DC, CM, RS | encode | `Seen*` length > 512 (RS: also when hashing) | `ErrTooManyPrevMixMsgs` | same text |
| PR | decode | `MixAmount < 0` | `ErrInvalidMsg` | `mixing pair request contains negative mixed amount` |
| PR | decode | script class longer than 32 bytes or not strict ASCII | from `ReadAsciiVarString` | (wire specification) |
| PR | decode | `InputValue < 0` | `ErrInvalidMsg` | `mixing pair request contains negative input value` |
| PR | decode | UTXO count > 512 | `ErrTooManyMixPairReqUTXOs` | `too many UTXOs in message [count %v, max %v]` |
| PR | decode | UTXO script / public key / signature longer than 16384 / 33 / 64 | from `ReadVarBytes` (field names `MixPairReqUTXO.Script`, `MixPairReqUTXO.PubKey`, `MixPairReqUTXO.Signature`, `msgmixpairreq.go:167-181`) | (wire specification) |
| PR | decode | change marker not 0 or 1 | `ErrInvalidMsg` | `invalid change TxOut encoding` |
| PR | encode (also when hashing), constructor | script class longer than 32 bytes | `ErrMixPairReqScriptClassTooLong` | `script class length is too long [len %d, max %d]` |
| PR | encode (also when hashing), constructor | script class not strict ASCII | `ErrMalformedStrictString` | `script class string is not strict ASCII` |
| PR | encode | UTXO count > 512 | `ErrTooManyMixPairReqUTXOs` | `too many UTXOs in message [%v]` |
| PR | constructor | UTXO count > 512 | `ErrTooManyMixPairReqUTXOs` | `too many input UTXOs [len %d, max %d]` |
| PR | encode | UTXO script / public key / signature too long | `ErrVarBytesTooLong` | `UTXO script is too long [len %v, max %v]`, `UTXO public key is too long [len %v, max %v]`, `UTXO signature is too long [len %v, max %v]` |
| CT | encode | `len(Ciphertexts) != len(SeenKeyExchanges)` | `ErrInvalidMsg` | `differing counts of ciphertexts (%d) and seen key exchange messages (%d)` |
| SR | both | row count 0 | `ErrInvalidMsg` | `too few mixed messages [%v]` |
| SR | both | row count > 1024 | `ErrInvalidMsg` | `too many total mixed messages [%v]` |
| SR | both | column count 0 | `ErrInvalidMsg` | `too few mixing peers [%v]` |
| SR | both | column count > 512 | `ErrInvalidMsg` | decode: `too many mixing peers [count %v, max %v]`; encode: `too many mixing peers [%v]` |
| SR | encode | a row whose length differs from row 0's | `ErrInvalidMsg` | `invalid matrix dimensions` |
| SR | encode | a value longer than 32 bytes | `ErrInvalidMsg` | `value exceeds bytes necessary to represent number in field` |
| SR | decode | a value longer than 32 bytes | from `ReadVarBytes` (field name `slot reservation field value`) | (wire specification) |
| FP | decode | root count > 1024 | `ErrInvalidMsg` | `too many roots in message [count %v, max %v]` |
| FP | encode | root count > 1024 | `ErrInvalidMsg` | `too many solutions to factored polynomial [count %v, max %v]` |
| FP | encode | a root longer than 32 bytes | `ErrInvalidMsg` | `root exceeds bytes necessary to represent number in field` |
| FP | decode | a root longer than 32 bytes | from `ReadVarBytes` (field name `MixFactoredPoly.Roots`) | (wire specification) |
| DC | encode | zero vectors | `ErrInvalidMsg` | `too few mixed messages [%v]` |
| DC | encode | more than 1024 vectors | `ErrInvalidMsg` | `too many total mixed messages [%v]` |
| DC | decode | `x > 1024` or `y > 1024` (tested after `x`, `y` and `msize` have all been read) | `ErrInvalidMsg` | `DC-net mix vector dimensions are too large for maximum message count` |
| DC, RS | decode | `msize != 20` | `ErrInvalidMsg` | `mixed message length must be %d [got: %d]` |
| RS | decode | SR message count > 1024 | `ErrInvalidMsg` | `too many total mixed messages [count %v, max %v]` |
| RS | decode | an SR message longer than 32 bytes | from `ReadVarBytes` (field name `slot reservation mixed message`) | (wire specification) |
| RS | both (encode: also when hashing) | more than 1024 DC messages (decode: tested after `msize` has been read) | `ErrInvalidMsg` | `DC-net mix vector dimensions are too large for maximum message count` |
| CM | decode | embedded transaction fails `MsgTx.BtcDecode` | from the transaction decoder | (wire specification) |

Asymmetries a port must keep in mind: a DC with zero vectors decodes (leaving `DCNet` nil; only `x = 0` is on the wire) but cannot be encoded; the DC encoder writes `y = len(DCNet[0])` and then every element of every row without checking that the rows have equal length, so a jagged matrix would produce a mis-framed message (the client never builds one); the RS encoder checks neither the number nor the length of `SlotReserveMsgs`, only the decoder does (`dcrd/wire@v1.8.0/msgmixdcnet.go:130-248`, `msgmixsecrets.go:36-164,233-275`, `msgmixslotreserve.go:46-120,177-249`, `msgmixfactoredpoly.go:33-87,144-197`, `msgmixciphertexts.go:34-78,135-174`, `msgmixpairreq.go:112-216,273-382,483-523`).

### 8.6 Panics

None is reachable through the client's own call sequences with well-formed inputs; a port should turn each into an internal error or a debug assertion.

| Where | Condition | Text |
|---|---|---|
| `chacha20prng.New` | seed length ≠ 32 | `chacha20prng: bad seed length N` (`internal/chacha20prng/prng.go:25-28`) |
| `Vec.Xor` / `XorVectors` | differing lengths; `XorVectors` on an empty list indexes element 0 | `dcnet: vectors do not share identical dimensions` (`vec.go:59-62,72-78`) |
| `DCMix` | message length ≠ 20 | `m is not len Msize` (`dcnet.go:165-168`) |
| `SharedSecrets` | internal XOR helper given unequal lengths (cannot happen: all operands are 32 bytes) | `dcnet: different lengths in xor` (`keyagreement.go:180-183`) |
| `cloneLocalPeer` | called on a remote placeholder | `cloneLocalPeer: remote peer` (`mixclient/client.go:261-263`) |
| every wire `WriteHash` | hasher whose digest size is not 32 | `hasher type %T has invalid Size() for chainhash.Hash` (e.g. `wire@v1.8.0/msgmixciphertexts.go:118-122`) |
| `CoinJoin.AddInput` | empty `prevScript` (reads byte 0 unconditionally) | Go index-out-of-range runtime panic (`mixclient/coinjoin.go:100`) |
| `constantTimeOutputSearch` | no generated scripts while the transaction has an output with the mixed value and script version 0 (`mcount = 0`; a PR with `MessageCount = 0` is rejected by the mixpool first, `mixpool/mixpool.go:1415-1416`) | Go index-out-of-range runtime panic (`mixclient/coinjoin.go:222`) |
| `csppsolver` `main` | RPC registration failure | the registration error (`cspp/v2@v2.4.0/cmd/csppsolver/solver.go:84-88`) |

---

## 9. Compatibility-critical behaviours

A drop-in replacement mixes with unmodified dcrwallet peers through unmodified dcrd nodes. Each item below is observable by, or required for agreement with, another participant.

1. **Message bytes.** The eight message encodings of section 4.2, including field order, varint forms, the absent `y`/`msize` when a vector list is empty, and minimal big-endian field values (zero as a zero-length value). The message hash covers these bytes, and every later stage references that hash.
2. **Signatures.** The three-step hash construction of section 4.3 (tag strings, commas, lowercase hex, decimal run, zero sid for PRs) and EC-Schnorr-DCRv0 with the deterministic nonce. dcrd and every wallet mixpool drop a message whose signature fails.
3. **UTXO proofs.** Section 4.6, bound to the PR expiry. dcrd bans peers relaying a PR with an invalid proof (`mixpool/errors.go:93`).
4. **PR field values and pairing ID.** Script class `P2PKH-secp256k1-v0`, transaction version 1, lock time 0, pairing flags 3 (4 on testnet3), flag bit 0 only when roots can actually be solved, opcode rule for stake-tagged previous outputs. A different value in any pairing field silently partitions the wallet from all other mixers.
5. **Epoch arithmetic.** Epoch boundaries are multiples of `E` in Unix time; KE `Epoch` is that boundary in seconds; the same epoch value is used for every re-formed session started from that boundary. Mixpools reject KEs that are early by more than 5 s, late by more than 20 min, or not on a whole minute.
6. **Session ID and peer order.** Section 4.5, including the big-endian epoch and the XOR ordering. KE `SeenPRs` must be in this order and `Pos` must be the sender's index, or the KE is rejected (`sid.go:93-122`).
7. **Run is always 0.** A re-formed session is a new session ID. Non-zero runs are rejected by every mixpool.
8. **Selection of the initial PR set.** `CompatiblePRs` (pairing match, lower-expiry double spends removed), minus `ExcludePRs`, trimmed to 512. Agreement on the first session ID depends on every wallet computing the same set from the same mixpool contents.
9. **Run PRNG consumption.** The exact byte consumption of section 4.8: the ECDH scalar, the sntrup key generation loop, then one encapsulation per peer in peer order including the own position. Other peers recompute all of it from the revealed seed and blame any difference.
10. **sntrup4591761 bit-exactness.** Key generation, encapsulation, decapsulation and all four encodings as in section 5.10 (the 2017 round-1 scheme with SHA-512 confirmation, not `sntrup761`).
11. **Hybrid shared key and pair PRNG.** Section 4.9: BLAKE-256 of the ECDH x-coordinate XOR both KEM keys (one for the self pair), the 68-byte seed preimage with little-endian counter, the nonce rule, 32 bytes of SR secret followed by `20·mtot` pad bytes.
12. **Each peer encapsulates to itself**, and CT carries exactly n ciphertexts in peer order; any other count gets the sender excluded by every peer.
13. **SR vectors.** `mcount × mtot` values `m^(c+1) + pad[c] mod F` with the sign convention of section 5.9.2; any other dimensions get the sender excluded.
14. **Coefficient derivation and slot order.** Newton identities of section 5.9.3 over `F`; roots sorted ascending as integers; slot = index. All peers must place their message in the same slot numbering.
15. **FP messages.** A solver-capable peer publishes the sorted roots (minimal bytes) with its DC. Non-solving peers accept only an FP with exactly `mtot` in-field roots that are roots of the polynomial, distinct and ascending.
16. **DC vectors.** `mcount` vectors of `mtot` 20-byte elements; pad XOR per section 5.9.5; own message at its slot.
17. **Coinjoin transaction.** Version, lock time, expiry, input fields (sequence `0xffffffff`), the output set (all change outputs plus `mtot` P2PKH outputs of `mixValue`), and the sort of section 5.8.3. The transaction hash must be identical on all peers or the CM is rejected and its sender excluded.
18. **CM contents and signature merging.** Own inputs signed with standard P2PKH `SigHashAll` scripts; only scripts on inputs the CM sender contributed are used; the merged transaction must pass the script engine with the listed flags, judged against the public keys declared in the PRs.
19. **Stage timing.** Deadlines at `begin + k·T`; CMs are not sent before `begin + 5T`; each send is delayed by at most `peerJitter + msgJitter`. A peer whose message is not in the others' mixpools by the stage deadline is excluded from the re-formed session.
20. **`Seen*` references.** Each stage message lists the n hashes of the previous stage in peer order. At this pin the mixpool does not look at the contents of `Seen*` in CT/SR/FP/DC/CM/RS messages: it checks only their count (section 6) and orphans a non-KE message solely when the sender's KE for that session ID is not yet in the pool (`mixpool/mixpool.go:1312-1331`). For a KE it resolves `SeenPRs` against known PRs, requires `Pos < len(SeenPRs)`, requires all known referenced PRs to share one pairing ID and the PR at `Pos` to carry the KE's identity, and orphans the KE (returning `MissingOwnPRError`) when the PR at `Pos` is unknown (`mixpool/mixpool.go:1675,1732-1782`); `ValidateSession` checks the order (item 6). The `Seen*` hashes are nevertheless covered by the message hash and signature, and the client's blame fallback reads RS `SeenSecrets` (section 5.12), so they must be reproduced exactly.
21. **Timeout exclusion rule.** On a short stage, exactly the peers with no message of that stage are excluded, and the next session is `SortPRsForSession(remaining PRs, same epoch)`. All honest peers must compute the same exclusion set to meet again.
22. **In-run exclusion rules** (no reveal): unparsable ECDH key; wrong ciphertext count; wrong SR dimensions; wrong DC dimensions; CM for a different transaction hash; inputs whose signatures fail.
23. **Size limiting**, including the truncation behaviour of step 4 in section 5.6.4.
24. **Alternate session formation**, section 5.6.7: unresponsive detection, the sighting counts, the `totalPRs / 2` threshold, the removal order (count, then hash), the termination condition, and the limit of two retries per session ID.
25. **Secrets reveal protocol.** When to reveal (shared-key derivation failure, own SR message missing from the roots, solver failure or wrong root count, own output missing from the coinjoin, or any RS seen during the run); an initiator's RS has empty `SeenSecrets`, a responder's lists the RS hashes it saw; the RS reveals the seed, SR messages and DC messages committed to in the KE.
26. **Blame verification.** The checks and their order in section 5.12, the fallback rule, and the resulting exclusion set. A peer that computes a different set forms a different session ID from everyone else.
27. **Regeneration after a reveal.** After secrets were revealed, a peer must use a new seed, new KX keys, new SR messages and new mixed addresses in the next session; without a reveal it reuses them (peers do not check reuse, but reusing revealed values links the outputs to the identity).
28. **Solver capability flag.** Cleared for future PRs once the solver process has died; the session still needs at least one PR with the flag or it stops after the KE stage.
29. **Unresponsive-PR removal.** PRs whose identity sent no KE in an epoch where at least `MinPeers` KEs of that pairing were seen are removed from the local mixpool, which shapes the next epoch's PR set.
30. **Completed-session retention.** Messages of a mixed session stay in the wallet's mixpool for 10 s so that SPV peers requesting the CMs are not answered `notfound` (`client.go:1157-1166`).
31. **KE-only sessions.** With fewer than `MinPeers` compatible PRs, or no solver among them, the wallet still broadcasts its KE each epoch; remote nodes rely on KEs to learn of PRs they are missing.
32. **`Dicemix` contract toward the wallet.** Blocking semantics, one result per call, the error identities and texts of section 8.1, `MixedIndices` after success, `Tx()` returning the published transaction (same transaction hash for every local request; fully signed only for one of them, Quirk Q41).
33. **Solver process contract** (only if the external `csppsolver` is kept): `--csppsolver` semantics, start-up probing, warning and failure texts of section 5.11.3, the `net/rpc`/gob protocol of section 4.13, one process per wallet process started with no arguments.
34. **Log lines** of the `MIXC` subsystem at debug/error level, if log-compatibility is in scope for the project.
35. **Confirmed-session sweep.** At every epoch tick, before sessions are formed, every session holding as many CMs as PRs is removed from the wallet's mixpool together with its PRs, then height-based expiry runs (section 5.4, loop step 6; Quirk Q34). The next epoch's candidate PR set, and therefore the session ID every participant must agree on, is computed after this sweep.
36. **Decode acceptance of the eight messages.** The limits and structural rules of section 8.5 (decode rows) decide which messages a wallet accepts from the network; accepting more or less than Go changes which peers are seen as having sent a stage message, and therefore the timeout exclusion set.
37. **Blame comparisons on raw values.** Commitment equality, ciphertext byte equality, the unreduced integer comparison of SR values (Quirk Q35) and whole-vector DC equality (section 5.12) must give the same verdicts as Go for every possible remote input, not only for honest ones.
38. **Secrets messages count as stage messages.** A peer's RS satisfies the "message from this identity" count of every stage receive; before the CM stage it diverts the run to blame assignment, during the CM stage it is ignored and the sender is excluded by CM timeout without a reveal (section 5.7, run details). All honest peers must take the same branch to compute the same next session ID.
39. **KE-stage gating.** A wallet sends its KE even when it will not continue, then stops silently when the PR set has fewer than `MinPeers` members or no PR carries the solver flag (section 5.7 steps 3–4). A port that continues in those cases publishes CTs nobody else will answer.
40. **First session at the second epoch boundary** after start-up, and `Dicemix` requests held until first boundary + `T` + 2 s (section 5.4). The warm-up exists so that PRs already in the mixpool at start-up whose owners sent no KE at the first boundary are pruned before the wallet forms its first PR set (`client.go:878-908`); a port that joins at the first boundary forms that set from an unpruned pool.

---

## 10. Quirks

Each is stated with what a faithful port must do. "Reproduce" means other participants or agreement depend on it; "may diverge" means no other party can observe the difference.

- **Q1 — "Rerun" never increments `Run`.** The wire has a `Run` field in every message and the code passes run numbers around, but the client hard-codes 0 everywhere (`mixclient/client.go:1538,1545,1765,1852,1951,2042,2192`) and the mixpool rejects anything else (`mixpool/mixpool.go:1188-1190`). `SharedSecrets` ignores its `run` argument (`keyagreement.go:130`). Reproduce.
- **Q2 — The post-DC secrets window is always waited out.** The receive for a single RS (`client.go:2052-2061`) returns early only if an RS arrives, so every successful run idles until `begin + 5T` before sending CMs. Reproduce: sending CMs earlier is harmless to peers that already hold all DCs, but a port that proceeds early and then sees an RS has confirmed a transaction it should have abandoned.
- **Q3 — Ciphertexts stop being reproducible after a non-fresh re-formation.** A non-fresh peer clone shares the run PRNG object (`client.go:268-274`), so if a previous attempt already encapsulated, the next attempt's `Encapsulate` (`client.go:1759`) reads further down the stream. Blame assignment recomputes ciphertexts from the position immediately after key generation (`blame.go:237-296`) and would blame such a peer for `different ciphertexts`. This follows from the code; no upstream test covers a reveal after a non-fresh re-formation. A port that restarts the stream at the post-key-generation position for every encapsulation pass produces ciphertexts that Go peers *can* verify, and is indistinguishable otherwise. Decision required (see Open questions).
- **Q4 — Size-limit truncation.** The loop that builds the kept PR list stops at the last excluded PR and drops every PR after it, even ones the size walk accepted (`client.go:1674-1685`). Reproduce exactly; the kept list determines the next session ID.
- **Q5 — Busy loop while waiting for roots.** A non-solving client (or one whose solver died mid-session) that receives no valid FP spins in `roots` without sleeping once `recvSR` has passed (`client.go:2209-2257`), until the session disappears from the mixpool or an RS arrives. It sends nothing meanwhile and is excluded by the others for a DC timeout. May diverge: wait instead of spinning, and give up at the session deadline.
- **Q6 — Revealed secrets can be re-queued.** The exit handler regenerates keys only if the *last* loop iteration revealed secrets (`client.go:1275,1178-1185`), and it clones from the pairing's original peer objects, not from the latest run's. After "reveal, re-form, fail again without a reveal" the request returns to the pending set with the seed, SR messages and DC messages that were published in its RS, while its `CoinJoin` holds the scripts generated later; its next mix would use publicly attributed addresses and then fail its own confirmation check. Derived from the code; untested upstream. May diverge (track "revealed at any point in this pairing").
- **Q7 — PR timing overshoot.** For a request arriving inside the quiet window, the wait is `(epoch + T − now) + jitter + U[0, nextSendBefore − now)` (`client.go:741-750`); the random term is measured from `now` rather than from `epoch + T`, so the PR can land almost `2T + 2·maxJitter` past the start of the *next* quiet window (latest send time = next epoch boundary + `T` + `peer.jitter` + `U[0, 300 ms)`, i.e. just under next boundary + `T + maxJitter`, reached when `now = epoch − T − maxJitter`), contrary to the comment at `client.go:731-734`. Timing is observable only statistically. May diverge; reproducing is simplest.
- **Q8 — SR messages cannot be encoded when `mtot > 512`.** The SR encoder and decoder cap the inner dimension at `wire.MaxMixPeers` (512) (`wire@v1.8.0/msgmixslotreserve.go:81-85,200-203`) although that dimension is `mtot`, which the mixing limits allow up to 1024. The mixpool's own SR limit is the higher `MaxMtot` (`mixpool/limits.go:46`), so the local `AcceptMessage` succeeds and the failure occurs when the message is encoded for publication. With the dcrd-RPC backend the encoding error (`rpc/client/dcrd/calls.go:243-250`) fails `SubmitMixMessage`, which removes the SR from the local mixpool again and clears it on the peer; a session with `mtot` in 513..1024 therefore passes size limiting, then no SR is ever received, every peer is blamed for an SR timeout and the session aborts. With the SPV backend `SubmitMixMessage` only announces an inventory vector (`spv/backend.go:479-519`) and succeeds, so the wallet keeps its own SRs, never obtains anyone else's (the full message cannot be encoded when it is requested), and blames every other peer instead; the exclusion sets of RPC-mode and SPV-mode wallets then differ. Derived from the code; UNVERIFIED by execution, and the SPV `getdata` reply path was not traced. Reproduce (do not "fix" the size limit unilaterally; it would change session agreement).
- **Q9 — `removeUnresponsiveDuringEpoch` stops at the first quiet pairing.** It returns, rather than continuing, when a pairing group has fewer than `MinPeers` KEs, and it visits groups in Go map order (`client.go:868-874`), so which groups are processed is not deterministic. May diverge (process every group); effect is limited to local mixpool hygiene.
- **Q10 — A request can be orphaned without a result.** The exit handler silently skips local peers whose PR is no longer in the mixpool (`client.go:1173-1177,1195-1199`); such a `Dicemix` call returns only when its own context ends. The ticker's expiry sweep covers only peers in the pending set. Derived from the code. May diverge by delivering `expiredPRErr`.
- **Q11 — Testnet is detected by the epoch length.** `NewClient` switches timing and pairing version on `epoch == 3 min` (`client.go:428-436`); simnet and regnet use mainnet values. Reproduce the resulting values per network.
- **Q12 — Decapsulation failure during blame is charged to the decapsulating peer**, not to `DecapsulateError.SubmittingIndex` (`blame.go:371-377`). Reproduce (exclusion sets must agree).
- **Q13 — Unused parameters.** `Encapsulate`'s `my` (`keyagreement.go:96`), `SharedSecrets`' `run` (`keyagreement.go:130`), `AddInput`'s `prevScriptVersion` (`coinjoin.go:86`). No behaviour to port.
- **Q14 — `resetUnmixed` touches every local peer of the pairing**, including requests that are not part of the completed run (`client.go:1631-1633`); such a request can simultaneously be cloned into the next epoch's session, so two sessions may mutate one `CoinJoin`. May diverge by giving each run its own transaction object.
- **Q15 — Documentation disagreements.** (a) `mixing.Message` says FP's `PrevMsgs` returns nil (`message.go:33-36`); it returns `SeenSlotReserves` (`wire@v1.8.0/msgmixfactoredpoly.go:253-255`). (b) The SR struct comment calls `DCMix` an "mcount-by-peers matrix" (`msgmixslotreserve.go:35`); it is mcount-by-mtot. (c) `MsgMixSecrets.writeMessageNoSignature` is documented as never failing (`msgmixsecrets.go:232`) but fails on more than 512 seen secrets or more than 1024 DC messages. (d) The key agreement is secp256k1 ECDH; no X25519 exists in `mixing@v0.7.5`, and `cspp/v2`'s `x25519` package is not linked by dcrwallet (only `solverrpc` is imported). dcroxide's crate comment describing it as "X25519-style ECDH" (`dcroxide:crates/dcroxide-mixing/src/lib.rs:8-11`) is inaccurate. The code wins in each case.
- **Q16 — Logging defects.** After the DC send, the wrong variable is logged (`client.go:1957-1959` prints `err`, not `sendErr`); `blaming %x for unparsable pubkey` is called without its argument (`blame.go:260`, unreachable). Reproduce only if byte-exact logs are required.
- **Q17 — Inconsistent sort comparator** in alternate-session formation (`client.go:2376-2386`: "identity lower **or** fewer seen PRs"). Because the input holds at most one KE per identity and nothing downstream depends on the order, it has no effect. Do not port the sort.
- **Q18 — Ticker returns a stale nil error** if its context ends during the warm-up wait (`client.go:892-893`). Unreachable in practice (nothing cancels that context before the ticker itself fails).
- **Q19 — Majority threshold** is `count ≥ totalPRs / 2` with integer division (`client.go:2459-2466`), so with an odd total a PR seen by fewer than half still passes. Reproduce.
- **Q20 — Publication details.** The publishing delay uses the smallest peer jitter with 0 meaning "unset" (`client.go:2109-2111`); the coinjoin object used for merging is that of the first non-remote peer even if its request was cancelled (`client.go:2106-2108`); publication uses a background context (`client.go:2133`). Reproduce the first two in effect; the context choice means a publish is not abandoned when the session deadline passes.
- **Q21 — Go map iteration order** decides: the order of identities in the "wrong ciphertext count" and "shared SR message" blame lists (`client.go:1857-1862`, `blame.go:334-341`); which local PRs come first when trimming to 512 (`client.go:979-981`); the order in which pairings start sessions (`client.go:940`). None is visible to other peers; exclusion works on sets.
- **Q22 — Null witness values.** Coinjoin inputs are created with `ValueIn = −1`, `BlockHeight = 0`, `BlockIndex = 0xffffffff` (`coinjoin.go:137`) and are never filled in, so the CM's transaction and the published transaction carry those values. Reproduce (transaction bytes).
- **Q23 — Dead code.** The seed-length check in blame (`blame.go:218-223`; the seed is a fixed 32-byte array), the commented-out unmixed-data validation (`blame.go:466-490`), the inner loop that compares the whole DC vector once per element (`blame.go:453-459`).
- **Q24 — Solver output is trusted.** A solving client does not check that returned values are roots; it checks only the count (`client.go:2174`). It never looks at other peers' FPs. Every local solver-capable peer publishes its own, identical FP (`client.go:2188-2195`). Reproduce the publication; a native solver should verify its own output.
- **Q25 — The solver is process-global.** One child per process shared by every client; a failed start is never retried; changing `SolverProcess` after the first start has no effect (`solverrpc/rpc.go:48-97`). Under `--mixing` without an explicit `--csppsolver`, a missing binary is only a warning, but a binary that starts and then fails the probe call with an end-of-file or shut-down RPC error is a fatal configuration error (`config.go:739-758`). A child that has already been observed to exit before the probe call makes the probe return `ErrSolverProcessExited`, which the probe does not treat as a failure (`testsolver.go:29-34`, `solverrpc/rpc.go:143-145`); the capability flag is then set and cleared again at the first root-solving attempt.
- **Q26 — Hashing edge cases in the wire package.** Hashing an SR whose `DCMix` is empty panics (`msgmixslotreserve.go:186-195` skips the emptiness check when hashing and then indexes row 0). Hashing a PR with an over-long or non-ASCII script class writes nothing after the signature (`msgmixpairreq.go:278-288`). Neither is reachable from decoded messages or from the client's own constructors.
- **Q27 — Expiry is deferred.** `ExpireMessages(height)` only records the height (`client.go:1097-1106`); mixpool expiry and `expiredPRErr` delivery happen at the next epoch tick, so that sessions in progress can keep using PRs that expire mid-epoch.
- **Q28 — Blame can list an identity twice** (two equal SR messages from one peer, `blame.go:327-341`), and the "too few remaining peers" test subtracts the list length, not the set size (`client.go:1365`). Reproduce the arithmetic.
- **Q29 — Go runtime dependencies.** Worker count and queue depth are `runtime.NumCPU()`; all jitter values have nanosecond resolution; `errors.Join` produces multi-line error text in logs. None is visible to peers.
- **Q30 — `AddVectors` sizes its result by the number of vectors** (`dcnet.go:88-98`); it is correct only because the SR matrix is square (`mtot` vectors of `mtot` values), which the dimension checks guarantee before it is called.
- **Q31 — Re-submission on retry.** When an alternate session cannot be formed, the same run is retried and its KEs are signed and submitted again (`client.go:1311-1316`); the mixpool treats them as duplicates.
- **Q32 — A request can be expired before its PR is submitted.** `Dicemix` inserts the peer into the pending set (`client.go:1065-1072`) before `SubmitMixMessage` returns (`client.go:1074-1075`). If the ticker's expiry sweep (`client.go:1108-1123`) runs in between, the PR is not yet in the mixpool, so the peer is removed and `expiredPRErr` is queued; the submission then succeeds and `Dicemix` immediately returns `expiredPRErr` while its PR stays in the mixpool with nobody serving it. If instead the submission fails, the cleanup at `client.go:1077-1082` operates on the pending-pairing object captured before the sweep: when that stale object is empty it deletes whatever entry is **currently** registered under the pairing ID, which can be a newer entry holding other requests; those requests are then never paired and never swept. Derived from the code; UNVERIFIED by execution. The window is one `SubmitMixMessage` call around an epoch tick. May diverge (insert after a successful submission, or look the entry up again).
- **Q33 — Success can go undelivered.** After publishing, `nil` is delivered through `forLocalPeers` under the session context (`client.go:2138-2141`). When that context is already done (its deadline is the next epoch boundary, and a re-formed session started late in an epoch has stage deadlines beyond it), the enqueue `select` (`client.go:562-566`) may take the context branch, and that peer receives no result. The exit handler nevertheless treats the session as mixed and drops its peers (`client.go:1152-1155`), so such a `Dicemix` call blocks until its own context ends although its outputs were published. Derived from the code; Go-`select`-order dependent. May diverge (deliver the result directly).
- **Q34 — Sessions with all CMs are removed at the next tick whether or not they mixed** (`client.go:932-938`, upstream `XXX`). `RemoveConfirmedSessions` removes every session holding as many CMs as PRs, together with its PRs (`mixpool/mixpool.go:694-714`). A run that collected all CMs but ended in exclusion for bad signatures, in a failed `PublishTransaction` (`Run error`), or in an unsuccessful re-formation leaves its local requests in the pending set with PRs that disappear at the next epoch tick; those requests then end with `expiredPRErr` although nothing expired. Reproduce the removal (it shapes the next epoch's PR set for every participant); the error text delivered to the caller is local.
- **Q35 — Blame compares unreduced SR values.** The run reduces published SR values modulo `F` (`dcnet.go:88-98`), but blame verification compares the recomputed canonical value with the raw published integer (`blame.go:390-396`). A peer that publishes `v + F` (still at most 32 bytes) does not disturb the mix, but is blamed for `bad SR mix` if a reveal happens for any other reason. Reproduce.
- **Q36 — Write-only state.** `CoinJoin.peerPRs` and `CoinJoin.gens` are maintained but never read (`coinjoin.go:47,51,126-134,167`); `excludeBlamed` sets `idx` on the run it returns, which `pairSession` overwrites when the run is materialized (`client.go:2576,1220`); `sizeLimited.excluded` is never read by `pairSession`. No behaviour to port.
- **Q37 — Send errors are not gating.** Send errors at every stage are only logged; in particular a local peer whose own stage message failed to submit (and was therefore cleared, section 5.13) keeps running and is then counted by `blameTimedOut` among the peers lacking that message, so a wallet can blame its own identity and deliver `testPeerBlamedError` to that request (`client.go:686-689,85-117,2559-2562`). Derived from the code. Reproduce the exclusion (peers that also lack the message exclude the same identity for the same timeout).
- **Q38 — Wire codec asymmetries.** A DC with zero vectors decodes but cannot be encoded; the DC encoder does not check that rows have equal length; the RS encoder does not bound `SlotReserveMsgs`; SR and FP error texts differ between encode and decode (section 8.5). Reproduce the decode side exactly (it decides what a peer accepts); the encode side is local.
- **Q39 — No logger, no logs.** Every log helper returns early when the logger is nil (`client.go:351-365,460-482`). dcrwallet always sets one (`wallet/wallet.go:5538-5552`); `log.go:42` still carries a commented-out `mixclient.UseLogger` call for an API that does not exist.
- **Q40 — sntrup randomness depends on the Go toolchain and `int` width.** `zx.RandomTSmall` sorts with `crypto/subtle.ConstantTimeLessOrEq` applied to full 32-bit values, outside the function's documented domain (`sntrup4591761@9e0f3af2/zx/random.go:29-35`). Go ≥ 1.26 on a 64-bit target yields an unsigned ascending sort; Go ≥ 1.26 on a 32-bit target a signed sort; Go ≤ 1.25 a non-transitive wrapped-difference comparison whose result depends on the network (section 5.10.2). The private polynomial `f` and every encapsulation's `r` change with it, so the KE's PQ public key and all ciphertexts derived from one seed differ between such builds; this is invisible in an honest run and surfaces only in blame assignment, where peers of the other kind are blamed for `KE public keys not derived from their PRNG`. The parity pin (`go 1.26.0`, 64-bit) is the unsigned sort. Decision required on which variant(s) the Rust client implements.
- **Q41 — `CoinJoin.Tx()` is fully signed for only one local request.** Signatures are merged into the `CoinJoin` of the first non-remote peer in peer order only (`client.go:2102-2126`); that object's transaction is the one published. Every other local request in the same run keeps its own transaction object, which has the identical prefix (same transaction hash, same output order, so `MixedIndices()` is valid) but carries signature scripts only on that request's own inputs (`coinjoin.go:248-264`). `Dicemix` returns nil to all of them, and `cj.Tx()` then returns the partially signed object. dcrwallet uses the returned transaction for its hash and log line in the account mixer (`wallet/mixing.go:437-439`), and in the mixed ticket-split path records it and passes it to `PublishTransactions` again (`wallet/createtx.go:1089-1092,1457-1478`); what the network backend does with a partially signed duplicate of an already published transaction belongs to the ticket-purchase specification and was not traced. Reproduce the hash and indices; a port may hand every request the fully signed transaction (not observable by peers).
- **Q42 — Non-modular exponentiation in `SRMix`.** `m^(c+1)` is computed as an exact integer power and reduced afterwards (`dcnet.go:52-54`), so for `mtot` = 1024 the intermediate has about 130 000 bits; `IsRoot` uses modular exponentiation (`dcnet.go:141`). The result is identical to a modular power. Do not port the cost.
- **Q43 — `Coefficients`' leading coefficient is `F − 1`, not reduced from −1 by a general routine**, and the inverse `(i+1)^(−1)` is negated before the final reduction (`dcnet.go:119-130`); intermediate sums are not reduced until the end of each coefficient. All coefficients handed to the solver and to `IsRoot` are canonical residues in `[0, F)`. A port working in a reduced field type gets the same values.
- **Q44 — `signatures.go` writes the signature through the `Sig()` accessor** (upstream `XXX`, `signatures.go:36-37`): the accessor must return a mutable view of the message's signature array. `SignMessage` on a message whose `Sig()` returned a copy would silently leave the signature zero.
- **Q45 — Cancelled requests keep a pairing alive.** A cancelled `Dicemix` call's peer stays in the pending set and is cloned into a session every epoch until its PR leaves the mixpool; when no other local request of that pairing is active, such a session ends at the KE step without sending (section 5.4); otherwise the session runs with the cancelled request's PR in its peer list and that peer never sends a KE. The PR itself stays in every other participant's mixpool and makes their sessions wait for a KE that never comes, until `RemoveUnresponsiveDuringEpoch` or expiry removes it. There is no PR-withdrawal message. Reproduce (nothing else is possible on the wire); local bookkeeping may diverge.
- **Q46 — `validateMergedCoinjoin` reuses one script buffer and never clears the opcode byte.** The tagged form writes the opcode into byte 0 of the shared 26-byte buffer; the untagged form returns bytes 1..26 and is unaffected by a stale byte 0 (`client.go:2263-2288`). No behavioural consequence; noted so that a port does not read meaning into the stale byte.

---

## 11. Upstream tests

| File | What it pins | Port / regenerate |
|---|---|---|
| `dcrd/mixing@v0.7.5/sid_test.go` | `TestSessionID`: session ID over six fixed hashes at epoch 0 and that the XOR-sorted order validates. `TestInvalidSession`: fixed sid `695794d3492979b67b51cd79fa330eaae643955ecea866b3e35d7abed2ec621e` with order `{1},{0},{3},{2},{5},{4}`; a flipped sid byte gives `errInvalidSessionID`; swapped PRs give `errInvalidPROrder`. | Port directly (dcroxide already carries equivalent vectors). |
| `dcrd/mixing@v0.7.5/signatures_test.go` | `TestSignMessage`: private key `…01`, a fixed PR, expected signature `1ffdddfb…7f18b50a`. `TestVerifySignedMessage`. Two benchmarks. | Port directly. |
| `dcrd/mixing@v0.7.5/mixclient/client_test.go` | `TestHonest`: four local peers with message counts 1–4 in one client on testnet3 parameters mix successfully. `TestCTDisruption` (flip a ciphertext bit), `TestCTLength` (one ciphertext too few, then one too many), `TestSRDisruption` (flip an SR value bit), `TestDCDisruption` (flip a DC bit): five peers split over two clients sharing one mixpool; each asserts that exactly the misbehaving identity is removed and the rest complete. All require a working `csppsolver` and are skipped without it. | Port the scenarios onto equivalent hooks. They are not vector tests; they pin behaviour, not bytes. |
| `dcrd/mixing@v0.7.5/mixclient/log_test.go` | Test logger plumbing. | Not needed. |
| `dcrd/mixing@v0.7.5/utxoproof/utxoproof_bench_test.go` | Benchmarks only. | — |
| `dcrd/wire@v1.8.0/msgmix*_test.go` (eight files) | Encode/decode round trips, cross-protocol-version errors and maximum payload lengths for each message; `TestNewMixPairReqErrs`. | Covered by the wire specification / `dcroxide-wire`. |
| `cspp/v2@v2.4.0/solverrpc/solver_test.go`, `solver/solver_test.go` | `TestRoots`, `TestRootFactors` over the P-256 prime field for 3 to 50+ messages (not over `F`); `BenchmarkRoots`. The two files are the same tests against the RPC client and the in-process cgo solver respectively; the `solverrpc` one needs a `csppsolver` binary on `PATH`. | Port as solver tests; add vectors over `F`. |
| `sntrup4591761@9e0f3af2/sntrup4591761_test.go` + `testdata/sage128.gz` | `TestSage`: 128 Sage-generated instances, seven hex fields per line, checking key derivation, encapsulation and decapsulation from fixed `f`, `g`, `r`. | Port with the data file. It does not pin the byte consumption from a reader; add oracle vectors for `GenerateKey`/`Encapsulate` driven by a ChaCha20 stream. |
| `dcroxide:crates/dcroxide-mixing/tests/mixing_vectors.rs` + `tests/data/mixing_vectors.txt` | Go-generated vectors for message hashes and signatures, `SortPRsForSession`, `ValidateSession`, `SRMixPads`/`SRMix`, `AddVectors`, `Coefficients`, `IsRoot`, `InField`, `DCMixPads`/`DCMix`, `XorVectors`, the ChaCha20 PRNG, UTXO proofs, `MaxExpiry`. | Reuse as is. |

| `dcrd/mixing@v0.7.5/mixpool/mixpool_test.go`, `mixpool/orphans_test.go` | Mixpool acceptance and orphan handling (outside this scope; owned by the mixpool specification). They exercise `SortPRsForSession`, signing and the wire constructors indirectly. | With the mixpool. |
| dcrwallet | No test covers `testsolver.go`, the solver checks in `config.go`, or the `mixclient.Wallet` adapter. | Add start-up tests: explicit `--csppsolver` pointing at a missing binary (fatal), default name missing (warning only), a binary that exits immediately. |

The upstream integration tests run each client with `testWaiting`/`testTickC` set (section 5.4, test mode), a `testWallet` whose `SubmitMixMessage` is the mixpool's `AcceptMessage`, whose `PublishTransaction` records the transaction, and whose `SignInput` signs with in-memory keys; the disruption tests install one of the three publish hooks to corrupt a single identity's CT, SR or DC before it is signed, plus a `hookBeforeRun` hook that shortens `recvKE` to five seconds from the moment each run starts (`mixclient/client_test.go:46-52,82-175,263-397`).

**Untested upstream (no vectors, no unit tests):** `Client.roots` in isolation; `validateMergedCoinjoin`; `constTimeSlotSearch` and `constantTimeOutputSearch`; the size estimator (`coinjoinSize.join`); `sendLocalPeerMsgs` ordering and cancellation; `expireMessages` and the races of Q32–Q34; the unreduced-value comparison of Q35; `KX`/`NewKX`/`Encapsulate`/`SharedSecrets` (exercised only through the integration tests); `MsgMixSecrets.Commitment`; `prDelay`; the epoch ticker and warm-up; `completePairing` with several candidate runs; alternate session formation; size limiting; the non-solving FP path and solver-exit fallback; CM-stage misbehaviour (wrong transaction, bad signature); RS timeouts and the blame fallback rule; reveal after a non-fresh re-formation (Q3); multiple-message peers under disruption; shutdown; the signature state of `CoinJoin.Tx()` for the second and later local requests of a run (Q41; `TestHonest` asserts only that every `Dicemix` call returned nil); a reveal arriving during pairing completion or during the CM stage (sections 5.12 and 5.7 details); cancelled requests (Q45); the dispatch precedence of joined run errors (section 5.6.2).

**Vectors to generate through a Go oracle shim** (deterministic inputs, byte outputs):

1. `NewKX` from a ChaCha20 seed: ECDH public key, PQ public key, PQ private key, and the stream offset afterwards.
2. `Encapsulate` continuing the same stream for n public keys: ciphertexts and cleartext keys.
3. `SharedSecrets` for small sessions with `mcounts` such as `[1,1,1,1]`, `[2,1,3,1]`, `[16,1,1,1]`, from each peer's view: all SR secrets and DC pad vectors (and a check that paired entries are equal).
4. `MsgMixSecrets.Commitment` and a full KE built from the same seed.
5. A complete honest transcript for 4–5 peers with fixed seeds, SR messages and HASH160s: every message's bytes and hash, the power sums, coefficients, sorted roots, mixed vector and the sorted unsigned coinjoin.
6. Size-limiting cases, including one where an excluded PR is followed by an accepted one (Q4), with the resulting kept list and session ID.
7. Alternate-session cases: KE view matrices in, `(prs, sid, unresponsive, err)` out.
8. Blame cases: one per check of section 5.12, expected blamed set.
9. gob byte captures of one `Solver.Roots` request and response (if the external solver is kept).
10. `SerializeSize` and `MaxPayloadLength` of each message for the transcript of item 5, and `Pairing()` bytes for a mainnet and a testnet3 PR.
11. A two-local-request run (two `CoinJoin` objects in one client): the bytes of `Tx()` for each request after success, to pin Q41.

---

## 12. Rust porting notes

### 12.1 What `dcroxide-mixing` already provides

Looked at: `dcroxide:crates/dcroxide-mixing/src/{lib,dcnet,field,prng,sid,signatures,utxoproof,mixpool}.rs` and its tests.

| Client-protocol element | dcroxide status |
|---|---|
| Constants `MinPeers`, `MaxPeers`, `MaxMcount`, `MaxMtot`, `MaxMixTxSerializeSize`, `MaxMixAmount`, `PRFlagCanSolveRoots`, `ScriptClassP2PKHv0`; `MaxExpiry` | **Present** (`lib.rs:39-85`). |
| Field `F`, `InField`, field arithmetic | **Present** as `FieldInt` over `u128` (`field.rs`), reducing on entry; `in_field_be_bytes` works on raw bytes. Sufficient for pads, power sums, coefficients, `IsRoot`. |
| `Vec` operations, `XorVectors`, `randVec` | **Present** (`dcnet.rs:24-82`). |
| `SRMixPads`, `SRMix`, `IntVectorsFromBytes`/`ToBytes`, `AddVectors`, `Coefficients`, `IsRoot`, `DCMixPads`, `DCMix` | **Present** (`dcnet.rs:84-214`), vector-tested against Go. |
| ChaCha20 PRNG (`New`, `Read`, `Next`) | **Present** (`prng.rs`). |
| `SortPRsForSession`, `ValidateSession` | **Present** (`sid.rs`); the sort clones PRs and recomputes hashes per comparison — acceptable, but a client wants a cached-hash variant. |
| `SignMessage`, `VerifySignedMessage`, `VerifySignature`, the `Message`/`Signed` abstraction | **Present** (`signatures.rs`, trait `MixMessage`). |
| UTXO proofs (sign and validate) | **Present** (`utxoproof.rs`). |
| Wire messages, hashes, signed data, `Pairing` | **Present** in `dcroxide-wire` (`msg_mix.rs`). `MsgMixSecrets::Commitment` has no helper (trivially: zero the signature, empty the seen list, hash). |
| Mixpool (accept, orphans, limits, `CompatiblePRs`, `ReceiveKEsByPairing`, removals, expiry, observer `check_prev_epoch`/`exclude_prs`) | **Present** (`mixpool.rs`), but synchronous: `receive` returns what is there and never waits (`mixpool.rs:1225-1232`), and `Pool` is not internally locked. |
| `keyagreement.go`: `KX`, `NewKX`, `Encapsulate`, `RevealedKeys`, `SharedSecrets`, `DecapsulateError` | **Missing** (explicitly deferred, `lib.rs:8-11`). |
| sntrup4591761 | **Missing** everywhere in dcroxide and dcr-rs. |
| secp256k1 ECDH (x-coordinate) and "private key from a byte stream with retry" | **Missing** from `dcroxide-dcrec`'s public surface (it wraps `k256`; `PrivateKey::from_bytes` rejects out-of-range values, which is the retry condition). Schnorr sign/verify and public key parsing are present. |
| Uniform random field element (`rand.BigInt(F)`) | **Missing** (`dcroxide-crypto::rand` documents `BigInt` as not ported; `duration` and `uint64n` are present). |
| `txsort` | **Missing** (no occurrence in dcroxide). |
| Root solver (RPC client or native) | **Missing**. |
| Entire `mixclient`: client, epoch ticker, pairing, run state machine, alternate sessions, size limiting, blame, `CoinJoin`, constant-time searches, size estimator | **Missing**. |
| Script engine for merged-coinjoin validation, `hash160`, P2PKH signing | **Present** in `dcroxide-txscript` (`engine.rs` flags including `VERIFY_TREASURY`, `stdaddr::hash160`, `sign::signature_script`). |

dcroxide tracks dcrd master `46e9df48` (`dcroxide:docs/PARITY.md:4`), while dcrwallet pins the tagged module `mixing v0.7.5`. dcroxide's mixpool already contains the v0.7.5-era rules checked during this study (non-zero run rejection, late-KE and whole-minute epoch checks, per-identity message cap), but a file-level diff of the two pins was not possible here (no checkout of `46e9df48` was available); it must be done before reuse is declared exact.

`dcr-rs` (`dcr-rs:src/` — `address`, `airgap`, `amount`, `blake256`, `hashing`, `hd`, `network`, `sighash`, `sign`, `tx`) contains nothing mixing-specific; nothing from it is needed here.

### 12.2 Suggested boundaries

| Crate / module | Mirrors | Contents |
|---|---|---|
| `sntrup4591761` (standalone, no dependencies beyond SHA-512) | `companyzero/sntrup4591761` | Section 5.10, constant-time, with a reader-driven API (`generate_key(&mut impl Read)`, `encapsulate(&mut impl Read, &pk)`). |
| `dcroxide-mixing::keyagreement` (add to the existing crate, since it belongs to the root package) | `mixing/keyagreement.go` | `Kx`, `new_kx`, `encapsulate`, `shared_secrets`, `DecapsulateError`; needs an ECDH helper and stream key generation added to `dcroxide-dcrec`. |
| `purse-mixsolver` | `cspp/v2/solverrpc`, `cmd/csppsolver`, `solver` | A `RootSolver` trait with (a) a native factorization over `F` and/or (b) a gob/`net/rpc` client for an external `csppsolver`. |
| `purse-mixclient` | `mixing/mixclient` | Client, ticker, pairing, run state machine, blame, coinjoin, limits; depends on an async wrapper around the mixpool that adds wait/notify to `receive`. |
| `txsort` helper (in the wallet's transaction utilities crate) | `dcrutil/txsort` | `in_place_sort`; other wallet code needs it too. |

### 12.3 Hazards

- **Determinism from the stream.** `NewKX` and `Encapsulate` must consume exactly the bytes of section 4.8, including both retry loops, and must reproduce the `RandomTSmall` ordering of the Go build being matched (section 5.10.2, Quirk Q40). Use the oracle vectors of section 11, generated with the pinned toolchain.
- **Blocking receive.** Model `Receive` as "wait for count or deadline, then return what is there with no timeout error"; treating a deadline as an error breaks the timeout-blame path, which depends on inspecting partial results.
- **Deadline discipline.** Three different clocks are in play: stage deadlines (absolute), the session context (next epoch), and submit deadlines detached from the session context. Keep them separate.
- **Exclusion-set agreement.** Every rule that removes a peer (timeouts, in-run blame, size limiting including Q4, alternate sessions, blame verification order) must match exactly; a single disagreement isolates the wallet into its own session ID.
- **Field values on the wire.** Remote SR values and roots may be up to 32 bytes and non-canonical; reduce on entry for arithmetic but compare root membership the way section 5.11.1 does (`InField` on the raw value first).
- **Secret hygiene.** Seeds, KX private keys, SR messages and unmixed HASH160s are secrets until revealed; zeroize on drop, keep the slot search and output search constant-time (sections 5.9.4, 5.8.4), and never reuse revealed material (Q6).
- **Shared `CoinJoin` state.** The Go object is shared by pointer among clones and across sessions (Q14); prefer a per-run transaction with an explicit hand-off of `mixedIndices` and the final transaction to the request.
- **Solver choice.** A native solver removes the gob protocol, the child process and the FLINT dependency but must reproduce the acceptance rule "exactly `mtot` distinct roots or fail", and should be fast enough for `mtot` up to 512 within one stage timeout (10 s on testnet3). An external-solver port needs a minimal gob codec for two fixed message shapes.
- **Shutdown.** Upstream shutdown waits for running sessions (up to one epoch). Decide whether the Rust daemon keeps that or cancels sessions.
- **Process-wide solver state** (Q25) versus per-client injection in Rust.
- **Receive capacity.** The Go API encodes "how many senders to wait for" in a slice capacity and "which type" in which slice is non-nil (section 5.14); make both explicit parameters, and keep "an RS from an identity counts as that identity's message".
- **Big-integer cost.** Do not copy `SRMix`'s exact (non-modular) powers (Q42); with a 127-bit field type the whole SR stage is cheap. `Coefficients` is quadratic in `mtot` (about half a million field multiplications at 1024) and `IsRoot` is linear per root, so validating an FP costs `mtot²` modular exponentiations as written in Go; Horner evaluation gives the same verdicts.
- **Result hand-off.** Give each request the fully signed transaction and its own mixed indices explicitly instead of reproducing the partially signed `Tx()` of Q41, unless the ticket-purchase specification turns out to depend on it.
- **Mutable accessors.** `Sig()`/`Pub()` alias message storage and `Commitment` hashes a modified copy (section 3.8, Q44); in Rust model signing as "compute signature, then set field" and the commitment as a pure function of `(identity, sid, run, seed, sr_msgs, dc_msgs)`.

### 12.4 Size estimate

**L.** The client proper is about 3,500 lines of Go with intricate control flow (pairing, re-formation, blame), plus roughly 950 lines for sntrup4591761, 240 for key agreement and 300–800 for the solver depending on the choice; the DC-net math, PRNG, session ID, signatures, proofs, wire messages and mixpool already exist in dcroxide. What pushes it to the top of L is verification: correctness is only demonstrable by byte-level oracle transcripts and by live interop against Go wallets in multi-peer, multi-epoch scenarios, including misbehaving peers.

---

## 13. Open questions

1. **Q3 (PRNG reuse).** Confirm with a Go test — reveal after a non-fresh re-formation — that honest Go peers blame each other for `different ciphertexts`. Then decide whether the Rust client reproduces the advancing stream or restarts it per encapsulation pass (verifiable by Go peers). UNVERIFIED by execution; derived from `client.go:260-277,1759` and `blame.go:237-296`.
2. **Q6 and Q10.** Confirm by test that (a) a request can return to the pending set with revealed secrets and (b) a request whose PR vanished mid-session never receives a result. Decide whether to diverge (recommended: never reuse revealed material; always deliver a result).
3. **Q8.** Confirm that a session with `mtot > 512` cannot complete (SR encoding failure) and decide whether the Rust client mirrors the failure or refuses such sessions locally in a way that keeps exclusion sets identical. UNVERIFIED by execution.
4. **Solver strategy.** External `csppsolver` over gob (drop-in for existing installations and packaging), a native Rust root finder (no child process), or both behind `--csppsolver`. If native: what `--csppsolver` then means, and whether the PR flag is always set.
5. **gob bytes.** The exact type-descriptor and value bytes of the `net/rpc` exchange were not captured; settle with a byte capture from the Go client against `csppsolver` (needed only for the external-solver option).
6. **FLINT factor order and non-monic input.** The client always sorts and always sends a polynomial with leading coefficient `F − 1`; whether FLINT's handling of the non-monic leading coefficient ever affects the reported constant terms was not examined beyond the Go wrapper (`solver/solver.go:41-103`). A native solver should normalize to monic first; verify against the oracle on random instances over `F`.
7. **dcroxide pin delta.** Diff `mixing` at dcroxide's pin (`46e9df48`) against `mixing v0.7.5` and `wire v1.8.0`, particularly `mixpool`, before depending on `dcroxide-mixing` unchanged.
8. **Q40 (sntrup sort variant).** Establish which Go toolchain (and `int` width) the dcrwallet releases that are live on the mixing network were built with, and confirm with an oracle (same seed through `NewKX` under Go 1.25 and Go 1.26) that the derived PQ public keys differ. Then decide whether the Rust client follows the parity pin only (unsigned sort) or must be switchable. UNVERIFIED by execution; derived from `zx/random.go:29-98` and the Go 1.25.14 / 1.26.0 `crypto/subtle` sources. **Existing Rust sntrup4591761 implementations.** None was examined. Whether any published crate implements the 2017 parameter set with a reader-driven API and identical randomness consumption is UNVERIFIED; the safe assumption is a direct port of the 946 Go lines.
9. **Shutdown latency and second `Run`.** Whether the Rust daemon must keep "finish running mixes before exit" (up to one epoch) is a product decision; dcrwallet's own shutdown path around `Wallet.Run` is outside this scope.
10. **Log parity.** Whether `MIXC` debug lines must be byte-identical (including the two defects of Q16) is a project-level decision.
11. **Mixpool wait semantics in Rust.** `dcroxide-mixing`'s `Pool::receive` is non-blocking and its `Received` uses `Option` fields rather than capacities; the wrapper must carry the expected count explicitly. Owner: mixpool specification.
12. **Q32, Q33, Q37.** Each is derived from reading the code and has not been reproduced: the expiry-sweep race in `Dicemix`, the undelivered success result when the session context has ended, and a wallet blaming its own identity after a failed submission. Settle each with a Go test built on the upstream test seams, then decide whether the Rust client diverges (none is visible to other peers except through the exclusion set of Q37, which must match).
13. **Q34.** Decide whether a request whose PR was removed by `RemoveConfirmedSessions` reports the upstream `expiredPRErr` text or a more accurate local error; the removal itself must be kept.
14. **Solver concurrency.** Whether concurrent `Solver.Roots` calls from sessions of different pairings occur in practice (it needs two pairings reaching the SR stage within the same seconds) was not observed; the protocol permits it and a port of the external-solver client must support it.
15. **Wire error parity.** Whether the encode-side error texts of section 8.5 must match byte for byte depends on whether they reach any log or RPC reply in dcrwallet; that path belongs to the wallet-mixing specification and was not traced here.
16. **Q41 downstream.** Trace, in the ticket-purchase specification, what `PublishTransactions` does when handed the partially signed split transaction of a second local request (`wallet/createtx.go:1457-1478`), and whether any RPC result (`purchasetickets` split transaction hex) exposes those bytes. UNVERIFIED here; it decides whether the Rust client may return the fully signed transaction to every request.
17. **Reveal before recording.** The "every remote peer is blamed for missing messages" outcome of a reveal during pairing completion (section 5.12 details) is derived from the code only; confirm with a Go test and decide whether the Rust client must reproduce it (it changes the exclusion set, so by default it must).
