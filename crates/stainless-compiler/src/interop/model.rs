use std::collections::BTreeSet;
use std::error::Error;
use std::fmt;

/// A type as seen by Stainless semantic analysis.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum TypeRef {
    /// The absence of a return value.
    Void,
    /// Stainless `bool`.
    Bool,
    /// Stainless `char`, which is a Rust Unicode scalar value.
    Char,
    /// Stainless `u8`.
    U8,
    /// Stainless `usize`.
    Usize,
    /// A generic type parameter declared by the native type.
    Parameter(&'static str),
    /// A native Rust type under the reserved Stainless `rust::` namespace.
    Native {
        /// Canonical Stainless path, such as `rust::Vec`.
        path: &'static str,
        /// Explicit generic arguments.
        arguments: Vec<TypeRef>,
    },
    /// A non-escaping function parameter borrow.
    Reference {
        /// Whether the borrowed value may be mutated.
        mutable: bool,
        /// Borrowed type.
        target: Box<TypeRef>,
    },
}

impl TypeRef {
    /// Creates a native type with explicit type arguments.
    #[must_use]
    pub fn native(path: &'static str, arguments: Vec<Self>) -> Self {
        Self::Native { path, arguments }
    }

    /// Creates an immutable parameter reference.
    #[must_use]
    pub fn shared_ref(target: Self) -> Self {
        Self::Reference {
            mutable: false,
            target: Box::new(target),
        }
    }

    /// Creates a mutable parameter reference.
    #[must_use]
    pub fn mutable_ref(target: Self) -> Self {
        Self::Reference {
            mutable: true,
            target: Box::new(target),
        }
    }

    /// Returns whether this type contains a reference at its outermost level.
    #[must_use]
    pub const fn is_reference(&self) -> bool {
        matches!(self, Self::Reference { .. })
    }
}

/// How a callable is written in Stainless source.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CallStyle {
    /// C++ constructor syntax, such as `Vec()`.
    Constructor,
    /// An associated Rust function, such as `Vec::with_capacity(16)`.
    AssociatedFunction,
    /// Receiver syntax, such as `values.push(value)`.
    Method,
}

/// Ownership and mutability of a native method receiver.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Receiver {
    /// Rust `&self`; does not consume or mutate the binding.
    Shared,
    /// Rust `&mut self`; mutates the existing binding.
    Mutable,
    /// Rust `self`; consumes and invalidates a named Stainless binding.
    Value,
}

/// Per-argument conversion performed after exact Stainless call resolution.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ArgumentAdaptation {
    /// Pass the resolved argument without an interop representation change.
    #[default]
    Identity,
    /// Borrow a Stainless `rust::String` as Rust `&str`.
    StringRefToStr,
}

/// One callable parameter.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Parameter {
    /// Diagnostic/source name.
    pub name: &'static str,
    /// Stainless-visible type.
    pub ty: TypeRef,
    /// Rust-boundary conversion applied after call resolution.
    pub adaptation: ArgumentAdaptation,
}

impl Parameter {
    /// Creates an identity-lowered parameter.
    #[must_use]
    pub fn new(name: &'static str, ty: TypeRef) -> Self {
        Self {
            name,
            ty,
            adaptation: ArgumentAdaptation::Identity,
        }
    }

    /// Creates a parameter with an explicit Rust-boundary adaptation.
    #[must_use]
    pub fn adapted(name: &'static str, ty: TypeRef, adaptation: ArgumentAdaptation) -> Self {
        Self {
            name,
            ty,
            adaptation,
        }
    }
}

/// A Rust trait that must hold for a generic native call.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TraitRequirement {
    /// Generic parameter constrained by the call.
    pub parameter: &'static str,
    /// Fully qualified Rust trait path checked again by rustc.
    pub rust_trait: &'static str,
}

/// Code-generation strategy for a resolved callable.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RustLowering {
    /// Call a fully qualified Rust associated function.
    AssociatedFunction { rust_path: &'static str },
    /// Invoke a Rust method on the lowered receiver.
    Method { rust_name: &'static str },
    /// Clone a constructor argument without exposing Rust `From` details.
    CloneArgument { index: usize },
}

/// A constructor, associated function, or method exposed to Stainless.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CallableBinding {
    /// Stainless source name. Constructors use their type's short name.
    pub source_name: &'static str,
    /// Source call form.
    pub style: CallStyle,
    /// Method receiver, absent for constructors and associated functions.
    pub receiver: Option<Receiver>,
    /// Exact Stainless-visible parameters.
    pub parameters: Vec<Parameter>,
    /// Stainless-visible return type.
    pub return_type: TypeRef,
    /// Rust trait requirements attached to this call.
    pub requirements: Vec<TraitRequirement>,
    /// Rust code-generation operation.
    pub lowering: RustLowering,
}

impl CallableBinding {
    /// Returns the exact parameter types used during Stainless call matching.
    #[must_use]
    pub fn parameter_types(&self) -> impl ExactSizeIterator<Item = &TypeRef> {
        self.parameters.iter().map(|parameter| &parameter.ty)
    }
}

/// All built-in call metadata for one native Rust type.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NativeTypeBinding {
    /// Canonical source path below the reserved `rust::` namespace.
    pub stainless_path: &'static str,
    /// Canonical generated Rust type path.
    pub rust_path: &'static str,
    /// Generic type parameters in declaration order.
    pub type_parameters: Vec<&'static str>,
    /// Constructors, associated functions, and methods.
    pub callables: Vec<CallableBinding>,
}

impl NativeTypeBinding {
    /// Finds a callable by style, name, and exact parameter types.
    #[must_use]
    pub fn find_callable(
        &self,
        style: CallStyle,
        source_name: &str,
        parameter_types: &[TypeRef],
    ) -> Option<&CallableBinding> {
        self.callables.iter().find(|callable| {
            callable.style == style
                && callable.source_name == source_name
                && callable.parameter_types().eq(parameter_types)
        })
    }
}

/// Registry of compiler-provided native Rust type bindings.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct NativeBindings {
    types: Vec<NativeTypeBinding>,
}

impl NativeBindings {
    /// Creates a registry and validates invariants needed by semantic analysis.
    ///
    /// # Errors
    ///
    /// Returns a [`BindingError`] when a type or callable violates the
    /// invariants required for deterministic Stainless resolution.
    pub fn new(mut types: Vec<NativeTypeBinding>) -> Result<Self, BindingError> {
        types.sort_by_key(|binding| binding.stainless_path);
        let bindings = Self { types };
        bindings.validate()?;
        Ok(bindings)
    }

    /// Returns every registered type in deterministic path order.
    #[must_use]
    pub fn types(&self) -> impl ExactSizeIterator<Item = &NativeTypeBinding> {
        self.types.iter()
    }

    /// Finds a native type by its canonical Stainless path.
    #[must_use]
    pub fn type_by_path(&self, path: &str) -> Option<&NativeTypeBinding> {
        self.types
            .binary_search_by_key(&path, |binding| binding.stainless_path)
            .ok()
            .map(|index| &self.types[index])
    }

    fn validate(&self) -> Result<(), BindingError> {
        let mut paths = BTreeSet::new();

        for native_type in &self.types {
            if !native_type.stainless_path.starts_with("rust::") {
                return Err(BindingError::InvalidNativePath(native_type.stainless_path));
            }
            if !paths.insert(native_type.stainless_path) {
                return Err(BindingError::DuplicateNativeType(
                    native_type.stainless_path,
                ));
            }

            let mut signatures = BTreeSet::new();
            for callable in &native_type.callables {
                if callable.return_type.is_reference() {
                    return Err(BindingError::ReferenceReturn {
                        type_path: native_type.stainless_path,
                        callable: callable.source_name,
                    });
                }

                let signature = (
                    callable.style as u8,
                    callable.source_name,
                    callable.parameter_types().cloned().collect::<Vec<_>>(),
                );
                if !signatures.insert(signature) {
                    return Err(BindingError::DuplicateCallable {
                        type_path: native_type.stainless_path,
                        callable: callable.source_name,
                    });
                }

                match (callable.style, callable.receiver) {
                    (CallStyle::Method, None) => {
                        return Err(BindingError::MissingReceiver {
                            type_path: native_type.stainless_path,
                            callable: callable.source_name,
                        });
                    }
                    (CallStyle::Constructor | CallStyle::AssociatedFunction, Some(_)) => {
                        return Err(BindingError::UnexpectedReceiver {
                            type_path: native_type.stainless_path,
                            callable: callable.source_name,
                        });
                    }
                    _ => {}
                }
            }
        }

        Ok(())
    }
}

/// Invalid compiler-provided native metadata.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BindingError {
    /// Native type path did not enter through `rust::`.
    InvalidNativePath(&'static str),
    /// Two type bindings used the same canonical source path.
    DuplicateNativeType(&'static str),
    /// Two callable bindings had the same exact Stainless signature.
    DuplicateCallable {
        /// Containing type.
        type_path: &'static str,
        /// Duplicated source name.
        callable: &'static str,
    },
    /// A method omitted its receiver metadata.
    MissingReceiver {
        /// Containing type.
        type_path: &'static str,
        /// Invalid method.
        callable: &'static str,
    },
    /// A non-method declared receiver metadata.
    UnexpectedReceiver {
        /// Containing type.
        type_path: &'static str,
        /// Invalid callable.
        callable: &'static str,
    },
    /// A binding attempted to expose a forbidden reference return.
    ReferenceReturn {
        /// Containing type.
        type_path: &'static str,
        /// Invalid callable.
        callable: &'static str,
    },
}

impl fmt::Display for BindingError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidNativePath(path) => {
                write!(
                    formatter,
                    "native type path `{path}` must start with `rust::`"
                )
            }
            Self::DuplicateNativeType(path) => {
                write!(formatter, "duplicate native type binding for `{path}`")
            }
            Self::DuplicateCallable {
                type_path,
                callable,
            } => write!(
                formatter,
                "duplicate callable `{callable}` on native type `{type_path}`"
            ),
            Self::MissingReceiver {
                type_path,
                callable,
            } => write!(
                formatter,
                "method `{type_path}::{callable}` is missing receiver metadata"
            ),
            Self::UnexpectedReceiver {
                type_path,
                callable,
            } => write!(
                formatter,
                "non-method `{type_path}::{callable}` has receiver metadata"
            ),
            Self::ReferenceReturn {
                type_path,
                callable,
            } => write!(
                formatter,
                "native callable `{type_path}::{callable}` cannot return a reference"
            ),
        }
    }
}

impl Error for BindingError {}
