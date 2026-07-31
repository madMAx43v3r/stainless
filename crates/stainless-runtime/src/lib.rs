//! Compact runtime support for generated Stainless programs.
//!
//! Most Stainless values lower directly to Rust. JSON is the deliberate
//! exception: [`Var`] provides the language's dynamically typed JSON value
//! while delegating parsing and serialization to `serde_json`.

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;
use std::fs::File;
use std::io::BufReader;
use std::path::Path;
use std::sync::Arc;

use serde_json::{Number, Value};

/// Runtime crate source directory used by `stainlessc` for its hidden Cargo
/// build. Packaged tools fall back to the crates.io version if this directory
/// is no longer present.
#[doc(hidden)]
pub const CRATE_SOURCE_DIR: &str = env!("CARGO_MANIFEST_DIR");

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
    Array(Arc<[Var]>),
    Object(Arc<BTreeMap<String, Var>>),
}

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
        Self(VarRepr::Array(values.into_iter().collect()))
    }

    /// Creates a JSON object. A later duplicate key replaces an earlier one.
    #[must_use]
    pub fn object<K>(fields: impl IntoIterator<Item = (K, Self)>) -> Self
    where
        K: Into<String>,
    {
        Self(VarRepr::Object(Arc::new(
            fields
                .into_iter()
                .map(|(key, value)| (key.into(), value))
                .collect::<BTreeMap<_, _>>(),
        )))
    }

    /// Creates an empty JSON object without requiring a key type annotation.
    #[must_use]
    pub fn empty_object() -> Self {
        Self(VarRepr::Object(Arc::new(BTreeMap::new())))
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
            VarRepr::Object(object) => object.get(name),
            _ => None,
        }
        .cloned()
        .unwrap_or_else(Self::null)
    }

    /// Returns an owned array element or JSON `null` when it is unavailable.
    #[must_use]
    pub fn index(&self, index: usize) -> Self {
        match &self.0 {
            VarRepr::Array(array) => array.get(index),
            _ => None,
        }
        .cloned()
        .unwrap_or_else(Self::null)
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
            VarRepr::Array(values) => values
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
            VarRepr::Array(values) => Value::Array(values.iter().map(Self::to_value).collect()),
            VarRepr::Object(values) => Value::Object(
                values
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

/// Failure while reading or parsing JSON.
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
    use super::Var;

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
}
