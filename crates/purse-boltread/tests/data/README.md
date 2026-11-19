# Test data

- `upstream/`: dcrwallet's own fixture databases (see `upstream/README.md`).
- `frozen-4k.db`: a small database written by Go bbolt v1.3.12 through the
  oracle (`tools/oracle/cmd/boltdump`), used by the corruption tests, which
  run without Go. It is a valid bbolt file exactly as bbolt left it
  (262144 bytes; 4096-byte pages; high-water mark 59 pages; newest meta is
  page 1 at txid 5, the older page 0 at txid 4). It holds regular, inline
  and empty nested buckets, buckets with nonzero sequences, a bucket tree
  with a branch page below a branch page, leaves with overflow pages, and a
  freelist with 25 pages.
- `frozen-deep-4k.db`: a second one, 262144 bytes, whose one regular bucket
  is a tree of four levels (three levels of branch pages over the leaves),
  plus an inline bucket; newest meta is page 1 at txid 3.
- `dump-digests.txt`: the Go oracle's canonical dump (txid, entry count,
  SHA-256) of each fixture, and of `frozen-4k.db` with meta page 1
  invalidated (`frozen-4k.db@meta0`: the older snapshot bbolt falls back
  to).
- `bucket-stats.json`: Go's `BucketStats` of every bucket of every fixture,
  keyed by file and by bucket path in the dump's notation.

The Go-free tests compare against the last two files; the oracle tests
check that they still match Go.

`frozen-4k.db` was made with the oracle's `generate` command, seed 4, and
this recipe:

```json
{"page_size":4096,"commits":4,"top_buckets":2,"depth":1,"sub_buckets":1,
 "keys":[3,18],"key_size":[600,1300],"value_size":[0,40],
 "large_permille":40,"large_size":[4500,7000],"inline_buckets":4,
 "empty_buckets":2,"add_percent":40,"delete_percent":20,
 "overwrite_percent":10,"drop_percent":30,"seq_percent":30}
```

that is, as one line on the oracle's stdin:

```
{"cmd":"generate","args":{"path":"frozen-4k.db","seed":4,"recipe":<the recipe>}}
```

`frozen-deep-4k.db` was made the same way with seed 3 and:

```json
{"page_size":4096,"commits":2,"top_buckets":1,"depth":0,"sub_buckets":0,
 "keys":[42,46],"key_size":[1250,1350],"value_size":[0,8],
 "inline_buckets":1,"add_percent":5,"delete_percent":10,
 "overwrite_percent":5}
```

The content is deterministic, but bbolt spills sibling buckets in Go map
order, so a regenerated file can place pages differently; its dump digest
is what must match. If a file is ever replaced, `dump-digests.txt` and
`bucket-stats.json` must be regenerated from Go (the oracle's `dump` and
`stats` commands), and the targeted mutations in `tests/corrupt.rs` need,
in `frozen-4k.db`: a branch page below a branch page, a bucket rooted at a
branch over branches, a leftmost branch over at least three leaves, a leaf
with overflow pages, a leaf of plain pairs, a non-empty inline bucket with
an element whose value is shorter than 16 bytes, a regular nested bucket,
and a newest meta on page 1.
