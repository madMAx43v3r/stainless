//! Compact runtime support for generated Stainless programs.
//!
//! Most Stainless values lower directly to Rust. JSON is the deliberate
//! exception: [`Var`] provides the language's dynamically typed JSON value
//! while delegating parsing and serialization to `serde_json`.

use std::collections::{BTreeMap, BTreeSet, LinkedList, VecDeque};
use std::error::Error;
use std::fmt;
use std::fs::{self, File};
use std::io::BufReader;
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard, RwLock, RwLockReadGuard, RwLockWriteGuard};

use serde_json::{Number, Value};

/// Compiler-owned storage for one embedded Stainless class base.
///
/// The base has one owner while the complete derived value is mutably
/// accessible. A derived-to-base shared-owner conversion clones the inner
/// [`Arc`], after which Stainless permits only shared access to the derived
/// object as well.
#[doc(hidden)]
pub struct ClassBase<T>(Arc<T>);

impl<T> ClassBase<T> {
    /// Creates one independently owned base subobject.
    pub fn new(value: T) -> Self {
        Self(Arc::new(value))
    }

    /// Produces the representation of `shared_ptr<Base>`.
    #[must_use]
    pub fn share(&self) -> Arc<T> {
        Arc::clone(&self.0)
    }
}

impl<T> std::ops::Deref for ClassBase<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T> std::ops::DerefMut for ClassBase<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        Arc::get_mut(&mut self.0)
            .expect("a mutably borrowed derived class cannot have an aliased base subobject")
    }
}

/// Runtime crate source directory used by `stainlessc` for its hidden Cargo
/// build. Packaged tools fall back to the crates.io version if this directory
/// is no longer present.
#[doc(hidden)]
pub const CRATE_SOURCE_DIR: &str = env!("CARGO_MANIFEST_DIR");

/// Operating-system entropy exposed to generated Stainless code.
pub struct Random;

impl Random {
    /// Maximum allocation accepted by [`Self::bytes`].
    pub const MAX_BYTES: usize = 1024 * 1024;

    /// Returns `length` bytes from the operating system's random source.
    ///
    /// # Errors
    ///
    /// Returns [`RandomError`] when `length` exceeds [`Self::MAX_BYTES`] or
    /// the operating system cannot provide entropy.
    pub fn bytes(length: usize) -> Result<Vec<u8>, RandomError> {
        if length > Self::MAX_BYTES {
            return Err(RandomError::new(format!(
                "requested {length} random bytes, maximum is {}",
                Self::MAX_BYTES
            )));
        }
        let mut bytes = vec![0; length];
        getrandom::fill(&mut bytes)
            .map_err(|error| RandomError::new(format!("randomness unavailable: {error}")))?;
        Ok(bytes)
    }
}

/// Failure to obtain a bounded random-byte buffer from the operating system.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RandomError {
    message: String,
}

impl RandomError {
    fn new(message: String) -> Self {
        Self { message }
    }
}

impl fmt::Display for RandomError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl Error for RandomError {}

/// Invokes `callback` with a shared map value when `key` exists.
///
/// This confines the borrow to one non-escaping callback instead of exposing
/// `Option<&V>` as a storable Stainless type.
pub fn btree_map_with<K, V, F>(map: &BTreeMap<K, V>, key: &K, callback: F) -> bool
where
    K: Ord,
    F: FnOnce(&V),
{
    let Some(value) = map.get(key) else {
        return false;
    };
    callback(value);
    true
}

/// Invokes `callback` with a mutable map value when `key` exists.
///
/// The reference cannot escape the callback, preserving `BTreeMap`'s key and
/// value borrowing rules at the Stainless boundary.
pub fn btree_map_with_mut<K, V, F>(map: &mut BTreeMap<K, V>, key: &K, callback: F) -> bool
where
    K: Ord,
    F: FnOnce(&mut V),
{
    let Some(value) = map.get_mut(key) else {
        return false;
    };
    callback(value);
    true
}

/// Invokes `callback` with the greatest entry inside an inclusive key range.
pub fn btree_map_with_last_in_range<K, V, F>(
    map: &BTreeMap<K, V>,
    lower: &K,
    upper: &K,
    callback: F,
) -> bool
where
    K: Ord,
    F: FnOnce(&K, &V),
{
    if lower > upper {
        return false;
    }
    let Some((key, value)) = map.range(lower..=upper).next_back() else {
        return false;
    };
    callback(key, value);
    true
}

/// Invokes `callback` with the least entry inside an inclusive key range.
pub fn btree_map_with_first_in_range<K, V, F>(
    map: &BTreeMap<K, V>,
    lower: &K,
    upper: &K,
    callback: F,
) -> bool
where
    K: Ord,
    F: FnOnce(&K, &V),
{
    if lower > upper {
        return false;
    }
    let Some((key, value)) = map.range(lower..=upper).next() else {
        return false;
    };
    callback(key, value);
    true
}

/// Visits every entry in an inclusive key range in ascending order.
pub fn btree_map_with_range<K, V, F>(
    map: &BTreeMap<K, V>,
    lower: &K,
    upper: &K,
    mut callback: F,
) -> usize
where
    K: Ord,
    F: FnMut(&K, &V),
{
    if lower > upper {
        return 0;
    }
    let mut count = 0;
    for (key, value) in map.range(lower..=upper) {
        callback(key, value);
        count += 1;
    }
    count
}

/// Invokes `callback` with the greatest entry at or above `lower` and strictly
/// below `upper`.
pub fn btree_map_with_last_before<K, V, F>(
    map: &BTreeMap<K, V>,
    lower: &K,
    upper: &K,
    callback: F,
) -> bool
where
    K: Ord,
    F: FnOnce(&K, &V),
{
    if lower >= upper {
        return false;
    }
    let Some((key, value)) = map.range(lower..upper).next_back() else {
        return false;
    };
    callback(key, value);
    true
}

/// Invokes `callback` with the least entry strictly above `lower` and at or
/// below `upper`.
pub fn btree_map_with_first_after<K, V, F>(
    map: &BTreeMap<K, V>,
    lower: &K,
    upper: &K,
    callback: F,
) -> bool
where
    K: Ord,
    F: FnOnce(&K, &V),
{
    if lower >= upper {
        return false;
    }
    let Some((key, value)) = map
        .range((
            std::ops::Bound::Excluded(lower),
            std::ops::Bound::Included(upper),
        ))
        .next()
    else {
        return false;
    };
    callback(key, value);
    true
}

/// Retains map entries accepted by a read-only, non-escaping callback.
pub fn btree_map_retain<K, V, F>(map: &mut BTreeMap<K, V>, mut predicate: F)
where
    K: Ord,
    F: FnMut(&K, &V) -> bool,
{
    map.retain(|key, value| predicate(key, value));
}

/// Retains map entries whose keys are accepted by a read-only, non-escaping
/// callback without exposing values that the predicate does not need.
pub fn btree_map_retain_keys<K, V, F>(map: &mut BTreeMap<K, V>, mut predicate: F)
where
    K: Ord,
    F: FnMut(&K) -> bool,
{
    map.retain(|key, _| predicate(key));
}

/// Visits a half-open vector range without exposing a storable slice or
/// allocating a temporary vector. Returns `false` for an invalid range.
pub fn vec_with_range<T, F>(values: &[T], begin: usize, end: usize, mut callback: F) -> bool
where
    F: FnMut(&T),
{
    let Some(values) = values.get(begin..end) else {
        return false;
    };
    for value in values {
        callback(value);
    }
    true
}

/// Copies a checked half-open vector range into a new owned vector.
///
/// # Panics
///
/// Panics with Rust's normal range semantics when the bounds are invalid.
pub fn vec_copy_range<T: Clone>(values: &[T], begin: usize, end: usize) -> Vec<T> {
    values
        .get(begin..end)
        .expect("Stainless Vec::copy_range bounds are invalid")
        .to_vec()
}

/// An ordered multimap of independently iterable key/value associations.
///
/// Stainless exposes this as `rust::MultiMap<K, V>`. Its private buckets use
/// the same linked representation as Stainless `List<V>` while its public API
/// presents duplicate key/value entries, never nested collections.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
#[allow(clippy::linkedlist)]
pub struct MultiMap<K, V> {
    entries: BTreeMap<K, LinkedList<V>>,
    len: usize,
}

/// Consuming iterator for [`MultiMap`].
///
/// A key is cloned for each yielded association because one stored key can
/// own multiple independently yielded values.
#[allow(clippy::linkedlist)]
pub struct MultiMapIntoIter<K, V> {
    entries: std::collections::btree_map::IntoIter<K, LinkedList<V>>,
    current_key: Option<K>,
    current_values: std::collections::linked_list::IntoIter<V>,
}

impl<K: Clone, V> Iterator for MultiMapIntoIter<K, V> {
    type Item = (K, V);

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some(value) = self.current_values.next() {
                let key = self
                    .current_key
                    .as_ref()
                    .expect("a current multimap value always has a key")
                    .clone();
                return Some((key, value));
            }
            let (key, values) = self.entries.next()?;
            self.current_key = Some(key);
            self.current_values = values.into_iter();
        }
    }
}

impl<K: Clone, V> IntoIterator for MultiMap<K, V> {
    type Item = (K, V);
    type IntoIter = MultiMapIntoIter<K, V>;

    fn into_iter(self) -> Self::IntoIter {
        MultiMapIntoIter {
            entries: self.entries.into_iter(),
            current_key: None,
            current_values: LinkedList::new().into_iter(),
        }
    }
}

impl<K, V> MultiMap<K, V> {
    /// Creates an empty multimap.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            entries: BTreeMap::new(),
            len: 0,
        }
    }

    /// Returns the number of key/value associations.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.len
    }

    /// Returns the number of distinct keys.
    #[must_use]
    pub fn key_len(&self) -> usize {
        self.entries.len()
    }

    /// Returns whether the multimap has no key/value associations.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Removes every association.
    pub fn clear(&mut self) {
        self.entries.clear();
        self.len = 0;
    }

    /// Returns an iterator in ascending key and per-key insertion order.
    pub fn iter(&self) -> impl Iterator<Item = (&K, &V)> {
        self.entries
            .iter()
            .flat_map(|(key, values)| values.iter().map(move |value| (key, value)))
    }

    /// Returns a mutable value iterator in ascending key and per-key insertion
    /// order. Keys stay immutable so their ordering cannot be invalidated.
    pub fn iter_mut(&mut self) -> impl Iterator<Item = (&K, &mut V)> {
        self.entries
            .iter_mut()
            .flat_map(|(key, values)| values.iter_mut().map(move |value| (key, value)))
    }
}

impl<K: Ord, V> MultiMap<K, V> {
    /// Appends one association after the existing values for `key`.
    pub fn insert(&mut self, key: K, value: V) {
        self.entries.entry(key).or_default().push_back(value);
        self.len += 1;
    }

    /// Returns whether at least one value is associated with `key`.
    #[must_use]
    pub fn contains_key(&self, key: &K) -> bool {
        self.entries.contains_key(key)
    }

    /// Invokes `callback` once per association for `key` and returns its count.
    pub fn with<F>(&self, key: &K, mut callback: F) -> usize
    where
        F: FnMut(&V),
    {
        let Some(values) = self.entries.get(key) else {
            return 0;
        };
        for value in values {
            callback(value);
        }
        values.len()
    }

    /// Invokes `callback` mutably once per association for `key` and returns
    /// its count.
    pub fn with_mut<F>(&mut self, key: &K, mut callback: F) -> usize
    where
        F: FnMut(&mut V),
    {
        let Some(values) = self.entries.get_mut(key) else {
            return 0;
        };
        let count = values.len();
        for value in values {
            callback(value);
        }
        count
    }

    /// Removes every association for `key` and returns how many were removed.
    pub fn remove_all(&mut self, key: &K) -> usize {
        let removed = self.entries.remove(key).map_or(0, |values| values.len());
        self.len -= removed;
        removed
    }

    /// Removes the first association whose key and value both match.
    pub fn remove(&mut self, key: &K, value: &V) -> bool
    where
        V: PartialEq,
    {
        let Some(values) = self.entries.get_mut(key) else {
            return false;
        };
        let mut retained = LinkedList::new();
        let mut removed = false;
        while let Some(candidate) = values.pop_front() {
            if !removed && candidate == *value {
                removed = true;
            } else {
                retained.push_back(candidate);
            }
        }
        let empty = retained.is_empty();
        *values = retained;
        if removed {
            self.len -= 1;
        }
        if empty {
            self.entries.remove(key);
        }
        removed
    }

    /// Retains associations accepted by `predicate` and removes empty keys.
    pub fn retain<F>(&mut self, mut predicate: F)
    where
        F: FnMut(&K, &V) -> bool,
    {
        let mut retained_len = 0;
        self.entries.retain(|key, values| {
            let mut retained = LinkedList::new();
            while let Some(value) = values.pop_front() {
                if predicate(key, &value) {
                    retained.push_back(value);
                    retained_len += 1;
                }
            }
            *values = retained;
            !values.is_empty()
        });
        self.len = retained_len;
    }
}

/// Fixed-width big-endian integer encoding for byte vectors.
///
/// Stainless exposes these operations as `stainless::BigEndian`. They delegate
/// to Rust's optimized `to_be_bytes()` and `from_be_bytes()` implementations.
const _: () = assert!(std::mem::size_of::<usize>() <= std::mem::size_of::<u64>());

pub struct BigEndian;

impl BigEndian {
    /// Writes one byte at the end of `output`.
    pub fn write_u8(output: &mut Vec<u8>, value: u8) {
        output.push(value);
    }

    /// Writes one `u16` in network byte order at the end of `output`.
    pub fn write_u16(output: &mut Vec<u8>, value: u16) {
        output.extend_from_slice(&value.to_be_bytes());
    }

    /// Writes one `u32` in network byte order at the end of `output`.
    pub fn write_u32(output: &mut Vec<u8>, value: u32) {
        output.extend_from_slice(&value.to_be_bytes());
    }

    /// Writes one `u64` in network byte order at the end of `output`.
    pub fn write_u64(output: &mut Vec<u8>, value: u64) {
        output.extend_from_slice(&value.to_be_bytes());
    }

    /// Writes one `u128` in network byte order at the end of `output`.
    pub fn write_u128(output: &mut Vec<u8>, value: u128) {
        output.extend_from_slice(&value.to_be_bytes());
    }

    /// Writes a `usize` as one checked big-endian `u32`.
    ///
    /// # Errors
    ///
    /// Returns `InvalidInput` when `value` does not fit in `u32`.
    pub fn write_usize_u32(output: &mut Vec<u8>, value: usize) -> std::io::Result<()> {
        let value = u32::try_from(value).map_err(|_| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("value {value} does not fit in a big-endian u32 field"),
            )
        })?;
        Self::write_u32(output, value);
        Ok(())
    }

    /// Writes a `usize` as one big-endian `u64`.
    pub fn write_usize_u64(output: &mut Vec<u8>, value: usize) {
        Self::write_u64(output, value as u64);
    }

    /// Decodes one byte.
    ///
    /// # Errors
    ///
    /// Returns `InvalidData` unless `bytes` contains exactly one byte.
    pub fn read_u8(bytes: &[u8]) -> std::io::Result<u8> {
        let [value] = bytes else {
            return Err(invalid_integer_width("big-endian", "u8", 1, bytes.len()));
        };
        Ok(*value)
    }

    /// Decodes one byte at `offset` without allocating a subslice.
    ///
    /// # Errors
    ///
    /// Returns `InvalidData` when `offset` is outside `bytes`.
    pub fn read_u8_at(bytes: &[u8], offset: usize) -> std::io::Result<u8> {
        bytes
            .get(offset)
            .copied()
            .ok_or_else(|| invalid_integer_offset("big-endian", "u8", offset, 1, bytes.len()))
    }

    /// Decodes one big-endian `u16`.
    ///
    /// # Errors
    ///
    /// Returns `InvalidData` unless `bytes` contains exactly two bytes.
    pub fn read_u16(bytes: &[u8]) -> std::io::Result<u16> {
        let bytes = bytes
            .try_into()
            .map_err(|_| invalid_integer_width("big-endian", "u16", 2, bytes.len()))?;
        Ok(u16::from_be_bytes(bytes))
    }

    /// Decodes one big-endian `u16` at `offset`.
    ///
    /// # Errors
    ///
    /// Returns `InvalidData` when two bytes are not available at `offset`.
    pub fn read_u16_at(bytes: &[u8], offset: usize) -> std::io::Result<u16> {
        let end = offset
            .checked_add(2)
            .ok_or_else(|| invalid_integer_offset("big-endian", "u16", offset, 2, bytes.len()))?;
        Self::read_u16(
            bytes.get(offset..end).ok_or_else(|| {
                invalid_integer_offset("big-endian", "u16", offset, 2, bytes.len())
            })?,
        )
    }

    /// Decodes one big-endian `u32`.
    ///
    /// # Errors
    ///
    /// Returns `InvalidData` unless `bytes` contains exactly four bytes.
    pub fn read_u32(bytes: &[u8]) -> std::io::Result<u32> {
        let bytes = bytes
            .try_into()
            .map_err(|_| invalid_integer_width("big-endian", "u32", 4, bytes.len()))?;
        Ok(u32::from_be_bytes(bytes))
    }

    /// Decodes one big-endian `u32` at `offset`.
    ///
    /// # Errors
    ///
    /// Returns `InvalidData` when four bytes are not available at `offset`.
    pub fn read_u32_at(bytes: &[u8], offset: usize) -> std::io::Result<u32> {
        let end = offset
            .checked_add(4)
            .ok_or_else(|| invalid_integer_offset("big-endian", "u32", offset, 4, bytes.len()))?;
        Self::read_u32(
            bytes.get(offset..end).ok_or_else(|| {
                invalid_integer_offset("big-endian", "u32", offset, 4, bytes.len())
            })?,
        )
    }

    /// Decodes one big-endian `u64`.
    ///
    /// # Errors
    ///
    /// Returns `InvalidData` unless `bytes` contains exactly eight bytes.
    pub fn read_u64(bytes: &[u8]) -> std::io::Result<u64> {
        let bytes = bytes
            .try_into()
            .map_err(|_| invalid_integer_width("big-endian", "u64", 8, bytes.len()))?;
        Ok(u64::from_be_bytes(bytes))
    }

    /// Decodes one big-endian `u64` at `offset`.
    ///
    /// # Errors
    ///
    /// Returns `InvalidData` when eight bytes are not available at `offset`.
    pub fn read_u64_at(bytes: &[u8], offset: usize) -> std::io::Result<u64> {
        let end = offset
            .checked_add(8)
            .ok_or_else(|| invalid_integer_offset("big-endian", "u64", offset, 8, bytes.len()))?;
        Self::read_u64(
            bytes.get(offset..end).ok_or_else(|| {
                invalid_integer_offset("big-endian", "u64", offset, 8, bytes.len())
            })?,
        )
    }

    /// Decodes one big-endian `u128`.
    ///
    /// # Errors
    ///
    /// Returns `InvalidData` unless `bytes` contains exactly sixteen bytes.
    pub fn read_u128(bytes: &[u8]) -> std::io::Result<u128> {
        let bytes = bytes
            .try_into()
            .map_err(|_| invalid_integer_width("big-endian", "u128", 16, bytes.len()))?;
        Ok(u128::from_be_bytes(bytes))
    }

    /// Decodes one big-endian `u128` at `offset`.
    ///
    /// # Errors
    ///
    /// Returns `InvalidData` when sixteen bytes are not available at `offset`.
    pub fn read_u128_at(bytes: &[u8], offset: usize) -> std::io::Result<u128> {
        let end = offset
            .checked_add(16)
            .ok_or_else(|| invalid_integer_offset("big-endian", "u128", offset, 16, bytes.len()))?;
        Self::read_u128(
            bytes.get(offset..end).ok_or_else(|| {
                invalid_integer_offset("big-endian", "u128", offset, 16, bytes.len())
            })?,
        )
    }

    /// Reads one byte into `output` and advances `offset`.
    ///
    /// # Errors
    ///
    /// Returns `InvalidData` when no byte is available at `offset`.
    pub fn read_u8_advance(
        bytes: &[u8],
        offset: &mut usize,
        output: &mut u8,
    ) -> std::io::Result<()> {
        *output = Self::read_u8_at(bytes, *offset)?;
        *offset += 1;
        Ok(())
    }

    /// Reads one big-endian `u16` into `output` and advances `offset`.
    ///
    /// # Errors
    ///
    /// Returns `InvalidData` when two bytes are not available at `offset`.
    pub fn read_u16_advance(
        bytes: &[u8],
        offset: &mut usize,
        output: &mut u16,
    ) -> std::io::Result<()> {
        *output = Self::read_u16_at(bytes, *offset)?;
        *offset += 2;
        Ok(())
    }

    /// Reads one big-endian `u32` into `output` and advances `offset`.
    ///
    /// # Errors
    ///
    /// Returns `InvalidData` when four bytes are not available at `offset`.
    pub fn read_u32_advance(
        bytes: &[u8],
        offset: &mut usize,
        output: &mut u32,
    ) -> std::io::Result<()> {
        *output = Self::read_u32_at(bytes, *offset)?;
        *offset += 4;
        Ok(())
    }

    /// Reads one big-endian `u64` into `output` and advances `offset`.
    ///
    /// # Errors
    ///
    /// Returns `InvalidData` when eight bytes are not available at `offset`.
    pub fn read_u64_advance(
        bytes: &[u8],
        offset: &mut usize,
        output: &mut u64,
    ) -> std::io::Result<()> {
        *output = Self::read_u64_at(bytes, *offset)?;
        *offset += 8;
        Ok(())
    }

    /// Reads one big-endian `u128` into `output` and advances `offset`.
    ///
    /// # Errors
    ///
    /// Returns `InvalidData` when sixteen bytes are not available at `offset`.
    pub fn read_u128_advance(
        bytes: &[u8],
        offset: &mut usize,
        output: &mut u128,
    ) -> std::io::Result<()> {
        *output = Self::read_u128_at(bytes, *offset)?;
        *offset += 16;
        Ok(())
    }
}

/// Fixed-width little-endian integer encoding for byte vectors.
///
/// Stainless exposes these operations as `stainless::LittleEndian`. They
/// delegate to Rust's optimized `to_le_bytes()` and `from_le_bytes()`
/// implementations.
pub struct LittleEndian;

impl LittleEndian {
    /// Writes one `u32` in little-endian order at the end of `output`.
    pub fn write_u32(output: &mut Vec<u8>, value: u32) {
        output.extend_from_slice(&value.to_le_bytes());
    }

    /// Writes one `u64` in little-endian order at the end of `output`.
    pub fn write_u64(output: &mut Vec<u8>, value: u64) {
        output.extend_from_slice(&value.to_le_bytes());
    }

    /// Writes a `usize` as one checked little-endian `u32`.
    ///
    /// # Errors
    ///
    /// Returns `InvalidInput` when `value` does not fit in `u32`.
    pub fn write_usize_u32(output: &mut Vec<u8>, value: usize) -> std::io::Result<()> {
        let value = u32::try_from(value).map_err(|_| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("value {value} does not fit in a little-endian u32 field"),
            )
        })?;
        Self::write_u32(output, value);
        Ok(())
    }

    /// Writes a `usize` as one little-endian `u64`.
    pub fn write_usize_u64(output: &mut Vec<u8>, value: usize) {
        Self::write_u64(output, value as u64);
    }

    /// Decodes one byte.
    ///
    /// # Errors
    ///
    /// Returns `InvalidData` unless `bytes` contains exactly one byte.
    pub fn read_u8(bytes: &[u8]) -> std::io::Result<u8> {
        let [value] = bytes else {
            return Err(invalid_integer_width("little-endian", "u8", 1, bytes.len()));
        };
        Ok(*value)
    }

    /// Decodes one byte at `offset` without allocating a subslice.
    ///
    /// # Errors
    ///
    /// Returns `InvalidData` when `offset` is outside `bytes`.
    pub fn read_u8_at(bytes: &[u8], offset: usize) -> std::io::Result<u8> {
        bytes
            .get(offset)
            .copied()
            .ok_or_else(|| invalid_integer_offset("little-endian", "u8", offset, 1, bytes.len()))
    }

    /// Decodes one little-endian `u32`.
    ///
    /// # Errors
    ///
    /// Returns `InvalidData` unless `bytes` contains exactly four bytes.
    pub fn read_u32(bytes: &[u8]) -> std::io::Result<u32> {
        let bytes = bytes
            .try_into()
            .map_err(|_| invalid_integer_width("little-endian", "u32", 4, bytes.len()))?;
        Ok(u32::from_le_bytes(bytes))
    }

    /// Decodes one little-endian `u32` at `offset`.
    ///
    /// # Errors
    ///
    /// Returns `InvalidData` when four bytes are not available at `offset`.
    pub fn read_u32_at(bytes: &[u8], offset: usize) -> std::io::Result<u32> {
        let end = offset.checked_add(4).ok_or_else(|| {
            invalid_integer_offset("little-endian", "u32", offset, 4, bytes.len())
        })?;
        Self::read_u32(bytes.get(offset..end).ok_or_else(|| {
            invalid_integer_offset("little-endian", "u32", offset, 4, bytes.len())
        })?)
    }

    /// Decodes one little-endian `u64`.
    ///
    /// # Errors
    ///
    /// Returns `InvalidData` unless `bytes` contains exactly eight bytes.
    pub fn read_u64(bytes: &[u8]) -> std::io::Result<u64> {
        let bytes = bytes
            .try_into()
            .map_err(|_| invalid_integer_width("little-endian", "u64", 8, bytes.len()))?;
        Ok(u64::from_le_bytes(bytes))
    }

    /// Decodes one little-endian `u64` at `offset`.
    ///
    /// # Errors
    ///
    /// Returns `InvalidData` when eight bytes are not available at `offset`.
    pub fn read_u64_at(bytes: &[u8], offset: usize) -> std::io::Result<u64> {
        let end = offset.checked_add(8).ok_or_else(|| {
            invalid_integer_offset("little-endian", "u64", offset, 8, bytes.len())
        })?;
        Self::read_u64(bytes.get(offset..end).ok_or_else(|| {
            invalid_integer_offset("little-endian", "u64", offset, 8, bytes.len())
        })?)
    }
}

fn invalid_integer_width(
    byte_order: &str,
    name: &str,
    expected: usize,
    actual: usize,
) -> std::io::Error {
    std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        format!("{byte_order} {name} requires {expected} bytes, found {actual}"),
    )
}

fn invalid_integer_offset(
    byte_order: &str,
    name: &str,
    offset: usize,
    width: usize,
    actual: usize,
) -> std::io::Error {
    std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        format!(
            "{byte_order} {name} at offset {offset} requires {width} bytes, buffer has {actual}"
        ),
    )
}

/// Exact-signature facade for the Rust standard library's whole-file and
/// directory operations exposed through Stainless `rust::std::fs` bindings.
///
/// Rust's public functions accept generic `AsRef` parameters. Keeping those
/// generics behind this facade lets Stainless expose deterministic overloads
/// while preserving the original [`std::io::Error`] values.
pub struct Fs;

impl Fs {
    /// Reads an entire UTF-8 file into an owned string.
    ///
    /// # Errors
    ///
    /// Returns the underlying filesystem or UTF-8 decoding error.
    pub fn read_to_string(path: &str) -> std::io::Result<String> {
        fs::read_to_string(path)
    }

    /// Reads an entire file as bytes.
    ///
    /// # Errors
    ///
    /// Returns the underlying filesystem error.
    pub fn read(path: &str) -> std::io::Result<Vec<u8>> {
        fs::read(path)
    }

    /// Creates or truncates a file and writes UTF-8 text into it.
    ///
    /// # Errors
    ///
    /// Returns the underlying filesystem or write error.
    pub fn write_text(path: &str, contents: &str) -> std::io::Result<()> {
        fs::write(path, contents)
    }

    /// Creates or truncates a file and writes bytes into it.
    ///
    /// # Errors
    ///
    /// Returns the underlying filesystem or write error.
    pub fn write_bytes(path: &str, contents: &[u8]) -> std::io::Result<()> {
        fs::write(path, contents)
    }

    /// Returns whether a filesystem entry exists, preserving lookup errors.
    ///
    /// # Errors
    ///
    /// Returns the underlying filesystem lookup error.
    pub fn exists(path: &str) -> std::io::Result<bool> {
        fs::exists(path)
    }

    /// Copies one file and returns the number of bytes copied.
    ///
    /// # Errors
    ///
    /// Returns the underlying filesystem, read, or write error.
    pub fn copy(from: &str, to: &str) -> std::io::Result<u64> {
        fs::copy(from, to)
    }

    /// Renames or moves a filesystem entry.
    ///
    /// # Errors
    ///
    /// Returns the underlying filesystem error.
    pub fn rename(from: &str, to: &str) -> std::io::Result<()> {
        fs::rename(from, to)
    }

    /// Removes one file.
    ///
    /// # Errors
    ///
    /// Returns the underlying filesystem error.
    pub fn remove_file(path: &str) -> std::io::Result<()> {
        fs::remove_file(path)
    }

    /// Creates one directory whose parent already exists.
    ///
    /// # Errors
    ///
    /// Returns the underlying filesystem error.
    pub fn create_dir(path: &str) -> std::io::Result<()> {
        fs::create_dir(path)
    }

    /// Recursively creates a directory and any missing parents.
    ///
    /// # Errors
    ///
    /// Returns the underlying filesystem error.
    pub fn create_dir_all(path: &str) -> std::io::Result<()> {
        fs::create_dir_all(path)
    }

    /// Removes one empty directory.
    ///
    /// # Errors
    ///
    /// Returns the underlying filesystem error.
    pub fn remove_dir(path: &str) -> std::io::Result<()> {
        fs::remove_dir(path)
    }

    /// Recursively removes a directory and its contents.
    ///
    /// # Errors
    ///
    /// Returns the underlying filesystem error.
    pub fn remove_dir_all(path: &str) -> std::io::Result<()> {
        fs::remove_dir_all(path)
    }
}

/// One open file handle supporting cursor-free positioned reads.
///
/// Unlike [`Fs::read`], [`Self::pread`] does not reopen the path for each
/// operation. It also leaves the file cursor untouched, so shared references
/// to the same handle may read different ranges concurrently.
pub struct PositionedFile(File);

impl PositionedFile {
    /// Consumes and returns an already open handle.
    ///
    /// This is the runtime target of Stainless's explicit move construction
    /// for a native `File` data member.
    #[must_use]
    pub fn from_owned(file: Self) -> Self {
        file
    }

    /// Opens one existing file for reading.
    ///
    /// # Errors
    ///
    /// Returns the underlying filesystem error when the file cannot be opened.
    pub fn open(path: &str) -> std::io::Result<Self> {
        File::open(path).map(Self)
    }

    /// Creates or truncates one file and opens it for writing.
    ///
    /// # Errors
    ///
    /// Returns the underlying filesystem error when the file cannot be
    /// created.
    pub fn create(path: &str) -> std::io::Result<Self> {
        File::create(path).map(Self)
    }

    /// Reads at most `length` bytes beginning at the absolute byte `offset`.
    ///
    /// The returned vector is shorter than `length` at end of file. The
    /// operation is cursor-free and may run concurrently through shared
    /// references to this same handle.
    ///
    /// # Errors
    ///
    /// Returns the underlying positioned-read error.
    pub fn pread(&self, offset: u64, length: usize) -> std::io::Result<Vec<u8>> {
        let mut bytes = vec![0; length];
        let count = loop {
            match read_at(&self.0, &mut bytes, offset) {
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                result => break result?,
            }
        };
        bytes.truncate(count);
        Ok(bytes)
    }

    /// Reads exactly `length` bytes beginning at the absolute byte `offset`.
    ///
    /// # Errors
    ///
    /// Returns [`std::io::ErrorKind::UnexpectedEof`] when the requested range
    /// extends beyond the file, or the underlying positioned-read error.
    pub fn pread_exact(&self, offset: u64, length: usize) -> std::io::Result<Vec<u8>> {
        let mut bytes = vec![0; length];
        let mut filled = 0;
        while filled < length {
            let read_offset = offset
                .checked_add(u64::try_from(filled).map_err(std::io::Error::other)?)
                .ok_or_else(|| std::io::Error::other("positioned read offset overflow"))?;
            let count = loop {
                match read_at(&self.0, &mut bytes[filled..], read_offset) {
                    Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                    result => break result?,
                }
            };
            if count == 0 {
                return Err(std::io::Error::from(std::io::ErrorKind::UnexpectedEof));
            }
            filled += count;
        }
        Ok(bytes)
    }

    /// Writes bytes beginning at the absolute byte `offset` without changing
    /// a shared cursor and returns the number of bytes written.
    ///
    /// Like Rust's low-level write operations, a successful call may write
    /// fewer bytes than supplied. Callers implementing durable formats must
    /// handle short writes before calling [`Self::sync_data`] or
    /// [`Self::sync_all`].
    ///
    /// # Errors
    ///
    /// Returns the underlying positioned-write error.
    pub fn pwrite(&self, offset: u64, bytes: &[u8]) -> std::io::Result<usize> {
        loop {
            match write_at(&self.0, bytes, offset) {
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                result => return result,
            }
        }
    }

    /// Writes every byte beginning at the absolute byte `offset` without
    /// changing a shared cursor.
    ///
    /// # Errors
    ///
    /// Returns [`std::io::ErrorKind::WriteZero`] when the file accepts no
    /// further bytes, an offset-overflow error, or the underlying
    /// positioned-write error.
    pub fn pwrite_all(&self, offset: u64, bytes: &[u8]) -> std::io::Result<()> {
        let mut written = 0;
        while written < bytes.len() {
            let write_offset = offset
                .checked_add(u64::try_from(written).map_err(std::io::Error::other)?)
                .ok_or_else(|| std::io::Error::other("positioned write offset overflow"))?;
            let count = self.pwrite(write_offset, &bytes[written..])?;
            if count == 0 {
                return Err(std::io::Error::from(std::io::ErrorKind::WriteZero));
            }
            written += count;
        }
        Ok(())
    }

    /// Flushes file contents and metadata to durable storage.
    ///
    /// # Errors
    ///
    /// Returns the underlying synchronization error.
    pub fn sync_all(&self) -> std::io::Result<()> {
        self.0.sync_all()
    }

    /// Flushes file contents, and any metadata required to preserve them, to
    /// durable storage.
    ///
    /// # Errors
    ///
    /// Returns the underlying synchronization error.
    pub fn sync_data(&self) -> std::io::Result<()> {
        self.0.sync_data()
    }

    /// Changes the file's length.
    ///
    /// # Errors
    ///
    /// Returns the underlying truncation or extension error.
    pub fn set_len(&self, size: u64) -> std::io::Result<()> {
        self.0.set_len(size)
    }

    /// Returns the current file length.
    ///
    /// # Errors
    ///
    /// Returns the underlying metadata lookup error.
    pub fn len(&self) -> std::io::Result<u64> {
        self.0.metadata().map(|metadata| metadata.len())
    }

    /// Returns whether the file currently has length zero.
    ///
    /// # Errors
    ///
    /// Returns the underlying metadata lookup error.
    pub fn is_empty(&self) -> std::io::Result<bool> {
        self.len().map(|length| length == 0)
    }

    /// Opens another operating-system handle referring to the same file.
    ///
    /// Positioned concurrent reads do not require cloning; this operation is
    /// available for APIs that specifically need independently owned handles.
    ///
    /// # Errors
    ///
    /// Returns the underlying handle-duplication error.
    pub fn try_clone(&self) -> std::io::Result<Self> {
        self.0.try_clone().map(Self)
    }
}

/// Rust-style file-open builder producing [`PositionedFile`] handles.
pub struct PositionedOpenOptions(std::fs::OpenOptions);

impl PositionedOpenOptions {
    /// Creates an options builder with every access mode disabled.
    #[must_use]
    pub fn new() -> Self {
        Self(std::fs::OpenOptions::new())
    }

    /// Configures read access.
    pub fn read(&mut self, enabled: bool) {
        self.0.read(enabled);
    }

    /// Configures write access.
    pub fn write(&mut self, enabled: bool) {
        self.0.write(enabled);
    }

    /// Configures append access.
    pub fn append(&mut self, enabled: bool) {
        self.0.append(enabled);
    }

    /// Configures truncation when opening an existing file.
    pub fn truncate(&mut self, enabled: bool) {
        self.0.truncate(enabled);
    }

    /// Configures creation when the path does not exist.
    pub fn create(&mut self, enabled: bool) {
        self.0.create(enabled);
    }

    /// Configures atomic creation failure when the path already exists.
    pub fn create_new(&mut self, enabled: bool) {
        self.0.create_new(enabled);
    }

    /// Opens one handle with these options.
    ///
    /// # Errors
    ///
    /// Returns the underlying filesystem error.
    pub fn open(&self, path: &str) -> std::io::Result<PositionedFile> {
        self.0.open(path).map(PositionedFile)
    }
}

impl Default for PositionedOpenOptions {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(unix)]
fn read_at(file: &File, bytes: &mut [u8], offset: u64) -> std::io::Result<usize> {
    use std::os::unix::fs::FileExt as _;

    file.read_at(bytes, offset)
}

#[cfg(unix)]
fn write_at(file: &File, bytes: &[u8], offset: u64) -> std::io::Result<usize> {
    use std::os::unix::fs::FileExt as _;

    file.write_at(bytes, offset)
}

#[cfg(windows)]
fn read_at(file: &File, bytes: &mut [u8], offset: u64) -> std::io::Result<usize> {
    use std::os::windows::fs::FileExt as _;

    file.seek_read(bytes, offset)
}

#[cfg(windows)]
fn write_at(file: &File, bytes: &[u8], offset: u64) -> std::io::Result<usize> {
    use std::os::windows::fs::FileExt as _;

    file.seek_write(bytes, offset)
}

/// An owned value from the JSON data model.
///
/// Access is null-safe: selecting a missing object member, indexing beyond an
/// array's end, or applying either operation to the wrong JSON kind produces
/// [`Var::null`]. Returned children are owned clones, so they cannot dangle
/// after their parent changes or is dropped.
#[derive(Clone, Debug, Default)]
pub struct Var(VarRepr);

#[derive(Clone, Debug, Default)]
enum VarRepr {
    #[default]
    Null,
    Bool(bool),
    Number(Number),
    String(String),
    Array(Arc<RwLock<Vec<Var>>>),
    Object(Arc<RwLock<BTreeMap<String, Var>>>),
}

static JSON_MUTATION: Mutex<()> = Mutex::new(());

#[allow(clippy::cast_possible_truncation)]
impl Var {
    /// Creates JSON `null`.
    #[must_use]
    pub const fn null() -> Self {
        Self(VarRepr::Null)
    }

    /// Creates a JSON array from already converted elements.
    #[must_use]
    pub fn array(values: impl IntoIterator<Item = Self>) -> Self {
        Self(VarRepr::Array(Arc::new(RwLock::new(
            values.into_iter().collect(),
        ))))
    }

    /// Creates a JSON object. A later duplicate key replaces an earlier one.
    #[must_use]
    pub fn object<K>(fields: impl IntoIterator<Item = (K, Self)>) -> Self
    where
        K: Into<String>,
    {
        Self(VarRepr::Object(Arc::new(RwLock::new(
            fields
                .into_iter()
                .map(|(key, value)| (key.into(), value))
                .collect::<BTreeMap<_, _>>(),
        ))))
    }

    /// Creates an empty JSON object without requiring a key type annotation.
    #[must_use]
    pub fn empty_object() -> Self {
        Self(VarRepr::Object(Arc::new(RwLock::new(BTreeMap::new()))))
    }

    /// Parses one complete UTF-8 JSON document.
    ///
    /// # Errors
    ///
    /// Returns [`JsonError`] when `source` is not valid JSON.
    pub fn parse(source: &str) -> Result<Self, JsonError> {
        serde_json::from_str(source)
            .map(Self::from_value)
            .map_err(JsonError::parse)
    }

    /// Parses one complete UTF-8 JSON document from bytes.
    ///
    /// # Errors
    ///
    /// Returns [`JsonError`] when `source` is not valid UTF-8 JSON.
    pub fn parse_bytes(source: &[u8]) -> Result<Self, JsonError> {
        serde_json::from_slice(source)
            .map(Self::from_value)
            .map_err(JsonError::parse)
    }

    /// Parses one UTF-8 JSON document from a file through a buffered reader.
    ///
    /// # Errors
    ///
    /// Returns [`JsonError`] when the file cannot be opened/read or does not
    /// contain valid JSON.
    pub fn parse_file(path: impl AsRef<Path>) -> Result<Self, JsonError> {
        let file = File::open(path).map_err(JsonError::io)?;
        serde_json::from_reader(BufReader::new(file))
            .map(Self::from_value)
            .map_err(JsonError::parse)
    }

    /// Serializes this value as compact JSON.
    ///
    /// `Var` can contain only the JSON data model, so this operation is
    /// infallible and does not participate in Stainless checked exceptions.
    #[must_use]
    pub fn to_json(&self) -> String {
        self.to_value().to_string()
    }

    /// Returns whether the value is JSON `null`.
    #[must_use]
    pub fn is_null(&self) -> bool {
        matches!(self.0, VarRepr::Null)
    }

    /// Returns an owned object member or JSON `null` when it is unavailable.
    #[must_use]
    pub fn field(&self, name: &str) -> Self {
        match &self.0 {
            VarRepr::Object(object) => read_lock(object).get(name).cloned(),
            _ => None,
        }
        .unwrap_or_else(Self::null)
    }

    /// Returns an owned array element or JSON `null` when it is unavailable.
    #[must_use]
    pub fn index(&self, index: usize) -> Self {
        match &self.0 {
            VarRepr::Array(array) => read_lock(array).get(index).cloned(),
            _ => None,
        }
        .unwrap_or_else(Self::null)
    }

    /// Replaces or creates an object member.
    ///
    /// # Errors
    ///
    /// Returns [`JsonError`] when the receiver is not an object or inserting
    /// `value` would create a reference cycle.
    pub fn set_field(&mut self, name: &str, value: Self) -> Result<(), JsonError> {
        let _mutation = mutation_lock();
        let VarRepr::Object(object) = &self.0 else {
            return Err(JsonError::mutation("member assignment requires an object"));
        };
        reject_cycle(aggregate_id(object), &value)?;
        write_lock(object).insert(name.to_owned(), value);
        Ok(())
    }

    /// Replaces an array element, extending the array with `null` values when
    /// `index` is beyond its current end, like JavaScript indexed assignment.
    ///
    /// # Errors
    ///
    /// Returns [`JsonError`] when the receiver is not an array or inserting
    /// `value` would create a reference cycle.
    pub fn set_index(&mut self, index: usize, value: Self) -> Result<(), JsonError> {
        let _mutation = mutation_lock();
        let VarRepr::Array(array) = &self.0 else {
            return Err(JsonError::mutation("indexed assignment requires an array"));
        };
        reject_cycle(aggregate_id(array), &value)?;
        let mut array = write_lock(array);
        if index >= array.len() {
            let length = index
                .checked_add(1)
                .ok_or_else(|| JsonError::mutation("array index exceeds addressable length"))?;
            array.resize(length, Self::null());
        }
        array[index] = value;
        Ok(())
    }

    /// Appends one value to an array.
    ///
    /// # Errors
    ///
    /// Returns [`JsonError`] when the receiver is not an array or inserting
    /// `value` would create a reference cycle.
    pub fn push(&mut self, value: Self) -> Result<(), JsonError> {
        let _mutation = mutation_lock();
        let VarRepr::Array(array) = &self.0 else {
            return Err(JsonError::mutation("push requires an array"));
        };
        reject_cycle(aggregate_id(array), &value)?;
        write_lock(array).push(value);
        Ok(())
    }

    /// Removes and returns the last array element, or `null` when empty.
    ///
    /// # Errors
    ///
    /// Returns [`JsonError`] when the receiver is not an array.
    pub fn pop(&mut self) -> Result<Self, JsonError> {
        let _mutation = mutation_lock();
        let VarRepr::Array(array) = &self.0 else {
            return Err(JsonError::mutation("pop requires an array"));
        };
        Ok(write_lock(array).pop().unwrap_or_else(Self::null))
    }

    /// Inserts one value at an existing array boundary.
    ///
    /// # Errors
    ///
    /// Returns [`JsonError`] when the receiver is not an array, `index` is
    /// greater than its length, or inserting `value` would create a cycle.
    pub fn insert(&mut self, index: usize, value: Self) -> Result<(), JsonError> {
        let _mutation = mutation_lock();
        let VarRepr::Array(array) = &self.0 else {
            return Err(JsonError::mutation("insert requires an array"));
        };
        reject_cycle(aggregate_id(array), &value)?;
        let mut array = write_lock(array);
        if index > array.len() {
            return Err(JsonError::mutation(format!(
                "array insert index {index} exceeds length {}",
                array.len()
            )));
        }
        array.insert(index, value);
        Ok(())
    }

    /// Removes and returns an array element.
    ///
    /// # Errors
    ///
    /// Returns [`JsonError`] when the receiver is not an array or `index` is
    /// out of bounds.
    pub fn remove_index(&mut self, index: usize) -> Result<Self, JsonError> {
        let _mutation = mutation_lock();
        let VarRepr::Array(array) = &self.0 else {
            return Err(JsonError::mutation("indexed remove requires an array"));
        };
        let mut array = write_lock(array);
        if index >= array.len() {
            return Err(JsonError::mutation(format!(
                "array remove index {index} exceeds length {}",
                array.len()
            )));
        }
        Ok(array.remove(index))
    }

    /// Removes and returns an object member, or `null` when absent.
    ///
    /// # Errors
    ///
    /// Returns [`JsonError`] when the receiver is not an object.
    pub fn remove_field(&mut self, name: &str) -> Result<Self, JsonError> {
        let _mutation = mutation_lock();
        let VarRepr::Object(object) = &self.0 else {
            return Err(JsonError::mutation("member remove requires an object"));
        };
        Ok(write_lock(object).remove(name).unwrap_or_else(Self::null))
    }

    /// Removes every element or member from an array or object.
    ///
    /// # Errors
    ///
    /// Returns [`JsonError`] when the receiver is a scalar.
    pub fn clear(&mut self) -> Result<(), JsonError> {
        let _mutation = mutation_lock();
        match &self.0 {
            VarRepr::Array(array) => write_lock(array).clear(),
            VarRepr::Object(object) => write_lock(object).clear(),
            _ => return Err(JsonError::mutation("clear requires an array or object")),
        }
        Ok(())
    }

    /// Returns the number of array elements or object members.
    ///
    /// # Errors
    ///
    /// Returns [`JsonError`] when the receiver is a scalar.
    pub fn len(&self) -> Result<usize, JsonError> {
        match &self.0 {
            VarRepr::Array(array) => Ok(read_lock(array).len()),
            VarRepr::Object(object) => Ok(read_lock(object).len()),
            _ => Err(JsonError::mutation("len requires an array or object")),
        }
    }

    /// Returns whether an array or object has no contents.
    ///
    /// # Errors
    ///
    /// Returns [`JsonError`] when the receiver is a scalar.
    pub fn is_empty(&self) -> Result<bool, JsonError> {
        self.len().map(|length| length == 0)
    }

    /// Returns whether an object contains `name`.
    ///
    /// # Errors
    ///
    /// Returns [`JsonError`] when the receiver is not an object.
    pub fn contains_key(&self, name: &str) -> Result<bool, JsonError> {
        let VarRepr::Object(object) = &self.0 else {
            return Err(JsonError::mutation("contains_key requires an object"));
        };
        Ok(read_lock(object).contains_key(name))
    }

    /// Applies JavaScript truthiness to a JSON-compatible value.
    #[must_use]
    pub fn to_bool(&self) -> bool {
        match &self.0 {
            VarRepr::Null => false,
            VarRepr::Bool(value) => *value,
            VarRepr::Number(value) => value.as_f64().is_some_and(|value| value != 0.0),
            VarRepr::String(value) => !value.is_empty(),
            VarRepr::Array(_) | VarRepr::Object(_) => true,
        }
    }

    /// Applies JavaScript `String(value)` behavior to JSON-compatible values.
    #[must_use]
    pub fn to_string_value(&self) -> String {
        match &self.0 {
            VarRepr::Null => "null".to_owned(),
            VarRepr::Bool(value) => value.to_string(),
            VarRepr::Number(value) => value.to_string(),
            VarRepr::String(value) => value.clone(),
            VarRepr::Array(values) => read_lock(values)
                .iter()
                .map(js_array_element_string)
                .collect::<Vec<_>>()
                .join(","),
            VarRepr::Object(_) => "[object Object]".to_owned(),
        }
    }

    /// Converts through JavaScript's numeric coercion and then to `i8`.
    #[must_use]
    pub fn to_i8(&self) -> i8 {
        signed_integer(self.js_number(), 8) as i8
    }

    /// Converts through JavaScript's numeric coercion and then to `i16`.
    #[must_use]
    pub fn to_i16(&self) -> i16 {
        signed_integer(self.js_number(), 16) as i16
    }

    /// Converts through JavaScript's `ToInt32` operation.
    #[must_use]
    pub fn to_i32(&self) -> i32 {
        signed_integer(self.js_number(), 32) as i32
    }

    /// Converts through JavaScript's numeric coercion and then to `i64`.
    #[must_use]
    pub fn to_i64(&self) -> i64 {
        signed_integer(self.js_number(), 64) as i64
    }

    /// Converts through JavaScript's numeric coercion and then to `i128`.
    #[must_use]
    pub fn to_i128(&self) -> i128 {
        signed_integer(self.js_number(), 128)
    }

    /// Converts through JavaScript's numeric coercion and then to `isize`.
    #[must_use]
    pub fn to_isize(&self) -> isize {
        signed_integer(self.js_number(), isize::BITS) as isize
    }

    /// Converts through JavaScript's numeric coercion and then to `u8`.
    #[must_use]
    pub fn to_u8(&self) -> u8 {
        unsigned_integer(self.js_number(), 8) as u8
    }

    /// Converts through JavaScript's numeric coercion and then to `u16`.
    #[must_use]
    pub fn to_u16(&self) -> u16 {
        unsigned_integer(self.js_number(), 16) as u16
    }

    /// Converts through JavaScript's `ToUint32` operation.
    #[must_use]
    pub fn to_u32(&self) -> u32 {
        unsigned_integer(self.js_number(), 32) as u32
    }

    /// Converts through JavaScript's numeric coercion and then to `u64`.
    #[must_use]
    pub fn to_u64(&self) -> u64 {
        unsigned_integer(self.js_number(), 64) as u64
    }

    /// Converts through JavaScript's numeric coercion and then to `u128`.
    #[must_use]
    pub fn to_u128(&self) -> u128 {
        unsigned_integer(self.js_number(), 128)
    }

    /// Converts a JSON unsigned integer or decimal string to `u128` without
    /// floating-point coercion.
    ///
    /// # Errors
    ///
    /// Returns [`JsonError`] for other JSON kinds, fractional/negative input,
    /// or values outside the `u128` range.
    pub fn to_u128_exact(&self) -> Result<u128, JsonError> {
        let source = match &self.0 {
            VarRepr::Number(value) => value.to_string(),
            VarRepr::String(value) => value.clone(),
            _ => {
                return Err(JsonError::mutation(
                    "exact u128 conversion requires a number or decimal string",
                ));
            }
        };
        if source.is_empty() || !source.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(JsonError::mutation(
                "exact u128 conversion requires an unsigned decimal integer",
            ));
        }
        source
            .parse::<u128>()
            .map_err(|error| JsonError::mutation(format!("u128 conversion failed: {error}")))
    }

    /// Converts through JavaScript's numeric coercion and then to `usize`.
    #[must_use]
    pub fn to_usize(&self) -> usize {
        unsigned_integer(self.js_number(), usize::BITS) as usize
    }

    /// Applies JavaScript numeric coercion and narrows the result to `f32`.
    #[must_use]
    pub fn to_f32(&self) -> f32 {
        self.js_number() as f32
    }

    /// Applies JavaScript numeric coercion.
    #[must_use]
    pub fn to_f64(&self) -> f64 {
        self.js_number()
    }

    fn js_number(&self) -> f64 {
        match &self.0 {
            VarRepr::Null => 0.0,
            VarRepr::Bool(value) => f64::from(u8::from(*value)),
            VarRepr::Number(value) => value.as_f64().unwrap_or(f64::NAN),
            VarRepr::String(value) => js_string_number(value),
            VarRepr::Array(_) => js_string_number(&self.to_string_value()),
            VarRepr::Object(_) => f64::NAN,
        }
    }

    fn from_value(value: Value) -> Self {
        match value {
            Value::Null => Self::null(),
            Value::Bool(value) => Self(VarRepr::Bool(value)),
            Value::Number(value) => Self(VarRepr::Number(value)),
            Value::String(value) => Self(VarRepr::String(value)),
            Value::Array(values) => Self::array(values.into_iter().map(Self::from_value)),
            Value::Object(values) => Self::object(
                values
                    .into_iter()
                    .map(|(key, value)| (key, Self::from_value(value))),
            ),
        }
    }

    fn to_value(&self) -> Value {
        match &self.0 {
            VarRepr::Null => Value::Null,
            VarRepr::Bool(value) => Value::Bool(*value),
            VarRepr::Number(value) => Value::Number(value.clone()),
            VarRepr::String(value) => Value::String(value.clone()),
            VarRepr::Array(values) => {
                Value::Array(read_lock(values).iter().map(Self::to_value).collect())
            }
            VarRepr::Object(values) => Value::Object(
                read_lock(values)
                    .iter()
                    .map(|(key, value)| (key.clone(), value.to_value()))
                    .collect(),
            ),
        }
    }
}

impl PartialEq for Var {
    fn eq(&self, other: &Self) -> bool {
        match (&self.0, &other.0) {
            (VarRepr::Null, VarRepr::Null) => true,
            (VarRepr::Bool(left), VarRepr::Bool(right)) => left == right,
            (VarRepr::Number(left), VarRepr::Number(right)) => numbers_equal(left, right),
            (VarRepr::String(left), VarRepr::String(right)) => left == right,
            (VarRepr::Array(left), VarRepr::Array(right)) => Arc::ptr_eq(left, right),
            (VarRepr::Object(left), VarRepr::Object(right)) => Arc::ptr_eq(left, right),
            _ => false,
        }
    }
}

impl fmt::Display for Var {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.to_json())
    }
}

impl From<bool> for Var {
    fn from(value: bool) -> Self {
        Self(VarRepr::Bool(value))
    }
}

impl From<char> for Var {
    fn from(value: char) -> Self {
        Self(VarRepr::String(value.to_string()))
    }
}

impl From<String> for Var {
    fn from(value: String) -> Self {
        Self(VarRepr::String(value))
    }
}

impl From<&str> for Var {
    fn from(value: &str) -> Self {
        Self(VarRepr::String(value.to_owned()))
    }
}

impl<T> From<Option<T>> for Var
where
    T: Into<Self>,
{
    fn from(value: Option<T>) -> Self {
        value.map_or_else(Self::null, Into::into)
    }
}

impl<T> From<Vec<T>> for Var
where
    T: Into<Self>,
{
    fn from(values: Vec<T>) -> Self {
        Self::array(values.into_iter().map(Into::into))
    }
}

impl<T> From<LinkedList<T>> for Var
where
    T: Into<Self>,
{
    fn from(values: LinkedList<T>) -> Self {
        Self::array(values.into_iter().map(Into::into))
    }
}

impl<T> From<VecDeque<T>> for Var
where
    T: Into<Self>,
{
    fn from(values: VecDeque<T>) -> Self {
        Self::array(values.into_iter().map(Into::into))
    }
}

impl<T> From<BTreeSet<T>> for Var
where
    T: Into<Self>,
{
    fn from(values: BTreeSet<T>) -> Self {
        Self::array(values.into_iter().map(Into::into))
    }
}

impl<T> From<BTreeMap<String, T>> for Var
where
    T: Into<Self>,
{
    fn from(values: BTreeMap<String, T>) -> Self {
        Self::object(values.into_iter().map(|(key, value)| (key, value.into())))
    }
}

macro_rules! integer_from {
    ($($ty:ty),+ $(,)?) => {$(
        impl From<$ty> for Var {
            fn from(value: $ty) -> Self {
                Self(VarRepr::Number(Number::from(value)))
            }
        }
    )+};
}

integer_from!(i8, i16, i32, i64, u8, u16, u32, u64);

impl From<isize> for Var {
    fn from(value: isize) -> Self {
        Self(VarRepr::Number(Number::from(value as i64)))
    }
}

impl From<usize> for Var {
    fn from(value: usize) -> Self {
        Self(VarRepr::Number(Number::from(value as u64)))
    }
}

impl From<i128> for Var {
    fn from(value: i128) -> Self {
        Self(VarRepr::Number(
            Number::from_i128(value).expect("arbitrary-precision JSON number supports i128"),
        ))
    }
}

impl From<u128> for Var {
    fn from(value: u128) -> Self {
        Self(VarRepr::Number(
            Number::from_u128(value).expect("arbitrary-precision JSON number supports u128"),
        ))
    }
}

impl From<f32> for Var {
    fn from(value: f32) -> Self {
        Self::from(f64::from(value))
    }
}

impl From<f64> for Var {
    fn from(value: f64) -> Self {
        Number::from_f64(value).map_or_else(Self::null, |number| Self(VarRepr::Number(number)))
    }
}

/// Failure while reading, parsing, or mutating JSON.
#[derive(Debug)]
pub struct JsonError {
    operation: JsonOperation,
    message: String,
    source: Option<Box<dyn Error + Send + Sync>>,
}

impl JsonError {
    fn io(error: std::io::Error) -> Self {
        Self::new(JsonOperation::Read, error)
    }

    fn parse(error: serde_json::Error) -> Self {
        Self::new(JsonOperation::Parse, error)
    }

    fn mutation(message: impl Into<String>) -> Self {
        Self {
            operation: JsonOperation::Mutation,
            message: message.into(),
            source: None,
        }
    }

    fn new(operation: JsonOperation, error: impl Error + Send + Sync + 'static) -> Self {
        Self {
            operation,
            message: error.to_string(),
            source: Some(Box::new(error)),
        }
    }

    /// Describes the failed JSON operation.
    #[must_use]
    pub const fn operation(&self) -> &'static str {
        match self.operation {
            JsonOperation::Read => "read",
            JsonOperation::Parse => "parse",
            JsonOperation::Mutation => "mutation",
        }
    }
}

impl fmt::Display for JsonError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "JSON {} failed: {}",
            self.operation(),
            self.message
        )
    }
}

impl Error for JsonError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        self.source
            .as_deref()
            .map(|source| source as &(dyn Error + 'static))
    }
}

#[derive(Clone, Copy, Debug)]
enum JsonOperation {
    Read,
    Parse,
    Mutation,
}

fn mutation_lock() -> MutexGuard<'static, ()> {
    JSON_MUTATION
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn read_lock<T>(lock: &RwLock<T>) -> RwLockReadGuard<'_, T> {
    lock.read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn write_lock<T>(lock: &RwLock<T>) -> RwLockWriteGuard<'_, T> {
    lock.write()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn aggregate_id<T>(aggregate: &Arc<RwLock<T>>) -> usize {
    Arc::as_ptr(aggregate).cast::<()>() as usize
}

fn reject_cycle(target: usize, value: &Var) -> Result<(), JsonError> {
    if contains_aggregate(value, target, &mut BTreeSet::new()) {
        Err(JsonError::mutation(
            "mutation would create a reference cycle",
        ))
    } else {
        Ok(())
    }
}

fn contains_aggregate(value: &Var, target: usize, visited: &mut BTreeSet<usize>) -> bool {
    match &value.0 {
        VarRepr::Array(array) => {
            let id = aggregate_id(array);
            id == target
                || visited.insert(id)
                    && read_lock(array)
                        .iter()
                        .any(|value| contains_aggregate(value, target, visited))
        }
        VarRepr::Object(object) => {
            let id = aggregate_id(object);
            id == target
                || visited.insert(id)
                    && read_lock(object)
                        .values()
                        .any(|value| contains_aggregate(value, target, visited))
        }
        VarRepr::Null | VarRepr::Bool(_) | VarRepr::Number(_) | VarRepr::String(_) => false,
    }
}

fn js_array_element_string(value: &Var) -> String {
    match &value.0 {
        VarRepr::Null => String::new(),
        _ => value.to_string_value(),
    }
}

#[allow(clippy::float_cmp)]
fn numbers_equal(left: &Number, right: &Number) -> bool {
    if let (Some(left), Some(right)) = (left.as_i128(), right.as_i128()) {
        return left == right;
    }
    if let (Some(left), Some(right)) = (left.as_u128(), right.as_u128()) {
        return left == right;
    }
    matches!((left.as_f64(), right.as_f64()), (Some(left), Some(right)) if left == right)
}

fn js_string_number(value: &str) -> f64 {
    let value = value.trim();
    if value.is_empty() {
        0.0
    } else {
        value.parse().unwrap_or(f64::NAN)
    }
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn unsigned_integer(value: f64, bits: u32) -> u128 {
    if !value.is_finite() || value == 0.0 {
        return 0;
    }
    let modulus = 2.0f64.powi(i32::try_from(bits).expect("integer width fits i32"));
    value.trunc().rem_euclid(modulus) as u128
}

fn signed_integer(value: f64, bits: u32) -> i128 {
    let unsigned = unsigned_integer(value, bits);
    let sign = 1_u128 << (bits - 1);
    if unsigned >= sign {
        if bits == 128 {
            -i128::try_from(u128::MAX - unsigned).expect("signed magnitude fits i128") - 1
        } else {
            unsigned.wrapping_sub(1_u128 << bits).cast_signed()
        }
    } else {
        i128::try_from(unsigned).expect("value below signed threshold fits i128")
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet, LinkedList, VecDeque};
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::{
        BigEndian, Fs, LittleEndian, MultiMap, PositionedFile, Random, Var, btree_map_retain,
        btree_map_retain_keys, btree_map_with_first_after, btree_map_with_first_in_range,
        btree_map_with_last_before, btree_map_with_last_in_range, btree_map_with_range,
        vec_copy_range, vec_with_range,
    };

    #[test]
    fn random_bytes_are_bounded_and_have_the_requested_length() {
        assert!(Random::bytes(0).expect("empty random buffer").is_empty());
        assert_eq!(
            Random::bytes(32).expect("operating-system entropy").len(),
            32
        );
        let error = Random::bytes(Random::MAX_BYTES + 1).expect_err("oversized request");
        assert!(error.to_string().contains("maximum"));
    }

    #[test]
    fn vector_ranges_are_checked_and_non_allocating() {
        let values = [1, 2, 3, 4];
        let mut visited = Vec::new();
        assert!(vec_with_range(&values, 1, 3, |value| visited.push(*value)));
        assert_eq!(visited, [2, 3]);
        assert_eq!(vec_copy_range(&values, 1, 3), [2, 3]);
        assert!(!vec_with_range(&values, 3, 5, |_| {}));
    }

    #[test]
    fn fixed_width_endian_encoding_round_trips() {
        let mut bytes = vec![0xaa];
        BigEndian::write_u16(&mut bytes, 0xbbcc);
        BigEndian::write_u32(&mut bytes, 0x0102_0304);
        BigEndian::write_u64(&mut bytes, 0x0506_0708_090a_0b0c);
        BigEndian::write_u128(&mut bytes, 0x0d0e_0f10_1112_1314_1516_1718_191a_1b1c);
        let mut length_bytes = Vec::new();
        BigEndian::write_usize_u32(&mut length_bytes, 4).unwrap();
        BigEndian::write_usize_u64(&mut length_bytes, 8);
        assert_eq!(&length_bytes[..4], &4_u32.to_be_bytes());
        assert_eq!(&length_bytes[4..], &8_u64.to_be_bytes());
        assert_eq!(
            bytes,
            [
                0xaa, 0xbb, 0xcc, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b,
                0x0c, 0x0d, 0x0e, 0x0f, 0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19,
                0x1a, 0x1b, 0x1c,
            ]
        );
        assert_eq!(BigEndian::read_u8(&bytes[..1]).unwrap(), 0xaa);
        assert_eq!(BigEndian::read_u16(&bytes[1..3]).unwrap(), 0xbbcc);
        assert_eq!(BigEndian::read_u32(&bytes[3..7]).unwrap(), 0x0102_0304);
        assert_eq!(
            BigEndian::read_u64(&bytes[7..15]).unwrap(),
            0x0506_0708_090a_0b0c
        );
        assert_eq!(
            BigEndian::read_u128(&bytes[15..31]).unwrap(),
            0x0d0e_0f10_1112_1314_1516_1718_191a_1b1c
        );
        assert_eq!(BigEndian::read_u8_at(&bytes, 0).unwrap(), 0xaa);
        assert_eq!(BigEndian::read_u16_at(&bytes, 1).unwrap(), 0xbbcc);
        assert_eq!(BigEndian::read_u32_at(&bytes, 3).unwrap(), 0x0102_0304);
        assert_eq!(
            BigEndian::read_u64_at(&bytes, 7).unwrap(),
            0x0506_0708_090a_0b0c
        );
        assert_eq!(
            BigEndian::read_u128_at(&bytes, 15).unwrap(),
            0x0d0e_0f10_1112_1314_1516_1718_191a_1b1c
        );

        let mut offset = 0;
        let mut byte = 0;
        let mut short = 0;
        let mut word = 0;
        let mut long = 0;
        let mut wide = 0;
        BigEndian::read_u8_advance(&bytes, &mut offset, &mut byte).unwrap();
        BigEndian::read_u16_advance(&bytes, &mut offset, &mut short).unwrap();
        BigEndian::read_u32_advance(&bytes, &mut offset, &mut word).unwrap();
        BigEndian::read_u64_advance(&bytes, &mut offset, &mut long).unwrap();
        BigEndian::read_u128_advance(&bytes, &mut offset, &mut wide).unwrap();
        assert_eq!(
            (offset, byte, short, word, long, wide),
            (
                bytes.len(),
                0xaa,
                0xbbcc,
                0x0102_0304,
                0x0506_0708_090a_0b0c,
                0x0d0e_0f10_1112_1314_1516_1718_191a_1b1c,
            )
        );
        assert_eq!(
            BigEndian::read_u32(&bytes[..3]).unwrap_err().kind(),
            std::io::ErrorKind::InvalidData
        );

        let mut little = Vec::new();
        LittleEndian::write_u32(&mut little, 0x0102_0304);
        LittleEndian::write_u64(&mut little, 0x0506_0708_090a_0b0c);
        assert_eq!(
            little,
            [
                0x04, 0x03, 0x02, 0x01, 0x0c, 0x0b, 0x0a, 0x09, 0x08, 0x07, 0x06, 0x05
            ]
        );
        assert_eq!(LittleEndian::read_u32(&little[..4]).unwrap(), 0x0102_0304);
        assert_eq!(
            LittleEndian::read_u64(&little[4..12]).unwrap(),
            0x0506_0708_090a_0b0c
        );
        assert_eq!(LittleEndian::read_u32_at(&little, 0).unwrap(), 0x0102_0304);
        assert_eq!(
            LittleEndian::read_u64_at(&little, 4).unwrap(),
            0x0506_0708_090a_0b0c
        );
    }

    #[test]
    fn ordered_map_range_callback_selects_one_predecessor() {
        let mut values = BTreeMap::from([
            (("alpha", 1_u32), 10),
            (("alpha", 3_u32), 30),
            (("beta", 1_u32), 40),
        ]);
        let mut selected = 0;
        assert!(btree_map_with_last_in_range(
            &values,
            &("alpha", 0),
            &("alpha", 2),
            |_, value| selected = *value,
        ));
        assert_eq!(selected, 10);
        assert!(btree_map_with_first_in_range(
            &values,
            &("alpha", 2),
            &("beta", 1),
            |key, value| {
                assert_eq!(*key, ("alpha", 3));
                selected = *value;
            },
        ));
        assert_eq!(selected, 30);
        let mut visited = Vec::new();
        assert_eq!(
            btree_map_with_range(&values, &("alpha", 2), &("beta", 1), |key, value| visited
                .push((*key, *value)),),
            2
        );
        assert_eq!(visited, [(("alpha", 3), 30), (("beta", 1), 40)]);
        assert!(btree_map_with_last_before(
            &values,
            &("alpha", 0),
            &("beta", 0),
            |key, value| {
                assert_eq!(*key, ("alpha", 3));
                selected = *value;
            },
        ));
        assert_eq!(selected, 30);
        assert!(btree_map_with_first_after(
            &values,
            &("alpha", 3),
            &("beta", 1),
            |key, value| {
                assert_eq!(*key, ("beta", 1));
                selected = *value;
            },
        ));
        assert_eq!(selected, 40);
        assert!(!btree_map_with_last_in_range(
            &values,
            &("alpha", 4),
            &("alpha", 2),
            |_, _| unreachable!(),
        ));

        btree_map_retain(&mut values, |key, _| key.1 < 3);
        assert_eq!(values.len(), 2);
        btree_map_retain_keys(&mut values, |key| key.0 == "alpha");
        assert_eq!(values.len(), 1);
    }

    #[test]
    fn ordered_multimap_flattens_duplicate_key_associations() {
        let mut values = MultiMap::new();
        values.insert(2, "second-a".to_owned());
        values.insert(1, "first".to_owned());
        values.insert(2, "second-b".to_owned());

        assert_eq!(values.len(), 3);
        assert_eq!(values.key_len(), 2);
        assert_eq!(
            values
                .iter()
                .map(|(key, value)| (*key, value.as_str()))
                .collect::<Vec<_>>(),
            [(1, "first"), (2, "second-a"), (2, "second-b")]
        );

        let mut matching = Vec::new();
        assert_eq!(values.with(&2, |value| matching.push(value.clone())), 2);
        assert_eq!(matching, ["second-a", "second-b"]);

        assert!(values.remove(&2, &"second-a".to_owned()));
        assert!(!values.remove(&2, &"missing".to_owned()));
        values.retain(|key, value| *key == 1 || value.ends_with('b'));
        assert_eq!(
            values.into_iter().collect::<Vec<_>>(),
            [(1, "first".to_owned()), (2, "second-b".to_owned())]
        );
    }

    #[test]
    fn whole_file_io_preserves_bytes_text_and_errors() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("test clock follows the Unix epoch")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "stainless-runtime-fs-{}-{unique}",
            std::process::id()
        ));
        let nested = root.join("nested");
        let text = nested.join("value.txt");
        let copy = nested.join("copy.bin");
        let renamed = nested.join("renamed.bin");
        let path = |value: &std::path::Path| value.to_string_lossy().into_owned();

        Fs::create_dir_all(&path(&nested)).expect("nested directory is created");
        Fs::write_text(&path(&text), "Stainless").expect("text is written");
        assert_eq!(
            Fs::read_to_string(&path(&text)).expect("text is read"),
            "Stainless"
        );
        assert!(Fs::exists(&path(&text)).expect("existence is checked"));
        assert_eq!(
            Fs::copy(&path(&text), &path(&copy)).expect("file is copied"),
            9
        );
        assert_eq!(
            Fs::read(&path(&copy)).expect("bytes are read"),
            b"Stainless"
        );
        Fs::write_bytes(&path(&copy), &[0, 1, 2]).expect("bytes are replaced");
        Fs::rename(&path(&copy), &path(&renamed)).expect("file is renamed");
        Fs::remove_file(&path(&renamed)).expect("renamed file is removed");
        assert!(Fs::read(&path(&renamed)).is_err());
        Fs::remove_file(&path(&text)).expect("text file is removed");
        Fs::remove_dir(&path(&nested)).expect("empty nested directory is removed");
        Fs::remove_dir_all(&path(&root)).expect("root directory is removed");
    }

    #[test]
    fn positioned_reads_share_one_handle_without_a_cursor() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("test clock follows the Unix epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "stainless-runtime-pread-{}-{unique}.bin",
            std::process::id()
        ));
        std::fs::write(&path, b"zero-one-two-three").expect("fixture is written");
        let file = std::sync::Arc::new(
            PositionedFile::open(&path.to_string_lossy()).expect("fixture is opened once"),
        );

        let readers = [(0, 4, b"zero".as_slice()), (9, 3, b"two".as_slice())]
            .into_iter()
            .map(|(offset, length, expected)| {
                let file = std::sync::Arc::clone(&file);
                std::thread::spawn(move || {
                    assert_eq!(
                        file.pread(offset, length).expect("positioned read"),
                        expected
                    );
                })
            })
            .collect::<Vec<_>>();
        for reader in readers {
            reader.join().expect("reader did not panic");
        }
        assert_eq!(
            file.pread(13, 99).expect("short read at end of file"),
            b"three"
        );

        std::fs::remove_file(path).expect("fixture is removed");
    }

    #[test]
    fn missing_access_is_null_and_children_are_owned() {
        let value = Var::object([
            ("items", Var::array([Var::from(1), Var::from("text")])),
            ("enabled", Var::from(true)),
        ]);

        assert_eq!(value.field("items").index(1), Var::from("text"));
        assert!(value.field("missing").is_null());
        assert!(value.field("items").index(99).is_null());
        assert!(Var::from(1).field("missing").is_null());
    }

    #[test]
    fn parses_and_serializes_json() {
        let value = Var::parse(r#"{"name":"Stainless","values":[1,null]}"#).expect("valid JSON");
        let bytes = Var::parse_bytes(br#"{"name":"Stainless","values":[1,null]}"#)
            .expect("valid JSON bytes");

        assert_eq!(value.field("name"), Var::from("Stainless"));
        assert_eq!(bytes.to_json(), value.to_json());
        assert_eq!(value.to_json(), r#"{"name":"Stainless","values":[1,null]}"#);
    }

    #[test]
    fn converts_owned_standard_collections_recursively() {
        let list = LinkedList::from([1, 2]);
        let queue = VecDeque::from([3, 4]);
        let set = BTreeSet::from([6, 5]);
        let map = BTreeMap::from([
            ("list".to_owned(), Var::from(list)),
            ("queue".to_owned(), Var::from(queue)),
            ("set".to_owned(), Var::from(set)),
            ("missing".to_owned(), Var::from(None::<i32>)),
        ]);
        let value = Var::from(map);

        assert_eq!(
            value.to_json(),
            r#"{"list":[1,2],"missing":null,"queue":[3,4],"set":[5,6]}"#
        );
    }

    #[test]
    fn scalar_conversions_follow_javascript_coercion() {
        assert_eq!(Var::from(" 42 ").to_i32(), 42);
        assert_eq!(Var::from("not a number").to_i32(), 0);
        assert_eq!(Var::null().to_i32(), 0);
        assert!(!Var::from(0).to_bool());
        assert!(Var::array([]).to_bool());
        assert_eq!(
            Var::array([Var::from(1), Var::null(), Var::from("x")]).to_string_value(),
            "1,,x"
        );
    }

    #[test]
    fn exact_u128_conversion_preserves_wapi_decimal_amounts() {
        let maximum = u128::MAX.to_string();
        assert_eq!(
            Var::from(maximum.clone())
                .to_u128_exact()
                .expect("decimal string"),
            u128::MAX
        );
        assert_eq!(
            Var::parse(&maximum)
                .expect("arbitrary precision JSON number")
                .to_u128_exact()
                .expect("exact JSON number"),
            u128::MAX
        );
        assert!(Var::from("1.5").to_u128_exact().is_err());
        assert!(Var::from("-1").to_u128_exact().is_err());
    }

    #[test]
    fn aggregate_clones_share_identity() {
        let object = Var::object([("value", Var::from(1))]);
        let array = Var::array([object.clone()]);

        assert_eq!(object, object.clone());
        assert_eq!(array, array.clone());
        assert_eq!(array.index(0), object);
        assert_ne!(Var::empty_object(), Var::empty_object());
        assert_ne!(Var::array([]), Var::array([]));
    }

    #[test]
    fn numeric_equality_compares_values_not_json_spellings() {
        let integer = Var::parse("1").expect("valid integer");
        let decimal = Var::parse("1.0").expect("valid decimal");

        assert_eq!(integer, decimal);
    }

    #[test]
    fn aggregate_aliases_observe_mutations() {
        let mut object = Var::object([("items", Var::array([Var::from(1)]))]);
        let shared_object = object.clone();

        object
            .set_field("enabled", Var::from(true))
            .expect("object mutation succeeds");
        let mut items = object.field("items");
        items.push(Var::from(2)).expect("array push succeeds");
        items
            .set_index(4, Var::from(5))
            .expect("indexed assignment extends with null");

        assert_eq!(shared_object.field("enabled"), Var::from(true));
        assert_eq!(shared_object.field("items").index(1), Var::from(2));
        assert!(shared_object.field("items").index(3).is_null());
        assert_eq!(shared_object.field("items").index(4), Var::from(5));
    }

    #[test]
    fn mutations_reject_wrong_kinds_invalid_indices_and_cycles() {
        let mut scalar = Var::from(1);
        assert!(scalar.push(Var::from(2)).is_err());

        let mut array = Var::array([]);
        assert!(array.remove_index(0).is_err());

        let mut object = Var::empty_object();
        let self_reference = object.clone();
        let error = object
            .set_field("self", self_reference)
            .expect_err("cycles are rejected");
        assert_eq!(error.operation(), "mutation");
        assert_eq!(object.to_json(), "{}");
    }

    #[test]
    fn shared_array_mutation_is_thread_safe() {
        let array = Var::array([]);
        let threads = (0..8)
            .map(|value| {
                let mut array = array.clone();
                std::thread::spawn(move || array.push(Var::from(value)))
            })
            .collect::<Vec<_>>();

        for thread in threads {
            thread
                .join()
                .expect("mutation thread does not panic")
                .expect("array push succeeds");
        }
        assert_eq!(array.len().expect("array has a length"), 8);
    }
}
