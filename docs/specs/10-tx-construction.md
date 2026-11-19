# 10 — Transaction construction: coin selection, fees, sizes, change, signing helpers, address buffers

## Header

**Scope (files read in full, line counts at the parity pin):**

| File | Lines | Notes |
|---|---|---|
| `wallet/createtx.go` | 2176 | Everything except the deep details of `purchaseTickets` (owned by the ticket-purchase spec). `makeTicket`, the split-transaction builders, the vote builder and the signing helpers are covered here. |
| `wallet/txauthor/author.go` | 234 | Unsigned transaction authoring, change, signing loop. |
| `wallet/txauthor/author_test.go` | 239 | Test vectors. |
| `wallet/txrules/rules.go` | 145 | Dust, fee, high-fee policy. |
| `wallet/txrules/poolfees.go` | 95 | VSP ("stake pool") fee formula. |
| `wallet/txrules/doc.go` | 41 | Package doc (formula description). |
| `wallet/txrules/poolfees_test.go` | 93 | Test vectors. |
| `wallet/txsizes/size.go` | 209 | Size constants and estimators. |
| `wallet/txsizes/size_test.go` | 70 | Test vectors. |
| `wallet/multisig.go` | 101 | P2SH multisig output lookup and redeem-output preparation. |
| `wallet/addresses.go` | 1038 | Address wrappers, per-account address buffers, gap policy, change sources. |
| `wallet/addresses_test.go` | 307 | Test vectors. |

`wallet/internal/txsizes` does not exist at the pin (`wallet/internal/` holds only `bdb` and `snacl`).

Code outside the listed files that carries behaviour belonging to this scope, and was read for it:

| Location | What |
|---|---|
| `wallet/udb/txmined.go:2351-2531` | `UnspentOutputCount`, `randomUTXO`, `RandomUTXO`, `UnspentOutputs` |
| `wallet/udb/txmined.go:2931-2967` | `confirmed`, `confirms`, `coinbaseMatured`, `ticketChangeMatured`, `ticketMatured` (duplicated verbatim in `wallet/wallet.go:4404-4440`) |
| `wallet/udb/txmined.go:3012-3351` | `InputSource`, `SelectInputs`, `MakeInputSource` (the default coin-selection algorithm) |
| `wallet/wallet.go:891-1065` | `watchHDAddrs` (the watch-ahead of HD addresses in the network backend's filter) |
| `wallet/wallet.go:1952-1973` | `CurrentAddress` |
| `wallet/wallet.go:4250-4283` | `LockedOutpoint`, `LockOutpoint`, `UnlockOutpoint`, `ResetLockedOutpoints` |
| `wallet/wallet.go:4572-4643` | `SendOutputs`, `SendOutputsToTreasury` (the two plain callers of `authorTx`) |
| `wallet/wallet.go:5446-5493`, `2203-2213`, `3873-3889`, `3933-3949`, `wallet/cointype.go:52-58`, `wallet/discovery.go:801-818`, `893-938` | Every site that creates or adjusts an address buffer |
| `wallet/udb/addressmanager.go:1872-2065` | `MarkUsed`, `MarkUsedChildIndex`, `MarkReturnedChildIndex`, `SyncAccountToAddrIndex` |
| `internal/compat/compat.go:10-14` | `HD2Address` |
| `errors/errors.go` | `errors.E`, kinds, string formatting |

**Parity pin:** dcrwallet `54b85b33` (version 2.2.0-pre, module `decred.org/dcrwallet/v5`).

**Dependency versions read (from `go.mod`):**

| Module | Version | Read for |
|---|---|---|
| `github.com/decred/dcrd/crypto/rand` | v1.0.1 | `Read`, `IntN`, `Int32N`, `Duration`, `ShuffleSlice` |
| `github.com/decred/dcrd/txscript/v4` | v4.1.2 | `sign` (whole file), `stdscript.MultiSigScriptV0`, `ProvablyPruneableScriptV0`, `stdaddr` stake scripts, `IsUnspendable`, `GenerateSSGenBlockRef`, script flags, `MaxScriptElementSize` |
| `github.com/decred/dcrd/wire` | v1.8.0 | `NewMsgTx`, `NewTxIn`, `NewTxOut`, `TxOut.SerializeSize`, tx versions, network magics |
| `github.com/decred/dcrd/dcrutil/v4` | v4.0.3 | `MaxAmount`, `Amount.String` |
| `github.com/decred/dcrd/hdkeychain/v3` | v3.1.3 | `Child`, `ErrInvalidChild`, `HardenedKeyStart` |
| `github.com/decred/dcrd/blockchain/stake/v5` | v5.0.3 | `SStxNullOutputAmounts`, `CalculateRewards` |
| `github.com/decred/dcrd/chaincfg/v3` | v3.3.0 | `MaxTxSize`, maturities, `StakeBaseSigScript` |
| `github.com/decred/dcrd/blockchain/standalone/v2` | v2.3.0 | Referenced only (`SubsidyCache.CalcStakeVoteSubsidyV3`); not re-read here, dcroxide already ports it |

**Summary.** This subsystem turns "pay these outputs from this account" into a signed, policy-checked
transaction. `txsizes` supplies worst-case serialized-size arithmetic; `txrules` supplies the fee, dust,
high-fee and VSP-fee formulas; `txauthor` runs the fee/input/change fixed-point loop and the signing loop
against two small interfaces (an input source and a change source). `wallet/createtx.go` binds those to the
wallet: it implements the wallet's coin selection wrappers, the outpoint lock set, the change sources,
`authorTx` (author, sign, verify), `recordAuthoredTx`, `publishAndWatch`, the legacy multisig and
consolidation paths, split transactions for ticket purchases, and the vote builder and signer.
`wallet/addresses.go` owns the per-account, per-branch in-memory address buffers (`lastUsed`, `cursor`,
`lastWatched`), the three gap policies, and the rules that decide which child index the next address uses,
what is written to the database for it, and which addresses are loaded into the network backend's
transaction filter. Selection order is randomised with a CSPRNG, so authored transactions are not
reproducible byte for byte; everything else (sizes, fees, change amount, change eligibility, scripts,
signatures given the same inputs) is deterministic and must match exactly.

## 1. Role in the system

**What it does.**

- Estimates signed transaction sizes without signing (`txsizes`).
- Computes relay fees, dust thresholds, the high-fee guard, output sanity (`txrules`), and the VSP fee
  amount (`txrules.StakePoolTicketFee`).
- Authors unsigned transactions from an abstract input source and change source (`txauthor`), and signs
  them through `txscript/sign`.
- Selects wallet UTXOs for an account (random first, then shuffled full scan), honouring confirmation
  depth, maturity, the in-memory locked-outpoint set, and script class.
- Hands out BIP0044 addresses per account and branch, with gap-limit policy, and keeps the network
  backend's transaction filter loaded with the addresses the wallet may receive on.

**Who calls it.**

| Caller | Entry points used |
|---|---|
| JSON-RPC server (`internal/rpc/jsonrpc/methods.go`) | `Wallet.NewUnsignedTransaction` (`fundrawtransaction`, `sweepaccount`), `txauthor.NewUnsignedTransaction` + `Wallet.SecretsSource` (`spendoutputs`), `SendOutputs`/`SendOutputsToTreasury` (`sendtoaddress`, `sendmany`, `sendfrom`, `sendtotreasury`, …), `CreateMultisigTx` (`sendtomultisig`), `Consolidate` (`consolidate`), `FetchP2SHMultiSigOutput` + `PrepareRedeemMultiSigOutTxOutput` (`redeemmultisigout`, `getmultisigoutinfo`), `NewExternalAddress`, `NewInternalAddress`, `NewChangeAddress`, `CurrentAddress`, `KnownAddress`, `ImportedAddresses`, `BIP0044BranchNextIndexes` (`accountaddressindex`), `SyncLastReturnedAddress` (`accountsyncaddressindex`) |
| gRPC server (`internal/rpc/rpcserver/server.go`) | `NewUnsignedTransaction` (`ConstructTransaction`, `SweepAccount`), `NewExternalAddress`/`NewInternalAddress` (`NextAddress`), `AddressAtIdx` (`Address`), `NewChangeAddress`, `KnownAddress` (`ValidateAddress`), `txrules` helpers |
| Wallet internals | `authorTx`/`recordAuthoredTx`/`publishAndWatch` from `SendOutputs`, `SendOutputsToTreasury`, `PurchaseTickets` (VSP fee split), `individualSplit`; `findEligibleOutputsAmount` from `mixing.go` (mix output selection) and `ReserveOutputsForAmount` (VSP fee inputs); `nextAddress` from `mixing.go` (mixed outputs, mix change), `purchaseTickets` (vote and commitment addresses); `createUnsignedVote`/`signVote` from `chainntfns.go` (voting); `validateTxInput` from `chainntfns.go`; `markUsedAddress` from `chainntfns.go` (transaction processing) |
| Sync/startup | `watchHDAddrs` from the initial filter load (`wallet.go:1130`), after block connects and transaction acceptance (`chainntfns.go:294,424,919`), after publishing (`createtx.go:353,1253`) |
| Standalone tools | `cmd/sweepaccount`, `cmd/movefunds` call `txauthor.NewUnsignedTransaction` directly |

**What it calls.**

- `udb.Store` (transaction store): `MainChainTip`, `MakeInputSource`, `UnspentOutputs`, `UnspentOutputCount`,
  `RandomUTXO`, `InsertMemPoolTx`, `AddCredit`, `AddMultisigOut`, `GetMultisigOutput`.
- `udb.Manager` (address manager): `LastAccount`, `Address`, `AddrAccount`, `AccountName`,
  `AccountProperties`, `PrivateKey`, `RedeemScript`, `ImportScript`, `MarkUsed`, `SyncAccountToAddrIndex`,
  `MarkReturnedChildIndex`, `ForEachAccountAddress`.
- `walletdb`: `View`, `Update`, `BeginReadTx`, `BeginReadWriteTx`.
- `NetworkBackend`: `PublishTransactions`, `LoadTxFilter`.
- `Wallet.processTransactionRecord`, `Wallet.AbandonTransaction`, `Wallet.RescanPoint`, `Wallet.RelayFee`.
- dcrd libraries: `txscript` engine (`NewEngine`/`Execute`), `txscript/sign`, `stdscript`, `stdaddr`,
  `hdkeychain`, `stake`, `blockchain/standalone` subsidy cache, `crypto/rand`, `wire`.

## 2. Public surface inventory

Conventions: "Amount" is `dcrutil.Amount` (int64 atoms). Sizes and counts are Go `int` (64 bits on 64-bit
targets); every fee and dust computation converts to int64 before multiplying.

### 2.1 Package `wallet/txsizes`

| Name | Kind | Value / signature | Contract |
|---|---|---|---|
| `RedeemP2PKSigScriptSize` | const | 74 (`1 + 73`) | Worst-case sigScript redeeming compressed P2PK: `OP_DATA_73` + 72-byte DER sig + 1 sighash byte. |
| `RedeemP2PKHSigScriptSize` | const | 108 (`1 + 73 + 1 + 33`) | Worst-case sigScript redeeming compressed P2PKH: sig push + pubkey push. |
| `RedeemP2SHSigScriptSize` | const | 110 (`1 + 73 + 1 + 1 + 33 + 1`) | Worst-case sigScript redeeming a P2SH that wraps a single-key `<pubkey> OP_CHECKSIG` script. |
| `RedeemP2PKHInputSize` | const | 166 (`32+4+1+8+4+4+1+108+4`) | Full serialized input (prefix + witness) redeeming P2PKH. |
| `P2PKHPkScriptSize` | const | 25 | `OP_DUP OP_HASH160 OP_DATA_20 <20> OP_EQUALVERIFY OP_CHECKSIG`. |
| `P2PKHPkTreasuryScriptSize` | const | 26 | `OP_SSTXCHANGE` + P2PKH script (treasury-add change). |
| `P2SHPkScriptSize` | const | 23 | `OP_HASH160 OP_DATA_20 <20> OP_EQUAL`. |
| `TicketCommitmentScriptSize` | const | 32 (`1+1+20+8+2`) | `OP_RETURN OP_DATA_30 <20 hash><8 amount><2 limits>`. |
| `P2PKHOutputSize` | const | 36 (`8+2+1+25`) | Full serialized P2PKH output. |
| `TSPENDInputSize` | const | 109 (`1+73+1+33+1`) | Sig push + pubkey push + `OP_TSPEND`. Despite the name this is a sigScript size. |
| `EstimateSerializeSize` | func | `(scriptSizes []int, txOuts []*wire.TxOut, changeScriptSize int) int` | Worst-case signed size from input sigScript sizes and concrete outputs; adds one change output when `changeScriptSize > 0`. See 4.1. |
| `EstimateSerializeSizeFromScriptSizes` | func | `(inputSizes []int, outputSizes []int, changeScriptSize int) int` | Same, with outputs given as pkScript sizes. |
| `EstimateInputSize` | func | `(scriptSize int) int` | `57 + varint(scriptSize) + scriptSize`. |
| `EstimateOutputSize` | func | `(scriptSize int) int` | `10 + varint(scriptSize) + scriptSize`. |
| `sumOutputSerializeSizes` | unexported | `(outputs []*wire.TxOut) int` | Sum of `TxOut.SerializeSize()`. |

### 2.2 Package `wallet/txrules`

| Name | Kind | Signature | Contract |
|---|---|---|---|
| `DefaultRelayFeePerKb` | const | `dcrutil.Amount = 10000` (`0.0001 * 1e8`) | Default relay fee, atoms per 1000 bytes. |
| `StakeSubScriptType` | func | `(stdscript.ScriptType) (stdscript.ScriptType, bool)` | Maps stake-tagged P2PKH types (submission, change, gen, revocation, treasury-gen) to `STPubKeyHashEcdsaSecp256k1` and stake-tagged P2SH types to `STScriptHash`, returning `true`; any other type is returned unchanged with `false`. |
| `IsDustAmount` | func | `(amount Amount, scriptSize int, relayFeePerKb Amount) bool` | Dust predicate from an amount and a pkScript length. See 4.3. |
| `IsDustOutput` | func | `(output *wire.TxOut, relayFeePerKb Amount) bool` | Null-data scripts are never dust; other unspendable outputs always are; otherwise `IsDustAmount`. |
| `CheckOutput` | func | `(output *wire.TxOut, relayFeePerKb Amount) error` | Rejects negative value, value above `MaxAmount`, and dust. See 4.3. |
| `FeeForSerializeSize` | func | `(relayFeePerKb Amount, txSerializeSize int) Amount` | Fee for a size at a rate. See 4.2. |
| `PaysHighFees` | func | `(totalInput Amount, tx *wire.MsgTx) bool` | True when the fee exceeds the fee at 1000 × default rate for the transaction's actual serialized size. |
| `TxPaysHighFees` | func | `(tx *wire.MsgTx) (bool, error)` | Same, with total input summed from `TxIn.ValueIn`; errors if any `ValueIn < 0`. |
| `ValidPoolFeeRate` | func | `(feeRate float64) bool` | `floor(feeRate*100)` in `[1, 10000]`. |
| `StakePoolTicketFee` | func | `(stakeDiff, relayFee Amount, height int32, poolFee float64, params *chaincfg.Params, dcp0010Active, dcp0012Active bool) Amount` | VSP fee amount. See 4.4. |
| `subsidyCache`, `initSubsidyCacheOnce` | unexported vars | package-level `*SubsidyCache`, `sync.Once` | A single process-wide subsidy cache created from the params of the first call. |
| `sumOutputValues` | unexported | `([]*wire.TxOut) Amount` | Sum of output values. |

### 2.3 Package `wallet/txauthor`

| Name | Kind | Signature | Contract |
|---|---|---|---|
| `generatedTxVersion` | unexported const | `1` | Version of every authored transaction. |
| `InputDetail` | struct | `Amount Amount; Inputs []*wire.TxIn; Scripts [][]byte; RedeemScriptSizes []int` | Cumulative result of an input source: total value, inputs, previous pkScripts (same order), worst-case sigScript size per input. Despite the field name, `RedeemScriptSizes` holds signature-script sizes. |
| `InputSource` | func type | `func(target Amount) (*InputDetail, error)` | Returns inputs totalling at least `target` when possible; a short total signals insufficient funds. Called repeatedly with increasing targets and must be cumulative. |
| `AuthoredTx` | struct | `Tx *wire.MsgTx; PrevScripts [][]byte; TotalInput Amount; ChangeIndex int; EstimatedSignedSerializeSize int` | `ChangeIndex` is negative (−1) when there is no change output. |
| `ChangeSource` | interface | `Script() (script []byte, version uint16, err error)`; `ScriptSize() int` | `ScriptSize` is consulted before and independently of `Script`; `Script` is called at most once per authoring. |
| `NewUnsignedTransaction` | func | `(outputs []*wire.TxOut, relayFeePerKb Amount, fetchInputs InputSource, fetchChange ChangeSource, maxTxSize int) (*AuthoredTx, error)` | The authoring loop. See 4.5. `fetchChange` must be non-nil (it is dereferenced unconditionally). |
| `RandomizeOutputPosition` | func | `(outputs []*wire.TxOut, index int) int` | Swaps `outputs[index]` with a uniformly random position in `[0, len)` (may be itself) and returns the new index. Panics if `len(outputs) == 0` or `index` is out of range. |
| `(*AuthoredTx).RandomizeChangePosition` | method | `()` | `ChangeIndex = RandomizeOutputPosition(Tx.TxOut, ChangeIndex)`. Must only be called when `ChangeIndex >= 0`. |
| `SecretsSource` | interface | `sign.KeyDB` + `sign.ScriptDB` + `ChainParams() *chaincfg.Params` | Key and redeem-script lookup by address. |
| `AddAllInputScripts` | func | `(tx *wire.MsgTx, prevPkScripts [][]byte, secrets SecretsSource) error` | Signs every input with `SigHashAll`. See 4.6. |
| `(*AuthoredTx).AddAllInputScripts` | method | `(secrets SecretsSource) error` | `AddAllInputScripts(tx.Tx, tx.PrevScripts, secrets)`. |
| `sumOutputValues` | unexported | | Sum of output values. |

### 2.4 `wallet/createtx.go`

| Name | Kind | Signature | Contract |
|---|---|---|---|
| `revocationFeeLimit` | const | `1 << 14` (16384) | Revocation fee limit committed in ticket reward commitments. |
| `sanityVerifyFlags` | const | `ScriptDiscourageUpgradableNops \| ScriptVerifyCleanStack \| ScriptVerifyCheckLockTimeVerify \| ScriptVerifyCheckSequenceVerify \| ScriptVerifyTreasury` (numeric `0x4f`) | Script engine flags for self-verification of signed transactions. |
| `Input` | struct | `OutPoint wire.OutPoint; PrevOut wire.TxOut` | A spendable previous output. |
| `OutputSelectionAlgorithm` | type | `uint` | Selection algorithm tag. |
| `OutputSelectionAlgorithmDefault`, `OutputSelectionAlgorithmAll` | consts | 0, 1 | Declared as untyped integer constants (not of type `OutputSelectionAlgorithm`). |
| `(*Wallet).NewUnsignedTransaction` | method | `(ctx, outputs []*wire.TxOut, relayFeePerKb Amount, account uint32, minConf int32, algo OutputSelectionAlgorithm, changeSource txauthor.ChangeSource, inputSource txauthor.InputSource) (*txauthor.AuthoredTx, error)` | Unsigned authoring from wallet UTXOs. See 4.7. |
| `secretSource` | unexported struct | embeds `*udb.Manager`; `addrmgrNs`, `doneFuncs` | `GetKey` → `Manager.PrivateKey`, returns `(key.Serialize(), STEcdsaSecp256k1, true)`; `GetScript` → `Manager.RedeemScript`. Accumulates key-zeroing callbacks. No `ChainParams` method of its own: `ChainParams` is promoted from the embedded `*udb.Manager`. |
| `SecretsSource` | struct | `wallet`, `dbtx`, `doneFuncs` | Exported `txauthor.SecretsSource` holding an open read transaction. Methods: `ChainParams()`, `GetKey(addr)`, `GetScript(addr)`, `Close() error` (runs key-zeroing callbacks, rolls back the read transaction). |
| `(*Wallet).SecretsSource` | method | `() (*SecretsSource, error)` | Opens a read transaction with `db.BeginReadTx()`; caller must `Close`. |
| `CreatedTx` | struct | `MsgTx *wire.MsgTx; ChangeAddr stdaddr.Address; ChangeIndex int; Fee Amount` | Result of `CreateMultisigTx`. Only `MsgTx` is populated; `ChangeAddr` is nil, `ChangeIndex` is −1 and `Fee` is 0 even when a change output exists. |
| `insertIntoTxMgr` | unexported | `(dbtx, msgTx) (*udb.TxRecord, error)` | `NewTxRecordFromMsgTx(msgTx, time.Now())` then `txStore.InsertMemPoolTx`. |
| `insertCreditsIntoTxMgr` | unexported | `(op, dbtx, msgTx, rec) error` | For each output, for each extracted address owned by the wallet: `AddCredit(dbtx, rec, nil, i, ma.Internal(), ma.Account())` then `markUsedAddress`. Unknown addresses are skipped. |
| `insertMultisigOutIntoTxMgr` | unexported | `(dbtx, msgTx, index uint32) error` | New record then `txStore.AddMultisigOut(dbtx, rec, nil, index)`. |
| `checkHighFees` | unexported | `(totalInput Amount, tx) error` | Nil if `allowHighFees`; else `errors.E(errors.Policy, "high fee")` when `txrules.PaysHighFees`. |
| `publishAndWatch` | unexported | `(ctx, op, n NetworkBackend, tx, watch []wire.OutPoint) error` | Publish; abandon on failure; then load filter. See 4.10. |
| `authorTx` | unexported struct + method | fields `outputs, account, changeAccount, minconf, randomizeChangeIdx, txFee, dontSignTx, isTreasury` (inputs) and `atx, changeSourceUpdates, watch` (results) | Author, optionally sign and verify. See 4.8. |
| `recordAuthoredTx` | unexported | `(ctx, op, a *authorTx) error` | Persist change-address rows and the transaction. See 4.9. |
| `txToMultisig`, `txToMultisigInternal` | unexported | `(ctx, op, [dbtx,] account, amount, pubkeys [][]byte, nRequired int8, minconf int32) (*CreatedTx, stdaddr.Address, []byte, error)` | Legacy pay-to-multisig. See 4.11. |
| `validateTxInput` | unexported | `(op, tx, i int, prevScript []byte) error` | Runs the script engine for one input with `sanityVerifyFlags`, script version 0, no sig cache. |
| `validateMsgTx` | unexported | `(op, tx, prevScripts [][]byte) error` | `validateTxInput` for `i` in `0..len(prevScripts)`. |
| `creditScripts` | unexported | `([]Input) [][]byte` | Projects `PrevOut.PkScript`. |
| `compressWallet`, `compressWalletInternal` | unexported | `(ctx, op, [dbtx,] maxNumIns int, account uint32, changeAddr stdaddr.Address) (*chainhash.Hash, error)` | Consolidation. See 4.12. |
| `makeTicket` | unexported | `(params, input *Input, addrVote, addrSubsidy stdaddr.StakeAddress, ticketCost int64) (*wire.MsgTx, error)` | Builds an unsigned solo ticket. See 4.17. |
| `p2pkhSizedScript` | unexported var | 25 zero bytes | Placeholder pkScript for sizing mixed split outputs. |
| `mixedSplit` | unexported | `(ctx, req *PurchaseTicketsRequest, neededPerTicket Amount) (tx, outIndexes []int, err)` | Split transaction through a mix. See 4.16. |
| `individualSplit` | unexported | same shape | Plain split transaction. See 4.16. |
| `errVSPFeeRequiresUTXOSplit` | unexported var | `errors.New("paying VSP fee requires UTXO split")` | Sentinel consumed by `PurchaseTickets`. |
| `purchaseTickets` | unexported | `(ctx, op, n, req) (*PurchaseTicketsResponse, error)` | Owned by the ticket-purchase spec. |
| `(*Wallet).ReserveOutputsForAmount` | method | `(ctx, account uint32, amount Amount, minconf int32) ([]Input, error)` | Selects and **locks** outputs totalling at least `amount`. Caller unlocks. See 4.14. |
| `reserveOutputs` | unexported | `(ctx, account, minconf) ([]Input, error)` | Selects and locks every eligible output of the account. |
| `findEligibleOutputs` | unexported | `(dbtx, account, minconf, currentHeight) ([]Input, error)` | Full scan. See 4.13. |
| `findEligibleOutputsAmount` | unexported | `(dbtx, account, minconf, amount, currentHeight, minAmount Amount, maxResults int) ([]Input, error)` | Random then shuffled scan with target, minimum output amount and result cap. See 4.13. |
| `signP2PKHMsgTx` | unexported | `(msgtx, prevOutputs []Input, addrmgrNs) error` | Signs each input as P2PKH with `SigHashAll`. See 4.15. |
| `signVoteOrRevocation`, `signVote` | unexported | `(addrmgrNs, ticketPurchase, tx *wire.MsgTx[, isVote bool]) error` | Signs input 1 of a vote (or input 0 of a revocation) against the ticket's output 0 script. See 4.18. |
| `newVoteScript` | unexported | `(stake.VoteBits) ([]byte, error)` | Null-data script carrying vote bits. See 4.18. |
| `createUnsignedVote` | unexported | `(ticketHash, ticketPurchase, blockHeight int32, blockHash, voteBits, subsidyCache, params, dcp0010Active, dcp0012Active bool) (*wire.MsgTx, error)` | Builds an unsigned vote. See 4.18. |

### 2.5 `wallet/multisig.go`

| Name | Signature | Contract |
|---|---|---|
| `(*Wallet).FetchP2SHMultiSigOutput` | `(ctx, outPoint *wire.OutPoint) (*P2SHMultiSigOutput, error)` | Looks up a recorded multisig output and its redeem script. See 4.19. |
| `(*Wallet).PrepareRedeemMultiSigOutTxOutput` | `(msgTx *wire.MsgTx, p2shOutput *P2SHMultiSigOutput, pkScript *[]byte) error` | Appends one output paying `OutputAmount − fee` to `*pkScript`. See 4.19. |
| `P2SHMultiSigOutput` (declared in `wallet/common.go:47-61`) | struct | `OutPoint wire.OutPoint; OutputAmount dcrutil.Amount; ContainingBlock BlockIdentity; P2SHAddress *stdaddr.AddressScriptHashV0; RedeemScript []byte; M, N uint8; Redeemer *OutputRedeemer`. The result type of `FetchP2SHMultiSigOutput`; `Redeemer` is nil unless the output is spent. |
| `OutputRedeemer` (declared in `wallet/common.go:35-38`) | struct | `TxHash chainhash.Hash; InputIndex uint32`. The spending transaction and input index of a spent multisig output. |

### 2.6 `wallet/addresses.go`

| Name | Kind | Signature | Contract |
|---|---|---|---|
| `AccountKind` | type `int` | `AccountKindBIP0044 = 0`, `AccountKindImported = 1`, `AccountKindImportedXpub = 2` | Account classification. |
| `Address` | interface | `String()`, `PaymentScript() (version uint16, script []byte)`, `ScriptLen() int` | |
| `KnownAddress` | interface | `Address` + `AccountName() string`, `AccountKind() AccountKind` | |
| `PubKeyHashAddress` | interface | `KnownAddress` + `PubKey() []byte` (33-byte compressed), `PubKeyHash() []byte` (20 bytes) | |
| `BIP0044Address` | interface | `PubKeyHashAddress` + `Path() (account, branch, child uint32)` | Account is 0 for imported-xpub accounts. |
| `P2SHAddress` | interface | `KnownAddress` + `RedeemScript() (version uint16, script []byte)` | |
| `managedAddress`, `managedP2PKHAddress`, `managedBIP0044Address`, `managedP2SHAddress` | unexported structs | wrappers over `udb.ManagedAddress` | See 4.26. |
| `wrapManagedAddress` | unexported | `(addr udb.ManagedAddress, account string, kind AccountKind) (KnownAddress, error)` | See 4.26. |
| `(*Wallet).KnownAddress` | method | `(ctx, a stdaddr.Address) (KnownAddress, error)` | See 4.26. |
| `stakeAddress` | unexported interface | `voteRights`, `ticketChange`, `rewardCommitment`, `payVoteCommitment`, `payRevokeCommitment` | Implemented by `xpubAddress`; never called (dead code, see Quirks). |
| `xpubAddress` | unexported struct | embeds `*stdaddr.AddressPubKeyHashEcdsaSecp256k1V0`; `xpub`, `accountName`, `account`, `branch`, `child` | The concrete type returned by `nextAddress` and `AddressAtIdx`. Implements `BIP0044Address` and (through the embedded type) `stdaddr.StakeAddress`. See 4.26. |
| `DefaultGapLimit` | const | `uint32(20)` | Default address gap limit. |
| `DefaultAccountGapLimit` | const | `10` | Default account gap limit (consumed by `NextAccount`, outside this scope). |
| `gapPolicy` | unexported type `int` | `gapPolicyError = 0`, `gapPolicyIgnore = 1`, `gapPolicyWrap = 2` | |
| `NextAddressCallOption` | func type | `func(*nextAddressCallOptions)` | |
| `WithGapPolicyError`, `WithGapPolicyIgnore`, `WithGapPolicyWrap` | funcs | `() NextAddressCallOption` | Select the policy; default (no option) is `gapPolicyError`. |
| `withGapPolicy` | unexported | `(gapPolicy) NextAddressCallOption` | |
| `addressBuffer` | unexported struct | `branchXpub *hdkeychain.ExtendedKey; lastUsed, cursor, lastWatched uint32` | Per-branch state. See 3.2. |
| `bip0044AccountData` | unexported struct | `xpub; albExternal, albInternal addressBuffer` | Per-account state. |
| `accountBranchChild` | unexported struct | `account, branch, child uint32` | A deferred address-row write. |
| `accountBranchChildUpdates` | unexported slice type | `UpdateDB(ctx, w, maybeDBTX walletdb.ReadWriteTx) error` | Writes address rows for every entry. See 4.21. |
| `nextAddress` | unexported | `(ctx, op, updates *accountBranchChildUpdates, maybeDBTX, accountName string, account, branch uint32, callOpts ...NextAddressCallOption) (stdaddr.Address, error)` | The address allocator. See 4.20. |
| `signingAddressAtIdx` | unexported | `(ctx, op, account, childIdx uint32) (stdaddr.Address, error)` | External-branch address at an index, with address rows written. |
| `(*Wallet).AddressAtIdx` | method | `(ctx, account, branch, childIdx uint32) (stdaddr.Address, error)` | Derives without touching buffers or database. |
| `markUsedAddress` | unexported | `(op, dbtx, addr udb.ManagedAddress) error` | Records usage and extends derived address rows. See 4.23. |
| `(*Wallet).NewExternalAddress`, `NewInternalAddress` | methods | `(ctx, account uint32, callOpts ...NextAddressCallOption) (stdaddr.Address, error)` | Public allocators. See 4.22. |
| `notVotingAcct` | unexported | `(ctx, op, account) error` | Rejects imported voting accounts. |
| `newChangeAddress` | unexported | `(ctx, op, updates, maybeDBTX, accountName, account, gap gapPolicy) (stdaddr.Address, error)` | Internal-branch allocator with imported-account redirect. |
| `(*Wallet).NewChangeAddress` | method | `(ctx, account uint32) (stdaddr.Address, error)` | `newChangeAddress` with `gapPolicyWrap`. |
| `(*Wallet).BIP0044BranchNextIndexes` | method | `(ctx, account) (extChild, intChild uint32, err error)` | `lastUsed + 1 + cursor` per branch. |
| `(*Wallet).SyncLastReturnedAddress` | method | `(ctx, account, branch, child uint32) error` | See 4.24. |
| `(*Wallet).ImportedAddresses` | method | `(ctx, account string) ([]KnownAddress, error)` | See 4.26. |
| `p2PKHChangeSource`, `p2PKHTreasuryChangeSource` | unexported structs | `updates, dbtx, account, wallet, ctx, gapPolicy` | Wallet change sources. See 4.25. |
| `deriveChildAddresses` | unexported | `(key, startIndex, count uint32, params) ([]stdaddr.Address, error)` | Children `startIndex .. startIndex+count−1`, silently skipping invalid children (result may be shorter than `count`). |
| `deriveChildAddress` | unexported | `(key, child uint32, params) (stdaddr.Address, error)` | One child; returns `hdkeychain.ErrInvalidChild` unchanged. |
| `deriveBranches` | unexported | `(acctXpub) (extKey, intKey, err)` | Children 0 and 1 of the account xpub. |

### 2.7 Related surface outside the listed files (in-scope behaviour)

| Name | Location | Contract |
|---|---|---|
| `(*Wallet).watchHDAddrs` | `wallet/wallet.go:891` | Loads HD addresses into the backend filter. See 4.27. |
| `(*Wallet).CurrentAddress` | `wallet/wallet.go:1952` | External address at `lastUsed + 1 + cursor`. See 4.22. |
| `(*Wallet).LockedOutpoint`, `LockOutpoint`, `UnlockOutpoint`, `ResetLockedOutpoints` | `wallet/wallet.go:4250-4283` | The in-memory outpoint lock set consulted by every selector. |
| `(*udb.Store).MakeInputSource`, `(*udb.InputSource).SelectInputs` | `wallet/udb/txmined.go:3012-3351` | Default coin selection. See 4.13. |
| `(*udb.Store).RandomUTXO`, `randomUTXO`, `UnspentOutputs`, `UnspentOutputCount` | `wallet/udb/txmined.go:2351-2531` | UTXO enumeration primitives. |
| `confirmed`, `confirms`, `coinbaseMatured`, `ticketChangeMatured` | `wallet/wallet.go:4404-4431` | Spendability predicates. See 4.13. |
| `(*Wallet).SendOutputs`, `SendOutputsToTreasury` | `wallet/wallet.go:4572-4643` | `CheckOutput` on every output, then `authorTx` → `recordAuthoredTx` → `publishAndWatch`. |
| `(*Wallet).Consolidate`, `CreateMultisigTx` | `wallet/wallet.go:1539-1547` | Thin wrappers over `compressWallet` and `txToMultisig` with ops `"wallet.Consolidate"` and `"wallet.CreateMultisigTx"`. |
| `(*Wallet).RelayFee`, `SetRelayFee` | `wallet/wallet.go:770-783` | Read and replace the wallet-wide fee rate (atoms per 1000 bytes) under `relayFeeMu`. No validation, no persistence. See 4.28. |
| `(*Wallet).AllowsHighFees` | `wallet/wallet.go:5096-5098` | Returns the immutable `allowHighFees` flag set from `Config.AllowHighFees`; the same flag `checkHighFees` reads. |

Consumers of the pure packages outside the listed files (each must keep calling the same function with the
same arguments, because the resulting fee or decision is observable):

| Function / constant | Consumer | Use |
|---|---|---|
| `txrules.TxPaysHighFees` | gRPC `PublishTransaction` (`internal/rpc/rpcserver/server.go:1771-1780`); JSON-RPC `sendrawtransaction` (`internal/rpc/jsonrpc/methods.go:4691-4703`) | High-fee guard on externally supplied transactions. Both raise `errors.E(errors.Policy, "high fees")` (plural, unlike `checkHighFees`' `high fee`). gRPC applies it when `!AllowsHighFees()`; JSON-RPC applies it when the request's `allowhighfees` argument is absent or false and **ignores the wallet's `--allowhighfees` setting**. |
| `txrules.CheckOutput` | `SendOutputs`, `SendOutputsToTreasury` (`wallet/wallet.go:4576,4613`) | Per-output pre-check at the wallet's current relay fee. |
| `txrules.IsDustOutput` | JSON-RPC treasury spend construction (`internal/rpc/jsonrpc/methods.go:3731`); `compressWalletInternal` | Reject dust payouts. |
| `txrules.IsDustAmount` | `txauthor.NewUnsignedTransaction`; `CreateVspPayment` (`wallet/wallet.go:4730`) | Change-or-no-change decision. |
| `txrules.StakeSubScriptType` | `wallet/chainntfns.go:549`, `wallet/wallet.go:5038`, `wallet/udb/txmined.go:1787,3323` | Collapse stake-tagged script classes. |
| `txrules.StakePoolTicketFee` | `purchaseTickets` (`wallet/createtx.go:1297`), `PurchaseTickets` fee split (`wallet/wallet.go:1649`) | VSP fee amount. |
| `txrules.ValidPoolFeeRate` | none at the pin | Exported only. |
| `txsizes.EstimateSerializeSizeFromScriptSizes` | `purchaseTickets` ticket size (`wallet/createtx.go:1235-1239`: inputs `[108]`, outputs `[26, 32, 26]`, no change); mixing (`wallet/mixing.go:228-230,353-369`) | Ticket and mix fee estimates. |
| `txsizes.EstimateSerializeSize` | `CreateVspPayment` (`wallet/wallet.go:4722-4726`: 108 per input, the fee output, change script size 25); `PurchaseTickets` fee split (`wallet/wallet.go:1665`); JSON-RPC treasury spend (`internal/rpc/jsonrpc/methods.go:3741`) | Fee estimates. |
| `txauthor.RandomizeOutputPosition` | `CreateVspPayment` (`wallet/wallet.go:4733`) | Randomise the change position of the VSP fee transaction. |
| `txauthor.NewUnsignedTransaction` + `Wallet.SecretsSource` | JSON-RPC `spendoutputs` (`internal/rpc/jsonrpc/methods.go:4384`); `cmd/sweepaccount`, `cmd/movefunds` | Authoring with caller-defined sources. |
| `txsizes.P2SHPkScriptSize`, `EstimateInputSize`, `EstimateOutputSize` | no caller outside `txsizes` | Exported only. |

## 3. Data formats

This subsystem defines no bucket, key or value layout of its own. It produces transactions and scripts
(transmitted to the network and stored by the transaction store), drives a small set of address-manager
writes, and keeps in-memory state that determines which child indexes are handed out.

### 3.1 Database effects driven from this scope

Byte layouts of the rows named here are owned by the address-manager and transaction-store specs; this
table fixes *which* writes occur and *when*.

| Trigger | Write | Source |
|---|---|---|
| `accountBranchChildUpdates.UpdateDB` (every address returned by `nextAddress`, and `signingAddressAtIdx`) | `Manager.SyncAccountToAddrIndex(ns, account, child, branch)`: walking **downward** from `child`, insert a chained-address row for every child whose address is not yet recorded, stopping at the first one that is already recorded or after child 0. Each new row is `addrType = chain`, `account`, `addTime = time.Now().Unix()` (uint64), raw data `branch` (uint32 LE) ‖ `index` (uint32 LE), keyed by `SHA-256(hash160)`, plus the address→account index entry. | `wallet/addresses.go:491-500`, `wallet/udb/addressmanager.go:1985-2055`, `wallet/udb/addressdb.go:1177-1186,1337-1364` |
| Same | **Nothing else.** The account variables `extret` / `intret` (last returned child) are *not* written. | `wallet/addresses.go:468-502` |
| `markUsedAddress` | `Manager.MarkUsed`: if the address is a chained (BIP0044) row and `child + 1 > lastUsed + 1` (uint32 wrapping), set `extused`/`intused = child`; additionally if `lastReturned + 1 < child + 1`, set `extret`/`intret = child`. Then `SyncAccountToAddrIndex(account, min(2^31 − 1, lastUsed' + gapLimit), branch)` where `lastUsed'` is the value re-read after `MarkUsed`. Imported-account addresses stop after `MarkUsed` (which is a no-op for them). | `wallet/addresses.go:718-746`, `wallet/udb/addressmanager.go:1872-1941` |
| `SyncLastReturnedAddress` | `SyncAccountToAddrIndex(account, child, branch)` then `MarkReturnedChildIndex`: set `extret`/`intret = child` when `child > lastReturned` or `lastReturned == 0xffffffff`. | `wallet/addresses.go:881-888`, `wallet/udb/addressmanager.go:1949-1970` |
| `recordAuthoredTx` | The change-address rows above, then `processTransactionRecord(ctx, dbtx, rec, nil, nil)` (unmined transaction record, debits, credits, address usage) in the same write transaction. | `wallet/createtx.go:523-533` |
| `txToMultisigInternal` | `Manager.ImportScript(redeemScript)` (an `errors.Exist` result is ignored), change-address rows (when change is added), `txStore.AddMultisigOut(rec, nil, 0)`. The transaction itself is **not** inserted. | `wallet/createtx.go:640-646,671-680,712` |
| `compressWalletInternal` | Change-address rows (when no address was supplied), `txStore.InsertMemPoolTx`, then per owned output `txStore.AddCredit(rec, nil, index, internal, account)` and `markUsedAddress`. | `wallet/createtx.go:819-826,907-914` |

Because `addTime` is the wall clock at insertion, address rows are not byte-reproducible across runs.

### 3.2 In-memory address buffer

One `bip0044AccountData` per derivable account, keyed by account number in `Wallet.addressBuffers`
(`map[uint32]*bip0044AccountData`, guarded by `Wallet.addressBuffersMu`). The imported account
(`0x7fffffff`) has no entry.

| Field | Type | Meaning |
|---|---|---|
| `xpub` | extended public key | Account-level xpub. Used by `xpubAddress.PubKey`. |
| `albExternal`, `albInternal` | `addressBuffer` | Branch 0 and branch 1 state. |
| `addressBuffer.branchXpub` | extended public key | `xpub.Child(0)` or `xpub.Child(1)`. |
| `addressBuffer.lastUsed` | uint32 | Highest child index known used on chain. `0xffffffff` means none. |
| `addressBuffer.cursor` | uint32 | Number of children handed out (or skipped) beyond `lastUsed`. The next child index is `lastUsed + 1 + cursor` in wrapping uint32 arithmetic. Not decremented on error. |
| `addressBuffer.lastWatched` | uint32 | Highest child index believed loaded in the backend filter. Read and written only by `watchHDAddrs`; initialised by account-import paths. |

Initial values by creation site:

| Site | `lastUsed` | `cursor` | `lastWatched` |
|---|---|---|---|
| Wallet open, every account `0..=lastAccount` and every imported-xpub/voting account `0x80000000..=lastImportedAccount` (`wallet/wallet.go:5456-5493`) | DB `LastUsed{External,Internal}Index` | DB `LastReturned − LastUsed` (wrapping uint32) | 0 |
| `NextAccount` (`wallet/wallet.go:2207-2213`) | `0xffffffff` | 0 | 0 |
| `ImportVotingAccount`, `ImportXpubAccount` (`wallet/wallet.go:3873-3889,3933-3949`) | `0xffffffff` | 0 | `gapLimit − 1` |
| `UpgradeToSLIP0044CoinType`, replacing account 0 (`wallet/cointype.go:52-58`) | `0xffffffff` | 0 | 0 |
| Accounts created during account discovery (`wallet/discovery.go:801-818`) | **0** (zero value) | 0 | 0 |

### 3.3 Script layouts produced here

All scripts are script version 0. Offsets are byte offsets within the script.

**P2PKH payment script** (25 bytes; `managedAddress.p2pkhScript`, `xpubAddress.PaymentScript`,
`wallet/addresses.go:127-138,277-287`):

| Offset | Size | Value |
|---|---|---|
| 0 | 1 | `0x76` `OP_DUP` |
| 1 | 1 | `0xa9` `OP_HASH160` |
| 2 | 1 | `0x14` `OP_DATA_20` |
| 3 | 20 | hash160 of the compressed public key |
| 23 | 1 | `0x88` `OP_EQUALVERIFY` |
| 24 | 1 | `0xac` `OP_CHECKSIG` |

**P2SH payment script** (23 bytes; `managedAddress.p2shScript`, `wallet/addresses.go:139-148`):

| Offset | Size | Value |
|---|---|---|
| 0 | 1 | `0xa9` `OP_HASH160` |
| 1 | 1 | `0x14` `OP_DATA_20` |
| 2 | 20 | hash160 of the redeem script |
| 22 | 1 | `0x87` `OP_EQUAL` |

**Treasury-add change script** (26 bytes; `p2PKHTreasuryChangeSource.Script`, `wallet/addresses.go:981-996`):
`0xbd` (`OP_SSTXCHANGE`) followed by the 25-byte P2PKH script of a new internal address.

**Multisig redeem script** (`stdscript.MultiSigScriptV0`, `txscript/v4@v4.1.2/stdscript/scriptv0.go:822-849`):
`<threshold as small int> <33-byte pubkey push>… <pubkey count as small int> OP_CHECKMULTISIG (0xae)`. Every
key must be a strictly encoded compressed secp256k1 key. Numbers 1..16 are encoded as `OP_1..OP_16`
(`0x51..0x60`), 0 as `OP_0`, larger values as minimal data pushes. Errors: negative threshold
(`ErrNegativeRequiredSigs`), threshold above key count (`ErrTooManyRequiredSigs`), non-compressed key
(`ErrPubKeyType`). The output script paying it is the P2SH script of `hash160(redeemScript)`.

**Ticket scripts** (built by `makeTicket`, 4.17):

| Output | Script | Bytes |
|---|---|---|
| 0, voting rights (P2PKH address) | `OP_SSTX (0xba)` + P2PKH script | 26 |
| 1, reward commitment (P2PKH address) | `0x6a` `OP_RETURN`, `0x1e` `OP_DATA_30`, 20-byte hash160, amount as uint64 LE with bit 63 cleared, limits as uint16 LE | 32 |
| 2, change | `OP_SSTXCHANGE (0xbd)` + P2PKH script of the all-zero hash160 | 26 |

The limits word is `(ceil(log2(voteFeeLimit)) | 0x40)` in the low byte when `voteFeeLimit != 0` and
`(ceil(log2(revocationFeeLimit)) | 0x40)` in the high byte when `revocationFeeLimit != 0`
(`txscript/v4@v4.1.2/stdaddr/addressv0.go:606-621`). With the wallet's arguments `(0, 16384)` the word is
`0x4e00`, serialized as bytes `00 4e`. Voting-rights and commitment scripts for P2SH addresses come from
the `stdaddr` P2SH implementation (not re-read for this spec; the commitment sets bit 63 of the amount).

**Vote scripts** (built by `createUnsignedVote`, 4.18):

| Output | Script |
|---|---|
| 0, block reference | `0x6a` `OP_RETURN`, `0x24` `OP_DATA_36`, 32-byte block hash (internal byte order), block height uint32 LE (38 bytes) |
| 1, vote bits | `0x6a` `OP_RETURN`, canonical data push of: vote bits uint16 LE ‖ extended vote bits |
| 2.., reward payments | `OP_SSGEN (0xbb)` + P2PKH script or P2SH script of the committed hash160 |

"Canonical data push" is `txscript.ScriptBuilder.AddData`: length 2..75 uses `OP_DATA_n`, 76..255 uses
`OP_PUSHDATA1`, 256 uses `OP_PUSHDATA2`. The pushed data may be at most 256 bytes
(`stdscript.MaxDataCarrierSizeV0`), so the extended bits may be at most 254 bytes.

### 3.4 Signature script layouts

| Previous output class | Signature script | Source |
|---|---|---|
| P2PKH (plain or stake-tagged) | `<DER sig ‖ sighash byte> <33-byte compressed pubkey>` | `sign.SignatureScript`, `txscript/v4@v4.1.2/sign/sign.go:76-105` |
| P2PK | `<DER sig ‖ sighash byte>` | `sign.p2pkSignatureScript`, `sign.go:108-118` |
| P2SH (plain or stake-tagged) | `<sigScript for the redeem script, as raw opcodes> <redeem script push>` | `sign.SignTxOutput`, `sign.go:600-631` |
| Bare multisig / multisig redeem script | Up to `nRequired` signature pushes in address order, with **no** leading dummy; after merging with a previous script, missing signatures are padded with `OP_0` | `sign.signMultiSig`, `mergeMultiSig`, `sign.go:125-152,342-457` |

The sighash byte is `0x01` (`SigHashAll`) everywhere in this scope. Signatures are ECDSA over secp256k1
produced by `ecdsa.Sign`, which is deterministic and canonical per RFC 6979 and BIP0062
(`dcrec/secp256k1/v4@v4.4.1/ecdsa/signature.go:726-731`); for a fixed transaction and key the signature
bytes are fixed.

### 3.5 Transaction skeletons

Fields common to every transaction built here unless a row says otherwise: `SerType = TxSerializeFull`,
`Version = 1`, `LockTime = 0`, `Expiry = 0`. Every input is created with `wire.NewTxIn`, giving
`Sequence = 0xffffffff`, `BlockHeight = 0` (`NullBlockHeight`), `BlockIndex = 0xffffffff`
(`NullBlockIndex`), and `ValueIn` set to the previous output's amount.

| Builder | Inputs | Outputs | Deviations |
|---|---|---|---|
| `txauthor.NewUnsignedTransaction` | As returned by the input source, in source order | Caller outputs in caller order, then the change output (if any) appended last | Callers may then swap the change output with a random position, or set `Version = 3` (treasury) |
| `MakeInputSource` inputs | `PreviousOutPoint.Tree` is `TxTreeStake (1)` when the credit's recorded stake opcode is not "non-stake", else `TxTreeRegular (0)`; `SignatureScript = nil` | — | — |
| `txToMultisigInternal` | Selected outputs; `PreviousOutPoint.Tree` as stored in `udb.Credit.OutPoint`; nil sigScript before signing | `[0]` amount to the P2SH script; `[1]` change to a new internal P2PKH address when `totalInput > amount + fee` | — |
| `compressWalletInternal` | First `maxNumIns` eligible outputs that fit | `[0]` everything minus fee to the destination script | — |
| `makeTicket` | One input: the split output, `SignatureScript = []byte{}` (empty, non-nil) | 3 outputs (3.3) | Caller sets `Expiry` |
| `createUnsignedVote` | `[0]` stakebase: prev hash all zero, index `0xffffffff`, tree regular, `ValueIn = subsidy`, `SignatureScript = params.StakeBaseSigScript`; `[1]` ticket: `(ticketHash, 0, TxTreeStake)`, `ValueIn = ticket output 0 value`, nil sigScript | 3.3 | Caller may change `Version` |
| `PrepareRedeemMultiSigOutTxOutput` | Supplied by caller | Appends one output with `Version = 0` | — |

## 4. Algorithms and state machines

### 4.1 Size estimation (`wallet/txsizes/size.go`)

`varint(n)` is `wire.VarIntSerializeSize`: 1 for `n < 0xfd`, 3 for `n <= 0xffff`, 5 for `n <= 0xffffffff`,
else 9.

- `EstimateInputSize(s) = 32 + 4 + 1 + 8 + 4 + 4 + varint(s) + s + 4` = `57 + varint(s) + s`
  (prefix: prev hash 32, index 4, tree 1, sequence 4; witness: value 8, block height 4, block index 4,
  script length varint, script).
- `EstimateOutputSize(s) = 8 + 2 + varint(s) + s`.
- `EstimateSerializeSize(scriptSizes, txOuts, changeScriptSize)`:
  1. `txInsSize = Σ EstimateInputSize(size)` over `scriptSizes`.
  2. `inputCount = len(scriptSizes)`, `outputCount = len(txOuts)`.
  3. If `changeScriptSize > 0`: `changeSize = EstimateOutputSize(changeScriptSize)`, `outputCount += 1`;
     else `changeSize = 0`.
  4. Return `12 + 2*varint(inputCount) + varint(outputCount) + txInsSize + Σ txOut.SerializeSize() + changeSize`.
     The 12 is version (4) + lock time (4) + expiry (4). The input count is counted twice because the full
     serialization writes it once in the prefix and once in the witness section.
- `EstimateSerializeSizeFromScriptSizes(inputSizes, outputSizes, changeScriptSize)`: identical with
  `Σ EstimateOutputSize(outputSize)` in place of the concrete outputs.

Reference values: one P2PKH input and no outputs = 181; plus one P2PKH output = 217; two P2PKH inputs and
one P2PKH output plus P2PKH change = 419 (`wallet/txsizes/size_test.go:38-48`).

### 4.2 Fee for a size (`txrules.FeeForSerializeSize`, `wallet/txrules/rules.go:94-106`)

1. `fee = relayFeePerKb * int64(txSerializeSize) / 1000` using int64 arithmetic: the multiplication wraps
   on overflow (two's complement), the division truncates toward zero.
2. If `fee == 0` and `relayFeePerKb > 0`: `fee = relayFeePerKb`.
3. If `fee < 0` or `fee > MaxAmount` (`21e6 * 1e8 = 2_100_000_000_000_000`): `fee = MaxAmount`.

Consequences: any non-empty estimate under 1000 bytes at a rate below 1000 atoms/kB may floor to 0 and is
then lifted to the full per-kB rate; a zero rate yields a zero fee; a negative rate yields `MaxAmount`
when `|rate × size| >= 1000` and 0 otherwise (the truncated quotient is 0 and the lift applies only to
positive rates).

### 4.3 Dust and output checks (`wallet/txrules/rules.go:43-90`)

`IsDustAmount(amount, scriptSize, relayFeePerKb)`:

1. `totalSize = 8 + 2 + varint(scriptSize) + scriptSize + 165`.
2. Return `int64(amount) * 1000 / (3 * int64(totalSize)) < int64(relayFeePerKb)` (wrapping multiply,
   truncating division).

For a 25-byte script `totalSize = 201`, so the smallest non-dust amount is `ceil(relayFeePerKb * 603 / 1000)`:
6030 atoms at the default 10000 atoms/kB, 603 at 1000, 1538 at 2550
(`wallet/txauthor/author_test.go:114-148`).

`IsDustOutput(output, relayFeePerKb)`:

1. If `stdscript.IsNullDataScript(output.Version, output.PkScript)`: not dust.
2. Else if `txscript.IsUnspendable(output.Value, output.PkScript)`: dust. `IsUnspendable` is true when the
   value is 0, the script is longer than the maximum script size, the script begins with `OP_RETURN`, or
   the script fails to parse as version 0 (`txscript/v4@v4.1.2/script.go:458-470`).
3. Else `IsDustAmount(output.Value, len(output.PkScript), relayFeePerKb)`.

`CheckOutput(output, relayFeePerKb)` returns, in this order:

| Condition | Error |
|---|---|
| `output.Value < 0` | `errors.E(errors.Invalid, "transaction output amount is negative")` |
| `output.Value > MaxAmount` | `errors.E(errors.Invalid, "transaction output amount exceeds maximum value")` |
| `IsDustOutput` | `errors.E(errors.Policy, "transaction output is dust")` |

`PaysHighFees(totalInput, tx)`: `fee = totalInput − Σ outputs`; if `fee <= 0` return false; else return
`fee > FeeForSerializeSize(1000 * DefaultRelayFeePerKb, tx.SerializeSize())`, i.e. above 10,000,000
atoms/kB (0.1 DCR/kB) at the transaction's **current** serialized size (signed or not, as passed).

`TxPaysHighFees(tx)`: for each input in order, if `ValueIn < 0` return
`errors.E(errors.Invalid, fmt.Errorf("transaction input %d does not specify the input value", i))`; else
sum and delegate to `PaysHighFees`.

### 4.4 VSP fee (`txrules.StakePoolTicketFee`, `wallet/txrules/poolfees.go:35-95`)

Inputs: ticket price `v`, fee term `z` (the parameter is named `relayFee`; the two callers disagree on
what they pass: `purchaseTickets` passes the absolute fee of one ticket transaction,
`wallet/createtx.go:1297-1299`, while `PurchaseTickets`' fee-split path passes the wallet's relay fee
**rate** in atoms/kB, `wallet/wallet.go:1648-1650`), `height`, percentage `poolFee` (e.g. `1.00` for 1%).

1. `poolFeeInt = int64(floor(poolFee * 100.0))` (float64 arithmetic).
2. `adjs = int(ceil(float64(params.TicketPoolSize) / float64(params.SubsidyReductionInterval)))`
   (mainnet: `ceil(8192/6144) = 2`).
3. Initialise the package-level subsidy cache once (`sync.Once`) from `params`.
4. Subsidy split variant: `SSVDCP0012` if `dcp0012Active`, else `SSVDCP0010` if `dcp0010Active`, else
   `SSVOriginal`.
5. `subsidy = cache.CalcStakeVoteSubsidyV3(int64(height), variant)`; then repeat `adjs` times:
   `subsidy = subsidy * 100 / 101` (int64, truncating).
6. With arbitrary-precision integers: `num = poolFeeInt * subsidy * (v + z)`, shifted left 64 bits;
   `den = (subsidy + v) * 10000`; `result = (num / den) >> 64` (truncating division; all operands are
   non-negative in practice). Return `int64(result)`.

If `subsidy + v == 0` the division panics (big-integer division by zero). `ValidPoolFeeRate(feeRate)` is
`1.0 <= floor(feeRate * 100) <= 10000.0`. Test vectors are in section 10.

### 4.5 Authoring loop (`txauthor.NewUnsignedTransaction`, `wallet/txauthor/author.go:90-166`)

Inputs: `outputs`, `relayFeePerKb`, `fetchInputs`, `fetchChange`, `maxTxSize`.

1. `targetAmount = Σ outputs[i].Value`.
2. `changeScriptSize = fetchChange.ScriptSize()`.
3. `maxSignedSize = EstimateSerializeSize([108], outputs, changeScriptSize)` (one worst-case P2PKH input).
4. `targetFee = FeeForSerializeSize(relayFeePerKb, maxSignedSize)`.
5. Loop:
   1. `inputDetail, err = fetchInputs(targetAmount + targetFee)`. On error return
      `errors.E("txauthor.NewUnsignedTransaction", err)`.
   2. If `inputDetail.Amount < targetAmount + targetFee` return
      `errors.E("txauthor.NewUnsignedTransaction", errors.InsufficientBalance)`.
   3. `scriptSizes` = a copy of `inputDetail.RedeemScriptSizes`.
   4. `maxSignedSize = EstimateSerializeSize(scriptSizes, outputs, changeScriptSize)`;
      `maxRequiredFee = FeeForSerializeSize(relayFeePerKb, maxSignedSize)`;
      `remainingAmount = inputDetail.Amount − targetAmount`.
   5. If `remainingAmount < maxRequiredFee`: set `targetFee = maxRequiredFee` and restart the loop (asks
      the source for more).
   6. If `maxSignedSize > maxTxSize` return
      `errors.E(errors.Invalid, "signed tx size exceeds allowed maximum")` (no op attached). The size
      compared includes the change output even if no change is ultimately added.
   7. Build the transaction: `SerType = TxSerializeFull`, `Version = 1`, `TxIn = inputDetail.Inputs`,
      `TxOut = outputs`, `LockTime = 0`, `Expiry = 0`. The input and output slices are the caller's/source's
      slices, not copies.
   8. `changeAmount = inputDetail.Amount − targetAmount − maxRequiredFee`.
   9. If `changeAmount != 0` and `!IsDustAmount(changeAmount, changeScriptSize, relayFeePerKb)`:
      1. `changeScript, changeScriptVersion, err = fetchChange.Script()`; on error return
         `errors.E("txauthor.NewUnsignedTransaction", err)`.
      2. If `len(changeScript) > 2048` (`txscript.MaxScriptElementSize`) return
         `errors.E(errors.Invalid, "script size exceed maximum bytes pushable to the stack")`.
      3. Append `TxOut{Value: changeAmount, Version: changeScriptVersion, PkScript: changeScript}` after the
         caller outputs **in a freshly allocated slice** (the caller's `outputs` slice is not grown in
         place); `changeIndex = len(outputs)`.
   10. Otherwise (`changeAmount == 0` or dust): no change output, `changeIndex = −1`, and
       `maxSignedSize = EstimateSerializeSize(scriptSizes, outputs, 0)`. The whole remainder
       (`maxRequiredFee + changeAmount`) becomes the fee; the fee is not recomputed for the smaller size.
   11. Return `AuthoredTx{Tx, PrevScripts: inputDetail.Scripts, TotalInput: inputDetail.Amount, ChangeIndex,
       EstimatedSignedSerializeSize: maxSignedSize}`.

Properties the port must preserve:

- When a change output exists the fee is exactly `FeeForSerializeSize(rate, EstimateSerializeSize(inputSizes,
  outputs, changeScriptSize))`, and the change value is `TotalInput − Σ outputs − fee`.
- `ScriptSize()` (not the real script length) drives both the size estimate and the dust test.
- A dust remainder is never used to trigger selection of additional inputs
  (`wallet/txauthor/author_test.go:162-174`).
- With `outputs` empty (sweep), the "change" output is the only output. If the remainder is dust the
  result is a transaction with **zero outputs** and `ChangeIndex = −1`, returned without error.
- With values in range the loop terminates because each retry raises the target; a source that returns
  the same detail forever ends in `InsufficientBalance` on the next iteration. Corrected by the oracle
  (2026-11-19, QK-0744): a retry can also lower the target (the first estimate assumes one P2PKH input,
  so a source answering with none makes the second target smaller), and when the output total lies
  within the fee of `math.MaxInt64` the target wraps negative, the wallet's source answers with no
  inputs, and the loop asks for the same negative target forever. `purse-txauthor` reproduces that
  pending the owner's ruling.

### 4.6 Signing loop (`txauthor.AddAllInputScripts`, `wallet/txauthor/author.go:205-227`)

1. If `len(tx.TxIn) != len(prevPkScripts)` return the plain error
   `"tx.TxIn and prevPkScripts slices must have equal length"`.
2. For `i` in input order:
   `script, err = sign.SignTxOutput(secrets.ChainParams(), tx, i, prevPkScripts[i], SigHashAll, secrets,
   secrets, tx.TxIn[i].SignatureScript, true)`; on error return it unchanged; else store `script` in
   `tx.TxIn[i].SignatureScript`.

`isTreasuryEnabled` is always `true`, so treasury-gen outputs are signed as their P2PKH/P2SH sub-type. The
existing signature script is passed as `previousScript` and merged (3.4). For every class other than P2SH
and multisig the merge keeps the **longer** of the new and the previous script, and the previous one on a
tie (`txscript/v4@v4.1.2/sign/sign.go:547-558`), so an input that already carries a script at least as long
as the fresh signature script keeps its old script. Inputs signed earlier in the loop
stay signed if a later input fails; the function does not roll back.

`sign.SignTxOutput` behaviour relevant to the wallet (`txscript/v4@v4.1.2/sign/sign.go:217-631`): classify
the pkScript as version 0; for P2PK/P2PKH call `GetKey(address)` and build the script of 3.4; for P2SH call
`GetScript(address)` and sign the redeem script recursively (one level); for multisig sign with every key
`GetKey` can supply, silently skipping addresses whose key lookup fails; null-data scripts fail with
`"can't sign NULLDATA transactions"`, anything else with `"can't sign unknown transactions"`.

Observed by the oracle (2026-11-19). Every secp256k1 key `GetKey` returns goes through
`secp256k1.PrivKeyFromBytes`, which never fails: any length, at most the first 32 bytes taken,
left-padded, reduced modulo N; ECDSA then signs even with a zero key, and Schnorr fails on one with
`"cannot sign tx input: private key is zero"` (QK-0682). A P2SH redeem script that is itself P2SH,
plain or stake-tagged, makes `sign` look up a second script and return it unsigned; the signature
script is that script's opcodes followed by a push of the redeem script, built with the script
builder's limits (a step that would exceed them is dropped, the script so far kept), and the merge
with an existing script recurses until the process dies when both scripts are nonempty and parse and
the new one ends in a push of a plain P2SH script (QK-0746).

### 4.7 `Wallet.NewUnsignedTransaction` (`wallet/createtx.go:82-161`)

Op: `"wallet.NewUnsignedTransaction"`.

1. Lock `lockedOutpointMu` (held until return).
2. Open a read transaction (`walletdb.View`). Inside:
   1. Read the main-chain tip height.
   2. If `account != ImportedAddrAccount (0x7fffffff)`: read `lastAcct = Manager.LastAccount`; if
      `account > lastAcct` return `errors.E(errors.NotExist, "missing account")`.
   3. If `inputSource == nil`: create `MakeInputSource(dbtx, account, minConf, tipHeight, ignoreInput)`
      where `ignoreInput` reports membership in the locked-outpoint set, then:
      - `OutputSelectionAlgorithmDefault`: use `SelectInputs` as is.
      - `OutputSelectionAlgorithmAll`: wrap so that every call asks for `dcrutil.MaxAmount` and converts
        an `InsufficientBalance` error from the source into success.
      - anything else: `errors.E(errors.Invalid, fmt.Errorf("unknown output selection algorithm %v", algo))`.
   4. If `changeSource == nil`: use `p2PKHChangeSource{updates: &changeSourceUpdates, account, wallet,
      ctx: context.Background()}`. Its gap policy is the zero value, `gapPolicyError`.
   5. `txauthor.NewUnsignedTransaction(outputs, relayFeePerKb, inputSource, changeSource,
      chainParams.MaxTxSize)`.
3. Any error is returned as `errors.E(op, err)`.
4. If the change source recorded address updates, write them with
   `changeSourceUpdates.UpdateDB(ctx, w, nil)` (a separate write transaction, after the read transaction
   closed); errors as `errors.E(op, err)`.
5. Return the authored transaction. Selected outpoints are **not** added to the lock set and nothing is
   recorded about the transaction.

When the caller passes its own `inputSource`, `account`, `minConf` and `algo` are used only for the account
existence check and the default change source.

### 4.8 `authorTx` (`wallet/createtx.go:388-502`)

Parameters (struct fields): `outputs`, `account` (source), `changeAccount`, `minconf`,
`randomizeChangeIdx`, `txFee` (rate, atoms/kB), `dontSignTx`, `isTreasury`.

1. Lock `lockedOutpointMu`. A deferred function removes every outpoint added in step 2.5 from the lock set
   and then unlocks the mutex.
2. Open a read transaction. Inside:
   1. Read tip height; `inputSource = MakeInputSource(dbtx, account, minconf, tipHeight, ignoreInput)`.
   2. Change source: if `isTreasury`, `p2PKHTreasuryChangeSource{updates, account: changeAccount, wallet,
      ctx}` (gap policy `gapPolicyError`); otherwise `p2PKHChangeSource{updates, account: changeAccount,
      wallet, ctx, gapPolicy: gapPolicyWrap}`. `updates` points at a fresh `accountBranchChildUpdates`.
   3. `atx = txauthor.NewUnsignedTransaction(outputs, txFee, inputSource.SelectInputs, changeSource,
      chainParams.MaxTxSize)`; on error return it.
   4. (No algorithm option: always the default selection.)
   5. Add every input's previous outpoint to the lock set and remember it for the deferred removal.
   6. If `atx.ChangeIndex >= 0` and `randomizeChangeIdx`: `atx.RandomizeChangePosition()`.
   7. If `isTreasury`: if `atx.Tx.Version != 1` return
      `errors.E(op, "violated assumption: expected unsigned tx to be version 1")`; else set
      `atx.Tx.Version = 3` (`wire.TxVersionTreasury`).
   8. If `!dontSignTx`: build `secretSource{Manager, addrmgrNs}`, call `atx.AddAllInputScripts(secrets)`,
      then run every accumulated key-zeroing callback (also on error), and return the signing error.
3. Any error from step 2 is returned as `errors.E(op, err)`.
4. If `atx.ChangeIndex >= 0` and `account == ImportedAddrAccount`, log at warning level:
   `Spend from imported account produced change: moving <amount> from imported account into default account.`
   (the amount formatted with `dcrutil.Amount.String`, e.g. `0.5 DCR`).
5. `checkHighFees(atx.TotalInput, atx.Tx)`; error as `errors.E(op, err)`.
6. If `!dontSignTx`: `validateMsgTx(op, atx.Tx, atx.PrevScripts)`; error as `errors.E(op, err)`.
7. Store `atx` and `changeSourceUpdates` on the struct and return nil.

Notes: the high-fee check runs before script validation; with `dontSignTx` the high-fee check sees the
unsigned size. The transaction is neither recorded nor published by `authorTx`.

### 4.9 `recordAuthoredTx` (`wallet/createtx.go:510-540`)

1. `rec = udb.NewTxRecordFromMsgTx(a.atx.Tx, time.Now())`; error as `errors.E(op, err)`.
2. Lock `lockedOutpointMu` (held until return).
3. One write transaction:
   1. `a.changeSourceUpdates.UpdateDB(ctx, w, dbtx)`.
   2. `watch, err = w.processTransactionRecord(ctx, dbtx, rec, nil, nil)`.
4. Error as `errors.E(op, err)`. On success `a.watch = watch`.

The write is committed before publication so that a notification of the transaction arriving from the
backend finds it already recorded.

### 4.10 `publishAndWatch` (`wallet/createtx.go:331-365`)

1. If `n == nil`: `n = w.NetworkBackend()`; error as `errors.E(op, err)` (kind `NoPeers`).
2. `n.PublishTransactions(ctx, tx)`. On error:
   1. Log error: `Abandoning transaction <hash> which failed to publish`.
   2. `w.AbandonTransaction(ctx, &hash)`; if that fails log error `Cannot abandon <hash>: <err>`.
   3. Return `errors.E(op, publishErr)`.
3. `w.watchHDAddrs(ctx, false, n)`; on error log error
   `Failed to watch for future address usage after publishing transaction: <err>` and continue.
4. If `len(watch) > 0`: `n.LoadTxFilter(ctx, false, nil, watch)`; on error log error
   `Failed to watch outpoints: <err>` and continue.
5. Return nil.

`SendOutputs` and `SendOutputsToTreasury` (`wallet/wallet.go:4572-4643`) are: for each output
`txrules.CheckOutput(output, RelayFee())` (error as `errors.E(op, err)`), then `authorTx`, `recordAuthoredTx`,
`publishAndWatch(ctx, op, nil, tx, watch)`, returning the transaction hash. They differ only in
`randomizeChangeIdx` (true vs false), `isTreasury` (false vs true) and op (`"wallet.SendOutputs"` vs
`"wallet.SendOutputsToTreasury"`).

### 4.11 Pay to multisig (`txToMultisig`, `wallet/createtx.go:544-724`)

`txToMultisig` locks `lockedOutpointMu`, runs `txToMultisigInternal` inside a single write transaction, and
wraps any error as `errors.E(op, err)`.

`txToMultisigInternal(ctx, op, dbtx, account, amount, pubkeys, nRequired, minconf)`:

1. `n = w.NetworkBackend()`; on error return it (kind `NoPeers`).
2. Read the tip height.
3. Fixed fee allowance `feeEstForTx` by network magic: `50_000_000` atoms (`5e7`) for mainnet
   (`0xd9b400f9`), for the constant `0x48e7a065` (the retired testnet2 magic) and for testnet3
   (`0xb194aa75`); `30_000` atoms (`3e4`) for every other network.
4. `amountRequired = amount + feeEstForTx`.
5. `eligible = findEligibleOutputsAmount(dbtx, account, minconf, amountRequired, tipHeight, 0, 0)`; error as
   `errors.E(op, err)` (typically kind `InsufficientBalance`). If `eligible == nil` return
   `errors.E(op, "not enough funds to send to multisig address")`.
6. Add every selected outpoint to the lock set; a deferred function removes them when the function returns.
7. Create the transaction; add one input per selected output (in selection order), summing `totalInput`,
   and record sigScript size `RedeemP2SHSigScriptSize` (110) per input.
8. `msScript = stdscript.MultiSigScriptV0(int(nRequired), pubkeys...)`; error as `errors.E(op, err)`.
9. `Manager.ImportScript(addrmgrNs, msScript)`; an error of kind `Exist` is ignored, others returned as
   `errors.E(op, err)`.
10. `scAddr = stdaddr.NewAddressScriptHashV0(msScript, params)`; add output 0 with value `amount` and the
    address's payment script and version.
11. `changeSize = 25` if `totalInput > amount + feeEstForTx`, else 0.
12. `feeSize = EstimateSerializeSize(scriptSizes, tx.TxOut, changeSize)`;
    `feeEst = FeeForSerializeSize(w.RelayFee(), feeSize)`.
13. If `totalInput < amount + feeEst` return `errors.E(op, errors.InsufficientBalance)`.
14. If `totalInput > amount + feeEst`: obtain a change script from
    `p2PKHChangeSource{dbtx, account, wallet, ctx}` (gap policy `gapPolicyError`, address row written in
    the open write transaction); on error return it unwrapped; append a change output with value
    `totalInput − (amount + feeEst)`. No dust test is applied.
15. `signP2PKHMsgTx(tx, eligible, addrmgrNs)`; error as `errors.E(op, err)`.
16. `checkHighFees(totalInput, tx)`; error as `errors.E(op, err)`.
17. `n.PublishTransactions(ctx, tx)`; error as `errors.E(op, err)`.
18. `n.LoadTxFilter(ctx, false, [scAddr], nil)`; error as `errors.E(op, err)`.
19. `insertMultisigOutIntoTxMgr(dbtx, tx, 0)`; error as `errors.E(op, err)`.
20. Return `CreatedTx{MsgTx: tx, ChangeAddr: nil, ChangeIndex: −1}`, `scAddr`, `msScript`.

The transaction is published while the write transaction is still open; an error in steps 18–19 or at
commit rolls the database back after the network has the transaction. There is no `validateMsgTx` call.

### 4.12 Consolidation (`compressWallet`, `wallet/createtx.go:769-920`)

`compressWallet` locks `lockedOutpointMu`, runs `compressWalletInternal` in one write transaction, wraps
errors as `errors.E(op, err)`.

`compressWalletInternal(ctx, op, dbtx, maxNumIns, account, changeAddr)`:

1. `n = w.NetworkBackend()`; error as `errors.E(op, err)`.
2. Read tip height. `eligible = findEligibleOutputs(dbtx, account, 1, tipHeight)` (minconf fixed at 1);
   error as `errors.E(op, err)`.
3. If `len(eligible) <= 1` return `errors.E(op, "too few outputs to consolidate")`.
4. Lock all eligible outpoints; deferred removal at return.
5. If `changeAddr == nil`: `changeAddr = newChangeAddress(ctx, op, nil, dbtx, "", account, gapPolicyIgnore)`;
   error as `errors.E(op, err)`.
6. Create the transaction with output 0 = `{Value: 0, PkScript, Version}` from `changeAddr.PaymentScript()`.
7. `maximumTxSize = params.MaxTxSize`, except on mainnet where it is `100000`.
8. For each eligible output in order, while `count < maxNumIns`:
   1. Append `RedeemP2PKHSigScriptSize` (108) to the size list and compute
      `newSzEst = EstimateSerializeSize(sizes, tx.TxOut, 0)`.
   2. If `newSzEst > maximumTxSize` stop.
   3. `szEst = newSzEst`; add the input; `totalAdded += value`; `count++`.
9. `feeRate = w.RelayFee()`; `feeEst = FeeForSerializeSize(feeRate, szEst)`.
10. `tx.TxOut[0].Value = totalAdded − feeEst`. If `IsDustOutput(tx.TxOut[0], feeRate)` return
    `errors.E(op, errors.InsufficientBalance)`.
11. `signP2PKHMsgTx`; then `validateMsgTx(op, tx, creditScripts(selected))`; each error as
    `errors.E(op, err)`.
12. `checkHighFees(totalAdded, tx)`; error as `errors.E(op, err)`.
13. `n.PublishTransactions(ctx, tx)`; error as `errors.E(op, err)`.
14. `insertIntoTxMgr(dbtx, tx)` (error as `errors.E(op, err)`), then `insertCreditsIntoTxMgr(op, dbtx, tx, rec)`.
15. Log info `Successfully consolidated funds in transaction <hash>` and return the hash.

`eligible` is in transaction-store iteration order (mined unspent outputs ascending by
`txhash ‖ index(big-endian)`); it is not shuffled, so consolidation always takes the lowest-keyed outputs.
No filter reload follows the publish.

### 4.13 Coin selection

#### 4.13.1 Spendability predicates (`wallet/wallet.go:4404-4431`, duplicated in `wallet/udb/txmined.go:2931-2958`)

- `confirms(txHeight, curHeight)` = 0 when `txHeight == −1` or `txHeight > curHeight`, else
  `curHeight − txHeight + 1`.
- `confirmed(minconf, txHeight, curHeight)` = `confirms(txHeight, curHeight) >= minconf`.
- `coinbaseMatured(params, txHeight, curHeight)` = `txHeight >= 0 && curHeight − txHeight + 1 > CoinbaseMaturity`.
- `ticketChangeMatured(params, txHeight, curHeight)` = `txHeight >= 0 && curHeight − txHeight + 1 > SStxChangeMaturity`.

All arithmetic is int32. Unmined outputs have height −1.

#### 4.13.2 Default selection: `udb.Store.MakeInputSource` (`wallet/udb/txmined.go:3030-3351`)

`MakeInputSource(dbtx, account, minConf, syncHeight, ignore)` returns a stateful source bound to the read
transaction. State (initially empty): `currentTotal`, `currentInputs`, `currentScripts`,
`redeemScriptSizes`, `seen` (set of unspent-bucket keys picked by the random search), `randTries = 0`,
`remainingKeys = nil`, and `numUnspent` = the number of keys in the mined-unspent bucket at creation time.
If `minConf != 0` it logs at debug level `Unspent bucket k/v count: <numUnspent>`.

`skip(k, v)` (used for mined-unspent entries) is true when: the outpoint is spent by an unmined
transaction; or the value fails to decode; or `!confirmed(minConf, blockHeight, syncHeight)`; or `k` is in
`seen`.

`SelectInputs(target)` repeats the following while `currentTotal < target` or `target == 0`:

1. **Pick a candidate key.**
   - If `minConf != 0 && target != 0 && randTries < numUnspent/2` (integer division): `randTries++`;
     `k, v = randomUTXO(dbtx, skip)` (4.13.3); if `k != nil` add it to `seen`.
   - Else if `remainingKeys == nil` (first time the random phase is over or was never entered):
     1. If `randTries > 0` log debug `Abandoned random UTXO selection attempts after <randTries> tries`.
     2. Collect, in bucket order, a copy of every mined-unspent key for which `skip` is false.
     3. If `minConf == 0`, also collect every unmined-credit key that is not in `seen`, whose transaction is
        not marked unpublished, and whose recorded stake opcode is none of `OP_SSTX`, `OP_SSGEN`,
        `OP_SSTXCHANGE`, `OP_SSRTX`, `OP_TADD`, `OP_TGEN`. These are flagged unmined. Unlike
        `UnspentOutputs` (4.13.4), this scan does **not** test whether the unmined credit is already spent
        by another unmined transaction (`wallet/udb/txmined.go:3137-3164`; the unmined-input check exists
        only in `skip`, which is applied to mined keys), so with `minConf == 0` such a credit can be
        selected.
     4. Shuffle the combined list (`rand.ShuffleSlice`, Fisher–Yates from the last index down).
   - If no key was picked in this iteration: if `remainingKeys` is empty (including still nil), **stop the
     loop**; otherwise pop the first remaining key.
2. **Load and filter the candidate.** For a mined key (36 bytes `txhash ‖ index`) with unspent value
   (`height(4) ‖ blockhash(32)`):
   1. Form the 72-byte credit key `txhash ‖ height ‖ blockhash ‖ index` and load the credit value.
   2. Look up the pkScript; determine the credit's account (recorded account, falling back to address
      lookup of the first address extracted from the script). Any error aborts the whole selection with
      that error. If the account differs from `account`, skip.
   3. Skip if the credit is flagged spent, if its amount is 0, or if its stake opcode is `OP_SSTX`.
   4. Skip if `!confirmed(minConf, txHeight, syncHeight)`.
   5. Skip if the opcode is "non-stake" and the credit is flagged coinbase and `!coinbaseMatured`.
   6. Skip if the opcode is `OP_SSGEN`, `OP_SSRTX`, `OP_TADD` or `OP_TGEN` and `!coinbaseMatured`.
   7. Skip if the opcode is `OP_SSTXCHANGE` and `!ticketChangeMatured`.
   8. Tree = stake if the opcode is not "non-stake", else regular.

   For an unmined key: look up the pkScript and account (skip on account mismatch, abort on error), read
   the amount, set the tree by the same opcode rule.
3. If `ignore != nil && ignore(&outpoint)` skip (the wallet passes the locked-outpoint test).
4. Classify the pkScript (`stdscript.DetermineScriptType` at version 0, then `StakeSubScriptType`):
   P2PKH-secp256k1 → sigScript size 108; P2PK-secp256k1 → 74; anything else → log error
   `Unexpected script class for credit: <class>` and skip.
5. Accept: `currentTotal += amount`; append `wire.NewTxIn(&outpoint, amount, nil)`, the pkScript and the
   size.

It then returns `InputDetail{Amount: currentTotal, Inputs, Scripts, RedeemScriptSizes}` (the accumulated
slices themselves). The source never returns `InsufficientBalance`; a short `Amount` is the signal.

Observable properties:

- Every random pick consumes a try whether or not the candidate is later rejected in step 2–4; a rejected
  random candidate stays in `seen` and is never reconsidered.
- A nil result from `randomUTXO` during the random phase ends selection immediately (every confirmed,
  not-unmined-spent mined output has then been seen, and with `minConf != 0` unmined credits are not
  eligible).
- `target == 0` selects every eligible output and never uses the random phase.
- State persists across calls: a later call with a larger target extends the same selection.

#### 4.13.3 `randomUTXO` and `RandomUTXO` (`wallet/udb/txmined.go:2358-2459`)

`randomUTXO(dbtx, skip)`:

1. Draw 33 random bytes. The first 32 are a seek key; the low bit of byte 32 selects the first search
   direction (`1` = backward).
2. Position a cursor on the mined-unspent bucket with `Seek(seekKey)` (first key ≥ seek key). For a
   backward-first search, step once to the previous key before starting.
3. Walk in the chosen direction. Collect every key for which `skip` is false; once at least one key has
   been collected, stop at the first key whose transaction hash (first 32 bytes) differs from the first
   collected key's hash.
4. If anything was collected, pick one uniformly (`rand.IntN(len)`), re-seek to it and return its
   key/value.
5. Otherwise search the other direction the same way: forward starts at `Seek(seekKey)` itself; backward
   starts one before it. Return a uniform pick if anything was collected.
6. Otherwise return nil.

The effect is "the outputs of the nearest transaction (in key order) that has an acceptable output",
then a uniform choice among that transaction's acceptable outputs in the walked range.

`RandomUTXO(dbtx, minConf, syncHeight)`: if `minConf == 0` return
`errors.E(errors.Invalid, "optimized random utxo selection not possible with minConf=0")`; otherwise call
`randomUTXO` with a `skip` that rejects outputs spent by unmined transactions and outputs that are not
`confirmed(minConf, …)`, and return the full credit (or nil when none).

#### 4.13.4 `findEligibleOutputs` (`wallet/createtx.go:1734-1825`)

Reads every unspent output with `txStore.UnspentOutputs` (mined-unspent bucket in key order, skipping those
spent by unmined transactions, then unmined credits in key order, skipping those spent by unmined
transactions and those of unpublished transactions). An output is eligible when all hold, tested in this
order:

1. Its outpoint is not in the lock set.
2. `confirmed(minconf, output.Height, currentHeight)`.
3. `stdscript.ExtractAddrs(0, pkScript, params)` yields exactly one address.
4. By script class:
   - stake-gen P2PKH/P2SH, stake-revocation P2PKH/P2SH, treasury-add, treasury-gen P2PKH/P2SH: requires
     `coinbaseMatured`;
   - stake-change P2PKH/P2SH: requires `ticketChangeMatured`;
   - P2PKH-secp256k1: requires `coinbaseMatured` only if the credit is from a coinbase;
   - every other class (P2PK, plain P2SH, ticket submissions, multisig, alternative signature schemes,
     non-standard): ineligible.
5. `Manager.AddrAccount(address)` succeeds and equals `account`.

Each eligible output becomes `Input{OutPoint: credit.OutPoint, PrevOut: {Value: amount, Version: 0,
PkScript}}`. Order is the store's iteration order. Errors: only from `UnspentOutputs`, returned unwrapped.

#### 4.13.5 `findEligibleOutputsAmount` (`wallet/createtx.go:1829-1987`)

Parameters: `amount` (target, 0 = no target), `minAmount` (0 = none), `maxResults` (0 = no cap).

`skip(output)` is the eligibility test of 4.13.4 negated, with two additions: an output already in the
local `seen` set is skipped (tested first), and when `minAmount != 0` an output with
`Amount < minAmount` is skipped (tested after the confirmation test).

1. `maxTries = 0`. If `(amount != 0 || maxResults != 0) && minconf > 0`:
   `numUnspent = txStore.UnspentOutputCount(dbtx)`; log debug `Unspent bucket k/v count: <numUnspent>`;
   `maxTries = numUnspent / 2`.
2. Random phase, for `randTries` from 0 while `< maxTries`:
   1. `output = txStore.RandomUTXO(dbtx, minconf, currentHeight)`; on error return it. If nil, leave the
      phase (without counting the try).
   2. If `skip(output)` continue.
   3. Add to `seen`, append to the result, add its amount to the running total.
   4. If `amount != 0 && total >= amount`: return the result.
   5. If `maxResults != 0 && len(result) == maxResults`: return the result.
3. If `randTries > 0` log debug `Abandoned random UTXO selection attempts after <randTries> tries`.
4. **Discard** the random-phase result: empty the result, drop `seen`, zero the total.
5. `unspent = txStore.UnspentOutputs(dbtx)` (error returned unwrapped); shuffle it (`rand.ShuffleSlice`).
6. For each output: if `skip` continue; append; add amount; apply the two early returns of steps 2.4–2.5.
7. If `amount != 0 && total < amount` return `nil, errors.InsufficientBalance` (the bare kind value).
8. Return the result (possibly empty or nil).

Random draws are independent (with replacement); a repeated draw is rejected through `seen`.

### 4.14 Reserving outputs (`wallet/createtx.go:1672-1729`)

`ReserveOutputsForAmount(ctx, account, amount, minconf)`: lock `lockedOutpointMu`; in a read transaction
call `findEligibleOutputsAmount(dbtx, account, minconf, amount, tipHeight, 0, 0)`; add every returned
outpoint to the lock set; return the inputs. Errors are returned unwrapped (so an insufficient balance is
the bare `errors.InsufficientBalance` kind). The outpoints stay locked until the caller unlocks them.

`reserveOutputs(ctx, account, minconf)` is the same with `findEligibleOutputs` (everything eligible).

Lock-set API (`wallet/wallet.go:4250-4283`): `LockOutpoint(hash, index)`, `UnlockOutpoint(hash, index)`,
`LockedOutpoint(hash, index) bool`, `ResetLockedOutpoints()` (replaces the set with an empty one). The set
is keyed by `(hash, index)` only (no tree), lives in memory only, and is empty after every start.

### 4.15 `signP2PKHMsgTx` and script validation (`wallet/createtx.go:728-757,1992-2023`)

`signP2PKHMsgTx(msgtx, prevOutputs, addrmgrNs)`:

1. If `len(prevOutputs) != len(msgtx.TxIn)` return the plain error
   `Number of prevOutputs (<a>) does not match number of tx inputs (<b>)`.
2. For each `i`:
   1. `addrs = ExtractAddrs(prevOut.Version, prevOut.PkScript, params)`. If `len(addrs) != 1`,
      **silently leave the input unsigned** and continue.
   2. If the address is not a P2PKH-secp256k1 address return
      `errors.E(errors.Bug, "previous output address is not P2PKH")`.
   3. `privKey, done = Manager.PrivateKey(addrmgrNs, address)`; on error return it unwrapped. From the
      address manager (`wallet/udb/addressmanager.go:437-455,2470-2520`, `wallet/udb/addressdb.go:1330`):
      kind `NotExist` for an address with no row; kind `Locked` when the account's private key is
      encrypted and not unlocked; the **kind-less** `no private key for <account name>/<branch>/<index>`
      when the account holds no private key material at all (the watching-only case; `PrivateKey` never
      returns kind `WatchingOnly`); kind `Invalid` for a script address row; kind `Crypto` for an
      undecryptable imported key. `done` is deferred to function return.
   4. `sigscript = sign.SignatureScript(msgtx, i, prevOut.PkScript, SigHashAll, privKey.Serialize(),
      STEcdsaSecp256k1, true)`; error as `errors.E(errors.Op("txscript.SignatureScript"), err)`.
   5. Store the script on the input.

Stake-tagged P2PKH previous outputs are accepted (address extraction sees through the tag and the full
tagged script is the signed subscript).

`validateTxInput(op, tx, i, prevScript)`: create `txscript.NewEngine(prevScript, tx, i, sanityVerifyFlags,
0, nil)`; creation error as `errors.E(op, err)`; `Execute()` error is logged at error level as
`Script validation failed (outpoint <hash:index> pkscript <hex> sigscript <hex>): <err>` and returned as
`errors.E(op, errors.ScriptFailure, err)`. `validateMsgTx` validates inputs `0..len(prevScripts)−1` in
order and stops at the first failure.

### 4.16 Split transactions (`wallet/createtx.go:1000-1150`)

These build the transaction whose outputs fund individual tickets. The surrounding purchase flow is owned
by the ticket-purchase spec; the construction steps are:

`individualSplit(ctx, req, neededPerTicket)` (op `"individualSplit"`):

1. `splitTxAddr = NewInternalAddress(ctx, req.SourceAccount, WithGapPolicyWrap())` (opens its own write
   transaction).
2. Build `req.Count` identical outputs `{Value: neededPerTicket, PkScript, Version}` paying that one
   address; `outIndexes = [0, 1, …, Count−1]`.
3. `authorTx` with `account = req.SourceAccount`, `changeAccount = req.ChangeAccount`,
   `minconf = req.MinConf`, `randomizeChangeIdx = false`, `txFee = w.RelayFee()`,
   `dontSignTx = req.DontSignTx`, `isTreasury = false`.
4. Unless `req.DontSignTx`: `recordAuthoredTx`, then `publishAndWatch(ctx, op, nil, tx, watch)`.
5. Return the transaction and indexes. With `DontSignTx` the change-address row updates collected by
   `authorTx` are never written.

`mixedSplit(ctx, req, neededPerTicket)`:

1. Build `req.Count` placeholder outputs `{Value: neededPerTicket, Version: 0, PkScript: 25 zero bytes}`.
2. Lock `lockedOutpointMu`; in a read transaction: input source for `(req.SourceAccount, req.MinConf)`;
   change source `p2PKHChangeSource{updates, account: req.ChangeAccount, gapPolicy: gapPolicyIgnore}`;
   `txauthor.NewUnsignedTransaction(placeholders, w.RelayFee(), SelectInputs, changeSource, MaxTxSize)`;
   add all selected outpoints to the lock set. Unlock the mutex.
3. For each input log info `Selected input <outpoint> (<amount>) for ticket purchase split transaction`.
4. Take the change output if one exists; discard it (use no change) if its value is below
   `smallestMixChange(relayFee)` (mixing spec).
5. Create a coinjoin with a generator for `(req.MixedSplitAccount, req.MixedAccountBranch)`, the change
   output, mixed amount `neededPerTicket`, an expiry from `dicemixExpiry`, and `req.Count` mixed outputs;
   add each selected input with its previous script (script version 0); run the mix.
6. Log info `Completed CoinShuffle++ mix of ticket split transaction <hash>`; return the mixed transaction
   and the mixed output indexes.
7. On return (any outcome) the selected outpoints are removed from the lock set; on success the
   change-address row updates are written with `UpdateDB(ctx, w, nil)` and its error becomes the return
   error.

### 4.17 `makeTicket` (`wallet/createtx.go:923-992`)

`makeTicket(params, input, addrVote, addrSubsidy, ticketCost)`:

1. New transaction; one input spending `input.OutPoint` with `ValueIn = input.PrevOut.Value` and an empty
   (non-nil) signature script.
2. If `addrVote == nil` return `errors.E(errors.Invalid, "nil vote address")`.
3. Output 0: `{Value: ticketCost, script/version: addrVote.VotingRightsScript()}`.
4. `_, amountsCommitted, err = stake.SStxNullOutputAmounts([input value], [0], ticketCost)`; return `err`
   unchanged (it fails when `ticketCost <= 0`). `amountsCommitted[0]` equals the input value.
5. Output 1: `{Value: 0, script/version: addrSubsidy.RewardCommitmentScript(amountsCommitted[0], 0, 16384)}`.
6. Output 2: `{Value: 0, script/version: StakeChangeScript()}` of the P2PKH address whose hash160 is 20
   zero bytes.
7. `stake.CheckSStx(tx)`; failure as `errors.E(errors.Op("stake.CheckSStx"), errors.Bug, err)`.

The ticket's implied fee is `input value − ticketCost`. The caller sets `Expiry`, signs with
`signP2PKHMsgTx`, validates with `validateMsgTx`, and applies `checkHighFees(input value, ticket)`.

### 4.18 Vote construction and signing (`wallet/createtx.go:2027-2176`)

`newVoteScript(voteBits)`: `data = uint16 LE(voteBits.Bits) ‖ voteBits.ExtendedBits`; return
`stdscript.ProvablyPruneableScriptV0(data)` (error kind `ErrTooMuchNullData` when `len(data) > 256`).

`createUnsignedVote(ticketHash, ticketPurchase, blockHeight, blockHash, voteBits, subsidyCache, params,
dcp0010Active, dcp0012Active)`:

1. `payKinds, hash160s, values, _, _, _ = stake.TxSStxStakeOutputInfo(ticketPurchase)`.
2. Variant: `SSVDCP0012` if `dcp0012Active`, else `SSVDCP0010` if `dcp0010Active`, else `SSVOriginal`.
   `subsidy = subsidyCache.CalcStakeVoteSubsidyV3(int64(blockHeight), variant)`.
3. `rewards = stake.CalculateRewards(values, ticketPurchase.TxOut[0].Value, subsidy)`.
4. New transaction (version 1) with the two inputs and the outputs of 3.3/3.5: stakebase input, ticket
   input, block-reference output (value 0, version 0), vote-bits output (value 0, version 0), then for
   each commitment `i` an output `{Value: rewards[i], script/version: PayVoteCommitmentScript()}` of the
   P2SH address (when `payKinds[i]`) or P2PKH address built from `hash160s[i]`.
5. Errors: from `newVoteScript` and from address construction, returned unchanged.

`blockHeight` and `blockHash` identify the block being voted on (the parent of the block the vote will be
mined in); the subsidy is computed at that same height value.

`signVoteOrRevocation(addrmgrNs, ticketPurchase, tx, isVote)`:

1. `inputToSign = 1` for a vote, `0` for a revocation.
2. `script = sign.SignTxOutput(params, tx, inputToSign, ticketPurchase.TxOut[0].PkScript, SigHashAll,
   getKey, getScript, tx.TxIn[inputToSign].SignatureScript, true)` where `getKey` is
   `Manager.PrivateKey` (returning `(key.Serialize(), STEcdsaSecp256k1, true)`) and `getScript` is
   `Manager.RedeemScript`.
3. Error as `errors.E(errors.Op("txscript.SignTxOutput"), errors.ScriptFailure, err)`.
4. Store the script. Key-zeroing callbacks run at return.

`signVote` is `signVoteOrRevocation(…, true)`. No caller passes `false` at the pin.

### 4.19 Multisig output helpers (`wallet/multisig.go`)

`FetchP2SHMultiSigOutput(ctx, outPoint)` (op `"wallet.FetchP2SHMultiSigOutput"`):

1. In a read transaction: `mso = txStore.GetMultisigOutput(txmgrNs, outPoint)` (kind `NotExist`,
   `no multisig output for outpoint <op>` when absent); build the P2SH address from `mso.ScriptHash`
   (construction error ignored); `redeemScript = Manager.RedeemScript(addrmgrNs, addr)`.
2. Error as `errors.E(op, err)`.
3. Rebuild the P2SH address; a construction error here is returned **unwrapped**.
4. Return `P2SHMultiSigOutput{OutPoint: *mso.OutPoint, OutputAmount: mso.Amount, ContainingBlock: {Hash:
   mso.BlockHash, Height: int32(mso.BlockHeight)}, P2SHAddress, RedeemScript, M: mso.M, N: mso.N,
   Redeemer}` where `Redeemer` is nil unless `mso.Spent`, in which case it is `{TxHash: mso.SpentBy,
   InputIndex: mso.SpentByIndex}`.

`PrepareRedeemMultiSigOutTxOutput(msgTx, p2shOutput, pkScript)` (op
`"wallet.PrepareRedeemMultiSigOutTxOutput"`):

1. One sigScript size of `RedeemP2SHSigScriptSize` (110) per existing input of `msgTx`.
2. `txOut = wire.NewTxOut(0, *pkScript)` (script version 0 regardless of the destination address).
3. `feeEst = FeeForSerializeSize(w.RelayFee(), EstimateSerializeSize(sizes, [txOut], 0))`.
4. If `feeEst >= p2shOutput.OutputAmount` return
   `errors.E(op, fmt.Errorf("estimated fee %v is above output value %v", feeEst, OutputAmount))` (amounts
   formatted with `dcrutil.Amount.String`).
5. `txOut.Value = OutputAmount − feeEst`; append to `msgTx`.

### 4.20 `nextAddress` (`wallet/addresses.go:505-649`)

`nextAddress(ctx, op, updates, maybeDBTX, accountName, account, branch, callOpts...)`. Exactly one of three
persistence modes applies:

| Mode | `updates` | `maybeDBTX` | Persistence of the address row |
|---|---|---|---|
| Deferred | non-nil | nil | Appended to `*updates`; the caller writes it later with `UpdateDB`. |
| In transaction | nil | non-nil | Written immediately in the caller's open write transaction. |
| Self-contained | nil | nil | `nextAddress` opens its own write transaction and commits it. |

Steps:

1. If both `updates` and `maybeDBTX` are non-nil return
   `errors.E(op, errors.Bug, "nextAddress must not provide both a slice to defer db updates and an open write transaction")`.
2. Self-contained mode only: create a local updates slice; `dbtx = w.db.BeginReadWriteTx()` (error as
   `errors.E(op, err)`); register a deferred finaliser:
   - if the function is returning an error: roll back;
   - else `updates.UpdateDB(ctx, w, dbtx)`; on error roll back and return that error (unwrapped);
   - else commit and return the commit's error.
3. Apply call options to a zero-valued options struct (default policy `gapPolicyError`).
4. Lock `addressBuffersMu` (released at return, **before** the finaliser of step 2 runs).
5. Look up `addressBuffers[account]`; if absent return
   `errors.E(op, errors.NotExist, fmt.Errorf("account %d", account))`.
6. Select the branch buffer: 0 → external, 1 → internal, else
   `errors.E(op, errors.Invalid, "branch must be external (0) or internal (1)")`.
7. Loop:
   1. If `ctx.Err() != nil` return it unwrapped.
   2. If `cursor >= gapLimit`, by policy:
      - `gapPolicyError`: return
        `errors.E(op, errors.Policy, "generating next address violates the unused address gap limit policy")`.
      - `gapPolicyIgnore`: if `cursor % gapLimit == 0` and a network backend is set, derive `gapLimit`
        addresses starting at child `lastUsed + 1 + cursor` (`deriveChildAddresses`; error as
        `errors.E(op, err)`) and `n.LoadTxFilter(ctx, false, addrs, nil)` (error returned unwrapped). If
        `cursor % gapLimit != 0` or no backend is set, do nothing.
      - `gapPolicyWrap`: `cursor = 0`.
   3. `childIndex = lastUsed + 1 + cursor` (wrapping uint32). If `childIndex >= 0x80000000` return
      `errors.E(op, fmt.Errorf("account %d branch %d exhausted", account, branch))`.
   4. `child = branchXpub.Child(childIndex)`. If the error is `hdkeychain.ErrInvalidChild`: `cursor++` and
      restart the loop. Any other error as `errors.E(op, err)`.
   5. `apkh = HD2Address(child, params)`: P2PKH-secp256k1 address of
      `hash160(compressed pubkey)`; error as `errors.E(op, err)`.
   6. Persist: deferred or self-contained mode → append `{account, branch, childIndex}` to the updates
      slice; in-transaction mode → `UpdateDB` of that single entry in `maybeDBTX`, error as
      `errors.E(op, err)`.
   7. `cursor++`.
   8. Log info `Returning address (account=<account> branch=<branch> child=<childIndex>)`.
   9. Return `xpubAddress{apkh, xpub: account xpub, account, accountName, branch, child: childIndex}`.

State after an error: `cursor` keeps any increments already applied (invalid children, and the returned
child if the failure is in the self-contained finaliser). Nothing writes the last-returned index
(`extret`/`intret`) to the database.

### 4.21 `accountBranchChildUpdates.UpdateDB` (`wallet/addresses.go:468-502`)

1. If the slice is empty return nil.
2. If `maybeDBTX == nil`: begin a write transaction (error returned unwrapped); at return commit if no
   error occurred (returning the commit error) or roll back.
3. For each entry in order: `Manager.SyncAccountToAddrIndex(ns, account, child, branch)`; return the first
   error unwrapped.

`SyncAccountToAddrIndex` errors: imported account → `errors.E(errors.Invalid, "cannot sync imported account
branch index")`; branch not 0/1 → `errors.E(errors.Invalid, fmt.Errorf("account branch %d", branch))`;
`child > 2^31 − 1` → `errors.E(errors.Invalid, fmt.Errorf("child index %d exceeds max", child))`.

### 4.22 Public allocators and lookups (`wallet/addresses.go:654-840`, `wallet/wallet.go:1952-1973`)

`notVotingAcct(ctx, op, account)`: in a read transaction read `AccountProperties(account)` (error as
`errors.E(op, err)`); if the account type is the imported-voting type return
`errors.E(op, errors.Invalid, "cannot use voting accounts for normal transactions")`.

`NewExternalAddress(ctx, account, opts...)` (op `"wallet.NewExternalAddress"`) and
`NewInternalAddress(ctx, account, opts...)` (op `"wallet.NewInternalAddress"`):

1. `notVotingAcct`; return its error.
2. `accountName, _ = w.AccountName(ctx, account)` (error ignored; name is empty on failure).
3. `nextAddress(ctx, op, nil, nil, accountName, account, branch, opts...)` (self-contained mode).

`newChangeAddress(ctx, op, updates, maybeDBTX, accountName, account, gap)`:

1. `notVotingAcct(ctx, op, account)` on the **original** account.
2. If `account == ImportedAddrAccount`: `account = 0` (default account).
3. `nextAddress(ctx, op, updates, maybeDBTX, accountName, account, 1, withGapPolicy(gap))`.

`NewChangeAddress(ctx, account)` (op `"wallet.NewChangeAddress"`): account name lookup (error ignored),
then `newChangeAddress(ctx, op, nil, nil, name, account, gapPolicyWrap)`.

`AddressAtIdx(ctx, account, branch, childIdx)` (op `"wallet.AddressAtIdx"`): under `addressBuffersMu`; no
buffer → `errors.E(op, errors.NotExist, fmt.Errorf("account %d", account))`; bad branch →
`errors.E(op, errors.Invalid, "branch must be external (0) or internal (1)")`; `childIdx >= 2^31` →
`errors.E(op, fmt.Errorf("child out of range"))`; derivation errors (including `ErrInvalidChild`) as
`errors.E(op, err)`. Returns an `xpubAddress` with an empty account name. Logs info
`Returning address (account=… branch=… child=…)`. Touches neither buffer state nor database.

`signingAddressAtIdx(ctx, op, account, childIdx)`: `AddressAtIdx(ctx, account, 0, childIdx)` then
`UpdateDB` of `{account, 0, childIdx}` in its own write transaction; errors as `errors.E(op, err)`.

`BIP0044BranchNextIndexes(ctx, account)` (op `"wallet.BIP0044BranchNextIndexes"`): under the mutex return
`lastUsed + 1 + cursor` for each branch; no buffer →
`errors.E(op, errors.NotExist, fmt.Errorf("account %v", account))`.

`CurrentAddress(account)` (op `"wallet.CurrentAddress"`): under the mutex derive the external child at
`lastUsed + 1 + cursor` and return its plain P2PKH address (not an `xpubAddress`). No buffer →
`errors.E(op, errors.NotExist, fmt.Errorf("no account %d", account))`; derivation errors as
`errors.E(op, err)`. It does not advance the cursor, apply a gap policy, skip invalid children, or write
anything.

### 4.23 Usage tracking and buffer re-synchronisation

`markUsedAddress(op, dbtx, addr)` (`wallet/addresses.go:718-746`):

1. `Manager.MarkUsed(dbtx, addr.Address())`; error as `errors.E(op, err)`.
2. If the address's account is the imported account, return nil.
3. `props = Manager.AccountProperties(ns, account)`; error as `errors.E(op, err)`.
4. `lastUsed` = the branch's last-used index from `props` (internal if `addr.Internal()`).
5. `SyncAccountToAddrIndex(ns, account, min(2^31 − 1, lastUsed + gapLimit), branch)`; error as
   `errors.E(op, err)`.
6. Log debug `Marked address <address> used`.

The in-memory buffers are **not** updated here. They catch up to the database in two places, both using
the same rule per branch, where `dbLastUsed` is the database value:

```
if dbLastUsed + 1 > buf.lastUsed + 1:            // wrapping uint32, so 0xffffffff means "none"
    buf.cursor += buf.lastUsed - dbLastUsed      // wrapping
    if buf.cursor > 0x7fffffff: buf.cursor = 0   // clamp what would be negative
    buf.lastUsed = dbLastUsed
```

- `watchHDAddrs` (4.27), for every buffered account, on every call that passes its rescan-point gate.
- `DiscoverActiveAddresses` (`wallet/discovery.go:893-938`), for each discovered account, while holding
  `addressBuffersMu` across the write transaction that records discovered usage.

The rule keeps the next child index (`lastUsed + 1 + cursor`) unchanged when the newly used child is below
it, and moves it to `dbLastUsed + 1` when the used child is at or beyond it
(`wallet/addresses_test.go:231-239`).

### 4.24 `SyncLastReturnedAddress` (`wallet/addresses.go:845-912`)

Op `"wallet.SyncLastReturnedAddress"`. Inputs `account`, `branch`, `child`.

1. Under `addressBuffersMu`: no buffer →
   `errors.E(op, errors.NotExist, fmt.Errorf("account %v", account))`; bad branch →
   `errors.E(op, errors.Invalid, "branch must be external (0) or internal (1)")`. Capture `branchXpub` and
   `lastUsed`. If `lastUsed != 0xffffffff && child > lastUsed`: `cursor = child − lastUsed`.
2. In a write transaction: `SyncAccountToAddrIndex(ns, account, child, branch)` then
   `MarkReturnedChildIndex(tx, account, branch, child)`. Errors returned unwrapped.
3. If a network backend is set: `lastWatched = lastUsed + gapLimit` (wrapping; a local value, not the
   buffer field). If `child <= lastWatched` return nil. Otherwise derive `child − lastWatched` addresses
   starting at child `lastUsed + 1 + gapLimit` and `LoadTxFilter(ctx, false, addrs, nil)`; both errors as
   `errors.E(op, err)`.

### 4.25 Change sources (`wallet/addresses.go:944-1002`)

| Type | `ScriptSize()` | `Script()` |
|---|---|---|
| `p2PKHChangeSource` | 25 | `newChangeAddress(src.ctx, "", src.updates, src.dbtx, "", src.account, src.gapPolicy)`; returns the address's `PaymentScript()` as `(script, version 0)`. |
| `p2PKHTreasuryChangeSource` | 26 | Same address allocation; returns `0xbd ‖ paymentScript` with the address's version. |

The op passed to `newChangeAddress` is the empty string. Gap policy per construction site:

| Site | Change account | Persistence mode | Gap policy | `ctx` |
|---|---|---|---|---|
| `Wallet.NewUnsignedTransaction` default (`createtx.go:133-140`) | the source `account` | deferred | `gapPolicyError` (zero value) | `context.Background()` |
| `authorTx`, non-treasury (`createtx.go:420-426`) | `changeAccount` | deferred | `gapPolicyWrap` | request ctx |
| `authorTx`, treasury (`createtx.go:413-418`) | `changeAccount` | deferred | `gapPolicyError` (zero value) | request ctx |
| `txToMultisigInternal` (`createtx.go:671-676`) | source `account` | in transaction | `gapPolicyError` (zero value) | request ctx |
| `mixedSplit` (`createtx.go:1037-1043`) | `req.ChangeAccount` | deferred | `gapPolicyIgnore` | request ctx |
| `compressWalletInternal` (direct `newChangeAddress`, `createtx.go:821-822`) | source `account` | in transaction | `gapPolicyIgnore` | request ctx |

Other change sources live with their callers and only need the `txauthor.ChangeSource` contract: the RPC
servers' fixed-script sources (`ScriptSize() = len(script)`) and `spendoutputs`' account source.

### 4.26 Address wrappers (`wallet/addresses.go:111-382,915-942`)

`KnownAddress(ctx, a)` (op `"wallet.KnownAddress"`): in a read transaction `ma = Manager.Address(ns, a)`,
`acctName = Manager.AccountName(ns, ma.Account())`; errors as `errors.E(op, err)`. Account kind:
`account < 0x7fffffff` → `AccountKindBIP0044`; `== 0x7fffffff` → `AccountKindImported`; greater →
`AccountKindImportedXpub`. Then `wrapManagedAddress(ma, acctName, kind)`.

`wrapManagedAddress(addr, account, kind)`:

| `addr` implements | Result |
|---|---|
| `udb.ManagedPubKeyAddress`, kind `Imported` | `managedP2PKHAddress`: P2PKH script, `ScriptLen() = 25`, `PubKey()`, `PubKeyHash()` |
| `udb.ManagedPubKeyAddress`, other kinds | `managedBIP0044Address`: as above plus `Path() = (acct, branch, child)` with `acct = addr.Account()` only for kind `BIP0044` (else 0), `branch = 1` if `addr.Internal()` else 0, `child = addr.Index()` |
| `udb.ManagedScriptAddress` | `managedP2SHAddress`: P2SH script, `ScriptLen() = 23`, `RedeemScript()` from the managed address |
| anything else | `errors.E(errors.Bug, fmt.Errorf("don't know how to wrap %T", a))` |

`String()` is the managed address's encoding; `PaymentScript()` returns version 0 and the scripts of 3.3.

`xpubAddress` (returned by `nextAddress`/`AddressAtIdx`):

| Method | Result |
|---|---|
| `String()` and all `stdaddr.StakeAddress` methods | From the embedded P2PKH address |
| `PaymentScript()` | `(0, 25-byte P2PKH script)` |
| `ScriptLen()` | 25 |
| `AccountName()` | The name supplied by the caller (often empty) |
| `AccountKind()` | `AccountKindImportedXpub` if `account > 0x7fffffff`, else `AccountKindBIP0044` |
| `Path()` | `(account, branch, child)`, with account reported as 0 when `account > 0x7fffffff` |
| `PubKey()` | `xpub.Child(branch).Child(child)` serialized compressed; **panics** on any derivation error |
| `PubKeyHash()` | The 20-byte hash160 |

`ImportedAddresses(ctx, account)`: if `account != "imported"` return
`errors.E("account does not record imported keys")`; else in a read transaction wrap every address of the
imported account (`Manager.ForEachAccountAddress(ns, 0x7fffffff, …)`) with kind `Imported` and the given
name. Any error is wrapped with the op `wallet.ImportedAddresses("<account>")` (Go `%q` quoting).

### 4.27 `watchHDAddrs` (`wallet/wallet.go:891-1065`)

`watchHDAddrs(ctx, firstWatch, n)` returns the number of addresses loaded.

1. If `!firstWatch`: `rp = w.RescanPoint(ctx)`; on error return it; if `rp != nil` (the wallet's
   transaction history is behind its headers) return `(0, nil)` without watching anything.
2. In a read transaction, for every account `0..=LastAccount` and every account
   `0x80000000..=LastImportedAccount`, read `AccountProperties` and record:
   `externalCount = min(LastReturnedExternalIndex + gapLimit, 2^31 − 1)` (the addition wraps, so "none
   returned" gives `gapLimit − 1`), `internalCount` likewise, and the last-returned and last-used indexes.
3. Under `addressBuffersMu`, for every buffered account: apply the re-synchronisation rule of 4.23 to both
   branches using the database last-used values; copy the branch xpubs; if `firstWatch` set each branch's
   `lastWatched` to its count, else read the branch's current `lastWatched` as the start index.
4. For every account (map iteration order), external branch then internal branch,
   `loadBranchAddrs(branchKey, start, end)` with `start` = 0 on first watch or the previous `lastWatched`
   otherwise, and `end` = the count:
   1. If `start == 0 && watchLast != 0 && end − gapLimit > watchLast` (uint32 arithmetic):
      `start = end − gapLimit − watchLast`.
   2. In chunks of 256 children from `start` through `end` **inclusive**, run up to `NumCPU` concurrent
      tasks; each derives its chunk's addresses (skipping invalid children), adds their number to the
      count, and calls `n.LoadTxFilter(ctx, false, addrs, nil)`. A cancelled context or any task error
      aborts and is returned (with count 0).
5. Under `addressBuffersMu`, raise each branch's `lastWatched` to its count if it is lower.
6. Return the total.

On a non-first call the range starts at the previously watched index, so at least that one address is
re-sent each time; `LoadTxFilter` calls for a branch may be issued concurrently and in any order.

### 4.28 Fee-rate sources (`wallet/wallet.go:770-783,5520`, `config.go:115,373`, `internal/cfgutil/amount.go`)

Every fee in this scope is `FeeForSerializeSize(rate, size)`; this section fixes where `rate` comes from.

**The wallet-wide rate (`Wallet.relayFee`).**

1. Initial value: `Config.RelayFee` (`wallet/wallet.go:5520`), which the daemon fills from `--txfee`
   (`config.go:115`; default `txrules.DefaultRelayFeePerKb` = 10000 atoms/kB, `config.go:373`).
2. `--txfee` parsing (`cfgutil.AmountFlag.UnmarshalFlag`, `internal/cfgutil/amount.go:32-44`): strip one
   trailing `" DCR"` suffix if present; `strconv.ParseFloat(value, 64)` (its error is returned as the
   option-parse error); `dcrutil.NewAmount(f)`: error `invalid coin amount` for NaN or ±Inf, otherwise
   `f * 1e8` rounded half away from zero (`int64(f*1e8 + 0.5)` for `f >= 0`, `int64(f*1e8 − 0.5)` for
   `f < 0`; `dcrutil/v4@v4.0.3/amount.go:61-89`). The value is a DCR-per-kB decimal, e.g. `0.0001`.
   `MarshalFlag` renders `Amount.String()` (e.g. `0.0001 DCR`), which is why the suffix is accepted.
   Nothing in `config.go` range-checks the result: zero and negative rates are accepted.
3. Runtime change: `SetRelayFee(amount)` replaces the value under `relayFeeMu`. It is not written to the
   database or the config file; a restart returns to `--txfee`.
4. Read: `RelayFee()` returns a snapshot under `relayFeeMu`. Functions that need the rate more than once
   read it once and reuse it (`SendOutputs`, `SendOutputsToTreasury`, `mixedSplit`, `compressWalletInternal`,
   `purchaseTickets`), so a concurrent `settxfee` never changes the rate mid-transaction.

**RPC surface of the wallet-wide rate.**

| RPC | Behaviour | Source |
|---|---|---|
| JSON-RPC `settxfee amount` | `amount < 0` → error code −8 (`ErrRPCInvalidParameter`), message `negative amount`; `dcrutil.NewAmount(amount)` failure → code −8 with the `NewAmount` message; otherwise `SetRelayFee` and result `true`. Zero is accepted. | `internal/rpc/jsonrpc/methods.go:4753-4772` |
| JSON-RPC `getwalletfee` | `RelayFee().ToCoin()` (float64 DCR/kB). | `internal/rpc/jsonrpc/methods.go:2847-2855` |
| JSON-RPC `getinfo` | `paytxfee = RelayFee().ToCoin()`; `relayfee` is 0 unless filled from the consensus RPC server's `getinfo`. | `internal/rpc/jsonrpc/methods.go:1805-1822` |
| JSON-RPC `walletinfo` | `txfee = RelayFee().ToCoin()`. | `internal/rpc/jsonrpc/methods.go:5511-5522` |

**Which rate each builder uses.**

| Builder | Rate |
|---|---|
| `SendOutputs`, `SendOutputsToTreasury` (dust check and `authorTx.txFee`) | `RelayFee()` read once (`wallet/wallet.go:4574,4611`) |
| `txToMultisigInternal`, `compressWalletInternal`, `PrepareRedeemMultiSigOutTxOutput` | `RelayFee()` (`wallet/createtx.go:665,879`, `wallet/multisig.go:90`) |
| `individualSplit`, `mixedSplit`, `purchaseTickets` ticket fee, `PurchaseTickets` fee split, `CreateVspPayment` | `RelayFee()` (`wallet/createtx.go:1007,1129,1225`, `wallet/wallet.go:1648,4721`) |
| `Wallet.NewUnsignedTransaction` | The caller's `relayFeePerKb` argument, unvalidated |
| JSON-RPC `fundrawtransaction` | `RelayFee()`, replaced by `dcrutil.NewAmount(options.feerate)` (DCR/kB) when the option is present; the `NewAmount` error is returned as is; negative values are not rejected (`internal/rpc/jsonrpc/methods.go:887-903`) |
| JSON-RPC `sweepaccount` | `RelayFee()`, replaced by `dcrutil.NewAmount(feeperkb)` when present (error → code −8) (`internal/rpc/jsonrpc/methods.go:5249-5256`) |
| JSON-RPC `spendoutputs`, treasury spend construction | `RelayFee()` (`internal/rpc/jsonrpc/methods.go:4384,3743`) |
| gRPC `ConstructTransaction` | `txrules.DefaultRelayFeePerKb` (**not** the wallet's rate) unless `fee_per_kb != 0`, in which case `dcrutil.Amount(req.FeePerKb)`: an integer count of atoms per kB, sign not checked (`internal/rpc/rpcserver/server.go:1226-1229`) |
| gRPC `SweepAccount` | `RelayFee()`; `fee_per_kb < 0` → `InvalidArgument`, message `fee per kb argument cannot be negative`; `fee_per_kb > 0` → `dcrutil.NewAmount(fee_per_kb)` (a float64 in DCR/kB; failure → `InvalidArgument` with the error text) (`internal/rpc/rpcserver/server.go:969-984`) |

The high-fee threshold never uses the wallet-wide rate; it is always `1000 × DefaultRelayFeePerKb`.

### 4.29 Address allocation call sites: branch, persistence mode and gap policy

The policy is fixed per call site except where a client selects it. Change-source sites are in 4.25; the
remaining sites are:

| Site | Entry | Account / branch | Mode (4.20) | Gap policy |
|---|---|---|---|---|
| JSON-RPC `getnewaddress [account] [gappolicy]` (`internal/rpc/jsonrpc/methods.go:2307-2345`) | `NewExternalAddress` | named account (default `"default"`), external | self-contained | absent or `""` → no option (`gapPolicyError`); `"error"`, `"ignore"`, `"wrap"` → the matching option; any other string → error code −8, message `unknown gap policy "<value>"` (Go `%q`), raised before the account lookup |
| gRPC `NextAddress` (`internal/rpc/rpcserver/server.go:525-560`) | `NewExternalAddress` / `NewInternalAddress` by `kind` | request account | self-contained | `GAP_POLICY_UNSPECIFIED` → no option (`gapPolicyError`); `GAP_POLICY_ERROR`, `GAP_POLICY_IGNORE`, `GAP_POLICY_WRAP` → the matching option; any other enum value → `InvalidArgument`, message `gap_policy=<value>`; an unknown `kind` → `InvalidArgument`, message `kind=<value>` (checked after the gap policy) |
| JSON-RPC `getrawchangeaddress` (`internal/rpc/jsonrpc/methods.go:2353-2376`) | `NewChangeAddress` | named account (imported → account 0), internal | self-contained | `gapPolicyWrap` |
| gRPC `FundTransaction` change script (`internal/rpc/rpcserver/server.go:1127-1132`) | `NewChangeAddress` | request account, internal | self-contained | `gapPolicyWrap` |
| JSON-RPC `spendoutputs` change (`internal/rpc/jsonrpc/methods.go:4301-4311`) | `NewChangeAddress`, called at most once and cached | request account, internal | self-contained | `gapPolicyWrap` |
| JSON-RPC `redeemmultisigout` without an address (`internal/rpc/jsonrpc/methods.go:4088-4094`) | `NewInternalAddress` | account 0, internal | self-contained | `gapPolicyWrap` |
| `individualSplit` split address (`wallet/createtx.go:1104`) | `NewInternalAddress` | `req.SourceAccount`, internal | self-contained | `gapPolicyWrap` |
| `PurchaseTickets` VSP fee-split output (`wallet/wallet.go:1659`) | `NewInternalAddress` with no option | `req.SourceAccount`, internal | self-contained | `gapPolicyError` |
| `CreateVspPayment` change (`wallet/wallet.go:4702`) | `NewChangeAddress` | change account, internal | self-contained | `gapPolicyWrap` |
| `purchaseTickets` vote and reward-commitment addresses (`wallet/createtx.go:1187-1203`) | `nextAddress` directly, op of the caller | voting account branch 1; source account branch 1, or the mixed account and its branch when mixing | self-contained | `gapPolicyIgnore` |
| Mixed outputs generator `makeGen` (`wallet/mixing.go:53-62`) | `nextAddress` directly, op `"gen"` | mixed account and branch | deferred | `gapPolicyIgnore` |
| Mix change (`wallet/mixing.go:398-402`) | `nextAddress` directly | change account, internal | deferred | `gapPolicyIgnore` |

The three direct `nextAddress` callers skip `notVotingAcct` (it is applied only by `NewExternalAddress`,
`NewInternalAddress` and `newChangeAddress`), so they do not reject imported voting accounts. They pass
an empty account name.

`purchaseTickets` additionally requires the returned address to implement `stdaddr.StakeAddress`
(otherwise `errors.E(errors.Invalid, "account does not return compatible stake addresses")`), reads the
child index from the `xpubAddress`, and for the vote address calls
`signingAddressAtIdx(ctx, op, req.VotingAccount, idx)`, which re-derives the **external** child at the same
index and writes its address rows (`wallet/createtx.go:1518-1525`).

### 4.30 Exported `SecretsSource` lifecycle (`wallet/createtx.go:184-238`)

1. `Wallet.SecretsSource()` calls `w.db.BeginReadTx()` and returns its error **unwrapped** (no op). The
   read transaction stays open until `Close`; it pins a database snapshot for that long.
2. `GetKey(addr)`: `Manager.PrivateKey(ns, addr)`; on error returns `(nil, 0, false, err)` with the error
   unwrapped; on success appends the key's `done` callback to `doneFuncs` and returns
   `(privKey.Serialize(), STEcdsaSecp256k1, true, nil)`. A key requested twice is fetched twice and both
   callbacks are queued.
3. `GetScript(addr)`: `Manager.RedeemScript(ns, addr)`, error unwrapped.
4. `ChainParams()`: the wallet's parameters.
5. `Close()`: run every queued callback in order, set `doneFuncs = nil`, `dbtx.Rollback()`; set
   `dbtx = nil` only when the rollback succeeded; return the rollback error. Calling `GetKey`/`GetScript`
   after a successful `Close` dereferences a nil transaction and panics.

The unexported `secretSource` used by `authorTx` differs only in borrowing the caller's read bucket and
in having its callbacks run by `authorTx` itself (4.8 step 2.8).

### 4.31 `insertCreditsIntoTxMgr` error handling (`wallet/createtx.go:269-303`)

For each output index `i` in order and each address extracted from the output script
(`stdscript.ExtractAddrs(output.Version, output.PkScript, params)`; a multisig script yields several):

1. `ma, err = Manager.Address(ns, addr)`.
2. On success: `txStore.AddCredit(dbtx, rec, nil, uint32(i), ma.Internal(), ma.Account())` (error as
   `errors.E(op, err)`), then `markUsedAddress(op, dbtx, ma)` (its error returned as is; it already carries
   `op`). An output with several wallet-owned addresses calls `AddCredit` once per owned address for the
   same index.
3. On an error of kind `NotExist`: skip the address.
4. On any other error: return `errors.E(op, err)`.

The function comment states that it assumes a regular transaction and "will cause balance issues" when
called for anything else. Its only caller is consolidation.

## 5. Constants, limits and defaults

| Name | Value | Source | What depends on it |
|---|---|---|---|
| `txsizes.RedeemP2PKSigScriptSize` | 74 | `wallet/txsizes/size.go:18` | Fee estimate for P2PK inputs |
| `txsizes.RedeemP2PKHSigScriptSize` | 108 | `wallet/txsizes/size.go:28` | Fee estimate for P2PKH inputs; initial estimate in the authoring loop; consolidation |
| `txsizes.RedeemP2SHSigScriptSize` | 110 | `wallet/txsizes/size.go:40` | `txToMultisigInternal` and `PrepareRedeemMultiSigOutTxOutput` fee estimates |
| `txsizes.RedeemP2PKHInputSize` | 166 | `wallet/txsizes/size.go:55` | Used (as a sigScript size) by `PurchaseTickets`' fee-split estimate and by JSON-RPC `spendoutputs` |
| `txsizes.P2PKHPkScriptSize` | 25 | `wallet/txsizes/size.go:66` | Change script size; `xpubAddress.ScriptLen` |
| `txsizes.P2PKHPkTreasuryScriptSize` | 26 | `wallet/txsizes/size.go:80` | Treasury change script size |
| `txsizes.P2SHPkScriptSize` | 23 | `wallet/txsizes/size.go:89` | Exported only |
| `txsizes.TicketCommitmentScriptSize` | 32 | `wallet/txsizes/size.go:99` | Ticket size estimate (ticket-purchase spec) |
| `txsizes.P2PKHOutputSize` | 36 | `wallet/txsizes/size.go:108` | Tests; JSON-RPC `spendoutputs` change-size estimate |
| `txsizes.TSPENDInputSize` | 109 | `wallet/txsizes/size.go:117` | JSON-RPC treasury-spend construction (`sendOutputsFromTreasury`) fee estimate, passed as the single input's sigScript size (`internal/rpc/jsonrpc/methods.go:3741`) |
| Fixed transaction overhead | 12 bytes | `wallet/txsizes/size.go:147-148` | Every size estimate |
| Fixed input overhead | 57 bytes + varint | `wallet/txsizes/size.go:198-200` | Every size estimate |
| Fixed output overhead | 10 bytes + varint | `wallet/txsizes/size.go:207-209` | Every size estimate |
| `txrules.DefaultRelayFeePerKb` | 10000 atoms/kB | `wallet/txrules/rules.go:38` | Default `--txfee`; gRPC `ConstructTransaction` default rate; high-fee threshold |
| Dust input-size constant | 165 bytes | `wallet/txrules/rules.go:50-51` | Dust threshold |
| Dust divisor | 3 | `wallet/txrules/rules.go:55` | Dust threshold |
| High-fee multiplier | 1000 × `DefaultRelayFeePerKb` = 10,000,000 atoms/kB | `wallet/txrules/rules.go:125` | `PaysHighFees`, `checkHighFees` |
| `dcrutil.MaxAmount` | 2,100,000,000,000,000 atoms | `dcrutil/v4@v4.0.3/const.go:16` | Fee clamp; `CheckOutput`; "select all" target |
| Pool fee scale | percent × 100, valid `[1, 10000]` | `wallet/txrules/poolfees.go:19-23,43-44` | VSP fee |
| Subsidy forward adjustment | `ceil(TicketPoolSize / SubsidyReductionInterval)` steps of `× 100 / 101` | `wallet/txrules/poolfees.go:54-71` | VSP fee |
| Fixed-point shift | 64 bits | `wallet/txrules/poolfees.go:74` | VSP fee rounding |
| `txauthor.generatedTxVersion` | 1 | `wallet/txauthor/author.go:28` | Version of authored transactions |
| `wire.TxVersionTreasury` | 3 | `wire@v1.8.0/msgtx.go:28` | Treasury-add transactions |
| `txscript.MaxScriptElementSize` | 2048 | `txscript/v4@v4.1.2/script.go:20` | Change script length limit in the authoring loop |
| `stdscript.MaxDataCarrierSizeV0` | 256 | `txscript/v4@v4.1.2/stdscript/scriptv0.go:18` | Vote-bits script data limit |
| `revocationFeeLimit` | 16384 (`1 << 14`) | `wallet/createtx.go:40` | Ticket commitment limits word `0x4e00` |
| `sanityVerifyFlags` | `0x4f` (bits 0, 1, 2, 3, 6) | `wallet/createtx.go:45-49`, `txscript/v4@v4.1.2/engine.go:20-60` | Self-verification of signed transactions |
| `scriptVersionAssumed` | 0 | `wallet/wallet.go:83` | Script classification and engine version everywhere |
| Multisig fee allowance | 50,000,000 atoms (mainnet, magic `0x48e7a065`, testnet3); 30,000 otherwise | `wallet/createtx.go:584-594` | `sendtomultisig` input target |
| Consolidation max size | 100,000 bytes on mainnet; `params.MaxTxSize` elsewhere | `wallet/createtx.go:841-846` | Number of inputs consolidated |
| `params.MaxTxSize` | mainnet 393216; testnet3, simnet, regnet 1000000 | `chaincfg/v3@v3.3.0/*params.go` | Authoring-loop size limit |
| `params.CoinbaseMaturity` | mainnet 256; testnet3, simnet, regnet 16 | `chaincfg/v3@v3.3.0/*params.go` | Spendability of coinbase, vote, revocation, treasury outputs |
| `params.SStxChangeMaturity` | 1 on all networks | `chaincfg/v3@v3.3.0/*params.go` | Spendability of ticket change |
| Random selection budget | `numUnspent / 2` tries | `wallet/udb/txmined.go:3104`, `wallet/createtx.go:1913` | Random phase length |
| Consolidation minconf | 1 | `wallet/createtx.go:798` | `consolidate` |
| `DefaultGapLimit` | 20 | `wallet/addresses.go:385` | Default `--gaplimit` |
| `DefaultAccountGapLimit` | 10 | `wallet/addresses.go:389` | Default `--accountgaplimit` |
| `hdkeychain.HardenedKeyStart` | `0x80000000` | `hdkeychain/v3@v3.1.3` | Branch exhaustion test; child index limit |
| `udb.ImportedAddrAccount` | `0x7fffffff` | `wallet/udb/addressmanager.go:47` | Imported-account redirect; account kind |
| `udb.MaxAddressesPerAccount` | `0x7fffffff` | `wallet/udb/addressmanager.go:41` | `SyncAccountToAddrIndex` limit |
| Filter load chunk | 256 children | `wallet/wallet.go:1013` | `watchHDAddrs` batching |
| Filter load concurrency | `runtime.NumCPU()` | `wallet/wallet.go:1012` | `watchHDAddrs` |
| `Config.WatchLast` (`--watchlast`) | default 0 (unlimited) | `config.go:112`, `wallet/wallet.go:1004-1006` | Lower bound of first-watch range |
| Default gap policy | `gapPolicyError` | `wallet/addresses.go:396,546` | Any `nextAddress` call without an option |
| `--txfee` default | 0.0001 DCR/kB (10000 atoms/kB) | `config.go:373` | Initial `Wallet.relayFee` (4.28) |
| `--allowhighfees` default | false | `config.go:58,114,372` | `checkHighFees`, gRPC `PublishTransaction` guard |
| `--gaplimit` default | 20 (`wallet.DefaultGapLimit`) | `config.go:57,111,371` | `Wallet.gapLimit` |
| `--accountgaplimit` default | 10 (`wallet.DefaultAccountGapLimit`) | `config.go:59,116,374` | Account discovery / `NextAccount` |
| `dcrutil.AtomsPerCoin` rounding | `int64(f*1e8 ± 0.5)` | `dcrutil/v4@v4.0.3/amount.go:61-89` | Every DCR-denominated fee rate parsed from a flag or RPC argument |
| JSON-RPC gap policy strings | `""`, `"error"`, `"ignore"`, `"wrap"` | `internal/rpc/jsonrpc/methods.go:2316-2326` | `getnewaddress` |
| Ticket size estimate script sizes | inputs `[108]`, outputs `[26, 32, 26]` | `wallet/createtx.go:1216,1234-1239` | Ticket fee (ticket-purchase spec) |
| Mixing eligibility minconf | 2 | `wallet/mixing.go:464` | `findEligibleOutputsAmount` call from the mix-account path, with `amount = 0`, `minAmount` = the smallest split point, `maxResults` = split semaphore capacity × number of split points |
| VSP fee payment input minconf | 1 | `wallet/wallet.go:4673` | `ReserveOutputsForAmount` in `CreateVspPayment` |

## 6. Concurrency and lifecycle

**Mutexes.**

| Mutex | Protects | Held across |
|---|---|---|
| `Wallet.lockedOutpointMu` | `lockedOutpoints` set | The whole of `NewUnsignedTransaction`, `authorTx`, `recordAuthoredTx`, `txToMultisig`, `compressWallet`, `ReserveOutputsForAmount`, `reserveOutputs` (including their database transactions, signing, and, for multisig/consolidation, network publication); the selection phase only of `mixedSplit` |
| `Wallet.addressBuffersMu` | `addressBuffers` map and every `addressBuffer` | `nextAddress` body (including an optional `LoadTxFilter` network call under `gapPolicyIgnore`), `AddressAtIdx`, `CurrentAddress`, `BIP0044BranchNextIndexes`, the buffer phases of `watchHDAddrs` and `SyncLastReturnedAddress`, buffer creation sites |
| `Wallet.relayFeeMu` | `relayFee` | `RelayFee`, `SetRelayFee` |
| `udb.Manager.mtx` | Address manager state | Inside `SyncAccountToAddrIndex`, `PrivateKey`, `AccountProperties`, … |

**Lock and transaction ordering observed at the pin.**

1. `lockedOutpointMu` → read transaction → (`notVotingAcct` opens a second, nested read transaction) →
   `addressBuffersMu` (authoring with a wallet change source).
2. `lockedOutpointMu` → write transaction → `addressBuffersMu` (`txToMultisig`, `compressWallet`; the
   nested `notVotingAcct` read transaction is opened while the write transaction is open).
3. `lockedOutpointMu` → write transaction (`recordAuthoredTx`; `NewUnsignedTransaction`'s trailing
   `UpdateDB`).
4. Write transaction → `addressBuffersMu` (`nextAddress` in self-contained mode; the mutex is released
   before the address rows are written and the transaction committed).
5. `addressBuffersMu` → write transaction (`DiscoverActiveAddresses`, `wallet/discovery.go:893-938`). This
   is the reverse of (4); see Quirks.
6. No code in this scope takes `lockedOutpointMu` while holding `addressBuffersMu`.
7. Self-contained `nextAddress` (every `getnewaddress`, `getrawchangeaddress`, gRPC `NextAddress`, and the
   ticket address allocations) begins its write transaction **before** validating the account or branch
   and before taking `addressBuffersMu` (`wallet/addresses.go:521-552`). The database allows one write
   transaction at a time, so these calls queue behind any other writer (block processing, transaction
   recording) even when they end in an error, and each successful call is one committed write
   transaction.
8. `mixedSplit` runs its two deferred functions in reverse registration order: first the selected
   outpoints are removed from the lock set (re-taking `lockedOutpointMu`), then, on success only, the
   change-address rows are written in a new write transaction (`wallet/createtx.go:1009-1025`).
9. `ReserveOutputsForAmount`, `reserveOutputs` and `NewUnsignedTransaction` take `lockedOutpointMu` and then
   a read transaction; the lock-set mutation of the reserve functions happens inside the read
   transaction closure and is not undone if the closure later fails (it cannot: locking is the last step).

**Runtime trace regions.** `accountBranchChildUpdates.UpdateDB`, when it opens its own transaction, starts
Go execution-trace regions named `db.Update` (whole call) and `db.ReadWriteTx` (from begin to
commit/rollback) (`wallet/addresses.go:475,480-488`). They are visible only in a Go runtime trace and
need no equivalent.

**Goroutines.** Only `watchHDAddrs` spawns goroutines in this scope: an `errgroup` per branch limited to
`runtime.NumCPU()` concurrent tasks, each deriving up to 256 addresses and issuing one `LoadTxFilter`.
The group's derived context cancels remaining tasks on the first error. `NextAccount` (outside scope)
spawns two goroutines to derive and load the first `gapLimit` addresses of each branch.

**Cancellation.**

- `nextAddress` checks `ctx.Err()` at the top of each loop iteration and returns the raw context error.
- `watchHDAddrs` checks the errgroup context before scheduling each chunk.
- The default change source of `Wallet.NewUnsignedTransaction` uses `context.Background()`; cancelling the
  request does not interrupt change-address allocation there.
- `publishAndWatch` passes the request context to the backend; a cancelled publish is treated as a failed
  publish and the transaction is abandoned.

**Secrets lifetime.** Private keys obtained from `Manager.PrivateKey` come with a `done` callback that
zeroes them. `authorTx` runs the callbacks immediately after signing; `signP2PKHMsgTx` and
`signVoteOrRevocation` run them at function return; the exported `SecretsSource` runs them in `Close`.
Key bytes passed to `txscript/sign` are a fresh serialization (`privKey.Serialize()`) that is not zeroed.

**Ordering guarantees callers observe.**

- A transaction authored through `authorTx` is committed to the database (with its change-address rows)
  before it is published.
- Addresses are loaded into the backend filter *after* publication (`publishAndWatch`), not before.
- In-memory cursors advance at allocation time, before any database write, and are never rolled back.
- The lock set is process-local and empty at start; `authorTx` leaves nothing locked on return, while
  `ReserveOutputsForAmount` and `mixedSplit` leave their selections locked until explicitly released (or,
  for `mixedSplit`, until it returns).

## 7. Errors

Error values are `*errors.Error{Op, Kind, Err}` from the wallet's `errors` package unless noted. Message
formatting (`errors/errors.go:247-290`): within one `*Error` the op, the kind description and the
underlying non-`*Error` message are joined with `": "`; a directly nested `*Error` is joined to its
parent with `errors.Separator`, which the daemon sets to `":: "` (`dcrwallet.go:38`); an op or kind equal
to the one printed last is not repeated; a kind prints as its description (`errors/errors.go:77-124`).
`errors.E(op, err)` on an `*Error` promotes the nested kind (and the nested op when no op is given) to
the new outer error and drops the nested error from the chain when it then carries no distinct op or
kind (`errors/errors.go:187-236`).

**Kinds raised in this scope.**

| Kind (description string) | Raised by | Message text |
|---|---|---|
| `InsufficientBalance` ("insufficient balance") | Authoring loop when the source is short; `findEligibleOutputsAmount` (bare kind); multisig step 13; consolidation when the output would be dust | — |
| `Invalid` ("invalid operation") | Authoring loop | `signed tx size exceeds allowed maximum`; `script size exceed maximum bytes pushable to the stack` |
| `Invalid` | `Wallet.NewUnsignedTransaction` | `unknown output selection algorithm <n>` |
| `Invalid` | `CheckOutput` | `transaction output amount is negative`; `transaction output amount exceeds maximum value` |
| `Invalid` | `TxPaysHighFees` | `transaction input <i> does not specify the input value` |
| `Invalid` | `nextAddress`, `AddressAtIdx`, `SyncLastReturnedAddress` | `branch must be external (0) or internal (1)` |
| `Invalid` | `notVotingAcct` | `cannot use voting accounts for normal transactions` |
| `Invalid` | `makeTicket` | `nil vote address` |
| `Invalid` | `RandomUTXO` | `optimized random utxo selection not possible with minConf=0` |
| `Policy` ("policy violation") | `CheckOutput` | `transaction output is dust` |
| `Policy` | `checkHighFees` | `high fee` |
| `Policy` | `nextAddress` | `generating next address violates the unused address gap limit policy` |
| `NotExist` ("item does not exist") | `Wallet.NewUnsignedTransaction` | `missing account` |
| `NotExist` | `nextAddress`, `AddressAtIdx` | `account <n>` |
| `NotExist` | `BIP0044BranchNextIndexes`, `SyncLastReturnedAddress` | `account <n>` |
| `NotExist` | `CurrentAddress` | `no account <n>` |
| `NotExist` | `FetchP2SHMultiSigOutput` (from the store) | `no multisig output for outpoint <hash:index>` |
| `ScriptFailure` ("transaction script fails to execute") | `validateTxInput`; `signVoteOrRevocation` | underlying engine or signer error |
| `Bug` ("internal wallet error") | `nextAddress` | `nextAddress must not provide both a slice to defer db updates and an open write transaction`; `no method to update DB with addresses` (unreachable) |
| `Bug` | `signP2PKHMsgTx` | `previous output address is not P2PKH` |
| `Bug` | `makeTicket` | wraps the `stake.CheckSStx` error with op `stake.CheckSStx` |
| `Bug` | `wrapManagedAddress` | `don't know how to wrap <Go type>` |
| `NoPeers` ("Decred network is unreachable") | `publishAndWatch`, multisig, consolidation (no backend) | op `wallet.NetworkBackend` |
| none (`Other`) | `authorTx` | `violated assumption: expected unsigned tx to be version 1` |
| none | `txToMultisigInternal` | `not enough funds to send to multisig address` |
| none | `compressWalletInternal` | `too few outputs to consolidate` |
| none | `nextAddress` | `account <a> branch <b> exhausted` |
| none | `AddressAtIdx` | `child out of range` |
| none | `PrepareRedeemMultiSigOutTxOutput` | `estimated fee <amount> is above output value <amount>` |
| none | `ImportedAddresses` | `account does not record imported keys` |
| none (plain Go errors) | `AddAllInputScripts` | `tx.TxIn and prevPkScripts slices must have equal length` |
| none (plain Go errors) | `signP2PKHMsgTx` | `Number of prevOutputs (<a>) does not match number of tx inputs (<b>)` |
| sentinel | `purchaseTickets` | `paying VSP fee requires UTXO split` |
| `Policy` | gRPC `PublishTransaction`, JSON-RPC `sendrawtransaction` (via `TxPaysHighFees`) | `high fees` |
| `Invalid` | `purchaseTickets` address helper | `account does not return compatible stake addresses` |
| `Invalid` (from `SyncAccountToAddrIndex`, reached through `UpdateDB`, `markUsedAddress`, `SyncLastReturnedAddress`) | address manager | `cannot sync imported account branch index`; `account branch <n>`; `child index <n> exceeds max` |
| none (`hdkeychain.ErrDeriveHardFromPublic`) | `deriveChildAddresses`, `deriveChildAddress` when `startIndex + i >= 2^31` | `cannot derive a hardened key from a public key` |
| none (`dcrutil.NewAmount`) | `--txfee`, `settxfee`, `fundrawtransaction` fee rate, `sweepaccount` fee rate, gRPC `SweepAccount` | `invalid coin amount` |

Errors raised by the RPC layers for inputs that select behaviour in this scope (not `*errors.Error`
values): JSON-RPC code −8 `unknown gap policy "<value>"` (`getnewaddress`), code −8 `negative amount`
(`settxfee`); gRPC `InvalidArgument` `gap_policy=<value>` and `kind=<value>` (`NextAddress`),
`InvalidArgument` `fee per kb argument cannot be negative` (`SweepAccount`), `InvalidArgument`
`unknown output selection algorithm` (`ConstructTransaction`, raised by the server before the wallet's
own `Invalid` check can be reached).

Wrapping asymmetries a port must keep if error strings are compared: `Wallet.SecretsSource()`,
`ReserveOutputsForAmount`, `reserveOutputs`, `mixedSplit`, `SyncLastReturnedAddress` (database phase),
`signP2PKHMsgTx` (key lookup) and `UpdateDB` return underlying errors without an op; `ImportedAddresses`
wraps every error, including its own, with the formatted op `wallet.ImportedAddresses("<account>")`
(`errors.Opf`, `wallet/addresses.go:916-922`); the change sources call `newChangeAddress` with the empty
op, so their errors carry no op of their own until the caller (`txauthor.NewUnsignedTransaction`, then
`authorTx`/`Wallet.NewUnsignedTransaction`) adds one.

Errors passed through from dependencies: `Manager.PrivateKey` (`Locked` when the wallet or account is
locked, `NotExist` for an unknown address, `Invalid` for a script address, `Crypto`, and the kind-less
`no private key for <account name>/<branch>/<index>` when the account has no private key material; never
`WatchingOnly`, see 4.15), `sign.SignTxOutput` (`can't sign NULLDATA transactions`,
`can't sign unknown transactions`, key lookup errors), `stdscript.MultiSigScriptV0` kinds, context errors,
backend errors from `PublishTransactions`/`LoadTxFilter`.

**Surfacing.**

- JSON-RPC (`internal/rpc/jsonrpc/errors.go:16-49`): the first kind found in the chain selects the code:
  `Bug` → −32603 (internal error); `Encoding` → −8; `Locked` → −13; `Passphrase` → −14; `NoPeers` → −9;
  `InsufficientBalance` → −6; everything else (including `Invalid`, `Policy`, `NotExist`, `ScriptFailure`
  and kind-less errors) → −4. The message is the full error string.
- gRPC (`internal/rpc/rpcserver/server.go:80-135`): `Invalid` → `InvalidArgument`; `NotExist` → `NotFound`;
  `Locked` → `FailedPrecondition`; `InsufficientBalance` → `ResourceExhausted`; `NoPeers` → `Unavailable`;
  `Policy`, `ScriptFailure`, `Bug` and kind-less errors → `Unknown` (unless the chain contains
  `context.Canceled` → `Canceled`). The status message is the full error string.
- Logs (subsystem `WLLT`): the messages quoted in section 4 at the stated levels.
- None of these errors terminate the process. `xpubAddress.PubKey` and `RandomizeOutputPosition` can panic
  (see Quirks).

## 8. Compatibility-critical behaviours

1. **Size arithmetic.** `EstimateInputSize`, `EstimateOutputSize`, `EstimateSerializeSize` and
   `EstimateSerializeSizeFromScriptSizes` must return the exact integers of 4.1, including the doubled
   input-count varint and the varint width changes at 253 inputs/outputs. Fees, change amounts and gRPC
   `ConstructTransactionResponse.estimated_signed_size` derive from them.
2. **Fee formula.** `FeeForSerializeSize` must reproduce the truncating division, the lift of a zero fee to
   the full per-kB rate, and the clamp to `MaxAmount`. The fee of every wallet-built transaction is
   visible on chain and in RPC results (`fundrawtransaction` returns it; gRPC `ConstructTransaction`
   returns the input and output totals).
3. **Dust rule.** Change is omitted exactly when `changeAmount == 0` or
   `changeAmount * 1000 / (3 * (10 + varint(scriptSize) + scriptSize + 165)) < rate`, evaluated with the
   change source's declared `ScriptSize()`. `CheckOutput` must reject with the same boundary so that
   `sendtoaddress`-family RPCs accept and reject the same amounts.
4. **Authoring-loop outcomes.** For a given input sequence the number of inputs used, the fee, the change
   value, the presence of change, and the error (`InsufficientBalance` vs `Invalid`) must match 4.5,
   including: no extra input is pulled to rescue a dust remainder; the fee is not recomputed when change
   is dropped; the max-size test counts the change output.
5. **Transaction template.** Version 1 (3 for treasury adds), lock time 0, expiry 0, sequence
   `0xffffffff`, `ValueIn` = previous amount, block height 0, block index `0xffffffff`, outpoint tree from
   the credit's stake tag. dcrd rejects a transaction whose `ValueIn` differs from the spent output's
   amount (rule error `ErrFraudAmountIn`, `dcroxide:crates/dcroxide-blockchain/src/ruleerror.rs:108,332`),
   and the witness bytes are part of the full transaction hash.
6. **Change placement.** Change is appended last, then swapped with a uniformly random output only where
   the caller asks (`SendOutputs`, the VSP fee split, gRPC `ConstructTransaction`, JSON-RPC
   `spendoutputs`); it stays last for treasury adds, `individualSplit`, `fundrawtransaction`,
   `sweepaccount`. gRPC returns `change_index`, and treasury-add consensus rules require version 3, one
   or two outputs, the `OP_TADD` output first and an optional stake-change output second
   (`dcroxide:crates/dcroxide-stake/src/treasury.rs:33-107`).
7. **Treasury change script.** `OP_SSTXCHANGE` + P2PKH, 26 bytes, estimated at 26.
8. **Signature scripts.** Exactly the layouts of 3.4 with `SigHashAll`, compressed public keys and
   canonical minimal pushes. Signatures must verify under dcrd's script engine with standardness flags.
9. **Self-verification flags.** Signed transactions are executed with flag set `0x4f` and script version 0
   before being recorded; a transaction that fails must be rejected with kind `ScriptFailure` and never
   published.
10. **High-fee guard.** With `--allowhighfees` unset, any wallet-authored transaction paying more than
    10,000,000 atoms/kB of its serialized size is refused with kind `Policy`, message `high fee`.
11. **Spendability rules.** Confirmation counting (`tip − height + 1`, 0 for unmined), coinbase maturity
    (`> CoinbaseMaturity` confirmations in that counting), ticket-change maturity, exclusion of ticket
    submission outputs, exclusion of locked outpoints, exclusion of outputs already spent by unmined
    wallet transactions (mined outputs in every selector; unmined credits only in the
    `findEligibleOutputs*` paths, see 4.13.2 step 1), exclusion of outputs of unpublished transactions. Spending an immature output
    would be rejected by dcrd.
12. **Which script classes are auto-spendable.** P2PKH secp256k1 (plain or stake-tagged, ticket
    submissions excluded) and plain P2PK secp256k1 through the default input source; P2PKH (plain or stake-tagged) only through the `findEligibleOutputs*` paths.
    Plain P2SH credits are never auto-selected.
13. **Lock set semantics.** `lockunspent`/`listlockunspent`-visible state: `ReserveOutputsForAmount`
    leaves outpoints locked after it returns; `mixedSplit` keeps its selection locked for the duration of
    the mix and removes it when it returns; `authorTx` leaves nothing locked; the set is keyed by hash and
    index and is lost at shutdown.
14. **Address derivation.** Child `i` of branch `b` of account `a` is the P2PKH address of the compressed
    public key at `xpub_a / b / i` using dcrd's `hdkeychain` rules, skipping indexes that fail to derive.
    Addresses must equal what dcrwallet returns for the same seed (`wallet/addresses_test.go:58-117`).
15. **Next-index rule.** The next child is `lastUsed + 1 + cursor`; `accountaddressindex` and gRPC clients
    read it through `BIP0044BranchNextIndexes`. Usage discovered on chain moves `lastUsed` and rebases
    `cursor` by the rule in 4.23 so the next index never moves backward.
16. **Gap policies.** `error` fails with the exact `Policy` message at `cursor >= gapLimit`; `wrap` resets
    the cursor to 0 (re-issuing from `lastUsed + 1`); `ignore` continues and loads `gapLimit` further
    addresses into the backend filter each time the cursor crosses a multiple of the gap limit. The
    policy used by each internal caller is fixed (4.25) and determines when RPCs start failing or reusing
    change addresses.
17. **Address rows written.** Returning child `c` ensures rows for every child `≤ c` of that branch exist;
    marking child `u` used ensures rows up to `min(u' + gapLimit, 2^31 − 1)` exist, where `u'` is the
    branch's last-used index after the update. An existing `wallet.db` written by dcrwallet relies on
    these rows to recognise incoming payments; a replacement must write them in the same circumstances
    so that the file stays usable by dcrwallet as well.
18. **Imported-account change redirect.** Change from spending the imported account goes to account 0's
    internal branch.
19. **Filter watch-ahead.** After the first full load, the backend filter must contain every address of
    both branches from 0 (or `count − gapLimit − watchLast` when `--watchlast` applies) through
    `lastReturned + gapLimit`, and be topped up after each processed block/transaction and each publish.
    The filter is the only way the wallet tells its backend which addresses are relevant (network-backend
    specs); an address missing from it is a payment the wallet does not see until a rescan.
20. **Publish failure handling.** A transaction that fails to publish is removed from the wallet
    (`AbandonTransaction`) and a removed-transaction notification is emitted; the RPC returns the publish
    error.
21. **Multisig RPC behaviour.** `sendtomultisig` imports the redeem script, pays the exact amount to the
    P2SH script at output 0, puts change (if any) at output 1, and registers the multisig output in the
    store so `getmultisigoutinfo`/`redeemmultisigout` find it; `redeemmultisigout` computes its single
    output as `amount − FeeForSerializeSize(rate, EstimateSerializeSize([110 × inputs], [output], 0))`.
22. **Consolidation RPC behaviour.** `consolidate` uses confirmed outputs only, at most the requested
    number of inputs, caps the estimated size at 100,000 bytes on mainnet, and pays a single output of
    `total − fee` with the fee computed for worst-case P2PKH inputs.
23. **VSP fee amount.** `StakePoolTicketFee` must match bit for bit, including the float64 multiply-and-floor
    of the percentage and the 64-bit fixed-point truncation. The wallet uses it to size the outputs it
    reserves for VSP fee payments (`wallet/createtx.go:1297-1299,1324-1325`) and the value of the fee-split
    output (`wallet/wallet.go:1649-1668`), both of which are visible in the transactions it publishes; it
    is also an exported library function.
24. **Vote layout.** Stakebase input script from chain parameters, ticket input at index 1, block reference
    and vote-bits outputs, one `OP_SSGEN`-tagged payment per commitment with `stake.CalculateRewards`
    amounts. Votes are consensus-checked by dcrd.
25. **Ticket layout.** `makeTicket` output scripts and the limits word `0x4e00`; tickets are
    consensus-checked (`stake.CheckSStx`) and the VSP inspects the commitment.
26. **Error kinds and strings.** The strings in section 7 reach RPC clients verbatim and the kinds select
    JSON-RPC codes and gRPC status codes.
27. **Log lines.** `Returning address (account=… branch=… child=…)` at info level for every allocated or
    index-derived address, and the other quoted messages, for operators and tooling that scrape logs.
28. **Fee-rate inputs and units (4.28).** `--txfee`, `settxfee`, `fundrawtransaction` `feerate`,
    `sweepaccount` `feeperkb` and gRPC `SweepAccount.fee_per_kb` are DCR-per-kB floats converted with
    `dcrutil.NewAmount` (round half away from zero); gRPC `ConstructTransaction.fee_per_kb` is an integer
    of atoms per kB and defaults to 10000 atoms/kB regardless of `--txfee`/`settxfee`. `settxfee` is
    process-local and is reported back by `getwalletfee`, `getinfo.paytxfee` and `walletinfo.txfee`.
29. **Client-selected gap policy (4.29).** JSON-RPC `getnewaddress` accepts exactly `""`, `error`, `ignore`,
    `wrap`; gRPC `NextAddress` accepts the four enum values; "unspecified" means the error policy. The
    fixed policies of the other entry points (`getrawchangeaddress` wraps, ticket and mixing addresses
    ignore, the VSP fee-split output errors) decide whether those RPCs fail, reuse addresses, or run past
    the gap limit, and so are visible to clients and on chain.
30. **High-fee guard on raw publication.** gRPC `PublishTransaction` and JSON-RPC `sendrawtransaction`
    reject with kind `Policy`, message `high fees`, computed from the inputs' `ValueIn` fields; an input
    with a negative `ValueIn` is kind `Invalid`. JSON-RPC consults only the request's `allowhighfees`
    argument; gRPC consults only the wallet configuration.
31. **Ticket vote-address side effect.** Each purchased ticket advances the voting account's internal
    branch cursor by one and writes address rows for the *external* child at the same index (4.29); the
    VSP client later signs with that external key, so the rows must exist in a `wallet.db` shared with
    dcrwallet.

## 9. Quirks

Each item is observed behaviour at the pin that a faithful port must reproduce or deliberately change.

1. **Last-returned index is not persisted by address allocation.** `nextAddress` and `UpdateDB` write only
   address rows; they never call `MarkReturnedChildIndex` (`wallet/addresses.go:468-502,623-636`). Since
   the buffer cursor is initialised at open as `LastReturned − LastUsed` (`wallet/wallet.go:5471-5481`),
   addresses handed out but not yet used on chain are handed out again after a restart, and
   `AccountProperties.LastReturned*` (gRPC `Accounts` key counts, the `watchHDAddrs` upper bound) does
   not advance on `getnewaddress`. The persistence call existed before commit `bea91d63` ("Fix acquire
   ordering on DB, addresses mutexes") and was dropped in that rewrite; commit `5a416aee` ("Avoid address
   reuse after restarting") had introduced it. Only `MarkUsedChildIndex` and `SyncLastReturnedAddress`
   now write `extret`/`intret`.
2. **`watchHDAddrs` upper bound follows the stale database value.** Because of (1), the watched range is
   `DB lastReturned + gapLimit`, not `in-memory next index + gapLimit` (`wallet/wallet.go:931-932`). With
   `gapPolicyWrap`/`Error` the cursor cannot exceed `gapLimit`, so every issued address is still inside
   the watched range; with `gapPolicyIgnore` the extra batches are loaded by `nextAddress` itself.
3. **`SyncLastReturnedAddress` does not advance the in-memory cursor for a branch with no used address**
   (`lastUsed == 0xffffffff`), although it updates the database (`wallet/addresses.go:872-874`). The
   documented "next returned address will be child+1" only holds after a restart, or when the branch
   already has a used address.
4. **`CurrentAddress` returns the next address to be issued, not the most recently issued one**, contrary
   to its comment (`wallet/wallet.go:1948-1973`). It does not skip invalid children.
5. **Discovery-created account buffers start with `lastUsed = 0`** (Go zero value) instead of
   `0xffffffff` (`wallet/discovery.go:811-815`). A branch of such an account with no on-chain usage
   issues child 1 first until the next restart.
6. **Lock-order inversion.** `nextAddress` (self-contained mode) opens a write transaction and then takes
   `addressBuffersMu` (`wallet/addresses.go:521-552`); `DiscoverActiveAddresses` takes `addressBuffersMu`
   and then opens a write transaction, with a comment asserting that this is the order used elsewhere
   (`wallet/discovery.go:893-896`). Concurrent address allocation during discovery can deadlock.
7. **Default gap policy leaks into change allocation.** The default change source of
   `Wallet.NewUnsignedTransaction`, the treasury change source and the multisig change source do not set
   a gap policy and therefore use `gapPolicyError` (`wallet/createtx.go:134-139,413-418,671-676`).
   `ConstructTransaction`, `fundrawtransaction` (without a change address), `sendtotreasury` and
   `sendtomultisig` fail with the gap-limit `Policy` error once `gapLimit` unused internal addresses have
   been issued, while `SendOutputs` wraps and keeps working.
8. **`Wallet.NewUnsignedTransaction` cannot be used with imported-xpub or imported-voting accounts**:
   their numbers exceed `LastAccount`, so the existence check returns `NotExist: missing account`
   (`wallet/createtx.go:101-109`). `authorTx` has no such check.
9. **`context.Background()` in the default change source** (`wallet/createtx.go:138`).
10. **Unreachable `InsufficientBalance` swallow.** The "select all" wrapper converts an
    `InsufficientBalance` error from the source to success, but the store source never returns one
    (`wallet/createtx.go:120-126`).
11. **`authorTx`'s transient locks are unobservable.** Inputs are added to the lock set and removed again
    while `lockedOutpointMu` is continuously held (`wallet/createtx.go:389-400,435-439`). Between `authorTx`
    returning and `recordAuthoredTx` recording the transaction nothing reserves the inputs; two concurrent
    sends can select the same output, and the second fails when recorded or published.
12. **High-fee check precedes script validation and uses the current size**; with `dontSignTx` the unsigned
    (smaller) size lowers the threshold (`wallet/createtx.go:486-497`).
13. **Imported-account warning keys on the source account** even when a different change account was
    requested (`wallet/createtx.go:480-484`).
14. **Dust remainder inflates the fee.** When change is dropped the fee keeps the amount computed for a
    transaction *with* change plus the dust (`wallet/txauthor/author.go:134-157`).
15. **Zero-output transactions.** A sweep whose remainder is dust returns a transaction with no outputs
    and no error (`wallet/txauthor/author.go:126-157`).
16. **Change-script length check uses the stack element limit (2048)**, not the script size limit, and the
    real script length is never compared with `ScriptSize()` (`wallet/txauthor/author.go:142-145`).
17. **The size-limit error carries no op** while sibling errors do (`wallet/txauthor/author.go:122-124`).
18. **Slice aliasing.** `AuthoredTx.Tx.TxOut` is the caller's `outputs` slice when no change is added, and
    the individual `*TxOut`/`*TxIn` values are shared with the caller and the input source. Not externally
    observable; a port may copy.
19. **`FeeForSerializeSize` overflow and floor behaviour**: int64 wrap on multiply, zero fee lifted to the
    whole per-kB rate, negative results clamped to `MaxAmount` (`wallet/txrules/rules.go:94-106`).
20. **`PaysHighFees` cannot flag when the fee is zero or negative** ("impossible to determine"), so a
    caller passing a wrong `totalInput` disables the guard (`wallet/txrules/rules.go:118-127`).
21. **`StakePoolTicketFee` float input.** `floor(poolFee * 100.0)` in float64 loses a unit for
    percentages such as 0.29 or 1.15 (`0.29 * 100 = 28.999999999999996`); the port must use IEEE-754
    double multiplication and floor, not decimal arithmetic (`wallet/txrules/poolfees.go:43-44`).
22. **`StakePoolTicketFee` global cache.** One subsidy cache per process, bound to the parameters of the
    first call (`wallet/txrules/poolfees.go:25-26,56-58`). The parameter named `relayFee` receives the
    ticket's absolute fee from one caller (`wallet/createtx.go:1297`) and the relay fee rate per kB from
    the other (`wallet/wallet.go:1648-1649`). `doc.go` says the subsidy is "adjusted two difficulty periods into the future"; the code
    applies `ceil(TicketPoolSize / SubsidyReductionInterval)` subsidy reductions of `100/101`.
23. **Misnamed size constants.** `TSPENDInputSize` and `InputDetail.RedeemScriptSizes` are signature-script
    sizes. Callers pass full-input or full-output sizes where script sizes are expected:
    `PurchaseTickets`' fee-split estimate passes `RedeemP2PKHInputSize` (166) as a sigScript size
    (`wallet/wallet.go:1665-1666`); JSON-RPC `spendoutputs` passes 166 per input and reports a change
    `ScriptSize()` of `P2PKHOutputSize` (36) (`internal/rpc/jsonrpc/methods.go:4270,4316`). The
    resulting fees are higher than necessary and are observable.
24. **JSON-RPC `spendoutputs` randomises the change position unconditionally**
    (`internal/rpc/jsonrpc/methods.go:4389`); with no change output `RandomizeOutputPosition` indexes
    position −1 and panics (`wallet/txauthor/author.go:171-175`). `cmd/movefunds` passes a nil change
    source, which panics at `fetchChange.ScriptSize()` (`cmd/movefunds/main.go:189`).
25. **Untyped algorithm constants.** `OutputSelectionAlgorithmDefault`/`All` are untyped integers
    (`wallet/createtx.go:65-73`).
26. **Multisig fee allowance by magic number**, including the retired testnet2 magic `0x48e7a065`
    (`wallet/createtx.go:584-594`): on mainnet/testnet3 the wallet insists on 0.5 DCR more than the
    amount being available even though the real fee is far smaller.
27. **Multisig path estimates P2PKH inputs with the P2SH sigScript size** (110 instead of 108), adds
    change without a dust test, can add change when the estimate assumed none (when `totalInput` equals
    `amount + allowance` exactly), skips script validation, returns a `CreatedTx` with unset change and
    fee fields, and publishes inside the open write transaction
    (`wallet/createtx.go:625-631,659-688,700-723`).
28. **Multisig path does not record the transaction.** Only the multisig output row is stored
    (`wallet/createtx.go:712`); the spent inputs remain selectable until the transaction comes back from
    the network.
29. **The "not enough funds to send to multisig address" branch is effectively unreachable**: a short
    selection already returns `InsufficientBalance` (`wallet/createtx.go:601-608`).
30. **Consolidation is not randomised** and always takes the lowest-keyed confirmed outputs; with
    `maxNumIns == 0` it selects nothing and fails with `InsufficientBalance`; with `maxNumIns < 0` (the
    JSON-RPC handler passes the request's `inputs` integer through unchecked,
    `internal/rpc/jsonrpc/methods.go:597`) and at least two eligible outputs, `make([]int, 0, maxNumIns)`
    panics with a negative capacity (`wallet/createtx.go:850`); it publishes inside the open
    write transaction and does not reload the filter (`wallet/createtx.go:785-920`).
31. **`signP2PKHMsgTx` silently leaves an input unsigned** when its script does not yield exactly one
    address (`wallet/createtx.go:2000-2002`); key-zeroing is deferred to function exit inside a loop.
32. **Two eligibility implementations disagree.** The store input source decides by the recorded stake
    opcode and coinbase flag, accepts P2PK, rejects zero-value credits, and logs
    `Unexpected script class for credit` for stake-tagged P2SH; `findEligibleOutputs*` decide by script
    classification, reject P2PK, accept zero-value credits and accept stake-tagged P2SH credits whose
    address maps to the account, which `signP2PKHMsgTx` then fails on with kind `Bug`
    (`wallet/udb/txmined.go:3196-3332`, `wallet/createtx.go:1750-1822`).
33. **`findEligibleOutputsAmount` throws away its random-phase picks** and restarts from a shuffled full
    scan when the random budget is exhausted; random draws are with replacement
    (`wallet/createtx.go:1915-1957`).
34. **`treasury-add` in the maturity switch is unreachable** in `findEligibleOutputs*` because such scripts
    yield no address and are filtered earlier (`wallet/createtx.go:1771-1788`).
35. **Script version is assumed 0 everywhere** (`// XXX` markers at `wallet/createtx.go:1078,1815,1930,1967,
    2139,2150`); previous-output versions are not carried.
36. **Revocation signing branch is dead** (`isVote == false` has no caller, `wallet/createtx.go:2056-2059`),
    as are the unexported `stakeAddress` methods of `xpubAddress` (`wallet/addresses.go:256-382`), the
    `"no method to update DB with addresses"` branch (`wallet/addresses.go:634-636`), and the `Fee` and
    `ChangeAddr` fields of `CreatedTx`.
37. **`xpubAddress.PubKey` panics** on derivation failure (`wallet/addresses.go:306-318`).
38. **`xpubAddress.AccountKind` reports imported voting accounts as `AccountKindImportedXpub`**, and the
    account name of allocator-returned addresses is whatever the caller supplied, usually empty
    (`wallet/addresses.go:290-297,638-645`).
39. **Gap limit 0.** Nothing in this scope validates `gapLimit`; with 0, `gapPolicyError` always fails,
    `gapPolicyWrap` always returns child `lastUsed + 1`, and `gapPolicyIgnore` divides by zero and panics
    (`wallet/addresses.go:572-604`). The daemon does not reject 0: `config.go` only declares the
    `--gaplimit` flag and its default (`config.go:111,371`), and the value is passed unchanged through
    `dcrwallet.go:169`, `walletsetup.go:116` and `internal/loader/loader.go:57-65,173,263,322` into
    `wallet/wallet.go:5398`.
40. **`gapPolicyIgnore` performs network I/O under `addressBuffersMu`** and returns the backend error
    unwrapped (`wallet/addresses.go:578-599`).
41. **Self-contained `nextAddress` can return both an address and an error** (named results; the commit
    error is assigned after the address) and never rewinds the cursor (`wallet/addresses.go:531-543`).
42. **`watchHDAddrs` marks branches watched before loading them** on the first watch, re-sends the
    boundary address on later calls, iterates accounts in Go map order, and issues `LoadTxFilter` calls
    concurrently, so the order of filter loads is nondeterministic (`wallet/wallet.go:957-1062`). It would
    dereference a nil buffer if the database had an account without a buffer.
43. **`individualSplit` with `DontSignTx` drops change-address row updates**, and `PurchaseTickets` writes
    the fee-split transaction's change updates twice (`wallet/createtx.go:1137-1149`,
    `wallet/wallet.go:1673-1684`).
44. **`FetchP2SHMultiSigOutput`** ignores one address-construction error and returns the second
    unwrapped; **`PrepareRedeemMultiSigOutTxOutput`** assumes a 110-byte sigScript per input regardless of
    M and N (under-estimating real multisig redemptions) and forces output script version 0
    (`wallet/multisig.go:38,46-50,81-90`).
45. **Wall-clock fields.** `NewTxRecordFromMsgTx(…, time.Now())` and address-row `addTime` make database
    bytes time-dependent.
46. **Duplicate helpers.** `confirmed`, `confirms`, `coinbaseMatured`, `ticketChangeMatured`,
    `ticketMatured` exist identically in `wallet` and `wallet/udb`; `sumOutputValues` exists in `txauthor`,
    `txrules` and both RPC servers.
47. **Nested transactions.** Change allocation inside an open read or write transaction opens a second
    read transaction (`notVotingAcct`) on the same goroutine (`wallet/addresses.go:778-795,801-803`).
48. **`SyncLastReturnedAddress` can move the cursor backward and is not atomic.** When the branch has a
    used address and `child > lastUsed`, the cursor is **assigned** `child − lastUsed`, even if it was
    larger, so the next address handed out becomes `child + 1` and previously issued higher children are
    issued again; the database side (`MarkReturnedChildIndex`) only ever raises the stored value. The
    in-memory change happens before the write transaction and is not reverted when that transaction
    fails, for example with `child >= 2^31` (`Invalid`, `child index <n> exceeds max`); no range check
    precedes it (`wallet/addresses.go:870-891`).
49. **Derivation helpers fail near the top of the index space.** `deriveChildAddresses` computes
    `startIndex + i` in wrapping uint32 and skips only `ErrInvalidChild`; an index at or above `2^31` on
    a public branch key fails the whole batch with `cannot derive a hardened key from a public key`. This
    reaches `nextAddress` under `gapPolicyIgnore` (which derives `gapLimit` children ahead) and
    `SyncLastReturnedAddress` (`wallet/addresses.go:591-595,900-904,1004-1021`).
50. **Direct `nextAddress` callers bypass the voting-account check** (ticket vote/commitment addresses,
    mixed outputs, mix change), while every public allocator and every change source applies it
    (`wallet/createtx.go:1187-1190`, `wallet/mixing.go:60-61,401-402`, `wallet/addresses.go:753,767,801`).
51. **Mixed-split change address is consumed even when the change is dropped.** `mixedSplit` lets the
    authoring loop allocate a change address, then discards the change output if it is below
    `smallestMixChange`; the address row update is still written and the cursor stays advanced. The
    function ignores `req.DontSignTx`, returns `cj.MixedIndices()` as the output indexes, and returns all
    errors without an op (`wallet/createtx.go:996-1093`).
52. **`gRPC ConstructTransaction` ignores the wallet's configured fee rate** and falls back to the
    compile-time default, unlike every JSON-RPC builder (`internal/rpc/rpcserver/server.go:1226-1229`).
    Its `fee_per_kb` is atoms, while `SweepAccount.fee_per_kb` is DCR.
53. **`sendrawtransaction` ignores `--allowhighfees`** and `fundrawtransaction` accepts a negative
    `feerate` (which `FeeForSerializeSize` turns into a zero or `MaxAmount` fee, 4.2)
    (`internal/rpc/jsonrpc/methods.go:4691-4703,898-903`). No entry point rejects a zero or negative
    `--txfee`.
54. **High-fee message differs by path**: `high fee` from wallet-authored transactions, `high fees` from
    raw publication (`wallet/createtx.go:324`, `internal/rpc/jsonrpc/methods.go:4701`,
    `internal/rpc/rpcserver/server.go:1777`).
55. **Stale or wrong comments** (code wins):
    - `WithGapPolicyError` tells callers to check for a "GapLimit error code"; no such kind exists, the
      error is kind `Policy` (`wallet/addresses.go:415-419,575-576`).
    - `txauthor.NewUnsignedTransaction` documents an `InputSourceError`; the code returns
      `errors.InsufficientBalance` (`wallet/txauthor/author.go:86-88,108`). It also says the change is "a
      P2PKH change output"; any script the change source returns is used.
    - `RedeemP2PKHInputSize`'s comment lists a "compact int encoding value 107" and a 107-byte signature
      script; the constant is built from `RedeemP2PKHSigScriptSize` = 108 (`wallet/txsizes/size.go:42-55`).
      `TSPENDInputSize` has no descriptive sentence and is a script size, not an input size (quirk 23).
    - `StakePoolTicketFee`'s comment says the percentage "is truncated from 0.01% to 100.00%"; nothing
      clamps or validates it, and `ValidPoolFeeRate` has no caller (`wallet/txrules/poolfees.go:17-35`).
      `doc.go` defines `p` as a proportion (`0.5000 = 50.00%`) while the function takes a percentage.
    - `signP2PKHMsgTx` says it "sets the SignatureScript for every item"; see quirk 31.
    - `SyncLastReturnedAddress` says "the next returned address for the branch will be child+1"; see
      quirks 3 and 48.
    - `PaysHighFees` and `TxPaysHighFees` say "signed transaction"; neither checks for signatures, and
      `authorTx` calls the former on unsigned transactions (quirk 12).
56. **`AddAllInputScripts` calls `secrets.ChainParams()` before the length check**, so a nil
    `SecretsSource` panics even for a mismatched call (`wallet/txauthor/author.go:205-213`).
57. **`StakePoolTicketFee` big-integer semantics.** The quotient uses Go `big.Int.Div` (Euclidean
    division) and `Rsh` (arithmetic shift, rounding toward negative infinity), and the result is narrowed
    with `big.Int.Int64()` (documented as undefined when the value does not fit). These differ from truncation only for
    negative or overflowing operands, which need a negative ticket price, fee term or percentage. The
    float-to-integer conversion `int64(math.Floor(poolFee * 100.0))` is implementation-defined in Go for
    NaN and out-of-range values (`wallet/txrules/poolfees.go:43-44,73-94`). Settled by the oracle
    (2026-11-19, QK-0743): amd64 gives `0x8000000000000000` for a NaN, ±Inf and any value outside
    [-2^63, 2^63), arm64 saturates and gives 0 for a NaN; `purse-txrules` gives amd64's answer
    (ADR-0012 Rule 9(a)) and reproduces the big-integer semantics.
58. **`SecretsSource.Close` is not idempotent-safe for later use**: after a successful close `dbtx` is
    nil and `GetKey`/`GetScript` panic; after a failed rollback the callbacks have already run and the
    transaction is kept (`wallet/createtx.go:228-238`).
59. **`insertCreditsIntoTxMgr` adds one credit per owned address**, so a multi-address output script with
    two wallet-owned keys calls `AddCredit` twice for the same output index
    (`wallet/createtx.go:274-292`). Its only caller builds single-address outputs.
60. **Source TODO/XXX markers in scope** not covered above: per-account credit tracking
    (`wallet/createtx.go:279`), passing `isTreasury` into the authoring function (`:455`), recording
    authored transactions through the notification path "does a lot of extra work" (`:529`), "make a
    better fee estimator" for multisig (`:582`), "Import from dcrd" for the 100000-byte standard size
    (`:843`), `context.TODO()` for the post-purchase filter load (`:1252`), "bad estimate" markers in the
    VSP fee reservation (`:1393,1411`), moving eligibility filters into the store (`:1744`), multisig
    eligibility (`:1806`), call options "add to wallet config later" (`wallet/addresses.go:546`), and the
    `SecretsSource` interface redesign (`wallet/txauthor/author.go:189`). None changes behaviour at the
    pin.

## 10. Upstream tests

| Test | File | What it pins | Port |
|---|---|---|---|
| `TestEstimateSerializeSize` | `wallet/txsizes/size_test.go` | 17 vectors for `EstimateSerializeSize`: 1–2 P2PKH inputs with P2PKH/P2SH outputs and optional change (181, 217, 217, 253, 215, 251, 347, 383, 383, 419, 381, 417); varint boundary at 252/253 outputs (9253, 9253+36+2) and inputs (41847, 41847+166+4) | Port verbatim |
| `TestNewUnsignedTransaction` | `wallet/txauthor/author_test.go` | 13 cases: insufficient funds; change amounts at rates 1e3/1e4/2.55e3; dust thresholds 602/603 and 1537/1538; one-of-two inputs used; dust remainder does not pull a second input; two inputs needed; zero fee rate with exact amount yields no change | Port verbatim. The helper builds "P2PKH outputs" with 36-byte scripts (`P2PKHOutputSize`), which the vectors depend on |
| `TestStakePoolTicketFee` | `wallet/txrules/poolfees_test.go` | 7 mainnet vectors: (10 DCR, 0.01, h25000, 1.00%) → 1500463; (20, 0.01, 25000, 1.00) → 1621221; (5, 0.05, 50000, 2.59) → 3310616; (15, 0.05, 50000, 2.59) → 3956376; same with DCP0010 → 9023823; with DCP0012 (with or without DCP0010) → 9784185 | Port verbatim |
| `TestAddresses` | `wallet/addresses_test.go` | First five internal and external simnet addresses, hash160s and public keys for a fixed 32-byte seed, for locked/unlocked and watching-only wallets, via `NewInternalAddress`/`NewExternalAddress` + `KnownAddress` (`BIP0044Address.Path`, `PubKey`, `PubKeyHash`, `AccountName`) | Port verbatim |
| `TestAccountIndexes` | `wallet/addresses_test.go` | `(lastUsed, cursor)` after sequences of allocations, `watchHDAddrs`, and `markUsedAddress` with gap limit 20: `(∅,0) → (∅,1) → (∅,20) → (∅,20) → use 10: (10,9) → (10,10) → (10,20) → use 30: (30,0) → use 31: (31,0)` | Port verbatim |
| `TestDiscoveryCursorPos` | `wallet/discovery_test.go` | Discovery does not rewind the cursor: with the external cursor preset to 9 (and `lastUsed = 0xffffffff`) discovery leaves `(∅, 9)`; after marking child 4 used and recording child 9 as returned, discovery gives `lastUsed = 4`, `cursor = 4` | Port (with the discovery spec) |
| `TestCoinbaseMatured` | `wallet/wallet_test.go` | `coinbaseMatured` truth table on mainnet (maturity 256), including negative heights | Port verbatim |

`wallet/udb/testdata/v5.db.go` contains a private copy of `createUnsignedVote`/`newVoteScript` used to
generate database fixtures; it is not a test of the wallet functions.

Test support the in-scope tests depend on (port alongside them):

| File | Provides |
|---|---|
| `wallet/setup_test.go` | `basicWalletConfig` (`GapLimit: 20`, `RelayFee: 1e5` atoms/kB, simnet parameters, `MixingEnabled: true`, the insecure public passphrase) and `testWallet`, which creates a temporary database, creates the wallet from a seed and opens it |
| `wallet/network_test.go` | `mockNetwork`, a `NetworkBackend` whose `PublishTransactions` and `LoadTxFilter` succeed without doing anything; `TestAccountIndexes` installs it so `watchHDAddrs` and `gapPolicyIgnore` have a backend |
| `wallet/main_test.go` | `TestMain` for the package; its `-log` flag attaches a stderr logger for subsystem `WLLT` |
| `wallet/addresses_test.go:15-185` | The fixed seed (`b46bc650…8d641567`), the expected address/hash160/public-key tables for both branches, and the helper that checks `KnownAddress` results for locked, unlocked and watching-only wallets |
| `wallet/txauthor/author_test.go:19-64` | A change source returning 25 zero bytes with `ScriptSize() = 25`, the `p2pkhOutputs` helper (36-byte scripts), and an input source that returns inputs one at a time, cumulatively, each with sigScript size 108 |

`TestAccountIndexes` drives usage through `markUsedAddress("", dbtx, ma)` followed by
`watchHDAddrs(ctx, false, n)`; the expected `(lastUsed, cursor)` pairs therefore pin both the database
side of 4.23 and the buffer re-synchronisation rule. `TestTicketMatured` and `TestTicketExpired`
(`wallet/wallet_test.go:48-101`) pin the duplicated `ticketMatured`/`ticketExpired` helpers that sit next
to the spendability predicates of 4.13.1 (owned by the ticket specs). No test in `wallet/` exercises
`createtx.go` or `multisig.go` directly, and neither RPC server has tests for the fee-rate or gap-policy
argument handling of 4.28–4.29.

**Untested upstream (needs new tests, preferably against a Go oracle shim):**

- `txrules`: `IsDustAmount`, `IsDustOutput`, `CheckOutput`, `FeeForSerializeSize` (overflow, zero and
  negative rates), `PaysHighFees`, `TxPaysHighFees`, `ValidPoolFeeRate`, `StakeSubScriptType`; non-mainnet
  and float-edge inputs of `StakePoolTicketFee`.
- `txsizes.EstimateSerializeSizeFromScriptSizes`, `EstimateInputSize`, `EstimateOutputSize` directly.
- `txauthor.AddAllInputScripts` (P2PKH, P2PK, P2SH, stake-tagged, merge with an existing script) and the
  size-limit and oversized-change-script errors. Covered since 2026-11-19 by the `sign` and `author`
  commands of the oracle `tools/oracle/cmd/txauthor` and `purse-txauthor`'s differential, which found
  QK-0745 and QK-0746.
- Everything in `wallet/createtx.go`: `authorTx` (including treasury), `recordAuthoredTx`,
  `publishAndWatch` failure path, multisig, consolidation, both `findEligibleOutputs*` functions,
  `ReserveOutputsForAmount`, `signP2PKHMsgTx`, `makeTicket`, `createUnsignedVote`, `signVote`.
- `wallet/multisig.go`.
- `MakeInputSource` and `randomUTXO` (statistical tests plus deterministic tests with an injected RNG).
- `nextAddress` under `gapPolicyIgnore` and `gapPolicyWrap`, branch exhaustion, invalid-child skipping,
  the three persistence modes; `SyncLastReturnedAddress`; `AddressAtIdx`; `CurrentAddress`;
  `ImportedAddresses`; `watchHDAddrs` with `--watchlast`; the restart behaviour of quirk 1.

Oracle-shim candidates (pure functions, cheap to drive from Go): the four `txsizes` functions, the
`txrules` functions, `txauthor.NewUnsignedTransaction` with scripted input/change sources,
`createUnsignedVote`, `makeTicket`, and signing of fixed transactions with fixed keys (signatures are
deterministic).

## 11. Rust porting notes

**Suggested boundaries.**

| Crate / module | Contents | Dependencies |
|---|---|---|
| `purse-txrules` (pure) | `txsizes` constants and estimators, `txrules` fee/dust/high-fee functions, `StakePoolTicketFee`, `StakeSubScriptType` | wire types, txscript helpers, subsidy cache |
| `purse-txauthor` (pure) | `InputDetail`, `InputSource` (a trait or `FnMut(Amount) -> Result<InputDetail>`), `ChangeSource` trait, `AuthoredTx`, authoring loop, randomisation helpers, `SecretsSource` trait and signing loop | `purse-txrules`, txscript sign |
| wallet crate, module `createtx` | `authorTx` family, selection wrappers, lock set, multisig, consolidation, split builders, `makeTicket`, vote builder/signer, `multisig.rs` | store, address manager, network backend trait |
| wallet crate, module `addresses` | Address wrapper types, address buffers, gap policy, allocators, change sources, `watch_hd_addrs` | HD keys, address manager, network backend trait |

**Reuse from dcroxide (inspected).**

- `dcroxide-wire`: `MsgTx`, `TxIn`, `TxOut`, `OutPoint`, `TxOut::serialize_size`, `MsgTx::serialize_size`,
  `var_int_serialize_size` (`dcroxide:crates/dcroxide-wire/src/varint.rs:62`,
  `dcroxide:crates/dcroxide-wire/src/msgtx.rs:160-656`).
- `dcroxide-txscript`: `sign::sign_tx_output`, `sign::signature_script`, `sign::raw_tx_in_signature`, the
  `KeyDb`/`ScriptDb` traits (`dcroxide:crates/dcroxide-txscript/src/sign.rs:62-90,119,184,606`);
  `stdscript::multi_sig_script_v0`, `provably_pruneable_script_v0`, `extract_addrs`
  (`dcroxide:crates/dcroxide-txscript/src/stdscript.rs:842,896,1157`); `stdaddr::Address` with
  `payment_script`, `voting_rights_script`, `reward_commitment_script`, `stake_change_script`,
  `pay_vote_commitment_script` (`dcroxide:crates/dcroxide-txscript/src/stdaddr.rs:483-609`);
  `script::is_unspendable`, `generate_ssgen_block_ref`
  (`dcroxide:crates/dcroxide-txscript/src/script.rs:404,420`); the script `Engine` and `ScriptFlags`
  (`dcroxide:crates/dcroxide-txscript/src/engine.rs:32,101,560,603`).
- `dcroxide-stake`: `sstx_null_output_amounts`, `calculate_rewards`, `check_sstx`,
  `tx_sstx_stake_output_info` (`dcroxide:crates/dcroxide-stake/src/lib.rs:386,458,562,689`).
- `dcroxide-standalone`: `SubsidyCache::calc_stake_vote_subsidy_v3`
  (`dcroxide:crates/dcroxide-standalone/src/subsidy.rs:390`).
- `dcroxide-crypto`: `rand::read`, `int_n`, `duration`, `shuffle_slice`, and `Prng::from_seed` for
  deterministic tests (`dcroxide:crates/dcroxide-crypto/src/rand.rs:159-483`).
- `dcroxide-mempool` has `calc_min_required_tx_relay_fee` and `is_dust`
  (`dcroxide:crates/dcroxide-mempool/src/policy.rs:71,230`). The fee function is arithmetically identical
  to `FeeForSerializeSize`. `is_dust` is **not** a drop-in for `IsDustOutput`: the wallet exempts
  null-data scripts before the unspendable test. Implement the wallet's versions in `purse-txrules`
  rather than depending on the mempool crate.

**Reuse from dcr-rs (inspected).** `ExtPubKey::derive_child` (`dcr-rs:src/hd.rs:574`) implements public
child derivation with dcrd's rejection of an overflowing or zero tweak, but reports an invalid tweak, a
depth overflow and a failed point addition all as the one `Error::Derivation` value
(`dcr-rs:src/lib.rs:65-69`, `dcr-rs:src/hd.rs:122-128,578,592-595`; a hardened index is the separate
`Error::HardenedFromPublic`). dcrd does not test the final point sum for infinity
(`hdkeychain/v3@v3.1.3/extendedkey.go:334-341`), so dcr-rs additionally refuses a sum that dcrd would
serialize. The allocator must
distinguish "invalid child, skip this index" from other failures (depth overflow), so either extend the error type or wrap derivation in a function
that reproduces `hdkeychain.ErrInvalidChild` semantics exactly. `dcr-rs:src/address.rs:152-164`
(`p2pkh_script`, `p2sh_script`) produce the same 25- and 23-byte scripts as 3.3. dcroxide has no HD key
crate.

**Hazards.**

- Integer semantics: `Amount` and sizes are int64 with wrapping multiplication and truncating division;
  child-index arithmetic is wrapping uint32 with `0xffffffff` as "none". Use explicit `wrapping_*`
  operations and mirror the comparisons literally (`a + 1 > b + 1`, not `a > b`).
- `StakePoolTicketFee` needs a float64 multiply and floor on the percentage and arbitrary-precision
  integers (a 256-bit intermediate suffices: three factors below 2^63 shifted by 64 bits can exceed 192
  bits only marginally; use a big-integer type to be safe).
- Randomness: selection order, change position, and shuffles come from a CSPRNG. Inject the RNG so tests
  can be deterministic; do not attempt to match Go's byte stream in production.
- The input source borrows a database read transaction and keeps cursor-like state across calls; in Rust
  this wants a struct holding the transaction reference with an explicit lifetime, not a boxed closure
  that outlives it.
- Lock ordering must be chosen deliberately (quirk 6) and nested read transactions (quirk 47) must be
  supported by the storage layer or designed out by passing the open transaction down.
- Publishing inside an open write transaction (multisig, consolidation) holds the writer across network
  I/O; an async port must not hold a synchronous database write lock across an await without thought.
- Private-key hygiene: wrap key bytes in zeroizing containers; the Go code leaves serialized copies
  un-zeroed.
- `RandomizeOutputPosition` and `xpubAddress.PubKey` panic in Go; return errors in Rust unless a panic is
  itself the compatible behaviour (it is not externally depended on).

**Size estimate: L.** The pure crates are small (S): roughly 600 lines of Go with complete vectors. The
wallet-side code is the bulk: about 1,700 lines of in-scope `createtx.go` and `addresses.go` plus about
500 lines of selection and filter-watch code in other files, almost none of it covered by upstream
tests, with heavy coupling to the store, the address manager, the network backend and three mutexes, and
with many behaviours (section 9) that need explicit keep-or-fix decisions and new tests.

## 12. Open questions

1. **Last-returned persistence (quirk 1).** Reproduce the pin (addresses re-issued after restart, key
   counts not advancing) or restore the `MarkReturnedChildIndex` write? Restoring it is compatible with
   the database format (the variables exist and older dcrwallet versions wrote them) but changes which
   address `getnewaddress` returns after a restart relative to dcrwallet at the pin. Needs a project
   decision and, ideally, confirmation of upstream intent.
2. **Lock order for address buffers vs database writes (quirk 6).** Pick one order for the port; this is
   an internal change with no compatible-behaviour cost.
3. **Panics (quirk 24, 37, 39).** Decide whether the port returns errors where Go panics. For JSON-RPC
   `spendoutputs` without change, UNVERIFIED what a client observes at the pin (whether the RPC server
   recovers the panic or the process dies); settle by reading the JSON-RPC dispatch code or running the
   Go daemon.
4. **Fee over-estimates caused by misused size constants (quirk 23).** These produce observable fees.
   Reproduce exactly (recommended for parity) or correct?
5. **Multisig and consolidation legacy paths (quirks 26–30).** Reproduce bug for bug, including not
   recording the multisig funding transaction and publishing inside the write transaction, or tidy up
   while keeping the transaction bytes identical?
6. **Gap limit 0 (quirk 39).** Nothing at the pin rejects `--gaplimit=0` (see quirk 39). Decide whether
   the port reproduces the resulting failures and divide-by-zero panic or rejects the value at startup.
7. **P2SH stake scripts.** The P2SH variants of `VotingRightsScript`, `RewardCommitmentScript` and
   `PayVoteCommitmentScript` were not re-read for this spec; the ticket/voting specs or the dcroxide
   `stdaddr` port are the reference.
8. **Subsidy function.** `CalcStakeVoteSubsidyV3` was taken as given from dcroxide's port; the VSP-fee
   vectors in section 10 exercise it end to end.
9. **Concurrent double selection (quirk 11).** Keep the window between authoring and recording, or hold
   the inputs locked across it? Holding them changes only failure timing for racing sends.
10. **Random selection distribution.** `randomUTXO` is not uniform over outputs (it is weighted by gaps in
    transaction-hash space). Is statistical equivalence required, or is any privacy-preserving random
    order acceptable? Nothing external can depend on the exact distribution.
11. **`SyncLastReturnedAddress` rewind (quirk 48).** Reproduce the unconditional cursor assignment and
    the non-atomic memory/database update, or only ever advance? `accountsyncaddressindex` clients observe
    the difference only when they pass an index below the current next index.
12. **Fee-rate validation (quirks 52–53).** Keep accepting zero and negative rates from `--txfee`,
    `fundrawtransaction` and gRPC `ConstructTransaction`, and keep `ConstructTransaction`'s independence
    from `settxfee`, or validate and unify? Any change alters fees clients can observe.
13. **`StakePoolTicketFee` on invalid inputs (quirk 57).** Settled (2026-11-19): the oracle
    `tools/oracle/cmd/txrules` gives Go's results for NaN, infinite and negative percentages and
    negative amounts, and `purse-txrules` matches them (QK-0743); a zero denominator panics in Go and
    is an error in the port (QK-0742).
