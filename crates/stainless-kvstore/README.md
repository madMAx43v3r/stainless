# stainless-kvstore

This crate is the first nontrivial Stainless showcase: an append-only,
versioned key/value store whose implementation lives in
[`src/kvstore.stl`](src/kvstore.stl).

Every inserted value is tagged with the current write version. `commit(next)`
syncs an explicit commit record before advancing the version, while
`revert(version)` syncs a revert record before removing index entries at or
above that boundary. Startup validates record lengths and checksums, restores
the last committed/reverted state, and truncates incomplete or uncommitted log
tail data.

The store owns one read/write `File` handle. Its index and version metadata are
kept in RAM and rebuilt from the append-only log at startup. An internal
`rwlock<StoreState>` lets lookups scan that index concurrently. A lookup holds
its shared read guard through `File.pread_exact()` on the shared handle, so the
entire read is synchronized against mutations without cloning the index.
Mutations, commits, reverts, and recovery use an exclusive write guard.

The current index is a linked list, so lookup is still O(number of entries),
but it performs no whole-index allocation or copy. Multiple threads can read
values concurrently without reopening the path and without contending on a
file cursor.

Run the end-to-end showcase with:

```sh
cargo test -p stainless-kvstore
```

This initial crate deliberately uses an append-only log. Immutable sorted
blocks, sparse indexes, and compaction can be layered onto the same versioned
record/revert model without changing its public semantics.
