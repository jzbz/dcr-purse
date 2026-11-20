# Benchmark ledger

Every performance number dcr-purse relies on, against the Go reference,
keyed by machine, commits and workload, so that any two measurements can be
compared, or ruled incomparable, after the fact. Prose elsewhere (the
brief, ADRs, the README) cites these entries; this file is the record
(ADR-0014, section 4, "Ledger"). The rules are dcroxide's
(`docs/bench-ledger.md` there), with Go in the same session.

## Rules

- **Append only.** Entries are added, never rewritten. A correction or a
  later finding is a dated note appended under the entry it concerns.
- **What an entry names:** the machine (table below) and what else it was
  doing; the dcr-purse commit measured (or the commit it sits on plus the
  uncommitted change it measures); the Go reference (dcrwallet's parity
  commit, the Go release and the module versions the Go side reported);
  the dcroxide revision; the Rust toolchain and profile; every other
  version that bears on the result (for the store, the redb release); the
  workloads and their inputs; the command; and the raw files the figures
  come from.
- **Go and Rust in one session.** Both sides run on the same machine in
  one session, alternating process by process (Rust then Go, then Go then
  Rust, and so on), at least 3 processes per side. A process reports, per
  workload, the median of its timed repetitions; an entry reports the
  median of those per side and their range (least to greatest), and the
  ratio of Rust's median to Go's (below 1, Rust is faster). Best runs are
  never reported.
- **Rust against Rust.** Where no Go side runs (one layout or build of
  dcr-purse against another, as the store's measurements compare), an
  entry reports the median and range of at least three runs and says that
  Go was not run.
- **Same inputs, checked.** A session counts only if every output the two
  sides cross-check (`CHECK` lines: keys, addresses, digests, sealed
  boxes, and a digest of each solver instance's roots) is equal in every
  process; `purse-bench drive` refuses the session otherwise. The solver
  workloads' `CHECK` covers the instance both sides solve, not the
  solvers' output (the Go side may be built without FLINT and then prints
  none); each side holds its own solver's roots to the instance's, value
  by value, before timing it.
- **Load is stated.** A shared or loaded machine is said to be one, with
  the load averages the driver recorded and how the measured processes were
  pinned. Wall-clock figures from such a machine are marked as such; the
  exit criteria and the 15%-warn and 25%-fail ratio gates are decided on a
  dedicated machine (ADR-0014, section 4, "Regressions"), each such run an
  entry of its own.
- **Raw data stays out of the tree** and is not retained. A `Raw:` line
  names the files a session produced so the entry says what it rests on,
  not a location anyone can open. The figures in this file are the record.
- **The CI gate's baseline** (`crates/purse-bench/gate-baseline.tsv`) is
  taken from an entry, and every change to it cites the entry it comes
  from.
- **Machines are described generically:** CPU model, core and thread
  counts, memory, storage, operating system; never a host name or a local
  path.

ADR-0014 names its own evidence of 2026-11-19 (dcrwallet's SPV sync [S]
and the micro-benchmarks [H]) as this ledger's first rows; they have not
been entered yet.

## Machines

| id | CPU | cores/threads | RAM | storage | system | notes |
|---|---|---|---|---|---|---|
| d1 | AMD Ryzen 9 7950X3D | 16/32 (two 8-core dies, one with the stacked cache) | 96 GB | NVMe SSD, btrfs with zstd compression over an encrypted mapping | Linux 7.3, x86_64 | a shared desktop: other jobs run on it during measurements unless an entry says otherwise; `powersave` frequency governor with boost on |

## Phase 1 workloads (`purse-bench`)

The workloads, their inputs and the Phase 1 criteria they are held to are
listed in `crates/purse-bench/src/lib.rs`; the Go side is
`tools/oracle/cmd/bench`, which links the packages dcrwallet links at the
parity pin. Each calibrated workload repeats its operation for about the
stated time per repetition; `hd.addr20k`, `snacl.derive`, `kdf.*` and
`mix.*` time single operations.

### First entry: Phase 1's row (2026-11-19)

**Session.** Machine d1, shared and loaded (see below). dcr-purse
`6ea843d` plus the uncommitted changes that add `purse-bench` and this
ledger (including the `purse-dcrutil` change described under "Outside the
criteria"); dcroxide `5c6742f7`; Go side at dcrwallet `54b85b33`, built
with Go 1.27.4 (`go1.27.4-X:nodwarf5`) and the build tag `flint`, linking
hdkeychain v3.1.3, dcrec/secp256k1 v4.4.1, txscript v4.1.2, base58 v1.0.6,
x/crypto v0.57.0 and cspp v2.4.0; Rust 1.100.0, the workspace's release
profile (thin LTO, one codegen unit, `panic = "abort"`). Command:

```text
cargo build --release -p purse-bench
target/release/purse-bench drive --runs 5 --reps 5 --ms 200 \
    --cpus 12-15 --go-tags flint --out <session directory>
```

Five processes per side, alternating, 5 repetitions of about 200 ms per
calibrated workload; every measured process pinned with `taskset -c 12-15`
(four cores of one die; their SMT siblings were not reserved, and other
jobs could run on them), so GOMAXPROCS was 4 and Rust saw 4 threads.
2026-11-19, 09:29:04 to 09:33:31 UTC. One-minute load average 3.4 at the
start, 22 to 24 during runs 1 and 2 (other jobs on the machine), 4.8 at the
end; the high ends of several ranges below come from those two runs. All
21 cross-checked outputs were equal in all ten processes.

| workload | Go median (range) | Rust median (range) | Rust/Go | Phase 1 criterion | verdict |
|---|---|---|---|---|---|
| hd.master | 22.45 µs (22.27–35.78 µs) | 35.80 µs (35.62–35.92 µs) | 1.59 | secret path ≤ 1.15 | **missed** |
| hd.hardened_child | 22.91 µs (22.69–45.44 µs) | 36.14 µs (35.87–36.58 µs) | 1.58 | secret path ≤ 1.15 | **missed** |
| hd.priv_child | 22.94 µs (22.72–30.51 µs) | 35.99 µs (35.88–36.26 µs) | 1.57 | secret path ≤ 1.15 | **missed** |
| hd.pub_child | 36.35 µs (36.32–46.18 µs) | 20.13 µs (20.06–20.32 µs) | 0.55 | public derivation ≤ 1.00 | met |
| hd.address | 37.59 µs (37.41–38.40 µs) | 21.08 µs (21.05–21.26 µs) | 0.56 | address ≤ 1.00 | met |
| hd.script | 37.03 µs (36.77–37.97 µs) | 20.42 µs (20.34–20.57 µs) | 0.55 | script ≤ 1.00 | met |
| hd.addr20k | 762.69 ms (748.22–775.10 ms) | 422.63 ms (420.31–425.54 ms) | 0.55 | 20,000 addresses ≥ 1.5x faster | met (1.80x) |
| hd.xpub_string | 4.74 µs (4.66–4.76 µs) | 4.47 µs (4.43–5.04 µs) | 0.94 | — | — |
| hd.parse_xpub | 7.67 µs (7.60–11.56 µs) | 3.84 µs (3.81–4.83 µs) | 0.50 | — | — |
| crypto.hmac_sha512 | 793 ns (766 ns–2.08 µs) | 679 ns (669–911 ns) | 0.86 | HMAC ≤ 1.00 | met |
| crypto.hash160 | 327 ns (324–534 ns) | 274 ns (273–341 ns) | 0.84 | hash160 ≤ 1.00 | met |
| crypto.base58check_encode | 693 ns (683–775 ns) | 659 ns (658–868 ns) | 0.95 | base58 ≤ 1.00 | met |
| crypto.base58check_decode | 358 ns (352–377 ns) | 352 ns (350–550 ns) | 0.98 | base58 ≤ 1.00 | met |
| snacl.derive | 23.29 ms (21.66–24.55 ms) | 14.12 ms (13.98–21.62 ms) | 0.61 | — | — |
| snacl.seal_32 | 277 ns (272–283 ns) | 861 ns (854 ns–1.13 µs) | 3.10 | — | — |
| snacl.open_32 | 209 ns (205–232 ns) | 826 ns (818 ns–1.01 µs) | 3.96 | — | — |
| snacl.seal_111 | 451 ns (447–501 ns) | 1.32 µs (1.30–1.64 µs) | 2.92 | — | — |
| snacl.open_111 | 389 ns (383–395 ns) | 1.29 µs (1.28–1.55 µs) | 3.31 | — | — |
| kdf.argon2id_t1 | 107.33 ms (106.00–117.52 ms) | 101.42 ms (100.92–225.07 ms) | 0.94 | — | — |
| kdf.argon2id_t4 | 42.48 ms (37.82–51.56 ms) | 100.88 ms (100.62–274.90 ms) | 2.37 | — | — |
| dcrutil.amount_string | 67 ns (66–69 ns) | 133 ns (132–219 ns) | 2.00 | — | — |
| mix.roots_20 | 5.40 ms (5.38–5.71 ms) | 665.88 µs (661.60 µs–3.66 ms) | 0.12 | — | — |
| mix.roots_64 | 28.43 ms (28.07–29.02 ms) | 4.73 ms (4.69–6.81 ms) | 0.17 | — | — |
| mix.roots_512 | 445.95 ms (440.27–457.58 ms) | 216.32 ms (214.39–354.66 ms) | 0.49 | — | — |

**Phase 1's row.** Met: public derivation 0.55, address 0.56, script 0.55,
HMAC 0.86, hash160 0.84, base58 0.95 (encode) and 0.98 (decode), and the
20,000-address workload 1.80 times faster than Go (criterion 1.5).
**Missed: the secret paths**, master 1.59, hardened child 1.58 and private
child 1.57 times Go's time, against at most 1.15.

**Why the secret paths miss** (a `perf` profile of `hd.hardened_child`,
user-space cycles, flat): 40.3% in k256's projective point addition, 31.5%
in point doubling and 3.5% in its constant-time table lookup, all the
windowed multiplication of the generator by the private key (the child's
public key); 7.7% in field inversion (the conversion to affine); HMAC-SHA512,
BLAKE-256 and RIPEMD-160 together under 2.5%. dcroxide-dcrec builds k256
without its `precomputed-tables` feature, so the generator is multiplied as
any point is; Go's dcrec multiplies the generator from precomputed tables,
in variable time. The cause is in dcroxide, not in dcr-purse: it is
ADR-0014's O1 (Open point 8, decided as recommended: propose it to
dcroxide), which ADR-0014 measured at 36.8 to 19.2 µs per multiplication
against Go's 23.1 µs, which would put these three near 0.85. The change is
being made in dcroxide; dcroxide was not patched here, and nothing in
purse-hdkeychain's own code shows in the profile. The criterion is to be
re-measured when the dcroxide pin moves past it.

**base58 decode** meets the criterion by 2%. In the profiles, 50% of
Rust's time is dcroxide-crypto's portable BLAKE-256 compression (the
checksum's two hashes) and 34% dcroxide-base58's decode, against Go's AVX
assembly for BLAKE-256 (46% of its time) and 27% in its decode; both
causes are in dcroxide. ADR-0014 rejected SIMD BLAKE-256 for sync (S17)
for its small share there.

**Outside the criteria** (no Phase 1 criterion holds these; recorded for
the decisions they need):

- **secretbox seal and open, 2.9 to 4.0 times Go's time.** Profiled: most
  of the time is in the `poly1305` crate's (0.9.1, the latest release)
  AVX2 backend, whose helper functions call AVX2 intrinsics without being
  compiled for AVX2 themselves, so each intrinsic is an out-of-line call
  (`_mm256_permutevar8x32_epi32` alone took 23.5% of the samples). Built
  with `RUSTFLAGS='--cfg poly1305_backend="soft"'` (one process of that
  build, `taskset -c 12`, not alternating with Go, so indicative only):
  `snacl.open_32` 199 ns, `snacl.seal_32` 229 ns, `snacl.open_111` 385 ns,
  `snacl.seal_111` 411 ns, against Go's 209, 277, 389 and 451 ns above.
  The cause is in a third-party crate; `chacha20poly1305` (the
  per-account blobs) uses the same `poly1305` and is presumably affected
  the same way (not measured).
- **Argon2id with 4 lanes, 2.37 times Go's time** (with 1 lane, 0.94).
  RustCrypto's `argon2` computes the lanes on one thread unless its
  `parallel` feature (rayon) is on, and purse-snacl does not turn it on;
  x/crypto runs one goroutine per lane. dcrwallet derives an account
  passphrase's key with as many lanes as the machine has CPUs, so the gap
  grows with the core count (not measured above 4).
- **`Amount`'s string form, 2.0 times Go's time.** `purse-dcrutil`'s
  `Amount::format` and `Display` built the text through two more Strings
  than needed; they now append the unit to the number's own buffer and
  write the parts straight to the formatter (outputs unchanged; the
  crate's differential and upstream tests pass, and the session's
  cross-check covers 1,024 amounts). Before the change, in a 3-run session
  on the same machine at 09:21 UTC (`--reps 3 --ms 100`), Rust took 161 ns
  against Go's 68 ns. The rest is dcroxide-dcrjson's `format_float_f`,
  which formats through core's shortest exponential form and reassembles
  it through further Strings: a cause in dcroxide.
- **The mixing root solver**, in process against FLINT through cgo (what
  dcrwallet's `csppsolver` program runs; its process round trip is not
  timed): 0.12 at degree 20, 0.17 at 64, 0.49 at 512.
- **scrypt** (a master key at snacl's defaults) 0.61.

**What this entry does not show.** A dedicated machine's figures: the exit
verdict above is from a shared, loaded desktop and is to be confirmed on a
dedicated machine (ADR-0014, section 4). Other processors and ARM, multiple
threads, memory use, and the account blobs' XChaCha20-Poly1305 seal and
open (not a workload yet).

Raw: `run-1-rust.txt` to `run-5-go.txt` (with the driver's start and end
notes and load averages), `table.md`; the session before the
`purse-dcrutil` change, `trial1/run-1-rust.txt` to `run-3-go.txt`; the
profiles `hard.data`, `b58.data`, `b58go.data`, `box.data`, `boxgo.data`,
`boxsoft.data`, `amt.data`, `amtgo.data`.

**Note, 2026-11-19: base58 decode is at parity, not met.** The 0.98 above
(and "meets the criterion by 2%") did not reproduce. Nine later sessions on
machine d1, at the same commits (dcr-purse `6ea843d` plus the
uncommitted changes, dcroxide `5c6742f7`, the same Go toolchain and
modules; no base58 code on either side changed in between, though the Go
side was rebuilt for the last three), each with the entry's flags (`--runs 5 --reps 5 --ms 200
--cpus 12-15`, one with `--runs 7`), one over every workload, one over
`crypto.` and seven over `crypto.base58`, 10:29:15 to 10:42:49 UTC,
one-minute load average 25.3 at the first start, then between 5.4 and
12.4: decode ratios 1.00 (370.0 ns Go, 370.4 ns Rust: missed), 1.02
(missed), 0.98, 1.00, 1.01 (missed), 1.00, 1.00, 0.99 and 1.00 (358 against
360 ns: missed). The median ratio across the ten sessions is 1.00 and the
verdict flips with noise (missed in 4 of the 9), so the decode clause is
undecided, not met. Encode reproduced as met in all nine (0.91 to 0.95).
The cause of the parity is the one profiled above (about half of Rust's
time in dcroxide-crypto's portable BLAKE-256 against Go's AVX assembly),
in dcroxide. The clause is decided by the dedicated-machine run, or given
real margin by a dcroxide change. Raw: `sess1/` to `sess3/`, `sess-a/` to
`sess-c/`, `b58-1/` to `b58-3/` (`run-*.txt`, `table.md`).

**Note, 2026-11-19: the Go solver arm checked only the root count.** In the
session above, the Go side's `mix.roots_*` workloads compared the number of
roots FLINT returned with the instance's, not their values (the Rust side
compared the values); the roots `CHECK` covered only the generated
instance. The Go arm now compares every root, sorted, with the instance's
before timing (`checkRoots` in `tools/oracle/cmd/bench/mix.go`, with tests
`TestCheckRoots` and, under the `flint` tag, `TestFlintSolverRoots`, which
passes for the degrees 4, 8, 20, 32 and 64). The solver timings above are
not affected: the check runs once, outside the timed repetitions. Whether
FLINT returned the right values in that session was not recorded. A
3-run session over `mix.` with the checking Go side (built with `flint`,
10:43:43 UTC, pinned to cores 12-15, load average 8.2 at the start and
18.2 at the end) ran degrees 20, 64 and 512 without a mismatch on either
side; its times are not reported here (the load rose during it, and the
Rust 512-root runs ranged from 216 to 829 ms). Raw: `mix-1/`.

**Note, 2026-11-19: Argon2id's lanes now run in parallel.** purse-snacl
turns on the `argon2` crate's `parallel` feature, the cause named under
"Outside the criteria" above; with it, `kdf.argon2id_t4` measured 1.18
and 1.15 times Go's time on a rayon pool made for each derivation, and
1.13 and 1.14 on rayon's global pool, in four sessions on machine d1
(the entry "Argon2id with parallel lanes" below), against 2.37 in this
entry. The 1-lane workload is unchanged (0.96 to 0.97).

**Note, 2026-11-20: the secret paths meet the criterion at dcroxide
`2074168`.** The re-measurement ADR-0014's addendum of 2026-11-19 (item 1)
asked for once the pin includes dcroxide `5ace7a2` (k256's basepoint table
for the generator, ADR-0014 O1) is the entry "The secret paths at dcroxide
`2074168`" below: master 0.86, hardened child 0.84 and private child 0.85
times Go's time, against 1.59, 1.58 and 1.57 in this entry. base58
decoding measured 1.01 there and stays undecided, so the speed clause is
not closed (ADR-0014, addendum of 2026-11-20).

### The CI gate on an unpinned, loaded machine (2026-11-19)

The CI gate (`purse-bench drive --gate crates/purse-bench/gate-baseline.tsv`,
ADR-0014's micro ratios at a 2x regression) was run three times in a row on
machine d1, unpinned, as a CI job runs it: `--runs 3 --reps 5 --ms 100
--only hd.,crypto.,snacl.seal,snacl.open,dcrutil.`, the Go side prebuilt
from the session above, 09:34:17 to 09:38:39 UTC, one-minute load average
between 4.3 and 37 (the lowest and highest of the driver's records, those
taken at the start and end of each measured process included; the
sessions' own start and end records alone span 4.4 to 25). All three
passed. Across the 18 gated workloads and the
three sessions, the measured ratio lay between 0.73 (`hd.master`) and 1.31
(`dcrutil.amount_string`) times its baseline; the gate fails above 2.0.
Not run on GitHub's runners, whose processors differ from d1's: a
workload whose ratio depends on the processor (an AVX2 or BMI2 code path
on one side only) can sit nearer the limit there.

Raw: `gate-1/`, `gate-2/`, `gate-3/` (`run-*.txt`, `table.md`).

### Argon2id with parallel lanes (2026-11-19)

**Session.** Machine d1, shared but lightly loaded at the time:
one-minute load average 0.79 at the start of the first session and 1.64
at the end of the last. dcr-purse `b44f267` plus the uncommitted change
that turns on the `argon2` crate's `parallel` feature in purse-snacl
(argon2 0.6.0 on rayon 1.12.0 and rayon-core 1.13.0) and runs each
derivation on a rayon pool of its own ("pool per derivation" below);
for comparison, the same tree with `crates/purse-snacl/src/kdf.rs` as at
`b44f267`, under which the feature fills the lanes on rayon's global
pool ("global pool"). dcroxide `f4b3d2e8`; Go side at dcrwallet
`54b85b33`, built with Go 1.27.4 (`go1.27.4-X:nodwarf5`) without build
tags, linking x/crypto v0.57.0, one binary for every session; Rust
1.100.0, the workspace's release profile. Four sessions back to back,
21:03:14 to 21:03:56 UTC, alternating pool per derivation, global pool,
pool per derivation, global pool, each:

```text
cargo build --release -p purse-bench
target/release/purse-bench drive --runs 5 --reps 5 --ms 200 \
    --cpus 12-15 --only kdf. --go-bin <prebuilt Go side> --out <session directory>
```

Five processes per side, alternating, five timed derivations per
process; every measured process pinned with `taskset -c 12-15`, as in the
first entry, so GOMAXPROCS was 4, and a Rust derivation ran on 4 threads:
the calling thread and 3 it started, on a pool per derivation, or 4
threads of the global pool. Each timed derivation on a pool of its own
includes starting and joining those 3 threads. Both derived keys were
equal in all ten processes of each session.

| workload | Rust variant | session | Go median (range) | Rust median (range) | Rust/Go |
|---|---|---|---|---|---|
| kdf.argon2id_t1 | pool per derivation | 1 | 108.57 ms (107.79–109.50 ms) | 105.36 ms (102.62–108.16 ms) | 0.97 |
| kdf.argon2id_t4 | pool per derivation | 1 | 38.68 ms (38.52–43.81 ms) | 45.69 ms (44.20–47.33 ms) | 1.18 |
| kdf.argon2id_t1 | global pool | 1 | 109.07 ms (108.49–110.19 ms) | 104.17 ms (103.22–105.46 ms) | 0.96 |
| kdf.argon2id_t4 | global pool | 1 | 39.55 ms (38.83–40.66 ms) | 44.71 ms (44.08–46.28 ms) | 1.13 |
| kdf.argon2id_t1 | pool per derivation | 2 | 106.92 ms (106.15–107.32 ms) | 103.11 ms (102.19–105.13 ms) | 0.96 |
| kdf.argon2id_t4 | pool per derivation | 2 | 38.30 ms (38.06–42.83 ms) | 43.99 ms (43.67–45.40 ms) | 1.15 |
| kdf.argon2id_t1 | global pool | 2 | 106.80 ms (106.11–107.36 ms) | 102.75 ms (102.28–103.09 ms) | 0.96 |
| kdf.argon2id_t4 | global pool | 2 | 38.57 ms (38.31–39.22 ms) | 44.00 ms (43.71–44.54 ms) | 1.14 |

The 4-lane derivation went from 2.37 times Go's time (first entry) to
1.15 to 1.18 on a pool per derivation and 1.13 to 1.14 on the global
pool. In Rust's own medians the pool per derivation was 0.98 ms slower
than the global pool in the first pair of sessions and 0.01 ms faster
in the second; the ranges overlap in both. Two sessions of the global
pool alone, earlier the same day on the same machine under load
(one-minute load average 11.6 at the start, 9.2 at the end; 20:18:27 to
20:19:03 UTC), measured 1.14 and 1.11 (raw: `kdf-c4/`, `kdf-c4-b/`);
they are not comparable with the sessions above. The remaining gap to Go
was not profiled. One candidate, not measured: purse-snacl writes the
256 MiB of working memory twice outside the lane fill, zeroing it on
allocation (on the calling thread) and wiping it after (on whichever of
the pool's threads ran the derivation), while Go's runtime zeroes a
reused allocation once, on one goroutine, and a fresh one not at all.

**What this entry does not show.** More than 4 lanes or 4 threads
(dcrwallet stores as many lanes as the creating machine has CPUs, and a
pool per derivation starts one thread fewer than it has threads), a
derivation whose threads are refused (the calling thread then fills
every lane alone), a dedicated machine, other processors. `kdf.*` is
not in the CI gate's baseline (`crates/purse-bench/gate-baseline.tsv`),
which is unchanged.

Raw: `kdf-c4-scoped/`, `kdf-c4-global/`, `kdf-c4-scoped-b/` and
`kdf-c4-global-b/` (`run-1-rust.txt` to `run-5-go.txt`, `table.md`).

### The secret paths at dcroxide `2074168` (2026-11-20)

**Session.** Machine d1, shared and loaded (see below). dcr-purse
`acd4bae` plus the uncommitted changes that move the dcroxide pin to
`2074168` (`docs/SKEW.md`, 2026-11-20 section) and add this entry;
dcroxide `2074168d`, whose `dcroxide-dcrec` multiplies a secret scalar by
the generator through k256's basepoint table (dcroxide `5ace7a2`, its
default feature `precomputed-tables`, which every dcr-purse crate leaves
on; k256 0.14.0, primeorder 0.14.0). Go side at dcrwallet `54b85b33`,
built with Go 1.27.4 (`go1.27.4-X:nodwarf5`) and the build tag `flint`,
linking hdkeychain v3.1.3, dcrec/secp256k1 v4.4.1, txscript v4.1.2,
base58 v1.0.6, x/crypto v0.57.0 and cspp v2.4.0, one binary for all three
sessions; Rust 1.100.0, the workspace's release profile. Since the first
entry, the code of the `hd.*` and `crypto.*` workloads changed on neither
side except through the dcroxide pin (`5c6742f7` there); `purse-snacl`'s
Argon2id gained its parallel lanes (the entry above), and the Go solver arm
its root check, which runs outside the timed repetitions. Three sessions
back to back, each the first entry's command, the second and third with
the first session's Go side:

```text
cargo build --release -p purse-bench
target/release/purse-bench drive --runs 5 --reps 5 --ms 200 \
    --cpus 12-15 --go-tags flint --out <session directory>
target/release/purse-bench drive --runs 5 --reps 5 --ms 200 \
    --cpus 12-15 --go-bin <the first session's Go side> --out <session directory>
```

Five processes per side and session, alternating, 5 repetitions of about
200 ms per calibrated workload; every measured process pinned with
`taskset -c 12-15` as in the first entry (their SMT siblings not
reserved), so GOMAXPROCS was 4 and Rust saw 4 threads. 2026-11-20,
09:31:36 to 09:45:22 UTC. One-minute load average, from the driver's
records at the start and end of every process: 22.9 at the start of the
first session falling to 3.8 at its end; 2.8 to 10.7 in the second; 6.7 to
16.1 in the third (other jobs on the machine). Four processes stand out on
the secret paths: Rust's fourth process of the second session and Go's
first, fourth and fifth of the third (`hd.hardened_child` 31.0 µs for that
Rust process, 51.7, 28.5 and 30.2 µs for those Go processes); they are the
high ends of the ranges below and lower the third session's ratios. All
21 cross-checked outputs were equal in all thirty processes.

The table pools the three sessions: per side, the median of the fifteen
processes' medians and their range; the column "by session" gives each
session's own ratio, in session order.

| workload | Go median (range), 15 processes | Rust median (range), 15 processes | Rust/Go | by session | Phase 1 criterion | verdict |
|---|---|---|---|---|---|---|
| hd.master | 23.05 µs (22.75–47.71 µs) | 19.84 µs (19.55–29.90 µs) | 0.86 | 0.86, 0.85, 0.79 | secret path ≤ 1.15 | met |
| hd.hardened_child | 24.16 µs (23.41–51.72 µs) | 20.24 µs (19.75–30.96 µs) | 0.84 | 0.85, 0.83, 0.75 | secret path ≤ 1.15 | met |
| hd.priv_child | 23.88 µs (23.46–49.20 µs) | 20.37 µs (19.73–31.68 µs) | 0.85 | 0.86, 0.87, 0.79 | secret path ≤ 1.15 | met |
| hd.pub_child | 38.48 µs (37.46–83.30 µs) | 20.94 µs (20.59–38.02 µs) | 0.54 | 0.56, 0.54, 0.50 | public derivation ≤ 1.00 | met |
| hd.address | 39.24 µs (38.43–46.65 µs) | 21.85 µs (21.67–38.98 µs) | 0.56 | 0.56, 0.55, 0.54 | address ≤ 1.00 | met |
| hd.script | 38.62 µs (37.82–42.31 µs) | 21.09 µs (20.78–36.53 µs) | 0.55 | 0.55, 0.54, 0.55 | script ≤ 1.00 | met |
| hd.addr20k | 790.30 ms (774.32–868.33 ms) | 438.20 ms (432.48–731.71 ms) | 0.55 | 0.56, 0.56, 0.55 | 20,000 addresses ≥ 1.5x faster | met (1.80x) |
| hd.xpub_string | 4.81 µs (4.74–5.51 µs) | 4.56 µs (4.51–5.82 µs) | 0.95 | 0.94, 1.00, 0.93 | — | — |
| hd.parse_xpub | 7.88 µs (7.71–9.01 µs) | 3.97 µs (3.92–5.01 µs) | 0.50 | 0.50, 0.53, 0.49 | — | — |
| crypto.hmac_sha512 | 844 ns (801 ns–1.19 µs) | 702 ns (685 ns–1.42 µs) | 0.83 | 0.83, 0.88, 0.81 | HMAC ≤ 1.00 | met |
| crypto.hash160 | 344 ns (330–405 ns) | 278 ns (274–447 ns) | 0.81 | 0.81, 0.83, 0.79 | hash160 ≤ 1.00 | met |
| crypto.base58check_encode | 722 ns (699–820 ns) | 678 ns (668–755 ns) | 0.94 | 0.96, 0.95, 0.91 | base58 ≤ 1.00 | met |
| crypto.base58check_decode | 365 ns (358–432 ns) | 369 ns (360–390 ns) | 1.01 | 1.02, 1.01, 0.99 | base58 ≤ 1.00 | **undecided** (at parity) |
| snacl.derive | 23.84 ms (23.12–25.60 ms) | 15.52 ms (14.46–16.65 ms) | 0.65 | 0.65, 0.66, 0.61 | — | — |
| snacl.seal_32 | 282 ns (277–314 ns) | 875 ns (868–922 ns) | 3.10 | 3.08, 3.13, 3.02 | — | — |
| snacl.open_32 | 214 ns (208–244 ns) | 849 ns (836 ns–1.02 µs) | 3.97 | 3.99, 4.03, 3.95 | — | — |
| snacl.seal_111 | 460 ns (454–524 ns) | 1.35 µs (1.32–1.59 µs) | 2.95 | 2.97, 2.86, 2.83 | — | — |
| snacl.open_111 | 395 ns (386–440 ns) | 1.32 µs (1.28–1.96 µs) | 3.34 | 3.33, 3.31, 3.35 | — | — |
| kdf.argon2id_t1 | 112.69 ms (110.18–117.05 ms) | 108.36 ms (105.12–367.47 ms) | 0.96 | 0.98, 0.97, 0.93 | — | — |
| kdf.argon2id_t4 | 41.83 ms (39.37–47.28 ms) | 51.19 ms (45.79–143.85 ms) | 1.22 | 1.22, 1.24, 1.17 | — | — |
| dcrutil.amount_string | 70 ns (69–89 ns) | 152 ns (145–258 ns) | 2.16 | 2.20, 2.07, 2.17 | — | — |
| mix.roots_20 | 5.62 ms (5.42–6.10 ms) | 685.63 µs (671.10 µs–1.03 ms) | 0.12 | 0.12, 0.12, 0.13 | — | — |
| mix.roots_64 | 29.16 ms (28.48–31.90 ms) | 4.88 ms (4.78–7.37 ms) | 0.17 | 0.17, 0.17, 0.17 | — | — |
| mix.roots_512 | 459.75 ms (449.06–705.43 ms) | 223.00 ms (217.97–322.14 ms) | 0.49 | 0.48, 0.47, 0.49 | — | — |

**Phase 1's row.** **The secret paths meet it**: master 0.86, hardened
child 0.84 and private child 0.85 times Go's time, against at most 1.15
(1.59, 1.58 and 1.57 in the first entry); Rust's time fell from about
36 µs to about 20 µs per derivation, while Go's medians were 22.5 to
22.9 µs then and 23.1 to 24.2 µs now, as ADR-0014's measurement of O1
predicted (near 0.85). The other met
clauses are met again, within a few hundredths of the first entry:
public derivation 0.54, address 0.56, script 0.55, HMAC 0.83, hash160
0.81, base58 encode 0.94, and 20,000 addresses 1.80 times faster.
**base58 decode is still at parity, not met**: 1.01 pooled, 1.02, 1.01
and 0.99 by session, so the driver's verdict was "missed" in two of the
three sessions, as in the first entry's dated note (no base58 or
BLAKE-256 code changed on either side). The row is therefore not met as a
whole: the one workload short of it is `crypto.base58check_decode`, 1.01
against at most 1.00. ADR-0014's addendum of 2026-11-19 (item 2) settles
it by a measurement on a dedicated machine.

Outside the criteria, nothing moved beyond the noise of a shared machine
except `kdf.argon2id_t4`, at 1.22 here against 1.13 to 1.18 in the
Argon2id entry above, which was measured at a load average near 1; its
Rust range (to 143.85 ms) shows the load, and it was not investigated
further. secretbox stays at 2.95 to 3.97 times Go's time and `Amount`'s
string form at 2.16, both accepted for now (ADR-0014, addendum of
2026-11-19, items 3 and 4).

**What this entry does not show.** A dedicated machine's figures (the
verdicts above are from a shared, loaded desktop and are to be confirmed
on one, ADR-0014 section 4); a profile of the new secret paths (not
taken); other processors and ARM; more than 4 threads.

Raw: `s1/`, `s2/`, `s3/` (`run-1-rust.txt` to `run-5-go.txt` with the
driver's start and end notes and load averages, `table.md`), and the
pooling script's output `pooled.txt`.

## purse-store

### 2026-11-19 — headers and filters by height or by hash; tables per commit

Brief §6 Phase 2 exit item (e): ADR-0014 Open point 6 and ADR-0003 Open
point 9, answered in ADR-0003's addendum of the same date.

- **Machine:** d1, carrying other work: load average 27, 9 and 13 at the
  end of the three runs, so the sync of a single durable commit took
  between 8 and 20 ms depending on the run.
- **Commits:** dcr-purse `6ea843d` with the uncommitted `purse-store`
  change that adds this row (its `purse-store` code is that of the commit
  adding this entry, which was later stacked on the commits before it);
  redb 5.0.0; Rust 1.100.0, release profile. Go: not run.
- **Harness:** `crates/purse-store/tests/measure.rs`, both ignored tests,
  `--test-threads 1`, three runs back to back, 10:12 to 10:22 UTC. Every
  store is a real file committed through `Store::update` with
  `Durability::Immediate`, under a backend that counts what reaches redb's
  file backend. Bytes, writes and
  syncs were identical in all three runs; times are the median of the three
  runs' figures with their range.
- **Raw:** per run, `headers-and-filters.txt`, `tables-per-commit.txt` and
  the test log; not retained.

**`headers_and_filters_by_height_or_hash`.** 1,050,000 generated blocks,
180-byte headers, filters of 170 to 369 bytes (16-byte key plus filter),
525 durable commits of 2,000 blocks; then 200 one-block commits; then
100,000 warm header reads by height and by hash and 200 warm runs of 2,000
consecutive filters by height. Layouts as in the ADR-0003 addendum.

| figure | hash | height | merged |
|---|---:|---:|---:|
| ingest, s | 90.0 (89.3–91.5) | 32.4 (30.1–45.5) | 32.5 (28.8–43.2) |
| ingest bytes written | 13.99 GiB | 4.06 GiB | 4.05 GiB |
| ingest write calls; syncs | 3,667,272; 543 | 1,065,867; 543 | 1,062,337; 543 |
| 2,000-block commit (last 50), p50 ms | 201.8 (198.0–212.8) | 71.9 (68.3–88.9) | 65.2 (64.8–69.2) |
| 2,000-block commit (last 50), p90 ms | 224.8 (219.0–246.2) | 81.6 (73.9–185.6) | 70.2 (69.6–88.1) |
| bytes per 2,000-block commit | 32.25 MiB | 9.21 MiB | 9.18 MiB |
| one-block commit, p50 ms | 18.25 (9.73–18.78) | 18.28 (17.40–19.46) | 18.59 (17.01–19.84) |
| bytes and writes per one-block commit | 85.2 KiB, 22.2 | 84.4 KiB, 22.0 | 52.5 KiB, 14.0 |
| header by height, warm, µs | 1.59 (1.40–2.12) | 0.65 (0.52–1.76) | 0.80 (0.78–0.88) |
| header by hash, warm, µs | 0.87 (0.81–1.02) | 1.57 (1.47–3.81) | 1.87 (1.81–2.28) |
| 2,000 filters by height, warm, ms | 1.79 (1.64–1.88) | 0.17 (0.16–0.44) | 0.26 (0.24–0.40) |
| file after ingest; after close | 1,088; 1,088 MiB | 1,088; 1,029 MiB | 1,088; 1,031 MiB |

**`commit_cost_by_tables_touched`.** 15 tables of 100,000 rows each
(36-byte random keys, 60-byte values), 272 MiB, grown in 20,000-row
commits; then 100 commits per spread of 30 new random-key rows over k
tables chosen at random, the spreads interleaved.

| k | bytes per commit | writes per commit | syncs | p50 ms | p90 ms |
|---:|---:|---:|---:|---:|---:|
| 1 | 222.6 KiB | 56.6 | 1.0 | 18.21 (7.59–19.30) | 20.72 (10.34–21.95) |
| 2 | 245.9 KiB | 62.4 | 1.0 | 18.38 (7.82–19.48) | 20.79 (10.97–22.16) |
| 4 | 264.6 KiB | 67.1 | 1.0 | 18.36 (7.96–19.69) | 20.25 (11.13–22.11) |
| 8 | 287.6 KiB | 72.8 | 1.0 | 18.62 (8.39–19.60) | 21.19 (11.21–21.84) |
| 15 | 319.8 KiB | 80.9 | 1.0 | 19.11 (8.77–20.16) | 22.13 (12.77–22.86) |

Within each run, k = 15 took 0.86, 0.90 and 1.18 ms longer than k = 1 at
the median. Not measured: cold reads (the page cache was not dropped),
reorganisations and detached rows, relaxed durability in the three
layouts, other platforms and file systems.

### 2026-11-19 — the chain tables keyed by height, as built

Schema version 1 with main-chain headers and filters keyed by height and
detached ones by hash (ADR-0003, addendum "Chain tables keyed by height,
as built"), measured through the chain operations beside the three raw
layouts of the entry above, in one session.

- **Machine:** d1, carrying other work (builds and tests of other
  worktrees): one-minute load average 8.2 at the start of the first run,
  22.4 and 25.0 at the ends of the first and second, 6.9 at the end of the
  third. Not pinned. The sync of a single durable commit took between
  about 6 and 19 ms depending on the run, so every time below that a
  commit's sync dominates moves with the load.
- **Commits:** dcr-purse `b44f267` with the uncommitted change that keys
  the chain tables by height and adds this entry; redb 5.0.0; Rust
  1.100.0, release profile. Go: not run (Rust against Rust).
- **Harness:** `crates/purse-store/tests/measure.rs`, both ignored tests,
  `--test-threads 1`, three runs back to back, 22:06:02 to 22:20:44 UTC:

  ```text
  PURSE_STORE_BENCH_DIR=<scratch directory> cargo test --release -p purse-store \
      --locked --test measure -- --ignored --nocapture --test-threads 1
  ```

  Bytes, writes and syncs were identical in all three runs; times are the
  median of the three runs' figures with their range.
- **Raw:** per run, `headers-and-filters.txt`, `tables-per-commit.txt`,
  the test log and the start and end notes with the load averages; not
  retained.

**`headers_and_filters_by_height_or_hash`.** The workload of the entry
above (1,050,000 blocks, 525 commits of 2,000, then 200 one-block commits
and the warm reads), now in five layouts: *hash*, *height* and *merged*
written as raw rows as before; *chain*, the layout as built, written
through `WriteTx::connect_blocks` in durable `Store::update`s and read
through `ReadTx::header_at`, `ReadTx::header` and `ReadTx::cfilters_in`
(headers of the same size, with the height and previous-block fields the
operations check set); *chain-relaxed*, the same with the ingest as
`InitialSync` batches at the default interval (a durable commit every
50th batch and at `finish`), the tip commits durable. On the two *chain*
files, also 100 durable reorganisations per depth at the tip, alternating
between the main chain's last blocks and a branch of as many.

| figure | hash | height | merged | chain | chain-relaxed |
|---|---:|---:|---:|---:|---:|
| ingest, s | 103.9 (97.9–134.8) | 37.5 (33.0–53.1) | 33.3 (30.0–67.4) | 33.9 (30.1–36.4) | 9.3 (8.5–10.8) |
| ingest bytes written | 13.99 GiB | 4.06 GiB | 4.05 GiB | 4.06 GiB | 1.09 GiB |
| ingest write calls; syncs | 3,667,272; 543 | 1,065,867; 543 | 1,062,337; 543 | 1,065,867; 543 | 286,786; 29 |
| 2,000-block commit (last 50), p50 ms | 294.8 (218.2–302.9) | 80.1 (76.4–94.2) | 68.4 (64.5–75.6) | 68.5 (63.3–71.3) | 13.2 (12.7–16.0) |
| 2,000-block commit (last 50), p90 ms | 523.8 (283.2–627.6) | 99.0 (95.6–160.9) | 79.9 (69.1–111.8) | 83.2 (68.0–87.7) | 15.8 (13.2–17.3) |
| bytes per 2,000-block commit (last 50, mean) | 32.25 MiB | 9.21 MiB | 9.18 MiB | 9.21 MiB | 2.64 MiB |
| one-block commit, p50 ms | 10.80 (10.25–18.93) | 11.40 (9.31–19.63) | 17.23 (9.41–19.22) | 10.02 (9.41–17.02) | 10.57 (6.10–10.62) |
| bytes and writes per one-block commit | 85.2 KiB, 22.2 | 84.4 KiB, 22.0 | 52.5 KiB, 14.0 | 84.4 KiB, 22.0 | 84.4 KiB, 22.0 |
| header by height, warm, µs | 1.91 (1.34–2.08) | 0.81 (0.77–1.23) | 0.85 (0.75–0.96) | 0.73 (0.64–0.77) | 0.76 (0.68–0.87) |
| header by hash, warm, µs | 1.00 (0.66–1.02) | 1.77 (1.75–2.58) | 1.86 (1.61–2.39) | 1.62 (1.55–1.72) | 1.63 (1.54–2.08) |
| 2,000 filters by height, warm, ms | 1.98 (1.90–2.20) | 0.19 (0.18–0.26) | 0.28 (0.25–0.29) | 0.16 (0.15–0.17) | 0.17 (0.16–0.20) |
| file after ingest; after close | 1,088; 1,088 MiB | 1,088; 1,029 MiB | 1,088; 1,031 MiB | 1,088; 1,029 MiB | 1,088; 1,088 MiB |

| reorganisation at the tip (durable) | chain | chain-relaxed file |
|---|---:|---:|
| depth 1, p50 ms | 10.20 (9.72–18.54) | 10.49 (6.29–10.76) |
| depth 1, p90 ms | 13.01 (10.19–27.25) | 11.29 (6.59–12.14) |
| depth 6, p50 ms | 11.20 (10.49–19.56) | 11.31 (7.09–19.12) |
| depth 6, p90 ms | 12.58 (11.01–36.56) | 18.50 (7.70–19.86) |
| bytes and writes per commit, depth 1 | 100.4 KiB, 26.0 | 100.4 KiB, 26.0 |
| bytes and writes per commit, depth 6 | 184.4 KiB, 47.0 | 184.4 KiB, 47.0 |

Read against each other: the chain operations write exactly the bytes of
the raw height layout (4.06 GiB, 9.21 MiB per batch, 84.4 KiB per tip
commit), so their extra reads per block (the hash index, the two detached
tables, the tip) cost no I/O; their time is within this session's noise
of the raw layout's (33.9 against 37.5 s at the median, the raw layout's
first run at the load's peak). Keyed by hash, the same ingest writes 3.45
times the bytes and, at the median, took 3.1 times as long as *chain*
(the entry above: 3.4 and 2.8). With relaxed batches the ingest writes
1.09 GiB in 29 syncs and took 9.3 s, 3.6 times faster than *chain* with
every batch durable. The relaxed per-batch mean is spread over the
durable commits that write what the relaxed ones kept in memory; a
relaxed batch alone writes almost nothing. A reorganisation of one block
at the tip writes 100.4 KiB in 26 writes, against 84.4 KiB in 22 for
connecting one block: the detached tables' leaves on top.

**`commit_cost_by_tables_touched`.** Unchanged workload.

| k | bytes per commit | writes per commit | syncs | p50 ms | p90 ms |
|---:|---:|---:|---:|---:|---:|
| 1 | 222.6 KiB | 56.6 | 1.0 | 7.70 (7.56–18.31) | 13.40 (8.87–21.98) |
| 2 | 245.9 KiB | 62.4 | 1.0 | 7.87 (7.81–17.67) | 14.65 (9.60–21.75) |
| 4 | 264.6 KiB | 67.1 | 1.0 | 8.16 (8.01–17.83) | 15.14 (9.53–22.73) |
| 8 | 287.6 KiB | 72.8 | 1.0 | 8.42 (8.33–18.97) | 15.26 (9.49–22.94) |
| 15 | 319.8 KiB | 80.9 | 1.0 | 8.72 (8.65–18.89) | 16.04 (9.87–22.80) |

Bytes and writes are those of the entry above; the times are lower in two
of the three runs, the machine's syncs being faster then.

**What this entry does not show.** A dedicated machine's figures; cold
reads (the page cache was not dropped); headers from a real chain (the
generated ones are random bytes, which neither redb nor the counting
backend compresses, so the byte counts do not depend on it); deep
reorganisations (above 6 blocks) on a grown file; other platforms and file
systems.

## purse-txrules and purse-txauthor

The crates' ignored `tests/bench.rs` time their main entry points against
dcrwallet's through the Go oracles' `bench` commands
(`tools/oracle/cmd/txrules`, `tools/oracle/cmd/txauthor`), with the same
inputs on both sides. Each side reports the best of the timed rounds that
fit its budget (Go: `testing.Benchmark`'s ns/op), so these figures do not
meet this file's rules (no alternation, one process per side, best rounds
rather than medians); they are indicative only, recorded so that the
numbers the crates' reports cite are on file.

### First entry: single unpinned runs (2026-11-19)

**Session.** Machine d1, shared and loaded (other jobs ran on it; load
averages not recorded), not pinned with `taskset`. dcr-purse `6ea843d`
plus the uncommitted change that adds both crates; dcroxide `5c6742f7`;
Go side at dcrwallet `54b85b33`, Go 1.27.4; Rust 1.100.0, the release
profile. One run of each command, about 11:23 UTC, with the default
budget of 300 ms per entry point and side:

```text
PURSE_REQUIRE_ORACLE=1 cargo test --release -p purse-txrules --test bench -- --ignored --nocapture
PURSE_REQUIRE_ORACLE=1 cargo test --release -p purse-txauthor --test bench -- --ignored --nocapture
```

| entry point | Go ns | Rust ns | Go/Rust |
|---|---:|---:|---:|
| EstimateSerializeSize (2 P2PKH in, 2 P2PKH out, change) | 4.43 | 3.83 | 1.15 |
| EstimateSerializeSizeFromScriptSizes (2 in, 2 out, change) | 4.43 | 3.63 | 1.22 |
| FeeForSerializeSize | 1.21 | 0.84 | 1.45 |
| IsDustOutput (P2PKH) | 21.13 | 7.87 | 2.69 |
| CheckOutput (P2PKH) | 22.89 | 9.64 | 2.37 |
| PaysHighFees (2 in, 2 out) | 6.58 | 5.40 | 1.22 |
| TxPaysHighFees (2 in, 2 out) | 7.88 | 6.72 | 1.17 |
| StakePoolTicketFee (mainnet, height 1,000,000), Rust with a kept `StakePoolFees` | 175.39 | 15.38 | 11.40 |
| NewUnsignedTransaction (2 P2PKH out, 5 UTXOs, change) | 489.76 | 248.90 | 1.97 |
| RandomizeOutputPosition (3 outputs) | 55.08 | 22.53 | 2.44 |
| AddAllInputScripts (2 P2PKH inputs) | 95,187.01 | 150,046.34 | 0.63 |

Go/Rust above 1 means the crate is faster (the inverse of the ratio the
Phase 1 table reports). The free function `stake_pool_ticket_fee`, which
builds a subsidy cache per call, took 492.59 ns, about 2.8 times Go's
175.39. Signing is the one entry point slower than Go: dcroxide's
`sign_tx_output` called directly took 150,030.73 ns for the two inputs,
of which 34,554.03 ns per input deriving the secp256k1 public key and
39,343.85 ns per ECDSA signature, so the cost is in dcroxide's signer, not
in `purse-txauthor`. `randomize_output_position_with` over an owned `Prng`
took 21.73 ns.

**What this entry does not show.** Medians and ranges over alternating
processes, a pinned or idle machine, other processors.

Raw: `bench-txrules.log`, `bench-txauthor.log`.

### Signing at dcroxide `2074168` (2026-11-20)

**Session.** Machine d1, shared (one-minute load average 8.9 at the start,
8.0 at the end). dcr-purse `acd4bae` plus the uncommitted changes that
move the dcroxide pin to `2074168` (which includes dcroxide `5ace7a2`:
ECDSA's nonce point and the public key are now multiplied through k256's
basepoint table) and add this entry; Go side at dcrwallet `54b85b33`, the
oracle `tools/oracle/cmd/txauthor` built with Go 1.27.4
(`go1.27.4-X:nodwarf5`) without tags; Rust 1.100.0, the release profile.
The first entry's purse-txauthor command, its test binary run three times
back to back under `taskset -c 12-15` (the Go oracle it starts inherits
the pinning), 09:46:33 to 09:46:51 UTC, default budget of 300 ms per entry
point and side. The section's caveat holds: each run reports each side's
best round, so the table gives the median and range of three best-round
figures, one process per side per run, not alternating; indicative only.

| entry point | Go ns, median (range) | Rust ns, median (range) | Go/Rust |
|---|---:|---:|---:|
| NewUnsignedTransaction (2 P2PKH out, 5 UTXOs, change) | 437.39 (418.78–478.07) | 234.02 (231.35–235.31) | 1.87 |
| RandomizeOutputPosition (3 outputs) | 56.97 (56.49–57.54) | 23.29 (23.26–23.46) | 2.45 |
| AddAllInputScripts (2 P2PKH inputs) | 100,027.58 (98,335.28–102,056.56) | 86,269.22 (86,215.78–86,790.31) | 1.16 |

Signing two inputs went from 0.63 (first entry, unpinned) to 1.16 by
Go/Rust, that is, Rust's time 0.86 of Go's. dcroxide's `sign_tx_output`
called directly took 86,263.98 ns (86,120.31–88,015.93) for the two
inputs, of which 18,637.25 ns per input deriving the public key and
23,639.68 ns per ECDSA signature (34,554.03 and 39,343.85 ns in the first
entry). `randomize_output_position_with` on an owned `Prng` (now
`dcroxide_crypto::rand::Prng`, the crate's own random source being gone)
took 21.58 ns (21.53–21.65). No Phase exit holds these figures yet
(ADR-0014's Phase 4 row bounds signing at 1.15 times Go's time, end to
end through `createtx`).

**What this entry does not show.** Medians over alternating processes, an
idle machine, other processors, signing end to end.

Raw: `txauthor-1.log` to `txauthor-3.log` (with start and end load
averages).
