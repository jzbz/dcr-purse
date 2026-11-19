# ADR-0001 — The logical wallet export, format `purse-export 2`

- **Status:** **Accepted** as proposed (project owner, 2026-11-19), with the five
  questions put to the owner answered as recommended; see "Decision by the
  project owner" before "Discrepancies found".
- **Date:** 2026-11-19 (format 1 drafted; revised the same day to format 2
  after an adversarial review and the owner's decisions of 2026-11-18 and
  2026-11-19, ADR-0013 above all)

## Context

dcr-purse keeps its own store (engine `redb` 5, accepted 2026-11-18,
ADR-0003) and never reads, opens, writes or imports a dcrwallet `wallet.db`;
a user moves by restoring the seed (brief §1, §3 W2 as amended; ADR-0013,
Accepted). The two wallets therefore never share bytes on disk, and nothing
in the project can compare a Go wallet with a Rust wallet by comparing
files. What the plan compares instead is a canonical, format-independent
text form of a wallet that both sides write from their own storage, once
both have reached the same state by the same means:

- **operation-script differentials**: the same script of `udb` calls run
  through Go and Rust leaves equal exports (ADR-0002; brief §7 Layer 2,
  §6 Phase 3 exit);
- **wallet creation**: a Go wallet and a Rust wallet created from one seed
  and one set of passphrases have equal exports (brief §6 Phase 3 exit);
- **restore differentials**: Go and Rust each restore one seed with the
  same discovery options against the same chain, and their exports after
  the rescan are compared, which is how the W2 restore contract's state is
  checked (brief §3 W2, §7 Layer 6; ADR-0013 Open point 6);
- **scripted-history replays and prepared states**: a recorded simnet
  history replayed through both wallets (brief §6 Phase 5 exit), and the
  prepared states of Phase 4 and of the Phase 7 golden captures, whose
  exports are compared before anything else is (ADR-0013 Open point 6);
- **paired-wallet soaks**: Go and Rust wallets on copies of one seed,
  compared periodically (brief §6 Phase 7 and 1.0 exits, §7 Layer 7, R8).

The brief sets the coverage target: "everything dcrwallet persists (specs
01–05)" (brief §5, `tools/oracle`; §6 Phase 3). That target still earns its
place without an import. A comparison can only show that dcr-purse's store
holds what dcrwallet's holds for the same history if the Go side of the
comparison sees every piece of state dcrwallet keeps: a counter, flag or
index the Go exporter skipped is one the comparison never tests. The form
needs masking rules for values derived from randomness, from the host and
from clocks, a rule for anything Go iterates in map order, and a fixed
format before code depends on it.

The Go source at the pin (dcrwallet `54b85b33`) is the specification of
what a schema-26 database holds: `wallet/udb/addressdb.go`, `txdb.go`,
`txmined.go`, `vsp.go`, `treasury.go`, `agendas.go`, `metadata.go` and the
bucket creations of `upgrades.go`. Specs 02, 03 and 05 map it; since
ADR-0013 they describe what dcrwallet persists, which dcr-purse's store
logic and this export must cover, and no longer an importer's input (brief
§3 W2). udb's row codecs are unexported (spec 23 §5.5.1), and several
stored fields have no read API at all (block-record times, the stored
credit account, the stake-invalidated buckets, the VSP host counter), so
no exporter built only on udb's public API can see everything a Go wallet
keeps.

## Options considered

Three designs were drawn up independently and judged against the brief and
the Go source.

**A. Closed-world normal form.** A raw cursor walk claims every entry of
the database with exactly one rule; a claimed entry becomes a field of a
record keyed by format-independent identities (hashes, outpoints, account
numbers, hash160s); secondary indexes are folded into the records they
point at, and an index entry that points at nothing becomes an orphan
record; rows that fail Go's own checks go to a compared `[bad]` section;
dead data goes to an uncompared residue digest; anything else fails the
export. Write-only fields are exported and tagged. Masking is done by the
comparator. Balances and similar answers come from the query API in a
separate derived section. Headers and filters may be listed or digested per
4096 heights.

**B. Observable surface (read API).** Every value of the main sections is
the answer of an exported `udb` read call, in the order that call returns;
a "latent" section adds the persistent facts no read returns; sealed
secrets sit in their own section; write-only fields are excluded with
evidence that nothing reads them; balances are taken at every minconf at
which the spendable amount can change.

**C. DWX (diffable, self-describing).** One fact per line, `name=value`
everywhere, each section carrying its own schema in `=type` lines (key count,
volatile and optional flags) and a trailer with a raw and a masked SHA-256,
so two exports compare section by section from the trailers alone and a
plain `diff -u` names the section, record and field of a difference.
Secrets are proven equal by verdicts computed with the test passphrases,
never by writing them or a digest of them. Write-only fields and redundant
indexes are left out.

Also rejected outright: comparing database dumps (the two stores share no
format, brief §3 W2), keyed JSON (number formatting and member order become
an encoder question in two languages, and diffs read worse), and mirroring
dcrwallet's buckets one to one as text (it bakes dcrwallet's layout into
dcr-purse's store, which then has to emulate indexes it does not need;
W2 requires no schema-26 layout in dcr-purse's store).

## Decision (accepted, 2026-11-19)

Design A is the backbone: it is the only one whose coverage is closed and
checked by the exporter itself. Every stored byte of the Go database is
either a field of a record or on the exhaustive list of section 9, and
anything else fails the export, so a Go-versus-Rust comparison cannot pass
because the Go side looked away. Design C supplies the lexical layer.
Design B supplies the way plaintext fields and API answers are obtained. In
detail:

| Element | Source | Reason |
| --- | --- | --- |
| Closed world: every entry claimed by one rule, else `[unknown]` and failure | A | the only property that catches a counter, flag or blob dropped by dcr-purse's store, or skipped by the exporter itself |
| Normal-form records keyed by logical identity; secondary indexes folded into their records; dangling entries as `[orphans]`; reader-check failures as `[bad]`; dead data as `[residue]`, outside the eof digests | A | a store that keeps different indexes still produces the same fields; an index that disagrees shows as a difference; dcrwallet's dead layout never has to be reproduced |
| Write-only fields exported and tagged (`^`) instead of dropped | A | whether dcr-purse's store keeps them is ADR-0003's (D2) to decide; dropping them in the format would decide it now |
| `[derived]`: balances, account properties, unspent tickets from the query API | A, B | balances are behaviour, not rows; both sides compute them through their own query code |
| Comparator-side masking; the exporter always writes full values | A, B, C | one masking implementation, applied to text |
| 4096-height chain digest segments with defined digest items | A | a deterministic function of full mode, convertible from it (a test does) |
| In-band `=section`/`=type` schema with key count and flags; `name=value` fields; per-section trailers with raw and masked digests; an eof line over the compared metas and compared section digests | C | self-describing, diffable, comparable from trailers; the Rust parser is generic over the schema |
| `[privcheck]`: verdicts only, no secret and no digest of one, one verdict per sealed private field | C (A's plaintext fingerprints rejected) | a fingerprint of an xpriv or a private key is still derived from the secret; a verdict proves the same equality |
| Key fields never optional; one record type per identity form (`tx` and `utx`, `agenda` and `ticketagenda`, ...); equal keys are an error | C | no optional sort keys, so no special cases in the order |
| Plaintext from udb's API: xpubs via `Manager.Decrypt(CKTPublic, ...)`, address strings, hash160s and imported public keys via `Manager.ForEachAccountAddress` | B (A derived chain addresses itself) | one less re-implementation in the exporter; Rust derives them independently, which makes the comparison a real check of `purse-hdkeychain` |
| Values exported as stored where no writer at the pin can store anything else (treasury vote 0, empty agenda choices) | new | A normalised them on a no-reader argument; exporting them is simpler and loses nothing |
| Block timestamps treated as chain data, separate from the wallet clock | new | they are part of the header and hash, not a clock reading of the wallet |
| Clock values the caller injects (`@`) kept apart from randomness and udb's own clock reads (`~`) | review | masking injected times would hide a wrong received time or birthday in every scripted comparison |
| A streaming producer and a streaming validator | review; performance is a project goal (ADR-0014, in preparation) | an export of a large wallet must not need memory in proportion to its text |

Rejected from B: main sections in API order (they inherit the quirks of
QK-0083, QK-0090, QK-0091, QK-0104, QK-0109 by construction, need a waiver
per recorded divergence, and cannot show completeness); `MakeInputSource`
listings and the minconf set M (quadratic on a large wallet; coin selection
and cursor order belong to the Phase 2 cursor-conformance test and the
Phase 3 query sweep). Rejected from C: excluding write-only fields and
redundant indexes (see above); `group` interleaving of record types (records
of one type stay together, which keeps the order rule one line long); the
`~` sigil on values (the schema says which fields are volatile); omitting
absent optional fields (fixed positions read better and diff better); the
transaction length and digest in place of the bytes (raw transactions are
exported; size is a soak-profile question); forward-compatible projection of
newer exports onto older schemas (there are no old exports to keep). Rejected
from A: deriving chain-address identities inside the exporter; the
`@acctpass` meta (the `[privcheck]` verdicts say which passphrases were
supplied); checking `@network` against the genesis hash of block 0 (upstream
fixtures written on testnet2 and upgraded with testnet3 parameters, as
upstream's own tests do, would fail it); the crosscheck mode is not built
(see "What remains open").

### 1. Principles

- **P1 Closed world.** The exporter claims every entry of the database it
  consumes, bucket entries included. A final cursor walk sends every
  unclaimed entry under a dead-item rule (section 9) to `[residue]` and any
  other unclaimed entry to `[unknown]`, which fails the export unless
  `allow_unknown` is set. A database whose `meta/ver` is not 26 is refused
  (`wallet/udb/metadata.go:28-37`, `wallet/udb/open.go:33-38`).
- **P2 Logical identity.** Records are keyed by facts that do not depend on
  dcrwallet's layout. Surrogate keys (VSP host IDs) are not exported, except
  as an offset from the counter where a future write would reuse them
  (section 6.22). The address bucket key, SHA-256 of the hash160, is
  exported because it is a pure function of the address
  (`wallet/udb/addressdb.go:1337-1350`).
- **P3 Fold indexes.** A secondary index is a field of the record it points
  at, holding what the index says (`account.idname`, `address.idxacct` and
  `inset`, `credit.utxo`, `commitment.unspent`, `multisig.utxo`,
  `vsphost.idhost`). An index entry whose target is missing is an
  `[orphans]` record.
- **P4 Export as stored.** A stored byte is dropped only when no reader at
  the pin can observe it; the list is exhaustive (section 9).
- **P5 Never storage order, never map order.** Every record type has a total
  order on its rendered key fields (section 7). The only stored order that
  is kept is a block's transaction list, which clients see through
  `RangeTransactions` (`wallet/udb/txquery.go:420-470`); it is data, not
  order. No Go map is ranged over when emitting; maps returned by udb are
  sorted first.
- **P6 Full values.** Exporters write unmasked values; masking and profiles
  are functions of the text (section 8).
- **P7 One snapshot.** An export reads one read transaction.
- **P8 Bounded memory.** Sections are written in registry order as they are
  produced, and nothing in the format requires a producer or a validator to
  hold the export in memory (section 11).

### 2. Lexical grammar

The file is printable ASCII (0x20-0x7E); every line ends with LF; there is
no CR, no empty line and no trailing space. ABNF (RFC 5234):

```
export   = magic *comment 7meta 29section eof
magic    = %s"purse-export 2" LF
comment  = "#" *(%x20-7E) LF                 ; header only; not digested, not compared
meta     = "@" mname "=" 1*(%x21-7E) LF      ; in the order of section 5
section  = sechead 1*typedecl *record sectail
sechead  = %s"=section" SP sname SP version LF
typedecl = %s"=type" SP rtype SP keycount *(SP fname ":" vtype ["?"] ["~" / "@"] ["^"]) LF
record   = rtype *(SP fname "=" value) LF
sectail  = %s"=end" SP sname SP %s"records=" count SP %s"sha256=" hex64 SP %s"msha256=" hex64 LF
eof      = %s"=eof" SP %s"sections=" count SP %s"sha256=" hex64 SP %s"msha256=" hex64 LF
sname, rtype, fname, mname = 1*(%x61-7A / DIGIT)
version, keycount, count   = "0" / %x31-39 *DIGIT
vtype    = "u" / "i" / "b" / "h" / "x" / "s" / "L" / "e"
hex64    = 64(DIGIT / %x61-66)
```

Every section of the registry (section 6) is present, in registry order
(the order of the headings of section 6), even when it has no records. Its
`=type` lines are exactly the registry's. A record lists every field of its
type, in declaration order, as `name=value`; a line splits on single spaces
and each field at its first `=`. Field flags: `?` the value may be `-`
(absent); `~` volatile: randomness, the host, or a clock udb reads itself
(masked by the masked profiles, section 8); `@` a clock value the caller
of udb supplies (masked only by the masked-clock profile); `^` write-only
at the pin (projected out by the observable profiles). A field is never
both `~` and `@`. Key fields are never `?`, `~` or `@`.

### 3. Value types

| Type | Rendering |
| --- | --- |
| `u` | unsigned decimal, no sign, no leading zeros |
| `i` | signed decimal, `-` only before a nonzero value; stored unsigned 64-bit amounts and times are read as two's-complement `int64`, as Go reads them |
| `b` | `0` or `1` |
| `h` | a chain hash: 64 lower-case hex digits in display order, the stored 32 bytes reversed (`chainhash.Hash.String()`) |
| `x` | bytes as lower-case hex in stored order; the empty string is `_` |
| `s` | a byte string between `"`: each byte 0x21-0x7E other than `"` and `\` as itself, every other byte (and only those) as `\xHH` in lower-case hex; no UTF-8 interpretation |
| `L` | comma-separated `h` values; the empty list is `_` |
| `e` | an enumerated token `[a-z0-9]+`, one of the values the table below lists for its field |

`-` is the value of an absent `?` field and never a value of any type.
Every value has exactly one rendering, so equal values render equally.

**Enumerated values.** Every `e` field takes only these values; producers
refuse to write and parsers refuse to read any other (a test checks this
table against both registries).

| Field | Values |
| --- | --- |
| `masterkey.which` | `pub`, `priv` |
| `cryptokey.which` | `pub`, `priv` |
| `cointype.which` | `legacy`, `slip0044` |
| `address.type` | `chain`, `key`, `script` |
| `orphan.index` | `addracct`, `acctaddrs`, `acctvars`, `acctidname`, `utxo`, `multisigutxo`, `cmtunspent`, `vsphostid` |
| `bad.reason` | `len`, `key`, `layout`, `type`, `value`, `missing`, `trail`, `counter`, `decrypt`, `unreached`, `api` |
| `failed.item` | `coin`, `tipheight`, `missingcf`, `utxocount`, `acctprops`, `balance`, `unspentticket` |
| `mkeyok.ok` | `yes`, `no`, `nopass`, `error` |
| `ctok.which` | `legacy`, `slip0044` |
| `ctok.ok` | `yes`, `no`, `nopass`, `error` |
| `slip0044acct0ok.ok` | `yes`, `no`, `nopass`, `error` |
| `acctok.ok` | `yes`, `no`, `nopass`, `error` |
| `acctok.via` | `wallet`, `account` |
| `addrok.ok` | `yes`, `no`, `nopass`, `error` |

### 4. Digests

- **Section digests.** `sha256` is SHA-256 over the section's record lines,
  each with its LF, in file order; `msha256` the same over the lines masked
  by the masked profile (section 8). `records` counts the record lines.
  Every section has them, `[residue]` included, so Go-against-Go
  determinism covers every section.
- **Eof digests.** `sha256` is SHA-256 over the concatenation of, first,
  `@<mname>=<value> LF` for each compared meta (`network`, `chain`,
  `decrypt`, `derived`, `privcheck`, section 5) in that order, then
  `<sname> SP <section sha256> LF` for every compared section in registry
  order; `msha256` the same with each section's `msha256`. Every section is
  compared except `[residue]`: it digests dcrwallet's dead layout items
  (section 9), which every version-26 Go database carries
  (`waddrmgr/main/mgrver` at least) and which dcr-purse's store, whose
  layout is its own, never has. With `[residue]` inside, no Go export could
  ever equal a Rust export. `[unknown]` stays inside: it is empty in every
  export that may be compared at all (one with `[unknown]` records was made
  with `allow_unknown`, as a diagnostic). `[bad]` and `[orphans]` stay
  inside: they show inconsistent state, which a wallet built by the same
  operations on the other side does not have, so a non-empty one is a
  difference to see. Comments and the metas `@origin` and `@exporter` are
  outside every digest.
- **Equality.** Two exports are equal under the exact (masked) profile when
  their eof `sha256` (`msha256`) are equal. The digests of the other
  profiles of section 8 are computed the same way from the records
  (`purse-export-check`'s `Export::digest` and `validate`); none is stored.
- **Chain segments** (digest mode, section 6.10).

### 5. Header metas

In this order: `@network` (`mainnet`, `testnet3`, `simnet`, `regnet`: the
parameters the export was produced with); `@chain` (`full` or `digest`,
section 6.10); `@decrypt` (`none`: no public passphrase, so no plaintext
field, no `[derived]` and no `[privcheck]`; `public`: udb was opened with
the public passphrase); `@derived` (`1` when `[derived]` was computed);
`@privcheck` (`1` when `[privcheck]` was computed); `@origin` (the producer
and source schema, `dcrwallet-udb/26` for the Go exporter); `@exporter` (the
producing program). The first five are compared, through the eof digests;
`@origin` and `@exporter` are not.

The metas must agree with the sections, and parsers check it:
`@derived=1` and `@privcheck=1` need `@decrypt=public`, and `@privcheck=1`
needs `@derived=1`; with `@derived=0` `[derived]` is empty, with
`@privcheck=0` `[privcheck]` is empty, and with `@privcheck=1` it has its
`mkeyok` record; with `@decrypt=none` every plaintext field
(`cointype.xpub`, `slip0044acct0.xpub`, `account.xpub`, `address.id`,
`address.addr`, `address.pubkey`) is `-`; with `@chain=full` there is no
`blockseg`, `hdrseg` or `cfseg` record, and with `@chain=digest` no
`header`, no `cfilter` and no `block` with an empty list and `inv=0`.

### 6. Section registry

Declarations are given exactly as the `=type` lines carry them (without the
`=type ` prefix). Citations are to `wallet/udb/` at the pin unless stated.
"Reader check" means a check Go's own reader performs; a row that fails one,
or that would make Go panic, becomes a `[bad]` record instead (section
6.24), and a field that fails one renders `-` and adds a `[bad]` record.

#### 6.1 `[wallet]` (version 1)

```
wallet 0 watchonly:b? lastaccount:u? lastimported:u? mgrcreated:u?~^ txcreated:i?~^ minedbal:i?^
```

One record. `watchonly`: `waddrmgr/main/watchonly`, exactly one byte,
nonzero is 1 (`addressdb.go:489-499`). `lastaccount`:
`waddrmgr/meta/lastaccount`, LE32, exactly 4 bytes (`addressdb.go:711-720`).
`lastimported`: `waddrmgr/meta/lastimportedaccount` as
`fetchLastImportedAccount` reads it: absent or empty is 2147483647; 4 bytes
above 2147483646 is the value; anything else is a reader-check failure
(`addressdb.go:724-743`). `mgrcreated`: `waddrmgr/main/mgrcreated`, LE64,
written once from udb's own `time.Now()` and never read
(`addressdb.go:1683-1686`). `txcreated`: `wtxmgr/date`, BE64, written from
udb's own `time.Now()`, never read (`txdb.go:2180-2185`). `minedbal`:
`wtxmgr/bal`, BE64; read only to be rewritten (`txmined.go:640-2226`),
never returned.

#### 6.2 `[masterkeys]` (version 1)

```
masterkey 1 which:e salt:x~ digest:x~ n:u r:u p:u
```

`which` is `priv` or `pub`. `waddrmgr/main/mpriv` and `mpub`: 88 bytes,
salt (32), SHA-256 of the derived key (32), LE64 N, R and P
(`addressdb.go:318-341`; snacl `Marshal`). The salt is random and the digest
follows from it. `mpriv` is absent in a watching-only wallet.

#### 6.3 `[cryptokeys]` (version 1)

```
cryptokey 1 which:e sealed:x~
```

`waddrmgr/main/cpriv` and `cpub`, verbatim (`addressdb.go:443-466`): snacl
boxes with random nonces.

#### 6.4 `[cointypes]` (version 1)

```
cointype 1 which:e encpub:x?~ encpriv:x?~ xpub:s?
slip0044acct0 0 encpub:x~ encpriv:x~ nextext:u nextint:u name:s xpub:s?
```

`cointype`: `which` is `legacy` (`ctpub`, `ctpriv`) or `slip0044`
(`ctpub-slip0044`, `ctpriv-slip0044`); a record exists when either half
exists (`addressdb.go:367-441`). `xpub` is the plaintext of `encpub` through
`Manager.Decrypt(CKTPublic, ...)` (`addressmanager.go:2622`), the extended
public key string. `slip0044acct0`: `waddrmgr/main/slip0044acct0`, a type-0
account row in the version-1 layout, decoded as
`UpgradeToSLIP0044CoinType` decodes it (`addressmanager.go:783-796`,
`addressdb.go:555-608`): sealed public and private keys, the two version-1
"next index" counters unconverted, and the name. Wallet creation writes
both coin types' keys and this row (`addressmanager.go:2961-3007`).

#### 6.5 `[accounts]` (version 1)

```
account 1 num:u type:u name:s? idname:s? encpub:x~ encpriv:x~ kdfsalt:x?~ kdftime:u? kdfmem:u? kdfthreads:u?~ extused:u? intused:u? extret:u? intret:u? xpub:s?
```

One record per `waddrmgr/acct` row (key LE32 number; value type byte, LE32
length, raw data `<LE32 len><encpub><LE32 len><encpriv>` with nothing left
over; `addressdb.go:99-137`, `518-537`). Only types 1 (BIP0044) and 2
(imported voting) are readable at version 26 (`addressdb.go:803-851`); any
other type is a `[bad]` record. `name` and the four counters come from
`waddrmgr/acctvars/<num>/` (`name`, `extused`, `intused`, `extret`,
`intret`; counters LE32, exactly 4 bytes; a missing name reads as `""`;
`addressdb.go:1032-1106`); `name` is `-` only when the variables bucket is
missing. `kdf*` decode `acctvars/<num>/kdf-params`, 25 bytes: salt (16),
LE32 time, LE32 memory, thread byte (`kdf/kdf.go:45-65`); all four are `-`
when it is absent. When present, the account has its own passphrase and
`encpriv` is an XChaCha20-Poly1305 seal (`addressmanager.go:170-183`);
otherwise it is a snacl box, or `_` (imported xpub accounts, the reserved
account 2147483647). The thread count is `min(runtime.NumCPU(), 256)`
(`kdf/kdf.go:31`), hence volatile. `idname` folds `waddrmgr/acctididx`
(`<LE32 len><name>`, read by `AccountName`, `addressdb.go:745-757`). `xpub`
is the plaintext of `encpub` (absent for an empty `encpub`).

#### 6.6 `[accountnames]` (version 1)

```
nameindex 1 name:s num:u?
```

One record per `waddrmgr/acctnameidx` entry: key `<LE32 len><name>`, value
LE32 account, read by `LookupAccount` (`addressdb.go:761-770`). An entry
naming a missing account is still a record; the lint flags it.

#### 6.7 `[addresses]` (version 1)

```
address 1 key:x id:x? addr:s? type:e account:u idxacct:u? inset:b branch:u? index:u? encpub:x?~ encpriv:x?~ enchash:x?~ script:x? pubkey:x? added:u~^
```

One record per `waddrmgr/addr` row: key SHA-256(hash160); value type byte,
LE32 account, LE64 add time, an unused byte, LE32 length, raw data
(`addressdb.go:1111-1152`). `type` is `chain` (raw data LE32 branch, LE32
index, exactly 8 bytes, `addressdb.go:1156-1188`), `key` (`<len><encpub>
<len><encpriv>`, `encpriv` `_` when no private key,
`addressdb.go:1190-1240`) or `script` (`<len><enchash><len><script>`, the
script in plaintext since version 14, `addressdb.go:1241-1295`,
`upgrades.go:1119-1260`). Fields that do not apply to a type are `-`.
`id`, `addr` and (for `key` rows) `pubkey` are what
`Manager.ForEachAccountAddress` returns for the row (`addressmanager.go:2428`):
`AddrHash()`, `Address().String()` and `PubKey()`; a row the API does not
reach gets `-` and a `[bad]` record `unreached`. `idxacct` folds the flat
`waddrmgr/addracctidx[key]` (LE32, `addressdb.go:907-926`); `inset` is 1
when the nested set `addracctidx/<LE32 account>/` holds the key (it drives
`ForEachAccountAddress` and `ImportedKeyCount`, `addressdb.go:1420-1449`;
its value is not exported, N10). `added` is the add time, udb's own
`time.Now()` at insertion (`addressdb.go:1360, 1375, 1390`) and never read.

#### 6.8 `[chain]` (version 2)

```
tip 0 hash:h
processed 0 hash:h
havecfilters 0 raw:x?
birth 0 set:b fromheight:b? fromtime:b? hash:h? height:u? time:i?@
```

`tip`: `wtxmgr/tip` copied into a 32-byte hash, zero-padded or truncated,
as `MainChainTip` reads it (`txmined.go:183-200`). Its other reader,
`ExtendMainChain`, compares the raw bytes with the new header's parent and
looks them up in `h` (`txmined.go:215-231`), so a tip of any length other
than 32 cannot be extended; such a tip is also a `[bad]` record (`len`, or
`missing` when absent). `processed`: `wtxmgr/lasttxsblock`, padded the same
way; its only readers use that padded copy (`txmined.go:277-319`), so its
length is not observable. `havecfilters`: the stored bytes verbatim,
because its two readers disagree on what they accept
(`txmined.go:213-216, 400-406`). `birth`: `wtxmgr/birthstate` as
`BirthState` reads it: it counts only at exactly 45 bytes (otherwise
`set=0` and the rest `-`); options bits 0 and 1, the hash, BE32 height and
BE64 time read as `int64` (`txmined.go:327-398`). The birthday time is
supplied by udb's caller in the `BirthdayState` passed to `SetBirthState`,
hence `@`; Go's zero time renders as `-62135596800`.

#### 6.9 `[blocks]` (version 2)

```
blockseg 1 seg:u n:u sha256:x
block 1 height:u hash:h time:u votebits:u inv:b txs:L
```

`wtxmgr/b`, key BE32 height; value hash (32), BE64 time, BE16 vote bits,
the stake-invalidated byte (nonzero is 1), BE32 n, then n hashes
(`txdb.go:216-243, 308-330`). `txs` keeps the stored order (P5). The block
time and vote bits duplicate the header; they are exported because
`TxDetails` reads the block time from this record (`txquery.go:71`). A
value longer than 47+32n bytes keeps its record (Go's reader serves the
first n hashes) and is also a `[bad]` record `trail` with the whole value:
`appendRawBlockRecord` appends a new hash after `len(v)`, not after
47+32n, and then raises n (`txdb.go:266-274`), so the next mined insert
(`txmined.go:1061, 1365`) turns the first 32 trailing bytes into
transaction n+1. In digest mode, `block` records with an empty list and
`inv=0` are folded into `blockseg` records (6.10).

#### 6.10 `[headers]` (version 1) and `[cfilters]` (version 1)

```
hdrseg 1 seg:u n:u sha256:x
header 1 height:u hash:h raw:x
hdrmissing 1 height:u hash:h
sideheader 1 hash:h raw:x
cfseg 1 seg:u n:u sha256:x
cfilter 1 height:u hash:h key:x filter:x
cfmissing 1 height:u hash:h
sidecfilter 1 hash:h key:x filter:x
```

The main chain M is the (height, hash) pairs of the block records. A
`wtxmgr/h` row (key block hash, value the serialized header,
`txdb.go:460-485`) or `wtxmgr/cf` row (value 16-byte key then the filter,
shorter is `[bad]`, `txdb.go:1968-2005`) whose hash is in M is main; every
other row is side. Neither bucket is ever pruned (spec 03 I1), so side rows
are reorganized-away blocks. `hdrmissing` and `cfmissing` list main heights
without a row (or, for filters, without a readable one).

Digest mode replaces the main-chain `header`, `cfilter` and empty `block`
records by one segment record per 4096 heights (segment k covers heights
4096k to 4096k+4095) holding at least one item: `n` items and the SHA-256
of their concatenation in ascending height. The items are format-defined
byte strings: block, `BE32(height) ‖ hash (stored order) ‖ BE64(time) ‖
BE16(vote bits)`; header, `BE32(height) ‖ hash ‖ BE32(len(raw)) ‖ raw`;
filter, `BE32(height) ‖ hash ‖ key (16) ‖ BE32(len(filter)) ‖ filter`.
Side rows and missing heights are always listed. Digest mode is a
deterministic function of full mode; tools convert full to digest, never
the reverse.

#### 6.11 `[txs]` (version 2)

```
tx 3 hash:h height:u block:h received:i@ raw:x
utx 1 hash:h received:i@ raw:x
```

`tx`: `wtxmgr/t`, key hash ‖ BE32 height ‖ block hash; value BE64 received
time then the serialized transaction, byte for byte (`txdb.go:487-526`).
`utx`: `wtxmgr/m`, keyed by hash, same value (`txdb.go:1351-1355`). The
received time is the one udb's caller passes to `NewTxRecord` or
`NewTxRecordFromMsgTx` (spec 23 §5.6.1), hence `@`, carried across mining
and rollback.

#### 6.12 `[unpublished]` (version 1)

```
unpublished 1 hash:h
```

One record per `wtxmgr/up` key whose value is exactly one nonzero byte, the
reader's rule (`txdb.go:1396-1420`); which nonzero byte is not observable
(N10); any other value is `[bad]`. A set of its own, because an `up` row
can outlive its `m` row (spec 03 I8).

#### 6.13 `[credits]` (version 1)

```
credit 5 tx:h height:u block:h index:u inv:b amount:i change:b optag:u coinbase:b expiry:b spent:b sptx:h? spheight:u? spblock:h? spindex:u? stype:u?^ scriptoff:u? scriptlen:u? account:u? utxo:b
```

`wtxmgr/c` (`inv=0`) and `wtxmgr/ic` (`inv=1`); key the 68-byte incidence
then BE32 index; value at least 9 bytes (`txdb.go:687-937`). Flags (byte
8): spent 0x01, change 0x02, `optag` = bits 2-4 (the condensed stake opcode
tag), coinbase 0x20, expiry 0x40. `sp*` are bytes 9-81 and only while spent.
`stype` (byte 81 low 7 bits, written and never read), `scriptoff` (BE32 at
82) and `scriptlen` (BE32 at 86) need 94 bytes; `account` (BE32 at 90) also
needs bit 0x80 of byte 81. `utxo` folds `wtxmgr/u`: 1 when the row for this
outpoint holds BE32 height ‖ block hash equal to this credit's key
(`txdb.go:1084-1103`).

#### 6.14 `[debits]` (version 1)

```
debit 5 tx:h height:u block:h input:u inv:b amount:i crtx:h crheight:u crblock:h crindex:u
```

`wtxmgr/d` (`inv=0`) and `wtxmgr/id` (`inv=1`); key the spender's incidence
then BE32 input; value BE64 amount then the 72-byte credit key
(`txdb.go:1166-1199`).

#### 6.15 `[ucredits]` (version 1)

```
ucredit 2 tx:h index:u amount:i change:b optag:u coinbase:b expiry:b spentbit:b stype:u? scriptoff:u? scriptlen:u? account:u?
```

`wtxmgr/mc`, key the outpoint; value 22 bytes (or 9 in the legacy form):
BE64 amount, flags as in 6.13 (`spentbit` is bit 0x01, never set), byte 9
script type (low 7 bits) and account-valid flag (0x80), BE32 offset, length
and account (`txdb.go:1422-1565`).

#### 6.16 `[uinputs]` (version 1)

```
uinput 2 tx:h index:u spender:h
```

`wtxmgr/mi`: key the spent outpoint, value (first 32 bytes) the spending
unmined transaction (`txdb.go:1678-1713`; Go panics on a shorter value).

#### 6.17 `[tickets]` (version 1)

```
ticket 1 hash:h picked:i?^
```

`wtxmgr/tix`, value BE32 read as `int32`. Every writer writes -1 and nothing
reads it (`txdb.go:1715-1746`); rows are never deleted.

#### 6.18 `[commitments]` (version 1)

```
commitment 2 tx:h index:u amount:i account:u? unspent:b unminedspent:b?
```

`wtxmgr/cmt`: BE64 amount, BE32 account (`txdb.go:2007-2054`). `unspent`
folds `wtxmgr/cmu` (the key is present); `unminedspent` is bit 0 of the
`cmu` value, `-` when not unspent (`txdb.go:2056-2089`).

#### 6.19 `[multisig]` (version 1)

```
multisig 2 tx:h index:u scripthash:x m:u n:u spent:b stake:b block:h height:u amount:i sptx:h? spindex:u? txhash:h utxo:b
```

`wtxmgr/ms`, exactly 135 bytes (`txdb.go:1748-1846`): script hash (20), m,
n, flags (spent bit 0, stake-tree bit 1, as stored; bits 2-7 not exported,
N11), block hash, BE32 height, BE64 amount, spender hash and BE32 index
(only while spent), the stored transaction hash. `utxo` folds the presence
of `wtxmgr/mu` (N10).

#### 6.20 `[agendas]` (version 1)

```
agenda 2 version:u agenda:s choice:s
ticketagenda 3 ticket:h version:u agenda:s choice:s
```

`agendaprefs` and `ticketsagendaprefs/<ticket hash>/`: key BE32 version ‖
agenda ID, value the choice ID (`agendas.go:23-62`). Every version is
exported, not only the current deployments.

#### 6.21 `[treasury]` (version 1)

```
tspendpolicy 1 tspend:h vote:u
vsptspendpolicy 2 ticket:h tspend:h vote:u
treasurypolicy 1 key:x vote:u
vsptreasurypolicy 2 ticket:h key:x vote:u
```

`tspendpolicy` (key tspend hash), `vsptspendpolicy` (ticket ‖ tspend),
`treasurypolicy` (key the treasury key bytes) and `vsptreasurypolicy`
(ticket ‖ key); the value is one byte, exported as stored (1 yes, 2 no;
setters delete instead of storing 0; `treasury.go:33-264`).

#### 6.22 `[vsp]` (version 2)

```
vspfuture 1 offset:u host:s?
vsphost 1 host:s pubkey:x idhost:s? future:u?
vspticket 1 ticket:h feetx:h status:u host:s pubkey:x? future:u?
```

Host IDs are surrogate keys (P2). `vsphost`: one per `vsppubkey` row (key
host; value BE32 ID, BE32 length, pubkey; `vsp.go:241-289`); `idhost` is
the host stored under that ID in `vsphost` (BE32 length ‖ host,
`vsp.go:186-234`), `-` if none. `vspticket`: one per `vsp` row (fee hash,
BE32 status, BE32 host ID; shorter than 40 bytes is `[bad]`, Go panics),
with host and pubkey resolved exactly as `GetVSPTicket` resolves them
(`vsp.go:98-127`): the host under the ID if present and non-empty, else
`""`; the pubkey stored for that host, else `-`.

The counter `vsphost/vsphostindex` (BE32) decides which IDs a later write
reuses: `SetVSPTicket` gives each new host the counter plus one, raises the
counter, and writes the host under that ID without checking whether the
slot is taken (`vsp.go:47-90`); it is the only writer of host rows besides
the upgrade that seeds ID 0 (`upgrades.go:1550-1595`). An ID at or below the
counter is never handed out again, so it is not exported. An ID above the
counter is exported as `offset` = ID − counter (at least 1): one
`vspfuture` record for each such ID that a `vsphost` key, a `vsppubkey` row
or a `vsp` row names, with `host` the host stored under it (`-` if none),
and `future` = that offset on the `vsphost` and `vspticket` records that
name it (`-` for IDs at or below the counter). Any `vspfuture` record also
files a `[bad]` record `counter` for the counter, which is behind an ID in
use. A missing or malformed counter is `[bad]` (`missing`, `len`; Go panics
on the next new host, `vsp.go:53-55`) and offsets are then taken from 0.

#### 6.23 `[orphans]` (version 2)

```
orphan 3 index:e ref:s val:s
```

Index entries whose target is missing. Account numbers in `ref` are
decimal when the stored key is 4 bytes (LE32) and otherwise `0x` followed
by the key in hex; hashes are in display order; keys of address rows are in
hex. By `index`:

| `index` | Entry | `ref` | `val` |
| --- | --- | --- | --- |
| `addracct` | a flat `waddrmgr/addracctidx` entry with no decodable `addr` row | the key in hex | the LE32 value in decimal |
| `acctaddrs` | an entry of the nested `waddrmgr/addracctidx/<a>/` with no decodable `addr` row of account a | `<a>:<key hex>` | `""` |
| `acctvars` | a `waddrmgr/acctvars/<a>/` bucket with no account row (its contents are not exported) | the account | `""` |
| `acctidname` | a `waddrmgr/acctididx` entry with no account row | the account | the stored value, verbatim |
| `utxo` | a `wtxmgr/u` row (36-byte key and value) whose mined credit is missing or does not decode | `<tx>:<index>` | `<height>:<block>` |
| `multisigutxo` | a `wtxmgr/mu` key with no decodable `ms` row | `<tx>:<index>` | the value in hex, `_` when empty |
| `cmtunspent` | a `wtxmgr/cmu` key with no decodable `cmt` row | `<tx>:<index>` | the value in hex, `_` when empty |
| `vsphostid` | host rows (other than ID 0 with an empty host) whose host has no `vsppubkey` row naming their ID | the host | the number of such rows, decimal |

An index entry whose own key or value has the wrong length is `[bad]`
instead.

#### 6.24 `[bad]` (version 2)

```
bad 3 path:s key:x reason:e value:x?
```

One per row or field that fails a reader check, would make Go panic, or
holds bytes a later write would turn into data. `path` is the bucket path,
components joined by `/`, each as itself when it is made of `[a-z0-9-]` and
in hex otherwise. `reason`: `len`, `key`, `layout`, `type`, `value`,
`missing` (a required row or bucket is absent), `trail` (bytes after a
block record's hash list, 6.9), `counter` (the VSP host counter is behind
an ID in use, 6.22), `decrypt` (a public-sealed blob did not open),
`unreached` (the address API did not return the row), `api` (an address API
call failed). `value` is the stored value, `-` when absent.

#### 6.25 `[unknown]` (version 1)

```
unknown 2 path:s key:x value:x?
```

Entries no rule claims (value `-` for a nested bucket). Written only with
`allow_unknown`; otherwise the export fails.

#### 6.26 `[derived]` (version 1)

```
coin 0 type:u?
tipheight 0 height:i
missingcf 0 v:b
utxocount 0 n:u
acctprops 1 num:u name:s type:u extused:u intused:u extret:u intret:u importedkeys:u encrypted:b
balance 2 num:u minconf:i immcoinbase:i immstakegen:i lockedtickets:i spendable:i total:i votingauth:i unconfirmed:i
unspentticket 1 hash:h
failed 1 item:e
```

Answers of the query API, never rows; empty when `@derived=0`.
`Manager.CoinType` (an error such as `WatchingOnly` renders `-`);
`Store.MainChainTip`; `IsMissingMainChainCFilters`; `UnspentOutputCount`;
`Manager.AccountProperties` for every account row (`AccountUnlocked` is
memory-only and left out); `Store.AccountBalances` at minconf 0 and 1, every
key of the returned map; `UnspentTickets` at the tip with immature tickets.
A call that errors or panics (Go panics are recovered) gives a `failed`
record naming it; error text is not exported.

#### 6.27 `[privcheck]` (version 2)

```
mkeyok 0 ok:e
ctok 1 which:e ok:e
slip0044acct0ok 0 ok:e
acctok 1 num:u ok:e via:e
addrok 1 key:x ok:e
```

Present only with the private passphrase (`@privcheck=1`); verdicts only,
never a secret or a digest of one. `ok` is `yes`, `no`, `nopass` (the
passphrase needed was not supplied, or the wallet did not unlock) or
`error`. `mkeyok`: `Manager.Unlock` with the private passphrase, which
opens the private master key and the private crypto key. `ctok`, for each
`cointype` record whose `encpriv` is present: the private half opened with
`Manager.Decrypt(CKTPrivate, ...)`, parsed as an extended key and
neutered, equals the plaintext of the same record's public half; it does
not use `CoinTypePrivKey`, which reaches only the coin type in use
(`addressdb.go:367-392`). `slip0044acct0ok`, when that row exists: the
same test on its two sealed halves (`UpgradeToSLIP0044CoinType` installs
them as account 0, `addressmanager.go:749-861`). `acctok`, for every
account whose sealed xpriv is non-empty: the neutered
`AccountExtendedPrivKey` equals `AccountExtendedPubKey`; `via` is `wallet`
(opened by the private passphrase) or `account` (by the account's own
passphrase through `UnlockAccount`). `addrok`, for every imported key with
a sealed private key: the public key of `Manager.PrivateKey` equals the
stored public key. A private key is unique for its public key, so these
verdicts prove that two wallets with equal public material hold equal
secrets in exactly these sealed fields: `masterkey` `priv`, `cryptokey`
`priv`, every `cointype.encpriv`, `slip0044acct0.encpriv`, every
non-empty `account.encpriv` and every imported key's `address.encpriv`.
These are every sealed private field of the format.

#### 6.28 `[residue]` (version 1)

```
residue 1 path:s entries:u sha256:x
```

Never compared and outside the eof digests (section 4). One record per dead
item of section 9 present in the source: the number of entries and the
SHA-256 of their lines in the `tools/oracle/cmd/boltdump` dump format
(version 1); plus `bucket-sequences` for buckets with a nonzero sequence
counter.

### 7. Ordering

Within a section, all records of the first declared type come first, then
the second, and so on. Within a type, records are ordered by their key
fields in declaration order, compared by type: `u`, `i` and `b`
numerically; `s` by its decoded bytes; `h`, `x`, `L` and `e` by their
rendering (for `x` this equals byte order); `_` sorts before and `-` after
every other value. The key fields of every record type identify a record:
two records of one type with equal keys are an error, for producers and
parsers alike. Every key in the registry is a logical identity (a bucket
key, an outpoint, an incidence, all three fields of `orphan`, and path, key
and reason of `bad`, where path and key fix the value). A type with key
count 0 has at most one record. No masked field is a key, so masking never
reorders records.

### 8. Masking and comparison profiles

Two classes of field are masked, each for its own reason.

**Volatile (`~`)**: values from randomness, from the host, or from a clock
udb reads itself, which no caller can steer without the seams of spec 23
§5.6.2-5.6.3. `masterkey.salt` (scrypt salt,
`wallet/internal/snacl/snacl.go:237`) and `masterkey.digest` (SHA-256 of a
key derived with that salt, `snacl.go:246`); every snacl box (24-byte
random nonce, `snacl.go:35-43`): `cryptokey.sealed`, `cointype.encpub`,
`cointype.encpriv`, `slip0044acct0.encpub`, `slip0044acct0.encpriv`,
`account.encpub`, `account.encpriv`, `address.encpub`, `address.encpriv`,
`address.enchash`; every XChaCha20-Poly1305 seal (`account.encpriv` of an
account with its own passphrase, `addressmanager.go:170-183`); the Argon2id
salt `account.kdfsalt` and thread count `account.kdfthreads`
(`kdf/kdf.go:30-39`); and udb's own clock reads `wallet.mgrcreated`,
`wallet.txcreated` and `address.added` (spec 23 §5.6.1). Spec 23 §5.6.4
omits the master-key digest; it belongs on the list.

**Injected clock (`@`)**: clock values udb's caller supplies:
`tx.received`, `utx.received` (the `received` argument of
`NewTxRecord`/`NewTxRecordFromMsgTx`) and `birth.time` (the
`BirthdayState` passed to `SetBirthState`). Every generated database and
every operation script injects them, the same on both sides, so they are
compared by default; spec 23 §5.6.4 masks the received time only "when the
scenario does not fix it".

Masking a line replaces each masked value other than `-` and `_`: an `x`
value by `*` and its byte length (so a blob of the wrong length still
shows), any other value by `*`. Masking is done by comparators and by the
`msha256` digests, never by producers.

Profiles:

- **exact**: unmasked lines (`sha256`). For Go against Go (determinism,
  stability) and for deterministic-build differentials once both sides
  share seeded randomness and udb's clock seam (open item 5).
- **masked**: `~` fields masked (`msha256`). For two wallets that cannot
  share randomness or udb's internal clock but were given the same
  injected clock values: the default for operation-script differentials,
  same-seed creation and scripted-history replays. Valid only when the
  private passphrase was supplied on both sides and every sealed private
  field present has a `yes` verdict (6.27), because masking hides sealed
  content; `purse-export-check`'s `Export::check_sealed_secrets` checks
  this.
- **maskedclock**: `~` and `@` fields masked; no stored digest. Only for
  comparisons whose scenario does not fix the clock: a live-synced restore
  or a soak, where each wallet stamps received times from its own clock.
  `birth` must then still be compared on its own where the scenario gave
  both sides the same birthday, since the restore contract names it (brief
  §3 W2); open item 10.
- **observable(P)**: profile P with every `^` field removed from every
  record (a comparator projection; no digest is stored for it). For a store
  that does not carry write-only values, if ADR-0003 so decides.

Map order: no output of format 2 follows Go map order. `AccountBalances` is
sorted by account; the treasury policy maps are not used (rows are read
directly); `UnminedTxs` is not used.

### 9. Normalisations and dead items (exhaustive)

Not exported, because no reader at the pin observes them:

- **N1** Bytes after the last field a reader reads, where no writer at the
  pin extends the value from its stored length: after the raw data of an
  account or address row, after the private-key field of an imported key
  and the script of a script address, after 94 bytes of a credit, 80 of a
  debit, 22 of an unmined credit, 32 of an unmined input, 4 of a ticket
  row, 12 of a commitment, 1 of an unspent commitment, a VSP host string or
  a VSP pubkey. Block records are not on this list (6.9). The audit behind
  it: the in-place rewriters at the pin (stake validation and
  invalidation, spending and unspending) copy trailing bytes along without
  reinterpreting them; the only appends after `len(v)` are
  `appendRawBlockRecord` (`txdb.go:266-274`, hence 6.9) and two upgrade
  steps (`upgrades.go:1512, 1605`) that never run on a version-26 database.
  The audit is repeated at every pin move (section 10).
- **N2** Address byte 13 (unused; `addressdb.go:1111-1134`).
- **N3** Credit flag bit 0x80 (never written, never read).
- **N4** Credit spender bytes 9-81 and multisig spender fields while not
  spent (stale after an unspend, spec 03 Q7; read only when spent).
- **N5** `cmu` value bits 1-7 and `birthstate` option bits 2-7.
- **N6** VSP host IDs at or below the counter, the counter's value, and the
  ID-0 seed row (what a later write depends on is carried by
  `vspfuture`, the `future` offsets, `vsphost.idhost` and `vsphostid`
  orphans, 6.22).
- **N7** Empty nested buckets of `ticketsagendaprefs` (no record to carry).
- **N8** `meta/ver` (consumed by the version check; recorded in `@origin`).
- **N9** bbolt's physical layout: pages, freelist, inline buckets, fill.
- **N10** Values of presence-only entries. Each nested
  `waddrmgr/addracctidx/<LE32 account>/<key>` value is written as one 0x00
  byte (`putAddrAccountIndex`, `addressdb.go:917-921`) and read by key
  presence only (`forEachAccountAddress`, `addressdb.go:1430-1445`;
  `upgrades.go:1178-1190`); each `wtxmgr/mu` value is written as 0x00
  (`putMultisigOutUS`, `txdb.go:1946-1952`) and read by presence only
  (`existsMultisigOutUS`, `txdb.go:1963-1966`); which nonzero byte a
  one-byte `wtxmgr/up` value holds (`existsUnpublished`,
  `txdb.go:1413-1416`, tests only for nonzero). Presence is exported, as
  `address.inset`, `multisig.utxo` and `[unpublished]`; a dangling `mu`
  entry carries its value in its orphan's `val`.
- **N11** Bits 2-7 of the `wtxmgr/ms` flags byte (byte 22):
  `fetchMultisigOut` and its single-field readers read only bit 0 (spent)
  and bit 1 (stake tree) (`txdb.go:1830-1836, 1859-1876`).

Dead items, sent to `[residue]`: `wstakemgr` (the legacy stake store, never
read at the pin, `upgrades.go:419-423`, spec 05 §4.5),
`waddrmgr/main/mgrver` (read only by the pre-unified migration,
`migration.go:43`, `addressdb.go:1721-1724`), `waddrmgr/main/seed`
(deleted by `upgrades.go:453`), `cscript` (deleted by `upgrades.go:1262`),
`lastaddrs` (deleted by the legacy manager's `upgradeToVersion5`,
`addressdb.go:1694-1706`), `waddrmgr/usedaddrs`, `waddrmgr/sync` (deleted by
the legacy manager's `upgradeToVersion6`, `addressdb.go:1709-1718`), keys
of `waddrmgr/meta` beginning `addrpoolext` or `addrpoolint` (of any length;
Go's are 15 bytes, `addressdb.go:212-233, 1593-1603`), `wtxmgr/vers`
(deleted by the migration, `migration.go:63`), `wtxmgr/sc` (deleted by
`upgrades.go:1255`), and nonzero bucket sequence counters.

Which of them a version-26 database can hold: `mgrver` always (written at
creation, `addressdb.go:1679`, never deleted at the unified versions), and
`wstakemgr` in a database migrated from the pre-unified layout.
`usedaddrs` is created by `createManagerNS` (`addressdb.go:1654-1658`) but
deleted by `lastUsedAddressIndexUpgrade` (`upgrades.go:395`), which every
new database runs, since `Initialize` and `InitializeWatchOnly` write
version 1 and then call `Upgrade` (`initialize.go:18-51, 56-90`). The
`addrpool` keys are deleted by the same upgrade only for accounts 0 to
`lastaccount` (`upgrades.go:296-389`) and have no writer at the pin
(`putNextToUseAddrPoolIdx`, `addressdb.go:1609-1621`, has no caller), so
keys an older release wrote for other account numbers survive into a
version-26 database. The rest are deleted by the steps cited above and can
reach version 26 only in a database those steps did not run on, which the
version check refuses.

### 10. Versioning and extension

The number in the magic line changes only when the grammar of section 2,
the value types of section 3 or the digest definitions of section 4
change; format 2 changed all three (the `@` flag, the enumerated-value
table, the eof digests over compared metas and compared sections). Each
section has a version; adding, removing, renaming or re-encoding a field
or record type, changing a flag, a key count, an enumerated value set, the
order rule, a normalisation, a digest item or the segment size raises it
and amends this ADR. Format 2 raised `[chain]`, `[blocks]`, `[txs]`,
`[vsp]`, `[orphans]`, `[bad]` and `[privcheck]` to version 2; N10 and N11
record normalisations format 1 already applied, so `[addresses]`,
`[unpublished]` and `[multisig]` keep version 1. Parsers validate against
the exact registry, so an export from another registry version is
rejected and regenerated, never compared. A move of the dcrwallet pin
re-audits the claim rules, the write-only tags, the normalisations
(including N1's append audit) and the dead-item list; a new schema version
needs a new section version at least.

### 11. Producers

**Go (`tools/oracle/cmd/walletexport`, body in
`tools/oracle/internal/walletexport`).** Commands `export {path, out,
network, pubpass?, privpass?, acctpass?, chain?, allow_unknown?, comments?}`
(writes the file; returns the eof digests, the file's SHA-256, the line
count, `{name, records, sha256, msha256}` per section and the metas) and
`sections` (the magic, the registry with each section's `compared` flag,
the enumerated values and the compared metas). It opens the database with
bbolt read-only (no write lock) and, with the public passphrase, runs
`udb.Open` on the same handle through a read-only `walletdb.DB` adapter;
dcrwallet's own `bdb` driver opens read-write
(`wallet/internal/bdb/db.go`). Every section except `[derived]` and
`[privcheck]` is decoded from raw buckets, each decoder citing the Go that
defines its encoding; plaintext fields (xpubs, address strings, hash160s,
imported public keys) come from `Manager.Decrypt` and
`Manager.ForEachAccountAddress`; `[derived]` and `[privcheck]` come only
from the API. The same code serves `tools/oracle/cmd/oprunner` (ADR-0002).

It streams (P8). Sections are written in registry order as they are
produced and each record is checked (order, distinct keys, enumerated
values), rendered, hashed and written at once. `[addresses]` follows its
bucket's cursor, whose order is the record order; `[blocks]`, `[headers]`
and `[cfilters]` follow the `wtxmgr/b` cursor (BE32 height order) with a
lookup per height in `h` and `cf`. Buckets keyed by internal hashes (`t`,
`m`, `up`, `c`, `ic`, `d`, `id`, `mc`, `mi`, `tix`, `cmt`, `ms`, and the
side rows of `h` and `cf`) are not in record order, which compares hashes
in display order, so their keys are collected as fixed-width sort keys
with the hash segments reversed, sorted, and the rows read back one by
one. The closed-world claims are kept per bucket when a producer consumes
every entry of a bucket, and per key only for the few entries claimed one
by one. Memory therefore grows with the row count only through those sort
keys (33 to 73 bytes per transaction-store row), the list of main-chain
block hashes (32 bytes per block) and the address API's answers (one per
address), never with the rendered text. The output goes to a temporary
file beside `out`, renamed over it when complete.

**Rust (`purse-udb`, later).** A writer of the same text from `purse-store`
through `purse-udb`'s own model, with `[derived]` from its own query code
and `[privcheck]` from `purse-snacl` and `purse-hdkeychain`. What it must
produce:

- every compared section with the records Go writes for the same state,
  each field from dcr-purse's own state: what dcr-purse stores, or what it
  can compute from what it stores. Its store's layout is its own (brief §3
  W2); the export is the contract, not the bytes;
- under the masked profile, every field that is not `~`: the account rows
  (numbers, types, names, the four counters, Argon2id time and memory, the
  plaintext xpubs), the address rows (identities, branches, indexes,
  scripts, public keys, both index memberships), the coin-type records and
  the parked SLIP0044 account 0 row that wallet creation writes
  (`addressmanager.go:2961-3007`) with its two version-1 counters, the
  chain markers, the block records with their per-block transaction order,
  the raw transactions with their injected received times, credits and
  debits with their stake-invalidated state, the stored credit account,
  script type and script location fields, the unspent index, unmined
  credits and inputs, unpublished markers (including one that outlives
  its unmined record, spec 03 I8, when the same operations leave one in
  Go), tickets, commitments, multisig outputs, agenda and treasury
  preferences, VSP records with host and pubkey resolved and, should its
  counter ever fall behind, the same offsets, and `[derived]` and
  `[privcheck]` from its own code; an account's name three times (the
  variables, the ID index and the name index), which it can render from
  one stored name for as long as dcrwallet's operations keep the three
  equal, and where an operation sequence makes them differ in Go, that is
  a quirk to reproduce or record under ADR-0012;
- the write-only `^` fields (`minedbal`, `stype`, `picked`, and the `~`
  ones `mgrcreated`, `txcreated`, `added`) unless ADR-0003 decides its
  store does not keep them, in which case comparisons use observable(P);
- `[residue]` and `[unknown]` always empty; `[bad]` and `[orphans]` empty,
  since its store holds only states dcr-purse writes (ADR-0013,
  Consequences) — a non-empty one in a Go export of the same history is a
  difference to examine, not something the Rust side must imitate;
- `@origin=purse-udb/<store schema version>`, and `@chain`, `@decrypt`,
  `@derived` and `@privcheck` as the comparison asks;
- identities and strings it derives itself (`purse-hdkeychain`,
  `purse-dcrutil`), which the comparison then checks.

Under the exact profile it would also have to reproduce the sealed blobs,
salts and nonces byte for byte, which needs its randomness to follow Go's
draw order (ADR-0002 Open item 2). Before `purse-udb` exists,
`crates/purse-export-check` holds a strict parser of the format (grammar,
registry conformance, enumerated values, metas, order, distinct keys,
masking, profiles, digests), in memory (`parse`) and line by line
(`validate`, memory bounded by the longest line), a frozen export, and the
conformance tests below.

### 12. Generated databases

`tools/oracle/cmd/walletgen` writes the databases the checks export:
`upgrade` decompresses an upstream fixture
(`wallet/udb/testdata/v*.db.gz`, kept in `crates/purse-boltread`) and runs
`udb.Upgrade` with testnet3 parameters and the public passphrase `public`,
as upstream's `TestUpgrades` does; `create` runs `udb.Initialize` from a
seed and passphrases on simnet and applies a scenario (`empty`, or `full`:
two BIP0044 accounts, a rename, an imported xpub account, an imported voting
account, an account passphrase, addresses on both branches with used and
returned counters, an imported private key and an imported 2-of-2 script; a
coinbase, regular payments with change, a ticket and its vote, a second live
ticket, two multisig outputs of which one is spent, a block disapproved by
its successor (stake-invalidated credit and debit), a reorganized block
(side header and filter), an unpublished unmined transaction with expiry and
an unmined spend of its change; agenda preferences, all four policy kinds, a
VSP ticket, the birthday and the processed marker); `reopen` opens a
database as dcrwallet does at start-up; `mutate` applies raw bbolt writes
to a generated database for fault injection. Every byte comes from udb at
the pin except those `mutate` writes.

Sections each unmutated database populates (records present):

| Database | Populated |
| --- | --- |
| `full` (simnet) | every section except `[orphans]`, `[bad]`, `[unknown]` |
| `empty` (simnet) | wallet, masterkeys, cryptokeys, cointypes, accounts, accountnames, chain, blocks, headers, cfilters, vsp, derived, residue |
| upgraded `v1` | as `empty`, plus addresses (1100) |
| upgraded `v2`, `v3`, `v4`, `v24` | as `empty` |
| upgraded `v5` | as `empty`, plus addresses, txs, credits, debits, ucredits, uinputs, tickets, commitments |
| upgraded `v7` | as `empty`, plus addresses, txs, credits |
| upgraded `v11` | as `v5` |

udb's public API offers no way to produce an inconsistent database, so
`[orphans]`, `[bad]` and the VSP offsets are exercised by `mutate` on a
copy of `full`: one orphan of every class, a block record with trailing
bytes, a tip of 33 bytes, a host row above the VSP counter and a ticket
naming an ID above it with no host row. Dead items are exercised the same
way (a `seed` key and a `wtxmgr/vers` key), and the `no` verdicts by
swapping the SLIP0044 coin-type private key for the legacy one and the
parked account 0's private key for account 0's. `[privcheck]` is filled
only for `full` (no private passphrase is given for the fixtures). Without
a chain backend the scenario also cannot produce: mined tickets that are
revoked, tspend and treasury-base transactions, a `cmu` row with the
unmined-spent flag, a watching-only wallet's `ImportPublicKey` rows, a
missing-cfilters state, or anything that depends on real proof of work
(none of which the store checks). A simnet harness (a later phase) covers
them.

### 13. Conformance checks (`crates/purse-export-check`)

With the oracle (`PURSE_REQUIRE_ORACLE=1` in CI on Linux, macOS, Windows and
arm64): the registries, enumerated values and compared metas of Go and Rust
are equal; every generated database exports twice to identical bytes, and
to identical bytes again after a dcrwallet re-open; every export parses
strictly, in memory and streamed, with `[bad]`, `[orphans]`, `[unknown]`
and `[derived].failed` empty; the `full` scenario populates every other
section; two `full` wallets of one seed and one injected clock with udb's
own randomness differ only in `~` fields, have equal masked digests and
pass the sealed-secret check, and with injected clocks a day apart differ
under masked and agree under maskedclock, only in `~` and `@` fields; the
digest-mode export equals the conversion of the full-mode export; a
version-24 database is refused; the fault-injected copy shows every orphan
class, the `trail`, tip `len` and `counter` records and both VSP offsets,
and still parses strictly; two exports that differ only in dead items have
equal eof digests; and the swapped private keys get `no` verdicts while
`[cointypes]` stays masked-equal. Independently of Go, `purse-boltread`
reads each database and checks the entry count of every bucket that maps
to a section, and the records of accounts (number, type, name, counters,
the kdf presence, `idname`), account names, addresses (type, account, add
time, branch and index, membership in the per-account index, the hash160
against the bucket key), block records (hash, the invalidated flag, the
transaction list), transactions (key parts and bytes), agenda preferences,
`lastaccount` and the tip. Without Go: the frozen export
`tests/data/full-simnet.pe` parses and validates, in memory and streamed,
with every profile digest agreeing; every registry section appears in this
ADR at its version, in registry order, with its declarations, and every
enumerated value set in the table of section 3; removing `[residue]` from
the frozen export leaves its eof line unchanged; and a range of malformed
files is rejected (among them duplicate keys, out-of-set enumerated values
and metas that contradict the sections).

## Coverage

### The restore contract

What the W2 restore contract names (brief §3 W2; ADR-0013 Open point 1)
and where a restore differential finds it in the export:

| Contract clause | Where |
| --- | --- |
| The same accounts by number | `[accounts]` `num`, `type`; `[derived].acctprops`; `wallet.lastaccount` |
| The same last-used and last-returned child index per branch | `account.extused`, `intused`, `extret`, `intret`; `acctprops` |
| The same active coin type, legacy or SLIP0044 | `[cointypes]` (which records exist), `slip0044acct0`, `[derived].coin` |
| The same birthday and rescan point | `[chain]` `birth`, `processed`, `tip` |
| The same balances by category | `[derived].balance` (seven categories at minconf 0 and 1) |
| The same mined transaction history | `[txs].tx`, `[blocks]` `txs`, `[credits]` and `[debits]` |
| The same tickets and their commitments | `[tickets]`, `[commitments]`, `[derived].unspentticket` |
| State the seed does not carry is absent on both sides | the sections of non-seed state are empty or equal on both sides: imported keys and scripts in `[addresses]`, imported and voting accounts in `[accounts]`, `[agendas]`, `[treasury]`, `[vsp]`, `[txs].utx`, `[unpublished]` |
| The same discovery outcome | the addresses and counters above; the requests the backend saw are compared outside the export (brief §7 Layer 6) |

### Everything dcrwallet persists

Every item of the former W2 import list, kept as the checklist of
"everything dcrwallet persists" (brief §5):

| Item | Where |
| --- | --- |
| Accounts with names, numbers, types | `[accounts]` (`num`, `type`, `name`, `idname`), `[accountnames]`, `[derived].acctprops` |
| Every address with branch and index | `[addresses]` (`branch`, `index`, `id`, `addr`) |
| Last-used and last-returned counters | `account.extused`, `intused`, `extret`, `intret`; `slip0044acct0.nextext`, `nextint` |
| Imported keys | `address` with `type=key` (sealed keys, `pubkey`); `[privcheck].addrok` |
| Imported scripts | `address` with `type=script` (`script`, `enchash`) |
| Imported xpubs | `account` with `num` above 2147483647, `type=1`, `encpriv=_`; `wallet.lastimported` |
| Voting accounts | `account` with `type=2` and the `kdf*` fields |
| Per-account passphrases | `account.kdfsalt`, `kdftime`, `kdfmem`, `kdfthreads`, the sealed `encpriv`; `acctprops.encrypted`; `[privcheck].acctok via=account` |
| Balances by category | `[derived].balance`; `wallet.minedbal` |
| Full history with block placement | `[txs].tx`, `block.txs` (stored order), `[credits]` and `[debits]` with `inv=0` |
| Unmined transactions, credits, spent inputs, unpublished flag | `[txs].utx`, `[ucredits]`, `[uinputs]`, `[unpublished]` |
| Stake-invalidated credits and debits | `credit` and `debit` with `inv=1`; `block.inv` |
| Tickets and their commitments | `[tickets]`, `[commitments]`; `[derived].unspentticket` |
| Multisig records | `[multisig]` |
| Agenda preferences, wallet-wide and per ticket | `[agendas]` `agenda`, `ticketagenda` |
| Tspend and treasury-key policies, wallet-wide and per VSP ticket | `[treasury]` (four record types) |
| VSP ticket records | `[vsp]` `vspticket`, `vsphost`, `vspfuture` |
| Birthday and rescan markers | `[chain]` `birth`, `processed`, `tip`, `havecfilters` |
| Stored headers and committed filters | `[headers]`, `[cfilters]` (full or 4096-height digests, side rows, missing heights) |
| Master key parameters and sealed crypto keys | `[masterkeys]`, `[cryptokeys]`, `[cointypes]`; `[privcheck]` |
| Memory-only state (outpoint locks, address-buffer cursors, side-chain forest, in-flight VSP fees) | not in the file; not exported |

### Schema-26 buckets and keys

| Bucket or key | Where |
| --- | --- |
| `meta/ver` | version check, `@origin` (N8) |
| `waddrmgr/main/mgrver` | `[residue]` |
| `waddrmgr/main/mgrcreated` | `wallet.mgrcreated` |
| `waddrmgr/main/mpub`, `mpriv` | `[masterkeys]` |
| `waddrmgr/main/cpub`, `cpriv` | `[cryptokeys]` |
| `waddrmgr/main/ctpub`, `ctpriv`, `ctpub-slip0044`, `ctpriv-slip0044` | `[cointypes].cointype`; `[privcheck].ctok` |
| `waddrmgr/main/watchonly` | `wallet.watchonly` |
| `waddrmgr/main/slip0044acct0` | `[cointypes].slip0044acct0`; `[privcheck].slip0044acct0ok` |
| `waddrmgr/main/seed`, `cscript`, `lastaddrs` | `[residue]` (deleted by upgrades, section 9) |
| `waddrmgr/acct` | `[accounts]` |
| `waddrmgr/acctvars/<n>/` (`extused`, `intused`, `extret`, `intret`, `name`, `kdf-params`) | `[accounts]`; a bucket without a row: orphan `acctvars` |
| `waddrmgr/acctididx` | `account.idname`; without a row: orphan `acctidname` |
| `waddrmgr/acctnameidx` | `[accountnames]` |
| `waddrmgr/addr` | `[addresses]` |
| `waddrmgr/addracctidx` (flat) | `address.idxacct`; dangling: orphan `addracct` |
| `waddrmgr/addracctidx/<n>/` | `address.inset` (N10); dangling: orphan `acctaddrs` |
| `waddrmgr/meta/lastaccount`, `lastimportedaccount` | `[wallet]` |
| `waddrmgr/meta/addrpool*` | `[residue]` (can survive from an older release, section 9) |
| `waddrmgr/usedaddrs`, `waddrmgr/sync` | `[residue]` (deleted by upgrades, section 9) |
| `wtxmgr/date`, `bal` | `wallet.txcreated`, `wallet.minedbal` |
| `wtxmgr/vers`, `wtxmgr/sc` | `[residue]` (deleted by the migration and an upgrade, section 9) |
| `wtxmgr/tip`, `lasttxsblock`, `havecfilters`, `birthstate` | `[chain]`; a tip not of 32 bytes also `[bad]` |
| `wtxmgr/b` | `[blocks]`; trailing bytes also `[bad]` `trail` |
| `wtxmgr/h` | `[headers]` |
| `wtxmgr/cf` | `[cfilters]` |
| `wtxmgr/t`, `m` | `[txs]` |
| `wtxmgr/up` | `[unpublished]` (N10) |
| `wtxmgr/c`, `ic` | `[credits]` |
| `wtxmgr/u` | `credit.utxo`; dangling: orphan `utxo` |
| `wtxmgr/d`, `id` | `[debits]` |
| `wtxmgr/mc` | `[ucredits]` |
| `wtxmgr/mi` | `[uinputs]` |
| `wtxmgr/tix` | `[tickets]` |
| `wtxmgr/cmt` | `[commitments]` |
| `wtxmgr/cmu` | `commitment.unspent`, `unminedspent`; dangling: orphan `cmtunspent` |
| `wtxmgr/ms` | `[multisig]` (N11) |
| `wtxmgr/mu` | `multisig.utxo` (N10); dangling: orphan `multisigutxo` |
| `agendaprefs`, `ticketsagendaprefs/<ticket>/` | `[agendas]` |
| `tspendpolicy`, `vsptspendpolicy`, `treasurypolicy`, `vsptreasurypolicy` | `[treasury]` |
| `vsp` | `[vsp].vspticket` (`future` above the counter) |
| `vsphost/<id>` | `vspticket.host`, `vsphost.idhost`, `vspfuture.host` above the counter; inconsistent: orphan `vsphostid`; ID 0 empty and IDs at or below the counter (N6) |
| `vsphost/vsphostindex` | the reference point of the offsets; behind an ID in use: `[bad]` `counter` (N6) |
| `vsppubkey` | `[vsp].vsphost`, `vspticket.pubkey` |
| `wstakemgr` | `[residue]` |
| bucket sequences, pages, freelist | N9; nonzero sequences in `[residue]` |

Exclusions in short: the normalisations N1 to N11 and the dead items of
section 9, each resting on the absence of a reader at the pin.

## Consequences

- A Go wallet and a Rust wallet that reached their state by the same
  operations, or by restoring the same seed against the same chain,
  compare as one equality of eof digests under the profile the scenario
  calls for (section 8). A difference names its section, record type, key
  and field; an index disagreement, a lost counter or an unaccounted row
  cannot pass silently, and dcrwallet's dead layout cannot fail it.
- The Go exporter carries its own row decoders, a second reading of the
  schema besides dcrwallet's. A shared misreading of a spec would pass on
  both sides of a comparison only if `purse-udb` made the same mistake in
  its own model; the decoders were written from the Go, the
  `purse-boltread` cross-read checks counts and simple records, and the
  `[privcheck]` and `[derived]` sections exercise the API on the same data.
- dcr-purse's store must be able to produce everything section 11 lists,
  including values dcrwallet only writes, unless ADR-0003 explicitly drops
  them.
- Equal exports are not equal behaviour: result order, cursor semantics,
  coin selection, notification order and error text are outside the
  format. The Phase 2 cursor-conformance test, the Phase 3 query sweep and
  ADR-0002's per-operation results are required alongside it.
- The normalisations rest on "no reader at the pin"; a pin move can turn a
  dropped byte into an observed one, and then a section version moves.
- Memory and time. Measured on a generated simnet database of 300,000 block
  records (451 MB) on an x86_64 Linux desktop, warm page cache: the format-1
  exporter, which held every record twice and one claim string per
  database entry, peaked at 2.3 GB RSS in full chain mode (3.1 s) and
  0.56 GB in digest mode (1.2 s); the streaming exporter peaks at 0.35 GB
  in both modes (2.4 s full, 0.9 s digest; RSS includes the database's
  memory-mapped pages) and writes byte-identical records. The Rust
  `validate` checks the 396 MB full export in 2.0 s at a 13 MB peak; `parse`
  keeps every record and needs memory in proportion to the file. A large
  mainnet wallet still gives an export of hundreds of megabytes in full
  chain mode (raw transactions, headers, filters); exports belong under
  an artifacts directory, and soaks use digest mode and `validate` unless
  they need the listed chain data. Performance is a project goal (ADR-0014,
  in preparation); these figures are the baseline for it.
- `tools/oracle/cmd/oprunner` writes its exports through the same code, so
  every frozen operation vector (`crates/purse-export-check/tests/data/opvectors`)
  is regenerated in format 2, and its clock-variation check compares under
  maskedclock rather than masked (section 8).

## What remains open

1. Whether comparisons include the write-only (`^`) fields: ADR-0003 (D2)
   decides whether dcr-purse's store keeps `minedbal`, `stype`, `picked`
   and udb's internal timestamps. With no import, any such value in
   dcr-purse comes only from its own writes, and udb's internal `time.Now`
   reads cannot be injected without spec 23 §5.6.3's seam. If the store
   drops them, comparisons run under observable(P).
2. Chain data in comparisons: whether dcr-purse's store keeps every header
   and committed filter as dcrwallet's does (ADR-0003). If it does not, or
   if the two sides sync different header ranges (a restore in SPV mode
   against one in RPC mode), comparisons need a projection that excludes
   `[headers]`, `[cfilters]` and the empty `block` records; it is not yet
   specified.
3. `[orphans]` and `[bad]` describe Go-written fixtures and fault
   injections only: dcr-purse's store never holds such states (ADR-0013,
   Consequences), so a Rust producer emits them empty, and no policy for
   repairing or preserving them is needed. Whether a non-empty one in a Go
   export of a scripted history should fail the differential outright, or
   be reported as a Go defect (D13), is for ADR-0002's runner to settle.
4. The minconf set of `[derived].balance` (0 and 1 here) and whether
   `[derived]` should grow (ticket status needs chain parameters beyond
   udb).
5. Seeded randomness and udb's clock seam in the export's own generator:
   `walletgen` uses dcrwallet's unmodified udb, so its wallets differ in
   `~` fields from run to run and compare masked; ADR-0002's runner already
   links a copy of udb under both seams. Whether `walletgen` should move
   onto that copy, so that the exact profile can be used for creation
   differentials, is open.
6. A crosscheck mode in the Go exporter (raw decodings against the read API
   on the same transaction, as design A proposed) is not built.
7. The simnet harness for the states listed in section 12.
8. The digest segment size (4096) and whether a drill-down command for a
   differing segment is wanted.
9. Spec 23 needs amending: §5.5.2 and §5.7 (steps 5 to 7) compare database
   dumps and hand files between the two sides, which the own store
   excludes; the state comparison ends in this export instead; §5.6.4 omits
   the master-key digest, and should say which clock values udb's caller
   supplies (`@`) and which udb reads itself (`~`).
10. Restore differentials that do not fix the wallet clock compare under
    maskedclock, which also masks `birth.time`, a clause of the restore
    contract. Either the restore comparator compares the `birth` record
    exactly on its own (proposed), or `@` is split into a received-time
    class and a birthday class in a later format.

## Decision by the project owner (2026-11-19)

Accepted as proposed, with these answers ("your recommendations for the
rest"):

1. *Format 2.* The magic line is `purse-export 2`, and the seven changed
   sections carry version 2.
2. *The birthday in restore differentials* (open item 10). The restore
   comparator compares the `birth` record exactly on its own; `@` is not
   split into two flags.
3. *`walletgen` on ADR-0002's seamed copy of udb* (open item 5). Later,
   after ADR-0002's open item 2 (whether the Rust store follows Go's draw
   order) is settled.
4. *The eof digests* leave out `[residue]` only; `[unknown]`, `[bad]` and
   `[orphans]` stay inside them.
5. *Chain data in comparisons* (open item 2). ADR-0003 answers the first
   half: the store keeps every header and committed filter, detached ones
   included (its open point 5). The projection for two sides that synced
   different header ranges is still to be specified.

The other items of "What remains open" stand as open.

## Discrepancies found

- The task description lists `wallet/udb/stake.go`; there is no such file at
  the pin. The legacy stake store is dead data (`upgrades.go:419-423`; spec
  05 §4.5), carried here as `[residue]` `wstakemgr`.
- `Manager.ImportPublicKey` is refused outside watching-only wallets
  (`addressmanager.go:1297`), as spec 02 says; an imported key without a
  private key therefore needs a watching-only wallet to generate.
- Upstream fixtures `v1` to `v7` were written on testnet2 (spec 23 §3.5) and
  are upgraded with testnet3 parameters, as upstream's `TestUpgrades` does,
  so their block 0 is testnet2's genesis while the genesis filter written by
  the version-15 upgrade is testnet3's (`upgrades.go:1279-1336`); their
  exports show `cfmissing` at height 0 and a `sidecfilter`. `v11` has a block
  1 without a filter and `havecfilters` 0, so `cfmissing` at height 1. These
  are the fixtures' states, not exporter faults.
- Format 1 of this record placed `[residue]` inside the eof digests while
  calling it never compared, ordered `[unknown]` before `[bad]` in section 6
  against both implementations, listed orphan tokens with hyphens that its
  own `e` rule forbids, let masking hide the injected clock and two sealed
  private keys, and called its normalisation list exhaustive without N10
  and N11; format 2 corrects each (an adversarial review, 2026-11-19).
- No disagreement between specs 02, 03, 05 and the Go was found in the parts
  this work relied on.
