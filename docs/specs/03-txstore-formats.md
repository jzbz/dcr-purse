# 03 — Transaction store: bucket layout and byte formats

## 1. Header

**Scope** (dcrwallet `wallet/udb`, 3,125 lines of Go):

| File | Lines | Contents |
| --- | --- | --- |
| `wallet/udb/txdb.go` | 2,438 | Bucket names, root keys, every key/value codec of the `wtxmgr` namespace, iterators, `createStore`, legacy `upgradeTxDB` |
| `wallet/udb/txunmined.go` | 410 | Unmined ("mempool") transaction insert, removal, pruning, listing |
| `wallet/udb/kahnsort.go` | 116 | Topological (dependency) sort of unmined transactions |
| `wallet/udb/cfilters.go` | 67 | Version 2 compact filter read and bulk import |
| `wallet/udb/agendas.go` | 94 | Default and per-ticket agenda (vote choice) preference buckets |

Read for context, not specified here (they belong to sibling specs): `wallet/udb/txmined.go`
(mined-transaction logic, header helpers, root-key users), `wallet/udb/txquery.go`,
`wallet/udb/upgrades.go`, `wallet/udb/initialize.go`, `wallet/udb/migration.go`,
`wallet/udb/metadata.go`, `wallet/udb/vsp.go`, `wallet/udb/treasury.go`,
`wallet/walletdb/interface.go`, `wallet/internal/bdb/db.go`, `errors/errors.go`.

Sibling specs referred to below by role: "walletdb/bbolt spec" = `01-walletdb-bbolt.md`,
"address manager spec" = `02-address-manager.md`, "mined-transaction spec" =
`04-txstore-behavior.md`, "upgrades spec" (also VSP and treasury buckets) =
`05-db-migrations-aux-buckets.md`.

**Parity pin**

| Component | Version |
| --- | --- |
| dcrwallet | master `54b85b33` (2.2.0-pre, module `decred.org/dcrwallet/v5`) |
| `go.etcd.io/bbolt` | v1.3.12 |
| `github.com/decred/dcrd/wire` | v1.8.0 |
| `github.com/decred/dcrd/blockchain/stake/v5` | v5.0.3 |
| `github.com/decred/dcrd/txscript/v4` | v4.1.2 |
| `github.com/decred/dcrd/gcs/v4` | v4.1.1 |
| `github.com/decred/dcrd/chaincfg/chainhash` | v1.0.5 |
| `github.com/decred/dcrd/chaincfg/v3` | v3.3.0 |
| `github.com/decred/dcrd/dcrutil/v4` | v4.0.3 |
| `github.com/decred/dcrd/crypto/ripemd160` | v1.0.2 |

**Summary**

The transaction store ("wtxmgr") is one top-level bbolt bucket named `wtxmgr` inside `wallet.db`.
At database version 26 it holds 6 root key/value pairs (a 7th, `vers`, exists only in
pre-unified databases) and 18 nested buckets (a 19th, `sc`, existed before database version 14). `txdb.go` is a flat library of codec helpers: every persisted integer is
**big-endian**, every key is a fixed-width concatenation of hashes and integers, and values are
fixed-layout byte strings with a small number of flag bits. Nothing in the store is
self-describing; there are no length prefixes, tags or checksums apart from what is listed here.
`txunmined.go` implements the unmined transaction set on top of four of those buckets (`m`, `mc`,
`mi`, `up`) plus side effects on tickets, ticket commitments and multisig outputs.
`kahnsort.go` orders unmined transactions parents-first for rebroadcast. `cfilters.go` reads and
bulk-imports version 2 committed filters. `agendas.go` stores vote choices in two *other*
top-level buckets (`agendaprefs`, `ticketsagendaprefs`). A Rust replacement must read and write
all of these byte-for-byte, including several legacy value shapes that only exist in wallets
created by old software.

---

## 2. Role in the system

`wallet/udb` ("unified database") is the persistence layer of the wallet. It exposes two manager
objects, the address manager (`*Manager`, namespace `waddrmgr`) and the transaction store
(`*Store`, namespace `wtxmgr`), both living in one bbolt file and both operated through
caller-supplied `walletdb` transactions.

The files in scope sit at the bottom and the side of the transaction store:

- **`txdb.go`** is called by everything else in `wallet/udb` that touches the `wtxmgr` namespace:
  `txmined.go` (insert mined tx, add credit, rollback, stake (in)validation, balances, UTXO
  selection), `txquery.go` (details, ranges, spender lookup), `txunmined.go`, `cfilters.go`,
  `upgrades.go` (database upgrades 6, 8, 9, 10, 11, 12, 14, 15, 18, 26 use these helpers and
  bucket names) and `initialize.go` (`createStore`). Outside the package it calls only `walletdb`
  bucket/cursor primitives, `wire` (transaction and outpoint types, `MsgTx.Serialize`/`Deserialize`),
  `stake.DetermineTxType`, the `txscript.OP_TGEN` constant, `chaincfg.Params` (genesis block and
  hash), `chainhash.NewHash`, `time.Now` and `runtime/debug.PrintStack`. Inside the package it
  uses `extractBlockHeaderUnixTime` / `extractBlockHeaderVoteBits` (`wallet/udb/txmined.go:581,563`)
  and the constants `hasExpiryVersion` / `hasExpiryFixedVersion` (`wallet/udb/upgrades.go:78,85`).
- **`txunmined.go`** is called by `wallet` (`wallet/chainntfns.go:255` `PruneUnmined`,
  `wallet/chainntfns.go:451` and `wallet/createtx.go:258` `InsertMemPoolTx`,
  `wallet/wallet.go:4366` `UnminedTxs`, `wallet/wallet.go:5084` `RemoveUnconfirmed` via
  `AbandonTransaction`, `wallet/wallet.go:5639` `SetPublished`, `wallet/notifications.go:241,333`
  `UnminedTxHashes`) and by `txmined.go` (`removeDoubleSpends` from `InsertMinedTx`,
  `RemoveUnconfirmed` from `Rollback`). It calls `txdb.go` helpers, `Store.MainChainTip` and
  `Store.replaceTicketCommitmentUnminedSpent` (both `txmined.go`), and `stake` type detection.
- **`kahnsort.go`** is called only by `Store.UnminedTxs`.
- **`cfilters.go`** is called directly only from package `wallet`: `Store.CFilterV2` by
  `Wallet.CFilterV2` (`wallet/wallet.go:838`), `Wallet.RangeCFiltersV2` (`wallet/wallet.go:867`),
  the missing-cfilter fetch loop (`wallet/wallet.go:1334,1396`) and the chain-switch code
  (`wallet/chainntfns.go:134`); `Store.ImportCFiltersV2` by `Wallet.ImportCFiltersV2`
  (`wallet/wallet.go:5281`). The SPV backend (`spv/backend.go:529`), the gRPC server
  (`internal/rpc/rpcserver/server.go:2204`) and the `getcfilterv2` / `importcfiltersv2` JSON-RPC
  handlers (`internal/rpc/jsonrpc/methods.go:1967,1998`) reach it only through those `Wallet`
  methods. It calls `gcs.FromBytesV2`, `fetchRawBlockHeader` and `extractBlockHeaderMerkleRoot`.
- **`agendas.go`** is called by `wallet/wallet.go` (`readDBVoteBits` at line 330,
  `readDBTicketVoteBits` at 361, `AgendaChoices` at 440–442 and `SetAgendaChoices` at 530–532)
  and reads/writes top-level buckets that `upgrades.go` creates at database versions 3 and 16.

The store has no state of its own beyond the database: `Store` (declared in `txmined.go:175`)
only carries `chainParams`, an account lookup function and a `*Manager` pointer.

---

## 3. Public surface inventory

Conventions used in the tables: `ns` is the `wtxmgr` namespace bucket; `k`/`v` are raw key and
value byte slices; "IO" etc. are error kinds from the wallet `errors` package (section 8).
Layouts referenced here are defined byte-exactly in section 4.

### 3.1 Exported identifiers

| Identifier | Signature | Contract |
| --- | --- | --- |
| `(*Store).InsertMemPoolTx` | `(dbtx walletdb.ReadWriteTx, rec *TxRecord) error` | Insert an unmined transaction, its `mi` spend markers and (for tickets) its `tix` row. `Exist` if already mined, no-op if already unmined, `DoubleSpend` on conflict with another unmined tx (with a vote/revocation exception). Algorithm 5.8. `wallet/udb/txunmined.go:23` |
| `(*Store).SetPublished` | `(dbtx walletdb.ReadWriteTx, txHash *chainhash.Hash, published bool) error` | `published=true` deletes the `up` row, `false` writes `up[txHash]=0x01`. Does not check that the transaction exists. `wallet/udb/txunmined.go:137` |
| `(*Store).RemoveUnconfirmed` | `(ns walletdb.ReadWriteBucket, tx *wire.MsgTx, txHash *chainhash.Hash) error` | Remove an unmined transaction and, recursively, every unmined transaction that spends any of its outputs. Algorithm 5.11. `wallet/udb/txunmined.go:181` |
| `(*Store).UnminedTxs` | `(dbtx walletdb.ReadTx) ([]*TxRecord, error)` | All unmined transactions (published and unpublished) in a dependency-respecting order. Order among independent transactions is unspecified. `wallet/udb/txunmined.go:285` |
| `(*Store).UnminedTxHashes` | `(ns walletdb.ReadBucket) ([]*chainhash.Hash, error)` | Hashes of all `m` rows in ascending key order. Returns a nil slice when empty. `wallet/udb/txunmined.go:320` |
| `(*Store).PruneUnmined` | `(dbtx walletdb.ReadWriteTx, sdiff dcrutil.Amount) ([]*chainhash.Hash, error)` | Remove expired transactions, tickets whose price is not `sdiff` (when `sdiff != 0`) and votes not on the tip block. Returns the hashes it selected (not their removed descendants). Algorithm 5.13. `wallet/udb/txunmined.go:344` |
| `(*Store).CFilterV2` | `(dbtx walletdb.ReadTx, blockHash *chainhash.Hash) ([16]byte, *gcs.FilterV2, error)` | Load the stored filter key and filter for a block. `NotExist` when no row. `wallet/udb/cfilters.go:17` |
| `(*Store).ImportCFiltersV2` | `(dbtx walletdb.ReadWriteTx, startHeight int32, filterData [][]byte) error` | Overwrite the stored filters of consecutive main-chain blocks starting at `startHeight`. Algorithm 5.16. `wallet/udb/cfilters.go:34` |
| `SetDefaultAgendaPreference` | `(dbtx walletdb.ReadWriteTx, version uint32, agendaID, choiceID string) error` | `agendaprefs[be32(version)‖agendaID] = choiceID`. Any error is wrapped as `IO`. `wallet/udb/agendas.go:64` |
| `SetTicketAgendaPreference` | `(dbtx walletdb.ReadWriteTx, txHash *chainhash.Hash, version uint32, agendaID, choiceID string) error` | Creates `ticketsagendaprefs/<txHash>` if missing, then writes the same k/v inside it. Errors wrapped as `IO`. `wallet/udb/agendas.go:74` |
| `DefaultAgendaPreference` | `(dbtx walletdb.ReadTx, version uint32, agendaID string) string` | Stored choice ID or `""`. `wallet/udb/agendas.go:85` |
| `TicketAgendaPreference` | `(dbtx walletdb.ReadTx, ticketHash *chainhash.Hash, version uint32, agendaID string) string` | Stored per-ticket choice ID or `""` (also `""` when the ticket has no nested bucket). `wallet/udb/agendas.go:92` |

No exported types, constants or variables are declared in the five files in scope. Types that the
helpers produce or consume (`Block`, `BlockMeta`, `blockRecord`, `TxRecord`, `credit`,
`indexedIncidence`, `CreditRecord`, `DebitRecord`, `MultisigOut`, `RawBlockHeader`, `Store`) are
declared in `txmined.go` / `txquery.go`; their field lists are repeated in section 4.1 where a
codec depends on them.

### 3.2 Unexported constants and variables (`txdb.go`, `agendas.go`)

| Identifier | Value | Source |
| --- | --- | --- |
| `accountExistsMask` | `uint8(0x80)` | `wallet/udb/txdb.go:63` |
| `scriptType` (uint8 enum) | `scriptTypeNonexisting=0`, `scriptTypeUnspecified=1`, `scriptTypeP2PKH=2`, `scriptTypeP2PK=3`, `scriptTypeP2PKHAlt=4`, `scriptTypeP2PKAlt=5`, `scriptTypeP2SH=6`, `scriptTypeSP2PKH=7`, `scriptTypeSP2SH=8` | `wallet/udb/txdb.go:72-112` |
| `scriptLocNotStored` | `0` | `wallet/udb/txdb.go:118` |
| `byteOrder` | `binary.BigEndian` | `wallet/udb/txdb.go:123` |
| `bucketBlocks` … `bucketTicketCommitmentsUsp` | 19 nested-bucket names, table 4.4 | `wallet/udb/txdb.go:133-151` |
| `rootCreateDate` … `rootBirthState` | 8 root keys, table 4.3 | `wallet/udb/txdb.go:156-163` |
| `creditKeySize` | `72` | `wallet/udb/txdb.go:728` |
| `creditValueSize` | `94` | `wallet/udb/txdb.go:731` |
| `unconfCreditKeySize` | `36` | `wallet/udb/txdb.go:1450` |
| `unconfValueSizeLegacy` | `9` | `wallet/udb/txdb.go:1454` |
| `unconfValueSize` | `22` | `wallet/udb/txdb.go:1458` |
| `agendaPreferences` | zero-size value of `agendaPreferencesTy`; method namespace only | `wallet/udb/agendas.go:13-16` |
| `defaultAgendaPrefsBucketKey` | `"agendaprefs"` | `wallet/udb/agendas.go:19` |
| `ticketsAgendaPrefsBucketKey` | `"ticketsagendaprefs"` | `wallet/udb/agendas.go:20` |

The two ranges above, spelled out (Go variable → bytes), so that a search for any identifier
lands here:

| Go variable | Bytes | Source |
| --- | --- | --- |
| `bucketBlocks` | `b` | `wallet/udb/txdb.go:133` |
| `bucketHeaders` | `h` | `wallet/udb/txdb.go:134` |
| `bucketTxRecords` | `t` | `wallet/udb/txdb.go:135` |
| `bucketCredits` | `c` | `wallet/udb/txdb.go:136` |
| `bucketUnspent` | `u` | `wallet/udb/txdb.go:137` |
| `bucketDebits` | `d` | `wallet/udb/txdb.go:138` |
| `bucketUnmined` | `m` | `wallet/udb/txdb.go:139` |
| `bucketUnpublished` | `up` | `wallet/udb/txdb.go:140` |
| `bucketUnminedCredits` | `mc` | `wallet/udb/txdb.go:141` |
| `bucketUnminedInputs` | `mi` | `wallet/udb/txdb.go:142` |
| `bucketTickets` | `tix` | `wallet/udb/txdb.go:143` |
| `bucketScripts` | `sc` (source comment: "removed in db v14") | `wallet/udb/txdb.go:144` |
| `bucketMultisig` | `ms` | `wallet/udb/txdb.go:145` |
| `bucketMultisigUsp` | `mu` | `wallet/udb/txdb.go:146` |
| `bucketStakeInvalidatedCredits` | `ic` | `wallet/udb/txdb.go:147` |
| `bucketStakeInvalidatedDebits` | `id` | `wallet/udb/txdb.go:148` |
| `bucketCFilters` | `cf` | `wallet/udb/txdb.go:149` |
| `bucketTicketCommitments` | `cmt` | `wallet/udb/txdb.go:150` |
| `bucketTicketCommitmentsUsp` | `cmu` | `wallet/udb/txdb.go:151` |
| `rootCreateDate` | `date` | `wallet/udb/txdb.go:156` |
| `rootVersion` | `vers` | `wallet/udb/txdb.go:157` |
| `rootMinedBalance` | `bal` | `wallet/udb/txdb.go:158` |
| `rootTipBlock` | `tip` | `wallet/udb/txdb.go:159` |
| `rootHaveCFilters` | `havecfilters` | `wallet/udb/txdb.go:160` |
| `rootLastTxsBlock` | `lasttxsblock` | `wallet/udb/txdb.go:161` |
| `rootVSPHostIndex` | `vsphostindex` (not a `wtxmgr` key, quirk Q1) | `wallet/udb/txdb.go:162` |
| `rootBirthState` | `birthstate` | `wallet/udb/txdb.go:163` |

Also declared: the blank compile-time assertion `var _ [32]byte = chainhash.Hash{}`
(`wallet/udb/txdb.go:129`), which fails the build if the hash width ever changes. The nine
`scriptType*` constants are declared with a bare `iota` (`wallet/udb/txdb.go:74-112`), so they
are untyped integer constants, not values of type `scriptType`; they convert implicitly where a
`scriptType` is expected.

### 3.3 Unexported helpers in `txdb.go`

The file's header comment (`wallet/udb/txdb.go:25-54`) fixes the helper naming scheme
`Op[Raw]Type[Field]`. It is not enforced by anything, but the names below follow it and a port
that keeps the names keeps the cross-references of the sibling specs valid:

| `Op` | Meaning |
| --- | --- |
| `key` | Build a database key from typed data |
| `value` | Build a database value from typed data |
| `put` | Insert or replace a row |
| `fetch` | Read and return a (usually length-checked) field |
| `read` | Decode into an out parameter |
| `exists` | Return the raw value, nil when the row is absent |
| `delete` | Remove a row |
| `extract` | Unchecked slice of a key or value (panics on short input) |

`Raw` marks helpers that take or return raw key/value bytes instead of typed data. Reserved
variable names in the Go: `ns` namespace bucket, `b` bucket, `k`/`v` key/value, `c` cursor,
`ck`/`cv` current cursor key/value.

Every function in the file is listed. A bare `:N` at the end of a contract is a line number in
`wallet/udb/txdb.go`. "panics" means the Go code indexes or slices without a
length check and a short/nil input raises a runtime panic (section 10 lists the ones reachable
from a corrupt database).

**Root values and canonical outpoints**

| Function | Contract |
| --- | --- |
| `fetchMinedBalance(ns) (Amount, error)` | Reads `bal`; value must be exactly 8 bytes else `IO` `"mined balance len %d"`. `:171` |
| `putMinedBalance(ns, amt) error` | Writes 8-byte BE amount; put error wrapped `IO`. `:179` |
| `canonicalOutPoint(txHash, index) []byte` | 36-byte key `hash‖be32(index)`. `:200` |
| `readCanonicalOutPoint(k, *wire.OutPoint) error` | Requires `len(k) >= 36` else `IO` `"outpoint len %d"`. Sets `Hash`, `Index`; does not touch `Tree`. `:207` |

**Block records (`b`)**

| Function | Contract |
| --- | --- |
| `keyBlockRecord(height int32) []byte` | `be32(uint32(height))`. `:230` |
| `valueBlockRecordEmptyFromHeader(blockHash, header []byte) []byte` | 47-byte record with zero transactions; time and vote bits extracted from the serialized header. `:236` |
| `valueBlockRecordStakeValidated(v) []byte` | Copy of `v` with byte 42 = 0. Panics if `len(v) < 43`. `:247` |
| `valueBlockRecordStakeInvalidated(v) []byte` | Copy of `v` with byte 42 = 1. Panics if `len(v) < 43`. `:256` |
| `appendRawBlockRecord(v, txHash) ([]byte, error)` | New slice = `v‖txHash` with the count at `[43:47]` incremented. `len(v) < 47` → `IO` `"block record len %d"`. Never mutates `v`. `:266` |
| `putRawBlockRecord(ns, k, v) error` | Put into `b`; error wrapped `IO`. `:276` |
| `fetchBlockTime(ns, height) (time.Time, error)` | Seconds field of the record; missing or `< 47` bytes → `IO` `"block record len %d"`. `:284` |
| `fetchBlockRecord(ns, height) (*blockRecord, error)` | Get + `readRawBlockRecord`. Always returns a non-nil pointer, even with an error. `:293` |
| `existsBlockRecord(ns, height) (k, v []byte)` | Returns the key always and the raw value (nil if absent). `:302` |
| `readRawBlockRecord(k, v, *blockRecord) error` | Decodes height (from key), hash, time, vote bits and the tx hash list. Errors: `IO` `"block key len %d"`, `IO` `"block record len %d"` (prints **`len(k)`**, see quirk Q4), `IO` `"%d tx block record len %d"`. Does not expose the invalidation byte. `:308` |
| `extractRawBlockRecordHash(v) []byte` | `v[:32]`; panics on short input. `:337` |
| `extractRawBlockRecordStakeInvalid(v) bool` | `v[42] != 0`; panics on short input. `:341` |
| `makeReadBlockIterator(ns, height) blockIterator` | Forward iterator starting at the first record with height ≥ `height`. `:354` |
| `makeReverseBlockIterator(ns ReadWriteBucket) blockIterator` | Reverse iterator starting at the last record. `:363` |
| `(*blockIterator).next() / prev() / close() / reposition(height)` | See algorithm 5.3. `:370-453` |
| `deleteBlockRecord(ns, height) error` | Delete from `b`; error returned **unwrapped**. `:455` |

**Block headers (`h`)**

| Function | Contract |
| --- | --- |
| `keyBlockHeader(blockHash) []byte` | The 32 hash bytes. `:463` |
| `putRawBlockHeader(ns, k, v) error` | Put into `h`; error wrapped `IO`. `:465` |
| `fetchRawBlockHeader(ns, k) ([]byte, error)` | Returns a copy; missing → `NotExist` `"block header"`. `:473` |
| `existsBlockHeader(ns, k) []byte` | Raw value or nil (no copy). `:483` |

**Transaction records (`t`)**

| Function | Contract |
| --- | --- |
| `keyTxRecord(txHash, *Block) []byte` | 68-byte key. `:502` |
| `valueTxRecord(*TxRecord) ([]byte, error)` | `be64(Received.Unix())‖serialized tx`. Uses `rec.SerializedTx` verbatim when non-nil, otherwise `rec.MsgTx.Serialize`; serialization failure → `Invalid`. `:510` |
| `putTxRecord(ns, rec, block) error` | key + value + put into `t`; put error `IO`. `:528` |
| `putRawTxRecord(ns, k, v) error` | Put into `t`; `IO`. `:541` |
| `readRawTxRecordMsgTx(v, *wire.MsgTx) error` | `len(v) < 8` → `IO` `"tx record len %d"`; deserialize error → `IO`. `:549` |
| `readRawTxRecord(txHash, v, *TxRecord) error` | As above, also sets `Hash`, `Received` (whole seconds) and `TxType = stake.DetermineTxType`. Does not set `SerializedTx` or `Unpublished`. `:561` |
| `readRawTxRecordHash(k, *Hash) error` | `len(k) < 68` → prints a goroutine stack trace to stderr (`debug.PrintStack`) and returns `IO` `"tx record key len %d"`. `:578` |
| `readRawTxRecordBlockHeight(k, *int32) error` | Same check and stack dump. `:587` |
| `readRawTxRecordBlock(k, *Block) error` | Same check and stack dump. `:596` |
| `fetchRawTxRecordPkScript(k, v, index, scrLoc, scrLen) ([]byte, error)` | Output script by stored offset, or by full deserialization when `scrLoc == 0`. Algorithm 5.7. `:606` |
| `fetchRawTxRecordReceived(v) time.Time` | `time.Unix(be64(v[0:8]), 0)`; panics on short input. `:649` |
| `existsTxRecord(ns, txHash, block) (k, v)` | Key always, value or nil. `:653` |
| `existsRawTxRecord(ns, k) []byte` | Value or nil. `:659` |
| `deleteTxRecord(ns, txHash, block) error` | Delete; `IO`. `:663` |
| `latestTxRecord(ns, txHash []byte) (k, v []byte)` | Last record whose key has prefix `txHash` (highest height, then greatest block hash); `(nil, nil)` when none. `:675` |

**Credits (`c`, `ic`)**

| Function | Contract |
| --- | --- |
| `keyCredit(txHash, index, *Block) []byte` | 72-byte key. `:734` |
| `condenseOpCode(opCode uint8) byte` | `OP_TGEN (0xc3)` → `0x1c`; otherwise `(opCode - 0xb9) << 2` in wrapping uint8 arithmetic. `:743` |
| `expandOpCode(flag byte) uint8` | `flag & 0x1c == 0x1c` → `0xc3`; otherwise `((flag >> 2) & 0x07) + 0xb9`. `:754` |
| `valueUnspentCredit(*credit, scrType, scrLoc, scrLen, account, dbVersion) []byte` | 94-byte value, spent bit clear, spender key zero. `:771` |
| `putRawCredit(ns, k, v) error` | Put into `c`; `IO`. `:800` |
| `putUnspentCredit(ns, *credit, …) error` | key + value + `putRawCredit`. `:811` |
| `extractRawCreditTxHash(k) chainhash.Hash` | `k[0:32]`. `:818` |
| `extractRawCreditTxRecordKey(k) []byte` | `k[0:68]`. `:823` |
| `extractRawCreditHeight(k) int32` | `be32(k[32:36])`. `:827` |
| `extractRawCreditIndex(k) uint32` | `be32(k[68:72])`. `:831` |
| `extractRawUnminedCreditTxHash(k) []byte` | `k[:32]` (for `mc` keys). `:835` |
| `extractRawCreditIsSpent(v) bool` | `v[8] & 0x01 != 0`. `:839` |
| `extractRawCreditSpenderDebitKey(v) []byte` | `v[9:81]`. Panics if `len(v) < 81` (legacy 9-byte values). `:843` |
| `fetchRawCreditAmount(v) (Amount, error)` | `len(v) < 9` → `IO` `"credit len %d"`. `:848` |
| `fetchRawCreditAmountSpent(v) (Amount, bool, error)` | Amount and bit 0. Same check. `:857` |
| `fetchRawCreditAmountChange(v) (Amount, bool, error)` | Amount and bit 1. Same check. `:866` |
| `fetchRawCreditUnspentValue(k) ([]byte, error)` | `k[32:68]` (the `u` value for this credit); `len(k) < 72` → `IO` `"credit key len %d"`. `:875` |
| `fetchRawCreditTagOpCode(v) uint8` | `expandOpCode(v[8])`. `:883` |
| `fetchRawCreditIsCoinbase(v) bool` | `v[8] & 0x20 != 0`. `:889` |
| `fetchRawCreditHasExpiry(v, dbVersion) bool` | `dbVersion >= 9`: bit 6; `dbVersion == 8`: bit 4; lower: false. Also used on `mc` values. `:895` |
| `fetchRawCreditScriptOffset(v) uint32` | `be32(v[82:86])`, or 0 when `len(v) < 94`. `:908` |
| `fetchRawCreditScriptLength(v) uint32` | `be32(v[86:90])`, or 0 when `len(v) < 94`. `:917` |
| `fetchRawCreditAccount(v) (uint32, error)` | `len(v) < 94` → `IO` `"credit len %d"`; `v[81] & 0x80 == 0` → `IO` `"credit account unset"`; else `be32(v[90:94])`. `:926` |
| `spendCredit(ns, k, *indexedIncidence) (Amount, error)` | Rewrites the credit as spent. Algorithm 5.5. `:942` |
| `unspendRawCredit(ns, k) (Amount, error)` | Clears the spent bit. Returns `(0, nil)` if the credit does not exist. Algorithm 5.5. `:959` |
| `existsCredit(ns, txHash, index, block) (k, v)` | Lookup in `c`. `:976` |
| `existsRawCredit(ns, k) []byte` | Lookup in `c`. `:982` |
| `existsInvalidatedCredit(ns, txHash, index, block) (k, v)` | Same key, bucket `ic`. `:986` |
| `deleteRawCredit(ns, k) error` | Delete from `c`; `IO`. `:992` |
| `makeReadCreditIterator(ns, prefix, dbVersion) creditIterator` and `readElem/next/close` | Prefix scan of `c`. Algorithm 5.4. `:1030-1082` |

**Unspent index (`u`)**

| Function | Contract |
| --- | --- |
| `valueUnspent(*Block) []byte` | 36 bytes `be32(height)‖blockHash`. `:1098` |
| `putUnspent(ns, *wire.OutPoint, *Block) error` | `IO` on failure. `:1105` |
| `putRawUnspent(ns, k, v) error` | `IO` on failure. `:1115` |
| `readUnspentBlock(v, *Block) error` | `len(v) < 36` → `IO` `"unspent len %d"`. `:1123` |
| `existsUnspent(ns, *wire.OutPoint) (k, credKey []byte)` | `k` always; `credKey` is the reconstructed 72-byte `c` key or nil. `:1135` |
| `existsRawUnspent(ns, k) []byte` | nil if `len(k) < 36`, row missing, or value `< 36` bytes; else `k[0:32]‖v[0:36]‖k[32:36]`. `:1143` |
| `deleteRawUnspent(ns, k) error` | `IO` on failure. `:1158` |

**Debits (`d`, `id`)**

| Function | Contract |
| --- | --- |
| `keyDebit(txHash, index, *Block) []byte` | 72-byte key (same shape as a credit key, index = input index). `:1185` |
| `valueDebit(amount, credKey) []byte` | 80 bytes `be64(amount)‖credKey[0:72]`. `:1194` |
| `putDebit(ns, txHash, index, amount, block, credKey) error` | Put into `d`; `IO`. `:1201` |
| `putRawDebit(ns, k, v) error` | Put into `d`; `IO`. `:1212` |
| `extractRawDebitHash(k) []byte` | `k[:32]`. `:1220` |
| `extractRawDebitTxRecordKey(k) []byte` | `k[:68]`. `:1224` |
| `extractRawDebitInputIndex(k) uint32` | `be32(k[68:72])`. `:1228` |
| `extractRawDebitAmount(v) Amount` | `be64(v[:8])`. `:1232` |
| `extractRawDebitCreditKey(v) []byte` | `v[8:80]`. `:1236` |
| `extractRawDebitUnspentValue(v) []byte` | `v[40:76]` (height‖block hash of the spent credit). `:1240` |
| `existsDebit(ns, txHash, index, block) (k, credKey, err)` | `(nil, nil, nil)` if absent; value `< 80` bytes → `IO` `"debit len %d"`. `:1247` |
| `existsInvalidatedDebit(...)` | Same against bucket `id`. `:1259` |
| `deleteRawDebit(ns, k) error` | Delete from `d`; `IO`. `:1272` |
| `makeReadDebitIterator(ns, prefix) debitIterator` and `readElem/next/close` | Prefix scan of `d`. Algorithm 5.4. `:1303-1349` |

The stake-invalidated buckets `ic` and `id` have **no** `put`, `delete` or iterator helper in
`txdb.go`; only the two point readers `existsInvalidatedCredit` and `existsInvalidatedDebit`
exist. Every write and delete of an `ic`/`id` row is a direct
`ns.NestedReadWriteBucket(bucketStakeInvalidated…).Put/Delete` in `txmined.go`
(`wallet/udb/txmined.go:686, 740, 825, 866, 1284, 1423`), using keys and values built by the `c`
and `d` codecs above (`keyCredit`, `valueUnspentCredit`, `keyDebit`, `valueDebit`, or bytes moved
verbatim from `c`/`d`). Those raw calls wrap or return errors as the mined-transaction spec
states, not through a helper in this file. `makeReadCreditIterator` and `makeReadDebitIterator`
are hard-wired to `c` and `d` and cannot scan `ic`/`id`.

**Unmined transactions (`m`), unpublished markers (`up`)**

| Function | Contract |
| --- | --- |
| `putRawUnmined(ns, k, v) error` | `IO` on failure. `:1357` |
| `readRawUnminedHash(k, *Hash) error` | `len(k) < 32` → `IO` `"unmined key len %d"`. `:1365` |
| `existsRawUnmined(ns, k) []byte` | Value or nil. `:1373` |
| `deleteRawUnmined(ns, k) error` | `IO` on failure. `:1377` |
| `fetchRawUnminedReceiveTime(v) (int64, error)` | `len(v) < 8` → `IO` `"unmined len %d"`. `:1385` |
| `extractRawUnminedTx(v) []byte` | `v[8:]`; **panics** when `len(v) < 8`, including nil. `:1392` |
| `putUnpublished(ns, k) error` | `up[k] = {0x01}`; `IO`. `:1401` |
| `existsUnpublished(ns, k) bool` | True iff the value is exactly one non-zero byte. `:1409` |
| `deleteUnpublished(ns, k) error` | `IO` on failure. `:1414` |

**Unmined credits (`mc`)**

| Function | Contract |
| --- | --- |
| `valueUnminedCredit(amount, change, opCode, isCoinbase, hasExpiry, scrType, scrLoc, scrLen, account, dbVersion) []byte` | 22-byte value. `:1461` |
| `putRawUnminedCredit(ns, k, v) error` | `IO`. `:1492` |
| `fetchRawUnminedCreditIndex(k) (uint32, error)` | `len(k) < 36` → `IO` `"unmined credit key len %d"`. `:1500` |
| `fetchRawUnminedCreditAmount(v) (Amount, error)` | `len(v) < 9` → `IO` `"unmined credit len %d"`. `:1507` |
| `fetchRawUnminedCreditAmountChange(v) (Amount, bool, error)` | Same check; change = bit 1. `:1514` |
| `fetchRawUnminedCreditTagOpCode(v) uint8` | `expandOpCode(v[8])`. `:1523` |
| `fetchRawUnminedCreditTagIsCoinbase(v) bool` | `v[8] & 0x20 != 0`. `:1527` |
| `fetchRawUnminedCreditScriptType(v) scriptType` | `v[9] & 0x7f`, or `scriptTypeNonexisting` when `len(v) < 22`. `:1531` |
| `fetchRawUnminedCreditScriptOffset(v) uint32` | `be32(v[10:14])`, or 0 when `len(v) < 22`. `:1538` |
| `fetchRawUnminedCreditScriptLength(v) uint32` | `be32(v[14:18])`, or 0 when `len(v) < 22`. `:1545` |
| `fetchRawUnminedCreditAccount(v) (uint32, error)` | `len(v) < 22` → `IO` `"unmined credit len %d"`; flag clear → `IO` `"unmined credit account unset"`. `:1552` |
| `existsRawUnminedCredit(ns, k) []byte` | Value or nil. `:1565` |
| `deleteRawUnminedCredit(ns, k) error` | `IO`. `:1569` |
| `makeReadUnminedCreditIterator(ns, txHash, dbVersion)` and `readElem/next/close` | Prefix scan of `mc`. Algorithm 5.4. `:1614-1676` |
| `readCursor` (struct embedding `walletdb.ReadCursor`), `readCursor.Delete()` | Adapter so read cursors satisfy `ReadWriteCursor`; `Delete` always returns `IO` `"delete called from read-only cursor"`. `:1606-1612` |

**Unmined inputs (`mi`)**

| Function | Contract |
| --- | --- |
| `putRawUnminedInput(ns, k, v) error` | `IO`. `:1691` |
| `existsRawUnminedInput(ns, k) []byte` | Spender tx hash bytes or nil. `:1699` |
| `deleteRawUnminedInput(ns, k) error` | `IO`. `:1703` |
| `readRawUnminedInputSpenderHash(v, *Hash)` | Copies `v[:32]`; panics if `len(v) < 32`. `:1711` |

**Tickets (`tix`)**

| Function | Contract |
| --- | --- |
| `valueTicketRecord(pickedHeight int32) []byte` | `be32(uint32(pickedHeight))`. `:1720` |
| `putRawTicketRecord(ns, k, v) error` | `IO`. `:1726` |
| `putTicketRecord(ns, ticketHash, pickedHeight) error` | `:1734` |
| `existsRawTicketRecord(ns, k) []byte` | Value or nil. `:1740` |
| `extractRawTicketPickedHeight(v) int32` | `int32(be32(v))`. Used only by a test. `:1744` |

**Multisig outputs (`ms`, `mu`)**

| Function | Contract |
| --- | --- |
| `keyMultisigOut(hash, index) []byte` | `canonicalOutPoint`. `:1773` |
| `valueMultisigOut(sh [20]byte, m, n uint8, spent bool, tree int8, blockHash, blockHeight uint32, amount, spentBy, sbi uint32, txHash) []byte` | 135-byte value. `:1777` |
| `fetchMultisigOut(k, v) (*MultisigOut, error)` | Requires `len(k) == 36` and `len(v) == 135` exactly; errors are `IO` with the **literal, unformatted** messages `"multisig output key len %d"` / `"multisig output len %d"` (quirk Q5). `OutPoint.Tree` is always regular; `Tree` field reflects flag bit 1. `:1808` |
| `fetchMultisigOutScrHash(v) [20]byte` | `v[0:20]`. `:1849` |
| `fetchMultisigOutMN(v) (uint8, uint8)` | `v[20], v[21]`. `:1855` |
| `fetchMultisigOutSpent(v) bool` | `v[22] & 0x01 != 0`. `:1859` |
| `fetchMultisigOutTree(v) int8` | `1` if `v[22] & 0x02 != 0` else `0`. `:1865` |
| `fetchMultisigOutSpentVerbose(v) (bool, Hash, uint32)` | Spent bit, `v[67:99]`, `be32(v[99:103])`. `:1875` |
| `fetchMultisigOutMined(v) (Hash, uint32)` | `v[23:55]`, `be32(v[55:59])`. `:1884` |
| `fetchMultisigOutAmount(v) Amount` | `be64(v[59:67])`. `:1892` |
| `setMultisigOutSpent(v, spendHash, spendIndex)` | In place: `v[22] = 0x01` (clears the tree bit), writes spender hash and index. `:1896` |
| `setMultisigOutUnSpent(v)` | In place: `v[22] = 0x00` (clears the tree bit), zeroes `v[67:98]` (**31 bytes**; byte 98 untouched), `v[99:103] = ff ff ff ff`. `:1904` |
| `setMultisigOutMined(v, blockHash, blockHeight)` | In place write of `[23:59]`. `:1912` |
| `setMultisigOutUnmined(v)` | In place: zero `[23:59]`. `:1918` |
| `putMultisigOutRawValues(ns, k, v) error` | Put into `ms`; `IO`. `:1924` |
| `existsMultisigOut(ns, k) []byte` | Raw value or nil. `:1932` |
| `existsMultisigOutCopy(ns, k) []byte` | nil if absent, else a fresh 135-byte buffer holding the first `min(len, 135)` bytes. `:1936` |
| `putMultisigOutUS(ns, k) error` | `mu[k] = {0x00}`; `IO`. `:1946` |
| `deleteMultisigOutUS(ns, k) error` | `IO`. `:1955` |
| `existsMultisigOutUS(ns, k) bool` | Row present. `:1963` |

**Compact filters (`cf`)**

| Function | Contract |
| --- | --- |
| `valueRawCFilter2(key [16]byte, data []byte) []byte` | `key‖data`. `:1981` |
| `putRawCFilter(ns, k, v) error` | Put into `cf`; error returned **unwrapped** (driver error kind). `:1988` |
| `fetchRawCFilter2(ns, k) ([16]byte, []byte, error)` | nil value → `NotExist` `"no cfilter saved for block %v"`; `len(v) < 16` → `IO` `"cfilter data len: %d"`; returned data slice aliases database memory. `:1992` |

**Ticket commitments (`cmt`, `cmu`)**

| Function | Contract |
| --- | --- |
| `keyTicketCommitment(ticketHash, index) []byte` | `canonicalOutPoint`. `:2021` |
| `valueTicketCommitment(amount, account) []byte` | 12 bytes `be64(amount)‖be32(account)`. `:2025` |
| `existsRawTicketCommitment(ns, k) []byte` | Value or nil. `:2032` |
| `putRawTicketCommitment(ns, k, v) error` | `IO`. `:2036` |
| `fetchRawTicketCommitmentAmount(v) (Amount, error)` | `len(v) < 8` → `IO` `"ticket commitment amount %d"`. `:2044` |
| `fetchRawTicketCommitmentAccount(v) (uint32, error)` | `len(v) < 12` → `IO` `"ticket commitment account %d"`. `:2051` |
| `deleteRawTicketCommitment(ns, k) error` | `IO`. `:2058` |
| `valueUnspentTicketCommitment(unminedSpent bool) []byte` | 1 byte: `0x01` or `0x00`. `:2079` |
| `putRawUnspentTicketCommitment(ns, k, v) error` | `IO`. `:2091` |
| `deleteRawUnspentTicketCommitment(ns, k) error` | `IO`. `:2099` |
| `makeUnspentTicketCommitsIterator(ns)` and `next/close` | Full scan of `cmu` joined with `cmt`. Algorithm 5.4. `:2119-2166` |

**Store creation and legacy upgrade**

| Function | Contract |
| --- | --- |
| `createStore(ns, chainParams) error` | Populate an empty namespace (legacy version-1-of-unified layout). Algorithm 5.1. `:2170` |
| `upgradeTxDB(ns, chainParams) error` | Pre-unified upgrade driver keyed on root `vers`. Algorithm 5.2. `:2298` |
| `upgradeToVersion2(ns) error` | Writes `vers = 2`. `:2333` |
| `upgradeToVersion3(ns, chainParams) error` | Adds `h`, `ic`, `id`, widens block records, records genesis and `tip`. `:2358` |

### 3.4 Unexported helpers in `txunmined.go`, `kahnsort.go`, `agendas.go`

| Function | Contract |
| --- | --- |
| `(*Store).removeDoubleSpends(ns, rec *TxRecord) error` | For each input of `rec`, if an unmined transaction spends the same outpoint, `RemoveUnconfirmed` it. Used when a mined transaction arrives. `wallet/udb/txunmined.go:149` |
| `(*Store).unminedTxRecords(ns) (map[Hash]*TxRecord, error)` | Decode every `m` row; sets `Unpublished` from `up`. `wallet/udb/txunmined.go:296` |
| `(*Store).unminedTxHashes(ns) ([]*Hash, error)` | Implementation of `UnminedTxHashes`. `wallet/udb/txunmined.go:324` |
| `graphNode` | `{value *TxRecord; outEdges []*Hash; inDegree int}`. `wallet/udb/kahnsort.go:12` |
| `hashGraph` | `map[Hash]graphNode`. `wallet/udb/kahnsort.go:18` |
| `makeGraph(set) hashGraph` | Build the in-set dependency graph. Algorithm 5.14. `wallet/udb/kahnsort.go:20` |
| `graphRoots(graph) []*TxRecord` | Records with in-degree 0, in map iteration order. `wallet/udb/kahnsort.go:75` |
| `dependencySort(txs) []*TxRecord` | Kahn's algorithm. Algorithm 5.14. `wallet/udb/kahnsort.go:87` |
| `agendaPreferencesTy.key(version, agendaID) []byte` | `be32(version)‖agendaID`. `wallet/udb/agendas.go:23` |
| `agendaPreferencesTy.defaultBucketKey() / ticketsBucketKey()` | The two bucket names. `wallet/udb/agendas.go:30-32` |
| `agendaPreferencesTy.setDefaultPreference / setTicketPreference / defaultPreference / ticketPreference` | Unwrapped implementations of the four exported functions. `wallet/udb/agendas.go:34-60` |

### 3.5 Types declared in scope

All unexported. Field lists are given because iterator state is read directly by callers in
`txmined.go` and `txquery.go` (`it.elem`, `it.ck`, `it.cv`, `it.err`).

| Type | Definition | Source |
| --- | --- | --- |
| `scriptType` | `uint8` | `wallet/udb/txdb.go:72` |
| `blockIterator` | `{c walletdb.ReadWriteCursor; seek []byte; ck []byte; cv []byte; elem blockRecord; err error}`. Returned by value | `wallet/udb/txdb.go:345-352` |
| `creditIterator` | `{c walletdb.ReadWriteCursor; dbVersion uint32; prefix []byte; ck []byte; cv []byte; elem CreditRecord; err error}`. Returned by value | `wallet/udb/txdb.go:1020-1028` |
| `debitIterator` | `{c walletdb.ReadWriteCursor; prefix []byte; ck []byte; cv []byte; elem DebitRecord; err error}`. Returned by value | `wallet/udb/txdb.go:1294-1301` |
| `unminedCreditIterator` | `{c walletdb.ReadWriteCursor; dbVersion uint32; prefix []byte; ck []byte; cv []byte; elem CreditRecord; err error}`. Returned by value | `wallet/udb/txdb.go:1596-1604` |
| `readCursor` | `struct{ walletdb.ReadCursor }` plus a value-receiver `Delete() error` that always fails. Lets a read-only cursor fill the `ReadWriteCursor`-typed `c` field of the four iterators above | `wallet/udb/txdb.go:1606-1612` |
| `unspentTicketCommitsIterator` | `{c walletdb.ReadCursor; ns walletdb.ReadBucket; ck []byte; unminedSpent bool; amount dcrutil.Amount; account uint32; err error}`. Returned by **pointer**; holds a plain read cursor (no `readCursor` wrapper) and keeps `ns` for the `cmt` join. The `cmu` value is not retained | `wallet/udb/txdb.go:2107-2117` |
| `kvpair` | Function-local `struct{ k, v []byte }` used by `upgradeToVersion3` to buffer `b` rows | `wallet/udb/txdb.go:2382` |
| `removeTx` | Function-local `struct{ tx wire.MsgTx; hash *chainhash.Hash }` used by `PruneUnmined` to buffer selections | `wallet/udb/txunmined.go:349-352` |
| `graphNode`, `hashGraph` | See 3.4 | `wallet/udb/kahnsort.go:12-18` |
| `agendaPreferencesTy` | Empty struct; method namespace | `wallet/udb/agendas.go:13-14` |

### 3.6 Call sites at the pin

Where each group of helpers is actually used outside its own file (non-test code). This fixes
what is live, what exists only for upgrades, and what is dead.

| Helper(s) | Used by |
| --- | --- |
| `extractRawTicketPickedHeight` | **No non-test caller.** Only `wallet/udb/upgrades_test.go:279` |
| `putTicketRecord` | Only the database version 6 upgrade (`wallet/udb/upgrades.go:589`). Runtime writers call `valueTicketRecord(-1)` + `putRawTicketRecord` directly (`wallet/udb/txunmined.go:122-123`, `wallet/udb/txmined.go:1335-1336`) |
| `fetchBlockRecord` | Only the database version 8 and 9 upgrades (`wallet/udb/upgrades.go:647,741`) |
| `upgradeTxDB`, `upgradeToVersion2`, `upgradeToVersion3` | Only `Migrate` (`wallet/udb/migration.go:49`) |
| `makeReverseBlockIterator` | Only `Rollback` (`wallet/udb/txmined.go:1903`), together with `reposition` (`wallet/udb/txmined.go:2184`) and `deleteBlockRecord` (`wallet/udb/txmined.go:2199`) |
| `blockIterator.prev` | `Rollback` (on the reverse iterator) and the backwards branches of the range queries in `txquery.go` (`wallet/udb/txquery.go:416,756`), which call `prev` on an iterator built by `makeReadBlockIterator(ns, begin)`: the first `prev` seeks to `be32(begin)` and steps back once if no record has exactly that height |
| `fetchBlockTime` | Only `wallet/udb/txquery.go:71` |
| `readRawTxRecordMsgTx` | Only `wallet/udb/txquery.go:667` |
| `fetchRawTxRecordReceived` | Only `wallet/udb/txmined.go:2318` |
| `fetchRawUnminedReceiveTime` | Only `wallet/udb/txmined.go:2286` |
| `fetchRawUnminedCreditScriptType` | Only `moveMinedTx` (`wallet/udb/txmined.go:1161`): the type byte read from the `mc` row is written into the new `c` row. It is the sole reader of any stored script type |
| `fetchRawUnminedCreditTagIsCoinbase` | Only `moveMinedTx` (`wallet/udb/txmined.go:1157`) |
| `fetchRawCreditAccount`, `fetchRawUnminedCreditAccount` | Only the account lookup in `wallet/udb/txmined.go:1027,1033` |
| `existsInvalidatedCredit`, `existsInvalidatedDebit` | Only stake validation (`wallet/udb/txmined.go:681,714`) |
| `valueBlockRecordStakeValidated` / `valueBlockRecordStakeInvalidated` | Only `wallet/udb/txmined.go:652` / `:788` |
| `extractRawDebitUnspentValue` | Only `wallet/udb/txmined.go:874` |
| `existsMultisigOut` (no copy) | Only `RemoveUnconfirmed` (`wallet/udb/txunmined.go:259`); every `txmined.go` user calls `existsMultisigOutCopy` |
| `deleteRawTicketCommitment` | Only `RemoveUnconfirmed` (`wallet/udb/txunmined.go:226`) |
| `fetchRawTicketCommitmentAccount` / `fetchRawTicketCommitmentAmount` | Only `unspentTicketCommitsIterator.next` (`wallet/udb/txdb.go:2152,2156`) |
| `makeUnspentTicketCommitsIterator` | Only `wallet/udb/txmined.go:3519` |
| `makeReadUnminedCreditIterator` | Only `wallet/udb/txquery.go:120` |
| `valueMultisigOut`, `setMultisigOutMined`, `setMultisigOutSpent`, `deleteMultisigOutUS`, `existsMultisigOutUS`, `fetchMultisigOut*` | `txmined.go` multisig code (`wallet/udb/txmined.go:1754-1864, 2865-2913`) |
| `removeDoubleSpends` | Only `InsertMinedTx` (`wallet/udb/txmined.go:1358`) |
| `PruneUnmined` | Only `wallet/chainntfns.go:255` |
| `UnminedTxs` | Only `wallet/wallet.go:4366` |
| `SetDefaultAgendaPreference` / `SetTicketAgendaPreference` | Only `wallet/wallet.go:530` / `:532` |

---

## 4. Data formats

### 4.1 Conventions

- **Byte order.** Every integer the store itself encodes is **big-endian** (`byteOrder =
  binary.BigEndian`, `wallet/udb/txdb.go:123`). The stated reason is that cursor scans over
  integer keys then iterate in numeric order. The only little-endian reads are of fields inside
  embedded wire-format blobs (block headers and transactions), which keep their consensus
  encoding.
- **Hashes.** A `chainhash.Hash` is stored as its 32 raw bytes, the same order as on the wire.
  (The human-readable string form used in logs, errors and RPC is the byte-reversed hex.)
  The code statically asserts the hash is 32 bytes (`wallet/udb/txdb.go:129`).
- **Amounts.** `dcrutil.Amount` (int64 atoms) is stored as `be64(uint64(amount))`, i.e. two's
  complement. Negative amounts are never written by correct callers.
- **Heights.** `int32` heights are stored as `be32(uint32(height))`. `-1` is `ff ff ff ff`.
- **Times.** Stored as `be64(uint64(t.Unix()))`, whole seconds. Decoded with
  `time.Unix(int64(be64), 0)`. Sub-second precision is lost. The Go zero `time.Time{}` encodes as
  `ff ff ff f1 88 6e 09 00` (−62135596800).
- **Notation.** `[a:b]` is a half-open byte range. `be16/be32/be64` are big-endian unsigned
  integers. `‖` is concatenation.
- **Absent vs. empty.** `Get` on the bbolt driver returns nil both for a missing key and for a key
  that names a nested bucket (`bbolt@v1.3.12/bucket.go:262-275`). `Delete` of a missing key is not
  an error (`bbolt@v1.3.12/bucket.go:318-332`). `Put` rejects empty keys, keys longer than 32,768
  bytes, and keys that currently name a bucket (`bbolt@v1.3.12/bucket.go:281-306`).
- **Canonical outpoint** (used as key in `u`, `mc`, `mi`, `ms`, `mu`, `cmt`, `cmu`), 36 bytes
  (`wallet/udb/txdb.go:200-205`):

  | Offset | Size | Field |
  | --- | --- | --- |
  | 0 | 32 | Transaction hash |
  | 32 | 4 | Output index, be32 |

  The outpoint's tree is **not** part of the key.
- **Transaction incidence prefix** (the first 68 bytes of `t`, `c`, `ic`, `d`, `id` keys):

  | Offset | Size | Field |
  | --- | --- | --- |
  | 0 | 32 | Transaction hash |
  | 32 | 4 | Block height, be32 |
  | 36 | 32 | Block hash |

In-memory shapes the codecs map to (declared outside the scope, repeated for reference):

| Type | Fields | Source |
| --- | --- | --- |
| `Block` | `Hash chainhash.Hash`, `Height int32` | `wallet/udb/txmined.go:44` |
| `BlockMeta` | `Block`, `Time time.Time`, `VoteBits uint16` | `wallet/udb/txmined.go:52` |
| `blockRecord` | `Block`, `Time`, `VoteBits`, `transactions []chainhash.Hash` | `wallet/udb/txmined.go:60` |
| `indexedIncidence` | `txHash`, `block Block`, `index uint32` | `wallet/udb/txmined.go:71-81` |
| `credit` | `outPoint wire.OutPoint`, `block Block`, `amount`, `change bool`, `spentBy indexedIncidence`, `opCode uint8`, `isCoinbase bool`, `hasExpiry bool` | `wallet/udb/txmined.go:84` |
| `TxRecord` | `MsgTx wire.MsgTx`, `Hash`, `Received time.Time`, `SerializedTx []byte` (optional), `TxType stake.TxType`, `Unpublished bool` | `wallet/udb/txmined.go:96` |
| `CreditRecord` | `Index uint32`, `Amount`, `Spent`, `Change`, `OpCode uint8`, `IsCoinbase`, `HasExpiry` | `wallet/udb/txquery.go:23` |
| `DebitRecord` | `Amount`, `Index uint32` | `wallet/udb/txquery.go:36` |
| `MultisigOut` | `OutPoint *wire.OutPoint`, `Tree int8`, `ScriptHash [20]byte`, `M`, `N uint8`, `TxHash`, `BlockHash`, `BlockHeight uint32`, `Amount`, `Spent bool`, `SpentBy`, `SpentByIndex uint32` | `wallet/udb/txmined.go:145` |
| `RawBlockHeader` | `[180]byte` | `wallet/udb/txmined.go:607` |

### 4.2 Where the store lives in `wallet.db`

`wallet.db` is a bbolt file. Its top-level buckets at database version 26 are listed for
orientation; only the three in bold are specified here.

| Top-level bucket (ASCII name) | Created | Owner |
| --- | --- | --- |
| `waddrmgr` | initialization | address manager spec |
| **`wtxmgr`** | initialization (`wallet/udb/initialize.go:24`) | **this spec** |
| `meta` | initialization; key `ver` = be32 database version (`wallet/udb/metadata.go:15-28`) | upgrades spec |
| **`agendaprefs`** | db v3 (`wallet/udb/upgrades.go:426`) | **this spec**, section 4.7 |
| **`ticketsagendaprefs`** | db v16 (`wallet/udb/upgrades.go:1355`) | **this spec**, section 4.7 |
| `treasurypolicy` | db v19 (`wallet/udb/upgrades.go:1459`) | treasury spec |
| `vsp` | db v20 | VSP spec |
| `tspendpolicy` | db v22 | treasury spec |
| `vsphost`, `vsppubkey` | db v23 (`wallet/udb/upgrades.go:1565-1570`) | VSP spec |
| `vsptreasurypolicy`, `vsptspendpolicy` | db v24 (`wallet/udb/upgrades.go:1633-1637`) | treasury spec |

The namespace key constants are `wtxmgrBucketKey = "wtxmgr"` and `waddrmgrBucketKey =
"waddrmgr"` (`wallet/udb/migration.go:17-20`).

### 4.3 Root key/value pairs of the `wtxmgr` bucket

Root keys and nested-bucket names share one keyspace (they are entries of the same bbolt
bucket). All names are ASCII without terminator.

| Key | Key bytes (hex) | Value | Written by | Read by |
| --- | --- | --- | --- | --- |
| `date` | `64617465` | 8 bytes: be64 Unix seconds of store creation | `createStore` (`wallet/udb/txdb.go:2180-2185`) | never read |
| `vers` | `76657273` | 4 bytes: be32 legacy store version (1, 2 or 3). **Absent** in every unified database: `createStore` does not write it and `Migrate` deletes it (`wallet/udb/migration.go:63`) | pre-unified software; `upgradeToVersion2/3` | `upgradeTxDB` only |
| `bal` | `62616c` | 8 bytes: be64 mined balance (definition: invariant I9) | `createStore` (zero), `putMinedBalance` | `fetchMinedBalance`; length must be exactly 8 |
| `tip` | `746970` | 32 bytes: hash of the main-chain tip block | `createStore`, `upgradeToVersion3`, `ExtendMainChain`, `Rollback` | `MainChainTip`, `ExtendMainChain`, upgrades 10/11/15 |
| `havecfilters` | `6861766563 66696c74657273` | 1 byte: `01` when a filter is stored for every main-chain block, `00` otherwise | db upgrades 10 and 15, `InsertMissingCFilters` | `ExtendMainChain` (requires length 1 and value exactly `01`), `IsMissingMainChainCFilters` (missing ⇔ length ≠ 1 or value `00`), `InsertMissingCFilters` |
| `lasttxsblock` | `6c61737474 7873626c6f636b` | 32 bytes: hash of the last block processed for relevant transactions | db upgrade 11 (copy of `tip`), `UpdateProcessedTxsBlockMarker` | `ProcessedTxsBlockMarker` (zero hash if absent) |
| `birthstate` | `6269727468 7374617465` | 45 bytes, layout below | db upgrade 26, `SetBirthState` | `BirthState` (returns nil unless length is exactly 45) |
| `vsphostindex` | `767370686f 7374696e646578` | **Not a `wtxmgr` key.** Declared with the root keys (`wallet/udb/txdb.go:162`) but only ever used inside the top-level `vsphost` bucket, where it holds a be32 counter (`wallet/udb/vsp.go:53-65`, `wallet/udb/upgrades.go:1590`) | — | — |

(Spaces inside the longer hex strings are only for line wrapping.)

`birthstate` value (`wallet/udb/txmined.go:334-363`):

| Offset | Size | Field |
| --- | --- | --- |
| 0 | 1 | Options: bit 0 (`0x01`) `SetFromHeight`, bit 1 (`0x02`) `SetFromTime` |
| 1 | 32 | Birthday block hash |
| 33 | 4 | Birthday block height, be32 |
| 37 | 8 | Birthday time, be64 Unix seconds |

### 4.4 Nested buckets of `wtxmgr`

| Go name | Name | Name bytes | Created | Key | Value | Purpose |
| --- | --- | --- | --- | --- | --- | --- |
| `bucketBlocks` | `b` | `62` | `createStore` | be32 height (4) | block record (47 + 32·n) | Main-chain block records and the hashes of wallet transactions in each |
| `bucketHeaders` | `h` | `68` | `createStore` / legacy v3 | block hash (32) | serialized block header (180) | Every header ever accepted, main chain and side chains; never pruned |
| `bucketTxRecords` | `t` | `74` | `createStore` | tx hash ‖ be32 height ‖ block hash (68) | be64 received ‖ serialized tx | Mined wallet transactions |
| `bucketCredits` | `c` | `63` | `createStore` | incidence ‖ be32 output index (72) | credit (94; legacy shorter) | Mined outputs controlled by the wallet |
| `bucketUnspent` | `u` | `75` | `createStore` | outpoint (36) | be32 height ‖ block hash (36) | Index of `c` rows not spent by a mined transaction |
| `bucketDebits` | `d` | `64` | `createStore` | incidence ‖ be32 input index (72) | be64 amount ‖ credit key (80) | Mined inputs that spend a credit |
| `bucketUnmined` | `m` | `6d` | `createStore` | tx hash (32) | be64 received ‖ serialized tx | Unmined wallet transactions |
| `bucketUnpublished` | `up` | `7570` | db v18 (`wallet/udb/upgrades.go:1435`) | tx hash (32) | `01` | Marks an `m` transaction as not yet broadcast |
| `bucketUnminedCredits` | `mc` | `6d63` | `createStore` | outpoint (36) | unmined credit (22; legacy 9) | Outputs of unmined transactions controlled by the wallet |
| `bucketUnminedInputs` | `mi` | `6d69` | `createStore` | previous outpoint (36) | spending unmined tx hash (32) | Every outpoint spent by an unmined transaction |
| `bucketTickets` | `tix` | `746978` | db v6 (`wallet/udb/upgrades.go:540`) | ticket tx hash (32) | be32 picked height (4), always `ffffffff` | Set of all ticket purchases ever seen |
| `bucketScripts` | `sc` | `7363` | `createStore`; **deleted** at db v14 (`wallet/udb/upgrades.go:1255`) | 20-byte HASH160 of the script | redeem script | Legacy cleartext P2SH redeem scripts. Absent at db ≥ 14 |
| `bucketMultisig` | `ms` | `6d73` | `createStore` | outpoint (36) | multisig output (135) | P2SH multisig outputs the wallet participates in |
| `bucketMultisigUsp` | `mu` | `6d75` | `createStore` | outpoint (36) | `00` | Index of unspent `ms` rows |
| `bucketStakeInvalidatedCredits` | `ic` | `6963` | `createStore` / legacy v3 | same as `c` | same as `c` | Credits of regular-tree transactions in a stake-invalidated block |
| `bucketStakeInvalidatedDebits` | `id` | `6964` | `createStore` / legacy v3 | same as `d` | same as `d` | Debits of regular-tree transactions in a stake-invalidated block |
| `bucketCFilters` | `cf` | `6366` | db v10; dropped and recreated at db v15 (`wallet/udb/upgrades.go:858,1298-1302`) | block hash (32) | 16-byte key ‖ filter bytes | Version 2 compact filters |
| `bucketTicketCommitments` | `cmt` | `636d74` | db v12 (`wallet/udb/upgrades.go:935`) | outpoint (36) | be64 amount ‖ be32 account (12) | Ticket commitment outputs that pay back to the wallet |
| `bucketTicketCommitmentsUsp` | `cmu` | `636d75` | db v12 (`wallet/udb/upgrades.go:940`) | outpoint (36) | flags (1) | Index of `cmt` rows whose ticket is not yet spent by a mined vote/revocation |

Lexicographic order of all entries of the `wtxmgr` bucket at db v26 (what a cursor over the
namespace yields): `b`, `bal`, `birthstate`, `c`, `cf`, `cmt`, `cmu`, `d`, `date`, `h`,
`havecfilters`, `ic`, `id`, `lasttxsblock`, `m`, `mc`, `mi`, `ms`, `mu`, `t`, `tip`, `tix`, `u`,
`up`.

### 4.5 Value layouts

#### 4.5.1 Block record (`b`) — `wallet/udb/txdb.go:216-243`

Key: `be32(height)`.

| Offset | Size | Field |
| --- | --- | --- |
| 0 | 32 | Block hash |
| 32 | 8 | Block time, be64 Unix seconds (the header's 32-bit timestamp, zero-extended) |
| 40 | 2 | Vote bits, be16 (the header's value, re-encoded big-endian) |
| 42 | 1 | Stake-invalidated flag: `00` = regular tree valid, `01` = regular tree invalidated by the next block. Readers treat any non-zero byte as invalidated |
| 43 | 4 | Number of transaction hashes `n`, be32 |
| 47 | 32·n | Transaction hashes, in the order the wallet inserted them (not block order) |

- A new record is 47 bytes with `n = 0` (`valueBlockRecordEmptyFromHeader`).
- Readers accept values longer than `47 + 32·n` and ignore the excess; they reject shorter ones.
- Pre-unified version 1/2 stores used a 46 + 32·n layout without byte 42;
  `upgradeToVersion3` inserts the byte (algorithm 5.2).
- The time and vote bits duplicate header fields (acknowledged as redundant in the source
  comment at `wallet/udb/txdb.go:219-220`); they must still be written.

#### 4.5.2 Block header (`h`) — `wallet/udb/txdb.go:460-485`

Key: block hash. Value: the 180-byte wire serialization of the header
(`dcrd/wire@v1.8.0/blockheader.go:26`: `MaxBlockHeaderPayload = 84 + 3·32`). The store reads fields
straight out of the blob at fixed offsets, little-endian, with no length check
(`wallet/udb/txmined.go:545-593`):

| Offset | Size | Field | Reader |
| --- | --- | --- | --- |
| 4 | 32 | Previous block hash | `extractBlockHeaderParentHash` |
| 36 | 32 | Merkle root (first 16 bytes are the cfilter key) | `extractBlockHeaderMerkleRoot` |
| 100 | 2 | Vote bits, le16 | `extractBlockHeaderVoteBits` |
| 128 | 4 | Height, le32 | `extractBlockHeaderHeight` |
| 136 | 4 | Timestamp, le32 Unix seconds | `extractBlockHeaderUnixTime` |

#### 4.5.3 Transaction record (`t`) and unmined transaction (`m`) — `wallet/udb/txdb.go:487-526, 1351-1355`

`t` key: 68-byte incidence prefix. `m` key: 32-byte transaction hash. Value (both buckets):

| Offset | Size | Field |
| --- | --- | --- |
| 0 | 8 | Received time, be64 Unix seconds |
| 8 | varies | Serialized transaction |

- The transaction bytes are exactly `rec.SerializedTx` when the caller supplied them, otherwise
  `rec.MsgTx.Serialize()` (honouring the message's `SerType`). The store does not check which
  serialization type it was given. Script offsets stored in credits (4.5.4, 4.5.7) are offsets
  into these bytes and are computed by `wire.MsgTx.PkScriptLocs`
  (`dcrd/wire@v1.8.0/msgtx.go:1125-1157`), which assumes the full (prefix + witness) serialization;
  records are expected to hold that form. UNVERIFIED that every caller of `NewTxRecord` /
  `NewTxRecordFromMsgTx` passes a full serialization; settled by auditing those call sites in
  the wallet, chain and SPV packages.
- When a transaction moves between `m` and `t` (mined, or rolled back) the value bytes are copied
  verbatim, so the received time survives (`wallet/udb/txmined.go:1070,1989`).
- Duplicate transaction hashes in different blocks are distinct `t` rows; the key order puts the
  highest block last.

#### 4.5.4 Credit (`c`, `ic`) — `wallet/udb/txdb.go:687-798`

Key (72 bytes): incidence prefix ‖ `be32(output index)`.

Value (`creditValueSize` = 94 bytes):

| Offset | Size | Field |
| --- | --- | --- |
| 0 | 8 | Amount, be64 |
| 8 | 1 | Flags (table below) |
| 9 | 32 | Spender transaction hash |
| 41 | 4 | Spender block height, be32 |
| 45 | 32 | Spender block hash |
| 77 | 4 | Spender input index, be32 |
| 81 | 1 | Low 7 bits: `scriptType`; bit 7 (`0x80`): account field is valid |
| 82 | 4 | Script offset within the serialized transaction, be32 (`0` = not stored) |
| 86 | 4 | Script length, be32 |
| 90 | 4 | Account number, be32 |

Bytes `[9:81]` are exactly the 72-byte key of the spending row in `d`. They are zero in a freshly
created credit and meaningful only while flag bit 0 is set (see quirk Q7 for stale content).

Flags byte:

| Bits | Mask | Meaning |
| --- | --- | --- |
| 0 | `0x01` | Spent by a mined transaction |
| 1 | `0x02` | Change output |
| 2–4 | `0x1c` | Stake opcode tag: `0x00` none (decodes to `OP_NOP10` = `0xb9`), `0x04` `OP_SSTX` (`0xba`), `0x08` `OP_SSGEN` (`0xbb`), `0x0c` `OP_SSRTX` (`0xbc`), `0x10` `OP_SSTXCHANGE` (`0xbd`), `0x1c` `OP_TGEN` (`0xc3`). `0x14` and `0x18` are never written (they would decode to `0xbe`, `0xbf`) |
| 5 | `0x20` | Output of a coinbase transaction (`compat.IsEitherCoinBaseTx`: coinbase under either the pre- or post-treasury rule set) |
| 6 | `0x40` | Transaction has a non-zero expiry (db version ≥ 9) |
| 7 | `0x80` | Never set |

Database version 8 stored "has expiry" in bit 4 (`0x10`), colliding with `OP_SSTXCHANGE`; the
version 9 upgrade moved it to bit 6 (`wallet/udb/upgrades.go:720-836`). Code at the pin always
passes `DBVersion` (26), so only bit 6 is ever read or written.

Both writers (`valueUnspentCredit`, `wallet/udb/txdb.go:791-792`, and `valueUnminedCredit`,
`wallet/udb/txdb.go:1483-1484`) set the account-valid bit unconditionally: byte 81 (byte 9 of an
unmined credit) is always `scrType | 0x80` and the account field is always whatever the caller
passed. There is no way to write a row with the bit clear at the pin; a clear bit only occurs in
rows widened from a legacy value by `spendCredit`/`unspendRawCredit` (zero padding) or written by
old software. A script type ≥ `0x80` would be indistinguishable from the flag; the nine defined
values stay below it.

The stake opcode tag is derived by `getStakeOpCode` from the output script class and covers both
the pubkey-hash and the script-hash form of each stake script
(`wallet/udb/txmined.go:1436-1452`), although the layout comment labels the field "P2PKH stake
flag" (`wallet/udb/txdb.go:703`).

`scriptType` values written (`pkScriptType`, `wallet/udb/txmined.go:1456-1480`): 1 unspecified,
2 P2PKH (ECDSA secp256k1), 3 P2PK (ECDSA secp256k1), 4 P2PKH alt (Ed25519 or Schnorr secp256k1),
5 P2PK alt, 6 P2SH, 7 stake-tagged P2PKH (ticket, vote, revocation, stake change, treasury gen),
8 stake-tagged P2SH. 0 means "no script type recorded". The mined-credit script type byte is
write-only at the pin: no reader exists.

Shorter legacy values: every reader of the amount and flags accepts any value of at least 9
bytes; readers of the script location return 0 (meaning "deserialize the transaction") and the
account reader fails with `"credit len %d"` when the value is shorter than 94 bytes. The source
comment describes the spender key and the script block as optional (`wallet/udb/txdb.go:712-724`),
i.e. 9-byte (unspent) and 81-byte (spent) values written by old software. No code at the pin
writes them, and any spend or unspend of such a row rewrites it at 94 bytes with zero padding.
UNVERIFIED: whether such rows still exist in current wallets; settled by inspecting a wallet
created before script locations were added, or the pre-1.0 history of `wtxmgr`.

#### 4.5.5 Unspent index (`u`) — `wallet/udb/txdb.go:1084-1103`

Key: canonical outpoint. Value (36 bytes):

| Offset | Size | Field |
| --- | --- | --- |
| 0 | 4 | Block height, be32 |
| 4 | 32 | Block hash |

The credit key is rebuilt as `key[0:32] ‖ value[0:36] ‖ key[32:36]`.

#### 4.5.6 Debit (`d`, `id`) — `wallet/udb/txdb.go:1166-1199`

Key (72 bytes): incidence prefix of the spending transaction ‖ `be32(input index)`.

Value (80 bytes):

| Offset | Size | Field |
| --- | --- | --- |
| 0 | 8 | Amount of the spent credit, be64 |
| 8 | 32 | Credit transaction hash |
| 40 | 4 | Credit block height, be32 |
| 44 | 32 | Credit block hash |
| 76 | 4 | Credit output index, be32 |

Bytes `[8:80]` are the `c` key of the spent credit; bytes `[40:76]` are the `u` value to restore
if the debit is undone.

#### 4.5.7 Unmined credit (`mc`) — `wallet/udb/txdb.go:1422-1490`

Key: canonical outpoint of the output.

Value (`unconfValueSize` = 22 bytes):

| Offset | Size | Field |
| --- | --- | --- |
| 0 | 8 | Amount, be64 |
| 8 | 1 | Flags: same bit assignment as 4.5.4; bit 0 is never set |
| 9 | 1 | Low 7 bits: `scriptType`; bit 7 (`0x80`): account field is valid |
| 10 | 4 | Script offset within the serialized transaction, be32 |
| 14 | 4 | Script length, be32 |
| 18 | 4 | Account number, be32 |

Legacy values are 9 bytes (`unconfValueSizeLegacy`: amount + flags). Readers of the script
fields return 0 / `scriptTypeNonexisting` for them and the account reader fails with
`"unmined credit len %d"`, which callers treat as "look the account up from the script".

When the transaction is mined, `moveMinedTx` copies the script type, offset and length read from
the `mc` row into the new `c` row and sets the account-valid bit with the account it resolved
(`wallet/udb/txmined.go:1159-1181`). A `c` row minted from a legacy 9-byte `mc` row therefore has
byte 81 = `0x80` (type 0, account valid), offset 0 and length 0, and its script is found through
the deserializing path of 5.7.

#### 4.5.8 Unmined input (`mi`) — `wallet/udb/txdb.go:1678-1713`

Key: canonical outpoint being spent. Value: 32-byte hash of the unmined transaction spending it.
Input 0 of a vote (the stakebase, null outpoint) is never recorded
(`wallet/udb/txunmined.go:104-107`; the db v6 upgrade deletes any old row keyed by 32 zero bytes ‖
`ff ff ff ff`, `wallet/udb/upgrades.go:596-602`).

#### 4.5.9 Unpublished marker (`up`) — `wallet/udb/txdb.go:1396-1420`

Key: transaction hash. Value: single byte `01`. "Published" is represented by the row being
absent. A reader treats any value that is not exactly one non-zero byte as "published".

#### 4.5.10 Ticket record (`tix`) — `wallet/udb/txdb.go:1715-1746`

Key: ticket purchase transaction hash. Value: 4 bytes, be32 of the height at which the ticket
was picked, `-1` if not picked. Every writer at the pin writes `-1` (`ff ff ff ff`); the field
is never read outside a test.

#### 4.5.11 Multisig output (`ms`) and its unspent index (`mu`) — `wallet/udb/txdb.go:1748-1806`

`ms` key: canonical outpoint. Value (exactly 135 bytes):

| Offset | Size | Field |
| --- | --- | --- |
| 0 | 20 | P2SH script hash (HASH160 of the redeem script) |
| 20 | 1 | `m` (required signatures) |
| 21 | 1 | `n` (number of public keys) |
| 22 | 1 | Flags: bit 0 (`0x01`) spent, bit 1 (`0x02`) output is in the stake tree |
| 23 | 32 | Block hash (all zero while unmined) |
| 55 | 4 | Block height, be32 (0 while unmined) |
| 59 | 8 | Amount, be64 |
| 67 | 32 | Spending transaction hash (all zero when created unspent) |
| 99 | 4 | Spending input index, be32 (`ff ff ff ff` when unspent) |
| 103 | 32 | Hash of the transaction containing the output (same as the key's hash) |

`mu` key: canonical outpoint. Value: single byte `00`. Presence means unspent.

State-changing helpers rewrite the whole flags byte, so the stake-tree bit is lost on the first
spend or unspend, and "unspend" leaves byte 98 as it was (quirks Q8, Q9).

#### 4.5.12 Compact filter (`cf`) — `wallet/udb/txdb.go:1968-2005`

Key: block hash. Value:

| Offset | Size | Field |
| --- | --- | --- |
| 0 | 16 | Filter key: the first 16 bytes of the block header's merkle root (`blockcf2.Key`, `dcrd/gcs/v4@v4.1.1/blockcf2/blockcf.go:96-100`) |
| 16 | varies | Serialized version 2 GCS filter exactly as returned by `FilterV2.Bytes()`: a wire varint `N` followed by the Golomb-coded data. An empty filter is zero bytes, giving a 16-byte value |

The filter parameters are not stored; readers assume `B = 19`, `M = 784931`
(`dcrd/gcs/v4@v4.1.1/blockcf2/blockcf.go:33-38`).

#### 4.5.13 Ticket commitment (`cmt`) and unspent commitment index (`cmu`) — `wallet/udb/txdb.go:2007-2089`

Key (both): canonical outpoint of the commitment output, i.e. ticket hash ‖ be32(odd output
index).

`cmt` value (12 bytes):

| Offset | Size | Field |
| --- | --- | --- |
| 0 | 8 | Committed amount, be64 (decoded from the commitment script, not the output value) |
| 8 | 4 | Account number, be32 |

`cmu` value (1 byte): bit 0 (`0x01`) = `unminedSpent`, an unmined vote or revocation currently
redeems the ticket. Other bits are zero.

#### 4.5.14 Legacy scripts (`sc`)

Only relevant when upgrading a database older than version 14. From the upgrade code
(`wallet/udb/upgrades.go:1161-1258`): each value is a raw redeem script and rows are deleted by
the 20-byte HASH160 of that script, so the key is the script hash. The bucket itself is removed
by that upgrade.

### 4.6 Contents of a freshly created store

`Initialize` builds a version-1 unified database with `createStore` and then runs every upgrade
through version 26 (`wallet/udb/initialize.go:18-50`). The resulting `wtxmgr` namespace is:

| Entry | Content |
| --- | --- |
| `bal` | `00 00 00 00 00 00 00 00` |
| `date` | be64 of wall-clock seconds at creation |
| `tip` | genesis block hash |
| `havecfilters` | `01` (set to `00` then `01` by upgrades 10 and 15 because the tip is the genesis block) |
| `lasttxsblock` | genesis block hash (upgrade 11 copies `tip`) |
| `birthstate` | 45 bytes: `00` ‖ genesis hash ‖ `00 00 00 00` ‖ `ff ff ff f1 88 6e 09 00` (upgrade 26 stores a zero `time.Time`, `wallet/udb/upgrades.go:1680-1685`) |
| `b` | one row: key `00 00 00 00`, 47-byte record for the genesis block (hash, header time, header vote bits, flag `00`, count 0) |
| `h` | one row: genesis hash → 180-byte serialized genesis header |
| `cf` | one row: genesis hash → 16-byte key (genesis merkle root prefix) ‖ bytes of `blockcf2.Regular(genesisBlock)` built with no previous scripts (`wallet/udb/upgrades.go:1314-1320`) |
| `t`, `c`, `d`, `u`, `m`, `mc`, `mi`, `ms`, `mu`, `ic`, `id`, `tix`, `cmt`, `cmu`, `up` | present and empty |
| `sc`, `vers` | absent |

### 4.7 Agenda preference buckets (`wallet/udb/agendas.go`)

Two top-level buckets, outside `wtxmgr`:

**`agendaprefs`** (`61 67 65 6e 64 61 70 72 65 66 73`) — wallet-wide default choices.

| Part | Encoding |
| --- | --- |
| Key | `be32(deployment version)` ‖ agenda ID bytes (the ASCII vote ID from chain parameters, no terminator, no length prefix) |
| Value | Choice ID bytes (ASCII, no terminator) |

**`ticketsagendaprefs`** (`74 69 63 6b 65 74 73 61 67 65 6e 64 61 70 72 65 66 73`) — per-ticket
choices. It contains one nested bucket per ticket, named by the 32 raw bytes of the ticket
hash, created on first write with `CreateBucketIfNotExists`. Each nested bucket holds key/value
pairs in exactly the `agendaprefs` encoding.

Nothing in scope ever deletes a row or a per-ticket bucket. A choice is replaced by overwriting.
The "no preference" state is the absence of the row (the reader also maps an empty value to
`""`).

### 4.8 Invariants between buckets

These hold in a consistent store. I1–I8 and I10–I14 are relied on by code in scope; where code
tolerates a violation that is stated. The code that maintains I2–I6 and I9 lives in
`txmined.go` and is specified in the mined-transaction spec; they are listed because the formats
only make sense together.

- **I1 (tip).** `tip` is 32 bytes, has a row in `h`, and the `b` row at that header's height has
  the same hash. `b` holds exactly the main chain, contiguous from height 0 to the tip height.
  `h` is a superset (side-chain headers stay forever).
- **I2 (block ↔ tx).** Each hash listed in a `b` row at height `H` with block hash `BH` has a
  `t` row keyed `hash‖be32(H)‖BH`, and each `t` row is listed in its block's record exactly once.
- **I3 (credits).** The first 68 bytes of every `c` and `ic` key are the key of an existing `t`
  row, and the output index is in range for that transaction.
- **I4 (unspent index).** A `u` row exists for outpoint `(h, i)` with value `(H, BH)` if and only
  if `c` has a row `h‖H‖BH‖i` whose spent bit is clear. Outputs spent only by an *unmined*
  transaction keep their `u` row; the spend is visible only through `mi`.
- **I5 (debits).** A `c` row has its spent bit set if and only if bytes `[9:81]` are the key of a
  `d` row whose value bytes `[8:80]` are that credit's key and whose amount equals the credit's
  amount. The first 68 bytes of each `d`/`id` key are an existing `t` key.
- **I6 (stake invalidation).** If a block's flag byte is `01`, the credits of its regular-tree
  transactions are in `ic` (not `c`, no `u` row) and their debits are in `id` (not `d`; the
  credits they reference stay unspent). Stake-tree transactions of that block are unaffected.
- **I7 (mined xor unmined).** A transaction hash is never a key of `m` while any `t` row has it
  as prefix. `InsertMemPoolTx` enforces this at insert; mining moves the row.
- **I8 (unmined set closure).** For every `m` transaction `T`: each input's previous outpoint
  (except input 0 of a vote) has an `mi` row whose value is `hash(T)`; every `mc` key's hash
  prefix and every `mi` value is the key of an `m` row. At most
  one unmined transaction spends a given outpoint. An `up` key normally names an `m` row, but
  this is **not** enforced: `SetPublished(…, false)` writes the `up` row without looking at `m`,
  and the wallet calls it in its own `Update` *before* the transaction is inserted
  (`wallet/wallet.go:4752-4756`), so an `up` row can precede its `m` row and stays behind if the
  insert then fails. `up` rows are deleted only by `SetPublished(…, true)`, `RemoveUnconfirmed`
  and `moveMinedTx` (`wallet/udb/txmined.go:1201`); a mined insert of a transaction that is not
  in `m` leaves an existing `up` row in place. A stray row is observable:
  `Store.IsUnspentOutpoint` returns false for every outpoint whose transaction hash has an `up`
  row, without checking `m` (`wallet/udb/txmined.go:2633-2636`); the other readers key the
  lookup off an `m` or `mc` row (`wallet/udb/txunmined.go:310`,
  `wallet/udb/txmined.go:2508,2605,3142,3489`). If an `mi` value points at a missing `m` row,
  `InsertMemPoolTx` (vote/revocation path) and `Store.UnspentTickets` **panic**, while
  `RemoveUnconfirmed` and `removeDoubleSpends` return `IO` `"tx record len 0"` (quirk Q2).
- **I9 (mined balance).** `bal` equals the sum of amounts of all `c` rows whose spent bit is
  clear and whose stake opcode tag is not `OP_SSTX` (ticket submission outputs are never
  counted). Outputs spent by unmined transactions are still counted; immature outputs are
  counted.
- **I10 (tickets).** `tix` has a row for every ticket purchase transaction ever inserted, mined or
  unmined. Rows are never deleted, so a `tix` key may have neither a `t` nor an `m` row (the
  ticket was pruned or abandoned). Readers must tolerate that
  (`wallet/udb/txmined.go:2766-2832`).
- **I11 (ticket commitments).** `cmu` keys ⊆ `cmt` keys. A `cmt` row is created (with a `cmu`
  row, flag `00`) the first time a wallet-owned commitment output is seen and is deleted only
  when its *unmined* ticket is removed from the store. The `cmu` row is deleted when a mined
  vote/revocation redeems the ticket and its flag is `01` while an unmined vote/revocation
  spends it. The `cmu` iterator fails with `"ticket commitment account 0"` if a `cmu` key has no
  `cmt` row.
- **I12 (multisig).** `mu` keys ⊆ `ms` keys; a `mu` row exists iff the `ms` flags spent bit is
  clear.
- **I13 (filters).** Every main-chain block has a `cf` row when `havecfilters` is `01`.
  `ImportCFiltersV2` requires the header of each main-chain block it touches to be in `h`.
- **I14 (agenda preferences).** No cross-bucket invariant: per-ticket buckets may exist for
  hashes the wallet no longer tracks, and keys may reference retired deployment versions.

---

## 5. Algorithms and state machines

General rules for everything below:

- No function in scope opens, commits or rolls back a database transaction. Each runs inside the
  `walletdb` transaction (or bucket obtained from one) that the caller passes. Atomicity is
  whatever the caller's `walletdb.Update` gives: commit if the closure returns nil, roll back if
  it returns an error or panics (`wallet/walletdb/interface.go:225-255`).
- No function in scope takes a lock, starts a goroutine, sleeps, retries or observes a context.
- Unless stated, a failed `Put`/`Delete` is returned wrapped as kind `IO`.
- Hashes in messages are formatted with `%v`, giving the byte-reversed hex string; outpoints
  print as `<hash>:<index>` (`dcrd/wire@v1.8.0/msgtx.go:290-302`).

### 5.1 `createStore` — `wallet/udb/txdb.go:2170-2293`

Runs inside the single `Update` transaction of `Initialize` / `InitializeWatchOnly`, after the
address manager is created and before the `meta` bucket is written.

1. Open a cursor on `ns`; if `First()` returns a non-nil key or value return `IO`
   `"bucket is not empty"`.
2. Put `date` = be64(`time.Now().Unix()`).
3. Put `bal` = 8 zero bytes.
4. Create nested buckets in this order: `b`, `h`, `t`, `c`, `d`, `u`, `m`, `mc`, `mi`, `sc`,
   `ms`, `mu`, `ic`, `id`. Any failure → `IO`.
5. Serialize the genesis block header from chain parameters into a 180-byte buffer (a failure
   panics). Put `h[genesisHash]` = header.
6. Put `b[00 00 00 00]` = `valueBlockRecordEmptyFromHeader(genesisHash, header)`.
7. Put `tip` = genesis hash.

`createStore` does not write `vers` and does not create `tix`, `cf`, `cmt`, `cmu` or `up`; those
come from the upgrades that `Initialize` runs immediately afterwards (section 4.6).

### 5.2 Legacy upgrade: `upgradeTxDB`, `upgradeToVersion2`, `upgradeToVersion3` — `wallet/udb/txdb.go:2295-2438`

Only called from `Migrate` (`wallet/udb/migration.go:49`), which converts a pre-unified database
(one without a `meta` bucket) and then deletes `vers`.

`upgradeTxDB(ns, params)`:

1. `v = ns.Get("vers")`. If `len(v) != 4` → `IO` `"no transaction store version"`.
2. `version = be32(v)`. If 0 → `IO` `"invalid transaction store version 0"`.
3. Loop: version 1 → `upgradeToVersion2`; version 2 → `upgradeToVersion3`; any version ≥ 3 →
   return nil. After each successful step increment `version`.

`upgradeToVersion2`: put `vers` = `00 00 00 02`. Nothing else.

`upgradeToVersion3`:

1. Put `vers` = `00 00 00 03`.
2. Create nested buckets `h`, `ic`, `id` (failure, including "already exists", → `IO`).
3. Collect all key/value pairs of `b` with `ForEach`, then for each write a value one byte
   longer: bytes `[0:42]` unchanged, byte 42 = `00`, old bytes `[42:]` shifted to `[43:]`.
4. Serialize the genesis header (panic on failure) and put it in `h` under the genesis hash.
5. If `b` has no row for height 0, put the empty genesis block record.
6. Put `tip` = genesis hash (deliberately not the highest recorded block; headers for existing
   blocks are missing at this point and must be supplied by the caller).

Edge cases of step 3 (`wallet/udb/txdb.go:2392-2400`): the rewrite does not validate the old
value. A `b` value shorter than 42 bytes **panics** on the slice `kvp.v[:42]`; a value of 42 to
45 bytes is widened without complaint into a record that every later reader rejects (`len < 47`).
The count field and the hash list are not inspected, and any bytes after the hash list are carried
along. The key/value slices collected inside `ForEach` are the driver's borrowed slices and are
used after the `ForEach` returns and while the same bucket is being written; this is valid in
bbolt because the borrowed memory lives until the transaction ends, and a port must copy the
pairs if its store does not give that guarantee. The genesis header put in step 4 overwrites any
existing `h` row for the genesis hash (there can be none: step 2 has just created `h`). Step 5
tests only `b[00 00 00 00] == nil`; an existing height-0 record is kept as is (after widening),
even if its hash is not the genesis hash of `chainParams`.

### 5.3 Block record iterator — `wallet/udb/txdb.go:345-453`

State: cursor `c` (nil once exhausted), `seek` (4-byte key), current `ck`/`cv`, decoded `elem`
(`blockRecord`), `err`.

- `makeReadBlockIterator(ns, height)`: `seek = be32(height)`; read cursor on `b` wrapped in
  `readCursor`.
- `makeReverseBlockIterator(ns)`: `seek = ff ff ff ff`; read-write cursor on `b`.
- `next()`:
  1. If `c` is nil return false.
  2. If `ck` is nil (first call) `ck, cv = c.Seek(seek)`, else `c.Next()`.
  3. If `ck` is nil: close the cursor, set `c = nil`, return false.
  4. `readRawBlockRecord(ck, cv, &elem)`; on error set `c = nil`, `err`, return false.
  5. Return true.
- `prev()`:
  1. If `c` is nil return false.
  2. First call: `ck, cv = c.Seek(seek)`; if `ck` does not have `seek` as prefix (including nil)
     `ck, cv = c.Prev()`. Later calls: `c.Prev()`.
  3. If `ck` is nil: close, `c = nil`, return false.
  4. Decode as in `next()`; on error close, `c = nil`, set `err`, return false.
  This depends on the bbolt cursor contract that a `Seek` past the last key returns nil yet
  leaves the cursor positioned so that `Prev` yields the last key
  (`bbolt@v1.3.12/cursor.go:117-133`, `:212-244`, `:248-277`).
- `close()`: closes the cursor if non-nil (a no-op in the bbolt driver,
  `wallet/internal/bdb/db.go:262`).
- `reposition(height)`: `c.Seek(be32(height))`, result discarded. Used by `Rollback` to re-anchor
  the cursor after other writes; must not be called once `c` is nil.
- There is no delete-through-cursor: the method is commented out with a reference to a bolt bug
  (`wallet/udb/txdb.go:442-449`); callers collect heights and call `deleteBlockRecord` afterwards.

Elements of `elem.transactions` are freshly allocated per record (`make`), other fields are
overwritten in place.

`prev()` is also called on iterators built by `makeReadBlockIterator(ns, begin)` (the
backwards range queries, `wallet/udb/txquery.go:412-420, 752-760`). There the first call seeks
to `be32(begin)`: if a record at exactly that height exists it is returned; otherwise the seek
lands on the next higher record (or past the end) and the single `Prev` step yields the highest
record below `begin`. The prefix test is exact because the seek key and the record key are both
4 bytes.

`appendRawBlockRecord` does not look for an existing occurrence of the hash; appending the same
transaction hash twice yields a record that lists it twice with the count incremented twice. The
caller is responsible for invariant I2.

### 5.4 Prefix and index iterators — `wallet/udb/txdb.go:1020-1082, 1294-1349, 1596-1676, 2107-2166`

All three prefix iterators share one shape. State: cursor `c`, `prefix`, `ck`, `cv`, `elem`,
`err` (credit iterators also carry `dbVersion`).

`next()`:

1. If `c` is nil return false.
2. First call `ck, cv = c.Seek(prefix)`, later calls `c.Next()`.
3. If `ck` does not start with `prefix` (including nil): close, `c = nil`, return false.
4. `readElem()`; on error set `err` and return false **without** closing or clearing `c` (a
   further `next()` would continue with the following row).
5. Return true.

`close()` closes the cursor if `c` is non-nil.

| Iterator | Bucket | Prefix | `readElem` |
| --- | --- | --- | --- |
| `creditIterator` | `c` | 68-byte tx record key | `len(ck) < 72` → `IO` `"credit key len %d"`; `len(cv) < 9` → `IO` `"credit len %d"`. Sets `Index`, `Amount`, `Spent` (bit 0 only: mined spends), `Change`, `OpCode` (expanded), `IsCoinbase`, `HasExpiry` |
| `debitIterator` | `d` | 68-byte tx record key | `len(ck) < 72` → `IO` `"debit key len %d"`; `len(cv) < 80` → `IO` `"debit len %d"`. Sets `Index`, `Amount` |
| `unminedCreditIterator` | `mc` | 32-byte tx hash | `IO` `"unmined credit key len %d"` / `"unmined credit len %d"`. Sets `Index`, `Amount`, `Change`, `HasExpiry`. Does **not** set `Spent`, `OpCode` or `IsCoinbase`; they stay at the Go zero value (`OpCode == 0`, not `0xb9`) |

Neither credit iterator reports spends by unmined transactions; callers check
`existsRawUnminedInput(canonicalOutPoint(txHash, index))` themselves.

None of the iterators can delete through its cursor. `unminedCreditIterator` carries a
commented-out `delete()` with the same bolt-bug reference as `blockIterator`
(`wallet/udb/txdb.go:1662-1669`), and the cursors of the three prefix iterators are
`readCursor`-wrapped, so a `Delete` would return `IO` `"delete called from read-only cursor"`.
Callers delete by key after or between iteration steps.

`unspentTicketCommitsIterator` (`cmu` joined with `cmt`):

1. If `c` is nil return false.
2. First call `ck, v = c.Seek(nil)` (first key), later `c.Next()`.
3. If `ck` is nil: close, `c = nil`, return false.
4. If `len(v) < 1`: `err` = `IO` `"wrong unspent ticket commitment len %d"`, return false.
5. `unminedSpent = (v[0] & 0x01) == 0x01`.
6. `v = cmt[ck]`; `account` from `fetchRawTicketCommitmentAccount` (error → return false);
   `amount` from `fetchRawTicketCommitmentAmount`.
7. Return `err == nil`.

Error semantics of this iterator differ from the prefix iterators and matter to a port:

- None of the error exits (steps 4 and 6) closes the cursor or clears `c`, so a further `next()`
  advances to the following `cmu` row.
- `err` is **assigned on every step that reaches step 6** (`it.account, it.err = …`, then
  `it.amount, it.err = …`, `wallet/udb/txdb.go:2152-2158`). A successful `next()` after a failed
  one therefore resets `err` to nil; the error is not sticky. The step-4 error is the only one
  set without also being cleared by the same call.
- On a step-6 failure `unminedSpent` already holds the new row's flag, `account` has been set to
  0, and `amount` still holds the previous row's value. `ck` is the failing key.
- The amount fetch cannot fail once the account fetch has succeeded (12 ≥ 8), so the message
  `"ticket commitment amount %d"` is unreachable through the iterator, which is the only caller
  of `fetchRawTicketCommitmentAmount` (section 3.6).
- The single consumer, `balanceFullScan`, tests `it.err` only **inside** the loop body, where it
  is always nil, and never after the loop (`wallet/udb/txmined.go:3519-3544`). An iterator error
  therefore ends the scan silently: the commitments at and after the bad row are left out of
  `LockedByTickets`/`Total` and no error is returned (quirk Q36).

### 5.5 `spendCredit` and `unspendRawCredit` — `wallet/udb/txdb.go:939-974`

`spendCredit(ns, k, spender)`:

1. `v = c[k]` (no existence check).
2. Allocate 94 zero bytes and copy `v` into them (short legacy values are zero-extended; a
   missing value yields all zeros).
3. Set bit 0 of byte 8.
4. Write `spender.txHash` at `[9:41]`, `be32(spender.block.Height)` at `[41:45]`,
   `spender.block.Hash` at `[45:77]`, `be32(spender.index)` at `[77:81]`.
5. Put the 94-byte value under `k`. Return `be64([0:8])` and the put error (kind `IO`).

`unspendRawCredit(ns, k)`:

1. `v = c[k]`; if nil return `(0, nil)`.
2. Allocate 94 zero bytes, copy `v`, clear bit 0 of byte 8. The spender key at `[9:81]` is left
   as it was.
3. Put under `k` (failure → `IO`). Return the amount read from the *old* value.

### 5.6 Transaction record helpers — `wallet/udb/txdb.go:502-685`

`valueTxRecord(rec)`:

1. If `rec.SerializedTx` is nil: allocate `8 + rec.MsgTx.SerializeSize()` bytes, serialize the
   transaction directly after the 8-byte header; serialization error → kind `Invalid`.
2. Otherwise allocate `8 + len(SerializedTx)` and copy the bytes verbatim (no validation that
   they match `rec.MsgTx` or `rec.Hash`).
3. Write `be64(uint64(rec.Received.Unix()))` at `[0:8]`.

`readRawTxRecord(txHash, v, rec)`: `len(v) < 8` → `IO` `"tx record len %d"`; `rec.Hash =
*txHash` (taken from the caller, not recomputed); `rec.Received = time.Unix(be64(v[0:8]), 0)`;
deserialize `v[8:]` into `rec.MsgTx` (trailing bytes after a complete transaction are ignored;
failure → `IO`); `rec.TxType = stake.DetermineTxType(&rec.MsgTx)`
(`dcrd/blockchain/stake/v5@v5.0.3/staketx.go:1272-1294`).

`latestTxRecord(ns, txHash)`: cursor on `t`; `Seek(txHash)`; while the current key has `txHash`
as prefix remember it and advance; close; return the last remembered pair (or nil, nil).

### 5.7 `fetchRawTxRecordPkScript` — `wallet/udb/txdb.go:606-647`

Inputs: a `t` (or synthesized) record key `k`, record value `v`, output `index`, and the script
offset/length taken from a credit value.

1. Read the tx hash from `k` (`len(k) < 68` → stack trace on stderr and `IO`
   `"tx record key len %d"`).
2. If `scrLoc == 0` (`scriptLocNotStored`): fully decode the record; `index >= len(TxOut)` →
   `IO` `"missing transaction output for credit index"`; return that output's script.
3. Otherwise let `off = scrLoc + 8` and `n = scrLen`. If `off > len(v) - 1` or `off + n >
   len(v)` → `IO` `"invalid script offset %d for tx %v"` (the printed offset is `off`, i.e.
   includes the 8-byte header). Return a copy of `v[off : off+n]`. `index` is not consulted.

### 5.8 `InsertMemPoolTx` — `wallet/udb/txunmined.go:23-134`

Input: `rec` with `Hash`, `MsgTx`, `TxType`, `Received`, optional `SerializedTx`, `Unpublished`.

1. `ns` = `wtxmgr` bucket of the write transaction.
2. If `latestTxRecord(ns, rec.Hash)` finds a value → return `Exist`
   `"transaction already exists mined"`.
3. If `m[rec.Hash]` exists → return nil (no change of any kind, including the `up` marker).
4. For each input, in order, with `k = canonicalOutPoint(prevHash, prevIndex)`: if `mi[k]`
   exists, let `spenderHash` be its first 32 bytes and:
   1. If `rec.TxType` is `SSGen` (vote) or `SSRtx` (revocation):
      - Load `m[spenderHash]`, take bytes `[8:]` (panics if the row is missing or shorter than 8
        bytes) and deserialize; failure → `IO`.
      - If the spender is **not** a vote (`stake.DetermineTxType != SSGen`), fall through to the
        generic conflict error (step 4.2).
      - `votedBlock` = hash returned by `stake.SSGenBlockVotedOn(spender)` (bytes `[2:34]` of
        output 0's script); `tipBlock` = `MainChainTip` hash. If equal → return `DoubleSpend`
        `"vote or revocation <rec.Hash> double spends unmined vote <spenderHash> on the tip block"`.
      - Otherwise `RemoveUnconfirmed(ns, spender, spenderHash)` (propagate its error) and
        continue with the next input.
   2. Return `DoubleSpend` `"<rec.Hash> conflicts with <spenderHash> by double spending
      <prevOutPoint>"`.
5. Log at Info: `Inserting unconfirmed transaction <hash>`.
6. `v = valueTxRecord(rec)`; put `m[rec.Hash] = v`.
7. If `rec.Unpublished`, put `up[rec.Hash] = 01`.
8. `txType = stake.DetermineTxType(&rec.MsgTx)` (recomputed; step 4 used `rec.TxType`).
9. For each input `i`: skip if `i == 0 && txType == SSGen`; otherwise put `mi[k_i] = rec.Hash`.
10. If `txType == SStx` and `tix[rec.Hash]` is absent, put `tix[rec.Hash] = ff ff ff ff`.
11. Return nil.

Not done here: unmined credits (`mc`), ticket commitments and multisig bookkeeping are added by
the caller afterwards (`Store.AddCredit`, `AddTicketCommitment`, `RedeemTicketCommitments`,
`AddMultisigOut`, `SpendMultisigOut`), in the same database transaction.

Unlike a mined transaction, an unmined transaction never evicts a conflicting unmined
transaction; the newcomer is rejected. The only exception is step 4.1.

Side effects before a failure: step 4 processes inputs one at a time, so a vote or revocation
whose first conflicting input evicted a stale vote (step 4.1, including that vote's descendants,
its `mi` rows, its `cmu` flag reset and its `up` row) can still fail on a later input with
`DoubleSpend`, or in steps 6–10 with `IO`/`Invalid`. The evictions are then already written to
the database transaction; `InsertMemPoolTx` does not undo them. Both callers return the error
from their `walletdb.Update` closure (`wallet/chainntfns.go:451-462`, `wallet/createtx.go:258-261`),
so the driver rolls the whole transaction back. The second values returned by
`stake.SSGenBlockVotedOn` (the voted height, `dcrd/blockchain/stake/v5@v5.0.3/staketx.go:466`) and
`Store.MainChainTip` (the tip height, `wallet/udb/txmined.go:183`) are discarded; neither call
can fail.

The source carries a `TODO: increment credit amount for each credit (but those are unknown here
currently)` at the end of the function (`wallet/udb/txunmined.go:130-131`): no balance
bookkeeping happens for unmined transactions, and `bal` is untouched.

### 5.9 `SetPublished` — `wallet/udb/txunmined.go:137-143`

`published == true`: delete `up[txHash]`. `published == false`: put `up[txHash] = 01`. No check
that `txHash` is in `m`.

### 5.10 `removeDoubleSpends` — `wallet/udb/txunmined.go:149-173`

Called by `InsertMinedTx` for a newly mined transaction that was not already in `m`.

For each input of `rec`, in order:

1. `doubleSpendHash = mi[canonicalOutPoint(prev)]`; if nil continue.
2. Decode `m[doubleSpendHash]` with `readRawTxRecord` (a missing row yields `IO`
   `"tx record len 0"`).
3. Log at Debug: `Removing double spending transaction <hash>`.
4. `RemoveUnconfirmed(ns, &doubleSpend.MsgTx, &doubleSpend.Hash)`; propagate errors.

### 5.11 `RemoveUnconfirmed` — `wallet/udb/txunmined.go:181-280`

Inputs: `ns`, the transaction `tx` and its hash. Recursive, depth = length of the longest unmined
spend chain below `tx`.

1. `stxType = stake.DetermineTxType(tx)`.
2. For each output index `i` from 0 to `len(tx.TxOut) - 1`, with `k = canonicalOutPoint(txHash, i)`:
   1. If `mi[k]` exists: decode the spender from `m[mi[k]]` (`readRawTxRecord`; error
      propagated), log at Debug `Transaction <spender hash> is part of a removed conflict chain
      -- removing as well`, and recurse `RemoveUnconfirmed(ns, spender.MsgTx, spender.Hash)`.
      (The recursion's step 4 deletes `mi[k]`.)
   2. Delete `mc[k]`.
   3. If `stxType == SStx` and `i` is odd: if `cmt[k]` exists, log at Debug `Removing unconfirmed
      ticket commitment <txHash>:<i>`, delete `cmt[k]`, delete `cmu[k]`.
3. If `stxType` is `SSGen` or `SSRtx`: `replaceTicketCommitmentUnminedSpent(ns, stxType, tx,
   false)` (`wallet/udb/txmined.go:1661-1686`):
   - ticket hash = previous-outpoint hash of input 1 (vote) or input 0 (revocation);
     ticket output count = `(len(TxOut) - 2)·2 + 1` (vote) or `len(TxOut)·2 + 1` (revocation);
   - for `j = 1, 3, 5, … < count`: if `cmt[ticketHash‖be32(j)]` exists, log at Debug `Marking
     ticket commitment <hash>:<j> unmined spent as false` and put `cmu[...] = 00` (the row is
     created if it was absent).
4. For each input (all of them, including a vote's stakebase), `k = canonicalOutPoint(prev)`:
   1. Delete `mi[k]` (unconditionally; the value is not compared with `txHash`).
   2. If `ms[k]` exists: copy the value, apply `setMultisigOutUnSpent`, put it back, and put
      `mu[k] = 00`.
5. Delete `up[txHash]`.
6. Delete `m[txHash]` and return its result.

The function does not check that `txHash` is actually in `m`, does not touch `tix`, and does not
log at Info. Mined credits spent by the removed transaction become spendable again purely by the
`mi` row disappearing.

### 5.12 Listing: `UnminedTxs`, `unminedTxRecords`, `UnminedTxHashes` — `wallet/udb/txunmined.go:285-335`

`unminedTxRecords(ns)`: `ForEach` over `m`. For each row: key → hash (`len < 32` → `IO`
`"unmined key len %d"`); value → `TxRecord` via `readRawTxRecord`; `Unpublished =
existsUnpublished(ns, hash)`; store in a map keyed by hash. The first error aborts and is
returned together with the partial map.

`UnminedTxs(dbtx)`: build the map, then return `dependencySort(map)` (5.14). Includes
unpublished transactions; the wallet-level caller filters them (`wallet/wallet.go:4361-4379`).

`UnminedTxHashes(ns)`: `ForEach` over `m`, appending each key as a hash. Result order is
ascending key order. Returns a nil slice (not an empty one) when `m` is empty. A key shorter
than 32 bytes stops the scan with `IO` `"unmined key len %d"`; the hashes collected before it are
returned **together with** the error (`wallet/udb/txunmined.go:324-335`). `UnminedTxs`, by
contrast, returns `(nil, err)` on any scan error (`wallet/udb/txunmined.go:287-290`). Keys longer
than 32 bytes are accepted by both and truncated to their first 32 bytes.

### 5.13 `PruneUnmined` — `wallet/udb/txunmined.go:344-410`

Inputs: write transaction, `sdiff` = ticket price required for the next block, or 0 to disable
ticket pruning.

1. `tipHash, tipHeight = MainChainTip(dbtx)`.
2. Open a read cursor on `m`; iterate from `First()` with `Next()` until the key is nil. For
   each row:
   1. Deserialize `v[8:]` (panics if `len(v) < 8`); failure → return `(nil, IO)`.
   2. Classify, first match wins:
      - `tx.Expiry != 0 && tx.Expiry <= uint32(tipHeight)` → expired, remove.
      - `stake.IsSStx(tx)` → ticket; keep if `sdiff == 0` or `tx.TxOut[0].Value == sdiff`,
        otherwise remove.
      - `stake.IsSSGen(tx)` → vote; keep if the voted-on block hash equals `tipHash`, otherwise
        remove.
      - anything else → keep.
   3. For a row to remove: `txHash = chainhash.NewHash(k)` (wrong length → return `(nil, IO)`);
      log at Info one of `Removing expired unmined transaction <hash>`, `Removing old unmined
      ticket purchase <hash>`, `Removing missed or invalid vote <hash>`; append `(tx, txHash)` to
      a removal list.
3. After the scan, for each entry of the list in scan (ascending hash-byte) order:
   `RemoveUnconfirmed(ns, &tx, hash)`. On error return `(hashes removed so far, err)`.
   Otherwise append the hash.
4. Return the list (an empty non-nil slice when nothing was removed).

Notes:

- The expiry rule is evaluated before the stake rules, so an expired ticket or vote is reported
  as "expired".
- There is no revocation-specific rule: a revocation (and any other non-ticket, non-vote
  transaction) is removed only by the expiry rule, i.e. only if it carries a non-zero `Expiry`
  that is `<=` the tip height.
- A listed transaction may already have been removed as a descendant of an earlier one;
  `RemoveUnconfirmed` is then a harmless second pass (all deletes hit missing keys) and the hash
  is still reported.
- Descendants removed by recursion that were not themselves selected are **not** in the returned
  list.
- The wallet calls this once per chain extension, inside the same `Update` transaction that
  attached the blocks, and treats an error as non-fatal: it logs `Failed to prune unmined
  transactions when connecting block height <h>: <err>` at Error level and still commits
  (`wallet/chainntfns.go:255-259`). Partial removals therefore persist.

### 5.14 `dependencySort` (Kahn's algorithm) — `wallet/udb/kahnsort.go`

Input: `set`, a map from transaction hash to `*TxRecord`. Output: a slice containing every
record of the set exactly once such that a transaction appears after every in-set transaction it
spends from (given an acyclic set).

`makeGraph(set)`:

1. `graph` = empty map from hash to node `{value, outEdges, inDegree}`.
2. For each `rec` in `set` (Go map iteration order, i.e. unspecified):
   1. If `graph` has no node for `rec.Hash`, insert `{value: rec}`.
   2. For each input of `rec`, in order, with `p` = the input's previous transaction hash:
      - If `p` is not in `set`, skip.
      - Let `inputNode = graph[p]` (zero node if absent).
      - "Skip duplicate edges": if any hash in `inputNode.outEdges` equals **`p`**, skip this
        input. (This compares child hashes against the parent hash and so never matches; see
        quirk Q11. Consequently one edge is added per input, not per distinct parent.)
      - Set `graph[p] = {value: inputNode.value or set[p] if nil, outEdges: inputNode.outEdges +
        [rec.Hash], inDegree: inputNode.inDegree}`.
      - Set `graph[rec.Hash] = {value: rec, outEdges: unchanged, inDegree: +1}`.

`graphRoots(graph)`: all node values with `inDegree == 0`, in map iteration order.

`dependencySort(txs)`:

1. `graph = makeGraph(txs)`; `s = graphRoots(graph)`.
2. If `len(s) == len(txs)` (no in-set edges) return `s` as is.
3. `sorted` = empty. While `s` is non-empty: pop the **front** of `s` as `rec`, append it to
   `sorted`; for each `mHash` in `graph[rec.Hash].outEdges` in order: if the node's `inDegree`
   is non-zero, decrement it and store the node back, and if it reached zero append its value to
   the **back** of `s`.
4. Return `sorted`.

Because edges and in-degrees are both counted per input, a child that spends several outputs of
the same parent is still released exactly when its last parent edge is processed. If the set
contained a cycle (impossible for valid transactions) the members of the cycle would be silently
omitted from the result.

The relative order of independent transactions depends on Go map iteration and is different on
every call.

### 5.15 `CFilterV2` — `wallet/udb/cfilters.go:17-30`

1. `key, data, err = fetchRawCFilter2(ns, blockHash)`. On error return a zero key, nil filter
   and the error (`NotExist` `"no cfilter saved for block <hash>"`, or `IO` `"cfilter data len:
   <n>"` for a value shorter than 16 bytes).
2. Copy `data` (the filter object retains the slice it is given).
3. `gcs.FromBytesV2(19, 784931, copy)`. On failure return `(key, nil, err)` with the `gcs`
   error unwrapped. The only failure is a leading varint that `wire.ReadVarInt` rejects, i.e.
   one that is truncated or non-canonically encoded (`dcrd/gcs/v4@v4.1.1/gcs.go:539-566`,
   `dcrd/wire@v1.8.0/common.go:742-805`); the filter body is not validated. Zero-length data is a
   valid empty filter.
4. Return `(key, filter, nil)`.

### 5.16 `ImportCFiltersV2` — `wallet/udb/cfilters.go:34-67`

Inputs: write transaction, `startHeight`, a list of serialized filters.

1. Create a forward block iterator positioned at `startHeight` (5.3).
2. For filter `i` (0-based) in order:
   1. Advance the iterator; if it is exhausted return `NotExist`
      `"block height of <startHeight + i> unknown"`. (Iterator decode errors are also reported
      this way; `blockIter.err` is not inspected.)
   2. `gcs.FromBytesV2(19, 784931, filter)`; on error return it unwrapped.
   3. `bh` = the block record's hash. Fetch the header copy from `h` (`NotExist`
      `"block header"` if missing).
   4. `key` = bytes `[36:52]` of the header (first 16 bytes of the merkle root).
   5. Put `cf[bh] = key ‖ filter` (error returned unwrapped).
3. Return nil.

Properties: existing rows are overwritten; the filters are not checked against header
commitments; `havecfilters` is not modified; the iterator walks whatever `b` rows follow
`be32(startHeight)` in key order, so the heights are consecutive only because `b` is contiguous
(I1). A negative `startHeight` maps to a key above every real height and fails at `i = 0` with
`"block height of <startHeight> unknown"`. On error the rows already written remain in the
transaction; the wallet-level caller's `Update` rolls them back (`wallet/wallet.go:5278-5287`).

Further properties: an empty `filterData` returns nil without reading or writing any row (the
iterator is created but never advanced). The block iterator is never closed on any path
(`wallet/udb/cfilters.go:36-66`), which is harmless only because the bbolt cursor `Close` is a
no-op. The height in the error message is computed as `startHeight + int32(i)` in wrapping
32-bit arithmetic. The filter bytes are stored exactly as passed (not the re-serialization of
the parsed filter), and the parse result is discarded.

### 5.17 Agenda preferences — `wallet/udb/agendas.go`

- `SetDefaultAgendaPreference`: `agendaprefs.Put(be32(version)‖agendaID, choiceID)`; any error →
  `IO`.
- `SetTicketAgendaPreference`: `b = ticketsagendaprefs.CreateBucketIfNotExists(ticketHash)`
  (error → `IO`), then `b.Put(be32(version)‖agendaID, choiceID)` (error → `IO`).
- `DefaultAgendaPreference`: `string(agendaprefs.Get(key))`; missing → `""`.
- `TicketAgendaPreference`: nested bucket for the ticket; if absent `""`; else
  `string(bucket.Get(key))`.

None of them validates the agenda or choice against chain parameters; the caller
(`Wallet.SetAgendaChoices`, `wallet/wallet.go:476-545`) does. All four dereference the top-level
bucket without a nil check and therefore panic on a database older than version 3 / 16
(impossible after `Open`, which requires version 26).

---

## 6. Constants, limits and defaults

| Name | Value | Source | What depends on it |
| --- | --- | --- | --- |
| `byteOrder` | big-endian | `wallet/udb/txdb.go:123` | Every integer in every key and value of the namespace and of the agenda buckets |
| `wtxmgrBucketKey` | `"wtxmgr"` | `wallet/udb/migration.go:19` | Top-level bucket name |
| Root keys | `date`, `vers`, `bal`, `tip`, `havecfilters`, `lasttxsblock`, `birthstate` (`vsphostindex` is declared here but belongs to bucket `vsphost`) | `wallet/udb/txdb.go:156-163` | Section 4.3 |
| Nested bucket names | `b h t c u d m up mc mi tix sc ms mu ic id cf cmt cmu` | `wallet/udb/txdb.go:133-151` | Section 4.4 |
| `defaultAgendaPrefsBucketKey` | `"agendaprefs"` | `wallet/udb/agendas.go:19` | Top-level bucket name |
| `ticketsAgendaPrefsBucketKey` | `"ticketsagendaprefs"` | `wallet/udb/agendas.go:20` | Top-level bucket name |
| Canonical outpoint size | 36 | `wallet/udb/txdb.go:200-205` | Keys of `u mc mi ms mu cmt cmu` |
| Tx record key size | 68 | `wallet/udb/txdb.go:502-508` | `t` keys; prefix of `c ic d id` keys |
| `creditKeySize` | 72 | `wallet/udb/txdb.go:728` | `c`/`ic` keys (debit keys are also 72) |
| `creditValueSize` | 94 | `wallet/udb/txdb.go:731` | `c`/`ic` values; threshold for script/account readers |
| Minimum credit value | 9 | `wallet/udb/txdb.go:849,1039` | Amount + flags readers |
| Debit value size | 80 | `wallet/udb/txdb.go:1194-1199` | `d`/`id` values |
| Unspent value size | 36 | `wallet/udb/txdb.go:1098-1103` | `u` values |
| `unconfCreditKeySize` | 36 | `wallet/udb/txdb.go:1450` | `mc` keys |
| `unconfValueSizeLegacy` | 9 | `wallet/udb/txdb.go:1454` | Minimum `mc` value |
| `unconfValueSize` | 22 | `wallet/udb/txdb.go:1458` | `mc` values; threshold for script/account readers |
| Block record fixed part | 47 (46 before legacy store v3) | `wallet/udb/txdb.go:236-243, 2392-2395` | `b` values |
| Serialized header size | 180 | `wallet/udb/txmined.go:607`, `dcrd/wire@v1.8.0/blockheader.go:26` | `h` values, fixed-offset field extraction |
| Header field offsets | parent 4, merkle root 36, vote bits 100, height 128, timestamp 136 | `wallet/udb/txmined.go:545-584` | Block record creation, `MainChainTip`, cfilter key |
| Tx record header | 8 (received time) | `wallet/udb/txdb.go:510-526` | `t`/`m` values; script offsets are relative to byte 8 |
| Ticket record size | 4 | `wallet/udb/txdb.go:1720-1724` | `tix` values |
| Ticket "not picked" sentinel | `-1` (`ff ff ff ff`) | `wallet/udb/txunmined.go:122`, `wallet/udb/txmined.go:1335` | `tix` values |
| Multisig value size | 135 (exact) | `wallet/udb/txdb.go:1781,1812` | `ms` values |
| Multisig unspent index sentinel | `0xFFFFFFFF` | `wallet/udb/txdb.go:1909`, `wallet/udb/txmined.go:1820` | `ms[99:103]` |
| `mu` value | `00` | `wallet/udb/txdb.go:1947` | Presence index |
| `up` value | `01` | `wallet/udb/txdb.go:1402` | Unpublished marker |
| Ticket commitment value size | 12 | `wallet/udb/txdb.go:2025-2030` | `cmt` values |
| `cmu` flag `unminedSpent` | `0x01` | `wallet/udb/txdb.go:2079-2089` | `cmu` values |
| CFilter key size (`gcs.KeySize`) | 16 | `dcrd/gcs/v4@v4.1.1/gcs.go:69` | `cf` value prefix |
| `blockcf2.B` | 19 | `dcrd/gcs/v4@v4.1.1/blockcf2/blockcf.go:33` | Filter decoding |
| `blockcf2.M` | 784931 | `dcrd/gcs/v4@v4.1.1/blockcf2/blockcf.go:38` | Filter decoding |
| Birth state size | 45; flags `0x01` set-from-height, `0x02` set-from-time | `wallet/udb/txmined.go:341-363` | `birthstate` |
| `accountExistsMask` | `0x80` | `wallet/udb/txdb.go:63` | Byte 81 of credits, byte 9 of unmined credits |
| `scriptType` | 0 nonexisting, 1 unspecified, 2 P2PKH, 3 P2PK, 4 P2PKH-alt, 5 P2PK-alt, 6 P2SH, 7 stake P2PKH, 8 stake P2SH | `wallet/udb/txdb.go:74-112` | Low 7 bits of the same bytes |
| `scriptLocNotStored` | 0 | `wallet/udb/txdb.go:118` | Script offset meaning "deserialize the transaction" |
| Credit flag masks | spent `0x01`, change `0x02`, stake tag `0x1c`, coinbase `0x20`, has-expiry `0x40` (db v8: `0x10`) | `wallet/udb/txdb.go:699-711, 771-789, 895-904` | Byte 8 of credits and unmined credits |
| Opcode tag base | `OP_NOP10 = 0xb9`; `OP_SSTX 0xba`, `OP_SSGEN 0xbb`, `OP_SSRTX 0xbc`, `OP_SSTXCHANGE 0xbd`, `OP_TGEN 0xc3` | `dcrd/txscript/v4@v4.1.2/opcode.go:238-248`, `wallet/udb/txmined.go:35` | `condenseOpCode` / `expandOpCode` |
| Multisig flag masks | spent `0x01`, stake tree `0x02` | `wallet/udb/txdb.go:1788-1794` | `ms[22]` |
| `hasExpiryVersion` | 8 | `wallet/udb/upgrades.go:78` | Has-expiry in bit 4 |
| `hasExpiryFixedVersion` | 9 | `wallet/udb/upgrades.go:85` | Has-expiry in bit 6 |
| `DBVersion` | 26 | `wallet/udb/upgrades.go:213` | Passed as `dbVersion` by every caller of the credit codecs |
| `initialVersion` | 1 | `wallet/udb/upgrades.go:31` | Unified version written right after `createStore` |
| Legacy store versions | 1, 2, 3 (root `vers`) | `wallet/udb/txdb.go:2298-2326` | `upgradeTxDB` |
| `wire.NoExpiryValue` | 0 | `dcrd/wire@v1.8.0/msgtx.go:40` | Expiry pruning, has-expiry flag |
| `wire.TxTreeRegular` / `TxTreeStake` | 0 / 1 | `dcrd/wire@v1.8.0/msgtx.go:64-68` | Multisig tree flag |
| `stake.TxType` | 0 regular, 1 SStx, 2 SSGen, 3 SSRtx, 4 TAdd, 5 TSpend, 6 TreasuryBase | `dcrd/blockchain/stake/v5@v5.0.3/staketx.go:31-37` | Branches in 5.8, 5.11, 5.13 |
| `chainhash.HashSize` | 32 (also asserted at compile time, `wallet/udb/txdb.go:129`) | `dcrd/chaincfg/chainhash@v1.0.5/hash.go:14` | Every hash field; the per-transaction stride of block records (`wallet/udb/txdb.go:317,331`) |
| `ripemd160.Size` | 20 | `dcrd/crypto/ripemd160@v1.0.2/ripemd160.go:22` (used at `wallet/udb/txdb.go:1777,1849`) | Script-hash field `ms[0:20]` |
| Block record pre-count prefix | 42 (hash 32 + time 8 + vote bits 2) | `wallet/udb/txdb.go:249,258,2394` | Position of the invalidation byte; split point of the legacy widening |
| Reverse-iterator seek key | `ff ff ff ff` | `wallet/udb/txdb.go:364-365` | `makeReverseBlockIterator` |
| bbolt `MaxKeySize` | 32,768 bytes | `bbolt@v1.3.12/bucket.go:11` | Upper bound on `be32(version)‖agendaID` keys |
| bbolt `MaxValueSize` | 2³¹ − 2 bytes | `bbolt@v1.3.12/bucket.go:14` | Upper bound on any value |

There are no timeouts, retry counts, cache sizes or tunables in scope.

---

## 7. Concurrency and lifecycle

- **No concurrency primitives.** The five files declare no goroutine, channel, mutex, atomic or
  context use. `Store` methods are not internally synchronized.
- **Serialization comes from the database.** bbolt allows one read-write transaction at a time
  and any number of concurrent read-only snapshot transactions. Every mutating function in scope
  takes a `ReadWriteTx`/`ReadWriteBucket`, so mutations are totally ordered by the writer lock;
  readers see a consistent snapshot as of their transaction start.
- **Borrowed bytes.** Slices returned by `Get`, cursors and `ForEach` alias the memory-mapped
  database and are valid only until the transaction ends. Helpers that hand such slices to
  callers without copying: all `exists*`, `extract*`, `fetchRawCFilter2`, `latestTxRecord`,
  iterator `ck`/`cv`. Helpers that copy: `fetchRawBlockHeader`, `existsMultisigOutCopy`,
  `fetchRawTxRecordPkScript` (stored-offset path), `CFilterV2`. A Rust port expresses this with
  lifetimes; the copy points matter only where the Go mutates the result in place (`ms` values)
  or lets it escape the transaction (headers, filters).
- **Write-while-iterating.** The code never writes to a bucket while a cursor on that same bucket
  is being advanced, except `Rollback`, which re-seeks with `blockIterator.reposition` after each
  block and defers row deletion until after the loop. `PruneUnmined` and `upgradeToVersion3`
  collect first and mutate afterwards. `RemoveUnconfirmed` uses point lookups only.
- **Recursion.** `RemoveUnconfirmed` recurses once per dependent unmined transaction. Go grows
  goroutine stacks on demand; a port should use an explicit work list or otherwise guarantee
  equivalent depth.
- **Caller-held locks and transaction boundaries** (observed, not owned by this scope):
  - `PruneUnmined` runs inside the chain-extension `Update` of `wallet/chainntfns.go` with
    `w.lockedOutpointMu` held; removed hashes are sent to transaction notification clients and
    VSP clients inside that same closure.
  - `InsertMemPoolTx` runs inside the `Update` that also records credits for the transaction
    (`processTransactionRecord`, `insertIntoTxMgr`).
  - `RemoveUnconfirmed` runs in its own `Update` from `Wallet.AbandonTransaction`, which first
    requires the transaction to be unmined (`details.Block.Height == -1`) and fails with
    `Invalid` `"transaction <hash> is mined in main chain"` otherwise.
  - `SetPublished`, `ImportCFiltersV2` and `SetAgendaChoices` each use their own `Update`.
  - `UnminedTxs` and the agenda getters run in `View` transactions (the agenda getters also
    inside `Update` when re-reading after a write). `CFilterV2` runs in `View` transactions from
    `Wallet.CFilterV2` / `RangeCFiltersV2` (`wallet/wallet.go:836,858`) and
    `fetchMissingCFilters` (`wallet/wallet.go:1323`), and inside `Update` transactions in the
    chain-switch closure (`wallet/chainntfns.go:134`) and in `fetchMissingCFilters`
    (`wallet/wallet.go:1395-1396`).
- **Lifecycle.** `createStore` runs exactly once per database, at creation. `upgradeTxDB` runs
  only during migration of a pre-unified database. Nothing in scope runs at open or shutdown;
  there is no in-memory cache to flush.
- **Ordering guarantees visible to callers.** `UnminedTxHashes`: ascending hash bytes.
  `UnminedTxs`: parents before children, otherwise arbitrary. `PruneUnmined`: selection and
  result in ascending hash bytes. Iterators: ascending key order (`b` by height; credits and
  debits by output/input index within one transaction incidence).

---

## 8. Errors

All errors are `*errors.Error` values from the wallet `errors` package unless noted. The kinds
used in scope and their string forms (`errors/errors.go:52-121`):

| Kind | String | Used for |
| --- | --- | --- |
| `IO` | `I/O error` | Malformed or missing database content, and every wrapped driver `Put`/`Delete`/`CreateBucket` failure |
| `NotExist` | `item does not exist` | Missing header (`"block header"`), missing filter, unknown import height |
| `Exist` | `item already exists` | Unmined insert of a transaction that is already mined |
| `DoubleSpend` | `double spend` | Unmined insert that conflicts with another unmined transaction |
| `Invalid` | `invalid operation` | Transaction serialization failure in `valueTxRecord`; driver errors such as empty key |

Message catalogue (format verbs as in the Go source):

| Message | Kind | Raised by |
| --- | --- | --- |
| `mined balance len %d` | IO | `fetchMinedBalance` |
| `outpoint len %d` | IO | `readCanonicalOutPoint` |
| `block record len %d` | IO | `appendRawBlockRecord`, `fetchBlockTime`, `readRawBlockRecord` (prints the key length there) |
| `block key len %d` | IO | `readRawBlockRecord` |
| `%d tx block record len %d` | IO | `readRawBlockRecord` |
| `block header` | NotExist | `fetchRawBlockHeader` |
| `tx record len %d` | IO | `readRawTxRecordMsgTx`, `readRawTxRecord` |
| `tx record key len %d` | IO | `readRawTxRecordHash/BlockHeight/Block` (plus stack trace on stderr) |
| `missing transaction output for credit index` | IO | `fetchRawTxRecordPkScript` |
| `invalid script offset %d for tx %v` | IO | `fetchRawTxRecordPkScript` |
| `credit len %d` | IO | credit amount/account readers, `creditIterator` |
| `credit key len %d` | IO | `fetchRawCreditUnspentValue`, `creditIterator` |
| `credit account unset` | IO | `fetchRawCreditAccount` |
| `unspent len %d` | IO | `readUnspentBlock` |
| `debit len %d` | IO | `existsDebit`, `existsInvalidatedDebit`, `debitIterator` |
| `debit key len %d` | IO | `debitIterator` |
| `unmined key len %d` | IO | `readRawUnminedHash` |
| `unmined len %d` | IO | `fetchRawUnminedReceiveTime` |
| `unmined credit key len %d` | IO | `fetchRawUnminedCreditIndex` |
| `unmined credit len %d` | IO | unmined credit amount/account readers |
| `unmined credit account unset` | IO | `fetchRawUnminedCreditAccount` |
| `delete called from read-only cursor` | IO | `readCursor.Delete` |
| `multisig output key len %d` (literal `%d`) | IO | `fetchMultisigOut` |
| `multisig output len %d` (literal `%d`) | IO | `fetchMultisigOut` |
| `no cfilter saved for block %v` | NotExist | `fetchRawCFilter2` |
| `cfilter data len: %d` | IO | `fetchRawCFilter2` |
| `ticket commitment amount %d` | IO | `fetchRawTicketCommitmentAmount` |
| `ticket commitment account %d` | IO | `fetchRawTicketCommitmentAccount` |
| `wrong unspent ticket commitment len %d` | IO | `unspentTicketCommitsIterator.next` |
| `bucket is not empty` | IO | `createStore` |
| `no transaction store version` | IO | `upgradeTxDB` |
| `invalid transaction store version 0` | IO | `upgradeTxDB` |
| `transaction already exists mined` | Exist | `InsertMemPoolTx` |
| `vote or revocation %v double spends unmined vote %v on the tip block` | DoubleSpend | `InsertMemPoolTx` |
| `%v conflicts with %v by double spending %v` | DoubleSpend | `InsertMemPoolTx` |
| `block height of %d unknown` | NotExist | `ImportCFiltersV2` |

Errors that are **not** wrapped in a wallet kind by this scope:

- `gcs.Error` (`ErrMisserialized`, message `failed to read number of filter items: …`) from
  `CFilterV2` and `ImportCFiltersV2`.
- Driver errors from `putRawCFilter` and `deleteBlockRecord` (they arrive already converted by
  the bbolt driver: `Invalid` for closed/read-only transaction, empty/oversized key or bucket
  collision; `Other` otherwise; `wallet/internal/bdb/db.go:17-36`).
- Wire deserialization errors keep their text inside an `IO` wrapper.

How they surface:

- **Error text.** `Error()` renders `Op: Kind: message` with nested wallet errors joined by the
  separator, which the daemon sets to `":: "` at start-up (`dcrwallet.go:38`; the library
  default is `":\n\t"`). Wallet-level callers add an `Op` such as `wallet.ImportCFiltersV2`.
- **JSON-RPC.** The generic conversion maps unclassified kinds, including `IO`, `NotExist`,
  `Exist`, `DoubleSpend` and `Invalid`, to code `ErrRPCWallet` with the full error string as
  message (`internal/rpc/jsonrpc/errors.go:16-49`). `importcfiltersv2` instead returns
  `ErrRPCInvalidRequest` with the error string (`internal/rpc/jsonrpc/methods.go:1998-2000`);
  `getcfilterv2` returns the wallet error through the generic conversion.
- **Logs.** `InsertMemPoolTx`'s `Exist` is caught by `processTransactionRecord` and logged at
  Warn (`Refusing to add unmined transaction <hash> since same transaction already exists mined`,
  `wallet/chainntfns.go:451-456`) and is not an error for that caller. The other caller,
  `insertIntoTxMgr` (`wallet/createtx.go:258-261`), does not test for it and returns the error.
  `PruneUnmined` failures are logged at Error and swallowed.
- **Panics**: unchecked slicing listed in
  quirk Q2, the genesis-header serialization failure in `createStore`/`upgradeToVersion3`, and nil
  top-level buckets in the agenda helpers. `walletdb.Update` does not recover; its deferred
  function rolls the transaction back and the panic propagates
  (`wallet/walletdb/interface.go:240-250`). The outcome then depends on the goroutine: an
  unrecovered panic aborts the process with a Go runtime trace and exit status 2, but a JSON-RPC
  request received by HTTP POST runs its handler synchronously on the `net/http` serving
  goroutine (`internal/rpc/jsonrpc/server.go:595`), where the Go standard library recovers the
  panic, logs the stack and drops the connection without ending the process. UNVERIFIED which
  of the two applies on each websocket, gRPC and sync code path; settled by tracing the
  goroutine that runs each caller (the only `recover()` in the repository, `dcrwallet.go:180`,
  re-panics).
- **stderr side channel.** `readRawTxRecordHash`, `readRawTxRecordBlockHeight` and
  `readRawTxRecordBlock` print the current goroutine's stack with `debug.PrintStack()` before
  returning their `IO` error.

Log lines emitted by code in scope (subsystem tag `WLLT`; `udb` shares the wallet logger,
`log.go:33`):

| Level | Text |
| --- | --- |
| Info | `Inserting unconfirmed transaction <hash>` |
| Info | `Removing expired unmined transaction <hash>` |
| Info | `Removing old unmined ticket purchase <hash>` |
| Info | `Removing missed or invalid vote <hash>` |
| Debug | `Removing double spending transaction <hash>` |
| Debug | `Transaction <hash> is part of a removed conflict chain -- removing as well` |
| Debug | `Removing unconfirmed ticket commitment <hash>:<index>` |
| Debug | `Marking ticket commitment <hash>:<index> unmined spent as <bool>` (from `replaceTicketCommitmentUnminedSpent`, called by `RemoveUnconfirmed`) |

---

## 9. Compatibility-critical behaviours

Each item is something an existing `wallet.db`, a Go dcrwallet opening the same file later, or an
RPC client depends on.

1. **Bucket and key names are exact byte strings.** The namespace is `wtxmgr`; nested buckets are
   `b h t c u d m up mc mi tix ms mu ic id cf cmt cmu`; root keys are `date bal tip havecfilters
   lasttxsblock birthstate`; agenda buckets are `agendaprefs` and `ticketsagendaprefs`. No
   bucket may be added, renamed or left out at database version 26.
2. **All store integers are big-endian**, including heights, indexes, amounts, times, counts,
   account numbers and the deployment version in agenda keys.
3. **Fixed key shapes**: 4 (`b`), 32 (`h`, `m`, `up`, `tix`, `cf`), 36 (`u`, `mc`, `mi`, `ms`,
   `mu`, `cmt`, `cmu`), 68 (`t`), 72 (`c`, `ic`, `d`, `id`). Go dcrwallet slices these at fixed
   offsets without length checks in many places, so a deviating key crashes it.
4. **Fixed value shapes** as in section 4.5. In particular: block record 47 + 32·n with the
   invalidation byte at 42; credit 94 bytes; unmined credit 22 bytes; debit 80 bytes; unspent 36
   bytes; multisig exactly 135 bytes; ticket commitment 12 bytes; `tix` 4 bytes `ff ff ff ff`;
   `up` = `01`; `mu` = `00`; `cmu` = `00`/`01`; `bal` exactly 8 bytes; `havecfilters` exactly 1
   byte with value `00` or `01`; `birthstate` exactly 45 bytes.
5. **Credit flag bits**: spent `0x01`, change `0x02`, stake opcode tag in `0x1c` with the exact
   code points of 4.5.4 (including `0x1c` for `OP_TGEN`), coinbase `0x20`, has-expiry `0x40`.
   Bit 7 of the script-type byte marks a valid account field.
6. **Script offsets** in credits are byte offsets into the serialized transaction as stored
   (not including the 8-byte time prefix), computed with the full-serialization layout. Offset 0
   means "not stored". A wrong offset makes Go return the wrong script or `invalid script
   offset`.
7. **Transaction bytes are stored verbatim** from the caller-provided serialization, preceded by
   the received time in whole seconds. Do not re-serialize or normalize; other rows index into
   these bytes.
8. **Legacy rows must be readable**: credits shorter than 94 bytes (≥ 9), unmined credits of 9
   bytes, credits whose account-valid bit is clear, and script offset 0. When such a credit is
   spent or unspent it is rewritten zero-extended to 94 bytes.
9. **The unspent index, debit rows and the spent bit move together** (invariants I4, I5), and
   `bal` follows invariant I9. Go dcrwallet computes balances and selects coins from `u` and
   `bal` without re-deriving them.
10. **Spends by unmined transactions are represented only by `mi` rows.** Mined credits keep
    their `u` row and clear spent bit. Every input of an unmined transaction except a vote's
    stakebase gets an `mi` row; removing the transaction removes them.
11. **Unmined conflict policy**: reject the newcomer with `DoubleSpend`, except that a vote or
    revocation replaces an unmined vote that does not vote on the current tip block; a mined
    transaction always evicts conflicting unmined transactions and their descendants. Peers and
    the ticket buyer/voter depend on this (stale votes are replaced after a reorg).
12. **Idempotent unmined insert**: inserting a transaction already in `m` is a silent no-op;
    inserting one that is already mined yields kind `Exist`, which callers test for.
13. **Ticket set**: every ticket purchase inserted (mined or unmined) gets a `tix` row and it is
    never deleted. `OwnTicket`, `IterateTickets` and `UnspentTickets` (and through them the
    `gettickets`-family RPCs and the voter) depend on stale rows being tolerated, not purged.
14. **Removing an unmined ticket deletes its `cmt` and `cmu` rows**; removing an unmined vote or
    revocation resets `cmu` flags to `00`. Locked/voting-authority balance figures reported over
    RPC are computed from these rows.
15. **Removing an unmined spender of a multisig output** rewrites the `ms` row with the exact
    byte effects of `setMultisigOutUnSpent` and re-adds the `mu` row.
16. **Prune rules and their order** (expiry first, then ticket price when `sdiff != 0`, then
    vote-not-on-tip; revocations and regular transactions only by expiry) and the returned hash list, which drives
    removed-transaction notifications to gRPC clients (Decrediton) and VSP client cleanup.
17. **`UnminedTxs` ordering contract**: parents precede children. It is the order in which
    unmined transactions are rebroadcast to dcrd or SPV peers; a child sent before its parent
    would be rejected as an orphan.
18. **Compact filter rows**: key = block hash; value = first 16 bytes of the header's merkle
    root followed by the unmodified serialized v2 filter. `getcfilterv2` returns exactly these
    bytes (hex) and SPV peers are matched against them. Parameters B = 19, M = 784931.
19. **`importcfiltersv2` semantics**: filters apply to consecutive main-chain heights from the
    start height, overwrite existing rows, require stored headers, are checked only for a
    complete, canonically encoded leading varint, and do not flip `havecfilters`.
20. **Agenda preferences**: key = `be32(version) ‖ agendaID`, value = choice ID; per-ticket
    preferences in a nested bucket named by the raw ticket hash, created on first write. An
    absent row (or empty value) means "no preference". Vote bits produced by the voter and by
    `getvotechoices` are derived from these rows.
21. **Fresh database content** (section 4.6) including `havecfilters = 01`, `lasttxsblock` =
    genesis hash, the genesis rows in `b`, `h`, `cf`, and `birthstate` with the zero-time
    encoding `ff ff ff f1 88 6e 09 00`. A Go dcrwallet must be able to open a store created by
    the replacement and vice versa.
22. **Error kinds** `Exist`, `NotExist`, `DoubleSpend`, `IO`, `Invalid` for the conditions in
    section 8, because callers branch on them (`errors.Is`) and because the error strings reach
    RPC clients verbatim.
23. **Multisig flag byte semantics as implemented** (quirk Q8): readers on both sides must agree
    that the stake-tree bit disappears after the first spend/unspend.
24. **Seconds resolution** of received times and the zero-extension of 32-bit header timestamps
    into the 8-byte block-record time field.
25. **Script metadata survives mining.** The script type, offset and length of an `mc` row are
    carried byte-for-byte into the `c` row when the transaction is mined (4.5.7), including the
    legacy "type 0, offset 0, length 0" shape. A Go dcrwallet reading a `c` row written by the
    replacement locates the output script from these fields.
26. **Log lines** of section 8 (text, level, subsystem `WLLT`, and the order in which
    `RemoveUnconfirmed` emits them: outputs ascending, depth-first, before the transaction's own
    rows are deleted). Operators and test harnesses grep for `Inserting unconfirmed transaction`
    and the three `Removing …` lines.
27. **No balance bookkeeping in the unmined path.** `InsertMemPoolTx`, `RemoveUnconfirmed`,
    `PruneUnmined` and `SetPublished` never write `bal`, `u`, `c` or `d`. A mined credit spent
    by an unmined transaction is distinguishable from an unspent one only through `mi`.
28. **Result shapes on error**: `UnminedTxHashes` returns the partial list with its error,
    `PruneUnmined` returns the hashes removed so far with a removal error and nil with a scan
    error, `UnminedTxs` returns nil. `PruneUnmined`'s caller logs the error and then still walks
    the returned list, sending a removed-transaction notification, a VSP client
    `RemoveTicket(hash, "pruned")` and a `DeleteVSPTicket` for every hash in it
    (`wallet/chainntfns.go:255-273`), so the partial list is observable to gRPC clients.

---

## 10. Quirks

Each entry: what happens, and where. "Reproduce" means byte- or behaviour-compatibility requires
it; "diverge" means a port may deliberately differ and should record that it does.

- **Q1. `vsphostindex` is declared as a `wtxmgr` root key but is not one.** The variable sits in
  the root-key block (`wallet/udb/txdb.go:162`) yet is only read and written in the top-level
  `vsphost` bucket (`wallet/udb/vsp.go:53-65`). Reproduce the real location.
- **Q2. Unchecked slicing panics on inconsistent data.** `extractRawUnminedTx(nil)` panics; it is
  reached with a possibly-nil value in `InsertMemPoolTx` when an `mi` row names a transaction
  that is not in `m` (`wallet/udb/txunmined.go:55-56`) and in `Store.UnspentTickets`
  (`wallet/udb/txmined.go:2681`), and with a too-short value in `PruneUnmined`
  (`wallet/udb/txunmined.go:359`). `readRawUnminedInputSpenderHash` panics on an `mi` value
  shorter than 32 bytes. `extractRawBlockRecordHash`, `extractRawBlockRecordStakeInvalid`,
  `extractRawCreditSpenderDebitKey`, the `extractRawDebit*` helpers, the `fetchMultisigOut*`
  helpers, `fetchRawCreditTagOpCode`/`fetchRawCreditIsCoinbase` and the header field extractors
  have no length checks. In contrast `removeDoubleSpends` and `RemoveUnconfirmed` report the same
  dangling reference as `IO` `"tx record len 0"`. Diverge: return an `IO`-kind error instead of
  aborting (decision D2).
- **Q3. Unmined credit iteration leaves `OpCode` and `IsCoinbase` unset.** `CreditRecord.OpCode`
  is 0 (not `OP_NOP10`) and `IsCoinbase` is false for every unmined credit, whatever the stored
  flags say (`wallet/udb/txdb.go:1619-1636`). Mined credits report the expanded opcode. Callers
  of `TxDetails` see this difference. Reproduce unless every consumer is audited.
- **Q4. Wrong length in an error message.** `readRawBlockRecord` reports `len(k)` in
  `"block record len %d"` when the *value* is too short (`wallet/udb/txdb.go:312-314`), so the
  message always says 4 or more. Reproduce or fix; only log text is affected.
- **Q5. Unformatted error text.** `fetchMultisigOut` calls `errors.E(errors.IO, "multisig output
  key len %d", len(k))`; `errors.E` does not format, so the message is the literal string
  containing `%d` and the length is dropped (`wallet/udb/txdb.go:1809-1814`,
  `errors/errors.go:187-211`).
- **Q6. `spendCredit` does not check that the credit exists.** For a missing key it writes a
  94-byte row with amount 0, flags `0x01` and the spender key, and returns amount 0
  (`wallet/udb/txdb.go:942-954`). Two of the three callers take the key from a `u` row
  (`wallet/udb/txmined.go:1088-1100`, `:1253-1291`), so there it needs a broken invariant I4 to
  trigger. The third, `stakeValidate` (`wallet/udb/txmined.go:714-728`), takes the key from the
  value of an `id` row without consulting `u` or `c`, so there it triggers whenever the credit
  named by a stake-invalidated debit is no longer in `c`. UNVERIFIED whether that state is
  reachable; settled in the mined-transaction spec.
- **Q7. `unspendRawCredit` leaves the old spender key in place.** Only the spent bit is cleared;
  bytes `[9:81]` keep the previous debit key (`wallet/udb/txdb.go:959-974`). Likewise `spendCredit`
  and `unspendRawCredit` silently widen legacy short values to 94 bytes. Reproduce: a Go and a
  Rust wallet alternating on one file must produce identical rows.
- **Q8. Multisig spend/unspend clobbers the stake-tree flag.** `setMultisigOutSpent` assigns
  `v[22] = 0x01` and `setMultisigOutUnSpent` assigns `v[22] = 0x00`, discarding bit 1
  (`wallet/udb/txdb.go:1896-1910`). After either, `fetchMultisigOutTree` reports the regular tree.
  Reproduce.
- **Q9. `setMultisigOutUnSpent` zeroes 31 of the 32 spender-hash bytes.** It copies into
  `v[67:98]`, leaving byte 98 (the last byte of the previous spender hash) untouched
  (`wallet/udb/txdb.go:1908`). Reproduce.
- **Q10. `tix` rows are immortal and their value is unused.** No delete helper exists; pruned or
  abandoned unmined tickets keep their row, so `OwnTicket` stays true for them. The picked-height
  field is always `-1` and is read only by a test (`wallet/udb/txdb.go:1715-1746`,
  `wallet/udb/upgrades_test.go:279`). Reproduce.
- **Q11. The duplicate-edge check in `makeGraph` is dead code.** It compares each recorded child
  hash with the *parent* hash and can never match (`wallet/udb/kahnsort.go:42-47`). The sort is
  still correct because in-degrees are over-counted consistently. The output order among
  unrelated transactions follows Go map iteration (random per call). Diverge on the exact order
  (any valid topological order is indistinguishable from Go's); decision D1.
- **Q12. `InsertMemPoolTx` mixes two notions of transaction type.** The conflict exception uses
  the caller-supplied `rec.TxType`; the stakebase skip and the `tix` insert use a freshly
  computed type (`wallet/udb/txunmined.go:53,101`). They agree for records built by
  `NewTxRecord*`. Also, re-inserting an existing unmined transaction with a different
  `Unpublished` value changes nothing.
- **Q13. `PruneUnmined` reporting and failure handling.** The returned list contains only the
  directly selected transactions, not descendants removed with them, so no removal notification
  is sent for those. A mid-way error leaves earlier removals in the (still committed) transaction
  because the wallet ignores the error (`wallet/udb/txunmined.go:400-409`,
  `wallet/chainntfns.go:255-259`). A transaction selected and also removed as someone's descendant
  is processed twice. Reproduce the list contents; the partial-commit behaviour follows from the
  caller.
- **Q14. `RemoveUnconfirmed` trusts its arguments.** It never checks that the transaction is in
  `m`; it deletes `mi` rows for all inputs without comparing the recorded spender; for votes and
  revocations it writes `cmu = 00` for every tracked commitment of the ticket, creating the row
  if a mined spender had already deleted it (`wallet/udb/txunmined.go:238-256`,
  `wallet/udb/txmined.go:1669-1683`). It also deletes an `mi` row for a vote's stakebase outpoint
  (which is never written).
- **Q15. `ImportCFiltersV2` is permissive.** No commitment check, no update of `havecfilters`,
  iterator decode errors are reported as `"block height of %d unknown"`, and filter-parse and
  put errors are returned without a wallet error kind (`wallet/udb/cfilters.go:34-67`).
- **Q16. Stack trace on stderr.** The three `readRawTxRecord{Hash,BlockHeight,Block}` helpers call
  `debug.PrintStack()` on a short key (`wallet/udb/txdb.go:578-604`). Diverge (log instead).
- **Q17. Documentation disagrees with code.**
  - The credit layout comment calls bytes `[81:86]` an "OPTIONAL scriptPk location (5 bytes)"
    and then lists fields through byte 94; the value is always 94 bytes
    (`wallet/udb/txdb.go:717-721`).
  - The unmined-credit comment omits the `OP_TGEN` tag that the code writes
    (`wallet/udb/txdb.go:1435-1440` vs `:1467`).
  - `fetchRawCreditScriptLength` is documented as returning the script offset
    (`wallet/udb/txdb.go:915-916`).
  - `createStore` says it creates the store "with the latest db version"; it creates the
    version-1 unified layout (including the obsolete `sc` bucket and without `tix`, `cf`, `cmt`,
    `cmu`, `up`) and relies on upgrades (`wallet/udb/txdb.go:2168-2169`).
  - The `votingPreferencesUpgrade` comment says it creates `ticketsagendaprefs`; it creates
    `agendaprefs` (`wallet/udb/upgrades.go:419-429`).
  - The multisig layout comment lists the flags as "[0]: Spent, [1]: Tree"; these are bit
    positions inside byte 22, not byte offsets (`wallet/udb/txdb.go:1759-1762`).
  - The package-level TODO admits "only about 1/2 of functions are properly commented"
    (`wallet/udb/txdb.go:56-58`).
- **Q18. Time encodings.** Received times are truncated to seconds on write. A zero `time.Time`
  is stored as the negative Unix second count `ff ff ff f1 88 6e 09 00`; tests insert records
  with a zero received time, and the version-26 upgrade stores a zero birthday time. Decoding
  yields a time in the local zone. Reproduce the bytes.
- **Q19. Dead version branches.** The `dbVersion` parameter of `valueUnspentCredit`,
  `valueUnminedCredit` and `fetchRawCreditHasExpiry` selects bit 4 for version 8 and "no flag"
  below 8. Every call site passes 26. Only the upgrade functions need the bit-4 meaning, and they
  manipulate the bits directly (`wallet/udb/upgrades.go:666,703,760-768,807-820`).
- **Q20. `condenseOpCode` relies on uint8 wraparound.** For any opcode other than `OP_TGEN` it
  computes `(opCode - 0xb9) << 2` with wrapping arithmetic and no mask; only the six opcodes
  from `getStakeOpCode` are ever passed, all of which map inside `0x1c`
  (`wallet/udb/txdb.go:743-752`).
- **Q21. Has-expiry is lost when an unmined credit is mined** (cross-reference; the code is in
  `txmined.go`). `moveMinedTx` rebuilds the mined credit from a `credit` struct whose `hasExpiry`
  is never assigned, so bit 6 is clear in the resulting `c` row even if the `mc` row had it
  (`wallet/udb/txmined.go:1133-1181`). The stake-invalidated path of `AddCredit` computes the
  flag as `Expiry != 0` directly (`wallet/udb/txmined.go:1417`). Byte compatibility requires
  reproducing both; flag it in the mined-transaction spec.
- **Q22. Inconsistent error wrapping.** `deleteBlockRecord` and `putRawCFilter` return the driver
  error as is while every sibling wraps in `IO`; the exported agenda setters wrap everything in
  `IO`, including "bucket name required" style driver errors.
- **Q23. `existsUnpublished` is stricter than the writer.** It requires exactly one non-zero
  byte; a zero byte or longer value means "published" (`wallet/udb/txdb.go:1409-1412`). Only
  `01` is ever written.
- **Q24. nil versus empty results.** `UnminedTxHashes` returns a nil slice for an empty set;
  `PruneUnmined` returns a non-nil empty slice on success and nil on a scan error;
  `dependencySort` returns a non-nil empty slice for an empty set. Matters only where a result is
  marshalled (e.g. JSON `null` vs `[]`) by a caller.
- **Q25. `valueTxRecord` assumes `SerializeSize` is exact.** It serializes into a
  pre-sized slice through a `bytes.Buffer`; if the size were under-reported the buffer would
  reallocate and the stored value would hold only the bytes written before the reallocation
  followed by zeros (an over-report would leave trailing zeros) (`wallet/udb/txdb.go:512-519`). Not
  reachable with a correct `wire` implementation; do not copy the technique.
- **Q26. `readRawTxRecord` does not verify the hash.** `rec.Hash` is whatever the caller derived
  from the key; the stored transaction is not re-hashed (`wallet/udb/txdb.go:561-576`).
- **Q27. Iterator error paths differ.** `blockIterator.next` drops the cursor without closing it
  on a decode error, `prev` closes it, and the prefix iterators keep the cursor usable after an
  error (`wallet/udb/txdb.go:386-391, 424-430, 1069-1073`). Harmless with bbolt (close is a
  no-op).
- **Q28. Dead test code.** `txexample_test.go` and `txquery_test.go` both carry
  `//go:build ignore` (line 6 of each), so nothing in them is compiled or run:
  `ExampleStore_Rollback` and `Example_basicUsage` (`wallet/udb/txexample_test.go:19,107`, which
  additionally take a `*testing.T` parameter and so would not qualify as examples), and
  `TestStoreQueries` and `TestPreviousPkScripts` (`wallet/udb/txquery_test.go:239,552`), which
  are written against an older `Store` API.
- **Q29. `InsertMemPoolTx` can fail after evicting.** A vote or revocation that replaces a stale
  unmined vote through one input and then conflicts through a later input returns `DoubleSpend`
  with the eviction already applied to the open database transaction
  (`wallet/udb/txunmined.go:43-83`). Only the callers' rollback hides it (section 5.8).
  Reproduce the order of operations; do not pre-validate all inputs before evicting unless the
  caller-rollback guarantee is kept.
- **Q30. More documentation that disagrees with the code** (extends Q17).
  - The comment on `scriptTypeSP2SH` names it `scriptTypeP2SH`
    (`wallet/udb/txdb.go:109-111`).
  - The iterator usage comments call constructors that do not exist (`makeCreditIterator`,
    `makeDebitIterator`, `makeUnminedCreditIterator`; the real ones are `makeRead…Iterator` and
    take more arguments) and advertise `it.delete()`, which is commented out
    (`wallet/udb/txdb.go:1003-1013, 1283-1293, 1580-1589, 1662-1669`).
  - `PruneUnmined` says of `stake.SSGenBlockVotedOn` "This will never actually error"; the
    discarded second result is the voted block height, not an error
    (`wallet/udb/txunmined.go:375-376`, `dcrd/blockchain/stake/v5@v5.0.3/staketx.go:466`).
  - The compact filter comment calls the 16-byte key "the merkle root of the block"; it is the
    first 16 bytes of it (`wallet/udb/txdb.go:1977-1978`, `wallet/udb/cfilters.go:55-57`).
  - `valueBlockRecordStakeValidated` / `…Invalidated` are documented as setting the "stake
    validated byte" to zero / one; the byte at 42 is the *invalidated* flag
    (`wallet/udb/txdb.go:245-262`).
  - `upgradeTxDB`'s comment refers to a `namespaceKey` parameter that the function does not
    have (`wallet/udb/txdb.go:2295-2298`).
  - The `mi` bucket comment says it maps previous outputs spent "for both mined and unmined
    transactions"; it records only spends made **by** unmined transactions, of outputs that may
    themselves be mined or unmined (`wallet/udb/txdb.go:1678-1680`).
  - `InsertMemPoolTx` is documented as marking previous outputs "as spent"; it writes `mi` rows
    only and leaves credits, `u` and `bal` alone (`wallet/udb/txunmined.go:19-22`).
  - `UnminedTxs` promises records "sorted by their dependency order"; the order among
    independent records is random (Q11).
- **Q31. Platform-width arithmetic.** `readRawBlockRecord` computes `47 + 32·int(be32)` and
  `fetchRawTxRecordPkScript` computes `int(scrLoc) + 8` and `int(scrLoc) + 8 + int(scrLen)` in
  Go `int` (`wallet/udb/txdb.go:316-317, 631-637`). With a 64-bit `int` these cannot overflow and
  a corrupt count or offset yields the `IO` errors of section 8; with a 32-bit `int` they can
  wrap and pass the bounds check. A port must do the arithmetic in at least 64 bits (or checked
  `usize`) and keep the error outcome of the 64-bit build.
- **Q32. Multisig mutators assume 135 bytes.** `RemoveUnconfirmed` copies an `ms` value at its
  stored length (`make([]byte, len(v))`, not 135) before `setMultisigOutUnSpent`, so a stored
  value shorter than 103 bytes panics there (`wallet/udb/txunmined.go:259-263`,
  `wallet/udb/txdb.go:1904-1910`). The `txmined.go` paths use `existsMultisigOutCopy`, which
  pads to 135. Same class as Q2; a value of the wrong length is never written by code at the pin.
- **Q33. Unfinished work noted in the source.** Three `TODO`s: replace magic numbers with
  cursor-style offsets and finish comments (`wallet/udb/txdb.go:56-58`); drop the redundant time
  and vote bits from block records "in a future update" (`wallet/udb/txdb.go:219-220`; not done
  through database version 26, so both fields must still be written); credit-amount accounting
  for unmined inserts (`wallet/udb/txunmined.go:130-131`). The two commented-out cursor `delete`
  methods cite `boltdb/bolt` issue 620 (`wallet/udb/txdb.go:442-449, 1662-1669`). No `XXX`,
  `BUG`, `FIXME` or deprecation marker exists in the five files.
- **Q34. One helper is test-only.** `extractRawTicketPickedHeight` has no non-test caller
  (section 3.6). `putTicketRecord` and `fetchBlockRecord` survive only for old upgrades.
- **Q35. `appendRawBlockRecord` appends at the end of the value, not at the end of the hash
  list.** The new hash is placed at `len(v)` and the count at `[43:47]` is incremented
  (`wallet/udb/txdb.go:266-274`). For a record that carries bytes beyond `47 + 32·n` (which
  readers accept and ignore, 4.5.1), hash slot `n` of the new record starts at the old excess
  bytes, so readers decode those bytes (followed, if the excess was shorter than 32 bytes, by the
  leading part of the appended hash) as transaction hash `n`, and the appended hash itself is
  misaligned or lies beyond the counted list.
  No writer at the pin produces excess bytes, so this needs a foreign or corrupt row. The count
  is incremented in wrapping `uint32` arithmetic. Reproduce (append at the end) for byte parity.
- **Q36. Errors of the unspent-commitment iterator are lost.** `unspentTicketCommitsIterator`
  overwrites `err` on every step and its only consumer never reads it after the loop (5.4,
  `wallet/udb/txdb.go:2143-2158`, `wallet/udb/txmined.go:3519-3544`). A `cmu` row with an empty
  value, or a `cmu` key with no (or a short) `cmt` row (violating I11), makes `balanceFullScan`
  stop adding ticket commitments at that row and return balances without them and without an
  error. The message `"ticket commitment amount %d"` cannot be produced. Reproduce the silent
  truncation or surface the error; either way record the decision, because the affected figures
  (`LockedByTickets`, `Total`) reach RPC clients.
- **Q37. A missing nested bucket is a nil-pointer panic, not an error.** Every helper calls
  `ns.NestedReadBucket(name)` / `NestedReadWriteBucket(name)` and immediately invokes `Get`, `Put`,
  `Delete`, `ForEach` or a cursor constructor on the result. The driver returns a nil interface
  for an absent bucket (`wallet/internal/bdb/db.go:100-111`), so a `wtxmgr` namespace lacking any
  of the 18 nested buckets crashes on first use of that bucket; the same holds for the namespace
  itself (`dbtx.ReadWriteBucket(wtxmgrBucketKey)` returns nil when `wtxmgr` is absent,
  `wallet/internal/bdb/db.go:49-55`). No helper in `txdb.go`, `txunmined.go` or `cfilters.go`
  checks; the only nil check on a bucket in scope is the per-ticket bucket in `ticketPreference`
  (`wallet/udb/agendas.go:55-57`). Same class as Q2; a port should report `IO`.
- **Q38. The `mi` value is decoded three different ways.** `InsertMemPoolTx` uses
  `readRawUnminedInputSpenderHash` (slices `v[:32]`, panics on a short value,
  `wallet/udb/txunmined.go:47-48`). `removeDoubleSpends` and `RemoveUnconfirmed` instead use the
  raw value bytes, at whatever length they have, as the lookup key into `m` and build the hash
  with `copy` (short values are zero-padded, long ones truncated, no panic,
  `wallet/udb/txunmined.go:153-157, 191-195`). For a well-formed 32-byte value the three agree.
- **Q39. Further documentation that disagrees with the code** (extends Q17, Q30).
  - The comment on `makeReverseBlockIterator` says it "works just like `makeBlockIterator`"; no
    such function exists (the forward constructor is `makeReadBlockIterator`), and the two differ
    in cursor type: the forward one wraps a read cursor in `readCursor`, the reverse one holds a
    real read-write cursor (`wallet/udb/txdb.go:354-368`).
  - `fetchRawCreditTagOpCode` is documented as fetching "the compressed OP code"; it returns the
    **expanded** opcode (`0xb9`–`0xbd`, `0xc3`) (`wallet/udb/txdb.go:882-885`).
  - The stake tag field is labelled "P2PKH stake flag" in both layout comments but is set for
    stake script-hash outputs as well (`wallet/udb/txdb.go:703, 1435`,
    `wallet/udb/txmined.go:1436-1452`).
  - The mined-balance comment says `bal` is "the total balance for all unspent credits from mined
    transactions" (`wallet/udb/txdb.go:166-170`); per invariant I9 ticket submission outputs are
    excluded (subject to U3).
  - The `cmu` comment refers to "the bucketTicketCommitment"; the variable is
    `bucketTicketCommitments` (`wallet/udb/txdb.go:2071-2072`).
  - The unmined-credit size constants call the 9-byte and 22-byte shapes "version 1" and
    "version 2" (`wallet/udb/txdb.go:1452-1458`); these are not database versions and no version
    tag is stored, the shape is told apart by length alone.
  - `RemoveUnconfirmed`'s comment at the ticket-commitment cleanup notes that it reuses the `mc`
    key as the `cmt`/`cmu` key and "assumes" both are canonical outpoints
    (`wallet/udb/txunmined.go:218-221`); `keyTicketCommitment` is not called there.

---

## 11. Upstream tests

All in `wallet/udb` unless noted. None of the tests asserts raw bytes of the formats directly;
they pin behaviour through the store API and through old-database fixtures.

| Test file | What it exercises in this scope | Port / regenerate |
| --- | --- | --- |
| `tx_test.go` `TestInsertsCreditsDebitsRollbacks` | `InsertMemPoolTx`, mined insert of an existing unmined tx, rollback to unmined, `UnminedTxs` membership, spendable/unconfirmed balances and the unspent set after each step. Uses records with a zero received time | Port as an API-level test. Also dump the namespace after each step from a Go oracle and compare bytes |
| `tx_test.go` `TestCoinbases` | Coinbase credits, maturity, spending and rollback; exercises credit flag `0x20`, `u`, `bal` | Port |
| `txquery_test.go` `TestStoreQueries` | **Not compiled or run at the pin**: the file carries `//go:build ignore` (`wallet/udb/txquery_test.go:6`) and uses a stale API (`s.Rollback(height)`, three-argument `s.RangeTransactions(…)`, `wallet/udb/txquery_test.go:100,284`). Its intent: `TxDetails`/`RangeTransactions` over mined and unmined records, credit and debit iterators, `mi`-based spent detection, duplicate transaction hashes | Do not treat as coverage. Rewrite against the current API if its scenarios are wanted, and generate expectations from a Go oracle |
| `txquery_test.go` `TestPreviousPkScripts` | **Not compiled or run** (same build-ignored file). Its intent: `fetchRawTxRecordPkScript` through `PreviousPkScripts` for mined and unmined credits | As above |
| `stakevalidation_test.go` `TestStakeInvalidationOfTip` | Block-record invalidation byte, `ic`/`id` movement (uses `InsertMemPoolTx` as setup) | Port (mined-tx spec owns it) |
| `txmined_test.go` `TestSetBirthState` | `birthstate` round trip with random values | Port; add a fixed vector for the 45-byte layout |
| `upgrades_test.go` `TestUpgrades` + `testdata/v5.db.gz` (`verifyV6Upgrade`) | `tix` bucket contents and `-1` picked height; removal of the stakebase `mi` row | Reuse the fixture; it is a real bbolt file |
| `upgrades_test.go` + `testdata/v7.db.gz` (`verifyV8Upgrade`) | Has-expiry bit readable with `fetchRawCreditHasExpiry(v, DBVersion)` on `c` and `mc` after upgrades 8 and 9 | Reuse the fixture |
| `upgrades_test.go` + `testdata/v11.db.gz` (`verifyV12Upgrade`) | `cmt`/`cmu` creation and resulting per-account balances for unmined/mined tickets, votes, revocations | Reuse the fixture |
| `testdata/v1.db.gz`, `v2.db.gz`, `v3.db.gz`, `v4.db.gz` (`verifyV2Upgrade` … `verifyV5Upgrade`), `v24.db.gz`, and the generators beside them (`v1.db.go` … `v5.db.go`, `v7.go`, `v11.db.go`, `v24.db.go`) | Old-format databases for the upgrade path; generators document how each was built (they target old module versions and are not compiled). `v24.db.gz` is an empty version-24 database that no test at the pin opens | Reuse as read fixtures for the legacy value shapes |
| `txexample_test.go` | Not compiled (`//go:build ignore`; quirk Q28) | Ignore |
| `common_test.go` (support) | `createEmptyDB` (`walletdb.Create("bdb", …)`, `Initialize` then `Upgrade` on testnet3 parameters with a fixed seed and passphrases) and `cloneDB` (file copy of that database, `walletdb.Open`, `Open`), plus `hexToBytes`. `tx_test.go`, `stakevalidation_test.go` and `txmined_test.go` all start from a `cloneDB` copy, so every store test runs against the fresh-database content of section 4.6 on **testnet3**. The template path `emptyDbPath` is set by `TestMain` in `addressmanager_test.go:1251-1258` | Recreate as a fixture builder; the template database is a ready-made check of section 4.6 |
| `txcommon_test.go` (support) | `tempDB`, `blockGenerator.generate(voteBits)` (synthetic chained headers), `makeHeaderData`, `makeHeaderDataSlice`, `emptyFilters(n)`, `makeBlockMeta`, `decodeHash` (reversed-hex to hash) | Port with the tests that use them |
| `internal_test.go`, `addressmanager_test.go`, `cointype_test.go`, `vsp_test.go` | Nothing in this scope (address manager, coin type upgrade, VSP buckets), apart from `TestMain` above | Owned by the sibling specs |
| Other packages | No test outside `wallet/udb` calls `InsertMemPoolTx`, `RemoveUnconfirmed`, `PruneUnmined`, `SetPublished`, `UnminedTxs`/`UnminedTxHashes`, `CFilterV2`, `ImportCFiltersV2` or the agenda preference functions (searched every `_test.go` in the repository) | — |

Untested upstream (needs new tests and Go-oracle vectors):

- Byte-exact encodings of every key and value (no golden vectors exist).
- The credit, debit and unmined-credit iterators and `latestTxRecord` with duplicate hashes
  (the only `wallet/udb` tests that name them are in the build-ignored `txquery_test.go`).
  `fetchRawTxRecordPkScript` is reached only indirectly, through `moveMinedTx`,
  `UnspentOutputs` and the balance code in `tx_test.go`; no test asserts its error paths or
  targets the legacy `scrLoc == 0` path.
- `dependencySort` / `makeGraph` (no unit test; only set membership is checked).
- `PruneUnmined` (expiry, ticket price, vote-on-tip rules; return list).
- The vote/revocation replacement exception and both `DoubleSpend` messages in
  `InsertMemPoolTx`.
- `RemoveUnconfirmed` recursion, ticket-commitment cleanup, multisig unspend.
- `SetPublished` / `up` handling.
- `CFilterV2`, `ImportCFiltersV2` (no test in `wallet/udb`; no wallet-level test found either).
- Agenda preference getters/setters.
- Multisig codecs and the Q8/Q9 byte effects.
- Legacy `upgradeTxDB` (versions 1→3) and `createStore`'s emptiness check.
- Short/legacy credit values and all `IO` error paths.
- `unspentTicketCommitsIterator` error handling (5.4, Q36) and `blockIterator.prev` on a
  forward-constructed iterator (5.3): both are reached only indirectly (balances in
  `tx_test.go` and `verifyV12Upgrade`; `Rollback`), never with a malformed or missing row.
- `appendRawBlockRecord` on a record with trailing bytes (Q35) and `upgradeToVersion3` on short
  block records (5.2).
- Behaviour when a nested bucket is absent (Q37).

Recommended oracle shim: a small Go program linked against dcrwallet at the pin that (a) creates
a store with `udb.Initialize`, (b) replays a scripted sequence of store calls (insert unmined,
add credit, mine, spend, rollback, prune, abandon, multisig add/spend/unspend, import filters,
set agenda choices), and (c) after each step dumps every bucket of `wtxmgr`, `agendaprefs` and
`ticketsagendaprefs` as sorted hex key/value pairs. The Rust tests replay the same script and
diff the dumps. `date` must be masked (wall clock).

---

## 12. Rust porting notes

**Suggested boundary.** One crate mirroring `wallet/udb` (working name `purse-udb`), with this
scope as four modules:

| Module | Mirrors | Contents |
| --- | --- | --- |
| `txdb` (split into `txdb::keys`, `txdb::blocks`, `txdb::records`, `txdb::credits`, `txdb::unmined`, `txdb::multisig`, `txdb::tickets`, `txdb::cfilters`, `txdb::create`) | `txdb.go` | Pure codec functions over byte slices plus thin bucket accessors and the iterators. No business logic |
| `txunmined` | `txunmined.go`, `kahnsort.go` | Unmined set operations and the dependency sort |
| `cfilters` | `cfilters.go` | Filter fetch/import |
| `agendas` | `agendas.go` | Agenda preference storage |

Model each fixed layout as a zero-copy view type (`CreditValue<'a>(&'a [u8])` with accessor
methods and explicit minimum-length checks) and a builder returning `[u8; N]`. Keep the "raw"
helpers the Go uses (`extract*` returning sub-slices) because mined-transaction code passes keys
between buckets by slicing: a debit value contains a credit key which contains a tx-record key.

**Dependencies.**

| Need | Candidate (looked at) | Notes |
| --- | --- | --- |
| bbolt-compatible store with nested buckets, cursors (`First/Last/Next/Prev/Seek`), `ForEach`, `KeyN` | none in dcroxide or dcr-rs (`dcroxide:crates/dcroxide-database/src/lib.rs:1-15` mirrors dcrd's ffldb interface on a `redb` backend; it cannot open a bbolt file) | Must come from the walletdb/bbolt spec. This scope needs: Seek-past-end followed by Prev returning the last key; Get returning "absent" for bucket-valued keys; Delete of a missing key succeeding; Put rejecting empty keys |
| `MsgTx` serialize/deserialize, hashing, `OutPoint` | `dcroxide:crates/dcroxide-wire/src/msgtx.rs` (`MsgTx::from_bytes`, `serialize`, `serialize_size`, `tx_hash`) | No equivalent of `PkScriptLocs` was found there; it must be added (or implemented locally) exactly as `dcrd/wire@v1.8.0/msgtx.go:1125-1157`. `dcr-rs:src/tx.rs` has `parse_full`/`serialize_full` but is a smaller signing-oriented model; prefer dcroxide-wire |
| Block header serialize, field offsets | `dcroxide:crates/dcroxide-wire/src/blockheader.rs` (`serialize() -> [u8; MAX_BLOCK_HEADER_PAYLOAD]`, `from_bytes`) | The store reads raw offsets; keep helper functions over `&[u8; 180]` |
| Stake tx classification | `dcroxide:crates/dcroxide-stake/src/lib.rs` (`determine_tx_type`, `is_sstx`, `is_ssgen`, `ssgen_block_voted_on`, `amount_from_sstx_pk_scr_commitment`) | Verify parity with stake v5.0.3 for treasury types |
| Opcode constants, script classification | `dcroxide:crates/dcroxide-txscript/src/opcodes.rs`, `stdscript.rs` (`determine_script_type`) | Needed by callers that compute the tag and `scriptType`; the codec itself needs only six opcode bytes |
| GCS v2 filter parse, `B`/`M`, key | `dcroxide:crates/dcroxide-gcs/src/lib.rs` (`FilterV2::from_bytes`, `bytes`), `blockcf2.rs` (`B`, `M`, `key`) | `from_bytes` accepts empty input and fails only on an unreadable varint, matching `FromBytesV2` (`dcroxide:crates/dcroxide-gcs/src/lib.rs:497-528`); the error text is the same `failed to read number of filter items: …` prefix |
| Genesis block/hash per network | `dcroxide:crates/dcroxide-chaincfg` | For `createStore` and the fresh-database content |
| Hash type | `dcroxide:crates/dcroxide-chainhash` | Display must be the reversed hex for error/log parity |
| Error kinds | none | A port of the wallet `errors` package (kinds, `Op`, separator) is a prerequisite |

**Hazards.**

- Offsets are easy to get wrong by one: the block-record flag byte at 42, the credit script block
  starting at 81 (type), 82 (offset), 86 (length), 90 (account), the unmined-credit equivalents
  at 9/10/14/18, the multisig layout, and the 31-byte zeroing (Q9).
- Script offsets are relative to the serialized transaction, but slicing is done on the record
  value, hence the `+ 8`.
- `i32` heights are stored as `u32`; `-1` sentinels appear (`tix`, in-memory unmined height).
- Preserve the stale and widened bytes of Q7, the flag clobbering of Q8, the tag code points of
  4.5.4 and the has-expiry loss of Q21, or a database touched alternately by both
  implementations will diverge.
- The Go API leans on nil-means-absent. Use `Option<&[u8]>`; do not conflate an empty stored
  value with absence (bbolt distinguishes them, and `cf` could in principle hold a 16-byte or
  shorter value).
- `RemoveUnconfirmed` recursion: use an explicit stack, keeping the same visiting order
  (outputs ascending, depth-first) so that log order and intermediate states match.
- `dependencySort`: implement Kahn with a deterministic tie-break (decision D1); do not try to
  mimic Go map order.
- Panics in Go (Q2) should become `IO` errors, but then callers that today cannot observe an
  error (e.g. `MainChainTip` has no error return) need a decision on how to surface it.
- Time: store and compare whole seconds; never write sub-second data.

**Size estimate: M.** About 3,100 lines of Go, of which roughly 2,400 are mechanical
codec/accessor code (`txdb.go`) that compresses well into typed views plus exhaustive unit
tests, 400 are the unmined-set logic with a handful of subtle rules, and the rest is small. The
formats are simple and fixed; the effort is in the golden-vector harness and in reproducing the
quirks exactly, not in algorithmic complexity. It becomes L only if the legacy pre-unified
upgrade path and the short legacy value shapes must be fuzz-verified against real old wallets.

---

## 13. Open questions

Decisions needed:

- **D1. `UnminedTxs` order.** Go's order among independent transactions is random. Proposal:
  deterministic Kahn with roots and released nodes ordered by transaction hash bytes. Confirm
  that no consumer relies on per-call randomness (none found).
- **D2. Panics versus errors** for corrupt or inconsistent rows (Q2). Proposal: return `IO`
  errors everywhere; decide what `MainChainTip`-style infallible signatures become.
- **D3. Legacy paths.** Must the replacement open pre-unified databases (`vers` 1–3, no `meta`
  bucket) and run `upgradeTxDB`, or may it refuse them with a clear message and require a pass
  through Go dcrwallet first? The same question applies to unified versions below 26 (owned by
  the upgrades spec) and decides whether `sc`, the version-8 has-expiry bit and 46-byte block
  records ever need to be handled.
- **D4. Quirk fidelity for multisig rows** (Q8, Q9): reproduce exactly (recommended, since a
  user may switch back to Go dcrwallet) or fix and accept divergent bytes in a rarely used
  bucket.
- **D5. `debug.PrintStack` side effect** (Q16) and the unformatted `%d` messages (Q5): reproduce
  the text, or correct it.
- **D6. Error wrapping for filter import** (Q15, Q22): keep the unwrapped `gcs`/driver errors
  (affects only the `importcfiltersv2` error message text) or normalize.

Unverified:

- **U1.** Whether 9-byte and 81-byte mined credit values and 9-byte unmined credit values still
  occur in wallets in the field. The readers handle them; no writer exists at the pin. Settle by
  reading the history of `wtxmgr/db.go` before the script-location fields were added and by
  scanning an old mainnet wallet.
- **U2.** The exact key of the removed `sc` bucket is inferred from the version-14 upgrade
  (delete by HASH160 of the value). Settle by reading the pre-v14 writer (`InsertTxScript` in
  dcrwallet ≤ 1.5) if upgrade support is in scope.
- **U3.** Invariant I9 (definition of `bal`) is derived from the increments and decrements in
  `txmined.go` (`addCredit`, `moveMinedTx`, `InsertMinedTx`, `Rollback`, `stakeValidate`,
  `stakeInvalidate`), which use slightly different tests for "ticket output" (opcode tag in some
  places, transaction type and index in others). The mined-transaction spec should confirm the
  two tests always coincide.
- **U4.** Whether any gRPC or JSON-RPC response exposes `CreditRecord.OpCode` or `IsCoinbase`
  for unmined credits (Q3). Settle in the RPC specs by tracing `TxDetails.Credits` consumers.
- **U5.** Whether dcroxide's varint reader rejects exactly the same non-canonical encodings as
  `wire.ReadVarInt`, and renders the same inner error text, when used by `FilterV2::from_bytes`
  (the outer structure was checked and matches; the inner reader was not read).
- **U6.** bbolt returns a non-nil empty slice for a stored empty value read from a committed
  page but may return nil for an uncommitted `Put(k, nil)` in the same transaction. No code in
  scope stores empty values at database version 26, so this is believed unobservable; the
  walletdb spec should state the rule the Rust store follows.

