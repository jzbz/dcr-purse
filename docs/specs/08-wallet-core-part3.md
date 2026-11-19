# 08 — Wallet core, part 3 (`wallet/wallet.go` 4429–5828) and the notification server

## 1. Header

**Scope (files read end to end, with line counts):**

| File | Lines in scope | Total lines |
|---|---|---|
| `wallet/wallet.go` | 4429–5828 (1,400) | 5,828 |
| `wallet/notifications.go` | all | 988 |
| `wallet/utxos.go` | all | 87 |
| `wallet/unstable.go` | all | 75 |
| `wallet/tickets.go` | all | 93 |
| `wallet/ticketstatus_string.go` | all (generated) | 32 |

Sibling specs cover `wallet/wallet.go` lines 1–2393 and 2394–4428. Where this
spec relies on something defined there (for example `confirms`, `Config`,
`TicketStatus`, `GetTickets`, `RangeCFiltersV2`, locked-outpoint methods), the
definition is restated only as far as needed and cited.

**What the range contains.** Sweeping, coin/UTXO listings, account property
queries and the locked-outpoint methods live before line 4429 (lower line
numbers) and belong to the sibling specs. Lines 4429–5828 hold: the ticket maturity/expiry predicates, the
received-total queries, `SendOutputs`/`SendOutputsToTreasury`,
`CreateVspPayment`, `SignTransaction`, `CreateSignature`, transaction
relevance tests, `AbandonTransaction`, `PublishTransaction`,
`PublishUnminedTransactions`, `NeedsAccountsSync`, pre-DCP0005 cfilter
validation and import, `Create`/`CreateWatchOnly`/`Open`/`Run`, small config
getters, the coinjoin-output-count query and the VSP ticket query helpers.

**Parity pin:** dcrwallet `54b85b33` (2.2.0-pre, module
`decred.org/dcrwallet/v5`, `go 1.26.0` in `go.mod:3`). Dependency versions
read (from `go.mod`):

| Module | Version | Read for |
|---|---|---|
| `github.com/decred/dcrd/txscript/v4` | v4.1.2 | `sign/sign.go` (whole file), `sighash.go` entry point, `stdaddr` `Hash160er`, `stdscript` redeem-script extraction |
| `github.com/decred/dcrd/blockchain/standalone/v2` | v2.3.0 | `IsCoinBaseTx` |
| `github.com/decred/dcrd/chaincfg/v3` | v3.3.0 | ticket/coinbase maturity parameters |
| `github.com/decred/dcrd/wire` | v1.8.0 | `OutPoint.String` |
| `github.com/decred/dcrd/dcrutil/v4` | v4.0.3 | `Amount.String`, `WIF` method set |
| `github.com/decred/dcrd/gcs/v4` | v4.1.1 | `FilterV2.Bytes`, `KeySize` |
| `github.com/decred/dcrd/hdkeychain/v3` | v3.1.3 | seed length constants, `HardenedKeyStart` |
| `github.com/decred/dcrd/crypto/blake256` | v1.1.0 | `New` |
| `github.com/decred/dcrd/blockchain/stake/v5` | v5.0.3 | referenced only (tx-type predicates); not re-read in full |
| `github.com/decred/dcrd/mixing` | v0.7.5 | referenced only (`mixpool.NewPool`, `mixclient.NewClient`) |

**Summary.** This slice of the wallet is the glue between the transaction
store and everything outside the wallet: it decides which transactions are
relevant, signs them, records and publishes them, removes them again when
publishing fails, opens the wallet database and builds the in-memory `Wallet`
object, and tells subscribers what changed. The notification server has five
subscriber kinds (transactions, removed transactions, account properties, main
tip changes, confirmations). Every delivery is a blocking send on an
unbuffered channel made while the server mutex is held (confirmation results
additionally give up when the subscriber's context ends; the one exception to
the mutex rule is the immediate result of
`ConfirmationNotificationsClient.Watch`, which is sent without the server
mutex), and several are made
from inside an open database write transaction, so a slow subscriber stalls
the wallet. Three of the five kinds are exposed to RPC clients (gRPC
streams `TransactionNotifications`, `AccountNotifications`,
`ConfirmationNotifications`); `MainTipChanged` feeds only the in-process
ticket buyer and `RemovedTransaction` has no consumer at all. Upstream has no tests for the
notification server, signing, publishing or relevance; only `ticketMatured`
and `ticketExpired` have direct tests, and `Create`/`Open` are exercised only
as a test fixture.

---

## 2. Role in the system

**What it does.**

- *Predicates* (`ticketChangeMatured`, `ticketMatured`, `ticketExpired`):
  height arithmetic used by balance, UTXO selection, ticket listing and status
  code throughout the `wallet` package.
- *Queries*: received totals per account and per address; coinjoin output
  counts per account; VSP ticket records by fee status / host / fee hash;
  live ticket hashes; input selection preview; output info; three
  "unstable" pass-throughs to the transaction store.
- *Spending*: `SendOutputs`, `SendOutputsToTreasury` (author, record,
  publish), `CreateVspPayment` (build and sign a VSP fee transaction and store
  it unpublished).
- *Signing*: `SignTransaction` (sign every input it can, then run the script
  engine on every input and report failures), `CreateSignature` (one raw
  signature plus compressed pubkey).
- *Relevance and publication*: `isRelevantTx`, `DetermineRelevantTxs`,
  `appendRelevantOutpoints`, `PublishTransaction`,
  `PublishUnminedTransactions`, `AbandonTransaction`.
- *Lifecycle*: `Create`, `CreateWatchOnly`, `Open`, `Run`, and getters for
  configuration captured at `Open`.
- *Notification server*: accumulates per-chain-switch transaction
  notifications, emits unmined-transaction, removed-transaction, account,
  main-tip-changed and confirmation notifications.

**Who calls it.**

| Caller | Uses |
|---|---|
| `dcrwallet.go:424`, `dcrwallet.go:479` | `ProcessedTickets` (VSP sync at startup), `NeedsAccountsSync` (startup passphrase prompt) |
| `walletsetup.go:305`, `walletsetup.go:356`, `internal/loader/loader.go:163,186,253,275,335` | `Create`, `CreateWatchOnly`, `Open` |
| `chain/sync.go:362,689,733,737,919,934`, `spv/sync.go:450,803,911,1155,1870` | `GapLimit`, `PublishUnminedTransactions`, `Run`, `MixingEnabled`, `ManualTickets` |
| `spv/backend.go:442` | `DetermineRelevantTxs` (decides which published transactions the SPV syncer keeps in its own mempool) |
| `internal/rpc/jsonrpc/methods.go` | `AbandonTransaction` (322), `CreateSignature` (786), `GapLimit` (846), `ImportCFiltersV2` (1998), `TotalReceivedForAccounts` (2411, 3014), `TotalReceivedForAddr` (2441), `LiveTicketHashes` (2605), `UnstableAPI(...).TxDetails` (2634), `UnstableAPI(...).RangeTransactions` (3087), `SendOutputs` (3611), `SendOutputsToTreasury` (3651), `UnstableAPI(...).UnspentMultisigCreditsForAddress` (4186), `PublishTransaction` (4395, 4706), `VSPHostForTicket` (4470, 4814, 4837), `ForUnspentUnexpiredTickets` (4836), `SignTransaction` (5094), `ValidatePreDCP0005CFilters` (5398), `ManualTickets` (5528), `GetCoinjoinTxsSumbByAcct` (5841) |
| `internal/rpc/rpcserver/server.go` | `PublishUnminedTransactions` (434), `GapLimit` (851, 4301), `SelectInputs` (1058, 1101), `OutputInfo` (1072, 1111), `VSPHostForTicket` (1477, 1518, 4230), `SignTransaction` (1623, 1688), `CreateSignature` (1748), `AllowsHighFees` (1771), `PublishTransaction` (1782), `TransactionNotifications` (2391), `AccountNotifications` (2419), `ConfirmationNotifications` (2445), `AbandonTransaction` (3781), `GetCoinjoinTxsSumbByAcct` (3850), `GetVSPTicketsByFeeStatus` (4065, 4081), `ProcessedTickets` (4137), `UnprocessedTickets` (4167), `ForUnspentUnexpiredTickets` (4225) |
| `ticketbuyer/tb.go:71` | `NtfnServer.MainTipChangedNotifications` |
| `wallet/createtx.go:346`, `wallet/feepayment.go:335,442,539`, `wallet/mixing.go:164`, `wallet/vspticket.go:292`, `wallet/chainntfns.go:854` | `AbandonTransaction`, `CreateVspPayment`, `SetPublished`, `PublishTransaction`, `VSPFeeHashForTicket`, `appendRelevantOutpoints` |
| `wallet/chainntfns.go:52,146,262,312,313,664,671`, `wallet/wallet.go:2130,2236,3734,3771,5090` | the unexported `notify*` methods (see §5.3) |
| `internal/rpc/rpcserver/server.go:3715` | `TxTransactionType` (gRPC `DecodeRawTransaction`, field `transaction_type`, on an arbitrary caller-supplied transaction) |
| `wallet/wallet.go:2805` (`TransactionSummary`), `2894`, `2911` (`makeTicketSummary`: ticket and spender), `3268` (`GetTransactions`) | `makeTxSummary` outside the notification server; the same construction rules (§5.22) therefore apply to gRPC `GetTransaction`, `GetTransactions`, `GetTickets` |
| `wallet/wallet.go:2906` (`makeTicketSummary`), `3525`, `3531` (`ListUnspent`), `4048`, `4080` (`StakeInfo`), `4164` (`StakeInfoPrecise`), `wallet/createtx.go:1790`, `1884` (`findEligibleOutputs`, `findEligibleOutputsAmount`), `wallet/tickets.go:44,51` | `ticketMatured`, `ticketChangeMatured`, `ticketExpired` |
| `wallet/wallet.go:5762,5799` only | `IsVSPTicketConfirmed` (exported, but no caller outside `UnprocessedTickets`/`ProcessedTickets`) |
| no caller anywhere in the tree | `RemovedTransactionNotifications` |
| many callers throughout the tree (not enumerated) | `ChainParams` |

**What it calls.** `wallet/udb` (transaction store `Store`, address manager
`Manager`, VSP buckets), `wallet/walletdb` (`View`, `Update`),
`wallet/txrules`, `wallet/txsizes`, `wallet/txauthor`, `validate`,
`internal/compat`, the `NetworkBackend` interface (`wallet/network.go:35`),
dcrd `txscript` (engine and `sign`), `stake`, `stdscript`, `stdaddr`,
`blockchain/standalone`, `gcs`, `hdkeychain`, `mixing/mixpool`,
`mixing/mixclient`.

---

## 3. Public surface inventory

Everything in scope is listed. "U" marks unexported items that carry
observable behaviour.

### 3.1 `wallet/wallet.go` 4429–5828

| Item | Line | Signature | Contract |
|---|---|---|---|
| `ticketChangeMatured` (U) | 4429 | `(params, txHeight, curHeight int32) bool` | `txHeight >= 0 && curHeight-txHeight+1 > SStxChangeMaturity`. |
| `ticketMatured` (U) | 4435 | same | `txHeight >= 0 && curHeight-txHeight > TicketMaturity` (one block later than the parameter suggests; mirrors dcrd). |
| `ticketExpired` (U) | 4444 | same | `txHeight >= 0 && curHeight-txHeight > TicketMaturity+TicketExpiry`. |
| `AccountTotalReceivedResult` | 4451 | struct `{AccountNumber uint32; AccountName string; TotalReceived dcrutil.Amount; LastConfirmation int32}` | One row of `TotalReceivedForAccounts`. |
| `(*Wallet).TotalReceivedForAccounts` | 4460 | `(ctx, minConf int32) ([]AccountTotalReceivedResult, error)` | One row per account row in the address manager, in database key order; sums wallet credits. §5.1. |
| `(*Wallet).TotalReceivedForAddr` | 4527 | `(ctx, addr stdaddr.Address, minConf int32) (dcrutil.Amount, error)` | Sum of wallet credits whose script pays `addr`. §5.1. |
| `(*Wallet).SendOutputs` | 4572 | `(ctx, outputs []*wire.TxOut, account, changeAccount uint32, minconf int32) (*chainhash.Hash, error)` | Check outputs, author, record, publish. Returns the transaction hash. §5.2. |
| `(*Wallet).SendOutputsToTreasury` | 4609 | same | As `SendOutputs` with `isTreasury=true`, `randomizeChangeIdx=false`. §5.2. |
| `SignatureError` | 4647 | struct `{InputIndex uint32; Error error}` | One failed input from `SignTransaction`. Not itself an `error`. |
| `sigDataSource` (U) | 4652 | struct of two closures; methods `GetKey`, `GetScript` | Adapts closures to `sign.KeyDB` and `sign.ScriptDB`. |
| `(*Wallet).CreateVspPayment` | 4665 | `(ctx, tx *wire.MsgTx, fee dcrutil.Amount, feeAddr stdaddr.Address, feeAcct, changeAcct uint32) error` | Completes, signs and stores (unpublished) a VSP fee transaction. Mutates `tx`. §5.4. |
| `(*Wallet).SignTransaction` | 4773 | `(ctx, tx *wire.MsgTx, hashType txscript.SigHashType, additionalPrevScripts map[wire.OutPoint][]byte, additionalKeysByAddress map[string]*dcrutil.WIF, p2shRedeemScriptsByAddress map[string][]byte) ([]SignatureError, error)` | Signs in place; returns per-input failures and a fatal error. §5.5. |
| `(*Wallet).CreateSignature` | 4914 | `(ctx, tx *wire.MsgTx, idx uint32, addr stdaddr.Address, hashType txscript.SigHashType, prevPkScript []byte) (sig, pubkey []byte, err error)` | Raw ECDSA signature (DER plus hash-type byte) and 33-byte compressed pubkey. §5.6. |
| `(*Wallet).isRelevantTx` (U) | 4952 | `(dbtx walletdb.ReadTx, tx *wire.MsgTx) bool` | Relevance test used by publish paths. §5.7. |
| `(*Wallet).DetermineRelevantTxs` | 4988 | `(ctx, txs ...*wire.MsgTx) (relevant, nonRelevant []*wire.MsgTx, err error)` | Partitions `txs` under one view; preserves input order within each partition. |
| `(*Wallet).appendRelevantOutpoints` (U) | 5004 | `(relevant []wire.OutPoint, dbtx walletdb.ReadTx, tx *wire.MsgTx) []wire.OutPoint` | Appends outpoints of `tx` to watch. §5.8. |
| `(*Wallet).AbandonTransaction` | 5073 | `(ctx, hash *chainhash.Hash) error` | Removes an unmined transaction and its unmined descendants; notifies removal. §5.9. |
| `(*Wallet).AllowsHighFees` | 5096 | `() bool` | Returns `Config.AllowHighFees` captured at `Open`. |
| `(*Wallet).PublishTransaction` | 5108 | `(ctx, tx *wire.MsgTx, n NetworkBackend) (*chainhash.Hash, error)` | Record if relevant, publish, abandon on failure, watch outpoints. §5.10. |
| `(*Wallet).PublishUnminedTransactions` | 5172 | `(ctx, n NetworkBackend) error` | Rebroadcast all unmined, published-flagged transactions in dependency order. §5.11. |
| `(*Wallet).ChainParams` | 5187 | `() *chaincfg.Params` | Returns the params given at `Open`. |
| `(*Wallet).NeedsAccountsSync` | 5196 | `(ctx) (bool, error)` | True iff last BIP0044 account is 0 and account 0 has no address rows. §5.12. |
| `(*Wallet).ValidatePreDCP0005CFilters` | 5230 | `(ctx) error` | Hashes all stored cfilters below the DCP0005 activation height and compares to a hard-coded hash. §5.13. |
| `(*Wallet).ImportCFiltersV2` | 5278 | `(ctx, startBlockHeight int32, filterData [][]byte) error` | Overwrites stored cfilters for consecutive main-chain blocks. §5.14. |
| `Create` | 5292 | `(ctx, db DB, pubPass, privPass, seed []byte, params *chaincfg.Params) error` | Initialises an empty database with a seeded wallet. §5.15. |
| `CreateWatchOnly` | 5316 | `(ctx, db DB, extendedPubKey string, pubPass []byte, params *chaincfg.Params) error` | Initialises an empty database as watching-only. §5.15. |
| `(*Wallet).GapLimit` | 5326 | `() uint32` | Returns `Config.GapLimit`. |
| `(*Wallet).ManualTickets` | 5334 | `() bool` | Returns `Config.ManualTickets`. |
| `Open` | 5340 | `(ctx, cfg *Config) (*Wallet, error)` | Migrates, upgrades, opens and builds the `Wallet`. §5.16. |
| `(*Wallet).MixingEnabled` | 5534 | `() bool` | Returns `Config.MixingEnabled`. |
| `newMixClient` (U) | 5538 | `(w *Wallet) *mixclient.Client` | `mixclient.NewClient((*mixingWallet)(w))` with logger `loggers.MixcLog` (subsystem `MIXC`). |
| `(*Wallet).Run` | 5545 | `(ctx) error` | If mixing is enabled: create a mix client, store it in `w.mixClient`, run it until `ctx` ends. Otherwise return nil immediately. §5.17. |
| `(*Wallet).getCoinjoinTxsSumbByAcct` (U) | 5556 | `(ctx) (map[uint32]int, error)` | Count of wallet-owned mixed-denomination outputs per account. §5.18. |
| `(*Wallet).GetCoinjoinTxsSumbByAcct` | 5601 | same | Wrapper adding op `wallet.GetCoinjoinTxsSumbByAcct`. |
| `(*Wallet).GetVSPTicketsByFeeStatus` | 5613 | `(ctx, feeStatus int) ([]chainhash.Hash, error)` | Ticket hashes whose VSP record has `FeeTxStatus == feeStatus`. Order unspecified. §5.19. |
| `(*Wallet).SetPublished` | 5636 | `(ctx, hash *chainhash.Hash, published bool) error` | Adds (false) or deletes (true) the hash in the unpublished-transactions bucket. Error returned unwrapped. |
| `(*Wallet).VSPFeeHashForTicket` | 5653 | `(ctx, ticketHash *chainhash.Hash) (chainhash.Hash, error)` | Fee tx hash from the VSP record; `NotExist` if no record or the stored hash is all zero. |
| `(*Wallet).VSPHostForTicket` | 5670 | `(ctx, ticketHash *chainhash.Hash) (string, error)` | Host from the VSP record; `NotExist` if no record or host empty. |
| `(*Wallet).IsVSPTicketConfirmed` | 5688 | `(ctx, ticketHash *chainhash.Hash) (bool, error)` | True iff record exists with `FeeTxStatus == 3` (`VSPFeeProcessConfirmed`). `NotExist` if no record. |
| `(*Wallet).ForUnspentUnexpiredTickets` | 5705 | `(ctx, f func(hash *chainhash.Hash) error) error` | Calls `f` for tickets with status live/immature/unspent in a height window; joins all errors. §5.20. |
| `(*Wallet).UnprocessedTickets` | 5751 | `(ctx) ([]*VSPTicket, error)` | Tickets with no fee hash recorded and not confirmed. §5.21. |
| `(*Wallet).ProcessedTickets` | 5795 | `(ctx) ([]*VSPTicket, error)` | Tickets that have a VSP record that is not confirmed. §5.21. |

### 3.2 `wallet/notifications.go`

| Item | Line | Signature | Contract |
|---|---|---|---|
| `NotificationServer` | 40 | struct (all fields unexported) | One per `Wallet`, exposed as `Wallet.NtfnServer` (`wallet/wallet.go:168`). |
| `newNotificationServer` (U) | 53 | `(wallet *Wallet) *NotificationServer` | Zero server bound to the wallet. |
| `lookupInputAccount` (U) | 59 | `(dbtx, w, details *udb.TxDetails, deb udb.DebitRecord) uint32` | Account of the previous output of a debit; 0 on any failure (logged). |
| `lookupOutputChain` (U) | 89 | `(dbtx, w, details, cred udb.CreditRecord) (account uint32, internal bool, address stdaddr.Address, amount int64, outputScript []byte)` | Wallet metadata for a credit; all-zero results on failure. |
| `makeTxSummary` (U) | 114 | `(dbtx, w, details *udb.TxDetails) TransactionSummary` | Builds the summary used by notifications, `TransactionSummary`, `GetTransactions`, ticket summaries. §5.22. |
| `totalBalances` (U) | 183 | `(dbtx, w, m map[uint32]dcrutil.Amount) error` | Adds every unspent output amount to `m[account]` for accounts already present in `m`. |
| `flattenBalanceMap` (U) | 206 | `(m) []AccountBalance` | Map to slice; order is map iteration order. |
| `relevantAccounts` (U) | 214 | `(m, txs []TransactionSummary)` | Sets `m[a] = 0` for every input and output account in `txs`. |
| `(*NotificationServer).notifyUnminedTransaction` (U) | 225 | `(dbtx walletdb.ReadTx, details *udb.TxDetails)` | Sends one `TransactionNotifications` for a new unmined transaction. §5.23. |
| `(*NotificationServer).notifyDetachedBlock` (U) | 263 | `(header *wire.BlockHeader)` | Appends to the pending notification's `DetachedBlocks`. |
| `(*NotificationServer).notifyMinedTransaction` (U) | 273 | `(dbtx, details *udb.TxDetails, block *udb.BlockMeta)` | Appends a summary to the last pending attached block if its hash matches; otherwise drops it. |
| `(*NotificationServer).notifyAttachedBlock` (U) | 288 | `(block *wire.BlockHeader, blockHash *chainhash.Hash)` | Appends a `Block{Header}` to the pending notification unless it is already the last one. |
| `(*NotificationServer).sendAttachedBlockNotification` (U) | 306 | `(ctx)` | Completes and sends the pending notification. §5.24. |
| `TransactionNotifications` | 376 | struct `{AttachedBlocks []Block; DetachedBlocks []*wire.BlockHeader; UnminedTransactions []TransactionSummary; UnminedTransactionHashes []*chainhash.Hash; NewBalances []AccountBalance}` | §4.2. |
| `Block` | 386 | struct `{Header *wire.BlockHeader; Transactions []TransactionSummary}` | `Header` nil means "mempool" (used by `GetTransactions`, never by notifications). |
| `TransactionSummary` | 393 | struct `{Hash *chainhash.Hash; Transaction []byte; MyInputs []TransactionSummaryInput; MyOutputs []TransactionSummaryOutput; Fee dcrutil.Amount; Timestamp int64; Type TransactionType}` | §4.2. |
| `TransactionType` | 406 | `int8` | Constants: `TransactionTypeRegular`=0, `TransactionTypeCoinbase`=1, `TransactionTypeTicketPurchase`=2, `TransactionTypeVote`=3, `TransactionTypeRevocation`=4 (lines 408–427). |
| `TxTransactionType` | 430 | `(tx *wire.MsgTx) TransactionType` | First match of: either-coinbase, SStx, SSGen, SSRtx (through the package helper `isRevocation`, `wallet/wallet.go:3977`), else regular. §5.22. |
| `TransactionSummaryInput` | 448 | struct `{Index uint32; PreviousAccount uint32; PreviousAmount dcrutil.Amount}` | |
| `TransactionSummaryOutput` | 457 | struct `{Index uint32; Account uint32; Internal bool; Amount dcrutil.Amount; Address stdaddr.Address; OutputScript []byte}` | |
| `AccountBalance` | 470 | struct `{Account uint32; TotalBalance dcrutil.Amount}` | Zero-confirmation total. |
| `TransactionNotificationsClient` | 477 | struct `{C <-chan *TransactionNotifications; server}` | |
| `(*NotificationServer).TransactionNotifications` | 488 | `() TransactionNotificationsClient` | Registers an unbuffered channel. |
| `(*TransactionNotificationsClient).Done` | 502 | `()` | Asynchronously drains, deregisters and closes the channel. Call exactly once. |
| `RemovedTransactionNotification` | 526 | struct `{TxHash chainhash.Hash}` | |
| `RemovedTransactionNotificationsClient` | 531 | struct `{C chan *RemovedTransactionNotification; server}` | |
| `(*NotificationServer).RemovedTransactionNotifications` | 539 | `() RemovedTransactionNotificationsClient` | Registers an unbuffered channel. No in-tree caller. |
| `(*RemovedTransactionNotificationsClient).Done` | 553 | `()` | As above. |
| `(*NotificationServer).notifyRemovedTransaction` (U) | 574 | `(hash chainhash.Hash)` | Sends to all removed-transaction clients. |
| `AccountNotification` | 592 | struct `{AccountNumber uint32; AccountName string; ExternalKeyCount, InternalKeyCount, ImportedKeyCount uint32}` | |
| `(*NotificationServer).notifyAccountProperties` (U) | 600 | `(props *udb.AccountProperties)` | Builds and sends an `AccountNotification`. §5.25. |
| `AccountNotificationsClient` | 630 | struct `{C chan *AccountNotification; server}` | |
| `(*NotificationServer).AccountNotifications` | 638 | `() AccountNotificationsClient` | Registers an unbuffered channel. |
| `(*AccountNotificationsClient).Done` | 652 | `()` | As above. |
| `MainTipChangedNotification` | 678 | struct `{AttachedBlocks []*chainhash.Hash; DetachedBlocks []*chainhash.Hash; NewHeight int32}` | Both lists sorted by increasing height. |
| `MainTipChangedNotificationsClient` | 686 | struct `{C chan *MainTipChangedNotification; server}` | |
| `(*NotificationServer).MainTipChangedNotifications` | 695 | `() MainTipChangedNotificationsClient` | Registers an unbuffered channel. |
| `(*MainTipChangedNotificationsClient).Done` | 709 | `()` | As above. |
| `(*NotificationServer).notifyMainChainTipChanged` (U) | 730 | `(n *MainTipChangedNotification)` | Sends to tip clients, then runs every confirmation client. §5.26. |
| `(*NotificationServer).ConfirmationNotifications` | 754 | `(ctx) *ConfirmationNotificationsClient` | Registers a client that lives until `ctx` is done. |
| `ConfirmationNotificationsClient` | 790 | struct (unexported fields) | |
| `confNtfnResult` (U) | 799 | struct `{result []ConfirmationNotification; err error}` | Item carried on the client's result channel. |
| `ConfirmationNotification` | 809 | struct `{TxHash *chainhash.Hash; Confirmations int32; BlockHash *chainhash.Hash; BlockHeight int32}` | `Confirmations == -1` means unknown/removed/invalidated. `BlockHash` nil and `BlockHeight` -1 unless `Confirmations > 0`. |
| `(*ConfirmationNotificationsClient).Watch` | 820 | `(txHashes []*chainhash.Hash, stopAfter int32)` | Emits one immediate result, then watches the survivors. Mutates `txHashes`. §5.27. |
| `(*ConfirmationNotificationsClient).Recv` | 903 | `() ([]ConfirmationNotification, error)` | Blocks for the next result; `context.Canceled` once the client context is done. |
| `(*ConfirmationNotificationsClient).process` (U) | 912 | `(tipHeight int32)` | Per-tip-change evaluation of every watched hash. §5.28. |

### 3.3 `wallet/utxos.go`

| Item | Line | Signature | Contract |
|---|---|---|---|
| `(*Wallet).SelectInputs` | 22 | `(ctx, targetAmount dcrutil.Amount, account uint32, requiredConfs int32) (*txauthor.InputDetail, error)` | Runs the store's input source once for `targetAmount`. §5.29. |
| `OutputInfo` (type) | 56 | struct `{Received time.Time; Amount dcrutil.Amount; FromCoinbase bool}` | |
| `(*Wallet).OutputInfo` | 64 | `(ctx, out *wire.OutPoint) (OutputInfo, error)` | Looks up the transaction of `out.Hash` and reports on output `out.Index`. §5.30. |

### 3.4 `wallet/unstable.go`

| Item | Line | Signature | Contract |
|---|---|---|---|
| `unstableAPI` (U) | 17 | struct `{w *Wallet}` | |
| `UnstableAPI` | 26 | `(w *Wallet) unstableAPI` | Exported constructor returning an unexported type. |
| `(unstableAPI).TxDetails` | 29 | `(ctx, txHash *chainhash.Hash) (*udb.TxDetails, error)` | `Store.TxDetails` in one view; op `wallet.TxDetails`. |
| `(unstableAPI).RangeTransactions` | 47 | `(ctx, begin, end int32, f func([]udb.TxDetails) (bool, error)) error` | `Store.RangeTransactions` in one view; op `wallet.RangeTransactions`. |
| `(unstableAPI).UnspentMultisigCreditsForAddress` | 62 | `(ctx, p2shAddr *stdaddr.AddressScriptHashV0) ([]*udb.MultisigCredit, error)` | `Store.UnspentMultisigCreditsForAddress` in one view; op `wallet.UnspentMultisigCreditsForAddress`. |

### 3.5 `wallet/tickets.go`

| Item | Line | Signature | Contract |
|---|---|---|---|
| `LiveTicketQuerier` | 18 | interface `{ ExistsLiveTickets(ctx, tickets []*chainhash.Hash) (bitset.Bytes, error) }` | Implemented by the dcrd RPC backend (`chain/backend.go:81`). The SPV syncer does not implement it. |
| `(*Wallet).LiveTicketHashes` | 28 | `(ctx, rpc LiveTicketQuerier, includeImmature bool) ([]chainhash.Hash, error)` | §5.31. |

### 3.6 `wallet/ticketstatus_string.go`

Generated by `stringer -type=TicketStatus -linecomment`
(`wallet/wallet.go:2849`). `TicketStatus` is `uint` (`wallet/wallet.go:2847`).

| Value | Constant | `String()` |
|---|---|---|
| 0 | `TicketStatusUnknown` | `unknown` |
| 1 | `TicketStatusUnmined` | `unmined` |
| 2 | `TicketStatusImmature` | `immature` |
| 3 | `TicketStatusLive` | `live` |
| 4 | `TicketStatusVoted` | `voted` |
| 5 | `TicketStatusMissed` | `missed` |
| 6 | `TicketStatusExpired` | `expired` |
| 7 | `TicketStatusUnspent` | `unspent` |
| 8 | `TicketStatusRevoked` | `revoked` |
| other `n` | — | `TicketStatus(n)` with `n` in base 10 (`wallet/ticketstatus_string.go:29`) |

---

## 4. Data formats

Nothing in this scope defines a new on-disk layout. It reads and writes
records whose byte layouts are owned by the `wallet/udb` specs, and it builds
the in-memory structures that the gRPC server serialises. Both are listed so
that the implementer knows exactly which bytes each function touches.

### 4.1 Database records touched

Top-level namespace bucket keys are `waddrmgr` and `wtxmgr`
(`wallet/wallet.go:77-78`). All integers written by `wallet/udb` transaction
store code are big-endian (`wallet/udb/txdb.go:123`); the address manager uses
little-endian (`wallet/udb/addressdb.go:269`).

| Function in scope | Bucket path | Key | Value | Access |
|---|---|---|---|---|
| `SetPublished(hash, false)` | `wtxmgr` / `up` | 32-byte tx hash (raw, internal byte order) | single byte `0x01` | Put (`wallet/udb/txdb.go:1401`) |
| `SetPublished(hash, true)` | `wtxmgr` / `up` | 32-byte tx hash | — | Delete (`wallet/udb/txdb.go:1414`); deleting an absent key is not an error |
| `ImportCFiltersV2` | `wtxmgr` / `cf` | 32-byte block hash | `[0:16]` filter key = first 16 bytes of the block header's merkle root (header bytes 36..52), `[16:]` raw filter bytes (`N` followed by filter data, exactly as supplied) | Put, overwriting (`wallet/udb/cfilters.go:34-67`, `wallet/udb/txdb.go:1981-1990`, `wallet/udb/txmined.go:550-553`) |
| `ValidatePreDCP0005CFilters` | `wtxmgr` / `b` (block records), `wtxmgr` / `cf` | — | — | Read |
| `GetVSPTicketsByFeeStatus`, `VSPFeeHashForTicket`, `VSPHostForTicket`, `IsVSPTicketConfirmed` | top-level `vsp` | 32-byte ticket hash | 40 bytes: `[0:32]` fee tx hash, `[32:36]` fee status `uint32` BE, `[36:40]` VSP host id `uint32` BE (`wallet/udb/vsp.go:149-179`) | Read. `GetVSPTicketsByFeeStatus` reads only `vsp`. The other three go through `udb.GetVSPTicket` (`wallet/udb/vsp.go:98-125`), which for every call additionally reads top-level `vsphost` (key: host id `uint32` BE) and, when the stored host is non-empty, `vsppubkey` (key: host bytes); a `NotExist` from either lookup is ignored, any other error from them fails the call |
| `AbandonTransaction` | `wtxmgr` / `m` (unmined), `mc` (unmined credits), `mi` (unmined inputs), `up`, ticket commitment (`cmt`, `cmu`) and multisig buckets | — | — | Delete/update through `Store.RemoveUnconfirmed` (`wallet/udb/txunmined.go:181-283`) |
| `PublishTransaction`, `CreateVspPayment` (through `AddTransaction`), `SendOutputs*` (through `recordAuthoredTx`) | transaction store and address manager | — | — | Write through `processTransactionRecord` (`wallet/chainntfns.go:438`) |
| `TotalReceivedForAccounts` | `waddrmgr` / `acct` | account number `uint32` **little-endian** | account row | ForEach in bbolt key order (`wallet/udb/addressdb.go:698-708`) |
| `NeedsAccountsSync`, `Open` | `waddrmgr` / `meta` | `lastaccount`, `lastimportedaccount` | `uint32` LE (an absent `lastimportedaccount` reads as `0x7FFFFFFF`) | Read (`wallet/udb/addressdb.go:711-740`) |
| `LiveTicketHashes` | `wtxmgr` / `tix` | 32-byte ticket hash | ticket record | Cursor, key order (`wallet/udb/txmined.go:2751`) |
| notifications (`totalBalances`, unmined hashes) | `wtxmgr` / `u`, `mc`, `mi`, `m`, `up`, `c` (`wallet/udb/txmined.go:2463-2531`, `wallet/udb/txunmined.go:320-335`) | — | — | Read |

VSP fee status values (`wallet/udb/vsp.go:24-36`): 0 `VSPFeeProcessStarted`,
1 `VSPFeeProcessPaid`, 2 `VSPFeeProcessErrored`, 3 `VSPFeeProcessConfirmed`.

### 4.2 In-memory notification structures and their wire mapping

The Go structures in §3.2 are not serialised by this package. They become
observable only through the gRPC server (`internal/rpc/rpcserver/server.go`),
which is specified elsewhere; the field mapping is repeated here because it
fixes which fields of the Go structures matter.

**`TransactionNotifications` → `walletrpc.TransactionNotificationsResponse`**
(`internal/rpc/rpcserver/server.go:2398-2404`, `rpc/api.proto:681-703`):

| Go field | Proto field (number) | Conversion |
|---|---|---|
| `AttachedBlocks []Block` | `attached_blocks` (1) repeated `BlockDetails` | per block: `hash` (1) = `Header.BlockHash()` 32 bytes; `height` (2) = `int32(Header.Height)`; `timestamp` (3) = `Header.Timestamp.Unix()`; `approves_parent` (5) = `Header.VoteBits & 0x0001 != 0`; `transactions` (4); `prev_block` (6) = `Header.PrevBlock` 32 bytes (`server.go:2323-2343`) |
| `DetachedBlocks []*wire.BlockHeader` | `detached_blocks` (2) repeated bytes | `BlockHash()` of each header, same order |
| `DetachedBlocks` (again) | `detached_block_headers` (5) repeated `DetachedBlockDetails` | `hash` (1), `height` (2), `prev_block` (3), `timestamp` (4) |
| `UnminedTransactions []TransactionSummary` | `unmined_transactions` (3) | see below |
| `UnminedTransactionHashes []*chainhash.Hash` | `unmined_transaction_hashes` (4) repeated bytes | raw 32 bytes each |
| `NewBalances []AccountBalance` | **not sent** | computed, then dropped by the gRPC handler |

**`TransactionSummary` → `walletrpc.TransactionDetails`**
(`server.go:2212-2271`, `rpc/api.proto:137-165`):

| Go field | Proto field (number) | Conversion |
|---|---|---|
| `Hash` | `hash` (1) | raw 32 bytes |
| `Transaction` | `transaction` (2) | full wire serialisation of the stored transaction (§5.22 step 1) |
| `MyInputs` | `debits` (3) | `index` (1), `previous_account` (2), `previous_amount` (3, int64 atoms) |
| `MyOutputs` | `credits` (4) | `index` (1), `account` (2), `internal` (3), `amount` (4), `address` (5; empty string when `Address` is nil), `output_script` (6) |
| `Fee` | `fee` (5) | int64 atoms |
| `Timestamp` | `timestamp` (6) | Unix seconds |
| `Type` | `transaction_type` (7) | Regular→`REGULAR`(0), TicketPurchase→`TICKET_PURCHASE`(1), Vote→`VOTE`(2), Revocation→`REVOCATION`(3), Coinbase→`COINBASE`(4); anything else→`REGULAR`. The Go and proto enum numbers differ. |

**`AccountNotification` → `walletrpc.AccountNotificationsResponse`**
(`server.go:2426-2432`, `rpc/api.proto:706-712`): fields 1–5 map one to one
(`account_number`, `account_name`, `external_key_count`, `internal_key_count`,
`imported_key_count`).

**`ConfirmationNotification` → `ConfirmationNotificationsResponse.TransactionConfirmations`**
(`server.go:2473-2488`, `rpc/api.proto:718-726`): `tx_hash` (1) raw 32 bytes,
`confirmations` (2), `block_hash` (3; empty when `BlockHash` is nil),
`block_height` (4). One response message is sent per `Recv` result, carrying
all entries of that result in order.

`MainTipChangedNotification` and `RemovedTransactionNotification` are never
serialised.

### 4.3 Transaction bytes produced

- `CreateSignature` returns `sig = DER(ECDSA signature) || byte(hashType)` and
  `pubkey = 33-byte compressed secp256k1 public key`
  (`wallet/wallet.go:4941-4947`, `dcrd/txscript@v4.1.2/sign/sign.go:28-66`).
  dcrd's `ecdsa.Sign` is deterministic (RFC 6979), so the bytes are a pure
  function of key, transaction, index, script and hash type.
- `SignTransaction` writes signature scripts produced by dcrd
  `sign.SignTxOutput` with `isTreasuryEnabled = true`
  (`wallet/wallet.go:4856-4857`); layouts are those of
  `dcrd/txscript@v4.1.2/sign/sign.go` (P2PKH: `<sig> <pubkey>`; P2PK:
  `<sig>`; P2SH: `<inner sig script ops> <redeem script>`; multisig: up to
  `nRequired` signatures in pubkey order, `OP_0` padding only after a merge
  with a previous script).
- `CreateVspPayment` produces a transaction with one fee output and at most
  one P2PKH change output in random order (§5.4).

---

## 5. Algorithms and state machines

Conventions used below. "View" means `walletdb.View` (one read-only database
transaction, rolled back at the end); "Update" means `walletdb.Update` (one
read-write transaction, committed if the closure returns nil, otherwise rolled
back) (`wallet/walletdb/interface.go:203-223`, `230`). The `ctx` argument of
both is used only for runtime trace regions; cancelling it does not abort the
transaction. The exception inside a transaction is `Store.RangeTransactions`
(used by §5.1, §5.18, §5.20 and `unstableAPI.RangeTransactions`), which checks
`ctx.Err()` before every block and before every unmined record and returns it
unwrapped (`wallet/udb/txquery.go:358-360`, `425-427`); the wrapping op is then
added by the caller. `E(op, err)` means
`errors.E(op, err)` from the wallet `errors` package (§8). `confirms(h, tip)`
is `0` when `h == -1` or `h > tip`, else `tip - h + 1`
(`wallet/wallet.go:4411-4418`).

### 5.1 Received totals

**`TotalReceivedForAccounts(ctx, minConf)`** (`wallet/wallet.go:4460-4522`),
op `wallet.TotalReceivedForAccounts`. One View:

1. `tipHeight` ← `Store.MainChainTip`.
2. For each account row in `waddrmgr/acct` in bbolt key order (key is the
   account number as 4 bytes little-endian, so the order is lexicographic on
   those bytes, **not** numeric): fetch the account name; append
   `{AccountNumber, AccountName}` to `results` and record its slice index in
   `resultIdxs`. The reserved imported account (`0x7FFFFFFF`, key
   `ff ff ff 7f`) has a row and is included. Any error aborts.
3. `stopHeight` ← `tipHeight - minConf + 1` if `minConf > 0`, else `-1`.
4. `Store.RangeTransactions(ctx, txmgrNs, 0, stopHeight, rangeFn)`. With the
   store's conventions (`wallet/udb/txquery.go:521-538`, `388-398`): a negative
   end height means "to the tip and then the unmined set". So:
   - `minConf <= 0`: all mined blocks ascending, then unmined transactions.
   - `0 < minConf <= tipHeight + 1`: blocks `0..=stopHeight` ascending (when
     `stopHeight == 0` the iteration runs backwards over block 0 only).
   - `minConf > tipHeight + 1`: `stopHeight` is negative, so **every** mined
     block and the unmined set are included (see Quirk Q1).
5. `rangeFn`, for each transaction detail and each credit in it: extract
   addresses from the credited output's `(Version, PkScript)`; skip if none;
   look up the account of the **first** address with `Manager.AddrAccount`;
   on success add `cred.Amount` to that account's `TotalReceived` and set its
   `LastConfirmation = confirms(detail.Block.Height, tipHeight)` (overwrite;
   the last credit visited wins, so with ascending iteration it is the
   confirmation count of the most recent counted credit, or 0 if that credit
   is unmined). A lookup error is ignored. If the account is somehow missing
   from `resultIdxs` the Go map yields index 0 and the amount is credited to
   the first row.
6. Any error → `E(op, err)`. Result slice is nil when the account bucket is
   empty.

Spent credits are counted (this is "received", not "balance"). The store's
credit iterator reads only the regular credits bucket
(`wallet/udb/txdb.go:1030-1033`); credits of regular-tree transactions in
stake-invalidated blocks are moved to a separate bucket by the store
(`wallet/udb/txmined.go:812-825`) and so are not counted.

**`TotalReceivedForAddr(ctx, addr, minConf)`** (`wallet/wallet.go:4527-4568`),
op `wallet.TotalReceivedForAddr`. Same range and `stopHeight` rule. For each
credit: extract addresses from the credited output; if any extracted address's
`String()` equals `addr.String()`, add `cred.Amount` once (break after the
first match). No check is made that `addr` belongs to the wallet; an unknown
address yields `0, nil`.

### 5.2 `SendOutputs` and `SendOutputsToTreasury`

`wallet/wallet.go:4572-4643`. Ops `wallet.SendOutputs` and
`wallet.SendOutputsToTreasury`.

1. `relayFee` ← `w.RelayFee()`.
2. For each output in order: `txrules.CheckOutput(output, relayFee)`
   (`wallet/txrules/rules.go:79-90`); first failure returns `E(op, err)` with
   kind `Invalid` ("transaction output amount is negative", "transaction
   output amount exceeds maximum value") or `Policy` ("transaction output is
   dust").
3. Build `authorTx{outputs, account, changeAccount, minconf,
   randomizeChangeIdx, txFee: relayFee, dontSignTx: false, isTreasury}` with
   `randomizeChangeIdx = true, isTreasury = false` for `SendOutputs` and
   `randomizeChangeIdx = false, isTreasury = true` for
   `SendOutputsToTreasury`.
4. `w.authorTx(ctx, op, a)` (`wallet/createtx.go:388`; other spec). Error
   returned as is.
5. `w.recordAuthoredTx(ctx, op, a)` (`wallet/createtx.go:510-538`): takes
   `lockedOutpointMu`, one Update that persists change-address usage and calls
   `processTransactionRecord(rec, nil, nil)`. This is where the unmined
   transaction notification fires (§5.3). Error returned as is.
6. `w.publishAndWatch(ctx, op, nil, a.atx.Tx, a.watch)`
   (`wallet/createtx.go:331-365`): resolves the network backend (if none, the
   error is `E(op, err)` around `errors.E("wallet.NetworkBackend", NoPeers)`
   from `wallet/network.go:61-71`; kind `NoPeers`, rendered
   `<op>: Decred network is unreachable<Separator>wallet.NetworkBackend`,
   **leaving the transaction recorded**, Quirk Q9);
   publishes; on publish failure logs
   `Abandoning transaction %v which failed to publish`, calls
   `AbandonTransaction`, returns `E(op, err)`; on success reloads the address
   filter and watches `a.watch`.
7. Return the transaction hash.

### 5.3 Notification firing points and ordering

| Notification method | Called from | Inside DB transaction? | Locks held by caller |
|---|---|---|---|
| `notifyAttachedBlock` | `extendMainChain` after `Store.ExtendMainChain` succeeds or returns `Exist` (`wallet/chainntfns.go:46-52`) | yes, the `ChainSwitch` Update | `lockedOutpointMu` |
| `notifyDetachedBlock` | `ChainSwitch`, once per block being disconnected, tip first, before the rollback (`wallet/chainntfns.go:125-146`) | yes | `lockedOutpointMu` |
| `notifyMinedTransaction` | `processTransactionRecord` when `header != nil` and `Store.UniqueTxDetails` returned no error (`wallet/chainntfns.go:666-672`; the returned details pointer is not nil-checked). Reached from `extendMainChain`, `AddTransaction` with a main-chain block hash, and rescan (`wallet/rescan.go:200`) | yes | `lockedOutpointMu` (also in rescan, `wallet/rescan.go:265-266`) |
| `notifyUnminedTransaction` | `processTransactionRecord` when `header == nil` and `Store.UniqueTxDetails` returned no error (`wallet/chainntfns.go:659-665`; the returned details pointer is not nil-checked). Reached from `PublishTransaction` (`wallet/wallet.go:5138`), `AddTransaction` (unmined, `wallet/chainntfns.go:416`), `recordAuthoredTx` (`wallet/createtx.go:531`), ticket purchase (split transaction and each ticket, `wallet/createtx.go:1467,1581`) and vote creation in `VoteOnOwnedTickets` (`wallet/chainntfns.go:907`). There is no revocation-creation path at the pin: `RevokeOwnedTickets` is an empty function returning nil (`wallet/chainntfns.go:931-933`) | yes, a write transaction | `lockedOutpointMu`, except the vote-creation path, which deliberately does not take it (`wallet/chainntfns.go:904`) |
| `notifyRemovedTransaction` | `ChainSwitch` for each hash returned by `Store.PruneUnmined` (`wallet/chainntfns.go:261-262`) | yes | `lockedOutpointMu` |
| `notifyRemovedTransaction` | `AbandonTransaction` after its Update commits (`wallet/wallet.go:5090`) | no | none |
| `notifyAccountProperties` | `RenameAccount` (`wallet/wallet.go:2130`), `NextAccount` (2236), `ImportPrivateKey` (3734), `ImportPublicKey` (3771), each after the Update commits and after any filter load | no | none |
| `notifyMainChainTipChanged` then `sendAttachedBlockNotification` | end of a successful `ChainSwitch`, in that order (`wallet/chainntfns.go:312-313`) | no | none |

Per-`ChainSwitch` order observed by subscribers:

1. Inside the Update: zero or more removed-transaction notifications (pruned
   unmined transactions) — after all blocks were attached in the database
   transaction but before it commits.
2. After commit: one `MainTipChangedNotification` to every tip client, in
   client-slice order.
3. Then every confirmation client is processed concurrently and each emits
   one result (§5.28); `notifyMainChainTipChanged` returns only when all have
   been delivered or their contexts are done.
4. Then one `TransactionNotifications` carrying the accumulated attached and
   detached blocks.

Account notifications are **not** fired when addresses are derived or marked
used, when scripts are imported, or when xpub or voting accounts are imported
(only the four call sites above exist), despite the type comment
(`wallet/notifications.go:589-591`).

### 5.4 `CreateVspPayment`

`wallet/wallet.go:4665-4762`. No op wrapping; errors are plain.

1. If `len(tx.TxIn) == 0`:
   1. `inputs, err := w.ReserveOutputsForAmount(ctx, feeAcct, fee, 1)`
      (`wallet/createtx.go:1672`; minimum confirmations 1; locks the selected
      outpoints). Error → `fmt.Errorf("unable to reserve outputs: %w", err)`.
   2. For each input: `tx.AddTxIn(wire.NewTxIn(&in.OutPoint, in.PrevOut.Value, nil))`.
   3. Defer: unlock every reserved outpoint with `UnlockOutpoint` when the
      function returns, on every path.
2. `input` ← sum of `TxIn.ValueIn`. If `input < fee` →
   `fmt.Errorf("not enough input value to pay fee: %v < %v", dcrutil.Amount(input), fee)`
   (amounts render as e.g. `0.0001 DCR`).
3. `vers, feeScript := feeAddr.PaymentScript()`.
4. `addr, err := w.NewChangeAddress(ctx, changeAcct)`; on error log at WARN
   `Failed to get new change address: %v` and return the error. The address
   must implement the wallet `Address` interface (provides `PaymentScript`);
   otherwise `fmt.Errorf("failed to convert '%T' to wallet.Address", addr)`.
   `changeOut = &wire.TxOut{PkScript: script, Version: vers}`.
5. Replace all outputs: `tx.TxOut = [ {Value: fee, Version: vers, PkScript: feeScript} ]`.
6. `feeRate` ← `w.RelayFee()`. `scriptSizes` ← `RedeemP2PKHSigScriptSize`
   (108) for every input. `est = txsizes.EstimateSerializeSize(scriptSizes,
   tx.TxOut, P2PKHPkScriptSize)` (25; i.e. the estimate includes one P2PKH
   change output). `change = input - fee - FeeForSerializeSize(feeRate, est)`.
7. If `!txrules.IsDustAmount(change, 25, feeRate)`: set `changeOut.Value =
   change`, append it, and call
   `txauthor.RandomizeOutputPosition(tx.TxOut, len(tx.TxOut)-1)`, which swaps
   the change output with a uniformly random index in `[0, len)`
   (`wallet/txauthor/author.go:171-175`). Otherwise no change output is added
   and the remainder becomes fee, including when `change` is negative (Quirk
   Q12).
8. `feeHash := tx.TxHash()` (computed before signing; the hash does not cover
   signature scripts).
9. `sigErrs, err := w.SignTransaction(ctx, tx, txscript.SigHashAll, nil, nil, nil)`.
   If `err != nil || len(sigErrs) > 0`:
   - log ERROR `Failed to sign transaction: %v` with `err` (may be `<nil>`);
   - for each `sigErr`: log ERROR `"\t%v"` with the struct; set
     `sigErrStr = fmt.Sprintf("\t%v", sigErr) + " "` (assignment, so only the
     last one survives);
   - if `err != nil` return `err`; else return `fmt.Errorf("%s", sigErrStr)`.
10. `w.SetPublished(ctx, &feeHash, false)` — its own Update, **before** the
    transaction exists in the store.
11. `w.AddTransaction(ctx, tx, nil)` (`wallet/chainntfns.go:324`) — records the
    transaction as unmined; fires the unmined notification.
12. Return nil. The caller (`wallet/feepayment.go`) later flips the published
    flag and broadcasts.

### 5.5 `SignTransaction`

`wallet/wallet.go:4773-4909`. Op `wallet.SignTransaction`. The whole loop runs
inside one View. `doneFuncs` collects key-zeroing callbacks, all run when the
function returns.

For each input `i` (ascending):

1. If `i == 0 && stake.IsSSGen(tx)`: skip entirely (stakebase input).
2. Previous output script: `additionalPrevScripts[txIn.PreviousOutPoint]`
   (map key includes hash, index **and tree**). If absent: `Store.TxDetails`
   for the previous transaction hash; `NotExist` → fatal
   `errors.Errorf("%v not found", &txIn.PreviousOutPoint)` (renders
   `<hash>:<index>`); other error → fatal. Script =
   `txDetails.MsgTx.TxOut[prevIndex].PkScript` (no bounds check; an
   out-of-range index panics in Go).
3. Key source `source.key(addr)`:
   - If `len(additionalKeysByAddress) != 0`: look up `addr.String()`; missing →
     `errors.Errorf("no key for address (needed: %v, have %v)", addr, additionalKeysByAddress)`
     (see Quirk Q6); found → `(wif.PrivKey(), STEcdsaSecp256k1, true, nil)`.
   - Else `Manager.PrivateKey(addrmgrNs, addr)`; error returned; on success
     append its `done` to `doneFuncs` and return
     `(key.Serialize(), STEcdsaSecp256k1, true, nil)`.
   The compressed flag is always `true` and the signature type is always
   ECDSA-secp256k1.
4. Script source `source.script(addr)`:
   - If `len(additionalKeysByAddress) != 0`: look up
     `p2shRedeemScriptsByAddress[addr.String()]`; missing →
     `errors.New("no script for address")`.
   - Else `Manager.RedeemScript(addrmgrNs, addr)`.
5. If `(hashType & SigHashSingle) != SigHashSingle || i < len(tx.TxOut)`:
   `script, err := sign.SignTxOutput(params, tx, i, prevOutScript, hashType,
   source, source, txIn.SignatureScript, true)`. On error append
   `SignatureError{uint32(i), E(op, err)}` and `continue` (no engine run for
   this input). On success `txIn.SignatureScript = script`.
6. Validate: `txscript.NewEngine(prevOutScript, tx, i, sanityVerifyFlags,
   0, nil)` then `Execute()`. `sanityVerifyFlags` =
   `ScriptDiscourageUpgradableNops | ScriptVerifyCleanStack |
   ScriptVerifyCheckLockTimeVerify | ScriptVerifyCheckSequenceVerify |
   ScriptVerifyTreasury` (`wallet/createtx.go:45-49`). Script version 0 is
   assumed.
7. If the engine errored:
   - If `errors.Is(err, txscript.ErrInvalidStackOperation)`: take
     `pkScript := additionalPrevScripts[txIn.PreviousOutPoint]` (**only** the
     caller-supplied map; nil if the script came from the wallet database),
     `class, addr := ExtractAddrs(0, pkScript, params)`; if `class ==
     STScriptHash && len(addr) > 0` and `source.script(addr[0])` returns a
     version-0 multisig script, set `multisigNotEnoughSigs = true`.
   - Else `errEval = err` (overwrites any earlier value).
   - Unless `multisigNotEnoughSigs`, append `SignatureError{uint32(i), E(op, err)}`.

After the loop the View returns `errEval`. If the View's error is non-nil the
function returns `(signErrors, E(op, err))`; otherwise `(signErrors, nil)`.

Consequences to preserve:

- A script-engine failure other than a stack underflow is reported **twice**:
  once in the `SignatureError` list and once as the function's fatal error
  (the last such failure). JSON-RPC `signrawtransaction` maps "any
  `SignatureError` or non-nil error" to `complete: false`
  (`internal/rpc/jsonrpc/methods.go:5094-5118`).
- A partially signed P2SH multisig input is silent (no `SignatureError`) only
  when the P2SH output script was passed in `additionalPrevScripts`.
- `tx` is mutated even when errors are returned.
- When `additionalKeysByAddress` is non-empty the wallet's own keys and
  scripts are never consulted.
- The lookup in step 2 is the Go two-value map read: an outpoint present in
  `additionalPrevScripts` with a nil or empty script counts as supplied and
  the database is not consulted (`wallet/wallet.go:4799-4800`).
- The mask test in step 5 skips signing only for hash types whose low two
  bits are both set (`SigHashSingle` = 3, with or without
  `SigHashAnyOneCanPay`) on an input with no output at the same index. The
  engine check of step 6 still runs for such an input, against whatever
  signature script it already carries.
- A failure of `txscript.NewEngine` itself is handled exactly like a failure
  of `Execute` (`wallet/wallet.go:4872-4877`).
- The returned `[]SignatureError` is nil (not empty) when no input failed.
  When the fatal error is the "not found" or store error of step 2, the
  entries collected for earlier inputs are still returned with it, and inputs
  after the failing one are left untouched.
- Each `GetKey` call against the address manager appends one `done` callback;
  an address needed by several inputs is fetched, and later zeroed, once per
  use.

### 5.6 `CreateSignature`

`wallet/wallet.go:4914-4948`. Op `wallet.CreateSignature`.

1. One View: `Manager.PrivateKey(waddrmgrNs, addr)` → `privKey, done`;
   `pubKey = privKey.PubKey()`. Error → `(nil, nil, E(op, err))` (kinds from
   the manager: `Locked`, `NotExist`, `WatchingOnly`, …).
2. `sig, err = sign.RawTxInSignature(tx, int(idx), prevPkScript, hashType,
   privKey.Serialize(), dcrec.STEcdsaSecp256k1)`; error → `E(op, err)`.
3. Return `sig, pubKey.SerializeCompressed(), nil`.
4. Deferred: `done()` zeroes the key if it was obtained.

No bounds check on `idx` is made here; callers validate (the JSON-RPC handler
rejects negative indexes as of the pin commit).

### 5.7 `isRelevantTx` and `DetermineRelevantTxs`

`wallet/wallet.go:4952-5002`. A transaction is relevant if any of the
following holds, tested in this order with early return:

1. For each input, in order:
   a. `rs` ← data of the final push of the signature script
      (`stdscript.MultiSigRedeemScriptFromScriptSigV0`, which is just "final
      opcode data"; nil if the script is empty, fails to parse, or its last
      opcode is not a data push). If `rs != nil` and
      `Manager.ExistsHash160(Hash160(rs))` → relevant. Because the last push
      of a P2PKH spend is the public key, this also matches any input that
      spends from a wallet P2PKH address, whether or not the wallet knows the
      output.
   b. `Store.ExistsUTXO(&in.PreviousOutPoint)` (present in the unspent bucket,
      or an unmined credit; `wallet/udb/txquery.go:301-308`) → relevant.
2. For each output: `ExtractAddrs(out.Version, out.PkScript, params)`; for each
   address that implements `stdaddr.Hash160er` (P2PKH of any signature suite,
   P2SH; **not** pay-to-pubkey addresses), `Manager.ExistsHash160(hash160)` →
   relevant.

`DetermineRelevantTxs` runs the test for every transaction inside a single
View and returns `(relevant, nonRelevant, err)`; `err` is the View error,
unwrapped.

### 5.8 `appendRelevantOutpoints`

`wallet/wallet.go:5004-5060`. Returns the outpoints of `tx` the network
backend should watch.

1. If `relevant == nil`, allocate with capacity `len(tx.TxOut)`.
2. `op.Hash = tx.TxHash()`. `isTicket = DetermineTxType(tx) == TxTypeSStx`.
   `watchedTicketOutputZero = false`.
3. For each output `i`:
   1. If `isTicket && i > 0 && i is odd && !watchedTicketOutputZero`: decode
      the commitment address (`stake.AddrFromSStxPkScrCommitment`). On decode
      error `continue` to the next output. If the address has a hash160 known
      to the manager: append `{Hash, Index: 0, Tree: TxTreeStake}`, set
      `watchedTicketOutputZero = true`, `continue`. Otherwise fall through.
   2. `class, addrs = ExtractAddrs(out.Version, out.PkScript)`. `tree =
      TxTreeStake` if `txrules.StakeSubScriptType(class)` reports a stake
      type, else `TxTreeRegular`.
   3. For each address with a hash160 known to the manager: append
      `{Hash, Index: i, Tree: tree}`; if `isTicket && i == 0` set
      `watchedTicketOutputZero = true`; stop scanning this output's addresses.

Effect for tickets: output 0 is watched once if the wallet owns either the
voting address or any commitment (reward) address.

### 5.9 `AbandonTransaction`

`wallet/wallet.go:5073-5092`. Op is formatted:
`wallet.AbandonTransaction(<hash>)`.

1. One Update on the `wtxmgr` bucket:
   1. `details, err := Store.TxDetails(ns, hash)` — prefers an unmined record,
      else the most recent mined record; `NotExist` if neither
      (`wallet/udb/txquery.go:181-196`). Error returned.
   2. If `details.Block.Height != -1` →
      `errors.E(errors.Invalid, errors.Errorf("transaction %v is mined in main chain", hash))`.
   3. `Store.RemoveUnconfirmed(ns, &details.MsgTx, hash)` — recursively removes
      unmined spenders of each output, the transaction's unmined credits,
      ticket commitments, unmined-input markers, multisig spent flags, the
      unpublished flag and the unmined record.
2. On error return `E(op, err)`.
3. `w.NtfnServer.notifyRemovedTransaction(*hash)` — only for the requested
   hash, not for the recursively removed descendants.

`lockedOutpointMu` is not taken. Outpoints locked for the abandoned
transaction's inputs are not unlocked here.

### 5.10 `PublishTransaction`

`wallet/wallet.go:5108-5167`. Op `wallet.PublishTransaction(<hash>)` on every
error path.

1. `txHash = tx.TxHash()`.
2. View: `relevant = isRelevantTx(dbtx, tx)`. View error → `E(op, err)`.
3. If relevant:
   1. Serialise `tx` (`tx.Serialize`); error → `E(op, err)`.
   2. Lock `lockedOutpointMu`.
   3. Update: `rec = udb.NewTxRecord(serialised, time.Now())`;
      `watchOutPoints = processTransactionRecord(ctx, dbtx, rec, nil, nil)`.
      This inserts the transaction as unmined (or logs
      `Refusing to add unmined transaction %v since same transaction already exists mined`
      and returns no outpoints), removes its input outpoints from the
      in-memory lock set (skipped when the transaction is a vote or a
      revocation, `wallet/chainntfns.go:464-473`), records credits, and fires
      the unmined notification before the Update commits.
   4. Unlock `lockedOutpointMu`. Update error → `E(op, err)`.
4. `n.PublishTransactions(ctx, tx)`. On error: if relevant, call
   `AbandonTransaction(ctx, &txHash)` and log at WARN
   `Failed to abandon unmined transaction: %v` if that fails; return
   `E(op, err)`.
5. If `len(watchOutPoints) > 0`: `n.LoadTxFilter(ctx, false, nil,
   watchOutPoints)`; failure is only logged at ERROR
   `Failed to watch outpoints: %v`.
6. Return `&txHash`.

Not done here: no high-fee check (callers do it using `AllowsHighFees` and
`txrules.TxPaysHighFees`), no signature validation of inputs (contrast
`AddTransaction`), no `recentlyPublished` bookkeeping, no reload of the HD
address filter (contrast `publishAndWatch`).

### 5.11 `PublishUnminedTransactions`

`wallet/wallet.go:5172-5183`. Op `wallet.PublishUnminedTransactions`.

1. `w.UnminedTransactions(ctx)` (`wallet/wallet.go:4361-4379`): all unmined
   records in dependency order (`Store.UnminedTxs`), skipping those flagged
   unpublished. The records are collected into a Go map and topologically
   sorted (`wallet/udb/txunmined.go:285-316`, `wallet/udb/kahnsort.go:87-116`),
   so only the parent-before-child constraint is deterministic; the relative
   order of unrelated transactions follows map iteration. Error → `E(op, err)`.
2. `n.PublishTransactions(ctx, unminedTxs...)` — called even when the list is
   empty. Error → `E(op, err)`.

### 5.12 `NeedsAccountsSync`

`wallet/wallet.go:5196-5223`. One View:

1. `lastAcct = Manager.LastAccount`; error returned (with result `true`).
2. If `lastAcct != 0` → `false`.
3. Iterate address rows of account 0; on the first row set `false` and stop.
4. Otherwise `true`.

The View error is returned unwrapped. The chain tip is not examined, although
the doc comment says it is.

### 5.13 `ValidatePreDCP0005CFilters`

`wallet/wallet.go:5230-5268`. Op `wallet.ValidatePreDCP0005CFilters`.

1. `end` ← height `431488 - 1` on mainnet, `323328 - 1` on testnet3
   (`validate/validate.go:19-25`). Any other network →
   `E(op, "The current network does not have pre-DCP0005 cfilters")`.
2. Create a BLAKE-256 hasher (`blake256.New()`, 32-byte digest).
3. `w.RangeCFiltersV2(ctx, nil, end, rangeFn)` (`wallet/wallet.go:855-881`):
   for main-chain blocks from height 0 to `end` ascending, load the stored
   filter and write `filter.Bytes()` (the serialised form including the `N`
   prefix; empty for an empty filter) to the hasher. A missing filter or any
   store error → `E(op, err)` (the inner error already carries op
   `wallet.RangeCFiltersV2`). The loop simply stops at the tip if the wallet
   has fewer blocks.
4. `cfsethash` ← the 32 digest bytes, taken as a `chainhash.Hash` in digest
   byte order.
5. `validate.PreDCP0005CFilterHash(net, &cfsethash)`
   (`validate/validate.go:107-130`): decode the expected hash string with
   `chainhash.Decode` (byte-reversed hex) and compare. Expected strings:
   - mainnet `f95e09f9ded38f8d6c32e5158a1f286633881393659218c63f5ab0fc86b36c83`
   - testnet3 `619c08f5adda6f834212bbdaee3002fdc4efed731477af6c0fed490bbe2488d0`
   Mismatch → `errors.Errorf("hash of provided cfilters %s does not match expected hash %s", cfsethash, target)`
   (both printed byte-reversed), wrapped `E(op, err)`.

Equivalently: `reverse(BLAKE256(concat(filter bytes)))` rendered as hex must
equal the string above.

### 5.14 `ImportCFiltersV2`

`wallet/wallet.go:5278-5287`, `wallet/udb/cfilters.go:34-67`. Op
`wallet.ImportCFiltersV2`. One Update:

1. Position a block iterator at `startBlockHeight` in the main-chain block
   records.
2. For each `filterData[i]`:
   1. Advance the iterator; if exhausted →
      `errors.E(errors.NotExist, errors.Errorf("block height of %d unknown", startHeight+int32(i)))`.
   2. Parse with `gcs.FromBytesV2(blockcf2.B, blockcf2.M, fd)`; error returned.
   3. Read the block's stored header; filter key = header bytes `[36:52]`.
   4. Put `key || fd` under the block hash in `wtxmgr/cf`.
3. Any error rolls back all writes and is returned as `E(op, err)`.

No commitment check is made on the imported data. The iterator does not verify
that the first block returned is at `startBlockHeight` (a seek lands on the
next existing key).

### 5.15 `Create` and `CreateWatchOnly`

`wallet/wallet.go:5292-5323`.

`Create(ctx, db, pubPass, privPass, seed, params)`, op `wallet.Create`:

1. If `seed == nil`: `hdkeychain.GenerateSeed(32)`; error → `E(op, err)`.
2. If `len(seed) < 16 || len(seed) > 64` →
   `E(op, hdkeychain.ErrInvalidSeedLen)` (message
   `seed length must be between 128 and 512 bits`). A non-nil empty slice
   fails here.
3. `udb.Initialize(ctx, db.internal(), params, seed, pubPass, privPass)`;
   error → `E(op, err)`.

`CreateWatchOnly(ctx, db, extendedPubKey, pubPass, params)`, op
`wallet.CreateWatchOnly`: `udb.InitializeWatchOnly(...)`; error → `E(op, err)`.

### 5.16 `Open`

`wallet/wallet.go:5340-5530`. Op `wallet.Open`. Every error below is returned
as `E(op, err)`.

1. `db = cfg.DB.internal()`.
2. `udb.NeedsMigration(ctx, db)`; if true `udb.Migrate(ctx, db, cfg.Params)`.
3. `udb.Upgrade(ctx, db, cfg.PubPassphrase, cfg.Params)`.
4. If `params.Net == wire.TestNet3`: `minTestNetTarget = PowLimit >> 6`,
   `minTestNetDiffBits = standalone.BigToCompact(minTestNetTarget)`; otherwise
   both stay nil/0.
5. Build `deploymentsByID`: for every stake version in `params.Deployments`
   and every deployment, key by `Vote.Id`; a duplicate id panics with
   `reused deployment ID "<id>"`.
6. Construct the `Wallet` with: `votingEnabled`, `gapLimit`, `watchLast`,
   `allowHighFees`, `accountGapLimit`, `disableCoinTypeUpgrades`,
   `manualTickets`, `mixingEnabled`, `vspMaxFee`, `dialer` copied from `cfg`;
   empty maps for `tspends`, `tspendPolicy`, `tspendKeyPolicy`,
   `vspTSpendPolicy`, `vspTSpendKeyPolicy`, `lockedOutpoints`,
   `recentlyPublished`, `addressBuffers`, `vspClients`; a new subsidy cache;
   `mixSems = newMixSemaphores(cfg.MixSplitLimit)` (one channel of capacity
   `MixSplitLimit` per split point, `wallet/mixing.go:240-246`).
7. `w.manager, w.txStore = udb.Open(ctx, db, params, cfg.PubPassphrase)`
   (`wallet/udb/open.go:21-54`): fails with `NotExist` "database has not been
   initialized", `Invalid` "database upgrade required", `Invalid` "database has
   been upgraded to an unknown newer version", or a public-passphrase error
   from `loadManager`.
8. Log at INFO: `Opened wallet`.
9. Update: `w.rollbackInvalidCheckpoints(dbtx)` (`wallet/checkpoints.go:46-68`):
   on testnet3 only, if the main-chain block at height 962928 exists and its
   hash differs from the checkpoint, `Store.Rollback(dbtx, 962928)`. The
   checkpoint hash is
   `0000004fd1b267fd39111d456ff557137824538e6f6776168600e56002e23b93`
   (`wallet/checkpoints.go:22`, parsed with `chainhash.NewHashFromStr`, i.e.
   the usual byte-reversed display form). A `NotExist` from the height lookup
   (wallet not yet synced that far) skips the checkpoint; any other lookup
   error and any rollback error fail `Open`. The first return value of
   `Rollback` is discarded. On every other network the checkpoint list is
   empty and the Update commits without writing.
10. View:
    1. `lastAcct = Manager.LastAccount`, `lastImported =
       Manager.LastImportedAccount`.
    2. For `acct` in `0..=lastAcct`, then `acct` in
       `0x80000000..=lastImported` (imported-xpub accounts; the range is empty
       when `lastImported == 0x7FFFFFFF`): fetch the account xpub, derive the
       external (child 0) and internal (child 1) branch keys, fetch account
       properties, and store
       `addressBuffers[acct] = {xpub, albExternal{branchXpub, lastUsed:
       LastUsedExternalIndex, cursor: LastReturnedExternalIndex -
       LastUsedExternalIndex}, albInternal{... internal ...}}`. The
       subtraction is `uint32` wrapping; "never used" is `0xFFFFFFFF`, so
       `lastReturned = 0xFFFFFFFF, lastUsed = 0xFFFFFFFF` gives cursor 0 and
       `lastReturned = 4, lastUsed = 0xFFFFFFFF` gives cursor 5.
       The xpub comes from `Manager.AccountExtendedPubKey(tx, acct)`; the
       branch keys from `deriveBranches` (`wallet/addresses.go:1031-1038`),
       which is `xpub.Child(udb.ExternalBranch)` then
       `xpub.Child(udb.InternalBranch)` (non-hardened children 0 and 1). An
       error from any of the three lookups or from either derivation
       (including the BIP0032 invalid-child case) aborts the View and fails
       `Open`; there is no retry with another index. The imported account
       `0x7FFFFFFF` gets no address buffer.
    3. `vb = w.readDBVoteBits(tx)` (`wallet/wallet.go:316-343`).
    4. `readDBTreasuryPolicies`, `readDBTreasuryKeyPolicies`
       (`wallet/wallet.go:377-395`).
11. `w.NtfnServer = newNotificationServer(w)`; assign `defaultVoteBits` and
    the four treasury policy maps (replacing the empty maps from step 6).
12. `w.relayFee = cfg.RelayFee`.
13. `_, w.initialHeight = w.MainChainTip(ctx)` (one more View; errors ignored).
14. If `mixingEnabled`: `w.mixpool = mixpool.NewPool((*mixpoolBlockchain)(w))`.
15. Return `w`.

The database is not closed by `Open` on failure; the caller owns it.

`Open` reads every field of `Config` (`wallet/wallet.go:183-204`): `DB`,
`PubPassphrase`, `VotingEnabled`, `GapLimit`, `WatchLast`, `AccountGapLimit`,
`MixSplitLimit`, `DisableCoinTypeUpgrades`, `MixingEnabled`, `ManualTickets`,
`AllowHighFees`, `RelayFee`, `VSPMaxFee`, `Params`, `Dialer`. None is
validated here (for example a zero `GapLimit` is accepted). `cfg.PubPassphrase` is passed to
both `udb.Upgrade` and `udb.Open`. All other `Wallet` fields keep their Go
zero values after `Open`, in particular the network backend (nil until a
syncer sets it), `mixClient` (nil until `Run`) and `mixpool` when mixing is
disabled.

### 5.17 `Run`

`wallet/wallet.go:5545-5552`. If `mixingEnabled`: build a new
`mixclient.Client` over the wallet (`newMixClient`), set its logger to the
`MIXC` subsystem, store it atomically in `w.mixClient`, and return
`c.Run(ctx)`. Otherwise return nil at once. Called once per syncer run
(`chain/sync.go:733`, `spv/sync.go:450`); every call creates and stores a new
client.

### 5.18 Coinjoin output counts

`wallet/wallet.go:5556-5609`. Inner op `wallet.getCoinjoinTxsSumbByAcct`,
outer op `wallet.GetCoinjoinTxsSumbByAcct`. One View:

1. `tipHeight` ← `Store.MainChainTip`.
2. `Store.RangeTransactions(ctx, txmgrNs, 0, tipHeight, rangeFn)` — mined
   transactions only.
3. For each detail: skip unless `detail.TxType == TxTypeRegular`; `isMix,
   mixDenom, _ = PossibleCoinJoin(&detail.MsgTx)` (`wallet/mixing.go:517-553`);
   skip unless `isMix`. For each output with `Value == mixDenom`: extract
   addresses; if exactly one address and `Manager.AddrAccount` succeeds,
   `count[account]++`.

The result counts outputs (not transactions, not amounts) and includes outputs
regardless of whether they are recorded as credits or spent.

### 5.19 VSP record queries

`wallet/wallet.go:5613-5701`.

- `GetVSPTicketsByFeeStatus(ctx, feeStatus)`, op
  `wallet.GetVSPTicketsByFeeStatus`: View → `udb.GetVSPTicketsByFeeStatus`
  (full scan of the `vsp` bucket, deserialising each 40-byte value). The
  result is copied out of a Go map, so the order of hashes is unspecified.
  The slice is allocated with `make` and is non-nil even when empty. The
  comparison is `int(record.FeeTxStatus) == feeStatus`
  (`wallet/udb/vsp.go:134`).
- `VSPFeeHashForTicket`: View → `udb.GetVSPTicket`; returns `data.FeeHash`.
  If the lookup succeeded and the hash is all zero, the error becomes
  `errors.E(errors.NotExist)` (no message). Errors are not op-wrapped.
- `VSPHostForTicket`: same, returning `data.Host`; empty host with no error
  becomes `errors.E(errors.NotExist)`.
- `IsVSPTicketConfirmed`: View → `udb.GetVSPTicket`; true iff
  `data.FeeTxStatus == 3`. Missing record → `NotExist` with message
  `no VSP info for ticket <hash>` (`wallet/udb/vsp.go:100-104`).

### 5.20 `ForUnspentUnexpiredTickets`

`wallet/wallet.go:5705-5747`.

1. `blockHeight` ← `w.MainChainTip(ctx)` height.
2. `startBlockNum = blockHeight - int32(TicketExpiry + uint32(TicketMaturity) - 8)`;
   start identifier = that height, end identifier = `blockHeight`.
3. `w.GetTickets(ctx, iter, start, end)` (`wallet/wallet.go:3176`; no RPC, so
   statuses are `unmined`, `immature`, `unspent`, `voted`, `missed`,
   `expired`). `iter` calls `f(&hashCopy)` for every summary whose status is
   `live`, `immature` or `unspent`; an error from `f` is collected and
   iteration continues. `iter` never stops early.
4. A `GetTickets` error is appended to the collected errors.
5. Return `errors.Join(errs...)` (nil when empty; the joined message is the
   individual messages separated by newlines).

Behaviour of the range when `startBlockNum < 0` is described in Quirk Q3.

### 5.21 `UnprocessedTickets` and `ProcessedTickets`

`wallet/wallet.go:5751-5828`. Both iterate with `ForUnspentUnexpiredTickets`
and then build a `*VSPTicket` for each selected hash with `w.NewVSPTicket`
(`wallet/vspticket.go:41`); the first construction error aborts and is
returned. Each `VSP…ForTicket` call inside the callback opens its own View.

`UnprocessedTickets` selects a hash when:

1. `VSPFeeHashForTicket` returns an error (no record, or zero fee hash, or a
   database error), **and**
2. `IsVSPTicketConfirmed` returns `(false, nil)` or a `NotExist` error. Any
   other error is returned from the callback (collected, joined, and makes the
   whole call fail).

`ProcessedTickets` selects a hash when `IsVSPTicketConfirmed` returns
`(false, nil)`. `NotExist` skips the ticket; other errors fail the call.

Both return an empty non-nil slice when nothing is selected. Order follows the
`GetTickets` iteration order.

### 5.22 `makeTxSummary` and `TxTransactionType`

`wallet/notifications.go:114-181`, `430-442`.

1. `Transaction` ← `details.SerializedTx` if non-nil, else
   `details.MsgTx.Serialize()` into a fresh buffer. Records read from the
   database never carry `SerializedTx` (`wallet/udb/txdb.go:561-576`), so in
   practice this is always a re-serialisation. A serialisation error is logged
   at ERROR `Transaction serialization: %v` and the (partial) buffer is used.
2. `Fee`: if `len(details.Debits) == len(details.MsgTx.TxIn)` (every input is
   a wallet debit): sum of debit amounts minus sum of all output values.
   Otherwise 0. Coinbases and votes therefore always report fee 0 (their
   null-outpoint input is never a debit), as does any transaction with at
   least one foreign input. No clamp is applied; the value is whatever the
   signed subtraction yields.
3. `MyInputs`: nil if there are no debits; else one entry per debit, in debit
   order: `{Index: d.Index, PreviousAccount: lookupInputAccount(d),
   PreviousAmount: d.Amount}`.
   `lookupInputAccount` (`wallet/notifications.go:59-87`): fetch the previous
   transaction with `Store.TxDetails`; take its output at the previous index;
   extract addresses; return `Manager.AddrAccount(addrs[0])`. Returns account
   0 and logs at ERROR on a failed transaction lookup
   (`Cannot query previous transaction details for %v: %v`), a nil result
   (`Missing previous transaction %v`), or a failed account lookup
   (`Cannot fetch account for previous output %v: %v`); returns 0 silently
   when the script yields no address.
4. `MyOutputs`: always a non-nil slice. Walk output indexes ascending with a
   credit cursor `credIndex = len(outputs so far)`; output `i` is "mine" iff
   `details.Credits[credIndex].Index == i` (credits are index-ordered). For
   each: `lookupOutputChain` (`wallet/notifications.go:89-112`) extracts
   addresses and calls `Manager.Address(addrs[0])`; on success returns
   `(ma.Account(), ma.Internal(), ma.Address(), output.Value, output.PkScript)`.
   With no address, or on lookup error (logged at ERROR
   `Cannot fetch account for wallet output: %v`), **all five results are zero
   values**: account 0, internal false, nil address, amount 0, nil script. The
   entry is still emitted with the correct `Index`.
5. `Type` ← `TxTransactionType(&details.MsgTx)`:
   1. `compat.IsEitherCoinBaseTx` → Coinbase. That function is
      `standalone.IsCoinBaseTx(tx, false) || standalone.IsCoinBaseTx(tx, true)`
      (`internal/compat/compat.go`), and the first call is true for **any** transaction
      with exactly one input whose previous outpoint has a zero hash and index
      `0xFFFFFFFF` (`dcrd/blockchain/standalone@v2.3.0/tx.go:49-102`). Treasury
      base and treasury spend transactions therefore classify as Coinbase.
   2. `stake.IsSStx` → TicketPurchase.
   3. `stake.IsSSGen` → Vote.
   4. `isRevocation` (= `stake.IsSSRtx`, `wallet/wallet.go:3977-3979`) → Revocation.
   5. Otherwise Regular (includes treasury adds).
6. `Timestamp`: `details.Received`; if the transaction is mined
   (`details.Height() >= 0`) and the block time is earlier, the block time.
   Converted with `.Unix()`. Stored receive times have one-second resolution.
7. `Hash` ← pointer to `details.Hash`.

### 5.23 `notifyUnminedTransaction`

`wallet/notifications.go:225-261`. Runs with the server mutex held for its
whole duration, inside the caller's database write transaction.

1. If `currentTxNtfn != nil` log at TRACE
   `Notifying unmined tx notification while creating notification for blocks`.
2. If there are no transaction clients, return (no work).
3. `unminedTxs = [makeTxSummary(details)]`.
4. `unminedHashes = Store.UnminedTxHashes` — all keys of `wtxmgr/m` in key
   order (ascending raw hash bytes), nil when empty. Error → log ERROR
   `Cannot fetch unmined transaction hashes: %v`, return without sending.
5. `bals` ← map with a zero entry for every `PreviousAccount` and `Account` in
   the summary (`relevantAccounts`), then `totalBalances` adds, for every
   unspent output (`Store.UnspentOutputs`: mined unspent outputs not spent by
   an unmined transaction, plus unmined credits that are neither spent by an
   unmined transaction nor from an unpublished transaction), its amount to the
   entry of the account owning the first address of its script (version 0
   assumed), if that account is in the map. Error → log ERROR
   `Cannot determine balances for relevant accounts: %v`, return without
   sending.
6. Build `&TransactionNotifications{UnminedTransactions, UnminedTransactionHashes,
   NewBalances: flattenBalanceMap(bals)}` and send the same pointer to every
   transaction client in slice order, blocking on each.

Because the transaction being notified has already been inserted in the same
database transaction, its own hash is in `UnminedTransactionHashes` and its
effects are in the balances.

### 5.24 Attached-block accumulation and `sendAttachedBlockNotification`

State: `currentTxNtfn *TransactionNotifications` (nil when idle), protected by
the server mutex.

- `notifyDetachedBlock(header)`: create `currentTxNtfn` if nil; append
  `header` to `DetachedBlocks`. Called tip-first, so `DetachedBlocks` is in
  descending height order.
- `notifyAttachedBlock(header, hash)`: create if nil; append
  `Block{Header: header}` unless the last attached block's
  `Header.BlockHash()` already equals `hash`.
- `notifyMinedTransaction(dbtx, details, block)`: create if nil; if there is no
  attached block or the **last** attached block's hash differs from
  `block.Hash`, return without recording anything; otherwise append
  `makeTxSummary(details)` to that block's `Transactions`.
- `sendAttachedBlockNotification(ctx)` (`wallet/notifications.go:306-356`):
  1. Lock. If there are no transaction clients: set `currentTxNtfn = nil`,
     unlock, return.
  2. Take `currentTxNtfn` into a local, set the field to nil, unlock.
  3. One View (using `ctx`): `unminedHashes = Store.UnminedTxHashes`; for each
     attached block, `relevantAccounts(bals, block.Transactions)`;
     `totalBalances`. Error → log ERROR
     `Failed to construct attached blocks notification: %v` and return; the
     accumulated notification is lost.
  4. Set `UnminedTransactionHashes` and `NewBalances` on the local. The
     `UnminedTransactions` field is deliberately left nil.
  5. Lock; send the pointer to every transaction client currently registered
     (clients that registered during step 3 receive it too); unlock.

Consequences:

- Transactions recorded as mined outside a `ChainSwitch` attach (rescan,
  `AddTransaction` with a block hash) never appear in any
  `TransactionNotifications`; they only leave behind an empty non-nil
  `currentTxNtfn`.
- A `ChainSwitch` whose database transaction fails after detaching or
  attaching blocks leaves those blocks in `currentTxNtfn`; they are delivered
  with the next successful chain switch (Quirk Q4).
- If `currentTxNtfn` is nil at step 2 while a client is registered, step 3
  dereferences nil (Quirk Q5).

### 5.25 `notifyAccountProperties`

`wallet/notifications.go:600-627`. With the server mutex held:

1. Return if there are no account clients.
2. Build `AccountNotification{AccountNumber, AccountName, ExternalKeyCount: 0,
   InternalKeyCount: 0, ImportedKeyCount: props.ImportedKeyCount}`.
3. If `props.AccountNumber <= udb.MaxAccountNum` (`0x7FFFFFFE`):
   `ExternalKeyCount = min(0x80000000, props.LastUsedExternalIndex + gapLimit)`
   and likewise for internal, with `uint32` wrapping addition. For an account
   with no used address (`LastUsed = 0xFFFFFFFF`) and gap limit 20 the count
   is 19; with last used index 0 it is 20.
4. Send the pointer to each account client, blocking on each.

`ImportedKeyCount` is non-zero only for the imported account
(`wallet/udb/addressmanager.go:550-601`). Imported-xpub accounts (numbers
above `0x7FFFFFFF`) get zero key counts.

### 5.26 `notifyMainChainTipChanged`

`wallet/notifications.go:730-750`. With the server mutex held throughout:

1. Send `n` to each tip client in slice order, blocking on each.
2. If there are confirmation clients: start one goroutine per client running
   `c.process(n.NewHeight)`, wait for all of them.
3. Unlock.

`n` is built by `ChainSwitch` (`wallet/chainntfns.go:98-102,123,140,184`):
`AttachedBlocks` is a non-nil slice of the attached block hashes in ascending
height; `DetachedBlocks` is nil when nothing was detached, else the detached
hashes in ascending height; `NewHeight` is the height of the last block of the
chain passed to `ChainSwitch`. It is sent even when no block was attached.

### 5.27 `ConfirmationNotificationsClient.Watch`

`wallet/notifications.go:820-899`.

1. Return immediately if `txHashes` is empty (no result is emitted).
2. One View (client context):
   1. `tipHeight` ← `Store.MainChainTip`.
   2. `i = 0`; while `i < len(txHashes)`:
      1. `h = txHashes[i]`; `height, err = Store.TxBlockHeight(h)` (−1 for an
         unmined transaction; `NotExist` if unknown).
      2. `confs`:
         - `NotExist` → −1.
         - other error → abort the View with it.
         - else if `tipHeight > height && height > 0`: load `Store.TxDetails(h)`
           (error aborts) and test
           `Store.BlockInMainChain(&txDetails.Block.Hash)`; if its
           `invalidated` flag is set → −1; otherwise
           `confirms(height, tipHeight)`.
         - else `confirms(height, tipHeight)`.
      3. Append `{TxHash: h, Confirmations: confs, BlockHeight: -1}`.
      4. If `confs > 0`: look up the height again, then
         `Store.GetMainChainBlockHashForHeight(height)`; set `BlockHash` and
         `BlockHeight`. Errors abort.
      5. If `confs >= stopAfter || confs == -1`: remove `h` from `txHashes` by
         moving the last element into slot `i` and shrinking the slice; do
         **not** advance `i`. Otherwise `i++`.
3. If the View failed, the result list is discarded (nil).
4. Send `{result, err}` on the client's result channel, or give up if the
   client context is done.
5. Lock the client mutex; for every hash left in `txHashes` set
   `watched[hash] = stopAfter` (overwriting an earlier `stopAfter`); unlock.
   This happens even when the View failed, with whatever was left in the
   slice at that point.

Result order follows the swap-remove traversal: for input `[A, B, C, D]` where
`A` is finished, the visiting order is `A, D, B, C`.

Further details of `Watch` and `process`:

- `Store.TxBlockHeight` (`wallet/udb/txquery.go:335-348`) returns −1 when an
  unmined record exists (checked first, so a transaction that is both unmined
  and mined reports 0 confirmations), otherwise the height of the most recent
  mined record, otherwise `NotExist`.
- The block hash reported for `confs > 0` is the **main-chain block hash at
  the transaction's height** (`GetMainChainBlockHashForHeight`), looked up by
  height, not read from the transaction record.
- `stopAfter` is compared as given. With `stopAfter <= 0` every hash is
  reported once and never watched (an unmined transaction has
  `0 >= stopAfter`). The Go API accepts negative values; the gRPC handler
  rejects `stop_after < 0` with `InvalidArgument`
  `stop_after must be non-negative` before calling `Watch`
  (`internal/rpc/rpcserver/server.go:2459-2462`).
- A hash listed twice in one `Watch` call produces two result entries and one
  watch entry. A hash already watched is re-reported and its `stopAfter`
  replaced.
- `TxHash` in `Watch` results is the caller's pointer; in `process` results it
  points at a per-iteration copy of the map key.
- The gRPC handler runs `Watch` (request reader) and `Recv` (response writer)
  in two goroutines (`server.go:2447-2494`), so the immediate result of one
  request and the per-tip results interleave in channel-rendezvous order; a
  stream receives them strictly one message per result.

### 5.28 `ConfirmationNotificationsClient.process`

`wallet/notifications.go:912-988`. Called once per main-tip change with the
new tip height.

1. Return if the client context is done.
2. Lock the client mutex.
3. `result` ← empty non-nil list. One View (client context): for every
   `(txHash, stopAfter)` in `watched` (Go map order): compute `confs` exactly
   as in §5.27 step 2.2.2 but using the `tipHeight` argument; append the
   entry; fill block hash/height when `confs > 0`; if
   `confs >= stopAfter || confs == -1` schedule the hash for removal.
4. On View error: `result = nil`, `err` set.
5. Remove scheduled hashes from `watched` (also after an error, for those
   scheduled before the failure). Unlock.
6. Send `{result, err}` on the result channel, or give up if the context is
   done.

A result is sent on **every** tip change, including an empty list when nothing
is watched.

### 5.29 `SelectInputs`

`wallet/utxos.go:22-52`. Op `wallet.SelectInputs`.

1. Lock `lockedOutpointMu` for the whole call.
2. One View: `tipHeight`; if `account != udb.ImportedAddrAccount`, load
   `LastAccount` and fail with `errors.E(errors.NotExist, "account not found")`
   when `account > lastAcct` (so imported-xpub accounts are rejected);
   `sourceImpl = Store.MakeInputSource(dbtx, account, requiredConfs, tipHeight,
   nil)`; `inputDetail = sourceImpl.SelectInputs(targetAmount)`.
3. Error → `E(op, err)`; `inputDetail` may still be returned alongside.

The `ignore` callback is nil, so outpoints in the wallet's locked set are
**not** excluded even though the lock is held. The store source returns the
inputs it gathered even if their total is below the target; it is the caller's
job to compare `InputDetail.Amount`. With `targetAmount == 0` the source
returns every eligible output (`wallet/udb/txmined.go:3101`). Selection order
is random (`wallet/udb/txmined.go:3103-3169`).

### 5.30 `OutputInfo`

`wallet/utxos.go:64-87`. Op `wallet.OutputInfo`. One View: `Store.TxDetails`
for `out.Hash` (error propagates, e.g. `NotExist`); if
`out.Index >= len(TxOut)` → `errors.Errorf("transaction has no output %d", out.Index)`;
else `Received = txDetails.Received`, `Amount = TxOut[index].Value`,
`FromCoinbase = compat.IsEitherCoinBaseTx(tx)`. `out.Tree` is ignored. The
output need not be a wallet credit.

### 5.31 `LiveTicketHashes`

`wallet/tickets.go:28-93`. Op `wallet.LiveTicketHashes`.

1. One View: `tipHeight`; iterate the tickets bucket in key order
   (`Store.IterateTickets`; tickets whose transaction record has been removed
   are skipped). For each ticket with block height `h` (−1 if unmined):
   1. If `ticketExpired(params, h, tipHeight)` → skip.
   2. If `!ticketMatured(params, h, tipHeight)`: if `includeImmature`, append
      the hash to the result; continue. Unmined tickets fall in this branch.
   3. Otherwise append to `maybeLive`.
   Iterator error → `E(op, err)`.
2. If `rpc == nil` (SPV) or `maybeLive` is empty → return the result so far.
3. `live, err := rpc.ExistsLiveTickets(ctx, maybeLive)`; error → `E(op, err)`.
4. Append every `maybeLive[i]` whose bit `i` is set.

Result order: immature (and unmined) tickets in bucket order, then live
tickets in bucket order. Whether a ticket has been spent is not consulted
locally; with an RPC backend a voted or revoked ticket is excluded because the
node reports it not live; in SPV mode no matured ticket is ever returned.

---

## 6. Constants, limits and defaults

| Name | Value | Source | What depends on it |
|---|---|---|---|
| `SStxChangeMaturity` | 1 (mainnet, testnet3, simnet, regnet) | `dcrd/chaincfg@v3.3.0/mainnetparams.go:556` etc. | `ticketChangeMatured` |
| `TicketMaturity` | 256 mainnet; 16 testnet3/simnet/regnet | `dcrd/chaincfg@v3.3.0/mainnetparams.go:553`, `testnetparams.go:527`, `simnetparams.go:562`, `regnetparams.go:549` | `ticketMatured`, `ticketExpired`, `ForUnspentUnexpiredTickets`, `LiveTicketHashes` |
| `TicketExpiry` | 40960 mainnet; 6144 testnet3; 384 simnet/regnet | same files | `ticketExpired`, `ForUnspentUnexpiredTickets` window |
| `requiredConfs` | `6 + 2` = 8 | `wallet/wallet.go:5734` | start height of the `ForUnspentUnexpiredTickets` window (mainnet window: tip − 41208 … tip) |
| VSP fee input `minconf` | 1 | `wallet/wallet.go:4673` | `CreateVspPayment` input reservation |
| `txsizes.RedeemP2PKHSigScriptSize` | 108 (`1 + 73 + 1 + 33`) | `wallet/txsizes/size.go:28` | VSP fee transaction size estimate |
| `txsizes.P2PKHPkScriptSize` | 25 | `wallet/txsizes/size.go:66` | VSP fee change output estimate and dust test |
| Dust rule | `amount*1000 / (3*(8+2+varintSize(scriptSize)+scriptSize+165)) < relayFeePerKb` | `wallet/txrules/rules.go:43-56` | change output inclusion in `CreateVspPayment`; `CheckOutput` in `SendOutputs*` |
| `FeeForSerializeSize` | `relayFee*size/1000`; if 0 and `relayFee > 0` then `relayFee`; clamped to `MaxAmount` | `wallet/txrules/rules.go:94-106` | VSP fee transaction fee |
| `sanityVerifyFlags` | `ScriptDiscourageUpgradableNops \| ScriptVerifyCleanStack \| ScriptVerifyCheckLockTimeVerify \| ScriptVerifyCheckSequenceVerify \| ScriptVerifyTreasury` | `wallet/createtx.go:45-49` | post-signing validation in `SignTransaction` |
| `scriptVersionAssumed` | 0 | `wallet/wallet.go:83` | engine version in `SignTransaction`; address extraction in `totalBalances` and the multisig check |
| `isTreasuryEnabled` argument to `SignTxOutput` | `true` | `wallet/wallet.go:4857` | treasury-gen outputs are signable |
| Signature type / compression | `STEcdsaSecp256k1`, compressed = true | `wallet/wallet.go:4824,4832,4942` | all wallet signatures |
| `validate.DCP0005ActiveHeightMainNet` | 431488 | `validate/validate.go:25` | `ValidatePreDCP0005CFilters` range end (431487) |
| `validate.DCP0005ActiveHeightTestNet3` | 323328 | `validate/validate.go:21` | range end (323327) |
| Pre-DCP0005 cfilter set hash, mainnet | `f95e09f9ded38f8d6c32e5158a1f286633881393659218c63f5ab0fc86b36c83` | `validate/validate.go:114` | validation result |
| Pre-DCP0005 cfilter set hash, testnet3 | `619c08f5adda6f834212bbdaee3002fdc4efed731477af6c0fed490bbe2488d0` | `validate/validate.go:112` | validation result |
| `gcs.KeySize` | 16 | `dcrd/gcs@v4.1.1/gcs.go:69` | cfilter value prefix length |
| `hdkeychain.RecommendedSeedLen` | 32 | `dcrd/hdkeychain@v3.1.3/extendedkey.go:30` | generated seed length in `Create` |
| `hdkeychain.MinSeedBytes` / `MaxSeedBytes` | 16 / 64 | `dcrd/hdkeychain@v3.1.3/extendedkey.go:40,44` | seed validation in `Create` |
| `maxTestDiffShift` | 6 | `wallet/wallet.go:5369` | `minTestNetTarget = PowLimit >> 6` (testnet3 only) |
| Testnet3 checkpoint height | 962928 | `wallet/checkpoints.go:50` | `rollbackInvalidCheckpoints` in `Open` |
| `udb.MaxAccountNum` | `0x7FFFFFFE` | `wallet/udb/addressmanager.go:36` | key-count branch in `notifyAccountProperties` |
| `udb.ImportedAddrAccount` | `0x7FFFFFFF` | `wallet/udb/addressmanager.go:47` | account check in `SelectInputs`; second address-buffer loop start (`+1`) in `Open` |
| `hdkeychain.HardenedKeyStart` | `0x80000000` | `dcrd/hdkeychain@v3.1.3/extendedkey.go:36` | cap on key counts in account notifications |
| `udb.VSPFeeProcessConfirmed` | 3 | `wallet/udb/vsp.go:24-36` | `IsVSPTicketConfirmed` |
| `TransactionType` values | 0 regular, 1 coinbase, 2 ticket purchase, 3 vote, 4 revocation | `wallet/notifications.go:408-427` | `TransactionSummary.Type` |
| `TicketStatus` values | 0..8, see §3.6 | `wallet/wallet.go:2851-2886` | `String()`, ticket selection |
| Notification channel capacity | 0 (unbuffered) for all five kinds, including the confirmation result channel | `wallet/notifications.go:489,540,639,696,757` | back-pressure (§7) |
| `errors.Separator` | package default `":\n\t"` (`errors/errors.go:27`); **the daemon overwrites it with `":: "` in an `init` function** (`dcrwallet.go:36-39`), so every error string the running daemon produces uses `":: "` | `errors/errors.go:27`, `dcrwallet.go:38` | rendering of nested wallet errors, including `SignatureError.Error` strings |
| Log subsystems | `WLLT` (package `wallet`), `MIXC` (mix client) | `internal/loggers/loggers.go:48,55` | log output |

---

## 7. Concurrency and lifecycle

### 7.1 Notification server

- One mutex `mu` protects the five client slices and `currentTxNtfn`
  (`wallet/notifications.go:40-51`). The struct comment says it protects only
  registered clients; in fact every `notify*` method and the accumulation
  state run under it.
- **All sends are blocking sends on unbuffered channels performed while `mu`
  is held**, with one exception: the immediate result of
  `ConfirmationNotificationsClient.Watch` is sent holding neither `mu` nor the
  client mutex (`wallet/notifications.go:889-892`). A subscriber that stops
  receiving blocks the sender, and with it
  every other notification and every registration or deregistration.
- `notifyUnminedTransaction`, `notifyMinedTransaction`, `notifyAttachedBlock`,
  `notifyDetachedBlock` and the pruning call of `notifyRemovedTransaction`
  run **inside an open database write transaction**, usually with
  `lockedOutpointMu` held as well (§5.3). A stalled transaction-notification
  subscriber therefore holds the wallet's single writer open.
- Lock order that occurs: `lockedOutpointMu` → database write transaction →
  `NotificationServer.mu` (unmined notification); and
  `NotificationServer.mu` → database read transaction (confirmation
  processing inside `notifyMainChainTipChanged`). This is deadlock-free only
  because bbolt read transactions do not wait for the open write transaction.
- Ordering guarantee: each client receives notifications of its kind in the
  order the wallet produced them. There is no ordering guarantee between
  kinds or between clients, except the fixed sequence at the end of
  `ChainSwitch` (§5.3).
- The same notification pointer is sent to every client of a kind; receivers
  must treat it as read-only.
- Deregistration (`Done`, four channel-based kinds,
  `wallet/notifications.go:502-523` and equivalents): spawns goroutine A that
  receives and discards until the channel is closed, and goroutine B that
  locks `mu`, finds the channel, replaces it with the last slice element,
  truncates the slice, closes the channel and unlocks. `Done` returns
  immediately. Swap-removal reorders the remaining clients.
- There is no server shutdown. Channels are closed only by `Done`.
- Confirmation clients (`wallet/notifications.go:754-785`): registered under
  `mu`; a goroutine waits for the client context and swap-removes the client.
  On each tip change one goroutine per client runs `process`, and
  `notifyMainChainTipChanged` waits for all of them while holding `mu`. Each
  `process` blocks on the unbuffered result channel until `Recv` takes the
  result or the client context ends. `Watch` blocks the same way and is called
  synchronously by the gRPC request reader, so a client must keep reading to
  be able to submit further requests.
- `ConfirmationNotificationsClient.mu` protects the `watched` map. `process`
  holds it across its database view. `Watch` reads `len(c.watched)` without
  the lock (capacity hint only) and takes it only to insert.

Complete list of goroutine launches in scope (there are no others in the
six files):

| Site | Count | Lifetime |
|---|---|---|
| `TransactionNotificationsClient.Done` (`wallet/notifications.go:503,509`) | 2 per call | drainer ends when the channel is closed; remover ends after one pass under `mu` |
| `RemovedTransactionNotificationsClient.Done` (`554,558`) | 2 per call | same |
| `AccountNotificationsClient.Done` (`653,657`) | 2 per call | same |
| `MainTipChangedNotificationsClient.Done` (`710,714`) | 2 per call | same |
| `notifyMainChainTipChanged` (`741`) | 1 per confirmation client per tip change | joined with a `sync.WaitGroup` before `mu` is released |
| `ConfirmationNotifications` (`768`) | 1 per client | blocks on `ctx.Done()`; never ends if the context is never cancelled |

A second `Done` on the same client is harmless in the code as written: the
remover finds no matching channel and closes nothing (no double-close panic),
and the drainer ends at once because the channel is already closed. The
"exactly once" rule in the doc comment is still the contract.

How the in-tree consumers use the clients (fixes what "slow subscriber"
means in practice): the gRPC `TransactionNotifications` and
`AccountNotifications` handlers register on stream start, `defer n.Done()`,
and loop on `select { <-n.C → svr.Send; <-ctx.Done() → return }`
(`internal/rpc/rpcserver/server.go:2388-2442`). The wallet is therefore
blocked for the duration of each `svr.Send` of the previous notification
(HTTP/2 flow control included) but not after the stream ends, because `Done`
starts the drainer. A `Send` error ends the stream with the translated error.
The ticket buyer consumes `MainTipChangedNotifications` the same way
(`ticketbuyer/tb.go:71`).

### 7.2 Wallet methods in scope

| Method | Locks / transactions, in order |
|---|---|
| `TotalReceivedForAccounts`, `TotalReceivedForAddr`, `DetermineRelevantTxs`, `NeedsAccountsSync`, `OutputInfo`, `UnstableAPI` methods, `getCoinjoinTxsSumbByAcct`, VSP queries | one View |
| `SignTransaction` | one View for all inputs; private keys obtained through the address manager stay alive until return, then are zeroed |
| `CreateSignature` | one View; signing happens after the View closes; key zeroed on return |
| `SelectInputs` | `lockedOutpointMu` → one View |
| `AbandonTransaction` | one Update; then `NotificationServer.mu` |
| `PublishTransaction` | View; `lockedOutpointMu` → Update (→ `NotificationServer.mu` inside) → unlock; network publish; on failure `AbandonTransaction`; network filter load |
| `SendOutputs*` | `authorTx` (own locking) → `recordAuthoredTx` (`lockedOutpointMu` → Update) → publish |
| `CreateVspPayment` | `ReserveOutputsForAmount` (`lockedOutpointMu` → View) → `NewChangeAddress` → `SignTransaction` (View) → `SetPublished` (Update) → `AddTransaction` (`lockedOutpointMu` → Update) → deferred `UnlockOutpoint` per reserved input |
| `ForUnspentUnexpiredTickets` | View for the tip; one View for `GetTickets`, within which the callback runs; `UnprocessedTickets`/`ProcessedTickets` callbacks open nested Views (read-in-read; see Quirk Q31) |
| `LiveTicketHashes` | one View (ticket iterator closed by `defer` before the View ends); the `ExistsLiveTickets` RPC is made after the View has closed, so the tip may have moved between the two |
| `ValidatePreDCP0005CFilters` | whatever `RangeCFiltersV2` uses (sibling spec); hashing happens inside its callback |
| `ImportCFiltersV2`, `SetPublished` | one Update each; no wallet mutex |
| `GetVSPTicketsByFeeStatus` | one View; the map is flattened after the View closes |
| `Create`, `CreateWatchOnly` | no wallet object exists; database transactions are those of `udb.Initialize` / `udb.InitializeWatchOnly` |
| `Open` | no wallet locks; sequential Views/Updates as listed in §5.16 |
| `Run` | blocks until the mix client returns |

`AllowsHighFees`, `ChainParams`, `GapLimit`, `ManualTickets`, `MixingEnabled`
read fields that are written once in `Open` and never mutated; no locking.

### 7.3 Lifecycle

- `Open` does not start goroutines. `Run` is the only long-running entry
  point in scope and is driven by the network syncer; the syncer cancels the
  wallet context after its own context ends (`chain/sync.go:726-734`,
  `spv/sync.go:447-459`).
- `w.mixClient` is an atomic pointer replaced on every `Run`.
- `w.initialHeight` is fixed at `Open`.

---

## 8. Errors

Errors use the wallet `errors` package (`errors/errors.go`): `E(args...)`
builds an `*Error{Op, Kind, Err}`; a nested `*Error` that adds nothing is
collapsed; `Error()` renders `op: kind: message`, separating nested wallet
errors with `errors.Separator` (package default `":\n\t"`, but the daemon sets
it to `":: "` at init, `dcrwallet.go:38`, so RPC clients and logs of the
running daemon see `":: "`); an op or kind equal to the one last printed is
not repeated; kind `Other` is not printed. Kind strings
(`errors/errors.go:77-124`) are observable in RPC error messages.

| Function | Error | Kind | Message (after op prefix) |
|---|---|---|---|
| `SendOutputs*` | output check | `Invalid` | `transaction output amount is negative` / `transaction output amount exceeds maximum value` |
| `SendOutputs*` | output check | `Policy` | `transaction output is dust` |
| `SendOutputs*` | no backend | `NoPeers` | kind string `Decred network is unreachable`, then the separator and the inner op `wallet.NetworkBackend` (no message) |
| `SignTransaction` | previous tx unknown | none | `<hash>:<index> not found` |
| `SignTransaction` | per input | as produced by `sign.SignTxOutput`, the address manager (`Locked`, `NotExist`, `WatchingOnly`, …) or the script engine | wrapped with op `wallet.SignTransaction` |
| `SignTransaction` key source | additional keys given but none for address | none | `no key for address (needed: <addr>, have map[...])` |
| `SignTransaction` script source | no redeem script supplied | none | `no script for address` |
| `CreateVspPayment` | reservation | wraps inner | `unable to reserve outputs: <inner>` |
| `CreateVspPayment` | value | none | `not enough input value to pay fee: <in> DCR < <fee> DCR` |
| `CreateVspPayment` | address type | none | `failed to convert '<Go type>' to wallet.Address` |
| `CreateVspPayment` | signing | none | `"\t{<index> <error>} "` for the last `SignatureError` (tab prefix, trailing space) |
| `AbandonTransaction` | unknown hash | `NotExist` | op only: `wallet.AbandonTransaction(<hash>): item does not exist` |
| `AbandonTransaction` | mined | `Invalid` | `transaction <hash> is mined in main chain` |
| `PublishTransaction` | any | from backend/store | op `wallet.PublishTransaction(<hash>)` |
| `ValidatePreDCP0005CFilters` | wrong network | none | `The current network does not have pre-DCP0005 cfilters` |
| `ValidatePreDCP0005CFilters` | mismatch | none | `hash of provided cfilters <got> does not match expected hash <want>` |
| `ImportCFiltersV2` | missing block | `NotExist` | `block height of <n> unknown` |
| `Create` | seed length | none | `seed length must be between 128 and 512 bits` |
| `Open` | see §5.16 step 7 | `NotExist` / `Invalid` / passphrase | |
| `SelectInputs` | account beyond last | `NotExist` | `account not found` |
| `OutputInfo` | index | none | `transaction has no output <n>` |
| `VSPFeeHashForTicket`, `VSPHostForTicket` | record missing | `NotExist` | `no VSP info for ticket <hash>` |
| `VSPFeeHashForTicket`, `VSPHostForTicket` | zero hash / empty host | `NotExist` | no message (`item does not exist`) |
| `ForUnspentUnexpiredTickets` | multiple | joined | messages separated by `\n` |
| `ConfirmationNotificationsClient.Recv` | context done | — | `context.Canceled` (always this value, even for a deadline) |
| `ConfirmationNotificationsClient.Recv` | database error in `Watch`/`process` | from store | the View error, unwrapped (no op), with a nil result list |
| `TotalReceivedForAccounts`, `TotalReceivedForAddr` | any store/manager error, or `ctx.Err()` from the range | from store | op `wallet.TotalReceivedForAccounts` / `wallet.TotalReceivedForAddr` |
| `CreateSignature` | key lookup | `Locked`, `NotExist`, `WatchingOnly`, … | op `wallet.CreateSignature` |
| `CreateSignature` | `sign.RawTxInSignature` failure: `prevPkScript` does not parse, or `SigHashSingle` with `idx >= len(tx.TxOut)` (`attempt to sign single input at index <idx> >= <n> outputs`, `dcrd/txscript@v4.1.2/sighash.go:236-240`) | none (dcrd script error) | op `wallet.CreateSignature`, dcrd message. `idx >= len(tx.TxIn)` is **not** an error in dcrd: with `SigHashAnyOneCanPay` the slice expression at `sighash.go:251` panics; without it no bounds check exists on the path (`sighash.go:249`) and a signature is returned (UNVERIFIED by execution) |
| `CreateWatchOnly` | any | from `udb.InitializeWatchOnly` | op `wallet.CreateWatchOnly` |
| `Create` | seed generation or `udb.Initialize` | from callee | op `wallet.Create` |
| `ImportCFiltersV2` | filter bytes do not parse | from `gcs.FromBytesV2` | op `wallet.ImportCFiltersV2` |
| `LiveTicketHashes` | iterator error, RPC error | from callee | op `wallet.LiveTicketHashes`; result is nil on error |
| `GetVSPTicketsByFeeStatus` | bucket iteration | from store | op `wallet.GetVSPTicketsByFeeStatus` |
| `GetCoinjoinTxsSumbByAcct` | any | from store | two nested ops: outer `wallet.GetCoinjoinTxsSumbByAcct`, then the separator, then inner `wallet.getCoinjoinTxsSumbByAcct` (the ops differ, so `errors.E` does not collapse them, `errors/errors.go:226-237`) |
| `IsVSPTicketConfirmed` | record missing | `NotExist` | `no VSP info for ticket <hash>`, no op |
| `unstableAPI.TxDetails` / `RangeTransactions` / `UnspentMultisigCreditsForAddress` | any | from store (`NotExist` for an unknown hash in `TxDetails`) | ops `wallet.TxDetails`, `wallet.RangeTransactions`, `wallet.UnspentMultisigCreditsForAddress` |
| `SelectInputs`, `OutputInfo` | store errors | from store | ops `wallet.SelectInputs`, `wallet.OutputInfo` |
| `PublishUnminedTransactions` | listing or backend | from callee | op `wallet.PublishUnminedTransactions` |
| `NeedsAccountsSync` | manager | from manager | no op; the boolean returned with the error is `true` |
| `SetPublished` | store | from store | no op |

A bare string passed to `errors.E` (as in `ValidatePreDCP0005CFilters` and
`SelectInputs`) becomes the wrapped error through `errors.New`
(`errors/errors.go:200-201`); it carries no kind of its own.

Sentinel used only for control flow: `errors.New("break")` in
`NeedsAccountsSync` (`wallet/wallet.go:5212`) stops the address iteration and
is never returned.

Methods that return store errors **without** an op wrapper: `SetPublished`,
`VSPFeeHashForTicket`, `VSPHostForTicket`, `IsVSPTicketConfirmed`,
`DetermineRelevantTxs`, `NeedsAccountsSync`, `CreateVspPayment`,
`UnprocessedTickets`, `ProcessedTickets`.

Notification construction failures never reach subscribers; they are logged at
ERROR in subsystem `WLLT` with the strings given in §5.22–§5.24 and the
notification is dropped.

Panics reachable in scope: duplicate deployment id in `Open`
(`wallet/wallet.go:5380`); out-of-range previous output index in
`SignTransaction` (`wallet/wallet.go:4809`) and `lookupInputAccount`
(`wallet/notifications.go:75`); nil pending notification in
`sendAttachedBlockNotification` (Quirk Q5).

---

## 9. Compatibility-critical behaviours

1. **Maturity arithmetic.** `ticketMatured` and `ticketExpired` use
   `curHeight - txHeight > N` (one block later than `ticketChangeMatured`'s
   `curHeight - txHeight + 1 > N` form). Unmined (`-1`) is never matured or
   expired. These feed balances, ticket status, `gettickets` and UTXO
   selection; `wallet/udb` carries duplicate copies of `coinbaseMatured`,
   `ticketChangeMatured` and `ticketMatured` (not `ticketExpired`) that must
   agree (`wallet/udb/txmined.go:2949-2967`).
2. **`TransactionSummary` construction** (§5.22) is what gRPC
   `TransactionNotifications`, `GetTransaction`, `GetTransactions`,
   `GetTickets` clients (Decrediton) see: fee only when all inputs are wallet
   debits; `MyOutputs` entries with zero amount/empty address on lookup
   failure; timestamp = min(receive time, block time) for mined transactions;
   type classification including treasury transactions as coinbase.
3. **gRPC `TransactionNotifications` stream content and order**: one message
   per relevant unmined transaction carrying exactly one
   `unmined_transactions` entry plus the full list of unmined hashes in
   ascending raw-byte order; one message per chain switch carrying attached
   blocks ascending, detached blocks descending (both as hashes and as header
   details), no `unmined_transactions`, and the unmined hash list. A chain
   switch produces a message even when no attached block has wallet
   transactions.
4. **Unmined notifications precede commit and publish.** Subscribers see a
   newly authored or published transaction before it is broadcast; if the
   broadcast fails the transaction disappears without a dedicated message and
   is simply absent from the next `unmined_transaction_hashes`.
5. **gRPC `AccountNotifications`** fire only on rename, new account, WIF
   import and public-key import, with key counts computed as
   `min(2^31, lastUsed + gapLimit)` in wrapping `uint32` arithmetic for
   BIP0044 accounts and zero for others (§5.25).
6. **gRPC `ConfirmationNotifications`** semantics (§5.27, §5.28): an immediate
   response to every request with a non-empty hash list; one response per
   main-tip change for every stream, even with nothing watched; `-1`
   confirmations for unknown hashes and for transactions recorded in a
   stake-invalidated block; `block_hash` empty and `block_height = -1` unless
   confirmations > 0; a hash stops being watched once
   `confirmations >= stop_after` or `-1`; unmined transactions report 0.
7. **`MainTipChangedNotification`** drives the ticket buyer: attached and
   detached hashes ascending by height, sent after the database commit and
   before the transaction notification.
8. **`SignTransaction` results** surface verbatim in JSON-RPC
   `signrawtransaction` (`errors[].error`, `complete`) and gRPC
   `SignTransaction(s)`. The per-input error strings, the rule for
   suppressing "not enough multisig signatures", the skip of a vote's
   stakebase input and the use of caller-supplied keys to the exclusion of
   wallet keys must match. Signature bytes must be deterministic RFC 6979
   ECDSA with compressed pubkeys.
9. **`CreateSignature` output**: DER signature with the hash-type byte
   appended, and the 33-byte compressed public key (JSON-RPC `createsignature`,
   gRPC `CreateSignature`).
10. **Relevance rule** (§5.7) decides whether `sendrawtransaction` /
    `PublishTransaction` records a transaction in `wallet.db`, and which
    transactions the SPV syncer treats as wallet-backed when answering
    `getdata`.
11. **Watched outpoints** (§5.8, and those returned by
    `processTransactionRecord`) are sent to dcrd with `loadtxfilter` or used
    for SPV filter matching; the tree byte of each outpoint is part of that
    message.
12. **`PublishTransaction` side effects on `wallet.db`**: a relevant
    transaction is inserted as unmined before broadcast and fully removed
    (with unmined descendants) when the broadcast fails.
13. **`AbandonTransaction`** errors: unknown hash is `NotExist`, mined is
    `Invalid` with the message in §8; JSON-RPC `abandontransaction` and gRPC
    `AbandonTransaction` pass them through their error translation.
14. **`PublishUnminedTransactions`** sends transactions in dependency order and
    omits those flagged unpublished (VSP fee transactions not yet released).
15. **Unpublished flag ordering in `CreateVspPayment`**: the `up` bucket entry
    is written before the fee transaction is inserted, so the fee transaction's
    outputs never count as spendable or in notification balances while
    unpublished, and it is never rebroadcast early.
16. **VSP fee transaction shape**: single fee output plus optional P2PKH change
    (random position), fee computed from the wallet relay fee over the
    108-byte-per-input estimate; change dropped when dust.
17. **`ValidatePreDCP0005CFilters`**: BLAKE-256 over the concatenated stored
    filter bytes for heights 0…activation−1 must equal the hard-coded hash;
    result surfaces as JSON-RPC `validatepredcp0005cf` (`true`, or an error).
18. **`ImportCFiltersV2`** writes `merkleRoot[0:16] || data` under each block
    hash, all-or-nothing, and fails with `NotExist` when a header is missing.
19. **`TotalReceivedForAccounts` row order** is the bbolt order of
    little-endian account keys, and JSON-RPC `getreceivedbyaccount` indexes
    the result by account number (`internal/rpc/jsonrpc/methods.go:2415-2419`).
    `listreceivedbyaccount` output order follows the same rows.
20. **`minConf` handling in received totals**, including the wrap to "all
    transactions plus unmined" when `minConf > tipHeight + 1`.
21. **`LiveTicketHashes` order and membership** (JSON-RPC `gettickets`):
    immature first then live, each in ticket-hash byte order; SPV returns only
    immature/unmined tickets and only when `includeImmature`.
22. **`ForUnspentUnexpiredTickets` window and status filter** determine which
    tickets the VSP client processes at startup (`dcrwallet.go:424`) and on
    gRPC `ProcessManagedTickets` / `ProcessUnmanagedTickets`
    (`internal/rpc/rpcserver/server.go:4137,4167`), and which tickets receive
    updated vote choices at their VSP (gRPC `SetVspdVoteChoices`,
    `server.go:4225`; JSON-RPC vote-choice updates,
    `internal/rpc/jsonrpc/methods.go:4836`).
23. **`Open` sequence against an existing `wallet.db`**: migration, upgrade,
    version checks and messages, testnet3 checkpoint rollback at 962928,
    address-buffer cursors reconstructed from persisted last-used and
    last-returned indexes, default vote bits and treasury policies loaded.
24. **Log lines**: `Opened wallet` (INFO) and the ERROR/WARN strings quoted in
    §5 appear in the daemon log under `WLLT`.
25. **`TicketStatus.String()`** values (§3.6) and **`TransactionType`**
    numeric values, where they are exposed through RPC marshalling.
26. **`TxTransactionType` on foreign transactions.** gRPC
    `DecodeRawTransaction` classifies any caller-supplied transaction with it
    (`internal/rpc/rpcserver/server.go:3715`), so the first-match order
    (either-coinbase, ticket, vote, revocation, regular) and the
    treasury-as-coinbase result (Q14) are visible without any wallet state.
27. **`stop_after` handling.** `stop_after = 0` yields exactly one response
    per request and never registers a watch; negative values are rejected by
    the gRPC layer, not by the wallet (§5.27).
28. **Confirmation `block_hash`** is the main-chain block hash at the
    transaction's recorded height (§5.27).
29. **Testnet3 checkpoint.** A wallet whose stored block 962928 differs from
    `0000004fd1b267fd39111d456ff557137824538e6f6776168600e56002e23b93` is
    rolled back to that height at every `Open` (§5.16 step 9); the rollback is
    a `wallet.db` write an old and a new binary must agree on.
30. **Address buffers at `Open`** exist for BIP0044 accounts `0..=lastAcct`
    and imported-xpub accounts only, never for the imported account
    `0x7FFFFFFF`; a failure to derive branch 0 or 1 of any account makes the
    wallet unopenable (§5.16 step 10).
31. **Empty versus nil Go results** that RPC layers may distinguish:
    `GetVSPTicketsByFeeStatus` returns a non-nil empty slice when nothing
    matches; `UnprocessedTickets`/`ProcessedTickets` return non-nil empty
    slices; `TotalReceivedForAccounts`, `LiveTicketHashes` and
    `DetermineRelevantTxs` return nil slices when empty;
    `GetCoinjoinTxsSumbByAcct` returns a non-nil empty map.

---

## 10. Quirks

Each item is a behaviour a faithful port must either reproduce or diverge
from deliberately.

**Q1. `minConf` larger than the chain counts everything.**
`stopHeight = tipHeight - minConf + 1` goes negative when
`minConf > tipHeight + 1`; the store treats any negative end height as "to the
tip, then unmined" (`wallet/wallet.go:4488-4492`, `4540-4544`;
`wallet/udb/txquery.go:394-398`, `534-537`). `getreceivedbyaddress` with a
huge `minconf` returns the total including unconfirmed transactions instead of
zero.

**Q2. Account rows are ordered by little-endian key bytes.**
`TotalReceivedForAccounts` appends rows in bbolt order of the 4-byte
little-endian account number (`wallet/udb/addressdb.go:698-708`, `930-934`):
account 0 (`00 00 00 00`), imported-xpub account `0x80000000`
(`00 00 00 80`), account 256 (`00 01 00 00`), account 1 (`01 00 00 00`), …,
imported `0x7FFFFFFF` (`ff ff ff 7f`). JSON-RPC `getreceivedbyaccount` then
indexes the slice by account number, or uses the last row for the imported
account (`internal/rpc/jsonrpc/methods.go:2415-2419`). Once an account
numbered 256 or higher exists (257 or more BIP0044 accounts), or any
imported-xpub account exists, the wrong row is returned for the accounts whose
row position no longer equals their number.

**Q3. `ForUnspentUnexpiredTickets` on a short chain only looks at the tip
block.** When `tipHeight < TicketExpiry + TicketMaturity - 8` (41208 mainnet,
6152 testnet3, 392 simnet/regnet) the start height is negative
(`wallet/wallet.go:5734-5738`). `RangeTransactions` then visits the unmined
set first and iterates blocks **backwards from the maximum height down to the
end height**, which is the tip, so only the tip block is visited
(`wallet/udb/txquery.go:394-421`, `524-538`). Unmined tickets are visited but
filtered out by status. On simnet or a young testnet, `UnprocessedTickets`,
`ProcessedTickets` and VSP vote-choice updates therefore see almost nothing.

**Q4. Stale accumulation state.** `currentTxNtfn` is created by
`notifyDetachedBlock`/`notifyAttachedBlock`/`notifyMinedTransaction` inside the
`ChainSwitch` database transaction and cleared only by
`sendAttachedBlockNotification`. If the transaction fails (for example
"failed reorganize: sidechain … has less total work", raised after the
detached blocks were recorded, `wallet/chainntfns.go:146-154`), the detached
and attached entries stay and are delivered with the next successful switch.
The same happens when the database transaction commits but the following
`watchHDAddrs` call fails: `ChainSwitch` returns the error at
`wallet/chainntfns.go:293-297` before reaching the two notification calls at
lines 312-313, so neither the tip-changed nor the transaction notification is
sent for a chain switch that is already committed.
Likewise `notifyMinedTransaction` called from rescan or `AddTransaction`
leaves an empty non-nil `currentTxNtfn` behind
(`wallet/notifications.go:277-283`).

**Q5. Latent nil dereference.** `sendAttachedBlockNotification` reads
`currentTxNtfn.AttachedBlocks` without a nil check
(`wallet/notifications.go:314-339`). `ChainSwitch` returns early from its
database transaction when the wallet tip already equals the chain tip
(`wallet/chainntfns.go:118-120`) without touching `currentTxNtfn`; if a
transaction client is registered and no stale state exists, the process
panics. `EvaluateBestChain` returns a chain only when its net work relative to
the current tip is strictly positive (`wallet/sidechains.go:417-458`), so a
chain taken from it cannot hit the early return as long as the wallet tip does
not move between the evaluation and the switch. Three callers obtain the
chain this way: `chain/sync.go:891` (evaluated at 885), `chain/sync.go:325`
(evaluated at 310, with `sidechainsMu` released in between) and
`spv/sync.go:1542` (evaluated at 1488, with a network cfilter fetch in
between). The SPV initial-sync pipeline is different: `spv/sync.go:1811` passes
`batch.bestChain`, which was evaluated ahead of time in an earlier pipeline
stage, and trims already-attached leading blocks only when
`0 < tipIndex < len(bestChain)-1` (`spv/sync.go:1796-1808`). A batch whose
last block is already the wallet tip is passed untrimmed and reaches the early
return. UNVERIFIED whether the pipeline can actually produce such a batch.

**Q6. Private keys in an error string.** When `additionalKeysByAddress` is
non-empty and lacks a needed address, the error is formatted with `%v` of the
whole `map[string]*dcrutil.WIF` (`wallet/wallet.go:4820-4822`). `*WIF`
implements `String()` (`dcrd/dcrutil@v4.0.3/wif.go:167`), so the message
contains every supplied WIF private key, keyed by address, in Go's sorted-map
format `map[addr1:Wif1 addr2:Wif2]`. The string is returned to the JSON-RPC
caller in `signrawtransaction` `errors[].error`.

**Q7. Asymmetries in `SignTransaction` validation.** The
"multisig, not enough signatures" suppression inspects only
`additionalPrevScripts`, not the script fetched from the wallet
(`wallet/wallet.go:4880`); `errEval` keeps only the last non-underflow engine
error and that error is reported both in the list and as the fatal return
(`wallet/wallet.go:4889-4906`); the compressed flag passed to the signer is
always true, whatever the address (`wallet/wallet.go:4824,4832`).

**Q8. `AbandonTransaction` documentation versus code.** The comment says it
does not error when the transaction does not exist unmined
(`wallet/wallet.go:5062-5066`); the code returns `NotExist` for an unknown
hash (`wallet/wallet.go:5077-5080`). Only the requested hash is notified as
removed, not the descendants deleted with it. Locked input outpoints are not
released.

**Q9. `SendOutputs` without a network backend leaves the transaction in the
wallet.** The transaction is recorded (and notified) before
`publishAndWatch` resolves the backend; a missing backend returns `NoPeers`
without abandoning it (`wallet/createtx.go:334-340`). It will be broadcast by
the next `PublishUnminedTransactions`.

**Q10. `SelectInputs` ignores the locked set and rejects imported-xpub
accounts.** It holds `lockedOutpointMu` but passes a nil ignore function
(`wallet/utxos.go:25-43`); any account above `LastAccount` other than
`0x7FFFFFFF` is "account not found" (`wallet/utxos.go:32-39`).

**Q11. Account notification key counts.** `lastUsed + gapLimit` wraps for
unused branches (`0xFFFFFFFF + 20 = 19`) and is one short of the number of
watched addresses for used ones (`wallet/notifications.go:618-623`). The type
comment claims notifications fire when any property changes; only four call
sites exist (§5.3).

**Q12. `CreateVspPayment` oddities.** A negative remainder (inputs cover the
fee but not the transaction fee) is treated as dust and silently dropped,
producing a transaction that pays less than the relay fee
(`wallet/wallet.go:4727-4734`); only the last signature error is kept in the
returned message, which has a leading tab and trailing space
(`wallet/wallet.go:4741-4749`); the log line prints `Failed to sign
transaction: <nil>` when only per-input errors occurred; a change address is
consumed even when no change output is added; the unpublished flag is written
in a separate database transaction before the transaction exists, so a crash
in between leaves an orphan `up` entry.

**Q13. `NewBalances` is dead weight.** It is computed for every transaction
notification (a full scan of the unspent set) and returned in Go map order
(`wallet/notifications.go:206-212`), but the only consumer drops it
(`internal/rpc/rpcserver/server.go:2398-2404`). Its only observable effect is
that a failure while computing it suppresses the whole notification.

**Q14. Treasury transactions are "coinbase".** `TxTransactionType` and
`OutputInfo.FromCoinbase` use `compat.IsEitherCoinBaseTx`, which accepts any
single-input transaction with a null previous outpoint, so treasury-base and
treasury-spend transactions classify as coinbase; treasury adds are regular
(`wallet/notifications.go:430-442`,
`dcrd/blockchain/standalone@v2.3.0/tx.go:49-102`). The Go enum numbers differ
from the protobuf enum numbers (§4.2).

**Q15. Silent zero values in summaries.** `lookupOutputChain` returns account
0, amount 0, nil address and nil script when the address lookup fails or the
script has no address, and `lookupInputAccount` returns account 0 on every
failure (`wallet/notifications.go:59-112`). Account 0 is then also added to
the "relevant accounts" set.

**Q16. Confirmation notification details.** (a) A result is emitted on every
tip change even with an empty watch set (`wallet/notifications.go:921-987`).
(b) The invalidation test applies to any transaction recorded in a block whose
stake-invalid flag is set, including stake-tree transactions that remain
valid; it is skipped for height 0 and for the tip block
(`wallet/notifications.go:842-852`). (c) `Watch` reorders its results by
swap-removal and mutates the caller's slice. (d) After a database error
`Watch` still adds the remaining hashes to the watch set, and `process` still
unwatches those scheduled before the failure. (e) Map iteration order makes
the per-block result order random. (f) `Recv` returns `context.Canceled`
regardless of why the context ended.

**Q17. Notifications from inside uncommitted transactions.** Unmined
notifications and pruning removals are sent before the enclosing database
transaction commits; if it then fails, subscribers have been told about state
that never existed. `RemovedTransactionNotifications` has no in-tree
subscriber, so removals are never observable outside the process.

**Q18. `NeedsAccountsSync` documentation versus code.** The comment says the
genesis block must be the tip; the code checks only accounts and addresses
(`wallet/wallet.go:5191-5223`).

**Q19. Relevance by last push.** `isRelevantTx` hashes the final data push of
each signature script and looks it up as an address id
(`wallet/wallet.go:4958-4962`). For P2PKH spends that push is the public key,
so inputs spending from wallet addresses match even when the output is
unknown. Outputs paying directly to a wallet public key (P2PK) are not
relevant because pubkey addresses do not implement `Hash160er`
(`wallet/wallet.go:4970-4976`), although `processTransactionRecord` would
credit them.

**Q20. `UnstableAPI`.** An exported function returning an unexported type;
its ops are `wallet.TxDetails`, `wallet.RangeTransactions`,
`wallet.UnspentMultisigCreditsForAddress` (no "unstable" in the name).

**Q21. Partial pre-DCP0005 validation.** On a wallet not yet synced to the
activation height, `ValidatePreDCP0005CFilters` hashes whatever filters exist
and reports a hash mismatch rather than "not synced". On simnet/regnet it
fails with a capitalised message.

**Q22. `ImportCFiltersV2` seek.** The block iterator seeks to
`startBlockHeight` and takes whatever key comes next; block records are
contiguous in practice, but nothing checks that the first block is at the
requested height. Filter contents are not validated against header
commitments.

**Q23. `Open`.** A duplicate deployment id panics instead of returning an
error. `Config.MixSplitLimit == 0` (the loader's `CreateNewWallet` path omits
the field, `internal/loader/loader.go:259-274`) creates zero-capacity
semaphores, so every mix request in that process is throttled
(`wallet/mixing.go:388-395`). `MainChainTip` errors are ignored when
recording `initialHeight`.

**Q24. Map-ordered results.** `GetVSPTicketsByFeeStatus` returns hashes in Go
map order (`wallet/wallet.go:5625-5630`); `GetCoinjoinTxsSumbByAcct` returns a
map that gRPC flattens in map order
(`internal/rpc/rpcserver/server.go:3855-3861`). The misspellings `Sumb` and
`Outputsp` are part of the Go and gRPC names.

**Q25. Dead branch.** `GetTickets` without an RPC client never yields
`TicketStatusLive`, so that case in `ForUnspentUnexpiredTickets` is
unreachable (`wallet/wallet.go:2899-2901`, `5716`). Unmined tickets are
excluded by the status filter.

**Q26. `LiveTicketHashes` in SPV mode** returns no matured tickets at all, and
"immature" includes unmined tickets (`wallet/tickets.go:44-73`).

**Q27. Re-publishing a known transaction.** `PublishTransaction` on a
transaction already stored unmined runs `processTransactionRecord` again
(`InsertMemPoolTx` returns nil for an existing unmined record,
`wallet/udb/txunmined.go:31-34`) and fires a second unmined notification. If
the transaction is already mined it logs a warning, still broadcasts, and on
broadcast failure the abandon attempt fails with `Invalid` (logged at WARN).

**Q28. `Done` must be called exactly once**
(`wallet/notifications.go:499-501`). Never calling it leaves a channel that
blocks the wallet on the next notification. Removal reorders the client slice.

**Q29. Go-runtime dependence.** Per-iteration loop variables (Go ≥ 1.22) are
required for `&txHash` in `process` and for the goroutines in
`notifyMainChainTipChanged` to be correct; `go.mod` declares 1.26. Receive
times come from `time.Now()` at record creation and are persisted with
one-second resolution.

**Q30. Truncated doc comment.** `SendOutputsToTreasury`'s comment is the
single line "transaction hash upon success" (`wallet/wallet.go:4608`).

**Q31. Nested read transactions.** `UnprocessedTickets`, `ProcessedTickets`
and the RPC callbacks passed to `ForUnspentUnexpiredTickets` open a new View
(`VSPFeeHashForTicket`, `IsVSPTicketConfirmed`, `VSPHostForTicket`) while the
`GetTickets` View is still open on the same goroutine
(`wallet/wallet.go:5741`, `5756-5762`, `5799`). With bbolt every read
transaction holds the mmap read lock for its lifetime and a writer that must
grow the mapping takes the write lock (`bbolt@v1.3.12/db.go:433`, `735`); a
writer queued between the outer and inner read can therefore deadlock all
three. Upstream lives with this; a port should collect the hashes first and
query afterwards, which yields the same results.

**Q32. Upstream TODO notes in scope** (none changes behaviour; each marks a
place where upstream considers the design provisional):
`wallet/notifications.go:26-28` (errors during notification creation are only
logged, not sent to the RPC client); `30-33` (account lookups in notifications
are slow because the database is not organised by account);
`63-64` (debits do not record their account, hence `lookupInputAccount`);
`374-375` (`TransactionNotifications` is misnamed since it fires for
block-only changes); `wallet/wallet.go:5432` (`Opened wallet` might log
balance or sync height). The struct comments at `wallet/notifications.go:42-43`
("Coalesce transaction notifications since wallet previously did not add
mined txs together. Now it does and this can be rewritten.") and `:50`
("smells like hacks", on the back-pointer to the wallet) are in the same
category. There are no `XXX`, `BUG`, `FIXME` or `Deprecated` markers in
scope.

**Q33. Channel direction differs between client types.**
`TransactionNotificationsClient.C` is receive-only
(`wallet/notifications.go:478`); the `C` fields of the removed-transaction,
account and tip-changed clients are bidirectional
(`wallet/notifications.go:532,631,687`), so Go callers could send on or close
them. No in-tree caller does.

**Q34. `TicketStatus.String` dead check and conversion.** The generated code
tests `i < 0` on an unsigned type (never true) and formats out-of-range values
through `int64(i)` (`wallet/ticketstatus_string.go:26-31`), so a value above
`2^63 - 1` would print as a negative number. No code path produces a value
above 8.

**Q35. `Recv` races its two cases.** When the client context is already done
and a result is also ready, Go's `select` picks either
(`wallet/notifications.go:903-910`); the last result may or may not be
delivered. `process` likewise returns without evaluating when the context is
done at entry; a context that ends during its View yields a result that is
dropped at the send unless a receiver happens to be waiting.

**Q36. `GetVSPTicketsByFeeStatus` argument type.** The status is an `int`
compared with `int(uint32)` from the record (`wallet/udb/vsp.go:134`); a
negative or out-of-range argument matches nothing and is not an error. Values
shorter than 40 bytes in the `vsp` bucket make the deserialiser panic
(`wallet/udb/vsp.go:149-160`); only this wallet writes that bucket.

**Q37. `LiveTicketHashes` consistency window.** The tip height used to
classify tickets is read in the View, the liveness query goes to dcrd
afterwards (`wallet/tickets.go:33-83`); a block arriving in between can make
the two disagree. A ticket that is exactly at the maturity or expiry boundary
uses the off-by-one predicates of §3.1, so a ticket mined at `h` is
"immature" through tip `h + TicketMaturity` inclusive and is dropped as
expired from tip `h + TicketMaturity + TicketExpiry + 1`.

**Q38. Misleading doc comments.** `ProcessedTickets` and `UnprocessedTickets`
say they return "the hash of every live/immature ticket"; they return
`*VSPTicket` objects (`wallet/wallet.go:5749-5751`, `5793-5795`).
`PublishTransaction` says "If the send fails, the transaction is not added to
the wallet"; it is added first and removed on failure, with an unmined
notification already sent (`wallet/wallet.go:5100-5102`, §5.10). `SetPublished`
says "sets the informed hash as true or false"; `published == false` *adds*
the unpublished marker. `MixingEnabled` says the wallet "is running the mixing
client"; it reports only the configuration flag. `Open`'s comment mentions
"namespaces configuration options", which no longer exist.

---

## 11. Upstream tests

| Test | File | Pins | Port? |
|---|---|---|---|
| `TestTicketMatured` | `wallet/wallet_test.go:48-73` | nine `(txHeight, tipHeight)` cases on mainnet params, including the off-by-one at exactly `TicketMaturity` and unmined (`-1`) | Port verbatim. |
| `TestTicketExpired` | `wallet/wallet_test.go:75-101` | nine cases at `TicketMaturity + TicketExpiry` boundaries | Port verbatim. |
| `TestCoinbaseMatured` | `wallet/wallet_test.go:21-46` | sibling predicate (line 4422, outside this range) | Port with the sibling spec. |
| `testWallet` helper | `wallet/setup_test.go:20-55` | exercises `Create` then `Open` on a fresh bdb file with simnet params, `GapLimit: 20`, `RelayFee: 1e5`, `MixingEnabled: true` | Port as the standard fixture. |
| `TestReorg`, `TestSetBirthStateAndScan` | `wallet/reorg_test.go:155`, `wallet/wallet_test.go:147` | drive `ChainSwitch`, which calls the notification accumulation paths with no subscribers | Port with the chain-switch spec; extend with subscribers. |
| `TestLocking` | `wallet/locking_test.go:15` | uses `Open`ed wallet | Indirect. |
| `TestAddresses`, `TestAccountIndexes` | `wallet/addresses_test.go:186,219` | use the `testWallet` fixture; the first row of `TestAccountIndexes` pins the address buffer that `Open` builds for a fresh wallet (§5.16 step 10): `lastUsed = 0xFFFFFFFF`, `cursor = 0` on both branches of account 0. Reconstruction from non-trivial persisted indexes is not tested | Port with the address spec; add a reopen case to pin cursor reconstruction. |
| `TestDiscoveryCursorPos` | `wallet/discovery_test.go:16` | calls `w.GapLimit()` (lines 46, 86) and the fixture | Indirect. |
| `mockNetwork` | `wallet/network_test.go:19-40` | a `NetworkBackend` whose `PublishTransactions` and `LoadTxFilter` are no-ops; never passed to `PublishTransaction` or `PublishUnminedTransactions` by any test | Reuse as the backend double for new publish tests. |

Test files in package `wallet` that do not touch this scope at all:
`wallet/main_test.go` (logger setup), `wallet/mixing_test.go` (`TestIsMixTx`;
exercises `PossibleCoinJoin`, which §5.18 depends on, but not the count
query), `wallet/sidechains_test.go`. There are no test files for
`wallet/notifications.go`, `wallet/utxos.go`, `wallet/unstable.go`,
`wallet/tickets.go` or `wallet/ticketstatus_string.go`, no test data files
for this scope, and no test in `internal/rpc` or the repository root that
calls any function listed in §3.

There is no `ticketChangeMatured` test and **no upstream test at all** for:
the notification server (every type), `makeTxSummary`, `TxTransactionType`,
`SignTransaction`, `CreateSignature`, `isRelevantTx`,
`appendRelevantOutpoints`, `AbandonTransaction`, `PublishTransaction`,
`PublishUnminedTransactions`, `TotalReceivedFor*`, `CreateVspPayment`,
`NeedsAccountsSync`, `ValidatePreDCP0005CFilters`, `ImportCFiltersV2`,
`SelectInputs`, `OutputInfo`, `LiveTicketHashes`, the VSP query helpers,
`ForUnspentUnexpiredTickets`, `TicketStatus.String`. The `validate` package
has no test file. dcrd's `txscript/v4/sign/sign_test.go` covers the signer.

Vectors to generate through a Go oracle shim (run the pinned dcrwallet against
a scripted simnet chain and record outputs):

1. `TransactionNotifications` sequences for: receive unmined, mine it, reorg
   it out and back, double-spend it, send from wallet, publish failure.
2. `makeTxSummary` for each transaction type (regular, coinbase, ticket,
   vote, revocation, treasury base, tspend, tadd), with partial and full
   debits and with a credit to an address-less script.
3. `ConfirmationNotifications` traces, including `stop_after` 0/1/N, unknown
   hashes, a stake-invalidated block and the swap-removal ordering.
4. `AccountNotification` values after rename, new account and imports, with
   used and unused branches.
5. `SignTransaction` over P2PKH, P2SH multisig (partial then complete, with
   and without `additionalPrevScripts`), stake outputs and a vote; byte-exact
   signature scripts and error strings.
6. `CreateSignature` byte vectors for each sighash type.
7. `TotalReceivedForAccounts` with accounts 0, 1, 256 and an imported xpub
   (row order), and the `minConf > tip + 1` case.
8. `appendRelevantOutpoints` and `isRelevantTx` truth tables for tickets with
   owned voting address only, owned commitment only, both, neither; P2PK
   outputs; P2PKH spends of unknown outputs.
9. `ValidatePreDCP0005CFilters` against a mainnet/testnet3 header-and-filter
   snapshot (large fixture; alternatively verify the expected hash from the
   published filter set offline).
10. `ForUnspentUnexpiredTickets` on chains shorter and longer than the window.

---

## 12. Rust porting notes

**Suggested module boundary** (inside the wallet crate; names illustrative):

| Module | Contents | Mirrors |
|---|---|---|
| `wallet::maturity` | the three predicates (plus `confirms`, `coinbase_matured` from the sibling spec); shared with the store crate so there is one copy | `wallet/wallet.go:4402-4447`, `wallet/udb/txmined.go:2949-2966` |
| `wallet::received` | received totals, coinjoin counts | `wallet/wallet.go:4449-4568`, `5554-5609` |
| `wallet::signing` | `sign_transaction`, `create_signature`, `SignatureError`, key/script sources | `wallet/wallet.go:4645-4948` |
| `wallet::relevance` | `is_relevant_tx`, `determine_relevant_txs`, `append_relevant_outpoints` | `wallet/wallet.go:4950-5060` |
| `wallet::publish` | `publish_transaction`, `publish_unmined_transactions`, `abandon_transaction`, `send_outputs*` | `wallet/wallet.go:4570-4643`, `5062-5183` |
| `wallet::cfilters` | pre-DCP0005 validation, import | `wallet/wallet.go:5225-5287`, `validate/validate.go:107-130` |
| `wallet::lifecycle` | `create`, `create_watch_only`, `open`, `run`, config getters | `wallet/wallet.go:5289-5552` |
| `wallet::vsp_queries` | VSP record getters, ticket iteration helpers, `create_vsp_payment` | `wallet/wallet.go:4662-4762`, `5611-5828` |
| `wallet::tickets` | `live_ticket_hashes`, `LiveTicketQuerier` trait, `TicketStatus` display | `wallet/tickets.go`, `wallet/ticketstatus_string.go` |
| `wallet::utxos`, `wallet::unstable` | thin wrappers | `wallet/utxos.go`, `wallet/unstable.go` |
| `wallet::notify` | the notification server, all payload types, `make_tx_summary`, `tx_transaction_type` | `wallet/notifications.go` |

**Reuse (each item below was opened and checked for the named function):**

- `dcroxide-txscript`: `sign.rs` has `sign_tx_output`, `raw_tx_in_signature`,
  `signature_script`, `KeyDb`/`ScriptDb` traits
  (`dcroxide:crates/dcroxide-txscript/src/sign.rs:62,78,119,184,606`);
  `stdscript.rs` has `extract_addrs`, `is_multi_sig_script_v0`,
  `multi_sig_redeem_script_from_script_sig_v0`
  (`dcroxide:crates/dcroxide-txscript/src/stdscript.rs:450,506,1157`);
  `engine.rs` supplies the script engine for the post-signing check. The
  documented panic of `sign_tx_output` on scripts that classify but yield no
  address (`sign.rs:599-604`) matches dcrd and must be guarded or accepted.
- `dcroxide-stake`: `is_sstx`, `is_ssgen`, `is_ssrtx`, `determine_tx_type`,
  `addr_from_sstx_pk_scr_commitment`, `treasury::is_tspend`
  (`dcroxide:crates/dcroxide-stake/src/lib.rs:392,808,1117,1209,1214`,
  `treasury.rs:290`).
- `dcroxide-standalone`: `is_coin_base_tx` for the either-coinbase test
  (`dcroxide:crates/dcroxide-standalone/src/tx.rs:40`), `big_to_compact`
  (`pow.rs:69`), `SubsidyCache` (`subsidy.rs:98`).
- `dcroxide-gcs`: `FilterV2::from_bytes` and `bytes`
  (`dcroxide:crates/dcroxide-gcs/src/lib.rs:497,536`).
- `dcroxide-crypto`: `blake256.rs` for the cfilter set hash.
- `dcroxide-mixing`: `mixpool.rs` supplies the pool created in `Open`. The
  crate has no mix **client** module (its `src/` lists `dcnet`, `field`,
  `mixpool`, `prng`, `sid`, `signatures`, `utxoproof` only); `Run` depends on
  the mix client that the mixing spec must deliver.
- `dcroxide-chaincfg`, `dcroxide-wire`, `dcroxide-chainhash`: parameters,
  `MsgTx`/`OutPoint`/`BlockHeader`, hashes.
- `dcr-rs`: `sign.rs` exposes only `sign_p2pkh_input`
  (`dcr-rs:src/sign.rs:38,52`), which cannot replace `SignTxOutput`'s merge,
  P2SH and stake handling. Prefer the dcroxide crates here; `dcr-rs` `hd.rs`
  may serve the branch derivation in `Open` if the address-manager spec
  adopts it.

**Hazards.**

1. *Back-pressure model.* Go's rendezvous channels under a mutex give strict
   per-client ordering and block the wallet on slow clients. A Rust port can
   reproduce this with zero-capacity channels, or decouple with per-client
   unbounded queues. Decoupling changes memory behaviour and removes the
   implicit throttling but keeps per-client order; it must not reorder the
   tip-changed → confirmations → transactions sequence for a given chain
   switch. Decide once (§13).
2. *Sending inside a write transaction.* If the Rust store's read
   transactions can block behind an open writer, the Go lock order (§7.1)
   deadlocks. Building the notification payload inside the transaction and
   sending after commit avoids this but changes Q17; payload contents must
   still be computed against the in-transaction state.
3. *Wrapping arithmetic.* `LastReturned - LastUsed` in `Open` and
   `LastUsed + gapLimit` in account notifications rely on `u32` wraparound
   with `0xFFFFFFFF` as "none"; use `wrapping_sub`/`wrapping_add`.
4. *Signed height arithmetic.* All predicates use `i32`; `stopHeight` may be
   negative on purpose (Q1), and `ForUnspentUnexpiredTickets` computes a
   `u32` sum then converts (Q3).
5. *Nil versus empty.* `MyInputs` is nil with no debits, `MyOutputs` is always
   non-nil, `UnminedTransactionHashes` is nil when empty,
   `MainTipChangedNotification.AttachedBlocks` is non-nil while
   `DetachedBlocks` may be nil. Protobuf encodes these identically, so the
   distinction matters only for Go-API parity tests.
6. *Secrets.* Private keys obtained for signing are zeroed on return in Go;
   use zeroising containers. Do not reproduce Q6 without a decision.
7. *Determinism.* Map-ordered outputs (Q13, Q16e, Q24) cannot be matched
   byte-for-byte against a Go oracle; compare as sets.
8. *Panics.* Decide per site (§8) whether to return an error instead.

**Size estimate: L.** The line count is modest (about 2,700 lines of Go) and
most functions are thin, but three parts carry real weight: the notification
server (concurrency semantics, five client kinds, accumulation state machine,
no upstream tests, direct Decrediton exposure), `SignTransaction` (script
engine, merge semantics, error-string parity), and `Open` (touches every
persistent subsystem and fixes in-memory state for the rest of the wallet).
The remainder is S-sized glue once the store, address manager and network
backend traits exist.

---

## 13. Open questions

1. **Notification delivery model.** Keep rendezvous-under-mutex semantics
   (wallet stalls on a slow gRPC client, as upstream) or buffer per client?
   Needs a project-level decision; affects hazards 1 and 2 above.
2. **`NewBalances`.** Compute it (cost: full unspent scan per notification,
   and its failure suppresses the notification) or drop it since no consumer
   transmits it (Q13)?
3. **`RemovedTransactionNotifications`.** Keep the subscriber API for parity
   although nothing consumes it (Q17)?
4. **Q6 (WIF keys in error text).** Reproduce byte-for-byte or redact?
   Reproducing leaks caller-supplied keys back to the same caller; redacting
   changes one JSON-RPC error string.
5. **Q5 (nil dereference) and the other panics in §8.** Convert to errors or
   keep process-fatal behaviour?
6. **Q3 (short-chain ticket window) and Q2 (account row order versus
   `getreceivedbyaccount` indexing).** Both look like upstream bugs with
   externally visible results. Reproduce, or fix and record the divergence?
7. **Q4 (stale accumulation after a failed chain switch).** Reproduce or clear
   on failure?
8. UNVERIFIED: whether the syncers can call `ChainSwitch` with a chain whose
   last block is already the wallet tip (the way to reach Q5): either two
   concurrent calls for the same best chain, or an SPV initial-sync batch
   (`spv/sync.go:1796-1811`) that ends at the current tip. Settled by the sync
   specs (`chain/sync.go`, `spv/sync.go` locking around `EvaluateBestChain` +
   `ChainSwitch`, and the header-batch pipeline).
9. UNVERIFIED: that the block-record bucket is always contiguous from the
   seek height in `ImportCFiltersV2` (Q22). Settled by the `wallet/udb`
   block-record spec.
10. UNVERIFIED by execution: every byte-exact string in §5 and §8 was derived
    by reading format strings and the `errors` package, not by running the
    Go code (running was out of bounds for this study). The oracle vectors in
    §11 settle them.
11. UNVERIFIED: the result of `CreateSignature` with `idx >= len(tx.TxIn)` and
    no `SigHashAnyOneCanPay` (§8). Settled by a Go oracle vector; the callers'
    own index validation (JSON-RPC and gRPC specs) decides whether it is
    reachable.
12. Whether `Wallet.Run` should, like upstream, create a new mix client on
    every syncer run, or own a single client for the wallet's lifetime; the
    mixing spec should decide.
