use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;
use std::fs;
use std::io::ErrorKind;
use std::ops::Range;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use super::builtin::standard_bindings;
use super::model::{
    ArgumentAdaptation, CallStyle, CallableBinding, CallbackEscape, CallbackKind, NativeBindings,
    NativeErrorFormat, NativeTypeBinding, Parameter, Receiver, RustLowering, TypeRef,
    WrapperTarget,
};
use crate::ast::{self, Item, TypeKind};

/// Package-root filename containing user-authored Rust bindings.
pub const BINDINGS_MANIFEST_FILENAME: &str = "stainless-bindings.toml";

/// Parses one version-1 bindings manifest without adding compiler built-ins.
///
/// # Errors
///
/// Returns a [`ManifestError`] for malformed TOML, an unsupported schema,
/// invalid paths or types, unsupported initial features, or inconsistent
/// binding metadata.
#[allow(clippy::too_many_lines)]
pub fn parse_bindings_manifest(source: &str) -> Result<NativeBindings, ManifestError> {
    let manifest = toml::from_str::<Manifest>(source).map_err(|error| {
        ManifestError::new(
            format!("invalid bindings TOML: {error}"),
            error.span(),
            None,
        )
    })?;
    if manifest.schema != 1 {
        return Err(ManifestError::message(format!(
            "unsupported bindings schema {}; expected 1",
            manifest.schema
        )));
    }

    let builtins = standard_bindings().map_err(|error| {
        ManifestError::message(format!("invalid compiler-provided bindings: {error}"))
    })?;
    let mut known_arity = builtins
        .types()
        .map(|binding| {
            (
                binding.stainless_path.clone(),
                binding.type_parameters.len(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    known_arity.insert("rust::Option".to_owned(), 1);
    known_arity.insert("rust::Result".to_owned(), 2);

    let mut dependencies = BTreeMap::new();
    let mut types = Vec::new();
    let mut type_indices = BTreeMap::new();
    for entry in manifest.types {
        validate_type_entry(&entry)?;
        if type_indices.contains_key(&entry.stainless_path) {
            return Err(ManifestError::message(format!(
                "duplicate manifest type `{}`",
                entry.stainless_path
            )));
        }
        let index = types.len();
        type_indices.insert(entry.stainless_path.clone(), index);
        known_arity.insert(entry.stainless_path.clone(), 0);
        dependencies.insert(entry.stainless_path.clone(), entry.dependency.clone());
        types.push(NativeTypeBinding {
            stainless_path: entry.stainless_path,
            rust_path: absolute_rust_path(&entry.rust_path),
            type_parameters: Vec::new(),
            error_format: entry.error_format.map(Into::into),
            callables: Vec::new(),
        });
    }

    for entry in manifest.functions {
        validate_dependency_path(&entry.dependency, &entry.rust_path, "function Rust path")?;
        validate_stainless_path(&entry.stainless_path, "function Stainless path")?;
        let (owner, source_name) = split_callable_path(&entry.stainless_path)?;
        validate_declared_owner(&owner, &entry.dependency, &dependencies)?;
        let callable = callable(
            source_name,
            CallStyle::AssociatedFunction,
            None,
            &entry.parameters,
            &entry.return_type,
            RustLowering::GeneratedWrapper {
                wrapper_name: wrapper_name(
                    &entry.dependency,
                    &entry.rust_path,
                    CallStyle::AssociatedFunction,
                    &entry.parameters,
                    &entry.return_type,
                ),
                target: WrapperTarget::Function {
                    rust_path: absolute_rust_path(&entry.rust_path),
                },
            },
            &known_arity,
        )?;
        attach_callable(&mut types, &type_indices, &owner, callable)?;
    }

    for entry in manifest.methods {
        validate_stainless_identifier(&entry.stainless_name, "method Stainless name")?;
        validate_rust_identifier(&entry.rust_name, "method Rust name")?;
        let dependency = dependencies
            .get(&entry.receiver_type)
            .ok_or_else(|| {
                ManifestError::message(format!(
                    "method receiver type `{}` is not declared in this manifest",
                    entry.receiver_type
                ))
            })?
            .clone();
        let receiver = match entry.receiver {
            ManifestReceiver::Value => Receiver::Value,
            ManifestReceiver::Const => Receiver::Shared,
            ManifestReceiver::Mut => Receiver::Mutable,
        };
        let callable = callable(
            entry.stainless_name.clone(),
            CallStyle::Method,
            Some(receiver),
            &entry.parameters,
            &entry.return_type,
            RustLowering::GeneratedWrapper {
                wrapper_name: wrapper_name(
                    &dependency,
                    &format!("{}::{}", entry.receiver_type, entry.rust_name),
                    CallStyle::Method,
                    &entry.parameters,
                    &entry.return_type,
                ),
                target: WrapperTarget::Method {
                    rust_name: entry.rust_name,
                },
            },
            &known_arity,
        )?;
        attach_callable(&mut types, &type_indices, &entry.receiver_type, callable)?;
    }

    NativeBindings::new(types)
        .map_err(|error| ManifestError::message(format!("invalid binding metadata: {error}")))
}

fn attach_callable(
    types: &mut [NativeTypeBinding],
    type_indices: &BTreeMap<String, usize>,
    owner: &str,
    callable: CallableBinding,
) -> Result<(), ManifestError> {
    let index = type_indices.get(owner).copied().ok_or_else(|| {
        ManifestError::message(format!(
            "callable owner type `{owner}` is not declared in this manifest"
        ))
    })?;
    let binding = types.get_mut(index).ok_or_else(|| {
        ManifestError::message(format!(
            "internal binding index for callable owner `{owner}` is invalid"
        ))
    })?;
    binding.callables.push(callable);
    Ok(())
}

/// Loads one bindings manifest file without adding compiler built-ins.
///
/// # Errors
///
/// Returns a [`ManifestError`] when the file cannot be read or its contents
/// are invalid.
pub fn load_bindings_manifest(path: impl AsRef<Path>) -> Result<NativeBindings, ManifestError> {
    let path = path.as_ref();
    let source = fs::read_to_string(path).map_err(|error| manifest_read_error(path, &error))?;
    parse_manifest_file_source(path, &source)
}

/// Loads compiler built-ins plus an optional package-root bindings manifest.
///
/// A missing `stainless-bindings.toml` is equivalent to a package with no
/// external bindings.
///
/// # Errors
///
/// Returns a [`ManifestError`] when built-in metadata is invalid or the
/// package manifest exists but cannot be loaded and merged.
pub fn load_package_bindings(
    package_root: impl AsRef<Path>,
) -> Result<NativeBindings, ManifestError> {
    let builtins = standard_bindings().map_err(|error| {
        ManifestError::message(format!("invalid compiler-provided bindings: {error}"))
    })?;
    let path = package_root.as_ref().join(BINDINGS_MANIFEST_FILENAME);
    let source = match fs::read_to_string(&path) {
        Ok(source) => source,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(builtins),
        Err(error) => return Err(manifest_read_error(&path, &error)),
    };
    let external = parse_manifest_file_source(&path, &source)?;
    builtins.merge(external).map_err(|error| {
        ManifestError::new(
            format!("binding registry conflict: {error}"),
            None,
            Some(path),
        )
    })
}

fn parse_manifest_file_source(path: &Path, source: &str) -> Result<NativeBindings, ManifestError> {
    parse_bindings_manifest(source).map_err(|mut error| {
        error.path = Some(path.to_path_buf());
        error
    })
}

fn manifest_read_error(path: &Path, error: &std::io::Error) -> ManifestError {
    ManifestError::new(
        format!("failed to read {}: {error}", path.display()),
        None,
        Some(path.to_path_buf()),
    )
}

fn callable(
    source_name: String,
    style: CallStyle,
    receiver: Option<Receiver>,
    parameter_sources: &[ManifestParameter],
    return_source: &str,
    lowering: RustLowering,
    known_arity: &BTreeMap<String, usize>,
) -> Result<CallableBinding, ManifestError> {
    let parameters = parameter_sources
        .iter()
        .enumerate()
        .map(|(index, source)| {
            let (ty, adaptation) = match source {
                ManifestParameter::Value(source) => {
                    let ty = parse_type(source, known_arity)?;
                    reject_nested_reference(&ty, "parameter")?;
                    if ty == TypeRef::Void {
                        return Err(ManifestError::message(
                            "`void` is not a valid binding parameter type",
                        ));
                    }
                    let adaptation =
                        if ty == TypeRef::shared_ref(TypeRef::native("rust::String", Vec::new())) {
                            ArgumentAdaptation::StringRefToStr
                        } else {
                            ArgumentAdaptation::Identity
                        };
                    (ty, adaptation)
                }
                ManifestParameter::Callback { callback } => (
                    parse_callback_type(callback, known_arity)?,
                    ArgumentAdaptation::Identity,
                ),
            };
            Ok(Parameter::adapted(
                format!("argument_{index}"),
                ty,
                adaptation,
            ))
        })
        .collect::<Result<Vec<_>, ManifestError>>()?;
    let return_type = parse_type(return_source, known_arity)?;
    Ok(CallableBinding {
        source_name,
        style,
        receiver,
        parameters,
        return_type,
        return_borrow: None,
        requirements: Vec::new(),
        lowering,
    })
}

fn parse_callback_type(
    callback: &ManifestCallback,
    known_arity: &BTreeMap<String, usize>,
) -> Result<TypeRef, ManifestError> {
    let escape = match callback.escape {
        ManifestCallbackEscape::Call => CallbackEscape::Call,
        ManifestCallbackEscape::Static | ManifestCallbackEscape::Thread => {
            return Err(ManifestError::message(
                "only non-escaping callbacks with `escape = \"call\"` are implemented",
            ));
        }
    };
    let kind = match callback.kind {
        ManifestCallbackKind::Fn => CallbackKind::Fn,
        ManifestCallbackKind::FnMut => CallbackKind::FnMut,
        ManifestCallbackKind::FnOnce => CallbackKind::FnOnce,
        ManifestCallbackKind::FnPtr => CallbackKind::FunctionPointer,
    };
    let parameters = callback
        .parameters
        .iter()
        .map(|source| {
            let ty = parse_type(source, known_arity)?;
            reject_nested_reference(&ty, "callback parameter")?;
            if ty == TypeRef::Void {
                return Err(ManifestError::message(
                    "`void` is not a valid callback parameter type",
                ));
            }
            Ok(ty)
        })
        .collect::<Result<Vec<_>, ManifestError>>()?;
    let return_type = parse_type(&callback.return_type, known_arity)?;
    if return_type.is_reference() || return_type.contains_reference() {
        return Err(ManifestError::message(
            "callback return references are not supported",
        ));
    }
    Ok(TypeRef::callback(kind, escape, parameters, return_type))
}

fn parse_type(
    source: &str,
    known_arity: &BTreeMap<String, usize>,
) -> Result<TypeRef, ManifestError> {
    let probe = format!("void __stainless_binding_probe({source} value);");
    let parse = stainless_syntax::parse(&probe);
    if !parse.errors().is_empty() {
        let messages = parse
            .errors()
            .iter()
            .map(|error| error.message.as_str())
            .collect::<Vec<_>>()
            .join("; ");
        return Err(ManifestError::message(format!(
            "invalid Stainless binding type `{source}`: {messages}"
        )));
    }
    let lowered = crate::lowering::lower(&parse.tree());
    let Some(Item::Function(function)) = lowered.items.first() else {
        return Err(ManifestError::message(format!(
            "invalid Stainless binding type `{source}`"
        )));
    };
    let Some(parameter) = function.parameters.first() else {
        return Err(ManifestError::message(format!(
            "invalid Stainless binding type `{source}`"
        )));
    };
    lower_manifest_type(&parameter.ty, known_arity)
}

fn lower_manifest_type(
    ty: &ast::Type,
    known_arity: &BTreeMap<String, usize>,
) -> Result<TypeRef, ManifestError> {
    if ty.is_const && !ty.is_reference {
        return Err(ManifestError::message(
            "`const` is only meaningful on reference types in binding signatures",
        ));
    }
    let mut value = match &ty.kind {
        TypeKind::Inferred => {
            return Err(ManifestError::message(
                "`auto` is not valid in binding signatures",
            ));
        }
        TypeKind::Error => {
            return Err(ManifestError::message(
                "recovered type errors are not valid in binding signatures",
            ));
        }
        TypeKind::Named(named) => {
            let path = named.path.display();
            if let Some(primitive) = manifest_primitive(&path) {
                if !named.arguments.is_empty() {
                    return Err(ManifestError::message(format!(
                        "primitive binding type `{path}` cannot have type arguments"
                    )));
                }
                primitive
            } else {
                let Some(expected) = known_arity.get(&path) else {
                    return Err(ManifestError::message(format!(
                        "binding signature uses undeclared native type `{path}`"
                    )));
                };
                if *expected != named.arguments.len() {
                    return Err(ManifestError::message(format!(
                        "binding type `{path}` expects {expected} type argument(s), found {}",
                        named.arguments.len()
                    )));
                }
                TypeRef::native(
                    path,
                    named
                        .arguments
                        .iter()
                        .map(|argument| lower_manifest_type(argument, known_arity))
                        .collect::<Result<Vec<_>, _>>()?,
                )
            }
        }
    };
    if ty.is_reference {
        if value == TypeRef::Void {
            return Err(ManifestError::message(
                "`void` cannot be a binding reference target",
            ));
        }
        value = if ty.is_const {
            TypeRef::shared_ref(value)
        } else {
            TypeRef::mutable_ref(value)
        };
    }
    Ok(value)
}

fn manifest_primitive(path: &str) -> Option<TypeRef> {
    Some(match path {
        "void" => TypeRef::Void,
        "bool" => TypeRef::Bool,
        "char" => TypeRef::Char,
        "i8" => TypeRef::I8,
        "i16" => TypeRef::I16,
        "i32" => TypeRef::I32,
        "i64" => TypeRef::I64,
        "i128" => TypeRef::I128,
        "isize" => TypeRef::Isize,
        "u8" => TypeRef::U8,
        "u16" => TypeRef::U16,
        "u32" => TypeRef::U32,
        "u64" => TypeRef::U64,
        "u128" => TypeRef::U128,
        "usize" => TypeRef::Usize,
        "f32" => TypeRef::F32,
        "f64" => TypeRef::F64,
        _ => return None,
    })
}

fn reject_nested_reference(ty: &TypeRef, role: &str) -> Result<(), ManifestError> {
    let unsupported = match ty {
        TypeRef::Reference { target, .. } => target.contains_reference(),
        _ => ty.contains_reference(),
    };
    if unsupported {
        return Err(ManifestError::message(format!(
            "reference-bearing {role} values are not supported in initial bindings"
        )));
    }
    Ok(())
}

fn validate_type_entry(entry: &ManifestType) -> Result<(), ManifestError> {
    if entry.representation != Representation::Opaque {
        return Err(ManifestError::message(format!(
            "type `{}` uses unsupported representation; only `opaque` is implemented",
            entry.stainless_path
        )));
    }
    validate_dependency_path(&entry.dependency, &entry.rust_path, "type Rust path")?;
    validate_stainless_path(&entry.stainless_path, "type Stainless path")?;
    let dependency = dependency_identifier(&entry.dependency);
    let expected_prefix = format!("rust::{dependency}");
    if entry.stainless_path != expected_prefix
        && !entry
            .stainless_path
            .starts_with(&format!("{expected_prefix}::"))
    {
        return Err(ManifestError::message(format!(
            "Stainless path `{}` must be below `{expected_prefix}`",
            entry.stainless_path
        )));
    }
    Ok(())
}

fn validate_dependency_path(
    dependency: &str,
    rust_path: &str,
    description: &str,
) -> Result<(), ManifestError> {
    validate_dependency(dependency)?;
    validate_rust_path(rust_path, description)?;
    let dependency = dependency_identifier(dependency);
    let normalized = rust_path.trim_start_matches("::");
    if normalized != dependency && !normalized.starts_with(&format!("{dependency}::")) {
        return Err(ManifestError::message(format!(
            "{description} `{rust_path}` is outside dependency `{dependency}`"
        )));
    }
    Ok(())
}

fn validate_dependency(dependency: &str) -> Result<(), ManifestError> {
    if dependency.is_empty()
        || !dependency
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
    {
        return Err(ManifestError::message(format!(
            "invalid Cargo dependency key `{dependency}`"
        )));
    }
    validate_rust_identifier(
        &dependency_identifier(dependency),
        "Cargo dependency crate name",
    )
}

fn validate_rust_path(path: &str, description: &str) -> Result<(), ManifestError> {
    let normalized = path.trim_start_matches("::");
    let parsed = syn::parse_str::<syn::Path>(normalized).map_err(|error| {
        ManifestError::message(format!("invalid {description} `{path}`: {error}"))
    })?;
    if parsed.segments.is_empty()
        || parsed
            .segments
            .iter()
            .any(|segment| !matches!(segment.arguments, syn::PathArguments::None))
    {
        return Err(ManifestError::message(format!(
            "invalid {description} `{path}`: expected an item path without type arguments"
        )));
    }
    Ok(())
}

fn validate_rust_identifier(identifier: &str, description: &str) -> Result<(), ManifestError> {
    syn::parse_str::<syn::Ident>(identifier)
        .map(|_| ())
        .map_err(|error| {
            ManifestError::message(format!("invalid {description} `{identifier}`: {error}"))
        })
}

fn validate_stainless_path(path: &str, description: &str) -> Result<(), ManifestError> {
    if path.is_empty() {
        return Err(ManifestError::message(format!(
            "invalid {description}: path is empty"
        )));
    }
    for segment in path.split("::") {
        validate_stainless_identifier(segment, description)?;
    }
    let probe = format!("void __stainless_binding_path_probe({path} value);");
    let parse = stainless_syntax::parse(&probe);
    if !parse.errors().is_empty() {
        return Err(ManifestError::message(format!(
            "invalid {description} `{path}`: path is not valid Stainless syntax"
        )));
    }
    Ok(())
}

fn validate_stainless_identifier(identifier: &str, description: &str) -> Result<(), ManifestError> {
    let mut characters = identifier.chars();
    let valid_start = characters
        .next()
        .is_some_and(|character| character.is_ascii_alphabetic() || character == '_');
    if !valid_start
        || !characters.all(|character| character.is_ascii_alphanumeric() || character == '_')
        || identifier.starts_with("__")
    {
        return Err(ManifestError::message(format!(
            "invalid {description} segment `{identifier}`"
        )));
    }
    Ok(())
}

fn validate_declared_owner(
    owner: &str,
    dependency: &str,
    dependencies: &BTreeMap<String, String>,
) -> Result<(), ManifestError> {
    match dependencies.get(owner) {
        Some(actual) if actual == dependency => Ok(()),
        Some(actual) => Err(ManifestError::message(format!(
            "callable owner `{owner}` belongs to dependency `{actual}`, not `{dependency}`"
        ))),
        None => Err(ManifestError::message(format!(
            "callable owner type `{owner}` is not declared in this manifest"
        ))),
    }
}

fn split_callable_path(path: &str) -> Result<(String, String), ManifestError> {
    let Some((owner, name)) = path.rsplit_once("::") else {
        return Err(ManifestError::message(format!(
            "associated callable path `{path}` must include its declared native type"
        )));
    };
    if owner.is_empty() || name.is_empty() {
        return Err(ManifestError::message(format!(
            "invalid associated callable path `{path}`"
        )));
    }
    Ok((owner.to_owned(), name.to_owned()))
}

fn absolute_rust_path(path: &str) -> String {
    format!("::{}", path.trim_start_matches("::"))
}

fn dependency_identifier(dependency: &str) -> String {
    dependency.replace('-', "_")
}

fn wrapper_name(
    dependency: &str,
    target: &str,
    style: CallStyle,
    parameters: &[ManifestParameter],
    return_type: &str,
) -> String {
    let signature = format!("{dependency}|{target}|{style:?}|{parameters:?}|{return_type}");
    let hash = signature
        .bytes()
        .fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
            (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3)
        });
    let readable = target
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character
            } else {
                '_'
            }
        })
        .take(80)
        .collect::<String>();
    format!("__stainless_wrapper_v1_{readable}_{hash:016x}")
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    schema: u32,
    #[serde(rename = "type", default)]
    types: Vec<ManifestType>,
    #[serde(rename = "function", default)]
    functions: Vec<ManifestFunction>,
    #[serde(rename = "method", default)]
    methods: Vec<ManifestMethod>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ManifestType {
    dependency: String,
    rust_path: String,
    stainless_path: String,
    representation: Representation,
    error_format: Option<ManifestErrorFormat>,
}

#[derive(Clone, Copy, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
enum Representation {
    Opaque,
    FrozenAdapter,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ManifestErrorFormat {
    Display,
    Debug,
}

impl From<ManifestErrorFormat> for NativeErrorFormat {
    fn from(value: ManifestErrorFormat) -> Self {
        match value {
            ManifestErrorFormat::Display => Self::Display,
            ManifestErrorFormat::Debug => Self::Debug,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ManifestFunction {
    dependency: String,
    rust_path: String,
    stainless_path: String,
    parameters: Vec<ManifestParameter>,
    #[serde(rename = "return")]
    return_type: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ManifestMethod {
    receiver_type: String,
    rust_name: String,
    stainless_name: String,
    receiver: ManifestReceiver,
    parameters: Vec<ManifestParameter>,
    #[serde(rename = "return")]
    return_type: String,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ManifestReceiver {
    Value,
    Const,
    Mut,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum ManifestParameter {
    Value(String),
    Callback { callback: ManifestCallback },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ManifestCallback {
    kind: ManifestCallbackKind,
    parameters: Vec<String>,
    #[serde(rename = "return")]
    return_type: String,
    escape: ManifestCallbackEscape,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ManifestCallbackKind {
    Fn,
    FnMut,
    FnOnce,
    FnPtr,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ManifestCallbackEscape {
    Call,
    Static,
    Thread,
}

/// Error reported while loading user-authored native binding metadata.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManifestError {
    message: String,
    span: Option<Range<usize>>,
    path: Option<PathBuf>,
}

impl ManifestError {
    fn message(message: impl Into<String>) -> Self {
        Self::new(message.into(), None, None)
    }

    fn new(message: String, span: Option<Range<usize>>, path: Option<PathBuf>) -> Self {
        Self {
            message,
            span,
            path,
        }
    }

    /// Human-readable validation message.
    #[must_use]
    pub fn message_text(&self) -> &str {
        &self.message
    }

    /// Byte range supplied by the TOML parser when available.
    #[must_use]
    pub const fn span(&self) -> Option<&Range<usize>> {
        self.span.as_ref()
    }

    /// Manifest path when the error came from a file.
    #[must_use]
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }
}

impl fmt::Display for ManifestError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(path) = &self.path {
            write!(formatter, "{}: {}", path.display(), self.message)
        } else {
            formatter.write_str(&self.message)
        }
    }
}

impl Error for ManifestError {}
