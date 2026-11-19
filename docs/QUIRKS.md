# Quirks ledger

Behaviours of dcrwallet at the parity pin (`54b85b33`) that a faithful port has to reproduce or consciously diverge from: defects, surprising behaviour, places where documentation and code disagree, and behaviour that depends on the Go runtime. Collected from the quirks sections of the [specifications](specs/README.md) during the deep dive of 2026-11-14.

**Status of every entry: candidate.** Each was found by reading the source, not by running it. Before code depends on one, an oracle run confirms it; then the entry gets a disposition and a pinning test.

Dispositions follow the standing policy in the [project brief](dcr-purse-project-brief.md) (§3 and decision D10): reproduce when an external observer could depend on it; return an error where Go panics; pick a deterministic order where Go's is random; fix what would lose funds or leak secrets, and record the divergence in [PARITY.md](PARITY.md). dcr-purse keeps its own database format (project owner, 2026-11-14) and reads no `wallet.db`: migration from dcrwallet is by seed restore only, with no import (2026-11-18, ADR-0013). Entries that concern only the bytes dcrwallet writes into `wallet.db` therefore bind neither dcr-purse's store nor any surface of the wallet. They still describe what Go persists, which the logical export must cover and the tests read: the Go oracle reads those databases with dcrwallet's own code, and Rust tests with `purse-boltread`, a test utility only, not shipped in the wallet, which the owner may later remove (ADR-0013 Open point 2); QK-0730 to QK-0732 concern it.

Entry format: number, title, the spec that documents it in full, a description, and the source citation.

## QK-0001 — Byte-identical files are impossible even between two Go runs

Spec: [01](specs/01-walletdb-bbolt.md). Source: bbolt@v1.3.12/bucket.go:550, bucket.go:656-661

Dirty sibling buckets are spilled and nodes rebalanced in Go map iteration order, so page id assignment is non-deterministic in bbolt itself. The port should match all deterministic policies (split, inline, rebalance, first-fit allocation, growth) and pick a fixed order (ascending bucket name) where Go uses map order.

## QK-0002 — nil versus empty values are distinguishable and callers test for nil

Spec: [01](specs/01-walletdb-bbolt.md). Source: bbolt@v1.3.12/page.go:138-142; wallet/udb/upgrades.go:550,570,645,683,739,787,1511,1604

A zero-length value read from a committed page is a non-nil empty slice; nested buckets and end-of-iteration give nil; a value Put as nil is returned as nil by Get in the same write transaction but as non-nil empty after commit. Eight upgrade loops terminate on v != nil. The Rust API should return Some(empty) consistently and wallet/udb must audit nil tests.

## QK-0003 — KeyN counts committed pages, recursively, and ignores the current transaction's changes

Spec: [01](specs/01-walletdb-bbolt.md). Source: wallet/internal/bdb/db.go:190-192; bbolt@v1.3.12/bucket.go:420-499; callers wallet/udb/txmined.go:2353,3073

KeyN is Bucket.Stats().KeyN: leaf element counts from pages (not in-memory nodes), plus KeyN of every nested bucket. Inside a write transaction it does not see uncommitted puts/deletes; for a bucket with a sub-bucket created in the same transaction it walks page 0 and returns 0.

## QK-0004 — Cursor behaviour under mutation depends on whether the leaf was already materialised

Spec: [01](specs/01-walletdb-bbolt.md). Source: bbolt@v1.3.12/cursor.go:212-244,387-406; bucket.go:727-749; wallet/walletdb/interface_test.go:99-120,202-205

A cursor on a leaf not yet turned into an in-memory node keeps reading the page snapshot (mutations invisible); on a materialised leaf it reads the live node, so a delete at/before its index makes Next skip an element and an insert makes it repeat one. The cursor-delete conformance test is disabled for this reason (boltdb issue 620). dcrwallet has no live Cursor.Delete caller but iterate-and-mutate code inherits the semantics.

## QK-0005 — walletdb.View never returns a rollback error (dead code)

Spec: [01](specs/01-walletdb-bbolt.md). Source: wallet/walletdb/interface.go:203-223

The deferred rollback assigns to a local err that is always nil and the function has no named result; View returns exactly what the closure returned. Update does return the commit error.

## QK-0006 — walletdb documentation describes a removed API

Spec: [01](specs/01-walletdb-bbolt.md). Source: wallet/walletdb/doc.go; wallet/walletdb/example_test.go:7; bbolt@v1.3.12/tx.go:143-149

doc.go describes Namespace, Tx.RootBucket and panics on manual commit inside managed transactions; none exists. View/Update use ordinary transactions; a manual Commit/Rollback inside the closure makes the helper's own call fail with 'tx closed' (kind Invalid). example_test.go uses the removed API and is disabled by a build tag.

## QK-0007 — Open can create and Create can open

Spec: [01](specs/01-walletdb-bbolt.md). Source: wallet/internal/bdb/db.go:306-323; bbolt@v1.3.12/db.go:234,261-270

Both drivers call bolt.Open with O_CREATE; Create on an existing database opens it; Open creates the file if it vanishes after the pre-check and initialises an existing zero-length file as an empty database; the pre-check treats every stat error except not-exist as 'exists'.

## QK-0008 — DeleteNestedBucket with an empty key returns Invalid on an empty bucket and NotExist otherwise

Spec: [01](specs/01-walletdb-bbolt.md). Source: bbolt@v1.3.12/bucket.go:222-231; wallet/walletdb/interface_test.go:647

The seek on an empty bucket yields a nil key that compares equal to the empty name, falling through to ErrIncompatibleValue; on a non-empty bucket it is ErrBucketNotFound. The conformance test asserts Invalid on an empty bucket.

## QK-0009 — A second opener blocks silently forever, also within one process

Spec: [01](specs/01-walletdb-bbolt.md). Source: bbolt@v1.3.12/bolt_unix.go:16-45, db.go:44; internal/loader/loader.go:298-302; dcrwallet.go:206-225

No lock timeout is set, so bolt.Open polls flock every 50 ms indefinitely with no log output; the loader releases its mutex around OpenDB for this reason and the daemon runs the open in a goroutine so a signal can abandon it. gRPC OpenWallet simply does not return while another process holds the file.

## QK-0010 — Recorded file size is 0 after opening an existing file

Spec: [01](specs/01-walletdb-bbolt.md). Source: bbolt@v1.3.12/db.go:149,638,1144-1181

db.filesz is set only by init and grow, so the first growing commit of each session re-runs ftruncate (normally to the current size) plus a full fsync; file length follows mmap-size doubling up to 16 MiB then +16 MiB steps.

## QK-0011 — Empty commits still write

Spec: [01](specs/01-walletdb-bbolt.md). Source: bbolt@v1.3.12/tx.go:173-185,222-226

A write transaction with no changes still frees and rewrites the freelist page, writes a meta page and increments txid.

## QK-0012 — Rebalance merges unconditionally despite its comment

Spec: [01](specs/01-walletdb-bbolt.md). Source: bbolt@v1.3.12/node.go:462-507

The comment says nodes merge only if both are too small; the code always merges an under-threshold node with its sibling (next sibling if first child, else previous) and relies on the later split.

## QK-0013 — Put keeps the caller's value slice by reference until commit

Spec: [01](specs/01-walletdb-bbolt.md). Source: bbolt@v1.3.12/bucket.go:277-313

bbolt copies the key but not the value; a Go caller that reuses the buffer before commit changes what is written. A port should copy; wallet/udb port should check for buffer reuse.

## QK-0014 — Failed wallet creation unlinks a still-open database

Spec: [01](specs/01-walletdb-bbolt.md). Source: internal/loader/loader.go:146-150,236-240; walletsetup.go:356-363

The loader's deferred os.Remove runs without closing the handle: fd and lock leak until exit; on Windows the remove fails silently and a partial wallet.db remains. createWatchingOnlyWallet also removes before its deferred close.

## QK-0015 — CreateNewWallet omits MixSplitLimit from wallet.Config

Spec: [01](specs/01-walletdb-bbolt.md). Source: internal/loader/loader.go:259-274 vs 169-185 and 318-334

The other two loader paths set MixSplitLimit; CreateNewWallet does not, so a wallet used right after creation has a zero split limit until reopened.

## QK-0016 — Inconsistent loader error texts and kinds; stale loader doc comments

Spec: [01](specs/01-walletdb-bbolt.md). Source: internal/loader/loader.go:115,130,140,205,220,230,284-288,371-393

'wallet already opened' vs 'wallet already loaded'; 'wallet DB exists' vs 'wallet already exists'; not-a-directory has kind Invalid only in the watching-only path. UnloadWallet's comment says it stops the wallet and names LoadExistingWallet; it only closes the DB. OpenExistingWallet's comment mentions a non-existent canConsolePrompt parameter.

## QK-0017 — Close blocks while any transaction is open; daemon skips Close after a panic

Spec: [01](specs/01-walletdb-bbolt.md). Source: bbolt@v1.3.12/db.go:646-657; dcrwallet.go:176-189

bbolt Close takes the writer, meta and mmap locks, so it waits for all transactions; dcrwallet deliberately does not unload the wallet when unwinding from a panic to avoid a deadlock.

## QK-0018 — Error classification by identity leaves wrapped bbolt errors unclassified

Spec: [01](specs/01-walletdb-bbolt.md). Source: wallet/internal/bdb/db.go:18-36; bbolt@v1.3.12/bucket.go:235-240

convertErr uses switch err, so 'delete bucket: <err>' from a nested DeleteBucket failure, OS errors, 'version mismatch' and 'checksum error' get no kind; only ErrInvalid maps to IO.

## QK-0019 — End-of-range cursor moves leave the cursor in place

Spec: [01](specs/01-walletdb-bbolt.md). Source: bbolt@v1.3.12/cursor.go:117-133,225-229,257-265

After Next returns nil the cursor is still on the last element; after Prev returns nil it is on the first; after Seek past the end, Prev returns the last element.

## QK-0020 — Read handles are concretely read-write types

Spec: [01](specs/01-walletdb-bbolt.md). Source: wallet/internal/bdb/db.go:45-47,109-111,194-196

ReadBucket/ReadCursor return the read-write implementation; a Go type assertion to ReadWriteBucket succeeds and writes fail with 'tx not writable' (Invalid). Rust should not reproduce the escape hatch.

## QK-0021 — No bucket-name length limit and CreateTopLevelBucket errors on existing bucket

Spec: [01](specs/01-walletdb-bbolt.md). Source: bbolt@v1.3.12/bucket.go:153-163; wallet/walletdb/interface.go:40-42; wallet/internal/bdb/db.go:57-63

bbolt's comment promises an error for long bucket names but only empty names are rejected; walletdb's CreateTopLevelBucket comment reads as create-if-missing but returns kind Exist.

## QK-0022 — bbolt panics rather than erroring on corrupt files

Spec: [01](specs/01-walletdb-bbolt.md). Source: bbolt@v1.3.12/page.go:56-64, freelist.go:153-182,273-276, node.go:178, db.go:1106-1108

Wrong page id/flags, a bad freelist flag, zero-length keys, double frees and invalid metas after open all panic the Go process. The Rust reader must bounds-check and decide which become kind-IO errors.

## QK-0023 — DeleteBucket can leak nested buckets' pages

Spec: [01](specs/01-walletdb-bbolt.md). Source: bbolt@v1.3.12/bucket.go:234-254, cursor.go:212-244, node.go:144-158

If the leaf holding nested-bucket entries of the bucket being deleted was already materialised as a node in the same write transaction, the ForEachBucket cursor skips every second nested bucket after each in-place removal; skipped buckets' pages are never freed and become unreachable but unlisted. File stays valid. Derived by reading, not executed.

## QK-0024 — --createtemp refusals exit 0

Spec: [01](specs/01-walletdb-bbolt.md). Source: config.go:575-589

Missing explicit --appdata or missing --simnet with --createtemp prints to stderr and calls os.Exit(0), unlike other creation failures which return a config error.

## QK-0025 — Upstream TODOs marking real limits

Spec: [01](specs/01-walletdb-bbolt.md). Source: bbolt@v1.3.12/db.go:390, db.go:1210, db.go:1371, cursor.go:304, cursor.go:327

Meta-1 page-size probe tops out at 16 MiB pages; meta.write does not reject pgidNoFreelist when freelist sync is on; freepages would proceed after reported corruption were it not for the panic; branch search 'exact' flag relies on unique sorted keys.

## QK-0026 — Tx.Copy deprecated and walletdb Copy uncalled

Spec: [01](specs/01-walletdb-bbolt.md). Source: bbolt@v1.3.12/tx.go:335-342, wallet/internal/bdb/db.go:292-296, wallet/udb/testdata/v24.db.go:76-89

bdb's Copy uses the upstream-deprecated Tx.Copy; no dcrwallet code calls DB.Copy (testdata generators io.Copy the closed file).

## QK-0027 — SLIP0044 coin type upgrade writes counters 0 instead of 0xFFFFFFFF

Spec: [02](specs/02-address-manager.md). Source: wallet/udb/addressmanager.go:796-812; wallet/udb/addressdb.go:586-602; wallet/cointype.go:52-58; wallet/wallet.go:5469-5481

UpgradeToSLIP0044CoinType parses main/slip0044acct0 with the version-1 layout (which has next-index fields, not last-used/last-returned) and copies the unset zero fields into the new account 0, so extused/intused/extret/intret become 0. In the same process the wallet resets its in-memory buffers to 'nothing used', but after a restart it loads 0/0 and the next address is child 1; child 0 is never handed out. Affects every wallet created with a generated seed and every restored wallet upgraded by discovery. Must be reproduced; confirm with a Go oracle run.

## QK-0028 — Two hardened-derivation layouts inside one wallet

Spec: [02](specs/02-address-manager.md). Source: dcrd/hdkeychain/v3@v3.1.3/extendedkey.go:243-256,304-311,586-603; wallet/udb/addressmanager.go:2127-2135,2678-2705,2840-2850; wallet/discovery.go:432,567

hdkeychain Child strips leading zero bytes from a derived private key and a stripped parent is left-aligned in the hardened HMAC preimage. HDKeysFromSeed derives coin type keys and account 0 from in-memory (possibly stripped) parents, but NewAccount and account discovery derive from the coin type key re-parsed from its base58 string, which is always zero-padded (BIP32 layout). For a seed whose coin type key starts with 0x00, account 0 and accounts >= 1 use different layouts, and discovery's account-0 key differs from the stored one.

## QK-0029 — Creation writes version 1 and upgrades, leaving artefacts

Spec: [02](specs/02-address-manager.md). Source: wallet/udb/initialize.go:15-51; wallet/udb/upgrades.go:247-402,462-521,1364-1417; wallet/udb/addressdb.go:1633-1692

Initialize creates the version-1 layout and runs 25 upgrades. Results: the reserved imported account's counters are 0 while account 0's are 0xFFFFFFFF (the v2 upgrade only recomputes accounts 0..lastaccount), main/mgrver=6 and main/mgrcreated remain forever, slip0044acct0 stays in the v1 legacy format, usedaddrs bucket and addrpool keys are created then deleted. The doc comment claims no upgrades are needed.

## QK-0030 — Locked error message is the account name

Spec: [02](specs/02-address-manager.md). Source: wallet/udb/addressmanager.go:449-453,2262-2269; errors/errors.go:187-236

errors.E(errors.Locked, "private key %s/%d/%d is locked", acctName, branch, index) is not a format call: the last string argument (the account name) replaces the message and the integers are ignored. Rendered as `wallet locked: <account name>`. Same pattern drops the underlying error in importAccount ("undable to derive external branch").

## QK-0031 — fetchDBAccount loses account-variable read errors

Spec: [02](specs/02-address-manager.md). Source: wallet/udb/addressdb.go:823-832

When reading acctvars fails, the code returns errors.E(errors.IO, err) using the outer nil err instead of the reader's error, yielding a bare `I/O error` with no detail.

## QK-0032 — ConvertToWatchingOnly leaves account xprivs on disk

Spec: [02](specs/02-address-manager.md). Source: wallet/udb/addressdb.go:1483-1589; wallet/udb/addressmanager.go:1167-1218; wallet/addresses_test.go:135

deletePrivateKeys only rewrites account rows of legacy type 0, which do not exist after DB version 17, so encrypted account xprivs stay in the acct bucket. They are undecryptable once cpriv is deleted, except accounts with their own passphrase, which remain recoverable. Imported-key private fields are removed. Only tests call this method.

## QK-0033 — Private/public decided by key data, not version prefix; no IsPrivate checks

Spec: [02](specs/02-address-manager.md). Source: dcrd/hdkeychain/v3@v3.1.3/extendedkey.go:576-596; wallet/udb/addressmanager.go:3103,3163-3164,1409-1410

NewKeyFromString treats a key as private iff the key-data byte is 0x00 and accepts either version prefix. createWatchOnly and ImportXpubAccount never check IsPrivate, so a private key string would be stored encrypted under the PUBLIC crypto key. Whether upper layers reject this is unverified.

## QK-0034 — Argon2 thread count wraps to 0 at 256+ CPUs and Go panics on bad stored params

Spec: [02](specs/02-address-manager.md). Source: kdf/kdf.go:30-39; x/crypto@v0.57.0/argon2/argon2.go:106-111

NewArgon2idParams uses uint8(min(NumCPU, 256)), which is 0 on machines with >= 256 logical CPUs; argon2 panics on 0 threads or 0 time. The stored thread count otherwise depends on the creating machine and changes the KDF output, so it must be honoured.

## QK-0035 — Wrong passphrase to Unlock locks an unlocked wallet; wallet lock locks unique-passphrase accounts

Spec: [02](specs/02-address-manager.md). Source: wallet/udb/addressmanager.go:357-377,1533-1542,1561-1568

Unlock on an already-unlocked manager compares a keyed BLAKE2b hash; mismatch calls lock() before returning Passphrase. lock() zeroes acctKeyPriv for every cached account including those with their own passphrase, and Unlock never restores those.

## QK-0036 — Reserved imported account is inconsistent

Spec: [02](specs/02-address-manager.md). Source: wallet/udb/addressmanager.go:494-499,567-602,1857-1861

Its acct row is type 1 with empty keys, its acctvars counters are 0, AccountProperties reports type 0 and zero indexes without reading the DB, loadAccountInfo always fails on it with Crypto `missing nonce`, and accountHasPassphrase swallows that failure.

## QK-0037 — Watching-only wallets store a 'private' master key from the public passphrase

Spec: [02](specs/02-address-manager.md). Source: wallet/udb/addressmanager.go:3129-3171,2540-2541

createWatchOnly derives masterKeyPriv from the public passphrase, writes mpriv and cpriv, and stores account 0's private field as the xpub string encrypted under the private crypto key. HavePrivateKey still returns true for BIP0044 chained addresses on watching-only wallets.

## QK-0038 — RenameAccount checks existence before validity

Spec: [02](specs/02-address-manager.md). Source: wallet/udb/addressmanager.go:2352-2360

Renaming to the reserved name `imported` returns Exist (it is in the name index) rather than Invalid, and renaming an account to its current name fails with Exist.

## QK-0039 — MarkReturnedChildIndex doc disagrees with code; marking on unknown account panics

Spec: [02](specs/02-address-manager.md). Source: wallet/udb/addressmanager.go:1911-1917,1945-1968

The comment says lower indexes are written; the code only raises the value or replaces the sentinel, and treats any branch other than 1 as external. MarkUsedChildIndex/MarkReturnedChildIndex dereference acctvars/<acct> without a nil check, so an unknown account panics in Go.

## QK-0040 — ImportPrivateKey zeroes the caller's WIF; PrivateKey on imported key has no lock check

Spec: [02](specs/02-address-manager.md). Source: wallet/udb/addressmanager.go:1268-1270,2505-2509; dcrd/dcrutil/v4@v4.0.3/wif.go:183-187

After encrypting, ImportPrivateKey zeroes the slice returned by WIF.PrivKey (documented read-only), destroying the WIF. PrivateKey for an imported-key row decrypts with the zeroed crypto key when locked and returns kind Crypto, not Locked.

## QK-0041 — ImportVotingAccount oddities

Spec: [02](specs/02-address-manager.md). Source: wallet/udb/addressmanager.go:2181-2318,643-645

Encrypts the xpriv under the zeroed private crypto key when the wallet is locked/watching-only and overwrites it in the same transaction; does not check the key's network (a foreign-network key is stored and the account then never loads); importAccount has no account-number wraparound check, unlike ImportXpubAccount. A locked voting account's AccountExtendedPrivKey returns Invalid `imported xpub account has no extended privkey` instead of Locked.

## QK-0042 — Row parsers have no bounds checks

Spec: [02](specs/02-address-manager.md). Source: wallet/udb/addressdb.go:518-537,745-757,1111-1133,1190-1268

Account and address row deserializers trust embedded length fields; oversized lengths panic in Go (slice bounds) and trailing bytes after the account envelope are ignored. A port must return errors and pick a kind.

## QK-0043 — Data races and non-transactional cache

Spec: [02](specs/02-address-manager.md). Source: wallet/udb/addressmanager.go:551-568,1453-1469,1122-1128,609-651

AccountProperties and AccountHasPassphrase hold only the read lock while inserting into the account cache; Lock reads watchingOnly without the mutex. Cache/key-state mutations (ChangePassphrase, RenameAccount, SetAccountPassphrase, coin type upgrade) are applied inside the caller's DB transaction and are not undone if the commit fails. Cached key pointers are returned to callers, not copies.

## QK-0044 — RecordDerivedAddress overwrites and SyncAccountToAddrIndex can wrap

Spec: [02](specs/02-address-manager.md). Source: wallet/udb/addressmanager.go:2330-2338,2028-2053

RecordDerivedAddress writes a chained row unconditionally, replacing any existing row (including imported keys/scripts) for that HASH160. SyncAccountToAddrIndex's loop, if child 0 is an invalid child, decrements to 0xFFFFFFFF and fails with ErrDeriveHardFromPublic.

## QK-0045 — ForEachAccount order is not numeric

Spec: [02](specs/02-address-manager.md). Source: wallet/udb/addressdb.go:698-708

Accounts are iterated in bbolt bytewise order of their little-endian keys: 0, 0x80000000, 256, ..., 1, 0x80000001, ..., with the reserved imported account (FF FF FF 7F) near the end. Any RPC that lists accounts without sorting exposes this order.

## QK-0046 — Regnet legacy and SLIP0044 coin types are both 1

Spec: [02](specs/02-address-manager.md). Source: dcrd/chaincfg/v3@v3.3.0/regnetparams.go:542-543; wallet/udb/addressmanager.go:737-768

Both key pairs and both account-0 rows hold identical keys under different nonces; the coin type upgrade's 'already upgraded' test cannot distinguish the states and a second upgrade is stopped only by the returned-address check (counters are 0 after the first).

## QK-0047 — vsphostindex is declared as a wtxmgr root key but is not one

Spec: [03](specs/03-txstore-formats.md). Source: wallet/udb/txdb.go:162; wallet/udb/vsp.go:53-65; wallet/udb/upgrades.go:1590

The variable rootVSPHostIndex sits in the wtxmgr root-key block but is only ever read/written inside the top-level `vsphost` bucket as a be32 counter. Implement it there.

## QK-0048 — Unchecked slicing panics on inconsistent rows

Spec: [03](specs/03-txstore-formats.md). Source: wallet/udb/txunmined.go:55-56,359; wallet/udb/txmined.go:2681; wallet/udb/txdb.go:1392-1394,1711-1713

extractRawUnminedTx(nil) panics; it is reached when an `mi` row names a tx missing from `m` (InsertMemPoolTx vote/revocation path, Store.UnspentTickets) or an `m` value is shorter than 8 bytes (PruneUnmined). Many extract*/fetchMultisigOut* helpers and the header field extractors also have no length checks. removeDoubleSpends/RemoveUnconfirmed report the same dangling reference as IO 'tx record len 0' instead.

## QK-0049 — Unmined credit iterator leaves OpCode and IsCoinbase unset

Spec: [03](specs/03-txstore-formats.md). Source: wallet/udb/txdb.go:1619-1636

unminedCreditIterator.readElem sets Index, Amount, Change, HasExpiry only. CreditRecord.OpCode is 0 (not OP_NOP10 0xb9) and IsCoinbase false for every unmined credit regardless of the stored flags; mined credits report the expanded opcode.

## QK-0050 — readRawBlockRecord prints the key length for a short value

Spec: [03](specs/03-txstore-formats.md). Source: wallet/udb/txdb.go:312-314

The 'block record len %d' error raised for a too-short value formats len(k) instead of len(v).

## QK-0051 — fetchMultisigOut error text is unformatted

Spec: [03](specs/03-txstore-formats.md). Source: wallet/udb/txdb.go:1809-1814; errors/errors.go:187-211

errors.E(errors.IO, "multisig output key len %d", len(k)) does not format; the message is the literal string containing %d and the length argument is dropped. Same for 'multisig output len %d'.

## QK-0052 — spendCredit writes a row even when the credit does not exist

Spec: [03](specs/03-txstore-formats.md). Source: wallet/udb/txdb.go:942-954

No existence check: for a missing key it stores a 94-byte value with amount 0, flags 0x01 and the spender key, and returns amount 0. Only reachable with a broken unspent index.

## QK-0053 — unspendRawCredit leaves the stale spender key and widens legacy values

Spec: [03](specs/03-txstore-formats.md). Source: wallet/udb/txdb.go:942-974

Only bit 0 of the flags is cleared; bytes [9:81] keep the previous debit key. Both spendCredit and unspendRawCredit copy the old value into a fresh 94-byte buffer, permanently zero-extending short legacy rows. Must be reproduced for byte-identical rows.

## QK-0054 — Multisig spend/unspend clobbers the stake-tree flag

Spec: [03](specs/03-txstore-formats.md). Source: wallet/udb/txdb.go:1896-1910

setMultisigOutSpent assigns v[22]=0x01 and setMultisigOutUnSpent assigns v[22]=0x00, discarding bit 1 (stake tree). After either, the output reads back as regular tree.

## QK-0055 — setMultisigOutUnSpent zeroes only 31 of 32 spender-hash bytes

Spec: [03](specs/03-txstore-formats.md). Source: wallet/udb/txdb.go:1904-1910

copy(v[67:98], empty[:]) leaves byte 98 (last byte of the previous spender hash) untouched; the spender index is set to 0xFFFFFFFF.

## QK-0056 — tix rows are never deleted and their value is unused

Spec: [03](specs/03-txstore-formats.md). Source: wallet/udb/txdb.go:1715-1746; wallet/udb/txunmined.go:118-128; wallet/udb/upgrades_test.go:279

No delete helper exists for the tickets bucket; pruned or abandoned unmined tickets keep their row (OwnTicket stays true). The 4-byte picked-height value is always -1 and is read only by a test.

## QK-0057 — Dead duplicate-edge check and random order in dependencySort

Spec: [03](specs/03-txstore-formats.md). Source: wallet/udb/kahnsort.go:23-47,75-95

makeGraph's 'skip duplicate edges' loop compares recorded child hashes against the parent hash and never matches, so one edge is added per input; the sort stays correct because in-degrees are over-counted consistently. Order among independent transactions follows Go map iteration and differs per call.

## QK-0058 — InsertMemPoolTx uses two notions of tx type and ignores Unpublished on re-insert

Spec: [03](specs/03-txstore-formats.md). Source: wallet/udb/txunmined.go:31-34,53,101-106

The vote/revocation conflict exception uses caller-supplied rec.TxType; the stakebase skip and tix insert use a freshly computed stake.DetermineTxType. Re-inserting an existing unmined tx returns nil without touching the `up` marker.

## QK-0059 — PruneUnmined under-reports removals and its errors are swallowed by the caller

Spec: [03](specs/03-txstore-formats.md). Source: wallet/udb/txunmined.go:400-409; wallet/chainntfns.go:255-259

The returned list holds only directly selected hashes, not descendants removed recursively, so no removal notification is sent for those. On a mid-way error earlier removals stay in the transaction, and the wallet logs the error and still commits. A tx both selected and removed as a descendant is processed twice.

## QK-0060 — RemoveUnconfirmed trusts its arguments

Spec: [03](specs/03-txstore-formats.md). Source: wallet/udb/txunmined.go:181-280; wallet/udb/txmined.go:1661-1686

It never checks the tx is in `m`, deletes `mi` rows for all inputs without comparing the recorded spender (including a vote's never-written stakebase key), and for votes/revocations writes cmu=00 for every tracked commitment, re-creating a row a mined spender had deleted. It never touches `tix`.

## QK-0061 — ImportCFiltersV2 is permissive and returns unwrapped errors

Spec: [03](specs/03-txstore-formats.md). Source: wallet/udb/cfilters.go:34-67; wallet/udb/txdb.go:1988-1990

No header-commitment check, `havecfilters` is not updated, block-iterator decode errors surface as NotExist 'block height of %d unknown', and gcs parse errors and the cf Put error are returned without a wallet error kind.

## QK-0062 — Stack trace printed to stderr on short tx-record key

Spec: [03](specs/03-txstore-formats.md). Source: wallet/udb/txdb.go:578-604

readRawTxRecordHash, readRawTxRecordBlockHeight and readRawTxRecordBlock call debug.PrintStack() before returning IO 'tx record key len %d'.

## QK-0063 — Comments disagree with code

Spec: [03](specs/03-txstore-formats.md). Source: wallet/udb/txdb.go:717-721,1435-1440,915-916,2168-2169; wallet/udb/upgrades.go:419-429

Credit comment calls [81:86] a 5-byte optional script location but the value is always 94 bytes with fields to offset 94; unmined-credit comment omits the OP_TGEN tag; fetchRawCreditScriptLength doc says it returns the offset; createStore claims to create 'the latest db version' but creates the v1 layout (with obsolete `sc`, without tix/cf/cmt/cmu/up); votingPreferencesUpgrade comment names ticketsagendaprefs but creates agendaprefs.

## QK-0064 — Zero Go time is stored as a negative second count

Spec: [03](specs/03-txstore-formats.md). Source: wallet/udb/txdb.go:524; wallet/udb/txmined.go:361; wallet/udb/upgrades.go:1680-1685

Times are be64(uint64(t.Unix())) in whole seconds; time.Time{} becomes ff ff ff f1 88 6e 09 00. The v26 upgrade stores this in `birthstate` and tests insert records with a zero received time.

## QK-0065 — Dead dbVersion branches in credit codecs

Spec: [03](specs/03-txstore-formats.md). Source: wallet/udb/txdb.go:782-789,895-904,1471-1478

valueUnspentCredit, valueUnminedCredit and fetchRawCreditHasExpiry switch on dbVersion (bit 4 for v8, bit 6 for >=9, nothing below 8) but every call site passes DBVersion=26.

## QK-0066 — Has-expiry bit is lost when an unmined credit is mined (cross-reference)

Spec: [03](specs/03-txstore-formats.md). Source: wallet/udb/txmined.go:1133-1181

moveMinedTx rebuilds the mined credit from a struct whose hasExpiry is never assigned, so bit 0x40 is clear in the resulting `c` row even if the `mc` row had it. Code is in txmined.go but it determines persisted credit bytes.

## QK-0067 — Inconsistent error wrapping

Spec: [03](specs/03-txstore-formats.md). Source: wallet/udb/txdb.go:455-458,1988-1990; wallet/udb/agendas.go:64-80

deleteBlockRecord and putRawCFilter return the driver error as is, while sibling helpers wrap in IO; exported agenda setters wrap every failure in IO.

## QK-0068 — existsUnpublished stricter than the writer

Spec: [03](specs/03-txstore-formats.md). Source: wallet/udb/txdb.go:1401-1412

A transaction is unpublished only if the `up` value is exactly one non-zero byte; a zero byte or a longer value means published. Only 01 is ever written.

## QK-0069 — nil versus empty slices

Spec: [03](specs/03-txstore-formats.md). Source: wallet/udb/txunmined.go:324-335,400-410; wallet/udb/kahnsort.go:75-95

UnminedTxHashes returns a nil slice for an empty set; PruneUnmined returns a non-nil empty slice on success and nil on scan errors; dependencySort returns a non-nil empty slice.

## QK-0070 — Iterator error paths differ

Spec: [03](specs/03-txstore-formats.md). Source: wallet/udb/txdb.go:386-391,424-430,1069-1073

blockIterator.next drops its cursor without closing on a decode error, prev closes it, and the credit/debit/unmined-credit iterators keep the cursor usable after an error (a further next() continues with the following row).

## QK-0071 — Example tests are never executed

Spec: [03](specs/03-txstore-formats.md). Source: wallet/udb/txexample_test.go:19,107

ExampleStore_Rollback and Example_basicUsage take a *testing.T parameter, so go test runs them neither as examples nor as tests.

## QK-0072 — Reverse block iterator depends on a bbolt cursor property

Spec: [03](specs/03-txstore-formats.md). Source: wallet/udb/txdb.go:363-433; bbolt@v1.3.12/cursor.go:117-133,212-277

makeReverseBlockIterator seeks to ff ff ff ff (past the last key, returning nil) and then calls Prev, relying on bbolt leaving the cursor positioned so that Prev yields the last key.

## QK-0073 — Unspent ticket commitment iterator errors are silently dropped

Spec: [03](specs/03-txstore-formats.md). Source: wallet/udb/txdb.go:2124-2159, wallet/udb/txmined.go:3519-3544

`unspentTicketCommitsIterator.next` reassigns `it.err` on every step and leaves the cursor open on error; its only consumer `balanceFullScan` checks `it.err` inside the loop body (always nil there) and never after the loop, so a malformed `cmu` row or a `cmu` key without a 12-byte `cmt` row truncates `LockedByTickets`/`Total` with no error. The message "ticket commitment amount %d" is unreachable.

## QK-0074 — appendRawBlockRecord appends after trailing bytes

Spec: [03](specs/03-txstore-formats.md). Source: wallet/udb/txdb.go:266-274

The new transaction hash is appended at len(v), not at 47+32n, while the count is incremented; a block record carrying excess bytes (accepted by readers) gets those bytes reinterpreted as a transaction hash.

## QK-0075 — Absent nested bucket panics

Spec: [03](specs/03-txstore-formats.md). Source: wallet/internal/bdb/db.go:49-55, wallet/internal/bdb/db.go:100-111, wallet/udb/agendas.go:55-57

The bbolt driver returns a nil interface for a missing bucket and every txdb/txunmined/cfilters helper calls a method on it without a check; only `agendaPreferencesTy.ticketPreference` nil-checks a bucket.

## QK-0076 — upgradeToVersion3 does not validate block records

Spec: [03](specs/03-txstore-formats.md). Source: wallet/udb/txdb.go:2382-2425

A legacy `b` value shorter than 42 bytes panics on `kvp.v[:42]`; values of 42-45 bytes are widened into records that all later readers reject; an existing height-0 record is kept without comparing its hash to the genesis hash.

## QK-0077 — mi value decoded inconsistently

Spec: [03](specs/03-txstore-formats.md). Source: wallet/udb/txunmined.go:47-48, wallet/udb/txunmined.go:153-157, wallet/udb/txunmined.go:191-195

`InsertMemPoolTx` slices `v[:32]` (panics on short values) while `removeDoubleSpends` and `RemoveUnconfirmed` use the raw value as the `m` lookup key and zero-pad/truncate it into a hash with `copy`.

## QK-0078 — Account-valid bit is always written

Spec: [03](specs/03-txstore-formats.md). Source: wallet/udb/txdb.go:791-792, wallet/udb/txdb.go:1483-1484

`valueUnspentCredit` and `valueUnminedCredit` OR 0x80 into the script-type byte unconditionally; rows with the bit clear can only come from zero-widened legacy values or old software.

## QK-0079 — hasExpiry flag lost when an unmined tx is mined

Spec: [04](specs/04-txstore-behavior.md). Source: wallet/udb/txmined.go:1133-1181

moveMinedTx never sets cred.hasExpiry, so the mined credit is written with flag bit 6 clear even if the unmined credit had it; a later Rollback copies the cleared flag back. Directly-mined inserts keep the flag. Must be reproduced for byte parity; no consumer outside udb was found.

## QK-0080 — moveMinedTx keeps the unmined record bytes

Spec: [04](specs/04-txstore-behavior.md). Source: wallet/udb/txmined.go:1070, 1087-1125

The mined tx record value is the unmined value verbatim, so rec.Received and rec.SerializedTx from the mined notification are discarded. Its debit branch is dead because InsertMinedTx already consumed the unspent entries; only the unmined-input deletes matter.

## QK-0081 — Rollback ignores stake-invalidated buckets

Spec: [04](specs/04-txstore-behavior.md). Source: wallet/udb/txmined.go:2018, 2104

For a detached block whose regular tree was invalidated, credits/debits live in ic/id but Rollback only looks in c/d. The block's regular txs return to unmined with no unmined credits and unrestored debits, and ic/id entries are orphaned forever.

## QK-0082 — Stake invalidation of a same-block wallet spend chain leaves a dangling unspent entry

Spec: [04](specs/04-txstore-behavior.md). Source: wallet/udb/txmined.go:815-881

If tx A (credit) and tx B (spends it) are regular txs of the invalidated block and A precedes B in the block record, A's spent credit moves to ic, then B's debit loop unconditionally writes an unspent entry for A's output pointing at a credit no longer in c. UnspentOutputs and balance scans then fail with IO errors and ForEachUnspentOutpoint panics until re-validation or rollback. Derived from the code; no upstream test covers it.

## QK-0083 — Unmined CreditRecords are under-populated

Spec: [04](specs/04-txstore-behavior.md). Source: wallet/udb/txdb.go:1619-1636; wallet/udb/txmined.go:2275-2300

The unmined credit iterator never sets OpCode or IsCoinbase, so TxDetails.Credits of an unmined tx have OpCode 0 (not OP_NOP10) and IsCoinbase false; outputCreditInfo leaves FromCoinBase false for unmined credits. RPC code branches on these fields, so reproduce.

## QK-0084 — Credit.BlockMeta.Time and VoteBits are always zero

Spec: [04](specs/04-txstore-behavior.md). Source: wallet/udb/txmined.go:2271, 2331-2336; wallet/udb/txquery.go:71, 443-448

outputCreditInfo declares blockTime and never assigns it; TxDetails.Block.VoteBits is never set either. Only TxDetails.Block.Time is populated (from the block record).

## QK-0085 — LockedByTickets pass swallows iterator errors

Spec: [04](specs/04-txstore-behavior.md). Source: wallet/udb/txmined.go:3519-3542; wallet/udb/txdb.go:2124-2159

balanceFullScan checks it.err only inside the loop body, but next() returns false when it sets err, so a malformed unspent-commitment or missing commitment entry silently truncates LockedByTickets and returns nil error.

## QK-0086 — Nil-slice panics on stale headers

Spec: [04](specs/04-txstore-behavior.md). Source: wallet/udb/txmined.go:990-991, 1223-1224, 1395-1398, 2232-2233

InsertMinedTx and GetMainChainBlockHashes index the block record at the header's height without a nil check. Headers of detached blocks are never deleted, so a stale header above the tip panics instead of returning 'not in main chain'. AddCredit, MainChainTip, Rollback(height-1 missing) have similar unchecked reads.

Oracle confirmation (`tools/oracle/cmd/oprunner`; corpus scripts `06-chain` and `17-resolution-errors`). With the main chain tip at height t, `Store.Rollback(dbtx, h)` for h ≥ t+2 removes nothing, finds no block record at h-1 (`existsBlockRecord`, wallet/udb/txmined.go:2232) and slices the nil value in `extractRawBlockRecordHash` (wallet/udb/txdb.go:337-339): `runtime error: slice bounds out of range [:32] with capacity 0`. h = t+1 succeeds, returns no transactions and leaves the tip unchanged; h = 0 returns Invalid "cannot rollback the genesis block" (txmined.go:1882-1884). `walletdb.Update` rolls the transaction back on the panic, so the database is unchanged. dcrwallet's own callers pass a side chain's fork height at or below the tip (wallet/chainntfns.go:160) or a checkpoint height that has a main-chain hash (wallet/checkpoints.go:53-62), so only a direct udb caller reaches it. `Store.AddCredit` of a regular transaction with a `BlockMeta` whose header is not stored panics the same way (txmined.go:1394-1396; `extractBlockHeaderHeight` of a nil header, :570): `runtime error: slice bounds out of range [128:0]` on amd64 (arm64 prints `[0:0]`). `Store.TicketDetails` of a non-ticket does not panic: it returns nil details and no error (wallet/udb/txquery.go:206-215).

## QK-0087 — Coinbase detection includes treasurybase and treasury spends

Spec: [04](specs/04-txstore-behavior.md). Source: internal/compat/compat.go:19; wallet/udb/txunmined.go:43-83, 103-114; wallet/udb/txmined.go:1943

IsEitherCoinBaseTx accepts any single-input tx with a null prevout. Treasury-spend payouts are flagged coinbase; Rollback drops treasury spends instead of returning them to unmined; and because InsertMemPoolTx registers the null outpoint in unmined inputs for any non-vote tx, a second unmined treasury spend relevant to the wallet is rejected as DoubleSpend (derived, untested).

## QK-0088 — IsUnspentOutpoint is wrong and unused

Spec: [04](specs/04-txstore-behavior.md). Source: wallet/udb/txmined.go:2630-2650

It tests that the unspent bucket exists instead of looking up the key, so it returns true for any outpoint not unpublished and not spent by an unmined tx. No caller exists.

## QK-0089 — PreviousPkScripts is broken and unused

Spec: [04](specs/04-txstore-behavior.md). Source: wallet/udb/txquery.go:542-606

With block == nil it passes a 32-byte key where a 68-byte tx-record key is required (IO error plus a stack dump to stderr) and then dereferences the nil block. Only build-ignored tests call it.

## QK-0090 — UnspentTickets never returns unmined tickets

Spec: [04](specs/04-txstore-behavior.md). Source: wallet/udb/txmined.go:2670-2718

The first test requires an unspent-bucket entry for output 0, which unmined tickets lack, so includeImmature only adds mined-but-immature tickets despite the comment. Tickets spent by an unmined vote on a non-tip block are included and bypass the maturity test.

## QK-0091 — Ticket queries miss unmined spenders of mined tickets

Spec: [04](specs/04-txstore-behavior.md). Source: wallet/udb/txmined.go:2800-2817; wallet/udb/txquery.go:221-234

When the ticket's output 0 is a wallet credit, TicketIterator.Next and TicketDetails consult only the credit's mined-spent bit; an unmined vote/revocation is reported only when output 0 is not a wallet credit.

## QK-0092 — UnminedTxs order depends on Go map iteration

Spec: [04](specs/04-txstore-behavior.md). Source: wallet/udb/kahnsort.go:41-45, 75-84

Roots and independent subtrees come out in randomized order; the duplicate-edge check compares an out-edge with the parent hash and never matches (harmless). Only parents-before-children is guaranteed.

## QK-0093 — Genesis cfilter stored with an all-zero key

Spec: [04](specs/04-txstore-behavior.md). Source: wallet/udb/txmined.go:447-449, 473

InsertMissingCFilters skips key derivation for the genesis block, leaving the 16-byte key zero, while ExtendMainChain and ImportCFiltersV2 use the merkle-root prefix.

## QK-0094 — havecfilters checked three different ways

Spec: [04](specs/04-txstore-behavior.md). Source: wallet/udb/txmined.go:208-209, 402, 433, 499-515

ExtendMainChain requires the value to be exactly 0x01; IsMissingMainChainCFilters and InsertMissingCFilters accept any single non-zero byte. GetMainChainCFilters does not verify the start block is in the main chain despite its comment.

## QK-0095 — InsertMinedTx does work before its duplicate check and mixes type sources

Spec: [04](specs/04-txstore-behavior.md). Source: wallet/udb/txmined.go:1243-1327; wallet/udb/txunmined.go:53

Inputs are debited and 'bal' rewritten before the 'record exists' early return; the tx type is recomputed with DetermineTxType while AddCredit, AddTicketCommitment, RedeemTicketCommitments and InsertMemPoolTx's exception trust rec.TxType.

## QK-0096 — MakeInputSource oddities

Spec: [04](specs/04-txstore-behavior.md). Source: wallet/udb/txmined.go:3046-3058, 3104-3186, 3152-3153, 3227-3230, 3253-3254, 3321-3332

Dead deferred cursor cleanup; random phase is not account-aware (wasted tries count against the budget); any account-resolution error aborts selection for all accounts; P2SH/alt-signature credits log at error on each encounter and are never selected; zero-value skip applies to mined credits only; a nil random result ends the loop without enumerating; dead OP_TADD comparisons.

## QK-0097 — Multisig flag byte clobbered

Spec: [04](specs/04-txstore-behavior.md). Source: wallet/udb/txdb.go:1809-1814, 1896-1910; wallet/udb/txmined.go:1768-1780

Spending assigns the flags byte 0x01 and unspending 0x00, so the stake-tree bit is lost after the first spend; unspending zeroes 31 of 32 spender-hash bytes; fetchMultisigOut's length errors pass a format string to errors.E unformatted; AddMultisigOut for an unmined record over a mined entry silently resets it.

## QK-0098 — Unpublished marker handling

Spec: [04](specs/04-txstore-behavior.md). Source: wallet/udb/txunmined.go:31-34, 137-143; wallet/udb/txmined.go:1989

A repeated InsertMemPoolTx does not update the marker; SetPublished does not check existence; Rollback does not restore the marker, so a rolled-back tx is treated as published.

## QK-0099 — PruneUnmined return value omits descendants; expiry compares against tipHeight

Spec: [04](specs/04-txstore-behavior.md). Source: wallet/udb/txunmined.go:366, 400-409

Only directly queued hashes are returned, not transactions removed recursively; the expiry rule is Expiry <= tipHeight, not tipHeight+1.

## QK-0100 — Rollback treats amount 0 as 'credit already removed'

Spec: [04](specs/04-txstore-behavior.md). Source: wallet/udb/txmined.go:1966, 2061-2063

After unspending a credit, a returned amount of 0 skips balance and unspent restoration, which would also skip genuine zero-value credits (latent: the wallet does not create them). The coinbase branch caches a database slice without copying.

## QK-0101 — spendCredit fabricates a credit when none exists

Spec: [04](specs/04-txstore-behavior.md). Source: wallet/udb/txdb.go:942-974

A missing credit becomes a 94-byte zero-amount spent value; unspendRawCredit and spendCredit always rewrite to 94 bytes.

## QK-0102 — RangeBlocks discards the iterator error

Spec: [04](specs/04-txstore-behavior.md). Source: wallet/udb/txquery.go:763-772

A malformed block record ends the walk silently with a nil error, unlike rangeBlockTransactions which returns it.

## QK-0103 — UnspentOutput ignores unmined spends; outputCreditInfo prefers unmined credit

Spec: [04](specs/04-txstore-behavior.md). Source: wallet/udb/txmined.go:2248-2264, 2537-2553

UnspentOutput returns outputs already spent by an unmined tx, and outputCreditInfo returns the unmined credit even when a block is supplied; its 'mined and unmined' inconsistency branch is unreachable.

## QK-0104 — UnspentOutputs documents an undefined order but is deterministic

Spec: [04](specs/04-txstore-behavior.md). Source: wallet/udb/txmined.go:2461-2531

The doc comment says order is undefined; the implementation is strict key order over two buckets and callers iterate it.

## QK-0105 — Zero-confirmation coin selection ignores the unmined-inputs index for unmined credits

Spec: [04](specs/04-txstore-behavior.md). Source: wallet/udb/txmined.go:3135-3168, 3277-3309 (contrast 2500-2504, 3459-3461)

MakeInputSource filters mined outputs through skip() (which rejects keys spent by an unmined tx) but enumerates unmined credits at minConf == 0 with only the seen/unpublished/stake-tag tests, so an unmined credit already spent by another unmined transaction is selectable; balances and UnspentOutputs do exclude such credits. Derived from reading; needs an oracle test.

## QK-0106 — ImportCFiltersV2 stores unvalidated filter bytes verbatim

Spec: [04](specs/04-txstore-behavior.md). Source: wallet/udb/cfilters.go:34-67; dcrd/gcs/v4@v4.1.1/gcs.go:539-566

The blob is checked only by gcs.FromBytesV2 (readable canonical varint prefix; empty blob accepted) and then written as key || blob, overwriting any existing cfilter, with the merkle-root key even for genesis (InsertMissingCFilters uses an all-zero key there).

## QK-0107 — RemoveUnconfirmed rewrites multisig records without the 135-byte padding

Spec: [04](specs/04-txstore-behavior.md). Source: wallet/udb/txunmined.go:259-263; wallet/udb/txdb.go:1932-1944

It copies the stored value at its stored length before marking it unspent, while every other writer works on a zero-padded 135-byte copy; identical for normal records, divergent (index panic vs silent padding) for short ones.

## QK-0108 — PruneUnmined can report and re-remove a transaction already removed as a descendant

Spec: [04](specs/04-txstore-behavior.md). Source: wallet/udb/txunmined.go:355-409

Removal candidates are collected (with deserialized copies) before any removal; a queued child of a queued parent is removed recursively with the parent and then removed again from the queue, repeating idempotent deletes and re-puts, and its hash is still returned. Walk-phase errors return a nil slice, removal-phase errors a partial one.

## QK-0109 — RangeTransactions(-1, -1) reports no mined transactions

Spec: [04](specs/04-txstore-behavior.md). Source: wallet/udb/txquery.go:392-421, 524-537; wallet/udb/txdb.go:396-417

Both bounds map to 0x7FFFFFFF, selecting the backward walk, which positions on the tip record and stops immediately because its height is below end; only the unmined batch is delivered.

## QK-0110 — New wallets are created at version 1 and upgraded 25 times

Spec: [05](specs/05-db-migrations-aux-buckets.md). Source: wallet/udb/initialize.go:45-51; wallet/udb/txdb.go:2168-2170

Initialize writes initialVersion (1) with a v1 layout (usedaddrs bucket, addrpool keys, `sc` bucket, v1 account rows) and then calls Upgrade. The comment on createStore claims it creates the latest version. Port must reproduce the resulting bytes; the route is free.

## QK-0111 — Legacy `mgrver` key survives forever

Spec: [05](specs/05-db-migrations-aux-buckets.md). Source: wallet/udb/addressdb.go:1679; wallet/udb/migration.go:59

createManagerNS writes waddrmgr/main/mgrver = 06 00 00 00; only Migrate deletes it, so every database created in the unified layout still has it at v26 (confirmed in the v24 fixture).

## QK-0112 — Imported account index variables are 0, not 0xFFFFFFFF

Spec: [05](specs/05-db-migrations-aux-buckets.md). Source: wallet/udb/upgrades.go:303,510-517; fixture v24.db.gz

Upgrade 2 rewrites only accounts 0..lastAccount; the imported account's v1 'next index' zeros are reinterpreted as last-used = 0, copied to last-returned by upgrade 5 and into acctvars by upgrade 17.

## QK-0113 — `slip0044acct0` stays in the v1 row layout

Spec: [05](specs/05-db-migrations-aux-buckets.md). Source: wallet/udb/addressmanager.go:3073-3079,796-803

The SLIP0044 account-0 row in waddrmgr/main is written with the version 1 codec and never upgraded; it is later parsed with initialVersion.

## QK-0114 — Upgrade 26 stores Go zero time

Spec: [05](specs/05-db-migrations-aux-buckets.md). Source: wallet/udb/upgrades.go:1680-1685; wallet/udb/txmined.go:361

birthBlockUpgrade leaves BirthdayState.Time unset, so the 8-byte time field is BE64(uint64(-62135596800)) = ff ff ff f1 88 6e 09 00.

## QK-0115 — Wrong function names in error strings and comments

Spec: [05](specs/05-db-migrations-aux-buckets.md). Source: wallet/udb/upgrades.go:475,1561,419-429

Upgrade 5's mismatch error says `accountAddressCursorsUpgrade inappropriately called`; upgrade 23's says `vspStatusUpgrade inappropriately called`. Upgrade 3's comment says it creates `ticketsagendaprefs` but it creates `agendaprefs` (ticketsagendaprefs is created by upgrade 16).

## QK-0116 — Version 0 panics, version > 26 is silently accepted by Upgrade

Spec: [05](specs/05-db-migrations-aux-buckets.md). Source: wallet/udb/upgrades.go:219,1710-1720

upgrades[0] is nil; a stored version 0 passes the gate and calls a nil function. Any version >= 26 returns nil from Upgrade and is only rejected later by udb.Open.

## QK-0117 — Upgrade 14 is not byte-deterministic

Spec: [05](specs/05-db-migrations-aux-buckets.md). Source: wallet/udb/upgrades.go:1238-1252; wallet/udb/addressdb.go:1383-1394

Scripts found only in wtxmgr/sc are added as imported script addresses using a fresh random secretbox nonce and time.Now() as add-time; map iteration order is random. Golden tests must mask these.

## QK-0118 — Upgrade 12 ignores the block iterator error and uses latestTxRecord

Spec: [05](specs/05-db-migrations-aux-buckets.md). Source: wallet/udb/upgrades.go:1056-1071; wallet/udb/txdb.go:386-391,675-685

A malformed block record silently ends the mined pass and the upgrade still succeeds; each mined tx is loaded as the newest record with that hash rather than the record of the block being iterated.

## QK-0119 — Upgrade 17 mutates the bucket it iterates

Spec: [05](specs/05-db-migrations-aux-buckets.md). Source: wallet/udb/upgrades.go:1388-1410; bbolt@v1.3.12/bucket.go:386-402

accountVariablesUpgrade calls Put on waddrmgr/acct inside ForEach over the same bucket, which bbolt documents as undefined; in practice the cursor keeps reading the unmodified page so each account is visited once. Port should collect keys first.

## QK-0120 — Upgrade 9 can drop the OP_SSTXCHANGE tag

Spec: [05](specs/05-db-migrations-aux-buckets.md). Source: wallet/udb/upgrades.go:756-769

For any credit of a tx with expiry it clears bit 0x10 and restores it only if the tx is a real ticket and the output is a stake-change script; a non-ticket tx with such a credit ends with flags 0x42 (seen in fixture v7).

## QK-0121 — Go panics instead of erroring on malformed data

Spec: [05](specs/05-db-migrations-aux-buckets.md). Source: wallet/udb/upgrades.go:666,764,816; wallet/udb/vsp.go:53-54,149-161; wallet/udb/treasury.go:249; wallet/udb/migration.go:39-43

Missing buckets yield nil interfaces (method call panics); credit values shorter than 9 bytes, out-of-range credit indexes, short VSP records, short vsptreasurypolicy keys, a missing vsphostindex and Migrate on a file without waddrmgr all panic. Port should return IO errors.

## QK-0122 — SetVSPTicket mutates its argument and never updates a host pubkey

Spec: [05](specs/05-db-migrations-aux-buckets.md). Source: wallet/udb/vsp.go:49-88; wallet/udb/vsp_test.go:177-226

record.VSPHostID is overwritten with the stored host id (upstream test's DeepEqual depends on it). If the host already has a vsppubkey row, a different PubKey in the record is ignored. An empty Host fails with Invalid `key required` after writing host row and counter in the same tx.

## QK-0123 — VSP error strings contain pointers

Spec: [05](specs/05-db-migrations-aux-buckets.md). Source: wallet/udb/vsp.go:203,254

GetVSPHost formats `&id` with %v (prints a memory address) and GetVSPPubKey formats `&host` (prints `&[104 111 ...]`). Cannot be reproduced exactly; both errors are swallowed by in-tree callers.

## QK-0124 — GetVSPTicket returns PubKey aliasing bbolt memory

Spec: [05](specs/05-db-migrations-aux-buckets.md). Source: wallet/udb/vsp.go:121,269; wallet/vspticket.go:331-337

deserializeVSPPubKey returns a sub-slice of the stored value; GetVSPTicket hands it to callers that keep it after the transaction ends. Port must copy.

## QK-0125 — GetVSPTicketsByFeeStatus order is random

Spec: [05](specs/05-db-migrations-aux-buckets.md). Source: wallet/udb/vsp.go:131-137; wallet/wallet.go:5625-5630

Results are collected in a Go map and converted to a list in map order, so the gRPC TicketsHashes order differs per call; Hash.SetBytes errors are ignored.

## QK-0126 — `wstakemgr` is orphaned, Migrate no longer cleans its version key

Spec: [05](specs/05-db-migrations-aux-buckets.md). Source: wallet/udb/upgrades.go:419-423; wallet/udb/migration.go:54 (history a83b85bb)

Since commit a83b85bb the bucket is neither created nor touched; upgrade 3 no longer rewrites sstxrecords, so a DB upgraded from <=2 by the pinned code keeps old-format records there. Nothing reads them.

## QK-0127 — Treasury setter error asymmetry

Spec: [05](specs/05-db-migrations-aux-buckets.md). Source: wallet/udb/treasury.go:33-45,107-124,151-163; wallet/wallet.go:669-674

Delete errors are wrapped as IO, Put errors returned raw; an empty pi key fails on Put (Invalid `key required`) but succeeds on delete. Setters store any byte; validation to {0,1,2} is only in the wallet layer. Bulk readers return a partial map with the error.

## QK-0128 — Dead code in upgrades 14 and 23

Spec: [05](specs/05-db-migrations-aux-buckets.md). Source: wallet/udb/upgrades.go:1262-1265,1580-1582; bbolt@v1.3.12/bucket.go:318-338

Upgrade 14 tolerates NotExist from deleting `cscript` although bbolt Delete never returns it; upgrade 23 builds an unused 4-byte buffer.

## QK-0129 — Test fixture oddities

Spec: [05](specs/05-db-migrations-aux-buckets.md). Source: wallet/udb/upgrades_test.go:24-38,75,287-292; wallet/udb/testdata/v1.db.go:32; wallet/udb/testdata/v7.go:11-31,133; fixture inspection

Fixtures v1-v7 are testnet2 databases upgraded with testnet3 params (tip != genesis hash used for cf/birthstate, havecfilters ends 00); v2.db.gz still has usedaddrs and addrpool keys; v5.db.gz has no stakebase key so that assertion is vacuous; v7.go does not compile as committed (errors.Errorf without import); v24.db.gz is unused.

## QK-0130 — Upgrade is not cancellable and runs in a goroutine at daemon start

Spec: [05](specs/05-db-migrations-aux-buckets.md). Source: wallet/walletdb/interface.go:203-255; dcrwallet.go:208-230

ctx is used only for trace regions; on interrupt main returns while the upgrade tx is still running and the process exits uncommitted. bbolt atomicity keeps the file at the old version.

## QK-0131 — verifyV5Upgrade passes dbVersion 5 on a version 26 database

Spec: [05](specs/05-db-migrations-aux-buckets.md). Source: wallet/udb/upgrades_test.go:215-225

fetchDBAccount(ns, acct, 5) succeeds only because type 1 rows ignore the version argument; the verifier then requires *dbBIP0044Account, the post-17 form.

## QK-0132 — Version 3 doc comment claims vote bits are removed

Spec: [05](specs/05-db-migrations-aux-buckets.md). Source: wallet/udb/upgrades.go:42-45

votingPreferencesVersion says per-ticket vote bits are removed; the pinned upgrade only creates agendaprefs and leaves wstakemgr untouched.

## QK-0133 — Host known-ness decided only by vsppubkey

Spec: [05](specs/05-db-migrations-aux-buckets.md). Source: wallet/udb/vsp.go:49-80

SetVSPTicket never searches vsphost by host bytes; the reserved ID 0 (empty host) has no vsppubkey row and cannot get one because bbolt rejects empty keys.

## QK-0134 — Package test set-up calls Upgrade twice

Spec: [05](specs/05-db-migrations-aux-buckets.md). Source: wallet/udb/common_test.go:46-65

createEmptyDB runs Initialize (which upgrades to 26) and then Upgrade again; the second call is the only upstream exercise of the version >= 26 no-op return.

## QK-0135 — Upgrade 8 opens read-write cursors it only reads through

Spec: [05](specs/05-db-migrations-aux-buckets.md). Source: wallet/udb/upgrades.go:643

hasExpiryUpgrade uses ReadWriteCursor on c and mc but writes through the bucket afterwards; hasExpiryFixedUpgrade uses ReadCursor for the same traversal.

## QK-0136 — Go map iteration order leaks into vote transactions and RPC listings

Spec: [06](specs/06-wallet-core-part1.md). Source: wallet/wallet.go:280, :505, :645, :651; wallet/chainntfns.go:806-823; internal/rpc/jsonrpc/methods.go:3832-3850

GetAllTSpends returns cached tspends in map order, which becomes the order of treasury votes in the 'TV' output of every vote the wallet creates and of tspend policy listings; TreasuryKeyPolicies returns two map-ordered groups that become the JSON-RPC treasurypolicy array order; SetAgendaChoices iterates its input map so the reported invalid entry is arbitrary. Votes are not byte-reproducible when two or more tspends are in window.

## QK-0137 — Lock() does not cancel an armed unlock timeout

Spec: [06](specs/06-wallet-core-part1.md). Source: wallet/wallet.go:1804-1849

Lock sets passphraseTimeoutCancel = nil without signalling the old channel, so the timer task from an earlier timed unlock survives and calls Lock() when it fires. Sequence walletpassphrase p 60, walletlock, walletpassphrase p 0 leaves a wallet 'unlocked without a time limit' that locks about 60 s after the first call, logging 'The wallet has been locked due to timeout.' Not covered by TestLocking.

## QK-0138 — --watchlast wraparound watches nothing on never-returned branches

Spec: [06](specs/06-wallet-core-part1.md). Source: wallet/wallet.go:931-932, :1004-1006, :1014

For a branch with lastReturned = 0xffffffff, end = gapLimit-1, so end - gapLimit wraps to 0xffffffff; for any nonzero watchLast the start becomes 0xffffffff - watchLast and the load loop never runs. No address of that branch is put in the tx filter on first watch, and children 0..gapLimit-2 stay unwatched by this function afterwards.

## QK-0139 — Block locators repeat the tip and may omit genesis

Spec: [06](specs/06-wallet-core-part1.md). Source: wallet/wallet.go:1510-1532

The locator list is seeded with the tip hash and the loop's first iteration appends the tip again; steps double once the list exceeds 10 entries; genesis is included only if the sequence lands on height 0. The int32 step doubles without overflow check (wraps negative for heights >= 2^30 as the loop exits).

## QK-0140 — CurrentAddress returns the next address, not the most recently returned

Spec: [06](specs/06-wallet-core-part1.md). Source: wallet/wallet.go:1948-1973; wallet/addresses.go:637

Doc says 'most recently requested'; the index lastUsed + 1 + cursor is what the next NewExternalAddress will return because the cursor is post-incremented. The address is not recorded as returned, no gap policy is applied, and ErrInvalidChild is an error rather than skipped. Exposed by JSON-RPC getaccountaddress.

## QK-0141 — Split-tx fee estimate passes an input size as a script size

Spec: [06](specs/06-wallet-core-part1.md). Source: wallet/wallet.go:1648-1700; wallet/txsizes/size.go:55, :132-199

PurchaseTickets' retry path calls EstimateSerializeSize with RedeemP2PKHInputSize (166) where a signature-script size (108) is expected, giving 275 bytes instead of 217 (2750 vs 2170 atoms at the default fee). The same path also publishes through w.NetworkBackend() instead of the passed backend, runs changeSourceUpdates.UpdateDB twice, mutates the caller's request (Count=1, MinConf=0) and dereferences VSPClient without a nil check.

## QK-0142 — FetchMissingCFilters op string is a Go struct dump

Spec: [06](specs/06-wallet-core-part1.md). Source: wallet/wallet.go:1334-1338, :1377, :1411-1413, :1427, :1454

The error op is built with %v of the backend; chain.Syncer (the only real argument) has no String method, so the op text is Go's default struct dump including per-run pointer values. Progress and log always report start+1999 as the end even for a short final batch; height is advanced only on the next iteration; a backend returning more filters than requested panics; the non-progress FetchMissingCFilters has no caller.

## QK-0143 — Panics reachable from unvalidated indexes

Spec: [06](specs/06-wallet-core-part1.md). Source: wallet/wallet.go:230, :615; chain/sync.go:702-704, :925-931

FetchOutput indexes TxOut[outPoint.Index] without a bounds check (and ignores the tree); TSpendPolicy slices SignatureScript[66:99] of a cached tspend without validation, and in RPC mode cached tspends come from dcrd unvalidated.

## QK-0144 — watchHDAddrs / account-creation races

Spec: [06](specs/06-wallet-core-part1.md). Source: wallet/wallet.go:957-1000, :1023, :1033-1034, :2199-2213

NextAccount and the import functions commit an account before adding its address buffer. If watchHDAddrs reads the DB after the commit but locks addressBuffers before the buffer exists, branch keys are nil and derivation dereferences nil; in the opposite interleaving a zero-valued account record rewrites a fresh buffer's lastUsed from 0xffffffff to 0. Found by code reading, not observed. Also: lastWatched is advanced before loading on first watch, and the returned count includes failed batches.

## QK-0145 — Unlock log lines do not match the action

Spec: [06](specs/06-wallet-core-part1.md). Source: wallet/wallet.go:1738, :1808-1838

A timed unlock of a wallet already unlocked without limit logs 'The wallet has been temporarily unlocked' although the timeout is ignored; replacing one timeout with another logs nothing. Every failure of manager.Unlock on the locked path is reported as kind Passphrase regardless of cause.

## QK-0146 — AccountBalances order and cost; mismatched op name

Spec: [06](specs/06-wallet-core-part1.md). Source: wallet/wallet.go:1912, :1927-1946; wallet/udb/addressdb.go:269, :698-708; wallet/udb/txmined.go:3561-3583

Iteration is bbolt key order over little-endian account numbers (account 0, 0x80000000, account 1, 0x80000001, ..., imported 0x7fffffff last; account 256 would sort between 0 and 1). Each per-account call does a full scan of all balances (quadratic). AccountBalance wraps errors with op 'wallet.CalculateAccountBalance'.

## QK-0147 — NextAccount comment/code mismatches and partial failure

Spec: [06](specs/06-wallet-core-part1.md). Source: wallet/wallet.go:2136-2239

Comments say 'previous 100 accounts' but the limit is accountGapLimit (default 10). gapLimit+1 address rows are stored per branch while gapLimit addresses are loaded into the filter. A filter-load failure returns an error after the account is already committed and buffered, without the account notification. accountGapLimit 0 always refuses; a negative value always permits.

## QK-0148 — Per-ticket agenda preferences: no per-agenda fallback, version-keyed

Spec: [06](specs/06-wallet-core-part1.md). Source: wallet/wallet.go:346-375, :436-455, :472-476, :544-545; wallet/udb/agendas.go:23-28

A ticket with any saved preference reports abstain for its other agendas rather than the wallet default; preferences are keyed by vote version so all saved choices silently revert to abstain when the vote version is raised; the version is big-endian in the DB key and little-endian in vote bits. SetAgendaChoices with a ticket and an empty map returns vote bits 0; its doc comment about repeated choices cannot apply to a map argument.

## QK-0149 — Asymmetric fallback between TSpendPolicy and TreasuryKeyPolicy

Spec: [06](specs/06-wallet-core-part1.md). Source: wallet/wallet.go:572-628

TSpendPolicy falls back from per-ticket to wallet-wide policies; TreasuryKeyPolicy(pikey, ticket) consults only the per-ticket key map and returns 0 when absent.

## QK-0150 — TSpend cache docs stale; arrival logged before duplicate check

Spec: [06](specs/06-wallet-core-part1.md). Source: wallet/wallet.go:259-289

GetAllTSpends' comment says the list is not culled but the code deletes entries whose expiry is below the tip height; AddTSpend logs 'TSpend arrived' before detecting a duplicate and returns a plain 'tspend already cached' error.

## QK-0151 — LoadActiveDataFilters does network I/O inside DB read transactions

Spec: [06](specs/06-wallet-core-part1.md). Source: wallet/wallet.go:1112-1128, :1140-1150, :1169-1198

LoadTxFilter is called from inside the imported-address and unspent-outpoint View transactions and while holding lockedOutpointMu (blocking tx authoring). Ticket outpoints are sent twice. The last outpoint batch is flushed from a deferred function after the completion log line and after the lock is released.

## QK-0152 — MainChainTip swallows database errors

Spec: [06](specs/06-wallet-core-part1.md). Source: wallet/wallet.go:792-802

The View error is discarded; on failure the zero hash and height 0 are returned.

## QK-0153 — Inconsistent error wrapping

Spec: [06](specs/06-wallet-core-part1.md). Source: wallet/wallet.go:462, :480, :494, :686, :736, :828, :841, :1752-1769, :1994, :2339

Some exported methods wrap with an op, some return callee errors unchanged (BlockHeader, CFilterV2, SetAccountPassphrase, UnlockAccount, LockAccount, SignHashes, TxBlock, treasury setters), and several originate errors with no op ('ticket not found', 'no agendas to set for this network', 'transaction(s) not found').

## QK-0154 — GetTransactionsByHashes returns data together with an error

Spec: [06](specs/06-wallet-core-part1.md). Source: wallet/wallet.go:2319-2342; spv/sync.go:818

Found transactions and the not-found inventory list are returned along with a NotExist error; callers must treat it as partial success.

## QK-0155 — Networks without a vote version

Spec: [06](specs/06-wallet-core-part1.md). Source: wallet/wallet.go:291-314, :415-417, :479-481

Regnet and unknown networks get vote version 1 with no deployments: AgendaChoices returns an empty map and bits 0 (not 1), SetAgendaChoices fails with 'no agendas to set for this network'. dcrwallet has no regnet mode.

## QK-0156 — Deployment test asymmetries

Spec: [06](specs/06-wallet-core-part1.md). Source: deployments/deployments.go:92-157; wallet/wallet.go:1633-1647

DCP0010Active is unconditionally true on simnet while DCP0012Active needs a querier there (nil querier is a Bug error); int32(d.Since) truncates dcrd's 64-bit height; SPV callers skip the tests and assume both active.

## QK-0157 — BlockIdentity.None treats height 0 with zero hash as no block

Spec: [06](specs/06-wallet-core-part1.md). Source: wallet/common.go:27-32

Documented in source as a bug: both 0 and -1 denote unmined, so genesis cannot be represented.

## QK-0158 — Dead and vestigial code

Spec: [06](specs/06-wallet-core-part1.md). Source: wallet/log.go:22-38; wallet/network.go:126-134; wallet/wallet.go:86, :1422, :2300; deployments/deployments.go:41-55; wallet/doc.go:6-9

debugLog/infoLog and their types, closedDone and its init, StakeDifficultyInfo, TxConfirms, FetchMissingCFilters, DCP0002/DCP0003, testnet2 branches; wallet/doc.go is a TODO placeholder; OfflineNetworkBackend.Synced reports (true, 0).

## QK-0159 — SignHashes accepts non-32-byte hashes; VerifyMessage compares strings

Spec: [06](specs/06-wallet-core-part1.md). Source: wallet/wallet.go:1997-2001, :2062; dcrd/dcrec/secp256k1/v4@v4.4.1/nonce.go:117-141; dcrd/dcrec/secp256k1/v4@v4.4.1/modnscalar.go:360-368

ecdsa.Sign uses the first 32 bytes of longer inputs and left-pads shorter ones, so gRPC SignHashes signs arbitrary-length inputs; VerifyMessage compares address strings, so non-P2PKH address inputs can never verify.

## QK-0160 — Lock() on a locked wallet does not relock individually-unlocked accounts

Spec: [06](specs/06-wallet-core-part1.md). Source: wallet/wallet.go:1842-1849; wallet/udb/addressmanager.go:357-377, :1453-1469

manager.Lock returns kind Locked before clearing keys when already locked and Wallet.Lock discards the error, so an account unlocked with UnlockAccount while the wallet is locked survives walletlock; when the wallet is unlocked the same Lock() drops every account key including individually-unlocked ones.

## QK-0161 — Unlock acts on a stale wasLocked flag

Spec: [06](specs/06-wallet-core-part1.md). Source: wallet/wallet.go:1715-1718, :1740-1743

wasLocked and the passphrase check are sampled under a read lock that is released before the dispatch; on the already-unlocked path no lock is held until replacePassphraseTimeout, so a concurrent Lock can produce a nil return for a locked wallet, a misleading log line and an armed timeout task. Found by reading, not observed.

## QK-0162 — Filter outpoint sources are asymmetric and duplicate tickets

Spec: [06](specs/06-wallet-core-part1.md). Source: wallet/wallet.go:1171-1194; wallet/udb/txmined.go:2557-2627, :2656-2721

ForEachUnspentOutpoint skips outputs of unpublished transactions and yields mined-unspent then unmined credits; UnspentTickets only returns tickets with a mined-unspent output 0, so every such ticket not spent by an unmined tx is sent to LoadTxFilter twice, and only stale-vote tickets come solely from the ticket pass.

## QK-0163 — NetworkBackend.StakeDifficulty comment is wrong for RPC mode

Spec: [06](specs/06-wallet-core-part1.md). Source: wallet/network.go:43-46; chain/backend.go:58-60

The interface comment says the call always errors; that is true for the SPV backend only, chain.Syncer forwards dcrd's getstakedifficulty result.

## QK-0164 — Stale or misleading doc comments in scope

Spec: [06](specs/06-wallet-core-part1.md). Source: wallet/wallet.go:103-104, :120, :218; wallet/log.go:15-17; wallet/network.go:19-25

FetchOutput claims it cannot fetch multisig outputs (no such restriction in code); UseLogger refers to a nonexistent SetLogWriter; FilterProof refers to a Peer interface; tspendKeyPolicy is called 'keyed by politeia key' though the key is the raw treasury pubkey bytes; disapprovePercent is described as simnet/testnet-only though the wallet layer has no network check.

## QK-0165 — Account iteration is little-endian byte order, not numeric

Spec: [07](specs/07-wallet-core-part2.md). Source: wallet/udb/addressdb.go:269-273, wallet/udb/addressdb.go:698-708, wallet/wallet.go:3390

ForEachAccount walks the acct bucket whose keys are little-endian uint32 account numbers, so with accounts 0,1,2,256,0x7FFFFFFF,0x80000000,0x80000001 the order is 0, 0x80000000, 256, 1, 0x80000001, 2, imported. Visible in gRPC Accounts, lockaccount *, and TotalReceivedForAccounts.

## QK-0166 — listtransactions paging counts transactions, not entries

Spec: [07](specs/07-wallet-core-part2.md). Source: wallet/wallet.go:2567-2604

from skips transactions even if they would yield no entries; count limits transactions that yield at least one entry; the returned array can be longer than count; the stop test runs before the skip test so count <= 0 returns []. The list is built newest-first (sends then receives) and reversed, giving receives then sends in descending vout order per transaction.

## QK-0167 — Negative zero amounts in JSON

Spec: [07](specs/07-wallet-core-part2.md). Source: wallet/wallet.go:2507

Send entries negate the float amount; for zero-value outputs (ticket commitments, vote OP_RETURN outputs) this is IEEE -0.0, which Go's encoding/json writes as -0.

## QK-0168 — fee is emitted as 0 when it cannot be determined

Spec: [07](specs/07-wallet-core-part2.md). Source: wallet/wallet.go:2426-2440, wallet/wallet.go:2508

Fee points at a float that stays 0.0 unless every input is a wallet debit, and is attached to every send entry; receive entries never carry fee. The value is the negated fee (outputs minus debits).

## QK-0169 — Treasurybases and treasury spends are treated as coinbases

Spec: [07](specs/07-wallet-core-part2.md). Source: internal/compat/compat.go:19-27, dcrd/blockchain/standalone/v2@v2.3.0/tx.go:49-101

compat.IsEitherCoinBaseTx ORs the pre- and post-treasury tests; the pre-treasury test accepts any single-input null-prevout transaction, so treasury spend payouts are listed as generate/immature with generated:true and TxTransactionType reports Coinbase.

## QK-0170 — ListAddressTransactions matches only plain P2PKH credits

Spec: [07](specs/07-wallet-core-part2.md). Source: wallet/wallet.go:2626-2639, internal/rpc/jsonrpc/methods.go:3242-3256

It tests ExtractPubKeyHashV0 on version-0 scripts only; P2SH, P2PK and stake-tagged outputs never match even though the JSON-RPC handler puts any Hash160er address (including P2SH) into the lookup set. A match returns all entries of the transaction.

## QK-0171 — listsinceblock includes the named block

Spec: [07](specs/07-wallet-core-part2.md). Source: wallet/wallet.go:2541, internal/rpc/jsonrpc/methods.go:3164-3179

RangeTransactions(-1, height) is inclusive of height, and the handler does not start at the fork point (its own TODO).

## QK-0172 — 'Precise' ticket queries never call dcrd

Spec: [07](specs/07-wallet-core-part2.md). Source: wallet/wallet.go:2888-2933

makeTicketSummary uses the rpc argument only as a nil test to choose Live versus Unspent; missed/expired come solely from a recorded revocation. The doc comments claim otherwise.

## QK-0173 — Ticket spent by an unmined vote/revocation is reported unspent

Spec: [07](specs/07-wallet-core-part2.md). Source: wallet/udb/txquery.go:221-234, wallet/udb/txmined.go:2800-2817, wallet/wallet.go:2917-2928

TicketDetails and TicketIterator consult the unmined-inputs bucket only when no credit exists for output 0; if the credit exists and is not spent by a mined transaction, the spender is treated as absent. Status stays live/unspent and StakeInfo counts it under Unspent. In the no-credit branch an unmined revocation has height -1 and is classified missed.

## QK-0174 — Two different expiry thresholds

Spec: [07](specs/07-wallet-core-part2.md). Source: wallet/wallet.go:2921-2923, wallet/wallet.go:4193-4195, wallet/wallet.go:4444-4447

makeTicketSummary and StakeInfoPrecise use h_rev - h >= expiry + maturity; ticketExpired (UnspentExpired) uses tip - h > maturity + expiry.

## QK-0175 — listunspent tie order depends on Go's unstable sort

Spec: [07](specs/07-wallet-core-part2.md). Source: wallet/wallet.go:3435-3470, wallet/wallet.go:3491

sort.Sort(sort.Reverse(creditSlice)) compares different transactions at the same height (or unmined with the same receive second) as equal; the resulting order is whatever Go's sort produces from bucket-key input order.

## QK-0176 — ListUnspent returns nil (JSON null) and uses inclusive bounds, contrary to its doc comment

Spec: [07](specs/07-wallet-core-part2.md). Source: wallet/wallet.go:3472-3479, wallet/wallet.go:3510

The comment says 'more than minconf, less than maxconf' and 'an empty array will be returned'; the code uses confs < minconf || confs > maxconf to exclude and returns a nil slice.

## QK-0177 — ListUnspent spendability and redeem-script gaps

Spec: [07](specs/07-wallet-core-part2.md). Source: wallet/wallet.go:3576-3657, internal/rpc/jsonrpc/methods.go:3317-3321

Stake submission, treasury and non-ECDSA script types report spendable:false; redeem scripts are returned only for untagged P2SH; a P2SH output whose redeem script is unknown aborts the whole call with NotExist, which listunspent maps to address-not-in-wallet; only the first address is reported (source BUG comment) while the address filter considers all.

## QK-0178 — Lock-order inversion between Accounts and ListUnspent with a real deadlock window

Spec: [07](specs/07-wallet-core-part2.md). Source: wallet/wallet.go:3379-3382, wallet/wallet.go:3550, wallet/chainntfns.go:110-112, bbolt@v1.3.12/db.go:433, bbolt@v1.3.12/db.go:735

Accounts takes lockedOutpointMu then a read transaction (and never reads the set); ListUnspent takes a read transaction then lockedOutpointMu per output. Writers such as chain switch hold lockedOutpointMu while opening a write transaction; bbolt read transactions hold the mmap read lock and a growing write transaction needs the mmap write lock.

## QK-0179 — listlockunspent order is random and locks are unvalidated

Spec: [07](specs/07-wallet-core-part2.md). Source: wallet/wallet.go:4260-4355

The set is copied by Go map iteration; LockOutpoint accepts any hash/index and ignores the tree; LockedOutpoints silently hides entries that do not resolve and returns its error unwrapped along with a partial slice.

## QK-0180 — ImportPrivateKey/ImportPublicKey can fail after committing

Spec: [07](specs/07-wallet-core-part2.md). Source: wallet/wallet.go:3710-3729, internal/rpc/jsonrpc/methods.go:2043-2046

A LoadTxFilter failure is returned although the key is already in the database; a retry fails with Exist, which JSON-RPC reports as success while skipping the rescan. No log or notification is produced for the failed attempt.

## QK-0181 — ImportScript does network I/O inside the write transaction and logs before commit

Spec: [07](specs/07-wallet-core-part2.md). Source: wallet/wallet.go:3782-3800

LoadTxFilter is called inside walletdb.Update and the info log line is emitted before the commit; a failed commit leaves the address in the backend filter. No account notification is sent for script imports.

## QK-0182 — Account import ordering asymmetry

Spec: [07](specs/07-wallet-core-part2.md). Source: wallet/wallet.go:3838-3951

ImportXpubAccount loads the filter before the DB write (a failed write leaves the addresses watched); ImportVotingAccount writes first and a later failure leaves the account without an in-memory address buffer until reopen; ImportVotingAccount deliberately does not watch the internal branch.

## QK-0183 — ImportPrivateKey does not check the WIF signature scheme

Spec: [07](specs/07-wallet-core-part2.md). Source: wallet/wallet.go:3705-3738, wallet/udb/addressmanager.go:1253-1291

A WIF with scheme byte 1 or 2 is stored as a secp256k1-ECDSA P2PKH key; neither RPC handler checks wif.DSA(). End-to-end behaviour for Ed25519 is unverified.

## QK-0184 — UnminedTransactions order is only partially defined

Spec: [07](specs/07-wallet-core-part2.md). Source: wallet/udb/kahnsort.go:75-116, wallet/wallet.go:4361-4380

dependencySort seeds its queue from a Go map, so unrelated transactions come out in random order; only parent-before-child is guaranteed. Unpublished transactions are excluded.

## QK-0185 — Inconsistent error wrapping in key/account functions

Spec: [07](specs/07-wallet-core-part2.md). Source: wallet/wallet.go:3817-3827, wallet/wallet.go:3896, wallet/wallet.go:2826-2834, wallet/wallet.go:3308-3332

votingXprivFromSeed attaches op and kind Invalid only to the length check and returns HDKeysFromSeed errors bare; ImportXpubAccount's 'extended key must be an xpub' has kind Other (gRPC default code); fetchTicketDetails errors are plain fmt errors; Spender, UnspentOutput and LockedOutpoints return store errors without an op.

## QK-0186 — GetTickets ignores a header decode error and reuses one header object

Spec: [07](specs/07-wallet-core-part2.md). Source: wallet/wallet.go:3147, wallet/wallet.go:3221, wallet/wallet.go:3260-3264

header.FromBytes's error is discarded and the same *wire.BlockHeader is passed to every callback; TicketSummary hash pointers alias storage reused by the next callback. GetTransactions copies and checks instead.

## QK-0187 — StakeInfoPrecise can return before its goroutine finishes

Spec: [07](specs/07-wallet-core-part2.md). Source: wallet/wallet.go:4119-4127, wallet/wallet.go:4235-4243

On a database error the function returns without errgroup.Wait; the MempoolCount goroutine later writes to a discarded struct.

## QK-0188 — Sdiff is zero on regnet and unknown networks

Spec: [07](specs/07-wallet-core-part2.md). Source: deployments/deployments.go:59-73, wallet/wallet.go:4018

HardcodedDeployment.Active has cases only for mainnet, testnet2, testnet3 and simnet, so StakeInfo never computes the next stake difficulty elsewhere.

## QK-0189 — BlockIdentifier heights are not validated

Spec: [07](specs/07-wallet-core-part2.md). Source: wallet/wallet.go:3040-3081, internal/rpc/rpcserver/server.go:1366

A negative height flows into RangeTransactions where negative start means 'unmined first, then tip downward' and negative end means 'to tip, then unmined'; gRPC forwards any non-zero request height. Hash identifiers are not checked to be on the main chain.

## QK-0190 — Imported account pseudo-properties

Spec: [07](specs/07-wallet-core-part2.md). Source: wallet/udb/addressmanager.go:586-600, internal/rpc/rpcserver/server.go:401-403

For account 0x7FFFFFFF AccountProperties leaves the four index fields at 0 rather than 0xFFFFFFFF, so gRPC Accounts (which adds 1) reports one external and one internal key for the imported account.

## QK-0191 — WIF checksum comment disagrees with code

Spec: [07](specs/07-wallet-core-part2.md). Source: dcrd/dcrutil/v4@v4.0.3/wif.go:94, dcrd/dcrutil/v4@v4.0.3/wif.go:164-180

dcrutil's DecodeWIF comment says the checksum is double SHA256; the code uses chainhash.HashB (single BLAKE-256).

## QK-0192 — Short chains hide tickets from ForUnspentUnexpiredTickets

Spec: [07](specs/07-wallet-core-part2.md). Source: wallet/wallet.go:5734-5741, wallet/udb/txquery.go:388-397, :520-538

start = tip - (TicketExpiry + TicketMaturity - 8) is negative on chains shorter than that span; RangeTransactions treats a negative begin as 'unmined first, then backward from MaxInt32 to end', so only unmined tickets and the tip block are visited. Affects simnet/regnet harnesses (span 392).

## QK-0193 — walletdb.View never reports a rollback error

Spec: [07](specs/07-wallet-core-part2.md). Source: wallet/walletdb/interface.go:203-255

The deferred rollback assigns to the local err from BeginReadTx, not to the function result (View has an unnamed return), so rollback failures of read transactions are silently dropped; Update does return commit errors.

## QK-0194 — ListAddressTransactions empty-key match

Spec: [07](specs/07-wallet-core-part2.md). Source: wallet/wallet.go:2630-2631

Non-P2PKH credits look up string(nil) == "" in pkHashes; a set containing the empty string would match every version-0 non-P2PKH credit. Unreachable from the JSON-RPC handler.

## QK-0195 — listunspent address filter is form-sensitive

Spec: [07](specs/07-wallet-core-part2.md). Source: wallet/wallet.go:3576-3584, internal/rpc/jsonrpc/methods.go:3288-3299, dcrd/txscript/v4@v4.1.2/stdscript/addressv0.go:55-67

The filter compares addr.String() of extracted addresses with the handler's decoded-address strings; P2PK and multisig keys are extracted in pubkey form, so a pubkey-hash request address never selects a P2PK output of the same key.

## QK-0196 — minConf larger than the chain counts everything, including unmined

Spec: [08](specs/08-wallet-core-part3.md). Source: wallet/wallet.go:4488-4492, 4540-4544; wallet/udb/txquery.go:394-398, 534-537

TotalReceivedForAccounts/TotalReceivedForAddr compute stopHeight = tip - minConf + 1; when minConf > tip+1 it is negative and the store treats any negative end height as 'to the tip then the unmined set', so a huge minconf returns the full total including unconfirmed transactions instead of zero.

## QK-0197 — Account rows ordered by little-endian key bytes, but getreceivedbyaccount indexes by account number

Spec: [08](specs/08-wallet-core-part3.md). Source: wallet/wallet.go:4470-4481; wallet/udb/addressdb.go:698-708, 930-934; internal/rpc/jsonrpc/methods.go:2416-2420

TotalReceivedForAccounts appends rows in bbolt order of the 4-byte LE account number (0, 0x80000000, 256, 1, 257, ..., imported 0x7FFFFFFF last). JSON-RPC getreceivedbyaccount uses results[accountNumber] (or the last row for imported), so with >= 256 accounts or any imported-xpub account it returns the wrong account's total.

## QK-0198 — ForUnspentUnexpiredTickets only examines the tip block on short chains

Spec: [08](specs/08-wallet-core-part3.md). Source: wallet/wallet.go:5734-5741; wallet/udb/txquery.go:394-421, 524-538

When tip < TicketExpiry+TicketMaturity-8 (41208 mainnet, 6152 testnet3, 392 simnet/regnet) the start height is negative; RangeTransactions then visits unmined first and iterates blocks backwards from max height down to the end height (the tip), so only tip-block tickets are seen. UnprocessedTickets, ProcessedTickets and VSP vote-choice updates see almost nothing on simnet/young chains.

## QK-0199 — Stale attached/detached block state survives a failed ChainSwitch

Spec: [08](specs/08-wallet-core-part3.md). Source: wallet/notifications.go:263-304; wallet/chainntfns.go:146-154; wallet/rescan.go:200

currentTxNtfn is filled by notifyDetachedBlock/notifyAttachedBlock/notifyMinedTransaction inside the ChainSwitch DB transaction and cleared only by sendAttachedBlockNotification. If the transaction fails (e.g. 'failed reorganize ... less total work' after detached blocks were recorded) the entries are delivered with the next successful switch. Rescan and AddTransaction-with-block leave an empty non-nil currentTxNtfn and their mined transactions are never notified.

## QK-0200 — Latent nil dereference in sendAttachedBlockNotification

Spec: [08](specs/08-wallet-core-part3.md). Source: wallet/notifications.go:314-339; wallet/chainntfns.go:118-120; wallet/sidechains.go:417-458

It reads currentTxNtfn.AttachedBlocks without a nil check. ChainSwitch returns early from its DB transaction when the wallet tip already equals the chain tip without creating currentTxNtfn; with a registered transaction client and no stale state the process panics. All four callers pass EvaluateBestChain results (strictly more work than tip), so it needs two concurrent ChainSwitch calls for the same chain to trigger.

## QK-0201 — Private keys printed in a SignTransaction error string

Spec: [08](specs/08-wallet-core-part3.md). Source: wallet/wallet.go:4816-4823; dcrd/dcrutil@v4.0.3/wif.go:167; internal/rpc/jsonrpc/methods.go:5104-5113

When additionalKeysByAddress is non-empty and lacks a needed address the error is formatted with %v of the whole map[string]*dcrutil.WIF; *WIF implements String(), so the message contains every supplied WIF private key ('no key for address (needed: X, have map[addr:WIF ...])') and is returned in signrawtransaction errors[].error.

## QK-0202 — SignTransaction validation asymmetries

Spec: [08](specs/08-wallet-core-part3.md). Source: wallet/wallet.go:4809, 4824, 4832, 4877-4906

The 'multisig, not enough signatures' suppression inspects only the caller-supplied additionalPrevScripts map, never the script fetched from the wallet DB; errEval keeps only the last non-stack-underflow engine error, which is reported both as a SignatureError and as the fatal return; the compressed flag is always true; a missing previous output index panics.

## QK-0203 — AbandonTransaction doc vs code

Spec: [08](specs/08-wallet-core-part3.md). Source: wallet/wallet.go:5062-5092

The comment says it does not error when the transaction does not exist unmined; the code returns NotExist for an unknown hash. Only the requested hash is notified as removed, not recursively removed descendants, and locked input outpoints are not released.

## QK-0204 — SendOutputs without a network backend leaves the transaction recorded

Spec: [08](specs/08-wallet-core-part3.md). Source: wallet/wallet.go:4592-4603; wallet/createtx.go:334-340

The transaction is recorded in the DB (and notified) before publishAndWatch resolves the backend; a missing backend returns NoPeers without abandoning it, so it is broadcast later by PublishUnminedTransactions even though the RPC call failed.

## QK-0205 — SelectInputs ignores the locked-outpoint set and rejects imported-xpub accounts

Spec: [08](specs/08-wallet-core-part3.md). Source: wallet/utxos.go:25-46

It holds lockedOutpointMu but passes a nil ignore function to MakeInputSource, so locked outputs can be selected; any account above LastAccount other than 0x7FFFFFFF fails with NotExist 'account not found'.

## QK-0206 — Account notification key counts are off and wrap

Spec: [08](specs/08-wallet-core-part3.md). Source: wallet/notifications.go:589-627; wallet/wallet.go:2130, 2236, 3734, 3771

ExternalKeyCount/InternalKeyCount = min(2^31, lastUsed + gapLimit) with u32 wraparound: an unused branch (0xFFFFFFFF) with gap limit 20 reports 19, a branch with last used index 0 reports 20 (21 addresses are watched). The type comment claims notifications fire on any property change; only four call sites exist.

## QK-0207 — CreateVspPayment oddities

Spec: [08](specs/08-wallet-core-part3.md). Source: wallet/wallet.go:4702-4756

Negative change (inputs cover the VSP fee but not the tx fee) is treated as dust and silently dropped; only the last signature error survives in the returned message (leading tab, trailing space); logs 'Failed to sign transaction: <nil>' when only per-input errors occurred; a change address is consumed even if no change output is added; the unpublished flag is written in a separate DB transaction before the tx exists.

## QK-0208 — NewBalances is computed but never transmitted

Spec: [08](specs/08-wallet-core-part3.md). Source: wallet/notifications.go:183-212, 246-252, 340-349; internal/rpc/rpcserver/server.go:2397-2403

Every transaction notification scans the whole unspent set to compute per-account balances returned in Go map order, but the gRPC handler does not put them in the response. The only observable effect is that a failure while computing them suppresses the whole notification.

## QK-0209 — Treasury base and treasury spend classify as coinbase

Spec: [08](specs/08-wallet-core-part3.md). Source: wallet/notifications.go:430-442; internal/compat/compat.go; dcrd/blockchain/standalone@v2.3.0/tx.go:49-102

TxTransactionType and OutputInfo.FromCoinbase use compat.IsEitherCoinBaseTx, whose first test (isTreasuryEnabled=false) accepts any single-input transaction with a null previous outpoint; treasury adds are 'regular'.

## QK-0210 — Silent zero values in transaction summaries

Spec: [08](specs/08-wallet-core-part3.md). Source: wallet/notifications.go:59-112, 146-162

lookupOutputChain returns account 0, amount 0, nil address and nil script when the address lookup fails or the script yields no address (the entry is still emitted); lookupInputAccount returns account 0 on every failure. Account 0 then joins the relevant-accounts set.

## QK-0211 — Confirmation notification details

Spec: [08](specs/08-wallet-core-part3.md). Source: wallet/notifications.go:820-988

A result is emitted on every tip change even with an empty watch set; the invalidation test marks any tx (including still-valid stake-tree txs) in a stake-invalidated block as -1 and is skipped at height 0 and for the tip block; Watch reorders results by swap-removal, mutates the caller's slice and still adds remaining hashes to the watch set after a DB error; per-block result order is Go map order; Recv always returns context.Canceled.

## QK-0212 — Notifications sent from inside uncommitted DB transactions

Spec: [08](specs/08-wallet-core-part3.md). Source: wallet/chainntfns.go:261-262, 659-672; wallet/notifications.go:225-261

Unmined-transaction notifications and pruning removals are sent before the enclosing write transaction commits (with the server mutex and usually lockedOutpointMu held); if the transaction then fails, subscribers were told about state that never existed, and a slow subscriber holds the wallet's writer open.

## QK-0213 — NeedsAccountsSync doc vs code

Spec: [08](specs/08-wallet-core-part3.md). Source: wallet/wallet.go:5191-5223

The comment says the genesis block must be the main chain tip; the code checks only that the last account is 0 and account 0 has no address rows.

## QK-0214 — Relevance by last sigscript push; P2PK outputs not relevant

Spec: [08](specs/08-wallet-core-part3.md). Source: wallet/wallet.go:4955-4981

isRelevantTx hashes the final data push of each signature script and looks it up as an address id, so any P2PKH spend from a wallet address matches via its pubkey even if the output is unknown; outputs paying directly to a wallet pubkey are not relevant because pubkey addresses do not implement Hash160er, although processTransactionRecord would credit them.

## QK-0215 — Partial pre-DCP0005 validation reports a hash mismatch

Spec: [08](specs/08-wallet-core-part3.md). Source: wallet/wallet.go:5235-5266; validate/validate.go:107-130

On a wallet not yet synced to the activation height ValidatePreDCP0005CFilters hashes whatever filters exist and returns 'hash of provided cfilters ... does not match expected hash ...' rather than a not-synced error; on simnet/regnet it fails with 'The current network does not have pre-DCP0005 cfilters'.

## QK-0216 — MixSplitLimit zero throttles all mixing

Spec: [08](specs/08-wallet-core-part3.md). Source: wallet/wallet.go:5374-5384, 5418; internal/loader/loader.go:259-274; wallet/mixing.go:240-246, 388-395

Open builds one semaphore channel of capacity Config.MixSplitLimit per split point; the loader's CreateNewWallet path omits the field, so a wallet created and used in the same process gets capacity 0 and every mix request returns the throttled error. Open also panics (not errors) on a duplicate deployment id.

## QK-0217 — Map-ordered results and misspelled names

Spec: [08](specs/08-wallet-core-part3.md). Source: wallet/wallet.go:5556-5633; internal/rpc/rpcserver/server.go:3855-3861

GetVSPTicketsByFeeStatus returns hashes in Go map order; GetCoinjoinTxsSumbByAcct returns a map that gRPC flattens in map order; the misspellings 'Sumb' and 'Outputsp' are part of the Go and gRPC API names. The coinjoin 'sum' is a count of outputs, not an amount.

## QK-0218 — Dead Live branch and SPV ticket listing

Spec: [08](specs/08-wallet-core-part3.md). Source: wallet/wallet.go:2899-2901, 5715-5721; wallet/tickets.go:44-73

GetTickets without an RPC client never yields TicketStatusLive, so that case in ForUnspentUnexpiredTickets is unreachable and unmined tickets are excluded; LiveTicketHashes with a nil querier (SPV) returns no matured tickets at all and treats unmined tickets as immature.

## QK-0219 — Re-publishing a known transaction re-notifies

Spec: [08](specs/08-wallet-core-part3.md). Source: wallet/wallet.go:5124-5157; wallet/udb/txunmined.go:26-34; wallet/chainntfns.go:450-456

PublishTransaction on a transaction already stored unmined runs processTransactionRecord again (InsertMemPoolTx returns nil for an existing unmined record) and fires a second unmined notification; if it is already mined it logs a warning, still broadcasts, and a failed broadcast triggers an abandon attempt that fails with Invalid (logged at WARN).

## QK-0220 — Nested read transactions can deadlock under bbolt

Spec: [08](specs/08-wallet-core-part3.md). Source: wallet/wallet.go:5741, 5756-5762, 5799; bbolt@v1.3.12/db.go:433, 735

UnprocessedTickets, ProcessedTickets and RPC callbacks given to ForUnspentUnexpiredTickets open new Views (VSPFeeHashForTicket, IsVSPTicketConfirmed, VSPHostForTicket) while the GetTickets View is open on the same goroutine. bbolt read transactions hold the mmap read lock for their lifetime and a writer growing the mapping takes the write lock, so a writer queued between the outer and inner read deadlocks all three.

## QK-0221 — Go-runtime dependence

Spec: [08](specs/08-wallet-core-part3.md). Source: wallet/notifications.go:499-523, 740-745, 927-971; wallet/udb/txdb.go:566

Correctness of &txHash in ConfirmationNotificationsClient.process and of the goroutine-per-client loop in notifyMainChainTipChanged depends on per-iteration loop variables (Go >= 1.22); receive times come from time.Now() and are persisted with one-second resolution; Done() must be called exactly once and removal reorders the client slice.

## QK-0222 — Second Done call is harmless despite the 'exactly once' contract

Spec: [08](specs/08-wallet-core-part3.md). Source: wallet/notifications.go:502-523

The remover goroutine finds no matching channel and closes nothing; the drainer exits at once because the channel is already closed. No double-close panic.

## QK-0223 — Confirmation block hash is looked up by height

Spec: [08](specs/08-wallet-core-part3.md). Source: wallet/notifications.go:860-872, 956-968

For confs > 0 the reported BlockHash is GetMainChainBlockHashForHeight(TxBlockHeight(hash)), not the block hash stored with the transaction record.

## QK-0224 — TransactionNotificationsClient.C is receive-only, the other three client channels are bidirectional

Spec: [08](specs/08-wallet-core-part3.md). Source: wallet/notifications.go:478, 532, 631, 687

Go callers could send on or close the removed-transaction, account and tip-changed channels; no in-tree caller does.

## QK-0225 — Misleading doc comments on PublishTransaction, SetPublished, MixingEnabled, ProcessedTickets/UnprocessedTickets and Open

Spec: [08](specs/08-wallet-core-part3.md). Source: wallet/wallet.go:5100-5102, 5635, 5532-5533, 5749-5751, 5793-5795, 5338-5339

PublishTransaction claims the transaction is not added on send failure (it is added, notified, then removed); SetPublished(false) adds the unpublished marker; MixingEnabled reports only the config flag; the ticket helpers return *VSPTicket, not hashes.

## QK-0226 — CreateSignature has no input-index bounds check

Spec: [08](specs/08-wallet-core-part3.md). Source: dcrd/txscript@v4.1.2/sighash.go:236-251

dcrd's signature hash path errors only for an unparsable script or SigHashSingle with idx >= outputs; idx >= len(TxIn) panics with AnyOneCanPay and is otherwise unchecked (UNVERIFIED by execution).

## QK-0227 — Nil dereference in isAncestorOf during header validation

Spec: [09](specs/09-chain-sync-rescan-discovery.md). Source: wallet/difficulty.go:675-709, wallet/difficulty.go:428-432, wallet/difficulty.go:482-494

ancestorHeaderAtHeight returns (nil, nil) when the requested height exceeds the target's height; isAncestorOf then calls BlockHash() on the nil header and panics (dcrd's equivalent returns false). Triggered when a header reaches the cached-candidate-anchor step (version >= min blake3 version, height >= first possible activation height, blake3 hash <= its own claimed target, which is not range-checked) while its parent is below the cached anchor height, e.g. a header forking off the main chain below the anchor. Reachability from an untrusted SPV peer is unverified. The Rust port must return 'not an ancestor'.

## QK-0228 — Positional-only PoW difficulty; PoW hash not recomputed

Spec: [09](specs/09-chain-sync-rescan-discovery.md). Source: wallet/difficulty.go:363-377, wallet/difficulty.go:883-893, spv/backend.go:327-338

A header is accepted if its bits match EMA or ASERT against any eligible candidate anchor; the 'contextual checks that happen later' mentioned in the comment do not exist in the wallet. The blake256 PoW check uses the caller-supplied BlockNode.Hash (SPV takes it from the next header's PrevBlock) rather than hashing the header, and either blake256 or blake3 PoW passes regardless of which difficulty algorithm matched.

## QK-0229 — Cfilter-based account discovery is not a binary search

Spec: [09](specs/09-chain-sync-rescan-discovery.md). Source: wallet/discovery.go:516-534

After a linear scan of the first 2*accountGapLimit accounts, the fallback loop only lowers hi (lo stays 2), probing a fixed halving sequence of segments. With account gap limit 10 the low end of the sequence is accounts [130,140), [70,80), [40,50), [20,30); used accounts in unprobed segments >= 20 are never discovered on SPV. The dcrd exists-address variant is a real bisect.

## QK-0230 — cacheMissingCommitments returns early instead of continuing

Spec: [09](specs/09-chain-sync-rescan-discovery.md). Source: wallet/discovery.go:82-84

When a 500-hash chunk contains no uncached blocks the function returns nil, leaving later chunks unfetched; their blocks then look like they contain no scripts. Reachable only with more than 500 matching blocks during account discovery.

## QK-0231 — Reorg back onto a detached block without wallet transactions stalls the marker (RPC mode)

Spec: [09](specs/09-chain-sync-rescan-discovery.md). Source: wallet/chainntfns.go:160-166, wallet/chainntfns.go:212-239, wallet/sidechains.go:88-102, chain/sync.go:874-883

ChainSwitch sets detached nodes' RelevantTxs to removedTxs[hash], which is nil for blocks with no wallet transactions. If such a node is re-attached later (the RPC syncer's AddBlockNode rejects the fresh blockconnected node as a duplicate and keeps the old one), the marker loop stops at it and the gap rule blocks all later advances; RescanPoint stays non-nil (no incremental address watching, ticket buyer idle) until a rescan runs. SPV overwrites RelevantTxs and is unaffected.

## QK-0232 — Discovered accounts' address buffers start at lastUsed = 0

Spec: [09](specs/09-chain-sync-rescan-discovery.md). Source: wallet/discovery.go:811-815, wallet/discovery.go:919-935, wallet/cointype.go:53-55

Buffers for accounts created by discovery use zero values rather than the 0xFFFFFFFF 'none used' sentinel; for a branch with no discovered usage the next address handed out is child 1, not child 0, until the wallet is reopened.

## QK-0233 — Treasury vote bytes depend on Go map iteration

Spec: [09](specs/09-chain-sync-rescan-discovery.md). Source: wallet/wallet.go:274-289, wallet/chainntfns.go:806-824

GetAllTSpends iterates a map, so the order of (tspend hash, vote) pairs in the vote's OP_RETURN output, and therefore the vote transaction hash, is nondeterministic when more than one tspend is voted.

## QK-0234 — RPC account discovery ignores the gapLimit argument

Spec: [09](specs/09-chain-sync-rescan-discovery.md). Source: wallet/discovery.go:627, wallet/discovery.go:649

existsAddrIndexFinder.branchUsed derives wallet.gapLimit (configured) addresses while findLastUsedAddress uses the gapLimit passed to DiscoverActiveAddresses.

## QK-0235 — ChainSwitch side effects are not atomic with the DB transaction

Spec: [09](specs/09-chain-sync-rescan-discovery.md). Source: wallet/chainntfns.go:261-316, wallet/chainntfns.go:469-473, wallet/chainntfns.go:505-517

lockedOutpoints deletions, queued notifications, VSP RemoveTicket calls and multisig-script filter loads happen inside the update closure and are not undone on failure. After a successful commit a watchHDAddrs error makes ChainSwitch return an error without the detached chain, without pruning the forest and without tip notifications. vspClients is read without its mutex.

## QK-0236 — Rescan saver goroutine leak and blocking progress send

Spec: [09](specs/09-chain-sync-rescan-discovery.md). Source: wallet/rescan.go:296-348, internal/rpc/rpcserver/server.go:472-481

If NetworkBackend.Rescan fails, the saver goroutine still commits the pending partial batch and then blocks forever on an unbuffered channel. Progress sends are not cancellable: a consumer that stops reading leaves the rescan goroutine blocked once the 1-element buffer fills.

## QK-0237 — Vote log pairs the wrong ticket; zero-vote publish

Spec: [09](specs/09-chain-sync-rescan-discovery.md). Source: wallet/chainntfns.go:735-893

After nil votes are removed by swap-compaction, the 'Voting on block' log indexes ticketHashes and usedVoteBits by the compacted position, so it can name a different ticket/bits. A failed build view is logged, not returned. PublishTransactions is called with zero transactions when nothing was built. GetAllTSpends (nested DB view) runs once per ticket.

## QK-0238 — Forest nondeterminism, root-only best chain, and consistency panic

Spec: [09](specs/09-chain-sync-rescan-discovery.md). Source: wallet/sidechains.go:147-186, wallet/sidechains.go:206-223, wallet/sidechains.go:274-303, chain/sync.go:281-322

Equal-work tips and the order of trees from pruneChain follow Go map iteration; when all tips are invalid best() returns only the root; AddBlockNode panics ('sidechain forest internal consistency error') if a waiting orphan's height is not parent+1; the RPC header sync adds nodes before validating them.

## QK-0239 — Rescan progress doc vs code; first rescan never logs

Spec: [09](specs/09-chain-sync-rescan-discovery.md). Source: wallet/rescan.go:208-218, wallet/rescan.go:346-348

The comment says progress starts with the start height; the code reports only the last height of each 2000-block batch. The first rescan in a process lifetime does not log discovered transactions (flag is set after being read).

## QK-0240 — DCP0001 gate in ChainSwitch uses the pre-switch tip height

Spec: [09](specs/09-chain-sync-rescan-discovery.md). Source: wallet/chainntfns.go:246

The decision whether the next ticket price is computable for unmined pruning tests the tip height read before the switch, not the new tip.

## QK-0241 — AddTransaction demotes non-main-chain block hashes to unmined; no relevance check before insert

Spec: [09](specs/09-chain-sync-rescan-discovery.md). Source: wallet/chainntfns.go:365-380, wallet/chainntfns.go:446-462

A non-nil blockHash that is not a main-chain block silently makes the transaction unmined. processTransactionRecord inserts the record before examining any input or output, so unrelated transactions passed by a caller are stored.

## QK-0242 — validate leniency

Spec: [09](specs/09-chain-sync-rescan-discovery.md). Source: validate/validate.go:74-89, spv/backend.go:82-85

Callers accept a block when either the separate-roots or the combined-root merkle check passes, with no height gate; cfilters below the DCP0005 height are accepted without an inclusion proof.

## QK-0243 — Wrong op string and unused API

Spec: [09](specs/09-chain-sync-rescan-discovery.md). Source: wallet/sidechains.go:363, wallet/rescan.go:532-581

FullSideChain errors carry op 'wallet.EvaluateBestChain'. SetBirthStateAndScan does not scan, returns one error unwrapped, mutates its argument and has no in-repository caller.

## QK-0244 — Testnet ASERT clamp compares compact encodings

Spec: [09](specs/09-chain-sync-rescan-discovery.md). Source: wallet/difficulty.go:335-337, wallet/difficulty.go:259-263

The testnet3 maximum-difficulty clamp on the ASERT path compares uint32 compact bits numerically (not targets) and applies at every height, unlike the EMA path which is gated at height 962928.

## QK-0245 — Mixed ancestor walks

Spec: [09](specs/09-chain-sync-rescan-discovery.md). Source: wallet/difficulty.go:581-600, wallet/difficulty.go:675-695

sumPurchasedTickets and findPrevTestNetDifficulty follow PrevBlock hashes below chain[0] while ancestorHeaderAtHeight and the EMA window walk use main-chain heights; they agree only when the starting header's ancestry is the main chain (NextStakeDifficultyAfterHeader with a stored side-chain header mixes them).

## QK-0246 — Inclusive watch ranges and duplicate filter entries

Spec: [09](specs/09-chain-sync-rescan-discovery.md). Source: wallet/wallet.go:1003-1036, spv/backend.go:423-427

watchHDAddrs loads count+1 children on first watch and re-sends the lastWatched child on each incremental watch; the SPV backend appends every address to its cfilter match-entry list without deduplication.

## QK-0247 — gRPC CreateWallet tests set_birth_time twice

Spec: [09](specs/09-chain-sync-rescan-discovery.md). Source: internal/rpc/rpcserver/server.go:2713-2752

The second condition should test set_birth_height. set_birth_time therefore stores a height-based birthday at birth_height (time discarded after the 24h validation), and set_birth_height alone falls through to the default SetFromHeight with height 0.

## QK-0248 — --watchlast leaves never-used branches unwatched

Spec: [09](specs/09-chain-sync-rescan-discovery.md). Source: wallet/wallet.go:931-932,1003-1006,1014

For a branch with no returned address end = gapLimit-1, the unsigned end - gapLimit wraps, start becomes 0xFFFFFFFF - watchLast > end, and the load loop does not run; lastWatched is still set to end.

## QK-0249 — Discovered account name collision aborts discovery

Spec: [09](specs/09-chain-sync-rescan-discovery.md). Source: wallet/discovery.go:785-789; wallet/udb/addressmanager.go:2100-2105

Accounts are named account-<n> unconditionally; an existing account with that name makes NewAccount fail with kind Exist and the single update creating the whole batch rolls back.

## QK-0250 — filterBlocks leaks workers on a database error

Spec: [09](specs/09-chain-sync-rescan-discovery.md). Source: wallet/discovery.go:397-405

The cfilter batch read error path returns without closing the work channel, leaving runtime.NumCPU() goroutines blocked forever.

## QK-0251 — AddTransaction rejections keyed on blockHash == nil

Spec: [09](specs/09-chain-sync-rescan-discovery.md). Source: wallet/chainntfns.go:336-380; internal/rpc/jsonrpc/methods.go:459-479

Unmined-tspend and orphan-vote rejections run before the block lookup; with a non-nil hash of a non-main-chain block the transaction is recorded as unmined without them.

## QK-0252 — Backend Blocks results trusted by position and length

Spec: [09](specs/09-chain-sync-rescan-discovery.md). Source: wallet/discovery.go:85-91,342-354

Discovery ranges over the returned slice and indexes the request by position; shorter results are silently accepted, longer ones panic, and block hashes are never compared.

## QK-0253 — Last-returned child index is not persisted by address allocation

Spec: [10](specs/10-tx-construction.md). Source: wallet/addresses.go:468-502,623-636; wallet/wallet.go:5471-5480; wallet/udb/addressmanager.go:1949-1970

nextAddress/UpdateDB write only address rows and never call MarkReturnedChildIndex (dropped by commit bea91d63). Since the cursor is initialised at open as LastReturned - LastUsed, addresses handed out but unused are re-issued after a restart, and gRPC account key counts and the watchHDAddrs upper bound do not advance on getnewaddress. Only MarkUsedChildIndex and SyncLastReturnedAddress write extret/intret.

## QK-0254 — Change-address gap policy differs by call site

Spec: [10](specs/10-tx-construction.md). Source: wallet/createtx.go:134-139,413-426,671-676,821-822,1037-1043

authorTx (SendOutputs) uses gapPolicyWrap; the default change source of Wallet.NewUnsignedTransaction, the treasury change source and the multisig change source leave the policy at its zero value (gapPolicyError), so ConstructTransaction, fundrawtransaction without a change address, sendtotreasury and sendtomultisig fail with the gap-limit Policy error once gapLimit unused internal addresses were issued; mixedSplit and consolidate use gapPolicyIgnore.

## QK-0255 — Lock-order inversion between address buffers and DB write transaction

Spec: [10](specs/10-tx-construction.md). Source: wallet/addresses.go:521-552; wallet/discovery.go:893-938

Self-contained nextAddress opens a write transaction then locks addressBuffersMu; DiscoverActiveAddresses locks addressBuffersMu then opens a write transaction (with a comment claiming that is the usual order). Concurrent allocation during discovery can deadlock.

Disposition (project owner, 2026-11-19; ADR-0008 Open point 2): not reproduced. dcr-purse takes the database write transaction before the address-buffer lock at every site, the order dcrwallet documents and uses everywhere but one; `DiscoverActiveAddresses` opens its transaction before the lock. The inversion is a possible deadlock in the Go, and no result an outside observer sees depends on it. To be listed under PARITY.md's deliberate divergences when the address-buffer code is built (Phase 4).

## QK-0256 — SyncLastReturnedAddress does not advance the in-memory cursor on a branch with no used address

Spec: [10](specs/10-tx-construction.md). Source: wallet/addresses.go:872-874

The cursor is only set when lastUsed != 0xffffffff; the database is updated regardless, so the documented 'next address is child+1' holds only after restart for fresh branches.

## QK-0257 — CurrentAddress returns the next address to be issued, not the most recently issued

Spec: [10](specs/10-tx-construction.md). Source: wallet/wallet.go:1948-1973

It derives lastUsed + 1 + cursor without advancing, without gap policy, and without skipping invalid children, contradicting its doc comment.

## QK-0258 — Discovery-created account buffers start with lastUsed = 0

Spec: [10](specs/10-tx-construction.md). Source: wallet/discovery.go:811-815

Buffers for accounts created during account discovery use Go zero values instead of 0xffffffff, so a branch with no on-chain usage issues child 1 first until the next restart.

## QK-0259 — authorTx leaves inputs unreserved between authoring and recording

Spec: [10](specs/10-tx-construction.md). Source: wallet/createtx.go:389-400,435-439,516-517

Inputs are added to and removed from the lock set while lockedOutpointMu is held continuously (unobservable). Between authorTx returning and recordAuthoredTx nothing reserves them, so two concurrent sends can pick the same UTXO and the second fails at record/publish.

## QK-0260 — Dust remainder inflates the fee; sweeps can yield zero-output transactions

Spec: [10](specs/10-tx-construction.md). Source: wallet/txauthor/author.go:126-157

When change is zero or dust the loop drops it without recomputing the fee for the smaller size; with no caller outputs (sweep) a dust remainder returns a transaction with no outputs and no error.

## QK-0261 — Wallet.NewUnsignedTransaction rejects imported-xpub and imported-voting accounts

Spec: [10](specs/10-tx-construction.md). Source: wallet/createtx.go:101-109,138

The account existence check compares against LastAccount, so account numbers above 0x7fffffff fail with NotExist 'missing account'; authorTx has no such check. The default change source there also uses context.Background().

## QK-0262 — Misused size constants produce observable fee over-estimates

Spec: [10](specs/10-tx-construction.md). Source: wallet/wallet.go:1665-1666; internal/rpc/jsonrpc/methods.go:4270,4316

PurchaseTickets' fee-split estimate passes RedeemP2PKHInputSize (166, a full input size) as a sigScript size; JSON-RPC spendoutputs passes 166 per input and declares a change ScriptSize of P2PKHOutputSize (36). Fees are higher than the formula intends.

## QK-0263 — Panics reachable from callers

Spec: [10](specs/10-tx-construction.md). Source: internal/rpc/jsonrpc/methods.go:4389; wallet/txauthor/author.go:171-175; wallet/addresses.go:306-318; cmd/movefunds/main.go:189

JSON-RPC spendoutputs calls RandomizeChangePosition unconditionally; with no change output RandomizeOutputPosition indexes -1 and panics. xpubAddress.PubKey panics on derivation error. cmd/movefunds passes a nil ChangeSource, which panics at ScriptSize().

## QK-0264 — sendtomultisig legacy path

Spec: [10](specs/10-tx-construction.md). Source: wallet/createtx.go:584-594,625-631,659-723

Requires amount + 0.5 DCR selectable on mainnet/testnet3 (keyed on network magic, including retired testnet2 0x48e7a065); estimates P2PKH inputs at 110 bytes; adds change without a dust test; no script self-verification; returns CreatedTx with unset ChangeAddr/ChangeIndex/Fee; publishes inside the open write transaction; records only the multisig output row, not the transaction, so spent inputs stay selectable until the transaction returns from the network.

## QK-0265 — Consolidation is not randomised and publishes inside the write transaction

Spec: [10](specs/10-tx-construction.md). Source: wallet/createtx.go:785-920

consolidate always takes the lowest-keyed confirmed outputs (minconf 1), fails with InsufficientBalance when maxNumIns <= 0, publishes before commit, and does not reload the backend filter.

## QK-0266 — signP2PKHMsgTx silently leaves inputs unsigned

Spec: [10](specs/10-tx-construction.md). Source: wallet/createtx.go:1998-2006

An input whose previous script does not yield exactly one address is skipped without error; non-P2PKH single-address scripts return kind Bug.

## QK-0267 — Two disagreeing eligibility implementations

Spec: [10](specs/10-tx-construction.md). Source: wallet/udb/txmined.go:3196-3332; wallet/createtx.go:1750-1822,1837-1906

The store input source decides by recorded stake opcode/coinbase flag, accepts P2PK, rejects zero-value credits; findEligibleOutputs* decide by script classification, reject P2PK, accept zero-value credits and stake-tagged P2SH credits that signP2PKHMsgTx then fails on.

## QK-0268 — findEligibleOutputsAmount discards its random-phase picks

Spec: [10](specs/10-tx-construction.md). Source: wallet/createtx.go:1908-1957

If the random budget (numUnspent/2 tries, draws with replacement) does not satisfy the target, all picks are thrown away and selection restarts from a shuffled full scan.

## QK-0269 — StakePoolTicketFee float and global-cache behaviour

Spec: [10](specs/10-tx-construction.md). Source: wallet/txrules/poolfees.go:25-26,43-44,54-71; wallet/txrules/doc.go:27

floor(poolFee*100.0) in float64 loses a unit for values such as 0.29 or 1.15; a single process-wide subsidy cache is bound to the first call's params; the parameter named relayFee is the ticket's absolute fee; doc.go's description of the subsidy adjustment does not match the code.

## QK-0270 — Gap limit 0 is unguarded in this scope

Spec: [10](specs/10-tx-construction.md). Source: wallet/addresses.go:572-604

With gapLimit 0, error policy always fails, wrap always returns lastUsed+1, and ignore divides by zero and panics. No validation was found in config.go (unverified elsewhere).

## QK-0271 — watchHDAddrs ordering and state

Spec: [10](specs/10-tx-construction.md). Source: wallet/wallet.go:931-932,957-1062

Marks branches as watched before loading on first watch, re-sends the boundary address on later calls, iterates accounts in Go map order and issues LoadTxFilter calls concurrently, so filter-load order is nondeterministic; bound uses the stale DB last-returned value.

## QK-0272 — individualSplit with DontSignTx drops change-address row updates

Spec: [10](specs/10-tx-construction.md). Source: wallet/createtx.go:1137-1149; wallet/wallet.go:1673-1684

authorTx collects deferred address-row updates but they are only written by recordAuthoredTx, which is skipped when DontSignTx is set; the in-memory cursor still advances. PurchaseTickets writes the fee-split change updates twice.

## QK-0273 — Dead code

Spec: [10](specs/10-tx-construction.md). Source: wallet/createtx.go:120-126,242-247,1785,2056-2059; wallet/addresses.go:256-382,634-636

Revocation branch of signVoteOrRevocation (no caller passes isVote=false), xpubAddress's unexported stakeAddress methods, the 'no method to update DB with addresses' branch, the InsufficientBalance swallow in the select-all wrapper, CreatedTx.Fee/ChangeAddr, and the treasury-add case in the eligibility switch.

## QK-0274 — PrepareRedeemMultiSigOutTxOutput under-estimates and forces version 0

Spec: [10](specs/10-tx-construction.md). Source: wallet/multisig.go:38,46-50,81-90

Assumes a 110-byte sigScript per input regardless of M and N and creates the output with script version 0 regardless of the destination; FetchP2SHMultiSigOutput ignores one address-construction error and returns another unwrapped.

## QK-0275 — Revocation creation removed

Spec: [11](specs/11-staking-vsp.md). Source: wallet/chainntfns.go:927-933; wallet/createtx.go:2053-2059

RevokeOwnedTickets always returns nil (DCP0009 auto-revocations); the revocation branch of signVoteOrRevocation is dead code. The port needs no revocation builder, but the API entry point must remain a no-op.

## QK-0276 — payfee is sent twice; confirmation progresses via ErrFeeAlreadyReceived

Spec: [11](specs/11-staking-vsp.md). Source: wallet/vsp.go:235-267; wallet/feepayment.go:434-475

Process submits payfee synchronously, and the reconcile timer armed at job creation submits the same fee tx again. Only the ErrFeeAlreadyReceived branch marks the fee tx published and moves to confirmPayment. The sequence of HTTP calls seen by the VSP is the compatibility surface.

## QK-0277 — No automatic resume of fee payments after restart

Spec: [11](specs/11-staking-vsp.md). Source: dcrwallet.go:422-432; wallet/vsp.go:118-211

Jobs and timers are in memory only. They are recreated only by --vsp.sync at startup, gRPC ProcessManagedTickets / ProcessUnmanagedTickets / SyncVSPFailedTickets, or JSON-RPC processunmanagedticket.

## QK-0278 — VSP failures during purchase are swallowed

Spec: [11](specs/11-staking-vsp.md). Source: wallet/createtx.go:1649-1659

After a ticket is published, errors from NewVSPTicket and VSPClient.Process are dropped (not returned, not logged at the call site); the purchase reports success and the ticket is left with an errored/absent vsp record.

## QK-0279 — Leaked UTXO locks on partial VSP purchase

Spec: [11](specs/11-staking-vsp.md). Source: wallet/createtx.go:1304-1314,1633

unlockCredits is set false after the first VSP ticket is published; if a later ticket fails to publish or ctx is cancelled during trickle, fee credits reserved for remaining tickets stay in the in-memory locked set until restart or explicit unlock.

Disposition (project owner, 2026-11-18; ADR-0012 Rule 7): fixed, not reproduced. When a later ticket of a VSP purchase fails to publish or the purchase is cancelled, dcr-purse releases the fee credits reserved for the tickets that were not published, instead of leaving them locked until a restart. No external observer depends on the leak. To be implemented and tested with the purchase flow (Phase 9) and listed under PARITY.md's deliberate divergences then.

## QK-0280 — Treasury key policy keys sent as raw bytes in payfee

Spec: [11](specs/11-staking-vsp.md). Source: wallet/vspticket.go:213-234; internal/rpc/rpcserver/server.go:4206

VSPTicket.TreasuryKeyPolicy uses the 33 raw key bytes (as a Go string) as the JSON map key instead of hex; Go's encoder emits U+FFFD for invalid UTF-8. All RPC-driven paths hex-encode. Tickets with a per-ticket treasury key policy send a malformed payfee.

## QK-0281 — VSP client registry keyed by raw URL, first caller wins

Spec: [11](specs/11-staking-vsp.md). Source: wallet/vspclientloader.go:12-40; wallet/vsp.go:67-81

Wallet.VSP returns an existing client for the same URL string without comparing pubkey or policy, so the first caller's FeeAcct/ChangeAcct/MaxFee apply to all later users. The DB stores url.Parse(...).String(); LookupVSP(storedHost) fails if normalization changed the string.

## QK-0282 — Jobs can idle forever

Spec: [11](specs/11-staking-vsp.md). Source: wallet/feepayment.go:412-420,442-455,470-473,604-610

reconcilePayment returns without rescheduling when makeFeeTx fails (e.g. wallet locked, fee too high, insufficient funds) or a status write fails; confirmPayment schedules nothing on an unknown feetxstatus. The idle job stays in the jobs map, and ProcessUnprocessedTickets/ProcessManagedTickets skip tickets that already have a job.

## QK-0283 — fee = -1 sentinel and stale fee data after ErrFeeExpired

Spec: [11](specs/11-staking-vsp.md). Source: wallet/feepayment.go:160-168,323-335,535-546

A job rebuilt from an existing fee tx sets fee=-1; makeFeeTx only re-requests a fee address when fee==0. After ErrFeeExpired the fee tx is cleared but fee/feeAddr are kept, so the old address/amount is reused without a new feeaddress call; with fee=-1 and nil feeAddr CreateVspPayment would call a method on a nil interface (crash on the timer goroutine). Unverified by execution.

## QK-0284 — Mixing with ticket buyer Limit 0 buys nothing

Spec: [11](specs/11-staking-vsp.md). Source: ticketbuyer/tb.go:167-171,198-200,282-286

When mixing, multiple = cfg.Limit and the purchase loop runs 'multiple' times, so Limit 0 starts no purchases; the later 'limit == 0 && mixing -> buy = 1' branch is unreachable. CLI default is 1 but gRPC RunTicketBuyer passes the request limit through.

## QK-0285 — Split transaction recorded and published twice

Spec: [11](specs/11-staking-vsp.md). Source: wallet/createtx.go:1137-1146,1459-1482

individualSplit records and publishes the split (and the mixing client publishes the mixed one); purchaseTickets then runs processTransactionRecord and PublishTransactions on it again, producing duplicate notifications and a duplicate broadcast.

## QK-0286 — Inconsistent StakePoolTicketFee argument and wrong size constant in the UTXO-split retry

Spec: [11](specs/11-staking-vsp.md). Source: wallet/createtx.go:1297; wallet/wallet.go:1649,1665

The reservation path passes the absolute ticket fee; the errVSPFeeRequiresUTXOSplit path passes the relay fee rate, and estimates the split tx size by passing RedeemP2PKHInputSize (165) where a sigScript size (108) is expected, overestimating by 57 bytes.

## QK-0287 — Vote log lines can name the wrong ticket; winner and tspend order are random

Spec: [11](specs/11-staking-vsp.md). Source: wallet/chainntfns.go:804-824,862-888; rpc/client/dcrd/notifications.go:31-41

Nil votes are swap-removed without permuting ticketHashes/usedVoteBits, so the info log pairs vote hashes with the wrong ticket. Winners arrive in Go map order; treasury votes are appended in tspend-cache map order with no cap of 7.

## QK-0288 — Voting error handling is best-effort

Spec: [11](specs/11-staking-vsp.md). Source: wallet/chainntfns.go:858-916

A failed DB View is logged and voting continues; PublishTransactions is called even with zero votes; publish failures are ignored and votes are still recorded; lockedOutpointMu is intentionally not held.

## QK-0289 — Permissive VSP error decoding and Ed25519 panic

Spec: [11](specs/11-staking-vsp.md). Source: vspd/client/v4@v4.0.2/client.go:227-235,267; wallet/vsp.go:71-79

Any non-200 JSON body with only code/message keys (including {} or null) decodes as ErrorResponse code 0. ed25519.Verify panics if the configured pubkey is not 32 bytes; length is never validated at configuration.

## QK-0290 — No HTTP timeouts; trace logging leaks the voting key

Spec: [11](specs/11-staking-vsp.md). Source: wallet/vsp.go:76-84; vspd/client/v4@v4.0.2/client.go:189-211

http.Client.Timeout and all Transport timeouts are zero and background jobs use context.Background(), so requests can hang indefinitely. At trace level full request dumps (including the voting WIF in payfee) are logged.

## QK-0291 — SetVoteChoice hides VSP status failures

Spec: [11](specs/11-staking-vsp.md). Source: wallet/vsp.go:297-304; internal/rpc/jsonrpc/methods.go:3925-3928,4059-4062

A failed ticketstatus query (other than wallet locked) is logged and nil is returned, so setvotechoice/settreasurypolicy/settspendpolicy report success with the VSP not updated. Policy strings '', 'invalid', 'abstain' are forwarded verbatim; an empty votechoices map is sent from the treasury/tspend RPCs.

## QK-0292 — Voting address taken from the internal branch, external index bumped

Spec: [11](specs/11-staking-vsp.md). Source: wallet/createtx.go:1518-1525; wallet/addresses.go:654-669

The ticket voting address is the next branch-1 address of the voting account; signingAddressAtIdx then persists the same child index as returned on branch 0 of that account and discards the derived address.

## QK-0293 — ForUnspentUnexpiredTickets never sees unmined tickets

Spec: [11](specs/11-staking-vsp.md). Source: wallet/wallet.go:5705-5747,2888-2908

It ranges mined heights only and filters on status live/immature/unspent; without an RPC client 'live' never occurs and unmined tickets have status unmined. UnprocessedTickets/ProcessedTickets and VSP vote-choice propagation therefore skip unmined tickets.

## QK-0294 — Confirmed status can zero the stored fee hash

Spec: [11](specs/11-staking-vsp.md). Source: wallet/feepayment.go:160-168,588-591

confirmPayment writes UpdateFeeConfirmed with the job's feeHash; when the job never learned one (fee tx not found in the wallet) the stored hash is overwritten with zeros. Creating a job via the started/errored path with an existing fee tx regresses the DB status to started.

## QK-0295 — Prune hook touches VSP state for every pruned tx without locking

Spec: [11](specs/11-staking-vsp.md). Source: wallet/chainntfns.go:261-274

ChainSwitch iterates w.vspClients without vspClientsMu and calls RemoveTicket and DeleteVSPTicket for every pruned unmined transaction, not only tickets.

## QK-0296 — DontSignTx corner cases

Spec: [11](specs/11-staking-vsp.md). Source: wallet/createtx.go:999,1599-1601; wallet/wallet.go:1600-1602

mixedSplit ignores DontSignTx (it mixes, signs and publishes the split) while tickets come back unsigned; with a VSP client, errVSPFeeRequiresUTXOSplit escapes to the caller verbatim.

## QK-0297 — Doc/code disagreement on trickle delay

Spec: [11](specs/11-staking-vsp.md). Source: wallet/createtx.go:1160-1161,1487-1500

Comment says 20-60 seconds per ticket; code samples each ticket's time independently from the same instant, so all tickets are published within 60 seconds total.

## QK-0298 — Ticket buyer reads VSP client unsynchronised; window-edge skip also skips change mixing

Spec: [11](specs/11-staking-vsp.md). Source: ticketbuyer/tb.go:122-140,173-211,302

buy() reads tb.cfg.VSP from the live config instead of the snapshot and without the mutex. At height == W-2 the loop 'continue's before starting the mixChange goroutine. Purchase attempts from consecutive blocks overlap.

## QK-0299 — Ticket listings for VSP processing are all-or-nothing

Spec: [11](specs/11-staking-vsp.md). Source: wallet/wallet.go:5782-5788,5819-5825; dcrwallet.go:424-431

UnprocessedTickets and ProcessedTickets return the first NewVSPTicket error and discard every ticket built so far; --vsp.sync logs the failure and then calls ProcessManagedTickets with a nil slice.

## QK-0300 — Nil VSP policy is dereferenced

Spec: [11](specs/11-staking-vsp.md). Source: wallet/feepayment.go:271,335; wallet/vsp.go:58-94

NewVSPClient accepts a nil Policy; receiveFeeAddress and makeFeeTx read fp.policy.MaxFee/FeeAcct/ChangeAcct unconditionally and panic.

## QK-0301 — A signed 200 response with body null panics the client

Spec: [11](specs/11-staking-vsp.md). Source: wallet/vsp.go:113; vspd/client/v4@v4.0.2/client.go:59,93,115,149

json.Unmarshal leaves the response pointer nil; FeePercentage reads resp.FeePercentage and the POST methods read resp.Request.

## QK-0302 — Unvalidated tspends from dcrd can crash treasury policy resolution

Spec: [11](specs/11-staking-vsp.md). Source: wallet/wallet.go:611-615; chain/sync.go:697-704,925-931

The RPC syncer caches whatever dcrd reports as a tspend; TSpendPolicy slices SignatureScript[66:99] without a length check.

## QK-0303 — VSP fee transactions can underpay the relay fee

Spec: [11](specs/11-staking-vsp.md). Source: wallet/wallet.go:4693-4731; wallet/txrules/rules.go:43-56

CreateVspPayment requires only input >= fee; when input < fee + mining fee the negative change counts as dust and the transaction is built with a sub-relay or zero mining fee.

## QK-0304 — ProcessManagedTickets cannot recover after a seed restore as documented

Spec: [11](specs/11-staking-vsp.md). Source: wallet/vsp.go:144-147; wallet/wallet.go:5795-5814; dcrwallet.go:424-431

Its comment says it recovers VSP tracking after seed restores, but both in-tree callers pass ProcessedTickets(), which only lists tickets with an existing local vsp record.

## QK-0305 — confirmPayment doc/code disagreement

Spec: [11](specs/11-staking-vsp.md). Source: wallet/feepayment.go:556-611

The comment says a fee tx removed from the wallet schedules another reconcile; the code only inspects the VSP's feetxstatus.

## QK-0306 — task doc/code disagreement

Spec: [11](specs/11-staking-vsp.md). Source: wallet/feepayment.go:213-226

The comment says a failing method puts the payment in an errored state; only the unexported fp.err is set (never read) and the error is logged.

## QK-0307 — GetAllTSpends doc/code disagreement

Spec: [11](specs/11-staking-vsp.md). Source: wallet/wallet.go:272-289

The comment says the tspend list does not get culled; the function deletes entries whose expiry is below the tip height.

## QK-0308 — Votes are not structurally self-checked

Spec: [11](specs/11-staking-vsp.md). Source: wallet/chainntfns.go:793-913; dcrd/blockchain/stake/v5@v5.0.3/staketx.go:269-296,872-950

No CheckSSGen or script execution is run on a constructed vote; a vote rejected by the backend (for example with more than 7 treasury votes) is still recorded in the wallet.

## QK-0309 — Policy maps can exceed the VSP's 3-entry limit

Spec: [11](specs/11-staking-vsp.md). Source: vspd/types/v3@v3.0.0/types.go:54-55,67-68; wallet/feepayment.go:520-528

tspendpolicy and treasurypolicy are tagged binding max=3 in the vspd types, but payfee and setvotechoices send every per-ticket policy and gRPC SetVspdVoteChoices sends every cached tspend.

## QK-0310 — AccessConfig is dead API and Limit is signed

Spec: [11](specs/11-staking-vsp.md). Source: ticketbuyer/tb.go:167-171,282-286,318-322

No caller of TB.AccessConfig exists; a negative Limit means no limit without mixing and zero purchase goroutines with mixing.

## QK-0311 — Policy bucket decoders panic on short keys and VSP record decoders on short values

Spec: [11](specs/11-staking-vsp.md). Source: wallet/udb/treasury.go:131-134,247-251; wallet/udb/vsp.go:149-161,212-220,262-272

VSPTSpendPolicies/VSPTreasuryKeyPolicies slice k[32:] unchecked; deserializeVSPTicket/Host/PubKey index without length checks; GetVSPTicket returns a pubkey sub-slice of the database value.

## QK-0312 — Wallet.VSP documented as package-global

Spec: [11](specs/11-staking-vsp.md). Source: wallet/vspclientloader.go:10-26

The comment calls the client registry package-global; the map is a field of each Wallet.

## QK-0313 — Size-limit kept list is truncated after the last excluded PR

Spec: [12](specs/12-mixclient-protocol.md). Source: dcrd/mixing@v0.7.5/mixclient/client.go:1674-1685

When the first agreed session exceeds limits, the loop that rebuilds the kept PR list breaks as soon as the last excluded PR is met, dropping all later PRs even though the size walk accepted them. The kept list determines the next session ID, so the Rust client must reproduce this exactly.

## QK-0314 — Run PRNG object is shared across non-fresh re-formed sessions

Spec: [12](specs/12-mixclient-protocol.md). Source: dcrd/mixing@v0.7.5/mixclient/client.go:260-277,1759; mixclient/blame.go:237-296

cloneLocalPeer(false) copies the PRNG pointer, so a second Encapsulate (after a timeout re-formation or a later epoch) reads further down the stream, while blame recomputes ciphertexts from the position right after key generation and would blame such peers for 'different ciphertexts'. Derived from code, untested upstream. Restarting the stream per encapsulation pass makes Rust ciphertexts verifiable by Go peers; decision needed.

## QK-0315 — Busy loop while waiting for roots without a solver

Spec: [12](specs/12-mixclient-protocol.md). Source: dcrd/mixing@v0.7.5/mixclient/client.go:2209-2257

A non-solving client (or one whose solver died) that never receives a valid FP spins in roots() with no sleep once the SR deadline has passed, until the session leaves the mixpool or an RS appears; it also blocks shutdown. A port should wait and give up at the session deadline.

## QK-0316 — Post-DC secrets window is always waited out

Spec: [12](specs/12-mixclient-protocol.md). Source: dcrd/mixing@v0.7.5/mixclient/client.go:2052-2066; mixpool/mixpool.go:926-1058

The receive for one RS returns early only if an RS arrives, so every successful run idles until begin+5T before CMs are sent; a mainnet mix completes no sooner than about 150 s after the epoch boundary.

## QK-0317 — Revealed secrets can be re-queued for the next epoch

Spec: [12](specs/12-mixclient-protocol.md). Source: dcrd/mixing@v0.7.5/mixclient/client.go:1172-1187,1275

The exit handler regenerates keys only if the last loop iteration revealed secrets and clones from the pairing's original peer objects; after reveal -> re-form -> non-reveal failure, the request returns to pending with its published seed, SR messages and HASH160s while its CoinJoin holds later scripts. Derived from code; recommend diverging (sticky 'revealed' flag).

## QK-0318 — Dicemix can hang without a result

Spec: [12](specs/12-mixclient-protocol.md). Source: dcrd/mixing@v0.7.5/mixclient/client.go:1173-1177,1195-1199

On session exit, local peers whose PR is no longer in the mixpool are skipped silently (no result sent, not re-queued); the caller returns only when its own context ends. Recommend delivering the expired-PR error.

## QK-0319 — SR messages cannot be encoded when mtot > 512

Spec: [12](specs/12-mixclient-protocol.md). Source: dcrd/wire@v1.8.0/msgmixslotreserve.go:81-85,200-203; dcrd/mixing@v0.7.5/limits.go:32-34

The SR encoder/decoder cap the inner matrix dimension at wire.MaxMixPeers (512) although it is mtot, which mixing limits allow up to 1024; such a session passes size limiting, then every SR fails to broadcast and all peers are blamed for an SR timeout. Derived from code, not executed.

## QK-0320 — Wire Run is always 0; run arguments are dead

Spec: [12](specs/12-mixclient-protocol.md). Source: dcrd/mixing@v0.7.5/mixclient/client.go:1538,1545,1765,1852,1951,2042; keyagreement.go:130; mixpool/mixpool.go:1188-1190

Every message is built with run 0, SharedSecrets ignores its run argument, and mixpool rejects non-zero runs. Reruns are new session IDs.

## QK-0321 — Testnet parameters selected by epoch length

Spec: [12](specs/12-mixclient-protocol.md). Source: dcrd/mixing@v0.7.5/mixclient/client.go:428-436

NewClient switches stage timeout, jitter and pairing version when the mixpool epoch equals 3 minutes; simnet and regnet therefore use mainnet values (30 s, pairing version 3).

## QK-0322 — PR timing overshoot

Spec: [12](specs/12-mixclient-protocol.md). Source: dcrd/mixing@v0.7.5/mixclient/client.go:731-750

For a request arriving in the quiet window the random delay is measured from now rather than from epoch+T, so the PR can be published inside the next quiet window, contrary to the function comment.

## QK-0323 — removeUnresponsiveDuringEpoch returns at the first quiet pairing

Spec: [12](specs/12-mixclient-protocol.md). Source: dcrd/mixing@v0.7.5/mixclient/client.go:857-875

It returns instead of continuing when a pairing group has fewer than MinPeers KEs and iterates groups in Go map order, so which groups are cleaned is non-deterministic.

## QK-0324 — Blame charges decapsulation failure to the decapsulating peer

Spec: [12](specs/12-mixclient-protocol.md). Source: dcrd/mixing@v0.7.5/mixclient/blame.go:369-377

DecapsulateError.SubmittingIndex is ignored; the peer whose secrets are being recomputed is blamed ('unrecoverable secrets'). Must be reproduced for exclusion-set agreement.

## QK-0325 — Solver output trusted; one FP per local peer

Spec: [12](specs/12-mixclient-protocol.md). Source: dcrd/mixing@v0.7.5/mixclient/client.go:2160-2197; cspp/v2@v2.4.0/solver/solver.go:41-103

A solving client checks only the root count (which is how a non-splitting polynomial is detected, because the solver reports one value per irreducible factor of any degree), never reads other peers' FPs, and every local solver-capable peer publishes an identical FP.

## QK-0326 — Solver is process-global and started once

Spec: [12](specs/12-mixclient-protocol.md). Source: cspp/v2@v2.4.0/solverrpc/rpc.go:48-97,138-168; testsolver.go:18-35; config.go:739-758

sync.Once start; a failed start is never retried; SolverProcess changes after the first start are ignored; the PR capability flag is cleared for good when the child is found dead. A child that already exited passes the config probe (ErrSolverProcessExited is not treated as failure).

## QK-0327 — Null witness values in the published coinjoin

Spec: [12](specs/12-mixclient-protocol.md). Source: dcrd/mixing@v0.7.5/mixclient/coinjoin.go:133-140; dcrd/wire@v1.8.0/msgtx.go:338-347

Inputs are created with ValueIn=-1, BlockHeight=0, BlockIndex=0xffffffff and never updated, so CM transactions and the published transaction carry these values.

## QK-0328 — Shutdown waits for running sessions; client not restartable

Spec: [12](specs/12-mixclient-protocol.md). Source: dcrd/mixing@v0.7.5/mixclient/client.go:486-530,697-725

Run's tasks use a background-rooted context; cancelling Run's context only stops new sessions, and Run returns ErrStopping after in-flight sessions finish (up to one epoch). The stopping flag is never reset.

## QK-0329 — Alternate-session majority uses integer division

Spec: [12](specs/12-mixclient-protocol.md). Source: dcrd/mixing@v0.7.5/mixclient/client.go:2376-2386,2459-2466

PRs are kept when seen count >= totalPRs/2 (integer division); the KE sort comparator before it is not a valid ordering but has no effect since there is one KE per identity.

## QK-0330 — Documentation disagreements

Spec: [12](specs/12-mixclient-protocol.md). Source: dcrd/mixing@v0.7.5/message.go:33-36; dcrd/wire@v1.8.0/msgmixfactoredpoly.go:253-255; msgmixslotreserve.go:35; msgmixsecrets.go:232-240; dcrd/mixing@v0.7.5/keyagreement.go:76-80; dcroxide:crates/dcroxide-mixing/src/lib.rs:8-11

mixing.Message says FP PrevMsgs returns nil but it returns SeenSlotReserves; SR struct comment says mcount-by-peers but it is mcount-by-mtot; MsgMixSecrets writer documented as never failing but can fail; key agreement is secp256k1 ECDH, not X25519 (dcroxide-mixing's crate comment says 'X25519-style').

## QK-0331 — Logging defects

Spec: [12](specs/12-mixclient-protocol.md). Source: dcrd/mixing@v0.7.5/mixclient/client.go:1957-1959; mixclient/blame.go:260

After the DC send the wrong error variable is logged; one blame log call lacks its format argument (unreachable). Matters only if byte-exact MIXC logs are required.

## QK-0332 — resetUnmixed touches every local peer of the pairing

Spec: [12](specs/12-mixclient-protocol.md). Source: dcrd/mixing@v0.7.5/mixclient/client.go:1628-1633

After pairing completes, the coinjoin of every request in the pairing is reset, including requests not in the run that may concurrently be in the next epoch's session, so two sessions can mutate one CoinJoin object.

## QK-0333 — Blame list may contain duplicates and map-ordered entries

Spec: [12](specs/12-mixclient-protocol.md). Source: dcrd/mixing@v0.7.5/mixclient/blame.go:326-341; mixclient/client.go:1365

Shared-SR-message blame appends an identity once per occurrence and in Go map order; the too-few-peers test subtracts the list length rather than the set size.

## QK-0334 — CoinJoin.Tx() is fully signed for only one local request

Spec: [12](specs/12-mixclient-protocol.md). Source: dcrd/mixing@v0.7.5/mixclient/client.go:2102-2126; mixclient/coinjoin.go:248-285; wallet/createtx.go:1089-1092,1457-1478

Signatures from all CMs are merged only into the CoinJoin of the first non-remote peer in peer order; other local requests in the same run get nil from Dicemix but their Tx() carries signature scripts only on their own inputs (same tx hash, valid MixedIndices). dcrwallet's mixed ticket-split path records and re-publishes that returned object.

## QK-0335 — RS during the CM stage excludes its sender by timeout without a reveal

Spec: [12](specs/12-mixclient-protocol.md). Source: dcrd/mixing@v0.7.5/mixclient/client.go:2074-2093; mixpool/mixpool.go:972-1000

The mixpool counts an RS toward the expected distinct senders; the CM receive ignores ErrSecretsRevealed, so a peer sending an RS instead of a CM after the recvDCRS window is blamed for CM timeout and the re-formed session reuses all remaining peers' keys, SR messages and addresses (freshGen false).

## QK-0336 — Blame entered before remote KEs are recorded blames every remote peer

Spec: [12](specs/12-mixclient-protocol.md). Source: dcrd/mixing@v0.7.5/mixclient/client.go:1601-1610,1696-1700; mixclient/blame.go:200-205

With peer agreement already set, a pairing-completion failure of ErrSecretsRevealed returns from run before KEs are stored on remote peer objects; if all RSs then arrive, blame's first pass blames every remote peer for 'missing messages'. Derived from code, untested upstream.

## QK-0337 — Cancelled Dicemix requests keep being cloned into sessions

Spec: [12](specs/12-mixclient-protocol.md). Source: dcrd/mixing@v0.7.5/mixclient/client.go:547-571,958-963,1059-1064,1172-1201

A cancelled request's peer stays in the pending set and is cloned and counted every epoch until its PR leaves the mixpool; with no other active local request the session ends at the KE step with errNoActiveLocalPeers without sending. There is no PR withdrawal message.

## QK-0338 — SRMix uses exact (non-modular) big-integer powers

Spec: [12](specs/12-mixclient-protocol.md). Source: dcrd/mixing@v0.7.5/dcnet.go:48-57,137-147

m^(c+1) is computed with a nil modulus and reduced afterwards, giving ~130000-bit intermediates at mtot=1024; result equals the modular power. IsRoot uses modular exponentiation.

## QK-0339 — Empty SeenSecrets when the first RS receive in blame fails

Spec: [12](specs/12-mixclient-protocol.md). Source: dcrd/mixing@v0.7.5/mixclient/blame.go:80-103,154-172

blame discards the error of its first RS receive; if nothing is returned, non-triggering local peers publish RSs with empty SeenSecrets, which other peers' fallback rule classifies as a false failure accusation when their verification finds no culprit.

## QK-0340 — Wallets created through the loader cannot mix outputs until restart

Spec: [13](specs/13-wallet-mixing-integration.md). Source: internal/loader/loader.go:259-274; wallet/mixing.go:240-246, 388-395, 467

Loader.CreateNewWallet builds wallet.Config without MixSplitLimit, so the ten split semaphores are unbuffered. Every MixOutput returns 'throttled mix request for split denomination', and MixAccount selects every eligible output (maxResults 0 means unlimited) and mixes none. Mixed ticket splits are unaffected. A wallet created over gRPC and used in the same process is in this state.

## QK-0341 — --mixsplitlimit is not validated

Spec: [13](specs/13-wallet-mixing-integration.md). Source: config.go:179; wallet/mixing.go:240-246; wallet/wallet.go:5418

A value of 0 means nothing ever mixes through MixOutput. A negative value panics in make(chan struct{}, n) when the wallet is opened.

## QK-0342 — A notfound reply for a mix message disconnects the peer

Spec: [13](specs/13-wallet-mixing-integration.md). Source: p2p/peering.go:1194-1217, 1645-1707; spv/sync.go:1199-1210

receivedNotFound consults only pending transaction requests. A notfound for a mix hash requested via MixMessages is treated as unrequested and the peer is disconnected with 'received notfound for unrequested hash'. The NotExist branch in handleMixInvs and the ErrNotFound return of MixMessages are dead code. The ErrNotFound text is 'transaction not found'.

## QK-0343 — Mixed split transaction is recorded and published twice and may be returned partially signed

Spec: [13](specs/13-wallet-mixing-integration.md). Source: wallet/createtx.go:1089-1092, 1460-1482; dcrd/mixing@v0.7.5/mixclient/client.go:2100-2137; rpc/client/dcrd/calls.go:207-213; wallet/udb/txunmined.go:31-34

The mix client records and publishes the coinjoin; purchaseTickets then records and publishes cj.Tx() again. When several local requests share a session (mixed ticket buyer with limit > 1), only the first local peer's CoinJoin gets the other participants' signatures. The other requests return a transaction signed only on their own inputs, which is re-published and exposed as SplitTx to gRPC/JSON-RPC clients. It is harmless on the network because the hash is identical and dcrd's duplicate reply (-40) is treated as success. dcrd's reply when the coinjoin is already mined is UNVERIFIED.

## QK-0344 — DontSignTx does not stop a mixed split

Spec: [13](specs/13-wallet-mixing-integration.md). Source: wallet/createtx.go:999-1000

mixedSplit ignores req.DontSignTx. The coinjoin is still signed, recorded and published by the mix client before unsigned tickets are returned.

## QK-0345 — Mixed ticket buyer with limit 0 buys nothing

Spec: [13](specs/13-wallet-mixing-integration.md). Source: ticketbuyer/tb.go:167-171, 198, 282-283

Under mixing, the number of purchase goroutines per block is cfg.Limit. So --ticketbuyer.limit=0 (documented as 'no limit') and gRPC RunTicketBuyer with limit=0 start no purchases. The 'limit == 0 && mixing' branch in buy is unreachable from Run.

## QK-0346 — --mixchange alone neither requires --mixing nor obtains the passphrase

Spec: [13](specs/13-wallet-mixing-integration.md). Source: ticketbuyer/tb.go:332-334; dcrwallet.go:241-250, 289-365, 468-498

With --mixchange and no --mixing, the loop starts and demands that --mixedaccount and --changeaccount resolve, but mixChange is a no-op. With --mixchange and a locked wallet (no --pass, --promptpass or ticket buyer), every block logs 'Account mixing failed' with a Locked error.

## QK-0347 — gRPC RunAccountMixer validates nothing

Spec: [13](specs/13-wallet-mixing-integration.md). Source: internal/rpc/rpcserver/server.go:2531-2566

Unknown accounts, a branch other than 0/1, or a wallet opened without mixing only produce a per-block error log. The stream stays open and silent until the client cancels it.

## QK-0348 — --spvdisablerelaytx breaks SPV mixing without any config check

Spec: [13](specs/13-wallet-mixing-integration.md). Source: p2p/peering.go:2214-2236; config.go:146; dcrwallet.go:538

With relay disabled, the local peer disconnects any peer whose inv contains a mix (or transaction) vector, and the version message asks peers not to relay. The combination with --mixing is not rejected at start-up.

## QK-0349 — Unused mixing surface and no initial mixpool sync

Spec: [13](specs/13-wallet-mixing-integration.md). Source: p2p/peering.go:2101-2121; rpc/client/dcrd/calls.go:309-318; wallet/mixing.go:563-570

RemotePeer.PublishMixMessages (trickled inv), RPC.MixPairRequests (getmixpairrequests) and the deprecated Wallet.AcceptMixMessage have no in-tree callers. The wallet never fetches messages that existed before it connected, apart from pair requests fetched for orphan key exchanges.

## QK-0350 — MixOutput trusts its caller; JSON-RPC mixoutput always uses tree 0

Spec: [13](specs/13-wallet-mixing-integration.md). Source: wallet/mixing.go:286-311; internal/rpc/jsonrpc/methods.go:5679-5696

MixOutput does not check that the output is unspent, mature, confirmed or owned by the wallet. The outpoint parsed by JSON-RPC mixoutput always has tree 0, so a stake-tree output named over RPC yields a pair request UTXO and coinjoin input with the wrong tree. The exact dcrd rejection is UNVERIFIED.

## QK-0351 — Stake-change outputs are selected for mixing but get opcode 0

Spec: [13](specs/13-wallet-mixing-integration.md). Source: wallet/createtx.go:1883-1886; dcrd/mixing@v0.7.5/mixclient/coinjoin.go:99-103; dcrd/mixing@v0.7.5/mixpool/mixpool.go:1485-1502

findEligibleOutputsAmount admits matured OP_SSTXCHANGE P2PKH outputs, and addCoinJoinInput accepts them. CoinJoin.AddInput sets opcode 0 for them, and a UTXO-fetching mixpool then uses the plain P2PKH extractor and fails the ownership proof. Derived from the pinned source; UNVERIFIED against a running dcrd.

## QK-0352 — Mix addresses are not durable as 'returned'

Spec: [13](specs/13-wallet-mixing-integration.md). Source: wallet/addresses.go:449-452, 468-502, 623-637; wallet/udb/addressmanager.go:2058-2060; wallet/wallet.go:5471-5480

nextAddress persists only address records, not the last-returned index. After a restart the cursor is rebuilt from account properties, so addresses generated for mixes that never reached the chain are handed out again. Within a process the cursor advances even when the mix fails.

## QK-0353 — Overlapping MixAccount calls can fail on 'already locked'

Spec: [13](specs/13-wallet-mixing-integration.md). Source: wallet/mixing.go:279-284, 460-476, 492-502

MixAccount selects outputs without locking them; each MixOutput locks its own outpoint later. Two overlapping calls can select the same output, and the later MixOutput fails with 'output ... already locked', which MixAccount does not suppress and returns as its error.

## QK-0354 — sendtotreasury fails under --mixing without --mixedaccount

Spec: [13](specs/13-wallet-mixing-integration.md). Source: internal/rpc/jsonrpc/methods.go:3594-3605, 3630-3642

sendAmountToTreasury resolves the mixed account name unconditionally when mixing is enabled, so an empty name makes every call fail. sendPairs guards on non-empty names.

## QK-0355 — --changeaccount 'required when mixing' is not checked at config time

Spec: [13](specs/13-wallet-mixing-integration.md). Source: config.go:177; internal/rpc/jsonrpc/methods.go:3427-3431, 5635-5641

With --mixing and no --changeaccount, the process starts normally. JSON-RPC mixaccount/mixoutput then fail per call with 'account not found', and purchaseticket fails with 'Mixing enabled, but error on changeAccount'.

## QK-0356 — PossibleCoinJoin edge behaviours

Spec: [13](specs/13-wallet-mixing-integration.md). Source: wallet/mixing.go:517-557

The duplicate-script rejection also applies to zero-valued outputs. mixDenom and mixCount are returned non-zero with isMix=false when the half-of-outputs test fails. The result is independent of map iteration order.

## QK-0357 — Misspelled API names and unordered gRPC result

Spec: [13](specs/13-wallet-mixing-integration.md). Source: wallet/wallet.go:5599-5609; rpc/api.proto:78, 1203; internal/rpc/rpcserver/server.go:3848-3866; internal/rpchelp/helpdescs_en_US.go:677-679

GetCoinjoinTxsSumbByAcct, gRPC GetCoinjoinOutputspByAcct, the lower-case proto message coinjoinTxsSumByAcct and JSON-RPC getcoinjoinsbyacct are the public names. The gRPC response order is Go map iteration order. The mixaccount help table has a 'mixaccount-account' entry for a parameter that does not exist.

## QK-0358 — Change mix is not cancelled at stake-difficulty window boundaries

Spec: [13](specs/13-wallet-mixing-integration.md). Source: ticketbuyer/tb.go:122-150, 173-211

mixChange runs with the loop context and is started on every tip without waiting for earlier runs. Purchases use a context cancelled when a new window approaches. The change mix is skipped entirely on the tip where height+2 equals the next interval start.

## QK-0359 — Stale client pointer and nil backend hazards

Spec: [13](specs/13-wallet-mixing-integration.md). Source: wallet/wallet.go:5545-5552; wallet/mixing.go:99-118, 157-169

Wallet.mixClient keeps the stopped client between syncer runs. mixingWallet.PublishTransaction passes the current backend to Wallet.PublishTransaction without a nil check; this is unreachable in-tree only because syncers clear the backend after wallet.Run returns. SubmitMixMessage removes the message from the pool on any failure, including when the pool reported a duplicate.

## QK-0360 — SPV Syncer.Run can return 'mixing client is stopping' at shutdown

Spec: [13](specs/13-wallet-mixing-integration.md). Source: spv/sync.go:374-375, 447-465; dcrwallet.go:543-547; dcrd/mixing@v0.7.5/mixclient/client.go:697-715

The handler group's context is not derived from the caller's context. After cancellation, the first group error may be mixclient.ErrStopping from wallet.Run instead of a context error. The process loop treats any error as a clean stop when its own context is done.

## QK-0361 — Go-runtime-dependent choices

Spec: [13](specs/13-wallet-mixing-integration.md). Source: wallet/mixing.go:342, 388-395, 483-510; spv/backend.go:499-511; spv/sync.go:1253-1256; wallet/createtx.go:1076

Several outcomes depend on the Go runtime: the random pick between a done context and a free semaphore slot; which error MixAccount returns when several outputs fail; peer iteration order in SPV PublishMixMessages; the order of missing pair request hashes requested; and uint32 truncation of amount/mixValue and of req.Count.

## QK-0362 — Address manager only works for the first Syncer.Run in a process

Spec: [14](specs/14-spv-p2p.md). Source: dcrwallet.go:534-545; dcrd/addrmgr/v3@v3.0.0/addrmanager.go:609-639

spvLoop creates one addrmgr.AddrManager and reuses it across syncer restarts. Start/Stop are guarded by never-reset counters, so on the 2nd+ run Start is a no-op (no reload, no 10-minute dump loop) and Stop only logs 'Address manager is already in the process of shutting down'. After the first restart (e.g. NoPeers) peers.json is never written again until the process restarts.

## QK-0363 — gRPC SpvSync ignores --proxy and --spvdisablerelaytx

Spec: [14](specs/14-spv-p2p.md). Source: internal/rpc/rpcserver/server.go:3014-3016 vs dcrwallet.go:536-538

The SpvSync handler builds its LocalPeer without SetDialFunc or SetDisableRelayTx, so peers and seeders are dialled directly even when a SOCKS proxy is configured; the CLI path sets both.

## QK-0364 — No ban list

Spec: [14](specs/14-spv-p2p.md). Source: p2p/peering.go:1312-1318, 2154-2162; spv/sync.go:626-643

Misbehaving peers are only disconnected; their address stays in the address manager and is retried subject to an in-memory per-address backoff that is lost when Run restarts. DynamicBanScore is used only for oversized getdata (len*100/50000) and orphan header announcements (+10).

## QK-0365 — Any unknown or undecodable message disconnects the peer

Spec: [14](specs/14-spv-p2p.md). Source: p2p/peering.go:794-853; dcrd/wire@v1.8.0/message.go:233-236

The read loop treats every wire.ReadMessage error as fatal, including unknown commands, `reject` (not a known command in wire v1.8.0) and addrv2 below pver 12. dcrd's own peer code tolerates unknown commands.

## QK-0366 — notfound for a mix message or a block disconnects the peer

Spec: [14](specs/14-spv-p2p.md). Source: p2p/peering.go:1194-1217, 1639-1644; spv/sync.go:1199-1210

receivedNotFound consults only the transaction request table and ignores the inv type. MixMessages documents an ErrNotFound-with-nil-entries result that can never occur; the syncer's NotExist branch for mix messages is dead code. A block-typed notfound carrying a pending tx hash resolves that tx request.

## QK-0367 — LRU capacity is limit+1

Spec: [14](specs/14-spv-p2p.md). Source: lru/cache.go:47-54; lru/map.go:45-59

Eviction happens only when len > limit before insert, so InvsSent holds 5001 hashes and seenTxs/seenMixMsgs 2001. lru.Map.Add also keeps the old value for an existing key, and lru.Map has no users.

## QK-0368 — Remote verack never required; nonce never checked

Spec: [14](specs/14-spv-p2p.md). Source: p2p/peering.go:580-616

The wallet sends verack after reading the remote version and proceeds; the remote verack is just ignored by the read loop when it arrives. No self-connection detection.

## QK-0369 — Per-message goroutine dispatch gives no handler ordering

Spec: [14](specs/14-spv-p2p.md). Source: p2p/peering.go:802-850

Every received message is handled on its own goroutine, so two headers announcements from one peer, or a tx followed by notfound, can be processed in either order.

## QK-0370 — waitForRemote busy-loops and defeats the cfilter watchdog

Spec: [14](specs/14-spv-p2p.md). Source: spv/sync.go:742-773; spv/backend.go:192-205

With waitEligible=true, peers connected and none eligible, the loop re-locks and rescans without sleeping or checking the context, so the 2-minute cfilter watchdog cannot fire until the peer set changes.

## QK-0371 — forRemotes holds the peer-map mutex across network sends

Spec: [14](specs/14-spv-p2p.md). Source: spv/sync.go:716-729; spv/backend.go:469-474; p2p/peering.go:2126-2135

PublishTransactions/PublishMixMessages call SendMessage (up to 30 s per peer, and it does not notice a dead peer) under remotesMu, blocking getpeerinfo, walletinfo, peer registration and peer selection; iteration stops at the first failing peer so later peers (random order) get nothing and the publish reports failure.

## QK-0372 — Stall timers differ per request type

Spec: [14](specs/14-spv-p2p.md). Source: p2p/peering.go:1457-1497, 1605-1631, 1813-1862

Transactions/mix: one 30 s budget for the whole batch. Blocks: 30 s per block in request order. Pipelined cfilters: 30 s from when each request was queued. Headers/batched cfilters/init state: 30 s from the call including queue wait.

## QK-0373 — Unsolicited pong disconnects on the next ping cycle

Spec: [14](specs/14-spv-p2p.md). Source: p2p/peering.go:945-971

A pong with no pending ping sits in the 1-slot buffer and is consumed by the next ping exchange, failing the nonce comparison ('pong contains nonmatching nonce').

## QK-0374 — Checkpoint enforced only during initial sync

Spec: [14](specs/14-spv-p2p.md). Source: spv/backend.go:340-342; spv/sync.go:1465-1486

BadCheckpoint marking (testnet3 height 962928) is applied in getHeaders but not in handleBlockAnnouncements.

## QK-0375 — Initial sync can finish after a failed final chain switch

Spec: [14](specs/14-spv-p2p.md). Source: spv/sync.go:1667-1711, 1786-1817

If stage 3 fails to attach the last real batch it disconnects the source peer and invalidates, but once stage 1 has emitted the done batch the pipeline still completes and the rescan runs from the older tip; missing blocks arrive later via announcements.

## QK-0376 — Announcement handling refetches every filter under a global lock

Spec: [14](specs/14-spv-p2p.md). Source: spv/sync.go:1411-1559

Each announcement producing a better chain requests one getcfilterv2 per node of that chain from the announcing peer (never batched) and may download matching blocks, all while holding sidechainMu, serializing announcements from all peers.

## QK-0377 — 'too few headers' rule also polices announcements

Spec: [14](specs/14-spv-p2p.md). Source: p2p/peering.go:1112-1121

A sendheaders-mode headers message with <2000 headers whose last height is below the height the peer advertised at handshake disconnects the peer; a negative advertised height wraps to a huge uint32 in the comparison.

## QK-0378 — Block locators repeat the tip and may omit genesis

Spec: [14](specs/14-spv-p2p.md). Source: wallet/wallet.go:1510-1532

wallet.BlockLocators emits the tip hash twice, then h-1..h-9, then doubling gaps; genesis is included only if the descent lands exactly on height 0.

## QK-0379 — addrmgr v3.0.0 load path: timestamps/services not restored, stale counters after corrupt file

Spec: [14](specs/14-spv-p2p.md). Source: dcrd/addrmgr/v3@v3.0.0/addrmanager.go:536-556, 723-741, 766-824

TimeStamp is written but not applied on load; services are not persisted (assumed SFNodeNetwork). A failed load deletes peers.json and reset() does not zero nNew/nTried; v3.0.0 GetAddress retries blindly on empty buckets, so stale counters can make it spin while holding the manager mutex (established by reading, not run).

## QK-0380 — Seeder client laxity

Spec: [14](specs/14-spv-p2p.md). Source: p2p/peering.go:348-433

HTTP status is ignored; the decode struct is reused across objects (missing fields inherit previous values); the `services` field in the response is unused; the source address for AddAddresses is the first returned address; Go default HTTP headers (Go-http-client/1.1, gzip) are sent.

## QK-0381 — Persistent-peer host names resolved by the system resolver; failed handshakes leak the socket

Spec: [14](specs/14-spv-p2p.md). Source: p2p/peering.go:260, 624-638

ConnectOutbound calls net.ResolveTCPAddr before dialling, so --spvconnect host names leak to local DNS even with --proxy (then the original string is dialled through the proxy). On handshake failure the connection is not closed explicitly.

## QK-0382 — peerStartup strands a peer on a DB error

Spec: [14](specs/14-spv-p2p.md). Source: spv/sync.go:1930-1934

If UnminedTransactions fails, peerStartup logs and returns nil: the peer stays connected but never receives sendheaders.

## QK-0383 — Syncer task group ignores the caller's context

Spec: [14](specs/14-spv-p2p.md). Source: spv/sync.go:374-375, 401-408, 447-459

The errgroup derives from context.Background(); outer cancellation propagates only because the wallet.Run wrapper task returns walletCtx.Err().

## QK-0384 — Go map/select randomness is load-bearing

Spec: [14](specs/14-spv-p2p.md). Source: spv/sync.go:310-318, 722, 760, 1254-1256; p2p/peering.go:1837-1862

Peer selection (waitForRemote/forRemotes), TipChanged tx order, order of missing-PR hashes in follow-up getdata, and element order in peers.json all come from Go map iteration; CFiltersV2 can report a duplicate request either as an Invalid error or as a stall disconnect depending on select order.

## QK-0385 — FetchHeadersProgress reports a height in a field named count

Spec: [14](specs/14-spv-p2p.md). Source: spv/sync.go:263-267; internal/rpc/rpcserver/server.go:3077-3085

The syncer passes the tip height; the gRPC handler maps it to FetchHeadersNotification.fetched_headers_count.

## QK-0386 — Dead code and stale comments

Spec: [14](specs/14-spv-p2p.md). Source: p2p/peering.go:127,152-157,2101-2121; spv/backend.go:94,121,405,436

RemotePeer.InvsRecv is never populated; LocalPeer.receivedHeaders/receivedInitState/extaddr unused; RemotePeer.PublishMixMessages and Syncer.DisableDiscoverAccounts have no callers; lru.Map unused; pver guards for cfilterv2/sendheaders/initstate are always true with minPver 9; comments reference a wallet.Peer interface that no longer exists.

## QK-0387 — Duplicate tx hashes in one inv drop the whole inv; MempoolTxs over-reports

Spec: [14](specs/14-spv-p2p.md). Source: p2p/peering.go:1587-1593; spv/sync.go:1108-1176

Transactions() rejects a repeated hash as 'already being requested' so handleTxInvs gives up on the entire inv. MempoolTxs is called with every filter-relevant tx (including skipped manual tickets and failed inserts) and even with an empty list.

## QK-0388 — --spvconnect entries are not de-duplicated

Spec: [14](specs/14-spv-p2p.md). Source: config.go:860-865; spv/sync.go:546, 560

config uses NormalizeAddress per entry; two identical entries run two connect loops sharing one remotes-map key.

## QK-0389 — sendMessageAck cannot report a write failure

Spec: [14](specs/14-spv-p2p.md). Source: p2p/peering.go:466-473, 2139-2150

The write pump sends on the ack channel before examining the wire.WriteMessage result, so the trickler treats a failed inv write as sent and truncates the queue; the failure only surfaces through the writer task.

## QK-0390 — Never-attempted addresses have minimum selection weight

Spec: [14](specs/14-spv-p2p.md). Source: dcrd/addrmgr/v3@v3.0.0/knownaddress.go:63-79; addrmanager.go:766-824

chance() returns 0.01 when lastattempt is zero as well as when attempted within 10 minutes; GetAddress needs many factor*=1.2 rounds on a fresh book and prefers once-failed addresses over untried ones.

## QK-0391 — New bucket holds 65 addresses

Spec: [14](specs/14-spv-p2p.md). Source: dcrd/addrmgr/v3@v3.0.0/addrmanager.go:285-289

Expiry is triggered by len > 64 before insertion, so peers.json can list 65 keys in one NewBuckets element.

## QK-0392 — Discovery cannot find non-routable peers

Spec: [14](specs/14-spv-p2p.md). Source: dcrd/addrmgr/v3@v3.0.0/addrmanager.go:224-228; spv/sync.go:678-686

AddAddresses drops loopback/RFC1918/CGNAT and similar ranges; with no seeders on simnet/regnet the candidate loop silently retries 'no addresses' every 5 s forever.

## QK-0393 — Comments say DNS seeding, code does HTTPS seeding

Spec: [14](specs/14-spv-p2p.md). Source: spv/sync.go:158-159, 368-371; p2p/peering.go:167-168, 345-434

SetPersistentPeers and Run comments mention DNS seeding; only HTTPS /api/addrs requests are made. NewLocalPeer's comment claims external reachability though the wallet never listens.

## QK-0394 — Stage-2 watchdog log names the wrong peer

Spec: [14](specs/14-spv-p2p.md). Source: spv/sync.go:1749-1753

The debug line for a cfilter watchdog trigger prints the peer that supplied the batch's headers, not the peers that failed to serve filters.

## QK-0395 — getHeaders ignores BlockInMainChain error

Spec: [14](specs/14-spv-p2p.md). Source: spv/backend.go:366-374

The error is discarded; on failure the node is treated as not in the main chain and added to the sidechain forest.

## QK-0396 — Same-peer transaction request collision

Spec: [14](specs/14-spv-p2p.md). Source: p2p/peering.go:1587-1593; spv/sync.go:1036, 1118

GetInitState's tspend fetch and handleTxInvs both call rp.Transactions; requesting the same hash concurrently fails the later caller with 'already being requested'.

## QK-0397 — discoverAccounts sampled once

Spec: [14](specs/14-spv-p2p.md). Source: spv/sync.go:141-144, 1870-1874

NewSyncer reads !w.Locked() at construction; later lock state changes do not affect account discovery.

## QK-0398 — Unreachable and unused code

Spec: [14](specs/14-spv-p2p.md). Source: spv/sync.go:1936-1940; spv/backend.go:73-75, 419, 524, 569-573; p2p/peering.go:314, 343, 892, 1411, 1715

peerStartup's resend error branch and log line (PublishTransactions always returns nil), unused op constant in Syncer.Rescan, never-true nil-filter test in LoadTxFilter, dead skips in Rescan and blocksFromPeer, and RemoveHandledMessages / RemotePeer.Block / CFilterV2 / NA / InvsRecv without callers.

## QK-0399 — Good decrements nNew before failing

Spec: [14](specs/14-spv-p2p.md). Source: dcrd/addrmgr/v3@v3.0.0/addrmanager.go:913-933

Good subtracts from nNew before detecting that the address was in no new bucket; the error is ignored by the wallet. Not reachable through the wallet's call sequence.

## QK-0400 — getstakedifficulty decoded with the wrong key

Spec: [15](specs/15-dcrd-rpc-sync.md). Source: rpc/client/dcrd/calls.go:478-489; dcrd/rpc/jsonrpc/types/v4@v4.4.0/chainsvrresults.go:339-342; wallet/createtx.go:1209-1212

dcrd.RPC.StakeDifficulty reads JSON key 'nextstakedifficulty' but dcrd returns 'current' and 'next', so the RPC backend always returns (0, nil). Only reached when DCP0001 is inactive at the tip (mainnet < 149248), so effectively dead; the NetworkBackend comment says the method 'will always error'.

## QK-0401 — Gap between header sync and notifyblocks

Spec: [15](specs/15-dcrd-rpc-sync.md). Source: chain/sync.go:670-680,848-858

Blocks dcrd connects after getHeaders finishes but before notifyblocks is issued are not fetched until the next blockconnected arrives with an unknown parent; the wallet can report synced while behind for up to a block interval.

## QK-0402 — blockconnected error policy depends on prior success

Spec: [15](specs/15-dcrd-rpc-sync.md). Source: chain/sync.go:779-790

Until one blockconnected has succeeded on a connection, failures are logged ('Failed to connect block: ...') and swallowed; after the first success any failure closes the connection and restarts the whole sync.

## QK-0403 — Notified blocks skip PoW/stake-difficulty validation

Spec: [15](specs/15-dcrd-rpc-sync.md). Source: chain/sync.go:319,860-891

ValidateHeaderChainDifficulties runs only in the bulk getHeaders path; blocks from blockconnected and getMissingHeaders are attached with only the cfilter commitment proof and checkpoint check.

## QK-0404 — blockdisconnected and reorganization are ignored

Spec: [15](specs/15-dcrd-rpc-sync.md). Source: chain/sync.go:773-807,883-891

Reorgs are inferred solely from blockconnected notifications for the new chain via the sidechain forest; the wallet switches only when EvaluateBestChain finds strictly more work.

## QK-0405 — Winner order is Go map iteration order

Spec: [15](specs/15-dcrd-rpc-sync.md). Source: rpc/client/dcrd/notifications.go:31-41

WinningTickets collects ticket hashes by ranging over a map[string]*hash, so the winners slice (and the order votes are published) is random per run. A port must choose a deterministic order.

## QK-0406 — Lenient positional notification decoding

Spec: [15](specs/15-dcrd-rpc-sync.md). Source: rpc/client/dcrd/notifications.go:16-22

unmarshalArray decodes into a []any of pointers: short arrays, null elements, null params and extra elements are accepted silently and leave targets zero-valued (e.g. relevanttxaccepted [null] yields an empty MsgTx passed to AddTransaction).

## QK-0407 — Nil dereference panics on malformed dcrd replies

Spec: [15](specs/15-dcrd-rpc-sync.md). Source: rpc/client/dcrd/calls.go:281,351,379-383,497-501; rpc/client/dcrd/notifications.go:38-40; chain/sync.go:241,478

getcfilterv2 without a 'data' string, a null ticket value in winningtickets, or a null result for getblockchaininfo/getmixmessage are dereferenced without checks and would crash the Go process; the port should return errors.

## QK-0408 — Streaming hex decoders ignore trailing data

Spec: [15](specs/15-dcrd-rpc-sync.md). Source: rpc/client/dcrd/json.go:32-44,106-120,128-147,234-281

unhex/headers/transactions feed a streaming hex decoder to Deserialize, so bytes after a complete header/tx/block are never read or validated; upper-case hex is accepted; JSON escapes inside the string are not interpreted; 'hashes' accepts hash strings shorter than 64 digits while 'hash' requires exactly 64.

## QK-0409 — Client certificate only applied when a CA is present

Spec: [15](specs/15-dcrd-rpc-sync.md). Source: chain/sync.go:555-578; dcrwallet.go:579-584

The tls.X509KeyPair branch is nested inside 'len(CA) != 0 && !Insecure'. With dcrdauthtype=clientcert and an unreadable CA file the connection is attempted with no client cert, no Basic credentials (cleared by rpcSyncLoop) and the system trust store.

## QK-0410 — Password without username sends no auth

Spec: [15](specs/15-dcrd-rpc-sync.md). Source: chain/sync.go:547-549

WithBasicAuth is added only when User != "".

## QK-0411 — Duplicate tip hash in block locators

Spec: [15](specs/15-dcrd-rpc-sync.md). Source: wallet/wallet.go:1497-1536

wallet.blockLocators puts the tip hash at index 0 and then the loop appends the hash at the tip height again, so every getheaders request carries the tip twice.

## QK-0412 — Unbounded request concurrency

Spec: [15](specs/15-dcrd-rpc-sync.md). Source: chain/sync.go:253-275; rpc/client/dcrd/calls.go:324-336,368-388; wsrpc/v2@v2.3.8/rpc.go:386-404

Header catch-up launches one goroutine per header (up to 2000 simultaneous getcfilterv2 calls) on the single connection; Blocks and CFiltersV2 do the same. The notification queue is likewise unbounded (one parked goroutine per pending notification).

## QK-0413 — Run can hang in its notifier-drain defer

Spec: [15](specs/15-dcrd-rpc-sync.md). Source: chain/sync.go:713-742

If notifymixmessages fails with a JSON-RPC error while the connection is healthy, the deferred '<-s.notifier.done' usually blocks until something else closes the connection, because the errgroup context is not yet done when the defer checks it.

## QK-0414 — notifier.done does not mean handlers finished

Spec: [15](specs/15-dcrd-rpc-sync.md). Source: chain/sync.go:722-723,810-815; wsrpc/v2@v2.3.8/rpc.go:270-281

notifier.Close is invoked by wsrpc when the client's final error is set, not after the last Notify returns; the comment 'Wait for notifications to finish before returning' overstates it and a handler may still be running after Run returns.

## QK-0415 — Call returns the context error even after a successful reply

Spec: [15](specs/15-dcrd-rpc-sync.md). Source: wsrpc/v2@v2.3.8/rpc.go:434-438

wsrpc.Client.Call's deferred check overrides the result with ctx.Err() whenever the context is done at return, so an accepted sendrawtransaction can be reported as cancelled.

## QK-0416 — Error op contains a Go struct dump of the Syncer

Spec: [15](specs/15-dcrd-rpc-sync.md). Source: wallet/wallet.go:1423-1431,1448-1456

wallet.FetchMissingCFilters formats the backend with %v in its op; *chain.Syncer has no String method so the op text is Go's dump of the struct (addresses, internals). Not reproducible; choose a stable rendering.

## QK-0417 — Account discovery flag semantics

Spec: [15](specs/15-dcrd-rpc-sync.md). Source: chain/sync.go:82,117-119,360-367; internal/rpc/rpcserver/server.go:2848-2872

discoverAccts is set to !w.Locked() at NewSyncer time and DisableDiscoverAccounts has no caller: gRPC RpcSync with discover_accounts=false still discovers accounts if the wallet is unlocked, and each reconnect (new Syncer) re-runs account discovery when unlocked and a rescan point exists.

## QK-0418 — FetchHeadersProgress parameter is misnamed

Spec: [15](specs/15-dcrd-rpc-sync.md). Source: chain/sync.go:95,176-180,294; internal/rpc/rpcserver/server.go:2913-2922

Callbacks.FetchHeadersProgress declares 'lastHeaderHeight' but receives the count of headers newly added to the forest in that batch; gRPC forwards it as fetched_headers_count.

## QK-0419 — targetSyncHeight is 0 early and frozen on re-sync

Spec: [15](specs/15-dcrd-rpc-sync.md). Source: chain/sync.go:226-242

Synced() reports target 0 during waitRPCSync and the missing-cfilter phase; a re-sync that starts in the synced state never updates it.

## QK-0420 — Redundant round when no better chain

Spec: [15](specs/15-dcrd-rpc-sync.md). Source: chain/sync.go:315-317

When EvaluateBestChain returns empty the header loop 'continue's without refreshing locators, repeating one identical getheaders and all its getcfilterv2 calls before exiting on added==0.

## QK-0421 — CA and client cert read once per process

Spec: [15](specs/15-dcrd-rpc-sync.md). Source: dcrwallet.go:563-564

rpcSyncLoop reads the CA file and client key pair before the loop; reconnect attempts reuse the bytes, so a rotated dcrd certificate is not picked up without a restart.

## QK-0422 — HTTP 401 surfaces as 'websocket: bad handshake'

Spec: [15](specs/15-dcrd-rpc-sync.md). Source: gorilla/websocket@v1.5.1/client.go:402-413

Any non-101 upgrade reply, including bad credentials, yields gorilla's generic ErrBadHandshake text in the 'RPC synchronization stopped' log line.

## QK-0423 — permessage-deflate always offered

Spec: [15](specs/15-dcrd-rpc-sync.md). Source: wsrpc/v2@v2.3.8/rpc.go:206-210; gorilla/websocket@v1.5.1/client.go:242-244,415-427

wsrpc sets EnableCompression, so the upgrade request carries Sec-WebSocket-Extensions: permessage-deflate with both no_context_takeover parameters; dcrd never accepts it.

## QK-0424 — JSON-RPC error mapping uses a type switch

Spec: [15](specs/15-dcrd-rpc-sync.md). Source: internal/rpc/jsonrpc/errors.go:16-49

convertError preserves dcrd's code only when the error is itself a *wsrpc.Error; once wrapped by any op it becomes ErrRPCWallet (-4) with the op-prefixed message.

## QK-0425 — rebroadcastwinners sent even when voting is disabled

Spec: [15](specs/15-dcrd-rpc-sync.md). Source: chain/sync.go:645-647,683

notifywinningtickets is conditional on VotingEnabled but rebroadcastwinners is unconditional.

## QK-0426 — Local-timezone Go time format in a log line

Spec: [15](specs/15-dcrd-rpc-sync.md). Source: chain/sync.go:344-345

'Connected %d blocks, new tip block %v, height %d, date %v' prints the header timestamp with Go's default time.Time format in the process's local zone.

## QK-0427 — Dead or mislabeled API

Spec: [15](specs/15-dcrd-rpc-sync.md). Source: rpc/client/dcrd/calls.go:85-93,195-198,309-318,526; chain/sync.go:117-119; dcrwallet.go:586-598

dcrd.RPC.PublishTransaction, ExistsLiveTicket, MixPairRequests, cfilterV2.Deserialize and Syncer.DisableDiscoverAccounts have no callers; GetConfirmationHeight reports errors under op 'dcrd.GetRawTransaction'; rpcSyncLoop's no-delay restart on a nil Run error is unreachable.

## QK-0428 — Commitment proof index is not pinned

Spec: [15](specs/15-dcrd-rpc-sync.md). Source: validate/validate.go:69-102

validate.CFilterV2HeaderCommitment accepts any (proofindex, proofhashes) pair from getcfilterv2 that hashes to header.StakeRoot, and on networks other than mainnet/testnet3 (e.g. simnet) the check applies from height 0; below the DCP0005 heights on mainnet/testnet3 no per-header check is done at all.

## QK-0429 — Account discovery depends on startup unlock

Spec: [15](specs/15-dcrd-rpc-sync.md). Source: dcrwallet.go:240-248,468-529; chain/sync.go:82

chain.NewSyncer samples !w.Locked(); on the daemon path the wallet is unlocked beforehand only by --pass or startPromptPass (--promptpass, --enableticketbuyer, NeedsAccountsSync), so account discovery is a side effect of those options rather than a syncer setting.

## QK-0430 — Two pong-deadline modes are exercised in-tree

Spec: [15](specs/15-dcrd-rpc-sync.md). Source: cmd/sweepaccount/main.go:317-322; wsrpc/v2@v2.3.8/rpc.go:201-243

The daemon syncer and cmd/repaircfilters pass WithoutPongDeadline; cmd/sweepaccount does not, so it runs with a ~126.7 s initial and ~66.7 s per-pong read deadline. A shared transport crate must implement both.

## QK-0431 — Lenient winningtickets decoding can yield a nil block hash

Spec: [15](specs/15-dcrd-rpc-sync.md). Source: rpc/client/dcrd/notifications.go:31-41; chain/sync.go:817-823

A null or missing first element leaves the block-hash pointer nil with no error and it is passed to VoteOnOwnedTickets; a port should reject the notification.

## QK-0432 — unimplemented/unsupported handlers are unreachable

Spec: [16](specs/16-jsonrpc-transport-types-help.md). Source: internal/rpc/jsonrpc/methods.go:192-199, 262-266; rpc/jsonrpc/types/methods.go:1189-1312

The 8 noHelp table entries (backupwallet, getwalletinfo, importwallet, listaddressgroupings, dumpwallet, encryptwallet, move, setaccount) have no dcrjson registration, so ParseParams fails first and the client gets -32600 'Invalid request' instead of -1 'Method unimplemented' / 'Request unsupported by dcrwallet'. They are also never forwarded to dcrd.

## QK-0433 — All parameter errors collapse to -32600

Spec: [16](specs/16-jsonrpc-transport-types-help.md). Source: internal/rpc/jsonrpc/methods.go:263-266

Any ParseParams error (wrong arity, wrong type, unregistered) is returned as dcrjson.ErrRPCInvalidRequest; the descriptive dcrjson text is discarded. Not -32602.

## QK-0434 — Server never closes a websocket

Spec: [16](specs/16-jsonrpc-transport-types-help.md). Source: internal/rpc/jsonrpc/server.go:338-352, 375, 401-405, 422-423, 466-470, 501-502

'Disconnect immediately' paths (failed/multiple authenticate, unauthenticated request, stop) only stop the responder goroutine; no close frame is sent and conn.Close() is never called. The client just stops receiving replies; one more client message blocks the reader goroutine forever on an unbuffered channel and the socket leaks until process exit.

## QK-0435 — Websocket special replies have "jsonrpc":""

Spec: [16](specs/16-jsonrpc-transport-types-help.md). Source: internal/rpc/jsonrpc/methods.go:284-306; server.go:378, 408, 429

authenticate success, websocket stop, and invalid-request replies are built by makeResponse without a version, yielding `"jsonrpc":""`, and skip the id-type validation that MarshalResponse applies everywhere else.

## QK-0436 — --noservertls with --jsonrpcauthtype=clientcert is unauthenticated

Spec: [16](specs/16-jsonrpc-transport-types-help.md). Source: rpcserver.go:350-367; internal/rpc/jsonrpc/server.go:124-127, 263-265

With TLS disabled the client-CA check is skipped and username/password are withheld because authtype is not basic, so authsha is nil and every request is accepted (localhost only, due to the noservertls config rule).

## QK-0437 — Requests with bool/array/object ids execute but cannot be answered

Spec: [16](specs/16-jsonrpc-transport-types-help.md). Source: dcrd/dcrjson/v4@v4.2.0/jsonrpc.go:135-138; internal/rpc/jsonrpc/server.go:599-605, 452-458

dcrjson.NewResponse rejects such ids after the handler has run: POST returns HTTP 500 '500 Internal Server Error', websocket sends nothing. State-changing calls still take effect.

## QK-0438 — Explicit null suppresses defaults and is accepted for required params

Spec: [16](specs/16-jsonrpc-transport-types-help.md). Source: dcrd/dcrjson/v4@v4.2.0/cmdparse.go:97-107, 132-160

ParseParams applies jsonrpcdefault values only to positions beyond the supplied count; a JSON null for an optional param leaves it nil (no default), and null for a required param silently yields the zero value. Handlers nil-check even defaulted params.

## QK-0439 — setdisapprovepercent on mainnet returns -4 '-32600: Invalid request'

Spec: [16](specs/16-jsonrpc-transport-types-help.md). Source: internal/rpc/jsonrpc/methods.go:3856-3858; errors.go:17-48

The handler returns the value-typed dcrjson.ErrInvalidRequest; convertError only recognises *RPCError, so it is treated as an unclassified error and its Error() text becomes the message.

## QK-0440 — POST authenticate gets an empty HTTP 200; POST accepts any HTTP method

Spec: [16](specs/16-jsonrpc-transport-types-help.md). Source: internal/rpc/jsonrpc/server.go:129-144, 585-589

authenticate over POST is logged and dropped without writing a body (200, Content-Type application/json, empty). The `/` handler never checks the HTTP method or path.

## QK-0441 — Stop strings differ by transport

Spec: [16](specs/16-jsonrpc-transport-types-help.md). Source: internal/rpc/jsonrpc/server.go:430, 593

POST result is 'dcrwallet stopping', websocket result is 'dcrwallet stopping.'; stop skips parameter parsing entirely.

## QK-0442 — Usage text misdocuments object keys

Spec: [16](specs/16-jsonrpc-transport-types-help.md). Source: dcrd/dcrjson/v4@v4.2.0/cmdinfo.go:55-97; rpc/jsonrpc/types/methods.go:165-169, 781, 862, 977-983

Usage strings use lower-cased Go field names: fundrawtransaction shows "conftarget" (wire key conf_target, which does NOT match case-insensitively), signrawtransaction shows "scriptpubkey"/"redeemscript"; sendfromtreasury shows bare `amounts` (no jsonrpcusage tag).

## QK-0443 — Help text disagrees with code

Spec: [16](specs/16-jsonrpc-transport-types-help.md). Source: internal/rpchelp/helpdescs_en_US.go:105, 678-679; internal/rpchelp/methods.go:30-132; internal/rpc/jsonrpc/methods.go:3491, 3514, 4218, 4660, 5849; log.go:47-61

debuglevel help lists 12 subsystems but the daemon has 13 (AMGR missing); mixaccount help implies an account argument but takes none; help result types are wrong for purchaseticket (array or object, not string), sendtomultisig (object), redeemmultisigouts ({results:[...]}), listunspent and getpeerinfo (arrays), getcoinjoinsbyacct; ~32 description keys are dead.

## QK-0444 — getpeerinfo narrows dcrd's reply

Spec: [16](specs/16-jsonrpc-transport-types-help.md). Source: internal/rpc/jsonrpc/methods.go:2500-2509; rpc/jsonrpc/types/results.go:71-80

Under dcrd sync the wallet unmarshals dcrd's getpeerinfo into its own 8-member struct and re-emits it (dropping dcrd-only members, always emitting addrlocal); under a non-SPV, non-dcrd backend the result is null.

## QK-0445 — Unrestricted dcrd passthrough

Spec: [16](specs/16-jsonrpc-transport-types-help.md). Source: internal/rpc/jsonrpc/methods.go:231-259, 2884-2886; server.go:585

Any unknown method, including dcrd websocket-only and notification-registration methods, is forwarded over the wallet's own dcrd websocket connection with raw params; UFWebsocketOnly flags are never consulted.

## QK-0446 — Invalid-request reply echoes id only for member type errors

Spec: [16](specs/16-jsonrpc-transport-types-help.md). Source: internal/rpc/jsonrpc/server.go:557-574, 369-392

Go json.Unmarshal continues after a member type error (e.g. params is an object), so the -32600 reply echoes the decoded id and jsonrpc; syntax errors and non-object top-level values (batches) yield id null and jsonrpc 1.0. No batching support on either transport.

## QK-0447 — ReadTimeout likely cancels long POST handlers

Spec: [16](specs/16-jsonrpc-transport-types-help.md). Source: internal/rpc/jsonrpc/server.go:95-110; internal/rpc/jsonrpc/methods.go:268-272 (Go net/http server.go connReader.backgroundRead/handleReadErrorLocked)

http.Server.ReadTimeout=10s leaves the connection read deadline armed while the handler runs; net/http's background read then fails and cancels the request context, so POST handlers running >10 s see ctx cancellation ('Canceled RPC method ...'). Derived from Go net/http source; not verified on a live daemon. Websocket requests are unaffected.

## QK-0448 — Throttle precedes authentication

Spec: [16](specs/16-jsonrpc-transport-types-help.md). Source: internal/rpc/jsonrpc/server.go:129, 146, 287-302

Concurrency counters are incremented before auth, so unauthenticated clients can exhaust slots (429 for others); idle websocket sessions hold a slot for their lifetime.

## QK-0449 — Normal websocket close logged as warning

Spec: [16](specs/16-jsonrpc-transport-types-help.md). Source: internal/rpc/jsonrpc/server.go:340-346

gorilla reports a close frame as *websocket.CloseError, which is neither io.EOF nor io.ErrUnexpectedEOF, so 'Websocket receive failed from client' is logged at warn for clean disconnects.

## QK-0450 — Stop() does not shut down the HTTP server

Spec: [16](specs/16-jsonrpc-transport-types-help.md). Source: internal/rpc/jsonrpc/server.go:216-240

Server.Stop closes listeners and waits on a wait group; http.Server.Shutdown/Close is never called and websocket TCP connections are not closed.

## QK-0451 — sample config documents a non-existent option

Spec: [16](specs/16-jsonrpc-transport-types-help.md). Source: sample-dcrwallet.conf:175; config.go:155

sample-dcrwallet.conf mentions `legacyrpclisten`; the actual flag is `rpclisten`.

## QK-0452 — Handler panic is survivable over POST and fatal over websocket

Spec: [16](specs/16-jsonrpc-transport-types-help.md). Source: internal/rpc/jsonrpc/server.go:385,412,434,448-459; Go net/http/server.go conn.serve

POST handlers run on the net/http connection goroutine whose deferred recover logs 'http: panic serving <addr>: <value>' to stderr and drops the connection without a response; the same handler reached through /ws runs on a bare goroutine and an unrecovered panic terminates the process. The three panic(err) calls in the websocket responder are likewise unrecovered.

## QK-0453 — Client close frame is echoed but the socket is never closed and becomes write-dead

Spec: [16](specs/16-jsonrpc-transport-types-help.md). Source: gorilla/websocket@v1.5.1/conn.go:424-480,1162-1173; internal/rpc/jsonrpc/server.go:473-503

gorilla's default close handler echoes a close frame with the client's status code; afterwards every data write fails with 'websocket: close sent', so replies of in-flight requests are dropped and logged as 'Failed websocket send to client'. The wallet still never closes the TCP connection.

## QK-0454 — net/http and gorilla diagnostics bypass the wallet logger

Spec: [16](specs/16-jsonrpc-transport-types-help.md). Source: internal/rpc/jsonrpc/server.go:97-110; Go net/http/server.go Server.logf

With ErrorLog unset, 'http: TLS handshake error from ...', 'http: panic serving ...' and 'http: Accept error ...' go to stderr through Go's standard logger; rejected client certificates in clientcert mode leave no RPCS log line.

## QK-0455 — 'JSON-RPC server disabled' is logged even with no JSON-RPC listeners

Spec: [16](specs/16-jsonrpc-transport-types-help.md). Source: rpcserver.go:350-358

The basic-auth credential check precedes the listener-count check in the start decision chain, so --nolegacyrpc without username/password still prints the 'JSON-RPC server disabled (basic auth requires username and password, ...)' info line.

## QK-0456 — Upstream client-cert test uses one key for CA and client certificate

Spec: [16](specs/16-jsonrpc-transport-types-help.md). Source: x509_test.go:66-88; rpcserver.go:217-231

TestClientCert signs the client certificate for the same public key as the CA, whereas startRPCServers generates a separate client key in generateClientKeyPair; the test therefore does not exercise the production key arrangement.

## QK-0457 — addmultisigaddress returns {} instead of the address

Spec: [17](specs/17-jsonrpc-methods-part1.md). Source: internal/rpc/jsonrpc/methods.go:456; dcrd/txscript/v4@v4.1.2/stdaddr/addressv0.go:898-901

The handler returns the *stdaddr.AddressScriptHashV0 value itself; it has only unexported fields and no JSON/Text marshaler, so encoding/json emits `{}`. Help documents a string. createmultisig calls String() and is correct.

## QK-0458 — getreceivedbyaccount indexes results by iteration position

Spec: [17](specs/17-jsonrpc-methods-part1.md). Source: internal/rpc/jsonrpc/methods.go:2415-2419; wallet/udb/addressdb.go:698-708

results[int(account)] assumes TotalReceivedForAccounts returns accounts in numeric order, but the order is bbolt byte order of little-endian u32 keys. With an imported-xpub account (0x80000000) or >=256 accounts the wrong account's total is returned silently.

## QK-0459 — getbalance "*" account ordering is little-endian key order

Spec: [17](specs/17-jsonrpc-methods-part1.md). Source: wallet/wallet.go:1927-1946; wallet/udb/addressdb.go:698-708

balances array follows forEachAccount bucket order: 0, 0x80000000, 256, 512, ..., 1, 0x80000001, ..., imported last; numeric only for <256 accounts and no imported xpub.

## QK-0460 — auditreuse nil interface dereference

Spec: [17](specs/17-jsonrpc-methods-part1.md). Source: internal/rpc/jsonrpc/methods.go:509; wallet/notifications.go:96-105

out.Address.String() is called on a nil stdaddr.Address when a wallet credit's script yields no address or the manager lookup fails; POST drops the connection (net/http recover), websocket aborts the process.

## QK-0461 — gettransaction never sets type, ticketstatus, blockindex; ignores includewatchonly

Spec: [17](specs/17-jsonrpc-methods-part1.md). Source: internal/rpc/jsonrpc/methods.go:2652-2659; internal/rpchelp/helpdescs_en_US.go:448,467

`type` is always "", blockindex always 0, ticketstatus omitted; help documents type as regular/ticket/vote/revocation and category "recv" while code emits "receive".

## QK-0462 — createrawtransaction output order is random

Spec: [17](specs/17-jsonrpc-methods-part1.md). Source: internal/rpc/jsonrpc/methods.go:685

Outputs are appended while ranging over a Go map (cmd.Amounts); order and which error is reported for multiple bad entries vary between identical calls.

## QK-0463 — scriptPubKey.version always 0 in wallet-built vouts

Spec: [17](specs/17-jsonrpc-methods-part1.md). Source: internal/rpc/jsonrpc/methods.go:1686-1698, 2785-2791

createVoutList assigns only the outer vout.version; scriptPubKey.version is never set (dcrd sets both). SPV gettxout likewise reports version 0.

## QK-0464 — Explicit null for verbose returns hex in SPV getblock/getblockheader

Spec: [17](specs/17-jsonrpc-methods-part1.md). Source: internal/rpc/jsonrpc/methods.go:1252, 1377

Registered default is true, but explicit null leaves the pointer nil and the SPV code treats nil as false.

## QK-0465 — gettickets in SPV mode drops all mature tickets

Spec: [17](specs/17-jsonrpc-methods-part1.md). Source: wallet/tickets.go:70-73; internal/rpc/jsonrpc/methods.go:2602-2605

With no LiveTicketQuerier (SPV or no backend) LiveTicketHashes returns only immature/unmined tickets (when requested); mature unspent tickets are never listed. Iteration is not filtered by ownership or spent state.

## QK-0466 — syncstatus can be unanswerable

Spec: [17](specs/17-jsonrpc-methods-part1.md). Source: internal/rpc/jsonrpc/methods.go:1747-1754

headersfetchprogress = 1 - float32(blocksToFetch)/float32(target - initialHeight); a zero denominator gives -Inf which encoding/json refuses: HTTP 500 on POST, no reply on websocket. Negative denominator yields progress > 1.

## QK-0467 — SPV getblock retries forever for unknown hashes

Spec: [17](specs/17-jsonrpc-methods-part1.md). Source: spv/backend.go:95-111; internal/rpc/jsonrpc/methods.go:1363

spv.Syncer.Blocks loops over peers until success or context cancellation; an unservable hash never yields an error reply.

## QK-0468 — Unchecked integer narrowing of parameters

Spec: [17](specs/17-jsonrpc-methods-part1.md). Source: internal/rpc/jsonrpc/methods.go:347, 377-378, 1056, 1212, 732, 780, 2021, 2405, 2517

int->uint32 for branch/index, int->int32 for minconf/scanfrom, int64->int32 for getblockhash index, int64->uint32 for expiry, int->8-bit sighash, uint64->int32 peer id. e.g. accountaddressindex with branch 4294967296 returns the external index; getblockhash 4294967296 returns the genesis hash.

## QK-0469 — Large minconf counts everything in getreceivedby*

Spec: [17](specs/17-jsonrpc-methods-part1.md). Source: wallet/wallet.go:4486-4492, 4540-4544; wallet/udb/txquery.go:391-396, 534

stopHeight = tip - minconf + 1; a negative stop is interpreted by RangeTransactions as 'include unmined', so minconf > tip+1 returns the zero-confirmation total rather than 0.

## QK-0470 — Inconsistent error codes for similar failures

Spec: [17](specs/17-jsonrpc-methods-part1.md). Source: internal/rpc/jsonrpc/methods.go:899-930, 656, 1994, 2000, 1832-1840

fundrawtransaction reports bad tx hex, bad options and unknown account as -4 (no errAccountNotFound mapping); createrawtransaction bad txid is -8 while others use -22; importcfiltersv2 uses -32602/-32600 for application errors; decodeAddress pubkey path yields -4 vs -5 for base58; importpubkey bad length yields -8 via Encoding kind.

## QK-0471 — addtransaction does not verify block inclusion

Spec: [17](specs/17-jsonrpc-methods-part1.md). Source: wallet/chainntfns.go:365-380; internal/rpc/jsonrpc/methods.go:471-477

A block hash not on the wallet's main chain silently turns the request into 'add as unmined' (with input signature scrutiny); a main-chain hash is trusted without checking the tx is in that block. Trailing hex after a complete tx is ignored.

## QK-0472 — getunconfirmedbalance returns total - spendable

Spec: [17](specs/17-jsonrpc-methods-part1.md). Source: internal/rpc/jsonrpc/methods.go:1942-1951

Help says value of unmined outputs; code returns everything not spendable at 1 confirmation (immature coinbase, ticket-locked funds included).

## QK-0473 — getaccountaddress is a pure computation

Spec: [17](specs/17-jsonrpc-methods-part1.md). Source: internal/rpc/jsonrpc/methods.go:1890-1919; wallet/wallet.go:1952-1973

Returns external child lastUsed+1+cursor without generating/persisting anything, contrary to the comment and help; imported account yields -32603.

## QK-0474 — consolidate defaults to an internal change address

Spec: [17](specs/17-jsonrpc-methods-part1.md). Source: wallet/createtx.go:819-822; internal/rpc/jsonrpc/methods.go:587-595

Help says default destination comes from getnewaddress; code derives a new internal-branch address with gap policy ignore. Empty address string is treated as absent.

## QK-0475 — getpeerinfo strips dcrd fields; help type is a single object

Spec: [17](specs/17-jsonrpc-methods-part1.md). Source: internal/rpc/jsonrpc/methods.go:2500-2510; internal/rpchelp/methods.go:63

In dcrd mode the reply is decoded into the wallet's 8-field struct and re-encoded; help registers a single GetPeerInfoResult though an array is returned; synopsis says SPV only.

## QK-0476 — getstakeinfo field asymmetries

Spec: [17](specs/17-jsonrpc-methods-part1.md). Source: wallet/wallet.go:4094-4103, 4172-4211; rpc/jsonrpc/types/results.go:96

poolsize is filled in SPV mode despite the 'Not available to SPV wallets' comment; unspentexpired is never set in dcrd mode; live equals unspent in dcrd mode (no liveness query); unmined revocation counts as missed.

## QK-0477 — Negative zero amounts

Spec: [17](specs/17-jsonrpc-methods-part1.md). Source: wallet/wallet.go:2471, 2507

gettransaction send details compute amount = -float64(value)/1e8; zero-value outputs (ticket commitments) print as `-0`.

## QK-0478 — Go-specific text embedded in error messages

Spec: [17](specs/17-jsonrpc-methods-part1.md). Source: internal/rpc/jsonrpc/methods.go:700, 714, 1847-1848; dcrd/chaincfg/chainhash@v1.0.5/hash.go:98-129

Messages contain encoding/hex texts (`encoding/hex: invalid byte: U+0067 'g'`), io texts (`EOF`, `unexpected EOF`), Go %q/%#q quoting, %T type names (`*stdaddr.AddressPubKeyEd25519V0`) and Amount.String() (`0 DCR`).

## QK-0479 — Help usage names wrong option key

Spec: [17](specs/17-jsonrpc-methods-part1.md). Source: rpc/jsonrpc/types/methods.go:165-169; internal/rpc/jsonrpc/rpcserverhelp.go (requestUsages)

fundrawtransaction usage shows `conftarget` but the JSON member is `conf_target`; the unknown member is silently ignored so confs stays 1.

## QK-0480 — auditreuse help swaps key/value descriptions; since filters addresses not outpoints

Spec: [17](specs/17-jsonrpc-methods-part1.md). Source: internal/rpchelp/helpdescs_en_US.go:50-52; internal/rpc/jsonrpc/methods.go:513-515, 552-560

`--result0--value` is 'Reused address' and `--result0--key` is the array; an address with any output at/after `since` (or unmined) is reported with all its outpoints.

## QK-0481 — Dead entries and dead branches

Spec: [17](specs/17-jsonrpc-methods-part1.md). Source: internal/rpc/jsonrpc/methods.go:192-218, 263-266, 2443-2445, 1619

unimplemented/unsupported handlers are unreachable (no registered command type -> -32600); errAddressNotInWallet branch in getreceivedbyaddress is dead; createVoutList filter param always nil; makeResponse marshal-failure branch unreachable.

## QK-0482 — Multi-read handlers are not atomic; MainChainTip swallows errors

Spec: [17](specs/17-jsonrpc-methods-part1.md). Source: wallet/wallet.go:792-802; internal/rpc/jsonrpc/methods.go:1067-1092, 2634-2692

getbalance, getinfo, getblockheader, getblock, gettransaction, gettxout, syncstatus read several independent DB snapshots; MainChainTip ignores the View error and yields zero hash/height 0.

## QK-0483 — accountsyncaddressindex comments contradict code

Spec: [17](specs/17-jsonrpc-methods-part1.md). Source: internal/rpc/jsonrpc/methods.go:357-361, 385-388; wallet/addresses.go:872-874

Function comment promises an error if the pool is beyond the index and inline comment says child indexes are no longer changed; code performs no such check and reassigns the in-memory cursor (cursor = index - lastUsed).

## QK-0484 — Passthrough cancellation masks dcrd's answer

Spec: [17](specs/17-jsonrpc-methods-part1.md). Source: internal/rpc/jsonrpc/methods.go:247-258

In the unknown-method passthrough closure the `ctx.Err() != nil` test precedes the error test, so a request whose context ended is answered -1 with the context error text even when dcrd had already returned a result or an error object.

## QK-0485 — Unguarded Transactions[0] in SPV getblock

Spec: [17](specs/17-jsonrpc-methods-part1.md). Source: internal/rpc/jsonrpc/methods.go:1464; spv/backend.go:67-91

The verbose path reads `blk.Transactions[0].Version` without checking the regular tree is non-empty; a block with no regular transactions would panic (connection drop on POST, process abort on websocket). Reachability through the SPV layer is UNVERIFIED.

## QK-0486 — Default account by name vs by number

Spec: [17](specs/17-jsonrpc-methods-part1.md). Source: internal/rpc/jsonrpc/methods.go:573, 1931, 2329, 2360, 2463; wallet/udb/addressmanager.go:91-99, 2343-2350

getunconfirmedbalance, getnewaddress and getrawchangeaddress default to the account named `default`; consolidate and getmasterpubkey default to account number 0. After renaming account 0 the first three fail with -11 when called without an account.

## QK-0487 — gettransaction has a TODO'd, absent `generated` member

Spec: [17](specs/17-jsonrpc-methods-part1.md). Source: internal/rpc/jsonrpc/methods.go:2650-2659, 2696-2707

A TODO and a commented-out `Generated:` assignment show the member was intended but never added; the handler also discards nine members (including txtype) of each ListTransactionsResult it converts.

## QK-0488 — getAccountAddress comment promises ErrRPCWalletKeypoolRanOut

Spec: [17](specs/17-jsonrpc-methods-part1.md). Source: internal/rpc/jsonrpc/methods.go:1890-1895

The doc comment says the handler returns the keypool-ran-out code (-12); no code path in the range produces it and the wallet has no keypool.

## QK-0489 — getpeerinfo null for an unknown backend type

Spec: [17](specs/17-jsonrpc-methods-part1.md). Source: internal/rpc/jsonrpc/methods.go:2500-2510

When the backend is neither *spv.Syncer nor *chain.Syncer the handler returns a nil slice, encoded as JSON null rather than []. Unreachable at the pin (only two backend types exist).

## QK-0490 — Help tests are one-directional

Spec: [17](specs/17-jsonrpc-methods-part1.md). Source: internal/rpc/jsonrpc/rpchelp_test.go:27-35, 52-73, 90-110

TestRPCMethodHelpGeneration fails for a handler without help text but not for a help entry (rpchelp.Methods) with no handler; the usage test does not report leftovers at all.

## QK-0491 — setdisapprovepercent mainnet error is malformed

Spec: [18](specs/18-jsonrpc-methods-part2.md). Source: internal/rpc/jsonrpc/methods.go:3856-3858; internal/rpc/jsonrpc/errors.go:16-49; dcrd/dcrjson/v4@v4.2.0/jsonerr.go:10-13

Handler returns dcrjson.ErrInvalidRequest, an RPCError value rather than *RPCError; convertError does not recognise it, so the client receives code -4 with message "-32600: Invalid request".

## QK-0492 — spendoutputs panics when no change output is produced

Spec: [18](specs/18-jsonrpc-methods-part2.md). Source: internal/rpc/jsonrpc/methods.go:4389; wallet/txauthor/author.go:171-181

atx.RandomizeChangePosition() is called unconditionally; with ChangeIndex == -1 it indexes outputs[-1]. Over websocket the request goroutine is the server's own, so the process exits; over HTTP POST net/http recovers and drops the connection. Verified by reading only.

## QK-0493 — Unchecked output index panics in FetchOutput and SignTransaction

Spec: [18](specs/18-jsonrpc-methods-part2.md). Source: wallet/wallet.go:230; wallet/wallet.go:4809

spendoutputs (via w.FetchOutput) and signrawtransaction/signrawtransactions/redeemmultisigout (via SignTransaction) index TxOut[index] of a wallet transaction without bounds checks; an existing txid with an out-of-range index panics inside a database read transaction.

## QK-0494 — listunspent returns null when empty

Spec: [18](specs/18-jsonrpc-methods-part2.md). Source: wallet/wallet.go:3479; internal/rpc/jsonrpc/methods.go:3316-3323

ListUnspent declares a nil slice and returns it unchanged, so an empty result marshals as null rather than [].

## QK-0495 — Large minconf widens listreceivedby* instead of narrowing

Spec: [18](specs/18-jsonrpc-methods-part2.md). Source: internal/rpc/jsonrpc/methods.go:3081-3087; wallet/wallet.go:4486-4492; wallet/udb/txquery.go:392-397,533-536

Stop height tip-minconf+1 becomes negative when minconf > tip+1; RangeTransactions treats a negative end as 'through the tip and then unmined', so everything including unmined transactions is counted.

## QK-0496 — redeemmultisigouts off-by-one and zero-valued tail

Spec: [18](specs/18-jsonrpc-methods-part2.md). Source: internal/rpc/jsonrpc/methods.go:4190-4216

Loop breaks on itr > max, so number=N processes N+1 outputs; results array is pre-sized to the full output count so unprocessed entries remain {"hex":"","complete":false}.

## QK-0497 — walletpubpassphrasechange never returns 'incorrect passphrase'

Spec: [18](specs/18-jsonrpc-methods-part2.md). Source: internal/rpc/jsonrpc/methods.go:5707-5712

errors.Is(errors.Passphrase, err) has its arguments swapped and never matches; a wrong old passphrase yields code -14 from the error kind but with the raw wallet error text.

## QK-0498 — signrawtransactions hides publish failures and bypasses the wallet

Spec: [18](specs/18-jsonrpc-methods-part2.md). Source: internal/rpc/jsonrpc/methods.go:5152-5190

Complete transactions are published with n.PublishTransactions directly (never recorded by the wallet); on failure the error is discarded and the element is sent:false with txhash present as the empty string; incomplete ones omit txhash.

## QK-0499 — Map-iteration-ordered outputs

Spec: [18](specs/18-jsonrpc-methods-part2.md). Source: internal/rpc/jsonrpc/methods.go:3122,3569,3707,4463; wallet/wallet.go:645-659,280,4291

Element order of listreceivedbyaddress, listlockunspent, treasurypolicy (list), tspendpolicy (list), ticketinfo choices, and the transaction output order built by sendmany (makeOutputs) and sendfromtreasury all follow Go map iteration order, which is randomised per call.

## QK-0500 — tspendpolicy/treasurypolicy list-mode asymmetries

Spec: [18](specs/18-jsonrpc-methods-part2.md). Source: internal/rpc/jsonrpc/methods.go:3813-3851,3959-4000; wallet/wallet.go:274-289

Single queries report abstain as "abstain"; list queries report "". treasurypolicy list mode validates but ignores the ticket argument. tspendpolicy list mode includes abstaining tspends (help says non-abstaining only) and GetAllTSpends deletes expired tspends from the in-memory cache as a side effect.

## QK-0501 — spendoutputs fee is inflated by wrong size constants

Spec: [18](specs/18-jsonrpc-methods-part2.md). Source: internal/rpc/jsonrpc/methods.go:4270,4314-4317; wallet/txsizes/size.go:132-153,198-200; wallet/udb/txmined.go:3326

accountChangeSource.ScriptSize returns P2PKHOutputSize (36) instead of a script size (25), and spendOutputsInputSource records RedeemP2PKHInputSize (166) as the per-input signature-script size (wallet uses 108), so each input is estimated at 224 bytes and change at 47 bytes. The resulting fee is visible in the published transaction.

## QK-0502 — spendoutputs publishes ValueIn = -1 and holds a read tx across a write

Spec: [18](specs/18-jsonrpc-methods-part2.md). Source: internal/rpc/jsonrpc/methods.go:4347,4378-4395; wallet/createtx.go:196-202

Inputs are created with wire.NullValueIn and never updated; outpoints are always tree 0; output amounts are not checked for sign/zero/dust; SecretsSource opens a database read transaction that stays open while NewChangeAddress performs a write and while the transaction is published.

## QK-0503 — sendfromtreasury publishes without recording and checks the backend last

Spec: [18](specs/18-jsonrpc-methods-part2.md). Source: internal/rpc/jsonrpc/methods.go:3775-3785

The tspend is built and signed before s.walletLoader.NetworkBackend() is consulted, then sent with n.PublishTransactions; the wallet never records it and the key is not checked against network Pi keys.

## QK-0504 — signrawtransaction swallows fatal signing errors and ignores WIF suite

Spec: [18](specs/18-jsonrpc-methods-part2.md). Source: internal/rpc/jsonrpc/methods.go:5035-5068,5094-5119; wallet/wallet.go:4824

The error return of SignTransaction (e.g. previous output not found) is only reflected as complete:false while the unsigned hex is still returned; ed25519/schnorr WIFs are accepted and indexed but SignTransaction always signs as ecdsa-secp256k1; a WIF error returns while gettxout goroutines are still running.

## QK-0505 — Inconsistent sign checks on send amounts

Spec: [18](specs/18-jsonrpc-methods-part2.md). Source: internal/rpc/jsonrpc/methods.go:4512,4603,4732,4761,4635

sendfrom and settxfee test the float before rounding (-1e-9 rejected); sendtoaddress tests after rounding (-1e-9 rounds to 0, passes, then fails as dust); sendtotreasury rejects zero with the text 'negative amount'; sendtomultisig has no sign check.

## QK-0506 — validateaddress field oddities

Spec: [18](specs/18-jsonrpc-methods-part2.md). Source: internal/rpc/jsonrpc/methods.go:5321-5377; wallet/udb/address.go:218-220

iscompressed is true for every hash-bearing address including P2SH and false for pubkey addresses; script is the address's own class for all valid addresses and is overwritten with the redeem-script class for wallet P2SH addresses; pubkeyaddr is an address string; redeem-script fields are filled even when the wallet is locked (help says otherwise).

## QK-0507 — listsinceblock is inclusive and height-based

Spec: [18](specs/18-jsonrpc-methods-part2.md). Source: internal/rpc/jsonrpc/methods.go:3164-3179; wallet/wallet.go:2525-2547

The scan runs down to and including the height of the named block, so that block's own transactions are returned although help calls it the parent block; a side-chain hash selects the main-chain range at the same height.

## QK-0508 — listtransactions paging counts transactions and emits -0

Spec: [18](specs/18-jsonrpc-methods-part2.md). Source: wallet/wallet.go:2552-2607,2471-2510

count counts transactions that produced at least one entry, from counts all transactions; negative from behaves as 0, count <= 0 returns []. Send entries for zero-value outputs (ticket commitments, OP_RETURN) carry amount -0.

## QK-0509 — Validation before wallet check and early side effects

Spec: [18](specs/18-jsonrpc-methods-part2.md). Source: internal/rpc/jsonrpc/methods.go:3525-3542,5615-5620,4083-4102,3338-3348; wallet/multisig.go:88

processunmanagedticket validates hash and vsphost before checking for a loaded wallet; mixoutput/mixaccount check MixingEnabled first; redeemmultisigout derives and consumes a new internal address of account 0 before validating the hash and always leaves output script version 0; lockunspent applies earlier elements before failing on a later bad txid.

## QK-0510 — updateVSPVoteChoices error semantics

Spec: [18](specs/18-jsonrpc-methods-part2.md). Source: internal/rpc/jsonrpc/methods.go:4810-4864; wallet/wallet.go:5705-5747; wallet/vsp.go:293-306

Runs after the local database commit; per-ticket errors are collected and returned via errors.Join (newline-joined) despite the 'first error' comment; a failure to fetch ticket status from the VSP is logged and treated as success; on chains shorter than the expiry window the negative range start makes the iteration cover only unmined tickets and the tip block (derived by reading).

## QK-0511 — purchaseticket VSP policy fixed by first caller

Spec: [18](specs/18-jsonrpc-methods-part2.md). Source: internal/rpc/jsonrpc/methods.go:3434-3450; wallet/vspclientloader.go:12-26

w.VSP caches the client by URL, so FeeAcct/ChangeAcct from the first purchaseticket (or start-up config) are reused for later calls with a different fromaccount.

## QK-0512 — Float-to-int overflow in NewAmount is platform-dependent

Spec: [18](specs/18-jsonrpc-methods-part2.md). Source: dcrd/dcrutil/v4@v4.0.3/amount.go:61-89

round(f*1e8) converts to int64 without a range check; out-of-range values give MinInt64 on amd64 and saturate on arm64, so a huge amount is 'negative' on one and MaxInt64 on the other.

## QK-0513 — walletpassphrase accepts negative and overflowing timeouts

Spec: [18](specs/18-jsonrpc-methods-part2.md). Source: internal/rpc/jsonrpc/methods.go:5579-5584; wallet/wallet.go:1804-1839

time.Second * Duration(timeout) wraps for values above about 9.2e9 seconds; a negative duration makes time.After fire immediately so the wallet unlocks and re-locks at once.

## QK-0514 — Help text disagreements

Spec: [18](specs/18-jsonrpc-methods-part2.md). Source: internal/rpchelp/helpdescs_en_US.go:448,626,679,692,911; internal/rpchelp/methods.go:89,95,97,105; wallet/wallet.go:2361-2372

About 45 documented mismatches, e.g. category 'recv' vs emitted 'receive'; listunspent/redeemmultisigouts/sendtomultisig/purchaseticket declared result types differ from actual; ticketinfo status list includes values never produced; orphan help keys purchaseticket-nosplittransaction and mixaccount-account; comments documented as unused but rejected with -1 when non-empty.

## QK-0515 — WIF with unknown signature-suite byte surfaces as raw -4

Spec: [18](specs/18-jsonrpc-methods-part2.md). Source: dcrd/dcrutil/v4@v4.0.3/wif.go:138-161; internal/rpc/jsonrpc/methods.go:5036-5047

DecodeWIF's suite switch has no default, returning suite 0 and nil key bytes; signrawtransaction takes its ecdsa branch, NewAddressPubKeyEcdsaSecp256k1V0Raw(nil) fails, and that error is returned unwrapped (code -4) instead of the -22 used for every other bad WIF. No nil dereference occurs.

## QK-0516 — listsinceblock dead guard

Spec: [18](specs/18-jsonrpc-methods-part2.md). Source: internal/rpc/jsonrpc/methods.go:3148-3162

After rejecting targetConf < 1, `if targetConf > 0` is always true, so the tipHash default for lastblock is never returned; lastblock always comes from the BlockInfo lookup.

## QK-0517 — help holds a global mutex across a dcrd call

Spec: [18](specs/18-jsonrpc-methods-part2.md). Source: internal/rpc/jsonrpc/methods.go:2910-2932

helpDescsMu is locked before the help-map lookup and released by defer at return, so the dcrd `help <command>` round trip for non-wallet commands runs under the lock and serializes concurrent help requests.

## QK-0518 — sendfromtreasury rejects empty amounts only after signing

Spec: [18](specs/18-jsonrpc-methods-part2.md). Source: internal/rpc/jsonrpc/methods.go:3670-3773; dcrd/blockchain/stake/v5@v5.0.3/treasury.go:155-159

No up-front check on the amounts map; the private key is loaded and the tspend signed before stake.CheckTSpend rejects the single-output transaction with a raw -4 error.

## QK-0519 — ssrtx flag is a no-op

Spec: [18](specs/18-jsonrpc-methods-part2.md). Source: internal/rpc/jsonrpc/methods.go:4931-4934, 5001

`ssgen` and `ssrtx` both map to SigHashAll; the flag string is otherwise consulted only to skip the gettxout for input 0 when it equals `ssgen`.

## QK-0520 — Treasury transactions listed as txtype regular

Spec: [18](specs/18-jsonrpc-methods-part2.md). Source: wallet/wallet.go:2410-2423

wallet.listTransactions maps only SStx/SSGen/SSRtx; TAdd, TSpend and treasurybase fall through to `regular` while treasurybase/tspend credits are also generated=true.

## QK-0521 — Orphan help keys

Spec: [18](specs/18-jsonrpc-methods-part2.md). Source: internal/rpchelp/helpdescs_en_US.go:679, 692, 797-800; internal/rpchelp/methods.go:27-135

helpdescs_en_US.go carries keys for parameters and methods that are not registered (purchaseticket-nosplittransaction, mixaccount-account, setbalancetomaintain-*, getbalancetomaintain-*, setticketmaxprice-*, getticketmaxprice-*, setgenerate-*); they never reach generated help.

## QK-0522 — Services stay bound to the first wallet

Spec: [19](specs/19-grpc-wallet-service.md). Source: internal/rpc/rpcserver/server.go:332-342; internal/loader/loader.go:79-104; dcrwallet.go:385-394

Service ready flags are never cleared and the loader drops its callbacks after the first load. After WalletLoaderService.CloseWallet, WalletService still passes the readiness gate and runs against the closed wallet; re-opening does not re-attach. A second StartWalletService call would panic.

## QK-0523 — API version doc mismatch

Spec: [19](specs/19-grpc-wallet-service.md). Source: internal/rpc/rpcserver/server.go:63-68, 322-329; rpc/documentation/api.md:3

Code reports 10.0.0; api.md header says 7.13.x. prerelease/build_metadata fields are documented and never set.

## QK-0524 — Empty case arms in errorCode map seven kinds to Unknown

Spec: [19](specs/19-grpc-wallet-service.md). Source: internal/rpc/rpcserver/server.go:85-119

Bug, IO, Encoding, ScriptFailure, Policy, DoubleSpend, Protocol have empty case bodies; Go does not fall through, so they are Unknown although the layout suggests IO->AlreadyExists, Encoding->DataLoss, the last four->Unavailable. Consensus and Deployment are not listed at all.

## QK-0525 — Raw errors bypass the kind table

Spec: [19](specs/19-grpc-wallet-service.md). Source: internal/rpc/rpcserver/server.go:1268-1271, 1908-1911, 2098-2102, 3875-3879, 3985, 4067, 4083, 4308; grpc@v1.83.2/server.go:1443-1451

Twelve handlers return wallet errors untranslated; grpc-go turns them into Unknown (or Canceled/DeadlineExceeded for context errors). GetAccountExtendedPubKey unknown account is Unknown (doc: NotFound); every PurchaseTickets wallet failure is Unknown (doc: FailedPrecondition for low balance).

## QK-0526 — Nested 'rpc error: code = ...' messages

Spec: [19](specs/19-grpc-wallet-service.md). Source: internal/rpc/rpcserver/server.go:991-994, 1234-1237, 826, 4289, 2058

Passing a status error through translateError or %v embeds its string form in the message: SweepAccount bad destination, ConstructTransaction bad change destination, ImportVotingAccountFromSeed/DiscoverUsage backend failures (as Unknown), SignMessages per-message error strings.

## QK-0527 — Aborted is documented but never produced

Spec: [19](specs/19-grpc-wallet-service.md). Source: rpc/documentation/api.md:527, 570, 620 (and others); internal/rpc/rpcserver/server.go:85-136

api.md lists 'Aborted: The wallet database is closed' for about twenty methods; no code path returns Aborted.

## QK-0528 — Spender special cases are dead code

Spec: [19](specs/19-grpc-wallet-service.md). Source: internal/rpc/rpcserver/server.go:2154-2163; wallet/udb/txquery.go:640-725

errors.Is(errors.NotExist, err) and errors.Is(errors.Invalid, err) have reversed arguments and are always false; the intended messages 'output is unspent' / 'output is not relevant to the wallet' are never sent. Codes happen to match via translateError; messages are the wallet's.

## QK-0529 — ConfirmationNotifications half-close and empty tip responses

Spec: [19](specs/19-grpc-wallet-service.md). Source: internal/rpc/rpcserver/server.go:2444-2509; wallet/notifications.go:820-988

A client half-close surfaces as io.EOF and ends the RPC with Unknown 'EOF'. An empty response is sent on every main-chain tip change even with nothing watched. Tip-change entries are in Go map order. Stake-invalidated block yields -1.

## QK-0530 — Temporary unlock replaces an existing unlock timeout

Spec: [19](specs/19-grpc-wallet-service.md). Source: wallet/wallet.go:1712-1839; internal/rpc/rpcserver/server.go:506-515

If the wallet was unlocked with a timeout (JSON-RPC walletpassphrase), any gRPC call carrying a passphrase replaces that timeout and the wallet is locked when the call returns. Two concurrent passphrase RPCs: the wallet locks when the second unlocker returns. A wrong passphrase locks an unlocked wallet.

## QK-0531 — Inverted rescan-height validation

Spec: [19](specs/19-grpc-wallet-service.md). Source: internal/rpc/rpcserver/server.go:684-687, 760-763, 804-817, 842-849

ImportPrivateKey, ImportScript and ImportVotingAccountFromSeed reject scan_from > 0 (and discover_from > 0) exactly when rescan (discover_usage) is true, with a message claiming the flag is unset. Rescan/discovery from a non-zero height is impossible; a non-zero height without the flag is silently accepted; the discover-from lookup is unreachable.

## QK-0532 — Import RPCs require a network backend and unlock before validating

Spec: [19](specs/19-grpc-wallet-service.md). Source: internal/rpc/rpcserver/server.go:661-692, 714-717, 765-776

ImportPrivateKey, ImportExtendedPublicKey and ImportScript fail FailedPrecondition without a backend even when no rescan is requested. ImportPrivateKey unlocks the wallet before validating account and scan arguments. ImportScript ignores its passphrase and treats Exist as success.

## QK-0533 — PurchaseTickets ignored fields and stale docs

Spec: [19](specs/19-grpc-wallet-service.md). Source: internal/rpc/rpcserver/server.go:1791-1940; wallet/vspclientloader.go:12-26; rpc/documentation/api.md:1731-1817

tx_fee is sign-checked then unused; ticket_fee is never read; api.md documents a non-existent spend_limit, mixing_enabled (actual enable_mixing), unsigned_tickets (actual tickets), repeated split_tx (actual bytes). VSP clients are cached by URL, so a later request with different pubkey/accounts for the same host reuses the first config.

## QK-0534 — FundTransaction/UnspentOutputs ignore include_immature_coinbases and skip validation

Spec: [19](specs/19-grpc-wallet-service.md). Source: internal/rpc/rpcserver/server.go:1057-1140; wallet/udb/txmined.go:3100-3345

The flag is never read; negative target_amount or required_confirmations are not rejected (doc: InvalidArgument); target 0 selects everything, negative selects nothing. The InsufficientBalance tolerance is never exercised by the wallet's input source.

## QK-0535 — Block range semantics differ from both proto comment and api.md

Spec: [19](specs/19-grpc-wallet-service.md). Source: internal/rpc/rpcserver/server.go:1346-1386; wallet/udb/txquery.go:388-421, 521-538, 727-761; rpc/api.proto:367-369, 1183

Heights pass through unchanged. Negative start means 'unmined first, then blocks descending from the tip to end' (only unmined if end is unset); start >= end >= 0 iterates descending; height 0 is indistinguishable from unset. Proto says negative = last N blocks; api.md says subtracted from tip. GetCFilters ending height is sint32 while api.md says int32.

## QK-0536 — minimum_recent_transactions validated then ignored

Spec: [19](specs/19-grpc-wallet-service.md). Source: internal/rpc/rpcserver/server.go:1388-1412

GetTransactions rejects the field in combination with an end block or when negative, then discards it (source carries a BUGS comment).

## QK-0537 — Go map order and randomness exposed in responses

Spec: [19](specs/19-grpc-wallet-service.md). Source: internal/rpc/rpcserver/server.go:3855, 4274, 1246-1248; wallet/wallet.go:5625-5630; wallet/udb/txmined.go:3104-3172; wallet/txauthor/author.go:171-181

GetCoinjoinOutputspByAcct, GetTrackedVSPTickets (VSP list and ticket lists), GetVSPTicketsByFeeStatus and ConfirmationNotifications tip updates are in map iteration order; FundTransaction/UnspentOutputs order comes from shuffled selection; ConstructTransaction swaps the change output to a random index. Differential tests must compare as sets.

## QK-0538 — Two different 'key count' definitions

Spec: [19](specs/19-grpc-wallet-service.md). Source: internal/rpc/rpcserver/server.go:403-407; wallet/notifications.go:600-627

Accounts reports LastReturnedIndex+1 (0xFFFFFFFF wraps to 0); AccountNotifications reports min(2^31, LastUsedIndex+gapLimit) with uint32 wraparound (unused branch gives gapLimit-1). Accounts also returns account_encrypted/account_unlocked, absent from api.md.

## QK-0539 — Status message embeds unstable protobuf text

Spec: [19](specs/19-grpc-wallet-service.md). Source: internal/rpc/rpcserver/server.go:1612-1616, 1667-1671

'Invalid transaction hash length for script <script>, expected 32 got <n>' formats a protobuf message with %v; protobuf-go text output is deliberately unstable across builds, so exact bytes cannot be pinned.

## QK-0540 — CreateSignature hash_type truncated to a byte

Spec: [19](specs/19-grpc-wallet-service.md). Source: internal/rpc/rpcserver/server.go:1747; dcrd/txscript/v4@v4.1.2/sighash.go:18

The enum's int32 value is cast to txscript.SigHashType (byte) with no validation.

## QK-0541 — SetAccountPassphrase always relocks the account

Spec: [19](specs/19-grpc-wallet-service.md). Source: internal/rpc/rpcserver/server.go:3884-3892

When it unlocks with account_passphrase, a deferred LockAccount runs regardless of the prior state and its error is assigned to a variable that is not returned.

## QK-0542 — VSP RPCs hide or over-report errors

Spec: [19](specs/19-grpc-wallet-service.md). Source: internal/rpc/rpcserver/server.go:4101-4117, 4225-4249; wallet/wallet.go:5670-5684, 5705-5747

SyncVSPFailedTickets skips every per-ticket error and returns OK. SetVspdVoteChoices returns Unknown whenever any unspent ticket has no VSP record (VSPHostForTicket NotExist is collected), although matching tickets were updated.

## QK-0543 — Streaming reads hold a database read transaction across Send

Spec: [19](specs/19-grpc-wallet-service.md). Source: internal/rpc/rpcserver/server.go:1423-1445, 1507-1543, 2186-2204; wallet/wallet.go:855-881, 3248-3276

GetTransactions, GetTickets and GetCFilters call Send from inside the wallet's read-only transaction; a slow client keeps it open. Cancellation is checked before send (GetTransactions), after each block (GetTickets) or after send (GetCFilters).

## QK-0544 — Rescan: two Canceled messages and a possible goroutine leak

Spec: [19](specs/19-grpc-wallet-service.md). Source: internal/rpc/rpcserver/server.go:442-489; wallet/rescan.go:419-443

A cancelled rescan returns either Canceled 'wallet.RescanProgressFromHeight: context canceled' or Canceled 'rescan canceled'. begin_hash is undocumented and is resolved to a main-chain height. If the handler exits on a Send error with a value buffered, the rescan goroutine blocks forever on its final send.

## QK-0545 — All gRPC library logs are info level; Fatal exits the process

Spec: [19](specs/19-grpc-wallet-service.md). Source: internal/rpc/rpcserver/log.go:26-81; grpc@v1.83.2/grpclog/internal/logger.go:33-87; rpcserver.go:406-448

The legacy grpclog adapter routes library info/warning/error output to slog Info on GRPC, stripping a leading 'grpc: '; Fatal* logs Critical and calls os.Exit(1). Interceptors log every unary error at error level and do not log readiness failures.

## QK-0546 — Client usage examples predate mandatory client certs

Spec: [19](specs/19-grpc-wallet-service.md). Source: rpc/documentation/clientusage.md:115-460; rpcserver.go:321-322; internal/cfgutil/curve.go:29

Non-Go examples in clientusage.md present no client certificate (handshake fails against the current server) and force GRPC_SSL_CIPHER_SUITES=HIGH+ECDSA claiming ECDSA certs are required, while the default --tlscurve is Ed25519.

## QK-0547 — Misc doc-vs-code naming errors

Spec: [19](specs/19-grpc-wallet-service.md). Source: rpc/documentation/api.md:1215, 1881, 2655, 1518, 1681, 881; rpc/documentation/serverchanges.md:76, 88; rpc/regen.sh

api.md names wrong response types (ImportVotingAccountFromSeed, SignMessages), wrong field names (pulic_key, unsigned_transaction, signature, tickets), omits vsp_host and SignedTransaction.unsigned_input_indexes, and lists 'Expected errors: None' for RPCs that can fail. serverchanges.md shows an obsolete protoc command and a dead ../rpcserver link.

## QK-0548 — Negative discover_gap_limit wraps

Spec: [19](specs/19-grpc-wallet-service.md). Source: internal/rpc/rpcserver/server.go:851-854; rpc/documentation/api.md:1230

ImportVotingAccountFromSeed converts a non-zero int32 discover_gap_limit to uint32 without a range check, so -1 becomes 4294967295; api.md documents InvalidArgument.

## QK-0549 — unmined_transactions is empty on reorganization

Spec: [19](specs/19-grpc-wallet-service.md). Source: wallet/notifications.go:318-323; rpc/documentation/api.md:2040-2042; rpc/api.proto:691-695

api.md and the api.proto comment say transactions returned to the mempool by a reorg are included in unmined_transactions; the wallet intentionally leaves the field unset in main-chain notifications, so clients must use unmined_transaction_hashes.

## QK-0550 — Stale SignTransactionRequest comment

Spec: [19](specs/19-grpc-wallet-service.md). Source: rpc/api.proto:560-565; internal/rpc/rpcserver/server.go:1623-1644

The proto comment describes the removed input_indexes field and says the RPC errors when an input cannot be signed; the handler attempts every input and reports failures in unsigned_input_indexes with status OK.

## QK-0551 — Latent nil dereference behind InsufficientBalance tolerance

Spec: [19](specs/19-grpc-wallet-service.md). Source: internal/rpc/rpcserver/server.go:1062-1067, :1105-1110

FundTransaction and UnspentOutputs ignore an InsufficientBalance error from SelectInputs and then dereference the returned input detail unconditionally.

## QK-0552 — Zero-required-signature multisig reported redeemable

Spec: [19](specs/19-grpc-wallet-service.md). Source: internal/rpc/rpcserver/server.go:738-749

ImportScript computes redeemable as multisig && ownAddrs >= requiredSigs with uint16 counters and explicitly does not reject requiredSigs == 0.

## QK-0553 — Misleading enum comment before scProto

Spec: [19](specs/19-grpc-wallet-service.md). Source: internal/rpc/rpcserver/server.go:3646-3697, :2082

The comment block quotes a ScriptClass enum (NON_STANDARD ... TADD) belonging to DecodedTransaction, while scProto's result is also cast to ValidateAddressResponse.ScriptType with CamelCase names.

## QK-0554 — CreateWallet birthday flags are crossed

Spec: [20](specs/20-grpc-other-services.md). Source: internal/rpc/rpcserver/server.go:2717-2742

The second birthday block tests set_birth_time where set_birth_height was meant. birth_time is validated (must be at least 24h in the past) then discarded; birth_height is honoured only together with set_birth_time; set_birth_height alone writes height 0. Stored value is always 0x01, 32 zero bytes, height BE, then ff ff ff f1 88 6e 09 00 (Go zero time).

## QK-0555 — DecodeRawTransaction never sets commitment_amount

Spec: [20](specs/20-grpc-other-services.md). Source: internal/rpc/rpcserver/server.go:3612-3615

The commitment amount is assigned only when AmountFromSStxPkScrCommitment returns an error (inverted test), in which case it is 0. The field is always absent for well-formed tickets.

## QK-0556 — Reopened wallet is not rebound to wallet-dependent services

Spec: [20](specs/20-grpc-other-services.md). Source: internal/loader/loader.go:81-89; dcrwallet.go:388-394

Loader.onLoaded discards its callbacks after the first load and the gRPC service-start callback is registered once. After CloseWallet then OpenWallet/CreateWallet, WalletService, TicketBuyer, AccountMixer, Network and Voting services still point at the first, closed wallet; only loader RPCs and JSON-RPC see the new one.

## QK-0557 — Service readiness is never revoked

Spec: [20](specs/20-grpc-other-services.md). Source: internal/rpc/rpcserver/server.go:303-320, 2518-2528

After CloseWallet the gated services stay ready and fail inside the wallet with database-closed errors instead of FailedPrecondition 'is not ready'.

## QK-0558 — Lock-on-exit of RunTicketBuyer/RunAccountMixer is ineffective

Spec: [20](specs/20-grpc-other-services.md). Source: internal/rpc/rpcserver/server.go:2542-2557, 2662-2677; ticketbuyer/tb.go:65; wallet/wallet.go:1804-1832

The handler unlocks with a timeout channel, then ticketbuyer.Run unlocks again with a nil timeout, cancelling the first timeout. The deferred lockWallet only feeds a drained channel, so the wallet stays unlocked without a time limit after the RPC ends.

## QK-0559 — CloseWallet stops nothing but the database

Spec: [20](specs/20-grpc-other-services.md). Source: internal/loader/loader.go:375-393

UnloadWallet only closes the bbolt handle. Running RpcSync/SpvSync streams, ticket buyers and mixers are not cancelled, the wallet is not locked and its network backend is not cleared, contrary to the comment and api.md.

## QK-0560 — MixSplitLimit dropped for CreateNewWallet

Spec: [20](specs/20-grpc-other-services.md). Source: internal/loader/loader.go:259-274

The wallet.Config built in CreateNewWallet omits MixSplitLimit, so a wallet created over gRPC or --create runs with limit 0 until reopened; the other two load paths pass it.

## QK-0561 — Failed creation removes wallet.db without closing it

Spec: [20](specs/20-grpc-other-services.md). Source: internal/loader/loader.go:146-150, 236-240

On error after CreateDB the loader calls os.Remove while the bbolt handle is still open and never closes it. Unix leaks the descriptor; Windows behaviour unverified (remove of an open file normally fails).

## QK-0562 — Concurrent OpenWallet can block forever

Spec: [20](specs/20-grpc-other-services.md). Source: internal/loader/loader.go:298-302; wallet/internal/bdb/db.go:321

OpenExistingWallet releases the mutex around wallet.OpenDB and does not re-check afterwards; bbolt open has no timeout, so a second concurrent open blocks on the file lock until the first wallet is closed, and is not released by client cancellation.

## QK-0563 — Ticket commitment outputs classed STAKE_SUBMISSION

Spec: [20](specs/20-grpc-other-services.md). Source: internal/rpc/rpcserver/server.go:3597-3611

Every odd-index output of a ticket is reported with script_class STAKE_SUBMISSION and a single commitment address; the source itself marks this 'Questionable'. JSON-RPC reports sstxcommitment.

## QK-0564 — FetchHeadersNotification differs per syncer and docs are wrong on units

Spec: [20](specs/20-grpc-other-services.md). Source: chain/sync.go:294; spv/sync.go:263-266; rpc/documentation/api.md:2425

RpcSync sends the count of headers newly added in the batch; SpvSync sends the height of the last header. Both send Unix seconds; api.md says nanoseconds.

## QK-0565 — discover_accounts only unlocks the wallet

Spec: [20](specs/20-grpc-other-services.md). Source: chain/sync.go:82; spv/sync.go:144; internal/rpc/rpcserver/server.go:2848-2863

Both syncers decide account discovery from !wallet.Locked() at construction. A wallet already unlocked discovers accounts even with discover_accounts=false. If no rescan is needed, DiscoverAddressesFinished never fires and the wallet stays unlocked until the RPC ends.

## QK-0566 — gRPC-started syncers bypass proxy and most network config

Spec: [20](specs/20-grpc-other-services.md). Source: internal/rpc/rpcserver/server.go:2865-2872, 3014-3016; p2p/peering.go:169-172; dcrwallet.go:531-600

SpvSync creates a LocalPeer with the default net.Dialer and RpcSync leaves RPCOptions.Dial nil, so --proxy/Tor is bypassed. They also ignore --spvdisablerelaytx, --spvconnect, --rpcconnect, --cafile, dcrd client certs and --noclienttls, and do not retry.

## QK-0567 — Status codes differ from api.md

Spec: [20](specs/20-grpc-other-services.md). Source: internal/rpc/rpcserver/server.go:2973-2979, 3535-3540; rpc/documentation/api.md:175, 390-407, 2983

Create/Open while loaded is AlreadyExists (doc: FailedPrecondition); RpcSync bad address, bad credentials and unreachable dcrd are all Unknown (doc: InvalidArgument/Unavailable); wrong private passphrase is InvalidArgument; VerifyMessage rejects P2PK; birth_time must be a day in the past, not 'ahead'.

## QK-0568 — SetTSpendPolicy returns wallet errors untranslated

Spec: [20](specs/20-grpc-other-services.md). Source: internal/rpc/rpcserver/server.go:3439-3442

Unlike SetTreasuryPolicy, the handler returns the raw wallet error, so grpc-go reports Unknown (or Canceled/DeadlineExceeded for context errors) with the raw text.

## QK-0569 — Go map iteration order is observable

Spec: [20](specs/20-grpc-other-services.md). Source: wallet/wallet.go:280, 505, 645-659

SetAgendaChoices iterates the choice map, so with several invalid entries the reported error is random. TSpendPolicies (list form) and TreasuryPolicies return entries in map order.

## QK-0570 — errorCode empty case arms look like fall-through but are not

Spec: [20](specs/20-grpc-other-services.md). Source: internal/rpc/rpcserver/server.go:88-118

Bug, IO, Encoding, ScriptFailure, Policy, DoubleSpend and Protocol have empty case bodies; Go does not fall through, so they (and Consensus, Deployment) map to Unknown.

## QK-0571 — Streams withhold headers; send errors ignored

Spec: [20](specs/20-grpc-other-services.md). Source: internal/rpc/rpcserver/server.go:2883, 2557, 2677

grpc-go sends headers with the first message. RunTicketBuyer/RunAccountMixer never send one, so clients see nothing until the trailers-only terminal status. Sync notification send errors are discarded.

## QK-0572 — api.md, clientusage.md and serverchanges.md are stale

Spec: [20](specs/20-grpc-other-services.md). Source: rpc/documentation/api.md:3, 34-43, 361-386, 2895-2896; rpc/documentation/serverchanges.md:76

api.md omits AccountMixerService from its index and all four treasury/tspend policy RPCs, documents non-existent RpcSync fields spv_connect and syncing_status, and misnames fields. Non-Go client examples present no client certificate and cannot connect. serverchanges.md shows a removed protoc plugin invocation.

## QK-0573 — RunTicketBuyer VSP client cached by URL

Spec: [20](specs/20-grpc-other-services.md). Source: wallet/vspclientloader.go:12-26; internal/rpc/rpcserver/server.go:2596-2613

wallet.VSP returns an existing client for the same URL unchanged, so a later call with a different public key or account policy silently reuses the first. VSP validation runs before the negative-balance check; account and voting_account are never validated.

## QK-0574 — RunAccountMixer validates nothing

Spec: [20](specs/20-grpc-other-services.md). Source: internal/rpc/rpcserver/server.go:2531-2540

Non-existent accounts and branches other than 0/1 are accepted; failures only appear in the log on each new block.

## QK-0575 — isLoopback edge cases

Spec: [20](specs/20-grpc-other-services.md). Source: internal/rpc/rpcserver/server.go:2817-2830

Only exact lower-case 'localhost' or a literal loopback IP count; '[::1]' without a port, 'LOCALHOST' and host names resolving to loopback do not, so TLS is attempted for them.

## QK-0576 — GetRawBlock empty request returns genesis

Spec: [20](specs/20-grpc-other-services.md). Source: internal/rpc/rpcserver/server.go:3813-3827

With no block_hash the handler looks up block_height (default 0) in the main chain; a negative height is representable (sint32) and fails in the lookup.

## QK-0577 — Zero-length vs absent bytes relies on Go protobuf decoding

Spec: [20](specs/20-grpc-other-services.md). Source: internal/rpc/rpcserver/server.go:3363, 3413, 3814; protobuf@v1.36.11/internal/impl/codec_gen.go:5489-5500

Handlers test request bytes with != nil; this equals a length test only because protobuf-go decodes empty proto3 bytes to nil.

## QK-0578 — Inconsistent loader errors between create paths

Spec: [20](specs/20-grpc-other-services.md). Source: internal/loader/loader.go:115, 130, 140, 205, 220, 230

The not-a-directory error has kind Invalid in CreateWatchingOnlyWallet but no kind in CreateNewWallet (InvalidArgument vs Unknown); 'already loaded' and 'file exists' texts differ between entry points.

## QK-0579 — Dead protobuf messages remain in api.proto

Spec: [20](specs/20-grpc-other-services.md). Source: rpc/api.proto:760-788

Ten request/response messages from a removed multi-step loader API (StartConsensusRpc, DiscoverAddresses, FetchMissingCFilters, SubscribeToBlockNotifications, FetchHeaders) are referenced by no RPC but are part of the descriptor.

## QK-0580 — SetTreasuryPolicy comment claims VSP forwarding that gRPC does not do

Spec: [20](specs/20-grpc-other-services.md). Source: internal/rpc/rpcserver/server.go:3477-3516; internal/rpc/jsonrpc/methods.go:3924-3928

The handler comment says preferences are also set with a configured VSP; the gRPC code only writes locally. The JSON-RPC handler does forward.

## QK-0581 — Already-synchronizing guard is neither atomic nor permanent

Spec: [20](specs/20-grpc-other-services.md). Source: internal/rpc/rpcserver/server.go:2843-2846, :2993-2996; chain/sync.go:610-611; spv/sync.go:396-397; dcrwallet.go:531-600

RpcSync/SpvSync test wallet.NetworkBackend() but the backend is set by the syncer only once connected and cleared on return. Two sync RPCs issued close together both pass; the first to exit clears the backend while the other runs; with the main-package loop active the guard is open during its 5 s retry sleeps.

## QK-0582 — Loader doc comments disagree with code

Spec: [20](specs/20-grpc-other-services.md). Source: internal/loader/loader.go:79-80, :91-93, :195-197, :284-287, :371-374; internal/loader/doc.go

onLoaded claims to prevent further loads (it only drops callbacks); OpenExistingWallet documents a non-existent canConsolePrompt parameter; UnloadWallet says it stops the wallet and names a non-existent LoadExistingWallet; RunAfterLoad claims single-goroutine ordered execution but runs late registrations immediately on the caller's goroutine outside the mutex; doc.go claims the package manages services like the ticket buyer.

## QK-0583 — Explicit codes.Unknown bypasses translateError

Spec: [20](specs/20-grpc-other-services.md). Source: internal/rpc/rpcserver/server.go:2562, :2682, :2978

RunAccountMixer, RunTicketBuyer and RpcSync build Unknown statuses directly with a handler prefix, so wallet error kinds inside the wrapped error (Locked, Passphrase, NoPeers, Invalid) are never mapped to their usual gRPC codes for these three RPCs.

## QK-0584 — serviceName slices without checking for a missing slash

Spec: [20](specs/20-grpc-other-services.md). Source: rpcserver.go:399-404; grpc@v1.83.2/server.go:1787, :1830

method[:strings.IndexRune(method, '/')] would panic on -1; unreachable only because grpc-go rejects such paths first with Unimplemented 'malformed method name'. A Rust interceptor on raw paths needs its own guard and the same status.

## QK-0585 — api.md calls itself authoritative but diverges further

Spec: [20](specs/20-grpc-other-services.md). Source: rpc/documentation/api.md:16-17, :122, :273, :349-351, :2792, :2827, :2959-2962, :2994-2997, :3068-3070

Beyond the existing Q14/Q20 lists: WalletExists documented as never failing, ticket buyer/mixer documented with a 'Wallet has not been loaded' error whose real text is the interceptor's 'service ... is not ready', incomplete error lists for SetVoteChoices, VerifyMessage, GetRawBlock and RescanPoint, and a loopback definition narrower than isLoopback.

## QK-0586 — No --lifetimeevents in dcrwallet

Spec: [21](specs/21-config-cli-lifecycle.md). Source: config.go:164-168; ipc.go:116-203

The wallet has only --pipetx, --piperx, --rpclistenerevents and --issueclientcert. There is no lifetime-event message; the listener events are the only readiness signal a parent gets. The task brief's mention of --lifetimeevents applies to dcrd only.

## QK-0587 — Config file located under the unexpanded --appdata

Spec: [21](specs/21-config-cli-lifecycle.md). Source: config.go:423-426 vs config.go:453

When --appdata is given without --configfile, the config path is Join(appdata as typed, dcrwallet.conf) with no ~ or $VAR expansion, while every other path uses the expanded directory.

## QK-0588 — Command-line list options replace config-file lists

Spec: [21](specs/21-config-cli-lifecycle.md). Source: go-flags@v1.5.0/option.go:245-251; go-flags@v1.5.0/parser.go:210-213

go-flags clears a slice option on its first set in each parse pass, so --rpclisten/--grpclisten/--spvconnect/--profile on the command line discard the file's values instead of appending.

## QK-0589 — Values starting with a double quote are Go-unquoted

Spec: [21](specs/21-config-cli-lifecycle.md). Source: go-flags@v1.5.0/ini.go:448-464; go-flags@v1.5.0/parser.go:542-547

In both dcrwallet.conf and argv a value beginning with " is run through strconv.Unquote; a password like "abc is a syntax error and escapes are interpreted.

## QK-0590 — INI sections applied in random order

Spec: [21](specs/21-config-cli-lifecycle.md). Source: go-flags@v1.5.0/ini.go:508

go-flags iterates a Go map of sections; the outcome is random only when one option is set in two sections or two sections both contain errors. Port should use file order (dcroxide made the same choice).

## QK-0591 — Booleans cannot be disabled from the command line

Spec: [21](specs/21-config-cli-lifecycle.md). Source: go-flags@v1.5.0/parser.go:522-526

--flag=false is rejected ('bool flag `--x' cannot have an argument') and there is no negated form, so a boolean set in the config file can only be turned off by editing the file.

## QK-0592 — Cancelled --create exits 0 and may leave the terminal without echo

Spec: [21](specs/21-config-cli-lifecycle.md). Source: walletsetup.go:214-221; dcrwallet.go:52-54; x/term@v0.46.0/term_unix.go:74-91

Ctrl-C during the creation prompts prints 'Unable to create wallet: context canceled' and exits with status 0 because main treats context.Canceled as success; the abandoned prompt goroutine may be inside term.ReadPassword, whose terminal restore only runs on return (derived from code, not run).

## QK-0593 — --createwatchingonly cannot be interrupted while prompting

Spec: [21](specs/21-config-cli-lifecycle.md). Source: walletsetup.go:328-341; signal.go:47-71

Its prompts run on the main goroutine without watching the context; SIGINT is consumed by shutdownListener, which only logs.

## QK-0594 — --createtemp misuse exits 0

Spec: [21](specs/21-config-cli-lifecycle.md). Source: config.go:577-589

Missing explicit --appdata or missing --simnet prints a message to stderr and calls os.Exit(0).

## QK-0595 — Prompt loops spin forever at EOF on non-terminal stdin

Spec: [21](specs/21-config-cli-lifecycle.md). Source: internal/prompt/prompt.go:133-145; internal/prompt/prompt.go:312-363

PassPrompt treats EOF as an empty passphrase and re-prompts endlessly; the existing-seed loop does the same.

## QK-0596 — Existing-seed prompt swallows subsequent piped answers

Spec: [21](specs/21-config-cli-lifecycle.md). Source: internal/prompt/prompt.go:316-335; walletsetup.go:160-212

Seed entry reads through a new bufio.Scanner layered on the shared reader; bytes the scanner buffered past the seed are lost, so with fully piped stdin the birthday and account-import prompts hit EOF and take defaults. Depends on OS read chunking.

## QK-0597 — Answering 'no' to public encryption ignores --walletpass

Spec: [21](specs/21-config-cli-lifecycle.md). Source: internal/prompt/prompt.go:202-212

PublicPass returns the default 'public' when the user declines, even if a different --walletpass is configured.

## QK-0598 — Partial wallet creation is not rolled back

Spec: [21](specs/21-config-cli-lifecycle.md). Source: walletsetup.go:229-262

After loader.CreateNewWallet succeeds, a failure in the SLIP0044 upgrade, xpub account import (e.g. reserved name) or birthday write returns an error but leaves wallet.db, so a retry fails with 'already exists'.

## QK-0599 — --logsize units and edge cases

Spec: [21](specs/21-config-cli-lifecycle.md). Source: config.go:500-534; logrotate@v1.0.0/rotator/rotator.go:75,132-146

A bare number is invalid; K/M/G produce KiB but the rotator multiplies by 1000 (10M = 10,240,000 bytes); '0K' rotates after every line; error text ends with a trailing space; not validated under --nofilelogging.

## QK-0600 — Missing default config file warns on every start

Spec: [21](specs/21-config-cli-lifecycle.md). Source: config.go:429-437,553-565

A missing (or unreadable, or directory) default dcrwallet.conf is only a WRN log line; it is fatal only when --configfile was given explicitly.

## QK-0601 — --debuglevel applies partially on error

Spec: [21](specs/21-config-cli-lifecycle.md). Source: config.go:294-319; slog@v1.2.0/log.go:110-129

Subsystem=level pairs are applied as they are validated; a later invalid pair leaves earlier ones in effect. Extra '=' fields are ignored. 'off' and 3-letter level names are accepted though undocumented.

## QK-0602 — Solver probe passes for a solver that exited

Spec: [21](specs/21-config-cli-lifecycle.md). Source: testsolver.go:18-36; cspp/v2@v2.4.0/solverrpc/rpc.go:83-162

testStartedSolverWorks fails only on io.ErrUnexpectedEOF or rpc.ErrShutdown, but solverrpc substitutes ErrSolverProcessExited once it sees the child exit, so the check is timing-dependent; StartSolver's first result is cached (sync.Once).

## QK-0603 — Dead code

Spec: [21](specs/21-config-cli-lifecycle.md). Source: config.go:357,730; internal/cfgutil/address.go:15-59; internal/prompt/prompt.go:29-72; walletsetup.go:38-39

cfg.lookup (Tor lookup) is assigned and never read; cfgutil.AddressFlag, prompt.ProvideSeed, prompt.ProvidePrivPassphrase and the testnet2 branch of networkDir are unused.

## QK-0604 — Localhost-only checks are textual

Spec: [21](specs/21-config-cli-lifecycle.md). Source: config.go:797-815,912-953

--noclienttls/--noservertls accept only the literal hosts localhost, 127.0.0.1, ::1 (not 127.0.0.2, not an empty host); the shared-listener check compares normalised strings so localhost:9110 vs 127.0.0.1:9110 is not detected.

## QK-0605 — --rpclisten=[::1] rejected

Spec: [21](specs/21-config-cli-lifecycle.md). Source: internal/cfgutil/normalization.go:12-27

NormalizeAddress returns 'missing port in address' for a bracketed IPv6 literal without a port, while '::1' and '[::1]:9110' work.

## QK-0606 — --noinitialload silently ignores many options

Spec: [21](specs/21-config-cli-lifecycle.md). Source: dcrwallet.go:194,417

--pass, --promptpass, --promptpublicpass, --enableticketbuyer, --mixchange, VSP options, --spv, --offline and dcrd connection settings have no effect at startup because the whole load/sync block is skipped.

## QK-0607 — Sync loop nil-return behaviour

Spec: [21](specs/21-config-cli-lifecycle.md). Source: dcrwallet.go:544-547,586-598

rpcSyncLoop re-runs the syncer immediately with no delay or log if Run returns nil; spvLoop stops syncing permanently if Run returns nil.

## QK-0608 — dcrd CA and client cert read once

Spec: [21](specs/21-config-cli-lifecycle.md). Source: dcrwallet.go:563-564,602-636

readCAFile and readClientCertKey run before the reconnect loop, so certificates created after wallet start are not picked up until restart.

## QK-0609 — loader.CreateNewWallet omits MixSplitLimit

Spec: [21](specs/21-config-cli-lifecycle.md). Source: internal/loader/loader.go:259-274

The wallet.Config built for a newly created wallet leaves MixSplitLimit at 0, unlike OpenExistingWallet and CreateWatchingOnlyWallet.

## QK-0610 — Loader callbacks fire once and under the mutex

Spec: [21](specs/21-config-cli-lifecycle.md). Source: internal/loader/loader.go:81-104,299-302

onLoaded runs queued callbacks while holding the loader mutex and then discards them; a wallet reloaded over gRPC after unload triggers no callbacks. OpenExistingWallet also drops the mutex around OpenDB without re-checking state.

## QK-0611 — --issueclientcert without --pipetx

Spec: [21](specs/21-config-cli-lifecycle.md). Source: rpcserver.go:291-308; dcrwallet.go:101-105

An ephemeral client CA is generated and trusted, but the client key/cert message is drained and lost.

## QK-0612 — gRPC listener events are unordered

Spec: [21](specs/21-config-cli-lifecycle.md). Source: rpcserver.go:338-346; config.go:870-891

Each grpclistener message is sent from its listener's goroutine; order among them and relative to jsonrpclistener messages is nondeterministic. Default listener set order follows net.LookupHost('localhost').

## QK-0613 — Amount parsing leniency

Spec: [21](specs/21-config-cli-lifecycle.md). Source: internal/cfgutil/amount.go:32-44; dcrd/dcrutil/v4@v4.0.3/amount.go:61-91

AmountFlag accepts negatives, inf/nan spellings (rejected after parse), hex floats; --txfee=-1 passes loadConfig; float-to-int64 overflow is platform-defined in Go.

## QK-0614 — --circuitlimit cast to uint32

Spec: [21](specs/21-config-cli-lifecycle.md). Source: config.go:703; go-socks@v1.1.0/socks/pool.go:39-45

Negative values wrap; 0 makes every proxied dial fail with 'maximum number of connections reached'.

## QK-0615 — Go runtime text in user-visible output

Spec: [21](specs/21-config-cli-lifecycle.md). Source: config.go:410-414; dcrwallet.go:89-90,107-157; slog@v1.2.0/log.go:72-88; go-flags@v1.5.0/parser.go:220-234

-V output and the first log line contain runtime.Version() and GOOS/GOARCH; --profile/--cpuprofile/--memprofile produce Go pprof data; LOGFLAGS=longfile prints Go source positions; GO_FLAGS_COMPLETION turns the process into a completion helper.

## QK-0616 — sample-dcrwallet.conf disagrees with the code

Spec: [21](specs/21-config-cli-lifecycle.md). Source: sample-dcrwallet.conf:27-28,65-67,114,175; config.go:155,363

Documents a non-existent 'legacyrpclisten' key (fatal if uncommented), shows tlscurve=P-256 while the default is Ed25519, names the testnet directory 'testnet' (actual: testnet3), and claims a proxy disables RPC listening (not implemented).

## QK-0617 — --createwatchingonly works without --create

Spec: [21](specs/21-config-cli-lifecycle.md). Source: config.go:631,649-653; docs/offline_wallets.md:12-14

Either flag triggers creation and watching-only wins when both are given; docs/offline_wallets.md documents '--create --createwatchingonly'.

## QK-0618 — Positional arguments silently ignored

Spec: [21](specs/21-config-cli-lifecycle.md). Source: dcrwallet.go:81

loadConfig returns remaining args and run discards them; 'dcrwallet foo' starts normally.

## QK-0619 — Imported xpub accounts are dropped when stdin ends without a final 'no'

Spec: [21](specs/21-config-cli-lifecycle.md). Source: internal/prompt/prompt.go:419-422, walletsetup.go:207-212

prompt.ImportedAccounts returns (nil, nil, err) on every read error and createWallet converts io.EOF to success, so accounts already accepted (and announced with 'Importing account ...') are not imported.

## QK-0620 — Startup passphrase prompts lose piped read-ahead

Spec: [21](specs/21-config-cli-lifecycle.md). Source: dcrwallet.go:451-464

passPrompt builds a new bufio.Reader over stdin for every prompt; with non-terminal stdin the first reader can swallow the next prompt's line, leaving the following prompt at EOF (where it spins).

## QK-0621 — localhost listener accepted by config, unbindable at runtime

Spec: [21](specs/21-config-cli-lifecycle.md). Source: config.go:797-801,943; rpcserver.go:484-487

loadConfig normalises and whitelists the host name 'localhost', but makeListeners skips any non-IP-literal host with a warning; a sole rpclisten=localhost yields 'failed to create listeners for JSON-RPC server'.

## QK-0622 — Shutdown during wallet open

Spec: [21](specs/21-config-cli-lifecycle.md). Source: dcrwallet.go:207-230; internal/loader/loader.go:291-342

run returns on cancellation while the open goroutine continues; the deferred UnloadWallet either blocks on the loader mutex until wallet.Open finishes, or (in the OpenDB window where the mutex is released) returns Invalid silently and main returns with the open still in flight.

## QK-0623 — --mixing requirements are not enforced at config time

Spec: [21](specs/21-config-cli-lifecycle.md). Source: dcrwallet.go:326-338; config.go:760-782

Neither --mixedaccount nor --changeaccount is required by loadConfig despite the help text; the only enforcement is an account lookup of the empty name ('mixedaccount: account "" does not exist') when the ticket buyer or change mixer starts.

## QK-0624 — grpc-go fatal log records exit the process without cleanup

Spec: [21](specs/21-config-cli-lifecycle.md). Source: internal/rpc/rpcserver/log.go:56-81

The grpclog adapter maps Fatal/Fatalf/Fatalln to a CRT log line followed by os.Exit(1); the log rotator is not closed and no deferred shutdown runs. All other library levels are emitted at INF.

## QK-0625 — Error separator is set by package main, not the library

Spec: [22](specs/22-aux-packages-tools.md). Source: errors/errors.go:27; dcrwallet.go:36-39

errors.Separator defaults to ":\n\t" (multi-line); the daemon overrides it to ":: " in init. Tests, tools and library users therefore see different renderings from RPC clients. A port needs a process-wide setting with the daemon value ":: " and the multi-line form for ported tests.

Disposition: reproduced. `purse-errors` keeps one process-wide separator, defaulting to the multi-line form; `render` takes one explicitly. Confirmed against the oracle under both separators.

## QK-0626 — E() kind promotion leaks from discarded errors

Spec: [22](specs/22-aux-packages-tools.md). Source: errors/errors.go:202-207; errors/errors_test.go:72-81

An *Error argument copies its Kind into the new error immediately; if a later string/error argument replaces Err, the kind stays (E(E(Invalid,"abc"),"def") = {Invalid,"def"}). With two *Error arguments the first one's kind wins. Pinned by an upstream test.

## QK-0627 — E() ignores unknown argument types including nil error

Spec: [22](specs/22-aux-packages-tools.md). Source: errors/errors.go:194-211

E(op, err) with err == nil returns a non-nil *Error that renders as the Op alone; unknown-typed args are silently dropped. A port must not convert this to 'returns nil/None'.

## QK-0628 — errors.match and E can panic on edge inputs

Spec: [22](specs/22-aux-packages-tools.md). Source: errors/errors.go:204-205,376,385

match() calls Error() on a nil interface when the template has a non-*Error Err and the candidate's Err is nil (e.g. Is(E(Op("x"),Invalid), E("text"))), and uses interface equality that panics on uncomparable dynamic types; E((*Error)(nil)) dereferences nil. None occur at the pin; a Rust port should return false and record the divergence.

Disposition: diverges, as the standing rule on Go panics requires, but not by returning false outright: the `*Error` level at which Go's `match` would panic does not match and the walk continues, so `is` can be true where Go panics (for example when a later member of a join matches). The differential excludes exactly those queries; unit tests pin the Rust result. A typed-nil `*Error` cannot be constructed.

## QK-0629 — RPC error code comes from the outermost classified kind

Spec: [22](specs/22-aux-packages-tools.md). Source: errors/errors.go:295-323; internal/rpc/jsonrpc/errors.go:27-44

errors.As(err,&kind) stops at the first *Error with a non-Other Kind walking outward-in; leaf *Errors unwrap to their Kind value. An intermediate caller that sets its own kind therefore overrides the inner classification for JSON-RPC/gRPC code mapping.

## QK-0630 — Argon2 thread count truncates to 0 on machines with >=256 CPUs

Spec: [22](specs/22-aux-packages-tools.md). Source: kdf/kdf.go:31,35,56-65; x/crypto@v0.57.0/argon2/argon2.go:106-111

NewArgon2idParams uses uint8(min(NumCPU,256)); 256 becomes 0 and DeriveKey then panics with 'argon2: parallelism degree too low'. UnmarshalBinary also accepts any 25 bytes without range checks, so a corrupted record panics at unlock.

## QK-0631 — lru capacity is limit+1 and Map.Add keeps stale values

Spec: [22](specs/22-aux-packages-tools.md). Source: lru/cache.go:48; lru/map.go:46-53,94-101

Eviction is tested with len > limit before inserting, so the structure holds limit+1 entries; Map.Add on an existing key only promotes it and does not store the new value; Map.Contains doc says it modifies priority but it does not. Not interchangeable with dcrd container/lru (dcroxide-containers).

## QK-0632 — Mnemonic input split on ASCII space only; one word means hex

Spec: [22](specs/22-aux-packages-tools.md). Source: walletseed/seed.go:73-82

DecodeUserInput splits on ' ' only, so newline/tab separated mnemonics fail (hex error or 'word a\nb is not in the PGP word list'); a single-word input is always parsed as hex. gRPC DecodeSeed callers get this raw behaviour; the interactive prompt joins lines first.

Disposition: reproduced. Confirmed against the oracle, including the consequence that an unknown-word error can quote a whole run of newline- or tab-separated seed words (see QK-0729).

## QK-0633 — Mnemonic case folding uses Go simple Unicode lowercasing

Spec: [22](specs/22-aux-packages-tools.md). Source: pgpwordlist/pgpwordlist.go:46; go1.27.4/src/strings/strings.go ToLower

strings.ToLower maps U+212A (Kelvin sign) to 'k' and U+0130 to 'i', so e.g. 'İstanbul' matches. Rust str::to_lowercase maps U+0130 to two code points and would reject it.

## QK-0634 — Seed-length error carries different kinds

Spec: [22](specs/22-aux-packages-tools.md). Source: walletseed/seed.go:23-28,98-100; internal/prompt/prompt.go:271; internal/rpc/rpcserver/server.go:3177-3197

GenerateRandomSeed wraps ErrInvalidSeedLen as Invalid, DecodeUserInput as Encoding; the 'size >= 255' check in GenerateRandomSeed is dead code. The daemon itself never calls GenerateRandomSeed (prompt, simnet setup and gRPC generate seeds directly).

## QK-0635 — cmd/movefunds panics at the pin

Spec: [22](specs/22-aux-packages-tools.md). Source: cmd/movefunds/main.go:173,189,223-224; wallet/txauthor/author.go:97

It passes a nil ChangeSource to txauthor.NewUnsignedTransaction, which calls fetchChange.ScriptSize() unconditionally (nil-pointer panic, exit 2). Also: handled errors print without newline and exit 0; config 'txfee' is converted as DCR (sample value = 10,000,000 DCR); sign.sh's 'scriptpubkey' carries the empty signature script.

## QK-0636 — cmd/sweepaccount cannot work at the pin

Spec: [22](specs/22-aux-packages-tools.md). Source: cmd/sweepaccount/main.go:189,231,322,420,446,452; gorilla/websocket@v1.5.1/client.go:173-185

(a) wsrpc.Dial is given 'host:port' with no wss:// scheme or /ws path, which gorilla rejects; (b) signrawtransaction is sent a *wire.MsgTx (JSON object, no marshaller) instead of hex; (c) redeemScriptSizes is never filled so the fee is computed for a 51-byte tx (510 atoms at default rate) regardless of inputs. Also summary noun chosen by error count, skipped groups counted as published, TxOut[0] panic when change is dust. The wallet's own sweepaccount RPC supersedes it.

## QK-0637 — repaircfilters latent bugs and doc drift

Spec: [22](specs/22-aux-packages-tools.md). Source: cmd/repaircfilters/main.go:94-104,193-195,204-211; cmd/repaircfilters/README.md:25,33

Read-error check tests 'err' instead of 'ioErr' (non-EOF read errors ignored, can spin); uint16 arithmetic overflows for record lengths 65534/65535; README shows '-p' but the flag is '-P' and says 'Requires go 1.13'; --cafile alone is insufficient when the default cert is missing because the --connect default was cleared; -h exits with status 1; empty file prints 'Height: -1'.

## QK-0638 — treasurykey WIF checksum is single BLAKE-256

Spec: [22](specs/22-aux-packages-tools.md). Source: dcrutil/v4@v4.0.3/wif.go:94-95,178-179; cmd/treasurykey/treasurykey.go:21-22,59-63

dcrutil's WIF doc comment says double SHA-256 but the code uses chainhash.HashB (one BLAKE-256). The 'one flag' error goes to stdout while usage goes to stderr; signature type byte is 2 (Schnorr secp256k1).

## QK-0639 — Typed Go client has drifted from the server

Spec: [22](specs/22-aux-packages-tools.md). Source: rpc/client/dcrwallet/methods.go:258-287,492,505,516,694; internal/rpc/jsonrpc/methods.go:85-200; rpc/jsonrpc/types/methods.go:666-673

ListReceivedByAccount* call 'listreceivedbyacccount' (three c's) and AddTicket calls 'addticket'; neither is in the server handler table. PurchaseTicket sends a bool and a number where the server expects comment (string) and dontsigntx (bool). GetRawChangeAddress has an unused parameter.

## QK-0640 — version ignores vcs.modified and only truncates git revisions

Spec: [22](specs/22-aux-packages-tools.md). Source: version/version.go:40-67; README.md:100

BuildMetadata comes from Go build info (first 9 chars of vcs.revision for git); dirty trees report the clean revision; 'go install pkg@version' builds have no build metadata. README still tells users to install the latest release from the v4 module path.

## QK-0641 — Tooling/documentation drift on Go versions

Spec: [22](specs/22-aux-packages-tools.md). Source: flake.nix:21; go.mod:3; README.md:77; debug.go:13-15; rpc/rpc_example/main.go:19

flake.nix provides go_1_25 while go.mod requires go 1.26.0 and README/CI say 1.26 or 1.27; debug.go pins GODEBUG defaults to go1.25 under a go1.25 build tag; rpc_example targets port 9113 which is not a dcrwallet default.

## QK-0642 — assets.Path, errors.WithStack/Stacks and lru.Map are dead code

Spec: [22](specs/22-aux-packages-tools.md). Source: assets/assets.go:10-25; errors/errors.go:241-245,390-400; lru/map.go

None has a caller in the module at the pin; assets.Path only works on the build host without -trimpath; WithStack stores Go-runtime-specific debug.Stack() bytes.

## QK-0643 — version.String comments say invalid strings are not appended; code strips invalid characters and appends the rest

Spec: [22](specs/22-aux-packages-tools.md). Source: version/version.go:75-91,100-113

PreRelease 'rc_1' renders as 2.2.0-rc1; the part is omitted only when nothing survives normalization. Added as Q37.

## QK-0644 — Help text of the flag-based tools depends on file system state

Spec: [22](specs/22-aux-packages-tools.md). Source: cmd/sweepaccount/main.go:113-118; go-flags@v1.5.0/option.go:507-540

The --connect and --cafile defaults are cleared before parsing when the default certificate is missing, and go-flags shows a default only for non-zero struct values at parse time. Added as Q39.

## QK-0645 — sweepaccount accepts --feerate NaN at start-up

Spec: [22](specs/22-aux-packages-tools.md). Source: cmd/sweepaccount/main.go:152-157,376-379

Both range checks are float comparisons that NaN passes; the failure ('invalid fee rate') appears only after the passphrase prompt.

## QK-0646 — Typed client LockUnspent never sends null

Spec: [22](specs/22-aux-packages-tools.md). Source: rpc/client/dcrwallet/methods.go:129-156

The outputs slice is allocated with make, so nil ops becomes [] and LockUnspent(true, nil) is the documented unlock-all request.

## QK-0647 — Server behaviours asserted only in typed-client doc comments

Spec: [22](specs/22-aux-packages-tools.md). Source: rpc/client/dcrwallet/methods.go:129-145,557-590,899-901

Several JSON-RPC semantics (lockunspent persistence, walletpassphrase timeout 0, lockaccount error code, accountunlocked field presence) are stated in comments and need confirmation by the JSON-RPC spec. Added as Q38.

## QK-0648 — CI never runs TestManager

Spec: [23](specs/23-upstream-tests-oracle-plan.md). Source: run_tests.sh:9; wallet/udb/addressmanager_test.go:1212-1214

run_tests.sh passes -short and TestManager skips under it, so the API checks on a reopened non-watching address manager run nowhere in upstream CI.

## QK-0649 — 1,163 test lines are dead code

Spec: [23](specs/23-upstream-tests-oracle-plan.md). Source: wallet/udb/txquery_test.go:6; wallet/udb/txexample_test.go:6; wallet/walletdb/example_test.go:7

Three test files are excluded by build tags and written against APIs that no longer exist (s.InsertTx, testStore(), db.Namespace). They are useful only as a list of query cases to regenerate.

## QK-0650 — TestCoinbases never checks the spendable balance

Spec: [23](specs/23-upstream-tests-oracle-plan.md). Source: wallet/udb/tx_test.go:416-424

The second assertion repeats the immature-coinbase condition instead of comparing bal.Spendable, so every spendable value in the test's tables is unverified upstream. The port should assert both and take spendable values from the oracle.

## QK-0651 — v24.db.gz fixture is unused

Spec: [23](specs/23-upstream-tests-oracle-plan.md). Source: wallet/udb/upgrades_test.go:24-38

No entry of dbUpgradeTests names the version-24 fixture; upgrade steps 12->13 through 25->26 are observed only as not failing on the older fixtures.

## QK-0652 — Fixtures cross networks and are not reproducible

Spec: [23](specs/23-upstream-tests-oracle-plan.md). Source: wallet/udb/testdata/v1.db.go:32,53; wallet/udb/upgrades_test.go:75

Six fixtures were created with testnet2 parameters from a random seed and are upgraded by the test with testnet3 parameters. All contain random salts and nonces.

## QK-0653 — Generator v7.go does not compile as written

Spec: [23](specs/23-upstream-tests-oracle-plan.md). Source: wallet/udb/testdata/v7.go:11-31,133,154,160,178

It calls errors.Errorf at four places without importing any errors package; all generators import long-gone module paths and are documentation only.

## QK-0654 — txauthor test outputs carry 36-byte scripts

Spec: [23](specs/23-upstream-tests-oracle-plan.md). Source: wallet/txauthor/author_test.go:34

p2pkhOutputs allocates txsizes.P2PKHOutputSize (36) bytes as the script, not 25. Expectations are self-consistent because they are computed from the same outputs, but a port that substitutes real 25-byte P2PKH scripts gets different fees.

## QK-0655 — A failed Unlock locks an unlocked wallet

Spec: [23](specs/23-upstream-tests-oracle-plan.md). Source: wallet/locking_test.go:61-85

Passing a wrong passphrase to Unlock while the wallet is unlocked returns kind Passphrase and leaves the wallet locked.

## QK-0656 — Ticket maturity is one block later than coinbase maturity

Spec: [23](specs/23-upstream-tests-oracle-plan.md). Source: wallet/wallet.go:4422-4447; wallet/wallet_test.go:58

coinbaseMatured uses tip-tx+1 > maturity while ticketMatured and ticketExpired use tip-tx > maturity, documented as mirroring a dcrd off-by-one.

## QK-0657 — Known address-buffer off-by-one after coin-type upgrade is avoided, not tested

Spec: [23](specs/23-upstream-tests-oracle-plan.md). Source: wallet/discovery_test.go:20-33

TestDiscoveryCursorPos sets DisableCoinTypeUpgrades because the buffers record off-by-ones after the upgrade; the upgrade call is commented out. Behaviour after a SLIP0044 upgrade is untested and admitted to be off.

## QK-0658 — snacl tests depend on source order

Spec: [23](specs/23-upstream-tests-oracle-plan.md). Source: wallet/internal/snacl/snacl_test.go:14-20

Ten tests share package-level variables set by earlier tests; no fixed ciphertext or marshalled-parameter vector exists because salts and nonces are random.

## QK-0659 — Cursor deletion during iteration is untested

Spec: [23](specs/23-upstream-tests-oracle-plan.md). Source: wallet/walletdb/interface_test.go:99-101,202-205

testCursorDelete exists in the walletdb conformance suite but is skipped with a reference to a bolt defect.

## QK-0660 — Seed generation bypasses the wallet's randomness package

Spec: [23](specs/23-upstream-tests-oracle-plan.md). Source: dcrd/hdkeychain/v3@v3.1.3/extendedkey.go:15,630; wallet/wallet.go:5298; walletsetup.go:278; internal/prompt/prompt.go:271; walletseed/seed.go:29

hdkeychain.GenerateSeed reads the standard crypto/rand directly, so a seeded replacement of dcrd/crypto/rand does not make seed generation deterministic; four callers. Deterministic scenarios must always supply the seed.

## QK-0661 — Upstream random stream XORs into the caller's buffer

Spec: [23](specs/23-upstream-tests-oracle-plan.md). Source: dcrd/crypto/rand@v1.0.1/prng.go:85-110

crypto/rand Read does XORKeyStream(s, s); a replacement stream that overwrote the buffer would differ for any caller passing a non-zero buffer.

## QK-0662 — chaingen output differs on every run

Spec: [23](specs/23-upstream-tests-oracle-plan.md). Source: dcrd/blockchain/v5@v5.1.1/chaingen/generator.go:564,3167-3170

Block one takes its timestamp from time.Now() and coinbases carry a random extra nonce, so TestReorg and TestSetBirthStateAndScan assert relations only. Chain fixtures must be generated once and frozen.

## QK-0663 — Re-importing the same account key is allowed until an address is indexed

Spec: [23](specs/23-upstream-tests-oracle-plan.md). Source: wallet/udb/addressmanager_test.go:627-647,673-678

TestImportAccount imports the same tprv twice under different names successfully; Exist is returned only after SyncAccountToAddrIndex on the first copy.

## QK-0664 — Watching-only expectations in test helpers are never reached

Spec: [23](specs/23-upstream-tests-oracle-plan.md). Source: wallet/udb/addressmanager_test.go:768-781,818-828,1100-1107

testNewAccount returns early unless create is set and no caller sets create with watchingOnly; testChangePassphrase's watching-only branches have no caller. RenameAccount is the write that is shown to succeed on a watching-only manager.

## QK-0665 — Upgrade test reads version-5 account rows from a version-26 database

Spec: [23](specs/23-upstream-tests-oracle-plan.md). Source: wallet/udb/upgrades_test.go:215-218

verifyV5Upgrade calls fetchDBAccount(ns, acct, 5) after the full upgrade and expects last-used/last-returned fields there.

## QK-0666 — Release tag families disagree on DB version

Spec: [23](specs/23-upstream-tests-oracle-plan.md). Source: local tags of the dcrwallet repository, wallet/udb/upgrades.go at each tag

Tag v1.7.0 reports DB version 22 while release-v1.7.0 reports 25; only the release- family was used for the historical version table.

## QK-0667 — dcrdtest harness defaults to dcrd v1.8.0

Spec: [23](specs/23-upstream-tests-oracle-plan.md). Source: dcrtest/dcrdtest@v1.0.1-0.20240404170936-a2529e936df1/go.mod:8; chain/sync.go:33

Used without SetPathToDCRD it builds and starts a node far older than the JSON-RPC API 8.3.0 the wallet at the pin requires.

## QK-0668 — Tests write into the package directory and mutate shared config

Spec: [23](specs/23-upstream-tests-oracle-plan.md). Source: wallet/udb/common_test.go:70-97; wallet/setup_test.go:47

bdb, walletdb and udb tests create database files by relative name; testWallet stores the opened DB handle in the package-level basicWalletConfig, leaving a closed handle in the global between tests.

## QK-0669 — chaingen proof-of-work race

Spec: [23](specs/23-upstream-tests-oracle-plan.md). Source: dcrd/blockchain/v5@v5.1.1/chaingen/generator.go:2068-2085

The nonce range is split across runtime.NumCPU() goroutines and the first reported solution is used, so block hashes depend on core count and scheduling even with a seeded random stream and fixed clock.

## QK-0670 — Malformed P2SH script in reorg test helper

Spec: [23](specs/23-upstream-tests-oracle-plan.md). Source: wallet/reorg_test.go:43-58

chaingenPrevScripter returns OP_HASH160, 20 hash bytes, OP_EQUAL (22 bytes) with no 0x14 push opcode, although described as the chaingen OP_TRUE p2sh script.

## QK-0671 — Zero received time in store tests and fixtures

Spec: [23](specs/23-upstream-tests-oracle-plan.md). Source: wallet/udb/tx_test.go:37; wallet/udb/txdb.go:524; wallet/udb/testdata/v7.go:36,136

NewTxRecordFromMsgTx is called with time.Time{} in udb tests and with a zero epoch in the v5, v7, v11 generators; the current writer stores uint64(Received.Unix()) = 0xFFFFFFF1886E0900.

## QK-0672 — Synthetic block hashes in v5 and v7 fixtures

Spec: [23](specs/23-upstream-tests-oracle-plan.md). Source: wallet/udb/testdata/v5.db.go:127-135; wallet/udb/testdata/v7.go:111

Headers are inserted under chainhash.Hash{31: byte(height)} rather than their real hash, and PrevBlock chains through those invented hashes.

## QK-0673 — Create on an existing file opens it

Spec: [23](specs/23-upstream-tests-oracle-plan.md). Source: wallet/internal/bdb/db.go:316-323; wallet/setup_test.go:29-34

bdb openDB checks existence only for Open; Create calls bolt.Open on whatever is there, and all test helpers pre-create an empty temp file before calling walletdb.Create.

## QK-0674 — Conformance suite copies have drifted

Spec: [23](specs/23-upstream-tests-oracle-plan.md). Source: wallet/walletdb/interface_test.go:59,167,198; wallet/internal/bdb/interface_test.go:14

Only the walletdb copy re-puts keys before nested-bucket checks and compares with reflect.DeepEqual (so an absent key must read as nil); the bdb copy uses bytes.Equal and carries a 'Test must be updated for API changes' note.

## QK-0675 — Mis-named 'ok watching only' import cases

Spec: [23](specs/23-upstream-tests-oracle-plan.md). Source: wallet/udb/addressmanager_test.go:495-499,532,662

The watchingOnly flag in TestImportVotingAccount/TestImportAccount only reaches the test context; the manager is an ordinary one and is even unlocked in TestImportAccount.

## QK-0676 — Client certificate issued to the authority's own key

Spec: [23](specs/23-upstream-tests-oracle-plan.md). Source: x509_test.go:67-77,134-144

TestClientCert and TestUntrustedClientCert pass the same public key to generateAuthority and createSignedClientCert; a client key distinct from the CA key is never tested.

## QK-0677 — Usage test never checks its handler set

Spec: [23](specs/23-upstream-tests-oracle-plan.md). Source: internal/rpc/jsonrpc/rpchelp_test.go:65-67,91-110

TestRPCMethodUsageGeneration builds serverMethods() and deletes entries but never reports leftovers; both help tests stop comparing after the first mismatch.

## QK-0678 — Two go.mod requirements are not in the binary

Spec: [24](specs/24-reuse-dcroxide.md). Source: rpc/rpc_example/main.go:15; wallet/reorg_test.go:12

rpcclient/v8 is imported only by a standalone example program and blockchain/v5 only by one test (chaingen). Neither needs a Rust counterpart; the reorg test chains should be regenerated by a Go tool and stored as vectors.

## QK-0679 — dcroxide's mainnet seeder list has one more host than chaincfg v3.3.0

Spec: [24](specs/24-reuse-dcroxide.md). Source: dcroxide:crates/dcroxide-chaincfg/src/mainnet.rs:286-291; p2p/peering.go:348

dcroxide-chaincfg carries four mainnet seeders (one added by a later dcrd commit); dcrwallet's pin has three. A wallet built on dcroxide-chaincfg queries one more seeder than dcrwallet at the pin, a network-visible difference to reproduce or accept.

## QK-0680 — WIF checksum is a single BLAKE-256

Spec: [24](specs/24-reuse-dcroxide.md). Source: dcrd/dcrutil/v4@v4.0.3/wif.go:93-94,129,178

Addresses and extended keys use a double BLAKE-256 checksum; WIF uses chainhash.HashB once. dcroxide_base58::check_encode therefore cannot produce a WIF; the 39 bytes must be built by hand and passed to plain encode. The Go doc comment wrongly says 'double SHA256'.

## QK-0681 — Extended key kind comes from the key data byte, not the version

Spec: [24](specs/24-reuse-dcroxide.md). Source: dcrd/hdkeychain/v3@v3.1.3/extendedkey.go:574-612; dcr-rs:src/hd.rs:480-485

NewKeyFromString treats a key as private iff keyData[0]==0x00 whatever the version bytes say, and re-serializes with the version matching the data. dcr-rs rejects such strings instead.

## QK-0682 — secp256k1.PrivKeyFromBytes never fails; dcroxide rejects

Spec: [24](specs/24-reuse-dcroxide.md). Source: dcrd/dcrec/secp256k1/v4@v4.4.1/privkey.go:27-46; dcroxide:crates/dcroxide-dcrec/src/secp256k1.rs:353; dcroxide:crates/dcroxide-txscript/src/sign.rs:92-99

Go reduces any input modulo the group order and accepts zero, so DecodeWIF and importprivkey accept out-of-range keys. dcroxide's PrivateKey::from_bytes returns None and sign returns 'invalid privkey' for zero or >= N. Bug-for-bug parity needs a reducing constructor added to dcroxide-dcrec.

## QK-0683 — Stripped child keys change hardened children after a string round trip

Spec: [24](specs/24-reuse-dcroxide.md). Source: dcrd/hdkeychain/v3@v3.1.3/extendedkey.go:248,309-311,595-603

Child stores private keys with leading zeros removed (about 1 in 256 keys) and uses the short form left-aligned in hardened derivation; NewKeyFromString always restores 32 bytes. Serializing and reparsing such a key yields different hardened children. The Rust type must keep the stored-length form.

## QK-0684 — SerializedPrivKey can return fewer than 32 bytes

Spec: [24](specs/24-reuse-dcroxide.md). Source: dcrd/hdkeychain/v3@v3.1.3/extendedkey.go:422-428

It returns the stored (possibly stripped) key slice, 1 to 32 bytes; callers pad or pass it to PrivKeyFromBytes, which accepts short input.

## QK-0685 — Zero private child sum is not rejected

Spec: [24](specs/24-reuse-dcroxide.md). Source: dcrd/hdkeychain/v3@v3.1.3/extendedkey.go:300-312,442-444

Child checks Il but not the sum Il+parent; a zero sum becomes an empty key that String renders as 'zeroed extended key'. Probability about 2^-256; dcr-rs rejects it.

## QK-0686 — Neuter shares buffers with the private key; Zero corrupts the neutered key

Spec: [24](specs/24-reuse-dcroxide.md). Source: dcrd/hdkeychain/v3@v3.1.3/extendedkey.go:157-171,396-408,488-497

Neuter's result aliases the private key's memoized public key, chain code and parent fingerprint slices; Zero on the private key overwrites them in place. Whether any dcrwallet call site depends on this was not traced; a Rust port with owned buffers differs exactly there.

## QK-0687 — DecodeWIF with an unknown scheme byte succeeds

Spec: [24](specs/24-reuse-dcroxide.md). Source: dcrd/dcrutil/v4@v4.0.3/wif.go:138-161

The switch has no default, so a scheme byte other than 0, 1 or 2 returns a WIF with nil keys and scheme 0 (ECDSA) and no error; the caller fails later with a call-site-specific message.

## QK-0688 — dcroxide MsgTx hashing and serialization are infallible

Spec: [24](specs/24-reuse-dcroxide.md). Source: dcroxide:docs/PARITY.md:1019

dcrd's Bytes returns ErrNegativeTxTree and TxHash panics for a negative outpoint tree; dcroxide writes the byte. Wallet sites that build inputs from RPC parameters must validate the tree first where Go errors or crashes.

## QK-0689 — dcroxide's mixpool is deliberately stricter than dcrd's in three places

Spec: [24](specs/24-reuse-dcroxide.md). Source: dcroxide:crates/dcroxide-mixing/src/mixpool.rs:9-15,225

A cap of 64 key exchanges per identity (MAX_KES_PER_IDENTITY), the one-message-per-type rule applied to un-orphaned messages, and an expiry latch that keeps the newest height. Chosen for a relaying node; a wallet pool refusing what dcrwallet accepts changes which peers a mix sees.

## QK-0690 — The wallet's mixpool validates no UTXOs

Spec: [24](specs/24-reuse-dcroxide.md). Source: wallet/mixing.go:38-49; dcrd/mixing@v0.7.5/mixpool/mixpool.go:294-296

mixpool.NewPool enables UTXO validation only if its BlockChain argument also implements UtxoFetcher; dcrwallet's mixpoolBlockchain has only CurrentTip and ChainParams, so pair requests are accepted without input checks. The Rust wallet must pass None for the fetcher.

## QK-0691 — Invalid base58 surfaces as a length error

Spec: [24](specs/24-reuse-dcroxide.md). Source: dcrd/hdkeychain/v3@v3.1.3/extendedkey.go:559-562; dcrd/dcrutil/v4@v4.0.3/wif.go:120-124

base58.Decode returns an empty slice on any invalid character, so a mistyped extended key reports ErrInvalidKeyLen and a mistyped WIF 'malformed private key', never a character error.

## QK-0692 — ErrWrongWIFNetwork names the expected network

Spec: [24](specs/24-reuse-dcroxide.md). Source: dcrd/dcrutil/v4@v4.0.3/wif.go:34-40,134-136

The error value carries the network the caller asked for, not the one found in the string; text is 'WIF is not for the network identified by ' + %#04x of the [2]byte (derived as 0x22de on mainnet, not executed).

## QK-0693 — hdkeychain.GenerateSeed uses Go's standard crypto/rand and a uint8 length

Spec: [24](specs/24-reuse-dcroxide.md). Source: dcrd/hdkeychain/v3@v3.1.3/extendedkey.go:14-16,623-632

Seeds come from the standard library generator, not dcrd's crypto/rand package, and the length parameter is uint8 so wider caller values truncate before the 16..64 check.

## QK-0694 — dcroxide ecdsa::Error displays kind names, not Go's sentences

Spec: [24](specs/24-reuse-dcroxide.md). Source: dcroxide:crates/dcroxide-dcrec/src/secp256k1/ecdsa.rs:111-115

RecoverCompact and DER parse errors print names such as ErrSigTooShort; wherever dcrwallet returns such an error to a client the Go sentence must be supplied. SignError strings in dcroxide_txscript::sign were not compared with Go's.

## QK-0695 — dcroxide ports are synchronous where Go's types lock themselves

Spec: [24](specs/24-reuse-dcroxide.md). Source: dcroxide:crates/dcroxide-standalone/src/subsidy.rs:91-97; dcrd/addrmgr/v3@v3.0.0/addrmanager.go:401-420,609-640; dcroxide:crates/dcroxide-mixing/src/mixpool.rs:14-18

SubsidyCache, AddrManager, DynamicBanScore and mixpool Pool take &mut self with injected clocks; Go's are safe for concurrent use with internal goroutines and timers (addrmgr saves peers.json every 10 minutes and once at Stop; mixpool Receive blocks). The wallet must add the mutexes, timers and the wait loop.

## QK-0696 — Ed25519 WIF yields a key the signer cannot use

Spec: [24](specs/24-reuse-dcroxide.md). Source: dcrd/dcrutil/v4@v4.0.3/wif.go:146-153; dcroxide:crates/dcroxide-txscript/src/sign.rs:101-114; internal/rpc/jsonrpc/methods.go:5049-5054

DecodeWIF returns a 32-byte scalar for scheme 1, while the sign package (Go and dcroxide alike) requires the 64-byte seed||pubkey form, so signrawtransaction with an Ed25519 WIF fails at signing. That is dcrwallet's behaviour and its error text must be matched.

## QK-0697 — dcroxide's SOCKS port drops two go-socks behaviours dcrwallet uses

Spec: [24](specs/24-reuse-dcroxide.md). Source: dcroxide:crates/dcroxide-node/src/socks.rs:11-14; config.go:693-705; p2p/peering.go:208-228

No connection pool with a maximum (socks.NewPool behind --circuitlimit) and no ProxiedAddr: the port returns the raw stream and discards the address, while dcrwallet reads RemoteAddr() as the dialed host/port for the version message.

## QK-0698 — mixclient.UseLogger does not exist

Spec: [24](specs/24-reuse-dcroxide.md). Source: log.go:42; wallet/wallet.go:5540

dcrwallet keeps a commented-out XXX call; the mix client's logger is set per instance with SetLogger, but the subsystem tag still exists so --debuglevel accepts it.

## QK-0699 — dcroxide has two Tor resolvers

Spec: [24](specs/24-reuse-dcroxide.md). Source: dcroxide:docs/PARITY.md:1123; dcroxide:crates/dcroxide-addrmgr/src/tordns.rs:58

dcroxide_addrmgr::tor_lookup_ip reproduces dcrd's TorLookupIP exactly (single unchecked read) and is only vector-tested; the node runs a different, stricter one in the node crate. dcrwallet calls connmgr/v3's TorLookupIP.

## QK-0700 — dcroxide-certgen has P-384 in gentool but not in certgen::Curve

Spec: [24](specs/24-reuse-dcroxide.md). Source: dcroxide:crates/dcroxide-certgen/src/certgen.rs:19-26; dcroxide:crates/dcroxide-certgen/src/gentool.rs:59-70; internal/cfgutil/curve.go:21-30

certgen::Curve has P256 and P521 only (dcrd's two curves); the gentool module in the same crate generates, signs with and PKCS#8-marshals P-384. dcrwallet accepts P-384 for --tlscurve.

## QK-0701 — dcrutil.NewAmount's error is not a sentinel

Spec: [24](specs/24-reuse-dcroxide.md). Source: dcrd/dcrutil/v4@v4.0.3/amount.go:85

Each call builds a fresh errors.New("invalid coin amount"); it can only be matched by text.

## QK-0702 — dcroxide Params constructors allocate per call and reshape Go types

Spec: [24](specs/24-reuse-dcroxide.md). Source: dcroxide:crates/dcroxide-chaincfg/src/lib.rs:138-229

mainnet_params() etc. build a fresh owned Params each call (including the 3,146-entry block-one ledger); Deployments is a sorted Vec of pairs, PowLimit a Uint256, durations are i64 seconds, Vote::vote_index returns Option. Build once and share.

## QK-0703 — ecdsa.Sign accepts any hash length

Spec: [24](specs/24-reuse-dcroxide.md). Source: dcrd/dcrec/secp256k1/v4@v4.4.1/nonce.go:135-141; ecdsa/signature.go:641; modnscalar.go:360-368; internal/rpc/rpcserver/server.go:3763; wallet/wallet.go:1998-2000

NonceRFC6979 truncates the hash to its first 32 bytes and left-pads shorter input; ModNScalar.SetByteSlice does the same for the signed scalar. dcrwallet exposes this through gRPC SignHashes with no length check. dcroxide's ecdsa::sign requires exactly 32 bytes.

## QK-0704 — Three websocket replies carry an empty jsonrpc version

Spec: [24](specs/24-reuse-dcroxide.md). Source: internal/rpc/jsonrpc/methods.go:284-306; internal/rpc/jsonrpc/server.go:378-380, :408-410, :429-431; dcroxide:crates/dcroxide-dcrjson/src/jsonrpc.rs:277-287

makeResponse builds a dcrjson.Response without setting Jsonrpc and the websocket loop marshals it with json.Marshal directly (invalid request to an authenticated client, authenticate acknowledgement, stop reply), producing "jsonrpc":"". dcroxide's marshal_response normalizes the version to 1.0 or 2.0 and cannot emit it.

## QK-0705 — mixing.F type differs

Spec: [24](specs/24-reuse-dcroxide.md). Source: testsolver.go:29; dcroxide:crates/dcroxide-mixing/src/field.rs:17

Go's mixing.F is a *big.Int that dcrwallet passes to solverrpc.Roots; dcroxide's F is a u128 constant.

## QK-0706 — Go transactions alias inputs and outputs

Spec: [24](specs/24-reuse-dcroxide.md). Source: wire@v1.8.0/msgtx.go; dcroxide:crates/dcroxide-wire/src/msgtx.rs:160-296

MsgTx.TxIn and TxOut are slices of pointers, so elements can be shared between transactions or retained by callers; dcroxide-wire owns the values. MsgTx.Copy is not called by dcrwallet's non-test code. Whether any call site relies on the aliasing is untraced.

## QK-0707 — Static strings in dcroxide vote types

Spec: [24](specs/24-reuse-dcroxide.md). Source: dcroxide:crates/dcroxide-chaincfg/src/lib.rs:59-88

Vote.id/description, Choice.id/description and ConsensusDeployment.forced_choice_id are &'static str in dcroxide-chaincfg where Go has string.

## QK-0708 — Stripping is slice length, and a dprv round trip undoes it

Spec: [25](specs/25-reuse-dcr-rs-and-conventions.md). Source: dcrd/hdkeychain/v3@v3.1.3/extendedkey.go:248, :309-311, :464-465, :599

dcrd has no stripped flag; a Child-derived private key that began with zero bytes is simply a shorter slice, which changes the hardened HMAC preimage layout. String() pads to 32 and NewKeyFromString stores 32, so serialize-then-parse changes the hardened children of roughly 1 key in 256. The Rust key type must carry a stored-length/stripped state set by child() and cleared by parse and by new_master.

## QK-0709 — ChildBIP32Std from a stripped parent is not strict BIP32

Spec: [25](specs/25-reuse-dcr-rs-and-conventions.md). Source: dcrd/hdkeychain/v3@v3.1.3/extendedkey.go:248, :309

The strictBIP32 flag controls only whether the produced child is stored stripped; the preimage layout comes from how the parent is already stored. Not reachable from dcrwallet (it never calls ChildBIP32Std) but must be reproduced if the API is ported.

## QK-0710 — Zero private child sum and infinity public child sum are not rejected

Spec: [25](specs/25-reuse-dcr-rs-and-conventions.md). Source: dcrd/hdkeychain/v3@v3.1.3/extendedkey.go:277-311, :337-341; dcr-rs:src/hd.rs:116-121

dcrd checks Il for overflow/zero but not Il+parent. A zero private sum is stripped to an empty key that prints 'zeroed extended key' while IsPrivate() is true; a public sum at infinity is serialized. dcr-rs returns an error in both cases. Probability about 2^-256; needs a recorded reproduce-or-diverge decision.

## QK-0711 — Key type on parse comes from the data byte, not the version

Spec: [25](specs/25-reuse-dcr-rs-and-conventions.md). Source: dcrd/hdkeychain/v3@v3.1.3/extendedkey.go:593-614; dcr-rs:src/hd.rs:506-518

NewKeyFromString sets isPrivate = keyData[0]==0x00 and returns a key carrying the network's own versions, so a dpub-versioned string with private data parses as a private key and re-serializes as dprv, and the reverse. dcr-rs rejects both; the wallet port must accept them as dcrd does.

## QK-0712 — Depth is uint16 in memory, one byte on the wire

Spec: [25](specs/25-reuse-dcr-rs-and-conventions.md). Source: dcrd/hdkeychain/v3@v3.1.3/extendedkey.go:125, :347-348, :447, :587; dcr-rs:src/hd.rs:302

Depth() can exceed 255 for a key derived in memory while String() writes depth % 256; depth wraps at 65,536. dcr-rs uses u8 and errors on the 256th level.

## QK-0713 — Neuter aliases the private key's buffers; Zero propagates

Spec: [25](specs/25-reuse-dcr-rs-and-conventions.md). Source: dcrd/hdkeychain/v3@v3.1.3/extendedkey.go:396-407, :488-497; wallet/discovery.go:436-443; wallet/udb/addressmanager.go:2136-2140

The neutered key's key is the private key's memoized pubKey slice and it shares chainCode and parentFP slices. Zero() on the private key leaves the neutered key with all-zero key, chain code and fingerprint; Zero() on the neutered key clears the private key's chain code and cached pubkey. Neuter on a public key returns the same pointer. dcrwallet does Neuter then Zero on the private key in discovery and account creation; at the sites traced no output depends on it, but the remaining sites need confirming since Rust value semantics cannot alias.

Disposition: not reproduced. `purse-hdkeychain` keys own their bytes; a neutered key or a clone survives `zero()` on the original. The remaining call sites were traced (walletsetup, discovery, address manager): each uses or serializes the public key before zeroing, so no output depends on the aliasing.

## QK-0714 — SerializedPrivKey returns the stored (possibly short) slice

Spec: [25](specs/25-reuse-dcr-rs-and-conventions.md). Source: dcrd/hdkeychain/v3@v3.1.3/extendedkey.go:422-428; wallet/udb/addressmanager.go:2498-2503

For a stripped key the returned slice is shorter than 32 bytes and is the key's own storage; dcrwallet's one caller passes it to PrivKeyFromBytes (which left-pads and reduces) and then zeroes it, zeroing the extended key itself.

Disposition: the stored-length slice is reproduced and compared with Go, but it is lent read-only. `priv_key_bytes` returns the left-padded 32 bytes that `PrivKeyFromBytes` makes of it, compared with Go on the leading-zero corpus.

## QK-0715 — Invalid base58 reports as a length error

Spec: [25](specs/25-reuse-dcr-rs-and-conventions.md). Source: dcrd/hdkeychain/v3@v3.1.3/extendedkey.go:559-562; dcroxide:crates/dcroxide-base58/src/lib.rs:38

decred/base58 Decode returns an empty slice for any invalid character, so NewKeyFromString reports ErrInvalidKeyLen, not a base58 error. dcroxide-base58 reproduces the empty-slice behaviour; dcr-rs (bs58) returns a distinct Base58 error.

## QK-0716 — DecodeUserInput edge cases

Spec: [25](specs/25-reuse-dcr-rs-and-conventions.md). Source: walletseed/seed.go:73-100

Any single token is treated as hex (a one-word mnemonic attempt yields a hex error); an input of only spaces reaches the length check and reports the seed-length error; exactly one decoded mnemonic byte skips the checksum test and reports the seed-length error; only the space character splits words, so tab or newline separated words fail as unknown words.

## QK-0717 — Mnemonic position counts accepted words

Spec: [25](specs/25-reuse-dcr-rs-and-conventions.md). Source: pgpwordlist/pgpwordlist.go:40-57

DecodeMnemonics reports 'position <idx>' as the count of words accepted so far, not the index in the input slice, and echoes the trimmed word as typed (not lower-cased). Lookup is case-insensitive via strings.ToLower.

Disposition: reproduced. Confirmed against the oracle.

## QK-0718 — GenerateRandomSeed's size >= 255 guard and the gRPC bypass

Spec: [25](specs/25-reuse-dcr-rs-and-conventions.md). Source: walletseed/seed.go:23-28; internal/rpc/rpcserver/server.go:3177-3196

walletseed.GenerateRandomSeed rejects size >= 255 before the 16..64 range check (uint8 narrowing guard, redundant with the range check). The gRPC GenerateRandomSeed handler does not call it and validates length itself with its own message.

## QK-0719 — dcr-rs calls its seed a BIP39 seed and binds key type to prefix

Spec: [25](specs/25-reuse-dcr-rs-and-conventions.md). Source: dcr-rs:src/hd.rs:200-201, :482-488, :728-757; dcr-rs:src/hashing.rs:45; dcr-rs:src/lib.rs:65-103

dcr-rs makes BIP39 a default feature, detects the network from the version among four hard-coded networks (no expected-network parameter, so no ErrWrongNetwork), gates string length at 128 not 113, and collapses dcrd's sentinels into Derivation/Parse/Base58/BadChecksum/UnknownPrefix. All deliberate for a hardware-signer audience; all parity gaps for the wallet.

## QK-0720 — dcroxide PrivateKey::from_bytes rejects where dcrd reduces

Spec: [25](specs/25-reuse-dcr-rs-and-conventions.md). Source: dcroxide:crates/dcroxide-dcrec/src/secp256k1.rs:335-359; wallet/udb/addressmanager.go:2498-2510

dcrd PrivKeyFromBytes accepts any length up to 32, left-pads and reduces mod N; dcroxide rejects zero and >= N and takes exactly 32 bytes, recorded there as unobservable for a node. For a wallet it is observable: stripped HD keys and imported private keys reach PrivKeyFromBytes.

## QK-0721 — dcroxide ADR bodies are stale by design

Spec: [25](specs/25-reuse-dcr-rs-and-conventions.md). Source: dcroxide:docs/adr/README.md:3-5, :13, :15; dcroxide:docs/adr/0005-concurrency-model.md:28-39, :53-60

ADRs are immutable; ADR-0005's Decision still says tokio and ADR-0007's still says serde, while the shipped decisions (OS threads, no serde) are in the status line and addenda. Read status and addenda first.

## QK-0722 — dcr-rs format_amount is not dcrutil.Amount.String

Spec: [25](specs/25-reuse-dcr-rs-and-conventions.md). Source: dcr-rs:src/amount.rs:11-24

It trims trailing zeros and omits the fraction for whole coins ('1 DCR', '1.2 DCR'); must not be used for any dcrwallet-compatible output.

## QK-0723 — EncodeMnemonic vs EncodeMnemonicSlice on an empty seed

Spec: [25](specs/25-reuse-dcr-rs-and-conventions.md). Source: walletseed/seed.go:44-67

EncodeMnemonic writes a space before the checksum word unconditionally, so for an empty seed it returns ' exceed' while the slice form returns ['exceed']. Unreachable through the daemon.

## QK-0724 — Zeroed ExtendedKey is a public key with an empty key

Spec: [25](specs/25-reuse-dcr-rs-and-conventions.md). Source: dcrd/hdkeychain/v3@v3.1.3/extendedkey.go:488-497

After Zero: IsPrivate false, String 'zeroed extended key', SerializedPrivKey ErrNotPrivExtKey, hardened Child ErrDeriveHardFromPublic, normal Child fails in secp256k1.ParsePubKey of the empty key. Versions are not cleared.

## QK-0725 — NewMaster doc names ErrUnusable

Spec: [25](specs/25-reuse-dcr-rs-and-conventions.md). Source: dcrd/hdkeychain/v3@v3.1.3/extendedkey.go:503-506

The comment refers to an ErrUnusable error; the identifier is ErrUnusableSeed.

## QK-0726 — Prompt and DecodeUserInput disagree on hex and splitting

Spec: [25](specs/25-reuse-dcr-rs-and-conventions.md). Source: internal/prompt/prompt.go:318-360; walletseed/seed.go:71-82

Prompt collapses all Unicode whitespace and pads odd-length hex; DecodeUserInput splits on U+0020 only and rejects odd-length hex, so tab/newline-separated mnemonics over gRPC fail.

## QK-0727 — dcr-rs Error::Derivation semantics oppose dcrwallet's

Spec: [25](specs/25-reuse-dcr-rs-and-conventions.md). Source: dcr-rs:src/lib.rs:66-69

dcr-rs documents its single derivation error as fatal/retry, folding in the invalid-child case that dcrwallet skips on.

## QK-0728 — purse-errors does not look inside a foreign error

Spec: [22](specs/22-aux-packages-tools.md). Source: go/src/errors/wrap.go (`is`, `as`)

Go's `errors.Is` and `errors.As` call `Unwrap`, `Is` and `As` on every node whatever its type, so a wallet `*Error` or a sentinel nested under a foreign wrapper is found. In `purse-errors` a foreign error is a leaf. Disposition: diverges by design. A wallet error or a matchable sentinel is never placed behind a foreign type's `source()`; wrapping is written with `e!` or `AnyError::wrap`, and a Go pointer sentinel is ported as one shared `AnyError`. No call site at the pin needs more.

## QK-0729 — Seed words reach the wallet log through decode errors

Spec: [22](specs/22-aux-packages-tools.md), [20](specs/20-grpc-other-services.md). Source: pgpwordlist/pgpwordlist.go:47-55; internal/rpc/rpcserver/server.go:3199-3204; rpcserver.go:443-445

`DecodeMnemonics` quotes the caller's word in its unknown-word and wrong-position errors; the latter always quotes a valid seed word with its position. gRPC `DecodeSeed` wraps that text and the unary interceptor logs every failed call at error level, so seed words are written to the log at the default level. Disposition (project owner, 2026-11-18): the reply is reproduced and the log is not. `purse-seed` keeps the text, so the client that sent the words gets dcrwallet's exact error. The daemon's log line for a failed `DecodeSeed` call must not carry it: the quoted word is redacted there, under the standing rule on leaked secrets. The redaction belongs to the gRPC server's logging, which does not exist yet; it needs a test when it does.

## QK-0730 — bbolt panics or reads outside the page on a malformed file

Spec: [01](specs/01-walletdb-bbolt.md). Source: bbolt@v1.3.12/page.go:56-64; cursor.go:304; bucket.go:727-749; freelist.go:284

On a damaged file bbolt panics (page id or type mismatch, a meta or freelist page inside a tree, a branch with no elements, an empty bucket value, an inline page without the leaf bit, a root bucket at page 0, a freelist count that overflows), reads adjacent mapped memory (element counts, offsets, keys or values outside the page), faults (pages beyond the end of the file) or recurses without bound (cycles). Disposition: `purse-boltread` returns an error in each case, with tests. It also refuses what bbolt accepts on such files: trees deeper than 64 levels, a page reached twice in a walk, keys out of order under a cursor. Scope since 2026-11-18 (ADR-0013): `purse-boltread` is a test utility and dcr-purse reads no `wallet.db`, so this disposition governs what tests read, not a surface of the wallet.

## QK-0731 — Prev landing on an empty leaf ends the walk

Spec: [01](specs/01-walletdb-bbolt.md). Source: bbolt@v1.3.12/cursor.go:248-277

`Cursor.Prev` returns nil when it steps onto an empty leaf even though earlier entries exist; `Next` skips empty leaves. bbolt never writes an empty non-root leaf, so this shows only on crafted files. Disposition: reproduced, compared op for op with Go. Scope since 2026-11-18 (ADR-0013): in `purse-boltread`, a test utility; dcr-purse reads no `wallet.db`.

## QK-0732 — Meta page 1's own page-size field is never used

Spec: [01](specs/01-walletdb-bbolt.md). Source: bbolt@v1.3.12/db.go:322-407

The page size comes from meta 0 or, failing that, from probing for a valid meta 1 at each candidate size; the size recorded inside meta 1 is not compared with where it was found. Disposition: reproduced. Rejecting an inconsistent meta 1, as spec 01 §4.3 suggests, would differ from Go. Scope since 2026-11-18 (ADR-0013): in `purse-boltread`, a test utility; dcr-purse reads no `wallet.db`.

## QK-0733 — The address manager's memory keeps what a rolled-back transaction changed

Spec: [02](specs/02-address-manager.md). Source: wallet/udb/addressmanager.go (`ChangePassphrase`, `SetAccountPassphrase`, `ConvertToWatchingOnly`); found by the oracle (`tools/oracle/cmd/oprunner`, corpus script `15-rollback-bookkeeping`)

The manager updates its in-memory state inside the database transaction and does not restore it when the transaction rolls back. After a rolled-back private passphrase change, `Unlock` rejects the old passphrase and accepts the new one although the store still holds keys sealed under the old; `AccountHasPassphrase` and `WatchingOnly` likewise keep the values the rolled-back transaction set. dcrwallet reaches this only when one of its own write transactions fails after the manager call. Disposition (project owner, 2026-11-19; ADR-0002): not reproduced. dcr-purse keeps memory consistent with the store (ADR-0012 Rule 2).

## QK-0734 — An outpoint both mined and unmined is reported and counted twice

Spec: [04](specs/04-txstore-behavior.md). Source: wallet/udb/txunmined.go:26-29; found by the store spike (ADR-0003, Decision item 5)

When a block is disapproved and its regular transactions are later revalidated while an unmined copy rolled back from another block also exists, one outpoint is held both as a mined and as an unmined credit. `UnspentOutputs` reports it twice and the balance counts it twice; the transaction stays unmined, and `InsertMemPoolTx` refuses it as already mined. Confirmed by a run. Disposition: open; to be decided in Phase 3 with an oracle run, reproduced under ADR-0012 Rule 1 or fixed under Rule 7 (ADR-0003 Open point 2). The schema represents both outcomes.

## QK-0735 — stakeInvalidate restores an unspent entry for a credit the disapproved block spends

Spec: [04](specs/04-txstore-behavior.md). Source: wallet/udb/txmined.go:857-878, 728; wallet/udb/txdb.go:942-953

`stakeInvalidate` restores an unspent entry for a credit it has just moved to the invalidated bucket when the disapproved block also spends that credit, and `stakeValidate`'s `spendCredit` then writes a zero-filled credit for it. Read from the source at the pin; not run. Disposition: open; Phase 3, as QK-0734.

## QK-0736 — A regular transaction of a block disapproved and never mined again keeps its mined record

Spec: [04](specs/04-txstore-behavior.md). Source: wallet/udb/txmined.go (`stakeInvalidate`); wallet/udb/txunmined.go:26-29

Such a transaction keeps its mined record, so `InsertMemPoolTx` refuses it as already mined and the wallet cannot track it in the mempool again. Read from the source at the pin; not run. Disposition: open; Phase 3, as QK-0734.
