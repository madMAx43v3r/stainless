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
kept in RAM and rebuilt from the append-only log at startup. The index is a
Stainless `Map<tuple<Vec<u8>, u32>, IndexEntry>`, backed by Rust's ordered
`BTreeMap`. Its compound key is the sole version metadata and orders each
logical key by version; another write to the same key in one version replaces
that version's location. `with_last_in_range()` selects the greatest entry in
the inclusive `(key, 0)..=(key, current_version)` range through a non-escaping
callback, so lookups neither scan nor clone the index. An internal
`rwlock<StoreState>` lets multiple lookups run concurrently while mutations,
commits, reverts, and recovery use an exclusive write guard.

The WAL stores variable-length encoded key and value bytes. `IndexEntry` and
the WAL header store value lengths as `u32`, limiting each encoded value to
`u32::MAX` bytes. Oversized writes fail before a record is written. The
Rust-facing `Table<K, V>` remains statically typed through two explicit traits:

- `Codec` defines the stable persistent representation of a key or value.
- `OrderedKey: Codec + Ord` additionally guarantees that encoded byte order
  preserves the source key order.

Built-in implementations cover booleans, fixed-width integers, `String`,
`Vec<u8>`, and two-/three-element tuples. Applications can implement `Codec`
for their own structs. Compound tuple keys use escaped, self-delimiting
segments so their encoded order is lexicographic.

Keys and application values cross the persistence boundary as bytes because
arbitrary Rust `Codec` implementations can fail. The Rust `Table<K, V>` facade
performs that fallible conversion and keeps the Stainless WAL engine
independent of application-specific codecs.

```rust
use stainless_kvstore::Table;

let store = Table::<(u32, String), String>::open("users.db")?;
store.insert((7, "alice".into()), "active".into())?;
store.commit(1)?;
assert_eq!(store.find(&(7, "alice".into()))?, Some("active".into()));
# Ok::<(), stainless_kvstore::Error>(())
```

Multiple threads can read values concurrently from the same open file handle
without sharing a cursor or reopening the path.

Run the end-to-end showcase with:

```sh
cargo test -p stainless-kvstore
```

This initial crate deliberately uses an append-only log. Immutable sorted
blocks, sparse indexes, and compaction can be layered onto the same versioned
record/revert model without changing its public semantics.
