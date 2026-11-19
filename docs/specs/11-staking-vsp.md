# 11 — Staking: ticket purchase, voting, revocation, VSP client, ticket buyer

## 1. Header

**Scope** (line counts are whole-file; the in-scope ranges are given where only part of a file belongs here):

| File | Lines | In-scope part |
| --- | --- | --- |
| `wallet/createtx.go` | 2176 | constants 39-50; `makeTicket` 922-992; `mixedSplit` 996-1093; `individualSplit` 1099-1150; `purchaseTickets` 1152-1668; `ReserveOutputsForAmount`, `reserveOutputs`, `findEligibleOutputs`, `findEligibleOutputsAmount` 1670-1987; `signP2PKHMsgTx` 1989-2023; vote signing and construction 2025-2176. (`authorTx`, `recordAuthoredTx`, `publishAndWatch` 329-540 are called from here and summarised; their owner is the transaction-authoring spec.) |
| `wallet/wallet.go` | 5828 | stake settings and policies 99-180, 241-766; `PurchaseTicketsRequest/Response`, `PurchaseTickets` 1549-1701; `SignMessage` 2008-2032; `hasVotingAuthority` 3985-4005; maturity helpers 4404-4447; `CreateVspPayment` 4662-4762; VSP queries 5611-5828 |
| `wallet/chainntfns.go` | 933 | VSP pruning hook in `ChainSwitch` 241-274; `selectOwnedTickets`, `VoteOnOwnedTickets`, `RevokeOwnedTickets` 678-933 |
| `wallet/vsp.go` | 403 | all |
| `wallet/vspticket.go` | 339 | all |
| `wallet/feepayment.go` | 618 | all |
| `wallet/vspclientloader.go` | 49 | all |
| `wallet/udb/vsp.go` | 289 | all (the `vsp`, `vsphost`, `vsppubkey` buckets) |
| `wallet/udb/treasury.go` | 264 | all (the four treasury policy buckets) |
| `wallet/txrules/poolfees.go` | 95 | all |
| `deployments/deployments.go` | 157 | `Querier`, `DCP0010Active`, `DCP0012Active` 75-157 |
| `ticketbuyer/tb.go` | 340 | all |
| `ticketbuyer/log.go` | 19 | all |
| `vspd/client/v4@v4.0.2/client.go` | 272 | all |
| `vspd/types/v3@v3.0.0/types.go` | 103 | all |
| `vspd/types/v3@v3.0.0/errors.go` | 120 | all |
| `dcrd/blockchain/stake/v5@v5.0.3/staketx.go` | 1478 | all (wallet uses a subset; see section 3.9) |
| `dcrd/blockchain/stake/v5@v5.0.3/scripttype.go` | 82 | all |
| `dcrd/blockchain/stake/v5@v5.0.3/treasury.go` | 318 | `CheckTSpend`, `IsTSpend` |
| `dcrd/blockchain/stake/v5@v5.0.3/lottery.go` | 209 | `Hash256PRNG` (only reachable through `CalculateRevocationRewards`, which the wallet does not call) |

Callers read for wiring only (owned by other specs): `dcrwallet.go:240-443`, `config.go:186-197,562-572,850-854,952-972`, `internal/rpc/jsonrpc/methods.go:3353-3561,3855-4064,4775-4864`, `internal/rpc/rpcserver/server.go:1790-1940,2530-2686,4049-4284`, `chain/sync.go:645-651,697-711,774-822,925-931`, `rpc/client/dcrd/notifications.go:24-43`.

**Parity pin**: dcrwallet `54b85b33` (2.2.0-pre, module `decred.org/dcrwallet/v5`). Dependencies read at the versions `go.mod` pins: `github.com/decred/vspd/client/v4 v4.0.2`, `github.com/decred/vspd/types/v3 v3.0.0`, `github.com/decred/dcrd/blockchain/stake/v5 v5.0.3`, `github.com/decred/dcrd/blockchain/standalone/v2 v2.3.0`, `github.com/decred/dcrd/txscript/v4 v4.1.2`, `github.com/decred/dcrd/wire v1.8.0`, `github.com/decred/dcrd/chaincfg/v3 v3.3.0`, `github.com/decred/dcrd/crypto/rand v1.0.1`, `github.com/decred/dcrd/mixing v0.7.5` (only `mixclient.NewCoinJoin` and the `CoinJoin` accessors), `github.com/decred/slog v1.2.0`.

**Summary**

- A ticket purchase is always two steps: a *split transaction* that creates one exactly-sized P2PKH output per ticket (plain, or a StakeShuffle coinjoin when mixing), then one single-input, three-output ticket per split output. Tickets carry a revocation fee limit of 2^14 atoms and a vote fee limit of zero (limit flag unset: no fee may be taken from a vote).
- VSP registration is an HTTP/JSON client (`/api/v3/{vspinfo,feeaddress,payfee,ticketstatus,setvotechoices}`) with secp256k1 compact message signatures on requests and Ed25519 signatures on responses. Each ticket being registered has one in-memory *fee payment job* with a jittered single-shot timer; the only persisted state is a 40-byte record per ticket in the `vsp` bucket plus host and pubkey lookup buckets.
- Fee payment jobs are **not** resumed automatically on restart. They are recreated only by `--vsp.sync` at startup or by explicit RPCs.
- Votes are created only when voting is enabled and the network backend is the dcrd JSON-RPC syncer (SPV voting is rejected at config time). The wallet builds the vote, appends an optional treasury-vote output, signs input 1, publishes, then records.
- The wallet no longer creates revocations. `RevokeOwnedTickets` is a no-op kept for API compatibility (DCP0009 automatic revocations).
- The ticket buyer runs once per main-chain tip change, cancels in-flight purchases two blocks before a stake difficulty window boundary (and again on the first block of the new window, which cancels the purchases started on the last block of the old one), sets ticket expiry to the window boundary, and when mixing launches `Limit` concurrent single-ticket purchases.
- Several behaviours are bugs or accidents that a drop-in port must either reproduce or consciously fix: see section 10 (54 items).

---

## 2. Role in the system

This subsystem turns spendable wallet funds into tickets, registers tickets with a Voting Service Provider, casts votes for tickets the wallet itself has voting rights for, and automates purchasing.

**Called by**

- JSON-RPC handlers `purchaseticket`, `processunmanagedticket`, `setvotechoice`, `settreasurypolicy`, `settspendpolicy`, `setdisapprovepercent` (`internal/rpc/jsonrpc/methods.go:3356,3522,4780,3877,4008,3855`).
- gRPC `WalletService.PurchaseTickets`, `GetVSPTicketsByFeeStatus`, `SyncVSPFailedTickets`, `ProcessManagedTickets`, `ProcessUnmanagedTickets`, `SetVspdVoteChoices`, `GetTrackedVSPTickets`; `TicketBuyerService.RunTicketBuyer`; `AccountMixerService.RunAccountMixer` (which runs a `ticketbuyer.TB` with `BuyTickets=false`) (`internal/rpc/rpcserver/server.go:1791,4049,4079,4120,4150,4177,4269,2588,2531`).
- The main package at startup: creates the configured VSP client, starts the ticket buyer, and runs VSP sync (`dcrwallet.go:252-366,422-432`).
- The dcrd RPC syncer: `winningtickets` notifications call `VoteOnOwnedTickets`; `tspend` notifications and the initial `getrawmempool ... "tspend"` fill the tspend cache (`chain/sync.go:645-651,697-711,817-822,925-931`). The SPV syncer fills the tspend cache from `initstate` and from relayed transactions (`spv/sync.go:1014-1056,1147`).
- `ChainSwitch`, which removes pruned unmined transactions from every VSP client and from the `vsp` bucket (`wallet/chainntfns.go:261-274`).

**Calls**

- Transaction store and address manager (`udb.Store`, `udb.Manager`): UTXO enumeration, `RandomUTXO`, credit recording, private-key lookup, address derivation, `SetPublished`.
- `txauthor.NewUnsignedTransaction` for split-transaction input selection and change.
- The mixing client (`mixclient.Client.Dicemix`) for mixed splits.
- The network backend (`NetworkBackend.PublishTransactions`, `LoadTxFilter`, `StakeDifficulty`, `WrapContext`) and, when it implements `deployments.Querier`, dcrd's `getblockchaininfo` agenda status.
- The stake difficulty calculator (`NextStakeDifficulty`, `NextStakeDifficultyAfterHeader`).
- `blockchain/stake` (transaction classification, commitment parsing, reward calculation), `blockchain/standalone` (vote subsidy, tspend window), `txscript` (script building, signing, verification engine).
- HTTP(S) to the VSP, through the wallet's dial function when one is configured (`--proxy`), otherwise through Go's default dialer.

---

## 3. Public surface inventory

### 3.1 `wallet` package — ticket purchase (`wallet/wallet.go`, `wallet/createtx.go`)

| Item | Signature | Contract |
| --- | --- | --- |
| `PurchaseTicketsRequest` | struct, `wallet/wallet.go:1550` | `Count int`, `SourceAccount uint32`, `VotingAccount uint32`, `MinConf int32`, `Expiry int32`, `DontSignTx bool`, `Mixing bool`, `MixedAccount uint32`, `MixedAccountBranch uint32`, `MixedSplitAccount uint32`, `ChangeAccount uint32`, `VSPClient *VSPClient`, unexported `extraSplitOutput *Input`. **The request is mutated**: `Count`, `MinConf` and `extraSplitOutput` are written during the call. |
| `PurchaseTicketsResponse` | struct, `wallet/wallet.go:1578` | `TicketHashes []*chainhash.Hash`, `Tickets []*wire.MsgTx`, `SplitTx *wire.MsgTx`. May be returned non-nil together with a non-nil error (partial progress). |
| `(*Wallet).PurchaseTickets` | `(ctx, n NetworkBackend, req *PurchaseTicketsRequest) (*PurchaseTicketsResponse, error)` | Entry point. Wraps `purchaseTickets` with the UTXO-split retry (section 5.2). |
| `(*Wallet).purchaseTickets` | `(ctx, op errors.Op, n NetworkBackend, req) (*PurchaseTicketsResponse, error)` | The purchase algorithm (section 5.3). |
| `(*Wallet).individualSplit` | `(ctx, req, neededPerTicket dcrutil.Amount) (tx *wire.MsgTx, outIndexes []int, err error)` | Non-mixed split (section 5.4). |
| `(*Wallet).mixedSplit` | same shape | Mixed split via StakeShuffle (section 5.5). |
| `makeTicket` | `(params, input *Input, addrVote, addrSubsidy stdaddr.StakeAddress, ticketCost int64) (*wire.MsgTx, error)` | Builds the unsigned 1-in/3-out ticket (section 4.1). |
| `(*Wallet).ReserveOutputsForAmount` | `(ctx, account uint32, amount dcrutil.Amount, minconf int32) ([]Input, error)` | Selects eligible outputs summing to at least `amount` and inserts them into the in-memory locked-outpoint set. Caller must unlock. `errors.InsufficientBalance` (bare kind) when not enough. |
| `(*Wallet).reserveOutputs` | `(ctx, account, minconf) ([]Input, error)` | Locks and returns **every** eligible output of the account. |
| `(*Wallet).findEligibleOutputs` | `(dbtx, account, minconf, currentHeight) ([]Input, error)` | Full scan with the eligibility filter (section 5.6). Caller holds `lockedOutpointMu`. |
| `(*Wallet).findEligibleOutputsAmount` | `(dbtx, account, minconf, amount, currentHeight, minAmount, maxResults) ([]Input, error)` | Random-probe then shuffled-scan selection (section 5.6). Caller holds `lockedOutpointMu`. |
| `(*Wallet).signP2PKHMsgTx` | `(msgtx, prevOutputs []Input, addrmgrNs) error` | Signs every input as P2PKH, `SigHashAll`, compressed secp256k1 ECDSA. `len(prevOutputs) != len(TxIn)` is a plain error. An input whose previous script does not extract to exactly one address is **silently skipped** (left unsigned, no error); one address that is not a secp256k1 P2PKH address is `errors.Bug`. |
| `(*Wallet).CreateVspPayment` | `(ctx, tx *wire.MsgTx, fee dcrutil.Amount, feeAddr stdaddr.Address, feeAcct, changeAcct uint32) error` | Completes, signs and stores (unpublished) the VSP fee transaction (section 5.9). |
| `Input` | `struct{ OutPoint wire.OutPoint; PrevOut wire.TxOut }` | UTXO descriptor used throughout. |
| `revocationFeeLimit` | const `1 << 14` | Revocation fee limit committed to in every ticket. |
| `sanityVerifyFlags` | const | Script flags used to verify wallet-signed inputs: `ScriptDiscourageUpgradableNops \| ScriptVerifyCleanStack \| ScriptVerifyCheckLockTimeVerify \| ScriptVerifyCheckSequenceVerify \| ScriptVerifyTreasury`. |
| `errVSPFeeRequiresUTXOSplit` | var | Sentinel, message `paying VSP fee requires UTXO split`. Escapes `PurchaseTickets` only when `DontSignTx` is set, or when the second `purchaseTickets` call made after the UTXO split returns it again (5.2 step 12). |
| `p2pkhSizedScript` | var, 25 zero bytes | Placeholder script for sizing mixed outputs. |
| `stakeAddrFunc` | closure in `purchaseTickets`, `(op, account, branch uint32) (stdaddr.StakeAddress, uint32, error)` (`wallet/createtx.go:1187-1205`) | `nextAddress` with gap policy *ignore* and an empty account name; returns the address and, when it is an xpub-derived address, its child index (else 0). A result that is not a `stdaddr.StakeAddress` is `errors.Invalid`, `account does not return compatible stake addresses`. |
| `validateMsgTx`, `validateTxInput` | `(op, tx, prevScripts [][]byte) error`, `(op, tx, i int, prevScript []byte) error` (`wallet/createtx.go:728-757`) | Run the script engine over input `i` with `sanityVerifyFlags` and script version 0. Engine construction failure is `errors.E(op, err)`; execution failure is `errors.E(op, errors.ScriptFailure, err)` after logging (error level) `Script validation failed (outpoint <op> pkscript <hex> sigscript <hex>): <err>`. `validateMsgTx` checks input `i` against `prevScripts[i]` for each supplied script and stops at the first failure. |
| `creditScripts` | `(credits []Input) [][]byte` (`wallet/createtx.go:759-765`) | The `PrevOut.PkScript` of each input, in order. |
| `(*Wallet).checkHighFees` | `(totalInput dcrutil.Amount, tx) error` (`wallet/createtx.go:319-327`) | nil when the wallet allows high fees; else `errors.E(errors.Policy, "high fee")` when `txrules.PaysHighFees(totalInput, tx)`. |
| `confirms`, `confirmed` | `(txHeight, curHeight int32) int32`, `(minconf, txHeight, curHeight int32) bool` (`wallet/wallet.go:4404-4418`) | `confirms` is 0 when `txHeight == -1` or `txHeight > curHeight`, else `curHeight - txHeight + 1`; `confirmed` is `confirms >= minconf`. |
| `coinbaseMatured` | `(params, txHeight, curHeight int32) bool` (`wallet/wallet.go:4420-4424`) | `txHeight >= 0 && curHeight - txHeight + 1 > CoinbaseMaturity`. |
| `ticketChangeMatured` | same shape (`wallet/wallet.go:4426-4431`) | `txHeight >= 0 && curHeight - txHeight + 1 > SStxChangeMaturity`. |
| `ticketMatured` | same shape (`wallet/wallet.go:4433-4440`) | `txHeight >= 0 && curHeight - txHeight > TicketMaturity` (one block later than the parameter suggests, matching dcrd). Not used by the purchase path; used by ticket status and balance code. |
| `ticketExpired` | same shape (`wallet/wallet.go:4442-4447`) | `txHeight >= 0 && curHeight - txHeight > TicketMaturity + TicketExpiry`. Note the strict `>`: `VSPTicket.Expired` (3.4) uses `tip >= minedHeight + TicketMaturity + 1 + TicketExpiry`, which is the same boundary. |

### 3.2 `wallet` package — voting (`wallet/chainntfns.go`, `wallet/createtx.go`, `wallet/wallet.go`)

| Item | Signature | Contract |
| --- | --- | --- |
| `(*Wallet).VoteOnOwnedTickets` | `(ctx, winningTicketHashes []*chainhash.Hash, blockHash *chainhash.Hash, blockHeight int32) error` | Creates, signs, publishes and records votes (section 5.11). |
| `(*Wallet).RevokeOwnedTickets` | `(ctx, missedTicketHashes []*chainhash.Hash) error` | Always returns nil. Deprecated no-op. |
| `selectOwnedTickets` | `(w, dbtx, tickets) []*chainhash.Hash` | Filters to hashes for which `txStore.OwnTicket` is true, preserving input order. |
| `createUnsignedVote` | `(ticketHash, ticketPurchase *wire.MsgTx, blockHeight int32, blockHash, voteBits stake.VoteBits, subsidyCache, params, dcp0010Active, dcp0012Active bool) (*wire.MsgTx, error)` | Builds the vote (section 4.2). |
| `newVoteScript` | `(voteBits stake.VoteBits) ([]byte, error)` | `OP_RETURN <LE16 bits \|\| extended bits>`. |
| `(*Wallet).signVote`, `signVoteOrRevocation` | `(addrmgrNs, ticketPurchase, tx *wire.MsgTx[, isVote bool]) error` | Signs input 1 (vote) or input 0 (revocation; unreachable) against `ticketPurchase.TxOut[0].PkScript`. |
| `(*Wallet).hasVotingAuthority` | `(addrmgrNs, ticketPurchase) (mine, havePrivKey bool, err error)` | First hash160 address of ticket output 0 that the address manager knows. |
| `(*Wallet).VotingEnabled` | `() bool` | Config flag, under `stakeSettingsLock`. |
| `(*Wallet).VoteBits` | `() stake.VoteBits` | Cached default vote bits. |
| `(*Wallet).AgendaChoices` | `(ctx, ticketHash *chainhash.Hash) (map[string]string, uint16, error)` | Agenda ID to choice ID for the wallet's stake version, per ticket when preferences are saved for it, else defaults. |
| `(*Wallet).SetAgendaChoices` | `(ctx, ticketHash, choices map[string]string) (uint16, error)` | Persists choices; updates cached default vote bits when `ticketHash` is nil. |
| `(*Wallet).TSpendPolicy` | `(tspendHash, ticketHash *chainhash.Hash) stake.TreasuryVoteT` | Policy resolution order at the end of section 5.11. |
| `(*Wallet).TreasuryKeyPolicy` | `(pikey []byte, ticket *chainhash.Hash) stake.TreasuryVoteT` | Map lookup only. |
| `(*Wallet).TreasuryKeyPolicies` | `() []TreasuryKeyPolicy` | Global then per-ticket entries, each in map order. |
| `(*Wallet).SetTreasuryKeyPolicy`, `SetTSpendPolicy` | `(ctx, key, policy stake.TreasuryVoteT, ticketHash *chainhash.Hash) error` | Validate policy in {0,1,2}; write DB then memory; policy 0 deletes. |
| `(*Wallet).AddTSpend`, `IsTSpendCached`, `GetAllTSpends` | see `wallet/wallet.go:249-289` | In-memory tspend cache keyed by tx hash. `GetAllTSpends` evicts entries with `tipHeight > Expiry`. |
| `(*Wallet).DisapprovePercent`, `SetDisapprovePercent` | atomic `uint32` | Non-mainnet random block disapproval. |
| `voteVersion` | `(params) uint32` | mainnet 11, testnet2 (net `0x48e7a065`) 6, testnet3 12, simnet 12, otherwise 1 (`wallet/wallet.go:291-304`). |
| `CurrentAgendas` | `(params) (version uint32, agendas []chaincfg.ConsensusDeployment)` | `params.Deployments[voteVersion]`; `(version, nil)` when `params.Deployments` is nil. |
| `TreasuryKeyPolicy` (type) | `struct{ PiKey []byte; Ticket *chainhash.Hash; Policy stake.TreasuryVoteT }` (`wallet/wallet.go:633-637`) | Element of `TreasuryKeyPolicies()`. `Ticket` is nil for a global policy and points to a private copy of the ticket hash for a per-ticket policy. `PiKey` is the raw key bytes (the map key converted back from a string). |
| `(*Wallet).readDBVoteBits` | `(dbtx) stake.VoteBits` (`wallet/wallet.go:316-344`) | Default vote bits: `Bits = 0x0001`, `ExtendedBits` = 4-byte LE vote version; for each agenda with a saved default preference whose choice ID matches, OR in the choice bits. A saved choice ID that matches no choice of the agenda is ignored. |
| `(*Wallet).readDBTicketVoteBits` | `(dbtx, ticketHash) (stake.VoteBits, bool)` (`wallet/wallet.go:346-375`) | Same from the per-ticket preferences; the bool is true when at least one per-ticket preference (even one whose choice ID no longer matches) is saved. `false` when the network has no agendas for the vote version. |
| `(*Wallet).readDBTreasuryPolicies`, `readDBTreasuryKeyPolicies` | `(dbtx) (global map, perTicket map, error)` (`wallet/wallet.go:377-395`) | Load `tspendpolicy`+`vsptspendpolicy` and `treasurypolicy`+`vsptreasurypolicy` (4.5). If the first bucket fails both results are nil. |
| `(*Wallet).ManualTickets` | `() bool` (`wallet/wallet.go:5330-5336`) | Config flag `--manualtickets`. When set, the syncers drop every relayed (unmined) ticket transaction instead of adding it (`chain/sync.go:919`, `spv/sync.go:1155`), and a ticket in an attached block is recorded only if its hash is already in the transaction store (`wallet/chainntfns.go:61-69`). This keeps the wallet from voting on tickets bought by others that reuse one of its voting addresses. Tickets purchased through this subsystem are recorded directly by `purchaseTickets` (5.3 step 11.6) and are therefore unaffected. |

### 3.3 `wallet` package — VSP client (`wallet/vsp.go`, `wallet/vspclientloader.go`)

| Item | Signature | Contract |
| --- | --- | --- |
| `DialFunc` | `func(ctx, network, addr string) (net.Conn, error)` | Dialer used for VSP HTTP connections. |
| `VSPPolicy` | `struct{ MaxFee dcrutil.Amount; ChangeAcct uint32; FeeAcct uint32 }` | Fee cap; account for fee-tx change addresses; account fee inputs are taken from when none were provided. |
| `VSPClientConfig` | `struct{ URL string; PubKey string; Policy *VSPPolicy }` | `PubKey` is standard base64 of the VSP's Ed25519 public key. |
| `VSPClient` | struct embedding `*vspd.Client` | Fields `wallet`, `policy`, `mu sync.Mutex`, `jobs map[chainhash.Hash]*vspFeePayment`, `log`. The embedded client's exported fields and methods are promoted. |
| `(*Wallet).NewVSPClient` | `(cfg, log slog.Logger, dialer DialFunc) (*VSPClient, error)` | Validates and constructs; does not register in the wallet map. Errors: `vsp url can not be null`, `vsp pubkey can not be null`, URL parse error, base64 error. |
| `(*Wallet).VSP` | `(cfg) (*VSPClient, error)` | Get-or-create keyed by the **raw** `cfg.URL`. An existing client is returned unchanged; `cfg.PubKey` and `cfg.Policy` are ignored in that case. |
| `(*Wallet).LookupVSP` | `(host string) (*VSPClient, error)` | `errors.NotExist` wrapping ``VSP client for "<host>" not found``. |
| `(*Wallet).AllVSPs` | `() map[string]*VSPClient` | Shallow copy. |
| `(*Wallet).VSPMaxFee` | `() dcrutil.Amount` | Config value. |
| `(*VSPClient).FeePercentage` | `(ctx) (float64, error)` | `GET /api/v3/vspinfo`, returns `feepercentage`; `-1` on error. |
| `(*VSPClient).Process` | `(ctx, ticket *VSPTicket, feeTx *wire.MsgTx) error` | Synchronous first pass of fee payment (section 5.8.3). |
| `(*VSPClient).ProcessUnprocessedTickets` | `(ctx, tickets []*VSPTicket)` | One goroutine per ticket not already in `jobs`, each calling `Process(ctx, t, nil)`; waits for all; errors only logged. |
| `(*VSPClient).ProcessManagedTickets` | `(ctx, tickets []*VSPTicket) error` | Resume/recover from VSP-reported status (section 5.8.4). |
| `(*VSPClient).SetVoteChoice` | `(ctx, ticket, choices, tspendPolicy, treasuryPolicy map[string]string) error` | Compare with VSP's stored preferences, update only on mismatch (section 5.10). |
| `(*VSPClient).RemoveTicket` | `(ticketHash chainhash.Hash, reason string)` | Stops and removes the job if present. |
| `(*VSPClient).TrackedTickets` | `() []*VSPTicketInfo` | Snapshot of jobs, in map iteration order. |
| `VSPTicketInfo` | `struct{ TicketHash; CommitmentAddr, VotingAddr stdaddr.StakeAddress; State State; Fee dcrutil.Amount; FeeHash chainhash.Hash }` | |
| `State` | `uint32` | `Unprocessed = 1`, `FeePublished = 2`, `TicketSpent = 4`; 0 and 3 are unnamed (`wallet/feepayment.go:61-69`). |

### 3.4 `wallet` package — `VSPTicket` (`wallet/vspticket.go`)

| Item | Signature | Contract |
| --- | --- | --- |
| `VSPTicket` | struct (`wallet/vspticket.go:21-36`) | Immutable after construction: `hash *chainhash.Hash` (the caller's pointer, not copied), `rawTx`, `parentTx *wire.MsgTx`, `commitmentAddr`, `votingAddr stdaddr.StakeAddress`, `votingKey string` (WIF), `wallet *Wallet`. Mutable under `mu`: `live`, `expiry int32` (0 until the ticket is seen mined). |
| `(*VSPTicket).calcHeights` | `(ctx)`, caller holds `mu` (`wallet/vspticket.go:110-127`) | Reads the mined height; on a lookup error logs (wallet logger, error level) `Failed to query block which mines ticket: <err>` and leaves both heights 0; leaves them 0 when `minedHeight < 2`; otherwise sets `live` and `expiry` as below. Called again on every `LiveHeight`/`ExpiryHeight` call while the cached value is 0. |
| `(*Wallet).NewVSPTicket` | `(ctx, hash *chainhash.Hash) (*VSPTicket, error)` | Loads ticket and parent tx, extracts voting and commitment addresses, exports the voting key as WIF (section 5.7). |
| `Hash`, `RawTx`, `ParentTx`, `CommitmentAddr`, `VotingAddr`, `VotingKey`, `String` | accessors | Immutable after construction. `String` is the ticket hash string. |
| `LiveHeight`, `ExpiryHeight` | `(ctx) int32` | Lazily computed and cached once non-zero: `live = minedHeight + TicketMaturity + 1`, `expiry = live + TicketExpiry`; 0 while unmined (`minedHeight < 2`). |
| `Expired` | `(ctx) bool` | `expiry > 0 && tipHeight >= expiry`. |
| `Spent` | `(ctx) bool` | True iff `txStore.Spender` of outpoint `(hash, 0, tree 1)` returns no error. |
| `TxBlock` | `(ctx) (int32, error)` | Height mining the ticket, `-1` if unmined. |
| `AgendaChoices` | `(ctx) (map[string]string, error)` | `Wallet.AgendaChoices(ctx, hash)` without the vote bits. |
| `TSpendPolicy` | `() map[string]string` | Per-ticket tspend policies only; key is the tspend hash string; values `yes`/`no`/`abstain`. |
| `TreasuryKeyPolicy` | `() map[string]string` | Per-ticket treasury-key policies only; **key is the raw 33 key bytes cast to a string, not hex** (quirk Q9). |
| `UpdateFeeStarted/Paid/Confirmed` | `(ctx, feeHash chainhash.Hash, host string, pubkey []byte) error` | One DB update writing the `vsp` record with status 0/1/3. |
| `UpdateFeeErrored` | `(ctx, host string, pubkey []byte) error` | Writes status 2 and an all-zero fee hash. |
| `FeeHash` | `(ctx) (chainhash.Hash, error)` | `Wallet.VSPFeeHashForTicket`; `errors.NotExist` when no record or the stored hash is all zero. |
| `FeeTx` | `(ctx) (*wire.MsgTx, error)` | Looks the fee hash up in the transaction store. |
| `VSPTicketInfo` | `(ctx) (*TicketInfo, error)` | DB record with host and pubkey resolved. |
| `TicketInfo` | `struct{ FeeHash; FeeTxStatus uint32; VSPHostID uint32; Host string; PubKey []byte }` | |

### 3.5 `wallet` package — fee payment job (`wallet/feepayment.go`, all unexported)

| Item | Contract |
| --- | --- |
| `vspFeePayment` | Per-ticket job: `client`, `ctx` (always `context.Background()`), `ticket`, `policy`, `params`; under `mu`: `fee`, `feeAddr`, `feeHash`, `feeTx`, `state`, `err`; under `timerMu`: `timer`. |
| `(*VSPClient).feePayment(ctx, ticket, paidConfirmed bool) *vspFeePayment` | Get-or-create the job and schedule `reconcilePayment` on creation (section 5.8.2). Never returns nil. |
| `schedule(name, method)` | Replace the job's single timer; `method == nil` cancels. |
| `next() time.Duration` | Uniform random delay in `[0, jitter)` (section 5.8.1). |
| `task(name, method) func()` | Runs `method`, stores its error in `fp.err`, logs `Ticket <hash>: <name>: <err>` at error level. |
| `stop()`, `remove(reason)`, `removedExpiredOrSpent()` | Cancel timer; cancel + log + delete from `jobs`; remove with reason `expired` or `spent`. |
| `receiveFeeAddress()`, `makeFeeTx(tx)`, `submitPayment()`, `confirmPayment()`, `reconcilePayment()` | Job steps (section 5.8). |
| `(*VSPClient).status(ctx, ticket)`, `setVoteChoices(...)` | Thin wrappers over `ticketstatus` and `setvotechoices`. |
| `marshalTx(tx) (string, error)` | Lower-case hex of the full wire serialization. |
| `errStopped` | `fee processing stopped`. |
| `immatureJitter`, `liveJitter`, `unminedJitter` | 1 h, 5 min, 2 min. |

### 3.6 `wallet` package — VSP ticket queries (`wallet/wallet.go:5611-5828`)

| Item | Contract |
| --- | --- |
| `GetVSPTicketsByFeeStatus(ctx, feeStatus int) ([]chainhash.Hash, error)` | All ticket hashes whose record has that status; order is Go map order. |
| `SetPublished(ctx, hash, published bool) error` | Toggles the transaction store's unpublished marker. |
| `VSPFeeHashForTicket(ctx, ticketHash) (chainhash.Hash, error)` | `NotExist` if no record **or** fee hash is zero. |
| `VSPHostForTicket(ctx, ticketHash) (string, error)` | `NotExist` if no record **or** host is empty. |
| `IsVSPTicketConfirmed(ctx, ticketHash) (bool, error)` | Status equals 3. `NotExist` if no record. |
| `ForUnspentUnexpiredTickets(ctx, f) error` | Calls `f` for every ticket whose `TicketSummary.Status` is live, immature or unspent among tickets mined in heights `[tip - (TicketExpiry + TicketMaturity - 8), tip]` (`GetTickets` with height identifiers, `wallet/wallet.go:5734-5741`). That range holds only when the lower bound is non-negative. When `tip < TicketExpiry + TicketMaturity - 8` (never on a synced mainnet or testnet3 wallet; simnet below height 392) the start height is negative, which the transaction store treats as the unmined marker: unmined transactions are visited first, the block iteration then starts from the highest recorded block and runs **backwards** while `height >= tip`, so only tickets mined in the tip block are visited (`wallet/udb/txquery.go:392-420,521-537`). Because `GetTickets` passes no RPC client, mined mature unspent tickets have status *unspent* and *live* never occurs; **unmined tickets have status unmined and are skipped** (`wallet/wallet.go:2899-2908`). Errors from `f` do not stop iteration; all are combined with `errors.Join` (nil when none). |
| `UnprocessedTickets(ctx) ([]*VSPTicket, error)` | Tickets from the above with no non-zero fee hash and not confirmed. |
| `ProcessedTickets(ctx) ([]*VSPTicket, error)` | Tickets from the above that have a `vsp` record and are not confirmed. |

### 3.7 `udb` package — VSP persistence (`wallet/udb/vsp.go`, `wallet/udb/treasury.go`)

| Item | Contract |
| --- | --- |
| `FeeStatus` (`int`) | `VSPFeeProcessStarted = 0`, `VSPFeeProcessPaid = 1`, `VSPFeeProcessErrored = 2`, `VSPFeeProcessConfirmed = 3`. |
| `VSPTicket` | `FeeHash chainhash.Hash`, `FeeTxStatus uint32`, `VSPHostID uint32`, `Host string`, `PubKey []byte` (last two are not serialized in the ticket record). |
| `SetVSPTicket(dbtx, ticketHash, record) error` | Resolves or allocates the host ID, then writes the record (section 4.4). Mutates `record.VSPHostID`. |
| `GetVSPTicket(dbtx, hash) (*VSPTicket, error)` | `NotExist` with message `no VSP info for ticket <hash>` when absent. |
| `DeleteVSPTicket(dbtx, hash) error` | Deletes the key; absent key is not an error. |
| `GetVSPTicketsByFeeStatus(dbtx, feeStatus int) (map[chainhash.Hash]*VSPTicket, error)` | Full bucket scan. Host and pubkey are not resolved. |
| `VSPHost{Host []byte}`, `SetVSPHost`, `GetVSPHost` | Host record by numeric ID. |
| `VSPPubKey{ID uint32; PubKey []byte}`, `SetVSPPubKey`, `GetVSPPubKey` | Pubkey record by host string. |
| `VSPTSpend{Ticket, TSpend chainhash.Hash}`, `VSPTreasuryKey{Ticket chainhash.Hash; TreasuryKey string}` | Map keys for per-ticket policies. |
| `SetTSpendPolicy`, `TSpendPolicy`, `TSpendPolicies`, `SetVSPTSpendPolicy`, `VSPTSpendPolicy`, `VSPTSpendPolicies`, `SetTreasuryKeyPolicy`, `TreasuryKeyPolicy`, `TreasuryKeyPolicies`, `SetVSPTreasuryKeyPolicy`, `VSPTreasuryKeyPolicy`, `VSPTreasuryKeyPolicies` | CRUD over the four policy buckets (section 4.5). |
| `vspBucketKey`, `vspHostBucketKey`, `vspPubKeyBucketKey` | vars: `vsp`, `vsphost`, `vsppubkey` (`wallet/udb/vsp.go:13-17`). |
| `tspendPolicyBucketKey`, `treasuryPolicyBucketKey`, `vspTspendPolicyBucketKey`, `vspTreasuryPolicyBucketKey` | vars: `tspendpolicy`, `treasurypolicy`, `vsptspendpolicy`, `vsptreasurypolicy` (`wallet/udb/treasury.go:14-20`). |
| `rootVSPHostIndex` | var: `vsphostindex` (`wallet/udb/txdb.go:162`), the host-ID counter key inside `vsphost`. |
| `hashSize` | const 32 (`wallet/udb/vsp.go:19`). |
| `serializeVSPTicket` / `deserializeVSPTicket`, `serializeVSPHost` / `deserializeVSPHost`, `serializeVSPPubKey` / `deserializeVSPPubKey` | Unexported codecs for the three records (4.4). The deserializers do no length validation: a short value panics with a slice-bounds error; trailing bytes are ignored. `deserializeVSPHost` and `deserializeVSPPubKey` return sub-slices of the database value (valid only inside the transaction; `GetVSPTicket` copies the host through a `string` conversion but hands the pubkey sub-slice to its caller). |

Per-function details not covered in 4.4 and 4.5:

- The `Set*Policy` functions wrap a failed **delete** as `errors.E(errors.IO, err)` but return a failed **put** unwrapped (`wallet/udb/treasury.go:38-44` and the three siblings). `SetVSPTicket` wraps only the failed write of `vsphostindex` as `errors.IO`; the host, pubkey and ticket puts are returned unwrapped (`wallet/udb/vsp.go:56-87`).
- The four `*Policies` loaders iterate the bucket with `ForEach`, re-read each key through the single-key getter, and **omit** entries whose stored byte is `0x00` (they are not deleted). Any other stored byte value (for example `0x03`) is loaded as is.
- `GetVSPTicketsByFeeStatus` ignores the error of `chainhash.Hash.SetBytes` on the key; a key that is not 32 bytes yields the zero hash.
- `SetVSPTicket` treats *any* `NotExist` from `GetVSPPubKey` as "new host"; other errors are returned. It dereferences `vsphost["vsphostindex"]` without a nil check, so a database missing that key (only possible if the version-23 upgrade did not run) panics.
### 3.8 `ticketbuyer` package

| Item | Signature | Contract |
| --- | --- | --- |
| `Config` | struct, `ticketbuyer/tb.go:18` | `BuyTickets bool`, `Account uint32`, `VotingAccount uint32`, `Maintain dcrutil.Amount`, `Limit int`, `Mixing bool`, `MixedAccount uint32`, `MixedAccountBranch uint32`, `TicketSplitAccount uint32`, `ChangeAccount uint32`, `MixChange bool`, `VSP *wallet.VSPClient`. |
| `TB` | struct | `wallet`, `cfg`, `mu`. |
| `New` | `(w *wallet.Wallet, cfg Config) *TB` | |
| `(*TB).Run` | `(ctx, passphrase []byte) error` | Main loop (section 5.13). Returns `ctx.Err()` on cancellation, or the passphrase error that stopped it. |
| `(*TB).AccessConfig` | `(f func(cfg *Config))` | Runs `f` with the config under the mutex. |
| `(*TB).buy`, `(*TB).mixChange` | unexported | One purchase attempt; one change-account mix. |
| `UseLogger` | `(logger slog.Logger)` | Package logger, disabled by default; the main package wires subsystem tag `TKBY` (`log.go:34,51`). |
| `log` | package var, `slog.Disabled` until `UseLogger` (`ticketbuyer/log.go:12`) | Not synchronised; set once at startup. |

`AccessConfig` has no caller anywhere in dcrwallet at this pin (the main package and both gRPC services construct a `TB` and only call `Run`); the buyer's configuration is therefore never changed after `New` in practice. `buy` and `mixChange` each open a Go runtime trace task (`ticketbuyer.buy`, `ticketbuyer.mixChange`); these are visible only in execution traces and need no equivalent.

Callers of `ticketbuyer.New`: the main package (5.13 startup wiring), gRPC `TicketBuyerService.RunTicketBuyer` and gRPC `AccountMixerService.RunAccountMixer`, which runs a buyer with `{Mixing: true, MixChange: true, BuyTickets: false, MixedAccount, MixedAccountBranch, ChangeAccount}` and every other field zero (`internal/rpc/rpcserver/server.go:2531-2566`).
### 3.9 `vspd/client/v4` and `vspd/types/v3`

| Item | Contract |
| --- | --- |
| `client.Client` | `struct{ http.Client; URL string; PubKey []byte; Sign func(ctx, string, stdaddr.Address) ([]byte, error); Log slog.Logger }`. |
| `VspInfo(ctx)`, `FeeAddress(ctx, req, commitmentAddr)`, `PayFee(...)`, `TicketStatus(...)`, `SetVoteChoices(...)` | One HTTP call each (section 4.6). |
| `ValidateServerSignature(resp *http.Response, body, serverPubkey []byte) error` | Ed25519 check of the `VSP-Server-Signature` header over the raw body. |
| `types.ErrorResponse{Code ErrorCode; Message string}` | Implements `error`; `Error()` returns `Message` only. |
| `types.ErrorCode` (`int64`) with `HTTPStatus()`, `DefaultMessage()` | 18 codes (section 8.3). |
| `types.VspInfoResponse`, `FeeAddressRequest/Response`, `PayFeeRequest/Response`, `SetVoteChoicesRequest/Response`, `TicketStatusRequest/Response`, `SetAltSignAddrRequest/Response` | JSON shapes (section 4.6). `SetAltSignAddr*` is defined but no client method uses it. |
| `(*Client).post`, `get`, `do` | Unexported. `post(ctx, path, addr, resp, req)` and `get(ctx, path, resp)` call `do(ctx, method, path, addr, resp, req)`, which implements 4.6.1 and 4.6.2 (`vspd/client/v4@v4.0.2/client.go:156-256`). `resp` is a pointer to a nil pointer of the response type; `encoding/json` allocates it. |

Every endpoint method takes its request struct **by value**, so the nil-map replacement in `PayFee` and `SetVoteChoices` does not change the caller's struct. The endpoint methods return the `json.Marshal` error of the request unwrapped (cannot occur for these types), and compare `resp.Request` with the request bytes only after `post` succeeded.
### 3.10 `blockchain/stake/v5` symbols the wallet depends on

Every use in dcrwallet outside tests: `TxType` and constants `TxTypeRegular=0, TxTypeSStx=1, TxTypeSSGen=2, TxTypeSSRtx=3, TxTypeTAdd=4, TxTypeTSpend=5, TxTypeTreasuryBase=6` (the last three are never referenced by name in dcrwallet but are possible return values of `DetermineTxType`); `DetermineTxType`; `IsSStx`, `CheckSStx`, `IsSSGen`, `IsSSRtx`, `IsTSpend`, `CheckTSpend`; `VoteBits{Bits uint16; ExtendedBits []byte}`; `TreasuryVoteT` with `TreasuryVoteInvalid=0x00, TreasuryVoteYes=0x01, TreasuryVoteNo=0x02`; `AddrFromSStxPkScrCommitment`, `AmountFromSStxPkScrCommitment`; `TxSStxStakeOutputInfo`; `SStxNullOutputAmounts`; `CalculateRewards`; `SSGenBlockVotedOn`; `IsStakeCommitmentTxOut`. All of these exist in `dcroxide:crates/dcroxide-stake/src/lib.rs` (section 12). The ticket database half of the package (`tickets.go`, `internal/ticketdb`, `internal/tickettreap`) is not used by the wallet.

`DetermineTxType` order is observable (a transaction satisfying more than one form is classified by the first match): ticket, vote, revocation, then only if `tx.Version >= 3`: treasury add, treasury spend, treasurybase; otherwise regular (`dcrd/blockchain/stake/v5@v5.0.3/staketx.go:1272-1294`).

#### 3.10.1 Contracts of the stake helpers (`dcrd/blockchain/stake/v5@v5.0.3`)

The port takes these from the dcroxide stake crate; the contracts are restated here because the wallet's observable behaviour (which transactions it calls tickets or votes, which tickets a VSP can be given, what a vote pays) is defined by them. All predicates are purely structural; none looks at signatures or amounts unless stated. Failures are `stake.RuleError{Err ErrorKind, Description string}` values (`error.go:317-330`); the wallet only tests them for nil, except where a message is quoted below.

Script predicates (`scripttype.go:13-82`). *Tagged script* with opcode `op`: script version 0, first byte `op`, and the remainder is exactly a 25-byte P2PKH script (`76 a9 14 <20> 88 ac`) or a 23-byte P2SH script (`a9 14 <20> 87`).

| Function | Tag opcode |
| --- | --- |
| `IsTicketPurchaseScript` | `OP_SSTX` (`0xba`) |
| `IsVoteScript` | `OP_SSGEN` (`0xbb`) |
| `IsRevocationScript` | `OP_SSRTX` (`0xbc`) |
| `IsStakeChangeScript` | `OP_SSTXCHANGE` (`0xbd`) |
| `IsTreasuryGenScript` | `OP_TGEN` (`0xc3`) |

`IsNullDataScript(version, script)` (`staketx.go:269-296`): version 0; `script[0] == OP_RETURN`; and either the script is that single byte, or the rest is exactly one opcode that is a small integer (`OP_0`, `OP_1`..`OP_16`) or a data push (opcode `<= OP_PUSHDATA4`) carrying at most `MaxDataCarrierSize = 256` bytes.

`CheckSStx(tx)` / `IsSStx(tx)` (`staketx.go:713-818`), in this order; the first failing rule is the error:

1. `len(TxIn) <= 64` (`MaxInputsPerSStx`).
2. `len(TxOut) <= 129` (`MaxOutputsPerSStx`).
3. `len(TxOut) != 0`.
4. Every output has script version 0.
5. Output 0 is a ticket-purchase tagged script.
6. `len(TxIn) * 2 + 1 == len(TxOut)`.
7. For every output index `i >= 1`: even `i` must be a stake-change tagged script; odd `i` must be a null-data script whose total script length is in `[32, 77]`, with `script[0] == OP_RETURN` and `script[1]` (the push opcode) in `[OP_DATA_30, OP_DATA_75]`.

No rule constrains the transaction version, the input outpoints or trees, the values, or the expiry.

`TxSStxStakeOutputInfo(tx)` (`staketx.go:346-413`) returns six parallel slices of length `len(TxOut) / 2`, one entry per odd output `i` (entry `i/2`), without validating anything (a script shorter than 32 bytes panics):

| Result | Derivation from the odd output's script |
| --- | --- |
| `isP2SH` | bit 7 of `script[29]` (the most significant bit of the little-endian amount) |
| `hash160` | `script[2:22]` (a sub-slice of the script) |
| `amount` | `script[22:30]` as `uint64` LE with bit 63 cleared |
| `changeAmount` | value of output `i + 1` |
| `spendRules` | `[limits & 0x0040 != 0, limits & 0x4000 != 0]` where `limits = uint16 LE script[30:32]` |
| `spendLimits` | `[limits & 0x003f, (limits & 0x3f00) >> 8]` |

`createUnsignedVote` uses only the first three.

`AddrFromSStxPkScrCommitment(script, params)` (`staketx.go:417-443`): requires `len(script) >= 32`, else rule error `ErrSStxBadCommitAmount`, `short read of sstx commit pkscript`; returns a P2SH address from `script[2:22]` when bit 7 of `script[29]` is set, else a P2PKH (secp256k1) address. `AmountFromSStxPkScrCommitment(script)` (`staketx.go:446-459`): same length check and error; `script[22:30]` as `uint64` LE with bit 63 cleared.

`SStxNullOutputAmounts(amounts, changeAmounts, amountTicket)` (`staketx.go:508-545`): plain error `amounts was not equal in length to change amounts!` when the slice lengths differ; rule error `ErrSStxBadCommitAmount`, `committed amount was too small!`, when `amountTicket <= 0`; `contrib[i] = amounts[i] - changeAmounts[i]`, a negative value is rule error `ErrSStxBadChangeAmts`, `change at idx <i> spent more coins than allowed (have: <amount>, spent: <change>)`; returns `(sum(contrib) - amountTicket, contrib, nil)`. `makeTicket` calls it with one amount (the split output value) and change 0, so a ticket price of zero or less makes `makeTicket` fail with the second error.

`CalculateRewards(contribAmounts, ticketPurchaseAmount, voteSubsidy)` (`staketx.go:563-630`): formula in 4.2. The division is by `sum(contribAmounts)`; a ticket whose commitments sum to zero makes `big.Int.Div` panic (division by zero). The results are not adjusted to sum to the total: the remainder of the truncating divisions is left as the vote's fee.

`SSGenBlockVotedOn(tx)` (`staketx.go:466-475`): `(hash = TxOut[0].PkScript[2:34], height = uint32 LE PkScript[34:38])`, no validation. `IsStakeCommitmentTxOut(index)` (`staketx.go:1302-1304`): `index % 2 != 0`.

`CheckSSGen(tx)` / `IsSSGen(tx)` (`staketx.go:990-1169`), in order:

1. Exactly 2 inputs.
2. `len(TxOut) <= 66` (`MaxOutputsPerSSGen = 64 + 2`; the limit is applied to the count including a treasury-vote output).
3. `len(TxOut) >= 2`.
4. Input 0 is a stakebase: previous outpoint hash all zero, index `0xffffffff`, tree regular, `BlockHeight == 0x00000000`, `BlockIndex == 0xffffffff`.
5. Input 1 has previous outpoint index 0 and tree stake.
6. Every output has script version 0.
7. Output 0 is a null-data script of exactly 38 bytes starting `6a 24`.
8. Output 1 is a null-data script of total length in `[4, 77]` with `script[0] == OP_RETURN` and `script[1]` in `[OP_DATA_2, OP_DATA_75]`.
9. If the **last** output is a null-data script it is taken as the treasury-vote output and must parse (`GetSSGenTreasuryVotes`, `staketx.go:872-950`): at least 4 bytes; `script[1] <= OP_PUSHDATA1`; payload starts at offset 2 (offset 3 for `OP_PUSHDATA1`) with the bytes `'T' 'V'`; the rest is a non-empty multiple of 33 bytes; every vote byte is `0x01` or `0x02`; no tspend hash appears twice. It must yield at least one vote **and** the transaction version must be 3, else `ErrSSGenInvalidTxVersion`. Because a null-data push holds at most 256 bytes, at most `(256 - 2) / 33 = 7` votes fit; an eighth vote makes the output fail `IsNullDataScript`, after which rule 10 rejects it.
10. Every remaining output from index 2 (excluding an accepted treasury-vote output) is a vote tagged script.

A vote with exactly two outputs passes rule 9's test on output 1 (it is the last output and is null data), which then fails to parse as treasury votes; such a transaction is not a vote. The wallet never builds one (a ticket always has at least one commitment).

`CheckSSRtx(tx)` / `IsSSRtx(tx)` (`staketx.go:1192-1268`): exactly 1 input; `1 <= len(TxOut) <= 64`; every output script version 0; the input's previous outpoint tree is stake; every output is a revocation tagged script; and when `tx.Version >= 2` (`TxVersionAutoRevocations`): input 0 has an empty signature script and `sum(output values) >= TxIn[0].ValueIn` (zero fee).

`CheckTSpend(tx)` / `IsTSpend(tx)` (`treasury.go:140-232`), returning `(signature, pubKey, error)`:

1. `tx.Version == 3`.
2. Exactly 1 input; at least 2 outputs.
3. Every output has script version 0 and a non-empty script.
4. Input 0's signature script is exactly 100 bytes (`TSpendScriptLen`): `[0] == OP_DATA_64 (0x40)`, `[65] == OP_DATA_33 (0x21)`, `[99] == OP_TSPEND (0xc2)`.
5. `signature = sigScript[1:65]`, `pubKey = sigScript[66:99]`; the key must be a strictly encoded compressed secp256k1 key (33 bytes, first byte `0x02` or `0x03`).
6. Output 0 is `OP_RETURN` followed by exactly one 32-byte data push (`txscript.IsStrictNullData(version, script, 32)`).
7. Every other output starts with `OP_TGEN` followed by exactly a P2PKH or P2SH script.

The wallet relies on rule 4 when it slices `SignatureScript[66:99]` in `TSpendPolicy` (5.11).

`Hash256PRNG` and the rest of `lottery.go`, `CalculateRevocationRewards`, `CreateRevocationFromTicket`, `FindSpentTicketsInBlock`, `SSGenVoteBits`, `SSGenVersion`, `CheckTAdd`/`IsTAdd`, `CheckTreasuryBase`/`IsTreasuryBase`, `CheckTreasuryVote`/`IsTreasuryVote`, `IsStakeBase`, `ConvertToMinimalOutputs`, `SStxStakeOutputInfo` (called only through `TxSStxStakeOutputInfo`) are not referenced by name from dcrwallet's non-test code; `IsTAdd` and `IsTreasuryBase` are reached only through `DetermineTxType`.

### 3.11 `deployments` package (in-scope part)

| Item | Contract |
| --- | --- |
| `Querier` | `interface{ Deployments(ctx) (map[string]dcrdtypes.AgendaInfo, error) }`. Implemented only by the dcrd RPC syncer (`chain/backend.go:63`). |
| `DCP0010Active(ctx, height int32, params, querier) (bool, error)` | simnet: true. Networks other than mainnet/testnet3: false. Otherwise query agenda `changesubsidysplit`: true if status is `active`, or `lockedin` with `height == Since + RuleChangeActivationInterval - 1`; false if the agenda is unknown. Nil querier: `errors.Bug`, `DCP0010 activation check requires RPC syncer`. |
| `DCP0012Active(...)` | Same with agenda `changesubsidysplitr2`; applies to mainnet, testnet3 **and simnet** (no simnet shortcut); other networks false. Nil querier: `errors.Bug`, `DCP0012 activation check requires RPC syncer`. |
| `lockedinStatus`, `activeStatus` | Unexported consts: dcrd's agenda status strings `lockedin` and `active` (`deployments/deployments.go:75-78`). Any other status (`defined`, `started`, `failed`) is "not active". |
| `HardcodedDeployment`, `DCP0001`, `DCP0002`, `DCP0003`, `(*HardcodedDeployment).Active` | Outside this spec's range of the file (lines 16-73) but used by this subsystem through the ticket price: `Active(height, net)` is `activationHeight >= 0 && height >= activationHeight` with the height chosen by network (mainnet, testnet2 `0x48e7a065`, testnet3, simnet; any other network is never active). `DCP0001` heights: mainnet 149248, testnet2 46128, testnet3 0, simnet 0. `NextStakeDifficulty` fails with kind `errors.Deployment` when `DCP0001` is not active at the tip height (`wallet/difficulty.go:797-798`), which is what triggers the `n.StakeDifficulty` RPC fallback in 5.3 step 4 and the "next ticket price is unknown" branch of the prune hook (5.15). |

Both activation checks compute `rcai = int32(params.RuleChangeActivationInterval)` and compare `height == int32(d.Since) + rcai - 1`, where `Since` is the `since` field of dcrd's `getblockchaininfo` agenda entry (the height at which the current status began). Each call issues its own `Deployments` request; no result is cached. A `Deployments` error is returned unwrapped.

### 3.12 `txrules` package (in-scope part, `wallet/txrules/poolfees.go`)

| Item | Contract |
| --- | --- |
| `ValidPoolFeeRate(feeRate float64) bool` | `floor(feeRate * 100)` (float64) must be in `[1.0, 10000.0]`, i.e. a percentage from 0.01 to 100.00 after truncation to two decimals. Not called from the purchase path or the VSP client (the VSP's `feepercentage` is used unvalidated); it remains exported API. Subject to the same float behaviour as `StakePoolTicketFee`: `0.01 * 100` is `1.0` in IEEE double arithmetic, so 0.01 is accepted. |
| `StakePoolTicketFee(stakeDiff, relayFee dcrutil.Amount, height int32, poolFee float64, params, dcp0010Active, dcp0012Active bool) dcrutil.Amount` | Section 5.14. |
| `subsidyCache`, `initSubsidyCacheOnce` | Package-level `*blockchain.SubsidyCache` and `sync.Once`. The cache is created on the first call with **that call's** `params` and reused for every later call, whatever network parameters are passed afterwards (a process serving one network is unaffected; tests mixing networks are). |

---

## 4. Data formats

All multi-byte integers inside transactions and scripts are little-endian. All integers inside wallet database records in this section are **big-endian** (`byteOrder = binary.BigEndian`, `wallet/udb/txdb.go:123`). Transaction hashes inside database keys and values are the 32 raw hash bytes in internal order (not the reversed display order).

### 4.1 Ticket purchase transaction (as built by `makeTicket`, `wallet/createtx.go:923-992`)

Transaction fields: `Version = 1`, `LockTime = 0`, `Expiry = uint32(req.Expiry)` (0 means no expiry), serialization type full.

**Input 0** (exactly one input):

| Field | Value |
| --- | --- |
| `PreviousOutPoint.Hash` | split transaction hash |
| `PreviousOutPoint.Index` | split output index |
| `PreviousOutPoint.Tree` | 0 (regular) |
| `Sequence` | `0xffffffff` |
| `ValueIn` | value of the split output |
| `BlockHeight` | `0x00000000` (`wire.NullBlockHeight`) |
| `BlockIndex` | `0xffffffff` (`wire.NullBlockIndex`) |
| `SignatureScript` | empty (zero length) until signed; then `<sig+hashtype> <compressed pubkey>` |

**Output 0 — voting rights (stake submission)**: `Value = ticketPrice`, `Version = 0`, 26-byte script

```
offset 0      0xba            OP_SSTX
offset 1      0x76            OP_DUP
offset 2      0xa9            OP_HASH160
offset 3      0x14            OP_DATA_20
offset 4..23  hash160 of the voting address public key
offset 24     0x88            OP_EQUALVERIFY
offset 25     0xac            OP_CHECKSIG
```

**Output 1 — reward commitment**: `Value = 0`, `Version = 0`, 32-byte script

```
offset 0      0x6a            OP_RETURN
offset 1      0x1e            OP_DATA_30
offset 2..21  hash160 of the commitment (reward) address
offset 22..29 uint64 LE: committed amount, bit 63 cleared (bit 63 set would mean P2SH)
offset 30..31 uint16 LE: fee limits = 0x4e00, i.e. bytes 00 4e
```

The committed amount is `SStxNullOutputAmounts([input value], [0], ticketPrice)` output 0, which is simply the input value (`ticketPrice + ticketFee`); the function's first return (fees) is discarded.

Fee limits encoding (`dcrd/txscript/v4@v4.1.2/stdaddr/addressv0.go:606-621`, written at `:645-651`): low byte is the vote fee limit, high byte the revocation fee limit; each is `0x40 | ceil(log2(limit))` when the limit is non-zero, else 0. The wallet passes `voteFeeLimit = 0`, `revocationFeeLimit = 16384`, giving low byte `0x00`, high byte `0x40 | 14 = 0x4e`.

**Output 2 — stake change**: `Value = 0`, `Version = 0`, 26-byte script `bd 76 a9 14 <20 zero bytes> 88 ac` (`OP_SSTXCHANGE` tagged P2PKH to the all-zero hash160).

After construction `stake.CheckSStx` must pass; failure is `errors.Bug` with op `stake.CheckSStx`.

Signing (`signP2PKHMsgTx`): `sign.SignatureScript(tx, 0, splitOutputPkScript, SigHashAll, privKey, STEcdsaSecp256k1, compressed=true)`.

Worst-case size used for the ticket fee (`wallet/createtx.go:1217-1242`): one input with a 108-byte signature script, output scripts of 26, 32 and 26 bytes, no change:

```
12 + 2*1 + 1 + (32+4+1+8+4+4+1+108+4) + (8+2+1+26) + (8+2+1+32) + (8+2+1+26) = 298 bytes
ticketFee        = FeeForSerializeSize(relayFee, 298)   // relayFee*298/1000; see 6
neededPerTicket  = ticketPrice + ticketFee
```

With the default relay fee of 10000 atoms/kB, `ticketFee = 2980` atoms.

### 4.2 Vote transaction (`createUnsignedVote`, `wallet/createtx.go:2093-2176`, plus the treasury output added in `wallet/chainntfns.go:802-842`)

Transaction fields: `Version = 1`, or `3` when a treasury-vote output was appended; `LockTime = 0`; `Expiry = 0`.

**Input 0 — stakebase**: previous outpoint hash all zero, index `0xffffffff`, tree 0; `ValueIn = voteSubsidy`; `SignatureScript = params.StakeBaseSigScript` (`00 00` on mainnet); `Sequence = 0xffffffff`; `BlockHeight = 0`, `BlockIndex = 0xffffffff`.

`voteSubsidy = subsidyCache.CalcStakeVoteSubsidyV3(blockHeight, variant)` where `blockHeight` is the height of the block being voted on and `variant` is `SSVDCP0012` if DCP0012 is active, else `SSVDCP0010` if DCP0010 is active, else `SSVOriginal`.

**Input 1 — ticket**: previous outpoint `(ticketHash, index 0, tree 1)`; `ValueIn = ticket.TxOut[0].Value`; `Sequence = 0xffffffff`; `BlockHeight = 0x00000000`, `BlockIndex = 0xffffffff` (`wire.NewTxIn` defaults, the same null fraud-proof values as input 0); signature script produced by `sign.SignTxOutput(params, vote, 1, ticket.TxOut[0].PkScript, SigHashAll, getKey, getScript, nil, isTreasuryEnabled=true)`.

**Output 0 — block reference**: `Value = 0`, `Version = 0`, 38-byte script `6a 24 <32-byte block hash, internal order> <uint32 LE block height>`.

**Output 1 — vote bits**: `Value = 0`, `Version = 0`, script `6a <push of (uint16 LE vote bits || extended bits)>`. The wallet's extended bits are always exactly 4 bytes, the `uint32` LE vote version, so the script is 8 bytes: `6a 06 <bits lo> <bits hi> <ver 0..3>`.

**Outputs 2..n+1 — rewards**, one per ticket commitment (odd ticket outputs, in order): `Version = 0`; script `bb 76 a9 14 <hash160> 88 ac` (P2PKH commitment) or `bb a9 14 <hash160> 87` (P2SH commitment, when bit 63 of the committed amount is set); `Value = CalculateRewards(commitAmounts, ticket.TxOut[0].Value, voteSubsidy)[i]`:

```
value[i] = floor( floor( commitAmounts[i] * (ticketValue + voteSubsidy) * 2^32 / sum(commitAmounts) ) / 2^32 )
```

computed with arbitrary-precision integers (`calculateTicketReturnAmounts`, `dcrd/blockchain/stake/v5@v5.0.3/staketx.go:563-608`, called by `CalculateRewards` at `:625-630`). `commitAmounts` are the committed amounts parsed from the ticket's odd outputs with bit 63 cleared (`staketx.go:346-404`).

**Optional last output — treasury votes**: `Value = 0`, `Version = 0`, script `6a <canonical push of data>` where

```
data = 'T' 'V' || for each voted tspend: <32-byte tspend hash, internal order> <1 byte vote: 0x01 yes, 0x02 no>
```

Push encoding follows the canonical script builder: `OP_DATA_n` for `len(data) <= 75` (1 or 2 votes: 35 or 68 bytes), `OP_PUSHDATA1 <len>` (`4c nn`) for 3 or more votes (101 bytes and up). The output is added only when at least one vote was appended; adding it sets `vote.Version = 3`.

The wallet does not run `stake.CheckSSGen` (or any other structural check) on the vote it built before signing and publishing it; the only local validation is that `sign.SignTxOutput` succeeds. A vote that dcrd would reject (for example one carrying more than 7 treasury votes, 3.10.1 rule 9) is published, rejected by the backend, and then still recorded in the wallet (5.11 steps 9 and 11).

The vote pays an implicit fee equal to `ticketValue + voteSubsidy - sum(rewards)`, which is between 0 and `number of commitments - 1` atoms from the truncating divisions. For the single-commitment tickets this wallet buys the reward is exactly `ticketValue + voteSubsidy` and the fee is 0.
### 4.3 VSP fee transaction (`CreateVspPayment`, `wallet/wallet.go:4665-4762`)

`LockTime = 0`, `Expiry = 0`. `Version` is inherited from the `wire.MsgTx` the fee transaction is built in, and `CreateVspPayment` never sets it: it is **1** when that transaction came from `wire.NewMsgTx()` (the purchase path, `wallet/createtx.go:1634`, and `makeFeeTx` when neither the caller nor the job supplied one, `wallet/feepayment.go:319-321`), and **0** when it is a zero-value `new(wire.MsgTx)`: gRPC `SyncVSPFailedTickets` passes one to `Process` (`internal/rpc/rpcserver/server.go:4111-4112`), and `submitPayment` substitutes one when the job has no fee transaction (`wallet/feepayment.go:499-501`). The serialization type is full (0) in both cases, so such a fee transaction serializes with the leading bytes `00 00 00 00` instead of `01 00 00 00` and has a different hash than a version-1 transaction with the same inputs and outputs. Inputs: the reserved UTXOs in reservation order, each built with `wire.NewTxIn(outpoint, value, nil)`. Outputs: the fee output `{Value: fee, Version/PkScript: feeAddr.PaymentScript()}` and, unless it would be dust, one P2PKH change output to the address returned by `NewChangeAddress(changeAcct)` (next internal-branch address with gap policy *wrap*, so it can be a previously returned address once the gap limit is reached; the imported account is redirected to the default account); when a change output exists its position is randomised against the fee output (`txauthor.RandomizeOutputPosition(tx.TxOut, len-1)`). Every input is signed `SigHashAll` by `Wallet.SignTransaction`. The transaction is stored as unmined **and unpublished** and is never broadcast by the wallet itself until marked published (section 5.9).

### 4.4 VSP buckets (`wallet/udb/vsp.go`)

Three top-level buckets (siblings of `waddrmgr` and `wtxmgr`), created by database upgrades to versions 20 (`vsp`) and 23 (`vsphost`, `vsppubkey`) (`wallet/udb/upgrades.go:1468-1616`). A new wallet database gets them by running all upgrades from the initial version (`wallet/udb/initialize.go:45-50`).

**Bucket `vsp`** — key: 32-byte ticket hash. Value: exactly 40 bytes.

| Offset | Size | Field |
| --- | --- | --- |
| 0 | 32 | fee transaction hash (all zero when none or errored) |
| 32 | 4 | fee status, `uint32` BE: 0 started, 1 paid, 2 errored, 3 confirmed |
| 36 | 4 | VSP host ID, `uint32` BE (key into `vsphost`) |

Records written before version 21 were 32 bytes; the version-21 upgrade appended status 2 (errored) to every record, and the version-23 upgrade appended host ID 0. A current reader always indexes 40 bytes without a length check (`deserializeVSPTicket`, `wallet/udb/vsp.go:149-161`).

**Bucket `vsphost`** — two kinds of key:

| Key | Value |
| --- | --- |
| 4-byte `uint32` BE host ID | `uint32` BE host length `n`, then `n` host bytes |
| the 12 ASCII bytes `vsphostindex` | `uint32` BE: highest host ID allocated so far |

The version-23 upgrade creates ID 0 with a zero-length host (value `00 00 00 00`) and sets `vsphostindex` to 0. The first real host gets ID 1.

**Bucket `vsppubkey`** — key: the host string bytes (variable length, exactly as in `vsphost`). Value:

| Offset | Size | Field |
| --- | --- | --- |
| 0 | 4 | host ID, `uint32` BE |
| 4 | 4 | pubkey length `n`, `uint32` BE |
| 8 | n | VSP public key bytes (32 for Ed25519) |

The *host string* stored in both buckets is `client.URL`, i.e. the result of Go `url.Parse(cfg.URL).String()` — a full base URL such as `https://vsp.example.org`, not a bare hostname.

`SetVSPTicket(ticketHash, record)` algorithm:

1. Look up `vsppubkey[record.Host]`.
2. If absent: read `vsphost["vsphostindex"]`, add 1, write `vsphost[newID] = serializeVSPHost(record.Host)`, write `vsphost["vsphostindex"] = newID`, write `vsppubkey[record.Host] = {ID: newID, PubKey: record.PubKey}`.
3. If present: reuse its ID. **The stored pubkey is not updated** even if `record.PubKey` differs.
4. Set `record.VSPHostID` and put the 40-byte record under the ticket hash.

Lookup failures are kind `errors.NotExist` with these messages (the second and third format a **pointer** with `%v`, see Q24): `no VSP info for ticket <hash>` (`GetVSPTicket`), `no VSP host info for id <address of a uint32, e.g. 0xc000123456>` (`GetVSPHost`), `no VSP pubkey info for host <&[104 116 ...], the Go rendering of a pointer to a byte slice>` (`GetVSPPubKey`). The last two never reach a user through this subsystem because `GetVSPTicket` and `SetVSPTicket` swallow them.

`GetVSPTicket` reads the record, then `vsphost[id]`; a missing host entry is tolerated. If the host is non-empty it sets `Host` and looks up the pubkey (missing pubkey tolerated). For host ID 0 the result has `Host == ""` and `PubKey == nil`.

### 4.5 Treasury policy buckets (`wallet/udb/treasury.go`)

Top-level buckets; value is always a single byte `0x01` (yes) or `0x02` (no). Setting policy `0x00` deletes the key. A stored value whose length is not 1 is an `errors.IO` error (`invalid length <n> for tspend vote policy` / `... for treasury key policy`).

| Bucket | Key |
| --- | --- |
| `tspendpolicy` | 32-byte tspend hash |
| `treasurypolicy` | treasury (Pi) public key bytes (33 bytes when set through the RPCs) |
| `vsptspendpolicy` | 32-byte ticket hash ‖ 32-byte tspend hash (64 bytes) |
| `vsptreasurypolicy` | 32-byte ticket hash ‖ treasury public key bytes |

All four are loaded into memory at `Open` (`wallet/wallet.go:5495-5517`) and the in-memory maps are the source for reads afterwards.

In-memory key types: `tspendPolicy` is keyed by the 32-byte hash; `tspendKeyPolicy` by the key bytes as a string; `vspTSpendPolicy` by `udb.VSPTSpend{Ticket, TSpend}` (first and second 32 bytes of the database key); `vspTSpendKeyPolicy` by `udb.VSPTreasuryKey{Ticket, TreasuryKey}` (first 32 bytes, and the remaining bytes as a string). The loaders do not validate key lengths: a `vsptspendpolicy` key shorter than 32 bytes or a `vsptreasurypolicy` key shorter than 32 bytes panics with a slice-bounds error (`wallet/udb/treasury.go:131-134,247-251`); a `tspendpolicy` key of another length is copied/truncated into a 32-byte hash and then looked up under that padded key, which reads as "no policy". The wallet itself never writes such keys.

Neither the database layer nor `Wallet.SetTreasuryKeyPolicy` checks that the treasury key is 33 bytes or one of the network's Pi keys; the JSON-RPC `settreasurypolicy` handler checks only that the hex decodes to 33 bytes (`internal/rpc/jsonrpc/methods.go:3898-3905`); membership in the network's Pi keys is never checked. Other callers are owned by the RPC specs.
Agenda preferences (bucket `agendaprefs`, key `uint32 BE version ‖ agenda ID`, value choice ID; bucket `ticketsagendaprefs` with one nested bucket per 32-byte ticket hash and the same key/value inside; `wallet/udb/agendas.go`) are owned by the wallet-core spec and are referenced here only because `AgendaChoices` feeds VSP requests.

### 4.6 VSP HTTP API

Base URL is `client.URL` (no trailing-slash handling: the request URL is the plain concatenation `URL + path`).

| Method | Path | Request body | Signed by client | Response type |
| --- | --- | --- | --- | --- |
| GET | `/api/v3/vspinfo` | none | no | `VspInfoResponse` |
| POST | `/api/v3/feeaddress` | `FeeAddressRequest` | yes | `FeeAddressResponse` |
| POST | `/api/v3/payfee` | `PayFeeRequest` | yes | `PayFeeResponse` |
| POST | `/api/v3/ticketstatus` | `TicketStatusRequest` | yes | `TicketStatusResponse` |
| POST | `/api/v3/setvotechoices` | `SetVoteChoicesRequest` | yes | `SetVoteChoicesResponse` |

#### 4.6.1 Request construction (`vspd/client/v4@v4.0.2/client.go:164-256`)

1. The body is the Go `encoding/json` encoding of the request struct: compact (no whitespace), fields in declaration order, **no `omitempty` anywhere**, map keys sorted by byte value; inside strings the characters `<`, `>`, `&`, U+2028 and U+2029 are written as the six-character escapes `\u003c`, `\u003e`, `\u0026`, `\u2028`, `\u2029`, and each invalid UTF-8 byte is written as the six-character escape `\ufffd`. The body is marshalled twice (once in the endpoint method, again in `do` wrapped as `json.RawMessage`); the second pass is byte-identical.
2. Signature: `sig = Sign(ctx, string(body), commitmentAddr)`. In the wallet `Sign` is `Wallet.SignMessage` (`wallet/wallet.go:2008-2032`):
   - message hash = BLAKE-256 of `varstring("Decred Signed Message:\n") ‖ varstring(body)` where `varstring` is a wire varint length followed by the bytes;
   - signature = 65-byte secp256k1 compact recoverable signature with the *compressed* flag set (`ecdsa.SignCompact(priv, hash, true)`);
   - the key is the private key of the ticket's **commitment address** (output 1), so the wallet (or the account) must be unlocked. A locked wallet surfaces as an error wrapping kind `errors.Locked` behind the prefix `sign request: `.
3. Header `VSP-Client-Signature: <standard base64 with padding of the 65 bytes>` (Go canonicalises the name on the wire to `Vsp-Client-Signature`).
4. No `Content-Type` header is set. `Content-Length` is set from the body. Other headers are Go `net/http` defaults (`User-Agent: Go-http-client/1.1` on an HTTP/1.1 connection, `Go-http-client/2.0` on an HTTP/2 connection — see the transport paragraph below for which is used; `Accept-Encoding: gzip` with transparent decompression).
5. GET requests carry no body and no signature header.

Transport (`wallet/vsp.go:76-84`): a fresh `http.Transport{DialContext: dialer}` per client with every other field zero — no proxy from the environment, no TLS/response-header/idle timeouts, default TLS configuration and system roots, keep-alives on. The protocol depends on whether `dialer` is nil. The wallet's dial function is non-nil **only when `--proxy` is configured** (`config.go:690-725`; it is otherwise left nil and passed through the loader unchanged, `dcrwallet.go:172`, `internal/loader/loader.go:184`). With a non-nil `DialContext` and `ForceAttemptHTTP2` false, Go does not enable HTTP/2, so connections are HTTP/1.1 only. With a nil `DialContext` (no proxy) the transport is Go's default profile: the standard dialer, and for `https` URLs HTTP/2 is offered through ALPN (`h2`, `http/1.1`) and used when the VSP's server selects it; `http` URLs stay HTTP/1.1. `http.Client.Timeout` is zero: the only bound on a request is its context. Every request made from a fee payment job step (`feeaddress`, `payfee`, and the `ticketstatus` of `confirmPayment`) uses the job's `context.Background()`, **including when the step runs synchronously inside `Process`** (`wallet/feepayment.go:233,293,393,486,561`); only `vspinfo` (`FeePercentage`) and the `ticketstatus`/`setvotechoices` requests of `ProcessManagedTickets` and `SetVoteChoice` use the caller's context. Redirects follow the Go default policy (up to 10).

At log level trace the full request and response are dumped (`Request to <URL>\n<dump>\n`, `Response from <URL>\n<dump>\n`), which includes the voting private key in `payfee` bodies. The dumps are produced only when the logger's level is exactly trace (`c.Log.Level() == slog.LevelTrace`); `<URL>` is the client's base URL, not the request URL; the request dump is the wire form Go would send (`httputil.DumpRequestOut`, body included for POST) and the response dump includes the body (`httputil.DumpResponse(reply, true)`). If a dump fails the line is `VSP request dump failed: <err>` or `VSP response dump failed: <err>` instead, and the request proceeds.

#### 4.6.2 Response handling

1. Transport error: `"<METHOD> <full URL>: <err>"`.
2. Read the entire body; error `read response body: <err>`.
3. Status other than 200:
   - empty body: `http status <code> (<status text>) with no body`;
   - else decode the body as `ErrorResponse` with unknown fields disallowed; on success the returned error **is** the `ErrorResponse` value (callers match it with `errors.As` and read `Code`);
   - else `http status <code> (<status text>) with body "<body, Go %q quoted>"`.
   No signature check is made on non-200 responses.
4. Status 200: header `VSP-Server-Signature` must be present (`authenticate server response: no signature provided`), must be standard base64 (`authenticate server response: failed to decode signature: <err>`), and must be a valid Ed25519 signature by the configured public key over the exact body bytes (`authenticate server response: invalid signature`).
5. JSON-decode the body into the response type (unknown fields ignored; error `unmarshal response body: <err>`).
6. For every POST endpoint the decoded `request` field (base64 in JSON) must equal the request body bytes exactly, else `server response contains differing request`.

Response timestamps are never validated.

#### 4.6.3 JSON shapes (`vspd/types/v3@v3.0.0/types.go`)

Field order below is the serialization order.

`FeeAddressRequest`: `{"timestamp":int64,"tickethash":string,"tickethex":string,"parenthex":string}`

- `timestamp`: Unix seconds at request time. `tickethash`: ticket hash string (reversed hex). `tickethex`, `parenthex`: lower-case hex of the full serialization of the ticket and of the transaction its input 0 spends.

`FeeAddressResponse`: `{"timestamp":int64,"feeaddress":string,"feeamount":int64,"expiration":int64,"request":base64}` — the wallet uses `feeaddress` (decoded with the wallet's network parameters) and `feeamount` (atoms); `expiration` is ignored.

`PayFeeRequest`: `{"timestamp":int64,"tickethash":string,"feetx":string,"votingkey":string,"votechoices":{..},"tspendpolicy":{..},"treasurypolicy":{..}}`

- `feetx`: hex of the signed fee transaction. `votingkey`: WIF of the voting address private key (`dcrutil.NewWIF(key, params.PrivateKeyID, STEcdsaSecp256k1)`). The three maps are never `null`: the client replaces nil maps with `{}`.

`PayFeeResponse`: `{"timestamp":int64,"request":base64}`

`SetVoteChoicesRequest`: `{"timestamp":int64,"tickethash":string,"votechoices":{..},"tspendpolicy":{..},"treasurypolicy":{..}}` (nil maps replaced with `{}`).

`SetVoteChoicesResponse`: `{"timestamp":int64,"request":base64}`

`TicketStatusRequest`: `{"tickethash":string}` (no timestamp).

`TicketStatusResponse`: `{"timestamp":int64,"ticketconfirmed":bool,"feetxstatus":string,"feetxhash":string,"altsignaddress":string,"votechoices":{..},"tspendpolicy":{..},"treasurypolicy":{..},"request":base64}` — the wallet reads `feetxstatus` (`received`, `broadcast`, `confirmed`, `error`; anything else is "unknown"), `feetxhash` (reversed hex or empty), and the three maps.

`VspInfoResponse`: `{"apiversions":[int64],"timestamp":int64,"pubkey":base64,"feepercentage":float64,"vspclosed":bool,"vspclosedmsg":string,"network":string,"vspdversion":string,"voting":int64,"voted":int64,"totalvotingwallets":int64,"votingwalletsonline":int64,"expired":int64,"missed":int64,"blockheight":uint32,"estimatednetworkproportion":float32}` — the wallet reads only `feepercentage` (a percentage, e.g. `2.5`). `vspclosed`, `network` and `pubkey` are not checked.

`ErrorResponse`: `{"code":int64,"message":string}`.

`SetAltSignAddrRequest`: `{"timestamp":int64,"tickethash":string,"tickethex":string,"parenthex":string,"altsignaddress":string}`; `SetAltSignAddrResponse`: `{"timestamp":int64,"request":base64}`. Defined in the types module for the VSP's alternate-signing-address request; the pinned client has no method (and no path constant) for it and the wallet never sends it. `TicketStatusResponse.altsignaddress` is decoded and ignored.

**Server-side validation tags.** The request structs carry `binding` tags that the VSP server's request binder enforces (the tags are in the pinned types module; the server's enforcement itself was not read and is UNVERIFIED):

| Field | Tag | Consequence for the wallet |
| --- | --- | --- |
| `timestamp`, `tickethash`, `tickethex`, `parenthex`, `feetx`, `votingkey`, `altsignaddress` | `required` | must be present and non-zero |
| `votechoices` (in `payfee` and `setvotechoices`) | `required` | must be present; the client always sends at least `{}` |
| `tspendpolicy`, `treasurypolicy` (in `payfee` and `setvotechoices`) | `max=3` | at most **3 entries** per map |

The wallet applies no such limit before sending: `payfee` sends every per-ticket tspend and treasury-key policy of the ticket, and gRPC `SetVspdVoteChoices` sends one `tspendpolicy` entry per cached tspend and one `treasurypolicy` entry per configured key (5.10). If the VSP enforces the tag, a request exceeding the limit is rejected (which error code it uses is UNVERIFIED), and on the `payfee` path the fee payment then cannot complete while more than three per-ticket policies of either kind exist for the ticket: `reconcilePayment` keeps rescheduling (5.8.5).

Go decoding rules that apply to every response: unknown JSON fields are ignored on 200 responses; a missing field leaves the Go zero value (`0`, `""`, `false`, nil map/slice); JSON `null` for any field likewise leaves the zero value; `request` and `pubkey` are standard base64 with padding (a decode failure is an `unmarshal response body: ` error); `int64` fields reject fractional or out-of-range numbers; duplicate keys keep the last value. A 200 response whose body is the JSON literal `null` (with a valid signature over it) decodes to a nil response pointer and the client then dereferences it (Q41).

Map contents sent by the wallet:

- `votechoices`: agenda ID to choice ID, one entry for **every** agenda of the wallet's vote version, `abstain` where nothing is saved.
- `tspendpolicy`: tspend hash string to `yes`/`no`/`abstain`.
- `treasurypolicy`: treasury key to `yes`/`no`/`abstain`; hex-encoded in the RPC-driven paths, raw bytes in the `payfee` path (quirk Q9).

---

## 5. Algorithms and state machines

Conventions: "View" and "Update" mean a read-only and a read-write wallet database transaction (`walletdb.View` / `walletdb.Update`). "Locked set" is the in-memory `lockedOutpoints` map guarded by `lockedOutpointMu`; it is keyed by `(hash, index)` without the tree and is not persisted. Throughout, `lockedOutpointMu` is acquired **before** the database transaction is opened, never inside one.

### 5.1 Stake state loaded at `Open` (`wallet/wallet.go:5386-5517`)

1. `votingEnabled`, `manualTickets`, `vspMaxFee`, `dialer` are copied from the config. The tspend cache and `vspClients` start empty.
2. In one View: default vote bits are computed (`readDBVoteBits`): `Bits = 0x0001`, `ExtendedBits = uint32 LE voteVersion`; for each agenda of `params.Deployments[voteVersion]` with a saved default choice whose ID matches one of the agenda's choices, OR in that choice's bits. Then the four policy buckets are loaded into `tspendPolicy`, `vspTSpendPolicy`, `tspendKeyPolicy`, `vspTSpendKeyPolicy`.
3. A subsidy cache is created per wallet (`blockchain.NewSubsidyCache(params)`); `txrules.StakePoolTicketFee` uses its own process-global cache created on first use from whichever network parameters are passed first (`wallet/txrules/poolfees.go:25-58`).

### 5.2 `PurchaseTickets` wrapper (`wallet/wallet.go:1585-1701`)

1. If `req.Mixing` and the wallet was opened without mixing support: `errors.E("wallet.PurchaseTickets", errors.Invalid, "wallet mixing support is disabled")`.
2. `ctx = n.WrapContext(ctx)` (cancelled when the backend disconnects); cancel on return.
3. `resp, err = purchaseTickets(ctx, op, n, req)`.
4. Return `(resp, err)` unless `err` is `errVSPFeeRequiresUTXOSplit` **and** `req.DontSignTx` is false.
5. If `req.Mixing && req.MixedAccount == req.SourceAccount`: return `errors.E(op, errors.InsufficientBalance)`.
6. `feePercent = req.VSPClient.FeePercentage(ctx)`; `sdiff = NextStakeDifficulty(ctx)` (no RPC fallback here); `height` = tip height. Any error is returned unwrapped.
7. `dcp0010Active = dcp0012Active = true`; if `n` implements `deployments.Querier`, replace both with the queried values at `height`.
8. `vspFee = StakePoolTicketFee(sdiff, relayFee, height, feePercent, params, dcp0010Active, dcp0012Active)`. **The second argument is the relay fee rate (atoms/kB), not the ticket's absolute fee** as in step 5.3.7 (quirk Q3).
9. Build an output to a new internal address of `req.SourceAccount` (default gap policy: fails when the address gap limit would be exceeded) with value `vspFee + txfee`, where `txfee = FeeForSerializeSize(relayFee, EstimateSerializeSize([]int{RedeemP2PKHInputSize /* 166 */}, outputs, 0))`. Passing the 166-byte *input* size where a signature-script size (108) is expected overestimates the size by 58 bytes (quirk Q4). With one P2PKH output the estimate is `12 + 2 + 1 + (57+1+166) + 36 = 275` bytes (2750 atoms at the default relay fee).
10. Author a transaction with that single output: source and change account both `req.SourceAccount`, `minconf = req.MinConf`, change position randomised, fee rate `relayFee`, signed. Then `recordAuthoredTx`, `publishAndWatch` (with a nil backend, i.e. the wallet's current backend), then `changeSourceUpdates.UpdateDB` a second time. Errors are returned unwrapped beyond what those helpers add.
11. Mutate the request: `MinConf = 0`, `Count = 1`, `extraSplitOutput = {OutPoint{txHash, index, tree 0}, PrevOut: tx.TxOut[index]}` where `index = 1` if the change landed at index 0, else `0`.
12. Return `purchaseTickets(ctx, op, n, req)`. A second `errVSPFeeRequiresUTXOSplit` from this call is returned to the caller as is.

### 5.3 `purchaseTickets` (`wallet/createtx.go:1162-1668`)

1. `req.MinConf < 0`: `errors.E(op, Invalid, "negative minconf")`. `req.Expiry < 0`: `errors.E(op, Invalid, "negative expiry")`.
2. View: read `tipHeight`.
3. If `0 < req.Expiry <= tipHeight+1`: `errors.E(op, Invalid, "expiry height must be above next block height")`.
4. `ticketPrice = NextStakeDifficulty(ctx)`; if that fails with kind `errors.Deployment`, `ticketPrice = n.StakeDifficulty(ctx)`. Remaining error returned as is.
5. `ticketFee = FeeForSerializeSize(RelayFee(), 298)`; `neededPerTicket = ticketFee + ticketPrice` (section 4.1).
6. Register a deferred step that always runs at exit, using a fresh non-cancellable context: `watchHDAddrs(ctx, false, n)` (failure logged: `Failed to watch for future addresses after ticket purchases: <err>`), then, if any outpoints were collected, `n.LoadTxFilter(ctx, false, nil, watchOutPoints)` (failure logged: `Failed to watch outpoints: <err>`).
7. **VSP fee reservation** (only if `req.VSPClient != nil`):
   1. `feePrice = VSPClient.FeePercentage(ctx)` (HTTP GET). Error returned as is.
   2. `dcp0010Active = dcp0012Active = true`; when `n` implements `deployments.Querier` use the queried values at `tipHeight`.
   3. `fee = StakePoolTicketFee(ticketPrice, ticketFee, tipHeight, feePrice, params, dcp0010Active, dcp0012Active)`. This value is used **only to size the UTXO reservation**; the amount actually paid is whatever the VSP's `feeaddress` response says.
   4. Register a deferred unlock of every credit in `vspFeeCredits` that runs only if `unlockCredits` is still true at exit (log debug `Unlocked unneeded credit for VSP fee tx: <outpoint>`).
   5. If `req.extraSplitOutput != nil`: `vspFeeCredits = [[extraSplitOutput]]` and lock that outpoint.
   6. For `i` in `0..req.Count`:
      - if `extraSplitOutput == nil`: `ReserveOutputsForAmount(SourceAccount, fee, MinConf)`; on `InsufficientBalance` set `lowBalance` and stop the loop; other errors are logged (`ReserveOutputsForAmount failed: <err>`) and returned; else append to `vspFeeCredits`.
      - `ReserveOutputsForAmount(SourceAccount, ticketPrice, MinConf)`; on `InsufficientBalance` set `lowBalance`, call `reserveOutputs(SourceAccount, MinConf)` (locks and returns everything still eligible, error ignored), append it to `ticketCredits` if non-empty, and stop the loop; other errors logged and returned; else append to `ticketCredits`.
   7. Unlock every credit in `ticketCredits` (log debug `Unlocked credit for ticket tx: <outpoint>`). They were locked only to keep the fee reservation from picking them.
   8. If `lowBalance` (UTXO contention), reallocate:
      1. `credits = vspFeeCredits ++ ticketCredits`, sorted by descending total value (`sort.Slice`, not stable).
      2. If empty: return bare `errors.E(errors.InsufficientBalance)`.
      3. `req.Count = min(req.Count, len(credits)-1)` (may become 0).
      4. `freedBalance = 0`, `extraSplit = true`. While `req.Count > 1`: unlock every UTXO of `credits[0]` and add its value to `freedBalance`; drop `credits[0]`; if `freedBalance > ticketPrice * req.Count` set `extraSplit = false` and stop; else `req.Count--`.
      5. `vspFeeCredits = credits` (what remains); lock every UTXO in it; `remaining` = its total value.
      6. If `req.Count < 2 && extraSplit`: if `ticketPrice > freedBalance + remaining` return bare `errors.E(errors.InsufficientBalance)`, else return `errVSPFeeRequiresUTXOSplit` (handled by 5.2). The deferred unlock from 7.4 releases the fee credits.
   9. Log info `Reserved credits for <Count> tickets: total fee: <fee>` and, per fee credit, debug `<outpoint> reserved for vsp fee transaction`.
8. **Split loop**: call `mixedSplit` if `req.Mixing`, else `individualSplit`. If the error is `InsufficientBalance` and `req.Count > 1`: `req.Count--`; if `vspFeeCredits` is non-empty unlock and drop its first element; retry. Any other error (or `InsufficientBalance` at `Count == 1`): return `errors.E(op, err)`.
9. `response.SplitTx = splitTx`. If not `DontSignTx`: build a record for the split tx; under `lockedOutpointMu`, Update: `processTransactionRecord(rec, nil, nil)` and collect its watch outpoints; add the hash to `recentlyPublished`; `n.PublishTransactions(ctx, splitTx)`. Errors return `(nil, err)` unwrapped. (The non-mixed split was already recorded and published inside `individualSplit`; the mixed split was already published by the mixing client. This is a second record/publish of the same transaction.)
10. If `req.Mixing`: for each split output pick `now + 20s + uniform[0, 40s)` and sort ascending. These are the publish times of the tickets in order.
11. **Create tickets**, for each split output index in the order returned by the split:
    1. Log info `Split output is <outpoint>`.
    2. Voting address: next address of `req.VotingAccount`, **branch 1 (internal)**, gap policy *ignore*; remember its child index `idx` (0 if the address is not an xpub-derived address). The result must implement `stdaddr.StakeAddress`, else `errors.E(Invalid, "account does not return compatible stake addresses")`.
    3. `signingAddressAtIdx(VotingAccount, idx)`: derive branch 0 (external) child `idx` of the voting account, then in its own Update call `Manager.SyncAccountToAddrIndex(account, idx, branch 0)`, which writes address-manager address rows for the external branch from child `idx` downwards until it meets a child that already has a row (`wallet/addresses.go:654-669,468-502`, `wallet/udb/addressmanager.go:1985-2065`). **No last-used or last-returned index is changed** by this call (the same holds for the `UpdateDB` step of every `nextAddress` call at this pin: `MarkReturnedChildIndex` is only called from `SyncLastReturnedAddress`, `wallet/addresses.go:887`). The derived address itself is discarded. Errors are wrapped with `op`.
    4. Commitment address: next address, gap policy ignore, of `(SourceAccount, branch 1)`; or of `(MixedAccount, MixedAccountBranch)` when mixing.
    5. Errors in 2-4 return `(nil, err)`.
    6. Under `lockedOutpointMu`, one Update per ticket:
       - `makeTicket(params, splitOutput, addrVote, addrSubsidy, ticketPrice)`; set `Expiry`.
       - Append the ticket and its hash to the response **before signing** (the hash of an unsigned ticket equals the hash of the signed one because Decred transaction hashes exclude witness data).
       - If `DontSignTx`: done with this ticket.
       - `signP2PKHMsgTx`; `validateMsgTx` (script engine with `sanityVerifyFlags`; failure is `errors.ScriptFailure` and logs `Script validation failed (outpoint ... pkscript ... sigscript ...): <err>`); `checkHighFees(inputValue, ticket)` (`errors.Policy`, `high fee`, unless `allowHighFees`); `processTransactionRecord(rec, nil, nil)`; add hash to `recentlyPublished`.
       - Error: return `(response, errors.E(op, err))`. The response still lists the failed ticket (quirk Q7).
12. If `DontSignTx`: return `(response, nil)`. In the non-mixed case nothing was recorded or published and the split transaction returned is unsigned; in the mixed case the split was already mixed, signed, recorded and published by the mixing client (quirk Q36).
13. **Publish**, for each ticket `i` in order:
    1. If trickle times exist, wait until the next one or until `ctx` is done (`(response, errors.E(op, ctx.Err()))`). Otherwise fail fast if `ctx` is already done.
    2. `n.PublishTransactions(ctx, ticket)`; error returns `(response, errors.E(op, err))`. Log info `Published ticket purchase <hash>`.
    3. If no VSP client: next ticket.
    4. `unlockCredits = false` (from here on, fee credits of later tickets are **not** released by the deferred unlock if the function exits early; quirk Q6).
    5. `feeTx` = new transaction whose inputs are `vspFeeCredits[i]`.
    6. `NewVSPTicket(ctx, ticketHash)`; on error unlock `vspFeeCredits[i]` and continue with the next ticket (error dropped).
    7. `req.VSPClient.Process(ctx, vspTicket, feeTx)`; on error unlock `vspFeeCredits[i]` and continue (error dropped, not logged here). `Process` returns **nil** when `feeaddress` or fee-transaction creation fails (5.8.3 steps 2.3 and 2.4), so in those cases `vspFeeCredits[i]` is **not** unlocked here: the credits remain in the locked set as the inputs of the job's pending fee transaction.
14. Return `(response, nil)`.

A VSP failure therefore never fails the purchase: the ticket exists, is published, and is left with whatever `vsp` record `Process` wrote (typically status 2, errored).

### 5.4 `individualSplit` (`wallet/createtx.go:1099-1150`)

1. `NewInternalAddress(ctx, SourceAccount, WithGapPolicyWrap())`: the next internal-branch address (opens its own Update; with gap policy *wrap* the branch cursor resets and a previously returned address is handed out once the gap limit is reached). All ticket-funding outputs pay to this one address.
2. Outputs: `Count` outputs of `neededPerTicket` atoms to that P2PKH script, version 0; `outIndexes = [0, 1, ..., Count-1]`.
3. `authorTx` with op `individualSplit`: inputs selected from `SourceAccount` at `MinConf`, change (if any) to a new internal address of `ChangeAccount` with gap policy wrap, appended after the ticket outputs (`randomizeChangeIdx = false`), fee rate `RelayFee()`, signed unless `DontSignTx`, selected inputs added to the locked set while authoring and removed when it returns. Fails with `errors.Policy` `high fee` or `errors.InsufficientBalance` among others.
4. Unless `DontSignTx`: `recordAuthoredTx` (one Update: persist change-address usage and `processTransactionRecord`), then `publishAndWatch` on the wallet's current backend. If publishing fails the transaction is abandoned (log error `Abandoning transaction <hash> which failed to publish`).
5. Each retry from 5.3 step 8 consumes another internal address.

### 5.5 `mixedSplit` (`wallet/createtx.go:1000-1093`)

1. Template outputs: `Count` outputs of `neededPerTicket` atoms with a 25-byte placeholder script (for size and change calculation only).
2. Under `lockedOutpointMu`, in a View: `txauthor.NewUnsignedTransaction(template, RelayFee(), inputSource(SourceAccount, MinConf), changeSource(ChangeAccount, gap policy ignore), MaxTxSize)`; add every selected input to the locked set (released when `mixedSplit` returns, success or failure). `req.DontSignTx` is ignored.
3. Log info `Selected input <outpoint> (<amount>) for ticket purchase split transaction` per input.
4. Take the authored change output, if any; drop it (treat as fee) when its value is below `smallestMixChange(relayFee) = FeeForSerializeSize(relayFee, 12+2+1+166+36 /* 217 */) + 262144` atoms (`wallet/mixing.go:227-234`).
5. `gen` = address generator for `(MixedSplitAccount, MixedAccountBranch)` (gap policy ignore; returns hash160s). `expires = mixing.MaxExpiry(tipHeight, params) - 2`.
6. `cj = mixclient.NewCoinJoin(gen, change, neededPerTicket, expires, Count)`; add each selected input with its previous script (script version 0) and private key (`errors.Invalid`, `previous output is not P2PKH`, for anything else).
7. `w.dicemix(ctx, cj)`: `errors.Invalid`, `mixing client is not running`, if the client has not been started; otherwise blocks until the mix completes, fails or `ctx` is cancelled.
8. Log info `Completed CoinShuffle++ mix of ticket split transaction <hash>`; return `cj.Tx()` and `cj.MixedIndices()`.
9. On success only, persist the change-address usage (`changeSourceUpdates.UpdateDB`).

The mixed outputs pay to addresses of the *ticket split account*; the ticket commitments (5.3 step 11.4) pay to the *mixed account*; change goes to the *change account* (unmixed).

### 5.6 Eligible-output selection (`wallet/createtx.go:1670-1987`)

An unspent output is **eligible** for account `A` at `minconf` and tip height `H` when all hold:

1. It is not in the locked set.
2. `confirms(height, H) >= minconf`, where `confirms` is 0 for unmined (height -1) or `height > H`, else `H - height + 1`.
3. `stdscript.ExtractAddrs(version 0, pkScript)` yields exactly one address.
4. Script class and maturity:

| Class | Extra condition |
| --- | --- |
| stake generation (vote reward) P2PKH/P2SH | coinbase-mature: `height >= 0 && H - height + 1 > CoinbaseMaturity` |
| stake revocation P2PKH/P2SH | coinbase-mature |
| treasury add, treasury generation P2PKH/P2SH | coinbase-mature |
| stake change P2PKH/P2SH | `height >= 0 && H - height + 1 > SStxChangeMaturity` |
| P2PKH (secp256k1 ECDSA) | coinbase-mature if the output is from a coinbase, else none |
| anything else (including plain P2SH and stake submission) | not eligible |

5. The address belongs to account `A` (`manager.AddrAccount`; any lookup error means not eligible).

The returned `Input.PrevOut` always has `Version = 0`.

`findEligibleOutputsAmount(account, minconf, amount, H, minAmount, maxResults)`:

1. If `(amount != 0 || maxResults != 0) && minconf > 0`: `maxTries = UnspentOutputCount / 2` (log debug `Unspent bucket k/v count: <n>`), else 0.
2. Up to `maxTries` times: `txStore.RandomUTXO(dbtx, minconf, H)`; nil result ends this phase; skip if already seen in this phase, ineligible, or `minAmount != 0 && value < minAmount`; else accumulate. Return as soon as `amount != 0 && total >= amount`, or `maxResults != 0 && count == maxResults`.
3. If the random phase did not return and its loop counter is above zero (at least one probe returned a UTXO, whether or not it was kept): log debug `Abandoned random UTXO selection attempts after <n> tries`, where `<n>` is the number of probes that returned a UTXO (a nil result ends the phase without being counted). A `RandomUTXO` error is returned as is.
4. Discard the partial selection. Read all unspent outputs, shuffle them uniformly (`rand.ShuffleSlice`), and accumulate eligible ones with the same two stop conditions.
5. If `amount != 0 && total < amount`: return the bare kind `errors.InsufficientBalance`. Otherwise return everything accumulated.

Selection is intentionally random; no ordering by value or age is observable.

### 5.7 `NewVSPTicket` (`wallet/vspticket.go:41-98`)

1. Load the transaction by hash from the wallet (`failed to retrieve ticket: <err>`). The lookup is `GetTransactionsByHashes` with one hash; a hash that is not in the transaction store gives kind `errors.NotExist` with message `transaction(s) not found` (`wallet/wallet.go:2319-2342`), so the full text is `failed to retrieve ticket: transaction(s) not found` (the wrapped error keeps its kind for `errors.Is`).
2. Must satisfy `stake.IsSStx` (`<hash> is not a ticket`) and have exactly 3 outputs (`ticket <hash> has multiple commitments`).
3. Voting address: `ExtractAddrs(out0.Version, out0.PkScript)` must give exactly one address (`cannot parse voting addr for ticket <hash>`) that is a `StakeAddress` (`address cannot be used for voting rights: <addr>`).
4. Commitment address: `stake.AddrFromSStxPkScrCommitment(out1.PkScript, params)` (`cannot parse commitment address: <err>`).
5. Voting key: `DumpWIFPrivateKey(votingAddr)` — requires the private key to be in this wallet and the wallet/account to be unlocked; the error is returned as is (kinds `Locked`, `NotExist`, `WatchingOnly`).
6. Parent transaction: the transaction spent by input 0, loaded from the wallet (`failed to retrieve parent <hash> of ticket: <err>`). A ticket whose funding transaction is not in the wallet cannot be registered.

Consequences: tickets with pool-fee commitments (5 outputs) and tickets whose voting key the wallet does not hold cannot be given to a VSP by this wallet.

The checks run in exactly this order, so a 3-output ticket whose voting key is missing fails at step 5, and a locked wallet fails at step 5 before the parent lookup. The commitment-address error of step 4 can only be `short read of sstx commit pkscript` wrapped as `cannot parse commitment address: ...`, and cannot occur after `IsSStx` passed (the script is at least 32 bytes). The returned `VSPTicket` keeps the caller's `hash` pointer; callers must not mutate the hash afterwards.

`UnprocessedTickets` and `ProcessedTickets` (3.6) call `NewVSPTicket` for every selected hash and **return the first error, discarding all tickets built so far** (`wallet/wallet.go:5782-5788,5819-5825`). One ticket that cannot be turned into a `VSPTicket` (wallet locked, voting key not in the wallet, 5-output ticket, parent transaction missing) therefore makes the whole call fail: gRPC `ProcessUnmanagedTickets` returns `failed to retrieve unmanaged tickets. Error: <err>`, gRPC `ProcessManagedTickets` returns `ProcessedTickets failed. Error: <err>` (both code `Unknown`), and `--vsp.sync` logs `Getting VSP tickets failed: <err>` and then still calls `ProcessManagedTickets` with the nil list, which does nothing (`dcrwallet.go:422-432`). `UnprocessedTickets` selects every unspent unexpired ticket without a non-zero fee hash, including tickets that were never meant for a VSP, so a wallet holding any such non-registerable ticket cannot use `ProcessUnmanagedTickets` at all. `ProcessedTickets` selects only tickets that have a `vsp` record. Both require an unlocked wallet (step 5).
### 5.8 VSP fee payment

#### 5.8.1 Timer and jitter (`wallet/feepayment.go:172-230`)

Each job owns at most one pending `time.AfterFunc` timer. `schedule(name, method)` computes the delay first, then under `timerMu` stops and clears the previous timer and, if `method != nil`, logs debug ``Scheduling "<name>" for ticket <hash> in <delay>`` and arms a new one. Stopping does not wait for a method that is already running.

`next()` reads the tip height and the ticket's live and expiry heights (`VSPTicket.LiveHeight` / `ExpiryHeight`, both 0 while the ticket is unmined):

| Condition | Jitter |
| --- | --- |
| `tip < live` (mined, immature) | `min(TargetTimePerBlock * (live - tip), 1 hour)` |
| else `tip < expiry` (live) | 5 minutes |
| otherwise (unmined, or past expiry) | 2 minutes |

Delay = uniform random duration in `[0, jitter)` from the CSPRNG.

Only two task names exist and they appear verbatim in the debug and error log lines: `reconcile payment` (method `reconcilePayment`) and `confirm payment` (method `confirmPayment`). `receiveFeeAddress`, `makeFeeTx` and `submitPayment` are never scheduled on their own. The delay in the scheduling line is a Go `time.Duration` rendered with `%v` (for example `1m23.456789012s`). `next()` is evaluated before `timerMu` is taken and performs a database read for the tip plus, while the ticket's heights are still 0, a mined-height lookup under the ticket's mutex. `stop()` is `schedule("", nil)`: it computes no delay and logs nothing.

The timer callback (`task`) runs the method on its own goroutine, stores the returned error in `fp.err` (nil clears it) and logs non-nil errors at error level as `Ticket <hash>: <name>: <err>`. There is no retry counter, no backoff growth and no maximum number of attempts.

#### 5.8.2 Job creation: `feePayment(ctx, ticket, paidConfirmed)` (`wallet/feepayment.go:97-170`)

1. If `jobs[ticketHash]` exists, return it.
2. Build a new job with `ctx = context.Background()`, the client's policy, state 0.
3. On every exit path below, a deferred step registers the job in `jobs` unless another goroutine registered one meanwhile (then that one is returned instead), and **only when it registered this job** schedules `reconcilePayment` with a jittered delay.
4. If `ticket.Spent(ctx)`: `state = TicketSpent`; return.
5. `feeHash = ticket.FeeHash(ctx)`; on error (no record, or zero hash): return (fresh job, nothing known).
6. `feeTx = ticket.FeeTx(ctx)`; on error set `fp.err = "fee transaction not found in wallet: <err>"` and return (job has no fee tx; reconcile will build a new one).
7. Store `feeTx` and `feeHash` in the job.
8. If `!paidConfirmed`: `UpdateFeeStarted(feeHash, URL, PubKey)` (DB status 0, keeping the hash); on success `state = Unprocessed`, `fee = -1`.

#### 5.8.3 `Process(ctx, ticket, feeTx)` (`wallet/vsp.go:221-287`)

1. Read the `vsp` record. Errors other than `NotExist` are returned. No record means status *started*.
2. Status **started (0)** or **errored (2)**:
   1. `fp = feePayment(ctx, ticket, false)`. (`fp == nil` is handled — write errored, return `fee payment cannot be processed` — but cannot happen.)
   2. If the job has no fee transaction yet, adopt the caller's `feeTx` pointer (possibly nil, possibly a transaction with inputs only).
   3. `receiveFeeAddress()`; on error write status errored (`UpdateFeeErrored`) and return the result of **that write**: the DB error if the write fails, otherwise **nil**. The `feeaddress` error itself is discarded (not returned, not logged): the Go declares a new `err` inside the `if err != nil` block, shadowing the original, and returns the shadow (`wallet/vsp.go:248-258`). A caller therefore sees success when the VSP rejected the ticket, when the fee address was invalid, or when the fee exceeded `MaxFee`.
   4. `makeFeeTx(feeTx)`; on error likewise: status errored is written and nil is returned unless that write fails (`wallet/vsp.go:259-266`).
   5. Return `submitPayment()`. No status is written on its failure here; the record stays *paid* (written by `makeFeeTx`).
3. Status **paid (1)**: if the record's host is non-empty and differs from this client's URL, return `ticket already paid or confirmed with another vsp`. Otherwise `fp = feePayment(ctx, ticket, true)` and return `fp.confirmPayment()`.
4. Status **confirmed (3)** or any other value: return nil.

In the started/errored path `Process` itself never schedules a follow-up; the follow-up is the `reconcilePayment` armed by job creation in step 2.1 (only when this call created the job). Consequently, after a successful synchronous `payfee`, the reconcile timer fires, submits the same fee transaction again, receives `ErrFeeAlreadyReceived`, and only then moves on to confirmation (5.8.6). After a swallowed failure of step 2.3 or 2.4 the same reconcile timer is the retry: it calls `makeFeeTx(nil)`, which works on the job's fee transaction (the caller's inputs-only transaction when one was supplied, otherwise a new empty one) and, when the job's `fee` is still 0, requests a fee address again. In the paid path (step 3) the synchronous `confirmPayment` replaces the reconcile timer armed at job creation whenever it schedules a step or removes the job; only an unknown `feetxstatus` leaves that reconcile timer armed.

#### 5.8.4 `ProcessManagedTickets(ctx, tickets)` (`wallet/vsp.go:148-211`)

For each ticket not already in `jobs` (those log debug `Ticket <hash> already added to vsp client`):

1. `status = ticketstatus`. On error: if the error is kind `errors.Locked`, **return it** (aborting the remaining tickets); if it is `ErrorResponse` with code `ErrUnknownTicket` log debug `Ticket <hash> not known by vsp`; otherwise log debug `Ticket <hash> not added to vsp client, vsp status error: <err>`. Continue with the next ticket.
2. `feetxstatus == "confirmed"`: parse `feetxhash`, `UpdateFeeConfirmed`, log debug `Ticket <hash> already confirmed by vsp`. No job is created. Parse or DB errors are returned.
3. Else if `feetxhash != ""`: parse it, `UpdateFeePaid`, then `feePayment(ctx, ticket, true)` (job created; reconcile scheduled).
4. Else: `feePayment(ctx, ticket, false)` (job created; reconcile scheduled; it will obtain a fee address and pay).

#### 5.8.5 Job steps

**`removedExpiredOrSpent()`** — first action of `receiveFeeAddress`, `reconcilePayment`, `submitPayment` and `confirmPayment` (`makeFeeTx` does not call it directly). If `ticket.Expired` then reason `expired`, else if `ticket.Spent` then `spent`; with a reason: `remove(reason)` (stop timer, log info `Ticket <hash> is <reason>; removing from VSP client`, delete from `jobs`) and the step returns `errStopped`.

**`receiveFeeAddress()`** (`wallet/feepayment.go:232-284`): POST `feeaddress` with the ticket and parent hex. Decode `feeaddress` (`server fee address invalid: <err>`). Log info `VSP requires fee <amount>`. If `feeamount > policy.MaxFee`: `server fee amount too high: <fee> > <max>`. Store `fee` and `feeAddr` in the job. The order matters: the address is decoded before the fee is logged and compared, and nothing is stored when either check fails. The comparison is strict, so a fee equal to `MaxFee` is accepted; a zero or negative `feeamount` is accepted too and stored (a stored fee of 0 makes the next `makeFeeTx` request a fee address again, 5.8.5 `makeFeeTx` step 4). The job's `policy` pointer is dereferenced without a nil check (Q40). The fee address is not checked to be of any particular type beyond decoding for the wallet's network, and `expiration` is not stored.

**`makeFeeTx(tx)`** (`wallet/feepayment.go:292-353`):

1. Snapshot `fee`, `feeTx`, `feeAddr` from the job.
2. If the job has a fee tx: if `tx != nil` overwrite `*tx` with a shallow copy of it, else `tx =` the job's. If that fee tx already has outputs: return nil (finished).
3. If `tx` is still nil: new empty transaction.
4. If `fee == 0`: `receiveFeeAddress()` and re-read `fee`, `feeAddr`. (`fee == -1` from 5.8.2 step 8 does not trigger this; quirk Q12.)
5. `CreateVspPayment(ctx, tx, fee, feeAddr, policy.FeeAcct, policy.ChangeAcct)`; error `unable to create VSP fee tx for ticket <hash>: <err>`.
6. `UpdateFeePaid(tx.TxHash(), URL, PubKey)` — the DB status becomes **paid (1)** as soon as the fee transaction exists locally, before the VSP has seen it.
7. Store `feeTx` and `feeHash` in the job.

**`submitPayment()`** (`wallet/feepayment.go:485-554`):

1. If the job has no fee tx use an empty one; if it has no outputs call `makeFeeTx`.
2. `voteChoices = ticket.AgendaChoices(ctx)`; serialise the fee tx.
3. POST `payfee` with `timestamp`, `tickethash`, `feetx`, `votingkey`, `votechoices`, `tspendpolicy = ticket.TSpendPolicy()`, `treasurypolicy = ticket.TreasuryKeyPolicy()`.
4. On error: log error `VSP fee payment for ticket <hash> failed: <err>`. If the error is `ErrorResponse` with `ErrFeeExpired`: `AbandonTransaction(feeTxHash)` (failure logged `Error abandoning expired fee tx <err>`) and clear the job's fee tx. Return `payfee: <err>` (wrapping).
5. On success log info `Successfully processed <hash>`.

**`reconcilePayment()`** (`wallet/feepayment.go:392-483`):

1. If the job has no fee tx or it has no outputs: `makeFeeTx(nil)`. On error: if it is `ErrorResponse` with `ErrTicketCannotVote`, `remove("ticket cannot vote")`. Return the error **without rescheduling** (the job stays in `jobs` idle unless it was just removed).
2. `err = submitPayment()`; read `feeHash`.
3. If `err` is an `ErrorResponse`:
   - `ErrFeeAlreadyReceived`: `SetPublished(feeHash, true)`, `UpdateFeePaid(feeHash, ...)`; errors from either are returned (no reschedule); else treat as success.
   - `ErrInvalidFeeTx` or `ErrCannotBroadcastFee`: `UpdateFeeErrored` (DB status 2, fee hash zeroed; a DB error is returned without reschedule), clear the job's `feeHash` and `feeTx`; `err` stays set.
4. If `err != nil`: schedule `reconcilePayment` again and return `err`.
5. `UpdateFeePaid(feeHash, ...)` (error returned, no reschedule), then return `confirmPayment()`.

**`confirmPayment()`** (`wallet/feepayment.go:560-611`): POST `ticketstatus`.

| Outcome | Action |
| --- | --- |
| request error | log warn `Rescheduling status check for <hash>: VSP err: <err>`; schedule `confirmPayment`; return nil |
| `feetxstatus == "received"` | schedule `confirmPayment` |
| `"broadcast"` | log info `VSP has broadcast the fee tx for <hash>`; schedule `confirmPayment` |
| `"confirmed"` | `UpdateFeeConfirmed(feeHash, ...)`; on DB error log warn `Rescheduling status check for <hash>: db err: <err>` and schedule `confirmPayment`; else `remove("confirmed by VSP")` |
| `"error"` | log warn `VSP failed to broadcast feetx for <hash> -- restarting payment`; schedule `reconcilePayment` |
| anything else | log warn ``VSP responded with unknown FeeTxStatus "<s>" for <hash>``; nothing scheduled; job stays in `jobs` |

All branches return nil.

#### 5.8.6 State machine summary

Persisted status (`vsp` record) transitions:

| From | Event | To | Fee hash |
| --- | --- | --- | --- |
| none | `feeaddress` or fee-tx creation fails in `Process` | errored (2) | zero |
| none / started / errored | fee tx created and stored (`makeFeeTx`) | paid (1) | new fee tx hash |
| any with a fee hash and fee tx in the wallet | job created with `paidConfirmed == false` | started (0) | unchanged |
| paid | `payfee` returns `ErrInvalidFeeTx` / `ErrCannotBroadcastFee` (in `reconcilePayment`) | errored (2) | zero |
| paid | `payfee` succeeds or returns `ErrFeeAlreadyReceived` | paid (1) (rewritten) | unchanged |
| paid | `ticketstatus` says `confirmed` | confirmed (3) | job's fee hash (see quirk Q13) |
| none / any | `ProcessManagedTickets` and VSP reports a fee hash | paid (1) or confirmed (3) | VSP-reported hash |
| any | ticket's unmined tx pruned (`ChainSwitch`) | record deleted | — |

In-memory job lifecycle: created by `Process` (states started/errored/paid) or `ProcessManagedTickets`; removed on confirmed, expired, spent, `ErrTicketCannotVote`, or prune. The `State` field only ever takes 0, `Unprocessed` (1) or `TicketSpent` (4); `FeePublished` (2) is never assigned.

Across restarts: jobs and timers are gone; only the `vsp` records and the unpublished fee transactions in the transaction store survive. Nothing resumes on its own. Recovery paths:

- `--vsp.sync` at startup: `ProcessedTickets()` then `ProcessManagedTickets` (`dcrwallet.go:422-432`), executed in the after-load callback before network sync starts; failures are logged (`Getting VSP tickets failed: <err>`, `Adding tickets to VSP client failed: <err>`).
- gRPC `ProcessManagedTickets`, `ProcessUnmanagedTickets` (`UnprocessedTickets()` then `ProcessUnprocessedTickets`), `SyncVSPFailedTickets` (every record with status errored: `NewVSPTicket` then `Process(ctx, t, new(wire.MsgTx))`, per-ticket errors ignored; fee transactions created on this path have transaction version 0, see 4.3).
- JSON-RPC `processunmanagedticket <hash>`: `LookupVSP(cfg vsp url)`, `NewVSPTicket`, `Process(ctx, ticket, nil)`.

### 5.9 `CreateVspPayment` (`wallet/wallet.go:4665-4762`)

1. If `tx` has no inputs: `ReserveOutputsForAmount(feeAcct, fee, minconf = 1)` (`unable to reserve outputs: <err>`), add them as inputs, and unlock them again when the function returns (the stored unpublished transaction keeps them from being reselected).
2. `input = sum(ValueIn)`; if `input < fee`: `not enough input value to pay fee: <input> < <fee>`.
3. `NewChangeAddress(ctx, changeAcct)` (log warn `Failed to get new change address: <err>`; return). A change address is consumed even when no change output ends up in the transaction.
4. Replace all outputs with the single fee output.
5. `est = EstimateSerializeSize([108]*numInputs, [feeOutput], changeScriptSize = 25)`; `change = input - fee - FeeForSerializeSize(RelayFee(), est)`.
6. If `!IsDustAmount(change, 25, RelayFee())`: append the change output and randomise its position. `IsDustAmount` is `change*1000 / (3*(8+2+1+25+165)) < relayFee` with integer division, i.e. `change*1000/603 < relayFee`. `change` can be **negative** here (step 2 only guarantees `input >= fee`, not `input >= fee + miningFee`); a negative value counts as dust, no change output is added, and the transaction then pays a mining fee of `input - fee`, which is below the relay rate and can be 0. When `change` is non-negative dust it is likewise left to the miners. The change address must be a wallet `Address` (it always is); otherwise ``failed to convert '<Go type>' to wallet.Address``.
7. `feeHash = tx.TxHash()`; `SignTransaction(tx, SigHashAll, nil, nil, nil)`. Any signing error or per-input signature error fails the call (logged `Failed to sign transaction: <err>` and one line per input error).
8. `SetPublished(feeHash, false)` — marks the hash unpublished **before** the transaction is stored.
9. `AddTransaction(ctx, tx, nil)` — stores it as unmined, creating credits for the change and debiting the inputs.

### 5.10 Propagating voting preferences to VSPs

`VSPClient.SetVoteChoice(ctx, ticket, choices, tspendPolicy, treasuryPolicy)` (`wallet/vsp.go:293-360`):

1. POST `ticketstatus`. On error: kind `errors.Locked` is returned; anything else is logged at error level (`Could not check status of VSP ticket <hash>: <err>`) and **nil is returned**.
2. `update = true` if any provided `choices` entry is missing from or differs in the VSP's `votechoices`; likewise `tspendPolicy` against `tspendpolicy` and `treasuryPolicy` against `treasurypolicy`. Entries the VSP has but the caller did not supply are not considered.
3. No difference: log debug `VSP already has correct vote choices for ticket <hash>`; return nil.
4. Else log debug `Updating vote choices on VSP for ticket <hash>` and POST `setvotechoices` with the three maps exactly as passed (nil becomes `{}`) and the current timestamp. Return its error.

Callers:

| Trigger | Tickets | Maps passed |
| --- | --- | --- |
| JSON-RPC `setvotechoice agenda choice [ticket]` | the given ticket, or every unspent unexpired ticket | `choices = {agenda: choice}` only |
| JSON-RPC `settreasurypolicy key policy [ticket]` | same | `treasuryPolicy = {keyHexAsGiven: policyStringAsGiven}` only |
| JSON-RPC `settspendpolicy hash policy [ticket]` | same | `tspendPolicy = {hashAsGiven: policyStringAsGiven}` only |
| gRPC `SetVspdVoteChoices(vsp host, pubkey, accounts)` | every unspent unexpired ticket whose stored host equals the request's host | `choices = AgendaChoices(ticket)`; `tspendPolicy` = every cached tspend hash to `yes`/`no`/`""` (global policy); `treasuryPolicy` = every entry of `TreasuryKeyPolicies()` (global entries, then the per-ticket entries of **all** tickets, merged into one map by hex key with later entries overwriting earlier ones) with `yes`/`no`/`abstain`. The same two maps are sent for every ticket; the host comparison uses the request's raw `vsp_host` string against the stored (normalised) host |
| `payfee` (5.8.5) | the ticket | `AgendaChoices(ticket)`, per-ticket tspend policies, per-ticket treasury-key policies |

JSON-RPC lookup (`updateVSPVoteChoices`, `internal/rpc/jsonrpc/methods.go:4810-4864`): `VSPHostForTicket` (`NotExist` means "not VSP-managed", skipped), `LookupVSP(host)` (error if no client with that key was created in this process), `NewVSPTicket`, `SetVoteChoice`. With a ticket hash the first error is returned; without one, errors are collected across all tickets and joined. The wallet-local preference is always saved before the VSP is contacted, so an RPC error can mean "saved locally, VSP not updated".

The policy strings in the JSON-RPC paths are forwarded verbatim, so `""`, `invalid` and `abstain` (all accepted locally as "no policy") reach the VSP unchanged.

The gRPC `SetVspdVoteChoices` callback returns the `VSPHostForTicket` error for every unspent ticket that has no VSP record (`NotExist`), so with any non-VSP ticket in the wallet the RPC ends with `ForUnspentUnexpiredTickets failed. Error: ...` (code `Unknown`) even though the VSP tickets visited were updated (`internal/rpc/rpcserver/server.go:4225-4249`).

### 5.11 `VoteOnOwnedTickets` (`wallet/chainntfns.go:698-925`)

Input: the winning ticket hashes, hash and height of the block that selected them (the block to vote on). Invoked for every `winningtickets` notification from dcrd; the syncer requests those notifications (`notifywinningtickets`) only when voting is enabled and logs (Go format) `Wallet voting enabled: vote bits = %#04x, extended vote bits = %x` (the bits render as `0x0001`, the extended bits as lower-case hex), followed by `Please ensure your wallet remains unlocked so it may vote` (`chain/sync.go:645-654`).

1. Return nil if voting is disabled or `blockHeight < StakeValidationHeight - 1`.
2. `n = NetworkBackend()` (error wrapped with op `wallet.VoteOnOwnedTickets`). `n` must implement `deployments.Querier`, else `network backend does not provide deployment information`.
3. Query `DCP0010Active` and `DCP0012Active` at `blockHeight` (one `Deployments` RPC each).
4. `defaultVoteBits = VoteBits()`.
5. In one View, `ticketHashes = selectOwnedTickets(winners)`; if empty the View ends. Allocate `votes` and `usedVoteBits` with one slot per owned ticket. For each owned ticket `i`:
   1. Load the ticket transaction; on failure log error `Failed to read ticket purchase transaction for owned winning ticket <hash>: <err>` and skip.
   2. `hasVotingAuthority`: skip silently unless the wallet both knows the voting address and has its private key. An error here aborts the View.
   3. Vote bits = per-ticket bits if any per-ticket agenda preference is saved for this ticket (`readDBTicketVoteBits`: `0x0001` OR the saved choices' bits), else the defaults.
   4. If `DisapprovePercent() > 0`: on mainnet log warn `Block disapprove percent set on mainnet` and do nothing; elsewhere, with probability `dp/100` (`dp > uniform[0,100)`) log info `Disapproving block <hash> voted with ticket <hash>` and clear bit 0.
   5. `tspends = GetAllTSpends(ctx)` (evicts expired entries; opens a nested View for the tip height).
   6. `createUnsignedVote(...)`; on error log `Failed to create vote transaction for ticket hash <hash>: <err>` and skip.
   7. Treasury votes: start `tVotes = "TV"`. For each cached tspend, in map order: skip unless `InsideTSpendWindow(blockHeight, tspend.Expiry, TreasuryVoteInterval, TreasuryVoteIntervalMultiplier)`; resolve the policy with `TSpendPolicy(tspendHash, ticketHash)` (resolution order below); skip if invalid (0); append the 32-byte tspend hash and the policy byte.
   8. If anything was appended: build `OP_RETURN <push tVotes>`; on success add it as a zero-value version-0 output and set `vote.Version = 3`; on failure log `Failed to create treasury vote for ticket hash <hash>: <err>` and vote without it.
   9. `signVote`; on error log `Failed to sign vote for ticket hash <hash>: <err>` and skip. Store the vote and bits in slot `i`; append the vote's relevant outpoints to the watch list.
6. A View error is logged (`View failed: <err>`) and execution continues with whatever was built.
7. Remove nil slots from `votes` by swapping with the last element (order not preserved; `ticketHashes` and `usedVoteBits` are **not** permuted alongside — quirk Q16).
8. Build transaction records; add each hash to `recentlyPublished`; log info `Voting on block <hash> (height <h>) using ticket <ticket hash> (vote hash: <hash> bits: <bits decimal>)`.
9. `n.PublishTransactions(ctx, votes...)` — **before** recording, to cut latency; failure logged `Failed to send one or more votes: <err>` and ignored. This call is made even when there are no votes.
10. `LoadTxFilter` for the watch list if non-empty (failure logged).
11. One Update, **without** taking `lockedOutpointMu`: `processTransactionRecord(rec, nil, nil)` for each vote. Error returned unwrapped.
12. `watchHDAddrs` if a backend is still set; error returned.

Treasury policy resolution, `TSpendPolicy(tspendHash, ticketHash)` (`wallet/wallet.go:591-628`), first match wins:

1. per-ticket policy for `(ticket, tspend)`;
2. global policy for the tspend hash;
3. if the tspend is not in the cache: invalid (abstain);
4. `pikey = tspend.TxIn[0].SignatureScript[66:99]` (the 33-byte key of the `<sig64> <pubkey33> OP_TSPEND` script);
5. per-ticket policy for `(ticket, pikey)`;
6. global policy for `pikey`, or invalid.

In `VoteOnOwnedTickets` the ticket hash is always supplied, so a locally voting wallet honours per-ticket policies too.

A per-ticket or global map entry wins by **presence** (`ok`), not by value; since abstain entries are deleted rather than stored (4.5), presence always means yes or no. `Wallet.TreasuryKeyPolicy(pikey, ticket)` (the lookup-only accessor) does **not** fall back: with a ticket it consults only the per-ticket map, without one only the global map.

Tspend cache feeding (`wallet/wallet.go:248-289`):

- `AddTSpend(tx)` logs info `TSpend arrived: <hash>` **before** checking the cache, then returns the plain error `tspend already cached` for a duplicate; the transaction is stored by value.
- The dcrd RPC syncer adds every transaction returned by `getrawmempool`-with-`tspend` at startup (errors ignored) and every `tspend` notification (the error is returned to the notification dispatcher) **without validating it** (`chain/sync.go:697-704,925-931`); dcrd is trusted. The SPV syncer adds a transaction only after `stake.CheckTSpend` passes, its expiry is not below the tip height, its public key is one of the network's Pi keys and its signature verifies (`spv/sync.go:982-1010`).
- Step 4 of the resolution slices `SignatureScript[66:99]` without a length check. For a cached transaction that is not a well-formed tspend (possible only on the unvalidated RPC path) this panics with a slice-bounds error, or with an index error when the transaction has no inputs.
- `GetAllTSpends` reads the tip height in its own View, then under `stakeSettingsLock` deletes every entry with `uint32(tipHeight) > Expiry` and returns pointers to copies of the remaining transactions in map order. It is called once per owned winning ticket inside the voting loop.

`VoteOnOwnedTickets` carries a source `TODO` noting that when blocks arrive faster than votes are processed the transaction store can be ahead of the block being voted on (`wallet/chainntfns.go:724-728`); nothing compensates for it.

`hasVotingAuthority`: for each address extracted from ticket output 0 that exposes a hash160, if the address manager knows that hash160 return `(true, HavePrivateKey(addr), err)` immediately.

### 5.12 Revocations

None. `RevokeOwnedTickets` returns nil. `signVoteOrRevocation` retains an `isVote == false` branch (sign input 0) that has no caller. Revocations appear in the wallet only as transactions received from the network; the committed `revocationFeeLimit` in tickets is retained for consensus-format compatibility.

### 5.13 Ticket buyer (`ticketbuyer/tb.go`)

`Run(ctx, passphrase)`:

1. If `passphrase` is non-empty: `wallet.Unlock(ctx, passphrase, nil)`; return its error.
2. Subscribe to main-tip-changed notifications (unbuffered channel); unsubscribe on return.
3. State: `nextIntervalStart = 0`, `expiry = 0`, `cancels = []`, `fatal = nil`.
4. Loop:
   - On `ctx.Done()`: return `fatal` if set, else `ctx.Err()`.
   - On a notification with no attached blocks: ignore.
   - Otherwise, with `tip` = last attached block hash:
     1. `RescanPoint(ctx)`: on error log debug `Skipping autobuyer actions: RescanPoint err: <err>`; if non-nil log debug `Skipping autobuyer actions: transactions are not synced`; in both cases skip this notification.
     2. `tipHeader = BlockHeader(tip)` (error logged at error level; skip). `height = tipHeader.Height`.
     3. If `height + 2 >= nextIntervalStart`:
        - call and clear every stored cancel function (aborting all in-flight purchases);
        - `W = StakeDiffWindowSize`; `nextIntervalStart = (height / W + 1) * W`;
        - if `height + 2 == nextIntervalStart`: log debug `Skipping purchase: next sdiff interval starts soon` and skip this notification **entirely** (no purchase and no change mixing);
        - `expiry = nextIntervalStart`; if `height + 1 == nextIntervalStart` then `expiry += W`.
     4. Prepare a once-only `sdiff` = `NextStakeDifficultyAfterHeader(ctx, tipHeader)` (error logged `Calculating sdiff failed: <err>`, value 0).
     5. Snapshot the config under the mutex.
     6. `multiple = 1`; if `cfg.Mixing`: `multiple = cfg.Limit` and `cfg.Limit = 1` (in the snapshot).
     7. Create a cancellable child context, remember its cancel function.
     8. If `cfg.BuyTickets`: start `multiple` goroutines each running: `sdiff()`; if 0 return; `buy(cancelCtx, passphrase, sdiff, expiry, &cfg)`; on error log error `Ticket purchasing failed: <err>` unless the error is `InsufficientBalance`, `context.Canceled` or `context.DeadlineExceeded`; if the error is kind `errors.Passphrase` set `fatal` and cancel the outer context.
     9. Always start one goroutine running `mixChange(ctx, &cfg)` (with the **outer** context); errors other than `context.Canceled` are logged `Account mixing failed: <err>`.

Window arithmetic, with `W = 144` on mainnet: a ticket bought at tip `height` can be mined no earlier than `height + 2` (split in `height + 1`). At `height ≡ W-2 (mod W)` nothing is bought. At `height ≡ W-1` the purchase targets the next window: price is the next window's and expiry is the start of the window after it. At all other heights expiry is the start of the next window, so an unmined ticket never crosses into a window with a different price. Purchases started in earlier blocks of the same window keep running until `height ≡ W-2`. Because the stored `nextIntervalStart` only advances inside the step-3 branch, that branch (and its cancel-all) runs on three consecutive tips: `height ≡ W-2`, `≡ W-1` and `≡ 0`. The cancel at `≡ 0` aborts the purchases that were started one block earlier at `≡ W-1`, so those have a single block interval to complete; purchases started at `≡ 0` or later run until the next `≡ W-2`. The change-mix goroutines are never cancelled by this mechanism.

`buy(ctx, passphrase, sdiff, expiry, cfg)`:

1. `n = NetworkBackend()` (error returned); `ctx = n.WrapContext(ctx)`.
2. If `passphrase` non-empty: `Unlock(ctx, passphrase, nil)` again (detects a changed passphrase; error returned and is fatal for `Run` when its kind is `Passphrase`).
3. `minconf = 1`, or `2` when mixing.
4. Count:
   - `Maintain != 0`: `spendable = AccountBalance(Account, minconf).Spendable`; if `spendable < Maintain` log debug `Skipping purchase: low available balance`, return nil; `buy = (spendable - Maintain) / sdiff`; if 0, same log, return nil; cap at `MaxFreshStakePerBlock`.
   - `Maintain == 0`: `buy = MaxFreshStakePerBlock` (20 on mainnet). The purchase path trims this by its insufficient-balance retries.
   - If `limit == 0 && mixing`: `buy = 1`; else if `limit > 0 && buy > limit`: `buy = limit`.
5. `PurchaseTickets` with `Count = buy`, `SourceAccount = Account`, `VotingAccount`, `MinConf = minconf`, `Expiry = expiry`, `Mixing`, `MixedAccount`, `MixedAccountBranch`, `MixedSplitAccount = TicketSplitAccount`, `ChangeAccount`, `VSPClient = tb.cfg.VSP` (read from the live config, not the snapshot, without the mutex).
6. For each returned ticket hash (also on partial failure) log info `Purchased ticket <hash> at stake difficulty <sdiff>`. Return the error.

`mixChange`: if `MixChange && Mixing`, `wallet.MixAccount(ctx, ChangeAccount, MixedAccount, MixedAccountBranch)`.

Further details of `Run` and `buy` (`ticketbuyer/tb.go:63-312`):

- **Passphrase handling.** `Wallet.Unlock(ctx, passphrase, nil)` is called once at the start of `Run` and again at the start of every `buy`. A nil timeout means "unlocked without a time limit": if the wallet was unlocked with a timeout (for example by `walletpassphrase`), that timeout is cancelled and the wallet stays unlocked. A wrong passphrase **locks the wallet** before the `errors.Passphrase` error is returned (the wallet logs `The wallet has been locked due to an incorrect passphrase.` when it was unlocked before). With an empty passphrase neither call is made and the buyer relies on the wallet already being unlocked; if it is locked, purchases fail when a private key is first needed (kind `errors.Locked` from the address manager), which is logged as `Ticket purchasing failed: <err>` on every eligible block and never ends `Run`.
- **Fatal error.** Only an error for which `errors.Is(err, errors.Passphrase)` holds is stored (under a private mutex) and cancels the context derived inside `Run`; `Run` then returns that error rather than `context.Canceled`. Such an error is also logged through the default branch (`Ticket purchasing failed: <err>`). If several purchase goroutines fail this way the last one to store wins.
- **Expiry between window boundaries.** `expiry` and `nextIntervalStart` are loop-local state that persists across notifications; they are recomputed only inside the step-3 branch. A purchase started at any other height reuses the `expiry` computed at the last boundary crossing, which is the start of the next window.
- **First notification.** `nextIntervalStart` starts at 0, so the first notification always enters the step-3 branch whatever the height.
- **Skipped notifications.** A notification that is skipped (not synced, header lookup failure, window edge) starts neither purchases nor change mixing, and a skipped window-edge notification has already cancelled the in-flight purchases.
- **Reorganisations.** Only the last attached block of each notification is used; detached blocks are ignored. If a reorganisation lowers the tip, `height + 2 >= nextIntervalStart` is simply evaluated with the new height (no special handling).
- **`sdiff`.** Computed lazily at most once per notification (`sync.OnceValue`) from the tip header with `NextStakeDifficultyAfterHeader`, shared by all purchase goroutines of that notification, and passed to `buy` only for the count computation and the `Purchased ticket` log line. `PurchaseTickets` recomputes the ticket price itself (5.3 step 4). When no purchase goroutine runs (`BuyTickets` false, or `multiple <= 0`) it is never computed.
- **`Limit` values.** `Limit` is a signed `int`. Without mixing: 0 or negative means "no limit other than `MaxFreshStakePerBlock`" (`limit > 0 && buy > limit` is the only cap). With mixing: `multiple = Limit` goroutines each buying one ticket, so 0 or negative buys nothing (Q19). The CLI option is an unsigned integer converted with `int(...)`; gRPC `RunTicketBuyer` passes its request field through.
- **Count when `Maintain != 0`.** `buy = int((spendable - Maintain) / sdiff)` uses integer division of atoms and ignores ticket, split and VSP fees; the purchase path's insufficient-balance retries correct an over-estimate. The balance is `AccountBalance(Account, minconf).Spendable` at the `minconf` chosen above.
- **Config snapshot.** The snapshot taken under the mutex is a shallow struct copy shared by pointer among all goroutines of the notification; with mixing its `Limit` was overwritten with 1 before they start. `buy` reads everything from the snapshot except `VSP` (Q20).
- **Subscription.** `NtfnServer.MainTipChangedNotifications()` returns a client whose channel `C` the loop reads and whose `Done()` is deferred. The loop body does no network I/O itself (only database reads for the rescan point and the tip header); purchases and mixing run on their own goroutines. Delivery semantics of the channel are owned by the notifications spec.

Startup wiring (`dcrwallet.go:289-366`): the buyer is started when `--enableticketbuyer` or `--mixchange` is set, after the passphrase was obtained (`--pass` or prompt). Accounts: purchase = `--purchaseaccount` (default `default`); voting = `--ticketbuyer.votingaccount`, else purchase (mixing without a voting account is refused: `cannot run mixed ticketbuyer without --votingaccount`); change = `--changeaccount`, else purchase; mixed and ticket-split accounts from the mixing options. `Maintain = --ticketbuyer.balancetomaintainabsolute`, `Limit = --ticketbuyer.limit` (default 1). The VSP client is created first when `--vsp.url` is set, with `MaxFee = --vsp.maxfee` (default 0.2 DCR), `FeeAcct` = purchase account, `ChangeAcct` = change account (or purchase account, with a warning, when no change account is set and mixing is off). Log `Starting auto transaction creator`; on exit with an error other than cancellation `Transaction creator ended: <err>`.

### 5.14 `StakePoolTicketFee` (`wallet/txrules/poolfees.go:35-95`)

```
poolFeeInt = int64(floor(poolFeePercent * 100.0))                  // float64 arithmetic
adjs       = ceil(TicketPoolSize / SubsidyReductionInterval)       // float64; 2 on mainnet (8192/6144)
subsidy    = CalcStakeVoteSubsidyV3(height, variant)               // variant as in 4.2
repeat adjs times: subsidy = subsidy * 100 / 101                   // int64, truncating
num = ((poolFeeInt * subsidy * (stakeDiff + relayFee)) << 64) / (10000 * (subsidy + stakeDiff))   // big integers, floor
fee = num >> 64
```

`relayFee` is a misnomer: the purchase path passes the ticket's absolute fee. The subsidy cache is a process global initialised once.

### 5.15 Prune hook (`wallet/chainntfns.go:241-274`)

After attaching blocks, inside the chain-switch Update, `txStore.PruneUnmined(dbtx, sdiff)` removes expired unmined transactions, unmined tickets whose output-0 value differs from the next ticket price (when that price is known), and unmined votes not voting on the tip. For **every** removed hash (ticket or not): notify removal, call `RemoveTicket(hash, "pruned")` on every VSP client (iterating `w.vspClients` without `vspClientsMu`), and `DeleteVSPTicket(dbtx, hash)` (failure logged `Failed to remove pruned ticket from db: <err>`).

### 5.16 How callers fill `PurchaseTicketsRequest`

| Field | JSON-RPC `purchaseticket` (`internal/rpc/jsonrpc/methods.go:3356-3518`) | gRPC `PurchaseTickets` (`internal/rpc/rpcserver/server.go:1791-1940`) | Ticket buyer |
| --- | --- | --- | --- |
| `Count` | `numtickets` if greater than 1, else 1 | `num_tickets`; less than 1 is `InvalidArgument`, `Zero or negative number of tickets given` | computed (5.13) |
| `SourceAccount` | account number of `fromaccount` | `account` | `Account` |
| `VotingAccount` | mixed account when the server runs with mixing, else source account | `voting_account` if `use_voting_account`, else mixed account when `enable_mixing`, else `account` | `VotingAccount` |
| `MinConf` | `minconf`, default 1, negative rejected | `required_confirmations` | 1, or 2 when mixing |
| `Expiry` | `expiry`, default 0 | `expiry` | window boundary |
| `DontSignTx` | `dontsigntx` | `dont_sign_tx` | false |
| `Mixing` | server config (mixing enabled) | `enable_mixing` | `Mixing` |
| `MixedAccount`, `MixedAccountBranch`, `MixedSplitAccount` | server config (mixed account, branch 0 or 1, ticket split account) | request fields (accounts must exist; branch 0 or 1) | config |
| `ChangeAccount` | source account; the configured change account when mixing | `change_account` | `ChangeAccount` |
| `VSPClient` | when a VSP URL is configured: `VSP{URL, PubKey, Policy{MaxFee: VSPMaxFee(), FeeAcct: source, ChangeAcct: change}}` | when `vsp_host` or `vsp_pubkey` is non-empty: same with `FeeAcct: account`, `ChangeAcct: change_account` | the buyer's configured client |

The gRPC handler unlocks the wallet with the request passphrase for the duration of the call when signing and a non-empty passphrase was supplied; it validates `tx_fee >= 0` (`Negative fees per KB given`) but does not use the value. gRPC `RunTicketBuyer` builds its VSP policy with both `FeeAcct` and `ChangeAcct` set to the purchase account (`internal/rpc/rpcserver/server.go:2596-2610`). Because of Q10, whichever of these callers first creates the client for a URL fixes its policy.

---

## 6. Constants, limits and defaults

| Name | Value | Source | Depends on it |
| --- | --- | --- | --- |
| `revocationFeeLimit` | 16384 atoms (`1 << 14`) | `wallet/createtx.go:40` | bytes 30-31 of every ticket commitment (`00 4e`) |
| vote fee limit | 0 (no fees allowed) | `wallet/createtx.go:967-968` | same |
| `sanityVerifyFlags` | discourage-upgradable-NOPs, clean stack, CLTV, CSV, treasury | `wallet/createtx.go:45-49` | self-check of signed tickets and unmined wallet-spending transactions |
| ticket size estimate | 298 bytes | `wallet/createtx.go:1235-1239` | `ticketFee`, split output value, fee on every ticket |
| `RedeemP2PKHSigScriptSize` | 108 | `wallet/txsizes/size.go:28` | all size estimates |
| `RedeemP2PKHInputSize` | 166 (`32+4+1+8+4+4+1+108+4`) | `wallet/txsizes/size.go:55` | (mis)used in the UTXO-split estimate (Q4). Not the literal 165 ("average" input size) hard-coded in the dust rule |
| `P2PKHPkScriptSize` | 25 | `wallet/txsizes/size.go:66` | change sizing, dust |
| `TicketCommitmentScriptSize` | 32 | `wallet/txsizes/size.go:99` | ticket estimate |
| `DefaultRelayFeePerKb` | 10000 atoms/kB | `wallet/txrules/rules.go:38` | default fee rate; high-fee threshold is 1000 times this |
| `FeeForSerializeSize` | `rate*size/1000`; if that is 0 and rate > 0 then `rate`; clamped to `MaxAmount` on overflow | `wallet/txrules/rules.go:94-106` | all fees |
| dust rule | `amount*1000/(3*(8+2+varint(len)+len+165)) < rate` | `wallet/txrules/rules.go:43-56` | fee-tx change |
| mixed ticket trickle | each ticket at `now + 20 s + uniform[0, 40 s)`, sorted | `wallet/createtx.go:1487-1500` | ticket publish timing when mixing |
| smallest mix change | `FeeForSerializeSize(rate, 217) + 262144` atoms | `wallet/mixing.go:214-234` | whether a mixed split keeps its change output |
| mixed split PR expiry | `mixing.MaxExpiry(tip, params) - 2` | `wallet/mixing.go:171-174` | pair request expiry height |
| random UTXO probes | `unspentCount / 2` | `wallet/createtx.go:1908-1914` | selection performance only |
| fee-tx reservation minconf | 1 | `wallet/wallet.go:4673` | which UTXOs can pay a VSP fee when none were supplied |
| `immatureJitter` | 1 hour | `wallet/feepayment.go:33` | job delay upper bound, immature ticket |
| `liveJitter` | 5 minutes | `wallet/feepayment.go:34` | job delay upper bound, live ticket |
| `unminedJitter` | 2 minutes | `wallet/feepayment.go:35` | job delay upper bound, unmined or expired ticket |
| `State` values | 1 `Unprocessed`, 2 `FeePublished`, 4 `TicketSpent` | `wallet/feepayment.go:61-69` | gRPC `GetTrackedVSPTickets.state` |
| `FeeStatus` values | 0 started, 1 paid, 2 errored, 3 confirmed | `wallet/udb/vsp.go:25-36` | bytes 32-35 of `vsp` records; gRPC `GetVSPTicketsByFeeStatus` |
| VSP ticket record size | 40 bytes | `wallet/udb/vsp.go:165-179` | database compatibility |
| `vsphostindex` key | ASCII `vsphostindex` | `wallet/udb/txdb.go:162` | host ID allocation |
| required confs constant | `6 + 2` | `wallet/wallet.go:5734` | lower bound of the ticket scan in `ForUnspentUnexpiredTickets` |
| vote version | mainnet 11, testnet3 12, simnet 12, testnet2 6, other 1 | `wallet/wallet.go:291-304` | bytes 4-7 of the vote-bits script; which agendas exist; `votechoices` sent to VSPs |
| block-valid bit | `0x0001` | `wallet/chainntfns.go:784` | default vote bits; disapproval |
| tspend key offset | signature script bytes `[66:99]` | `wallet/wallet.go:615` | treasury key policy lookup |
| treasury vote bytes | `0x01` yes, `0x02` no | `dcrd/blockchain/stake/v5@v5.0.3/staketx.go:825-837` | vote output, policy buckets |
| `MaxInputsPerSStx` | 64 | `dcrd/blockchain/stake/v5@v5.0.3/staketx.go:46` | `IsSStx` |
| `SStxPKHMinOutSize` / `MaxOutSize` | 32 / 77 | same file, 81-85 | commitment parsing |
| `MaxOutputsPerSStx` | 129 (`64*2 + 1`) | same file, 48-50 | `IsSStx` |
| `NumInputsPerSSGen` / `MaxOutputsPerSSGen` | 2 / 66 | same file, 52-67 | `IsSSGen` |
| `NumInputsPerSSRtx` / `MaxOutputsPerSSRtx` | 1 / 64 | same file, 69-76 | `IsSSRtx` |
| `TxVersionAutoRevocations` | 2 | same file, 41-43 | extra `IsSSRtx` rules |
| `SSGenBlockReferenceOutSize` | 38 | same file, 87-89 | vote output 0 |
| `SSGenVoteBitsOutputMinSize` / `MaxSize` | 4 / 77 | same file, 91-97 | vote output 1 |
| `MaxSingleBytePushLength` | 75 | same file, 99-101 | commitment and vote-bits push range |
| `SSGenVoteBitsExtendedMaxSize` | 73 | same file, 103-111 | upper bound on extended vote bits (the wallet uses 4) |
| `SStxVoteReturnFractionMask`, `SStxRevReturnFractionMask`, `SStxRevReturnFractionShift`, `SStxVoteFractionFlag`, `SStxRevFractionFlag` | `0x003f`, `0x3f00`, 8, `0x0040`, `0x4000` | same file, 113-143 | decoding of commitment bytes 30-31 (the wallet's `00 4e` decodes to: vote flag off, revocation flag on, revocation limit exponent 14) |
| `MaxDataCarrierSize` | 256 | same file, 149-151 | null-data push limit; caps treasury votes per vote at 7 |
| `VoteConsensusVersionAbsent` | 0 | same file, 145-147 | `SSGenVersion` of a short vote-bits script |
| `TSpendScriptLen` | 100 | `dcrd/blockchain/stake/v5@v5.0.3/treasury.go:20` | tspend signature script; key at `[66:99]` |
| treasury-vote discriminator | bytes `'T' 'V'` (`54 56`) | `wallet/chainntfns.go:805`, `dcrd/blockchain/stake/v5@v5.0.3/staketx.go:893` | first two payload bytes of the optional last vote output |
| `wire.TxVersionTreasury` | 3 | `dcrd/wire@v1.8.0/msgtx.go:28` | version of a vote carrying treasury votes; minimum version for treasury tx classification |
| job task names | `reconcile payment`, `confirm payment` | `wallet/feepayment.go:118,466,572,580,585,594,602` | log lines |
| VSP policy map limit | at most 3 entries in `tspendpolicy` and in `treasurypolicy` | `vspd/types/v3@v3.0.0/types.go:54-55,67-68` (`binding:"max=3"`) | server-side rejection of larger `payfee`/`setvotechoices` requests (4.6.3) |
| `ValidPoolFeeRate` bounds | `floor(rate*100)` in `[1, 10000]` | `wallet/txrules/poolfees.go:19-23` | exported validator, unused here |
| `DCP0001` activation heights | mainnet 149248, testnet2 46128, testnet3 0, simnet 0 | `deployments/deployments.go:32-37` | whether the ticket price is computed locally |
| `StakePoolTicketFee` decay factor | `subsidy * 100 / 101`, applied `ceil(TicketPoolSize / SubsidyReductionInterval)` times | `wallet/txrules/poolfees.go:54-71` | VSP fee estimate |
| tspend cache eviction | entry dropped when `uint32(tipHeight) > Expiry` | `wallet/wallet.go:282` | which tspends a vote can carry |
| ticket buyer minconf | 1; 2 when mixing | `ticketbuyer/tb.go:252-255` | spendable balance and input selection |
| ticket buyer max per attempt | `MaxFreshStakePerBlock` (20 mainnet) | `ticketbuyer/tb.go:275-281` | purchase count |
| `--ticketbuyer.limit` default | 1 | `config.go:67,383` | purchases per block |
| `--ticketbuyer.balancetomaintainabsolute` default | 0 | `config.go:66` | |
| `--vsp.maxfee` default | 0.2 DCR | `config.go:63` | reject VSP fee above this |
| `--purchaseaccount` default | `default` | `config.go:53` | |
| `--enableticketbuyer`, `--enablevoting` defaults | false | `config.go:51-52` | |
| VSP API paths | `/api/v3/vspinfo`, `/feeaddress`, `/payfee`, `/ticketstatus`, `/setvotechoices` | `vspd/client/v4@v4.0.2/client.go:37,53,87,109,143` | |
| VSP headers | `VSP-Client-Signature`, `VSP-Server-Signature` | same file, 186, 259 | |
| message-signing prefix | `Decred Signed Message:\n` | `wallet/wallet.go:2011` | request signatures |
| DCP0010 / DCP0012 agenda IDs | `changesubsidysplit`, `changesubsidysplitr2` | `deployments/deployments.go:111,145` | vote subsidy variant |
| mainnet parameters used | `TicketMaturity 256`, `TicketExpiry 40960`, `StakeDiffWindowSize 144`, `MaxFreshStakePerBlock 20`, `StakeValidationHeight 4096`, `CoinbaseMaturity 256`, `SStxChangeMaturity 1`, `TicketPoolSize 8192`, `SubsidyReductionInterval 6144`, `TreasuryVoteInterval 288`, multiplier 12, `TargetTimePerBlock 5 min` | `dcrd/chaincfg/v3@v3.3.0/mainnetparams.go:105-586` | heights, jitter, windows |

---

## 7. Concurrency and lifecycle

### 7.1 Mutexes

| Lock | Protects | Notes |
| --- | --- | --- |
| `Wallet.lockedOutpointMu` | locked set | Taken before opening a database transaction. Held for the whole Update that records a split or ticket. `VoteOnOwnedTickets` deliberately does not take it. |
| `Wallet.recentlyPublishedMu` | `recentlyPublished` set | Taken inside the ticket-recording Update (order: `lockedOutpointMu` → Update → `recentlyPublishedMu`). |
| `Wallet.stakeSettingsLock` | default vote bits, voting flag, tspend cache, four policy maps | `SetTSpendPolicy`/`SetTreasuryKeyPolicy` hold it across a database Update; `VoteOnOwnedTickets` takes it while inside a View. The Go database lets a reader and a writer proceed concurrently, so this does not deadlock; a port must preserve that property or reorder. |
| `Wallet.vspClientsMu` | `vspClients` map | Not taken by the prune hook (Q25). |
| `VSPClient.mu` | `jobs` map | Never held while calling into a job. |
| `vspFeePayment.mu` | `fee`, `feeAddr`, `feeHash`, `feeTx`, `state`, `err` | Never held across I/O. |
| `vspFeePayment.timerMu` | the timer | |
| `VSPTicket.mu` | cached live/expiry heights | Held across a database View in `calcHeights`. |
| `TB.mu` | ticket buyer config | Not used for the `VSP` field read in `buy` (Q20). |
| `fatalMu` (local to `TB.Run`) | the `fatal` error | Written by purchase goroutines, read by the loop on context cancellation. |
| `txrules.initSubsidyCacheOnce` | one-time creation of the package subsidy cache | 3.12. |

Additional lock facts:

- `VSPTicket.TSpendPolicy` and `VSPTicket.TreasuryKeyPolicy` take `Wallet.stakeSettingsLock` and scan the whole per-ticket map (linear in the number of per-ticket policies across all tickets). They are called from `submitPayment` on the job's goroutine with no other lock held.
- `feePayment` (job creation) reads and writes the new job's fields without `vspFeePayment.mu`; this is safe only because the job is not yet published in `jobs`. Two concurrent creations for the same ticket both run the database queries and the `UpdateFeeStarted` write; the loser's job object is discarded and only the winner's timer is armed.
- `Process` holds no lock across `receiveFeeAddress`, `makeFeeTx` and `submitPayment`; `fp.mu` is taken only for the brief `feeTx` adoption.
- `remove` calls `stop()` (takes `timerMu`) before taking `VSPClient.mu`; no code path holds both.
- `ProcessUnprocessedTickets` checks `jobs` under `VSPClient.mu` once per ticket before starting its goroutine; a ticket appearing twice in the input, or a job created between the check and `Process`, is handled by `feePayment`'s get-or-create.

### 7.2 Goroutines and timers

- **Fee payment timers**: one `time.AfterFunc` per job, each firing on its own goroutine. Two steps of the same job can overlap: a timer callback and a synchronous `Process` call are not mutually excluded, and replacing a timer does not wait for a running callback.
- **Job context**: always `context.Background()`. Job HTTP requests and database calls are not cancelled by wallet shutdown; pending timers simply die with the process. No shutdown hook stops jobs.
- **`ProcessUnprocessedTickets`**: one goroutine per ticket, all started before any is awaited. Within each synchronous `Process` the caller's context covers only the initial `vsp` record read, the job-creation queries and the errored-status write; the job steps themselves (`receiveFeeAddress`, `makeFeeTx`, `submitPayment`, `confirmPayment`, and their HTTP requests) run on the job's `context.Background()` and cannot be cancelled by the caller.
- **Ticket buyer**: one loop goroutine; per tip change up to `multiple` purchase goroutines plus one change-mix goroutine. Purchase goroutines are cancelled as a group at window boundaries and when `Run`'s context ends; they are not joined. The main package waits for `Run` to return before exiting (`dcrwallet.go:357-365`).
- **Purchase**: `PurchaseTickets` is synchronous. With mixing it can block for the whole mix epoch plus up to 60 s of trickle; cancellation during trickle returns a partial response.
- **Voting**: runs on the dcrd notification handler goroutine; a slow vote delays later notifications from the same connection.

### 7.3 Ordering guarantees observers rely on

1. Split transaction is recorded in the wallet, then published, before any ticket spending it is published.
2. Each ticket is recorded (and its hash put in `recentlyPublished`) before it is published; tickets are published one at a time in split-output order.
3. The VSP is contacted for ticket `i` only after ticket `i` was published, and before ticket `i+1` is published.
4. A VSP fee transaction is stored unpublished before it is sent to the VSP; the wallet does not broadcast it.
5. Votes are published before they are recorded.
6. Watch-list updates (`LoadTxFilter`) for purchased tickets happen after publication (the source notes this is the wrong way round).

---

## 8. Errors

### 8.1 Wallet error kinds produced here (`errors` package of dcrwallet)

| Kind (string) | Where | Message |
| --- | --- | --- |
| `Invalid` (`invalid operation`) | `PurchaseTickets` | `wallet mixing support is disabled` |
| `Invalid` | `purchaseTickets` | `negative minconf`; `negative expiry`; `expiry height must be above next block height` |
| `Invalid` | ticket address derivation | `account does not return compatible stake addresses` |
| `Invalid` | `makeTicket` | `nil vote address` |
| `Invalid` | mixed split | `previous output is not P2PKH`; `mixing client is not running` |
| `Invalid` | policy setters | `invalid treasury vote policy 0x<n>` (Go `%#x`) |
| `Invalid` | `SetAgendaChoices` | ``no agenda with ID "<id>"``; ``agenda "<id>" has no choice ID "<id>"`` |
| `Bug` (`internal wallet error`) | `makeTicket` | op `stake.CheckSStx` wrapping the stake rule error |
| `Bug` | `signP2PKHMsgTx` | `previous output address is not P2PKH` |
| `Bug` | deployments | `DCP0010 activation check requires RPC syncer`; `DCP0012 ...` |
| `InsufficientBalance` (`insufficient balance`) | selection, reallocation, split | bare kind, or wrapped with op |
| `Policy` (`policy violation`) | `checkHighFees` | `high fee` |
| `ScriptFailure` | `validateTxInput`, `signVoteOrRevocation` (op `txscript.SignTxOutput`) | engine error |
| `NotExist` | `LookupVSP` | ``VSP client for "<host>" not found`` |
| `NotExist` | `GetVSPTicket` | `no VSP info for ticket <hash>` |
| `NotExist` | `VSPFeeHashForTicket`, `VSPHostForTicket` | bare kind (zero hash / empty host) |
| `NotExist` | `AgendaChoices`, `SetAgendaChoices` | `ticket not found` |
| `Deployment` (`inactive deployment`) | `NextStakeDifficulty` | `DCP0001 is not known to be active` (triggers RPC fallback) |
| `IO` | policy buckets | `invalid length <n> for tspend vote policy` / `... treasury key policy` |
| `Locked` | any signing or key export while locked | from the address manager |
| `Passphrase` | ticket buyer unlock | terminates `TB.Run` |

Plain (unclassified) errors: `vsp url can not be null`, `vsp pubkey can not be null`, `fee payment cannot be processed`, `ticket already paid or confirmed with another vsp`, `fee processing stopped`, `server fee address invalid: ...`, `server fee amount too high: <a> > <b>`, `unable to create VSP fee tx for ticket <hash>: ...`, `payfee: ...`, `unable to reserve outputs: ...`, `not enough input value to pay fee: <a> < <b>`, the `NewVSPTicket` messages in 5.7, `network backend does not provide deployment information`, `tspend already cached`, `no agendas to set for this network`, `Number of prevOutputs (<n>) does not match number of tx inputs (<m>)`.

Amounts in messages use Go `dcrutil.Amount` formatting (for example `0.2 DCR`).

Further errors produced in scope that the table above does not list:

| Kind | Where | Message |
| --- | --- | --- |
| `NotExist` | `GetTransactionsByHashes` under `NewVSPTicket` and `VSPTicket.FeeTx` | `transaction(s) not found` (prefixed by `failed to retrieve ticket: ` / `failed to retrieve parent <hash> of ticket: ` in `NewVSPTicket`; unprefixed from `FeeTx`, then wrapped by job creation as `fee transaction not found in wallet: ...`, which is only stored in the job) |
| `NotExist` | `GetVSPHost`, `GetVSPPubKey` | `no VSP host info for id <pointer>`, `no VSP pubkey info for host <pointer>` (4.4) |
| `IO` | `SetVSPTicket` | bare wrap of the database error when writing `vsphostindex` |
| `IO` | the four `Set*Policy` functions | bare wrap of the database error when deleting a key |
| none (op only) | `signP2PKHMsgTx` | op `txscript.SignatureScript` wrapping the signing error |
| none (op only) | `VoteOnOwnedTickets` | op `wallet.VoteOnOwnedTickets` wrapping `NetworkBackend()`, `DCP0010Active` and `DCP0012Active` errors, and the string `network backend does not provide deployment information` |
| none (op only) | `GetVSPTicketsByFeeStatus` | op `wallet.GetVSPTicketsByFeeStatus` wrapping the View error |
| none (op only) | `AgendaChoices`, `SetAgendaChoices` | ops `wallet.AgendaChoices`, `wallet.SetAgendaChoices` wrapping database errors; `no agendas to set for this network` is `errors.E` with only a string |
| plain | `CreateVspPayment` | ``failed to convert '<Go type>' to wallet.Address`` (unreachable); the concatenated per-input signing errors (only the last survives, Q33) |
| plain | `AddTSpend` | `tspend already cached` |
| stake `RuleError` | `makeTicket` via `SStxNullOutputAmounts` | `committed amount was too small!` when the ticket price is not positive (returned unwrapped, no wallet error kind) |
| stake `RuleError` | `NewVSPTicket` via `AddrFromSStxPkScrCommitment` | `cannot parse commitment address: short read of sstx commit pkscript` (unreachable after `IsSStx`) |
| chainhash error | `ProcessManagedTickets` | the `chainhash.NewHashFromStr` error for a malformed `feetxhash` from the VSP, returned unwrapped and aborting the remaining tickets |
| `context` errors | `purchaseTickets` | `errors.E(op, ctx.Err())` when cancelled before or during ticket publication; a partial response accompanies it |

Go `%v`/`%s` renderings a port must match in these messages: hashes print in the reversed-hex display form; outpoints print as `<hash>:<index>`; `dcrutil.Amount` prints as a decimal DCR value with the unit suffix (`0.0000298 DCR`); `time.Duration` prints in Go's compound form; `%q` produces a Go double-quoted string with Go escapes; `%x` of a byte slice is lower-case hex; `%#x` of a policy byte is `0x` plus lower-case hex without padding (for example `0x3`).

### 8.2 VSP client errors (`vspd/client/v4@v4.0.2/client.go`)

Prefixes, in the order they can occur: `marshal request: `, `sign request: `, `new request: `, `<METHOD> <url>: `, `read response body: `, `http status <n> (<text>) with no body`, `http status <n> (<text>) with body "<quoted>"`, `authenticate server response: no signature provided | failed to decode signature: <err> | invalid signature`, `unmarshal response body: `, `server response contains differing request`. A decoded `ErrorResponse` is returned unwrapped and prints only its `message`.

### 8.3 VSP error codes (`vspd/types/v3@v3.0.0/errors.go`)

| Code | Name | HTTP status | Default message | Wallet reaction |
| --- | --- | --- | --- | --- |
| 0 | `ErrBadRequest` | 400 | `bad request` | generic |
| 1 | `ErrInternalError` | 500 | `internal error` | generic |
| 2 | `ErrVspClosed` | 400 | `vsp is closed` | generic |
| 3 | `ErrFeeAlreadyReceived` | 400 | `fee tx already received for ticket` | in `reconcilePayment`: mark fee tx published, status paid, proceed to confirmation |
| 4 | `ErrInvalidFeeTx` | 400 | `invalid fee tx` | in `reconcilePayment`: status errored, discard fee tx, retry |
| 5 | `ErrFeeTooSmall` | 400 | `fee too small` | generic |
| 6 | `ErrUnknownTicket` | 400 | `unknown ticket` | `ProcessManagedTickets`: skip ticket |
| 7 | `ErrTicketCannotVote` | 400 | `ticket not eligible to vote` | `reconcilePayment` while creating the fee tx: remove job |
| 8 | `ErrFeeExpired` | 400 | `fee has expired` | `submitPayment`: abandon fee tx, clear it |
| 9 | `ErrInvalidVoteChoices` | 400 | `invalid vote choices` | generic |
| 10 | `ErrBadSignature` | 400 | `bad request signature` | generic |
| 11 | `ErrInvalidPrivKey` | 400 | `invalid private key` | generic |
| 12 | `ErrFeeNotReceived` | 400 | `no fee tx received for ticket` | generic |
| 13 | `ErrInvalidTicket` | 400 | `not a valid ticket tx` | generic |
| 14 | `ErrCannotBroadcastTicket` | 500 | `ticket transaction could not be broadcast` | generic |
| 15 | `ErrCannotBroadcastFee` | 500 | `fee transaction could not be broadcast` | same as code 4 |
| 16 | `ErrCannotBroadcastFeeUnknownOutputs` | 428 | `fee transaction could not be broadcast due to unknown outputs` | generic |
| 17 | `ErrInvalidTimestamp` | 400 | `old or reused timestamp` | generic |
| other | — | 500 | `unknown error` | generic |

"Generic" means: in a timer step the error is logged and, in `reconcilePayment` after `submitPayment`, the step is rescheduled; in a synchronous call it is returned to the caller, except that any error from `feeaddress` or from fee-transaction creation inside `Process` is swallowed (5.8.3 steps 2.3 and 2.4).

### 8.4 Surfacing

- Purchase errors return to the RPC caller through the RPC layer's error mapping (owned by the RPC specs). VSP failures during a purchase do not surface at all (5.3 step 13).
- Timer-step errors are visible only in the `VSPC` log and transiently in the job's `err` field (not exported).
- Ticket buyer errors are logged under `TKBY`; only a passphrase error ends the buyer.
- Voting errors are logged by the syncer (`chain/sync.go:775-778`); they never end the sync loop.

### 8.5 Log lines (subsystems: wallet package logger, `VSPC` for VSP clients, `TKBY` for the ticket buyer)

Info: `Selected input <op> (<amt>) for ticket purchase split transaction`, `Completed CoinShuffle++ mix of ticket split transaction <hash>`, `Reserved credits for <n> tickets: total fee: <amt>`, `Split output is <op>`, `Published ticket purchase <hash>`, `Voting on block <hash> (height <h>) using ticket <hash> (vote hash: <hash> bits: <n>)`, `Disapproving block <hash> voted with ticket <hash>`, `TSpend arrived: <hash>`, `[VSPC] VSP requires fee <amt>`, `[VSPC] Successfully processed <hash>`, `[VSPC] VSP has broadcast the fee tx for <hash>`, `[VSPC] Ticket <hash> is <reason>; removing from VSP client`, `[TKBY] Purchased ticket <hash> at stake difficulty <amt>`. The remaining warn/error/debug lines are quoted where they occur in section 5.

Complete inventory of the other lines, by logger and level (format verbs resolved; `<t>` is a ticket hash):

| Logger | Level | Line | Source |
| --- | --- | --- | --- |
| wallet | error | `Failed to watch for future addresses after ticket purchases: <err>` | `wallet/createtx.go:1255` |
| wallet | error | `Failed to watch outpoints: <err>` | `wallet/createtx.go:1261`, `wallet/chainntfns.go:900` |
| wallet | error | `ReserveOutputsForAmount failed: <err>` | `wallet/createtx.go:1332,1350` |
| wallet | error | `Script validation failed (outpoint <op> pkscript <hex> sigscript <hex>): <err>` | `wallet/createtx.go:739` |
| wallet | debug | `Unlocked unneeded credit for VSP fee tx: <outpoint>` | `wallet/createtx.go:1308` |
| wallet | debug | `Unlocked credit for ticket tx: <outpoint>` | `wallet/createtx.go:1357` |
| wallet | debug | `<outpoint> reserved for vsp fee transaction` | `wallet/createtx.go:1426` |
| wallet | debug | `Unspent bucket k/v count: <n>`; `Abandoned random UTXO selection attempts after <n> tries` | `wallet/createtx.go:1829-1987` |
| wallet | error | `Failed to query block which mines ticket: <err>` | `wallet/vspticket.go:115` |
| wallet | warn | `Failed to get new change address: <err>` | `wallet/wallet.go:4705` |
| wallet | error | `Failed to sign transaction: <err>` followed by one line `\t<sigErr>` per failed input | `wallet/wallet.go:4739-4742` |
| wallet | error | `Failed to read ticket purchase transaction for owned winning ticket <t>: <err>` | `wallet/chainntfns.go:752` |
| wallet | warn | `Block disapprove percent set on mainnet` | `wallet/chainntfns.go:778` |
| wallet | error | `Failed to create vote transaction for ticket hash <t>: <err>` | `wallet/chainntfns.go:797` |
| wallet | error | `Failed to create treasury vote for ticket hash <t>: <err>` | `wallet/chainntfns.go:834` |
| wallet | error | `Failed to sign vote for ticket hash <t>: <err>` | `wallet/chainntfns.go:847` |
| wallet | error | `View failed: <err>` (the error is wrapped with op `wallet.VoteOnOwnedTickets`) | `wallet/chainntfns.go:859` |
| wallet | error | `Failed to create transaction record: <err>` (the vote is still published, not recorded) | `wallet/chainntfns.go:876` |
| wallet | error | `Failed to send one or more votes: <err>` | `wallet/chainntfns.go:894` |
| wallet | error | `Failed to remove pruned ticket from db: <err>` | `wallet/chainntfns.go:272` |
| `VSPC` | error | the bare error from `Process` (`ProcessUnprocessedTickets`) | `wallet/vsp.go:136` |
| `VSPC` | debug | `Ticket <t> already added to vsp client`; `Ticket <t> not known by vsp`; `Ticket <t> not added to vsp client, vsp status error: <err>`; `Ticket <t> already confirmed by vsp` | `wallet/vsp.go:155-191` |
| `VSPC` | error | `Could not check status of VSP ticket <t>: <err>` | `wallet/vsp.go:302` |
| `VSPC` | debug | `VSP already has correct vote choices for ticket <t>`; `Updating vote choices on VSP for ticket <t>` | `wallet/vsp.go:350,354` |
| `VSPC` | debug | ``Scheduling "<task>" for ticket <t> in <duration>`` | `wallet/feepayment.go:187` |
| `VSPC` | error | `Ticket <t>: <task>: <err>` | `wallet/feepayment.go:223` |
| `VSPC` | error | `VSP fee payment for ticket <t> failed: <err>`; `Error abandoning expired fee tx <err>` | `wallet/feepayment.go:532,541` |
| `VSPC` | warn | `Rescheduling status check for <t>: VSP err: <err>`; `Rescheduling status check for <t>: db err: <err>`; `VSP failed to broadcast feetx for <t> -- restarting payment`; ``VSP responded with unknown FeeTxStatus "<s>" for <t>`` | `wallet/feepayment.go:571-607` |
| `VSPC` | trace | `Request to <URL>\n<dump>\n`; `Response from <URL>\n<dump>\n`; `VSP request dump failed: <err>`; `VSP response dump failed: <err>` | `vspd/client/v4@v4.0.2/client.go:189-211` |
| `TKBY` | debug | `Skipping autobuyer actions: RescanPoint err: <err>`; `Skipping autobuyer actions: transactions are not synced`; `Skipping purchase: next sdiff interval starts soon`; `Skipping purchase: low available balance` | `ticketbuyer/tb.go:104,108,138,266,272` |
| `TKBY` | error | the bare `BlockHeader` error; `Calculating sdiff failed: <err>`; `Ticket purchasing failed: <err>`; `Account mixing failed: <err>` | `ticketbuyer/tb.go:114,156,188,208` |

`remove(reason)` reasons that can appear in `Ticket <t> is <reason>; removing from VSP client`: `expired`, `spent`, `confirmed by VSP`, `ticket cannot vote`, `pruned`.

---

## 9. Compatibility-critical behaviours

1. **Ticket byte layout** (4.1): one input, three outputs, commitment limits bytes `00 4e`, zero-hash stake change output with value 0, committed amount equal to the input value, version 1. dcrd consensus and VSPs (which require exactly this 3-output shape with a P2PKH commitment) depend on it.
2. **Ticket fee and split output value**: `ticketPrice + FeeForSerializeSize(relayFee, 298)`. Deviating changes fee rates seen by mempools and breaks the "perfectly sized" split assumption (a ticket must have zero change).
3. **Ticket expiry semantics**: `Expiry` field equals the requested height; 0 means none; the ticket buyer always sets the next window boundary (5.13).
4. **Address derivation side effects on an existing `wallet.db`**: per ticket, one internal-branch address of the voting account and one internal-branch address of the source (or mixed-branch address of the mixed) account are taken with gap policy *ignore*, and address rows are written for the voting account's **external** branch up to the voting child index (`signingAddressAtIdx`, 5.3 step 11.3); the non-mixed split takes one internal address of the source account per attempt (gap policy wrap); fee transactions take one change-account internal address each (gap policy wrap). Each next-address call advances the in-memory branch cursor and persists address rows through `SyncAccountToAddrIndex` (`signingAddressAtIdx` writes rows only and moves no cursor); at this pin that persistence step does not write the last-returned (`extret`/`intret`) variables (`wallet/addresses.go:468-502`; the counter semantics are owned by the address-manager spec). A drop-in replacement opening the same database must write the same address rows so that both implementations recognise each other's addresses.
5. **`vsp`, `vsphost`, `vsppubkey` bucket encodings** (4.4), including big-endian integers, the `vsphostindex` counter, host ID 0 as the empty host, and host strings being full normalised URLs.
6. **Policy bucket encodings** (4.5): one-byte values, deletion on abstain, 64-byte and 32+33-byte composite keys.
7. **Fee status numbering** 0/1/2/3 and `State` numbering 1/2/4 as exposed by gRPC `GetVSPTicketsByFeeStatus` and `GetTrackedVSPTickets`.
8. **VSP request signing** (4.6.1): Decred signed-message hash of the exact body bytes, compact recoverable signature with compressed flag, by the commitment address key, standard base64 in `VSP-Client-Signature`.
9. **VSP response authentication** (4.6.2): Ed25519 over the raw body with the configured key; reject missing/invalid signatures on 200 responses; require the echoed `request` to equal the sent body byte for byte.
10. **VSP JSON field names and types** (4.6.3), maps never `null` in `payfee`/`setvotechoices`, `timestamp` in Unix seconds, hashes as reversed hex, transactions as lower-case hex of the full serialization, voting key as network WIF.
11. **`votechoices` content**: every agenda of the wallet's vote version, with `abstain` defaults. vspd validates agenda IDs and choices against its own network parameters, so the vote version table (3.2) must match.
12. **Fee transaction handling**: stored in the wallet as unmined and unpublished so that its inputs are not reused and it is not rebroadcast; marked published only on `ErrFeeAlreadyReceived`; abandoned on `ErrFeeExpired`.
13. **Fee cap**: refuse when the VSP's `feeamount` exceeds the configured maximum. On the synchronous `Process` path the status is written as errored and `Process` still returns nil; on the timer path (`reconcilePayment` → `makeFeeTx` → `receiveFeeAddress`) no status is written and the job goes idle (Q14).
14. **Vote byte layout** (4.2): stakebase input with the network's stakebase script and `ValueIn` equal to the subsidy, block reference, 6-byte vote-bits push carrying the vote version, reward outputs from `CalculateRewards`, optional `TV` output with transaction version 3. dcrd rejects anything else.
15. **Vote subsidy variant selection** from dcrd's agenda status with the `lockedin` boundary rule (3.11), evaluated at the height of the block voted on.
16. **Vote eligibility**: only when voting is enabled, height is at least `StakeValidationHeight - 1`, the wallet records the ticket as its own, and it holds the voting private key. A wallet with voting enabled also votes tickets it registered with a VSP (it holds the same key); the resulting duplicate vote is identical apart from the signature and any policy differences.
17. **Per-ticket vote bits override defaults entirely** when any per-ticket preference is saved for the ticket (they are not merged with defaults).
18. **Treasury vote resolution order** (5.11) and the window test `InsideTSpendWindow(votedBlockHeight, expiry, TVI, multiplier)`.
19. **Orphan-vote and prune rules** interact with this subsystem: unmined tickets at a stale price and unmined votes not on the tip are removed, and the `vsp` record of a pruned ticket is deleted.
20. **Ticket buyer timing**: act only on tip changes when the wallet's transactions are synced through the tip; never buy at `height ≡ W-2`; cancel in-flight purchases at `height ≡ W-2` and again at `height ≡ 0` (5.13); minconf 1 (2 when mixing); balance to maintain applied to *spendable* balance of the purchase account.
21. **Mixed purchases**: split through StakeShuffle with mixed outputs to the ticket split account and commitments to the mixed account/branch, unmixed change to the change account, trickled ticket publication between 20 and 60 seconds after the split is recorded.
22. **JSON-RPC `purchaseticket` result shapes** depend on this subsystem returning ticket hashes in creation order and, with `dontsigntx`, the unsigned tickets and split transaction.
23. **Startup option semantics**: `--vsp.sync` is the only automatic resume; `--enablevoting` with `--spv` is rejected at config time; VSP options require url, pubkey and a non-zero max fee together.
24. **HTTP profile toward VSPs**: no environment proxy in either case. With `--proxy`: HTTP/1.1 only, every connection dialed through the configured (possibly SOCKS/Tor) dialer. Without `--proxy`: the default dialer, with HTTP/2 negotiated by ALPN on `https` URLs (4.6.1). A replacement that honours `HTTPS_PROXY` or bypasses the dialer would leak connections around Tor.
25. **Eligibility filter for funding** (5.6): only P2PKH-class outputs of the account, with the stated maturities; plain P2SH and unconfirmed-below-minconf outputs are never selected.
26. **Stake transaction classification** (3.10.1): the wallet's notion of ticket, vote, revocation and tspend is exactly `CheckSStx`, `CheckSSGen`, `CheckSSRtx`, `CheckTSpend` in the `DetermineTxType` order. It decides how transactions are stored in an existing `wallet.db` (transaction type, credit flags, ticket indexes) and which tickets `NewVSPTicket` accepts, so the port must classify identically to both dcrd and the Go wallet.
27. **Treasury-vote capacity**: a vote can carry at most 7 treasury votes and the output must be the last one with transaction version 3 (3.10.1 rule 9). The Go wallet does not cap the number (Q18); a port that caps at 7 changes which votes get published when more than 7 tspends are in their voting window (the Go vote is rejected by dcrd in that case).
28. **VSP policy map sizes**: `tspendpolicy` and `treasurypolicy` carry a server-side limit of 3 entries (4.6.3). The request shape and the absence of client-side truncation are what an existing VSP sees.
29. **Per-ticket policy maps in `payfee`**: only per-ticket entries are sent (global tspend and treasury-key policies are not), with `yes`/`no` values (abstain entries do not exist in memory); the `votechoices` map is the ticket's own preferences when it has any, otherwise the wallet defaults.
30. **Manual-ticket mode** (`--manualtickets`): tickets created by this wallet are always recorded; tickets seen from the network are not (3.2). Voting eligibility (item 16) follows from what is recorded.
31. **Fee transaction version** (4.3): 1 on the purchase and reconcile paths, 0 on the `SyncVSPFailedTickets` and `submitPayment`-fallback paths. The fee transaction hash is stored in the `vsp` record and reported by the VSP, so a port that normalises the version produces different hashes than the Go wallet would for the same inputs (harmless to the VSP, visible in the database and in `GetVSPTicketsByFeeStatus`-driven tooling).
32. **Log-line formats** listed in 8.5 where operators or tooling parse them (notably `Voting on block ...`, `Published ticket purchase ...`, `Purchased ticket ... at stake difficulty ...`, and the `VSPC` lines).

---

## 10. Quirks

Each item states what the Go does; a port must reproduce it or record a deliberate divergence.

- **Q1. Revocations are gone.** `RevokeOwnedTickets` is a no-op and `signVoteOrRevocation`'s revocation branch is dead code (`wallet/chainntfns.go:927-933`, `wallet/createtx.go:2053-2059`). The gRPC/JSON-RPC surfaces that once revoked tickets still exist elsewhere.
- **Q2. The split transaction is recorded and published twice.** `individualSplit` records and publishes it, the mixing client publishes the mixed one, and `purchaseTickets` then calls `processTransactionRecord` and `PublishTransactions` again (`wallet/createtx.go:1137-1146,1459-1482`). Observable as a duplicate unmined-transaction notification and a duplicate `sendrawtransaction`/inv.
- **Q3. Inconsistent `StakePoolTicketFee` argument.** The reservation path passes the absolute ticket fee (`wallet/createtx.go:1297`); the UTXO-split path passes the relay fee *rate* (`wallet/wallet.go:1649`). With defaults the second is larger (10000 vs 2980), so the extra output is slightly over-funded.
- **Q4. Wrong size constant.** `EstimateSerializeSize([]int{txsizes.RedeemP2PKHInputSize}, ...)` passes an input size (166) as a script size (108) (`wallet/wallet.go:1665`), over-estimating by 58 bytes.
- **Q5. Deployment assumptions.** Under SPV both DCP0010 and DCP0012 are assumed active for the VSP fee estimate (`wallet/createtx.go:1279-1296`). `DCP0010Active` short-circuits to true on simnet but `DCP0012Active` queries the backend even on simnet (`deployments/deployments.go:98-100,135-140`).
- **Q6. Leaked UTXO locks.** After the first ticket of a VSP purchase is published `unlockCredits` is false, so if a later ticket fails to publish or the context is cancelled during trickle, the fee credits reserved for the remaining tickets stay in the locked set until restart or an explicit unlock (`wallet/createtx.go:1304-1314,1633`). Separately, when `Process` swallows a `feeaddress` or fee-transaction error and returns nil (Q38), the purchase loop does not unlock that ticket's fee credits; they stay locked for the job's reconcile retry, and for good if that retry leaves the job idle (Q14).
- **Q7. Failed ticket listed in the response.** The ticket and hash are appended to the response before signing and recording; if that Update fails the response returned with the error still contains it (`wallet/createtx.go:1548-1553,1594-1596`).
- **Q8. VSP failures are invisible to the purchaser.** `NewVSPTicket` and `Process` errors are dropped without logging at the call site (`wallet/createtx.go:1649-1659`); the dead statement that follows (`udb.NewTxRecordFromMsgTx(feeTx, ...)`, result discarded) can only return `(nil, err)` on a malformed fee tx.
- **Q9. Treasury key policy keys are raw bytes in `payfee`.** `VSPTicket.TreasuryKeyPolicy` uses `key.TreasuryKey` (the 33 key bytes as a Go string) as the JSON map key (`wallet/vspticket.go:219-231`); Go's encoder turns invalid UTF-8 into U+FFFD. Every other path hex-encodes (`internal/rpc/rpcserver/server.go:4206`, `internal/rpc/jsonrpc/methods.go:3925-3927`). A ticket with a per-ticket treasury-key policy therefore sends garbage in `payfee`.
- **Q10. VSP client identity.** `Wallet.VSP` keys clients by the caller's raw URL string and returns an existing client without comparing pubkey or policy (`wallet/vspclientloader.go:12-26`), so the first caller's fee/change accounts and max fee apply to all later users of that URL in the process. The database stores the normalised URL (`url.Parse(...).String()`), and `LookupVSP(storedHost)` fails if normalisation changed the string (for example an upper-case scheme).
- **Q11. `payfee` is sent twice.** `Process` submits synchronously and the reconcile timer armed at job creation submits again; progress to confirmation happens only in that reconcile step (in the expected case through its `ErrFeeAlreadyReceived` branch), and that branch is the only place the fee transaction is marked published (`wallet/vsp.go:235-267`, `wallet/feepayment.go:434-450`). If the VSP answered the second `payfee` with success instead, the flow still continues but the fee tx stays marked unpublished.
- **Q12. `fee = -1` sentinel and stale fee data.** A job rebuilt from an existing fee tx sets `fee = -1` (`wallet/feepayment.go:166-167`); `makeFeeTx` only re-requests a fee address when `fee == 0` (`wallet/feepayment.go:323-333`). After `ErrFeeExpired` the fee tx is cleared but `fee`/`feeAddr` are kept, so the next attempt pays the old address and amount without a new `feeaddress` call; if `fee` is still -1 and `feeAddr` nil, `CreateVspPayment` is reached with a nil address (a nil-interface method call, i.e. a crash, on the timer goroutine). UNVERIFIED by execution; follows from the code paths cited.
- **Q13. Confirmed status overwrites the stored fee hash with the job's.** `confirmPayment` writes `UpdateFeeConfirmed(fp.feeHash, ...)` (`wallet/feepayment.go:588-591`); when the job never learned a fee hash (fee tx not found in the wallet) this zeroes the stored hash.
- **Q14. Jobs can idle forever.** `reconcilePayment` returns without rescheduling when `makeFeeTx` fails or a status write fails; `confirmPayment` schedules nothing on an unknown `feetxstatus` (`wallet/feepayment.go:412-420,442-455,470-473,604-610`). The idle job stays in `jobs`, and `ProcessUnprocessedTickets`/`ProcessManagedTickets` skip tickets that have a job, so only `Process` called directly (which reuses the job) or a restart recovers.
- **Q15. No automatic resume after restart** (5.8.6).
- **Q16. Vote log lines can name the wrong ticket.** After nil votes are swap-removed, the log pairs `voteRecords[i]` with `ticketHashes[i]` and `usedVoteBits[i]`, which were not permuted (`wallet/chainntfns.go:862-888`). Winner order itself comes from a Go map (`rpc/client/dcrd/notifications.go:31-41`) and is random.
- **Q17. Voting error handling.** A failed View is logged and voting continues with partial results; `PublishTransactions` is called even with zero votes; publish failures are ignored and the votes are still recorded (`wallet/chainntfns.go:858-895`).
- **Q18. Treasury vote ordering and count.** tspends are iterated in Go map order and no cap is applied in the wallet (consensus allows at most 7 per vote) (`wallet/chainntfns.go:804-824`, `wallet/wallet.go:274-289`). The tspend cache is pruned only inside `GetAllTSpends`.
- **Q19. Mixing with `Limit == 0` buys nothing.** `multiple = cfg.Limit` when mixing, and the purchase loop runs `multiple` times (`ticketbuyer/tb.go:167-171,198-200`); the later special case `limit == 0 && mixing` in `buy` is unreachable because the snapshot's limit was already set to 1 (`ticketbuyer/tb.go:282-286`). The CLI default limit is 1, but gRPC `RunTicketBuyer` passes the request's `limit` straight through. The option's help text says `0 for no limit` (`config.go:188`); that holds only without mixing.
- **Q20. Unsynchronised config read.** `buy` reads `tb.cfg.VSP` from the live config, not the snapshot and not under the mutex (`ticketbuyer/tb.go:302`).
- **Q21. Window-edge skip also skips change mixing**, and purchase attempts from consecutive blocks overlap: each tip change starts new attempts without waiting for earlier ones, bounded only by the boundary cancellation (`ticketbuyer/tb.go:122-140,173-211`).
- **Q22. VSP error decoding is permissive and one path can panic.** Any non-200 body that is a JSON object containing only `code`/`message` keys (including `{}`) or the literal `null` decodes as an `ErrorResponse` (code 0, possibly empty message) (`vspd/client/v4@v4.0.2/client.go:227-235`). `ed25519.Verify` panics when the configured public key is not 32 bytes; the key length is never validated (`wallet/vsp.go:71-79`, `vspd/client/v4@v4.0.2/client.go:267`).
- **Q23. No HTTP timeouts; secrets in trace logs.** See 4.6.1. Job-step requests can hang indefinitely, including those made synchronously by `Process` (and therefore by `purchaseTickets` after publishing a ticket), because they use `context.Background()` rather than the caller's context.
- **Q24. VSP record helpers.** A host's stored pubkey is never updated once written; `GetVSPHost`/`GetVSPPubKey` format their `NotExist` messages with `%v` of a pointer (`&id`, `&host`), printing a memory address or `&[..]`; `deserializeVSPTicket` indexes 40 bytes with no length check (`wallet/udb/vsp.go:49-82,149-161,203,254`).
- **Q25. Prune hook.** Iterates `w.vspClients` without its mutex and runs for every pruned transaction, not only tickets (`wallet/chainntfns.go:261-274`).
- **Q26. `SetVoteChoice` hides failures.** A failed status query (other than locked) is logged and reported as success (`wallet/vsp.go:297-304`). The JSON-RPC helper's comment says the first error is returned; `ForUnspentUnexpiredTickets` actually joins all (`internal/rpc/jsonrpc/methods.go:4855-4863`, `wallet/wallet.go:5746`).
- **Q27. Voting addresses come from the internal branch**, and address rows up to the same child index are then recorded on the **external** branch of the voting account, without changing any last-used or last-returned index (`wallet/createtx.go:1518-1525`, `wallet/addresses.go:654-669`, `wallet/udb/addressmanager.go:2058-2065`).
- **Q28. Balance estimates ignore fees.** Ticket credits are reserved for `ticketPrice` rather than `neededPerTicket`, and the low-balance reallocation compares against `ticketPrice * Count` (source marks both `XXX`) (`wallet/createtx.go:1338-1339,1393-1414`). The split loop's decrement-and-retry corrects for it.
- **Q29. `ForUnspentUnexpiredTickets` never sees unmined tickets** and its status filter lists `live` although that status cannot occur without an RPC client (3.6). `UnprocessedTickets` treats a ticket as unprocessed when its record has a zero fee hash and is not confirmed, which includes records in status errored.
- **Q30. Nested read transaction.** `GetAllTSpends` opens a View (for the tip height) while `VoteOnOwnedTickets` is already inside a View (`wallet/chainntfns.go:790`, `wallet/wallet.go:274-275`).
- **Q31. Map-ordered results.** `GetVSPTicketsByFeeStatus`, `TrackedTickets`, `AllVSPs`, `TreasuryKeyPolicies` return Go-map order (random per call).
- **Q32. `VSPTicket.Spent` treats every lookup error as "unspent"** (`wallet/vspticket.go:236-240`); it is true only when ticket output 0 is a recorded wallet credit with a recorded spender.
- **Q33. `CreateVspPayment` details.** A change address is consumed even when no change output is added; the unpublished marker is written before the transaction exists in the store; when several inputs fail to sign only the last error string is returned; `makeFeeTx` copies a transaction by value (`*tx = *fpFeeTx`), sharing input/output slices (`wallet/wallet.go:4702-4756`, `wallet/feepayment.go:307-313`).
- **Q34. Doc/code disagreement on trickle.** The comment says "a trickle delay of 20-60 seconds per ticket" (`wallet/createtx.go:1160-1161`); the code samples every ticket's time independently from the same instant, so all tickets are out within 60 seconds in total.
- **Q35. `vspinfo` is trusted minimally.** `vspclosed`, `network` and `pubkey` are ignored; a closed VSP is discovered only by `feeaddress` failing after the ticket was bought.
- **Q36. `DontSignTx` corner cases.** `mixedSplit` ignores it (it mixes, signs and publishes the split) while tickets are returned unsigned; with a VSP client, fee credits are reserved and then released, and `errVSPFeeRequiresUTXOSplit` is returned to the caller verbatim (`wallet/createtx.go:999,1599-1601`, `wallet/wallet.go:1600-1602`).
- **Q37. Status regress to started.** Creating a job for a ticket that already has a fee hash and fee tx, through the started/errored path, rewrites the record to status 0 while keeping the hash (`wallet/feepayment.go:160-168`).
- **Q38. `Process` swallows `feeaddress` and fee-transaction errors.** In both error branches a new `err` is declared for the `UpdateFeeErrored` result and that shadow is what the trailing `return err` returns, so `Process` returns nil after writing status errored (`wallet/vsp.go:248-266`). The original error is neither returned nor logged. Observable consequences: JSON-RPC `processunmanagedticket` and gRPC `SyncVSPFailedTickets`/`ProcessUnmanagedTickets` report success for such tickets; `ProcessUnprocessedTickets` logs nothing; `purchaseTickets` keeps the ticket's fee credits locked (Q6); the `vsp` record is left at status 2 with a zero fee hash; and the only retry is the job's single reconcile timer.
- **Q39. `UnprocessedTickets`/`ProcessedTickets` are all-or-nothing.** The first `NewVSPTicket` failure aborts the whole listing (`wallet/wallet.go:5782-5788,5819-5825`); see 5.7. `--vsp.sync` logs the failure and then calls `ProcessManagedTickets` with a nil slice (`dcrwallet.go:424-431`).
- **Q40. Nil policy is dereferenced.** `VSPClientConfig.Policy` may be nil (`NewVSPClient` does not check), and `receiveFeeAddress` and `makeFeeTx` read `fp.policy.MaxFee`, `.FeeAcct`, `.ChangeAcct` unconditionally (`wallet/feepayment.go:271,335`). Every in-tree caller supplies a policy; a client created with a nil policy panics on its first `feeaddress` response.
- **Q41. A 200 response with body `null` panics.** `json.Unmarshal` leaves the response pointer nil; `FeePercentage` then reads `resp.FeePercentage` and the POST methods read `resp.Request` (`wallet/vsp.go:113`, `vspd/client/v4@v4.0.2/client.go:59,93,115,149`). The body must still carry a valid server signature, so only the configured VSP can trigger it.
- **Q42. Unvalidated tspends from dcrd can crash policy resolution.** The RPC syncer caches whatever dcrd reports as a tspend; `TSpendPolicy` slices `SignatureScript[66:99]` unchecked (`wallet/wallet.go:611-615`, `chain/sync.go:697-704,925-931`). Reachable only with a faulty or hostile trusted dcrd.
- **Q43. Fee transactions can underpay the relay fee.** `CreateVspPayment` requires only `input >= fee`; when `fee <= input < fee + estimated mining fee` the computed change is negative, is treated as dust, and the transaction is built with a mining fee below the relay rate, possibly zero (`wallet/wallet.go:4693-4731`). How the VSP reacts is UNVERIFIED (its server source is not among the pinned modules); if it answers `ErrCannotBroadcastFee` or `ErrInvalidFeeTx`, `reconcilePayment` discards the fee transaction and retries, and for any other error it resubmits the same transaction on every reconcile. Reachable when the caller supplied the inputs (the purchase path reserves inputs for the **estimated** VSP fee, 5.3 step 7.3, while the amount charged is the VSP's `feeamount`).
- **Q44. Doc/code: `confirmPayment`.** Its comment says a fee transaction removed from the wallet schedules another reconcile (`wallet/feepayment.go:556-559`); the code never looks at the wallet's copy of the fee transaction, only at the VSP's `feetxstatus`.
- **Q45. Doc/code: `task`.** The comment says a failing method puts the payment "in an errored state" (`wallet/feepayment.go:213-215`); the only effect is setting the unexported `fp.err` and logging. No database status changes and `fp.err` is never read.
- **Q46. Doc/code: `GetAllTSpends`.** The comment says the tspend list "does not get culled" (`wallet/wallet.go:272-273`); the function deletes expired entries.
- **Q47. Doc/code: `ProcessManagedTickets` and seed restores.** The method's comment says it recovers VSP tracking after a seed restore (`wallet/vsp.go:144-147`), and it does query the VSP for any ticket it is given. But both in-tree callers feed it `ProcessedTickets()`, which lists only tickets that already have a local `vsp` record (`wallet/wallet.go:5795-5814`); a freshly restored wallet has none, so neither `--vsp.sync` nor gRPC `ProcessManagedTickets` recovers anything after a restore. gRPC `ProcessUnmanagedTickets` is the path that reaches such tickets, and it goes through `Process` (a new `feeaddress` request and a new fee transaction) rather than through `ticketstatus`; what the VSP answers to `feeaddress`/`payfee` for a ticket whose fee it already holds is UNVERIFIED.
- **Q48. Doc/code: `Wallet.VSP`.** Documented as a "package-global instance" (`wallet/vspclientloader.go:10-11`); the map is per `Wallet`.
- **Q49. Doc/code: `Process`.** The comment says newly selected fee inputs are "locked to prevent double spending the fee" (`wallet/vsp.go:213-215`); `CreateVspPayment` unlocks the inputs it reserved itself before returning and relies on the stored unpublished transaction instead (5.9 step 1). Inputs supplied by the purchase path stay locked.
- **Q50. `AccessConfig` is dead API**, and `Limit` is signed: a negative limit behaves as "no limit" without mixing and as "buy nothing" with mixing (`ticketbuyer/tb.go:167-171,282-286,318-322`).
- **Q51. Votes are not self-checked.** No `CheckSSGen` (or script execution, unlike tickets, which get `validateMsgTx`) is run on a constructed vote before publishing, and a vote rejected by the backend is still recorded (`wallet/chainntfns.go:793-913`).
- **Q52. Policy maps can exceed the VSP's limit of 3.** No truncation or batching is applied to `tspendpolicy`/`treasurypolicy` before `payfee` or `setvotechoices` (4.6.3); gRPC `SetVspdVoteChoices` in particular sends every cached tspend.
- **Q53. `VSPTicketInfo` omissions.** `TrackedTickets` reports `Fee` as the job's raw field, which is 0 before a fee address was received and -1 for a job rebuilt from an existing fee transaction (Q12), and `FeeHash` as all zero until a fee transaction exists; a source `TODO` notes that VSP status data is not included (`wallet/vsp.go:363-372`).
- **Q54. Source `XXX`/`TODO` markers not otherwise listed**: a fee hash recorded without its transaction "may require manual intervention" (`wallet/feepayment.go:145-150`); a ticket no longer in the wallet should remove the job (`:397-398`); unlocking fee inputs after payment is commented out (`:477-482`); an unknown `feetxstatus` should enter an "unknown state" (`:605`); server timestamps are not validated (`:276,366,387,550`); script version 0 is assumed for coinjoin inputs and eligible outputs (`wallet/createtx.go:1078,1815,2139,2150`); eligibility filtering should move into the transaction store and multisig outputs are not handled (`wallet/createtx.go:1744-1748,1806-1807`); a skipped unsigned input "not error?" (`wallet/createtx.go:2001`); voting can lag the transaction store when blocks arrive quickly (`wallet/chainntfns.go:724-728`). None of these is implemented behaviour; they mark where the Go is knowingly incomplete.

---

## 11. Upstream tests

| Test | File | Pins | Port |
| --- | --- | --- | --- |
| `TestStakePoolTicketFee` | `wallet/txrules/poolfees_test.go:11-93` | 7 mainnet vectors across the three subsidy variants | port verbatim (pure function) |
| `TestEstimateSerializeSize` | `wallet/txsizes/size_test.go:31` | size estimator | port verbatim |
| `TestVSPSerializeVSPHost`, `TestVSPSerializeVSPPubKey`, `TestVSPSerializeVSPTicket` | `wallet/udb/vsp_test.go:18-176` | serialize/deserialize round trips of the three records (no fixed byte vectors) | port, and add byte vectors from the oracle (item 6 below) |
| `TestSetGetDeleteVSPTicket` | `wallet/udb/vsp_test.go:177` | host ID allocation, round trip, delete | port against the Rust database layer |
| `TestCoinbaseMatured`, `TestTicketMatured`, `TestTicketExpired` | `wallet/wallet_test.go:21-102` | maturity predicates | port verbatim |
| `TestErrorDetails` | `vspd/client/v4@v4.0.2/client_test.go:22-120` | exact error strings for non-200 responses, `errors.As` to `ErrorResponse` | port verbatim |
| `TestSignatureValidation` | `vspd/client/v4@v4.0.2/client_test.go:122-192` | the three signature failure strings and success | port verbatim |
| `TestErrorDefaultMessages`, `TestErrorHTTPStatus`, `TestAPIErrorAs` | `vspd/types/v3@v3.0.0/errors_test.go`, `types_test.go` | code tables | port verbatim |
| `TestSStx`, `TestSSTxErrors`, `TestSSGen`, `TestSSGenErrors`, `TestSSGenTreasuryVotes`, `TestCheckSSRtx`, `TestGetSSGenBlockVotedOn`, `TestGetSStxStakeOutputInfo`, `TestGetSSGenVoteBits`, `TestGetSSGenVersion`, `TestGetSStxNullOutputAmounts`, `TestCalculateRewards`, `TestCalculateRevocationRewards`, `TestIsNullDataScript`, `TestCreateRevocationFromTicket` | `dcrd/blockchain/stake/v5@v5.0.3/staketx_test.go` | stake transaction classification and reward maths | already covered by the dcroxide stake crate's parity tests; reuse |
| `TestScriptTypes`, treasury tests | `scripttype_test.go`, `treasury_test.go` | tagged-script and tspend structure | reuse from dcroxide |
| `TestTreasuryIsFunctions`, `TestTreasurySpendErrors`, `TestTreasuryAddErrors`, `TestTreasuryBaseErrors` | `dcrd/blockchain/stake/v5@v5.0.3/treasury_test.go:182-600` | the four functions named in the row above: accept/reject vectors for `CheckTSpend`, `CheckTAdd`, `CheckTreasuryBase` with the expected `ErrorKind` | reuse from dcroxide; `TestTreasurySpendErrors` is the one that pins the rules the wallet relies on |
| `TestErrorKindStringer`, `TestRuleError`, `TestRuleErrorKindIsAs` | `dcrd/blockchain/stake/v5@v5.0.3/error_test.go` | `ErrorKind` names and `RuleError` formatting / `errors.Is` behaviour | reuse from dcroxide; the wallet depends only on nil/non-nil |
| `TestBasicPRNG`, `TestLotteryNumSelection`, `TestLotteryNumErrors`, `TestFetchWinnersErrors`, `TestTicketSorting` | `dcrd/blockchain/stake/v5@v5.0.3/lottery_test.go` | `Hash256PRNG` and winner selection | not needed by the wallet (3.10.1) |
| `TestTicketDBLongChain`, `TestTicketDBGeneral` | `dcrd/blockchain/stake/v5@v5.0.3/tickets_test.go`, data in `testdata/blocks0to168.bz2`, `testdata/testexpiry.bz2` | the node-side ticket database | not needed by the wallet |
| `TestUpgrades` | `wallet/udb/upgrades_test.go:42`, fixtures `wallet/udb/testdata/v1.db.gz` to `v11.db.gz` (an empty `v24.db.gz` fixture also exists and is not used by a verifier) | database upgrades, with verifiers only up to version 12 | owned by the migrations spec; noted here because the version 20, 21 and 23 upgrades that create and reshape the `vsp`, `vsphost` and `vsppubkey` buckets have **no** upstream verifier |
| `TestLocking` | `wallet/locking_test.go:15` | `Wallet.Unlock` semantics (a bad passphrase locks the wallet; a later timed unlock does not put a time limit on a wallet already unlocked without one; a timed unlock locks on expiry; a new timeout replaces an old one) | owned by the wallet-core spec; pins the behaviour the ticket buyer relies on (5.13) |
| JSON-RPC command marshalling cases for `settspendpolicy`, `settreasurypolicy` | `rpc/jsonrpc/types/methods_test.go:988-1040` | command shapes with and without the optional ticket argument | owned by the JSON-RPC specs |

Test files in scope with no test of this subsystem: `wallet/wallet_test.go` also holds `TestVotingXprivFromSeed` and `TestSetBirthStateAndScan` (other specs). There is no `_test.go` file in `ticketbuyer/`, in `deployments/`, or for `wallet/udb/treasury.go`, `wallet/vsp.go`, `wallet/vspticket.go`, `wallet/feepayment.go`, `wallet/vspclientloader.go`, `wallet/createtx.go` or `wallet/chainntfns.go`. `ValidPoolFeeRate` has no test.

**Untested upstream** (no unit test exists in dcrwallet for any of these): `purchaseTickets` and both split paths, `makeTicket`, `createUnsignedVote`, `VoteOnOwnedTickets`, `CreateVspPayment`, the entire fee payment job (`feepayment.go`), `VSPClient` methods, `NewVSPTicket`, the policy buckets, the `ticketbuyer` package, the `deployments` activation checks, and the endpoint methods of the vspd client (only `do` is tested).

**Vectors to generate with a Go oracle shim** (deterministic inputs, compare bytes):

1. `makeTicket` for fixed split outpoint, voting/commitment hash160s, price and expiry.
2. `createUnsignedVote` for a fixed ticket, block hash/height, vote bits, for each subsidy variant; plus the treasury output for 1, 2, 3 and 7 tspends.
3. `SignMessage` signatures for fixed key and body (deterministic RFC 6979 nonces) and the resulting `VSP-Client-Signature` header.
4. JSON bodies of each request type for fixed inputs, including maps with several keys and strings containing `<`, `>`, `&`.
5. `StakePoolTicketFee` on testnet3 and simnet parameters and at heights around subsidy reductions.
6. `vsp`/`vsphost`/`vsppubkey` bucket contents after a scripted sequence of `SetVSPTicket` calls with two hosts.
7. Ticket-buyer window arithmetic: `(height) -> (skip?, expiry, cancel?)` for a full window on mainnet and simnet parameters.
8. The four policy buckets after a scripted sequence of `Set*Policy` calls (yes, no, then abstain to delete), and the maps the loaders return, including a hand-written `0x00` value to pin the "present but omitted" case (3.7).
9. `IsSStx`/`IsSSGen`/`IsSSRtx`/`IsTSpend`/`DetermineTxType` on the wallet-built ticket and vote vectors from items 1 and 2, plus a vote with 8 treasury votes (must classify as *not* a vote) and a 2-output vote (must classify as not a vote).
10. `TxSStxStakeOutputInfo`, `AddrFromSStxPkScrCommitment` and `AmountFromSStxPkScrCommitment` on the item-1 ticket and on a ticket with a P2SH commitment (bit 63 set).
11. `CreateVspPayment` output layout for fixed inputs in three regimes: change above dust, change below dust, and `fee <= input < fee + mining fee` (Q43), with the change-position randomness injected.
12. `ValidPoolFeeRate` at the boundaries `0.0099`, `0.01`, `100.0`, `100.01`, and `StakePoolTicketFee` with fee percentages whose `*100` is not exactly representable (for example `0.29`, `1.15`, `2.59`), to pin the float-to-integer truncation.
13. `DCP0010Active`/`DCP0012Active` truth tables over network (mainnet, testnet3, simnet, other), status (`defined`, `started`, `lockedin`, `active`, `failed`, agenda absent) and `height` relative to `since + RuleChangeActivationInterval - 1`.
14. Fee-timer jitter selection: `(tip, minedHeight) -> jitter upper bound` for unmined (`-1`), heights 0 and 1 (treated as unmined), immature near and far from live, live, and past expiry.

A mock VSP (HTTP server signing with a fixed Ed25519 key) is needed for state-machine tests; none exists upstream in this module set.

---

## 12. Rust porting notes

**Suggested module boundary**

- `purse-stake-tx` (pure, no I/O): ticket builder, vote builder, treasury-vote script, VSP fee estimate (`stake_pool_ticket_fee`), size constants. Depends on wire, txscript, stake, standalone, chaincfg crates.
- `purse-vsp-client` (async HTTP): the five endpoints, request signing callback, response authentication, error decoding, typed error codes. No wallet dependencies beyond an address type and a signer trait.
- `purse-wallet::vsp` (inside the wallet crate): `VspClient`, `VspTicket`, fee payment jobs, `create_vsp_payment`, VSP queries; `purse-wallet::udb::vsp` and `::treasury` for the buckets.
- `purse-wallet::tickets`: `purchase_tickets`, split paths, eligible-output selection.
- `purse-wallet::voting`: `vote_on_owned_tickets`, policies, tspend cache.
- `purse-ticketbuyer`: the `TB` loop.

**Reusable code (inspected)**

- `dcroxide:crates/dcroxide-stake/src/lib.rs` — `check_sstx`/`is_sstx` (689, 808), `is_ssgen` (1117), `is_ssrtx` (1209), `determine_tx_type` (1214), `tx_sstx_stake_output_info` (386), `addr_from_sstx_pk_scr_commitment` (392), `amount_from_sstx_pk_scr_commitment` (417), `ssgen_block_voted_on` (433), `sstx_null_output_amounts` (458), `calculate_rewards` (562), `VoteBits` (196), treasury vote constants (813-820). `dcroxide:crates/dcroxide-stake/src/treasury.rs` — `check_tspend` (148), `is_tspend` (290), `tspend_signature_and_pub_key` (299). This covers every stake helper the wallet uses; the ticket database modules of that crate are not needed.
- `dcroxide:crates/dcroxide-txscript/src/stdaddr.rs` — `voting_rights_script` (555), `reward_commitment_script` (562), `stake_change_script` (591), `pay_vote_commitment_script` (597). `dcroxide:crates/dcroxide-txscript/src/script.rs:420` `generate_ssgen_block_ref`; `stdscript.rs:896` `provably_pruneable_script_v0`, `stdscript.rs:1157` `extract_addrs`; `sign.rs:184` `signature_script`, `sign.rs:606` `sign_tx_output`; the script engine for `sanityVerifyFlags` checks.
- `dcroxide:crates/dcroxide-standalone/src/subsidy.rs:390` `calc_stake_vote_subsidy_v3`; `treasury.rs:74` `inside_tspend_window`.
- `dcroxide:crates/dcroxide-wire` for `MsgTx`; `dcroxide-chaincfg` for parameters and deployments; `dcroxide-rpctypes` for the dcrd agenda-info type.
- `dcr-rs:src/sign.rs:38` (`sign_p2pkh_input`), `dcr-rs:src/address.rs`, `dcr-rs:src/tx.rs` duplicate smaller parts of the above; prefer the dcroxide crates so the wallet and node share one transaction type.

**Hazards**

1. Go's `time.AfterFunc` timers plus `context.Background()` give jobs a lifetime independent of any request. In Rust, give each job an owned task with an explicit cancellation handle and decide deliberately whether shutdown cancels it (Go does not).
2. A timer step and a synchronous `Process` can run concurrently on the same job in Go. Either reproduce with fine-grained locks or serialise per job (a behavioural tightening that removes the double-`payfee` interleavings; note Q11 if serialised, since the sequence of HTTP calls seen by the VSP is the compatibility surface).
3. JSON: use ordered maps (`BTreeMap`) and struct field order as declared. Byte equality with Go is not required for acceptance (the VSP verifies the signature over whatever bytes arrive and echoes them), but the echo comparison must be over the exact bytes sent.
4. Ed25519: verify with standard (non-batch) verification over the raw body; validate key length at configuration time instead of panicking (divergence from Q22, recommended).
5. `float64` in `StakePoolTicketFee` (`floor(pct*100)`): use the same IEEE operations; e.g. `2.59*100` must floor to 258 or 259 exactly as Go does.
6. The locked-outpoint set is process memory only; all "reserve" logic assumes single-process ownership of the wallet.
7. Random choices (UTXO selection, trickle, jitter, disapproval) use a CSPRNG; tests must inject randomness.
8. HTTP client must use the wallet's dialer when one is configured (HTTP/1.1 only in that case; Go negotiates HTTP/2 on `https` only when no dialer is set, 4.6.1), with no ambient proxy and no implicit timeouts shorter than Go's (none). Adding sane timeouts is a recommended divergence.
9. The purchase function mutates its request (`Count`, `MinConf`); callers (ticket buyer logging, RPC) see the mutated values.
10. `sort.Slice` in the reallocation is unstable; equal-valued credits may be ordered differently. Not externally distinguishable.
11. Go slice-bounds and nil-dereference panics are reachable from external data in four places (Q22 key length, Q40, Q41, Q42) and from malformed database values in the record and policy decoders (3.7, 4.5). In Rust these must be explicit length checks returning errors; do not index unchecked.
12. JSON decoding of VSP responses must follow Go's permissive rules (4.6.3): unknown fields ignored on success responses but **rejected** when decoding an error body, `null` treated as absent, base64 with padding for byte fields. A strict serde configuration would reject responses the Go accepts.
13. The stake classification functions (3.10.1) must come from one shared implementation for the wallet and the node; do not re-derive them in the wallet crate.
14. `StakePoolTicketFee` and the wallet's own subsidy cache are separate caches in Go (3.12, 5.1); in Rust a single cache keyed by network is equivalent as long as one process serves one network.

**Size**: **L**. Roughly 3,000 lines of Go are directly in scope (ticket purchase ~900, VSP client/job/ticket ~1,400 plus ~270 in the vspd module, voting ~450, persistence ~550, ticket buyer ~340) with no upstream tests for the stateful parts, so most effort is in building a mock VSP, a Go oracle for transaction vectors, and reproducing the purchase path's UTXO-contention logic. The stake primitives come from dcroxide unchanged, which keeps this from XL.

---

## 13. Open questions

1. **Q9 (raw-byte treasury keys in `payfee`)**: reproduce the malformed request or hex-encode like the other paths? What vspd does with such a request was not checked (vspd server source is not among the pinned modules).
2. **Q12**: confirm by experiment that an expired fee after a restart can reach `CreateVspPayment` with a nil fee address, and decide whether the port re-requests the fee address after `ErrFeeExpired`.
3. **Q11**: what the VSP returns for a second identical `payfee` is assumed to be `ErrFeeAlreadyReceived` from the client's handling; UNVERIFIED against vspd.
4. **Q19**: keep "mixing with limit 0 buys nothing" for gRPC `RunTicketBuyer`, or treat 0 as 1? Depends on what Decrediton sends; not checked here.
5. **Q15**: should the replacement resume fee payment jobs automatically at startup (behavioural improvement) or stay strictly opt-in via `--vsp.sync`?
6. **Q10**: keep the first-caller-wins VSP client registry, or key by (URL, policy)? Changing it alters which accounts pay fees when several gRPC calls name the same VSP with different accounts.
7. **Empty `votechoices` in `setvotechoices`** from `settreasurypolicy`/`settspendpolicy` (5.10): vspd's treatment of an empty map (ignore vs clear) is UNVERIFIED; it decides whether those RPCs can reset agenda choices at the VSP.
8. **Policy strings `""`/`invalid`** forwarded to the VSP (5.10): whether vspd accepts them is UNVERIFIED.
9. **Nested read transactions (Q30)** and reader/writer interleaving under `stakeSettingsLock` (7.1) depend on the database layer chosen for the port; settle together with the walletdb spec.
10. **Shutdown**: Go leaves jobs running until process exit and does not join purchase goroutines. Decide the port's shutdown ordering (cancel jobs, wait for in-flight `payfee`?) — any choice is invisible to the VSP protocol but affects whether a fee tx can be created and not submitted.
11. The exact RPC error codes that wallet error kinds map to for `purchaseticket` and friends are defined by the RPC specs; this spec only fixes the kinds and messages.
12. **`binding` tags (4.6.3)**: whether vspd enforces `max=3` on the policy maps and `required` on a zero `timestamp`, and with which error code, is UNVERIFIED (server source not pinned). It decides whether Q52 is a live failure mode and whether the port should cap or batch.
13. **Q43**: the VSP's response to a fee transaction paying less than the relay fee is UNVERIFIED; decide whether the port requires `input >= fee + mining fee` (a fix that changes which fee transactions get created).
14. **Q47**: decide whether `--vsp.sync` / `ProcessManagedTickets` should also query the VSP for unspent tickets without a local `vsp` record, as the method's documentation intends, or keep the Go behaviour.
15. **Q39**: keep the all-or-nothing listing, or skip tickets that cannot become a `VSPTicket`? Skipping changes which RPC calls succeed on wallets holding legacy pool tickets or tickets with foreign voting keys.
16. **Q51 / compatibility item 27**: cap treasury votes at 7 and self-check votes with `CheckSSGen`, or publish exactly what the Go would.
17. **Q40-Q42 panics**: the port should return errors instead of aborting; confirm no caller depends on the crash (none found).
18. The reaction of `ed25519.Verify` to non-canonical signatures and small-order keys follows Go's `crypto/ed25519`; the Rust verifier should accept and reject the same set. The Go standard library's exact rules (handling of non-canonical encodings and of the cofactor) were not read for this spec and are UNVERIFIED; settle by differential testing against a Go oracle. Only the configured VSP can produce such signatures, so the exposure is limited to interoperability with a non-standard VSP signer.

