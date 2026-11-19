# Upstream dcrwallet fixture databases

These files are copied byte for byte, still gzipped, from
`wallet/udb/testdata/` of [dcrwallet](https://github.com/decred/dcrwallet) at
commit `54b85b336077587eaf20112dc695355ce06fc76e` (the dcr-purse parity pin):

| File | Database version |
| --- | --- |
| `v1.db.gz` | 1 |
| `v2.db.gz` | 2 |
| `v3.db.gz` | 3 |
| `v4.db.gz` | 4 |
| `v5.db.gz` | 5 |
| `v7.db.gz` | 7 |
| `v11.db.gz` | 11 |
| `v24.db.gz` | 24 |

Each is a bbolt v1.3.12-compatible `wallet.db` written by the generator next
to it upstream (not copied): `v<N>.db.go` for every version except 7, whose
generator is `v7.go`. The tests decompress them into
cargo's target tmpdir; their canonical dumps, as the Go oracle writes them,
are recorded in `../dump-digests.txt`, and Go's `BucketStats` of every
bucket in `../bucket-stats.json`.

They are distributed under dcrwallet's license:

```
Copyright (c) 2013-2016 The btcsuite developers
Copyright (c) 2015-2020 The Decred developers

Permission to use, copy, modify, and distribute this software for any
purpose with or without fee is hereby granted, provided that the above
copyright notice and this permission notice appear in all copies.

THE SOFTWARE IS PROVIDED "AS IS" AND THE AUTHOR DISCLAIMS ALL WARRANTIES
WITH REGARD TO THIS SOFTWARE INCLUDING ALL IMPLIED WARRANTIES OF
MERCHANTABILITY AND FITNESS. IN NO EVENT SHALL THE AUTHOR BE LIABLE FOR
ANY SPECIAL, DIRECT, INDIRECT, OR CONSEQUENTIAL DAMAGES OR ANY DAMAGES
WHATSOEVER RESULTING FROM LOSS OF USE, DATA OR PROFITS, WHETHER IN AN
ACTION OF CONTRACT, NEGLIGENCE OR OTHER TORTIOUS ACTION, ARISING OUT OF
OR IN CONNECTION WITH THE USE OR PERFORMANCE OF THIS SOFTWARE.
```
