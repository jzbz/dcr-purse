# 04 — Transaction store: mined transaction logic, balances, queries

## Header

**Scope (files, line counts at the parity pin)**

| File | Lines | Status in this spec |
|---|---|---|
| `wallet/udb/txmined.go` | 3583 | Owned. Every type, function and method. |
| `wallet/udb/txquery.go` | 773 | Owned. Every type, function and method. |
| `wallet/udb/txunmined.go` | 410 | Covered in full. The assignment names `InsertMemPoolTx` and unmined pruning, and the mined-side algorithms cannot be specified without `RemoveUnconfirmed`. |
| `wallet/udb/kahnsort.go` | 116 | Covered (decides the order of `UnminedTxs`). |
| `wallet/udb/cfilters.go` | 67 | Covered briefly (two `Store` methods that share the cfilter state written by `ExtendMainChain`). |
| `wallet/udb/txdb.go` | 2438 | Read in full for semantics. Byte formats are owned by `03-txstore-formats.md` and are referenced here by name only, except where a format is defined inside `txmined.go` itself (birthday state, header field offsets). |

**Parity pin**

- dcrwallet `54b85b33` (`decred.org/dcrwallet/v5`, version 2.2.0-pre).
- Dependencies read at the versions `go.mod` pins: `dcrd/blockchain/stake/v5@v5.0.3`, `dcrd/blockchain/standalone/v2@v2.3.0`, `dcrd/crypto/rand@v1.0.1`, `dcrd/txscript/v4@v4.1.2`, `dcrd/wire@v1.8.0`, `dcrd/gcs/v4@v4.1.1`, `dcrd/chaincfg/v3@v3.3.0`, `dcrd/chaincfg/chainhash@v1.0.5`, `dcrd/dcrutil/v4@v4.0.3`, `bbolt@v1.3.12`.

**Summary**

`udb.Store` is the wallet's transaction store. It lives in the `wtxmgr` top-level bucket of `wallet.db` and records the main chain (headers, per-height block records, compact filters), every wallet-relevant transaction (mined or unmined), which outputs are wallet credits, which inputs debit those credits, ticket purchases, ticket commitments and P2SH multisig outputs. It has no in-memory state: every method runs inside a database transaction supplied by the caller and reads or writes buckets directly. The store never decides relevance; the wallet layer tells it which outputs are credits. Balances are not stored per account: they are recomputed on every query by a full scan of the unspent index, the unmined credits and the unspent ticket-commitment index. Coin selection is randomized (random seek into the unspent index, then a shuffled remainder). Reorgs are handled by `Rollback`, which moves the transactions of detached blocks back to the unmined set, and by stake (in)validation of the parent block's regular tree in `ExtendMainChain`. Several behaviours are bugs or accidents of the Go code that an existing `wallet.db` and existing RPC clients already depend on; they are listed in sections 9 and 10.

---

## 2. Role in the system

### 2.1 What it does

- Maintains the wallet's view of the main chain: `ExtendMainChain` appends one header + block record + cfilter; `Rollback` detaches blocks from a height upward; `MainChainTip` reads the tip.
- Records transactions: `InsertMemPoolTx` (unmined), `InsertMinedTx` (mined, including the unmined to mined move), `RemoveUnconfirmed`, `PruneUnmined`, `SetPublished`.
- Records ownership: `AddCredit`, `AddTicketCommitment`, `RedeemTicketCommitments`, `AddMultisigOut`, `SpendMultisigOut`.
- Tracks spending: debits for mined spenders, the unmined-inputs index for unmined spenders, the `bal` root key (mined balance).
- Answers queries: balances per account, UTXO enumeration, randomized coin selection, transaction details and range scans, ticket enumeration, spender lookup, block and header lookups, cfilter lookups, birthday state.

### 2.2 Who calls it

The only caller is package `wallet` (and, through it, the RPC servers). The store is constructed by `udb.Open` (`wallet/udb/open.go:46-50`) with `chainParams`, `acctLookupFunc = Manager.AddrAccount` and `manager = *Manager`. Representative call sites:

| Caller | Store methods used |
|---|---|
| `wallet/chainntfns.go:29-83` `extendMainChain` | `ExtendMainChain`, `GetBlockMetaForHash`, `ExistsTx` |
| `wallet/chainntfns.go:91-316` `ChainSwitch` | `MainChainTip`, `GetMainChainBlockHashForHeight`, `GetBlockHeader`, `CFilterV2`, `Rollback`, `UpdateProcessedTxsBlockMarker`, `PruneUnmined` (plus the free functions `BirthState` / `SetBirthState`) |
| `wallet/chainntfns.go:438-676` `processTransactionRecord` | `InsertMemPoolTx` or `InsertMinedTx`, then `GetMultisigOutput`/`SpendMultisigOut` per multisig input, then per owned output `AddTicketCommitment` or `AddCredit`, `AddMultisigOut`, then `RedeemTicketCommitments` for votes/revocations, then `UniqueTxDetails` for the notification |
| `wallet/createtx.go`, `wallet/utxos.go` | `MakeInputSource`, `UnspentOutputs`, `RandomUTXO`, `UnspentOutputCount` |
| `wallet/wallet.go` | `AccountBalance` (the wallet's own `AccountBalances` calls `Store.AccountBalance` once per account, `wallet/wallet.go:1927-1946`; `Store.AccountBalances` has no caller outside `udb`), `RangeTransactions`, `RangeBlocks`, `UnspentOutputs`, `UnspentTickets`, `IterateTickets`, `UnminedTxs`, `Spender`, `RemoveUnconfirmed`, `SetPublished`, `ForEachUnspentOutpoint`, `TxDetails`, `Tx`, `TxBlockHeight` |
| `wallet/tickets.go` | `IterateTickets` (`wallet/vspticket.go:238` reaches `Spender` only through the wallet's `Spender` wrapper, `wallet/wallet.go:3313`) |
| `wallet/notifications.go` | `UnspentOutputs`, `UnminedTxHashes` |
| `wallet/unstable.go` | `RangeTransactions`, `UnspentMultisigCreditsForAddress` |
| `wallet/rescan.go`, `wallet/difficulty.go` | `GetBlockMetaForHash`, `GetBlockHeaderTime`, header lookups |
| `wallet/createtx.go:251-314` (`insertIntoTxMgr`, `insertCreditsIntoTxMgr`, `insertMultisigOutIntoTxMgr`) | Wallet-authored transactions are recorded directly, not through `processTransactionRecord`: `NewTxRecordFromMsgTx(tx, time.Now())` then `InsertMemPoolTx`; then `AddCredit(dbtx, rec, nil, i, ma.Internal(), ma.Account())` for every output address the address manager knows (once per decoded address, so a multi-address script calls it repeatedly for the same index; the second call is the "already exists" no-op); `AddMultisigOut(dbtx, rec, nil, index)` with a freshly built record. `wallet/createtx.go:1911-1916` calls `UnspentOutputCount` and `RandomUTXO`. |
| `wallet/chainntfns.go:392`, `686` | `UnspentOutput(ns, prevOut, true)` (include mempool) while processing inputs; `OwnTicket` |
| `wallet/rescan.go:157`, `234`, `366`, `478` | `ExistsTxMinedOrUnmined`, `GetMainChainBlockHashes`, `GetSerializedBlockHeader`, `ProcessedTxsBlockMarker` (plus `UpdateProcessedTxsBlockMarker`) |
| `wallet/discovery.go:195`, `399` | `GetMainChainCFilters` (always `inclusive = true` at line 195) |
| `wallet/wallet.go` (remaining direct uses) | `OwnTicket` (430, 487), `RangeBlocks` (875), `IsMissingMainChainCFilters` / `MissingCFiltersHeight` / `GetMainChainBlockHashes` / `InsertMissingCFilters` (1297-1401), `TicketDetails` (2830, 3121, 3195), `UnspentOutput` (3328), `ExistsUTXO` (4963), `ImportCFiltersV2` (5281), `GetSerializedBlockHeader` (many) |
| `wallet/notifications.go:832-958` | `TxBlockHeight` |
| none | `IsUnspentOutpoint`, `PreviousPkScripts`, `Store.AccountBalances` (repository search at the pin) |

The ordering inside `processTransactionRecord` matters to this spec: `InsertMinedTx`/`InsertMemPoolTx` always runs before any `AddCredit` for the same record, and `RedeemTicketCommitments` runs last. Failures of `AddMultisigOut` and `RedeemTicketCommitments` are logged by the caller and not propagated (`wallet/chainntfns.go:635-652`).

### 2.3 What it calls

- `walletdb` bucket and cursor API over bbolt (`wallet/internal/bdb`). Cursor semantics relied upon are in section 5.1.
- `Manager.AddrAccount` (as `acctLookupFunc`) and `Manager.redeemScriptForHash160` in the address manager, always through the `waddrmgr` bucket of the same database transaction.
- `stake.DetermineTxType`, `stake.IsSStx`, `stake.IsSSGen`, `stake.SSGenBlockVotedOn`, `stake.AmountFromSStxPkScrCommitment` (`dcrd/blockchain/stake/v5@v5.0.3/staketx.go`).
- `standalone.IsCoinBaseTx` through `compat.IsEitherCoinBaseTx` (`internal/compat/compat.go`).
- `stdscript.DetermineScriptType`, `stdscript.ExtractAddrs`, `stdscript.ExtractMultiSigScriptDetailsV0`, `txscript.MakeScriptTokenizer`.
- `txrules.StakeSubScriptType`, `txsizes.RedeemP2PKHSigScriptSize`, `txsizes.RedeemP2PKSigScriptSize`, `txauthor.InputDetail`.
- `blockcf2.Key`, `gcs.FromBytesV2`, `(*gcs.FilterV2).Bytes`.
- `dcrd/crypto/rand` (`Read`, `IntN`, `ShuffleSlice`): a process-global CSPRNG.
- `wire.MsgTx` (de)serialization, `MsgTx.PkScriptLocs`, `wire.NewTxIn`, `wire.BlockHeader` (de)serialization.
- The package logger `log` (`wallet/udb/log.go`), which the daemon wires to the `WLLT` subsystem (`log.go:33`).

---

## 3. Public surface inventory

Notation: `RTx` = `walletdb.ReadTx`, `RWTx` = `walletdb.ReadWriteTx`, `RB` = `walletdb.ReadBucket` (always the `wtxmgr` namespace bucket unless stated), `RWB` = `walletdb.ReadWriteBucket`. "ns" means the `wtxmgr` bucket. Methods taking a `dbtx` open `wtxmgr` (and `waddrmgr` where noted) themselves; methods taking `ns` expect the caller to pass the `wtxmgr` bucket.

Methods that also open the top-level `waddrmgr` bucket of the same transaction (only ever read, to resolve accounts and redeem scripts): `InsertMinedTx` (`txmined.go:1215`), `AddMultisigOut` (`1744`) and `Rollback` (`1880`) open it with `ReadWriteBucket`; `UnspentMultisigCreditsForAddress` (`2877`), `MakeInputSource` (`3034`) and `balanceFullScan` (`3358`, hence `AccountBalance`/`AccountBalances`) open it with `ReadBucket`. `AddCredit`, `AddTicketCommitment`, `InsertMemPoolTx`, `RemoveUnconfirmed` and every query in `txquery.go` never touch `waddrmgr`.

### 3.1 Types (txmined.go)

| Type | Definition | Notes |
|---|---|---|
| `Block` | `{Hash chainhash.Hash; Height int32}` | `txmined.go:44`. Identifies a block. Height `-1` means unmined wherever a `Block` describes a transaction's location. |
| `BlockMeta` | `{Block; Time time.Time; VoteBits uint16}` | `txmined.go:52`. `VoteBits` is only populated by `GetBlockMetaForHash` / `blockMetaFromHeader`. `Time` is populated by those two and, from the block record, in `TxDetails.Block` (5.16); it is never populated in `Credit.BlockMeta`. See quirk Q-9. |
| `blockRecord` (unexported) | `{Block; Time; VoteBits; transactions []chainhash.Hash}` | `txmined.go:60`. In-memory decode of a block record. `transactions` is in insertion order. |
| `incidence`, `indexedIncidence`, `credit` (unexported) | `incidence {txHash chainhash.Hash; block Block}`; `indexedIncidence {incidence; index uint32}`; `credit {outPoint wire.OutPoint; block Block; amount dcrutil.Amount; change bool; spentBy indexedIncidence; opCode uint8; isCoinbase bool; hasExpiry bool}` (`txmined.go:71-93`) | Internal carriers for a spender/credit while building values. An `incidence` (tx hash + block) rather than a hash alone identifies a mined transaction because duplicate hashes in different blocks are allowed. `credit.spentBy.index == 0xFFFFFFFF` means unspent. `credit.outPoint.Tree` is never set (stays regular) and is not persisted. |
| `TxRecord` | `{MsgTx wire.MsgTx; Hash chainhash.Hash; Received time.Time; SerializedTx []byte; TxType stake.TxType; Unpublished bool}` | `txmined.go:96`. `SerializedTx` may be nil. `Unpublished` is only meaningful for `InsertMemPoolTx` (input) and `UnminedTxs` (output). |
| `MultisigOut` | `{OutPoint *wire.OutPoint; Tree int8; ScriptHash [20]byte; M, N uint8; TxHash, BlockHash chainhash.Hash; BlockHeight uint32; Amount dcrutil.Amount; Spent bool; SpentBy chainhash.Hash; SpentByIndex uint32}` | `txmined.go:145`. Decoded multisig bucket value. |
| `Credit` | `{wire.OutPoint; BlockMeta; Amount; PkScript []byte; Received time.Time; FromCoinBase bool; HasExpiry bool}` | `txmined.go:163`. Result of UTXO queries. `BlockMeta.Time` is always the zero time and `VoteBits` always 0 (Q-9). |
| `Store` | `{chainParams *chaincfg.Params; acctLookupFunc func(RB, stdaddr.Address) (uint32, error); manager *Manager}` | `txmined.go:175`. No other state. |
| `BirthdayState` | `{Hash chainhash.Hash; Height uint32; Time time.Time; SetFromHeight, SetFromTime bool}` | `txmined.go:327`. |
| `BlockCFilter` | `{BlockHash chainhash.Hash; FilterV2 *gcs.FilterV2; Key [16]byte}` | `txmined.go:493`. |
| `RawBlockHeader` | `[180]byte` | `txmined.go:607`. Method `Height() int32`. |
| `BlockHeaderData` | `{BlockHash chainhash.Hash; SerializedHeader RawBlockHeader}` | `txmined.go:618`. Not used by any `Store` method or by any non-test caller at this pin; used only by the `udb` tests and the `wallet/udb/testdata` database generators. |
| `Ticket` | `{TxRecord; Block Block; SpenderHash chainhash.Hash}` | `txmined.go:2734`. `Block.Height == -1` if unmined; `SpenderHash` zero if unspent. |
| `TicketIterator` | `{Ticket; ns; c; ck, cv; err}` | `txmined.go:2741`. Methods `Next() bool`, `Err() error`, `Close()`. |
| `MultisigCredit` | `{OutPoint *wire.OutPoint; ScriptHash [20]byte; MSScript []byte; M, N uint8; Amount dcrutil.Amount}` | `txmined.go:2852`. |
| `InputSource` | `{source func(dcrutil.Amount) (*txauthor.InputDetail, error)}` | `txmined.go:3012`. Method `SelectInputs(target)`. |
| `Balances` | type alias for an anonymous struct `{Account uint32; ImmatureCoinbaseRewards, ImmatureStakeGeneration, LockedByTickets, Spendable, Total, VotingAuthority, Unconfirmed dcrutil.Amount}` | `txmined.go:3548`. Field order as listed. |

### 3.2 Types (txquery.go)

| Type | Definition | Notes |
|---|---|---|
| `CreditRecord` | `{Index uint32; Amount dcrutil.Amount; Spent, Change bool; OpCode uint8; IsCoinbase, HasExpiry bool}` | `txquery.go:23`. For unmined transactions `OpCode` is 0 and `IsCoinbase` is false regardless of the stored flags (Q-10). |
| `DebitRecord` | `{Amount dcrutil.Amount; Index uint32}` | `txquery.go:36`. |
| `TxDetails` | `{TxRecord; Block BlockMeta; Credits []CreditRecord; Debits []DebitRecord}` | `txquery.go:44`. Method `Height() int32` returns `Block.Block.Height`. |
| `TicketDetails` | `{Ticket *TxDetails; Spender *TxDetails}` | `txquery.go:201`. |

### 3.3 Constants and package-level items

| Name | Value | Source | Contract |
|---|---|---|---|
| `opNonstake` | `txscript.OP_NOP10` = `0xb9` | `txmined.go:35` | Opcode tag meaning "not a stake-tagged output". |
| `scriptVersionAssumed` | `0` | `txmined.go:39` | Script version assumed wherever the real version is not threaded through. |
| header field offsets | parent hash 4, merkle root 36, vote bits 100, height 128, timestamp 136 | `txmined.go:545-584` | See 4.2. |

### 3.4 Constructors and free functions

| Function | Signature | Contract |
|---|---|---|
| `NewTxRecord` | `(serializedTx []byte, received time.Time) (*TxRecord, error)` | Deserializes the tx (error returned unwrapped from `wire`), sets `TxType = stake.DetermineTxType`, `Hash = MsgTx.TxHash()`, keeps the caller's slice as `SerializedTx` (no copy). `txmined.go:108`. |
| `NewTxRecordFromMsgTx` | `(msgTx *wire.MsgTx, received time.Time) (*TxRecord, error)` | Serializes the tx into a fresh buffer; copies the `MsgTx` struct by value (shares `TxIn`/`TxOut` pointers with the argument); sets `TxType`, `Hash`. `txmined.go:125`. |
| `SetBirthState` | `(dbtx RWTx, bs *BirthdayState) error` | Writes the 45-byte birthday value (4.1). Returns the raw bucket `Put` error. `txmined.go:341`. |
| `BirthState` | `(dbtx RTx) *BirthdayState` | Returns nil unless the stored value is exactly 45 bytes. `txmined.go:367`. |
| `ExtractBlockHeaderParentHash` | `(header []byte) []byte` | Returns `header[4:36]` (a subslice, not a copy). Panics on short input. |
| `ExtractBlockHeaderHeight` | `(header []byte) int32` | Little-endian `uint32` at offset 128, cast to `int32`. Panics on short input. |
| `ExtractBlockHeaderTime` | `(header []byte) int64` | Little-endian `uint32` at offset 136, widened. Panics on short input. |
| `extractBlockHeaderParentHash`, `extractBlockHeaderHeight` (unexported) | `(header []byte) []byte` / `(header []byte) int32` | The implementations behind the two exported wrappers above (`txmined.go:545`, `568`); offsets 4 (32 bytes) and 128 (LE `uint32` cast to `int32`). The three exported `ExtractBlockHeader*` wrappers carry an upstream "should not be exported" TODO (`txmined.go:558`, `576`, `590`) but are used by package `wallet` (`wallet/rescan.go`, `wallet/wallet.go`). |
| `extractBlockHeaderMerkleRoot`, `extractBlockHeaderVoteBits`, `extractBlockHeaderUnixTime`, `blockMetaFromHeader` (unexported) | | Offsets 36 (32 bytes), 100 (LE `uint16`), 136 (LE `uint32`). `blockMetaFromHeader` builds a `BlockMeta` with `Time = time.Unix(ts, 0)`. |
| `(*RawBlockHeader).Height` | `() int32` | `extractBlockHeaderHeight(h[:])` (`txmined.go:610`). |
| `(*TxDetails).Height` | `() int32` | `t.Block.Block.Height`; `-1` for unmined (`txquery.go:52`). |
| `approvesParent` (unexported) | `(voteBits uint16) bool` | `voteBits & 0x0001 != 0`. `txmined.go:1867`. |
| `getStakeOpCode` (unexported) | `(version uint16, pkScript []byte) uint8` | Maps the script type to the stake opcode tag; see 5.5. |
| `pkScriptType` (unexported) | `(ver uint16, pkScript []byte) scriptType` | Maps the script type to the stored script-type byte; see 5.5. |
| `originalTicketInfo` (unexported) | `(spenderType stake.TxType, spenderTx *wire.MsgTx) (chainhash.Hash, uint32)` | Ticket hash and an upper bound on the ticket's output count, derived from a vote or revocation; see 5.9. |
| `getScriptHashFromP2SHScript` (unexported) | `(pkScript []byte) ([]byte, error)` | Returns the data push that follows the first `OP_HASH160`; nil data (and the tokenizer error, usually nil) if there is none. `txmined.go:1713`. |
| `confirms` (unexported) | `(txHeight, curHeight int32) int32` | 0 if `txHeight == -1` or `txHeight > curHeight`, else `curHeight - txHeight + 1`. |
| `confirmed` (unexported) | `(minConf, txHeight, curHeight int32) bool` | `confirms(txHeight, curHeight) >= minConf`. |
| `coinbaseMatured` (unexported) | `(params, txHeight, curHeight) bool` | `txHeight >= 0 && curHeight - txHeight + 1 > CoinbaseMaturity`. |
| `ticketChangeMatured` (unexported) | `(params, txHeight, curHeight) bool` | `txHeight >= 0 && curHeight - txHeight + 1 > SStxChangeMaturity`. |
| `ticketMatured` (unexported) | `(params, txHeight, curHeight) bool` | `txHeight >= 0 && curHeight - txHeight > TicketMaturity`. Note the missing `+1` (deliberate, mirrors dcrd). |
| `stakeValidate`, `stakeInvalidate` (unexported) | `(ns RWB, height int32) error` | Flip the regular-tree validity of the main chain block at `height`; see 5.4. |
| `dependencySort`, `makeGraph`, `graphRoots` (unexported, kahnsort.go) | | Topological sort of the unmined set; see 5.17. |
| `graphNode`, `hashGraph` (unexported types, kahnsort.go) | `graphNode {value *TxRecord; outEdges []*chainhash.Hash; inDegree int}`; `hashGraph = map[chainhash.Hash]graphNode` | `wallet/udb/kahnsort.go:12-18`. Working state of the sort only. A record that is part of a dependency cycle (impossible for real transactions) never reaches in-degree 0 and is silently omitted from the `UnminedTxs` result. |

### 3.5 `Store` methods: chain and headers (txmined.go)

| Method | Signature | Contract |
|---|---|---|
| `MainChainTip` | `(dbtx RTx) (chainhash.Hash, int32)` | Reads root key `tip`, then that header, returns hash and header height. No error return; panics if the header is missing. |
| `ExtendMainChain` | `(ns RWB, header *wire.BlockHeader, blockHash *chainhash.Hash, f *gcs.FilterV2) error` | Appends one block to the tip. Algorithm 5.3. |
| `ProcessedTxsBlockMarker` | `(dbtx RTx) *chainhash.Hash` | Root key `lasttxsblock` copied into a fresh hash (zero hash if absent). |
| `UpdateProcessedTxsBlockMarker` | `(dbtx RWTx, hash *chainhash.Hash) error` | Advances the marker only forward relative to the main-chain ancestor of the current marker. Algorithm 5.13. |
| `IsMissingMainChainCFilters` | `(dbtx RTx) bool` | True unless root key `havecfilters` is exactly one byte and non-zero: `len(v) != 1 || v[0] == 0`. |
| `MissingCFiltersHeight` | `(dbtx RTx) (int32, error)` | Lowest main-chain height whose block has no cfilter entry; `NotExist` (no message) if none. |
| `InsertMissingCFilters` | `(dbtx RWTx, blockHashes []*chainhash.Hash, filters []*gcs.FilterV2) error` | Algorithm 5.14. |
| `GetMainChainCFilters` | `(dbtx RTx, startHash *chainhash.Hash, inclusive bool, storage []*BlockCFilter) ([]*BlockCFilter, error)` | Fills `storage` with consecutive main-chain filters starting at the height of `startHash` (or the next height). Algorithm 5.14. |
| `GetMainChainBlockHashForHeight` | `(ns RB, height int32) (chainhash.Hash, error)` | Block record hash at `height`; `NotExist` "no block at height %v in main chain". |
| `GetSerializedBlockHeader` | `(ns RB, blockHash *chainhash.Hash) ([]byte, error)` | Copy of the stored header; `NotExist` "block header". |
| `GetBlockHeaderTime` | `(dbtx RTx, blockHash *chainhash.Hash) (int64, error)` | Header timestamp (seconds); `NotExist` "block header". |
| `GetBlockHeader` | `(dbtx RTx, blockHash *chainhash.Hash) (*wire.BlockHeader, error)` | Deserialized header; `NotExist` "block header"; deserialize failure wrapped as `IO`. |
| `BlockInMainChain` | `(dbtx RTx, blockHash *chainhash.Hash) (inMainChain, invalidated bool)` | Header exists, a block record exists at the header's height and its hash equals `blockHash`. `invalidated` is the block record's stake-invalid byte. Missing data yields `(false, false)`. |
| `GetBlockMetaForHash` | `(ns RB, blockHash *chainhash.Hash) (BlockMeta, error)` | Built from the stored header only (does not require main chain membership). `NotExist` "block %v header not found". |
| `GetMainChainBlockHashes` | `(ns RB, startHash *chainhash.Hash, inclusive bool, storage []chainhash.Hash) ([]chainhash.Hash, error)` | Fills `storage` with consecutive main-chain hashes from `startHash`. `NotExist` "starting block %v not found"; `Invalid` "block %v not in main chain". Panics if no block record exists at the header's height (Q-3). |
| `Rollback` | `(dbtx RWTx, height int32) (map[chainhash.Hash][]*wire.MsgTx, error)` | Detaches every main-chain block at `height` and above. Algorithm 5.8. |
| `CFilterV2` (cfilters.go) | `(dbtx RTx, blockHash *chainhash.Hash) ([16]byte, *gcs.FilterV2, error)` | Stored key and a filter parsed from a copy of the stored bytes. `NotExist` "no cfilter saved for block %v". |
| `ImportCFiltersV2` (cfilters.go) | `(dbtx RWTx, startHeight int32, filterData [][]byte) error` | Overwrites cfilters for consecutive main-chain heights from `startHeight`. Algorithm 5.14. |

### 3.6 `Store` methods: recording transactions and ownership

| Method | Signature | Contract |
|---|---|---|
| `InsertMinedTx` | `(dbtx RWTx, rec *TxRecord, blockHash *chainhash.Hash) error` | Algorithm 5.6. |
| `moveMinedTx` (unexported) | `(ns RWB, addrmgrNs RB, rec *TxRecord, recKey, recVal []byte, block *BlockMeta) error` | Algorithm 5.7. |
| `InsertMemPoolTx` (txunmined.go) | `(dbtx RWTx, rec *TxRecord) error` | Algorithm 5.10. |
| `SetPublished` (txunmined.go) | `(dbtx RWTx, txHash *chainhash.Hash, published bool) error` | `published` deletes the unpublished marker; otherwise writes it. Does not check that the transaction exists. |
| `removeDoubleSpends` (unexported, txunmined.go) | `(ns RWB, rec *TxRecord) error` | For each input of `rec` in order, if the unmined-inputs index names a spender, load it and `RemoveUnconfirmed` it. |
| `RemoveUnconfirmed` (txunmined.go) | `(ns RWB, tx *wire.MsgTx, txHash *chainhash.Hash) error` | Algorithm 5.11. |
| `PruneUnmined` (txunmined.go) | `(dbtx RWTx, sdiff dcrutil.Amount) ([]*chainhash.Hash, error)` | Algorithm 5.12. |
| `AddCredit` | `(dbtx RWTx, rec *TxRecord, block *BlockMeta, index uint32, change bool, account uint32) error` | Algorithm 5.5. `block == nil` means unmined. |
| `addCredit` (unexported) | `(ns RWB, rec, block, index, change, account) (bool, error)` | Returns whether a new credit was written. |
| `AddTicketCommitment` | `(ns RWB, rec *TxRecord, index, account uint32) error` | Algorithm 5.9. |
| `RedeemTicketCommitments` | `(ns RWB, rec *TxRecord, block *BlockMeta) error` | Algorithm 5.9. |
| `removeUnspentTicketCommitments`, `replaceTicketCommitmentUnminedSpent` (unexported) | | Algorithm 5.9. |
| `AddMultisigOut` | `(dbtx RWTx, rec *TxRecord, block *BlockMeta, index uint32) error` | Algorithm 5.15. |
| `SpendMultisigOut` | `(ns RWB, op *wire.OutPoint, spendHash chainhash.Hash, spendIndex uint32) error` | Algorithm 5.15. |
| `fetchAccountForPkScript` (unexported) | `(addrmgrNs RB, credVal, unminedCredVal, pkScript []byte) (uint32, error)` | Account of a credit: stored account if present, else address lookup. Algorithm 5.2. |

### 3.7 `Store` methods: UTXO, balance and coin selection

| Method | Signature | Contract |
|---|---|---|
| `UnspentOutputCount` | `(dbtx RTx) int` | Number of keys in the unspent bucket (includes outputs spent by unmined transactions). |
| `randomUTXO` (unexported) | `(dbtx RTx, skip func(k, v []byte) bool) (k, v []byte)` | Algorithm 5.19. |
| `RandomUTXO` | `(dbtx RTx, minConf, syncHeight int32) (*Credit, error)` | `Invalid` if `minConf == 0`. Returns `(nil, nil)` when nothing eligible. Filters only on unmined-spent and confirmations. |
| `UnspentOutputs` | `(dbtx RTx) ([]*Credit, error)` | Algorithm and order 5.18. |
| `UnspentOutput` | `(ns RB, op wire.OutPoint, includeMempool bool) (*Credit, error)` | Unmined credit first when `includeMempool`, else mined unspent; `NotExist` "no unspent output %v". Does not check the unmined-inputs index. |
| `ForEachUnspentOutpoint` | `(dbtx RTx, f func(*wire.OutPoint) error) error` | Same set and order as `UnspentOutputs`, outpoint only (with `Tree`). |
| `IsUnspentOutpoint` | `(dbtx RTx, op *wire.OutPoint) bool` | Buggy and unused; see Q-1. |
| `outputCreditInfo` (unexported) | `(ns RB, op wire.OutPoint, block *Block) (*Credit, error)` | Builds a `Credit`. Algorithm 5.18. |
| `fastCreditPkScriptLookup` (unexported) | `(ns RB, credKey, unminedCredKey []byte) ([]byte, error)` | pkScript of a credit by either key. |
| `MakeInputSource` | `(dbtx RTx, account uint32, minConf, syncHeight int32, ignore func(*wire.OutPoint) bool) InputSource` | Algorithm 5.20. |
| `(*InputSource).SelectInputs` | `(target dcrutil.Amount) (*txauthor.InputDetail, error)` | Calls the closure. |
| `balanceFullScan` (unexported) | `(dbtx RTx, minConf, syncHeight int32) (map[uint32]*Balances, error)` | Algorithm 5.21. |
| `AccountBalance` | `(dbtx RTx, minConf int32, account uint32) (Balances, error)` | `AccountBalances` then lookup; a missing account yields `Balances{Account: account}`. |
| `AccountBalances` | `(dbtx RTx, minConf int32) (map[uint32]*Balances, error)` | `balanceFullScan` at `syncHeight = MainChainTip height`. |

### 3.8 `Store` methods: tickets and multisig queries

| Method | Signature | Contract |
|---|---|---|
| `UnspentTickets` | `(dbtx RTx, syncHeight int32, includeImmature bool) ([]chainhash.Hash, error)` | Algorithm 5.22. |
| `OwnTicket` | `(dbtx RTx, ticketHash *chainhash.Hash) bool` | Key exists in the tickets bucket. |
| `IterateTickets` | `(dbtx RTx) *TicketIterator` | Positions a cursor at the first tickets-bucket key. Algorithm 5.22. |
| `(*TicketIterator).Next`, `Err`, `Close` | | Algorithm 5.22. |
| `GetMultisigOutput` | `(ns RB, op *wire.OutPoint) (*MultisigOut, error)` | `NotExist` "no multisig output for outpoint %v". |
| `UnspentMultisigCreditsForAddress` | `(dbtx RTx, addr stdaddr.Address) ([]*MultisigCredit, error)` | Algorithm 5.15. |

### 3.9 `Store` methods: transaction queries (txquery.go)

| Method | Signature | Contract |
|---|---|---|
| `TxDetails` | `(ns RB, txHash *chainhash.Hash) (*TxDetails, error)` | Unmined record wins; else the newest mined record; `NotExist` with no message. |
| `minedTxDetails`, `unminedTxDetails` (unexported) | | Algorithm 5.16. |
| `TicketDetails` | `(ns RB, txDetails *TxDetails) (*TicketDetails, error)` | `(nil, nil)` if the tx is not a ticket. Algorithm 5.22. |
| `parseTx` (unexported) | `(txHash chainhash.Hash, v []byte) (*wire.MsgTx, error)` | Decodes a record value and returns the `MsgTx`. |
| `Tx` | `(ns RB, txHash *chainhash.Hash) (*wire.MsgTx, error)` | Unmined first, then newest mined; `NotExist` "tx %s not found". |
| `ExistsTx` | `(ns RB, txHash *chainhash.Hash) bool` | Mined or unmined. |
| `ExistsTxMinedOrUnmined` | `(ns RB, txHash *chainhash.Hash) (mined, unmined bool)` | If an unmined record exists returns `(false, true)` without checking mined records. |
| `ExistsUTXO` | `(dbtx RTx, op *wire.OutPoint) bool` | True if the outpoint is in the unspent bucket or is an unmined credit. |
| `UniqueTxDetails` | `(ns RB, txHash *chainhash.Hash, block *Block) (*TxDetails, error)` | Exact incidence; `(nil, nil)` when absent. |
| `TxBlockHeight` | `(dbtx RTx, txHash *chainhash.Hash) (int32, error)` | `-1` if unmined; newest mined record's height; `NotExist` "no transaction %v". |
| `RangeTransactions` | `(ctx, ns RB, begin, end int32, f func([]TxDetails) (bool, error)) error` | Algorithm 5.16. |
| `rangeUnminedTransactions`, `rangeBlockTransactions` (unexported) | | Algorithm 5.16. |
| `PreviousPkScripts` | `(ns RB, rec *TxRecord, block *Block) ([][]byte, error)` | Unused outside disabled tests and broken for `block == nil` (Q-2). |
| `Spender` | `(dbtx RTx, out *wire.OutPoint) (*wire.MsgTx, uint32, error)` | Algorithm 5.23. |
| `RangeBlocks` | `(ns RB, begin, end int32, f func(*Block) (bool, error)) error` | Same range convention as `RangeTransactions`, blocks only. |
| `UnminedTxs` (txunmined.go) | `(dbtx RTx) ([]*TxRecord, error)` | All unmined records in dependency order (5.17). |
| `UnminedTxHashes` (txunmined.go) | `(ns RB) ([]*chainhash.Hash, error)` | Unmined bucket keys in key order. Thin wrapper over the unexported `unminedTxHashes` (`txunmined.go:324`). |
| `unminedTxRecords` (unexported, txunmined.go) | `(ns RB) (map[chainhash.Hash]*TxRecord, error)` | Step 1 of 5.17: every unmined record decoded into a hash-keyed map, `Unpublished` set from the unpublished bucket (`txunmined.go:296-316`). The only place that populates `TxRecord.Unpublished` on read. |

---

## 4. Data formats

The byte layouts of every bucket in `wtxmgr` are owned by `03-txstore-formats.md`. This section fixes only (a) the formats that are defined in `txmined.go`, (b) the names this spec uses for buckets and root keys, and (c) the key/value identities the algorithms depend on. There is no JSON or protobuf in this subsystem.

All integers inside `wtxmgr` keys and values are big-endian (`wallet/udb/txdb.go:123`). Fields read out of a serialized block header are little-endian (wire format).

### 4.1 Birthday state (root key `birthstate`), defined in `txmined.go:334-394`

Value, exactly 45 bytes:

| Offset | Size | Field | Encoding |
|---|---|---|---|
| 0 | 1 | options | bit 0 (`0x01`) = `SetFromHeight`, bit 1 (`0x02`) = `SetFromTime`; other bits written as 0 and ignored on read |
| 1 | 32 | birthday block hash | raw hash bytes (internal order) |
| 33 | 4 | birthday block height | `uint32` big-endian |
| 37 | 8 | birthday time | `uint64` big-endian of `int64` Unix seconds (two's complement; a zero `time.Time` is stored as `0xFFFFFFF1886E0900`) |

Read rule: any stored length other than 45 yields nil (not an error). Time is rebuilt with second resolution; sub-second precision is discarded on write.

### 4.2 Fields read from a stored serialized header (180 bytes)

| Field | Offset | Size | Encoding | Source |
|---|---|---|---|---|
| parent block hash | 4 | 32 | raw | `txmined.go:545` |
| merkle root | 36 | 32 | raw | `txmined.go:550` |
| vote bits | 100 | 2 | `uint16` little-endian | `txmined.go:563` |
| height | 128 | 4 | `uint32` little-endian, cast to `int32` | `txmined.go:568` |
| timestamp | 136 | 4 | `uint32` little-endian (Unix seconds) | `txmined.go:581` |

The cfilter key of a block is the first 16 bytes of its merkle root (`dcrd/gcs/v4@v4.1.1/blockcf2/blockcf.go:96`).

### 4.3 Bucket and root-key names used below

| Name in this spec | Bucket / key | Key | Value (summary) |
|---|---|---|---|
| headers | `h` | block hash (32) | serialized header |
| blocks | `b` | height `uint32` BE | hash, time, vote bits, stake-invalid byte (offset 42), tx count, tx hashes |
| tx records | `t` | tx hash ‖ height ‖ block hash (68) | received time (8) ‖ serialized tx |
| credits | `c` | tx hash ‖ height ‖ block hash ‖ output index (72) | amount, flags, spender debit key, script type/offset/length, account (94) |
| unspent | `u` | tx hash ‖ output index (36, "canonical outpoint") | height ‖ block hash (36) |
| debits | `d` | tx hash ‖ height ‖ block hash ‖ input index (72) | amount ‖ credit key (80) |
| unmined | `m` | tx hash (32) | received time (8) ‖ serialized tx |
| unpublished | `up` | tx hash (32) | `0x01` |
| unmined credits | `mc` | canonical outpoint (36) | amount, flags, script type/offset/length, account (22; legacy 9) |
| unmined inputs | `mi` | canonical outpoint of the previous output (36) | hash of the unmined spender (32) |
| tickets | `tix` | ticket hash (32) | picked height `int32` BE (always `-1` = `0xFFFFFFFF` when written by this code) |
| multisig | `ms` | canonical outpoint (36) | 135-byte multisig record |
| multisig unspent | `mu` | canonical outpoint (36) | `0x00` |
| invalidated credits | `ic` | credit key (72) | credit value |
| invalidated debits | `id` | debit key (72) | debit value |
| cfilters | `cf` | block hash (32) | 16-byte key ‖ filter bytes |
| ticket commitments | `cmt` | canonical outpoint (36) | amount (8) ‖ account (4) |
| unspent ticket commitments | `cmu` | canonical outpoint (36) | flags byte, bit 0 = unminedSpent |
| mined balance | root key `bal` | | `uint64` BE atoms |
| tip | root key `tip` | | block hash |
| have-cfilters flag | root key `havecfilters` | | 1 byte |
| processed-txs marker | root key `lasttxsblock` | | block hash |
| birthday state | root key `birthstate` | | 4.1 |

### 4.4 Key/value identities the algorithms rely on

1. **Credit key from an unspent entry.** For unspent key `k` (36 bytes) and value `v` (36 bytes): `credKey = k[0:32] ‖ v[0:36] ‖ k[32:36]` (72 bytes). Implemented three times: `existsRawUnspent` (`txdb.go:1143`), `MakeInputSource` (`txmined.go:3194-3198`), `balanceFullScan` (`txmined.go:3369-3373`).
2. **Unspent value from a credit key.** `unspentVal = credKey[32:68]` (`txdb.go:875`).
3. **Unspent value from a debit value.** `unspentVal = debitVal[40:76]`, which is bytes 32..68 of the embedded credit key (`txdb.go:1240`).
4. **Tx record key from a credit or debit key.** First 68 bytes.
5. **Ticket commitment key and multisig key** are the canonical outpoint; `RemoveUnconfirmed` relies on this identity (`txunmined.go:218-221`).
6. **Stake opcode tag in the credit flags byte.** Stored as `(opcode - 0xb9) << 2` for `OP_NOP10..OP_SSTXCHANGE` and as `0x1c` for `OP_TGEN`; read back as `OP_TGEN` when `flags & 0x1c == 0x1c`, else `((flags >> 2) & 7) + 0xb9` (`txdb.go:743-766`). Tag values this subsystem compares against: `0xb9` nonstake, `0xba` `OP_SSTX`, `0xbb` `OP_SSGEN`, `0xbc` `OP_SSRTX`, `0xbd` `OP_SSTXCHANGE`, `0xc3` `OP_TGEN`. `OP_TADD` (`0xc1`) is never stored and never read back (Q-17).

### 4.5 Values this subsystem constructs

| Where | Value written | Field sources |
|---|---|---|
| `addCredit` mined | credit value (94 bytes) | amount = output value; change = argument; opcode = `getStakeOpCode`; isCoinbase = `IsEitherCoinBaseTx(tx)`; hasExpiry = `tx.Expiry != 0`; script type = `pkScriptType`; script offset = `MsgTx.PkScriptLocs()[index]`; script length = `len(pkScript)`; account = argument; account-exists bit set; spent bit clear |
| `addCredit` unmined | unmined credit value (22 bytes) | same sources |
| `AddCredit` invalidated | credit value into `ic` | same sources |
| `moveMinedTx` | credit value | amount, change, opcode, isCoinbase copied from the unmined credit; **hasExpiry always written false** (Q-6); script type/offset/length copied from the unmined credit (0 for legacy 9-byte values); account from `fetchAccountForPkScript` |
| `Rollback` | unmined credit value | amount, change, opcode, isCoinbase, hasExpiry copied from the mined credit; script type/offset/length recomputed from the transaction; account from `fetchAccountForPkScript` |
| `InsertMinedTx`, `moveMinedTx` | debit value (80 bytes) | amount = credit amount returned by `spendCredit`; credit key |
| `ExtendMainChain` | block record (47 bytes, no transactions) | hash, header timestamp widened to 8 bytes, header vote bits, invalid byte 0, count 0 |
| `ExtendMainChain`, `InsertMissingCFilters`, `ImportCFiltersV2` | cfilter value | 16-byte key ‖ `FilterV2.Bytes()`. `ImportCFiltersV2` has no `FilterV2`: it stores the caller's blob verbatim after the key (`cfilters.go:42-63`); see 5.14. |
| `InsertMinedTx`, `InsertMemPoolTx` | ticket record | `0xFFFFFFFF` |
| `AddTicketCommitment` | commitment value, unspent-commitment value | amount from `AmountFromSStxPkScrCommitment`, account; flags `0x00` |
| `AddMultisigOut` | multisig value (135 bytes) | script hash, m, n, spent=false, tree, block hash/height (zero/0 when unmined), amount, spentBy = zero hash, spentByIndex = `0xFFFFFFFF`, tx hash |

---

## 5. Algorithms and state machines

Conventions for this section:

- Every algorithm runs inside the single database transaction the caller passed. There are no nested transactions, retries, backoff or timeouts in this subsystem. A returned error leaves partial writes in the transaction; the caller is expected to abort it (the `walletdb.Update` wrapper rolls back on error).
- There are no locks in this subsystem (section 7).
- "IO error" means an `*errors.Error` with kind `errors.IO`; "Invalid" kind `errors.Invalid`; and so on (section 8). Bucket `Put`/`Delete` failures are wrapped as `IO` unless stated.
- `%v` of a hash prints the byte-reversed hex string; `%v` of an outpoint prints `hash:index`.

### 5.1 Cursor semantics relied upon

The store depends on these bbolt behaviours (`bbolt@v1.3.12/cursor.go`). A port on any other storage layout must reproduce them exactly because they determine iteration orders and the distribution of random selection.

1. Keys within a bucket iterate in ascending lexicographic byte order. Hashes are stored in internal byte order (not the reversed display order), so "ascending by tx hash" means ascending by raw hash bytes.
2. `Seek(k)` positions at the first key `>= k` and returns it; returns nil if no such key exists.
3. `Prev()` immediately after a `Seek(k)` returns the greatest key `< k'` where `k'` is the key `Seek` returned; if `Seek` returned nil (seek key beyond the last key), `Prev()` returns the last key in the bucket (`bbolt@v1.3.12/cursor.go:117-136`, `248-277`). `Prev()` from the first key returns nil.
4. `Next()` from the last key returns nil.
5. `Bucket.Get(nil)` returns nil.
6. `KeyN()` is the number of keys in the bucket (bbolt `Stats().KeyN`, `wallet/internal/bdb/db.go:190-192`, a full bucket walk). The walk visits pages only (`bbolt@v1.3.12/bucket.go:420-511`), not the in-memory nodes of a write transaction, so the count is exact in a read-only transaction but does not reflect puts and deletes made to that bucket earlier in the same read-write transaction. The wallet's `MakeInputSource` call sites all use read-only transactions (`wallet/createtx.go:97-112`, `404-409`, `1033-1035`, `wallet/utxos.go:28-42`).
7. Cursor `Close()` is a no-op (`wallet/internal/bdb/db.go:262`); the Go code's cursor-leak paths have no observable effect.
8. Values returned by `Get`/cursors alias database memory and are only valid until the bucket is modified. The Go code mostly copies before mutating; a port must always copy.

### 5.2 Account resolution for a credit: `fetchAccountForPkScript` (`txmined.go:1015-1051`)

Inputs: mined credit value or nil, unmined credit value or nil, pkScript.

1. If the mined credit value is non-nil, has length `>= 94` and its account-exists bit (`0x80` of byte 81) is set, return the `uint32` BE at `[90:94]`.
2. Else if the unmined credit value is non-nil, has length `>= 22` and its account-exists bit (`0x80` of byte 9) is set, return the `uint32` BE at `[18:22]`.
3. Else decode addresses from the pkScript with `stdscript.ExtractAddrs(0, pkScript, params)`. If none: return a plain standard-library error (`errors.New`, `errors/errors.go:153-155`; not an `*errors.Error`, so it carries no kind and matches no kind) with text `no addresses decoded from pkScript`.
4. Return `acctLookupFunc(addrmgrNs, addrs[0])`, that is `Manager.AddrAccount` for the **first** address only. A missing address yields `NotExist` "no address %v".

Errors from steps 1 and 2 are swallowed. Step 3/4 is only reached for credits written before accounts were stored (legacy values).

### 5.3 `ExtendMainChain` (`txmined.go:203-272`)

Inputs: `header`, `blockHash` (not verified to be the hash of `header`), filter `f` (must be non-nil; a nil filter panics at step 9).

1. `height = int32(header.Height)`. If `height < 1`: Invalid "block 0 cannot be added".
2. Read `havecfilters`. If it is not exactly the single byte `0x01`: Invalid "main chain may not be extended without first saving all previous cfilters". (Note: this test requires the value `1`; `IsMissingMainChainCFilters` accepts any non-zero byte.)
3. Read `tip`. If `header.PrevBlock != tip`:
   a. Read the block record at `height`. If it exists and its hash equals `blockHash`: Exist "block already recorded in main chain".
   b. Otherwise: Invalid "not direct child of current tip".
4. Read the tip header; `tipHeight` = its height. If `tipHeight + 1 != height`: Invalid "invalid height for next block".
5. If `header.VoteBits & 1 != 0` run `stakeValidate(ns, tipHeight)`, else `stakeInvalidate(ns, tipHeight)` (5.4). Return its error.
6. Serialize `header` (180 bytes) and put it in headers under `blockHash`. A serialization error is returned unwrapped (cannot occur with an in-memory buffer); the `Put` error is wrapped as IO.
7. Put `tip = blockHash`.
8. Put an empty block record at `height` (hash, header timestamp, header vote bits, invalid byte 0, zero transactions).
9. Put the cfilter under `blockHash`: key = first 16 bytes of `header.MerkleRoot`, data = `f.Bytes()`. This `Put` error is returned unwrapped.

Nothing is written before step 5 except what `stakeValidate`/`stakeInvalidate` write.

### 5.4 Stake validation and invalidation of the parent block (`txmined.go:631-885`)

Both operate on the main-chain block record at `height` (always the previous tip).

Common preamble:

1. Read the block record at `height`. If missing: IO "missing block record for height %v".
2. `stakeValidate` returns nil immediately if the record's invalid byte is 0. `stakeInvalidate` returns nil immediately if it is non-zero.
3. Read the mined balance (IO "mined balance len %d" if the root value is not 8 bytes).
4. Decode the block record, then rewrite it with the invalid byte set to 0 (validate) or 1 (invalidate).
5. For each transaction hash of the block record, in record order:
   a. Read the tx record for (hash, block). Missing: IO "missing transaction record for tx %v block %v".
   b. Decode it. If its `TxType` is not `TxTypeRegular`, skip it (stake-tree transactions are never invalidated).
   c. Run the credit loop, then the debit loop below.
6. Write the mined balance.

`stakeInvalidate`, credit loop, for each output index `i` ascending:

1. If no credit exists in credits for (tx, i, block): continue.
2. Delete it from credits; put the same key/value into invalidated credits.
3. Delete the unspent entry for (tx, i) (delete of a missing key is not an error).
4. `minedBalance -= output.Value` (unconditional: also when the credit had already been spent by a mined transaction and therefore had no unspent entry; see Q-12).

`stakeInvalidate`, debit loop, for each input index `i` ascending:

1. Look up the debit for (tx, i, block) in debits. Short value: IO "debit len %d". If absent: continue.
2. `debitAmount` = first 8 bytes of the debit value.
3. `unspendRawCredit(credKey)`: if the credit still exists in credits, rewrite it as a 94-byte value with the spent bit cleared (the stale spender key bytes are left in place); if it does not exist, do nothing.
4. Delete the debit; put the same key/value into invalidated debits.
5. Put an unspent entry for the previous outpoint with value `debitVal[40:76]` (unconditional, even if step 3 found no credit).
6. `minedBalance += debitAmount`.

`stakeValidate`, credit loop, for each output index `i` ascending:

1. If no entry exists in invalidated credits for (tx, i, block): continue.
2. Delete it from invalidated credits; put the same key/value into credits.
3. Put an unspent entry for (tx, i) with the block (unconditional, even for a credit whose spent bit is set: a stake transaction's live debit of it leaves it so, QK-0785).
4. `minedBalance += output.Value`.

`stakeValidate`, debit loop, for each input index `i` ascending:

1. Look up the debit in invalidated debits (short value: IO "debit len %d"). If absent: continue.
2. `debitAmount` = first 8 bytes.
3. `spendCredit(credKey, spender = (tx, block, i))`: read the credit from credits, copy it into a zeroed 94-byte buffer, set the spent bit, write the 72-byte spender debit key at `[9:81]`, put it. If the credit is missing a 94-byte value with only the spent bit and spender key is created (amount 0).
4. Delete the unspent entry for the previous outpoint.
5. Delete the entry from invalidated debits; put the same key/value into debits.
6. `minedBalance -= debitAmount`.

Neither function special-cases ticket (`OP_SSTX`) amounts; this is harmless because only regular-tree transactions are processed and their outputs are never ticket submissions, but a regular transaction that spends a wallet credit tagged `OP_SSTX` cannot exist either.

### 5.5 `AddCredit` / `addCredit` (`txmined.go:1384-1558`)

Opcode tag (`getStakeOpCode`), from `stdscript.DetermineScriptType(version, pkScript)`:

| Script type | Tag |
|---|---|
| `STStakeSubmissionPubKeyHash`, `STStakeSubmissionScriptHash` | `OP_SSTX` |
| `STStakeGenPubKeyHash`, `STStakeGenScriptHash` | `OP_SSGEN` |
| `STStakeRevocationPubKeyHash`, `STStakeRevocationScriptHash` | `OP_SSRTX` |
| `STStakeChangePubKeyHash`, `STStakeChangeScriptHash` | `OP_SSTXCHANGE` |
| `STTreasuryGenPubKeyHash`, `STTreasuryGenScriptHash` | `OP_TGEN` |
| anything else (including treasury add) | `opNonstake` (`OP_NOP10`) |

Stored script type (`pkScriptType`):

| Script type | Stored value |
|---|---|
| `STPubKeyHashEcdsaSecp256k1` | `scriptTypeP2PKH` (2) |
| `STPubKeyEcdsaSecp256k1` | `scriptTypeP2PK` (3) |
| `STScriptHash` | `scriptTypeP2SH` (6) |
| `STPubKeyHashEd25519`, `STPubKeyHashSchnorrSecp256k1` | `scriptTypeP2PKHAlt` (4) |
| `STPubKeyEd25519`, `STPubKeySchnorrSecp256k1` | `scriptTypeP2PKAlt` (5) |
| stake/treasury-gen pubkey-hash forms | `scriptTypeSP2PKH` (7) |
| stake/treasury-gen script-hash forms | `scriptTypeSP2SH` (8) |
| anything else | `scriptTypeUnspecified` (1) |

`AddCredit(dbtx, rec, block, index, change, account)`:

1. If `index >= len(rec.MsgTx.TxOut)`: Invalid "transaction output index for credit does not exist".
2. `invalidated = false`. If `rec.TxType == TxTypeRegular` and `block != nil`: read the header for `block.Hash` (a missing header panics), take its height, read the block record at that height (a missing record panics) and set `invalidated` to its invalid byte.
3. If `invalidated`: build a credit value (4.5; `hasExpiry = rec.MsgTx.Expiry != 0`) and put it into invalidated credits under the credit key, **overwriting** any existing entry. No unspent entry, no balance change, no existence check against credits. Return.
4. Otherwise call `addCredit`.

`addCredit`:

1. Compute opcode tag, `isCoinbase = IsEitherCoinBaseTx(tx)`, `hasExpiry = tx.Expiry != 0`.
2. **Unmined (`block == nil`)**:
   a. If no unmined record exists for `rec.Hash`: **panic** with `attempted to add credit for unmined tx, but unmined tx with same hash does not exist`.
   b. If an unmined credit already exists for (hash, index): return `(false, nil)`.
   c. Put the unmined credit value. Return `(true, err)`.
3. **Mined**:
   a. If a credit already exists in credits for (hash, index, block): return `(false, nil)`.
   b. Log at debug: `Marking transaction %v output %d (%v) spendable` (hash, index, amount formatted as `dcrutil.Amount`).
   c. Put the credit value.
   d. Read the mined balance. If the opcode tag is not `OP_SSTX`, write `balance + amount`.
   e. Put the unspent entry for (hash, index) → block. Return `(true, err)`.

`IsEitherCoinBaseTx` is true for any transaction with exactly one input whose previous outpoint is (zero hash, index `0xFFFFFFFF`) under the pre-treasury rule (`dcrd/blockchain/standalone/v2@v2.3.0/tx.go:49-102`), which also matches treasurybase and treasury-spend transactions. Their credits are therefore flagged coinbase.

### 5.6 `InsertMinedTx` (`txmined.go:1213-1375`)

1. Read the header for `blockHash`. Missing: Invalid "block header must be recorded".
2. `height` = header height. Read the block record at `height` (**panics** if no block record exists at that height, Q-3). If its hash differs from `blockHash`: Invalid "mined transactions must be added to main chain blocks".
3. Read the mined balance.
4. `block = blockMetaFromHeader(blockHash, header)`. `txType = stake.DetermineTxType(&rec.MsgTx)` (recomputed; `rec.TxType` is not consulted here).
5. `invalidated` = (txType is regular) AND the block record's invalid byte is non-zero.
6. For each input `i` ascending:
   a. Look up the previous outpoint in unspent. If absent: continue (no debit is recorded for inputs that do not spend a mined, unspent wallet credit).
   b. If `invalidated`: read the credit, take its amount (IO "credit len %d" if shorter than 9 bytes), and put a debit value into invalidated debits under the debit key (tx, i, block). Do not touch the credit, the unspent entry or the balance.
   c. Else: `spendCredit` (5.4 step 3 of validate), put the debit into debits, subtract the amount from the mined balance **unless** `(txType == SSGen && i == 1) || (txType == SSRtx && i == 0)`, then delete the unspent entry.
7. Write the mined balance (always).
8. If a tx record already exists for (hash, block): return nil. (The input loop above has already run; it is a no-op on a true duplicate because the spent credits are no longer in unspent.)
9. If `txType == SStx` and the tickets bucket has no entry for the hash: put the ticket record (`0xFFFFFFFF`).
10. If an unmined record exists for `rec.Hash`:
    a. If `invalidated`: **panic** with `unimplemented: moveMinedTx called on a stake-invalidated tx: block %v height %v tx %v`.
    b. Return `moveMinedTx(ns, addrmgrNs, rec, txRecordKey, unminedValue, &block)` (5.7).
11. `removeDoubleSpends(ns, rec)`: for each input in order, if the unmined-inputs index names a spender of that previous outpoint, decode that unmined record (IO "tx record len 0" if it is missing), log at debug `Removing double spending transaction %v`, and `RemoveUnconfirmed` it (5.11).
12. Append `rec.Hash` to the block record at `block.Height` (count incremented) and put it.
13. Put the tx record for (hash, block): received time = `rec.Received.Unix()`, transaction bytes = `rec.SerializedTx` if non-nil, else a fresh serialization.

### 5.7 `moveMinedTx` (`txmined.go:1055-1207`)

Called only from 5.6 step 10. `recVal` is the **unmined** record value, reused verbatim as the mined value, so the mined record keeps the received time and bytes of the unmined record, not those of the `rec` argument.

1. Log at debug: `Marking unconfirmed transaction %v mined in block %d`.
2. Append `rec.Hash` to the block record at `block.Height`; put it.
3. Put the tx record: key (hash, block), value `recVal`.
4. Read the mined balance.
5. For each input `i` ascending:
   a. Compute the canonical outpoint key and look up unspent.
   b. Delete the unmined-inputs entry for that key (unconditional).
   c. If no unspent entry: continue.
   d. `spendCredit`; read the credit back (missing: IO "missing credit %v, key %x, spent by %v"); if its opcode tag is not `OP_SSTX` subtract the amount from the balance; delete the unspent entry; put the debit.
   Steps c-d are unreachable in practice because 5.6 step 6 already consumed every unspent entry this transaction spends; a port must still implement step b.
6. For each output `i` ascending for which an unmined credit exists:
   a. Read amount and change (IO "unmined credit len %d" if shorter than 9 bytes), opcode tag, isCoinbase.
   b. Read script type, offset and length from the unmined credit (all 0 for a legacy 9-byte value).
   c. Fetch the pkScript from the record value: if offset is 0, deserialize the whole transaction and index the output (IO "missing transaction output for credit index" if out of range); else copy `recVal[8+offset : 8+offset+length]` (IO "invalid script offset %d for tx %v" if out of bounds).
   d. `acct = fetchAccountForPkScript(addrmgrNs, nil, unminedCredVal, pkScript)` (5.2); return its error.
   e. Delete the unmined credit.
   f. Put the mined credit (4.5; hasExpiry false) and the unspent entry.
   g. If the opcode tag is not `OP_SSTX`: `balance += amount`.
7. Write the mined balance.
8. Delete the unpublished marker for the hash.
9. Delete the unmined record.

Moved credits are added to unspent even when another unmined transaction already spends them (that spender stays recorded in unmined inputs).

### 5.8 `Rollback` (`txmined.go:1874-2240`)

Input: `height`, the lowest height to detach.

1. If `height == 0`: Invalid "cannot rollback the genesis block".
2. Read the mined balance. Initialise `coinBaseCredits` (list of outpoints), `heightsToRemove`, `removedTxs` (map block hash → list of transactions).
3. Iterate block records from the highest key downward (seek `0xFFFFFFFF`, then `Prev`). Stop at the first block with `Height < height`. For each block `b`:
   1. Append `b.Height` to `heightsToRemove`. Log at debug `Rolling back transactions from block %v height %d`. Create an empty per-block cache `removedCredits` (credit key → credit value copy).
   2. For each transaction hash in the block record, in record order:
      1. Read and decode the tx record (missing: IO "tx record len 0"). Delete the tx record. Append a copy of the `MsgTx` to `removedTxs[b.Hash]`.
      2. **If `IsEitherCoinBaseTx(tx)`** (coinbase, treasurybase, treasury spend): for each output `i` ascending with a credit in credits:
         - append `(hash, i, tree regular)` to `coinBaseCredits`;
         - if an unspent entry exists for (hash, i): `balance -= output.Value` and delete it;
         - cache the credit value in `removedCredits`; delete the credit;
         - if a multisig record exists for (hash, i): zero its block hash and height and rewrite it.
         Then continue with the next transaction. The transaction is **not** moved to unmined.
      3. Otherwise put the record value under the tx hash in unmined (same bytes, same received time).
      4. For each input `i` ascending, skipping `i == 0` when the tx is a vote (`rec.TxType == SSGen`):
         - put unmined inputs: previous outpoint → `rec.Hash` (for every input, credit or not);
         - look up the debit (tx, i, block) in debits; if absent continue;
         - find the credit value in credits, else in `removedCredits`; if neither: IO "missing credit %v, key %x, spent by %v";
         - `amt = unspendRawCredit(credKey)` (0 when the credit no longer exists in credits);
         - delete the debit;
         - if `amt == 0` continue (this also skips zero-value credits);
         - if the credit's opcode tag is not `OP_SSTX`: `balance += amt`;
         - put unspent: previous outpoint → `credKey[32:68]`;
         - if a multisig record exists for the previous outpoint: mark it unspent (5.15), rewrite it and add it to multisig unspent.
      5. For each output `i` ascending with a credit in credits:
         - cache a copy of the credit value in `removedCredits`;
         - read amount, change, opcode tag, isCoinbase, hasExpiry from the credit;
         - recompute script type, offset (`PkScriptLocs()[i]`) and length from the transaction;
         - `acct = fetchAccountForPkScript(addrmgrNs, creditVal, nil, output.PkScript)`; return its error;
         - put the unmined credit (hash, i); delete the mined credit;
         - if an unspent entry exists for (hash, i): unless `rec.TxType == SStx && i == 0`, `balance -= output.Value`; delete the unspent entry;
         - if a multisig record exists for (hash, i): zero its block hash and height and rewrite it.
      6. If the tx is a vote or revocation: `replaceTicketCommitmentUnminedSpent(ns, type, tx, true)` (5.9), which re-creates the unspent-commitment entries with the unminedSpent flag set.
   3. Re-seek the cursor to the current height key (no functional effect other than keeping the cursor valid).
4. Return the iterator's error if any (IO, from a malformed block record).
5. Delete every block record in `heightsToRemove`.
6. For each outpoint in `coinBaseCredits`, in collection order: if unmined inputs names a spender, decode that unmined record (IO "tx record len 0" if missing), log at debug `Transaction %v spends a removed coinbase output -- removing as well`, and `RemoveUnconfirmed` it.
7. Write the mined balance.
8. Read the block record at `height - 1` (panics if absent) and put `tip` = its hash.
9. Return `removedTxs`.

Shape of the result (`txmined.go:1901`, `1936-1937`): a map keyed by detached block hash. A key is present only for detached blocks whose block record listed at least one wallet transaction; detached blocks without wallet transactions have **no** entry (not an empty list). Each list is in block-record (insertion) order and includes the coinbase-like transactions that were dropped rather than moved to unmined. Every element is a pointer to a separate by-value copy of the decoded `MsgTx`. The map is allocated before any work, so it is non-nil (possibly empty) on success and nil on every error return. The caller (`wallet/chainntfns.go:160`) receives it inside the same database transaction.

Order of side effects that a port must keep because later steps read what earlier steps wrote: (a) blocks are processed from the highest height down; (b) within a block, transactions in record order, and for each transaction the tx record is deleted first, then inputs, then outputs, then the commitment flags; (c) block records are deleted only after the whole walk; (d) coinbase-spender eviction runs after the block-record deletes and before the single `bal` write; (e) `tip` is written last. `removedCredits` is scoped to one block. That suffices in a consistent database: a spender is never mined below its funding transaction, and higher blocks are processed first, so a credit funded in a lower block is still in credits when its spender's block is processed; the cache only serves a spender that follows its funding transaction in the same block record.

What `Rollback` does not do: it never deletes headers or cfilters of detached blocks; it never touches the tickets bucket; it never touches the invalidated credits/debits buckets (Q-5); it does not restore the unpublished marker; it does not stake-validate the new tip (the next `ExtendMainChain` does); it does not update `lasttxsblock`.

### 5.9 Ticket commitments (`txmined.go:1560-1710`)

`AddTicketCommitment(ns, rec, index, account)`:

1. If a commitment entry already exists for (rec.Hash, index): return nil. (Checked first, before any validation.)
2. If `index >= len(TxOut)`: Invalid "index should be of an existing output".
3. If `index % 2 != 1`: Invalid "index should be of a ticket commitment".
4. If `rec.TxType != SStx`: Invalid "transaction record should be of a ticket".
5. Amount = `stake.AmountFromSStxPkScrCommitment(pkScript)`: little-endian `uint64` at script bytes `[22:30]` with the top bit of byte 29 cleared; a script shorter than 32 bytes returns the stake rule error unwrapped.
6. Log at debug `Accounting for ticket commitment %v:%d (%v) from the wallet`.
7. Put the commitment (amount, account) and the unspent-commitment entry with flags 0.

`originalTicketInfo(type, tx)`:

- Vote: ticket hash = `TxIn[1].PreviousOutPoint.Hash`; `ticketOutCount = (len(TxOut) - 2) * 2 + 1`.
- Revocation: ticket hash = `TxIn[0].PreviousOutPoint.Hash`; `ticketOutCount = len(TxOut) * 2 + 1`.

The count is an upper bound, not exact: a vote carrying treasury-vote or other trailing outputs overestimates it. This is harmless because every candidate index is checked against the commitments bucket.

`replaceTicketCommitmentUnminedSpent(ns, type, tx, value)`: for `i = 1, 3, 5, ... < ticketOutCount`, if a commitment exists for (ticketHash, i): log at debug `Marking ticket commitment %v:%d unmined spent as %v` and **put** the unspent-commitment entry with flags = `value` (creating the entry if it had been deleted).

`removeUnspentTicketCommitments(ns, type, tx)`: same loop; log at debug `Removing unspent ticket commitment %v:%d from the wallet` and delete the unspent-commitment entry. The commitment entry itself is kept forever (except 5.11).

`RedeemTicketCommitments(ns, rec, block)`:

1. If `rec.TxType` is neither SSGen nor SSRtx: Invalid "rec must be a vote or revocation tx".
2. If `block == nil`: `replaceTicketCommitmentUnminedSpent(..., true)`.
3. Else: `removeUnspentTicketCommitments(...)`.

State machine of an unspent-commitment entry:

| Event | Effect on `cmu` entry |
|---|---|
| `AddTicketCommitment` (first sight of ticket) | created, flag 0 |
| unmined vote/revocation inserted, wallet calls `RedeemTicketCommitments(block=nil)` | flag 1 |
| that unmined vote/revocation removed (`RemoveUnconfirmed`) | flag 0 (entry re-created if absent) |
| vote/revocation mined, wallet calls `RedeemTicketCommitments(block)` | deleted |
| mined vote/revocation rolled back | re-created with flag 1 |
| unmined ticket removed (`RemoveUnconfirmed`) | deleted together with the `cmt` entry |

### 5.10 `InsertMemPoolTx` (`txunmined.go:23-134`)

1. If any mined tx record exists for `rec.Hash` (newest-record lookup): Exist "transaction already exists mined".
2. If an unmined record already exists: return nil. Nothing is updated, including the unpublished marker.
3. For each input in order, if unmined inputs names a spender `S` of the previous outpoint:
   a. If `rec.TxType` (the caller-supplied field) is SSGen or SSRtx:
      - load and deserialize `S` (a missing record panics; a deserialize failure is IO);
      - if `S` is not a vote: fall through to step 3b;
      - if the block `S` votes on equals the current tip hash: DoubleSpend "vote or revocation %v double spends unmined vote %v on the tip block";
      - otherwise `RemoveUnconfirmed(S)` and continue with the next input.
   b. DoubleSpend "%v conflicts with %v by double spending %v" (new hash, spender hash, outpoint).
4. Log at info: `Inserting unconfirmed transaction %v`.
5. Put the unmined record (received time ‖ serialized tx). If `rec.Unpublished`, put the unpublished marker.
6. `txType = stake.DetermineTxType(&rec.MsgTx)`. For each input in order, except input 0 of a vote: put unmined inputs: previous outpoint → `rec.Hash` (overwrites silently).
7. If `txType == SStx` and the tickets bucket has no entry: put the ticket record (`0xFFFFFFFF`).

No credits are created here. Unlike a mined insert, an unmined double spend never evicts the existing unmined transaction (except the stale-vote case in step 3a).

### 5.11 `RemoveUnconfirmed` (`txunmined.go:181-280`)

Inputs: the transaction and its hash. Recursive; depth equals the length of the unmined spend chain.

1. `stxType = stake.DetermineTxType(tx)`.
2. For each output index `i` ascending:
   a. If unmined inputs names a spender of (txHash, i): decode that unmined record (IO "tx record len 0" if missing), log at debug `Transaction %v is part of a removed conflict chain -- removing as well`, and recurse.
   b. Delete the unmined credit (txHash, i).
   c. If `stxType == SStx` and `i` is odd and a commitment exists for (txHash, i): log at debug `Removing unconfirmed ticket commitment %s:%d`, delete the commitment and the unspent-commitment entry.
3. If `stxType` is SSGen or SSRtx: `replaceTicketCommitmentUnminedSpent(ns, stxType, tx, false)` (5.9).
4. For each input in order:
   a. Delete the unmined-inputs entry for the previous outpoint (including a vote's stakebase key, which never exists).
   b. If a multisig record exists for the previous outpoint: mark it unspent, rewrite it and add it to multisig unspent.
5. Delete the unpublished marker.
6. Delete the unmined record.

The tickets-bucket entry of a removed ticket is left in place; ticket iteration tolerates it (5.22). The function does not verify that the unmined record exists; it is idempotent with respect to deletes.

### 5.12 `PruneUnmined` (`txunmined.go:344-410`)

Input: `sdiff`, the stake difficulty of the next block (0 disables the ticket-price rule).

1. `(tipHash, tipHeight) = MainChainTip`.
2. Walk the unmined bucket in key order. For each record deserialize the transaction (IO on failure) and classify with the first matching rule:
   1. `tx.Expiry != 0 && tx.Expiry <= uint32(tipHeight)` → expired, remove.
   2. `stake.IsSStx(tx)` → keep if `sdiff == 0 || tx.TxOut[0].Value == int64(sdiff)`, else remove (old ticket price).
   3. `stake.IsSSGen(tx)` → keep if the voted-on block hash equals `tipHash`, else remove.
   4. anything else → keep.
3. For each record to remove, log at info one of `Removing expired unmined transaction %v`, `Removing old unmined ticket purchase %v`, `Removing missed or invalid vote %v`, and queue (tx, hash).
4. After the walk, for each queued entry in key order: `RemoveUnconfirmed`; on error return the hashes removed so far together with the error; else append the hash.
5. Return the list of removed hashes (non-nil, possibly empty).

Descendants removed recursively by step 4 are not in the returned list unless they were queued themselves. Revocations are only removed by the expiry rule.

Further details (`txunmined.go:355-409`):

- An error during the walk of step 2 returns a **nil** slice with the error; only a failure in step 4 returns the partial (non-nil) list.
- The hash of a record to remove is built from the bucket key with `chainhash.NewHash(k)`; a key that is not exactly 32 bytes is IO wrapping the chainhash error `invalid hash length of %v, want %v` (key length, 32) (`dcrd/chaincfg/chainhash@v1.0.5/hash.go:75-82`). The length is checked only for records that are going to be removed; kept records are never length-checked here.
- The ticket rule reads `tx.TxOut[0]` after `stake.IsSStx` has accepted the transaction, and the vote rule ignores the error of `SSGenBlockVotedOn` (it cannot fail once `IsSSGen` is true).
- The transaction passed to `RemoveUnconfirmed` is the copy deserialized during the walk, not a re-read; a queued record that an earlier removal in step 4 already deleted as a descendant is "removed" a second time, which only repeats idempotent deletes and re-puts (commitment flags, multisig unspent markers), and its hash is still appended to the result.

### 5.13 `UpdateProcessedTxsBlockMarker` (`txmined.go:289-321`)

1. `prev` = current marker hash.
2. While `prev` is not in the main chain: load its header (error `NotExist` "block header" if unknown, which includes an unset all-zero marker) and set `prev` to its parent hash.
3. Load the header of `prev`.
4. If `hash` is not in the main chain: Invalid "%v is not a main chain block".
5. Load the header of `hash`.
6. If `header(hash).Height > header(prev).Height`: put `lasttxsblock = hash`. Otherwise leave it unchanged.

### 5.14 Compact filter maintenance

`MissingCFiltersHeight` (`txmined.go:408-421`): walk blocks ascending by height; return the height of the first block whose hash has no cfilter entry. A malformed (shorter than 16 bytes) entry is treated as present. If every block has one: `NotExist` with no message.

`InsertMissingCFilters` (`txmined.go:430-490`):

1. If `havecfilters` is one non-zero byte: Invalid "all cfilters for main chain blocks are already recorded".
2. If the slice lengths differ: Invalid "slices must have equal len".
3. If empty: return nil.
4. For each `i`:
   a. If `i == 0` and the hash is the network's genesis hash: accept, with an **all-zero 16-byte key** (Q-15).
   b. Otherwise the header must exist (NotExist "missing header for block %v"); key = first 16 bytes of its merkle root; for `i == 0` a cfilter must already exist for the parent hash; for `i > 0` the parent hash must equal `blockHashes[i-1]`.
   c. If the check fails: Invalid "block hashes are not ordered or previous cfilters are missing".
   d. Put the cfilter (unwrapped `Put` error).
5. If the last hash equals the main chain tip: put `havecfilters = 0x01`.

Main-chain membership of the hashes is not verified beyond the parent linkage.

`GetMainChainCFilters` (`txmined.go:505-543`): the header for `startHash` must exist (NotExist "starting block %v not found"); `height` = its height, plus one when not inclusive; then while `storage` has room and a block record exists at `height`, load that block's cfilter (errors returned as-is: NotExist "no cfilter saved for block %v", IO "cfilter data len: %d"), parse a **copy** of the filter bytes with `B = 19`, `M = 784931` (parse error returned as-is), store `{hash, filter, key}`, and advance. Returns the filled prefix of `storage`. Main-chain membership of `startHash` is not checked despite the doc comment (Q-16).

`CFilterV2` (`cfilters.go:17-30`): fetch, copy, parse; returns the stored key even on parse error.

`ImportCFiltersV2` (`cfilters.go:34-67`): iterate block records forward starting at the first block record whose height is `>= startHeight` (a cursor `Seek`; the height of the record found is not compared with `startHeight + i`); for each supplied blob: if the iterator is exhausted or the block record is malformed (the iterator's own IO error is discarded), NotExist "block height of %d unknown" (`startHeight + i`); validate the blob with `FromBytesV2` (error as-is); load the block's header (NotExist "block header"); key = first 16 bytes of the merkle root; put (unwrapped error). Does not change `havecfilters`.

`ImportCFiltersV2` details that affect the stored bytes: the value written is the 16-byte key followed by the supplied blob **verbatim** (the parsed filter is discarded). The only validation is that `gcs.FromBytesV2(19, 784931, blob)` succeeds, which for a non-empty blob requires nothing more than a readable, canonically encoded leading varint (the element count); an empty blob is accepted and stored as a 16-byte value (`dcrd/gcs/v4@v4.1.1/gcs.go:539-566`). The blob is not checked against the block in any way. Existing cfilter entries are overwritten without comparison. An empty `filterData` list is a no-op that does not even position the cursor. Unlike `InsertMissingCFilters`, a genesis block reached by the iterator (`startHeight = 0`) gets the merkle-root key, not the all-zero key (Q-15).

### 5.15 Multisig outputs (`txmined.go:1738-1865`, `2861-2927`)

`AddMultisigOut(dbtx, rec, block, index)`:

1. If `index >= len(TxOut)`: Invalid "transaction output does not exist".
2. Read a 135-byte zero-padded copy of the multisig record for (hash, index), if any.
3. If it exists and `block != nil`: if its stored block hash is all zero, set block hash and height from `block`, rewrite and return nil; else Invalid "multisig credit is mined".
4. If it exists and `block == nil` and its stored block hash is all zero: return nil. (If it exists, `block == nil` and it is already mined, execution continues and the record is **overwritten** as unmined and unspent.)
5. If `block == nil` use a dummy block: zero hash, height 0.
6. Classify the output script; map stake script-hash forms to `STScriptHash` and note `tree = stake` for them, else `tree = regular`. If the (mapped) class is not `STScriptHash`: Invalid "multisig output must be P2SH".
7. Extract the script hash (`getScriptHashFromP2SHScript`), look up the redeem script with `manager.redeemScriptForHash160` (errors returned as-is: address-manager NotExist, or Invalid "redeem script lookup requires P2SH address").
8. If the output's script version is not 0: IO "only version 0 scripts are supported".
9. `ExtractMultiSigScriptDetailsV0(redeemScript, false)`; if not valid: IO "invalid m-of-n multisig script".
10. Write the 135-byte record (4.5) and add the key to multisig unspent.

`SpendMultisigOut(ns, op, spendHash, spendIndex)`:

1. Read the record copy. Missing: NotExist "no multisig output for outpoint %v".
2. If already spent: if spent by exactly (`spendHash`, `spendIndex`) return nil; else DoubleSpend "outpoint %v spent by %v".
3. Set spent: flags byte **assigned** `0x01` (this clears the tree bit), spender hash and index written.
4. If the key is not in multisig unspent: IO "missing unspent multisig record" (the record has not been rewritten at this point).
5. Rewrite the record; delete the key from multisig unspent.

Marking unspent (used by `Rollback` and `RemoveUnconfirmed`): flags byte assigned `0x00`, spender-hash bytes `[67:98]` zeroed (31 of the 32 bytes; byte 98 keeps its old value), spender index set to `0xFFFFFFFF`. Marking unmined (used by `Rollback`): block hash zeroed, block height 0.

`GetMultisigOutput`: record copy → decoded `MultisigOut`. `OutPoint.Tree` is always regular; the `Tree` field carries the stored tree bit. The key-length error path uses a format string without formatting, so its message is the literal `multisig output key len %d` (unreachable with a 36-byte key).

`UnspentMultisigCreditsForAddress(dbtx, addr)`:

1. `addr` must be `*stdaddr.AddressScriptHashV0`: else Invalid "address must be P2SH".
2. Walk multisig unspent in key order. For each key: read the record (missing: IO "missing multisig credit"); skip if its script hash differs from the address hash; decode the outpoint; fetch the redeem script (error as-is); emit `{&op (Tree = stored tree bit), scriptHash, redeemScript, m, n, amount}`.

### 5.16 Transaction details and range queries (`txquery.go:56-538`)

`minedTxDetails(ns, txHash, recKey, recVal)`:

1. Decode the record (`TxRecord` with `Received`, `MsgTx`, `TxType`; `SerializedTx` is left nil; `Unpublished` false).
2. `Block.Block` = (height, hash) from the record key. `Block.Time` = the time stored in the block record at that height (IO "block record len %d" if missing). `Block.VoteBits` stays 0.
3. Credits: iterate the credits bucket over keys with the 68-byte record-key prefix, ascending output index. For each: if `Index >= len(TxOut)`: IO "saved credit index exceeds number of outputs". Fields: `Index`, `Amount`, `Spent` (stored bit), `Change`, `OpCode` (expanded tag), `IsCoinbase`, `HasExpiry` (bit 6). If not spent by a mined transaction, `Spent` = whether unmined inputs has an entry for (txHash, Index).
4. Debits: iterate the debits bucket over the same prefix, ascending input index. For each: if `Index >= len(TxIn)`: IO "saved debit index exceeds number of inputs". Fields: `Amount`, `Index`.

`unminedTxDetails(ns, txHash, v)`:

1. `Block = {Height: -1}`, zero hash, zero time.
2. Decode the record.
3. Credits: iterate unmined credits with the 32-byte hash prefix, ascending output index. If `Index >= len(TxOut)`: IO "credit output index %d does not exist for tx %v". Fields set: `Index`, `Amount`, `Change`, `HasExpiry`, `Spent` = unmined-inputs entry exists. **`OpCode` is 0 and `IsCoinbase` is false always** (Q-10).
4. Debits: for each input `i` in order: if the previous outpoint is in unspent, emit `{Amount: mined credit amount, Index: i}`; else if it is an unmined credit, emit `{Amount: that credit's amount, Index: i}`; else nothing.

In both functions the embedded `TxRecord` is filled only by the record decoder: `Hash`, `Received` (second resolution), `MsgTx`, `TxType`. `SerializedTx` is nil and `Unpublished` is **false even for an unpublished unmined transaction** (only `UnminedTxs` reads the unpublished bucket). The same holds for every `TxDetails` passed to a `RangeTransactions` callback and for `Ticket.TxRecord` (5.22). In step 4 of `unminedTxDetails` a mined previous output listed in unspent whose credit is missing from credits fails with IO "credit len 0"; the amount of an unmined previous credit is read with the mined-credit accessor (same offset, first 8 bytes).

`TxDetails(ns, txHash)`: unmined record → `unminedTxDetails`; else the newest mined record (the greatest key among tx-record keys with the hash prefix, that is the highest block height) → `minedTxDetails`; else `NotExist` (no message).

`UniqueTxDetails(ns, txHash, block)`: `block == nil` → unmined lookup; else exact (hash, block) record lookup. Absent → `(nil, nil)`.

`Tx`, `ExistsTx`, `ExistsTxMinedOrUnmined`, `TxBlockHeight`: as in 3.9; all prefer the unmined record.

`ExistsUTXO(dbtx, op)`: true if unspent has the outpoint, or unmined credits has it. Spent-by-unmined outputs still return true (intended: lets the caller detect conflicts).

`RangeTransactions(ctx, ns, begin, end, f)`:

1. If `begin < 0`: call `f` once with all unmined transactions (see below). If it returns an error or `true`, return (the error, or nil).
2. Run the block range (below).
3. If the block range finished without error and without a break, step 1 did not run, and `end < 0`: call `f` once with all unmined transactions; its break flag is ignored.

Unmined batch (`rangeUnminedTransactions`): walk the unmined bucket in key order; before each record check `ctx.Err()` and return it if set; a key shorter than 32 bytes is IO "bad unmined tx key len %d"; build `unminedTxDetails` for each. If no error and at least one record exists, call `f(details)` once. `f` is not called for an empty set.

Block range (`rangeBlockTransactions`):

1. Replace a negative `begin` or `end` with `0x7FFFFFFF`.
2. If `begin < end`: forward. Seek to the first block record with height `>= begin`; continue while `height <= end`.
3. Else (including `begin == end`): backward. Seek to `begin`; if the key found does not equal `begin` exactly (or nothing was found), step back one record; continue with `Prev` while `height >= end`.
4. For each block: check `ctx.Err()` first and return it if set. For each tx hash in the block record, in record order: load the tx record (IO "missing transaction %v for block %v" if absent), decode it, set `Block = {Block: block, Time: block record time}` (`VoteBits` 0), read credits and debits exactly as in `minedTxDetails`.
5. If the block had no transactions, skip it. Otherwise call `f(details)`. If it returns an error or `true`, stop and return.
6. On exhaustion return the iterator error, if any.

Resulting behaviour per argument combination (derived from the steps above and the block iterator, `wallet/udb/txdb.go:354-436`):

| `begin` | `end` | Callbacks, in order |
|---|---|---|
| `< 0` | `< 0` | The unmined batch only. Both bounds become `0x7FFFFFFF`, the backward walk starts at the tip record and stops at once because its height is below `end`; no block is reported. |
| `< 0` | `>= 0` | The unmined batch, then blocks from the tip **down** to `end`. |
| `>= 0` | `< 0` | Blocks from `begin` **up** to the tip, then the unmined batch. |
| `>= 0`, `< end` | `>= 0` | Blocks from `begin` up to `end`. No unmined batch. |
| `>= end` | `>= 0` | Blocks from `begin` (or, if no record has exactly that height, the highest record below it, in practice the tip) down to `end`. `begin == end` reports that single block. No unmined batch. |

A bound beyond the tip is not an error; the walk simply ends. The first block record outside the range is still decoded before the walk stops, so a malformed record just past the range surfaces as the iterator's IO error.

The slice passed to `f` is reused between blocks. Within a block, transactions appear in the order their hashes were appended to the block record (the order the wallet inserted them), which is not necessarily block order.

`RangeBlocks(ns, begin, end, f)`: same range rules; `f(&Block)` is called for every block record including empty ones; returns `f`'s error (nil when `f` breaks with `true`); the iterator error is **not** returned. No context is taken and none is checked. The pointer passed to `f` addresses the iterator's own element, which is overwritten on the next step; a callback that keeps the block must copy it (`txquery.go:763-766`).

Error-message arguments in the block pass: `missing transaction %v for block %v` is formatted with the transaction hash and the block **height** (not the block hash) (`txquery.go:441`).

`PreviousPkScripts(ns, rec, block)` (`txquery.go:542-635`; no production caller, Q-2). For a mined record (`block != nil`), the only path that works: iterate the debits bucket over the 68-byte prefix (rec.Hash, block) in ascending input index; for each debit take the 72-byte credit key embedded in the debit value; the credit must exist in credits, else IO "missing credit val for key %x"; read the script offset and length from the credit value, load the funding transaction's record (first 68 bytes of the credit key) and extract the pkScript as in 5.7 step 6c; append. Result: one script per debited input, in input order, nil when there are none; the iterator error is returned. For `block == nil` the Go first walks the inputs (previous transaction unmined and the output an unmined credit → its script; else previous output in unspent → its script, IO "missing credit value for key %x" if the credit is absent) and then crashes or errors as described in Q-2.

### 5.17 `UnminedTxs` and dependency sort (`txunmined.go:285-316`, `kahnsort.go`)

1. Walk the unmined bucket; decode each record; set `Unpublished` from the unpublished bucket (true only for a 1-byte non-zero value); collect into a hash-keyed map.
2. Build a graph: one node per record; for each input whose previous transaction hash is also in the set, add an edge parent → child and increment the child's in-degree. The duplicate-edge check compares the wrong hash and never fires, so a child spending `n` outputs of one parent gets `n` edges and in-degree `n` (self-consistent).
3. Roots = nodes with in-degree 0, in Go map iteration order.
4. If every node is a root, return the roots.
5. Otherwise Kahn's algorithm with a FIFO queue seeded with the roots; children are enqueued in edge-insertion order when their in-degree reaches 0.

The result is a valid topological order, but the relative order of independent transactions is unspecified (depends on Go's randomized map iteration). See Q-18.

`UnminedTxHashes`: keys of the unmined bucket in key order; a key shorter than 32 bytes is IO "unmined key len %d".

### 5.18 UTXO enumeration (`txmined.go:2242-2650`)

`outputCreditInfo(ns, op, block)`:

1. Look up an unmined credit for the outpoint. Only if there is none and `block != nil`, look up the mined credit (hash, index, block).
2. Neither: IO "missing credit for outpoint %v". (The "marked mined and unmined" inconsistency branch is unreachable because of step 1.)
3. Unmined: amount (IO "unmined credit len %d" if shorter than 9), opcode tag, `HasExpiry` = bit 6 of the flags byte; received time from the unmined record (IO "unmined len %d" if shorter than 8); pkScript by deserializing the unmined transaction (IO on failure; IO "no output %d for tx %v" if out of range). `FromCoinBase` is **false** regardless of the stored flag.
4. Mined: amount, opcode tag, `FromCoinBase` = coinbase bit, `HasExpiry`; received time from the tx record; pkScript via the stored offset/length or full deserialization (5.7 step 6c).
5. `op.Tree` = regular if the opcode tag is `opNonstake`, else stake.
6. Result: `Credit{OutPoint: op, BlockMeta: {Block: {Height: -1} or *block, Time: zero, VoteBits: 0}, Amount, PkScript, Received, FromCoinBase, HasExpiry}`.

`UnspentOutputs(dbtx)` returns, in this exact order:

1. Every key of the unspent bucket in ascending key order (raw tx-hash bytes, then output index big-endian), skipping keys that have an unmined-inputs entry; each resolved with `outputCreditInfo(op, block from the unspent value)`.
2. Then every key of the unmined credits bucket in ascending key order, skipping keys that have an unmined-inputs entry and keys whose transaction is unpublished; each resolved with `outputCreditInfo(op, nil)`.

Any error aborts the whole call. A trace log `%v many utxos found in database` is emitted at the end. The result includes ticket outputs, immature outputs, zero-value outputs and outputs of every account; filtering is the caller's job. The doc comment says the order is undefined; it is in fact deterministic and callers iterate it.

`ForEachUnspentOutpoint(dbtx, f)`: same two passes, same skips, same order; passes only the outpoint with `Tree` derived from the credit's opcode tag (mined: read from the credits bucket; a missing credit panics on the flag read) and stops at the first error from `f`.

The input outpoint's `Tree` is ignored by every lookup in this section (keys are hash and index only); the `Tree` of the returned outpoint is always recomputed from the credit tag. For a mined credit `Credit.Received` is the received time of the mined tx record and for an unmined credit that of the unmined record, both `time.Unix(sec, 0)`.

`UnspentOutput(ns, op, includeMempool)`: if `includeMempool` and an unmined credit exists → `outputCreditInfo(op, nil)`; else if an unspent entry exists → `outputCreditInfo(op, block)`; else NotExist "no unspent output %v". It does not consult unmined inputs, so an output spent by an unmined transaction is still returned.

`UnspentOutputCount`: `KeyN` of the unspent bucket.

### 5.19 Random UTXO selection (`txmined.go:2356-2459`)

`randomUTXO(dbtx, skip)`:

1. Draw 33 random bytes. `randKey` = first 32 bytes; `prevFirst` = low bit of byte 32.
2. First direction: `Seek(randKey)`; if `prevFirst`, the iterator is `Prev` and the first candidate is `Prev()` (the greatest key below the seek position); otherwise the iterator is `Next` and the first candidate is the key `Seek` returned.
3. Scan in that direction. For each key: if at least one key has been collected and this key's first 32 bytes differ from the first collected key's, stop; else if `skip(k, v)` is true, continue; else collect a copy of the key.
   Consequence: the scan passes over any number of ineligible keys (and whole transactions) until it finds the first eligible key, then collects every eligible key of that same transaction that is contiguous in the scan, and stops at the first key of a different transaction.
4. If anything was collected: pick one uniformly (`rand.IntN(len)`), `Seek` to it and return its key and value.
5. Otherwise scan the opposite direction from `randKey` with the same rules (if `prevFirst`: start at the key `Seek` returns, iterate `Next`; else `Seek`, then start at `Prev()`, iterate `Prev`), and pick as in step 4.
6. If still nothing: return nil.

The selection is uniform over a random position in tx-hash space, not uniform over UTXOs.

`RandomUTXO(dbtx, minConf, syncHeight)`:

1. If `minConf == 0`: Invalid "optimized random utxo selection not possible with minConf=0".
2. `skip` = key has an unmined-inputs entry, OR the unspent value is malformed, OR `!confirmed(minConf, blockHeight, syncHeight)`.
3. `randomUTXO`; nil → `(nil, nil)`.
4. Decode outpoint and block; return `outputCreditInfo`.

No account, maturity, ticket or zero-value filtering is applied here.

### 5.20 Coin selection: `MakeInputSource` (`txmined.go:3030-3351`)

`MakeInputSource(dbtx, account, minConf, syncHeight, ignore)` returns a stateful closure valid only while `dbtx` is open. State captured at creation: `numUnspent = KeyN(unspent)` (logged at debug as `Unspent bucket k/v count: %v` when `minConf != 0`), empty `seen` set, `randTries = 0`, `remainingKeys = nil` (distinct from empty), running totals and output slices.

`skip(k, v)` for mined keys: true if the key has an unmined-inputs entry; or the unspent value is malformed; or `!confirmed(minConf, height, syncHeight)`; or `k` is in `seen`.

`SelectInputs(target)` loops while `currentTotal < target || target == 0`:

1. **Pick a candidate key.**
   a. If `minConf != 0 && target != 0 && randTries < numUnspent / 2` (integer division): `randTries++`; `k = randomUTXO(skip)`; if non-nil add it to `seen`.
   b. Else if `remainingKeys` is still nil (first time only):
      - if `randTries > 0`, log at debug `Abandoned random UTXO selection attempts after %v tries`;
      - `remainingKeys` = every unspent key in key order for which `skip` is false;
      - if `minConf == 0`, append every unmined-credit key in key order that is not in `seen`, whose transaction is not unpublished, and whose opcode tag is none of `OP_SSTX`, `OP_SSGEN`, `OP_SSTXCHANGE`, `OP_SSRTX`, `OP_TADD`, `OP_TGEN` (flagged as unmined);
      - shuffle `remainingKeys` uniformly (`rand.ShuffleSlice`).
   c. If no key was picked by (a): if `remainingKeys` is empty (or still nil) **break**; else pop its first element.
   Note the interaction: when a random try returns nothing, `remainingKeys` is still nil and the loop ends without ever enumerating. That is correct only because `randomUTXO` returning nil means no eligible mined key exists and `minConf != 0` excludes unmined credits.
2. **Mined candidate.** Build the credit key (4.4.1) and read the credit.
   a. pkScript via `fastCreditPkScriptLookup` (IO "missing mined and unmined credit" if the credit is absent); account via `fetchAccountForPkScript` (5.2). **Either error aborts the whole selection.**
   b. Skip (continue the loop) if, in this order: account differs; credit is flagged spent; amount is 0; opcode tag is `OP_SSTX`; not `confirmed(minConf, height, syncHeight)`; tag is nonstake and the coinbase bit is set and not `coinbaseMatured`; tag is `OP_SSGEN`, `OP_SSRTX`, `OP_TADD` or `OP_TGEN` and not `coinbaseMatured`; tag is `OP_SSTXCHANGE` and not `ticketChangeMatured`.
   c. Outpoint from the key; `Tree` = stake if the tag is not nonstake, else regular.
3. **Unmined candidate.** pkScript by deserializing the unmined transaction; account as above (errors abort); skip if the account differs; amount from the unmined credit; `Tree` from the tag. No zero-amount check.
4. If `ignore != nil && ignore(&op)`: continue.
5. `input = wire.NewTxIn(&op, amount, nil)`: sequence `0xFFFFFFFF`, `ValueIn = amount`, `BlockHeight = 0`, `BlockIndex = 0xFFFFFFFF`, nil signature script.
6. Classify the pkScript at script version 0, map stake forms to their sub-type: P2PKH (secp256k1 ECDSA) → redeem script size 108; P2PK (secp256k1 ECDSA) → 74; anything else: log at error `Unexpected script class for credit: %v` and continue (the output is never selected; this includes all P2SH and alternate-signature credits).
7. Append to the running lists and add the amount to `currentTotal`.

After the loop return `InputDetail{Amount: currentTotal, Inputs, Scripts, RedeemScriptSizes}`. Never returns an insufficient-balance error itself; the caller compares `Amount` to `target`. Successive calls with larger targets extend the same lists (previous inputs stay first, in selection order). `target == 0` selects everything eligible, skipping the random phase.

Tries rejected in steps 2b-6 still count against `randTries` and stay in `seen`; they are not retried.

Additional details of the closure (`txmined.go:3100-3348`):

- After a key is chosen, its value is re-read with `Get` from the unspent bucket (mined) or the unmined-credits bucket (unmined); the value `randomUTXO` returned is discarded. A mined key whose unspent value is shorter than 36 bytes cannot reach the credit-key construction because `skip` rejects it.
- Keys popped from `remainingKeys` are **not** added to `seen`, and `skip` is not re-evaluated when a key is popped; the eligibility of an enumerated key is fixed at enumeration time except for the per-credit checks of step 2b.
- An error from the bucket `ForEach` during enumeration is returned as-is and leaves `remainingKeys` non-nil and partially filled.
- The `Scripts` entry for each selected input is the credit's pkScript exactly as stored in the transaction (stake-tagged scripts keep their tag opcode); `RedeemScriptSizes[i]` is 108 or 74 according to the sub-type. All four result fields are parallel and index-aligned.
- When nothing is selected the returned `InputDetail` has `Amount` 0 and nil `Inputs`, `Scripts` and `RedeemScriptSizes`; the error is nil.
- **Unmined credits are not tested against the unmined-inputs index.** The mined enumeration and the random phase go through `skip`, which rejects keys that an unmined transaction already spends. The unmined-credit enumeration of step 1b (`txmined.go:3135-3168`) applies only the three tests listed there (`seen`, unpublished, stake tag), and the unmined candidate path of step 3 (`3277-3309`) adds only the account test. With `minConf == 0` an unmined credit that another unmined transaction already spends is therefore eligible and can be returned (Q-37). The `seen` test in that enumeration can never match in a consistent database, since `seen` holds unspent-bucket keys only.
- No coinbase or maturity test is applied to unmined candidates beyond the tag filter of step 1b; an unmined nonstake credit whose coinbase bit is set is selectable at `minConf == 0`.
- The `ignore` callback receives a pointer to a per-candidate outpoint with `Tree` already set (the `TxIn` built afterwards holds its own copy); it is called after all eligibility tests and before the script-class test, so it is not called for candidates rejected earlier and is called for candidates that step 6 then rejects.
- Script classification in step 6 always uses script version 0 (`scriptVersionAssumed`), regardless of the output's actual version, because the version is not stored with the credit (upstream TODO at `txmined.go:167`).

### 5.21 Balances: `balanceFullScan` (`txmined.go:3356-3545`)

Inputs: `minConf`, `syncHeight` (main chain tip height when called through `AccountBalances`). Output: map account → `Balances`; an entry is created for every account that owns at least one scanned item, even if all categories stay 0.

Pass 1, unspent bucket in key order, skipping keys with an unmined-inputs entry:

1. Build the credit key; the credit must exist (IO "missing credit for unspent output").
2. Account via pkScript lookup + `fetchAccountForPkScript` (errors abort).
3. Amount, height (from the credit key), opcode tag. Then by tag:

| Tag | Effect |
|---|---|
| `opNonstake`, `OP_TGEN` | `fromCoinbase` = coinbase bit; `mature = fromCoinbase && coinbaseMatured`. If `(confirmed(minConf) && !fromCoinbase) || mature`: `Spendable += amt`. Else if `fromCoinbase && !mature`: `ImmatureCoinbaseRewards += amt`. Always `Total += amt`. |
| `OP_SSTX` | `VotingAuthority += amt` (not in `Total`) |
| `OP_SSGEN`, `OP_SSRTX` | if `coinbaseMatured`: `Spendable += amt`, else `ImmatureStakeGeneration += amt`. `Total += amt`. |
| `OP_SSTXCHANGE` | if `ticketChangeMatured`: `Spendable += amt`. `Total += amt`. |
| other | log at warn `Unhandled opcode: %v`; nothing added |

Consequences: a mature coinbase or stake-generation output is spendable regardless of `minConf`; a mined non-coinbase output below `minConf` appears only in `Total`.

Pass 2, unmined credits in key order, skipping keys with an unmined-inputs entry:

1. Account as above (errors abort). Amount. Opcode tag. `unpublished` = the transaction has an unpublished marker.
2. By tag:

| Tag | Effect |
|---|---|
| `opNonstake` | if `minConf == 0 && !unpublished`: `Spendable += amt`; else if the coinbase bit (bit 5 of the flags byte) is clear: `Unconfirmed += amt`. Always `Total += amt` (unpublished outputs included). |
| `OP_SSTX` | `VotingAuthority += amt` |
| `OP_SSGEN`, `OP_SSRTX` | `ImmatureStakeGeneration += amt`; `Total += amt` |
| `OP_SSTXCHANGE` | `Total += amt` |
| `OP_TGEN` | nothing |
| other | log at warn `Unhandled unconfirmed opcode %v: %v` (tag, raw value bytes) |

Pass 3, unspent ticket commitments in key order: skip entries whose unminedSpent flag is set; otherwise `LockedByTickets += amount` and `Total += amount` for the commitment's account. If an entry is malformed (flags value empty, commitment value shorter than 12 bytes) the pass **stops silently** and the function returns the partial result with a nil error (Q-11).

Category definitions that RPC clients see:

| Field | Meaning as computed |
|---|---|
| `Spendable` | mined regular non-coinbase outputs with `>= minConf` confirmations; mature coinbase, vote, revocation and treasury-gen outputs; mature ticket change; unmined published regular outputs when `minConf == 0` |
| `ImmatureCoinbaseRewards` | mined coinbase-flagged outputs (coinbase and treasury-gen) not yet past coinbase maturity |
| `ImmatureStakeGeneration` | mined vote/revocation outputs not yet past coinbase maturity; all unmined vote/revocation outputs |
| `LockedByTickets` | sum of wallet-owned ticket commitment amounts whose ticket has no vote/revocation recorded (mined or unmined) |
| `VotingAuthority` | sum of wallet-owned ticket submission outputs (output 0 of tickets) not yet spent, mined or unmined |
| `Unconfirmed` | unmined regular (nonstake-tagged) outputs not counted as spendable and whose coinbase bit is clear (all of them when `minConf != 0`; unpublished ones when `minConf == 0`) |
| `Total` | everything above except `VotingAuthority`, plus mined-but-unconfirmed regular outputs, immature mined ticket change, unmined ticket change, and unmined nonstake outputs whose coinbase bit is set (which enter no other category unless `minConf == 0` and published) |

All sums exclude outputs spent by an unmined transaction.

### 5.22 Ticket queries (`txmined.go:2652-2849`, `txquery.go:198-241`)

`UnspentTickets(dbtx, syncHeight, includeImmature)`: walk the tickets bucket in key order. For each ticket hash:

1. If output 0 of the ticket has no entry in unspent: skip. This excludes tickets spent by a mined transaction, **unmined tickets**, and tickets whose output 0 is not a wallet credit.
2. If unmined inputs names a spender of output 0: deserialize the spender (IO on failure; a missing spender record panics). Include the ticket only if the spender is a vote whose voted-on block hash differs from the current tip hash; otherwise skip. Inclusion on this path bypasses step 3.
3. If `!includeImmature`: find the newest mined tx record for the hash (skip if none), take its height, and skip unless `ticketMatured(params, height, syncHeight)`.
4. Append the hash.

`IterateTickets` / `TicketIterator.Next`: walk the tickets bucket in key order. For each key:

1. If a mined tx record exists (newest): `Block` from the record key; decode the record into the embedded `TxRecord`. Spender: if a credit exists for (hash, 0, block): `SpenderHash` = the spender tx hash from the credit if its spent bit is set, else zero (**unmined inputs is not consulted in this case**). If no credit exists for output 0: `SpenderHash` = the unmined-inputs spender of (hash, 0), else zero.
2. Else if an unmined record exists: `Block = {Height: -1}`, `SpenderHash` zero, decode the record.
3. Else (ticket record without any transaction): advance and try the next key.
4. Advance the cursor and return true. Returns false at the end or after an error (`Err()` reports it). `Close()` closes the cursor.

The iterator decodes every ticket into the same embedded `Ticket` value: `TxRecord.Hash`, `Received`, `MsgTx` and `TxType` are overwritten on each successful `Next`; `SerializedTx` stays nil and `Unpublished` stays false (also for an unpublished unmined ticket). Callers that keep a ticket across `Next` calls must copy it. The value stored in the tickets bucket (the picked height) is never read by this code; only its non-nil-ness drives the loop (`it.cv == nil` ends iteration). If `Next` fails while decoding, the cursor is not advanced and every later `Next` returns false because `err` is set. `Close` is nil-safe and may be called more than once.

`UnspentTickets` returns the hashes in ascending raw key order of the tickets bucket (the doc comment says the order is undefined) and returns a nil slice when there are none. The `syncHeight` argument is used only in the maturity test of step 3; the tip hash used in step 2 is read from the database with `MainChainTip`, independent of `syncHeight`. The `serializedSpender == nil` test at `txmined.go:2682-2684` can never be true (slicing a nil record value panics first, slicing a non-nil one yields a non-nil slice).

`OwnTicket`: key present in the tickets bucket (true also for tickets whose transaction was later removed).

`TicketDetails(ns, txDetails)`:

1. If the transaction is not a ticket (`stake.IsSStx`): `(nil, nil)`.
2. Spender hash exactly as in the iterator step 1, using `txDetails.Block.Block` (for an unmined ticket the credit lookup misses and the unmined-inputs path is used).
3. `Spender = TxDetails(ns, spenderHash)`; a `NotExist` result (including the all-zero hash) leaves `Spender` nil; other errors are returned.

### 5.23 `Spender` (`txquery.go:640-723`)

Input: outpoint (tree ignored).

1. If a mined tx record exists for the hash (newest):
   a. The credit (hash, index, that block) must exist: else Invalid "output is not a credit".
   b. If its spent bit is set: the spender is the mined transaction named by the embedded debit key; return its `MsgTx` and input index (IO if the record is malformed).
   c. Else read unmined inputs for the outpoint. None: NotExist "credit is unspent". Otherwise remember the spender hash.
2. If no spender hash is known yet (no mined record for the hash): an unmined credit for the outpoint must exist (else Invalid "output is not a credit"), and unmined inputs must name a spender (else NotExist "credit is unspent").
3. Load the unmined spender record. Missing: NotExist "missing unmined spending tx". Deserialize failure: kind `Bug`.
4. Find the first input whose previous outpoint hash and index match. None: NotExist "recorded spending tx does not spend credit". Return the transaction and that index.

### 5.24 Small accessors

- `MainChainTip`: 3.5. `GetMainChainBlockHashForHeight`, `GetSerializedBlockHeader`, `GetBlockHeaderTime`, `GetBlockHeader`, `BlockInMainChain`, `GetBlockMetaForHash`: 3.5, single lookups.
- `GetMainChainBlockHashes(ns, startHash, inclusive, storage)`: header must exist; the block record at its height must have the same hash; start at that height (plus one when not inclusive); copy consecutive block-record hashes into `storage` until it is full or a height is missing; return the filled prefix.
- `ProcessedTxsBlockMarker`, `IsMissingMainChainCFilters`, `OwnTicket`, `UnspentOutputCount`, `SetPublished`: single key operations.

---

## 6. Constants, limits and defaults

| Name | Value | Source | What depends on it |
|---|---|---|---|
| `opNonstake` | `0xb9` (`OP_NOP10`) | `wallet/udb/txmined.go:35`; `dcrd/txscript/v4@v4.1.2/opcode.go:238` | Tag for non-stake credits; tree selection; balance and selection switches |
| `OP_SSTX` / `OP_SSGEN` / `OP_SSRTX` / `OP_SSTXCHANGE` | `0xba` / `0xbb` / `0xbc` / `0xbd` | `opcode.go:239-242` | Credit tags |
| `OP_TADD` / `OP_TGEN` | `0xc1` / `0xc3` | `opcode.go:246-248` | `OP_TGEN` is a credit tag; `OP_TADD` is only compared against (dead) |
| `scriptVersionAssumed` | 0 | `txmined.go:39` | Address extraction and script classification where the version is not stored |
| Serialized header size | 180 bytes | `txmined.go:607` | `RawBlockHeader`; offsets in 4.2 |
| Birthday value size | 45 bytes | `txmined.go:350` | 4.1 |
| Birthday flags | `0x01` from height, `0x02` from time | `txmined.go:347-348` | 4.1 |
| Unmined height sentinel | `-1` | `txmined.go:2334`, `txquery.go:113` | `Credit`, `TxDetails`, `Ticket`, `TxBlockHeight`, `confirms` |
| Range bound for negative heights | `0x7FFFFFFF` | `txquery.go:392-397`, `731-736` | `RangeTransactions`, `RangeBlocks` |
| Reverse block iterator seek key | `0xFFFFFFFF` | `txdb.go:365` | `Rollback` |
| Ticket record picked height | `-1` (`0xFFFFFFFF`) | `txmined.go:1335`, `txunmined.go:122` | Tickets bucket value |
| Unspent credit spender index | `0xFFFFFFFF` | `txmined.go:1136` | In-memory only |
| Multisig unspent `SpentByIndex` | `0xFFFFFFFF` | `txmined.go:1820`, `txdb.go:1909` | Multisig record |
| Random seek material | 33 bytes (32 key + 1 direction bit) | `txmined.go:2361-2364` | 5.19 |
| Random-try budget | `KeyN(unspent) / 2` at source creation | `txmined.go:3073`, `3104` | 5.20 |
| `RedeemP2PKHSigScriptSize` | 108 | `wallet/txsizes/size.go:28` | `InputDetail.RedeemScriptSizes` (fee estimation) |
| `RedeemP2PKSigScriptSize` | 74 | `wallet/txsizes/size.go:18` | same |
| New input sequence / block height / block index | `0xFFFFFFFF` / `0` / `0xFFFFFFFF` | `dcrd/wire@v1.8.0/msgtx.go:338-347` | Inputs returned by `SelectInputs` |
| cfilter parameters `B`, `M` | 19, 784931 | `dcrd/gcs/v4@v4.1.1/blockcf2/blockcf.go:33-38` | Parsing stored filters |
| cfilter key size | 16 | `dcrd/gcs/v4@v4.1.1/gcs.go:69` | `BlockCFilter.Key` |
| `BlockValid` vote bit | `0x0001` | `dcrd/dcrutil/v4@v4.0.3/bitflags.go:17` | `approvesParent` |
| `NoExpiryValue` | 0 | `dcrd/wire@v1.8.0/msgtx.go:40` | hasExpiry flag; `PruneUnmined` |
| `CoinbaseMaturity` | mainnet 256; testnet3, simnet, regnet 16 | `dcrd/chaincfg/v3@v3.3.0/mainnetparams.go:555` and siblings | `coinbaseMatured` |
| `SStxChangeMaturity` | 1 on all networks | `mainnetparams.go:556` | `ticketChangeMatured` |
| `TicketMaturity` | mainnet 256; others 16 | `mainnetparams.go:553` | `ticketMatured` |
| Commitment amount location | pkScript bytes `[22:30]` LE, top bit masked; minimum script length 32 | `dcrd/blockchain/stake/v5@v5.0.3/staketx.go:446-459` | `AddTicketCommitment` |
| Vote block reference | `TxOut[0].PkScript[2:34]` hash, `[34:38]` LE height | `staketx.go:466-477` | `InsertMemPoolTx`, `PruneUnmined`, `UnspentTickets` |
| `stake.TxType` values | Regular 0, SStx 1, SSGen 2, SSRtx 3, TAdd 4, TSpend 5, TreasuryBase 6 | `staketx.go:30-38` | `TxRecord.TxType` |
| Ticket input positions | vote: input 1; revocation: input 0 | `txmined.go:1302-1303`, `1616-1626` | Balance exemption, commitment redemption |
| Maturity formulas | coinbase: `cur - h + 1 > CoinbaseMaturity`; change: `cur - h + 1 > SStxChangeMaturity`; ticket: `cur - h > TicketMaturity` | `txmined.go:2949-2967` | Selection, balances, `UnspentTickets` |
| Confirmation formula | `cur - h + 1`, 0 if unmined or `h > cur` | `txmined.go:2938-2945` | Selection, balances |

There are no configuration options, defaults or tunables in this subsystem.

---

## 7. Concurrency and lifecycle

1. `Store` has no goroutines, channels, mutexes or caches. All state is in the database.
2. Every method executes inside a `walletdb` transaction owned by the caller. bbolt allows one read-write transaction at a time and any number of concurrent read-only transactions, each with a consistent snapshot. All ordering and atomicity guarantees callers observe come from that: a batch of store calls made in one `walletdb.Update` is all-or-nothing, and readers never see a partially applied `InsertMinedTx`, `Rollback` or `ExtendMainChain`.
3. On error the store does not undo its partial writes; correctness relies on the caller rolling the transaction back. A port must not commit after an error from a mutating store call.
4. Panics (section 8.3) unwind through the caller's transaction closure; the Go wallet does not recover them in this package.
5. Objects that capture a transaction and must not outlive it: `InputSource` (closure over `dbtx` and both namespace buckets), `TicketIterator` (holds a cursor), every `[]byte` obtained from a bucket. `Credit.PkScript` for mined credits found via a stored offset is a copy; for legacy and unmined credits it is the slice produced by transaction deserialization (also independent of the database page).
6. `RangeTransactions` checks `ctx.Err()` once per unmined record and once per block; cancellation surfaces as the context error. In the block pass it is returned unwrapped. In the unmined pass it is returned from the bucket `ForEach` callback, and the `walletdb` bdb layer passes every `ForEach` error through `convertErr` (`wallet/internal/bdb/db.go:18-36`, `155-157`), which wraps it in an `*errors.Error` with no kind; the error string (`context canceled` / `context deadline exceeded`) and `errors.Is` matching against the context error are unchanged. No other method takes a context.
7. Randomness comes from the process-global CSPRNG in `dcrd/crypto/rand`, which serialises access internally (`dcrd/crypto/rand@v1.0.1/default.go:136-148`).
8. Lifecycle: constructed once by `udb.Open` after the address manager is loaded; never closed. The store requires the database to be at exactly `DBVersion` (`wallet/udb/open.go:33-38`); all `hasExpiry` reads and writes therefore use bit 6 of the flags byte.
9. Call-order contracts the store assumes and does not check:
   - a header and block record exist for a block before `InsertMinedTx`/`AddCredit` reference it;
   - `InsertMemPoolTx` precedes `AddCredit(block == nil)` for the same transaction;
   - `AddTicketCommitment` is called only for odd outputs of tickets owned by a wallet account;
   - transactions of a block are inserted after the block is attached, and spenders after the transactions they spend (a spender inserted before its funding credit exists records no debit, and the credit added later stays unspent);
   - `Rollback(height)` is called with `1 <= height <= tipHeight + 1`;
   - a mined `AddCredit` is preceded by `InsertMinedTx` for the same record and block: unlike the unmined path (explicit panic), the mined path of `addCredit` and the invalidated path of `AddCredit` never check that a tx record exists for (hash, block), nor that the block is the main-chain block at its height (only the header and the block record at the header's height are read, and only for regular transactions). A credit written without its tx record makes `outputCreditInfo` panic (received-time read of a nil value, 8.3) and makes `fastCreditPkScriptLookup` fail with IO "invalid script offset %d for tx %v" (or IO "tx record len 0" for a credit without a stored script offset) on the next UTXO, selection or balance query.

---

## 8. Errors

### 8.1 Kinds

Errors are `*errors.Error` values (`errors/errors.go`) carrying a `Kind` and usually a message; no `Op` is set by this package (callers add one). The kinds produced here and their string forms: `Invalid` ("invalid operation"), `IO` ("I/O error"), `Exist` ("item already exists"), `NotExist` ("item does not exist"), `DoubleSpend` ("double spend"), `Bug` ("internal wallet error"). An error string is `<kind string>: <message>`. How a kind maps to a JSON-RPC error code or gRPC status is decided by the RPC layers, which match on kind with `errors.Is`; this subsystem's contract is the kind and the message text.

### 8.2 Error catalogue

| Kind | Message (format) | Raised by |
|---|---|---|
| Invalid | `block 0 cannot be added` | `ExtendMainChain` |
| Invalid | `main chain may not be extended without first saving all previous cfilters` | `ExtendMainChain` |
| Exist | `block already recorded in main chain` | `ExtendMainChain` |
| Invalid | `not direct child of current tip` | `ExtendMainChain` |
| Invalid | `invalid height for next block` | `ExtendMainChain` |
| IO | `missing block record for height %v` | `stakeValidate`, `stakeInvalidate` |
| IO | `missing transaction record for tx %v block %v` | `stakeValidate`, `stakeInvalidate` |
| Invalid | `%v is not a main chain block` | `UpdateProcessedTxsBlockMarker` |
| NotExist | (none) | `MissingCFiltersHeight`, `TxDetails` |
| Invalid | `all cfilters for main chain blocks are already recorded` | `InsertMissingCFilters` |
| Invalid | `slices must have equal len` | `InsertMissingCFilters` |
| NotExist | `missing header for block %v` | `InsertMissingCFilters` |
| Invalid | `invalid stored header %v` | `InsertMissingCFilters` (unreachable) |
| Invalid | `block hashes are not ordered or previous cfilters are missing` | `InsertMissingCFilters` |
| NotExist | `starting block %v not found` | `GetMainChainCFilters`, `GetMainChainBlockHashes` |
| NotExist | `no cfilter saved for block %v` | cfilter fetch |
| IO | `cfilter data len: %d` | cfilter fetch |
| NotExist | `block height of %d unknown` | `ImportCFiltersV2` |
| NotExist | `no block at height %v in main chain` | `GetMainChainBlockHashForHeight` |
| NotExist | `block header` | `GetSerializedBlockHeader`, `GetBlockHeaderTime`, `GetBlockHeader` |
| NotExist | `block %v header not found` | `GetBlockMetaForHash` |
| Invalid | `block %v not in main chain` | `GetMainChainBlockHashes` |
| (none; plain standard-library error, not an `*errors.Error`) | `no addresses decoded from pkScript` | `fetchAccountForPkScript` |
| NotExist | `no address %v` | via `Manager.AddrAccount` |
| Invalid | `block header must be recorded` | `InsertMinedTx` |
| Invalid | `mined transactions must be added to main chain blocks` | `InsertMinedTx` |
| IO | `missing credit %v, key %x, spent by %v` | `moveMinedTx`, `Rollback` |
| Invalid | `transaction output index for credit does not exist` | `AddCredit` |
| Invalid | `index should be of an existing output` | `AddTicketCommitment` |
| Invalid | `index should be of a ticket commitment` | `AddTicketCommitment` |
| Invalid | `transaction record should be of a ticket` | `AddTicketCommitment` |
| (stake rule error, unwrapped) | `short read of sstx commit pkscript` | `AddTicketCommitment` |
| Invalid | `rec must be a vote or revocation tx` | `RedeemTicketCommitments` |
| Invalid | `transaction output does not exist` | `AddMultisigOut` |
| Invalid | `multisig credit is mined` | `AddMultisigOut` |
| Invalid | `multisig output must be P2SH` | `AddMultisigOut` |
| IO | `only version 0 scripts are supported` | `AddMultisigOut` |
| IO | `invalid m-of-n multisig script` | `AddMultisigOut` |
| NotExist | `no multisig output for outpoint %v` | `SpendMultisigOut`, `GetMultisigOutput` |
| DoubleSpend | `outpoint %v spent by %v` | `SpendMultisigOut` |
| IO | `missing unspent multisig record` | `SpendMultisigOut` |
| Invalid | `address must be P2SH` | `UnspentMultisigCreditsForAddress` |
| IO | `missing multisig credit` | `UnspentMultisigCreditsForAddress` |
| Invalid | `cannot rollback the genesis block` | `Rollback` |
| IO | `missing credit for outpoint %v` | `outputCreditInfo` |
| IO | `inconsistency: credit %v is marked mined and unmined` | `outputCreditInfo` (unreachable) |
| IO | `no output %d for tx %v` | `outputCreditInfo`, `fastCreditPkScriptLookup` |
| Invalid | `optimized random utxo selection not possible with minConf=0` | `RandomUTXO` |
| NotExist | `no unspent output %v` | `UnspentOutput` |
| IO | `missing mined and unmined credit` | `fastCreditPkScriptLookup` |
| IO | `missing credit for unspent output` | `balanceFullScan` |
| Exist | `transaction already exists mined` | `InsertMemPoolTx` |
| DoubleSpend | `vote or revocation %v double spends unmined vote %v on the tip block` | `InsertMemPoolTx` |
| DoubleSpend | `%v conflicts with %v by double spending %v` | `InsertMemPoolTx` |
| IO | `saved credit index exceeds number of outputs` | `minedTxDetails`, block range |
| IO | `saved debit index exceeds number of inputs` | `minedTxDetails`, block range |
| IO | `credit output index %d does not exist for tx %v` | `unminedTxDetails` |
| NotExist | `tx %s not found` | `Tx` |
| NotExist | `no transaction %v` | `TxBlockHeight` |
| IO | `bad unmined tx key len %d` | unmined range |
| IO | `missing transaction %v for block %v` | block range |
| IO | `missing credit value for key %x`, `missing credit val for key %x` | `PreviousPkScripts` |
| Invalid | `output is not a credit` | `Spender` |
| NotExist | `credit is unspent` | `Spender` |
| NotExist | `missing unmined spending tx` | `Spender` |
| Bug | (wire deserialize error) | `Spender` |
| NotExist | `recorded spending tx does not spend credit` | `Spender` |
| IO | low-level decode messages from `txdb.go`: `mined balance len %d`, `block record len %d`, `%d tx block record len %d`, `block key len %d`, `tx record len %d`, `tx record key len %d`, `credit len %d`, `credit key len %d`, `debit len %d`, `debit key len %d`, `unspent len %d`, `outpoint len %d`, `unmined len %d`, `unmined key len %d`, `unmined credit len %d`, `unmined credit key len %d`, `missing transaction output for credit index`, `invalid script offset %d for tx %v`, `wrong unspent ticket commitment len %d`, `ticket commitment amount %d`, `ticket commitment account %d` | any reader; owned by `03-txstore-formats.md` |
| IO | (wrapped bucket `Put`/`Delete` error; wrapped `wire` deserialize error) | all writers; record readers |
| (not re-wrapped by the store) | bucket `Put` error (`SetBirthState`, cfilter writes) or `Delete` error (`deleteBlockRecord` in `Rollback`). The value is what the `walletdb` bdb layer returns: the bbolt error already wrapped by `convertErr` in an `*errors.Error` whose kind is `Invalid` for a closed or read-only transaction and for key/value size or type errors, `IO` for `bolt.ErrInvalid`, and unset otherwise (`wallet/internal/bdb/db.go:18-36`). | `SetBirthState`, cfilter writes; `deleteBlockRecord` in `Rollback` |
| (unwrapped) | gcs parse error | `GetMainChainCFilters`, `CFilterV2`, `ImportCFiltersV2` |
| (unwrapped in the block pass; wrapped in a kind-less `*errors.Error` by the bucket `ForEach` layer in the unmined pass, section 7 item 6) | `context.Canceled` / `context.DeadlineExceeded` | `RangeTransactions` |

A short tx-record key additionally prints a Go stack trace to standard error before returning its IO error (`txdb.go:578-604`).

Format arguments, where the verb alone does not say what is printed:

| Message | Arguments |
|---|---|
| `missing block record for height %v` | the `int32` height (`txmined.go:634`, `770`) |
| `missing transaction record for tx %v block %v` | tx hash, block **hash** (`txmined.go:662-663`, `798-799`) |
| `missing transaction %v for block %v` | tx hash, block **height** (`txquery.go:441`) |
| `missing credit %v, key %x, spent by %v` | previous outpoint (`hash:index`), 72-byte credit key as lowercase hex, spender tx hash (`txmined.go:1107-1108`, `2035-2036`) |
| `missing credit for outpoint %v`, `inconsistency: credit %v is marked mined and unmined`, `no unspent output %v`, `no multisig output for outpoint %v` | the outpoint (`hash:index`) |
| `outpoint %v spent by %v` | outpoint, recorded spender tx hash (`txmined.go:1849`) |
| `no output %d for tx %v`, `credit output index %d does not exist for tx %v` | output index, tx hash |
| `%v is not a main chain block`, `missing header for block %v`, `invalid stored header %v`, `starting block %v not found`, `block %v header not found`, `block %v not in main chain` | block hash |
| `tx %s not found`, `no transaction %v` | tx hash (`txquery.go:274`, `343`) |
| `no block at height %v in main chain` | the `int32` height |
| `missing credit value for key %x`, `missing credit val for key %x` | 72-byte credit key as lowercase hex |
| `invalid script offset %d for tx %v` | the stored script offset **plus 8** (the offset into the record value, after the received-time prefix), tx hash (`txdb.go:631-639`) |
| `block record len %d` | the value length in `appendRawBlockRecord` and `fetchBlockTime` (`txdb.go:268`, `288`); in `readRawBlockRecord` the **key** length is printed instead (always 4 for a real block key), although the test is on the value length (`txdb.go:312-314`) |
| `%d tx block record len %d` | transaction count from the record, value length (`txdb.go:319-320`) |

`errors.E` stores a plain string argument and an `errors.Errorf` value alike as the nested `Err` of the `*errors.Error` (`errors/errors.go`, `func E`); both print as `<kind string>: <text>`.

### 8.3 Panics

These abort the calling goroutine in Go. A port must decide per case whether to keep a panic or return an error (section 13); none of them is reachable with a consistent database and a well-behaved caller.

| Condition | Where |
|---|---|
| `attempted to add credit for unmined tx, but unmined tx with same hash does not exist` (explicit) | `addCredit` |
| `unimplemented: moveMinedTx called on a stake-invalidated tx: block %v height %v tx %v` (explicit) | `InsertMinedTx` |
| Header exists but no block record at its height (nil slice index) | `InsertMinedTx`, `GetMainChainBlockHashes` |
| Missing header or block record for `block.Hash` | `AddCredit` (regular tx, mined) |
| Tip hash without a stored header | `MainChainTip`, `ExtendMainChain` (step 4) |
| No block record at `height - 1`; negative `height` | `Rollback` |
| Unmined-inputs entry naming a spender with no unmined record | `InsertMemPoolTx` (vote/revocation path), `UnspentTickets` |
| Unspent entry without a credit | `ForEachUnspentOutpoint` |
| Mined credit without a tx record | `outputCreditInfo` |
| nil filter | `ExtendMainChain`, `InsertMissingCFilters` |
| `block == nil` | `PreviousPkScripts` |
| Short header slice | `ExtractBlockHeader*` helpers |
| Vote with fewer than 2 inputs / malformed scripts passed where the type was asserted by the caller | `originalTicketInfo`, `SSGenBlockVotedOn` |
| Unspent value shorter than 36 bytes (credit key built by slicing without a length check) | `balanceFullScan` pass 1 (`txmined.go:3369-3373`). `MakeInputSource` and `RandomUTXO` are shielded by `skip`; `UnspentOutputs`, `UnspentOutput` and `ForEachUnspentOutpoint` return IO "unspent len %d" instead. |
| Credit or unmined-credit value shorter than 9 bytes where the tag byte (offset 8) is read before any length-checked accessor | `Rollback` input loop (`txmined.go:2038`), `ForEachUnspentOutpoint` unmined pass (`2615`), `MakeInputSource` unmined enumeration (`3146`). Elsewhere (`moveMinedTx`, `Rollback` output loop, `outputCreditInfo`, `SelectInputs` candidates, `balanceFullScan`) a length-checked amount read precedes the tag read and yields IO "credit len %d" / "unmined credit len %d". |
| Block record shorter than 43 bytes | every read of the stake-invalid byte (`ExtendMainChain` via `stakeValidate`/`stakeInvalidate`, `InsertMinedTx`, `AddCredit`, `BlockInMainChain`) |
| Unmined-inputs value shorter than 32 bytes | `Spender` and `InsertMemPoolTx` (`readRawUnminedInputSpenderHash`, `txdb.go:1711`; called at `txquery.go:681`, `697` and `txunmined.go:48`) |
| `index >= len(TxOut)` | `addCredit` when called directly (the exported `AddCredit` checks first) |
| nil pointer arguments (`rec`, `header`, `blockHash`, `txHash`, `op`, `bs`, `txDetails`) | nil dereference in the respective method; no pointer argument is validated, except that `block == nil` is the documented "unmined" selector for `AddCredit`, `AddMultisigOut`, `RedeemTicketCommitments`, `UniqueTxDetails` and `outputCreditInfo`, and `ignore == nil` is allowed in `MakeInputSource` |

### 8.4 Log output

All through the `WLLT` subsystem logger.

| Level | Message (format) | Where |
|---|---|---|
| Debug | `Marking unconfirmed transaction %v mined in block %d` | `moveMinedTx` |
| Debug | `Marking transaction %v output %d (%v) spendable` | `addCredit` (mined) |
| Debug | `Accounting for ticket commitment %v:%d (%v) from the wallet` | `AddTicketCommitment` |
| Debug | `Removing unspent ticket commitment %v:%d from the wallet` | `removeUnspentTicketCommitments` |
| Debug | `Marking ticket commitment %v:%d unmined spent as %v` | `replaceTicketCommitmentUnminedSpent` |
| Debug | `Rolling back transactions from block %v height %d` | `Rollback` |
| Debug | `Transaction %v spends a removed coinbase output -- removing as well` | `Rollback` |
| Debug | `Unspent bucket k/v count: %v` | `MakeInputSource` (`minConf != 0`) |
| Debug | `Abandoned random UTXO selection attempts after %v tries` | `SelectInputs` |
| Debug | `Removing double spending transaction %v` | `removeDoubleSpends` |
| Debug | `Transaction %v is part of a removed conflict chain -- removing as well` | `RemoveUnconfirmed` |
| Debug | `Removing unconfirmed ticket commitment %s:%d` | `RemoveUnconfirmed` |
| Trace | `%v many utxos found in database` | `UnspentOutputs` |
| Info | `Inserting unconfirmed transaction %v` | `InsertMemPoolTx` |
| Info | `Removing expired unmined transaction %v` / `Removing old unmined ticket purchase %v` / `Removing missed or invalid vote %v` | `PruneUnmined` |
| Warn | `Unhandled opcode: %v` | `balanceFullScan` pass 1 |
| Warn | `Unhandled unconfirmed opcode %v: %v` | `balanceFullScan` pass 2 |
| Error | `Unexpected script class for credit: %v` | `SelectInputs` |

---

## 9. Compatibility-critical behaviours

A drop-in replacement must reproduce each of these exactly. "Byte parity" means that after the same sequence of store calls the `wtxmgr` bucket holds the same keys and values a Go dcrwallet would have written, so that either implementation can reopen the other's `wallet.db`.

1. **Write sets.** Each mutating algorithm in 5.3-5.15 defines exactly which keys are put and deleted and in which buckets. Byte parity requires the same set, including the writes that look redundant: the unconditional `bal` rewrite in `InsertMinedTx`; the 94-byte rewrite of a credit on spend and unspend with the stale spender key left in place after unspend; unspent-commitment entries re-created by `replaceTicketCommitmentUnminedSpent`.
2. **Mined balance arithmetic (`bal`).** Additions and subtractions exactly as in 5.4-5.8, including the three different ticket exemptions (opcode tag `OP_SSTX` in `addCredit`, `moveMinedTx` and `Rollback` inputs; input position in `InsertMinedTx`; `SStx && output 0` in `Rollback` outputs). The stored value is otherwise unused by this subsystem's balance queries but is persisted and must match.
3. **Block-record transaction order is insertion order.** `RangeTransactions`, `Rollback` and stake (in)validation walk it; RPC transaction listings and notifications inherit it.
4. **`UnspentOutputs` / `ForEachUnspentOutpoint` order:** unspent bucket ascending by raw key, then unmined credits ascending by raw key, with the skips in 5.18. Callers build listings and watch lists from it.
5. **Balance categories** exactly as in 5.21, including: mature coinbase/stakegen outputs spendable regardless of `minConf`; `VotingAuthority` excluded from `Total`; unpublished unmined outputs counted in `Total` and `Unconfirmed`; treasury-gen outputs treated as coinbase; mined-but-unconfirmed regular outputs in `Total` only; unmined `OP_TGEN` ignored. These numbers are returned verbatim by `getbalance`, the gRPC `Balance`/`Accounts` calls and Decrediton's overview.
6. **Coin selection eligibility** (5.20): account match, non-zero amount (mined only), not a ticket submission, confirmations, coinbase/stakegen/change maturity, not spent by an unmined transaction, unmined credits only at `minConf == 0` and only regular published ones, only secp256k1 P2PKH/P2PK (bare or stake-tagged). Which UTXOs a transaction may spend, and the per-input script-size estimate (108 or 74) that drives the fee and therefore the change amount, must match. The random order is not reproducible and need not be, but the selection must stay unbiased in the same way (random seek, then shuffled remainder); selecting in key order would be an observable privacy regression.
7. **`InputDetail` shape:** inputs created with sequence `0xFFFFFFFF`, `ValueIn` = credit amount, `BlockHeight` 0, `BlockIndex` `0xFFFFFFFF`, nil signature script, outpoint tree derived from the credit tag; cumulative across calls.
8. **`TxDetails` semantics** (5.16): credits ascending by output index, debits ascending by input index; `Spent` true if spent by a mined **or** unmined transaction; for unmined transactions `OpCode == 0`, `IsCoinbase == false`, block height `-1`, zero hash and time; debits of unmined transactions synthesized from the unspent index, then unmined credits. `Block.VoteBits` always 0.
9. **Unmined-first lookup.** `TxDetails`, `Tx`, `TxBlockHeight`, `ExistsTxMinedOrUnmined` consult the unmined bucket before mined records; for mined duplicates the record in the highest block wins.
10. **Range conventions:** negative bound = unmined / end of chain; `begin < end` forward; otherwise backward; unmined batch first when `begin < 0`, last when only `end < 0`; empty blocks skipped for transactions but not for `RangeBlocks`; one callback per block; one callback for the whole unmined set.
11. **`InsertMemPoolTx` policy:** `Exist` when mined; silent success when already unmined; `DoubleSpend` for any conflict with an unmined transaction, except that a vote or revocation evicts an unmined vote that does not vote on the tip. Error kinds and messages reach RPC clients.
12. **`InsertMinedTx` policy:** unmined conflicts (and their descendants) are evicted; the same transaction already unmined is moved, keeping its unmined record bytes and received time and clearing the hasExpiry flag of its credits.
13. **Ticket records** are written on first sight (mined or unmined) with value `0xFFFFFFFF`, never updated and never deleted. `OwnTicket` therefore stays true for abandoned tickets.
14. **Unspent ticket-commitment state machine** (5.9 table). It alone determines `LockedByTickets`.
15. **`Rollback` semantics** (5.8): coinbase-like transactions are dropped and their unmined descendants removed; all other transactions return to unmined with their inputs re-registered in unmined inputs and their credits converted to unmined credits; headers and cfilters of detached blocks are retained; tip moves to `height - 1`; the returned map contains every transaction of every detached block in block-record order.
16. **Stake (in)validation** on every `ExtendMainChain`, driven by bit 0 of the new header's vote bits, moving regular-tree credits and debits of the previous tip between the live and invalidated buckets as in 5.4.
17. **`ExtendMainChain` gate and errors:** `Exist` for a block already recorded at that height with that hash (callers treat it as benign), `Invalid` otherwise; refusal unless `havecfilters == 0x01`.
18. **Compact filter storage:** key = first 16 bytes of the header merkle root; all-zero key for the genesis block when written through `InsertMissingCFilters`; filter bytes verbatim. SPV rescans match against these.
19. **`PruneUnmined` rules** (5.12): expiry test `Expiry <= tipHeight`; ticket price test against `TxOut[0].Value`; votes not on the tip; revocations never (unless expired). Determines which transactions vanish from listings and are no longer rebroadcast.
20. **Maturity and confirmation formulas** (section 6), including the off-by-one in `ticketMatured`.
21. **`UnspentTickets` semantics** (5.22): mined tickets whose output 0 is an unspent wallet credit; plus tickets spent only by an unmined vote for a non-tip block; never unmined tickets. Used to load tickets to watch and to decide what to vote/revoke.
22. **`Spender` results and error kinds** (5.23) are exposed by the gRPC `Spender` call and by the VSP client.
23. **`lasttxsblock` rule** (5.13): moves only when the new block is higher than the main-chain ancestor of the old marker.
24. **Birthday state format** (4.1).
25. **Account resolution for legacy credits:** stored account if the account-exists bit is set, else the account of the first address decoded from the script; failure aborts balance and selection queries.
26. **Timestamps** are stored and returned with one-second resolution.
27. **Unpublished transactions:** their outputs are not UTXOs (excluded from `UnspentOutputs`, `ForEachUnspentOutpoint`, selection, `Spendable`), but they do register their inputs as spent and do count in `Total`/`Unconfirmed`; the marker is cleared when the transaction is mined or removed.
28. **Error kinds and message texts** in 8.2 for every path an RPC can reach.
29. **Multisig record updates** including the flag-byte overwrites of 5.15 (byte parity with existing databases).
30. **Log lines** in 8.4, if log-output parity is in scope for the project (operators grep for the info-level lines).
31. **`Rollback` result shape** (5.8): only blocks with wallet transactions appear as keys; dropped coinbase-like transactions are included. The wallet builds its reorg notifications from this map.
32. **Record fields in query results:** `TxDetails.TxRecord` and `Ticket.TxRecord` always have `Unpublished == false` and `SerializedTx == nil`; only `UnminedTxs` reports the unpublished flag (consumed at `wallet/wallet.go:4374`). A port that "helpfully" fills `Unpublished` in details would change which transactions callers treat as published.
33. **Wallet-authored transactions** are recorded with `InsertMemPoolTx` followed by `AddCredit(block = nil)` in the same database transaction (`wallet/createtx.go:251-299`); the explicit panic in `addCredit` (8.3) is the guard for that ordering and must stay unreachable.
34. **Namespace access:** the methods listed in section 3 that open `waddrmgr` do so inside the caller's transaction; a port must not open a second transaction for account or redeem-script lookups, or balance and selection results would not be snapshot-consistent with the transaction store.
35. **`UnspentTickets` order** is ascending raw ticket hash; the wallet's vote/revoke and ticket-watch loops iterate it.
36. **Scope of the "spent by an unmined transaction" exclusion in coin selection** (refines item 6): it applies to mined outputs only. Unmined credits offered at `minConf == 0` are not filtered by the unmined-inputs index (5.20, Q-37). A port that adds the filter changes which inputs zero-confirmation transactions are built from; a port that keeps it must expect `InsertMemPoolTx` to reject the resulting transaction with DoubleSpend "%v conflicts with %v by double spending %v".
37. **`RangeTransactions` argument table** (5.16): in particular `(-1, -1)` reports unmined transactions only, `(0, -1)` reports mined ascending then unmined, `(-1, 0)` reports unmined then mined descending. RPC listing methods choose their bounds on this basis.
38. **`ImportCFiltersV2` stores the supplied filter bytes verbatim** under the merkle-root key, overwriting existing entries, with only the varint-prefix validation of 5.14.

---

## 10. Quirks

Each item states what the Go does, with a citation, and what a port must decide. "Reproduce" means byte parity or an RPC-visible result depends on it.

**Q-1. `IsUnspentOutpoint` is wrong and unused.** The test `if v := ns.NestedReadBucket(bucketUnspent); v != nil` checks that the bucket exists, not that the key is in it, so the method returns true for any outpoint that is not unpublished and not spent by an unmined transaction (`wallet/udb/txmined.go:2630-2650`). No caller exists. Port: omit, or implement the documented intent; nothing depends on it.

**Q-2. `PreviousPkScripts` cannot work for unmined records and is unused.** With `block == nil` it passes a 32-byte key where a 68-byte tx-record key is required (IO error plus a stack dump for any unmined previous credit), and then dereferences the nil block when building the record key (`wallet/udb/txquery.go:542-606`). Only the disabled tests call it. Port: omit.

**Q-3. Nil-slice panics instead of errors.** `InsertMinedTx` and `GetMainChainBlockHashes` index the block record at the header's height without a nil check (`txmined.go:1223-1224`, `990-991`). Headers of detached blocks are never deleted, so a stale header whose height is above the current tip triggers a panic rather than "not in main chain". `BlockInMainChain` handles the same case correctly. Port: return the Invalid error the next line would have produced.

**Q-4. `InsertMinedTx` does work before its duplicate check and mixes type sources.** Inputs are debited and `bal` rewritten before the "record already exists" early return (`txmined.go:1252-1327`). The transaction type is recomputed with `DetermineTxType`, while `AddCredit`, `AddTicketCommitment`, `RedeemTicketCommitments` and the double-spend exception in `InsertMemPoolTx` trust `rec.TxType`. Records built by `NewTxRecord*` always agree; a port should compute the type once in the record constructor.

**Q-5. `Rollback` ignores the invalidated buckets.** For a detached block whose regular tree was stake-invalidated, credits and debits live in `ic`/`id`. `Rollback` only looks in `c`/`d` (`txmined.go:2018`, `2104`), so the block's regular transactions return to unmined with no unmined credits and with their debits unrestored, and the `ic`/`id` entries are orphaned forever. Balances omit those outputs until the transaction is mined again (the wallet then re-adds credits). Reproduce for byte parity, or fix and document the divergence (decision D-3).

**Q-6. The hasExpiry flag is lost when an unmined transaction is mined.** `moveMinedTx` never sets `cred.hasExpiry`, so the mined credit is written with bit 6 clear even if the unmined credit had it set (`txmined.go:1133-1181`); a later `Rollback` copies the cleared flag back. A transaction inserted directly as mined keeps the flag. `HasExpiry` has no consumer outside this package at the pin, so the only observable effect is the stored byte. Reproduce for byte parity.

**Q-7. `moveMinedTx` ignores the caller's record contents.** The mined record value is the unmined value verbatim (`txmined.go:1070`), so `rec.Received` and `rec.SerializedTx` of the mined notification are discarded. Its debit branch (`txmined.go:1095-1124`) is dead because `InsertMinedTx` has already consumed the unspent entries. Reproduce the first; the second may be dropped provided the unmined-inputs deletes remain.

**Q-8. Three different tests exempt ticket amounts from `bal`.** Opcode tag (`txmined.go:1113`, `1191`, `1550`, `2071`), input position by transaction type (`txmined.go:1302-1303`), and `SStx && i == 0` (`txmined.go:2148`). They coincide for consensus-valid transactions.

**Q-9. Block metadata in query results is mostly empty.** `outputCreditInfo` declares `blockTime` and never assigns it, so `Credit.BlockMeta.Time` is always the zero time and `VoteBits` 0 (`txmined.go:2271`, `2331-2336`). `TxDetails.Block.VoteBits` is never set either (`txquery.go:71`, `443-448`). Reproduce (callers that need the time read `TxDetails.Block.Time` or the header).

**Q-10. Unmined credit records are under-populated.** The unmined credit iterator never sets `OpCode` or `IsCoinbase` (`wallet/udb/txdb.go:1619-1636`), so `TxDetails.Credits` of an unmined transaction carry `OpCode == 0` (not `opNonstake`) and `IsCoinbase == false`. `outputCreditInfo` likewise leaves `FromCoinBase` false for unmined credits (`txmined.go:2275-2300`). Consumers at the pin: `Credit.FromCoinBase` is read by the wallet and the JSON-RPC server (`wallet/createtx.go:1794`, `1888`, `wallet/wallet.go:3515`, `internal/rpc/jsonrpc/methods.go:2792`), so that behaviour is observable and must be reproduced. No code outside `wallet/udb` reads `CreditRecord.OpCode` or `CreditRecord.IsCoinbase` (repository search at the pin), so for those two fields the under-population matters only if the port exposes them.

**Q-11. Balance pass 3 drops iterator errors.** The loop tests `it.err` only inside the body, but `next()` returns false when it sets `err` (`txmined.go:3519-3542`, `txdb.go:2124-2159`). A malformed unspent-commitment or missing commitment entry silently truncates `LockedByTickets`. Port: decide between parity and surfacing the error (D-4).

**Q-12. Stake invalidation of a block containing a wallet spend chain leaves a dangling unspent entry.** In `stakeInvalidate`, if transaction A (credit) and transaction B (spends A's credit) are both regular transactions of the invalidated block and A precedes B in the block record, A's spent credit is moved to `ic` and `bal` is decremented a second time for it, then B's debit loop finds no credit to unspend but still writes an unspent entry for A's output and increments `bal` (`txmined.go:815-881`). The balance nets out, but the unspent entry now points at a credit that is not in `c`; `UnspentOutputs` then fails with IO "missing credit for outpoint", `balanceFullScan` with IO "missing credit for unspent output", and `ForEachUnspentOutpoint` panics, until the block is re-validated or rolled back. This is derived from reading the code; no upstream test covers it (the existing test has the two transactions in different blocks). Needs an oracle test before deciding (D-3).

**Q-13. "Coinbase" includes treasurybase and treasury spends.** `IsEitherCoinBaseTx` accepts any single-input transaction with a null previous outpoint (`internal/compat/compat.go`, `dcrd/blockchain/standalone/v2@v2.3.0/tx.go:49-102`). Consequences: treasury-spend payouts to the wallet are flagged coinbase and subject to coinbase maturity through that flag as well as through the `OP_TGEN` tag; `Rollback` drops treasury spends instead of returning them to unmined; and, because `InsertMemPoolTx` registers the null outpoint in unmined inputs for any non-vote transaction, a second unmined treasury spend relevant to the wallet is rejected with DoubleSpend against the first (`txunmined.go:43-83`, `103-114`). The last consequence is derived, not tested upstream.

**Q-14. `Rollback` equates "amount 0" with "credit already removed".** After unspending a credit it skips the balance and unspent restoration when the returned amount is 0 (`txmined.go:2061-2063`), which also skips genuine zero-value credits. The wallet does not create zero-value credits (`wallet/chainntfns.go:568-572`), so this is latent. The coinbase branch caches a database slice without copying (`txmined.go:1966`); a port must copy.

**Q-15. Genesis cfilter key.** `InsertMissingCFilters` leaves the key all-zero for the genesis block (`txmined.go:447-449`, `473`), whereas `ExtendMainChain` and `ImportCFiltersV2` always use the merkle root prefix. Reproduce.

**Q-16. cfilter and chain-extension checks are looser than documented.** `GetMainChainCFilters` does not verify that the start block is in the main chain although its comment says it errors (`txmined.go:499-515`). `ExtendMainChain` requires `havecfilters == 1` exactly while the other readers accept any non-zero byte (`txmined.go:208-209` vs `402`, `433`). `ExtendMainChain` trusts the caller's `blockHash`. `InsertMissingCFilters` does not check main-chain membership.

**Q-17. Dead `OP_TADD` branches.** `getStakeOpCode` never returns `OP_TADD` and the stored tag cannot decode to it, yet `MakeInputSource` tests for it twice (`txmined.go:3152-3153`, `3253-3254`). Treasury-add change is tagged `OP_SSTXCHANGE`. No effect.

**Q-18. `UnminedTxs` order depends on Go map iteration.** Roots and independent subtrees come out in randomized order; the "skip duplicate edges" test compares an out-edge with the parent hash and never matches (`wallet/udb/kahnsort.go:42-47`, `75-83`). Callers rely only on parents preceding children. Port: any deterministic topological order is acceptable (for example ascending hash among ready nodes); record the choice.

**Q-19. `UnspentTickets` never returns unmined tickets.** The first test requires an entry in the unspent bucket, which unmined tickets do not have, so `includeImmature` only adds mined-but-immature tickets; the comment at `txmined.go:2700-2701` suggests otherwise. Tickets reached through the "unmined vote on a non-tip block" path skip the maturity test (`txmined.go:2676-2698`, `2717`). Reproduce.

**Q-20. Unmined spenders of mined tickets are not reported by ticket queries.** When the ticket's output 0 is a wallet credit, `TicketIterator.Next` and `TicketDetails` look only at the credit's mined-spent bit (`txmined.go:2800-2817`, `txquery.go:221-234`); an unmined vote or revocation is reported only when output 0 is not a wallet credit. Reproduce: ticket status in RPC results is derived from this.

**Q-21. `MakeInputSource` oddities.** The deferred cursor cleanup refers to cursors that are never created (`txmined.go:3046-3058`). The random phase is not account-aware, so in multi-account wallets many tries are wasted (and counted). Any account-resolution error aborts selection for every account. P2SH and alternate-signature credits are logged at error level on every encounter and never selected. Zero-value outputs are skipped for mined credits only. The returned slices are shared between successive `InputDetail` results.

**Q-22. Multisig record handling.** Spending overwrites the flag byte with `0x01` and unspending with `0x00`, so the stake-tree bit is lost after the first spend (`wallet/udb/txdb.go:1896-1910`); unspending zeroes 31 of the 32 spender-hash bytes; the two length-check errors in `fetchMultisigOut` pass a format string to `errors.E` without formatting (`txdb.go:1809-1814`); `AddMultisigOut` for an unmined record over an already-mined entry silently resets it (`txmined.go:1768-1780`). Reproduce the stored bytes.

**Q-23. Unpublished marker handling.** A repeated `InsertMemPoolTx` does not update the marker (`txunmined.go:31-34`); `SetPublished` does not check existence; `Rollback` does not restore the marker, so a rolled-back transaction is treated as published.

**Q-24. `PruneUnmined` return value is incomplete,** and its expiry comparison is `Expiry <= tipHeight` (`txunmined.go:366`), not `tipHeight + 1`. Reproduce both.

**Q-25. `AddCredit` on an invalidated block overwrites.** No existence check against `ic` or `c` (`txmined.go:1400-1428`); repeated calls rewrite the same value.

**Q-26. `spendCredit` fabricates a credit when none exists.** A missing credit becomes a 94-byte zero-amount, spent value (`wallet/udb/txdb.go:942-954`). Reachable only through inconsistent state: a spender recorded in a block below its funding transaction's, whose block is rolled back, then the spender's block disapproved and approved again (QK-0101, confirmed by the oracle on 2026-11-23). *Corrected 2026-11-23 (ADR-0012 Rule 12):* Q-12 followed by re-validation does not reach it when A precedes B, as Q-12 states: `stakeValidate` handles each transaction's credits before its debits, so A's credit is back in `c` when B's debit loop spends it (QK-0735; the oracle run of Q-12's sequence, rolled back and approved again, ends with both sides' rows equal).

**Q-27. `UnspentOutput` ignores unmined spends,** and `outputCreditInfo` prefers an unmined credit over a mined one even when a block is supplied (`txmined.go:2248-2256`, `2537-2553`). Reproduce.

**Q-28. `RangeBlocks` discards the iterator error** (`txquery.go:763-772`). A malformed block record ends the walk silently.

**Q-29. `UnspentOutputs` documents an undefined order** (`txmined.go:2461-2462`) but is deterministic; treat the order as contract (9.4).

**Q-30. Time values.** `time.Unix(sec, 0)` yields local-zone times with second resolution; a zero `time.Time` round-trips through the birthday state as year-1 UTC seconds reinterpreted in the local zone (equal instant, different representation). `received` times of the zero `time.Time` are stored as `0xFFFFFFF1886E0900`.

**Q-31. Random selection depends on bbolt cursor edge behaviour** (5.1 item 3): `Seek` past the end followed by `Prev` yields the last key. A storage layer where that returns nothing would bias selection.

**Q-32. Go-runtime dependencies in results:** `AccountBalances` and `Rollback` return Go maps (unordered); `UnminedTxs` uses map iteration (Q-18). Callers that need an order sort themselves.

**Q-33. `SelectInputs(0)` followed by a non-zero target on the same source can return duplicate inputs.** After a `target == 0` call has enumerated and drained `remainingKeys`, a later call with `minConf != 0`, a non-zero target above the accumulated total and `randTries < numUnspent / 2` re-enters the random phase; `skip` consults only `seen`, which does not contain keys consumed from `remainingKeys`, so `randomUTXO` can return an outpoint that is already in `Inputs` (`txmined.go:3082-3110`, `3172-3181`). Derived from the code; no caller at the pin mixes the two modes on one source (`wallet/createtx.go`, `wallet/utxos.go`). Port: either reproduce or insert consumed keys into `seen`; no observable difference for existing callers.

**Q-34. Upstream TODO markers that describe intended but absent behaviour** (none changes what the code does; listed so a port does not implement the intent by mistake): `Credit.PkScript` has no script version (`txmined.go:167`); the `InsertMissingCFilters` comment names database "version TODO" (`425`); `GetBlockMetaForHash`/`BlockMeta` are called legacy (`955`); accounts are not back-filled for legacy credits (`1022-1025`); `moveMinedTx` should move the raw value (`1145-1148`); `InsertMinedTx` rewrites `bal` even when unchanged (`1315-1317`); `AddCredit` "should not be necessary" (`1381-1383`); `Rollback` should use a credit iterator (`2102`); cursor deletion in `Rollback` is avoided "until bolt issue #620 is resolved" (`2186-2190`), which is why block records are deleted after the walk. There are no `XXX`, `BUG`, `FIXME` or `Deprecated` markers in `txmined.go` or `txquery.go`. The only marker in the other covered files is in `InsertMemPoolTx`: "increment credit amount for each credit (but those are unknown here currently)" (`txunmined.go:130-131`); nothing is incremented, and `kahnsort.go` and `cfilters.go` have none.

**Q-35. Doc comments that disagree with the code,** beyond those already listed: `UnspentTickets` and `ForEachUnspentOutpoint` say the order is undefined (both are key-ordered, `txmined.go:2555-2556`, `2652-2655`); `Spender` documents error codes `ErrInput`/`ErrNoExist` that no longer exist (the kinds are `Invalid`/`NotExist`, `txquery.go:637-639`); `rangeBlockTransactions` says reverse order applies "when end > begin" (it is forward then, `txquery.go:384-387`); `RangeTransactions` says unmined transactions are processed first when the end height precedes the begin height; more precisely they are processed first exactly when `begin < 0` and not at all for a reverse range with `begin >= 0` (`txquery.go:508-512`, `525`); `stakeInvalidate` says it makes no changes "when the block is not marked invalid" (it makes none when the block **is** already marked invalid, `txmined.go:757-761`); `AccountBalance`/`AccountBalances` mention a `syncHeight` parameter that does not exist (the tip height is read from the database, `txmined.go:3559-3582`); `TicketDetails` says a missing ticket yields nil, but the test is whether the supplied transaction is a ticket (`txquery.go:206-215`); `BlockMeta` says its only metadata is the block time although it also carries vote bits (`txmined.go:49-56`).

**Q-36. `AddMultisigOut` builds a dummy block for unmined outputs** with `Time = rec.Received` and height 0 (`txmined.go:1776-1780`); only the zero hash and height 0 are persisted, so an unmined multisig record is indistinguishable from one "mined" in a block with an all-zero hash, and "is mined" is tested everywhere as "stored block hash is non-zero". `M` and `N` are stored as the low 8 bits of `RequiredSigs` and `NumPubKeys` (`txmined.go:1812-1813`).

**Q-37. Zero-confirmation coin selection can return an unmined credit that is already spent by another unmined transaction.** `MakeInputSource` filters mined outputs through `skip` (unmined-inputs index), but the unmined-credit enumeration used when `minConf == 0` has no such test (`txmined.go:3135-3168`, `3277-3309`), while `UnspentOutputs`, `ForEachUnspentOutpoint` and `balanceFullScan` all skip such credits (`2500-2504`, `2597-2601`, `3459-3461`). The wallet's `ignore` callbacks exclude only locked outpoints (`wallet/createtx.go:88-91`, `396-399`, `1026-1029`) and `wallet/utxos.go:42` passes nil. A transaction built from such an input is rejected when it is recorded (`InsertMemPoolTx`, DoubleSpend). Derived from reading the code; no upstream test exercises `MakeInputSource`. Port: reproduce, or add the filter and record the divergence (D-6).

**Q-38. `RemoveUnconfirmed` handles the multisig record differently from every other writer.** It reads the stored value with the non-padding accessor and copies exactly its stored length (`txunmined.go:259-263`), whereas `Rollback`, `AddMultisigOut` and `SpendMultisigOut` work on a 135-byte zero-padded copy (`wallet/udb/txdb.go:1936-1944`). For a record of the normal 135 bytes the rewritten value is identical; a shorter stored record would make the unspend helper index out of range here and would be silently padded to 135 bytes by the other paths.

---

## 11. Upstream tests

| File | Lines | Compiled? | What it pins |
|---|---|---|---|
| `wallet/udb/tx_test.go` | 574 | yes | `TestInsertsCreditsDebitsRollbacks`: nine steps on a testnet3 store: (1) empty store; (2) unmined insert plus unmined credit; (3) duplicate unmined insert; (4) mined insert of the same transaction (the unmined to mined move, no `AddCredit` call); (5) rollback of its block (back to unmined with an unmined credit); (6) chain re-extended and the transaction mined again; (7) a mined spender of that credit inserted twice (the second insert is a no-op); (8) rollback of the spender's block (spender returns to unmined, the restored credit stays hidden as spent-by-unmined); (9) unmined insert of a transaction spending an output the store does not know. After each step it checks `AccountBalance(minConf=1).Spendable`, `.Unconfirmed`, the exact set of `UnspentOutputs` outpoints (including `Tree`) and the set of `UnminedTxs` hashes. `TestCoinbases`: coinbase credit maturity at testnet3 `CoinbaseMaturity = 16` across chain extensions, a spend, and two rollbacks. Note the helper compares `ImmatureCoinbaseRewards` twice and never asserts `Spendable` (`tx_test.go:415-423`). |
| `wallet/udb/stakevalidation_test.go` | 187 | yes | `TestStakeInvalidationOfTip`: two chained transactions in consecutive blocks; a third header with vote bits 0 invalidates block 2; checks `AccountBalance(1, 0).Total` and `UnspentOutputs` before and after. Does not cover re-validation, same-block chains (Q-12) or rollback of an invalidated block (Q-5). |
| `wallet/udb/txmined_test.go` | 80 | yes | `TestSetBirthState`: round trip of a random birthday state. |
| `wallet/udb/txcommon_test.go` | 103 | yes | Helpers: header generator chained from the testnet3 genesis (headers are otherwise zero), `emptyFilters` (`FromBytesV2(B, M, nil)`), `makeBlockMeta`. |
| `wallet/udb/common_test.go` | 97 | yes | `cloneDB`: copies an empty, upgraded testnet3 database per test. |
| `wallet/udb/txquery_test.go` | 759 | **no** (`//go:build ignore`) | Written against an obsolete API (`s.InsertTx`, `s.Rollback(height)` without a transaction). Pins nothing at this pin, but its scenario list (`TestStoreQueries`, `TestPreviousPkScripts`) is a useful checklist for `TxDetails`/`RangeTransactions`. |
| `wallet/udb/txexample_test.go` | 229 | **no** (`//go:build ignore`) | Obsolete examples. |
| `wallet/udb/upgrades_test.go` + `wallet/udb/testdata/v*.db.gz` | | yes | Database upgrade tests (owned by the upgrade spec). The fixture databases contain real historical `wtxmgr` contents and are useful read-only inputs for query tests. |

The ignored `txexample_test.go` contains `ExampleStore_Rollback` and `Example_basicUsage`. No test outside `wallet/udb` constructs or drives a `udb.Store` directly (repository search of `*_test.go` for `txStore`, `udb.Store`, `udb.NewTxRecord`, `udb.Open` at the pin matches only `wallet/udb/common_test.go`); the remaining `wallet/udb` test files (`addressmanager_test.go`, `cointype_test.go`, `internal_test.go`, `vsp_test.go`) do not touch the transaction store.

**Vectors to port directly:** the four compiled tests above (`TestInsertsCreditsDebitsRollbacks`, `TestCoinbases`, `TestStakeInvalidationOfTip`, `TestSetBirthState`, in three files) translate one-to-one; their inputs are constructed in code and need no fixture files.

**Vectors to generate through a Go oracle shim** (a small Go program linked against dcrwallet `54b85b33` that applies a scripted sequence of store calls to a fresh database and dumps every `wtxmgr` bucket as sorted hex key/value pairs, plus the results of the query methods):

1. Mined insert / unmined insert / move / rollback sequences for each transaction type (regular, ticket, vote, revocation, coinbase, treasury spend), dumping all buckets after each step. Pins byte parity for 5.5-5.11 including Q-6 and Q-7.
2. Stake invalidation and re-validation, including a same-block spend chain in both record orders (Q-12) and rollback of an invalidated block (Q-5).
3. Balance matrix: one credit of each tag (nonstake, coinbase, `OP_SSTX`, `OP_SSGEN`, `OP_SSRTX`, `OP_SSTXCHANGE`, `OP_TGEN`), mined at heights straddling each maturity boundary and unmined (published and unpublished), queried at `minConf` 0, 1, 2; plus ticket commitments in each state of the 5.9 table.
4. `RangeTransactions` over (begin, end) in {(0,-1), (-1,0), (h,h), (h1,h2), (h2,h1), (-1,-1)} with empty blocks interleaved; `TxDetails` for mined and unmined records with spent/unspent credits.
5. `UnspentTickets`, `IterateTickets`, `TicketDetails` for tickets that are unmined, immature, live, voted (mined), voted by an unmined vote on the tip and on a non-tip block, revoked, and removed.
6. `Spender` for each branch of 5.23.
7. `PruneUnmined` with expired, wrong-price ticket, stale vote and descendants.
8. `InsertMemPoolTx` conflict cases including the stale-vote eviction.
9. Multisig add/spend/rollback sequences (flag byte evolution).
10. cfilter insertion paths (`ExtendMainChain`, `InsertMissingCFilters` from genesis, `ImportCFiltersV2`).
11. Coin selection eligibility: for a fixed store, the *set* of outpoints returned by `SelectInputs(0)` and by `SelectInputs(MaxAmount)` per account and `minConf`, compared as sets; plus a statistical check that order is not key order.
12. Zero-confirmation selection with an unmined credit that another unmined transaction spends (Q-37): whether `SelectInputs(0)` at `minConf = 0` returns it, and the error `InsertMemPoolTx` gives for a transaction built from it.
13. `ImportCFiltersV2` with an empty blob, a blob with a truncated varint, a start height of 0 and a start height above the tip; dump the `cf` bucket.

**Untested upstream:** everything in `txquery.go`; `MakeInputSource`/`RandomUTXO`; ticket commitments and `LockedByTickets`; ticket queries; multisig; cfilter maintenance; `UpdateProcessedTxsBlockMarker`; `PruneUnmined`; `Spender`; votes, revocations and tickets in general (the compiled tests use only untyped regular transactions and one coinbase); re-validation; all error paths.

---

## 12. Rust porting notes

### 12.1 Suggested boundary

One crate, working name `purse-txstore`, layered on the wallet database crate (the bbolt-compatible bucket/cursor layer) and on the formats module specified in `03-txstore-formats.md`:

| Module | Contents |
|---|---|
| `types` | `Block`, `BlockMeta`, `TxRecord`, `Credit`, `CreditRecord`, `DebitRecord`, `TxDetails`, `Ticket`, `TicketDetails`, `MultisigOut`, `MultisigCredit`, `Balances`, `BirthdayState`, `BlockCFilter` |
| `chain` | `main_chain_tip`, `extend_main_chain`, stake (in)validation, `rollback`, header/block accessors, processed-txs marker, birthday state |
| `cfilters` | cfilter read/insert/import |
| `insert` | `insert_mined_tx`, `move_mined_tx`, `insert_mempool_tx`, `add_credit`, ticket commitments, multisig |
| `unmined` | `remove_unconfirmed`, `prune_unmined`, `unmined_txs` + dependency sort, publish flag |
| `utxo` | `unspent_outputs`, `unspent_output`, `for_each_unspent_outpoint`, random selection, `InputSource` |
| `balance` | full-scan balances |
| `query` | details, ranges, tickets, spender |

The store should be a thin struct holding chain parameters and a handle to the address manager's two lookups (`addr_account`, `redeem_script_for_hash160`) expressed as a trait, so the crate does not depend on the address manager implementation and can be tested with a stub.

### 12.2 Reuse (each item was located in the sister repositories)

| Need | Candidate |
|---|---|
| Transaction (de)serialization, hashes, `TxIn` defaults, expiry/tree constants | `dcroxide:crates/dcroxide-wire/src/msgtx.rs` (`MsgTx::from_bytes`, `serialize`, `tx_hash`, `NO_EXPIRY_VALUE`, `TX_TREE_STAKE`, `MAX_TX_IN_SEQUENCE_NUM`, `NULL_BLOCK_INDEX`). No `pk_script_locs` equivalent was found; it must be added (formula in `dcrd/wire@v1.8.0/msgtx.go:1125-1157`). |
| Block header (de)serialization and hash | `dcroxide:crates/dcroxide-wire/src/blockheader.rs` |
| Stake transaction classification and field extraction | `dcroxide:crates/dcroxide-stake/src/lib.rs` (`determine_tx_type`, `is_sstx`, `is_ssgen`, `ssgen_block_voted_on`, `amount_from_sstx_pk_scr_commitment`) |
| Coinbase test | `dcroxide:crates/dcroxide-standalone/src/tx.rs` (`is_coin_base_tx`); the either-rule wrapper is two calls |
| Script classification, address extraction, multisig details, tokenizer | `dcroxide:crates/dcroxide-txscript/src/stdscript.rs` (`determine_script_type`, `extract_addrs`, `extract_multi_sig_script_details_v0`), `dcroxide:crates/dcroxide-txscript/src/tokenizer.rs` |
| Compact filters | `dcroxide:crates/dcroxide-gcs/src/lib.rs` (`FilterV2::from_bytes`, `bytes`), `dcroxide:crates/dcroxide-gcs/src/blockcf2.rs` (`key`, `B`, `M`) |
| Chain parameters (maturities, genesis hash) | `dcroxide:crates/dcroxide-chaincfg/src/lib.rs` |
| CSPRNG with `int_n` and `shuffle_slice` | `dcroxide:crates/dcroxide-crypto/src/rand.rs` |
| Hash type | `dcroxide:crates/dcroxide-chainhash` |

`dcr-rs` (`dcr-rs:src/tx.rs`) has its own `MsgTx` with `parse_full`/`serialize_full`/`tx_hash`; using the dcroxide wire types throughout avoids two transaction representations. Nothing else in `dcr-rs` is needed by this subsystem.

`dcroxide-database` is the full node's block store and is not a substitute for the bbolt-compatible wallet database.

### 12.3 Hazards

1. **Aliasing.** The Go relies on values staying readable after sibling keys in the same bucket are modified. In Rust, copy every value before mutating the bucket; the borrow checker will force most of this, but the "removed credits" cache in `Rollback` and the read-modify-write helpers need explicit owned buffers.
2. **Cursor mutation.** `Rollback` reads block records with a cursor while writing other buckets and defers block-record deletion until after iteration; keep that structure. Stake (in)validation deletes from and inserts into `c`/`ic` while iterating a decoded, owned copy of the block record, not a cursor.
3. **Signed/unsigned casts.** Heights are `int32` in memory and `uint32` big-endian on disk; `-1` round-trips as `0xFFFFFFFF`. `curHeight - txHeight + 1` is evaluated in `int32`; use wrapping arithmetic only if parity with overflow is wanted (not reachable with real heights). Amounts are `int64` stored as `uint64`; `bal` arithmetic can transiently go negative within an operation and must be done in `i64`.
4. **Nil versus empty.** `remainingKeys == nil` (not yet enumerated) versus empty (enumerated, exhausted) is a real state distinction in `SelectInputs`; model it with `Option<VecDeque<_>>`. `PruneUnmined` returns a non-nil empty slice; `UnspentOutputs`, `UnminedTxHashes`, `Credits`, `Debits` are nil when empty in Go. Whether that distinction reaches JSON is a matter for the RPC specs; keep the information (for example `Vec` plus the RPC layer's own rule) rather than guessing here.
5. **Panics.** Decide per 8.3 entry; the recommendation is to return IO/Invalid errors for all nil-slice cases and keep explicit panics only where the Go has an explicit `panic` (D-2).
6. **Recursion depth.** `RemoveUnconfirmed` recurses along unmined spend chains; convert to an explicit stack while keeping the same visit order (post-order over outputs ascending), because the order determines which deletes and commitment rewrites happen first.
7. **Randomness in tests.** Inject the RNG behind a trait so selection tests can be deterministic; production must use the CSPRNG.
8. **Key order.** Any in-memory index or alternative storage must iterate in raw-byte key order (5.1). Hash display order is the reverse of key order.
9. **Log formatting.** `%v` of a hash value and of a pointer to a hash both print the reversed hex string; `%v` of `dcrutil.Amount` prints `"<n> DCR"` formatting; `%x` of a credit key prints lowercase hex; the unhandled-opcode warning prints a Go byte-slice (`[1 2 3]`).

### 12.4 Size estimate

**L.** About 4,900 lines of Go in scope (3,583 + 773 + 410 + 116 + 67), nearly all of it state-mutating logic over eighteen buckets and five root keys with implicit invariants, plus a dense set of order- and byte-sensitive behaviours. The algorithms themselves are straightforward once specified; the cost is in the verification: upstream compiles only four tests for this scope, so byte parity has to be established with an oracle harness (section 11) covering ticket, vote, revocation, invalidation and rollback sequences. Expect the oracle shim and differential test corpus to be comparable in effort to the implementation.

---

## 13. Open questions

1. **D-1: scope of dead code.** `IsUnspentOutpoint`, `PreviousPkScripts`, the debit branch of `moveMinedTx`, the `OP_TADD` branches and the unreachable error paths have no production effect. Recommendation: do not port Q-1 and Q-2; confirm no out-of-tree consumer of the Go package API matters (the Rust project exposes RPC, not the Go API). *Decided (project owner, 2026-11-24) for `IsUnspentOutpoint` and `PreviousPkScripts`: both stay ported as Go behaves, ported with `purse-udb`'s credit slice, QK-0088 and QK-0089 reproduced; nothing in dcrwallet calls either at the pin, and the question is revisited when a caller exists (crate `txstore` of `purse-udb`, choice 6).*
2. **D-2: panics.** Whether the port keeps process-aborting behaviour for the 8.3 conditions or returns errors. Returning errors changes observable behaviour only for corrupted databases or caller bugs.
3. **D-3: stake-invalidation defects (Q-5, Q-12).** Reproduce for byte parity with a Go-written database in the same state, or fix. Both are derived from reading the code; an oracle run (section 11, item 2) must confirm the exact post-state before deciding. If fixed, a database last written by the Go implementation in the broken state still has to be readable.
4. **D-4: swallowed errors (Q-11, Q-28).** Parity or surface them.
5. **D-5: deterministic order for `UnminedTxs` (Q-18).** Pick and document one; verify no RPC result exposes the Go order in a way clients compare across runs.
6. **UNVERIFIED: treasury-spend double-spend rejection (Q-13).** Derived from `InsertMemPoolTx`; settle with an oracle test inserting two unmined treasury spends that pay the wallet, and by checking whether the wallet layer ever passes unmined treasury spends to the store (`wallet/chainntfns.go` mempool path).
7. **UNVERIFIED: whether any RPC or gRPC result exposes `Credit.HasExpiry` or `CreditRecord.HasExpiry`.** A search of the repository at the pin found no consumer outside `wallet/udb`; if confirmed, Q-6 matters only for stored bytes.
8. **UNVERIFIED: error-kind to RPC-code mapping** for the kinds in 8.1 is defined in the RPC server packages and must be cross-checked by the RPC specs against the message list in 8.2.
9. **Storage engine.** The random-selection and range algorithms assume bbolt cursor semantics (5.1). If the database layer is not a faithful bbolt reimplementation, the cursor contract in 5.1 must be added to its test suite.
10. **`KeyN` cost.** `MakeInputSource` and `UnspentOutputCount` call bbolt `Stats()`, a full bucket walk. A port may cache or count differently as long as the value equals the key count bbolt would report at source-creation time (exact in a read-only transaction; see the read-write caveat in 5.1 item 6).
11. **Legacy credits without a stored account.** How common they are in real databases decides how much the slow path in 5.2 matters; the historical fixtures under `wallet/udb/testdata` can be inspected by the formats/upgrade work to answer this.
12. **UNVERIFIED: duplicate inputs after mixed-mode selection (Q-33).** Derived by reading `MakeInputSource`; an oracle test calling `SelectInputs(0)` then `SelectInputs(n)` on one source with `minConf = 1` would settle whether duplicates actually appear.
13. **D-6 / UNVERIFIED: unmined credits spent by unmined transactions in zero-confirmation selection (Q-37).** Derived by reading `MakeInputSource`; settle with the oracle case in section 11 (item 12), then decide between parity and adding the unmined-inputs filter. Adding it cannot change stored bytes; it changes only which inputs are offered.

