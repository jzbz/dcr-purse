# 05 — Database versions, migrations, and the auxiliary buckets (VSP, treasury policy, legacy stake)

## 1. Header

**Scope (dcrwallet, package `wallet/udb`)**

| File | Lines | Content |
| --- | --- | --- |
| `wallet/udb/upgrades.go` | 1727 | Version constants 1..26, the upgrade table, 25 upgrade functions, `Upgrade` |
| `wallet/udb/migration.go` | 73 | `NeedsMigration`, `Migrate` (pre-unified database to unified version 1) |
| `wallet/udb/vsp.go` | 289 | VSP ticket, VSP host and VSP pubkey buckets |
| `wallet/udb/treasury.go` | 264 | Four treasury vote policy buckets |
| `wallet/udb/upgrades_test.go` | 482 | `TestUpgrades` and its verifiers |
| `wallet/udb/testdata/v1.db.go` | 159 | Generator for `v1.db.gz` (127077 bytes) |
| `wallet/udb/testdata/v2.db.go` | 114 | Generator for `v2.db.gz` (2702 bytes) |
| `wallet/udb/testdata/v3.db.go` | 78 | Generator for `v3.db.gz` (2371 bytes) |
| `wallet/udb/testdata/v4.db.go` | 125 | Generator for `v4.db.gz` (5041 bytes) |
| `wallet/udb/testdata/v5.db.go` | 433 | Generator for `v5.db.gz` (3751 bytes) |
| `wallet/udb/testdata/v7.go` | 214 | Generator for `v7.db.gz` (3931 bytes) |
| `wallet/udb/testdata/v11.db.go` | 469 | Generator for `v11.db.gz` (10401 bytes) |
| `wallet/udb/testdata/v24.db.go` | 94 | Generator for `v24.db.gz` (3358 bytes) |

Read in full as supporting context because the upgrade code calls into them (their complete
specification belongs to the address-manager, transaction-store and storage-engine specs):
`wallet/udb/metadata.go` (37), `wallet/udb/open.go` (54), `wallet/udb/initialize.go` (89),
`wallet/udb/addressdb.go` (1783), `wallet/udb/agendas.go` (94), `wallet/udb/vsp_test.go` (252),
`wallet/internal/bdb/db.go` (323), `wallet/internal/snacl/snacl.go` (249),
`internal/compat/compat.go`, and the parts of `wallet/udb/txdb.go`, `wallet/udb/txmined.go`,
`wallet/udb/addressmanager.go`, `wallet/walletdb/interface.go` and `wallet/wallet.go` cited below.
The eight `testdata/*.db.gz` files were decompressed and their bucket trees inspected with a
stand-alone bbolt page reader; statements marked "(fixture inspection)" come from that.
Removed code was read from local git history where the present code only hints at an old layout
(`wallet/udb/stakedb.go` and `wallet/udb/vsp.go` at earlier commits); those statements are marked
"(history)".

**Parity pin**

| Component | Version |
| --- | --- |
| dcrwallet | master `54b85b33` (2.2.0-pre, module `decred.org/dcrwallet/v5`) |
| `go.etcd.io/bbolt` | v1.3.12 (`bucket.go`) |
| `github.com/decred/dcrd/blockchain/stake/v5` | v5.0.3 (`staketx.go`) |
| `github.com/decred/dcrd/gcs/v4` | v4.1.1 (`blockcf2/blockcf.go`, `gcs.go`) |
| `github.com/decred/dcrd/hdkeychain/v3` | v3.1.3 (`extendedkey.go`) |
| `github.com/decred/dcrd/txscript/v4` | v4.1.2 (`stdscript/script.go`, `stdscript/scriptv0.go`) |
| `github.com/decred/dcrd/wire` | v1.8.0 (`msgtx.go`) |
| `github.com/decred/dcrd/chaincfg/chainhash` | v1.0.5 (`hash.go`) |

**Summary**

- The wallet database carries one version number, a big-endian `uint32` at top-level bucket `meta`,
  key `ver`. The current version is **26**. `Upgrade` applies every missing step in order inside
  **one** read-write transaction, so an upgrade is all-or-nothing.
- A brand-new wallet is **not** written at version 26 directly: `Initialize` writes a version 1
  layout and then runs all 25 upgrades. The upgrade chain is therefore on the creation path of
  every wallet, and the "fresh version 26" layout contains artefacts of that history.
- Fourteen of the 25 steps only bump the version, create an empty bucket or write a fixed key;
  four do a small mechanical rewrite (5, 15, 21, 23); seven carry real logic (2, 6, 8, 9, 12, 14,
  17) and depend on legacy record layouts,
  HD derivation, scrypt/secretbox, stake transaction classification and GCS filter construction.
- `Migrate` converts a pre-unified (dcrwallet 0.8.x and earlier) database to version 1.
- The auxiliary top-level buckets are: `vsp`, `vsphost`, `vsppubkey` (VSP fee-payment tracking),
  `treasurypolicy`, `tspendpolicy`, `vsptreasurypolicy`, `vsptspendpolicy` (treasury vote
  policies). All are flat key/value buckets with fixed big-endian layouts.
- The legacy stake manager bucket `wstakemgr` exists in every database created before
  dcrwallet 2.1.0 and is never read, written or deleted by the pinned code.
- Released dcrwallet versions map to database versions as: 1.0.0→3, 1.0.3→4, 1.0.5→5,
  1.0.7–1.1.0→6, 1.1.2→7, 1.2.0→9, 1.3.x/1.4.x→11, 1.5.x→13, 1.6.x→22, 1.7.x/1.8.x→25,
  2.0.x/2.1.x→26.

---

## 2. Role in the system

### 2.1 What the subsystem does

1. **Schema versioning.** Records one schema version for the whole `wallet.db` and refuses to
   operate on a database whose version differs from the code's `DBVersion`
   (`wallet/udb/open.go:30-38`).
2. **Upgrades.** Brings a database at any version 1..25 to version 26 in place
   (`wallet/udb/upgrades.go:1693-1727`).
3. **Migration.** Brings a database written before the unified layout existed (separate
   address-manager and transaction-store version numbers) to unified version 1
   (`wallet/udb/migration.go:37-73`).
4. **Auxiliary persistent state** that is neither address-manager nor transaction-store data:
   - VSP bookkeeping: for each ticket handed to a Voting Service Provider, the fee transaction
     hash, the fee-payment status and the VSP host and public key (`wallet/udb/vsp.go`).
   - Treasury vote policies: how to vote on treasury spends, by Politeia key or by tspend hash,
     wallet-wide or per ticket (`wallet/udb/treasury.go`).

### 2.2 Callers

| Caller | Calls | Citation |
| --- | --- | --- |
| `wallet.Open` | `NeedsMigration`, then `Migrate` if needed, then `Upgrade`, then `udb.Open` | `wallet/wallet.go:5340-5359`, `wallet/wallet.go:5428` |
| `wallet.Create` → `udb.Initialize` | writes version 1, then `Upgrade` | `wallet/wallet.go:5308`, `wallet/udb/initialize.go:18-51` |
| `wallet.CreateWatchOnly` → `udb.InitializeWatchOnly` | writes version 1, then `Upgrade` | `wallet/wallet.go:5318`, `wallet/udb/initialize.go:56-89` |
| `wallet.(*VSPTicket).UpdateFee{Confirmed,Paid,Started,Errored}` | `SetVSPTicket` | `wallet/vspticket.go:246-290` |
| `wallet.(*VSPTicket).VSPTicketInfo`, `wallet.VSPFeeHashForTicket`, `VSPHostForTicket`, `IsVSPTicketConfirmed` | `GetVSPTicket` | `wallet/vspticket.go:317-339`, `wallet/wallet.go:5653-5701` |
| `wallet.GetVSPTicketsByFeeStatus` (gRPC `GetVSPTicketsByFeeStatus`, `SyncVSPFailedTickets`) | `GetVSPTicketsByFeeStatus` | `wallet/wallet.go:5613-5633`, `internal/rpc/rpcserver/server.go:4049-4081` |
| Block-connect handler, after pruning unmined transactions | `DeleteVSPTicket` | `wallet/chainntfns.go:261-275` |
| `wallet.Open` (policy cache load) | `TSpendPolicies`, `VSPTSpendPolicies`, `TreasuryKeyPolicies`, `VSPTreasuryKeyPolicies` | `wallet/wallet.go:377-395`, `wallet/wallet.go:5497-5517` |
| `wallet.SetTreasuryKeyPolicy` | `SetTreasuryKeyPolicy`, `SetVSPTreasuryKeyPolicy` | `wallet/wallet.go:666-712` |
| `wallet.SetTSpendPolicy` | `SetTSpendPolicy`, `SetVSPTSpendPolicy` | `wallet/wallet.go:716-762` |
| `wallet.(*VSPClient).Process` | reads the stored status through `VSPTicketInfo` and dispatches on `udb.FeeStatus` (section 5.7) | `wallet/vsp.go:221-286` |
| gRPC `GetVSPTicketsByFeeStatus` | maps the protobuf enum to `udb.VSPFeeProcess*` one by one; any other enum value → `InvalidArgument` `fee status=<value>` | `internal/rpc/rpcserver/server.go:4049-4064` |
| `wallet.TreasuryKeyPolicy`, `wallet.TSpendPolicy` (vote construction, RPC listings) | no database access: read the in-memory maps keyed by `udb.VSPTSpend` / `udb.VSPTreasuryKey` that were loaded at open | `wallet/wallet.go:572-628` |

The single-policy readers `TSpendPolicy`, `VSPTSpendPolicy`, `TreasuryKeyPolicy`,
`VSPTreasuryKeyPolicy` and the host/pubkey accessors `SetVSPHost`, `GetVSPHost`, `SetVSPPubKey`,
`GetVSPPubKey` are exported but have no caller outside `wallet/udb` at the pin.

### 2.3 Callees

| Callee | Used for |
| --- | --- |
| `walletdb` transaction and bucket primitives over bbolt | all storage |
| `snacl.SecretKey` (scrypt) and `snacl.CryptoKey` (XSalsa20-Poly1305 secretbox) | upgrades 2 and 14: derive the public master key from the public passphrase, open the public crypto key, decrypt account xpubs and script hashes, encrypt script hashes |
| `hdkeychain.NewKeyFromString`, `(*ExtendedKey).Child` | upgrade 2: derive branch and address keys |
| `compat.HD2Address`, `stdaddr.Hash160` | upgrade 2: address hash of a derived key |
| `stake.IsSStx`, `stake.DetermineTxType`, `stake.AddrFromSStxPkScrCommitment`, `stake.AmountFromSStxPkScrCommitment` | upgrades 6, 9, 12 |
| `stdscript.IsStakeChangePubKeyHashScript`, `IsStakeChangeScriptHashScript` | upgrade 9 |
| `wire.MsgTx.Deserialize` | upgrades 6, 8, 9, 12 (through `readRawTxRecord`) |
| `blockcf2.Regular`, `blockcf2.Key`, `(*gcs.FilterV2).Bytes` | upgrade 15: genesis block filter |
| `chaincfg.Params` (`GenesisHash`, `GenesisBlock`, HD key IDs, address IDs) | upgrades 2, 10, 12, 15, 26; `Migrate` |
| address-manager row codecs in `addressdb.go` | upgrades 2, 5, 12, 14, 17 |
| transaction-store codecs in `txdb.go` / `txmined.go` | upgrades 6, 8, 9, 10, 11, 12, 14, 15, 18, 26 |
| `slog` logger `WLLT` | one Info line in `Upgrade`, Debug lines in upgrade 12 |

---

## 3. Public surface inventory

"Exported" means reachable from outside package `udb`. Kinds and messages are those of the
`decred.org/dcrwallet/v5/errors` package (`errors.E(kind, ...)`).

### 3.1 `upgrades.go`

| Symbol | Signature | Contract |
| --- | --- | --- |
| `DBVersion` (exported const) | `= birthBlockVersion = 26` | Latest schema version. `udb.Open` requires the stored version to equal it. |
| `initialVersion` | `= 1` | Version written by `Initialize`, `InitializeWatchOnly` and `Migrate`. |
| `lastUsedAddressIndexVersion` … `birthBlockVersion` | `= 2` … `= 26` | One constant per version; see the table in section 6. |
| `upgrades` | `[26]func(walletdb.ReadWriteTx, []byte, *chaincfg.Params) error` | `upgrades[n]` upgrades version `n` to `n+1`. `upgrades[0]` is nil. |
| `Upgrade` (exported) | `func Upgrade(ctx context.Context, db walletdb.DB, publicPassphrase []byte, params *chaincfg.Params) error` | Reads the version; returns nil if `version >= 26`; otherwise logs and runs `upgrades[version:]` in one read-write transaction. |
| `lastUsedAddressIndexUpgrade` | `func(tx, publicPassphrase, params) error` | 1→2. |
| `votingPreferencesUpgrade` | same | 2→3. |
| `noEncryptedSeedUpgrade` | same | 3→4. |
| `lastReturnedAddressUpgrade` | same | 4→5. |
| `ticketBucketUpgrade` | same | 5→6. |
| `slip0044CoinTypeUpgrade` | same | 6→7. |
| `hasExpiryUpgrade` | same | 7→8. |
| `hasExpiryFixedUpgrade` | same | 8→9. |
| `cfUpgrade` | same | 9→10. |
| `lastProcessedTxsBlockUpgrade` | same | 10→11. |
| `ticketCommitmentsUpgrade` | same | 11→12. |
| `importedXpubAccountUpgrade` | same | 12→13. |
| `unencryptedRedeemScriptsUpgrade` | same | 13→14. |
| `blockcf2Upgrade` | same | 14→15. |
| `perTicketVotingPreferencesUpgrade` | same | 15→16. |
| `accountVariablesUpgrade` | same | 16→17. |
| `unpublishedTxsUpgrade` | same | 17→18. |
| `tspendPolicyUpgrade` | same | 18→19. |
| `vspBucketUpgrade` | same | 19→20. |
| `vspStatusUpgrade` | same | 20→21. |
| `tspendHashPolicyUpgrade` | same | 21→22. |
| `vspHostVersionUpgrade` | same | 22→23. |
| `vspTreasuryPoliciesUpgrade` | same | 23→24. |
| `importVotingAccountUpgrade` | same | 24→25. |
| `birthBlockUpgrade` | same (passphrase ignored) | 25→26. |
| `genesisPrevScripter` | `struct{}`; `PrevScript(*wire.OutPoint) (uint16, []byte, bool)` returns `(0, nil, false)` | `blockcf2.PrevScripter` for the genesis block, which spends nothing. |

Only the public passphrase is ever available to an upgrade. No upgrade needs the private
passphrase or the seed.

#### 3.1.1 Stated purpose of each version constant

The doc comment of each constant (`wallet/udb/upgrades.go:30-214`) is the only upstream statement
of *why* a version exists. Three versions (7, 13, 25) change no data and exist solely as
forward-compatibility guards: the stamp makes older software refuse the file.

| Ver | Stated purpose | Difference between comment and pinned code |
| --- | --- | --- |
| 2 | Adds last-used address indexes to BIP0044 accounts; removes next-to-use indexes, all address-pool references and all per-address usage tracking. | — |
| 3 | "Removes all per-ticket vote bits, replacing them with vote preferences for choices on individual agendas from the current stake version." | The pinned upgrade removes nothing; it only creates `agendaprefs` (section 5.4.3). |
| 4 | Removes the encrypted seed earlier versions may have saved ("or more commonly, encrypted zeros on mainnet wallets"). | — |
| 5 | Adds last-returned child indexes for both branches to each BIP0044 account row, to avoid handing out identical addresses across restarts. | — |
| 6 | Adds a bucket recording the hashes of all tickets. | — |
| 7 | The BIP0044 coin-type key may be the legacy one or the SLIP0044 one. No key is added or removed; the bump exists "to prevent older software from assuming that coin type 20 exists" (not forwards-compatible). | — |
| 8 | Adds a has-expiry flag to credits so that stake-change outputs of transactions with an expiry become spendable only after coinbase maturity. | Superseded by 9. |
| 9 | Moves the has-expiry bit to an unused flag bit instead of the stake flag bits. | — |
| 10 | Adds the compact-filter bucket and a root key tracking whether all main-chain filters are saved. No filter headers are stored. | — |
| 11 | Adds a root key recording the last block processed for relevant transactions (distinct from the header tip); the current tip is recorded so a synced wallet does not rescan. | — |
| 12 | Adds the ticket-commitment buckets so that non-voting participants and VSP users get correct locked-funds balances. | — |
| 13 | Enables imported xpub accounts, numbered above `ImportedAddrAccount`. No data change; the bump keeps older software away from "account identifiers that would induce panics". | — |
| 14 | P2SH redeem scripts readable from the address manager without unlocking; checks that no cleartext script is missing. | — |
| 15 | Drops and recreates the filter bucket to switch to version 2 (consensus-committed) filters, triggering a re-download. | — |
| 16 | Top-level `ticketsagendaprefs` bucket for per-ticket vote preferences. | — |
| 17 | Per-account variables bucket, cheaper than reserialising a whole account row. | — |
| 18 | Bucket for unmined transactions that have not been published. | — |
| 19 | Top-level bucket for treasury-spend voting policy (by Politeia key). | — |
| 20 | Top-level bucket mapping VSP ticket hash → fee transaction hash. | — |
| 21 | Adds a status field to tracked VSP tickets. | — |
| 22 | Top-level bucket for treasury-spend voting policy by transaction hash. | — |
| 23 | Adds the VSP host and VSP pubkey buckets. | — |
| 24 | Top-level buckets for per-ticket (VSP customer) treasury policies. | — |
| 25 | Enables the imported voting account type (account row type 2). No data change; bumped "so that previous versions will error on startup". The comment is headed `importVotingAccount`, not the constant's name. | — |
| 26 | Allows saving a birth state; every existing wallet gets the genesis block. | — |

`DBVersion`'s own comment: databases with a recorded version higher than `DBVersion` fail to
open, "meaning any upgrades prevent reverting to older software"
(`wallet/udb/upgrades.go:210-213`).

#### 3.1.2 Where the version constants are consumed outside `upgrades.go`

The address-manager and transaction-store codecs are parametrised by database version; outside
an upgrade they are always called with `DBVersion`. A port that keeps versioned codecs needs the
same parameter; a port with a version floor can drop the lower arms.

| Constant | Consumer | Citation |
| --- | --- | --- |
| `DBVersion` | version gate | `wallet/udb/open.go:33-38` |
| `DBVersion` | `fetchDBAccount(ns, account, DBVersion)` at every run-time account load | `wallet/udb/addressmanager.go:483`, `:573`, `:1767`, `:1821`, `:2219`, `:2362` |
| `DBVersion` | `deletePrivateKeys(ns, DBVersion)` (conversion to watching-only) | `wallet/udb/addressmanager.go:1178` |
| `DBVersion` | credit iterators and credit writers (select the has-expiry bit) | `wallet/udb/txquery.go:76`, `:120`, `:454`; `wallet/udb/txmined.go:1181`, `:1422`, `:1507`, `:1539`, `:2119`, `:2133`, `:2283`, `:2312` |
| `hasExpiryVersion`, `hasExpiryFixedVersion` | `valueUnspentCredit`, `fetchRawCreditHasExpiry`, unmined credit value writer: version ≥ 9 → bit `0x40`; version 8 → bit `0x10`; below 8 → no flag (reader returns false) | `wallet/udb/txdb.go:784-787`, `:895-904`, `:1473-1476` |
| `accountVariablesVersion` | `fetchDBAccount` rejects type 0 rows at or above it | `wallet/udb/addressdb.go:812` |
| `initialVersion` | version written by `Initialize`, `InitializeWatchOnly`, `Migrate` | `wallet/udb/initialize.go:45`, `:83`; `wallet/udb/migration.go:71` |
| `initialVersion` | layout of the account rows written at creation (imported, default, `slip0044acct0`; watching-only: imported, default) | `wallet/udb/addressmanager.go:3056`, `:3065`, `:3074`, `:3206`, `:3214` |
| `initialVersion` | codec used to decode `slip0044acct0` in the coin-type upgrade | `wallet/udb/addressmanager.go:796` |

### 3.2 `migration.go`

| Symbol | Signature | Contract |
| --- | --- | --- |
| `waddrmgrBucketKey` | `[]byte("waddrmgr")` | Top-level address-manager bucket. |
| `wtxmgrBucketKey` | `[]byte("wtxmgr")` | Top-level transaction-store bucket. |
| `NeedsMigration` (exported) | `func NeedsMigration(ctx, db) (bool, error)` | True when top-level bucket `meta` is absent. The error is only a transaction-begin error. |
| `Migrate` (exported) | `func Migrate(ctx, db, params) error` | Runs the legacy address-manager and transaction-store upgrades, deletes both legacy version keys, creates `meta` and writes version 1. One read-write transaction. |

### 3.3 `vsp.go`

| Symbol | Signature | Contract |
| --- | --- | --- |
| `vspBucketKey`, `vspHostBucketKey`, `vspPubKeyBucketKey` | `"vsp"`, `"vsphost"`, `"vsppubkey"` | Top-level bucket names. |
| `hashSize` | `= 32` | |
| `FeeStatus` (exported) | `type FeeStatus int` | |
| `VSPFeeProcessStarted`, `VSPFeeProcessPaid`, `VSPFeeProcessErrored`, `VSPFeeProcessConfirmed` (exported) | `0, 1, 2, 3` | Stored as a `uint32`; same numbering as the gRPC enum `GetVSPTicketsByFeeStatusRequest.FeeStatus` (`rpc/api.proto:1277-1280`). |
| `VSPTicket` (exported) | `struct{ FeeHash chainhash.Hash; FeeTxStatus uint32; VSPHostID uint32; Host string; PubKey []byte }` | `Host` and `PubKey` are not part of the stored record; they are resolved through the host and pubkey buckets. |
| `SetVSPTicket` (exported) | `func(dbtx walletdb.ReadWriteTx, ticketHash *chainhash.Hash, record *VSPTicket) error` | Finds or allocates the host ID for `record.Host`, overwrites `record.VSPHostID`, stores the 40-byte record. |
| `DeleteVSPTicket` (exported) | `func(dbtx walletdb.ReadWriteTx, ticketHash chainhash.Hash) error` | Deletes the record; nil if absent. Host and pubkey rows are never deleted. |
| `GetVSPTicket` (exported) | `func(dbtx walletdb.ReadTx, tickethash chainhash.Hash) (*VSPTicket, error)` | `NotExist` if absent; otherwise the record with `Host`/`PubKey` filled when resolvable. |
| `GetVSPTicketsByFeeStatus` (exported) | `func(dbtx walletdb.ReadTx, feeStatus int) (map[chainhash.Hash]*VSPTicket, error)` | Every record whose status equals `feeStatus`. `Host`/`PubKey` are **not** filled. |
| `deserializeVSPTicket` / `serializeVSPTicket` | `[]byte ↔ *VSPTicket` | 40-byte codec. |
| `VSPHost` (exported) | `struct{ Host []byte }` | |
| `SetVSPHost` (exported) | `func(dbtx walletdb.ReadWriteTx, id uint32, record *VSPHost) error` | Stores host bytes under the 4-byte ID. Does not touch the index counter. |
| `GetVSPHost` (exported) | `func(dbtx walletdb.ReadTx, id uint32) (*VSPHost, error)` | `NotExist` if absent. |
| `deserializeVSPHost` / `serializeVSPHost` | | Length-prefixed codec. |
| `VSPPubKey` (exported) | `struct{ ID uint32; PubKey []byte }` | |
| `SetVSPPubKey` (exported) | `func(dbtx walletdb.ReadWriteTx, host []byte, record *VSPPubKey) error` | Stores ID and pubkey under the host bytes. |
| `GetVSPPubKey` (exported) | `func(dbtx walletdb.ReadTx, host []byte) (*VSPPubKey, error)` | `NotExist` if absent. |
| `deserializeVSPPubKey` / `serializeVSPPubKey` | | Codec. |

### 3.4 `treasury.go`

| Symbol | Signature | Contract |
| --- | --- | --- |
| `tspendPolicyBucketKey`, `treasuryPolicyBucketKey`, `vspTspendPolicyBucketKey`, `vspTreasuryPolicyBucketKey` | `"tspendpolicy"`, `"treasurypolicy"`, `"vsptspendpolicy"`, `"vsptreasurypolicy"` | Top-level bucket names. |
| `VSPTSpend` (exported) | `struct{ Ticket, TSpend chainhash.Hash }` | Map key for per-ticket tspend policies. |
| `VSPTreasuryKey` (exported) | `struct{ Ticket chainhash.Hash; TreasuryKey string }` | Map key for per-ticket key policies; `TreasuryKey` holds raw key bytes as a string. |
| `SetTSpendPolicy` (exported) | `func(dbtx walletdb.ReadWriteTx, hash *chainhash.Hash, policy stake.TreasuryVoteT) error` | Policy `0x00` deletes; otherwise stores one byte. |
| `SetVSPTSpendPolicy` (exported) | `func(dbtx, ticketHash, tspendHash *chainhash.Hash, policy) error` | Same, key `ticket‖tspend`. |
| `TSpendPolicy` (exported) | `func(dbtx walletdb.ReadTx, hash *chainhash.Hash) (stake.TreasuryVoteT, error)` | Absent → `(0x00, nil)`; value length ≠ 1 → `IO`. |
| `VSPTSpendPolicy` (exported) | `func(dbtx, ticketHash, tspendHash) (stake.TreasuryVoteT, error)` | Same. |
| `TSpendPolicies` (exported) | `func(dbtx walletdb.ReadTx) (map[chainhash.Hash]stake.TreasuryVoteT, error)` | All entries with a non-zero policy. |
| `VSPTSpendPolicies` (exported) | `func(dbtx) (map[VSPTSpend]stake.TreasuryVoteT, error)` | Same. |
| `SetTreasuryKeyPolicy` (exported) | `func(dbtx walletdb.ReadWriteTx, pikey []byte, policy stake.TreasuryVoteT) error` | Key is the raw `pikey` bytes. |
| `SetVSPTreasuryKeyPolicy` (exported) | `func(dbtx, ticket *chainhash.Hash, pikey []byte, policy) error` | Key `ticket‖pikey`. |
| `TreasuryKeyPolicy` (exported) | `func(dbtx walletdb.ReadTx, pikey []byte) (stake.TreasuryVoteT, error)` | Absent → `(0x00, nil)`; value length ≠ 1 → `IO`. |
| `VSPTreasuryKeyPolicy` (exported) | `func(dbtx, ticket, pikey) (stake.TreasuryVoteT, error)` | Same. |
| `TreasuryKeyPolicies` (exported) | `func(dbtx) (map[string]stake.TreasuryVoteT, error)` | All entries with a non-zero policy, keyed by `string(pikey)`. |
| `VSPTreasuryKeyPolicies` (exported) | `func(dbtx) (map[VSPTreasuryKey]stake.TreasuryVoteT, error)` | Same. |

`stake.TreasuryVoteT` is a `byte`: `TreasuryVoteInvalid = 0x00`, `TreasuryVoteYes = 0x01`,
`TreasuryVoteNo = 0x02` (`dcrd/blockchain/stake/v5@v5.0.3/staketx.go:823-836`).

### 3.5 Supporting symbols with observable behaviour (owned by other files, used here)

| Symbol | Where | Behaviour relied on |
| --- | --- | --- |
| `unifiedDBMetadata{}.rootBucketKey()` | `wallet/udb/metadata.go:15-17` | `"meta"` |
| `unifiedDBMetadataVersionKey` | `wallet/udb/metadata.go:19` | `"ver"` |
| `putVersion` / `getVersion` | `wallet/udb/metadata.go:21-37` | 4-byte big-endian; read fails with `IO` "bad udb version len N" unless the value is exactly 4 bytes |
| `byteOrder` | `wallet/udb/txdb.go:123` | `binary.BigEndian`; used by `metadata.go`, `vsp.go`, `agendas.go` and all of `txdb.go` |
| `udb.Open` version gate | `wallet/udb/open.go:26-38` | see section 5.3 |
| `rootVSPHostIndex` | `wallet/udb/txdb.go:162` | `"vsphostindex"`; despite living in the transaction-store key list it is a key of the top-level `vsphost` bucket |
| `agendaPreferences.defaultBucketKey()` / `.ticketsBucketKey()` | `wallet/udb/agendas.go:18-32` | `"agendaprefs"` (created by upgrade 3) / `"ticketsagendaprefs"` (created by upgrade 16) |
| `log`, `UseLogger` | `wallet/udb/log.go:10-20`, `log.go:33` | Package logger; `slog.Disabled` until `UseLogger` is called. The daemon installs the wallet logger (`WLLT`), so the upgrade Info line and the upgrade 12 Debug lines appear only in a process that wired the logger; a library user of `udb` gets no output. |
| `fetchRawCreditHasExpiry(v, dbVersion)` | `wallet/udb/txdb.go:895-904` | Version-dependent reader of the bit upgrades 8 and 9 write (section 3.1.2); used by `verifyV8Upgrade` with `DBVersion` |
| `extractRawTicketPickedHeight(v)` | `wallet/udb/txdb.go:1744-1746` | `int32(BE32(v))`; reads the `tix` value upgrade 6 writes; no length check |

### 3.6 Helpers the upgrades and `Migrate` call (owned by other files)

Every helper reached from the code in scope, with the behaviour the upgrade depends on. An
implementation of an upgrade step needs each of these in its **versioned** form.

Address-manager side (`wallet/udb/addressdb.go`, `wallet/udb/addressmanager.go`):

| Helper | Used by | Behaviour relied on | Citation |
| --- | --- | --- | --- |
| `fetchMasterKeyParams(ns)` | 2, 14 | Returns a copy of `main/mpub` (and `main/mpriv`, ignored). Absent `mpub` → `IO` "missing master pubkey params". | `wallet/udb/addressdb.go:318-338` |
| `fetchCryptoKeys(ns)` | 2, 14 | Returns a copy of `main/cpub` (and `main/cpriv`, ignored). Absent `cpub` → `IO` "missing encrypted crypto pubkey". | `wallet/udb/addressdb.go:443-463` |
| `fetchLastAccount(ns)` | 2, 5 | `LE32(meta/lastaccount)`; value length ≠ 4 (including absent) → `IO` "bad last account len N". | `wallet/udb/addressdb.go:711-720` |
| `fetchAccountRow(ns, account)` / `deserializeAccountRow` | 2, 5, 17 | Absent → `NotExist` "no account N"; value shorter than 5 → `IO` "bad account len N". `rdlen` is not checked against the value length: a row whose declared `rdlen` exceeds the bytes present panics. | `wallet/udb/addressdb.go:518-536`, `wallet/udb/addressdb.go:774-786` |
| `fetchAccountInfo(ns, account, dbVersion)` | 2, 5 | Accepts account type bytes **0 and 2** and decodes both with the legacy BIP0044 codec for `dbVersion`; any other type byte (including 1) → `IO` "unknown account type N". | `wallet/udb/addressdb.go:788-801` |
| `deserializeBIP0044AccountRow(id, row, dbVersion)` | 2, 5, 17 | Layouts of section 4.6.2. Only the minimum length (20 / 28) is checked; inner length fields that run past the data panic; bytes after the name are ignored (and dropped when the row is rewritten). | `wallet/udb/addressdb.go:555-608` |
| `bip0044AccountInfo(…, dbVersion)` / `serializeBIP0044AccountRow` | 2, 5 | Builds a type 0 row in the layout of `dbVersion`; the "next index" arguments are used only for version 1, the last-used pair for ≥ 2, the last-returned pair for ≥ 5. | `wallet/udb/addressdb.go:612-694` |
| `putBIP0044AccountInfo(ns, account, row)` | 2, 5 | Three puts, in order: `acct[LE32(account)]`, `acctididx[LE32(account)] = LE32(len(name)) ‖ name`, `acctnameidx[LE32(len(name)) ‖ name] = LE32(account)`. Each put error is `IO`. Old index entries are not deleted (the name does not change). | `wallet/udb/addressdb.go:883-952` |
| `forEachAccount(ns, fn)` | 17 | `ForEach` over `acct`, skipping nested-bucket entries, calling `fn(LE32(key))`. | `wallet/udb/addressdb.go:698-708` |
| `fetchDBAccount(ns, account, dbVersion)` | 17 | Type 0 with `dbVersion < 17` → legacy codec; type 0 with `dbVersion ≥ 17` → error "legacy BIP0044 account row unsupported in db version N" (no kind); type 1 or 2 → row + variables bucket; else `IO` "unknown account type N". | `wallet/udb/addressdb.go:803-851` |
| `putNewBIP0044Account(ns, account, a)` | 17 | Puts in order: account row, `acctididx`, `acctnameidx`, then `CreateBucketIfNotExists(acctvars/LE32(account))`, then `extused`, `intused`, `extret`, `intret`, `name`; `kdf-params` only when the account has a unique KDF (never in an upgrade). | `wallet/udb/addressdb.go:956-1006` |
| `accountNumberToAddrPoolKey(isInternal, account)` | 2 | 15-byte key: `addrpoolext` or `addrpoolint` (11 ASCII bytes) ‖ `LE32(account)`. | `wallet/udb/addressdb.go:1593-1605` |
| `fetchAddressByHash(ns, addrHash)` | 14 | `addrHash` is the bucket key itself (`SHA-256(hash160)`). Absent → `NotExist` "no address with hash <hex>"; shorter than 18 → `IO` "bad address len N"; type not 0/1/2 → `IO` "unknown address type N"; script rawData shorter than 8 → `IO` "bad script address len N". | `wallet/udb/addressdb.go:1111-1134`, `wallet/udb/addressdb.go:1241-1268`, `wallet/udb/addressdb.go:1296-1318` |
| `serializeScriptAddress(encHash, script)` | 14 | `LE32(len(encHash)) ‖ encHash ‖ LE32(len(script)) ‖ script`. | `wallet/udb/addressdb.go:1272-1289` |
| `putAddress(ns, addressID, row)` | 14 | `addr[SHA-256(addressID)] = row`, then `putAddrAccountIndex`: `addracctidx[SHA-256(addressID)] = LE32(account)`, `CreateBucketIfNotExists(addracctidx/LE32(account))`, and in it `SHA-256(addressID) → 00`. Errors `IO`. | `wallet/udb/addressdb.go:907-926`, `wallet/udb/addressdb.go:1337-1350` |
| `putScriptAddress(ns, addressID, account, encHash, script)` | 14 | `putAddress` of a type 2 row with add-time `time.Now().Unix()`. | `wallet/udb/addressdb.go:1383-1394` |
| `fetchAddrAccount(ns, addressID)` | 12 | `LE32(addracctidx[SHA-256(addressID)])`; absent → `NotExist` "no address for id <hex>". The value length is not checked (shorter than 4 panics). | `wallet/udb/addressdb.go:1407-1416` |
| `normalizeAddress(addr)`, `addressID(addr)` | 12 | Public-key addresses become their public-key-hash form; the ID is the 20-byte hash160. An address type without a hash160 → `Invalid` "address id cannot be created from type %T (requires Hash160 method)". | `wallet/udb/addressmanager.go:104-110`, `wallet/udb/addressmanager.go:968-977` |
| `upgradeManager(ns)`, `upgradeToVersion5`, `upgradeToVersion6`, `fetchManagerVersion`, `putManagerVersion` | `Migrate` | Section 5.5 step 1. The local names `lastaddrs` and `sync` exist only inside these two functions. | `wallet/udb/addressdb.go:291-311`, `wallet/udb/addressdb.go:1694-1783` |

Transaction-store side (`wallet/udb/txdb.go`, `wallet/udb/txmined.go`):

| Helper | Used by | Behaviour relied on | Citation |
| --- | --- | --- | --- |
| `readRawTxRecordHash(k, &hash)` | 6 | Key shorter than 68 → stack trace on stderr, then `IO` "tx record key len N"; else the first 32 bytes. | `wallet/udb/txdb.go:578-585` |
| `readRawUnminedHash(k, &hash)` | 6, 12 | Key shorter than 32 → `IO` "unmined key len N"; else the first 32 bytes. | `wallet/udb/txdb.go:1365-1371` |
| `readRawTxRecord(&hash, v, &rec)` | 6, 8, 9, 12 | Value shorter than 8 (including absent) → `IO` "tx record len N"; `Received = BE64(v[0:8])` seconds; transaction = `v[8:]`, decode failure → `IO`; sets `rec.TxType = stake.DetermineTxType(tx)`. Trailing bytes after the transaction are not rejected. | `wallet/udb/txdb.go:561-576` |
| `fetchBlockRecord(ns, height)` / `readRawBlockRecord` | 8, 9, 12 | Section 4.6.8 layout. Errors `IO`: "block key len N", "block record len N" (prints the key length), "N tx block record len M". | `wallet/udb/txdb.go:293-335` |
| `existsTxRecord(ns, &hash, &block)` | 8, 9 | `t[hash ‖ BE32(block.Height) ‖ block.Hash]`, nil when absent. | `wallet/udb/txdb.go:653-657` |
| `existsRawUnmined(ns, k)` | 8, 9 | `m[k]`, nil when absent. | `wallet/udb/txdb.go:1373-1375` |
| `latestTxRecord(ns, hash)` | 12 | Seeks `t` to the 32-byte prefix and returns the **last** entry sharing it; `(nil, nil)` when none. | `wallet/udb/txdb.go:675-685` |
| `makeReadBlockIterator(ns, 0)` / `(*blockIterator).next` | 12 | Cursor over `b` from `BE32(0)` upward. `next` returns false at the end of the bucket **and** on the first undecodable record (error stored in `it.err`, which upgrade 12 never reads; the cursor is then dropped without being closed). | `wallet/udb/txdb.go:354-391` |
| `extractRawCreditTxHash(k)`, `extractRawCreditHeight(k)`, `extractRawCreditIndex(k)` | 8, 9 | `k[0:32]`, `BE32(k[32:36])` as `int32`, `BE32(k[68:72])`. No length checks: a mined credit key shorter than 36 (72 for the index) panics. | `wallet/udb/txdb.go:818-833` |
| `extractRawUnminedCreditTxHash(k)` | 8, 9 | `k[:32]`; a key shorter than 32 panics. | `wallet/udb/txdb.go:835-837` |
| `fetchRawUnminedCreditIndex(k)` | 9 | Key shorter than 36 → `IO` "unmined credit key len N"; else `BE32(k[32:36])`. | `wallet/udb/txdb.go:1500-1505` |
| `putTicketRecord(ns, &hash, pickedHeight)` | 6 | `tix[hash] = BE32(uint32(pickedHeight))`. | `wallet/udb/txdb.go:1734-1738` |
| `canonicalOutPoint(&hash, index)` | 6, 12 | 36 bytes: hash ‖ `BE32(index)`. | `wallet/udb/txdb.go:200-205` |
| `putRawCFilter(ns, k, v)` | 10, 15 | `cf[k] = v`; error not wrapped by the helper. | `wallet/udb/txdb.go:1988-1990` |
| `valueRawCFilter2(key, data)` | 15 | `key (16) ‖ data`. | `wallet/udb/txdb.go:1981-1986` |
| `keyTicketCommitment`, `valueTicketCommitment`, `existsRawTicketCommitment`, `putRawTicketCommitment`, `valueUnspentTicketCommitment`, `putRawUnspentTicketCommitment`, `deleteRawUnspentTicketCommitment` | 12 | Section 4.6.8 layouts; put/delete errors are `IO`. | `wallet/udb/txdb.go:2007-2105` |
| `SetBirthState(dbtx, bs)` | 26 | Takes the top-level `wtxmgr` bucket itself and puts the 45-byte `birthstate` value; the put error is returned unwrapped. | `wallet/udb/txmined.go:341-363` |
| `upgradeTxDB(ns, params)`, `upgradeToVersion2`, `upgradeToVersion3` | `Migrate` | Section 5.5 step 2. | `wallet/udb/txdb.go:2298-2438` |
| `valueBlockRecordEmptyFromHeader(hash, header)` | `Migrate` | 47 bytes: hash ‖ `BE64(header time)` ‖ `BE16(header vote bits)` ‖ `00` ‖ `BE32(0)`. | `wallet/udb/txdb.go:236-243` |

---

## 4. Data formats

Conventions: `BE32`/`BE64` are big-endian unsigned integers, `LE32`/`LE64` little-endian.
`‖` is byte concatenation. Offsets are zero-based and half-open (`[a:b]`). Hashes are the 32
internal bytes of a `chainhash.Hash` (not the reversed display form). `HASH160(x)` / "hash160" is
the Decred form `RIPEMD-160(BLAKE-256(x))`, 20 bytes — not Bitcoin's `RIPEMD-160(SHA-256(x))`
(`dcrd/txscript/v4@v4.1.2/stdaddr/addressv0.go:91-96`, `dcrd/dcrutil/v4@v4.0.3/hash160.go:22-24`);
`SHA-256` is plain single SHA-256. All buckets live in a bbolt
file; keys within a bucket iterate in ascending byte order.

### 4.1 Schema version record

| Location | Key | Value | Citation |
| --- | --- | --- | --- |
| top-level bucket `meta` | `ver` (3 ASCII bytes) | `BE32(version)`, exactly 4 bytes | `wallet/udb/metadata.go:15-37` |

`meta` holds nothing else. Presence of the bucket is the "unified layout" marker
(`wallet/udb/migration.go:24-31`).

Note the two different byte orders in the same file: the unified version and everything in
`txdb.go`, `vsp.go`, `treasury.go` and `agendas.go` is big-endian, while the address manager
(`addressdb.go`) is little-endian.

### 4.2 Top-level buckets and the version that introduces each

| Bucket | Present from | Created by | Purpose |
| --- | --- | --- | --- |
| `waddrmgr` | pre-unified | `Initialize` / legacy | Address manager namespace |
| `wtxmgr` | pre-unified | `Initialize` / legacy | Transaction store namespace |
| `wstakemgr` | pre-unified; **not created** by the pinned code | older `Initialize` | Legacy stake store, dead data (section 4.5) |
| `meta` | version 1 | `Initialize` / `Migrate` | Schema version |
| `agendaprefs` | version 3 | `votingPreferencesUpgrade` | Default agenda vote choices |
| `ticketsagendaprefs` | version 16 | `perTicketVotingPreferencesUpgrade` | Per-ticket agenda vote choices (one nested bucket per ticket hash) |
| `treasurypolicy` | version 19 | `tspendPolicyUpgrade` | Treasury policy by Politeia key |
| `vsp` | version 20 | `vspBucketUpgrade` | VSP ticket records |
| `tspendpolicy` | version 22 | `tspendHashPolicyUpgrade` | Treasury policy by tspend hash |
| `vsphost` | version 23 | `vspHostVersionUpgrade` | VSP host table and ID counter |
| `vsppubkey` | version 23 | `vspHostVersionUpgrade` | VSP pubkey table |
| `vsptreasurypolicy` | version 24 | `vspTreasuryPoliciesUpgrade` | Per-ticket treasury policy by Politeia key |
| `vsptspendpolicy` | version 24 | `vspTreasuryPoliciesUpgrade` | Per-ticket treasury policy by tspend hash |

Nested buckets and root keys added or removed by upgrades inside `waddrmgr` and `wtxmgr`:

| Path | Kind | Added | Removed | By |
| --- | --- | --- | --- | --- |
| `waddrmgr/usedaddrs` | bucket | version 1 or earlier | version 2 | `lastUsedAddressIndexUpgrade` |
| `waddrmgr/meta/addrpoolext‖LE32(acct)`, `…/addrpoolint‖LE32(acct)` | keys | version 1 or earlier | version 2 | same |
| `waddrmgr/main/seed` | key | version 1 or earlier | version 4 | `noEncryptedSeedUpgrade` |
| `waddrmgr/main/cscript` | key | version 1 or earlier | version 14 | `unencryptedRedeemScriptsUpgrade` |
| `waddrmgr/acctvars` (one nested bucket per account) | bucket | version 17 | — | `accountVariablesUpgrade` |
| `wtxmgr/tix` | bucket | version 6 | — | `ticketBucketUpgrade` |
| `wtxmgr/havecfilters` | key | version 10 | — | `cfUpgrade` |
| `wtxmgr/cf` | bucket | version 10; emptied and recreated at 15 | — | `cfUpgrade`, `blockcf2Upgrade` |
| `wtxmgr/lasttxsblock` | key | version 11 | — | `lastProcessedTxsBlockUpgrade` |
| `wtxmgr/cmt`, `wtxmgr/cmu` | buckets | version 12 | — | `ticketCommitmentsUpgrade` |
| `wtxmgr/sc` | bucket | version 1 or earlier | version 14 | `unencryptedRedeemScriptsUpgrade` |
| `wtxmgr/up` | bucket | version 18 | — | `unpublishedTxsUpgrade` |
| `wtxmgr/birthstate` | key | version 26 | — | `birthBlockUpgrade` |
| `waddrmgr/main/mgrver` | key | pre-unified | only by `Migrate` | see quirk Q2 |
| `wtxmgr/vers` | key | pre-unified (never written by `createStore`) | by `Migrate` | `wallet/udb/migration.go:63` |

### 4.3 VSP buckets

All three are flat top-level buckets (`wallet/udb/vsp.go:13-17`).

#### 4.3.1 `vsp` — ticket records

Key: ticket hash, 32 bytes.

Value, 40 bytes (`wallet/udb/vsp.go:147-179`):

| Offset | Size | Field | Encoding |
| --- | --- | --- | --- |
| 0 | 32 | Fee transaction hash | raw hash bytes; all zero when no fee transaction is known (`UpdateFeeErrored` writes zero, `wallet/vspticket.go:279-289`) |
| 32 | 4 | Fee status | `BE32`; 0 started, 1 paid, 2 errored, 3 confirmed |
| 36 | 4 | VSP host ID | `BE32`; key into `vsphost`; 0 means "unknown host" |

Historical value sizes (history: `wallet/udb/vsp.go` at commits `259369a4`, `fdc4bf4f`):

| Database version | Value | Size |
| --- | --- | --- |
| 20 | fee hash | 32 |
| 21, 22 | fee hash ‖ `BE32(status)` | 36 |
| 23 and later | fee hash ‖ `BE32(status)` ‖ `BE32(hostID)` | 40 |

The reader does not check the length; a value shorter than 40 bytes makes the Go code panic
(slice out of range) (`wallet/udb/vsp.go:149-161`).

#### 4.3.2 `vsphost` — host table and counter

| Key | Value | Citation |
| --- | --- | --- |
| `BE32(hostID)` (4 bytes) | `BE32(len(host))` ‖ host bytes | `wallet/udb/vsp.go:185-234` |
| `vsphostindex` (12 ASCII bytes) | `BE32(highest allocated host ID)` | `wallet/udb/vsp.go:52-68`, `wallet/udb/txdb.go:162` |

The 12-byte counter key cannot collide with a 4-byte ID key. Version 23 seeds the bucket with
ID 0 → `00 00 00 00` (zero-length host) and `vsphostindex` → `00 00 00 00`
(`wallet/udb/upgrades.go:1575-1593`). Real hosts therefore receive IDs 1, 2, 3, … in order of
first use. Host bytes are whatever string the caller passed as `VSPTicket.Host` (the VSP URL as
configured); no normalisation is applied at this layer.

#### 4.3.3 `vsppubkey` — pubkey table

| Key | Value | Citation |
| --- | --- | --- |
| host bytes (same bytes as stored in `vsphost`) | `BE32(hostID)` ‖ `BE32(len(pubkey))` ‖ pubkey bytes | `wallet/udb/vsp.go:241-289` |

`vsphost` maps ID → host; `vsppubkey` maps host → (ID, pubkey). A host is entered into both in
the same transaction on first use and never updated or removed afterwards.

### 4.4 Treasury policy buckets

All four are flat top-level buckets (`wallet/udb/treasury.go:14-20`). Every value is exactly one
byte holding the `stake.TreasuryVoteT`: `0x01` yes, `0x02` no. A policy of `0x00`
(invalid/abstain) is represented by the **absence** of the key.

| Bucket | Key | Key size | Citation |
| --- | --- | --- | --- |
| `tspendpolicy` | tspend transaction hash | 32 | `wallet/udb/treasury.go:33-45` |
| `vsptspendpolicy` | ticket hash ‖ tspend hash | 64 | `wallet/udb/treasury.go:49-65` |
| `treasurypolicy` | Politeia ("pi") key bytes as passed by the caller | variable (33 in practice: compressed secp256k1 pubkey) | `wallet/udb/treasury.go:151-163` |
| `vsptreasurypolicy` | ticket hash ‖ pi key bytes | 32 + variable | `wallet/udb/treasury.go:167-183` |

The storage layer does not validate the pi key length or content.

### 4.5 Legacy stake store bucket `wstakemgr` (dead data)

Every database initialised by dcrwallet before commit `a83b85bb` (first released in 2.1.0)
contains a top-level bucket `wstakemgr`. The pinned code has no reference to it: it is not
created, read, upgraded or deleted. A drop-in replacement must tolerate its presence and leave
it untouched.

Layout as last written (history: `wallet/udb/stakedb.go` at `a83b85bb^`; fixture inspection
confirms the five nested buckets in all eight fixtures):

| Path | Key | Value |
| --- | --- | --- |
| `wstakemgr/main` | `stakestorecreated` | `LE64(unix seconds)` |
| `wstakemgr/main` | `stakestorever` | legacy stake store version; only in pre-unified databases. Older `Migrate` deleted it; the pinned `Migrate` leaves it. |
| `wstakemgr/sstxrecords` | ticket hash (32) | database version ≥ 3: `LE32(pkScript offset of output 0)` ‖ serialized transaction ‖ `LE64(received unix)`. Version < 3: `LE64(tx length)` ‖ `LE32(pkScript offset)` ‖ `u8(vote bits length)` ‖ `LE16(vote bits)` ‖ 73 bytes extended vote bits area ‖ serialized transaction ‖ `LE64(received unix)` |
| `wstakemgr/ssgenrecords` | ticket hash (per the source comment) | vote records; the bucket was still created but no writer exists at `a83b85bb^` |
| `wstakemgr/ssrtxrecords` | not read | revocation records; same status |
| `wstakemgr/meta` | `tickt` ‖ 20-byte script hash | n × 73 bytes: ticket hash (32) ‖ `LE32(ticket height)` ‖ `u8(status)` ‖ `LE32(spent height)` ‖ spender hash (32) |
| `wstakemgr/meta` | `invld` ‖ 20-byte script hash | n × 32-byte ticket hashes |

Because the pinned 2→3 upgrade no longer rewrites `sstxrecords`, a database upgraded from
version ≤ 2 by the pinned code keeps its `sstxrecords` values in the version < 3 form, whereas the
same database upgraded by an older release has them in the ≥ 3 form. Nothing reads either.

### 4.6 Legacy layouts that upgrades read or rewrite

These are the facts about old on-disk layouts that the upgrade code reveals. They are what an
implementation must be able to **read** to support a given starting version.

#### 4.6.1 Pre-unified database (input of `Migrate`)

| Item | Layout | Citation |
| --- | --- | --- |
| Address-manager version | `waddrmgr/main/mgrver` = `LE32`; values 1..6 seen by the upgrader; 6 is final | `wallet/udb/addressdb.go:291-299`, `wallet/udb/addressdb.go:1723-1783` |
| Transaction-store version | `wtxmgr/vers` = `BE32`; 1, 2 or ≥ 3 | `wallet/udb/txdb.go:2298-2326` |
| `waddrmgr/main/lastaddrs` | key removed by manager upgrade to 5 | `wallet/udb/addressdb.go:1697-1707` |
| `waddrmgr/sync` | nested bucket removed by manager upgrade to 6 | `wallet/udb/addressdb.go:1712-1719` |
| Block record (`wtxmgr/b`) in store version < 3 | hash (32) ‖ `BE64(time)` ‖ `BE16(vote bits)` ‖ `BE32(n)` ‖ n × hash — i.e. the present layout without the stake-invalidated byte at offset 42 | `wallet/udb/txdb.go:2354-2400` |
| `wtxmgr/h`, `wtxmgr/ic`, `wtxmgr/id`, `wtxmgr/tip` | absent before store version 3 | `wallet/udb/txdb.go:2366-2377`, `wallet/udb/txdb.go:2432` |
| No top-level `meta` bucket | | `wallet/udb/migration.go:27` |

#### 4.6.2 Account rows (`waddrmgr/acct`, key `LE32(account)`)

Outer row (`wallet/udb/addressdb.go:518-551`):

| Offset | Size | Field |
| --- | --- | --- |
| 0 | 1 | account type: 0 legacy BIP0044 row, 1 BIP0044 with variables bucket, 2 imported voting |
| 1 | 4 | `LE32(len(rawData))` |
| 5 | n | rawData |

rawData of a type 0 row, by database version (`wallet/udb/addressdb.go:555-658`):

| Versions | rawData |
| --- | --- |
| 1 | `LE32(len(encPub))` ‖ encPub ‖ `LE32(len(encPriv))` ‖ encPriv ‖ `LE32(nextExternalIndex)` ‖ `LE32(nextInternalIndex)` ‖ `LE32(len(name))` ‖ name |
| 2, 3, 4 | same, with `LE32(lastUsedExternalIndex)` ‖ `LE32(lastUsedInternalIndex)` in place of the two "next" indexes |
| 5 … 16 | as 2–4, followed by `LE32(lastReturnedExternalIndex)` ‖ `LE32(lastReturnedInternalIndex)` before the name length |
| 17 and later | type 0 rows are rejected ("legacy BIP0044 account row unsupported in db version N", `wallet/udb/addressdb.go:811-817`) |

Minimum rawData length accepted: 20 for versions < 5, 28 for versions ≥ 5.

`0xFFFFFFFF` in a last-used or last-returned field means "none". The version 1 "next" fields
count addresses generated so far (20 after generating indexes 0..19 — fixture inspection of
`v1.db.gz`).

rawData of a type 1 or 2 row (version ≥ 17) (`wallet/udb/addressdb.go:99-137`):
`LE32(len(encPub))` ‖ encPub ‖ `LE32(len(encPriv))` ‖ encPriv, with no trailing bytes.

Account variables (version ≥ 17), nested bucket `waddrmgr/acctvars/LE32(account)`
(`wallet/udb/addressdb.go:1030-1068`):

| Key | Value |
| --- | --- |
| `extused` | `LE32(lastUsedExternalIndex)` |
| `intused` | `LE32(lastUsedInternalIndex)` |
| `extret` | `LE32(lastReturnedExternalIndex)` |
| `intret` | `LE32(lastReturnedInternalIndex)` |
| `name` | account name, raw bytes, no length prefix |
| `kdf-params` | optional; never written by an upgrade |

Indexes kept alongside: `waddrmgr/acctididx`: `LE32(account)` → `LE32(len(name))` ‖ name;
`waddrmgr/acctnameidx`: `LE32(len(name))` ‖ name → `LE32(account)`
(`wallet/udb/addressdb.go:883-904`).

#### 4.6.3 Version 1 address-usage data (input of upgrade 2)

| Path | Key | Value | Citation |
| --- | --- | --- | --- |
| `waddrmgr/addr` | `SHA-256(hash160 of address)` | address row (section 4.6.5) | `wallet/udb/upgrades.go:256-259`, `wallet/udb/addressdb.go:1337-1350` |
| `waddrmgr/usedaddrs` | `SHA-256(hash160 of address)` | single byte `00` (presence is the flag) | `wallet/udb/upgrades.go:351`; fixture inspection; history `0dd83d98:wallet/udb/addressdb.go` |
| `waddrmgr/meta` | `addrpoolext` ‖ `LE32(account)` (15 bytes) | `LE32(next-to-use external index)` | `wallet/udb/addressdb.go:1593-1621` |
| `waddrmgr/meta` | `addrpoolint` ‖ `LE32(account)` (15 bytes) | `LE32(next-to-use internal index)` | same |
| `waddrmgr/meta` | `lastaccount` | `LE32(highest BIP0044 account number)` | `wallet/udb/addressdb.go:711-720` |

#### 4.6.4 Public-data encryption used by upgrades 2 and 14

| Item | Layout | Citation |
| --- | --- | --- |
| `waddrmgr/main/mpub` | 88 bytes: salt (32) ‖ SHA-256 of derived key (32) ‖ `LE64(N)` ‖ `LE64(r)` ‖ `LE64(p)` | `wallet/internal/snacl/snacl.go:129-180` |
| derived key | `scrypt(passphrase, salt, N, r, p, 32)`; accepted iff `SHA-256(key)` equals the stored digest (constant-time compare) | `wallet/internal/snacl/snacl.go:104-125`, `wallet/internal/snacl/snacl.go:192-204` |
| `waddrmgr/main/cpub` | secretbox of the 32-byte public crypto key under the derived key | `wallet/udb/addressdb.go:443-463` |
| any sealed value | nonce (24) ‖ XSalsa20-Poly1305 ciphertext (plaintext length + 16) | `wallet/internal/snacl/snacl.go:35-63` |
| account `encPub` | sealed, under the public crypto key, of the base58 extended public key string | `wallet/udb/upgrades.go:312-316` |

#### 4.6.5 Address rows and P2SH scripts before version 14

Address row (`waddrmgr/addr` value) (`wallet/udb/addressdb.go:1111-1152`):

| Offset | Size | Field |
| --- | --- | --- |
| 0 | 1 | address type: 0 chained, 1 imported key, 2 script |
| 1 | 4 | `LE32(account)` |
| 5 | 8 | `LE64(add time, unix seconds)` |
| 13 | 1 | unused, written as 0 |
| 14 | 4 | `LE32(len(rawData))` |
| 18 | n | rawData |

Script address rawData (`wallet/udb/addressdb.go:1241-1289`):
`LE32(len(encHash))` ‖ encHash ‖ `LE32(len(script))` ‖ script, where encHash is the sealed
20-byte script hash160 under the public crypto key.

| Versions | `script` field | Other copies of the script |
| --- | --- | --- |
| ≤ 13 | sealed under the script crypto key (itself sealed in `waddrmgr/main/cscript`) | cleartext in `wtxmgr/sc`: key = hash160(script) (20 bytes), value = script |
| ≥ 14 | cleartext | none; `wtxmgr/sc` and `cscript` are gone |

Address-to-account index (`waddrmgr/addracctidx`): `SHA-256(hash160)` → `LE32(account)`, plus a
nested bucket per account, key `LE32(account)`, containing `SHA-256(hash160)` → `00`
(`wallet/udb/addressdb.go:907-926`).

#### 4.6.6 Credit flag byte history (input of upgrades 8 and 9)

Mined credits: bucket `wtxmgr/c`, key = tx hash (32) ‖ `BE32(height)` ‖ block hash (32) ‖
`BE32(output index)` (72 bytes); flags are value byte 8. Unmined credits: bucket `wtxmgr/mc`,
key = tx hash (32) ‖ `BE32(output index)` (36 bytes); flags are value byte 8
(`wallet/udb/txdb.go:687-741`, `wallet/udb/txdb.go:1422-1459`).

| Bit | Mask | Versions ≤ 7 | Version 8 | Version ≥ 9 |
| --- | --- | --- | --- | --- |
| 0 | `0x01` | spent | spent | spent |
| 1 | `0x02` | change | change | change |
| 2–4 | `0x1c` | stake opcode tag: `0x00` none, `0x04` OP_SSTX, `0x08` OP_SSGEN, `0x0c` OP_SSRTX, `0x10` OP_SSTXCHANGE | bit 4 (`0x10`) **also** means "transaction has an expiry" (ambiguous with the tag) | tag only (`0x1c` additionally means OP_TGEN) |
| 5 | `0x20` | coinbase | coinbase | coinbase |
| 6 | `0x40` | unused | unused | transaction has an expiry |

(`wallet/udb/txdb.go:771-798`, `wallet/udb/txdb.go:895-904`.)

#### 4.6.7 Compact filter bucket history

| Versions | `wtxmgr/cf` value for key = block hash | Citation |
| --- | --- | --- |
| 10 … 14, as written by releases of that era | version 1 GCS filter in "N-bytes" form: `BE32(N)` ‖ filter bits (fixture inspection of `v11.db.gz`: genesis → `00 00 00 01 53 ea d0`) | history: `a9fa2485` diff of `cfUpgrade` |
| 10, as written by the pinned `cfUpgrade` for the genesis block | empty value | `wallet/udb/upgrades.go:863-875` |
| ≥ 15 | filter key (16 bytes = first 16 bytes of the block's merkle root) ‖ version 2 filter bytes (`gcs.FilterV2.Bytes()`: varint N then filter bits) | `wallet/udb/txdb.go:1968-1986` |

`wtxmgr/havecfilters`: one byte, `00` or `01`.

Known genesis value at version ≥ 15 for testnet3 (fixture inspection of `v24.db.gz`):
key `ac9ba434b6f7249b9698d1fcec26d6087e8358c811c7e922f4ca1839e5dc49a6` →
`2c0ad603d44a16698ac951fa22aab5e7 01922e18` (20 bytes). The mainnet, simnet and regnet values
are not recorded here; generate them with the Go oracle (section 11).

#### 4.6.8 Records created by upgrades in `wtxmgr`

| Bucket / key | Key | Value | Citation |
| --- | --- | --- | --- |
| `tix` | ticket hash (32) | `BE32(picked height as int32)`; upgrades write `ff ff ff ff` (−1) | `wallet/udb/txdb.go:1715-1746` |
| `cmt` | ticket hash (32) ‖ `BE32(output index)` | `BE64(amount)` ‖ `BE32(account)` (12 bytes) | `wallet/udb/txdb.go:2007-2030` |
| `cmu` | same key as `cmt` | one flags byte: `0x01` = spent by an unmined transaction, else `0x00` | `wallet/udb/txdb.go:2066-2089` |
| `lasttxsblock` (root key) | — | block hash (32) | `wallet/udb/upgrades.go:909` |
| `birthstate` (root key) | — | 45 bytes: options (1; bit 0 set-from-height, bit 1 set-from-time) ‖ block hash (32) ‖ `BE32(height)` ‖ `BE64(unix seconds)` | `wallet/udb/txmined.go:334-363` |
| `mi` (unmined inputs) | previous tx hash (32) ‖ `BE32(output index)` | not read by any upgrade (begins with a 32-byte spender hash) | `wallet/udb/txdb.go:200-205`, `wallet/udb/txdb.go:1711-1713` |

Transaction records read by upgrades: `wtxmgr/t` key = tx hash (32) ‖ `BE32(height)` ‖ block
hash (32) (68 bytes), value = `BE64(received unix)` ‖ serialized transaction; `wtxmgr/m` key =
tx hash (32), same value format; `wtxmgr/b` key = `BE32(height)`, value = hash (32) ‖
`BE64(time)` ‖ `BE16(vote bits)` ‖ `u8(stake invalidated)` ‖ `BE32(n)` ‖ n × tx hash
(`wallet/udb/txdb.go:216-229`, `wallet/udb/txdb.go:487-500`, `wallet/udb/txdb.go:1351-1355`).

### 4.7 Canonical content of a freshly created database at version 26

`Initialize` builds a version 1 layout and runs upgrades 1→26 (`wallet/udb/initialize.go:18-51`).
The result, confirmed against `v24.db.gz` (an untouched `Initialize` output at version 24) plus
the effects of upgrades 25 and 26, is:

Top-level buckets: `agendaprefs` (empty), `meta`, `ticketsagendaprefs` (empty), `treasurypolicy`
(empty), `tspendpolicy` (empty), `vsp` (empty), `vsphost`, `vsppubkey` (empty),
`vsptreasurypolicy` (empty), `vsptspendpolicy` (empty), `waddrmgr`, `wtxmgr`. No `wstakemgr`.

| Path | Content |
| --- | --- |
| `meta/ver` | `00 00 00 1a` |
| `vsphost/00000000` | `00 00 00 00` |
| `vsphost/vsphostindex` | `00 00 00 00` |
| `waddrmgr/main` keys (seeded wallet) | `cpriv`, `cpub`, `ctpriv`, `ctpriv-slip0044`, `ctpub`, `ctpub-slip0044`, `mgrcreated`, `mgrver`, `mpriv`, `mpub`, `slip0044acct0`, `watchonly` |
| `waddrmgr/main/mgrver` | `06 00 00 00` (legacy manager version, still present) |
| `waddrmgr/main/slip0044acct0` | an account row still in the **version 1** type 0 layout (never upgraded) |
| `waddrmgr/meta` | only `lastaccount` = `00 00 00 00` |
| `waddrmgr/acct/00000000` | type 1 row: `01` ‖ `LE32(rdlen)` ‖ `LE32(len)` ‖ encPub ‖ `LE32(len)` ‖ encPriv |
| `waddrmgr/acct/ffffff7f` (imported account, 2³¹−1) | `01 08 00 00 00 00 00 00 00 00 00 00 00` (13 bytes) |
| `waddrmgr/acctvars/00000000` | `extused`, `intused`, `extret`, `intret` = `ff ff ff ff`; `name` = `default` |
| `waddrmgr/acctvars/ffffff7f` | `extused`, `intused`, `extret`, `intret` = `00 00 00 00`; `name` = `imported` |
| `waddrmgr/acctididx`, `acctnameidx` | entries for `default` (0) and `imported` (2³¹−1) |
| `waddrmgr/addr`, `waddrmgr/addracctidx` | empty |
| `wtxmgr` buckets | `b`, `c`, `cf`, `cmt`, `cmu`, `d`, `h`, `ic`, `id`, `m`, `mc`, `mi`, `ms`, `mu`, `t`, `tix`, `u`, `up` (no `sc`) |
| `wtxmgr` root keys | `bal` (8 zero bytes), `birthstate`, `date`, `havecfilters` = `01`, `lasttxsblock` = genesis hash, `tip` = genesis hash (no `vers`) |
| `wtxmgr/b/00000000` | genesis block record, 47 bytes, zero transactions |
| `wtxmgr/h/<genesis hash>` | serialized genesis header (180 bytes) |
| `wtxmgr/cf/<genesis hash>` | filter key ‖ version 2 genesis filter |
| `wtxmgr/birthstate` | `00` ‖ genesis hash ‖ `00 00 00 00` ‖ `ff ff ff f1 88 6e 09 00` |

The last eight bytes of `birthstate` are `BE64` of −62135596800, the Unix time of Go's zero
`time.Time` (the upgrade leaves `BirthdayState.Time` unset; `wallet/udb/upgrades.go:1680-1685`,
`wallet/udb/txmined.go:361`).

The imported account's four index variables are 0, not `0xFFFFFFFF`, because upgrade 2 does not
touch the imported account and its version 1 "next index" zeros are reinterpreted as "last used
= 0" (sections 5.4.2 and 5.4.5).

**Watching-only variant** (`InitializeWatchOnly` → `createWatchOnly`,
`wallet/udb/addressmanager.go:3090-3216`; not confirmed by a fixture — no upstream fixture is a
watching-only database). The same 25 upgrades run and everything outside `waddrmgr/main` and
the account 0 row is as in the table above. Differences before the upgrades:

| Item | Watching-only creation |
| --- | --- |
| `waddrmgr/main` keys | `cpriv`, `cpub`, `mgrcreated`, `mgrver`, `mpriv`, `mpub`, `watchonly` (= `01`). No `ctpub`, `ctpriv`, `ctpub-slip0044`, `ctpriv-slip0044`, `slip0044acct0`. |
| `mpriv` | scrypt parameters of a second key derived from the **public** passphrase (there is no private passphrase) |
| account 0 `encPub` | the supplied account xpub string sealed under the public crypto key |
| account 0 `encPriv` | the **same xpub string** sealed under the private crypto key (not a private key) |
| imported account row | `encPub` and `encPriv` empty, as in the seeded case |
| address-pool keys for account 0 | written as 0, then deleted by upgrade 2, as in the seeded case |

Upgrades 2 and 14 need only `mpub`, `cpub` and each account's `encPub`, so they run unchanged
on a watching-only database.

---

## 5. Algorithms and state machines

### 5.1 Open sequence (`wallet.Open`)

Executed in this order, each step in its own database transaction
(`wallet/wallet.go:5340-5359`, `wallet/wallet.go:5428`):

1. `NeedsMigration` — read transaction. Result: `meta` bucket absent.
2. If true: `Migrate` — one read-write transaction (section 5.5).
3. `Upgrade` — one read transaction to read the version, then (if needed) one read-write
   transaction that runs all pending upgrades (section 5.2).
4. `udb.Open` — read transaction; version gate (section 5.3), then loads the address manager.

Any error from steps 1–3 is returned as `errors.E("wallet.Open", err)` and the wallet does not
open.

### 5.2 `Upgrade` driver (`wallet/udb/upgrades.go:1693-1727`)

1. Begin a read transaction.
   1. `meta` bucket absent → return `IO` "missing metadata bucket".
   2. Read `ver`: length ≠ 4 → `IO` "bad udb version len N".
2. If `version >= 26` → return nil. A version **greater** than 26 is not an error here; it is
   rejected later by `udb.Open`.
3. Log at Info level on the wallet logger (`WLLT`):
   `Upgrading database from version <version> to 26`.
4. Begin one read-write transaction. For each function in `upgrades[version:]`, in ascending
   order, call it with `(tx, publicPassphrase, params)`. The first error aborts the loop and is
   returned.
5. If every function returned nil the transaction commits; otherwise (error or panic) it rolls
   back (`wallet/walletdb/interface.go:230-255`). A commit error is returned as is.

Consequences:

- All pending upgrades are atomic as a group. After a failure the database is byte-for-byte what
  it was (still at the old version), and the next start retries from the same version.
- Every upgrade re-reads the version from `meta/ver` inside the transaction and writes the next
  version as its last step, so each sees the previous step's write.
- A stored version of 0 passes the gate in step 2, logs, and then calls `upgrades[0]`, which is a
  nil function: the Go program panics (nil function call) after rolling back.

### 5.3 Version gate in `udb.Open` (`wallet/udb/open.go:21-54`)

| Condition | Result |
| --- | --- |
| `meta` bucket absent | `NotExist` "database has not been initialized" |
| `ver` length ≠ 4 | `IO` "bad udb version len N" |
| version < 26 | `Invalid` "database upgrade required" |
| version > 26 | `Invalid` "database has been upgraded to an unknown newer version" |
| version = 26 | continue to load the address manager |

In the normal start sequence only the "> 26" row can occur (a database written by newer
software).

### 5.4 The upgrades

#### 5.4.1 Common shape

Every upgrade function:

1. Takes `meta` with `tx.ReadWriteBucket("meta")`.
2. Reads the version (`IO` "bad udb version len N" on a malformed value).
3. If it is not the expected old version, returns `Invalid` with the message
   `<name> inappropriately called`. The `<name>` strings are listed in section 6; two of them do
   not match the function they are in.
4. Does its work.
5. Writes `meta/ver = BE32(new)`; a put failure is wrapped as `IO`.

bbolt behaviours the upgrades rely on (`bbolt@v1.3.12/bucket.go:156-338`,
`wallet/internal/bdb/db.go:17-36`):

| Operation | Behaviour |
| --- | --- |
| `Delete(key)` of an absent key | succeeds (nil) |
| `Delete(key)` where key names a nested bucket | error "incompatible value", kind `Invalid` |
| `CreateBucket` / `CreateTopLevelBucket` of an existing name | error "bucket already exists", kind `Exist` |
| `DeleteNestedBucket` of an absent name | error "bucket not found", kind `NotExist` |
| `Put` with a zero-length key | error "key required", kind `Invalid` |
| `Put(key, nil)` | stores a zero-length value |
| `Get` of a key that names a nested bucket | nil |
| cursor `First`/`Next` on a nested-bucket entry | returns the key with a nil value |
| a nested or top-level bucket lookup that fails | returns a nil bucket; any method call on it panics in Go |

Several loops are written `for k, v := c.First(); v != nil; k, v = c.Next()`: they stop at the
end of the bucket **or at the first nested bucket entry**. None of the iterated buckets
contains nested buckets in a well-formed database.

"Precondition" below lists the structures an upgrade dereferences without a nil check; if one is
missing the Go program panics instead of returning an error.

Order of the precondition dereferences relative to the version check: upgrades 2 and 4 take
their nested buckets (`waddrmgr/addr`, `waddrmgr/usedaddrs`; `waddrmgr/main`) **before** step 2,
so a missing `waddrmgr` panics even when the version is wrong; every other upgrade only fetches
top-level buckets (which may be nil without harm) before the version check and dereferences
them afterwards. `meta` itself is dereferenced by step 2 in every upgrade; `Upgrade` has already
established that it exists.

Every upgrade function has the same three parameters; `publicPassphrase` is read only by
upgrades 2 and 14, and `params` only by upgrades 2, 10, 12, 15 and 26. The other functions ignore
both.

#### 5.4.2 Version 1 → 2: `lastUsedAddressIndexUpgrade` (`wallet/udb/upgrades.go:247-402`)

Purpose: replace per-address "used" flags and address-pool cursors with one "last used child
index" per account branch.

Precondition: buckets `waddrmgr`, `waddrmgr/main`, `waddrmgr/meta`, `waddrmgr/acct`,
`waddrmgr/acctididx`, `waddrmgr/acctnameidx`, `waddrmgr/addr` (queried for child 0 of both
branches of every account); `waddrmgr/usedaddrs` is dereferenced only when a derived child has
an `addr` row (nil-bucket panic if absent then), and its absence otherwise surfaces as the `IO`
error of step 9.

Steps:

1. Version check (expects 1).
2. Read `waddrmgr/main/mpub`. Absent → `IO` "missing master pubkey params".
3. Unmarshal it (88 bytes). Failure → `IO` "unmarshal master pubkey params: <cause>".
4. Derive the key from the public passphrase with scrypt and compare digests. Any failure →
   `Passphrase` "incorrect public passphrase".
5. Read `waddrmgr/main/cpub`. Absent → `IO` "missing encrypted crypto pubkey".
6. Open it with the derived key. Failure → `Crypto` "decrypt public crypto key: <cause>". The
   plaintext is copied into a 32-byte key (shorter plaintext is zero-padded, longer truncated).
7. Read `waddrmgr/meta/lastaccount` (`LE32`). Length ≠ 4 → `IO` "bad last account len N".
8. For `account` = 0, 1, …, lastAccount (ascending):
   1. Load the account row with the version 1 codec. Errors: `NotExist` "no account N"; `IO`
      "bad account len N"; `IO` "bip0044 account <hex id> bad len N"; `IO` "unknown account type
      N".
   2. Open `encPub` with the public crypto key → `Crypto` "decrypt extended pubkey: <cause>".
   3. Parse the plaintext as a base58 extended key for `params` → failure wrapped as `IO`.
   4. Derive branch keys `xpub/0` (external) and `xpub/1` (internal) with the legacy
      `Child` derivation. An error here is returned unwrapped.
   5. `lastUsedExt = 0xFFFFFFFF`. For `child` = 0 … 2³¹−1:
      1. Derive `xpubExt/child`. `ErrInvalidChild` → skip this index (continue). Other error →
         return it.
      2. `key = SHA-256(HASH160(compressed pubkey))`.
      3. If `waddrmgr/addr` has no value for `key` → stop the loop.
      4. If `waddrmgr/usedaddrs` has a value for `key` → `lastUsedExt = child`.
   6. Same loop on the internal branch → `lastUsedInt`.
   7. Rewrite the account: row = type 0, version 2 layout with the same `encPub`, `encPriv` and
      name and the two last-used indexes; write `waddrmgr/acct[LE32(account)]`,
      `waddrmgr/acctididx[LE32(account)]` and `waddrmgr/acctnameidx[name]`
      (`wallet/udb/addressdb.go:942-952`).
   8. Delete `waddrmgr/meta/addrpoolext‖LE32(account)` then `…/addrpoolint‖LE32(account)`.
9. Delete nested bucket `waddrmgr/usedaddrs`. Failure (including "bucket not found") → `IO`.
10. Write version 2.

Notes:

- The scan stops at the first index that was never generated, so a used address beyond a gap in
  the `addr` bucket is not seen.
- The imported account row (account 2³¹−1) is not rewritten. Its bytes are layout-compatible
  (two zero `LE32` fields in the same position).
- Cost: one scrypt derivation plus one EC child derivation per generated address.
- The version 1 "next index" fields of each row are read and discarded; they do not influence
  the result.
- `lastaccount` bounds the loop inclusively; the loop ends early with `NotExist` "no account N"
  at the first account number in `0..=lastaccount` that has no row.
- The account loader accepts a type byte of 0 or 2 (section 3.6, `fetchAccountInfo`), but the row
  written back is always type 0.
- The row is rewritten even when nothing was used; step 8.8 deletes the two address-pool keys
  for every BIP0044 account whether or not they exist.
- The address returned by `compat.HD2Address` is used only for its hash160; its error result is
  discarded (`wallet/udb/upgrades.go:346`, `wallet/udb/upgrades.go:364`).

#### 5.4.3 Version 2 → 3: `votingPreferencesUpgrade` (`wallet/udb/upgrades.go:404-433`)

1. Version check (expects 2).
2. Create top-level bucket `agendaprefs`. Error returned as is (`Exist` if present).
3. Write version 3.

The original upgrade also rewrote every `wstakemgr/sstxrecords` value; that part was removed
(section 4.5).

#### 5.4.4 Version 3 → 4: `noEncryptedSeedUpgrade` (`wallet/udb/upgrades.go:435-460`)

Precondition: `waddrmgr/main`.

1. Version check (expects 3).
2. Delete `waddrmgr/main/seed` (no error if absent).
3. Write version 4.

#### 5.4.5 Version 4 → 5: `lastReturnedAddressUpgrade` (`wallet/udb/upgrades.go:462-521`)

Precondition: `waddrmgr/acct`, `acctididx`, `acctnameidx`, `meta`.

1. Version check (expects 4; the error text names `accountAddressCursorsUpgrade`).
2. Read `lastaccount`.
3. For `account` = 0 … lastAccount, then for account 2³¹−1 (imported):
   1. Load the row with the version 4 codec (errors as in 5.4.2 step 8.1).
   2. Rewrite it in the version 5 layout with
      `lastReturnedExternal = lastUsedExternal` and `lastReturnedInternal = lastUsedInternal`;
      rewrite both name indexes.
4. Write version 5.

A missing imported account row fails the upgrade with `NotExist` "no account 2147483647".

#### 5.4.6 Version 5 → 6: `ticketBucketUpgrade` (`wallet/udb/upgrades.go:523-605`)

Precondition: `wtxmgr`, `wtxmgr/t`, `wtxmgr/m`, `wtxmgr/mi`.

1. Version check (expects 5).
2. Create `wtxmgr/tix`. Error returned as is.
3. Walk `wtxmgr/t` in key order. For each entry: key shorter than 68 bytes → `IO` "tx record key
   len N" (and a Go stack trace is printed to stderr, `wallet/udb/txdb.go:578-585`); value
   shorter than 8 bytes → `IO` "tx record len N"; transaction deserialisation failure → `IO`.
   If `stake.IsSStx(tx)`, add the hash (first 32 key bytes) to a set.
4. Walk `wtxmgr/m` the same way (key shorter than 32 → `IO` "unmined key len N").
5. For each hash in the set (unordered): `tix[hash] = ff ff ff ff`.
6. Delete `wtxmgr/mi[32 zero bytes ‖ ff ff ff ff]` (the stakebase outpoint).
7. Write version 6.

`IsSStx` is evaluated with the pinned stake rules (`dcrd/blockchain/stake/v5@v5.0.3/staketx.go:816`).

#### 5.4.7 Version 6 → 7: `slip0044CoinTypeUpgrade` (`wallet/udb/upgrades.go:607-624`)

Version check (expects 6); write version 7. No data change. (Releases at version 7 add
`ctpub-slip0044`, `ctpriv-slip0044` and `slip0044acct0` to **newly created** databases only.)

#### 5.4.8 Version 7 → 8: `hasExpiryUpgrade` (`wallet/udb/upgrades.go:626-718`)

Precondition: `wtxmgr/c`, `wtxmgr/b`, `wtxmgr/t`, `wtxmgr/mc`, `wtxmgr/m`.

1. Version check (expects 7).
2. Walk `wtxmgr/c`. For each credit key `k` / value `v`:
   1. `height = BE32(k[32:36])`; load block record `b[BE32(height)]`. Absent or shorter than 47
      bytes → `IO` "block record len 4" (the number printed is the key length, see quirk Q13).
   2. Look up `t[txhash ‖ BE32(height) ‖ block hash from the block record]` and deserialise it
      (`IO` "tx record len 0" when absent).
   3. If `tx.Expiry != 0`: remember `(k, copy of v with v[8] |= 0x10)`.
3. Put every remembered pair back into `wtxmgr/c` (unordered). Put errors are returned as is.
4. Walk `wtxmgr/mc`. For each: tx hash = `k[0:32]`; load `m[txhash]` and deserialise; if
   `tx.Expiry != 0` remember `(k, v with v[8] |= 0x10)`.
5. Put every remembered pair back into `wtxmgr/mc`.
6. Write version 8.

A credit value shorter than 9 bytes panics.

#### 5.4.9 Version 8 → 9: `hasExpiryFixedUpgrade` (`wallet/udb/upgrades.go:720-836`)

Same traversal as 5.4.8. For each credit whose transaction has `Expiry != 0`:

1. `flags = v[8]`; clear bit `0x10`; set bit `0x40`.
2. Let `out` be the transaction output at the credit's index (mined: `BE32(k[68:72])`; unmined:
   `BE32(k[32:36])`, key shorter than 36 → `IO` "unmined credit key len N").
3. If `stake.IsSStx(tx)` **and** `out` is a version 0 stake-change P2PKH or P2SH script
   (`OP_SSTXCHANGE` prefix), set bit `0x10` again.
4. Store.

Credits whose transaction has no expiry are untouched. An output index beyond the transaction's
outputs panics. Write version 9.

#### 5.4.10 Version 9 → 10: `cfUpgrade` (`wallet/udb/upgrades.go:838-889`)

Precondition: `wtxmgr`.

1. Version check (expects 9).
2. `wtxmgr/havecfilters = 00` (`IO` on failure).
3. Create `wtxmgr/cf` (`IO` on failure).
4. `cf[params.GenesisHash] =` empty value (`IO` on failure).
5. If `wtxmgr/tip` equals `params.GenesisHash` (a missing or short `tip` compares as zero-padded):
   `havecfilters = 01`.
6. Write version 10.

#### 5.4.11 Version 10 → 11: `lastProcessedTxsBlockUpgrade` (`wallet/udb/upgrades.go:891-916`)

1. Version check (expects 10).
2. `wtxmgr/lasttxsblock = value of wtxmgr/tip` (`IO` on failure). A missing `tip` stores an empty
   value.
3. Write version 11.

#### 5.4.12 Version 11 → 12: `ticketCommitmentsUpgrade` (`wallet/udb/upgrades.go:918-1098`)

Purpose: build the ticket-commitment index (`cmt`) and its unspent index (`cmu`) from the stored
transactions.

Precondition: `wtxmgr/b`, `wtxmgr/t`, `wtxmgr/m`, `waddrmgr/addracctidx`.

1. Version check (expects 11).
2. Create `wtxmgr/cmt`, then `wtxmgr/cmu` (`IO` on failure).
3. Define `handle(tx, unmined)`; `type = stake.DetermineTxType(tx)`; for each output index `i`
   in order:
   - **Ticket** (`type == SStx`):
     1. `i` even → skip.
     2. Decode the commitment address from the output script (`script[2:22]` is the hash160;
        `script[29] & 0x80` selects P2SH). Script shorter than 32 bytes → `IO`.
     3. Look up `waddrmgr/addracctidx[SHA-256(hash160)]`. Absent → skip (not ours). Present →
        `account = LE32(value)`.
     4. Decode the amount: `LE64(script[22:30])` with the top bit cleared. → `IO` on short script.
     5. Debug log `Adding ticket commitment <txhash>:<i> (<amount>) for account <account>`.
     6. `k = txhash ‖ BE32(i)`; `cmt[k] = BE64(amount) ‖ BE32(account)`; `cmu[k] = 00`.
   - **Vote** (`type == SSGen`): `i < 2` → skip. `ticket = tx.TxIn[1].PreviousOutPoint.Hash`;
     `txoIdx = (i − 2)·2 + 1`.
   - **Revocation** (`type == SSRtx`): `ticket = tx.TxIn[0].PreviousOutPoint.Hash`;
     `txoIdx = i·2 + 1`.
   - For a vote or revocation: `k = ticket ‖ BE32(txoIdx)`. If `cmt[k]` is absent → skip.
     If `unmined`: debug log `Marking ticket commitment <ticket>:<txoIdx> unmined spent`;
     `cmu[k] = 01`. Otherwise: debug log `Removing unspent ticket commitment <ticket>:<txoIdx>`;
     delete `cmu[k]`. Failure → `IO`.
   - Any other type: nothing.
4. Mined pass: iterate `wtxmgr/b` from height 0 upward. For each transaction hash listed in the
   block record, in record order: find the **last** `wtxmgr/t` entry whose key starts with that
   hash (highest height/hash suffix), deserialise it (`IO` on failure, including "tx record len
   0" if there is none) and call `handle(tx, false)`.
5. Unmined pass: iterate `wtxmgr/m` in key order; deserialise; `handle(tx, true)`. Any error from
   this pass is wrapped again as `IO`.
6. Debug log `Ticket commitments db upgrade done`.
7. Write version 12.

The mined pass stops silently at the first malformed block record (quirk Q8).

Further facts about this step:

- Processing order is what makes the result correct: blocks ascend by height, and within a block
  the transactions are taken in the order the block record lists them, so a ticket's `cmt`/`cmu`
  entries exist before a later vote or revocation looks them up. A vote or revocation processed
  before its ticket finds no `cmt` entry and is skipped, leaving the ticket's `cmu` entry at
  `00` when the ticket is handled afterwards.
- All unmined transactions are handled after all mined ones, in ascending order of raw hash
  bytes — not in dependency order.
- The mined pass visits every block record in `wtxmgr/b`, including stake-invalidated blocks;
  the stake-invalidated byte is not consulted.
- A mined vote or revocation deletes only the `cmu` entry; the `cmt` entry is kept.
- `account` is whatever `addracctidx` records for the commitment hash160, so commitments to
  imported addresses are recorded under account 2³¹−1.
- The commitment address is decoded with the active `params` but only its hash160 is used; a
  P2SH commitment and a P2PKH commitment with the same 20 bytes map to the same index key.
- The `Bug` exit of `addressID` cannot occur: both address types produced by
  `AddrFromSStxPkScrCommitment` expose a hash160.
- Errors raised inside `handle` during the mined pass are returned as produced (`IO`, or `Bug`);
  the same errors raised during the unmined pass gain one more `IO` wrapper
  (`wallet/udb/upgrades.go:1066-1069`, `wallet/udb/upgrades.go:1090-1092`).
- The three Debug lines are emitted on logger `WLLT`; the amount is formatted by
  `dcrutil.Amount.String()` (coins with a ` DCR` suffix), hashes in display (reversed) hex.
- One `TxRecord` value is reused for every mined transaction (`wallet/udb/upgrades.go:1057`);
  each decode assigns its hash, received time, transaction and type anew.

#### 5.4.13 Version 12 → 13: `importedXpubAccountUpgrade` (`wallet/udb/upgrades.go:1100-1117`)

Version check (expects 12); write version 13. No data change.

#### 5.4.14 Version 13 → 14: `unencryptedRedeemScriptsUpgrade` (`wallet/udb/upgrades.go:1119-1269`)

Purpose: store P2SH redeem scripts in cleartext in the address manager and drop the duplicate
copy in the transaction store.

Precondition: `waddrmgr/main`, `waddrmgr/addr`, `waddrmgr/addracctidx`, `wtxmgr/sc`.

1. Version check (expects 13).
2. Obtain the public crypto key exactly as in 5.4.2 steps 2–6 (same errors, one scrypt run).
3. Read every value of `wtxmgr/sc` into a map `scripts: HASH160(value) → value`. The map key is
   recomputed from the value; the bucket key is ignored.
4. If nested bucket `waddrmgr/addracctidx/LE32(2³¹−1)` exists, for each non-bucket entry key
   `ck` (a `SHA-256(hash160)`):
   1. Load the address row `waddrmgr/addr[ck]` (`IO` on any failure, including absence).
   2. If it is not a script row (type 2) → skip.
   3. Open its `encHash` with the public crypto key (error returned as is). Length ≠ 20 → `IO`
      "decrypted hash160 is not 20 bytes".
   4. Remember `p2shRows[hash160] = row`.
5. For each `(hash160, row)` in `p2shRows` (unordered):
   1. `script = scripts[hash160]`. Absent → `IO` "missing cleartext script for p2sh hash160
      <40 hex digits>" (aborts the whole upgrade).
   2. Rewrite the row with rawData = `LE32(len(encHash)) ‖ encHash ‖ LE32(len(script)) ‖ script`,
      preserving its account and add-time (row byte 13 is written as 0 whatever it was); this
      also re-puts both `addracctidx` entries.
   3. Delete `wtxmgr/sc[hash160]`; remove the entry from `scripts`.
6. For each remaining `(hash160, script)` in `scripts` (unordered) — scripts known only to the
   transaction store:
   1. `encHash = seal(hash160)` under the public crypto key (fresh random 24-byte nonce).
   2. Insert a new script address row for account 2³¹−1 with add-time = now (Unix seconds), plus
      its `addracctidx` entries.
   3. Delete `wtxmgr/sc[hash160]`.
7. Delete nested bucket `wtxmgr/sc` (error returned as is; an absent `sc` bucket never gets this
   far, it panics on the nil bucket in step 3).
8. Delete `waddrmgr/main/cscript`.
9. Write version 14.

Step 6 makes this upgrade non-deterministic at the byte level (random nonce, current time).

Further facts about this step:

- Step 4 walks the address-to-account index of the imported account only; script rows recorded
  under any other account are neither rewritten nor checked, and keep their sealed `script`
  field.
- Step 5.2 writes the row under `SHA-256(decrypted hash160)`, not under the key `ck` it was read
  from. In a well-formed database they are equal; if they differ, the old row stays and a
  second row is created.
- The old sealed script is discarded without being decrypted; the script crypto key
  (`cscript`) is never opened, which is why no private passphrase is needed.
- Two `wtxmgr/sc` values with the same hash160 collapse into one map entry (the later in key
  order wins). The step 3 loop is conditioned on `v != nil`; a zero-length value read from a
  committed page is a non-nil empty slice in bbolt and is therefore read as an empty script
  rather than ending the loop (quirk Q46).
- Step 6 never checks whether the script's hash160 is already an address of another type or
  account; `putAddress` overwrites the `addr` row at that key and adds the imported-account
  index entries.
- When `waddrmgr/addracctidx/LE32(2³¹−1)` does not exist (no address was ever imported) step 4
  is skipped entirely and step 6 creates the nested bucket on first use.

#### 5.4.15 Version 14 → 15: `blockcf2Upgrade` (`wallet/udb/upgrades.go:1279-1337`)

Precondition: `wtxmgr`.

1. Version check (expects 14).
2. Delete nested bucket `wtxmgr/cf` (`IO` on failure, including absence), then create it again.
3. `wtxmgr/havecfilters = 00`.
4. Build the version 2 regular filter of `params.GenesisBlock` with a previous-script source
   that knows nothing (the genesis block spends nothing):
   `filter = blockcf2.Regular(genesis, …)`, `key = first 16 bytes of the genesis merkle root`.
   A build error is returned as is.
5. `cf[params.GenesisHash] = key ‖ filter.Bytes()`.
6. If `wtxmgr/tip == params.GenesisHash`: `havecfilters = 01`.
7. Write version 15.

After this upgrade a wallet with a tip above genesis has `havecfilters = 00` and must fetch all
main-chain filters again before processing further blocks (that procedure belongs to the
transaction-store and sync specifications).

#### 5.4.16 Version 15 → 16: `perTicketVotingPreferencesUpgrade` (`wallet/udb/upgrades.go:1339-1362`)

Version check (expects 15); create top-level bucket `ticketsagendaprefs` (error returned as is);
write version 16.

#### 5.4.17 Version 16 → 17: `accountVariablesUpgrade` (`wallet/udb/upgrades.go:1364-1417`)

Precondition: `waddrmgr/acct`, `acctididx`, `acctnameidx`.

1. Version check (expects 16).
2. Create `waddrmgr/acctvars` (`IO` on failure).
3. For each non-bucket entry of `waddrmgr/acct` in key order (key = `LE32(account)`):
   1. Load the row with `fetchDBAccount(…, 16)` (`wallet/udb/addressdb.go:803-851`). In a
      version 16 database every row is type 0 and is decoded with the version 16 codec. Any
      other type does **not** pass through untouched: a type 1 or 2 row makes the loader read
      that account's variables from `acctvars/LE32(account)`, which cannot exist yet (the
      parent bucket was created empty in step 2), so the Go code panics on the nil bucket; any
      other type byte returns `IO` "unknown account type N". The `default: return nil` arm in
      the upgrade (`wallet/udb/upgrades.go:1405-1406`) is therefore unreachable.
   2. Build a type 1 row: rawData = `LE32(len(encPub)) ‖ encPub ‖ LE32(len(encPriv)) ‖ encPriv`.
   3. Write `acct[LE32(account)]`, `acctididx`, `acctnameidx`; create (if needed) nested bucket
      `acctvars/LE32(account)`; write `extused`, `intused`, `extret`, `intret` (`LE32`) and
      `name` (raw bytes).
4. Any error from step 3 is wrapped as `IO`.
5. Write version 17.

This rewrites the bucket being iterated. With bbolt the iteration continues over the
pre-modification snapshot of the leaf, so every pre-existing account is visited exactly once;
an implementation should collect the account numbers first and then write.

The imported account (2³¹−1) is converted like any other.

Implementation detail of step 3 (`wallet/udb/upgrades.go:1387-1409`): a single
`dbBIP0044Account` value is allocated before the loop and reused for every account. Per account
the outer row struct is copied from the decoded legacy row, its type byte is then set to 1
(`actBIP0044`), the two sealed keys, four indexes and name are copied, and `rawData` is replaced
by the new serialization, so no field of a previous account leaks into the next. The value has
no unique KDF parameters, which is why `kdf-params` is never written.

#### 5.4.18 Version 17 → 18: `unpublishedTxsUpgrade` (`wallet/udb/upgrades.go:1419-1442`)

Version check (expects 17); create `wtxmgr/up` (error returned as is); write version 18.

#### 5.4.19 Version 18 → 19: `tspendPolicyUpgrade` (`wallet/udb/upgrades.go:1444-1466`)

Version check (expects 18); create top-level `treasurypolicy` (`IO` on failure); write version 19.

#### 5.4.20 Version 19 → 20: `vspBucketUpgrade` (`wallet/udb/upgrades.go:1468-1490`)

Version check (expects 19); create top-level `vsp` (error returned as is); write version 20.

#### 5.4.21 Version 20 → 21: `vspStatusUpgrade` (`wallet/udb/upgrades.go:1492-1524`)

Precondition: top-level `vsp`.

1. Version check (expects 20).
2. For every entry of `vsp`: new value = old value ‖ `00 00 00 02` (`BE32(VSPFeeProcessErrored)`).
   Collect first, then put (`IO` on failure).
3. Write version 21.

Every ticket tracked at version 20 therefore starts in the "errored" state, which makes the
wallet re-process its fee payment.

#### 5.4.22 Version 21 → 22: `tspendHashPolicyUpgrade` (`wallet/udb/upgrades.go:1526-1548`)

Version check (expects 21); create top-level `tspendpolicy` (`IO` on failure); write version 22.

#### 5.4.23 Version 22 → 23: `vspHostVersionUpgrade` (`wallet/udb/upgrades.go:1550-1616`)

Precondition: top-level `vsp`.

1. Version check (expects 22; the error text names `vspStatusUpgrade`).
2. Create top-level `vsphost`, then `vsppubkey` (`IO` on failure).
3. `vsphost[00 00 00 00] = 00 00 00 00` (host ID 0 = zero-length host).
4. `vsphost["vsphostindex"] = 00 00 00 00`.
5. For every entry of `vsp`: new value = old value ‖ `00 00 00 00` (host ID 0). Collect first,
   then put (`IO` on failure).
6. Write version 23.

#### 5.4.24 Version 23 → 24: `vspTreasuryPoliciesUpgrade` (`wallet/udb/upgrades.go:1618-1644`)

Version check (expects 23); create top-level `vsptreasurypolicy`, then `vsptspendpolicy` (`IO` on
failure); write version 24.

#### 5.4.25 Version 24 → 25: `importVotingAccountUpgrade` (`wallet/udb/upgrades.go:1646-1663`)

Version check (expects 24); write version 25. No data change.

#### 5.4.26 Version 25 → 26: `birthBlockUpgrade` (`wallet/udb/upgrades.go:1665-1689`)

Precondition: `wtxmgr`.

1. Version check (expects 25).
2. `wtxmgr/birthstate = 00 ‖ params.GenesisHash ‖ 00 00 00 00 ‖ ff ff ff f1 88 6e 09 00`
   (section 4.7). Put error returned as is.
3. Write version 26.

### 5.5 `Migrate` (`wallet/udb/migration.go:37-73`)

One read-write transaction. Precondition: top-level `waddrmgr` (with `main`) and `wtxmgr`; a file
with neither panics.

1. Address-manager legacy upgrade (`wallet/udb/addressdb.go:1723-1783`):
   1. Read `waddrmgr/main/mgrver` (`LE32`). Absent → `IO` "missing address manager version".
   2. If version < 5: delete `waddrmgr/main/lastaddrs`; write `mgrver = 5`.
   3. If version < 6: delete nested bucket `waddrmgr/sync` (`IO` if absent); write `mgrver = 6`.
   4. If the resulting version ≠ 6 (i.e. it was > 6) → `Invalid` "incompatible address manager
      version N".
2. Transaction-store legacy upgrade (`wallet/udb/txdb.go:2298-2438`):
   1. Read `wtxmgr/vers` (`BE32`). Length ≠ 4 → `IO` "no transaction store version". Value 0 →
      `IO` "invalid transaction store version 0".
   2. Version 1 → write `vers = 2`.
   3. Version 2 → write `vers = 3`; create `wtxmgr/h`, `wtxmgr/ic`, `wtxmgr/id`; for every block
      record insert a zero byte at offset 42; store the genesis header in `h`; if block record 0
      is absent store an empty genesis record; `tip = params.GenesisHash`.
   4. Version ≥ 3 → nothing (a version above 3 is accepted silently).
3. Delete `waddrmgr/main/mgrver` (`IO` on failure).
4. Delete `wtxmgr/vers` (`IO` on failure).
5. Create top-level `meta` (`IO` on failure) and write `ver = 00 00 00 01`.

The result is expected to be a version 1 unified layout (section 4.6.2 version 1 rows,
`usedaddrs`, address-pool keys). `Migrate` itself does not verify that. `wstakemgr` is not
touched.

Details of step 2 (`wallet/udb/txdb.go:2298-2438`):

- The store upgrader loops: a store at version 1 runs step 2.2 and then step 2.3 in the same
  call; it returns only once the version is ≥ 3.
- Step 2.3 in execution order: write `vers = 00 00 00 03`; create `h`, `ic`, `id` (`IO` on
  failure, including "bucket already exists"); collect every `(key, value)` of `wtxmgr/b`; for
  each, put `value[0:42] ‖ 00 ‖ value[42:]` (a value shorter than 42 bytes panics); put
  `h[params.GenesisHash] =` the 180-byte serialized genesis header; if `b[BE32(0)]` is absent,
  put the 47-byte empty genesis record (section 3.6, `valueBlockRecordEmptyFromHeader`);
  put `tip = params.GenesisHash`.
- Because step 2.3 resets `tip` to the genesis hash whatever the wallet had synced to, the
  following chain of upgrades sets `havecfilters = 01` (upgrades 10 and 15) and
  `lasttxsblock =` genesis hash (upgrade 11), and `birthstate` names genesis (upgrade 26): a
  migrated wallet re-fetches headers and rescans from the genesis block. A store that was
  already at version 3 keeps its own `tip`.
- Step 1.2 jumps from any manager version 1–4 directly to 5 (only `lastaddrs` is deleted); no
  per-version manager upgrade code for versions 2–4 exists at the pin
  (`wallet/udb/addressdb.go:1723-1762`).
- `Migrate` runs steps 3–5 even when neither legacy upgrader changed anything; afterwards
  the absence of `waddrmgr/main/mgrver` is what distinguishes a migrated database from one
  created in the unified layout (quirk Q2).

`NeedsMigration` (`wallet/udb/migration.go:24-31`) opens one read transaction and reports
whether `tx.ReadBucket("meta")` is nil; its closure always returns nil, so the returned error is
only ever the error of beginning the read transaction (`walletdb.View` discards the rollback
error: its deferred assignment is guarded by `err != nil` on a variable that is nil there and is
not the function result; `wallet/walletdb/interface.go:203-223`).

### 5.6 Creation path

`Initialize` / `InitializeWatchOnly` (`wallet/udb/initialize.go:18-89`):

1. One read-write transaction: create `waddrmgr` and `wtxmgr`; populate the address manager
   (`createAddressManager` or `createWatchOnly`) and the transaction store (`createStore`) in
   their **version 1** layouts (account rows with "next index" fields, `usedaddrs` bucket,
   address-pool keys for account 0, `sc` bucket; no `tix`, `cf`, `cmt`, `cmu`, `up`, `acctvars`);
   create `meta` with `ver = 1`.
2. `Upgrade` (section 5.2): second and third transactions.

Deviations from a historical version 1 database: no `seed` and no `cscript` key are written
(`wallet/udb/addressmanager.go:2895-3082`), the SLIP0044 keys and `slip0044acct0` row are
written (seeded wallets), and no `wstakemgr` bucket is created.

Creation performs four scrypt derivations before the wallet is first opened: two to create the
public and private master keys, one in upgrade 2 and one in upgrade 14
(`wallet/udb/addressmanager.go:2927-2935`, `wallet/udb/upgrades.go:279`,
`wallet/udb/upgrades.go:1146`).

### 5.7 VSP operations (`wallet/udb/vsp.go`)

All take a caller-supplied transaction; none starts or ends one.

#### `SetVSPTicket(dbtx, ticketHash, record)` (`wallet/udb/vsp.go:47-88`)

1. Look up `vsppubkey[record.Host]`.
2. If absent:
   1. `id = BE32(vsphost["vsphostindex"]) + 1` (wraps at 2³²; a missing counter panics).
   2. `vsphost[BE32(id)] = BE32(len(Host)) ‖ Host`.
   3. `vsphost["vsphostindex"] = BE32(id)` (`IO` on failure).
   4. `vsppubkey[Host] = BE32(id) ‖ BE32(len(record.PubKey)) ‖ record.PubKey`.
3. If present: use the stored entry; `record.PubKey` is ignored.
4. Overwrite the caller's `record.VSPHostID` with the entry's ID.
5. `vsp[ticketHash] = FeeHash ‖ BE32(FeeTxStatus) ‖ BE32(VSPHostID)`.

With an empty `Host`, step 2.4 fails with `Invalid` "key required" (bbolt rejects empty keys) and
the caller's transaction is expected to roll back.

Whether a host is "known" is decided solely by the `vsppubkey` lookup of step 1; `vsphost` is
never searched by host bytes. A host that has a `vsphost` row but no `vsppubkey` row (possible
only through a direct `SetVSPHost` call, which nothing in the tree makes) is treated as new and
receives a second ID. An error from step 1 other than `NotExist` is returned unchanged (the
pinned `GetVSPPubKey` produces none). `record.Host` is converted with `[]byte(record.Host)`:
the stored host bytes are the string's bytes verbatim. `record.FeeHash` and
`record.FeeTxStatus` are stored as given, without validation.

#### `GetVSPTicket(dbtx, ticketHash)` (`wallet/udb/vsp.go:98-125`)

1. `v = vsp[ticketHash]`; absent → `NotExist` "no VSP info for ticket <hash>" (hash in display
   form).
2. Decode the 40-byte record.
3. `host = vsphost[BE32(VSPHostID)]`; absent → leave `Host`/`PubKey` empty.
4. If the host is non-empty: `Host = host`; `pk = vsppubkey[host]`; if present, `PubKey =` its
   pubkey bytes.
5. Return the ticket.

A ticket with host ID 0 (carried over from version ≤ 22) returns an empty `Host` and nil
`PubKey`.

#### `GetVSPTicketsByFeeStatus(dbtx, feeStatus)` (`wallet/udb/vsp.go:129-145`)

Iterate `vsp`; decode each value; include it when `int(FeeTxStatus) == feeStatus`. Result is a
map (no order). The wallet-level caller turns the map into a list in map-iteration order
(`wallet/wallet.go:5625-5630`).

#### `DeleteVSPTicket(dbtx, ticketHash)` (`wallet/udb/vsp.go:92-94`)

Delete `vsp[ticketHash]`; nil when absent.

#### Fee-status state machine (as driven by the wallet)

The storage layer stores whatever status it is given. The transitions that reach it are
(`wallet/vspticket.go:246-290`):

| Call | Stored status | Stored fee hash |
| --- | --- | --- |
| `UpdateFeeStarted` | 0 | given hash |
| `UpdateFeePaid` | 1 | given hash |
| `UpdateFeeErrored` | 2 | all zero |
| `UpdateFeeConfirmed` | 3 | given hash |
| upgrade 20→21 | 2 | unchanged |

Every `UpdateFee*` call passes the VSP client's configured URL as `Host` and its public key as
`PubKey`, so each status write also (re)assigns the ticket to that host's ID — a ticket carried
over with host ID 0 acquires a real host ID at its next status write.

How the stored status is consumed (`wallet/vsp.go:221-286`): `VSPClient.Process` reads the
record through `GetVSPTicket`; `NotExist` is treated as status 0. Status 0 or 2 → start a new fee
payment (and write status 2 on each failure before the fee is submitted); status 1 → confirm the
existing payment, unless the stored `Host` is non-empty and differs from the client URL, which
fails with `ticket already paid or confirmed with another vsp`; status 3 → nothing. Any other
stored status value is a silent no-op. The full client behaviour belongs to the VSP
specification; the storage layer only has to return the stored `uint32` unchanged.

Wallet-level readers add their own `NotExist` on top of `GetVSPTicket`
(`wallet/wallet.go:5653-5701`): `VSPFeeHashForTicket` returns a bare `NotExist` error (no
message) when the stored fee hash is all zero — the state every errored ticket is in —
and `VSPHostForTicket` does the same when the resolved host is empty. `IsVSPTicketConfirmed`
is true only for status 3.

#### Host and pubkey accessors (`wallet/udb/vsp.go:186-289`)

| Function | Steps |
| --- | --- |
| `SetVSPHost(dbtx, id, record)` | `vsphost[BE32(id)] = BE32(len(Host)) ‖ Host`; put error returned unwrapped. Does not read or write `vsphostindex`. |
| `GetVSPHost(dbtx, id)` | `v = vsphost[BE32(id)]`; nil → `NotExist`; else `n = BE32(v[0:4])`, `Host = v[4:4+n]` (sub-slice of database memory; bytes after `4+n` are ignored; `n` larger than the remainder panics). |
| `SetVSPPubKey(dbtx, host, record)` | `vsppubkey[host] = BE32(ID) ‖ BE32(len(PubKey)) ‖ PubKey`; put error returned unwrapped. |
| `GetVSPPubKey(dbtx, host)` | `v = vsppubkey[host]`; nil → `NotExist`; else `ID = BE32(v[0:4])`, `n = BE32(v[4:8])`, `PubKey = v[8:8+n]` (sub-slice; trailing bytes ignored; short value panics). A zero-length `host` is looked up as an empty key, which bbolt reports as absent. |

Record decoding in `GetVSPTicket` / `GetVSPTicketsByFeeStatus` reads exactly bytes `[0:40]`;
bytes beyond offset 40 are ignored. `GetVSPTicketsByFeeStatus` compares
`int(FeeTxStatus) == feeStatus`, so a negative argument or one above 2³²−1 matches nothing, and
the argument is not restricted to 0..3.

### 5.8 Treasury policy operations (`wallet/udb/treasury.go`)

Setters (`SetTSpendPolicy`, `SetVSPTSpendPolicy`, `SetTreasuryKeyPolicy`,
`SetVSPTreasuryKeyPolicy`):

1. Build the key (section 4.4).
2. If `policy == 0x00`: delete the key; a delete error is wrapped as `IO`; return.
3. Otherwise put `[policy]` (one byte); a put error is returned as is.

The setters do not validate `policy`; the wallet layer restricts it to `0x00`, `0x01`, `0x02`
before calling (`wallet/wallet.go:669-674`, `wallet/wallet.go:719-724`).

Single readers (`TSpendPolicy`, `VSPTSpendPolicy`, `TreasuryKeyPolicy`,
`VSPTreasuryKeyPolicy`):

1. Get the value. Absent → `(0x00, nil)`.
2. Length ≠ 1 → `IO` "invalid length N for tspend vote policy" (hash buckets) or "invalid length
   N for treasury key policy" (key buckets).
3. Return the byte.

Bulk readers (`TSpendPolicies`, `VSPTSpendPolicies`, `TreasuryKeyPolicies`,
`VSPTreasuryKeyPolicies`): iterate the bucket; for each key call the single reader (a second
lookup); skip `0x00`; add to the map. On error the partially filled map is returned together
with the error. Key decoding: `tspendpolicy` copies up to the first 32 key bytes (a shorter key
is zero-padded, a longer one truncated); `vsptspendpolicy` copies `k[0:32]` and `k[32:]` (each
truncated to 32) and `vsptreasurypolicy` splits at byte 32 — in both per-ticket buckets a key
shorter than 32 bytes panics (`k[32:]` out of range; `wallet/udb/treasury.go:134`,
`wallet/udb/treasury.go:249`).

Consequences of the re-lookup for keys of unexpected length (none are written by the pinned
setters for the two hash-keyed buckets):

| Bucket | Stored key length | Bulk reader result |
| --- | --- | --- |
| `tspendpolicy` | ≠ 32 | The padded or truncated 32-byte key is looked up; when no entry with exactly that 32-byte key exists it reads as `0x00` and the entry is **silently omitted** (when one exists, that entry's policy is recorded again under the same map key, so the map is unchanged). |
| `vsptspendpolicy` | 32..63 or > 64 | Same, with the rebuilt 64-byte key (zero-padded or truncated). |
| `vsptspendpolicy` | < 32 | Panic. |
| `treasurypolicy` | any ≥ 1 | Looked up with the identical key; returned under `string(key)`. |
| `vsptreasurypolicy` | ≥ 32 | Rebuilt key is identical; returned with `TreasuryKey = string(key[32:])` (empty string when the key is exactly 32 bytes). |
| `vsptreasurypolicy` | < 32 | Panic. |

A stored value of `0x00` (never written by the setters) is omitted by all four bulk readers; a
stored value of any other single byte (e.g. `0x03`) is returned as is. A nested bucket inside any
of the four buckets reads as absent and is skipped (in the two per-ticket buckets only when its
name is at least 32 bytes long; a shorter name panics like any short key).

The returned maps own their keys: `chainhash.Hash` arrays are copied and the pi key is converted
to a string, so nothing aliases database memory.

Wallet-level use: the four maps are loaded once in `wallet.Open` and kept in memory under
`stakeSettingsLock`; setters write the database first and then update the in-memory map
(`wallet/wallet.go:5497-5517`, `wallet/wallet.go:666-762`).

---

## 6. Constants, limits and defaults

### 6.1 Database versions

"Class": **B** = version bump only; **C** = creates empty bucket(s) or a fixed key; **R** = small
mechanical rewrite of existing records; **L** = logic over existing data. "First release" is the
first tagged program release whose `DBVersion` is at least that version (local git tags;
`wallet/udb/upgrades.go` at each tag). Dates are commit dates of the change that introduced the
version.

| Ver | Constant | `<name>` in the "inappropriately called" error | Class | Introduced | First release |
| --- | --- | --- | --- | --- | --- |
| 1 | `initialVersion` | — | — | 2017-03-06 | none (1.0.0 shipped 3) |
| 2 | `lastUsedAddressIndexVersion` | `lastUsedAddressIndexUpgrade` | L | 2017-04-06 | none |
| 3 | `votingPreferencesVersion` | `votingPreferencesUpgrade` | C | 2017-04-11 | 1.0.0 |
| 4 | `noEncryptedSeedVersion` | `noEncryptedSeedUpgrade` | C (deletes one key) | 2017-05-01 | 1.0.3 |
| 5 | `lastReturnedAddressVersion` | `accountAddressCursorsUpgrade` (sic) | R | 2017-06-12 | 1.0.5 |
| 6 | `ticketBucketVersion` | `ticketBucketUpgrade` | L | 2017-08-10 | 1.0.7 |
| 7 | `slip0044CoinTypeVersion` | `slip0044CoinTypeUpgrade` | B | 2017-11-08 | 1.1.2 |
| 8 | `hasExpiryVersion` | `hasExpiryUpgrade` | L | 2018-02-21 | 1.2.0 (shipped 9) |
| 9 | `hasExpiryFixedVersion` | `hasExpiryFixedUpgrade` | L | 2018-02-27 | 1.2.0 |
| 10 | `cfVersion` | `cfUpgrade` | C | 2018-07-26 | 1.3.0 (shipped 11) |
| 11 | `lastProcessedTxsBlockVersion` | `lastProcessedTxsBlockUpgrade` | C | 2018-07-26 | 1.3.0, 1.4.x |
| 12 | `ticketCommitmentsVersion` | `ticketCommitmentsUpgrade` | L | 2019-06-13 | 1.5.0 (shipped 13) |
| 13 | `importedXpubAccountVersion` | `importedXpubAccountUpgrade` | B | 2019-10-16 | 1.5.x |
| 14 | `unencryptedRedeemScriptsVersion` | `unencryptedRedeemScriptsUpgrade` | L | 2020-03-10 | 1.6.0 (shipped 22) |
| 15 | `blockcf2Version` | `blockcf2Upgrade` | R | 2020-03-13 | 1.6.0 |
| 16 | `perTicketVotingPreferencesVersion` | `perTicketVotingPreferencesUpgrade` | C | 2020-05-19 | 1.6.0 |
| 17 | `accountVariablesVersion` | `accountVariablesUpgrade` | L | 2020-08-17 | 1.6.0 |
| 18 | `unpublishedTxsVersion` | `unpublishedTxsUpgrade` | C | 2020-09-04 | 1.6.0 |
| 19 | `tspendPolicyVersion` | `tspendPolicyUpgrade` | C | 2020-09-21 | 1.6.0 |
| 20 | `vspBucketVersion` | `vspBucketUpgrade` | C | 2020-10-06 | 1.6.0 |
| 21 | `vspStatusVersion` | `vspStatusUpgrade` | R | 2020-10-19 | 1.6.0 |
| 22 | `tspendHashPolicyVersion` | `tspendHashPolicyUpgrade` | C | 2020-10-29 | 1.6.x |
| 23 | `vspHostVersion` | `vspStatusUpgrade` (sic) | R | 2021-08-20 | 1.7.0 (shipped 25) |
| 24 | `vspTreasuryPoliciesVersion` | `vspTreasuryPoliciesUpgrade` | C | 2021-12-07 | 1.7.0 |
| 25 | `importVotingAccountVersion` | `importVotingAccountUpgrade` | B | 2021-12-22 | 1.7.x, 1.8.x |
| 26 | `birthBlockVersion` = `DBVersion` | `birthBlockUpgrade` | C | 2024-05-13 | 2.0.x, 2.1.x |

Constants: `wallet/udb/upgrades.go:30-214`; table: `wallet/udb/upgrades.go:219-245`.

Database version shipped by tagged releases (local tags `v1.0.0` … `v1.2.0`,
`release-v1.3.0` … `release-v2.1.7`):

| Release | Date | `DBVersion` |
| --- | --- | --- |
| 0.8.2 and earlier | ≤ 2017-02 | pre-unified (no `meta`) |
| 1.0.0, 1.0.1 | 2017-04 | 3 |
| 1.0.3, 1.0.4 | 2017-06 | 4 |
| 1.0.5 | 2017-06 | 5 |
| 1.0.7, 1.0.8, 1.1.0 | 2017-08 … 2017-09 | 6 |
| 1.1.2 | 2017-12 | 7 |
| 1.2.0 | 2018-04 | 9 |
| 1.3.0, 1.4.0, 1.4.1 | 2018-08 … 2019-05 | 11 |
| 1.5.0, 1.5.1 | 2019-11 … 2020-01 | 13 |
| 1.6.0 … 1.6.3 | 2021-01 … 2021-05 | 22 |
| 1.7.0 … 1.7.7, 1.8.0, 1.8.1 | 2022-01 … 2023-09 | 25 |
| 2.0.0 … 2.0.6, 2.1.0 … 2.1.7 | 2024-05 … 2026-10 | 26 |

Versions 1, 2, 8, 10, 12, 14–21, 23 and 24 were never the version of a final tagged release; a
database rests at one of them only if it was last opened by a development build or, for
version 21, by the release candidate 1.6.0-rc1 (tags `v1.6.0-rc1` / `release-v1.6.0-rc1`,
2020-10-20, `DBVersion = vspStatusVersion`; rc2 onward ship 22). Version 12 was also the head
version at the library-module tags of June–October 2019 (e.g. `spv/v2.1.2`). Version 10 was
never the head version even in development: versions 10 and 11 were introduced by the same
commit (`97b81d0e`).

### 6.2 Other constants

| Name | Value | Citation | Depends on it |
| --- | --- | --- | --- |
| `DBVersion` | 26 | `wallet/udb/upgrades.go:213` | version gate; upgrade target |
| `initialVersion` | 1 | `wallet/udb/upgrades.go:31` | creation and migration start version |
| metadata bucket | `meta` | `wallet/udb/metadata.go:15` | unified-layout marker |
| version key | `ver` | `wallet/udb/metadata.go:19` | |
| `latestMgrVersion` | 6 | `wallet/udb/addressdb.go:21` | legacy manager version required by `Migrate`; value left in `mgrver` |
| legacy tx-store final version | 3 | `wallet/udb/txdb.go:2313-2320` | `Migrate` |
| `ImportedAddrAccount` | 2³¹−1 (`0x7FFFFFFF`) | `wallet/udb/addressmanager.go:47` | upgrades 5, 14 |
| `DefaultAccountNum` | 0 | `wallet/udb/addressmanager.go:53` | |
| `ExternalBranch`, `InternalBranch` | 0, 1 | `wallet/udb/addressmanager.go:80-85` | upgrade 2 |
| `hdkeychain.HardenedKeyStart` | 2³¹ | `dcrd/hdkeychain/v3@v3.1.3/extendedkey.go:36` | upgrade 2 scan bound |
| "no index" sentinel | `0xFFFFFFFF` | `wallet/udb/upgrades.go:331-332` | last-used / last-returned fields |
| picked-height sentinel | −1 (`ff ff ff ff`) | `wallet/udb/upgrades.go:589` | `tix` values written by upgrade 6 |
| stakebase outpoint key | 32 × `00` ‖ `ff ff ff ff` | `wallet/udb/upgrades.go:597` | upgrade 6 deletion |
| has-expiry bit, version 8 | `0x10` | `wallet/udb/upgrades.go:666` | credit flags |
| has-expiry bit, version ≥ 9 | `0x40` | `wallet/udb/upgrades.go:761` | credit flags |
| `wire.NoExpiryValue` | 0 | `dcrd/wire@v1.8.0/msgtx.go:40` | upgrades 8, 9 |
| `stake.SStxPKHMinOutSize` | 32 | `dcrd/blockchain/stake/v5@v5.0.3/staketx.go:81` | upgrade 12 commitment decoding |
| blockcf2 `B`, `M` | 19, 784931 | `dcrd/gcs/v4@v4.1.1/blockcf2/blockcf.go:33-38` | upgrade 15 genesis filter |
| filter key size | 16 | `wallet/udb/txdb.go:1981` | `cf` values |
| `hashSize` | 32 | `wallet/udb/vsp.go:19` | VSP record |
| VSP ticket record size | 40 | `wallet/udb/vsp.go:167` | `vsp` values |
| `VSPFeeProcessStarted/Paid/Errored/Confirmed` | 0 / 1 / 2 / 3 | `wallet/udb/vsp.go:25-36` | `vsp` status field; gRPC enum |
| reserved VSP host ID | 0 (zero-length host) | `wallet/udb/upgrades.go:1575-1585` | tickets from version ≤ 22 |
| host counter key | `vsphostindex` | `wallet/udb/txdb.go:162` | host ID allocation |
| `TreasuryVoteInvalid/Yes/No` | `0x00` / `0x01` / `0x02` | `dcrd/blockchain/stake/v5@v5.0.3/staketx.go:828-836` | policy values |
| policy value length | 1 | `wallet/udb/treasury.go:75`, `wallet/udb/treasury.go:193` | readers |
| snacl `KeySize`, `NonceSize`, `Overhead` | 32, 24, 16 | `wallet/internal/snacl/snacl.go:21-25` | upgrades 2, 14 |
| marshalled scrypt parameters | 88 bytes | `wallet/internal/snacl/snacl.go:129-150` | `mpub` |
| scrypt parameters for new keys | N = 262144, r = 8, p = 1; simnet N = 2, r = 1, p = 1 | `wallet/udb/addressmanager.go:119-129` | creation; upgrades use the stored parameters |
| bbolt `MaxKeySize`, `MaxValueSize` | 32768, 2³¹−2 | `bbolt@v1.3.12/bucket.go:10-14` | pi key and host key length limits |
| log line | `Upgrading database from version %d to %d` (Info, subsystem `WLLT`) | `wallet/udb/upgrades.go:1715`, `log.go:33` | operator-visible |
| default public passphrase | `public` (`wallet.InsecurePubPassphrase`) | `wallet/wallet.go:66`, `config.go:351` | passphrase the daemon hands to `Upgrade` unless configured |
| test public / private passphrase | `public` / `private` | `wallet/udb/upgrades_test.go:40`, testdata generators | fixtures |
| minimum account row length | 5 (`type` + `rdlen`) | `wallet/udb/addressdb.go:526-528` | upgrades 2, 5, 17 |
| minimum legacy BIP0044 rawData length | 20 (version < 5), 28 (version ≥ 5) | `wallet/udb/addressdb.go:567-571` | upgrades 2, 5, 17 |
| minimum type 1/2 rawData length | 8 | `wallet/udb/addressdb.go:116-119` | rows written by upgrade 17 |
| minimum address row length | 18 | `wallet/udb/addressdb.go:1120-1122` | upgrade 14 |
| minimum script rawData length | 8 | `wallet/udb/addressdb.go:1250-1252` | upgrade 14 |
| address-pool key length | 15 (11-byte prefix + 4) | `wallet/udb/addressdb.go:1593-1605` | upgrade 2 deletions |
| minimum block record length | 47 | `wallet/udb/txdb.go:312-314` | upgrades 8, 9, 12 |
| transaction record key / minimum value length | 68 / 8 | `wallet/udb/txdb.go:561-585` | upgrades 6, 8, 9, 12 |
| mined credit key length; flags offset | 72; value byte 8 | `wallet/udb/txdb.go:818-833`, `wallet/udb/upgrades.go:666` | upgrades 8, 9 |
| `unconfCreditKeySize` | 36 | `wallet/udb/txdb.go:1450` | upgrade 9 |
| legacy block record insert offset | 42 | `wallet/udb/txdb.go:2392-2396` | `Migrate` (store 2→3) |
| serialized block header size | 180 | `wallet/udb/txdb.go:2403-2411` | `Migrate` genesis header |
| ticket commitment value size; unspent flag size | 12; 1 | `wallet/udb/txdb.go:2025-2030`, `wallet/udb/txdb.go:2078-2087` | upgrade 12 |
| `birthstate` value size | 45 (1 + 32 + 4 + 8) | `wallet/udb/txmined.go:343-351` | upgrade 26 |
| script hash160 length checked by upgrade 14 | 20 | `wallet/udb/upgrades.go:1202` | upgrade 14 |
| number of upgrade functions / table length | 25 / 26 | `wallet/udb/upgrades.go:219-245` | driver loop |
| gRPC fee-status enum | `VSP_FEE_PROCESS_STARTED = 0`, `_PAID = 1`, `_ERRORED = 2`, `_CONFIRMED = 3` | `rpc/api.proto:1277-1280` | `GetVSPTicketsByFeeStatus` |

---

## 7. Concurrency and lifecycle

- **No goroutines, channels or mutexes exist in this scope.** `Upgrade`, `Migrate` and every
  VSP and treasury function run synchronously in the caller's goroutine.
- **Transactions.** `NeedsMigration`: one read transaction. `Migrate`: one read-write
  transaction. `Upgrade`: one read transaction, then at most one read-write transaction covering
  every pending upgrade. VSP and treasury functions run inside whatever transaction the caller
  supplies. bbolt allows one writer at a time; a read-write transaction holds the writer lock
  until commit or rollback.
- **Context.** `ctx` is passed to `walletdb.View`/`walletdb.Update` only to open runtime trace
  regions (`wallet/walletdb/interface.go:203-255`). It is never checked for cancellation: a
  started upgrade runs to completion or error.
- **Process level.** `dcrwallet` opens the wallet in a goroutine and waits for either the result
  or context cancellation (`dcrwallet.go:208-230`). An interrupt during a long upgrade makes the
  main flow return while the upgrade goroutine is still inside its transaction; the process then
  exits without committing. Because the upgrade is one bbolt transaction, the file is left at the
  old version.
- **Ordering guarantee visible to callers.** Either all pending upgrades are applied and `ver`
  is 26, or nothing changed.
- **Lifecycle.** Upgrades run before `udb.Open` constructs the address manager and transaction
  store, so no in-memory state has to be refreshed afterwards. They also run at the end of wallet
  creation (section 5.6).
- **Memory.** Upgrades 6, 8, 9, 14, 21 and 23 collect the records to rewrite in memory before
  writing (bbolt cursors may not be used across modifications of the same bucket). Upgrade 14
  holds every stored redeem script in memory. Each scrypt derivation at N = 2¹⁸, r = 8 needs
  about 256 MiB; the Go code calls `debug.FreeOSMemory()` after each
  (`wallet/internal/snacl/snacl.go:104-125`).
- **Wallet-level locking around this scope.**
  - Treasury setters: `stakeSettingsLock` is taken first and held across both the database
    update and the in-memory map update (`wallet/wallet.go:676-711`,
    `wallet/wallet.go:726-761`). Lock order: `stakeSettingsLock` → database writer.
  - VSP setters: each `UpdateFee*` is its own read-write transaction with no wallet lock
    (`wallet/vspticket.go:246-290`).
  - `DeleteVSPTicket` runs inside the block-connect transaction
    (`wallet/chainntfns.go:261-275`).
- **Failed open.** When `wallet.Open` returns an error (including any migration or upgrade
  error) the loader closes the database handle it had opened, so the file lock is released and
  a later open attempt does not hang (`internal/loader/loader.go:308-316`).
- **Logger.** The package logger is a process-wide variable that is disabled until
  `UseLogger` is called (`wallet/udb/log.go:10-20`); it is read without synchronisation.
- **Tests.** The seven upgrade subtests run in parallel, each on its own database file; nothing
  in scope shares state between databases.
- **Borrowed memory.** Values returned by bbolt are valid only until the transaction ends.
  `GetVSPTicket` returns `PubKey` as a sub-slice of the stored value without copying
  (`wallet/udb/vsp.go:121`, `wallet/udb/vsp.go:269`), and wallet code keeps it after the
  transaction (`wallet/vspticket.go:331-337`). `Host` is copied (string conversion).

---

## 8. Errors

Error kinds are those of the wallet `errors` package. The table gives the innermost kind and
message produced in scope. Callers wrap them with an operation name (`wallet.Open`,
`loader.OpenExistingWallet`, `wallet.Create`, …).

| Origin | Kind | Message | Citation |
| --- | --- | --- | --- |
| `Upgrade` | `IO` | `missing metadata bucket` | `wallet/udb/upgrades.go:1701` |
| `getVersion` (everywhere) | `IO` | `bad udb version len %d` | `wallet/udb/metadata.go:34` |
| each upgrade | `Invalid` | `<name> inappropriately called` (names in section 6.1) | e.g. `wallet/udb/upgrades.go:267` |
| `putVersion` | `IO` | driver error | `wallet/udb/metadata.go:24-27` |
| `udb.Open` | `NotExist` | `database has not been initialized` | `wallet/udb/open.go:27` |
| `udb.Open` | `Invalid` | `database upgrade required` | `wallet/udb/open.go:34` |
| `udb.Open` | `Invalid` | `database has been upgraded to an unknown newer version` | `wallet/udb/open.go:37` |
| upgrades 2, 14 | `IO` | `missing master pubkey params` | `wallet/udb/addressdb.go:324` |
| upgrades 2, 14 | `IO` | `unmarshal master pubkey params: %v` | `wallet/udb/upgrades.go:277`, `wallet/udb/upgrades.go:1144` |
| upgrades 2, 14 | `Passphrase` | `incorrect public passphrase` | `wallet/udb/upgrades.go:281`, `wallet/udb/upgrades.go:1148` |
| upgrades 2, 14 | `IO` | `missing encrypted crypto pubkey` | `wallet/udb/addressdb.go:449` |
| upgrades 2, 14 | `Crypto` | `decrypt public crypto key: %v` | `wallet/udb/upgrades.go:290`, `wallet/udb/upgrades.go:1156` |
| upgrades 2, 5 | `IO` | `bad last account len %d` | `wallet/udb/addressdb.go:716` |
| upgrades 2, 5, 17 | `NotExist` | `no account %d` | `wallet/udb/addressdb.go:780` |
| upgrades 2, 5, 17 | `IO` | `bad account len %d`; `bip0044 account %x bad len %d`; `unknown account type %d` | `wallet/udb/addressdb.go:527`, `wallet/udb/addressdb.go:570`, `wallet/udb/addressdb.go:800` |
| upgrade 2 | `Crypto` | `decrypt extended pubkey: %v` | `wallet/udb/upgrades.go:314` |
| upgrade 2 | `IO` | hdkeychain parse error (bad checksum, wrong network, bad length) | `wallet/udb/upgrades.go:316-319` |
| upgrade 2 | none (raw) | hdkeychain child-derivation error | `wallet/udb/upgrades.go:320-327`, `wallet/udb/upgrades.go:338-340` |
| upgrades 6, 8, 9, 12 | `IO` | `tx record key len %d`; `tx record len %d`; transaction decode error; `unmined key len %d` | `wallet/udb/txdb.go:561-585`, `wallet/udb/txdb.go:1365-1371` |
| upgrades 8, 9 | `IO` | `block key len %d`; `block record len %d`; `%d tx block record len %d` | `wallet/udb/txdb.go:308-321` |
| upgrade 9 | `IO` | `unmined credit key len %d` | `wallet/udb/txdb.go:1500-1505` |
| upgrade 12 | `IO` | stake rule error `short read of sstx commit pkscript` | `wallet/udb/upgrades.go:956-959` |
| upgrade 12 | `Bug` | `address id cannot be created from type %T (requires Hash160 method)` | `wallet/udb/upgrades.go:961-964` |
| upgrade 14 | `IO` | `no address with hash %x` and address-row decode errors | `wallet/udb/upgrades.go:1189-1192` |
| upgrade 14 | `Invalid` / `Crypto` | `cryptokey.Decrypt` errors (`missing nonce`, authentication failure) | `wallet/udb/upgrades.go:1198-1201` |
| upgrade 14 | `IO` | `decrypted hash160 is not 20 bytes` | `wallet/udb/upgrades.go:1203` |
| upgrade 14 | `IO` | `missing cleartext script for p2sh hash160 %x` | `wallet/udb/upgrades.go:1219-1220` |
| bucket creation in upgrades 3, 6, 16, 18, 20 | `Exist` | `bucket already exists` (driver error, not wrapped) | `wallet/internal/bdb/db.go:32-33` |
| bucket creation / deletion in upgrades 2, 10, 12, 15, 17, 19, 22, 23, 24 | `IO` | wraps the driver error | e.g. `wallet/udb/upgrades.go:397` |
| `Migrate` | `IO` | `missing address manager version` | `wallet/udb/addressdb.go:295` |
| `Migrate` | `Invalid` | `incompatible address manager version %d` | `wallet/udb/addressdb.go:1779` |
| `Migrate` | `IO` | `no transaction store version`; `invalid transaction store version 0` | `wallet/udb/txdb.go:2301`, `wallet/udb/txdb.go:2307` |
| `GetVSPTicket` | `NotExist` | `no VSP info for ticket %v` (display-order hash) | `wallet/udb/vsp.go:102-103` |
| `GetVSPHost` | `NotExist` | `no VSP host info for id %v` — argument is a pointer (quirk Q22) | `wallet/udb/vsp.go:203-204` |
| `GetVSPPubKey` | `NotExist` | `no VSP pubkey info for host %v` — argument is a pointer to a byte slice (quirk Q22) | `wallet/udb/vsp.go:254-255` |
| `SetVSPTicket` | `IO` | driver error on the counter put | `wallet/udb/vsp.go:65-68` |
| `SetVSPTicket`, `SetVSPHost`, `SetVSPPubKey` | driver kind | raw driver error from `Put` (e.g. `Invalid` `key required` for an empty host) | `wallet/udb/vsp.go:87`, `wallet/udb/vsp.go:193`, `wallet/udb/vsp.go:246` |
| treasury single readers | `IO` | `invalid length %v for tspend vote policy`; `invalid length %v for treasury key policy` | `wallet/udb/treasury.go:75-78`, `wallet/udb/treasury.go:193-196` |
| treasury setters | `IO` (delete) / driver kind (put) | driver error | `wallet/udb/treasury.go:37-44` |
| upgrade 12 | `IO` | wrapper around any error of the commitment-account lookup other than `NotExist` (the pinned helper produces no such error) | `wallet/udb/upgrades.go:965-972` |
| upgrade 14 | `IO` | `bad address len %d`; `unknown address type %d`; `bad script address len %d` (inside the `IO` wrapper of step 4.1) | `wallet/udb/addressdb.go:1111-1134`, `wallet/udb/addressdb.go:1241-1268`, `wallet/udb/addressdb.go:1318` |
| upgrade 14 | `IO` | put errors of `putAddress` / `putAddrAccountIndex` | `wallet/udb/addressdb.go:907-926`, `wallet/udb/addressdb.go:1337-1350` |
| upgrade 14 | driver kind | `wtxmgr/sc` delete errors and the `DeleteNestedBucket("sc")` error, unwrapped | `wallet/udb/upgrades.go:1228-1231`, `wallet/udb/upgrades.go:1255-1258` |
| upgrade 15 | none (raw) | `blockcf2.Regular` error, unwrapped | `wallet/udb/upgrades.go:1314-1317` |
| upgrade 17 | `IO` wrapping the inner error | any error of the per-account callback. The account-variable reader builds `bad len %d for uint32 value "%s"`, but `fetchDBAccount` drops that message and returns a bare `IO` (quirk Q50); these can only arise for type 1/2 rows, which a version 16 database does not contain (and there the missing variables bucket panics first, section 5.4.17) | `wallet/udb/upgrades.go:1411-1413`, `wallet/udb/addressdb.go:821-832`, `wallet/udb/addressdb.go:1074-1085` |
| upgrade 26 | driver kind | `birthstate` put error, unwrapped | `wallet/udb/upgrades.go:1683-1685` |
| `Migrate` | `IO` | driver errors of: deleting `lastaddrs`, deleting bucket `sync` (`bucket not found` when absent), writing `mgrver`, writing `vers`, creating `h` / `ic` / `id`, rewriting block records, writing `tip`, deleting `mgrver` and `vers`, creating `meta` | `wallet/udb/addressdb.go:1697-1719`, `wallet/udb/txdb.go:2332-2436`, `wallet/udb/migration.go:59-70` |
| `walletdb.View` / `walletdb.Update` (around `NeedsMigration`, `Migrate`, `Upgrade`) | driver kind | begin errors (both) and the commit error (`Update` only) of the storage engine; rollback errors are never returned by either | `wallet/walletdb/interface.go:203-255` |
| gRPC `GetVSPTicketsByFeeStatus` | gRPC `InvalidArgument` | `fee status=%v` for an enum value outside the four defined ones | `internal/rpc/rpcserver/server.go:4061-4063` |
| `wallet.VSPFeeHashForTicket`, `wallet.VSPHostForTicket` | `NotExist` | no message (bare kind) when the stored fee hash is zero / the resolved host is empty | `wallet/wallet.go:5663-5665`, `wallet/wallet.go:5680-5682` |
| `wallet.GetVSPTicketsByFeeStatus` | as produced | wrapped with operation `wallet.GetVSPTicketsByFeeStatus` | `wallet/wallet.go:5613-5622` |

Complete list of error constructions inside the four source files in scope (mechanical
inventory): 25 × `Invalid` "… inappropriately called" (one per upgrade); 2 × `IO` "unmarshal
master pubkey params: %v"; 2 × `Passphrase` "incorrect public passphrase"; 2 × `Crypto` "decrypt
public crypto key: %v"; 1 × `Crypto` "decrypt extended pubkey: %v"; 1 × `IO` "decrypted hash160 is
not 20 bytes"; 1 × `IO` "missing cleartext script for p2sh hash160 %x"; 1 × `IO` "missing
metadata bucket"; 1 × `Bug` (upgrade 12); bare `IO` wrappers of lower-level errors in upgrades 2,
10, 11, 12, 14, 15, 17, 19, 21, 22, 23, 24 and in `Migrate` (3); in `vsp.go` 3 × `NotExist`
("no VSP info for ticket %v", "no VSP host info for id %v", "no VSP pubkey info for host %v")
and 1 × `IO` wrapper; in `treasury.go` 4 × `IO` "invalid length %v for …" and 4 × `IO` delete
wrappers. Nothing in scope constructs a JSON-RPC or gRPC status error.

Surfacing:

- **Daemon start.** An upgrade or migration error makes `wallet.Open` fail; `dcrwallet` logs
  `Failed to open wallet: <error>` at Error level and exits with status 1 (`dcrwallet.go:45-53`,
  `dcrwallet.go:211-229`). Nested error strings are joined with `:: ` in the daemon
  (`dcrwallet.go:36-39`).
- **Passphrase kind.** If the error matches kind `Passphrase` and the configured public
  passphrase is the default, the daemon additionally logs
  `Configure public passphrase with walletpass or promptpublicpass options.`
  (`dcrwallet.go:214-219`). Upgrades 2 and 14 produce that kind for a wrong public passphrase.
- **Wallet creation.** The same errors surface from `wallet.Create` / `CreateWatchOnly` through
  the loader (`internal/loader/loader.go:163-166`, `internal/loader/loader.go:253-256`).
- **Panics.** Malformed data that the Go code indexes without a length check (sections 5.4 and
  5.7) terminates the process with a Go runtime panic after the transaction is rolled back. An
  implementation should return an `IO` error instead; no client can depend on the panic.
- **VSP / treasury errors** propagate to the RPC that triggered them; their mapping to JSON-RPC
  and gRPC status codes belongs to the RPC specifications.

---

## 9. Compatibility-critical behaviours

Each item names what depends on it.

1. **Version record.** `meta/ver` is a 4-byte big-endian integer and the replacement must write
   `00 00 00 1a` after upgrading and require exactly 26 to operate. *Depends:* the Go wallet
   opening the same file afterwards (it refuses any other value), and vice versa.
2. **Refusal of newer databases.** A stored version above 26 must not be modified and must be
   reported as `Invalid` "database has been upgraded to an unknown newer version". *Depends:*
   safety of a database touched by a newer Go release.
3. **Atomic multi-step upgrade.** All pending steps commit together or not at all. *Depends:*
   crash safety; a user downgrading after a failed start must find the old version intact.
4. **Exact post-upgrade content for every step** (section 5.4): bucket names, key bytes, value
   bytes. In particular: `tix` values `ff ff ff ff`; credit flag bits `0x10`/`0x40`; `cmt`/`cmu`
   contents; `cf` genesis entry and `havecfilters`; `lasttxsblock`; type 1 account rows plus
   `acctvars`; VSP records extended to 40 bytes with status 2 and host ID 0; `vsphost` seed
   entries; `birthstate` bytes. *Depends:* the Go wallet reading a database upgraded by the
   replacement, and the replacement's own address-manager and transaction-store code, which
   assume the version 26 layout.
5. **Content of a newly created database** (section 4.7), including the artefacts: `mgrver`
   present, imported-account index variables equal to 0, `slip0044acct0` in the version 1 row
   layout, `birthstate` time field `ff ff ff f1 88 6e 09 00`, `vsphost` seeded with ID 0 and
   counter 0, `havecfilters = 01`. *Depends:* the Go wallet opening a database created by the
   replacement; the SLIP0044 coin-type upgrade path reads `slip0044acct0` with the version 1
   codec (`wallet/udb/addressmanager.go:796`).
6. **Tolerance of dead data.** `wstakemgr` (any content), a leftover `waddrmgr/usedaddrs`
   bucket, leftover `addrpool…` keys and `mgrver` must be ignored and preserved. *Depends:*
   existing databases created by older releases.
7. **VSP record layout and host allocation.** 40-byte `vsp` values; `vsphost` IDs allocated as
   counter + 1 with the counter stored under `vsphostindex` in the same bucket; `vsppubkey` keyed
   by the exact host bytes; a host's pubkey is written once and never replaced; ID 0 means
   unknown host. *Depends:* the Go wallet and Decrediton (through gRPC) reading tickets tracked
   by the replacement; resumption of fee payment after restart.
8. **Fee status numbering** 0/1/2/3. *Depends:* gRPC `GetVSPTicketsByFeeStatus` clients and the
   wallet's own VSP state machine.
9. **`GetVSPTicket` result for host ID 0 or an unknown host ID:** success with empty host and
   empty pubkey, not an error. *Depends:* wallet-level `VSPHostForTicket` (which converts the
   empty host into `NotExist`) and VSP client start-up for tickets tracked before version 23.
10. **Treasury policy layout.** One-byte values `0x01`/`0x02`; absence = no policy; setting
    `0x00` deletes; keys exactly as in section 4.4 (pi key bytes unmodified; per-ticket keys
    prefixed by the 32-byte ticket hash). *Depends:* the Go wallet; votes cast after a restart
    (policies are loaded from these buckets at open).
11. **Bulk policy readers exclude zero-valued entries.** *Depends:* JSON-RPC `treasurypolicy` /
    `tspendpolicy` and gRPC policy listings, which enumerate the in-memory maps built from them.
12. **Upgrade 2 semantics:** last-used index = highest used child among the contiguous run of
    generated addresses starting at child 0, `0xFFFFFFFF` when none; imported account untouched.
    *Depends:* address discovery and the next address handed out after upgrade.
13. **Upgrade 14 failure rule:** an imported script address with no cleartext copy in `wtxmgr/sc`
    aborts the whole upgrade with `IO` "missing cleartext script for p2sh hash160 …"; scripts
    present only in `wtxmgr/sc` become imported script addresses. *Depends:* multisig users; no
    silent loss of redeem scripts.
14. **Upgrade 15 invalidates filters:** `havecfilters = 00` unless the tip is genesis, forcing a
    filter re-download. *Depends:* SPV and RPC sync, which check this flag before processing
    blocks.
15. **Upgrade 21 marks every tracked ticket as errored (2); upgrade 23 assigns host ID 0.**
    *Depends:* VSP fee re-processing at next start.
16. **Error kinds for a wrong public passphrase during upgrades 2 and 14:** `Passphrase`.
    *Depends:* the daemon's passphrase hint log line and loader clients that test the kind.
17. **Operator-visible log line** `Upgrading database from version X to 26` (Info, `WLLT`).
    *Depends:* log parsers and user support; emitted only when at least one upgrade runs,
    including once during every wallet creation (`from version 1`).
18. **Unified-layout detection:** absence of the top-level `meta` bucket triggers `Migrate`;
    presence skips it regardless of anything else. *Depends:* pre-1.0 databases.
19. **`Migrate` version rules:** address-manager version must end at exactly 6; transaction
    store version 0 is an error, ≥ 3 is accepted; both legacy version keys are deleted; `ver = 1`
    is written. *Depends:* pre-1.0 databases.
20. **No private passphrase or seed prompt during upgrade.** Only the public passphrase is used.
    *Depends:* unattended starts (`--pass` not given), Decrediton start-up.
21. **Host assignment on every status write.** Each `SetVSPTicket` resolves `Host` to an ID
    (allocating one on first sight of that exact byte string) and stores it in the record, so
    two URLs that differ in any byte (trailing slash, scheme, case) are two hosts with two IDs
    and two pubkey rows. *Depends:* the Go wallet resolving hosts written by the replacement and
    the "already paid or confirmed with another vsp" check, which compares the stored host
    string with the configured URL.
22. **Zero fee hash means "no fee transaction".** `UpdateFeeErrored` stores 32 zero bytes and
    wallet-level `VSPFeeHashForTicket` turns a zero hash into `NotExist`. *Depends:* VSP client
    fee-transaction lookup after restart.
23. **Bulk treasury readers drop entries they cannot re-find** (keys of unexpected length in the
    two hash-keyed buckets) and entries whose value is `0x00`, and return any other single-byte
    value unvalidated (section 5.8). *Depends:* identical in-memory policy maps, and therefore
    identical votes and RPC listings, when both implementations open the same database.
24. **`Migrate` resets nothing but versions,** except that a legacy store at version 1 or 2 has
    its `tip` set to the genesis hash (section 5.5). *Depends:* the rescan start point of a
    migrated pre-1.0 wallet.
25. **Empty-versus-absent values.** `cf[genesis]` after upgrade 10 and `lasttxsblock` after
    upgrade 11 on a database without `tip` are present keys with zero-length values, not absent
    keys. *Depends:* only intermediate states; relevant if the fixtures are compared step by
    step against the oracle.
26. **gRPC fee-status enum values** `VSP_FEE_PROCESS_STARTED = 0`, `PAID = 1`, `ERRORED = 2`,
    `CONFIRMED = 3` map one-to-one onto the stored status; an unknown enum value is rejected with
    `InvalidArgument` before the database is read (`rpc/api.proto:1277-1280`,
    `internal/rpc/rpcserver/server.go:4052-4064`). *Depends:* Decrediton.
27. **Watching-only databases** are created and upgraded by the same chain, using only the
    public passphrase, and end with the section 4.7 watching-only content (no coin-type keys,
    `watchonly = 01`, account 0 `encPriv` holding the sealed xpub string). *Depends:* the Go
    wallet opening a watching-only database created by the replacement, and the reverse.
28. **Version stamps as forward-compatibility guards.** Versions 7, 13 and 25 change no data;
    their only effect is that software older than the feature refuses the file. A replacement
    that stamps 26 must be able to read everything those features can have written: SLIP0044
    coin-type keys (7), accounts numbered above 2³¹−1 (13) and account rows of type 2 (25).
    *Depends:* existing databases that used those features.
29. **Default public passphrase.** The daemon passes the configured public passphrase to
    `Upgrade`; its default is the string `public` (`wallet/wallet.go:66`, `config.go:351`).
    Upgrades 2 and 14 fail with kind `Passphrase` when it does not match the database's
    (item 16). *Depends:* unattended upgrade of wallets created with the default.

---

## 10. Quirks

Each quirk states whether a faithful port must reproduce it ("keep") or may consciously diverge
("free").

| # | Quirk | Citation | Port |
| --- | --- | --- | --- |
| Q1 | New wallets are created in the version 1 layout and upgraded 25 times; the comment on `createStore` claims it creates "the latest db version". | `wallet/udb/initialize.go:45-51`, `wallet/udb/txdb.go:2168-2170` | Keep the resulting bytes (section 4.7); the route is free. |
| Q2 | `waddrmgr/main/mgrver` (`06 00 00 00`) is written at creation and only ever deleted by `Migrate`, so it survives in every database that was created in the unified layout. | `wallet/udb/addressdb.go:1679`, `wallet/udb/migration.go:59` | Keep. |
| Q3 | The imported account's four index variables are 0, every other fresh account's are `0xFFFFFFFF`, because upgrade 2 skips the imported account and upgrade 5 copies its zeros. | `wallet/udb/upgrades.go:303`, `wallet/udb/upgrades.go:510-517`; fixture inspection | Keep. |
| Q4 | No upgrade step ever rewrites `slip0044acct0`: it stays in the version 1 type 0 row layout until `UpgradeToSLIP0044CoinType` (address-manager scope) decodes it with the version 1 codec, writes account 0 from it and deletes the key; in a database where that coin-type upgrade never runs it stays for the life of the database. | `wallet/udb/addressmanager.go:3073-3079`, `wallet/udb/addressmanager.go:783-823` | Keep. |
| Q5 | The comment in upgrade 3 says it creates `ticketsagendaprefs`; it creates `agendaprefs`. `ticketsagendaprefs` is created by upgrade 16. | `wallet/udb/upgrades.go:419-429` | Code wins. |
| Q6 | Wrong names in two error strings (`accountAddressCursorsUpgrade` in upgrade 5, `vspStatusUpgrade` in upgrade 23) and wrong version numbers in comments of upgrades 23 and 24. | `wallet/udb/upgrades.go:475`, `wallet/udb/upgrades.go:1555-1561`, `wallet/udb/upgrades.go:1624` | Strings: keep if error text parity is wanted (only reachable through a programming error). |
| Q7 | Upgrade 2 comment says mainnet parameters are hard-coded; the code passes the active `params`. Only the hash160 is used, so the result is the same. | `wallet/udb/upgrades.go:341-346` | Free. |
| Q8 | Upgrade 12 never checks the block iterator's error: a malformed block record ends the mined pass silently and the upgrade still succeeds. | `wallet/udb/upgrades.go:1056-1071`, `wallet/udb/txdb.go:386-391` | Free (recommend failing with `IO`). |
| Q9 | Upgrade 12 loads each mined transaction by "newest record with this hash", not by the block being iterated. | `wallet/udb/upgrades.go:1060`, `wallet/udb/txdb.go:675-685` | Keep (same result unless a hash is recorded in two blocks). |
| Q10 | Upgrade 14 tolerates a `NotExist` error from deleting `cscript`; bbolt never returns one for a missing key. Dead code. | `wallet/udb/upgrades.go:1262-1265`, `bbolt@v1.3.12/bucket.go:318-338` | Free. |
| Q11 | Upgrade 14 is not byte-deterministic for scripts found only in `wtxmgr/sc`: random secretbox nonce and `time.Now()` add-time. | `wallet/udb/upgrades.go:1238-1252`, `wallet/udb/addressdb.go:1383-1394` | Keep semantics; exact bytes cannot be matched. |
| Q12 | Upgrade 14 computes map keys from the script values, ignoring the bucket keys of `wtxmgr/sc`, and then deletes by computed hash. | `wallet/udb/upgrades.go:1166-1171` | Keep. |
| Q13 | The "block record len %d" error prints the key length (always 4), not the value length. | `wallet/udb/txdb.go:312-314` | Free. |
| Q14 | `Upgrade` treats any version ≥ 26 as "nothing to do" and a stored version 0 as a call through a nil function (panic). | `wallet/udb/upgrades.go:219`, `wallet/udb/upgrades.go:1710-1720` | Version > 26: keep (rejected by `udb.Open`). Version 0: return an `IO` error. |
| Q15 | Version 8 reused stake-tag bit `0x10` for "has expiry"; upgrade 9 clears it for every credit of a transaction with an expiry and restores it only for stake-change outputs of real tickets. A credit that carried the `OP_SSTXCHANGE` tag on a non-ticket transaction with an expiry loses the tag (fixture `v7.db.gz`: flags `0x12` → `0x42`). | `wallet/udb/upgrades.go:756-769` | Keep. |
| Q16 | Upgrade 10 stores an empty value as the genesis "filter" (the version 1 filter can no longer be built); upgrade 15 discards it. | `wallet/udb/upgrades.go:863-875` | Keep (not observable after upgrade 15). |
| Q17 | Upgrades 8 and 9 and upgrade 6 re-insert records in Go map order (random). No logical effect; bbolt page layout may differ run to run. | `wallet/udb/upgrades.go:588`, `wallet/udb/upgrades.go:672` | Free. |
| Q18 | Upgrade 17 writes to the bucket it is iterating with `ForEach`, which bbolt documents as undefined; in practice the iteration reads the unmodified page. | `wallet/udb/upgrades.go:1388-1410`, `bbolt@v1.3.12/bucket.go:391-402` | Collect keys first. |
| Q19 | Loops conditioned on `v != nil` end at the first nested bucket entry as well as at the end of the bucket. | e.g. `wallet/udb/upgrades.go:550`, `wallet/udb/upgrades.go:1511` | Free (no nested buckets exist there). |
| Q20 | Upgrade 23 builds an unused 4-byte buffer. Dead code. | `wallet/udb/upgrades.go:1580-1582` | Free. |
| Q21 | Upgrade 26 writes the zero `time.Time` as Unix seconds −62135596800 (`ff ff ff f1 88 6e 09 00`). | `wallet/udb/upgrades.go:1680-1683`, `wallet/udb/txmined.go:361` | Keep. |
| Q22 | `GetVSPHost` formats `&id` and `GetVSPPubKey` formats `&host` with `%v`: the first message ends in a memory address (`0xc000…`), the second in `&[104 111 …]` (decimal bytes). | `wallet/udb/vsp.go:203`, `wallet/udb/vsp.go:254` | Free; the address cannot be reproduced. Both errors are swallowed by the only in-tree callers. |
| Q23 | `SetVSPTicket` overwrites the caller's `record.VSPHostID`. The upstream test's `DeepEqual` passes only because of this. | `wallet/udb/vsp.go:82`, `wallet/udb/vsp_test.go:177-226` | Keep the stored value; the mutation is free. |
| Q24 | Once a host has a pubkey row, later `SetVSPTicket` calls with a different pubkey for the same host do not update it. | `wallet/udb/vsp.go:49-80` | Keep. |
| Q25 | `SetVSPTicket` with an empty host allocates an ID, writes the host row and counter, then fails on the pubkey put ("key required"). | `wallet/udb/vsp.go:49-77`, `bbolt@v1.3.12/bucket.go:286-287` | Keep the failure. |
| Q26 | `GetVSPTicket` returns `PubKey` aliasing database memory that is invalid after the transaction. | `wallet/udb/vsp.go:121`, `wallet/udb/vsp.go:269` | Copy. |
| Q27 | VSP decoders have no length checks (panic on short values); `GetVSPTicketsByFeeStatus` ignores the error of `Hash.SetBytes` (a key that is not 32 bytes yields the zero hash). | `wallet/udb/vsp.go:136`, `wallet/udb/vsp.go:149-161`, `wallet/udb/vsp.go:212-220`, `wallet/udb/vsp.go:262-272` | Return `IO`. |
| Q28 | The result order of `GetVSPTicketsByFeeStatus` is Go map order; the gRPC response list order is therefore random per call. | `wallet/udb/vsp.go:131`, `wallet/wallet.go:5625-5630` | Free (clients cannot rely on order); bucket order is a reasonable choice. |
| Q29 | The host-counter key constant is declared among the transaction-store root keys although it lives in `vsphost`. | `wallet/udb/txdb.go:162` | Free. |
| Q30 | `DeleteVSPTicket` is called for every pruned unmined transaction hash, ticket or not; errors are only logged (`Failed to remove pruned ticket from db: %v`). | `wallet/chainntfns.go:261-275` | Keep. |
| Q31 | Treasury comments say "abstaining policies are excluded"; abstain and invalid are the same value (`0x00`), which is never stored. | `wallet/udb/treasury.go:105-106` | — |
| Q32 | Treasury bulk readers look every key up a second time and return a partially filled map together with an error. | `wallet/udb/treasury.go:107-124` | Free. |
| Q33 | Treasury setters wrap delete errors as `IO` but return put errors unwrapped; with an empty pi key the put fails ("key required", `Invalid`) while a delete succeeds. | `wallet/udb/treasury.go:151-163` | Keep kinds if error parity is wanted. |
| Q34 | Treasury setters store any byte; validation is only in the wallet layer. | `wallet/udb/treasury.go:33-45`, `wallet/wallet.go:669-674` | Keep. |
| Q35 | `Migrate` accepts a legacy transaction-store version above 3 without complaint and jumps a legacy address manager at version 1–4 straight to 5. | `wallet/udb/txdb.go:2318-2319`, `wallet/udb/addressdb.go:1754-1762` | Keep or reject (decision D3). |
| Q36 | `Migrate` on a bbolt file without `waddrmgr` panics (nil bucket). `NeedsMigration` is true for any file lacking `meta`, including an empty one. | `wallet/udb/migration.go:39-43` | Return an error. |
| Q37 | `Migrate` no longer deletes `wstakemgr/main/stakestorever`; the bucket and key are orphaned. | history `a83b85bb` | Keep (do not touch). |
| Q38 | Upgrade 2 skips an invalid child index without consulting the address bucket and continues scanning. | `wallet/udb/upgrades.go:334-337` | Keep. |
| Q39 | `readRawTxRecordHash` prints a Go stack trace to stderr before returning its error. | `wallet/udb/txdb.go:578-582` | Free. |
| Q40 | The daemon opens the wallet in a goroutine; on interrupt the main flow returns while the upgrade transaction is still running. | `dcrwallet.go:208-230` | Free; atomicity must hold. |
| Q41 | `v7.go` (not `v7.db.go`) calls `errors.Errorf` without importing any `errors` package, so the generator does not compile as committed. | `wallet/udb/testdata/v7.go:11-31`, `wallet/udb/testdata/v7.go:133` | — |
| Q42 | `v2.db.gz` is stamped version 2 but still contains `waddrmgr/usedaddrs` and the `addrpool…` keys (it was created directly at version 2 by code that still wrote them). | fixture inspection | Tolerate (item 6 of section 9). |
| Q43 | Fixtures `v1` … `v7` were generated for testnet2 but are upgraded in the test with testnet3 parameters, so for `v1`–`v4` the stored `tip` (testnet2 genesis, bytes `9cc27bbd…2a606142`) differs from the testnet3 genesis hash written to `cf` and `birthstate`, and `havecfilters` ends as `00` for all seven tested fixtures (for `v5`, `v7` and `v11` because their tip is not a genesis block at all). | `wallet/udb/testdata/v1.db.go:32`, `wallet/udb/upgrades_test.go:75`; fixture inspection | Reproduce when using the fixtures as golden vectors. |
| Q44 | `v5.db.gz` contains no stakebase key in `wtxmgr/mi` (its only all-zero-hash key has index 0 and belongs to the dummy input of ticket 0), so the test's "stakebase input removed" assertion is vacuous. | fixture inspection; `wallet/udb/upgrades_test.go:287-292` | Add a real vector. |
| Q45 | `v24.db.gz` is not referenced by any test. | `wallet/udb/upgrades_test.go:24-38` | — |
| Q46 | Nil versus empty values. A zero-length value read from a committed bbolt page is a non-nil empty slice, but a value put as `nil` and read back **in the same transaction** (still in a dirty node) is nil. All `v != nil` loop conditions and `Get(...) == nil` tests in the upgrades therefore depend on whether an empty value was written earlier in the same upgrade transaction. In the pinned chain the only nil puts are `cf[genesis]` (upgrade 10, bucket deleted by upgrade 15 without being read) and `lasttxsblock` when `tip` is absent (upgrade 11, never read by a later upgrade), so nothing observable depends on it. | `bbolt@v1.3.12/cursor.go:367-384`, `bbolt@v1.3.12/page.go:138-142`, `bbolt@v1.3.12/bucket.go:297-310`, `wallet/udb/upgrades.go:872`, `wallet/udb/upgrades.go:909` | Free; treat "present with empty value" as present. |
| Q47 | `Cursor.Close()` of the bbolt driver is a no-op, so the cursors the upgrades leave open on error paths (upgrade 12's block iterator; upgrade 14's second cursor when step 4 returns early) leak nothing. | `wallet/internal/bdb/db.go:262`, `wallet/udb/txdb.go:384-388`, `wallet/udb/upgrades.go:1189-1204` | Free. |
| Q48 | `fetchAccountInfo` (upgrades 2 and 5) decodes an account row of type 2 (imported voting) with the legacy BIP0044 codec and rejects type 1; `fetchDBAccount` (upgrade 17) does the opposite grouping. Neither type can exist at the versions those upgrades run on. | `wallet/udb/addressdb.go:794-800`, `wallet/udb/addressdb.go:810-817` | Free. |
| Q49 | Legacy account rows and VSP host/pubkey values tolerate trailing bytes (ignored on read; dropped when an upgrade rewrites the row), and a type 1/2 account row does not ("extra bytes in BIP0044 account row"). | `wallet/udb/addressdb.go:115-137`, `wallet/udb/addressdb.go:603-608`, `wallet/udb/vsp.go:212-220` | Keep tolerance on read. |
| Q50 | In `fetchDBAccount`, a failed account-variable read returns `errors.E(errors.IO, err)` with the **outer** `err`, which is nil at that point, instead of the reader's stored error: the returned error carries kind `IO` and no message. Reachable from upgrade 17 only for a type 1/2 row whose variables bucket exists but is incomplete. | `wallet/udb/addressdb.go:821-833` | Free. |
| Q51 | The doc comments of `Initialize` and `InitializeWatchOnly` say the database "is initialized with the latest version and does not require any upgrades"; both write version 1 and call `Upgrade`. | `wallet/udb/initialize.go:15-18`, `wallet/udb/initialize.go:45-51` | Code wins (see Q1). |
| Q52 | The comment in `Migrate` says "The old stake manager had no upgrades, so nothing to do there"; the legacy stake version key is left in place (see Q37). | `wallet/udb/migration.go:54` | — |
| Q53 | Comment/code mismatches inside `vsp.go`: `GetVSPHost` and `GetVSPPubKey` are documented as "gets a specific ticket"; `deserializeVSPPubKey` / `serializeVSPPubKey` as handling "vsp host" records; `serializeVSPTicket` as serializing "a single stake pool user ticket"; `VSPFeeProcessPaid` as "paid, but not published"; `VSPFeeProcessErrored` has no comment. | `wallet/udb/vsp.go:26-36`, `wallet/udb/vsp.go:163-164`, `wallet/udb/vsp.go:196`, `wallet/udb/vsp.go:249`, `wallet/udb/vsp.go:261`, `wallet/udb/vsp.go:274` | Code wins. |
| Q54 | `GetVSPTicketsByFeeStatus` takes a plain `int`, the stored field is `uint32`, the exported type is `FeeStatus int`, and wallet code converts between all three with unchecked casts. | `wallet/udb/vsp.go:22`, `wallet/udb/vsp.go:129-134`, `wallet/vsp.go:226-229`, `wallet/vspticket.go:251` | Free; store and compare as `u32`. |
| Q55 | The host counter is read with `byteOrder.Uint32` on whatever `vsphost["vsphostindex"]` holds: a value shorter than 4 bytes (or an absent key, or an absent `vsphost` bucket) panics inside `SetVSPTicket`; bytes beyond the fourth are ignored. | `wallet/udb/vsp.go:52-55` | Return `IO`. |
| Q56 | `SetVSPTicket` is not idempotent on failure inside its own body: the host row and counter are written before the pubkey row, and the ticket record last; atomicity comes only from the caller's transaction. | `wallet/udb/vsp.go:49-87` | Keep the write order inside one transaction. |
| Q57 | Upgrade 23's comment promises that tickets given host ID 0 "will be noticed on the next time the process managed tickets is completed and all entries will be updated"; the storage layer does nothing of the kind — the update happens only as a side effect of the next `UpdateFee*` write for that ticket. | `wallet/udb/upgrades.go:1595-1598`, `wallet/vspticket.go:246-290` | — |
| Q58 | The upgrade-table comment says "there was never a version zero so `upgrades[0]` is nil"; `Upgrade` does not guard against a stored 0 (Q14). The array length is fixed by the highest index (25), so the type is `[25+1]`. | `wallet/udb/upgrades.go:216-245` | — |
| Q59 | The only unresolved marker in scope is `// TODO: V10 upgrade test` in the test table; upgrades 7 and 9 are explicitly declared untested by comment. | `wallet/udb/upgrades_test.go:33-36` | — |
| Q60 | The doc comment of `votingPreferencesVersion` says version 3 "removes all per-ticket vote bits"; the pinned upgrade removes nothing (the legacy stake store keeps its vote bits, section 4.5). The comment inside the upgrade, repeated verbatim in `verifyV3Upgrade`, names the wrong bucket (Q5). | `wallet/udb/upgrades.go:42-45`, `wallet/udb/upgrades.go:419-423`, `wallet/udb/upgrades_test.go:162-166` | Code wins. |
| Q61 | Upgrade 8 opens read-write cursors on `c` and `mc` but only reads through them (writes happen afterwards through the bucket); upgrade 9 opens read cursors for the same traversal. | `wallet/udb/upgrades.go:643`, `wallet/udb/upgrades.go:681`, `wallet/udb/upgrades.go:737`, `wallet/udb/upgrades.go:785` | Free. |
| Q62 | `SetVSPTicket` decides whether a host is known by looking only in `vsppubkey`; the reserved `vsphost` ID 0 (zero-length host) has no `vsppubkey` row and can never get one (empty key, Q25). | `wallet/udb/vsp.go:49-80`, `wallet/udb/upgrades.go:1575-1593` | Keep. |
| Q63 | `verifyV5Upgrade` calls `fetchDBAccount(ns, acct, 5)` on a version 26 database. It works only because rows of type 1 ignore the version argument; the verifier then requires the concrete type `*dbBIP0044Account` (the post-17 form), so it would fail on a database that really was at version 5. | `wallet/udb/upgrades_test.go:215-225`, `wallet/udb/addressdb.go:803-851` | Port the assertion, not the constant. |
| Q64 | `verifyV2Upgrade` carries an unused `totalAddrs` column and reports the internal-branch address-pool key with the message "external address pool bucket still exists"; `verifyV4Upgrade`, `verifyV5Upgrade`, `verifyV6Upgrade` and `verifyV8Upgrade` never call `udb.Open`, so they do not check that the upgraded database is loadable. | `wallet/udb/upgrades_test.go:96-127`, `wallet/udb/upgrades_test.go:180-366` | — |
| Q65 | The package test set-up calls `Initialize` and then `Upgrade` again on the freshly created database; the second call is the "already at 26" no-op (returns nil without logging). This is the only upstream exercise of that path. | `wallet/udb/common_test.go:46-65`, `wallet/udb/addressmanager_test.go:1264` | Keep the no-op. |
| Q66 | In a watching-only database the "private" master key parameters (`mpriv`) derive from the public passphrase and account 0's `encPriv` is the xpub string sealed under the private crypto key. No upgrade reads either. | `wallet/udb/addressmanager.go:3129-3173` | Keep (address-manager scope). |
| Q67 | Typographical slips in the version comments ("eight version", "the twelfth version" without a verb, the comment for `importVotingAccountVersion` headed `importVotingAccount`, "all entried" in upgrade 23). | `wallet/udb/upgrades.go:74`, `wallet/udb/upgrades.go:105`, `wallet/udb/upgrades.go:196`, `wallet/udb/upgrades.go:1597` | — |

---

## 11. Upstream tests

### 11.1 `wallet/udb/upgrades_test.go`

`TestUpgrades` (`wallet/udb/upgrades_test.go:42-84`): for each fixture, gunzip to a temporary
file, open with the bbolt driver, call `Upgrade(ctx, db, []byte("public"),
chaincfg.TestNet3Params())` — which runs **every** remaining upgrade to version 26 — and then run
one verifier. Subtests run in parallel.

| Fixture | Verifier | What it pins |
| --- | --- | --- |
| `v1.db.gz` | `verifyV2Upgrade` (`:86-153`) | `udb.Open` succeeds after the full chain; 9 BIP0044 accounts; for each, both address-pool keys are gone and `LastUsedExternalIndex` / `LastUsedInternalIndex` equal the expected value (`0xFFFFFFFF`, 18, 19, 19, 25, 29, 29, 185, 199); `usedaddrs` bucket gone. Read through `AccountProperties`, so it also exercises upgrades 5 and 17. |
| `v2.db.gz` | `verifyV3Upgrade` (`:155-178`) | `udb.Open` succeeds; top-level `agendaprefs` exists. |
| `v3.db.gz` | `verifyV4Upgrade` (`:180-192`) | `waddrmgr/main/seed` is gone. |
| `v4.db.gz` | `verifyV5Upgrade` (`:194-249`) | For accounts 0..8 and the imported account: last-used and last-returned indexes equal the expected pairs — `(0xFFFFFFFF, 0xFFFFFFFF)`, `(0,0)`, `(9,9)`, `(5,15)`, `(19,20)`, `(20,19)`, `(29,30)`, `(30,29)`, `(2³¹−1, 2³¹−1)`, imported `(0,0)`. Reads the type 1 row and `acctvars`, so it also pins upgrade 17. |
| `v5.db.gz` | `verifyV6Upgrade` (`:251-299`) | `wtxmgr/tix` holds exactly six listed ticket hashes, each with picked height −1; no stakebase key in `wtxmgr/mi` (vacuous, Q44). |
| `v7.db.gz` | `verifyV8Upgrade` (`:301-366`) | Every mined and unmined credit has the has-expiry bit (read with the version 26 rule, bit `0x40`); the mined transaction bucket holds 3 transactions with and 3 without expiry. Covers upgrades 8 and 9 together. |
| `v11.db.gz` | `verifyV12Upgrade` (`:373-482`) | `cmt` and `cmu` exist; `AccountBalances` over 12 accounts matches the expected voting-authority, locked-by-tickets, immature-stake-generation and total amounts for the six ticket/vote/revocation × mined/unmined combinations. |

Harness details: the table `dbUpgradeTests` (`wallet/udb/upgrades_test.go:24-38`) lists the
seven `(verifier, filename)` pairs in the order above; subtests are named `TestUpgrades/group/test0` …
`TestUpgrades/group/test6` and call `t.Parallel()`; each decompressed copy is written to `<tempdir>/testN.db`
and opened with `walletdb.Open("bdb", path)`; a failing `Upgrade` is fatal with
`Upgrade failed: <err>`. Comments in the table state that version 7 has no test because it is
backwards compatible, that version 9 is covered by the version 8 test, and
`// TODO: V10 upgrade test`. The package-level `pubPass = []byte("public")` is shared with other
tests of the package.

Exact vectors the verifiers assert (port these literally):

- `verifyV2Upgrade` reads `lastaccount` and requires it to be 8 (nine accounts, 0..8), checks
  that `waddrmgr/meta` has neither address-pool key for each account, and that
  `waddrmgr/usedaddrs` is nil. Expected last-used index per account (identical for the external
  and internal branch): `0xFFFFFFFF, 18, 19, 19, 25, 29, 29, 185, 199`.
- `verifyV6Upgrade` — the six ticket hashes (display form) that must be the only keys of
  `wtxmgr/tix`, each with value `ff ff ff ff`:
  `7bc19eb0bf3a57be73d6879b6c411404b14b0156353dd47c5e0456768704bfd1`,
  `a6abeb0127c347b5f38ebc2401134b324612d5b1ad9a9b8bdf6a91521842b7b1`,
  `1107757fa4f238803192c617c7b60bf35bdc57bc0fc94b408c71239ff9eaeb98`,
  `3fd00cda28c4d148e0cd38e1d646ba1365116b3ddd9a49aca4483bef80513ff9`,
  `f4bdebefaa174470182960046fa53f554108b8ea09a86de5306a14c3a0124566`,
  `bca8c2649860585f10b27d774b354ea7b80007e9ad79c090ea05596d63995cf5`
  (`wallet/udb/upgrades_test.go:255-262`).
- `verifyV12Upgrade` — `AccountBalances(tx, 0)` (zero minimum confirmations), amounts in atoms;
  "absent" means the account must not appear in the result map; every unlisted field must be 0
  (`wallet/udb/upgrades_test.go:396-429`):

  | Account | Scenario | Voting authority | Locked by tickets | Immature stake generation | Total | Spendable | Unconfirmed |
  | --- | --- | --- | --- | --- | --- | --- | --- |
  | 1 | unmined ticket (voter) | 1100 | 0 | 0 | 0 | 0 | 0 |
  | 2 | unmined ticket (commitment) | 0 | 1000 | 0 | 1000 | 0 | 0 |
  | 3 | mined ticket (voter) | 1100 | 0 | 0 | 0 | 0 | 0 |
  | 4 | mined ticket (commitment) | 0 | 1000 | 0 | 1000 | 0 | 0 |
  | 5 | mined ticket + unmined vote (voter) | absent | | | | | |
  | 6 | mined ticket + unmined vote (commitment) | 0 | 0 | 1300 | 1300 | 0 | 0 |
  | 7 | mined ticket + mined vote (voter) | absent | | | | | |
  | 8 | mined ticket + mined vote (commitment) | 0 | 0 | 1300 | 1300 | 0 | 0 |
  | 9 | mined ticket + unmined revocation (voter) | absent | | | | | |
  | 10 | mined ticket + unmined revocation (commitment) | 0 | 0 | 700 | 700 | 0 | 0 |
  | 11 | mined ticket + mined revocation (voter) | absent | | | | | |
  | 12 | mined ticket + mined revocation (commitment) | 0 | 0 | 700 | 700 | 0 | 0 |

  The balance computation itself is specified with the transaction store; this table is the
  end-to-end check that upgrade 12 produced the `cmt`/`cmu` content that computation needs.

Further harness facts:

- Helpers the verifiers borrow from other test files of the package: `decodeHash`
  (`wallet/udb/txcommon_test.go:97-103`; parses display-order hex, panics on error) and the
  blank import of the `bdb` driver. `verifyV6Upgrade` decodes each `tix` value with
  `extractRawTicketPickedHeight`; `verifyV8Upgrade` tests each credit with
  `fetchRawCreditHasExpiry(v, DBVersion)` and counts only bucket `t` (3 with, 3 without expiry);
  `verifyV12Upgrade` runs one subtest per account named `acct=<n>`.
- Which verifiers call `udb.Open` after the upgrade (and so pin "the result is loadable"):
  `verifyV2Upgrade`, `verifyV3Upgrade`, `verifyV12Upgrade`. The others read buckets directly.
- `verifyV5Upgrade` loads accounts with `fetchDBAccount(ns, acct, 5)` and requires the type 1
  form (Q63).
- `wallet/udb/common_test.go:46-65` (`createEmptyDB`, run from the package `TestMain`,
  `wallet/udb/addressmanager_test.go:1264`) calls `Initialize` and then `Upgrade` a second time
  on testnet3: every other test of package `udb` therefore runs on a database produced by the
  creation path of section 5.6, and the second `Upgrade` pins the no-op return at version 26.
- Failure texts are test output only (`Upgrade failed: %v`, `Open after Upgrade failed: %v`,
  `Used address bucket still exists`, `Seed was not deleted`, `Agenda preferences bucket was not
  created`, …) and need not be reproduced.

Branches of tested upgrades that no fixture reaches (from the generator sources, section 11.3):

- Upgrades 8 and 9, unmined credits: `v7.db.gz` records its three unmined expiry transactions
  without credits and the unmined credits of `v5.db.gz` belong to transactions without expiry,
  so the `mc` rewrite loop never changes a value and the `mc` half of `verifyV8Upgrade` is
  vacuous.
- Upgrade 9, "restore `0x10` for a real ticket's stake-change output": the expiry transactions
  of `v7.db.gz` have no inputs and are not tickets.
- Upgrade 12, P2SH commitments and a vote or revocation stored in a block before its ticket.
  Commitments to an imported address are reached (the tickets of `v5.db.gz` commit to the
  imported key and pass through upgrade 12) but nothing asserts the resulting `cmt`/`cmu`
  entries.
- Upgrade 2, an invalid child index, a gap in the generated addresses, and any account whose
  external and internal usage differ (the generator marks the same child indexes on both
  branches).

Not covered by any assertion (each still *executes* on every fixture, on empty or trivial data):
upgrade 7, 10, 11, 13, 14 (never with a script present), 15 (genesis filter value not checked),
16, 18–26; `Migrate` and `NeedsMigration` (no pre-unified fixture); VSP record rewrites of
upgrades 21 and 23 (no fixture contains a `vsp` entry); the error exits of every upgrade; the
"upgrade from N to 26 equals upgrade by release software" property.

### 11.2 `wallet/udb/vsp_test.go` (adjacent, covers `vsp.go`)

| Test | Pins |
| --- | --- |
| `TestVSPSerializeVSPHost` | host codec round trip, including the empty host |
| `TestVSPSerializeVSPPubKey` | pubkey codec round trip (ID 69 / 0, empty pubkey) |
| `TestVSPSerializeVSPTicket` | ticket codec round trip; `Host` and `PubKey` are not serialized |
| `TestSetGetDeleteVSPTicket` | `NotExist` before insert; set then get returns the same struct (with the host ID rewritten, Q23); delete; `NotExist` again; deleting a missing ticket is not an error |

`treasury.go` has no unit test in `wallet/udb`.

### 11.3 Test data

All generators are Go `main` programs meant to be built at the commit that introduced them
("This file should not be updated for API changes"). All use public passphrase `public` and
private passphrase `private`. Statements about content are from the generator source and
confirmed by fixture inspection.

| Fixture | `ver` | Network | Seed | Produced by |
| --- | --- | --- | --- | --- |
| `v1.db.gz` | 1 | testnet2 | random | `Initialize`; accounts 1..8 named `account-N`; address indexes `[0, n)` generated on both branches with n = 0, 20, 20, 20, 30, 30, 30, 200, 200 for accounts 0..8; children marked used: acct 1 {0,10,18}, 2 {0,10,18,19}, 3 {19}, 4 {0,10,20,25}, 5 {9,18,29}, 6 {29}, 7 {0,20,…,180,185}, 8 {199}. 1100 address rows, 56 `usedaddrs` entries, 19 `waddrmgr/meta` keys; `main` has `seed` and `cscript`. |
| `v2.db.gz` | 2 | testnet2 | random | `Initialize`, then one ticket (testnet2 `4516ef1d…837033`) inserted into the legacy stake store with vote bits `0x0001` and extended bits `00 00 00 04`: one 635-byte `wstakemgr/sstxrecords` value in the version < 3 layout. Still has `usedaddrs` (Q42). |
| `v3.db.gz` | 3 | testnet2 | random | `Initialize` only. `main` still has `seed`. |
| `v4.db.gz` | 4 | testnet2 | random | `Initialize`; accounts 1..8 created and `MarkUsedChildIndex` called with the (ext, int) pairs listed for `verifyV5Upgrade`. |
| `v5.db.gz` | 5 | testnet2 | random | `Initialize`; imported private key `00…01`; 7 fake blocks (hash = height in the last byte); 6 tickets (unmined; mined at heights 2–6) with votes/revocations per the comment in the generator; all transactions carry the zero-time received stamp. |
| `v7.db.gz` | 7 | testnet2 | random | `Initialize`; imported key; one fake block; 3 mined transactions with `Expiry = 1000` paying `OP_SSTXCHANGE` scripts, credited as change to account 0; 3 unmined transactions with expiry (no credits recorded); 3 mined transactions without expiry (no credits). |
| `v11.db.gz` | 11 | testnet3 | 32 zero bytes | `Initialize`; accounts 1..12; one block; six ticket scenarios (unmined ticket; mined ticket; mined ticket + unmined vote; + mined vote; + unmined revocation; + mined revocation), each ticket voting to an odd account, committing 1000 atoms to the next account and 100 atoms to a non-wallet pool address; `cf` holds two version 1 filters. |
| `v24.db.gz` | 24 | testnet3 | 32 zero bytes | `Initialize` only. Unused by tests; its stated purpose is the 24→25 upgrade. A complete example of a freshly created database (section 4.7). |

Every fixture contains `wstakemgr` (they predate its removal).

Generator shape (all eight): `main` calls `setup()` then `compress()`. `setup` creates
`vN.db` in the working directory with `walletdb.Create("bdb", name)` and populates it through
the `udb` API of its era; `compress` copies the file through `gzip.NewWriter` (default
compression level) into `vN.db.gz` and removes the uncompressed file. No generator carries a
build constraint, and none is compiled by the package's tests (they are `package main` files in
`testdata/`, which the Go tool ignores). Their import paths date them: `v1`–`v5` and `v7` import
`github.com/decred/dcrwallet/...` with the pre-module `dcrutil`/`hdkeychain` packages; `v11`
and `v24` import `decred.org/dcrwallet/v3` (so `v11.db.go`, dated 2018, was edited after it was
introduced despite its own "should not be updated for API changes" notice, and building it at
the commit that introduced it is not the same as building the committed file). The gzip container bytes are not
reproducible input (header timestamps, compressor version); only the decompressed database
matters.

Transaction shapes the generators build (needed to regenerate equivalent fixtures or to explain
the expected numbers):

- **`v1.db.go`** (`wallet/udb/testdata/v1.db.go:47-139`): for each account the generator calls
  `SyncAccountToAddrIndex(account, numAddrs, branch)` — the API of that era generated indexes
  `[0, numAddrs)` — then marks the listed child indexes used with `MarkUsed` on **both**
  branches, deriving each address from the account branch xpub. `v1` and `v2` call a
  six-argument `udb.Initialize` whose last argument is `false`; its meaning is not visible at
  the pin (UNVERIFIED; settle by reading `udb.Initialize` at the commit that introduced
  `v1.db.go`).
- **`v2.db.go`** (`wallet/udb/testdata/v2.db.go:37-94`): the ticket is a real testnet2
  transaction embedded as hex and inserted with the legacy stake manager's `InsertSStx`.
- **`v4.db.go`** (`wallet/udb/testdata/v4.db.go:47-105`): accounts 1..8 named `account-N`;
  `MarkUsedChildIndex(acct, 0, ext)` and `(acct, 1, int)` with the pairs `(0,0)`, `(9,9)`,
  `(5,15)`, `(19,20)`, `(20,19)`, `(29,30)`, `(30,29)`, `(2³¹−1, 2³¹−1)`. Account 0 and the
  imported account are untouched.
- **`v5.db.go`** (`wallet/udb/testdata/v5.db.go:113-241`, `:263-299`): blocks at heights 1..7
  with header fields version 1, stake version 1, vote bits 1 and block hash = 31 zero bytes ‖
  height. Ticket `i` (0..5): one input spending outpoint (zero hash, index `i`) with value-in
  50e8 atoms; outputs `[0]` `OP_SSTX` P2PKH to the imported key's address, value 50e8 − 3e5;
  `[1]` commitment push to the same address with fee limits `0x5800`, value 0; `[2]`
  `OP_SSTXCHANGE` to the same address, value 0. Ticket 0 is unmined; ticket `i ≥ 1` is mined at
  height `i + 1`. Spenders: ticket 2 unmined vote, ticket 3 vote mined at height 5, ticket 4
  unmined revocation, ticket 5 revocation mined at height 7 (each vote references the block
  one above its ticket's, with vote bits 1; revocations pay a fee at 1e5 atoms/kB). Output 0 of every ticket and
  spender is credited to the imported account, not as change. The imported private key is 31
  zero bytes ‖ `01`.
- **`v7.go`** (`wallet/udb/testdata/v7.go:96-192`): one block at height 1 with hash 31 zero
  bytes ‖ `01`. Three mined transactions with `Expiry = 1000`, no inputs and one
  `OP_SSTXCHANGE` output to the imported key's address worth 1, 2 and 3 atoms, each credited
  (output 0, change = true, account 0). Three unmined transactions with `Expiry = 1000` and one
  `OP_SSTXCHANGE` output to the non-wallet address `TsWjioPrP8E1TuTMmTrVMM2BA4iPrjQXBpR`, no
  credit. Three mined transactions without expiry paying 1, 2 and 3 atoms P2PKH to the imported
  address, no credit.
- **`v11.db.go`** (`wallet/udb/testdata/v11.db.go:162-445`): commitment 1000 atoms, commitment
  reward 300, pool fee 100, pool fee reward 30, ticket price 1100; pool fee address
  `TsR28UZRprhgQQhzWns2M6cAwchrNVvbYq2` (not in the wallet); fee limits `0x0058`. Each ticket
  creates two accounts (named by their number) with one external address each: the first is
  the voting address, the second the commitment address. Ticket: two dummy inputs (zero hash,
  running index); outputs `[0]` `OP_SSTX` to the voting address, 1100; `[1]` pool fee
  commitment; `[2]` `OP_SSTXCHANGE` to a zero hash160; `[3]` wallet commitment of 1000; `[4]`
  `OP_SSTXCHANGE` to a zero hash160. Vote: inputs stakebase (zero hash, index `0xFFFFFFFF`) and
  ticket output 0; outputs `[0]` `6a 24` ‖ 36 zero bytes, `[1]` `6a 03 00 00 00`, `[2]`
  `OP_SSGEN` to the pool address, 130, `[3]` `OP_SSGEN` to the commitment address, 1300.
  Revocation: input ticket output 0; outputs `[0]` `OP_SSRTX` pool, 70, `[1]` `OP_SSRTX`
  commitment, 700. A credit is added for every non-zero output whose address the wallet owns.
  All mined transactions sit in one block at height 1 (the generator notes this breaks
  consensus rules and is acceptable for a database test); that block's stored version 1 filter
  is built from a zero key and one 16-zero-byte entry. The wallet commitment is ticket output
  3, reached from vote output 3 (`(3 − 2)·2 + 1`) and revocation output 1 (`1·2 + 1`), which is
  the mapping upgrade 12 implements; the expected balances 1300 and 700 are 1000 ± 300 and the
  voting authority 1100 is the ticket price.
- **`v24.db.go`** (`wallet/udb/testdata/v24.db.go:63-74`): `udb.Initialize` with a 32-zero-byte
  seed on testnet3 and nothing else.

No file under `wallet/udb/testdata/` and no test in the package exercises `Migrate`,
`NeedsMigration`, any function of `treasury.go`, `SetVSPHost` / `GetVSPHost` /
`SetVSPPubKey` / `GetVSPPubKey` directly, or host-ID allocation across more than one host.

### 11.4 Porting the vectors

1. **Port as is:** the eight `.db.gz` files and the seven verifiers. They need a bbolt-compatible
   reader/writer, the address-manager account/address codecs and the transaction-store balance
   code (for `verifyV12Upgrade`).
2. **Go oracle shim — full-tree equality.** Build a small Go program at the pin that (a) opens a
   database file, (b) runs `udb.Upgrade` with given passphrase and network, and (c) dumps every
   bucket recursively as sorted `path → hex(key) → hex(value)` lines. Run it on each fixture and
   compare with the same dump of the Rust-upgraded copy. This pins every byte for all 25 steps on
   eight starting states. Expected non-determinism: none for these fixtures (no fixture has a
   script in `wtxmgr/sc`).
3. **Regenerate through the oracle** (new fixtures the upstream suite lacks):
   - a version 13 database with (i) an imported P2SH script present in both stores, (ii) a script
     only in `wtxmgr/sc`, and (iii) an imported script whose `sc` entry was removed (expected
     failure);
   - version 20 and 22 databases with several `vsp` entries;
   - a version 5 database that really contains the stakebase key in `wtxmgr/mi`;
   - version 9 and 14 databases with a tip above genesis (checks `havecfilters = 00`);
   - a pre-unified database (requires building dcrwallet 0.8.2) for `Migrate`;
   - a freshly created database on each network, for the section 4.7 layout and the genesis
     `cf` value of mainnet, simnet and regnet;
   - a freshly created **watching-only** database (`InitializeWatchOnly`), for the
     watching-only variant of section 4.7;
   - a version 8 database with unmined credits of expiry transactions and with a real ticket's
     stake-change credit (the two untested branches of upgrade 9).
   Old-version databases can be produced by building the generator at the commit that introduced
   the version (the method the upstream fixtures used), or by writing the legacy records directly
   with the layouts in section 4.6.
4. **Creation equivalence.** With seed, passphrases, clock and random source fixed in a test
   build, compare the oracle dump of `udb.Initialize` against the Rust creation path. Random
   salts, nonces and the creation timestamps (`mgrcreated`, `date`) must be injected or masked.

---

## 12. Rust porting notes

### 12.1 Suggested module boundary

A module `upgrade` inside the wallet database crate (the crate that owns the `udb` codecs), with:

- `version` — read/write `meta/ver`; `DB_VERSION = 26`; the gate used by open.
- `upgrade::run(db, pub_passphrase, params)` — the driver (one read-write transaction).
- `upgrade::steps::{v2 … v26}` — one function per step, each taking the transaction and a
  context struct `{ pub_passphrase, params }`.
- `upgrade::legacy` — codecs used only by upgrades: version 1/2/5 account rows, `usedaddrs`,
  address-pool keys, pre-14 script rows, 32/36-byte VSP records, legacy `Migrate` pieces.
- `vsp` — the three VSP buckets.
- `treasury` — the four policy buckets.

Keep the rule stated at the top of `upgrades.go` (`wallet/udb/upgrades.go:25-28`): upgrade steps
must use only versioned raw codecs, never the "current" manager/store API.

### 12.2 Dependencies that can be reused (inspected)

| Need | Candidate | Note |
| --- | --- | --- |
| Genesis version 2 filter and key | `dcroxide:crates/dcroxide-gcs/src/blockcf2.rs:81` (`key`), `:230` (`regular`); `dcroxide:crates/dcroxide-gcs/src/lib.rs:536` (`FilterV2::bytes`) | `regular` takes a `PrevScripter`; supply one that always reports "not found". |
| Stake classification and commitment decoding | `dcroxide:crates/dcroxide-stake/src/lib.rs:808` (`is_sstx`), `:1214` (`determine_tx_type`), `:392` (`addr_from_sstx_pk_scr_commitment`), `:417` (`amount_from_sstx_pk_scr_commitment`) | Upgrade 12 needs only the hash160 from the commitment; the address object is optional. |
| Stake-change script predicates | `dcroxide:crates/dcroxide-txscript/src/stdscript.rs:654`, `:666` | Version 0 forms; the Go wrappers return false for any other script version. |
| Genesis block and hash per network | `dcroxide:crates/dcroxide-chaincfg/src/lib.rs:143-144` | |
| Transaction decoding | `dcroxide-wire` (`MsgTx`, as used by the stake functions above) | |
| Extended public key parsing and child derivation | `dcr-rs:src/hd.rs:560-660` (`ExtPubKey::from_base58`, `derive_child`, `compressed_pubkey`) | Public child derivation is identical in legacy and strict BIP32 modes (`dcrd/hdkeychain/v3@v3.1.3/extendedkey.go:288-340`). `derive_child` reports one `Error::Derivation` for both "invalid child" and depth overflow; upgrade 2 must distinguish the invalid-child case (skip) from other failures (abort). |

Not found in dcroxide or dcr-rs (must come from other dcr-purse work): a bbolt-compatible
storage engine, scrypt + XSalsa20-Poly1305 secretbox wrappers matching `snacl`, the
address-manager and transaction-store codecs, the wallet `errors` kinds. By their titles the
sibling specifications `01-walletdb-bbolt.md`, `02-address-manager.md`, `03-txstore-formats.md`
and `04-txstore-behavior.md` own those; this document was written without reading them, so any
overlap (account rows, credit flags, `cf`, `birthstate`) should be reconciled, with the Go source
as the arbiter.

### 12.3 Hazards

1. **Two byte orders** in one file (big-endian here and in the transaction store, little-endian
   in the address manager). Every table in section 4 states which.
2. **Nil-bucket panics** in Go become explicit errors in Rust; decide the kind (`IO` recommended)
   and keep the transaction rollback.
3. **Iteration while mutating.** Collect, then write, in every step (Go already does so except in
   upgrade 17).
4. **Upgrade 2 cost.** Up to one EC point addition per generated address per branch; a wallet
   with many addresses takes seconds. No progress logging exists upstream.
5. **scrypt memory** (256 MiB at N = 2¹⁸) during upgrades 2 and 14 and four times during
   creation.
6. **Creation must not regress to "write version 26 directly" without proving equivalence** to
   section 4.7 (Q2, Q3, Q4, Q21 are easy to miss).
7. **Do not "clean up" dead data** (`wstakemgr`, `mgrver`, stale `usedaddrs`): the Go wallet does
   not, and a user may switch back.
8. **`birthstate` time field** is a signed value stored through an unsigned cast; decode as
   `i64`.
9. **Sub-slices of database memory** must be copied before the transaction ends (Q26).
10. **Upgrade 14 non-determinism** means golden-file tests for that branch must mask the nonce
    and add-time.
11. **Versioned helpers.** Every helper in section 3.6 must exist in the form the upgrade's
    source version expects (account rows at versions 1, 4 and 16; block records with the
    stake-invalidated byte; pre-14 script rows). Reusing the "current" codec of the address
    manager or transaction store inside an upgrade step is the mistake the note at the top of
    `upgrades.go` warns about.
12. **Empty versus absent values** (Q46): model a present key with a zero-length value as
    `Some(empty)`, never as `None`, and do not copy Go's `v != nil` loop conditions.
13. **Unchecked length fields** (`rdlen`, `encPubLen`, `nameLen`, host and pubkey lengths) index
    past the buffer in Go; every one needs a bounds check that yields `IO`.
14. **Bulk treasury readers** must reproduce the "re-lookup and drop" outcome for odd-length
    keys (section 5.8) or the in-memory policy maps can differ from Go's on a damaged database.

### 12.4 Facts for the minimum-supported-version decision

What each floor costs beyond the floor above it:

| Floor (oldest database accepted) | Covers releases since | Extra work versus the next row |
| --- | --- | --- |
| 26 | 2.0.0 (2024-05) | None for existing databases. Creation must still yield the section 4.7 layout. |
| 25 | 1.7.0 (2022-01) | Upgrade 26: write one fixed 45-byte key. Trivial. |
| 22 | 1.6.0 (2021-01) | Upgrades 23–25: four buckets (`vsphost`, `vsppubkey`, `vsptreasurypolicy`, `vsptspendpolicy`), two seed keys, append 4 zero bytes to each `vsp` value. Trivial. |
| 13 | 1.5.0 (2019-11) | Upgrades 14–22. Real work: 14 (public-key decryption, script address row rewrite, imports from `sc`), 17 (account row split into row + variables). Small: 15 (genesis filter; needed for creation anyway), 21 (append status). Rest: empty buckets. Requires reading version 5–16 account rows and pre-14 script rows. |
| 11 | 1.3.0 (2018-08) | Upgrades 12–13. Real work: 12 (walk all stored transactions, classify, build commitment indexes). |
| 9 | 1.2.0 (2018-04) | Upgrades 10–11: one bucket, two keys. Trivial. |
| 3 … 7 | 1.0.0 … 1.1.2 (2017) | Upgrades 4–9. Real work: 6 (walk all transactions, classify tickets), 8+9 (walk all credits, load each transaction, rewrite one flag byte). Small: 5 (account row rewrite; requires version 2–4 rows), 4 (delete one key), 7 (bump). |
| 1 | development builds of March–April 2017; output of `Migrate` | Upgrades 2–3. Real work: 2 (public-key decryption, HD derivation scan, version 1 rows, `usedaddrs`). |
| pre-unified | 0.8.2 and earlier (≤ 2017-02) | `Migrate` plus the two legacy upgraders. No upstream fixture; the pre-unified layouts are specified here only as far as `Migrate` reveals them (section 4.6.1). |

Additional facts:

- The creation path in Go exercises upgrades 2–26 on an (almost) empty database on every wallet
  creation, so those code paths are current and not bit-rotted — but only for empty data. The
  data-bearing branches of upgrades 14, 21 and 23 have no upstream test.
- Database versions 1 and 2 never shipped in a tagged release (1.0.0 shipped version 3).
- Go itself has no minimum version: the pinned code accepts everything back to pre-unified.
  Choosing a floor above that is a deliberate divergence and needs a new, clearly worded error
  (no upstream string exists for "database too old").
- A user below a chosen floor can be told to open the wallet once with dcrwallet ≥ 2.0.0, which
  upgrades it to 26 in place.

### 12.5 Size estimate

**M** for full fidelity (versions 1–26 plus `Migrate`, VSP and treasury buckets): about 2,350
lines of Go in scope, of which roughly 1,200 are the upgrade steps; 14 steps are a few lines
each; the seven logic-bearing steps lean on codecs and primitives specified elsewhere. The
dominant cost is test data: an oracle harness and new fixtures for the uncovered branches. With a
floor of 22 or higher the upgrade part drops to **S**; the VSP and treasury buckets are **S** on
their own.

---

## 13. Open questions

1. **Minimum supported database version** (decision; facts in section 12.4).
2. **Creation route:** replay "version 1 + 25 upgrades" like Go, or write the version 26 layout
   directly and prove equivalence against the oracle dump (decision).
3. **Pre-unified layouts beyond what `Migrate` touches are UNVERIFIED.** `Migrate` assumes that a
   legacy database at manager version 6 / store version 3 already equals the unified version 1
   layout (account rows with "next" indexes, `usedaddrs`, address-pool keys). Settling it needs
   the dcrwallet 0.8.2 sources (`waddrmgr`, `wtxmgr`, `wstakemgr` packages at tag `v0.8.2`) and a
   real pre-unified wallet file. The value type of `wstakemgr/main/stakestorever` was not read.
4. **Mainnet, simnet and regnet genesis `cf` values** were not computed (only testnet3 was
   observed in a fixture). Settle by running the oracle or `blockcf2::regular` on each genesis
   block.
5. **Record formats of `wstakemgr/ssgenrecords` and `ssrtxrecords`** were not read in full; they
   are dead data and not needed unless a tool is expected to export them.
6. **Whether any shipped release wrote `vsp` records at version 20–22** with content that the
   21 and 23 rewrites must handle beyond "append bytes" is UNVERIFIED in the field; the code path
   itself is unambiguous. Settle with a 1.6.x wallet that used a VSP.
7. **Behaviour on malformed data:** Go panics in several places (sections 5.4, 5.7). Proposed:
   return `IO`. Confirm that no tooling depends on the crash.
8. **Error text parity for Q6 and Q22** (wrong function names; pointer-formatted values):
   reproduce the wrong names, or correct them? The pointer address cannot be reproduced.
9. **Result order of `GetVSPTicketsByFeeStatus`:** random in Go; pick a deterministic order
   (ascending ticket hash bytes) unless a client is found that needs otherwise.
10. **Interplay of upgrade 15 with sync:** the procedure that refills `cf` and flips
    `havecfilters` back to `01` is specified in the transaction-store and sync documents; this
    spec only fixes the state the upgrade leaves behind.
11. **Host string identity.** `vsppubkey` is keyed by the exact host bytes the caller passes
    (the configured VSP URL). Whether any caller normalises the URL before it reaches
    `SetVSPTicket` was not traced beyond `wallet/vsp.go:221-286` and
    `wallet/vspticket.go:246-290` (which pass `c.Client.URL` through unchanged); UNVERIFIED for the configuration and gRPC entry points. Settle in
    the VSP specification.
12. **Migrated wallets restart from genesis** when the legacy store was below version 3
    (section 5.5). Whether a real pre-unified wallet ever reaches `Migrate` with store version
    1 or 2 is UNVERIFIED; settle with the 0.8.2 sources.
13. **Step-by-step oracle comparison.** Upstream offers no way to stop `Upgrade` at an
    intermediate version (the table and functions are unexported). An oracle shim that calls
    individual upgrade functions must live inside package `udb` (a `_test.go` file or a patched
    copy); decide whether intermediate-state parity is a test goal or only end-state parity.
14. **Treasury policy value domain.** The storage layer returns any non-zero byte; whether the
    replacement should reject values other than `0x01`/`0x02` at load time (Go does not) is a
    decision.
15. **Watching-only creation layout** (section 4.7) is taken from `createWatchOnly` and the
    upgrade code only; no fixture confirms the post-upgrade bytes. Settle with an oracle dump
    of `udb.InitializeWatchOnly`.
16. **Sixth argument of `udb.Initialize` in the `v1`/`v2` generators** (`false`): meaning
    UNVERIFIED; settle by reading `udb.Initialize` at the commit that introduced `v1.db.go`.
    It affects only how those two fixtures could be regenerated.
