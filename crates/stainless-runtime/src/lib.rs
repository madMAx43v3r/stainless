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

/// Runtime crate source directory used by `stainlessc` for its hidden Cargo
/// build. Packaged tools fall back to the crates.io version if this directory
/// is no longer present.
#[doc(hidden)]
pub const CRATE_SOURCE_DIR: &str = env!("CARGO_MANIFEST_DIR");

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

    use super::{Fs, Var};

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

        assert_eq!(value.field("name"), Var::from("Stainless"));
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
