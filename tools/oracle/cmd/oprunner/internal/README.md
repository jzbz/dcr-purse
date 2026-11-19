# oprunner copies

The operation-script runner (`cmd/oprunner`) must run dcrwallet's
`wallet/udb` with injected randomness, clock and processor count. udb reads
`github.com/decred/dcrd/crypto/rand`, `time.Now()` and (through `kdf`)
`runtime.NumCPU()` directly, and the oracle is built with a plain `go build`
in this module, so neither a module replacement nor `-overlay` can reach
them. The runner links copies instead:

| Directory | Upstream | Changed |
| --- | --- | --- |
| `udb` | `decred.org/dcrwallet/v5/wallet/udb`, every non-test file | imports of the four packages below point here; `time.Now()` is `seam.Now()` in `addressdb.go` and `txdb.go`; `addressdb.go` drops its then unused `time` import |
| `snacl` | `wallet/internal/snacl/snacl.go` | the `crypto/rand` import points at `rand` here |
| `compat` | `internal/compat/compat.go` | nothing (an internal package cannot be imported from this module) |
| `kdf` | `kdf/kdf.go` | `runtime.NumCPU()` is `seam.NumCPU()` |
| `rand` | `github.com/decred/dcrd/crypto/rand` v1.0.1: `default.go`, `doc.go`, `uniform.go` | nothing; `prng.go` here is not a copy but the seeded stream that replaces upstream's `prng.go` and `prng_arc4random.go` |
| `seam` | — | the injected clock and processor count |

Each copied file keeps its upstream copyright and license header.
`../copies_test.go` fails unless every copy equals the upstream file of the
module version `go.mod` pins, as found in the module cache, after exactly
the listed rewrites (each with its expected count) and a gofmt pass, and
unless each directory holds nothing else. The counts cover only the files
a rewrite names, so a second test parses every copied file and fails on
any reference to `time.Now`, `time.Since`, `time.Until` or
`runtime.NumCPU`, and on any import of `crypto/rand`, `math/rand`,
`math/rand/v2` or an unrewritten `github.com/decred/dcrd/crypto/rand`
(the one exception: `rand/uniform.go` imports the standard `crypto/rand`
for `rand.Int` with the seeded generator as its reader). Neither test can
see the unmodified dependencies the copies call; the header of
`../copies_test.go` lists the hand audit of those at the pin, which a pin
move repeats. Run them with

```sh
go test ./cmd/oprunner
```

To move the pin, bump `go.mod`, regenerate the copies with
`PURSE_REGEN_COPIES=1 go test ./cmd/oprunner -run TestCopiesAreVerbatim`,
review the diff and any count the test reports as changed (a new clock read
or randomness draw upstream), repeat the dependency audit, and run the
tests again.
