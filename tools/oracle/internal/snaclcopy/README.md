# snaclcopy

`snacl.go` is a verbatim copy of dcrwallet's `wallet/internal/snacl/snacl.go`
at the parity pin (the `decred.org/dcrwallet/v5` version `go.mod` requires),
with only its package clause changed from `snacl` to `snaclcopy`. It keeps
its upstream copyright and license header.

The original is an internal package, which Go refuses to import from outside
dcrwallet's module, so the snacl oracle (`cmd/snacl`) links this copy
instead. Nothing here is reimplemented: `verbatim_test.go` fails unless the
copy is byte-identical, apart from the package clause, to the file in the
module cache that `go.mod` pins. Run it with

```sh
go test ./internal/snaclcopy
```

To move the pin, bump `go.mod`, replace `snacl.go` with the new upstream file
(changing only its package clause) and run the test.
