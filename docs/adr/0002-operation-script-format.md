# ADR-0002 — The operation-script format, `purse-opscript 1`

- **Status:** **Accepted** as proposed (project owner, 2026-11-19), with the four
  questions put to the owner answered as recommended; see "Decision by the
  project owner" at the end.
- **Date:** 2026-11-19; revised the same day to agree with the owner's
  decisions of 2026-11-18 and 2026-11-19 recorded in [README.md](README.md) (ADR-0003's
  engine, ADR-0008, ADR-0011, ADR-0012, ADR-0013) and to correct the
  defects an adversarial review of the runner confirmed.

## Context

Layer 2 of the testing plan (brief §7) runs one scripted sequence of
wallet-database operations through dcrwallet and through dcr-purse and
compares every return value and the resulting wallet state. Phase 0 asks for
the Go half, the operation-script runner (brief §6 Phase 0); Phase 3's exit
criterion is "the operation-script differential passes on return values and
logical export for a corpus covering every `Manager` and `Store` method"
(brief §6 Phase 3). The two databases share no byte format (dcr-purse keeps
its own store, ADR-0003, engine redb 5, accepted 2026-11-18), so state is
compared through the logical export of ADR-0001; results need a canonical
encoding of their own.

Since ADR-0013 (accepted 2026-11-18) there is no import of a dcrwallet
database: a user migrates by restoring the seed, and brief §3 W2 is now the
restore contract (same seed, same chain, same discovery options: the same
accounts, "the same last-used and last-returned child index per branch, so
that the next address issued is the one dcrwallet would issue", the same
balances and history). A Go wallet and a Rust wallet are therefore compared
only when both reached their state by the same operations or by restoring
the same seed against the same chain (brief §7 Layer 2). Operation scripts
are the first of those two paths, and ADR-0013 Open point 6 (Proposed;
*decided as proposed, project owner, 2026-11-22*) gives them more to
carry: the wallet shapes the import corpus was to cover
are to be built on both sides by the same scripts (brief §6 Phase 3 exit),
and the golden-capture harness is to prepare its states with this runner
(brief §6 Phase 7). Because restore is the only migration path, address
issuance and its gap-limit state must be comparable too; udb has no
gap-limit logic (it lives in `wallet/addresses.go:505-649`), so the
vocabulary reaches into the wallet package for it (section 2).

Four facts shape the format:

1. **Nondeterminism.** udb draws salts, nonces, passphrase-hasher keys and
   coin-selection choices from `github.com/decred/dcrd/crypto/rand`, reads
   `time.Now()` for address add times and both creation dates, and takes the
   Argon2id thread count from `runtime.NumCPU()` (spec 23 §5.6.1). Without
   seams, two runs of one script differ.
2. **Transaction boundaries are behaviour.** What commits together in Go
   commits together in dcr-purse (brief §4.6); a script must be able to say
   that several calls share one database transaction, as a Go caller's
   `walletdb.Update` does, including callers that check a udb error and go
   on in the same transaction (`wallet/chainntfns.go:46, 450-455, 486-494,
   505-509`; `wallet/createtx.go:640`).
3. **Go map order and Go panics.** Several udb calls return maps, and a few
   panic on inputs a caller can supply; ADR-0012 (accepted) decides what
   dcr-purse does about a panic.
4. **Error texts are compared byte for byte** (ADR-0011, accepted), and
   dcrwallet's daemon renders nested errors with the separator `":: "`
   (`dcrwallet.go:36-39`), not the package default `":\n\t"`
   (`errors/errors.go:27`).

The Go runner is `tools/oracle/cmd/oprunner`; its header comment holds the
coverage table (every exported entry point of `wallet/udb` and the operation
that runs it, or why none does) and the catalogue of every nondeterminism
source with how it is injected.

## Options considered

**Script language.** (a) JSON with one object per operation, named by
operation and keyed by argument name; (b) a line-oriented text language like
the export; (c) Rust test code driving both sides. (c) cannot drive Go; (b)
needs a second parser on each side for nested values (transactions,
outpoints, groups). JSON is already the oracle's protocol (linejson) and
serde_json is in the workspace. Chosen: (a).

**Seams on the Go side.** (a) A module replacement of `dcrd/crypto/rand`
and `-overlay` for `time.Now()` (spec 23 §5.4.4, §5.6.2-5.6.3); (b) verbatim
copies of `wallet/udb` and the packages it draws through, rewritten only at
the draws, with a test that holds each copy to the pinned module; (c) no
seams, masked comparisons only. purse-testutil builds every oracle with a
plain `go build` in one module, so (a) would change `go.mod` for every
oracle and needs build flags the harness does not pass. (c) leaves coin
selection untestable. Chosen: (b), the copy technique spec 23 §5.5 already
uses for snacl.

**Address issuance (the gap limit).** (a) Copy the wallet package under the
same seams; (b) link the unmodified wallet package for address issuance
only; (c) leave it out. (a) copies far more than issuance needs. Issuance
draws no randomness and reads no clock in the wallet package; its one
nondeterministic value is the add time its unmodified udb writes into each
new address row (`wallet/udb/addressdb.go:1360`). (b) handles that with a
`walletdb` interposer that rewrites that field to the injected clock as the
row is put, so results and exports stay exact. (c) would leave the restore
contract's "next address issued" untested. Chosen: (b).

**Results.** (a) Go's own `encoding/json` of udb's types; (b) a projection
per operation into JSON objects with fixed member names, specified here
(section 4). (a) leaks Go field names, time zones and pointer identity into
the comparison. Chosen: (b). A projection specified only by the Go source
would leave the Rust runner to reverse-engineer it, so the shapes are part
of this record.

**Error text.** (a) The errors package's default separator; (b) the
daemon's; (c) the error split into segments. ADR-0011 wants the whole text
byte-exact as the daemon prints it, and the sibling oracle `cmd/seed`
already renders with the daemon's separator (`tools/oracle/cmd/seed/main.go:42`).
Chosen: (b).

**udb errors met while resolving a reference.** (a) Script faults; (b)
operation results. A well-formed reference to an account or block the
wallet does not hold is a question about the wallet's state, which is what
results record. Chosen: (b).

**Groups whose Go caller tolerates an error.** (a) Every error ends the
group; (b) a per-member list of tolerated kinds. Under (a) the final state
of every known case is still reachable (each tolerated udb error is a
side-effect-free early return: `txunmined.go:25-28`,
`addressmanager.go:1361-1363`, `txmined.go:216-223`, and `Manager.Address`
only reads), but not the transaction composition, with the tolerated call's
result recorded inside the transaction that commits. Chosen: (b).
*(2026-11-26: two tolerable udb errors at the pin are not side-effect
free; section 2, "Transactions", records them.)*

## Decision (accepted, 2026-11-19)

### 1. Script

A script is one JSON object:

```json
{
  "format": "purse-opscript 1",
  "name": "free text",
  "setup": {
    "network": "simnet",
    "seed": "<hex>",
    "pubpass": "public",
    "privpass": "private",
    "rand": "<64 hex digits>",
    "clock": 1794990000,
    "ncpu": 1,
    "chain_time": 1794000000
  },
  "export": {"chain": "full", "privcheck": true},
  "ops": [ {"op": "mgr.unlock", "pass": "private"}, ... ]
}
```

- `setup.network`: `mainnet`, `testnet3`, `simnet` or `regnet`. Exactly one
  of `seed` (hex; `Initialize`) and `xpub` (a key spec, section 2,
  neutered; `InitializeWatchOnly`). `xpub` may also be `{"raw": "<string>"}`,
  the string passed to `InitializeWatchOnly` unparsed, so that a malformed
  string, another network's key or a private key reaches udb (added
  2026-11-22 for the watching-only slice of `purse-udb`). `privpass` is
  ignored for a watching-only wallet, and its exports carry no
  `[privcheck]`.
- `setup.rand`: the 32-byte key of the injected randomness stream: ChaCha20
  under that key with a zero 12-byte nonce, no re-keying, XORed into the
  caller's buffer, consumed by dcrd's derived-value functions unchanged
  (spec 23 §5.6.2). Every draw of every operation comes from this one
  stream, in operation order.
- `setup.clock`: the Unix second every clock read returns until an
  `rt.clock_set` or `rt.clock_advance`. The clock does not move by itself.
- `setup.ncpu` (default 1): the processor count the Argon2id parameters
  record.
- `setup.chain_time` (default `clock`): the base of block timestamps, which
  are chain data, not clock readings: block *h* is stamped
  `chain_time + 300·h` unless the operation gives `time`.
- `export`: `chain` is `full` (default) or `digest` (ADR-0001 §6.10);
  `privcheck` (default true) passes the private passphrase and every
  per-account passphrase the database holds, by the runner's bookkeeping
  (section 2, "Runner state"), to the exporter.

A setup error (a seed udb refuses, for example) fails the run: setup is the
script's parameters, not an operation.

### 2. Operations

Every element of `ops` is an object whose `op` names the operation; every
other member is an argument, and every argument must be one the operation
reads (a misspelt argument is a script fault). Names are the Go entry point
in snake case under a prefix: `mgr.` (a `Manager` method), `store.` (a
`Store` method), `udb.` (a package function), `wallet.` (the wallet
session), `tx.` (transaction fixtures), `rt.` (the runner itself). The full
list is the runner's `vocabulary` command; the table below gives the
arguments (`?` optional). Results are in section 4.

| Operations | Arguments |
| --- | --- |
| `mgr.watching_only`, `is_locked`, `lock`, `chain_params`, `coin_type`, `coin_type_priv_key`, `upgrade_to_slip0044_coin_type`, `convert_to_watching_only`, `for_each_account`, `last_account`, `last_imported_account`, `for_each_active_address` | — |
| `mgr.unlock`, `unlocked_with_passphrase` | pass |
| `mgr.unlock_account`, `set_account_passphrase` | account, pass |
| `mgr.lock_account`, `account_has_passphrase`, `account_properties`, `account_extended_pub_key`, `account_extended_priv_key`, `account_name`, `for_each_account_address`, `for_each_active_account_address` | account |
| `mgr.account_branch_extended_pub_key` | account, branch |
| `mgr.change_passphrase` | old, new, private |
| `mgr.encrypt`, `decrypt` | key (`private` or `public`), data |
| `mgr.address`, `addr_account`, `mark_used`, `private_key`, `have_private_key`, `redeem_script` | addr |
| `mgr.exists_hash160` | hash160 |
| `mgr.import_private_key` | key (32 bytes; the runner makes the WIF), label? |
| `mgr.import_public_key` | pubkey, label? |
| `mgr.import_script` | script, label? |
| `mgr.import_xpub_account` | name, xpub |
| `mgr.import_voting_account` | name, pass, xpriv |
| `mgr.new_account`, `lookup_account`, `udb.validate_account_name` | name |
| `mgr.rename_account` | account, name |
| `mgr.mark_used_child_index`, `mark_returned_child_index` | account, branch, child |
| `mgr.record_derived_address` | account, branch, child, pubkey? (default: the derived child's) |
| `mgr.sync_account_to_addr_index` | account, index, branch |
| `udb.coin_types`, `udb.needs_migration` | — |
| `udb.hd_keys_from_seed` | seed |
| `udb.is_imported_voting` | type |
| `tx.build` | label, version?, locktime?, expiry?, inputs?, outputs?, received? |
| `tx.raw` | label, tx, received? |
| `store.extend_main_chain` | label?, parent?, height?, votebits? (default 1), stakeversion?, time?, filter?; or again (alone) |
| `store.main_chain_tip`, `processed_txs_block_marker`, `is_missing_main_chain_cfilters`, `missing_cfilters_height`, `unspent_output_count`, `unspent_outputs`, `for_each_unspent_outpoint`, `iterate_tickets`, `unmined_txs`, `unmined_tx_hashes`, `udb.birth_state` | — |
| `store.update_processed_txs_block_marker`, `cfilter_v2`, `get_serialized_block_header`, `get_block_header_time`, `get_block_header`, `block_in_main_chain`, `get_block_meta_for_hash`, `udb.extract_block_header` | block |
| `store.get_main_chain_cfilters`, `get_main_chain_block_hashes` | start, inclusive, count (the length of the storage the Go caller passes) |
| `store.get_main_chain_block_hash_for_height`, `rollback` | height |
| `store.insert_missing_cfilters` | blocks |
| `store.import_cfilters_v2` | start_height, filters |
| `udb.set_birth_state` | block?, height?, time?, from_height?, from_time? |
| `store.insert_mined_tx` | tx, block |
| `store.insert_mem_pool_tx`, `remove_unconfirmed`, `tx_details`, `ticket_details`, `tx`, `exists_tx`, `exists_tx_mined_or_unmined`, `tx_block_height` | tx |
| `store.add_credit` | tx, block?, index, change?, account |
| `store.add_ticket_commitment` | tx, index, account |
| `store.redeem_ticket_commitments`, `unique_tx_details`, `previous_pk_scripts` | tx, block? |
| `store.add_multisig_out` | tx, block?, index |
| `store.spend_multisig_out` | outpoint, spender, index |
| `store.set_published` | tx, published |
| `store.prune_unmined` | sdiff |
| `store.random_utxo` | minconf, syncheight, rand? |
| `store.select_inputs` | account, minconf, syncheight, targets, ignore?, rand? |
| `store.unspent_output` | outpoint, include_mempool |
| `store.is_unspent_outpoint`, `exists_utxo`, `get_multisig_output`, `spender` | outpoint |
| `store.unspent_tickets` | syncheight, include_immature |
| `store.own_ticket` | ticket |
| `store.unspent_multisig_credits_for_address` | addr |
| `store.account_balance` | minconf, account |
| `store.account_balances` | minconf |
| `store.range_transactions`, `range_blocks` | begin, end, stop_after? (batches or blocks after which the callback returns true) |
| `udb.set_vsp_ticket` | ticket, fee_hash, fee_tx_status, host, pubkey, vsp_host_id? |
| `udb.get_vsp_ticket`, `delete_vsp_ticket` | ticket |
| `udb.get_vsp_tickets_by_fee_status` | status |
| `udb.set_vsp_host` | id, host |
| `udb.get_vsp_host` | id |
| `udb.set_vsp_pub_key` | host, id, pubkey |
| `udb.get_vsp_pub_key` | host |
| `udb.set_tspend_policy` / `tspend_policy` | tspend, policy / tspend |
| `udb.set_vsp_tspend_policy` / `vsp_tspend_policy` | ticket, tspend, policy / ticket, tspend |
| `udb.set_treasury_key_policy` / `treasury_key_policy` | pikey, policy / pikey |
| `udb.set_vsp_treasury_key_policy` / `vsp_treasury_key_policy` | ticket, pikey, policy / ticket, pikey |
| `udb.tspend_policies`, `vsp_tspend_policies`, `treasury_key_policies`, `vsp_treasury_key_policies` | — |
| `udb.set_default_agenda_preference` / `default_agenda_preference` | version, agenda, choice / version, agenda |
| `udb.set_ticket_agenda_preference` / `ticket_agenda_preference` | ticket, version, agenda, choice / ticket, version, agenda |
| `wallet.open` | gap_limit? (default 20, `wallet.DefaultGapLimit`, `wallet/addresses.go:385`) (top level only) |
| `wallet.new_external_address`, `wallet.new_internal_address` | account, policy? (`error`, `ignore` or `wrap`; absent passes no option, Go's zero policy, which is `error`) |
| `wallet.branch_next_indexes` | account |
| `wallet.close` | — (top level only) |
| `rt.clock_set` / `rt.clock_advance` | unix / seconds |
| `rt.checkpoint` | name (not `final`; top level only) |
| `rt.reopen` | — (top level only) |
| `update`, `view` | ops (each member may carry `tolerate`) |

**Transactions.** An operation that takes a bucket or a transaction runs in
its own `walletdb.View` (queries) or `walletdb.Update` (changes), as a Go
caller making that one call would. `update` and `view` run their `ops` in one
transaction, as a Go caller making several calls in one `walletdb.Update` or
`View` does (`wallet.processTransactionRecord` inserts a transaction and adds
its credits in one, `wallet/chainntfns.go:438-676`). A member's error ends
the group and rolls it back, unless the member's `tolerate` lists the
error's kind: a list of dcrwallet kind names other than `Other`, matched as
a Go caller's `errors.Is(err, errors.<Kind>)` matches (`errors/errors.go:330-343`;
`Other` never matches there). A tolerated error is recorded in the member's
record with `"tolerated": true` and the group goes on in the same
transaction; the group commits, and its record is `ok`, when no untolerated
error occurs. A recovered panic is never tolerated. `tolerate` on a
top-level operation, an empty list, `Other`, `Panic` or an unknown name is a
script fault, and so is a changing operation inside `view` or a session
operation inside either group. Every tolerated udb error known at the pin
is a side-effect-free early return (Options), so `tolerate` changes what is
recorded and how calls are grouped, not which database states are
reachable. *(2026-11-26: two exceptions are known, both in the auxiliary
records of the table above. `SetVSPTicket` refuses an empty or over-long
host with `Invalid` after it has written the host row and raised the host
counter (`wallet/udb/vsp.go:47-88`, QK-0122), so a group tolerating that
refusal keeps a host row no public-key row names, a state dcr-purse's store
cannot hold (ADR-0003 R2); the Rust runner refuses `tolerate` listing
`Invalid` on `udb.set_vsp_ticket` as a script fault, while the Go runner
accepts and runs such a script, as
`a_tolerated_vsp_ticket_refusal_is_refused_by_the_rust_runner` shows.
`SetTicketAgendaPreference` creates the ticket's nested bucket before a put
that can fail with `IO`, so a group tolerating it keeps an empty bucket,
which holds no preference and which the logical export does not show;
neither runner refuses it. PARITY.md, "`purse-udb` auxiliary records".)*

`rt.checkpoint` snapshots the database (`walletdb.DB.Copy`) and writes its
export; `rt.reopen` closes the wallet and opens it again as dcrwallet does
at start-up, so every in-memory state (locks, unlocked keys) is rebuilt.

**The wallet session.** `wallet.open` runs `wallet.OpenDB` and `wallet.Open`
(`wallet/wallet.go:5340`) over the runner's database handle with the
script's gap limit, the runner's public passphrase and no network backend,
then closes the copied udb's `Manager`: the wallet package links the
unmodified udb, whose `Manager` caches state of its own, so the two must
never run interleaved. Until `wallet.close`, only `wallet.*` operations,
`rt.checkpoint` and the clock operations are allowed; every other
operation, and a script ending inside a session, is a script fault.
`wallet.close` opens the copied udb again on the same handle (its `Manager`
starts locked). The address buffers holding the gap state are memory only
(`wallet/addresses.go:446-460`) and live for one session. Opening the
wallet draws its `Manager`'s passphrase-hasher keys from the real
`crypto/rand`; they stay in memory and never reach a result. The session's
handle is an interposer that rewrites the add time of every address row put
into `waddrmgr`'s `addr` bucket (bytes 5 to 12 of the row,
`wallet/udb/addressdb.go:1136-1152`) to the injected clock, which is what
the copied udb's seam would have written. A failing `wallet.open` is an
operation result and leaves the copied udb open.

**Runner state.** The runner's bookkeeping that names or unlocks database
state is transactional with the database transaction that changed it: the
passphrases the database holds (handed to `rt.reopen` and the exporter),
block labels and imported-address labels are saved before every operation
and group and restored when it fails, so a change rolled back in the
database leaves no trace in the runner. The bookkeeping therefore follows
the database, never Go's in-memory state, which may differ after a rollback
(section 3, "In-memory state after a rollback"). Transaction labels
(fixtures, not database state), the extend counter of section 6 and the
injected clock are not rolled back.

**Values.** Hashes are 64 hex digits in display order, or a label;
amounts, heights and indexes are JSON integers; byte strings are lower-case
hex; passphrases and names are strings. Labels:

- **transactions** — `tx.build` builds a transaction and records it with
  `NewTxRecordFromMsgTx`, `tx.raw` records serialized bytes with
  `NewTxRecord`. The received time is `received` or the injected clock.
  Inputs are `{prev, value, sequence?, script?, tree?}` with `prev` an
  outpoint or `"coinbase"` / `"stakebase"` (the null outpoint). Outputs are
  `{value}` plus exactly one of `pay` (an address; `tag` may prefix a stake
  opcode: `sstx`, `ssgen`, `ssrtx`, `sstxchange`, `tgen`), `script` (hex),
  `commitment` (`{pay, amount, limits?}`, a ticket commitment output),
  `voteblock` (a block: hash and height), `votebits` (hex) or `opreturn`
  (hex).
- **blocks** — `store.extend_main_chain` builds the header (section 6) and a
  version 2 filter over the output scripts of the `filter` transactions; a
  block argument is a label, a hash or `"tip"`. A `block` given to a credit
  or query is turned into the `BlockMeta` a Go caller builds from the stored
  header; an object `{hash, height, time, votebits}` is that `BlockMeta`
  itself, as a Go caller builds it from a block notification, whether or
  not the store holds the header. `again` (alone) hands
  `ExtendMainChain` the stored header and filter of an existing block a
  second time, as a caller re-notified of a block does; it builds no header.
- **outpoints** — `{tx, index, tree?}`.
- **addresses** — an encoded address; `{"ref": label}` (the address of a
  labelled import); `{"acct", "branch", "index"}` (the P2PKH address of the
  derived key, through `AccountBranchExtendedPubKey` in the operation's
  transaction); `{"p2sh": script}`.
- **extended keys** — an encoded key, or `{"seed": hex, "path": ["44'",
  "1'", "0'"]}` (a master key on the script's network, then each child; `'`
  marks hardened).

**Script faults and results.** A script fault ends the run with an error and
no results; it is never an operation result. Script faults are: an unknown
operation or label (a label registered in a group that rolled back is
unknown), a missing, malformed or unknown argument, a JSON shape that is
not the argument's, and an operation where the script's structure forbids it
(the cases above). Everything the wallet's state decides is a result:
every error dcrwallet returns, including errors met while resolving a
well-formed reference (an account `AccountBranchExtendedPubKey` does not
know, an invalid derived child, a block whose header the store does not
hold), and every panic inside dcrwallet's code. A resolution error keeps
udb's kind (it wraps udb's error) and its text is
`<what>: <Go entry point>: <udb's text>`, where `<what>` is the operation
name and the argument (`mgr.address addr`, `store.add_credit block`,
`store.extend_main_chain parent`, `store.insert_missing_cfilters blocks`,
`store.extend_main_chain again`) or, for a `tx.build` output,
`tx.build <label> output <n>`, and the entry point is
`AccountBranchExtendedPubKey`, `Child(<index>)`, `GetBlockHeader`,
`GetSerializedBlockHeader`, `BlockHeader.Deserialize` or `CFilterV2`. Every
argument of an operation is checked before any reference is resolved, so a
resolution error never hides a malformed argument. A panic in the runner's
own code is a runner fault: like a script fault it fails the run, so that it
cannot pass for dcrwallet behaviour (section 3).

### 3. Results

The runner writes `results.jsonl`: the line `purse-opresults 1`, then one
JSON object per top-level operation:

```
{"i":0,"ok":null,"op":"mgr.unlock"}
{"drawn":48,"i":1,"ok":1,"op":"mgr.new_account"}
{"err":{"kind":"Exist","text":"item already exists: account named \"second\" already exists"},"i":2,"op":"mgr.new_account"}
{"i":3,"ok":null,"op":"update","ops":[{"err":{...},"i":0,"op":"store.insert_mem_pool_tx","tolerated":true},...]}
```

- Exactly one of `ok` (the return value, section 4; `null` for none) and
  `err`. `err.text` is the full `Error()` text rendered with the daemon's
  separator `":: "` (`dcrwallet.go:36-39`); the runner sets
  `errors.Separator` once, which covers its copies and the unmodified
  packages alike, since all import `decred.org/dcrwallet/v5/errors`.
  ADR-0011 makes the text byte-exact. `err.kind` is the name of the kind
  `errors.As` finds (`Other` when none) among dcrwallet's 21 kinds, or
  `Panic` for a Go panic inside dcrwallet's code.
- **Panics.** A panic is attributed by its stack: walking out from the
  panicking frame past runtime and dependency frames, the first frame in
  the copied packages or in a `decred.org/dcrwallet/v5/` package makes it
  dcrwallet's (`Panic`, with the panic value's text; a bounds-check panic's
  text is cut after "out of range", because the indices the Go runtime
  prints differ between amd64 and arm64), and the first frame
  in the runner's own code makes it a runner fault (the run fails).
  `walletdb.Update` and `View` roll back on a panic before the runner
  recovers it (`wallet/walletdb/interface.go:203-252`), so the run continues
  on the last committed state. ADR-0012 has dcr-purse return an error where
  Go panics; a comparison accepts any error against `Panic`.
- `tolerated: true`, on a group member only: its error matched its
  `tolerate` and the group went on.
- `drawn`, when nonzero: the bytes the operation read from the injected
  stream (a seeded Rust store that claims Go's draw order must match it).
- Groups carry the records of the operations that ran in `ops`.
- **Volatility markers.** A member name ending in `~c` holds a value that
  may depend on the injected clock, `~r` on the injected randomness, `~cr`
  on both (`received~c`, `sealed~r`, `selections~r`, `credit~r`,
  `sha256~cr`, `msha256~c`: the masked export digests mask `~` fields only,
  and an export's `@` fields, received and birthday times, follow the
  injected clock, ADR-0001 §8). Every other value depends on neither.
- **Map order.** Where Go returns a map, the result is a list sorted by key:
  hashes in display order, accounts numerically, keys bytewise.
  `store.unmined_txs` (dependency order, ties in map order) is sorted by
  hash, and `dependency_order` says whether Go's order placed every
  transaction after the unmined transactions it spends.
- **Pointer text.** `GetVSPHost` prints `&id` with `%v`
  (`wallet/udb/vsp.go:203`); the address becomes `<pointer>`.
- **In-memory state after a rollback.** Go's `Manager` changes its memory
  before the database transaction commits, and a rollback does not undo
  that: after a group that changed the private passphrase rolls back,
  `Unlock` refuses the old passphrase and accepts the new one until a
  reopen; after one that set an account passphrase, `AccountHasPassphrase`
  reports it; after `ConvertToWatchingOnly`, `WatchingOnly` reports true
  (corpus script `15-rollback-bookkeeping`). Results record this as Go
  behaviour; the exports and the runner's bookkeeping follow the database.
- Encoding: Go's `encoding/json` with HTML escaping off (object members
  sorted bytewise, no insignificant space). Byte identity is required
  between two Go runs; between Go and Rust the parsed JSON values are
  compared.

A run returns the results, `results_sha256` (of `results.jsonl`), the total
bytes drawn, and per export its summary (section 4, "ExportSummary").
Exports are written beside their snapshots as `<name>.pe`; `final` is always
written.

### 4. Result shapes

Each `ok` value has fixed, lower-case member names. Times are Unix seconds
(Go's zero time is −62135596800, as `time.Time{}.Unix()` gives); hashes are
64 hex digits in display order; byte strings are lower-case hex; amounts are
integers in atoms; addresses are encoded strings; extended keys are their
string forms; transaction types are `regular`, `ticket`, `vote`,
`revocation`, `tadd`, `tspend`, `treasurybase` (`type<n>` for any other).
`null` is written where the Go call returns nothing or a nil pointer.

Named shapes:

| Shape | Members |
| --- | --- |
| Address | `addr`, `account`, `addrhash` (hex), `imported`, `internal`, `multisig`, `kind` (`pubkey`, `script` or `other`); for `pubkey` also `pubkey` (hex), `index`; for `script` also `scriptversion`, `script` (hex) |
| AcctProps | `account`, `name`, `type`, `extused`, `intused`, `extret`, `intret`, `importedkeys`, `encrypted`, `unlocked` |
| Block | `hash`, `height` |
| BlockMeta | `hash`, `height`, `time`, `votebits` |
| Outpoint | `hash`, `index`, `tree` |
| TxRecord | `hash`, `tx` (the serialized transaction), `received~c`, `type`, `unpublished` |
| TxDetails | TxRecord plus `block` (BlockMeta), `height`, `credits` (list of `index`, `amount`, `spent`, `change`, `opcode`, `coinbase`, `expiry`), `debits` (list of `index`, `amount`); `null` for a nil `*TxDetails` |
| Credit | `outpoint` (Outpoint), `block` (BlockMeta), `amount`, `pkscript`, `received~c`, `coinbase`, `expiry`; `null` for a nil `*Credit` |
| Balances | `account`, `immaturecoinbaserewards`, `immaturestakegeneration`, `lockedbytickets`, `spendable`, `total`, `votingauthority`, `unconfirmed` |
| VSPTicket | `feehash`, `feetxstatus`, `vsphostid`, `host`, `pubkey` (hex) |
| ExportSummary | `file`, `sha256~cr`, `msha256~c`, `file_sha256~cr`, `lines`, `sections` (list of `name`, `records`, `sha256~cr`, `msha256~c`) |

Per operation (`—` is `null`):

| Operations | `ok` |
| --- | --- |
| `mgr.lock`, `unlock`, `unlocked_with_passphrase`, `unlock_account`, `lock_account`, `set_account_passphrase`, `change_passphrase`, `convert_to_watching_only`, `import_xpub_account`, `mark_used`, `mark_used_child_index`, `mark_returned_child_index`, `sync_account_to_addr_index`, `record_derived_address`, `rename_account`; `udb.validate_account_name` | — |
| `mgr.upgrade_to_slip0044_coin_type` | — ; `{passphrase_dropped}` (`true`) when account 0 had a passphrase of its own, which dcr-purse's upgrade drops and reports (QK-0767; added 2026-11-22): `oprunner` reads account 0's `kdf-params` from the bucket before Go's call, so both sides give it |
| `mgr.watching_only`, `is_locked`, `exists_hash160`, `have_private_key`; `udb.is_imported_voting` | boolean |
| `mgr.chain_params` | the parameters' `Name` |
| `mgr.coin_type`, `addr_account`, `lookup_account`, `new_account`, `last_account`, `last_imported_account`, `import_voting_account` (the account) | integer |
| `mgr.account_name` | string |
| `mgr.account_extended_pub_key`, `account_extended_priv_key`, `account_branch_extended_pub_key`, `coin_type_priv_key` | extended key string |
| `mgr.encrypt` | `{sealed~r}` (hex) |
| `mgr.decrypt`, `redeem_script` | hex |
| `mgr.account_has_passphrase` | `{has, unlocked}` |
| `mgr.account_properties` | AcctProps |
| `mgr.address`, `import_private_key`, `import_public_key`, `import_script` | Address |
| `mgr.for_each_account` | list of integers |
| `mgr.for_each_account_address`, `for_each_active_account_address` | list of Address, in Go's order |
| `mgr.for_each_active_address` | list of address strings, in Go's order |
| `mgr.private_key` | `{key, pubkey}` (hex; the compressed public key) |
| `udb.coin_types` | `{legacy, slip0044}` |
| `udb.hd_keys_from_seed` | `{cointype_legacy, cointype_slip0044, account0_legacy, account0_slip0044}` |
| `tx.build` | `{hash, type, tx}` |
| `tx.raw` | `{hash, type}` |
| `rt.clock_set`, `rt.clock_advance`, `rt.reopen`, `wallet.open`, `wallet.close`, `update`, `view` | — |
| `rt.checkpoint` | ExportSummary |
| `store.main_chain_tip`, `extend_main_chain` | Block |
| `store.processed_txs_block_marker`, `get_main_chain_block_hash_for_height` | hash |
| `store.update_processed_txs_block_marker`, `insert_missing_cfilters`, `import_cfilters_v2`, `insert_mined_tx`, `insert_mem_pool_tx`, `add_credit`, `add_ticket_commitment`, `redeem_ticket_commitments`, `add_multisig_out`, `spend_multisig_out`, `set_published`, `remove_unconfirmed`; `udb.set_birth_state` | — |
| `udb.birth_state` | `{hash, height, time, from_height, from_time}`, or `null` when none is stored |
| `store.is_missing_main_chain_cfilters`, `block_in_main_chain` (`{main, invalidated}`), `is_unspent_outpoint`, `exists_utxo`, `own_ticket`, `exists_tx`; `udb.needs_migration` | boolean (or the object named) |
| `store.missing_cfilters_height`, `unspent_output_count`, `get_block_header_time`, `tx_block_height` | integer |
| `store.get_main_chain_cfilters` | list of `{block, key, filter}` (hex) |
| `store.cfilter_v2` | `{key, filter}` (hex) |
| `udb.extract_block_header` | `{parent, height, time, raw_height}`; `parent` is hex of the header's bytes (internal byte order) |
| `store.get_serialized_block_header`, `get_block_header`, `tx` | hex of the serialized header or transaction |
| `store.get_block_meta_for_hash` | BlockMeta |
| `store.get_main_chain_block_hashes`, `prune_unmined`, `unspent_tickets` | list of hashes, in Go's order |
| `store.unmined_tx_hashes` | list of hashes, in Go's order |
| `store.rollback` | list of `{block, txs}` sorted by block hash; `txs` the hashes in Go's order |
| `store.random_utxo` | `{credit~r}` (Credit or `null`) |
| `store.unspent_outputs` | list of Credit, in Go's order |
| `store.unspent_output` | Credit |
| `store.for_each_unspent_outpoint` | list of Outpoint, in Go's order |
| `store.iterate_tickets` | list of TxRecord plus `block` (Block), `spender` (hash) |
| `store.get_multisig_output` | `{outpoint, tree, scripthash, m, n, tx, block, height, amount, spent, spentby?, spentbyindex?}`; `spentby` and `spentbyindex` only while `spent` is true, as `wallet/multisig.go` reads them (an unspent record's spender bytes, stale after an unspend, QK-0055, reach no reader; 2026-11-24) |
| `store.unspent_multisig_credits_for_address` | list of `{outpoint, scripthash, script, m, n, amount}` |
| `store.select_inputs` | `{selections~r}`: one `{target, amount, inputs}` per target, each input `{outpoint, value, script, redeemsize}` |
| `store.account_balance` | Balances |
| `store.account_balances` | list of Balances plus `key` (the map key), sorted by key |
| `store.tx_details`, `unique_tx_details` | TxDetails |
| `store.ticket_details` | `{ticket, spender}` (TxDetails each), or `null` when the transaction is not a ticket (Go returns nil details and no error, `wallet/udb/txquery.go:204-215`) |
| `store.exists_tx_mined_or_unmined` | `{mined, unmined}` |
| `store.range_transactions` | list of batches, each a list of TxDetails |
| `store.range_blocks` | list of Block |
| `store.previous_pk_scripts` | list of hex |
| `store.spender` | `{tx, index}` |
| `store.unmined_txs` | `{txs, dependency_order}`: `txs` a list of TxRecord sorted by hash |
| `udb.set_vsp_ticket` | `{vsphostid}` (the ID `SetVSPTicket` assigned) |
| `udb.delete_vsp_ticket`, `set_vsp_host`, `set_vsp_pub_key`, `set_tspend_policy`, `set_vsp_tspend_policy`, `set_treasury_key_policy`, `set_vsp_treasury_key_policy`, `set_default_agenda_preference`, `set_ticket_agenda_preference` | — |
| `udb.get_vsp_ticket` | VSPTicket |
| `udb.get_vsp_tickets_by_fee_status` | list of VSPTicket plus `ticket`, sorted by ticket |
| `udb.get_vsp_host` | hex of the host bytes |
| `udb.get_vsp_pub_key` | `{id, pubkey}` |
| `udb.tspend_policy`, `vsp_tspend_policy`, `treasury_key_policy`, `vsp_treasury_key_policy` | integer (the policy byte) |
| `udb.tspend_policies` | list of `{tspend, policy}` sorted by tspend |
| `udb.vsp_tspend_policies` | list of `{ticket, tspend, policy}` sorted by ticket, then tspend |
| `udb.treasury_key_policies` | list of `{pikey, policy}` sorted by key bytes |
| `udb.vsp_treasury_key_policies` | list of `{ticket, pikey, policy}` sorted by ticket, then key bytes |
| `udb.default_agenda_preference`, `ticket_agenda_preference` | string |
| `wallet.new_external_address`, `new_internal_address` | `{addr, accountname, path}`; `path` is `{account, branch, child}` as `BIP0044Address.Path` returns it (account 0 for an imported xpub account, `wallet/addresses.go:298-304`) |
| `wallet.branch_next_indexes` | `{external, internal}` |

### 5. Determinism rules

- One script runs on one goroutine; every draw and clock read happens in
  operation order. The exporter's own udb (unmodified, used on snapshots)
  and the wallet session's unmodified udb never touch the injected stream;
  what they draw stays in memory.
- Two runs of one script give byte-identical `results.jsonl` and exports.
- A change of only `setup.rand` changes only `~r`/`~cr` values, `drawn`,
  and the exports' `~` fields; a change of only `setup.clock` (with
  `chain_time` fixed) changes only `~c`/`~cr` values and the exports' `~`
  and `@` fields (equal under ADR-0001's masked-clock profile). Both are
  checked by `crates/purse-export-check/tests/opscripts.rs`.
- Seeds are always supplied (`hdkeychain.GenerateSeed` reads the operating
  system and is never reached).
- The copy test fails on any clock read, randomness import or processor
  count left anywhere in the copied packages, not only where a rewrite
  applies; the unmodified dependencies the copies call cannot be checked
  that way, and the header of `tools/oracle/cmd/oprunner/copies_test.go`
  records their hand audit at the pin (sources exist in `txauthor`,
  `secp256k1`, `hdkeychain`, `txscript`, `wire` and `slog`; none is reached).

### 6. The Rust runner

`purse-udb`'s runner will read the same scripts and write the same
`results.jsonl` and export. It implements the operations over its own store
(ADR-0003: redb 5; a `walletdb.Update` or a group is one redb write
transaction, a `View` one read transaction), with `purse-rand`'s seeded
constructor for `setup.rand`, its clock object for `setup.clock`, and
`setup.ncpu` as the thread count it writes. It drives the wallet's
blocking API from one OS thread (ADR-0008; one script, one thread, as the
determinism rules require); no async runtime is involved. Its speed is a
design input like everywhere else (ADR-0014, in preparation), within what
this format fixes: the transaction boundaries above are semantics, not
overhead, and are kept. What it reproduces exactly, besides each
operation's call:

- the result shapes of section 4 and the error texts of section 3 (with
  `":: "` between nested errors, ADR-0011), including the resolution-error
  texts of section 2;
- the script-fault and runner-state rules of section 2: references
  resolved after every argument is checked, labels and passphrases rolled
  back with the transaction that registered them, `tolerate` matched by
  kind;
- the wallet session's rules and results (its gap state lives in memory
  for one session, as dcrwallet's address buffers do);
- the fixtures that have no store counterpart (`tx.build`, `tx.raw`,
  `rt.*`, `store.extend_main_chain`'s header). The header is version 7; the
  parent and height given or the tip and its height plus one; votebits,
  stake version and time as given (defaults 1, 0,
  `chain_time + 300·height`); bits the network's proof-of-work limit; nonce
  *n*, the number of header-building extend calls so far in the run
  counting this one (every call without `again` that passes argument
  checking, failed and rolled-back calls included); merkle root and stake
  root the SHA-256 of the ASCII text `merkle/<height>/<n>` and
  `stake/<height>/<n>`; every other field zero. Its filter is a version 2
  filter (`blockcf2` B and M) keyed by the merkle root over the output
  scripts of the `filter` transactions in order.

The differential is then:

1. per operation, equal `ok` values, or errors of equal kind and text (any
   error against `Panic`, ADR-0012);
2. equal exports under the exact profile where the Rust store claims Go's
   draw order and clock reads, otherwise under the masked profile with
   `[privcheck]` all `yes` (ADR-0001 §8), and with results compared after
   masking `~r` (and `drawn`) or `~c` values;
3. the frozen vectors in `crates/purse-export-check/tests/data/opvectors`
   (scripts with Go's results and final exports: `02-addresses`,
   `08-unmined`, `12-policies`, `16-tolerated-errors`,
   `17-resolution-errors`, `18-wallet-addresses`) as the cases that run
   without Go; `02-addresses` holds a nested error text, pinning the
   separator.

## Consequences

- Phase 3's operation-script differential has its Go half: a corpus that
  runs every operation (18 hand-written scripts, together covering the
  whole vocabulary, and generated scripts) and six frozen vectors. The same
  scripts can build the wallet shapes ADR-0013 Open point 6 (Proposed;
  *decided as proposed, project owner, 2026-11-22*) assigns to them.
- Coin selection (`MakeInputSource`, `RandomUTXO`) is testable byte for
  byte, given a Rust store that consumes the stream as Go does (spec 04
  §5.1).
- Address issuance under the gap limit, every policy and the next-index
  state are compared exactly, which the restore contract's "next address
  issued" needs.
- The runner links a second copy of udb. A pin move regenerates it
  (`PURSE_REGEN_COPIES=1`); the copy test then fails on any new clock read,
  randomness import or processor count inside the copied packages, and the
  dependency audit in its header is repeated by hand.
- Results expose Go's quirks as data: errors with odd texts, kinds of
  `Other`, panics, votebits of 0 in transaction details, a block time of the
  zero time in unspent outputs, the in-memory state a rollback leaves. Each
  is something the Rust side reproduces or records under ADR-0012.
- The corpus found states the export ADR thought unreachable through the
  public API: `RecordDerivedAddress` with a public key that is not the
  derived one leaves an address row no reader reaches (`[bad]`, reason
  `unreached`).
- The corpus now confirms with Go three quirks recorded from reading the
  source: QK-0253 (address issuance persists only the address rows, through
  `SyncAccountToAddrIndex`, which by its own comment leaves the
  last-returned index alone, `wallet/udb/addressmanager.go:2058-2060`, so a
  reopened wallet issues the same addresses again: corpus script
  `18-wallet-addresses`), QK-0270 (gap limit 0 under the ignore policy
  divides by zero, `wallet/addresses.go:584`: generated scripts) and
  QK-0086 (the unchecked block-record reads of `Store.Rollback` and
  `Store.AddCredit`: corpus scripts `06-chain`, `17-resolution-errors`).

## What remains open

1. The rest of the wallet-package runner (spec 23 `wallet.script`):
   transaction creation, ticket purchase, discovery. They draw randomness in
   the wallet package and need either a copy of it under the same seams or
   a decision that those paths compare masked only.
2. Whether the Rust store commits to Go's draw order (`drawn` equality and
   exact exports), or only to masked equality (spec 23 §13). *For the
   transaction store's random selections it does (addendum of
   2026-11-24): their results, `~r` values included, and their `drawn`
   are compared exactly; the address manager's draws stay masked.*
   *Dated note, 2026-11-27 (a fact for the question whether the brief's
   Phase 3 exit clause on same-seed creation is met by the masked profile
   or waits on this item, pending owner; no comparison profile changed):
   fed Go's stream, creation draws what Go's creation draws.
   `creation_draws_on_gos_stream` (`crates/purse-udb/tests/oracle.rs`)
   keys dcrd's generator with a creation script's `setup.rand`, runs
   `purse_udb::initialize` (or `initialize_watch_only`), then
   `Manager::open`, then the script's operations (the SLIP0044 upgrade and
   the birthday) on it, and sets the result beside Go's run of the same
   script (`Initialize` or `InitializeWatchOnly` with the upgrades, then
   `Open` and the operations, on the stream that key seeds), for one
   script per network and creation flow of the creation set (27): both
   setups draw 400 bytes for a seed's wallet (368 creating, 32 for the
   passphrase hasher's key at open) and 256 for a wallet made from an
   account extended public key (224 and 32), the operations draw nothing
   on either side, and the two final exports are equal unmasked, every
   salt, digest and sealed box included. Go's upgrades draw nothing on a
   new wallet. Not covered: the address manager's draws after setup
   (accounts, imports, passphrases), which no test has set beside Go's on
   one stream; and the Rust runner still gives the address manager a
   stream of its own (section 6), so its comparisons stay masked. The
   setup's count is reported by both runners (addendum of 2026-11-27).*
3. Whether `rt.reopen` should also be exercised after every write in a
   generated mode (crash-consistency against Go's in-memory state).
4. Chain-backend states (revocations of real chains, tspends, a
   missing-cfilters state) await the simnet harness.
5. Where the upstream findings go (QUIRKS.md and the D13 list). Already
   recorded, now confirmed by the oracle: the panics of `Store.Rollback` to
   a height more than one above the tip (`wallet/udb/txmined.go:2232-2233`)
   and of `Store.AddCredit` of a regular transaction with a `BlockMeta`
   whose header the store does not hold (`txmined.go:1394-1396`,
   `extractBlockHeaderHeight` of a nil header, `:570`), both QK-0086; the
   divide by zero of `Wallet.nextAddress` with gap limit 0 under the ignore
   policy (`wallet/addresses.go:584`; `--gaplimit` is not checked,
   `config.go:111`), QK-0270; addresses issued again after a reopen,
   QK-0253. Not yet recorded: the pointer in `GetVSPHost`'s error text, and
   the in-memory state a rollback leaves (section 3).
6. Whether the Rust side reproduces the in-memory state Go keeps after a
   rollback, or records the divergence under ADR-0012.

## Decision by the project owner (2026-11-19)

Accepted as proposed, with these answers ("your recommendations for the
rest"):

1. *Address issuance* is covered through dcrwallet's unmodified wallet
   package plus the add-time interposer, not by copying the wallet package.
2. *The in-memory state Go keeps after a rolled-back transaction* (open
   item 6; QK-0733) is not reproduced: dcr-purse keeps memory consistent
   with the store, and the difference is a recorded divergence under
   ADR-0012 Rule 2. Corpus script `15-rollback-bookkeeping` keeps Go's
   results as the record of the quirk.
3. *The runner-defined resolution-error texts*
   (`<what>: <entry point>: <udb text>`) are part of the format, and the
   Rust runner reproduces them.
4. *Freezing `15-rollback-bookkeeping` as a vector* waits until the Rust
   runner exists, since the vector pins the results of item 2 that the Rust
   side will not match.

Open items 1 to 5 above stand as open.

## Addendum, 2026-11-19 — The bounds-panic cut in other oracle commands

The rule under "Panics" above, that a bounds-check panic's text is cut
after "out of range" because the indices the Go runtime prints differ
between amd64 and arm64, was written for the operation-script runner.
Two later oracle commands apply the same cut, citing this record:
`tools/oracle/cmd/txauthor` (QK-0745) and `tools/oracle/cmd/boltcursor`
(QK-0748), so that their frozen vectors hold no architecture-dependent
indices. The project owner kept the cut in both (2026-11-19). As here,
nothing compares the indices: dcr-purse returns an error where Go panics.

## Addendum, 2026-11-24 — A stream key for the random store operations

`store.random_utxo` and `store.select_inputs` take an optional `rand`
argument, a 32-byte key (hex). The runner restarts its injected stream
under that key (`rand.Reseed` in `tools/oracle/cmd/oprunner/internal/rand`,
which, unlike `Seed`, keeps the count of bytes drawn, so `drawn` stays a
per-operation count) after every other argument is read and right before
the store draws, so the operation draws from the start of the stream the
key names whatever the script drew before it. Without `rand` the
operation continues the stream where it stands, as before; `setup.rand`
and every other operation are unchanged, and a script without `rand`
gives the same results as before the argument existed.

Why: the Rust runner's address manager draws from a stream of its own
(section 6; open item 2), so after setup the Rust runner cannot know
where Go's stream stands, and before this argument the random selections
could be compared only masked. With the key, the Rust runner keys dcrd's
generator (dcroxide's `Prng::from_seed`, the ChaCha20 stream of spec 23
§5.6.2) with it and draws what Go draws: the selections, the credits
`RandomUTXO` picks and `drawn` are compared exactly
(`crates/purse-udb/tests/support/compare.rs`, `RANDOM_OPS`). A random
operation without `rand` is accepted by the Rust runner only right after
another random store operation, with no operation in between that may
draw (an address manager, `rt.*` or wallet operation), and is refused as
a script fault otherwise, since the runner cannot know where Go's stream
stands. Group members carry `drawn` on both sides.

The keys are test inputs, never sent and never persisted; dcrwallet's
behaviour is not changed by them, only the stream the copied `crypto/rand`
yields at that point.

## Addendum, 2026-11-27 — The setup's bytes drawn

*Pending owner.* A run of `tools/oracle/cmd/oprunner` also
returns `setup_drawn`: the part of `drawn` the setup read from the
injected stream (`Initialize` or `InitializeWatchOnly`, the upgrades and
`Open`), before the first operation. `results.jsonl` and every record are
unchanged. The Rust runner reports the same count of its own setup
(`Run::setup_drawn` in `crates/purse-udb/tests/support/runner.rs`:
`purse_udb::initialize` or `initialize_watch_only`, then
`Manager::open`). Its stream is not Go's, so only the counts can be set
side by side: `same_seed_creation_matches_go` reports them for every
creation script (equal on every script of the creation set: 400 bytes for
a seed's wallet, 256 for one from an extended public key), and
`creation_draws_on_gos_stream` runs the creation on Go's stream to compare
the bytes themselves (open item 2, dated note of 2026-11-27). No
comparison of the differential reads either count.

## Addendum, 2026-11-27 — Groups in the Rust runner as one store transaction

Section 6 says a group is one redb write transaction (a `view` one read
transaction). Until this change the Rust runner emulated that: each member
ran in its own committed transaction, an `update` group that ended on an
untolerated error put a copy of the store file back, and address manager
operations were refused as members, since a file put back would bypass the
manager's memory. The runner now runs an `update` group inside one
`purse-store` write transaction, committed when no member's error ends the
group and rolled back when one does, and a `view` group on one read
transaction; every member reads and writes through the group's
transaction (`crates/purse-udb/tests/support/runner/groups.rs`). Members
are every operation Go's runner takes in a group, the address manager's
included, with two exceptions refused as script faults:
`mgr.change_passphrase` and `mgr.convert_to_watching_only`. dcr-purse's
`Manager::change_passphrase` and `Manager::convert_to_watching_only` take
the store and commit a transaction of their own, where Go's take the
caller's bucket (QK-0733, dispositions of 2026-11-22), and `purse-store`
runs one write transaction at a time, so they cannot share a group's. The
runner's bookkeeping (section 2, "Runner state") is saved before a group
and restored when it rolls back: block and address labels, account
passphrases and the wallet's passphrases, as Go's runner restores them.
The store's consistency checks, which the runner runs after every
operation (ADR-0018 BK4), also run after every member on the group's
transaction, before it commits or rolls back, and once more after the
group.
Since Go never tolerates a panic and the comparison accepts any error
against Go's `Panic`, a member whose operation returns an error where Go
panics may not tolerate that error's kind (a script fault): the
transaction store's stand-ins, and the address manager's `IO` on
`MarkUsed`, `MarkUsedChildIndex` and `MarkReturnedChildIndex` (QK-0039)
and `Crypto` on `PrivateKey` between a conversion and the next reopen
(QK-0763). `ImportVotingAccount` after a conversion succeeds where Go
panics (QK-0765), so no refusal of a `tolerate` list applies; the
comparison fails at that member, and its group-level effect is pinned.

The in-memory state a rollback leaves (section 3) is now compared: the
manager caches only what it reads from a committed snapshot, so after a
rolled-back group dcr-purse answers from the store, and every case found
where Go's memory answers otherwise is pinned on both sides by
`crates/purse-udb/tests/groups.rs` (QK-0733 for `SetAccountPassphrase`,
setting or removing a passphrase, and `RenameAccount`; QK-0790, an account read in the rolled-back transaction
staying cached; QK-0791, a rolled-back SLIP0044 upgrade), all under
decision item 2 above. Corpus script `15-rollback-bookkeeping` is refused
as written, for its passphrase-change and conversion members; Go's results
of it are asserted live, and the script without those four groups runs on
both sides, equal but for the one recorded divergence. The Phase 0 corpus
scripts whose groups hold address manager operations and no refused member
(`01-accounts`, `02-addresses`, `16-tolerated-errors`,
`17-resolution-errors`) run on both sides with every record and the
exports equal. The count of `unspent` inside a group (QK-0003; crate
`txstore`, choice 15) gives the same answers as under the emulation, which
counted the same rows.

Generated scripts shaped like dcrwallet's `processTransactionRecord`
(`crates/purse-udb/tests/support/ptrgen.rs`) put the address manager
operations that function runs in its transaction (`Address`, `MarkUsed`,
`AccountProperties`, `SyncAccountToAddrIndex`, `RedeemScript`) and the
imports in the transaction store's `update` groups beside `InsertMinedTx`,
`InsertMemPoolTx`, `AddCredit`, `AddTicketCommitment` and
`AddMultisigOut`, including groups that roll back after the manager
wrote. They create accounts, and convert a wallet to watching-only, only
at the top level, and never count `unspent` in a group after a write, so
they reach neither QK-0790 nor the pending count. Their campaign ran equal
against Go (PARITY.md, `purse-udb`, "Groups"), and a fixed set of 8 is
frozen (`ptr_genvectors.txt`).

Decision item 4 deferred freezing `15-rollback-bookkeeping` as a vector
until the Rust runner existed. It exists, refuses the script as written,
and the test above asserts Go's results only when the oracle runs. Whether
to freeze Go's results of the script (and of the script without its
refused groups) so that the pin also holds without Go: *pending owner*.
Recommendation: freeze them with the next regeneration of the corpus
vectors, with the refusal and the one divergent record asserted by the
replay.
