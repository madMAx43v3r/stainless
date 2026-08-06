# stainless-kvstore

This crate is the first nontrivial Stainless showcase: an append-only,
versioned key/value store whose implementation lives in
[`src/kvstore.stl`](src/kvstore.stl). Its end-to-end Stainless tests live
separately in [`test/kvstore_test.stl`](test/kvstore_test.stl).

Every inserted value is tagged with the current write version. The requested
path stores the data WAL and `<path>.index` stores a separate compact index
WAL. `commit(next)` syncs the data WAL before it appends and syncs an index
commit marker carrying the committed data length. Startup validates the index
record framing and checksums, restores the last committed state, and truncates
incomplete or uncommitted tails in both files.
Index records use kind-specific layouts: inserts store only their version,
value location, and key, while commit markers store only their version and
committed data-WAL length. Internal `detail::InsertRecord` and
`detail::CommitRecord` own their exact `read()` and `write()` implementations;
there is no union-like decoded record with inactive fields.
Every fixed-width WAL integer is big-endian. Record-size headers are `u32`,
while persistent file offsets and committed file lengths are `u64`. The
Stainless implementation delegates these fields to Rust's optimized
fixed-width byte conversions through
`stainless::BigEndian`. WAL record discriminants are grouped as typed
`static const u8` members of `RecordKind`, so their byte representation remains
explicit without consuming storage in a `RecordKind` value.

The store owns one read/write `File` handle for each WAL. Its key-ordered index
and version metadata are kept in RAM and rebuilt from the compact index WAL at
startup without reading every stored value. The RAM index is a Stainless
`Map<tuple<Vec<u8>, u32>, IndexEntry>`, backed by Rust's ordered `BTreeMap`.
Its compound key orders each logical key by version; another write to the same
key in one version replaces that version's location. `with_last_in_range()`
selects the greatest entry in the inclusive
`(key, 0)..=(key, current_version)` range through a non-escaping callback, so
lookups neither scan nor clone the index.

`find_range(lower, upper)` returns the latest visible value for every logical
key in the inclusive interval, ordered from low to high. It walks only the
matching compound-index interval and coalesces historical versions while
holding one shared state guard. `find_range_last(lower, upper, count)` returns
at most the last `count` logical keys in reverse order. The symmetric
`find_range_first(lower, upper, count)` returns at most the first `count` keys
in ascending order. The bounded calls perform ordered predecessor or successor
lookups per result, so finding a small prefix or suffix does not scan the
entire range.

Recovery additionally builds an in-memory `Map<u32, u64>` version index mapping
each committed version to its commit-record offset. `revert(version)` uses an
exact entry or the least successor entry to seek directly to the branch
boundary, then reads the discarded suffix forward. It collects those exact
`(key, version)` pairs, durably truncates the index WAL, and removes only those
pairs from the RAM map. It then truncates the data WAL to the data length carried
by the selected commit record. Revert never calls `Map::retain()` or performs
reverse WAL reads, and its work is proportional to recent discarded index
records rather than the total index size. An internal `rwlock<StoreState>` lets
multiple lookups run concurrently while mutations, commits, reverts, and
recovery use an exclusive write guard.

The data WAL stores variable-length encoded key and value bytes. `IndexEntry`
and the WAL headers store value lengths as `u32`, limiting each encoded value to
`u32::MAX` bytes. Oversized writes fail before a record is written. The
storage layers implemented in `kvstore.stl` are:

- `RawTable`, the byte-oriented WAL and ordered-index engine.
- `Table<K, V>`, a typed Stainless layer initialized with key/value encode and
  decode callbacks.
- `JsonTable<K>`, which uses the same callbacks for keys and stores `var`
  values as compact JSON by default.
- `JsonTable1<K>`, `JsonTable2<K1, K2>`, and `JsonTable3<K1, K2, K3>`, which
  compose one, two, or three ordered key codecs.

The callback result structs carry an explicit validity bit. A codec failure is
reported as checked `kvstore::CodecError`; it is never silently replaced with
a default value. `KeyCodec<T>` decoders accept an offset and return the next
offset, allowing composite keys to be concatenated without intermediate
slices. The built-in unsigned fixed-width key codecs use the exact-type
`stainless::BigEndian::write()` and `read()` overloads, so byte order preserves
unsigned integer order.

`codecs::string_key()` encodes Rust `String` values as self-delimiting UTF-8.
Its zero-byte escaping preserves normal string ordering, including prefixes
and embedded NUL characters, and lets strings safely occupy any position in a
compound key. Decoding rejects invalid UTF-8 as an ordinary codec failure.

Calling `vec()` on any `KeyCodec<T>` derives a `KeyCodec<Vec<T>>`:

```cpp
KeyCodec<Vec<u32>> path_codec = codecs::u32_key().vec();
KeyCodec<Vec<Vec<u32>>> nested_path_codec = path_codec.vec();
```

The vector codec escapes reserved zero bytes and writes distinct element and
vector terminators. This keeps the encoding self-delimiting for compound keys
while preserving lexicographic `Vec<T>` order, including empty vectors,
prefixes, and nested vectors.

For example, this Stainless code creates a two-part key with JSON values:

```cpp
JsonTable2<u32, u64> users = JsonTable2<u32, u64>(
    "users.db",
    codecs::u32_key(),
    codecs::u64_key());

var profile = {name: "alice", active: true};
users.insert(7, 42, profile);
users.commit(1);

var loaded = var();
if (users.find(7, 42, loaded)) {
    println!("{}", loaded.to_json());
}
```

Supplying all four callbacks to `Table<K, V>` selects a custom persistent
representation for both keys and values; the compiler does not synthesize
serialization code for the table.

The Rust-facing `Table<K, V>` remains statically typed through two explicit traits:

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
let rows = store.find_range(&(7, "a".into()), &(7, "z".into()))?;
let first_rows = store.find_range_first(&(7, "a".into()), &(7, "z".into()), 10)?;
let last_rows = store.find_range_last(&(7, "a".into()), &(7, "z".into()), 10)?;
# Ok::<(), stainless_kvstore::Error>(())
```

Multiple threads can read values concurrently from the same open file handle
without sharing a cursor or reopening the path.

Run the end-to-end showcase with:

```sh
cargo test -p stainless-kvstore
```

This initial crate deliberately uses append-and-truncate WALs. Immutable
sorted blocks, sparse indexes, and compaction can be layered onto the same
versioned model without changing its public semantics.
