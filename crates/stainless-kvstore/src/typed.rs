use std::fmt;
use std::marker::PhantomData;
use std::path::Path;
use std::sync::Arc;

use super::__stainless_namespace_kvstore::Table as RawTable;
use super::{
    __StainlessExceptionBox, stainless_kvstore_commit_raw, stainless_kvstore_find_raw,
    stainless_kvstore_insert_raw, stainless_kvstore_open_raw, stainless_kvstore_revert_raw,
    stainless_kvstore_version_raw,
};

/// An encoding, decoding, path, or Stainless storage failure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Error(String);

impl Error {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

/// A stable byte representation used for persisted keys and values.
///
/// Applications can implement this trait for their own structs. Decoding must
/// reject malformed or trailing data rather than silently accepting it.
pub trait Codec: Sized + Send + Sync + 'static {
    /// Encodes one complete value.
    ///
    /// # Errors
    ///
    /// Returns an error when this value has no valid persistent representation.
    fn encode(&self) -> Result<Vec<u8>, Error>;

    /// Decodes one complete value.
    ///
    /// # Errors
    ///
    /// Returns an error for malformed, unsupported, or trailing bytes.
    fn decode(bytes: &[u8]) -> Result<Self, Error>;
}

/// A key codec whose byte representation preserves the key's `Ord` ordering.
///
/// This invariant makes the Stainless `Map<Vec<u8>, ...>` index order match
/// the source key order. User implementations must uphold it.
pub trait OrderedKey: Codec + Ord {}

/// A typed view over the Stainless-written versioned storage engine.
///
/// Keys and values remain statically typed in Rust. Only their codec output is
/// passed into the Stainless WAL and its ordered in-memory index.
pub struct Table<K: OrderedKey, V: Codec> {
    raw: Arc<RawTable>,
    marker: PhantomData<(K, V)>,
}

impl<K: OrderedKey, V: Codec> Table<K, V> {
    /// Opens a store and recovers its last committed state.
    ///
    /// # Errors
    ///
    /// Returns an error for non-UTF-8 paths or file/recovery failures.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, Error> {
        let path = path
            .as_ref()
            .to_str()
            .ok_or_else(|| Error::new("the Stainless store path must be UTF-8"))?
            .to_owned();
        let raw = stainless_kvstore_open_raw(&path).map_err(stainless_error)?;
        Ok(Self {
            raw,
            marker: PhantomData,
        })
    }

    /// Appends a typed key/value update in the current version.
    ///
    /// # Errors
    ///
    /// Returns an error when either value cannot be encoded or the WAL write
    /// fails.
    pub fn insert(&self, key: K, value: V) -> Result<(), Error> {
        let key = key.encode()?;
        let value = value.encode()?;
        stainless_kvstore_insert_raw(Arc::clone(&self.raw), key, value).map_err(stainless_error)
    }

    /// Finds and decodes the newest visible value for `key`.
    ///
    /// # Errors
    ///
    /// Returns an error when the key cannot be encoded, the positioned read
    /// fails, or the persisted value does not decode as `V`.
    pub fn find(&self, key: &K) -> Result<Option<V>, Error> {
        let key = key.encode()?;
        let value =
            stainless_kvstore_find_raw(Arc::clone(&self.raw), &key).map_err(stainless_error)?;
        value.map(|value| V::decode(&value.bytes)).transpose()
    }

    /// Durably commits pending updates and advances to `next_version`.
    ///
    /// Returns `false` when the requested version does not advance the store.
    ///
    /// # Errors
    ///
    /// Returns an error when writing or syncing the commit record fails.
    pub fn commit(&self, next_version: u32) -> Result<bool, Error> {
        stainless_kvstore_commit_raw(Arc::clone(&self.raw), next_version).map_err(stainless_error)
    }

    /// Durably removes visibility of updates at or above `version`.
    ///
    /// Returns `false` when `version` is newer than the current version.
    ///
    /// # Errors
    ///
    /// Returns an error when writing or syncing the revert record fails.
    pub fn revert(&self, version: u32) -> Result<bool, Error> {
        stainless_kvstore_revert_raw(Arc::clone(&self.raw), version).map_err(stainless_error)
    }

    /// Returns the current visible version.
    #[must_use]
    pub fn current_version(&self) -> u32 {
        stainless_kvstore_version_raw(Arc::clone(&self.raw))
    }
}

fn stainless_error(error: __StainlessExceptionBox) -> Error {
    Error::new(error.to_string())
}

impl Codec for bool {
    fn encode(&self) -> Result<Vec<u8>, Error> {
        Ok(vec![u8::from(*self)])
    }

    fn decode(bytes: &[u8]) -> Result<Self, Error> {
        match bytes {
            [0] => Ok(false),
            [1] => Ok(true),
            _ => Err(Error::new("invalid bool encoding")),
        }
    }
}

impl OrderedKey for bool {}

macro_rules! unsigned_codecs {
    ($($ty:ty),+ $(,)?) => {
        $(
            impl Codec for $ty {
                fn encode(&self) -> Result<Vec<u8>, Error> {
                    Ok(self.to_be_bytes().to_vec())
                }

                fn decode(bytes: &[u8]) -> Result<Self, Error> {
                    let bytes: [u8; ::core::mem::size_of::<$ty>()] = bytes
                        .try_into()
                        .map_err(|_| Error::new(concat!("invalid ", stringify!($ty), " encoding")))?;
                    Ok(<$ty>::from_be_bytes(bytes))
                }
            }

            impl OrderedKey for $ty {}
        )+
    };
}

macro_rules! signed_codecs {
    ($(($signed:ty, $unsigned:ty)),+ $(,)?) => {
        $(
            impl Codec for $signed {
                fn encode(&self) -> Result<Vec<u8>, Error> {
                    let sign = (1 as $unsigned) << (<$unsigned>::BITS - 1);
                    Ok(((*self as $unsigned) ^ sign).to_be_bytes().to_vec())
                }

                fn decode(bytes: &[u8]) -> Result<Self, Error> {
                    let bytes: [u8; ::core::mem::size_of::<$unsigned>()] = bytes
                        .try_into()
                        .map_err(|_| Error::new(concat!("invalid ", stringify!($signed), " encoding")))?;
                    let sign = (1 as $unsigned) << (<$unsigned>::BITS - 1);
                    Ok((<$unsigned>::from_be_bytes(bytes) ^ sign) as $signed)
                }
            }

            impl OrderedKey for $signed {}
        )+
    };
}

unsigned_codecs!(u8, u16, u32, u64, u128);
signed_codecs!((i8, u8), (i16, u16), (i32, u32), (i64, u64), (i128, u128));

impl Codec for Vec<u8> {
    fn encode(&self) -> Result<Vec<u8>, Error> {
        Ok(self.clone())
    }

    fn decode(bytes: &[u8]) -> Result<Self, Error> {
        Ok(bytes.to_vec())
    }
}

impl OrderedKey for Vec<u8> {}

impl Codec for String {
    fn encode(&self) -> Result<Vec<u8>, Error> {
        Ok(self.as_bytes().to_vec())
    }

    fn decode(bytes: &[u8]) -> Result<Self, Error> {
        String::from_utf8(bytes.to_vec()).map_err(|error| Error::new(error.to_string()))
    }
}

impl OrderedKey for String {}

impl<A: Codec, B: Codec> Codec for (A, B) {
    fn encode(&self) -> Result<Vec<u8>, Error> {
        let mut output = Vec::new();
        push_segment(&self.0.encode()?, &mut output);
        push_segment(&self.1.encode()?, &mut output);
        Ok(output)
    }

    fn decode(bytes: &[u8]) -> Result<Self, Error> {
        let mut offset = 0;
        let first = take_segment(bytes, &mut offset)?;
        let second = take_segment(bytes, &mut offset)?;
        if offset != bytes.len() {
            return Err(Error::new("trailing bytes in 2-tuple encoding"));
        }
        Ok((A::decode(&first)?, B::decode(&second)?))
    }
}

impl<A: OrderedKey, B: OrderedKey> OrderedKey for (A, B) {}

impl<A: Codec, B: Codec, C: Codec> Codec for (A, B, C) {
    fn encode(&self) -> Result<Vec<u8>, Error> {
        let mut output = Vec::new();
        push_segment(&self.0.encode()?, &mut output);
        push_segment(&self.1.encode()?, &mut output);
        push_segment(&self.2.encode()?, &mut output);
        Ok(output)
    }

    fn decode(bytes: &[u8]) -> Result<Self, Error> {
        let mut offset = 0;
        let first = take_segment(bytes, &mut offset)?;
        let second = take_segment(bytes, &mut offset)?;
        let third = take_segment(bytes, &mut offset)?;
        if offset != bytes.len() {
            return Err(Error::new("trailing bytes in 3-tuple encoding"));
        }
        Ok((A::decode(&first)?, B::decode(&second)?, C::decode(&third)?))
    }
}

impl<A: OrderedKey, B: OrderedKey, C: OrderedKey> OrderedKey for (A, B, C) {}

fn push_segment(value: &[u8], output: &mut Vec<u8>) {
    for byte in value {
        if *byte == 0 {
            output.extend_from_slice(&[0, 255]);
        } else {
            output.push(*byte);
        }
    }
    output.extend_from_slice(&[0, 0]);
}

fn take_segment(input: &[u8], offset: &mut usize) -> Result<Vec<u8>, Error> {
    let mut output = Vec::new();
    while *offset < input.len() {
        let byte = input[*offset];
        *offset += 1;
        if byte != 0 {
            output.push(byte);
            continue;
        }
        let Some(marker) = input.get(*offset).copied() else {
            return Err(Error::new("truncated tuple segment escape"));
        };
        *offset += 1;
        match marker {
            0 => return Ok(output),
            255 => output.push(0),
            _ => return Err(Error::new("invalid tuple segment escape")),
        }
    }
    Err(Error::new("unterminated tuple segment"))
}
