# ADR-0017 — D13: Upstream defects worth reporting

- **Status:** Proposed (decision D13). Nothing in it is decided, and
  nothing has been reported to any upstream project by this work: the
  record proposes which defects to report, how and when, and gives the
  substance of each report for the project owner to use or not.
- **Date:** 2026-11-19

## Context

**The question.** The deep dive found behaviours in dcrwallet (and in
modules it links) that look like real defects. D13 asks which are worth
reporting upstream, who reports them, when, publicly or privately, and
whether dcr-purse's own disposition changes once upstream fixes one (brief
§9 D13, which names three: a nil dereference in difficulty validation, WIF
keys echoed in a `signrawtransaction` error, outpoint locks leaked by a
partial VSP purchase).

**Decisions in force.**

- [ADR-0012](0012-standing-quirk-policy.md) (Accepted), "Relation to D13":
  a confirmed entry is a candidate when it is a Rule 6 or Rule 7
  divergence, a Rule 2 panic reachable from network or RPC input (its
  examples QK-0227 and QK-0493), or a reproduced defect that harms
  dcrwallet's users (its example QK-0279); "D13 decides who submits
  and when" (`docs/adr/0012-standing-quirk-policy.md:237-240`). Its Open
  point 6, decided as proposed: Rule 6, Rule 7 and remote-panic candidates
  go to the Decred bug bounty rather than to public issues, only after
  oracle confirmation (`:305-309`).
- [ADR-0016](0016-upstream-tracking.md) (Proposed): an upstream fix of a
  defect on this list is a trigger for a pin move (its Decision item 1(b)).

**Upstream's channels.** dcrwallet's `SECURITY.md` at the pin sends
vulnerabilities to the Decred bug bounty (bounty.decred.org) and requires
them to be reproducible on the latest production release or on master
(`SECURITY.md:3-13`, spec 22 §11). dcrd's `SECURITY.md` at dcroxide's dcrd
pin `46e9df48` has the same text. No policy file was found for vspd's
client module or for jrick/wsrpc in the module versions cached; their
channels are not established here. Ordinary defects go to each project's
public issue tracker.

**The repository is private.** `jzbz/dcr-purse` reported
`"visibility":"PRIVATE"` when read with `gh repo view` on 2026-11-19, and
its `origin/master` holds `docs/QUIRKS.md`. QUIRKS.md, the specs and the
brief describe several of the security-relevant candidates below in enough
detail to act on (QK-0227 and spec 09 Q-7 most of all). Publishing the
repository, or any of those files, is disclosure.

**What was examined.** All 737 QUIRKS.md entries; the specs' open
questions that name an upstream report (spec 09 §12 item 1, Q-7); the
brief's three examples; the Go source at `54b85b33` for every candidate in
the security-relevant class; and dcrwallet's master log since the v2.1
branch point, which shows upstream fixing defects of the same kinds (for
example "spv: Validate merkle root for all fetched blocks", `2c37f240`;
"jsonrpc: Avoid nil pointer dereference", `43b095a5`; "mixing: add missing
bounds check", `520c9890`; ADR-0016, Context).

**What was run for this record.** Five candidates were confirmed by
running Go code at the pin: a copy of the dcrwallet module at `54b85b33`,
in the scratch directory of this work, with one test file added to its
`wallet` package (`go test ./wallet/ -run TestD13`, go1.27.4, three runs,
the same result each time). The copy is not in the repository and nothing
else in it was changed.

| Entry | What the test does | Result |
|---|---|---|
| QK-0227 | `checkDifficultyPositional` with mainnet parameters, a cached blake3 candidate anchor at height 794,367, a parent at height 700,000 and a version-10 header at 700,001 whose `Bits` are `0x207fffff`, its nonce stepped until its blake3 hash meets that target (3 tries) | panics: `runtime error: invalid memory address or nil pointer dereference` |
| QK-0201 | `SignTransaction` with a previous script paying address A and a supplied key map holding only a WIF for address B | the signature error's text contains B's WIF |
| QK-0558 | on a test wallet: `Unlock(pass, lockChan)` as `RunTicketBuyer`'s handler does, `Unlock(pass, nil)` as `ticketbuyer.Run` does, a pause standing for the ticket buyer's run, then the handler's deferred send on `lockChan`; control: the first unlock and the send only | control: locked; handler sequence: unlocked, with no time limit |
| QK-0160 | an account given its own passphrase; the wallet locked; `UnlockAccount`; then `Lock` (`walletlock`) | the account stays unlocked |
| QK-0033 | `CreateWatchOnly` given an extended private key (`sprv…`), then `Open` and `AccountXpub(0)` | accepted; the wallet is watching-only and its account 0 "xpub" is the private key |

On QK-0558: without the pause, the outcome depends on which ready case
Go's `select` picks (`wallet/wallet.go:1823-1829`), and the wallet locked
in one run of that variant. In the handler the ticket buyer runs between
the second unlock and the deferred send, so the timer goroutine has taken
the cancel case long before the send arrives.

Confirmed earlier, by other work: QK-0086 and QK-0733 by the oracle's
operation-script runner; QK-0729's error text (not its log line) by
`purse-seed`'s differential (QK-0632); QK-0734 by the store spike's
cross-check against dcrwallet's own `udb` (ADR-0003, Decision item 5);
QK-0737's transport buffering, measured with a grpc-go client
(ADR-0015). Every other
candidate is read from the source only.

**Terms used below.**

- *Security-relevant*: any one of
  (a) a party the operator does not control or has not authenticated (a
  network peer, another mixer, a VSP, a web page, a local process without
  credentials) can crash the wallet, control it, learn a secret, keep
  funds from being found, or deanonymise the user;
  (b) a protection the operator turned on (the wallet lock,
  authentication, a proxy) does not hold;
  (c) a Rule 6 or Rule 7 divergence of ADR-0012: a secret moves to a sink
  its supplier does not hold (the log, a client tool's log through an
  error text the owner ruled redacted, the network), or funds are lost as
  W1 defines it. ADR-0012's "Relation to D13" makes these candidates and
  its Open point 6 sends them to the bug bounty whoever triggers them, so
  they belong here even when only an authenticated client or the
  operator's own log level starts them (S6, S7, S17);
  (d) a participant in a mixing session can deny mixing to honest peers,
  by getting them blamed or excluded (S10, S12). The mixing protocol
  exists so that unauthenticated participants cannot do this; it is
  denial of service against a privacy feature, not against the wallet.
  Outside (c), a defect only an authenticated RPC client can trigger is
  not security-relevant, since that client can already stop the wallet;
  nor is input from the operator's own dcrd in RPC mode, which the wallet
  trusts by design.
- *Severity*: **high**, an unauthenticated party crashes or controls the
  wallet or takes a secret; **medium**, a secret reaches a sink by
  default, a protection the user turned on fails in a common flow, funds
  or votes are silently lost or not found, or a user is deanonymised;
  **low**, needs an uncommon configuration or only degrades behaviour.
- *Run*: confirmed by running Go code at the pin. *Read*: established from
  the source only.

## Candidates: security-relevant

The list is complete for this class as far as the entries and the source
were read. "Project" is where the defect lives. "dcr-purse" is the
disposition proposed for dcr-purse's own code (Decision item 6).

| # | Entry | Project | Severity | Confirmed | Defect | dcr-purse |
|---|---|---|---|---|---|---|
| S1 | QK-0227 | dcrwallet | high | run (the panic), read (reachability) | a peer serving an SPV wallet's start-up header sync crashes it with one crafted header | Rule 2: "not an ancestor" |
| S2 | QK-0436, QK-0738 | dcrwallet | high in that configuration | read | `--noservertls` with `--jsonrpcauthtype=clientcert` authenticates nobody, and any web page can reach the websocket | refuse the combination; check `Origin` |
| S3 | QK-0363, QK-0566 | dcrwallet | medium | read | gRPC `SpvSync` and `RpcSync` dial directly, ignoring `--proxy` | honour the proxy |
| S4 | QK-0558 | dcrwallet | medium | run | `RunTicketBuyer` and `RunAccountMixer` leave the wallet unlocked without limit | lock when the stream ends |
| S5 | QK-0033 | dcrwallet | medium | run | a watching-only wallet created from an extended private key keeps it, sealed by the public passphrase, and returns it as the account xpub | refuse private keys |
| S6 | QK-0201 | dcrwallet | medium | run | `signrawtransaction` errors quote every WIF the caller supplied | Rule 6: redact (decided, ADR-0012 Open point 1) |
| S7 | QK-0729 | dcrwallet | medium | run (the text), read (the log line) | seed words reach the log at the default level through `DecodeSeed` errors | Rule 6: redact the log (decided) |
| S8 | QK-0739, QK-0448, QK-0434 | dcrwallet | high | read | an unauthenticated websocket client can exhaust memory and every websocket slot | limits and an authentication deadline |
| S9 | QK-0242 | dcrwallet | medium | read | SPV accepts any committed filters below the DCP0005 height, so a peer can hide old history during a restore | validate them after sync |
| S10 | QK-0314 | dcrd (`mixing`) | medium | read | after a re-formation a peer's second encapsulation is blamed, so a participant who forces a re-formation can get honest peers excluded | reproduce (W3) or Rule 11 |
| S11 | QK-0317 | dcrd (`mixing`) | medium | read | a request whose secrets were revealed can return to the next epoch with the same revealed data | Rule 11: diverge (spec 12) |
| S12 | QK-0319 | dcrd (`wire`, `mixing`) | low to medium | read | sessions over 512 messages pass size limits but cannot encode their slot reservations, and every peer is blamed | reproduce (W3) |
| S13 | QK-0160 | dcrwallet | low | run | `walletlock` leaves an account unlocked by `unlockaccount` while the wallet was locked | Rule 11: lock all |
| S14 | QK-0565 | dcrwallet | low | read | gRPC sync with `discover_accounts` keeps the wallet unlocked for the whole stream when no rescan runs | Rule 11: lock |
| S15 | QK-0381 | dcrwallet | low | read | `--spvconnect` host names are resolved by the system resolver even with `--proxy` | resolve through the proxy |
| S16 | QK-0409 | dcrwallet | low | read | an unreadable dcrd CA file falls back to the system trust store and still sends credentials | refuse to connect |
| S17 | QK-0290 | vspd (client), dcrwallet | low | read | trace-level request dumps carry the ticket's voting WIF | Rule 6: redact |
| S18 | QK-0301 | vspd (client) | low | read | a VSP-signed reply with body `null` crashes the wallet | Rule 2: error |

### Substance of each report

Written for the owner to adapt. Line numbers are at `54b85b33` unless
another module is named.

**S1 — A peer serving an SPV wallet's start-up header sync can crash it
with one crafted header (QK-0227; spec 09 Q-7).**
- *Where:* `isAncestorOf` calls `BlockHash()` on the result of
  `ancestorHeaderAtHeight`, which returns `(nil, nil)` when the requested
  height is above the target's (`wallet/difficulty.go:675-680`,
  `:700-709`). `checkDifficultyPositional` calls it with the cached blake3
  candidate anchor whenever a header is version ≥ 10 (≥ 11 off mainnet),
  at height ≥ stake validation height plus two rule-change intervals
  (20,224 on mainnet), and its blake3 hash meets the target its own `Bits`
  claim, which nothing range-checks (`:423-441`, `:482-494`). The header's
  height cannot be faked: `validateHeaderChainDifficulties` requires it to
  be its parent's plus one before that call (`:872`, `:879`). The panic
  therefore needs a header whose parent lies below the cached anchor
  (794,367 on mainnet in the test above).
- *Trigger, by the start-up sync:* during `initialSyncHeaders`
  (`spv/sync.go:1616`, `:1680`), `getHeaders` asks only a peer that
  advertised at handshake a height above the wallet's main-chain tip and
  has not yet sent headers up to it (`pickForGetHeaders`,
  `spv/backend.go:39-46`, `:286`). The reply's first header must have one
  of the request's locators as its parent (`p2p/peering.go:1125-1144`);
  the locators reach back from the best known header in doubling steps, so
  one of them lies below the anchor on mainnet. The reply must also pass
  "too few headers": a full batch of 2,000 headers, or a last header at or
  above the advertised height (`p2p/peering.go:1112-1121`). For a wallet
  whose tip is past the anchor the second is impossible (the forged chain
  starts below the anchor and the advertised height is above the tip), so
  the reply is a full batch. Only its first header is crafted: the p2p
  layer checks only that hashes and heights run in sequence
  (`:1146-1160`), and validation panics on the first header before reading
  the others. The anchor must already be cached in the same process run,
  which happens when an earlier batch (from this peer, relaying real
  headers, or from any other) contained a valid header past the blake3
  activation (`wallet/difficulty.go:505-513`). A wallet that starts behind
  the network, which is the usual case, fetches such a batch first. While a
  restoring wallet's main-chain tip is still below the anchor but headers
  fetched ahead have cached it, the locators come from the batch's likely
  best chain and a shorter reply that reaches the advertised height also
  passes. Solving blake3 at `Bits = 0x207fffff` took three nonces.
- *Not by announcement, in practice:* before the initial sync is done, a
  peer that sends unrequested headers is disconnected
  (`p2p/peering.go:1100-1106`) and block `inv`s are ignored
  (`spv/sync.go:1066`); `sendheaders` goes out only after
  `initialSyncDone` (`spv/sync.go:1912-1955`), and after it a synchronous
  `getheaders` is refused (`p2p/peering.go:2001-2004`), so every later
  headers message, including replies to the asynchronous requests, goes
  through `handleBlockAnnouncements` (`p2p/peering.go:1173-1180`;
  `spv/sync.go:1266-1272`). That function rejects a batch whose first
  header is more than 256 blocks below the main-chain tip
  (`maxAnnHeaderTipDelta`, `spv/sync.go:1398-1405`) before it looks up the
  parent, and a parent below the real anchor is far more than 256 blocks
  below any synced mainnet or testnet3 tip. Reaching the panic from a
  synced wallet would need an anchor cached within 256 blocks of the tip,
  which means a header whose `Bits` match a recent interval-boundary
  candidate and whose blake3 hash meets them: roughly the work of one real
  block. This was read, not run.
- *Effect:* `ValidateHeaderChainDifficulties` runs on the SPV syncer's
  initial-sync goroutine (`spv/backend.go:353`), not on the p2p message
  goroutine, which only forwards the message on a channel
  (`p2p/peering.go:1172-1189`). Nothing on that goroutine recovers
  (`dcrwallet.go:180` is the only `recover` outside tests and covers the
  main goroutine), so the process exits. Affects SPV wallets on mainnet and
  testnet3 at each start while they catch up, if the attacker is among the
  peers asked; RPC-mode wallets take headers from the operator's dcrd.
- *Fix:* return false when the ancestor is nil, as dcrd's `IsAncestorOf`
  does; optionally reject blake3 claims whose target exceeds the network's
  limit.
- *Reproducer:* the scratch test above, as a test in upstream's `wallet`
  package. Before filing, the path through the network is to be run too: a
  fake peer that advertises a height above the tip and serves the start-up
  sync of a lagging SPV wallet past activation, on testnet3 or on a simnet
  with the agenda not forced, answering the first `getheaders` with real
  headers and the next with the crafted batch; on current master and on
  the latest release.
- *Severity:* high. The window is the start-up sync, not the whole
  running time, but an unauthenticated peer ends the process with
  negligible work at most starts of a wallet that is behind, and a crashed
  wallet that restarts goes through the same window again.

**S2 — `--noservertls` with `--jsonrpcauthtype=clientcert` disables JSON-RPC
authentication, and the websocket accepts any origin (QK-0436, QK-0738).**
- *Where:* with TLS off the client-CA branch is skipped, user and password
  are withheld because the auth type is not basic, so `authsha` stays nil
  (`rpcserver.go:350-373`; `internal/rpc/jsonrpc/server.go:124-127`) and
  `checkAuthHeader` accepts every request (`server.go:256-276`). The
  websocket upgrader's `CheckOrigin` returns true (`server.go:116-118`).
- *Effect:* any process on the host, and any web page open in a browser on
  it, can open `ws://127.0.0.1:<port>/ws` and call every method, sending
  funds when the wallet is unlocked. Start-up accepts the combination
  without a warning. With `--noservertls` and basic authentication the
  missing origin check still lets a page try passwords from the browser;
  with TLS on, the browser refuses the self-signed certificate.
- *Fix:* refuse `clientcert` without TLS at configuration time; check the
  `Origin` header (no origin, or a loopback origin) on the websocket.

**S3 — gRPC-started syncers bypass `--proxy` (QK-0363, QK-0566).**
- *Where:* `SpvSync` builds its `LocalPeer` without `SetDialFunc` or
  `SetDisableRelayTx` (`internal/rpc/rpcserver/server.go:3014-3016`),
  where the daemon's own loop sets both (`dcrwallet.go:536-538`); `RpcSync`
  leaves `RPCOptions.Dial` nil (`server.go:2865-2872`).
- *Effect:* a user who configured a proxy (for example Tor) and syncs
  through gRPC, as Decrediton does, connects to peers, HTTPS seeders and
  dcrd directly from their own address. Whether Decrediton passes
  `--proxy` to the wallet was not checked.
- *Fix:* pass the configured dialer, relay setting and persistent peers to
  the gRPC-started syncers.

**S4 — Stopping the ticket buyer or account mixer leaves the wallet
unlocked (QK-0558).**
- *Where:* the handler unlocks with a timeout channel and defers a send on
  it (`internal/rpc/rpcserver/server.go:2662-2674`; the mixer `:2542-2557`);
  `ticketbuyer.Run` unlocks again with a nil timeout
  (`ticketbuyer/tb.go:63-69`), which cancels the first timeout
  (`wallet/wallet.go:1804-1832`); the deferred send is then consumed by the
  cancelled timer goroutine, which does not lock.
- *Effect:* after the stream ends the wallet is unlocked with no time
  limit, so any JSON-RPC client with credentials but without the
  passphrase can spend.
- *Fix:* do not pass the passphrase to `Run` when the handler has
  unlocked, or lock explicitly when the handler returns.
- *Reproducer:* the scratch test above.

**S5 — A watching-only wallet created from an extended private key keeps
it and serves it as the xpub (QK-0033).**
- *Where:* `createWatchOnly` parses the string with `NewKeyFromString`,
  which accepts private keys, and stores its `String()` under crypto keys
  sealed by the public passphrase (`wallet/udb/addressmanager.go:3090-3175`);
  neither `--createwatchingonly` (`walletsetup.go:356`) nor gRPC
  `CreateWatchingOnlyWallet` (`internal/loader/loader.go:163`) checks
  `IsPrivate`, unlike `ImportXpubAccount` (`wallet/wallet.go:3895-3896`).
- *Effect:* a user who pastes the wrong key gets a "watching-only" wallet
  whose file holds the private key under the public passphrase (by
  default the constant `public`), and which hands the private key to any
  client asking for the account's extended public key.
- *Fix:* reject a private key in `createWatchOnly`.

**S6 — `signrawtransaction` errors quote the caller's WIF keys (QK-0201).**
- *Where:* `errors.Errorf("no key for address (needed: %v, have %v)",
  addr, additionalKeysByAddress)` formats the whole map of `*WIF`, whose
  `String()` is the key (`wallet/wallet.go:4816-4823`); the text reaches
  `errors[].error` (`internal/rpc/jsonrpc/methods.go:5094-5113`).
- *Effect:* RPC client tools that log errors write private keys to their
  logs. Only the caller's own keys are exposed.
- *Fix:* name the missing address only.

**S7 — Seed words in the wallet log (QK-0729).**
- *Where:* `DecodeMnemonics` quotes the word it rejects, and the
  wrong-position error always quotes a valid seed word with its position
  (`pgpwordlist/pgpwordlist.go:47-55`); gRPC `DecodeSeed` wraps that text
  (`internal/rpc/rpcserver/server.go:3199-3204`) and the unary interceptor
  logs every failed call at error level (`rpcserver.go:444`).
- *Effect:* a mistyped restore in a client writes seed words to the log
  file at the default level.
- *Fix:* do not log `DecodeSeed` failures' text, or do not quote words.

**S8 — Unauthenticated websocket clients can exhaust the JSON-RPC server
(QK-0739, QK-0448, QK-0434).**
- *Where:* the upgrade happens before authentication when no
  `Authorization` header is sent (`internal/rpc/jsonrpc/server.go:146-171`);
  the read deadline is then cleared (`:510-514`) and messages are read with
  gorilla's `ReadMessage` with no read limit (`:338-350`;
  `gorilla/websocket@v1.5.1/conn.go:279`, `:941`, `:1118-1126`), while POST
  bodies are capped at 4 MiB (`server.go:529-537`). The concurrency
  counter is taken before authentication (`:287-302`); 25 websocket slots
  by default (`config.go:49`). After a failed authentication the socket is
  never closed (QK-0434).
- *Effect:* anyone who can reach the listener can send one large frame to
  exhaust memory, or hold every websocket slot with idle connections that
  never authenticate. The default listeners are loopback; an exposed
  listener is reachable by anyone on the network.
- *Fix:* `SetReadLimit` (the POST cap), a deadline for the first message
  of an unauthenticated connection, closing the socket on every
  disconnect path.
- *Severity:* high, by the definition above: a frame large enough ends
  the process (the Go runtime's fatal out-of-memory error, or the
  kernel's OOM killer), which is a crash, and clause (a) counts a local
  process without credentials as an outside party, so the default
  loopback listeners do not lower the rating (S2 is rated the same way).
  What sets it apart from S1 is the attacker's cost and reach, not the
  rating: the sender transmits about as many bytes as the memory it
  exhausts (`io.ReadAll` grows its buffer to between 1.25 and 2 times
  what it has received, so somewhat fewer), which on loopback is a few
  seconds per gigabyte, and in the default configuration only a process
  on the same host reaches the listener. Not run.

**S9 — Committed filters below DCP0005 are not checked during SPV sync
(QK-0242).**
- *Where:* `CFilterV2HeaderCommitment` accepts every filter below height
  431,488 on mainnet and 323,328 on testnet3, leaving them to "a different
  function" (`validate/validate.go:19-25`, `:69-89`); that function,
  `ValidatePreDCP0005CFilters` (`wallet/wallet.go:5225-5266`), has one
  caller, the handler of the JSON-RPC method `validatepredcp0005cf`
  (`internal/rpc/jsonrpc/methods.go:180`, call at `:5398`).
- *Effect:* during the initial sync of a restored wallet a peer can serve
  filters that omit the wallet's scripts, and the wallet then does not
  find its transactions below that height. No funds move; the balance
  looks complete.
- *Fix:* run the pre-DCP0005 hash check automatically once the filters
  through that height are stored, and refetch from another peer on
  mismatch.

**S10 — Re-formed mixing sessions blame honest peers (QK-0314; dcrd
`mixing` v0.7.5).**
- *Where:* `cloneLocalPeer(false)` keeps the PRNG, so a second
  encapsulation reads further down the stream, while blame recomputes from
  the position after key generation (`mixclient/client.go:260-277`,
  `:1759`; `mixclient/blame.go:237-296`).
- *Effect:* derived from code and untested upstream: after a timeout
  re-formation, honest peers' ciphertexts fail blame's recomputation, so a
  participant who withholds a message to force the re-formation can get
  honest peers excluded.
- *Fix:* restart the stream per encapsulation pass, or record the position
  blame must start from.

**S11 — Revealed secrets can return to the next epoch (QK-0317; dcrd
`mixing` v0.7.5).**
- *Where:* keys are regenerated on exit only if the last iteration
  revealed secrets (`mixclient/client.go:1172-1187`, `:1275`).
- *Effect:* derived from code: after reveal, re-form and a non-reveal
  failure, the request goes back to pending with its published seed, slot
  reservation messages and HASH160s, so the next session reuses data
  already disclosed in blame.
- *Fix:* a sticky "revealed" flag that forces fresh keys and addresses.

**S12 — Slot reservations cannot encode sessions over 512 messages
(QK-0319; dcrd `wire` v1.8.0 and `mixing` v0.7.5).**
- *Where:* the message's encoder and decoder cap the inner dimension at
  `wire.MaxMixPeers` (512) though it is the session's message total, which
  the mixing limits allow up to 1,024 (`wire@v1.8.0/msgmixslotreserve.go:81-85`,
  `:200-203`; `mixing@v0.7.5/limits.go:32-34`).
- *Effect:* derived from code: such a session passes size limiting, every
  slot reservation then fails to broadcast and all peers are blamed for a
  timeout; participants with large message counts can provoke it.
- *Fix:* make the two limits agree.

**S13 — `walletlock` does not lock individually unlocked accounts
(QK-0160).** `Manager.Lock` returns `Locked` before clearing keys when the
manager is already locked, and `Wallet.Lock` drops the error
(`wallet/udb/addressmanager.go:1453-1469`; `wallet/wallet.go:1842-1849`),
so an account unlocked with `unlockaccount` while the wallet was locked
survives `walletlock`, though the same call clears it when the wallet was
unlocked. *Fix:* clear account keys whether or not the manager was locked.
*Reproducer:* the scratch test above.

**S14 — gRPC sync with account discovery can leave the wallet unlocked
(QK-0565).** The handlers lock in `DiscoverAddressesFinished` or when the
stream returns (`internal/rpc/rpcserver/server.go:2850-2858`, `:2935-2947`,
`:3001-3008`, `:3099-3107`); QK-0565 reads that the callback does not fire
when no rescan is needed, so the wallet stays unlocked while the stream
lives. *Fix:* lock once discovery ends, whether or not a rescan follows.

**S15 — `--spvconnect` names leak to DNS with `--proxy` (QK-0381).**
`ConnectOutbound` calls `net.ResolveTCPAddr` before dialling
(`p2p/peering.go:260`), then dials the original name through the proxy.
*Fix:* skip local resolution when a proxy is set.

**S16 — Unreadable dcrd CA file widens TLS trust (QK-0409).** A failed
read is logged and the connection proceeds with nil CA bytes
(`dcrwallet.go:602-618`), and the TLS configuration with its client
certificate is applied only when CA bytes exist (`chain/sync.go:555-578`),
so `wsrpc` dials with the system trust store and still sends Basic
credentials. *Fix:* fail the connection when TLS is on and the CA cannot
be read.

**S17 — Voting key in trace logs (QK-0290; vspd `client/v4` v4.0.2).** At
trace level the client dumps full requests, including the voting WIF in
`payfee` (`client.go:189-211`), and dcrwallet hands it its logger
(`wallet/vsp.go:76-84`). The VSP holds that key by design; the log file
does not need it. *Fix:* redact `votingkey` in dumps.

**S18 — A VSP's `null` reply crashes the wallet (QK-0301; vspd
`client/v4` v4.0.2).** A signed 200 response with body `null` leaves the
response pointer nil and the client dereferences it (`client.go:59`,
`:93`, `:115`, `:149`; `wallet/vsp.go:113`). Needs the VSP's signing key.
*Fix:* treat a nil response as an error.

## Candidates: other defects worth reporting

Selected from the rest of QUIRKS.md: defects that cost dcrwallet's users
funds, votes, privacy or a working feature, or that are one-line fixes
worth a short issue. One report per root cause; entries that describe the
same defect are grouped. All are public issues, all read only unless
noted.

| Entry | Project | Severity | Substance | Fix |
|---|---|---|---|---|
| QK-0204 | dcrwallet | medium | `SendOutputs` without a network backend records the transaction and returns `NoPeers`; it is broadcast later although the call failed, so a retry pays twice (`wallet/wallet.go:4592-4603`) | abandon on failure, or check the backend first |
| QK-0340, QK-0015, QK-0216, QK-0560, QK-0609 | dcrwallet | medium | `Loader.CreateNewWallet` omits `MixSplitLimit`, so a wallet created through gRPC (Decrediton) or `--create` cannot mix outputs until restarted (`internal/loader/loader.go:259-274`) | pass the field |
| QK-0247, QK-0554 | dcrwallet | medium | gRPC `CreateWallet` tests `set_birth_time` twice; a birth height alone becomes height 0 (`internal/rpc/rpcserver/server.go:2713-2752`) | test `set_birth_height` |
| QK-0229 | dcrwallet | medium | SPV account discovery probes a fixed halving sequence instead of bisecting, so used accounts are missed (spec 09 Q-11) | bisect as the RPC variant does |
| QK-0230 | dcrwallet | medium | `cacheMissingCommitments` returns at the first fully cached chunk, so later blocks look empty to discovery (`wallet/discovery.go:82-84`) | continue |
| QK-0138, QK-0248 | dcrwallet | medium | `--watchlast` wraps on never-returned branches and watches none of their addresses (`wallet/wallet.go:931-932`, `:1003-1014`) | guard the subtraction |
| QK-0253, QK-0352 | dcrwallet | medium | the last-returned index is not persisted on address issue, so addresses are reissued after a restart (`wallet/addresses.go:468-502`; dropped by `bea91d63`) | persist it |
| QK-0287, QK-0308 | dcrwallet | medium | votes append treasury votes with no cap of 7, so with more tspends in window the vote is invalid and the ticket misses (`wallet/chainntfns.go:804-824`) | cap at the consensus limit |
| QK-0309 | dcrwallet | medium | `payfee` and `setvotechoices` send more policy entries than vspd's `max=3` binding accepts (`wallet/feepayment.go:520-528`) | send at most what vspd accepts, or agree a limit with vspd |
| QK-0282, QK-0283 | dcrwallet | medium | a fee job whose `makeFeeTx` fails is never rescheduled (fee unpaid, ticket not voted by the VSP); after `ErrFeeExpired` with `fee = -1` a nil interface is called on a timer goroutine (`wallet/feepayment.go:412-420`, `:160-168`) | reschedule; clear stale fee state |
| QK-0279 | dcrwallet | medium | after the first ticket of a VSP purchase is published `unlockCredits` is cleared, so when a later ticket fails to publish or the purchase is cancelled, the fee credits reserved for the unpublished tickets stay locked in memory until a restart or an explicit unlock (`wallet/createtx.go:1304-1314`, `:1633`); named by ADR-0012's "Relation to D13" as its example of a candidate (Open point 8) | release the reserved credits of the tickets not published, as dcr-purse does (owner's decision, ADR-0012 Open point 3) |
| QK-0303 | dcrwallet | medium | VSP fee transactions can pay below the relay fee (`wallet/wallet.go:4693-4731`) | require input ≥ fee + mining fee |
| QK-0280 | dcrwallet | low | per-ticket treasury key policies are sent with raw-byte keys, malforming `payfee` (`wallet/vspticket.go:213-234`) | hex-encode as the RPC paths do |
| QK-0291, QK-0542 | dcrwallet | low | vote-choice RPCs report success when the VSP was not updated (`wallet/vsp.go:297-304`) | return the failure |
| QK-0734, QK-0081, QK-0082, QK-0735, QK-0736 | dcrwallet | medium | stake invalidation and rollback leave rows inconsistent: an outpoint mined and unmined at once is reported and counted twice (QK-0734, confirmed by the store spike's cross-check), invalidated rows are orphaned, a dangling unspent entry makes balance scans fail | report QK-0734 with its sequence; the others after an oracle run |
| QK-0231 | dcrwallet | medium | in RPC mode a reorg back onto a block without wallet transactions stalls the processed marker until a rescan (`wallet/chainntfns.go:160-166`) | treat a re-attached node as scanned |
| QK-0447 | dcrwallet | medium | `ReadTimeout` of 10 s cancels POST handlers that run longer, such as `purchaseticket` (`internal/rpc/jsonrpc/server.go:95-110`); from net/http's source, not run | set no read timeout past the body |
| QK-0178, QK-0220 | dcrwallet | medium | lock-order inversions and nested read transactions that can deadlock against a growing bbolt writer (`wallet/wallet.go:3379-3382`, `:5741-5799`) | one order; no nested views |
| QK-0522, QK-0556, QK-0557, QK-0559 | dcrwallet | low | after gRPC `CloseWallet` the services stay bound to the closed wallet and nothing running is stopped (`internal/loader/loader.go:375-393`) | rebind or refuse; stop the wallet's tasks |
| QK-0263, QK-0492, QK-0493, QK-0460, QK-0202, QK-0466 | dcrwallet | low | panics reachable from authenticated RPC arguments end the process over websocket and gRPC (`spendoutputs` with no change, out-of-range output indexes, `auditreuse` on an addressless credit), and `syncstatus` can be unanswerable | bounds checks; nil checks |
| QK-0302, QK-0143, QK-0407, QK-0431 | dcrwallet | low | malformed replies from dcrd crash the wallet (short tspend signature script, missing fields, null winning-ticket hash) | validate dcrd's replies |
| QK-0737 | dcrwallet | low | one gRPC client that stops reading stalls the wallet (ADR-0015) | bounded queues |
| QK-0458, QK-0197 | dcrwallet | low | `getreceivedbyaccount` indexes results by iteration position, returning another account's total with 256 or more accounts or an imported xpub account | look up by account number |
| QK-0531 | dcrwallet | low | gRPC imports reject a non-zero rescan height exactly when a rescan is asked for (`internal/rpc/rpcserver/server.go:684-687`) | invert the test |
| QK-0524, QK-0570, QK-0497, QK-0528, QK-0555, QK-0457 | dcrwallet | low | one-line slips: empty `case` arms map seven kinds to `Unknown`; `errors.Is` with swapped arguments; an inverted test that never sets `commitment_amount`; `addmultisigaddress` returns `{}` | as each entry says |
| QK-0034, QK-0630 | dcrwallet | low | Argon2 parallelism wraps to 0 on hosts with 256 or more CPUs and the KDF panics (`kdf/kdf.go:30-39`) | clamp to 255 |
| QK-0342, QK-0366, QK-0365 | dcrwallet | low | SPV disconnects peers for a `notfound` of a mix message and for any unknown command (`p2p/peering.go:1194-1217`, `:794-853`) | track mix requests; skip unknown commands |
| QK-0636, QK-0635 | dcrwallet | low | `cmd/sweepaccount` and `cmd/movefunds` cannot work at the pin | fix or remove |
| QK-0415 | jrick/wsrpc v2.3.8 | low | `Call` returns the context error even after a reply arrived, so an accepted `sendrawtransaction` can be reported as cancelled (`rpc.go:434-438`) | return the reply when one arrived |
| QK-0023 | etcd-io/bbolt | low | `DeleteBucket` can skip every second nested bucket of a materialised leaf and leak their pages; read in v1.3.12 and unchanged in v1.5.0's `DeleteBucket` | collect names before deleting; needs a reproduction first |

## Examined and not proposed

- **QK-0228** (positional difficulty; PoW hash taken from the next
  header): the p2p layer hashes every header and requires each to connect
  to the previous by hash (`p2p/peering.go:1146-1160`), so the hash used is
  the header's. Not a defect a peer can use.
- **QK-0428** (commitment proof index not pinned): forging a filter would
  need a hash that matches the stake root through some index, a preimage
  problem. Hardening at most.
- **QK-0485** (SPV `getblock` reads `Transactions[0]`): blocks are checked
  against their header's merkle roots, so an empty regular tree needs a
  header with work behind it. Not proposed.
- **QK-0690** (the wallet's mixpool validates no UTXOs): an SPV wallet has
  no UTXO set; this is the design, not a slip.
- **QK-0041** (voting account sealed under a zeroed key when locked): the
  row is rewritten with the Argon2id ciphertext in the same transaction
  (`wallet/udb/addressmanager.go:2191-2232`).
- **QK-0032** (`ConvertToWatchingOnly` leaves sealed xprivs): only tests
  call it.
- **QK-0142, QK-0252** (a backend returning more results than asked
  panics): the backend is the wallet's own syncer; whether a peer controls
  the length was not traced. Robustness, not security.
- **QK-0022, QK-0730** (bbolt panics on corrupt files): local files;
  dcr-purse links no bbolt; upstream's behaviour of long standing.
- **grpc-go:** no defect found. The buffering behind QK-0737 is grpc-go's
  design.
- **go-flags v1.5.0, logrotate v1.0.0:** their entries describe old
  releases' behaviour dcrwallet pins, not defects to report.
- **Documentation drift** (QK-0523, QK-0527, QK-0547, QK-0572, QK-0585,
  QK-0451, QK-0616): one public documentation issue at most, at the owner's
  choice.

## Options considered

**Who files.** (A) The project owner, under the owner's own accounts, from drafts
engineers prepare. (B) Engineers or agents file directly: rejected, since
a report, an issue or a comment publishes under the owner's name and
commits the owner to a disclosure the owner has not read. (C) Nobody: dcr-purse keeps
its dispositions and upstream is not told; dcrwallet's users keep the
defects.

**When.** (1) Now, from reading: rejected for the security class, whose
channel requires reproduction on the latest release or master. (2) After a
run on upstream's current master and latest release, per candidate. (3)
When the porting phase confirms each entry with the oracle (ADR-0012
Rule 12): years away for some, too late for the security class, natural
for the rest.

**How.** Privately through the bounty, or by public issue, or pull request
with a fix. What ADR-0012 Open point 6 decided is narrower than this
record's security class: Rule 6, Rule 7 and remote-panic candidates go to
the bounty, "only after oracle confirmation" (`docs/adr/0012-standing-quirk-policy.md:305-309`).
On this list that covers S1 and S18 (remote panics) and S6, S7 and S17
(Rule 6). The rest of the S-list (S2 to S5, S8 to S16) is not a Rule 6,
Rule 7 or panic case, so the decided route does not reach it; sending it
to the bounty too is a proposal of this record (Decision item 2, Open
point 3). Public issues or pull requests for everything else.

**dcr-purse after an upstream fix.** Follow the pin (keep the disposition
until a pin move carries the fix, then follow it), or diverge early to the
fixed behaviour before the pin carries it.

## Decision (proposed)

1. **The owner files every report**, from the drafts above or later ones.
   No engineer or agent files, comments on or otherwise contacts an
   upstream project about these defects.
2. **Security-relevant candidates go privately** to the Decred bug bounty,
   for dcrwallet, dcrd and vspd, one report per root cause, each only
   after it has been reproduced by running code on dcrwallet's current
   master and latest release (`SECURITY.md:13`), with a reproducer written
   as a test in upstream's own package. Reproducers are kept out of this
   repository until upstream has released a fix. Order: S1 first, then by
   severity. This item amends ADR-0012 Open point 6 in two ways, each for
   the owner to accept (Open point 3): it extends the bounty route from
   the Rule 6, Rule 7 and remote-panic candidates (S1, S6, S7, S17, S18)
   to the whole S-list; and for the security class it replaces "only
   after oracle confirmation" with the run on master and the latest
   release, because an oracle confirmation is a differential checked into
   this repository's oracle rig at the pin, which both discloses the
   defect in the repository and does not show what the bounty requires
   (reproduction on master or the latest release).
3. **Other candidates go to public issues**, one per root cause, when the
   entry is confirmed by an oracle run in the phase that ports its code.
   Earlier, after a targeted run, for those whose cost to users is
   clearest or whose fix is one line: QK-0204, the `MixSplitLimit` group,
   QK-0247, QK-0457, QK-0524, QK-0497 and QK-0531. A pull request with a
   fix only where the owner chooses.
4. **Disclosure.** The repository stays private, or the passages that
   describe security-relevant candidates (the QUIRKS entries of the
   S-list, spec 09 Q-7, brief §9 D13's examples, this record) stay out of
   anything published, until each candidate is fixed in an upstream
   release or the bounty program agrees to disclosure. The passages are in
   the repository's history, so publishing the history publishes them.
5. **Tracking.** A reported entry gets a dated "Upstream:" line in
   QUIRKS.md: the channel and date (for a private report nothing more),
   later the fixing commit and release. Changes to the list are addenda to
   this record. The list is re-checked at every pin move (ADR-0016 item
   5.10).
6. **dcr-purse's own disposition.**
   (a) Until a pin move carries an upstream fix, the ADR-0012 disposition
   stands.
   (b) The move that carries a fix makes the fixed behaviour the
   specification: the code follows the new pin, a PARITY.md divergence the
   fix makes unnecessary is retired, and where upstream's fix differs from
   dcr-purse's divergence in what a client sees, dcr-purse follows
   upstream's at that move.
   (c) After v2.2.0 a fix on master that no release carries is not
   followed (ADR-0016 item 2), except that a security-relevant defect
   dcr-purse already does not reproduce stays unreproduced.
   (d) The security-relevant candidates no standing rule covers (S2 to S5,
   S8, S9, S11, S13 to S16) are put to the owner under ADR-0012 Rule 11,
   with the "dcr-purse" column above as the recommendation. S1, S6, S7,
   S17 and S18 are already decided by Rules 2 and 6; S10 and S12 concern
   mixing, where W3 requires the wallet to behave as Go peers do.
7. **Nothing is reported by this record or by its acceptance.** Each
   filing is a separate act of the owner's.

## Consequences

- **Work created:** a run of each security-relevant candidate on current
  master and the latest release before filing, a reproducer each, and the
  network-level reproduction of S1; the "Upstream:" lines; QK-0738 and
  QK-0739, recorded with this record; addenda as reports progress.
- **The repository's publication waits** on the security list, unless the
  passages are kept out of what is published.
- **Divergences to decide** for S2 to S5, S8, S9, S11, S13 to S16 under
  Rule 11, each a PARITY.md line when built (Phases 3, 5, 7, 8, 10, 11,
  12).
- **Specs and QUIRKS gain** the confirmations of this record (QK-0227,
  QK-0201, QK-0558, QK-0160, QK-0033) and the answer to spec 09 Q-7 from
  reading: an SPV peer can deliver such a header in reply to `getheaders`
  during the start-up header sync, if it advertised a height above the
  wallet's tip and the anchor is already cached in that run; an
  announcement cannot deliver it to a synced wallet without about one
  real block's work (S1).
- **Risks accepted:** an upstream fix may differ from dcr-purse's earlier
  divergence and change a reply at the move that carries it (item 6(b));
  upstream may already have fixed a candidate after the pin, which the
  re-run of item 2 finds.
- **Constrains:** ADR-0016 (trigger (b)); ADR-0012 (Rule 11 cases above;
  its Open point 6, amended by Decision item 2 if the owner accepts Open
  point 3).

## Open points

Each for the owner, with a recommendation.

1. *The definitions of security-relevant and severity, and the S-list?*
   The definition has four clauses: outside parties (a), failed
   protections (b), Rule 6 and Rule 7 divergences whoever triggers them
   (c), and mixing denial of service (d); (c) and (d) are what admit S6,
   S7, S17, S10 and S12. Recommended: accept as written; the S-list is
   complete for the entries read. Alternative: keep only (a) and (b) and
   move those five rows to the public list, which leaves S6, S7 and S17
   out of the bounty route ADR-0012 Open point 6 already decided for them.
2. *Order and timing of the private reports.* Recommended: S1 as soon as
   its network-level run (a fake peer serving a lagging wallet's start-up
   sync) succeeds on current master; the others one by one, by severity,
   each as its run succeeds. S1, S2 and S8 are the high candidates. S1
   stays first although its window is the start-up sync: a remote P2P
   peer reaches it in the default SPV configuration and needs one crafted
   header. S8, also a crash by an unauthenticated party, follows: in the
   default configuration its sender must be a process on the same host
   (the listeners are loopback), and it must send about as many bytes as
   the memory it exhausts. S2 needs a configuration the operator chose
   (`--noservertls` with `--jsonrpcauthtype=clientcert`).
3. *Amending ADR-0012 Open point 6 (Decision item 2).* Two changes: the
   bounty route extended from S1, S6, S7, S17 and S18 to the whole S-list;
   and the precondition for the security class changed from oracle
   confirmation to a run on dcrwallet's master and latest release, with
   the reproducer kept outside this repository until upstream releases a
   fix (afterwards it may become an oracle test). Recommended: accept
   both. The bounty requires reproduction on master or the latest
   release, which an oracle run at the pin does not show, and an oracle
   differential checked in before a fix discloses the defect.
   Alternative: keep Open point 6 as decided, report S2 to S5 and S8 to
   S16 by public issue, and confirm the five decided ones by oracle first,
   which waits for the phases that port their code (Phases 5 to 12).
4. *Early public issues.* Recommended: QK-0204, the `MixSplitLimit` group,
   QK-0247, QK-0279, QK-0457, QK-0524, QK-0497 and QK-0531, each after a
   run; the rest as their phases confirm them.
5. *Publishing the repository.* Recommended: keep it private until the
   S-list is fixed upstream or cleared for disclosure. Alternative:
   publish with the S-list passages removed from the published tree, which
   needs care because history holds them.
6. *dcr-purse dispositions for the Rule 11 cases.* Recommended: as the
   "dcr-purse" column says, each decided when its phase builds the code
   (S13 in Phase 3, where `Manager.Lock` is built (spec 02), with
   `Wallet.Lock` in Phase 5 (spec 06) and `walletlock`'s reply checked in
   Phase 7; S2, S8 in Phase 7; S5, S16 in Phase 8; S9, S15 in Phase 10;
   S11 in Phase 11; S3, S4, S14 in Phase 12). S5 changes a reply (dcrwallet
   accepts the key, dcr-purse would refuse it), so it is a W5 and W6
   divergence.
7. *S10 and S12 (mixing).* Recommended: report both, and reproduce Go in
   dcr-purse until upstream changes, since peers that disagree blame each
   other (W3).
8. *QK-0279.* ADR-0012's "Relation to D13" names it as its example of
   the third class of candidate, a defect that harms dcrwallet's users,
   and it meets this record's rule for the public list: reserved fee
   credits stay unspendable until a restart or an explicit unlock, which
   costs users funds' availability, as the `MixSplitLimit` group costs
   them a feature until a restart. Recommended: keep it in the public
   list, as a medium-severity issue filed after a run, with the fix
   dcr-purse adopted (release the reserved credits of the tickets not
   published). Alternative: leave it out; that departs from ADR-0012's
   "Relation to D13" and needs the owner's word.

## What was not tested

- Every candidate marked "read". For S1, the network path (a peer serving
  the start-up header sync of a lagging SPV wallet) was not run; only the
  panic at the function was. The announcement route's closure by
  `maxAnnHeaderTipDelta`, and the cost of reaching a synced wallet through
  a near-tip anchor, are read from the source.
- Upstream after `54b85b33`: no candidate was checked against later
  commits or against the latest release, as item 2 requires before
  filing.
- Upstream issue trackers were not searched for duplicates.
- Whether Decrediton passes `--proxy` (S3), uses `CreateWatchingOnlyWallet`
  with user-pasted keys (S5) or stops the ticket buyer through
  `RunTicketBuyer` in a way that hits S4.
- The bounty program's scope for vspd and jrick/wsrpc.
- QK-0739's memory exhaustion and QK-0738's browser access were not
  exercised against a running dcrwallet.
- The scratch tests are not in the repository; this record describes them
  so they can be rewritten.
