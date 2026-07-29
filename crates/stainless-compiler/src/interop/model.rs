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
    /// A non-null borrow used as a parameter, local, or direct return.
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

    /// Returns whether this type is or contains a reference.
    #[must_use]
    pub fn contains_reference(&self) -> bool {
        match self {
            Self::Native { arguments, .. } => arguments.iter().any(Self::contains_reference),
            Self::Reference { .. } => true,
            _ => false,
        }
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

/// The input borrow that owns a directly returned reference.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReturnBorrow {
    /// The result is borrowed from a method's receiver.
    Receiver,
    /// The result is borrowed from the parameter at this zero-based index.
    Parameter(usize),
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
    /// Provenance for a direct reference return.
    pub return_borrow: Option<ReturnBorrow>,
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
                validate_return_borrow(native_type.stainless_path, callable)?;

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

fn validate_return_borrow(
    type_path: &'static str,
    callable: &CallableBinding,
) -> Result<(), BindingError> {
    let Some(return_mutable) = direct_reference_mutability(type_path, callable)? else {
        return if callable.return_borrow.is_some() {
            invalid_return_borrow(type_path, callable, ReturnBorrowError::UnexpectedProvenance)
        } else {
            Ok(())
        };
    };

    let Some(source) = callable.return_borrow else {
        return invalid_return_borrow(type_path, callable, ReturnBorrowError::MissingProvenance);
    };

    if callable.style == CallStyle::Constructor {
        return invalid_return_borrow(
            type_path,
            callable,
            ReturnBorrowError::ConstructorCannotReturnReference,
        );
    }

    match source {
        ReturnBorrow::Receiver => {
            validate_receiver_return_borrow(type_path, callable, return_mutable)
        }
        ReturnBorrow::Parameter(index) => {
            validate_parameter_return_borrow(type_path, callable, index, return_mutable)
        }
    }
}

fn direct_reference_mutability(
    type_path: &'static str,
    callable: &CallableBinding,
) -> Result<Option<bool>, BindingError> {
    let mutability = match &callable.return_type {
        TypeRef::Reference { mutable, target } if !target.contains_reference() => Some(*mutable),
        TypeRef::Reference { .. } => {
            return invalid_return_borrow(
                type_path,
                callable,
                ReturnBorrowError::NestedReferencesDeferred,
            );
        }
        return_type if return_type.contains_reference() => {
            return invalid_return_borrow(
                type_path,
                callable,
                ReturnBorrowError::ReferenceBearingValuesDeferred,
            );
        }
        _ => None,
    };
    Ok(mutability)
}

fn validate_receiver_return_borrow(
    type_path: &'static str,
    callable: &CallableBinding,
    return_mutable: bool,
) -> Result<(), BindingError> {
    let Some(receiver) = callable.receiver else {
        return invalid_return_borrow(
            type_path,
            callable,
            ReturnBorrowError::ReceiverSourceRequiresMethod,
        );
    };
    match receiver {
        Receiver::Value => {
            invalid_return_borrow(type_path, callable, ReturnBorrowError::ConsumedReceiver)
        }
        Receiver::Shared if return_mutable => invalid_return_borrow(
            type_path,
            callable,
            ReturnBorrowError::MutableReturnFromSharedSource,
        ),
        Receiver::Shared | Receiver::Mutable => Ok(()),
    }
}

fn validate_parameter_return_borrow(
    type_path: &'static str,
    callable: &CallableBinding,
    index: usize,
    return_mutable: bool,
) -> Result<(), BindingError> {
    if callable.style == CallStyle::Method {
        return invalid_return_borrow(
            type_path,
            callable,
            ReturnBorrowError::MethodReturnMustBorrowReceiver,
        );
    }

    if callable
        .parameters
        .iter()
        .filter(|parameter| parameter.ty.is_reference())
        .count()
        != 1
    {
        return invalid_return_borrow(
            type_path,
            callable,
            ReturnBorrowError::ExactlyOneReferenceParameterRequired,
        );
    }

    let Some(parameter) = callable.parameters.get(index) else {
        return invalid_return_borrow(type_path, callable, ReturnBorrowError::ParameterOutOfRange);
    };
    let TypeRef::Reference { mutable, .. } = parameter.ty else {
        return invalid_return_borrow(
            type_path,
            callable,
            ReturnBorrowError::ParameterIsNotReference,
        );
    };
    if return_mutable && !mutable {
        return invalid_return_borrow(
            type_path,
            callable,
            ReturnBorrowError::MutableReturnFromSharedSource,
        );
    }
    Ok(())
}

fn invalid_return_borrow<T>(
    type_path: &'static str,
    callable: &CallableBinding,
    reason: ReturnBorrowError,
) -> Result<T, BindingError> {
    Err(BindingError::InvalidReturnBorrow {
        type_path,
        callable: callable.source_name,
        reason,
    })
}

/// Why a native callable's returned-reference metadata is invalid.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReturnBorrowError {
    /// A direct reference return omitted its source borrow.
    MissingProvenance,
    /// A value return declared reference provenance.
    UnexpectedProvenance,
    /// References to references are not part of the initial lifetime model.
    NestedReferencesDeferred,
    /// Values such as `Option<&T>` that carry borrows are deferred.
    ReferenceBearingValuesDeferred,
    /// Constructors must produce values.
    ConstructorCannotReturnReference,
    /// Receiver provenance was attached to a callable without a receiver.
    ReceiverSourceRequiresMethod,
    /// A consumed receiver cannot own a returned borrow.
    ConsumedReceiver,
    /// A mutable reference cannot originate from a shared borrow.
    MutableReturnFromSharedSource,
    /// A method's returned reference must be tied to its receiver.
    MethodReturnMustBorrowReceiver,
    /// A non-method reference return currently requires one reference input.
    ExactlyOneReferenceParameterRequired,
    /// Parameter provenance named an index outside the signature.
    ParameterOutOfRange,
    /// Parameter provenance named a value parameter.
    ParameterIsNotReference,
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
    /// A callable has invalid or unsupported returned-reference metadata.
    InvalidReturnBorrow {
        /// Containing type.
        type_path: &'static str,
        /// Invalid callable.
        callable: &'static str,
        /// Violated returned-borrow rule.
        reason: ReturnBorrowError,
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
            Self::InvalidReturnBorrow {
                type_path,
                callable,
                reason,
            } => write!(
                formatter,
                "invalid returned borrow on native callable `{type_path}::{callable}`: {}",
                reason.description()
            ),
        }
    }
}

impl Error for BindingError {}

impl ReturnBorrowError {
    const fn description(self) -> &'static str {
        match self {
            Self::MissingProvenance => "a direct reference return requires borrow provenance",
            Self::UnexpectedProvenance => "a value return cannot declare borrow provenance",
            Self::NestedReferencesDeferred => "nested reference return types are deferred",
            Self::ReferenceBearingValuesDeferred => {
                "return values containing references are deferred"
            }
            Self::ConstructorCannotReturnReference => "a constructor cannot return a reference",
            Self::ReceiverSourceRequiresMethod => "receiver provenance requires a method receiver",
            Self::ConsumedReceiver => "a consumed receiver cannot own a returned reference",
            Self::MutableReturnFromSharedSource => {
                "a mutable reference cannot originate from a shared borrow"
            }
            Self::MethodReturnMustBorrowReceiver => {
                "a method reference return must be tied to its receiver"
            }
            Self::ExactlyOneReferenceParameterRequired => {
                "a non-method reference return requires exactly one reference parameter"
            }
            Self::ParameterOutOfRange => "the borrow-source parameter index is out of range",
            Self::ParameterIsNotReference => "the borrow-source parameter is not a reference",
        }
    }
}
