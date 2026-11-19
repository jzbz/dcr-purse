# 25 — Reuse assessment: dcr-rs, and the dcroxide engineering conventions the wallet inherits

## 1. Header

**Scope.** Two bodies of existing work, assessed for what dcr-purse takes from each.

Part A — dcr-rs (Rust Decred primitives crate, version 0.6.3, commit `683d9a7`):

| File | Lines | Read | Contents |
| --- | ---: | --- | --- |
| `dcr-rs:src/lib.rs` | 140 | full | crate attributes, feature gates, `Error`, re-exports |
| `dcr-rs:src/blake256.rs` | 400 | full | BLAKE-256 (14 rounds), incremental hasher, KATs |
| `dcr-rs:src/hashing.rs` | 105 | full | `hash160`, base58check encode/decode |
| `dcr-rs:src/network.rs` | 124 | full | four networks, address and HD version bytes, SLIP-0044 coin types |
| `dcr-rs:src/address.rs` | 248 | full | P2PKH/P2SH v0 addresses and scripts |
| `dcr-rs:src/hd.rs` | 800 | full | BIP32 keys with dcrd's `Child`/`ChildBIP32Std` variants |
| `dcr-rs:src/tx.rs` | 458 | full | `MsgTx` prefix/witness/full serialization and parsing |
| `dcr-rs:src/sighash.rs` | 100 | full | SigHashAll signature hash |
| `dcr-rs:src/sign.rs` | 90 | full | P2PKH signature scripts |
| `dcr-rs:src/amount.rs` | 40 | full | atom formatting |
| `dcr-rs:src/airgap.rs` | 898 | full | CBOR sign-request format version 3, review and signer |
| `dcr-rs:tests/vectors.rs` | 933 | full | dcrd-derived BLAKE-256, BIP32, address, hardened-variant, BIP39 vectors |
| `dcr-rs:tests/onchain_sighash.rs` | 244 | full | mainnet transaction sighash, txid and parser strictness |
| `dcr-rs:tests/airgap_review.rs` | 1429 | test index and doc comments | airgap format and policy tests |
| `dcr-rs:Cargo.toml`, `Cargo.lock`, `README.md`, `CHANGELOG.md`, `SECURITY.md`, `.github/workflows/ci.yml` | 80, 350, 276, 158, 101, 119 | full (lock: package list) | manifest, dependency closure, policy |

Compared against, at the versions dcrwallet's `go.mod` pins:

| File | Lines | Read |
| --- | ---: | --- |
| `dcrd/hdkeychain/v3@v3.1.3/extendedkey.go` | 636 | full, line by line against `dcr-rs:src/hd.rs` |
| `dcrd/hdkeychain/v3@v3.1.3/doc.go`, `README.md` | 96, 62 | full |
| `dcrd/hdkeychain/v3@v3.1.3/extendedkey_test.go` | 1051 | test index, error table, vector 4, zero-high-byte case |
| `walletseed/seed.go`, `walletseed/seed_test.go` | 102, 88 | full |
| `pgpwordlist/pgpwordlist.go`, `pgpwordlist/wordlist.go`, `pgpwordlist/pgpwordlist_test.go` | 60, 544, 66 | full (word list: structure, first and last entries, count) |
| `dcrd/hdkeychain/v3@v3.1.3/bench_test.go`, `example_test.go`, `go.mod` | 84, 195, 27 | full (§11.1) |
| `pgpwordlist/doc.go` | 5 | full (package comment only) |
| `errors/errors.go` (`E`, `(*Error).Error`, `Separator`, kind strings), `dcrwallet.go:38`, `internal/prompt/prompt.go:262-372`, `internal/rpc/rpcserver/server.go:110-126`, `:3177-3205` | — | the lines §3.10, §5.4 and §8.2 cite |
| `rpcserver.go` (gRPC and TLS set-up, `:248-460`), `rpc/api.proto` (1341 lines; feature survey) | — | the parts §4.9 cites |
| `grpc@v1.83.2` (`server.go`, `rpc_util.go`, `internal/transport/{defaults,http2_server,http_util}.go`, `credentials/tls.go`, `encoding/proto/proto.go`), `protobuf@v1.36.11` (`internal/impl/{codec_message,encode}.go`) | — | the lines §4.9 cites |

Part B — dcroxide (commit `8053abe`, 2026-11-14):

| File | Lines | Read |
| --- | ---: | --- |
| `dcroxide:docs/adr/README.md`, `template.md` | 102, 16 | full |
| `dcroxide:docs/adr/0001-oracle-driven-differential-testing.md` | 612 | body and addenda through the reference-Go addendum in full; later addenda by heading and index summary |
| `dcroxide:docs/adr/0002-vendor-blake256-from-dcr-rs.md` | 133 | full |
| `dcroxide:docs/adr/0003-slice-based-wire-decoding.md` | 79 | full |
| `dcroxide:docs/adr/0004-storage-backend.md` | 1854 | decision, headings and index summary (node storage; measurement addenda not read line by line) |
| `dcroxide:docs/adr/0005-concurrency-model.md` | 976 | body, 2026-07-26 and 2026-08-07 addenda in full; thread-by-thread addenda by heading and index summary |
| `dcroxide:docs/adr/0006-secp256k1-backend.md` | 641 | headings and index summary (superseded by ADR-0017) |
| `dcroxide:docs/adr/0007-json-emission-strategy.md` | 265 | full |
| `dcroxide:docs/adr/0008-clippy-lint-policy.md` | 384 | full |
| `dcroxide:docs/adr/0009-storage-shape.md`, `0010-height-first-block-keys.md`, `0011-exists-address-layout-3-and-the-flush-participant.md` | 1333, 195, 1407 | headings, index summaries, ADR-0009's guardrails |
| `dcroxide:docs/adr/0012-pure-rust.md` | 592 | full |
| `dcroxide:docs/adr/0013-upstream-tracking-release-tags.md` | 330 | context and decision in full |
| `dcroxide:docs/adr/0014-release-engineering.md` | 413 | full |
| `dcroxide:docs/adr/0015-tls-crypto-provider.md` | 1094 | context, T1, order of work, T7, T10, known limits in full; other sections and addenda by heading and index summary |
| `dcroxide:docs/adr/0016-secp256k1-vartime-arithmetic.md` | 885 | headings and index summary |
| `dcroxide:docs/adr/0017-secp256k1-in-pure-rust.md` | 346 | context, decision, soak and pin in full |
| `dcroxide:docs/dependency-ledger.md` | 172 | full (long table cells truncated at about 1,500 characters) |
| `dcroxide:SECURITY.md` | 523 | status, reporting, scope, first known gaps, closing section |
| `dcroxide:README.md` | 875 | layout and development sections |
| `dcroxide:Cargo.toml`, `clippy.toml`, `deny.toml`, `rust-toolchain.toml`, `_typos.toml` | 152, 37, 106, 19, 77 | full |
| `dcroxide:.github/workflows/ci.yml`, `constant-time.yml` | 987, 275 | job list; `lint`, `lint-latest`, `test`, `test-wrapping`, `dcrd-interop`, `go-tools`, `deny`, `docs`, `typos`, `no-std` bodies; other jobs by name and header comment |
| `dcroxide:docs/PARITY.md`, `docs/QUIRKS.md` | 4586, 929 | layout only (headings, the opening paragraphs, the table header, the entry format) |
| `dcroxide:docs/dcroxide-project-brief.md` | 946 | §2.1 (dcr-rs), §4 (engineering principles), §12 (working agreements) in full; the rest by heading |
| `dcroxide:docs/operating.md`, `docs/bench-ledger.md` | 1952, 7809 | headings only (their existence and role are conventions; their content is node-specific) |
| `dcroxide:.github/dit-probe.rs`, `tools/` (14 directories), `fuzz/` (27 targets) | 72, —, — | directory listings and the README's Layout description |
| `dcroxide:crates/*/src/lib.rs`, `crates/*/Cargo.toml` | — | module lists and dependency lists of the crates §12 names; public signatures of `dcroxide-dcrec` secp256k1, `dcroxide-base58`, `dcroxide-crypto`, `dcroxide-txscript` `sign`/`sighash`/`stdaddr`, `dcroxide-wire` `msgtx` |

**Parity pin.** dcrwallet `54b85b33` (2.2.0-pre, module `decred.org/dcrwallet/v5`). Dependencies read: `dcrd/hdkeychain/v3` v3.1.3, `dcrd/dcrec/secp256k1/v4` v4.4.1 (`privkey.go`, `modnscalar.go`), `dcrd/wire` v1.8.0 (`msgtx.go` null sentinels), `google.golang.org/grpc` v1.83.2, `google.golang.org/protobuf` v1.36.11, `companyzero/sntrup4591761` `9e0f3af2f07a` (size only). dcroxide's own parity pin at the commit read is dcrd master `46e9df48` with reference Go 1.27.3 (`dcroxide:docs/PARITY.md:3-5`, `dcroxide:.github/workflows/ci.yml:42`); dcrwallet's `go.mod` declares `go 1.26.0`.

**Summary.**

- dcr-rs is a `no_std + alloc` signer-oriented crate. Its hardened-derivation logic matches dcrd `hdkeychain` v3.1.3 on every derived key value, but its parse rules, error identities, depth handling and degenerate-sum handling deliberately differ from dcrd, and it supports only P2PKH/P2SH, SigHashAll and four hard-coded networks.
- dcr-rs depends on `secp256k1` 0.29 (libsecp256k1 C through `secp256k1-sys`), `bs58`, `bip39` and `minicbor`. The first is refused by name by dcroxide's `deny.toml`, so dcr-rs cannot be a dependency of a workspace that inherits dcroxide's pure-Rust rule.
- For every concern both offer (BLAKE-256, base58, addresses, transaction wire format, sighash, signing, network parameters), the dcroxide crate is the dcrd-parity implementation and the wallet takes it. dcr-rs contributes its `hd.rs` design (the stored-stripped provenance flag), its discriminating test vectors, and nothing else at link time.
- Neither project has an `hdkeychain` port. The wallet needs one, written over `dcroxide-dcrec`, with dcrd's exact parse and error behaviour (§5.1, §12.3).
- dcrwallet seeds are not BIP39. A seed is 16–64 raw bytes, shown to users as hex or as PGP-word-list words with a one-word double-SHA-256 checksum (§4.7, §5.4). dcr-rs's `mnemonic` feature must not be used by the wallet.
- dcroxide's conventions (oracle-driven differential testing, pinned upstream, PARITY/QUIRKS ledgers, pure Rust, no `unsafe`, curated lints, pinned toolchain, dependency ledger, signed releases) transfer unchanged. Five need a wallet-specific decision: the concurrency model against an HTTP/2 gRPC server, the storage engine (the wallet must read and write bbolt files in place, where dcroxide chose a fresh format), the gRPC/protobuf stack under the pure-Rust and no-serde rules, the password-hashing and secretbox crates, and the sntrup4591761 KEM. The node-storage ADRs (0004, 0009, 0010, 0011) do not apply.

## 2. Role in the system

This document is not the specification of a runtime subsystem. It fixes three inputs to the project plan:

1. **The primitive layer.** Which existing Rust code supplies hashing, encoding, keys, transactions and signing to the wallet crates specified in specs 01–24, and what has to be written new. Callers: every wallet crate. Callees: `dcroxide-crypto`, `dcroxide-base58`, `dcroxide-chaincfg`, `dcroxide-dcrec`, `dcroxide-wire`, `dcroxide-txscript`, and the new `hdkeychain` and `walletseed` ports.
2. **The seed and extended-key surface** as dcrwallet exposes it (`walletseed`, `pgpwordlist`, the `hdkeychain` API the wallet calls), because it decides whether dcr-rs's `hd.rs` and `mnemonic` feature are usable. The address manager's use of extended keys is specified in spec 02; this document specifies the library underneath it and the seed encoding above it.
3. **The engineering conventions.** The decisions dcroxide recorded as ADRs and enforces in CI, classified as inherit / decide / not applicable, so the wallet workspace starts with the same rules and the same review vocabulary.

Where this document states dcrd or dcrwallet behaviour, the Go source at the pin is the authority. Where it states a property of a Rust crate that could not be checked from the sources available here, the statement is marked UNVERIFIED.

## 3. Public surface inventory

### 3.1 dcr-rs crate-level items

| Item | Source | Contract |
| --- | --- | --- |
| `#![cfg_attr(not(feature = "std"), no_std)]`, `#![forbid(unsafe_code)]`, `#![warn(missing_docs)]` | `dcr-rs:src/lib.rs:32-34` | The crate itself needs only `alloc`. |
| feature `std` (default) | `dcr-rs:Cargo.toml:40-45` | Enables `secp256k1/std`, `bs58/std`, `bip39?/std`, `minicbor?/std`; implements `std::error::Error`. |
| feature `mnemonic` (default) | `dcr-rs:Cargo.toml:47` | Pulls `bip39`; enables `ExtPrivKey::from_entropy`/`from_phrase`. |
| feature `airgap` (default) | `dcr-rs:Cargo.toml:50` | Pulls `minicbor`; enables the `airgap` module. |
| `pub use secp256k1` | `dcr-rs:src/lib.rs:47` | Re-exports the curve crate; its types (`Secp256k1<All>`, `SecretKey`, `PublicKey`) appear in public signatures. |
| `enum Error` (`Copy`, `#[non_exhaustive]`) | `dcr-rs:src/lib.rs:65-103` | 13 variants; see §8.1. `Display` strings at `:105-124`. |
| re-exports `Address`, `ExtPrivKey`, `ExtPubKey`, `BRANCH_EXTERNAL`, `BRANCH_INTERNAL`, `HARDENED`, `Network`, `MsgTx`, `OutPoint`, `TxIn`, `TxOut`, `format_amount`, `ATOMS_PER_DCR`, and the airgap entry points | `dcr-rs:src/lib.rs:131-140` | Convenience only. |

### 3.2 dcr-rs `blake256`

| Item | Source | Contract |
| --- | --- | --- |
| `OUT_LEN: usize = 32` | `dcr-rs:src/blake256.rs:15` | Digest size. |
| `struct Blake256` (`Clone`, `Default`) | `:56` | State: `h: [u32; 8]`, 64-byte buffer, buffered length, bits compressed so far. |
| `impl Drop for Blake256` | `:78` | Zeroizes the buffer and chaining state. |
| `Blake256::new()` | `:92` | IV = SHA-256 IV; salt is always zero. |
| `Blake256::update(&mut self, &[u8])` | `:166` | Buffered absorb; any split yields the one-shot digest. |
| `Blake256::finalize(self) -> [u8; 32]` | `:208` | BLAKE padding (`0x80`, zeros, `0x01` bit at byte 55, 64-bit big-endian bit length); counter is zero for a padding-only block. |
| `sum256(&[u8]) -> [u8; 32]` | `:250` | One-shot. |
| `sum256d(&[u8]) -> [u8; 32]` | `:258` | `sum256(sum256(x))`; base58check checksum only. |

### 3.3 dcr-rs `hashing`

| Item | Source | Contract |
| --- | --- | --- |
| `hash160(&[u8]) -> [u8; 20]` | `dcr-rs:src/hashing.rs:18` | `RIPEMD160(BLAKE256(x))`. |
| `check_encode(payload, prefix: [u8; 2]) -> String` | `:30` | base58 of `prefix ‖ payload ‖ sum256d(prefix ‖ payload)[..4]`. |
| `MAX_BASE58_LEN: usize = 128` | `:45` | Length gate applied before any base58 decode. |
| `check_decode(&str) -> Result<([u8; 2], Vec<u8>), Error>` | `:56` | Length gate → `Parse`; bad character → `Base58`; decoded length < 6 → `Parse`; checksum → `BadChecksum`. Decode buffer wiped on every exit. |

### 3.4 dcr-rs `network`

| Item | Source | Contract |
| --- | --- | --- |
| `enum Network { Mainnet, Testnet, Simnet, Regnet }` | `dcr-rs:src/network.rs:11` | Closed set; no custom network. |
| `Network::ALL` | `:24` | Lookup order for prefix detection. |
| `name()` | `:32` | `"mainnet"`, `"testnet3"`, `"simnet"`, `"regnet"`. |
| `p2pkh_addr_id()`, `p2sh_addr_id()` | `:42`, `:52` | Two-byte address IDs (§6). |
| `hd_priv_id()`, `hd_pub_id()` | `:62`, `:72` | Four-byte extended key versions (§6). |
| `slip44()` | `:83` | 42 on mainnet, 1 elsewhere. No legacy coin type. |
| `from_p2pkh_id`, `from_p2sh_id`, `from_hd_priv_id`, `from_hd_pub_id` | `:91-108` | Reverse lookups over `ALL`. |

### 3.5 dcr-rs `address`

| Item | Source | Contract |
| --- | --- | --- |
| `enum AddressKind { P2pkh, P2sh }` | `dcr-rs:src/address.rs:27` | No P2PK, no Ed25519 or Schnorr variants. |
| `struct Address { network, kind, hash: [u8; 20] }` | `:36` | Public fields. |
| `Address::p2pkh`, `p2sh`, `from_pubkey(&[u8; 33], Network)` | `:47`, `:56`, `:70` | Constructors; `from_pubkey` hashes a compressed key. |
| `Address::from_script(&[u8], Network) -> Option<Address>` | `:75` | Exact-template match for the 25-byte P2PKH and 23-byte P2SH scripts only. |
| `Address::decode(&str)` | `:84` | Detects network and kind from the prefix; unknown prefix → `UnknownPrefix`; payload not 20 bytes → `Parse`. |
| `Address::decode_for(&str, Network)` | `:119` | As `decode`, plus network equality, else `UnknownPrefix`. |
| `Address::encode()`, `pk_script()`, `Display` | `:128`, `:137`, `:145` | String and payment script. |
| `p2pkh_script`, `p2sh_script`, `p2pkh_hash160`, `p2sh_hash160` | `:152`, `:164`, `:174`, `:191` | Script builders and matchers. |

### 3.6 dcr-rs `hd`

| Item | Source | Contract |
| --- | --- | --- |
| `HARDENED: u32 = 0x8000_0000`, `BRANCH_EXTERNAL = 0`, `BRANCH_INTERNAL = 1` | `dcr-rs:src/hd.rs:95-99` | |
| `MIN_SEED_BYTES = 16`, `MAX_SEED_BYTES = 64` (private) | `:91-92` | Same bounds as dcrd. |
| `child_tweak([u8; 32]) -> Result<Scalar, Error>` (private) | `:122-128` | Rejects `Il >= N` and `Il == 0` with `Derivation`. |
| `struct ExtPrivKey { pub network, pub secret: SecretKey, pub chain_code, pub depth: u8, pub parent_fingerprint: [u8; 4], pub child_number: u32, zeros_stripped: bool }` (`Clone`; no `Debug`) | `:132-176` | `zeros_stripped` is private and records whether dcrd would hold this key as a short byte slice. |
| `impl Drop for ExtPrivKey` | `:191-197` | Overwrites the secret with `0x01` bytes and zeroizes the chain code. |
| `ExtPrivKey::master_from_seed(&[u8], Network)` | `:202-232` | HMAC-SHA512 keyed `"Bitcoin seed"`; `zeros_stripped = false`; any failure → `Derivation`. |
| `ExtPrivKey::from_entropy`, `from_phrase` (feature `mnemonic`) | `:238`, `:249` | BIP39 English mnemonic → 64-byte PBKDF2 seed → `master_from_seed`. Not a dcrwallet seed format (§5.4). |
| `public_key`, `compressed_pubkey`, `fingerprint` | `:258`, `:263`, `:268` | Take a `&Secp256k1<All>` context. |
| `derive_child(secp, index)` | `:280` | dcrd `Child`: the child is stored stripped. |
| `derive_child_bip32_std(secp, index)` | `:292` | dcrd `ChildBIP32Std`: the child is stored unstripped. |
| `derive_child_inner` (private) | `:296-377` | §5.1. |
| `derive_path`, `derive_path_bip32_std` | `:381`, `:391` | Fold of the corresponding child function. |
| `account_key(secp, account)` | `:410-422` | `m/44'/slip44'/account'`; `account >= 2^31` → `HardenedIndex`. No legacy coin type. |
| `address_key(secp, branch, index)` | `:431-441` | Non-hardened only, else `HardenedIndex`. |
| `neuter(secp) -> ExtPubKey` | `:446` | |
| `p2pkh_address(secp) -> String` | `:458` | |
| `to_base58()` | `:468` | 82-byte serialization, base58. |
| `ExtPrivKey::from_base58(&str)` | `:503-540` | Network from the private version only; key data byte 0 must be `0x00`; result is unstripped. |
| `struct ExtPubKey { network, public_key: PublicKey, chain_code, depth: u8, parent_fingerprint, child_number }` (`Copy`, `Debug`, `Eq`) | `:545-558` | |
| `ExtPubKey::compressed_pubkey`, `fingerprint`, `derive_child`, `derive_path`, `pubkey_at`, `p2pkh_address`, `to_base58`, `from_base58` | `:562-677` | Public CKD; hardened index → `HardenedFromPublic`. |
| `serialize_ext_key`, `parse_ext_key`, `RawExtKey`, `KeyData` (private) | `:680-757` | §4.1. |

### 3.7 dcr-rs `tx`, `sighash`, `sign`, `amount`

| Item | Source | Contract |
| --- | --- | --- |
| `SER_FULL = 0`, `SER_NO_WITNESS = 1`, `SER_ONLY_WITNESS = 2` | `dcr-rs:src/tx.rs:18-22` | Serialization type in the high 16 bits of the version word. |
| `NULL_BLOCK_HEIGHT = 0`, `NULL_BLOCK_INDEX = 0xffff_ffff`, `NULL_VALUE_IN = -1` | `:26-30` | Equal to `dcrd/wire@v1.8.0/msgtx.go:43-51`. |
| `MAX_TX_IN_SEQUENCE = 0xffff_ffff` | `:42` | |
| `OutPoint { hash, index, tree: u8 }`, `TxIn`, `TxOut`, `MsgTx { version: u16, tx_in, tx_out, lock_time, expiry }` | `:53-103` | Plain data; `tree` is unsigned. |
| `put_varint`, `varint_size` | `:108`, `:124` | Compact-size encoding. |
| `MsgTx::serialize_prefix`, `serialize_witness`, `serialize_full`, `tx_hash` | `:289-318` | `tx_hash` = BLAKE-256 of the prefix serialization. |
| `MsgTx::parse_full`, `parse_prefix` | `:321`, `:403` | Reject a wrong serialization type, non-canonical varints, counts that cannot fit the remaining bytes, a witness count different from the input count, and trailing bytes; every failure is `Parse`. No `parse_witness`. |
| `SIGHASH_ALL: u32 = 1` | `dcr-rs:src/sighash.rs:23` | The only hash type implemented. |
| `prefix_hash_all(&MsgTx)`, `signature_hash_all(tx, idx, script)`, `signature_hash_all_cached(tx, idx, script, &prefix_hash)` | `:42`, `:52`, `:63` | §4.5. Index out of range → `SigHashIndex`. |
| `sign_p2pkh_input`, `sign_p2pkh_input_cached` | `dcr-rs:src/sign.rs:38`, `:52` | Derives the public key from the secret, requires the prevout script to be that key's P2PKH script (`ScriptMismatch` otherwise), signs with RFC 6979 and low S, returns `PUSH(der ‖ 0x01) PUSH(pubkey)`. |
| `ATOMS_PER_DCR: i64 = 100_000_000`, `format_amount(i64) -> String` | `dcr-rs:src/amount.rs:8`, `:11` | `"1.2345 DCR"` with trailing zeros trimmed. This is not `dcrutil.Amount.String` and is not used by any dcrwallet-compatible output. |

### 3.8 dcr-rs `airgap` (feature `airgap`)

| Item | Source | Contract |
| --- | --- | --- |
| `FORMAT_VERSION: u8 = 3` | `dcr-rs:src/airgap.rs:102` | The only version accepted. |
| `FEE_ALWAYS_ALLOWED_ATOMS = 100_000`, `MAX_FEE_FRACTION_DIVISOR = 20` | `:107`, `:117` | Fee plausibility policy. |
| `MAX_PACKAGE_BYTES = 512 * 1024`, `MAX_ATOMS = 21_000_000 * 10^8`, `MAX_INPUTS = 1000`, `MAX_OUTPUTS = 1000` | `:284`, `:377`, `:382`, `:384` | Hard caps. |
| `InputMeta`, `OutputMeta`, `SignRequest` (`minicbor` `Encode`/`Decode`) | `:121-243` | §4.8. |
| `encode_sign_request`, `decode_sign_request` | `:246`, `:295` | Decode refuses oversize input, trailing bytes and any version but 3. |
| `ReviewSummary` | `:330-365` | Recipients, change, totals, fee, lock time, expiry, non-final sequence flag. |
| `SignRequest::input_total`, `output_total`, `verify_prev_txs`, `validate`, `check_account_fp`, `review_owned`, `check_owned_inputs` | `:444-799` | Structural, economic and ownership checks. |
| `sign_request(secp, master, req) -> Result<Vec<u8>, Error>` | `:812-898` | Re-validates, derives the account key with the Decred variant, signs every input, returns the full serialization. |

dcrwallet has no counterpart to this module. It is listed for completeness and is out of the drop-in scope (§12.2, §13).

### 3.9 `hdkeychain` v3.1.3 — the surface the wallet needs

Every exported identifier of the package, with whether dcrwallet at the pin uses it outside tests.

| Identifier | Source | Used by dcrwallet | Contract |
| --- | --- | --- | --- |
| `RecommendedSeedLen = 32` | `dcrd/hdkeychain/v3@v3.1.3/extendedkey.go:30` | yes | |
| `HardenedKeyStart = 0x80000000` | `:36` | yes | |
| `MinSeedBytes = 16`, `MaxSeedBytes = 64` | `:40`, `:44` | yes | |
| `ErrDeriveHardFromPublic` | `:56` | no direct reference | `cannot derive a hardened key from a public key` |
| `ErrNotPrivExtKey` | `:61` | no direct reference | `unable to create private keys from a public extended key` |
| `ErrInvalidChild` | `:69` | yes (`errors.Is`, 13 sites) | `the extended key at this index is invalid` |
| `ErrUnusableSeed` | `:75` | yes | `unusable seed` |
| `ErrInvalidSeedLen` | `:79` | yes | `seed length must be between 128 and 512 bits` |
| `ErrBadChecksum` | `:84` | no direct reference | `bad extended key checksum` |
| `ErrInvalidKeyLen` | `:88` | no direct reference | `the provided serialized extended key length is invalid` |
| `ErrWrongNetwork` | `:93` | no direct reference | `the provided serialized extended key is for the wrong network` |
| `NetworkParams` interface (`HDPrivKeyVersion`, `HDPubKeyVersion`) | `:104-112` | yes (chain params satisfy it) | |
| `ExtendedKey` | `:117-127` | yes | One type for private and public keys. `key` is a byte slice whose length carries the stripped state. |
| `(*ExtendedKey).ChildNum`, `Depth`, `IsPrivate`, `ParentFingerprint` | `:174-196` | `IsPrivate` only (4 call sites: `internal/prompt/prompt.go:451`, `wallet/udb/addressmanager.go:907`, `:2247`, `wallet/wallet.go:3895`) | `Depth` returns `uint16`; `ParentFingerprint` returns the four bytes as a big-endian `uint32`. |
| `(*ExtendedKey).Child(i)` | `:377` | yes (every derivation) | Decred variant. |
| `(*ExtendedKey).ChildBIP32Std(i)` | `:384` | no | Strict BIP32. |
| `(*ExtendedKey).Neuter()` | `:396` | yes | Returns the receiver itself when already public. For a private key the result shares the parent's memoized public-key, chain-code and parent-fingerprint byte slices (§10 item 25). |
| `(*ExtendedKey).SerializedPubKey()` | `:412` | yes | 33 bytes; memoized for private keys. |
| `(*ExtendedKey).SerializedPrivKey()` | `:422` | yes, one site (`wallet/udb/addressmanager.go:2498`) | Returns the stored slice, which is shorter than 32 bytes for a stripped key. |
| `(*ExtendedKey).String()` | `:441` | yes | base58 serialization, or `zeroed extended key` when the key is empty. |
| `(*ExtendedKey).Zero()` | `:488` | yes | Clears key, memoized public key, chain code, fingerprint; resets depth, child number and `isPrivate`. |
| `NewMaster(seed, net)` | `:507` | yes | |
| `NewKeyFromString(key, net)` | `:538` | yes | |
| `GenerateSeed(length uint8)` | `:623` | yes | `crypto/rand` bytes. |

### 3.10 `walletseed` and `pgpwordlist`

| Identifier | Source | Contract |
| --- | --- | --- |
| `walletseed.GenerateRandomSeed(size uint) ([]byte, error)` | `walletseed/seed.go:21-34` | `size >= 255` or outside 16..64 → kind `Invalid` wrapping `ErrInvalidSeedLen`; otherwise `hdkeychain.GenerateSeed`. |
| `walletseed.checksumByte` (unexported) | `:38-41` | First byte of `SHA256(SHA256(data))`. |
| `walletseed.EncodeMnemonicSlice(seed) []string` | `:44-52` | One word per seed byte plus one checksum word. |
| `walletseed.EncodeMnemonic(seed) string` | `:55-67` | The same words joined by single spaces. |
| `walletseed.DecodeUserInput(input) ([]byte, error)` | `:71-102` | Hex or word list; §5.4. |
| `pgpwordlist.ByteToMnemonic(b byte, index int) string` | `pgpwordlist/pgpwordlist.go:26-32` | Word at table position `2*b + (index mod 2)`. |
| `pgpwordlist.DecodeMnemonics(words []string) ([]byte, error)` | `:36-60` | §5.4. |
| `wordList`, `wordIndexes`, `alternatingWords` (unexported) | `pgpwordlist/wordlist.go:23-33` | 512 newline-separated words; even positions are the two-syllable list, odd positions the three-syllable list; lookup is by lower-cased word. |
| `init()` (unexported) | `pgpwordlist/wordlist.go:27-31` | Fills `wordIndexes` (`map[string]uint16`) with `strings.ToLower(word) → table index` once at package load. The 512 lower-cased words are distinct, so no entry overwrites another. |

Callers at the pin, outside tests (tree search for `walletseed.` and `pgpwordlist.`):

| Function | Production callers | Note |
| --- | --- | --- |
| `walletseed.GenerateRandomSeed` | none. Only the database fixture generators `wallet/udb/testdata/v1.db.go:53`, `v2.db.go:61`, `v3.db.go:53`, `v4.db.go:53`, `v5.db.go:79`, `v7.go:63` call it | The daemon generates seeds with `hdkeychain.GenerateSeed(RecommendedSeedLen)` directly (`internal/prompt/prompt.go:271`, `walletsetup.go:278`, `wallet/wallet.go:5298`) or with `rand.Read` from `dcrd/crypto/rand` (`internal/rpc/rpcserver/server.go:3188-3189`). Its `Invalid`-kind error is therefore not reachable from any dcrwallet surface. |
| `walletseed.EncodeMnemonicSlice` | `internal/prompt/prompt.go:276` | The prompt prints the 33 words six to a line, each followed by one space. |
| `walletseed.EncodeMnemonic` | `walletsetup.go:287`, `internal/rpc/rpcserver/server.go:3194` | |
| `walletseed.DecodeUserInput` | `internal/prompt/prompt.go:350`, `internal/rpc/rpcserver/server.go:3200` | The prompt calls it only for input of two or more words; see §5.4. |
| `pgpwordlist.ByteToMnemonic`, `DecodeMnemonics` | `walletseed` only | |

### 3.11 `hdkeychain` v3.1.3 unexported items

| Identifier | Source | Contract |
| --- | --- | --- |
| `serializedKeyLen = 78` | `dcrd/hdkeychain/v3@v3.1.3/extendedkey.go:50` | `4 + 1 + 4 + 4 + 32 + 33`. |
| `masterKey = []byte("Bitcoin seed")` | `:99` | HMAC key of `NewMaster`. Package-level variable, never written. |
| `ExtendedKey` fields `privVer`, `pubVer` (`[4]byte`), `key`, `pubKey`, `chainCode`, `parentFP` (`[]byte`), `childNum uint32`, `depth uint16`, `isPrivate bool` | `:117-127` | `pubKey` is set only for private keys, lazily. |
| `newExtendedKey(privVer, pubVer, key, chainCode, parentFP, depth, childNum, isPrivate)` | `:132-147` | Stores its arguments without copying or checking; leaves `pubKey` nil. |
| `(*ExtendedKey).pubKeyBytes()` | `:157-171` | Public key: returns `key`. Private key: if `len(pubKey) == 0`, computes `secp256k1.PrivKeyFromBytes(key).PubKey().SerializeCompressed()` and stores it; returns `pubKey`. `PrivKeyFromBytes` left-pads a stripped key, so the public key of a stripped key equals that of its padded form. |
| `hash160(v)` | `:199-204` | `RIPEMD160(BLAKE256(v))`, 20 bytes. |
| `doubleBlake256Cksum(v)` | `:207-211` | First 4 bytes of `BLAKE256(BLAKE256(v))`. |
| `(*ExtendedKey).child(i, strictBIP32)` | `:218-349` | §5.1. |
| `paddedAppend(size, dst, src)` | `:433-438` | Appends `size - len(src)` zero bytes (none when `src` is already `size` or longer), then `src`. |
| `zero(b)` | `:477-482` | Overwrites every byte of `b` with zero. |
| `decodedDataLen = 82`, `maxKeyLen = 113` (function-local constants) | `:552-553` | §4.1. |

Buffer sharing that follows from `newExtendedKey` not copying: a master key's `key` and `chainCode` are the two halves of one 64-byte HMAC output (`:517-523`); a parsed key's `parentFP`, `chainCode` and `key` are sub-slices of the one 82-byte decoded buffer (`:588-591`, `:599`); a child's `key` (private) is a sub-slice of the 32-byte sum array and its `chainCode` the upper half of the HMAC output (`:268-269`, `:301-311`). `Zero` clears each field's bytes, so the unreferenced remainder of those buffers (the version, depth, child number and checksum bytes of a parsed key; the stripped leading zero bytes of a child) is the only part left, and it holds no secret.

### 3.12 dcr-rs unexported items

Listed so the inventory of `dcr-rs:src/` is complete; none is reachable from outside the crate.

| Item | Source | Contract |
| --- | --- | --- |
| `BLOCK_LEN = 64`, `IV` (SHA-256 initial values), `C` (16 BLAKE constants), `SIGMA` (10 permutation rows, used modulo 10 across 14 rounds) | `dcr-rs:src/blake256.rs:16-34` | BLAKE-256 tables. |
| `Blake256::compress(&mut self, block, counter)`, inner `g`, `rounds` | `:104-160` | One compression; `counter` is the total message bit length through this block. |
| `OP_DUP 0x76`, `OP_HASH160 0xa9`, `OP_DATA_20 0x14`, `OP_EQUALVERIFY 0x88`, `OP_EQUAL 0x87`, `OP_CHECKSIG 0xac` | `dcr-rs:src/address.rs:18-23` | The only opcodes the crate knows. |
| `HmacSha512` type alias | `dcr-rs:src/hd.rs:88` | `Hmac<Sha512>`. |
| `SIGHASH_SERIALIZE_WITNESS: u32 = 3` | `dcr-rs:src/sighash.rs:25` | Serialization type in the witness-signing preimage (§4.5 step 2). |
| `build_sig_script(sig, pubkey)` | `dcr-rs:src/sign.rs:80-90` | §4.5; `debug_assert!(der.len() + 1 < 76)` guards the single-byte push opcode. |
| `read_varint`, `read_count(buf, pos, min_item_size)`, `read_var_bytes`, `read_bytes`, `read_u32`, `read_u16`, `read_i64` | `dcr-rs:src/tx.rs:141-221` | Cursor readers; every short read is `Parse`. `read_count` rejects a count whose `count × min_item_size` exceeds the bytes remaining. `read_var_bytes` rejects a length beyond the bytes remaining. |
| `MsgTx::ser_version`, `prefix_body_size` (`pub(crate)`), `witness_body_size`, `write_prefix_body`, `write_witness_body` | `:224-286` | Serialization helpers. |
| `total_atoms`, `branch_slot`, `BranchPubCache { new, pubkey_at, script_at }`, `SignRequest::prove_change_outputs`, `prove_owned_inputs` | `dcr-rs:src/airgap.rs:390-440`, `:671-734` | Airgap helpers: saturating display totals, branch 0/1 → cache slot (anything else → `InvalidRequest("unknown derivation branch")`), a two-slot cache of branch public keys, and the two ownership proofs (§5.7). |

### 3.13 dcroxide convention artefacts

| Artefact | Source | What it fixes |
| --- | --- | --- |
| ADR process | `dcroxide:docs/adr/README.md:3-5`, `template.md` | One numbered file per decision; immutable once accepted; corrections are dated addenda; replacement is a superseding ADR. Template sections: Status, Date, Context, Decision, Consequences. |
| ADR-0001 | `0001-…md:26-41` | Go oracle binary, line-delimited JSON on stdin/stdout, module versions pinned to the parity target, built on demand, mandatory in CI. Addenda: frozen-battery regeneration rule (`:158`), one reference Go release (`:259`). |
| ADR-0002 | `0002-…md:16-28`, `:39` | BLAKE-256 vendored once from dcr-rs and owned; any later import from dcr-rs needs its own decision. |
| ADR-0003 | `0003-…md:14-46` | Slice-based decoding with `(value, consumed)`; error text parity; `encode(decode(b)) == b[..consumed]`; `no_std + alloc` primitives. |
| ADR-0004, 0009, 0010, 0011 | — | Node storage on `redb` with a fresh on-disk format; not applicable to the wallet except ADR-0009's guardrails (`0009-…md:771-791`). |
| ADR-0005 | `0005-…md:53-92`, `:94-139` | No async runtime: OS threads over `std::sync::mpsc`, runtime-free core crates, single writer; `panic = "abort"` in release. |
| ADR-0006 → ADR-0017, ADR-0016 | `0017-…md:57-108` | secp256k1: secrets on `k256` in constant time, public-value arithmetic on the owned `dcroxide-secp256k1-vartime`, dcrd's acceptance rules in `dcroxide-dcrec`; Ed25519 on `curve25519-dalek`. |
| ADR-0007 | `0007-…md:43-104` | No serde on the RPC path; Go `encoding/json` reimplemented over a runtime type model; frozen vectors dumped inside dcrd's packages and replayed byte for byte. |
| ADR-0008 | `0008-…md:110-216`, `:260` | Lint set and its refusals; explicit arithmetic policy. |
| ADR-0012 | `0012-…md:122-174` | Pure Rust: no C/C++ in anything shipped or built by `cargo test`; no cryptographic assembly; enforcement by `cargo-deny` bans and a no-C-compiler CI job. |
| ADR-0013 | `0013-…md:121-173` | The pin follows upstream release tags once the target release is tagged; master is read ahead into notes. |
| ADR-0014 | `0014-…md:117-266` | MSRV equals the pinned toolchain; no binary before a four-condition gate; SSH-signed tag, PGP-signed `SHA256SUMS`, artifact attestations, double build and off-CI rebuild. |
| ADR-0015 | `0015-…md:216-239`, `:332-366` | In-tree rustls crypto provider (`dcroxide-tlscrypto`) on RustCrypto; no process-default provider. |
| Dependency ledger | `dcroxide:docs/dependency-ledger.md:22-36`, `:61-73`, `:167-172` | One decision per load-bearing dependency; a table of permitted inline assembly. |
| Workspace manifest | `dcroxide:Cargo.toml` | Edition 2024, `resolver = "3"`, `rust-version` = toolchain pin, release/dist/soak profiles, workspace lints. |
| `clippy.toml`, `deny.toml`, `rust-toolchain.toml`, `_typos.toml` | — | §6.4. |
| CI | `dcroxide:.github/workflows/ci.yml`, `constant-time.yml` | §5.6. |
| `PARITY.md`, `QUIRKS.md` | `dcroxide:docs/PARITY.md:1-25`, `:1009`, `:1047`, `:1129`; `dcroxide:docs/QUIRKS.md:1-24` | §4.10. |
| Project brief | `dcroxide:docs/dcroxide-project-brief.md:525-535`, `:928-940` | Seven engineering principles and the definition of done for every task (§5.6). |
| `docs/operating.md` | `dcroxide:docs/operating.md:1-13` | The operator's document: supervisor requirement, data-directory stance, tuning knobs, paths and environment, status of what is running. |
| `docs/bench-ledger.md` | `dcroxide:docs/bench-ledger.md:1-40` | Every performance figure the project quotes, with machine, commit and method, appended in dated sections; superseded README text is retired into it. |
| `tools/` | `dcroxide:README.md:772-797` | Kept Go tools beside the oracle: `oracle`, `helpgen`, `pinbump` (pin-move review list), vector exporters that run as overlays inside upstream's packages (`mixpoolgen`, `mempoolvectors`, `utxoviewgen`, `chainopsgen`, `disapprovalgen`, `trsyboundarygen`, `profilergen`, `fullblockgen`), and measurement tools (`dcrdstat`, `dsample`, `powerloss`). |
| `fuzz/` | `dcroxide:Cargo.toml:40`; `dcroxide:.github/workflows/ci.yml:57-62`, `:572-577` | A `cargo-fuzz` workspace of its own, excluded from the main workspace, with its own `Cargo.lock`, built on nightly through `RUSTUP_TOOLCHAIN`. 27 targets at the commit read. |
| `.github/dit-probe.rs` | `dcroxide:docs/dependency-ledger.md` (end of the assembly section) | A stand-alone probe the constant-time workflow's arm64 legs compile with plain `rustc`; no workspace crate. |
| `SECURITY.md` | `dcroxide:SECURITY.md:3-63`, `:481` | Pre-alpha status statement, private reporting, in/out of scope, itemized known gaps, "what this project does instead of a guarantee". |

## 4. Data formats

### 4.1 Serialized extended key (82 bytes before base58)

Identical in dcrd (`dcrd/hdkeychain/v3@v3.1.3/extendedkey.go:450-472`, `:564-591`) and dcr-rs (`dcr-rs:src/hd.rs:687-717`, `:728-757`).

| Offset | Size | Field | Encoding |
| ---: | ---: | --- | --- |
| 0 | 4 | version | network HD private or public key ID, as stored (big-endian byte order of the constant) |
| 4 | 1 | depth | dcrd: `byte(depth % 256)` of a `uint16` (`:447`); dcr-rs: the `u8` field |
| 5 | 4 | parent fingerprint | first 4 bytes of `RIPEMD160(BLAKE256(parent compressed pubkey))`; `00 00 00 00` for a master |
| 9 | 4 | child number | `uint32`, big-endian |
| 13 | 32 | chain code | raw |
| 45 | 33 | key data | private: `0x00` then the key left-padded with zeros to 32 bytes (`paddedAppend`, `:433-438`, `:464-465`); public: 33-byte compressed point (`0x02`/`0x03` prefix) |
| 78 | 4 | checksum | first 4 bytes of `BLAKE256(BLAKE256(bytes 0..78))` |

The string form is `base58(bytes 0..82)` with the Bitcoin alphabet. With the four networks' version bytes the string is 111 characters and begins `dprv`/`dpub`, `tprv`/`tpub`, `sprv`/`spub`, `rprv`/`rpub`. dcrd bounds the accepted string length at `(82 * 137 / 100) + 1 = 113` characters (`:552-556`); dcr-rs bounds it at 128 (`dcr-rs:src/hashing.rs:45`, `dcr-rs:src/hd.rs:730`).

A zeroed key (empty `key` slice) renders in dcrd as the literal string `zeroed extended key` (`:442-444`). dcr-rs has no such state.

### 4.2 In-memory extended key state that affects output

dcrd's `ExtendedKey.key` for a private key is a byte slice of length 32 or less. The length is the only record of whether the key came out of `Child` (leading zero bytes removed), and it changes the bytes hashed when that key is a hardened parent (§5.1 step 3). It is never serialized: `String` pads back to 32. The producers:

| Producer | Stored private key length | Source |
| --- | --- | --- |
| `NewMaster` | always 32 (the HMAC left half as is) | `extendedkey.go:522`, `:532-533` |
| `NewKeyFromString` | always 32 (`keyData[1:]`) | `:599`, `:613-614` |
| `child(i, strictBIP32 = false)` (`Child`) | 32 minus the number of leading zero bytes of the 32-byte sum | `:301-311` |
| `child(i, strictBIP32 = true)` (`ChildBIP32Std`) | always 32 | `:301-311` |

dcr-rs models this with `ExtPrivKey::zeros_stripped: bool` (`dcr-rs:src/hd.rs:175`): `false` from `master_from_seed` (`:227`) and `from_base58` (`:531`), `!strict_bip32` from a derivation (`:372`). The stripped length is recomputed from the padded secret when needed (`:328-332`).

`SerializedPrivKey` returns the stored slice itself (`extendedkey.go:422-428`), so its length is 32 minus the stripped count. The one dcrwallet caller passes it to `secp256k1.PrivKeyFromBytes` (`wallet/udb/addressmanager.go:2498-2503`), which left-pads to 32 bytes and reduces modulo the group order (`dcrd/dcrec/secp256k1/v4@v4.4.1/privkey.go` `PrivKeyFromBytes`; `modnscalar.go:360-368`), so the short form does not reach any output there.

### 4.3 Base58check (addresses, WIF)

`base58(prefix(2) ‖ payload ‖ BLAKE256(BLAKE256(prefix ‖ payload))[0..4])` (`dcr-rs:src/hashing.rs:30-37`). dcr-rs encodes with the `bs58` crate; dcroxide ports `decred/base58` v1.0.6 including its quirk that `Decode` returns an empty slice for empty input and for any invalid character (`dcroxide:crates/dcroxide-base58/src/lib.rs:2-8`, `:38`). The two agree on every valid string; they differ in how an invalid string fails (§8.1).

### 4.4 Addresses and scripts (dcr-rs subset)

| Kind | Payload | pkScript | Source |
| --- | --- | --- | --- |
| P2PKH (ECDSA secp256k1) | `hash160(compressed pubkey)`, 20 bytes | `76 a9 14 <20> 88 ac` (25 bytes) | `dcr-rs:src/address.rs:152-161` |
| P2SH | `hash160(redeem script)`, 20 bytes | `a9 14 <20> 87` (23 bytes) | `dcr-rs:src/address.rs:164-171` |

Address IDs and HD versions are in §6.1. dcr-rs has no P2PK addresses, no Ed25519 or Schnorr-secp256k1 pubkey-hash addresses, no stake script forms and no script-version handling. `dcroxide-txscript`'s `stdaddr` has all address kinds and the stake script builders (`dcroxide:crates/dcroxide-txscript/src/stdaddr.rs:153`, `:220-389`, `:483-609`).

### 4.5 Transaction serialization and signature hash (dcr-rs subset)

Version word: `uint32` little-endian, `version | (serType << 16)` (`dcr-rs:src/tx.rs:224-226`).

Prefix body (`dcr-rs:src/tx.rs:258-275`):

| Field | Encoding |
| --- | --- |
| input count | compact-size varint |
| per input | 32-byte previous hash, `uint32` LE index, 1-byte tree, `uint32` LE sequence (41 bytes) |
| output count | varint |
| per output | `int64` LE value, `uint16` LE script version, varint script length, script |
| lock time, expiry | `uint32` LE each |

Witness body (`:277-286`): varint input count; per input `int64` LE value in, `uint32` LE block height, `uint32` LE block index, varint script length, signature script.

Full = version word with type 0, prefix body, witness body. Txid = `BLAKE256(version word with type 1 ‖ prefix body)` (`:314-318`).

SigHashAll signature hash for input `idx` (`dcr-rs:src/sighash.rs:63-100`):

1. `prefixHash` = the txid (`:42-44`).
2. `witnessBuf` = `uint32` LE `version | (3 << 16)`, varint input count, then for each input a varint script length and script: the signing script for input `idx`, length 0 for the others.
3. `witnessHash = BLAKE256(witnessBuf)`.
4. `sighash = BLAKE256(uint32 LE 1 ‖ prefixHash ‖ witnessHash)`.

This is the SigHashAll case of dcrd's `calcSignatureHash` only. `SigHashNone`, `SigHashSingle` and `SigHashAnyOneCanPay`, which alter the prefix (output and sequence handling), are not implemented; `dcroxide-txscript` implements all of them (`dcroxide:crates/dcroxide-txscript/src/sighash.rs:16-37`, `:343-355`).

P2PKH signature script (`dcr-rs:src/sign.rs:80-90`): `len(der)+1`, DER signature, `0x01`, `0x21`, 33-byte compressed public key.

### 4.6 Compact-size varint

| Value | Encoding | Source |
| --- | --- | --- |
| `< 0xfd` | 1 byte | `dcr-rs:src/tx.rs:108-121` |
| `<= 0xffff` | `0xfd`, `uint16` LE | |
| `<= 0xffff_ffff` | `0xfe`, `uint32` LE | |
| larger | `0xff`, `uint64` LE | |

Decoding rejects a non-minimal encoding (`:141-175`), as `wire.ReadVarInt` does.

### 4.7 Wallet seed encodings

A dcrwallet seed is a byte string of 16 to 64 bytes (`walletseed/seed.go:98`). It is the direct input to `hdkeychain.NewMaster`; there is no key stretching, salt, passphrase or word-to-entropy checksum step between the user-visible encoding and the seed bytes.

Two encodings are accepted on input and two are produced on output:

| Encoding | Form | Source |
| --- | --- | --- |
| Hex | The seed bytes as hexadecimal, a single whitespace-free token. Decoded with Go `hex.DecodeString`, so either letter case is accepted and an odd length or a non-hex character is an error. | `walletseed/seed.go:76-82` |
| Mnemonic | `len(seed) + 1` words separated by single spaces. Word `i` (0-based) is `wordList[2*seed[i] + (i mod 2)]`. The final word is `wordList[2*c + (len(seed) mod 2)]` where `c = SHA256(SHA256(seed))[0]`. | `walletseed/seed.go:36-67`, `pgpwordlist/pgpwordlist.go:26-32` |

The word table is the PGP word list: 512 words in one newline-separated constant, alternating a two-syllable word (even table index, used at even byte positions) and a three-syllable word (odd table index, used at odd byte positions) for each byte value 0..255 (`pgpwordlist/wordlist.go:23`, `:33-544`). Table index 0 is `aardvark`, 1 `adroitness`, 510 `Zulu`, 511 `Yucatan`. Words are emitted with the table's capitalization (for example `Istanbul`, `Pluto`, `Apollo`); lookup on input lower-cases both sides (`pgpwordlist/wordlist.go:27-31`, `pgpwordlist/pgpwordlist.go:46`). The Rust port embeds the 512 words verbatim from `pgpwordlist/wordlist.go` in table order.

A 32-byte seed (the default, `hdkeychain.RecommendedSeedLen`) is therefore 33 words. This is unrelated to BIP39: BIP39 maps 11 bits per word from a 2048-word list, appends 4–8 checksum bits of single SHA-256, and stretches the phrase through PBKDF2-HMAC-SHA512 into a 64-byte seed. A dcrwallet word list fed to a BIP39 decoder fails, and a BIP39 phrase fed to `DecodeUserInput` fails on the first word that is not in the PGP list.

gRPC `SeedService.GenerateRandomSeed` returns the seed three ways, as bytes, as lower-case hex and as the mnemonic string (`internal/rpc/rpcserver/server.go:3177-3196`); the interactive prompt prints the mnemonic words from `EncodeMnemonicSlice` and then the hex (`internal/prompt/prompt.go:271-287`); the `--createtemp` simulation-wallet path (`createSimulationWallet`) writes `EncodeMnemonic(seed)` to a file named `seed` in the network directory (`walletsetup.go:277-290`). The exact prompt text and RPC messages belong to the specs that own those surfaces.

### 4.8 dcr-rs airgap CBOR package (format version 3)

Not a dcrwallet format. Recorded so the decision in §13 is informed.

Each struct is a CBOR array whose element positions are the `n(..)` indexes; trailing `None` elements are omitted (`dcr-rs:src/airgap.rs:166-172`).

| Struct | Index: field (CBOR type) | Source |
| --- | --- | --- |
| `SignRequest` | 0 `format_version` (uint), 1 `tx_version` (uint), 2 `account` (uint), 3 `lock_time` (uint), 4 `expiry` (uint), 5 `inputs` (array), 6 `outputs` (array), 7 `account_fp` (optional 4-byte byte string) | `:212-243` |
| `InputMeta` | 0 `prev_hash` (32-byte byte string), 1 `prev_index`, 2 `tree`, 3 `sequence`, 4 `value_in` (int), 5 `branch`, 6 `index`, 7 `prev_script` (byte string), 8 `prev_tx_prefix` (byte string; typed optional, required by `validate`) | `:121-173` |
| `OutputMeta` | 0 `value` (int), 1 `version`, 2 `pk_script` (byte string), 3 `is_change` (bool), 4 `branch` (optional uint), 5 `index` (optional uint) | `:177-208` |

Transport names in the module documentation: UR types `dcr-sign-request` and `dcr-signed-tx`; files `unsigned.dcrtx` and `signed.dcrtx` (`:11-14`). Interoperability of version 3 with the KeyOS and Keystone encoders is stated by dcr-rs itself to be unverified (`dcr-rs:README.md:219-224`, `dcr-rs:SECURITY.md:79-83`).

### 4.9 gRPC wire format the wallet server must reproduce

dcrwallet serves gRPC with grpc-go v1.83.2 and protobuf-go v1.36.11. What the server is configured with (`rpcserver.go:271-332`):

| Property | Value | Source |
| --- | --- | --- |
| Transport | TLS only; gRPC is not started when server TLS is disabled or when no client CA is available | `rpcserver.go:264-265`, `:316-321` |
| TLS minimum version | 1.2 | `rpcserver.go:274` |
| Client authentication | `RequireAndVerifyClientCert` against the client CA pool, always for gRPC | `rpcserver.go:321-322` |
| ALPN | grpc-go appends `h2` to the TLS config's `NextProtos` | `grpc@v1.83.2/credentials/tls.go:239` |
| Server options | credentials, one stream interceptor, one unary interceptor; nothing else | `rpcserver.go:328-332` |
| Services | 11 (`VersionService`, `WalletService`, `WalletLoaderService`, `AccountMixerService`, `TicketBuyerService`, `SeedService`, `AgendaService`, `VotingService`, `MessageVerificationService`, `NetworkService`, `DecodeMessageService`) | `rpc/api.proto:7-133`, `:1003` |
| Methods | 88, of which 11 server-streaming and 1 bidirectional-streaming (`ConfirmationNotifications`) | `rpc/api.proto:33-108` |
| Interceptors | log at debug on entry, call `rpcserver.ServiceReady(serviceName)` and return its error before the handler, log handler errors (streaming: at debug when the code is `Canceled` and the context is done) | `rpcserver.go:406-447` |
| Compression | no compressor is registered: no file in the dcrwallet tree imports `grpc/encoding/gzip` | tree search at the pin |
| Reflection, health service | not registered: no import of `grpc/reflection` or `grpc/health` | tree search at the pin |

Because no option overrides them, grpc-go's defaults are observable:

| Default | Value | Source |
| --- | --- | --- |
| Maximum received message | 4 MiB (`1024 * 1024 * 4`) | `grpc@v1.83.2/server.go:61` |
| Maximum sent message | `math.MaxInt32` | `server.go:62` |
| Initial stream and connection window | 65,535 | `internal/transport/defaults.go:28-30` |
| Server keepalive ping interval / timeout | 2 hours / 20 seconds | `defaults.go:38-39` |
| Keepalive enforcement | A client PING is a strike when it arrives less than 5 minutes after the previous one while a stream is active, or less than 2 hours after the previous one while no stream is active. Strikes reset when the server next writes headers or data. More than 2 strikes → `GOAWAY` with `ENHANCE_YOUR_CALM` and debug data `too_many_pings`, then the connection closes | `defaults.go:40`, `internal/transport/http2_server.go:881-882`, `:904-930` |
| Maximum connection idle / age | unlimited | `defaults.go:35-37` |
| Maximum header list size | 16 MiB | `defaults.go:48` |

The byte-level contract a client observes:

1. **HTTP/2 over TLS with ALPN `h2`.** Each call is one HTTP/2 stream. Request pseudo-headers and headers the server acts on: `:method` (must be `POST`), `:path /walletrpc.<Service>/<Method>`, `content-type` (must be `application/grpc`, optionally followed by `+` or `;` and a subtype), `grpc-timeout`, `grpc-encoding`, `grpc-accept-encoding`, and `:authority` or `host` (one of them must be present). A `connection` header makes the request malformed (stream reset with `PROTOCOL_ERROR`). `te` is a reserved name that is dropped without being validated; `user-agent` and `:authority` are passed through to the handler's metadata (`internal/transport/http2_server.go:425-530`, `:562-570`, `internal/transport/http_util.go:96-127`, `internal/grpcutil/method.go:46-78`).
2. **Length-prefixed messages.** Every message in either direction is a 5-byte prefix followed by the payload: byte 0 is the compressed flag (0 = identity, 1 = compressed with the stream's `grpc-encoding`), bytes 1..4 are the payload length as `uint32` big-endian (`grpc@v1.83.2/rpc_util.go:731-732`, `:872-893`). A request whose `grpc-encoding` header names anything other than `identity` for which the server has no registered compressor fails at stream start, before any message is read and whatever the per-message flag says, with code `Unimplemented` and message `grpc: Decompressor is not installed for grpc-encoding "<name>"`, the name formatted with Go `%q` (`server.go:1361-1369`, `:1686-1693`). A message with the flag set on a stream with no `grpc-encoding` or with `identity` fails with code `Internal`, `grpc: compressed flag set with identity or empty encoding` (`rpc_util.go:908-914`).
3. **Response headers.** `:status 200`, `content-type application/grpc` (with the request's subtype when one was sent), and `grpc-encoding` only when the response is compressed (`http2_server.go:1053-1056`).
4. **Status trailers.** The stream ends with a HEADERS frame carrying `grpc-status` (the code as a decimal integer) and `grpc-message`, in that order, then `grpc-status-details-bin` when the status has details, then trailer metadata (`http2_server.go:1109-1128`). When no response headers have been sent yet (no message was written) and the handler set no header metadata, the response is trailers-only: one HEADERS frame holding `:status`, `content-type`, `grpc-status`, `grpc-message` with END_STREAM (`:1098-1110`). If header metadata was set, a separate header frame precedes the trailer frame (`:1099-1103`); no dcrwallet code does this: outside the generated `.pb.go` files there is no call to `SetHeader`, `SendHeader` or `SetTrailer` in `internal/rpc` or `rpcserver.go` (tree search at the pin), so every failure before the first message is trailers-only and no trailer metadata beyond the status fields is sent.
5. **`grpc-message` encoding.** Bytes `0x20..0x7E` other than `%` pass through; every other byte is written as `%XX` with upper-case hex, per UTF-8 byte; invalid UTF-8 becomes U+FFFD and is written `%EF%BF%BD`. An empty message is sent as an empty value (`internal/transport/http_util.go:221-262`).
6. **Unknown paths.** A known service with an unknown method: `Unimplemented`, `unknown method <M> for service <S>`. An unknown service: `Unimplemented`, `unknown service <S>`. A path without the `/service/method` shape: `Unimplemented`, `malformed method name: "<path>"`, the path formatted with Go `%q` (`server.go:1787-1788`, `:1866-1880`).
7. **Oversize messages.** A received message over the limit: `ResourceExhausted`, `grpc: received message larger than max (<n> vs. <max>)` (`rpc_util.go:798`).

Protobuf encoding rules that decide response bytes:

| Rule | Source |
| --- | --- |
| `syntax = "proto3"`, package `walletrpc`; generated with protoc-gen-go v1.36.10 and protoc v6.33.0 | `rpc/api.proto:1-3`, `rpc/walletrpc/api.pb.go:1-4` |
| Known fields are written in ascending field-number order, regardless of declaration order | `protobuf@v1.36.11/internal/impl/codec_message.go:158-160` |
| Unknown fields retained from a decoded message are written after the known fields | `protobuf@v1.36.11/internal/impl/encode.go:113`, `:220` |
| Marshalling uses `proto.MarshalOptions{UseCachedSize: true}`; `Deterministic` is not set | `grpc@v1.83.2/encoding/proto/proto.go:64` |
| proto3 implicit presence: a scalar, enum, string or bytes field equal to its zero value is not written; an unset message field is not written; a set message field is written even when empty | proto3 semantics as implemented by protobuf-go; UNVERIFIED against the generated code path by path — settle by a golden-bytes battery (§11.4) |
| Repeated scalar numeric fields are packed. `api.proto` has two `repeated uint32` fields (both named `unsigned_input_indexes`, `rpc/api.proto:570`, `:591`; the `repeated uint32` at `:565` is inside a comment); the other repeated fields are `bytes`, `string` and messages, which are never packed, and no enum field is repeated | `rpc/api.proto` (type survey); packing rule UNVERIFIED in the same sense as the row above |
| `sint32` fields (five, all block heights) use zigzag varints; `double` (`fee_per_kb`) is 8 bytes little-endian IEEE 754 | `rpc/api.proto:373`, `:401`, `:1143`, `:1181-1194` |
| `api.proto` declares no `map<>`, no `oneof`, no `optional` and no imports, so map ordering and explicit presence do not arise | `rpc/api.proto` (feature survey) |

The per-message field tables and the handler-level status codes and messages belong to the gRPC server spec; this section fixes only what the transport and codec contribute.

### 4.10 Ledger file formats inherited from dcroxide

`PARITY.md` (`dcroxide:docs/PARITY.md`):

1. An opening paragraph naming the single upstream pin (commit, version, protocol versions) and its history (`:3-23`).
2. An "oracle pin" paragraph stating where frozen vectors were generated and what each pin move regenerated (`:25-…`).
3. One account per pin move that ports or accounts for every upstream commit in the range (ADR-0013 item 6, `dcroxide:docs/adr/0013-upstream-tracking-release-tags.md:153-159`). At the commit read these are not headed sections: each is a run of paragraphs opened by a bold lead of the form `**<from>..<to> (N commits).**` (`:163`, `:239`, `:270`, `:499`, `:910`, `:937`). The file's only headings are its title and those of item 5.
4. A status legend (`:999-1006`: `—` not started, `WIP`, `vectors`, `semantics`, `diff`, `parity`) and one table, `| dcrd package | dcroxide crate | Status | Notes |`, with a row per upstream package (35 rows at the commit read; header at `:1009`). Status values in use: `vectors`, `vectors + diff`, `semantics`, `—`.
5. `## Deliberate divergences from dcrd: safety controls Go's runtime made unnecessary` (`:1047`) with a `### Known remaining gaps` subsection (`:1129`), itself split into `#### Accepted: the port matches dcrd here` (`:1141`) and `#### Open: the port does not match dcrd here` (`:2471`).

`QUIRKS.md` (`dcroxide:docs/QUIRKS.md:1-24`): upstream behaviour that deviates from upstream's own documentation and is reproduced on purpose. Numbered entries `## QK-NNNN — short title` with the fields **Where**, **What**, **Why reproduced**, **Pinned by** (test names), and **Status** (present only when the entry no longer describes current parity or nothing pins it). Retired entries keep their number and are marked `RETIRED`. A divergence the port cannot or will not reproduce is not a quirk; it goes in `PARITY.md` (`dcroxide:docs/adr/0007-json-emission-strategy.md`, 2026-10-26 addendum, last correction).

Dependency ledger rows (`dcroxide:docs/dependency-ledger.md:22-36`, `:40`): `| crate | version | role | decision | notes |` with the decision vocabulary `vendored`, `owned`, `reviewed@V`, `accepted`, `declined`; a second table `| crate | version | what the assembly does | compiled for |` lists every permitted `asm!` site.

## 5. Algorithms and state machines

### 5.1 Child derivation: dcrd `child` against dcr-rs, step by step

`child(i, strictBIP32)` (`dcrd/hdkeychain/v3@v3.1.3/extendedkey.go:218-349`). `Child(i)` is `child(i, false)` (`:377-379`); `ChildBIP32Std(i)` is `child(i, true)` (`:384-386`). dcrwallet calls only `Child`.

| # | dcrd | dcr-rs private path (`dcr-rs:src/hd.rs:296-377`) | dcr-rs public path (`:574-609`) | Parity |
| ---: | --- | --- | --- | --- |
| 1 | If the key is public and `i >= 0x80000000`, return `ErrDeriveHardFromPublic` (`:228-231`). | not reachable (separate type) | `HardenedFromPublic` (`:575-577`) | same condition |
| 2 | Allocate `data`, 37 zero bytes (`:241-242`). | `data = [0u8; 37]` (`:333`) | streams the two parts into the HMAC | same bytes |
| 3 | Hardened: `copy(data[1:], k.key)` (`:248`). The stored key lands at offset 1 with its stored length; for a stored length `L < 32` bytes `1..1+L` hold the key and bytes `1+L..33` stay zero. | Copies `key[skip..]` to `data[1..33-skip]`, where `skip` is the count of leading zero bytes when `zeros_stripped`, else 0 (`:328-334`). | — | same bytes |
| 4 | Non-hardened: `copy(data, k.pubKeyBytes())` (`:254`); for a private key the compressed public key is computed once and memoized (`:157-171`). | `mac.update(parent_pubkey)` (`:341`) | `mac.update(compressed_pubkey)` (`:580`) | same bytes |
| 5 | `data[33..37] = i` big-endian (`:256`). | `:335`, `:342` | `:581` | same |
| 6 | `ilr = HMAC-SHA512(key = chainCode, data)`; `il = ilr[0..32]`, child chain code `= ilr[32..64]` (`:261-269`). | `:308`, `:344`, `:359-360` | `:579-582`, `:595-596` | same |
| 7 | Parse `il` as a scalar; if it is `>= N` or zero, return `ErrInvalidChild` (`:277-280`). This check precedes the private/public split. | `child_tweak` → `Derivation` (`:122-128`, `:353`) | same helper (`:589`) | same condition; different error identity |
| 8 | Private: `childKey = (il + parentKey) mod N` as 32 big-endian bytes (`:298-302`). The parent is read with `SetByteSlice`, which left-pads a short stored key. **The sum is not checked for zero.** | `secret.add_tweak(&tweak)`; libsecp256k1 fails when the sum is zero → `Derivation` (`:354-357`) | — | **differs** when `il + parent ≡ 0 (mod N)`: dcrd returns a key, dcr-rs an error |
| 9 | Private, `strictBIP32 == false`: while the first byte of `childKey` is zero and the slice is non-empty, drop it (`:309-311`). An all-zero sum would become an empty slice. | Sets `zeros_stripped = !strict_bip32` (`:372`); the secret stays 32 bytes. | — | same derived values for every non-degenerate key |
| 10 | Public: `P = il·G` (variable time); if `P.x == 0` or `P.y == 0`, return `ErrInvalidChild` (`:317-322`). | — | not checked separately | unreachable in practice |
| 11 | Public: parse the stored parent key; a parse error is returned as is (`:326-330`). `child = P + parent`, serialized compressed (`:337-341`). **The sum is not checked for the point at infinity.** | — | `public_key.add_exp_tweak` fails at infinity → `Derivation` (`:590-593`) | **differs** when the sum is the identity |
| 12 | Parent fingerprint = `hash160(parent compressed pubkey)[0..4]` (`:346`). | `:362`, `:368` | `:603` | same |
| 13 | Result: versions copied from the parent, `depth = parent.depth + 1` in `uint16` (wraps at 65,536), child number `i`, `isPrivate` as the parent (`:347-348`). | `depth.checked_add(1)`, `u8`; the 256th level returns `Derivation` (`:302`) | same (`:578`) | **differs** beyond depth 255 |

Consequences of steps 3 and 9, stated as the rule the port implements: the hardened HMAC preimage is laid out from the parent's *stored* form; `strictBIP32` governs only whether the *child* is stored stripped. A `ChildBIP32Std` step from a stripped parent still uses the left-aligned layout; a `Child` step from a master or from a key parsed from a string uses the standard layout. dcr-rs reproduces all four combinations and pins them with vectors generated from dcrd (`dcr-rs:tests/vectors.rs:590-629`, `:830-853`, `:862-883`, `:890-933`).

Layout example for a parent whose 32-byte key begins with exactly one zero byte (`k31` is the remaining 31 bytes):

| Parent stored form | Bytes 0..36 of the HMAC data |
| --- | --- |
| unstripped (master, parsed, or `ChildBIP32Std` output) | `00` `00` `k31` `i(4)` |
| stripped (`Child` output) | `00` `k31` `00` `i(4)` |

### 5.2 `NewMaster(seed, net)`

`dcrd/hdkeychain/v3@v3.1.3/extendedkey.go:507-534`:

1. If `len(seed) < 16` or `> 64`, return `ErrInvalidSeedLen`.
2. `lr = HMAC-SHA512(key = "Bitcoin seed", seed)`; secret = `lr[0..32]`, chain code = `lr[32..64]`.
3. If the secret, as a scalar, is `>= N` or zero, return `ErrUnusableSeed`.
4. Return a private key with parent fingerprint `00 00 00 00`, depth 0, child number 0, the 32-byte secret stored as is.

dcr-rs `master_from_seed` (`dcr-rs:src/hd.rs:202-232`) performs the same steps and returns `Error::Derivation` for both failures.

`GenerateSeed(length uint8)` (`extendedkey.go:623-636`): `length` outside 16..64 → `ErrInvalidSeedLen`; otherwise `length` bytes from `crypto/rand`. dcr-rs has no counterpart.

### 5.3 `NewKeyFromString(key, net)` and `String()`

`extendedkey.go:538-615`, in order:

1. If `len(key) > 113`, return `ErrInvalidKeyLen`.
2. `decoded = base58.Decode(key)`. An invalid character yields an empty slice. If `len(decoded) != 82`, return `ErrInvalidKeyLen`.
3. If `decoded[78..82] != BLAKE256(BLAKE256(decoded[0..78]))[0..4]`, return `ErrBadChecksum`.
4. If the version is neither `net.HDPrivKeyVersion()` nor `net.HDPubKeyVersion()`, return `ErrWrongNetwork`.
5. Read depth (1 byte, widened to `uint16`), parent fingerprint, child number (big-endian), chain code, key data.
6. `isPrivate = keyData[0] == 0x00`. **The version read in step 4 is not consulted.**
7. Private: the 32 bytes after the zero must be a scalar in `[1, N-1]`, else `ErrUnusableSeed`. Public: `secp256k1.ParsePubKey(keyData)` must succeed, else that error is returned unwrapped.
8. Return a key carrying the *network's* private and public versions (not the version from the string), the 32-byte or 33-byte key data, and `isPrivate`.

`String()` (`:441-473`): empty key → `zeroed extended key`; otherwise the layout of §4.1, choosing the private or public version from `isPrivate`.

Consequence of steps 6 and 8: a string with the public version and private key data parses as a private key and re-serializes with the private version, and the reverse. dcr-rs refuses both (`dcr-rs:src/hd.rs:506-518`, `:659-660`; `dcr-rs:tests/vectors.rs:160-195`).

dcr-rs `parse_ext_key` order (`dcr-rs:src/hd.rs:728-757`): length over 128 → `Parse`; base58 error → `Base58`; decoded length not 82 → `Parse`; checksum → `BadChecksum`; then per type, unknown version → `UnknownPrefix`, wrong key-data type or invalid key → `Parse`. No expected network is taken; the network is detected from the version.

### 5.4 Seed decoding

`walletseed.DecodeUserInput(input)` (`walletseed/seed.go:71-102`):

1. `words = strings.Split(strings.TrimSpace(input), " ")`. Only the space character splits; tabs and newlines inside the input do not.
2. If there is exactly one word, decode it as hex. A hex error returns kind `Encoding` wrapping Go's hex error.
3. If there is more than one word, call `pgpwordlist.DecodeMnemonics(words)`; an error returns kind `Encoding` wrapping it. If fewer than 2 bytes decoded, leave the seed empty (falls to step 5). Otherwise require `SHA256(SHA256(decoded[..n-1]))[0] == decoded[n-1]`, else kind `Encoding` with the message `checksum mismatch`; the seed is `decoded[..n-1]`.
4. (No other case exists: `Split` never returns zero elements.)
5. If the seed length is under 16 or over 64, return kind `Encoding` wrapping `hdkeychain.ErrInvalidSeedLen`.

`pgpwordlist.DecodeMnemonics(words)` (`pgpwordlist/pgpwordlist.go:36-60`):

1. For each element, trim surrounding whitespace; skip it if empty (so runs of spaces are tolerated).
2. Look up the lower-cased word; if absent, return kind `Encoding` with `word <w> is not in the PGP word list` (`<w>` is the trimmed word as typed).
3. If the table index's parity differs from the parity of the count of words accepted so far, return kind `Encoding` with `word <w> is not valid at position <idx>, check for missing words`.
4. Append `index / 2`.

`walletseed.GenerateRandomSeed(size uint)` (`walletseed/seed.go:21-34`): `size >= 255` → kind `Invalid` wrapping `ErrInvalidSeedLen`; `size < 16` or `> 64` → the same; otherwise `hdkeychain.GenerateSeed(uint8(size))`.

`walletseed.EncodeMnemonicSlice(seed)` (`walletseed/seed.go:44-52`) and `EncodeMnemonic(seed)` (`:55-67`), exactly:

1. For each byte `seed[i]`, the word is `ByteToMnemonic(seed[i], i)`.
2. The checksum word is `ByteToMnemonic(checksumByte(seed), len(seed))`.
3. `EncodeMnemonicSlice` returns a slice of `len(seed) + 1` strings. `EncodeMnemonic` writes a space before every seed word except the first, then a space, then the checksum word. Neither validates the seed length; for an empty seed `EncodeMnemonicSlice` returns one word and `EncodeMnemonic` returns that word preceded by a space (§10 item 31).

`pgpwordlist.ByteToMnemonic(b, index)` (`pgpwordlist/pgpwordlist.go:26-32`): table position `uint16(b) * 2`, plus one when `index % 2 != 0`. Go's `%` keeps the sign of the dividend, so a negative odd `index` also selects the odd-position word; no caller passes a negative index.

How the two dcrwallet entry points reach `DecodeUserInput`:

- **gRPC `SeedService.DecodeSeed`** passes `req.UserInput` unmodified (`internal/rpc/rpcserver/server.go:3199-3205`), so every branch above, including the single-token hex branch, is reachable from a client.
- **The interactive creation prompt** (`internal/prompt/prompt.go:318-360`) reads lines until an empty one. If the first line alone decodes as hex it is taken as the whole input; otherwise lines are joined with a space. The result is trimmed, every run of Unicode white space is collapsed to one space (`collapseSpace`, `:465`), and words are counted as spaces plus one. For one word the prompt decodes the hex itself, first prefixing `0` when the length is odd, and does not call `DecodeUserInput`; only input of two or more words goes to `DecodeUserInput`. Tabs and newlines between words are therefore accepted at the prompt (they were collapsed first) although `DecodeUserInput` itself splits on the space character only, and an odd-length hex seed is accepted at the prompt and rejected over gRPC. The prompt's texts belong to the CLI spec.

Worked example (from `walletseed/seed_test.go:38-42`): 31 zero bytes encode as fifteen repetitions of `aardvark adroitness`, then `aardvark` (byte 30, even position), then the checksum word. `SHA256(SHA256(31 zero bytes))[0] = 0x7e`; position 31 is odd; table index `2*0x7e + 1 = 253` = `insurgent`.

### 5.5 dcr-rs algorithms the wallet does not take

- `sign_p2pkh_input_cached` (`dcr-rs:src/sign.rs:52-75`): binds the signing key to the prevout script before signing. dcrwallet signs through `txscript/v4/sign` with key and script lookup closures for every script class (spec 10); `dcroxide-txscript` ports that package (`dcroxide:crates/dcroxide-txscript/src/sign.rs:62-78`, `:119`, `:184`, `:606`).
- `SignRequest::validate` and the airgap review and signing flow (`dcr-rs:src/airgap.rs:513-649`, `:736-898`): companion-device policy with no dcrwallet equivalent.
- `MsgTx::parse_full` strictness (`dcr-rs:src/tx.rs:321-395`): rejecting trailing bytes is correct for a signer and wrong for a port of `wire.MsgTx.Deserialize`, where framing is the caller's job (`dcroxide:docs/adr/0003-slice-based-wire-decoding.md:16-18`).

### 5.6 Convention processes inherited from dcroxide

**Engineering principles** (`dcroxide:docs/dcroxide-project-brief.md:525-535`), restated for the wallet:

1. The upstream Go at the pinned commit is the specification; written documentation is secondary; every quirk reproduced on purpose gets a `QUIRKS.md` entry and a pinning test.
2. Oracle-driven development: each module ships with vectors extracted from upstream's own tests or generated by Go shims against upstream packages, and, where feasible, a differential fuzz target that compares against upstream live.
3. No hand-rolled cryptography for standard algorithms; the project owns only Decred-specific constructions, plus the exceptions individual ADRs record (the TLS provider's glue and RSA verification, BLAKE3 once ported, variable-time secp256k1 on public values).
4. `unsafe` is forbidden in every project crate, with the one audited exception of `dcroxide-winsvc`.
5. Critical code is boring code: explicit integer widths, checked arithmetic that mirrors upstream's, no floating point near consensus, deterministic iteration order where order is observable, and error enums mapped one to one onto upstream's error kinds.
6. Denial-of-service limits are ported verbatim and tested.
7. Pin, then track: every parity claim names one pinned upstream commit, and `PARITY.md` maps each upstream package to its crate and status.

**Definition of done for a task** (`dcroxide:docs/dcroxide-project-brief.md:928-940`): (1) the implementation matches the pin with every quirk documented; (2) upstream's corresponding tests are ported or regenerated and pass; (3) new parsers and state machines have fuzz and property targets wired into CI; (4) critical code has differential coverage against upstream, has been through an adversarial automated review whose findings are fixed or recorded, and lands in a commit the project owner signs; (5) `PARITY.md` and, where applicable, `QUIRKS.md` and the ADRs are updated; (6) public items are documented; (7) hot paths have benchmarks with recorded thresholds.

**Oracle rig** (`dcroxide:docs/adr/0001-oracle-driven-differential-testing.md:26-41`):

1. A standalone Go module under `tools/oracle` builds one binary that links the real upstream packages.
2. It reads one JSON object per line on stdin, `{"cmd": …}`, and writes one per line on stdout, `{"result": …}` or `{"error": …}`. New capabilities are new `cmd` values.
3. Every upstream module in its `go.mod` is pinned to the version the parity target's `go.mod` requires; versions move only when the pin moves.
4. Rust tests build it on demand with `go build` into the cargo target directory and spawn it. Without a Go toolchain the differential tests skip with a notice; with `<PROJECT>_REQUIRE_ORACLE=1` (set in CI) a missing toolchain is a failure. A prebuilt oracle can be named through an environment variable (`dcroxide:README.md`, Development).
5. A consistency test fails when the harness pin, CI's upstream commit and the commit the Go tool modules name disagree (`0001-…md:83-99`).
6. One reference Go release, the one upstream's release image builds the pin with, to the patch; CI's `setup-go` steps take it from one variable (`0001-…md:259-…`, `dcroxide:.github/workflows/ci.yml:36-42`).

**Frozen batteries** (`0001-…md:158-200`): vectors dumped from inside upstream's own packages are regenerated, never re-expected by hand, when a pin move changes the package they came from or the reference Go changes texts they pin. The exporter is a kept tool under `tools/`, and before it is run at a new pin it must reproduce the frozen file byte for byte at the commit the file was dumped at.

**Pin moves** (`dcroxide:docs/adr/0013-upstream-tracking-release-tags.md:121-173`): the move is the unit of mirroring. Each starts from a tool-generated review list of the upstream delta, and gets one `PARITY.md` section titled by its range that ports or accounts for every upstream commit. Once upstream tags the target release, the pin follows release tags of the newest line; master is read ahead into dated notes that are not parity claims. At a tag, "upstream at the pin" is the root module at the tag plus each nested module at the version the tag's `go.mod` requires, and a citation into a nested module names the module version.

**CI jobs** (`dcroxide:.github/workflows/ci.yml:45-854`), in the form the wallet copies:

| Job | Command or purpose |
| --- | --- |
| `lint` | `cargo fmt --all --check`; `cargo clippy --workspace --all-targets -- -D warnings`; `cargo metadata --locked` on the fuzz workspace |
| `lint-latest` | clippy on latest stable, `continue-on-error` |
| `test` | `cargo test --workspace` on `ubuntu-latest`, `ubuntu-24.04-arm`, `macos-latest`, `windows-latest`, with the reference Go installed |
| `test-wrapping` | the suite again with `CARGO_PROFILE_DEV_OVERFLOW_CHECKS=false`, as the release profile computes |
| `dcrd-interop` | builds upstream at the pinned commit with `-buildvcs=true` and runs a live-socket interop test, single-threaded |
| `go-tools` | `go vet ./... && go test ./...` in each Go tool module |
| `deny` | `cargo-deny` (advisories, licences, bans, sources) |
| `docs` | `cargo doc --workspace --no-deps` with `RUSTDOCFLAGS=-D warnings` |
| `typos` | `crate-ci/typos` with `_typos.toml` |
| `no-std` | `cargo check --no-default-features --target x86_64-unknown-none` for each `no_std` crate |
| `no-c-compiler` | `cargo build --workspace --all-targets --locked --keep-going` under `env -i` with every C/C++ driver hidden from build scripts (`dcroxide:docs/adr/0012-pure-rust.md:492-…`) |
| `fuzz-smoke`, `nightly-fuzz` | 60 s and 10 min per `cargo-fuzz` target |
| `nightly-sigsoak`, `nightly-key-sweep`, `test-vartime-release`, `tls-soft-backends` | node-specific soaks; the wallet defines its own equivalents |
| `constant-time` (separate workflow) | dudect-style fixed-against-random timing test of the TLS provider's signing, key exchange, AEAD and HMAC paths (`dcroxide:.github/workflows/constant-time.yml:1-25`) |

**Arithmetic policy** (`dcroxide:docs/adr/0008-clippy-lint-policy.md:6-80`, `:260-…`): no crate-wide `arithmetic_side_effects` allow. Where the Go wraps, the port calls `wrapping_*` at the width the Go computes in; elsewhere the operator stays under `#[allow(clippy::arithmetic_side_effects, reason = "…")]` on the smallest item, the reason stating the bound. Signed `/` and `%` keep the operator only when the reason rules out `MIN / -1`; shifts are checked by hand because the lint does not cover them; `as` between integers is the correct transcription of a Go conversion; a float-to-int `as` saturates where Go on amd64 yields `MIN` and must be audited site by site (`:205-216`).

**Dependency admission** (`dcroxide:docs/dependency-ledger.md:167-172`, `dcroxide:docs/adr/0012-pure-rust.md:303-368`): a dependency that becomes observable in consensus, key handling or the network surface gets a ledger row with a decision; a crate that brings `asm!` into a build for a supported target needs its row in the assembly table before it lands; `deny.toml` bans the C build drivers by name and denies duplicate versions, with every skip justified in a comment.

**Release** (`dcroxide:docs/adr/0014-release-engineering.md:117-266`): MSRV equals the pinned toolchain and both move in one commit; the pin takes a point release rather than a `.0` (a `.0` becomes eligible only once the next minor ships without a point release for it); no binary is published before the pure-Rust rule is met, the differential soak has passed, the upstream pin is a release tag and a reproduction prototype has passed; a release is a hand-made signed tag, a `SHA256SUMS` signed by the project owner after an off-CI rebuild matches a CI double build, and artifact attestations.

**Test replay and skip rules** (`dcroxide:README.md:800-846`; `dcroxide:.github/workflows/ci.yml:22-42`, `:246-250`, `:588-596`):

| Variable (dcroxide's name) | Effect |
| --- | --- |
| `DCROXIDE_REQUIRE_ORACLE=1` | A missing Go toolchain or oracle fails the differential tests instead of skipping them. Set in CI. |
| `DCROXIDE_ORACLE_BIN` | Absolute path of a prebuilt oracle; every test process runs it instead of building one. Taken as found: a binary built before a change to the oracle source answers for the old source. A value that is not an absolute path to a file skips the oracle tests, or fails them under `REQUIRE_ORACLE`. |
| `DCROXIDE_TEST_SEED` | Replays a randomized differential with the seed a failing run printed (`<label>: seed 0x…`). |
| `DCROXIDE_REQUIRE_FAULT_INJECTION=1` | The Linux-only fault-injection tests (full disk, power cut) fail instead of skipping when a prerequisite is missing. Set in CI. |
| `DCROXIDE_REQUIRE_DCRD=1`, `DCRD_COMMIT` (`ci.yml:242-248`) | The live-socket interop job fails without the upstream daemon binary; the commit is the same one the test harness names, and a unit test fails when the two differ. |
| `GO_VERSION` | The one reference Go release, used by every `setup-go` step. |
| `RUSTUP_TOOLCHAIN=nightly` | Set only in the fuzz jobs, because the repository's toolchain file would otherwise win inside `fuzz/`. |

CI runs on every push to the default branch, on every pull request, nightly at 03:00 UTC (long fuzzing, the signature soak, the key sweep) and on manual dispatch (`ci.yml:3-20`). Jobs install the Rust toolchain with `rustup toolchain install`, reading `rust-toolchain.toml`, instead of a toolchain action that would name the version a second time (`ci.yml:50-53`). The lint job also resolves the fuzz workspace with `--locked`, and a unit test holds the fuzz lock's package versions to the main lock's (`ci.yml:57-62`). The `test-wrapping` job runs the whole suite with overflow checks off and then re-runs the script-engine and block-validation crates with debug assertions off as well (`ci.yml:105-135`). The arm64 Linux leg exists because platform-dependent arithmetic has already produced an upstream discrepancy (`ci.yml:86-92`).

### 5.7 dcr-rs algorithms recorded for completeness

None of these is taken by the wallet (§5.5, §12.2). They are listed so the inventory of `dcr-rs:src/` has no silent omission.

`MsgTx::parse_full(buf)` (`dcr-rs:src/tx.rs:321-395`):

1. Read the version word; low 16 bits are the version, high 16 bits the serialization type, which must be 0.
2. Read the input count with a minimum item size of 41 bytes; read each input prefix (32-byte hash, index, tree, sequence).
3. Read the output count with a minimum item size of 11; read each output (value, script version, length-prefixed script).
4. Read lock time and expiry.
5. Read the witness count with a minimum item size of 17; it must equal the input count.
6. Read each witness (value in, block height, block index, length-prefixed signature script).
7. The cursor must be at the end of the buffer.

`parse_prefix` (`:403-458`) is steps 1–4 and 7 with serialization type 1; inputs get `value_in = -1`, block height 0, block index `0xffffffff` and an empty signature script. Pre-allocation is capped at 1,024 elements (`PREALLOC_LIMIT`). Every failure is `Error::Parse`.

`sign_p2pkh_input_cached` (`dcr-rs:src/sign.rs:52-75`): derive the compressed public key from the secret; require `prevout_script == p2pkh_script(hash160(pubkey))`, else `ScriptMismatch`; compute the SigHashAll hash (`SigHashIndex` when the index is out of range); sign with libsecp256k1's deterministic ECDSA; normalize S; build the signature script.

`SignRequest::validate` (`dcr-rs:src/airgap.rs:513-649`), in order, each failure being `InvalidRequest` with the message of §8.6 unless stated: format version (`UnsupportedVersion`); non-empty inputs and outputs; at most 1,000 of each; `tx_version == 1`; account below `2^31`; per input: positive amount, amount at most `MAX_ATOMS`, branch 0 or 1, index below `2^31`, tree 0, funding prefix present; per output: non-negative amount, at most `MAX_ATOMS`, script version 0, branch and index both present or both absent, `is_change` equal to path presence, branch 0 or 1, index below `2^31`; input total (128-bit) at most `MAX_ATOMS`; input total not below output total; fee either at most 100,000 atoms or at most one twentieth of the input total; no outpoint (hash, index, tree) listed twice; then `verify_prev_txs` (`:466-507`): each funding prefix parses, its BLAKE-256 equals the declared previous hash, the output at the declared index exists and carries the declared amount and script.

`sign_request` (`:812-898`): `validate`; derive `m/44'/slip44'/account'` with the Decred variant; `check_account_fp` (`AccountMismatch` when the optional fingerprint differs); prove every change output's script by derivation (`ScriptMismatch`); build the transaction with `block_height = 0`, `block_index = 0xffffffff` and empty signature scripts; compute the prefix hash once; for each input re-check branch, index and tree, derive `branch` then `index` (each branch key derived once), sign, and store the signature script; return the full serialization. `review_owned` (`:736-787`) and `check_owned_inputs` (`:790-799`) run `validate` and the same ownership proofs without signing.

## 6. Constants, limits and defaults

### 6.1 Network identifiers carried by dcr-rs

Checked against `dcrd/chaincfg/v3@v3.3.0` (`mainnetparams.go:534-547`, `testnetparams.go:508-521`, `simnetparams.go:543-556`, `regnetparams.go:530-543`); every dcr-rs value matches. dcr-rs does not carry the legacy coin types.

| Network | dcr-rs name | P2PKH ID | P2SH ID | HD private | HD public | SLIP-0044 | Legacy coin type (chaincfg only) |
| --- | --- | --- | --- | --- | --- | ---: | ---: |
| mainnet | `mainnet` | `07 3f` (`Ds`) | `07 1a` (`Dc`) | `02 fd a4 e8` (`dprv`) | `02 fd a9 26` (`dpub`) | 42 | 20 |
| testnet3 | `testnet3` | `0f 21` (`Ts`) | `0e fc` (`Tc`) | `04 35 83 97` (`tprv`) | `04 35 87 d1` (`tpub`) | 1 | 11 |
| simnet | `simnet` | `0e 91` (`Ss`) | `0e 6c` (`Sc`) | `04 20 b9 03` (`sprv`) | `04 20 bd 3d` (`spub`) | 1 | 115 |
| regnet | `regnet` | `0e 00` (`Rs`) | `0d db` (`Rc`) | `ea b4 04 48` (`rprv`) | `ea b4 f9 87` (`rpub`) | 1 | 1 |

Source in dcr-rs: `dcr-rs:src/network.rs:32-88`.

### 6.2 Extended keys and seeds

| Name | Value | Source | What depends on it |
| --- | --- | --- | --- |
| `RecommendedSeedLen` | 32 bytes | `dcrd/hdkeychain/v3@v3.1.3/extendedkey.go:30` | default seed length for wallet creation and `GenerateRandomSeed` |
| `MinSeedBytes` / `MaxSeedBytes` | 16 / 64 | `:40`, `:44`; `dcr-rs:src/hd.rs:91-92` | seed validation in `NewMaster`, `GenerateSeed`, `DecodeUserInput` |
| `HardenedKeyStart` | `0x80000000` | `:36`; `dcr-rs:src/hd.rs:95` | hardened index test |
| `serializedKeyLen` | 78 | `:50` | payload length before the checksum |
| decoded length | 82 | `:552` | `NewKeyFromString` length check |
| `maxKeyLen` | 113 characters | `:553` | pre-decode length gate in dcrd |
| `MAX_BASE58_LEN` | 128 characters | `dcr-rs:src/hashing.rs:45` | pre-decode length gate in dcr-rs (addresses and keys) |
| master HMAC key | ASCII `Bitcoin seed` (12 bytes) | `:99`; `dcr-rs:src/hd.rs:206` | master key derivation |
| checksum length | 4 bytes | `:207-211` | extended key and address checksums |
| seed-size guard | `size >= 255` rejected before the range check | `walletseed/seed.go:23` | `GenerateRandomSeed` (`uint` to `uint8` narrowing) |
| PGP word table | 512 words, all ASCII letters, 50 of them capitalized, all distinct after lower-casing; the constant has no trailing newline, so the split yields exactly 512 entries | `pgpwordlist/wordlist.go:23`, `:33-544` | mnemonic encode/decode |
| `wordIndexes` value type | `uint16` (table index 0..511) | `pgpwordlist/wordlist.go:25` | parity test `b % 2`, byte value `b / 2` |
| default mnemonic length | 33 words (32 seed bytes + checksum) | `internal/prompt/prompt.go:279` | prompt output, six words per line |
| `ErrInvalidSeedLen` text | built with `fmt.Errorf` from `MinSeedBytes*8` and `MaxSeedBytes*8`: `seed length must be between 128 and 512 bits` | `dcrd/hdkeychain/v3@v3.1.3/extendedkey.go:79-80` | every seed-length error text |
| HMAC data length | 37 bytes (`keyLen = 33` plus 4) | `:241-242` | child derivation preimage |
| nested-error separator in the daemon | `:: ` (the `errors` package default is `:\n\t`) | `dcrwallet.go:38`, `errors/errors.go:27` | formatted seed-decode errors (§8.2) |
| mnemonic checksum | 1 byte, `SHA256(SHA256(seed))[0]`, one extra word | `walletseed/seed.go:38-41` | mnemonic encode/decode |

### 6.3 dcr-rs policy constants (not dcrwallet behaviour)

| Name | Value | Source |
| --- | --- | --- |
| `FORMAT_VERSION` | 3 | `dcr-rs:src/airgap.rs:102` |
| `FEE_ALWAYS_ALLOWED_ATOMS` | 100,000 | `:107` |
| `MAX_FEE_FRACTION_DIVISOR` | 20 | `:117` |
| `MAX_PACKAGE_BYTES` | 524,288 | `:284` |
| `MAX_ATOMS` | 2,100,000,000,000,000 | `:377` |
| `MAX_INPUTS`, `MAX_OUTPUTS` | 1,000 each | `:382`, `:384` |
| `PREALLOC_LIMIT` | 1,024 elements | `dcr-rs:src/tx.rs:49` |
| minimum item sizes for count checks | prefix input 41 bytes, output 11 bytes, witness input 17 bytes | `dcr-rs:src/tx.rs:330-331`, `:342`, `:359` |
| accepted `tx_version` in an airgap request | 1 only | `dcr-rs:src/airgap.rs:533-535` |
| BIP39 entropy accepted by `from_entropy` | 16–32 bytes (the `bip39` crate's rule) | `dcr-rs:src/hd.rs:234-239` |
| secret erasure fill | `0x01` bytes for the secp256k1 secret, zeros for the chain code | `dcr-rs:src/hd.rs:191-197`, `:794-799` |
| CI toolchains | tests on stable; MSRV job on 1.85 with `--locked`; `rustfmt`, `clippy` and `cargo doc` on 1.97.1; `no_std` check for `thumbv7em-none-eabihf` | `dcr-rs:.github/workflows/ci.yml:12-119` |
| crate version, edition, MSRV | 0.6.3, 2021, 1.85 | `dcr-rs:Cargo.toml:10-23` |

### 6.4 dcroxide workspace settings to copy

| Setting | Value | Source | What depends on it |
| --- | --- | --- | --- |
| `resolver` | `"3"` (MSRV-aware) | `dcroxide:Cargo.toml:2` | dependency versions chosen for the pinned compiler |
| `members` / `exclude` | 35 crates under `crates/`; `exclude = ["fuzz"]` | `:3-40` | the fuzz workspace keeps its own lock and its libFuzzer C++ outside `cargo-deny`'s view of the main workspace |
| `edition` | `2024` | `:45` | |
| `rust-version` | `1.99.1`, equal to the toolchain pin | `:51`; `dcroxide:rust-toolchain.toml:18` | MSRV policy (ADR-0014 part 1) |
| toolchain components | `rustfmt`, `clippy` | `dcroxide:rust-toolchain.toml:19` | lint job installs from the file |
| `license` | `ISC` | `dcroxide:Cargo.toml:52` | matches upstream |
| `[profile.release]` | `codegen-units = 1`, `lto = "thin"`, `debug = "line-tables-only"`, `panic = "abort"`, `overflow-checks = false` | `:57-94` | abort on panic (ADR-0005); wrapping arithmetic as Go's |
| `[profile.dist]` | inherits `release`, `strip = "debuginfo"` | `:113-115` | release artifacts |
| `[profile.soak]`, `soak-checked`, `soak-asserts` | `soak` inherits `release` with `panic = "unwind"`; `soak-checked` adds `overflow-checks = true`; `soak-asserts` adds `debug-assertions = true` and `overflow-checks = true` | `:125-144` | differential soak binaries that must survive and report a panic, and the checked-arithmetic soak legs |
| `[workspace.package] version` | `0.1.0` (not the upstream version the binary reports) | `:43` | release version identity is an open item of ADR-0014 part 6 |
| `[workspace.lints.rust]` | `unsafe_code = "forbid"`, `missing_docs = "deny"` | `:146-148` | |
| `[workspace.lints.clippy]` | `all = warn` (priority −1), `arithmetic_side_effects = "warn"`; CI passes `-D warnings` | `:150-152` | |
| `clippy.toml` `disallowed-methods` | the six rustls builders that take the process-default provider, `CryptoProvider::install_default`, and thirteen `ureq` entry points that build its default agent | `dcroxide:clippy.toml:16-37` | no process-wide TLS provider (ADR-0015 T7) |
| `deny.toml` `[graph]` | `all-features = true` | `dcroxide:deny.toml:1-2` | |
| `deny.toml` `[advisories]` | `yanked = "deny"` | `:4-5` | |
| `deny.toml` `[licenses] allow` | `ISC`, `MIT`, `Apache-2.0`, `BSD-3-Clause`, `Unicode-3.0`, `CC0-1.0`, `CDLA-Permissive-2.0` | `:7-21` | |
| `deny.toml` `[bans]` | `multiple-versions = "deny"`; `deny` = `ring`, `aws-lc-rs`, `aws-lc-sys`, `secp256k1-sys`, `cc` (wrappers `blake3`, `iana-time-zone-haiku`), `cmake`, `pkg-config`, `vcpkg`, `bindgen`; `blake3` features held to exactly `pure`; one skip, `syn:2` | `:39`, `:77-102` | pure-Rust rule (ADR-0012) |
| `deny.toml` `[sources]` | `unknown-registry = "deny"`, `unknown-git = "deny"` | `:104-106` | no git or foreign-registry dependencies |
| `_typos.toml` | excludes `crates/*/tests/data` and `crates/*/testdata`; ignores base58 runs of 25+ characters; a word list of upstream's own misspellings reproduced for parity | `dcroxide:_typos.toml` | spell check that cannot "fix" parity strings |
| CI environment | `<PROJECT>_REQUIRE_ORACLE=1`, `<PROJECT>_REQUIRE_FAULT_INJECTION=1`, `GO_VERSION` | `dcroxide:.github/workflows/ci.yml:22-42` | differential and fault-injection tests cannot silently skip |

Consequences of the `deny.toml` values for dcr-rs as a dependency: `secp256k1-sys` is banned by name and `cc` is banned except under two named wrappers, so dcr-rs 0.6.3 (`secp256k1 = "0.29"`, `dcr-rs:Cargo.toml:53`; `secp256k1-sys` 0.10.1 and `cc` in `dcr-rs:Cargo.lock`) fails the `deny` job and the `no-c-compiler` job.

## 7. Concurrency and lifecycle

### 7.1 dcr-rs

dcr-rs has no threads, channels, locks, global state or I/O. Every function is a pure computation over its arguments. Callers supply a `secp256k1::Secp256k1<All>` context to every curve operation (`dcr-rs:src/hd.rs:258-446`); the context is created by the caller and can be shared by reference. Secret lifetimes are managed by `Drop` (`ExtPrivKey`, `Blake256`) and explicit `zeroize` calls on intermediates; the crate documents that copies made by by-value calls are not reached (`dcr-rs:src/hd.rs:178-190`, `dcr-rs:SECURITY.md:39-53`).

dcrd's `ExtendedKey` memoizes the public key of a private key on first use without synchronization (`dcrd/hdkeychain/v3@v3.1.3/extendedkey.go:157-171`); concurrent first use from two goroutines is a benign duplicate computation of the same bytes in practice but an unsynchronized write in Go's memory model. A Rust port computes the public key eagerly or behind `OnceLock`; nothing observable depends on the choice.

Neither `hdkeychain`, `walletseed` nor `pgpwordlist` starts a goroutine, takes a lock, uses a channel or accepts a context. Package-level state is `hdkeychain.masterKey` (never written), `pgpwordlist.wordList` (initialized at package load) and `pgpwordlist.wordIndexes` (filled by `init()` before any caller can run, read-only afterwards), so every function is safe for concurrent use except for the `ExtendedKey` memoization and `Zero` described here. `GenerateSeed` reads `crypto/rand` (`dcrd/hdkeychain/v3@v3.1.3/extendedkey.go:15`, `:630`), not `dcrd/crypto/rand`.

`Zero()` (`:488-497`) mutates the receiver and, through shared slices, any key obtained from it by `Neuter` (§10 item 25); a key used after `Zero` serializes as `zeroed extended key` and derives from an empty key. dcrwallet calls `Zero` on derived keys after use throughout the address manager and wallet. In Rust this is `Drop` plus `Zeroize`; a use-after-zero is not expressible and needs no port.

### 7.2 dcroxide's model

- No async runtime. Neither `tokio` nor `rayon` is in any manifest and no crate has an `async fn`; every I/O surface (listeners, per-peer read and write loops, dialer, RPC and websocket handlers, seeders, IPC) is an OS thread over `std::sync::mpsc` (`dcroxide:docs/adr/0005-concurrency-model.md:53-60`).
- Core crates are synchronous and runtime-free; shared state sits behind a single writer (`:77-78`).
- The reason recorded for keeping threads: they mirror upstream's goroutine-per-concern layout thread for goroutine, which keeps the line-by-line parity audit tractable (`:131-135`). The same addendum leaves multi-client RPC/websocket concurrency as an open question to be measured before wallet integrations.
- Rust mutexes poison on panic; Go's do not. The release profile therefore aborts on panic and the daemon must run under a supervisor (`:84-92`; `dcroxide:Cargo.toml:57-82`; `dcroxide:SECURITY.md`, Known gaps).
- Notification queues that are unbounded upstream stay unbounded (`:78-82`).
- A thread the OS refuses to start must not abort the process where a goroutine could not have failed (`:70-75`).

### 7.3 What changes for the wallet

The wallet has three I/O surfaces dcroxide does not: an HTTP/2 gRPC server with long-lived server streams and one bidirectional stream; a websocket JSON-RPC *client* to dcrd; and the mixing client, which runs timed protocol rounds against peers. Whether these fit the thread model is the decision D-1 in §13. The constraint from §4.9 is that HTTP/2 multiplexes many concurrent streams on one connection with per-stream and per-connection flow control, so a thread-per-connection reader must hand frames to per-stream handler threads and serialize writes through one writer; grpc-go itself is structured that way (a reader goroutine, a `controlBuf`-fed writer goroutine, a goroutine per stream).

Shutdown ordering, signal handling and the lifetime of sync and mixing loops are specified in the wallet-core, sync and mixing specs; this document adds only that dcroxide's `panic = "abort"` policy and supervisor requirement carry over, and that for a wallet an abort mid-write makes the storage engine's crash consistency (spec 01) a correctness dependency rather than a performance one.

## 8. Errors

### 8.1 dcr-rs `Error` against dcrd `hdkeychain` errors

dcr-rs has one `Copy` enum for the whole crate (`dcr-rs:src/lib.rs:65-103`). It does not preserve dcrd's error identities, and dcrwallet depends on three of them through `errors.Is`.

| Situation | dcrd error | dcr-rs error |
| --- | --- | --- |
| seed shorter than 16 or longer than 64 bytes | `ErrInvalidSeedLen` | `Derivation` |
| master secret zero or `>= N` | `ErrUnusableSeed` | `Derivation` |
| child `Il` zero or `>= N` | `ErrInvalidChild` | `Derivation` |
| private child sum zero | none (a zero or empty key is returned) | `Derivation` |
| public child sum at infinity | none | `Derivation` |
| hardened child of a public key | `ErrDeriveHardFromPublic` | `HardenedFromPublic` |
| depth past 255 | none (serialized depth wraps mod 256) | `Derivation` |
| key string longer than the gate | `ErrInvalidKeyLen` (over 113) | `Parse` (over 128) |
| invalid base58 character | `ErrInvalidKeyLen` (decode returns empty) | `Base58` |
| decoded length not 82 | `ErrInvalidKeyLen` | `Parse` |
| checksum mismatch | `ErrBadChecksum` | `BadChecksum` |
| version not of the expected network | `ErrWrongNetwork` | `UnknownPrefix` only when the version is of none of the four networks; a key of another known network parses successfully |
| private key data zero or `>= N` | `ErrUnusableSeed` | `Parse` |
| public key data not a valid point | the `secp256k1.ParsePubKey` error, unwrapped (for example `ErrPubKeyNotOnCurve`) | `Parse` |
| private data under a public version, or the reverse | accepted (type from the data) | `Parse` or `UnknownPrefix` |
| private key requested from a public key | `ErrNotPrivExtKey` | not expressible (separate types) |

How dcrwallet consumes the three identities it tests for:

- `ErrInvalidChild` — address derivation loops skip the index and continue (`wallet/addresses.go:612`, `:1008`; `wallet/udb/addressmanager.go:2030`; `wallet/udb/upgrades.go:335`, `:358`; `internal/rpc/jsonrpc/methods.go:1020`; `wallet/wallet.go:1024`); wallet creation converts it to kind `Seed` wrapping `ErrUnusableSeed` (`wallet/udb/addressmanager.go:2844-2876`, `:3107-3120`).
- `ErrInvalidSeedLen` — the gRPC error translation maps any error wrapping it to code `InvalidArgument` (`internal/rpc/rpcserver/server.go:120-122`); `walletseed` wraps it in kinds `Invalid` and `Encoding`; `wallet/wallet.go:5305` returns it.
- `ErrUnusableSeed` — produced by `NewMaster` and by the wrapping above.

The error *strings* of §3.9 reach users through log lines, JSON-RPC error messages and gRPC `grpc-message` values wherever these errors are formatted with `%v`. The Rust `hdkeychain` port therefore has one error enum with a variant per dcrd sentinel and `Display` output equal to the Go strings.

### 8.2 `walletseed` and `pgpwordlist`

All errors are dcrwallet `errors.E` values (the `errors` package is specified with the wallet core): an operation name, a kind and a wrapped error.

| Operation | Kind | Wrapped error or message | Source |
| --- | --- | --- | --- |
| `walletseed.GenerateRandomSeed` | `Invalid` | `hdkeychain.ErrInvalidSeedLen` | `walletseed/seed.go:23-28` |
| `walletseed.GenerateRandomSeed` | none | the random source's error | `:30-32` |
| `walletseed.DecodeUserInput` | `Encoding` | Go `encoding/hex` error (`encoding/hex: invalid byte: U+…` or `encoding/hex: odd length hex string`) | `:79-82` |
| `walletseed.DecodeUserInput` | `Encoding` | the `pgpwordlist.DecodeMnemonics` error (itself an `errors.E` with its own operation and kind `Encoding`) | `:85-88` |
| `walletseed.DecodeUserInput` | `Encoding` | `checksum mismatch` | `:92-94` |
| `walletseed.DecodeUserInput` | `Encoding` | `hdkeychain.ErrInvalidSeedLen` | `:98-100` |
| `pgpwordlist.DecodeMnemonics` | `Encoding` | `word <w> is not in the PGP word list` | `pgpwordlist/pgpwordlist.go:47-50` |
| `pgpwordlist.DecodeMnemonics` | `Encoding` | `word <w> is not valid at position <idx>, check for missing words` | `:51-55` |

The Go hex error texts are standard-library strings and follow the reference Go release (§5.6).

Formatted text. `(*Error).Error()` writes the operation, then `: ` and the kind's text when a kind is set, then, when the wrapped error is itself an `*Error`, the package variable `Separator` followed by that error's fields (an operation or kind equal to the one just written is not repeated), and otherwise `: ` and the wrapped error's text (`errors/errors.go:247-290`). The kind texts are `invalid encoding` for `Encoding` and `invalid operation` for `Invalid` (`errors/errors.go:83-94`). The daemon sets `errors.Separator = ":: "` at start-up (`dcrwallet.go:38`); a program that links the packages without doing so gets the default `":\n\t"` (`errors/errors.go:27`). `errors.E` does not collapse the `pgpwordlist` error into the `walletseed` one, because their operations differ (`errors/errors.go:213-233`). The strings the daemon therefore produces, and that gRPC `DecodeSeed` returns as the status message with code `InvalidArgument`:

| Case | Exact string |
| --- | --- |
| bad hex | `walletseed.DecodeUserInput: invalid encoding: ` + Go's hex error text |
| unknown word | `walletseed.DecodeUserInput: invalid encoding:: pgpwordlist.DecodeMnemonics: word <w> is not in the PGP word list` |
| wrong position | `walletseed.DecodeUserInput: invalid encoding:: pgpwordlist.DecodeMnemonics: word <w> is not valid at position <idx>, check for missing words` |
| checksum | `walletseed.DecodeUserInput: invalid encoding: checksum mismatch` |
| length | `walletseed.DecodeUserInput: invalid encoding: seed length must be between 128 and 512 bits` |
| `GenerateRandomSeed` size (no production caller) | `walletseed.GenerateRandomSeed: invalid operation: seed length must be between 128 and 512 bits` |

`<w>` is formatted with `%v` from the trimmed word as typed (case preserved) and `<idx>` with `%v` from the zero-based count of words accepted so far. These strings were derived by reading the formatter, not by running it; the oracle command `seed_decode` (§11.4) pins them.

### 8.3 gRPC transport-level statuses

Statuses the transport produces without reaching a handler are in §4.9 items 2, 6 and 7. The status-code mapping of wallet errors is owned by the gRPC server spec. Status codes travel as decimal integers in `grpc-status`; the HTTP status is always 200 for a gRPC response.

### 8.4 dcroxide's error conventions

- Error enums map one to one onto upstream's error kinds, and every error text a client can see renders upstream's text exactly (`dcroxide:docs/adr/0003-slice-based-wire-decoding.md:25-39`; project brief principle 5).
- Go's `io.EOF` and `io.ErrUnexpectedEOF` are distinct variants because clients see their text (`:19-23`, `:76-79`).
- Texts that come from Go's standard library (JSON syntax errors, hex errors, `%q` quoting, float formatting, TLS and HTTP errors) follow one named Go release (`dcroxide:docs/adr/0001-oracle-driven-differential-testing.md:259-…`).

### 8.5 dcr-rs `Error`: every variant and its `Display` text

`dcr-rs:src/lib.rs:63-124`. The enum derives `Debug`, `Clone`, `Copy`, `PartialEq`, `Eq` and is `#[non_exhaustive]`; `std::error::Error` is implemented under feature `std`. None of these texts is a dcrwallet text.

| Variant | `Display` | Raised by |
| --- | --- | --- |
| `Derivation` | `key derivation failed` | seed length, invalid master or child scalar, zero or infinite child sum, depth past 255, BIP39 parse or entropy failure |
| `HardenedFromPublic` | `cannot derive a hardened child from a public key` | `ExtPubKey::derive_child` |
| `HardenedIndex` | `hardened index where a non-hardened one is required` | `account_key`, `address_key` |
| `AccountMismatch` | `package was built for a different account key` | `SignRequest::check_account_fp` |
| `Parse` | `could not parse data` | transaction parser, base58check length rules, extended-key data rules, CBOR decode |
| `Base58` | `invalid base58 string` | `check_decode`, `parse_ext_key` |
| `BadChecksum` | `base58check checksum mismatch` | `check_decode`, `parse_ext_key` |
| `UnknownPrefix` | `version prefix matches no known network` | `Address::decode`, `decode_for`, `from_base58` |
| `Encode` | `could not encode data` | `encode_sign_request` |
| `UnsupportedVersion` | `unsupported package version` | `decode_sign_request`, `validate` |
| `SigHashIndex` | `input index out of range` | `signature_hash_all_cached` |
| `ScriptMismatch` | `input script does not match key (refusing to sign)` | `sign_p2pkh_input_cached`, the airgap ownership proofs |
| `InvalidRequest(&'static str)` | the message itself | airgap validation (§8.6) |

The documentation of `Derivation` tells callers to treat it as fatal and retry "rather than silently skipping an index" (`dcr-rs:src/lib.rs:66-68`). dcrwallet does the opposite for `ErrInvalidChild`: it skips the index (§8.1, §9 item 4).

### 8.6 dcr-rs airgap `InvalidRequest` messages

Every message literal in `dcr-rs:src/airgap.rs`, in source order: `package too large` (`:297`); `trailing bytes after package` (`:312`); `unknown derivation branch` (`:401`, `:555`); `format version 3 input without prev_tx_prefix` (`:468-470`); `unparseable prev_tx_prefix` (`:472`); `prev_tx_prefix does not hash to the declared prevout` (`:483`); `prev_index past end of prev_tx_prefix` (`:490`); `declared input amount does not match the funding output` (`:494`); `declared input script does not match the funding output` (`:499`); `transaction has no inputs or outputs` (`:523`); `too many inputs or outputs` (`:528`); `unsupported transaction version` (`:534`); `hardened account number` (`:540`); `non-positive input amount` (`:544`); `input amount exceeds max supply` (`:547`); `hardened address index` (`:558`, `:876`); `only regular-tree outputs can be spent` (`:561`, `:700`, `:879`); `input without prev_tx_prefix` (`:566`); `negative output amount` (`:571`); `output amount exceeds max supply` (`:574`); `unsupported output script version` (`:582`); `output branch and index must both be present or both absent` (`:588`); `change output must carry its derivation path` (`:593`); `unknown output derivation branch` (`:599`); `hardened output address index` (`:602`); `input total exceeds max supply` (`:611`); `outputs exceed inputs (negative fee)` (`:614`); `fee is implausibly large` (`:626`); `same coin listed twice` (`:633`). A script that is neither P2PKH nor P2SH is displayed by `review_owned` as `<non-standard script>` (`:747-751`).

## 9. Compatibility-critical behaviours

Behaviours a drop-in replacement must reproduce because something outside it depends on them. Items 1–11 concern keys and seeds; 12–17 the gRPC transport; 18–21 conventions whose absence would break compatibility silently; 22–26 further key and seed behaviours.

1. **Derive with `Child` semantics everywhere.** Every existing wallet's account keys, and so every address it has ever handed out, were derived with dcrd's leading-zero-stripping variant (`dcrd/hdkeychain/v3@v3.1.3/extendedkey.go:304-311`; all dcrwallet call sites use `Child`). A strict-BIP32 implementation restores a different, empty wallet for roughly 1 seed in 130 at `m/44'/42'/0'` (`dcr-rs:src/hd.rs:25-31`).
2. **The hardened preimage layout follows the parent's stored form, not the function called** (§5.1 steps 3 and 9). A key reloaded from its base58 string is unstripped. The address manager reloads coin-type and account keys from strings, so which layout applies depends on whether a parent was derived in memory or parsed (spec 02). The Rust key type must carry the stored-length state and must clear it on parse.
3. **The master key is never stripped** (`extendedkey.go:522-533`). Stripping at use time, as dcr-rs did before 0.5.0, yields keys no dcrd API produces for about 1 seed in 256 (`dcr-rs:CHANGELOG.md`, 0.5.0).
4. **`ErrInvalidChild` must be a distinguishable error.** Address discovery and watching, next-address generation, address-index syncing in the address manager, the `getaddressesbyaccount` JSON-RPC handler (`internal/rpc/jsonrpc/methods.go:976-1020`) and the last-used-address-index database upgrade skip an index on exactly this error and abort on any other (§8.1). A port that reports one undifferentiated derivation error either skips indexes dcrwallet would fail on or fails where dcrwallet skips.
5. **`NewKeyFromString` takes the key type from the key data byte, not from the version, and re-serializes with the type's version** (`extendedkey.go:593-614`, `:454-458`). The address manager does not check `IsPrivate` on some import paths (spec 02 quirk 3), so the lenient behaviour is reachable from RPC. dcr-rs's stricter parse is a divergence, not an improvement, in this project.
6. **`NewKeyFromString` checks the network passed in and returns `ErrWrongNetwork`** for a key of any other network, including another Decred network (`:576-584`). dcr-rs accepts any of its four networks.
7. **Error precedence in `NewKeyFromString`** is length gate, decode length, checksum, network, key validity (§5.3). A client that sends a malformed key sees the first applicable message.
8. **Serialized depth is `depth mod 256`** (`:447`). Not reachable through the wallet's BIP0044 paths (maximum depth 5), but reachable by parsing a depth-255 key and deriving a child through an RPC that derives from an imported xpub; the child serializes with depth 0. Reproduce or record as a divergence (D-6).
9. **Seeds are raw bytes, 16–64 long, encoded as hex or PGP words with a double-SHA-256 checksum word** (§4.7). Decrediton and every user's paper backup hold the 33-word form. BIP39 phrases must be rejected exactly as dcrwallet rejects them (unknown word or position error), never silently expanded.
10. **Mnemonic output capitalization and separators.** `EncodeMnemonic` emits table capitalization and single spaces with no trailing space (`walletseed/seed.go:55-67`); gRPC `GenerateRandomSeedResponse.seed_mnemonic` and the creation prompt expose it verbatim. Input is case-insensitive and tolerates repeated spaces, but splits on the space character only (§5.4).
11. **The seed decode error texts are client-visible.** gRPC `SeedService.DecodeSeed` returns code `InvalidArgument` with the formatted wallet error as the message (`internal/rpc/rpcserver/server.go:3199-3205`), so the operation prefixes, kind text and the messages of §8.2 travel in `grpc-message`.
12. **gRPC is HTTP/2 over TLS (minimum 1.2) with ALPN `h2` and a required, verified client certificate** (§4.9). Decrediton and other clients connect with the certificates dcrwallet issues or is configured with.
13. **Message framing and status trailers** as in §4.9 items 2–5, including trailers-only responses for calls that fail before any message, and the percent-encoding of `grpc-message`.
14. **Protobuf bytes.** Fields in ascending field-number order, proto3 zero-value omission, packed `repeated uint32`, zigzag `sint32`. A client that hashes or byte-compares responses is unusual, but golden-file tests in client projects are not; and field-number order is what protobuf-go emits, so a Rust encoder that emits in declaration order differs wherever `api.proto` declares fields out of numeric order.
15. **Transport-level refusals** (unknown service or method, unsupported compression, oversize request) use grpc-go's codes and messages (§4.9 items 2, 6, 7).
16. **Limits clients can hit:** 4 MiB maximum request message; no server-side limit on response size below 2 GiB; keepalive enforcement closes a connection after more than two client pings that come too soon (within 5 minutes of the previous ping with a stream active, within 2 hours without one; `GOAWAY`, `too_many_pings`). A Rust server with a smaller default maximum response size or a stricter ping policy breaks clients that work today.
17. **No gzip, no reflection, no health service.** A client probing for them gets `Unimplemented`; a Rust stack that enables any of them by default changes observable behaviour.
18. **One reference Go release.** Error texts and number formatting that come from Go's standard library (hex and JSON errors, `%v` of floats, `%q`) must be reproduced from one named Go release, the one dcrwallet's release builds use, and CI must build the oracle with it (§5.6). dcrwallet's `go.mod` says `go 1.26.0`; which patch release its release process builds with is not recorded in the files read (D-9).
19. **Release builds wrap on integer overflow and abort on panic.** Go wraps silently; a Rust build with overflow checks on turns a wrap dcrwallet tolerates into a process abort (`dcroxide:Cargo.toml:82-94`).
20. **Map iteration order.** Go randomizes map iteration; where dcrwallet's output order depends on a map, neither a sorted nor a hashed Rust container reproduces it, and where it picks "the first" entry as an arbitrary choice, an explicit random draw is required (`dcroxide:docs/adr/0008-clippy-lint-policy.md:137-165`). Each such site is a QUIRKS entry.
21. **Upstream's misspellings in observable strings are reproduced**, and the spell checker is configured not to correct them (`dcroxide:_typos.toml`).
22. **Seed-decode error strings use the daemon's nested-error separator `:: `**, not the `errors` package default, and name both operations for word errors (§8.2). A gRPC client that shows or matches `DecodeSeed`'s status message sees the full string.
23. **The hex branch of `DecodeUserInput` is strict** over gRPC: an odd number of hex digits is an error there, while the interactive prompt pads it with a leading `0` before decoding and never reaches `DecodeUserInput` for a single token (§5.4). The two entry points must keep their different behaviour.
24. **`SerializedPubKey` of a stripped private key equals that of its padded form**, and the parent fingerprint of its children is computed from it (`dcrd/hdkeychain/v3@v3.1.3/extendedkey.go:166-167`, `:346`). The stored-length state affects the hardened HMAC preimage and `SerializedPrivKey` only; it must not leak into public keys, fingerprints or non-hardened children.
25. **A public-key parse failure in `NewKeyFromString` and in public child derivation returns the `secp256k1` error itself** (`:327-330`, `:607-610`), so `errors.Is` against `secp256k1.ErrPubKeyNotOnCurve` and the other `ParsePubKey` kinds holds, and the text a client sees is the `secp256k1` package's text, not an `hdkeychain` one.
26. **The seed is not length-checked by the encoders.** `EncodeMnemonic` and `EncodeMnemonicSlice` accept any length; the 16–64 bound is enforced by the callers that generate or decode a seed (§5.4).

## 10. Quirks

1. **Stripping is a property of slice length, not a flag.** dcrd has no field recording that a key was stripped; the behaviour follows from `copy(data[1:], k.key)` with a short slice (`extendedkey.go:248`, `:309-311`). `doc.go:49-57` describes it as "strips leading zeros of a private key, causing subsequent child keys to differ", which is accurate but does not say that parsing a key undoes it.
2. **A `dprv` round trip is not derivation-neutral.** `String` zero-pads and `NewKeyFromString` stores 32 bytes, so a key that was stored stripped derives different hardened children after a round trip (`extendedkey.go:464-465`, `:599`). Pinned in dcr-rs by `base58_round_trip_restores_the_padded_form` (`dcr-rs:tests/vectors.rs:890-933`).
3. **`ChildBIP32Std` from a stripped parent is not strict BIP32.** The flag controls only the child's storage (`extendedkey.go:309`). Pinned by `mixed_variant_path_matches_dcrd` (`dcr-rs:tests/vectors.rs:862-883`). Not reachable from dcrwallet, which never calls `ChildBIP32Std`.
4. **A zero private child is not rejected.** dcrd checks `Il` but not `Il + parent` (`extendedkey.go:277-311`). With `Child`, a zero sum is stripped to an empty key, and the resulting `ExtendedKey` serializes as `zeroed extended key` and reports `IsPrivate() == true`. Probability about 2^-256 per derivation; dcr-rs returns an error instead (`dcr-rs:src/hd.rs:116-121`). Record as a deliberate divergence if the port follows dcr-rs.
5. **A public child at infinity is not rejected** (`extendedkey.go:337-341`); `NewPublicKey` of the identity serializes a point with zero coordinates. Same probability class; same divergence choice.
6. **The public path checks `x == 0 || y == 0` of `Il·G`** and calls that an invalid child (`extendedkey.go:320-322`). No secp256k1 point has a zero coordinate reachable this way; the branch is dead in practice.
7. **`depth` is `uint16` in memory and one byte on the wire** (`extendedkey.go:125`, `:447`, `:587`). `Depth()` can return up to 65,535 for a key derived in memory while `String()` shows it modulo 256.
8. **`ParentFingerprint()` reads a big-endian `uint32` from the 4-byte fingerprint slice** (`extendedkey.go:194-196`). `Zero` clears that slice in place without releasing it (`:492`), so the method returns 0 on a zeroed key rather than failing. dcrwallet does not call it.
9. **`Neuter` on a public key returns the same pointer** (`extendedkey.go:398-400`), so `Zero` on the result zeroes the original. dcrwallet zeroes derived keys freely; a Rust port with value semantics cannot alias and does not need to.
10. **`SerializedPrivKey` returns the internal slice**, shorter than 32 bytes for a stripped key, and the caller is told not to modify it; dcrwallet's one caller then zeroes it (`wallet/udb/addressmanager.go:2498-2503`), which zeroes the extended key's own storage.
11. **`hdkeychain` uses the name "Bitcoin seed"** as the master HMAC key (`extendedkey.go:99`); Decred differs from Bitcoin BIP32 only in version bytes, the BLAKE-256-based fingerprint and checksum, and the stripping.
12. **`walletseed.GenerateRandomSeed` rejects `size >= 255` before the range check** (`walletseed/seed.go:23`), a guard for the `uint8` narrowing that the range check alone would already cover. The gRPC `GenerateRandomSeed` handler does not call it: it validates the length itself, returns `invalid seed length` and fills the seed from the random source directly (`internal/rpc/rpcserver/server.go:3177-3196`).
13. **`DecodeUserInput` treats any single token as hex**, so a one-word mnemonic attempt reports a hex error, and an input of only spaces reaches the length check with an empty seed and reports the seed-length error (`walletseed/seed.go:73-82`, `:98-100`).
14. **`DecodeUserInput` with exactly one decoded mnemonic byte** skips the checksum test and reports the seed-length error (`:89-91`). Two or more words that decode to at least two bytes are always checksum-tested, so a 16-byte seed needs 17 words.
15. **`DecodeMnemonics` reports the position as the count of accepted words**, not the index in the input slice, so empty elements do not advance it (`pgpwordlist/pgpwordlist.go:40-57`).
16. **The mnemonic "checksum" is one byte**: a random word list of the right shape passes with probability 1/256. It detects transposed or missing words mostly through the even/odd position rule rather than the checksum.
17. **dcr-rs calls its seed input "a BIP39 seed"** (`dcr-rs:src/hd.rs:200-201`) and makes BIP39 a default feature; dcrwallet has no BIP39 anywhere in the tree at the pin. The two projects' seed formats are different on purpose (hardware signers restore BIP39 phrases).
18. **dcr-rs binds key type to version prefix and takes no expected network** (`dcr-rs:src/hd.rs:482-488`, `:649-655`). Documented in dcr-rs as deliberate narrowing; a parity gap for the wallet.
19. **dcr-rs `format_amount` is not `dcrutil.Amount.String()`** (`dcr-rs:src/amount.rs:11-24`): it trims trailing zeros and omits the fraction for whole coins.
20. **dcroxide's `PrivateKey::from_bytes` rejects out-of-range scalars where dcrd's `PrivKeyFromBytes` reduces modulo N and accepts any length** (`dcroxide:crates/dcroxide-dcrec/src/secp256k1.rs:335-359`; `dcrd/dcrec/secp256k1/v4@v4.4.1/modnscalar.go:349-368`). dcroxide records it as not observable for a node. For a wallet it is observable: imported private keys and stripped HD keys reach `PrivKeyFromBytes` (`wallet/udb/addressmanager.go:2502`, `:2510`).
21. **dcroxide's ADR bodies are not rewritten when reality diverges.** ADR-0005's Decision section still describes tokio and ADR-0007's still describes serde; each "reads correctly only through the addenda" (`dcroxide:docs/adr/0007-json-emission-strategy.md`, end of the 2026-10-26 addendum). Read the status line and the addenda first.
22. **dcroxide's CI has caught its own pins drifting apart** (harness, CI and oracle naming different upstream commits for two pin moves; `dcroxide:docs/PARITY.md:11-25`). The consistency test exists because the convention alone did not hold.
23. **dcroxide's JSON float formatter shipped with an exact-tie rounding bug for eleven weeks** because no generated corpus covered ties (`dcroxide:docs/adr/0007-json-emission-strategy.md`, corrections in the 2026-10-26 addendum). A kept generator and a committed corpus are still pending there.
24. **Comments are claims, not evidence.** dcroxide's review found five comments asserting the opposite of the code beneath them, and a later sweep retired twelve more (`dcroxide:SECURITY.md:500-505`, closing section). The same holds for dcrwallet's doc comments, which is why every spec in this set cites code.

25. **`Neuter` of a private key aliases the parent's buffers.** The neutered key's `key` is the private key's memoized `pubKey` slice, and its `chainCode` and `parentFP` are the private key's own slices (`extendedkey.go:406-407`, `:165-170`). `Zero` clears those slices in place (`:489-492`), so zeroing the private key leaves the neutered key with a 33-byte all-zero key, an all-zero chain code and fingerprint (it does not print as `zeroed extended key`, because its `key` is non-empty), and zeroing the neutered key clears the private key's chain code, fingerprint and memoized public key while leaving its secret intact. dcrwallet calls `Neuter` then `Zero` on the private key in discovery (`wallet/discovery.go:432-443`, `:571-575`). Account creation in the address manager neuters the account key and serializes both forms, but zeroes only the coin-type key it was derived from, not the account key (`wallet/udb/addressmanager.go:2135-2147`). At the discovery sites every later use is of keys derived from the neutered key before the `Zero`, or of strings already serialized, so no output depends on the aliasing; the address manager's cached account keys are parsed separately from their stored strings (`wallet/udb/addressmanager.go:500-535`) and do not alias. A Rust port with owned values cannot reproduce the aliasing; each `Neuter`-then-`Zero` site in the specs that own it must be confirmed not to read the neutered key after the private key is zeroed (the remaining sites, `walletsetup.go:83`, `wallet/wallet.go:3854`, `wallet/udb/addressmanager.go:2261`, `:2922-2975`, were located but not traced to the end of the neutered key's lifetime here).

26. **`NewMaster`'s documentation names an error that does not exist.** The comment says "The ErrUnusable error will be returned" (`extendedkey.go:503-506`); the identifier is `ErrUnusableSeed` (`:75`, `:528`).
27. **`Neuter`'s documentation and behaviour disagree for public keys.** `doc.go:74-77` says a private key "can be converted to a new instance" and "the original extended key is not modified"; for a key that is already public the function returns the receiver (`extendedkey.go:398-400`), and for a private key the new instance shares buffers with the original (item 25) and the call writes the original's memoized `pubKey` field (`:165-168`).
28. **A zeroed key is a public key with an empty key.** `Zero` sets `key = nil`, `depth = 0`, `childNum = 0`, `isPrivate = false`, leaves `privVer` and `pubVer` as they were, and leaves `chainCode`, `parentFP` and `pubKey` as zero-filled slices of their old lengths (`:488-497`). Afterwards `String()` is `zeroed extended key`; `IsPrivate()` is false; `SerializedPrivKey()` returns `ErrNotPrivExtKey`; `SerializedPubKey()` returns an empty slice; `Neuter()` returns the receiver; `Child(i)` with `i >= 0x80000000` returns `ErrDeriveHardFromPublic`; and `Child(i)` with a smaller `i` runs the HMAC over 33 zero bytes and the index under an all-zero chain code and then fails in `secp256k1.ParsePubKey` of the empty key, returning that error (or `ErrInvalidChild` first, should the HMAC's left half be out of range). `TestZero` pins the field values and the string, not the derivation behaviour.
29. **`walletseed.GenerateRandomSeed` is dead code in the daemon.** Its only callers at the pin are the database fixture generators under `wallet/udb/testdata` (§3.10). Its documentation says `hdkeychain.ErrInvalidSeedLen` "is returned"; the code returns it wrapped in an `errors.E` of kind `Invalid` (`walletseed/seed.go:18-28`), so only `errors.Is` finds it.
30. **`pgpwordlist.DecodeMnemonics`'s comment is garbled** ("Any words that are whitespace are empty are skipped", `pgpwordlist/pgpwordlist.go:34-35`); the code skips an element that is empty after `strings.TrimSpace` (`:42-45`). It returns a non-nil empty slice, not nil, when every element is skipped (`:39`, `:59`).
31. **`EncodeMnemonic` and `EncodeMnemonicSlice` disagree for an empty seed.** The slice form returns the single checksum word; the string form writes a space before the checksum word unconditionally (`walletseed/seed.go:63-65`), so it returns that word with a leading space. `SHA256(SHA256(""))[0]` is `0x5d` and position 0 is even, so the word is table index 186, `exceed`. Not reachable through the daemon, whose callers pass 16–64 bytes.
32. **The prompt and `DecodeUserInput` duplicate, and disagree on, the hex path and the splitting rule** (§5.4): the prompt collapses all Unicode white space and pads odd-length hex; `DecodeUserInput` splits on U+0020 only and rejects odd-length hex. A mnemonic sent over gRPC with only tab or newline separators is one token, is treated as hex and fails with a hex error; with mixed separators, a piece such as two words joined by a newline fails the word lookup and is echoed, newline included, in the error text.
33. **dcr-rs documents `Error::Derivation` as fatal, to be retried rather than skipped** (`dcr-rs:src/lib.rs:66-69`), and folds seed-length, invalid-child and degenerate-sum failures into it. dcrwallet's address loops depend on telling the invalid-child case apart and skipping it (§9 item 4).
34. **dcr-rs lints and documents on a newer toolchain than its MSRV.** The MSRV job only runs `cargo check --locked` on 1.85; `rustfmt`, `clippy` and the warnings-denied `cargo doc` run on 1.97.1, and the test job on whatever stable is current (`dcr-rs:.github/workflows/ci.yml:12-119`). Its changelog notes that a fresh dependency resolve on 1.85 can pick a patch release that needs a newer compiler (`dcr-rs:CHANGELOG.md`, 0.6.3).
35. **dcr-rs's secret erasure writes `0x01` bytes, not zeros**, for the secp256k1 secret, because the bindings refuse an all-zero scalar (`dcr-rs:src/hd.rs:184-196`). A wallet-side wipe test that looks for zeros would not transfer.
36. **`dcroxide-winsvc` does not inherit the workspace lint table.** It restates the lints by hand because it cannot forbid `unsafe_code`, so every workspace lint added must be mirrored there (`dcroxide:docs/adr/0008-clippy-lint-policy.md:226-232`). A package cannot combine `[lints] workspace = true` with a lint table of its own, so per-crate ratchets are `#![deny(...)]` attributes in `src/lib.rs`, which reach the library and its unit tests but not `tests/*.rs` (`:103-108`); `dcroxide-wire` carries `#![deny(clippy::iter_over_hash_type)]` that way (`dcroxide:crates/dcroxide-wire/src/lib.rs:48`).
37. **`Cargo.lock` lists crates no build compiles.** `ring`, and the crates only it takes, stay in dcroxide's lock because a `?` feature names them; `cargo-deny`'s graph leaves them out, so the bans do not fire (`dcroxide:deny.toml:41-53`). A reader auditing the lock alone would conclude the pure-Rust rule is broken.

## 11. Upstream tests

### 11.1 `hdkeychain` v3.1.3 (`dcrd/hdkeychain/v3@v3.1.3/extendedkey_test.go`)

| Test | Pins | Port |
| --- | --- | --- |
| `TestBIP0032Vectors` (`:66`) | BIP32 vectors 1–3 on mainnet prefixes and vector 1 on testnet prefixes: private and public serialization at each path step, through `Child` | Port all rows. dcr-rs carries only the six vector-1 mainnet rows (`dcr-rs:tests/vectors.rs:54-97`). |
| `TestPrivateDerivation` (`:322`) | Parse a `dprv`, derive non-hardened children, compare; includes `Derived privkey with zero high byte m/0` (`:412-416`) | Port. The zero-high-byte row checks that serialization pads; it does not derive below the stripped key. |
| `TestPublicDerivation` (`:451`) | Parse a `dpub`, derive, compare; twelve rows | Port. dcr-rs carries eight (`dcr-rs:tests/vectors.rs:226-273`). |
| `TestGenenerateSeed` (`:570`) | Lengths 16, 17, 20, 32, 64 accepted; 15 and 65 → `ErrInvalidSeedLen` | Port. |
| `TestExtendedKeyAPI` (`:606`) | For one parsed `dprv` and one parsed `dpub`: `IsPrivate`, `ParentFingerprint`, `String()` round trip, `SerializedPrivKey`/`SerializedPubKey` hex, `ErrNotPrivExtKey` | Port against the Rust API's equivalents. |
| `TestErrors` (`:695`) | Seed length errors; hardened-from-public; `NewKeyFromString` rows: too long → `ErrInvalidKeyLen`, `dpub1234` → `ErrInvalidKeyLen`, bad checksum, pubkey not on curve → `secp256k1.ErrPubKeyNotOnCurve`, other-network version → `ErrWrongNetwork` | Port with the exact error identities. |
| `TestZero` (`:777`) | Every field cleared after `Zero`, `String()` = `zeroed extended key`, for master, neutered and deserialized keys | Not portable as written under value semantics; replace with a wipe test. |
| `TestBIP0032Vector4` (`:903`) | BIP32 vector 4 through `ChildBIP32Std`, including a private key with a leading zero byte; the comment notes the second row "fails when using Child" | Port as the strict-variant test. |

The package's other test files:

| File | Contents | Port |
| --- | --- | --- |
| `bench_test.go` | `BenchmarkDeriveHardened`, `BenchmarkDeriveNormal`, `BenchmarkPrivToPub`, `BenchmarkDeserialize`, `BenchmarkSerialize`, all on the BIP32 vector 1 master `dprv3hCznBesA6jBtmoyVFPfyMSZ1qYZ3WdjdebquvkEfmRfxC9VFEFi2YDaJqHnx7uGe75eGSa3Mn3oHK11hBW7KZUrPxwbCPBmuCi1nwm182s` (`:14-15`) | Optional; they pin no output. Useful as the hot-path benchmarks the definition of done asks for (§5.6). |
| `example_test.go` | `Example_newMaster` (output `Private Extended Key?: true`); `Example_defaultWalletLayout`, which derives `m/0'/0/10` and `m/0'/1/0` from a hard-coded master and pins the mainnet P2PKH addresses `DshMmJ3bfvMDdk1mkXRD3x5xDuPwSxoYGfi` and `DsoTyktAyEDkYpgKSex6zx5rrkFDi2gAsHr` (`:150-152`); `Example_audits`, which pins the neutered master `dpubZ9169KDAEUnypHbWCe2Vu5TxGEcqJeNeX6XCYFU1fqw2iQZK7fsMhzsEFArbLmyUdprUw9aXHneUNd92bjc31TqC6sUduMY6PK2z4JXDS8j` (`:193-194`) | Port the two pinned outputs as vectors: they are the only upstream check that joins derivation, `Neuter`, `SerializedPubKey` and address encoding. |
| `extendedkey_test.go:32-64` | `mockNetParams` with `mockMainNetParams` and `mockTestNetParams`: the tests use stand-in parameter types, not `chaincfg` | The Rust tests can use `dcroxide-chaincfg` parameters; the version bytes are the same. |

`TestExtendedKeyAPI`'s two rows, for reference when porting (`:616-631`): the vector 1 master (private key `33a63922ea4e6686c9fc31daf136888297537f66c1aabe3363df06af0b8274c7`, public key `039f2e1d7b50b8451911c64cf745f9ba16193b319212a64096e5679555449d8f37`, parent fingerprint 0) and the vector 2 key at `m/0/2147483647/1/2147483646/2` in public form (parent fingerprint 4220580796, `ErrNotPrivExtKey` from `SerializedPrivKey`). The upstream test function for seed generation is spelled `TestGenenerateSeed` (`:570`).

Not covered upstream, and therefore to be generated through the oracle (dcr-rs's rows are the starting set, each already generated from dcrd; `dcr-rs:tests/vectors.rs:568-933`):

- `Child` across a stripped hardened parent (`hardened_derivation_matches_dcrd_legacy_child`, two seeds, both variants).
- A master key with a leading zero byte (`master_key_leading_zero_is_not_stripped`).
- Mixed `Child`/`ChildBIP32Std` along one path.
- Parse-then-derive against derive-in-memory for a stripped key.
- The dcrwallet path `m/44'/42'/0'/0/0` to an address for a leading-zero seed (`wallet_path_matches_dcrwallet_for_a_leading_zero_seed`).
- The version/data mismatch strings (`bip32_key_type_is_bound_to_the_version_prefix`, `:160-195`), with the expectation inverted: the wallet port must *accept* both strings as dcrd does.
- Legacy coin types (20, 11, 115, 1) and the SLIP-0044 upgrade path: no dcr-rs vector; generate.
- A randomized differential over seeds (dcr-rs's module documentation reports a 200,000-seed comparison against dcrd; the program is not in the repository, `dcr-rs:src/hd.rs:27-31`).

### 11.2 `walletseed` and `pgpwordlist`

| Test | Pins | Port |
| --- | --- | --- |
| `walletseed.TestEncodeMnemonicSlice`, `TestEncodeMnemonic` (`walletseed/seed_test.go:46-62`) | Four vectors (two 20-byte, two 31-byte) including the checksum word and capitalization | Port verbatim. |
| `walletseed.TestDecodeMnemonic`, `TestDecodeHex` (`:64-88`) | Round trip of the same vectors through `DecodeUserInput` | Port. Note the vectors are 20 and 31 bytes, inside 16..64. |
| `pgpwordlist.TestDecode`, `TestEncodeMnemonics` (`pgpwordlist/pgpwordlist_test.go:43-66`) | Two 20-byte vectors without checksum | Port. |

Untested upstream: every error path (unknown word, wrong position, checksum mismatch, bad hex, length bounds, `size >= 255`), case-insensitive input, repeated spaces, tab or newline separators, single-token input. Generate these through the oracle, including the full formatted error strings.

### 11.3 dcr-rs's own tests

| File | Pins | Use |
| --- | --- | --- |
| `dcr-rs:src/blake256.rs` tests | 17 dcrd-generated KAT lengths covering every padding path; every split point | Already in `dcroxide-crypto`, compared with dcrd live over the 17 KAT lengths and 5,000 random inputs (`dcroxide:docs/adr/0002-vendor-blake256-from-dcr-rs.md:82-88`). |
| `dcr-rs:tests/vectors.rs` | §11.1; ten address vectors (seven from dcrd, three self-generated regression pins for simnet/regnet, `:356-381`); six BIP39 vectors | BIP32 rows: take. Address rows: superseded by `dcroxide-txscript`'s `stdaddr` tests. BIP39 rows: not applicable. |
| `dcr-rs:tests/onchain_sighash.rs` | A mainnet transaction (`37564c16…d954`) whose two signatures verify against the recomputed SigHashAll hash; txid; byte-exact round trip; parser strictness | The fixture is a useful end-to-end signing check for the wallet's signer; the strictness tests do not apply to a `wire` port. |
| `dcr-rs:tests/airgap_review.rs` | The airgap format and policy | Not applicable. |

Every test function in dcr-rs, by file:

| File | Tests |
| --- | --- |
| `dcr-rs:src/blake256.rs` | `dcrd_generated_kats`, `incremental_matches_one_shot_for_every_split`, `incremental_byte_at_a_time`, `empty_string_kat`, `single_zero_byte_kat` |
| `dcr-rs:src/hashing.rs` | `p2pkh_address_roundtrip`, `bad_checksum_rejected` |
| `dcr-rs:src/address.rs` | `p2pkh_script_matches_dcrd`, `decode_roundtrip_all_networks`, `script_classification_roundtrip` |
| `dcr-rs:src/network.rs` | `prefix_lookup_is_unambiguous` |
| `dcr-rs:src/amount.rs` | `amount_formatting` (seven values including `i64::MIN` → `-92233720368.54775808 DCR`) |
| `dcr-rs:src/hd.rs` | `invalid_hmac_left_halves_are_rejected` (all-zero and all-`0xff` left halves), `a_zero_tweak_would_return_the_parent_unchanged` (pins libsecp256k1's acceptance of a zero tweak), `secret_erasure_fills_with_ones` |
| `dcr-rs:tests/vectors.rs` | `blake256_empty_kat`, `blake256_single_zero_kat`, `bip32_vector1_priv_and_pub_chains`, `bip32_seed_length_bounds`, `bip32_serialization_roundtrip`, `bip32_priv_pub_prefix_mixups_rejected`, `bip32_key_type_is_bound_to_the_version_prefix`, `bip32_testnet_simnet_version_bytes`, `bip32_public_derivation_vectors`, `bip32_public_hardened_derivation_rejected`, `bip32_priv_and_pub_derivation_agree`, `address_vectors_encode_and_decode`, `address_payscript_layouts`, `address_decode_rejects_garbage`, `address_key_rejects_hardened_components`, `account_key_rejects_hardened_account_numbers`, `decode_for_rejects_cross_network_addresses`, `base58_decoders_reject_overlong_input`, `ext_key_parse_error_paths`, `hardened_derivation_matches_dcrd_legacy_child`, `leading_zero_key_serializes_identically_under_both_variants`, `wallet_path_matches_dcrwallet_for_a_leading_zero_seed`, `bip39_mnemonic_expansion_matches_published_vectors`, `master_key_leading_zero_is_not_stripped`, `mixed_variant_path_matches_dcrd`, `base58_round_trip_restores_the_padded_form` |
| `dcr-rs:tests/onchain_sighash.rs` | `onchain_tx_signatures_verify_against_our_sighash`, `onchain_tx_serialize_roundtrip`, `parse_full_rejects_malformed_input`, `onchain_txid_matches`, `parse_full_rejects_non_canonical_varints`, `parse_full_rejects_trailing_bytes`, `parse_prefix_reproduces_the_mainnet_txid`, `cached_prefix_hash_matches_the_uncached_sighash` |
| `dcr-rs:tests/airgap_review.rs` | 29 tests of the airgap format and policy (CBOR round trip and golden layout, version gate, funding-prefix verification, change and input ownership proofs, amount and count limits, duplicate inputs, hardened paths, script binding, package size) |

Of the `vectors.rs` tests, the wallet's `hdkeychain` port takes the vectors of the tests named in §11.1 and re-expresses four that pin dcr-rs's narrowing: `bip32_priv_pub_prefix_mixups_rejected` and `bip32_key_type_is_bound_to_the_version_prefix` (dcrd parses all four strings; the expectation is inverted), `ext_key_parse_error_paths` (same inputs, dcrd's error identities: `ErrBadChecksum` for the flipped character, `ErrInvalidKeyLen` for `dprv` and for `dprv0OIl`) and `base58_decoders_reject_overlong_input` (the gate is 113 characters and the error `ErrInvalidKeyLen`). The expected dcrd results come from the oracle, not from this paragraph. `leading_zero_key_serializes_identically_under_both_variants` is taken as is: it pins that serialization pads a stripped key.

dcr-rs's CI (`dcr-rs:.github/workflows/ci.yml`) has seven jobs: `test` (the suite under all five feature combinations), `msrv` (1.85, `cargo check --locked`, plus a `thumbv7em-none-eabihf` check that installs `gcc-arm-none-eabi`), `lint` (1.97.1), `lint-latest` (advisory), `no-std`, `package` (`cargo package --locked`) and `docs`. The `msrv` and `no-std` jobs install the cross-compiler because `secp256k1-sys` builds C for the target (`dcr-rs:.github/workflows/ci.yml:49-52`), which is the dependency dcroxide's `no-c-compiler` job exists to refuse.

dcr-rs's CI does not regenerate any vector against dcrd (`dcr-rs:SECURITY.md:17-22`); under dcroxide's rule the wallet does (§5.6).

### 11.4 Batteries this document adds to the plan

- **gRPC golden bytes.** An exporter inside dcrwallet's `rpc/walletrpc` package that marshals each response message type with representative and zero values and dumps the bytes; plus a live capture harness that records the HTTP/2 frames of a call that succeeds, one that fails before any message, one that fails mid-stream, and the transport refusals of §4.9. This is the only way to settle the two UNVERIFIED rows of §4.9.
- **Oracle commands** `hd_new_master`, `hd_child`, `hd_child_bip32_std`, `hd_from_string`, `hd_string`, `seed_encode`, `seed_decode`, returning values and error identities and texts.

### 11.5 dcroxide's test conventions (to copy)

Differential tests print the seed they drew and accept it back through an environment variable for replay; fuzz targets have the invariants `encode(decode(bytes)) == bytes[..consumed]` and "decode never panics"; vectors live under `crates/*/tests/data`; each replay file's header names the upstream commit the vectors were dumped at (`dcroxide:README.md`, Development; `dcroxide:docs/adr/0003-slice-based-wire-decoding.md:41-43`).

## 12. Rust porting notes

### 12.1 Dependencies of dcr-rs against the pure-Rust rule

Direct dependencies (`dcr-rs:Cargo.toml:52-80`) and what `dcr-rs:Cargo.lock` resolves:

| Crate | Version | Role in dcr-rs | Under dcroxide's rules |
| --- | --- | --- | --- |
| `secp256k1` | 0.29.1 (`secp256k1-sys` 0.10.1, `cc` 1.5.1) | all curve arithmetic: key validation, tweak-add for CKD, point tweak-add, ECDSA signing; re-exported and present in public signatures | **Refused.** `secp256k1-sys` is banned by name and `cc` by wrapper list (`dcroxide:deny.toml:77-87`); libsecp256k1 is vendored C, which ADR-0012 item 1 excludes (`dcroxide:docs/adr/0012-pure-rust.md:130-131`). dcroxide removed the same binding with the merge of 2026-11-10 (`dcroxide:docs/adr/0017-secp256k1-in-pure-rust.md:104-106`, `:320`; `dcroxide:docs/adr/README.md:14`). |
| `sha2` 0.11.0, `hmac` 0.13.0, `ripemd` 0.2.0 (digest 0.11 family, `zeroize` features on) | HMAC-SHA512, RIPEMD-160 | Same family and majors as dcroxide (`dcroxide:crates/dcroxide-dcrec/Cargo.toml`, `dcroxide:crates/dcroxide-crypto/Cargo.toml`); no conflict. |
| `bs58` 0.5.1 | base58 | Pure Rust (UNVERIFIED beyond its manifest), but dcroxide owns a port of `decred/base58` with Go's failure behaviour; a second base58 is redundant. |
| `zeroize` 1.9.0 | wipes | Already in dcroxide's lock. |
| `bip39` 2.2.2 (pulls `bitcoin_hashes`, `unicode-normalization`) | BIP39, feature `mnemonic` | Not needed: dcrwallet has no BIP39 (§4.7). |
| `minicbor` 0.24.4 (`minicbor-derive`) | CBOR, feature `airgap` | Not needed (§3.8). |
| `hex` 0.4 (dev) | tests | — |

With `default-features = false` dcr-rs still requires `secp256k1`. There is no feature that swaps the curve backend, and `secp256k1` types are part of its public API (`dcr-rs:README.md:183-185`). dcr-rs therefore cannot be linked into a workspace that inherits `deny.toml` unchanged, with or without its optional features.

Other differences from the dcroxide baseline: edition 2021 and MSRV 1.85 (`dcr-rs:Cargo.toml:11`, `:23`) against edition 2024 and 1.99.1; `#![warn(missing_docs)]` against `deny`; published on crates.io at 0.6.3 with a signed `SHA256SUMS` (`dcr-rs:README.md:96-104`), where dcroxide publishes nothing.

### 12.2 Reuse matrix: which implementation the wallet takes for each concern

| Concern | dcr-rs | dcroxide | Take |
| --- | --- | --- | --- |
| BLAKE-256 | `blake256` (looped compress, wipes state on drop) | `dcroxide-crypto::blake256`, imported once from dcr-rs and since unrolled; differential against dcrd on every CI run; no wipe on drop, declined because the node hashes no secrets (`dcroxide:docs/adr/0002-vendor-blake256-from-dcr-rs.md:71-91`) | **dcroxide.** The wallet does hash secrets (the `dprv` checksum runs BLAKE-256 over a buffer holding a private key); see D-8. |
| RIPEMD-160, `hash160` | `ripemd` crate + own `hash160` | `dcroxide-crypto::ripemd160`, `dcroxide-txscript::stdaddr::hash160` | **dcroxide.** |
| CSPRNG | none | `dcroxide-crypto::rand`, a port of dcrd `crypto/rand` (`Prng`, `read`, `uint64`, `uint64n`, `int_n`, `duration`, `shuffle`) behind the crate feature `rand`, which is off by default (`dcroxide:crates/dcroxide-crypto/Cargo.toml:16-30`); its `PARITY.md` status is `semantics`, not `vectors` (`dcroxide:docs/PARITY.md:1013`) | **dcroxide.** dcrwallet depends on `dcrd/crypto/rand` v1.0.1. |
| base58, base58check | `bs58` + `hashing::check_*` | `dcroxide-base58` (`decred/base58` v1.0.6 semantics) | **dcroxide.** `hdkeychain.NewKeyFromString` relies on `Decode` returning empty for bad input (§5.3 step 2). |
| Network parameters | four networks, six identifiers each | `dcroxide-chaincfg` full `Params`, including `hd_private_key_id`, `hd_public_key_id`, `slip0044_coin_type`, `legacy_coin_type` (`dcroxide:crates/dcroxide-chaincfg/src/lib.rs:188-191`) | **dcroxide.** |
| Addresses and scripts | P2PKH and P2SH only | `dcroxide-txscript::stdaddr` (all v0 address kinds, stake scripts), `stdscript` | **dcroxide.** |
| Transaction wire format | prefix/witness/full; strict parse | `dcroxide-wire::MsgTx` (all serialization types, `(value, consumed)` decoding, dcrd error texts), plus every P2P message including the mixing and cfilter messages | **dcroxide.** |
| Signature hash | SigHashAll | `dcroxide-txscript` `calc_signature_hash_*` for all hash types | **dcroxide.** |
| Signing | P2PKH, ECDSA | `dcroxide-txscript::sign` (`raw_tx_in_signature`, `signature_script`, `sign_tx_output` with key and script lookups, `tspend_signature_script`), `dcroxide-dcrec` ECDSA, compact signatures, Schnorr, Ed25519 | **dcroxide.** |
| secp256k1 keys | libsecp256k1 | `dcroxide-dcrec` on `k256` (secrets) and `dcroxide-secp256k1-vartime` (public values) | **dcroxide**, extended (§12.3). |
| HD keys | `hd` (Decred variant, provenance flag, narrowed parsing, own errors) | **none** (`dcroxide:docs/adr/0002-vendor-blake256-from-dcr-rs.md:125`: "dcroxide has no `hdkeychain` port") | **New crate**, design informed by dcr-rs (§12.3). |
| Seed mnemonic | BIP39 | none | **New**, port of `walletseed` + `pgpwordlist` (§12.4). |
| Amount formatting | `format_amount` | upstream-compatible amount handling lives with the RPC types | **dcroxide / new port of `dcrutil.Amount`**, not dcr-rs. |
| Airgap CBOR | `airgap` | none | **Neither** (D-11). |
| GCS filters, stake, standalone, mixing primitives, JSON-RPC types and Go-JSON, TLS provider, certificate generation, address manager for peers, peer and connection manager | none | `dcroxide-gcs`, `-stake`, `-standalone`, `-mixing`, `-dcrjson`, `-rpctypes`, `-tlscrypto`, `-certgen`, `-addrmgr`, `-peer`, `-connmgr` | **dcroxide**, each assessed in the spec that owns the subsystem. |

What dcr-rs still contributes: (a) the `zeros_stripped` design and the reasoning recorded in its module documentation; (b) its dcrd-generated discriminating vectors (§11.1); (c) the wipe-on-every-exit closure pattern for fallible code that holds secrets (`dcr-rs:src/hd.rs:209-231`, `:346-376`, `:734-756`). Under ADR-0002's addendum, taking code or vectors from dcr-rs into a dcroxide-family tree "needs a decision of its own" (`dcroxide:docs/adr/0002-vendor-blake256-from-dcr-rs.md:119-121`); that is D-4.

dcr-rs release history that bears on taking its design or vectors (`dcr-rs:CHANGELOG.md`): 0.1.0–0.3.0 derived with strict BIP32 only; 0.4.0 (2026-07-29) introduced dcrd's stripping variant but stripped at use time, master included; 0.5.0 (2026-07-31) stopped stripping the master; 0.6.0 (2026-08-25) added the zero-tweak rejection; 0.6.1 and 0.6.3 are key-hygiene releases (wipes on every exit, HMAC state wiped through the `digest` 0.11 family) and 0.6.3 added the refusal of account numbers at or above `2^31`. Only the `hd.rs` and vectors of 0.5.0 or later match dcrd; the BLAKE-256 dcroxide imported came from dcr-rs commit `fd32c1a` (`dcroxide:docs/adr/README.md`, ADR-0002 entry). dcr-rs's own versioning rule treats any change to a derived key or address as a breaking (minor) release (`dcr-rs:README.md:169-188`), which is the same promise the wallet makes to an existing `wallet.db`.

dcr-rs's stated limits (`dcr-rs:SECURITY.md:37-97`), for the record of what a one-time import would and would not bring: no audit; wipes shrink the exposure window and do not reach by-value copies, HMAC internals or the returned `dprv` string; no constant-time claim for its own code or for `bs58`; out of scope are networking, RPC, staking, mixing and transaction-construction policy; only P2PKH inputs under SigHashAll are signed; only the latest release gets fixes.

### 12.3 What the wallet needs that neither has: `hdkeychain`

Suggested crate: `purse-hdkeychain` (or `dcroxide-hdkeychain` if it lands in the dcroxide workspace), `no_std + alloc`, depending on `dcroxide-dcrec`, `dcroxide-crypto`, `dcroxide-base58`, `hmac`, `sha2`, `zeroize`.

Required API, mirroring §3.9:

- One `ExtendedKey` type holding either a private or a public key, because dcrd's parse decides the type from data and callers branch on `is_private()`. Fields: private and public versions, key, chain code, parent fingerprint, child number, depth (`u16`), and for private keys the stored length (or stripped flag).
- `new_master`, `child`, `child_bip32_std`, `neuter`, `serialized_pub_key`, `serialized_priv_key` (returning the stored-length bytes so behaviour is testable against the oracle), `to_string`, `from_string(s, &params)`, `generate_seed`, accessors.
- An error enum with one variant per dcrd sentinel and dcrd's texts (§8.1), with the `ParsePubKey` errors passed through from `dcroxide-dcrec`.
- Generic over a `NetworkParams`-style trait implemented by `dcroxide-chaincfg::Params`.

Additions needed in `dcroxide-dcrec` (its public secp256k1 API at the commit read has `PrivateKey::from_bytes`, `PrivateKey::public_key`, `PublicKey::parse`, `serialize_compressed`, `serialize_uncompressed`, and, in its `ecdsa`, `schnorr` and `nonce` modules, signature parsing, signing, verification, compact signing and recovery and the RFC 6979 nonce; no scalar or point addition is exposed; `dcroxide:crates/dcroxide-dcrec/src/secp256k1.rs:11-13`, `:213-371`):

1. Constant-time scalar addition modulo N for the private child: `(Il + parent) mod N` returning 32 bytes, on `k256` scalars with wiped temporaries. dcrd's `ModNScalar.Add` is constant time.
2. Public-key tweak addition `Il·G + parent` for the public child. Operates on public values; dcrd uses variable-time routines here (`ScalarBaseMultNonConst`, `AddNonConst`, `extendedkey.go:318`, `:338`), so `dcroxide-secp256k1-vartime` is admissible.
3. A reducing private-key constructor with dcrd `PrivKeyFromBytes` semantics: take at most the first 32 bytes, left-pad, reduce modulo N, accept zero (`dcrd/dcrec/secp256k1/v4@v4.4.1/modnscalar.go:349-368`). Needed for stripped keys and for imported keys (§10 item 20).
4. Read access to a private key's 32 bytes for serialization (`secret_bytes` is `pub(crate)` today, `secp256k1.rs:386`).

Each addition is differential-tested through the oracle like the rest of `dcroxide-dcrec`.

Constraints the ledger records for the curve crate these additions sit on (`dcroxide:docs/dependency-ledger.md:42`): `k256` is taken with its `ecdsa`, `std` and `precomputed-tables` features off; dcrd's acceptance rules, range checks and error identities live in `dcroxide-dcrec` in front of it, never in `k256`; generator multiplication on a secret goes through `ProjectivePoint::GENERATOR * k` in constant time. The private-child scalar addition and the master and parsed-key range checks (`overflow || IsZero`) belong in `dcroxide-dcrec` under the same rule, so the `hdkeychain` crate never touches `k256` types directly.

Rust API consequences of §3.11 for the port:

- `pubKeyBytes`' memoization (keyed on `len(pubKey) == 0`) has no observable effect on any returned value; after `Zero` the key is public and the memo is no longer consulted. With eager or `OnceLock` computation there is nothing to reproduce.
- `paddedAppend(32, …)` on serialization and `PrivKeyFromBytes` on public-key computation are the two places a stripped key is re-padded; the Rust type that stores 32 bytes plus a stripped flag (or a stored length) gets both for free and must apply the stored length only when building the hardened preimage and when returning `serialized_priv_key`.
- `Depth()` returns `uint16`, `ChildNum()` `uint32`, `ParentFingerprint()` `uint32` read big-endian; dcrwallet calls none of the three (§3.9), so they exist for API parity and tests only.

Parity gaps of `dcr-rs:src/hd.rs` that rule out using it as is, collected from §5.1, §5.3 and §8.1:

| # | Gap | dcrd | dcr-rs |
| ---: | --- | --- | --- |
| 1 | Curve backend | pure Go | libsecp256k1 C |
| 2 | Key type on parse | from key data byte | from version prefix; mismatches rejected |
| 3 | Expected network | parameter; `ErrWrongNetwork` | detected among four built-in networks |
| 4 | Error identities | eight sentinels | `Derivation`, `Parse`, `Base58`, `BadChecksum`, `UnknownPrefix`, `HardenedFromPublic`, `HardenedIndex` |
| 5 | Length gate | 113 characters | 128 characters |
| 6 | Depth | `uint16`, serialized mod 256 | `u8`, error at overflow |
| 7 | Zero private sum | returned (stripped to empty) | error |
| 8 | Infinity public sum | returned | error |
| 9 | Single key type | yes | two types |
| 10 | `SerializedPrivKey` stored length | observable | not exposed |
| 11 | `Zero`, `zeroed extended key` | yes | `Drop` only |
| 12 | `GenerateSeed`, `RecommendedSeedLen` | yes | no |
| 13 | Legacy coin type, custom networks | via parameters | no |
| 14 | `account_key` / `address_key` helpers | none (the wallet composes `Child` calls) | present, SLIP-0044 only |

No gap affects a derived key value for a non-degenerate input: on that, dcr-rs matches dcrd in every combination the vectors cover.

### 12.4 What the wallet needs that neither has: seed encoding

Suggested crate: `purse-walletseed`, with the PGP word table as a `static [&str; 512]` copied in table order from `pgpwordlist/wordlist.go`, a lower-cased lookup map built once, and the five functions of §3.10. Dependencies: `sha2`, `dcroxide-crypto` (`rand`), the hdkeychain crate (seed bounds and `ErrInvalidSeedLen`), the wallet `errors` crate. Go's `strings.TrimSpace` trims Unicode white space and `strings.ToLower` is Unicode-aware; the table is ASCII, so ASCII lower-casing of the *table* side is exact, but the *input* side must use the same Unicode trimming as Go to keep error positions and echoed words identical (the echoed word is the trimmed input).

### 12.5 Other wallet needs outside both projects (pointers)

Listed so the dependency ledger starts complete; each is specified elsewhere.

| Need | dcrwallet source | Candidate | Status |
| --- | --- | --- | --- |
| scrypt | `golang.org/x/crypto/scrypt` (spec 02) | RustCrypto `scrypt` | UNVERIFIED: not in the local registry; check pure Rust, no `asm!`, digest-family version |
| XSalsa20-Poly1305 secretbox | `golang.org/x/crypto/nacl/secretbox` (spec 02) | RustCrypto `crypto_secretbox` or `xsalsa20poly1305` | UNVERIFIED; `poly1305` 0.9.1 is already in dcroxide's lock |
| Argon2id | `golang.org/x/crypto/argon2` (spec 02) | RustCrypto `argon2` | `argon2` 0.5.3 depends on `blake2` 0.10.6, which depends on `digest` 0.10 (local registry manifests). With dcroxide on `digest` 0.11 this is a duplicate major that `multiple-versions = "deny"` refuses unless skipped or a `digest` 0.11 release line of both exists (UNVERIFIED). |
| BLAKE2b | `golang.org/x/crypto/blake2b` (spec 02) | RustCrypto `blake2` | as above |
| sntrup4591761 | `companyzero/sntrup4591761` (mixing key agreement, spec 12) | none known | UNVERIFIED that no pure-Rust crate implements the 4591^761 parameter set (as opposed to sntrup761). The Go module is 1,287 lines of Go including its test file and three example programs; plan an in-tree port with Go-generated vectors. `dcroxide-mixing` defers it explicitly (`dcroxide:crates/dcroxide-mixing/src/lib.rs:8-11`). |
| bbolt file format | `go.etcd.io/bbolt` v1.3.12 (spec 01) | none in dcroxide (`redb`, with a fresh format) | in-tree port |
| HTTP/2, gRPC, protobuf | grpc-go, protobuf-go | §12.7 | decision D-3 |
| websocket JSON-RPC client | `jrick/wsrpc` v2.3.8 (spec 15) | dcroxide has server-side frames only (`dcroxide:crates/dcroxide-node/src/wsframe.rs`, 772 lines) | extend in tree |
| root solver | `decred.org/cspp/v2` `solverrpc` runs an external `csppsolver` process (spec 13) | external process, not linked | outside the pure-Rust rule's link scope; confirm under D-5 |

### 12.6 Conventions: inherit, decide, or not applicable

| Convention | Source | Class | Note for the wallet |
| --- | --- | --- | --- |
| Upstream source at one pinned commit is the specification; docs are secondary; quirks reproduced and recorded | brief principle 1; `QUIRKS.md` | inherit | Pin: dcrwallet `54b85b33`. |
| ADR process and template | `docs/adr/README.md`, `template.md` | inherit | Number wallet ADRs independently; cite dcroxide ADRs by number where inherited. |
| Oracle rig (Go binary, line JSON, pinned modules, mandatory in CI) | ADR-0001 | inherit | The oracle links dcrwallet packages and the dcrd modules at the versions dcrwallet's `go.mod` requires. dcrwallet's `internal/` packages cannot be imported by an external module; exporters for those run as overlays inside a dcrwallet checkout, as dcroxide's kept generators do. |
| Frozen-battery regeneration rule; exporters kept under `tools/` | ADR-0001 addendum 2026-10-26 | inherit | Keep every exporter from day one; dcroxide's cost came from scratch exporters. |
| One reference Go release | ADR-0001 second addendum 2026-10-26 | inherit, with a decision | Which release: D-9. |
| BLAKE-256 owned in `dcroxide-crypto` | ADR-0002 | inherit | By depending on the crate. |
| Later imports from dcr-rs need their own decision | ADR-0002 addendum | inherit | Triggers D-4. |
| Slice-based decoding, `(value, consumed)`, error-text parity, round-trip invariant, `no_std + alloc` primitives | ADR-0003 | inherit | Applies to database value codecs as well as wire codecs. |
| Storage: `redb`, fresh on-disk format, no in-place reuse | ADR-0004, 0009, 0010, 0011 | **not applicable** | The wallet's on-disk format is fixed by existing `wallet.db` files. Only ADR-0009's guardrails carry over: no multi-backend trait layer, durable defaults with any deferred-sync mode opt-in, crash tests that fail a wrong design, big-endian range keys (`0009-…md:771-791`). |
| No async runtime; OS threads over channels; runtime-free core; single writer | ADR-0005 | **decide** (D-1) | Holds without question for database, key, transaction and sync-logic crates. Open for the HTTP/2 server and the mixing client. |
| `panic = "abort"` in release; supervisor required | ADR-0005 addendum; `Cargo.toml` | inherit, with review | For a wallet an abort interrupts signing, ticket purchase or a mix round; correctness then rests on the database's atomic commits. Confirm against spec 01. |
| `overflow-checks = false` in release, suite re-run with checks off | `Cargo.toml:83-94`; CI `test-wrapping` | inherit | |
| secp256k1: secrets on `k256`, public arithmetic on the owned vartime crate, dcrd's rules in dcrec | ADR-0016, ADR-0017 | inherit | The wallet is a far heavier user of the constant-time (secret) side than the node: derivation, signing, mixing. The gate soak covered signing and verification, not scalar tweak-add; the additions of §12.3 need their own differential coverage. |
| No serde on the RPC path; Go `encoding/json` over `GoType`/`GoValue`; byte-for-byte vectors | ADR-0007 | inherit for JSON-RPC; **decide** for protobuf (D-3) | `dcroxide-dcrjson` and `dcroxide-rpctypes` are reusable; wallet command and result types are new registrations. |
| Lint set, refusals, arithmetic policy, `#[expect]` migration | ADR-0008 | inherit | The 2026-09-26 addendum's rules for `/`, `%` and shifts apply to fee, amount and vote arithmetic. |
| Hash containers are not converted to ordered ones for "determinism"; an arbitrary pick is an explicit random draw | ADR-0008 | inherit | Coin selection and address selection sites need this review. |
| Pure Rust; two exceptions (the Go oracle, libFuzzer) | ADR-0012 | inherit; **decide** per new dependency (D-3, D-5, D-7) | |
| Enforcement: `cargo-deny` bans, no-C-compiler job | ADR-0012 | inherit | Copy `deny.toml`; the `blake3` wrapper entry is only needed if `dcroxide-wire` is linked with its header hash. |
| Pin follows upstream release tags; master read ahead | ADR-0013 | inherit, adapted | dcrwallet has its own release line; the wallet has two upstreams to track (dcrwallet, and the dcrd modules it requires). D-10. |
| MSRV equals toolchain pin; point releases only | ADR-0014 part 1; `rust-toolchain.toml` | inherit | Use the same pin as dcroxide while sharing crates. |
| Release gate, tiers, signing, reproducibility | ADR-0014 parts 2–5 | inherit, with wallet conditions | The four gate conditions become: pure-Rust rule met, key and signing differential soak passed, pin at a dcrwallet release tag, reproduction prototype passed. Tier bars need a wallet equivalent of "one mainnet sync timed". |
| In-tree TLS provider; no process-default provider; `clippy.toml` refusals | ADR-0015 | inherit | The wallet serves TLS (JSON-RPC, gRPC) and dials TLS (dcrd RPC, VSP HTTPS). The Go-style client-certificate verifier lives in `dcroxide-node` (`rpcrun.rs`), not in a library crate. HTTP/2 additionally requires ALPN `h2`. |
| Dependency ledger and assembly table | `dependency-ledger.md` | inherit | New rows at minimum: KDF and AEAD crates, KEM, protobuf/HTTP/2 stack or their in-tree replacements, the bbolt port. |
| `SECURITY.md` shape | `SECURITY.md` | inherit | State pre-alpha status; a wallet's "do not use with funds" warning is the headline. Private reporting channel to be chosen. |
| `PARITY.md` and `QUIRKS.md` | §4.10 | inherit | The package table maps dcrwallet packages (and the Go dependencies it links that dcroxide does not port) to crates. The 16+ specs' "Quirks" sections seed `QUIRKS.md`. |
| `_typos.toml` | — | inherit | Start with the exclusions and the base58 pattern; add dcrwallet's own misspellings as specs find them. |
| CI matrix (four OS/arch test legs, lint, wrapping, interop, go-tools, deny, docs, typos, no-std, no-C, fuzz) | `ci.yml` | inherit | Interop legs: against dcrd (RPC and SPV), against Go dcrwallet (same `wallet.db`, mixing peers), against a VSP. |
| Constant-time workflow | `constant-time.yml` | inherit, extended | Add HD private derivation, the KDFs' key handling and the KEM. |
| `dcroxide-winsvc` as the single audited `unsafe` exception | ADR-0008 addendum 2026-09-25 | inherit if needed | dcrwallet's Windows service and pipe handling are in the CLI/lifecycle spec. |
| One crate per upstream package | brief §5; `README.md` Layout | inherit | |
| Node-specific addenda (read-ahead thread, script validation pool, commit thread, exists-address index) | ADR-0005 addenda from 2026-10-11; ADR-0011 | not applicable | |
| Seven engineering principles; definition of done | project brief §4, §12 (§5.6) | inherit | Principle 6 (denial-of-service limits verbatim) applies to the SPV peer code, the mixing client and both RPC servers. The "commit the project owner signs" clause stands in for two reviewers on a single-maintainer project. |
| Lints refused, adopted and deferred | ADR-0008 (`0008-…md:110-216`) | inherit | Refused: `cast_possible_truncation` (an integer `as` is the transcription of a Go conversion; `try_from().expect()` would turn a faithful truncation into an abort). Adopted: `iter_over_hash_type`, `allow_attributes` (suppressions written as `#[expect]`), `unreachable_pub` (a rustc lint, not a clippy one). Deferred: `cast_sign_loss`, `cast_possible_wrap` (`.cast_signed()` / `.cast_unsigned()`). Per-crate ratchets are `#![deny(...)]` in `src/lib.rs` (§10 item 36). |
| Float-to-integer conversions audited by hand | ADR-0008 (`:205-216`) | inherit | Rust saturates where Go on amd64 yields `i64::MIN`. dcrwallet calls `dcrutil.NewAmount` at 21 sites outside tests, in `internal/rpc/jsonrpc/methods.go`, `internal/rpc/rpcserver/server.go`, `internal/cfgutil/amount.go`, `rpc/client/dcrd/calls.go`, `rpc/client/dcrwallet/marshaling.go` and two `cmd/` tools (tree search at the pin). Each site is range-checked by its caller or is a `QUIRKS.md` entry. |
| TLS provider limits that become wallet behaviour | ADR-0015 T4, T6, Known limits (`0015-…md:279-331`, `:368-384`) | inherit, with review | An RSA server key is refused at start-up and on rotation, where Go accepts it; Go's four TLS 1.2 CBC suites cannot be offered under rustls; client-certificate verification is to be widened to Go's set (T4 adds P-521 keys, ECDSA with SHA-512 and RSA keys from 1,024 bits to what the earlier provider verified; the ADR index lists it as later work, so its state at the commit the wallet takes must be checked). Whether dcrwallet users run with RSA `rpc.key` files, and what the wallet's own certificate generation produces, are for the TLS/CLI spec to state; each refusal that remains is a `PARITY.md` divergence. |
| Trust roots for outbound HTTPS | ADR-0015 "Not decided here" (`:385-392`) | **decide** | dcroxide's seeder client trusts the compiled-in `webpki-roots`, where Go uses the platform store. The wallet's VSP client is an outbound HTTPS client to arbitrary operator hosts; the same question applies with more weight. |
| `ureq` pinned exactly while its provider hook is used; `clippy.toml` refuses its default-agent entry points | ADR-0015 T7; `clippy.toml:24-36` | inherit if `ureq` is used | The VSP client and any other outbound HTTP must name the in-tree provider explicitly. |
| Fuzzing as a separate nightly workspace; per-target smoke on every push, long runs nightly, corpus restored from the nightly runs; oracle-backed targets stop rather than run without the oracle | `ci.yml:563-700`; `README.md:820-830` | inherit | First wallet targets: database value decoders, the bbolt page reader, protobuf and HTTP/2 frame decoders, JSON-RPC request parsing, seed and extended-key parsing, the mixing message decoders. |
| Bench ledger: rows appended and never rewritten, each naming machine, commit and workload; prose elsewhere cites rows | `docs/bench-ledger.md:1-22` | inherit | Wallet workloads: rescan, address discovery, transaction creation with many UTXOs, ticket purchase, database open on a large `wallet.db`. |
| Operator document; a test that fails if `panic = "abort"` is dropped from the release profile | `docs/operating.md:13-25` | inherit | The wallet's operator document must state the supervisor requirement and what an abort during a write leaves on disk. |
| Third-party test vectors committed with a `SOURCE` record (commit, size, SHA-256, git blob hash per file) and pinned case counts | `docs/dependency-ledger.md`, "Test vectors taken from elsewhere" | inherit | Applies to any Wycheproof or reference vectors for scrypt, Argon2id, XSalsa20-Poly1305 and the KEM. |
| `cargo-vet` deliberately not adopted; vendoring declined on size for every listed crate | `docs/dependency-ledger.md`, "What is deliberately not here" | inherit | Revisit only with a second maintainer. |
| Ledger rows the wallet inherits by linking dcroxide crates | `docs/dependency-ledger.md:40-59` | inherit | `k256`, `dcroxide-secp256k1-vartime`, `curve25519-dalek`, `blake3` (only if the block-header hash is linked), `dcroxide-tlscrypto` with `aes-gcm`, `chacha20poly1305`, `ml-dsa`, `rustls`, `getrandom`, `p256`/`p384`/`p521`, `sha2`/`hmac`, `siphasher`, `num-bigint`, `windows-sys`, `flate2`. `redb` and `ureq` come only if the wallet links the crates that use them. The wallet's ledger restates the rows for what it links and adds its own. |
| Release tiers | ADR-0014 part 2 (`0014-…md:159-194`) | inherit, adapted | Tier 1 x86_64 Linux (binary); Tier 2 aarch64 Linux, arm64 macOS, x86_64 Windows (CI-tested, built from source); Tier 3 everything else. The node's promotion bar includes "Decrediton launches, supervises and cleanly stops the daemon over IPC"; for the wallet that check is central rather than incidental, since Decrediton drives dcrwallet over gRPC and the pipe IPC. |
| Release assets never replaced; `SHA256SUMS` never regenerated; tags made by hand and never by a workflow; signing keys never in CI | ADR-0014 part 3 (`:196-220`) | inherit | |
| No expiry for stale binaries | ADR-0014 part 5 (`:267-304`) | inherit, with review | Recorded as a refusal argued from the node's threat model (a voting wallet behind a wedged node misses votes). The wallet's own argument has to be written: an expired wallet binary would stop voting and ticket purchase directly. |

### 12.7 gRPC: what a wire-compatible server needs, and the pure-Rust options

Required functionality, from §4.9:

1. TLS 1.2/1.3 server with ALPN `h2` and mandatory client-certificate verification (rustls with `dcroxide-tlscrypto`).
2. An HTTP/2 server: connection preface, SETTINGS exchange, HEADERS/CONTINUATION/DATA/RST_STREAM/PING/GOAWAY/WINDOW_UPDATE handling, stream multiplexing, stream and connection flow control, HPACK decoding (including Huffman and the dynamic table) and encoding.
3. gRPC over HTTP/2: path routing, the 5-byte message framing, `grpc-timeout` parsing and deadline propagation, `grpc-encoding` negotiation (identity only), response headers, status trailers and trailers-only responses, percent-encoded `grpc-message`, cancellation on RST_STREAM, the four call shapes (unary, server-streaming, and one bidirectional method), keepalive pings and the ping-strike policy.
4. A protobuf codec for the proto3 subset `api.proto` uses: varints, zigzag, 64-bit fixed, length-delimited strings/bytes/messages, packed and unpacked repeated fields, enums, unknown-field skipping on decode.

What must match byte for byte is the *content* a client parses: message payloads, the framing prefix, header and trailer names and values. HPACK encoding choices, frame boundaries, SETTINGS values and WINDOW_UPDATE timing are not required to equal grpc-go's; any compliant client decodes them the same. They become relevant only where a client depends on a limit (§9 item 16).

Options:

| Option | Fits ADR-0005 (no async) | Fits ADR-0007 (no serde, runtime type model) | Fits ADR-0012 (pure Rust) | Notes |
| --- | --- | --- | --- | --- |
| A. `tonic` + `prost` on `tokio`/`hyper`/`h2` | no | n/a (derive-based structs, not serde) | UNVERIFIED for the full dependency tree. `h2` 0.4.20 requires `tokio` and `tokio-util` (its manifest in the local registry). `prost-build` is commonly driven by the `protoc` binary, a C++ tool run at build time (UNVERIFIED for the version that would be used); checking generated code in, or a pure-Rust compiler such as `protox`, avoids it (UNVERIFIED). | Least code. Brings an async runtime and roughly a hundred transitive crates into a wallet process (count UNVERIFIED). Whether `tonic`'s rustls integration accepts an explicit provider without touching the process default is UNVERIFIED and is what `clippy.toml` guards against. Whether `prost` emits fields in field-number order where the message declares them out of order is UNVERIFIED. |
| B. `h2` + in-tree gRPC layer + in-tree protobuf codec | no (tokio at the edge only) | yes | as A for `h2`/`tokio` | Confines async to one crate behind a channel boundary; still an ADR-0005 exception. |
| C. In-tree HTTP/2 (sans-I/O state machine driven by threads), HPACK, gRPC layer and protobuf codec | yes | yes | yes | Consistent with dcroxide writing its own HTTP/1.1 server, websocket framing and TLS provider. Largest effort and the largest new attack surface the project would own; needs h2spec-style conformance testing and interop against grpc-go, grpc-node (Decrediton) and `grpcurl`-class clients. |

For the protobuf codec, an in-tree table-driven encoder over a runtime message model (the protobuf analogue of `GoType`/`GoValue`) is recommended under every option: the subset is small, byte-exactness is then testable against protobuf-go's own output, and no build-time code generator is needed. The HTTP/2 choice is D-3.

### 12.8 Hazards

1. **Module-version skew between dcroxide and dcrwallet.** dcroxide ports dcrd *master* at `46e9df48`; dcrwallet links *tagged* module versions (`wire` v1.8.0, `txscript/v4` v4.1.2, `gcs/v4` v4.1.1, `blockchain/stake/v5` v5.0.3, `mixing` v0.7.5, `dcrec/secp256k1/v4` v4.4.1, `chaincfg/v3` v3.3.0, `dcrjson/v4` v4.2.0, `rpc/jsonrpc/types/v4` v4.4.0, `addrmgr/v3` v3.0.0, `connmgr/v3` v3.1.4, `certgen` v1.2.0). dcroxide's own record shows the two can differ: its oracle pins a module by pseudo-version whenever the in-tree source differs from the tag (`dcroxide:docs/adr/0001-oracle-driven-differential-testing.md:58-81`, `:120-141`). Each reused crate must be checked against the module version dcrwallet requires, not assumed equal (D-10).
2. **Node-internal modules are not library crates.** The go-flags/INI front end, logging and log rotation, the HTTP server, websocket framing, TLS negotiation as Go's server performs it, the client-certificate verifier, SOCKS dialing, secret-file handling and pipe IPC are modules of `dcroxide-node` (`dcroxide:crates/dcroxide-node/src/lib.rs:32-76`). Reusing them means extracting crates or depending on the node crate. dcroxide's `flags` module ports go-flags v1.6.1 (`dcroxide:crates/dcroxide-node/src/lib.rs:5-6`); dcrwallet pins go-flags v1.5.0.
3. **dcroxide crates are not published and `deny.toml` refuses unknown git sources.** How the wallet consumes them is D-2.
4. **`PrivateKey::from_bytes` strictness** (§10 item 20).
5. **Secret hygiene.** dcroxide's primitives were tuned for a node that holds no user secrets: its BLAKE-256 does not wipe, and `dcroxide-base58` makes no wiping or constant-time claim (not examined). The wallet's hdkeychain serialization, WIF handling and seed display pass secrets through both.
6. **`unsafe_code = "forbid"` and memory locking.** Go dcrwallet does not lock memory either, but any future wish for `mlock` or guard pages collides with the workspace lint; decide before, not after.
7. **Two readings of "reference Go".** dcroxide's reference is the Go that dcrd's release image builds with (1.27.3 at its pin); dcrwallet's `go.mod` says 1.26.0. Go-JSON error texts differ between Go lines. At the commit read `dcroxide-dcrjson` does not yet follow dcroxide's own reference for them: its syntax-error and type-error texts, its quoting of non-ASCII bytes and its handling of invalid UTF-8 are still Go 1.26.2's, recorded as an interim divergence until the batteries that pin them are regenerated under 1.27.3 (`dcroxide:docs/adr/0007-json-emission-strategy.md:245-265`). Which Go line the crate's texts follow at the commit the wallet takes must be checked against the wallet's reference (D-9).

### 12.9 Size estimates

| Piece | Size | Reasoning |
| --- | --- | --- |
| `hdkeychain` port, with the four `dcroxide-dcrec` additions and oracle commands | S | 636 lines of Go, one algorithm, existing vectors; the work is error parity and the stored-length state. |
| `walletseed` + `pgpwordlist` port | S | 160 lines of Go plus a 512-word table. |
| Workspace bootstrap: manifests, lints, `deny.toml`, toolchain pin, CI workflows, oracle skeleton, PARITY/QUIRKS/ledger/SECURITY files, first ADRs | M | Mostly adaptation, but the oracle overlay mechanism for dcrwallet's `internal/` packages is new. |
| Extraction of shared node modules into library crates | M | No new behaviour; mechanical, but touches dcroxide. |
| In-tree protobuf codec with golden vectors | S–M | Small subset; one exporter. |
| gRPC transport, option A | M | Integration, provider wiring, interop tests. |
| gRPC transport, option C | L–XL | HTTP/2 and HPACK are specifications of their own; conformance and interop testing dominate. |
| sntrup4591761 port | M | 1,287 lines of Go with tests; constant-time review required. |

This document's own subject (the assessment and the two small ports) is **S**; the decisions it surfaces gate work that is not.

## 13. Open questions

### 13.1 Decisions needed

- **D-1 Concurrency model at the edges.** Keep ADR-0005 unchanged (threads everywhere, in-tree HTTP/2), or permit a contained async runtime for the gRPC server (and possibly the websocket client and mixing client) behind a channel seam with runtime-free core crates. Decide before the gRPC and mixing work is scheduled.
- **D-2 How dcroxide crates are consumed.** Same workspace, sibling checkout with path dependencies, an allow-listed git source in `deny.toml`, or publication. Also whether node-internal modules (§12.8 item 2) are extracted into shared crates.
- **D-3 gRPC/protobuf stack.** Options A, B or C of §12.7; and, independently, in-tree protobuf codec (recommended) or generated `prost` types.
- **D-4 `hdkeychain` implementation and the import from dcr-rs.** Recommended: a new crate over `dcroxide-dcrec` with dcrd's parse and error behaviour, taking dcr-rs's stored-stripped design and vectors under a recorded one-time import (ADR-0002 addendum). Alternative: generalize dcr-rs behind a curve-backend trait and loosen its parsing; this changes dcr-rs's signer-oriented guarantees and its public API.
- **D-5 KDF and AEAD crates** (scrypt, Argon2id, XSalsa20-Poly1305, BLAKE2b): accept RustCrypto crates with ledger rows, resolving the `digest` 0.10/0.11 duplication, or port from Go. Includes confirming that an external `csppsolver` process is outside the pure-Rust rule.
- **D-6 Degenerate derivation cases.** Reproduce dcrd's unchecked zero sum, unchecked point at infinity and depth wrap (§10 items 4, 5, 7), or return errors and record three divergences in `PARITY.md`. Unreachable in practice except the depth wrap, which needs a depth-255 imported key.
- **D-7 sntrup4591761.** In-tree port (expected) and its assurance: Go-generated vectors, constant-time check.
- **D-8 Zeroization policy.** Whether wallet code paths use a wiping BLAKE-256 (restore dcr-rs's `Drop` behind a feature of `dcroxide-crypto`, or a wallet-side wrapper), and a project-wide rule for buffers that hold seeds, private keys and passphrases.
- **D-9 Reference Go release for the wallet** and whether it may differ from dcroxide's while both share `dcroxide-dcrjson`.
- **D-10 Pin policy with two upstreams.** The wallet pin (dcrwallet commit or release tag) and the rule for dcrd module versions: verify each reused dcroxide crate against the module version dcrwallet's `go.mod` requires, and record per crate whether it is identical, or differs and how.
- **D-11 dcr-rs `airgap` and BIP39.** Confirm both are out of scope for the drop-in replacement. Offline or hardware signing through dcr-rs's format would be a new feature with no Go dcrwallet counterpart.
- **D-12 Naming.** Crate prefix, environment-variable prefix for the oracle and fault-injection switches, and whether new shared crates (hdkeychain) live in the dcroxide workspace under its prefix.
- **D-13 Release gate wording for the wallet** (ADR-0014 part 2 adapted, §12.6).

### 13.2 Unverified, with what would settle each

1. **proto3 zero-value omission and packed encoding as emitted by protobuf-go v1.36.11 for `api.proto`'s messages** (§4.9). Settle with the golden-bytes exporter of §11.4.
2. **Properties of crates not present in the local registry** (`tonic`, `prost`, `protox`, `scrypt`, `crypto_secretbox`/`xsalsa20poly1305`, any sntrup4591761 crate, and newer `argon2`/`blake2` lines): purity, `asm!` sites, build scripts, dependency majors. Settle by reading each crate's source at the candidate version and adding ledger rows.
3. **Which Go patch release dcrwallet's release builds use.** Settle from dcrwallet's release tooling or the `go version -m` output of a released binary.
4. **Whether each tagged dcrd module dcrwallet requires is identical to dcroxide's ported source at its pin** (§12.8 item 1). Settle with a per-module diff between the module-cache directory and dcrd at `46e9df48`.
5. **dcr-rs's 200,000-seed comparison against dcrd** (`dcr-rs:src/hd.rs:27-31`) is reported, not reproducible from the repository. Settle by re-running it through the wallet oracle.
6. **`bs58`'s purity and constant-time properties** were not examined; moot if the wallet uses `dcroxide-base58`.
7. **The sections of the dcroxide ADRs read by heading and index summary only** (§1): the measurement addenda of ADR-0004/0009/0010/0011, the soak mechanics of ADR-0006/0016, and the later addenda of ADR-0001, ADR-0005 and ADR-0015. The ADR index (`dcroxide:docs/adr/README.md:9-25`) summarizes every addendum and was read in full. None is expected to change a classification in §12.6, but a reader adopting a convention from one of them should read it in full first.
8. **Whether any dcrwallet RPC exposes `ExtendedKey.Depth()` or a depth-255 derivation** (§9 item 8). Settle in the JSON-RPC and gRPC method specs.
9. **The formatted seed-decode error strings of §8.2** were derived by reading `errors.E` and `(*Error).Error`, not by running them. Settle with the `seed_decode` oracle command, run with `errors.Separator` set as the daemon sets it.
10. **The expected dcrd results for dcr-rs's narrowing tests** (§11.3: `dprv`, `dprv0OIl`, the overlong string, the swapped-version strings) were derived by reading `NewKeyFromString` and `base58.Decode`'s documented behaviour. Settle with `hd_from_string`.
11. **The behaviour of a zeroed key under `Child`** (§10 item 28) was derived by reading; `TestZero` does not cover it. Settle with an oracle command only if the port keeps a `zero` operation at all.
12. **Whether ADR-0015's T4 (Go's client-certificate set) is built at the dcroxide commit the wallet takes.** The ADR index names it as work after the switch. Settle by reading `dcroxide-tlscrypto`'s verification module and `PARITY.md`'s client-certificate entries at that commit.
13. **dcroxide documents not read beyond headings**: `docs/operating.md`, `docs/bench-ledger.md`, and the project brief outside its §2.1, §4 and §12. Their role as conventions is recorded (§3.13, §12.6); their content is node-specific and was not checked.
14. **dcr-rs's airgap test file** (`dcr-rs:tests/airgap_review.rs`, 29 tests) was indexed by test name, not read line by line; the airgap module's code and every error message were read (§5.7, §8.6).
