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
