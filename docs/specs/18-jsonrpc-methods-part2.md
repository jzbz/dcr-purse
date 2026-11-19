# 18 — JSON-RPC methods, part 2 (`getvotechoices` … `getcoinjoinsbyacct`)

## 1. Header

**Scope** (dcrwallet, line counts from `wc -l`):

| File | Lines | Covered here |
|---|---|---|
| `internal/rpc/jsonrpc/methods.go` | 5863 | lines 2798-5863: every handler whose declaration starts in that range (55) and every helper function, type and variable defined there (3.2) |
| `rpc/jsonrpc/types/methods.go` | 1336 | the command structs of the methods in scope (cross-check only; owned by spec 16) |
| `rpc/jsonrpc/types/results.go` | 416 | the result structs of the methods in scope (cross-check only; owned by spec 16) |
| `internal/rpchelp/helpdescs_en_US.go` | 1044 | help strings of the methods in scope (cross-check only; owned by spec 16) |
| `internal/rpchelp/methods.go` | 142 | declared result types of the methods in scope (cross-check only) |
| `internal/rpc/jsonrpc/errors.go` | 111 | read in full; owned by spec 16 |
| `internal/rpc/jsonrpc/config.go` | 42 | read in full (`Options` fields the handlers consult) |

Supporting code read to pin observable behaviour (owned by other specs,
cited where it determines a result byte or an error string):
`methods.go:1-306` (handler table, dispatch), `methods.go:1829-1863`
(`decodeAddress`, `decodeStakeAddress`), `errors/errors.go` (all),
`wallet/wallet.go` (the methods the handlers call: lines 200-830,
1060-1090, 1540-2112, 2344-2791, 2836-3231, 3340-3684, 4248-4643,
4764-4909, 5100-5268, 5330-5336, 5554-5747), `wallet/addresses.go:28-330,
755-840`, `wallet/createtx.go:60-215, 544-595`, `wallet/multisig.go`,
`wallet/network.go:59-71`, `wallet/rescan.go:384-407, 514-526`,
`wallet/unstable.go`, `wallet/vsp.go:29-345`, `wallet/vspclientloader.go`,
`wallet/vspticket.go:38-90`, `wallet/mixing.go:253-275, 443-470`,
`wallet/txauthor/author.go:60-181`, `wallet/txrules/rules.go:37-145`,
`wallet/txsizes/size.go:20-117`, `wallet/udb/txquery.go:206-241, 355-538`,
`wallet/udb/txmined.go:889-931, 2723-2729, 2863-2871`,
`wallet/udb/addressdb.go:745-770`, `validate/validate.go:107-129`,
`internal/loader/loader.go:364-404`, `chain/backend.go:72-74`,
`rpc/client/dcrd/calls.go:509-518`, `dcrwallet.go:38, 176-189`.

**Parity pin**: dcrwallet `54b85b33` (2.2.0-pre, module
`decred.org/dcrwallet/v5`). Dependency versions read (from `go.mod`):

| Module | Version |
|---|---|
| `github.com/decred/dcrd/dcrjson/v4` | v4.2.0 |
| `github.com/decred/dcrd/rpc/jsonrpc/types/v4` | v4.4.0 |
| `github.com/decred/dcrd/dcrutil/v4` | v4.0.3 |
| `github.com/decred/dcrd/chaincfg/chainhash` | v1.0.5 |
| `github.com/decred/dcrd/chaincfg/v3` | v3.3.0 |
| `github.com/decred/dcrd/txscript/v4` | v4.1.2 |
| `github.com/decred/dcrd/wire` | v1.8.0 |
| `github.com/decred/dcrd/blockchain/stake/v5` | v5.0.3 |
| `github.com/decred/dcrd/blockchain/standalone/v2` | v2.3.0 |
| `github.com/decred/dcrd/crypto/rand` | v1.0.1 |
| `github.com/jrick/wsrpc/v2` | v2.3.8 |
| `go.etcd.io/bbolt` | v1.3.12 (one doc comment, cited in 7) |
| `github.com/decred/dcrd/dcrec` | v1.0.1 (signature-suite constants) |
| `github.com/decred/dcrd/dcrec/secp256k1/v4` | v4.4.1 (`ParsePubKey` error text) |
| `github.com/decred/base58` | v1.0.6 (`Decode` on invalid input) |

**Summary.** This spec is the per-method contract for the second half of
the JSON-RPC handler file: vote and treasury policy (`getvotechoices`,
`setvotechoice`, `treasurypolicy`, `settreasurypolicy`, `tspendpolicy`,
`settspendpolicy`, `setdisapprovepercent`), the listing methods
(`listaccounts` … `listunspent`, `ticketinfo`, `getcoinjoinsbyacct`),
outpoint locking, ticket purchase and VSP processing, every "send" method
including the treasury add and treasury spend builders, multisig
redemption, `spendoutputs`, `sweepaccount`, raw transaction signing and
publishing, message signing/verification, `validateaddress`, `version`,
`walletinfo`, wallet/account locking and passphrase methods, and the two
mixing triggers. For each method it gives parameters and defaults, the
validation order with the exact error code and message of every exit, the
wallet calls made, the exact JSON result (field order, `omitempty`, `null`
versus `[]`, number formatting), side effects, and dcrd-versus-SPV
differences. The handlers are thin; most complexity is in what they *fail
to* validate, in Go-runtime-dependent output ordering (map iteration), and
in a set of upstream bugs (section 10) that a port must either reproduce or
consciously fix. Help-text disagreements are collected in 10.3.

Transport, authentication, the request/response envelope, positional
parameter decoding, `convertError`, the help generator and the `types`
package as a whole are specified in spec 16 and are only referenced here.
Handlers at lines 1-2797 are in the sibling spec for part 1.

---

## 2. Role in the system

Each handler is a method `func (s *Server) name(ctx context.Context, icmd
any) (any, error)` registered in the package-level `handlers` map
(`methods.go:85-200`). The dispatcher (`lazyApplyHandler`,
`methods.go:229-279`; spec 16 §5.5) decodes the positional parameters into
the method's command struct with `dcrjson.ParseParams`, calls the handler
on the request goroutine, converts a returned error with `convertError`
(spec 16 §8.2), and marshals a returned value with `encoding/json` into
the `result` member of the response.

Callers: any JSON-RPC client of the wallet (for example dcrctl with
`--wallet`, and whatever third-party tooling speaks the legacy JSON-RPC
API) over HTTP POST or websocket.

Callees:

- `s.walletLoader.LoadedWallet()` / `s.walletLoader.NetworkBackend()`
  (`internal/loader/loader.go:364-404`): the single loaded
  `*wallet.Wallet`, and the wallet's current network backend
  (`*chain.Syncer` for dcrd RPC mode, `*spv.Syncer` for SPV mode, absent
  when not connected).
- `*wallet.Wallet` methods (specs 06-08, 10, 11, 13) for everything that
  touches the database, keys, transaction authoring, tickets, VSPs and
  mixing.
- `s.cfg` (`jsonrpc.Options`, `config.go:13-31`): static configuration
  captured at server start: `MixingEnabled`, `MixAccount`, `MixBranch`,
  `MixChangeAccount`, `TicketSplitAccount`, `VSPHost`, `VSPPubKey`.
  Populated from the application config at `rpcserver.go:368-380`:
  `MixingEnabled` from `--mixing`; `MixAccount`/`MixBranch` from the
  parsed `--mixedaccount` value (`account/branch`,
  `config.go:760-775`); `MixChangeAccount` from `--changeaccount`;
  `TicketSplitAccount` from `--ticketsplitaccount` (defaults to the
  mixed account, `config.go:780-782`); `VSPHost`/`VSPPubKey` from
  `--vsp.url`/`--vsp.pubkey`.
- `s.activeNet` (`*chaincfg.Params`): used by the two handlers that do not
  require a loaded wallet for network parameters (`verifymessage`,
  `setdisapprovepercent`).
- dcrd, through `chainSyncer.RPC().Call` (`help`, `version`, `walletinfo`
  ping) and `chainSyncer.GetTxOut` (`signrawtransaction`).
- The network backend's `PublishTransactions` directly (`sendfromtreasury`,
  `signrawtransactions`) or through `w.PublishTransaction`
  (`sendrawtransaction`, `spendoutputs`).

Nothing in this scope persists data itself; all database writes happen
inside wallet methods.

---

## 3. Public surface inventory

Nothing in the package is exported apart from `Server`, `Options` and
`Loggers` (spec 16). The observable surface is the method table. All
identifiers below are unexported.

### 3.1 Handlers (in source order)

"Wallet" = requires a loaded wallet (else `errUnloadedWallet`, -4).
"Net" = needs a network backend. Method names are the lower-case keys of
the `handlers` map (`methods.go:85-188`).

| Method | Handler (`methods.go` lines) | Command type | Wallet | Net | One-line contract |
|---|---|---|---|---|---|
| `getvotechoices` | `getVoteChoices` 2798-2845 | `GetVoteChoicesCmd` | yes | no | Agenda choices (default or per-ticket) for the wallet's vote version |
| `getwalletfee` | `getWalletFee` 2848-2855 | `GetWalletFeeCmd` | yes | no | Current relay fee per kB as a DCR number |
| `help` | `help` 2882-2937 | `HelpCmd` | no | optional dcrd | Usage list or per-method help; merges dcrd's help in RPC mode (spec 16 §5.6) |
| `listaccounts` | `listAccounts` 2941-2971 | `ListAccountsCmd` | yes | no | Object mapping account name to spendable balance |
| `listlockunspent` | `listLockUnspent` 2975-2987 | `ListLockUnspentCmd` | yes | no | Array of outpoints locked with `lockunspent` |
| `listreceivedbyaccount` | `listReceivedByAccount` 3002-3028 | `ListReceivedByAccountCmd` | yes | no | Per-account total received |
| `listreceivedbyaddress` | `listReceivedByAddress` 3044-3132 | `ListReceivedByAddressCmd` | yes | no | Per-address total received with txids |
| `listsinceblock` | `listSinceBlock` 3136-3189 | `ListSinceBlockCmd` | yes | no | Transactions at or after a block, plus a `lastblock` cursor |
| `listtransactions` | `listTransactions` 3193-3223 | `ListTransactionsCmd` | yes | no | Paged recent transaction entries |
| `listaddresstransactions` | `listAddressTransactions` 3230-3259 | `ListAddressTransactionsCmd` | yes | no | Transaction entries crediting given P2PKH/P2SH addresses |
| `listalltransactions` | `listAllTransactions` 3265-3278 | `ListAllTransactionsCmd` | yes | no | All transaction entries |
| `listunspent` | `listUnspent` 3281-3324 | `ListUnspentCmd` | yes | no | Unspent, unlocked wallet outputs |
| `lockunspent` | `lockUnspent` 3327-3351 | `LockUnspentCmd` | yes | no | Lock/unlock outpoints in memory |
| `purchaseticket` | `purchaseTicket` 3356-3518 | `PurchaseTicketCmd` | yes | yes | Buy tickets; hashes, or unsigned txs with `dontsigntx` |
| `processunmanagedticket` | `processUnmanagedTicket` 3522-3561 | `ProcessUnmanagedTicketCmd` | yes | implicit | Register an existing ticket with the configured VSP |
| `treasurypolicy` | `treasuryPolicy` 3792-3852 | `TreasuryPolicyCmd` | yes | no | Query treasury-key voting policies |
| `setdisapprovepercent` | `setDisapprovePercent` 3855-3870 | `SetDisapprovePercentCmd` | yes (checked last) | no | Set test-only block disapproval percentage; refused on mainnet |
| `settreasurypolicy` | `setTreasuryPolicy` 3877-3931 | `SetTreasuryPolicyCmd` | yes | VSP HTTP | Save a treasury-key policy; push to VSPs |
| `tspendpolicy` | `tspendPolicy` 3938-4001 | `TSpendPolicyCmd` | yes | no | Query tspend voting policies |
| `settspendpolicy` | `setTSpendPolicy` 4008-4064 | `SetTSpendPolicyCmd` | yes | VSP HTTP | Save a tspend policy; push to VSPs |
| `redeemmultisigout` | `redeemMultiSigOut` 4071-4162 | `RedeemMultiSigOutCmd` | yes | optional dcrd | Build and partially sign a tx spending one P2SH multisig output |
| `redeemmultisigouts` | `redeemMultiSigOuts` 4169-4219 | `RedeemMultiSigOutsCmd` | yes | optional dcrd | Same for every unspent multisig output of a P2SH address |
| `rescanwallet` | `rescanWallet` 4223-4242 | `RescanWalletCmd` | yes | yes | Blocking rescan from a height |
| `spendoutputs` | `spendOutputs` 4322-4400 | `SpendOutputsCmd` | yes | yes | Spend exactly the named outputs to the named outputs, publish |
| `ticketinfo` | `ticketInfo` 4402-4482 | `TicketInfoCmd` | yes | no | Details of every wallet ticket |
| `sendfrom` | `sendFrom` 4493-4532 | `SendFromCmd` | yes | yes (inside wallet) | Pay one address from an account |
| `sendmany` | `sendMany` 4539-4577 | `SendManyCmd` | yes | yes (inside wallet) | Pay many addresses from an account |
| `sendtoaddress` | `sendToAddress` 4584-4614 | `SendToAddressCmd` | yes | yes (inside wallet) | Pay one address from the default account, minconf 1 |
| `sendtomultisig` | `sendToMultiSig` 4627-4670 | `SendToMultiSigCmd` | yes | yes (inside wallet) | Fund a new m-of-n P2SH output from the default account |
| `sendrawtransaction` | `sendRawTransaction` 4674-4712 | `SendRawTransactionCmd` | yes | yes | Record (if relevant) and publish a serialized tx |
| `sendtotreasury` | `sendToTreasury` 4719-4738 | `SendToTreasuryCmd` | yes | yes (inside wallet) | Publish a treasury add from the default account |
| `sendfromtreasury` | `sendFromTreasury` 4742-4750 | `SendFromTreasuryCmd` | yes | yes | Build, sign and publish a treasury spend |
| `settxfee` | `setTxFee` 4753-4773 | `SetTxFeeCmd` | yes | no | Set the in-memory relay fee |
| `setvotechoice` | `setVoteChoice` 4780-4808 | `SetVoteChoiceCmd` | yes | VSP HTTP | Save an agenda choice; push to VSPs |
| `signmessage` | `signMessage` 4868-4890 | `SignMessageCmd` | yes | no | Compact-signature of a message, base64 |
| `signrawtransaction` | `signRawTransaction` 4896-5120 | `SignRawTransactionCmd` | yes | optional dcrd | Sign inputs of a serialized tx |
| `signrawtransactions` | `signRawTransactions` 5123-5203 | `SignRawTransactionsCmd` | yes | if `send` | Sign several txs; optionally publish the complete ones |
| `sweepaccount` | `sweepAccount` 5241-5303 | `SweepAccountCmd` | yes | no | Build an *unsigned* tx moving all of an account to an address |
| `validateaddress` | `validateAddress` 5306-5389 | `ValidateAddressCmd` | yes | no | Decode an address and report wallet knowledge of it |
| `validatepredcp0005cf` | `validatePreDCP0005CF` 5392-5400 | `ValidatePreDCP0005CFCmd` | yes | no | Check stored pre-DCP0005 cfilters against a hardcoded hash |
| `verifymessage` | `verifyMessage` 5404-5432 | `VerifyMessageCmd` | **no** | no | Verify a `signmessage` signature |
| `version` | `version` 5438-5463 | `VersionCmd` | **no** | optional dcrd | Semver objects keyed by component (spec 16 §4.8) |
| `walletinfo` | `walletInfo` 5468-5541 | `WalletInfoCmd` | yes | optional | Global wallet state |
| `walletislocked` | `walletIsLocked` 5546-5553 | `WalletIsLockedCmd` | yes | no | Boolean lock state |
| `walletlock` | `walletLock` 5558-5566 | `WalletLockCmd` | yes | no | Lock the wallet |
| `walletpassphrase` | `walletPassphrase` 5572-5586 | `WalletPassphraseCmd` | yes | no | Unlock, optionally with a timeout |
| `walletpassphrasechange` | `walletPassphraseChange` 5595-5611 | `WalletPassphraseChangeCmd` | yes | no | Change the private passphrase |
| `mixoutput` | `mixOutput` 5613-5647 | `MixOutputCmd` | yes | yes (inside wallet) | Mix one output; blocks until done |
| `mixaccount` | `mixAccount` 5649-5677 | `MixAccountCmd` | yes | yes (inside wallet) | Mix the change account's outputs; blocks until done |
| `walletpubpassphrasechange` | `walletPubPassphraseChange` 5700-5713 | `WalletPubPassphraseChangeCmd` | yes | no | Change the public passphrase |
| `setaccountpassphrase` | `setAccountPassphrase` 5715-5731 | `SetAccountPassphraseCmd` | yes | no | Set/change/remove a per-account passphrase |
| `accountunlocked` | `accountUnlocked` 5733-5765 | `AccountUnlockedCmd` | yes | no | Per-account encryption and lock state |
| `unlockaccount` | `unlockAccount` 5767-5783 | `UnlockAccountCmd` | yes | no | Unlock an individually encrypted account |
| `lockaccount` | `lockAccount` 5785-5817 | `LockAccountCmd` | yes | no | Lock one account, or all with `"*"` |
| `getcoinjoinsbyacct` | `getcoinjoinsbyacct` 5834-5863 | `GetCoinjoinsByAcctCmd` | yes | no | Count of probable coinjoin outputs per account |

Handler count: 55 method-table entries above plus the three internal
"send" workers in 3.2 that carry most of the send logic.

### 3.2 Helper functions and types

| Identifier | Lines | Signature | Contract |
|---|---|---|---|
| `helpDescs`, `helpDescsMu` | 2875-2876 | `map[string]string`, `sync.Mutex` | Lazily built en_US help map and its lock (spec 16 §5.6) |
| `makeOutputs` | 3567-3587 | `(pairs map[string]dcrutil.Amount, chainParams) ([]*wire.TxOut, error)` | One output per pair, in Go map iteration order; rejects negative amounts with `errNeedPositiveAmount`; address errors from `decodeAddress` |
| `(*Server).sendPairs` | 3592-3623 | `(ctx, w, amounts map[string]dcrutil.Amount, account uint32, minconf int32) (string, error)` | Shared body of `sendfrom`/`sendmany`/`sendtoaddress` (5.20) |
| `(*Server).sendAmountToTreasury` | 3628-3665 | `(ctx, w, amount, account, minconf) (string, error)` | Body of `sendtotreasury` (5.23) |
| `(*Server).sendOutputsFromTreasury` | 3670-3786 | `(ctx, w, cmd types.SendFromTreasuryCmd) (string, error)` | Body of `sendfromtreasury` (5.24) |
| `(*Server).updateVSPVoteChoices` | 4810-4864 | `(ctx, w, ticketHash *chainhash.Hash, choices, tspendPolicy, treasuryPolicy map[string]string) error` | Pushes changed preferences to the VSP(s) of one ticket or of all unspent unexpired tickets (5.19) |
| `spendOutputsInputSource` | 4247-4292 | `(ctx, w, account string, inputs []*wire.TxIn) (txauthor.InputSource, error)` | Fixed input source for `spendoutputs`: validates each previous output is wallet-owned P2PKH in `account` |
| `accountChangeSource` | 4294-4317 | struct `{ctx, wallet, account uint32, addr stdaddr.Address}`; `Script() ([]byte, uint16, error)`, `ScriptSize() int` | Lazily derives one change address with `w.NewChangeAddress`; size fixed at `txsizes.P2PKHOutputSize` (36) |
| `isNilOrEmpty` | 4484-4486 | `(s *string) bool` | `s == nil || *s == ""` |
| `scriptChangeSource` | 5207-5218 | struct `{version uint16, script []byte}`; `Script()`, `ScriptSize()` | Fixed-script change source; size is `len(script)` |
| `makeScriptChangeSource` | 5220-5231 | `(address string, params) (*scriptChangeSource, error)` | `stdaddr.DecodeAddress` then `PaymentScript()`; raw decode error |
| `sumOutputValues` | 5233-5238 | `(outputs []*wire.TxOut) dcrutil.Amount` | Sum of output values |
| `parseOutpoint` | 5679-5696 | `(s string) (*wire.OutPoint, error)` | Parses `"<64 hex>:<decimal u32>"`; tree always 0 (4.1.5) |
| `decodeHexStr` | 5823-5832 | `(hexStr string) ([]byte, error)` | Hex decode, left-padding odd-length input with `'0'`; error is `-22 hex string decode failed: <err>` |
| `AddrData` (function-local type in `listReceivedByAddress`) | 3052-3059 | struct `{amount dcrutil.Amount; confirmations int32; tx []string}` | Per-address accumulator keyed by address string in a Go map (5.7); never serialized itself |

Value-versus-pointer result types (not observable in JSON, but they
constrain the internal calls): `signRawTransaction` returns the **value**
`types.SignRawTransactionResult` (`methods.go:5115`), which
`redeemMultiSigOut` (`methods.go:4160`) and `signRawTransactions`
(`methods.go:5140`) type-assert as a value; `redeemMultiSigOut` returns
the value `types.RedeemMultiSigOutResult`, type-asserted by
`redeemMultiSigOuts` (`methods.go:4212`). `sendOutputsFromTreasury`
receives the command struct **by value** (`methods.go:4749`).

Helpers defined before line 2798 and used here: `decodeAddress`,
`decodeStakeAddress` (`methods.go:1829-1863`, 4.1.4), `walletPubKeys`
(`methods.go:393-426`, used by `sendtomultisig`), `confirms`
(`methods.go:75-82`), the constant `scriptVersionAssumed = 0`
(`methods.go:69`).

---

## 4. Data formats

### 4.1 Conventions shared by every method

#### 4.1.1 Amounts

Amounts are JSON numbers in DCR, never strings.

- **Output**: `dcrutil.Amount.ToCoin()` is `float64(atoms) / 1e8`
  (`dcrd/dcrutil/v4@v4.0.3/amount.go:93-100`), then Go `encoding/json`
  float formatting: the shortest decimal that round-trips, in plain
  notation when `1e-6 <= |x| < 1e21` and exponent notation otherwise, no
  trailing `.0` (`1` not `1.0`, one atom is `1e-8`, zero is `0`). Negative
  zero cannot occur from an integer atom count except through explicit
  negation of a zero amount (see `listtransactions` "send" entries, where
  `-amountF64` of `0.0` yields `-0`, which Go marshals as `-0`).
- **Input**: `dcrutil.NewAmount(f)` (`amount.go:76-89`): rejects NaN and
  ±Inf with the plain error `invalid coin amount`; otherwise
  `round(f * 1e8)` where `round` adds `0.5` (or subtracts it for negative
  `f`) and truncates toward zero by conversion to `int64`
  (`amount.go:61-66`). NaN/Inf cannot be expressed in JSON and the JSON
  decoder rejects numbers outside the float64 range, so the error exit is
  unreachable from the wire; every `NewAmount` error branch below is dead
  code. A value whose product exceeds the int64 range is converted with
  Go's implementation-defined float-to-int conversion (10.2 Q-R1).

#### 4.1.2 Hashes

- **Output**: `chainhash.Hash.String()`: 64 lower-case hex characters,
  byte-reversed.
- **Input**: `chainhash.NewHashFromStr`
  (`dcrd/chaincfg/chainhash@v1.0.5/hash.go:87-129`):
  - longer than 64 characters: error `max hash string length is 64 bytes`;
  - odd length: a `'0'` is prepended;
  - shorter than 64: accepted, the missing high-order characters are
    zero (so `""` decodes to the all-zero hash and `"1"` to hash
    `…0001`);
  - a non-hex character: Go's `encoding/hex` error, for example
    `encoding/hex: invalid byte: U+0067 'g'`.
  Only `settreasurypolicy` and `settspendpolicy` additionally require
  exactly 64 characters.

The error code attached to a bad hash differs per method (-8 or -22);
see each method.

#### 4.1.3 Hex and serialized transactions

- Output hex is lower-case (`hex.EncodeToString` / `hex.NewEncoder`).
- Serialized transactions are the full wire serialization
  (`wire.MsgTx.Serialize`, prefix and witness).
- Transactions are **parsed** with
  `tx.Deserialize(hex.NewDecoder(strings.NewReader(s)))`. This is a
  streaming decode: the transaction reader pulls only the bytes it needs,
  so any characters following a complete transaction (valid hex or not)
  are never examined and are silently ignored. A truncated transaction
  or an invalid hex digit inside the consumed region is an error whose
  text comes from `encoding/hex` or `wire` (`unexpected EOF`,
  `encoding/hex: invalid byte: …`, or a `wire` message error).
  UNVERIFIED by execution; settle with a Go oracle vector of a valid
  transaction followed by `zz`.

#### 4.1.4 Addresses

`decodeAddress(s, params)` (`methods.go:1829-1851`):

1. If `len(s)` is 66 or 130, `s` is treated as a hex secp256k1 public
   key: `hex.DecodeString` (raw error on failure, not an RPC error) then
   `stdaddr.NewAddressPubKeyEcdsaSecp256k1V0Raw` (raw error on failure).
   The result is a P2PK address.
2. Otherwise `stdaddr.DecodeAddress(s, params)`; on failure the error is
   `-5` (`ErrRPCInvalidAddressOrKey`) with message
   ``invalid address "<s>": decode failed: `<err>` `` (the inner error is
   formatted with `%#q`, i.e. back-quoted when it contains no back-quote).

`decodeStakeAddress` (`methods.go:1853-1863`) additionally requires the
address to implement `stdaddr.StakeAddress` (P2PKH-ecdsa-secp256k1 or
P2SH), else `-5` `invalid stake address "<s>"`.

Raw (non-RPC) errors reach the client through `convertError` as code `-4`
with the error text as the message.

Some handlers call `stdaddr.DecodeAddress` directly and therefore neither
accept hex public keys nor produce the `-5` wrapper: `spendoutputs`
outputs, `sweepaccount` destination, `verifymessage`.

Addresses are rendered with `Address.String()`.

#### 4.1.5 Outpoint strings (`parseOutpoint`, `methods.go:5679-5696`)

Format `<hash>:<index>`. Steps and errors (all `*errors.Error` with op
`parseOutpoint`, no kind):

| Check | Error text |
|---|---|
| `len(s) < 66` | `parseOutpoint: bad len` |
| `s[64] != ':'` | `parseOutpoint: bad separator` |
| `chainhash.NewHashFromStr(s[:64])` fails | `parseOutpoint: <hash error>` |
| `strconv.ParseUint(s[65:], 10, 32)` fails | `parseOutpoint: strconv.ParseUint: parsing "<text>": invalid syntax` (or `value out of range`) |

The hash must therefore be exactly 64 characters. The index accepts
leading zeros, no sign, no whitespace. The returned outpoint always has
`Tree = 0`. A string of exactly 65 characters fails `bad len` even though
the separator is present.

#### 4.1.6 JSON encoding rules that matter here

All results are marshalled by Go `encoding/json` (`makeResponse`, spec 16
§4.2):

- Struct fields are emitted in declaration order.
- `omitempty` omits `false`, `0`, `""`, nil pointers, and nil **or empty**
  slices and maps.
- A nil slice without `omitempty` (or as the top-level result) is `null`;
  an empty non-nil slice is `[]`. Each method below states which one it
  produces.
- Maps are emitted as objects with keys sorted by byte value.
- Strings are HTML-escaped (`<`, `>`, `&` become `\u003c`, `\u003e`,
  `\u0026`), U+2028 and U+2029 are written as the escapes `\u2028`
  and `\u2029`, and each invalid UTF-8 byte is written as the
  six-character escape `\ufffd` (not as the raw three-byte encoding of
  U+FFFD). This applies to account names appearing as values or keys.
- A handler returning `nil` produces `"result":null`.

Nested object parameters are decoded by `encoding/json`, which matches
object keys to struct tags **case-insensitively** and ignores unknown
keys (spec 16 §4.5).

#### 4.1.7 Confirmations

`confirms(txHeight, curHeight)` (`methods.go:75-82`,
`wallet/wallet.go:4411-4418`): `0` if `txHeight == -1` or
`txHeight > curHeight`, else `curHeight - txHeight + 1`.

### 4.2 Parameters

Positional, decoded per spec 16 §4.3. "Opt" parameters may be omitted
(trailing) or `null`; a `null` suppresses the registered default and
leaves the pointer nil, in which case the *handler's* default below
applies (they coincide except where noted).

| Method | # | Name | Go type | Req/Opt | Registered default | Handler default / notes |
|---|---|---|---|---|---|---|
| `getvotechoices` | 1 | tickethash | `*string` | opt | – | nil: default choices |
| `getwalletfee` | – | | | | | |
| `help` | 1 | command | `*string` | opt | – | nil or `""`: usage list |
| `listaccounts` | 1 | minconf | `*int` | opt | 1 | 1 |
| `listlockunspent` | 1 | account | `*string` | opt | – | `""` (all) |
| `listreceivedbyaccount` | 1 | minconf | `*int` | opt | 1 | 1 |
| | 2 | includeempty | `*bool` | opt | false | ignored |
| | 3 | includewatchonly | `*bool` | opt | false | ignored |
| `listreceivedbyaddress` | 1 | minconf | `*int` | opt | 1 | 1 |
| | 2 | includeempty | `*bool` | opt | false | ignored |
| | 3 | includewatchonly | `*bool` | opt | false | ignored |
| `listsinceblock` | 1 | blockhash | `*string` | opt | – | nil: from genesis |
| | 2 | targetconfirmations | `*int` | opt | 1 | 1 |
| | 3 | includewatchonly | `*bool` | opt | false | ignored |
| `listtransactions` | 1 | account | `*string` | opt | – | must be nil or `"*"` |
| | 2 | count | `*int` | opt | 10 | 10 |
| | 3 | from | `*int` | opt | 0 | 0 |
| | 4 | includewatchonly | `*bool` | opt | false | ignored |
| `listaddresstransactions` | 1 | addresses | `[]string` | req | | |
| | 2 | account | `*string` | opt | – | must be nil or `"*"` |
| `listalltransactions` | 1 | account | `*string` | opt | – | must be nil or `"*"` |
| `listunspent` | 1 | minconf | `*int` | opt | 1 | 1 |
| | 2 | maxconf | `*int` | opt | 9999999 | 9999999 |
| | 3 | addresses | `*[]string` | opt | – | nil: no filter |
| | 4 | account | `*string` | opt | – | `""` (all) |
| `lockunspent` | 1 | unlock | `bool` | req | | |
| | 2 | transactions | `[]dcrdtypes.TransactionInput` | req | | objects `{amount, txid, vout, tree}`; only `txid` and `vout` are used |
| `purchaseticket` | 1 | fromaccount | `string` | req | | |
| | 2 | minconf | `*int` | opt | 1 | 1 |
| | 3 | numtickets | `*int` | opt | 1 | 1; values `<= 1` mean 1 |
| | 4 | expiry | `*int` | opt | – | 0 |
| | 5 | comment | `*string` | opt | – | ignored |
| | 6 | dontsigntx | `*bool` | opt | – | false |
| `processunmanagedticket` | 1 | tickethash | `string` | req | | |
| `treasurypolicy` | 1 | key | `*string` | opt | – | nil or `""`: list mode |
| | 2 | ticket | `*string` | opt | – | nil or `""`: none |
| `setdisapprovepercent` | 1 | percent | `uint32` | req | | |
| `settreasurypolicy` | 1 | key | `string` | req | | hex, 33 bytes |
| | 2 | policy | `string` | req | | |
| | 3 | ticket | `*string` | opt | – | nil or `""`: none |
| `tspendpolicy` | 1 | hash | `*string` | opt | – | nil or `""`: list mode |
| | 2 | ticket | `*string` | opt | – | nil or `""`: none |
| `settspendpolicy` | 1 | hash | `string` | req | | exactly 64 chars |
| | 2 | policy | `string` | req | | |
| | 3 | ticket | `*string` | opt | – | nil or `""`: none |
| `redeemmultisigout` | 1 | hash | `string` | req | | |
| | 2 | index | `uint32` | req | | |
| | 3 | tree | `int8` | req | | |
| | 4 | address | `*string` | opt | – | nil: new internal address of account 0 |
| `redeemmultisigouts` | 1 | fromscraddress | `string` | req | | |
| | 2 | toaddress | `*string` | opt | – | as above, per output |
| | 3 | number | `*int` | opt | – | 0xffffffff |
| `rescanwallet` | 1 | beginheight | `*int` | opt | 0 | 0 |
| `spendoutputs` | 1 | account | `string` | req | | |
| | 2 | previousoutpoints | `[]string` | req | | 4.1.5 format |
| | 3 | outputs | `[]AddressAmountPair` | req | | objects `{"address": string, "amount": number}` |
| `ticketinfo` | 1 | startheight | `*int32` | opt | 0 | 0 |
| `sendfrom` | 1 | fromaccount | `string` | req | | |
| | 2 | toaddress | `string` | req | | |
| | 3 | amount | `float64` | req | | DCR |
| | 4 | minconf | `*int` | opt | 1 | 1 |
| | 5 | comment | `*string` | opt | – | must be nil or `""` |
| | 6 | commentto | `*string` | opt | – | must be nil or `""` |
| `sendmany` | 1 | fromaccount | `string` | req | | |
| | 2 | amounts | `map[string]float64` | req | | object address → DCR |
| | 3 | minconf | `*int` | opt | 1 | 1 |
| | 4 | comment | `*string` | opt | – | must be nil or `""` |
| `sendtoaddress` | 1 | address | `string` | req | | |
| | 2 | amount | `float64` | req | | |
| | 3 | comment | `*string` | opt | – | must be nil or `""` |
| | 4 | commentto | `*string` | opt | – | must be nil or `""` |
| `sendtomultisig` | 1 | fromaccount | `string` | req | | ignored |
| | 2 | amount | `float64` | req | | |
| | 3 | pubkeys | `[]string` | req | | hex pubkeys or wallet P2PKH addresses |
| | 4 | nrequired | `*int` | opt | 1 | 1; truncated to `int8` |
| | 5 | minconf | `*int` | opt | 1 | 1 |
| | 6 | comment | `*string` | opt | – | ignored |
| `sendrawtransaction` | 1 | hextx | `string` | req | | |
| | 2 | allowhighfees | `*bool` | opt | false | false |
| `sendtotreasury` | 1 | amount | `float64` | req | | |
| `sendfromtreasury` | 1 | key | `string` | req | | address or hex pubkey whose private key the wallet holds |
| | 2 | amounts | `map[string]float64` | req | | object address → DCR |
| `settxfee` | 1 | amount | `float64` | req | | DCR per kB |
| `setvotechoice` | 1 | agendaid | `string` | req | | |
| | 2 | choiceid | `string` | req | | |
| | 3 | tickethash | `*string` | opt | – | nil: default choices |
| `signmessage` | 1 | address | `string` | req | | |
| | 2 | message | `string` | req | | |
| `signrawtransaction` | 1 | rawtx | `string` | req | | |
| | 2 | inputs | `*[]RawTxInput` | opt | – | nil: none |
| | 3 | privkeys | `*[]string` | opt | – | nil: wallet keys |
| | 4 | flags | `*string` | opt | `"ALL"` | `"ALL"` |
| `signrawtransactions` | 1 | rawtxs | `[]string` | req | | |
| | 2 | send | `*bool` | opt | true | true |
| `sweepaccount` | 1 | sourceaccount | `string` | req | | |
| | 2 | destinationaddress | `string` | req | | |
| | 3 | requiredconfirmations | `*uint32` | opt | – | 1 |
| | 4 | feeperkb | `*float64` | opt | – | wallet relay fee |
| `validateaddress` | 1 | address | `string` | req | | |
| `validatepredcp0005cf` | – | | | | | |
| `verifymessage` | 1 | address | `string` | req | | |
| | 2 | signature | `string` | req | | base64 (standard alphabet, padded) |
| | 3 | message | `string` | req | | |
| `version` | – | | | | | |
| `walletinfo`, `walletislocked`, `walletlock` | – | | | | | |
| `walletpassphrase` | 1 | passphrase | `string` | req | | |
| | 2 | timeout | `int64` | req | | seconds; 0 = no timeout |
| `walletpassphrasechange` | 1 | oldpassphrase | `string` | req | | |
| | 2 | newpassphrase | `string` | req | | |
| `mixoutput` | 1 | outpoint | `string` | req | | 4.1.5 format |
| `mixaccount` | – | | | | | |
| `walletpubpassphrasechange` | 1 | oldpassphrase | `string` | req | | |
| | 2 | newpassphrase | `string` | req | | |
| `setaccountpassphrase` | 1 | account | `string` | req | | |
| | 2 | passphrase | `string` | req | | `""` removes the per-account passphrase |
| `accountunlocked` | 1 | account | `string` | req | | |
| `unlockaccount` | 1 | account | `string` | req | | |
| | 2 | passphrase | `string` | req | | |
| `lockaccount` | 1 | account | `string` | req | | `"*"` = all |
| `getcoinjoinsbyacct` | – | | | | | |

`RawTxInput` (`types/methods.go:977-983`), JSON object keys in order:
`txid` string, `vout` uint32, `tree` int8, `scriptPubKey` string,
`redeemScript` string. Missing keys take the zero value.

Integer narrowing performed by handlers (no range checks, Go two's
complement truncation): `*int` (64-bit) to `int32` for every `minconf`,
`maxconf`, `targetconfirmations`, `expiry`, `beginheight`; `*int` to
`int8` for `nrequired`; `*int` to `uint32` for `number`; `*uint32` to
`int32` for `requiredconfirmations`. A port must truncate identically
(for example `minconf = 4294967297` behaves as `1`).

### 4.3 Results

Field order is declaration order. "omit" = `omitempty`.

**`getvotechoices`** → `GetVoteChoicesResult` (`types/results.go:161-164`):

| Key | Type | Notes |
|---|---|---|
| `version` | uint32 | wallet vote version (6.1) |
| `choices` | array of `VoteChoice` | always an array (`[]` when the version has no agendas) |

`VoteChoice` (`results.go:153-158`): `agendaid` string;
`agendadescription` string, omit; `choiceid` string; `choicedescription`
string, omit.

**`getwalletfee`** → number.

**`help`** → string.

**`listaccounts`** → object, account name → number (spendable balance),
keys sorted.

**`listlockunspent`** → array (never `null`) of
`dcrdtypes.TransactionInput`
(`dcrd/rpc/jsonrpc/types/v4@v4.4.0/chainsvrcmds.go:116-121`): `amount`
number, omit (so a zero-value output has no `amount` key); `txid` string;
`vout` uint32; `tree` int8.

**`listreceivedbyaccount`** → array (never `null`) of
`ListReceivedByAccountResult`: `account` string; `amount` number;
`confirmations` uint64.

**`listreceivedbyaddress`** → array (never `null`) of
`ListReceivedByAddressResult`: `account` string (always `""`, key always
present); `address` string; `amount` number; `confirmations` uint64;
`txids` array of string, omit; `involvesWatchonly` bool, omit (never
set).

**`listsinceblock`** → `ListSinceBlockResult`: `transactions` array
(never `null`) of `ListTransactionsResult`; `lastblock` string.

**`listtransactions`, `listaddresstransactions`, `listalltransactions`**
→ array (never `null`) of `ListTransactionsResult`
(`results.go:216-236`):

| Key | Type | omit | Value as produced by `wallet.listTransactions` (`wallet/wallet.go:2394-2520`) |
|---|---|---|---|
| `account` | string | no | `""` for "send" entries; account name of the single output address for credit entries (`""` if not exactly one address or lookup fails) |
| `address` | string | yes | the output's address when the script yields exactly one |
| `amount` | number | no | output value; negated for "send" |
| `blockhash` | string | yes | mined only |
| `blockindex` | int64 pointer | yes | never set |
| `blocktime` | int64 | yes | block header time (Unix seconds), mined only |
| `category` | string | no | `send`, `receive`, `generate`, `immature` |
| `confirmations` | int64 | no | 0 when unmined |
| `fee` | number pointer | yes | "send" entries only: `(sum outputs − sum debits)` in DCR, i.e. the fee as a **negative** number; `0` when not every input is a wallet debit |
| `generated` | bool | yes | true when `compat.IsEitherCoinBaseTx` holds (`internal/compat/compat.go:19-27`): `standalone.IsCoinBaseTx` with the treasury flag false **or** true, i.e. any transaction with exactly one input whose previous outpoint is the zero hash with index `0xffffffff`. That covers coinbases and, through the flag-false test, also treasurybases and treasury spends. Votes have two inputs and are **not** `generated`. The same predicate selects the `generate`/`immature` categories (`wallet/wallet.go:2380-2388`); credits of every other transaction, votes included, are `receive` |
| `involveswatchonly` | bool | yes | never set |
| `time` | int64 | no | time the wallet first saw the tx (Unix seconds) |
| `timereceived` | int64 | no | same value |
| `txid` | string | no | |
| `txtype` | string pointer | yes | always set: `regular`, `ticket`, `vote`, `revocation` |
| `vout` | uint32 | no | output index |
| `walletconflicts` | array | no | always `[]` |
| `comment` | string | yes | never set |
| `otheraccount` | string | yes | never set |

Entry generation per transaction: for each output in index order, skip it
if it is a wallet credit flagged as change; if the transaction has any
wallet debit, emit a "send" entry; if the output is a wallet credit, emit
a credit entry (so one output can yield zero, one or two entries). The
function returns two lists, `sends` and `receives`; the order in which
callers concatenate them differs per method (5.8-5.11).

**`listunspent`** → array of `ListUnspentResult`, or **`null`** when
there are no matching outputs (`results.go:265-277`): `txid` string;
`vout` uint32; `tree` int8; `txtype` int (`stake.TxType`: 0 regular, 1
ticket, 2 vote, 3 revocation, 4 treasury add, 5 treasury spend, 6
treasurybase); `address` string (first address of the
script, `""` if none; key always present); `account` string;
`scriptPubKey` hex string; `redeemScript` hex string, omit; `amount`
number; `confirmations` int64; `spendable` bool.

**`lockunspent`** → `true`.

**`purchaseticket`** → array of ticket hash strings (never `null`), or,
with `dontsigntx`, `CreateUnsignedTicketResult`
(`types/methods.go:689-692`): `unsignedtickets` array of hex strings;
`splittx` hex string.

**`processunmanagedticket`, `setdisapprovepercent`, `settreasurypolicy`,
`settspendpolicy`, `rescanwallet`, `setvotechoice`, `walletlock`,
`walletpassphrase`, `walletpassphrasechange`,
`walletpubpassphrasechange`, `mixoutput`, `mixaccount`,
`setaccountpassphrase`, `unlockaccount`, `lockaccount`** → `null`.

**`treasurypolicy`** → `TreasuryPolicyResult` object (key given) or array
of them (never `null`): `key` string; `policy` string; `ticket` string,
omit.

**`tspendpolicy`** → `TSpendPolicyResult` object (hash given) or array of
them (never `null`): `hash` string; `policy` string; `ticket` string,
omit.

**`redeemmultisigout`** → `RedeemMultiSigOutResult` (identical layout to
`SignRawTransactionResult`): `hex` string; `complete` bool; `errors`
array of `SignRawTransactionError`, omit.

**`redeemmultisigouts`** → `RedeemMultiSigOutsResult`: `results` array
(never `null`) of `RedeemMultiSigOutResult`.

**`spendoutputs`, `sendfrom`, `sendmany`, `sendtoaddress`,
`sendrawtransaction`, `sendtotreasury`, `sendfromtreasury`** → transaction
hash string.

**`ticketinfo`** → array (never `null`) of `TicketInfoResult`
(`results.go:343-354`): `hash` string; `cost` number; `votingaddress`
string; `status` string; `blockhash` string, omit; `blockheight` int32
(`-1` when unmined); `vote` string, omit; `revocation` string, omit;
`choices` array of `VoteChoice`, omit; `vsphost` string, omit.

**`sendtomultisig`** → `SendToMultiSigResult`: `txhash` string; `address`
string; `redeemscript` hex string.

**`settxfee`** → `true`.

**`signmessage`** → base64 string.

**`signrawtransaction`** → `SignRawTransactionResult`
(`results.go:313-317`): `hex` string; `complete` bool; `errors` array,
omit (absent when there are none). `SignRawTransactionError`
(`results.go:303-309`): `txid` string; `vout` uint32; `scriptSig` hex
string; `sequence` uint32; `error` string.

**`signrawtransactions`** → `SignRawTransactionsResult`: `results` array
(never `null`) of `SignedTransaction` (`results.go:321-325`):
`signingresult` (`SignRawTransactionResult`); `sent` bool; `txhash`
string pointer, omit — **absent** when not attempted, **`""`** when
attempted and failed (10.1 Q-B9).

**`sweepaccount`** → `SweepAccountResult`: `unsignedtransaction` hex
string; `totalpreviousoutputamount` number; `totaloutputamount` number;
`estimatedsignedsize` uint32.

**`validateaddress`** → `ValidateAddressResult` (`results.go:372-389`):
`isvalid` bool; then all omit: `address` string; `ismine` bool;
`iswatchonly` bool (never set); `isscript` bool; `pubkeyaddr` string;
`pubkey` hex string; `iscompressed` bool; `account` string; `addresses`
array of string; `hex` string; `script` string; `sigsrequired` int32;
`accountn` uint32 pointer; `branch` uint32 pointer; `index` uint32
pointer. Pointer fields are emitted when non-nil even if `0`.

**`validatepredcp0005cf`, `verifymessage`, `walletislocked`** → bool.

**`version`** → object, component name →
`dcrdtypes.VersionResult` (`chainsvrresults.go:565-572`):
`versionstring`, `major`, `minor`, `patch`, `prerelease`,
`buildmetadata` (no `omitempty`). See spec 16 §4.8.

**`walletinfo`** → `WalletInfoResult` (`results.go:395-409`):
`daemonconnected` bool; `spv` bool; `unlocked` bool; `cointype` uint32,
omit; `txfee` number; `votebits` uint16; `votebitsextended` hex string;
`voteversion` uint32; `voting` bool; `vsp` string; `manualtickets` bool;
`birthhash` string (`""` when unset, key always present); `birthheight`
uint32 (`0` when unset, key always present).

**`accountunlocked`** → `AccountUnlockedResult`: `encrypted` bool;
`unlocked` bool pointer, omit.

**`getcoinjoinsbyacct`** → object, account name → integer, keys sorted.

---

## 5. Algorithms and state machines

Common preamble. Unless stated otherwise the first step of every handler
is:

> **W.** `w, ok := s.walletLoader.LoadedWallet()`; if not loaded return
> `errUnloadedWallet` = `-4 request requires a wallet but wallet has not
> loaded yet`.

"raw" means the error is returned unconverted and reaches the client via
`convertError` (spec 16 §8.2): code from the wallet error kind (`-4`
unless the kind is `Bug` -32603, `Encoding` -8, `Locked` -13,
`Passphrase` -14, `NoPeers` -9, `InsufficientBalance` -6) and message
`err.Error()`. Wallet error strings are composed by `errors.Error()`
(`errors/errors.go:247-290`) as `op: kind description: detail`, with
nested wallet errors joined by the process-wide separator `":: "`
(`dcrwallet.go:38`).

Two recurring raw errors:

- Account name not found via `w.AccountNumber(name)`: kind `NotExist`,
  text `wallet.AccountNumber: item does not exist: no account "<name>"`
  (`wallet/wallet.go:2083-2096`, `wallet/udb/addressdb.go:761-770`),
  hence code `-4`. Handlers that map it use `errAccountNotFound` =
  `-11 account not found`.
- No network backend via `w.NetworkBackend()`: kind `NoPeers`, text
  `wallet.NetworkBackend: Decred network is unreachable`
  (`wallet/network.go:61-71`), hence code `-9`. Handlers that use
  `s.walletLoader.NetworkBackend()` instead return `errNoNetwork` =
  `-9 disconnected from network`.

Each wallet call below runs in its own database transaction unless a
note says otherwise; handlers never hold a transaction across calls
(exception: `spendoutputs`, 5.17).

### 5.1 `getvotechoices` (`methods.go:2798-2845`)

1. W.
2. If `tickethash` is non-nil: `chainhash.NewHashFromStr`; on error
   `-8 <hash error>`. An empty string is a valid all-zero hash (4.1.2).
3. `version, agendas := wallet.CurrentAgendas(params)`
   (`wallet/wallet.go:291-314`): vote version by network (6.1) and
   `params.Deployments[version]` (possibly nil).
4. `choices, _, err := w.AgendaChoices(ctx, ticketHash)`
   (`wallet/wallet.go:412-469`), one read transaction:
   - no agendas: empty map;
   - every agenda initialised to `"abstain"`;
   - with a ticket hash: if the ticket has no ticket record, error
     kind `NotExist`, text `item does not exist: ticket not found`
     (code `-4`); if the ticket has no saved preference for any agenda,
     the **default** choices are returned instead;
   - otherwise each saved preference replaces `"abstain"`.
   Errors raw.
5. For each agenda in deployment order build `VoteChoice{agendaid:
   Vote.Id, agendadescription: Vote.Description, choiceid:
   choices[id], choicedescription: description of the matching choice}`
   (empty, hence omitted, if the stored choice ID matches no choice).
6. Return `{version, choices}`.

No backend dependence.

### 5.2 `getwalletfee` (`methods.go:2848-2855`)

W, then return `w.RelayFee().ToCoin()` (in-memory value, 5.25).

### 5.3 `help` (`methods.go:2882-2937`)

Specified in spec 16 §5.6. Summary of the exits visible to a client: no
wallet is required; with a `*chain.Syncer` backend and no command the
result is `"Chain server usage:\n\n" + <dcrd help> + "\n\nWallet server
usage (overrides chain requests):\n\n" + requestUsages` (only when dcrd's
usage is non-empty); otherwise `requestUsages`. With a command: the
wallet's help text if the wallet implements the method, else dcrd's
`help <command>` text if non-empty, else
`-8 no help for method "<command>"`. dcrd call errors are returned raw
(a `*wsrpc.Error` from dcrd keeps dcrd's code and message).

### 5.4 `listaccounts` (`methods.go:2941-2971`)

1. W. `minConf` = 1 or `int32(*minconf)`.
2. `w.AccountBalances(ctx, minConf)` (`wallet/wallet.go:1927-1946`): one
   read transaction, `ForEachAccount` order (includes the `imported`
   account and imported-xpub accounts). Raw error.
3. For each balance, `w.AccountName(ctx, account)` (separate read
   transaction each). `NotExist` → `-32603 <error text>`; other errors
   raw.
4. Result map `name → Spendable.ToCoin()`.

Only the **spendable** balance at `minConf` is reported, not the total.

### 5.5 `listlockunspent` (`methods.go:2975-2987`)

W; `account` = `""` or the parameter; return
`w.LockedOutpoints(ctx, account)` (`wallet/wallet.go:4287-4355`):

1. Snapshot the in-memory locked-outpoint set (Go map order, so the
   result order is unspecified).
2. In one read transaction, for each locked `(hash, index)`:
   - transaction not in the wallet: skip silently;
   - `index >= len(TxOut)`: skip silently;
   - if `account` is neither `""` nor `"*"`: extract the output's
     addresses; skip if none; look up the first address's account name
     (empty on failure); skip unless it equals `account`;
   - `tree` = 1 if the transaction's stake type is not regular, else 0;
   - append `{amount: value in DCR, txid, vout, tree}`.
3. Result is always a non-nil array.

An unknown account name is not an error; it simply matches nothing.

### 5.6 `listreceivedbyaccount` (`methods.go:3002-3028`)

1. W. `minConf` = 1 or `int32(*minconf)`.
2. `w.TotalReceivedForAccounts(ctx, minConf)`
   (`wallet/wallet.go:4460-4522`), one read transaction:
   - one result per account in `ForEachAccount` order, initialised to
     zero;
   - `stopHeight = tip − minConf + 1` if `minConf > 0`, else `-1`;
   - `RangeTransactions(0, stopHeight)`; for every credit of every
     transaction, take the first address of the output script, look up
     its account, add the credit amount to that account and set the
     account's `LastConfirmation = confirms(txBlockHeight, tip)`.
3. Emit `{account, amount: TotalReceived.ToCoin(), confirmations:
   uint64(LastConfirmation)}` for every account, including accounts
   that received nothing (`includeempty` is ignored).

`RangeTransactions` semantics (spec 04; `wallet/udb/txquery.go:384-538`)
that are observable here and in 5.7: a negative `end` means "through the
tip **and then unmined transactions**". Therefore `minconf = 0` includes
unmined transactions, and so does any `minconf > tip + 1`, because
`stopHeight` becomes negative (10.1 Q-B3). `confirmations` is the value
for the last credit visited: blocks are visited in ascending height, so
it is the confirmation count of the account's most recent credit in
range (0 if that credit is unmined).

### 5.7 `listreceivedbyaddress` (`methods.go:3044-3132`)

1. W. `tipHeight` from `w.MainChainTip`.
2. `w.SortedActivePaymentAddresses(ctx)` (`wallet/wallet.go:4384-4400`):
   every active address string of the address manager; each becomes a
   map key with a zero entry. Raw error.
3. `minConf` = 1 or `int32(*minconf)`. `endHeight = -1` if
   `minConf == 0`, else `tipHeight − minConf + 1`.
4. `UnstableAPI(w).RangeTransactions(ctx, 0, endHeight, f)` (one read
   transaction; raw error with op `wallet.RangeTransactions`). For each
   batch (all transactions of one block, or all unmined transactions):
   `confirmations = confirms(batch[0].Block.Height, tipHeight)`; for
   every credit of every transaction, for **every** address extracted
   from the credited output script: add the credit amount, overwrite the
   confirmations, append the transaction hash string to `tx` (appended
   once per credit, so a transaction with two outputs to the same
   address appears twice). Addresses not pre-seeded in step 2 are added
   on first sight.
5. Build the result array by ranging over the map: **Go map iteration
   order, i.e. unspecified and different on every call** (10.2 Q-R2).
   Each element: `{account: "", address, amount, confirmations,
   txids}`; `txids` is omitted for addresses with no transactions.

`includeempty` and `includewatchonly` are ignored; addresses with no
receipts are always listed. The same negative-`endHeight` behaviour as
5.6 applies.

### 5.8 `listsinceblock` (`methods.go:3136-3189`)

1. W. `targetConf` = 1 or `int32(*targetconfirmations)`.
2. `targetConf < 1`: `-8 target_confirmations must be positive`.
3. `tipHash, tipHeight := w.MainChainTip`.
4. `w.BlockInfo(ctx, height = tipHeight + 1 − targetConf)`
   (`wallet/wallet.go:2752-2791`). If no main-chain block exists at that
   height (including negative heights) the error is raw, kind
   `NotExist`: `wallet.BlockInfo: item does not exist: no block at height
   <h> in main chain` (code `-4`). `lastBlock` = that block's hash.
5. If `blockhash` is non-nil: `NewHashFromStr`, on error `-22 <hash
   error>`; `w.BlockHeader(ctx, hash)`, on a missing header the raw
   error `item does not exist: block header` (code `-4`); `end =
   header.Height`. Otherwise `end = 0`.
6. `w.ListSinceBlock(ctx, start = -1, end, tipHeight)`
   (`wallet/wallet.go:2525-2547`): `RangeTransactions(-1, end)` visits
   unmined transactions first, then blocks from the tip **down to and
   including** height `end`. For each transaction the credit entries
   are appended first, then the send entries.
7. Return `{transactions, lastblock: lastBlock.String()}`.

Consequences: the transactions of the block named by `blockhash` are
themselves included (the scan is inclusive of `end`); the lookup is by
height only, so a side-chain block hash selects the main-chain range
starting at the same height (source TODO at `methods.go:3164-3165`);
order is newest block first, unmined first of all.

### 5.9 `listtransactions` (`methods.go:3193-3223`)

1. W.
2. If `account` is non-nil and not `"*"`: raw `errors.E(string)`, i.e.
   code `-4`, message exactly
   `Transactions can not be searched by account. Use "*" to reference
   all accounts.`
3. `count` = 10 or `*count`; `from` = 0 or `*from` (no validation).
4. `w.ListTransactions(ctx, from, count)`
   (`wallet/wallet.go:2552-2607`), one read transaction:
   `RangeTransactions(-1, 0)` (unmined first, then tip down to genesis).
   Within a batch, transactions are processed in reverse stored order.
   For each transaction: stop when `n >= count`; skip while fewer than
   `from` transactions have been skipped; otherwise append the send
   entries then the credit entries, and increment `n` if the
   transaction produced any entry. Finally the whole entry list is
   reversed, giving oldest-first output in which each transaction's
   entries appear credits-reversed then sends-reversed.

`count` counts transactions that yield at least one entry, not entries;
`from` counts transactions regardless of whether they yield entries.
`count <= 0` returns `[]`. A negative `from` behaves as 0.

### 5.10 `listaddresstransactions` (`methods.go:3230-3259`)

1. W.
2. `account` non-nil and not `"*"`: `-8 listing transactions for
   addresses may only be done for all accounts`.
3. For each address string: `decodeAddress` (errors as 4.1.4, returned
   at the first bad address). If the address implements
   `stdaddr.Hash160er` (any P2PKH variant or P2SH) add its 20-byte
   hash to a set; other address kinds (P2PK) are silently ignored.
4. `w.ListAddressTransactions(ctx, set)` (`wallet/wallet.go:2612-2651`):
   `RangeTransactions(0, -1)` (genesis to tip, then unmined). A
   transaction is included when any of its credits is a version-0
   output whose `stdscript.ExtractPubKeyHashV0(pkScript)` is in the
   set; its credit entries are appended, then its send entries.

`ExtractPubKeyHashV0` only recognises plain P2PKH-ecdsa-secp256k1
scripts, so P2SH hashes and stake-tagged outputs never match even though
step 3 accepts those addresses (10.1 Q-B4).

### 5.11 `listalltransactions` (`methods.go:3265-3278`)

1. W.
2. `account` non-nil and not `"*"`: `-8 listing all transactions may
   only be done for all accounts`.
3. `w.ListAllTransactions(ctx)` (`wallet/wallet.go:2656-2693`): as 5.9
   without `from`/`count`.

### 5.12 `listunspent` (`methods.go:3281-3324`)

1. W.
2. If `addresses` is non-nil: each is passed through `decodeAddress`
   (first error returned) and its canonical `String()` added to a
   filter set. An empty array means "no filter".
3. `account` = `""` or the parameter; `minConf` = 1; `maxConf` =
   9999999.
4. `w.ListUnspent(ctx, minConf, maxConf, set, account)`
   (`wallet/wallet.go:3477-3667`). If the error is kind `NotExist`
   return `errAddressNotInWallet` (`-4 address not found in wallet`);
   other errors raw.
5. Return the slice (nil, hence `null`, when empty).

`ListUnspent` in one read transaction: take all unspent credits; sort
with `sort.Reverse(creditSlice)` (newest first: unmined by descending
receive time, then mined by descending height; same-transaction outputs
by descending index; the sort is not stable); for each:

1. load the transaction details (error aborts);
2. skip if `confs < minconf` or `confs > maxconf`;
3. skip immature coinbase outputs; for tickets skip output 0 until
   ticket maturity and even-index outputs > 0 until ticket-change
   maturity; skip vote and revocation outputs until coinbase maturity
   (`wallet/wallet.go:4420-4440`);
4. skip outputs locked with `lockunspent`;
5. account name = default account's name unless the first extracted
   address resolves to an account; skip if `account != ""` and differs;
6. if filtering, keep only if any extracted address string is in the
   set;
7. `spendable`: true for P2PKH, P2PK (ecdsa-secp256k1), P2SH (redeem
   script must be known, otherwise the whole call fails), stake-gen,
   stake-revoke and stake-change scripts; for bare multisig only if
   every key address is known to the wallet; false otherwise and
   whenever no address could be extracted;
8. emit the result with `address` = first extracted address.

### 5.13 `lockunspent` (`methods.go:3327-3351`)

1. W.
2. If `unlock` is true and `transactions` is empty (or `null`):
   `w.ResetLockedOutpoints()`.
3. Otherwise for each element in order: `NewHashFromStr(txid)`, on
   error `-22 <hash error>` (elements already processed stay applied);
   then `w.UnlockOutpoint(hash, vout)` or `w.LockOutpoint(hash, vout)`.
4. Return `true`.

The lock set is in-memory only, keyed by `(hash, index)`; `tree` and
`amount` are ignored and nothing checks that the outpoint exists, is
unspent, or belongs to the wallet.

### 5.14 `purchaseticket` (`methods.go:3356-3518`)

1. W.
2. `n, err := w.NetworkBackend()`: raw `NoPeers` error (code `-9`).
3. `w.AccountNumber(fromaccount)`: `NotExist` → `errAccountNotFound`
   (`-11`); other errors raw.
4. `minConf` = 1; if `minconf` is non-nil, `int32(*minconf)` and, if
   negative, `-8 negative minconf`.
5. `numTickets` = 1 unless `*numtickets > 1`.
6. `expiry` = 0 or `int32(*expiry)`. `dontSignTx` = false or the
   parameter. `comment` is never read.
7. `changeAccount = account`. If `s.cfg.MixingEnabled`:
   1. `mixedAccount = AccountNumber(cfg.MixAccount)`; on error
      `-8 Mixing enabled, but error on mixed account: <err>`;
   2. `mixedAccountBranch = cfg.MixBranch`; if not 0 or 1,
      `-8 MixedAccountBranch should be 0 or 1.`;
   3. `mixedSplitAccount = AccountNumber(cfg.TicketSplitAccount)`; on
      error `-8 Mixing enabled, but error on mixedSplitAccount: <err>`;
   4. `changeAccount = AccountNumber(cfg.MixChangeAccount)`; on error
      `-8 Mixing enabled, but error on changeAccount: <err>`.
8. If `cfg.VSPHost != ""`: `w.VSP(VSPClientConfig{URL: cfg.VSPHost,
   PubKey: cfg.VSPPubKey, Policy: {MaxFee: w.VSPMaxFee(), FeeAcct:
   account, ChangeAcct: changeAccount}})`
   (`wallet/vspclientloader.go:12-26`). The client is cached per URL in
   the wallet: the **first** configuration wins, so a later call with a
   different source account reuses the earlier policy. On error
   `-1 VSP Server instance failed to start: <err>`.
9. Build `wallet.PurchaseTicketsRequest{Count, SourceAccount: account,
   MinConf, Expiry, DontSignTx, Mixing: cfg.MixingEnabled, MixedAccount,
   MixedAccountBranch, MixedSplitAccount, ChangeAccount, VSPClient}`;
   `VotingAccount` = `mixedAccount` when mixing is enabled, else
   `account`.
10. `w.PurchaseTickets(ctx, n, request)` (spec 11): raw error.
11. If not `dontSignTx`: return the ticket hashes as strings.
12. Otherwise serialize each returned ticket transaction and the split
    transaction to hex and return `{unsignedtickets, splittx}`.

Backend differences are inside `PurchaseTickets` (spec 11): with SPV,
DCP0010/DCP0012 are assumed active when computing a VSP fee.

### 5.15 `processunmanagedticket` (`methods.go:3522-3561`)

Validation happens **before** the wallet check:

1. `tickethash == ""`: `-8 ticket hash must be provided`.
2. `NewHashFromStr`: `-8 <hash error>`.
3. `cfg.VSPHost == ""`: `-8 vsphost must be set in options`.
4. W.
5. `w.LookupVSP(cfg.VSPHost)`: if no client for that URL has been
   created yet (by application start-up, `purchaseticket`, or gRPC),
   raw `NotExist`: `item does not exist: VSP client for "<url>" not
   found` (code `-4`).
6. `w.NewVSPTicket(ctx, hash)` (`wallet/vspticket.go:41-90`): raw
   errors, e.g. `failed to retrieve ticket: …`, `<hash> is not a
   ticket`, `ticket <hash> has multiple commitments`; requires the
   voting private key, so a locked wallet fails here with a `Locked`
   kind (code `-13`).
7. `vspClient.Process(ctx, ticket, nil)` (spec 11): raw error.
8. Return `null`.

### 5.16 Treasury and tspend policy

Policy string mapping used by all four methods:

| String (set) | `stake.TreasuryVoteT` | String (get, single) | String (get, list) |
|---|---|---|---|
| `"yes"` | `0x01` | `"yes"` | `"yes"` |
| `"no"` | `0x02` | `"no"` | `"no"` |
| `"abstain"`, `"invalid"`, `""` | `0x00` | `"abstain"` | `""` |

#### 5.16.1 `treasurypolicy` (`methods.go:3792-3852`)

1. W.
2. If `ticket` is non-nil and non-empty: `NewHashFromStr`, on error
   `-22 <hash error>`.
3. **Single mode** (`key` non-nil and non-empty): `hex.DecodeString`,
   on error `-22 <hex error>` (no length check);
   `w.TreasuryKeyPolicy(pikey, ticketHash)`
   (`wallet/wallet.go:572-584`): the per-ticket map entry when a ticket
   is given (no fallback to the global key policy), else the global
   entry; absent entries read as abstain. Return `{key: <the key string
   exactly as supplied>, policy, ticket: <the ticket string exactly as
   supplied, when the parameter was non-nil>}`.
4. **List mode**: `w.TreasuryKeyPolicies()`
   (`wallet/wallet.go:640-661`): all global key policies (Go map
   order) followed by all per-ticket policies (Go map order). Emit
   `{key: hex, policy, ticket: hash string when per-ticket}` for each.
   The `ticket` parameter is validated but otherwise **ignored** in
   list mode.

#### 5.16.2 `setdisapprovepercent` (`methods.go:3855-3870`)

1. If `s.activeNet.Net == wire.MainNet`: return
   `dcrjson.ErrInvalidRequest`, which is an `RPCError` **value**, not a
   pointer; `convertError` does not recognise it and the client
   receives code `-4`, message `-32600: Invalid request` (10.1 Q-B1).
2. `percent > 100`: `-8 percent must be from 0 to 100`.
3. W.
4. `w.SetDisapprovePercent(percent)` (atomic store, in-memory only).
   Return `null`.

#### 5.16.3 `settreasurypolicy` (`methods.go:3877-3931`)

1. W.
2. If `ticket` is non-nil and non-empty: length must be 64, else
   `-22 invalid ticket hash length, expected 64 got <n>`; then
   `NewHashFromStr`, on error `-22 <hash error>`.
3. `hex.DecodeString(key)`: `-22 <hex error>`.
4. Decoded length must be 33: `-8 treasury key must be 33 bytes`.
5. Policy string (table above); unknown: `-8 unknown policy "<s>"`
   (Go `%q` quoting).
6. `w.SetTreasuryKeyPolicy(ctx, pikey, policy, ticketHash)`
   (`wallet/wallet.go:666-711`): one write transaction, then the
   in-memory map is updated (entry deleted for abstain). Raw error.
7. `updateVSPVoteChoices(ctx, w, ticketHash, nil, nil, {key: policy})`
   with the key and policy **strings as supplied** (5.19); its error is
   returned raw. The database change of step 6 is not rolled back.

The key is not checked against the network's Pi keys.

#### 5.16.4 `tspendpolicy` (`methods.go:3938-4001`)

1. W. Ticket parsing as 5.16.1 step 2.
2. **Single mode** (`hash` non-nil and non-empty): `NewHashFromStr`,
   `-22` on error; `w.TSpendPolicy(hash, ticketHash)`
   (`wallet/wallet.go:591-628`), resolution order: per-ticket tspend
   policy; global tspend policy; if the tspend is in the wallet's
   in-memory tspend cache, the policy of the Pi key that signed it
   (per-ticket key policy, then global key policy); else abstain.
   Return `{hash: as supplied, policy, ticket: as supplied when
   non-nil}`.
3. **List mode**: `w.GetAllTSpends(ctx)` (`wallet/wallet.go:274-289`)
   returns the cached tspends in Go map order, **deleting** from the
   cache any whose expiry is below the current tip height. For each,
   compute `TSpendPolicy(hash, ticketHash)` and emit `{hash, policy,
   ticket: the ticket string as supplied when non-nil}`. Abstaining
   entries are included with `policy: ""`.

#### 5.16.5 `settspendpolicy` (`methods.go:4008-4064`)

1. W.
2. `len(hash) != 64`: `-22 invalid tspend hash length, expected 64 got
   <n>`; `NewHashFromStr`: `-22`.
3. Ticket as 5.16.3 step 2.
4. Policy string; unknown: `-8 unknown policy "<s>"`.
5. `w.SetTSpendPolicy(ctx, hash, policy, ticketHash)`
   (`wallet/wallet.go:716-761`). Raw error.
6. `updateVSPVoteChoices(ctx, w, ticketHash, nil, {hash: policy},
   nil)`.

### 5.17 Multisig redemption, rescan, `spendoutputs`

#### 5.17.1 `redeemmultisigout` (`methods.go:4071-4162`)

1. W.
2. Destination: if `address` is given, `decodeAddress` (4.1.4);
   otherwise `w.NewInternalAddress(ctx, 0, WithGapPolicyWrap())` — a
   new internal address of the default account is consumed **before**
   any other validation. Raw errors.
3. `NewHashFromStr(hash)`: `-8 <hash error>`.
4. `w.FetchP2SHMultiSigOutput(ctx, {hash, index, tree})`
   (`wallet/multisig.go:21-74`): raw; unknown outpoint is
   `wallet.FetchP2SHMultiSigOutput: item does not exist: no multisig
   output for outpoint <hash>:<index>`. The lookup key is `(hash,
   index)`; `tree` is not part of it.
5. The redeem script must classify as `STMultiSig` (script version 0),
   else raw `P2SH redeem script is not multisig` (code `-4`).
6. Build a version-1 `MsgTx` with one input (the outpoint with the
   caller's `tree`, `ValueIn` = output amount, empty script, sequence
   `0xffffffff`) and call
   `w.PrepareRedeemMultiSigOutTxOutput(msgTx, p2shOutput, &pkScript)`
   (`wallet/multisig.go:78-101`): fee = `FeeForSerializeSize(relayFee,
   EstimateSerializeSize([RedeemP2SHSigScriptSize = 110],
   [output], 0))`; if `fee >= amount`, raw
   `wallet.PrepareRedeemMultiSigOutTxOutput: estimated fee <a> is above
   output value <b>`; else one output of `amount − fee` paying the
   destination script. The output's script **version is left 0**
   regardless of the destination address's script version.
7. Serialize to hex and invoke `signRawTransaction` internally (5.29)
   with `inputs = [{txid: hash as supplied, vout: index, tree,
   scriptPubKey: hex of the P2SH payment script, redeemScript: ""}]`,
   `privkeys = []` (non-nil, empty), `flags = "ALL"`.
8. Return the signing result converted to `RedeemMultiSigOutResult`.

Nothing is published. The already-spent state of the output is not
checked.

#### 5.17.2 `redeemmultisigouts` (`methods.go:4169-4219`)

1. W.
2. `decodeAddress(fromscraddress)`; must be `*AddressScriptHashV0`,
   else `-8 address is not P2SH`.
3. `UnstableAPI(w).UnspentMultisigCreditsForAddress(ctx, addr)` (raw).
4. `max = 0xffffffff`, or `uint32(*number)`.
5. `results := make([]RedeemMultiSigOutResult, len(msos))`. For `i,
   mso` in order with a counter `itr` starting at 0: `if itr > max
   { break }`; call `redeemMultiSigOut` with the outpoint and
   `toaddress`; on error return it; store at `results[i]`; `itr++`.
6. Return `{results}`.

Because the test is `itr > max`, `number = N` processes `N + 1`
outputs, and because the array is pre-sized, the unprocessed tail
remains as zero-valued elements `{"hex":"","complete":false}` (10.1
Q-B5). With no `toaddress`, each processed output consumes a fresh
internal address.

#### 5.17.3 `rescanwallet` (`methods.go:4223-4242`)

1. W.
2. `s.walletLoader.NetworkBackend()`; absent: `errNoNetwork` (`-9
   disconnected from network`).
3. `beginHeight` = 0 or `int32(*beginheight)`.
4. `w.RescanFromHeight(ctx, n, beginHeight)`
   (`wallet/rescan.go:386-407`): raw error; an unknown height gives
   `wallet.RescanFromHeight: item does not exist: no block at height
   <h> in main chain`. Blocks until the rescan finishes or the request
   context is cancelled.
5. Return `null`.

#### 5.17.4 `spendoutputs` (`methods.go:4322-4400`)

1. W.
2. `n, err := w.NetworkBackend()`: raw (`-9`).
3. `w.AccountNumber(account)`: raw (unknown account is `-4`, not
   `-11`).
4. For each previous outpoint string: `parseOutpoint`; on error
   `-8 <parseOutpoint error text>`. Create
   `wire.NewTxIn(op, wire.NullValueIn, nil)` (`ValueIn = -1`,
   sequence `0xffffffff`, tree 0).
5. For each output: `stdaddr.DecodeAddress(address, params)` — raw
   error (code `-4`, no hex-pubkey form); `dcrutil.NewAmount`
   (`-8`, unreachable); `TxOut{Value, PkScript, Version}` from
   `PaymentScript()`. Amounts are not checked for sign, zero, or dust.
6. `rand.ShuffleSlice(inputs)`; `rand.ShuffleSlice(outputs)`
   (cryptographic PRNG).
7. `spendOutputsInputSource(ctx, w, account, inputs)`
   (`methods.go:4247-4292`). For each input:
   1. `w.FetchOutput(ctx, &prevOutPoint)` (`wallet/wallet.go:219-238`):
      raw; unknown transaction is `wallet.FetchOutput: item does not
      exist: tx <hash> not found`. **An index beyond the transaction's
      outputs is not checked and indexes out of range** (10.1 Q-B6).
   2. Accumulate the value; record the pkScript.
   3. The script must be `STPubKeyHashEcdsaSecp256k1`, else raw
      `unsupport address type` (sic).
   4. `w.KnownAddress(ctx, addr)`: raw.
   5. `ka.AccountName() != account`: raw, kind `Invalid`:
      `invalid operation: output address of <hash>:<index> does not
      belong to account "<account>"`.
   6. Per-input "redeem script size" `txsizes.RedeemP2PKHInputSize`
      (166; see 10.1 Q-B33).
   The returned source ignores the requested target and always returns
   all inputs.
8. Change source: `accountChangeSource{account}`.
9. `w.SecretsSource()`: opens a database **read transaction that stays
   open until the deferred `Close`** at handler exit.
10. `txauthor.NewUnsignedTransaction(outputs, w.RelayFee(), inputSource,
    changeSource, params.MaxTxSize)`
    (`wallet/txauthor/author.go:90-166`): raw; insufficient input value
    is `txauthor.NewUnsignedTransaction: insufficient balance` (code
    `-6`). If a change output is needed, `changeSource.Script()` calls
    `w.NewChangeAddress(ctx, account)` (a database write while the read
    transaction of step 9 is open).
11. `atx.RandomizeChangePosition()` — called unconditionally; when no
    change output was added `ChangeIndex` is `-1` and this indexes the
    output slice at `-1` (10.1 Q-B7).
12. `atx.AddAllInputScripts(secretsSource)`: raw (a locked wallet
    yields a `Locked` kind, code `-13`).
13. `w.PublishTransaction(ctx, atx.Tx, n)`
    (`wallet/wallet.go:5108-5167`): raw.
14. Return the hash string.

The published transaction carries `ValueIn = -1` on every input (the
fraud-proof field is never filled in); dcrd's mempool rewrites
fraud-proof data on acceptance (seen in
`dcrd@v1.10.9/internal/mempool/mempool.go:1668-1714`, a dcrd release in
the module cache, not a dcrwallet dependency), and signatures do not
commit to it. The wallet's own record of the transaction keeps `-1`.
Outpoints are always tree 0, so stake-tree outputs cannot be spent with
this method.

### 5.18 `ticketinfo` (`methods.go:4402-4482`)

1. W. `startHeight` = 0 or `*startheight`.
2. `w.GetTickets(ctx, f, start = startHeight, end = -1)`
   (`wallet/wallet.go:3176-3231`): one read transaction ranging blocks
   `startHeight … tip` and then unmined transactions; `f` is called
   once per block (or once for all unmined) that contains at least one
   ticket purchase recorded by the wallet. The summaries are built
   **without** a dcrd client regardless of the backend, so statuses are
   (`wallet/wallet.go:2888-2933`): `unmined`, `immature`, `unspent`,
   `voted`, `missed`, `expired` — never `live`, `unknown` or
   `revoked`.
3. For each ticket:
   1. `status` = the summary status, with `unmined` replaced by
      `immature`.
   2. Deserialize the ticket; `cost` = output 0 value in DCR.
   3. `votingaddress` = first address of output 0; none: abort with
      `unable to decode ticket pkScript`. Every error returned from
      the callback (this one, the deserialize error of step 2, and
      those of steps 6 and 7) passes back through `GetTickets`, which
      wraps it with its op, so the client sees
      `wallet.GetTickets: unable to decode ticket pkScript`
      (`wallet/wallet.go:3227-3229`).
   4. If a header was supplied: `blockhash` = header hash,
      `blockheight` = header height; else `blockheight = -1` and no
      `blockhash`.
   5. Spender: `revocation` = spender hash when it is a revocation,
      else `vote` = spender hash.
   6. `w.AgendaChoices(ctx, ticketHash)` (a nested read transaction;
      falls back to default choices when the ticket has none). On error
      the whole call aborts (`wallet.GetTickets: item does not exist:
      ticket not found` if the transaction is a ticket without a ticket
      record, code `-4`). `choices` = one `{agendaid,
      choiceid}` per map entry in **Go map iteration order**;
      descriptions are not filled in; omitted when the map is empty.
   7. `w.VSPHostForTicket(ctx, ticketHash)`: `NotExist` is ignored;
      other errors abort. `vsphost` = host or omitted.
4. Return the array (always non-nil). An error from step 2 or the
   callback replaces the result.

Array order: ascending block height, unmined last; within a block, the
stored transaction order. This holds for `startheight >= 0`. A negative
`startheight` makes the range function treat the start as "unmined"
(`wallet/udb/txquery.go:392-397, 522-531`): unmined tickets are
visited first and, because both bounds then map to the maximum height,
no mined block is visited at all. Derived by reading; UNVERIFIED by
execution.

### 5.19 `updateVSPVoteChoices` (`methods.go:4810-4864`)

Shared by `setvotechoice`, `settreasurypolicy`, `settspendpolicy`.

- **With a ticket hash**:
  1. `w.VSPHostForTicket`: `NotExist` → return nil (ticket not managed
     by a VSP); other errors returned.
  2. `w.LookupVSP(host)`: error returned (`item does not exist: VSP
     client for "<host>" not found` when no client has been created
     for the ticket's recorded host in this process).
  3. `w.NewVSPTicket(ctx, hash)`: error returned (needs the voting key;
     fails when the wallet is locked).
  4. `vspClient.SetVoteChoice(ctx, ticket, choices, tspendPolicy,
     treasuryPolicy)` (`wallet/vsp.go:293-359`, spec 11): fetches the
     ticket status from the VSP; a status failure other than `Locked`
     is logged and **swallowed** (returns nil); if any supplied
     preference differs from the VSP's, sends an update.
- **Without a ticket hash**: `w.ForUnspentUnexpiredTickets(ctx, f)`
  (`wallet/wallet.go:5705-5747`) calls the same four steps for every
  ticket with status immature/live/unspent in the block range
  `[tip − (TicketExpiry + TicketMaturity − 8), tip]`. Errors from
  individual tickets do not stop the iteration; they are collected and
  returned as `errors.Join`, whose text is the individual messages
  joined with `"\n"`. The source comment "the first error will be
  returned" is wrong.

The local database update has already been committed when this runs; an
error here is reported to the client although the local change stands.

### 5.20 `sendPairs` and its three callers

`sendPairs(ctx, w, amounts, account, minconf)`
(`methods.go:3592-3623`):

1. `changeAccount = account`. If `cfg.MixingEnabled && cfg.MixAccount
   != "" && cfg.MixChangeAccount != ""`: resolve `cfg.MixAccount` (raw
   error); if `account` equals it, `changeAccount` = number of
   `cfg.MixChangeAccount` (raw error).
2. `makeOutputs(amounts, params)`: for each pair in Go map order: a
   negative amount → `errNeedPositiveAmount` (`-8 amount must be
   positive`); `decodeAddress` errors (4.1.4); output `{Value,
   PkScript, Version}` from `PaymentScript()`.
3. `w.SendOutputs(ctx, outputs, account, changeAccount, minconf)`
   (`wallet/wallet.go:4572-4606`, spec 10). Error mapping:
   - kind `Locked` → `errWalletUnlockNeeded` (`-13 wallet or account
     locked; use walletpassphrase or unlockaccount first`);
   - kind `InsufficientBalance` → `-6 <error text>`;
   - anything else raw (a dust or zero output is `wallet.SendOutputs:
     policy violation: transaction output is dust`, code `-4`; no
     backend is a `NoPeers` kind, code `-9`).
4. Return the transaction hash string.

#### 5.20.1 `sendfrom` (`methods.go:4493-4532`)

1. W.
2. `comment` or `commentto` non-empty: `-1 transaction comments are
   unsupported`.
3. `w.AccountNumber(fromaccount)`: raw (`-4`).
4. `amount < 0` (the float): `-8 negative amount`.
5. `minConf` = 1 or `int32(*minconf)`; negative: `-8 negative
   minconf`.
6. `NewAmount(amount)` (`-8`, unreachable).
7. `sendPairs({toaddress: amt}, account, minConf)`.

#### 5.20.2 `sendmany` (`methods.go:4539-4577`)

1. W.
2. `comment` non-empty: `-1 transaction comments are unsupported`.
3. `w.AccountNumber(fromaccount)`: raw.
4. `minConf` as above (`-8 negative minconf`).
5. Convert every amount with `NewAmount`.
6. `sendPairs(pairs, account, minConf)`. An empty or `null` `amounts`
   object is not rejected here.

#### 5.20.3 `sendtoaddress` (`methods.go:4584-4614`)

1. W.
2. `comment` or `commentto` non-empty: `-1 transaction comments are
   unsupported`.
3. `NewAmount(amount)`: raw (unreachable).
4. Converted amount `< 0`: `-8 negative amount`.
5. `sendPairs({address: amt}, account = 0, minconf = 1)`.

### 5.21 `sendtomultisig` (`methods.go:4627-4670`)

1. W. Account is always 0; `fromaccount` and `comment` are ignored.
2. `NewAmount(amount)` (`-8`, unreachable). No sign check here.
3. `nrequired` = 1 or `int8(*nrequired)`; `minConf` = 1 or
   `int32(*minconf)`.
4. `walletPubKeys(ctx, w, pubkeys)` (`methods.go:393-426`): each entry
   through `decodeAddress`; a P2PK address contributes its serialized
   key; any other address must be known to the wallet
   (`errAddressNotInWallet` otherwise) and be a P2PKH address (`-5
   address has no associated public key` otherwise).
5. `w.CreateMultisigTx(ctx, 0, amount, pubKeys, nrequired, minConf)`
   (spec 10): raw.
6. Log at info: `Successfully sent funds to multisignature output in
   transaction <hash>`.
7. Return `{txhash, address: P2SH address, redeemscript: hex}`.

### 5.22 `sendrawtransaction` (`methods.go:4674-4712`)

1. W.
2. `w.NetworkBackend()`: raw (`-9`).
3. Deserialize `hextx` (4.1.3): `-22 <error>`.
4. `allowHighFees` = false or the parameter. If false:
   `txrules.TxPaysHighFees(tx)` (`wallet/txrules/rules.go:134-145`):
   - any input with `ValueIn < 0` → raw, kind `Invalid`: `invalid
     operation: transaction input <i> does not specify the input
     value` (code `-4`);
   - fee = sum of `ValueIn` − sum of outputs; high when positive and
     greater than `FeeForSerializeSize(1000 × 1e4 atoms/kB,
     tx.SerializeSize())`, i.e. more than 0.1 DCR per kB → raw, kind
     `Policy`: `policy violation: high fees` (code `-4`).
5. `w.PublishTransaction(ctx, tx, n)`: if the transaction is relevant
   to the wallet it is recorded first, then
   `n.PublishTransactions`; on publish failure a recorded transaction
   is abandoned and the error is raw with op
   `wallet.PublishTransaction(<hash>)`.
6. Return the hash string.

Backend: with dcrd the publish is dcrd's `sendrawtransaction`; with SPV
it is an inventory announcement to peers (specs 14, 15) and succeeds
without mempool validation.

### 5.23 `sendtotreasury` (`methods.go:4719-4738`,
`sendAmountToTreasury` `3628-3665`)

1. W.
2. `NewAmount(amount)`: raw (unreachable).
3. Converted amount `<= 0`: `-8 negative amount` (also for zero).
4. `sendAmountToTreasury(ctx, w, amt, account = 0, minconf = 1)`:
   1. `changeAccount = 0`. If `cfg.MixingEnabled` (no emptiness check
      on the names, unlike `sendPairs`): resolve `cfg.MixAccount`
      (raw); if it is account 0, `changeAccount` = number of
      `cfg.MixChangeAccount` (raw).
   2. One output `{Value: amt, PkScript: [0xc1] (OP_TADD), Version:
      0}`.
   3. `w.SendOutputsToTreasury(ctx, outputs, 0, changeAccount, 1)`
      (`wallet/wallet.go:4609-4643`; treasury-add authoring, change
      index not randomised). Error mapping as `sendPairs` step 3.
   4. Return the hash string.

### 5.24 `sendfromtreasury` (`methods.go:4742-4750`,
`sendOutputsFromTreasury` `3670-3786`)

1. W.
2. `decodeAddress(key)` (4.1.4): the address (typically a 66-character
   hex public key) whose private key signs the spend.
3. `w.LoadPrivateKey(ctx, addr)`: raw (`wallet.LoadPrivateKey: …`;
   `Locked` kind → `-13`). The key is zeroed on return.
4. `tipHeight` from `w.MainChainTip`.
5. Payload: 32 bytes; bytes 8-31 are filled from the cryptographic
   PRNG; bytes 0-7 are written in step 10.
6. `opretScript = OP_RETURN OP_DATA_32 <payload>` (34 bytes: `6a 20 …`).
   A builder failure would be `-32603 sendOutputsFromTreasury
   NewScriptBuilder: <err>` (unreachable).
7. `msgTx`: `Version = 3` (`wire.TxVersionTreasury`), `TxOut[0] =
   {Value 0, Version 0, opretScript}`, `LockTime 0`,
   `Expiry = CalcTSpendExpiry(tipHeight + 1, params.TreasuryVoteInterval,
   params.TreasuryVoteIntervalMultiplier)`
   (`dcrd/blockchain/standalone/v2@v2.3.0/treasury.go:12-18`):
   `nextTVI = nbh + (tvi − nbh mod tvi)`; `expiry = nextTVI + tvi ×
   multiplier + 2`.
8. For each `(address, amount)` of `amounts` in **Go map iteration
   order** (output order is therefore unspecified):
   1. `NewAmount` (`-8`, unreachable); add to `totalPayout`;
   2. `decodeStakeAddress` (4.1.4);
   3. `PayFromTreasuryScript()` (OP_TGEN-tagged P2PKH or P2SH);
   4. if `txrules.IsDustOutput(txOut, w.RelayFee())`:
      `-8 Amount is dust: <address> <amount>` with the amount in
      `dcrutil.Amount.String()` form (for example `0.00001 DCR`);
      negative amounts are caught here too;
   5. append the output.
9. `fee = FeeForSerializeSize(w.RelayFee(), EstimateSerializeSize([109],
   msgTx.TxOut, 0))` (`txsizes.TSPENDInputSize = 109`).
10. Append the single input: previous outpoint `(zero hash, 0xffffffff,
    tree 0)`, `Sequence 0xffffffff`, `ValueIn = fee + totalPayout`,
    `BlockHeight 0`, `BlockIndex 0xffffffff`, empty script. Write
    `uint64(fee + totalPayout)` little-endian into
    `TxOut[0].PkScript[2:10]`.
11. `sign.TSpendSignatureScript(msgTx, privKey)`
    (`dcrd/txscript/v4@v4.1.2/sign/sign.go:638-655`): Schnorr-secp256k1
    signature over `CalcSignatureHash(nil, SigHashAll, msgTx, 0, nil)`;
    script `OP_DATA_64 <sig> OP_DATA_33 <compressed pubkey> OP_TSPEND`
    (100 bytes). Raw error.
12. `stake.CheckTSpend(msgTx)`: raw error.
13. `s.walletLoader.NetworkBackend()`; absent: `errNoNetwork` (`-9`) —
    checked only now, after signing.
14. `n.PublishTransactions(ctx, msgTx)` **directly on the backend**:
    the wallet does not record the transaction. Raw error.
15. Return `msgTx.TxHash()` as a string.

No check is made that the key is a network Pi key, that the treasury
balance suffices, or that the amounts are within the expenditure policy;
dcrd's mempool decides.

An empty (or `null`) `amounts` object is not rejected up front: steps
1-11 run (the private key is loaded, the transaction is built with only
the `OP_RETURN` output and signed) and step 12 then fails with the raw
`stake.CheckTSpend` error `treasury spend transaction does not have
enough outputs (min: 2, have: 1)` (code `-4`;
`dcrd/blockchain/stake/v5@v5.0.3/treasury.go:155-159`). The other
`CheckTSpend` exits (version, input count, script version, signature
script layout, `OP_TGEN` tagging; `treasury.go:140-215`) cannot be
reached from this builder.

### 5.25 `settxfee` (`methods.go:4753-4773`)

1. W.
2. `amount < 0`: `-8 negative amount`.
3. `NewAmount` (`-8`, unreachable).
4. `w.SetRelayFee(fee)`: in-memory only; not persisted; no upper bound;
   zero accepted.
5. Return `true`.

### 5.26 `setvotechoice` (`methods.go:4780-4808`)

1. W.
2. `tickethash` non-nil: `NewHashFromStr`, `-8` on error (`""` is the
   zero hash).
3. `w.SetAgendaChoices(ctx, ticketHash, {agendaid: choiceid})`
   (`wallet/wallet.go:476-567`). Raw errors:
   - no agendas for the network's vote version: `no agendas to set for
     this network`;
   - ticket hash without a ticket record: `item does not exist: ticket
     not found`;
   - `wallet.SetAgendaChoices: invalid operation: no agenda with ID
     "<id>"`;
   - `wallet.SetAgendaChoices: invalid operation: agenda "<id>" has no
     choice ID "<choice>"`.
   All are code `-4`. On success the preference is written (default or
   per-ticket) and, for the default, the cached default vote bits are
   updated.
4. `updateVSPVoteChoices(ctx, w, ticketHash, choice, nil, nil)` (5.19).
5. Return `null` (or the error from step 4).

### 5.27 `signmessage` (`methods.go:4868-4890`)

1. W.
2. `decodeAddress(address)`.
3. `w.SignMessage(ctx, message, addr)` (`wallet/wallet.go:2008-2032`):
   hash = BLAKE-256 of `varstring("Decred Signed Message:\n") ||
   varstring(message)` where `varstring` is the wire var-int length
   followed by the bytes; signature = `ecdsa.SignCompact(privKey, hash,
   isCompressedKey = true)` (65 bytes). Error mapping: kind `NotExist`
   → `errAddressNotInWallet` (`-4 address not found in wallet`); kind
   `Locked` → `errWalletUnlockNeeded` (`-13`); else raw.
4. Return standard base64 with padding (88 characters).

### 5.28 `verifymessage` (`methods.go:5404-5432`)

No wallet is required.

1. `stdaddr.DecodeAddress(address, s.activeNet)`: raw error (code
   `-4`, message is the stdaddr error text).
2. `base64.StdEncoding.DecodeString(signature)`: raw error (code `-4`,
   for example `illegal base64 data at input byte 3`).
3. The address must be `*AddressPubKeyHashEcdsaSecp256k1V0`, else
   `-8 address must be secp256k1 pay-to-pubkey-hash`.
4. `wallet.VerifyMessage(message, addr, sig, s.activeNet)`
   (`wallet/wallet.go:2036-2063`): recompute the hash as 5.27;
   `ecdsa.RecoverCompact`; hash160 of the recovered key serialized
   compressed or uncompressed according to the signature's header
   byte; compare the resulting P2PKH address string with `addr`.
5. Return `true` only if no error occurred and the addresses match;
   **every** verification error yields `false`, never an RPC error.

### 5.29 `signrawtransaction` (`methods.go:4896-5120`)

1. W.
2. Deserialize `rawtx` (4.1.3): `-22 <error>`.
3. No inputs: `-8 transaction with no inputs cannot be signed`.
4. `flags` (default `"ALL"`), exact case-sensitive match:

   | Flag | Sighash type |
   |---|---|
   | `ALL` | `0x01` |
   | `NONE` | `0x02` |
   | `SINGLE` | `0x03` |
   | `ALL|ANYONECANPAY` | `0x81` |
   | `NONE|ANYONECANPAY` | `0x82` |
   | `SINGLE|ANYONECANPAY` | `0x83` |
   | `ssgen` | `0x01` |
   | `ssrtx` | `0x01` |
   | anything else | `-8 invalid sighash flag` |

5. For each element of `inputs`:
   1. `NewHashFromStr(txid)`: `-8 <hash error>`.
   2. `decodeHexStr(scriptPubKey)`: `-22 hex string decode failed:
      <err>` (odd length is left-padded with `0`).
   3. Only when `privkeys` is non-nil **and non-empty**:
      `decodeHexStr(redeemScript)` (`-22`), then
      `stdaddr.NewAddressScriptHashV0(redeemScript, params)` (raw
      error) and record `scripts[address string] = redeemScript` (an
      empty redeem script is recorded under the hash of the empty
      script).
   4. `inputs[OutPoint{Hash, Index: vout, Tree: tree}] = script`. The
      tree is part of the key: an entry whose `tree` differs from the
      transaction input's tree is never used.
6. If the backend is a `*chain.Syncer` (dcrd RPC mode): for every
   transaction input not supplied in step 5 (skipping input 0 when
   `flags == "ssgen"`), start a goroutine in an `errgroup` calling
   `chainSyncer.GetTxOut(ctx, hash, index, tree, includeMempool =
   true)` and storing the result. In SPV mode or with no backend
   nothing is requested.
7. If `privkeys` is non-nil: for each WIF string
   `dcrutil.DecodeWIF(key, params.PrivateKeyID)`: `-22 <error>`
   (`malformed private key`, `checksum mismatch`, wrong-network error).
   Build the P2PK address for the key's signature suite
   (ecdsa-secp256k1, ed25519, or schnorr-secp256k1; raw error on
   failure) and record `keys[pubkey address string] = wif`; if the
   address has a pubkey-hash form, also `keys[pkh address string] =
   wif`.

   `DecodeWIF` exits, in order
   (`dcrd/dcrutil/v4@v4.0.3/wif.go:101-161`), each surfacing as `-22`
   with the quoted text:
   1. string longer than 54 characters, or base58 payload not exactly
      39 bytes (2 network bytes, 1 signature-suite byte, 32 key bytes,
      4 checksum bytes): `malformed private key`. Base58 decoding of a
      string containing a non-alphabet character yields an empty
      payload (`base58@v1.0.6/base58.go:74-76`), hence the same error;
   2. checksum (first 4 bytes of BLAKE-256 of the first 35 bytes)
      mismatch: `checksum mismatch`;
   3. network bytes differ from `params.PrivateKeyID`: `WIF is not for
      the network identified by 0x<4 hex digits of the wallet's
      PrivateKeyID>` (Go `%#04x` of the 2-byte array, for example
      `0x22de` on mainnet);
   4. suite byte 1 (ed25519) with a scalar the edwards package
      rejects: that package's error text.

   The suite byte is `0` ecdsa-secp256k1, `1` ed25519, `2`
   schnorr-secp256k1 (`dcrd/dcrec@v1.0.1/common.go:12-22`). Any
   **other** suite byte is not rejected by `DecodeWIF`: its switch has
   no default, so it returns a WIF whose suite reads as `0` and whose
   key bytes are nil (10.1 Q-B36). The handler's own switch on the
   suite (`methods.go:5042-5061`) likewise has no default.
8. `errgroup.Wait()`: the first `GetTxOut` error is returned raw
   (`dcrd.GetTxOut: <dcrd message>`, code `-4`). Note steps 7's errors
   return without waiting for the goroutines; they are cancelled by
   the request context only.
9. For each fetched result: a JSON `null` result (output spent or
   unknown to dcrd) is skipped; otherwise
   `hex.DecodeString(result.ScriptPubKey.Hex)` (`-22` on error) is
   recorded in `inputs`.
10. `signErrs, signErr := w.SignTransaction(ctx, tx, hashType, inputs,
    keys, scripts)` (`wallet/wallet.go:4773-4909`, spec 10). Key
    behaviours visible here:
    - previous output scripts come from `inputs` when present, else
      from the wallet's transaction store; a transaction not in the
      store makes the whole call fail with `signErr` = `… <outpoint>
      not found` (an out-of-range index on a known transaction is not
      checked, 10.1 Q-B6);
    - when the `keys` map is **non-empty**, only those keys and the
      supplied redeem scripts are used; when it is empty or nil
      (including `privkeys: []`), wallet keys and scripts are used;
    - input 0 is skipped when the transaction is a vote
      (`stake.IsSSGen`), regardless of `flags`;
    - a failure to sign an input is recorded as a per-input error
      (text `wallet.SignTransaction: <err>`) and the loop continues
      with the next input;
    - after signing, each input is run through the script engine. An
      engine failure is recorded as a per-input error, with one
      exception: when the failure is `ErrInvalidStackOperation`, the
      previous output script **as found in the `inputs` map** (not the
      one read from the wallet's transaction store) is P2SH, and the
      redeem script for it is a multisig script, nothing is recorded
      (too few signatures so far). A wallet-store-sourced P2SH
      multisig input with too few signatures therefore *is* reported
      as an error (`wallet/wallet.go:4878-4900`);
    - an engine failure other than `ErrInvalidStackOperation` also
      becomes the function's `signErr` (the last such error wins,
      `wallet/wallet.go:4889, 4903`), in addition to its per-input
      entry;
    - a SIGHASH_SINGLE-type input whose index has no matching output
      is not signed, only verified (`wallet/wallet.go:4853-4868`).
11. Serialize the (possibly partially signed) transaction to hex. The
    hex is returned **even when `signErr` is non-nil**; `signErr`
    itself is never reported to the client except through `complete`.
12. `errors` = one element per `signErrs` entry: `{txid, vout} of the
    input's previous outpoint, scriptSig: hex of the input's current
    signature script, sequence, error: err.Error()}`.
13. Return `{hex, complete: len(errors) == 0 && signErr == nil,
    errors}`.

### 5.30 `signrawtransactions` (`methods.go:5123-5203`)

No wallet check of its own (the inner call performs it). With an empty
`rawtxs` array the inner call never runs, so no wallet is required:
`send = false` returns `{"results":[]}` and `send = true` reaches the
backend check of step 3.

1. For each raw transaction in order: `signRawTransaction` with
   `flags = "ALL"`, no inputs, no private keys. The first error aborts
   the whole call.
2. `send` = true unless the parameter is false.
3. If sending: `s.walletLoader.NetworkBackend()`; absent →
   `errNoNetwork` (`-9`) (after all signing has been done). For each
   result:
   - `complete`: re-deserialize the hex (`-22` on error, unreachable);
     `n.PublishTransactions(ctx, tx)` directly on the backend (the
     wallet does not record it). Success: `{signingresult, sent: true,
     txhash: "<hash>"}`. Failure: the error is discarded and the
     element is `{signingresult, sent: false, txhash: ""}`.
   - not complete: `{signingresult, sent: false}` (no `txhash` key).
4. If not sending: every element `{signingresult, sent: false}`.
5. Return `{results}`.

### 5.31 `sweepaccount` (`methods.go:5241-5303`)

1. W.
2. `feePerKb` = `w.RelayFee()`, or `NewAmount(*feeperkb)` (`-8`,
   unreachable; no sign or range check).
3. `requiredConfs` = 1, or `int32(*requiredconfirmations)`; if that
   is negative (values ≥ 2^31): `errNeedPositiveAmount` (`-8 amount
   must be positive`).
4. `w.AccountNumber(sourceaccount)`: `NotExist` →
   `errAccountNotFound` (`-11`); else raw.
5. `makeScriptChangeSource(destinationaddress, params)`:
   `stdaddr.DecodeAddress` raw error (code `-4`).
6. `w.NewUnsignedTransaction(ctx, outputs = nil, feePerKb, account,
   requiredConfs, OutputSelectionAlgorithmAll, changeSource, nil)`
   (`wallet/createtx.go:82-162`): selects every eligible output of the
   account (excluding locked outpoints) and sends everything minus the
   fee to the "change" script. Kind `InsufficientBalance` → `-6 <error
   text>`; other errors raw.
7. Return `{unsignedtransaction: hex, totalpreviousoutputamount:
   TotalInput, totaloutputamount: sum of outputs,
   estimatedsignedsize}`.

The transaction is neither signed, recorded, published, nor are its
inputs locked.

### 5.32 `validateaddress` (`methods.go:5306-5389`)

1. W.
2. `decodeAddress(address)`. On **any** error return
   `{"isvalid":false,"script":"nonstandard"}` (no RPC error).
3. `address` = canonical string; `isvalid = true`; `script` = name of
   the script class of the address's payment script
   (`stdscript.ScriptType.String()`: `pubkey`, `pubkey-ed25519`,
   `pubkey-schnorr-secp256k1`, `pubkeyhash`, `pubkeyhash-ed25519`,
   `pubkeyhash-schnorr-secp256k1`, `scripthash`).
4. If the address is a public-key address: `pubkey` = hex of the
   serialized key, `pubkeyaddr` = the address string itself.
5. `isscript = true` when the class is `scripthash`.
6. `iscompressed = true` when the address has a 20-byte hash (every
   P2PKH variant **and P2SH**); never for public-key addresses.
7. `w.KnownAddress(ctx, addr)`: `NotExist` → return the result so far;
   other errors raw.
8. `ismine = true`; `account` = account name.
9. By wallet address kind:
   - P2PKH: `pubkey` = hex of the wallet's public key; `pubkeyaddr` =
     the P2PK address string for it.
   - P2SH: `hex` = redeem script hex; `addresses` = addresses extracted
     from the redeem script (omitted when none); `script` is
     **overwritten** with the redeem script's class (e.g. `multisig`,
     `nonstandard`); for multisig, `sigsrequired` =
     `DetermineRequiredSigs`.
10. If the address is BIP0044-derived (seed accounts and imported-xpub
    accounts, not the `imported` account): `branch` (0 external, 1
    internal) and `index` are set; `accountn` is set only when the
    account is not an imported-xpub account.

### 5.33 `validatepredcp0005cf` (`methods.go:5392-5400`)

W, then `w.ValidatePreDCP0005CFilters(ctx)`
(`wallet/wallet.go:5230-5268`); return `(err == nil, err)`. The result
is therefore `true` or an error, never `false`. Errors (raw, code `-4`):
`wallet.ValidatePreDCP0005CFilters: The current network does not have
pre-DCP0005 cfilters` on networks other than mainnet and testnet3;
`… hash of provided cfilters <h> does not match expected hash <h>`;
range errors when filters are missing.

### 5.34 `version` (`methods.go:5438-5463`)

No wallet required. If the backend is a `*chain.Syncer`, call dcrd's
`version` and start from its map (raw error on failure). Add/overwrite
`dcrwallet` (`versionstring = version.String()`, `major`, `minor`,
`patch`, `prerelease`, `buildmetadata`) and `dcrwalletjsonrpcapi`
(`"11.0.0"`, 11, 0, 0, `""`, `""`). See spec 16 §4.8.

### 5.35 `walletinfo` (`methods.go:5468-5541`)

1. W.
2. Backend (`w.NetworkBackend()`, error ignored):
   - `*spv.Syncer`: `spv = true`; `daemonconnected` = at least one
     remote peer;
   - `*chain.Syncer`: call dcrd `ping`; if the request context is done
     return `ctx.Err()` raw (`-4 context canceled`); on error log
     `Ping failed on connected daemon client: <err>` (warn) and leave
     `daemonconnected = false`; else `true`;
   - none: log `walletInfo - no network backend` (warn);
   - any other type: log `walletInfo - invalid network backend (<type>).`
     (error) and return `-1 invalid network backend`.
3. `w.CoinType(ctx)`: on a `WatchingOnly` kind, log (debug) `Watching
   only wallets do not store the coin type keys.` and use 0; on any
   other error log `Failed to retrieve the active coin type: <err>`
   and use 0. A zero coin type is omitted from the result.
4. `unlocked = !w.Locked()`; `txfee = RelayFee().ToCoin()`; `votebits`
   = cached default vote bits; `votebitsextended` = hex of the extended
   bits (4 bytes: vote version little-endian); `voteversion` = the
   first 4 extended-bits bytes read little-endian; `voting =
   w.VotingEnabled()`; `vsp = cfg.VSPHost`; `manualtickets`.
5. `w.BirthState(ctx)`: on error log `Failed to get birth state:
   <err>`; if a state exists and neither `SetFromTime` nor
   `SetFromHeight` is set, `birthhash` = hash string and `birthheight`
   = height. Otherwise both stay at zero values (`""`, `0`).
6. Return the object.

### 5.36 Wallet lock state

- **`walletislocked`** (`5546-5553`): W; return `w.Locked()`.
- **`walletlock`** (`5558-5566`): W; `w.Lock()`; return `null`. Never
  errors (also on watching-only wallets, contrary to the source
  comment).
- **`walletpassphrase`** (`5572-5586`): W; `timeout = time.Second ×
  timeout`; a non-zero timeout creates `time.After(timeout)`; call
  `w.Unlock(ctx, []byte(passphrase), channelOrNil)`
  (`wallet/wallet.go:1712-1744`); error raw (`wallet.Unlock: invalid
  passphrase…` → `-14`; watching-only → `-4`). A wrong passphrase
  given to an already unlocked wallet **locks** it. Timeout handling
  (`replacePassphraseTimeout`, `wallet/wallet.go:1804-1839`): if the
  wallet was already unlocked **without** a timeout, a non-zero timeout
  is ignored (the channel is merely drained) and the wallet stays
  unlocked indefinitely; otherwise the new timeout replaces any
  earlier one, and a zero timeout cancels an earlier one. A negative
  timeout fires immediately (a locked wallet is unlocked and then
  locked at once; subject to the same "already unlocked without a
  timeout" exception); the multiplication wraps for very large values.
- **`walletpassphrasechange`** (`5595-5611`): W;
  `w.ChangePrivatePassphrase(ctx, old, new)`; kind `Passphrase` → `-14
  incorrect passphrase`; else raw.
- **`walletpubpassphrasechange`** (`5700-5713`): W;
  `w.ChangePublicPassphrase(ctx, old, new)`; the `Passphrase` check has
  its arguments swapped (`errors.Is(errors.Passphrase, err)`) and never
  matches, so every error is raw: a wrong old passphrase yields code
  `-14` (from the kind) with the full wallet error text, not the fixed
  `incorrect passphrase` message (10.1 Q-B8).

### 5.37 Per-account encryption

All four start with W and resolve the account with
`w.AccountNumber` (`NotExist` → `errAccountNotFound` `-11`; else raw),
except `lockaccount "*"`.

- **`setaccountpassphrase`** (`5715-5731`):
  `w.SetAccountPassphrase(ctx, account, []byte(passphrase))`; raw
  error; `null`.
- **`accountunlocked`** (`5733-5765`): `w.AccountHasPassphrase` (raw);
  not encrypted → `{"encrypted":false}`; else `w.AccountUnlocked`
  (raw) → `{"encrypted":true,"unlocked":<bool>}`.
- **`unlockaccount`** (`5767-5783`): `w.UnlockAccount(ctx, account,
  []byte(passphrase))`; raw; `null`.
- **`lockaccount`** (`5785-5817`): if `account == "*"`: `w.Accounts`
  (raw), then `w.LockAccount` for every account that is individually
  encrypted and currently unlocked (first error returned raw);
  `null`. Otherwise resolve the name and `w.LockAccount` (raw);
  `null`.

### 5.38 Mixing triggers

- **`mixoutput`** (`5613-5647`):
  1. `!cfg.MixingEnabled`: raw `Mixing is not configured` (code `-4`)
     — before the wallet check.
  2. W.
  3. `parseOutpoint(outpoint)`: **raw** (code `-4`, text as 4.1.5).
  4. `AccountNumber(cfg.MixAccount)` then
     `AccountNumber(cfg.MixChangeAccount)`: `NotExist` →
     `errAccountNotFound`; else raw.
  5. `w.MixOutput(ctx, outpoint, changeAccount, mixAccount,
     cfg.MixBranch)` (spec 13): blocks until the mix completes; raw
     error; `null`.
- **`mixaccount`** (`5649-5677`): same without step 3, calling
  `w.MixAccount(ctx, changeAccount, mixAccount, cfg.MixBranch)`. The
  outputs mixed are those of the **change** account.

### 5.39 `getcoinjoinsbyacct` (`methods.go:5834-5863`)

1. W.
2. `w.GetCoinjoinTxsSumbByAcct(ctx)` (`wallet/wallet.go:5554-5609`):
   over all mined regular transactions (blocks 0 … tip, unmined
   excluded), for those `PossibleCoinJoin` classifies as a mix, count
   each output whose value equals the mix denomination and whose
   single address belongs to a wallet account. Kind `Passphrase` →
   `-14 incorrect passphrase` (dead branch); else raw.
3. Map each account number to its name (`NotExist` → `-32603 <text>`;
   else raw).
4. Return the object `name → count`. Accounts with no such outputs are
   absent; the result is `{}` when there are none.

---

## 6. Constants, limits and defaults

### 6.1 Table

| Name | Value | Source | What depends on it |
|---|---|---|---|
| `scriptVersionAssumed` | 0 | `methods.go:69` | `redeemmultisigout` redeem-script classification |
| Vote version, mainnet | 11 | `wallet/wallet.go:291-304` | `getvotechoices.version`, agenda set, `walletinfo.voteversion` |
| Vote version, testnet3 | 12 | same | same |
| Vote version, simnet | 12 | same | same |
| Vote version, net `0x48e7a065` (testnet2) | 6 | same | same |
| Vote version, any other net | 1 | same | same |
| Default `minconf` (`listaccounts`, `listreceivedby*`, `listunspent`, `purchaseticket`, `sendfrom`, `sendmany`, `sendtomultisig`) | 1 | handlers | selection and totals |
| Fixed `minconf` of `sendtoaddress`, `sendtotreasury` | 1 | `methods.go:4613, 4737` | input selection |
| Fixed account of `sendtoaddress`, `sendtotreasury`, `sendtomultisig`, default `redeemmultisigout` destination | 0 (`udb.DefaultAccountNum`) | `methods.go:4613, 4737, 4634, 4089` | source of funds / new address |
| Default `maxconf` (`listunspent`) | 9999999 | `methods.go:3311` | filter |
| Default `count` / `from` (`listtransactions`) | 10 / 0 | `methods.go:3213-3220` | paging |
| Default `targetconfirmations` | 1 | `methods.go:3143` | `lastblock` |
| Default `numtickets` | 1 (also for any value ≤ 1) | `methods.go:3386-3391` | ticket count |
| Default `expiry` (`purchaseticket`) | 0 | `methods.go:3394` | ticket expiry |
| Default `number` (`redeemmultisigouts`) | `0xffffffff` | `methods.go:4190` | loop bound |
| Default `beginheight` / `startheight` | 0 | `methods.go:4235, 4411` | scan start |
| Default `flags` (`signrawtransaction`) | `"ALL"` | `methods.go:4914` | sighash type |
| Default `send` (`signrawtransactions`) | true | `methods.go:5148` | publishing |
| Default `requiredconfirmations` (`sweepaccount`) | 1 | `methods.go:5259` | input selection |
| Default `nrequired` | 1 | `methods.go:4639` | multisig threshold |
| Sighash values | ALL `0x01`, NONE `0x02`, SINGLE `0x03`, ANYONECANPAY `0x80` | `dcrd/txscript/v4@v4.1.2` | signatures |
| `parseOutpoint` minimum length / separator offset | 66 / 64 | `methods.go:5681-5684` | outpoint strings |
| `chainhash.MaxHashStringSize` | 64 | `chainhash@v1.0.5/hash.go:17` | hash parsing; exact-length checks in `settreasurypolicy`, `settspendpolicy` |
| Treasury key length | 33 (`secp256k1.PubKeyBytesLenCompressed`) | `methods.go:3902` | `settreasurypolicy` |
| `setdisapprovepercent` maximum | 100 | `methods.go:3860` | validation |
| `stake.TreasuryVoteInvalid/Yes/No` | `0x00` / `0x01` / `0x02` | `stake/v5@v5.0.3/staketx.go:826-836` | policy mapping |
| `OP_RETURN`, `OP_TADD`, `OP_TSPEND`, `OP_TGEN` | `0x6a`, `0xc1`, `0xc2`, `0xc3` | `txscript/v4@v4.1.2/opcode.go:157, 246-248` | treasury transactions |
| `wire.TxVersionTreasury` | 3 | `wire@v1.8.0/msgtx.go:28` | tspend version |
| `wire.MaxPrevOutIndex`, `MaxTxInSequenceNum`, `NullBlockIndex` | `0xffffffff` | `wire@v1.8.0/msgtx.go:32-51` | tspend input |
| `wire.NullValueIn` | -1 | `wire@v1.8.0/msgtx.go:43` | `spendoutputs` inputs |
| `wire.NullBlockHeight` | 0 | `wire@v1.8.0/msgtx.go:47` | tspend input |
| `txsizes.TSPENDInputSize` | 109 | `wallet/txsizes/size.go:117` | tspend fee |
| `txsizes.RedeemP2PKHSigScriptSize` | 108 | `wallet/txsizes/size.go:28` | fee estimation |
| `txsizes.RedeemP2SHSigScriptSize` | 110 | `wallet/txsizes/size.go:40` | `redeemmultisigout` fee |
| `txsizes.RedeemP2PKHInputSize` | 166 | `wallet/txsizes/size.go:55` | `spendoutputs` fee |
| `txsizes.P2PKHOutputSize` | 36 | `wallet/txsizes/size.go:108` | `accountChangeSource.ScriptSize()` (note: an output size used where a script size is expected, so change/dust and fee estimation assume a 36-byte change script) |
| `txrules.DefaultRelayFeePerKb` | 10000 atoms | `wallet/txrules/rules.go:38` | high-fee threshold |
| High-fee threshold | fee > `FeeForSerializeSize(1000 × 10000 atoms/kB, size)` | `wallet/txrules/rules.go:118-127` | `sendrawtransaction` |
| Tspend expiry | `nextTVI + tvi × mul + 2` | `standalone/v2@v2.3.0/treasury.go:12-18` | `sendfromtreasury` |
| `TreasuryVoteInterval` / multiplier | mainnet 288 / 12; testnet3 60 / 4; simnet 48 / 3 | `chaincfg/v3@v3.3.0/*params.go` | tspend expiry |
| Message magic | `"Decred Signed Message:\n"` | `wallet/wallet.go:2011, 2041` | `signmessage`, `verifymessage` |
| VSP update ticket window | `tip − (TicketExpiry + TicketMaturity − 8)` … `tip` | `wallet/wallet.go:5734-5739` | which tickets `set*` methods push to VSPs |
| `DCP0005ActiveHeightMainNet` / `TestNet3` | 431488 / 323328 | `validate/validate.go:21-25` | `validatepredcp0005cf` range (heights 0 … activation − 1) |
| Pre-DCP0005 cfilter set hash, mainnet | `f95e09f9ded38f8d6c32e5158a1f286633881393659218c63f5ab0fc86b36c83` | `validate/validate.go:114` | `validatepredcp0005cf` |
| Pre-DCP0005 cfilter set hash, testnet3 | `619c08f5adda6f834212bbdaee3002fdc4efed731477af6c0fed490bbe2488d0` | `validate/validate.go:112` | same |
| JSON-RPC API version | `11.0.0` | `methods.go:55-60` | `version` (spec 16) |
| WIF maximum encoded length / decoded length | 54 characters / 39 bytes | `dcrutil/v4@v4.0.3/wif.go:113-124` | `signrawtransaction` `privkeys` (`malformed private key`) |
| Signature-suite bytes | 0 ecdsa-secp256k1, 1 ed25519, 2 schnorr-secp256k1 | `dcrec@v1.0.1/common.go:12-22` | WIF suite, P2PK address kind built for each key |
| Minimum tspend outputs | 2 (`OP_RETURN` plus one payout) | `stake/v5@v5.0.3/treasury.go:155-159` | `sendfromtreasury` with empty `amounts` |
| Tspend random payload | 24 bytes at payload offset 8 (script offset 10) | `methods.go:3686-3687` | `sendfromtreasury` `OP_RETURN` |
| Compact signature length / base64 length | 65 bytes / 88 characters | `wallet/wallet.go:2008-2032`, `methods.go:4889` | `signmessage` result |
| Help locale | `"en_US"` (hardcoded) | `methods.go:2913-2917` | `help <command>` text |

### 6.2 Error codes used in scope

| Code | dcrjson name(s) | Used for |
|---|---|---|
| -1 | `ErrRPCMisc`, `ErrRPCUnimplemented` | VSP client start failure; transaction comments; invalid backend type |
| -4 | `ErrRPCWallet` | default for raw errors; `errUnloadedWallet`; `errAddressNotInWallet` |
| -5 | `ErrRPCInvalidAddressOrKey` | address decode failures; non-stake address; address without public key |
| -6 | `ErrRPCWalletInsufficientFunds` | insufficient balance |
| -8 | `ErrRPCInvalidParameter` | parameter validation; some hash parse failures |
| -9 | `ErrRPCClientNotConnected` | no network backend |
| -11 | `ErrRPCWalletInvalidAccountName` | `errAccountNotFound` |
| -13 | `ErrRPCWalletUnlockNeeded` | locked wallet/account |
| -14 | `ErrRPCWalletPassphraseIncorrect` | wrong passphrase |
| -22 | `ErrRPCDeserialization`, `ErrRPCDecodeHexString` | transaction/WIF/hex decode; some hash parse failures |
| -32603 | `ErrRPCInternal.Code` | account number without a name; `Bug`-kind wallet errors |

---

## 7. Concurrency and lifecycle

- **Goroutines.** A handler runs on the goroutine the transport gives it
  (spec 16 §7.1): the HTTP server's per-connection goroutine for POST,
  a per-request goroutine for websocket. Handlers in scope start
  goroutines in exactly one place: `signrawtransaction` starts one
  `errgroup` goroutine per transaction input that needs a `gettxout`
  from dcrd (`methods.go:4995-5026`); they are joined at
  `requestedGroup.Wait()` on the success path only (10.1 Q-B15).
  `walletpassphrase` indirectly causes the wallet to start a timeout
  goroutine (`wallet/wallet.go:1804-1839`).
- **Handler-level shared state.** Only `helpDescs` guarded by
  `helpDescsMu` (`methods.go:2875-2876`). Everything else is per-request
  or lives in the wallet. `help <command>` takes `helpDescsMu` before
  the lookup and releases it by `defer` at handler return
  (`methods.go:2910-2911`), so for a command the wallet does not
  implement the mutex is **held across the dcrd round trip**
  (`methods.go:2927-2932`): concurrent `help <command>` requests are
  serialized behind a slow or stalled dcrd until the request context
  ends. `help` with no command never takes the mutex. The map is built
  once, on the first `help <command>` call, and never invalidated.
- **`signrawtransaction` shared state.** The `requested` map filled by
  the `gettxout` goroutines is guarded by a function-local
  `requestedMu` (`methods.go:4993-4994, 5020-5022`); the goroutines run
  on the derived `errgroup` context `gctx`, so the first failing
  `gettxout` cancels the others, and `Wait` returns that first error.
- **Wallet locks touched** (acquired and released inside wallet
  methods, never held across handler steps): `lockedOutpointMu`
  (`lockunspent`, `listlockunspent`, `listunspent`, `sweepaccount`,
  `lockaccount "*"` via `Accounts`, `sendrawtransaction`/`spendoutputs`
  via `PublishTransaction`), `stakeSettingsLock` (vote bits, treasury
  and tspend policies, tspend cache; `SetTreasuryKeyPolicy` and
  `SetTSpendPolicy` hold it across their database write),
  `relayFeeMu`, `passphraseUsedMu` and `passphraseTimeoutMu`
  (lock/unlock/passphrase change), `vspClientsMu`, `networkBackendMu`,
  and the loader mutex.
- **Atomicity.** No handler is atomic across its wallet calls.
  Concurrent requests interleave freely; in particular `set*` methods
  commit locally and then talk to VSPs, `lockunspent` applies elements
  one at a time, `listaccounts` reads balances and names in separate
  transactions, and `listreceivedbyaddress` reads the tip, the address
  list and the transaction range in three separate transactions.
- **Database transactions.** Handlers never open transactions
  themselves except `spendoutputs`, whose `SecretsSource` holds a read
  transaction from `methods.go:4378` until handler return, spanning a
  possible address-derivation write and the publish. `ticketinfo`
  opens nested read transactions (`AgendaChoices`, `VSPHostForTicket`)
  from inside the `GetTickets` read transaction on the same goroutine.
  The nested-read pattern is tolerated because bbolt allows concurrent
  read transactions. The read-then-write pattern of `spendoutputs` is
  one bbolt documents as able to deadlock the writer when the write
  needs to re-mmap a grown database file while the read transaction is
  open (`bbolt@v1.3.12/db.go:708-711`); upstream is exposed to that.
  A port on a different storage engine must not deadlock here (12.3).
- **Cancellation.** The request context is cancelled when the client
  disconnects or the server shuts down (spec 16 §7.3). It is passed to
  every wallet call. Long-running methods that only return on
  completion or cancellation: `rescanwallet`, `mixoutput`,
  `mixaccount`, `purchaseticket` (mixed purchases, VSP calls),
  `processunmanagedticket`, the `set*` methods when VSPs are slow,
  `signrawtransaction` (dcrd round trips), `sendrawtransaction` and the
  other publishers. A cancelled handler's error is whatever the wallet
  returned (usually a wrapped `context canceled`, code `-4`); the
  dispatcher additionally logs a warning (spec 16 §5.5.2).
- **Blocking.** There is no per-method timeout. `rescanwallet` holds
  one of the limited client slots (spec 16 §5.3) for the whole rescan.
- **Panics.** Nothing in the package recovers from panics. The two
  reachable index-out-of-range panics (10.1 Q-B6, Q-B7) therefore
  terminate the process when the request arrived over websocket (the
  request goroutine is the server's own) and are caught by `net/http`
  (connection closed without a response, stack trace on stderr) when it
  arrived over HTTP POST. In both cases the panic can occur inside a
  database read transaction.
- **Ordering guarantees clients can rely on.** Within one request,
  local state changes precede remote effects: `settxfee` takes effect
  for the next authored transaction; `lockunspent` takes effect before
  the response; a `set*` response (success or VSP error) is sent only
  after the local commit.

---

## 8. Errors

Conversion rules are in spec 16 §8.2. This section lists every error a
handler in scope creates itself. `<e>` is the Go `Error()` text of the
underlying error.

### 8.1 Fixed and formatted RPC errors

| Code | Message | Raised by |
|---|---|---|
| -4 | `request requires a wallet but wallet has not loaded yet` | every method except `help`, `verifymessage`, `version` |
| -8 | `<hash error>` | `getvotechoices`, `setvotechoice` (tickethash); `processunmanagedticket`; `redeemmultisigout` (hash); `signrawtransaction` (input txid) |
| -22 | `<hash error>` | `listsinceblock` (blockhash); `lockunspent` (txid); `treasurypolicy`, `tspendpolicy`, `settreasurypolicy`, `settspendpolicy` (ticket, tspend hash) |
| -22 | `<hex error>` | `treasurypolicy`, `settreasurypolicy` (key); `signrawtransaction` (dcrd-returned script hex) |
| -22 | `hex string decode failed: <e>` | `signrawtransaction` input `scriptPubKey` / `redeemScript` |
| -22 | `<deserialize error>` | `sendrawtransaction`, `signrawtransaction`, `signrawtransactions` (transaction hex) |
| -22 | `<wif error>` | `signrawtransaction` private keys |
| -22 | `invalid ticket hash length, expected 64 got <n>` | `settreasurypolicy`, `settspendpolicy` |
| -22 | `invalid tspend hash length, expected 64 got <n>` | `settspendpolicy` |
| -8 | `no help for method "<name>"` | `help` |
| -32603 | `<e>` | `listaccounts`, `getcoinjoinsbyacct` when an account number has no name |
| -8 | `target_confirmations must be positive` | `listsinceblock` |
| -8 | `listing transactions for addresses may only be done for all accounts` | `listaddresstransactions` |
| -8 | `listing all transactions may only be done for all accounts` | `listalltransactions` |
| -4 | `address not found in wallet` | `listunspent` (`NotExist`), `signmessage`, `sendtomultisig` |
| -11 | `account not found` | `purchaseticket`, `sweepaccount`, `mixoutput`, `mixaccount`, `setaccountpassphrase`, `accountunlocked`, `unlockaccount`, `lockaccount` |
| -8 | `negative minconf` | `purchaseticket`, `sendfrom`, `sendmany` |
| -8 | `Mixing enabled, but error on mixed account: <e>` | `purchaseticket` |
| -8 | `MixedAccountBranch should be 0 or 1.` | `purchaseticket` |
| -8 | `Mixing enabled, but error on mixedSplitAccount: <e>` | `purchaseticket` |
| -8 | `Mixing enabled, but error on changeAccount: <e>` | `purchaseticket` |
| -1 | `VSP Server instance failed to start: <e>` | `purchaseticket` |
| -8 | `<serialize error>` | `purchaseticket` split transaction serialization (unreachable) |
| -8 | `ticket hash must be provided` | `processunmanagedticket` |
| -8 | `vsphost must be set in options` | `processunmanagedticket` |
| -8 | `amount must be positive` | `makeOutputs` (negative amount); `sweepaccount` (`requiredconfirmations` ≥ 2^31) |
| -13 | `wallet or account locked; use walletpassphrase or unlockaccount first` | `sendPairs`, `sendAmountToTreasury`, `signmessage` |
| -6 | `<e>` | `sendPairs`, `sendAmountToTreasury`, `sweepaccount` (`InsufficientBalance`) |
| -32603 | `sendOutputsFromTreasury NewScriptBuilder: <e>` | `sendfromtreasury` (unreachable) |
| -8 | `<e>` (`invalid coin amount`) | every `NewAmount` wrapper (unreachable) |
| -8 | `Amount is dust: <address> <amount>` | `sendfromtreasury` |
| -9 | `disconnected from network` | `rescanwallet`, `sendfromtreasury`, `signrawtransactions` |
| -8 | `percent must be from 0 to 100` | `setdisapprovepercent` |
| -8 | `treasury key must be 33 bytes` | `settreasurypolicy` |
| -8 | `unknown policy "<s>"` | `settreasurypolicy`, `settspendpolicy` |
| -8 | `address is not P2SH` | `redeemmultisigouts` |
| -8 | `<parseOutpoint error>` | `spendoutputs` |
| -1 | `transaction comments are unsupported` | `sendfrom`, `sendmany`, `sendtoaddress` |
| -8 | `negative amount` | `sendfrom`, `sendtoaddress`, `sendtotreasury` (also zero), `settxfee` |
| -8 | `transaction with no inputs cannot be signed` | `signrawtransaction` |
| -8 | `invalid sighash flag` | `signrawtransaction` |
| -8 | `address must be secp256k1 pay-to-pubkey-hash` | `verifymessage` |
| -1 | `invalid network backend` | `walletinfo` |
| -14 | `incorrect passphrase` | `walletpassphrasechange`; `getcoinjoinsbyacct` (dead) |
| -5 | ``invalid address "<s>": decode failed: `<e>` `` | every `decodeAddress` caller |
| -5 | `invalid stake address "<s>"` | `sendfromtreasury` |
| -5 | `address has no associated public key` | `sendtomultisig` |

### 8.2 Raw errors created in scope (surface via `convertError`)

| Text | Kind → code | Raised by |
|---|---|---|
| `Transactions can not be searched by account. Use "*" to reference all accounts.` | none → -4 | `listtransactions` |
| `-32600: Invalid request` | none → -4 | `setdisapprovepercent` on mainnet |
| `P2SH redeem script is not multisig` | none → -4 | `redeemmultisigout` |
| `unsupport address type` | none → -4 | `spendoutputs` |
| `invalid operation: output address of <hash>:<index> does not belong to account "<name>"` | `Invalid` → -4 | `spendoutputs` |
| `wallet.GetTickets: unable to decode ticket pkScript` (the handler creates the bare text; `GetTickets` prefixes its op) | none → -4 | `ticketinfo` |
| `policy violation: high fees` | `Policy` → -4 | `sendrawtransaction` |
| `Mixing is not configured` | none → -4 | `mixoutput`, `mixaccount` |
| `parseOutpoint: bad len` / `bad separator` / `<e>` | none → -4 | `mixoutput` |
| stdaddr decode error text | none → -4 | `spendoutputs`, `sweepaccount`, `verifymessage` |
| base64 decode error text | none → -4 | `verifymessage` |
| hex decode error text (hex-pubkey branch of `decodeAddress`) | none → -4 | every `decodeAddress` caller given a 66- or 130-character string |
| `context canceled` | none → -4 | `walletinfo` during ping |
| `treasury spend transaction does not have enough outputs (min: 2, have: 1)` | none (stake rule error) → -4 | `sendfromtreasury` with empty `amounts` (5.24) |
| `failed to parse public key: malformed public key: invalid length: 0` (stdaddr `ErrInvalidPubKey` wrapping the secp256k1 parse error; `txscript/v4@v4.1.2/stdaddr/addressv0.go:134-138`, `dcrec/secp256k1/v4@v4.4.1/pubkey.go:176-180`). Text composed by reading; UNVERIFIED by execution | none → -4 | `signrawtransaction` given a WIF with an unknown signature-suite byte (Q-B36) |
| dcrd's own code and message (`*wsrpc.Error` passes through `convertError` unchanged) | dcrd's | `help` and `version` in dcrd-RPC mode when the dcrd call fails at the RPC level; a transport failure is a plain error → -4 |

### 8.3 Log lines emitted in scope (subsystem `RPCS`)

| Level | Text | Where |
|---|---|---|
| info | `Successfully sent funds to multisignature output in transaction <hash>` | `sendtomultisig` |
| warn | `Ping failed on connected daemon client: <e>` | `walletinfo` |
| warn | `walletInfo - no network backend` | `walletinfo` |
| error | `walletInfo - invalid network backend (<Go type>).` | `walletinfo` |
| debug | `Watching only wallets do not store the coin type keys.` | `walletinfo` |
| error | `Failed to retrieve the active coin type: <e>` | `walletinfo` |
| error | `Failed to get birth state: <e>` | `walletinfo` |

---

## 9. Compatibility-critical behaviours

1. **Method names, positional parameter order, optionality and
   defaults** exactly as 4.2, including the ignored parameters
   (`includeempty`, `includewatchonly`, `comment` on `purchaseticket` and
   `sendtomultisig`, `fromaccount` on `sendtomultisig`) which must still
   be accepted and type-checked.
2. **Result shapes** exactly as 4.3: key names (including the camel-case
   `scriptPubKey`, `redeemScript`, `scriptSig`, `involvesWatchonly`),
   key order, `omitempty` behaviour, and the `null`-versus-`[]`
   distinction — notably `listunspent` returning `null` when empty,
   `listreceivedbyaddress` always emitting `"account":""`, `walletinfo`
   always emitting `birthhash`/`birthheight`, and `validateaddress`
   returning `{"isvalid":false,"script":"nonstandard"}` for undecodable
   input instead of an error.
3. **Amounts as JSON floats** with Go's shortest-round-trip formatting
   and `NewAmount` rounding (4.1.1). Clients (dcrctl, exchange
   integrations) parse these numbers; a different formatter changes
   bytes and, for some values, the parsed atom count.
4. **Category strings** `send`, `receive`, `generate`, `immature`;
   **txtype strings** `regular`, `ticket`, `vote`, `revocation`;
   negative `amount` and negative `fee` on "send" entries.
5. **Ticket status strings** from `ticketinfo`: `immature`, `unspent`,
   `voted`, `missed`, `expired` only (never `live`), `blockheight: -1`
   for unmined tickets.
6. **Error codes and messages** of section 8. Automation matches on
   codes: `-13` to trigger an unlock, `-6` for insufficient funds,
   `-11` for unknown accounts, `-9` for no backend, `-14` for bad
   passphrases. The mixed use of `-4`/`-11` for unknown accounts and
   `-8`/`-22` for bad hashes must be preserved per method.
7. **`signmessage`/`verifymessage` format**: BLAKE-256 over the two
   var-strings with the magic `Decred Signed Message:\n`, 65-byte
   compact signature flagged compressed, standard padded base64.
   Signatures must interoperate with dcrwallet, Decrediton, and
   third-party verifiers.
8. **Treasury spend construction** (5.24): version 3, null input with
   `ValueIn = fee + payout`, `OP_RETURN` + 32-byte payload whose first 8
   bytes are the little-endian amount, OP_TGEN outputs, expiry formula,
   Schnorr signature script layout. dcrd's `CheckTSpend` and mempool
   must accept the result. The 24 random bytes and the output order are
   free.
9. **Treasury add construction**: single `OP_TADD` (0xc1) output,
   script version 0.
10. **`signrawtransaction` semantics**: the flag strings (including
    `ssgen` and `ssrtx`), odd-length hex padding for scripts, the rule
    that a non-empty `privkeys` list disables wallet keys and scripts,
    the `(hash, index, tree)` matching of supplied inputs, dcrd
    `gettxout` lookups only in dcrd-RPC mode, and the
    `{hex, complete, errors}` reply in which `hex` is returned even on
    failure. Cold-wallet and multisig workflows depend on each.
11. **`purchaseticket` with `dontsigntx`** returns
    `{unsignedtickets, splittx}` hex for offline signing; without it an
    array of hash strings, even for one ticket.
12. **`sweepaccount` returns an unsigned transaction** and has no side
    effects; callers sign it with `signrawtransaction` and publish with
    `sendrawtransaction`.
13. **Wallet-affecting side effects**: `settxfee` is in-memory and lost
    on restart; `lockunspent` locks are in-memory, keyed by `(txid,
    vout)`, unvalidated; `setvotechoice`, `settreasurypolicy`,
    `settspendpolicy` persist to the same database keys as the gRPC
    equivalents (spec 05) and then push to VSPs (5.19) using the vspd
    client protocol (spec 11).
14. **VSP interaction**: `purchaseticket` registers the VSP client under
    the configured URL; `processunmanagedticket` looks the client up by
    the configured `cfg.VSPHost` (`methods.go:3534-3544`), the `set*`
    methods by the host stored with the ticket
    (`methods.go:4814-4822`); each fails with
    `item does not exist: VSP client for "<host>" not found` if no
    client was registered under that key in this process.
15. **`walletpassphrase` timeout semantics**: 0 = unlocked indefinitely;
    a later call replaces the earlier timeout, except that a timeout
    given while the wallet is already unlocked with no timeout is
    ignored (5.36); a wrong passphrase on an unlocked wallet locks it.
16. **`version`** keys and the API version `11.0.0`; clients gate on
    the major version (spec 16 §4.8).
17. **`walletinfo`** fields, in particular `daemonconnected` semantics
    per backend (dcrd: a successful `ping`; SPV: at least one peer) and
    `spv`.
18. **`sendrawtransaction` high-fee guard** (default on) and the
    record-then-publish-then-abandon-on-failure behaviour of
    `PublishTransaction`.
19. **Methods usable without a loaded wallet**: `help`, `verifymessage`,
    `version`.
20. **Dispatch-level behaviour retained from spec 16**: methods not in
    the table are passed through to dcrd in RPC mode, so a port must
    not add or remove table entries (e.g. implementing `getrawtransaction`
    locally would change SPV-mode errors).

---

## 10. Quirks

### 10.1 Bugs and surprising behaviour

**Q-B1. `setdisapprovepercent` on mainnet returns a malformed error.**
The handler returns the value `dcrjson.ErrInvalidRequest` (type
`RPCError`, not `*RPCError`); `convertError` falls through to its
default and the client sees `{"code":-4,"message":"-32600: Invalid
request"}`. `methods.go:3856-3858`, `errors.go:16-49`,
`dcrd/dcrjson/v4@v4.2.0/jsonerr.go:10-13`.

**Q-B2. `listunspent` returns `null`, not `[]`, when nothing matches.**
`wallet/wallet.go:3479` declares a nil slice that is returned unchanged.

**Q-B3. A `minconf` larger than `tip + 1` widens instead of narrows.**
In `listreceivedbyaccount` and `listreceivedbyaddress` the stop height
`tip − minconf + 1` goes negative, which the range function treats as
"through the tip and then unmined transactions".
`methods.go:3081-3087`, `wallet/wallet.go:4486-4492`,
`wallet/udb/txquery.go:392-397, 533-536`.

**Q-B4. `listaddresstransactions` accepts any hash-bearing address but
can only match plain P2PKH outputs.** `methods.go:3249-3255` versus
`wallet/wallet.go:2627-2633`.

**Q-B5. `redeemmultisigouts` `number` is off by one and leaves
zero-valued array elements.** `methods.go:4195-4216`.

**Q-B6. Unchecked output indexes panic.** `w.FetchOutput`
(`wallet/wallet.go:230`, reached from `spendoutputs`) and
`SignTransaction` (`wallet/wallet.go:4809`, reached from
`signrawtransaction`, `signrawtransactions`, `redeemmultisigout`) index
`TxOut[index]` of a wallet transaction without a bounds check. A request
naming an existing wallet transaction with an out-of-range output index
panics (process exit over websocket; dropped connection over HTTP, 7).

**Q-B7. `spendoutputs` panics when no change output is created.**
`atx.RandomizeChangePosition()` is called unconditionally
(`methods.go:4389`); with `ChangeIndex == -1` it evaluates
`outputs[-1]` (`wallet/txauthor/author.go:171-181`). The method works
only when the inputs exceed outputs plus fee by a non-dust amount.
Verified by reading; UNVERIFIED by execution.

**Q-B8. `walletpubpassphrasechange` never produces `incorrect
passphrase`.** `errors.Is(errors.Passphrase, err)` has its arguments
reversed (`methods.go:5709`); the client gets code `-14` with the raw
wallet error text.

**Q-B9. `signrawtransactions` hides publish errors.** A failed publish
yields `"sent":false,"txhash":""` (pointer to an empty string, so the
key is present); the error is discarded. Transactions are published
straight to the backend and never recorded by the wallet.
`methods.go:5158-5181`.

**Q-B10. Validation order leaks configuration before wallet state.**
`processunmanagedticket` validates its argument and the VSP option
before checking for a loaded wallet; `mixoutput`/`mixaccount` check
`MixingEnabled` first; `setdisapprovepercent` checks the network and
range first.

**Q-B11. `redeemmultisigout` consumes an internal address before
validating its other arguments**, and always writes script version 0 on
the output. `methods.go:4083-4102`, `wallet/multisig.go:88`.

**Q-B12. Inconsistent unknown-account errors.** `sendfrom`, `sendmany`,
`spendoutputs` return the raw `-4 wallet.AccountNumber: item does not
exist: no account "<name>"`; other methods return `-11 account not
found`.

**Q-B13. Inconsistent hash-parse codes** (`-8` versus `-22`, 8.1), and
`mixoutput` returns outpoint errors raw (`-4`) where `spendoutputs`
wraps the same text as `-8`.

**Q-B14. Empty-string optional hashes.** `getvotechoices` and
`setvotechoice` treat `""` as the all-zero ticket hash (yielding `ticket
not found`); the treasury/tspend methods treat `""` as absent.

**Q-B15. `signrawtransaction` leaks in-flight `gettxout` goroutines on
a WIF error** (returns between starting and waiting for the errgroup)
and swallows the fatal `signErr` from `SignTransaction`, reporting only
`complete: false`. `methods.go:5035-5039, 5094-5119`.

**Q-B16. `signrawtransaction` ignores the WIF signature suite when
signing.** Keys of all three suites are accepted and indexed, but
`SignTransaction` always signs with ecdsa-secp256k1
(`wallet/wallet.go:4824`).

**Q-B17. Treasury policy queries are asymmetric.** Single-key and
single-hash queries report abstain as `"abstain"`; list queries report
it as `""`. `treasurypolicy` list mode ignores `ticket`; `tspendpolicy`
list mode includes abstaining tspends (contrary to help) and prunes
expired tspends from the wallet cache as a side effect of a read.

**Q-B18. `sendfromtreasury` bypasses the wallet** when publishing and
checks for a backend only after building and signing.

**Q-B19. `spendoutputs` publishes inputs with `ValueIn = -1`,** always
uses tree 0, does not validate output amounts (negative, zero, dust),
and holds a read transaction across a write (7).

**Q-B20. `sweepaccount` reports a too-large `requiredconfirmations` as
`amount must be positive`** and does not validate `feeperkb`
(negative or zero rates are passed to the author).

**Q-B21. `validateaddress.iscompressed` is true for P2SH addresses and
false for public-key addresses**; `script` is overwritten by the redeem
script class for wallet P2SH addresses; `pubkeyaddr` for a public-key
address input is the input address itself.

**Q-B22. `sendtotreasury` reports a zero amount as `negative amount`.**
The sign checks also differ: `sendfrom` and `settxfee` test the float
before rounding, so `-0.000000001` is rejected; `sendtoaddress` tests
after rounding, so the same value rounds to zero atoms, passes the
handler, and fails later with the dust policy error; `sendmany` and
`sendtomultisig` have no handler-level sign check of their own
(`sendmany` relies on `makeOutputs`).

**Q-B23. `walletpassphrase` accepts negative timeouts** (immediate
re-lock, unless the wallet was already unlocked without a timeout, in
which case any non-zero timeout is ignored) and overflows for timeouts
above ~9.2e9 seconds.

**Q-B24. `purchaseticket` VSP policy is fixed by the first caller.**
`w.VSP` caches by URL; the fee and change accounts of the first
`purchaseticket` (or of start-up configuration) are reused for later
calls with a different `fromaccount`.

**Q-B25. `updateVSPVoteChoices` reports VSP failures after the local
change is committed**, returns all per-ticket errors joined by newlines
despite the comment, and silently ignores failures to fetch the ticket
status from the VSP.

**Q-B26. `ticketinfo` can fail as a whole** if any ticket-shaped
transaction in range lacks a ticket record (`AgendaChoices` returns
`ticket not found`). Whether such a state is reachable depends on when
ticket records are written (spec 04); UNVERIFIED.

**Q-B27. `listtransactions` paging counts transactions, not entries**,
and `count` only counts transactions that produced entries. "send"
entries for zero-value outputs (ticket commitments, `OP_RETURN`) carry
`"amount":-0`.

**Q-B28. `listsinceblock` includes the named block's own
transactions** and resolves the block by height only.

**Q-B29. `decodeAddress` treats any 66- or 130-character string as a
hex public key**, returning a raw (`-4`) hex or key error rather than
the `-5` address error.

**Q-B30. `sendtomultisig` truncates `nrequired` to `int8`** and ignores
`fromaccount`.

**Q-B31. `getcoinjoinsbyacct` has an unreachable `Passphrase` branch.**

**Q-B32. `lockunspent` applies elements before validating later
ones**, so a bad `txid` in position *k* leaves the first *k − 1*
changes applied while returning an error.

**Q-B33. `spendoutputs` feeds whole-object sizes where script sizes
are expected.** `accountChangeSource.ScriptSize()` returns an output
size (36) instead of a script size (25) (`methods.go:4314-4317`), and
`spendOutputsInputSource` records `RedeemP2PKHInputSize` (166, a whole
input) per input where the author expects a signature-script size (the
wallet's own input source uses `RedeemP2PKHSigScriptSize` = 108,
`wallet/udb/txmined.go:3326`). `EstimateSerializeSize` therefore counts
224 bytes per input instead of 166 and 47 bytes of change output
instead of 36, so `spendoutputs` pays a higher fee than other send
methods and its dust test for change uses script size 36. The fee is
observable in the published transaction and must be reproduced to
obtain identical outputs.

**Q-B34. VSP pushes on young chains.** `ForUnspentUnexpiredTickets`
starts at `tip − (TicketExpiry + TicketMaturity − 8)`; when that is
negative (chains shorter than the expiry window, e.g. a fresh simnet)
the range function interprets the negative start as "unmined first,
then from the tip downward to the end height", which visits only
unmined tickets and the tip block (`wallet/wallet.go:5734-5741`,
`wallet/udb/txquery.go:392-420, 521-538`). Derived by reading;
UNVERIFIED by execution. Owned by the wallet-core spec; noted here
because it changes which tickets the `set*` methods push to VSPs.

**Q-B35. `listsinceblock` contains dead code.** After the `targetConf <
1` rejection (`methods.go:3148-3150`) the guard `if targetConf > 0`
(`methods.go:3154`) is always true, so the initial `lastBlock =
&tipHash` (`methods.go:3153`) is never the value returned; `lastblock`
always comes from the `BlockInfo` lookup, and the handler fails when
that lookup fails.

**Q-B36. A WIF with an unknown signature-suite byte is misreported.**
`DecodeWIF` accepts a correctly checksummed, correct-network WIF whose
suite byte is not 0, 1 or 2 and returns suite `0` with nil key bytes
(`dcrutil/v4@v4.0.3/wif.go:138-161`). `signrawtransaction` then takes
its ecdsa branch and `NewAddressPubKeyEcdsaSecp256k1V0Raw(nil)` fails;
that error is returned **raw** (code `-4`, text in 8.2) instead of the
`-22` every other bad WIF produces (`methods.go:5036-5047`). No panic
occurs: the nil address is never dereferenced because the constructor
error returns first.

**Q-B37. `ssrtx` is accepted but has no effect; `ssgen` only suppresses
one lookup.** Both map to `SigHashAll` (`methods.go:4931-4934`). The
only other use of the flag string is skipping the dcrd `gettxout` for
input 0 when it is exactly `"ssgen"` (`methods.go:5001`). Whether input
0 is *signed* is decided inside `SignTransaction` from the transaction
itself (`stake.IsSSGen`), not from the flag.

**Q-B38. Treasury transactions are `regular` in listings.**
`listTransactions` maps only ticket, vote and revocation types; treasury
adds, treasury spends and treasurybases fall through to `txtype:
"regular"` (`wallet/wallet.go:2415-2423`) while treasurybase and tspend
credits are simultaneously `generated: true` with category
`generate`/`immature` (4.3). `listunspent.txtype` is unaffected (numeric
`stake.TxType`, 4-6 for treasury types).

**Q-B39. Unreachable branches kept by upstream.** In addition to the
`NewAmount` wrappers (4.1.1), Q-B31 and Q-B35: the `signedTxResult ==
nil` half of the guard in `redeemMultiSigOut` (`methods.go:4157`;
`signRawTransaction` never returns `nil, nil`); the script-builder
error in `sendOutputsFromTreasury` (`methods.go:3691-3695`); the
serialize-to-`strings.Builder` errors in `purchaseTicket`
(`methods.go:3499-3510`), `redeemMultiSigOut` (`methods.go:4142-4145`),
`signRawTransaction` (`methods.go:5098-5101`) and `sweepAccount`
(`methods.go:5290-5293`); the re-deserialize error in
`signRawTransactions` (`methods.go:5162-5165`); `return hashStrs, err`
in `purchaseTicket` with `err` known nil (`methods.go:3491`); the
`coinType = 0` assignment in `walletInfo` (`methods.go:5507`; the value
is already 0 on error). A port may omit them.

**Q-B40. `sendfromtreasury` does its expensive work before rejecting
trivially bad requests.** The private key is loaded (and a locked
wallet reported) before any amount or payee is examined; an empty
`amounts` object is rejected only by `CheckTSpend` after signing
(5.24), and a missing backend only after that.

**Q-B41. `help <command>` holds a process-wide mutex across a dcrd
call** (7; `methods.go:2910-2932`).

### 10.2 Go-runtime-dependent behaviour

**Q-R1. Float-to-integer overflow.** `NewAmount` converts
`f × 1e8 ± 0.5` to `int64` without a range check. For out-of-range
values Go's result is implementation-defined: on amd64 it is
`-9223372036854775808` for any out-of-range value; on arm64 it
saturates. An amount such as `1e300` therefore behaves as a negative
amount on amd64 and as `MaxInt64` atoms on arm64. Rust's `as i64`
saturates (arm64 behaviour).

**Q-R2. Map iteration order** decides: the element order of
`listreceivedbyaddress`, `listlockunspent`, `treasurypolicy` (list),
`tspendpolicy` (list), `ticketinfo[].choices`; the output order of
transactions built by `sendmany` (`makeOutputs`) and
`sendfromtreasury`. Go randomises it per iteration. Results emitted
from maps directly (`listaccounts`, `getcoinjoinsbyacct`, `version`)
are sorted by key and deterministic.

**Q-R3. Integer truncation** of 64-bit parameters to `int32`, `int8`,
`uint32` (4.2).

**Q-R4. nil versus empty slices** (4.1.6, 4.3) and `omitempty` on empty
non-nil slices (`signrawtransaction.errors`, `ticketinfo.choices`,
`validateaddress.addresses`).

**Q-R5. `encoding/json` specifics**: HTML-escaping of `<`, `>`, `&` in
strings (account names, error texts inside `signrawtransaction.errors`),
sorted map keys, `-0` for negative zero, case-insensitive object-key
matching and silently ignored unknown keys in `RawTxInput`,
`TransactionInput`, `AddressAmountPair`; duplicate keys in an object
parameter keep the last value.

**Q-R6. `sort.Sort` is not stable**: `listunspent` ordering among
credits that compare equal (same height, different transactions) is
unspecified.

**Q-R7. `time.After` with a wrapped or negative duration** (Q-B23).

**Q-R8. `errors.Join` text** uses `"\n"` between messages (Q-B25).

**Q-R9. Panic handling** differs by transport (7).

**Q-R10. Streaming hex decode** ignores trailing data after a complete
transaction (4.1.3).

### 10.3 Help text versus behaviour

Help strings are from `internal/rpchelp/helpdescs_en_US.go`; declared
result types from `internal/rpchelp/methods.go`. The generated help text
itself must be reproduced verbatim (spec 16); the table records where it
misdescribes the behaviour specified above.

| Method | Help says | Code does |
|---|---|---|
| `listtransactions`, `gettransaction` details | category is `"recv"` for ordinary receipts (`helpdescs:448, 626`) | category string is `"receive"` (`wallet/wallet.go:2361-2372`) |
| `listtransactions` | `blockhash` "or the empty string if unmined"; `blocktime` "or 0 if unmined" | both keys are omitted when unmined (`omitempty`) |
| `listtransactions` | `fee`: "total input value minus the total output value" | reported as outputs minus inputs (negative) |
| `listunspent` | result is a single object (`methods.go:89` declares `*ListUnspentResult`) | array of objects, or `null` |
| `listunspent` | `redeemScript` "if scriptPubKey is P2SH" | correct, but the whole call fails if a P2SH output's script is unknown |
| `listreceivedbyaccount` | "Number of block confirmations of the most recent transaction relevant to the account" | confirmations of the last credit visited in range; 0 for accounts with none |
| `listreceivedbyaddress` | `txids`: "all transactions involving this address" | only transactions crediting it; repeated per credit |
| `listsinceblock` | `blockhash` is "the parent block of the first block to consider" | the named block's transactions are included |
| `listsinceblock` | condition 1 "Lists all transactions since the genesis block" | also includes unmined transactions |
| `listaccounts` | "all accounts and their balances" | spendable balance at `minconf` only |
| `lockunspent`, `listlockunspent` | `transactioninput-tree`: "The tree to generate transaction for"; `amount`: "The previous output amount" | both ignored on input; on output `tree` is derived from the transaction type and `amount` is omitted when zero |
| `purchaseticket` | result: string "Hash of the resulting ticket" (`methods.go:95`) | array of hash strings, or an object with `dontsigntx` |
| `purchaseticket` | `fromaccount` "(default=\"default\")" | required parameter, no default |
| `purchaseticket` | key `purchaseticket-nosplittransaction` | no such parameter exists |
| `redeemmultisigout` | "constructs a P2PKH paying to the specified address"; `hex`: "Resulting hash." | pays to any decodable address; `hex` is the serialized transaction |
| `redeemmultisigouts` | declared result `RedeemMultiSigOutResult`; `number`: "Number of outpoints found."; synopsis "Takes a hash" | result is `{"results":[…]}`; `number` is an (off-by-one) limit; takes a P2SH address |
| `sendfrom`, `sendmany`, `sendtoaddress` | `comment`/`commentto`: "Unused" | a non-empty comment is rejected with `-1` |
| `sendtomultisig` | result: string "The transaction hash of the sent transaction" (`methods.go:105`); `pubkeys`: "Pubkey to send to." | result is `{txhash, address, redeemscript}`; an array of keys/addresses |
| `sendfromtreasury` | `key`: "Politeia public key" | any address or hex public key whose private key the wallet holds |
| `sendrawtransaction` | "Submits … to the local peer and relays it" | also records wallet-relevant transactions; SPV publishes to peers |
| `setdisapprovepercent` | "will fail on mainnet" | fails with a malformed error (Q-B1) |
| `signrawtransaction` | valid flags: ALL, NONE, SINGLE and the ANYONECANPAY forms | `ssgen` and `ssrtx` are also accepted |
| `signrawtransactionresult-errors` | "Script verification errors (if exists)" | also signing failures; fatal lookup failures are not listed at all |
| `spendoutputs` | "change (if needed) is returned" | panics when no change is needed (Q-B7) |
| `sweepaccount` | "Moves as much value as possible in a transaction from an account." | only constructs an unsigned transaction |
| `ticketinfo` | status ∈ unknown, unmined, immature, mature, live, voted, missed, expired, unspent, revoked | only immature, unspent, voted, missed, expired; `mature` is not a status the wallet defines |
| `ticketinfo` | `blockheight`: "Height of block ticket is mined in" | `-1` when unmined |
| `treasurypolicy` | result0: "Array of all non-abstaining voting policies" | correct for stored entries, but `ticket` filter is ignored |
| `tspendpolicy` | result0: "Array of all non-abstaining policies for known tspends" | one entry per cached tspend including abstaining ones (`policy: ""`) |
| `treasurypolicyresult-policy`, `tspendpolicyresult-policy` | "(abstain, yes, or no)" | `""` appears in list mode |
| `validateaddress` | multisig fields "left unset if the wallet is locked since the redeem script cannot be decrypted" | redeem scripts are readable while locked; fields are always set |
| `validateaddress` | `pubkeyaddr`: "The pubkey for this payment address" | a P2PK **address string** |
| `validateaddress` | `iscompressed`: "created by hashing a compressed public key" | true for every hash-bearing address including P2SH |
| `validateaddress` | `script`: "The class of redeem script for a multisig address" | the address's own script class for every valid address; the redeem script class for wallet P2SH addresses |
| `validateaddress` | fields "(only when isvalid is true)" | `script` is present (`nonstandard`) when `isvalid` is false |
| `validatepredcp0005cf` | result "Whether the cfilters are valid" | `true` or an error; never `false` |
| `verifymessage` | (none) | never returns an error for a bad signature; works without a wallet |
| `walletinfo` | `cointype`: "Not available for watching-only wallets." | omitted for watching-only wallets and on lookup errors |
| `walletinfo` | `birthhash`/`birthheight` unconditional | populated only once the birthday has been resolved to a block (`SetFromTime`/`SetFromHeight` both false) |
| `walletlock` | (source comment) errors for watching-only wallets | never errors |
| `mixaccount` | key `mixaccount-account`: "Account to mix" | the method takes no parameters; mixes the configured change account |
| `getcoinjoinsbyacct` | declared result `map[string]uint32` | `map[string]int`; same JSON |
| `getvotechoices` | "currently configured default vote choices" | per-ticket choices when `tickethash` is given, falling back to defaults |
| `lockaccount` | "Account to lock" | `"*"` locks every unlocked individually-encrypted account |
| `listtransactionsresult-account` | "DEPRECATED -- Unset" (`helpdescs:624`) | set to the receiving account name on credit entries; `""` only on "send" entries or when the lookup fails (`wallet/wallet.go:2456-2468, 2511`) |
| `listtransactionsresult-generated` | "Whether the transaction output is a coinbase output" | also true for treasurybase and treasury-spend outputs (4.3) |
| `listtransactionsresult-walletconflicts` | "Unset" | always present as `[]` |
| `listtransactionsresult-txtype` | "The type of tx (regular tx, stake tx)" | one of `regular`, `ticket`, `vote`, `revocation`; treasury types report `regular` (Q-B38) |
| `getcoinjoinsbyacct` | "coinjoin outputs sum" (`helpdescs:812-815`) | a **count** of matching outputs per account, not a sum of amounts (`wallet/wallet.go:5570-5583`) |
| `walletpassphrase-timeout` | "seconds to wait before the wallet automatically locks. 0 leaves the wallet unlocked indefinitely" | a non-zero timeout is ignored when the wallet is already unlocked without one; negative values are accepted (5.36, Q-B23) |
| `settreasurypolicy-policy`, `settspendpolicy-policy` | "(invalid/abstain, yes, or no)" | the empty string is also accepted and means abstain |
| `redeemmultisigout-tree` | "Tree the transaction is on." | not used to find the output (lookup is by hash and index); only copied into the new input's outpoint and into the `inputs` key handed to signing |
| `sendfrom`, `sendmany`, `sendtoaddress` synopsis | "change output … back to the original account" | change goes to the configured mix change account when mixing is enabled and the source account is the mixed account (5.20 step 1) |
| `sendtomultisig` synopsis | same sentence | change handling is `CreateMultisigTx`'s (spec 10); the handler passes only account 0 |
| `validateaddress` synopsis | "`pubkey` … valid only when the address is controlled by this wallet" | `pubkey` and `pubkeyaddr` are also set for any public-key address not known to the wallet (5.32 step 4) |
| `walletinforesult-daemonconnected` | "connected to the daemon RPC" | in SPV mode: at least one remote peer |
| `help` (source comments, `methods.go:2878-2881, 5434-5437, 4892-4895`) | refer to `HelpNoChainRPC`/`HelpWithChainRPC`, `versionWithChainRPC`/`versionNoChainRPC` handlers and a `chainClient` argument | none of these exist; each is a single handler that inspects the backend type |
| orphan help keys | `purchaseticket-nosplittransaction`, `mixaccount-account`, and whole groups `setbalancetomaintain-*`, `getbalancetomaintain-*`, `setticketmaxprice-*`, `getticketmaxprice-*`, `setgenerate-*` exist in `helpdescs_en_US.go` | no such parameters or methods are registered; the keys never reach generated help output and need not be carried over |

### 10.4 Source markers and stale doc comments in scope

Every `TODO`/`XXX` marker in `methods.go:2798-5863` (there are no
`BUG`, `FIXME` or `Deprecated` markers and no `go` statements other
than the `errgroup` in `signRawTransaction`). Each describes behaviour
upstream considers unfinished; a port reproduces the current
behaviour.

| Line | Marker | Current behaviour it documents |
|---|---|---|
| 2884-2886 | TODO: `help` should use an HTTP POST client to dcrd | dcrd's help is fetched over the wallet's websocket client, so the merged usage text lists dcrd's websocket-only methods although wallet JSON-RPC clients cannot call them |
| 2914-2916 | TODO: other locales | locale is hardcoded to `en_US` |
| 3164-3165 | TODO: `listsinceblock` must begin at the fork point | range starts at the named block's height (5.8, Q-B28) |
| 3200-3202 | TODO: `listtransactions` cannot filter by account | any account other than `"*"` is an error (5.9) |
| 4272 | XXX: do not assume P2PKH | `spendoutputs` rejects every non-P2PKH-ecdsa input with `unsupport address type` |
| 4315 | XXX: should not assume P2PKH | `accountChangeSource.ScriptSize()` returns a constant (Q-B33) |
| 4626 | TODO: use with non-default accounts | `sendtomultisig` always spends from account 0 |
| 4939-4940 | TODO: look inputs up with dcrd anyway | caller-supplied `scriptPubKey` values are trusted and never cross-checked against the chain |

Doc comments in scope that misdescribe the code (not observable; listed
so nobody ports from them): `listReceivedByAccount`/`listReceivedByAddress`
say they take two parameters and that `includeempty` filters
(`methods.go:2996-3001, 3038-3043`; three parameters, both extras
ignored); `sendPairs`, `sendAmountToTreasury`, `sendOutputsFromTreasury`
say "All errors are returned in dcrjson.RPCError format"
(`methods.go:3591, 3627, 3669`; most are raw); `purchaseTicket` says
"using all currently available funds" (`methods.go:3353-3355`);
`redeemMultiSigOut` says "a single P2PKH" (`methods.go:4068`; any
address kind); `sendToMultiSig` says the function fails unless one key
is owned locally (`methods.go:4618-4620`; not checked by the handler);
`walletIsLocked` says "of an account" (`methods.go:5543-5545`; whole
wallet); `walletLock` (5.36); `updateVSPVoteChoices` "the first error
will be returned" (Q-B25); the `sendFromTreasury` comment is truncated
(`methods.go:4740-4741`); the comment above the `gettxout` loop says
`getrawtransaction` (`methods.go:4989-4990`; the call is `gettxout`
with mempool inclusion).

---

## 11. Upstream tests

There are **no handler tests** for anything in this scope. Coverage that
touches it:

| File | What it pins | Port? |
|---|---|---|
| `internal/rpc/jsonrpc/rpchelp_test.go` | every non-`noHelp` handler has help text and a usage line, and the generated help file is current | via spec 16 |
| `internal/rpc/jsonrpc/rpcserver_test.go` | HTTP throttling only | spec 16 |
| `rpc/jsonrpc/types/methods_test.go` | command construction, JSON-RPC 1.0 marshalling and unmarshalling for: `listaccounts`, `listlockunspent`, `listreceivedbyaccount`, `listreceivedbyaddress`, `listsinceblock`, `listtransactions`, `listunspent`, `lockunspent`, `sendfrom`, `sendfromtreasury`, `sendmany`, `sendtoaddress`, `sendtotreasury`, `settreasurypolicy`, `settspendpolicy`, `settxfee`, `signmessage`, `signrawtransaction`, `sweepaccount`, `walletlock`, `walletpassphrase`, `walletpassphrasechange`, `walletpubpassphrasechange` (each with optional-parameter variants) | port the vectors verbatim (spec 16) |
| `rpc/jsonrpc/types/walletsvrwscmds_test.go` | same for `listaddresstransactions`, `listalltransactions`, `walletislocked` | port |
| `wallet/*_test.go` | none of the wallet functions the handlers call for listing, signing, policy or treasury are covered by focused tests | – |
| `wallet/wallet_test.go` (`TestCoinbaseMatured`, `TestTicketMatured`, `TestTicketExpired`) | the maturity predicates behind `listunspent` filtering (5.12 step 3) and the `generate`/`immature` split | port with the wallet-core spec |
| `wallet/txauthor/author_test.go` (`TestNewUnsignedTransaction`) | fee/change arithmetic of the author used by `spendoutputs` and `sweepaccount`; does not exercise `RandomizeChangePosition` with no change (Q-B7) | port with spec 10 |
| `wallet/txsizes/size_test.go` (`TestEstimateSerializeSize`) | the size estimator used for the `sendfromtreasury`, `redeemmultisigout` and `spendoutputs` fees | port with spec 10 |
| `wallet/txrules/poolfees_test.go` | stake pool fee only; `TxPaysHighFees`, `IsDustOutput` and `FeeForSerializeSize` as used here are untested | – |

Exact test function names in the two in-package files:
`TestRPCMethodHelpGeneration` and `TestRPCMethodUsageGeneration`
(`rpchelp_test.go:39, 78`), `TestThrottle` (`rpcserver_test.go:15`);
in the types package `TestWalletSvrCmds` (`methods_test.go:23`) and
`TestWalletSvrWsCmds` (`walletsvrwscmds_test.go:22`). The types tests
have no vectors for: `getvotechoices`, `setvotechoice`,
`getwalletfee`, `purchaseticket`, `processunmanagedticket`,
`treasurypolicy`, `tspendpolicy`, `setdisapprovepercent`,
`redeemmultisigout(s)`, `rescanwallet`, `spendoutputs`, `ticketinfo`,
`sendtomultisig`, `sendrawtransaction`, `signrawtransactions`,
`validateaddress`, `validatepredcp0005cf`, `verifymessage`, `version`,
`walletinfo`, `mixoutput`, `mixaccount`, the account-passphrase
methods, `lockaccount`, `accountunlocked`, `getcoinjoinsbyacct`,
`help` (none of these method-name strings occurs in either test
file).

Untested upstream and therefore requiring new vectors from a Go oracle
shim (a small Go program linking dcrwallet at the pin, driving a
simnet/regtest-style wallet database and dumping request/response
pairs):

1. Result bytes for every listing method against a fixed wallet
   database (mined and unmined transactions, coinbase, tickets, votes,
   revocations, multisig, imported keys/scripts/xpubs), including the
   `null`/`[]`/`omitempty` cases and float formatting edge values
   (1 atom, 21e6 DCR, values needing 17 significant digits).
2. Every error exit in section 8 (code and message).
3. `signmessage` vectors (fixed key and message; signatures are
   deterministic RFC6979) and `verifymessage` accept/reject cases.
4. `validateaddress` for each address kind × wallet-knowledge
   combination.
5. `sendfromtreasury` transaction layout with the PRNG stubbed, and
   `sendtotreasury` output.
6. `signrawtransaction` matrix: flags, supplied inputs with matching
   and mismatching `tree`, `privkeys` nil / empty / non-empty, multisig
   partial signing, vote transaction with `ssgen`.
7. `redeemmultisigouts` with `number` smaller than the output count
   (zero-valued tail).
8. The panics of Q-B6 and Q-B7 (to confirm reachability before
   deciding how to diverge).
9. `listtransactions` paging against transactions that produce no
   entries.
10. Trailing-garbage acceptance in transaction hex (Q-R10).
11. `signrawtransaction` with each bad-WIF class (over-long, bad
    checksum, wrong network, unknown suite byte; 5.29 step 7, Q-B36).
12. `sendfromtreasury` with an empty `amounts` object (5.24).
13. `listtransactions` entries for a treasury add, a treasury spend
    and a treasurybase credit (Q-B38).

Vectors whose order depends on map iteration (Q-R2) must be compared as
multisets.

---

## 12. Rust porting notes

### 12.1 Suggested boundary

One crate for the JSON-RPC method layer (working name
`purse-jsonrpc`), with the transport from spec 16 in a sibling module
or crate. Within it, split handlers by domain rather than by the Go
file's alphabetical order:

| Module | Methods |
|---|---|
| `voting` | `getvotechoices`, `setvotechoice`, `treasurypolicy`, `settreasurypolicy`, `tspendpolicy`, `settspendpolicy`, `setdisapprovepercent`, VSP push helper |
| `listing` | `listaccounts`, `listreceivedby*`, `listsinceblock`, `listtransactions`, `listaddresstransactions`, `listalltransactions`, `listunspent`, `listlockunspent`, `lockunspent`, `ticketinfo`, `getcoinjoinsbyacct` |
| `send` | `sendfrom`, `sendmany`, `sendtoaddress`, `sendtomultisig`, `sendtotreasury`, `sendfromtreasury`, `sendrawtransaction`, `spendoutputs`, `sweepaccount` |
| `sign` | `signmessage`, `verifymessage`, `signrawtransaction`, `signrawtransactions`, `redeemmultisigout(s)` |
| `tickets` | `purchaseticket`, `processunmanagedticket` |
| `admin` | `walletinfo`, `walletislocked`, `walletlock`, `walletpassphrase*`, account passphrase methods, `settxfee`, `getwalletfee`, `rescanwallet`, `validateaddress`, `validatepredcp0005cf`, `version`, `help`, `mixoutput`, `mixaccount` |
| `util` | `decode_address`, `decode_stake_address`, `parse_outpoint`, `decode_hex_str`, amount conversion, the `ListTransactionsResult` builder (which upstream keeps in the wallet package with a TODO to move it) |

Handlers should take a trait object for the wallet and for the network
backend so the dcrd/SPV distinctions in 5.3, 5.29, 5.34, 5.35 are
explicit enum matches rather than downcasts.

### 12.2 Reuse (crates inspected)

| Need | Candidate | Notes |
|---|---|---|
| Positional parameter decoding, usage/help generation, Go-compatible JSON encoding and float formatting | `dcroxide:crates/dcroxide-dcrjson` (`cmdparse.rs`, `gojson.rs`, `help.rs`, `usage.rs`) | `gojson::format_float_json` implements the `encoding/json` float rule needed by 4.1.1 |
| dcrd result/param types reused by the wallet (`TransactionInput`, `GetTxOutResult`, `VersionResult`) | `dcroxide:crates/dcroxide-rpctypes` (`chainsvrcmds.rs`, `chainsvrresults.rs`) | type descriptors for `gojson` |
| Hash parsing with dcrd's leniency | `dcroxide:crates/dcroxide-chainhash/src/lib.rs:164-200` (`FromStr`) | documented as reproducing `chainhash.Decode` |
| Address decoding, script classification, required-sigs | `dcroxide:crates/dcroxide-txscript` (`stdaddr.rs:762`, `stdscript.rs:797-1157`) | |
| Tspend signature script | `dcroxide:crates/dcroxide-txscript/src/sign.rs:665` (`tspend_signature_script`) | |
| `CheckTSpend` | `dcroxide:crates/dcroxide-stake/src/treasury.rs:148` | |
| `CalcTSpendExpiry` | `dcroxide:crates/dcroxide-standalone/src/treasury.rs:13` | |
| Compact signatures | `dcroxide:crates/dcroxide-dcrec/src/secp256k1/ecdsa.rs:435, 633` (`recover_compact`, `sign_compact`) | |
| CSPRNG shuffle / read | `dcroxide:crates/dcroxide-crypto/src/rand.rs:447, 483` | |
| Atom → DCR float | `dcroxide:crates/dcroxide-rpc/src/txresults.rs:24` (`to_coin`) | trivial; `dcr-rs:src/amount.rs` only offers string formatting |
| WIF decoding (`dcrutil.DecodeWIF`, three signature suites) | not found in dcroxide or dcr-rs | must be written |
| `dcrutil.NewAmount` rounding | not found | must be written, with the overflow decision of Q-R1 |

### 12.3 Hazards

1. **Byte-exact JSON.** Use the Go-compatible encoder for every result;
   `serde_json` differs in float formatting, HTML escaping and map
   ordering.
2. **`Option<Vec<_>>` discipline.** Model each result field's
   nil/empty/omit state explicitly (4.3); a blanket
   `skip_serializing_if = "Vec::is_empty"` or its absence will be wrong
   somewhere.
3. **Raw error texts** depend on the wallet error formatter (operation
   names, kind descriptions, the `":: "` separator). The handler layer
   must not re-wrap them.
4. **Per-method error-code asymmetries** (Q-B12, Q-B13) are easy to
   "fix" accidentally by sharing helpers.
5. **Nested database access** in `ticketinfo` and `spendoutputs` (7):
   with a single-writer/multi-reader engine these patterns can
   self-deadlock if the port upgrades or re-enters a transaction. Fetch
   what is needed inside one transaction instead; the observable result
   is unchanged.
6. **Panics** (Q-B6, Q-B7): Rust slice indexing would also panic.
   Decide deliberately (13).
7. **Map-order outputs** (Q-R2): pick a deterministic order; any order
   is within upstream's observable behaviour.
8. **Integer truncation** (Q-R3): decode optional integers as `i64` and
   truncate with `as i32` / `as i8` / `as u32` to match; do not reject
   out-of-range values.
9. **`walletpassphrase` timeout arithmetic** (Q-B23): compute
   `timeout.wrapping_mul(1_000_000_000)` nanoseconds and treat
   non-positive results as "fire immediately" to match, or diverge
   deliberately.
10. **Background goroutines** in `signrawtransaction`: use structured
    concurrency (join set scoped to the request) — the leak in Q-B15
    is not observable to clients.

### 12.4 Size estimate

**L.** About 3,070 Go lines, 55 methods. Individually the handlers are
small and mechanical once the wallet API exists (most are 20-60 lines),
but the result-shape and error-string fidelity, the two transaction
builders (`sendfromtreasury`, `spendoutputs`), `signrawtransaction`, and
`validateaddress` each need careful vector-driven work, and none of it
has upstream tests, so the oracle harness of section 11 is part of the
cost. Expect roughly 4,000-5,000 lines of Rust including tests, after
the wallet-core, tx-construction, staking and transport specs are
implemented.

---

## 13. Open questions

1. **Q-B7 reachability.** Confirm by oracle that `spendoutputs` panics
   when no change output results. If confirmed, upstream effectively
   never returns success in that case, so any non-crashing behaviour is
   compatible.
2. **Q-B6 reachability.** Confirm the out-of-range index panics in
   `FetchOutput` and `SignTransaction`.
3. **Q-B26.** Can a wallet contain a ticket-shaped transaction without
   a ticket record (for example a ticket where the wallet holds only a
   commitment address)? Needs spec 04 / an oracle database.
4. **Q-R10 / 4.1.3.** Confirm that trailing data after a serialized
   transaction is ignored by `hex.NewDecoder` + `Deserialize`, and the
   exact error strings for truncated input and odd-length input.
5. **Exact error text for `wire` deserialization failures** returned
   with `-22` (depends on `wire@v1.8.0` message error formatting; not
   enumerated here).
6. **`walletpubpassphrasechange` raw error text** for a wrong old
   passphrase (address-manager wording; spec 02).
7. **`PurchaseTickets` with `dontsigntx`**: whether `SplitTx` can be
   nil (which would make step 12 of 5.14 dereference nil) — spec 11.
8. **`sendmany` with an empty `amounts` object**: behaviour of
   `SendOutputs` with zero outputs — spec 10.
9. **SPV `PublishTransactions` failure modes** as seen by
   `sendrawtransaction`, `sendfromtreasury`, `signrawtransactions`
   (no peers, rejection) — spec 14.
10. **Q-B36 error text.** The exact message for a WIF with an unknown
    suite byte (8.2) was composed from the stdaddr and secp256k1
    sources, not observed; confirm by oracle.
11. **`help` mutex across the dcrd call (Q-B41).** Reproduce the
    serialization (not observable except as latency) or release the
    lock before the round trip? Recommended: release it.

### Decisions needed

- **D1.** Reproduce or fix the process-terminating panics (Q-B6, Q-B7)?
  Recommended: return an error (`-8` for a bad index; for
  `spendoutputs` without change, skip the randomisation and succeed),
  and record the divergence.
- **D2.** Reproduce the malformed mainnet error of
  `setdisapprovepercent` (Q-B1) byte-for-byte? Recommended: yes; it is
  cheap and observable.
- **D3.** Reproduce `walletpubpassphrasechange`'s raw error text
  (Q-B8)? Recommended: yes (code `-14` is what clients match on, and
  the message must come from the wallet layer anyway).
- **D4.** Float-to-int overflow in `NewAmount` (Q-R1): follow amd64
  (most deployments; out-of-range becomes `i64::MIN`, i.e. rejected as
  negative) or Rust's saturation? Recommended: emulate amd64.
- **D5.** Deterministic ordering for map-ordered outputs (Q-R2):
  recommended sorted by address / key / agenda ID for results, and
  input order as received for transaction outputs is **not** available
  for `sendmany` (JSON object) — choose sorted-by-address or a CSPRNG
  shuffle; shuffle is closer to upstream's privacy properties.
- **D6.** `redeemmultisigouts` off-by-one and zero-valued tail (Q-B5):
  reproduce exactly (recommended, it is observable and harmless) or
  fix.
- **D7.** Whether to keep `signrawtransactions` publishing directly to
  the backend without recording (Q-B9) and `sendfromtreasury` likewise
  (Q-B18). Recommended: reproduce; recording would change wallet
  balances seen by other calls.
- **D8.** `listunspent` `null` on empty (Q-B2): reproduce
  (recommended; clients written against dcrwallet already handle it,
  and switching to `[]` is a byte-level change).

