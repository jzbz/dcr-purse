# 24 — Reuse assessment: dcrd modules used by dcrwallet versus dcroxide crates

## 1. Header

**Scope** (line counts from `wc -l`)

| Source | Lines | How it was read |
|---|---|---|
| dcrwallet, every non-test `.go` file outside `testdata/` (135 files) | 83,418 | import blocks parsed mechanically; every `pkg.Ident` selector collected per imported dcrd module; method calls matched against each module's exported method set; cited call sites read |
| dcrwallet `_test.go` files (43 files) | — | same scan, reported separately (section 11) |
| dcrwallet `cmd/` (4 tools) | — | same scan, reported separately |
| `go.mod` | 69 | all |
| `dcrd/mixing@v0.7.5`, non-test files | 7,336 | import scan of every file (`mixclient/client.go` 2,606, `mixpool/mixpool.go` 1,935, `mixpool/observer.go` 525, `mixclient/blame.go` 493, `mixclient/coinjoin.go` 295, `keyagreement.go` 237, rest under 180 each); exported surface listed; `mixpool.go:33-51`, `:1669-1700` read |
| `dcrd/hdkeychain/v3@v3.1.3/extendedkey.go` | 636 | all |
| `dcrd/dcrutil/v4@v4.0.3`: `amount.go`, `wif.go`, `appdata.go`, `bitflags.go`, `const.go`, `hash160.go`, `txsort/txsort.go` | 145, 198, 99, 61, 17, 24, 138 | all |
| `dcrd/dcrec/secp256k1/v4@v4.4.1`: `privkey.go:42-86`, `ecdh.go` | — | the cited functions |
| `dcrd/dcrec/edwards/v2@v2.0.4/privkey.go:122-152` | — | `PrivKeyFromScalar` |
| `dcrd/addrmgr/v3@v3.0.0/addrmanager.go` | — | exported surface, `:100-135`, `:480-540`, `:760-835` |
| `vspd/client/v4@v4.0.2/client.go` | 272 | lines 1-260 |
| dcroxide `Cargo.toml`, `deny.toml`, `clippy.toml`, `rust-toolchain.toml` | 152, 106, 37, 19 | all |
| dcroxide `docs/PARITY.md` | 4,586 | lines 1-140 and the ledger table, rows `:1009-1045`, for every module in this scope; the gap lists after `:1129` were not read |
| dcroxide `docs/adr/README.md`, `docs/dependency-ledger.md`, `tools/oracle/go.mod`, `.github/workflows/ci.yml` (job list, `no-std` job) | — | the parts cited |
| dcroxide crates: `Cargo.toml` and `src/lib.rs` of each crate in section 3, plus the public item list (`pub fn/struct/enum/trait/const`) of each module named there | wire 7,348; chaincfg 5,202; chainhash 416; txscript 12,261; stake 8,660; standalone 1,565; gcs 1,792; mixing 4,258; addrmgr 3,456; connmgr 3,096; dcrec 5,573; crypto 1,037; dcrjson 5,115; rpctypes 2,125; certgen 2,169; base58 252; containers 1,230 | public surface listed in full; function bodies read only where cited |
| dcr-rs `Cargo.toml`, `src/lib.rs` (140), `src/hd.rs` (800), `src/amount.rs` (40) | — | `hd.rs:1-200`, `:276-380`, `:440-545`; public item list of every module |
| Completeness pass: `dcrd/mixing@v0.7.5/mixpool/mixpool.go:28-56`, `:105-125`, `:228-306`, `:895-925`, `mixpool/errors.go`, `mixpool/observer.go:60-145`, `keyagreement.go:20-110`, `mixclient/coinjoin.go` declarations; `dcrutil/v4@v4.0.3` declarations of every non-test file (`block.go`, `tx.go`, `util.go` included); `go-socks@v1.1.0/socks` (4 files) declarations and `pool.go`; `sntrup4591761` declarations; `vspd/client/v4@v4.0.2/client.go` declarations; `p2p/peering.go:345-431`; dcroxide `Cargo.toml` of 22 crates, `dcroxide-addrmgr/src/seed.rs:17-135`, `dcroxide-mixing/src/mixpool.rs:445-760`, `:2686-2706`, `dcroxide-gcs/src/blockcf2.rs` declarations, `dcroxide-node/src/socks.rs:1-34`, `Cargo.lock` package names, `.github/workflows/ci.yml` job list, `docs/PARITY.md:1097`, `:1108-1112`, `:1123` | — | the parts cited; inventories rebuilt by script (every `pkg.Ident` selector per import in non-test, test and `cmd/` files; every exported method of each module matched against call sites) and diffed against this document |
| Verification pass: the six `dcroxide-wire` gaps, `dcroxide-crypto`'s `rand` globals, `PrivateKey::from_bytes`/`secret_bytes`, `sign.rs:92-99`, `certgen::Curve` against `gentool::ToolKeyPair`, `deny.toml`, each crate's `[dependencies]`, `hdkeychain` `extendedkey.go:95-130`, `:200-350`, `:500-540`, `dcrutil` `wif.go:30-165`, `wire@v1.8.0/msgtx.go:1123-1158`, `go-socks@v1.1.0/socks/conn.go` and `dial.go:225-285`, `p2p/peering.go:205-242`, and `dcrd@v1.2.1-0.20261106175134-46e9df48109a/go.mod` (dcrd's root module at the pin) | — | re-read against this document; section 4.1 and open question O-8 amended from the last |
| Second completeness pass: struct-field inventory (every exported field of each dcrd struct type matched against `.Field` selectors in dcrwallet's non-test files); method inventory repeated per module; declaration inventory (every `func`, `type`, `const`, `var`) of `hdkeychain/v3@v3.1.3`, `dcrutil/v4@v4.0.3`, `mixing@v0.7.5` root, `utxoproof`, `go-socks`, `connmgr/v3` `dynamicbanscore.go` and `tor.go`, `crypto/rand`, `dcrjson/v4@v4.2.0/jsonrpc.go`; `dcrec/secp256k1/v4@v4.4.1/ecdsa/signature.go:630-735`, `nonce.go:117-141`, `modnscalar.go:349-368`; `wallet/wallet.go:1975-2065`, `internal/rpc/rpcserver/server.go:3736-3772`, `internal/rpc/jsonrpc/server.go:360-460`, `:553-600`, `methods.go:281-306`; dcroxide `dcroxide-wire` struct definitions, `dcroxide-chaincfg/src/lib.rs:55-90`, `dcroxide-dcrjson/src/jsonrpc.rs:170-300`, `dcroxide-rpc/src/http.rs:130-160`, `:300-310`, `dcroxide-dcrec/src/secp256k1/ecdsa.rs` and `schnorr.rs` public items, `dcroxide-mixing/src/dcnet.rs` public items; `docs/PARITY.md` after `:1129` by keyword search only | — | additions marked in sections 3.1, 3.3, 3.7, 3.9, 3.14, 3.15, 3.17, 3.19, 3.29, 8, 9, 10, 11, 12.3, 13 and 14 |

**Parity pin**

| Component | Version read |
|---|---|
| dcrwallet | master `54b85b33`, version 2.2.0-pre, module `decred.org/dcrwallet/v5`, `go 1.26.0` (`go.mod:3`) |
| dcrd modules | exactly the versions of `go.mod:5-65`; listed per module in section 4.1 |
| dcroxide | working tree at `8053abe` (2026-11-14); its parity pin is dcrd master `46e9df48` (`dcroxide:docs/PARITY.md:3-7`) |
| dcr-rs | `683d9a7` (release 0.6.3) |

**Summary**

dcrwallet requires 22 dcrd modules directly (`go.mod:7-28`) and four more
indirectly that matter here (`base58`, `container/lru`, `dcrec/edwards/v2`,
`database/v3`; `go.mod:50-53`). For most of them dcroxide already holds a
differential-tested Rust port whose public surface covers what the wallet
calls: `wire`, `chainhash`, `chaincfg`, `txscript` with `stdaddr`, `stdscript`
and `sign` (signing with private keys included), `blockchain/stake`,
`blockchain/standalone`, `gcs` with `blockcf2`, `dcrjson`,
`rpc/jsonrpc/types`, `base58`, `container/lru`, `crypto/blake256`,
`crypto/ripemd160`, `certgen`, and most of `dcrec`, `crypto/rand`, `addrmgr`
and `mixing` (with `mixpool` and `utxoproof`).
Four pieces the wallet cannot run without exist nowhere in dcroxide:
`hdkeychain`, `dcrutil` (WIF, `Amount`, `AppDataDir`, `txsort`),
`mixing/mixclient` with `mixing`'s key agreement (secp256k1 ECDH plus the
sntrup4591761 KEM), and a dcrd RPC client. `rpcclient` and `blockchain/v5` are
not linked into the dcrwallet binary at all (sections 3.12, 3.21). Of the
module versions dcrwallet pins, `wire`, `chainhash`, `dcrutil`, `dcrec`,
`dcrec/edwards`, `ripemd160`, `base58` and the code of `blockchain/stake` are
byte-identical to what dcroxide tracks; `chaincfg`, `blake256`, `crypto/rand`,
`secp256k1`, `gcs`, `txscript` and `standalone` differ only in comments,
refactors and one mainnet seeder host; `addrmgr` (v3 against v4) and `connmgr`
(v3 against dcrd's rewritten internal package) differ in API generation
(section 4.1). dcr-rs has a faithful model of hdkeychain's derivation quirk
but cannot be a dependency: it links the C libsecp256k1, which dcroxide's
`deny.toml` bans, and it deliberately diverges from dcrd in four places. The
recommended dependency form is one shared workspace (or a git dependency
pinned by revision) with a short list of additive changes to six dcroxide
crates and five new crates (section 12).

## 2. Role in the system

This document is not the specification of a runtime subsystem. It is the
inventory that decides, for each Go package dcrwallet imports from dcrd, where
its Rust counterpart comes from: an existing dcroxide crate used as is, an
existing crate extended, or new code. Every other spec in this directory names
dcrd identifiers (`wire.MsgTx`, `stdaddr.DecodeAddress`,
`hdkeychain.ExtendedKey.Child`); this one maps each of them to a Rust item or
records that none exists. The behaviour of the missing pieces that no other
spec owns (`hdkeychain`, `dcrutil`) is specified here in sections 4 and 5
because the implementing engineer needs it to write the crate.

Callers: the project plan (crate layout, phase ordering, staffing), and every
implementing engineer choosing an import.

What it relies on: the dcrwallet source at the pin, the module cache at the
versions `go.mod` names, the dcroxide working tree, its parity ledger and
decision records, and dcr-rs.

## 3. Public surface inventory

Conventions for the tables in this section:

- **Go identifier** is the selector as written in dcrwallet (`pkg.Ident`) or
  a method on a value of the package's type (`T.Method`).
- **Used in** lists the dcrwallet packages (directories) with a non-test,
  non-`cmd/` use. `.` is package `main` at the repository root.
- **dcroxide item** is the Rust path. `—` means nothing exists.
- **Status**: `ok` (present, same behaviour by dcroxide's own parity record),
  `shape` (present, different calling shape that the wallet must adapt to),
  `missing` (must be added or written).

Method-level rows come from matching `.Name(` against the exported method set
of the module's types; a method name shared with another type (`String`,
`Bytes`, `Serialize`) was confirmed by reading a call site when it is listed.

### 3.1 `github.com/decred/dcrd/wire` v1.8.0 → `dcroxide-wire`

Imported by 66 files. dcroxide-wire mirrors the same release
(`dcroxide:crates/dcroxide-wire/src/lib.rs:2-4`, `dcroxide:tools/oracle/go.mod`
requires `wire v1.8.0`). The crate is `no_std` + `alloc`, decodes from byte
slices through `Cursor` rather than `io.Reader`, and returns the number of
bytes consumed (`dcroxide:crates/dcroxide-wire/src/lib.rs:16-22`).

| Go identifier | Used in | dcroxide item | Status |
|---|---|---|---|
| `MsgTx`, `TxIn`, `TxOut`, `OutPoint` | nearly every package | `MsgTx`, `TxIn`, `TxOut`, `OutPoint` (public fields `ser_type`, `version`, `tx_in`, `tx_out`, `lock_time`, `expiry`; `msgtx.rs:160-296`) | ok |
| `NewMsgTx`, `NewTxIn`, `NewTxOut`, `NewOutPoint` | internal/rpc/jsonrpc, wallet, wallet/udb, rpc/client/dcrwallet | `MsgTx::default()` (version 1, full serialization, `msgtx.rs:298-311`); struct literals | shape |
| `MsgTx.AddTxIn`, `AddTxOut` | internal/rpc/jsonrpc, wallet | `tx.tx_in.push(..)`, `tx.tx_out.push(..)` | shape |
| `MsgTx.TxHash` | 43 call sites | `MsgTx::tx_hash` (`msgtx.rs:637`) | ok; infallible (quirk Q-11) |
| `MsgTx.CachedTxHash` | internal/rpc/jsonrpc (`methods.go:1519`) | — ; use `tx_hash` | shape |
| `MsgTx.Serialize`, `Bytes`, `SerializeSize` | many | `MsgTx::serialize`, `serialize_size`, `encode_into` (`msgtx.rs:544-593`) | ok |
| `MsgTx.Deserialize`, `FromBytes`, `BtcDecode` | many | `MsgTx::from_bytes(&[u8]) -> (MsgTx, usize)`, `MsgTx::decode(&mut Cursor)` (`msgtx.rs:326-360`) | shape (slice, trailing bytes allowed) |
| `MsgTx.PkScriptLocs` | wallet/udb (`txmined.go:1420`, `:1501`, `:1534`, `:2122`) | — | **missing** |
| `TxSerializeFull`, `MsgTx.SerType` | wallet/txauthor | `TxSerializeType::Full`, `ser_type` | ok |
| `TxVersion`, `TxVersionTreasury` | wallet, internal/rpc/jsonrpc | `TX_VERSION` (1); no constant for 3 | `TxVersionTreasury` **missing** (constant `3`) |
| `TxTreeRegular`, `TxTreeStake` | internal/rpc/jsonrpc, spv, wallet, wallet/udb | `TX_TREE_REGULAR`, `TX_TREE_STAKE` | ok |
| `NoExpiryValue`, `NullValueIn`, `NullBlockHeight`, `NullBlockIndex`, `MaxPrevOutIndex`, `MaxTxInSequenceNum`, `DefaultPkScriptVersion` | wallet/udb, internal/rpc/jsonrpc, wallet | `NO_EXPIRY_VALUE`, `NULL_VALUE_IN`, `NULL_BLOCK_HEIGHT`, `NULL_BLOCK_INDEX`, `MAX_PREV_OUT_INDEX`, `MAX_TX_IN_SEQUENCE_NUM`, `DEFAULT_PK_SCRIPT_VERSION` (`msgtx.rs:18-65`) | ok |
| `VarIntSerializeSize` | wallet/txrules, wallet/txsizes | `var_int_serialize_size` | ok |
| `WriteVarString` | wallet (`wallet.go:2011-2012`, `:2041-2042`) | — (private `read_var_string_bytes` only) | **missing** (public writer) |
| `BlockHeader` and fields | 15 files | `BlockHeader` (`blockheader.rs:39-78`); `timestamp` is `u32` unix seconds, not a time value | shape |
| `BlockHeader.BlockHash`, `PowHashV1`, `PowHashV2` | chain, wallet, validate, internal/rpc/jsonrpc | `block_hash`, `pow_hash_v1`, `pow_hash_v2` (`blockheader.rs:150-171`) | ok |
| `BlockHeader.Serialize`, `Bytes`, `Deserialize`, `FromBytes` | wallet, wallet/udb, rpc/client/dcrd | `serialize() -> [u8; 180]`, `from_bytes`, `decode` (`blockheader.rs:81-117`) | shape |
| `MaxBlockHeaderPayload` (tests) | wallet/udb tests | `MAX_BLOCK_HEADER_PAYLOAD` (180) | ok |
| `MsgBlock`, `MsgBlock.AddTransaction`, `BlockHash`, `Deserialize`, `Header` | chain, p2p, spv, validate, wallet, rpc/client/dcrd | `MsgBlock { header, transactions, stransactions }`, `from_bytes`, `serialize`, `block_hash` (`msg_data.rs:274-394`) | shape |
| `CurrencyNet`, `MainNet`, `TestNet3`, `SimNet` | 7+ files | `CurrencyNet(u32)`, `CurrencyNet::MAIN_NET`, `TEST_NET3`, `SIM_NET`, `REG_NET` (`protocol.rs:102-112`) | ok |
| `ServiceFlag`, `SFNodeNetwork` | p2p, spv | `ServiceFlag(u64)`, `ServiceFlag::NODE_NETWORK` | ok |
| `ProtocolVersion` gates: `SendHeadersVersion`, `CFilterV2Version`, `InitStateVersion`, `RemoveRejectVersion`, `MixVersion`, `BatchedCFiltersV2Version` | p2p, spv, rpc/client/dcrd | `SEND_HEADERS_VERSION` (3), `CFILTER_V2_VERSION` (7), `INIT_STATE_VERSION` (8), `REMOVE_REJECT_VERSION` (9), `MIX_VERSION` (10), `BATCHED_CFILTERS_V2_VERSION` (11) (`protocol.rs:26-51`) | ok |
| `Message` (interface), `ReadMessage`, `WriteMessage` | p2p, rpc/client/dcrd | `enum Message`, `read_message(buf, pver, net) -> (Message, usize)`, `read_message_header`, `write_message(msg, pver, net) -> Vec<u8>` (`message.rs:69`, `:818`, `:891`, `:949`) | shape (enum, slice framing) |
| `Message.Command`, `BtcEncode`, `BtcDecode` | p2p, rpc/client/dcrd | `Message::command`, `encode_payload(pver)`, `decode_message_payload(command, payload, pver)` (`message.rs:117`, `:560`, `:715`) | shape |
| `CmdMixPairReq`, `CmdMixKeyExchange`, `CmdMixCiphertexts`, `CmdMixSlotReserve`, `CmdMixFactoredPoly`, `CmdMixDCNet`, `CmdMixConfirm`, `CmdMixSecrets` | rpc/client/dcrd | no constants; the literals `mixpairreq`, `mixkeyxchg`, `mixcphrtxt`, `mixslotres`, `mixfactpoly`, `mixdcnet`, `mixconfirm`, `mixsecrets` appear in `message.rs:152-159` | **missing** (public constants) |
| `MsgVersion`, `NewMsgVersion`, `MsgVersion.AddUserAgent`, `MsgVerAck`, `NewMsgVerAck` (`p2p/peering.go:607`) | p2p (`peering.go:237-238`) | `MsgVersion` with public fields incl. `user_agent: String` (`msg_control.rs:33-52`); `Message::VerAck` | `AddUserAgent` **missing** (formats `/name:version/` appended, enforces `MaxUserAgentLen`); the rest shape |
| `MsgAddr`, `MsgAddr.AddAddress(es)`, `NewMsgGetAddr`, `NetAddress`, `NewNetAddressIPPort` | p2p, spv, wallet | `MsgAddr { addr_list }`, `Message::GetAddr`, `NetAddress` (`netaddress.rs:19`) | shape |
| `MsgPing`, `MsgPong`, `NewMsgPing`, `NewMsgPong`, `NewMsgSendHeaders` | p2p | `MsgPing { nonce }`, `MsgPong { nonce }`, `Message::SendHeaders` | shape |
| `MsgInv`, `MsgGetData`, `MsgNotFound`, `NewMsgInvSizeHint`, `NewMsgGetDataSizeHint`, `.AddInvVect` | p2p, spv | `MsgInv { inv_list }`, `MsgGetData`, `MsgNotFound` (`msg_data.rs:89-105`) | shape |
| `InvVect`, `NewInvVect`, `InvTypeTx`, `InvTypeBlock`, `InvTypeMix` | p2p, spv, wallet | `InvVect`, `InvType::TX`, `InvType::BLOCK`, `InvType::MIX` (`invvect.rs:23-55`) | ok |
| `MsgGetHeaders`, `MsgHeaders` | p2p | `MsgGetHeaders(BlockLocator)`, `MsgHeaders { headers }` | shape |
| `MsgCFilterV2`, `MsgCFiltersV2`, `NewMsgGetCFilterV2`, `NewMsgGetCFsV2` | p2p | `MsgCFilterV2`, `MsgCFiltersV2`, `MsgGetCFilterV2`, `MsgGetCFsV2` (`msg_cf.rs:94-125`) | ok |
| `MsgGetInitState`, `NewMsgGetInitState`, `.AddTypes`, `MsgInitState`, `NewMsgInitState`, `InitStateTSpends` | p2p, spv | `MsgGetInitState { types }`, `MsgInitState`, `INIT_STATE_TSPENDS` | shape |
| `MsgGetMiningState`, `NewMsgMiningState` | p2p | `Message::GetMiningState`, `MsgMiningState` | ok |
| `MsgMixPairReq`, `MsgMixKeyExchange`, `MsgMixCiphertexts`, `MsgMixSlotReserve`, `MsgMixFactoredPoly`, `MsgMixDCNet`, `MsgMixConfirm`, `MsgMixSecrets`, `MixVect`, `MixPairReqUTXO` | chain, p2p, spv, wallet, rpc/client/dcrd; mixclient | same names (`msg_mix.rs`, re-exported `lib.rs:98-103`) | ok |
| mix message `Hash`, `WriteHash`, `Pub`, `Sid`, `GetRun`, `PrevMsgs` (methods of the `mixing.Message` interface, which is `wire.Message` plus `Pub`, `Sig`, `WriteHash`, `Hash`, `WriteSignedData`, `PrevMsgs`, `Sid`, `GetRun`; `mixing@v0.7.5/message.go:23-40`) and `MsgMixPairReq.Expires` (not part of that interface) | chain, p2p, spv, wallet | `mix_hash()`, `signed_data()` on each type (`msg_mix.rs:1100-1149`), `Message::mix_hash`, `MsgMixPairReq::expires`, `pairing`; the rest through `dcroxide_mixing::MixMessage` | shape |
| `NewMsgMix*` (eight constructors) | mixclient only | struct literals | shape |
| `MaxInvPerMsg`, `MaxBlockHeadersPerMsg`, `MaxCFiltersV2PerBatch`, `MaxMixPeers` | p2p, chain, spv, mixclient | `MAX_INV_PER_MSG` (50,000), `MAX_BLOCK_HEADERS_PER_MSG`, `MAX_CFILTERS_V2_PER_BATCH`, `MAX_MIX_PEERS` | ok |
| `MaxBlocksPerMsg` | wallet (`discovery.go:70-73`) | — | **missing** (constant `500`, `wire@v1.8.0/msgblock.go:24`) |

Gaps in dcroxide-wire for the wallet: `pk_script_locs`, a public
`write_var_string`, `MAX_BLOCKS_PER_MSG`, `TX_VERSION_TREASURY`, the eight
`CMD_MIX_*` constants (and the other command names if the wallet's P2P code
wants them), `MsgVersion::add_user_agent`. All are additive and small.

`PkScriptLocs` returns, for each output, the byte offset of its pkScript
inside the transaction's serialization (section 5.11). dcrwallet reads
credits out of stored transactions by these offsets (spec 03).

Struct fields. dcrwallet reads and writes the exported fields of the wire
types directly; the port keeps every field under its snake-case name, with
the representation changes below (field inventory of dcrwallet's non-test
files against `wire@v1.8.0`; Rust definitions in
`dcroxide:crates/dcroxide-wire/src/blockheader.rs:39-78`, `msgtx.rs:160-296`,
`msg_control.rs:33-52`, `netaddress.rs:19`, `msg_cf.rs:94-125`,
`msg_data.rs`, `msg_mix.rs`):

| Go type | Fields dcrwallet names | dcroxide fields | Representation change |
|---|---|---|---|
| `BlockHeader` | all 18: `Version`, `PrevBlock`, `MerkleRoot`, `StakeRoot`, `VoteBits`, `FinalState`, `Voters`, `FreshStake`, `Revocations`, `PoolSize`, `Bits`, `SBits`, `Height`, `Size`, `Timestamp`, `Nonce`, `ExtraData`, `StakeVersion` | `version: i32`, `prev_block`, `merkle_root`, `stake_root`, `vote_bits: u16`, `final_state: [u8; 6]`, `voters: u16`, `fresh_stake: u8`, `revocations: u8`, `pool_size: u32`, `bits: u32`, `sbits: i64`, `height: u32`, `size: u32`, `timestamp: u32`, `nonce: u32`, `extra_data: [u8; 32]`, `stake_version: u32` | `Timestamp` is a `time.Time` in Go (second precision on the wire) and the raw `u32` of unix seconds in the port |
| `OutPoint` | `Hash`, `Index`, `Tree` | `hash`, `index: u32`, `tree: i8` | none |
| `TxIn` | `PreviousOutPoint`, `Sequence`, `ValueIn`, `BlockHeight`, `BlockIndex`, `SignatureScript` | `previous_out_point`, `sequence: u32`, `value_in: i64`, `block_height: u32`, `block_index: u32`, `signature_script: Vec<u8>` | none |
| `TxOut` | `Value`, `Version`, `PkScript` | `value: i64`, `version: u16`, `pk_script: Vec<u8>` | none |
| `MsgTx` | `SerType`, `Version`, `TxIn`, `TxOut`, `LockTime`, `Expiry` | `ser_type`, `version: u16`, `tx_in: Vec<TxIn>`, `tx_out: Vec<TxOut>`, `lock_time: u32`, `expiry: u32` | Go holds `[]*TxIn` and `[]*TxOut` (shared pointers, so two transactions can alias one input); the port owns the values |
| `MsgBlock` | `Header`, `Transactions`, `STransactions` | `header`, `transactions`, `stransactions` | owned `Vec<MsgTx>` instead of `[]*MsgTx` |
| `MsgVersion` | `ProtocolVersion`, `Services`, `UserAgent`, `LastBlock`, `DisableRelayTx` (read from the remote's message and set on the local one, `p2p/peering.go`); `AddrYou`, `AddrMe`, `Nonce`, `Timestamp` through `NewMsgVersion` | `protocol_version: i32`, `services`, `timestamp: i64`, `addr_you`, `addr_me`, `nonce: u64`, `user_agent: String`, `last_block: i32`, `disable_relay_tx: bool` | `Timestamp` is unix seconds; `NewMsgVersion` stamps `time.Now()` itself, the port's caller supplies it |
| `NetAddress` | `Timestamp`, `Services`, `IP`, `Port` | `timestamp: u32`, `services`, `ip: [u8; 16]`, `port: u16` | Go's `IP` is a `net.IP` of 4 or 16 bytes; the port always holds the 16-byte form (IPv4 as an IPv4-mapped IPv6 address, as on the wire) |
| `MsgAddr` | `AddrList` | `addr_list` | none |
| `MsgInv`, `MsgGetData`, `MsgNotFound` | `InvList` | `inv_list` | owned `Vec<InvVect>` |
| `InvVect` | `Type`, `Hash` | `inv_type`, `hash` | none |
| `MsgHeaders` | `Headers` | `headers: Vec<BlockHeader>` | owned values |
| `MsgCFilterV2` | `BlockHash`, `Data`, `ProofIndex`, `ProofHashes` | `block_hash`, `data`, `proof_index: u32`, `proof_hashes: Vec<Hash>` | none |
| `MsgCFiltersV2` | `CFilters` | `cfilters` | none |
| `MsgInitState` | `BlockHashes`, `VoteHashes`, `TSpendHashes` | `block_hashes`, `vote_hashes`, `tspend_hashes` | none |
| `MsgMixPairReq` | none by dcrwallet's own code (it uses the methods of section 3.1's mix row); every field in mixclient and mixpool | `signature: [u8; 64]`, `identity: [u8; 33]`, `expiry: u32`, `mix_amount: i64`, `script_class: String`, `tx_version: u16`, `lock_time: u32`, `message_count: u32`, `input_value: i64`, `utxos`, `change: Option<TxOut>`, `flags: u8`, `pairing_flags: u8` | `Change` is a nil-able `*wire.TxOut` in Go and an `Option` in the port |
| `MsgMixKeyExchange` | `SeenPRs`, `Pos` (`spv/backend.go:495`), `Identity` (`spv/sync.go:1243-1245`); every field in mixclient and mixpool | `signature`, `identity`, `session_id`, `epoch: u64`, `run: u32`, `pos: u32`, `ecdh: [u8; 33]`, `pqpk: [u8; 1218]`, `commitment: [u8; 32]`, `seen_prs: Vec<Hash>` | none |

A wallet function that mutates a transaction through a shared `*TxIn` or
`*TxOut` pointer (Go's `MsgTx.TxIn[i]` is a pointer) has no direct
equivalent; each such site becomes an index into the owned vector. Which
sites rely on aliasing was not traced here (the transaction-construction
specs own them).

### 3.2 `github.com/decred/dcrd/chaincfg/chainhash` v1.0.5 → `dcroxide-chainhash`

Imported by 58 files.

| Go identifier | Used in | dcroxide item | Status |
|---|---|---|---|
| `Hash`, `HashSize` | everywhere | `Hash(pub [u8; 32])`, `HASH_SIZE` (`lib.rs:27`, `:78`) | ok |
| `NewHash(bytes)` | internal/rpc/rpcserver, wallet/udb | `Hash::new(&[u8]) -> Result<Hash, InvalidHashLen>` (`lib.rs:86`) | ok |
| `NewHashFromStr`, `Decode` | internal/rpc/jsonrpc, rpc/client/*, wallet, validate | `impl FromStr for Hash`, error `HashError` with Go's texts (`lib.rs:40-71`) | ok |
| `Hash.String` | everywhere | `impl Display` (byte-reversed hex) | ok |
| `Hash.IsEqual`, `Hash.SetBytes` | p2p, validate, wallet, wallet/udb | `==`; `Hash::new` then assign | shape |
| `HashB` | wallet | `hash_b(&[u8]) -> [u8; 32]` (`lib.rs:230`) | ok |
| `HashH` (through dependencies) | — | `hash_h` | ok |
| `MaxHashStringSize` | internal/rpc/jsonrpc | `MAX_HASH_STRING_SIZE` (64) | ok |

No gap.

### 3.3 `github.com/decred/dcrd/chaincfg/v3` v3.3.0 → `dcroxide-chaincfg`

Imported by 44 files. `Params` is ported field for field
(`dcroxide:crates/dcroxide-chaincfg/src/lib.rs:138-229`), except the
deprecated, always-empty `Checkpoints` field, which the port leaves out
(`dcroxide:crates/dcroxide-chaincfg/src/lib.rs:15`, `:134`); dcrwallet does
not read it.

| Go identifier | Used in | dcroxide item | Status |
|---|---|---|---|
| `Params` (type) | 27 files | `Params` | ok |
| `MainNetParams()`, `TestNet3Params()`, `SimNetParams()` | internal/netparams, `.` | `mainnet_params()`, `testnet3_params()`, `simnet_params()` — each call builds a fresh owned `Params` | shape (no shared static; cache one per process) |
| `RegNetParams()` | cmd/treasurykey only | `regnet_params()` | ok |
| `Choice`, `Vote`, `ConsensusDeployment` | wallet | same names (`lib.rs:59-106`); `Vote::vote_index` returns `Option<usize>` where Go returns `-1` | shape |
| `VoteIDBlake3Pow`, `VoteIDChangeSubsidySplit`, `VoteIDChangeSubsidySplitR2` | wallet, deployments | `VOTE_ID_BLAKE3_POW`, `VOTE_ID_CHANGE_SUBSIDY_SPLIT`, `VOTE_ID_CHANGE_SUBSIDY_SPLIT_R2` (`votes.rs:37-41`) | ok |
| `Params.Deployments` (`map[uint32][]ConsensusDeployment`) | chain, wallet | `deployments: Vec<(u32, Vec<ConsensusDeployment>)>`, sorted by version | shape |
| `Params.PowLimit` (`*big.Int`) | wallet | `pow_limit: Uint256` (`dcroxide-uint256`) | shape |
| `Params.TargetTimePerBlock`, `TargetTimespan`, `MinDiffReductionTime` (`time.Duration`) | wallet | `target_time_per_block_secs`, `target_timespan_secs`, `min_diff_reduction_time_secs` (`i64` seconds) | shape |
| `Params.TicketExpiryBlocks()`, `BlockOneSubsidy()`, `PiKeyExists()`, `Seeders()` | wallet, spv, p2p | field `ticket_expiry`; `block_one_subsidy()`; `pi_key_exists()`; field `seeders` | shape |
| `stdaddr.AddressParams` methods on `*Params` | every address call | `impl AddressParamsV0 for Params` (`dcroxide:crates/dcroxide-txscript/src/stdaddr.rs:119-135`) | ok |
| `standalone.SubsidyParams` methods on `*Params` | wallet, wallet/txrules | `impl SubsidyParams for Params` (in dcroxide-chaincfg; `dcroxide:docs/PARITY.md:1020`) | ok |
| `hdkeychain.NetworkParams` methods (`HDPrivKeyVersion`, `HDPubKeyVersion`) | every HD key call | fields `hd_private_key_id`, `hd_public_key_id` exist; no trait | **missing** (trait impl, with the hdkeychain crate) |

Fields of `Params` dcrwallet reads directly (non-test code): `Name`, `Net`,
`DefaultPort`, `GenesisBlock`, `GenesisHash`, `PowLimit`, `PowLimitBits`,
`ReduceMinDifficulty`, `MinDiffReductionTime`, `MaxTxSize`,
`TargetTimePerBlock`, `WorkDiffAlpha`, `WorkDiffWindowSize`,
`WorkDiffWindows`, `TargetTimespan`, `RetargetAdjustmentFactor`,
`WorkDiffV2Blake3StartBits`, `WorkDiffV2HalfLifeSecs`, `BaseSubsidy`,
`MulSubsidy`, `DivSubsidy`, `SubsidyReductionInterval`,
`RuleChangeActivationInterval`, `Deployments`, `PrivateKeyID`,
`SLIP0044CoinType`, `LegacyCoinType`, `MinimumStakeDiff`, `TicketPoolSize`,
`TicketsPerBlock`, `TicketMaturity`, `TicketExpiry`, `CoinbaseMaturity`,
`SStxChangeMaturity`, `StakeDiffWindowSize`, `MaxFreshStakePerBlock`,
`StakeValidationHeight`, `StakeBaseSigScript`, `TreasuryVoteInterval`,
`TreasuryVoteIntervalMultiplier`. All 40 exist in dcroxide's struct under
snake-case names.

Fields of the vote types dcrwallet reads (`wallet/wallet.go`,
`wallet/difficulty.go`, internal/rpc/jsonrpc, internal/rpc/rpcserver), all
present (`dcroxide:crates/dcroxide-chaincfg/src/lib.rs:59-88`):

| Go type | Fields | dcroxide fields |
|---|---|---|
| `Vote` | `Id`, `Description`, `Mask`, `Choices` | `id: &'static str`, `description: &'static str`, `mask: u16`, `choices: Vec<Choice>` |
| `Choice` | `Id`, `Description`, `Bits`, `IsAbstain`, `IsNo` | `id: &'static str`, `description: &'static str`, `bits: u16`, `is_abstain: bool`, `is_no: bool` |
| `ConsensusDeployment` | `Vote`, `ForcedChoiceID`, `StartTime`, `ExpireTime` | `vote`, `forced_choice_id: &'static str`, `start_time: u64`, `expire_time: u64` |

The string fields are `&'static str`: code that builds a `Vote` or
`Choice` from run-time strings needs its own owned type (quirk Q-34).

One data delta: dcroxide's mainnet `seeders` list contains `dcr-seed.jz.bz`,
which `chaincfg/v3@v3.3.0` does not (section 4.1, quirk Q-2).

### 3.4 `github.com/decred/dcrd/dcrutil/v4` v4.0.3 → no crate

Imported by 50 files. dcroxide has no dcrutil crate and records why
(`dcroxide:docs/PARITY.md:1044`): `hash160` lives in txscript's `stdaddr`,
directory resolution in the node binary's crate, and amounts stay `i64`.

| Go identifier | Used in | dcroxide item | Status |
|---|---|---|---|
| `Amount` (type), `Amount.ToCoin`, `Amount.String` | 31 files (61 `ToCoin` call sites) | `pub fn to_coin(i64) -> f64` in `dcroxide-rpc`'s public `txresults` module (`txresults.rs:24`; `lib.rs:27`), reachable only by linking the node's RPC server crate; nothing for `String` | **missing** as a library item |
| `NewAmount(float64)` | internal/cfgutil, internal/rpc/jsonrpc, internal/rpc/rpcserver, rpc/client/dcrd, rpc/client/dcrwallet | private `new_amount` in `dcroxide-node` (`config.rs:1359`) and `dcroxide-rpc` (`handlers.rs:111`) | **missing** as a public item |
| `AtomsPerCoin`, `MaxAmount` | internal/rpc/jsonrpc, wallet, wallet/txrules | private constants in `dcroxide-standalone` (`tx.rs:23-27`) | **missing** as public items |
| `Amount.Format(unit)` | rpc/grpc_example only (not the binary) | — | not needed |
| `WIF` (type), `NewWIF`, `DecodeWIF`, `WIF.String`, `WIF.PrivKey`, `WIF.PubKey`, `WIF.DSA` | internal/rpc/jsonrpc (`methods.go:2035`, `:5036-5060`), internal/rpc/rpcserver (`server.go:655`), rpc/client/dcrwallet, wallet (`wallet.go:3695`, `:3705`), wallet/udb | — | **missing** |
| `Hash160` | `.`, internal/rpc/jsonrpc, wallet, wallet/udb | `dcroxide_txscript::stdaddr::hash160(&[u8]) -> [u8; 20]` (`stdaddr.rs:144`) | ok |
| `AppDataDir(name, roaming)` | `.` | `app_data_dir` inside `dcroxide-node` (`config.rs:1189`), not reusable without linking the node | **missing** as a library item |
| `BlockValid`, `IsFlagSet16` | internal/rpc/rpcserver, wallet/udb | — | **missing** (trivial: bit 0 of the header vote bits; `flags & flag != 0`) |
| `Tx` (wrapper), `Tx.MsgTx` | rpc/client/dcrwallet only | — | not needed in the daemon; the client library can pass `MsgTx` |
| `txsort.InPlaceSort` | `dcrd/mixing@v0.7.5/mixclient` only | — | **missing** (section 5.7) |

The rest of the package's exported surface, so the new crate's boundary is
explicit. None of it is named by dcrwallet's non-test code, its tests, its
`cmd/` tools, `mixing@v0.7.5` or the vspd client (script inventory), so none of
it is required:

| Go identifier | Source | Contract | Needed |
|---|---|---|---|
| `AmountUnit` (`int`), `AmountMegaCoin` 6, `AmountKiloCoin` 3, `AmountCoin` 0, `AmountMilliCoin` -3, `AmountMicroCoin` -6, `AmountAtom` -8 | `amount.go:18-29` | exponent of the unit relative to one coin | only `AmountCoin`, implicitly, through `ToCoin` and `String` |
| `AmountUnit.String` | `amount.go:34-51` | `MDCR`, `kDCR`, `DCR`, `mDCR`, `μDCR`, `Atom`; any other value `1e<N> DCR` | only the `DCR` case, through `Amount.String` |
| `Amount.ToUnit(u)` | `amount.go:93-95` | `float64(a) / math.Pow10(int(u+8))` | as the body of `ToCoin` |
| `Amount.Format(u)` | `amount.go:106-109` | `FormatFloat(a.ToUnit(u), 'f', -int(u+8), 64) + " " + u.String()` | as the body of `String`; called directly only by `rpc/grpc_example` |
| `Amount.MulF64(f)` | `amount.go:120-122` | `round(float64(a) * f)` | no |
| `AmountSorter` (`Len`, `Swap`, `Less`) | `amount.go:126-145` | ascending `sort.Interface` | no |
| `AtomsPerCent` | `const.go:10` | 1e6 | no |
| `Flags16`, `FlagNone`, `Flag01` … `Flag15` | `bitflags.go:12-33` | bit masks `1 << n`; `BlockValid` is bit 0 | only `BlockValid` |
| `SetFlag16`, `BoolArray16`, `GenerateBoolArray16` | `bitflags.go:41-61` | set or clear one bit; expand to 16 booleans | no |
| `ErrMalformedPrivateKey`, `ErrChecksumMismatch`, `ErrWrongWIFNetwork` (a `[2]byte` error type) | `wif.go:20-40` | section 5.5; dcrwallet does not name them, it wraps whatever `DecodeWIF` returns | the texts, through `DecodeWIF` |
| `AddressParams` (interface), `VerifyMessage(address, signature, message, params)` | `util.go:22-49` | message-signature check; dcrwallet has its own (`wallet/wallet.go:2036`) | no |
| `Block`, `NewBlock`, `NewBlockDeepCopyCoinbase`, `NewBlockDeepCopy`, `NewBlockFromBytes`, `NewBlockFromReader`, `NewBlockFromBlockAndBytes`, `Block.MsgBlock`, `Bytes`, `BlockHeaderBytes`, `Hash`, `Tx`, `STx`, `Transactions`, `STransactions`, `TxHash`, `STxHash`, `TxLoc`, `Height`, `OutOfRangeError`, `BlockHeightUnknown` (-1) | `block.go:19-386` | memoizing wrapper around `wire.MsgBlock` | no: dcrwallet works on `wire.MsgBlock` and `wire.BlockHeader` directly |
| `Tx`, `NewTx`, `NewTxDeep`, `NewTxDeepTxIns`, `NewTxFromBytes`, `NewTxFromReader`, `Tx.MsgTx`, `Hash`, `Index`, `SetIndex`, `Tree`, `SetTree`, `Version`, `SetVersion`, `TxIndexUnknown` (-1) | `tx.go:24-219` | memoizing wrapper around `wire.MsgTx` | only as the parameter type of one client-library method, `rpc/client/dcrwallet/methods.go:693` (`AddTicket(ctx, ticket *dcrutil.Tx)`), which reads `ticket.MsgTx()` |
| `txsort.Sort`, `txsort.IsSorted` | `txsort/txsort.go:121-138` | sorted copy; sortedness test | no |

`dcrutil.Hash160(buf)` is `ripemd160(chainhash.HashB(buf))` and returns a
fresh 20-byte slice (`hash160.go:16-24`). `NewWIF` selects the public key by
scheme: for ECDSA and Schnorr `secp256k1.PrivKeyFromBytes(privKey).PubKey().SerializeCompressed()`
(33 bytes, the key reduced modulo N, never an error); for Ed25519
`edwards.PrivKeyFromScalar(privKey)` and the public key's
`SerializeCompressed()` (32 bytes), returning that function's error; any
other scheme `unsupported signature type '<n>'` (`wif.go:66-84`). It stores
`privKey` as passed, without copying or checking its length, so `String`
emits `2 + 1 + len(privKey) + 4` bytes before base58 (`wif.go:167-181`);
section 4.3's 39-byte layout holds only for 32-byte keys.

### 3.5 `github.com/decred/dcrd/txscript/v4` v4.1.2 → `dcroxide-txscript`

Imported by 13 files. dcroxide-txscript states the same module version
(`dcroxide:crates/dcroxide-txscript/src/lib.rs:2-3`).

| Go identifier | Used in | dcroxide item | Status |
|---|---|---|---|
| `NewEngine(pkScript, tx, idx, flags, scriptVersion, sigCache)`, `Engine.Execute` | wallet; mixclient | `Engine::new(script_pub_key, tx, tx_idx, flags, script_version)`, `set_sig_cache`, `execute` (`engine.rs:560`, `:603`) | ok |
| `ScriptDiscourageUpgradableNops`, `ScriptVerifyCheckLockTimeVerify`, `ScriptVerifyCheckSequenceVerify`, `ScriptVerifyCleanStack`, `ScriptVerifyTreasury` | wallet; mixclient | `ScriptFlags::DISCOURAGE_UPGRADABLE_NOPS`, `VERIFY_CHECK_LOCK_TIME_VERIFY`, `VERIFY_CHECK_SEQUENCE_VERIFY`, `VERIFY_CLEAN_STACK`, `VERIFY_TREASURY` (`engine.rs:34-54`) | ok |
| `CalcSignatureHash` | spv | `calc_signature_hash_checked`, `calc_signature_hash_as_hash` (`sighash.rs:343-355`) | ok |
| `SigHashType`, `SigHashAll`, `SigHashNone`, `SigHashSingle`, `SigHashAnyOneCanPay` | internal/rpc/*, spv, wallet, wallet/txauthor | `SigHashType(pub u8)`, `SIG_HASH_ALL`, `SIG_HASH_NONE`, `SIG_HASH_SINGLE`, `SIG_HASH_ANY_ONE_CAN_PAY` | ok |
| `NewScriptBuilder`, `ScriptBuilder`, `.AddOp`, `.AddData`, `.Script`, `.Reset` | internal/rpc/jsonrpc, wallet (`chainntfns.go:828`) | `ScriptBuilder::new`, `add_op`, `add_data`, `script`, `reset` — by-value builder (each method consumes and returns `Self`; `builder.rs:49-253`) | shape |
| `MakeScriptTokenizer`, `.Next`, `.Opcode`, `.Data`, `.Done`, `.Err` | wallet/udb | `ScriptTokenizer::new(version, script)`, `next`, `opcode`, `data`, `done`, `err` (`tokenizer.rs:140-224`) | ok |
| `DisasmString` | internal/rpc/jsonrpc, internal/rpc/rpcserver | `disasm_string(script) -> (String, Option<ScriptError>)` (`script.rs:117`) | ok |
| `IsUnspendable`, `GenerateSSGenBlockRef` | wallet/txrules, wallet | `is_unspendable`, `generate_ssgen_block_ref` (`script.rs:404`, `:420`) | ok |
| `MaxScriptElementSize` | wallet/txauthor | `MAX_SCRIPT_ELEMENT_SIZE` (2048) | ok |
| `ErrInvalidStackOperation` | wallet | `ErrorKind::InvalidStackOperation` (compare `ScriptError.kind`) | shape |
| opcodes `OP_CHECKSIG`, `OP_DATA_20`, `OP_DATA_30`, `OP_DUP`, `OP_EQUAL`, `OP_EQUALVERIFY`, `OP_HASH160`, `OP_NOP10`, `OP_RETURN`, `OP_SSGEN`, `OP_SSRTX`, `OP_SSTX`, `OP_SSTXCHANGE`, `OP_TADD`, `OP_TGEN` | wallet, wallet/udb, internal/rpc/jsonrpc; mixclient, mixpool | `pub use opcode_table::*` (`lib.rs:55`) | ok |

No gap. The `std` feature turns on the signature cache; without it the crate is
`no_std` (`lib.rs:26`).

### 3.6 `txscript/v4/stdaddr` → `dcroxide_txscript::stdaddr`

Imported by 34 files. Go models addresses as an interface with seven concrete
types; the port is one enum with seven variants
(`dcroxide:crates/dcroxide-txscript/src/stdaddr.rs:152-215`).

| Go identifier | Used in | dcroxide item | Status |
|---|---|---|---|
| `Address` (interface), `.String`, `.PaymentScript` | 20 files | `enum Address`, `encode()`, `payment_script() -> (u16, Vec<u8>)` (`stdaddr.rs:434`, `:483`) | shape |
| `StakeAddress` (interface): `VotingRightsScript`, `RewardCommitmentScript`, `StakeChangeScript`, `PayVoteCommitmentScript`, `PayRevokeCommitmentScript`, `PayFromTreasuryScript` | internal/cfgutil, internal/rpc/jsonrpc, wallet | methods returning `Option<(u16, Vec<u8>)>`, `None` for kinds that are not stake addresses (`stdaddr.rs:555-609`) | shape (a Go type assertion becomes an `Option` check) |
| `AddressPubKeyHasher.AddressPubKeyHash`, `Hash160er.Hash160`, `SerializedPubKeyer.SerializedPubKey` | internal/rpc/*, wallet, wallet/udb | `address_pub_key_hash() -> Option<Address>`, `hash160() -> Option<&[u8; 20]>`, `serialized_pub_key() -> Option<&[u8]>` (`stdaddr.rs:615-664`) | shape |
| `AddressPubKeyEcdsaSecp256k1V0`, `AddressPubKeyHashEcdsaSecp256k1V0`, `AddressScriptHashV0` (type switches) | internal/compat, internal/rpc/*, wallet, wallet/udb | `Address::PubKeyEcdsaSecp256k1V0 { .. }`, `PubKeyHashEcdsaSecp256k1V0 { .. }`, `ScriptHashV0 { .. }` | shape |
| `DecodeAddress(string, params)` | internal/cfgutil, internal/rpc/*, rpc/client/dcrwallet, wallet | `decode_address(&str, &dyn AddressParamsV0)` with dcrd's error kinds and texts (`stdaddr.rs:762`) | ok |
| `NewAddressPubKeyEcdsaSecp256k1V0Raw`, `NewAddressPubKeyEd25519V0Raw`, `NewAddressPubKeySchnorrSecp256k1V0Raw` | internal/rpc/jsonrpc, internal/rpc/rpcserver | `new_address_pub_key_ecdsa_secp256k1_v0_raw`, `new_address_pub_key_ed25519_v0_raw`, `new_address_pub_key_schnorr_secp256k1_v0_raw` (`stdaddr.rs:220-291`) | ok |
| `NewAddressPubKeyHashEcdsaSecp256k1V0`, `NewAddressScriptHashV0`, `NewAddressScriptHashV0FromHash` | `.`, internal/compat, internal/rpc/*, wallet, wallet/udb | `new_address_pub_key_hash_ecdsa_secp256k1_v0`, `new_address_script_hash_v0`, `new_address_script_hash_v0_from_hash` (`stdaddr.rs:341-389`) | ok |
| `AddressParams` (interface) | internal/cfgutil, internal/compat, rpc/client/dcrwallet, wallet | `trait AddressParamsV0` (`stdaddr.rs:104`) | ok |
| `Hash160` | internal/compat, wallet, wallet/udb; mixclient, mixpool | `hash160` | ok |

No gap.

### 3.7 `txscript/v4/stdscript` → `dcroxide_txscript::stdscript`

Imported by 12 files. dcrd's versioned entry points take a script version;
the port exposes the version-0 functions with a `_v0` suffix plus versioned
wrappers for classification, required signatures and address extraction.

| Go identifier | Used in | dcroxide item | Status |
|---|---|---|---|
| `ScriptType` and the 21 `ST*` constants dcrwallet names (`STNonStandard`, `STPubKeyEcdsaSecp256k1`, `STPubKeyEd25519`, `STPubKeySchnorrSecp256k1`, `STPubKeyHashEcdsaSecp256k1`, `STPubKeyHashEd25519`, `STPubKeyHashSchnorrSecp256k1`, `STScriptHash`, `STMultiSig`, `STNullData`, `STStakeSubmissionPubKeyHash`, `STStakeSubmissionScriptHash`, `STStakeGenPubKeyHash`, `STStakeGenScriptHash`, `STStakeRevocationPubKeyHash`, `STStakeRevocationScriptHash`, `STStakeChangePubKeyHash`, `STStakeChangeScriptHash`, `STTreasuryAdd`, `STTreasuryGenPubKeyHash`, `STTreasuryGenScriptHash`) | internal/rpc/*, wallet, wallet/txrules, wallet/udb | `enum ScriptType` with dcrd's names through `name()` (`stdscript.rs:82-108`) | ok |
| `DetermineScriptType(version, script)` | internal/rpc/jsonrpc, wallet/udb | `determine_script_type` (`stdscript.rs:788`) | ok |
| `DetermineRequiredSigs` | internal/rpc/* | `determine_required_sigs` (`stdscript.rs:833`) | ok |
| `ExtractAddrs(version, script, params)` | internal/rpc/*, spv, wallet, wallet/udb | `extract_addrs` (`stdscript.rs:1157`) | ok |
| `ExtractMultiSigScriptDetailsV0` | wallet/udb | `extract_multi_sig_script_details_v0` (`stdscript.rs:361`) | ok |
| `MultiSigDetailsV0` fields `RequiredSigs`, `NumPubKeys`, `Valid` (read at `wallet/udb/txmined.go`; `PubKeys` is not read) | wallet/udb | `MultiSigDetailsV0 { required_sigs: u16, num_pub_keys: u16, pub_keys: Vec<Vec<u8>>, valid: bool }` (`stdscript.rs:347-356`) | ok |
| `ExtractPubKeyHashV0` | wallet; mixpool | `extract_pub_key_hash_v0` (`stdscript.rs:253`) | ok |
| `ExtractStakeGenPubKeyHashV0`, `ExtractStakeRevocationPubKeyHashV0`, `ExtractTreasuryGenPubKeyHashV0`, `IsPubKeyHashScriptV0`, `IsScriptHashScriptV0` | mixpool | same names in snake case (`stdscript.rs:269-690`) | ok |
| `IsMultiSigScriptV0`, `IsMultiSigSigScript`, `MultiSigRedeemScriptFromScriptSigV0`, `MultiSigScriptV0` | wallet, internal/rpc/jsonrpc | `is_multi_sig_script_v0`, `is_multi_sig_sig_script_v0`, `multi_sig_redeem_script_from_script_sig_v0`, `multi_sig_script_v0(threshold, &[&[u8]])` (`stdscript.rs:450-842`) | ok (`IsMultiSigSigScript(version, ..)` → call the `_v0` function when the version is 0, else false) |
| `IsNullDataScript(version, script)` | wallet/txrules | `is_null_data_script_v0` (`stdscript.rs:537`); a versioned form is `dcroxide_stake::is_null_data_script` | shape |
| `IsStakeChangePubKeyHashScript`, `IsStakeChangeScriptHashScript` | wallet/udb | `is_stake_change_pub_key_hash_script_v0`, `is_stake_change_script_hash_script_v0` | shape |
| `ProvablyPruneableScriptV0` | wallet | `provably_pruneable_script_v0` (`stdscript.rs:896`) | ok |

No gap. One recorded divergence: `multi_sig_script_v0` relabels the builder's
size-limit error where dcrd returns txscript's `ErrScriptNotCanonical`
unchanged (`dcroxide:docs/PARITY.md:1078`); the wallet surfaces this error
text from `createmultisig`-style calls, so the text must be checked there.

### 3.8 `txscript/v4/sign` → `dcroxide_txscript::sign`

Imported by 5 files. This is the "signing with private keys" surface; it
exists in dcroxide and is differential-tested byte for byte against dcrd
across all three signature suites (`dcroxide:docs/PARITY.md:1023`).

| Go identifier | Used in | dcroxide item | Status |
|---|---|---|---|
| `SignTxOutput(params, tx, idx, pkScript, hashType, kdb, sdb, previousScript, isTreasuryEnabled)` | wallet, wallet/txauthor | `sign_tx_output` with the same nine arguments (`sign.rs:606-616`) | ok |
| `SignatureScript(tx, idx, subscript, hashType, privKey, sigType, compress)` | wallet | `signature_script` (`sign.rs:184`) | ok |
| `RawTxInSignature(tx, idx, subScript, hashType, key, sigType)` | wallet | `raw_tx_in_signature` (`sign.rs:119`) | ok |
| `TSpendSignatureScript(tx, privKey)` | internal/rpc/jsonrpc | `tspend_signature_script` (`sign.rs:665`) | ok |
| `KeyDB`, `KeyClosure` | wallet, wallet/txauthor | `trait KeyDb`, implemented for every `FnMut(&Address) -> Result<(Vec<u8>, SignatureType, bool), SignError>` (`sign.rs:62-74`) | ok |
| `ScriptDB`, `ScriptClosure` | wallet, wallet/txauthor | `trait ScriptDb`, implemented for every `FnMut(&Address) -> Result<Vec<u8>, SignError>` (`sign.rs:78-90`) | ok |

Private keys cross this API as byte slices: 32 bytes for both secp256k1
suites, and for Ed25519 the 64-byte `seed ‖ public key` form
(`sign.rs:9-16`). One divergence the wallet has to close or accept: dcrd's
`secp256k1.PrivKeyFromBytes` reduces any input modulo the group order and
accepts zero (`dcrd/dcrec/secp256k1/v4@v4.4.1/privkey.go:42-46`), while the
port rejects a zero or out-of-range key with `invalid privkey`
(`sign.rs:92-99`). See quirk Q-5.

### 3.9 `github.com/decred/dcrd/hdkeychain/v3` v3.1.3 → no crate

Imported by 17 files. dcroxide has no port and says so
(`dcroxide:docs/adr/0002-vendor-blake256-from-dcr-rs.md:122-125`). The whole
exported surface of the Go package, with what dcrwallet uses:

| Go identifier | Signature / value | Used by dcrwallet in | Contract |
|---|---|---|---|
| `RecommendedSeedLen` | `32` | `.`, internal/prompt, internal/rpc/rpcserver, wallet | recommended seed bytes |
| `HardenedKeyStart` | `0x80000000` | internal/rpc/jsonrpc, wallet, wallet/udb | first hardened child index |
| `MinSeedBytes`, `MaxSeedBytes` | `16`, `64` | internal/prompt, internal/rpc/rpcserver, wallet, walletseed | seed length bounds |
| `ErrDeriveHardFromPublic` | `cannot derive a hardened key from a public key` | not named | returned by `Child` |
| `ErrNotPrivExtKey` | `unable to create private keys from a public extended key` | not named | returned by `SerializedPrivKey` |
| `ErrInvalidChild` | `the extended key at this index is invalid` | internal/rpc/jsonrpc, wallet, wallet/udb (compared with `errors.Is`, child skipped) | Il out of range |
| `ErrUnusableSeed` | `unusable seed` | wallet/udb | master or parsed private key out of range |
| `ErrInvalidSeedLen` | `seed length must be between 128 and 512 bits` | internal/rpc/rpcserver, wallet, walletseed | seed length |
| `ErrBadChecksum` | `bad extended key checksum` | not named | `NewKeyFromString` |
| `ErrInvalidKeyLen` | `the provided serialized extended key length is invalid` | not named | `NewKeyFromString` |
| `ErrWrongNetwork` | `the provided serialized extended key is for the wrong network` | not named | `NewKeyFromString` |
| `NetworkParams` | interface: `HDPrivKeyVersion() [4]byte`, `HDPubKeyVersion() [4]byte` | satisfied by `*chaincfg.Params` | version bytes |
| `ExtendedKey` | opaque struct (`extendedkey.go:117-127`) | 11 files | section 4.2 |
| `NewMaster(seed, net)` | `(*ExtendedKey, error)` | wallet/udb | section 5.1 |
| `NewKeyFromString(key, net)` | `(*ExtendedKey, error)` | internal/prompt, internal/rpc/jsonrpc, internal/rpc/rpcserver, rpc/client/dcrwallet, wallet/udb | section 5.3 |
| `GenerateSeed(length uint8)` | `([]byte, error)` | `.`, internal/prompt, wallet, walletseed | random bytes from Go's `crypto/rand`; `ErrInvalidSeedLen` outside 16..64 |
| `ExtendedKey.Child(i)` | `(*ExtendedKey, error)` | `.`, internal/rpc/jsonrpc, wallet, wallet/udb (32 call sites) | legacy derivation, section 5.2 |
| `ExtendedKey.ChildBIP32Std(i)` | `(*ExtendedKey, error)` | **not used** | strict derivation |
| `ExtendedKey.Neuter()` | `*ExtendedKey` | `.`, wallet, wallet/udb | public counterpart; returns the receiver itself when already public |
| `ExtendedKey.IsPrivate()` | `bool` | internal/prompt, wallet, wallet/udb | |
| `ExtendedKey.SerializedPubKey()` | `[]byte` (33 bytes) | `.`, internal/compat, internal/rpc/*, wallet, wallet/udb | compressed public key, memoized |
| `ExtendedKey.SerializedPrivKey()` | `([]byte, error)` | wallet/udb (`addressmanager.go:2498`) | the stored key bytes, 1 to 32 bytes (quirk Q-7) |
| `ExtendedKey.String()` | `string` | many | base58 form, or `zeroed extended key` |
| `ExtendedKey.Zero()` | — | `.`, wallet, wallet/udb | wipes and resets fields |
| `ExtendedKey.ChildNum()`, `Depth()`, `ParentFingerprint()` | `uint32`, `uint16`, `uint32` | **not used** | accessors |

Unexported declarations of the package, listed because each one carries
behaviour the new crate reproduces (declaration inventory of
`extendedkey.go`; the file has no other declaration):

| Declaration | Source | Behaviour |
|---|---|---|
| `masterKey = []byte("Bitcoin seed")` | `extendedkey.go:99` | HMAC key of `NewMaster` (section 5.1) |
| `serializedKeyLen = 4 + 1 + 4 + 4 + 32 + 33` | `extendedkey.go:50` | 78, payload length before the checksum (section 4.2) |
| `newExtendedKey(privVer, pubVer, key, chainCode, parentFP, depth, childNum, isPrivate)` | `extendedkey.go:132-147` | plain constructor; stores the slices it is given without copying and leaves `pubKey` nil |
| `ExtendedKey.pubKeyBytes()` | `extendedkey.go:157-171` | returns `key` for a public key; for a private key computes the compressed public key through `secp256k1.PrivKeyFromBytes(key).PubKey()` on first use and memoizes it in `pubKey` (so a stripped key of fewer than 32 bytes is read as the same integer) |
| `hash160(v)` | `extendedkey.go:199-203` | RIPEMD-160 of BLAKE-256, the parent fingerprint hash |
| `doubleBlake256Cksum(v)` | `extendedkey.go:207-211` | first 4 bytes of BLAKE-256(BLAKE-256(v)), the string checksum |
| `ExtendedKey.child(i, strictBIP32)` | `extendedkey.go:218-349` | section 5.2 |
| `paddedAppend(size, dst, src)` | `extendedkey.go:433-438` | appends `size - len(src)` zero bytes and then `src`; `String` uses it with `size = 32` so a stripped private key is serialized left-padded (section 4.2). When `src` is longer than `size` nothing is truncated |
| `zero(b)` | `extendedkey.go:477-482` | overwrites a slice with zero bytes in place (section 5.4) |

Candidates to build from: `dcr-rs:src/hd.rs` models the same derivation,
including the stripped-key provenance (`dcr-rs:src/hd.rs:153-175`), but
differs from dcrd where section 12.4 lists and cannot be linked (section
12.2). The arithmetic the Go code takes from `secp256k1` (`ModNScalar`
add, `ScalarBaseMultNonConst`, `AddNonConst`, `ParsePubKey`) is not public in
`dcroxide-dcrec` (section 3.17).

### 3.10 `github.com/decred/dcrd/blockchain/stake/v5` v5.0.3 → `dcroxide-stake`

Imported by 17 files. Only the transaction-level surface is used; the ticket
database and treap are not.

| Go identifier | Used in | dcroxide item | Status |
|---|---|---|---|
| `TxType`, `TxTypeRegular`, `TxTypeSStx`, `TxTypeSSGen`, `TxTypeSSRtx` | spv, wallet, wallet/udb, internal/rpc/* | `enum TxType { Regular, SStx, SSGen, SSRtx, TAdd, TSpend, TreasuryBase }` (`lib.rs:74-83`) | ok |
| `DetermineTxType` | 8 files | `determine_tx_type` (`lib.rs:1214`) | ok |
| `IsSStx`, `IsSSGen`, `IsSSRtx`, `IsTSpend` | chain, spv, wallet, wallet/udb, internal/rpc/jsonrpc | `is_sstx`, `is_ssgen`, `is_ssrtx`, `is_tspend` | ok |
| `CheckSStx`, `CheckTSpend` | wallet; internal/rpc/jsonrpc, spv | `check_sstx`, `check_tspend -> Result<(Vec<u8>, Vec<u8>), RuleError>` (`treasury.rs:148`) | ok |
| `AddrFromSStxPkScrCommitment`, `AmountFromSStxPkScrCommitment` | internal/rpc/*, wallet, wallet/udb | `addr_from_sstx_pk_scr_commitment`, `amount_from_sstx_pk_scr_commitment` (`lib.rs:392-417`) | ok |
| `TxSStxStakeOutputInfo`, `SStxNullOutputAmounts`, `CalculateRewards` | wallet | `tx_sstx_stake_output_info`, `sstx_null_output_amounts`, `calculate_rewards` (`lib.rs:386`, `:458`, `:562`) | ok |
| `SSGenBlockVotedOn` | wallet, wallet/udb | `ssgen_block_voted_on -> (Hash, u32)` (`lib.rs:433`) | ok |
| `IsStakeCommitmentTxOut` | internal/rpc/rpcserver | `is_stake_commitment_tx_out` (`lib.rs:1240`) | ok |
| `VoteBits` | wallet | `VoteBits { bits, extended_bits }` (`lib.rs:196`) | ok |
| `TreasuryVoteT`, `TreasuryVoteInvalid`, `TreasuryVoteYes`, `TreasuryVoteNo` | internal/rpc/*, wallet, wallet/udb | `TreasuryVote` (`u8`), `TREASURY_VOTE_INVALID` (0), `TREASURY_VOTE_YES` (1), `TREASURY_VOTE_NO` (2) (`lib.rs:813-820`) | ok |

No functional gap. One structural cost: `dcroxide-stake` depends on
`dcroxide-database`, which depends on `redb`
(`dcroxide:crates/dcroxide-stake/Cargo.toml`), so a wallet that links
`dcroxide-stake` — or `dcroxide-gcs`, which depends on it — compiles redb
although it never opens a redb file (section 12.3).

### 3.11 `github.com/decred/dcrd/blockchain/standalone/v2` v2.3.0 → `dcroxide-standalone`

Imported by 9 files.

| Go identifier | Used in | dcroxide item | Status |
|---|---|---|---|
| `CalcTxTreeMerkleRoot`, `CalcCombinedTxTreeMerkleRoot` | validate | `calc_tx_tree_merkle_root`, `calc_combined_tx_tree_merkle_root` (`merkle.rs:77`, `:96`) | ok |
| `VerifyInclusionProof` | validate | `verify_inclusion_proof` (`inclusionproof.rs:119`) | ok |
| `CompactToBig`, `BigToCompact`, `CalcWork` | wallet, internal/rpc/jsonrpc | `compact_to_big`, `big_to_compact`, `calc_work` over `num_bigint::BigInt` (re-exported, `lib.rs:61`) | ok |
| `CheckProofOfWork`, `CheckProofOfWorkHash`, `ErrUnexpectedDifficulty` | wallet | `check_proof_of_work`, `check_proof_of_work_hash`, `ErrorKind::UnexpectedDifficulty` (`pow.rs:189-197`) | ok |
| `CalcASERTDiff` | wallet | `calc_asert_diff` (`pow.rs:221`) | ok |
| `CalcTSpendExpiry`, `InsideTSpendWindow` | internal/rpc/jsonrpc, wallet | `calc_tspend_expiry`, `inside_tspend_window` (`treasury.rs:13`, `:74`) | ok |
| `IsCoinBaseTx`, `IsTreasuryBase` | internal/compat, internal/rpc/jsonrpc | `is_coin_base_tx(tx, is_treasury_enabled)`, `is_treasury_base` (`tx.rs:40`, `:107`) | ok |
| `SubsidyCache`, `NewSubsidyCache`, `.CalcStakeVoteSubsidyV3` | wallet, wallet/txrules | `SubsidyCache<P: SubsidyParams>::new`, `calc_stake_vote_subsidy_v3` (`subsidy.rs:98-390`) | shape: methods take `&mut self`; dcrd's cache is safe for concurrent use |
| `SSVOriginal`, `SSVDCP0010`, `SSVDCP0012` | wallet, wallet/txrules | `SubsidySplitVariant::{Original, Dcp0010, Dcp0012}` (`subsidy.rs:77-88`) | ok |

No gap.

### 3.12 `github.com/decred/dcrd/blockchain/v5` v5.1.1 → not needed

The only import is `blockchain/v5/chaingen` in one test file
(`wallet/reorg_test.go:12`; identifiers `chaingen.Generator`,
`chaingen.MakeGenerator`, `chaingen.SpendableOut`). No non-test file imports
the module. `database/v3` is in `go.mod` only as an indirect requirement
(`go.mod:52`). dcroxide carries no port of `chaingen`: it treats dcrd's
generators as oracles and stores their output as vector files
(`dcroxide:docs/PARITY.md:1026`). The Rust wallet needs neither
`dcroxide-blockchain` nor `dcroxide-database` at run time. Test chains for
the reorg tests come from a Go generator tool or a small Rust block builder
(section 11).

### 3.13 `github.com/decred/dcrd/gcs/v4` v4.1.1 (+ `blockcf2`) → `dcroxide-gcs`

Imported by 14 and 9 files.

| Go identifier | Used in | dcroxide item | Status |
|---|---|---|---|
| `FilterV2` | 12 files | `FilterV2` (`lib.rs:477`) | ok |
| `FromBytesV2(B, M, data)` | p2p, rpc/client/dcrd, wallet/udb | `FilterV2::from_bytes(b, m, d)` (`lib.rs:497`) | ok |
| `FilterV2.N`, `.MatchAny(key, data)`, `.Bytes`, `.Hash` | spv (`sync.go:1323`), wallet (`discovery.go:326-376`), wallet/udb, validate | `n()`, `matches_any(key, &[&[u8]])`, `bytes()`, `hash()` (`lib.rs:536-556`) | ok |
| `KeySize` | internal/rpc/rpcserver, spv, wallet, wallet/udb | `KEY_SIZE` (16) | ok |
| `blockcf2.B`, `blockcf2.M` | p2p, rpc/client/dcrd, wallet/udb | `blockcf2::B` (19), `blockcf2::M` (784931) | ok |
| `blockcf2.Key(merkleRoot)` | spv, wallet/udb | `blockcf2::key` (`blockcf2.rs:81`) | ok |
| `blockcf2.Entries`, `.AddRegularPkScript` | spv, wallet | `blockcf2::Entries<'a>(pub Vec<&'a [u8]>)`, `add_regular_pk_script` (`blockcf2.rs:56-61`) | shape (borrows the scripts) |
| `blockcf2.Regular(block, prevScripts)` | wallet/udb | `blockcf2::regular` (`blockcf2.rs:230`) | ok |
| `blockcf2.PrevScripter` (interface, one method `PrevScript(*wire.OutPoint) (uint16, []byte, bool)`; `gcs/v4@v4.1.1/blockcf2/blockcf.go:208-210`) | wallet/udb implements it with `genesisPrevScripter`, which reports every outpoint as not found (`wallet/udb/upgrades.go:1271-1277`), for the one call that builds the genesis block's filter (`upgrades.go:1314`) | `trait PrevScripter` (`blockcf2.rs:185`), taken as `&impl PrevScripter` | ok |

No gap.

### 3.14 `github.com/decred/dcrd/mixing` v0.7.5 (+ `mixpool`, `mixclient`, `utxoproof`) → `dcroxide-mixing` (partial)

What dcrwallet itself names:

| Go identifier | Used in | dcroxide item | Status |
|---|---|---|---|
| `mixing.Message` (interface) | p2p, rpc/client/dcrd, spv, wallet | `trait MixMessage` (`dcroxide:crates/dcroxide-mixing/src/signatures.rs:28`), `enum PoolMessage` (`mixpool.rs:273`) | shape |
| `mixing.MaxExpiry(tip, params)` | wallet | `max_expiry` (`lib.rs:78`) | ok |
| `mixing.F` | `.` (`testsolver.go:29`) | `F` (`field.rs`, re-exported `lib.rs:28`) | ok |
| `mixpool.NewPool(blockchain)` | wallet | `Pool::new(blockchain, utxo_fetcher)` (`mixpool.rs:719`) | shape |
| `mixpool.Pool.AcceptMessage(msg, src)` | wallet | `Pool::accept_message` / `accept_hashed` (`mixpool.rs:1368-1387`) | shape (`&mut self`, caller answers the mempool-spent question up front) |
| `mixpool.Pool.Message(hash)`, `RemoveMessage(msg)`, `ExpireMessages(height)` | wallet | `message`, `remove_message`, `expire_messages` (`mixpool.rs:809`, `:1061`, `:955`) | ok |
| `mixpool.Source`, `mixpool.ZeroSource` | wallet, chain | no `Source` type; the source id is the plain `src: u64` argument of `accept_message` (`mixpool.rs:1368-1373`), `0` for `ZeroSource` | shape |
| `mixpool.MissingOwnPRError` | chain, spv | `PoolError::MissingOwnPR(Hash)` (`mixpool.rs:514-520`) | shape |
| `mixpool.UseLogger` | `.` | — (the port logs nothing itself) | n/a |
| `mixclient.NewClient(wallet)`, `Client.Run`, `Client.Dicemix`, `Client.ExpireMessages`, `Client.SetLogger` | wallet, internal/rpc/rpcserver | — | **missing** |
| `mixclient.NewCoinJoin`, `CoinJoin.AddInput`, `CoinJoin.Tx`, `CoinJoin.MixedIndices`, `mixclient.GenFunc` | wallet | — | **missing** |
| `mixclient.Wallet` (interface the wallet implements): `BestBlock`, `Mixpool`, `SubmitMixMessage`, `SignInput`, `PublishTransaction` (`dcrd/mixing@v0.7.5/mixclient/client.go:126-145`) | wallet | — | **missing** |

What `mixclient` and `mixpool` need from `mixing` and beyond (the wallet
links all of it):

| Go identifier | Called from | dcroxide item | Status |
|---|---|---|---|
| `SignMessage`, `VerifySignedMessage`, `VerifySignature` | mixclient, mixpool | `sign_message`, `verify_signed_message`, `verify_signature` (`signatures.rs:175-211`) | ok |
| `SortPRsForSession`, `ValidateSession` | mixclient, mixpool | `sort_prs_for_session`, `validate_session` (`sid.rs:40-78`) | ok |
| `SRMixPads`, `SRMix`, `DCMixPads`, `DCMix`, `AddVectors`, `Coefficients`, `IsRoot`, `IntVectorsFromBytes`, `IntVectorsToBytes`, `XorVectors`, `Vec`, `InField` | mixclient | `sr_mix_pads`, `sr_mix`, `dc_mix_pads`, `dc_mix`, `add_vectors`, `coefficients`, `is_root`, `int_vectors_from_bytes`, `int_vectors_to_bytes`, `xor_vectors`, `Vect`, `in_field_be_bytes` (`dcnet.rs`, `field.rs`) | ok (field elements are canonical `u128` residues, not `big.Int`) |
| `Vec` methods `Equals`, `Xor`, `String`; constant `Msize` (20); unexported `randVec` (`mixing@v0.7.5/vec.go:16-70`) | mixclient (`Equals` at `mixclient/blame.go:454`); `Xor` and `randVec` inside `mixing` | free functions `vec_equals`, `vec_xor`, `vec_string`, `rand_vec`; `MSIZE` (`dcnet.rs:18-61`) | shape (functions over `Vect`, not methods) |
| `Signed` (interface taken by `SignMessage` and `VerifySignedMessage`; `signatures.go:20-29`); signature tag `decred-mix-signature` (`signatures.go:17`) | mixclient, mixpool | `trait MixMessage` (`signatures.rs:28`) | shape (one trait serves as both `Signed` and `mixing.Message`) |
| `internal/chacha20prng` (`New`, `Reader`, `SeedSize`) | mixing, mixclient | `ChaCha20Prng`, `SEED_SIZE` (`prng.rs:13-47`) | ok |
| `utxoproof.Secp256k1KeyPair`, `utxoproof.ValidateSecp256k1P2PKH` | mixclient, mixpool | `Secp256k1KeyPair::sign_utxo_proof`, `validate_secp256k1_p2pkh` (`utxoproof.rs:22-50`) | ok |
| constants `MinPeers`, `MaxPeers`, `MaxMcount`, `MaxMtot`, `MaxMixTxSerializeSize`, `MaxMixAmount`, `PRFlagCanSolveRoots`, `ScriptClassP2PKHv0` | mixclient, mixpool | `MIN_PEERS` (4), `MAX_PEERS` (64), `MAX_MCOUNT` (16), `MAX_MTOT` (1024), `MAX_MIX_TX_SERIALIZE_SIZE` (100,000), `MAX_MIX_AMOUNT`, `PR_FLAG_CAN_SOLVE_ROOTS` (1), `SCRIPT_CLASS_P2PKH_V0` (`lib.rs:38-72`) | ok |
| `KX`, `NewKX`, `KX.Encapsulate`, `KX.SharedSecrets`, `RevealedKeys`, `SharedSecrets`, `PQPublicKey`, `PQCiphertext`, `DecapsulateError` (`keyagreement.go`, 237 lines) | mixclient | — (`lib.rs:8-11`: deferred with the wallet-side client) | **missing** |
| `mixpool.Pool.Receive(ctx, *Received)` — blocks until the expected messages arrive or the context ends | mixclient | `Pool::receive(&self, &mut Received)` returns what is accepted now and never waits (`mixpool.rs:14-18`, `:1225-1232`) | shape: the wallet adds the waiting |
| `mixpool.Pool` methods `Epoch`, `MixPRs`, `CompatiblePRs`, `ReceiveKEsByPairing`, `RemoveSession`, `RemoveConfirmedSessions`, `RemoveUnresponsiveDuringEpoch`, `Observer` | mixclient | `epoch_secs`, `mix_prs`, `compatible_prs`, `receive_kes_by_pairing`, `remove_session`, `remove_confirmed_sessions`, `remove_unresponsive_during_epoch`, observer methods on `Pool` (`mixpool.rs:803-2582`) | shape |
| `container/lru.Map`, `NewMapWithDefaultTTL` | mixpool | `dcroxide_containers::lru::Map::new_with_default_ttl` | ok |
| `github.com/companyzero/sntrup4591761`: `GenerateKey`, `Encapsulate`, `Decapsulate`, size constants | mixing (`keyagreement.go:62-101`) | — | **missing** |
| `secp256k1.GeneratePrivateKeyFromRand`, `GenerateSharedSecret` | mixing (`keyagreement.go:34`, `:77`), mixclient (`client.go:288`) | — | **missing** (section 3.17) |
| `decred.org/cspp/v2/solverrpc`: `StartSolver`, `Roots`, `ErrSolverProcessExited`, `SolverProcess` | mixclient; `.` (`config.go:378`, `:742-744`, `testsolver.go:19-29`) | — | **missing** (an external solver process; spec 12/13) |
| `dcrutil/txsort.InPlaceSort` | mixclient | — | **missing** (section 5.7) |
| `golang.org/x/sync/errgroup` | mixclient | — | wallet concurrency design |

dcroxide's `Pool` carries three deliberate divergences from dcrd's mixpool
(`dcroxide:crates/dcroxide-mixing/src/mixpool.rs:9-15`): a cap of 64 key
exchanges per identity (`MAX_KES_PER_IDENTITY`, `mixpool.rs:225`), the
one-message-per-type rule applied to un-orphaned messages too, and an expiry
latch that keeps the newest height. They were chosen for a relaying node. A
wallet pool that refuses a message dcrwallet's pool accepts changes which
peers a mix run sees; whether to keep them is decision D-5.

The rest of what the wallet and `mixclient` take from `mixpool`, item by
item (script inventory of both, against `mixpool`'s exported declarations):

| Go identifier | Source | Used by | dcroxide item | Status |
|---|---|---|---|---|
| `mixpool.BlockChain` (interface): `ChainParams() *chaincfg.Params`, `CurrentTip() (chainhash.Hash, int64)` | `mixpool/mixpool.go:251-258` | wallet implements it with `mixpoolBlockchain` (`wallet/mixing.go:38-49`) and passes it to `NewPool` (`wallet/wallet.go:5526`) | `trait MixBlockChain { chain_params(&self) -> &Params; current_tip(&self) -> (Hash, i64) }` (`mixpool.rs:597-602`); `Pool<B: MixBlockChain>` owns the value | shape |
| `mixpool.UtxoFetcher` (`FetchUtxoEntry(wire.OutPoint) (UtxoEntry, error)`), `mixpool.UtxoEntry` (`IsSpent`, `PkScript`, `ScriptVersion`, `BlockHeight`, `Amount`) | `mixpool/mixpool.go:231-247` | `NewPool` type-asserts the `BlockChain` value to `UtxoFetcher` (`mixpool.go:294-296`); the wallet's `mixpoolBlockchain` has no `FetchUtxoEntry`, so the wallet pool runs **without** UTXO validation of pair requests | `Pool::new(blockchain, utxo_fetcher: Option<Arc<dyn MixUtxoFetcher + Send + Sync>>)` (`mixpool.rs:719-722`) | shape: pass `None` |
| `mixpool.Source` (interface, `ID() uint64`), `mixpool.Uint64Source`, `mixpool.ZeroSource` | `mixpool/mixpool.go:111-125` | wallet, chain, mixclient | plain `u64` | shape |
| `mixpool.Received { Sid [32]byte; KEs, CTs, SRs, DCs, CMs, FPs, RSs (slices); ReceiveAll bool }` — one non-nil slice selects the message type and its capacity is the expected count | `mixpool/mixpool.go:898-909` | mixclient | `Received { sid, kes, cts, srs, dcs, cms, fps, rss: Option<Vec<_>>, receive_all }` (`mixpool.rs:2688-2706`); `Some` replaces non-nil, and there is no capacity-as-count, since `receive` never waits | shape |
| `mixpool.Pool.HaveMessage(hash)` | `mixpool/mixpool.go:328` | mixclient (3 call sites) | `Pool::have_message` (`mixpool.rs:821`) | ok |
| `mixpool.ErrSecretsRevealed` (`secrets revealed by peer`) | `mixpool/errors.go:118` | mixclient (3 sites; switches a run into blame assignment) | `PoolError::SecretsRevealed`, same text (`mixpool.rs:526`, `:556`) | ok |
| `mixpool.Pool.Observer()`, `Observer.CheckPrevEpoch(prevEpoch)`, `Observer.ExcludePRs(prs)` | `mixpool/observer.go:82`, `:138`, `:501` | mixclient (`mixclient/client.go:441`, `:920`, `:945`) | `Pool::check_prev_epoch`, `Pool::exclude_prs` (`mixpool.rs:2269`, `:2582`): the observer state lives in `Pool` | shape |
| `mixpool.Pool.Epoch()` | `mixpool/mixpool.go:308` | mixclient | `Pool::epoch_secs() -> i64` (`mixpool.rs:803`) | shape |
| `mixpool.MissingOwnPRError { MissingPR chainhash.Hash }`, text `KE identity's own PR is missing from mixpool` | `mixpool/errors.go:126-132` | chain, spv (matched with `errors.As`; the missing pair request is then fetched) | `PoolError::MissingOwnPR(Hash)` | shape |
| exported but named by neither the wallet nor mixclient: `Pool.RecentMessage`, `ExpireMessagesInBackground`, `RemoveConfirmedMixes`, `RemoveSpentPRs`, `NonMixSpendsPR`; `Observer.Run`, `MisbehavingBlock`, `MisbehavingTx`; `RuleError`; `IsBannable`; the twelve rule errors `ErrChangeDust` … `ErrPeerPositionOutOfBounds` | `mixpool/mixpool.go:341-764`, `observer.go:107-462`, `errors.go:18-110` | reached only as the error values `AcceptMessage` returns, which the wallet logs | all present (`recent_message`, `expire_messages_in_background`, `remove_confirmed_mixes`, `remove_spent_prs`, `non_mix_spends_pr`, `misbehaving_block`, `misbehaving_tx`, `RuleKind`, `PoolError::is_bannable`; `mixpool.rs:445-537`, `:828-1195`, `:2552-2562`) | ok |
| `mixing.ScriptClass` (string type) and `ScriptClassP2PKHv0 = "P2PKH-secp256k1-v0"` | `mixing@v0.7.5/scriptclass.go:10-15` | mixpool, mixclient | `SCRIPT_CLASS_P2PKH_V0` | ok |

Facts of the Go pool a wallet-side owner has to supply or match, none of
them a parameter of `NewPool`:

- The epoch is 10 minutes, 3 minutes when `ChainParams().Net` is
  `wire.TestNet3` (`mixpool/mixpool.go:280`, `:289-292`; two `XXX` comments
  say it belongs in the chain parameters). dcroxide carries the same rule
  (`mixpool.rs:765` onward).
- The pool's own constants: `minconf = 1`, `feeRate = 0.0001e8` atoms per kB,
  `maxRelayFeeMultiplier = 1e4`, `earlyKEDuration = 5 s`,
  `lateKEDuration = 20 min`, `maxOrphans = 250`,
  `maxPostEvictionOrphans = 187`, `maxMessagesPerIdentity = 7 * 2 * 64 = 896`
  (`mixpool/mixpool.go:30-51`); dcroxide exports `MAX_ORPHANS`,
  `MAX_POST_EVICTION_ORPHANS`, `MAX_MESSAGES_PER_IDENTITY`
  (`mixpool.rs:55-153`).
- Logging: `mixpool.UseLogger` sets a package-level logger
  (`mixpool/log.go:20`; dcrwallet passes `loggers.MixpLog`, `log.go:43`).
  `mixclient` has no `UseLogger`; dcrwallet's `log.go:42` carries the
  commented-out line `// XXX mixclient.UseLogger(loggers.MixcLog)` and sets
  the logger per client with `Client.SetLogger` (`wallet/wallet.go:5540`;
  `mixclient/client.go:456`).

Key agreement types the missing `keyagreement` module defines
(`mixing@v0.7.5/keyagreement.go:20-110`; spec 12 owns the protocol):

| Item | Definition |
|---|---|
| `PQPublicKey`, `PQPrivateKey`, `PQCiphertext`, `PQSharedKey` | byte arrays of `sntrup4591761.PublicKeySize` 1218, `PrivateKeySize` 1600, `CiphertextSize` 1047, `SharedKeySize` 32 (`sntrup4591761.go:23-26`) |
| `KX` | `ECDHPublicKey`, `ECDHPrivateKey` (secp256k1), `PQPublicKey`, `PQPrivateKey`, `PQCleartexts []PQSharedKey` |
| `NewKX(csprng)` | draws the secp256k1 key first (`GeneratePrivateKeyFromRand`; a nil reader means `crypto/rand.Reader()`), then `sntrup4591761.GenerateKey(csprng)` from the **same** stream — the order fixes which stream bytes each key consumes |
| `KX.Encapsulate(prng, pubkeys, my)` | calls `sntrup4591761.Encapsulate(prng, pk)` once per public key, in slice order, own position included; returns the ciphertexts and stores the cleartexts in `PQCleartexts` |
| ECDH shared key | `blake256.Sum256(secp256k1.GenerateSharedSecret(priv, pub))` (`keyagreement.go:76-80`) |
| PQ shared key | `sntrup4591761.Decapsulate(ciphertext, priv)`; a result flag other than 1 makes the unexported `pqSharedKey` return `sntrup4591761: decapsulate failure` (`keyagreement.go:82-87`). That text never leaves the package: `KX.SharedSecrets`, the only caller, discards it and returns `*DecapsulateError{SubmittingIndex: peer}`, whose text is `decapsulate failure of ciphertext by peer <n>` (`keyagreement.go:164-170`, `errors.go:20-28`); mixclient matches it with `errors.As` (`mixclient/blame.go:371`) |
| `sntrup4591761` exported functions | `GenerateKey(io.Reader) (*PublicKey, *PrivateKey, error)`, `Encapsulate(io.Reader, *PublicKey) (*Ciphertext, *SharedKey, error)`, `Decapsulate(*Ciphertext, *PrivateKey) (*SharedKey, int)` (`sntrup4591761.go:55`, `:108`, `:121`) |

### 3.15 `github.com/decred/dcrd/addrmgr/v3` v3.0.0 → `dcroxide-addrmgr` (ports addrmgr v4)

Imported by 5 files.

| Go identifier | Used in | dcroxide item | Status |
|---|---|---|---|
| `New(dataDir)` | `.`, internal/rpc/rpcserver | `AddrManager::new(&Path)` (`manager.rs:600`) | ok |
| `UseLogger` | `.` | — ; `load_peers` returns what dcrd logs (`PeersLoad`) | shape |
| `AddrManager.Start`, `.Stop` | spv, p2p | — ; the port has no goroutine: the caller calls `load_peers` at start and `save_peers` on a timer and at stop (`manager.rs:1381-1435`) | shape |
| `AddrManager.AddAddresses`, `NeedMoreAddresses`, `Attempt`, `Connected`, `Good` | p2p | `add_addresses`, `need_more_addresses`, `attempt`, `connected`, `good` (`manager.rs:866-1101`), all `&mut self` except `need_more_addresses` (`&self`) | shape |
| `AddrManager.GetAddress()` | spv, internal/rpc/rpcserver | `get_address(filter_fn)` (`manager.rs:924`) — v4 takes an address-type filter | shape: pass a filter accepting IPv4 and IPv6 |
| `KnownAddress.NetAddress`, `.LastAttempt` | spv | `KnownAddress::net_address`, `last_attempt` (`manager.rs:199-206`) | ok |
| `NetAddress`, `NewNetAddressFromIPPort`, `NetAddress.Key`, `.Clone` | p2p, spv | `NetAddress { addr_type, ip, port, timestamp, services }`, `new_net_address_from_ip_port`, `key()` (`netaddress.rs:15-249`) | ok |

The `peers.json` format has the same version number in both generations
(`serialisationVersion = 1`, `dcrd/addrmgr/v3@v3.0.0/addrmanager.go:218`;
`SERIALIZATION_VERSION`, `dcroxide:crates/dcroxide-addrmgr/src/manager.rs:61`)
and the same field names. Two differences come with v4: Tor v3 addresses are a
known address type, and address selection keeps per-bucket type statistics.
dcrwallet at the pin never sees `addrv2` messages (it advertises protocol
version 11, `p2p/peering.go:48`), so it never learns a Tor v3 address.

Signature differences on the items above:
`addrmgr.NewNetAddressFromIPPort(ip, port, services)` stamps the current
time itself (`addrmgr/v3@v3.0.0/netaddress.go:175`); the port takes the
timestamp as a fourth argument
(`new_net_address_from_ip_port(ip: &[u8], port, services, timestamp: i64)`,
`netaddress.rs:249-254`). `Attempt`, `Connected` and `Good` return `error`
in Go and `Result<(), AddrError>` in the port (`manager.rs:1061-1101`).
`get_address` returns `Option<KnownAddressRef>` where Go returns a nil
pointer.

`addrmgr.NetAddress` fields dcrwallet reads (`p2p/peering.go`,
`spv/sync.go`): `IP`, `Port`, `Timestamp`, `Services`. Go's v3 struct is
`{Type NetAddressType; IP []byte; Port uint16; Timestamp time.Time; Services wire.ServiceFlag}`
(`addrmgr/v3@v3.0.0/netaddress.go`); the port's is
`{addr_type, ip: Vec<u8>, port: u16, timestamp: i64, services}`
(`dcroxide:crates/dcroxide-addrmgr/src/netaddress.rs:15`), the timestamp
being unix seconds. `NetAddress.Clone()` is the derived `Clone`.

Exported `addrmgr/v3` items dcrwallet does not name: `AddressCache`,
`EncodeHost`, `SetServices`, `AddLocalAddress`, `HasLocalAddress`,
`LocalAddresses`, `GetBestLocalAddress`, `IsExternalAddrCandidate`,
`NewNetAddressFromParams`, `NetAddress.IsRoutable`, `String`, `AddService`,
`GroupKey`, `IsRoutable` (`addrmanager.go:676-1192`, `netaddress.go:38-134`,
`network.go:221-232`). The wallet never advertises a local address.

**HTTPS seeding is not shared code.** dcroxide-addrmgr ports dcrd's seeder
client (`seed_addrs`, `seeder_url`, `HttpsSeederFilters`, `SeederTransport`,
`SeedEnv`; `dcroxide:crates/dcroxide-addrmgr/src/seed.rs:26-327`), which
dcrwallet does not use: `LocalPeer.SeedPeers` is dcrwallet's own
(`p2p/peering.go:345-431`) and differs where a seeder can observe it. It
requests `https://<seeder>/api/addrs?services=<n>` with the single
`services` parameter (dcrd's adds `ipversion` and `pver` when set), through
the configured dialer, with a one-minute timeout per seeder and all seeders
queried concurrently; it reads at most 4,096 bytes of the body (dcrd's bound
is 16 × 256, the same number), decodes a stream of JSON objects with fields
`host` and `services`, takes at most 16 per seeder, skips entries whose
host does not split into an IP literal and a port, and adds each seeder's
batch to the address manager with the batch's first address as the source.
`seed.rs`'s URL builder and limits can be reused as building blocks; the
request must be dcrwallet's (spec 14 owns the algorithm).

### 3.16 `github.com/decred/dcrd/connmgr/v3` v3.1.4 → pieces of `dcroxide-connmgr` and `dcroxide-addrmgr`

Imported by 3 files. dcrwallet uses three things and none of the connection
manager.

| Go identifier | Used in | dcroxide item | Status |
|---|---|---|---|
| `DynamicBanScore` (zero value), `.Increase(persistent, transient)`, `.Int()` | p2p (`peering.go:128`, `:763`, `:1313`, `:2157`) | `DynamicBanScore::new`, `increase_at(persistent, transient, now_unix)`, `int_at(now_unix)` (`dcroxide:crates/dcroxide-connmgr/src/banscore.rs:41-98`) | shape: caller passes the clock and holds the lock (Go's type has its own mutex) |
| `TorLookupIP(ctx, host, proxy)` | `.` | `dcroxide_addrmgr::tor_lookup_ip<T: TorTransport>(host, conn)` (`tordns.rs:58`); the socket-level wrapper lives in the node crate (`dcroxide:crates/dcroxide-node/src/socks.rs:480`) | shape; dialing wrapper **missing** as a library item |
| `UseLogger` | `.` | — | n/a |

`dcroxide-connmgr` depends on `dcroxide-addrmgr`, `dcroxide-containers`,
`dcroxide-crypto` (feature `rand`), `dcroxide-dcrjson`, `dcroxide-ratelimit`
and `siphasher`. For the ban score alone that is a wide dependency; the type
(all of `banscore.rs`, 110 lines) can be re-exported from a smaller crate or
copied (decision D-3).

### 3.17 `dcrec` v1.0.1, `dcrec/secp256k1/v4` v4.4.1 (+ `ecdsa`, `schnorr`), `dcrec/edwards/v2` v2.0.4 → `dcroxide-dcrec`

| Go identifier | Used in | dcroxide item | Status |
|---|---|---|---|
| `dcrec.SignatureType`, `STEcdsaSecp256k1` (0), `STEd25519` (1), `STSchnorrSecp256k1` (2) | internal/rpc/jsonrpc, wallet | `dcroxide_txscript::sign::SignatureType::{EcdsaSecp256k1, Ed25519, SchnorrSecp256k1}` (`sign.rs:36-40`); no numeric conversion is public | shape; numeric mapping **missing** (needed for the WIF byte) |
| `secp256k1.PrivKeyFromBytes([]byte) *PrivateKey` | wallet/udb; hdkeychain; dcrutil WIF | `PrivateKey::from_bytes(&[u8; 32]) -> Option<PrivateKey>` (`secp256k1.rs:353`) | shape + divergence (rejects zero and values at or above the group order; quirk Q-5) |
| `PrivateKey.Serialize()`, `.Zero()`, `.PubKey()` | wallet, wallet/udb, internal/rpc/* | `public_key()`; the bytes accessor `secret_bytes` is `pub(crate)` (`secp256k1.rs:386`); the key is wiped on drop | `Serialize` **missing** as a public item |
| `PublicKey`, `ParsePubKey`, `.SerializeCompressed`, `.SerializeUncompressed`, `.IsEqual` | wallet; mixing, hdkeychain | `PublicKey::parse`, `serialize_compressed`, `serialize_uncompressed`, `==` (`secp256k1.rs:213-322`) | ok |
| `PubKeyBytesLenCompressed`, `PubKeyFormatCompressedEven`, `PubKeyFormatCompressedOdd` | internal/rpc/*, wallet, wallet/udb | `PUB_KEY_BYTES_LEN_COMPRESSED`, `PUB_KEY_FORMAT_COMPRESSED_EVEN`, `PUB_KEY_FORMAT_COMPRESSED_ODD` (`secp256k1.rs:93-102`) | ok |
| `ecdsa.Sign(priv, hash)` | wallet (`wallet.go:1999`, `SignHashes`) | `ecdsa::sign(&PrivateKey, &[u8; 32]) -> Signature` (`ecdsa.rs:619`) | shape: Go takes a `[]byte` of any length and dcrwallet passes client-supplied hashes unchecked; the port takes exactly 32 bytes (quirk Q-31 gives the normalization) |
| `ecdsa.Signature.Serialize()` (DER) | wallet (`wallet.go:2000`) | `Signature::serialize() -> Vec<u8>` (`ecdsa.rs:180`) | ok |
| `ecdsa.SignCompact(priv, hash, isCompressed)` | wallet (signmessage) | `ecdsa::sign_compact -> [u8; 65]` (`ecdsa.rs:633`) | ok |
| `ecdsa.RecoverCompact(sig, hash)` | wallet (verifymessage) | `ecdsa::recover_compact -> Result<(PublicKey, bool), Error>` (`ecdsa.rs:435`) | ok; error `Display` is the kind name, not Go's text (section 8) |
| `schnorr.ParsePubKey`, `schnorr.ParseSignature`, `Signature.Verify` | spv (treasury spend signatures) | `schnorr::parse_pub_key`, `parse_signature`, `Signature::verify` (`schnorr.rs:135-316`) | ok |
| `schnorr.Sign` | mixing (message signatures, UTXO proofs) | `schnorr::sign` (`schnorr.rs:259`) | ok |
| `secp256k1.GeneratePrivateKey()` | cmd/treasurykey | — | **missing** |
| `secp256k1.GeneratePrivateKeyFromRand(io.Reader)` | mixing, mixclient | — | **missing** (section 5.8; the byte consumption is observable) |
| `secp256k1.GenerateSharedSecret(priv, pub)` | mixing | — | **missing** (section 5.9) |
| `ModNScalar` (`SetByteSlice`, `IsZero`, `Add`, `Bytes`), `JacobianPoint`, `ScalarBaseMultNonConst`, `AddNonConst`, `NewPublicKey`, `PublicKey.AsJacobian` | hdkeychain (`extendedkey.go:277-340`) | — (k256 and the vartime crate are private dependencies of dcrec) | **missing**: either a narrow public API (`scalar_add_mod_n`, `pub_key_add_scalar_base`) or the hdkeychain crate takes `k256` directly |
| `edwards.PrivKeyFromScalar([]byte)` | dcrutil `NewWIF` / `DecodeWIF` for Ed25519 (`dcrd/dcrutil/v4@v4.0.3/wif.go:73`, `:148`) | — ; recorded as not implemented (`dcroxide:docs/PARITY.md:1017`) | **missing** |

### 3.18 `crypto/blake256` v1.1.0, `crypto/rand` v1.0.1, `crypto/ripemd160` v1.0.2 → `dcroxide-crypto`

| Go identifier | Used in | dcroxide item | Status |
|---|---|---|---|
| `blake256.New()` (a `hash.Hash`) | chain (`sync.go:79`), p2p (`peering.go:1255`), wallet (`wallet.go:5246`) | `blake256::Blake256::new`, `update`, `finalize(self)` (`blake256.rs:78-242`); `Clone` | shape (`finalize` consumes; clone before finalizing to keep hashing) |
| `blake256.NewHasher256`, `Hasher256`, `Sum256` | mixing, mixclient, utxoproof, hdkeychain | `Blake256`, `blake256::sum256` (`blake256.rs:284`) | ok |
| `ripemd160.Size`, `ripemd160.New` | wallet, wallet/udb; hdkeychain | `ripemd160::SIZE` (20), `ripemd160::sum160` | ok |
| `rand.Read` | internal/rpc/*, wallet/internal/snacl, wallet/udb; mixclient | `rand::read(&mut [u8])` (`rand.rs:447`) | ok (feature `rand`) |
| `rand.Uint64` | p2p | `rand::uint64` | ok |
| `rand.IntN` | wallet/udb | `rand::int_n(usize)` | ok |
| `rand.Duration` | p2p, spv, wallet; mixclient | `rand::duration(i64)` | ok |
| `rand.ShuffleSlice` | internal/rpc/jsonrpc, wallet, wallet/udb | `rand::shuffle_slice` | ok |
| `rand.Int32N` | wallet/txauthor (`author.go:172`) | — (`Prng::uint64n` exists) | **missing** as a global |
| `rand.Int64N` | wallet (`chainntfns.go:779`) | — | **missing** as a global |
| `rand.BigInt(max)` | `.` (`rpcserver.go:120`); mixclient | — | **missing** |
| `rand.Reader()` (an `io.Reader`) | `.` (`rpcserver.go:156-292`), wallet/udb (`addressmanager.go:1745`, `:2196`); mixing | — | **missing** (an `impl Read` or `RngCore` adapter over `rand::read`) |

The draws are not reproducible by an observer, so only their distribution and
their failure behaviour matter; the missing globals are two-line wrappers over
`Prng` (`dcroxide:docs/PARITY.md:1013` says the same of the ones the node did
not need). The process generator is lazily seeded; `dcroxide_crypto::rand::init()`
is the explicit startup hook (`rand.rs:435`).

### 3.19 `github.com/decred/dcrd/dcrjson/v4` v4.2.0 → `dcroxide-dcrjson`

Imported by 8 files.

| Go identifier | Used in | dcroxide item | Status |
|---|---|---|---|
| `MustRegister(method, cmd, flags)`, `UFWebsocketOnly` | rpc/jsonrpc/types | `Registry::must_register`, `UF_WEBSOCKET_ONLY` (`registry.rs:18`, `:289`) — an explicit `Registry` value, not a process global | shape |
| `ParseParams(method, params)` | internal/rpc/jsonrpc | `parse_params` (`cmdparse.rs:182`) | ok |
| `MarshalResponse(rpcVersion, id, result, rpcErr)` | internal/rpc/jsonrpc (`server.go:452`, `:560`, `:599`) | `marshal_response(rpc_version, &RpcId, Option<&str>, Option<&RPCError>)`, the result pre-marshalled (`jsonrpc.rs:277`) | shape |
| `Request` as the **target of `json.Unmarshal`** (fields `Jsonrpc`, `Method`, `Params []json.RawMessage`, `ID interface{}`; `dcrjson/v4@v4.2.0/jsonrpc.go:70-75`) | internal/rpc/jsonrpc (`server.go:369-370`, `:557-558`), then `req.Method`, `req.Params`, `req.ID`, `req.Jsonrpc` | `dcroxide_dcrjson::Request` has `jsonrpc`, `params`, `id` and **no method field and no decoder** (`jsonrpc.rs:213-220`; it is the client-side marshalling shape). The decoder that reproduces `json.Unmarshal` into `dcrjson.Request` with Go's error texts is `dcroxide_rpc::http::unmarshal_request(body) -> Result<RawRequest, String>` with `RawRequest { jsonrpc, method, params, params_present, id }` (`dcroxide:crates/dcroxide-rpc/src/http.rs:134-151`, `:308`) | **missing from the library crate**: reachable only by linking `dcroxide-rpc` (section 3.27) |
| `Response` built as a struct literal and passed to `json.Marshal` (fields `Jsonrpc`, `Result json.RawMessage`, `Error *RPCError`, `ID *interface{}`, no `omitempty`; `jsonrpc.go:118-123`) | internal/rpc/jsonrpc (`methods.go:284-306`, marshalled at `server.go:380`, `:410`, `:431`) | — ; `marshal_response` always writes the version it normalized to `1.0` or `2.0` (`jsonrpc.rs:283-287`), so it cannot emit the `"jsonrpc":""` these three paths send (quirk Q-32) | **missing** |
| `RPCError` fields `Code`, `Message` (`json:"code,omitempty"`, `json:"message,omitempty"`; `jsonrpc.go:21-24`) | internal/rpc/jsonrpc (`errors.go:16-110`) | `RPCError { code, message }`, `RPCError::new`, `marshal` (`jsonrpc.rs:15-34`) | ok |
| `RPCError`, `RPCErrorCode` and the codes `ErrRPCMisc`, `ErrRPCInvalidParameter`, `ErrRPCInvalidAddressOrKey`, `ErrRPCDeserialization`, `ErrRPCDecodeHexString`, `ErrRPCClientNotConnected`, `ErrRPCWallet`, `ErrRPCWalletInsufficientFunds`, `ErrRPCWalletInvalidAccountName`, `ErrRPCWalletUnlockNeeded`, `ErrRPCWalletPassphraseIncorrect`, `ErrRPCNoTxInfo`, `ErrRPCUnimplemented`, `ErrRPCInternal`, `ErrRPCInvalidParams`, `ErrRPCInvalidRequest`, `ErrInvalidRequest` | internal/rpc/jsonrpc | `RPCError`, `codes::{MISC, INVALID_PARAMETER, INVALID_ADDRESS_OR_KEY, DESERIALIZATION, DECODE_HEX_STRING, CLIENT_NOT_CONNECTED, WALLET, WALLET_INSUFFICIENT_FUNDS, WALLET_INVALID_ACCOUNT_NAME, WALLET_UNLOCK_NEEDED, WALLET_PASSPHRASE_INCORRECT, NO_TX_INFO, UNIMPLEMENTED}`, `err_rpc_internal`, `err_rpc_invalid_params`, `err_rpc_invalid_request` (`jsonrpc.rs:61-169`) | ok |
| `GenerateHelp`, `MethodUsageText` | internal/rpchelp | `Registry::generate_help`, `method_usage_text` (`help.rs:442`, `usage.rs:140`) | ok |
| `Float64` (pointer helper) | internal/rpc/jsonrpc | not needed: optionals are `GoValue::Null` (`lib.rs:14-16`) | shape |
| `NewCmd`, `MarshalCmd`, `Bool`, `Int`, `String` | tests only | `new_cmd`, `marshal_cmd` | ok |

Two gaps, both on the server's envelope path and both small: the request
decoder lives in `dcroxide-rpc` instead of `dcroxide-dcrjson`, and there is
no way to marshal a bare `Response` whose `Jsonrpc` is empty. Command
parsing, help and usage have no gap. The port replaces Go reflection with explicit `GoType`/`GoValue`
descriptor trees and its own `encoding/json` emitter and decoder
(`dcroxide:crates/dcroxide-dcrjson/src/lib.rs:5-12`). Every wallet command
and result type has to be written as a descriptor (spec 16 lists them).

### 3.20 `github.com/decred/dcrd/rpc/jsonrpc/types/v4` v4.4.0 → `dcroxide-rpctypes`

Imported by 10 files. dcrwallet uses the module three ways: it re-registers
19 dcrd command types for methods it answers or proxies, it reuses dcrd
result types as its own results, and its dcrd client decodes dcrd results
into them.

| Go identifier | Used in | Role | dcroxide item |
|---|---|---|---|
| `AuthenticateCmd`, `CreateRawTransactionCmd`, `DebugLevelCmd`, `GetBestBlockCmd`, `GetBestBlockHashCmd`, `GetBlockCmd`, `GetBlockCountCmd`, `GetBlockHashCmd`, `GetBlockHeaderCmd`, `GetCFilterV2Cmd`, `GetCurrentNetCmd`, `GetInfoCmd`, `GetPeerInfoCmd`, `GetTxOutCmd`, `HelpCmd`, `SendRawTransactionCmd`, `ValidateAddressCmd`, `VerifyMessageCmd`, `VersionCmd` | rpc/jsonrpc/types | re-registered command types | descriptors in `chainsvrcmds.rs` / `chainsvrwscmds.rs` |
| `TransactionInput` | internal/rpchelp, rpc/client/dcrwallet, rpc/jsonrpc/types, wallet | parameter element | `chainsvrcmds.rs` |
| `GetBestBlockResult`, `GetBlockHeaderVerboseResult`, `GetBlockVerboseResult`, `GetTxOutResult`, `TxRawResult`, `Vin`, `Vout`, `ScriptPubKeyResult`, `ScriptSig`, `VersionResult`, `InfoChainResult` | internal/rpc/jsonrpc, internal/rpchelp | wallet server results | `chainsvrresults.rs` |
| `GetBlockChainInfoResult`, `AgendaInfo`, `AgendaInfoStatusActive`, `AgendaInfoStatusLockedIn`, `GetMixMessageResult`, `GetTxOutResult` | rpc/client/dcrd, chain, deployments | decoded from dcrd | `chainsvrresults.rs` |

Status: ok for every name (each was not checked field by field against the
descriptor; spec 16 owns the field tables). The crate documents itself as the
`v4` module (`dcroxide:crates/dcroxide-rpctypes/src/lib.rs:2-3`). dcrd's
tree has since renamed the module to `types/v5`; the only source difference
between `types/v4@v4.4.0` and the v5 tree of dcrd at `5dd4ca59` is one doc
comment in `chainsvrcmds.go`. Whether `46e9df48` adds fields is
UNVERIFIED (open question O-2).

### 3.21 `github.com/decred/dcrd/rpcclient/v8` v8.1.0 → not used at run time

One file imports it: `rpc/rpc_example/main.go:15`, a standalone example
program (`rpcclient.ConnConfig` at `:42`, `rpcclient.New` at `:49`). It is a
separate `package main`; nothing in the dcrwallet binary links `rpcclient`.
The wallet talks to dcrd through its own client, `rpc/client/dcrd`, over
`github.com/jrick/wsrpc/v2` (spec 15). dcroxide has no `rpcclient` port
(`dcroxide:docs/PARITY.md:1039`: not started). The Rust wallet therefore
needs a JSON-RPC-over-websocket client written for it, and gains nothing from
dcroxide here beyond `dcroxide-dcrjson`'s request marshalling and JSON codec.

### 3.22 `github.com/decred/dcrd/certgen` v1.2.0 → `dcroxide-certgen` (partial)

Imported by 1 file (`internal/cfgutil/curve.go:117`, `:120`).

| Go identifier | dcroxide item | Status |
|---|---|---|
| `NewTLSCertPair(curve, org, validUntil, extraHosts)` with `elliptic.P256()`, `P384()` or `P521()` | `new_tls_cert_pair(env, Curve, org, valid_until_unix, extra_hosts)`; `enum Curve { P256, P521 }` (`certgen.rs:19-26`, `:218`) | P-384 **missing** |
| `NewEd25519TLSCertPair(org, validUntil, extraHosts)` | `new_ed25519_tls_cert_pair` (`certgen.rs:305`) | ok |

dcrwallet's `--tlscurve` accepts `P-256`, `P-384`, `P-521` and `Ed25519`, and
its default is Ed25519 (`internal/cfgutil/curve.go:21-30`, `:59-90`; `config.go:363`); dcrd's
accepts two curves, which is why the port has two. The wallet also builds two
more certificate shapes with Go's `x509.CreateCertificate` directly — a
client-certificate authority and the client certificate it signs
(`rpcserver.go:118-215`). `dcroxide-certgen`'s DER writer is a private module
(`mod x509`, `lib.rs:22`) that reproduces Go only for certgen's template
shape (`dcroxide:docs/PARITY.md:1041`), so those two shapes are new work
(spec 16). `dcroxide-certgen` depends on `dcroxide-tlscrypto`, `p256`, `p384`
and `p521`.

The crate has a second public module, `gentool`
(`dcroxide:crates/dcroxide-certgen/src/lib.rs:19`), the port of dcrd's
`cmd/gencerts`, which is closer to those two shapes than `certgen` is:

| `gentool` item | What it does | Fit for dcrwallet's client-certificate code (`rpcserver.go:118-231`) |
|---|---|---|
| `enum ToolKeyPair { P256, P384, P521, Ed25519 }`, `generate_key(algo)` for `"P-256"`, `"P-384"`, `"P-521"`, `"Ed25519"` (`gentool.rs:59-70`, `:184-213`) | key generation by rejection sampling over system randomness | covers all four `--tlscurve` values, P-384 included |
| `ToolKeyPair::marshal_pkcs8`, `pem_private_key` (`gentool.rs:124`, `:216`) | PKCS#8 DER and the `PRIVATE KEY` PEM block | what `marshalPrivateKey` produces (`rpcserver.go:180-191`) |
| `generate_authority(env, key, hosts, org, years, signs)` (`gentool.rs:509-520`) | self-signed CA from the gencerts template | **template differs**: dcrwallet's CA has subject `CN=dcrwallet, O=dcrwallet, OU=dcrwallet certificate authority`, no subject alternative names, `KeyUsageCertSign` only, ten years clamped to 2049-12-31 23:59:59 UTC, `NotBefore` one day back, a serial drawn below 2^128 (`rpcserver.go:118-156`); `GenTemplate` is private and has no organizational-unit field (`gentool.rs:231-239`) |
| `create_issued_cert(env, key, ca, ca_key, hosts, org, years, signs)`, `LoadedCa`, `load_ca_pair`, `parse_private_key` (`gentool.rs:524-797`) | CA-issued certificate with Go's parent checks | **template differs**: dcrwallet's client certificate has subject OU `dcrwallet client certificate`, `NotAfter` equal to the CA's, `NotBefore` one day back, no key usage and no SANs (`rpcserver.go:193-205`) |

So the DER assembly, signing and PEM code exist and are public in spirit
but not parameterised for dcrwallet's subjects; the additive change is a
template argument (subject fields, key usage, SAN list), not a new writer.
`gentool` documents its own divergences from Go (no RSA, its own PEM
search, CA certificate read only for the fields issuance needs;
`gentool.rs:12-31`).

### 3.23 `github.com/decred/base58` v1.0.6 → `dcroxide-base58`

Not imported by dcrwallet itself; reached through `stdaddr`, `hdkeychain` and
`dcrutil`.

| Go identifier | Caller | dcroxide item | Status |
|---|---|---|---|
| `Encode`, `Decode` | hdkeychain (`extendedkey.go:472`, `:559`), dcrutil WIF (`wif.go:120`, `:180`) | `encode(&[u8]) -> String`, `decode(&str) -> Vec<u8>` (invalid input decodes to empty, as Go's) (`lib.rs:38-129`) | ok |
| `CheckEncode`, `CheckDecode` | stdaddr | `check_encode`, `check_decode` (`lib.rs:220-235`) | ok |

### 3.24 `github.com/decred/dcrd/container/lru` v1.0.0 → `dcroxide_containers::lru`

Indirect (`go.mod:51`), used by `mixpool` only
(`lru.Map`, `NewMapWithDefaultTTL`). Present:
`Map::new_with_default_ttl(limit, ttl)` (`dcroxide:crates/dcroxide-containers/src/lru.rs:120`).
dcrwallet's own `decred.org/dcrwallet/v5/lru` package is a different, smaller
cache and belongs to the wallet port, not to this mapping.

### 3.25 `github.com/decred/slog` v1.2.0 → no crate (code inside `dcroxide-node`)

Imported by 14 files.

| Go identifier | Used in | dcroxide item | Status |
|---|---|---|---|
| `Logger` (interface, 14 methods: six levels × plain and `f` forms, `Level`, `SetLevel`; `slog@v1.2.0/interface.go:11-64`) | 11 files; mixclient, mixpool, vspd client | free functions `log`, `trace`, `debug`, `info`, `warn`, `error` keyed by subsystem tag in `dcroxide-node` (`logging.rs:256-296`) | shape; not a library crate |
| `NewBackend(w)`, `Backend.Logger(tag)` | internal/loggers | the same module renders slog v1.2.0's line format `YYYY-MM-DD hh:mm:ss.sss [LVL] TAG: message` (`logging.rs:2-13`) | as above |
| `LevelFromString`, `Disabled`, `LevelTrace`, `LevelDebug` | `.`, chain, p2p, spv, wallet, …; mixpool; vspd client | `LogLevel` and the `--debuglevel` grammar in `dcroxide-node` (`logsubsys.rs`) | as above |

The line renderer, the per-subsystem level table and the log-file rotator are
written against dcrd's subsystem list and live in the node crate, which a
wallet cannot link without linking the node (section 12.5).

### 3.26 `github.com/decred/vspd/client/v4` v4.0.2 and `types/v3` v3.0.0

`wallet/vsp.go:23` embeds `*vspd.Client` (`:38`, constructed at `:76`). From
dcrd modules the client needs only `stdaddr.Address` (an opaque argument to
the caller's `Sign` callback) and `slog.Logger`
(`vspd/client/v4@v4.0.2/client.go:19-33`). Everything else is Go standard
library: `net/http`, `encoding/json`, `encoding/base64`, `crypto/ed25519`,
`net/http/httputil`. There is no dcroxide counterpart; spec 11 owns it.

Identifiers dcrwallet names from the two modules (script inventory):

| Go identifier | Used in |
|---|---|
| `vspd.Client` with fields `URL`, `PubKey`, `Sign func(context.Context, string, stdaddr.Address) ([]byte, error)`, `Log slog.Logger` (and the embedded `http.Client`; `client.go:24-33`) | `wallet/vsp.go:38`, `:76` |
| `Client.VspInfo`, `FeeAddress`, `PayFee`, `TicketStatus`, `SetVoteChoices` (`client.go:35-122`) | `wallet/vsp.go:109`, `wallet/feepayment.go:361`, `:382` and the fee-payment state machine |
| `types.FeeAddressRequest`, `PayFeeRequest`, `TicketStatusRequest`, `TicketStatusResponse`, `SetVoteChoicesRequest`, `ErrorResponse` | wallet |
| `types.ErrUnknownTicket`, `ErrTicketCannotVote`, `ErrFeeAlreadyReceived`, `ErrInvalidFeeTx`, `ErrCannotBroadcastFee`, `ErrFeeExpired` (compared with `ErrorResponse.Code` after `errors.As`) | `wallet/vsp.go:170-171`, `wallet/feepayment.go:415-451`, `:534-535` |

The response types the client returns but dcrwallet does not name
(`VspInfoResponse`, `FeeAddressResponse`, `PayFeeResponse`,
`SetVoteChoicesResponse`) are still decoded and read by field. The only
HTTP client in dcroxide's dependency graph is `ureq` 3.5.1, used by the
node's seeder transport with `dcroxide-tlscrypto`'s provider
(`dcroxide:crates/dcroxide-node/Cargo.toml:82-92`); it is blocking and
subject to the `clippy.toml` entry-point bans (section 12.1).

### 3.27 dcroxide crates with no dcrwallet import behind them

| Crate | Relevance to the wallet |
|---|---|
| `dcroxide-uint256` | type of `Params.pow_limit`; needed wherever the wallet compares against the proof-of-work limit |
| `dcroxide-secp256k1-vartime` | private dependency of `dcroxide-dcrec` (`publish = false`, licence `ISC AND MIT`) |
| `dcroxide-tlscrypto` | the rustls crypto provider the node serves and dials TLS with; the workspace `clippy.toml` forbids every rustls and ureq entry point that would use a process-default provider, so any TLS in the same workspace goes through it |
| `dcroxide-testutil` | the oracle harness (`tools/oracle`, a Go program linking dcrd's packages) and hex helpers; `publish = false`; reusable as the pattern and partly as code for a dcrwallet oracle |
| `dcroxide-database` | compiled transitively through `dcroxide-stake`; not used |
| `dcroxide-peer` | port of dcrd's `peer` package; dcrwallet has its own `p2p` package and does not import `peer` |
| `dcroxide-rpc`, `dcroxide-node` | hold the HTTP/1.1 server, websocket framing, TLS listener, config-file and flag parser, slog renderer, log rotator and SOCKS5 dialer ports, all coupled to the node (section 12.5) |
| `dcroxide-blockchain` | dcrd's `internal/blockchain`; by its own description currently the UTXO serialization layer. Not linked by the wallet (section 3.12) |
| `dcroxide-ratelimit` | token-bucket limiter from dcrd's `internal/ratelimit`; compiled only if the wallet links `dcroxide-connmgr` (section 3.16) |
| `dcroxide-fees`, `dcroxide-mempool`, `dcroxide-mining`, `dcroxide-netsync`, `dcroxide-indexers` | ports of dcrd's `internal/fees`, `internal/mempool`, `internal/mining`, `internal/netsync`, `internal/blockchain/indexers`. dcrwallet imports none of these Go packages (they are `internal` to dcrd); its fee, relay-policy and sync logic is its own (`wallet/txrules`, `spv`, `chain`). No reuse |
| `dcroxide-winsvc` | dcrd's `service_windows.go`; no dcrwallet source file imports `golang.org/x/sys/windows/svc` (grep over the repository), so there is no service mode to mirror; the inherited-pipe handling (`dcrwallet.go:99-102`) is dcrwallet's own protocol (spec 21) |
| `dcroxide-bench`, `dcroxide-sigdiff` | development-only harnesses with no dcrd counterpart; `sigdiff` is the pipelined client for `tools/oracle` and the pattern for a wallet signature soak |

### 3.28 Non-dcrd modules of `go.mod` and what dcroxide's graph offers

Outside this document's dcrd scope, listed so that no direct requirement of
`go.mod:5-44` is left without an owner. "Graph" is dcroxide's `Cargo.lock`.

| Go module (pin) | Identifiers dcrwallet names (non-test) | dcroxide | Owner |
|---|---|---|---|
| `github.com/decred/go-socks` v1.1.0 | `socks.Proxy{Addr, Username, Password, TorIsolation}`, `Proxy.DialContext`, `socks.NewPool(proxy, maxOpen)`, `Pool.DialContext` (`config.go:693-705`), `*socks.ProxiedAddr` with fields `Host`, `Port` (`p2p/peering.go:215-217`) | `Proxy { addr, username, password, tor_isolation }` and `Proxy::dial(addr, timeout)` inside `dcroxide-node` (`socks.rs:25-34`, `:346`). **No `Pool`** (no circuit limit) and **no `ProxiedAddr`**: the port returns the raw stream and discards the proxy's bound address (`socks.rs:11-14`) | section 12.5; quirk Q-23 |
| `decred.org/cspp/v2` v2.4.0 | `solverrpc.StartSolver`, `solverrpc.SolverProcess` (`config.go:378`, `:742-744`), `solverrpc.Roots` (`testsolver.go:29`) | — | spec 12/13 |
| `github.com/jrick/wsrpc/v2` v2.3.8 | `wsrpc.Dial`, `Client`, `Option`, `Error`, `WithBasicAuth`, `WithTLSConfig`, `WithDial`, `WithNotifier`, `WithoutPongDeadline` (the `Notifier` interface is implemented, not named) | — (the node has a websocket **server**, `dcroxide-node/src/websocket.rs`, `wsframe.rs`; no client) | spec 15 |
| `github.com/gorilla/websocket` v1.5.1 | `websocket.Upgrader`, `Conn`, `TextMessage` (internal/rpc/jsonrpc) | server-side framing in `dcroxide-node`, written against the same release (section 4.1) | spec 16 |
| `go.etcd.io/bbolt` v1.3.12 | `bbolt.Open` (called with a nil `*Options`, `wallet/internal/bdb/db.go:321`; `Options` itself is not named), `DB`, `Tx`, `Bucket`, `Cursor` and eleven `Err*` values (wallet/internal/bdb) | — ; `redb` 4.5 is the node's engine and cannot open a bbolt file | spec 01 |
| `golang.org/x/crypto` v0.57.0 | `scrypt.Key`, `nacl/secretbox.Seal`, `Open`, `Overhead` (wallet/internal/snacl); `argon2.IDKey` (kdf); `blake2b.New256`, `chacha20poly1305.NewX` (wallet/udb/addressmanager.go) | graph has `chacha20poly1305` 0.11.0 and `poly1305` 0.9.1 (through `dcroxide-tlscrypto`), `chacha20` 0.10.2, `sha2`, `hmac`, `zeroize`; it has **no** `scrypt`, `argon2`, `blake2`, `salsa20` or secretbox crate | spec 02 and spec 22 (each new crate must satisfy section 12.1's licence, single-version and no-C rules if adopted) |
| `golang.org/x/sync` v0.23.0 | `errgroup.Group`, `errgroup.WithContext` (chain, p2p, spv, wallet, internal/rpc/jsonrpc, rpc/client/dcrd) | — (ADR-0005: OS threads) | each runtime spec |
| `golang.org/x/term` v0.46.0 | `term.IsTerminal`, `term.ReadPassword` (internal/prompt; `cmd/` tools) | — | spec 21 |
| `github.com/jessevdk/go-flags` v1.5.0 | `flags.NewParser`, `NewIniParser`, `Default`, `ErrHelp`, `Error` (`.`) | `dcroxide-node/src/flags.rs`, ports v1.6.1 (section 12.5) | spec 21 |
| `github.com/jrick/logrotate` v1.0.0 | `rotator.New`, `rotator.Rotator` (internal/loggers) | `dcroxide-node/src/logrotate.rs`, ports v1.1.2 | spec 21 |
| `github.com/jrick/bitset` v1.0.0 | `bitset.Bytes` (chain, rpc/client/dcrd, wallet) | — | spec 15 (wire form of the `existsaddresses`-family results) |
| `google.golang.org/grpc` v1.83.2, `google.golang.org/protobuf` v1.36.11 | generated service code and `codes`/`status` | — ; no `tonic`, `prost` or HTTP/2 crate in the graph | specs 19 and 20 |

### 3.29 Identifiers named only by tests or by the `cmd/` tools

Script inventory of the 43 `_test.go` files and the four `cmd/` programs
(`movefunds`, `repaircfilters`, `sweepaccount`, `treasurykey`). Every other
identifier they use is already in the tables above.

| Where | Identifiers not used by the daemon | Rust counterpart |
|---|---|---|
| tests | `chaingen.Generator`, `MakeGenerator`, `SpendableOut` (`wallet/reorg_test.go`) | none; section 3.12 |
| tests | `dcrjson.NewCmd`, `MarshalCmd`, `Bool`, `Int`, `String` (`rpc/jsonrpc/types`) | `new_cmd`, `marshal_cmd`; the pointer helpers are not needed |
| tests | `wire.MaxBlockHeaderPayload` (`wallet/udb`) | `MAX_BLOCK_HEADER_PAYLOAD` |
| tests | `slog.NewBackend` over a test writer (`wallet`) | section 3.25 |
| tests | `stdaddr.NewAddressPubKeyHashEcdsaSecp256k1(scriptVersion, pkHash, params)`, the versioned constructor (`wallet/udb/addressmanager_test.go:288`; `txscript/v4@v4.1.2/stdaddr/address.go:301-312`: version 0 forwards to the `V0` constructor, any other version fails with `ErrUnsupportedScriptVersion` and the text `pubkey hash addresses for version <n> are not supported`) | none: the port has only the `_v0` constructors (`stdaddr.rs:220-389`); the test calls it with version 0, so `new_address_pub_key_hash_ecdsa_secp256k1_v0` serves |
| `cmd/treasurykey` | `secp256k1.GeneratePrivateKey`, `dcrutil.NewWIF` with `dcrec.STSchnorrSecp256k1`, `chaincfg.RegNetParams` | `GeneratePrivateKey` missing (section 3.17); WIF in the new `dcrutil` crate |
| `cmd/movefunds`, `cmd/sweepaccount` | `dcrutil.Amount`, `NewAmount`, `MaxAmount`, `stdaddr.DecodeAddress`, `wire.NewTxIn`, `OutPoint`, `TxIn`, `TxOut`, `chainhash.NewHashFromStr` | all mapped above |
| `cmd/repaircfilters`, `cmd/sweepaccount` | `dcrutil.AppDataDir`, `wire.MainNet`/`TestNet3`/`SimNet`, `blake256.New`, `wsrpc.Dial` with `WithBasicAuth`, `WithTLSConfig`, `WithoutPongDeadline`, `term.ReadPassword` | section 5.6; spec 15 |

Whether the four tools are part of the drop-in deliverable is a project
decision (open question O-9; spec 22 describes them); none of them needs a dcrd identifier the
daemon does not.

### 3.30 Build facts of each reusable crate

From each crate's `Cargo.toml` and the head of its `src/lib.rs`. "std" is
what a wallet build has to accept; "External" lists non-workspace
dependencies of the library target (dev-dependencies left out).

| Crate | `no_std` | Features | Workspace dependencies | External |
|---|---|---|---|---|
| `dcroxide-chainhash` | yes | — | crypto | — |
| `dcroxide-crypto` | yes without `rand` | `rand` (pulls `chacha20`, `getrandom`, and `std`) | — | `ripemd` 0.2; optional `chacha20` 0.10.2, `getrandom` 0.4 |
| `dcroxide-base58` | yes | — | crypto | — |
| `dcroxide-uint256` | yes | — | — | — |
| `dcroxide-wire` | yes | `count-header-hashes` (test instrumentation; pulls `std`) | chainhash, crypto | `blake3` 1 with `pure` only |
| `dcroxide-standalone` | yes | — | chainhash, wire | `num-bigint` 0.5.1 |
| `dcroxide-chaincfg` | yes | — | chainhash, crypto, standalone, uint256, wire | — |
| `dcroxide-dcrec` | yes | — | secp256k1-vartime, crypto | `k256` 0.14 (`arithmetic`), `hmac` 0.13, `sha2` 0.11, `zeroize` 1.9, `curve25519-dalek` 5 |
| `dcroxide-secp256k1-vartime` | yes | `internals` | — | — (`publish = false`) |
| `dcroxide-txscript` | yes without `std` | `std` (signature cache) | base58, chaincfg, chainhash, crypto, dcrec, wire | `sha1` 0.11, `sha2` 0.11 |
| `dcroxide-stake` | its own code only; the crate builds only with std because of `dcroxide-database` (`lib.rs:25-30`) | — | chainhash, database, txscript, uint256, wire | — |
| `dcroxide-database` | no | — | chainhash, wire | `redb` 4, `crc` 3.4 |
| `dcroxide-gcs` | attribute present; std through `dcroxide-stake` | — | chainhash, stake, txscript, wire | `siphasher` 1 |
| `dcroxide-mixing` | no | — | chaincfg, containers, chainhash, crypto, dcrec, txscript, wire | `chacha20` 0.10.2 |
| `dcroxide-containers` | no | — | — | `siphasher` 1 |
| `dcroxide-addrmgr` | no | — | crypto (`rand`), dcrjson, wire | `sha3` 0.12, `serde` 1 (derive), `serde_json` 1 |
| `dcroxide-connmgr` | no | — | addrmgr, containers, crypto (`rand`), dcrjson, ratelimit | `siphasher` 1 |
| `dcroxide-dcrjson` | no | — | chainhash | — |
| `dcroxide-rpctypes` | no | — | dcrjson | — |
| `dcroxide-certgen` | no | — | dcrec, tlscrypto | `getrandom` 0.4, `p256`/`p384`/`p521` 0.14, `sha2` 0.11 |
| `dcroxide-tlscrypto` | no | — | crypto (`rand`) | `rustls` 0.23.46 (`std`, `tls12`, no default features), `aes-gcm` 0.11.1, `chacha20poly1305` 0.11.0, `poly1305` 0.9.1, `p256`/`p384`/`p521` 0.14, `ecdsa` 0.17, `curve25519-dalek` 5, `ml-dsa` =0.1.2, `num-bigint` 0.5.1, `subtle` 2.6, `sha2`, `hmac`, `getrandom`, `zeroize` |

`dcroxide-certgen` already depends on `p384`; only the `Curve` enum and the
signing path lack the P-384 case (section 3.22). Because `multiple-versions`
is `deny` (section 12.1), a wallet in the same graph has to take the
RustCrypto generation shown here (`sha2` 0.11, `hmac` 0.13, `k256` 0.14,
`chacha20` 0.10, `curve25519-dalek` 5) for its own KDF, AEAD and HD code, or
carry skips.

## 4. Data formats

This section holds the version table that decides whether a dcroxide crate is
the same code dcrwallet links, and the byte formats of the two packages that
have no Rust port and no other spec (`hdkeychain`, `dcrutil`).

### 4.1 Module versions: dcrwallet's pin against dcroxide's

dcroxide tracks dcrd master `46e9df48`. For each module it either links the
release tag (when the in-tree source is byte-identical to it) or the
pseudo-version at the pin (`dcroxide:tools/oracle/go.mod`, header comment).
"Delta" below is the result of diffing the non-test `.go` files of the two
versions in the module cache, where both are present.

| dcrd module | dcrwallet `go.mod` | dcroxide tracks | Delta in non-test source | Matters to the wallet |
|---|---|---|---|---|
| `wire` | v1.8.0 | v1.8.0 | none | no |
| `chaincfg/chainhash` | v1.0.5 | v1.0.5 | none | no |
| `chaincfg/v3` | v3.3.0 | v3.3.1-0.20261106175134-46e9df48109a | `mainnetparams.go`: one added seeder host, `dcr-seed.jz.bz` | yes, mildly: the SPV seeder list differs (quirk Q-2) |
| `crypto/blake256` | v1.1.0 | pseudo-version at the pin | copyright year, `nolint: govet` comments | no |
| `crypto/rand` | v1.0.1 | pseudo-version at the pin | copyright year, one doc comment | no |
| `crypto/ripemd160` | v1.0.2 | v1.0.2 (in-tree source identical) | none | no |
| `dcrec` | v1.0.1 | v1.0.1 | none | no |
| `dcrec/secp256k1/v4` | v4.4.1 | v4.4.2-0.20261106175134-46e9df48109a | new 64-bit field backends (`field64*.go`, `internal/`), array-clearing idioms, `ParsePubKey` split into a method, hex-constant helpers renamed; `nonce.go` and `error.go` comment-only | no observable change found; signatures are RFC 6979 deterministic in both |
| `dcrec/edwards/v2` | v2.0.4 (indirect) | v2.0.4 (the pin's non-test source is identical) | none | no |
| `dcrutil/v4` | v4.0.3 | v4.0.3 | none | not ported at all |
| `txscript/v4` | v4.1.2 | v4.1.3-0.20261106175134-46e9df48109a | comments in `error.go` and `opcode.go` | no |
| `blockchain/stake/v5` | v5.0.3 | v5.0.4-0.20261106175134-46e9df48109a | none (test file only) | no |
| `blockchain/standalone/v2` | v2.3.0 | v2.3.1-0.20261106175134-46e9df48109a | `subsidy.go`: the interval cache skips an insert when another goroutine already cached the interval | no (same values) |
| `gcs/v4` | v4.1.1 | v4.1.2-0.20261106175134-46e9df48109a | one comment in `blockcf2/blockcf.go` | no |
| `blockchain/v5` | v5.1.1 | v5.1.2-0.20261106175134-46e9df48109a | `fullblocktests` only | not linked |
| `database/v3` | v3.0.3 (indirect) | pseudo-version at the pin | comments (per the oracle's `go.mod`) | not linked by wallet code |
| `mixing` | v0.7.5 | in-tree at the pin | UNVERIFIED: v0.7.5 was tagged at dcrd `46fc30ad`, before the pin; the features dcroxide cites as ported (per-identity message cap, late key-exchange gate, whole-minute rule) are all present in v0.7.5 (`dcrd/mixing@v0.7.5/mixpool/mixpool.go:45-51`, `:1669-1694`) | settle with `git diff 46fc30ad..46e9df48 -- mixing/` in a dcrd clone |
| `addrmgr` | `addrmgr/v3` v3.0.0 | `addrmgr/v4` in-tree | new major: Tor v3 address type, typed address filters on `GetAddress`, per-bucket type statistics, the HTTPS seeder and Tor DNS moved in | yes: selection behaviour and API (section 3.15) |
| `connmgr` | `connmgr/v3` v3.1.4 | `internal/connmgr` (rewritten for dcrd 2.2) | whole package | only `DynamicBanScore` and `TorLookupIP` are used; the ban score constants are equal (`Halflife` 60, `Lifetime` 1800) |
| `dcrjson/v4` | v4.2.0 | in-tree; dcrd's root module required v4.2.0 at `5dd4ca59` | UNVERIFIED at the pin | — |
| `rpc/jsonrpc/types` | `types/v4` v4.4.0 | `types/v5` in-tree | at dcrd `5dd4ca59`: one doc comment; at the pin UNVERIFIED | field tables decide (spec 16) |
| `certgen` | v1.2.0 | in-tree; dcrd's root module required v1.2.0 at `5dd4ca59` | UNVERIFIED at the pin | — |
| `container/lru` | v1.0.0 (indirect) | in-tree; dcrd's root module required v1.0.0 at `5dd4ca59` | UNVERIFIED at the pin | — |
| `github.com/decred/base58` | v1.0.6 (indirect) | v1.0.6 | none | no |
| `hdkeychain/v3` | v3.1.3 | not ported | — | — |
| `rpcclient` | `rpcclient/v8` v8.1.0 | not ported | — | not linked |
| `github.com/decred/slog` | v1.2.0 | v1.2.0 line format, inside `dcroxide-node` | — | — |

dcrd's root module at the pin states which release of each in-tree module
and neighbouring library the node was written against
(`dcrd@v1.2.1-0.20261106175134-46e9df48109a/go.mod`): `certgen v1.2.0`
(`:14`), `container/lru v1.0.0` (`:18`), `dcrjson/v4 v4.2.0` (`:25`),
`mixing v0.6.0` (`:29`), `rpc/jsonrpc/types/v5 v5.0.0` (`:31`),
`rpcclient/v8 v8.1.0` and `rpcclient/v9 v9.0.0` (`:32-33`),
`go-socks v1.1.0` (`:37`), `slog v1.2.0` (`:38`),
`gorilla/websocket v1.5.1` (`:39`), `go-flags v1.6.1` (`:40`),
`logrotate v1.1.2` (`:42`), `addrmgr/v4 v4.0.0` (`:9`). The in-tree modules
are replaced by their directories (`:64-92`), so the version numbers of
`certgen`, `lru`, `dcrjson`, `mixing` and `types` there name the last tag,
not the code linked; the module cache holds nested modules only at their
tags, which is why the four UNVERIFIED rows above cannot be settled from
it. `go-socks`, `slog` and `gorilla/websocket` are external and are the
same releases dcrwallet pins.

Neighbouring non-dcrd libraries whose dcroxide ports follow a different
release than dcrwallet pins (their specs must check the delta before reusing
the port): `jessevdk/go-flags` is v1.5.0 in dcrwallet (`go.mod:34`) and the
node's parser ports v1.6.1 (`dcroxide:crates/dcroxide-node/src/flags.rs:2`);
`jrick/logrotate` is v1.0.0 in dcrwallet (`go.mod:36`) and the node's rotator
ports v1.1.2 (`dcroxide:crates/dcroxide-node/src/logrotate.rs:2`).
`gorilla/websocket` is v1.5.1 in dcrwallet (`go.mod:33`), the release
dcroxide's parity ledger cites for dcrd's websocket behaviour
(`dcroxide:docs/PARITY.md:1092`).

A second version axis is the Go toolchain. Behaviour dcrwallet takes from
Go's standard library (JSON error texts, `strconv`, `net/http`, `crypto/x509`
defaults) follows the Go release the dcrwallet binary is built with, not
dcrd's. dcroxide's reference Go is go1.27.3 since the `46e9df48` move, with
its JSON error texts still Go 1.26's as an open interim divergence
(`dcroxide:docs/adr/README.md`, ADR-0001 entry; `dcroxide:docs/PARITY.md:1038`).
dcrwallet's `go.mod` says `go 1.26.0`; the reference Go for dcr-purse is
decision D-6.

### 4.2 Extended key (`hdkeychain`)

In-memory state of an `ExtendedKey` (`hdkeychain/v3@v3.1.3/extendedkey.go:117-127`):

| Field | Type | Meaning |
|---|---|---|
| `privVer`, `pubVer` | `[4]byte` each | network version bytes for private and public serialization; both are kept on every key |
| `key` | `[]byte` | private key bytes, **1 to 32 bytes** (leading zero bytes removed by `Child`), or the 33-byte compressed public key |
| `pubKey` | `[]byte` | memoized compressed public key of a private extended key; empty until first needed |
| `chainCode` | `[]byte` | 32 bytes |
| `parentFP` | `[]byte` | 4 bytes |
| `childNum` | `uint32` | index this key was derived at |
| `depth` | `uint16` | levels below the master |
| `isPrivate` | `bool` | |

String form: plain base58 (not base58check) of 82 bytes
(`extendedkey.go:441-472`):

| Offset | Size | Field | Encoding |
|---|---|---|---|
| 0 | 4 | version | `privVer` when private, else `pubVer` |
| 4 | 1 | depth | `depth % 256` |
| 5 | 4 | parent fingerprint | first 4 bytes of RIPEMD-160(BLAKE-256(parent compressed public key)); `00 00 00 00` for a master key |
| 9 | 4 | child number | big-endian `uint32` |
| 13 | 32 | chain code | |
| 45 | 33 | key data | private: `0x00` then the key left-padded with zero bytes to 32; public: the 33-byte compressed key |
| 78 | 4 | checksum | first 4 bytes of BLAKE-256(BLAKE-256(bytes 0..78)) |

A key whose `key` is empty (after `Zero`) renders as the literal string
`zeroed extended key` (`extendedkey.go:442-444`).

Version bytes per network (`chaincfg/v3@v3.3.0`):

| Network | `HDPrivateKeyID` | Prefix | `HDPublicKeyID` | Prefix | `SLIP0044CoinType` | `LegacyCoinType` |
|---|---|---|---|---|---|---|
| mainnet (`mainnetparams.go:541-547`) | `02 fd a4 e8` | `dprv` | `02 fd a9 26` | `dpub` | 42 | 20 |
| testnet3 (`testnetparams.go:515-521`) | `04 35 83 97` | `tprv` | `04 35 87 d1` | `tpub` | 1 | 11 |
| simnet (`simnetparams.go:550-556`) | `04 20 b9 03` | `sprv` | `04 20 bd 3d` | `spub` | 1 | 115 |
| regnet (`regnetparams.go:537-543`) | `ea b4 04 48` | `rprv` | `ea b4 f9 87` | `rpub` | 1 | 1 |

### 4.3 WIF (`dcrutil`)

A WIF string is plain base58 of 39 bytes (`dcrutil/v4@v4.0.3/wif.go:167-181`):

| Offset | Size | Field |
|---|---|---|
| 0 | 2 | network `PrivateKeyID` |
| 2 | 1 | signature scheme: 0 ECDSA-secp256k1, 1 Ed25519, 2 Schnorr-secp256k1 |
| 3 | 32 | private key bytes as given to `NewWIF` (not padded, not validated for length by `String`) |
| 35 | 4 | first 4 bytes of a **single** BLAKE-256 over bytes 0..35 |

The checksum is one BLAKE-256 (`chainhash.HashB`, `wif.go:129`, `:178`),
unlike addresses and extended keys, which use two. `dcroxide_base58::check_encode`
therefore cannot produce a WIF; use `dcroxide_base58::encode` over the
hand-built 39 bytes.

`PrivateKeyID` per network: mainnet `22 de` (`Pm`), testnet3 `23 0e` (`Pt`),
simnet `23 07` (`Ps`), regnet `22 fe` (`Pr`) (`chaincfg/v3@v3.3.0`,
`mainnetparams.go:538`, `testnetparams.go:512`, `simnetparams.go:547`,
`regnetparams.go:534`).

### 4.4 Amount (`dcrutil`)

`Amount` is an `int64` count of atoms. `AtomsPerCoin = 1e8`,
`MaxAmount = 21e6 * 1e8` (`dcrutil/v4@v4.0.3/const.go:8-17`).

| Operation | Result | Source |
|---|---|---|
| `ToCoin()` | `float64(a) / math.Pow10(8)` — one IEEE-754 division | `amount.go:93-100` |
| `String()` | `Format(AmountCoin)` = `strconv.FormatFloat(a.ToCoin(), 'f', -8, 64) + " DCR"`. The precision argument is `-int(unit+8)`, which is `-8` for coins; Go treats every negative precision as "shortest representation that round-trips", so the text is the shortest decimal for the `float64`, never in exponent form, with no trailing zeros and no decimal point for whole coins (`1 DCR`, `0.5 DCR`, `0.00000001 DCR`) | `amount.go:106-114` |
| `NewAmount(f)` | error `invalid coin amount` for NaN and ±Inf; otherwise `round(f * 1e8)` where `round` adds 0.5 (subtracts for negatives) and converts to `int64` by truncation | `amount.go:61-87` |

In JSON-RPC results dcrwallet emits `ToCoin()` as a JSON number, so the bytes
are Go's `encoding/json` float formatting of that quotient;
`dcroxide_dcrjson::gojson::format_float_json` reproduces it
(`dcroxide:crates/dcroxide-dcrjson/src/gojson.rs:262`).

`dcr-rs:src/amount.rs:11-24` formats from the integer (`{whole}.{frac}` with
trailing zeros trimmed). The two are different algorithms. Within
±`MaxAmount` they should give the same digits: the quotient is the correctly
rounded `float64` of an exact ratio, and below 2^25 coins adjacent `float64`
values are less than 1e-8 apart, so the shortest round-trip decimal is the
8-place decimal itself (an argument, not an executed check; the differential
test of section 11 covers it). Beyond roughly 2^26 coins dcrd's text loses
digits and dcr-rs's does not. A faithful port divides and formats as Go
does.

### 4.5 `peers.json` (`addrmgr`)

The wallet's address manager persists to `peers.json` in the directory given
to `addrmgr.New`. JSON object, field names exactly `Version`, `Key`,
`Addresses` (array of objects with `Addr`, `Src`, `Attempts`, `TimeStamp`,
`LastAttempt`, `LastSuccess`), `NewBuckets`, `TriedBuckets`
(`dcrd/addrmgr/v3@v3.0.0/addrmanager.go:102-119`). `Version` is `1` in both
v3.0.0 and the v4 that dcroxide ports. A file that fails to parse is deleted
and the manager starts empty (`addrmanager.go:496-509`). dcroxide's
serializer uses the same names
(`dcroxide:crates/dcroxide-addrmgr/src/manager.rs:340-373`). Nothing other
than dcrwallet reads this file.

## 5. Algorithms and state machines

Only the algorithms of the unported packages are given here. Everything in
`wire`, `txscript`, `stake`, `standalone`, `gcs`, `mixing` primitives and
`dcrjson` is already implemented in dcroxide and specified by dcrd's source
and dcroxide's parity ledger.

### 5.1 `hdkeychain.NewMaster(seed, net)` (`extendedkey.go:507-534`)

1. If `len(seed) < 16` or `len(seed) > 64`, return `ErrInvalidSeedLen`.
2. `I = HMAC-SHA512(key = "Bitcoin seed", data = seed)`.
3. `secretKey = I[0..32]`, `chainCode = I[32..64]`.
4. Interpret `secretKey` as a big-endian integer. If it is zero or not below
   the secp256k1 group order, return `ErrUnusableSeed`.
5. Return a private key with `key = secretKey` (all 32 bytes, never
   stripped), `parentFP = 00 00 00 00`, `depth = 0`, `childNum = 0`, and the
   network's two version byte arrays.

### 5.2 `ExtendedKey.child(i, strictBIP32)` (`extendedkey.go:218-349`)

`Child(i)` calls it with `strictBIP32 = false`; `ChildBIP32Std(i)` with
`true`. dcrwallet only calls `Child`.

1. If the key is public and `i >= 0x80000000`, return
   `ErrDeriveHardFromPublic`.
2. Build a 37-byte zero-filled buffer `data`.
   - Hardened (`i >= 0x80000000`): copy the stored `key` bytes to
     `data[1..]`. The copy is `len(key)` bytes long. With a 32-byte key this
     is `00 ‖ key`; with a stored key of 31 bytes it is
     `00 ‖ key31 ‖ 00` — the key is left-aligned and the gap before the
     index stays zero (`extendedkey.go:248`).
   - Otherwise: copy the 33-byte compressed public key to `data[0..33]`
     (computed from the private key and memoized when the key is private).
3. Write `i` big-endian into `data[33..37]`.
4. `I = HMAC-SHA512(key = chainCode, data = data)`; `Il = I[0..32]`,
   `childChainCode = I[32..64]`.
5. Interpret `Il` as a big-endian integer. If it is zero or not below the
   group order, return `ErrInvalidChild`.
6. Private parent: `child = (Il + parentKey) mod N`, where `parentKey` is the
   stored key bytes read as a big-endian integer. Serialize `child` as 32
   bytes. When `strictBIP32` is false, remove leading zero bytes while the
   first byte is `0x00` and the slice is non-empty (`extendedkey.go:309-311`).
   The sum is **not** checked for zero: a zero sum becomes an empty key under
   the legacy rule (quirk Q-8).
7. Public parent: compute the point `Il·G`; if its affine X or Y is zero
   return `ErrInvalidChild`; parse the parent's 33-byte key as a point (an
   error from the parse is returned as is); add the two points; the child key
   is the 33-byte compressed serialization of the sum. The sum is not checked
   for the point at infinity.
8. `parentFP` = first 4 bytes of RIPEMD-160(BLAKE-256(parent compressed
   public key)).
9. Return a key with the parent's version arrays, `depth = parent.depth + 1`
   (a `uint16` that wraps at 65,536), `childNum = i`, and the parent's
   private/public kind.

### 5.3 `hdkeychain.NewKeyFromString(key, net)` (`extendedkey.go:538-613`)

1. If `len(key) > 113` (`(82 * 137 / 100) + 1`), return `ErrInvalidKeyLen`.
2. `decoded = base58.Decode(key)`; if its length is not 82, return
   `ErrInvalidKeyLen` (an invalid base58 character decodes to an empty slice
   and lands here).
3. Split `payload = decoded[0..78]`, `checksum = decoded[78..82]`; if the
   checksum is not the first 4 bytes of the double BLAKE-256 of the payload,
   return `ErrBadChecksum`.
4. If `payload[0..4]` equals neither the network's private nor its public
   version, return `ErrWrongNetwork`.
5. `depth = payload[4]`, `parentFP = payload[5..9]`,
   `childNum = BE32(payload[9..13])`, `chainCode = payload[13..45]`,
   `keyData = payload[45..78]`.
6. The key is private **iff `keyData[0] == 0x00`**, whatever the version
   bytes said (`extendedkey.go:595`). Private: drop the first byte; if the
   remaining 32 bytes are zero or not below the group order, return
   `ErrUnusableSeed`. Public: parse the 33 bytes as a secp256k1 public key and
   return the parse error on failure.
7. Return the key with **both** of the network's version arrays, so `String`
   re-emits the version matching the key data kind, not the version read.

The key parsed in step 6 keeps all 32 bytes. A key that `Child` had stored
stripped therefore changes its hardened children after a round trip through
its string (quirk Q-6).

### 5.4 `Neuter`, `SerializedPubKey`, `SerializedPrivKey`, `Zero`

- `Neuter()` on a public key returns the receiver. On a private key it
  returns a new key whose `key` slice **is** the private key's memoized
  `pubKey` slice (the return value of `pubKeyBytes()`, not a copy) and which
  also **shares** the `chainCode` and `parentFP` byte slices with the private
  key (`extendedkey.go:396-408`, `:157-171`).
- `SerializedPrivKey()` returns `ErrNotPrivExtKey` for a public key,
  otherwise the stored key slice as is — 1 to 32 bytes
  (`extendedkey.go:422-428`).
- `Zero()` overwrites `key`, `pubKey`, `chainCode` and `parentFP` in place
  with zeros, then sets `key = nil`, `depth = 0`, `childNum = 0`,
  `isPrivate = false` (`extendedkey.go:488-497`); `pubKey` is zeroed but
  not set to nil. Because of the sharing above, zeroing a private key also
  zeroes the public key bytes (leaving 33 zero bytes, so `String` does not
  take the `zeroed extended key` branch), the chain code and the parent
  fingerprint of a public key neutered from it earlier (quirk Q-9).

### 5.5 `dcrutil.DecodeWIF(wif, net)` and `NewWIF` (`wif.go:66-162`)

`DecodeWIF`:

1. If `len(wif) > 54`, return `ErrMalformedPrivateKey` (`malformed private key`).
2. `decoded = base58.Decode(wif)`; if its length is not 39, return
   `ErrMalformedPrivateKey`.
3. If the first 4 bytes of BLAKE-256(`decoded[0..35]`) differ from
   `decoded[35..39]`, return `ErrChecksumMismatch` (`checksum mismatch`).
4. If `decoded[0..2] != net`, return `ErrWrongWIFNetwork(net)`; its text is
   `WIF is not for the network identified by ` followed by Go's `%#04x`
   rendering of the **expected** two bytes (`wif.go:37-40`). By Go's `fmt`
   rules for a byte array that is `0x` and the four hex digits, `0x22de` on
   mainnet; this was derived, not executed (open question O-6).
5. Switch on `decoded[2]`:
   - 0: private key = `decoded[3..35]`; public key = compressed
     `PrivKeyFromBytes(key).PubKey()` (the key is reduced modulo N, no error);
     scheme ECDSA.
   - 1: private key = `decoded[3..35]`; `edwards.PrivKeyFromScalar` must
     accept it (it reads the 32 bytes as a big-endian integer `D` and fails
     with `bad private scalar size` when the length is not 32,
     `not on subgroup (>N)` when `D > N` — strictly greater, so `D == N` is
     accepted — `zero or negative scalar` when `D == 0`, or
     `scalarbase mult failure to get pubkey` when the base-point
     multiplication returns no point;
     `dcrec/edwards/v2@v2.0.4/privkey.go:122-152`);
     public key = that key's public point in its 32-byte serialization;
     scheme Ed25519. The byte-order handling inside the base-point
     multiplication was not read (open question O-6).
   - 2: as 0, scheme Schnorr.
   - any other value: no case matches; the function returns a WIF with a nil
     private key, nil public key and scheme 0, and **no error** (quirk Q-10).

`NewWIF(privKey, net, scheme)` computes the public key the same way and
returns `unsupported signature type '<n>'` for an unknown scheme
(`wif.go:66-84`). `WIF.String` is section 4.3.

### 5.6 `dcrutil.AppDataDir(appName, roaming)` (`appdata.go:21-99`)

1. If `appName` is empty or `.`, return `.`.
2. Strip one leading `.` from `appName`; form `Upper` (first **byte**
   upper-cased through `unicode.ToUpper(rune(appName[0]))`, not the first
   UTF-8 rune) and `lower` (first byte lower-cased the same way)
   (`appdata.go:28-30`). An `appName` of `..` is not caught by step 1 and
   becomes `.` after the strip.
3. `home` = Go's `os.UserHomeDir()` result, empty on error.
4. By operating system:
   - windows: `appData = %LOCALAPPDATA%`; if `roaming` or it is empty,
     `appData = %APPDATA%`; if non-empty return `appData\Upper`.
   - darwin: if `home` is non-empty return
     `home/Library/Application Support/Upper`.
   - plan9: if `home` is non-empty return `home/lower`.
   - anything else: if `home` is non-empty return `home/.lower`.
5. Otherwise return `.`.

dcroxide's node has this logic for `dcrd`
(`dcroxide:crates/dcroxide-node/src/config.rs:1189`); it must be lifted out
or rewritten for `dcrwallet`.

### 5.7 `txsort.InPlaceSort(tx)` (`txsort/txsort.go:39-115`)

Sorts inputs, then outputs, each with Go's `sort.Sort` (not stable).

- Inputs: by previous-outpoint tree ascending; then, when the hashes are
  equal, by index ascending; otherwise by the hash compared bytewise **after
  reversing it** (so in displayed, big-endian order).
- Outputs: by value ascending; then script version ascending; then script
  bytes lexicographically.

Equal elements can only be identical inputs or identical outputs, so the
instability of `sort.Sort` is not observable in the serialized transaction;
it would be observable to a caller tracking which of two identical outputs
is "its" index (mixclient does track mixed output indices; spec 12 owns that
analysis).

### 5.8 `secp256k1.GeneratePrivateKeyFromRand(r)` (`privkey.go:52-86`)

1. Read exactly 32 bytes from `r`.
2. Interpret them as a big-endian integer. If it is zero or not below the
   group order, go to 1.
3. Return the key.

The number of bytes consumed from `r` is part of the mixing protocol: the
mix client draws its keys from a ChaCha20 stream seeded per session, and a
peer that is later blamed has its seed revealed and its keys recomputed by
everyone else (`dcrd/mixing@v0.7.5/keyagreement.go:34`,
`mixclient/blame.go:237`).

### 5.9 `secp256k1.GenerateSharedSecret(priv, pub)` (`ecdh.go:14-21`)

Returns the 32-byte big-endian affine X coordinate of `priv·pub`. No hashing.

### 5.10 Adopting a dcroxide crate (procedure)

For each dcrd identifier a wallet spec names:

1. Look it up in section 3. If the status is `ok`, call the Rust item.
2. If `shape`, write the adaptation at the call site or in a thin wallet
   module; do not fork the crate for it.
3. If `missing` and the item belongs to a package dcroxide ports, add it to
   the dcroxide crate with a test pinned through the Go oracle, under
   dcroxide's own rules (section 12.1), rather than re-implementing it in the
   wallet.
4. If the package has no port, it goes into one of the new crates of
   section 12.4.
5. When a wallet-visible string comes out of a dcroxide error type, check
   section 8 before printing it.

### 5.11 The three small `wire` functions dcroxide-wire lacks

`MsgTx.PkScriptLocs()` (`wire@v1.8.0/msgtx.go:1125-1157`):

1. If the transaction has no outputs, return an empty result (Go returns
   `nil`).
2. `n = 4 + varint_size(len(TxIn)) + varint_size(len(TxOut)) + 41 * len(TxIn)`
   (41 is the prefix size of one input: 32-byte hash, 4-byte index, 1-byte
   tree, 4-byte sequence).
3. For each output in order: `n += 8 + 2 + varint_size(len(PkScript))`;
   record `n` as this output's offset; `n += len(PkScript)`.

The arithmetic assumes the prefix comes first, which holds for the full and
the no-witness serializations; the function does not look at `SerType`
(the Go source carries a TODO saying so).

`MsgVersion.AddUserAgent(name, version, comments...)`
(`wire@v1.8.0/msgversion.go:295-311`): form `name:version`; when comments are
given append `(` + comments joined by `; ` + `)`; the new user agent is the
existing `UserAgent` + that + `/`; if it is longer than 256 bytes fail with
`ErrUserAgentTooLong` (description `user agent too long [len N, max 256]`),
if it is not strict ASCII fail with `ErrMalformedStrictString` (description
`user agent is not strict ASCII`); otherwise store it. Both errors are
`MessageError`s whose `Error()` text is the function name, `: ` and the
description, so `MsgVersion.validateUserAgent: user agent too long [len N, max 256]`
(`msgversion.go:276-287`, `error.go:271-276`). `NewMsgVersion`
starts from `/dcrwire:1.0.0/` (`msgversion.go:22`), so dcrwallet announces
`/dcrwire:1.0.0/dcrwallet:<version>/` (`p2p/peering.go:38-41`, `:237-238`).
dcrwallet discards `AddUserAgent`'s return value (`p2p/peering.go:238`).

`WriteVarString(w, pver, s)` (`wire@v1.8.0/common.go:936-950`): the
variable-length integer encoding of `len(s)` followed by the bytes of `s`.

## 6. Constants, limits and defaults

| Name | Value | Source | Depends on it |
|---|---|---|---|
| `hdkeychain.RecommendedSeedLen` | 32 bytes | `hdkeychain/v3@v3.1.3/extendedkey.go:30` | default seed size at wallet creation |
| `hdkeychain.MinSeedBytes` / `MaxSeedBytes` | 16 / 64 | `extendedkey.go:40-44` | seed validation, error text `seed length must be between 128 and 512 bits` |
| `hdkeychain.HardenedKeyStart` | `0x80000000` | `extendedkey.go:36` | account, coin-type and purpose derivation |
| `serializedKeyLen` | 78 | `extendedkey.go:50` | extended key string |
| max extended key string length | 113 characters | `extendedkey.go:552-553` | `ErrInvalidKeyLen` |
| master HMAC key | ASCII `Bitcoin seed` | `extendedkey.go:99` | every wallet seed |
| `dcrutil.AtomsPerCoin` | 1e8 | `dcrutil/v4@v4.0.3/const.go` | every amount conversion |
| `dcrutil.AtomsPerCent` | 1e6 | same | not used by dcrwallet |
| `dcrutil.MaxAmount` | 21e6 × 1e8 atoms | same | amount range checks |
| WIF decoded length / max string length | 39 bytes / 54 characters | `dcrutil/v4@v4.0.3/wif.go:113-114` | `importprivkey`, `signrawtransaction`, `dumpprivkey` |
| `dcrec.STEcdsaSecp256k1` / `STEd25519` / `STSchnorrSecp256k1` | 0 / 1 / 2 | `dcrec@v1.0.1` | WIF scheme byte, sign calls |
| `wire.MaxBlocksPerMsg` | 500 | `wire@v1.8.0/msgblock.go:24` | batch size in address discovery (`wallet/discovery.go:70-73`) |
| `wire.TxVersion` / `TxVersionSeqLock` / `TxVersionTreasury` | 1 / 2 / 3 | `wire@v1.8.0/msgtx.go:20-28` | transaction construction |
| `wire.MaxUserAgentLen` | 256 | `wire@v1.8.0/msgversion.go:19` | version message |
| dcrwallet P2P protocol version (`Pver`) | 11 (`BatchedCFiltersV2Version`) | `p2p/peering.go:48` | dcroxide-wire is called with `pver = 11`, not its `PROTOCOL_VERSION` 12 |
| dcrwallet minimum peer protocol version | 9 (`RemoveRejectVersion`) | `p2p/peering.go:45` | peer admission |
| `blockcf2.B` / `blockcf2.M` | 19 / 784931 | `gcs/v4@v4.1.1/blockcf2/blockcf.go:33-38` | every committed filter |
| `gcs.KeySize` | 16 | `gcs/v4@v4.1.1/gcs.go:69` | filter keys |
| `connmgr.Halflife` / `Lifetime` | 60 s / 1800 s | `connmgr/v3@v3.1.4/dynamicbanscore.go:18-25` | peer ban score decay |
| addrmgr `serialisationVersion` | 1 | `addrmgr/v3@v3.0.0/addrmanager.go:218` | `peers.json` |
| addrmgr `dumpAddressInterval` | 10 minutes | `addrmgr/v3@v3.0.0/addrmanager.go:161` | the save timer the wallet must run itself |
| `mixing.MinPeers` / `MaxPeers` / `MaxMcount` / `MaxMtot` / `MaxMixTxSerializeSize` | 4 / 64 / 16 / 1024 / 100,000 | `mixing@v0.7.5/client.go:9`, `limits.go:26-38` | mix sessions |
| mixpool epoch | 10 minutes; 3 minutes on testnet3 | `mixing@v0.7.5/mixpool/mixpool.go:280`, `:289-292` | mix session timing; must equal every other participant's |
| mixpool `minconf` / `feeRate` / `maxRelayFeeMultiplier` | 1 / 0.0001e8 atoms per kB / 1e4 | `mixpool/mixpool.go:30-32` | pair request acceptance |
| mixpool `earlyKEDuration` / `lateKEDuration` | 5 s / 20 min | `mixpool/mixpool.go:33-34` | key exchange acceptance window |
| mixpool `maxOrphans` / `maxPostEvictionOrphans` / `maxMessagesPerIdentity` | 250 / 187 / 896 | `mixpool/mixpool.go:36-51` | orphan and per-identity limits |
| `mixing.ScriptClassP2PKHv0` | `P2PKH-secp256k1-v0` | `mixing@v0.7.5/scriptclass.go:15` | pair request script class string |
| `sntrup4591761` `PublicKeySize` / `PrivateKeySize` / `CiphertextSize` / `SharedKeySize` | 1218 / 1600 / 1047 / 32 bytes | `sntrup4591761@v0.0.0-20220309191932-9e0f3af2f07a/sntrup4591761.go:23-26` | key exchange and ciphertext message sizes |
| `secp256k1.PrivKeyBytesLen` | 32 | `dcrec/secp256k1/v4@v4.4.1/privkey.go:103` | WIF, key serialization |
| `dcrutil.BlockValid` | `1 << 0` | `dcrutil/v4@v4.0.3/bitflags.go:17` | header vote-bits test for regular-tree validity |
| `dcrutil.BlockHeightUnknown`, `TxIndexUnknown` | -1, -1 | `dcrutil/v4@v4.0.3/block.go:24`, `tx.go:24` | not used by dcrwallet |
| seeder request timeout / body limit / entries per seeder | 1 minute / 4,096 bytes / 16 | `p2p/peering.go:368`, `:395`, `:398` | SPV peer discovery (dcrwallet's own code, section 3.15) |
| `--circuitlimit` default | 32 | `config.go:61`, `:376` | `socks.NewPool` bound on open proxied connections under `--torisolation` |
| dcroxide `MAX_KES_PER_IDENTITY` | 64 (no dcrd counterpart) | `dcroxide:crates/dcroxide-mixing/src/mixpool.rs:225` | wallet mixpool acceptance if the divergence is kept |
| dcroxide toolchain and MSRV | Rust 1.99.1, edition 2024, resolver 3 | `dcroxide:Cargo.toml:2`, `:42-51`; `dcroxide:rust-toolchain.toml:18` | every crate built in or against the workspace |
| dcroxide release profile | `panic = "abort"`, `overflow-checks = false`, `codegen-units = 1`, `lto = "thin"`, `debug = "line-tables-only"` | `dcroxide:Cargo.toml:57-94` | section 12.1 |
| dcroxide lints | `unsafe_code = "forbid"`, `missing_docs = "deny"`, `clippy::all = "warn"`, `clippy::arithmetic_side_effects = "warn"`; CI runs clippy with `-D warnings` | `dcroxide:Cargo.toml:146-152`; `dcroxide:.github/workflows/ci.yml:56` | section 12.1 |
| dcroxide extra profiles | `dist` (release + `strip = "debuginfo"`), `soak` (release + `panic = "unwind"`), `soak-checked` (soak + `overflow-checks = true`), `soak-asserts` (soak + `debug-assertions`, `overflow-checks`) | `dcroxide:Cargo.toml:113-144` | release artifacts are cut from `dist`; a wallet signature or derivation soak reuses the `soak*` profiles |
| dcroxide workspace package fields | version 0.1.0, edition 2024, licence ISC, `exclude = ["fuzz"]` (a separate workspace with its own lock) | `dcroxide:Cargo.toml:40-53` | crates added to the workspace inherit them |

## 7. Concurrency and lifecycle

The Go packages hide goroutines, mutexes and timers that dcrwallet relies on
without naming. The dcroxide ports of the same packages are, by design,
synchronous state machines with injected clocks and random sources; the node
binary supplies the threads (`dcroxide:docs/adr/README.md`, ADR-0005: OS
threads over channels, no async runtime). A wallet that takes the crates
takes on the following.

| Go behaviour dcrwallet relies on | Source | dcroxide form | What the wallet adds |
|---|---|---|---|
| `addrmgr.Start` loads `peers.json` and starts a goroutine that saves it every 10 minutes; `Stop` closes the goroutine, which saves once more | `addrmgr/v3@v3.0.0/addrmanager.go:401-420`, `:609-640` | `load_peers`, `save_peers` called by the owner (`manager.rs:1381-1435`) | a mutex around the manager, a 10-minute save timer, a final save at shutdown |
| every `AddrManager` method is safe for concurrent use | same file, `a.mtx` | methods take `&self`/`&mut self` | the mutex above |
| `DynamicBanScore` locks itself and reads `time.Now` | `connmgr/v3@v3.1.4/dynamicbanscore.go:64-110` | caller passes `now_unix` | per-peer lock and a clock read |
| `SubsidyCache` is safe for concurrent use (`sync.RWMutex`) | `blockchain/standalone/v2@v2.3.0/subsidy.go:93-100` | `&mut self` (`subsidy.rs:91-97`) | a mutex, or one cache per owner |
| `mixpool.Pool` is safe for concurrent use; `Receive` blocks on a broadcast until enough messages arrive or the context is cancelled; expiry can run in a background goroutine | `mixing@v0.7.5/mixpool/mixpool.go:162-229`, `:430-470`, `:926` | `&mut self` acceptance; `receive` is a snapshot; expiry is a call the owner makes | mutex plus condition variable (or channel) signalled on every accepted message; the wait loop around `receive`; the epoch timer |
| `mixclient.Client.Run` owns an epoch ticker and one goroutine group per pairing; `Dicemix` blocks the caller until its coinjoin completes or fails | `mixing@v0.7.5/mixclient/client.go:486`, `:1006` | none | the whole client (spec 12) |
| `crypto/rand`'s package-level generator is seeded in `init` and locked per call | `crypto/rand@v1.0.1` | lazily seeded `OnceLock<Mutex<Prng>>`; explicit `rand::init()` (`rand.rs:435`) | call `init()` at startup so an entropy failure is fatal there, as in Go |
| `dcrjson` registry is a process global guarded by a mutex | `dcrjson/v4@v4.2.0` | explicit `Registry` value | build once at startup, share immutably |
| `slog` loggers are safe for concurrent use; each has an atomic level | `slog@v1.2.0/log.go:495-507` | node-crate globals (`logging.rs:39-45`) | section 12.5 |
| `socks.Pool` counts open proxied connections under a mutex, refuses a dial with `maximum number of connections reached` when `numOpen + 1 > maxOpen`, and decrements once per connection on `Close` (`sync.Once`) or on a failed dial | `go-socks@v1.1.0/socks/pool.go:14-61`, `conn.go:29-39` | none (`dcroxide-node/src/socks.rs` has no pool) | the counter, the limit and the release-on-close hook around whatever SOCKS dialer is used |
| `mixpool.Observer` strike state has its own `sync.RWMutex`; mixclient calls `CheckPrevEpoch` and `ExcludePRs` from its epoch loop and never starts `Observer.Run` | `mixing@v0.7.5/mixpool/observer.go:73-78`, `:107`, `mixclient/client.go:920-945` | observer methods on `Pool` (`check_prev_epoch` takes `&mut self`) | covered by the pool mutex |
| `ExtendedKey.pubKeyBytes` memoizes without a lock | `hdkeychain/v3@v3.1.3/extendedkey.go:157-170` | — | in Rust compute eagerly or behind `OnceLock`; Go's lazy write is a data race when two goroutines share one private key, which is not behaviour to reproduce |

Shutdown ordering that the wallet owns once it drives these crates itself:
stop the P2P or RPC syncer, then save `peers.json`, then close the database.
Go's `addrmgr.Stop` returns only after the final save
(`addrmanager.go:628-640`).

dcroxide's release profile aborts on panic (`dcroxide:Cargo.toml:57-82`).
Several ported functions keep dcrd's panics on malformed input by contract —
`sign_tx_output` indexes `addresses[0]` unguarded as dcrd does
(`dcroxide:crates/dcroxide-txscript/src/sign.rs:598-603`), and the stake
extraction helpers panic on transactions that did not pass the matching
`is_*` check (`dcroxide:crates/dcroxide-stake/src/lib.rs:20-22`). In Go a
panic inside an RPC handler goroutine kills dcrwallet too (there is no
recover in the handlers this document could confirm; UNVERIFIED, spec 16),
so the wallet must keep the same call discipline: classify first, then
extract.

## 8. Errors

Error types of the crates the wallet would link, and how faithful their text
is to Go's. "Kind" means a `kind_name()` equal to dcrd's `ErrorKind` string.

| Crate | Error type | Text fidelity recorded by dcroxide | Wallet surfaces that print it |
|---|---|---|---|
| `dcroxide-wire` | `WireError` (`kind_name`, `message_text`) | Go's text for every error the transaction, header, block and mix-message decoders produce; kind names only for the remaining P2P codecs and framing, except the wrong-network text (`dcroxide:docs/PARITY.md:1019`) | `decoderawtransaction`, `sendrawtransaction`, gRPC decode calls; P2P log lines |
| `dcroxide-chainhash` | `HashError` | Go's texts (`lib.rs:51-71`) | every RPC taking a hash |
| `dcroxide-txscript` | `ScriptError { kind, description }` | dcrd's text (returned by dcrd's `sendrawtransaction`; `dcroxide:docs/PARITY.md:1021`) | `signrawtransaction` per-input errors, script validation |
| `dcroxide_txscript::stdaddr` | `AddrError { kind, description }` | dcrd's text including Go's `%q` quoting (`stdaddr.rs:762-800`) | every RPC taking an address |
| `dcroxide_txscript::stdscript` | `StdScriptError` | kinds; one relabelled case (`dcroxide:docs/PARITY.md:1078`) | multisig script creation |
| `dcroxide_txscript::sign` | `SignError(String)` | dcrd's sign package returns plain string errors; the port's strings were not compared against Go's here — UNVERIFIED | `signrawtransaction`, transaction authoring failures |
| `dcroxide_dcrec::secp256k1` | `Error` (public key parse) | dcrd's `Error()` descriptions (`dcroxide:docs/PARITY.md:1015`) | `importpubkey`-style calls, address construction |
| `dcroxide_dcrec::secp256k1::ecdsa` | `Error` | **kind names only** (`ecdsa.rs:111-115`); dcrd's daemon never prints these | wherever dcrwallet returns a `RecoverCompact` or DER parse error to a client (spec 16 decides per method) |
| `dcroxide_dcrec::secp256k1::schnorr` | `Error`, `PubKeyError` | kind names; dcrd's text is formatted at the one node call site | SPV treasury-spend validation logs |
| `dcroxide-stake` | `RuleError { kind, description }` | kinds; descriptions ported | ticket and vote validation |
| `dcroxide-standalone` | `RuleError` | kinds | header validation |
| `dcroxide-gcs` | `Error` | kinds | filter decode |
| `dcroxide-mixing` | `MixError`, `PoolError` (`RuleKind`) | dcrd's messages for the pool rules (`dcroxide:docs/PARITY.md:1035`) | mixing log lines, `MissingOwnPRError` handling |
| `dcroxide-addrmgr` | `AddrError`; `String` from load and save | Go's `*os.PathError` and JSON texts on Unix | log lines only |
| `dcroxide-dcrjson` | `DcrjsonError`, `RPCError`, `gojson::JsonError` | byte-exact against Go 1.26's `encoding/json` (`dcroxide:docs/PARITY.md:1038`) | every JSON-RPC parameter error |
| `dcroxide-certgen` | `String` | — | startup failure text |
| `dcroxide-base58` | `CheckError` | kinds | through stdaddr |

No crate uses `anyhow` or `thiserror`; each error type implements
`core::error::Error` and `Display` by hand. dcrwallet's own error model
(`decred.org/dcrwallet/v5/errors`, kinds and `errors.E`) is wallet code and
wraps these; nothing in dcroxide corresponds to it.

Errors of the unported packages, with their exact texts, are in sections 3.9,
5.5 and 5.11. Collected here with the remaining ones this document names:

| Package | Error | Text | Kind of value |
|---|---|---|---|
| `dcrutil` | `ErrMalformedPrivateKey` | `malformed private key` | sentinel (`errors.New`), `wif.go:25` |
| `dcrutil` | `ErrChecksumMismatch` | `checksum mismatch` | sentinel, `wif.go:29` |
| `dcrutil` | `ErrWrongWIFNetwork` | `WIF is not for the network identified by %#04x` of the expected `[2]byte` | `[2]byte` error type, `wif.go:34-40` |
| `dcrutil` | from `NewWIF` | `unsupported signature type '%v'` | `fmt.Errorf`, `wif.go:79` |
| `dcrutil` | from `NewAmount` | `invalid coin amount` | `errors.New` built per call (not a comparable sentinel), `amount.go:85` |
| `edwards` | from `PrivKeyFromScalar` | `bad private scalar size`, `not on subgroup (>N)`, `zero or negative scalar`, `scalarbase mult failure to get pubkey` | section 5.5 |
| `mixing` | from `KX.pqSharedKey` (unexported) | `sntrup4591761: decapsulate failure` | `errors.New`, `keyagreement.go:85`; replaced before it reaches a caller of `KX.SharedSecrets` (next row) |
| `mixing` | `DecapsulateError` from `KX.SharedSecrets` | `decapsulate failure of ciphertext by peer %d` of `SubmittingIndex` | pointer type carrying `SubmittingIndex uint32`, `errors.go:20-28`, `keyagreement.go:164-170` |
| `mixpool` | `ErrSecretsRevealed` | `secrets revealed by peer` | sentinel, `mixpool/errors.go:118` |
| `mixpool` | `MissingOwnPRError` | `KE identity's own PR is missing from mixpool` | pointer type carrying `MissingPR`, `mixpool/errors.go:126-132` |
| `mixpool` | rule errors | `change output is dust`, `mix output is dust`, `not enough input value, or too low fee`, `too high fee`, `message count must be positive`, `invalid script`, `invalid session ID`, `invalid message signature`, `invalid total mix amount`, `invalid UTXO ownership proof`, `pair request contains no UTXOs`, `peer position cannot be a valid seen PRs index` | bannable sentinels, `mixpool/errors.go:54-102`; `dcroxide_mixing::RuleKind` carries the same twelve texts (`mixpool.rs:492-509`) |
| `go-socks` | `ErrPoolMaxConnections` | `maximum number of connections reached` | sentinel, `socks/pool.go:15` |

dcrwallet wraps the `dcrutil` and `hdkeychain` errors with its own kinds
(`errors.E(errors.Encoding, err)` and similar; the owning specs give the
kind per call site). A Rust `dcrutil`/`hdkeychain` crate must expose each
sentinel as a distinct, comparable variant with the text above as its
`Display`, because the text reaches RPC clients inside the wrapped message.

## 9. Compatibility-critical behaviours

1. **HD derivation uses dcrd's legacy child function.** Every key in an
   existing `wallet.db` was derived with `Child`, which stores child private
   keys with leading zero bytes removed and lays a short parent key
   left-aligned in the next hardened HMAC input (section 5.2). A strict
   BIP32 implementation produces different accounts for roughly one seed in
   130 on the standard path (`dcr-rs:src/hd.rs:26-30`). An existing seed must
   restore to the same addresses.
2. **Extended key strings round-trip as dcrd's do**, including the rule that
   the key kind comes from the key data byte and that a parsed private key is
   never stored stripped (section 5.3). Account xpubs in the database,
   `getmasterpubkey`, `importxpub` and Decrediton depend on the exact string.
3. **WIF strings** use a single BLAKE-256 checksum and the scheme byte
   (section 4.3). `dumpprivkey` output must import into dcrwallet and the
   reverse.
4. **Amount rendering** in JSON-RPC results is the `float64` quotient
   formatted as Go's `encoding/json` does (section 4.4). dcrctl and
   Decrediton parse these numbers; scripts compare them as text.
5. **Transaction, header and block bytes and hashes** must equal dcrd's.
   dcroxide-wire is differential-tested for this; the wallet must call it
   with protocol version 11 where dcrwallet does (`p2p/peering.go:48`).
6. **Signature bytes.** ECDSA and Schnorr signatures are RFC 6979
   deterministic; `dcroxide_txscript::sign` matches dcrd byte for byte, so
   dcr-purse produces the signature scripts dcrwallet would for the same
   transaction and keys. Tests and tools comparing raw signed transactions
   depend on it.
7. **`signmessage` / `verifymessage`** use the compact signature format and
   the magic prefix written with `WriteVarString` (`wallet/wallet.go:2011-2012`,
   `:2041-2042`); `ecdsa::sign_compact` and `recover_compact` match dcrd.
8. **Committed filter matching** (`blockcf2` parameters, key derivation,
   `MatchAny`) decides which blocks an SPV wallet downloads; a false negative
   loses transactions. dcroxide-gcs is the same code as gcs v4.1.1.
9. **Stake transaction classification** (`DetermineTxType`, `IsSStx`,
   commitment extraction) decides what the wallet records as tickets, votes
   and revocations in the database.
10. **Mix message identities, signatures, session ids and the DC-net math**
    must match other participants bit for bit; `dcroxide-mixing` covers these.
    The key agreement, including how many bytes each key generation draws
    from the session PRNG (section 5.8) and the sntrup4591761 key and
    ciphertext bytes derived from that stream, must also match, and has no
    Rust implementation yet.
11. **Mixpool acceptance rules** on the wallet side must not be stricter
    than dcrwallet's in a way honest peers trip (section 3.14, decision D-5).
12. **The user agent** sent to peers is `/dcrwire:1.0.0/dcrwallet:<version>/`
    (section 5.11); some operators filter on it.
13. **JSON-RPC command parsing, help and usage text** come from dcrjson's
    reflection over command structs; `dcroxide-dcrjson` reproduces them when
    given equivalent descriptors, with Go's JSON error texts.
14. **`peers.json`** written by dcr-purse must load in dcrwallet v5 when a
    user switches back, or be discarded cleanly: dcrwallet deletes a file it
    cannot parse (section 4.5). A Tor v3 entry (possible only if dcr-purse
    ever speaks protocol 12) would make dcrwallet discard the file.
15. **TLS certificate files** generated on first start must be accepted by
    dcrctl, Decrediton and Go gRPC clients; `dcroxide-certgen` reproduces
    certgen's shape for P-256, P-521 and Ed25519, and dcrwallet's default is
    Ed25519 (`internal/cfgutil/curve.go:29`, `config.go:363`).
16. **The application data directory** is derived from the name `dcrwallet`
    by the rules of section 5.6; a drop-in replacement must find the existing
    directory on every operating system.
17. **The seeder request** is `GET https://<seeder>/api/addrs?services=<n>`
    with no other query parameter (section 3.15). Reusing dcroxide's
    `seed_addrs` with `ipversion` or `pver` filters set changes what a seeder
    operator sees and which addresses come back.
18. **The mixing epoch** (10 minutes, 3 on testnet3) and the pool's
    acceptance constants (section 6) are shared protocol state with every
    other mixing participant; they come from the pool crate, not from
    configuration.
19. **Key-exchange stream consumption.** `NewKX` draws the secp256k1 key and
    then the sntrup4591761 key pair from one session stream, and
    `Encapsulate` draws once per peer in slice order (section 3.14). Blame
    assignment recomputes these from a revealed seed, so order and byte
    counts are part of the protocol.
20. **Under a SOCKS proxy the version message names the dialed address.**
    go-socks returns a connection whose `RemoteAddr()` is a `ProxiedAddr`
    holding the host and port that were requested through the proxy;
    `newNetAddress` parses that host as an IP for the `AddrYou` field of the
    version message, and blanks it (nil IP, port 0) when it is loopback,
    private, CGNAT or not global unicast, or when the host is not an IP
    literal and parses to nil (`p2p/peering.go:208-228`;
    `go-socks@v1.1.0/socks/conn.go:48-53`, `dial.go:275`). A dialer that
    reports the proxy's socket address instead changes the bytes sent to the
    peer.
21. **`--circuitlimit`** bounds concurrently open proxied connections when
    `--torisolation` is set; the dial error text at the limit is
    `maximum number of connections reached` and appears in log lines.
22. **`Amount.String` unit suffix** is ` DCR` (one space); log lines and
    error texts that embed amounts depend on it (section 4.4).
23. **gRPC `SignHashes` signs hashes of any length.** The handler passes each
    client-supplied byte string to `ecdsa.Sign` without a length check
    (`internal/rpc/rpcserver/server.go:3763`, `wallet/wallet.go:1998-2000`).
    A hash longer than 32 bytes is cut to its first 32; a shorter one is
    left-padded with zero bytes, both for the RFC 6979 nonce and for the
    signed scalar (quirk Q-31). A client that sends a 20-byte or 64-byte
    digest gets a DER signature today and must get the same bytes.
24. **JSON-RPC request envelopes are decoded by Go's `encoding/json` into
    `dcrjson.Request`** (case-insensitive field names, last duplicate wins,
    unknown fields ignored, type errors naming `Request.<field>`), and three
    websocket replies carry `"jsonrpc":""` (quirk Q-32). The first exists in
    dcroxide only inside `dcroxide-rpc`; the second nowhere (section 3.19;
    spec 16 owns the byte tables).
25. **Extended private keys serialize left-padded to 32 bytes** although
    they are held stripped in memory (`paddedAppend`, section 3.9), so the
    `dprv`/`tprv` string of a stripped key is the same as that of its
    32-byte form; only the in-memory length, and through it the hardened
    children, differ (item 1, quirk Q-6).

## 10. Quirks

- **Q-1. Two `go.mod` requirements are not in the binary.** `rpcclient/v8`
  is imported only by an example program (`rpc/rpc_example/main.go:15`) and
  `blockchain/v5` only by a test (`wallet/reorg_test.go:12`). Neither needs a
  Rust counterpart.
- **Q-2. Seeder list differs by one host.** `chaincfg/v3@v3.3.0` lists three
  mainnet seeders; dcroxide's `mainnet_params()` lists four, adding
  `dcr-seed.jz.bz` from a later dcrd commit
  (`dcroxide:crates/dcroxide-chaincfg/src/mainnet.rs:286-291`). dcrwallet
  reads the list through `Params.Seeders()` (`p2p/peering.go:348`). A wallet
  built on dcroxide-chaincfg queries one more seeder than dcrwallet at the
  pin. This is a network-visible difference to reproduce or accept
  (decision D-4).
- **Q-3. WIF uses one hash, everything else two.** Section 4.3.
- **Q-4. An extended key's kind is read from its data, not its version.**
  `NewKeyFromString` accepts a string with private version bytes and public
  key data as a public key, and re-serializes it with the public version
  (`hdkeychain/v3@v3.1.3/extendedkey.go:574-612`). dcr-rs rejects such a
  string (`dcr-rs:src/hd.rs:480-485`).
- **Q-5. `PrivKeyFromBytes` never fails.** It reduces the input modulo the
  group order, truncating inputs longer than 32 bytes, and accepts zero
  (`dcrec/secp256k1/v4@v4.4.1/privkey.go:27-46`). `DecodeWIF` therefore
  accepts a WIF whose 32 key bytes are zero or at or above the group order,
  and `importprivkey` proceeds with the reduced key. dcroxide rejects such
  keys (`dcroxide:crates/dcroxide-dcrec/src/secp256k1.rs:336-360`). A
  faithful wallet needs the reducing constructor; the alternative is a
  deliberate divergence on inputs no honest tool produces.
- **Q-6. Serializing and reparsing a derived private key changes its
  hardened children** when the key had a leading zero byte (about 1 in 256).
  Section 5.3; `dcr-rs:src/hd.rs:486-497` documents the same.
- **Q-7. `SerializedPrivKey` can return fewer than 32 bytes**
  (`extendedkey.go:422-428`). Callers that need 32 bytes pad themselves or go
  through `secp256k1.PrivKeyFromBytes`, which accepts short input.
- **Q-8. A child whose private sum is zero is not rejected**; under `Child`
  it becomes an empty key, which `String` then renders as
  `zeroed extended key`. Probability about 2^-256; dcr-rs rejects it instead
  (`dcr-rs:src/hd.rs:118-124`).
- **Q-9. `Neuter` shares buffers; `Zero` on the private key corrupts the
  neutered key's public key bytes, chain code and parent fingerprint**
  (section 5.4). Whether
  any dcrwallet call site zeroes a private key while still using a key
  neutered from it was not traced here (open question O-4); a Rust port with
  owned buffers changes the outcome exactly in that case.
- **Q-10. `DecodeWIF` with an unknown scheme byte returns success** with nil
  keys and scheme ECDSA (`dcrutil/v4@v4.0.3/wif.go:138-161`). The caller then
  fails later, with a different message per call site.
- **Q-11. `MsgTx::tx_hash` and `serialize` are infallible in dcroxide** and
  write a negative outpoint tree as its byte, where dcrd's `Bytes` returns
  `ErrNegativeTxTree` and `TxHash` panics
  (`dcroxide:docs/PARITY.md:1019`). dcrwallet builds inputs from RPC
  parameters; each such site must validate the tree before hashing if the
  Go behaviour there is an error or a crash.
- **Q-12. `Amount.String` goes through `float64`.** For values far beyond
  `MaxAmount` (from roughly 2^26 coins, where adjacent `float64` values are
  more than 1e-8 apart) digits are lost; such values reach `String` only
  through arithmetic on untrusted amounts. `NewAmount` on a
  value outside `int64` range is a Go float-to-int conversion with
  implementation-defined result; dcroxide records its own handling of that
  case for dcrd's flags and RPCs (`dcroxide:docs/PARITY.md:1079`).
- **Q-13. dcroxide's mixpool is deliberately stricter than dcrd's in three
  places** (section 3.14).
- **Q-14. Invalid base58 is a length error.** `base58.Decode` returns an
  empty slice for any invalid character, so a mistyped extended key reports
  `ErrInvalidKeyLen` and a mistyped WIF `malformed private key`, never a
  character error.
- **Q-15. Depth is 16 bits in memory and 8 on the wire.** `String` writes
  `depth % 256` (`extendedkey.go:447`); `NewKeyFromString` reads one byte.
- **Q-16. `ErrWrongWIFNetwork` names the expected network**, not the one
  found in the string (`wif.go:134-136`).
- **Q-17. `hdkeychain.GenerateSeed` reads Go's standard `crypto/rand`**, not
  dcrd's `crypto/rand` package (`extendedkey.go:14-16`, `:629-632`).
- **Q-18. dcroxide's `ecdsa::Error` displays kind names** such as
  `ErrSigTooShort` rather than dcrd's sentences (`ecdsa.rs:111-115`).
- **Q-19. dcroxide's `SubsidyCache` is not shareable** without a lock
  (section 7); dcrwallet shares one across goroutines.
- **Q-20. The `sign` module rejects Ed25519 keys that are not the 64-byte
  `seed ‖ pubkey` form** (`sign.rs:101-114`), as dcrd's
  `edwards.PrivKeyFromBytes` does, while `DecodeWIF` yields a 32-byte scalar
  for an Ed25519 WIF (section 5.5). So `signrawtransaction` with an Ed25519
  WIF builds an address map entry (`internal/rpc/jsonrpc/methods.go:5049-5054`)
  whose key the signer cannot use. That is dcrwallet's behaviour; whether the
  resulting error text matches is for spec 16.
- **Q-21. The wallet's mixpool validates no UTXOs.** `NewPool` discovers
  UTXO validation by type-asserting its `BlockChain` argument to
  `UtxoFetcher`; the wallet's `mixpoolBlockchain` has only `CurrentTip` and
  `ChainParams` (`wallet/mixing.go:38-49`), so pair requests are accepted
  without checking that their inputs exist, are unspent or are mature
  (`mixing@v0.7.5/mixpool/mixpool.go:294-296`). The Rust wallet passes
  `None` for the fetcher; passing one would make its pool stricter than
  dcrwallet's.
- **Q-22. dcrwallet seeds with its own HTTPS client, not dcrd's.** Section
  3.15. The response decoder stops at the first JSON error for a seeder and
  keeps what it had; an entry with a bad host is skipped, not fatal
  (`p2p/peering.go:399-424`).
- **Q-23. dcroxide's SOCKS port drops two go-socks behaviours dcrwallet
  uses**: the connection pool with a maximum (`socks.NewPool`,
  `config.go:703`) and the `ProxiedAddr` remote address
  (`dcroxide:crates/dcroxide-node/src/socks.rs:11-14` says the bound address
  is read and discarded). Compatibility items 20 and 21 depend on both.
- **Q-24. `mixclient.UseLogger` does not exist.** dcrwallet's logger setup
  keeps a commented-out call marked `XXX` (`log.go:42`); the mix client's
  logger is set per instance (`wallet/wallet.go:5540`). The subsystem tag
  still exists in the logger table, so `--debuglevel` accepts it.
- **Q-25. dcroxide's `crypto/rand` port differs from dcrd's generator in
  three recorded ways**: it rekeys after 4 MiB only (dcrd also rekeys every
  20 s), it ignores an entropy failure after startup and rekeys from the
  outgoing keystream (Go's runtime kills the process), and
  `dcroxide_connmgr::SystemCsprng::uint64n` and
  `dcroxide_addrmgr::SystemRng::int_n` reduce by rejection sampling rather
  than through `Uint64N` (`dcroxide:docs/PARITY.md:1110-1112`). None is
  observable from outside the process; a wallet that requires
  abort-on-entropy-failure for key generation must check `getrandom` itself
  at the points where it creates seeds and keys.
- **Q-26. dcroxide has two Tor resolvers and the reusable one is not the one
  the node runs.** `dcroxide_addrmgr::tor_lookup_ip` reproduces dcrd's
  `TorLookupIP` exactly, including an unchecked single read and off-length
  IPv6 answers, and is exercised only by vectors; the node calls its own in
  `dcroxide-node/src/socks.rs` with full reads, one absolute deadline and a
  16-byte IPv6 check (`dcroxide:docs/PARITY.md:1123`). dcrwallet calls
  `connmgr/v3`'s `TorLookupIP` (section 3.16); which behaviour the wallet
  takes is part of decision D-3.
- **Q-27. `WIF.String` does not pad.** It appends `privKey` as given to
  `NewWIF` (`dcrutil/v4@v4.0.3/wif.go:167-181`). dcrwallet's one daemon call
  site passes `privKey.Serialize()` of a `secp256k1.PrivateKey`, which is
  always 32 bytes (`wallet/wallet.go:3690-3696`), so `dumpprivkey` output is
  always the 39-byte form. A Rust `NewWIF` that takes a `[u8; 32]` loses
  nothing the daemon uses.
- **Q-28. `hdkeychain.GenerateSeed` takes a `uint8` length**, so a caller's
  wider integer is truncated by the conversion before the 16..64 check
  (`extendedkey.go:623-626`); callers in dcrwallet pass constants or
  validated values (not traced per call site here).
- **Q-29. `dcrutil.NewAmount`'s error is not a sentinel.** Each call builds
  a fresh `errors.New("invalid coin amount")` (`amount.go:85`); Go callers
  cannot match it with `errors.Is`, only by text.
- **Q-30. `dcroxide-certgen` has P-384 in one module and not the other.**
  `certgen::Curve` has two variants, while the public `gentool` module (the
  port of dcrd's `cmd/gencerts`) generates, signs with and PKCS#8-marshals
  P-384 keys (`dcroxide:crates/dcroxide-certgen/src/gentool.rs:59-70`,
  `:184-213`). Section 3.22.
- **Q-31. `ecdsa.Sign` accepts a hash of any length.** The nonce function
  truncates the hash to its first 32 bytes and left-pads a shorter one with
  zeros (`dcrec/secp256k1/v4@v4.4.1/nonce.go:135-141`), and the signed
  scalar is `ModNScalar.SetByteSlice(hash)`, which applies the same
  truncation and padding before reducing modulo N
  (`ecdsa/signature.go:641`, `modnscalar.go:360-368`). The two agree, so
  `Sign(key, h)` equals `Sign(key, n(h))` where `n` keeps the first 32 bytes
  of a longer input and left-pads a shorter one to 32. dcrwallet reaches
  this through gRPC `SignHashes`, which forwards `req.Hashes` unvalidated
  (`internal/rpc/rpcserver/server.go:3763`; an empty hash signs the zero
  scalar). `dcroxide_dcrec::secp256k1::ecdsa::sign` takes `&[u8; 32]`
  (`ecdsa.rs:619`), so the wallet applies `n` first. `SignCompact` and
  `RecoverCompact` are only ever given a 32-byte `chainhash.HashB` result
  (`wallet/wallet.go:2030`, `:2044`).
- **Q-32. Three websocket replies bypass `MarshalResponse`.** They marshal a
  `dcrjson.Response` struct whose `Jsonrpc` field was never set, so the
  reply begins `{"jsonrpc":"","result":` — an empty version string where
  every other reply says `1.0` or `2.0`
  (`internal/rpc/jsonrpc/methods.go:284-306`, `server.go:378-380`,
  `:408-410`, `:429-431`: the invalid-request reply to an authenticated
  client, the `authenticate` acknowledgement and the `stop` reply).
  `dcroxide_dcrjson::marshal_response` cannot produce it (section 3.19).
- **Q-33. `mixing.F` is a `*big.Int` that dcrwallet hands to the external
  solver** (`testsolver.go:29`, `solverrpc.Roots(coeffs, mixing.F)`); in
  dcroxide it is the `u128` constant `F = 2^127 - 1`
  (`dcroxide:crates/dcroxide-mixing/src/field.rs:17`). Whatever encodes the
  solver request converts it (spec 12/13 own the solver wire format).
- **Q-34. dcroxide's `Vote`, `Choice` and `ConsensusDeployment` hold
  `&'static str`** where Go holds `string` (section 3.3). Reading is
  unaffected; a value built from run-time strings (a fixture read from a
  file, for instance) needs a wallet-side owned mirror.
- **Q-35. Go transactions alias their inputs and outputs.** `MsgTx.TxIn`
  and `TxOut` are slices of pointers, so a `*wire.TxIn` appended to two
  transactions, or kept by the caller after `AddTxIn`, stays linked to both;
  the port owns `Vec<TxIn>` and `Vec<TxOut>` (section 3.1). `MsgTx.Copy` is
  not called by dcrwallet's non-test code (method inventory). Whether any
  call site depends on the aliasing was not traced (open question O-13).

## 11. Upstream tests

Tests that pin the packages this document maps, and what to do with each.

| Package | Test files (module cache) | Pins | Action |
|---|---|---|---|
| `hdkeychain/v3@v3.1.3` | `extendedkey_test.go` (1,051 lines: `TestBIP0032Vectors`, `TestPrivateDerivation`, `TestPublicDerivation`, `TestGenenerateSeed`, `TestExtendedKeyAPI`, `TestErrors`, `TestZero`, `TestBIP0032Vector4`), `example_test.go`, `bench_test.go` | by their names: the BIP32 vectors under Decred version bytes, private and public derivation chains, seed generation bounds, the accessor API, error returns and `Zero`; the bodies were not read | port all vectors to the new crate; add an oracle command for `NewMaster`/`Child`/`ChildBIP32Std`/`Neuter`/`String`/`NewKeyFromString` and a differential over random seeds biased toward keys with leading zero bytes |
| `dcrutil/v4@v4.0.3` | `amount_test.go` (`TestAmountCreation`, `TestAmountUnitConversions`, `TestAmountMulF64`, `TestAmountSorter`), `wif_test.go` (`TestWIF`), `appdata_test.go` (`TestAppDataDir`), `txsort/txsort_test.go` (`TestSort`) with `txsort/testdata` | by their names: amount creation and unit conversion tables, WIF round trips, directory rules per operating system, sort vectors; the bodies were not read | port; differential for `NewAmount` and `String` over random and boundary values |
| `dcr-rs` | `tests/vectors.rs` (933 lines) | dcrd-generated HD vectors including the master-with-leading-zero and mixed-variant cases (`dcr-rs:src/hd.rs:50-56`) | reuse as additional vectors; where dcr-rs diverges from dcrd on purpose (section 12.2) the expected values differ and must come from the oracle |
| `mixing@v0.7.5` | `signatures_test.go`, `sid_test.go`, `mixpool/mixpool_test.go`, `mixpool/orphans_test.go`, `mixclient/client_test.go` (475 lines), `utxoproof/utxoproof_bench_test.go` | signatures, session ids, pool acceptance, an in-process multi-peer mix | the first four are covered by dcroxide's scripted vectors; `client_test.go` is the template for the mix client's integration test (spec 12) |
| dcrwallet | `wallet/reorg_test.go` (the only file that imports `chaingen`); `wallet/network_test.go` supplies the `mockNetwork` backend and does not use `chaingen` | `reorg_test.go` uses `chaingen` to build side chains | regenerate the chains with a Go tool and store them as vectors, as dcroxide does for dcrd's generators (`dcroxide:docs/PARITY.md:1026`) |
| dcrwallet | `wallet/udb/*_test.go`, `wallet/udb/testdata/*.db.go` | old database versions built with historical dcrd modules (`github.com/decred/dcrutil/hdkeychain`, `github.com/decred/dcrd/blockchain`, …) | the `.db.go` generators are separate programs against old module versions; use their checked-in `.db.gz` outputs, do not port the generators |

Test files of the same modules not named in the table, listed so that none
is silently skipped:

| Module | Test files | Relevance |
|---|---|---|
| `dcrutil/v4@v4.0.3` | `block_test.go`, `tx_test.go`, `util_test.go`, `example_test.go` | cover `Block`, `Tx`, `VerifyMessage` and the amount examples; the first three test surface the wallet does not need (section 3.4), so they are not ported |
| `hdkeychain/v3@v3.1.3` | `example_test.go`, `bench_test.go` | usage examples and benchmarks; no additional vectors |
| `mixing@v0.7.5` | `mixclient/log_test.go` | test logger plumbing (`mixpool.UseLogger`), no vectors |
| `sntrup4591761` (pinned pseudo-version) | `sntrup4591761_test.go`, `testdata/sage128.gz` | known-answer vectors for the KEM; the primary source for the new crate's vectors (bodies not read) |
| `dcrec/secp256k1/v4@v4.4.1` | `ecdh_test.go`, `privkey_test.go`, `privkey_bench_test.go` | `GenerateSharedSecret`, `PrivKeyFromBytes`, key generation; port the vectors with the additions of section 12.3 (bodies not read) |
| `dcrec/edwards/v2@v2.0.4` | `privkey_test.go` | `PrivKeyFromScalar` among others (body not read) |
| `go-socks@v1.1.0` | none | the pool and `ProxiedAddr` behaviour of section 9 items 20-21 has no upstream test; write one against a local SOCKS5 stub |
| `vspd/client/v4@v4.0.2` | not listed here | spec 11 |

Upstream tests that pin the additive items of section 12.3 (test function
names from the module cache; bodies read only where a line is cited):

| Module | Test file | Pins | Action |
|---|---|---|---|
| `wire@v1.8.0` | `msgtx_test.go` (`TestTxSerialize`, `:348`; calls `PkScriptLocs` at `:421`) | `PkScriptLocs` offsets for its transaction table | port the table with `pk_script_locs` |
| `wire@v1.8.0` | `msgversion_test.go` (`TestVersion`, `:22`; `AddUserAgent` at `:73-83`) | user agent composition with and without comments | port with `add_user_agent` |
| `wire@v1.8.0` | `common_test.go` (`TestVarStringWire`, `TestVarStringWireErrors`, `TestVarStringOverflowErrors`, `:560-690`) | `WriteVarString` and its reader | port the write half |
| `dcrec/secp256k1/v4@v4.4.1` | `ecdsa/signature_test.go` (`TestSignatureSerialize`, `TestSignAndVerify`, `TestSignAndVerifyRandom`, `TestSignFailures`, `TestSignAndRecoverCompact`, `TestRecoverCompactErrors`, `TestSignAndRecoverCompactRandom`), `ecdsa/error_test.go` | DER serialization, signing, compact signatures and their error kinds; whether any case signs a hash that is not 32 bytes was not checked | oracle differential for `Sign` over hash lengths 0, 1, 31, 33 and 64 (quirk Q-31) |
| `dcrjson/v4@v4.2.0` | `jsonrpc_test.go`, `cmdparse_test.go`, `cmdinfo_test.go`, `help_test.go`, `parse_test.go`, `register_test.go`, `error_test.go`, `helpers_test.go`, `example_test.go` | envelope marshalling, parameter parsing, usage and help generation | `dcroxide-dcrjson` carries its own tests for these (not compared file by file here); the `json.Unmarshal`-into-`Request` behaviour is Go standard-library behaviour with no dcrjson test, and dcroxide tests it inside `dcroxide-rpc` (`dcroxide:crates/dcroxide-rpc/src/http.rs:686-732`) |
| `connmgr/v3@v3.1.4` | `dynamicbanscore_test.go` (`TestDynamicBanScoreDecay`, `TestDynamicBanScoreLifetime`, `TestDynamicBanScoreReset`, `TestDynamicBanScoreStringer`) | ban score decay; the module has no test file for `TorLookupIP` | dcroxide has vector tests for the ban score (`dcroxide:crates/dcroxide-connmgr/tests/banscore_vectors.rs`) |
| `addrmgr/v3@v3.0.0` | `addrmanager_test.go`, `knownaddress_test.go`, `netaddress_test.go`, `network_test.go`, `error_test.go`, `log_test.go` | v3 behaviour; dcroxide's address manager tests are vector files generated from addrmgr v4 (`dcroxide:crates/dcroxide-addrmgr/tests/`) | the v3 tests are not ported; see hazard 8 of section 12.6 |
| `crypto/rand@v1.0.1` | `bench_test.go` only (benchmarks for `Read`, `Int32N`, `Uint32N`, `Int64N`, `Uint64N`, `Duration`, `ShuffleSlice`) | nothing: the module has no unit test | the added globals get distribution tests of their own |
| `certgen@v1.2.0` | `certgen_test.go` (`TestNewTLSCertPair`, P-521 only) | one curve | P-384, P-256 and Ed25519 need differential tests against Go |

dcrwallet's own tests use the dcrd modules only through identifiers the
daemon also uses, with the exceptions in section 3.29. Untested upstream
and therefore needing new differential tests: `dcrutil.WIF` for the Ed25519
and unknown-scheme paths (UNVERIFIED whether `wif_test.go` covers them; the
body was not read), `Amount.String` beyond `MaxAmount`, `hdkeychain` keys
with stripped leading zeros through a string round trip (quirk Q-6), and
`MsgTx.PkScriptLocs` against `dcroxide-wire`'s serializer for transactions
with large varint counts.

dcroxide's own tests for the reused crates stay in dcroxide. The pattern to
copy for new code is its oracle rig: a Go program linking the exact module
versions, driven over line-delimited JSON by `dcroxide-testutil`
(`dcroxide:crates/dcroxide-testutil/src/lib.rs:4-11`,
`dcroxide:docs/adr/README.md`, ADR-0001). The wallet's oracle must link the
versions of `go.mod`, which differ from dcroxide's oracle for most modules
(section 4.1), so it is a second `go.mod`, not an extension of
`dcroxide:tools/oracle`.

Coverage notes: `secp256k1.GenerateSharedSecret` has a test file in the
secp256k1 module (`ecdh_test.go`, not read); `mixing/keyagreement.go` has no dedicated test
file in v0.7.5 (the directory listing shows none) and is exercised only
through `mixclient/client_test.go`.

## 12. Rust porting notes

### 12.1 Workspace policies a dependent must satisfy

These apply in full if dcr-purse crates become members of the dcroxide
workspace, and in part otherwise (noted per row).

| Policy | Where | Applies to a separate workspace? |
|---|---|---|
| Toolchain pinned to Rust 1.99.1 with `rustfmt` and `clippy`; `rust-version = "1.99.1"`; edition 2024; resolver 3 | `dcroxide:rust-toolchain.toml:17-19`, `dcroxide:Cargo.toml:2`, `:42-51` | yes for the MSRV: the crates declare it, so the wallet cannot build with an older compiler |
| `unsafe_code = "forbid"`, `missing_docs = "deny"` | `dcroxide:Cargo.toml:146-148` | no (lints are per workspace), unless adopted |
| `clippy::all` and `clippy::arithmetic_side_effects` at warn, and CI's `cargo clippy --workspace --all-targets -- -D warnings` | `dcroxide:Cargo.toml:150-152`, `dcroxide:.github/workflows/ci.yml:56` | no, unless adopted |
| `clippy.toml` forbids `rustls::ClientConfig::builder`, `ServerConfig::builder`, the verifier builders, `CryptoProvider::install_default` and every `ureq` entry point that builds a default agent; TLS must name `dcroxide_tlscrypto::default_provider()` | `dcroxide:clippy.toml:16-37` | no, but any binary that links both dcroxide-certgen/tlscrypto and its own rustls use should follow it to avoid two providers |
| Release profile: `panic = "abort"`, `overflow-checks = false` pinned on purpose ("so nobody turns it on as hardening": an overflowing operation where Go wraps would abort) | `dcroxide:Cargo.toml:57-94` | **profiles are taken from the root workspace of the build**, so a separate workspace must set the same two values itself; with overflow checks on in release, dcroxide code that relies on Go-width wrapping would panic |
| cargo-deny: licence allow-list (ISC, MIT, Apache-2.0, BSD-3-Clause, Unicode-3.0, CC0-1.0, CDLA-Permissive-2.0); `yanked = "deny"`; `multiple-versions = "deny"` with one skip (`syn:2`); `unknown-registry = "deny"`, `unknown-git = "deny"` | `dcroxide:deny.toml:4-21`, `:39`, `:100-106` | if adopted: a git dependency on dcroxide needs an `allow-git` entry; every new dependency must resolve to a single version across the graph |
| Pure Rust: bans on `ring`, `aws-lc-rs`, `aws-lc-sys`, `secp256k1-sys`, `cc` (wrappers `blake3`, `iana-time-zone-haiku` only), `cmake`, `pkg-config`, `vcpkg`, `bindgen`; `blake3` held to the `pure` feature; a CI job builds every target with no C compiler reachable | `dcroxide:deny.toml:77-98`, `dcroxide:.github/workflows/ci.yml:383-410` | if adopted: rules out the `secp256k1` crate (so dcr-rs as is), libsecp256k1, OpenSSL, `ring`-backed rustls, C-backed LMDB or SQLite, and a C sntrup implementation |
| `no_std` discipline: `base58`, `chaincfg`, `chainhash`, `crypto`, `dcrec`, `secp256k1-vartime`, `standalone`, `uint256`, `wire` are checked on `x86_64-unknown-none` with `--no-default-features` | `dcroxide:.github/workflows/ci.yml:345-381` | changes to those nine crates must keep them `no_std`; additions that need `std` go behind a feature, as `dcroxide-crypto`'s `rand` does |
| `cargo doc` with `-D warnings`; `typos` | `dcroxide:.github/workflows/ci.yml:321-343` | if adopted |
| Upstream tracking: the pin follows dcrd master until dcrd tags v2.2.0, then release tags; a pin move reviews every crate a dcrd delta touches | `dcroxide:docs/adr/README.md`, ADR-0013 | yes: a dcroxide pin move can change behaviour under the wallet; the wallet pins dcroxide by revision and takes pin moves deliberately |
| Deliberate divergences from dcrd are recorded in a table, not left implicit | `dcroxide:docs/PARITY.md:1047-1127` | the wallet inherits each one in the crates it links; three touch it visibly (seeder list aside): the multisig error relabel, the amount conversion, the mixpool rules. Four more sit in linked crates without an outside-visible effect: the `Prng` rekey schedule, entropy failure after startup, range reduction in the connmgr/addrmgr generators, and the two Tor resolvers (quirks Q-25, Q-26; `dcroxide:docs/PARITY.md:1108-1112`, `:1123`) |

CI gates a member crate passes (`dcroxide:.github/workflows/ci.yml`, job
names as written there):

| Job | What it runs | Consequence for wallet crates in the workspace |
|---|---|---|
| `lint` | `cargo fmt --all --check`; `cargo clippy --workspace --all-targets -- -D warnings`; `cargo metadata --locked` on `fuzz/` | formatting and warning-free clippy, tests and benches included |
| `lint-latest` | clippy on latest stable, advisory | early notice of new lints |
| `test` | `cargo test --workspace` on an operating-system matrix | wallet tests run on every listed OS |
| `test-wrapping` | the suites again with overflow checks off, as release ships | code must be correct both with and without overflow checks |
| `test-vartime-release` | release and `soak-asserts` runs of `dcroxide-secp256k1-vartime` and the vartime branches of `dcroxide-dcrec` | applies to additions in `dcroxide-dcrec` (section 12.3) |
| `tls-soft-backends` | `dcroxide-tlscrypto` tests; asserts `ring` is absent from the tree | any wallet TLS dependency that pulls `ring` fails it |
| `dcrd-interop` | builds dcrd at the parity commit and runs live-socket tests | the model for a dcrwallet/dcrd interop job |
| `go-tools` | `go vet` and `go test` in each module under `tools/` | a wallet oracle under `tools/` is vetted and tested here |
| `deny` | cargo-deny: licences, advisories, sources, bans | section 12.1 rows above |
| `docs`, `typos` | `cargo doc --workspace --no-deps` with warnings denied; `typos` | every public item documented (`missing_docs = "deny"`) |
| `no-std`, `no-c-compiler` | rows above | — |
| `fuzz-smoke`, `nightly-fuzz`, `nightly-key-sweep`, `nightly-sigsoak` | fuzz targets from the separate `fuzz/` workspace; nightly key and signature soaks through `dcroxide-sigdiff` | new parsers (extended keys, WIF) and signing paths are expected to get targets and soak classes |

`deny.toml` sets `[graph] all-features = true` (`dcroxide:deny.toml:1-2`),
so the licence and ban checks see every optional dependency of every member
crate, including features a wallet build would not enable.

dcroxide's crates are all version 0.1.0, unpublished, and depend on each
other by path (`dcroxide:Cargo.toml:42-51`, each crate's `Cargo.toml`).
`dcroxide-secp256k1-vartime`, `dcroxide-sigdiff` and `dcroxide-testutil` set
`publish = false`.
A crates.io dependency is therefore not available today.

### 12.2 dcr-rs

What it offers that the wallet lacks from dcroxide: HD keys (`src/hd.rs`),
an amount formatter (`src/amount.rs`), P2PKH/P2SH address helpers, a
`SigHashAll` P2PKH signer. Reasons it cannot be linked as is:

1. It depends on `secp256k1 = "0.29"` (`dcr-rs:Cargo.toml`, `[dependencies]`),
   the binding to the C library, which dcroxide's deny rules ban by name
   (`secp256k1-sys`) and through `cc`.
2. `hd.rs` diverges from dcrd on purpose: it rejects a zero private sum and
   the point at infinity (`hd.rs:118-124`), fails at depth 256 instead of
   wrapping (`hd.rs:138-146`), decides the key kind from the version prefix
   and detects the network instead of taking it (`hd.rs:472-485`), and
   reports one `Error::Derivation` where dcrd distinguishes four errors.
3. It has its own `MsgTx`, `Address` and `Network` types, parallel to
   dcroxide's.
4. dcroxide already decided that any import from dcr-rs beyond BLAKE-256
   needs its own decision record
   (`dcroxide:docs/adr/0002-vendor-blake256-from-dcr-rs.md:118-125`).

Use: read `hd.rs` as a second description of the stripping rule, reuse
`tests/vectors.rs`, and write the hdkeychain crate against dcrd's source on
`k256` (already in dcroxide's graph) or on a small arithmetic API added to
`dcroxide-dcrec`.

### 12.3 How the wallet should depend on dcroxide

Three workable forms, with what each costs:

| Form | Mechanics | Cost |
|---|---|---|
| A. One workspace | dcr-purse crates become members of the dcroxide workspace (or both move into a new shared workspace) | one lock file, one deny/clippy/toolchain policy, atomic changes across wallet and library crates; the wallet inherits every node-driven policy and CI job, and dcroxide's CI grows by the wallet's |
| B. Separate workspace, git dependency pinned by `rev` | `dcroxide-wire = { git = "…", rev = "…" }` for each crate; Cargo resolves the path dependencies inside the repository | independent release cadence; the wallet must copy the release profile (section 12.1), keep a duplicate-free graph on its own, and every additive change to a dcroxide crate is a two-repository change |
| C. Fork | copy the crates | loses dcroxide's pin moves and fixes; only justified if a required change is refused upstream |

Extraction of the 17 library crates into a third repository shared by node
and wallet is form A with an extra move; it helps only if both projects want
independent CI. Recommendation: form A or B decided by project ownership
(decision D-1); in either, the changes below go into dcroxide crates as
additive, oracle-tested items, not into wallet-side copies.

Additive changes needed in existing dcroxide crates:

| Crate | Addition | Size |
|---|---|---|
| `dcroxide-wire` | `MsgTx::pk_script_locs`, public `write_var_string`, `MAX_BLOCKS_PER_MSG`, `TX_VERSION_SEQ_LOCK`/`TX_VERSION_TREASURY`, command-name constants, `MsgVersion::add_user_agent` | S |
| `dcroxide-dcrec` | `PrivateKey::serialize`; a reducing `from_bytes_mod_n` (dcrd's `PrivKeyFromBytes`); `generate_private_key_from_rand`; `generate_shared_secret`; scalar-add and point-add entry points for hdkeychain (or none, if hdkeychain uses `k256` directly); `edwards` key from scalar; numeric `SignatureType` conversion | M (constant-time review for the secret paths; new differential classes) |
| `dcroxide-crypto` | `rand::{int32n, int64n, uint64n, big_int}` globals and a `Read`/`RngCore` adapter | S |
| `dcroxide-dcrjson` | move `unmarshal_request` and `RawRequest` down from `dcroxide-rpc` (`http.rs:134-151`, `:302-434`), or re-export them, so a wallet server decodes request envelopes without linking the node's RPC crate; a marshaller for a bare `Response` that writes the `jsonrpc` string as given, the empty string included (section 3.19, quirk Q-32) | S |
| `dcroxide-certgen` | `Curve::P384` in `certgen` (key generation and signing for P-384 already exist in `gentool`); a template parameter on `gentool::generate_authority` / `create_issued_cert` for dcrwallet's subjects, key usage and empty SAN list (section 3.22) | S for P-384, S–M for the client-CA shapes |
| `dcroxide-mixing` | `keyagreement` (with the KEM crate below); a way to construct the pool without the three node-policy divergences if D-5 says so | M |
| `dcroxide-stake` | put `stakedb`, `ticketdb`, `ticketnode`, `tickettreap` behind a default-on feature so a wallet build can drop `dcroxide-database` and `redb` | S |
| `dcroxide-standalone` | none required; optionally an internally locked `SubsidyCache` wrapper | — |

### 12.4 New crates

| Crate (working name) | Mirrors | Contents | Depends on | Size |
|---|---|---|---|---|
| `hdkeychain` | `hdkeychain/v3` v3.1.3 | sections 4.2, 5.1–5.4; a `NetworkParams` trait implemented for `dcroxide_chaincfg::Params`; zeroizing key storage that keeps the stored-length (stripped) form | `dcroxide-base58`, `dcroxide-crypto`, `dcroxide-dcrec` or `k256`, `hmac`, `sha2`, `zeroize` | S–M: about 640 lines of Go, but every wallet address depends on it; needs oracle differential and a leading-zero-biased soak |
| `dcrutil` | `dcrutil/v4` v4.0.3 | `Amount` (sections 4.4), `WIF` (4.3, 5.5), `app_data_dir` (5.6), `BLOCK_VALID`, `is_flag_set16`, `txsort` (5.7) | `dcroxide-base58`, `dcroxide-chainhash`, `dcroxide-dcrec`, `dcroxide-dcrjson` (float formatting) | S |
| `sntrup4591761` | `github.com/companyzero/sntrup4591761` at the pinned pseudo-version | key generation, encapsulation, decapsulation, byte-compatible including how randomness is consumed | none | M–L, the highest-risk item: no existing Rust crate was confirmed byte-compatible (open question O-3) |
| `mixclient` | `mixing/mixclient` v0.7.5 | spec 12 | `dcroxide-mixing`, `dcroxide-wire`, `dcroxide-txscript`, the solver interface | L (3,500 lines of Go, protocol state machine, blame) |
| `dcrd-rpc-client` | dcrwallet's `rpc/client/dcrd` over `jrick/wsrpc` (not dcrd's `rpcclient`) | spec 15 | `dcroxide-dcrjson`, `dcroxide-rpctypes`, a websocket client, `dcroxide-tlscrypto` | M |

### 12.5 Infrastructure inside `dcroxide-node` and `dcroxide-rpc`

The node already ports Go libraries that dcrwallet also uses, but as modules
of the node crate, tied to dcrd's option table, subsystem list and RPC
server type:

| Go library (dcrwallet's pin) | dcroxide location | Coupling |
|---|---|---|
| `jessevdk/go-flags` (v1.5.0) | `dcroxide:crates/dcroxide-node/src/flags.rs` (2,188 lines; ports v1.6.1) | parser is written over `crate::config::Config` and the dcrd option registry |
| `decred/slog` (v1.2.0) + `jrick/logrotate` (v1.0.0) | `logging.rs` (533), `logrotate.rs`, `logsubsys.rs` | subsystem ids are dcrd's; rotator ports logrotate v1.1.2 |
| `decred/go-socks` (v1.1.0) | `socks.rs` (959) | usable logic, node `Config` constructor; lacks `socks.Pool` (the `--circuitlimit` bound) and the `ProxiedAddr` remote address dcrwallet reads (section 3.28, quirk Q-23); the file names no release, but dcrd at the pin requires go-socks v1.1.0, dcrwallet's pin (section 4.1; open question O-8) |
| Go `net/http` server + `gorilla/websocket` (v1.5.1) | `dcroxide-node/src/rpcrun.rs`, `websocket.rs`, `wsframe.rs`; `dcroxide-rpc/src/http.rs`, `server.rs` | generic over the node's `RpcChain` and `Server` |
| Go `crypto/tls` behaviour | `rpctls.rs` (2,283), `dcroxide-tlscrypto` | listener policy in the node; provider reusable |
| `dcrutil.AppDataDir`, Go path and duration helpers | `config.rs`, `gostd.rs` (private module) | node-private |
| Windows service, signals, pipe IPC | `dcroxide-winsvc`, `ipc.rs`, `pipeserve.rs` | dcrwallet has its own pipe protocol (spec 16) |

Linking `dcroxide-node` to reach these would link the whole node. They are
reusable only after extraction into library crates with the dcrd-specific
tables passed in. That extraction is work inside dcroxide (decision D-2) and
is the largest reuse lever outside the dcrd-module crates: the specs for
configuration, logging and the JSON-RPC transport should each state which of
these modules they would take and what differs at dcrwallet's library
versions.

### 12.6 Hazards

1. **Profile inheritance.** Section 12.1: `overflow-checks` and `panic`
   come from the building workspace.
2. **Transitive weight.** `dcroxide-gcs → dcroxide-stake → dcroxide-database
   → redb`; `dcroxide-connmgr → dcroxide-addrmgr + dcroxide-ratelimit +
   dcroxide-dcrjson`; `dcroxide-certgen → dcroxide-tlscrypto → rustls +
   RustCrypto + ml-dsa`; `dcroxide-addrmgr → serde + serde_json + sha3`.
3. **Owned `Params`.** Each `*_params()` call allocates the full parameter
   set including the 3,146-entry mainnet block-one ledger behind an `Arc`
   (`dcroxide:crates/dcroxide-chaincfg/src/lib.rs:215-219`). Build once.
4. **Enum `Address`.** Go code that type-switches on concrete address types
   or asserts optional interfaces maps to matches and `Option`s; the mapping
   is mechanical but touches every address call site.
5. **Slice decoding.** dcroxide-wire has no streaming reader. The wallet's
   P2P reader must buffer a frame (bounded by `read_message_header`) before
   decoding; database values decode with `from_bytes` and must tolerate
   trailing bytes exactly where Go's `Deserialize` does.
6. **Key hygiene.** `PrivateKey` wipes on drop, but `sign` takes `&[u8]`
   keys from the `KeyDb` closure as `Vec<u8>`; the wallet's closure must
   return zeroizing buffers or wipe them itself. Go's wallet zeroes keys
   explicitly in the same places.
7. **Error text.** Section 8, the `ecdsa` and `sign` rows.
8. **Two generations.** `dcroxide-addrmgr` is addrmgr v4. Address selection
   is randomized, so no vector distinguishes v3 from v4 selection, but the
   bucket statistics and filters are new code paths the Go wallet does not
   run.
9. **Pin drift.** dcroxide will follow dcrd to v2.2.0 and beyond; dcrwallet's
   pin moves separately. Each dcroxide revision bump in the wallet repeats
   the table of section 4.1.
10. **Mixed TLS providers.** A gRPC stack that brings its own rustls
    configuration must be pointed at `dcroxide-tlscrypto` or the binary
    carries two crypto stacks and violates the no-C rule if the default
    provider is `aws-lc-rs` or `ring`.
11. **RustCrypto generation lock-in.** `multiple-versions = "deny"` plus
    the versions in section 3.30 mean the wallet's scrypt, Argon2id,
    BLAKE2b, XSalsa20-Poly1305 and XChaCha20-Poly1305 crates must build
    against `sha2` 0.11, `hmac` 0.13, `chacha20` 0.10 and `poly1305` 0.9, or
    each mismatch needs a recorded skip.
12. **Seeder and SOCKS look reusable and are not drop-in.** Sections 3.15
    and 3.28: same protocol family, different observable request and
    address handling.

### 12.7 Size of this subsystem

The reuse layer itself — adapters, the additive changes of section 12.3 and
the two small new crates (`hdkeychain`, `dcrutil`) — is **M**: little new
logic, but each addition lands in a differential-tested crate under its
rules, and hdkeychain needs a soak of its own. `sntrup4591761`, `mixclient`
and the RPC client are sized in their own specs and are not part of this
estimate.

## 13. Open questions

- **O-1.** Does `dcrd/mixing` at `46e9df48` differ from the v0.7.5 tag
  (`46fc30ad`)? Settle with a diff of `mixing/` between the two commits in a
  dcrd clone that has both. dcroxide-mixing's vectors were generated at
  `382ff4dd`.
- **O-2.** Do `rpc/jsonrpc/types` (v5 in-tree), `dcrjson`, `certgen` and
  `container/lru` at `46e9df48` differ from the releases dcrwallet pins?
  Settle with the same kind of diff. For the types module the field tables in
  `dcroxide-rpctypes` must be compared with `types/v4@v4.4.0` for the 19
  commands and the result and parameter types of section 3.20.
- **O-3.** Is there a Rust sntrup4591761 implementation byte-compatible with
  `companyzero/sntrup4591761` in key generation and encapsulation from a
  given byte stream? If not, it is a port from the Go source (spec 12).
- **O-4.** Does any dcrwallet call site call `Zero()` on a private extended
  key while a key obtained from its `Neuter()` is still in use (quirk Q-9)?
  Settle by tracing each `Neuter()` result in `wallet/udb/addressmanager.go`
  and `wallet/` against the `Zero()` calls on its source.
- **O-5.** Are the strings in `dcroxide_txscript::sign::SignError` equal to
  the Go sign package's error strings for every path dcrwallet can reach?
  Settle by an oracle differential on failing inputs.
- **O-6.** Exact text of `ErrWrongWIFNetwork` (derived as
  `WIF is not for the network identified by 0x22de` on mainnet) and the
  public key bytes `edwards.PrivKeyFromScalar` yields for a given 32-byte
  scalar. Settle through the oracle.
- **O-7.** Which Go release is the reference for behaviour dcr-purse takes
  from Go's standard library (decision D-6)?
- **O-8.** Does the handshake of `dcroxide-node/src/socks.rs` equal
  go-socks v1.1.0's in the bytes sent to the proxy? The release question is
  settled: dcrd at `46e9df48` requires `go-socks v1.1.0`
  (`dcrd@v1.2.1-0.20261106175134-46e9df48109a/go.mod:37`), the release
  dcrwallet pins. The port's byte sequence was not compared with
  `go-socks@v1.1.0/socks/dial.go` here; settle by reading the two side by
  side or with a recording SOCKS5 stub.
- **O-9.** Are the four `cmd/` tools (`movefunds`, `repaircfilters`,
  `sweepaccount`, `treasurykey`) part of the drop-in deliverable? If yes,
  `secp256k1.GeneratePrivateKey` and a `wsrpc`-style client are needed
  outside the daemon too (section 3.29).
- **O-10.** Do `wif_test.go` and `amount_test.go` cover the Ed25519,
  unknown-scheme and out-of-range paths this document specifies from the
  source (sections 4.4, 5.5)? Settle by reading the two test bodies; if
  not, the oracle differential is the only pin.
- **O-11.** Can `dcroxide_certgen::gentool` take a caller-supplied subject
  and key-usage template without changing the bytes it emits for dcrd's
  `gencerts` (section 3.22)? Settle in dcroxide with its existing gencerts
  vectors.
- **O-12.** Does `dcroxide_rpc::http::unmarshal_request` reproduce, for the
  Go release dcr-purse takes as its reference (decision D-6), the error
  texts dcrwallet's server would log or return for a malformed envelope?
  dcrwallet does not send the decode error to the client (it answers
  `ErrRPCInvalidRequest` or disconnects, `internal/rpc/jsonrpc/server.go:370-380`,
  `:558-560`), so only the accept/reject decision and the fields recovered
  from a partially decoded request (`req.ID`, `req.Jsonrpc`) are
  observable; whether the port's partial-decode state equals Go's on error
  was not checked. Settle with an oracle differential on malformed bodies.
- **O-13.** Does any dcrwallet call site rely on two `MsgTx` values (or a
  `MsgTx` and a retained `*TxIn`/`*TxOut`) sharing an element, so that a
  later write through one is seen through the other (quirk Q-35)? Settle by
  tracing every `AddTxIn`/`AddTxOut` argument and every `*msgTx` struct copy
  in `wallet/createtx.go`, `wallet/txauthor` and `wallet/wallet.go`.
- **O-14.** dcroxide's `docs/PARITY.md` list of known remaining gaps
  (`dcroxide:docs/PARITY.md:1129` onward, about 3,400 lines) was searched by
  keyword only (`wallet`, `hdkeychain`, `dcrutil`, `WIF`, `PkScriptLocs`,
  `AddUserAgent`, `WriteVarString`, `PrivKeyFromBytes`, `mixclient`,
  `sntrup`, `P-384`, `rpcclient`, `Int32N`); the hits concern the node's TLS
  and RPC server as seen by dcrwallet as a client, none a library item the
  wallet links. A full read may still surface a divergence in a linked
  crate.

Decisions for the project plan:

- **D-1.** Dependency form: one workspace with dcroxide, or a separate
  workspace pinned to a dcroxide revision (section 12.3).
- **D-2.** Whether dcroxide extracts its go-flags, slog/logrotate, SOCKS5,
  HTTP/websocket and TLS-listener ports into library crates for the wallet,
  or the wallet re-implements them (section 12.5).
- **D-3.** Where `DynamicBanScore` and the Tor resolver's socket wrapper
  live so the wallet does not link the connection manager and the node.
- **D-4.** Seeder list: reproduce `chaincfg/v3@v3.3.0`'s three mainnet
  seeders or take dcroxide's four (quirk Q-2).
- **D-5.** Wallet mixpool: dcrd v0.7.5 rules exactly, or dcroxide's three
  stricter node rules.
- **D-6.** Reference Go release and JSON error-text generation (section 4.1).
- **D-7.** `PrivKeyFromBytes`: add the reducing constructor (bug-for-bug) or
  reject out-of-range imported keys (quirk Q-5).
- **D-8.** Whether dcr-purse adopts dcroxide's pure-Rust rule, which decides
  the choices for the KEM, the database engine, the gRPC stack's TLS and
  whether any C-backed crate is admissible.
- **D-9.** SOCKS: extend dcroxide's port with the pool and the proxied
  remote address, or write the wallet's own dialer (quirk Q-23).
- **D-10.** HTTPS seeding: reuse `dcroxide_addrmgr::seed` pieces behind
  dcrwallet's request shape, or write it in the wallet's P2P crate
  (section 3.15).

## 14. Summary table

Coverage is of what dcrwallet (and the mixing and VSP client code it links)
uses, not of the whole Go module. Work is the size of closing the gap for
the wallet: S under a week, M weeks, L months, for one engineer.

| dcrd module (dcrwallet pin) | dcroxide crate | Coverage | Gap list | Work |
|---|---|---|---|---|
| `wire` v1.8.0 | `dcroxide-wire` | full | `pk_script_locs`, public `write_var_string`, `MAX_BLOCKS_PER_MSG`, `TX_VERSION_TREASURY`, command constants, `add_user_agent` | S |
| `chaincfg/chainhash` v1.0.5 | `dcroxide-chainhash` | full | none | — |
| `chaincfg/v3` v3.3.0 | `dcroxide-chaincfg` | full | hdkeychain version-bytes trait (in the new crate); one extra mainnet seeder | S |
| `dcrutil/v4` v4.0.3 (+ `txsort`) | none (`hash160` in txscript; helpers inside the node and rpc crates) | none | `Amount`, `NewAmount`, WIF, `AppDataDir`, `BlockValid`, `IsFlagSet16`, `txsort` | S (new crate) |
| `txscript/v4` v4.1.2 | `dcroxide-txscript` | full | none | — |
| `txscript/v4/stdaddr` | `dcroxide_txscript::stdaddr` | full | none (enum instead of interfaces) | — |
| `txscript/v4/stdscript` | `dcroxide_txscript::stdscript` | full | none; one relabelled error | — |
| `txscript/v4/sign` | `dcroxide_txscript::sign` | full | out-of-range secp256k1 keys rejected rather than reduced; error strings unverified | S |
| `hdkeychain/v3` v3.1.3 | none | none | whole package | S–M (new crate) |
| `blockchain/stake/v5` v5.0.3 | `dcroxide-stake` | full | pulls `dcroxide-database`/`redb` transitively; feature gate wanted | S |
| `blockchain/standalone/v2` v2.3.0 | `dcroxide-standalone` | full | `SubsidyCache` needs external locking | — |
| `blockchain/v5` v5.1.1 | `dcroxide-blockchain` (not needed) | n/a | test-only import of `chaingen`; replace with stored vectors | S (test tooling) |
| `gcs/v4` v4.1.1 (+ `blockcf2`) | `dcroxide-gcs` | full | none | — |
| `mixing` v0.7.5 (root package) | `dcroxide-mixing` | partial | `keyagreement.go` (ECDH + sntrup4591761), `DecapsulateError` | M |
| `mixing/mixpool` | `dcroxide_mixing::Pool` | partial | blocking `Receive`, internal locking and timers; three stricter rules to reconcile; pass no UTXO fetcher (quirk Q-21) | M |
| `mixing/utxoproof` | `dcroxide_mixing` (`utxoproof`) | full | none | — |
| `mixing/mixclient` | none | none | whole package, plus `sntrup4591761`, the solver interface and `txsort` | L |
| `addrmgr/v3` v3.0.0 | `dcroxide-addrmgr` (v4) | full, different generation | lifecycle (load, timed save, final save) and locking in the wallet; filter argument on `get_address`; timestamp argument on `new_net_address_from_ip_port`; its HTTPS seeder is dcrd's, not dcrwallet's | S |
| `connmgr/v3` v3.1.4 | `dcroxide-connmgr` (`DynamicBanScore`), `dcroxide-addrmgr` (`tor_lookup_ip`) | partial (all the wallet uses) | wide dependency for one 110-line file; Tor resolver socket wrapper is in the node crate | S |
| `dcrec` v1.0.1 | `dcroxide_txscript::sign::SignatureType` | partial | numeric conversion of the signature type | S |
| `dcrec/secp256k1/v4` v4.4.1 (+ `ecdsa`, `schnorr`) | `dcroxide-dcrec` | partial | `PrivateKey::serialize`, reducing constructor, key generation from a reader, ECDH, scalar and point arithmetic for HD derivation; `ecdsa` error text; `ecdsa::sign` takes exactly 32 hash bytes (wallet normalizes, quirk Q-31) | M |
| `dcrec/edwards/v2` v2.0.4 | `dcroxide_dcrec::edwards` | partial | `PrivKeyFromScalar` (for Ed25519 WIFs) | S |
| `crypto/blake256` v1.1.0 | `dcroxide_crypto::blake256` | full | none | — |
| `crypto/rand` v1.0.1 | `dcroxide_crypto::rand` | partial | `Int32N`, `Int64N`, `BigInt`, `Reader` | S |
| `crypto/ripemd160` v1.0.2 | `dcroxide_crypto::ripemd160` | full | none | — |
| `dcrjson/v4` v4.2.0 | `dcroxide-dcrjson` | full for commands, help and response marshalling; partial for the envelope | request decoder sits in `dcroxide-rpc`; no bare-`Response` marshaller (empty `jsonrpc`); wallet command and result types must be written as descriptors | S (descriptors counted in spec 16) |
| `rpc/jsonrpc/types/v4` v4.4.0 | `dcroxide-rpctypes` | full (field tables to be confirmed against v4.4.0) | none known | S (verification) |
| `rpcclient/v8` v8.1.0 | none | n/a | not linked by dcrwallet; the wallet's own dcrd client is new work | — here; M in spec 15 |
| `certgen` v1.2.0 | `dcroxide-certgen` | partial | P-384 in `certgen::Curve` (present in `gentool`); client-CA and client-certificate templates on `gentool` | S–M |
| `base58` v1.0.6 | `dcroxide-base58` | full | none | — |
| `container/lru` v1.0.0 | `dcroxide_containers::lru` | full | none | — |
| `slog` v1.2.0 | none as a crate (format in `dcroxide-node`) | none as a library | logger type, backend, levels, rotation: extract from the node or rewrite | S–M |
| `vspd/client/v4` v4.0.2, `vspd/types/v3` v3.0.0 | none | none | whole client (HTTP, JSON, Ed25519 response signatures) | spec 11 |
| `go-socks` v1.1.0 (not a dcrd module; listed because dcroxide ports it) | none as a crate (`dcroxide-node/src/socks.rs`) | partial, not a library | extraction from the node; `Pool` with a connection limit; `ProxiedAddr` remote address | S–M |
| `sntrup4591761` (indirect, through `mixing`) | none | none | whole KEM, byte-compatible with the Go stream consumption | M–L (section 12.4) |

