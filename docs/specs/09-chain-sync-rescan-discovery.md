# 09 — Wallet-side chain processing: block/header ingestion, sidechains, rescans, address discovery, difficulty validation

## Header

**Scope (dcrwallet files, line counts at the parity pin)**

| File | Lines | Contents |
| --- | --- | --- |
| `wallet/chainntfns.go` | 933 | `ChainSwitch`, `extendMainChain`, `AddTransaction`, `processTransactionRecord`, `VoteOnOwnedTickets`, `RevokeOwnedTickets` |
| `wallet/rescan.go` | 581 | `RescanFilter`, `rescan` and its public wrappers, `RescanPoint`, birthday state |
| `wallet/discovery.go` | 979 | account and address discovery over cfilters/blocks and over dcrd's exists-address index |
| `wallet/sidechains.go` | 465 | `SidechainForest`, `BlockNode`, `EvaluateBestChain` |
| `wallet/difficulty.go` | 912 | PoW (EMA and ASERT) and PoS (DCP0001) difficulty calculation and header-chain validation |
| `wallet/checkpoints.go` | 69 | the single hard-coded checkpoint and its enforcement at open |
| `validate/validate.go` | 130 | context-free merkle root, cfilter commitment and pre-DCP0005 cfilter set checks |
| **Total** | **4069** | |

Adjacent code read because the scope cannot be specified without it (owned by other specs, cited here for
contract only): `wallet/wallet.go:883-1201` (`watchHDAddrs`, `LoadActiveDataFilters`), `wallet/wallet.go:5361-5439`
(testnet target, deployment map, checkpoint rollback at open), `wallet/addresses.go:718-746,1004-1038`,
`wallet/network.go`, `deployments/deployments.go`, `spv/rescan.go`, `spv/backend.go:63-92,410-432,524-620`,
`spv/sync.go:1293-1345,1405-1570,1848-1900`, `chain/sync.go:225-460,817-923`, `chain/backend.go`,
`wallet/udb/txmined.go:181-543,885-1012,1867-2242`.

**Parity pin**

| Component | Version |
| --- | --- |
| dcrwallet | master `54b85b33` (2.2.0-pre, module `decred.org/dcrwallet/v5`, `go 1.26.0`) |
| `dcrd/blockchain/standalone/v2` | v2.3.0 |
| `dcrd/blockchain/stake/v5` | v5.0.3 |
| `dcrd/chaincfg/v3` | v3.3.0 |
| `dcrd/gcs/v4` (and `blockcf2`) | v4.1.1 |
| `dcrd/hdkeychain/v3` | v3.1.3 |
| `dcrd/txscript/v4` (`stdaddr`, `stdscript`) | v4.1.2 |
| `dcrd/wire` | v1.8.0 |
| `dcrd/dcrutil/v4` | v4.0.3 |
| `dcrd/crypto/blake256` | v1.1.0 |
| `dcrd/crypto/rand` | v1.0.1 |
| `github.com/jrick/bitset` | v1.0.0 |
| `golang.org/x/sync` (`errgroup`) | v0.23.0 |

**Summary**

The wallet does not trust its network backend for chain selection. Every syncer (SPV or dcrd JSON-RPC) hands
the wallet block headers wrapped in `BlockNode`s; the wallet keeps not-yet-main-chain headers in an in-memory
`SidechainForest`, validates height continuity, PoW difficulty (EMA/blake256 or ASERT/blake3, decided
positionally without agenda state), proof of work and DCP0001 stake difficulty itself, picks the chain with
strictly more cumulative work than the current tip, and applies it in one database transaction
(`ChainSwitch`), rolling back and re-attaching blocks, inserting relevant transactions, moving the
"transactions processed through" marker, setting the wallet birthday, and pruning stale unmined
transactions. Relevant transactions are turned into credits, ticket commitments and multisig records by
`processTransactionRecord`, which is also the entry for mempool transactions (`AddTransaction`) and rescans.
Rescans walk the main chain in batches of 2000 blocks from a "rescan point". Account and address discovery
bisects BIP0044 index space in gap-limit sized segments, matching derived payment scripts against stored
version 2 committed filters and then against the exact output scripts of fetched blocks (SPV), or asking
dcrd's exists-address index (RPC). The `validate` package supplies the three context-free checks used on data
from untrusted peers. Voting on winning tickets is orchestrated here; revocation is a no-op since DCP0009.

---

## 1. Role in the system

### 1.1 What it does

1. **Header acceptance.** Given candidate headers, decide whether they form a valid extension or
   reorganization of the wallet's main chain: height continuity, positional PoW difficulty, proof of work,
   stake difficulty (`ValidateHeaderChainDifficulties`), and one hard-coded checkpoint.
2. **Fork bookkeeping.** Hold headers (and, once fetched, their cfilters and relevant transactions) that are
   not in the main chain in a forest of rooted trees, and report the best chain (`EvaluateBestChain`).
3. **Chain switch.** Atomically detach and attach main-chain blocks in the database, record relevant
   transactions, emit notifications, and tell the network backend what new data to watch (`ChainSwitch`).
4. **Transaction ingestion.** Record a mined or unmined transaction's effects on the wallet
   (`processTransactionRecord`), with anti-forgery checks for unmined transactions from untrusted peers
   (`AddTransaction`).
5. **Rescan.** Re-derive wallet transaction history for a range of main-chain blocks through the network
   backend, with progress reporting (`Rescan*`), and compute where a rescan must start (`RescanPoint`).
6. **Discovery.** Find the last used account and the last used child index of each account branch, extend the
   address database past them by the gap limit, and upgrade unused legacy-coin-type wallets to the SLIP0044
   coin type (`DiscoverActiveAddresses`).
7. **Watching.** Keep the network backend's transaction filter loaded with gap-limit lookahead addresses and
   newly created outpoints (`watchHDAddrs`, calls to `NetworkBackend.LoadTxFilter`).
8. **Stake difficulty.** Compute the next ticket price from stored headers (`NextStakeDifficulty*`).
9. **Voting.** Build, sign, publish and record votes for owned winning tickets (`VoteOnOwnedTickets`).
10. **Context-free validation.** Merkle roots, cfilter header commitments and the pre-DCP0005 cfilter set hash
    (`validate` package).

### 1.2 Callers

| Caller | Calls | Source |
| --- | --- | --- |
| SPV syncer, initial header sync | `NewBlockNode`, `BadCheckpoint`, `(*BlockNode).BadCheckpoint`, `FullSideChain`, `ValidateHeaderChainDifficulties`, `AddBlockNode`, `EvaluateBestChain`, `PruneAll`, `validate.CFilterV2HeaderCommitment` | `spv/backend.go:190-392` |
| SPV syncer, initial sync chain-switch stage | `ChainSwitch` (possibly on a suffix `bestChain[tipIndex+1:]` of the tree's best chain, so `chain[0]` need not be a forest root), `AddBlockNode` for the returned detached nodes | `spv/sync.go:1773-1835` |
| SPV syncer, block announcements | `HasSideChainBlock`, `NewBlockNode` (no `BadCheckpoint` call on this path), `FullSideChain`, `ValidateHeaderChainDifficulties`, `AddBlockNode`, `EvaluateBestChain`, `RescanPoint`, `ChainSwitch`, `validate.CFilterV2HeaderCommitment` | `spv/sync.go:1405-1560` |
| SPV syncer, block download | `validate.MerkleRoots`, then `validate.DCP0005MerkleRoot` on failure | `spv/backend.go:67-92` |
| SPV syncer, mempool | `AddTransaction(ctx, tx, nil)` | `spv/sync.go:1158` |
| SPV syncer, post-header sync | `RescanPoint`, `DiscoverActiveAddresses`, `LoadActiveDataFilters`, `RescanProgressFromHeight` | `spv/sync.go:1848-1900` |
| SPV syncer as `NetworkBackend` | holds a `RescanFilter`; `NewRescanFilter`, `AddAddress`, `AddUnspentOutPoint`, `ExistsAddress`, `ExistsUnspentOutPoint` | `spv/backend.go:410-432`, `spv/rescan.go` |
| RPC syncer, `getHeaders` | `NewBlockNode`, `BadCheckpoint`, `AddBlockNode`, `EvaluateBestChain`, `ValidateHeaderChainDifficulties`, `ChainSwitch`, `RescanPoint`, `DiscoverActiveAddresses`, `LoadActiveDataFilters`, `RescanProgressFromHeight` | `chain/sync.go:225-397` |
| RPC syncer, `blockconnected` | `HasSideChainBlock`, `NewBlockNode`, `AddBlockNode`, `EvaluateBestChain`, `ChainSwitch` (no difficulty validation on this path) | `chain/sync.go:833-908` |
| RPC syncer, `relevanttxaccepted` | `AddTransaction(ctx, tx, nil)` unless manual tickets and the tx is a ticket | `chain/sync.go:914-923` |
| RPC syncer, `winningtickets` | `VoteOnOwnedTickets` | `chain/sync.go:817-823` |
| RPC syncer, cfilter fetch in `getHeaders` and `blockconnected` | `validate.CFilterV2HeaderCommitment` on every cfilter obtained from dcrd before the node is built | `chain/sync.go:258-267,858-869` |
| RPC syncer, `getMissingHeaders` (reconnect catch-up) | for each missing header (fewer than `wire.MaxBlockHeadersPerMsg`; otherwise a full `getHeaders`): one dcrd `rescan` of that single block called on the RPC client directly (not through `Wallet.rescan`), then the `blockconnected` path above with the returned transactions as `RelevantTxs` | `chain/sync.go:400-452` |
| Both syncers, `Run` start | `RescanPoint`, only to log at info `Transactions synced through block <hash> height <n>` — the rescan point's parent when non-nil, else the tip (after `Headers synced through block <hash> height <n>`); an error aborts `Run` | `chain/sync.go:613-630`, `spv/sync.go:341-356` |
| SPV syncer, transaction inventory | `RescanPoint`; while it is non-nil announced transactions are **not fetched at all** (`handleTxInvs` returns); an error is logged at warn and the invs are dropped | `spv/sync.go:1094-1104` |
| `Wallet.CreateVspPayment` | `AddTransaction(ctx, feeTx, nil)` after `SetPublished(feeHash, false)`, to record the signed VSP fee transaction as unmined | `wallet/wallet.go:4665-4762` |
| JSON-RPC `walletinfo`, gRPC `WalletService.BirthBlock` | `BirthState` (§3.9) | `internal/rpc/jsonrpc/methods.go:5531-5538`, `internal/rpc/rpcserver/server.go:628-648` |
| gRPC `WalletService.Rescan` | `RescanProgressFromHeight` | `internal/rpc/rpcserver/server.go:442-489` |
| gRPC import/discover methods | `RescanFromHeight` (in a goroutine), `DiscoverActiveAddresses` | `internal/rpc/rpcserver/server.go:700,725,775,856,863,4306` |
| gRPC `WalletLoaderService.RescanPoint` | `RescanPoint` | `internal/rpc/rpcserver/server.go:3160-3175` |
| JSON-RPC `rescanwallet`, `discoverusage`, `addtransaction`, import methods | `RescanFromHeight`, `DiscoverActiveAddresses`, `AddTransaction` | `internal/rpc/jsonrpc/methods.go:477,851,2062,2120,2172,4240` |
| Ticket buyer, ticket purchase, mixing | `RescanPoint`, `NextStakeDifficulty`, `NextStakeDifficultyAfterHeader` | `ticketbuyer/tb.go:102,154`, `wallet/createtx.go:1209`, `wallet/mixing.go:274`, `wallet/wallet.go:1626` |
| `wallet.Open` | `rollbackInvalidCheckpoints` | `wallet/wallet.go:5434-5436` |
| Wallet creation, gRPC | `SetBirthState`, `BirthState` | `walletsetup.go:255`, `internal/rpc/rpcserver/server.go:631,2749` |
| `Wallet.ValidatePreDCP0005CFilters`, missing-cfilter fetch | `validate.PreDCP0005CFilterHash`, `validate.CFilterV2HeaderCommitment` | `wallet/wallet.go:5230-5268,1381` |

`SetBirthStateAndScan` has no non-test caller inside the repository; it is exercised only by
`TestSetBirthStateAndScan` (`wallet/wallet_test.go:147-231`). `Wallet.Rescan` (start by block hash) has
no caller at all inside the repository, test or otherwise; every in-repository rescan starts from a
height. `(*RescanFilter).RemoveAddress` and `RemoveUnspentOutPoint` likewise have no in-repository
caller (the SPV backend only adds and queries).

### 1.3 Callees

| Dependency | Used for |
| --- | --- |
| `udb.Store` (`w.txStore`) | main chain tip, headers, cfilters, block records, `ExtendMainChain`, `Rollback`, `InsertMemPoolTx`, `InsertMinedTx`, `AddCredit`, `AddTicketCommitment`, `AddMultisigOut`, `SpendMultisigOut`, `RedeemTicketCommitments`, `PruneUnmined`, `UnspentOutput`, `OwnTicket`, `Tx`, processed-txs marker, birthday state |
| `udb.Manager` (`w.manager`) | address lookup, `MarkUsed`, `SyncAccountToAddrIndex`, `MarkUsedChildIndex`, `ImportScript`, `RedeemScript`, account/branch xpubs, `CoinType`, `CoinTypePrivKey`, `NewAccount` |
| `NetworkBackend` | `Blocks`, `LoadTxFilter`, `Rescan`, `PublishTransactions`; optional `UsedAddresses` (discovery) and `deployments.Querier` (voting) |
| `NotificationServer` | attached/detached block, mined/unmined/removed transaction, main tip changed |
| `standalone` | `CalcWork`, `CompactToBig`, `BigToCompact`, `CheckProofOfWork`, `CheckProofOfWorkHash`, `CalcASERTDiff`, `InsideTSpendWindow`, merkle roots, `VerifyInclusionProof` |
| `stake` | `DetermineTxType`, `IsSStx`, `IsSSGen`, `IsTSpend`, `SSGenBlockVotedOn`, `AddrFromSStxPkScrCommitment` |
| `stdscript`, `stdaddr`, `txscript` | address extraction, multisig redeem script extraction, script engine for unmined input verification, treasury vote script building |
| `gcs`/`blockcf2` | `FilterV2.MatchAny`, `FilterV2.N`, `FilterV2.Hash`, `blockcf2.Key`, `blockcf2.Entries` |
| `hdkeychain` | child derivation for discovery and watching |
| `deployments` | `DCP0001.Active`, `DCP0010Active`, `DCP0012Active` |
| VSP clients, mixing client | `VSPClient.RemoveTicket`, `udb.DeleteVSPTicket`, `mixclient.ExpireMessages` |

---

## 2. Public surface inventory

Go receivers are written `(w)` for `*Wallet`, `(f)` for the receiver named in the row. "U" marks unexported
items listed because they carry observable behaviour.

### 2.1 `wallet/chainntfns.go`

| Item | Signature | Contract |
| --- | --- | --- |
| `extendMainChain` (U) | `(w) (ctx, op errors.Op, dbtx ReadWriteTx, n *BlockNode) ([]wire.OutPoint, error)` | Enforce checkpoint, `ExtendMainChain` (tolerating `Exist`), queue attached-block notification, process `n.RelevantTxs`. Returns outpoints to watch. §4.9. |
| `ChainSwitch` | `(w) (ctx, forest *SidechainForest, chain []*BlockNode) ([]*BlockNode, error)` | Make `chain` the tail of the main chain (extend or reorg) in one DB update; returns the detached blocks as nodes (ascending height) or nil. §4.8. |
| `AddTransaction` | `(w) (ctx, tx *wire.MsgTx, blockHash *chainhash.Hash) error` | Record `tx` as mined in `blockHash` (when it is a main-chain block) or as unmined. §4.11. |
| `processTransactionRecord` (U) | `(w) (ctx, dbtx ReadWriteTx, rec *udb.TxRecord, header *wire.BlockHeader, blockMeta *udb.BlockMeta) ([]wire.OutPoint, error)` | Insert the record, credits, ticket commitments, multisig data; mark addresses used; notify. §4.10. |
| `selectOwnedTickets` (U) | `(w *Wallet, dbtx ReadTx, tickets []*chainhash.Hash) []*chainhash.Hash` | Filter to hashes for which `txStore.OwnTicket` is true, preserving order. Returns nil (not empty) when none. |
| `VoteOnOwnedTickets` | `(w) (ctx, winningTicketHashes []*chainhash.Hash, blockHash *chainhash.Hash, blockHeight int32) error` | Create, sign, publish and record votes on `blockHash` for owned winners. §4.12. |
| `RevokeOwnedTickets` | `(w) (ctx, missedTicketHashes []*chainhash.Hash) error` | Deprecated no-op; always returns nil (`wallet/chainntfns.go:931-933`). |

### 2.2 `wallet/rescan.go`

| Item | Signature | Contract |
| --- | --- | --- |
| `maxBlocksPerRescan` (U const) | `2000` | Block hashes handed to `NetworkBackend.Rescan` per batch. |
| `RescanFilter` | struct, unexported fields | Exact (not probabilistic) in-memory set of watched addresses and unspent outpoints. Not concurrency safe. Zero value invalid. §4.18. |
| `NewRescanFilter` | `(addresses []stdaddr.Address, unspentOutPoints []*wire.OutPoint) *RescanFilter` | Build a filter containing every argument. |
| `(*RescanFilter).AddAddress` | `(a stdaddr.Address)` | Insert by type-specific key. |
| `(*RescanFilter).ExistsAddress` | `(a stdaddr.Address) bool` | Membership; a compressed P2PK address also matches its P2PKH form. |
| `(*RescanFilter).RemoveAddress` | `(a stdaddr.Address)` | Delete by type-specific key (no P2PKH fallback). |
| `(*RescanFilter).AddUnspentOutPoint` | `(op *wire.OutPoint)` | Insert `*op` (hash, index and tree all significant). |
| `(*RescanFilter).ExistsUnspentOutPoint` | `(op *wire.OutPoint) bool` | Membership. |
| `(*RescanFilter).RemoveUnspentOutPoint` | `(op *wire.OutPoint)` | Delete. |
| `logRescannedTx` (U) | `(w) (txmgrNs ReadBucket, height int32, tx *wire.MsgTx)` | Info-log newly discovered or newly mined transactions during a rescan. |
| `saveRescanned` (U) | `(w) (ctx, dbtx ReadWriteTx, hash *chainhash.Hash, txs []*wire.MsgTx, logTxs bool) error` | Process each tx as mined in block `hash`; errors wrapped with op `wallet.saveRescanned`. |
| `rescan` (U) | `(w) (ctx, n NetworkBackend, startHash *chainhash.Hash, height int32, p chan<- RescanProgress) error` | Core loop. §4.13. |
| `Rescan` | `(w) (ctx, n NetworkBackend, startHash *chainhash.Hash) error` | Rescan from a block hash (inclusive) through the tip; op `wallet.Rescan`. |
| `RescanFromHeight` | `(w) (ctx, n NetworkBackend, startHeight int32) error` | Same from a main-chain height; op `wallet.RescanFromHeight`. |
| `RescanProgress` | `struct { Err error; ScannedThrough int32 }` | One progress or terminal error message. |
| `RescanProgressFromHeight` | `(w) (ctx, n NetworkBackend, startHeight int32, p chan<- RescanProgress)` | As `RescanFromHeight` with progress on `p`; closes `p` before returning; op `wallet.RescanProgressFromHeight`. |
| `mainChainAncestor` (U) | `(w) (dbtx ReadTx, hash *chainhash.Hash) (*chainhash.Hash, error)` | Walk `PrevBlock` links from `hash` until a main-chain block is found. |
| `RescanPoint` | `(w) (ctx) (*chainhash.Hash, error)` | Hash of the first main-chain block whose transactions are not yet processed, or nil when none; op `wallet.RescanPoint`. |
| `rescanPoint` (U) | `(w) (dbtx ReadTx) (*chainhash.Hash, error)` | Transactional form. §4.14. |
| `SetBirthState` | `(w) (ctx, bs *udb.BirthdayState) error` | Persist the birthday state verbatim; op `wallet.SetBirthState`. |
| `BirthState` | `(w) (ctx) (*udb.BirthdayState, error)` | Read the birthday state; nil when unset; op `wallet.BirthState`. |
| `SetBirthStateAndScan` | `(w) (ctx, bs *udb.BirthdayState) error` | Resolve a height- or time-based birthday against the current main chain and persist it. Does not scan. §4.15. |

### 2.3 `wallet/discovery.go`

| Item | Signature | Contract |
| --- | --- | --- |
| `usedAddressesQuerier` (U interface) | `UsedAddresses(ctx, addrs []stdaddr.Address) (bitset.Bytes, error)` | Optional capability of a network backend; when implemented (dcrd RPC syncer) discovery uses it instead of cfilters. |
| `blockCommitmentCache` (U) | `map[chainhash.Hash]map[string]struct{}` | Block hash → set of exact output scripts (stake opcode stripped for stake outputs). |
| `blockCommitments` (U) | `(block *wire.MsgBlock) map[string]struct{}` | Reduce a block to the scripts discovery compares against. §4.16.3. |
| `cacheMissingCommitments` (U) | `(ctx, n NetworkBackend, cache blockCommitmentCache, include []*chainhash.Hash) error` | Fetch uncached blocks in chunks of 500 and cache their commitments. |
| `accountUsage`, `scriptPath`, `addrFinder` (U) | structs | Bisect state per account; script → path; finder. |
| `newAddrFinder` (U) | `(ctx, w *Wallet, gapLimit uint32) (*addrFinder, error)` | Load every BIP0044 and imported-xpub account's branch keys and last-used indexes. |
| `(*addrFinder).find` (U) | `(ctx, start *chainhash.Hash, n NetworkBackend) error` | Cfilter-based bisect of last used child per branch. §4.16.5. |
| `(*addrFinder).filter` (U) | `(ctx, fs []*udb.BlockCFilter, data blockcf2.Entries, n NetworkBackend) error` | Match `data` against cfilters, fetch matching blocks, cache commitments. |
| `(w) filterBlocks` (U) | `(ctx, startBlock *chainhash.Hash, data blockcf2.Entries) ([]*chainhash.Hash, error)` | Hashes of main-chain blocks from `startBlock` whose cfilter matches any entry; result order is not deterministic. |
| `(w) findLastUsedAccount` (U) | `(ctx, n NetworkBackend, blockCache blockCommitmentCache, coinTypeXpriv *hd.ExtendedKey, gapLimit uint32, startBlock *chainhash.Hash) (uint32, error)` | Cfilter-based account discovery. §4.16.2. |
| `existsAddrIndexFinder` (U) | `struct { wallet *Wallet; rpc usedAddressesQuerier; gapLimit uint32 }` | Discovery through a trusted dcrd. |
| `(*existsAddrIndexFinder).findLastUsedAccount` (U) | `(ctx, coinTypeXpriv *hd.ExtendedKey) (uint32, error)` | §4.16.4. |
| `(*existsAddrIndexFinder).accountUsed` / `branchUsed` (U) | `(ctx, xpub) (bool, error)` | Any of the first `wallet.gapLimit` children of either branch used. |
| `(*existsAddrIndexFinder).findLastUsedAddress` (U) | `(ctx, xpub *hd.ExtendedKey) (uint32, error)` | Last used child of a branch, `0xFFFFFFFF` when none. §4.16.6. |
| `(*existsAddrIndexFinder).find` (U) | `(ctx, finder *addrFinder) error` | Run `findLastUsedAddress` for both branches of every account concurrently. |
| `DiscoverActiveAddresses` | `(w) (ctx, n NetworkBackend, startBlock *chainhash.Hash, discoverAccts bool, gapLimit uint32) error` | Full discovery pipeline. §4.16.1. Op `wallet.DiscoverActiveAddresses`. |

### 2.4 `wallet/sidechains.go`

| Item | Signature | Contract |
| --- | --- | --- |
| `SidechainForest` | `struct { trees []*sidechainRootedTree }` | Disjoint rooted trees of non-main-chain blocks. Zero value is ready to use. Not concurrency safe. |
| `BlockNode` | `struct { Header *wire.BlockHeader; Hash *chainhash.Hash; FilterV2 *gcs.FilterV2; RelevantTxs []*wire.MsgTx; parent *BlockNode; workSum *big.Int; invalid bool }` | One block in the forest. `RelevantTxs == nil` and `len(RelevantTxs) == 0` are distinguished (§4.8 step 9). |
| `sidechainRootedTree` (U) | `struct { root *BlockNode; children, tips map[chainhash.Hash]*BlockNode; bestChain []*BlockNode }` | `children` excludes the root. `tips` are childless non-root nodes. |
| `newSideChainRootedTree` (U) | `(root *BlockNode) *sidechainRootedTree` | Sets `root.workSum = CalcWork(root.Header.Bits)`. |
| `NewBlockNode` | `(header, hash, filter, relevantTxs) *BlockNode` | Plain constructor; no validation. |
| `(*BlockNode).BadCheckpoint` | `()` | Mark invalid. Must be called before the node is added to a forest. |
| `duplicateNode` (U) | `(t) (n *BlockNode) bool` | True if a node with `n.Hash` is the root or a child; copies `n.FilterV2` into the existing node when the existing one has none. |
| `maybeAttachNode` (U) | `(t) (n *BlockNode) bool` | Attach `n` under the root or a child when `PrevBlock` and `Height == parent.Height+1` match. |
| `pruneChain` (U) | `(t) (chain []*BlockNode) []*sidechainRootedTree` | Remove `chain` and re-root what remains. §4.6.5. |
| `best` (U) | `(t) () ([]*BlockNode, *big.Int)` | Highest-work valid root-to-tip path and its work; panics if the root is invalid. |
| `(*SidechainForest).AddBlockNode` | `(n *BlockNode) bool` | Insert; false iff a duplicate. §4.6.2. |
| `(*SidechainForest).PruneAll` | `()` | Drop every tree. |
| `(*SidechainForest).Prune` | `(mainChainHeight int32, params *chaincfg.Params)` | Drop trees whose root is more than `CoinbaseMaturity` below `mainChainHeight`. |
| `(*SidechainForest).PruneChain` | `(chain []*BlockNode)` | Remove `chain` from the tree rooted at `chain[0]`. |
| `(*SidechainForest).HasSideChainBlock` | `(blockHash *chainhash.Hash) bool` | Root or child of any tree. |
| `(*SidechainForest).FullSideChain` | `(newBlocks []*BlockNode) ([]*BlockNode, error)` | Prefix `newBlocks` with the forest path from a root down to `newBlocks[0]`'s parent. §4.6.7. |
| `EvaluateBestChain` | `(w) (ctx, f *SidechainForest) ([]*BlockNode, error)` | Best sidechain with strictly more work than the main chain portion it replaces, or nil. Op `wallet.EvaluateBestChain`. §4.7. |

### 2.5 `wallet/difficulty.go`

| Item | Signature | Contract |
| --- | --- | --- |
| `isTestNet3` (U) | `(w) () bool` | `chainParams.Net == wire.TestNet3`. |
| `testNet3MaxDiffActivationHeight` (U const) | `962928` | Height from which testnet3 drops the min-difficulty reduction rule and enforces the max-difficulty rule on the EMA path. |
| `bigZero` (U var) | `big.NewInt(0)` | |
| `findPrevTestNetDifficulty` (U) | `(w) (dbtx, h *wire.BlockHeader, chain []*BlockNode) (uint32, error)` | Bits of the most recent ancestor (starting with `h`) not mined under the testnet min-difficulty rule. |
| `calcNextBlake256Diff` (U) | `(w) (dbtx, header *wire.BlockHeader, chain []*BlockNode, newBlockTime time.Time) (uint32, error)` | Required bits for the child of `header` under the launch EMA algorithm. §4.3. |
| `loadCachedBlake3WorkDiffCandidateAnchor` / `storeCachedBlake3WorkDiffCandidateAnchor` (U) | `() *wire.BlockHeader` / `(candidate *wire.BlockHeader)` | Mutex-protected single-slot cache. |
| `isBlake3PowAgendaForcedActive` (U) | `(w) () bool` | Deployment `blake3pow` exists in `deploymentsByID` with `ForcedChoiceID == "yes"`. |
| `calcNextBlake3DiffFromAnchor` (U) | `(w) (prevNode, blake3Anchor *wire.BlockHeader) uint32` | ASERT bits for the child of `prevNode` relative to `blake3Anchor`. §4.4. |
| `calcWantHeight` (U) | `(stakeValidationHeight, interval, height int64) int64` | Height of the final block of the rule-change interval before `height`. |
| `checkDifficultyPositional` (U) | `(w) (dbtx, header, prevNode *wire.BlockHeader, chain []*BlockNode) error` | Accept `header.Bits` if it matches either algorithm that could apply. §4.2. |
| `estimateSupply` (U) | `(params, height int64) int64` | Supply estimate used as the stake difficulty upper bound. |
| `sumPurchasedTickets` (U) | `(w) (dbtx, startHeader *wire.BlockHeader, chain []*BlockNode, numToSum int64) (int64, error)` | Sum of `FreshStake` over `startHeader` and up to `numToSum-1` ancestors. |
| `calcNextStakeDiffV2` (U) | `(params, nextHeight, curDiff, prevPoolSizeAll, curPoolSizeAll int64) int64` | DCP0001 core formula. |
| `ancestorHeaderAtHeight` (U) | `(w) (dbtx, h *wire.BlockHeader, chain []*BlockNode, height int32) (*wire.BlockHeader, error)` | Ancestor of `h` at `height`, from `chain` when in range, else the main chain by height. Returns `(nil, nil)` when `height > h.Height` or `height < 0`. |
| `isAncestorOf` (U) | `(w) (dbtx, node, target *wire.BlockHeader, chain []*BlockNode) (bool, error)` | Hash comparison of `target`'s ancestor at `node.Height` with `node`. See quirk Q-7. |
| `relativeAncestor` (U) | `(w) (dbtx, node *wire.BlockHeader, distance int64, chain []*BlockNode) (*wire.BlockHeader, error)` | `ancestorHeaderAtHeight(node, chain, int32(node.Height)-int32(distance))`. |
| `nextRequiredDCP0001PoSDifficulty` (U) | `(w) (dbtx, curHeader *wire.BlockHeader, chain []*BlockNode) (dcrutil.Amount, error)` | Stake difficulty for the child of `curHeader`. §4.5. |
| `NextStakeDifficulty` | `(w) (ctx) (dcrutil.Amount, error)` | Ticket price for the block after the main-chain tip. Kind `Deployment` when DCP0001 not known active at the tip height. Op `wallet.NextStakeDifficulty`. |
| `NextStakeDifficultyAfterHeader` | `(w) (ctx, h *wire.BlockHeader) (dcrutil.Amount, error)` | Ticket price for the child of `h`. Op `wallet.NextStakeDifficultyAfterHeader`. |
| `ValidateHeaderChainDifficulties` | `(w) (ctx, chain []*BlockNode, idx int) ([]*BlockNode, error)` | Validate `chain[idx:]`; on a consensus violation return the subslice starting at the offending node together with the error. §4.1. |
| `validateHeaderChainDifficulties` (U) | `(w) (dbtx, chain, idx)` | Transactional form; op `wallet.validateHeaderChainDifficulties`. |

### 2.6 `wallet/checkpoints.go`

| Item | Signature | Contract |
| --- | --- | --- |
| `mustParseHash` (U) | `(s string) chainhash.Hash` | Parse a display-order hex hash; panic on error. |
| `testnet3block962928Hash` (U var) | `0000004fd1b267fd39111d456ff557137824538e6f6776168600e56002e23b93` | The only checkpoint. |
| `CheckpointHash` | `(network wire.CurrencyNet, height int32) *chainhash.Hash` | Non-nil only for `(TestNet3, 962928)`. |
| `BadCheckpoint` (func) | `(network, hash *chainhash.Hash, height int32) bool` | True iff a checkpoint exists at `height` and `*hash` differs. |
| `rollbackInvalidCheckpoints` (U) | `(w) (dbtx ReadWriteTx) error` | At open, roll the main chain back to below any checkpoint height whose stored block differs. §4.19. |

### 2.7 `validate/validate.go` (package `validate`)

| Item | Signature | Contract |
| --- | --- | --- |
| `DCP0005ActiveHeightTestNet3` | `int32 = 323328` | |
| `DCP0005ActiveHeightMainNet` | `int32 = 431488` | |
| `MerkleRoots` | `(block *wire.MsgBlock) error` | Regular tree root must equal `Header.MerkleRoot` and stake tree root must equal `Header.StakeRoot`. |
| `DCP0005MerkleRoot` | `(block *wire.MsgBlock) error` | Combined root must equal `Header.MerkleRoot`. |
| `CFilterV2HeaderCommitment` | `(net wire.CurrencyNet, header *wire.BlockHeader, filter *gcs.FilterV2, leafIndex uint32, proof []chainhash.Hash) error` | Inclusion proof of `filter.Hash()` in `header.StakeRoot`; unconditionally nil below the DCP0005 height on mainnet/testnet3. |
| `PreDCP0005CFilterHash` | `(net wire.CurrencyNet, cfsethash *chainhash.Hash) error` | Compare against a per-network constant. |

### 2.8 Adjacent items specified here (defined in other files)

| Item | Source | Why here |
| --- | --- | --- |
| `(w) watchHDAddrs(ctx, firstWatch bool, n NetworkBackend) (count uint64, err error)` | `wallet/wallet.go:891-1065` | Watching of future addresses; called from `ChainSwitch`, `AddTransaction`, `VoteOnOwnedTickets`. §4.17. |
| `(w) LoadActiveDataFilters(ctx, n, reload bool) error` | `wallet/wallet.go:1101-1201` | Initial filter load around discovery/rescan. §4.17.3. |
| `(w) markUsedAddress(op, dbtx, addr udb.ManagedAddress) error` | `wallet/addresses.go:718-746` | Gap-limit extension on use. §4.10 step 6. |
| `deriveChildAddresses`, `deriveChildAddress`, `deriveBranches` | `wallet/addresses.go:1004-1038` | Derivation used by discovery and watching. |
| `NetworkBackend` interface | `wallet/network.go:35-57` | The contract `rescan`, discovery and voting depend on. §3.6. |
| `deployments.HardcodedDeployment.Active`, `DCP0001`, `DCP0010Active`, `DCP0012Active`, `Querier` | `deployments/deployments.go` | Gate stake difficulty validation and vote subsidy selection. |
| `isVote(tx)`, `isRevocation(tx)`, `isTreasurySpend(tx)` (U) | `wallet/wallet.go:3973-3983` | One-line aliases of `stake.IsSSGen`, `stake.IsSSRtx`, `stake.IsTSpend`. `AddTransaction` calls `isTreasurySpend` and `isVote` (§4.11 names the `stake` functions they forward to). |
| `(w) appendRelevantOutpoints(relevant []wire.OutPoint, dbtx ReadTx, tx *wire.MsgTx) []wire.OutPoint` (U) | `wallet/wallet.go:5004-5060` | Outpoints of `tx` to watch, decided by hash160 lookup only; used for votes before they are recorded. §4.12.1. |
| `(w) hasVotingAuthority(addrmgrNs ReadBucket, ticketPurchase *wire.MsgTx) (mine, havePrivKey bool, err error)` (U) | `wallet/wallet.go:3985-4005` | Vote eligibility of a ticket. §4.12.1. |
| `validateTxInput(op, tx, i, prevScript) error` (U), `sanityVerifyFlags` | `wallet/createtx.go:45-49,728-744` | Script-engine check of one unmined input. §4.11 step 3.4. |
| `(w) expireMixMessages(height uint32)` (U) | `wallet/mixing.go:583-589` | Loads the atomic mixing-client pointer; nil → no-op; otherwise `ExpireMessages(height)`. Called at the end of `ChainSwitch`. |
| `voteVersion(params) uint32` (U) | `wallet/wallet.go:291-304` | Stake version this wallet votes with; compared with each attached header's `StakeVersion`. |
| `outpoint` (U type) | `wallet/wallet.go:94-97` | `struct { hash chainhash.Hash; index uint32 }` — key of `w.lockedOutpoints`. The tree is **not** part of the key. |
| `udb.BlockCFilter` | `wallet/udb/txmined.go:492-497` | `{ BlockHash chainhash.Hash; FilterV2 *gcs.FilterV2; Key [16]byte }` — one stored main-chain cfilter with its key, as returned by `GetMainChainCFilters`. |
| `udb.NewTxRecordFromMsgTx(msgTx, received) (*TxRecord, error)` | `wallet/udb/txmined.go:125-141` | Serializes the transaction (its only error source), copies the `MsgTx` by value, sets `Received`, `TxType = stake.DetermineTxType`, `Hash = TxHash()`. Every record built in scope goes through it. |

### 2.9 `Wallet` fields read or written by code in scope

`wallet/wallet.go:102-178`; initialised in `Open` (`wallet/wallet.go:5361-5425`). All are in-memory.

| Field | Type | Initial value | Use in scope |
| --- | --- | --- | --- |
| `votingEnabled` | `bool` | `cfg.VotingEnabled` | gate of `VoteOnOwnedTickets` |
| `manualTickets` | `bool` | `cfg.ManualTickets` | ticket skip in `extendMainChain` |
| `subsidyCache` | `*standalone.SubsidyCache` | `NewSubsidyCache(params)` | passed to `createUnsignedVote` |
| `gapLimit` | `uint32` | `cfg.GapLimit` | `markUsedAddress`, `watchHDAddrs`, `branchUsed` (Q-15) |
| `watchLast` | `uint32` | `cfg.WatchLast` | first-watch range limit |
| `accountGapLimit` | `int` | `cfg.AccountGapLimit` | account discovery segment size (converted with `uint32(...)`) |
| `disableCoinTypeUpgrades` | `bool` | `cfg.DisableCoinTypeUpgrades` | coin type upgrade gate |
| `networkBackend` / `networkBackendMu` | `NetworkBackend` | nil | `NetworkBackend()` returns kind `NoPeers` while nil |
| `lockedOutpoints` / `lockedOutpointMu` | `map[outpoint]struct{}` | empty | entries deleted for every input of a non-vote, non-revocation transaction |
| `recentlyPublished` / `recentlyPublishedMu` | `map[chainhash.Hash]struct{}` | empty | §4.8 step 14, §4.11 step 1, §4.12 step 8 |
| `logRescannedTransactions` / `...Mu` | `bool` | false | §4.13 step 1 |
| `addressBuffers` / `addressBuffersMu` | `map[uint32]*bip0044AccountData` | one entry per account at open | discovery and watching; `bip0044AccountData{ xpub; albExternal, albInternal addressBuffer }`, `addressBuffer{ branchXpub; lastUsed, cursor, lastWatched uint32 }` (`wallet/addresses.go:446-460`) |
| `mixClient` | `atomic.Pointer[mixclient.Client]` | nil | `expireMixMessages` |
| `cachedBlake3WorkDiffCandidateAnchor` / `...Mu` | `*wire.BlockHeader` | nil | §4.2 steps 5-6 |
| `chainParams` | `*chaincfg.Params` | `cfg.Params` | everywhere |
| `deploymentsByID` | `map[string]*chaincfg.ConsensusDeployment` | every deployment of every stake version in `params.Deployments`, keyed by `Vote.Id`; a repeated id panics with `reused deployment ID "<id>"` (`wallet/wallet.go:5373-5384`) | `isBlake3PowAgendaForcedActive` (key `chaincfg.VoteIDBlake3Pow` = `blake3pow`) |
| `minTestNetTarget`, `minTestNetDiffBits` | `*big.Int`, `uint32` | nil, 0 except on testnet3 (§4.4) | EMA and ASERT clamps |
| `vspClients` (`vspClientsMu` not taken) | `map[string]*VSPClient` | empty | §4.8 step 12 |
| `NtfnServer` | `*NotificationServer` | — | notifications |

---

## 3. Data formats

This subsystem owns no bucket of its own. It reads and writes the `wtxmgr` and `waddrmgr` namespaces
through `udb` (specified in the database specs). The items below are the ones whose *content* is decided by
code in scope, plus the wire/script structures it constructs or matches.

### 3.1 Database keys written or read directly through `udb` helpers

All keys are in the top-level bucket `wtxmgr` (`wallet/wallet.go:76-79`, `wallet/udb/txdb.go:156-164`).
Multi-byte integers in `udb` values are **big endian** (`wallet/udb/txdb.go:123`).

| Key (ASCII) | Value | Written by code in scope | Read by code in scope |
| --- | --- | --- | --- |
| `tip` | 32-byte main-chain tip block hash (internal byte order) | indirectly: `ExtendMainChain`, `Rollback` | `MainChainTip` everywhere |
| `lasttxsblock` | 32-byte hash of the last block whose transactions are processed ("marker") | `UpdateProcessedTxsBlockMarker` from `ChainSwitch` (two places) and `rescan` | `rescanPoint` |
| `birthstate` | 45-byte birthday record, §3.2 | `SetBirthState`, `SetBirthStateAndScan`, `ChainSwitch` | `BirthState`, `ChainSwitch` |
| `havecfilters` | 1 byte, `0x01` when every main-chain block has a cfilter | not written here | `ExtendMainChain` refuses to extend unless it is exactly `{0x01}` (`wallet/udb/txmined.go:208-211`) |

A database created by the pinned version obtains `lasttxsblock` from the version 10→11 upgrade, which
copies `tip` (`wallet/udb/upgrades.go:907-912`); creation writes only `tip` (`wallet/udb/txdb.go:2287`). When
`lasttxsblock` is absent the marker reads as the all-zero hash (`wallet/udb/txmined.go:277-282`) and
`rescanPoint` fails with `NotExist` because no header has that hash.

`UpdateProcessedTxsBlockMarker(dbtx, hash)` semantics that callers in scope rely on
(`wallet/udb/txmined.go:289-321`):

1. Walk the current marker back along `PrevBlock` until a main-chain block `prev` is found (header lookup
   failure is returned).
2. Fail with kind `Invalid`, message `<hash> is not a main chain block`, when `hash` is not in the main chain.
3. Write `lasttxsblock = hash` only when `header(hash).Height > header(prev).Height`. Otherwise succeed
   without writing. The marker therefore never moves backwards relative to its main-chain ancestor.

### 3.2 Birthday state value (`birthstate`)

`wallet/udb/txmined.go:327-394`. Fixed length 45 bytes. A stored value of any other length reads as "unset"
(nil).

| Offset | Size | Field | Encoding |
| --- | --- | --- | --- |
| 0 | 1 | options | bit 0 (`0x01`) = `SetFromHeight`; bit 1 (`0x02`) = `SetFromTime`; computed by addition, other bits always 0 on write and ignored on read |
| 1 | 32 | `Hash` | block hash, internal byte order |
| 33 | 4 | `Height` | uint32 big endian |
| 37 | 8 | `Time` | uint64 big endian of `Time.Unix()` (seconds; sub-second precision is discarded; negative values are stored as two's complement and read back through `int64`) |

In-memory form: `udb.BirthdayState{ Hash chainhash.Hash; Height uint32; Time time.Time; SetFromHeight, SetFromTime bool }`.

Where stored values come from (all through `udb.SetBirthState`, which writes all 45 bytes every time):

| Writer | Stored state | Source |
| --- | --- | --- |
| Database upgrade 25→26 (`birthBlockUpgrade`), which every database runs — creation writes the initial version and then runs all upgrades (`wallet/udb/initialize.go:45-50,83-88`) | options `0x00`, `Hash` = `params.GenesisHash`, `Height` = 0, `Time` = Go zero time, whose `Unix()` is −62135596800, stored as `ff ff ff f1 88 6e 09 00`. The value is therefore never absent in a database at the pinned version, and reads back as "resolved at genesis" | `wallet/udb/upgrades.go:1665-1689` |
| CLI `--create`, new seed | `SetFromTime`, `Time` = now − 24 h; hash zero, height 0 | `walletsetup.go:192-196,255` |
| CLI `--create`, restored seed, birthday date entered | `SetFromTime`, `Time` = the parsed date | `walletsetup.go:180-185` |
| CLI `--create`, restored seed, block number entered | `SetFromHeight`, `Height` = the number | `walletsetup.go:186-191` |
| CLI `--create`, restored seed, no birthday (or EOF on the prompt) | `SetFromHeight`, `Height` = 0 | `walletsetup.go:198-204` |
| gRPC `WalletLoaderService.CreateWallet` | see Q-50: `SetFromHeight` with `Height = req.BirthHeight` when `set_birth_time` is true, else `SetFromHeight` with `Height` = 0 | `internal/rpc/rpcserver/server.go:2713-2752` |
| `ChainSwitch` resolution (§4.8 step 8.3), `SetBirthStateAndScan` (§4.15) | both flags clear, `Hash`/`Height` of the birthday block; `Time` unchanged from the pending state | `wallet/chainntfns.go:186-207`, `wallet/rescan.go:532-581` |

Creation paths that never call `SetBirthState` (`createSimulationWallet` and
`createWatchingOnlyWallet`, `walletsetup.go:270-369`; gRPC `CreateWatchingOnlyWallet`) keep the
upgrade's genesis value. `SetFromHeight`
with `Height` = 0 resolves when block 1 is attached (`Height + 1`), giving the genesis block as birthday.

State machine of the two flags:

| Flags | Meaning | Resolved by |
| --- | --- | --- |
| `SetFromHeight` | `Height` is a requested birthday height; `Hash` not yet known | `ChainSwitch` when a block at `Height+1` is attached (§4.8 step 8), or `SetBirthStateAndScan` |
| `SetFromTime` | `Time` is a requested birthday time; `Hash`/`Height` not yet known | `ChainSwitch` when the first block with `Timestamp > Time` is attached, or `SetBirthStateAndScan` |
| neither | `Hash`/`Height` name the birthday block; rescans start at its child | — |

### 3.3 Header, block record and cfilter rows written on main-chain extension

Written by `udb.Store.ExtendMainChain(ns, header, blockHash, filter)` (`wallet/udb/txmined.go:203-272`),
called only from `extendMainChain`. Listed for the dependency on values this subsystem supplies:

| Row | Key | Value |
| --- | --- | --- |
| header | block hash (32 bytes), headers bucket | 180-byte serialized `wire.BlockHeader` |
| block record | height (uint32 BE), blocks bucket | empty block record built from hash and header |
| cfilter | block hash, cfilters bucket | `[0:16]` filter key = first 16 bytes of `header.MerkleRoot` (`blockcf2.Key`, `gcs/v4@v4.1.1/blockcf2/blockcf.go:96-100`); `[16:]` = `filter.Bytes()` (`wallet/udb/txdb.go:1973-1986`) |
| `tip` | — | block hash |

`ExtendMainChain` also stake-validates or stake-invalidates the previous tip according to bit 0
(`dcrutil.BlockValid`) of `header.VoteBits`. A nil `FilterV2` on the node panics at `f.Bytes()`; every caller
must have populated the filter before `ChainSwitch`.

### 3.4 `BlockNode` (in memory only)

| Field | Type | Set by | Notes |
| --- | --- | --- | --- |
| `Header` | `*wire.BlockHeader` | constructor | never nil |
| `Hash` | `*chainhash.Hash` | constructor | must equal `Header.BlockHash()`; **not verified** by the forest or by `validateHeaderChainDifficulties`, which runs the blake256 PoW check against this field (§4.1 step 2.4) |
| `FilterV2` | `*gcs.FilterV2` | constructor, later assignment by syncers, or `duplicateNode` copy | may be nil while in the forest |
| `RelevantTxs` | `[]*wire.MsgTx` | syncers; `ChainSwitch` for detached nodes | nil = "transactions not scanned"; non-nil (possibly empty) = "scanned" |
| `parent` | `*BlockNode` | `maybeAttachNode` | not cleared when a node becomes a root (§4.6.5) |
| `workSum` | `*big.Int` | `newSideChainRootedTree`, `maybeAttachNode` | cumulative `CalcWork` from the tree root inclusive |
| `invalid` | `bool` | `BadCheckpoint()`, inherited in `maybeAttachNode` | sticky |

### 3.5 Cfilter match entries and block commitment scripts

**Match entries (what is hashed into `FilterV2.MatchAny`).** Every entry is the version-0 payment script
returned by `stdaddr.Address.PaymentScript()` for an address:

| Address type | Entry bytes |
| --- | --- |
| P2PKH (ECDSA secp256k1) | `76 a9 14 <20-byte hash160> 88 ac` (25 bytes) |
| P2SH | `a9 14 <20-byte hash160> 87` (23 bytes) |
| other types | whatever `PaymentScript()` returns |

Sources of entries:

| Consumer | Entries | Source |
| --- | --- | --- |
| SPV syncer `filterData` (used for new blocks and rescans) | one entry per address passed to `LoadTxFilter`, appended with `Entries.AddRegularPkScript` (empty scripts ignored); never deduplicated; reset only on `reload` | `spv/backend.go:417-432` |
| Address discovery (SPV) | payment scripts of the `gapLimit` children of the current bisect segment of every unfinished branch | `wallet/discovery.go:209-243` |
| Account discovery (SPV) | payment scripts of children `0..gapLimit-1` of both branches of every account in the probed account range | `wallet/discovery.go:425-461` |

Outpoints are **never** cfilter match entries: version 2 filters do not commit to spent outpoints, so a
spend is found only because the spent output's script (an address the wallet watches) is committed as a
previous-output script (`spv/backend.go:412-416`, `gcs/v4@v4.1.1/blockcf2/blockcf.go:214-246`).

Filter parameters are `B = 19`, `M = 784931`; key = first 16 bytes of the header merkle root, interpreted
by the filter as two little-endian uint64 SipHash keys (`gcs/v4@v4.1.1/blockcf2/blockcf.go:29-39,96-100`,
`gcs/v4@v4.1.1/gcs.go:333-360`). `MatchAny` returns false for an empty filter or an empty entry list.
Callers in scope additionally skip filters with `N() == 0`.

**Block commitment scripts (false-positive elimination in discovery).** `blockCommitments` reduces a block
to a set of byte strings (§4.16.3). A discovery entry "is used in a block" iff the entry's exact bytes are a
member of that set.

### 3.6 `NetworkBackend` contract as consumed here

`wallet/network.go:35-57`. Only the behaviours this subsystem depends on:

| Method | Dependence |
| --- | --- |
| `Blocks(ctx, hashes) ([]*wire.MsgBlock, error)` | Result index `i` corresponds to `hashes[i]`. Discovery calls it with at most 500 hashes (`wire.MaxBlocksPerMsg`). The result length is not checked: discovery ranges over the **returned** slice and indexes the request by the same position (`wallet/discovery.go:89-91,346-354`), so a shorter result silently leaves the tail uncached (in `addrFinder.filter` those blocks are matched and requested again on the next bisect iteration) and a longer result panics with an index out of range. |
| `LoadTxFilter(ctx, reload, addrs, outpoints) error` | Additive unless `reload`, in which case the filter is emptied first. Called with `(false, addrs, nil)`, `(false, nil, outpoints)` and `(true, nil, nil)`. |
| `Rescan(ctx, blocks []chainhash.Hash, save func(*chainhash.Hash, []*wire.MsgTx) error) error` | Calls `save` sequentially, in block order, at most once per block, for blocks with relevant transactions (the SPV backend never calls it with an empty list, `spv/backend.go:610-618`; the RPC backend calls it for every `discovereddata` element dcrd returns, `rpc/client/dcrd/calls.go:451-469`). Must stop and return `save`'s error. |
| `PublishTransactions(ctx, txs...) error` | Called by voting, possibly with zero transactions. |
| optional `UsedAddresses(ctx, addrs) (bitset.Bytes, error)` | Bit `i` (LSB-first within each byte, `jrick/bitset`) set iff `addrs[i]` has been used on chain. Implemented by the RPC syncer through dcrd `existsaddresses` in batches of 400000 (`rpc/client/dcrd/calls.go:98-127`). Not implemented by the SPV syncer. |
| optional `Deployments(ctx) (map[string]dcrdtypes.AgendaInfo, error)` | Required by `VoteOnOwnedTickets`. Implemented only by the RPC syncer (`chain/backend.go:63-69`). |

### 3.7 Treasury vote output appended to votes

Built in `VoteOnOwnedTickets` (`wallet/chainntfns.go:802-842`) when at least one tspend is in its voting
window and has a non-abstain policy:

```
data   = 'T' 'V' || ( tspend_tx_hash[32] || vote[1] )*      vote: 0x01 = yes, 0x02 = no
script = OP_RETURN (0x6a) || canonical data push of data
output = TxOut{ Value: 0, Version: 0, PkScript: script }      appended as the last vote output
tx.Version = 3                                                (wire.TxVersionTreasury)
```

`tspend_tx_hash` is in internal byte order. The push opcode follows `txscript.ScriptBuilder.AddData`
canonical rules (1-75 bytes: length opcode; 76-255: `OP_PUSHDATA1`; larger: `OP_PUSHDATA2`). The order of
`(hash, vote)` pairs follows Go map iteration over the tspend cache and is not deterministic (quirk Q-14).

### 3.8 Hard-coded hashes

| Purpose | Value (display order, i.e. byte-reversed relative to storage) | Source |
| --- | --- | --- |
| testnet3 checkpoint at height 962928 | `0000004fd1b267fd39111d456ff557137824538e6f6776168600e56002e23b93` | `wallet/checkpoints.go:22` |
| pre-DCP0005 cfilter set hash, testnet3 | `619c08f5adda6f834212bbdaee3002fdc4efed731477af6c0fed490bbe2488d0` | `validate/validate.go:112` |
| pre-DCP0005 cfilter set hash, mainnet | `f95e09f9ded38f8d6c32e5158a1f286633881393659218c63f5ab0fc86b36c83` | `validate/validate.go:114` |

The cfilter set hash is BLAKE-256 over the concatenation of `filter.Bytes()` (no key, no length prefix) of
every main-chain block from height 0 through `DCP0005ActiveHeight - 1` inclusive, in height order
(`wallet/wallet.go:5230-5268`, range walk in `wallet/wallet.go:855-881`). The 32 digest bytes are used as the hash's internal bytes; the constants
above are decoded with `chainhash.Decode`, which reverses byte order, so each constant is the byte-reversed
hex of the digest.

### 3.9 RPC-visible encodings fed from this subsystem

| Datum | Encoding | Source |
| --- | --- | --- |
| Rescan progress | gRPC `RescanResponse.rescanned_through` = `RescanProgress.ScannedThrough` (int32), one message per batch | `internal/rpc/rpcserver/server.go:470-481` |
| Rescan point | gRPC `RescanPointResponse.rescan_point_hash` = 32 raw hash bytes (internal order), or absent/empty when nil | `internal/rpc/rpcserver/server.go:3160-3175` |
| Next ticket price | `dcrutil.Amount` (int64 atoms) | `NextStakeDifficulty` consumers |
| Wallet birthday, JSON-RPC `walletinfo` | `"birthhash"` (string, display-order hex) and `"birthheight"` (uint32) are filled only when the state is non-nil and **neither** flag is set; while a birthday is pending they stay `""` and `0` (the fields have no `omitempty`). A `BirthState` error is logged at error `Failed to get birth state: <err>` and the call still succeeds | `internal/rpc/jsonrpc/methods.go:5531-5538`, `rpc/jsonrpc/types/results.go:407-408` |
| Wallet birthday, gRPC `WalletService.BirthBlock` | `BirthBlockResponse{ hash = 1: 32 raw bytes (internal order); height = 2: uint32 }`. Unset → `NotFound` `birth block not set`; pending (either flag set) → `NotFound` `birth block is pending...`; read error → `Internal` `failed to get birth state: <err>` | `internal/rpc/rpcserver/server.go:628-648`, `rpc/api.proto:308-313` |

---

## 4. Algorithms and state machines

Notation: `chain` is always a slice of `*BlockNode` ordered by strictly increasing height with each node
the child of the previous one. "DB view" / "DB update" mean one `walletdb.View` / `walletdb.Update`
transaction on the wallet database. `E(op, kind, msg)` is `errors.E` from the wallet `errors` package.
Unsigned 32-bit arithmetic wraps modulo 2^32 exactly as in Go wherever `uint32` is named.

### 4.1 Header chain validation — `ValidateHeaderChainDifficulties`

`wallet/difficulty.go:837-912`. One DB view for the whole call.

Preconditions: `len(chain) > 0` (index 0 is dereferenced unconditionally; an empty chain panics);
`0 <= idx <= len(chain)`.

1. If `chain[0].Header.PrevBlock` is not a main-chain block (`txStore.BlockInMainChain`), return
   `(nil, E(op, Bug, "parent of chain[0] is not in main chain"))`.
2. `parent = nil`. For `i = idx; i < len(chain); i++` with `n = chain[i]`, `h = n.Header`, `hash = n.Hash`:
   1. If `parent == nil`: when `i == 0` load `parent` from the database by `h.PrevBlock`
      (`GetBlockHeader`; failure returns `(nil, err)` unwrapped); otherwise `parent = chain[i-1].Header`.
   2. **Height continuity.** If `h.Height != parent.Height + 1` (uint32) return
      `(chain[i:], E(op, Consensus, "block <hash> height <h.Height> does not follow parent height <parent.Height>"))`.
   3. **Positional difficulty.** `checkDifficultyPositional(dbtx, h, parent, chain)` (§4.2). Any error
      (consensus or database) returns `(chain[i:], E(op, err))`.
   4. **Proof of work, blake256.** `standalone.CheckProofOfWork(hash, h.Bits, params.PowLimit)` — note the
      hash checked is `n.Hash`, not a hash recomputed from the header.
   5. **Proof of work, blake3.** Only if step 4 failed: `CheckProofOfWork(h.PowHashV2(), h.Bits, params.PowLimit)`.
      If this fails too, return `(chain[i:], E(op, Consensus, err))` where `err` is the blake3 attempt's error
      (the blake256 error is discarded).
   6. **Stake difficulty.** Only if `deployments.DCP0001.Active(int32(h.Height), params.Net)`:
      `sdiff = nextRequiredDCP0001PoSDifficulty(dbtx, parent, chain)` (§4.5); a calculation error returns
      `(nil, E(op, err))`. If `dcrutil.Amount(h.SBits) != sdiff` return
      `(chain[i:], E(op, Consensus, "<hash> has invalid PoS difficulty, got <h.SBits as Amount>, want <sdiff>"))`.
      Amounts format as `dcrutil.Amount.String()` (e.g. `123.45678901 DCR`).
   7. `parent = h`.
3. Return `(nil, nil)`.

`standalone.CheckProofOfWork(hash, bits, powLimit)` (`dcrd/blockchain/standalone/v2@v2.3.0/pow.go:161-221`):
`target = CompactToBig(bits)`; error `ErrUnexpectedDifficulty` with message
`target difficulty of <%064x target> is too low` when `target <= 0`, or
`target difficulty of <%064x> is higher than max of <%064x powLimit>` when `target > powLimit`; error
`ErrHighHash` with message `proof of work hash <%064x> is higher than expected max of <%064x>` when the hash,
read as a little-endian 256-bit integer, is greater than `target`.

`DCP0001.Active(height, net)` (`deployments/deployments.go:32-74`): activation heights mainnet 149248,
testnet2 (`0x48e7a065`) 46128, testnet3 0, simnet 0; any other network is never active. Consequently no
stake difficulty validation happens on mainnet below height 149248 or on unrecognized networks.

What this validation does **not** check: header timestamps (median time, future limit), block version
rules, stake version, `Voters`/`FreshStake`/`Revocations`/`PoolSize` consistency, merkle roots, or the final
(contextual) choice of PoW algorithm. It also is not applied to blocks received through dcrd
`blockconnected` notifications (`chain/sync.go:885-891`).

### 4.2 Positional PoW difficulty — `checkDifficultyPositional`

`wallet/difficulty.go:361-541`. Inputs: `header` (block being checked), `prevNode` (its parent header),
`chain`. Runs inside the caller's DB view.

Constants derived per call:

| Name | Value |
| --- | --- |
| `rcai` | `int64(params.RuleChangeActivationInterval)` |
| `svh` | `params.StakeValidationHeight` |
| `firstPossibleActivationHeight` | `svh + 2*rcai` |
| `minBlake3BlockVersion` | 10 on mainnet, 11 on every other network |

Steps:

1. **Forced-active networks** (`isBlake3PowAgendaForcedActive()`, true only for simnet at the pinned
   chaincfg, `dcrd/chaincfg/v3@v3.3.0/simnetparams.go:438-463`):
   1. If `prevNode.Height == 0`: `want = params.WorkDiffV2Blake3StartBits`.
   2. Else `anchor = ancestorHeaderAtHeight(dbtx, prevNode, chain, 1)` (database error returned as is);
      `want = calcNextBlake3DiffFromAnchor(prevNode, anchor)` (§4.4).
   3. If `header.Bits != want` return `E(Consensus, err)` with `err` =
      `ErrUnexpectedDifficulty: block difficulty of <header.Bits> is not the expected value of <want> (difficulty algorithm: ASERT)`
      (the prefix is the `%w`-wrapped `standalone.ErrUnexpectedDifficulty`, whose text is the literal
      `ErrUnexpectedDifficulty`; both numbers are printed in decimal).
   4. Return nil.
2. `isBlake3PossiblyActive = uint32(header.Version) >= minBlake3BlockVersion && int64(header.Height) >= firstPossibleActivationHeight`.
3. `isSolvedBlake3 = standalone.CheckProofOfWorkHash(header.PowHashV2(), header.Bits) == nil` — a hash
   comparison only; the bits are not range checked here. Evaluated lazily, only when step 2 is true.
4. If `!isBlake3PossiblyActive || !isSolvedBlake3`: **EMA only.**
   `want = calcNextBlake256Diff(dbtx, prevNode, chain, header.Timestamp)` (§4.3; errors returned as is). If
   `header.Bits != want` return `E(Consensus, "ErrUnexpectedDifficulty: block difficulty of <bits> is not the expected value of <want> (difficulty algorithm: EMA)")`.
   Return nil.
5. **Cached candidate anchor.** `cached = loadCachedBlake3WorkDiffCandidateAnchor()`. If non-nil:
   `isAncestorOf(dbtx, cached, prevNode, chain)` (error returned as is). If it is an ancestor and
   `header.Bits == calcNextBlake3DiffFromAnchor(prevNode, cached)` return nil.
6. **Candidate search.** `finalNodeHeight = calcWantHeight(svh, rcai, int64(header.Height))`;
   `candidate = ancestorHeaderAtHeight(dbtx, prevNode, chain, int32(finalNodeHeight))`. While
   `candidate != nil && uint32(candidate.Version) >= minBlake3BlockVersion && int64(candidate.Height) >= firstPossibleActivationHeight - 1`:
   1. If `header.Bits == calcNextBlake3DiffFromAnchor(prevNode, candidate)`: store `candidate` in the cache
      and return nil.
   2. `candidate = relativeAncestor(dbtx, candidate, rcai, chain)` (one interval further back).
7. **Fallback to EMA.** Same as step 4, with the message
   `ErrUnexpectedDifficulty: block difficulty of <bits> is not the expected value of <want> (difficulty algorithm: EMA)`.

`calcWantHeight(svh, interval, height)` (`wallet/difficulty.go:345-353`), all int64 with Go truncated
division/remainder:

```
intervalOffset = svh % interval
adjusted       = height - intervalOffset - 1
want           = (adjusted - ((adjusted + 1) % interval)) + intervalOffset
```

`ancestorHeaderAtHeight(dbtx, h, chain, height)` (`wallet/difficulty.go:675-695`):

1. `height == int32(h.Height)` → `h`.
2. `height > int32(h.Height)` or `height < 0` → `(nil, nil)`.
3. `len(chain) > 0 && height - int32(chain[0].Header.Height) >= 0` → `chain[height - chain[0].Height].Header`.
4. Else the main-chain block at `height` (`GetMainChainBlockHashForHeight` then `GetBlockHeader`); a missing
   height is kind `NotExist`, message `no block at height <height> in main chain`.

`isAncestorOf(dbtx, node, target, chain)`:
`ancestorHeaderAtHeight(dbtx, target, chain, int32(node.Height)).BlockHash() == node.BlockHash()`. When
`node.Height > target.Height` the looked-up header is nil and the Go dereferences it (quirk Q-7).

The cache holds one `*wire.BlockHeader` per `Wallet` instance (a mutex-protected struct field, not a
package global), never invalidated, never persisted.

`isBlake3PowAgendaForcedActive()` (`wallet/difficulty.go:287-295`) looks up `chaincfg.VoteIDBlake3Pow`
(`"blake3pow"`) in `w.deploymentsByID` and returns `deployment.ForcedChoiceID == "yes"`; a network whose
parameters do not define the deployment returns false. The map is built once in `wallet.Open` from
`params.Deployments` across all stake versions (§2.9).

Errors returned by `checkDifficultyPositional` carry kind `Consensus` but **no op**; database errors
from the ancestor lookups and from `calcNextBlake256Diff` are returned as they come from `udb`. The
caller (§4.1 step 2.3) wraps both with op `wallet.validateHeaderChainDifficulties` and reports the
header as invalid either way, so a database failure during the positional check is indistinguishable
to the syncer from a consensus violation except by error kind.

### 4.3 EMA (blake256) difficulty — `calcNextBlake256Diff`

`wallet/difficulty.go:76-268`. Computes the required `Bits` for the child of `header` given the child's
timestamp `newBlockTime`. All big-number operations are Go `math/big`; `Div` is **Euclidean** division
(remainder always non-negative), `Rsh` on a negative value is an arithmetic shift (rounds toward negative
infinity). Parameters from `chaincfg.Params`.

1. `oldDiff = header.Bits`; `oldDiffBig = CompactToBig(header.Bits)`; `nextHeight = int64(header.Height) + 1`.
2. **Not a retarget height** (`nextHeight % WorkDiffWindowSize != 0`):
   1. If `ReduceMinDifficulty && (!isTestNet3 || nextHeight < 962928)`:
      1. `allowMinTime = header.Timestamp.Unix() + int64(MinDiffReductionTime / time.Second)`.
      2. If `newBlockTime.Unix() > allowMinTime` return `PowLimitBits`.
      3. Else return `findPrevTestNetDifficulty(dbtx, header, chain)`.
   2. Else return `oldDiff`.
3. **Retarget.** `nextDiffBigMin = oldDiffBig / RetargetAdjustmentFactor`;
   `nextDiffBigMax = oldDiffBig * RetargetAdjustmentFactor`; `alpha = WorkDiffAlpha`;
   `windowChanges = [WorkDiffWindows]big`; `olderTime = 0`, `windowPeriod = 0` (int64); `weights = 0` (uint64);
   `oldHeight = header.Height` (uint32); `recentTime = header.Timestamp.Unix()`.
4. Loop `i = 0, 1, 2, ...`:
   1. If `i != 0`:
      1. `timeDifference = recentTime - olderTime`; if `oldHeight == 0` then
         `timeDifference = int64(TargetTimespan / time.Second)`.
      2. `windowAdjusted = (timeDifference << 32) Div int64(TargetTimespan / time.Second)`.
      3. `shift = (WorkDiffWindows - windowPeriod) * alpha`; `windowAdjusted <<= shift`;
         `weights += 1 << uint64(shift)`.
      4. `windowChanges[windowPeriod] = windowAdjusted`; `windowPeriod++`; `recentTime = olderTime`.
   2. If `i == WorkDiffWindows` break.
   3. If `int64(oldHeight) > WorkDiffWindowSize` then `oldHeight -= WorkDiffWindowSize` else `oldHeight = 0`.
   4. If `oldHeight != 0`: if `len(chain) > 0 && int32(oldHeight) - int32(chain[0].Header.Height) >= 0` then
      `olderTime = chain[oldHeight - chain[0].Header.Height].Header.Timestamp.Unix()`; else
      `olderTime = ` timestamp of the main-chain block at height `oldHeight`
      (`GetMainChainBlockHashForHeight`, `GetBlockHeaderTime`; errors returned). If `oldHeight == 0`,
      `olderTime` keeps its previous value.
5. `weightedSum = Σ windowChanges[0..WorkDiffWindows)`; `q = weightedSum Div int64(weights)`;
   `nextDiffBig = (q * oldDiffBig) Rsh 32`.
6. Clamp, in this order (first matching branch only):
   1. `oldDiffBig == 0` → unchanged.
   2. `nextDiffBig == 0` → `PowLimit`.
   3. `nextDiffBig > nextDiffBigMax` → `nextDiffBigMax`.
   4. `nextDiffBig < nextDiffBigMin` → `nextDiffBigMin`.
7. If `nextDiffBig > PowLimit` → `PowLimit`.
8. Testnet max difficulty: if `minTestNetTarget != nil && nextDiffBig < minTestNetTarget && (!isTestNet3 || nextHeight >= 962928)`
   → `minTestNetTarget`. (`minTestNetTarget` is non-nil only on testnet3, so the rule is exactly "testnet3
   and `nextHeight >= 962928`".)
9. Return `BigToCompact(nextDiffBig)`.

`findPrevTestNetDifficulty(dbtx, h, chain)` (`wallet/difficulty.go:40-68`):

1. `blocksPerRetarget = WorkDiffWindowSize * WorkDiffWindows`.
2. While `int64(h.Height) % blocksPerRetarget != 0 && h.Bits == PowLimitBits`:
   1. If `h.PrevBlock` is the zero hash: `h = nil`, break.
   2. If `len(chain) > 0 && int32(h.Height) - int32(chain[0].Header.Height) > 0`:
      `h = chain[h.Height - chain[0].Header.Height - 1].Header`; else `h = GetBlockHeader(h.PrevBlock)`
      (by hash; error returned).
3. Return `h.Bits`, or `PowLimitBits` when `h == nil`.

The wallet replaces dcrd's parent-pointer traversal with height arithmetic: for heights at or above
`chain[0]` the header comes from `chain`, below it from the main-chain height index. The result equals
dcrd's as long as the parent of `chain[0]` is a main-chain block, which §4.1 step 1 guarantees.

### 4.4 ASERT (blake3) difficulty — `calcNextBlake3DiffFromAnchor`

`wallet/difficulty.go:302-340`.

```
timeDelta   = prevNode.Timestamp.Unix() - anchor.Timestamp.Unix()
heightDelta = int64(prevNode.Height) - int64(anchor.Height)
next = standalone.CalcASERTDiff(params.WorkDiffV2Blake3StartBits, params.PowLimit,
                                int64(params.TargetTimePerBlock.Seconds()),
                                timeDelta, heightDelta, params.WorkDiffV2HalfLifeSecs)
if minTestNetTarget != nil && next < minTestNetDiffBits { next = minTestNetDiffBits }
return next
```

The testnet clamp compares **compact encodings as integers** (`uint32 <`), not targets, and unlike the EMA
path has no activation-height condition.

`minTestNetTarget = PowLimit >> 6` and `minTestNetDiffBits = BigToCompact(minTestNetTarget)`, set at
`wallet.Open` for testnet3 only (`wallet/wallet.go:5361-5372`).

`CalcASERTDiff` (`dcrd/blockchain/standalone/v2@v2.3.0/pow.go:253-405`) panics when the start bits are
outside `[1, powLimit]` or `heightDelta < 0`. With the anchors chosen in §4.2 `heightDelta >= 0` always
holds (anchors are at or below `prevNode`). Its arithmetic (64.16 fixed point exponent with truncating
`Quo`, cubic approximation with coefficients 195766423245049, 971821376, 5127, rounding term `1<<47`, shift
by `n-16`, clamp to `[1, powLimit]`, `BigToCompact`) is already implemented in
`dcroxide:crates/dcroxide-standalone/src/pow.rs:221`.

### 4.5 Stake difficulty (DCP0001) — `nextRequiredDCP0001PoSDifficulty`

`wallet/difficulty.go:725-785` (helpers at `:550-720`). Input `curHeader` (may be nil) and `chain`.

1. `nextHeight = 0` if `curHeader == nil` else `int64(curHeader.Height) + 1`.
2. If `nextHeight < int64(CoinbaseMaturity) + 1` return `MinimumStakeDiff`.
3. `curDiff = curHeader.SBits`. If `nextHeight % StakeDiffWindowSize != 0` return `curDiff`.
4. `prevRetargetHeight = nextHeight - StakeDiffWindowSize - 1`;
   `prevRetargetHeader = ancestorHeaderAtHeight(dbtx, curHeader, chain, int32(prevRetargetHeight))` (nil when
   negative). `prevPoolSize = prevRetargetHeader.PoolSize` or 0 when nil.
5. `prevImmature = sumPurchasedTickets(dbtx, prevRetargetHeader, chain, TicketMaturity)`.
6. `prevPoolSizeAll = prevPoolSize + prevImmature`. If 0 return `curDiff`.
7. `immature = sumPurchasedTickets(dbtx, curHeader, chain, TicketMaturity)`;
   `curPoolSizeAll = int64(curHeader.PoolSize) + immature`.
8. Return `calcNextStakeDiffV2(params, nextHeight, curDiff, prevPoolSizeAll, curPoolSizeAll)`.

`sumPurchasedTickets(dbtx, start, chain, numToSum)` (`wallet/difficulty.go:581-600`):

```
sum = 0
for h, n := start, 0; h != nil && n < numToSum; n++ {
    sum += int64(h.FreshStake)
    if h.PrevBlock is the zero hash { break }
    if len(chain) > 0 && int32(h.Height) - int32(chain[0].Header.Height) > 0 {
        h = chain[h.Height - chain[0].Header.Height - 1].Header
        continue
    }
    h = GetBlockHeader(h.PrevBlock)      // by hash; error returned
}
```

`calcNextStakeDiffV2` (`wallet/difficulty.go:611-673`):

```
targetPoolSizeAll = TicketsPerBlock * (TicketPoolSize + TicketMaturity)
next = curDiff * curPoolSizeAll * curPoolSizeAll        (big)
next = next Div prevPoolSizeAll                         (big, Euclidean)
next = next Div targetPoolSizeAll
nextDiff = next.Int64()                                 (Go leaves out-of-range results undefined; see Q-22)
maximum  = estimateSupply(params, nextHeight) / TicketPoolSize      (int64 truncated)
if nextDiff > maximum          { nextDiff = maximum }
if nextDiff < MinimumStakeDiff { nextDiff = MinimumStakeDiff }
```

`estimateSupply(params, height)` (`wallet/difficulty.go:550-576`), int64:

```
if height <= 0 { return 0 }
supply     = params.BlockOneSubsidy()
reductions = height / SubsidyReductionInterval
subsidy    = BaseSubsidy
repeat reductions times:
    supply  += SubsidyReductionInterval * subsidy
    subsidy  = subsidy * MulSubsidy / DivSubsidy
supply += (1 + height % SubsidyReductionInterval) * subsidy
supply -= BaseSubsidy * 2
```

Public wrappers:

* `NextStakeDifficulty(ctx)`: one DB view. Read `(tipHash, tipHeight)`; if `!DCP0001.Active(tipHeight, net)`
  fail with `E(Deployment, "DCP0001 is not known to be active")`; load the tip header; compute with
  `chain = nil`. Any error is wrapped with op `wallet.NextStakeDifficulty`.
* `NextStakeDifficultyAfterHeader(ctx, h)`: fail before opening a transaction with
  `E(op, Deployment, "DCP0001 is not known to be active")` when `!DCP0001.Active(int32(h.Height), net)`;
  otherwise compute with `chain = nil` in one DB view.

### 4.6 Sidechain forest

`wallet/sidechains.go`. The forest is a slice of trees; order is significant for duplicate detection and
for tie-breaking in §4.7. Removal anywhere is swap-with-last then truncate, which reorders the slice.

#### 4.6.1 Tree invariants

* `root` is either the first block of a fork off the main chain or an orphan.
* `children[hash]` holds every non-root node; `tips[hash]` holds every non-root node without children in
  the tree. A tree consisting of only a root has empty `children` and `tips`.
* `workSum(root) = CalcWork(root.Bits)`; `workSum(n) = workSum(n.parent) + CalcWork(n.Bits)`.
  `CalcWork(bits) = floor(2^256 / (CompactToBig(bits) + 1))`, or 0 when the target is not positive.
* `invalid(n) = n.invalid || invalid(n.parent)` at attach time.

#### 4.6.2 `AddBlockNode(n)`

1. For each tree `t` in slice order:
   1. If `t.duplicateNode(n)` return false. (`duplicateNode`: the existing node is the root when hashes are
      equal, else `children[n.Hash]`; when found and `n.FilterV2 != nil` and the existing node's filter is
      nil, copy the filter pointer to the existing node.)
   2. If `t.maybeAttachNode(n)`: `nodeTree = t`; stop iterating.
2. If no tree accepted `n`: `nodeTree = newSideChainRootedTree(n)`; append to the forest.
3. Merge orphans: scan the forest with index `i` from 0; for each tree whose
   `root.Header.PrevBlock == *n.Hash`:
   1. Collect its root and all children, sort by `Header.Height` ascending (`sort.Slice`, unstable).
   2. For each collected node, in order: if `nodeTree.duplicateNode(node) || !nodeTree.maybeAttachNode(node)`
      **panic** with `sidechain forest internal consistency error`.
   3. Swap-remove the orphan tree and re-examine index `i` (do not advance).
4. Return true.

Duplicate detection and attachment are interleaved per tree: the loop stops at the first tree that
accepts `n`, so trees later in the slice are not searched for a duplicate of `n`. The forest invariants
make that unreachable for consistent input (a tree whose root is `n` would have been merged when `n`'s
parent arrived, or `n` would have attached under its parent instead of starting a tree).

Because the merge condition tests only `PrevBlock` while `maybeAttachNode` also requires
`Height == parent.Height + 1`, adding a node whose height is inconsistent with a waiting orphan child
panics. Callers must only add nodes whose heights are consistent (SPV validates before adding; the RPC
syncer adds dcrd-supplied headers before validating them, `chain/sync.go:281-322`).

#### 4.6.3 `maybeAttachNode(n)`

1. If `*t.root.Hash == n.Header.PrevBlock && n.Header.Height == t.root.Header.Height + 1`: set `n.parent = root`,
   `n.invalid ||= root.invalid`, add to `children` and `tips`, set `workSum`, clear `bestChain`, return true.
2. Else if `parent = children[n.Header.PrevBlock]` exists and `n.Header.Height == parent.Header.Height + 1`:
   same, and delete `parent` from `tips`. Return true.
3. Return false.

#### 4.6.4 `best()`

1. Panic `no best chain for invalid sidechain tree` if `root.invalid`.
2. If a memoized `bestChain` is non-empty return it with the work sum of its last element.
3. Over `tips` (Go map order): skip invalid tips; keep the first tip seen, replace it only when a later tip
   has strictly greater `workSum`. Equal-work tips resolve nondeterministically.
4. No valid tip: memoize and return `[root]`, `root.workSum`. (This also happens when the tree has children
   but every tip is invalid; valid intermediate nodes are then not offered.)
5. Otherwise build the path root..tip by following `parent` for `tip.Height - root.Height + 1` elements,
   memoize and return it with `tip.workSum`.

#### 4.6.5 `PruneChain(chain)` and `pruneChain`

`PruneChain`: find the first tree with `*chain[0].Hash == *tree.root.Hash`; if none, do nothing. Swap-remove
it, call `tree.pruneChain(chain)` and append the returned trees to the forest.

`pruneChain(chain)`:

1. Delete every node of `chain` from `children` and `tips`; clear `bestChain`.
2. If `tips` is now empty return nil (the tree is gone).
3. For each remaining tip (Go map order):
   1. `root = tip`; while `root.parent != nil && children[root.parent.Hash] != nil`: `root = root.parent`.
   2. Find an already-created result tree with this exact `root` pointer, or create one with
      `newSideChainRootedTree(root)` (which resets `root.workSum` to the root's own work; `root.parent` and
      `root.invalid` are left as they were).
   3. Collect nodes from `tip` up to (excluding) `root`, stopping early at a node already in the result
      tree's `children`; attach them to the result tree in root-to-tip order with `maybeAttachNode`
      (recomputing `workSum`).
4. Return the result trees (order follows map iteration).

#### 4.6.6 `Prune`, `PruneAll`, `HasSideChainBlock`

* `Prune(mainChainHeight, params)`: remove every tree with
  `int32(root.Header.Height) + int32(params.CoinbaseMaturity) < mainChainHeight`. The whole tree goes
  regardless of how tall it is.
* `PruneAll()`: `trees = nil`.
* `HasSideChainBlock(hash)`: true iff `hash` equals a root hash or is a key of some `children`.

#### 4.6.7 `FullSideChain(newBlocks)`

1. Empty input → return it unchanged.
2. `wantParent = newBlocks[0].Header.PrevBlock`. For each tree in order:
   1. `wantParent == root.Hash` → return `[root] ++ newBlocks` (fresh slice).
   2. `parent = children[wantParent]`; absent → next tree.
   3. If `parent.Header.Height <= root.Header.Height` → error `broken assumption of height > parent.Height`.
   4. `prefixLen = parent.Height - root.Height + 1`; fill the prefix from `parent` back to the root by
      following `parent` pointers; a nil pointer before the prefix is full → error
      `broken assumption about connectedness of sidechain nodes`.
   5. Return `prefix ++ newBlocks`.
3. No tree matched → return `newBlocks`.

Both errors are `E("wallet.EvaluateBestChain", msg)` — the op string is that of a different function
(`wallet/sidechains.go:363`).

### 4.7 Best chain selection — `EvaluateBestChain`

`wallet/sidechains.go:417-465`. One DB view.

1. Read `tipHash` and the tip header. `workDiff = 0`; `best = nil`.
2. For each tree `t` in forest order:
   1. Skip if `t.root.invalid`.
   2. `fork = t.root.Header.PrevBlock`; skip if `fork` is not a main-chain block (orphan tree).
   3. `(chain, chainWork) = t.best()`.
   4. `work = 0`. Walk from the tip toward genesis: starting with `(hash, header) = (tipHash, tipHeader)`,
      while `hash != fork`: `work -= CalcWork(header.Bits)`; `header = GetBlockHeader(header.PrevBlock)`
      (error aborts the call); `hash = previous PrevBlock`.
   5. `work += chainWork`.
   6. If `work > workDiff` (strict): `best = chain`, `workDiff = work`.
3. Return `best` (nil when no tree strictly beats the main chain). Errors are wrapped with op
   `wallet.EvaluateBestChain`.

Consequences: a sidechain with work exactly equal to the main-chain blocks it would replace never wins
(first-seen rule). Among several winning trees, the one with the largest surplus wins; equal surpluses
resolve to the earlier tree in forest order. The returned slice aliases the tree's memoized `bestChain`.

### 4.8 Chain switch — `ChainSwitch`

`wallet/chainntfns.go:91-316`. Op `wallet.ChainSwitch`.

Inputs: `forest` (the caller's forest; mutated at the end), `chain` (non-empty; `chain[0]`'s parent must be a
main-chain block; every node must carry a non-nil `FilterV2`).

**Before the transaction**

1. `len(chain) == 0` → return `E(op, Invalid, "zero-length chain")`.
2. Build `MainTipChangedNotification{ AttachedBlocks: empty (cap len(chain)), DetachedBlocks: nil, NewHeight: int32(chain[last].Header.Height) }`.
3. `sideChainForkHeight = int32(chain[0].Header.Height)`; `newWork = chain[last].workSum` (nil if the node
   was never placed in a tree — dereferenced only on the reorg path); `oldWork = 0`.
4. Lock `w.lockedOutpointMu`.

**One DB update** (steps 5-12; any returned error rolls the database back):

5. `(tipHash, tipHeight) = MainChainTip`. If `tipHash == *chain[last].Hash` return nil from the closure (no
   database change; the post-transaction steps still run).
6. **Detach** — only if `sideChainForkHeight <= tipHeight`:
   1. Allocate `DetachedBlocks` and `prevChain` with `tipHeight - sideChainForkHeight + 1` elements.
   2. For `i = tipHeight` down to `sideChainForkHeight`: load the main-chain hash at `i`, its header, and its
      cfilter (`CFilterV2`); store the hash pointer at `DetachedBlocks[i - forkHeight]` and
      `NewBlockNode(header, &hash, filter, nil)` at `prevChain[i - forkHeight]` (both ascending by height);
      call `NtfnServer.notifyDetachedBlock(header)` (so detached-block notifications are queued in
      **descending** height order); `oldWork += CalcWork(header.Bits)`.
   3. If `newWork.Cmp(oldWork) != 1` return the error
      `failed reorganize: sidechain ending at block <chain[last].Hash> has less total work than the main chain tip block <tipHash>`
      (plain formatted error, no kind).
   4. `removedTxs = txStore.Rollback(dbtx, sideChainForkHeight)` — removes blocks at and above the fork
      height, moves their non-coinbase transactions to the unmined set, and returns every removed
      transaction keyed by former block hash.
   5. For each `n` in `prevChain`: `n.RelevantTxs = removedTxs[*n.Hash]` (nil when the block held no wallet
      transactions).
7. `birthState = udb.BirthState(dbtx)` (read once).
8. **Attach** — for each `n` in `chain`, in order:
   1. If `voteVersion(params) < n.Header.StakeVersion` log at warn:
      `Old vote version detected (v<voteVersion>), please update your wallet to the latest version.`
      (`voteVersion`: mainnet 11, testnet2 6, testnet3 12, simnet 12, other 1; `wallet/wallet.go:291-304`).
   2. `watch = extendMainChain(ctx, op, dbtx, n)` (§4.9); error returned. Append `watch` to
      `watchOutPoints`. Append `n.Hash` to `AttachedBlocks`.
   3. **Birthday resolution.** If `birthState != nil` and
      `((SetFromTime && n.Header.Timestamp.After(birthState.Time)) || (SetFromHeight && n.Header.Height == birthState.Height + 1))`
      and `n.Header.Height != 0`:
      set `Hash = n.Header.PrevBlock`, `Height = n.Header.Height - 1`, clear both flags,
      `udb.SetBirthState`, then `UpdateProcessedTxsBlockMarker(dbtx, &n.Header.PrevBlock)`, then log at info
      `Set wallet birthday to block <height> (<hash>).` Because the flags are cleared in the in-memory
      copy, this fires at most once per call.
9. **Marker.** `marker = nil`; iterate `chain` from the start, stop at the first node with
   `RelevantTxs == nil`, otherwise `marker = n.Hash`. If `marker != nil`:
   1. `r = rescanPoint(dbtx)` (§4.14); `rHeader = GetBlockHeader(dbtx, r)`.
   2. If **not** `rHeader.Height + 1 < chain[0].Header.Height`: log at debug
      `Updating processed txs block marker to <marker>` and `UpdateProcessedTxsBlockMarker(dbtx, marker)`.
      (The marker is left alone when there is a gap between the current rescan point and the first block
      attached here.)
10. **Next ticket price for pruning.** `tip = chain[last]`. If `!DCP0001.Active(tipHeight, net)` — note
    `tipHeight` is the tip height read in step 5, i.e. *before* this switch — the price is unknown (error
    kind `Deployment`, `DCP0001 is not known to be active`); otherwise
    `sdiff = nextRequiredDCP0001PoSDifficulty(dbtx, tip.Header, nil)`. On any error log at error
    `Unable to prune unmined tickets: next ticket price is unknown: <err>` and use `sdiff = 0`.
11. `hashes = txStore.PruneUnmined(dbtx, sdiff)` — removes expired unmined transactions, unmined tickets
    priced differently from `sdiff` (when non-zero) and unmined votes not voting on the tip. An error is
    logged at error (`Failed to prune unmined transactions when connecting block height <tip height>: <err>`)
    and otherwise ignored; `hashes` returned alongside an error are still processed.
12. For each pruned hash: `NtfnServer.notifyRemovedTransaction(hash)`; for every VSP client in
    `w.vspClients` (map order, read without `vspClientsMu`) `RemoveTicket(hash, "pruned")`;
    `udb.DeleteVSPTicket(dbtx, hash)` with errors logged at error
    (`Failed to remove pruned ticket from db: <err>`).

**After the transaction**

13. Unlock `w.lockedOutpointMu`. If the update failed return `(nil, E(op, err))`.
14. If `AttachedBlocks` is non-empty: under `recentlyPublishedMu`, delete from `w.recentlyPublished` the hash
    of every transaction in every `chain[i].RelevantTxs`.
15. If a network backend is set:
    1. `watchHDAddrs(ctx, false, n)` (§4.17); an error returns `(nil, E(op, err))` — the database switch
       has already committed and the steps below are skipped.
    2. If `watchOutPoints` is non-empty: `n.LoadTxFilter(ctx, false, nil, watchOutPoints)`; error logged at
       error (`Failed to watch outpoints: <err>`), not returned.
16. `forest.PruneChain(chain)`; `forest.Prune(int32(chain[last].Header.Height), params)`.
17. `w.expireMixMessages(chain[last].Header.Height)` (no-op when no mixing client is running).
18. `NtfnServer.notifyMainChainTipChanged(notification)` then
    `NtfnServer.sendAttachedBlockNotification(ctx)`.
    `sendAttachedBlockNotification` runs its own DB view (only when a transaction-notification
    client is registered) to build the combined transaction notification; its failure is logged at
    error (`Failed to construct attached blocks notification: <err>`, `wallet/notifications.go:343-346`)
    and does not affect the return value.
19. Return `(prevChain, nil)`. The returned nodes have `parent == nil`, `workSum == nil` and
    `invalid == false`; their `RelevantTxs` were set in step 6.5. Callers re-add every node of `prevChain` to the forest
    (`spv/sync.go:1546-1552,1820-1825`, `chain/sync.go:331-338,896-901`).

Work comparison detail: `newWork` is `chain[last].workSum`, which is cumulative from the **root of the
tree the node lives in**, while `oldWork` sums the main-chain blocks from `chain[0].Header.Height` up.
The two are commensurable only when `chain[0]` is that tree's root, which holds for every chain returned
by `EvaluateBestChain`. A caller passing a suffix of a best chain (the SPV initial-sync stage) never
reaches the comparison because a suffix starts above the tip. Errors inside the update closure other
than those from `extendMainChain` (which already carry the op, except the checkpoint error of §4.9
step 1) are returned bare and wrapped once with
op `wallet.ChainSwitch` in step 13.

### 4.9 `extendMainChain`

`wallet/chainntfns.go:29-83`. Runs inside the `ChainSwitch` update.

1. **Checkpoint.** `ckpt = CheckpointHash(params.Net, int32(n.Header.Height))`. If non-nil and different from
   `n.Hash` return `E(Consensus, "block hash <hash> does not satisfy checkpoint hash <ckpt> for height <height>")`
   (no op at this level).
2. `txStore.ExtendMainChain(txmgrNs, n.Header, n.Hash, n.FilterV2)`. An error of kind `Exist` (block already
   recorded in the main chain) is ignored; any other error is returned as `E(op, err)`. Other
   `ExtendMainChain` failures: `Invalid` `block 0 cannot be added`; `Invalid`
   `main chain may not be extended without first saving all previous cfilters`; `Invalid`
   `not direct child of current tip`; `Invalid` `invalid height for next block`.
3. `NtfnServer.notifyAttachedBlock(n.Header, n.Hash)`.
4. `blockMeta = GetBlockMetaForHash(n.Hash)` (hash, height, header time, vote bits); error returned as
   `E(op, err)`. Steps 3 and 4 run even when step 2 hit the tolerated `Exist` case, so an already-recorded
   block is notified as attached again and its `RelevantTxs` are processed again.
5. For each `tx` in `n.RelevantTxs`, in order:
   1. If `w.manualTickets && stake.IsSStx(tx)` and the transaction is not already recorded (mined or
      unmined, `ExistsTx`): skip it.
   2. `rec = udb.NewTxRecordFromMsgTx(tx, time.Now())`; a serialization error is returned as `E(op, err)`.
   3. `ops = processTransactionRecord(ctx, dbtx, rec, n.Header, &blockMeta)`; error returned as `E(op, err)`.
      Accumulate `ops`.
6. Return the accumulated outpoints.

### 4.10 `processTransactionRecord`

`wallet/chainntfns.go:438-676`. Op `wallet.processTransactionRecord`. Caller holds a DB update and (except
for votes from `VoteOnOwnedTickets`) `w.lockedOutpointMu`. `header == nil` means "unmined"; then
`blockMeta` is nil too.

1. **Insert the record.**
   * Unmined: `txStore.InsertMemPoolTx(dbtx, rec)`. Kind `Exist` (already recorded as mined) → log at warn
     `Refusing to add unmined transaction <hash> since same transaction already exists mined` and return
     `(nil, nil)`.
   * Mined: `txStore.InsertMinedTx(dbtx, rec, &blockMeta.Hash)`.
   * Any other error → `E(op, err)`.
2. `skipOutpoints = rec.TxType is SSGen or SSRtx`.
3. **Inputs** — for each input `i`:
   1. Unless `skipOutpoints`: delete `{PreviousOutPoint.Hash, PreviousOutPoint.Index}` from
      `w.lockedOutpoints` (in-memory; not undone if the transaction later fails).
   2. If `stdscript.IsMultiSigSigScript(0, input.SignatureScript)` (the signature script ends in a data push
      that parses as a standard multisig script; script version is assumed 0):
      1. `rs = MultiSigRedeemScriptFromScriptSigV0(sigScript)`; `(class, addrs) = ExtractAddrs(0, rs, params)`;
         if `class != STMultiSig` continue with the next input.
      2. For each address: `manager.Address`; `NotExist` → skip; other error → `E(op, err)`; found →
         `isRelevant = true` and `markUsedAddress` (step 6 below).
      3. If `isRelevant`: `manager.ImportScript(addrmgrNs, rs)`. `Exist` → nothing; other error → `E(op, err)`;
         success and a network backend is set → `n.LoadTxFilter(ctx, false, [scriptAddress], nil)`, whose
         error is returned as `E(op, err)` (aborting the enclosing database update).
      4. `mso, err = txStore.GetMultisigOutput(txmgrNs, &input.PreviousOutPoint)`; if found without error →
         `SpendMultisigOut(txmgrNs, &prevOut, rec.Hash, uint32(i))`, error → `E(op, err)`. Lookup errors are
         ignored.
4. **Outputs** — for each output `i`:
   1. `(class, addrs) = stdscript.ExtractAddrs(output.Version, output.PkScript, params)`. `STNonStandard` →
      skip the output.
   2. `(subClass, isStakeType) = txrules.StakeSubScriptType(class)`: stake-tagged P2PKH classes
      (submission, change, gen, revocation, treasury gen) map to `STPubKeyHashEcdsaSecp256k1`; stake-tagged
      P2SH classes map to `STScriptHash`; `class = subClass` when stake.
   3. `isTicketCommit = rec.TxType == SStx && i % 2 == 1`.
      * Ticket commitment: `addr = stake.AddrFromSStxPkScrCommitment(output.PkScript, params)` (bytes
        `[2:22]` are the hash160; bit 7 of byte 29 selects P2SH; scripts shorter than the minimum commitment
        size fail). On failure log at warn `Failed to decode ticket commitment script of <txhash>:<i>` and skip
        the output. Otherwise `addrs = [addr]`, `watchOutPoint = false`.
      * Not a commitment and `output.Value == 0` → skip the output.
   4. `tree = 1` if `isStakeType` else 0; `outpoint = {Hash: rec.Hash, Tree: tree}`.
   5. For each address: `manager.Address`; `NotExist` → skip; other error → `E(op, err)`. Found:
      * commitment → `txStore.AddTicketCommitment(txmgrNs, rec, uint32(i), ma.Account())`;
      * otherwise → `txStore.AddCredit(dbtx, rec, blockMeta, uint32(i), ma.Internal(), ma.Account())`;
      * error → `E(op, err)`; then `markUsedAddress(op, dbtx, ma)`; then, when `watchOutPoint`, append
        `outpoint` with `Index = i` to the result (once per matching address, so a bare multisig output with
        two owned keys is appended twice).
   6. If `class == STScriptHash`: for each address try `manager.RedeemScript`; failures log at debug
      `Failed to find redeemscript for address <addr> in address manager: <err>`. The script variable is
      assigned on every iteration, including failed ones (where `RedeemScript` returns nil), so it holds
      the result for the **last** address; a P2SH output has exactly one address, so this is the script
      for that address or nil. A nil script classifies as non-standard and ends processing of the
      output. With that script, `(expClass, multisigAddrs) = ExtractAddrs(0, script, params)`; if `expClass != STMultiSig` go to
      the next output. For each multisig address that `manager.Address` finds without error:
      `txStore.AddMultisigOut(dbtx, rec, blockMeta, uint32(i))`; errors logged at debug
      `Unable to add multisignature output: <err>` (expected when several keys are owned).
5. If `rec.TxType` is SSGen or SSRtx: `txStore.RedeemTicketCommitments(txmgrNs, rec, blockMeta)`; error logged
   at error `Error redeeming ticket commitments: <err>`, not returned.
6. **`markUsedAddress(op, dbtx, ma)`** (`wallet/addresses.go:718-746`), invoked from steps 3 and 4:
   `manager.MarkUsed(dbtx, address)` (raises the branch's last-used child, and last-returned if lower; no
   effect for imported or script addresses). If `ma.Account()` is the imported account (`2^31 - 1`) stop.
   Otherwise read the account properties, take the branch's last-used index and call
   `manager.SyncAccountToAddrIndex(ns, account, min(2^31 - 1, lastUsed + gapLimit), branch)` so that address
   rows exist through `lastUsed + gapLimit`. Log at debug `Marked address <addr> used`.
7. **Notification.** Unmined: `details = txStore.UniqueTxDetails(txmgrNs, &rec.Hash, nil)` then
   `NtfnServer.notifyUnminedTransaction(dbtx, details)`. Mined: `UniqueTxDetails(..., &blockMeta.Block)` then
   `notifyMinedTransaction(dbtx, details, blockMeta)`. A lookup error is logged at error
   `Cannot query transaction details for notification: <err>` and the notification is dropped.
8. Return the outpoints to watch.

Error wrapping: errors from `markUsedAddress` are returned as produced (that helper wraps with the op
it is handed, here `wallet.processTransactionRecord`); every other returned error is `E(op, err)`.
`markUsedAddress` steps and messages: `MarkUsed` error, `AccountProperties` error and
`SyncAccountToAddrIndex` error are each `E(op, err)`. The multisig signature-script test, redeem-script
parse and P2SH expansion all assume script version 0 (`scriptVersionAssumed`) regardless of the real
version of the previous output (`TODO` at `wallet/chainntfns.go:474`). An imported multisig script's
address is **not** marked used (`TODO` at `wallet/chainntfns.go:503`).

**Stake transaction detection summary**

| Transaction (`stake.DetermineTxType`) | Wallet effects in this function |
| --- | --- |
| Ticket (SStx) | output 0 (stake submission, tree 1) credited when its voting address is a wallet address; odd outputs: commitment recorded when the committed reward address is a wallet address (never watched as an outpoint); even outputs ≥ 2 (stake change, tree 1) credited when non-zero and owned |
| Vote (SSGen) | inputs do not unlock outpoints; outputs ≥ 2 (stake gen, tree 1) credited when owned; outputs 0 and 1 are null-data with value 0 and are skipped; ticket commitments redeemed |
| Revocation (SSRtx) | inputs do not unlock outpoints; outputs (stake revocation, tree 1) credited when owned; ticket commitments redeemed |
| Treasury spend/add/base | outputs classified through `ExtractAddrs`; treasury-gen outputs are stake type (tree 1) |

Ownership of a ticket for voting purposes is decided elsewhere: `txStore.OwnTicket` (a ticket record
exists) and `hasVotingAuthority` (output 0's hash160 is a wallet address, and whether its private key is
available) — §4.12.

### 4.11 `AddTransaction`

`wallet/chainntfns.go:324-436`. Op `wallet.AddTransaction`.

1. If `tx.TxHash()` is in `w.recentlyPublished` (under `recentlyPublishedMu`) return nil. The entry is not
   removed here.
2. If `stake.IsTSpend(tx)` and `blockHash == nil`: log at debug `Ignoring unmined TSPend <hash>`; return nil.
3. Lock `w.lockedOutpointMu`. **One DB update:**
   1. `rec = NewTxRecordFromMsgTx(tx, time.Now())`.
   2. If `stake.IsSSGen(tx)` and `blockHash == nil`: `(votedBlock, _) = stake.SSGenBlockVotedOn(tx)` (bytes
      `[2:34]` of output 0's script). If it differs from the current main-chain tip hash, log at debug
      `Rejected unmined orphan vote <txhash> which votes on block <votedBlock>` and return nil.
   3. If `blockHash != nil` and it is a main-chain block: load its header and block meta (errors
      returned). If it is not a main-chain block the transaction is treated as **unmined** for everything
      below.
   4. **Unmined input checks** (only when no header was loaded). For each input `i`:
      `credit = txStore.UnspentOutput(txmgrNs, PreviousOutPoint, includeMempool = true)`. Kind `NotExist` →
      not a wallet UTXO, skip. Other error → `E(op, err)`. If `credit.OutPoint != PreviousOutPoint` (the
      database key ignores the tree, so this detects a wrong tree) →
      `E(op, ScriptFailure, "input <i> spends <prevOut> from the wrong transaction tree")`. Then
      `validateTxInput(op, tx, i, credit.PkScript)`: run the script engine with flags
      `ScriptDiscourageUpgradableNops | ScriptVerifyCleanStack | ScriptVerifyCheckLockTimeVerify | ScriptVerifyCheckSequenceVerify | ScriptVerifyTreasury`
      (`wallet/createtx.go:45-49`), script version 0, no signature cache. Engine creation error →
      `E(op, err)`. Execution error → log at error
      `Script validation failed (outpoint <prevOut> pkscript <hex> sigscript <hex>): <err>` and return
      `E(op, ScriptFailure, err)`.
   5. `watchOutPoints = processTransactionRecord(ctx, dbtx, rec, header, meta)`.
4. Unlock. On error return `E(op, err)`.
5. If a network backend is set **and** `watchOutPoints` is non-empty: `watchHDAddrs(ctx, false, n)` (error →
   `E(op, err)`), then `n.LoadTxFilter(ctx, false, nil, watchOutPoints)` (error logged at error
   `Failed to watch outpoints: <err>`).
6. Return nil.

Unlike block processing, this function adds tickets even in manual-ticket mode; callers filter
(`chain/sync.go:919-921`).

### 4.12 `VoteOnOwnedTickets`

`wallet/chainntfns.go:698-925`. Op `wallet.VoteOnOwnedTickets`. Vote construction and signing details
(`createUnsignedVote`, `signVote`) belong to the staking spec; this section fixes the orchestration.

1. If `!w.votingEnabled` or `blockHeight < int32(StakeValidationHeight) - 1` return nil.
2. `n = NetworkBackend()`; error (`NoPeers`) → `E(op, err)`. If `n` does not implement
   `deployments.Querier` → `E(op, "network backend does not provide deployment information")`.
3. `dcp0010Active = deployments.DCP0010Active(ctx, blockHeight, params, n)`; then `dcp0012Active` likewise;
   errors → `E(op, err)`. (`DCP0010Active`: simnet → true; networks other than mainnet/testnet3 → false;
   otherwise query `Deployments`, look up `changesubsidysplit`; true when its status is `active`, or
   `lockedin` with `height == Since + RuleChangeActivationInterval - 1`. `DCP0012Active`: same for
   `changesubsidysplitr2` on mainnet, testnet3 and simnet with no simnet shortcut; other networks false.
   `deployments/deployments.go:88-157`.)
4. `defaultVoteBits = w.VoteBits()`.
5. **One DB view:**
   1. `ticketHashes = selectOwnedTickets(winners)`; if empty return from the view.
   2. Allocate `votes` and `usedVoteBits` with `len(ticketHashes)` elements.
   3. For each ticket `i`:
      1. `ticketPurchase = txStore.Tx(ticketHash)`; error → log at error
         `Failed to read ticket purchase transaction for owned winning ticket <hash>: <err>`, continue.
      2. `(owned, haveKey) = hasVotingAuthority(addrmgrNs, ticketPurchase)`; error → abort the view with it.
         Not both true → continue.
      3. `ticketVoteBits = defaultVoteBits`, replaced by per-ticket vote bits when
         `readDBTicketVoteBits` reports saved preferences.
      4. `dp = w.DisapprovePercent()`. If `dp > 0`: on mainnet log at warn
         `Block disapprove percent set on mainnet`; otherwise if `int64(dp) > rand.Int64N(100)` (CSPRNG) log
         at info `Disapproving block <blockHash> voted with ticket <ticketHash>` and clear bit 0 of
         `ticketVoteBits.Bits`.
      5. `tspends = w.GetAllTSpends(ctx)` (drops cached tspends with `Expiry < tip height`; map order).
      6. `vote = createUnsignedVote(ticketHash, ticketPurchase, blockHeight, blockHash, ticketVoteBits, subsidyCache, params, dcp0010Active, dcp0012Active)`;
         error → log at error `Failed to create vote transaction for ticket hash <hash>: <err>`, continue.
      7. Build treasury votes: for each cached tspend with
         `standalone.InsideTSpendWindow(int64(blockHeight), Expiry, TreasuryVoteInterval, TreasuryVoteIntervalMultiplier)`
         and `w.TSpendPolicy(&tspendHash, ticketHash) != TreasuryVoteInvalid (0)`, append
         `hash || byte(policy)`. If anything was appended, build the script of §3.7; on builder error log at
         error `Failed to create treasury vote for ticket hash <hash>: <err>`; otherwise append the output
         and set `vote.Version = 3`.
      8. `signVote(addrmgrNs, ticketPurchase, vote)`; error → log at error
         `Failed to sign vote for ticket hash <hash>: <err>`, continue.
      9. `votes[i] = vote`; `usedVoteBits[i] = ticketVoteBits`;
         `watchOutPoints = appendRelevantOutpoints(watchOutPoints, dbtx, vote)`.
   4. A view error is logged at error `View failed: <E(op, err)>` and execution continues.
6. Remove nil entries from `votes` by swapping with the last element and truncating (order not preserved;
   `ticketHashes` and `usedVoteBits` are **not** permuted alongside).
7. Build `voteRecords` with `NewTxRecordFromMsgTx(vote, time.Now())`; failures logged at error
   `Failed to create transaction record: <err>` and skipped.
8. Under `recentlyPublishedMu`: add each record hash to `w.recentlyPublished` and log at info
   `Voting on block <blockHash> (height <blockHeight>) using ticket <ticketHashes[i]> (vote hash: <hash> bits: <usedVoteBits[i].Bits>)`.
9. `n.PublishTransactions(ctx, votes...)` — before the database write; error logged at error
   `Failed to send one or more votes: <err>`. Called even when `votes` is empty.
10. If `watchOutPoints` non-empty: `n.LoadTxFilter(ctx, false, nil, watchOutPoints)`; error logged
    (`Failed to watch outpoints: <err>`).
11. **One DB update**, without `lockedOutpointMu`: `processTransactionRecord(ctx, dbtx, rec, nil, nil)` for each
    record. Error returned unwrapped.
12. If a network backend is set: `watchHDAddrs(ctx, false, n)`; error returned unwrapped.
13. Return nil.

#### 4.12.1 Helpers used by voting

`hasVotingAuthority(addrmgrNs, ticketPurchase)` (`wallet/wallet.go:3985-4005`):

1. `(_, addrs) = stdscript.ExtractAddrs(out0.Version, out0.PkScript, params)` for output 0 of the ticket
   (index 0 is dereferenced unconditionally).
2. For each address in order: skip it unless it implements `stdaddr.Hash160er`. If
   `manager.ExistsHash160(addrmgrNs, hash160)` then return
   `(true, manager.HavePrivateKey(addrmgrNs, addr))` — the first matching address decides; a
   `HavePrivateKey` error is returned with `mine = true`.
3. No match → `(false, false, nil)`.

`appendRelevantOutpoints(relevant, dbtx, tx)` (`wallet/wallet.go:5004-5060`). A nil `relevant` is replaced
by an empty slice with capacity `len(tx.TxOut)` (so the result is never nil). `isTicket =
stake.DetermineTxType(tx) == SStx`. For each output `i`:

1. If `isTicket`, `i` is odd and output 0 has not already been appended: decode the commitment address
   (`stake.AddrFromSStxPkScrCommitment`; on error go to the next output). If it is a `Hash160er` whose
   hash160 the manager knows (`ExistsHash160`): append `{txHash, Index: 0, Tree: 1}`, remember that output
   0 is watched, and go to the next output. Otherwise fall through to step 2 for this output.
2. `(class, addrs) = ExtractAddrs(out.Version, out.PkScript, params)`; `tree = 1` when
   `txrules.StakeSubScriptType(class)` reports a stake type, else 0.
3. For the first address that is a `Hash160er` known to the manager: append `{txHash, i, tree}`; when
   `isTicket && i == 0` remember that output 0 is watched; stop examining this output's addresses.

Differences from the outpoints returned by `processTransactionRecord` (§4.10 step 4): ownership is tested
with `ExistsHash160` rather than `manager.Address`; zero-value outputs are not skipped; an output is
appended at most once; and for a ticket an owned commitment address causes output 0 to be watched. For a
vote (the only caller in scope) the result is the owned stake-generation outputs (index ≥ 2, tree 1).
These outpoints are loaded into the backend filter in step 10, before the vote is recorded; the
outpoints returned by `processTransactionRecord` in step 11 are discarded.

**Revocations.** `RevokeOwnedTickets` does nothing. Since DCP0009 revocations are created by block
producers; the wallet only observes them as mined SSRtx transactions through §4.10.

### 4.13 Rescan — `rescan` and wrappers

`wallet/rescan.go:212-443`.

**Wrappers**

| Function | Start resolution (one DB view) | Errors |
| --- | --- | --- |
| `Rescan(ctx, n, startHash)` | read the serialized header of `startHash` (`GetSerializedBlockHeader`; any stored header, main chain or not) and extract its height with `udb.ExtractBlockHeaderHeight` (little-endian uint32 at byte offset 128 of the 180-byte header, converted to int32; `wallet/udb/txmined.go:568-579`). A stored side-chain hash passes this step and then fails in the first batch with `Invalid` `block <hash> not in main chain` | wrapped with op `wallet.Rescan` |
| `RescanFromHeight(ctx, n, startHeight)` | `GetMainChainBlockHashForHeight(startHeight)`; kind `NotExist`, `no block at height <h> in main chain` when beyond the tip | wrapped with op `wallet.RescanFromHeight` |
| `RescanProgressFromHeight(ctx, n, startHeight, p)` | as above | sent as `RescanProgress{Err: E(op, err)}` on `p`; `p` is closed on return in every case (`defer close(p)`) |

**Core loop `rescan(ctx, n, startHash, height, p)`**

1. Under `logRescannedTransactionsMu`: `logTxs = w.logRescannedTransactions`; set the field to true. (The
   first rescan of a process therefore never logs discovered transactions; all later ones do.)
2. `rescanFrom = *startHash`; `inclusive = true`; allocate storage for 2000 hashes.
3. Loop:
   1. If `ctx` is done return `ctx.Err()`.
   2. **DB view:** `rescanBlocks = txStore.GetMainChainBlockHashes(&rescanFrom, inclusive, storage)` — up to
      2000 consecutive main-chain hashes starting at (or after) `rescanFrom`. Errors: kind `NotExist`
      `starting block <hash> not found`; kind `Invalid` `block <hash> not in main chain`. Returned unwrapped.
   3. If empty, break.
   4. `through = height + len(rescanBlocks) - 1`.
   5. If `height == 0`: drop the first hash (genesis is never rescanned), `height = 1`; if nothing remains,
      break.
   6. Log at info `Rescanning block range [<height>, <through>]...`.
   7. Start the **batch saver** goroutine reading a channel (buffer 1) of
      `{blockHash, txs, errc}`. For each item it appends to a pending batch and adds `len(txs)` to a counter;
      when the counter reaches **256 or more** it saves the batch (below), resets, and replies on `errc` with
      the save result (on a save error it replies with the error and exits without the final save).
      Otherwise it replies nil. When the channel is closed it saves whatever is pending and sends the
      result on `lastBatchErr` (unbuffered).
   8. `err = n.Rescan(ctx, rescanBlocks, save)` where `save(blockHash, txs)` sends an item to the saver and
      waits for its reply.
   9. Close the saver channel. If `n.Rescan` returned an error, return it (the final-batch result is not
      read; see Q-10).
   10. Read `lastBatchErr`; return it if non-nil.
   11. **DB update:** `UpdateProcessedTxsBlockMarker(dbtx, &rescanBlocks[last])`. Error returned.
   12. If `p != nil`: send `RescanProgress{ScannedThrough: through}` (blocking send, not cancellable).
   13. `rescanFrom = rescanBlocks[last]`; `height += len(rescanBlocks)`; `inclusive = false`.
4. Log at info `Rescan complete`; return nil.

**Saving a batch** (`wallet/rescan.go:257-281` and `saveRescanned`, `:171-206`): requires
`len(blocks) == len(txs)` (else kind `Bug`, `len(blocks) must match len(txs)`); empty batch is a no-op.
Lock `lockedOutpointMu`, then **one DB update** for the whole batch. For each block: load its block meta
and header (errors wrapped with op `wallet.saveRescanned`); for each transaction: when `logTxs`, call
`logRescannedTx`; `rec = NewTxRecordFromMsgTx(tx, time.Now())`;
`processTransactionRecord(ctx, dbtx, rec, header, &blockMeta)`. The returned outpoints are discarded: the
rescanning backend is expected to add new outpoints to its own filter while it scans
(`spv/rescan.go:53-71`; dcrd's `rescan` does the same server-side).

`logRescannedTx(txmgrNs, height, tx)`: `(haveMined, haveUnmined) = txStore.ExistsTxMinedOrUnmined(hash)`. If
unmined: info `Rescan of block <height> discovered previously unmined transaction <hash>`. Else if not mined:
info `Rescan of block <height> discovered new transaction <hash>`.

**Progress semantics.** One `RescanProgress` per batch of at most 2000 blocks, carrying the height of the
last block of the batch. Nothing is sent for the start height itself, and nothing at all is sent (and the
marker is not touched) when the only block in range is the genesis block.

### 4.14 Rescan point — `rescanPoint`

`wallet/rescan.go:445-498`. One DB view (inside `RescanPoint`) or the caller's transaction.

1. `r = txStore.ProcessedTxsBlockMarker(dbtx)` (the `lasttxsblock` hash; zero hash when absent).
2. `r = mainChainAncestor(dbtx, r)`: while `r` is not a main-chain block, `r = header(r).PrevBlock`; a
   missing header returns its error (kind `NotExist`).
3. If `r` equals the main-chain tip hash return `(nil, nil)`: transactions are processed through the tip.
4. Load `r`'s header; return the main-chain hash at `header.Height + 1`. Either lookup failing is logged at
   info (the bare error) and returned.

`RescanPoint` wraps errors with op `wallet.RescanPoint`.

Meaning: the result is the first block whose transactions have not been processed. Syncers treat non-nil
as "discovery and rescan required before the wallet is synced" and skip scanning newly announced blocks
for transactions until it becomes nil (`spv/sync.go:1522-1540`). The SPV syncer also ignores every
transaction inventory announcement while it is non-nil (`spv/sync.go:1094-1104`), so no unmined
transaction reaches `AddTransaction` from peers during discovery or a rescan. `watchHDAddrs(firstWatch=false)` does
nothing while it is non-nil.

### 4.15 Birthday state

* `SetBirthState(ctx, bs)`: one DB update writing §3.2 verbatim.
* `BirthState(ctx)`: one DB view; nil when unset.
* Resolution during sync: §4.8 step 8.3.
* `SetBirthStateAndScan(ctx, bs)` (`wallet/rescan.go:532-581`), op `wallet.SetBirthStateAndScan`:
  1. Neither flag set → `E(op, Invalid, "nothing to set")`.
  2. Read the main-chain tip hash.
  3. If `SetFromHeight` (takes precedence when both are set): look up the main-chain hash at
     `int32(bs.Height)` (error → `E(op, err)`), set `bs.Hash`, clear both flags, `SetBirthState`, log at info
     `Set wallet birthday to block <bs.Height> (<hash>).`, return nil. `bs.Time` is stored as passed.
  4. Otherwise (`SetFromTime`): load the tip header (error returned **unwrapped**); walk back one header per
     DB view. At the first header with `Height == 0` or `Timestamp.Before(bs.Time)`: set `bs.Hash` to its
     hash and `bs.Height` to its height, clear both flags, `SetBirthState`, log the same info line, return
     nil. A header lookup failure during the walk → `E(op, err)`.
  The caller's `bs` is mutated. Despite the name, no rescan is started and the processed-transactions
  marker is not touched.

### 4.16 Account and address discovery

#### 4.16.1 `DiscoverActiveAddresses(ctx, n, startBlock, discoverAccts, gapLimit)`

`wallet/discovery.go:716-979`. All errors are wrapped with op `wallet.DiscoverActiveAddresses` unless
stated. `startBlock` must be non-nil for the cfilter path (only account discovery substitutes the genesis
hash for nil).

1. **Coin type.** `(_, slip0044) = udb.CoinTypes(params)` — the legacy value returned by `CoinTypes` is
   discarded; the `<legacy>` printed in step 9 is the wallet's *active* coin type read here
   (`wallet/discovery.go:718,966-967`). In one DB view read
   `manager.CoinType`: kind `WatchingOnly` → `coinTypeKnown = false`; other error → fail; otherwise
   `coinTypeKnown = true`, `isSLIP0044 = (active == slip0044)`, debug log
   `DiscoverActiveAddresses: activeCoinType=<n>`.
2. Create an empty block commitment cache for account discovery.
3. **Account discovery** (only when `discoverAccts`):
   1. Info log `Discovering used accounts`.
   2. One DB view: `manager.CoinTypePrivKey` (kinds `Locked` / `WatchingOnly` when unavailable). The key is
      zeroed when the function returns.
   3. If `n` implements `usedAddressesQuerier`: `lastUsed = existsAddrIndexFinder{w, n, gapLimit}.findLastUsedAccount(ctx, key)`
      (§4.16.4); else `lastUsed = w.findLastUsedAccount(ctx, n, cache, key, gapLimit, startBlock)` (§4.16.2).
   4. If `lastUsed != 0`: **one DB update**: `lastRecorded = manager.LastAccount`; for
      `acct = lastRecorded+1 .. lastUsed` call `manager.NewAccount(ns, "account-<acct>")` (decimal account
      number) and read its xpub. Then under `addressBuffersMu`, for each new account without an entry in
      `w.addressBuffers`, derive both branch keys and store a buffer with `lastUsed = 0`, `cursor = 0`,
      `lastWatched = 0` (zero values, not the "none used" sentinel; Q-12). When
      `lastUsed <= lastRecorded` the update still runs but creates nothing. `NewAccount` failures abort
      discovery (all wrapped with the op): kind `WatchingOnly`; kind `Locked` (the address manager must be
      unlocked — account discovery needs it twice, for the coin type key and here); kind `Exist`
      `account named "account-<n>" already exists` when a user account already carries that name
      (`wallet/udb/addressmanager.go:2084-2105`; Q-51). Because all accounts are created in one update,
      a failure creates none of them. A `deriveBranches` failure for a new account returns `E(op, err)`
      **after** the accounts were committed, leaving that account and the later ones without address
      buffers for the rest of the process (`wallet/discovery.go:806-810`).
4. **Address discovery.** `finder = newAddrFinder(ctx, w, gapLimit)`: one DB view loading, for every account
   `0..LastAccount` and every imported-xpub account `2^31 .. LastImportedAccount`, the two branch xpubs and
   the stored last-used indexes; per branch `lo = lastUsed / gapLimit` (0 when none used),
   `hi = segments - 1` with `segments = 2^31 / gapLimit` (integer division; `gapLimit == 0` panics).
   Info log `Discovering used addresses for <N> account(s)`. Snapshot the usage array (`lastUsed` before
   discovery). If `n` implements `usedAddressesQuerier` run `existsAddrIndexFinder.find` (§4.16.6) else
   `finder.find(ctx, startBlock, n)` (§4.16.5).
5. For each account log at info
   `Account <acct> next child indexes: external:<extLastUsed+1> internal:<intLastUsed+1>` (uint32; "none
   used" prints 0).
6. Info log `Updating DB with discovered addresses...`. For each account `u` with pre-discovery snapshot
   `s`, for the external branch then the internal branch:
   1. `max = lastUsed(u) + gapLimit` (uint32, wrapping: "none used" gives `gapLimit - 1`).
   2. `j = lastUsed(s)`; loop: if `ctx` is done return `ctx.Err()` (**unwrapped**);
      `to = min(j + 256, max)` (uint32 wrapping add); **one DB update**:
      `manager.SyncAccountToAddrIndex(ns, acct, to, branch)` (creates address rows from `to` downward until
      an existing row is met; kind `Invalid` `child index <n> exceeds max` when `to > 2^31 - 1`); if
      `to == max` stop; `j += 256`.
   3. After both branches: lock `addressBuffersMu`, **one DB update**:
      * if `extLastUsed < 2^31`: `manager.MarkUsedChildIndex(dbtx, acct, 0, extLastUsed)`; same for internal
        with branch 1 (only ever raises the stored last-used and, if lower, last-returned index);
      * read the account properties `props`; with `buf = w.addressBuffers[acct]`, for each branch:
        if `props.LastUsed + 1 > buf.lastUsed + 1` (uint32): `buf.cursor += buf.lastUsed - props.LastUsed`
        (wrapping); if `buf.cursor > 0x7FFFFFFF` set it to 0; `buf.lastUsed = props.LastUsed`.
      Unlock. `w.addressBuffers[acct]` is dereferenced without a presence check: every account
      enumerated by `newAddrFinder` must already have a buffer (created at open,
      `wallet/wallet.go:5456-5500`, on account creation, or in step 3.4), otherwise the Go panics. The
      buffer is adjusted even when the update's `MarkUsedChildIndex` calls were skipped; if the update
      closure fails after mutating a buffer the mutation is not undone.
   4. `SyncAccountToAddrIndex` and the last-used update run for **every** account in `finder.usage`,
      including imported-xpub accounts and accounts on which nothing was discovered (for those, with a
      gap limit of at most 256, the first loop iteration computes `to = max` and performs one update).
7. If `!coinTypeKnown || isSLIP0044`: info log `Finished address discovery`; return nil.
8. If `w.disableCoinTypeUpgrades`, or more than one account exists, or account 0 has any used address on
   either branch: info log `Finished address discovery`, warn log
   `Wallet contains addresses derived for the legacy BIP0044 coin type and seed restores may not work with some other wallet software`;
   return nil.
9. **Coin type upgrade.** Info log
   `Upgrading wallet from legacy coin type <legacy> to SLIP0044 coin type <slip0044>`;
   `w.UpgradeToSLIP0044CoinType(ctx)`. On error: error log `Coin type upgrade failed: <err>`, warn log
   `Continuing with legacy BIP0044 coin type -- seed restores may not work with some other wallet software`,
   return nil. On success: info log `Upgraded coin type.` and **recurse** with the same arguments (the
   recursion's error is returned as is).

#### 4.16.2 Account discovery over cfilters — `(w) findLastUsedAccount`

`wallet/discovery.go:418-535`. `g = uint32(w.accountGapLimit)` (configured account gap limit).

`lastUsedInRange(begin, end)` (half-open account range):

1. If `end >= 2^31` set `end = 2^31 - 1`.
2. For each account in range: derive the hardened account key `coinType.Child(2^31 + acct)`, neuter, derive
   both branch keys, derive children `0..gapLimit-1` of each branch (`deriveChildAddresses`, which silently
   skips invalid children), and record each payment script → account. Private keys are zeroed.
3. `searchBlocks = w.filterBlocks(ctx, startBlock or genesis, scripts)` (§4.16.7).
4. `cacheMissingCommitments(ctx, n, cache, searchBlocks)` (§4.16.3).
5. Result = the highest account number having any script present in the commitments of any matching
   block; 0 when none. Each hit is logged at debug
   `Found match for script <hex> account <acct> in block <hash>`.

Every derivation error in step 2 aborts the whole account discovery, including
`hdkeychain.ErrInvalidChild` for a hardened account index (unlike address derivation, an invalid
*account* is not skipped); the exists-index variant (§4.16.4) behaves the same. The return value 0 means
both "only account 0 is used" and "nothing is used"; `DiscoverActiveAddresses` treats them alike
(no account is created). Used accounts at or below the last recorded account are also a no-op, so
account discovery never lowers or renames anything.

Search:

1. `lastUsed = lastUsedInRange(0, 2g)`. If `lastUsed < g` return it.
2. `lo = 2`, `hi = 2^31 / g`. While `lo <= hi`: `mid = (hi + lo) / 2`;
   `last = lastUsedInRange(mid * g, mid * g + g)`; if `last > lastUsed` then `lastUsed = last`; if `mid == 0`
   break; `hi = mid - 1`.
3. Return `lastUsed`.

`lo` is never raised, so this is a fixed halving probe sequence, not a binary search (Q-11).

#### 4.16.3 Block commitments

`blockCommitments(block)` (`wallet/discovery.go:37-67`) returns the set of:

| Transactions | Scripts added |
| --- | --- |
| every regular-tree transaction | `PkScript` of every output, unmodified |
| stake tree, ticket (SStx) | for even output indexes `i >= 2` with `Value != 0`: `PkScript[1:]` |
| stake tree, vote (SSGen) | for every output index `>= 2`: `PkScript[1:]` |
| stake tree, revocation (SSRtx) | for every output: `PkScript[1:]` |
| other stake-tree types (treasury) | nothing |

Not included: ticket submission outputs (voting address), ticket commitment outputs (reward address),
and previous-output scripts of inputs. An address therefore counts as "used" for discovery only when an
output of one of the listed kinds pays to it.

`cacheMissingCommitments(ctx, n, cache, include)` (`wallet/discovery.go:69-94`): process `include` in chunks
of 500; in each chunk collect hashes absent from the cache; if a chunk has none **return nil immediately**
(remaining chunks are not examined; Q-13); otherwise `n.Blocks(ctx, missing)` and cache each block's
commitments under the requested hash.

#### 4.16.4 Account discovery over the exists-address index

`existsAddrIndexFinder.findLastUsedAccount` (`wallet/discovery.go:545-597`). `scanLen = uint32(w.accountGapLimit)`.

1. `lo = 0`, `hi = 2^31 / scanLen`, `lastUsed = 0`.
2. While `lo <= hi`: `mid = (hi + lo) / 2`. For `i = scanLen-1 .. 0`, `account = mid * scanLen + i`; accounts
   `>= 2^31` are skipped; derive the hardened account key (derivation error aborts), neuter, and start a
   goroutine computing `accountUsed(xpub)`. Wait for all.
3. Scan results from `i = scanLen-1` down: an error aborts with it; the first used account sets
   `lastUsed = account`, `lo = mid + 1`, and restarts step 2.
4. No used account: if `mid == 0` stop; `hi = mid - 1`.
5. Return `lastUsed`.

`accountUsed(xpub)`: derive both branch keys; query `branchUsed` for each concurrently; return on the first
error or the first `true`; false when both are false. `branchUsed(branchXpub)`: derive children
`0 .. w.gapLimit - 1` — the **wallet's configured** gap limit, not the `gapLimit` argument (Q-15) — call
`UsedAddresses`, and report whether any **byte** of the returned bitset is non-zero.

#### 4.16.5 Address discovery over cfilters — `(*addrFinder).find`

`wallet/discovery.go:185-359`.

1. **One DB view:** load the header of `start`, allocate `tipHeight - start.Height + 1` slots, and load the
   stored cfilters (with keys and block hashes) of every main-chain block from `start` inclusive
   (`GetMainChainCFilters`).
2. Loop:
   1. Return `ctx.Err()` if cancelled.
   2. **Build the probe set.** For each account, for the external then the internal branch with bounds
      `(lo, hi)`: if `lo > hi` or `hi >= segments` the branch is finished and contributes nothing; else
      `mid = (hi + lo) / 2`, derive children `mid*gapLimit .. mid*gapLimit + gapLimit - 1`, and for each
      derived address record `script → {usage index, account, branch, child index}` and append the script
      to `data`. (Invalid children are skipped by `deriveChildAddresses`, shifting the computed child index
      of later addresses in that segment; Q-16.)
   3. If `data` is empty return nil (every branch finished).
   4. `a.filter(ctx, fs, data, n)`: split `fs` into chunks of 500 and process chunks concurrently
      (`errgroup`). In each chunk, skip filters with `N() == 0` and blocks already in the finder's
      commitment cache; collect hashes whose filter `MatchAny(key, data)`; fetch them with one
      `n.Blocks` call; compute and cache each block's commitments (one goroutine per block). The first
      error cancels the group and is returned.
   5. **Check every cached block** (not only newly fetched ones) against every script in `data`,
      concurrently per block. For each exact hit, debug log
      `Found match for script <hex> path <{usageIndex account branch index}> in block <hash>` and, under the
      finder mutex, raise the branch's last-used index when it is the sentinel `0xFFFFFFFF` or lower than
      the hit's child index.
   6. **Advance the bisect.** For each account and each branch with `lo <= hi`: `mid = (hi + lo) / 2`,
      `begin = mid * gapLimit`, `end = begin + gapLimit`. If `begin <= lastUsed < end` then `lo = mid + 1`
      else `hi = mid - 1` (uint32; wraps to `0xFFFFFFFF` when `mid == 0`, which step 2.2 treats as finished).
      This step tests only `lo <= hi`, not `hi < segments` (`wallet/discovery.go:290,302`), so a branch
      that finished by wrapping (`lo = 0`, `hi = 0xFFFFFFFF`) is still advanced on every later iteration
      that other branches keep alive: `hi` becomes `0x7FFFFFFE`, `0x3FFFFFFE`, … (the `[begin, end)` test
      uses wrapping `mid * gapLimit` and `begin + gapLimit`). Once `hi` falls below `segments` step 2.2
      probes that branch again. With `gapLimit = 20` (`segments = 107374182`) that needs six further
      iterations after the wrap; with small gap limits (1 or 2) it needs one or two.

Step 1 details (`wallet/discovery.go:187-200`, `wallet/udb/txmined.go:505-543`): `start` is used only for
its stored header's **height** — `GetMainChainCFilters` does not check that `start` is itself a main-chain
block and returns the main-chain filters from that height upward, stopping at the first missing block
record or when the storage slice is full. A `start` with no stored header fails with kind `NotExist`
(the header lookup's error; `GetMainChainCFilters` itself would report `starting block <hash> not found`).
A `start` whose height exceeds `tipHeight + 1` makes the slice length negative and the Go panics
(`makeslice: len out of range`); a height of exactly `tipHeight + 1` yields no filters, so step 2.5 has
nothing to check and every branch walks to "finished" with no usage added to the stored values. A main-chain
block without a stored cfilter row aborts with the `udb` lookup error. The filter list is loaded once
and not refreshed; blocks attached while discovery runs are not searched. `filterBlocks` (§4.16.7) has
the same height-only treatment of its start hash.

The per-branch state is `accountUsage{ account; extkey, intkey; extLastUsed, intLastUsed; extlo, intlo;
exthi, inthi }` and hits are keyed through `scriptPath{ usageIndex int; account, branch, index uint32 }`
(`wallet/discovery.go:96-108`). `scrPaths` is a map keyed by script bytes: if two probed branches derive
the same script (not possible for distinct keys) the later insertion wins. The initial capacity of the
usage slice is `lastAcct + 1 + lastImported - ImportedAddrAccount`.

The search assumes usage is contiguous at gap-limit granularity: a segment with no used address ends the
upward search even if higher segments are used. The starting `lo` (the segment containing the stored
last-used index) lets a previously synced wallet resume without re-probing lower segments; note the first
probe is still the midpoint of `[lo, segments-1]`, i.e. roughly child index `2^30`.

#### 4.16.6 Address discovery over the exists-address index

`existsAddrIndexFinder.find` (`wallet/discovery.go:679-705`): for every account start two tasks (external,
internal) in an unbounded `errgroup`; each reads the branch xpub in its own DB view and runs
`findLastUsedAddress`, then **overwrites** the finder's last-used index for that branch with the result
(including overwriting a stored index with `0xFFFFFFFF`).

`findLastUsedAddress(xpub)` (`wallet/discovery.go:646-677`): `scanLen = f.gapLimit`,
`segments = 2^31 / scanLen`, `lo = 0`, `hi = segments - 1`, `lastUsed = 0xFFFFFFFF`. While `lo <= hi`:
`mid = (hi + lo) / 2`; derive children `mid*scanLen ..` (count `scanLen`); `bits = UsedAddresses(addrs)`; scan
`i = len(addrs)-1 .. 0`: the first set bit sets `lastUsed = mid*scanLen + i`, `lo = mid + 1`, and continues
the outer loop. No bit set: stop if `mid == 0`, else `hi = mid - 1`.

#### 4.16.7 `filterBlocks`

`wallet/discovery.go:363-416`. Start `runtime.NumCPU()` workers reading batches from a channel of the same
capacity; each worker appends (under a mutex) the hash of every block whose filter has `N() != 0` and
`MatchAny(key, data)`. The producer reads main-chain cfilters in batches of 2000 (one DB view each, first
batch inclusive of `startBlock`, later batches exclusive of the previous batch's last block) until an
empty batch. On cancellation it closes the channel and returns `(nil, ctx.Err())` without waiting for
workers. Otherwise it closes the channel, waits, and returns `(matches, ctx.Err())`. The order of
`matches` depends on scheduling.

### 4.17 Watching future addresses

#### 4.17.1 `watchHDAddrs(ctx, firstWatch, n)`

`wallet/wallet.go:891-1065`.

1. If `!firstWatch`: `rp = RescanPoint(ctx)`; error returned; `rp != nil` → return `(0, nil)` (do not watch
   while transaction history is behind the tip).
2. **One DB view:** for every account `0..LastAccount` and `2^31..LastImportedAccount` read the properties
   and compute per branch
   `count = min(LastReturnedIndex + w.gapLimit, 2^31 - 1)` (uint32 wrapping: nothing returned yet gives
   `gapLimit - 1`), plus last-returned and last-used indexes.
3. Under `addressBuffersMu`, for every account buffer: per branch, if
   `dbLastUsed + 1 > buf.lastUsed + 1`: `buf.cursor += buf.lastUsed - dbLastUsed`; clamp to 0 when above
   `0x7FFFFFFF`; `buf.lastUsed = dbLastUsed`. Take the branch xpubs from the buffer. If `firstWatch` set
   `buf.lastWatched = count`; otherwise remember `start = buf.lastWatched`.
4. For each account (Go map order), external then internal branch, `loadBranchAddrs(key, start, end = count)`:
   1. If `start == 0 && w.watchLast != 0 && end - w.gapLimit > w.watchLast`:
      `start = end - w.gapLimit - w.watchLast`.
   2. In steps of 256 from `start` while `start <= end`: derive children `[start, min(end + 1, start + 256))`
      in a task (at most `runtime.NumCPU()` concurrent), skipping `ErrInvalidChild`, and call
      `n.LoadTxFilter(ctx, false, addrs, nil)`; count addresses. The first error is returned.
5. Under `addressBuffersMu`: raise each `buf.lastWatched` to `count` if lower.
6. Return the number of addresses loaded.

Step 3 iterates `w.addressBuffers` (not the accounts read in step 2) and step 5 indexes
`w.addressBuffers[acct]` for every account read in step 2 without a presence check: an account in the
database without a buffer panics there (and earlier, on its nil branch key in step 4); a buffer without
a database account is loaded with the zero-value counts (`end = 0`, child 0 only). The `watchLast`
adjustment in step 4.1 applies to any call whose `start` is 0, which includes incremental watches of
buffers whose `lastWatched` is still 0 (accounts created or discovered after the first watch). The
subtraction `end - w.gapLimit` is unsigned (Q-52).

Ranges are inclusive on both ends: a first watch covers children `0 .. lastReturned + gapLimit`; an
incremental watch covers `lastWatched .. lastReturned + gapLimit` (re-sending `lastWatched` itself).
`--watchlast=N` limits a first watch to the last `N` returned addresses plus the gap-limit lookahead.

#### 4.17.2 When watching is refreshed

| Trigger | Call |
| --- | --- |
| end of every successful `ChainSwitch` with a backend set | `watchHDAddrs(false)`, then new credit outpoints |
| `AddTransaction` that produced outpoints | `watchHDAddrs(false)`, then outpoints |
| after votes are recorded | `watchHDAddrs(false)` |
| a new multisig redeem script imported in §4.10 | its P2SH address, immediately |
| syncer start / after discovery | `LoadActiveDataFilters(reload = true)` |

Address rows beyond the last used index are created by `markUsedAddress` (§4.10 step 6) so that
`manager.Address` recognises payments to lookahead addresses; the network filter is extended relative to
the last *returned* index.

#### 4.17.3 `LoadActiveDataFilters(ctx, n, reload)`

`wallet/wallet.go:1101-1201`. Op `wallet.LoadActiveDataFilters` (applied only to the final view's error).

1. Info log `Loading active addresses and unspent outputs...`.
2. If `reload`: `n.LoadTxFilter(ctx, true, nil, nil)`.
3. `watchHDAddrs(ctx, true, n)`; info log
   `Registered for transaction notifications for <count> HD address(es)`.
4. One DB view over every address of the imported account, sent in batches of 256
   (`LoadTxFilter(false, addrs, nil)`); if any, info log
   `Registered for transaction notifications for <count> imported address(es)`.
5. Under `lockedOutpointMu`, one DB view: every unspent outpoint (`ForEachUnspentOutpoint`, undefined
   order) and, for every unspent ticket including immature ones, `{ticketHash, 0, TxTreeStake}`; sent in
   batches of 64 with a final partial batch flushed by a deferred call when no error occurred.
6. Info log `Registered for transaction notifications for all relevant outputs`.

### 4.18 `RescanFilter`

`wallet/rescan.go:27-149`. Five maps:

| Map | Key | Populated by address type |
| --- | --- | --- |
| `pubKeyHashes` | 20-byte hash160 | `*stdaddr.AddressPubKeyHashEcdsaSecp256k1V0` |
| `scriptHashes` | 20-byte hash160 | `*stdaddr.AddressScriptHashV0` |
| `compressedPubKeys` | 33-byte serialized key | `*stdaddr.AddressPubKeyEcdsaSecp256k1V0` whose `SerializedPubKey()` is 33 bytes |
| `otherAddresses` | `Address.String()` | any other type, and secp256k1 pubkey addresses with a non-33-byte serialization |
| `unspent` | `wire.OutPoint` (hash, index, tree) | `AddUnspentOutPoint` |

`ExistsAddress` for a 33-byte secp256k1 pubkey address checks `compressedPubKeys` first and, on a miss,
`pubKeyHashes` with the hash160 of the key (`AddressPubKeyHash()`). `RemoveAddress` has no such fallback.

How the SPV backend uses it (owned by the SPV spec; summarised because it defines relevance):
`rescanCheckTransactions` (`spv/rescan.go:20-77`) marks a transaction relevant when any input's previous
outpoint is in `unspent` (all inputs of the first regular transaction — the coinbase — and the first input
of a vote are not examined) or any output pays (per `stdscript.ExtractAddrs`) to an address in the filter;
each matching output's outpoint (with the tree of the transaction) is added to `unspent`. Stake
transactions are checked before regular ones.

### 4.19 Checkpoints

* `CheckpointHash(net, height)` is non-nil only for testnet3 at 962928.
* The SPV initial header sync (`spv/backend.go:339-342`) and both RPC syncer paths
  (`chain/sync.go:269-272,879-882`) call `BadCheckpoint(net, hash, height)` on every new node and mark it
  with `(*BlockNode).BadCheckpoint()` before adding it to the forest, which makes it and all descendants
  ineligible in `best()` and `EvaluateBestChain`. The SPV header-announcement path
  (`spv/sync.go:1465`) does **not** make this call; a mismatching block arriving that way is caught only
  by `extendMainChain`.
* `extendMainChain` refuses a mismatching block (§4.9 step 1).
* `rollbackInvalidCheckpoints(dbtx)` (`wallet/checkpoints.go:46-69`), run in its own DB update during
  `wallet.Open` after the stores are opened: on testnet3, if a main-chain block exists at 962928 and its
  hash differs from the checkpoint, `txStore.Rollback(dbtx, 962928)`. A missing block at that height is not
  an error. On other networks it does nothing. The processed-transactions marker is not adjusted; it is
  re-derived through `mainChainAncestor` on the next `rescanPoint`.

### 4.20 `validate` package

`validate/validate.go`. No state, no database access.

* `MerkleRoots(block)`: `standalone.CalcTxTreeMerkleRoot(block.Transactions) != block.Header.MerkleRoot` →
  `E(Opf("validate.MerkleRoots(%v)", &blockHash), Consensus, "invalid regular merkle root")`;
  `CalcTxTreeMerkleRoot(block.STransactions) != block.Header.StakeRoot` → same op,
  `invalid stake merkle root`. Regular is checked first.
* `DCP0005MerkleRoot(block)`:
  `standalone.CalcCombinedTxTreeMerkleRoot(block.Transactions, block.STransactions) != block.Header.MerkleRoot`
  → `E(Opf("validate.DCP0005MerkleRoot(%v)", &blockHash), Consensus, "invalid combined merkle root")`.
* Callers try `MerkleRoots` first and `DCP0005MerkleRoot` only if it fails, returning the second error
  (`spv/backend.go:82-89`). There is no height gate: a block is accepted if it satisfies either rule.
* `CFilterV2HeaderCommitment(net, header, filter, leafIndex, proof)`:
  1. testnet3 and `header.Height < 323328` → nil. Mainnet and `header.Height < 431488` → nil. All other
     networks are always checked.
  2. `standalone.VerifyInclusionProof(&header.StakeRoot, &filter.Hash(), leafIndex, proof)`; false →
     `E(Opf("validate.CFilterV2HeaderCommitment(%v)", &blockHash), Consensus, "invalid header inclusion proof for cfilterv2")`.
     (`VerifyInclusionProof` rejects proofs longer than 32 hashes and leaf indexes above
     `2^len(proof) - 1`; `FilterV2.Hash()` of an empty filter is the zero hash.)
* `PreDCP0005CFilterHash(net, cfsethash)`: testnet3 and mainnet compare against §3.8; any other network →
  `unknown network <net as decimal uint32>`; mismatch →
  `hash of provided cfilters <got> does not match expected hash <want>` (both in display order). These two
  errors are plain formatted errors without op or kind.

---

## 5. Constants, limits and defaults

| Name | Value | Source | What depends on it |
| --- | --- | --- | --- |
| `maxBlocksPerRescan` | 2000 | `wallet/rescan.go:21` | rescan batch size; progress granularity; marker update frequency |
| rescan save threshold | ≥ 256 transactions pending | `wallet/rescan.go:305` | database transaction size during rescans |
| rescan item channel buffer | 1 | `wallet/rescan.go:292` | — |
| `testNet3MaxDiffActivationHeight` | 962928 | `wallet/difficulty.go:30` | EMA min-difficulty rule off / max-difficulty rule on (testnet3) |
| testnet3 checkpoint | height 962928, hash §3.8 | `wallet/checkpoints.go:22-34` | forest validity, `extendMainChain`, open-time rollback |
| `maxTestDiffShift` | 6 (`minTestNetTarget = PowLimit >> 6`) | `wallet/wallet.go:5369-5371` | testnet3 maximum difficulty |
| min blake3 block version | 10 mainnet, 11 elsewhere | `wallet/difficulty.go:436-439` | positional difficulty branch selection |
| first possible blake3 activation height | `StakeValidationHeight + 2 * RuleChangeActivationInterval` (mainnet 20224, testnet3 10848, simnet 784) | `wallet/difficulty.go:433-435` | same |
| `validate.DCP0005ActiveHeightMainNet` | 431488 | `validate/validate.go:25` | cfilter commitment checks; pre-DCP0005 set hash range |
| `validate.DCP0005ActiveHeightTestNet3` | 323328 | `validate/validate.go:21` | same |
| `deployments.DCP0001` heights | mainnet 149248, testnet2 46128, testnet3 0, simnet 0 | `deployments/deployments.go:32-37` | stake difficulty validation and `NextStakeDifficulty` availability |
| `voteVersion` | mainnet 11, testnet2 6, testnet3 12, simnet 12, other 1 | `wallet/wallet.go:291-304` | "old vote version" warning |
| forest prune depth | `params.CoinbaseMaturity` (mainnet 256, testnet3 16, simnet 16) | `wallet/sidechains.go:315-326` | how long stale side chains are retained |
| discovery block fetch chunk | `wire.MaxBlocksPerMsg` = 500 | `wallet/discovery.go:70-74,318-322`, `dcrd/wire@v1.8.0/msgblock.go:24` | `NetworkBackend.Blocks` request size |
| `filterBlocks` cfilter batch | 2000 | `wallet/discovery.go:395` | DB read batch |
| `filterBlocks` workers | `runtime.NumCPU()` | `wallet/discovery.go:367-369` | — |
| discovery DB sync step `N` | 256 child indexes per update | `wallet/discovery.go:854` | cancellation granularity |
| `watchHDAddrs` derivation step | 256 addresses per `LoadTxFilter` call | `wallet/wallet.go:1013` | filter load batch size |
| `watchHDAddrs` concurrency | `runtime.NumCPU()` | `wallet/wallet.go:1012` | — |
| `LoadActiveDataFilters` outpoint batch | 64 | `wallet/wallet.go:1112` | — |
| `LoadActiveDataFilters` imported address batch | 256 | `wallet/wallet.go:1138` | — |
| `DefaultGapLimit` | 20 | `wallet/addresses.go:385` | discovery segment size, lookahead |
| `DefaultAccountGapLimit` | 10 | `wallet/addresses.go:389` | account discovery segment size |
| `hdkeychain.HardenedKeyStart` | `0x80000000` | `dcrd/hdkeychain/v3@v3.1.3/extendedkey.go:36` | segment counts, child index bounds |
| `udb.ImportedAddrAccount` | `2^31 - 1` | `wallet/udb/addressmanager.go:47` | account enumeration boundaries |
| "none used" sentinel | `0xFFFFFFFF` (`^uint32(0)`) | throughout | last-used / last-returned indexes |
| cursor clamp | cursor `> 0x7FFFFFFF` → 0 | `wallet/discovery.go:923-925`, `wallet/wallet.go:976-978` | address buffer adjustment |
| `scriptVersionAssumed` | 0 | `wallet/wallet.go:83` | multisig sig-script detection, redeem script parsing, input validation |
| treasury vote tx version | 3 (`wire.TxVersionTreasury`) | `wallet/chainntfns.go:840` | vote serialization |
| treasury vote prefix | `'T' 'V'` (`0x54 0x56`) | `wallet/chainntfns.go:805` | vote output |
| block-valid vote bit | `0x0001` | `wallet/chainntfns.go:784` | disapproval |
| disapprove draw | `rand.Int64N(100)` | `wallet/chainntfns.go:779` | non-mainnet random disapproval |
| blockcf2 `B`, `M`, key size | 19, 784931, 16 | `dcrd/gcs/v4@v4.1.1/blockcf2/blockcf.go:29-39`, `gcs.go:69` | filter matching |
| account name for discovered accounts | `account-<n>` | `wallet/discovery.go:786` | account names visible over RPC |
| cfilter account discovery linear range | accounts `[0, 2 * accountGapLimit)` | `wallet/discovery.go:508` | first probe; search ends here when the result is `< accountGapLimit` |
| cfilter account discovery fallback bounds | `lo = 2`, `hi = 2^31 / accountGapLimit` | `wallet/discovery.go:517` | probe sequence (Q-11) |
| exists-index account discovery bounds | `lo = 0`, `hi = 2^31 / accountGapLimit` (accounts `>= 2^31` skipped) | `wallet/discovery.go:546-566` | bisect |
| address bisect segment count | `2^31 / gapLimit` (integer division); `lo = lastUsed / gapLimit`, `hi = segments - 1` | `wallet/discovery.go:123,150-166,650-651` | address discovery |
| forced-active ASERT anchor | ancestor at height 1; child of genesis uses `WorkDiffV2Blake3StartBits` | `wallet/difficulty.go:398-406` | simnet difficulty |
| stake difficulty start height | `CoinbaseMaturity + 1`; below it the price is `MinimumStakeDiff` | `wallet/difficulty.go:732-735` | stake difficulty |
| minimum voting height | `StakeValidationHeight - 1` (block height being voted on) | `wallet/chainntfns.go:701` | `VoteOnOwnedTickets` gate |
| treasury vote codes | `0x00` invalid/abstain (omitted), `0x01` yes, `0x02` no | `dcrd/blockchain/stake/v5@v5.0.3/staketx.go:826-836`, `wallet/chainntfns.go:816-823` | treasury vote output |
| treasury vote buffer initial capacity | 256 bytes (not a limit) | `wallet/chainntfns.go:804` | — |
| rescan hash storage | 2000 `chainhash.Hash` values allocated once per rescan and reused by every batch | `wallet/rescan.go:220` | `rescanBlocks` aliases this storage; it is overwritten by the next batch |
| `lockedOutpoints` key | `{hash, index}` (no tree) | `wallet/wallet.go:94-97`, `wallet/chainntfns.go:471-472` | unlock on spend |
| compressed pubkey length | 33 bytes | `wallet/rescan.go:31,73,95,122` | `RescanFilter` fast path |
| `sanityVerifyFlags` | `ScriptDiscourageUpgradableNops \| ScriptVerifyCleanStack \| ScriptVerifyCheckLockTimeVerify \| ScriptVerifyCheckSequenceVerify \| ScriptVerifyTreasury` | `wallet/createtx.go:45-49` | unmined input verification |
| ticket commitment outputs | odd output indexes of an SStx | `wallet/chainntfns.go:554` | commitment recording |
| discovery "change" outputs of a ticket | even indexes `>= 2` | `wallet/discovery.go:47` | block commitments |

Chain parameters consumed by the difficulty code (`dcrd/chaincfg/v3@v3.3.0`):

| Parameter | mainnet | testnet3 | simnet |
| --- | --- | --- | --- |
| `PowLimit` | `2^224 - 1` | `2^232 - 1` | `2^255 - 1` |
| `PowLimitBits` | `0x1d00ffff` | `0x1e00ffff` | `0x207fffff` |
| `ReduceMinDifficulty` | false | true | false |
| `MinDiffReductionTime` | 0 | 10 min | 0 |
| `TargetTimePerBlock` | 5 min | 2 min | 1 s |
| `WorkDiffAlpha` | 1 | 1 | 1 |
| `WorkDiffWindowSize` | 144 | 144 | 8 |
| `WorkDiffWindows` | 20 | 20 | 4 |
| `TargetTimespan` | 720 min | 288 min | 8 s |
| `RetargetAdjustmentFactor` | 4 | 4 | 4 |
| `WorkDiffV2Blake3StartBits` | `0x1b00a5a6` | `0x1e00ffff` | `0x207fffff` |
| `WorkDiffV2HalfLifeSecs` | 43200 | 720 | 6 |
| `BaseSubsidy` | 3119582664 | 2500000000 | 50000000000 |
| `MulSubsidy` / `DivSubsidy` | 100 / 101 | 100 / 101 | 100 / 101 |
| `SubsidyReductionInterval` | 6144 | 2048 | 128 |
| `RuleChangeActivationInterval` | 8064 | 5040 | 320 |
| `MinimumStakeDiff` | 200000000 | 20000000 | 20000 |
| `TicketPoolSize` | 8192 | 1024 | 64 |
| `TicketsPerBlock` | 5 | 5 | 5 |
| `TicketMaturity` | 256 | 16 | 16 |
| `CoinbaseMaturity` | 256 | 16 | 16 |
| `StakeDiffWindowSize` | 144 | 144 | 8 |
| `StakeValidationHeight` | 4096 | 768 | 144 |
| `TreasuryVoteInterval` / multiplier | 288 / 12 | 60 / 4 | 48 / 3 |
| `blake3pow` `ForcedChoiceID` | (none) | (none) | `yes` |
| legacy / SLIP0044 coin type | 20 / 42 | 11 / 1 | 115 / 1 |

---

## 6. Concurrency and lifecycle

### 6.1 Locks and acquisition order

| Lock | Protects | Taken in scope by |
| --- | --- | --- |
| `w.lockedOutpointMu` | `w.lockedOutpoints` | `ChainSwitch`, `AddTransaction`, rescan batch save, `LoadActiveDataFilters` — always **before** opening the DB transaction and held for its whole duration |
| DB write transaction | all buckets | exclusive (bbolt single writer) |
| `w.addressBuffersMu` | `w.addressBuffers` | `DiscoverActiveAddresses` (taken **before** the DB update, `wallet/discovery.go:893-896`), `watchHDAddrs` (taken with no DB transaction open) |
| `w.recentlyPublishedMu` | `w.recentlyPublished` | `AddTransaction`, `ChainSwitch`, `VoteOnOwnedTickets` — never while a DB transaction is open in this scope |
| `w.logRescannedTransactionsMu` | the flag | `rescan` start |
| `w.cachedBlake3WorkDiffCandidateAnchorMu` | the anchor cache | load/store; may be taken inside a DB view |
| `w.networkBackendMu` | backend pointer | `NetworkBackend()`; may be taken inside a DB update (§4.10 step 3.2.3) |
| `w.stakeSettingsLock` | vote bits, tspend caches and policies | `VoteBits` (called once, before the voting DB view is opened); `GetAllTSpends`, `TSpendPolicy` (taken inside the voting DB view) |
| `NotificationServer.mu` | queued notifications, client lists | notify calls inside DB transactions |
| `addrFinder.mu` (RW) | finder commitment cache and usage | `find`, `filter` |
| syncer forest mutex (`sidechainMu` / `sidechainsMu`) | the `SidechainForest` | callers; the forest itself is not thread safe |

Required order where two are held: `lockedOutpointMu` → DB transaction; `addressBuffersMu` → DB transaction.
`VoteOnOwnedTickets` deliberately records votes **without** `lockedOutpointMu`
(`wallet/chainntfns.go:904`); this is safe only because votes skip the `lockedOutpoints` deletion.

### 6.2 Transaction boundaries

| Operation | Transactions |
| --- | --- |
| `ValidateHeaderChainDifficulties` | 1 view |
| `EvaluateBestChain` | 1 view |
| `ChainSwitch` | 1 update (everything from detach to prune); then `watchHDAddrs` (1 view for `RescanPoint`, 1 view for account properties) and `sendAttachedBlockNotification` (own view) |
| `AddTransaction` | 1 update |
| `VoteOnOwnedTickets` | 1 view (build) + `GetAllTSpends`'s nested tip view per ticket + 1 update (record) |
| `rescan` | per 2000-block batch: 1 view (hashes), ≥ 1 update per ≥256 transactions, 1 final update for the remainder, 1 update for the marker |
| `DiscoverActiveAddresses` | several views; 1 update for new accounts; per account branch 1 update per 256 indexes; 1 update per account for last-used marking |
| `SetBirthStateAndScan` (time) | 1 view per header walked |
| `rollbackInvalidCheckpoints` | 1 update at open |

Network calls made **inside** a database transaction: `LoadTxFilter` for a newly imported multisig script
(inside the `ChainSwitch` / `AddTransaction` / rescan / vote update) and, in `LoadActiveDataFilters`,
`LoadTxFilter` for imported addresses inside one view and for full 64-outpoint batches inside the second
view (the latter also under `lockedOutpointMu`).

### 6.3 Goroutines and channels

| Where | Goroutines | Termination |
| --- | --- | --- |
| `rescan` | one batch saver per 2000-block batch | exits after sending the final-batch result; leaks (blocked on the unbuffered `lastBatchErr`) when `n.Rescan` fails (Q-10) |
| `RescanProgressFromHeight` | run by callers in its own goroutine; sends on `p`, closes `p` | sends are not selectable on `ctx`; callers use a buffer of 1 and must drain |
| `filterBlocks` | `NumCPU` workers | on cancellation the function returns before workers exit |
| `addrFinder.filter` | one per 500-filter chunk plus one per fetched block (`errgroup` with derived context) | `g.Wait()` |
| `addrFinder.find` step 2.5 | one per cached block | `WaitGroup` |
| `existsAddrIndexFinder` | `scanLen` per bisect step, each spawning 2; 2 per account for addresses | `WaitGroup` / buffered channel of 2 / `errgroup` |
| `watchHDAddrs` | bounded `errgroup` (`NumCPU`) per branch | `Wait()` |

### 6.4 Ordering guarantees observable by callers and clients

1. Within one `ChainSwitch`, detached-block notifications are queued tip-first, attached blocks in ascending
   height, and each attached block's mined-transaction summaries in `RelevantTxs` order. The combined
   transaction notification is emitted once per `ChainSwitch`, after `notifyMainChainTipChanged`.
2. `MainTipChangedNotification.DetachedBlocks` and the returned `prevChain` are ascending by height.
3. Removed-transaction notifications for pruned unmined transactions are issued inside the update, before
   commit.
4. Votes are published to the network **before** they are written to the database, and their hashes are in
   `recentlyPublished` before publication so that the echo from the backend is ignored.
5. Rescan progress values are strictly increasing; the marker in the database has been advanced through
   `ScannedThrough` before that value is sent.
6. `watchHDAddrs` and outpoint watching happen after the database commit; a block arriving between commit
   and filter load can therefore be scanned with a filter lacking the newest lookahead addresses.

### 6.5 Cancellation

`ctx` is checked at: each rescan batch; each `filterBlocks` batch; each `addrFinder.find` iteration; each
256-index discovery DB step; each `watchHDAddrs` derivation step. `walletdb.View/Update` take `ctx` as
well. Cancellation errors from `rescan`'s loop and from discovery's DB-sync loop are the bare `ctx.Err()`
(the rescan one is then wrapped by the public wrapper's op; the discovery one is not).

### 6.6 Lifecycle

* The forest, the blake3 anchor cache, `recentlyPublished`, `lockedOutpoints`, `logRescannedTransactions`
  and address buffers are process-lifetime, in-memory state; none is persisted.
* `recentlyPublished` entries are removed only when the transaction appears in `RelevantTxs` of an
  attached block.
* Checkpoint rollback runs once per `wallet.Open`.

---

## 7. Errors

Error kinds are those of the wallet `errors` package (`errors/errors.go:52-76`); strings below are the
`Kind.String()` forms that appear in formatted errors. A formatted error is
`<op>: <kind string>: <message>` with nested ops separated by `":\n\t"` by default
(`errors/errors.go:24-28,236-279`).

| Origin | Kind | Message (after op/kind) |
| --- | --- | --- |
| `ChainSwitch` | `Invalid` ("invalid operation") | `zero-length chain` |
| `ChainSwitch` reorg | (none) | `failed reorganize: sidechain ending at block <hash> has less total work than the main chain tip block <hash>` |
| `extendMainChain` | `Consensus` ("consensus violation") | `block hash <hash> does not satisfy checkpoint hash <hash> for height <n>` |
| `validateHeaderChainDifficulties` | `Bug` ("internal wallet error") | `parent of chain[0] is not in main chain` |
| same | `Consensus` | `block <hash> height <n> does not follow parent height <n>` |
| same | `Consensus` | `ErrUnexpectedDifficulty: block difficulty of <n> is not the expected value of <n> (difficulty algorithm: ASERT)` / `(difficulty algorithm: EMA)` |
| same | `Consensus` | `standalone` PoW messages (§4.1) |
| same | `Consensus` | `<hash> has invalid PoS difficulty, got <amount>, want <amount>` |
| `NextStakeDifficulty*` | `Deployment` ("inactive deployment") | `DCP0001 is not known to be active` |
| `AddTransaction` | `ScriptFailure` ("transaction script fails to execute") | `input <i> spends <outpoint> from the wrong transaction tree`; or the script engine error |
| `VoteOnOwnedTickets` | `NoPeers` ("Decred network is unreachable") | from `NetworkBackend()` with op `wallet.NetworkBackend` |
| same | (none) | `network backend does not provide deployment information` |
| `deployments.DCP0010Active` / `DCP0012Active` | `Bug` | `DCP0010 activation check requires RPC syncer` / `DCP0012 activation check requires RPC syncer` (nil querier; unreachable from `VoteOnOwnedTickets`) |
| rescan batch save | `Bug` | `len(blocks) must match len(txs)` |
| rescan start | `NotExist` ("item does not exist") | `no block at height <n> in main chain`; `starting block <hash> not found` |
| rescan start | `Invalid` | `block <hash> not in main chain` |
| `SetBirthStateAndScan` | `Invalid` | `nothing to set` |
| `FullSideChain` | (none; op `wallet.EvaluateBestChain`) | `broken assumption of height > parent.Height`; `broken assumption about connectedness of sidechain nodes` |
| `DiscoverActiveAddresses` | `Locked` / `WatchingOnly` | account discovery without the coin type private key |
| `DiscoverActiveAddresses` (account creation) | `Exist` ("item already exists") | `account named "account-<n>" already exists` (from `udb.Manager.NewAccount`) |
| `DiscoverActiveAddresses` (address rows) | `Invalid` | `child index <n> exceeds max` (from `SyncAccountToAddrIndex`) |
| `validate.MerkleRoots` | `Consensus` | `invalid regular merkle root`; `invalid stake merkle root` |
| `validate.DCP0005MerkleRoot` | `Consensus` | `invalid combined merkle root` |
| `validate.CFilterV2HeaderCommitment` | `Consensus` | `invalid header inclusion proof for cfilterv2` |
| `validate.PreDCP0005CFilterHash` | (none) | `unknown network <n>`; `hash of provided cfilters <hash> does not match expected hash <hash>` |

Panics (process aborts unless recovered by a caller): `sidechain forest internal consistency error`
(`AddBlockNode`); `no best chain for invalid sidechain tree` (`best` on an invalid root — unreachable through
`EvaluateBestChain`, which filters invalid roots first); nil dereference in `isAncestorOf` (Q-7);
`CalcASERTDiff` precondition panics; index out of range on an empty chain in
`ValidateHeaderChainDifficulties`; nil `FilterV2` in `ExtendMainChain`; division by zero for a zero gap
limit or account gap limit.

Further error paths not in the table above:

* `udb.NewTxRecordFromMsgTx` fails only when the transaction cannot be serialized. In `extendMainChain`
  the error is `E(op, err)` with the `ChainSwitch` op; in `AddTransaction` and `saveRescanned` it is
  returned through the op wrapper; in `VoteOnOwnedTickets` it is logged
  (`Failed to create transaction record: <err>`) and the vote is still published but not recorded.
* `VoteOnOwnedTickets` returns the recording update's error and the final `watchHDAddrs` error **without**
  its own op (`wallet/chainntfns.go:914-922`); the errors already carry
  `wallet.processTransactionRecord` or the `watchHDAddrs`/`RescanPoint` ops.
* `ValidateHeaderChainDifficulties` returns the view's error as is; the op
  `wallet.validateHeaderChainDifficulties` is attached inside (not to the `GetBlockHeader` failure for
  the parent of `chain[0]`, which is returned bare).
* `rescan` returns `GetMainChainBlockHashes`, `n.Rescan`, batch-save and marker-update errors bare; the
  exported wrappers add their op once.
* `rollbackInvalidCheckpoints` returns `GetMainChainBlockHashForHeight` errors other than `NotExist` and
  `Rollback` errors bare; `wallet.Open` wraps them with op `wallet.Open` and the wallet fails to open.
* `validate.PreDCP0005CFilterHash` also returns the `chainhash.Decode` error for its own constant
  (unreachable with the hard-coded strings).
* `newAddrFinder`, `find`, `filter`, `filterBlocks`, `findLastUsedAccount` and the
  `existsAddrIndexFinder` methods return `udb`, `hdkeychain`, backend and context errors bare;
  `DiscoverActiveAddresses` adds the op.

Additional panics: `reused deployment ID "<id>"` at `wallet.Open` for chain parameters that repeat a
deployment id; `mustParseHash` at package initialisation for a malformed checkpoint constant;
negative slice length in `addrFinder.find` (§4.16.5); nil address buffer in §4.16.1 step 6.3; index 0
of a ticket without outputs in `hasVotingAuthority`; `PruneChain` on an empty chain (index 0).

Surfacing:

* Syncers treat a `ValidateHeaderChainDifficulties` error as peer misbehaviour (SPV: disconnect the peer,
  `spv/backend.go:353-362`) or as a fatal sync error (RPC `getHeaders`).
* gRPC handlers map wallet errors through `translateError`; JSON-RPC handlers through their own kind
  mapping (specified in the RPC specs). `RescanProgress.Err` is returned to the gRPC stream caller.
* Non-fatal conditions are logged only: pruning failures, outpoint watch failures, notification detail
  lookups, per-ticket vote failures, vote publication failures, ticket commitment redemption failures.

Log lines emitted by this subsystem (subsystem logger of package `wallet`), exact text with `%v`
placeholders shown as `<…>`:

| Level | Text |
| --- | --- |
| warn | `Old vote version detected (v<n>), please update your wallet to the latest version.` |
| info | `Set wallet birthday to block <height> (<hash>).` |
| debug | `Updating processed txs block marker to <hash>` |
| error | `Unable to prune unmined tickets: next ticket price is unknown: <err>` |
| error | `Failed to prune unmined transactions when connecting block height <n>: <err>` |
| error | `Failed to remove pruned ticket from db: <err>` |
| error | `Failed to watch outpoints: <err>` |
| debug | `Ignoring unmined TSPend <hash>` |
| debug | `Rejected unmined orphan vote <hash> which votes on block <hash>` |
| error | `Script validation failed (outpoint <op> pkscript <hex> sigscript <hex>): <err>` |
| warn | `Refusing to add unmined transaction <hash> since same transaction already exists mined` |
| warn | `Failed to decode ticket commitment script of <hash>:<i>` |
| debug | `Failed to find redeemscript for address <addr> in address manager: <err>` |
| debug | `Unable to add multisignature output: <err>` |
| error | `Error redeeming ticket commitments: <err>` |
| error | `Cannot query transaction details for notification: <err>` |
| debug | `Marked address <addr> used` |
| error | `Failed to read ticket purchase transaction for owned winning ticket <hash>: <err>` |
| warn | `Block disapprove percent set on mainnet` |
| info | `Disapproving block <hash> voted with ticket <hash>` |
| error | `Failed to create vote transaction for ticket hash <hash>: <err>` |
| error | `Failed to create treasury vote for ticket hash <hash>: <err>` |
| error | `Failed to sign vote for ticket hash <hash>: <err>` |
| error | `View failed: <err>` |
| error | `Failed to create transaction record: <err>` |
| info | `Voting on block <hash> (height <n>) using ticket <hash> (vote hash: <hash> bits: <n>)` |
| error | `Failed to send one or more votes: <err>` |
| info | `Rescan of block <n> discovered previously unmined transaction <hash>` |
| info | `Rescan of block <n> discovered new transaction <hash>` |
| info | `Rescanning block range [<n>, <n>]...` |
| info | `Rescan complete` |
| info | the bare error, twice possible, in `rescanPoint` lookups |
| debug | `DiscoverActiveAddresses: activeCoinType=<n>` |
| info | `Discovering used accounts` |
| info | `Discovering used addresses for <n> account(s)` |
| debug | `Found match for script <hex> path <struct> in block <hash>` |
| debug | `Found match for script <hex> account <n> in block <hash>` |
| info | `Account <n> next child indexes: external:<n> internal:<n>` |
| info | `Updating DB with discovered addresses...` |
| info | `Finished address discovery` |
| warn | `Wallet contains addresses derived for the legacy BIP0044 coin type and seed restores may not work with some other wallet software` |
| info | `Upgrading wallet from legacy coin type <n> to SLIP0044 coin type <n>` |
| error | `Coin type upgrade failed: <err>` |
| warn | `Continuing with legacy BIP0044 coin type -- seed restores may not work with some other wallet software` |
| info | `Upgraded coin type.` |
| info | `Loading active addresses and unspent outputs...` |
| info | `Registered for transaction notifications for <n> HD address(es)` |
| info | `Registered for transaction notifications for <n> imported address(es)` |
| info | `Registered for transaction notifications for all relevant outputs` |

---

## 8. Compatibility-critical behaviours

A drop-in replacement must reproduce each of these exactly; the dependent party is named.

1. **Marker and birthday bytes (existing `wallet.db`, Go wallet reopening the file).** `lasttxsblock` is a
   bare 32-byte hash that only moves forward relative to its main-chain ancestor; `birthstate` is the 45-byte
   record of §3.2 with flags `0x01` (height) and `0x02` (time), big-endian height and Unix seconds.
2. **Rescan point definition (gRPC `RescanPoint`, Decrediton, syncers, ticket buyer).** Nil iff the
   main-chain ancestor of the marker is the tip; otherwise the main-chain block one above that ancestor
   (§4.14).
3. **Marker advancement rule in `ChainSwitch` (existing databases, sync progress).** Advance only through
   the leading run of nodes with non-nil `RelevantTxs`, and only when the current rescan point is not more
   than one block below `chain[0]` (§4.8 step 9). `nil` versus empty `RelevantTxs` is a semantic
   distinction that must survive the port (use `Option<Vec<_>>`).
4. **Birthday resolution during sync (restored wallets).** The birthday block is the *parent* of the first
   attached block whose timestamp is strictly after the birthday time, or of the block at birthday height
   + 1; the marker is moved to that parent in the same transaction (§4.8 step 8.3).
5. **Chain selection (agreement with dcrd and other wallets on the tip).** Strictly-greater cumulative work
   relative to the replaced main-chain suffix; ties keep the current chain; orphan trees and trees rooted at
   an invalid (bad checkpoint) node are ignored; work is `floor(2^256 / (target + 1))`.
6. **Header acceptance rules (SPV peers).** Exactly the checks of §4.1/§4.2 and no others: height
   continuity, bits equal to EMA or (when version/height/blake3-solved conditions hold) ASERT against a
   candidate anchor, PoW under blake256 *or* blake3, DCP0001 stake difficulty from the hard-coded
   activation height. Stricter or looser checks change which peers are disconnected and which chains are
   followed.
7. **EMA arithmetic.** Euclidean big-integer division, arithmetic right shift, the clamp order of §4.3
   step 6, the testnet3 min-difficulty rule below height 962928 and max-difficulty clamp from 962928.
8. **ASERT arithmetic and testnet clamp on compact bits** (§4.4).
9. **DCP0001 stake difficulty** (§4.5) — used for header validation, for the ticket price returned to
   clients and used to price tickets, and for pruning unmined tickets. A one-atom difference produces
   invalid tickets or rejected headers.
10. **Testnet3 checkpoint** at 962928 with the hash of §3.8, including the open-time rollback of a
    database that followed the other fork.
11. **`validate` rules (SPV peers, dcrd).** Either separate-roots or combined-root merkle validation
    passes a block; cfilter inclusion proofs are verified against `StakeRoot` from height 431488 (mainnet)
    / 323328 (testnet3) / always (other networks); the two pre-DCP0005 cfilter set hashes.
12. **Cfilter key and match entries.** Key = first 16 bytes of the merkle root; entries = version-0
    P2PKH/P2SH payment scripts of watched addresses, with `B = 19`, `M = 784931`. Outpoints are not
    entries.
13. **Relevance and recording rules of `processTransactionRecord`** (database contents, balances, RPC
    results): which outputs become credits (non-zero value, standard script, owned address), tree
    assignment (1 for stake-tagged scripts), ticket commitments on odd outputs of tickets recorded per
    owned reward address, vote/revocation commitment redemption, multisig script import from signature
    scripts, P2SH multisig output records, zero-value outputs ignored, addresses marked used with address
    rows extended to `lastUsed + gapLimit`.
14. **Unmined transaction acceptance in `AddTransaction`** (protection against forged spends from SPV
    peers; JSON-RPC `addtransaction`): recently published hashes ignored; unmined tspends ignored; unmined
    votes not voting on the tip ignored; inputs spending wallet UTXOs must use the right tree and pass
    script verification with the listed flags; a block hash not in the main chain demotes the transaction
    to unmined.
15. **Manual ticket mode.** Tickets in attached blocks are skipped unless already recorded.
16. **Unmined pruning on every chain switch** with the next ticket price (or 0 when unknown), with removed
    transaction notifications and VSP ticket removal (`"pruned"`).
17. **Rescan batching and progress (gRPC `Rescan` stream, Decrediton progress UI, JSON-RPC
    `rescanwallet`).** Batches of 2000 blocks, genesis skipped, one progress value per batch equal to the
    last height of the batch, marker advanced per batch before the progress is reported, channel closed at
    the end, error delivered as a final element.
18. **Discovery outcome (accounts and address indexes after seed restore; must match what the Go wallet
    and Decrediton would produce from the same chain data).** Account names `account-<n>`; accounts
    created through the last used account; per branch, address rows through `lastUsed + gapLimit`;
    last-used/last-returned indexes only raised; the definition of "used" (§4.16.3 for SPV, dcrd's
    exists-address index for RPC); the legacy→SLIP0044 coin type upgrade conditions (exactly one account,
    no used address on either branch, upgrades not disabled, coin type known and legacy) followed by a
    second discovery pass.
19. **Watched address range (dcrd `loadtxfilter` calls; SPV filter contents).** Children
    `0 ..= lastReturned + gapLimit` on first watch (limited by `--watchlast`), incremental thereafter, and no
    incremental watching while a rescan point exists.
20. **dcrd RPC call pattern during discovery** (`existsaddresses` with batches of derived addresses in
    child-index order; bit `i` ↔ address `i`) and during watching/rescan (`loadtxfilter`, `rescan`), as
    consumed through `NetworkBackend`.
21. **Vote construction orchestration (network consensus, VSP expectations).** Votes only when voting is
    enabled, the height is at least `StakeValidationHeight - 1`, the backend can report deployments, the
    ticket is recorded, output 0 pays a wallet address and its private key is available; per-ticket vote
    bits override defaults; treasury vote output format of §3.7 with transaction version 3; DCP0010/DCP0012
    subsidy selection from dcrd deployment status; publish before recording.
22. **Notification ordering** (gRPC `TransactionNotifications`, `MainTipChanged`, JSON-RPC websocket
    notifications): §6.4.
23. **Vote outpoint watching (dcrd `loadtxfilter`, SPV filter).** The outpoints loaded for a freshly
    built vote are those of §4.12.1 (owned stake-generation outputs by hash160), loaded after
    publication and before the vote is recorded.
24. **Exported no-op and helper API (library embedders, the RPC servers in this repository).**
    `RevokeOwnedTickets` stays callable and returns nil; `RescanFilter` keeps its exact (non-probabilistic)
    semantics including the compressed-pubkey → pubkey-hash fallback in `ExistsAddress`;
    `CheckpointHash`/`BadCheckpoint` keep their signatures; `NextStakeDifficulty*` fail with kind
    `Deployment` (not a price of zero) when DCP0001 activation is unknown; ticket purchase then falls
    back to the backend's `StakeDifficulty` (`wallet/createtx.go:1209-1211`), while gRPC `TicketPrice`
    (`internal/rpc/rpcserver/server.go:893-897`), VSP ticket purchase (`wallet/wallet.go:1626`) and
    mixing (`wallet/mixing.go:274`) return the error.
25. **Idempotent re-attachment.** A block already recorded in the main chain (`Exist` from
    `ExtendMainChain`) is still notified and its relevant transactions re-processed (§4.9), so a
    replayed `ChainSwitch` cannot lose transactions.
26. **Log text.** Operators and test harnesses grep for `Rescanning block range`, `Rescan complete`,
    `Voting on block`, `Set wallet birthday to block`, `Finished address discovery`, and the discovery
    progress lines (§7).
27. **Birthday visibility (JSON-RPC `walletinfo`, gRPC `BirthBlock`).** A pending birthday
    (either flag set) is reported as absent (`""`/`0`, or `NotFound` `birth block is pending...`); a
    database that never had a birthday set reports the genesis block at height 0, because the 25→26
    upgrade stores exactly `00 || genesis hash || 00000000 || fffffff1886e0900` (§3.2). The Rust
    upgrade/creation path must write those 45 bytes, including the zero-time encoding, for the Go
    wallet to read the same state back.
28. **No mempool intake while a rescan point exists (SPV peers).** Transaction announcements are not
    requested from peers until `RescanPoint` is nil (§4.14), and `watchHDAddrs(false)` loads nothing in
    that state; a port that fetches them earlier records unmined transactions the Go wallet would not
    have at the same moment.

---

## 9. Quirks

Each item is behaviour a faithful port either reproduces or consciously diverges from.

* **Q-1 — Positional-only PoW difficulty.** The wallet accepts a header whose bits match EMA *or* ASERT
  against any eligible candidate anchor. The source comment says contextual checks "that happen later"
  pin the algorithm; no such contextual check exists in the wallet (it is copied from dcrd).
  `wallet/difficulty.go:363-377,460-540`.
* **Q-2 — PoW checked against a caller-supplied hash, under either hash function.** The blake256 check
  uses `BlockNode.Hash`, which the SPV syncer takes from the *next* header's `PrevBlock` for all but the
  last header of a batch (`spv/backend.go:327-338`); the header is not re-hashed here. A header passes if
  either blake256 or blake3 meets the target, independent of which difficulty algorithm matched in Q-1.
  `wallet/difficulty.go:883-893`.
* **Q-3 — `isSolvedBlake3` does not range-check bits.** Only `hash <= CompactToBig(bits)` is tested when
  choosing the ASERT branch, so an absurdly easy `bits` value selects the branch (the header is then
  rejected by the bits equality test or the later `CheckProofOfWork` range check, unless Q-7 triggers
  first). `wallet/difficulty.go:428-432`.
* **Q-4 — Testnet ASERT clamp compares compact encodings numerically**, and applies at every height.
  `wallet/difficulty.go:335-337`.
* **Q-5 — EMA leftovers.** `nextDiffBig.Set(nextDiffBig)` is a no-op branch; time differences may be
  negative and are divided with Euclidean semantics; `weights` is a `uint64` built from shifts by
  `(WorkDiffWindows - windowPeriod) * alpha` (overflows silently for large parameters).
  `wallet/difficulty.go:157-171,230-238`.
* **Q-6 — One candidate-anchor cache per wallet.** A single header pointer (a `Wallet` field) shared by
  all chains and all callers of that wallet, never cleared; its only effect on results is ordering of candidate attempts, except for Q-7.
  `wallet/difficulty.go:270-282,482-494`.
* **Q-7 — Nil dereference in `isAncestorOf`.** `ancestorHeaderAtHeight` returns `(nil, nil)` when the
  requested height is above the target's height; `isAncestorOf` then calls `BlockHash()` on the nil header,
  which panics in Go (dcrd's equivalent returns false). This occurs when a header reaches step 5 of §4.2
  (version ≥ minimum, height ≥ first possible activation height, blake3 hash ≤ its own claimed target)
  while its parent's height is **below** the cached candidate anchor's height, e.g. a header forking off
  the main chain below the anchor. `wallet/difficulty.go:675-709`. Whether an untrusted SPV peer can deliver
  such a header past the p2p layer's own checks is UNVERIFIED here (settle by reading the header sanity
  checks in the `p2p` package). The Rust port must return "not an ancestor" instead of aborting.
* **Q-8 — DCP0001 gate uses the pre-switch tip height** when pricing tickets for unmined pruning.
  `wallet/chainntfns.go:246`. Only matters within a switch that crosses height 149248 on mainnet.
* **Q-9 — Side effects outside the database transaction are not atomic with it.** In-memory
  `lockedOutpoints` deletions, queued notifications, VSP `RemoveTicket` calls and (for multisig imports)
  backend filter loads happen inside the update closure and are not undone if the transaction fails. After
  a successful commit, a `watchHDAddrs` error makes `ChainSwitch` return an error **without** returning the
  detached chain, pruning the forest, expiring mix messages or sending tip notifications.
  `wallet/chainntfns.go:261-316,469-473,505-517`.
* **Q-10 — Rescan saver on backend failure.** When `n.Rescan` returns an error, the saver goroutine still
  commits the pending partial batch and then blocks forever sending on `lastBatchErr` (goroutine leak).
  The progress send is a plain blocking send: a consumer that stops reading (for example a gRPC stream
  whose `Send` failed) leaves the rescan goroutine blocked once the 1-element buffer is full.
  `wallet/rescan.go:296-348`, `internal/rpc/rpcserver/server.go:472-481`.
* **Q-11 — Cfilter account discovery is not a binary search.** After the linear scan of the first
  `2 * accountGapLimit` accounts, the fallback loop only ever lowers `hi`, probing a fixed sequence of
  segments (`mid = (hi + 2) / 2` repeatedly). With the default account gap limit of 10 the probed account
  ranges end `…, [130,140), [70,80), [40,50), [20,30)`; used accounts in other segments at or above 20 are
  not found. `wallet/discovery.go:516-534`. The exists-address variant (§4.16.4) is a real bisect.
* **Q-12 — Buffers of discovered accounts start at `lastUsed = 0`.** New account buffers are created with
  zero values rather than `0xFFFFFFFF`; for a branch with no discovered usage the buffer is never
  corrected, so the next address handed out for that branch is child 1, not child 0, until the wallet is
  reopened. `wallet/discovery.go:811-815,919-935` versus `wallet/cointype.go:53-55`.
* **Q-13 — `cacheMissingCommitments` returns early.** A 500-hash chunk with nothing to fetch ends the
  function (`return nil` where `continue` was intended); later chunks stay unfetched and their blocks read
  as "no commitments". Reachable only with more than 500 matching blocks. `wallet/discovery.go:82-84`.
* **Q-14 — Treasury vote order follows Go map iteration.** `GetAllTSpends` iterates a map, so the order of
  `(hash, vote)` pairs, and therefore the vote's bytes and hash, differ run to run when more than one
  tspend is voted. `wallet/wallet.go:274-289`, `wallet/chainntfns.go:806-824`.
* **Q-15 — RPC account discovery ignores the `gapLimit` argument.** `branchUsed` derives
  `f.wallet.gapLimit` addresses; `findLastUsedAddress` uses `f.gapLimit`. `wallet/discovery.go:627,649`.
* **Q-16 — Child index mislabeled after an invalid child.** `deriveChildAddresses` drops invalid children,
  but `find` computes `index = mid*gapLimit + position in the returned slice`.
  `wallet/discovery.go:217-229`, `wallet/addresses.go:1004-1021`. Probability per child is about 2^-127.
* **Q-17 — Vote bookkeeping oddities.** After nil votes are removed by swap-compaction, the info log pairs
  `voteRecords[i]` with `ticketHashes[i]` and `usedVoteBits[i]` from the uncompacted arrays, so the logged
  ticket hash/bits can belong to a different ticket whenever any owned winner was skipped. A failure of
  the build view is logged, not returned. `PublishTransactions` is invoked with zero transactions when no
  vote was built. `GetAllTSpends` (with its own nested DB view) is called once per ticket inside the
  build view. `wallet/chainntfns.go:735-893`.
* **Q-18 — Wrong op string.** `FullSideChain` errors carry op `wallet.EvaluateBestChain`.
  `wallet/sidechains.go:363`.
* **Q-19 — Forest nondeterminism and truncation.** Equal-work tips and the order of trees produced by
  `pruneChain` depend on Go map iteration; with all tips invalid `best()` returns only the root even when
  valid intermediate nodes exist; a re-rooted node keeps its stale `parent` pointer.
  `wallet/sidechains.go:147-186,206-223`.
* **Q-20 — Rescan documentation versus code.** The comment says progress starts "with the start height";
  the code reports only batch ends. The first rescan in a process never logs discovered transactions.
  `wallet/rescan.go:151-154,208-218,346-348`.
* **Q-21 — Reorg back onto a detached block without wallet transactions stalls the marker.** Detached
  nodes get `RelevantTxs = removedTxs[hash]`, which is nil for blocks that held no wallet transactions.
  If such a node is later re-attached unchanged (the RPC syncer's `AddBlockNode` rejects the fresh node
  from `blockconnected` as a duplicate and keeps the old one), the marker loop stops at it, the gap rule
  then blocks every later advance, and `RescanPoint` stays non-nil (no incremental address watching,
  ticket buyer idle) until a rescan runs. The SPV syncer overwrites `RelevantTxs` for every best-chain node
  when the rescan point is nil and is not affected. `wallet/chainntfns.go:160-166,212-239`,
  `wallet/sidechains.go:88-102`, `chain/sync.go:874-883`.
* **Q-22 — `big.Int.Int64()` on an out-of-range stake difficulty** is undefined by Go's documentation.
  Not reachable with real chain parameters (the product is bounded far below 2^63 after the divisions).
  `wallet/difficulty.go:663`.
* **Q-23 — `AddTransaction` with a block hash that is not in the main chain** silently records the
  transaction as unmined (after the unmined checks). `wallet/chainntfns.go:365-380`.
* **Q-24 — No relevance check before insertion.** The record is inserted before any input or output is
  examined; a caller that passes an unrelated transaction gets it stored. The comment still says SPV
  support is future work. `wallet/chainntfns.go:446-462`.
* **Q-25 — Per-address repetition.** For an output with several owned addresses (bare multisig),
  `AddCredit`/`markUsedAddress` run once per address and the outpoint is appended to the watch list once
  per address. `wallet/chainntfns.go:579-607`.
* **Q-26 — Forest panics on inconsistent heights; RPC header sync adds before validating.** See §4.6.2.
  In `getHeaders` a batch that fails validation stays in the forest. `wallet/sidechains.go:274-303`,
  `chain/sync.go:281-322`.
* **Q-27 — `SetBirthStateAndScan`** does not scan, returns one error path unwrapped, mutates its argument
  and has no in-repository caller outside `wallet/wallet_test.go`. `wallet/rescan.go:532-581`.
* **Q-28 — Inclusive watch ranges.** `watchHDAddrs` loads `count + 1` children on a first watch and
  re-sends the `lastWatched` child on each incremental watch; the SPV backend appends every received
  address to its match-entry list without deduplication. `wallet/wallet.go:1003-1036`,
  `spv/backend.go:423-427`.
* **Q-29 — `validate` leniency.** No height decides which merkle rule applies; cfilters below the DCP0005
  height are accepted without proof (only the optional whole-set hash covers them).
  `validate/validate.go:74-89`, `spv/backend.go:82-85`.
* **Q-30 — Checkpoint plumbing.** The checkpoint error has no op of its own; checkpoint heights are
  listed twice (`CheckpointHash` and `rollbackInvalidCheckpoints`). `wallet/chainntfns.go:36-42`,
  `wallet/checkpoints.go:26-34,48-51`.
* **Q-31 — Go per-iteration loop variables.** Closures in discovery capture `hash`, `commitments`, `fs`,
  `i` and `b` per iteration (Go ≥ 1.22 semantics; the module declares `go 1.26.0`).
  `wallet/discovery.go:257-283,318-356`.
* **Q-32 — Receive time is wall-clock at processing.** Every record built here uses `time.Now()`, including
  rescans and re-attachment after a reorg. Whether an existing record's stored time is preserved is decided
  by `udb` (see the transaction store spec). `wallet/chainntfns.go:71,346,874`, `wallet/rescan.go:196`.
* **Q-33 — Exists-index address discovery overwrites in-memory last-used values** (possibly with
  "none"), relying on the database only ever raising them; `branchUsed` tests whole bytes of the bitset.
  `wallet/discovery.go:635-639,691-703`.
* **Q-34 — Unsynchronized read of `w.vspClients`** inside `ChainSwitch`. `wallet/chainntfns.go:265`.
* **Q-35 — `ChainSwitch` on an already-current tip** returns `(nil, nil)` but still runs every
  post-transaction step, including a `MainTipChanged` notification with no attached blocks.
  `wallet/chainntfns.go:118-120,282-315`.
* **Q-36 — Latent nil dereference in the marker step.** `rescanPoint` may return nil, which is passed to
  `GetBlockHeader`. This requires the marker's main-chain ancestor to be the block just attached as tip,
  which the strict-work rule appears to make unreachable. `wallet/chainntfns.go:224-231`.
* **Q-37 — Mixed ancestor walks.** `sumPurchasedTickets` and `findPrevTestNetDifficulty` follow
  `PrevBlock` hashes below `chain[0]`; `ancestorHeaderAtHeight` and the EMA window walk use main-chain
  heights. They agree only when the starting header's ancestry is the main chain;
  `NextStakeDifficultyAfterHeader` with a stored side-chain header mixes the two.
  `wallet/difficulty.go:581-600,675-695`.
* **Q-38 — Discovery's notion of "used" is narrower than the filter.** Filters commit previous-output
  scripts and ticket commitment scripts, which `blockCommitments` omits, so such matches fetch a block and
  then count as unused. An address that only ever appears as a ticket voting address (output 0) is in
  neither. `wallet/discovery.go:37-67`, `dcrd/gcs/v4@v4.1.1/blockcf2/blockcf.go:214-246`.

* **Q-39 — Source TODO/XXX/deprecation markers in scope** (each names behaviour the Go authors consider
  provisional; a port reproduces the behaviour, not the intent):
  `wallet/chainntfns.go:446-449` (all notified transactions assumed relevant; see Q-24);
  `:474` (`TODO: the prevout's actual pkScript version is needed` — version 0 is assumed);
  `:503` (`TODO Markused script address?` — imported multisig script addresses are never marked used);
  `:520-528` (the multisig-output lookup error is ignored on purpose, e.g. during rescans);
  `:658` (`TODO: Avoid the extra db hits` — one `UniqueTxDetails` lookup per processed transaction);
  `:724-728` (`TODO` — when blocks arrive quickly the transaction store can be out of step with the
  winning-ticket notification being voted on; no mitigation exists);
  `:927-930` (`RevokeOwnedTickets` deprecated, to be removed in the next major version);
  `wallet/rescan.go:305` (`XXX: tune this` — the 256-transaction save threshold);
  `wallet/rescan.go:33-35` (`otherAddresses` "only exists for completeness");
  `wallet/difficulty.go:8-9` (file copied from dcrd and adapted to wallet header storage);
  `wallet/difficulty.go:92-94` (testnet min-difficulty reduction deprecated);
  `wallet/difficulty.go:164-165` (weight overflow warning, Q-5);
  `wallet/discovery.go:102` (`hi` is "set to lo - 1 when finished, be cautious of unsigned underflow").
  `wallet/sidechains.go`, `wallet/checkpoints.go` and `validate/validate.go` contain no such markers.
* **Q-40 — Two notions of "owned output" for watching.** `processTransactionRecord` uses
  `manager.Address` and skips zero-value outputs; `appendRelevantOutpoints` (votes) uses
  `manager.ExistsHash160` and does not. The vote's watch list is taken from the latter and the
  former's result is discarded in `VoteOnOwnedTickets`. `wallet/chainntfns.go:854,907`,
  `wallet/wallet.go:5004-5060`.
* **Q-41 — Start hash of discovery is only a height.** `GetMainChainCFilters` reads the start header's
  height and then walks main-chain block records; a side-chain start hash silently scans the main chain
  from that height, and a start above `tip + 1` panics in `addrFinder.find` on a negative slice
  length. `wallet/discovery.go:189-196,399-400`, `wallet/udb/txmined.go:505-543`.
* **Q-42 — Upgrade log prints the active coin type.** The "legacy" number in
  `Upgrading wallet from legacy coin type <n> …` is the value read from the database, not the
  legacy constant from `udb.CoinTypes` (they are equal whenever the line is reached with a wallet
  created by this software). `wallet/discovery.go:718,966-967`.
* **Q-43 — Reused hash storage in `rescan`.** `rescanBlocks` is a subslice of one 2000-element array
  reused across batches; the pointer `&rescanBlocks[last]` passed to the marker update and the value
  copied into `rescanFrom` are taken before the next batch overwrites it. A `NetworkBackend.Rescan`
  that retained the slice beyond its return would observe later batches' hashes.
  `wallet/rescan.go:220,234-235,341,349`.
* **Q-44 — `ChainSwitch` work comparison assumes `chain[0]` is a tree root.** `workSum` is relative to
  the node's tree root; see §4.8. `wallet/chainntfns.go:107,151`, `wallet/sidechains.go:57,114,124`.
* **Q-45 — Already-recorded blocks are re-notified and re-processed.** The `Exist` result of
  `ExtendMainChain` is swallowed and the function continues with the attached-block notification and
  transaction processing. `wallet/chainntfns.go:44-80`.
* **Q-46 — Named return assigned (not shadowed) in the P2SH branch.** `expandedScript, err = w.manager.RedeemScript(...)`
  assigns the function's named `err` result; it has no effect because every later return is explicit,
  but the script variable is overwritten with the failed call's result (nil).
  `wallet/chainntfns.go:612-621`.
* **Q-47 — Redundant inner length test in `AddTransaction`.** The filter-load branch re-tests
  `len(watchOutPoints) > 0` already required by the enclosing condition; consequently `watchHDAddrs` is
  **not** called after an unmined transaction that created no watchable outpoint (for example one that
  only spends wallet outputs, or one that only added a ticket commitment), unlike `ChainSwitch` which
  always calls it. `wallet/chainntfns.go:423-434`.
* **Q-48 — `validateHeaderChainDifficulties` blames the header for database failures in the positional
  check** (returns `chain[idx:]` with the error), but returns a nil slice for a database failure in the
  stake-difficulty calculation. `wallet/difficulty.go:879-900`.
* **Q-49 — Sidechain doc comment versus code.** `EvaluateBestChain`'s comment says "an empty slice
  indicates there is no better chain"; the code returns a nil slice. `AddBlockNode`'s comment reads
  "Returns true iff the node if the node was not a duplicate". `FullSideChain` returns its **input
  slice itself** (not a copy) when the input is empty or no tree contains the parent, and a fresh slice
  otherwise. `wallet/sidechains.go:241-246,360-412,414-417`.

* **Q-50 — gRPC `CreateWallet` tests `set_birth_time` twice.** After building a `SetFromTime` state from
  `birth_time` (rejected with `InvalidArgument`
  `birth time cannot be in the future or too close (one day) to the present` when less than 24 h old),
  the handler tests `req.SetBirthTime` again where `req.SetBirthHeight` was intended and replaces the
  state with `SetFromHeight`, `Height = req.BirthHeight`. Net effect: `set_birth_time` stores a
  height-based birthday at `birth_height` (0 unless also supplied) and discards the time;
  `set_birth_height` alone falls through to the default `SetFromHeight`, `Height = 0`, ignoring
  `birth_height`; setting both is `InvalidArgument` `both set birth time and height cannot be set`. The
  handler belongs to the gRPC spec; it is recorded here because it decides the `birthstate` bytes a
  gRPC-created wallet starts with. `internal/rpc/rpcserver/server.go:2713-2752`.
* **Q-51 — Discovered account names can collide.** Discovery names account `n` `account-<n>`
  unconditionally; if an existing account of a different number already has that name, `NewAccount`
  fails with kind `Exist` and the whole discovery fails (no account of the batch is created).
  `wallet/discovery.go:785-789`, `wallet/udb/addressmanager.go:2100-2105`.
* **Q-52 — `--watchlast` with a never-used branch watches nothing on that branch.** For a branch with no
  returned address, `end = gapLimit - 1`, so the unsigned `end - w.gapLimit` wraps to `0xFFFFFFFF`, the
  test `> w.watchLast` passes, and `start = end - gapLimit - watchLast = 0xFFFFFFFF - watchLast`, which
  is above `end`; the load loop does not run and the branch's lookahead addresses `0 .. gapLimit-1` are
  not sent to the backend on that call (`lastWatched` is still set to `end`, so they are not sent
  later either, apart from child `gapLimit - 1`, the inclusive lower bound of the next incremental
  watch). With `watchLast == 0` the branch is watched normally.
  `wallet/wallet.go:931-932,1003-1006,1014`.
* **Q-53 — `filterBlocks` leaks its workers on a database error.** The cfilter batch read's error path
  returns without closing the work channel, leaving `runtime.NumCPU()` goroutines blocked on it for
  the life of the process; only the cancellation and normal paths close it.
  `wallet/discovery.go:397-405`.
* **Q-54 — `AddTransaction` ordering with a non-main-chain block hash.** The unmined-tspend and
  orphan-vote rejections test `blockHash == nil`, before the block is looked up. A tspend or a vote
  passed with a non-nil hash of a block that is not in the main chain is therefore recorded as
  **unmined** (Q-23) without either rejection. Reachable from JSON-RPC `addtransaction`.
  `wallet/chainntfns.go:336-380`.
* **Q-55 — Backend block results are trusted positionally and by length** (§3.6): discovery never
  compares a returned block's hash with the requested hash, and a result longer than the request
  panics. `wallet/discovery.go:85-91,342-354`.

---

## 10. Upstream tests

| Test file | Lines | What it pins | Port strategy |
| --- | --- | --- | --- |
| `wallet/sidechains_test.go` (`TestPruneChainFromSRT`) | 140 | `pruneChain` re-rooting: a four-branch tree pruned along one branch yields the right roots, children and tips; nested prunes down to empty | Port directly; pure data structure, random headers only need distinct hashes |
| `wallet/reorg_test.go` (`TestReorg`) | 238 | `AddBlockNode`, `EvaluateBestChain`, `ChainSwitch` and forest pruning across block one, a two-block extension, a non-winning two-block side chain, and a winning three-block side chain; `BlockInMainChain` results after reorg; detached chain re-added as a tree rooted at 2a | Blocks come from dcrd `chaingen` on simnet with `blockcf2.Regular` filters. Regenerate the six headers + filters (block one, 2a, 3a, 2b, 3b, 4b) with a Go oracle shim and replay them in Rust |
| `wallet/discovery_test.go` (`TestDiscoveryCursorPos`) | 101 | Address-buffer cursor/last-used adjustment in §4.16.1 step 6.3: cursor not reset when nothing is found; reduced by the newly used distance when an address inside the returned window is marked used. Runs with coin type upgrades disabled and a genesis-only chain | Port directly against the Rust address manager with a no-op backend |
| `wallet/wallet_test.go` (`TestSetBirthStateAndScan`, `:147-231`) | 231 (file) | `SetBirthStateAndScan` on a nine-block simnet chain built through `AddBlockNode`/`EvaluateBestChain`/`ChainSwitch`: height 6 → block 6a; height 0 → genesis; time one minute in the future → the tip (9a); time one minute before the test started → genesis; neither flag → error. Only the resulting `BirthState().Hash` is checked | Port with the same oracle-generated chain as `TestReorg` |
| `wallet/udb/txmined_test.go` (`TestSetBirthState`) | 80 | Round trip of a random `BirthdayState` (hash, height, whole-second time, both flags) through `udb.SetBirthState`/`udb.BirthState` on a clone of the `mgr_watching_only.kv` test database; does not pin the byte layout | Port, and add a fixed-bytes vector for §3.2 |
| `wallet/network_test.go` | 40 | Test support only: `mockNetwork`, a `NetworkBackend` whose every method returns zero values and nil (`Blocks` and `CFiltersV2` return nil slices, `Rescan` never calls `save`, `Synced` returns `(false, 0)`, `WrapContext` returns the context unchanged). Used by `TestDiscoveryCursorPos` and the address tests; it does not implement `UsedAddresses` or `Deployments`, so those tests take the cfilter path | Port as a no-op backend test double |
| `wallet/setup_test.go` | 55 | Test support only: `testWallet(ctx, t, cfg, seed)` creates a temporary `bdb` database, runs `Create` with the insecure public passphrase and a test private passphrase, then `Open`; the shared default config is simnet with mixing enabled | Port as the common fixture |
| `wallet/main_test.go` | 24 | `TestMain` with a `-log` flag that attaches a stderr logger (subsystem tag `WLLT`) | Not needed |
| `wallet/addresses_test.go` (uses `watchHDAddrs`) | 307 | Address derivation and gap-limit behaviour with a mock backend (owned by the address spec) | Shared with that spec |

`wallet/locking_test.go`, `wallet/mixing_test.go` and the remaining tests of `wallet/wallet_test.go`
(`TestCoinbaseMatured`, `TestTicketMatured`, `TestTicketExpired`, `TestVotingXprivFromSeed`) touch no
symbol of this scope. There is no `testdata` directory for this scope; `TestReorg` and
`TestSetBirthStateAndScan` generate their chains at run time with dcrd `chaingen`.

The `validate` package has no test file. No file under `spv/`, `chain/`, `internal/rpc/` or `ticketbuyer/`
contains a test that calls a symbol of this scope (checked by searching every `_test.go` in the
repository for the exported and unexported names of §2).

Untested upstream (no test references any of these symbols): all of `wallet/difficulty.go` (EMA, ASERT
positional logic, stake difficulty, `ValidateHeaderChainDifficulties`), `wallet/checkpoints.go`,
`validate/validate.go`, `RescanFilter`, `rescan` and its wrappers, `rescanPoint`, birthday resolution
inside `ChainSwitch` (§4.8 step 8.3; only `SetBirthStateAndScan` and the `udb` round trip are tested),
`processTransactionRecord`, `AddTransaction`, `VoteOnOwnedTickets`, account discovery (both variants),
cfilter-based address discovery beyond the genesis-only case, `filterBlocks`, `blockCommitments`,
`cacheMissingCommitments`, `watchHDAddrs` ranges, `LoadActiveDataFilters`.

Vectors to generate through a Go oracle shim (calling the pinned dcrwallet code, since most functions are
unexported the shim lives in package `wallet` as an external `_test` helper or uses a patched copy):

1. **Header validation over real chains.** For mainnet and testnet3: header ranges crossing EMA retarget
   boundaries, the DCP0001 activation height (149248), the blake3 activation interval, and (testnet3) the
   962928 boundary; expected `ValidateHeaderChainDifficulties` result per header, plus mutated headers
   (bits ±1, sbits ±1, height off by one, version below minimum, blake3-solved but EMA bits) with expected
   error strings.
2. **`calcNextBlake256Diff` / `calcNextStakeDiffV2` / `estimateSupply`** — dcroxide already carries Go-derived
   vectors for the dcrd originals; add wallet-shaped cases where part of the ancestry comes from `chain`
   and part from the database.
3. **Forest scenarios**: orphan merge, duplicate with filter copy, invalid propagation, `Prune` depth,
   `FullSideChain` prefixes, `EvaluateBestChain` ties.
4. **`processTransactionRecord` matrices**: regular payment, ticket with owned voting address only, ticket
   with owned commitment only, vote, revocation, treasury spend payout, P2SH multisig funding and
   spending, zero-value outputs, manual ticket mode; expected database rows via a bucket dump diff.
5. **`AddTransaction` rejections**: wrong tree, bad signature, orphan vote, unmined tspend, recently
   published.
6. **Rescan**: batch boundaries at 2000 blocks, the 256-transaction save threshold, genesis start, progress
   sequence, marker after each batch.
7. **Discovery**: synthetic chains with usage at chosen accounts/indexes for both backends, including the
   Q-11 probe sequence, gap-limit boundaries (index 19/20/21 with gap 20), stored last-used resume, coin
   type upgrade and its second pass; expected accounts, address rows and account variables.
8. **`validate`**: blocks before and after DCP0005 for both merkle functions; cfilter proofs with valid,
   wrong-index and truncated proofs; the two set hashes recomputed from chain data.
9. **Treasury vote script**: one, two and enough tspends to cross the 75-byte push boundary.

---

## 11. Rust porting notes

### 11.1 Suggested module boundary

One crate (working name `purse-chain`) depending on the database, address manager and transaction store
crates, with these modules:

| Module | Contents | I/O |
| --- | --- | --- |
| `forest` | `SidechainForest`, `BlockNode`, tree operations (§4.6) | none (pure; unit-testable in isolation) |
| `difficulty` | header view over `chain` + main-chain DB, positional check, stake difficulty, `validate_header_chain` | DB read |
| `checkpoints` | checkpoint table, bad-checkpoint test, open-time rollback | DB write |
| `validate` | the four context-free checks | none |
| `chain_switch` | `evaluate_best_chain`, `chain_switch`, `extend_main_chain` | DB write, notifications, backend |
| `ingest` | `process_transaction_record`, `add_transaction`, unmined input verification | DB write, backend |
| `rescan` | rescan loop, progress type, rescan point, birthday | DB, backend |
| `discovery` | both discovery variants, block commitments, coin type upgrade trigger | DB, backend |
| `watch` | `watch_hd_addrs`, `load_active_data_filters`, `RescanFilter` | DB, backend |

`VoteOnOwnedTickets` belongs with the staking crate; it needs only `process_transaction_record`,
`watch_hd_addrs` and the backend trait from here.

Represent the `NetworkBackend` optional capabilities (`UsedAddresses`, `Deployments`) as explicit optional
trait objects or default trait methods returning "unsupported", not downcasts, and keep the selection
rule: capability present → index-based discovery / voting possible.

### 11.2 Reuse (items inspected)

| Need | Candidate | Notes |
| --- | --- | --- |
| Compact/big conversions, `CalcWork`, PoW checks, ASERT, merkle roots, inclusion proof, tspend window | `dcroxide:crates/dcroxide-standalone/src/pow.rs:33-221`, `merkle.rs:77-96`, `inclusionproof.rs:119`, `treasury.rs:74` | direct |
| EMA difficulty, ASERT-from-anchor, `estimate_supply`, `sum_purchased_tickets`, `calc_next_stake_diff_v2` | `dcroxide:crates/dcroxide-blockchain/src/difficulty.rs:161,368,747,777,807` | Written against a `ChainView::node(height)` trait (`difficulty.rs:59-90`). A wallet view that answers heights ≥ `chain[0]` from the slice and lower heights from the main-chain index reproduces §4.3/§4.5. The crate pulls in the full-node database; consider moving these functions to a lighter crate |
| `calc_want_height` | `dcroxide:crates/dcroxide-blockchain/src/stakever.rs:145` | direct |
| Positional check | **not** `dcroxide:crates/dcroxide-blockchain/src/validate.rs:1230` | dcrd's version enforces a confirmed anchor and needs threshold state; the wallet's variant (§4.2) must be written separately. `DiffNode` lacks the header version and hash that the wallet's candidate loop and ancestor test need |
| Cfilter matching, key | `dcroxide:crates/dcroxide-gcs/src/lib.rs:477-556`, `blockcf2.rs:14-81` | direct |
| Stake transaction classification and helpers | `dcroxide:crates/dcroxide-stake/src/lib.rs:74,392,433,808,1117,1209,1214`, `treasury.rs:290` | direct |
| Script classification, multisig sig-script helpers, engine, builder, addresses | `dcroxide:crates/dcroxide-txscript/src/stdscript.rs:82,484,506,1157`, `engine.rs`, `builder.rs`, `stdaddr.rs` | direct |
| Header hashing (blake256, blake3) | `dcroxide:crates/dcroxide-wire/src/blockheader.rs:150,164` | direct |
| Chain parameters and deployments | `dcroxide:crates/dcroxide-chaincfg` | check that `ForcedChoiceID` and testnet2's absence are represented |
| HD public child derivation | `dcr-rs:src/hd.rs:574` (`ExtPubKey::derive_child`), hardened account derivation `dcr-rs:src/hd.rs:280` (Decred variant, not strict BIP32) | invalid-child error mapping must match `hdkeychain.ErrInvalidChild` skipping; not verified |
| P2PKH script bytes | `dcr-rs:src/address.rs:152` | or `dcroxide-txscript` `stdaddr` |

No dcroxide crate offers an equivalent of dcrd `chaingen`; dcroxide's tests are vector driven, which is
the model to follow (§10).

### 11.3 Hazards

1. **Wrapping `u32` arithmetic is load-bearing**: `lastUsed + gapLimit`, `j + 256`, `lastUsed + 1 > other + 1`,
   `mid - 1` at zero, cursor adjustment and clamp. Use `wrapping_*` deliberately and test the sentinel
   cases.
2. **Nil versus empty** `RelevantTxs` (item 3 of §8) and nil versus `(nil, nil)` returns from
   `rescanPoint`/`ancestorHeaderAtHeight`: model with `Option`.
3. **Euclidean division and arithmetic shift** in EMA when reimplementing rather than reusing.
4. **Do not "fix" acceptance rules silently**: Q-1/Q-2/Q-29 define which headers and blocks the Go wallet
   accepts.
5. **Panics**: replace Q-7 and the forest consistency panic with errors that disconnect the peer; never
   abort the process on peer-supplied data.
6. **Side effects inside transactions** (Q-9): decide whether to keep Go's ordering (network call inside
   the write transaction) or buffer the effects until commit; the latter changes failure behaviour
   only in error paths.
7. **Unbounded concurrency** in the exists-index discovery (`2 * accounts` tasks, `2 * accountGapLimit`
   RPC calls per bisect step): keep request contents and order-independence, bound the parallelism.
8. **Map-order dependence** (Q-14, Q-19): pick a deterministic order and record the divergence.
9. **Blocking channel sends** in rescan progress: make them cancellable without changing the sequence of
   values a well-behaved consumer sees.
10. **`time.Now()`** inside record construction: inject a clock for tests.

### 11.4 Size estimate

**L.** About 4,100 Go lines. The numerically delicate parts (EMA, ASERT, stake difficulty, merkle, GCS)
already exist in dcroxide, and the forest and `validate` are small and pure. The cost is in
`ChainSwitch`/`processTransactionRecord`, which are tightly coupled to the transaction store, address
manager, notifications and backend with many ordered side effects; in discovery's concurrent bisects with
wrapping arithmetic and upstream bugs to reproduce or deliberately fix; and in the test effort, since
upstream has almost no tests for this scope and nearly everything in §10 must be generated through an
oracle.

---

## 12. Open questions

1. **Q-7 reachability.** Can an SPV peer deliver a header that reaches `isAncestorOf` with a parent below
   the cached anchor? Needs the p2p header sanity checks (SPV spec). Decide whether to report upstream.
2. **Q-11.** Reproduce the halving probe sequence (identical discovered-account set to the Go wallet on
   SPV) or implement a true bisect (finds more accounts; diverges)?
3. **Q-12.** Reproduce the child-1-first behaviour for freshly discovered accounts or initialise the
   buffer with the sentinel?
4. **Q-13.** Reproduce the early return or fix it?
5. **Q-21.** Reproduce the stalled marker after a reorg back onto a transaction-less block, or treat
   re-attached blocks as scanned?
6. **Q-14 / Q-19.** Which deterministic order replaces Go map iteration for treasury votes (suggest
   ascending tspend hash) and for equal-work tips?
7. **Q-17.** Keep the mismatched vote log pairing or log the correct ticket?
8. **Q-9.** Keep network calls and in-memory mutations inside the write transaction?
9. **Hash verification.** Should the Rust `BlockNode` constructor recompute the header hash (stricter than
   Go, Q-2)? This changes nothing for honest peers.
10. **`SetBirthStateAndScan`.** Port for library API parity or drop (no in-repository caller other than
    its test)? UNVERIFIED
    whether external embedders call it.
11. **Testnet2.** The hard-coded network value `0x48e7a065` appears in `DCP0001.Active` and `voteVersion`;
    no chain parameters for it exist in the pinned chaincfg. Keep the dead branches?
12. **Receive-time semantics on rescan/reorg (Q-32)** depend on `udb.InsertMinedTx`; confirm against the
    transaction store spec.
13. **Regnet.** `validate.PreDCP0005CFilterHash`, `DCP0001.Active` and `voteVersion` have no regnet case
    (stake difficulty is never validated there, vote version 1). UNVERIFIED whether the wallet can be
    started on regnet at all; settle from the configuration spec.
14. **`dcr-rs` child derivation parity** with `hdkeychain` v3.1.3 for invalid children (error type and
    skip behaviour) has not been verified by test.
15. **Q-50.** Reproduce the gRPC `CreateWallet` birthday mix-up (byte-identical `birthstate` to the Go
    wallet for the same request) or honour `set_birth_height`/`birth_time` as the proto intends? Decide
    together with the gRPC spec.
16. **Q-52.** Reproduce the `--watchlast` wrap (unused branches unwatched, so payments to their first
    addresses are missed until the next reload) or fix it? A fix only adds watched addresses.
17. **Q-51 / Q-53.** Keep the hard failure on an `account-<n>` name collision, and is the leaked
    worker set worth reproducing (it is not observable outside the process)?
