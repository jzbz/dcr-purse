# 07 — Wallet core, part 2: queries, listings, key import/export, stake info, outpoint locks

## 1. Header

**Scope**

| File | Lines in scope | Total lines |
|---|---|---|
| `wallet/wallet.go` | 2394–4428 (2,035 lines) | 5,828 |

Every declaration whose first line falls in 2394–4428 is specified here. Lines
4426–4428 are the doc comment of `ticketChangeMatured`, whose declaration starts
at line 4429 and therefore belongs to part 3.

Files read in full or in the relevant part to resolve calls made from the scope
(they are owned by sibling specs and are cited here only where the behaviour
is visible through a function in scope):

| File | Lines | Why it was read |
|---|---|---|
| `wallet/udb/txquery.go` | 773 (all) | `TxDetails`, `TicketDetails`, `Tx`, `RangeTransactions`, `Spender` |
| `wallet/udb/txdb.go` | 190–700, 960–1000, 1115–1135, 1400–1415, 1690–1745 | bucket names, key layouts, block iterator |
| `wallet/udb/txmined.go` | 36–215, 540–600, 880–960, 2236–2362, 2461–2553, 2731–2849 | `Credit`, `MainChainTip`, header accessors, `UnspentOutputs`, `UnspentOutput`, `TicketIterator` |
| `wallet/udb/txunmined.go` | 270–330 | `UnminedTxs` |
| `wallet/udb/kahnsort.go` | 116 (all) | dependency sort used by `UnminedTxs` |
| `wallet/udb/addressmanager.go` | 25–125, 200–240, 540–600, 960–1060, 1220–1480, 2060–2080, 2170–2320, 2395–2580, 2795–2882 | account/address lookups and imports |
| `wallet/udb/addressdb.go` | 265–290, 690–780, 1390–1480 | key encodings, iteration order |
| `wallet/notifications.go` | 50–182, 380–475, 590–625 | `makeTxSummary`, `TransactionSummary`, `Block`, `notifyAccountProperties` |
| `wallet/addresses.go` | 440–460, 1000–1038 | `addressBuffer`, `deriveBranches`, `deriveChildAddresses` |
| `wallet/network.go` | 142 (all) | `NetworkBackend` |
| `wallet/ticketstatus_string.go` | 32 (all) | `TicketStatus.String` |
| `errors/errors.go` | 400 (all) | error construction and formatting |
| `deployments/deployments.go` | 1–75 | `DCP0001.Active` |
| `internal/compat/compat.go` | 27 (all) | `IsEitherCoinBaseTx` |
| `rpc/jsonrpc/types/results.go` | 195–280 | JSON result structs |
| `rpc/client/dcrd/calls.go` | 145–165 | `MempoolCount` |
| `wallet/walletdb/interface.go` | 203–255 | `View`, `Update` error propagation |
| `chain/backend.go` | 48–50 | RPC-syncer `LoadTxFilter` |
| `spv/backend.go` | 417–433 | SPV `LoadTxFilter` |
| `rpc/client/dcrd/calls.go` | 59–65, 411–433 | `loadtxfilter` request shape |
| `dcrd/txscript/v4@v4.1.2/stdscript/address.go`, `addressv0.go` | all | `ExtractAddrs` classification order (§5.21) |
| `dcrd/txscript/v4@v4.1.2/stdscript/scriptv0.go` | 14–66, 453–480 | P2PK and null-data recognisers |
| `dcrd/txscript/v4@v4.1.2/stdaddr/addressv0.go` | 185–235 | pubkey-address string form and pubkey-hash conversion |
| `internal/rpc/jsonrpc/methods.go` | selected handlers | how results reach JSON-RPC clients |
| `internal/rpc/rpcserver/server.go` | selected handlers | how results reach gRPC clients |

**Parity pin**

- dcrwallet `54b85b33` (2.2.0-pre, module `decred.org/dcrwallet/v5`, `go 1.26.0` in `go.mod`).
- Dependencies read at the versions pinned in `go.mod`:
  `dcrd/blockchain/stake/v5@v5.0.3`, `dcrd/blockchain/standalone/v2@v2.3.0`,
  `dcrd/chaincfg/v3@v3.3.0`, `dcrd/dcrutil/v4@v4.0.3`, `dcrd/dcrec@v1.0.1`,
  `dcrd/hdkeychain/v3@v3.1.3`, `dcrd/txscript/v4@v4.1.2` (`stdscript`, `stdaddr`),
  `dcrd/wire@v1.8.0`, `dcrd/rpc/jsonrpc/types/v4@v4.4.0`.
  `bbolt@v1.3.12` was consulted only for the mmap lock sites in `db.go`
  (Q-16); its cursor semantics and `golang.org/x/sync@v0.23.0` (`errgroup`)
  are relied on but were not re-read, and are stated only where the calling
  Go makes them evident.

**Summary**

This range of `wallet.go` is the wallet's read-side query surface plus a small
set of mutating imports. It contains: the `listtransactions`-family result
builders (`listTransactions`, `ListSinceBlock`, `ListTransactions`,
`ListAddressTransactions`, `ListAllTransactions`, `ListTransactionDetails`);
block and transaction lookups (`BlockInfo`, `TransactionSummary`,
`GetTransactions`, `Spender`, `UnspentOutput`); ticket queries
(`GetTicketInfo[Precise]`, `GetTickets[Precise]`, `TicketStatus`); account and
UTXO listings (`Accounts`, `ListUnspent`, `SortedActivePaymentAddresses`,
`UnminedTransactions`); key export and imports (`LoadPrivateKey`,
`DumpWIFPrivateKey`, `ImportPrivateKey`, `ImportPublicKey`, `ImportScript`,
`VotingXprivFromSeed`, `ImportVotingAccount`, `ImportXpubAccount`); staking
statistics (`StakeInfo`, `StakeInfoPrecise`); the in-memory locked-outpoint
set; and the confirmation/maturity helpers `confirmed`, `confirms`,
`coinbaseMatured`.

Nothing in this range defines a persisted format. What it does define, and what
JSON-RPC and gRPC clients observe directly, is **result ordering, filtering and
field population**. Those rules are the core of this spec.

The assignment description mentions "signing of transactions and messages,
publishing". None of that is in this line range: `SignHashes`/`SignMessage`/
`VerifyMessage` are at `wallet/wallet.go:1977–2060` (part 1) and
`SignTransaction`/`PublishTransaction`/`PublishUnminedTransactions` are at
`wallet/wallet.go:4773–5185` (part 3).

---

## 2. Role in the system

**What it does.** Translates wallet database state (transaction store `wtxmgr`
and address manager `waddrmgr`) into the result shapes the RPC servers return,
and performs the wallet-level orchestration around key/script/account imports
(database write, network transaction-filter update, log line, notification,
in-memory address-buffer registration).

**Who calls it.**

| Caller | Functions used |
|---|---|
| JSON-RPC server (`internal/rpc/jsonrpc/methods.go`) | `ListSinceBlock` (listsinceblock), `ListTransactions` (listtransactions), `ListAddressTransactions` (listaddresstransactions), `ListAllTransactions` (listalltransactions), `ListTransactionDetails` (gettransaction), `BlockInfo` (several), `GetTransactions` (auditreuse), `GetTickets` (ticketinfo), `UnspentOutput` (gettxout in SPV mode), `Accounts` (lockaccount `*`), `ListUnspent` (listunspent), `LoadPrivateKey` (sendfromtreasury, `methods.go:3676`), `DumpWIFPrivateKey` (dumpprivkey), `ImportPrivateKey` (importprivkey), `ImportPublicKey` (importpubkey), `ImportScript` (importscript; addmultisigaddress at `methods.go:451`, which ignores `Exist`), `ImportXpubAccount` (importxpub), `StakeInfo`/`StakeInfoPrecise` (getstakeinfo), `LockOutpoint`/`UnlockOutpoint`/`ResetLockedOutpoints` (lockunspent), `LockedOutpoints` (listlockunspent), `SortedActivePaymentAddresses` (listreceivedbyaddress) |
| gRPC server (`internal/rpc/rpcserver/server.go`) | `Accounts`, `BlockInfo`, `TransactionSummary` (GetTransaction), `GetTransactions`, `GetTicketInfo[Precise]` (GetTicket), `GetTickets[Precise]`, `Spender`, `DumpWIFPrivateKey`, `ImportPrivateKey`, `ImportXpubAccount`, `ImportScript`, `VotingXprivFromSeed` + `ImportVotingAccount` (ImportVotingAccountFromSeed), `StakeInfo[Precise]` |
| Wallet package itself | `GetTickets` (`wallet/wallet.go:5741`), `UnminedTransactions` (`wallet/wallet.go:5174`, `PublishUnminedTransactions`), `LockOutpoint`/`UnlockOutpoint` (`wallet/createtx.go:1310–1644`, `wallet/wallet.go:4685`; `wallet/feepayment.go:478` only mentions `UnlockOutpoint` in a comment and does not call it), `LockedOutpoint` (`ListUnspent`), `Spender` and `DumpWIFPrivateKey` (`wallet/vspticket.go:238`, `:75`), `confirmed` (`wallet/createtx.go:1761`, `:1850`), `hasVotingAuthority` (`wallet/chainntfns.go:759`), `isVote`/`isTreasurySpend` (`wallet/chainntfns.go:336`, `:353`), `isRevocation` (`wallet/notifications.go:437`) |
| SPV syncer (`spv/sync.go:1930`) | `UnminedTransactions` |
| Wallet setup (`walletsetup.go:241`) | `ImportXpubAccount` |

Further call sites found by a mechanical search of the repository (non-test,
non-generated Go):

| Symbol in scope | Additional callers | Note |
|---|---|---|
| `blockRange` | `RangeCFiltersV2` (`wallet/wallet.go:859`, part 1) | Same `(0, -1)` defaults and hash-to-height rule apply to compact-filter ranging |
| `confirms` | `wallet/wallet.go:2311` (part 1, transaction confirmations), `:4509` (part 3, `TotalReceivedForAccounts`), `wallet/notifications.go:853`, `:949` (confirmation notifications) | The JSON-RPC server does not call this one; it has its own byte-identical copy (`internal/rpc/jsonrpc/methods.go:75–82`) |
| `coinbaseMatured` | `recvCategory` (`wallet/wallet.go:2382`, part 1), input selection in `wallet/createtx.go:1778–1795`, `:1872–1889` | |
| `NewBlockIdentifierFromHeight` | `wallet/wallet.go:5238`, `:5240` (pre-DCP0005 cfilter import, part 3), `:5738–5739` (part 3, the `GetTickets` call at `:5741`); JSON-RPC `methods.go:1213`, `:1274`, `:1399`, `:3155`, `:4416–4417`; gRPC `server.go:461`, `:843`, `:1032`, `:1366`, `:1382`, `:3815` | |
| `NewBlockIdentifierFromHash` | JSON-RPC `methods.go:1738`; gRPC `server.go:459`, `:1030`, `:1364`, `:1380` | |
| `ImportPrivateKey` | gRPC `ImportPrivateKey` handler (`server.go:650`, call at `:694`) | Called after the handler has required a network backend (`:689–692`); the returned address string is discarded |
| `Accounts` | JSON-RPC `lockaccount` with account `*` (`methods.go:5793`), which then locks every account having `AccountEncrypted && AccountUnlocked` in the returned order | |
| `LockedOutpoints` | JSON-RPC `listlockunspent` (`methods.go:2986`) passes the optional account argument, `""` when omitted | |

**Precise versus non-precise selection.** The wallet package never chooses
between `GetTicketInfo`/`GetTicketInfoPrecise`, `GetTickets`/
`GetTicketsPrecise` or `StakeInfo`/`StakeInfoPrecise`; the servers do, with
one rule: the precise variant is used if and only if the wallet's current
network backend is a `*chain.Syncer` (dcrd RPC sync), passing
`chainSyncer.RPC()`; any other backend (SPV syncer) or no backend selects the
non-precise variant (`internal/rpc/rpcserver/server.go:906–914`,
`:1462–1472`, `:1537–1543`; `internal/rpc/jsonrpc/methods.go:2549–2556`).
JSON-RPC `ticketinfo` is the exception: it always calls `GetTickets`
(`methods.go:4419`). No caller passes a nil `rpc` to a precise variant.

`rpc/client/dcrwallet/methods.go` contains same-named methods
(`ListTransactions`, `ListUnspent`, `GetTickets`, `ImportScript`, …); those
are JSON-RPC *client* wrappers and do not call into this scope.

**What it calls.**

| Callee | Owner |
|---|---|
| `walletdb.View` / `walletdb.Update` | database layer |
| `udb.Store` (`w.txStore`): `MainChainTip`, `RangeTransactions`, `TxDetails`, `Tx`, `TicketDetails`, `GetMainChainBlockHashForHeight`, `GetSerializedBlockHeader`, `GetBlockHeader`, `BlockInMainChain`, `Spender`, `UnspentOutput`, `UnspentOutputs`, `IterateTickets`, `UnminedTxs` | transaction store spec |
| `udb.Manager` (`w.manager`): `AddrAccount`, `AccountName`, `ForEachAccount`, `AccountProperties`, `Address`, `RedeemScript`, `PrivateKey`, `HavePrivateKey`, `ExistsHash160`, `ImportPrivateKey`, `ImportPublicKey`, `ImportScript`, `ImportVotingAccount`, `ImportXpubAccount`, `LookupAccount`, `ForEachActiveAddress`; package function `udb.HDKeysFromSeed` | address manager spec |
| `makeTxSummary`, `NtfnServer.notifyAccountProperties` | notifications spec (`wallet/notifications.go`) |
| `w.NetworkBackend()`, `NetworkBackend.LoadTxFilter` | network backend (`wallet/network.go`), chain/SPV syncers |
| `deriveBranches`, `deriveChildAddresses`, `addressBuffer`, `bip0044AccountData` | address buffers (`wallet/addresses.go`) |
| `w.nextRequiredDCP0001PoSDifficulty` | `wallet/difficulty.go` |
| `deployments.DCP0001.Active` | `deployments/deployments.go` |
| `dcrd.RPC.MempoolCount` | dcrd RPC client (`rpc/client/dcrd/calls.go:153`) |
| `ticketMatured`, `ticketChangeMatured`, `ticketExpired` | part 3 (`wallet/wallet.go:4429–4447`) |
| `stdscript.ExtractAddrs`, `stdscript.ExtractPubKeyHashV0`, `stake.IsSSGen/IsSSRtx/IsTSpend`, `compat.IsEitherCoinBaseTx`, `dcrutil.NewWIF`, `hdkeychain` | dcrd libraries |

---

## 3. Public surface inventory

Receiver `w` is `*Wallet`. "View" means the body runs inside one
`walletdb.View` read transaction; "Update" means one `walletdb.Update`
read-write transaction.

### 3.1 Transaction listing (JSON result builders)

| Symbol | Signature | Contract |
|---|---|---|
| `listTransactions` (unexported) | `func listTransactions(tx walletdb.ReadTx, details *udb.TxDetails, addrMgr *udb.Manager, syncHeight int32, net *chaincfg.Params) (sends, receives []types.ListTransactionsResult)` | Builds the per-output `listtransactions` entries for one transaction. Never fails. §5.1. `wallet/wallet.go:2394` |
| `ListSinceBlock` | `func (w *Wallet) ListSinceBlock(ctx, start, end, syncHeight int32) ([]types.ListTransactionsResult, error)` | View. Entries for every transaction returned by `RangeTransactions(start, end)`; per transaction receives then sends. Confirmations are computed against the caller-supplied `syncHeight`. Result is never nil. `:2525` |
| `ListTransactions` | `func (w *Wallet) ListTransactions(ctx, from, count int) ([]types.ListTransactionsResult, error)` | View. Skips the `from` newest transactions, then returns entries for up to `count` further transactions, in oldest-to-newest order. §5.3. `:2552` |
| `ListAddressTransactions` | `func (w *Wallet) ListAddressTransactions(ctx, pkHashes map[string]struct{}) ([]types.ListTransactionsResult, error)` | View. Entries for every transaction having at least one credit that pays a version-0 plain P2PKH script whose 20-byte hash is a key of `pkHashes`. §5.4. `:2612` |
| `ListAllTransactions` | `func (w *Wallet) ListAllTransactions(ctx) ([]types.ListTransactionsResult, error)` | View. `ListTransactions` without paging. §5.5. `:2656` |
| `ListTransactionDetails` | `func (w *Wallet) ListTransactionDetails(ctx, txHash *chainhash.Hash) ([]types.ListTransactionsResult, error)` | View. Entries for one transaction, receives then sends. `NotExist` if unknown. `:2697` |

### 3.2 Blocks and transactions

| Symbol | Signature | Contract |
|---|---|---|
| `BlockIdentifier` | `struct { height int32; hash *chainhash.Hash }` (fields unexported) | A block named by hash (when `hash != nil`) or else by main-chain height. `:2725` |
| `NewBlockIdentifierFromHeight` | `func(height int32) *BlockIdentifier` | `hash` nil. `:2731` |
| `NewBlockIdentifierFromHash` | `func(hash *chainhash.Hash) *BlockIdentifier` | `height` 0; the pointer is stored, not copied. `:2736` |
| `BlockInfo` (type) | `struct { Hash chainhash.Hash; Height int32; Confirmations int32; Header []byte; Timestamp int64; StakeInvalidated bool }` | `Header` is the 180-byte serialized header. `:2742` |
| `(*Wallet).BlockInfo` | `func (w *Wallet) BlockInfo(ctx, blockID *BlockIdentifier) (*BlockInfo, error)` | View. §5.6. `:2752` |
| `TransactionSummary` (method) | `func (w *Wallet) TransactionSummary(ctx, txHash *chainhash.Hash) (txSummary *TransactionSummary, confs int32, blockHash *chainhash.Hash, err error)` | View. Summary via `makeTxSummary`, confirmation count, and block hash (non-nil only when `confs > 0`). `:2795` |
| `GetTransactions` | `func (w *Wallet) GetTransactions(ctx, f func(*Block) (bool, error), startBlock, endBlock *BlockIdentifier) error` | View. Calls `f` once per block that has wallet transactions, and once for all unmined transactions. §5.8. `:3248` |
| `blockRange` (unexported) | `func (w *Wallet) blockRange(dbtx walletdb.ReadTx, startBlock, endBlock *BlockIdentifier) (int32, int32, error)` | Maps two optional identifiers to `(start, end)` heights; defaults `(0, -1)`. §5.7. `:3040` |
| `Spender` | `func (w *Wallet) Spender(ctx, out *wire.OutPoint) (*wire.MsgTx, uint32, error)` | View. Thin wrapper over `txStore.Spender`; the error is returned unwrapped. `:3308` |
| `UnspentOutput` | `func (w *Wallet) UnspentOutput(ctx, op wire.OutPoint, includeMempool bool) (*udb.Credit, error)` | View. Thin wrapper over `txStore.UnspentOutput`; error returned unwrapped. `:3323` |
| `UnminedTransactions` | `func (w *Wallet) UnminedTransactions(ctx) ([]*wire.MsgTx, error)` | View. All unmined, published transactions in dependency order. §5.16. `:4361` |

### 3.3 Tickets

| Symbol | Signature | Contract |
|---|---|---|
| `TicketSummary` | `struct { Ticket *TransactionSummary; Spender *TransactionSummary; Status TicketStatus }` | `Spender` nil when no spender is recorded. `:2840` |
| `TicketStatus` | `type TicketStatus uint` with constants below | `String()` is generated by stringer with `-linecomment`. `:2847` |
| `fetchTicketDetails` (unexported) | `func (w *Wallet) fetchTicketDetails(ns walletdb.ReadBucket, hash *chainhash.Hash) (*udb.TicketDetails, error)` | Looks up a transaction and requires `TxType == TxTypeSStx`. `:2820` |
| `makeTicketSummary` (unexported) | `func makeTicketSummary(rpc *dcrd.RPC, dbtx walletdb.ReadTx, w *Wallet, details *udb.TicketDetails) *TicketSummary` | Status state machine, §5.9. `rpc` is only tested for nil. `:2888` |
| `GetTicketInfoPrecise` | `func (w *Wallet) GetTicketInfoPrecise(ctx, rpc *dcrd.RPC, hash *chainhash.Hash) (*TicketSummary, *wire.BlockHeader, error)` | View. Summary and the ticket's block header (nil if unmined). Passes `rpc` to `makeTicketSummary`. `:2947` |
| `GetTicketInfo` | `func (w *Wallet) GetTicketInfo(ctx, hash *chainhash.Hash) (*TicketSummary, *wire.BlockHeader, error)` | View. Same with `rpc == nil`. `:2995` |
| `GetTicketsPrecise` | `func (w *Wallet) GetTicketsPrecise(ctx, rpc *dcrd.RPC, f func([]*TicketSummary, *wire.BlockHeader) (bool, error), startBlock, endBlock *BlockIdentifier) error` | View. Calls `f` once per block containing at least one ticket. §5.10. `:3102` |
| `GetTickets` | `func (w *Wallet) GetTickets(ctx, f func([]*TicketSummary, *wire.BlockHeader) (bool, error), startBlock, endBlock *BlockIdentifier) error` | View. Same with `rpc == nil`. `:3176` |

`TicketStatus` constants (`wallet/wallet.go:2851–2886`, `wallet/ticketstatus_string.go:22–32`):

| Constant | Value | `String()` |
|---|---|---|
| `TicketStatusUnknown` | 0 | `unknown` |
| `TicketStatusUnmined` | 1 | `unmined` |
| `TicketStatusImmature` | 2 | `immature` |
| `TicketStatusLive` | 3 | `live` |
| `TicketStatusVoted` | 4 | `voted` |
| `TicketStatusMissed` | 5 | `missed` |
| `TicketStatusExpired` | 6 | `expired` |
| `TicketStatusUnspent` | 7 | `unspent` |
| `TicketStatusRevoked` | 8 | `revoked` (deprecated; never produced by code in scope) |

Any other value formats as `TicketStatus(<decimal>)`.

### 3.4 Accounts, UTXOs, addresses

| Symbol | Signature | Contract |
|---|---|---|
| `AccountProperties` | `struct { AccountNumber uint32; AccountName string; AccountType uint8; LastUsedExternalIndex, LastUsedInternalIndex, LastReturnedExternalIndex, LastReturnedInternalIndex, ImportedKeyCount uint32; AccountEncrypted, AccountUnlocked bool }` | Field-for-field identical to `udb.AccountProperties` (an alias of an anonymous struct, `wallet/udb/addressmanager.go:213`). Unused index = `0xFFFFFFFF`. `AccountType` is the database account-row type: `0` legacy BIP0044, `1` BIP0044, `2` imported voting (`wallet/udb/addressdb.go:49–54`); neither RPC server exposes it. `:3338` |
| `AccountResult` | `struct { AccountProperties; TotalBalance dcrutil.Amount }` | `:3352` |
| `AccountsResult` | `struct { Accounts []AccountResult; CurrentBlockHash *chainhash.Hash; CurrentBlockHeight int32 }` | `:3359` |
| `Accounts` | `func (w *Wallet) Accounts(ctx) (*AccountsResult, error)` | View under `lockedOutpointMu`. All accounts with a total balance. §5.11. `:3371` |
| `creditSlice` (unexported) | `type creditSlice []*udb.Credit` with `Len`, `Less`, `Swap` | `sort.Interface` ordering used by `ListUnspent`. §5.12. `:3440` |
| `ListUnspent` | `func (w *Wallet) ListUnspent(ctx, minconf, maxconf int32, addresses map[string]struct{}, accountName string) ([]*types.ListUnspentResult, error)` | View. §5.12. Returns a nil slice when nothing matches. `:3477` |
| `SortedActivePaymentAddresses` | `func (w *Wallet) SortedActivePaymentAddresses(ctx) ([]string, error)` | View. Every address row of the address manager, encoded, sorted bytewise ascending. Nil slice when empty. `:4384` |

### 3.5 Keys, scripts, account imports

| Symbol | Signature | Contract |
|---|---|---|
| `LoadPrivateKey` | `func (w *Wallet) LoadPrivateKey(ctx, addr stdaddr.Address) (key *secp256k1.PrivateKey, zero func(), err error)` | View. `manager.PrivateKey`; `zero` is `key.Zero`. `:3669` |
| `DumpWIFPrivateKey` | `func (w *Wallet) DumpWIFPrivateKey(ctx, addr stdaddr.Address) (string, error)` | WIF string of the address's private key. §5.13. `:3688` |
| `ImportPrivateKey` | `func (w *Wallet) ImportPrivateKey(ctx, wif *dcrutil.WIF) (string, error)` | Update, then filter load, log, notification. Returns the P2PKH address string. §5.14. `:3705` |
| `ImportPublicKey` | `func (w *Wallet) ImportPublicKey(ctx, pubkey []byte) (string, error)` | Same flow for a compressed pubkey; watching-only wallets only. §5.14. `:3742` |
| `ImportScript` | `func (w *Wallet) ImportScript(ctx, rs []byte) error` | Update that also performs the filter load inside the transaction. §5.14. `:3780` |
| `VotingXprivFromSeed` | `func (w *Wallet) VotingXprivFromSeed(seed []byte) (*hdkeychain.ExtendedKey, error)` | `votingXprivFromSeed(seed, w.ChainParams())`. `:3808` |
| `votingXprivFromSeed` (unexported) | `func votingXprivFromSeed(seed []byte, params *chaincfg.Params) (*hdkeychain.ExtendedKey, error)` | Account-0 xpriv at `m/44'/<SLIP0044 coin type>'/0'`. §5.15. `:3814` |
| `ImportVotingAccount` | `func (w *Wallet) ImportVotingAccount(ctx, xpriv *hdkeychain.ExtendedKey, passphrase []byte, name string) (uint32, error)` | Update, filter load of external gap-limit addresses, address-buffer registration. Returns the account number. §5.15. `:3838` |
| `ImportXpubAccount` | `func (w *Wallet) ImportXpubAccount(ctx, name string, xpub *hdkeychain.ExtendedKey) error` | Filter load of external and internal gap-limit addresses, Update, address-buffer registration. §5.15. `:3893` |

### 3.6 Staking statistics

| Symbol | Signature | Contract |
|---|---|---|
| `StakeInfoData` | `struct { BlockHeight int64; TotalSubsidy, Sdiff dcrutil.Amount; OwnMempoolTix, Unspent, Voted, Revoked, UnspentExpired uint32; PoolSize, AllMempoolTix, Immature, Live, Missed, Expired uint32 }` | Field order as written at `:3954–3971`. |
| `isVote` / `isRevocation` / `isTreasurySpend` (unexported) | `func(tx *wire.MsgTx) bool` | `stake.IsSSGen`, `stake.IsSSRtx`, `stake.IsTSpend`. `:3973–3983` |
| `hasVotingAuthority` (unexported) | `func (w *Wallet) hasVotingAuthority(addrmgrNs walletdb.ReadBucket, ticketPurchase *wire.MsgTx) (mine, havePrivKey bool, err error)` | Whether output 0 of a ticket pays an address known to the address manager, and whether its private key is held. §5.17. `:3987` |
| `StakeInfo` | `func (w *Wallet) StakeInfo(ctx) (*StakeInfoData, error)` | View. Local-only statistics. §5.17. `:4008` |
| `StakeInfoPrecise` | `func (w *Wallet) StakeInfoPrecise(ctx, rpc *dcrd.RPC) (*StakeInfoData, error)` | View plus one concurrent dcrd RPC. §5.17. `:4115` |

### 3.7 Locked outpoints

| Symbol | Signature | Contract |
|---|---|---|
| `LockedOutpoint` | `func (w *Wallet) LockedOutpoint(txHash *chainhash.Hash, index uint32) bool` | Membership test. `:4250` |
| `LockOutpoint` | `func (w *Wallet) LockOutpoint(txHash *chainhash.Hash, index uint32)` | Insert; no validation that the outpoint exists. `:4260` |
| `UnlockOutpoint` | `func (w *Wallet) UnlockOutpoint(txHash *chainhash.Hash, index uint32)` | Remove; no-op when absent. `:4269` |
| `ResetLockedOutpoints` | `func (w *Wallet) ResetLockedOutpoints()` | Replace the set with an empty one. `:4278` |
| `LockedOutpoints` | `func (w *Wallet) LockedOutpoints(ctx, accountName string) ([]dcrdtypes.TransactionInput, error)` | Snapshot of the set, filtered to outputs the wallet has a transaction for and optionally to one account. §5.18. `:4287` |

The set is `map[outpoint]struct{}` where `outpoint` is `struct { hash chainhash.Hash; index uint32 }` (`wallet/wallet.go:94`): the tree is not part of the key. It is memory-only and is empty after every process start.

### 3.8 Confirmation and maturity helpers (unexported, observable through every listing)

| Symbol | Definition | Citation |
|---|---|---|
| `confirms(txHeight, curHeight int32) int32` | `0` if `txHeight == -1` or `txHeight > curHeight`; otherwise `curHeight - txHeight + 1` | `:4411` |
| `confirmed(minconf, txHeight, curHeight int32) bool` | `confirms(txHeight, curHeight) >= minconf` | `:4404` |
| `coinbaseMatured(params, txHeight, curHeight int32) bool` | `txHeight >= 0 && curHeight - txHeight + 1 > int32(params.CoinbaseMaturity)` | `:4422` |

Cross-referenced from part 3 (used by code in scope):

| Symbol | Definition | Citation |
|---|---|---|
| `ticketChangeMatured` | `txHeight >= 0 && curHeight - txHeight + 1 > int32(params.SStxChangeMaturity)` | `:4429` |
| `ticketMatured` | `txHeight >= 0 && curHeight - txHeight > int32(params.TicketMaturity)` | `:4435` |
| `ticketExpired` | `txHeight >= 0 && curHeight - txHeight > int32(params.TicketMaturity) + int32(params.TicketExpiry)` | `:4444` |

All subtractions are Go `int32` and wrap on overflow; with realistic heights no wrap occurs.

### 3.9 Cross-references into the other parts of `wallet.go`

| Symbol | Location | Used by |
|---|---|---|
| `waddrmgrNamespaceKey` = `"waddrmgr"`, `wtxmgrNamespaceKey` = `"wtxmgr"` | `wallet/wallet.go:77–78` | every View/Update |
| `scriptVersionAssumed` = `0` | `wallet/wallet.go:83` | `ListAddressTransactions`, `Accounts`, `ListUnspent` |
| `outpoint` | `wallet/wallet.go:94` | locked-outpoint set |
| `Wallet` fields `db`, `txStore`, `manager`, `chainParams`, `gapLimit`, `lockedOutpoints`, `lockedOutpointMu`, `addressBuffers`, `addressBuffersMu`, `NtfnServer` | `wallet/wallet.go:102–180` | throughout |
| `CreditCategory`, `recvCategory` | `wallet/wallet.go:2349–2388` (part 1) | `listTransactions` |
| `ticketChangeMatured`, `ticketMatured`, `ticketExpired` | `wallet/wallet.go:4429–4447` (part 3) | `ListUnspent`, `makeTicketSummary`, `StakeInfo*` |
| `ChainParams()` | `wallet/wallet.go:5187` (part 3) | `VotingXprivFromSeed` |

`CreditCategory.String()` values (part 1, needed to read §5.1): `CreditReceive` → `"receive"`, `CreditGenerate` → `"generate"`, `CreditImmature` → `"immature"`. `recvCategory` returns `CreditGenerate` when `compat.IsEitherCoinBaseTx(tx)` and `coinbaseMatured(params, details.Block.Height, syncHeight)`, `CreditImmature` when it is a coinbase that is not matured, else `CreditReceive` (`wallet/wallet.go:2380–2388`).

---

## 4. Data formats

Nothing in scope writes database bytes directly. The formats below are the
ones the functions in scope *emit* (JSON structs handed to the RPC layer, WIF
text) and the database key orders they *depend on* for result ordering.

### 4.1 `types.ListTransactionsResult` (JSON object)

Defined at `rpc/jsonrpc/types/results.go:216–236`. Fields are marshalled in
declaration order by Go's `encoding/json`:

| # | JSON key | Go type | `omitempty` | Value written by `listTransactions` |
|---|---|---|---|---|
| 1 | `account` | string | no | `""` for `send`; account name (or `""`) for receive categories |
| 2 | `address` | string | yes | encoded address when the output script yields exactly one address; otherwise absent |
| 3 | `amount` | float64 | no | `+value/1e8` for receive categories; `-(value/1e8)` for `send` (negative zero for a zero-value output, see Q-3) |
| 4 | `blockhash` | string | yes | block hash (byte-reversed hex) when mined; absent when unmined |
| 5 | `blockindex` | *int64 | yes | never set (always absent) |
| 6 | `blocktime` | int64 | yes | block record Unix time when mined; absent when unmined (or 0) |
| 7 | `category` | string | no | `send`, `receive`, `generate` or `immature` |
| 8 | `confirmations` | int64 | no | `confirms(blockHeight, syncHeight)`; 0 when unmined |
| 9 | `fee` | *float64 | yes | present only on `send` entries: `(Σoutputs − Σdebits)/1e8` (≤ 0) when every input is a wallet debit, else `0` |
| 10 | `generated` | bool | yes | `true` when `IsEitherCoinBaseTx`; absent otherwise |
| 11 | `involveswatchonly` | bool | yes | never set (always absent) |
| 12 | `time` | int64 | no | transaction received time (Unix seconds) |
| 13 | `timereceived` | int64 | no | same value as `time` |
| 14 | `txid` | string | no | transaction hash (byte-reversed hex) |
| 15 | `txtype` | *ListTransactionsTxType (a named `string` type) | yes | always present: `regular`, `ticket`, `vote` or `revocation` |
| 16 | `vout` | uint32 | no | output index |
| 17 | `walletconflicts` | []string | no | always the empty array `[]` (never `null`) |
| 18 | `comment` | string | yes | never set |
| 19 | `otheraccount` | string | yes | never set |

`txtype` constants: `LTTTRegular = "regular"`, `LTTTTicket = "ticket"`,
`LTTTVote = "vote"`, `LTTTRevocation = "revocation"`
(`rpc/jsonrpc/types/results.go:201–213`).

### 4.2 `types.ListUnspentResult` (JSON object)

Defined at `rpc/jsonrpc/types/results.go:265–277`; field order:

| # | JSON key | Go type | `omitempty` | Value written by `ListUnspent` |
|---|---|---|---|---|
| 1 | `txid` | string | no | outpoint hash (byte-reversed hex) |
| 2 | `vout` | uint32 | no | outpoint index |
| 3 | `tree` | int8 | no | 0 regular, 1 stake (from the stored credit's opcode tag, `wallet/udb/txmined.go:2326–2329`) |
| 4 | `txtype` | int | no | `int(stake.TxType)` of the containing transaction: 0 regular, 1 SStx, 2 SSGen, 3 SSRtx, 4 TAdd, 5 TSpend, 6 treasurybase (`dcrd/blockchain/stake/v5@v5.0.3/staketx.go:31–37`) |
| 5 | `address` | string | no | first extracted address, or `""` |
| 6 | `account` | string | no | account name of the first extracted address, else the default account's current name |
| 7 | `scriptPubKey` | string | no | lowercase hex of the output script |
| 8 | `redeemScript` | string | yes | lowercase hex of the P2SH redeem script for plain `STScriptHash` outputs; absent otherwise |
| 9 | `amount` | float64 | no | `value/1e8` |
| 10 | `confirmations` | int64 | no | `confirms(height, tipHeight)` |
| 11 | `spendable` | bool | no | see §5.12 steps 7.9–7.10 |

### 4.3 `dcrdtypes.TransactionInput` (JSON object, `listlockunspent`)

Defined at `dcrd/rpc/jsonrpc/types/v4@v4.4.0/chainsvrcmds.go:116–121`; field order:

| # | JSON key | Go type | `omitempty` | Value written by `LockedOutpoints` |
|---|---|---|---|---|
| 1 | `amount` | float64 | yes | output value /1e8; **absent when the value is 0** |
| 2 | `txid` | string | no | outpoint hash (byte-reversed hex) |
| 3 | `vout` | uint32 | no | outpoint index |
| 4 | `tree` | int8 | no | 0 if the transaction's `TxType` is regular, else 1 |

### 4.4 Amount-to-float conversion and JSON number formatting

`dcrutil.Amount.ToCoin()` is `float64(atoms) / math.Pow10(8)` — a single IEEE-754
double division (`dcrd/dcrutil/v4@v4.0.3/amount.go:93–100`). The result is then
marshalled by Go's `encoding/json` float encoder: shortest round-trip decimal,
`'f'` format unless `|x| < 1e-6` or `|x| >= 1e21`, in which case `'e'` format
with a two-digit negative exponent collapsed to one digit (for example 1 atom
is `1e-08` from `strconv` and is emitted as `1e-8`). Negative zero is emitted as
`-0`. The JSON-RPC server spec owns the general rule; it is repeated here
because every amount, fee and balance in this scope goes through it.
The Go standard library source was not re-read for this spec; the rule is
corroborated by dcroxide's port of the encoder
(`dcroxide:crates/dcroxide-dcrjson/src/gojson.rs:233–270`, and the `-0`
assertion at `dcroxide:crates/dcroxide-dcrjson/tests/gojson_nonfinite.rs:73`).
A Go oracle test over `[-0.0, 1e-8, 9.9e-7, 1e-6, 0.1, 21e6]` through the real
`listtransactions` path should still be added.

### 4.5 WIF text (`DumpWIFPrivateKey`)

`dcrd/dcrutil/v4@v4.0.3/wif.go:164–180`:

| Offset | Size | Field |
|---|---|---|
| 0 | 2 | network private-key ID (`chaincfg.Params.PrivateKeyID`) |
| 2 | 1 | signature scheme; `DumpWIFPrivateKey` always writes `0x00` (`dcrec.STEcdsaSecp256k1`) |
| 3 | 32 | private key scalar, big-endian, zero-padded (`secp256k1.PrivateKey.Serialize()`) |
| 35 | 4 | checksum: first 4 bytes of `chainhash.HashB(bytes[0:35])` (single BLAKE-256) |

The 39 bytes are base58-encoded with the alphabet
`123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz`
(`base58@v1.0.6/alphabet.go:12`; the same as Bitcoin's) and no additional
check bytes. The
comment at `wif.go:94` says "double SHA256"; the code uses `chainhash.HashB`.

`PrivateKeyID` per network (`dcrd/chaincfg/v3@v3.3.0`): mainnet `22 de` (`Pm…`),
testnet3 `23 0e` (`Pt…`), simnet `23 07` (`Ps…`), regnet `22 fe` (`Pr…`).

### 4.6 Database key orders that determine result order

All integers in the `wtxmgr` namespace are big-endian
(`wallet/udb/txdb.go:123`); account numbers in `waddrmgr` are little-endian
(`wallet/udb/addressdb.go:269–273`). bbolt iterates keys in ascending bytewise
order.

| Namespace / bucket | Key | Iterated by | Resulting order |
|---|---|---|---|
| `wtxmgr` / `b` (blocks) | height, 4 bytes BE | `rangeBlockTransactions` | ascending or descending block height |
| `wtxmgr` / `b` value | `[0:32]` hash, `[32:40]` Unix time BE, `[40:42]` vote bits BE, `[42]` stake-invalid flag, `[43:47]` tx count BE, `[47:]` tx hashes ×32 (`wallet/udb/txdb.go:216–228`) | `rangeBlockTransactions` | transactions within a block in the order their hashes were appended to the block record |
| `wtxmgr` / `m` (unmined) | tx hash, 32 bytes (internal byte order) | `rangeUnminedTransactions`, `unminedTxRecords` | ascending by raw hash bytes (not by the reversed-hex display string) |
| `wtxmgr` / `c` (credits) | tx hash ‖ height BE ‖ block hash ‖ output index BE | credit iterator | credits of one transaction ascending by output index |
| `wtxmgr` / `u` (unspent) | tx hash ‖ output index BE (36 bytes) | `UnspentOutputs` | ascending by raw hash bytes, then index |
| `wtxmgr` / `mc` (unmined credits) | tx hash ‖ output index BE | `UnspentOutputs` | same, after all mined unspent |
| `wtxmgr` / `tix` (tickets) | ticket hash, 32 bytes | `IterateTickets` | ascending by raw hash bytes |
| `waddrmgr` / `acct` | account number, 4 bytes **LE** | `ForEachAccount` | ascending by the LE byte string — **not** numeric order (see Q-1) |
| `waddrmgr` / `addr` | `SHA-256(hash160)` | `ForEachActiveAddress` | pseudo-random; `SortedActivePaymentAddresses` re-sorts |

Header field offsets used through `udb.ExtractBlockHeaderHeight` /
`ExtractBlockHeaderTime` (`wallet/udb/txmined.go:568–593`): height is the
little-endian `uint32` at byte 128 of the 180-byte serialized header, timestamp
the little-endian `uint32` at byte 136. `PoolSize` (used by `StakeInfo*`) is
the little-endian `uint32` at byte 112.

---

## 5. Algorithms and state machines

Conventions: "tip height" is the second return value of
`txStore.MainChainTip(dbtx)` — the height field of the header whose hash is
stored under root key `tip` in `wtxmgr` (`wallet/udb/txmined.go:183–193`).
Height `-1` means unmined. Unless stated otherwise each function opens exactly
one database transaction and every step runs inside it.

### 5.1 `listTransactions` — per-transaction entry builder

`wallet/wallet.go:2394–2520`. Inputs: an open read transaction, one
`udb.TxDetails`, the address manager, `syncHeight`, chain params. Output:
`(sends, receives)`; either may be a nil slice. No error path.

1. `addrmgrNs` := bucket `waddrmgr`.
2. If `details.Block.Height != -1`: `blockHashStr` := block hash string,
   `blockTime` := `details.Block.Time.Unix()`, `confirmations` :=
   `int64(confirms(details.Block.Height, syncHeight))`. Otherwise `""`, `0`, `0`.
3. `txHashStr` := hash string; `received` := `details.Received.Unix()`;
   `generated` := `compat.IsEitherCoinBaseTx(&details.MsgTx)`; `recvCat` :=
   `recvCategory(details, syncHeight, net).String()`.
4. `send` := `len(details.Debits) != 0`.
5. `txTypeStr` := `"ticket"` if `details.TxType == TxTypeSStx`, `"vote"` if
   `TxTypeSSGen`, `"revocation"` if `TxTypeSSRtx`, otherwise `"regular"`
   (TAdd, TSpend and treasurybase are `"regular"`).
6. Fee: `feeF64` := `0.0`. If `len(details.Debits) == len(details.MsgTx.TxIn)`:
   `feeF64` := `(Σ TxOut[i].Value − Σ Debits[j].Amount).ToCoin()` — i.e. the
   **negated** fee.
7. For each output index `i` in ascending order:
   1. Scan `details.Credits` in slice order for the first record with
      `Index == i`. If found and `Change` is true, skip this output entirely
      (no send entry, no receive entry). If found and not change,
      `isCredit` := true.
   2. `(_, addrs)` := `stdscript.ExtractAddrs(output.Version, output.PkScript, net)`.
      If `len(addrs) == 1`: `address` := `addrs[0].String()`; then
      `account, err := addrMgr.AddrAccount(addrmgrNs, addrs[0])`; if `err == nil`,
      `accountName, err = addrMgr.AccountName(addrmgrNs, account)`; if that
      errors, `accountName` := `""`. With zero or more than one address,
      `address` and `accountName` stay `""`.
   3. `amountF64` := `dcrutil.Amount(output.Value).ToCoin()`.
   4. Build the base entry: `Address`, `Vout = i`, `Confirmations`,
      `Generated`, `BlockHash`, `BlockTime`, `TxID`, `WalletConflicts = []`
      (empty, non-nil), `Time = received`, `TimeReceived = received`,
      `TxType = &txTypeStr`.
   5. If `send`: copy the entry with `Category = "send"`,
      `Amount = -amountF64`, `Fee = &feeF64`; append to `sends`. `Account`
      stays `""`.
   6. If `isCredit`: copy the entry with `Account = accountName`,
      `Category = recvCat`, `Amount = amountF64`, `Fee = nil`; append to
      `receives`.

Consequences an implementer must preserve:

- An output appears 0, 1 or 2 times: once as `send` if the transaction has any
  wallet debit (regardless of who owns the output), once as a receive category
  if it is a non-change wallet credit. Change credits never appear.
- `send` entries always carry `"fee"` (possibly `0`), receive entries never do.
- Both lists are in ascending `vout` order.
- `IsEitherCoinBaseTx` is true exactly when the transaction has one input
  whose previous outpoint has index `0xFFFFFFFF` and an all-zero hash
  (`internal/compat/compat.go:19–27`,
  `dcrd/blockchain/standalone/v2@v2.3.0/tx.go:49–101`: the `isTreasuryEnabled
  == false` variant has no version or treasury-spend exclusion, so it subsumes
  the `true` variant). Treasurybases and treasury spends therefore count as
  coinbases here (Q-5).

### 5.2 `RangeTransactions` iteration contract and `ListSinceBlock`

`RangeTransactions(ctx, ns, begin, end, f)` is owned by the transaction store
spec (`wallet/udb/txquery.go:350–538`). The properties this scope depends on:

1. If `begin < 0`: run the **unmined pass**. If it errors or `f` returned
   `true`, return.
2. **Block pass.** `b` := `begin`, or `0x7FFFFFFF` if `begin < 0`; `e` := `end`,
   or `0x7FFFFFFF` if `end < 0`.
   - If `b < e`: forward. Start at the first block record with height ≥ `b`;
     continue ascending while height ≤ `e`.
   - Otherwise (including `b == e`): backward. Start at the block record with
     height `b` if it exists, else at the greatest height below `b`
     (`wallet/udb/txdb.go:396–433`); continue descending while height ≥ `e`.
   - Before each block: if `ctx.Err() != nil`, return it.
   - For the block, build a `TxDetails` for each transaction hash in the block
     record's stored order. A block record with no transactions is skipped
     without calling `f`. Otherwise call `f(details)`; stop on `true` or error.
3. If no error, `f` did not stop, the unmined pass has not run, and `end < 0`:
   run the unmined pass.

**Unmined pass**: iterate bucket `m` in ascending raw-hash order (checking
`ctx.Err()` per record), build `TxDetails` for every record — unpublished
transactions included — and, if at least one exists, call `f` exactly once
with all of them.

The slice passed to `f` in the block pass reuses its backing array between
calls.

Height pairs used in scope:

| Caller | `(begin, end)` | Visit order |
|---|---|---|
| `ListSinceBlock(start, end, …)` as called by `listsinceblock` | `(-1, h)`, `h ≥ 0` | unmined, then blocks tip → `h` inclusive |
| `ListTransactions`, `ListAllTransactions` | `(-1, 0)` | unmined, then blocks tip → 0 |
| `ListAddressTransactions` | `(0, -1)` | blocks 0 → tip, then unmined |
| `GetTransactions`, `GetTickets*` with nil identifiers | `(0, -1)` | blocks 0 → tip, then unmined |
| `GetTransactions`, `GetTickets*` with caller identifiers | `blockRange` result | per the rules above; unmined is visited first if `start < 0`, last if `start ≥ 0` and `end < 0`, not at all if both are ≥ 0; with `start < 0` and `end < 0` only unmined is visited (the block pass runs backward from `0x7FFFFFFF` to `0x7FFFFFFF` and matches nothing) |

**`ListSinceBlock`** (`wallet/wallet.go:2525–2547`):

1. `txList` := empty, non-nil.
2. View: `RangeTransactions(ctx, wtxmgr, start, end, rangeFn)` where `rangeFn`,
   for each detail in slice order, calls `listTransactions(tx, &detail,
   w.manager, syncHeight, w.chainParams)` and appends **receives, then sends**.
   `rangeFn` always returns `(false, nil)`.
3. On error return `(nil, errors.E("wallet.ListSinceBlock", err))`.

The only caller, JSON-RPC `listsinceblock`, always passes `start = -1`. `end`
is the height of the header named by the request's block hash, or `0` when no
hash was given (`internal/rpc/jsonrpc/methods.go:3166–3179`), so the no-hash
form lists every wallet transaction. The function's doc comment says "If the
block is -1 then all transactions are included"; that is not what the code
does for `end = -1` (`start = -1, end = -1` visits unmined transactions only,
see the last row of the table above) and holds for `start = -1` only when
`end = 0` (Q-37).

`syncHeight` comes from the caller and is not re-read inside the transaction.
The JSON-RPC handler passes the tip height it read in an earlier, separate
transaction (`internal/rpc/jsonrpc/methods.go:3152–3179`).

### 5.3 `ListTransactions`

`wallet/wallet.go:2552–2607`.

1. `txList` := empty, non-nil.
2. View. `tipHeight` := tip height. `skipped` := 0, `n` := 0.
3. `RangeTransactions(ctx, wtxmgr, -1, 0, rangeFn)`. `rangeFn(details)` iterates
   `i` from `len(details)-1` down to `0`:
   1. If `n >= count`: return `(true, nil)` (stop the whole range).
   2. If `from > skipped`: `skipped++`; continue.
   3. `sends, receives := listTransactions(dbtx, &details[i], w.manager, tipHeight, w.chainParams)`.
   4. Append **sends, then receives** to `txList`.
   5. If `len(sends) != 0 || len(receives) != 0`: `n++`.
   After the loop return `(false, nil)`.
4. On error return `(nil, errors.E("wallet.ListTransactions", err))`.
5. After the transaction closes, reverse `txList` in place.

Resulting contract:

- Transactions are walked newest first: unmined in **descending** raw-hash
  order, then blocks from tip downward, each block's transactions in reverse
  of the block record order.
- `from` counts **transactions**, including transactions that would produce no
  entries (for example a transaction whose only wallet-related output is a
  change credit and which has no debits).
- `count` counts transactions that produced at least one entry, not entries.
  The array returned can therefore hold more than `count` elements.
- `count <= 0` returns `[]` (the stop test precedes everything). A negative
  `from` skips nothing.
- After the final reversal the array is oldest transaction first, and within
  each transaction: receive entries in **descending** `vout` order, followed
  by send entries in **descending** `vout` order.

### 5.4 `ListAddressTransactions`

`wallet/wallet.go:2612–2651`.

1. `txList` := empty, non-nil. View. `tipHeight` := tip height.
2. `RangeTransactions(ctx, wtxmgr, 0, -1, rangeFn)`. For each detail in slice
   order, for each credit in `detail.Credits` order:
   1. If `detail.MsgTx.TxOut[cred.Index].Version != 0`: next credit.
   2. `pkh` := `stdscript.ExtractPubKeyHashV0(pkScript)` — non-nil only for the
      exact 25-byte script `OP_DUP OP_HASH160 OP_DATA_20 <20> OP_EQUALVERIFY
      OP_CHECKSIG` (`dcrd/txscript/v4@v4.1.2/stdscript/scriptv0.go:151–163`).
   3. If `string(pkh)` is not a key of `pkHashes`: next credit.
   4. Otherwise call `listTransactions` for the whole transaction, append
      **receives, then sends**, and move to the next transaction.
3. On error return `(nil, errors.E("wallet.ListAddressTransactions", err))`.

Order: blocks ascending, block-record order within a block, unmined last in
ascending raw-hash order; per transaction receives (ascending `vout`) then
sends (ascending `vout`). The entries are **all** entries of a matching
transaction, not only those for the matching address. Change credits do count
for the match in step 2 even though they produce no entries themselves.
P2SH, stake-tagged and P2PK credits never match (Q-7).

### 5.5 `ListAllTransactions` and `ListTransactionDetails`

**`ListAllTransactions`** (`wallet/wallet.go:2656–2693`): identical to
`ListTransactions` with the `from`/`count` logic removed — every transaction
from `RangeTransactions(-1, 0)` is processed in reverse slice order, sends
appended before receives, and the whole list reversed at the end. Error op
`"wallet.ListAllTransactions"`.

**`ListTransactionDetails`** (`wallet/wallet.go:2697–2721`):

1. View. `tipHeight` := tip height.
2. `txd, err := w.txStore.TxDetails(wtxmgr, txHash)`: the unmined record if one
   exists, else the mined record with the greatest key for that hash; if
   neither, `errors.E(errors.NotExist)` (`wallet/udb/txquery.go:181–196`).
3. `sends, receives := listTransactions(dbtx, txd, w.manager, tipHeight, w.chainParams)`.
4. Result := receives, then sends (both ascending `vout`), in a fresh non-nil slice.
5. On error return `(nil, errors.E("wallet.ListTransactionDetails", err))`.

### 5.6 `BlockInfo` and `TransactionSummary`

**`BlockInfo`** (`wallet/wallet.go:2752–2791`). One View:

1. `tipHeight` := tip height.
2. `blockHash` := `blockID.hash`. If nil: `GetMainChainBlockHashForHeight(wtxmgr,
   blockID.height)` — reads block record `b[height BE]`; absent →
   `errors.E(errors.NotExist, "no block at height <h> in main chain")`.
3. `header` := `GetSerializedBlockHeader(wtxmgr, blockHash)` — a copy of
   `h[hash]`; absent → `errors.E(errors.NotExist, "block header")`.
4. `height` := LE `uint32` at header offset 128, as `int32`.
5. `(inMainChain, invalidated)` := `BlockInMainChain(dbtx, blockHash)`: true
   only if the block record at the header's height has the same hash;
   `invalidated` is byte 42 of that record ≠ 0.
6. `confs` := `confirms(height, tipHeight)` if `inMainChain`, else `0`.
7. Return `{Hash, Height, Confirmations: confs, Header: header,
   Timestamp: int64(LE uint32 at offset 136), StakeInvalidated: invalidated}`.

Error op: `"wallet.BlockInfo"`.

**`TransactionSummary`** (`wallet/wallet.go:2795–2817`). One View:

1. `tipHeight` := tip height; `txDetails` := `TxDetails(wtxmgr, txHash)`.
2. `txSummary` := `makeTxSummary(dbtx, w, txDetails)` (§5.8).
3. `confs` := `confirms(txDetails.Height(), tipHeight)`.
4. `blockHash` := `&txDetails.Block.Hash` if `confs > 0`, else nil.

On error returns `(nil, 0, nil, errors.E(op, err))` with
`op = "wallet.TransactionSummary(<txHash>)"` (the hash in byte-reversed hex).

### 5.7 `blockRange`

`wallet/wallet.go:3040–3081`. Runs inside the caller's read transaction.

1. `start` := 0, `end` := -1.
2. For the start identifier: nil → keep 0; `hash == nil` → `start = height`
   (no validation; may be negative); otherwise load `h[hash]`, deserialize it
   as a `wire.BlockHeader` and use `int32(header.Height)`.
3. Same for the end identifier with default -1.
4. A missing header yields `errors.E(errors.NotExist, "block header")`; a
   deserialization failure yields the raw `wire` error.

There is no check that a block named by hash is on the main chain; only its
height is used.

### 5.8 `GetTransactions` and `makeTxSummary`

**`GetTransactions`** (`wallet/wallet.go:3248–3303`). One View:

1. `(start, end)` := `blockRange(dbtx, startBlock, endBlock)`.
2. `RangeTransactions(ctx, wtxmgr, start, end, rangeFn)`. `rangeFn(details)`:
   1. Copy `details` into a fresh slice (the store reuses its backing array).
   2. `txs[i]` := `makeTxSummary(dbtx, w, &details[i])` for every element, in
      slice order.
   3. If `details[0].Block.Height != -1`: load `h[details[0].Block.Hash]`,
      deserialize into a new `wire.BlockHeader` (errors are returned), and
      build `Block{Header: header, Transactions: txs}`. Otherwise
      `Block{Header: nil, Transactions: txs}`.
   4. Return `f(block)`.
3. Error op `"wallet.GetTransactions"` (wraps errors from `f` too).

`f` runs inside the read transaction. `f` returning `true` stops iteration
without error.

**`makeTxSummary`** (`wallet/notifications.go:114–181`, owned by the
notifications spec; restated because `GetTransactions`, `TransactionSummary`
and the ticket functions return its output):

| Field | Value |
|---|---|
| `Hash` | pointer to the details' hash |
| `Transaction` | `details.SerializedTx` if non-nil, otherwise `MsgTx.Serialize()` |
| `MyInputs` | nil when there are no debits; otherwise one element per debit in debit order: `{Index, PreviousAccount, PreviousAmount}`. `PreviousAccount` is the account of the first address extracted from the previous output's script, or `0` on any lookup failure (a failed previous-transaction or account lookup is logged at error level; a previous script that yields no address returns `0` without logging). The previous output is indexed without a bounds check |
| `MyOutputs` | non-nil; one element per credit, matched by walking outputs in order against `details.Credits` in order: `{Index, Account, Internal, Amount, Address, OutputScript}`. If the output script yields no address (silently) or the `manager.Address` lookup of the first address fails (logged at error level), the element is still emitted with zero `Account`, `Internal = false`, zero `Amount`, nil `Address` and nil `OutputScript` |
| `Fee` | `Σdebits − Σoutputs` if `len(Debits) == len(TxIn)`, else 0 |
| `Timestamp` | `Received.Unix()`, or the block time if the transaction is mined and the block time is earlier |
| `Type` | `TransactionTypeCoinbase` (1) if `IsEitherCoinBaseTx`; else `TicketPurchase` (2) if `IsSStx`; else `Vote` (3) if `IsSSGen`; else `Revocation` (4) if `IsSSRtx`; else `Regular` (0) |

### 5.9 `fetchTicketDetails` and the `makeTicketSummary` status machine

**`fetchTicketDetails`** (`wallet/wallet.go:2820–2837`):

1. `txDetail` := `TxDetails(ns, hash)`; propagate error (`NotExist` if unknown).
2. If `txDetail.TxType != TxTypeSStx`: error `"<hash> is not a ticket"`
   (plain `fmt.Errorf`, no kind).
3. `ticketDetails` := `txStore.TicketDetails(ns, txDetail)`; on error return
   `"<err> while trying to get ticket details for txhash: <hash>"` (plain,
   the inner error is formatted with `%v` and not wrapped).

**`txStore.TicketDetails`** (`wallet/udb/txquery.go:211–241`) determines the
spender:

1. If the transaction is not `IsSStx`: return `(nil, nil)`.
2. Look up the mined credit for output 0 at the ticket's block
   (`c[hash ‖ height ‖ blockhash ‖ 0]`).
   - Credit exists and is flagged spent: spender hash := hash in the credit's
     spender debit key.
   - Credit exists and is not flagged spent: spender hash stays all-zero. The
     unmined-inputs bucket is **not** consulted (Q-10).
   - Credit does not exist (no credit for output 0, or the ticket is unmined):
     spender hash := value of `mi[hash ‖ 0]` if present.
3. `Spender` := `TxDetails(ns, spenderHash)`; a `NotExist` error is swallowed
   and leaves `Spender` nil.

**`makeTicketSummary`** (`wallet/wallet.go:2888–2933`):

1. `ticketHeight` := `details.Ticket.Height()`; `tipHeight` := tip height.
2. `Ticket` := `makeTxSummary(details.Ticket)`.
3. `Status` := `TicketStatusLive` if `rpc != nil`, else `TicketStatusUnspent`.
4. If `ticketHeight == -1`: `Status` := `Unmined`. Else if
   `!ticketMatured(params, ticketHeight, tipHeight)`: `Status` := `Immature`.
5. If `details.Spender != nil`: `Spender` := `makeTxSummary(details.Spender)`, and by
   `details.Spender.TxType`:
   - `TxTypeSSGen` → `Voted`.
   - `TxTypeSSRtx` → `Expired` if
     `Spender.Block.Height − Ticket.Block.Height >= int32(params.TicketExpiryBlocks()) + int32(params.TicketMaturity)`,
     else `Missed`.
   - anything else → status from step 3/4 is kept.

`rpc` is never called. The only difference between the "precise" and
non-precise variants is `Live` versus `Unspent` for a matured ticket with no
recorded spender (Q-9).

| Condition (evaluated in this precedence) | `rpc != nil` | `rpc == nil` |
|---|---|---|
| spender is a vote | `voted` | `voted` |
| spender is a revocation, height difference ≥ expiry + maturity | `expired` | `expired` |
| spender is a revocation, otherwise | `missed` | `missed` |
| ticket unmined | `unmined` | `unmined` |
| ticket mined, `tip − h ≤ TicketMaturity` | `immature` | `immature` |
| otherwise | `live` | `unspent` |

**`GetTicketInfoPrecise` / `GetTicketInfo`** (`wallet/wallet.go:2947–3035`).
One View each:

1. `ticketDetails` := `fetchTicketDetails(wtxmgr, hash)`.
2. `ticketSummary` := `makeTicketSummary(rpc or nil, dbtx, w, ticketDetails)`.
3. If the ticket is unmined, return with a nil header.
4. Load `h[ticket block hash]`; `blockHeader.FromBytes(bytes)`; errors are
   returned.

Error ops: `"wallet.GetTicketInfoPrecise"`, `"wallet.GetTicketInfo"`.

### 5.10 `GetTicketsPrecise` and `GetTickets`

`wallet/wallet.go:3102–3231`. The two bodies are identical except for the
`rpc`/`nil` argument to `makeTicketSummary` and the op string
(`"wallet.GetTicketsPrecise"`, `"wallet.GetTickets"`). One View:

1. `(start, end)` := `blockRange(dbtx, startBlock, endBlock)`.
2. Allocate one `*wire.BlockHeader` (`header`) for the whole call.
3. `RangeTransactions(ctx, wtxmgr, start, end, rangeFn)`. `rangeFn(details)`:
   1. For each `details[i]` in order: `ticketInfo := txStore.TicketDetails(wtxmgr, &details[i])`.
      On error return `"<err> while trying to get ticket details for txhash: <hash>"`.
      If `ticketInfo == nil` (not a ticket) continue. Otherwise append
      `makeTicketSummary(rpc, dbtx, w, ticketInfo)`.
   2. If no tickets were collected return `(false, nil)` — `f` is not called.
   3. If `details[0].Block.Height == -1` return `f(tickets, nil)`.
   4. Otherwise load `h[details[0].Block.Hash]` (error returned), call
      `header.FromBytes(bytes)` **ignoring its error**, and return
      `f(tickets, header)`.

`f` is therefore called once per block containing at least one ticket, with the
tickets in block-record order, and once for all unmined tickets in ascending
raw-hash order. The header pointer is the same object on every call and each
`TicketSummary.Ticket.Hash` points into storage reused by the next call; `f`
must not retain them.

### 5.11 `Accounts`

`wallet/wallet.go:3371–3433`.

1. Lock `w.lockedOutpointMu`; it stays held until the function returns.
2. View:
   1. `(tipHash, tipHeight)` := main chain tip.
   2. `unspent` := `txStore.UnspentOutputs(dbtx)` (§5.12 step 4); error aborts.
   3. `manager.ForEachAccount(waddrmgr, fn)` iterates every non-bucket key of
      bucket `acct` in ascending bytewise order of the 4-byte little-endian
      account number. For each, `props := manager.AccountProperties(waddrmgr,
      acct)` (error aborts) and `AccountResult{AccountProperties: *props}` is
      appended.
   4. Build a map from account number to a pointer to that element's
      `TotalBalance`.
   5. For each unspent credit, in `UnspentOutputs` order:
      `(_, addrs) := stdscript.ExtractAddrs(0, output.PkScript, params)`; skip
      if empty; `AddrAccount(waddrmgr, addrs[0])`; if it succeeds and the
      account is in the map, add `output.Amount`.
3. On error return `(nil, errors.E("wallet.Accounts", err))`.
4. Return `{Accounts, CurrentBlockHash: &tipHash, CurrentBlockHeight: tipHeight}`.

`TotalBalance` is thus the sum of every wallet UTXO that is not spent by an
unmined transaction and does not belong to an unpublished transaction —
including unconfirmed, immature and locked outputs — attributed to the account
of the output script's first address. Outputs whose script yields no address
or an unknown address are not counted anywhere.

`AccountProperties` values come from the address manager
(`wallet/udb/addressmanager.go:550–605`): for account `0x7FFFFFFF`
(`ImportedAddrAccount`) the name is `"imported"`, `AccountType` and all four
index fields are `0`, and `ImportedKeyCount` is the number of address rows
indexed under that account; for every other account the indexes come from the
account row (`0xFFFFFFFF` meaning "none") and `ImportedKeyCount` is 0.

### 5.12 `ListUnspent`

`wallet/wallet.go:3477–3667`. One View.

1. `results` := nil slice.
2. `tipHeight` := tip height.
3. `filter` := `len(addresses) != 0`.
4. `unspent` := `txStore.UnspentOutputs(dbtx)` (`wallet/udb/txmined.go:2463–2531`):
   first every key of bucket `u` in ascending order, skipping outpoints that
   have an entry in `mi` (spent by an unmined transaction); then every key of
   bucket `mc` in ascending order, skipping outpoints with an entry in `mi` and
   outpoints whose transaction hash is in `up` (unpublished). Each `Credit`
   carries `OutPoint` (with `Tree` derived from the credit's opcode tag),
   `Height` (`-1` for unmined), `Amount`, `PkScript`, `Received`,
   `FromCoinBase` (always false for unmined credits), `HasExpiry`.
5. `sort.Sort(sort.Reverse(creditSlice(unspent)))`. `creditSlice.Less(i, j)`
   (`wallet/wallet.go:3446–3466`), first matching case:
   1. same transaction hash → `index_i < index_j`;
   2. both heights `-1` → `Received_i` strictly before `Received_j`;
   3. `height_i == -1` → false;
   4. `height_j == -1` → true;
   5. otherwise → `height_i < height_j`.

   After reversal the order is: unmined outputs first (newest received time
   first), then mined outputs by descending height; outputs of the same
   transaction by descending index. Outputs of *different* transactions with
   equal height, or unmined with equal received second, compare equal, and
   `sort.Sort` is not stable: their relative order is whatever Go's sort
   produces from the step-4 input order (Q-13).
6. `defaultAccountName` := `manager.AccountName(waddrmgr, 0)`; error aborts.
7. For each output in sorted order:
   1. `details` := `txStore.TxDetails(wtxmgr, &output.Hash)`; error aborts the
      whole call.
   2. `confs` := `confirms(output.Height, tipHeight)`. Skip if
      `confs < minconf || confs > maxconf` (both bounds inclusive).
   3. If `output.FromCoinBase` and
      `!coinbaseMatured(params, output.Height, tipHeight)`: skip.
   4. By `details.TxType`:
      - `TxTypeSStx`: if `output.Index == 0` and
        `!ticketMatured(params, details.Height(), tipHeight)`: skip. If
        `output.Index > 0 && output.Index % 2 == 0` and
        `!ticketChangeMatured(params, details.Height(), tipHeight)`: skip.
      - `TxTypeSSGen`, `TxTypeSSRtx`: if
        `!coinbaseMatured(params, details.Height(), tipHeight)`: skip.
      - other types: no check.
   5. If `w.LockedOutpoint(&output.Hash, output.Index)`: skip.
   6. `acctName` := `defaultAccountName`.
      `(sc, addrs) := stdscript.ExtractAddrs(0, output.PkScript, params)`. If
      `len(addrs) > 0`: `AddrAccount(waddrmgr, addrs[0])` then
      `AccountName(waddrmgr, acct)`; if both succeed `acctName` := that name.
   7. If `accountName != "" && accountName != acctName`: skip.
   8. If `filter`: keep the output only if `addr.String()` of **any** element
      of `addrs` is a key of `addresses`; otherwise skip.
   9. Determine `spendable` and `redeemScript` from `sc`:

      | `sc` | `spendable` | Extra |
      |---|---|---|
      | `STPubKeyHashEcdsaSecp256k1` | true | |
      | `STPubKeyEcdsaSecp256k1` | true | |
      | `STScriptHash` | true | `len(addrs) != 1` → abort with `"invalid address count for pay-to-script-hash output"`; `redeemScript := manager.RedeemScript(waddrmgr, addrs[0])`, error aborts |
      | `STStakeGenPubKeyHash`, `STStakeGenScriptHash` | true | no redeem script lookup |
      | `STStakeRevocationPubKeyHash`, `STStakeRevocationScriptHash` | true | no redeem script lookup |
      | `STStakeChangePubKeyHash`, `STStakeChangeScriptHash` | true | no redeem script lookup |
      | `STMultiSig` | true only if `manager.Address(waddrmgr, a)` succeeds for every extracted address; the first `NotExist` makes it false; any other error aborts | |
      | every other type (stake submission, treasury gen/add, Ed25519, Schnorr, null data, non-standard) | false | |
   10. `spendable` := `spendable && len(addrs) > 0`.
   11. Build the result (§4.2): `TxID`, `Vout`, `Tree = output.Tree`,
       `Account = acctName`, `ScriptPubKey = hex(PkScript)`,
       `RedeemScript = hex(redeemScript)` (empty string when nil),
       `TxType = int(details.TxType)`, `Amount = output.Amount.ToCoin()`,
       `Confirmations = int64(confs)`, `Spendable`. `Address` :=
       `addrs[0].String()` if any address was extracted, else `""`.
   12. Append.
8. On error return `(nil, errors.E("wallet.ListUnspent", err))`; otherwise
   return `results` (nil if nothing was appended).

Consequences of step 7.4 for unmined transactions (`details.Height() == -1`):
every maturity predicate requires `txHeight >= 0`, so with `minconf = 0` an
unmined ticket's output 0 and its even-index outputs above 0, and **all**
outputs of an unmined vote or revocation, are never listed; outputs of unmined
regular transactions are. The `FromCoinBase` test in step 7.3 uses the
credit's own height and flag, the type switch in step 7.4 uses the containing
transaction's record; both heights are the same value for a given output.

Step order matters for error behaviour: `TxDetails` (7.1) runs before any
filter, so a store error for an output that would have been filtered out
still aborts the call; the P2SH redeem-script lookup (7.9) runs only for
outputs that passed the confirmation, maturity, lock, account and address
filters, so an unknown redeem script aborts the call only when that output is
selected.

`addresses` keys are compared against `stdaddr.Address.String()` of the
extracted addresses, i.e. exact encoded-string equality; how the JSON-RPC
handler builds the set is owned by the JSON-RPC spec. `accountName == ""`
disables the account filter.

### 5.13 `LoadPrivateKey` and `DumpWIFPrivateKey`

**`LoadPrivateKey`** (`wallet/wallet.go:3669–3684`): one View calling
`manager.PrivateKey(waddrmgr, addr)` (`wallet/udb/addressmanager.go:2470–2523`),
which normalises pubkey addresses to their pubkey-hash form, looks up the
address row and

- for a BIP0044-derived row derives the child private key from the account
  xpriv (fails with the manager's lock errors when the wallet/account is
  locked);
- for an imported row decrypts the stored key with the private crypto key
  (`errors.Crypto`, `"decrypt imported privkey: <err>"` on failure);
- for a script row fails with `errors.E(errors.Invalid, "no private key for
  P2SH address")`.

Returns `(key, key.Zero, nil)`, or `(nil, nil, errors.E("wallet.LoadPrivateKey", err))`.

**`DumpWIFPrivateKey`** (`wallet/wallet.go:3688–3701`):

1. `privKey, zero, err := w.LoadPrivateKey(ctx, addr)`; on error return
   `("", errors.E("wallet.DumpWIFPrivateKey", err))`.
2. `defer zero()`.
3. `wif := dcrutil.NewWIF(privKey.Serialize(), params.PrivateKeyID,
   dcrec.STEcdsaSecp256k1)`; cannot fail for this scheme.
4. Return `wif.String()` (§4.5).

### 5.14 `ImportPrivateKey`, `ImportPublicKey`, `ImportScript`

**`ImportPrivateKey`** (`wallet/wallet.go:3705–3738`):

1. Update transaction:
   1. `maddr, err := manager.ImportPrivateKey(waddrmgr, wif)`
      (`wallet/udb/addressmanager.go:1243–1292`): fails with `errors.Locked`
      if the manager is locked and not watching-only; fails with
      `errors.E(errors.Exist, "address for private key already exists")` if
      `Hash160(wif.PubKey())` is already an address row; stores the encrypted
      compressed pubkey and (unless watching-only) the encrypted private key
      under account `0x7FFFFFFF`.
   2. If that succeeded: `addr = maddr.Address()` (P2PKH-ECDSA-secp256k1 v0 of
      `Hash160(compressed pubkey)`) and `props, err =
      manager.AccountProperties(waddrmgr, ImportedAddrAccount)`.
   3. The closure returns `err`; the transaction commits only if nil.
2. On error return `("", errors.E("wallet.ImportPrivateKey", err))`.
3. `n, err := w.NetworkBackend()`. If a backend is set:
   `n.LoadTxFilter(ctx, false, []stdaddr.Address{addr}, nil)`; on error return
   `("", errors.E(op, err))` — the database change is already committed.
4. `log.Infof("Imported payment address %s", addrStr)`.
5. `w.NtfnServer.notifyAccountProperties(props)`.
6. Return `addrStr`.

**`ImportPublicKey`** (`wallet/wallet.go:3742–3775`): identical flow with op
`"wallet.ImportPublicKey"` and `manager.ImportPublicKey(waddrmgr, pubkey)`
(`wallet/udb/addressmanager.go:1297–1348`), whose checks in order are:

1. Not watching-only → `errors.E(errors.Invalid, "public keys may only be imported by watching-only wallets")`.
2. `len(pubkey) != 33` → `errors.E(errors.Encoding, "invalid length for compressed pubkey")`.
3. `pubkey[0]` not `0x02`/`0x03` → `errors.E(errors.Encoding, "invalid format byte for compressed pubkey")`.
4. Address exists → `errors.E(errors.Exist, "address for public key already exists")`.

The point is not validated to be on the curve. The log line and notification
are the same as for `ImportPrivateKey`.

**`ImportScript`** (`wallet/wallet.go:3780–3805`). Everything happens inside
one Update transaction:

1. `mscriptaddr, err := manager.ImportScript(waddrmgr, rs)`
   (`wallet/udb/addressmanager.go:1355–1382`): fails with
   `errors.E(errors.Exist, "script already exists")` if `Hash160(rs)` is
   already an address row; otherwise stores the script under account
   `0x7FFFFFFF`.
2. `addr := mscriptaddr.Address()` (P2SH v0 of `Hash160(rs)`).
3. If a network backend is set: `n.LoadTxFilter(ctx, false, [addr], nil)`; on
   error the closure returns it and the transaction rolls back.
4. `log.Infof("Imported script with P2SH address %v", addr)` — emitted before
   the commit.
5. Commit. On any error return `errors.E("wallet.ImportScript", err)`.

No account-properties notification is sent for script imports.

**Account notification payload for key imports.** `props` is read inside the
import's own Update transaction, after the manager has stored the key, so its
`ImportedKeyCount` already includes the new address row.
`notifyAccountProperties` (`wallet/notifications.go:600–624`) returns
without doing anything when no account-notification client is registered;
otherwise it sends one `AccountNotification` to every client, in the order
of the server's client slice:

| Field | Value for an import (`AccountNumber == 0x7FFFFFFF`) |
|---|---|
| `AccountNumber` | `0x7FFFFFFF` |
| `AccountName` | `"imported"` |
| `ExternalKeyCount` | `0` (the gap-limit adjustment applies only to accounts `<= MaxAccountNum`) |
| `InternalKeyCount` | `0` |
| `ImportedKeyCount` | number of address rows under the imported account, counting keys **and** scripts (`wallet/udb/addressmanager.go:586–600`) |

**Filter load as seen by the backend.** The three imports above and the two
account imports of §5.15 each make exactly one `LoadTxFilter(ctx, false,
addrs, nil)` call when a backend is set:

| Backend | Effect | Failure |
|---|---|---|
| dcrd RPC syncer (`chain/backend.go:48–50`, `rpc/client/dcrd/calls.go:411–433`) | one JSON-RPC request `loadtxfilter` with positional parameters `false`, a JSON array of the address strings in the order given, and `[]` (the nil outpoint slice is converted to an empty, non-null array) | the call error wrapped with inner op `dcrd.LoadTxFilter` |
| SPV syncer (`spv/backend.go:417–433`) | under the syncer's filter mutex, each address is added to the in-memory rescan filter and its payment script to the filter data; nothing is sent to peers | never fails |

Address order in the request: one address for key and script imports;
external children `0 … gapLimit−1` for a voting account; external children
`0 … gapLimit−1` followed by internal children `0 … gapLimit−1` for an xpub
account (indexes that fail with `ErrInvalidChild` are omitted, so the array
can be shorter). Every derived address is the version-0 ECDSA-secp256k1
pubkey-hash address of `Hash160(compressed child pubkey)`
(`wallet/addresses.go:1004–1020`, `internal/compat/compat.go:10–14`).

### 5.15 `votingXprivFromSeed`, `ImportVotingAccount`, `ImportXpubAccount`

**`votingXprivFromSeed`** (`wallet/wallet.go:3814–3833`):

1. If `len(seed) < 16 || len(seed) > 64`: return
   `errors.E("wallet.VotingXprivFromSeed", errors.Invalid, errors.New("invalid seed length"))`.
2. `udb.HDKeysFromSeed(seed, params)` (`wallet/udb/addressmanager.go:2807–2882`):
   master key from seed; `m/44'/<LegacyCoinType>'` and
   `m/44'/<SLIP0044CoinType>'`; account 0 of each; both branch keys (0 and 1)
   of each account must derive. All derivations use `hdkeychain.Child` (the
   Decred variant, not `ChildBIP32Std`). A failure with
   `hdkeychain.ErrInvalidChild` at the account or branch level becomes
   `errors.E(errors.Seed, hdkeychain.ErrUnusableSeed)`; other errors pass
   through. The error is returned **without** the op.
3. Zero the legacy coin-type key, the SLIP0044 coin-type key and the legacy
   account key.
4. Return the SLIP0044 account-0 extended private key
   (`m/44'/<SLIP0044CoinType>'/0'`).

Coin types (`dcrd/chaincfg/v3@v3.3.0`): mainnet SLIP0044 42 / legacy 20;
testnet3 1 / 11; simnet 1 / 115; regnet 1 / 1.

**`ImportVotingAccount`** (`wallet/wallet.go:3838–3891`):

1. Update: `accountN, err = manager.ImportVotingAccount(dbtx, xpriv,
   passphrase, name)` (`wallet/udb/addressmanager.go:2181–2318`). Checks, in
   order: empty passphrase → `errors.E(errors.Passphrase, "passphrase must be
   specified")`; name empty or `"imported"` → `errors.Invalid`; key not
   private → `errors.E(errors.Invalid, "extended key must be an xpriv")`;
   name in use → `errors.E(errors.Exist, "account named \"<name>\" already
   exists")`; address of external child 0 already known →
   `errors.E(errors.Exist, "address belonging to this key already exists in
   the database")`. The account number is the last imported account number
   plus one (first value `0x80000000`). The account row has type
   `importedVoting`; its private key is re-encrypted under a fresh Argon2id
   key derived from `passphrase`. Details are in the address manager spec.
2. On error return `(0, errors.E("wallet.ImportVotingAccount", err))`.
3. `xpub := xpriv.Neuter()`; `(extKey, intKey) := deriveBranches(xpub)`
   (children 0 and 1); error → `(0, errors.E(op, err))`.
4. If a network backend is set: `extAddrs := deriveChildAddresses(extKey, 0,
   w.gapLimit, params)` (children `0 … gapLimit−1`, silently skipping any
   index that yields `ErrInvalidChild`), then `n.LoadTxFilter(ctx, false,
   extAddrs, nil)`. Errors → `(0, errors.E(op, err))`. Internal-branch
   addresses are not watched.
5. Lock `addressBuffersMu`; set `w.addressBuffers[accountN]` to
   `bip0044AccountData{xpub, albExternal, albInternal}` where both buffers are
   `{branchXpub: extKey|intKey, lastUsed: 0xFFFFFFFF, cursor: 0,
   lastWatched: gapLimit − 1}`.
6. Return `accountN`.

**`ImportXpubAccount`** (`wallet/wallet.go:3893–3951`):

1. If `xpub.IsPrivate()`: return
   `errors.E("wallet.ImportXpubAccount", "extended key must be an xpub")`
   (kind `Other`).
2. `(extKey, intKey) := deriveBranches(xpub)`; error → `errors.E(op, err)`.
3. If a network backend is set: derive `gapLimit` external and `gapLimit`
   internal addresses and `n.LoadTxFilter(ctx, false, ext ++ int, nil)`
   (external first). Errors → `errors.E(op, err)`. This happens **before** the
   database write.
4. Update: `manager.ImportXpubAccount(waddrmgr, name, xpub)`
   (`wallet/udb/addressmanager.go:1384–1436`: name validation; name in use →
   `errors.E(errors.Exist, "account name in use")`; account number = last
   imported + 1; row type `actBIP0044`, public key only, all four indexes
   `0xFFFFFFFF`), then `account = manager.LookupAccount(waddrmgr, name)`.
5. On error return `errors.E(op, err)`.
6. Register address buffers exactly as in `ImportVotingAccount` step 5, keyed
   by `account`.

### 5.16 `UnminedTransactions`

`wallet/wallet.go:4361–4380`.

1. View: `recs := txStore.UnminedTxs(dbtx)` — every record of bucket `m`
   loaded into a hash map, flagged `Unpublished` when the hash is in bucket
   `up`, then topologically sorted so that a transaction spending another
   unmined transaction's output comes after it
   (`wallet/udb/txunmined.go:285–316`, `wallet/udb/kahnsort.go`).
2. On error return `(nil, errors.E("wallet.UnminedTransactions", err))`.
3. Return the `MsgTx` of every record with `Unpublished == false`, preserving
   the sorted order. The slice is non-nil.

The order among transactions with no dependency relation derives from Go map
iteration and is not deterministic (Q-25).

### 5.17 `hasVotingAuthority`, `StakeInfo`, `StakeInfoPrecise`

**`hasVotingAuthority`** (`wallet/wallet.go:3987–4005`):

1. `out := ticketPurchase.TxOut[0]`; `(_, addrs) :=
   stdscript.ExtractAddrs(out.Version, out.PkScript, params)`.
2. For each address implementing `Hash160er`: if
   `manager.ExistsHash160(waddrmgr, hash160)`, return
   `(true, haveKey, err)` from `manager.HavePrivateKey(waddrmgr, a)`.
3. Otherwise `(false, false, nil)`.

Only output 0 is examined and only the **first** extracted address whose
hash160 is an address row decides the result. Addresses that do not implement
`Hash160er` are skipped. `manager.HavePrivateKey`
(`wallet/udb/addressmanager.go:2527–2547`) takes the manager read lock and
returns:

| Address row found for the hash160 | `havePrivKey` |
|---|---|
| BIP0044-derived row (`dbChainAddressRow`) | `account < 0x7FFFFFFF` — true for ordinary accounts, **false for addresses of imported xpub and imported voting accounts** (numbers ≥ `0x80000000`), including a voting account whose xpriv the wallet holds |
| imported-key row | true iff an encrypted private key is stored (false for keys imported by a watching-only wallet or through `ImportPublicKey`) |
| script row or any other row type | false |
| row lookup fails | `(true, false, err)` is returned from `hasVotingAuthority`; `StakeInfo*` abort with that error |

`mine` does not depend on whether the wallet is watching-only or locked.
`StakeInfo*` use only `mine`; `wallet/chainntfns.go:759` uses both values
(part owned by the chain-notification spec).

**`StakeInfo`** (`wallet/wallet.go:4008–4111`). One View:

1. `(tipHash, tipHeight)` := tip. `res.BlockHeight = int64(tipHeight)`.
2. If `deployments.DCP0001.Active(tipHeight, params.Net)`: load the tip header
   and set `res.Sdiff = w.nextRequiredDCP0001PoSDifficulty(dbtx, tipHeader,
   nil)`; errors abort. `Active` is true only for mainnet (height ≥ 149248),
   testnet3 and simnet (height ≥ 0), and legacy testnet2 magic `0x48e7a065`
   (height ≥ 46128); for any other network (including regnet) `Sdiff` stays 0
   (`deployments/deployments.go:32–73`).
3. Iterate `txStore.IterateTickets(dbtx)` — bucket `tix` in ascending
   ticket-hash order. Entries whose transaction is neither mined nor unmined
   are skipped by the iterator. For each ticket `it`:
   1. `owned, _, err := hasVotingAuthority(waddrmgr, &it.MsgTx)`; error
      aborts; not owned → next.
   2. `it.Block.Height == -1` → `OwnMempoolTix++`; next.
   3. `!ticketMatured(params, it.Block.Height, tipHeight)` → `Immature++`; next.
   4. If `it.SpenderHash` is non-zero: `spender := txStore.Tx(wtxmgr,
      &it.SpenderHash)` (error aborts). If `IsSSGen(spender)`: `Voted++` and
      `TotalSubsidy += spender.TxIn[0].ValueIn`. Else if `IsSSRtx(spender)`:
      `Revoked++`. Else abort with `errors.E(errors.IO, "ticket spender
      <hash> is neither vote nor revocation")`. Next.
   5. Otherwise `Unspent++`, and if `ticketExpired(params, it.Block.Height,
      tipHeight)`: `UnspentExpired++`.
4. `it.Err()` non-nil aborts.
5. Load and deserialize the tip header; `res.PoolSize = tipHeader.PoolSize`.
6. Error op `"wallet.StakeInfo"`.

The iterator's `SpenderHash` follows the same rule as `TicketDetails` step 2
(§5.9): a ticket whose output-0 credit exists and is not flagged spent by a
*mined* transaction has a zero spender hash even if an unmined vote or
revocation spends it (`wallet/udb/txmined.go:2797–2817`). Two differences
from `TicketDetails`: the iterator looks for a mined record of the ticket
first and only then for an unmined one (`TxDetails` prefers the unmined
record), and for an unmined ticket it sets a zero spender hash without
consulting `mi` (`wallet/udb/txmined.go:2784`, `:2818–2822`).

**`StakeInfoPrecise`** (`wallet/wallet.go:4115–4246`):

1. Start one goroutine (via `errgroup.Group`) that calls
   `rpc.MempoolCount(ctx, "tickets")` — dcrd `getrawmempool false "tickets"`,
   result = number of hashes returned (`rpc/client/dcrd/calls.go:153–163`) —
   and stores it in `res.AllMempoolTix` as `uint32`.
2. Run the same View as `StakeInfo` steps 1–5 with these differences:
   - The spender is loaded with `txStore.TxDetails` (so its block height is
     available).
   - Revocation: `Revoked++`, then `Expired++` if
     `spender.Block.Height − it.Block.Height >= int32(params.TicketExpiryBlocks()) + int32(params.TicketMaturity)`,
     else `Missed++`.
   - Matured and unspent: `Unspent++` and `Live++`. `UnspentExpired` is never
     incremented.
3. If the View failed, return `(nil, errors.E("wallet.StakeInfoPrecise", err))`
   **without waiting** for the goroutine.
4. `g.Wait()`; on error return `(nil, errors.E(op, err))`.

Field population:

| Field | `StakeInfo` | `StakeInfoPrecise` |
|---|---|---|
| `BlockHeight`, `Sdiff`, `PoolSize`, `OwnMempoolTix`, `Immature`, `Voted`, `TotalSubsidy`, `Revoked`, `Unspent` | set | set |
| `UnspentExpired` | set | always 0 |
| `AllMempoolTix` | always 0 | set from dcrd |
| `Live` | always 0 | equals `Unspent` |
| `Missed`, `Expired` | always 0 | set; `Missed + Expired == Revoked` |

### 5.18 Locked outpoints

`LockedOutpoint`, `LockOutpoint`, `UnlockOutpoint`, `ResetLockedOutpoints`
(`wallet/wallet.go:4250–4282`) each take `lockedOutpointMu`, perform one map
operation on `w.lockedOutpoints` keyed by `(hash, index)`, and release it.

**`LockedOutpoints`** (`wallet/wallet.go:4287–4355`):

1. Under `lockedOutpointMu`, copy all keys of the map into a slice (Go map
   iteration order); release the mutex.
2. `allAccts` := `accountName == "" || accountName == "*"`.
3. `acctLocked` := empty, non-nil.
4. View. For each copied outpoint:
   1. `details := txStore.TxDetails(wtxmgr, &op.hash)`. `NotExist` → skip the
      outpoint; any other error aborts.
   2. If `op.index >= len(details.MsgTx.TxOut)` → skip.
   3. If not `allAccts`: extract addresses from the output with its own script
      version; none → skip. `opAcct` := name of the account of `addrs[0]`, or
      `""` if either lookup fails. Skip unless `opAcct == accountName`.
   4. `tree` := 0 if `details.TxType == TxTypeRegular`, else 1.
   5. Append `{Amount: value.ToCoin(), Txid, Vout, Tree}`.
5. Return `(acctLocked, err)` — the error is **not** wrapped with an op, and
   on error the partially filled slice is still returned alongside it.

### 5.19 `Spender` and `UnspentOutput`

Both are one View delegating to the store and returning its error unchanged.

`txStore.Spender` (`wallet/udb/txquery.go:640–723`) outcomes:

| Situation | Result |
|---|---|
| mined record exists, no credit for the output | `errors.E(errors.Invalid, "output is not a credit")` |
| mined credit spent by a mined transaction | spender `MsgTx` and input index from the debit key |
| mined credit, not spent by mined or unmined | `errors.E(errors.NotExist, "credit is unspent")` |
| no mined record and no unmined credit | `errors.E(errors.Invalid, "output is not a credit")` |
| unmined credit, no unmined spender | `errors.E(errors.NotExist, "credit is unspent")` |
| spent by an unmined transaction | that transaction and the index of the first input whose previous outpoint hash and index match (tree ignored) |
| unmined-input record points at a missing unmined tx | `errors.E(errors.NotExist, "missing unmined spending tx")` |
| recorded spender has no matching input | `errors.E(errors.NotExist, "recorded spending tx does not spend credit")` |
| unmined spender fails to deserialize | `errors.E(errors.Bug, err)` |
| mined spender record is short or fails to deserialize | `errors.E(errors.IO, …)` from `readRawTxRecordMsgTx` (`wallet/udb/txdb.go:549–559`) |

`txStore.UnspentOutput(ns, op, includeMempool)`
(`wallet/udb/txmined.go:2537–2553`): if `includeMempool` and an unmined credit
exists for `(hash, index)` → that credit; else if bucket `u` has the outpoint
→ the mined credit (even when an unmined transaction spends it); else
`errors.E(errors.NotExist, "no unspent output <outpoint>")`. The `Tree` of
the argument is ignored for lookup.

### 5.20 `SortedActivePaymentAddresses`

`wallet/wallet.go:4384–4400`. One View: `manager.ForEachActiveAddress` walks
every non-bucket key of `waddrmgr`/`addr`, converts each row to a managed
address and appends `Address().String()`. After the transaction, `sort.Strings`
(ascending bytewise). Error op `"wallet.SortedActivePaymentAddresses"`. "Active"
is a misnomer: every address row is returned — derived, imported-key and
script addresses alike.

### 5.21 Address extraction as used by the listings

`listTransactions`, `Accounts`, `ListUnspent`, `LockedOutpoints` and
`hasVotingAuthority` all derive addresses, account names and (in
`ListUnspent`) the script class from `stdscript.ExtractAddrs(version,
pkScript, params)`. Its result decides the `address`, `account` and
`spendable` fields, so the mapping is restated here
(`dcrd/txscript/v4@v4.1.2/stdscript/address.go:22–29`,
`dcrd/txscript/v4@v4.1.2/stdscript/addressv0.go:30–172`).

**Script version.** Any version other than 0 returns `(STNonStandard, nil)`.
The functions in scope do not agree on which version they pass:

| Function | Version passed | Consequence for a non-zero-version output |
|---|---|---|
| `listTransactions` | the output's own `Version` | no `address`, `account` `""` |
| `LockedOutpoints` (account filter only) | the output's own `Version` | never matches a named account |
| `hasVotingAuthority` | the output's own `Version` | not owned |
| `Accounts`, `ListUnspent` | always `0` (`scriptVersionAssumed`; `udb.Credit` carries no script version, `wallet/udb/txmined.go:167`) | the script bytes are classified as if version 0 |
| `ListAddressTransactions` | explicit `Version != 0` skip, then `ExtractPubKeyHashV0` | never matches |

**Version 0 classification**, tested in this order; the first match wins:

| # | Script form | `ScriptType` | Addresses returned (`String()` form) |
|---|---|---|---|
| 1 | P2PKH ECDSA-secp256k1 | `STPubKeyHashEcdsaSecp256k1` | 1 × pubkey-hash address |
| 2 | P2SH | `STScriptHash` | 1 × script-hash address |
| 3 | P2PKH Ed25519 | `STPubKeyHashEd25519` | 1 × Ed25519 pubkey-hash address |
| 4 | P2PKH Schnorr-secp256k1 | `STPubKeyHashSchnorrSecp256k1` | 1 × Schnorr pubkey-hash address |
| 5 | P2PK ECDSA-secp256k1: `OP_DATA_33 <02/03 ‖ X> OP_CHECKSIG` (35 bytes) or `OP_DATA_65 <04 ‖ X ‖ Y> OP_CHECKSIG` (67 bytes) (`stdscript/scriptv0.go:24–66`) | `STPubKeyEcdsaSecp256k1` | 1 × **pubkey** address if the key parses as a curve point, else none. The address always encodes the compressed key, also for the uncompressed script form |
| 6 | P2PK Ed25519 | `STPubKeyEd25519` | 1 × Ed25519 pubkey address |
| 7 | P2PK Schnorr-secp256k1 | `STPubKeySchnorrSecp256k1` | 1 × Schnorr pubkey address |
| 8 | bare multisig (m-of-n `OP_CHECKMULTISIG`) | `STMultiSig` | one pubkey address per key that parses, in script order; unparsable keys are skipped, so 0…n addresses |
| 9 | stake submission P2PKH / P2SH | `STStakeSubmissionPubKeyHash` / `…ScriptHash` | 1 × pubkey-hash / script-hash address |
| 10 | stake generation P2PKH / P2SH | `STStakeGenPubKeyHash` / `…ScriptHash` | same |
| 11 | stake revocation P2PKH / P2SH | `STStakeRevocationPubKeyHash` / `…ScriptHash` | same |
| 12 | stake change P2PKH / P2SH | `STStakeChangePubKeyHash` / `…ScriptHash` | same |
| 13 | null data: a lone `OP_RETURN`, or `OP_RETURN` followed by exactly one canonical data push of at most 256 bytes (`stdscript/scriptv0.go:455–480`); this is the class of ticket commitment outputs and of a vote's first two outputs | `STNullData` | none |
| 14 | treasury add | `STTreasuryAdd` | none |
| 15 | treasury generation P2PKH / P2SH | `STTreasuryGenPubKeyHash` / `…ScriptHash` | 1 × pubkey-hash / script-hash address |
| 16 | anything else | `STNonStandard` | none |

An address whose constructor fails is dropped rather than reported, so no
caller may assume a length from the script type; the Go checks `len(addrs)`
everywhere except the `STScriptHash` case of `ListUnspent`, which turns a
count other than 1 into an error (§5.12 step 7.9).

Consequences visible to clients:

- **P2PK outputs report a pubkey-form address.** For script form 5 the
  `address` field of `listtransactions`-family entries and of `listunspent` is
  `AddressPubKeyEcdsaSecp256k1V0.String()`: the base58check encoding of
  `PubKeyAddrID ‖ (0x00 | 0x80 if the Y coordinate is odd) ‖ X[32]`
  (`dcrd/txscript/v4@v4.1.2/stdaddr/addressv0.go:185–200`; mainnet prefix
  `Dk`, `dcrd/chaincfg/v3@v3.3.0/mainnetparams.go:533`), not the `Ds…`
  pubkey-hash address of the same key.
- **Account lookup still succeeds for pubkey addresses.**
  `manager.AddrAccount` first normalises any address implementing
  `AddressPubKeyHasher` to its pubkey-hash form
  (`wallet/udb/addressmanager.go:104–110`, `:1016–1030`), so the `account`
  field is filled for P2PK and multisig-member keys the wallet knows. An
  unknown address yields `errors.E(errors.NotExist, "no address <addr>")`,
  which every caller in scope swallows.
- **The `listunspent` address filter is a string comparison against the
  extracted form.** The JSON-RPC handler stores `decodeAddress(s).String()`
  for each requested address (`internal/rpc/jsonrpc/methods.go:3288–3299`).
  A P2PK output is therefore selected only by its pubkey-form address;
  requesting the pubkey-hash address of the same key does not select it. A
  multisig output is selected by the pubkey-form address of any of its keys.
  An explicitly empty address array behaves like an omitted one
  (`len(addresses) == 0` disables the filter).
- **`listTransactions` requires exactly one address.** Multisig outputs with
  two or more parsable keys get no `address` and an empty `account`; a
  multisig output with exactly one parsable key gets that key's pubkey-form
  address.
- **`Accounts` and `ListUnspent` attribute by the first address only**
  (`addrs[0]`), for multisig the first parsable key in script order.

---

## 6. Constants, limits and defaults

| Name | Value | Source | What depends on it |
|---|---|---|---|
| `waddrmgrNamespaceKey` | `"waddrmgr"` | `wallet/wallet.go:77` | top-level bucket for all address-manager reads |
| `wtxmgrNamespaceKey` | `"wtxmgr"` | `wallet/wallet.go:78` | top-level bucket for all transaction-store reads |
| `scriptVersionAssumed` | `0` | `wallet/wallet.go:83` | script version used by `ListAddressTransactions`, `Accounts`, `ListUnspent` |
| unmined height sentinel | `-1` | `wallet/udb/txquery.go:113`, `wallet/wallet.go:2402` | every "is mined" test; `confirms` |
| range "to the end / include unmined" sentinel | any negative height (callers use `-1`) | `wallet/udb/txquery.go:392–397`, `:525`, `:534` | `RangeTransactions`, `blockRange` default end |
| `blockRange` defaults | start `0`, end `-1` | `wallet/wallet.go:3041` | `GetTransactions`, `GetTickets*` |
| `TicketStatus*` | 0…8 (§3.3) | `wallet/wallet.go:2851–2886` | gRPC ticket status mapping, `ticketinfo` status strings |
| `CreditReceive/Generate/Immature` strings | `receive` / `generate` / `immature` | `wallet/wallet.go:2361–2372` | `category` |
| send category | `"send"` | `wallet/wallet.go:2506` | `category` |
| `LTTTRegular/Ticket/Vote/Revocation` | `regular` / `ticket` / `vote` / `revocation` | `rpc/jsonrpc/types/results.go:201–213` | `txtype` |
| `udb.DefaultAccountNum` | `0` | `wallet/udb/addressmanager.go:53` | `ListUnspent` fallback account name |
| `udb.MaxAccountNum` | `0x7FFFFFFE` | `wallet/udb/addressmanager.go:36` | account-number classification |
| `udb.ImportedAddrAccount` | `0x7FFFFFFF` | `wallet/udb/addressmanager.go:47` | account for imported keys/scripts; `AccountProperties` special case |
| `udb.ImportedAddrAccountName` | `"imported"` | `wallet/udb/addressmanager.go:50` | reserved account name |
| first xpub/voting imported account number | `0x80000000` | `wallet/udb/addressdb.go:724–741`, `wallet/udb/addressmanager.go:1399–1403`, `:2278–2282` | `ImportXpubAccount`, `ImportVotingAccount` |
| `udb.ExternalBranch` / `udb.InternalBranch` | `0` / `1` | `wallet/udb/addressmanager.go:80`, `:85` | `deriveBranches` |
| `hdkeychain.MinSeedBytes` / `MaxSeedBytes` | `16` / `64` | `dcrd/hdkeychain/v3@v3.1.3/extendedkey.go:40–44` | `votingXprivFromSeed` length check |
| BIP0044 purpose | `44'` | `wallet/udb/addressmanager.go:2678` | voting xpriv path |
| `gapLimit` | wallet configuration (`wallet.DefaultGapLimit` = 20, `wallet/addresses.go:385`) | `wallet/wallet.go:125` | number of addresses watched and `lastWatched` on account import |
| address buffer initial state | `lastUsed = 0xFFFFFFFF`, `cursor = 0`, `lastWatched = gapLimit − 1` | `wallet/wallet.go:3876–3881`, `:3936–3941` | address generation after import |
| WIF scheme byte written | `0` (`dcrec.STEcdsaSecp256k1`) | `wallet/wallet.go:3695–3696`, `dcrd/dcrec@v1.0.1` | `dumpprivkey` output |
| compressed pubkey length / format bytes | `33`; `0x02`, `0x03` | `wallet/udb/addressmanager.go:1306–1316` | `ImportPublicKey` |
| `DCP0001` activation heights | mainnet 149248; testnet2 46128; testnet3 0; simnet 0; others inactive | `deployments/deployments.go:32–37`, `:59–73` | whether `StakeInfo*` fills `Sdiff` |
| mempool kind string | `"tickets"` | `wallet/wallet.go:4121` | dcrd `getrawmempool false "tickets"` |
| JSON-RPC defaults that reach this scope | `listtransactions` count 10, from 0; `listunspent` minconf 1, maxconf 9999999; `listsinceblock` target confirmations 1 | `internal/rpc/jsonrpc/methods.go:3213–3220`, `:3306–3314`, `:3143` | argument values (owned by the JSON-RPC spec) |

Chain parameters consumed (`dcrd/chaincfg/v3@v3.3.0`):

| Parameter | mainnet | testnet3 | simnet | regnet | Used by |
|---|---|---|---|---|---|
| `CoinbaseMaturity` | 256 | 16 | 16 | 16 | `coinbaseMatured` |
| `TicketMaturity` | 256 | 16 | 16 | 16 | `ticketMatured`, expiry thresholds |
| `TicketExpiry` (`TicketExpiryBlocks()`) | 40960 | 6144 | 384 | 384 | `makeTicketSummary`, `StakeInfoPrecise`, `ticketExpired` |
| `SStxChangeMaturity` | 1 | 1 | 1 | 1 | `ticketChangeMatured` |
| `PrivateKeyID` | `22de` | `230e` | `2307` | `22fe` | WIF |
| `SLIP0044CoinType` | 42 | 1 | 1 | 1 | voting xpriv path |
| `LegacyCoinType` | 20 | 11 | 115 | 1 | derived and discarded in `votingXprivFromSeed` |

Maturity thresholds spelled out (tip height `T`, transaction height `h ≥ 0`):

| Predicate | True when | mainnet example |
|---|---|---|
| `coinbaseMatured` | `T − h + 1 > CoinbaseMaturity` | `T ≥ h + 256` |
| `ticketMatured` | `T − h > TicketMaturity` | `T ≥ h + 257` |
| `ticketChangeMatured` | `T − h + 1 > SStxChangeMaturity` | `T ≥ h + 1` |
| `ticketExpired` | `T − h > TicketMaturity + TicketExpiry` | `T ≥ h + 41217` |
| revocation classified "expired" | `h_rev − h ≥ TicketExpiry + TicketMaturity` | `h_rev ≥ h + 41216` |

---

## 7. Concurrency and lifecycle

**Goroutines.** Only `StakeInfoPrecise` starts one (the `MempoolCount` RPC,
`wallet/wallet.go:4119–4127`). It runs concurrently with the database View and
is joined with `errgroup.Group.Wait` only on the success path of the View. On
a View error the function returns immediately and the goroutine finishes on
its own when the RPC returns or `ctx` is cancelled; it then writes
`res.AllMempoolTix` on a struct nobody reads.

**Mutexes touched and acquisition order.**

| Function | Order |
|---|---|
| `Accounts` | `lockedOutpointMu` (held for the whole call) → database read transaction → `manager.mtx` (read lock, inside `AccountProperties`) |
| `ListUnspent` | database read transaction → `lockedOutpointMu` (taken and released once per candidate output through `LockedOutpoint`) and `manager.mtx` (inside `Address`) |
| `LockedOutpoints` | `lockedOutpointMu` (released) → database read transaction |
| `LockedOutpoint`, `LockOutpoint`, `UnlockOutpoint`, `ResetLockedOutpoints` | `lockedOutpointMu` only |
| `LoadPrivateKey`, `DumpWIFPrivateKey` | database read transaction → `manager.mtx` (write lock) |
| `ImportPrivateKey`, `ImportPublicKey` | database write transaction → `manager.mtx`; after commit: `networkBackendMu` (momentarily) → backend `LoadTxFilter` → `NtfnServer.mu` |
| `ImportScript` | database write transaction → `manager.mtx` (released) → `networkBackendMu` (momentarily) → backend `LoadTxFilter`, all before commit |
| `ImportVotingAccount` | database write transaction → `manager.mtx`; after commit: backend `LoadTxFilter` → `addressBuffersMu` |
| `ImportXpubAccount` | backend `LoadTxFilter` → database write transaction → `manager.mtx`; after commit: `addressBuffersMu` |
| `SortedActivePaymentAddresses`, `hasVotingAuthority` | database read transaction → `manager.mtx` |

`Accounts` and `ListUnspent` take `lockedOutpointMu` and the database read
transaction in opposite orders (Q-16). `Accounts` never reads the locked set;
holding the mutex only serialises it against the other holders of the same
mutex — transaction construction (`wallet/createtx.go`), chain switches and
transaction acceptance (`wallet/chainntfns.go:110`, `:341`), mixing
(`wallet/mixing.go`), rescans (`wallet/rescan.go:265`), UTXO queries
(`wallet/utxos.go:25`) and publishing (`wallet/wallet.go:5132`) — so its
balances are never computed in the middle of one of those operations.

**Database transactions.** Every read function uses exactly one
`walletdb.View`, so each result is a consistent snapshot, with these
exceptions: `LockedOutpoints` snapshots the lock set before opening the
transaction; `ListUnspent` reads the lock set live during the transaction;
`ListSinceBlock` uses a caller-supplied tip height. `walletdb.View` does not
itself observe `ctx`; cancellation is honoured only inside
`RangeTransactions` (once per block and once per unmined record), in the
backend `LoadTxFilter` call, and in the dcrd RPC.

**Callbacks.** `GetTransactions`, `GetTickets` and `GetTicketsPrecise` invoke
the caller's `f` while the read transaction is open. gRPC streams responses
from inside `f` (`internal/rpc/rpcserver/server.go:1423–1443`, `:1507–1537`),
so a slow client keeps a bbolt read transaction open for the duration of the
stream.

**Notifications.** `notifyAccountProperties` holds `NtfnServer.mu` and
performs a blocking send to every registered account-notification client on
an unbuffered channel (`wallet/notifications.go:600–624`).
`ImportPrivateKey`/`ImportPublicKey` therefore block until all such clients
have received.

**Ordering guarantees callers observe.**

- For imports: database commit happens-before filter load (private key,
  public key, voting account); filter load happens-before commit (script,
  inside the transaction; xpub account, before the transaction). The
  in-memory address buffer for a new account is installed last.
- Locked outpoints are process-local and vanish on restart.
- `f` callbacks are sequential, never concurrent.

**Unchecked preconditions (Go panics rather than returning an error).** None
of these is reachable from the RPC servers with a well-formed database, but a
port must not turn them into silent wrong answers:

| Site | Precondition | Citation |
|---|---|---|
| `BlockInfo` | `blockID != nil` (dereferenced unconditionally) | `wallet/wallet.go:2758` |
| `LockedOutpoint`, `LockOutpoint`, `UnlockOutpoint` | `txHash != nil` (copied by value into the key) | `:4251`, `:4261`, `:4270` |
| `ListAddressTransactions` | every credit's `Index < len(TxOut)` | `:2627` |
| `hasVotingAuthority` | the transaction has at least one output | `:3989` |
| `StakeInfo`, `StakeInfoPrecise` | a vote spender has at least one input (`TxIn[0].ValueIn`; guaranteed by `IsSSGen`) | `:4066`, `:4183` |
| `GetTransactions`, `GetTickets*` range callbacks | `len(details) > 0` (guaranteed by `RangeTransactions`, `wallet/udb/txquery.go:518–520`) | `:3138`, `:3212`, `:3272` |
| `ImportXpubAccount`, `ImportVotingAccount` | `xpub`/`xpriv` non-nil | `:3895`, `:3854` |
| `StakeInfoPrecise`, `GetTicket*Precise` | `rpc` non-nil for `StakeInfoPrecise` (it is called); the ticket functions only compare it with nil | `:4121` |
| `LockedOutpoints` | the only site with an explicit bounds check (`int(op.index) >= len(TxOut)` → skip) | `:4315` |

`StakeInfoPrecise` stores the mempool count with `uint32(int)` (plain
truncating conversion, `:4125`). `gapLimit` is `uint32`
(`wallet/wallet.go:125`); `lastWatched = gapLimit − 1` wraps to `0xFFFFFFFF`
if the wallet was configured with a gap limit of 0 (configuration validation
is owned by the config spec).

Installing an address buffer (`w.addressBuffers[account] = …`) overwrites any
existing entry for that account number without checking; it cannot collide in
practice because the database assigns a fresh number in the same call.

**Lifecycle.** No function in scope has start/stop state. All depend on an
open database and a loaded address manager; `NetworkBackend()` may legitimately
return an error (no backend), in which case filter loads are silently skipped.

---

## 8. Errors

Errors are `*errors.Error` values built with `errors.E` (`errors/errors.go`).
The string form joins `Op`, `Kind.String()` and the wrapped error with `": "`,
and nested `*Error` values with `errors.Separator`. The package default is
`":\n\t"` (`errors/errors.go:27`), but the dcrwallet binary overrides it to
`":: "` in an `init` function (`dcrwallet.go:36–39`), so that is the separator
in every error string an RPC client or log line of the daemon shows. An `Op`
or `Kind` equal to the one last written is not repeated
(`errors/errors.go:247–290`). When an outer `errors.E(op, err)` wraps an inner
`*Error` that has no `Op` of its own, the two collapse into one error carrying
the outer op and the inner kind (`errors/errors.go:217–233`).

Kinds are mapped by the servers, not here: gRPC through `translateError`
(`internal/rpc/rpcserver/server.go:80–125`: `Invalid`/`Passphrase`/`Seed` →
`InvalidArgument`, `Exist` → `AlreadyExists`, `NotExist` → `NotFound`,
`Crypto` → `DataLoss`, `Locked` → `FailedPrecondition`, `WatchingOnly` →
`Unimplemented`, `NoPeers` → `Unavailable`, …); JSON-RPC through its own
conversion (JSON-RPC spec).

| Function | Op string | Errors raised or passed through |
|---|---|---|
| `ListSinceBlock` | `wallet.ListSinceBlock` | store I/O errors; `ctx.Err()` from `RangeTransactions` |
| `ListTransactions` | `wallet.ListTransactions` | same |
| `ListAddressTransactions` | `wallet.ListAddressTransactions` | same |
| `ListAllTransactions` | `wallet.ListAllTransactions` | same |
| `ListTransactionDetails` | `wallet.ListTransactionDetails` | `NotExist` (no message) when the hash is unknown; I/O |
| `BlockInfo` | `wallet.BlockInfo` | `NotExist` `"no block at height <h> in main chain"`; `NotExist` `"block header"` |
| `TransactionSummary` | `wallet.TransactionSummary(<hash>)` | `NotExist`; I/O |
| `GetTicketInfoPrecise` | `wallet.GetTicketInfoPrecise` | `NotExist` (unknown hash); `"<hash> is not a ticket"` (kind `Other`); `"<err> while trying to get ticket details for txhash: <hash>"` (kind `Other`); `NotExist` `"block header"`; header decode error |
| `GetTicketInfo` | `wallet.GetTicketInfo` | same |
| `GetTicketsPrecise` | `wallet.GetTicketsPrecise` | `blockRange` errors; `"<err> while trying to get ticket details for txhash: <hash>"`; `NotExist` `"block header"`; whatever `f` returns; `ctx.Err()` |
| `GetTickets` | `wallet.GetTickets` | same |
| `GetTransactions` | `wallet.GetTransactions` | `blockRange` errors; `NotExist` `"block header"`; header decode error; whatever `f` returns; `ctx.Err()` |
| `Spender` | none | §5.19 table |
| `UnspentOutput` | none | `NotExist` `"no unspent output <outpoint>"`; I/O |
| `Accounts` | `wallet.Accounts` | I/O; `"unknown account type <T>"` from `AccountProperties` |
| `ListUnspent` | `wallet.ListUnspent` | `NotExist` from `TxDetails`, from `AccountName(0)` (`"no account 0"`), from `RedeemScript` (address row missing); `Invalid` `"redeem script lookup requires P2SH address"`; `"invalid address count for pay-to-script-hash output"` (kind `Other`); non-`NotExist` errors from `manager.Address` |
| `LoadPrivateKey` | `wallet.LoadPrivateKey` | `NotExist` `"no address with id <hex hash160>"` (address unknown, `wallet/udb/addressdb.go:1326–1333`); lock-state errors from key derivation (`Locked`, …); `Crypto` `"decrypt imported privkey: <err>"`; `Invalid` `"no private key for P2SH address"`; `Invalid` `"address id cannot be created from type <T> (requires Hash160 method)"` |
| `DumpWIFPrivateKey` | `wallet.DumpWIFPrivateKey` (outer) wrapping `wallet.LoadPrivateKey` | as above |
| `ImportPrivateKey` | `wallet.ImportPrivateKey` | `Locked`; `Exist` `"address for private key already exists"`; `Crypto`; backend `LoadTxFilter` error |
| `ImportPublicKey` | `wallet.ImportPublicKey` | `Invalid` `"public keys may only be imported by watching-only wallets"`; `Encoding` `"invalid length for compressed pubkey"`; `Encoding` `"invalid format byte for compressed pubkey"`; `Exist` `"address for public key already exists"`; `Crypto`; backend error |
| `ImportScript` | `wallet.ImportScript` | `Exist` `"script already exists"`; `Crypto` `"encrypt script hash: <err>"`; backend error |
| `votingXprivFromSeed` | `wallet.VotingXprivFromSeed` only for the length check | `Invalid` `"invalid seed length"`; `Seed` wrapping `hdkeychain.ErrUnusableSeed` (no op); raw `hdkeychain` errors (no op) |
| `ImportVotingAccount` | `wallet.ImportVotingAccount` | `Passphrase` `"passphrase must be specified"`; `Invalid` `"accounts may not be named the empty string"` / `"reserved account name"` / `"extended key must be an xpriv"`; `Exist` `"account named \"<name>\" already exists"` / `"address belonging to this key already exists in the database"`; `Invalid` `"undable to derive external branch"` / `"undable to derive index"` (sic; inside the manager the string argument replaces the `hdkeychain` error, `wallet/udb/addressmanager.go:2262–2269`); `Crypto`; raw `hdkeychain` child errors from the post-commit `deriveBranches`/`deriveChildAddresses`; backend error |
| `ImportXpubAccount` | `wallet.ImportXpubAccount` | `"extended key must be an xpub"` (kind `Other`); `Invalid` name errors; `Exist` `"account name in use"`; `Invalid` `"exhausted possible imported accounts"`; `Crypto`; `hdkeychain` child errors; backend error |
| `StakeInfo` | `wallet.StakeInfo` | `NotExist` `"block header"`; difficulty errors; `NotExist` `"tx <hash> not found"`; `IO` `"ticket spender <hash> is neither vote nor revocation"`; iterator I/O |
| `StakeInfoPrecise` | `wallet.StakeInfoPrecise` | same, except that a missing spender is `NotExist` with no message (it is loaded with `TxDetails`, not `Tx`); plus `dcrd.MempoolCount` (`getrawmempool`) errors, which carry the inner op `dcrd.MempoolCount` |
| `LockedOutpoints` | none | non-`NotExist` errors from `TxDetails` |
| `UnminedTransactions` | `wallet.UnminedTransactions` | I/O |
| `SortedActivePaymentAddresses` | `wallet.SortedActivePaymentAddresses` | `IO`-wrapped row decode errors; managed-address construction errors |

Behaviour that depends on these kinds in the servers (must keep working):

- `listunspent` maps any `NotExist` from `ListUnspent` to its "address not in
  wallet" error (`internal/rpc/jsonrpc/methods.go:3317–3321`).
- `importprivkey`, `importpubkey`, `importscript` (JSON-RPC) treat `Exist` as
  success and return `null` (`methods.go:2043–2046`, `:2103–2107`, `:2157–2159`);
  `importprivkey` maps `Locked` to the wallet-unlock-needed error; gRPC
  `ImportScript` also swallows `Exist` (`server.go:770–773`).
- `dumpprivkey` maps `Locked` to the wallet-unlock-needed error
  (`methods.go:871–874`).
- `gettxout` (SPV path) treats `NotExist` from `UnspentOutput` as a `null`
  result (`methods.go:2752–2758`).
- gRPC `GetTicket`/`GetTickets` pass errors from `GetTicketInfo*`/
  `GetTickets*` through `translateError` (`NotExist` → `NotFound`). The
  explicit `errors.Is(err, errors.NotExist)` tests in those handlers
  (`internal/rpc/rpcserver/server.go:1478`, `:1519`) apply to the error of
  `VSPHostForTicket` (part 3), not to a function in this scope.

No function in scope terminates the process or sets an exit code. Log output
produced in scope: two info lines (`Imported payment address <addr>`,
`Imported script with P2SH address <addr>`) on the wallet logger, and error
lines from `makeTxSummary` helpers (`Cannot query previous transaction details
for <hash>: <err>`, `Missing previous transaction <hash>`, `Cannot fetch
account for previous output <outpoint>: <err>`, `Cannot fetch account for
wallet output: <err>`, `Transaction serialization: <err>`;
`wallet/notifications.go:68–121`).

---

## 9. Compatibility-critical behaviours

Each item is observable by an external party named in brackets.

1. **`listtransactions` paging and order** [dcrctl, Decrediton, exchanges
   polling over JSON-RPC]. `from` and `count` count transactions, newest
   first; the result is oldest-first; within a transaction receive entries
   precede send entries and each group is in *descending* `vout` order
   (§5.3). `count <= 0` yields `[]`.
2. **`listalltransactions` order** [JSON-RPC clients]: as item 1 without paging.
3. **`listsinceblock` order and inclusivity** [JSON-RPC clients]: unmined
   transactions first (ascending raw-hash order), then blocks from the tip
   down to and **including** the block named by the request; within a
   transaction receives then sends, each ascending by `vout` (§5.2).
4. **`gettransaction.details` order** [JSON-RPC clients]: receives ascending
   by `vout`, then sends ascending by `vout` (§5.5).
5. **`listaddresstransactions` matching rule and order** [JSON-RPC clients]:
   only version-0 plain P2PKH credits match; all entries of a matching
   transaction are returned; blocks ascending then unmined (§5.4).
6. **Entry field population** [JSON-RPC clients]: exactly the presence/absence
   rules of §4.1 — `fee` on every `send` entry (including `0`), never on
   receive entries; `account` always present and empty on `send`; `txtype`
   always present; `walletconflicts` always `[]`; `blockhash`/`blocktime`
   absent when unmined; `generated` only when true; `address` only when the
   script decodes to exactly one address.
7. **Change outputs are omitted, own non-change outputs are doubled**
   [JSON-RPC clients]: §5.1.
8. **Negated fee and negated send amount** [JSON-RPC clients]: `fee` is
   `(outputs − debits)/1e8`; send `amount` is the negation of the output
   value, including IEEE negative zero for zero-value outputs.
9. **Category rules** [JSON-RPC clients]: `generate`/`immature` for any
   transaction with a single null-prevout input (coinbase, treasurybase,
   treasury spend), switched by `coinbaseMatured`; otherwise `receive`.
10. **Confirmation arithmetic** [all RPC clients]: `confirms` returns 0 for
    unmined or above-tip heights, else `tip − h + 1`. `listunspent` bounds
    are inclusive on both sides.
11. **Maturity rules in `listunspent`** [JSON-RPC clients, ticket buyers]:
    coinbase credits need `coinbaseMatured`; ticket output 0 needs
    `ticketMatured`; even-index ticket outputs > 0 need `ticketChangeMatured`;
    all vote and revocation outputs need `coinbaseMatured`; locked outpoints
    are excluded (§5.12).
12. **`listunspent` ordering** [JSON-RPC clients]: unmined first by
    descending receive time, then descending block height; same-transaction
    outputs by descending index (§5.12 step 5). Tie order: see Q-13.
13. **`listunspent` returns JSON `null` when empty** [JSON-RPC clients]: the
    Go function returns a nil slice (§5.12 step 8).
14. **`listunspent` account/address semantics** [JSON-RPC clients]: account
    is the name of the first address's account with fallback to the *current
    name of account 0*; the address filter matches any address of the script
    but `address` reports only the first; `spendable` and `redeemScript` per
    the table in §5.12.
15. **Account enumeration order** [gRPC `Accounts`, Decrediton]: ascending
    bytewise order of the little-endian account number (Q-1). `lockaccount *`
    locks accounts in this order.
16. **Account total balance definition** [gRPC `Accounts`]: §5.11 — includes
    immature, unconfirmed and locked outputs; excludes outputs spent by
    unmined transactions and outputs of unpublished transactions.
17. **Ticket status derivation** [gRPC `GetTicket`/`GetTickets`, JSON-RPC
    `ticketinfo`]: the table in §5.9, including the `≥ expiry + maturity` rule
    separating `expired` from `missed`. Status strings per §3.3. The
    `live` (dcrd RPC backend) vs `unspent` (SPV) distinction exists only in
    the wallet-level `TicketSummary.Status`; it does **not** reach gRPC
    clients: the gRPC enum has no `UNSPENT` member (`rpc/api.proto:410–419`)
    and `marshalTicketDetails` has no case for `TicketStatusLive` or
    `TicketStatusUnspent`, so both are sent as the default `LIVE`
    (`internal/rpc/rpcserver/server.go:2281–2298`). JSON-RPC
    `ticketinfo` always calls the non-precise `GetTickets` (`rpc == nil`), so
    it reports `unspent`, never `live`, with either backend, and it rewrites
    `unmined` to `immature` before formatting
    (`internal/rpc/jsonrpc/methods.go:4419–4427`).
18. **Ticket stream grouping** [gRPC `GetTickets`]: one callback per block
    with at least one ticket, block-record order inside a block, header nil
    for unmined; iteration direction per §5.2.
19. **`GetTransactions` grouping and direction** [gRPC `GetTransactions`,
    JSON-RPC `auditreuse`]: one callback per block with wallet transactions,
    header nil for the single unmined batch; direction and unmined placement
    per §5.2 for the heights produced by `blockRange`.
20. **Block identifier defaults** [gRPC]: nil start = height 0, nil end =
    "through tip plus unmined"; a hash identifier contributes only its
    header's height.
21. **`BlockInfo` for side-chain blocks** [gRPC `BlockInfo`, JSON-RPC]:
    confirmations 0 and `StakeInvalidated` false when the block is not the
    main-chain block at its height.
22. **`TransactionSummary` block hash rule** [gRPC `GetTransaction`]: the
    block hash is returned only when confirmations > 0.
23. **WIF export format** [dcrctl `dumpprivkey`, gRPC `DumpPrivateKey`, other
    wallets importing the key]: §4.5; scheme byte always 0.
24. **Import side effects** [existing `wallet.db`, dcrd/SPV peers, gRPC
    notification clients]: imported keys/scripts go to account `0x7FFFFFFF`;
    the address is added to the transaction filter with `reload = false`;
    key imports emit an account notification with the imported account's
    properties, script imports do not; duplicate imports fail with kind
    `Exist`.
25. **Imported xpub / voting account numbering and watch set** [existing
    `wallet.db`, dcrd/SPV]: numbers start at `0x80000000`; xpub import
    watches `gapLimit` external + `gapLimit` internal addresses; voting
    import watches `gapLimit` external addresses only.
26. **Voting xpriv derivation path** [VSPs and other wallets sharing a
    voting seed]: `m/44'/<SLIP0044CoinType>'/0'` using Decred's
    `hdkeychain.Child` derivation; the test vector in §11 pins it.
27. **`getstakeinfo` / gRPC `StakeInfo` field semantics** [dcrctl,
    Decrediton]: §5.17 including which fields are zero in SPV mode, `Live ==
    Unspent` in RPC mode, `PoolSize` taken from the wallet's tip header,
    `TotalSubsidy` summed from `TxIn[0].ValueIn` of votes, and `Sdiff` zero on
    networks without a hardcoded DCP0001 height.
28. **dcrd request made by `StakeInfoPrecise`** [dcrd]: exactly one
    `getrawmempool` with parameters `false, "tickets"` per call.
29. **Locked outpoints** [JSON-RPC `lockunspent`/`listlockunspent`,
    transaction creation]: keyed by `(hash, index)` without tree; never
    validated on lock; not persisted; `listlockunspent` silently drops locks
    on unknown transactions or out-of-range indexes; `amount` omitted when
    zero; `tree` is 1 for any non-regular transaction type; the account
    filter treats `""` and `"*"` as "all".
30. **`UnminedTransactions` content** [dcrd, SPV peers at startup
    rebroadcast]: all unmined transactions except those flagged unpublished,
    parents before children.
31. **`SortedActivePaymentAddresses`** [JSON-RPC `listreceivedbyaddress`]:
    every address row, bytewise ascending by encoded string.
32. **Error kinds the servers branch on** [all RPC clients]: §8 list
    (`Exist` swallowed by imports, `NotExist` → null / address-not-in-wallet,
    `Locked` → unlock-needed).
33. **Log lines** [operators, log scrapers]: `Imported payment address
    <addr>` and `Imported script with P2SH address <addr>` at info level on
    the wallet subsystem logger.
34. **Precise/non-precise selection** [gRPC `GetTicket`, `GetTickets`,
    `StakeInfo`; JSON-RPC `getstakeinfo`]: precise if and only if the active
    network backend is the dcrd RPC syncer; otherwise (SPV or no backend)
    non-precise; `ticketinfo` always non-precise (§2).
35. **`listsinceblock` without a block hash lists everything** [JSON-RPC
    clients]: the handler passes `(start, end) = (-1, 0)`; with a hash it
    passes `(-1, height of that header)`, whether or not the header is on the
    main chain (§5.2).
36. **Ticket ownership test in staking statistics** [dcrctl `getstakeinfo`,
    Decrediton]: a ticket is counted only if the first hash160 address of its
    output 0 is an address row of this wallet (any account, any row type,
    private key held or not); tickets the wallet merely funded (commitment
    outputs) are not counted (§5.17).
37. **Unmined stake outputs never appear in `listunspent`** [JSON-RPC
    clients]: unmined ticket output 0, unmined ticket even-index outputs and
    all outputs of unmined votes/revocations are excluded even with
    `minconf = 0` (§5.12).
38. **`getstakeinfo` mempool count width** [dcrctl]: `AllMempoolTix` is the
    number of hashes dcrd returned, converted to `uint32`.
39. **dcrd request made by imports** [dcrd]: with the dcrd RPC backend each
    of `ImportPrivateKey`, `ImportPublicKey`, `ImportScript`,
    `ImportVotingAccount` and `ImportXpubAccount` sends exactly one
    `loadtxfilter` request with parameters `false, [<addresses>], []`; the
    address order is fixed (§5.14). No request is sent when no backend is
    set, and the SPV backend sends nothing to peers.
40. **Account notification for key imports** [gRPC `AccountNotifications`
    subscribers]: one notification per successful `ImportPrivateKey` /
    `ImportPublicKey` with account `0x7FFFFFFF`, name `imported`, external
    and internal key counts `0`, and the post-import imported-row count
    (§5.14). None for script, xpub-account or voting-account imports made
    through this scope.
41. **Address form per script class** [JSON-RPC clients]: the `address` and
    `account` fields and the `listunspent` address filter follow the
    extraction table of §5.21 — in particular pubkey-form addresses for
    P2PK and multisig keys, no address for multi-key multisig in
    `listtransactions`, and first-address attribution in `listunspent` and
    account balances.
42. **Status values consumed inside the wallet** [VSP fee payment, ticket
    processing]: `ForUnspentUnexpiredTickets` (`wallet/wallet.go:5705–5747`,
    part 3) selects tickets by `Status ∈ {Live, Immature, Unspent}` from
    `GetTickets`; the `makeTicketSummary` table (§5.9) therefore decides
    which tickets are offered to VSP processing, and `Unmined` tickets are
    never offered.

---

## 10. Quirks

**Q-1. Account iteration is in little-endian byte order, not numeric order.**
`forEachAccount` iterates bucket `acct`, whose keys are
`binary.LittleEndian` account numbers (`wallet/udb/addressdb.go:269–273`,
`:698–708`). Accounts 0–255 come out in numeric order only as long as no
account ≥ 256 and no imported xpub/voting account exists. Example with
accounts 0, 1, 2, 256, `0x7FFFFFFF` (imported), `0x80000000`, `0x80000001`:

| Position | Key bytes | Account |
|---|---|---|
| 1 | `00 00 00 00` | 0 |
| 2 | `00 00 00 80` | `0x80000000` (first imported xpub/voting account) |
| 3 | `00 01 00 00` | 256 |
| 4 | `01 00 00 00` | 1 |
| 5 | `01 00 00 80` | `0x80000001` |
| 6 | `02 00 00 00` | 2 |
| 7 | `ff ff ff 7f` | `0x7FFFFFFF` (imported) |

Visible in gRPC `Accounts` and in the order `lockaccount *` works through
accounts. `TotalReceivedForAccounts` (part 3) uses the same iterator.

**Q-2. `listtransactions` paging units.** `from` skips transactions even if
they would produce no entries; `count` limits transactions that produce at
least one entry; the returned array length is unrelated to `count`
(`wallet/wallet.go:2572–2590`). The code comment says "skip the first from
transactions … include the next count transactions", which matches the code
but not the Bitcoin-derived expectation of counting entries.

**Q-3. Negative zero in JSON.** A `send` entry for a zero-value output
(ticket commitment outputs, vote `OP_RETURN` outputs, null-data outputs) has
`Amount = -amountF64` with `amountF64 == 0.0`, which is IEEE `-0.0`
(`wallet/wallet.go:2507`); Go's JSON encoder writes `-0`. See §4.4 for the
verification note.

**Q-4. `fee` is present with value 0 when it cannot be determined.** `Fee`
points at `feeF64` on every send entry, and `feeF64` stays `0.0` when not all
inputs are wallet debits (`wallet/wallet.go:2426–2440`, `:2508`). All send
entries of one transaction repeat the same whole-transaction fee.

**Q-5. Treasurybases and treasury spends are "coinbase".**
`compat.IsEitherCoinBaseTx` ORs the pre- and post-treasury coinbase tests;
the pre-treasury test accepts any single-input transaction with a null
previous outpoint. Outputs the wallet receives from a treasury spend are
therefore reported with `generated: true` and category `immature`/`generate`,
and `TxTransactionType` reports `Coinbase` for them.

**Q-6. `txtype` has four values.** `TxTypeTAdd`, `TxTypeTSpend` and
`TxTypeTreasuryBase` map to `"regular"` in `listTransactions`
(`wallet/wallet.go:2415–2423`), while `ListUnspent.TxType` exposes the raw
`stake.TxType` integer (0–6).

**Q-7. `ListAddressTransactions` ignores non-P2PKH credits.** It tests
`ExtractPubKeyHashV0` only (`wallet/wallet.go:2627–2633`). The JSON-RPC
handler nevertheless inserts the hash160 of any `Hash160er` address,
including P2SH (`internal/rpc/jsonrpc/methods.go:3242–3256`); the set is only
ever compared against plain P2PKH script hashes, so a P2SH address never
selects its own outputs. Stake-tagged outputs paying a requested address
(ticket submission, vote/revocation payouts, stake change) do not match
either.

**Q-8. `listsinceblock` includes the named block.** `RangeTransactions(-1,
height)` is inclusive of `height`; the handler's own TODO notes it should
start at the fork point (`internal/rpc/jsonrpc/methods.go:3164–3179`).

**Q-9. "Precise" ticket queries never call dcrd.** `makeTicketSummary` uses
`rpc` only as a nil test (`wallet/wallet.go:2899–2901`). The doc comments on
`GetTicketInfoPrecise`/`GetTicketsPrecise` claim the RPC client lets the
wallet "determine whether a ticket has been missed"; the code derives
missed/expired purely from a recorded revocation.

**Q-10. A ticket spent by an unmined vote or revocation is reported
unspent.** `TicketDetails` and `TicketIterator` consult the unmined-inputs
bucket only when *no* credit exists for output 0; when the credit exists and
is not spent by a mined transaction the spender is treated as absent
(`wallet/udb/txquery.go:221–234`, `wallet/udb/txmined.go:2800–2817`). Status
stays `live`/`unspent` and `StakeInfo` counts the ticket under `Unspent`
until the spender is mined.

**Q-11. Unmined revocation classified as "missed".** In the branch where the
spender is found through the unmined-inputs bucket and is a revocation, its
height is `-1`, so `revocationHeight − ticketHeight` is negative and the
status is `missed` (`wallet/wallet.go:2917–2928`).

**Q-12. Two different "expired" thresholds.** `makeTicketSummary` and
`StakeInfoPrecise` use `h_rev − h ≥ expiry + maturity`; `ticketExpired`
(used for `UnspentExpired`) uses `tip − h > maturity + expiry`
(`wallet/wallet.go:2921–2923`, `:4193–4195`, `:4444–4447`).

**Q-13. `listunspent` tie order depends on Go's sort.** `sort.Sort` is
unstable and `creditSlice.Less` reports equality for different transactions
at the same height (and for unmined transactions received in the same
second). The resulting order is deterministic for a given Go release and
input order (bucket key order) but is defined by Go's `sort` implementation,
not by the wallet (`wallet/wallet.go:3435–3470`, `:3491`). The doc comment
admits it: "not guaranteed to be sorted by the order they appear in the
block". UNVERIFIED: the exact algorithm of the Go release used for the
reference build was not read.

**Q-14. `ListUnspent` doc comment disagrees with code.** The comment says
confirmations are "more than minconf, less than maxconf" and that "an empty
array will be returned" when nothing is known (`wallet/wallet.go:3472–3476`).
The code uses inclusive bounds and returns a nil slice, which the JSON-RPC
server marshals as `null`.

**Q-15. `ListUnspent` only reports the first address.** Flagged in the source
with a `BUG:` comment (`wallet/wallet.go:3652–3657`). The address *filter*
considers all addresses of a multisig script, so an output can be returned
for an address that is not the one shown.

**Q-16. Lock-order inversion.** `Accounts` takes `lockedOutpointMu` then a
database read transaction; `ListUnspent` takes a read transaction then
`lockedOutpointMu` (`wallet/wallet.go:3379–3382`, `:3550`). `Accounts` does
not use the locked set at all. There is a deadlock window in the Go: writers
such as the chain-switch path take `lockedOutpointMu` and then open a write
transaction (`wallet/chainntfns.go:110–113`); a bbolt read transaction holds
the database's mmap read lock for its lifetime and a write transaction that
must grow the file takes the mmap write lock (`bbolt@v1.3.12/db.go:433`,
`:735`, `:835`). `ListUnspent` waiting on `lockedOutpointMu` with its read
transaction open, while the mutex holder's write transaction waits to remap,
would block both. The lock sites were verified; no reproduction was built.
A Rust port should take the lock-set snapshot before opening the read
transaction (as `LockedOutpoints` does) unless bug-for-bug behaviour is
wanted.

**Q-17. `ListUnspent` spendability gaps.** Ticket submission outputs
(`STStakeSubmission*`), treasury-gen outputs and any non-ECDSA-secp256k1
script report `spendable: false` even when the wallet holds the key; P2SH
redeem scripts are returned only for untagged `STScriptHash` (stake-tagged
script-hash outputs get none); a P2SH output whose redeem script is not in
the wallet aborts the *entire* call with `NotExist` (which the JSON-RPC
handler turns into "address not in wallet").

**Q-18. `listlockunspent` order is random.** The locked set is copied by Go
map iteration (`wallet/wallet.go:4289–4294`). There is no order to
reproduce; clients cannot depend on one.

**Q-19. Locks are not validated and ignore the tree.** `LockOutpoint`
accepts any hash/index; `LockedOutpoints` hides entries that do not resolve
(`wallet/wallet.go:4303–4318`). The request's `tree` is discarded by the
key type.

**Q-20. `ImportPrivateKey`/`ImportPublicKey` can fail after committing.** A
`LoadTxFilter` error is returned to the caller although the key is already in
the database (`wallet/wallet.go:3720–3729`); a retry then fails with `Exist`,
which the JSON-RPC handlers report as success while skipping the rescan
(`internal/rpc/jsonrpc/methods.go:2043–2046`, `:2103–2107`). No log line or
notification is produced for the failed first attempt.

**Q-21. `ImportScript` performs network I/O inside a write transaction** and
logs success before the commit (`wallet/wallet.go:3782–3800`). A failed
commit leaves the address in the backend's filter and the log line printed.

**Q-22. `ImportXpubAccount` loads the filter before writing.** If the
database update then fails (duplicate name), the backend keeps watching the
addresses (`wallet/wallet.go:3904–3932`). Conversely `ImportVotingAccount`
writes first; if `deriveBranches` or the filter load fails afterwards the
account exists in the database but has no in-memory address buffer until the
wallet is reopened.

**Q-23. `ImportVotingAccount` does not watch the internal branch.** By
design, per the source comment (`wallet/wallet.go:3861–3862`).

**Q-24. `ImportPrivateKey` does not check the WIF signature scheme.** A WIF
with scheme byte 1 (Ed25519) or 2 (Schnorr) is stored as a secp256k1-ECDSA
P2PKH key using whatever `wif.PubKey()` returns
(`wallet/udb/addressmanager.go:1253–1291`); neither RPC handler checks
`wif.DSA()` either. UNVERIFIED: the end-to-end result for an Ed25519 WIF
(what address is produced and whether later signing works); a Go oracle test
settles it.

**Q-25. `UnminedTransactions` order is only partially defined.**
`dependencySort` seeds its queue from a Go map (`wallet/udb/kahnsort.go:75–84`),
so unrelated transactions come out in random order; only parent-before-child
is guaranteed.

**Q-26. `votingXprivFromSeed` error wrapping is inconsistent.** Only the
length check carries the op and kind `Invalid`; `HDKeysFromSeed` errors are
returned as-is (`wallet/wallet.go:3817–3827`). Coin-type-level
`ErrInvalidChild` is not converted to kind `Seed`
(`wallet/udb/addressmanager.go:2830–2837`), and neither is a
`hdkeychain.NewMaster` failure (`:2823–2826`).

**Q-27. `ImportXpubAccount` uses a bare string error.**
`errors.E(op, "extended key must be an xpub")` has kind `Other`
(`wallet/wallet.go:3896`), so gRPC reports it with the default code rather
than `InvalidArgument`, unlike the equivalent xpriv check in the manager.

**Q-28. `GetTickets*` ignores a header decode error and reuses one header
object** (`wallet/wallet.go:3147`, `:3221`); `GetTransactions` does neither.

**Q-29. `StakeInfoPrecise` can return before its goroutine finishes**
(§7). The abandoned goroutine writes to a result struct that has been
dropped; in Go this is a benign data race on unreachable memory.

**Q-30. `StakeInfo` never reports missed/expired/live** and
`StakeInfoPrecise` never reports `UnspentExpired`; `getstakeinfo` therefore
shows `live: 0` and `proportionlive: 0` in SPV mode.

**Q-31. `Sdiff` is 0 on regnet.** `HardcodedDeployment.Active` has no case
for the regression-test network (`deployments/deployments.go:59–73`).

**Q-32. Imported account pseudo-properties.** For account `0x7FFFFFFF` the
four index fields are 0, not `0xFFFFFFFF`; gRPC `Accounts` adds 1 to the
last-returned indexes and so reports one external and one internal key for
the imported account (`wallet/udb/addressmanager.go:586–600`,
`internal/rpc/rpcserver/server.go:402–404`).

**Q-33. `BlockIdentifier` from height is not validated.** A negative height
passes straight to `RangeTransactions`, where any negative start means
"unmined first, then from the tip downward" and any negative end means "to
the tip, then unmined". gRPC forwards any non-zero request height
(`internal/rpc/rpcserver/server.go:1366`, `:1382`).

**Q-34. `GetTransactions` copies, `GetTickets*` aliases.** `GetTransactions`
copies the details slice because the store reuses it
(`wallet/wallet.go:3260–3264`); `GetTickets*` hands out summaries whose
`Hash` pointers alias the reused array. A Rust port with owned values has no
equivalent hazard; the observable results are the same.

**Q-35. gRPC `Spender` error mapping is dead code.** The handler calls
`errors.Is(errors.NotExist, err)` with the arguments reversed
(`internal/rpc/rpcserver/server.go:2155–2160`), so the intended `NotFound` /
`InvalidArgument` messages are never produced and the error falls through to
`translateError`, which yields the same codes with the wallet's own message.
Recorded here because `Wallet.Spender`'s unwrapped errors are what the client
actually sees; owned by the gRPC spec.

**Q-36. Dead/unused in scope.** `TicketStatusUnknown` and
`TicketStatusRevoked` are never assigned. `BlockIndex`,
`InvolvesWatchOnly`, `Comment` and `OtherAccount` are never set.
`confirmed` is used only by `wallet/createtx.go`. A duplicate `confirmed`/
`confirms` pair exists in `wallet/udb/txmined.go:2931–2945`.

**Q-37. Doc comments in scope that disagree with the code.** The code is
authoritative in every case; none changes behaviour, but a port written from
the comments would be wrong.

| Symbol | Comment says | Code does | Citation |
|---|---|---|---|
| `ListSinceBlock` | "If the block is -1 then all transactions are included" | `(-1, -1)` yields unmined transactions only; "everything" is `(-1, 0)` | `wallet/wallet.go:2522–2524`, §5.2 |
| `ListTransactions` / `ListAllTransactions` (inline) | reverse iteration "does nothing for unmined transactions, which are unsorted" | the unmined batch is in ascending raw-hash order and is walked in reverse, so paging over unmined transactions is deterministic (descending raw hash) | `:2568–2571`, `:2667–2671` |
| `ListAddressTransactions` | transactions "to or from any address belonging to a set" | only credits (outputs paying the wallet) are tested; a transaction that merely spends from a listed address does not match | `:2609–2611`, `:2626–2633` |
| `TicketStatusExpired` | "In SPV mode, this status may be used by unspent tickets definitely past the expiry period" | `makeTicketSummary` never assigns `Expired` without a recorded revocation; an unspent ticket past expiry stays `unspent` (SPV) or `live` (RPC). Only `StakeInfo.UnspentExpired` reflects it | `:2870–2874`, `:2888–2933` |
| `GetTicketInfoPrecise`, `GetTicketsPrecise` | mention an argument `chainClient` and say the RPC client lets the wallet tell missed from live | parameter is `rpc`; never called (Q-9) | `:2942–2946`, `:3097–3101` |
| `GetTransactions` | blocks "in ascending order"; unmined returned "on a Block structure with height == -1" | direction follows `(start, end)` (descending when `start ≥ end` or `start < 0`); `Block` has no height field — unmined is `Header == nil` (`wallet/notifications.go:386–389`) | `:3241–3244` |
| `Spender` | error "code ErrInput" for non-credits, "ErrNoExist" for unspent | kinds are `errors.Invalid` and `errors.NotExist` (§5.19) | `:3305–3307` |
| `Accounts` | names, numbers and total balances; tip included "for atomicity reasons" | also returns all other `AccountProperties` fields | `:3365–3370` |
| `creditSlice` | sorts "from oldest to newest" | only ever used through `sort.Reverse`, i.e. newest first | `:3435–3439`, `:3491` |
| `ListUnspent` | see Q-14 | | `:3472–3476` |
| `ImportPublicKey` (inline) | "Attempt to import private key", "payment address string of the imported private key" | copy-paste from `ImportPrivateKey`; imports a public key | `:3744`, `:3773` |
| `ImportScript` | "allows the user to specify whether or not they want the redeemscript to be rescanned, and how far back" | no rescan parameter exists; rescans are started by the RPC handlers | `:3777–3779` |
| `UnspentOutput` | "Mined transactions that are spent by a mempool transaction are not affected by this" | accurate; restated in §5.19 | `:3319–3322` |

`LoadPrivateKey` and `ImportXpubAccount` are exported without any doc comment.

**Q-38. Source annotations in scope.** Every `TODO`/`BUG`/`Deprecated`/
generator comment in lines 2394–4428:

| Line | Text (abridged) | Effect on a port |
|---|---|---|
| `:2393` | `TODO: This should be moved to the jsonrpc package.` (last line of the doc comment of `listTransactions`, whose declaration is at `:2394`) | none; explains why a wallet-package function returns JSON-RPC result structs |
| `:2849` | `//go:generate go tool stringer -type=TicketStatus -linecomment` | `TicketStatus.String()` values are the line comments (§3.3); the generated file is `wallet/ticketstatus_string.go` |
| `:2883` | `Deprecated:` on `TicketStatusRevoked` | value 8 stays reserved; never produced |
| `:3260` | `TODO: probably should make RangeTransactions not reuse the details backing array memory` | explains the defensive copy in `GetTransactions` (Q-34) |
| `:3369` | `TODO(jrick): Is the chain tip really needed, since only the total balances are included?` | `CurrentBlockHash`/`CurrentBlockHeight` must still be returned; gRPC `Accounts` exposes them |
| `:3592` | `TODO: Each case will need updates when watch-only addrs is added` | `spendable` ignores watching-only state: a watching-only wallet reports `spendable: true` for P2PKH/P2PK/P2SH outputs, and multisig checks only that each pubkey address *exists* in the manager, not that its private key is held |
| `:3652` | `BUG: this should be a JSON array …` | Q-15 |
| `:3558` | "This will be unnecessary once transactions and outputs are grouped under the associated account in the db" | the per-output account lookup with default-account fallback in `ListUnspent` must stay |
| `:4088`, `:4216` | pool size is "a 'good enough' estimate": the correct value would be the pool size mined into the *next* block | `PoolSize` is the tip header's field, not a prediction |
| `:4187` | revoked tickets are assumed expired unless revoked before the expiry height; "may not be accurate for tickets that were missed prior to the activation of DCP0009" | `StakeInfoPrecise.Expired`/`Missed` and the ticket status are a height heuristic, not a consensus fact: a ticket missed early but revoked after `expiry + maturity` blocks counts as expired |

**Q-39. Three copies of `confirms`.** Besides the pair in
`wallet/udb/txmined.go` (Q-36), the JSON-RPC server defines its own identical
`confirms` (`internal/rpc/jsonrpc/methods.go:75–82`) and uses it at
`methods.go:1281`, `:1406`, `:2664`, `:2778` and `:3088`. All three must stay arithmetically identical; a Rust
port should have one.

**Q-40. `hasVotingAuthority` reports no private key for imported voting
accounts.** `HavePrivateKey` decides by `account < ImportedAddrAccount` for
derived address rows (`wallet/udb/addressmanager.go:2540–2541`), so tickets
whose voting address belongs to an account imported with
`ImportVotingAccount` (number ≥ `0x80000000`) return `(true, false)` although
the wallet can sign for them once the account is unlocked. Not observable
through `StakeInfo*` (which ignore the second value); observable through the
caller in `wallet/chainntfns.go:759`. UNVERIFIED whether that caller's use of
the flag changes voting behaviour for imported voting accounts; reading
`wallet/chainntfns.go:740–790` settles it and belongs to the
chain-notification spec.

**Q-41. Small `listunspent` results are tie-stable in practice.** Addendum
to Q-13. In the Go toolchain available for inspection (go1.27, *not* the
`go 1.26.0` named in `go.mod`), `sort.Sort` runs a plain insertion sort when
the slice has at most 12 elements (`maxInsertion = 12` in
`sort/zsortinterface.go`), and `sort.Reverse` only swaps the arguments of
`Less`. An insertion sort never reorders elements that compare equal, so for
wallets with ≤ 12 unspent outputs (before filtering) ties keep the
`UnspentOutputs` order: bucket `u` ascending raw key, then bucket `mc`
ascending raw key. Above 12 elements the pattern-defeating quicksort applies
and tie order is implementation-defined. UNVERIFIED for go 1.26; reading
`sort/zsortinterface.go` of the reference toolchain settles it.

**Q-42. `ListAddressTransactions` matches every non-P2PKH credit if the set
contains the empty key.** `ExtractPubKeyHashV0` returns nil for a script
that is not plain P2PKH and the lookup key is `string(pkh)`, i.e. `""`
(`wallet/wallet.go:2630–2631`). A caller that put `""` into `pkHashes` would
select every transaction with a version-0 non-P2PKH credit. The JSON-RPC
handler only inserts 20-byte hash160 strings
(`internal/rpc/jsonrpc/methods.go:3242–3256`), so this is unreachable from
RPC; a port keyed by `[u8; 20]` removes the case, which is an acceptable
divergence.

**Q-43. A negative start height reaches `GetTickets` from inside the
wallet.** `ForUnspentUnexpiredTickets` (`wallet/wallet.go:5734–5741`, part 3)
calls `GetTickets` with start `tip − (TicketExpiry + TicketMaturity − 8)`
and end `tip`. While the chain is shorter than that span (simnet/regnet:
392 blocks; testnet3: 6152; mainnet: 41208) the start is negative, and per
§5.2 a negative start means "unmined batch first, then backward from
`0x7FFFFFFF` down to `end`": only the unmined tickets (which that caller
then discards by status) and the tickets of the tip block itself are
visited. Tickets in earlier blocks are not offered to the caller until the
chain grows past the span. Owned by part 3, recorded here because it is a
direct consequence of `blockRange` passing heights through unvalidated
(Q-33); test harnesses on short simnet chains observe it.

**Q-44. `walletdb.View` discards rollback errors; `walletdb.Update` reports
commit errors.** In `View` the deferred rollback assigns to a local `err`
that is not the function's result, so a failed rollback of a read
transaction is never seen (`wallet/walletdb/interface.go:203–223`). In
`Update` the commit error becomes the result (`:230–255`) and is wrapped
with the calling function's op like any other error; for `ImportPrivateKey`
and `ImportPublicKey` this means a failed commit returns before the filter
load, log line and notification, while for `ImportScript` the filter load
and log line have already happened (Q-21). Owned by the database spec.

---

## 11. Upstream tests

Coverage of this scope is thin. Nothing upstream tests result ordering,
filtering or field population of any listing function.

| Test | File | What it pins | Port? |
|---|---|---|---|
| `TestCoinbaseMatured` | `wallet/wallet_test.go:21–46` | `coinbaseMatured` on mainnet params: `(0,0)`→false, `(0,255)`→false, `(0,256)`→true, `(0,257)`→true, `(1,0)`→false, `(256,0)`→false, `(0,-1)`→false, `(-1,256)`→false, `(-1,MaxInt32)`→false | Port verbatim |
| `TestTicketMatured` | `wallet/wallet_test.go:48–73` | `ticketMatured` (part 3, used here): `(0,256)`→false ("dcrd off-by-one"), `(0,257)`→true, negative cases as above | Port verbatim |
| `TestTicketExpired` | `wallet/wallet_test.go:75–100` | `ticketExpired` with `expiry = TicketMaturity + TicketExpiry`: `(0,expiry)`→false, `(0,expiry+1)`→true | Port verbatim |
| `TestVotingXprivFromSeed` | `wallet/wallet_test.go:102–145` | 32 zero bytes on mainnet → `dprv3oaf1h1hLxLZNHn6Gn9AaUaMJxhpHAj2KGMMHTi2AnoiHdRhCciKbwf3dB6zpPEq8ffdT4NZ7gjtrZBQhSWDm2RVXbphmpdPnbq299ddB8a`; 1-byte seed → error whose kind is `Invalid` | Port verbatim; this is the only fixed vector in scope |
| `TestManagerImports` (`testImportPrivateKey`, `testImportScript`) | `wallet/udb/addressmanager_test.go:213–468` | manager-level import of WIF keys and scripts, expected addresses, duplicate rejection, behaviour when locked/watching-only | Belongs to the address manager spec; reuse its vectors for the wallet-level wrappers |
| `TestImportVotingAccount`, `TestImportAccount` | `wallet/udb/addressmanager_test.go:474–`, `:606–` | manager-level voting/xpub account import | Address manager spec |
| `TestStoreQueries`, `TestPreviousPkScripts` | `wallet/udb/txquery_test.go` | `RangeTransactions` forward/reverse, early break, `TxDetails`, `UniqueTxDetails` — but the file is `//go:build ignore` and uses a pre-`ctx` API | Disabled upstream. Treat as design notes; regenerate equivalent cases through an oracle |
| `TestWalletSvrCmds` | `rpc/jsonrpc/types/methods_test.go` | JSON marshalling of request commands only | JSON-RPC spec |

`wallet/wallet_test.go` (231 lines) is the only test file that exercises a
function declared in this range; its fifth test, `TestSetBirthStateAndScan`
(`:147`), belongs to another part. The other test files of the package
(`wallet/addresses_test.go`, `discovery_test.go`, `locking_test.go`,
`main_test.go`, `mixing_test.go`, `network_test.go`, `reorg_test.go`,
`setup_test.go`, `sidechains_test.go`) contain no call to any function in
scope (checked by searching every `_test.go` in the repository for the
in-scope names). `wallet/udb/testdata/v5.db.go` and `v7.go` call the
*manager-level* `ImportPrivateKey` of old module versions to build upgrade
fixtures; they are not tests of this scope. `TicketStatus.String()` has no
test; port the nine strings of §3.3 and the out-of-range format as a new
unit test.

Untested upstream and needing new tests (generate expected values with a Go
oracle shim that opens a fixture `wallet.db`, calls the function and dumps
JSON):

1. `listTransactions` entry matrix: mined/unmined; coinbase matured/immature;
   ticket, vote, revocation; partial debits (fee undeterminable); change
   outputs; multisig output (no address); zero-value outputs (`-0`).
2. `ListTransactions` paging: `from`/`count` across block boundaries, across
   the unmined/mined boundary, with transactions that produce no entries,
   `count = 0`, negative values.
3. `ListSinceBlock`, `ListAllTransactions`, `ListAddressTransactions`,
   `ListTransactionDetails` ordering on a fixture with several transactions
   per block.
4. `ListUnspent`: sort order including ties, each maturity rule at its
   boundary height, lock exclusion, account filter, address filter with
   multisig, P2SH with and without a known redeem script, `spendable` per
   script type, empty result (`null`).
5. `Accounts`: order with accounts ≥ 256 and imported xpub accounts;
   `TotalBalance` with unmined, unpublished and immature outputs.
6. `makeTicketSummary`: every row of the status table with and without the
   RPC flag; revocation at `expiry + maturity − 1` and `expiry + maturity`;
   unmined spender cases (Q-10, Q-11).
7. `GetTransactions`/`GetTickets` with each `(start, end)` shape in §5.2,
   including hash identifiers and reversed ranges.
8. `StakeInfo` vs `StakeInfoPrecise` field population; `Sdiff` on a network
   without DCP0001 height.
9. `BlockInfo` for main-chain, side-chain and stake-invalidated blocks.
10. `DumpWIFPrivateKey` round-trip against `dcrutil.DecodeWIF` for each
    network; `ImportPrivateKey` with compressed keys; duplicate imports;
    behaviour when the backend's `LoadTxFilter` fails.
11. `ImportXpubAccount`/`ImportVotingAccount`: account numbering, filter
    contents (address list and order), address-buffer state.
12. `LockedOutpoints`: unknown hash, out-of-range index, account filter,
    zero-value output (`amount` omitted), stake-tree transaction.
13. `Spender` and `UnspentOutput`: every row of the §5.19 table.
14. `UnminedTransactions`: unpublished exclusion; parent-before-child.
15. `hasVotingAuthority`: P2PKH and P2SH voting addresses; ordinary account,
    imported key with and without private key, imported voting account
    (Q-40), unknown address.
16. `ListSinceBlock` with `(-1, 0)`, `(-1, h)` and `(-1, -1)` (Q-37).
17. `ListUnspent` with `minconf = 0` on unmined tickets, votes and
    revocations (compatibility item 37), and with ≤ 12 versus > 12 outputs
    sharing a height (Q-41).
18. Address extraction matrix (§5.21): one credit per script class through
    `listTransactions`, `ListUnspent` and `Accounts` — P2PK (compressed and
    uncompressed key), 1-of-1 and 2-of-3 bare multisig, each stake-tagged
    form, treasury-gen, a non-zero script version — checking `address`,
    `account`, `spendable` and the `listunspent` address filter with
    pubkey-form versus pubkey-hash-form request addresses.
19. Import side effects against a recording backend: the exact
    `LoadTxFilter` argument list and order for each of the five imports, and
    the `AccountNotification` payload after a key import and after a script
    import followed by a key import (`ImportedKeyCount` counts both).
20. `GetTickets` with a negative start and a non-negative end on a short
    chain (Q-43).

Existing database fixtures (`wallet/udb/testdata/v*.db.gz`) are upgrade
fixtures for other specs; none is populated for these queries. A purpose-built
fixture wallet is needed.

---

## 12. Rust porting notes

**Suggested boundary.** One module group inside the wallet crate (working
name `purse-wallet`), mirroring the Go grouping but split by concern, all
built on the transaction-store and address-manager crates:

| Module | Contents | Go source |
|---|---|---|
| `query::listtx` | `list_transactions` entry builder; `list_since_block`, `list_transactions`, `list_address_transactions`, `list_all_transactions`, `list_transaction_details` | `wallet/wallet.go:2394–2721` |
| `query::blocks` | `BlockIdentifier`, `BlockInfo`, `block_info`, `block_range`, `get_transactions`, `transaction_summary`, `spender`, `unspent_output`, `unmined_transactions` | `:2723–2817`, `:3037–3081`, `:3233–3332`, `:4357–4380` |
| `query::tickets` | `TicketStatus`, `TicketSummary`, `make_ticket_summary`, `get_ticket_info`, `get_tickets` (one implementation with a `precise: bool` flag instead of an RPC handle) | `:2819–3231` |
| `query::accounts` | `AccountProperties`, `AccountsResult`, `accounts`, `list_unspent` and its comparator, `sorted_active_payment_addresses` | `:3334–3667`, `:4382–4400` |
| `keys::import` | `load_private_key`, `dump_wif_private_key`, `import_private_key`, `import_public_key`, `import_script`, `voting_xpriv_from_seed`, `import_voting_account`, `import_xpub_account` | `:3669–3951` |
| `stake::info` | `StakeInfoData`, `has_voting_authority`, `stake_info`, `stake_info_precise` | `:3953–4246` |
| `outpoint_locks` | the `(hash, index)` set and its five operations | `:4248–4355` |
| `maturity` | `confirms`, `confirmed`, `coinbase_matured` (+ the part-3 ticket predicates) | `:4402–4447` |

The JSON result structs (`ListTransactionsResult`, `ListUnspentResult`)
belong in the wallet JSON-RPC types crate; this module should build them
directly, as the Go does, so that field presence rules live in one place.

**Reusable pieces (each was looked at).**

| Need | Candidate |
|---|---|
| `stdscript.ExtractAddrs`, `ExtractPubKeyHashV0`, `ScriptType` | `dcroxide:crates/dcroxide-txscript/src/stdscript.rs` (`extract_addrs` at line 1157, `extract_pub_key_hash_v0` at 253, `ScriptType` at 82) |
| `stdaddr` address types, `Hash160` | `dcroxide:crates/dcroxide-txscript/src/stdaddr.rs` |
| `stake.IsSStx/IsSSGen/IsSSRtx/IsTSpend/DetermineTxType` | `dcroxide:crates/dcroxide-stake/src/lib.rs` (808, 1117, 1209, 1214), `dcroxide:crates/dcroxide-stake/src/treasury.rs:290` |
| `standalone.IsCoinBaseTx` (for `IsEitherCoinBaseTx`) | `dcroxide:crates/dcroxide-standalone/src/tx.rs:40` |
| `wire.MsgTx`, `wire.BlockHeader` (de)serialization | `dcroxide:crates/dcroxide-wire/src/msgtx.rs`, `blockheader.rs` |
| chain parameters (`coinbase_maturity`, `ticket_maturity`, `ticket_expiry`, `sstx_change_maturity`, `private_key_id`, coin types) | `dcroxide:crates/dcroxide-chaincfg/src/lib.rs:187–198` |
| `dcrdtypes.TransactionInput` | `dcroxide:crates/dcroxide-rpctypes/src/chainsvrcmds.rs:112` |
| base58 for WIF | `dcroxide:crates/dcroxide-base58` |
| Go-compatible JSON number/struct encoding | `dcroxide:crates/dcroxide-dcrjson/src/gojson.rs` |
| HD derivation with Decred's `Child` semantics, `neuter`, master from seed | `dcr-rs:src/hd.rs` (`ExtPrivKey::derive_child` at 280, `neuter` at 446, `master_from_seed` at 202, `ExtPubKey::derive_child` at 574) |

No WIF type was found in either repository (a case-insensitive search for
`wif` in the `.rs` files of `dcr-rs:src` and `dcroxide:crates` finds no
match); a small `Wif` type (§4.5)
has to be written, with decode owned by whichever spec covers
`importprivkey` parsing.

**Hazards.**

1. *Ordering is the product.* Implement the iteration primitives exactly
   (§5.2) and write the list functions as literal transcriptions including
   the final in-place reversal; do not "simplify" to a sort.
2. *Sort stability in `list_unspent`.* Rust's `sort_by` is stable and
   `sort_unstable_by` uses a different algorithm from Go's. Neither
   reproduces Go's tie order (Q-13). See Open questions.
3. *Float formatting.* A stock Rust JSON serializer does not format `f64`
   the way Go's `encoding/json` does (integral values, negative zero,
   exponent thresholds). dcroxide already carries a Go-compatible encoder
   (`dcroxide:crates/dcroxide-dcrjson/src/gojson.rs`, `format_float_json` at
   line 262); every amount, fee and balance built in this scope must be
   serialised through it or an equivalent.
4. *Nil versus empty.* `ListUnspent` and `SortedActivePaymentAddresses`
   return nil slices (JSON `null`) when empty; the transaction listings and
   `LockedOutpoints` return empty arrays. Model with `Option<Vec<_>>` or an
   explicit "null when empty" serializer.
5. *Account order.* Iterate the `acct` bucket by raw key, never by decoded
   number (Q-1).
6. *Hash ordering.* Unmined and UTXO iteration order is by raw (internal
   byte order) hash bytes. The displayed hex string is the byte-reversed
   hash, so sorting by the displayed strings gives a different order (not
   merely the reverse one); compare the raw 32 bytes.
7. *`i32` arithmetic.* Use wrapping semantics where the Go relies on `int32`
   (confirmation and maturity math) or prove the inputs cannot overflow.
8. *Locks.* Decide deliberately whether to reproduce the `Accounts`/
   `ListUnspent` lock-order inversion (Q-16).
9. *Side-effect order in imports.* Commit / filter-load / log / notify order
   differs per function (§5.14–5.15, Q-20–Q-22) and failures between steps
   leave observable partial state.
10. *Callback-in-transaction.* `get_transactions`/`get_tickets` call back
    while holding a read transaction; gRPC streaming happens inside. Keep the
    callback synchronous or collect per-block batches and release the
    transaction between blocks only if snapshot consistency is not required
    (it currently is: one snapshot for the whole stream).
11. *Secret hygiene.* `LoadPrivateKey` returns a zeroing closure;
    `votingXprivFromSeed` zeroes three intermediate keys. Use `zeroize` on
    the equivalents.

**Size estimate: M.** About 2,000 lines of Go, of which roughly a third is
duplicated (`GetTickets`/`GetTicketsPrecise`, `GetTicketInfo`/`…Precise`,
`StakeInfo`/`…Precise`, `ImportPrivateKey`/`ImportPublicKey`). The logic is
shallow — thin compositions over the store and address manager — so the Rust
is perhaps 1,200–1,500 lines plus tests. The cost is in the tests: an oracle
fixture wallet and golden JSON for every ordering rule in §9. It cannot
start until the transaction store (`RangeTransactions`, `TxDetails`,
`UnspentOutputs`, ticket iterator) and address manager (lookups and imports)
exist.

---

## 13. Open questions

1. **`listunspent` tie order (Q-13).** Reproduce Go's `sort.Sort` exactly
   (port the pattern-defeating quicksort of the Go release used for the
   reference build and feed it the same input order), or define a
   deterministic tie-break (for example raw outpoint key order) and accept a
   documented divergence? Needs a decision; exact parity requires reading and
   porting Go's `sort` package, which was not done for this spec.
2. **JSON float encoding (§4.4, Q-3).** Confirm with a Go oracle that
   zero-value send entries serialise as `-0` and that sub-`1e-6` amounts use
   exponent form; then decide whether the project-wide serializer reproduces
   both. Affects every amount in this scope.
3. **Lock-order inversion (Q-16).** Reproduce or fix (snapshot the locked set
   before the read transaction)? The fix is not observable to clients except
   as the absence of a potential hang.
4. **`listlockunspent` and `UnminedTransactions` ordering (Q-18, Q-25).** Go
   order is random. Choose a deterministic order for the Rust port (suggest:
   raw `(hash, index)` ascending for locks; raw-hash ascending among
   independent unmined transactions).
5. **Unmined ticket spenders (Q-10, Q-11).** Keep reporting a ticket as
   live/unspent while its vote is unmined, and an unmined revocation as
   `missed`? Bug-compatible is the default; confirm.
6. **Ed25519/Schnorr WIF import (Q-24).** Determine actual upstream behaviour
   with an oracle and decide whether to reproduce or reject.
7. **Treasury transactions as coinbase (Q-5).** Confirm with an oracle on a
   wallet that received a treasury spend that `listtransactions` reports
   `generate`/`immature`; the conclusion here is from reading
   `standalone.IsCoinBaseTx` only.
8. **bbolt backward-seek semantics (§5.2).** The "start at the greatest
   height below `b`" rule relies on bbolt's `Cursor.Seek` past the last key
   followed by `Prev` returning the last key. The database-layer spec must
   state that guarantee for whatever storage engine the Rust port uses.
9. **Signing and publishing.** The assignment text lists them under this
   spec, but no such function is declared in lines 2394–4428; confirm they
   are covered by parts 1 and 3.
10. **`AccountProperties` for the imported account (Q-32).** gRPC reports one
    external and one internal key for the imported account as a side effect
    of zero-valued indexes; owned by the gRPC/address-manager specs, listed
    here so it is not "fixed" by accident in `accounts`.
11. **`havePrivKey` for imported voting accounts (Q-40).** Confirm what the
    chain-notification caller does with the flag before deciding whether to
    reproduce `HavePrivateKey`'s `account < 0x7FFFFFFF` rule.
12. **Go toolchain of the reference build (Q-13, Q-41).** `go.mod` says
    `go 1.26.0`; the sort implementation inspected was go1.27. Pin the
    toolchain used to produce oracle vectors and re-read its `sort` package.
13. **Pubkey-address prefixes on non-mainnet networks (§5.21).** Only the
    mainnet `PubKeyAddrID` (`Dk`) was read; the testnet3, simnet and regnet
    values come from the chain-parameters crate and should be covered by the
    oracle vectors of test item 18.
14. **Short-chain ticket range (Q-43).** Reproduce the "tip block only"
    behaviour of `ForUnspentUnexpiredTickets` on chains shorter than
    `TicketExpiry + TicketMaturity − 8`, or clamp the start to 0? Decide in
    part 3; the range primitive here must keep the negative-start semantics
    either way because gRPC clients can pass negative heights (Q-33).
