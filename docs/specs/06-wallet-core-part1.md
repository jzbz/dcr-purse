# 06 — Wallet core, part 1 (`wallet/wallet.go` lines 1–2393)

## Header

**Scope**

| File | Lines in scope | Total lines |
|---|---|---|
| `wallet/wallet.go` | 1–2393 (every declaration that starts in this range) | 5828 |
| `wallet/common.go` | all | 61 |
| `wallet/log.go` | all | 38 |
| `wallet/doc.go` | all | 10 |
| `wallet/network.go` | all | 142 |
| `deployments/deployments.go` | all (the whole `deployments` package) | 157 |

Sibling specs cover `wallet/wallet.go` 2394–4428 (part 2) and 4429–5828 (part 3). `Create`,
`CreateWatchOnly`, `Open` and `Run` are declared in part 3 (`wallet/wallet.go:5292`, `:5316`, `:5340`, `:5545`),
not in this range, although the `Wallet` and `Config` structs they populate are declared here. The
cross-references this spec needed into the other ranges are listed in section 14.

**Parity pin**

| Component | Version read |
|---|---|
| dcrwallet | commit `54b85b33`, module `decred.org/dcrwallet/v5`, version 2.2.0-pre |
| `github.com/decred/dcrd/chaincfg/v3` | v3.3.0 (deployment tables, vote IDs) |
| `github.com/decred/dcrd/blockchain/stake/v5` | v5.0.3 (`VoteBits`, `TreasuryVoteT`, tspend script layout, sstx commitment helpers) |
| `github.com/decred/dcrd/blockchain/standalone/v2` | v2.3.0 (`VerifyInclusionProof`, `IsCoinBaseTx`) |
| `github.com/decred/dcrd/dcrec/secp256k1/v4` | v4.4.1 (`ecdsa.Sign`, `SignCompact`, `RecoverCompact`, `NonceRFC6979`) |
| `github.com/decred/dcrd/hdkeychain/v3` | v3.1.3 (`HardenedKeyStart`, `ErrInvalidChild`) |
| `github.com/decred/dcrd/wire` | v1.8.0 (`WriteVarString`, net magics, `InvTypeTx`, `TxTreeStake`) |
| `github.com/decred/dcrd/gcs/v4` | v4.1.1 (`KeySize`, `blockcf2.B`/`M`) |
| `github.com/decred/dcrd/rpc/jsonrpc/types/v4` | v4.4.0 (`AgendaInfo`, status strings) |
| `github.com/decred/dcrd/dcrjson/v4` | v4.2.0 (RPC error codes) |
| `github.com/decred/slog` | v1.2.0 (`Disabled` logger) |

**Summary**

This range holds the `Wallet` object itself (its fields, mutexes and configuration) and roughly
seventy thin-to-medium methods layered over the address manager (`udb.Manager`) and transaction
store (`udb.Store`): block disapproval percentage, the in-memory treasury-spend (tspend) cache,
agenda vote preferences and the derived vote bits, per-tspend and per-treasury-key vote policies
(wallet-wide and per-ticket), relay-fee and VSP-fee accessors, main-chain queries (tip, header,
cfilter, block locators), transaction-filter loading for the network backend (HD addresses,
imported addresses, unspent outpoints, tickets), back-filling of missing compact filters, the
VSP-fee UTXO-split retry wrapper around ticket purchasing, the passphrase lock/unlock state
machine with its timeout channels, per-account passphrase wrappers, balance queries, message and
hash signing, account lookup/rename/creation, and a few transaction lookups. `wallet/network.go`
defines the `NetworkBackend` interface that both syncers implement plus the offline stub, and the
`deployments` package supplies hardcoded DCP0001–3 activation heights and the DCP0010/DCP0012
activation tests that select the vote subsidy split. Most persisted bytes touched here belong to
other specs (address manager, tx store); this spec owns the agenda-preference and treasury-policy
key/value encodings as they are used, the message-signing preimage, the block-locator sequence,
and every log line, error string and ordering produced by the functions in range.

---

## 2. Role in the system

`wallet.Wallet` is the single in-process object that every front end drives: the JSON-RPC server
(`internal/rpc/jsonrpc`), the gRPC server (`internal/rpc/rpcserver`), the two syncers
(`chain.Syncer` for dcrd RPC mode, `spv.Syncer` for SPV mode), the ticket buyer (`ticketbuyer`),
the VSP client code inside the `wallet` package, and `dcrwallet.go` (startup unlock, offline mode).
It is constructed by `wallet.Open` (part 3) from a `Config` and lives for the life of the process.

Callers of the functions in this range (non-test, found by repository search):

| Function group | Called from |
|---|---|
| Disapprove percent | JSON-RPC `disapprovepercent`/`setdisapprovepercent` (`internal/rpc/jsonrpc/methods.go:821`, `:3868`); vote creation (`wallet/chainntfns.go:775`) |
| TSpend cache (`AddTSpend`, `IsTSpendCached`, `GetAllTSpends`) | RPC syncer mempool load and `tspend` notification (`chain/sync.go:703`, `:930`); SPV syncer (`spv/sync.go:1027`, `:1054`, `:1148`); vote creation (`wallet/chainntfns.go:790`); JSON-RPC and gRPC tspend-policy listings (`internal/rpc/jsonrpc/methods.go:3978`, `internal/rpc/rpcserver/server.go:3370`, `:4210`) |
| Agenda choices / vote bits | JSON-RPC `getvotechoices`, `setvotechoice`, `walletinfo` (`methods.go:2820`, `:4800`, `:5512`); gRPC `VoteChoices`, `SetVoteChoices` (`server.go:3270`, `:3315`); VSP fee payment (`wallet/feepayment.go:510`, `wallet/vspticket.go:177`); vote creation (`wallet/chainntfns.go:733`, `:769`); RPC syncer startup log (`chain/sync.go:650`) |
| Treasury/tspend policies | JSON-RPC `treasurypolicy`, `settreasurypolicy`, `tspendpolicy`, `settspendpolicy` (`methods.go:3814`–`:4053`); gRPC equivalents (`server.go:3347`–`:3509`, `:4195`, `:4213`); vote creation (`wallet/chainntfns.go:816`); VSP (`wallet/feepayment.go:526`, `wallet/vspticket.go:188`, `:213`) |
| `LoadActiveDataFilters`, `watchHDAddrs` | RPC syncer startup (`chain/sync.go:368`, `:390`); gRPC `LoadActiveDataFilters` (`server.go:1950`); after block attach, rescan and publish (`wallet/chainntfns.go:294`, `:424`, `:919`, `wallet/createtx.go:353`, `:1253`) |
| `FetchMissingCFiltersWithProgress` | RPC syncer only (`chain/sync.go:659`). `FetchMissingCFilters` has no caller. |
| `BlockLocators` | Both syncers (`chain/sync.go:221`, `:348`, `:407`; `spv/backend.go:296`; `spv/sync.go:1430`, `:1950`) |
| `PurchaseTickets` | JSON-RPC `purchaseticket` (`methods.go:3476`), gRPC `PurchaseTickets` (`server.go:1908`), ticket buyer (`ticketbuyer/tb.go:305`) |
| `Unlock`/`Lock` and friends | `dcrwallet.go:243`, `:509`, `:521` (startup unlock and prompt); JSON-RPC `walletpassphrase`, `walletlock` (`internal/rpc/jsonrpc/methods.go:5584`, `:5564`), `walletpassphrasechange`; gRPC handlers that take a passphrase; ticket buyer (`ticketbuyer/tb.go:65`, `:235`). Neither syncer calls them. |
| Account, balance, signing, address queries | JSON-RPC and gRPC handlers; `dcrwallet.go:259`–`:296` (account name resolution for ticket buyer and mixing config); ticket buyer (`ticketbuyer/tb.go:260`) |
| `OfflineNetworkBackend` | `dcrwallet.go:436` when `--offline` is set |
| `RelayFee` / `SetRelayFee` | `SetRelayFee`: JSON-RPC `settxfee` only (`internal/rpc/jsonrpc/methods.go:4769`). `RelayFee`: JSON-RPC handlers (`methods.go:889`, `:1805`, `:2854`, `:3731`, `:3743`, `:4384`, `:5249`, `:5511`), mixing (`wallet/mixing.go:322`), transaction authoring (`wallet/createtx.go:665`, `:879`, `:1007`, `:1129`, `:1225`, `wallet/multisig.go:90`) |
| `FetchOutput` | JSON-RPC `spendoutputs` only, through `spendOutputsInputSource` (`internal/rpc/jsonrpc/methods.go:4257`, called at `:4366`) |
| `CommittedTickets` | gRPC `CommittedTickets` only (`internal/rpc/rpcserver/server.go:1968`) |
| `RangeCFiltersV2` | gRPC only (`internal/rpc/rpcserver/server.go:2204`) |
| `CFilterV2` | JSON-RPC `getcfilterv2` (`internal/rpc/jsonrpc/methods.go:1967`) |
| `GetTransactionsByHashes` | SPV `getdata` responder (`spv/sync.go:818`); VSP ticket code (`wallet/vspticket.go:42`, `:81`, `:301`) |
| `TxBlock` | VSP ticket code (`wallet/vspticket.go:243`) |
| `InitialHeight` | SPV syncer only (`spv/backend.go:44`, `:311`, `spv/sync.go:205`–`206`) |
| `VotingEnabled` | RPC syncer startup (`chain/sync.go:645`); JSON-RPC `walletinfo` (`internal/rpc/jsonrpc/methods.go:5515`) |
| `Consolidate`, `CreateMultisigTx` | JSON-RPC only (`internal/rpc/jsonrpc/methods.go:597`, `:4655`) |
| `ChangePrivatePassphrase`, `ChangePublicPassphrase` | JSON-RPC `walletPassphraseChange` and `walletPubPassphraseChange` handlers (`internal/rpc/jsonrpc/methods.go:5602`, `:5707`); gRPC `ChangePassphrase` (`internal/rpc/rpcserver/server.go:1567`, `:1575`) |
| `SignHashes` | gRPC only (`internal/rpc/rpcserver/server.go:3763`) |
| `deployments.DCP0001.Active` | `wallet/difficulty.go:797`, `:818`, `:896`, `wallet/chainntfns.go:246`, `wallet/wallet.go:4018`, `:4134` |
| `deployments.DCP0010Active`/`DCP0012Active` | `wallet/wallet.go:1637`, `:1642` (this range); `wallet/chainntfns.go:713`, `:718`; `wallet/createtx.go:1286`, `:1291` |

What the range calls:

- `wallet/walletdb` (`View`, `Update`) for every database access; one call is one bbolt transaction.
- `wallet/udb` — `Manager` (address manager) and `Store` (tx store) methods, plus the free functions
  for agenda preferences (`wallet/udb/agendas.go`) and treasury policies (`wallet/udb/treasury.go`).
- `errors` (the dcrwallet error package; section 8).
- `validate.CFilterV2HeaderCommitment` (`validate/validate.go:71`).
- `wallet/txrules`, `wallet/txsizes` for the split-transaction fee arithmetic in `PurchaseTickets`.
- Other files of package `wallet`: `createtx.go` (`purchaseTickets`, `authorTx`, `recordAuthoredTx`,
  `publishAndWatch`, `compressWallet`, `txToMultisig`), `addresses.go` (`deriveChildAddress`,
  `deriveChildAddresses`, `deriveBranches`, `NewInternalAddress`, address buffers), `rescan.go`
  (`RescanPoint`), `difficulty.go` (`NextStakeDifficulty`), `notifications.go`
  (`notifyAccountProperties`), `vsp.go` (`VSPClient.FeePercentage`).
- The `NetworkBackend` passed in or stored on the wallet (`LoadTxFilter`, `CFiltersV2`,
  `WrapContext`, and, through `publishAndWatch`, `PublishTransactions`).

---

## 3. Public surface inventory

Conventions: "wraps with op" means the returned error is `errors.E(op, err)` where `op` is the string
shown; "unwrapped" means the callee's error is returned unchanged. `View`/`Update` name the kind of
database transaction used (read-only / read-write). Citations are `wallet/wallet.go:<line>` unless a
file is given.

### 3.1 Constants and package variables

| Name | Value | Source | Contract |
|---|---|---|---|
| `InsecurePubPassphrase` | `"public"` (string const) | `:66` | Default public (outer) passphrase. Used by config defaults, wallet creation and gRPC handlers when no public passphrase is given. |
| `SimulationPassphrase` | `[]byte("password")` (exported var) | `:72` | Private passphrase of a simnet wallet made with `--createtemp`; tried automatically at startup on simnet (`dcrwallet.go:509`). |
| `waddrmgrNamespaceKey` | `[]byte("waddrmgr")` | `:77` | Top-level bucket name of the address-manager namespace. |
| `wtxmgrNamespaceKey` | `[]byte("wtxmgr")` | `:78` | Top-level bucket name of the tx-store namespace. |
| `scriptVersionAssumed` | `0` (untyped const) | `:83` | Script version assumed wherever real versions are not threaded through. Not used inside this range; used in parts 2–3, `chainntfns.go`, `createtx.go`, `notifications.go`. |
| `log` | `slog.Disabled` | `wallet/log.go:13` | Package logger; silent until `UseLogger` is called. The daemon installs the `WLLT` subsystem logger (`log.go:32` at repository root; `internal/loggers/loggers.go:48`). |
| `debugLog`, `infoLog` | zero values of `debugLogger`, `infoLogger` | `wallet/log.go:24`, `:33` | Adapters exposing `Print`, `Printf`, `Log`, `Logf` at debug/info level. No references anywhere in the repository (dead code). |
| `errOfflineNetworkBackend` | `errors.New("operation not supported in offline mode")` | `wallet/network.go:92` | Returned by every failing `OfflineNetworkBackend` method. Plain error, kind `Other`. |
| `closedDone` | a `chan struct{}` closed in `init` | `wallet/network.go:130`–`134` | Unused (dead code). |
| `deployments.DCP0001`, `DCP0002`, `DCP0003` | see section 6 | `deployments/deployments.go:32`, `:41`, `:50` | Hardcoded activation heights. Exported mutable vars. `DCP0002` and `DCP0003` have no references outside their declaration. |
| `deployments.lockedinStatus`, `activeStatus` | `"lockedin"`, `"active"` | `deployments/deployments.go:75`–`78` | dcrd `AgendaInfo.Status` strings compared by the DCP0010/0012 tests. |

### 3.2 Types

| Type | Source | Definition and contract |
|---|---|---|
| `StakeDifficultyInfo` | `:86` | `{BlockHash *chainhash.Hash; BlockHeight int64; StakeDifficulty int64}`. Exported; no references in the repository. |
| `outpoint` | `:94` | `{hash chainhash.Hash; index uint32}`. Map key of `lockedOutpoints`. No tree field: outpoints in different trees with the same hash and index are the same lock. |
| `Wallet` | `:102` | Section 3.3. |
| `Config` | `:183` | Section 3.4. |
| `TreasuryKeyPolicy` (struct) | `:633` | `{PiKey []byte; Ticket *chainhash.Hash; Policy stake.TreasuryVoteT}`. `Ticket` is nil for wallet-wide policies. Shares its name with the method `(*Wallet).TreasuryKeyPolicy`. |
| `MissingCFilterProgress` | `:1435` | `{Err error; BlockHeightStart int32; BlockHeightEnd int32}`. One value per fetched batch, or a single final value with only `Err` set. |
| `PurchaseTicketsRequest` | `:1550` | Exported fields in order: `Count int`, `SourceAccount uint32`, `VotingAccount uint32`, `MinConf int32`, `Expiry int32`, `DontSignTx bool`, `Mixing bool`, `MixedAccount uint32`, `MixedAccountBranch uint32`, `MixedSplitAccount uint32`, `ChangeAccount uint32`, `VSPClient *VSPClient`; unexported `extraSplitOutput *Input` set only by the retry path of `PurchaseTickets`. The request is mutated by `PurchaseTickets` (section 5.9). |
| `PurchaseTicketsResponse` | `:1578` | `{TicketHashes []*chainhash.Hash; Tickets []*wire.MsgTx; SplitTx *wire.MsgTx}`. |
| `Balances` | `:1899` | `{Account uint32; ImmatureCoinbaseRewards, ImmatureStakeGeneration, LockedByTickets, Spendable, Total, VotingAuthority, Unconfirmed dcrutil.Amount}` in that field order. Structurally identical to `udb.Balances` (an alias of an anonymous struct, `wallet/udb/txmined.go:3548`), which is why a `udb` value assigns to it directly. |
| `CreditCategory` | `:2349` | `byte`. `CreditReceive = 0`, `CreditGenerate = 1`, `CreditImmature = 2` (`:2352`–`2356`). `String()` returns `"receive"`, `"generate"`, `"immature"`, and `"unknown"` for any other value (`:2361`). These strings are the `category` values of `listtransactions`/`gettransaction` results. |
| `BlockIdentity` | `wallet/common.go:20` | `{Hash chainhash.Hash; Height int32}`. `None()` is true when the value equals `{zero hash, -1}` or `{zero hash, 0}` (`wallet/common.go:27`–`32`). |
| `OutputRedeemer` | `wallet/common.go:35` | `{TxHash chainhash.Hash; InputIndex uint32}`. |
| `P2SHMultiSigOutput` | `wallet/common.go:47` | `{OutPoint wire.OutPoint; OutputAmount dcrutil.Amount; ContainingBlock BlockIdentity; P2SHAddress *stdaddr.AddressScriptHashV0; RedeemScript []byte; M, N uint8; Redeemer *OutputRedeemer}`. `Redeemer` is nil unless spent. Produced by `wallet/multisig.go`. |
| `debugLogger`, `infoLogger` | `wallet/log.go:22`, `:31` | Empty structs with `Print(args ...any)`, `Printf(format string, args ...any)`, `Log(args ...any)`, `Logf(format string, args ...any)` forwarding to `log.Debug`/`Debugf` or `log.Info`/`Infof`. Unused. |
| `FilterProof` | `wallet/network.go:26` | Type alias of the anonymous struct `{Filter *gcs.FilterV2; ProofIndex uint32; Proof []chainhash.Hash}`. One element per requested block in `CFiltersV2` results. |
| `NetworkBackend` | `wallet/network.go:35` | Interface, section 3.5. |
| `Caller` | `wallet/network.go:83` | Interface `{Call(ctx context.Context, method string, res any, args ...any) error}`. Positional-parameter RPC call; `res` is a pointer to a struct, slice or map, or nil. `wallet.Caller` itself has no references anywhere in the repository; `rpc/client/dcrd` (`rpc/client/dcrd/calls.go:31`) and `rpc/client/dcrwallet` (`rpc/client/dcrwallet/client.go:11`) each declare their own `Caller` interface with the same method. |
| `OfflineNetworkBackend` | `wallet/network.go:96` | Empty struct implementing `NetworkBackend`; section 3.5. |
| `deployments.HardcodedDeployment` | `deployments/deployments.go:23` | `{MainNetActivationHeight, TestNet2ActivationHeight, TestNet3ActivationHeight, SimNetActivationHeight int32}`. A negative height means inactive or unknown. |
| `deployments.Querier` | `deployments/deployments.go:82` | Interface `{Deployments(context.Context) (map[string]dcrdtypes.AgendaInfo, error)}`. Implemented only by `chain.Syncer` (`chain/backend.go:63`, via dcrd `getblockchaininfo`). `spv.Syncer` does not implement it. |

### 3.3 `Wallet` struct fields

All fields are unexported except `NtfnServer`. "Init" gives the value assigned by `Open`
(`wallet/wallet.go:5386`–`5527`, part 3).

| Field | Type | Guard | Init | Purpose |
|---|---|---|---|---|
| `disapprovePercent` | `atomic.Uint32` | atomic | 0 | Percentage of blocks to disapprove when voting (non-mainnet only). |
| `db` | `walletdb.DB` | — | `cfg.DB.internal()` | Database handle. |
| `manager` | `*udb.Manager` | own mutex | `udb.Open` | Address manager. |
| `txStore` | `*udb.Store` | — | `udb.Open` | Transaction store. |
| `stakeSettingsLock` | `sync.Mutex` | — | — | Guards `defaultVoteBits`, `votingEnabled` (see its row), `tspends` and the four policy maps through `vspTSpendKeyPolicy`. `manualTickets` and `subsidyCache`, declared in the same block, are not guarded by it. |
| `defaultVoteBits` | `stake.VoteBits` | `stakeSettingsLock` | `readDBVoteBits` | Cached wallet-wide vote bits and extended bits. |
| `votingEnabled` | `bool` | read under `stakeSettingsLock` by `VotingEnabled`, read without it at `wallet/chainntfns.go:701`; never written after `Open` | `cfg.VotingEnabled` | Whether the wallet votes its tickets. |
| `manualTickets` | `bool` | none | `cfg.ManualTickets` | Syncers must not auto-add ticket purchases. |
| `subsidyCache` | `*blockchain.SubsidyCache` | internal | `NewSubsidyCache(params)` | Used by vote creation. |
| `tspends` | `map[chainhash.Hash]wire.MsgTx` | `stakeSettingsLock` | empty map | In-memory tspend cache, keyed by tx hash, values stored by value. Never persisted. |
| `tspendPolicy` | `map[chainhash.Hash]stake.TreasuryVoteT` | `stakeSettingsLock` | `udb.TSpendPolicies` | Wallet-wide policy per tspend hash. |
| `tspendKeyPolicy` | `map[string]stake.TreasuryVoteT` | `stakeSettingsLock` | `udb.TreasuryKeyPolicies` | Wallet-wide policy per treasury ("Pi") key; map key is the raw key bytes as a string. |
| `vspTSpendPolicy` | `map[udb.VSPTSpend]stake.TreasuryVoteT` | `stakeSettingsLock` | `udb.VSPTSpendPolicies` | Per-ticket policy per tspend hash. |
| `vspTSpendKeyPolicy` | `map[udb.VSPTreasuryKey]stake.TreasuryVoteT` | `stakeSettingsLock` | `udb.VSPTreasuryKeyPolicies` | Per-ticket policy per treasury key. |
| `gapLimit` | `uint32` | immutable | `cfg.GapLimit` | Address gap limit. |
| `watchLast` | `uint32` | immutable | `cfg.WatchLast` | If nonzero, limits how far back HD addresses are watched. |
| `accountGapLimit` | `int` | immutable | `cfg.AccountGapLimit` | Maximum run of unused accounts. |
| `initialHeight` | `int32` | immutable after `Open` | main chain tip height at `Open` | Tip height before syncing. |
| `networkBackend` | `NetworkBackend` | `networkBackendMu` | nil | Current backend. |
| `networkBackendMu` | `sync.Mutex` | — | — | |
| `lockedOutpoints` | `map[outpoint]struct{}` | `lockedOutpointMu` | empty map | Outpoints excluded from coin selection. |
| `lockedOutpointMu` | `sync.Mutex` | — | — | Also serialises transaction authoring against filter loading (section 7). |
| `relayFee` | `dcrutil.Amount` | `relayFeeMu` | `cfg.RelayFee` | Fee rate in atoms per kB. |
| `relayFeeMu` | `sync.Mutex` | — | — | |
| `allowHighFees` | `bool` | immutable | `cfg.AllowHighFees` | Part 3. |
| `disableCoinTypeUpgrades` | `bool` | immutable | `cfg.DisableCoinTypeUpgrades` | Read only at `wallet/discovery.go:954`. |
| `recentlyPublished` | `map[chainhash.Hash]struct{}` | `recentlyPublishedMu` | empty map | Used by `wallet/chainntfns.go` and `wallet/createtx.go`; not referenced in `wallet/wallet.go` outside `Open`. |
| `recentlyPublishedMu` | `sync.Mutex` | — | — | |
| `logRescannedTransactions` | `bool` | `logRescannedTransactionsMu` | false | `wallet/rescan.go`. |
| `logRescannedTransactionsMu` | `sync.Mutex` | — | — | |
| `addressBuffers` | `map[uint32]*bip0044AccountData` | `addressBuffersMu` | one entry per BIP0044 and imported-xpub account | In-memory address cursors (`wallet/addresses.go:446`–`460`). |
| `addressBuffersMu` | `sync.Mutex` | — | — | |
| `passphraseUsedMu` | `sync.RWMutex` | — | — | Serialises `Lock` and `ChangePrivatePassphrase` (writers) against `Unlock` (reader). Used only at `:1715`–`1732`, `:1843`–`1848`, `:1872`–`1873`. |
| `passphraseTimeoutMu` | `sync.Mutex` | — | — | Guards `passphraseTimeoutCancel`. |
| `passphraseTimeoutCancel` | `chan struct{}` | `passphraseTimeoutMu` | nil | Non-nil exactly when an unlock timeout is armed; buffered with capacity 1. |
| `mixingEnabled` | `bool` | immutable | `cfg.MixingEnabled` | Gate for mixing requests. |
| `mixpool` | `*mixpool.Pool` | internal | created only if `mixingEnabled` | Mixing spec. |
| `mixSems` | `mixSemaphores` | channels | `newMixSemaphores(cfg.MixSplitLimit)` | Mixing spec. |
| `mixClient` | `atomic.Pointer[mixclient.Client]` | atomic | set by `Run` | Mixing spec. |
| `cachedBlake3WorkDiffCandidateAnchor` | `*wire.BlockHeader` | `cachedBlake3WorkDiffCandidateAnchorMu` | nil | `wallet/difficulty.go`. |
| `cachedBlake3WorkDiffCandidateAnchorMu` | `sync.Mutex` | — | — | |
| `NtfnServer` | `*NotificationServer` | internal | `newNotificationServer(w)` | Exported notification hub. |
| `chainParams` | `*chaincfg.Params` | immutable | `cfg.Params` | |
| `deploymentsByID` | `map[string]*chaincfg.ConsensusDeployment` | immutable | every deployment of every stake version; `Open` panics with `reused deployment ID "<id>"` on a duplicate | Lookup by agenda ID. |
| `minTestNetTarget` | `*big.Int` | immutable | on testnet3: `PowLimit >> 6`; else nil | `wallet/difficulty.go`. |
| `minTestNetDiffBits` | `uint32` | immutable | on testnet3: `BigToCompact(minTestNetTarget)`; else 0 | |
| `vspMaxFee` | `dcrutil.Amount` | immutable | `cfg.VSPMaxFee` | |
| `vspClientsMu` | `sync.Mutex` | — | — | `wallet/vspclientloader.go`. |
| `vspClients` | `map[string]*VSPClient` | `vspClientsMu` | empty map | |
| `dialer` | `DialFunc` | immutable | `cfg.Dialer` | Network dial function (proxy aware). |

### 3.4 `Config` fields

Field order as declared (`:183`–`204`): `DB DB`, `PubPassphrase []byte`, `VotingEnabled bool`,
`GapLimit uint32`, `WatchLast uint32`, `AccountGapLimit int`, `MixSplitLimit int`,
`DisableCoinTypeUpgrades bool`, `MixingEnabled bool`, `ManualTickets bool`, `AllowHighFees bool`,
`RelayFee dcrutil.Amount`, `VSPMaxFee dcrutil.Amount`, `Params *chaincfg.Params`, `Dialer DialFunc`.

`PubPassphrase` is not copied into the `Wallet`: `Open` passes it to `udb.Upgrade` and `udb.Open` (`wallet/wallet.go:5356`, `:5428`) and nothing in this range reads it. `DB` is the opaque interface from `wallet/db.go:15` (`io.Closer` plus an unexported `internal()`
accessor). `DialFunc` is `func(ctx context.Context, network, addr string) (net.Conn, error)`
(`wallet/vsp.go:27`). The loader fills `Config` from command-line options
(three `wallet.Config` literals: `CreateWatchingOnlyWallet` at `internal/loader/loader.go:169`–`185`, `CreateNewWallet` at `:259`–`274`, `OpenExistingWallet` at `:318`–`334`); defaults for those options are in section 6. The literals are not identical: the one in `CreateNewWallet` does not set `MixSplitLimit`, so a wallet opened through that path gets `MixSplitLimit = 0` (and therefore `newMixSemaphores(0)`, unbuffered semaphore channels, `wallet/mixing.go:240`–`246`) regardless of `--mixsplitlimit`; the other two pass `l.mixSplitLimit`.

### 3.5 `NetworkBackend` interface and `OfflineNetworkBackend`

Method set, in declaration order (`wallet/network.go:35`–`57`):

| Method | Contract |
|---|---|
| `Blocks(ctx, blockHashes []*chainhash.Hash) ([]*wire.MsgBlock, error)` | Fetch full blocks. |
| `CFiltersV2(ctx, blockHashes []*chainhash.Hash) ([]FilterProof, error)` | Fetch version 2 cfilters with header-commitment inclusion proofs, one result per hash, same order. |
| `PublishTransactions(ctx, txs ...*wire.MsgTx) error` | Broadcast transactions. |
| `PublishMixMessages(ctx, msgs ...mixing.Message) error` | Broadcast mixing messages. |
| `LoadTxFilter(ctx, reload bool, addrs []stdaddr.Address, outpoints []wire.OutPoint) error` | Add addresses/outpoints to the transaction filter; `reload` true first clears it. The wallet reuses the `outpoints` and `addrs` backing arrays after the call returns (section 5.5), so an implementation must not retain the slices. |
| `Rescan(ctx, blocks []chainhash.Hash, save func(block *chainhash.Hash, txs []*wire.MsgTx) error) error` | Rescan blocks for filter matches. |
| `StakeDifficulty(ctx) (dcrutil.Amount, error)` | Next ticket price as reported by the network. Always an error over the wire protocol (`spv/backend.go:633`). |
| `Synced(ctx) (bool, int32)` | Whether synced, and the target height (tip height when synced). |
| `WrapContext(ctx) (context.Context, context.CancelFunc)` | Derived context cancelled when the backend disconnects. The cancel func must always be called. |

`OfflineNetworkBackend` (`wallet/network.go:96`–`142`): `Blocks`, `CFiltersV2` return
`(nil, errOfflineNetworkBackend)`; `PublishTransactions`, `PublishMixMessages`, `LoadTxFilter`,
`Rescan` return `errOfflineNetworkBackend`; `StakeDifficulty` returns `(0, errOfflineNetworkBackend)`;
`Synced` returns `(true, 0)`; `WrapContext` returns the same context and a no-op cancel func. All
methods have value receivers. A compile-time assertion checks interface conformance
(`wallet/network.go:142`).

### 3.6 Functions and methods in `wallet/wallet.go` 1–2393

| # | Declaration | Line | Contract |
|---|---|---|---|
| 1 | `(w *Wallet) DisapprovePercent() uint32` | 207 | Atomic load. |
| 2 | `(w *Wallet) SetDisapprovePercent(percent uint32)` | 213 | Atomic store. No range or network check (the JSON-RPC handler rejects mainnet and values over 100). |
| 3 | `(w *Wallet) FetchOutput(ctx, outPoint *wire.OutPoint) (*wire.TxOut, error)` | 219 | `View`. Looks up the transaction by `outPoint.Hash` (unmined first, else latest mined) and returns `TxOut[outPoint.Index]`. Wraps with op `wallet.FetchOutput`. An unknown transaction is kind `NotExist` with inner message `tx <hash> not found` (`wallet/udb/txquery.go:273`–`274`; `<hash>` is the reversed-hex display form), giving `wallet.FetchOutput: item does not exist: tx <hash> not found`. The returned `*wire.TxOut` belongs to a freshly deserialised transaction, not to shared state. No bounds check on the index (section 10, Q3). The tree is ignored. |
| 4 | `(w *Wallet) VotingEnabled() bool` | 241 | Reads `votingEnabled` under `stakeSettingsLock`. |
| 5 | `(w *Wallet) IsTSpendCached(hash *chainhash.Hash) bool` | 249 | Membership test in `tspends` under `stakeSettingsLock`. |
| 6 | `(w *Wallet) AddTSpend(tx wire.MsgTx) error` | 259 | Section 5.1. |
| 7 | `(w *Wallet) GetAllTSpends(ctx) []*wire.MsgTx` | 274 | Section 5.1. |
| 8 | `voteVersion(params *chaincfg.Params) uint32` | 291 | Section 5.2. |
| 9 | `CurrentAgendas(params *chaincfg.Params) (version uint32, agendas []chaincfg.ConsensusDeployment)` | 308 | Section 5.2. |
| 10 | `(w *Wallet) readDBVoteBits(dbtx walletdb.ReadTx) stake.VoteBits` | 316 | Section 5.2. Called by `Open`. |
| 11 | `(w *Wallet) readDBTicketVoteBits(dbtx walletdb.ReadTx, ticketHash *chainhash.Hash) (stake.VoteBits, bool)` | 346 | Section 5.2. Called by `SetAgendaChoices` and vote creation. |
| 12 | `(w *Wallet) readDBTreasuryPolicies(dbtx) (map[chainhash.Hash]stake.TreasuryVoteT, map[udb.VSPTSpend]stake.TreasuryVoteT, error)` | 377 | `udb.TSpendPolicies` then `udb.VSPTSpendPolicies`; first error wins. Called by `Open`. |
| 13 | `(w *Wallet) readDBTreasuryKeyPolicies(dbtx) (map[string]stake.TreasuryVoteT, map[udb.VSPTreasuryKey]stake.TreasuryVoteT, error)` | 387 | `udb.TreasuryKeyPolicies` then `udb.VSPTreasuryKeyPolicies`. Called by `Open`. |
| 14 | `(w *Wallet) VoteBits() stake.VoteBits` | 401 | Returns a copy of `defaultVoteBits` under `stakeSettingsLock`. The `ExtendedBits` slice shares its backing array with the cached value. |
| 15 | `(w *Wallet) AgendaChoices(ctx, ticketHash *chainhash.Hash) (choices map[string]string, voteBits uint16, err error)` | 412 | Section 5.3. |
| 16 | `(w *Wallet) SetAgendaChoices(ctx, ticketHash *chainhash.Hash, choices map[string]string) (voteBits uint16, err error)` | 476 | Section 5.3. |
| 17 | `(w *Wallet) TreasuryKeyPolicy(pikey []byte, ticket *chainhash.Hash) stake.TreasuryVoteT` | 572 | Section 5.4. |
| 18 | `(w *Wallet) TSpendPolicy(tspendHash, ticketHash *chainhash.Hash) stake.TreasuryVoteT` | 591 | Section 5.4. |
| 19 | `(w *Wallet) TreasuryKeyPolicies() []TreasuryKeyPolicy` | 640 | Section 5.4. |
| 20 | `(w *Wallet) SetTreasuryKeyPolicy(ctx, pikey []byte, policy stake.TreasuryVoteT, ticketHash *chainhash.Hash) error` | 666 | Section 5.4. |
| 21 | `(w *Wallet) SetTSpendPolicy(ctx, tspendHash *chainhash.Hash, policy stake.TreasuryVoteT, ticketHash *chainhash.Hash) error` | 716 | Section 5.4. |
| 22 | `(w *Wallet) VSPMaxFee() dcrutil.Amount` | 764 | Returns `vspMaxFee`. |
| 23 | `(w *Wallet) RelayFee() dcrutil.Amount` | 770 | Reads `relayFee` under `relayFeeMu`. |
| 24 | `(w *Wallet) SetRelayFee(relayFee dcrutil.Amount)` | 779 | Writes `relayFee` under `relayFeeMu`. Not persisted; no validation. |
| 25 | `(w *Wallet) InitialHeight() int32` | 786 | Returns `initialHeight`. |
| 26 | `(w *Wallet) MainChainTip(ctx) (hash chainhash.Hash, height int32)` | 792 | `View`; `txStore.MainChainTip`. The `View` error is discarded: on database failure the zero hash and height 0 are returned. |
| 27 | `(w *Wallet) BlockInMainChain(ctx, hash *chainhash.Hash) (haveBlock, invalidated bool, err error)` | 807 | `View`; `txStore.BlockInMainChain`. Wraps with op `wallet.BlockInMainChain` (only a transaction-begin failure can error). |
| 28 | `(w *Wallet) BlockHeader(ctx, blockHash *chainhash.Hash) (*wire.BlockHeader, error)` | 821 | `View`; `txStore.GetBlockHeader`. Unwrapped. |
| 29 | `(w *Wallet) CFilterV2(ctx, blockHash *chainhash.Hash) ([gcs2.KeySize]byte, *gcs2.FilterV2, error)` | 833 | `View`; `txStore.CFilterV2`. Returns the 16-byte filter key and the filter. Unwrapped. |
| 30 | `(w *Wallet) RangeCFiltersV2(ctx, startBlock, endBlock *BlockIdentifier, f func(chainhash.Hash, [gcs2.KeySize]byte, *gcs2.FilterV2) (bool, error)) error` | 855 | Section 5.6. |
| 31 | `(w *Wallet) watchHDAddrs(ctx, firstWatch bool, n NetworkBackend) (count uint64, err error)` | 891 | Section 5.5. |
| 32 | `(w *Wallet) CoinType(ctx) (uint32, error)` | 1070 | `View`; `manager.CoinType`. Wraps with op `wallet.CoinType`. Kind `WatchingOnly` on wallets created from an account xpub. |
| 33 | `(w *Wallet) CoinTypePrivKey(ctx) (*hdkeychain.ExtendedKey, error)` | 1085 | `View`; `manager.CoinTypePrivKey`. Wraps with op `wallet.CoinTypePrivKey`. Requires an unlocked, non-watching wallet. |
| 34 | `(w *Wallet) LoadActiveDataFilters(ctx, n NetworkBackend, reload bool) (err error)` | 1101 | Section 5.5. |
| 35 | `(w *Wallet) CommittedTickets(ctx, tickets []*chainhash.Hash) ([]*chainhash.Hash, []stdaddr.Address, error)` | 1205 | Section 5.7. |
| 36 | `(w *Wallet) fetchMissingCFilters(ctx, n NetworkBackend, progress chan<- MissingCFilterProgress) error` | 1291 | Section 5.8. |
| 37 | `(w *Wallet) FetchMissingCFilters(ctx, n NetworkBackend) error` | 1422 | Section 5.8. |
| 38 | `(w *Wallet) FetchMissingCFiltersWithProgress(ctx, n NetworkBackend, progress chan<- MissingCFilterProgress)` | 1447 | Section 5.8. |
| 39 | `log2(x int) int` | 1461 | Number of halvings (integer division by 2) until `x` reaches 0; `log2(0) = 0`, `log2(1) = 1`, `log2(8) = 4`. Used only as a slice capacity hint. |
| 40 | `(w *Wallet) BlockLocators(ctx, sidechain []*BlockNode) ([]*chainhash.Hash, int32, error)` | 1482 | Section 5.6. `View`; wraps with op `wallet.BlockLocators`. |
| 41 | `(w *Wallet) blockLocators(dbtx walletdb.ReadTx, sidechain []*BlockNode) ([]*chainhash.Hash, int32, error)` | 1497 | Section 5.6. |
| 42 | `(w *Wallet) Consolidate(ctx, inputs int, account uint32, address stdaddr.Address) (*chainhash.Hash, error)` | 1539 | Calls `w.compressWallet(ctx, "wallet.Consolidate", inputs, account, address)` (`wallet/createtx.go:769`). |
| 43 | `(w *Wallet) CreateMultisigTx(ctx, account uint32, amount dcrutil.Amount, pubkeys [][]byte, nrequired int8, minconf int32) (*CreatedTx, stdaddr.Address, []byte, error)` | 1544 | Calls `w.txToMultisig(ctx, "wallet.CreateMultisigTx", ...)` (`wallet/createtx.go:544`). |
| 44 | `(w *Wallet) PurchaseTickets(ctx, n NetworkBackend, req *PurchaseTicketsRequest) (*PurchaseTicketsResponse, error)` | 1585 | Section 5.9. |
| 45 | `(w *Wallet) Unlock(ctx, passphrase []byte, timeout <-chan time.Time) error` | 1712 | Section 5.10. |
| 46 | `(w *Wallet) SetAccountPassphrase(ctx, account uint32, passphrase []byte) error` | 1751 | `Update`; `manager.SetAccountPassphrase`. Unwrapped. Zero-length passphrase reverts the account to the wallet-wide passphrase. |
| 47 | `(w *Wallet) UnlockAccount(ctx, account uint32, passphrase []byte) error` | 1758 | `View`; `manager.UnlockAccount`. Unwrapped. |
| 48 | `(w *Wallet) LockAccount(ctx, account uint32) error` | 1766 | `View`; `manager.LockAccount`. Unwrapped. |
| 49 | `(w *Wallet) AccountUnlocked(ctx, account uint32) (bool, error)` | 1773 | `View`; `manager.AccountHasPassphrase`. If the account has no individual passphrase (or does not exist) the error is kind `Invalid`, message `account is not individually encrypted`, wrapped with op `wallet.AccountUnlocked`. Otherwise returns whether it is unlocked. |
| 50 | `(w *Wallet) AccountHasPassphrase(ctx, account uint32) (bool, error)` | 1791 | `View`; returns the `hasPassphrase` result of `manager.AccountHasPassphrase`. Wraps with op `wallet.AccountHasPassphrase` (only a transaction-begin failure can error). A nonexistent account yields `false, nil`. |
| 51 | `(w *Wallet) replacePassphraseTimeout(wasLocked bool, newTimeout <-chan time.Time)` | 1804 | Section 5.10. |
| 52 | `(w *Wallet) Lock()` | 1842 | Section 5.10. |
| 53 | `(w *Wallet) Locked() bool` | 1852 | `manager.IsLocked()`. |
| 54 | `(w *Wallet) Unlocked() bool` | 1857 | `!w.Locked()`. |
| 55 | `(w *Wallet) WatchingOnly() bool` | 1862 | `manager.WatchingOnly()`. |
| 56 | `(w *Wallet) ChangePrivatePassphrase(ctx, old, new []byte) error` | 1870 | Holds `passphraseUsedMu` for writing across an `Update` that calls `manager.ChangePassphrase(ns, old, new, true)`. Wraps with op `wallet.ChangePrivatePassphrase`. Lock state is unchanged by the call. |
| 57 | `(w *Wallet) ChangePublicPassphrase(ctx, old, new []byte) error` | 1885 | `Update`; `manager.ChangePassphrase(ns, old, new, false)`. Wraps with op `wallet.ChangePublicPassphrase`. Takes no wallet mutex. |
| 58 | `(w *Wallet) AccountBalance(ctx, account uint32, confirms int32) (Balances, error)` | 1911 | `View`; `txStore.AccountBalance(dbtx, confirms, account)`. Wraps with op **`wallet.CalculateAccountBalance`** (not the method name; section 10, Q14). An account with no credits yields a `Balances` with only `Account` set. |
| 59 | `(w *Wallet) AccountBalances(ctx, confirms int32) ([]Balances, error)` | 1927 | `View`; iterates `manager.ForEachAccount` and calls `txStore.AccountBalance` per account, appending in iteration order. The iteration is a bbolt `ForEach` over the account bucket whose keys are the account number as 4-byte **little-endian** (`wallet/udb/addressdb.go:269`, `:698`–`708`, `:934`), so the order is byte-lexicographic over the little-endian key, not numeric (section 10, Q15). The imported account (`0x7fffffff`) and imported-xpub accounts are included. Wraps with op `wallet.AccountBalances`. Result is nil (not empty) if there are no accounts. |
| 60 | `(w *Wallet) CurrentAddress(account uint32) (stdaddr.Address, error)` | 1952 | Section 5.11. |
| 61 | `(w *Wallet) SignHashes(ctx, hashes [][]byte, addr stdaddr.Address) ([][]byte, []byte, error)` | 1977 | Section 5.12. |
| 62 | `(w *Wallet) SignMessage(ctx, msg string, addr stdaddr.Address) (sig []byte, err error)` | 2008 | Section 5.12. |
| 63 | `VerifyMessage(msg string, addr stdaddr.Address, sig []byte, params stdaddr.AddressParams) (bool, error)` | 2036 | Section 5.12. Package-level function. |
| 64 | `(w *Wallet) HaveAddress(ctx, a stdaddr.Address) (bool, error)` | 2066 | `View`; `manager.Address`. Kind `NotExist` becomes `(false, nil)`; any other error is wrapped with op `wallet.HaveAddress`; success is `(true, nil)`. |
| 65 | `(w *Wallet) AccountNumber(ctx, accountName string) (uint32, error)` | 2083 | `View`; `manager.LookupAccount`. Wraps with op `wallet.AccountNumber`. Unknown name: kind `NotExist`, inner message `no account "<name>"` (`wallet/udb/addressdb.go:766`). |
| 66 | `(w *Wallet) AccountName(ctx, accountNumber uint32) (string, error)` | 2099 | `View`; `manager.AccountName`. Wraps with op `wallet.AccountName`. Unknown number: kind `NotExist`, inner message `no account <n>` (`wallet/udb/addressdb.go:750`). |
| 67 | `(w *Wallet) RenameAccount(ctx, account uint32, newName string) error` | 2115 | Section 5.13. |
| 68 | `(w *Wallet) NextAccount(ctx, name string) (uint32, error)` | 2139 | Section 5.13. |
| 69 | `(w *Wallet) AccountXpub(ctx, account uint32) (*hdkeychain.ExtendedKey, error)` | 2242 | `View`; `manager.AccountExtendedPubKey`. Wraps with op `wallet.AccountXpub`. |
| 70 | `(w *Wallet) AccountXpriv(ctx, account uint32) (*hdkeychain.ExtendedKey, error)` | 2260 | `View`; `manager.AccountExtendedPrivKey`. Wraps with op `wallet.AccountXpriv`. Kind `Locked` when the key is unavailable. |
| 71 | `(w *Wallet) TxBlock(ctx, hash *chainhash.Hash) (chainhash.Hash, int32, error)` | 2277 | Section 5.14. |
| 72 | `(w *Wallet) TxConfirms(ctx, hash *chainhash.Hash) (int32, error)` | 2300 | Section 5.14. No non-test callers. |
| 73 | `(w *Wallet) GetTransactionsByHashes(ctx, txHashes []*chainhash.Hash) (txs []*wire.MsgTx, notFound []*wire.InvVect, err error)` | 2319 | Section 5.14. |
| 74 | `(c CreditCategory) String() string` | 2361 | See 3.2. |
| 75 | `recvCategory(details *udb.TxDetails, syncHeight int32, chainParams *chaincfg.Params) CreditCategory` | 2380 | If the transaction is a coinbase under either the pre-treasury or post-treasury definition (`compat.IsEitherCoinBaseTx`, `internal/compat/compat.go:19`): `CreditGenerate` when `coinbaseMatured(params, details.Block.Height, syncHeight)` (i.e. `height >= 0 && syncHeight - height + 1 > CoinbaseMaturity`, `wallet/wallet.go:4422`), else `CreditImmature`. Otherwise `CreditReceive`. Only caller is `listTransactions` (part 2). |

### 3.7 Functions in `wallet/network.go`, `wallet/log.go`, `wallet/common.go`

| Declaration | Source | Contract |
|---|---|---|
| `(w *Wallet) NetworkBackend() (NetworkBackend, error)` | `wallet/network.go:61` | Reads `networkBackend` under `networkBackendMu`. Nil backend: error with op `wallet.NetworkBackend`, kind `NoPeers`, no inner error. |
| `(w *Wallet) SetNetworkBackend(n NetworkBackend)` | `wallet/network.go:75` | Stores under `networkBackendMu`. A nil argument clears the backend. |
| `UseLogger(logger slog.Logger)` | `wallet/log.go:18` | Replaces the package logger. Not synchronised. |
| `(b *BlockIdentity) None() bool` | `wallet/common.go:27` | See 3.2. |

### 3.8 Package `deployments`

| Declaration | Source | Contract |
|---|---|---|
| `(d *HardcodedDeployment) Active(height int32, net wire.CurrencyNet) bool` | `deployments/deployments.go:60` | Selects the activation height by network magic (mainnet, `0x48e7a065` testnet2, testnet3, simnet; any other network gives -1) and returns `activationHeight >= 0 && height >= activationHeight`. |
| `DCP0010Active(ctx, height int32, params *chaincfg.Params, querier Querier) (bool, error)` | `deployments/deployments.go:92` | Section 5.15. |
| `DCP0012Active(ctx, height int32, params *chaincfg.Params, querier Querier) (bool, error)` | `deployments/deployments.go:129` | Section 5.15. |

---

## 4. Data formats

This range defines no new on-disk structures. It reads and writes the following through `udb`; the
encodings are restated here because the functions in range determine what is written and how it is
interpreted. Bucket creation and migration are owned by the database-upgrade spec.

### 4.1 Agenda preferences

Source: `wallet/udb/agendas.go`.

| Item | Encoding |
|---|---|
| Default preferences bucket | Top-level bucket `agendaprefs` (`wallet/udb/agendas.go:19`). |
| Per-ticket preferences bucket | Top-level bucket `ticketsagendaprefs` (`:20`), containing one nested bucket per ticket whose name is the 32-byte ticket hash in internal byte order (the `chainhash.Hash` array as stored, not the reversed display form). The nested bucket is created on first write (`:40`). |
| Key | `version` as 4-byte **big-endian** uint32 (`byteOrder` is `binary.BigEndian`, `wallet/udb/txdb.go:123`) followed by the agenda ID as raw ASCII bytes, no terminator, no length prefix (`:23`–`28`). |
| Value | The choice ID as raw ASCII bytes (`:36`, `:44`). |
| Absent key | Read as the empty string, meaning "no preference saved" (`:47`–`60`). |

`version` is the wallet's compiled-in vote version for the network (section 5.2), so preferences
saved under one vote version are invisible once the software's vote version changes. Example key
for mainnet agenda `maxtreasuryspend` at vote version 11:
`00 00 00 0b 6d 61 78 74 72 65 61 73 75 72 79 73 70 65 6e 64`; value for choice `yes`: `79 65 73`.

### 4.2 Treasury policies

Source: `wallet/udb/treasury.go`.

| Bucket (top level) | Key | Value |
|---|---|---|
| `tspendpolicy` | 32-byte tspend transaction hash (internal byte order) | 1 byte policy |
| `treasurypolicy` | treasury key bytes exactly as supplied by the caller (33-byte compressed secp256k1 public key in practice; the wallet layer does not check the length) | 1 byte policy |
| `vsptspendpolicy` | 64 bytes: ticket hash (32) then tspend hash (32) | 1 byte policy |
| `vsptreasurypolicy` | ticket hash (32) then treasury key bytes | 1 byte policy |

Policy byte: `0x01` yes, `0x02` no (`stake/v5@v5.0.3/staketx.go:828`–`836`). `0x00` (invalid/abstain)
is never stored: setting it deletes the key (`wallet/udb/treasury.go:37`–`43` and the three
siblings). On read, a value whose length is not 1 is an error of kind `IO` with message
`invalid length <n> for tspend vote policy` or `invalid length <n> for treasury key policy`
(`:75`–`78`, `:193`–`196`); a stored `0x00` is skipped when building the in-memory maps
(`:117`, `:140`, `:233`, `:257`).

### 4.3 Vote bits (in memory, later serialised into vote transactions)

`stake.VoteBits` is `{Bits uint16; ExtendedBits []byte}` (`stake/v5@v5.0.3/staketx.go:207`). The
wallet builds it as:

- `Bits`: `0x0001` (previous-block-valid bit) OR the `Bits` of each selected agenda choice.
- `ExtendedBits`: exactly 4 bytes, the vote version as a **little-endian** uint32
  (`wallet/wallet.go:318`–`322`, `:348`–`352`).

Note the opposite endianness of the same version number in the database key (4.1, big-endian) and
in the extended vote bits (little-endian).

### 4.4 Signed-message preimage and signature encodings

Message hash (`wallet/wallet.go:2010`–`2013`, `:2040`–`2043`):

```
preimage = varint(23) || "Decred Signed Message:\n" || varint(len(msg)) || msg
hash     = BLAKE-256(preimage)            (chainhash.HashB, single hash)
```

`varint` is the wire compact-size integer (`WriteVarInt`, `wire@v1.8.0/common.go:809`; `WriteVarString` at `:936` writes it followed by the string bytes): one byte for values below
`0xfd`, `0xfd` + uint16 LE up to `0xffff`, `0xfe` + uint32 LE, `0xff` + uint64 LE. The prefix string
is 23 bytes, so the first preimage byte is `0x17`. `msg` is the raw bytes of the string (UTF-8 as
received; no normalisation).

| Output | Encoding |
|---|---|
| `SignMessage` signature | 65 bytes: `code || R (32, big-endian) || S (32, big-endian)` where `code = 27 + recovery_id + 4`; the `+4` (compressed-key flag) is always present (`secp256k1/v4@v4.4.1/ecdsa/signature.go:775`–`790`). The nonce is RFC 6979 deterministic and S is canonical (low). |
| `SignHashes` signatures | One DER-encoded ECDSA signature per input hash (`Signature.Serialize`), no sighash-type byte appended, same order as the input. Deterministic (RFC 6979). |
| `SignHashes` public key | 33-byte compressed secp256k1 public key of the address's key. |

### 4.5 TSpend signature script (read only)

`TSpendPolicy` extracts the treasury key from a cached tspend as
`TxIn[0].SignatureScript[66:99]` (`wallet/wallet.go:615`). The tspend script layout is
`OP_DATA_64 (0x40) || 64-byte Schnorr signature || OP_DATA_33 (0x21) || 33-byte compressed pubkey
|| OP_TSPEND`, 100 bytes total (`stake/v5@v5.0.3/treasury.go:17`–`20`, `:180`–`187`), so bytes 66
through 98 are the public key.

### 4.6 Structures handed to the network backend

- `LoadTxFilter` receives HD addresses in batches of at most 256 per call, imported addresses in
  batches of exactly 256 with one final partial batch, and outpoints in batches of exactly 64 with
  one final partial batch (section 5.5). In RPC mode each call becomes one dcrd `loadtxfilter`
  request, so batch sizes are visible to dcrd.
- Ticket outpoints are `{Hash: ticket hash, Index: 0, Tree: 1 (stake)}` (`wallet/wallet.go:1183`–`1187`).
- `CFiltersV2` is called with at most 2000 block hashes (section 5.8).
- `GetTransactionsByHashes` reports misses as inventory vectors of type `InvTypeTx` (value 1,
  `wire@v1.8.0/invvect.go:32`) (`wallet/wallet.go:2328`).

### 4.7 JSON/protobuf

None is produced in this range. `CreditCategory.String` values and `MissingCFilterProgress`
heights are forwarded verbatim by the RPC layers (`listtransactions` category strings; gRPC
`FetchMissingCFiltersNotification.fetched_cfilters_start_height`/`_end_height`,
`internal/rpc/rpcserver/server.go:2891`–`2899`).

---

## 5. Algorithms and state machines

Database terminology: `View(f)` begins one read-only transaction, runs `f`, and rolls back
(`wallet/walletdb/interface.go:203`–`223`). `Update(f)` begins the single read-write transaction,
runs `f`, commits if `f` returned nil, otherwise rolls back; a commit failure is returned
(`wallet/walletdb/interface.go:230`–`255`). Unless a step says otherwise, an error from a
transaction-begin call is returned through the same path as an error from `f`.

Error-string notation: `Sep` is the nested-error separator, `":: "` in the daemon
(`dcrwallet.go:38`) and `":\n\t"` when the package default is left in place
(`errors/errors.go:27`). Section 8 gives the formatting rules.

### 5.1 TSpend cache

`AddTSpend(tx)` (`wallet/wallet.go:259`–`270`):

1. Compute `hash = tx.TxHash()`.
2. Log at info: `TSpend arrived: <hash>` (before any duplicate check, so duplicates log too).
3. Lock `stakeSettingsLock`.
4. If `hash` is already a key of `tspends`, return the plain error `tspend already cached`
   (created with `fmt.Errorf`, kind `Other`, no op).
5. Otherwise store `tspends[hash] = tx` (struct copy; the `TxIn`/`TxOut` slices are shared with the
   caller's value) and return nil.

No validation of the transaction is performed. The SPV syncer validates before calling
(`spv/sync.go:1053`, `:1147`); the RPC syncer trusts dcrd (`chain/sync.go:702`–`704`, `:925`–`931`).
The RPC syncer's notification handler returns the duplicate error to its caller; the other call
sites ignore it.

`GetAllTSpends(ctx)` (`:274`–`289`):

1. `_, height = w.MainChainTip(ctx)` — its own `View`, before the lock is taken.
2. Lock `stakeSettingsLock`.
3. For every entry of `tspends`, in Go map iteration order (unspecified, differs between calls):
   - if `uint32(height) > tx.Expiry`, delete the entry and skip it;
   - otherwise append a pointer to a fresh copy of the stored `wire.MsgTx` value.
4. Return the slice (empty, non-nil, when nothing is cached).

The result order is unspecified. It feeds the order of treasury votes inside the wallet's vote
transactions (`wallet/chainntfns.go:806`–`823`) and the order of the JSON-RPC `tspendpolicy` and
gRPC `TSpendPolicies` listings (section 10, Q1).

### 5.2 Vote version, agendas, vote bits from the database

`voteVersion(params)` (`:291`–`304`) switches on `params.Net`:

| Network | Magic | Vote version |
|---|---|---|
| mainnet | `0xd9b400f9` | 11 |
| testnet2 (retired) | `0x48e7a065` | 6 |
| testnet3 | `0xb194aa75` | 12 |
| simnet | `0x12141c16` | 12 |
| anything else (including regnet `0xdab500fa`) | — | 1 |

`CurrentAgendas(params)` (`:308`–`314`) returns `(voteVersion(params), params.Deployments[version])`,
or `(version, nil)` when `params.Deployments` is nil or has no entry for that version. With
chaincfg v3.3.0 the result for every supported network is a single agenda:

| Network | Version | Agenda ID | Mask | Choices (ID → bits), in order |
|---|---|---|---|---|
| mainnet | 11 | `maxtreasuryspend` | `0x0006` | `abstain` → `0x0000`, `no` → `0x0002`, `yes` → `0x0004` |
| testnet3 | 12 | `maxtreasuryspend` | `0x0006` | same |
| simnet | 12 | `maxtreasuryspend` | `0x0006` | same |

(`chaincfg/v3@v3.3.0/mainnetparams.go:486`–`514`, `testnetparams.go:461`–`487`,
`simnetparams.go:495`–`522`.) The implementation must nevertheless
handle a list of agendas with distinct masks, as the code does.

`readDBVoteBits(dbtx)` (`:316`–`344`):

1. `version, deployments = CurrentAgendas(chainParams)`.
2. `vb = {Bits: 0x0001, ExtendedBits: LE32(version)}`.
3. If there are no deployments, return `vb`.
4. For each deployment in slice order: read `choiceID = udb.DefaultAgendaPreference(dbtx, version, d.Vote.Id)`.
   If empty, continue. Otherwise find the first choice of that agenda whose `Id` equals `choiceID`
   and OR its `Bits` into `vb.Bits`. A stored choice ID that matches no choice contributes nothing.
5. Return `vb`.

`readDBTicketVoteBits(dbtx, ticketHash)` (`:346`–`375`) is identical except that it reads
`udb.TicketAgendaPreference(dbtx, ticketHash, version, d.Vote.Id)` and additionally returns
`hasSavedPrefs`, true when at least one agenda returned a non-empty choice ID. With no deployments
it returns `(vb, false)`. Agendas with no per-ticket preference contribute no bits; they do **not**
fall back to the wallet-wide default for that agenda.

### 5.3 Agenda choices

`AgendaChoices(ctx, ticketHash)` (`:412`–`469`):

1. `version, deployments = CurrentAgendas(chainParams)`. If empty, return
   `(empty non-nil map, 0, nil)`.
2. Build `choices` with every agenda ID mapped to `"abstain"`. Set `voteBits = 1`.
3. One `View`:
   a. If `ticketHash != nil`: `ownTicket = txStore.OwnTicket(tx, ticketHash)` (presence of a
      ticket record). If false, return from the closure.
   b. For each deployment in order: read the default preference (`ticketHash == nil`) or the
      per-ticket preference. If empty, continue. Otherwise set `hasSavedPrefs = true`, set
      `choices[agenda.Id] = choice` (the stored string, even if it matches no choice), and OR the
      bits of the first matching choice into `voteBits`.
4. If the `View` failed: return `(nil, 0, errors.E("wallet.AgendaChoices", err))`.
5. If `ticketHash != nil` and not `ownTicket`: return `(nil, 0, err)` where `err` has kind
   `NotExist`, message `ticket not found`, **no op**. String: `item does not exist: ticket not found`.
6. If `ticketHash != nil` and no preference was saved for the ticket: return
   `w.AgendaChoices(ctx, nil)` (a second `View`), i.e. the wallet-wide defaults.
7. Return `(choices, voteBits, nil)`.

`SetAgendaChoices(ctx, ticketHash, choices)` (`:476`–`567`):

1. `version, deployments = CurrentAgendas(chainParams)`. If empty, return `(0, err)` where `err`
   is `errors.E("no agendas to set for this network")`: kind `Other`, no op, string
   `no agendas to set for this network`.
2. If `ticketHash != nil`: one `View` computing `ownTicket`. `View` failure → wrap with op
   `wallet.SetAgendaChoices`. Not owned → kind `NotExist`, message `ticket not found`, no op.
3. One `Update`. For each `(agendaID, choiceID)` of the input map, in Go map iteration order:
   a. Find the deployment whose `Vote.Id == agendaID`. None → kind `Invalid`, message
      `no agenda with ID "<agendaID>"` (Go `%q` quoting).
   b. Find the choice of that agenda whose `Id == choiceID`. None → kind `Invalid`, message
      `agenda "<agendaID>" has no choice ID "<choiceID>"`.
   c. Write the preference: `udb.SetDefaultAgendaPreference(tx, version, agendaID, choiceID)` when
      `ticketHash == nil`, else `udb.SetTicketAgendaPreference(tx, ticketHash, version, agendaID,
      choiceID)`. A write failure has kind `IO`.
   d. Append `{mask: agenda.Mask, bits: choice.Bits}` to `appliedChoices`.
   e. If `ticketHash != nil`: `voteBits = readDBTicketVoteBits(tx, ticketHash).Bits` (reads through
      the same write transaction, so it sees the value just written).
   Any error aborts the loop, rolls back every write of this call, and is returned as
   `(0, errors.E("wallet.SetAgendaChoices", err))`, e.g.
   `wallet.SetAgendaChoices: invalid operation: no agenda with ID "foo"`.
4. If `ticketHash == nil`: lock `stakeSettingsLock`; for each applied choice in order,
   `defaultVoteBits.Bits &^= mask` then `defaultVoteBits.Bits |= bits`; set
   `voteBits = defaultVoteBits.Bits`; unlock.
5. Return `(voteBits, nil)`.

Consequences: an empty `choices` map with a nil ticket returns the current default bits; an empty
map with a ticket returns 0. The database commit (step 3) and the cache update (step 4) are not
atomic with respect to `VoteBits()` readers. Per-ticket bits are never cached.

### 5.4 Treasury policies

All five functions hold `stakeSettingsLock` for their whole body, except that the two setters
validate the policy value before taking it.

`TreasuryKeyPolicy(pikey, ticket)` (`:572`–`584`): if `ticket != nil` return
`vspTSpendKeyPolicy[{*ticket, string(pikey)}]` (zero if absent; there is **no** fallback to the
wallet-wide key policy); otherwise return `tspendKeyPolicy[string(pikey)]`.

`TSpendPolicy(tspendHash, ticketHash)` (`:591`–`628`), first match wins:

1. If `ticketHash != nil` and `vspTSpendPolicy` has `{*ticketHash, *tspendHash}`: return it.
2. If `tspendPolicy` has `*tspendHash`: return it.
3. If `tspends` does not hold `*tspendHash`: return 0 (invalid/abstain).
4. `pikey = tspend.TxIn[0].SignatureScript[66:99]` (section 4.5). No length check; a cached
   transaction with a shorter script or no inputs panics.
5. If `ticketHash != nil` and `vspTSpendKeyPolicy` has `{*ticketHash, string(pikey)}`: return it.
6. Return `tspendKeyPolicy[string(pikey)]` (zero if absent).

`TreasuryKeyPolicies()` (`:640`–`661`): returns one `TreasuryKeyPolicy` per entry of
`tspendKeyPolicy` (`Ticket` nil) followed by one per entry of `vspTSpendKeyPolicy` (`Ticket` set to
a copy of the ticket hash). Each group is in Go map iteration order. The slice is empty and non-nil
when there are no policies.

`SetTreasuryKeyPolicy(ctx, pikey, policy, ticketHash)` (`:666`–`711`):

1. If `policy` is not `0x00`, `0x01` or `0x02`: return kind `Invalid` with message
   `invalid treasury vote policy 0x<hex>` (Go `%#x` of the byte, e.g. `0x3`). No op. String:
   `invalid operation: invalid treasury vote policy 0x3`.
2. Lock `stakeSettingsLock` (held until return, including across the database write).
3. One `Update`: `udb.SetVSPTreasuryKeyPolicy(dbtx, ticketHash, pikey, policy)` if
   `ticketHash != nil`, else `udb.SetTreasuryKeyPolicy(dbtx, pikey, policy)`. Policy `0x00` deletes
   the key. The error is returned **unwrapped** (no op).
4. Update the in-memory map: for policy `0x00` delete the entry, otherwise store it. The map is
   `vspTSpendKeyPolicy` keyed `{*ticketHash, string(pikey)}` when a ticket is given, else
   `tspendKeyPolicy` keyed `string(pikey)`.

`SetTSpendPolicy(ctx, tspendHash, policy, ticketHash)` (`:716`–`761`) is the same with
`udb.SetVSPTSpendPolicy`/`udb.SetTSpendPolicy` and maps `vspTSpendPolicy` (key
`{*ticketHash, *tspendHash}`) / `tspendPolicy` (key `*tspendHash`). Same error text for an invalid
policy.

Neither setter checks that the ticket is owned by the wallet, that the tspend is known, or that
`pikey` is a valid key.

### 5.5 Transaction-filter loading

#### `watchHDAddrs(ctx, firstWatch, n)` (`:891`–`1065`)

All index arithmetic below is unsigned 32-bit with wraparound, as in Go. `MAX` is `0xffffffff`,
the "none" sentinel for last-used and last-returned child indexes.

1. If `!firstWatch`: `rp, err = w.RescanPoint(ctx)` (its own `View`). On error return `(0, err)`
   (already wrapped with op `wallet.RescanPoint`). If `rp != nil` (the wallet has blocks whose
   transactions are not yet processed) return `(0, nil)` without watching anything.
2. One `View` collecting per-account data into a map keyed by account number:
   a. `lastAcct = manager.LastAccount(ns)`; `lastImportedAcct = manager.LastImportedAccount(dbtx)`.
   b. For `acct` in `0..=lastAcct`, then for `acct` in `0x80000000..=lastImportedAcct`
      (`udb.ImportedAddrAccount + 1` is `0x80000000`; the loop is empty when no xpub account was
      imported): read `props = manager.AccountProperties(ns, acct)` and record
      - `externalCount = min(props.LastReturnedExternalIndex + gapLimit, 0x7fffffff)`
      - `internalCount = min(props.LastReturnedInternalIndex + gapLimit, 0x7fffffff)`
      - `lastReturnedExternal/Internal`, `lastUsedExternal/Internal` from `props`.
      For a branch that never returned an address (`MAX`), the sum wraps to `gapLimit - 1`.
   Any error is returned unwrapped as `(0, err)`.
3. Lock `addressBuffersMu`. For every `(acct, ad)` in `addressBuffers` (map order; order has no
   observable effect) with `hd = hdAccounts[acct]` (zero value if absent):
   a. External branch: if `hd.lastUsedExternal + 1 > ad.albExternal.lastUsed + 1` (the database
      records a later used index than memory):
      - `ad.albExternal.cursor += ad.albExternal.lastUsed - hd.lastUsedExternal`
      - if `ad.albExternal.cursor > 0x7fffffff`, set it to 0
      - `ad.albExternal.lastUsed = hd.lastUsedExternal`
   b. Same for the internal branch.
   c. `hd.externalKey = ad.albExternal.branchXpub`; `hd.internalKey = ad.albInternal.branchXpub`.
   d. If `firstWatch`: `ad.albExternal.lastWatched = hd.externalCount` and
      `ad.albInternal.lastWatched = hd.internalCount` (set **before** the addresses are loaded);
      `hd.lastWatchedExternal/Internal` stay 0.
      Else: `hd.lastWatchedExternal = ad.albExternal.lastWatched` and likewise internal.
   e. Store `hd` back into the map.
   Unlock.
4. For every account in the map (map order), call `loadBranchAddrs(externalKey,
   lastWatchedExternal, externalCount)` then `loadBranchAddrs(internalKey, lastWatchedInternal,
   internalCount)`; the first error returns `(0, err)`.
5. `loadBranchAddrs(branchKey, start, end)`:
   a. If `start == 0 && watchLast != 0 && end - gapLimit > watchLast`:
      `start = end - gapLimit - watchLast`.
   b. Create an error group bound to `ctx` with a concurrency limit of the number of logical CPUs.
   c. For `s = start; s <= end; s += 256`: if the group context is done, return its error.
      Otherwise submit a task that derives `deriveChildAddress(branchKey, child, chainParams)` for
      `child` in `s .. min(end + 1, s + 256)` (exclusive upper bound; `end` itself is included),
      skipping children that fail with `hdkeychain.ErrInvalidChild`, failing on any other error;
      then adds the number of derived addresses to an atomic counter and calls
      `n.LoadTxFilter(groupCtx, false, addrs, nil)`.
   d. Wait for the group and return its first error.
   Batches of one branch run concurrently and complete in any order; batches of different branches
   and accounts do not overlap because each branch is waited for before the next starts.
6. Lock `addressBuffersMu`. For each account in the map: raise
   `addressBuffers[acct].albExternal.lastWatched` to `externalCount` if lower, likewise internal.
   Unlock.
7. Return the atomic counter (number of addresses passed to `LoadTxFilter`, counted even for
   batches whose `LoadTxFilter` call later fails) and nil.

Derived address: child public key of the branch xpub at the index, HASH160 of the compressed
public key, version-0 P2PKH secp256k1 address for the wallet's network
(`wallet/addresses.go:1023`–`1029`, `internal/compat/compat.go:10`–`14`).

Worked example of the cursor adjustment (pinned by `TestAccountIndexes`,
`wallet/addresses_test.go:219`): memory `{lastUsed: MAX, cursor: 20}`, database last used 10 →
`cursor = 20 + (MAX - 10) = 9` (mod 2^32), `lastUsed = 10`. Memory `{lastUsed: 30, cursor: 0}`,
database last used 31 → `cursor = 0 + (30 - 31) = MAX`, which exceeds `0x7fffffff`, so `cursor = 0`.

#### `LoadActiveDataFilters(ctx, n, reload)` (`:1101`–`1201`)

1. Log info: `Loading active addresses and unspent outputs...`
2. If `reload`: `n.LoadTxFilter(ctx, true, nil, nil)`; on error return it unwrapped.
3. Allocate an outpoint buffer of capacity 64 and register a deferred flush (step 9).
4. `hdAddrCount, err = w.watchHDAddrs(ctx, true, n)`; on error return it unwrapped.
5. Log info: `Registered for transaction notifications for <hdAddrCount> HD address(es)`
6. Imported addresses. One `View`: `manager.ForEachAccountAddress(ns, udb.ImportedAddrAccount, fn)`
   where `fn` appends `a.Address()` to a buffer of capacity 256 and, when the buffer is full, adds
   256 to `importedAddrCount`, calls `n.LoadTxFilter(ctx, false, abuf, nil)` **inside the read
   transaction**, and truncates the buffer for reuse. On error return it unwrapped. After the
   `View`, if the buffer is non-empty, add its length to the count and call `LoadTxFilter` once
   more; on error return it unwrapped. Address order is the database cursor order of the imported
   account's address index.
7. If `importedAddrCount > 0`, log info:
   `Registered for transaction notifications for <n> imported address(es)`
8. Lock `lockedOutpointMu` (unlock deferred). One `View`:
   a. `txStore.ForEachUnspentOutpoint(dbtx, watchOutPoint)` (`wallet/udb/txmined.go:2557`–`2627`),
      which yields, in this order:
      1. every key of the mined-unspent bucket in cursor (byte-lexicographic key) order, skipping
         outpoints that an unmined transaction spends;
      2. every key of the unmined-credits bucket in cursor order, skipping outpoints that an
         unmined transaction spends and skipping every output of a transaction recorded as
         **unpublished**.
      The outpoint's `Tree` is set from the credit's stored opcode tag: regular (0) when the tag is
      "non-stake", stake (1) otherwise. A malformed key or unspent value is returned as an error.
   b. `_, height = txStore.MainChainTip(dbtx)`;
      `tickets = txStore.UnspentTickets(dbtx, height, true)` (`wallet/udb/txmined.go:2656`–`2721`).
      With `includeImmature = true` the height argument has no effect. A ticket record is returned
      when output 0 of the ticket has a mined-unspent record **and** either no unmined transaction
      spends it, or the unmined spender is a vote (`stake.IsSSGen`) whose voted-on block hash is
      not the current tip hash. Tickets that are themselves unmined have no mined-unspent record
      and are not returned. A spender that fails to deserialise is kind `IO`. The order is the
      cursor order of the tickets bucket (ticket hash bytes).
   c. For each ticket hash call `watchOutPoint(&wire.OutPoint{Hash, Index: 0, Tree: 1})`.
   `watchOutPoint` appends to the 64-entry buffer and, when it is full, calls
   `n.LoadTxFilter(ctx, false, nil, buf)` inside the read transaction and truncates the buffer.
   An error is returned wrapped with op `wallet.LoadActiveDataFilters`.
   Then log info: `Registered for transaction notifications for all relevant outputs`
9. Deferred, in this order after the body returns: unlock `lockedOutpointMu`; then, if the outpoint
   buffer is non-empty and the function is returning nil, call `n.LoadTxFilter(ctx, false, nil,
   buf)` and return its error (unwrapped) if any. The final partial outpoint batch is therefore sent
   after the "all relevant outputs" log line and outside the outpoint lock.

Because step 8b only returns tickets whose output 0 has a mined-unspent record, every returned
ticket that is not spent by an unmined transaction was already yielded by step 8a with the same
hash, index 0 and stake tree, and is therefore sent twice; nothing deduplicates the two sources.
The only outpoints contributed by step 8c alone are tickets spent by an unmined vote on a block
other than the tip (stale votes). Output 0 of an unmined, published ticket reaches the filter only
through step 8a (unmined-credits pass), and only if it is recorded as a wallet credit.

### 5.6 Main-chain ranges and block locators

`RangeCFiltersV2(ctx, startBlock, endBlock, f)` (`:855`–`881`): one `View`. Resolve
`(start, end) = w.blockRange(dbtx, startBlock, endBlock)` (part 2, `wallet/wallet.go:3040`:
defaults `start = 0`, `end = -1`; a block identified by hash is resolved through its stored
header). Call `txStore.RangeBlocks(ns, start, end, fn)` (`wallet/udb/txquery.go:727`: negative
bounds become `0x7fffffff`; iterates forwards when `begin < end`, otherwise backwards). For each
block, load `txStore.CFilterV2(dbtx, &block.Hash)` and call `f(block.Hash, key, filter)`; a `true`
first result or a non-nil error stops the iteration. Any error is wrapped with op
`wallet.RangeCFiltersV2`. The filter passed to `f` owns a private copy of its bytes
(`wallet/udb/cfilters.go:23`–`25`).

`blockLocators(dbtx, sidechain)` (`:1497`–`1535`):

1. If `sidechain` is empty: `(hash, height) = txStore.MainChainTip(dbtx)`. Otherwise take the last
   node: `hash = *n.Hash`, `height = int32(n.Header.Height)`.
2. `firstHeight = height`. `locators = [&hash]`. `step = 1`.
3. While `height >= 0`:
   a. If `sidechain` is non-empty and `height - sidechain[0].Header.Height >= 0`, append
      `sidechain[height - sidechain[0].Header.Height].Hash`. Otherwise append the main-chain hash
      at `height` (`txStore.GetMainChainBlockHashForHeight`; missing → kind `NotExist`, message
      `no block at height <h> in main chain`).
   b. `height -= step`.
   c. If `len(locators) > 10`, `step *= 2`.
4. Return `(locators, firstHeight, nil)`.

Resulting sequence of heights for a tip at height `T`: `T, T, T-1, T-2, …, T-9, T-10, T-12, T-16,
T-24, T-40, T-72, …` — the tip appears **twice** (once from step 2, once from the first loop
iteration). Entries 2 through 12 of the list are the heights `T` down to `T-10` in single steps;
from there the gaps are 2, 4, 8, 16, … (the step doubles after each append once the list holds
more than 10 entries, and the doubled step applies from the following subtraction). The walk
stops when the height goes negative; the genesis block is included only if the sequence lands
on height 0. For `T = 0` the result is `[genesis, genesis]`. With a sidechain the first entries come
from the sidechain nodes (which must be consecutive heights starting at `sidechain[0]`) and the
rest from the main chain.

`BlockLocators` runs this in one `View` and wraps errors with op `wallet.BlockLocators`. The
locator list is sent unchanged in `getheaders` wire messages (SPV) and dcrd `getheaders` requests.

### 5.7 `CommittedTickets(ctx, tickets)` (`:1205`–`1285`)

One `View`. For each input hash, in order:

1. `tx = txStore.Tx(txmgrNs, hash)`. On error: log at debug the error text and skip.
2. If `!stake.IsSStx(tx)`: skip.
3. Scan the commitment outputs at odd indexes `i = 1, 3, 5, …`:
   a. `addr = stake.AddrFromSStxPkScrCommitment(script, chainParams)`; on error log at debug and
      stop scanning this ticket.
   b. If `addr` is not a P2PKH secp256k1 v0 address: log at trace
      `Skipping commitment at index <i>: address is not P2PKH` and continue.
   c. `amt = stake.AmountFromSStxPkScrCommitment(script)`; on error log at debug and stop scanning.
   d. If `amt > bestAmount` (initially 0): remember `addr` and `amt`. Ties keep the earlier output;
      zero-amount commitments are never selected.
4. If no address was selected: log at debug `No best address` and skip.
5. Take the address's 20-byte hash; if `!manager.ExistsHash160(addrmgrNs, hash160)`: log at debug
   `Not our address: hash160=<hex>` and skip.
6. Log at trace `Ticket purchase <hash>: best commitment address <addr> amount <amt>`, append the
   input hash pointer and the address to the results.

Returns the two parallel slices (both non-nil, possibly empty). A `View` failure is wrapped with
op `wallet.CommittedTickets` and both slices are nil.

### 5.8 Missing compact filters

`fetchMissingCFilters(ctx, n, progress)` (`:1291`–`1415`), with `span = 2000`:

1. One `View`: `missing = txStore.IsMissingMainChainCFilters(dbtx)` (true unless the root key
   recording completeness holds exactly one non-zero byte; the store writes `0x01`). If missing,
   `height = txStore.MissingCFiltersHeight(dbtx)` (first main-chain height without a stored
   filter; kind `NotExist` if none). Return any error unwrapped. If not missing, return nil.
2. Allocate storage for 2000 hashes.
3. Loop:
   a. If `ctx` is done, return `ctx.Err()`.
   b. One `View`:
      - Re-read `missing`; if false, leave the closure.
      - `hash = GetMainChainBlockHashForHeight(ns, height)` (error if no block at that height).
      - If a filter is already stored for `hash`: `height += 2000`, set `cont`, leave.
      - `hashes = GetMainChainBlockHashes(ns, &hash, true, storage)` — up to 2000 consecutive
        main-chain hashes starting at `hash` inclusive.
      - If `hashes` is empty: error with op `udb.GetMainChainBlockHashes`, kind `Bug`, message
        `expected over 0 results`. (A second `Bug` check, `unexpected slice reallocation`, guards a
        Go slice-aliasing assumption and has no equivalent in a port.)
   c. On error return it. If not missing, return nil. If `cont`, restart the loop.
   d. `filterData = n.CFiltersV2(ctx, hashes)`; on error return it.
   e. One `View`: for each returned element `i`, load the stored header of `hashes[i]` and call
      `validate.CFilterV2HeaderCommitment(net, header, filter, proofIndex, proof)`. That check
      passes unconditionally only on mainnet and testnet3 for headers below DCP0005 activation (on
      every other network, simnet included, the proof is checked at every height) (mainnet height 431488, testnet3
      height 323328; `validate/validate.go:19`–`26`, `:80`–`89`) and otherwise requires
      `VerifyInclusionProof(header.StakeRoot, filter.Hash(), proofIndex, proof)`; failure is op
      `validate.CFilterV2HeaderCommitment(<block hash>)`, kind `Consensus`, message
      `invalid header inclusion proof for cfilterv2`. On error return it.
   f. One `Update`: if a filter is already stored for the **last** hash of the batch, set `cont`
      and commit nothing new; otherwise `txStore.InsertMissingCFilters(dbtx, hashes, filters)`
      (which also marks the wallet complete when the batch ends at the main-chain tip,
      `wallet/udb/txmined.go:479`–`487`). On error return it. If `cont`, restart the loop.
   g. If `progress != nil`, send `{BlockHeightStart: height, BlockHeightEnd: height + 1999}`
      (blocking send).
   h. Log info: `Fetched cfilters for blocks <height>-<height+1999>`.

`height` is not advanced after a successful insert; the next iteration finds the filter present at
`height` (step b) and advances by 2000 then. The reported end height is always `height + 1999`
even when the final batch holds fewer blocks. If the backend returns fewer filters than hashes,
`InsertMissingCFilters` fails with kind `Invalid`, message `slices must have equal len`; if it
returns more, step e indexes past the hash list (Go panic).

`FetchMissingCFilters(ctx, n)` (`:1422`–`1431`): runs the above with a nil progress channel; an
error is wrapped with op `wallet.FetchMissingCFilters(<n formatted with %v>)`.

`FetchMissingCFiltersWithProgress(ctx, n, progress)` (`:1447`–`1457`): runs the above; on error
sends one `{Err: errors.E(op, err)}` with the same op format; always closes `progress` on return
(deferred, so the error value is sent before the close). The caller must keep receiving until the
channel is closed.

### 5.9 `PurchaseTickets(ctx, n, req)` (`:1585`–`1701`)

`op = "wallet.PurchaseTickets"`.

1. If `req.Mixing && !w.mixingEnabled`: return kind `Invalid`, message
   `wallet mixing support is disabled`, with op.
2. `ctx, cancel = n.WrapContext(ctx)`; `cancel` is deferred.
3. `resp, err = w.purchaseTickets(ctx, op, n, req)` (`wallet/createtx.go:1162`).
4. Return `(resp, err)` unless **all** of: `err != nil`, `errors.Is(err,
   errVSPFeeRequiresUTXOSplit)` (the sentinel `paying VSP fee requires UTXO split`,
   `wallet/createtx.go:1152`; `purchaseTickets` returns it from its VSP-fee credit reservation
   when the purchasable count has dropped below two and a separate fee UTXO is still needed,
   `wallet/createtx.go:1407`–`1420`; the exact trigger belongs to the transaction-creation spec),
   and `!req.DontSignTx`.
5. If `req.Mixing && req.MixedAccount == req.SourceAccount`: return kind `InsufficientBalance`
   with op (no UTXO split is attempted from the mixed account).
6. `feePercent = req.VSPClient.FeePercentage(ctx)` (VSP `vspinfo` request); error returned
   unwrapped.
7. `sdiff = w.NextStakeDifficulty(ctx)`; error returned as is (already carries op
   `wallet.NextStakeDifficulty`).
8. `_, height = w.MainChainTip(ctx)`.
9. `dcp0010Active = dcp0012Active = true`. If `n` implements `deployments.Querier` (RPC mode
   only), replace them with `deployments.DCP0010Active(ctx, height, chainParams, n)` and
   `deployments.DCP0012Active(...)`; errors returned unwrapped.
10. `relayFee = w.RelayFee()`;
    `vspFee = txrules.StakePoolTicketFee(sdiff, relayFee, height, feePercent, chainParams,
    dcp0010Active, dcp0012Active)`.
11. Build an authoring request: outputs empty, `account = changeAccount = req.SourceAccount`,
    `minconf = req.MinConf`, `randomizeChangeIdx = true`, `txFee = relayFee`.
12. `addr = w.NewInternalAddress(ctx, req.SourceAccount)`; error returned unwrapped. Add one output
    paying to that address's payment script and version.
13. `txsize = txsizes.EstimateSerializeSize([]int{txsizes.RedeemP2PKHInputSize}, outputs, 0)` and
    `txfee = txrules.FeeForSerializeSize(relayFee, txsize)`. `RedeemP2PKHInputSize` (166) is passed
    where the function expects a signature-script size, so for a 25-byte P2PKH output script
    `txsize = 12 + 2·1 + 1 + (53 + 1 + 166 + 4) + (8 + 2 + 1 + 25) = 275` bytes, and with the
    default relay fee of 10000 atoms/kB `txfee = 2750` atoms (section 10, Q9).
14. Set the output value to `vspFee + txfee`.
15. `w.authorTx(ctx, op, a)`, then `w.recordAuthoredTx(ctx, op, a)`, then
    `w.publishAndWatch(ctx, op, nil, a.atx.Tx, a.watch)` — the nil backend argument makes
    `publishAndWatch` use `w.NetworkBackend()` rather than `n` — then
    `a.changeSourceUpdates.UpdateDB(ctx, w, nil)` (a second write of the change-address index
    already written by `recordAuthoredTx`). Any error is returned.
16. Mutate the request: `req.MinConf = 0`, `req.Count = 1`. Let `index = 1` if the authored
    transaction's change index is 0, else `index = 0`. Set
    `req.extraSplitOutput = {OutPoint: {Hash: txhash, Index: index, Tree: 0}, PrevOut:
    *tx.TxOut[index]}`.
17. Return `w.purchaseTickets(ctx, op, n, req)`.

The retry buys exactly one ticket regardless of the requested count and leaves the caller's
request struct modified.

### 5.10 Passphrase lock/unlock state machine

State: the address manager's `locked` flag (and, when unlocked, a keyed hash of the passphrase in
memory), and `passphraseTimeoutCancel` (nil = no timeout armed).

#### `Unlock(ctx, passphrase, timeout)` (`:1712`–`1744`)

`op = "wallet.Unlock"`.

1. Read-lock `passphraseUsedMu`; `wasLocked = manager.IsLocked()`;
   `err = manager.UnlockedWithPassphrase(passphrase)`; read-unlock.
   `UnlockedWithPassphrase` (`wallet/udb/addressmanager.go:1485`) returns: kind `WatchingOnly`
   (message `watching wallets can not be unlocked`) for watching-only wallets; kind `Locked` when
   locked; kind `Passphrase` when unlocked and the passphrase does not match the in-memory hash
   (constant-time compare); nil when unlocked and matching.
2. Dispatch on `err`:
   - kind `WatchingOnly` → return `errors.E(op, err)`. String:
     `wallet.Unlock: watching only wallet: watching wallets can not be unlocked`.
   - kind `Passphrase` → call `w.Lock()`; if `!wasLocked` log info
     `The wallet has been locked due to an incorrect passphrase.`; return `errors.E(op, err)`.
     String: `wallet.Unlock: invalid passphrase`.
   - kind `Locked` → read-lock `passphraseUsedMu` (held until `Unlock` returns); one `View`
     calling `manager.Unlock(ns, passphrase)` (key derivation and decryption). On error return
     `errors.E(op, errors.Passphrase, err)` — every failure on this path is reported with kind
     `Passphrase`, whatever its cause. For a wrong passphrase the string is
     `wallet.Unlock: invalid passphrase` + `Sep` + `secretkey.DeriveKey`. On success continue.
   - nil → continue.
   - anything else → return `errors.E(op, err)`.
3. `w.replacePassphraseTimeout(wasLocked, timeout)`; return nil.

The dispatch is written as a Go tagless `switch` whose `default` clause appears third in the
source (`:1728`); Go evaluates `default` last regardless of position, so the effective order is
`WatchingOnly`, `Passphrase`, `Locked`, nil, then everything else. `wasLocked` is sampled once in
step 1 and is not re-read after the read lock is released (section 10, Q34).

A wrong passphrase against an **unlocked** wallet locks it. A wrong passphrase against a locked
wallet leaves it locked.

#### `replacePassphraseTimeout(wasLocked, newTimeout)` (`:1804`–`1839`)

Lock `passphraseTimeoutMu` for the whole body. `hadTimeout = passphraseTimeoutCancel != nil`.

1. If `!wasLocked && !hadTimeout && newTimeout != nil` (already unlocked without a time limit):
   start a task that receives one value from `newTimeout` and discards it. State unchanged; the
   new timeout is ignored.
2. Otherwise:
   a. `oldCancel = passphraseTimeoutCancel`.
   b. `newCancel` = a fresh capacity-1 channel if `newTimeout != nil`, else nil. Store it in
      `passphraseTimeoutCancel`.
   c. If `oldCancel != nil`, send one value on it (never blocks: capacity 1, at most one send per
      channel).
   d. If `newTimeout != nil`, start a task that waits for whichever comes first:
      - a value on `newTimeout` → `w.Lock()`, then log info
        `The wallet has been locked due to timeout.`
      - a value on `newCancel` → receive one value from `newTimeout` (waits for the caller's
        eventual send, so callers that send on the channel are never blocked) and exit.
3. Log, at info:
   - `The wallet has been unlocked without a time limit` when `(wasLocked || hadTimeout) &&
     newTimeout == nil`;
   - `The wallet has been temporarily unlocked` when `(wasLocked || !hadTimeout) &&
     newTimeout != nil`;
   - nothing otherwise.

Truth table:

| `wasLocked` | `hadTimeout` | `newTimeout` | Effect on timeout | Log line |
|---|---|---|---|---|
| true | false | nil | none armed | unlocked without a time limit |
| true | false | set | new timeout armed | temporarily unlocked |
| true | true | nil | old cancelled, none armed | unlocked without a time limit |
| true | true | set | old cancelled, new armed | temporarily unlocked |
| false | false | nil | unchanged (none) | — |
| false | false | set | **ignored**, drained in background | temporarily unlocked (misleading) |
| false | true | nil | old cancelled, none armed | unlocked without a time limit |
| false | true | set | old cancelled, new armed | — |

#### `Lock()` (`:1842`–`1849`)

Write-lock `passphraseUsedMu`, lock `passphraseTimeoutMu`, call `manager.Lock()` ignoring its
error (kind `WatchingOnly` on watching wallets, kind `Locked` when already locked), set
`passphraseTimeoutCancel = nil`, unlock both. The old cancel channel is **not** signalled: a
timeout task armed earlier stays alive and will call `Lock()` again when its timer fires
(section 10, Q5).

`manager.Lock()` on an unlocked, non-watching manager zeroes and drops the private key of **every**
account held in memory, including accounts unlocked individually with `UnlockAccount`, then zeroes
the private crypto and master keys and forgets the passphrase hash
(`wallet/udb/addressmanager.go:357`–`377`). When the manager is already locked it returns kind
`Locked` before doing any of that (`wallet/udb/addressmanager.go:1462`–`1465`), so calling `Lock()`
on a locked wallet does **not** relock accounts that were individually unlocked afterwards
(section 10, Q33).

#### Per-account passphrases

`SetAccountPassphrase`, `UnlockAccount`, `LockAccount`, `AccountUnlocked`, `AccountHasPassphrase`
are single-transaction wrappers (section 3.6 rows 46–50). They take none of the wallet's
passphrase mutexes and have no timeout handling. Manager error kinds that pass through:
`WatchingOnly` (watching wallet and an account number below `0x7fffffff`; messages
`cannot unlock watching wallet`, `cannot lock watching wallet`,
`cannot set passphrase for watching wallet`), `Crypto` with message
`account is not encrypted with a unique passphrase` (unlock or lock of an account without an
individual passphrase), `Passphrase` (wrong passphrase; no message), `Locked` with message
`account is already locked` (`wallet/udb/addressmanager.go:1590`–`1720`). `SetAccountPassphrase`
additionally produces `Passphrase` with message
`watching-only imported accounts must have a passprase` (sic; watching wallet, account number
`0x7fffffff` or above, empty passphrase) and `Locked` with message
`wallet must be unlocked to set a unique account passphrase` or
`account must be unlocked to set a unique account passphrase` (account private key not in memory;
"wallet" when the account has no individual passphrase, "account" when it has one)
(`wallet/udb/addressmanager.go:1690`–`1727`). The list is not exhaustive for the removal path
(`removeAccountPassphrase`, `wallet/udb/addressmanager.go:1791`), which belongs to the address
manager spec.

### 5.11 `CurrentAddress(account)` (`:1952`–`1973`)

No database access.

1. Lock `addressBuffersMu`.
2. Look up `addressBuffers[account]`; absent (including the imported account `0x7fffffff`) →
   op `wallet.CurrentAddress`, kind `NotExist`, message `no account <n>`.
3. `childIndex = albExternal.lastUsed + 1 + albExternal.cursor` (uint32 wraparound).
4. Derive the external-branch child at `childIndex` and its P2PKH address; either failure is
   wrapped with op.

`cursor` is post-incremented each time `NewExternalAddress` hands out an address
(`wallet/addresses.go:613`–`647`), so this index is the address the **next** `NewExternalAddress`
call would return, not the one most recently returned; the address is not recorded as returned,
no gap policy is applied, and an invalid child is an error rather than skipped (section 10, Q6).

### 5.12 Signing and verification

`SignHashes(ctx, hashes, addr)` (`:1977`–`2004`):

1. One `View`: `privKey, done = manager.PrivateKey(ns, addr)`. Errors are returned **unwrapped**
   (kind `Locked` when the wallet or account is locked, `NotExist` for unknown addresses,
   `Invalid` with message `no private key for P2SH address` for script addresses).
2. For each hash in order: `ecdsa.Sign(privKey, hash)` → DER serialisation. The hash is used as
   given; inputs that are not 32 bytes are accepted: both the RFC 6979 nonce derivation and the
   message scalar use the first 32 bytes of a longer input and left-pad a shorter one with zeros
   (`secp256k1/v4@v4.4.1/nonce.go:117`–`141`, `modnscalar.go:360`–`368`).
3. Return the signatures and the 33-byte compressed public key. `done()` (zeroes the key) runs on
   return.

`SignMessage(ctx, msg, addr)` (`:2008`–`2032`): compute the message hash (section 4.4); fetch the
private key as above (errors wrapped with op `wallet.SignMessage`); return
`ecdsa.SignCompact(privKey, hash, true)`.

`VerifyMessage(msg, addr, sig, params)` (`:2036`–`2063`):

1. Compute the message hash.
2. `pk, wasCompressed, err = ecdsa.RecoverCompact(sig, hash)`. Errors (wrong length — must be 65
   bytes — invalid recovery code, R or S out of range, point not on curve) are wrapped with op
   `wallet.VerifyMessage`.
3. `pkHash = HASH160(compressed pk)` if `wasCompressed`, else `HASH160(uncompressed 65-byte pk)`.
4. Build the v0 P2PKH secp256k1 address for `params`; error wrapped with op.
5. Return `address.String() == addr.String()`.

Any `addr` type is accepted; only its string form is compared, so a pay-to-pubkey or script
address can never verify. The function reports `(false, nil)` for a well-formed signature by a
different key and `(false, err)` for a malformed signature.

### 5.13 Accounts

`RenameAccount(ctx, account, newName)` (`:2115`–`2132`): one `Update` calling
`manager.RenameAccount(ns, account, newName)` then `manager.AccountProperties(ns, account)`; error
wrapped with op `wallet.RenameAccount`. Manager errors: kind `Invalid` `reserved account` for the
imported account; kind `Exist` `account named "<name>" already exists`; kind `Invalid`
`accounts may not be named the empty string` / `reserved account name`
(`wallet/udb/addressmanager.go:2069`–`2077`, `:2343`–`2360`). The duplicate-name check precedes the
name-validity check. After commit, `NtfnServer.notifyAccountProperties(props)`.

`NextAccount(ctx, name)` (`:2139`–`2239`), `op = "wallet.NextAccount"`:

1. `maxEmptyAccounts = uint32(accountGapLimit)`.
2. One `Update`:
   a. `lastAcct = manager.LastAccount(ns)`.
   b. `canCreate = false`. For `i` in `0 .. maxEmptyAccounts-1`: `a = lastAcct - i`. If
      `a == 0 && i < maxEmptyAccounts - 1`, set `canCreate` and stop (fewer accounts exist than the
      limit). Otherwise read the properties of account `a`; if its last-used external or internal
      index is not `MAX`, set `canCreate` and stop.
   c. If `!canCreate`: plain error `last <maxEmptyAccounts> accounts have no transaction history`.
   d. `account = manager.NewAccount(ns, name)` — requires an unlocked, non-watching wallet (kinds
      `Locked`, `WatchingOnly`), a valid unused name (kinds `Invalid`, `Exist`).
   e. `props = manager.AccountProperties(ns, account)`; `xpub = manager.AccountExtendedPubKey(tx,
      account)`.
   f. `manager.SyncAccountToAddrIndex(ns, account, gapLimit, ExternalBranch)` then the same for
      `InternalBranch`: records address rows for children `0..=gapLimit` (that is `gapLimit + 1`
      addresses per branch) without changing last-used or last-returned.
   Error → `(0, errors.E(op, err))`, e.g.
   `wallet.NextAccount: last 10 accounts have no transaction history`.
3. Derive the external (child 0) and internal (child 1) branch keys from `xpub`; error wrapped.
4. Under `addressBuffersMu`, add `addressBuffers[account] = {xpub, albExternal: {branch key,
   lastUsed: MAX}, albInternal: {branch key, lastUsed: MAX}}` (cursor 0, lastWatched 0).
5. If a network backend is set (`w.NetworkBackend()` succeeds): start two concurrent tasks, one
   per branch, each deriving `deriveChildAddresses(branchKey, 0, gapLimit, chainParams)` (children
   `0..gapLimit-1`, invalid children skipped) and calling `n.LoadTxFilter(ctx, false, addrs, nil)`.
   Collect both results; on the first error received return `(0, errors.E(op, err))`. At that point
   the account already exists in the database and in memory, and no notification has been sent.
6. `NtfnServer.notifyAccountProperties(props)`; return `(account, nil)`.

With `accountGapLimit = 0` the loop body never runs and creation always fails with
`last 0 accounts have no transaction history`.

### 5.14 Transaction lookups

`TxBlock(ctx, hash)` (`:2277`–`2297`): one `View`. `height = txStore.TxBlockHeight(dbtx, hash)`
(-1 for an unmined transaction; kind `NotExist`, message `no transaction <hash>` if unknown). If
`height == -1` return `(zero hash, -1, nil)`. Otherwise return the main-chain block hash at that
height. Errors unwrapped, with height 0.

`TxConfirms(ctx, hash)` (`:2300`–`2312`): one `View` reading the tip height and the transaction's
block height; returns `confirms(txHeight, tip)` = 0 when `txHeight == -1` or `txHeight > tip`, else
`tip - txHeight + 1` (`wallet/wallet.go:4411`). Errors unwrapped.

`GetTransactionsByHashes(ctx, txHashes)` (`:2319`–`2342`): one `View`. For each hash in order:
`tx, err = txStore.Tx(ns, hash)`; an error other than kind `NotExist` aborts and is returned
unwrapped; a nil transaction appends `InvVect{Type: InvTypeTx, Hash}` to `notFound`; otherwise the
transaction is appended to `txs`. After the `View`, if any hash was not found the function returns
the found transactions, the not-found list, **and** an error of kind `NotExist` with message
`transaction(s) not found` (no op). Callers must treat that error as partial success.

### 5.15 Deployment tests

`HardcodedDeployment.Active(height, net)`: section 3.8.

`DCP0010Active(ctx, height, params, querier)` (`deployments/deployments.go:92`–`123`):

1. `rcai = int32(params.RuleChangeActivationInterval)`.
2. If the network is simnet: return `(true, nil)`.
3. If the network is neither mainnet nor testnet3: return `(false, nil)`.
4. If `querier == nil`: return kind `Bug`, message `DCP0010 activation check requires RPC syncer`.
5. `deployments = querier.Deployments(ctx)`; error returned unwrapped.
6. `d = deployments["changesubsidysplit"]`; absent → `(false, nil)`.
7. Return true if `d.Status == "lockedin" && height == int32(d.Since) + rcai - 1`, or if
   `d.Status == "active"`; otherwise false.

`DCP0012Active` (`:129`–`157`) is the same with these differences: step 2 is absent; step 3
returns `(false, nil)` unless the network is mainnet, testnet3 **or simnet**; the `Bug` message is
`DCP0012 activation check requires RPC syncer`; the agenda ID is `changesubsidysplitr2`.

`height` is the current tip height; the result answers "do the rules for the **next** block
require the new subsidy split". The locked-in clause makes the answer true for exactly the last
block of the lock-in interval, whose child is the first block under the new rules.

---

## 6. Constants, limits and defaults

| Name | Value | Source | What depends on it |
|---|---|---|---|
| `InsecurePubPassphrase` | `"public"` | `wallet/wallet.go:66` | Opening every wallet created without an explicit public passphrase. |
| `SimulationPassphrase` | `"password"` | `wallet/wallet.go:72` | Automatic unlock of simnet wallets at startup. |
| Namespace bucket names | `"waddrmgr"`, `"wtxmgr"` | `wallet/wallet.go:77`–`78` | Every database access in the package. |
| `scriptVersionAssumed` | 0 | `wallet/wallet.go:83` | Script-version assumptions elsewhere in the package. |
| Vote version, mainnet | 11 | `wallet/wallet.go:294` | Agenda set, preference keys, extended vote bits. |
| Vote version, testnet2 | 6 | `wallet/wallet.go:296` | Retired network. |
| Vote version, testnet3 | 12 | `wallet/wallet.go:298` | As mainnet. |
| Vote version, simnet | 12 | `wallet/wallet.go:300` | As mainnet. |
| Vote version, other networks | 1 | `wallet/wallet.go:302` | No agendas on regnet or unknown networks. |
| Base vote bits | `0x0001` | `wallet/wallet.go:319`, `:349`, `:427` | Previous-block-valid bit always set by this layer. |
| Extended vote bits length | 4 bytes (LE vote version) | `wallet/wallet.go:320`–`322` | Vote transaction contents. |
| Default agenda choice label | `"abstain"` | `wallet/wallet.go:421` | `getvotechoices` output for unset agendas. Hardcoded string, not taken from chaincfg. |
| TSpend pubkey slice | bytes `[66, 99)` of input 0 signature script | `wallet/wallet.go:615` | Treasury key policy lookup. |
| Treasury vote values | invalid `0x00`, yes `0x01`, no `0x02` | `stake/v5@v5.0.3/staketx.go:828`–`836` | Policy storage and validation. |
| HD address batch | 256 addresses per `LoadTxFilter` | `wallet/wallet.go:1013` | Size of dcrd `loadtxfilter` requests. |
| HD derivation concurrency | number of logical CPUs | `wallet/wallet.go:1012` | Parallel `LoadTxFilter` calls. |
| HD child upper bound | `0x7fffffff` (`HardenedKeyStart - 1`) | `wallet/wallet.go:931`–`932` | Clamp of the watch range. |
| Imported address batch | 256 | `wallet/wallet.go:1138` | As above. |
| Outpoint batch | 64 | `wallet/wallet.go:1112` | As above. |
| Ticket outpoint | index 0, tree 1 | `wallet/wallet.go:1183`–`1187` | Filter contents. |
| Missing-cfilter span | 2000 blocks per request | `wallet/wallet.go:1310` | `CFiltersV2` request size; progress heights. |
| DCP0005 activation heights | mainnet 431488, testnet3 323328 | `validate/validate.go:19`–`26` | Whether a fetched cfilter's inclusion proof is checked. |
| Locator single-step count | list length must exceed 10 before steps double | `wallet/wallet.go:1529` | `getheaders` locator contents. |
| Locator capacity hint | `10 + log2(height)` | `wallet/wallet.go:1510` | Allocation only. |
| Split-tx input size constant | `RedeemP2PKHInputSize` = 166 | `wallet/txsizes/size.go:55` | Value of the VSP-fee split output (section 5.9). |
| Unlock cancel channel capacity | 1 | `wallet/wallet.go:1814` | Non-blocking cancellation. |
| Signed-message prefix | `"Decred Signed Message:\n"` (23 bytes) | `wallet/wallet.go:2011`, `:2041` | Interoperable message signatures. |
| Compact signature | 65 bytes, header `27 + recid + 4` | `secp256k1/v4@v4.4.1/ecdsa/signature.go:741`–`752` | `signmessage`/`verifymessage`. |
| Unused-index sentinel | `0xffffffff` | `wallet/wallet.go:2166`, `:2210`–`2211` | Account-gap test; fresh address buffers. |
| Imported account number | `0x7fffffff` | `wallet/udb/addressmanager.go:47` | Imported-address filter load; first imported-xpub account is `0x80000000`. |
| Branch numbers | external 0, internal 1 | `wallet/udb/addressmanager.go:80`, `:85` | Branch derivation. |
| `CreditCategory` values | 0 receive, 1 generate, 2 immature | `wallet/wallet.go:2352`–`2356` | RPC category strings. |
| `DCP0001` heights | mainnet 149248, testnet2 46128, testnet3 0, simnet 0 | `deployments/deployments.go:32`–`37` | Stake-difficulty algorithm selection; `NextStakeDifficulty` availability. |
| `DCP0002` heights | mainnet 189568, testnet2 151968, testnet3 0, simnet 0 | `deployments/deployments.go:41`–`46` | Unused. |
| `DCP0003` heights | mainnet 189568, testnet2 151968, testnet3 0, simnet 0 | `deployments/deployments.go:50`–`55` | Unused. |
| Testnet2 magic | `0x48e7a065` | `wallet/wallet.go:295`, `deployments/deployments.go:65` | Literal; not a `wire` constant in v1.8.0. |
| DCP0010 agenda ID | `changesubsidysplit` | `chaincfg/v3@v3.3.0/params.go:166` | Subsidy split selection. |
| DCP0012 agenda ID | `changesubsidysplitr2` | `chaincfg/v3@v3.3.0/params.go:175` | Subsidy split selection. |
| Agenda status strings | `lockedin`, `active` | `rpc/jsonrpc/types/v4@v4.4.0/chainsvrresults.go:100`–`101` | Parsing dcrd `getblockchaininfo`. |
| Offline error text | `operation not supported in offline mode` | `wallet/network.go:92` | Error surfaced by RPCs in `--offline` mode. |

Defaults of the options that populate `Config` (declared outside this range; listed because the
behaviours above depend on them):

| Option | Default | Source |
|---|---|---|
| `--gaplimit` | 20 | `wallet/addresses.go:385`, `config.go:57` |
| `--watchlast` | 0 (watch everything) | `config.go:112` (no default assigned) |
| `--accountgaplimit` | 10 | `wallet/addresses.go:389`, `config.go:59` |
| `--txfee` (relay fee) | 0.0001 DCR/kB (10000 atoms) | `wallet/txrules/rules.go:38`, `config.go:373` |
| `--mixsplitlimit` | 10 | `config.go:62` |
| VSP max fee | 0.2 DCR | `config.go:63` |
| allow high fees | false | `config.go:58` |
| disable coin-type upgrades | false | `config.go:60` |

---

## 7. Concurrency and lifecycle

### 7.1 Mutexes and what they protect

| Mutex | Protects | Held across I/O? |
|---|---|---|
| `stakeSettingsLock` | `defaultVoteBits`, `votingEnabled`, `tspends`, the four policy maps | Yes, across the database `Update` in `SetTreasuryKeyPolicy`/`SetTSpendPolicy`. Not held during database access in `SetAgendaChoices`, `AgendaChoices`, `GetAllTSpends`. |
| `networkBackendMu` | `networkBackend` | No. |
| `relayFeeMu` | `relayFee` | No. |
| `addressBuffersMu` | `addressBuffers` and the buffers' cursors | Not across database transactions or network calls in this range. |
| `lockedOutpointMu` | `lockedOutpoints`; also taken by transaction authoring | Yes: `LoadActiveDataFilters` holds it across a `View` that makes `LoadTxFilter` network calls. |
| `passphraseUsedMu` (RW) | ordering of `Unlock` (reader) against `Lock` and `ChangePrivatePassphrase` (writers) | Yes: read-held across key derivation in `Unlock`; write-held across the `Update` in `ChangePrivatePassphrase`. |
| `passphraseTimeoutMu` | `passphraseTimeoutCancel` | No. |
| `disapprovePercent`, `mixClient` | atomics | — |

Lock orders that occur in this range:

1. `passphraseUsedMu` → `passphraseTimeoutMu` (`Lock`; `Unlock` on the locked path calls
   `replacePassphraseTimeout` while still read-holding `passphraseUsedMu`).
2. `passphraseUsedMu` → database transaction → manager mutex (`Unlock`, `ChangePrivatePassphrase`).
3. `stakeSettingsLock` → database write transaction (treasury policy setters). No function in this
   range takes `stakeSettingsLock` while inside a database transaction.
4. `lockedOutpointMu` → database read transaction (`LoadActiveDataFilters`).
5. `addressBuffersMu` is always a leaf here. `wallet/addresses.go:515`–`520` documents the
   package-wide rule that a database transaction is opened **before** `addressBuffersMu` is taken.

`Unlock` releases its first read lock before deciding what to do, so two concurrent `Unlock`
calls, or an `Unlock` racing a `Lock`, may interleave between the check and the action; the code
accepts this.

### 7.2 Background tasks

| Started by | Lifetime | Purpose |
|---|---|---|
| `replacePassphraseTimeout` (timeout armed) | until the timeout channel yields a value, or until cancelled **and** the timeout channel yields a value | Lock the wallet on timeout. |
| `replacePassphraseTimeout` (timeout ignored) | until the timeout channel yields a value | Drain the caller's channel. |
| `watchHDAddrs` | bounded by the call; error group limited to the CPU count; first error cancels the group context | Parallel address derivation and filter loading. |
| `NextAccount` | two tasks; results collected through a capacity-2 channel so neither blocks after an early error return | Load the new account's addresses into the filter. |
| `FetchMissingCFiltersWithProgress` | run by the caller in its own goroutine (`chain/sync.go:659`); closes `progress` on exit | Progress reporting. |

Every timeout task waits for one value from the caller's channel before exiting, including after
cancellation. Callers therefore must eventually deliver a value (`time.After` does; gRPC handlers
send one in a deferred statement, e.g. `internal/rpc/rpcserver/server.go:3737`–`3741`); a channel
that never fires leaks one task.

### 7.3 Context cancellation

- `walletdb.View`/`Update` use the context only for tracing; a cancelled context does not abort a
  transaction.
- `watchHDAddrs` checks the group context before submitting each batch and passes it to
  `LoadTxFilter`.
- `fetchMissingCFilters` checks `ctx` at the top of every loop iteration and passes it to
  `CFiltersV2`.
- `PurchaseTickets` derives its context from `n.WrapContext`, so a backend disconnect cancels the
  purchase (including the VSP request and any mixing it waits on).
- `NextAccount` passes `ctx` to `LoadTxFilter` only.

### 7.4 Ordering guarantees observable by callers

- After `SetAgendaChoices(nil, …)` returns, `VoteBits()` reflects the change. Between the database
  commit and the cache update a concurrent reader can observe the old bits.
- After `SetTreasuryKeyPolicy`/`SetTSpendPolicy` returns nil, both database and cache are updated;
  concurrent policy readers block for the duration of the database write.
- `Unlock` returning nil means the manager is unlocked at that instant; a concurrent `Lock` or a
  timeout (including a stale one, section 10, Q5) may relock it immediately.
- `LoadActiveDataFilters` logs "all relevant outputs" before the last partial outpoint batch is
  sent.
- `FetchMissingCFiltersWithProgress` delivers progress values in ascending height order, then at
  most one error value, then closes the channel.
- `NextAccount` notifies account-properties subscribers only after the filter load succeeded.
  `RenameAccount` notifies after commit.

### 7.5 Lifecycle

Fields are populated by `Open` (part 3). `SetNetworkBackend` is called by the syncers when they
start and with nil when they stop (sync specs), and with `OfflineNetworkBackend{}` in offline mode.
There is no `Close` on `Wallet` in this range; the database handle is closed by the loader. The
tspend cache, relay fee changes, disapprove percentage and locked outpoints live only in memory
and are lost at shutdown.

---

## 8. Errors

### 8.1 The error model

All errors are either plain Go errors or `*errors.Error{Op, Kind, Err}` (`errors/errors.go:32`).
Kinds, their numeric order and their display strings (`errors/errors.go:53`–`124`):

| # | Kind | String |
|---|---|---|
| 0 | `Other` | `unclassified error` (never printed unless the whole error is empty) |
| 1 | `Bug` | `internal wallet error` |
| 2 | `Invalid` | `invalid operation` |
| 3 | `Permission` | `permission denied` |
| 4 | `IO` | `I/O error` |
| 5 | `Exist` | `item already exists` |
| 6 | `NotExist` | `item does not exist` |
| 7 | `Encoding` | `invalid encoding` |
| 8 | `Crypto` | `encryption/decryption error` |
| 9 | `Locked` | `wallet locked` |
| 10 | `Passphrase` | `invalid passphrase` |
| 11 | `Seed` | `invalid seed` |
| 12 | `WatchingOnly` | `watching only wallet` |
| 13 | `InsufficientBalance` | `insufficient balance` |
| 14 | `ScriptFailure` | `transaction script fails to execute` |
| 15 | `Policy` | `policy violation` |
| 16 | `Consensus` | `consensus violation` |
| 17 | `DoubleSpend` | `double spend` |
| 18 | `Protocol` | `protocol violation` |
| 19 | `NoPeers` | `Decred network is unreachable` |
| 20 | `Deployment` | `inactive deployment` |

Construction (`errors.E`, `errors/errors.go:187`–`236`): arguments are scanned in order; an `Op`
sets the op, a `Kind` sets the kind, a string or error sets the inner error (last one wins). When
the inner error is itself an `*Error`, its kind is inherited if none was given, its op is
inherited if none was given, and if it then has no op or kind that differs from the outer error
it is removed from the chain (its inner error is adopted directly).

Formatting (`errors/errors.go:247`–`290`): walk the chain; for each level print the op if non-empty
and different from the last printed op, then the kind string if the kind is non-zero and
different from the last printed kind, preceded by `": "` only when the op was printed at this same level; if the inner error is another `*Error`, print `Sep` (only if
something was printed at this level) and continue; otherwise print `": "` (same condition) and the
inner error's text. An entirely empty error prints `unclassified error`.

The full message string is what RPC clients receive, so op names, kind strings, separator and
inner texts are all externally visible.

### 8.2 Mapping to RPC results (performed by the RPC layers, restated for context)

JSON-RPC (`internal/rpc/jsonrpc/errors.go:16`–`49`): the first kind found in the chain selects the
code — `Bug` → -32603, `Encoding` → -8, `Locked` → -13, `Passphrase` → -14, `NoPeers` → -9,
`InsufficientBalance` → -6, everything else (and plain errors) → -4 — and the message is the full
error string. An error that already is a `*dcrjson.RPCError` is returned unchanged, and a
`*wsrpc.Error` (an error relayed from dcrd) keeps its own code and message; neither goes through the
kind mapping.

gRPC (`internal/rpc/rpcserver/server.go:80`–`136`): `Invalid`, `Passphrase`, `Seed` →
`InvalidArgument`; `Permission` → `PermissionDenied`; `Exist` → `AlreadyExists`; `NotExist` →
`NotFound`; `Crypto` → `DataLoss`; `Locked` → `FailedPrecondition`; `WatchingOnly` →
`Unimplemented`; `InsufficientBalance` → `ResourceExhausted`; `NoPeers` → `Unavailable`. Every other
kind (`Bug`, `IO`, `Encoding`, `ScriptFailure`, `Policy`, `Consensus`, `DoubleSpend`, `Protocol`,
`Deployment`) and plain errors fall through to these checks, in order: wraps
`hdkeychain.ErrInvalidSeedLen` → `InvalidArgument`; wraps `context.Canceled` → `Canceled`; carries a
gRPC status whose code is neither `OK` nor `Unknown` → that code; otherwise `Unknown`. The status
message is the full error string.

### 8.3 Errors originated in this range

| Function | Op | Kind | Inner message |
|---|---|---|---|
| `AddTSpend` | — | Other | `tspend already cached` |
| `FetchOutput` (originated by `udb.Store.Tx`) | `wallet.FetchOutput` | NotExist | `tx <hash> not found` |
| `Unlock` (locked path) | `wallet.Unlock` | Passphrase (forced over the callee's kind) | callee error from `manager.Unlock` |
| `AgendaChoices` | — | NotExist | `ticket not found` |
| `SetAgendaChoices` | — | Other | `no agendas to set for this network` |
| `SetAgendaChoices` | — | NotExist | `ticket not found` |
| `SetAgendaChoices` | `wallet.SetAgendaChoices` | Invalid | `no agenda with ID "<id>"` |
| `SetAgendaChoices` | `wallet.SetAgendaChoices` | Invalid | `agenda "<id>" has no choice ID "<choice>"` |
| `SetTreasuryKeyPolicy`, `SetTSpendPolicy` | — | Invalid | `invalid treasury vote policy 0x<hex>` |
| `fetchMissingCFilters` | `udb.GetMainChainBlockHashes` | Bug | `expected over 0 results` |
| `fetchMissingCFilters` | `udb.GetMainChainBlockHashes` | Bug | `unexpected slice reallocation` |
| `PurchaseTickets` | `wallet.PurchaseTickets` | Invalid | `wallet mixing support is disabled` |
| `PurchaseTickets` | `wallet.PurchaseTickets` | InsufficientBalance | (none) |
| `AccountUnlocked` | `wallet.AccountUnlocked` | Invalid | `account is not individually encrypted` |
| `CurrentAddress` | `wallet.CurrentAddress` | NotExist | `no account <n>` |
| `NextAccount` | `wallet.NextAccount` | Other | `last <n> accounts have no transaction history` |
| `GetTransactionsByHashes` | — | NotExist | `transaction(s) not found` |
| `NetworkBackend` | `wallet.NetworkBackend` | NoPeers | (none) → string `wallet.NetworkBackend: Decred network is unreachable` |
| `OfflineNetworkBackend.*` | — | Other | `operation not supported in offline mode` |
| `DCP0010Active` | — | Bug | `DCP0010 activation check requires RPC syncer` |
| `DCP0012Active` | — | Bug | `DCP0012 activation check requires RPC syncer` |

### 8.4 Wrapping behaviour per function

Wraps callee errors with its own op: `FetchOutput`, `AgendaChoices` (database failure only),
`SetAgendaChoices` (ownership `View` and the `Update`), `BlockInMainChain`, `RangeCFiltersV2`,
`CoinType`, `CoinTypePrivKey`, `LoadActiveDataFilters` (outpoint `View` only),
`CommittedTickets`, `FetchMissingCFilters`/`…WithProgress` (formatted op), `BlockLocators`,
`Unlock`, `AccountUnlocked`, `AccountHasPassphrase`, `ChangePrivatePassphrase`,
`ChangePublicPassphrase`, `AccountBalance` (op `wallet.CalculateAccountBalance`),
`AccountBalances`, `CurrentAddress`, `SignMessage`, `VerifyMessage`, `HaveAddress`,
`AccountNumber`, `AccountName`, `RenameAccount`, `NextAccount`, `AccountXpub`, `AccountXpriv`.

Returns callee errors unchanged: `SetTreasuryKeyPolicy`, `SetTSpendPolicy` (database errors),
`BlockHeader`, `CFilterV2`, `watchHDAddrs`, `LoadActiveDataFilters` (everything before the
outpoint `View`, and the final flush), `fetchMissingCFilters`, `SetAccountPassphrase`,
`UnlockAccount`, `LockAccount`, `SignHashes`, `TxBlock`, `TxConfirms`,
`GetTransactionsByHashes`, and the pre-authoring steps of the `PurchaseTickets` retry path.

Discards errors: `MainChainTip` (database), `Lock` (manager).

No function in this range terminates the process. Panics reachable from this range: the index
operations noted in section 10 (Q3, Q4) and nil-key derivation under the race in Q12.

---

## 9. Compatibility-critical behaviours

Each item is something an existing database, dcrd, a peer, a VSP, Decrediton, dcrctl or a gRPC
client can observe.

1. **Agenda preference records.** Buckets `agendaprefs` and `ticketsagendaprefs/<32-byte ticket
   hash>`; key = big-endian uint32 vote version followed by the agenda ID bytes; value = choice ID
   bytes (section 4.1). An existing wallet's saved choices must be read back identically, and
   choices written by the replacement must be readable by dcrwallet.
2. **Vote versions.** Mainnet 11, testnet3 12, simnet 12, retired testnet2 6, any other network 1 (section 5.2). The
   version selects the agenda list, is part of the preference key, and is serialised into every
   vote's extended bits as 4 little-endian bytes.
3. **Vote bit derivation.** Start from `0x0001`; OR in the bits of each saved choice; an unsaved or
   unrecognised choice contributes nothing. Updating the cached default bits clears the agenda's
   mask before setting the choice bits. Per-ticket preferences replace the defaults wholesale
   (no per-agenda fallback), and a ticket with no saved preference at all reports the wallet
   defaults.
4. **`getvotechoices` semantics.** Every agenda of the current version is reported; unset agendas
   report the literal `abstain`; an unowned ticket hash yields `item does not exist: ticket not
   found`; an owned ticket without preferences yields the defaults.
5. **`setvotechoice` validation and atomicity.** Unknown agenda or choice IDs fail with the exact
   messages of section 8.3 and nothing from the call is persisted.
6. **Treasury policy records.** Four buckets, key layouts and the one-byte value of section 4.2;
   "abstain" deletes the record; only policy bytes 0, 1, 2 are accepted.
7. **TSpend policy precedence.** per-ticket tspend policy, then wallet tspend policy, then (only
   if the tspend is cached) per-ticket key policy, then wallet key policy (section 5.4). This
   decides the treasury votes cast on chain. `TreasuryKeyPolicy` with a ticket consults only the
   per-ticket key map.
8. **TSpend cache.** In-memory only; duplicates rejected with `tspend already cached`; entries are
   dropped when `tip height > expiry` at the time of the next listing.
9. **Signed messages.** Preimage and hash of section 4.4; 65-byte compact signature with the
   compressed flag always set; RFC 6979 nonces (signatures are deterministic and byte-identical to
   dcrwallet's for the same key and message). Verification must accept signatures flagged
   uncompressed, hashing the uncompressed key.
10. **`SignHashes`.** DER signatures without a sighash byte, RFC 6979 deterministic, compressed
    public key returned; hashes of any length are accepted with the truncate/left-pad rule of
    section 5.12.
11. **Block locators.** The exact height sequence of section 5.6, including the duplicated first
    hash, is what peers receive in `getheaders` and what dcrd receives over JSON-RPC.
12. **Transaction filter contents.** After `LoadActiveDataFilters` the backend's filter holds:
    every HD address of every BIP0044 and imported-xpub account from child 0 (or from
    `lastReturned - watchLast` when `--watchlast` is set) through `lastReturned + gapLimit`
    (clamped to `0x7fffffff`), every address of the imported account, every unspent outpoint not
    spent by an unmined transaction (outputs of unpublished transactions excluded), and output 0
    (stake tree) of every mined ticket that is unspent or spent only by an unmined vote on a
    non-tip block. With
    `reload` the filter is cleared first. dcrd sees `loadtxfilter` calls of at most 256 addresses
    or 64 outpoints each.
13. **Watching is suppressed while a rescan is pending.** `watchHDAddrs(firstWatch=false)` does
    nothing when the rescan point is non-nil.
14. **Missing-cfilter back-fill.** Requests of at most 2000 consecutive main-chain blocks; filters
    for blocks at or above the DCP0005 height must carry a valid inclusion proof against the
    stored header's stake root; progress notifications carry `(start, start + 1999)`.
15. **Unlock semantics.** A wrong passphrase on an unlocked wallet locks it; errors carry kind
    `Passphrase` (JSON-RPC -14, gRPC `InvalidArgument`); a watching-only wallet reports kind
    `WatchingOnly`; an unlock without timeout is never downgraded by a later timed unlock; a timed
    unlock replaces an earlier timeout; an untimed unlock cancels an earlier timeout.
16. **Lock-state log lines** (info level, `WLLT` subsystem), exact text:
    `The wallet has been locked due to an incorrect passphrase.`,
    `The wallet has been locked due to timeout.`,
    `The wallet has been unlocked without a time limit`,
    `The wallet has been temporarily unlocked` (only the first two end with a period).
17. **Other log lines** in this range: `TSpend arrived: <hash>`,
    `Loading active addresses and unspent outputs...`,
    `Registered for transaction notifications for <n> HD address(es)`,
    `Registered for transaction notifications for <n> imported address(es)`,
    `Registered for transaction notifications for all relevant outputs`,
    `Fetched cfilters for blocks <a>-<b>` (info); `No best address`,
    `Not our address: hash160=<hex>` (debug);
    `Skipping commitment at index <i>: address is not P2PKH`,
    `Ticket purchase <hash>: best commitment address <addr> amount <amt>` (trace).
18. **Error strings.** Ops, kind strings, inner messages and the `":: "` separator of section 8 are
    returned verbatim as JSON-RPC `message` and gRPC status message, including the op
    `wallet.CalculateAccountBalance` for `AccountBalance`.
19. **`NetworkBackend()` with no backend** is kind `NoPeers` → JSON-RPC code -9, gRPC
    `Unavailable`. Offline mode instead fails individual operations with
    `operation not supported in offline mode` (JSON-RPC code -4) while reporting `Synced = (true, 0)`.
20. **Account creation rule.** A new account is refused when the last `accountGapLimit` accounts
    all have no used address on either branch (and at least that many accounts exist); the new
    account's address records for children `0..=gapLimit` are persisted on both branches; the name
    rules and error kinds come from the address manager.
21. **`getaccountaddress`** returns the external child at `lastUsed + 1 + cursor`, i.e. the next
    address `getnewaddress` would return, without recording it.
22. **Balance listing order.** `AccountBalances` returns accounts in byte-lexicographic order of
    the little-endian account number (section 10, Q15); clients that print `getbalance "*"` or
    account lists see this order.
23. **Credit category strings** `receive`, `generate`, `immature`; a coinbase is `generate` once
    `tip - height + 1 > CoinbaseMaturity`.
24. **Ticket purchase retry.** On the VSP-fee UTXO-split condition one split transaction is
    published whose designated output is worth `vspFee + FeeForSerializeSize(relayFee, 275)` (for
    a P2PKH internal address) with a randomised change position, then exactly one ticket is bought
    with `minconf = 0`. In SPV mode DCP0010 and DCP0012 are assumed active when computing the VSP
    fee; in RPC mode dcrd's `getblockchaininfo` decides.
25. **Subsidy-split activation rule** of section 5.15, including the true result on the last
    locked-in block and the simnet special case for DCP0010.
26. **Hardcoded activation heights** of DCP0001 (section 6); `NextStakeDifficulty` fails with kind
    `Deployment` below them.
27. **`GetTransactionsByHashes` partial results.** Found transactions and the not-found inventory
    list are both returned together with a `NotExist` error; the SPV `getdata` responder depends
    on receiving both (`spv/sync.go:818`).
28. **`CommittedTickets` selection.** The largest strictly-positive P2PKH commitment of each ticket
    decides ownership; only tickets whose best commitment hash160 is a wallet address are
    returned, in input order.
29. **Default passphrases.** `public` for the public passphrase; `password` for simnet temp
    wallets.
30. **Relay fee and disapprove percentage are volatile.** Changes made over RPC are not persisted
    and revert to configuration at restart.
31. **Wallet lock also locks individually-encrypted accounts** when the wallet was unlocked
    (`walletlock`, unlock timeout, wrong-passphrase lock), and does nothing to them when the
    wallet was already locked (section 5.10, Q33). `accountunlocked` results depend on this.
32. **`FetchOutput` not-found text** `wallet.FetchOutput: item does not exist: tx <hash> not found`
    reaches JSON-RPC `spendoutputs` clients (`internal/rpc/jsonrpc/methods.go:4257`).

---

## 10. Quirks

Each quirk states what the Go does; whether to reproduce or diverge is a project decision unless
an external party depends on it (noted where so).

**Q1. Map iteration order leaks into observable output.**
`GetAllTSpends` returns cached tspends in Go map order (`wallet/wallet.go:280`), which becomes the
order of treasury votes in the `TV` output of every vote the wallet creates
(`wallet/chainntfns.go:806`–`823`) and of tspend policy listings. `TreasuryKeyPolicies` returns two
map-ordered groups (`:645`, `:651`), which becomes the array order of JSON-RPC `treasurypolicy`
(`internal/rpc/jsonrpc/methods.go:3832`–`3850`) and gRPC `TreasuryPolicies`. `SetAgendaChoices`
iterates its input map (`:505`), so with several invalid entries the reported one is arbitrary.
Vote transactions are therefore not byte-reproducible across runs when two or more tspends are in
their voting window.

**Q2. TSpend cache documentation is stale.** The comment on `GetAllTSpends` says the list is not
culled (`:273`); the code deletes entries whose expiry is below the tip height (`:282`–`285`).
`AddTSpend` logs the arrival before checking for a duplicate (`:261`).

**Q3. `FetchOutput` does not bounds-check the output index** (`:230`); an index past the end of
the transaction's outputs is a Go index-out-of-range panic. The outpoint's tree is ignored. The
statement `return err` at `:231` always returns nil.

**Q4. `TSpendPolicy` slices the signature script without validation** (`:615`). A cached
transaction with no inputs or a script shorter than 99 bytes panics. In RPC mode cached tspends
come from dcrd without local validation (`chain/sync.go:702`–`704`, `:925`–`931`).

**Q5. `Lock` does not cancel an armed timeout task.** It sets `passphraseTimeoutCancel = nil`
without signalling the old channel (`:1846`). The task armed by an earlier timed unlock keeps
waiting on its timer and calls `Lock()` when it fires, logging
`The wallet has been locked due to timeout.` Sequence: `walletpassphrase p 60`, `walletlock`,
`walletpassphrase p 0` → the wallet, although "unlocked without a time limit", is locked about 60
seconds after the first call. The same stale task also fires after a wrong-passphrase lock.
`TestLocking` does not cover this sequence.

**Q6. `CurrentAddress` returns the next address, not the most recent.** The doc comment
(`:1948`–`1951`) says "most recently requested"; the index `lastUsed + 1 + cursor` (`:1963`) is the
one the next `NewExternalAddress` call will return, because the cursor is incremented after each
address is handed out (`wallet/addresses.go:637`). The address is not persisted as returned, no gap
policy is applied (the index may lie beyond the watched range), and `ErrInvalidChild` is returned
as an error instead of being skipped. JSON-RPC `getaccountaddress` exposes this
(`internal/rpc/jsonrpc/methods.go:1910`).

**Q7. Unlock log lines do not match the action** in two rows of the truth table (section 5.10): a
timed unlock of a wallet already unlocked without limit logs `temporarily unlocked` although the
timeout is ignored; replacing one timeout with another logs nothing.

**Q8. Every failure of `manager.Unlock` is reported as kind `Passphrase`** (`:1738`), including
database and decryption errors.

**Q9. `PurchaseTickets` retry path oddities** (`:1648`–`1700`): (a) the fee estimate passes an
input size (166) where a signature-script size (108) is expected, giving 275 bytes instead of 217;
(b) `publishAndWatch` is called with a nil backend and so uses the wallet's current backend
instead of the `n` argument; (c) `changeSourceUpdates.UpdateDB` is executed a second time in its
own write transaction after `recordAuthoredTx` already executed it; (d) the caller's request
struct is mutated (`MinConf`, `Count`, private split output); (e) errors from `FeePercentage`, `NextStakeDifficulty`, the deployment tests, `NewInternalAddress` and the final `UpdateDB` are returned without the `wallet.PurchaseTickets` op (`authorTx`, `recordAuthoredTx` and `publishAndWatch` receive the op and wrap with it); (f) `req.VSPClient` is dereferenced without a nil check.

**Q10. Block locators repeat the tip and may omit genesis** (`:1510`–`1532`). The step variable is
a signed 32-bit integer doubled without an overflow check; for heights of 2^30 or more the final
doubling wraps negative just as the loop ends (harmless in Go; a checked-arithmetic port must use
wrapping multiplication or stop doubling).

**Q11. Missing-cfilter loop.** (a) Progress and log always report `height + 1999` as the end even
for a short final batch (`:1411`–`1413`). (b) `height` is advanced only on the following iteration,
costing one extra read transaction per batch (`:1334`–`1338`). (c) The op is built with `%v` of the
backend (`:1427`, `:1454`); `chain.Syncer`, the only real caller's argument, has no `String`
method, so the op text is Go's default struct dump of the syncer, including pointer values that
differ per run. `spv.Syncer` prints `spv.Syncer`. (d) The `unexpected slice reallocation` check
(`:1350`–`1353`) is Go-specific. (e) A backend returning more filters than requested panics at
`get[i]` (`:1377`). (f) `FetchMissingCFilters` (no progress) has no caller.

**Q12. `watchHDAddrs`.**
(a) *`--watchlast` wraparound.* For a branch that has never returned an address,
`end = gapLimit - 1`, so `end - gapLimit` wraps to `0xffffffff`, the condition at `:1004` is true
for any nonzero `watchLast`, `start` becomes `0xffffffff - watchLast`, and the loop body never
runs: **no address of that branch is watched** on the first watch. Later non-first calls start
from `lastWatched = gapLimit - 1`, so children `0..gapLimit-2` stay unwatched by this function
until the wallet restarts with an address returned on that branch.
(b) On the first watch `lastWatched` is advanced before the addresses are loaded (`:992`–`993`);
a failed load leaves memory claiming they are watched.
(c) Races with account creation: `NextAccount` (and the import functions in part 2) commit the
account to the database before adding its address buffer. If `watchHDAddrs` reads the database
after the commit but takes `addressBuffersMu` before the buffer is added, the account has nil
branch keys and derivation dereferences a nil key (panic) (`:989`–`990`, `:1023`). In the opposite
interleaving the buffer exists but the account data is zero-valued, and the adjustment at
`:974`–`980` rewrites a fresh buffer's `lastUsed` from `0xffffffff` to 0. Both windows are narrow
and were found by reading the code, not observed.
(d) The returned count includes batches whose `LoadTxFilter` call failed (`:1033`–`1034`).
(e) Batches of one branch are submitted to a CPU-limited pool, so the order of `loadtxfilter`
calls is nondeterministic.
(f) Non-first calls re-send the address at index `lastWatched` (inclusive start).

**Q13. `NextAccount`.** (a) Comments speak of "the previous 100 accounts" (`:2136`, `:2148`,
`:2158`); the limit is `accountGapLimit`, default 10. (b) `SyncAccountToAddrIndex` records
`gapLimit + 1` addresses per branch (children `0..=gapLimit`), while the filter is loaded with
`gapLimit` addresses (`0..gapLimit-1`) (`:2191`–`2197`, `:2219`–`2220`). (c) A filter-load failure
returns an error after the account has been committed and buffered, without the account
notification (`:2228`–`2233`). (d) `accountGapLimit <= 0`: zero always refuses; a negative value
converts to a huge unsigned bound, and the loop then runs until `lastAcct - i` reaches 0, which
always permits creation.

**Q14. Mismatched op and cost of balances.** `AccountBalance` uses the op
`wallet.CalculateAccountBalance` (`:1912`). `AccountBalances` calls `txStore.AccountBalance` once
per account, and each call computes the balances of all accounts by a full scan
(`wallet/udb/txmined.go:3561`–`3583`), i.e. quadratic work.

**Q15. Account iteration order is little-endian lexicographic.** `ForEachAccount` walks bucket
keys that are little-endian account numbers (`wallet/udb/addressdb.go:698`–`708`). With fewer than
256 BIP0044 accounts the order is: account 0; imported-xpub accounts `0x80000000 + k` (keys
`k 00 00 80`, interleaved by their low byte: `0x80000000` sorts immediately after account 0,
`0x80000001` after account 1, and so on); accounts 1, 2, …; the imported account `0x7fffffff`
(key `ff ff ff 7f`) last. Account 256 (key `00 01 00 00`) would sort between account 0 and
account 1.

**Q16. Inconsistent error wrapping.** See section 8.4: several exported methods return callee
errors without their own op, and several originate errors with no op at all, so messages differ in
shape between otherwise similar calls (`AgendaChoices`' `ticket not found` has no op while its
database errors do).

**Q17. `MainChainTip` hides database errors** (`:797`–`801`): the zero hash and height 0 are
returned as if the tip were genesis-less.

**Q18. `BlockIdentity.None` treats height 0 with a zero hash as "no block"**
(`wallet/common.go:28`–`31`), documented in the source as a bug: both 0 and -1 are used for
unmined transactions elsewhere.

**Q19. Per-ticket agenda preferences have no per-agenda fallback** (`:359`–`373`, `:436`–`455`),
and preferences are keyed by vote version, so every saved choice silently reverts to abstain when
a release raises the vote version. The same version number is big-endian in the key and
little-endian in the vote bits.

**Q20. `SetAgendaChoices` documentation vs. signature.** The comment (`:472`) says a choice set
multiple times takes the last value, which cannot happen with a map argument. With a ticket hash
and an empty map the function returns vote bits 0 (`:476`, `:544`–`545`).

**Q21. Asymmetric fallback.** `TSpendPolicy` falls back from per-ticket to wallet-wide policies;
`TreasuryKeyPolicy(pikey, ticket)` does not (`:577`–`583`).

**Q22. `VerifyMessage` compares address strings** (`:2062`), so the supplied address type matters;
and `SignHashes` accepts hashes that are not 32 bytes (section 5.12).

**Q23. Dead or vestigial code.** `debugLog`/`infoLog` and their types (`wallet/log.go:22`–`38`);
`closedDone` and its `init` (`wallet/network.go:130`–`134`); `StakeDifficultyInfo` (`:86`);
`TxConfirms` (`:2300`); `FetchMissingCFilters` (`:1422`); `DCP0002`, `DCP0003`; the testnet2
branches in `voteVersion` and `HardcodedDeployment.Active`; `wallet/doc.go` is a placeholder
("Package wallet provides ... TODO: Flesh out this section"); the comment on the `Wallet` struct is
truncated (`:99`–`101`).

**Q24. Networks without a vote version.** Regnet and any unknown network get vote version 1, for
which chaincfg defines no deployments: `AgendaChoices` returns an empty map and bits 0 (not 1),
`SetAgendaChoices` fails with `no agendas to set for this network`, and votes carry extended bits
`01 00 00 00`. dcrwallet exposes no regnet option (only `cmd/treasurykey` knows regnet).

**Q25. Deployment tests.** `DCP0010Active` is unconditionally true on simnet while
`DCP0012Active` requires a querier there; with a nil querier on simnet DCP0012 is a `Bug` error.
`int32(d.Since)` truncates dcrd's 64-bit height. In SPV mode callers skip the tests and assume
both active (`:1633`–`1647`).

**Q26. `LoadActiveDataFilters` performs network calls inside database read transactions**
(`:1145`, `:1124`) and while holding `lockedOutpointMu`, which blocks transaction authoring for
the duration. Ticket outpoints are sent twice whenever the ticket is not spent by an unmined transaction (section 5.5). The last outpoint batch is sent from a deferred
function after the completion log line and after the lock is released (`:1113`–`1120`).

**Q27. `CommittedTickets` stops scanning a ticket at the first malformed commitment** but keeps
any best address found before it (`:1233`–`1236`, `:1244`–`1247`); zero-amount commitments are
never selected; the returned hash pointers alias the caller's input slice elements.

**Q28. `OfflineNetworkBackend.Synced` reports `(true, 0)`** although nothing is synced
(`wallet/network.go:126`–`128`).

**Q29. `GetTransactionsByHashes` returns data together with an error** (`:2338`–`2341`).

**Q30. `AgendaChoices` returns whatever string is stored**, even one that matches no choice of the
agenda (`:448`); such a value contributes no vote bits.

**Q31. `VoteBits()` returns a struct whose `ExtendedBits` slice aliases the cached slice**
(`:403`); a caller that mutates it corrupts the cache. Nothing in the repository does.

**Q32. Source comments that do not match the code, and outstanding TODO/BUG markers in scope.**
None changes behaviour; they are listed so a port does not implement the comment.
(a) `FetchOutput`'s comment says it "cannot be used to fetch multi-signature outputs" (`:218`); the
code returns any output of any stored transaction.
(b) `disapprovePercent` is commented as applying "on simnet or testnet" (`:103`–`104`); the wallet
layer applies no network check (the JSON-RPC handler does).
(c) `tspendKeyPolicy` is commented "keyed by politeia key" (`:120`); the key is the raw treasury
public key bytes as a string.
(d) `NetworkBackend.StakeDifficulty` is commented as always failing (`wallet/network.go:43`–`45`);
that holds for the SPV backend, while `chain.Syncer.StakeDifficulty` forwards dcrd's answer
(`chain/backend.go:58`–`60`).
(e) `FilterProof`'s comment refers to a `Peer.CFiltersV2` call and a `Peer` interface
(`wallet/network.go:19`–`25`); the interface is `NetworkBackend`.
(f) `UseLogger`'s comment refers to a `SetLogWriter` function (`wallet/log.go:16`–`17`) that does
not exist.
(g) `InsecurePubPassphrase`'s comment notes that public encryption applies only to the `waddrmgr`
namespace and that transactions are not encrypted (`:64`–`65`); that is the address manager's and
tx store's behaviour, restated here because it bounds what a non-default public passphrase
protects.
(h) TODO at `:793`–`796`: cache the main-chain tip in memory so `MainChainTip` need not ignore a
database error (Q17). TODO at `:2348` and `:2378`–`2379`: `CreditCategory` and `recvCategory`
exist for the RPC server and "should be moved". TODOs at `wallet/common.go:45`–`46` (report how
many required signatures the wallet can create) and `:48`–`52` (replace the duplicated fields of
`P2SHMultiSigOutput` with a transaction-output member). BUG at `wallet/common.go:28`–`30` (Q18).
`wallet/common.go:14`–`16` states the common types must never reference `Wallet` and are meant to
move to their own package.
(i) `GetAllTSpends`' comment says the list is not culled (Q2); `NextAccount`'s comments say 100
accounts (Q13a); `CurrentAddress`' comment says "most recently requested" (Q6);
`SetAgendaChoices`' comment describes repeated choices (Q20).

**Q33. `Lock()` on an already-locked wallet leaves individually-unlocked accounts unlocked.**
`manager.Lock` returns kind `Locked` before clearing any key when the manager is locked
(`wallet/udb/addressmanager.go:1462`–`1465`), and `Wallet.Lock` discards that error (`:1845`).
Sequence: wallet locked, `UnlockAccount(a, pass)`, `Lock()` → account `a` stays unlocked. With the
wallet unlocked the same `Lock()` drops account `a`'s key too
(`wallet/udb/addressmanager.go:359`–`364`). `Lock()` clears `passphraseTimeoutCancel` in both
cases.

**Q34. `Unlock` samples `wasLocked` outside the lock it later acts under** (`:1715`–`1718`,
`:1742`). On the nil path (already unlocked with the right passphrase) no lock is held between the
check and `replacePassphraseTimeout`, so a concurrent `Lock` yields a nil return for a wallet that
is locked, a log decision based on the stale flag, and possibly an armed timeout task. Found by
reading the code, not observed.

**Q35. `ForEachUnspentOutpoint` skips outputs of unpublished transactions; `UnspentTickets` has no
such filter** (`wallet/udb/txmined.go:2603`–`2607`, `:2656`–`2721`). The latter cannot return
unmined tickets at all, so the asymmetry does not change the filter contents, but the two sources
are not interchangeable.

---

## 11. Upstream tests

| Test file | What it pins for this scope |
|---|---|
| `wallet/locking_test.go` (`TestLocking`, five sub-tests) | `Unlock` without timeout then `Lock`; a wrong passphrase on an unlocked wallet returns kind `Passphrase` and locks it (twice in a row); a timed unlock of a wallet already unlocked without limit drains the channel within 100 ms and leaves the wallet unlocked; a timed unlock locks after the channel fires; a second timed unlock replaces the first and the first channel is still drained. Uses real time (100 ms sleeps). |
| `wallet/addresses_test.go` (`TestAccountIndexes`) | The `lastUsed`/`cursor` adjustment of `watchHDAddrs` step 3 together with address generation, as the sequence of nine states listed at `wallet/addresses_test.go:229`–`239` (gap limit 20, simnet). |
| `wallet/reorg_test.go` | Not a test of this range, but its helpers assert through two functions in scope: `MainChainTip` after each chain switch (`wallet/reorg_test.go:122`) and `BlockInMainChain` with expected `haveBlock`/`invalidated` pairs (`:129`–`139`). Porting the reorg tests therefore pins both. |
| `wallet/network_test.go` | Not a test: `mockNetwork`, a `NetworkBackend` returning zero values (`Synced` = `(false, 0)`), embedded by other tests. |
| `wallet/setup_test.go`, `wallet/main_test.go` | Harness: `basicWalletConfig` (simnet, gap limit 20, relay fee 1e5, mixing enabled, public passphrase `public`), private passphrase `private`, temp bbolt database, `-log` flag enabling the `WLLT` logger. |
| `wallet/wallet_test.go` (`TestCoinbaseMatured`) | `coinbaseMatured`, the helper `recvCategory` depends on (function declared in part 2's range). The other tests in the file belong to parts 2 and 3. |
| `errors/errors_test.go` (`TestCollapse`, `TestIs`, `TestDoubleWrappedErrorWithKind`) | Error collapsing and matching rules that shape every message in section 8. |

`TestAccountIndexes` reaches the in-scope code by calling `w.watchHDAddrs(ctx, false, n)` directly
with the `mockNetwork` backend (`wallet/addresses_test.go:279`); it is the only test that calls an
unexported function of this range. The remaining test files of package `wallet`
(`discovery_test.go`, `mixing_test.go`, `sidechains_test.go`) reference nothing declared in this
range (checked by searching every `_test.go` file in the repository for each in-scope identifier).
The `deployments` and `validate` packages have no test files. No `wallet/udb` test references the
agenda-preference or treasury-policy functions.

Untested upstream (no test references found by repository search): the tspend cache; `voteVersion`,
`CurrentAgendas`, `readDBVoteBits`, `readDBTicketVoteBits`; `AgendaChoices`, `SetAgendaChoices`;
all treasury policy functions and their `udb` storage; `LoadActiveDataFilters`;
`CommittedTickets`; the missing-cfilter loop; `BlockLocators`; the `PurchaseTickets` retry path;
per-account passphrase wrappers; `ChangePrivatePassphrase`/`ChangePublicPassphrase` at the wallet
level (the manager's `ChangePassphrase` is tested in `wallet/udb/addressmanager_test.go`);
`AccountBalance(s)` at the wallet level; `CurrentAddress`; `SignHashes`, `SignMessage`,
`VerifyMessage`; `HaveAddress`, `AccountNumber`, `AccountName`, `RenameAccount`, `NextAccount`,
`AccountXpub`, `AccountXpriv`; `TxBlock`, `TxConfirms`, `GetTransactionsByHashes`;
`CreditCategory`; `OfflineNetworkBackend`; the whole `deployments` package; `BlockIdentity.None`.

Vectors to port directly: `TestLocking` (rewritten against a controllable clock or explicit
channels) and `TestAccountIndexes`.

Vectors to generate through a Go oracle shim (run dcrwallet's own functions against a fixed-seed
simnet wallet and record outputs):

1. `SignMessage`/`VerifyMessage`: signatures for several messages (empty, ASCII, multi-byte UTF-8,
   length ≥ 253 to exercise the 3-byte varint) and keys; verification of compressed- and
   uncompressed-flag signatures and of malformed inputs.
2. `SignHashes`: DER outputs for 32-byte, shorter and longer hashes.
3. `BlockLocators`: locator height lists for tips 0, 1, 9, 10, 11, 12, 100, 1000, 100000, and with
   sidechains of length 1 and 5.
4. Agenda preferences: database key/value bytes after `SetAgendaChoices` (default and per-ticket),
   returned vote bits, `AgendaChoices` maps, error strings for bad agenda/choice/ticket.
5. Treasury policies: bucket contents after set/clear for all four variants; `TSpendPolicy`
   results for the six precedence cases.
6. `LoadActiveDataFilters`: the multiset of addresses and outpoints passed to a recording backend
   for wallets with used/returned addresses, imported keys, imported xpub accounts, tickets, with
   `watchLast` 0 and nonzero (including the Q12a case).
7. Missing-cfilter loop: request sizes, progress values and final database state against a
   scripted backend for chain lengths 1, 1999, 2000, 2001, 4500.
8. Unlock/lock: the eight truth-table rows with their log lines, and the Q5 sequence.
9. `NextAccount`: allowed/refused creation around the account gap limit; persisted address rows.
10. Error strings of section 8.3 with the `":: "` separator.
11. `AccountBalances` ordering with an imported-xpub account present.
12. `DCP0010Active`/`DCP0012Active` over the cross product of network, status, `since`, height.
13. Split-output value for the `PurchaseTickets` retry at a fixed ticket price, relay fee and VSP
    fee percentage.

---

## 12. Rust porting notes

### 12.1 Suggested boundaries

| Suggested crate / module | Mirrors | Contents |
|---|---|---|
| `purse-errors` | `errors/` | `Kind`, `Op`, the collapsing constructor, the formatter with a process-wide separator. Needed by everything in this spec; specified by the errors spec. |
| `purse-deployments` | `deployments/` | `HardcodedDeployment`, the three constants, `Querier` trait, `dcp0010_active`, `dcp0012_active`. Tiny, no wallet dependency. |
| `purse-wallet::network` | `wallet/network.go` | `NetworkBackend` trait (async), `FilterProof`, `Caller` trait, `OfflineNetworkBackend`. |
| `purse-wallet::core` | `Wallet`, `Config` | Struct, field guards, accessors (rows 1–2, 4, 22–29 of section 3.6). |
| `purse-wallet::voting` | `:206`–`761` | TSpend cache, vote version, vote bits, agenda choices, treasury policies. |
| `purse-wallet::filters` | `:883`–`1201`, `:1287`–`1457` | `watch_hd_addrs`, `load_active_data_filters`, missing-cfilter loop. |
| `purse-wallet::chainquery` | `:785`–`881`, `:1459`–`1535`, `:2276`–`2342` | Tip/header/cfilter queries, locators, transaction lookups. |
| `purse-wallet::locking` | `:1703`–`1895` | Unlock/lock state machine, per-account passphrase wrappers, passphrase changes. |
| `purse-wallet::accounts` | `:1897`–`1973`, `:2065`–`2274` | Balances, current address, account lookup/rename/create, xpub/xpriv. |
| `purse-wallet::signing` | `:1975`–`2063` | Hash and message signing, verification. |
| `purse-wallet::types` | `wallet/common.go`, `CreditCategory`, `Balances` | Plain data types. |

`PurchaseTickets` (`:1549`–`1701`) belongs with the transaction-creation module that ports
`wallet/createtx.go`, since it is a retry wrapper around `purchaseTickets`.

### 12.2 Reusable code (each item was opened and checked)

| Need | Candidate | Notes |
|---|---|---|
| Ordered deployment tables, vote IDs | `dcroxide:crates/dcroxide-chaincfg/src/lib.rs:96`, `:176`; `dcroxide:crates/dcroxide-chaincfg/src/votes.rs:43` | `Params.deployments` is an ordered `Vec<(u32, Vec<ConsensusDeployment>)>`, which removes Go's map-order dependence in `Open`'s `deploymentsByID` construction. `maxtreasuryspend` is present. |
| `VoteBits`, treasury vote constants, sstx commitment helpers, `is_sstx` | `dcroxide:crates/dcroxide-stake/src/lib.rs:196`, `:392`, `:417`, `:808`, `:816`–`820` | Direct equivalents of the `stake` items used here. |
| ECDSA sign, compact sign, compact recover | `dcroxide:crates/dcroxide-dcrec/src/secp256k1/ecdsa.rs:435`, `:619`, `:633` | All three take `&[u8; 32]`. `SignHashes` needs the truncate/left-pad adapter of section 5.12 for other lengths. |
| Inclusion-proof verification, coinbase test, compact bits, subsidy cache | `dcroxide:crates/dcroxide-standalone/src/inclusionproof.rs:119`, `tx.rs:40`, `pow.rs:69`, `subsidy.rs:98` | For `validate.CFilterV2HeaderCommitment`, `recvCategory`, and `Open`. |
| `FilterV2` | `dcroxide:crates/dcroxide-gcs/src/lib.rs:477` | Filter type returned by `CFilterV2` and consumed by the back-fill. |
| BLAKE-256 hash of bytes | `dcroxide:crates/dcroxide-chainhash/src/lib.rs:230` | Message hash. |
| HASH160, P2PKH addresses | `dcroxide:crates/dcroxide-txscript/src/stdaddr.rs:144` | Address derivation and `VerifyMessage`. |
| dcrd `AgendaInfo` | `dcroxide:crates/dcroxide-rpctypes/src/chainsvrresults.rs:140` | Input of the `Querier` trait. |
| Signed-message preimage | `dcroxide:crates/dcroxide-rpc/src/handlers.rs:855` | dcroxide's `verifymessage` builds the same var-string preimage with a private helper; lift it into a shared place rather than duplicating. |
| HD key derivation | `dcr-rs:src/hd.rs:132`, `:280`, `:292`, `:545`, `:574` | `derive_child` is the Decred (`hdkeychain.Child`) variant; `derive_child_bip32_std` must not be used. dcr-rs builds on a different secp256k1 backend than dcroxide-dcrec, and its `ExtPubKey::derive_child` reports failures as a generic derivation error; the port needs a distinguishable "invalid child" outcome because `watchHDAddrs` and `deriveChildAddresses` skip exactly that case. dcroxide has no hdkeychain crate. |

### 12.3 Hazards

1. **Unsigned wraparound is load-bearing.** `lastReturned + gapLimit`, `lastUsed + 1`,
   `cursor += lastUsed - dbLastUsed`, `end - gapLimit`, `lastAcct - i`, `lastUsed + 1 + cursor` all
   rely on modulo-2^32 arithmetic with `0xffffffff` as a sentinel. Use explicit wrapping
   operations; debug-build overflow checks would otherwise panic on the common fresh-account case.
2. **The unlock state machine holds a read lock across key derivation** and is re-entered by
   timer tasks that take the write lock. An async port must not hold a synchronous lock across an
   await point in a way that lets the timer task deadlock; model it as a small actor or keep the
   same two-lock discipline with an async-aware lock. The eight-row truth table and Q5 define the
   observable behaviour.
3. **Timeout channels.** The Go API takes a receive-only channel and promises to drain it. A Rust
   signature taking a future/oneshot must preserve "caller's send never blocks" and "an ignored
   timeout is still consumed".
4. **Network calls inside read transactions** (Q26). With an LMDB/bbolt-style store a long read
   transaction pins pages; if the port keeps this shape it inherits that, and if it collects first
   and sends after, the batch boundaries (256/64) and ordering guarantees in section 9 item 12
   must still hold.
5. **Map-order nondeterminism** (Q1). Rust's `HashMap` order differs from Go's; any choice is
   "compatible" in the sense that Go's order is itself random, but tests against a Go oracle must
   compare as sets, and the project should decide on a deterministic order.
6. **Panics as behaviour** (Q3, Q4, Q11e, Q12c). The port should return errors; the error kind and
   text need a decision because Go has none.
7. **The `%v` op in the cfilter fetch** (Q11c) cannot be reproduced; choose a fixed op string.
8. **`Balances` field order and the per-account full scan** (Q14): compute all balances once per
   `AccountBalances` call; results are identical.
9. **Error collapsing** changes message shape depending on whether the callee already set an op or
   kind; port `errors.E` exactly before porting any message in section 8.3.
10. **`VerifyMessage` must hash the uncompressed key** when the flag says so, and must compare
    address strings, not key hashes, to keep `(false, nil)` for non-P2PKH inputs.

### 12.4 Size estimate

**M.** About 2,400 Go lines in range plus 400 in the small files, but roughly fifty of the
seventy-five functions are single-transaction wrappers. The substantive pieces are the voting and
treasury preference logic (~350 lines), `watchHDAddrs`/`LoadActiveDataFilters` (~320), the
missing-cfilter loop (~130), the unlock state machine (~140) and `NextAccount` (~100). The risk is
in fidelity (wraparound, lock ordering, exact strings), not volume. It cannot start before the
database layer, the address manager and the tx store expose the methods listed in section 14.

---

## 13. Open questions

1. **Deterministic ordering (Q1).** Should the port fix the order of tspend votes within a vote
   transaction and of policy listings (for example ascending by hash/key), accepting that it
   differs from any particular Go run? Go's order is random, so no client can depend on it, but the
   decision affects oracle tests.
2. **Stale timeout relock (Q5).** Reproduce (strict parity) or cancel the timer on `Lock` (the
   evident intent)? A client could in principle rely on neither; Decrediton's behaviour under the
   sequence was not examined. UNVERIFIED: whether any shipped client issues the triggering
   sequence; settling it needs a review of Decrediton's and dcrctl users' lock/unlock call
   patterns.
3. **`--watchlast` wraparound (Q12a).** Reproduce or fix? Fixing watches more addresses than
   dcrwallet, which is safe; reproducing preserves a case where receipts to never-returned
   branches go unnoticed until restart.
4. **Account-creation races (Q12c).** UNVERIFIED by execution; a Go test that interleaves
   `NextAccount` with `watchHDAddrs(false)` under the race detector would settle whether they are
   reachable. The port should close both windows regardless.
5. **Behaviour replacing Go panics** (Q3, Q4, Q11e): which error kind and message, and for
   JSON-RPC which code. UNVERIFIED: how a panic in these paths surfaces today (HTTP handler
   recovery vs. process crash for websocket and gRPC paths); settling it needs the RPC server
   specs.
6. **Op string for the cfilter fetch (Q11c).** Proposed: `wallet.FetchMissingCFilters(chain.Syncer)`
   or the bare `wallet.FetchMissingCFilters`; a decision is needed because the Go text is not
   reproducible.
7. **Split-output fee estimate (Q9a).** Reproducing 275 bytes keeps split transactions
   amount-identical to dcrwallet for the same inputs; correcting it changes the output value by
   580 atoms at the default fee rate. Recommended: reproduce.
8. **Regnet.** dcrwallet has no regnet mode; if dcr-purse adds one, the vote version (Q24) must be
   chosen deliberately rather than inherited as 1.
9. **`AccountBalances` order (Q15).** Reproduce the little-endian key order (falls out naturally
   if the port iterates the same bucket) or sort numerically? JSON-RPC consumers of
   `getbalance "*"` see the order.
10. **Whether `TreasuryKeyPolicy`'s missing fallback (Q21) is relied on by vspd-facing code**
    (`wallet/feepayment.go:527`, `wallet/vspticket.go:213`). UNVERIFIED here; the VSP spec should
    confirm what is sent to the VSP for a ticket without a per-ticket key policy.
11. **Error separator.** The daemon sets `":: "`; library consumers of the Go packages see
    `":\n\t"`. dcr-purse as a daemon needs only the former; confirm no test fixture or client
    expects the multi-line form.

---

## 14. Cross-references into other ranges and specs

Into `wallet/wallet.go` part 2 (2394–4428):

- `BlockIdentifier`, `NewBlockIdentifierFromHeight`, `NewBlockIdentifierFromHash` (`:2725`–`2736`)
  and `blockRange` (`:3040`) — used by `RangeCFiltersV2`.
- `listTransactions` (`:2394`) — the only caller of `recvCategory`.
- `confirms` (`:4411`), `coinbaseMatured` (`:4422`) — used by `TxConfirms` and `recvCategory`.
- `ImportVotingAccount` (`:3838`), `ImportXpubAccount` (`:3893`) — also add address buffers after
  commit (relevant to Q12c).
- `LockOutpoint` and the locked-outpoint accessors (`:4250`–`4287`) — share `lockedOutpointMu`.

Into `wallet/wallet.go` part 3 (4429–5828):

- `Create` (`:5292`), `CreateWatchOnly` (`:5316`), `Open` (`:5340`) — construct and populate the
  `Wallet` described in section 3.3; `Open` calls `readDBVoteBits`, `readDBTreasuryPolicies`,
  `readDBTreasuryKeyPolicies` and `MainChainTip`.
- `GapLimit` (`:5326`), `ManualTickets` (`:5334`), `MixingEnabled` (`:5534`), `Run` (`:5545`),
  `ChainParams` (`:5187`), `AllowsHighFees` (`:5096`) — accessors for fields declared here.
- `AbandonTransaction` (`:5073`) — called by `publishAndWatch` on publish failure.

Into other files of package `wallet`:

- `createtx.go`: `purchaseTickets` (`:1162`), `errVSPFeeRequiresUTXOSplit` (`:1152`), `authorTx`
  (`:388`), `recordAuthoredTx` (`:510`), `publishAndWatch` (`:331`), `compressWallet` (`:769`),
  `txToMultisig` (`:544`), `Input` (`:53`), `CreatedTx` (`:242`).
- `addresses.go`: `addressBuffer`, `bip0044AccountData` (`:446`–`460`), `deriveChildAddresses`
  (`:1004`), `deriveChildAddress` (`:1023`), `deriveBranches` (`:1031`), `NewInternalAddress`
  (`:763`), `accountBranchChildUpdates.UpdateDB` (`:467`), `Address` interface (`:48`),
  `DefaultGapLimit`, `DefaultAccountGapLimit` (`:385`, `:389`).
- `rescan.go`: `RescanPoint` (`:462`). `difficulty.go`: `NextStakeDifficulty` (`:792`).
- `notifications.go`: `NotificationServer`, `notifyAccountProperties` (`:600`).
- `sidechains.go`: `BlockNode` (`:33`). `db.go`: `DB` (`:15`). `vsp.go`: `DialFunc` (`:27`),
  `VSPClient` (`:35`), `FeePercentage` (`:108`). `mixing.go`: `mixSemaphores` (`:236`).
- `chainntfns.go`: vote creation consuming `VoteBits`, `readDBTicketVoteBits`,
  `DisapprovePercent`, `GetAllTSpends`, `TSpendPolicy` (`:701`–`823`).

Into other packages:

- `wallet/udb`: `Manager` methods `IsLocked`, `Lock`, `Unlock`, `UnlockedWithPassphrase`,
  `WatchingOnly`, `ChangePassphrase`, `SetAccountPassphrase`, `UnlockAccount`, `LockAccount`,
  `AccountHasPassphrase`, `PrivateKey`, `Address`, `LookupAccount`, `AccountName`,
  `RenameAccount`, `NewAccount`, `AccountProperties`, `AccountExtendedPubKey`,
  `AccountExtendedPrivKey`, `SyncAccountToAddrIndex`, `LastAccount`, `LastImportedAccount`,
  `ForEachAccount`, `ForEachAccountAddress`, `ExistsHash160`, `CoinType`, `CoinTypePrivKey`;
  `Store` methods `Tx`, `TxBlockHeight`, `OwnTicket`, `MainChainTip`, `BlockInMainChain`,
  `GetBlockHeader`, `CFilterV2`, `RangeBlocks`, `IsMissingMainChainCFilters`,
  `MissingCFiltersHeight`, `GetMainChainBlockHashForHeight`, `GetMainChainBlockHashes`,
  `InsertMissingCFilters`, `ForEachUnspentOutpoint`, `UnspentTickets`, `AccountBalance`; free
  functions in `agendas.go` and `treasury.go`.
- `wallet/walletdb`: `View`, `Update`, `ReadTx`, `ReadWriteTx`.
- `errors`, `validate` (`CFilterV2HeaderCommitment`), `internal/compat`, `wallet/txrules`
  (`StakePoolTicketFee`, `FeeForSerializeSize`), `wallet/txsizes` (`EstimateSerializeSize`,
  `RedeemP2PKHInputSize`).
- `chain` and `spv`: the two `NetworkBackend` implementations; `chain.Syncer.Deployments`
  (`chain/backend.go:63`) is the only `deployments.Querier`.
