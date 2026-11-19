# 02 — Address manager: keys, accounts, derivation, encryption, and the `waddrmgr` buckets

## 1. Header

**Scope** (dcrwallet, Go, read in full):

| File | Lines | Contents |
| --- | ---: | --- |
| `wallet/udb/addressmanager.go` | 3216 | `Manager`, account cache, locking, key hierarchy, creation, every Manager method |
| `wallet/udb/addressdb.go` | 1783 | bucket names, row (de)serialization, index maintenance, legacy manager upgrades |
| `wallet/udb/address.go` | 236 | `ManagedAddress` interfaces and the two concrete address types |
| `wallet/udb/initialize.go` | 89 | `Initialize`, `InitializeWatchOnly` |
| `wallet/udb/open.go` | 54 | `Open` |
| `wallet/udb/metadata.go` | 37 | unified database version record |
| `wallet/internal/snacl/snacl.go` | 249 | scrypt + XSalsa20-Poly1305 secret keys (`snacl_test.go`, 118 lines) |
| `kdf/kdf.go` | 71 | Argon2id parameters for per-account passphrases |
| `wallet/cointype.go` | 61 | `Wallet.UpgradeToSLIP0044CoinType` |
| `dcrd/hdkeychain/v3@v3.1.3/extendedkey.go` | 636 | extended keys as the manager uses them |

Read for context because the scope depends on them (owned by other specs): `wallet/udb/upgrades.go` (the upgrades that rewrite address manager rows), `wallet/udb/migration.go`, `errors/errors.go`, `wallet/internal/bdb/db.go`, `wallet/walletdb/interface.go`, `internal/compat/compat.go`.

**Parity pin**: dcrwallet `54b85b33` (2.2.0-pre, module `decred.org/dcrwallet/v5`). Dependencies at the versions `go.mod` pins: `dcrd/hdkeychain/v3` v3.1.3, `dcrd/dcrec/secp256k1/v4` v4.4.1, `dcrd/chaincfg/v3` v3.3.0, `dcrd/dcrutil/v4` v4.0.3, `dcrd/txscript/v4` v4.1.2 (`stdaddr`), `dcrd/crypto/rand` v1.0.1, `golang.org/x/crypto` v0.57.0 (`scrypt`, `nacl/secretbox`, `argon2`, `chacha20poly1305`, `blake2b`), `go.etcd.io/bbolt` v1.3.12.

**Summary.** The address manager is the key store of the wallet. It lives in the top-level bbolt bucket `waddrmgr` and holds: two passphrase-derived master keys (scrypt), two random 32-byte crypto keys sealed by them (XSalsa20-Poly1305), the BIP0044 coin type keys, one row per account holding the account xpub/xpriv as encrypted base58 strings, a per-account variables bucket (name, last-used and last-returned child indexes, optional Argon2id parameters), one row per known address keyed by `SHA-256(HASH160)`, and name/id/address indexes. Everything is little-endian except the unified database version, which is big-endian. A database is always created in the version 1 layout and then run through all 25 upgrades in the same call; several on-disk oddities (the imported account's zeroed counters, the surviving `mgrver` key, the legacy-format `slip0044acct0` row, the post-coin-type-upgrade counters being 0 instead of `0xFFFFFFFF`) are artefacts of that and must be reproduced. Key derivation uses hdkeychain's `Child` everywhere, which is Decred's non-BIP32 variant; because the manager reloads account and coin type keys from their base58 strings, which of the two hardened-derivation layouts applies depends on whether the parent key was derived in memory or parsed from the database.

## 2. Role in the system

The address manager answers four questions for the rest of the wallet: which accounts exist and what are their extended keys; which addresses (by HASH160) belong to the wallet, to which account, and at which branch/index; what private key or redeem script spends an address; and how far each account branch has been used and handed out.

**Callers.**

- `wallet.Wallet` holds one `*udb.Manager` (`w.manager`) for the lifetime of an open wallet and calls the exported methods listed in section 3.4 (except `Close`, `ConvertToWatchingOnly`, `ForEachActiveAccountAddress`, `Encrypt` and `Decrypt`, which have no production caller) from inside `walletdb.View`/`walletdb.Update` closures. The wallet layer owns address buffers, gap-limit handling, discovery and the RPC surface; the manager only persists.
- `udb.Store` (the transaction store) is constructed in `Open` with `acctLookupFunc = addrMgr.AddrAccount` and `manager = addrMgr` (`wallet/udb/open.go:46-50`) and calls `manager.redeemScriptForHash160` (`wallet/udb/txmined.go:1798`, `wallet/udb/txmined.go:2907`).
- The database upgrades (`wallet/udb/upgrades.go`) use the `addressdb.go` primitives directly with explicit version numbers and never call Manager methods (`wallet/udb/upgrades.go:25-28`).
- `walletsetup.go` and `wallet/wallet.go` call the package function `HDKeysFromSeed` to preview keys without a database.

**Callees.** `walletdb` buckets and transactions (bbolt through `wallet/internal/bdb`), `snacl` (scrypt, secretbox), `kdf` (Argon2id), `hdkeychain`, `stdaddr`/`dcrutil` for HASH160 and address construction, `dcrd/crypto/rand` for all randomness, `chaincfg.Params` for HD version bytes and coin types, and the `WLLT` logger (`wallet/udb/log.go:13`, `log.go:33`, `internal/loggers/loggers.go:48`).

The manager never opens a database transaction itself except in `Initialize`, `InitializeWatchOnly` and `Open`. Every other entry point receives a bucket or transaction from the caller, so atomicity is the caller's transaction. The manager's in-memory state is not transactional (section 7).

## 3. Public surface inventory

### 3.1 Constants and package variables

| Name | Value | Source | Contract |
| --- | --- | --- | --- |
| `MaxAccountNum` | `0x7FFFFFFE` (2^31 − 2) | `wallet/udb/addressmanager.go:36` | Highest BIP0044 account number. |
| `MaxAddressesPerAccount` | `0x7FFFFFFF` | `wallet/udb/addressmanager.go:41` | Upper bound (inclusive) for a child index passed to `SyncAccountToAddrIndex`. |
| `ImportedAddrAccount` | `0x7FFFFFFF` (2^31 − 1) | `wallet/udb/addressmanager.go:47` | Account number of the reserved "imported" account. Accounts above it are imported xpub and imported voting accounts. |
| `ImportedAddrAccountName` | `"imported"` | `wallet/udb/addressmanager.go:50` | Reserved account name. |
| `DefaultAccountNum` | `0` | `wallet/udb/addressmanager.go:53` | |
| `defaultAccountName` | `"default"` | `wallet/udb/addressmanager.go:62` | Initial name of account 0; not reserved. |
| `maxCoinType` | `0x7FFFFFFF` | `wallet/udb/addressmanager.go:75` | Bound checked by `deriveCoinTypeKey`. |
| `ExternalBranch` | `uint32(0)` | `wallet/udb/addressmanager.go:80` | |
| `InternalBranch` | `uint32(1)` | `wallet/udb/addressmanager.go:85` | |
| `xchacha20NonceSize`, `poly1305TagSize`, `xchacha20poly1305Overhead` | 24, 16, 40 | `wallet/udb/addressmanager.go:160-164` | Per-account sealing overhead. |
| `CKTPrivate`, `CKTPublic` | `CryptoKeyType` 0 and 2 (1 is the removed `CKTScript`) | `wallet/udb/addressmanager.go:281-293` | Selector for `Manager.Encrypt`/`Decrypt`. |
| `latestMgrVersion` | `uint32(6)` (a `var`, so tests can change it) | `wallet/udb/addressdb.go:21` | Written to `main/mgrver` at creation. |
| `adtChain`, `adtImport`, `adtScript` | `addressType` 0, 1, 2 | `wallet/udb/addressdb.go:34-38` | Address row type byte. |
| `actBIP0044Legacy`, `actBIP0044`, `importedVoting` | `accountType` 0, 1, 2 | `wallet/udb/addressdb.go:50-54` | Account row type byte. |
| `newSecretKey`, `newCryptoKey` | function variables | `wallet/udb/addressmanager.go:240`, `:298` | Test injection points; production values are `snacl.NewSecretKey` with the network's scrypt options and `snacl.GenerateCryptoKey`. |
| `snacl.Overhead`, `KeySize`, `NonceSize` | 16, 32, 24 | `wallet/internal/snacl/snacl.go:22-24` | |
| `snacl.DefaultN`, `DefaultR`, `DefaultP` | 16384, 8, 1 | `wallet/internal/snacl/snacl.go:25-27` | Used only by `snacl_test.go`; the manager never uses them. |
| `kdf.MarshaledLen` | 25 | `kdf/kdf.go:42` | |
| `DBVersion` | 26 | `wallet/udb/upgrades.go:213` | Checked by `Open`; defined by the upgrades spec. |
| `initialVersion` | 1 | `wallet/udb/upgrades.go:31` | Version written by `Initialize` before upgrading. |
| `accountVariablesVersion` | 17 | `wallet/udb/upgrades.go:158` | Threshold inside `fetchDBAccount`: at or above it a type `00` row in `acct` is refused (section 4.6.3); below it the row is parsed with the legacy layout. |
| `metadataRootBucketKey`, `unifiedDBMetadataVersionKey` | `"meta"`, `"ver"` | `wallet/udb/metadata.go:15`, `:19` | |
| `waddrmgrBucketKey`, `wtxmgrBucketKey` | `"waddrmgr"`, `"wtxmgr"` | `wallet/udb/migration.go:18-19` | Top-level buckets. |

Bucket and key names are listed in section 4.

### 3.2 Types

| Type | Source | Contract |
| --- | --- | --- |
| `Manager` | `wallet/udb/addressmanager.go:302-345` | The key store. Fields: `mtx sync.RWMutex`; `chainParams`; `watchingOnly`, `locked`, `closed` bools; `acctInfo map[uint32]*accountInfo` cache; `masterKeyPub`, `masterKeyPriv *snacl.SecretKey`; `cryptoKeyPub EncryptorDecryptor`; `cryptoKeyPrivEncrypted []byte`; `cryptoKeyPriv EncryptorDecryptor`; `privPassphraseHasher hash.Hash` with its own mutex; `privPassphraseHash []byte`. |
| `accountInfo` | `wallet/udb/addressmanager.go:136-154` | Cached account: `acctName`, `acctType`, `acctKeyEncrypted` (ciphertext of the xpriv string), `acctKeyPriv`, `acctKeyPub`, `uniqueKey *kdf.Argon2idParams`, `uniquePassHasher`, `uniquePassHash`. |
| `AccountProperties` | `wallet/udb/addressmanager.go:213-224` | Alias of an anonymous struct: `AccountNumber uint32`, `AccountName string`, `AccountType uint8`, `LastUsedExternalIndex`, `LastUsedInternalIndex`, `LastReturnedExternalIndex`, `LastReturnedInternalIndex`, `ImportedKeyCount uint32`, `AccountEncrypted`, `AccountUnlocked bool`. |
| `EncryptorDecryptor` | `wallet/udb/addressmanager.go:244-250` | `Encrypt`, `Decrypt`, `Bytes`, `CopyBytes`, `Zero`. Implemented by `cryptoKey` (wraps `snacl.CryptoKey`). Exists for test injection. |
| `CryptoKeyType` | `wallet/udb/addressmanager.go:278` | `byte`. |
| `scryptOptions` | `wallet/udb/addressmanager.go:114-116` | `N, R, P int`. |
| `keyType` | `wallet/udb/addressmanager.go:168` | `[]byte` newtype for Argon2id-derived keys. |
| `ObtainUserInputFunc` | `wallet/udb/addressdb.go:27` | `func() ([]byte, error)`. Exported, unused in the repository. |
| `ManagedAddress` | `wallet/udb/address.go:17-38` | `Account() uint32`, `Address() stdaddr.Address`, `AddrHash() []byte`, `Imported() bool`, `Internal() bool`, `Multisig() bool`. |
| `ManagedPubKeyAddress` | `wallet/udb/address.go:42-50` | Adds `PubKey() []byte`, `Index() uint32`. |
| `ManagedScriptAddress` | `wallet/udb/address.go:54-59` | Adds `RedeemScript() (uint16, []byte)`; the version is always 0. |
| `managedAddress` | `wallet/udb/address.go:63-72` | P2PKH (ECDSA secp256k1, script version 0) address: `account`, `address`, `imported`, `internal`, `multisig` (never set true), `pubKey`, `index`. |
| `scriptAddress` | `wallet/udb/address.go:159-164` | P2SH v0 address: `account`, `address`, `redeemScript`. `Imported()` is always true, `Internal()` and `Multisig()` always false. |
| `dbAccountRow`, `dbBIP0044AccountRow`, `dbBIP0044Account`, `dbAddressRow`, `dbChainAddressRow`, `dbImportedAddressRow`, `dbScriptAddressRow`, `accountVarReader` | `wallet/udb/addressdb.go:57-170`, `:1070` | In-memory forms of the rows in section 4. `dbBIP0044AccountRow` is the legacy (pre-version-17) account row and always reports type 0; `dbBIP0044Account` is the current row and reports the stored type. |
| `snacl.CryptoKey` | `wallet/internal/snacl/snacl.go:32` | `[32]byte`. |
| `snacl.Parameters` | `wallet/internal/snacl/snacl.go:82-88` | `Salt [32]byte`, `Digest [32]byte`, `N, R, P int`. |
| `snacl.SecretKey` | `wallet/internal/snacl/snacl.go:92-95` | `Key *CryptoKey`, `Parameters`. |
| `kdf.Argon2idParams` | `kdf/kdf.go:18-23` | `Salt [16]byte`, `Time uint32`, `Memory uint32` (KiB), `Threads uint8`. |
| `unifiedDBMetadata` | `wallet/udb/metadata.go:12` | Empty struct namespace for the version record. |
| `hdkeychain.ExtendedKey` | `dcrd/hdkeychain/v3@v3.1.3/extendedkey.go:117-127` | Section 5.1. |

### 3.3 Package functions

| Function | Source | Contract |
| --- | --- | --- |
| `Initialize(ctx, db, params, seed, pubPass, privPass) error` | `wallet/udb/initialize.go:18` | Creates a seeded wallet database at version 1, then runs `Upgrade` to `DBVersion`. Section 5.3. |
| `InitializeWatchOnly(ctx, db, params, hdPubKey string, pubPass) error` | `wallet/udb/initialize.go:56` | Same for a watching-only wallet from one account xpub string. Section 5.4. |
| `Open(ctx, db, params, pubPass) (*Manager, *Store, error)` | `wallet/udb/open.go:21` | Verifies the version equals `DBVersion`, loads the manager (locked), builds the `Store`. Section 5.5. |
| `HDKeysFromSeed(seed, params) (ctLegacyPriv, ctSLIP0044Priv, acct0LegacyPriv, acct0SLIP0044Priv, err)` | `wallet/udb/addressmanager.go:2807` | Derives both coin type keys and both account 0 keys. Zeroes everything on error. Section 5.2. |
| `CoinTypes(params) (legacy, slip0044 uint32)` | `wallet/udb/addressmanager.go:2801` | Returns `params.LegacyCoinType`, `params.SLIP0044CoinType`. |
| `ValidateAccountName(name) error` | `wallet/udb/addressmanager.go:2069` | `""` → `Invalid` "accounts may not be named the empty string"; `"imported"` → `Invalid` "reserved account name". No other restriction (any bytes, any length). |
| `IsImportedVoting(acctType uint8) bool` | `wallet/udb/addressmanager.go:228` | `acctType == 2`. |
| `snacl.NewSecretKey(password *[]byte, N, r, p int)` | `wallet/internal/snacl/snacl.go:228` | Random 32-byte salt, scrypt key, digest = SHA-256(key). |
| `snacl.GenerateCryptoKey()` | `wallet/internal/snacl/snacl.go:74` | 32 random bytes. Never fails. |
| `(*CryptoKey).Encrypt/Decrypt/Zero`, `(*SecretKey).Marshal/Unmarshal/DeriveKey/Encrypt/Decrypt/Zero` | `wallet/internal/snacl/snacl.go` | Sections 4.4 and 5.6. |
| `kdf.NewArgon2idParams(rand io.Reader)` | `kdf/kdf.go:30` | `Time=1`, `Memory=262144`, `Threads=uint8(min(NumCPU,256))`, random 16-byte salt. |
| `(*Argon2idParams).MarshalBinary/UnmarshalBinary` | `kdf/kdf.go:46`, `:56` | Section 4.5. |
| `kdf.DeriveKey(password, p, len uint32) []byte` | `kdf/kdf.go:68` | `argon2.IDKey(password, salt, Time, Memory, Threads, len)`; forces a Go GC afterwards. |
| `(*Wallet).UpgradeToSLIP0044CoinType(ctx)` | `wallet/cointype.go:23` | One `walletdb.Update` calling the manager upgrade, then resets account 0's in-memory address buffers. Section 5.10. |

Unexported functions that carry observable behaviour (`createAddressManager`, `createWatchOnly`, `loadManager`, `deriveCoinTypeKey`, `deriveAccountKey`, `checkBranchKeys`, `deriveKey`, `seal`, `unseal`, `normalizeAddress`, `addressID`, `scryptOptionsForNet`, and every `fetch*`/`put*`/`delete*`/`serialize*`/`deserialize*`/`forEach*` primitive in `addressdb.go`) are specified in sections 4 and 5. `fetchManagerVersion`, `putManagerVersion`, `upgradeToVersion5`, `upgradeToVersion6` and `upgradeManager` (`wallet/udb/addressdb.go:291-312`, `:1697-1783`) serve only `Migrate` for pre-unified databases and are covered in section 5.20.

### 3.4 Manager methods

Lock column: `W` write lock, `R` read lock, `–` no lock taken. "ns" means the method takes the `waddrmgr` bucket; "tx" means it takes the transaction and opens `waddrmgr` itself.

| Method | Arg | Lock | Contract (details in section 5) |
| --- | --- | --- | --- |
| `WatchingOnly() bool` | – | – | Returns the flag without locking. |
| `Close() error` | – | W | Locks if unlocked and not watching-only, zeroes public key material, sets `closed`. Always nil. No production caller. |
| `ChainParams()` | – | – | |
| `IsLocked() bool` | – | R | Always true for watching-only managers. |
| `Lock() error` | – | W | `WatchingOnly` if watching-only (checked before locking the mutex); `Locked` if already locked. |
| `Unlock(ns, passphrase) error` | ns (unused) | W | Section 5.7. |
| `UnlockedWithPassphrase(passphrase) error` | – | R | nil if unlocked and the passphrase hash matches; `WatchingOnly`, `Locked` or `Passphrase` otherwise. |
| `ChangePassphrase(ns, old, new, private bool) error` | ns rw | W | Section 5.8. |
| `ConvertToWatchingOnly(ns) error` | ns rw | W | Section 5.9. Test-only caller. |
| `CoinType(tx) (uint32, error)` | tx | – | Legacy if `main/ctpub` exists, else SLIP0044 if `main/ctpub-slip0044` exists, else `WatchingOnly`. |
| `CoinTypePrivKey(tx) (*ExtendedKey, error)` | tx | R | Decrypts and parses the active coin type xpriv. `Locked`, `WatchingOnly`. |
| `UpgradeToSLIP0044CoinType(tx) error` | tx rw | W (late) | Section 5.10. |
| `AccountProperties(ns, account)` | ns | R | Section 5.11. |
| `AccountExtendedPubKey(tx, account)` | tx | W | Returns the cached xpub pointer. `Invalid` for the imported account. |
| `AccountExtendedPrivKey(tx, account)` | tx | W | Returns the cached xpriv pointer. Section 5.11. |
| `AccountBranchExtendedPubKey(tx, account, branch)` | tx | W then – | `AccountExtendedPubKey(...).Child(branch)`. |
| `AccountName(ns, account)` | ns | – | Reads `acctididx`. |
| `LookupAccount(ns, name)` | ns | – | Reads `acctnameidx`. |
| `LastAccount(ns)` | ns | – | Reads `meta/lastaccount`. |
| `LastImportedAccount(tx)` | tx | – | Reads `meta/lastimportedaccount`, default `0x7FFFFFFF`. |
| `ForEachAccount(ns, fn)` | ns | – | Iterates `acct` in key order. Section 5.19. |
| `NewAccount(ns, name) (uint32, error)` | ns rw | W | Section 5.12. |
| `RenameAccount(ns, account, name) error` | ns rw | W | Section 5.13. |
| `ImportXpubAccount(ns, name, xpub) error` | ns rw | W | Section 5.14. |
| `ImportVotingAccount(tx, xpriv, passphrase, name) (uint32, error)` | tx rw | W | Section 5.14. |
| `AccountHasPassphrase(tx, account) (has, unlocked bool)` | tx | R | Section 5.15. |
| `SetAccountPassphrase(tx, account, passphrase) error` | tx rw | W | Section 5.15. |
| `UnlockAccount(tx, account, passphrase) error` | tx | W | Section 5.15. |
| `LockAccount(tx, account) error` | tx | W | Section 5.15. |
| `Address(ns, addr) (ManagedAddress, error)` | ns | W | Section 5.16. |
| `AddrAccount(ns, addr) (uint32, error)` | ns | – | Reads `addracctidx`. |
| `ExistsHash160(ns, hash160) bool` | ns | – | `addr` bucket has `SHA-256(hash160)`. |
| `ForEachAccountAddress(ns, account, fn)` | ns | W | Section 5.19. |
| `ForEachActiveAccountAddress(ns, account, fn)` | ns | W | Identical to `ForEachAccountAddress`. No caller. |
| `ForEachActiveAddress(ns, fn)` | ns | W | Every row of `addr`. Section 5.19. |
| `PrivateKey(ns, addr) (key, done func(), err)` | ns | W | Section 5.17. |
| `HavePrivateKey(ns, addr) (bool, error)` | ns | R | Section 5.17. |
| `RedeemScript(ns, addr) ([]byte, error)` | ns | – | Section 5.17. |
| `ImportPrivateKey(ns, wif) (ManagedPubKeyAddress, error)` | ns rw | W | Section 5.18. |
| `ImportPublicKey(ns, pubkey) (ManagedPubKeyAddress, error)` | ns rw | W | Section 5.18. |
| `ImportScript(ns, script) (ManagedScriptAddress, error)` | ns rw | W | Section 5.18. |
| `SyncAccountToAddrIndex(ns, account, syncToIndex, branch) error` | ns rw | W | Section 5.19. |
| `RecordDerivedAddress(tx, account, branch, child, pubkey) error` | tx rw | W | Section 5.19. |
| `MarkUsed(tx, addr) error` | tx rw | – | Section 5.19. |
| `MarkUsedChildIndex(tx, account, branch, child) error` | tx rw | – | Section 5.19. |
| `MarkReturnedChildIndex(tx, account, branch, child) error` | tx rw | – | Section 5.19. |
| `Encrypt(keyType, in)`, `Decrypt(keyType, in)` | – | W | Seal/open with the selected crypto key. No production caller. Section 5.21. |

### 3.5 Go identifiers for bucket and key names

The upgrades and the transaction store refer to these package variables by name; the literal is what reaches the database (`wallet/udb/addressdb.go:173-265`, `:1031-1038`).

| Go identifier | Literal | Where | Source |
| --- | --- | --- | --- |
| `mainBucketName` | `main` | nested bucket of `waddrmgr` | `wallet/udb/addressdb.go:243` |
| `addrBucketName` | `addr` | nested bucket | `:180` |
| `acctBucketName` | `acct` | nested bucket | `:178` |
| `acctVarsBucketName` | `acctvars` | nested bucket | `:179` |
| `addrAcctIdxBucketName` | `addracctidx` | nested bucket | `:192` |
| `acctNameIdxBucketName` | `acctnameidx` | nested bucket | `:199` |
| `acctIDIdxBucketName` | `acctididx` | nested bucket | `:206` |
| `metaBucketName` | `meta` | nested bucket (`waddrmgr/meta`) | `:210` |
| `usedAddrBucketName` | `usedaddrs` | nested bucket, removed by database version 2 | `:264` |
| `mgrVersionName` | `mgrver` | `main` | `:246` |
| `mgrCreateDateName` | `mgrcreated` | `main` | `:247` |
| `seedName` | `seed` | `main`, removed by database version 4; referenced only by the upgrade and its test | `:250` |
| `masterPrivKeyName`, `masterPubKeyName` | `mpriv`, `mpub` | `main` | `:251-252` |
| `cryptoPrivKeyName`, `cryptoPubKeyName` | `cpriv`, `cpub` | `main` | `:253-254` |
| `cryptoScriptKeyName` | `cscript` | `main`, removed by database version 14 | `:255` |
| `coinTypeLegacyPrivKeyName`, `coinTypeLegacyPubKeyName` | `ctpriv`, `ctpub` | `main` | `:256-257` |
| `coinTypeSLIP0044PrivKeyName`, `coinTypeSLIP0044PubKeyName` | `ctpriv-slip0044`, `ctpub-slip0044` | `main` | `:258-259` |
| `watchingOnlyName` | `watchonly` | `main` | `:260` |
| `slip0044Account0RowName` | `slip0044acct0` | `main` | `:261` |
| `lastAccountName` | `lastaccount` | `waddrmgr/meta` | `:237` |
| `lastImportedAccountName` | `lastimportedaccount` | `waddrmgr/meta` | `:241` |
| `addrPoolKeyPrefixExt`, `addrPoolKeyPrefixInt` | `addrpoolext`, `addrpoolint` (11 bytes each) | key prefixes in `waddrmgr/meta`, removed by database version 2 | `:225`, `:233` |
| `addrPoolMetaKeyLen` | 15 (a `var`, not a constant) | prefix + 4-byte account | `:215` |
| `acctVarLastUsedExternal`, `acctVarLastUsedInternal` | `extused`, `intused` | `acctvars/<acct>/` | `:1032-1033` |
| `acctVarLastReturnedExternal`, `acctVarLastReturnedInternal` | `extret`, `intret` | `acctvars/<acct>/` | `:1034-1035` |
| `acctVarName` | `name` | `acctvars/<acct>/` | `:1036` |
| `acctVarKDF` | `kdf-params` | `acctvars/<acct>/` | `:1037` |
| `nullVal` | the one byte `00` | value of every entry in `addracctidx/<acct>/` | `:175` |
| local `lastDefaultAddsrName` | `lastaddrs` | `main`, pre-unified only | `:1698` |
| local `syncBucketName` | `sync` | nested bucket, pre-unified only | `:1713` |
| `metadataRootBucketKey` | `meta` | root bucket | `wallet/udb/metadata.go:15` |
| `unifiedDBMetadataVersionKey` | `ver` (a string constant) | root `meta` | `wallet/udb/metadata.go:19` |

### 3.6 Additional types

| Type | Source | Contract |
| --- | --- | --- |
| `addressType` | `wallet/udb/addressdb.go:30` | `uint8`; values in section 3.1. "They must remain stable as their values are recorded to the DB." |
| `accountType` | `wallet/udb/addressdb.go:46` | `uint8`; values in section 3.1. Same stability requirement. |
| `dbAccount` | `wallet/udb/addressdb.go:40-43` | Interface `accountType() accountType`, `rowData() []byte`, implemented by `*dbBIP0044AccountRow` (always reports type 0) and `*dbBIP0044Account` (reports the stored type byte). The return type of `fetchDBAccount`; callers type-switch on it. |
| `cryptoKey` | `wallet/udb/addressmanager.go:253-265` | Struct embedding `snacl.CryptoKey`; the production `EncryptorDecryptor`. `Bytes()` returns a slice **aliasing** the 32 key bytes (the comment says "copy"); `CopyBytes(from)` copies up to 32 bytes into the key. |
| `hdkeychain.NetworkParams` | `dcrd/hdkeychain/v3@v3.1.3/extendedkey.go:104-112` | Interface `HDPrivKeyVersion() [4]byte`, `HDPubKeyVersion() [4]byte`; satisfied by `*chaincfg.Params`. The only network input to extended keys. |

Struct fields not listed in section 3.2: `managedAddress.manager` and `scriptAddress.manager` are back pointers to the `Manager` that are set by the constructors and never read (`wallet/udb/address.go:64`, `:160`). `dbAddressRow` is `addrType`, `account uint32`, `addTime uint64`, `rawData`; `dbChainAddressRow` adds `branch`, `index`; `dbImportedAddressRow` adds `encryptedPubKey`, `encryptedPrivKey`; `dbScriptAddressRow` adds `encryptedHash`, `script` (`wallet/udb/addressdb.go:141-170`). `dbAccountRow` is `acctType accountType`, `rawData []byte`. `dbBIP0044AccountRow` (legacy) embeds it and adds `pubKeyEncrypted`, `privKeyEncrypted []byte`, `nextExternalIndex`, `nextInternalIndex uint32` ("Removed by version 2"), `lastUsedExternalIndex`, `lastUsedInternalIndex uint32` ("Added in version 2"), `lastReturnedExternalIndex`, `lastReturnedInternalIndex uint32` ("Added in version 5"), `name string`; all six counters exist in memory at every version and the ones a version does not serialize read back as 0 (`wallet/udb/addressdb.go:57-75`). `dbBIP0044Account` (current) embeds `dbAccountRow` and adds `pubKeyEncrypted`, `privKeyEncrypted []byte` (the only two fields stored in the row) and `lastUsedExternalIndex`, `lastUsedInternalIndex`, `lastReturnedExternalIndex`, `lastReturnedInternalIndex uint32`, `name string`, `uniqueKey *kdf.Argon2idParams` (all six stored in the variables bucket) (`wallet/udb/addressdb.go:82-94`). `accountVarReader` has the single field `err error` and is a sticky-error reader: after the first failure every later `get*` call returns the zero value without touching the bucket (`wallet/udb/addressdb.go:1070-1106`).

Compile-time assertions `var _ ManagedPubKeyAddress = (*managedAddress)(nil)` and `var _ ManagedScriptAddress = (*scriptAddress)(nil)` (`wallet/udb/address.go:75`, `:167`) fix which interface each concrete type satisfies: callers type-assert a `ManagedAddress` to one of the two.

### 3.7 Unexported functions

Every unexported function in scope. "→ 5.x" means the algorithm is given there. All `put*`/`delete*` primitives wrap a bucket write failure as kind `IO` unless noted; all `fetch*` primitives dereference their nested bucket without a nil check (section 5.22).

`wallet/udb/addressmanager.go`:

| Function | Line | Contract |
| --- | ---: | --- |
| `isReservedAccountName(name)` | 91 | `name == "imported"`. |
| `isReservedAccountNum(acct)` | 97 | `acct == 0x7FFFFFFF`. |
| `normalizeAddress(addr)` | 104 | → 5.16. |
| `scryptOptionsForNet(net)` | 119 | simnet → `{2, 1, 1}`; every other network value → `{262144, 8, 1}`. Section 4.4.1. |
| `argon2idKey(password, params)` | 156 | `keyType(kdf.DeriveKey(password, params, 32))`. |
| `seal(key, plaintext)` | 170 | Section 4.5.2. The error result is always nil; a key that is not 32 bytes panics. |
| `unseal(key, ciphertext)` | 185 | Builds the AEAD first (panic on bad key length), then the length check, then `Open`. Errors in 5.15 step 5. |
| `defaultNewSecretKey(passphrase, opts)` | 233 | `snacl.NewSecretKey(passphrase, opts.N, opts.R, opts.P)`; production value of `newSecretKey`. |
| `defaultNewCryptoKey()` | 268 | Wraps `snacl.GenerateCryptoKey()` in a `cryptoKey`; production value of `newCryptoKey`. |
| `zero(b)` | 347 | Overwrites a byte slice with zeros. (`snacl` has its own copy, `wallet/internal/snacl/snacl.go:97`.) |
| `(*Manager).lock()` | 357 | → 5.7. |
| `(*Manager).zeroSensitivePublicData()` | 382 | For every cached account call `acctKeyPub.Zero()` and set it nil; zero `cryptoKeyPub` and the derived `masterKeyPub`. Called only by `Close`. |
| `(*Manager).keyToManaged(pubKey, account, branch, index)` | 422 | `newManagedAddressWithoutPrivKey`, then `internal = (branch == 1)` and `index`. |
| `deriveKey(acctInfo, branch, index, private)` | 438 | → 5.16. |
| `(*Manager).loadAccountInfo(ns, account)` | 475 | → 5.11. |
| `(*Manager).deriveKeyFromPath(ns, account, branch, index, private)` | 872 | → 5.16. |
| `(*Manager).chainAddressRowToManaged(ns, row)` | 890 | → 5.16 (chained row). Copies the public key before zeroing a private derived key. |
| `(*Manager).importedAddressRowToManaged(row)` | 917 | → 5.16 (imported key row). |
| `(*Manager).scriptAddressRowToManaged(row)` | 935 | → 5.16 (script row). A decrypted hash that is not 20 bytes makes `newScriptAddress` fail with the unwrapped stdaddr error `script hash is %d bytes vs required %d bytes` (`dcrd/txscript/v4@v4.1.2/stdaddr/addressv0.go:920-928`). |
| `(*Manager).rowInterfaceToManaged(ns, row)` | 950 | Dispatch on the three row types; any other type → `Invalid "address type %T"` (unreachable: `fetchAddressByHash` returns only those three). |
| `addressID(addr)` | 968 | → 5.16. |
| `(*Manager).loadAddress(ns, addr)` | 982 | → 5.16. |
| `(*Manager).removeAccountPassphrase(ns, account, acctInfo)` | 1791 | → 5.15. |
| `(*Manager).accountHasPassphrase(ns, account)` | 1857 | → 5.15. |
| `(*Manager).syncAccountToAddrIndex(ns, account, syncToIndex, branch)` | 1985 | → 5.19; the exported method is this under the write lock. |
| `(*Manager).importAccount(tx, acctType, xpriv, name)` | 2241 | → 5.14 step 2. Only ever called with `importedVoting`. |
| `(*Manager).redeemScriptForHash160(ns, hash160)` | 2559 | → 5.17. Called directly by the transaction store. Unreachable default: `Invalid "address row type %T"`. |
| `(*Manager).selectCryptoKey(keyType)` | 2581 | → 5.21. |
| `newManager(params, masterKeyPub, masterKeyPriv, cryptoKeyPub, cryptoKeyPrivEncrypted, hasher)` | 2641 | → 5.5 step 8. `watchingOnly` is set by the caller afterwards. |
| `deriveCoinTypeKey(master, coinType)` | 2664 | `coinType > 0x7FFFFFFF` → `Invalid "coin type %d"`; else `master.Child(44').Child(coinType')`, errors unwrapped. The intermediate purpose key is not zeroed. |
| `deriveAccountKey(coinTypeKey, account)` | 2698 | `account > 0x7FFFFFFE` → `Invalid "account %d"`; else `coinTypeKey.Child(account')`. |
| `checkBranchKeys(acctKey)` | 2719 | Derives child 0 then child 1 and discards them; returns the first error. |
| `loadManager(ns, pubPass, params)` | 2733 | → 5.5. |
| `createAddressManager(ns, seed, pubPass, privPass, params)` | 2895 | → 5.3. |
| `createWatchOnly(ns, hdPubKey, pubPass, params)` | 3090 | → 5.4. |

`wallet/udb/address.go`:

| Function | Line | Contract |
| --- | ---: | --- |
| `newManagedAddressWithoutPrivKey(m, account, pubKey)` | 139 | Address = `NewAddressPubKeyHashEcdsaSecp256k1V0(HASH160(pubKey), params)`; `imported`, `internal`, `multisig` false, `index` 0; `pubKey` is stored without copying (it aliases the caller's slice). The pubkey bytes are hashed as given, whatever their length. |
| `newScriptAddress(m, account, scriptHash, redeemScript)` | 223 | Address = `NewAddressScriptHashV0FromHash(scriptHash, params)`; script stored without copying. |
| `(*managedAddress)` `Account`, `Address`, `AddrHash`, `Imported`, `Internal`, `Multisig`, `PubKey`, `Index` | 80-134 | Field accessors. `AddrHash` is the 20-byte HASH160 of the address. `PubKey` returns the internal slice. |
| `(*scriptAddress)` `Account`, `Address`, `AddrHash`, `Imported`, `Internal`, `Multisig`, `RedeemScript` | 173-220 | `Imported` true, `Internal` and `Multisig` false, `RedeemScript` returns `(0, script)`. |

`wallet/udb/addressdb.go`:

| Function | Line | Contract |
| --- | ---: | --- |
| `(*dbBIP0044AccountRow).accountType/rowData` | 77-78 | Constant `actBIP0044Legacy`; the embedded raw data. |
| `(*dbBIP0044Account).accountType/rowData` | 96-97 | Stored type byte; the embedded raw data. |
| `(*dbBIP0044Account).serializeRow()` | 99 | Section 4.6.3 layout; also stores the result in `rawData`. Callers must call it after changing either key field and before `putAccountRow`. |
| `(*dbBIP0044Account).deserializeRow(v)` | 115 | Section 4.6.3 parse and validation. |
| `uint32ToBytes(n)` | 269 | 4 bytes little-endian. |
| `stringToBytes(s)` | 277 | Section 4.2. |
| `fetchManagerVersion(ns)`, `putManagerVersion(ns, v)` | 291, 303 | `main/mgrver` as u32 LE; → 5.20. The read does not check the value length. |
| `fetchMasterKeyParams(ns)` | 318 | Copies of `mpub` (missing → `IO "missing master pubkey params"`) and `mpriv` (nil when absent). |
| `putMasterKeyParams(ns, pub, priv)` | 343 | Writes `mpriv` if non-nil, then `mpub` if non-nil. |
| `fetchCoinTypeKeys(ns)` | 367 | → 5.10. |
| `putCoinTypeLegacyKeys(ns, pub, priv)` | 396 | Writes `ctpub` if non-nil, then `ctpriv` if non-nil. |
| `putCoinTypeSLIP0044Keys(ns, pub, priv)` | 419 | Writes `ctpub-slip0044` if non-nil, then `ctpriv-slip0044` if non-nil. |
| `fetchCryptoKeys(ns)` | 443 | Copies of `cpub` (missing → `IO "missing encrypted crypto pubkey"`) and `cpriv` (nil when absent). |
| `putCryptoKeys(ns, pub, priv)` | 468 | Writes `cpub` if non-nil, then `cpriv` if non-nil. |
| `fetchWatchingOnly(ns)`, `putWatchingOnly(ns, b)` | 489, 501 | Section 4.3. A missing key reports `bad watching-only flag len 0`. |
| `deserializeAccountRow(b)`, `serializeAccountRow(row)` | 518, 540 | Section 4.6.1. |
| `deserializeBIP0044AccountRow(accountID, row, dbVersion)`, `serializeBIP0044AccountRow(row, dbVersion)` | 555, 612 | Section 4.6.2, selected by `dbVersion` (1; 2–4; ≥ 5). `accountID` is used only in the error message. |
| `bip0044AccountInfo(pubEnc, privEnc, nextExt, nextInt, lastUsedExt, lastUsedInt, lastRetExt, lastRetInt, name, dbVersion)` | 660 | Builds a type `00` row keeping only the counters that exist at `dbVersion` (the others are forced to 0) and serializes it. |
| `forEachAccount(ns, fn)` | 698 | → 5.19 (`ForEachAccount`). |
| `fetchLastAccount(ns)`, `putLastAccount(ns, n)` | 711, 1009 | Section 4.9. |
| `fetchLastImportedAccount(ns)`, `putLastImportedAccount(ns, n)` | 724, 1020 | Section 4.9. |
| `fetchAccountName(ns, account)` | 745 | `acctididx` lookup; missing → `NotExist "no account %d"`. |
| `fetchAccountByName(ns, name)` | 761 | `acctnameidx` lookup; missing → `NotExist` with message `no account %q` (Go-quoted name). The value length is not checked. |
| `fetchAccountRow(ns, account)` | 774 | `acct` lookup and envelope parse; missing → `NotExist "no account %d"`. |
| `fetchAccountInfo(ns, account, dbVersion)` | 788 | Legacy loader used only by the version 2 and version 5 upgrades (`wallet/udb/upgrades.go:305`, `:480`): accepts type `00` **and type `02`** and parses both with the legacy layout; any other type (including `01`) → `IO "unknown account type %d"`. |
| `fetchDBAccount(ns, account, dbVersion)` | 803 | Section 4.6.3 and 4.7. Reads the six variables in the order `extused`, `intused`, `extret`, `intret`, `name`, `kdf-params`, then parses the row. |
| `accountVarsBucket(ns, account)` | 853 | `acctvars/<u32 LE account>` as a read-write bucket; nil when absent. Used by `RenameAccount`. |
| `deleteAccountNameIndex(ns, name)`, `deleteAccountIDIndex(ns, account)` | 859, 871 | Delete one index entry; deleting an absent key is not an error. |
| `putAccountNameIndex(ns, account, name)`, `putAccountIDIndex(ns, account, name)` | 883, 895 | Section 4.8. |
| `putAddrAccountIndex(ns, account, addrHash)` | 907 | Section 4.11: top-level entry, then `CreateBucketIfNotExists(<account>)`, then the `00` entry. |
| `putAccountRow(ns, account, row)` | 930 | `acct[<account>] = envelope(row)`. |
| `putBIP0044AccountInfo(ns, account, legacyRow)` | 942 | Legacy writer: `acct` row, then `acctididx`, then `acctnameidx`. Used by creation and by upgrades 2 and 5. Writes no variables bucket. |
| `putNewBIP0044Account(ns, account, a)` | 956 | Current writer: `acct` row, `acctididx`, `acctnameidx`, `CreateBucketIfNotExists` of the variables bucket, then `extused`, `intused`, `extret`, `intret`, `name`, and `kdf-params` only if `a.uniqueKey != nil`. It never deletes an existing `kdf-params` and never deletes index entries for a previous name. The bucket-creation error is returned as the database driver wrapper produced it, without the `IO` wrap (kind `Invalid` for a rejected key or a read-only transaction, otherwise no kind; `wallet/internal/bdb/db.go:18-36`). |
| `putAccountUint32Var`, `putAccountStringVar`, `putAccountKDFVar` | 1040, 1050, 1058 | Section 4.7 encodings. |
| `(*accountVarReader).getAccountUint32Var/getAccountStringVar/getAccountKDFVar` | 1074, 1087, 1095 | Section 4.7. A `kdf-params` value of the wrong length sets the reader error to the plain `invalid marshaled Argon2id parameters`. |
| `deserializeAddressRow`, `serializeAddressRow` | 1111, 1136 | Section 4.10 header. |
| `deserializeChainedAddress`, `serializeChainedAddress` | 1156, 1177 | Section 4.10 type `00`. |
| `deserializeImportedAddress`, `serializeImportedAddress` | 1190, 1221 | Section 4.10 type `01`. Trailing bytes after the private field are ignored on read. |
| `deserializeScriptAddress`, `serializeScriptAddress` | 1241, 1272 | Section 4.10 type `02`. Trailing bytes ignored on read. |
| `fetchAddressByHash(ns, addrHash)` | 1296 | `addr` lookup by the 32-byte key; missing → `NotExist "no address with hash %x"`. |
| `fetchAddress(ns, addressID)` | 1326 | Hashes the id and calls `fetchAddressByHash`; a `NotExist` is replaced by `NotExist "no address with id %x"` (the 20-byte id in lower-case hex). |
| `putAddress(ns, addressID, row)` | 1337 | → 5.19. |
| `putChainedAddress`, `putImportedAddress`, `putScriptAddress` | 1354, 1368, 1383 | Build the row with `addTime = now` (Unix seconds) and call `putAddress`. |
| `existsAddress(ns, addressID)` | 1397 | `addr[SHA-256(id)]` is present. |
| `fetchAddrAccount(ns, addressID)` | 1407 | `addracctidx[SHA-256(id)]` as u32 LE; missing → `NotExist "no address for id %x"` (replaced by `AddrAccount`, section 5.16). |
| `forEachAccountAddress(ns, account, fn)`, `forEachActiveAddress(ns, fn)` | 1420, 1451 | → 5.19. Both skip entries whose value is nil (nested buckets) and wrap a row load failure in `IO`. |
| `deletePrivateKeys(ns, dbVersion)` | 1483 | → 5.9. |
| `accountNumberToAddrPoolKey(isInternal, account)` | 1593 | 11-byte prefix (`addrpoolint` or `addrpoolext`) followed by the account as u32 LE. |
| `putNextToUseAddrPoolIdx(ns, isInternal, account, index)` | 1609 | `waddrmgr/meta[<pool key>] = u32 LE index`. Creation-time only. |
| `managerExists(ns)` | 1625 | `waddrmgr/main` exists. |
| `createManagerNS(ns)` | 1633 | → 5.3 step 3. Bucket creation order: `main`, `addr`, `acct`, `addracctidx`, `usedaddrs`, `acctnameidx`, `acctididx`, `meta`. |
| `upgradeToVersion5`, `upgradeToVersion6`, `upgradeManager` | 1697, 1712, 1723 | → 5.20. |

`wallet/udb/metadata.go`: `(unifiedDBMetadata).rootBucketKey()` returns `meta` (`:17`); `putVersion(bucket, v)` writes `ver` as u32 big-endian, failure → `IO` (`:21`); `getVersion(bucket)` → section 4.13 (`:31`).

`wallet/internal/snacl/snacl.go`: `(*SecretKey).deriveKey(op, password)` (`:104`) runs scrypt with output length 32, copies the result into `Key`, zeroes the temporary and calls `debug.FreeOSMemory()`; shared by `NewSecretKey` and `DeriveKey`, which pass their own op. `zero` (`:97`).

hdkeychain items not covered in section 5.1 (`dcrd/hdkeychain/v3@v3.1.3/extendedkey.go`): constants `RecommendedSeedLen = 32`, `HardenedKeyStart = 0x80000000`, `MinSeedBytes = 16`, `MaxSeedBytes = 64`, `serializedKeyLen = 78` (`:27-51`); `masterKey = "Bitcoin seed"` (`:99`); `newExtendedKey` (`:132`, plain constructor, public key left unmemoized); `pubKeyBytes` (`:157`, returns the key for public keys, otherwise computes and memoizes the compressed public key when the memo is empty); accessors `ChildNum() uint32`, `Depth() uint16`, `ParentFingerprint() uint32` (the 4 fingerprint bytes read big-endian) (`:174-196`), none of which dcrwallet's non-test code calls; `hash160`, `doubleBlake256Cksum`, `paddedAppend` (`:199-211`, `:433`); `GenerateSeed(length uint8)` (`:623-636`): `ErrInvalidSeedLen` unless 16 ≤ length ≤ 64, else that many bytes from Go's `crypto/rand` (not `dcrd/crypto/rand`). The manager never generates seeds; `walletsetup.go:278`, `wallet/wallet.go:5298`, `walletseed/seed.go:29` and `internal/prompt/prompt.go:271` do, always with length 32 unless the caller chooses otherwise.

## 4. Data formats

### 4.1 Bucket tree

All names are ASCII byte strings without terminators. bbolt orders keys bytewise; nested buckets and plain keys share one key space per bucket.

```
(root)
├── "meta"                         unified database metadata (wallet/udb/metadata.go:15)
│     "ver"            → u32 BIG-endian database version
├── "waddrmgr"                     address manager namespace (wallet/udb/migration.go:18)
│   ├── "main"/                    master key params, crypto keys, coin type keys, flags
│   ├── "addr"/                    SHA-256(HASH160) → address row
│   ├── "acct"/                    u32 LE account → account row
│   ├── "acctvars"/                (created by the version 17 upgrade)
│   │     └── <u32 LE account>/    per-account variables
│   ├── "addracctidx"/             SHA-256(HASH160) → u32 LE account
│   │     └── <u32 LE account>/    SHA-256(HASH160) → 0x00
│   ├── "acctnameidx"/             <u32 LE len><name> → u32 LE account
│   ├── "acctididx"/               u32 LE account → <u32 LE len><name>
│   └── "meta"/                    address manager metadata (not the root "meta")
└── "wtxmgr"                       transaction store (other spec)
```

Buckets that exist only transiently or in old databases: `waddrmgr/usedaddrs` (created by `createManagerNS`, deleted by the version 2 upgrade, `wallet/udb/addressdb.go:1655`, `wallet/udb/upgrades.go:395`) and `waddrmgr/sync` (pre-unified databases, deleted by `upgradeToVersion6`, `wallet/udb/addressdb.go:1712-1719`).

There are two different buckets named `meta`: the root one holds only `ver`; `waddrmgr/meta` holds account counters.

### 4.2 Primitive encodings

| Primitive | Encoding | Source |
| --- | --- | --- |
| account number, child index, branch, lengths | u32 little-endian, 4 bytes | `wallet/udb/addressdb.go:269-273` |
| length-prefixed string (`stringToBytes`) | u32 LE byte length, then the raw bytes (no terminator, no encoding validation) | `wallet/udb/addressdb.go:277-287` |
| timestamps | u64 LE Unix seconds | `wallet/udb/addressdb.go:1147`, `:1683-1685` |
| address id | the 20-byte HASH160 of the address (RIPEMD-160 of BLAKE-256), for both P2PKH and P2SH | `wallet/udb/addressmanager.go:968-977` |
| address key | `SHA-256(address id)`, 32 bytes | `wallet/udb/addressdb.go:1343`, `:1400` |
| unified database version | u32 **big-endian** (`byteOrder` is `binary.BigEndian`) | `wallet/udb/metadata.go:21-37`, `wallet/udb/txdb.go:123` |
| extended key | base58 string, section 4.12; always stored encrypted | |

### 4.3 `waddrmgr/main`

| Key | Value | Notes |
| --- | --- | --- |
| `mgrver` | u32 LE, `06 00 00 00` | Written by `createManagerNS` (`wallet/udb/addressdb.go:1679`). Never read on a unified database. Deleted only by `Migrate` (`wallet/udb/migration.go:59`), so it is present in every wallet created by `Initialize` and absent in wallets migrated from the pre-unified format. |
| `mgrcreated` | u64 LE Unix seconds at creation | Written once (`wallet/udb/addressdb.go:1683-1689`), never read. |
| `mpub` | 88 bytes, section 4.4.1 | scrypt parameters of the public master key. Required: missing → `IO` "missing master pubkey params". |
| `mpriv` | 88 bytes, section 4.4.1 | scrypt parameters of the private master key. Absent after `ConvertToWatchingOnly`. Present in wallets made by `createWatchOnly`, where it is derived from the public passphrase. |
| `cpub` | 72 bytes: `snacl(masterKeyPub, cryptoKeyPub[32])` | Required: missing → `IO` "missing encrypted crypto pubkey". |
| `cpriv` | 72 bytes: `snacl(masterKeyPriv, cryptoKeyPriv[32])` | Absent after `ConvertToWatchingOnly`. |
| `ctpub` | `snacl(cryptoKeyPub, xpub string of m/44'/<legacy>')` | Legacy coin type key. Present until the SLIP0044 upgrade. Absent in `createWatchOnly` wallets. |
| `ctpriv` | `snacl(cryptoKeyPriv, xpriv string of m/44'/<legacy>')` | Deleted by the SLIP0044 upgrade and by `ConvertToWatchingOnly`. |
| `ctpub-slip0044` | `snacl(cryptoKeyPub, xpub string of m/44'/<slip0044>')` | Written at creation; kept forever. |
| `ctpriv-slip0044` | `snacl(cryptoKeyPriv, xpriv string of m/44'/<slip0044>')` | Deleted by `ConvertToWatchingOnly`. |
| `watchonly` | 1 byte, `00` or `01` | Any other length → `IO` "bad watching-only flag len %d". Non-zero means true. |
| `slip0044acct0` | account row in the version 1 legacy layout (sections 4.6.1 and 4.6.2), type byte `00`, name `"default"` | The account 0 row for the SLIP0044 coin type, parked until the coin type upgrade consumes and deletes it. No database upgrade ever rewrites it. |
| `seed` | (removed) | Deleted by the version 4 upgrade (`wallet/udb/upgrades.go:453`); never written at this pin. The seed is not stored anywhere. |
| `cscript` | (removed) | Deleted by the version 14 upgrade (`wallet/udb/upgrades.go:1262`). |
| `lastaddrs` | (removed) | Deleted by `upgradeToVersion5` (`wallet/udb/addressdb.go:1697-1707`). |

`snacl(K, P)` is the ciphertext format of section 4.4.2. "xpub string"/"xpriv string" are the ASCII bytes of `ExtendedKey.String()`.

### 4.4 snacl formats

#### 4.4.1 Master key parameters (`mpub`, `mpriv`), 88 bytes

| Offset | Size | Field | Encoding |
| ---: | ---: | --- | --- |
| 0 | 32 | Salt | random bytes |
| 32 | 32 | Digest | SHA-256 of the 32-byte derived key |
| 64 | 8 | N | u64 LE (Go `int` cast to `uint64`) |
| 72 | 8 | R | u64 LE |
| 80 | 8 | P | u64 LE |

`Unmarshal` rejects any length other than 88 with kind `Encoding`, op `secretkey.Unmarshal`, message `bad marshalled data len %d` (`wallet/internal/snacl/snacl.go:164-166`). Values are read back as `int(uint64)`; no range validation happens until scrypt runs.

The derived key is `scrypt(password, Salt, N, R, P, 32)`. Values written by this pin: `N=262144, R=8, P=1` on every network except simnet (`wire.SimNet`), where `N=2, R=1, P=1` (`wallet/udb/addressmanager.go:119-129`). Existing databases may hold other parameters and must be honoured as stored.

#### 4.4.2 `CryptoKey` ciphertext

```
nonce[24] || tag[16] || ciphertext[len(plaintext)]
```

NaCl secretbox (XSalsa20-Poly1305) with a 32-byte key and a fresh random 24-byte nonce per call; the box places the Poly1305 tag before the ciphertext (`wallet/internal/snacl/snacl.go:35-43`, `x/crypto@v0.57.0/nacl/secretbox/secretbox.go:76`). Total length is `len(plaintext) + 40`. No associated data.

`Decrypt`: input shorter than 24 bytes → kind `Invalid`, op `cryptokey.Decrypt`, message `missing nonce`; authentication failure (including a box shorter than 16 bytes) → kind `Crypto`, op `cryptokey.Decrypt`, no message (`wallet/internal/snacl/snacl.go:47-63`).

`SecretKey.Encrypt`/`Decrypt` delegate to the contained key and re-wrap errors with ops `secretkey.Encrypt`/`secretkey.Decrypt`.

### 4.5 Per-account passphrase formats

#### 4.5.1 Argon2id parameters (`acctvars/<acct>/kdf-params`), 25 bytes

| Offset | Size | Field | Encoding |
| ---: | ---: | --- | --- |
| 0 | 16 | Salt | random |
| 16 | 4 | Time | u32 LE |
| 20 | 4 | Memory (KiB) | u32 LE |
| 24 | 1 | Threads | u8 |

`UnmarshalBinary` rejects any other length with the plain error `invalid marshaled Argon2id parameters` (`kdf/kdf.go:56-65`). Key = Argon2id version 0x13, `argon2.IDKey(passphrase, Salt, Time, Memory, Threads, 32)`. Threads is the Argon2 parallelism parameter and changes the output, so the stored value must be used as-is.

#### 4.5.2 XChaCha20-Poly1305 sealed xpriv

```
nonce[24] || ciphertext[len(plaintext)] || tag[16]
```

`chacha20poly1305.NewX(key).Seal(nonce, nonce, plaintext, nil)` with a random nonce and no associated data (`wallet/udb/addressmanager.go:170-183`). The plaintext is the account xpriv string. This blob is stored in the account row's private-key field in place of the snacl ciphertext. Nothing in the row marks which scheme was used: the presence of `kdf-params` in the account's variables bucket decides.

### 4.6 Account rows (`waddrmgr/acct`)

Key: u32 LE account number. Value:

#### 4.6.1 Common envelope

| Offset | Size | Field |
| ---: | ---: | --- |
| 0 | 1 | account type: `00` legacy BIP0044, `01` BIP0044, `02` imported voting |
| 1 | 4 | `rdlen`, u32 LE |
| 5 | `rdlen` | raw data, layout by type |

`deserializeAccountRow` errors with `IO` "bad account len %d" when the value is shorter than 5 bytes. It does not check `rdlen` against the remaining length: a short value panics in Go (slice bounds), and trailing bytes after the raw data are ignored (`wallet/udb/addressdb.go:518-537`).

#### 4.6.2 Legacy BIP0044 raw data (type `00`), by database version

Used for account rows before database version 17, and still used at this pin for `main/slip0044acct0` (always in the version 1 layout).

| Field | v1 | v2–v4 | v5–v16 |
| --- | --- | --- | --- |
| u32 LE `publen`, then encrypted xpub | yes | yes | yes |
| u32 LE `privlen`, then encrypted xpriv | yes | yes | yes |
| u32 LE next external index, u32 LE next internal index | yes | – | – |
| u32 LE last used external, u32 LE last used internal | – | yes | yes |
| u32 LE last returned external, u32 LE last returned internal | – | – | yes |
| u32 LE `namelen`, then name bytes | yes | yes | yes |

Minimum raw length: 20 below version 5, 28 from version 5; shorter → `IO` "bip0044 account %x bad len %d" with the 4-byte account key in hex (`wallet/udb/addressdb.go:555-608`, `:612-658`). Version 1 and versions 2–4 have the same byte layout; only the meaning of the two counters differs.

#### 4.6.3 Current raw data (types `01` and `02`), database version ≥ 17

| Offset | Size | Field |
| ---: | ---: | --- |
| 0 | 4 | `publen`, u32 LE |
| 4 | `publen` | encrypted account xpub string: `snacl(cryptoKeyPub, xpub)` |
| 4+`publen` | 4 | `privlen`, u32 LE |
| 8+`publen` | `privlen` | encrypted account xpriv string: `snacl(cryptoKeyPriv, xpriv)`, or the section 4.5.2 blob when the account has `kdf-params`; empty for imported xpub accounts |

`deserializeRow` requires at least 8 bytes (`IO` "BIP0044 account row bad len %d") and requires the two fields to consume the raw data exactly (`IO` "extra bytes in BIP0044 account row"); out-of-range lengths panic in Go (`wallet/udb/addressdb.go:99-137`).

The reserved imported account (`FF FF FF 7F`) is stored as type `01` with both lengths zero: `01 08 00 00 00 00 00 00 00 00 00 00 00`.

At database version ≥ 17, `fetchDBAccount` rejects a type `00` row in `acct` with the unclassified error "legacy BIP0044 account row unsupported in db version %v" and any other type with `IO` "unknown account type %d" (`wallet/udb/addressdb.go:803-851`).

### 4.7 Account variables (`waddrmgr/acctvars/<u32 LE account>/`)

| Key | Value | Meaning |
| --- | --- | --- |
| `extused` | u32 LE | last used child index on the external branch; `FF FF FF FF` means none |
| `intused` | u32 LE | last used child index on the internal branch |
| `extret` | u32 LE | last returned child index on the external branch; `FF FF FF FF` means none |
| `intret` | u32 LE | last returned child index on the internal branch |
| `name` | raw name bytes, no length prefix | account name |
| `kdf-params` | 25 bytes (section 4.5.1) | present only when the account has its own passphrase |

A u32 variable whose value is not exactly 4 bytes (including missing) yields `IO` with message `bad len %d for uint32 value "%s"` (`wallet/udb/addressdb.go:1074-1085`); see section 10 item 7 for how `fetchDBAccount` then loses that message. `name` is read with no validation; a missing key reads as the empty string.

The variables bucket is created with `CreateBucketIfNotExists` and all five non-optional variables are written whenever an account is created (`putNewBIP0044Account`, `wallet/udb/addressdb.go:956-1006`).

### 4.8 Account indexes

| Bucket | Key | Value |
| --- | --- | --- |
| `acctididx` | u32 LE account | u32 LE name length, name bytes |
| `acctnameidx` | u32 LE name length, name bytes | u32 LE account |

The imported account is indexed in both (`FF FF FF 7F` ↔ `08 00 00 00 "imported"`). The name is stored three times for normal accounts (`acctvars/.../name`, `acctididx`, `acctnameidx`); `RenameAccount` updates all three. `fetchAccountName` does not bounds-check the stored length (`wallet/udb/addressdb.go:745-757`).

### 4.9 `waddrmgr/meta`

| Key | Value | Notes |
| --- | --- | --- |
| `lastaccount` | u32 LE | Highest BIP0044 account number created. Any length other than 4 → `IO` "bad last account len %d". |
| `lastimportedaccount` | u32 LE | Highest imported (xpub or voting) account number. Absent until the first such import; an absent or empty value reads as `0x7FFFFFFF`. Length other than 0 or 4 → `IO` "bad last imported account len %d"; a value ≤ `0x7FFFFFFE` → `IO` "bad imported xpub account value %d". |
| `addrpoolext` + u32 LE account (15 bytes) | u32 LE | Legacy address pool index, written as 0 for account 0 at creation and deleted by the version 2 upgrade for accounts 0..lastaccount. Not present after `Initialize` returns. |
| `addrpoolint` + u32 LE account (15 bytes) | u32 LE | Same, internal branch. |

### 4.10 Address rows (`waddrmgr/addr`)

Key: `SHA-256(HASH160)`, 32 bytes. Value:

| Offset | Size | Field |
| ---: | ---: | --- |
| 0 | 1 | address type: `00` chained (derived), `01` imported key, `02` imported script |
| 1 | 4 | account, u32 LE |
| 5 | 8 | add time, u64 LE Unix seconds when the row was written |
| 13 | 1 | unused (former sync status), written `00`, ignored on read |
| 14 | 4 | `rdlen`, u32 LE |
| 18 | `rdlen` | raw data, by type |

Minimum length 18 (`IO` "bad address len %d"); `rdlen` is not bounds-checked (`wallet/udb/addressdb.go:1111-1152`). Unknown type → `IO` "unknown address type %d".

Raw data:

| Type | Layout | Validation |
| --- | --- | --- |
| `00` chained | u32 LE branch, u32 LE child index (8 bytes) | length must be exactly 8, else `IO` "bad chained address len %d" |
| `01` imported key | u32 LE `publen`, `snacl(cryptoKeyPub, serialized pubkey)`, u32 LE `privlen`, `snacl(cryptoKeyPriv, 32-byte private key)` or empty | minimum 8, else `IO` "bad imported address len %d" |
| `02` imported script | u32 LE `hashlen`, `snacl(cryptoKeyPub, 20-byte script HASH160)`, u32 LE `scriptlen`, the **plaintext** redeem script | minimum 8, else `IO` "bad script address len %d" |

With the sizes this pin writes, `publen` is 73 (33-byte compressed secp256k1 key; 72 when `ImportPrivateKey` is handed an Ed25519 WIF, whose `PubKey()` is the 32-byte Edwards encoding, `dcrd/dcrutil/v4@v4.0.3/wif.go:146-153`, `dcrd/dcrec/edwards/v2@v2.0.4/pubkey.go:17`), `privlen` is 72 or 0, `hashlen` is 60. Since database version 14 the script is stored in the clear (`wallet/udb/upgrades.go:122-134`).

### 4.11 Address-to-account index (`waddrmgr/addracctidx`)

Two kinds of entries share the bucket:

| Location | Key | Value |
| --- | --- | --- |
| `addracctidx` | `SHA-256(HASH160)` (32 bytes) | u32 LE account |
| `addracctidx/<u32 LE account>/` (nested bucket, created on first address) | `SHA-256(HASH160)` | single byte `00` |

Both are written by `putAddress` on every address insert (`wallet/udb/addressdb.go:907-926`, `:1337-1350`). The per-account nested bucket drives `ForEachAccountAddress` and the imported key count.

### 4.12 Extended key strings

The plaintext inside every xpub/xpriv ciphertext is `ExtendedKey.String()`: base58 (Bitcoin alphabet, no version/checksum wrapper beyond the bytes below) of 82 bytes (`dcrd/hdkeychain/v3@v3.1.3/extendedkey.go:441-473`):

| Offset | Size | Field |
| ---: | ---: | --- |
| 0 | 4 | version: `HDPrivateKeyID` for private keys, `HDPublicKeyID` for public |
| 4 | 1 | depth (`depth % 256`) |
| 5 | 4 | parent fingerprint: first 4 bytes of RIPEMD-160(BLAKE-256(parent compressed pubkey)); zero for a master |
| 9 | 4 | child number, u32 big-endian |
| 13 | 32 | chain code |
| 45 | 33 | key data: `00` followed by the private key left-padded with zeros to 32 bytes, or the 33-byte compressed public key |
| 78 | 4 | checksum: first 4 bytes of BLAKE-256(BLAKE-256(bytes 0..78)) |

Version bytes (`dcrd/chaincfg/v3@v3.3.0/mainnetparams.go:541-542`, `testnetparams.go:515-516`, `simnetparams.go:550-551`, `regnetparams.go:537-538`):

| Network | private | public |
| --- | --- | --- |
| mainnet | `02 FD A4 E8` (`dprv`) | `02 FD A9 26` (`dpub`) |
| testnet3 | `04 35 83 97` (`tprv`) | `04 35 87 D1` (`tpub`) |
| simnet | `04 20 B9 03` (`sprv`) | `04 20 BD 3D` (`spub`) |
| regnet | `EA B4 04 48` (`rprv`) | `EA B4 F9 87` (`rpub`) |

A zeroed key stringifies as the literal `zeroed extended key`.

### 4.13 Unified database version (`meta/ver`)

4 bytes, u32 big-endian. `00 00 00 1A` at this pin. Any other length → `IO` "bad udb version len %d" (`wallet/udb/metadata.go:31-37`).

### 4.14 Contents of `waddrmgr` after creation

Because creation writes the version 1 layout and the upgrades then transform it (section 5.3), the final state is not what the creation code literally writes. After `Initialize` returns for a seeded wallet (`E(k, s)` abbreviates `snacl(k, s)`; `cp` and `cv` are the public and private crypto keys):

| Bucket | Key | Value |
| --- | --- | --- |
| `main` | `mgrver` | `06 00 00 00` |
| `main` | `mgrcreated` | u64 LE creation time |
| `main` | `mpub`, `mpriv` | 88 bytes each |
| `main` | `cpub`, `cpriv` | 72 bytes each |
| `main` | `ctpub`, `ctpriv` | `E(cp, legacy coin type xpub)`, `E(cv, legacy coin type xpriv)` |
| `main` | `ctpub-slip0044`, `ctpriv-slip0044` | `E(cp, …)`, `E(cv, …)` |
| `main` | `watchonly` | `00` |
| `main` | `slip0044acct0` | `00` ‖ rdlen ‖ publen ‖ `E(cp, SLIP0044 acct 0 xpub)` ‖ privlen ‖ `E(cv, SLIP0044 acct 0 xpriv)` ‖ `00 00 00 00` ‖ `00 00 00 00` ‖ `07 00 00 00` ‖ `default` |
| `acct` | `00 00 00 00` | `01` ‖ rdlen ‖ publen ‖ `E(cp, legacy acct 0 xpub)` ‖ privlen ‖ `E(cv, legacy acct 0 xpriv)` |
| `acct` | `FF FF FF 7F` | `01 08 00 00 00 00 00 00 00 00 00 00 00` |
| `acctvars/00000000` | `extused`, `intused`, `extret`, `intret` | `FF FF FF FF` each |
| `acctvars/00000000` | `name` | `default` |
| `acctvars/FFFFFF7F` | `extused`, `intused`, `extret`, `intret` | `00 00 00 00` each |
| `acctvars/FFFFFF7F` | `name` | `imported` |
| `acctididx` | `00 00 00 00` | `07 00 00 00 default` |
| `acctididx` | `FF FF FF 7F` | `08 00 00 00 imported` |
| `acctnameidx` | `07 00 00 00 default` | `00 00 00 00` |
| `acctnameidx` | `08 00 00 00 imported` | `FF FF FF 7F` |
| `meta` | `lastaccount` | `00 00 00 00` |
| `addr`, `addracctidx` | (empty) | |

Why the two accounts differ: the version 2 upgrade recomputes last-used indexes only for accounts `0..lastaccount` and finds no address rows, giving `FF FF FF FF`; it leaves the imported account's row untouched, so its version 1 "next index" zeros are reinterpreted as last-used zeros. The version 5 upgrade copies last-used into last-returned for both. The version 17 upgrade moves all of it into `acctvars` and changes the type byte to `01` (`wallet/udb/upgrades.go:247-402`, `:462-521`, `:1364-1417`).

For a wallet made by `InitializeWatchOnly` the differences are: no `ctpub`, `ctpriv`, `ctpub-slip0044`, `ctpriv-slip0044`, `slip0044acct0`; `watchonly` is `01`; `mpriv` holds parameters derived from the public passphrase; the account 0 row's private field is `E(cv, account xpub string)`, a ciphertext of the public key string under the private crypto key (`wallet/udb/addressmanager.go:3163-3171`).

After `Manager.UpgradeToSLIP0044CoinType` (section 5.10), which `dcrwallet --create` runs immediately for a generated seed (`walletsetup.go:230-236`) and which address discovery runs for any other legacy-coin-type wallet on which it finds only account 0 with no used address on either branch, unless coin type upgrades are disabled in the wallet configuration (`wallet/discovery.go:952-975`): `ctpub`, `ctpriv` and `slip0044acct0` are gone, the `acct/00000000` row holds the SLIP0044 keys, and `acctvars/00000000` holds `extused`, `intused`, `extret`, `intret` all equal to `00 00 00 00` (not `FF FF FF FF`) with the previous name.

## 5. Algorithms and state machines

Error notation: `Kind "message"` means `errors.E(errors.Kind, message)`; the rendered string is `<kind text>: <message>` (section 8). "plain" means an error with no kind.

### 5.1 Extended keys (hdkeychain v3.1.3 as used)

An `ExtendedKey` holds: private and public version bytes, `key` (a byte slice: the private key for private keys, the 33-byte compressed public key for public keys), a memoized `pubKey` for private keys, 32-byte chain code, 4-byte parent fingerprint, child number, depth (`uint16` in memory, one byte on the wire), and `isPrivate` (`dcrd/hdkeychain/v3@v3.1.3/extendedkey.go:117-127`).

The private key slice is **not always 32 bytes**. Its length is the mechanism behind Decred's non-BIP32 derivation:

| Constructor | Stored private key length |
| --- | --- |
| `NewMaster` | always 32 (left half of the HMAC, leading zeros kept) |
| `NewKeyFromString` | always 32 (the padded serialization, leading zeros kept) |
| `Child` | 32 minus the number of leading zero bytes of the result |
| `ChildBIP32Std` | always 32 |

**`NewMaster(seed, net)`** (`extendedkey.go:507-534`):

1. If `len(seed) < 16` or `> 64` return `ErrInvalidSeedLen` ("seed length must be between 128 and 512 bits").
2. `I = HMAC-SHA512(key = "Bitcoin seed", data = seed)`. `IL = I[0:32]`, `IR = I[32:64]`.
3. If `IL` as a big-endian integer is zero or ≥ the secp256k1 group order return `ErrUnusableSeed` ("unusable seed").
4. Return a private key with `key = IL`, chain code `IR`, parent fingerprint `00 00 00 00`, depth 0, child number 0.

**`child(i, strictBIP32)`** (`extendedkey.go:218-349`); `Child(i)` passes `false`, `ChildBIP32Std(i)` passes `true`:

1. If the key is public and `i ≥ 0x80000000` return `ErrDeriveHardFromPublic` ("cannot derive a hardened key from a public key").
2. Build a 37-byte zeroed buffer `data`.
   - Hardened (`i ≥ 0x80000000`): copy the stored private key bytes to `data[1:]`. A 32-byte key fills `data[1..33]` (BIP32 layout `00 ‖ key32 ‖ ser32(i)`). A shorter key is **left-aligned**: a 31-byte key gives `00 ‖ key31 ‖ 00 ‖ ser32(i)`.
   - Non-hardened: copy the 33-byte compressed public key to `data[0..33]`.
   - Write `i` big-endian at `data[33..37]`.
3. `I = HMAC-SHA512(key = chain code, data)`; `IL = I[0:32]`, child chain code `= I[32:64]`.
4. If `IL` is zero or ≥ group order return `ErrInvalidChild` ("the extended key at this index is invalid").
5. Private parent: child key `= (IL + parent) mod n` as 32 big-endian bytes; if `strictBIP32` is false, remove all leading `00` bytes. No check that the sum is non-zero.
   Public parent: child key `= compress(IL·G + parentPoint)`. `ErrInvalidChild` if `IL·G` has a zero coordinate; a parent that fails to parse returns the secp256k1 parse error.
6. Parent fingerprint `= RIPEMD-160(BLAKE-256(parent compressed pubkey))[0:4]`; depth `+1`; child number `i`; same version bytes and privacy as the parent.

The strict flag controls only how the **child** is stored; the HMAC preimage layout in step 2 is decided by how the **parent** was stored. Non-hardened derivation never depends on the stored length.

**`NewKeyFromString(s, net)`** (`extendedkey.go:538-615`):

1. If `len(s) > 113` return `ErrInvalidKeyLen` ("the provided serialized extended key length is invalid").
2. Base58-decode (an invalid character makes the decoder return an empty result, `base58@v1.0.6/base58.go:71-76`); if the result is not exactly 82 bytes return `ErrInvalidKeyLen`.
3. Verify the 4-byte double-BLAKE-256 checksum, else `ErrBadChecksum` ("bad extended key checksum").
4. The version must equal the network's private **or** public version, else `ErrWrongNetwork` ("the provided serialized extended key is for the wrong network").
5. The key is private iff byte 45 is `00`, regardless of which version matched. Private: bytes 46..78 must be in `[1, n−1]`, else `ErrUnusableSeed`. Public: bytes 45..78 must parse as a curve point, else the secp256k1 error.
6. The result carries both of the network's version bytes; `String()` picks the one matching `isPrivate`.

**Other methods used**: `Neuter()` returns the same pointer for a public key, otherwise a public key whose key bytes are the private key's memoized public key slice and whose chain code and fingerprint slices are the private key's own (all three are shared, not copied, so `Zero()` on the private key also zeroes the neutered key); `SerializedPubKey()` returns the compressed public key (memoized for private keys); `SerializedPrivKey()` returns the stored private bytes, which can be shorter than 32, or `ErrNotPrivExtKey`; `IsPrivate()`; `String()` (section 4.12); `Zero()` overwrites key, memoized public key, chain code and fingerprint in place and makes `String()` return `zeroed extended key`.

**Call sites.** Every derivation in dcrwallet uses `Child`. `ChildBIP32Std` is never called anywhere in the repository.

| Call site | Parent (how stored) | Index | Preimage layout |
| --- | --- | --- | --- |
| `deriveCoinTypeKey` purpose step (`wallet/udb/addressmanager.go:2678`) | master from `NewMaster` (32 bytes) | `44'` | BIP32 |
| `deriveCoinTypeKey` coin type step (`:2684`) | purpose key from `Child` (stripped) | `coin'` | Decred (left-aligned when the purpose key has leading zero bytes) |
| `deriveAccountKey` from `HDKeysFromSeed` (`:2840`, `:2850`) | coin type key from `Child` (stripped) | `0'` | Decred |
| `deriveAccountKey` from `NewAccount` (`:2135`) | coin type key parsed from the database string (32 bytes) | `account'` | BIP32 |
| account discovery in the wallet layer (`wallet/discovery.go:432`, `:567`) | key returned by `CoinTypePrivKey` (parsed, 32 bytes) | `account'` | BIP32 |
| `deriveKey` (`:461-466`), `checkBranchKeys` (`:2719-2728`), `syncAccountToAddrIndex` (`:2007`, `:2029`), `importAccount` (`:2262-2266`), `AccountBranchExtendedPubKey` (`:660`) | account key (parsed or supplied) | branch, child (< 2^31) | not affected |

So the stripped layout can only influence the two coin type keys and the two account 0 keys computed by `HDKeysFromSeed`; every later account is derived from a zero-padded coin type key. For a seed whose coin type private key begins with a zero byte, account 0 and accounts ≥ 1 are therefore derived under different layouts. This is the behaviour to reproduce.

### 5.2 Key hierarchy and `HDKeysFromSeed`

```
passphrases                     seed (16..64 bytes, never stored)
  public  ──scrypt──► masterKeyPub  ──secretbox──► cryptoKeyPub  (32 random bytes, "cpub")
  private ──scrypt──► masterKeyPriv ──secretbox──► cryptoKeyPriv (32 random bytes, "cpriv")

cryptoKeyPub  seals: coin type xpubs, account xpubs, imported pubkeys, script hashes
cryptoKeyPriv seals: coin type xprivs, account xprivs (default), imported private keys
account passphrase ──Argon2id──► key ──XChaCha20-Poly1305──► account xpriv (unique-passphrase accounts)

m ──44'──► purpose ──coin'──► coin type key ──account'──► account key ──branch──► ──index──► address key
                    coin = LegacyCoinType or SLIP0044CoinType
```

Coin types (`dcrd/chaincfg/v3@v3.3.0`): mainnet legacy 20, SLIP0044 42; testnet3 11 and 1; simnet 115 and 1; regnet 1 and 1.

`HDKeysFromSeed(seed, params)` (`wallet/udb/addressmanager.go:2807-2882`):

1. `root = NewMaster(seed, params)`. Errors (`ErrInvalidSeedLen`, `ErrUnusableSeed`) are returned unwrapped.
2. `ctLegacy = root.Child(44').Child(legacy')`, then `ctSLIP = root.Child(44').Child(slip0044')` (the purpose key is derived twice). A coin type above `0x7FFFFFFF` → `Invalid "coin type %d"`. `ErrInvalidChild` from either step is returned unwrapped.
3. `acctLegacy = ctLegacy.Child(0')`, `acctSLIP = ctSLIP.Child(0')`. `ErrInvalidChild` → `Seed` wrapping `ErrUnusableSeed` ("invalid seed: unusable seed"). Other errors unwrapped.
4. For each account key, derive branch 0 and branch 1 and discard them; `ErrInvalidChild` → `Seed`/`ErrUnusableSeed`.
5. On any error, zero whichever of the four result keys have been derived so far and return four nils. The master key and the two intermediate purpose keys are never zeroed, on success or on failure (`wallet/udb/addressmanager.go:2809-2826`, `:2678-2689`).

The caller owns and must zero the four returned keys.

### 5.3 `Initialize` and `createAddressManager`

`Initialize` (`wallet/udb/initialize.go:18-51`):

1. In one read-write transaction:
   1. Create top-level bucket `waddrmgr`; failure → `IO`.
   2. Create top-level bucket `wtxmgr`; failure → `IO`.
   3. `createAddressManager(waddrmgr, seed, pubPass, privPass, params)`.
   4. `createStore(wtxmgr, params)` (transaction store spec).
   5. Create top-level bucket `meta`; put `ver = 1` (`00 00 00 01`).
2. After that transaction commits, call `Upgrade(ctx, db, pubPass, params)`, which reads the version in a read transaction and runs upgrades 1→26 in one read-write transaction (upgrades spec). It logs `Upgrading database from version 1 to 26` at Info.

If step 2 fails the database is left committed at version 1.

`createAddressManager` (`wallet/udb/addressmanager.go:2895-3082`):

1. If `waddrmgr/main` exists → `Exist "address manager already exists"`.
2. If `privPassphrase` is empty → `Invalid "private passphrase may not be empty"`. The public passphrase is not checked.
3. `createManagerNS` (`wallet/udb/addressdb.go:1633-1692`): create nested buckets `main`, `addr`, `acct`, `addracctidx`, `usedaddrs`, `acctnameidx`, `acctididx`, `meta` (each failure → `IO`); put `meta/lastaccount = 0`; put `main/mgrver = 6`; put `main/mgrcreated = now`.
4. `HDKeysFromSeed`; defer zeroing of the two coin type keys.
5. Neuter the two account keys.
6. `masterKeyPub = NewSecretKey(pubPassphrase, opts)`, then `masterKeyPriv = NewSecretKey(privPassphrase, opts)` with `opts = scryptOptionsForNet(params.Net)`; defer zeroing of `masterKeyPriv`.
7. Generate `cryptoKeyPub`, then `cryptoKeyPriv`; defer zeroing of `cryptoKeyPriv`.
8. Encrypt, in this order: `cryptoKeyPub` under `masterKeyPub`; `cryptoKeyPriv` under `masterKeyPriv`; legacy coin type xpub string under `cryptoKeyPub`; legacy coin type xpriv under `cryptoKeyPriv`; SLIP0044 coin type xpub, xpriv; legacy account 0 xpub, xpriv; SLIP0044 account 0 xpub, xpriv. Failures → `Crypto` with messages `encrypt crypto pubkey: %v`, `encrypt crypto privkey: %v`, `encrypt legacy cointype pubkey: %v`, `encrypt legacy cointype privkey: %v`, `encrypt SLIP0044 cointype pubkey: %v`, `encrypt SLIP0044 cointype privkey: %v`, `encrypt account 0 pubkey: %v`, `encrypt account 0 privkey: %v` (encryption cannot fail in practice).
9. Write: `mpriv`, `mpub`; `cpub`, `cpriv`; `ctpub`, `ctpriv`; `ctpub-slip0044`, `ctpriv-slip0044`; `watchonly = 00`; `meta/addrpoolext‖00000000 = 0`; `meta/addrpoolint‖00000000 = 0`.
10. Write the imported account as a legacy version 1 row with empty keys and name `imported` at `acct/FFFFFF7F`, plus both name indexes.
11. Write account 0 as a legacy version 1 row with the legacy-coin-type keys and name `default` at `acct/00000000`, plus both name indexes.
12. Write the same row shape with the SLIP0044 keys and name `default` to `main/slip0044acct0`.

The upgrades then produce the state in section 4.14.

scrypt is run twice here (step 6), once each in the version 2 and version 14 upgrades (`wallet/udb/upgrades.go:279`, `:1146`), and once more when the wallet is opened.

### 5.4 `InitializeWatchOnly` and `createWatchOnly`

`InitializeWatchOnly` is `Initialize` with `createWatchOnly` in step 1.3.

`createWatchOnly(ns, hdPubKey, pubPassphrase, params)` (`wallet/udb/addressmanager.go:3090-3216`):

1. `main` exists → `Exist "address manager already exists"`.
2. `createManagerNS`.
3. `acctKeyPub = NewKeyFromString(hdPubKey, params)`; errors returned unwrapped. The key is not checked to be public.
4. `checkBranchKeys(acctKeyPub)`; `ErrInvalidChild` → `Seed`/`ErrUnusableSeed`.
5. `masterKeyPub = NewSecretKey(pubPassphrase)` and `masterKeyPriv = NewSecretKey(pubPassphrase)`: both from the public passphrase, with independent salts.
6. Generate both crypto keys and seal them as in 5.3.
7. Account 0 public field `= E(cryptoKeyPub, acctKeyPub.String())`; private field `= E(cryptoKeyPriv, acctKeyPub.String())`.
8. Write `mpriv`, `mpub`, `cpub`, `cpriv`, `watchonly = 01`, the two address pool keys, the imported account row, the account 0 row (legacy version 1 layout). No coin type keys and no `slip0044acct0`.

### 5.5 `Open` and `loadManager`

`Open` (`wallet/udb/open.go:21-54`), in one read transaction:

1. Root bucket `meta` missing → `NotExist "database has not been initialized"`.
2. Read `ver`. Lower than 26 → `Invalid "database upgrade required"`. Higher → `Invalid "database has been upgraded to an unknown newer version"`.
3. `loadManager(waddrmgr, pubPass, params)`.
4. Construct the `Store` with the chain parameters, `acctLookupFunc = manager.AddrAccount`, and the manager.

`loadManager` (`wallet/udb/addressmanager.go:2733-2796`):

1. Read `watchonly`.
2. Read `mpub` (required) and `mpriv` (optional).
3. Read `cpub` (required) and `cpriv` (optional).
4. If not watching-only, unmarshal `mpriv` into `masterKeyPriv` without deriving; failure → `IO "unmarshal master privkey: %v"`. If watching-only, `masterKeyPriv` stays an empty value with no key allocated.
5. Unmarshal `mpub` (failure → `IO "unmarshal master pubkey: %v"`) and derive it from the public passphrase. Any failure, including a scrypt parameter error, is replaced by a bare `Passphrase` error.
6. Decrypt `cpub` with the master public key → `cryptoKeyPub`; failure → `Crypto "decrypt crypto pubkey: %v"`.
7. Create the private passphrase hasher: BLAKE2b-256 keyed with 32 fresh random bytes.
8. Build the manager: `locked = true`, empty account cache, `cryptoKeyPrivEncrypted = cpriv` (possibly nil), `cryptoKeyPriv` = an all-zero key object, `watchingOnly` from step 1.

No account is loaded eagerly. The derived `masterKeyPub` and `cryptoKeyPub` stay in memory until `Close`.

Consequences of the optional private halves:

- A database whose `watchonly` is `00` but which lacks `mpriv` fails to open at step 4: `Unmarshal` receives a nil slice and the error renders `I/O error: unmarshal master privkey: secretkey.Unmarshal: invalid encoding: bad marshalled data len 0` (`wallet/udb/addressmanager.go:2755-2759`, `wallet/internal/snacl/snacl.go:164-166`).
- A database whose `watchonly` is `00` but which lacks `cpriv` opens; the failure is deferred to `Unlock` step 5 (section 5.7), where decrypting the nil ciphertext fails with kind `Crypto` and a message starting `decrypt crypto privkey: `.
- When `watchonly` is `01`, `mpriv` and `cpriv` are read if present but `mpriv` is never unmarshalled and `cpriv` is only held in `cryptoKeyPrivEncrypted`; neither their absence nor malformed contents is an error.
- `SecretKey.Unmarshal` allocates the 32-byte key holder before it checks the length (`wallet/internal/snacl/snacl.go:156-166`). In the watching-only case `Unmarshal` is not called, so `masterKeyPriv.Key` is a nil pointer; `lock()` would dereference it, but every path to `lock()` is closed for a watching-only manager (`Lock`, `Unlock` and private `ChangePassphrase` return `WatchingOnly` first, `Close` skips it).

The public passphrase handed to `Open`, `Initialize` and `InitializeWatchOnly` is chosen by the caller. The wallet layer's default when the user sets none is the 6 ASCII bytes `public` (`wallet.InsecurePubPassphrase`, `wallet/wallet.go:66`; used at `config.go:351`, `walletsetup.go:146`, `:275`, `:338`, `internal/rpc/rpcserver/server.go:1570-1573`, `:2705`); the manager attaches no meaning to that value.

### 5.6 snacl operations

- `NewSecretKey(password, N, r, p)`: random 32-byte salt; `key = scrypt(password, salt, N, r, p, 32)`; `digest = SHA-256(key)`. scrypt parameter errors are wrapped with op `snacl.NewSecretKey` (`wallet/internal/snacl/snacl.go:228-249`).
- `DeriveKey(password)`: recompute the key from the stored parameters; scrypt errors wrapped with op `secretkey.DeriveKey`; then constant-time compare `SHA-256(key)` with the stored digest; mismatch → kind `Passphrase`, op `secretkey.DeriveKey`. The key field holds the wrong key after a mismatch until zeroed.
- scrypt rejects `N ≤ 1` or not a power of two ("scrypt: N must be > 1 and a power of 2"), `r ≤ 0` or `p ≤ 0` ("scrypt: parameters must be > 0"), and oversized products ("scrypt: parameters are too large") (`x/crypto@v0.57.0/scrypt/scrypt.go:195-204`).
- `Zero()` clears the 32 key bytes and keeps the parameters.
- Go forces `debug.FreeOSMemory()` after each scrypt and `runtime.GC()` after each Argon2id derivation; these are memory-pressure workarounds with no observable effect.

### 5.7 Locking, unlocking, closing

Internal `lock()` (`wallet/udb/addressmanager.go:357-377`), write lock held:

1. For **every** cached account, zero and drop `acctKeyPriv`. This includes accounts with their own passphrase: locking the wallet locks them too. Their `uniquePassHash` is left in place.
2. Zero `cryptoKeyPriv` and the derived `masterKeyPriv` key.
3. `locked = true`; `privPassphraseHash = nil`.

`Lock()`: watching-only → `WatchingOnly` (checked without the mutex); already locked → `Locked`; else `lock()`.

`Unlock(ns, passphrase)` (`wallet/udb/addressmanager.go:1518-1587`):

1. Watching-only → `WatchingOnly "cannot unlock watching wallet"`.
2. `passHash = keyedBLAKE2b256(passphrase)` using the manager's hasher (under the hasher mutex).
3. If already unlocked: constant-time compare with `privPassphraseHash`. Equal → return nil, nothing changes. Different → **`lock()`**, then return `Passphrase`.
4. `masterKeyPriv.DeriveKey(passphrase)`; on error `lock()` and return it (kind `Passphrase` with op `secretkey.DeriveKey` for a wrong passphrase).
5. Decrypt `cryptoKeyPrivEncrypted` with the master key into `cryptoKeyPriv`; failure → `lock()`, `Crypto "decrypt crypto privkey: %v"`.
6. For each cached account (Go map order) with a non-empty encrypted key and no unique passphrase: decrypt with `cryptoKeyPriv` (failure → `lock()`, `Crypto "decrypt account %d privkey: %v"`), parse (failure → `lock()`, `IO`), store as `acctKeyPriv`.
7. `locked = false`; `privPassphraseHash = passHash`.

The derived master private key remains in memory while unlocked.

`UnlockedWithPassphrase(passphrase)`: watching-only → `WatchingOnly "watching wallets can not be unlocked"`; locked → `Locked`; hash mismatch → `Passphrase`; else nil. It never changes state.

`Close()`: if not watching-only and unlocked, `lock()`; then zero and drop every cached account xpub, zero `cryptoKeyPub` and the derived `masterKeyPub`; set `closed`. The `closed` flag is never read.

### 5.8 `ChangePassphrase(ns, old, new, private)`

(`wallet/udb/addressmanager.go:1038-1155`), write lock held:

1. `private` and watching-only → `WatchingOnly`.
2. Copy the parameters of the relevant master key into a temporary secret key and `DeriveKey(old)`; errors returned as-is (kind `Passphrase`, op `secretkey.DeriveKey`). The temporary is zeroed on return only when this derivation succeeded (the `defer` is registered after the error check, `wallet/udb/addressmanager.go:1057-1060`).
3. `newMasterKey = NewSecretKey(new, scryptOptionsForNet(net))`: the new key always uses this version's parameters for the network, whatever the old parameters were. The new passphrase is not checked for emptiness here.
4. Private:
   1. New hasher: BLAKE2b-256 keyed with 32 random bytes.
   2. Decrypt `cryptoKeyPrivEncrypted` with the temporary key (`Crypto "decrypt crypto privkey: %v"`), re-encrypt under the new master key (`Crypto "encrypt crypto privkey: %v"`), zero the plaintext.
   3. If locked, zero the new master key's derived key. If unlocked, compute the new passphrase hash with the new hasher.
   4. Put `cpriv`, then put `mpriv`.
   5. Update memory: overwrite `cryptoKeyPrivEncrypted` in place, zero the old master key, install the new master key, hasher and hash (hash is nil when locked).
5. Public:
   1. Encrypt the in-memory `cryptoKeyPub` bytes under the new master key (`Crypto "encrypt crypto pubkey: %v"`).
   2. Put `cpub`, then put `mpub`.
   3. Zero the old public master key and install the new one (its derived key stays in memory).

The crypto keys themselves never change, so no other row is rewritten.

### 5.9 `ConvertToWatchingOnly(ns)`

(`wallet/udb/addressmanager.go:1167-1218`, `wallet/udb/addressdb.go:1483-1589`), write lock held. No-op if already watching-only.

1. `deletePrivateKeys(ns, 26)`:
   1. Delete `main/mpriv`, `main/cpriv`, `main/ctpriv`, `main/ctpriv-slip0044`.
   2. Scan `acct` and collect rows of type `00` only. At database version 26 there are none, so **no account row is rewritten and the encrypted account xprivs stay on disk**.
   3. Scan `addr`, and for every type `01` row rewrite it with the same public field and an empty private field (the row's add time and account are preserved).
2. Put `watchonly = 01`.
3. `lock()` if unlocked.
4. In memory: zero and drop every cached `acctKeyEncrypted`; zero and drop `cryptoKeyPrivEncrypted`; set `cryptoKeyPriv` and `masterKeyPriv` to nil; `watchingOnly = true`.

Only tests call this method (`wallet/addresses_test.go:135`).

### 5.10 Coin type and the SLIP0044 upgrade

`CoinType(tx)`: `main/ctpub` present → legacy coin type; else `main/ctpub-slip0044` present → SLIP0044 coin type; else `WatchingOnly "watching wallets do not record coin type keys"`.

`fetchCoinTypeKeys` applies the same preference and returns the matching pair; missing → `IO "missing encrypted cointype pubkey"` / `IO "missing encrypted cointype privkey"` (`wallet/udb/addressdb.go:367-391`).

`CoinTypePrivKey(tx)` (read lock): locked → `Locked`; watching-only → `WatchingOnly` (checked second, so it is unreachable: a watching-only manager is always locked and gets `Locked`); fetch the pair; decrypt the private half (`Crypto "decrypt cointype privkey: %v"`); parse (`IO`); zero the plaintext string. Returns a freshly parsed (32-byte) key that the caller must zero.

`Manager.UpgradeToSLIP0044CoinType(tx)` (`wallet/udb/addressmanager.go:736-866`):

1. `CoinType`; if it is not the legacy coin type → `Invalid "SLIP0044 coin type upgrade only possible on legacy coin type wallets"`. On regnet both coin types are 1, so this compares equal and the upgrade proceeds.
2. `main/ctpub-slip0044` or `main/ctpriv-slip0044` missing → `Invalid "missing keys for SLIP0044 coin type upgrade"`.
3. `lastAcct = LastAccount`; `props = AccountProperties(lastAcct)`. If `lastAcct != 0`, or the last returned external or internal index is not `0xFFFFFFFF` → `Invalid "wallets with returned addresses may not be upgraded to SLIP0044 coin type"`. Last-used indexes are not consulted.
4. Delete `main/ctpub` and `main/ctpriv`.
5. Read `main/slip0044acct0` (missing → `IO "missing SLIP0044 coin type account row"`); it must be type `00` (`IO "invalid SLIP0044 account 0 row type %d"`); parse its raw data with the **version 1** layout.
6. Build a current-format account: type `01`, the row's two encrypted keys, name = the account's current name from step 3, and the four counters copied from the parsed row's last-used/last-returned fields. Because a version 1 row has no such fields, all four are **0**.
7. `putNewBIP0044Account(ns, 0, …)`: rewrites `acct/00000000`, both name indexes and all five variables (so `extused = intused = extret = intret = 0`).
8. Delete `main/slip0044acct0`.
9. Take the write lock. If account 0 is cached (it is, because step 3 loaded it with the legacy keys): decrypt and parse the new xpub (`Crypto "decrypt SLIP0044 account 0 xpub: %v"`, `IO`); if unlocked also the xpriv (`Crypto "decrypt SLIP0044 account 0 xpriv: %v"`, `IO`); replace the cached encrypted key, xpriv (nil when locked) and xpub. The old cached keys are not zeroed.

`Wallet.UpgradeToSLIP0044CoinType(ctx)` (`wallet/cointype.go:23-61`): in one read-write transaction call the manager method, then fetch account 0's xpub and both branch xpubs; on success replace `addressBuffers[0]` with fresh buffers whose `lastUsed` is `0xFFFFFFFF` and cursor 0. Errors are wrapped with op `wallet.UpgradeToSLIP0044CoinType`. No addresses are registered with the network backend.

The in-memory buffers (`lastUsed = 0xFFFFFFFF`) and the database (`extused = 0`) disagree after this call; section 10 item 1.

### 5.11 Account loading and properties

`loadAccountInfo(ns, account)` (`wallet/udb/addressmanager.go:475-541`):

1. Return the cached entry if present.
2. `fetchDBAccount(ns, account, 26)`: `NotExist "no account %d"` if there is no row.
3. Decrypt the public field with `cryptoKeyPub` (`Crypto "decrypt account %d pubkey: %v"`) and parse it with the manager's network (`IO` wrapping the hdkeychain error).
4. Fill name, type, encrypted private field, xpub and `uniqueKey` (the parsed `kdf-params`, or nil). If `uniqueKey` is set, create a per-account BLAKE2b-256 hasher keyed with 32 random bytes.
5. If the manager is unlocked, the private field is non-empty and there is no unique key: decrypt with `cryptoKeyPriv` (`Crypto "decrypt account %d privkey: %v"`), parse (`IO`), store as `acctKeyPriv`.
6. Insert into the cache.

For the reserved imported account step 3 fails, because its public field is empty: `Crypto "decrypt account 2147483647 pubkey: cryptokey.Decrypt: invalid operation: missing nonce"`.

`AccountProperties(ns, account)` (`wallet/udb/addressmanager.go:550-605`), read lock:

- `account != 0x7FFFFFFF`: `loadAccountInfo`; name and type from the cache; then a second `fetchDBAccount` for the four counters (its error is wrapped in `IO`). `ImportedKeyCount = 0`.
- `account == 0x7FFFFFFF`: name `imported`; `AccountType = 0`; the four counters are left at **0** (the variables bucket is not read); `ImportedKeyCount` = number of entries in `addracctidx/FFFFFF7F` (keys and scripts), each of which must load from `addr` (failure wrapped in `IO`).
- `AccountEncrypted, AccountUnlocked = accountHasPassphrase(ns, account)` (false, false for the imported account because loading fails).

`AccountExtendedPubKey(tx, account)`: imported account → `Invalid "imported account has no extended pubkey"`; otherwise load and return the cached pointer.

`AccountExtendedPrivKey(tx, account)`: imported account → `Invalid "imported address account has no extended privkey"`; load; if `acctKeyPriv` is nil and `account > 0x7FFFFFFF` → `Invalid "imported xpub account has no extended privkey"` (this is also what a locked imported voting account returns); if nil otherwise → `Locked "unable to access account extended privkey"`; else the cached pointer.

`AccountBranchExtendedPubKey(tx, account, branch)`: `AccountExtendedPubKey` then `Child(branch)`; the branch value is not validated.

### 5.12 `NewAccount(ns, name)`

(`wallet/udb/addressmanager.go:2084-2175`), write lock:

1. Watching-only → `WatchingOnly`. Locked → `Locked`.
2. `ValidateAccountName(name)`.
3. Name present in `acctnameidx` → `Exist` with message `account named %q already exists` (Go `%q` quoting).
4. `account = lastaccount + 1`.
5. Fetch and decrypt the active coin type xpriv (`Crypto "decrypt cointype privkey: %v"`), parse it (`IO`), zero the string.
6. `acctKeyPriv = coinTypeKey.Child(account + 2^31)`; `account > 0x7FFFFFFE` → `Invalid "account %d"`; zero the coin type key. `ErrInvalidChild` is returned unwrapped and nothing has been written.
7. Encrypt the xpub string under `cryptoKeyPub` (`Crypto "encrypt account pubkey: %v"`) and the xpriv string under `cryptoKeyPriv` (`Crypto "encrypt account privkey: %v"`).
8. Put `meta/lastaccount = account`.
9. `putNewBIP0044Account` with type `01`, all four counters `0xFFFFFFFF`, and the name.
10. Return the account number. The account is not added to the cache.

### 5.13 `RenameAccount(ns, account, name)`

(`wallet/udb/addressmanager.go:2343-2399`), write lock:

1. `account == 0x7FFFFFFF` → `Invalid "reserved account"`.
2. `name` present in `acctnameidx` → `Exist` `account named %q already exists`. This runs before validation, so renaming to `imported` yields `Exist`, and renaming an account to its current name yields `Exist`.
3. `ValidateAccountName(name)`.
4. `fetchDBAccount` (`NotExist` if the account is unknown).
5. Read the old name from `acctvars/<acct>/name`; put the new name there.
6. Delete `acctididx[account]`; delete `acctnameidx[old name]`; put both indexes for the new name.
7. Update the cached account's name if cached.

Imported xpub and voting accounts can be renamed.

### 5.14 Imported accounts

`ImportXpubAccount(ns, name, xpub)` (`wallet/udb/addressmanager.go:1384-1436`), write lock:

1. `ValidateAccountName`.
2. Name in use → `Exist "account name in use"`.
3. `account = fetchLastImportedAccount + 1` (first import gets `0x80000000`). If the result is `< 0x7FFFFFFE` (wrapped past `0xFFFFFFFF`) → `Invalid "exhausted possible imported accounts"`.
4. Encrypt `xpub.String()` under `cryptoKeyPub` (`Crypto "encrypt account pubkey: %v"`).
5. `putNewBIP0044Account` with type `01`, empty private field, counters `0xFFFFFFFF`, the name.
6. Put `meta/lastimportedaccount = account`.

No lock-state or watching-only requirement, no check that the key is public or for this network, and no duplicate-key check.

`ImportVotingAccount(tx, xpriv, passphrase, name)` (`wallet/udb/addressmanager.go:2181-2318`), write lock:

1. Empty passphrase → `Passphrase "passphrase must be specified"`.
2. `importAccount(tx, importedVoting, xpriv, name)`:
   1. `ValidateAccountName`.
   2. Key not private → `Invalid "extended key must be an xpriv"`.
   3. Name in use → `Exist` `account named %q already exists`.
   4. Derive `xpub/0/0`; failures → `Invalid "undable to derive external branch"` / `Invalid "undable to derive index"` (spelling as in the source). If the HASH160 of that public key is already in `addr` → `Exist "address belonging to this key already exists in the database"`.
   5. `account = fetchLastImportedAccount + 1` (no wraparound check).
   6. Encrypt the xpub under `cryptoKeyPub`, and the xpriv under `cryptoKeyPriv` in whatever state it is in (an all-zero key when the wallet is locked or watching-only).
   7. Put `meta/lastimportedaccount`; `putNewBIP0044Account` with type `02`, counters `0xFFFFFFFF`.
3. `kdfp = NewArgon2idParams`; `key = Argon2id(passphrase, kdfp)`; `ciphertext = seal(key, xpriv.String())`; zero the plaintext.
4. Put `acctvars/<acct>/kdf-params`.
5. Re-read the account, replace its private field with `ciphertext`, rewrite the `acct` row.

All of this happens in the caller's single transaction, so the intermediate row from step 2.7 is never committed. The method works on locked and watching-only wallets. The account is not left unlocked and is not cached.

### 5.15 Per-account passphrases

`accountHasPassphrase(ns, account)`: load the account; on any error return `(false, false)`; otherwise `has = uniqueKey != nil`, `unlocked = has && acctKeyPriv != nil`. `AccountHasPassphrase(tx, account)` is the same under the read lock.

`SetAccountPassphrase(tx, account, passphrase)` (`wallet/udb/addressmanager.go:1690-1789`), write lock:

1. Watching-only and `account < 0x7FFFFFFF` → `WatchingOnly "cannot set passphrase for watching wallet"`. Watching-only and empty passphrase → `Passphrase "watching-only imported accounts must have a passprase"` (spelling as in the source).
2. Load the account.
3. If `acctKeyPriv` is nil: with no unique key → `Locked "wallet must be unlocked to set a unique account passphrase"`; with a unique key → `Locked "account must be unlocked to set a unique account passphrase"`.
4. Empty passphrase → `removeAccountPassphrase` (below).
5. New BLAKE2b-256 hasher keyed with 32 random bytes; `passHash = hasher(passphrase)`.
6. `kdfp = NewArgon2idParams`; `ciphertext = seal(Argon2id(passphrase, kdfp), acctKeyPriv.String())`; zero the plaintext.
7. Put `kdf-params`; re-read the account; set its private field to `ciphertext`; rewrite the `acct` row.
8. Update the cache: encrypted key, `uniqueKey`, hasher, hash. The account stays unlocked.

`removeAccountPassphrase`: watching-only → `WatchingOnly "cannot remove passphrase for watching wallet"`; wallet locked → `Locked "wallet must be unlocked to remove account's unique passphrase"`; encrypt the xpriv string under `cryptoKeyPriv` (`Crypto "encrypt account %d privkey: %v"`); delete `kdf-params`; rewrite the row; clear the cache's unique fields.

`UnlockAccount(tx, account, passphrase)` (`wallet/udb/addressmanager.go:1590-1649`), write lock:

1. Watching-only and `account < 0x7FFFFFFF` → `WatchingOnly "cannot unlock watching wallet"`.
2. Load the account. No unique key → `Crypto "account is not encrypted with a unique passphrase"`.
3. `passHash = accountHasher(passphrase)`.
4. Already unlocked: hash mismatch → `Passphrase` (the account stays unlocked); match → nil.
5. `key = Argon2id(passphrase, uniqueKey)`; `unseal(key, acctKeyEncrypted)`: shorter than 40 bytes → `Crypto "ciphertext too short (len %d) to encode nonce and MAC tag"`; authentication failure → bare `Passphrase`.
6. Parse the xpriv (`IO`); store it and the hash; log `Account %d has been unlocked` at Info.

The wallet does not need to be unlocked.

`LockAccount(tx, account)`: same watching-only check (`"cannot lock watching wallet"`); no unique key → `Crypto "account is not encrypted with a unique passphrase"`; already locked → `Locked "account is already locked"`; zero and drop `acctKeyPriv`; log `Account %d has been locked` at Info.

### 5.16 Address lookup

`normalizeAddress(addr)`: an address that implements `AddressPubKeyHasher` (the three version 0 pay-to-pubkey types) is replaced by its pubkey-hash address; everything else is returned unchanged.

`addressID(addr)`: the 20-byte `Hash160()` of the address; any type without it → `Invalid` `address id cannot be created from type %T (requires Hash160 method)`. The id carries no signature-suite or P2PKH/P2SH distinction: ECDSA, Ed25519 and Schnorr pubkey-hash addresses and script-hash addresses with the same 20 bytes are the same row.

`Address(ns, addr)` (write lock) → `loadAddress`: fetch the row by id (`NotExist "no address %s"` with the address string), then build a managed address:

- Chained row: decide `private = !locked`; if `account > 0x7FFFFFFF` then `private = false`; else if the account has a unique passphrase then `private = accountUnlocked`. Derive `account/branch/index` with `deriveKey` (below), take the compressed public key, zero the derived key if it was private. Result: `managedAddress` with `imported = false`, `internal = (branch == 1)`, `index`.
- Imported key row: decrypt the public field with `cryptoKeyPub` (`Crypto "decrypt imported pubkey: %v"`); `managedAddress` with `imported = true`, `index = 0`, address = P2PKH-ECDSA of `HASH160(stored pubkey bytes)`.
- Script row: decrypt the hash field (`Crypto "decrypt imported P2SH address: %v"`); `scriptAddress` with the stored script.

`deriveKey(acctInfo, branch, index, private)` (`wallet/udb/addressmanager.go:438-468`): start from the xpub, or from the xpriv when `private`. If `private` and the xpriv is absent: unique key present → `Locked "account with unique passphrase is locked"`; encrypted key present → `Locked` whose message is the **account name** (section 10 item 6); neither → plain error `no private key for <name>/<branch>/<index>`. Then `Child(branch).Child(index)`, zeroing the branch key.

`deriveKeyFromPath(ns, account, branch, index, private)`: `private` with the imported account → `Invalid "account does not record private keys"`; load the account; `deriveKey`.

`AddrAccount(ns, addr)`: `id = addressID(normalizeAddress(addr))` (an address without a HASH160 → the `Invalid` error of `addressID`); `addracctidx[SHA-256(id)]` as u32 LE; missing → `NotExist "no address %v"`, formatted with the address **as passed in**, not the normalized one, so a pay-to-pubkey argument prints in its pubkey form here while `Address` prints the pubkey-hash form (`wallet/udb/addressmanager.go:1016-1030`, `:1007-1013`); any other error from the index read passes through.

### 5.17 Private keys and redeem scripts

`PrivateKey(ns, addr)` (`wallet/udb/addressmanager.go:2470-2523`), write lock:

1. Fetch the row by id; `NotExist "no address with id %x"` passes through.
2. Chained row: `deriveKeyFromPath(..., private = true)`; take the stored private bytes; build a secp256k1 private key (left-padding to 32 bytes); zero the source bytes.
3. Imported key row: decrypt the private field with `cryptoKeyPriv` → `Crypto "decrypt imported privkey: %v"` on failure. There is no explicit lock check: on a locked wallet this fails as `Crypto` (the key is zeroed), and on a row with no private key the inner error is "missing nonce".
4. Script row → `Invalid "no private key for P2SH address"`.
5. Return the key and a `done` function that zeroes it.

`HavePrivateKey(ns, addr)` (read lock): an address with no HASH160 → `(false, nil)`; row lookup error → `(false, err)`; chained row → `account < 0x7FFFFFFF` (true for BIP0044 accounts even in a watching-only wallet; false for imported xpub and voting accounts); imported key row → private field non-empty; script row → false.

`RedeemScript(ns, addr)` / `redeemScriptForHash160(ns, hash160)`: script row → the stored script; chained or imported key row → `Invalid "redeem script lookup requires P2SH address"`; lookup errors pass through. No lock is taken and the wallet need not be unlocked.

### 5.18 Imports into the imported account

All three take the write lock and store under account `0x7FFFFFFF`.

`ImportPrivateKey(ns, wif)`:

1. Not watching-only and locked → `Locked`.
2. `pub = wif.PubKey()` (the WIF's stored public key: 33-byte compressed for the two secp256k1 suites, 32 bytes for an Ed25519 WIF); `id = HASH160(pub)`; already in `addr` → `Exist "address for private key already exists"`.
3. Encrypt `pub` under `cryptoKeyPub` (`Crypto "encrypt imported pubkey: %v"`).
4. If not watching-only, encrypt `wif.PrivKey()` under `cryptoKeyPriv` (`Crypto "encrypt imported privkey: %v"`) and then zero the WIF's own private key bytes in place. If watching-only, the private field is empty.
5. `putImportedAddress`; return a `managedAddress` with `imported = true`.

The WIF's signature suite is not examined: whatever `PubKey()` returns is hashed, encrypted and stored, and the returned managed address is always the ECDSA-secp256k1 P2PKH type.

`ImportPublicKey(ns, pubkey)`:

1. Not watching-only → `Invalid "public keys may only be imported by watching-only wallets"`.
2. Length ≠ 33 → `Encoding "invalid length for compressed pubkey"`; first byte not `02`/`03` → `Encoding "invalid format byte for compressed pubkey"`. The point is not validated.
3. Duplicate → `Exist "address for public key already exists"`.
4. Encrypt, `putImportedAddress` with an empty private field, return the managed address.

`ImportScript(ns, script)`:

1. `id = HASH160(script)`; duplicate → `Exist "script already exists"`.
2. Encrypt the 20-byte hash under `cryptoKeyPub` (`Crypto "encrypt script hash: %v"`).
3. `putScriptAddress` with the plaintext script; return a `scriptAddress`.

No lock-state requirement and no script validation.

### 5.19 Address recording, index tracking, iteration

`putAddress(ns, id, row)`: `h = SHA-256(id)`; put `addr[h] = row`; put `addracctidx[h] = account`; create `addracctidx/<account>/` if needed; put `h → 00` there. Every address insert uses the current wall-clock second as add time.

`SyncAccountToAddrIndex(ns, account, syncToIndex, branch)` (`wallet/udb/addressmanager.go:1985-2065`), write lock:

1. Imported account → `Invalid "cannot sync imported account branch index"`.
2. Load the account.
3. Branch not 0 or 1 → `Invalid "account branch %d"`; derive the branch xpub.
4. `syncToIndex > 0x7FFFFFFF` → `Invalid "child index %d exceeds max"`.
5. For `child = syncToIndex` down to 0: derive the child public key (skip on `ErrInvalidChild`); if its HASH160 already has a row, stop; otherwise write a chained row `(account, branch, child)`.

The loop stops at the first existing row, on the assumption that everything below is already recorded. It changes no counters. If child 0 itself is an invalid child, the decrement wraps to `0xFFFFFFFF` and the next derivation fails with `ErrDeriveHardFromPublic`.

`RecordDerivedAddress(tx, account, branch, child, pubkey)` (write lock): write a chained row keyed by `HASH160(pubkey)` unconditionally, overwriting any existing row of any type for that id. No validation of any argument.

`MarkUsedChildIndex(tx, account, branch, child)` (`wallet/udb/addressmanager.go:1896-1943`), no manager lock:

1. Branch 0 selects `extused`/`extret`, branch 1 `intused`/`intret`; otherwise `Invalid "account branch %d"`.
2. Read both variables (failure → `IO`).
3. Using wrapping 32-bit arithmetic: if `child + 1 <= lastUsed + 1` return (nothing to do).
4. Put `lastUsed = child`.
5. If `lastReturned + 1 < child + 1` put `lastReturned = child`.

With the sentinel `0xFFFFFFFF`, `sentinel + 1` wraps to 0, so any child advances it.

`MarkUsed(tx, addr)`: fetch the row by id (errors pass through); if it is a chained row call `MarkUsedChildIndex` with its account, branch and index; otherwise do nothing.

`MarkReturnedChildIndex(tx, account, branch, child)` (`wallet/udb/addressmanager.go:1949-1971`), no manager lock: the variable is `intret` if `branch == 1`, else `extret` (no validation of the branch); read it (the read error is returned as-is); if `child > lastReturned` or `lastReturned == 0xFFFFFFFF` put `child`. It never lowers the value and never touches last-used.

Both marking methods dereference `acctvars/<account>/` without checking that it exists; for an unknown account Go panics with a nil dereference.

`ForEachAccount(ns, fn)`: iterate `acct` in bbolt key order, skipping nested buckets, calling `fn(u32 LE key)`. Key order is bytewise over the little-endian encoding, so it is not numeric order: `0, 2147483648, 256, …, 1, 2147483649, 257, …, 2147483647` (the reserved imported account `FF FF FF 7F` sorts last among realistic values). The reserved imported account is included.

`ForEachAccountAddress(ns, account, fn)` (write lock): iterate `addracctidx/<account>/` in key order (i.e. ordered by `SHA-256(HASH160)`), load each row from `addr` (failure wrapped in `IO`), convert it to a managed address as in 5.16, call `fn`; stop at the first error. A missing nested bucket means no addresses and is not an error.

`ForEachActiveAddress(ns, fn)` (write lock): iterate all of `addr` in key order, convert each row, call `fn(managed.Address())`. Every account referenced by a chained row is loaded into the cache.

### 5.20 Pre-unified manager upgrades

`upgradeManager(ns)` (`wallet/udb/addressdb.go:1723-1783`) is called only by `Migrate` for databases without the root `meta` bucket: read `main/mgrver` (`IO "missing address manager version"` if absent); below 5 → delete `main/lastaddrs`, set 5; below 6 → delete nested bucket `sync` (`IO` on failure, including when it does not exist), set 6; a result other than 6 → `Invalid "incompatible address manager version %d"`. `Migrate` then deletes `main/mgrver` and writes root `meta/ver = 1`. Supporting such databases is a decision for the upgrades spec.

### 5.21 `Manager.Encrypt` / `Manager.Decrypt`

(`wallet/udb/addressmanager.go:2576-2638`), write lock held for the whole call.

1. `selectCryptoKey(keyType)`:
   1. `keyType == CKTPrivate` (0) and the manager is locked **or watching-only** → bare `Locked` (not `WatchingOnly`).
   2. `CKTPrivate` → `cryptoKeyPriv`; `CKTPublic` (2) → `cryptoKeyPub`; any other value, including 1 (the removed `CKTScript`) → `Invalid "crypto key kind %d"`.
2. Call `Encrypt(in)` / `Decrypt(in)` on the selected key (section 4.4.2 format). A failure is wrapped as `errors.E(errors.Crypto, err)`; with the production key type the inner error is the `cryptokey.Decrypt` `*Error`, wrapped directly rather than formatted into a message, so `errors.E` promotes its op to the outer error (`errors/errors.go:214-234`). An authentication failure (inner kind `Crypto`, no message) collapses into one error that renders `cryptokey.Decrypt: encryption/decryption error`. A short input (inner kind `Invalid`) stays nested and renders `cryptokey.Decrypt: encryption/decryption error` + `errors.Separator` + `invalid operation: missing nonce`, i.e. `cryptokey.Decrypt: encryption/decryption error:: invalid operation: missing nonce` in the daemon (section 8).

These two methods have no caller outside `wallet/udb/addressmanager_test.go`.

### 5.22 Database primitive conventions

Rules that hold for every `fetch*`/`put*`/`delete*` primitive in `wallet/udb/addressdb.go` and that the method-level sections rely on:

1. **Namespace argument.** Each primitive takes the `waddrmgr` bucket and opens its nested bucket by name on every call. The nested bucket handle is not nil-checked: on a database that lacks the bucket, Go panics with a nil dereference instead of returning an error. The same holds for `Open` when the root `waddrmgr` bucket is missing while root `meta/ver` is 26 (`wallet/udb/open.go:40-42`, `wallet/udb/addressdb.go:489-492`). The only nil checks are `managerExists` (on `main`), `forEachAccountAddress` (on the per-account index bucket) and `Open` (on root `meta`).
2. **Write failures** from `Put`, `Delete`, `CreateBucket` and `DeleteNestedBucket` are wrapped as kind `IO` with the driver error as the cause. Exceptions: `putNewBIP0044Account` returns the variables-bucket creation error as the driver produced it (`wallet/udb/addressdb.go:972-976`), and `Initialize`/`InitializeWatchOnly` wrap `CreateTopLevelBucket` failures as `IO` themselves (`wallet/udb/initialize.go:20-27`, `:41-44`).
3. **Nil means "do not write".** `putMasterKeyParams`, `putCryptoKeys`, `putCoinTypeLegacyKeys` and `putCoinTypeSLIP0044Keys` skip a nil argument and write the other; `ChangePassphrase` depends on this to rewrite only one half of each pair.
4. **Deleting an absent key succeeds** (`wallet/internal/bdb/db.go:179-185`), so `deletePrivateKeys` works on a database that already lacks `ctpriv`, and `RenameAccount` does not fail when an index entry is missing. Deleting an absent nested bucket fails.
5. **Lookup misses** are kind `NotExist` with these messages: `no account %d` (`fetchAccountName`, `fetchAccountRow`), `no account %q` (`fetchAccountByName`), `no address with hash %x` (`fetchAddressByHash`, seen only wrapped in `IO` from the two iteration helpers), `no address with id %x` (`fetchAddress`; surfaces from `PrivateKey`, `HavePrivateKey`, `RedeemScript`, `redeemScriptForHash160`, `MarkUsed`), `no address for id %x` (`fetchAddrAccount`; always replaced by `AddrAccount` with `no address %v`), `no address %s` (`loadAddress`, with the normalized address string).
6. **Values are copied or decoded before the transaction ends** by `fetchMasterKeyParams`, `fetchCryptoKeys` and every row deserializer (they allocate). `fetchCoinTypeKeys` returns the database's own slices, which are only valid inside the transaction; both callers consume them immediately.
7. **Fixed-width reads without length checks**: `fetchManagerVersion`, `fetchAccountByName`, `fetchAddrAccount` and `forEachAccount` decode a u32 from the value or key without checking it is 4 bytes long (shorter panics in Go; longer uses the first 4 bytes).

## 6. Constants, limits and defaults

| Name | Value | Source | What depends on it |
| --- | --- | --- | --- |
| Hardened offset | `0x80000000` | `dcrd/hdkeychain/v3@v3.1.3/extendedkey.go:36` | account, coin type and purpose child numbers |
| Purpose | 44 | `wallet/udb/addressmanager.go:2678` | BIP0044 path |
| Coin types (legacy / SLIP0044) | mainnet 20 / 42; testnet3 11 / 1; simnet 115 / 1; regnet 1 / 1 | `dcrd/chaincfg/v3@v3.3.0/mainnetparams.go:546-547`, `testnetparams.go:520-521`, `simnetparams.go:555-556`, `regnetparams.go:542-543` | every derived key |
| Seed length | 16..64 bytes; recommended 32 | `dcrd/hdkeychain/v3@v3.1.3/extendedkey.go:30-44` | `NewMaster` |
| HD master HMAC key | `"Bitcoin seed"` | `dcrd/hdkeychain/v3@v3.1.3/extendedkey.go:99` | master key |
| Serialized extended key | 78 bytes + 4 checksum; string length limit 113 | `dcrd/hdkeychain/v3@v3.1.3/extendedkey.go:50`, `:552-553` | parse limits |
| `MaxAccountNum` | `0x7FFFFFFE` | `wallet/udb/addressmanager.go:36` | `NewAccount`, imported-account validation |
| `ImportedAddrAccount` | `0x7FFFFFFF` | `wallet/udb/addressmanager.go:47` | reserved account, first imported account is `0x80000000` |
| `MaxAddressesPerAccount` | `0x7FFFFFFF` | `wallet/udb/addressmanager.go:41` | `SyncAccountToAddrIndex` |
| "No index" sentinel | `0xFFFFFFFF` | `wallet/udb/addressmanager.go:2161-2164` | all four counters |
| scrypt, all networks but simnet | N = 262144, r = 8, p = 1, key 32 bytes | `wallet/udb/addressmanager.go:124-128` | new master keys (about 256 MiB per derivation) |
| scrypt, simnet | N = 2, r = 1, p = 1 | `wallet/udb/addressmanager.go:120-122` | new master keys |
| scrypt salt, digest | 32 random bytes; SHA-256 | `wallet/internal/snacl/snacl.go:83-84`, `:237`, `:246` | `mpub`, `mpriv` |
| secretbox | key 32, nonce 24, tag 16 | `wallet/internal/snacl/snacl.go:22-24` | all snacl ciphertexts |
| Argon2id | version 0x13, time 1, memory 262144 KiB, threads `uint8(min(NumCPU, 256))`, salt 16, key 32 | `kdf/kdf.go:30-39`, `wallet/udb/addressmanager.go:156-158` | new account passphrases |
| XChaCha20-Poly1305 | key 32, nonce 24, tag 16 | `wallet/udb/addressmanager.go:160-164` | account passphrase ciphertexts |
| Passphrase hashers | BLAKE2b-256 keyed with 32 random bytes, in memory only | `wallet/udb/addressmanager.go:2782-2787`, `:511-517` | already-unlocked checks |
| `latestMgrVersion` | 6 | `wallet/udb/addressdb.go:21` | `main/mgrver` |
| `initialVersion` / `DBVersion` | 1 / 26 | `wallet/udb/upgrades.go:31`, `:213` | creation and open |
| `accountVariablesVersion` | 17 | `wallet/udb/upgrades.go:158` | which account row layout `fetchDBAccount` accepts |
| Default public passphrase (wallet layer) | `public` | `wallet/wallet.go:66` | `mpub` derivation when the user configures none |
| Passphrase hasher key | 32 random bytes per hasher | `wallet/udb/addressmanager.go:511-512`, `:1079-1080`, `:1735-1736`, `:2782-2783` | new key at open, at every private passphrase change, at every account load with `kdf-params`, at every `SetAccountPassphrase` |
| Legacy account row minimum raw length | 20 (version < 5), 28 (version ≥ 5) | `wallet/udb/addressdb.go:567-570` | `slip0044acct0`, legacy rows |
| Current account row minimum raw length | 8 | `wallet/udb/addressdb.go:116` | `acct` values at version ≥ 17 |
| Chained address raw length | exactly 8 | `wallet/udb/addressdb.go:1161` | `addr` type `00` |
| Imported / script address minimum raw length | 8 | `wallet/udb/addressdb.go:1199`, `:1250` | `addr` types `01`, `02` |
| Compressed public key import | 33 bytes, first byte `02` or `03` | `wallet/udb/addressmanager.go:1306-1316` | `ImportPublicKey` |
| Default account name | `default` | `wallet/udb/addressmanager.go:62` | account 0 |
| Reserved name | `imported` | `wallet/udb/addressmanager.go:50` | name validation |
| Address pool key length | 15 | `wallet/udb/addressdb.go:215` | legacy `meta` keys |
| Address row header | 18 bytes | `wallet/udb/addressdb.go:1120` | `addr` values |
| Account row header | 5 bytes | `wallet/udb/addressdb.go:526` | `acct` values |

## 7. Concurrency and lifecycle

**No goroutines, channels or contexts.** The manager is passive. `Initialize`, `InitializeWatchOnly` and `Open` accept a context only to pass it to `walletdb.View`/`Update`.

**Locks.**

- `Manager.mtx` (`sync.RWMutex`) guards `locked`, the account cache, the crypto and master keys and `privPassphraseHash`. The lock column of section 3.4 is authoritative.
- `privPassphraseHasherMu` guards the stateful private passphrase hasher. Order is always `mtx` then `privPassphraseHasherMu` (`Unlock`, `UnlockedWithPassphrase`).
- Per-account hashers are used only under the write lock.
- The database write transaction (single writer in bbolt) is taken by the caller **before** any manager lock: the order is database transaction, then `Manager.mtx`.

**Lock anomalies in the Go code** (a Rust port must not copy the races, only the outcomes):

- `AccountProperties` and `AccountHasPassphrase` hold the read lock but call `loadAccountInfo`, which inserts into the cache map (`wallet/udb/addressmanager.go:551-552`, `:568`, `:1849-1858`).
- `Lock` reads `watchingOnly` before taking the mutex; `WatchingOnly()` never takes it.
- `MarkUsed`, `MarkUsedChildIndex`, `MarkReturnedChildIndex`, `AddrAccount`, `ExistsHash160`, `AccountName`, `LookupAccount`, `LastAccount`, `LastImportedAccount`, `ForEachAccount`, `RedeemScript`, `CoinType` take no manager lock; they touch only the database.
- `UpgradeToSLIP0044CoinType` does its database work unlocked and takes the write lock only for the cache update.

**Lifecycle.**

1. `Initialize*` → database at version 26.
2. `Open` → manager locked, public keys in memory, cache empty.
3. Accounts enter the cache on first use and stay until `Close` (there is no eviction). `NewAccount`, `ImportXpubAccount` and `ImportVotingAccount` do not populate it.
4. `Unlock` ↔ `Lock`; per-account `UnlockAccount` ↔ `LockAccount`, plus the implicit account lock on every wallet lock.
5. `Close` zeroes everything. No production code path calls it; process exit is the normal end.

**Cache versus transaction.** Cache and key-state changes are applied inside the caller's transaction closure and are not undone if the transaction later fails to commit. Affected: `ChangePassphrase` (new master key and re-encrypted crypto key installed), `RenameAccount` (cached name), `SetAccountPassphrase`, `UpgradeToSLIP0044CoinType`, `ConvertToWatchingOnly`, and any account loaded from a row that a rolled-back transaction created. The Go wallet does not compensate.

**Returned pointers.** `AccountExtendedPubKey` and `AccountExtendedPrivKey` return the cached key objects, not copies. A caller that zeroes one zeroes the cache entry; `lock()` and `Close` zero keys that callers may still hold.

**Ordering guarantees callers observe.** Iteration orders are bbolt key orders (section 5.19). Account numbers are allocated densely and monotonically from `meta/lastaccount` and `meta/lastimportedaccount`; neither counter ever decreases.

## 8. Errors

Errors are `*errors.Error{Op, Kind, Err}` values. `Error()` renders `Op`, then `Kind` text, then the wrapped error, joined by `": "`; nested `*Error` values are joined by the package variable `errors.Separator`, whose default is `":\n\t"` (`errors/errors.go:27`) but which the dcrwallet binary overrides to `":: "` in an `init` function (`dcrwallet.go:36-39`), so every error string the daemon logs or returns uses `":: "`; repeated ops and kinds are printed once (`errors/errors.go:247-290`). The separator also ends up inside messages that format a nested error with `%v`: a failed crypto key decryption in `Unlock` renders in the daemon as `encryption/decryption error: decrypt crypto privkey: secretkey.Decrypt: encryption/decryption error:: cryptokey.Decrypt`. `errors.E` takes the **last** string or error argument as the message and silently ignores arguments of any other type (`errors/errors.go:187-236`).

Kinds produced in scope and their text (`errors/errors.go:77-124`):

| Kind | Text | Produced by |
| --- | --- | --- |
| `Invalid` | `invalid operation` | reserved names and accounts, bad branch, bad coin type, version mismatch on open, misuse (imported account has no keys, public key import on a non-watching wallet, upgrade preconditions), `cryptokey.Decrypt` short input |
| `IO` | `I/O error` | any bucket put/delete/create failure, malformed or missing database values, extended key parse failures after decryption |
| `Exist` | `item already exists` | duplicate account name, address, key or script; manager already created |
| `NotExist` | `item does not exist` | unknown account, name or address; uninitialized database |
| `Encoding` | `invalid encoding` | master key parameter length; public key import format |
| `Crypto` | `encryption/decryption error` | secretbox failures; account not uniquely encrypted; sealed blob too short |
| `Locked` | `wallet locked` | wallet or account locked |
| `Passphrase` | `invalid passphrase` | wrong public, private or account passphrase; missing passphrase |
| `Seed` | `invalid seed` | unusable seed during account 0 or branch derivation |
| `WatchingOnly` | `watching only wallet` | private operations on watching-only managers |
| (none) | message only | `no private key for %s/%d/%d`; `unknown account type %T`; `legacy BIP0044 account row unsupported in db version %v`; `invalid marshaled Argon2id parameters`; raw hdkeychain and scrypt errors where noted |

Ops attached in scope: `cryptokey.Decrypt`, `secretkey.Unmarshal`, `secretkey.DeriveKey`, `secretkey.Encrypt`, `secretkey.Decrypt`, `snacl.NewSecretKey`, `wallet.UpgradeToSLIP0044CoinType`. Manager methods attach no op; wallet-level callers add theirs. Two further op constants are declared and never attached to any error: `cryptokey.Encrypt` (`wallet/internal/snacl/snacl.go:36`) and `snacl.GenerateCryptoKey` (`wallet/internal/snacl/snacl.go:75`); neither function can fail.

Error constructions present in the source that no input can reach at this pin (a port may omit them, but must not repurpose their kinds):

| Error | Source | Why unreachable |
| --- | --- | --- |
| `Invalid "address type %T"` | `wallet/udb/addressmanager.go:962` | `fetchAddressByHash` returns only the three row types. |
| `Invalid "address row type %T"` | `wallet/udb/addressmanager.go:2519`, `:2571` | Same. |
| `Invalid "hardened account xpub usage is forbidden"` | `wallet/udb/addressmanager.go:620-622` | A loaded account always has a public key. |
| plain `unknown account type %T` | `wallet/udb/addressmanager.go:520`, `:584`, `:1780`, `:1834`, `:2232`, `:2376` | `fetchDBAccount` at version 26 returns only `*dbBIP0044Account`. |
| `Seed` wrapping `ErrUnusableSeed` after `NewKeyFromString` | `wallet/udb/addressmanager.go:3104-3109` | `NewKeyFromString` never returns `ErrInvalidChild`. |
| `IO` from `blake2b.New256` | `wallet/udb/addressmanager.go:513-516`, `:1737-1740` (and unwrapped at `:1081-1084`, `:2784-2787`) | A 32-byte key is always accepted. |
| `Crypto "encrypt …"` from every `Encrypt` call | sections 5.3, 5.4, 5.8, 5.12, 5.14, 5.15, 5.18 | `CryptoKey.Encrypt` always returns nil; reachable only through the test-only failing key. |
| errors from `kdf.NewArgon2idParams` and `seal` | `wallet/udb/addressmanager.go:1745-1755`, `:2196-2206` | The random source does not fail; `seal` returns nil. |

Reachable errors from dependencies that pass through unwrapped (no dcrwallet kind): hdkeychain's `ErrInvalidSeedLen`, `ErrUnusableSeed`, `ErrInvalidChild`, `ErrDeriveHardFromPublic`, `ErrInvalidKeyLen`, `ErrBadChecksum`, `ErrWrongNetwork` (texts in section 5.1) from `HDKeysFromSeed`, `createWatchOnly`, `NewAccount`, `AccountBranchExtendedPubKey`, `SyncAccountToAddrIndex` and `deriveKey`; `ErrNotPrivExtKey` ("unable to create private keys from a public extended key") from `PrivateKey` when a chained row's account "private" key is in fact public, which cannot happen for keys the manager stored itself; stdaddr `ErrInvalidHashLen` from `newScriptAddress` when a script row's decrypted hash is not 20 bytes.

Rendered examples:

| Situation | String |
| --- | --- |
| wrong private passphrase on a locked wallet | `secretkey.DeriveKey: invalid passphrase` |
| wrong passphrase on an unlocked wallet | `invalid passphrase` |
| wrong public passphrase at open | `invalid passphrase` |
| account lookup miss | `item does not exist: no account 5` |
| duplicate account name | `item already exists: account named "foo" already exists` |
| loading the imported account | `encryption/decryption error: decrypt account 2147483647 pubkey: cryptokey.Decrypt: invalid operation: missing nonce` |
| private derivation on a locked BIP0044 account named `default` | `wallet locked: default` |

Exit codes and the mapping of kinds to JSON-RPC and gRPC error codes are owned by the RPC specs; they key on the kind (`errors.Is(err, errors.Locked)` and similar), so kinds must match exactly even where messages are odd.

Log lines produced in scope (subsystem `WLLT`, level Info): `Account %d has been unlocked`, `Account %d has been locked`.

Panics reachable from malformed input (Go behaviour, not error returns): `seal`/`unseal` with a key that is not 32 bytes; Argon2id with stored `Time == 0` or `Threads == 0` (`x/crypto@v0.57.0/argon2/argon2.go:106-111`); row length fields that exceed the value; `MarkUsedChildIndex`/`MarkReturnedChildIndex` on an account with no variables bucket (any unknown account number); `fetchDBAccount`, and therefore every account load, on a type `01`/`02` row whose variables bucket is missing (`wallet/udb/addressdb.go:818-829`); any private-key operation after an in-session `ConvertToWatchingOnly` (nil crypto key).

## 9. Compatibility-critical behaviours

1. **Bucket and key names, nesting and encodings of section 4, byte for byte.** An existing `wallet.db` at version 26 must open in place and a database written by the replacement must open in dcrwallet.
2. **Endianness split.** Everything under `waddrmgr` is little-endian; root `meta/ver` is big-endian.
3. **Address key = `SHA-256(HASH160)`**, with the same key used in `addr`, `addracctidx` and the per-account nested bucket, and a single id space shared by P2PKH (all suites) and P2SH.
4. **Encryption formats.** secretbox blobs as `nonce ‖ tag ‖ ct`; XChaCha20-Poly1305 blobs as `nonce ‖ ct ‖ tag`; no associated data; plaintexts are base58 extended key strings (ASCII), raw 33-byte public keys, raw 32-byte private keys, or raw 20-byte script hashes.
5. **Scheme selection for the account private field** is by presence of `acctvars/<acct>/kdf-params`, not by any marker in the row.
6. **Master key parameters are honoured as stored** (88-byte layout, 64-bit N/R/P). New keys use the per-network values in section 6; `ChangePassphrase` always rewrites with those values.
7. **Argon2id parameters are honoured as stored**, including the thread count. New parameters use time 1, memory 262144 KiB and a thread count derived from the CPU count (see decision in section 13).
8. **Derivation uses hdkeychain `Child` semantics exactly**, including: leading-zero stripping of derived private keys; the left-aligned hardened preimage for stripped parents; no stripping of the master key or of any key parsed from a string. Consequently `m/44'/coin'` and account 0 are derived from in-memory (possibly stripped) parents, while accounts ≥ 1 and discovery derive from a parsed coin type key. A port that uses one layout uniformly will derive different keys for roughly 1 seed in 128 (two strippable hardened parents on the account 0 path) and lose funds.
9. **Both coin type keys and both account 0 rows are created** for seeded wallets; the legacy coin type is active while `main/ctpub` exists; the SLIP0044 upgrade deletes `ctpub`, `ctpriv`, `slip0044acct0` and rewrites account 0.
10. **Counters after the SLIP0044 upgrade are 0, not `0xFFFFFFFF`** (section 5.10 step 6). After a restart the wallet layer therefore treats child 0 of both branches of account 0 as used and returned, and the first address it hands out is child 1. A replacement that writes `0xFFFFFFFF` would hand out child 0 instead and report different account properties over RPC.
11. **Initial counters**: new BIP0044 and imported accounts get `0xFFFFFFFF` ×4; the reserved imported account's variables are 0 ×4; `AccountProperties` for the reserved imported account reports 0 ×4 and type 0 without reading them.
12. **Account numbering**: BIP0044 accounts from 0 via `meta/lastaccount`; imported xpub and voting accounts from `0x80000000` via `meta/lastimportedaccount` (absent means `0x7FFFFFFF`); the reserved imported account is `0x7FFFFFFF` and is present in `acct`, `acctvars`, `acctididx` and `acctnameidx`.
13. **Account type bytes**: 1 for BIP0044 and imported xpub accounts (indistinguishable except by number and empty private field), 2 for imported voting accounts. `AccountProperties.AccountType` exposes this byte.
14. **Name rules**: only the empty string and `imported` are rejected; names are raw bytes compared exactly; three stored copies kept in sync on rename; the default account may be renamed and `default` may be reused.
15. **Counter update rules** of `MarkUsedChildIndex` and `MarkReturnedChildIndex`, including the wrapping comparisons and the fact that marking used also raises last-returned.
16. **`SyncAccountToAddrIndex` stops at the first existing address row** and writes rows downward from the requested index; rows carry the wall-clock second as add time.
17. **Lock semantics**: a wrong passphrase to `Unlock` on an already-unlocked wallet locks it and returns `Passphrase`; a correct one is a no-op; locking the wallet locks every account with its own passphrase; unlocking the wallet does not unlock them; `UnlockAccount` works while the wallet is locked.
18. **Watching-only semantics**: `IsLocked` is always true; `Lock`/`Unlock`/`NewAccount`/private `ChangePassphrase` return `WatchingOnly`; `ImportPublicKey` works only when watching-only; `ImportPrivateKey` on a watching-only wallet stores the public key only; imported voting accounts can be imported, unlocked and locked on watching-only wallets; `CoinType` returns `WatchingOnly` for wallets created from an xpub.
19. **Watching-only creation layout**: `mpriv`/`cpriv` present and derived from the public passphrase; account 0's private field is the encrypted xpub string. dcrwallet tolerates either presence or absence of these on open.
20. **Error kinds** for every exit listed in section 5; RPC error codes derive from them.
21. **Iteration order** of `ForEachAccount`, `ForEachAccountAddress` and `ForEachActiveAddress` (bbolt key order) wherever an RPC response or notification order is built from it without sorting.
22. **`main/mgrver` and `main/mgrcreated` are written at creation** and left in place. They are never read, but a byte-identical creation path needs them and dcrwallet's own `Migrate` logic distinguishes databases by the root `meta` bucket only.
23. **`Open` requires `ver == 26` exactly**, with the two `Invalid` messages of section 5.5.
24. **Imported script rows hold the plaintext script** and the encrypted hash; `RedeemScript` works on a locked wallet.
25. **Address row header is always 18 bytes** with the add time at offset 5 and a zero byte at offset 13, even though neither is read. dcrwallet rejects a shorter row with `IO "bad address len %d"`, and `ConvertToWatchingOnly` re-serializes imported key rows through the same codec.
26. **Seed acceptance.** A seed is accepted only if its length is 16..64 bytes, the master key is valid, and both coin type keys, both account 0 keys and both branches of each account 0 key derive without `ErrInvalidChild`; the error kinds are those of section 5.2. A port that accepts a seed Go rejects (or the reverse) changes which wallets can be restored.
27. **`kdf-params` lifecycle.** The key is written by `SetAccountPassphrase` and `ImportVotingAccount`, deleted by `removeAccountPassphrase`, and never touched by `putNewBIP0044Account` unless the account being written carries parameters. Its presence must always agree with the encryption scheme of the row's private field (item 5), within the same database transaction.
28. **Name index consistency.** Every account present in `acct` at version 26 has a variables bucket with all five mandatory keys and one entry in each of `acctididx` and `acctnameidx`; `RenameAccount` reads the old name from `acctvars/<acct>/name` (not from `acctididx`) to find the `acctnameidx` entry to delete. A database where the three copies disagree is renamed inconsistently by dcrwallet, so the port must never produce one.
29. **Lookup-miss messages** of section 5.22 item 5, including Go `%q` quoting of account names and lower-case hex of the 20-byte id, wherever an RPC handler forwards the error text.

## 10. Quirks

1. **SLIP0044 upgrade zeroes the counters.** `UpgradeToSLIP0044CoinType` parses `slip0044acct0` with the version 1 layout, which has "next index" fields instead of last-used/last-returned, then copies the (unset, zero) last-used and last-returned fields into the new account (`wallet/udb/addressmanager.go:796-812`, `wallet/udb/addressdb.go:586-602`). The database says index 0 is used and returned on both branches; the wallet's in-memory buffers are reset to "nothing used" for the rest of that process (`wallet/cointype.go:52-58`). After a restart `wallet.Open` builds the buffers from the database values (`wallet/wallet.go:5469-5481`), so the next address is child 1 and child 0 is never returned by the wallet (it is still watched and spendable). Must be reproduced; a Go oracle run should confirm the post-restart first address before the port relies on this reading.
2. **Mixed hardened-derivation layouts.** Section 5.1 call-site table. In addition, the wallet's account discovery derives account keys, account 0 included, from the parsed coin type key (`wallet/discovery.go:432`, `:567`); for a seed whose coin type key begins with a zero byte the discovery key for account 0 differs from the stored account 0 key. The discovery spec must reproduce that mismatch rather than "fix" it.
3. **`NewKeyFromString` decides private versus public from the key data byte**, not the version prefix (`dcrd/hdkeychain/v3@v3.1.3/extendedkey.go:576-596`). `createWatchOnly` and `ImportXpubAccount` do not check `IsPrivate`, so a private key passed to them is stored, as a private key string, in the field encrypted with the **public** crypto key (`wallet/udb/addressmanager.go:3103`, `:3163-3164`, `:1409-1410`). `Wallet.ImportXpubAccount` rejects a private key before reaching the manager (op `wallet.ImportXpubAccount`, no kind, message `extended key must be an xpub`, `wallet/wallet.go:3895-3897`). Nothing above `createWatchOnly` does: `wallet.CreateWatchOnly` (`wallet/wallet.go:5316-5323`), `Loader.CreateWatchingOnlyWallet` (`internal/loader/loader.go:108-163`), the gRPC `CreateWatchingOnlyWallet` handler (`internal/rpc/rpcserver/server.go:2757-2766`) and the `--createwatchingonly` prompt (`walletsetup.go:315-356`) pass the string through unparsed, so a watching-only wallet can be created from an xpriv string. Such a wallet's account 0 "public" key then loads as a private extended key.
4. **Reserved imported account inconsistencies.** Its `acct` row is type 1 with empty keys, its variables are zeros, `AccountProperties` reports type 0 and zero counters, `loadAccountInfo` always fails on it with a `Crypto` error, and `accountHasPassphrase` swallows that failure.
5. **`ConvertToWatchingOnly` does not remove account xprivs from disk.** `deletePrivateKeys` only rewrites account rows of type 0, which no longer exist after version 17 (`wallet/udb/addressdb.go:1518-1543`). The ciphertexts become undecryptable once `cpriv` is deleted, except for accounts with their own passphrase, whose xprivs remain recoverable with that passphrase. The legacy rewrite branch is dead code. Test-only entry point.
6. **`Locked` error whose message is the account name.** `errors.E(errors.Locked, "private key %s/%d/%d is locked", acctName, branch, index)` is not a format call; the account name, being the last string argument, replaces the message and the integers are dropped (`wallet/udb/addressmanager.go:449-453`). Rendered: `wallet locked: <account name>`.
7. **Lost error detail in `fetchDBAccount`.** A failure reading account variables returns `errors.E(errors.IO, err)` where `err` is the outer, nil variable, not the reader's error (`wallet/udb/addressdb.go:830-832`). The result is a bare `I/O error`.
8. **Same pattern in `importAccount`**: `errors.E(errors.Invalid, err, "undable to derive …")` discards `err` and keeps the misspelled string (`wallet/udb/addressmanager.go:2262-2269`).
9. **Argon2 thread count wraps.** `uint8(min(runtime.NumCPU(), 256))` is 0 on a machine with 256 or more logical CPUs, and Argon2 panics on 0 threads (`kdf/kdf.go:31-36`). The stored value otherwise varies by the machine that created it.
10. **`RenameAccount` checks existence before validity**, so the reserved name yields `Exist` instead of `Invalid`, and renaming to the current name fails with `Exist` (`wallet/udb/addressmanager.go:2352-2360`).
11. **`MarkReturnedChildIndex` documentation is wrong.** The comment says it writes lower indexes; the code only raises the value or replaces the sentinel (`wallet/udb/addressmanager.go:1945-1968`). It also treats every branch other than 1 as external.
12. **`ImportPrivateKey` zeroes the caller's WIF** private key bytes after encrypting them (`wallet/udb/addressmanager.go:1268-1270`), although `WIF.PrivKey` documents the bytes as read-only (`dcrd/dcrutil/v4@v4.0.3/wif.go:183-187`). The WIF cannot be used afterwards.
13. **`PrivateKey` on an imported key has no lock check.** A locked wallet produces `Crypto`, not `Locked`; a public-only import produces `Crypto` with "missing nonce" (`wallet/udb/addressmanager.go:2505-2509`).
14. **`HavePrivateKey` is true for BIP0044 chained addresses on watching-only wallets** (`wallet/udb/addressmanager.go:2540-2541`).
15. **`AccountExtendedPrivKey` on a locked imported voting account** returns `Invalid "imported xpub account has no extended privkey"` rather than `Locked` (`wallet/udb/addressmanager.go:643-645`). `AccountExtendedPubKey`'s nil check is dead code (`:620-622`).
16. **`SetAccountPassphrase` on an imported xpub account** reports `Locked "wallet must be unlocked …"` although unlocking cannot help (`wallet/udb/addressmanager.go:1716-1727`).
17. **`ImportVotingAccount` encrypts the xpriv under the zeroed private crypto key** when the wallet is locked or watching-only, then overwrites that field in the same transaction (`wallet/udb/addressmanager.go:2290-2294`, `:2225-2227`). It does not check the key's network: a key for another network is stored and the account then fails to load with `ErrWrongNetwork` forever. `importAccount` has no wraparound check on the account number, unlike `ImportXpubAccount`.
18. **Chained addresses are derived privately when unlocked** only to obtain the public key (`wallet/udb/addressmanager.go:890-913`). The result equals the public derivation; the only difference is which errors can occur.
19. **`RecordDerivedAddress` overwrites** any existing row for the same HASH160, including imported keys and scripts, and resets the add time (`wallet/udb/addressmanager.go:2330-2338`).
20. **`SyncAccountToAddrIndex` wraparound** when child 0 is an invalid child (probability about 2^-127): the loop decrements to `0xFFFFFFFF` and returns `ErrDeriveHardFromPublic` (`wallet/udb/addressmanager.go:2028-2053`).
21. **No bounds checks in row parsers.** Length fields larger than the value panic; trailing bytes after the account envelope are ignored (`wallet/udb/addressdb.go:518-537`, `:1111-1133`, `:1190-1268`, `:745-757`).
22. **Dead and unused surface**: `Manager.closed`; `Close` (no production caller); `ForEachActiveAccountAddress` (alias with a TODO); `Manager.Encrypt`/`Decrypt` and `CryptoKeyType`; `ObtainUserInputFunc`; `snacl.DefaultN/R/P`; `managedAddress.multisig`; the `ErrInvalidChild` branch after `NewKeyFromString` in `createWatchOnly` (`wallet/udb/addressmanager.go:3104-3109`); the add-time and former sync-status fields of address rows (written, never read); `mgrcreated`.
23. **Documentation disagreements**: `Initialize` says the database "is initialized with the latest version and does not require any upgrades", but it writes version 1 and upgrades (`wallet/udb/initialize.go:15-17`, `:45-50`); `snacl` comments call the ciphertext a "JSON blob" (`wallet/internal/snacl/snacl.go:207`, `:217`); the address row comment lists a comment field that does not exist (`wallet/udb/addressdb.go:1137-1142`); `ImportPrivateKey` mentions uncompressed keys although `WIF.PubKey` is always compressed (`wallet/udb/addressmanager.go:1226-1228`); `NewAccount` and `RenameAccount` mention `ErrDuplicateAccount`, which does not exist.
24. **hdkeychain edge cases**: depth is 16-bit in memory and serialized modulo 256; a private child whose sum is zero is not rejected and strips to an empty key that stringifies as `zeroed extended key` (`dcrd/hdkeychain/v3@v3.1.3/extendedkey.go:298-311`, `:441-448`).
25. **Go-runtime dependence**: `Unlock` and `ConvertToWatchingOnly` iterate the account cache in map order, which only affects which account's error is reported first; `deletePrivateKeys` iterates Go maps when rewriting rows (final bytes are order-independent); forced GC calls after KDFs.
26. **Regnet has identical legacy and SLIP0044 coin types**, so both key pairs and both account 0 rows hold the same keys under different nonces, and the upgrade's "already upgraded" check cannot distinguish the states (it is stopped only by the returned-address check).
27. **A watching-only wallet from `createWatchOnly` stores a "private" master key derived from the public passphrase** and a private crypto key that encrypts nothing private (`wallet/udb/addressmanager.go:3133`, `:3168-3169`).
28. **`Close` is not idempotent and leaves the cache poisoned.** `zeroSensitivePublicData` sets every cached `acctKeyPub` to nil but keeps the cache entries (`wallet/udb/addressmanager.go:382-392`). A second `Close` calls `Zero()` on the nil key and panics (`dcrd/hdkeychain/v3@v3.1.3/extendedkey.go:488-497` dereferences the receiver), and any later method that uses a cached account dereferences a nil public key. Nothing checks `closed`.
29. **`RecordDerivedAddress` and every other address insert leave stale per-account index entries on overwrite.** `putAddrAccountIndex` adds `addracctidx/<new account>/<h>` and repoints `addracctidx[<h>]`, but never removes `addracctidx/<old account>/<h>` (`wallet/udb/addressdb.go:907-926`). After a HASH160 that belonged to the imported account is re-recorded as a derived address, `ForEachAccountAddress(imported)` still yields it (now as a chained address of the other account) and `AccountProperties(imported).ImportedKeyCount` still counts it.
30. **`fetchAccountInfo` parses type `02` rows with the legacy layout and rejects type `01`** (`wallet/udb/addressdb.go:795-800`). It runs only in the version 2 and version 5 upgrades, where neither type can exist, so the `importedVoting` case is dead.
31. **`fetchLastImportedAccount` fallback is a declared hack.** The source carries `TODO: add this, set to old imported account num in db upgrade` and `TODO: also remove this hack` (`wallet/udb/addressdb.go:728-729`): no upgrade ever writes `lastimportedaccount`, so absence (or an empty value) must keep meaning `0x7FFFFFFF`.
32. **`cryptoKey.Bytes` is documented as returning a copy but returns the key's own storage** (`wallet/udb/addressmanager.go:257-260`). `ChangePassphrase` and creation pass that slice straight to `Encrypt`.
33. **`Unlock` takes the namespace bucket and never uses it** (`wallet/udb/addressmanager.go:1518`). Only accounts already in the cache are decrypted; the rest are decrypted on first load.
34. **Public `ChangePassphrase` on a wallet made by `createWatchOnly` leaves `mpriv` derived from the old public passphrase**, and nothing can change it (private change returns `WatchingOnly`). The value is never derived, so the only effect is on disk. `ChangePassphrase` does not reject a new passphrase equal to the old one, nor an empty new private passphrase (creation does reject an empty private passphrase, `wallet/udb/addressmanager.go:2903-2905`).
35. **`putNewBIP0044Account` returns the variables-bucket creation error without wrapping it as `IO`**, unlike every other write in the file (`wallet/udb/addressdb.go:972-976`); the error keeps whatever kind the driver wrapper assigned (`Invalid` or none, `wallet/internal/bdb/db.go:18-36`).
36. **Outstanding source TODOs**: `AccountProperties` ("can be made more performant by simply returning the new account properties during the change", `wallet/udb/addressmanager.go:546-549`); `ForEachActiveAccountAddress` ("actually return only active addresses", `:2445`); the two in item 31. There are no `XXX`, `BUG`, `FIXME` or deprecation markers in scope.
37. **Managed addresses alias caller memory.** `managedAddress.pubKey` is the slice the caller passed (`ImportPublicKey`'s argument, the WIF's public key) and `scriptAddress.redeemScript` is the caller's script (`wallet/udb/address.go:147-155`, `:230-235`); for chained rows the manager copies the public key first when it came from a private derivation (`wallet/udb/addressmanager.go:906-910`).
38. **`AccountExtendedPubKey` reads the cached key after releasing the manager lock** (`wallet/udb/addressmanager.go:614-623`), and `deriveCoinTypeKey` never zeroes the intermediate purpose key (`:2678-2689`).
39. **Upstream acknowledges quirk 1 in a test comment.** `wallet/discovery_test.go:19-22` disables coin type upgrades with the remark that "the buffers record off-by-ones after the upgrade. will be fixed in a later commit", and the upgrade call below it is commented out.
40. **`hdkeychain.GenerateSeed` reads Go's `crypto/rand`**, while every random value produced inside the manager (salts, nonces, crypto keys, hasher keys, Argon2 salts) comes from `dcrd/crypto/rand` (`dcrd/hdkeychain/v3@v3.1.3/extendedkey.go:15`, `:623-636`). A deterministic test harness has to substitute both.
41. **`selectCryptoKey` reports `Locked` for a watching-only manager** rather than `WatchingOnly`, and its doc comment says it must be called with the read lock while both callers hold the write lock (`wallet/udb/addressmanager.go:2576-2587`, `:2606`, `:2625`).
42. **`AddrAccount` and `Address` format their not-found message from different forms of the address**: `AddrAccount` prints the caller's address, `Address`/`loadAddress` the normalized pubkey-hash address (`wallet/udb/addressmanager.go:991`, `:1008`, `:1024`).
43. **A watching-only manager's `masterKeyPriv` has a nil key pointer** (`Unmarshal` is skipped, `wallet/udb/addressmanager.go:2754-2760`), and after an in-session `ConvertToWatchingOnly` the field itself is nil (`:1213`); `lock()` would panic on either and is unreachable only because of the `WatchingOnly` early returns (section 5.5).
44. **`fetchDBAccount` type-gates on a constant from the upgrades file** (`accountVariablesVersion`, `wallet/udb/upgrades.go:158`), and every Manager call site passes `DBVersion`, so the legacy branch (`deserializeBIP0044AccountRow` through `fetchDBAccount`) is reachable only from the version 17 upgrade, the one other caller, which passes the pre-upgrade version (`wallet/udb/addressdb.go:810-817`, `wallet/udb/upgrades.go:1389`).

## 11. Upstream tests

| File | What it pins | Port? |
| --- | --- | --- |
| `wallet/udb/addressmanager_test.go` (1274 lines) | Full API run against a testnet3 database created from the fixed 32-byte seed and passphrases in `common_test.go`, in create, reopen and watching-only modes: lock/unlock/passphrase errors by kind (`testLocking`); WIF import vector (WIF `PtWUqkS3apLoZUevFtG3Bwt6uyX8LQfYttycGkt2XCzgxquPATQgG` → address `TsSYVKf24LcrxyWHBqj4oBcU542PcjH1iA2`, pubkey, privkey) and P2SH import vector (script `5121…51ae` → `TcsXPUraiDWZoeQBEbw7T7LSgrvD7dar9DA`); `ImportVotingAccount` and `ImportXpubAccount` numbering (`0x80000000`, `+1`, `+2`) and error kinds; passphrase change for both keys including injected KDF failure; `NewAccount`, `LookupAccount`, `RenameAccount`, `ForEachAccount`; `Encrypt`/`Decrypt` with an injected failing key | Port all as integration tests. The two import vectors are deterministic and can be copied. |
| `wallet/udb/internal_test.go` | Test hooks: replaceable `newSecretKey`, failing crypto key | Provide equivalent injection points or drop the two fault-injection cases. |
| `wallet/udb/common_test.go` (97 lines) | Fixtures: seed `b46bc650…641567`, four passphrases (`pubPassphrase`, `privPassphrase`, `pubPassphrase2`, `privPassphrase2`), `hexToBytes`, `createEmptyDB` (runs `Initialize` on testnet3 once, then an explicit second `Upgrade`, which returns nil without opening a write transaction because the version is already 26, `wallet/udb/upgrades.go:1710-1713`) and `cloneDB` (copies that file and calls `Open`) | Copy constants. |
| `wallet/setup_test.go` (55 lines) | `testWallet` helper for the wallet-level tests: creates a bdb file, calls `wallet.Create` with the public passphrase `public` and a fixed private passphrase, then `wallet.Open`. It is the only path by which wallet-package tests reach `udb.Initialize`/`udb.Open` | Port as the equivalent fixture. |
| `wallet/udb/cointype_test.go` | Coin type table for mainnet/testnet3/simnet; coin type upgrade: coin type before/after, coin type key equality, account 0 and account 1 xpubs equal the SLIP0044 derivations, address 0 recorded after `SyncAccountToAddrIndex`, second upgrade refused with `Invalid` | Port. It does **not** check the counters after the upgrade (quirk 1). |
| `wallet/internal/snacl/snacl_test.go` | Round trips only: new key, marshal/unmarshal, derive with right and wrong password, encrypt/decrypt, corrupt ciphertext, zero | Port as round trips; add fixed vectors (below). |
| `wallet/udb/upgrades_test.go` with `wallet/udb/testdata/` `v1.db.gz`, `v2.db.gz`, `v3.db.gz`, `v4.db.gz`, `v5.db.gz`, `v7.db.gz`, `v11.db.gz` (`v24.db.gz` is also present but not in the `TestUpgrades` table) | Upgrades from real old databases; `verifyV2Upgrade` and `verifyV5Upgrade` check last-used and last-returned values per account; `verifyV4Upgrade` checks the seed key is gone | Owned by the upgrades spec; the databases are also the best fixtures for reading legacy rows. |
| `dcrd/hdkeychain/v3@v3.1.3/extendedkey_test.go` | BIP32 vectors 1–3 with Decred prefixes, private/public derivation, errors, `Zero`, and BIP32 vector 4 through `ChildBIP32Std` (which fails under `Child`) | Port; dcr-rs already carries vectors from this file (`dcr-rs:src/hd.rs:11-12`). |
| `kdf/` | No tests | Add vectors. |
| `wallet/addresses_test.go` (307 lines) | Wallet-level, simnet, same 32-byte seed as `common_test.go`: `TestAddresses` pins the first five internal and first five external addresses of account 0 (address string, HASH160, compressed public key; e.g. external child 0 `SsfPTmZmaXGkXfcNGjftRPmoGGCqtNPCHKx`, pubkey `031634ef…bb0938`; internal child 0 `SsrFKd8aX4KHabWSQfbmEaDv3BJCpSH2ySj`) for locked and unlocked wallets and after `ConvertToWatchingOnly` (one of its two callers; the other is `TestManagerWatchingOnly`, `wallet/udb/addressmanager_test.go:1172`); `TestAccountIndexes` pins the in-memory last-used/cursor pairs through sequences of address requests and `MarkUsed` calls | Port. The ten address vectors are deterministic derivation vectors for the legacy simnet coin type (115) and can be copied. The buffer assertions belong to the address-buffer spec but exercise `Address`, `MarkUsed` and `MarkReturnedChildIndex`. |
| `wallet/locking_test.go` (150 lines) | Wallet-level `TestLocking` on the `testWallet` fixture: `testUnlock` (unlock then lock), `testLockOnBadPassphrase` (a wrong passphrase to `Unlock` on an already-unlocked wallet returns kind `Passphrase` **and leaves the wallet locked**, twice in a row: the upstream pin for section 5.7 step 3), and three timeout tests that belong to the wallet-locking spec | Port `testUnlock` and `testLockOnBadPassphrase` against the manager; the timeout cases are owned elsewhere. |
| `wallet/discovery_test.go` (101 lines) | `TestDiscoveryCursorPos`: discovery must not reset the cursor; uses `MarkReturnedChildIndex(0, 0, 9)` and `Address`. Runs with coin type upgrades disabled because of quirk 1 (quirk 39) | Owned by the discovery spec; relevant here as the upstream acknowledgement of the post-upgrade counter problem. |
| `dcrd/hdkeychain/v3@v3.1.3/bench_test.go`, `example_test.go` | Benchmarks and documentation examples only | Not needed. |

Test functions by file, for traceability: `wallet/udb/addressmanager_test.go` — `TestManagerImports` (`testImportPrivateKey`, `testImportScript`), `TestImportVotingAccount`, `TestImportAccount`, `TestChangePassphrase` (`testChangePassphrase`), `TestManagerEncryptDecrypt` (`testEncryptDecrypt`, `testEncryptDecryptErrors`), `TestManagerWatchingOnly` and `TestManager` (both run `testManagerAPI`: `testLocking`, `testNewAccount`, `testLookupAccount`, `testRenameAccount`, `testForEachAccount`), `TestMain`. `wallet/udb/cointype_test.go` — `TestCoinTypes`, `TestCoinTypeUpgrade`. `wallet/internal/snacl/snacl_test.go` — `TestNewSecretKey`, `TestMarshalSecretKey`, `TestUnmarshalSecretKey`, `TestUnmarshalSecretKeyInvalid`, `TestEncrypt`, `TestDecrypt`, `TestDecryptCorrupt`, `TestZero`, `TestDeriveKey`, `TestDeriveKeyInvalid`; these share package-level state and depend on running in file order.

Test hooks in `wallet/udb/internal_test.go` (80 lines): `TstLatestMgrVersion` (pointer to `latestMgrVersion`; no test uses it at this pin), `TstRunWithReplacedNewSecretKey` (replaces `newSecretKey` with a function returning a bare `Crypto` error), `(*Manager).TstCheckPublicPassphrase` (re-derives the public master key from a candidate passphrase), `failingCryptoKey` with `TstRunWithFailingCryptoKeyPriv` (swaps `cryptoKeyPriv` for a key whose `Encrypt`/`Decrypt` return the plain errors `failed to encrypt` / `failed to decrypt`), `TstDefaultAccountName`.

The `wallet/udb/testdata/` directory also holds the generator programs for the fixture databases (`v1.db.go`, `v2.db.go`, `v3.db.go`, `v4.db.go`, `v5.db.go`, `v7.go`, `v11.db.go`, `v24.db.go`); they document how each old database was produced and are not compiled into the tests.

**Untested upstream, needing a Go oracle shim:**

1. Byte-exact database contents after `Initialize`, `InitializeWatchOnly` and the SLIP0044 upgrade (section 4.14). Generate with a deterministic RNG substituted for `dcrd/crypto/rand` and dump every bucket.
2. Cross-implementation decryption: databases written by Go (mainnet scrypt parameters, per-account Argon2id with several thread counts) opened by the port, and the reverse.
3. Seeds chosen so that the purpose key, the coin type key, or both begin with a zero byte, comparing coin type keys, account 0 and account 1 (via `NewAccount`) between Go and the port.
4. Counter transitions of `MarkUsedChildIndex`/`MarkReturnedChildIndex` from the sentinel and across wraparound boundaries.
5. Error kind and message for every exit in section 5.
6. Per-account passphrase state machine (set, change, remove, unlock, lock, wallet lock interaction), which upstream covers only through `ImportVotingAccount`.
7. `ConvertToWatchingOnly` on-disk result.

## 12. Rust porting notes

**Module boundary.** One crate for the unified database's address manager (working name `purse-addrmgr`, or a module of the crate that owns `udb`), with four layers:

| Layer | Contents | Notes |
| --- | --- | --- |
| `snacl` | scrypt secret key, secretbox crypto key, 88-byte parameter codec | pure, no database |
| `kdf` | Argon2id parameters codec, XChaCha20-Poly1305 seal/unseal | pure |
| `rows` | every codec and bucket primitive of section 4, taking an explicit database version where the Go does | used by the upgrades too |
| `manager` | `Manager`, cache, lock state, all methods | holds a `RwLock`; database handle supplied per call |

`Initialize`/`Open` belong with the database crate that also owns the transaction store and the upgrades, since they span all three.

**Reuse.**

- **dcr-rs `hd`** (`dcr-rs:src/hd.rs`) already implements the Decred derivation with explicit provenance tracking: a private `zeros_stripped` flag set by `derive_child`, cleared for masters and parsed keys, and consulted when building the hardened preimage (`dcr-rs:src/hd.rs:153-175`, `:309-336`, `:526-531`). That is exactly the model section 5.1 requires. Differences to resolve before using it as the manager's key type: it rejects a private key under a public prefix and detects the network from the prefix instead of validating against an expected one (`dcr-rs:src/hd.rs:482-518`), it rejects a zero child sum, it limits depth to 255, and it is built on the `secp256k1` C bindings (`dcr-rs:Cargo.toml`) while dcroxide uses `k256` (`dcroxide:crates/dcroxide-dcrec/Cargo.toml:28`).
- **dcroxide-chaincfg**: `hd_private_key_id`, `hd_public_key_id`, `slip0044_coin_type`, `legacy_coin_type` (`dcroxide:crates/dcroxide-chaincfg/src/lib.rs:188-191`).
- **dcroxide-txscript `stdaddr`**: the `Address` enum with the pubkey, pubkey-hash and script-hash variants and `hash160` (`dcroxide:crates/dcroxide-txscript/src/stdaddr.rs:144`, `:153`, `:664`), for `normalizeAddress`/`addressID` and managed address construction.
- **dcroxide-crypto**: BLAKE-256, RIPEMD-160 and the CSPRNG (`dcroxide:crates/dcroxide-crypto/src/`).
- **dcroxide-base58** `encode`/`decode` (`dcroxide:crates/dcroxide-base58/src/lib.rs:38`, `:129`).
- **dcroxide-dcrec** secp256k1 `PrivateKey`/`PublicKey` (`dcroxide:crates/dcroxide-dcrec/src/secp256k1.rs:213-371`). Its `PrivateKey::from_bytes` takes exactly 32 bytes, so short stripped keys must be left-padded by the caller.
- dcroxide has no WIF type, no scrypt, no Argon2, no secretbox and no BLAKE2b. New third-party dependencies are needed for scrypt, Argon2id, XSalsa20-Poly1305 and BLAKE2b (or any keyed hash; see below); `chacha20poly1305` is already used by `dcroxide:crates/dcroxide-tlscrypto/Cargo.toml:15` and provides XChaCha20-Poly1305.

**Hazards.**

1. Model the extended private key's "stored length" explicitly. A 32-byte array type erases the one fact the hardened preimage depends on.
2. Do not normalise the creation path. Create at version 1 and run the same upgrade sequence, or write the section 4.14 state directly and prove equality against Go dumps; either way the zeros and the surviving `mgrver` must come out identical.
3. The passphrase hashers are in-memory only; any keyed PRF with a per-process random key and constant-time comparison is equivalent. BLAKE2b is not required for compatibility.
4. Argon2 lanes: the stored thread count is the lane count and must be passed to the KDF even if the port runs it on one thread. Check that the chosen crate applies the same memory rounding as the reference (multiple of `4 × lanes`, minimum `8 × lanes`).
5. scrypt at N = 2^18 allocates about 256 MiB; run it off the async executor and never concurrently with itself unless memory allows.
6. Zeroization: Go zeroes selectively and aliases key buffers between a private key and its neutered form. Use `Zeroize`-on-drop types and return owned copies from the cache rather than shared references, so a caller cannot zero the cache.
7. Replace the Go panics (section 8) with errors, and decide the kind (`IO` is the closest match for malformed rows).
8. Take the write lock in `AccountProperties`/`AccountHasPassphrase` or load the account under an upgradable lock.
9. Wrapping arithmetic in the counter comparisons must be explicit (`wrapping_add`).
10. `errors.E` argument semantics produce the odd messages of quirks 6–8; reproduce the resulting strings, not the call shape.

**Size estimate: L.** About 5,000 lines of Go with wide but shallow logic. The codecs and bucket primitives are mechanical (M). The cost is in the number of state-dependent branches across 48 exported methods, the three account kinds times two encryption schemes times three lock states, and the byte-exact creation and upgrade artefacts that need Go-oracle differential tests. The cryptography is all off-the-shelf.

## 13. Open questions

1. **Confirm quirk 1 against a running dcrwallet**: create a wallet with a generated seed, restart, request the first external address, and check it is `m/44'/42'/0'/0/1`. The reading follows from the code but is not pinned by any upstream test. Settled by a Go oracle run.
2. **Decision: Argon2id thread count for new parameters.** Go uses the CPU count of the creating machine (and breaks at ≥ 256 CPUs). Options: mirror it capped at 255, or fix a constant. Any stored value is compatible; only determinism and cost differ.
3. **Decision: which extended-key implementation** backs the manager: dcr-rs `hd` adapted to dcrd's parse rules, or a new implementation over dcroxide-dcrec. Affects the curve backend used for all wallet derivation.
4. **Decision: creation strategy** (replay version 1 plus upgrades, or write the final state directly). Tied to whether the port supports databases older than version 26 at all (upgrades spec) and pre-unified databases (section 5.20).
5. **Decision: cache/transaction consistency.** Go leaves the cache ahead of the database when a transaction fails after a manager mutation. Reproducing that is not observable to well-behaved clients; the port should probably stage cache changes until commit, which needs an explicit choice.
6. **Decision: `ConvertToWatchingOnly`.** It is test-only upstream and leaves ciphertexts behind. Port as-is for test parity, fix, or omit.
7. **Decision: private extended key passed to watching-only creation** (quirk 3). `Wallet.ImportXpubAccount` rejects private keys, but no caller of `createWatchOnly` does. Reproduce (store the xpriv string under the public crypto key) or reject; rejecting changes the result of `--createwatchingonly` and gRPC `CreateWatchingOnlyWallet` for such input.
8. **UNVERIFIED: historical account rows.** Whether real version-26 databases can contain uncompressed imported public keys (from wallets that imported before `WIF.PubKey` was always compressed) was not checked; the row format allows any length and the managed address hashes whatever is stored. Settled by inspecting old releases or the `testdata` databases.
9. **Decision: behaviour for inputs that panic in Go** (section 8). Returning an error is strictly safer; the error kind should be agreed project-wide.
