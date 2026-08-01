//! Name and type resolution for the currently lowered Stainless subset.

mod imports;
mod mangle;
mod resolver;

use crate::Diagnostic;
use crate::ast::Span;
use crate::interop::{ArgumentAdaptation, CallStyle, Receiver, RustLowering, TypeRef};

pub use resolver::resolve;

/// Stable index of a resolved Stainless function.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct FunctionId(pub usize);

/// Stable index of a resolved Stainless struct.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct StructId(pub usize);

/// Stable index of a resolved Stainless constructor.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ConstructorId(pub usize);

/// One resolved direct data field.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FieldSymbol {
    /// Source field name.
    pub name: String,
    /// Resolved field type.
    pub ty: TypeRef,
    /// Source range.
    pub span: Span,
}

/// A resolved data-only struct.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StructSymbol {
    /// Stable semantic ID.
    pub id: StructId,
    /// Fully qualified source path.
    pub path: Vec<String>,
    /// Optional single data base.
    pub base: Option<StructId>,
    /// Direct fields in aggregate initialization order.
    pub fields: Vec<FieldSymbol>,
    /// Definition source range.
    pub span: Span,
}

/// The implicit receiver attached to a member function.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StructReceiver {
    /// Static receiver struct.
    pub structure: StructId,
    /// Whether the member function may mutate its receiver.
    pub mutable: bool,
}

/// A resolved user-defined or synthesized struct constructor.
#[derive(Clone, Debug, Eq, PartialEq)]
#[allow(clippy::struct_excessive_bools)]
pub struct ConstructorSymbol {
    /// Stable semantic ID.
    pub id: ConstructorId,
    /// Constructed struct.
    pub structure: StructId,
    /// Resolved parameters.
    pub parameters: Vec<ParameterSymbol>,
    /// Declared checked exception set.
    pub throws: Vec<StructId>,
    /// Deterministic generated Rust function name.
    pub mangled_name: String,
    /// All matching declaration/definition ranges.
    pub declarations: Vec<Span>,
    /// Whether an out-of-struct body exists or the constructor is synthesized.
    pub has_definition: bool,
    /// Whether the signature appeared inside the struct body.
    pub has_member_declaration: bool,
    /// Whether construction is explicitly or implicitly deleted.
    pub is_deleted: bool,
    /// Whether the compiler synthesized this default constructor.
    pub synthesized: bool,
    /// Base and direct-field construction in representation order.
    pub initializations: Vec<ConstructorFieldInitialization>,
}

/// One resolved base or field initialization performed by a constructor.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConstructorFieldInitialization {
    /// Generated Rust representation field.
    pub rust_name: String,
    /// Field type.
    pub ty: TypeRef,
    /// Explicit initializer span, absent for implicit default construction.
    pub source: Option<Span>,
    /// Selected construction operation.
    pub call: ResolvedCall,
}

/// A resolved function parameter.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParameterSymbol {
    /// Source binding name.
    pub name: String,
    /// Declared type, including value/reference passing mode.
    pub ty: TypeRef,
    /// Source range.
    pub span: Span,
}

/// A declared Stainless function after signature resolution.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FunctionSymbol {
    /// Stable ID used by resolved call sites.
    pub id: FunctionId,
    /// Fully qualified source path.
    pub path: Vec<String>,
    /// Resolved parameters.
    pub parameters: Vec<ParameterSymbol>,
    /// Resolved return type.
    pub return_type: TypeRef,
    /// Declared checked exception set.
    pub throws: Vec<StructId>,
    /// Implicit member receiver, absent for free functions.
    pub receiver: Option<StructReceiver>,
    /// Deterministic generated Rust name.
    pub mangled_name: String,
    /// All matching declaration/definition ranges.
    pub declarations: Vec<Span>,
    /// Whether one declaration supplies a body.
    pub has_definition: bool,
    /// Whether the member signature was declared inside its struct body.
    pub has_member_declaration: bool,
}

/// Whether an expression denotes storage or a temporary value.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ValueCategory {
    /// A place that may be mutably borrowed or assigned.
    MutablePlace,
    /// A place that permits only shared access.
    SharedPlace,
    /// A value without a source binding that could be used afterward.
    Temporary,
}

/// One expression's resolved type and optional call target.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExpressionResolution {
    /// Expression source range.
    pub span: Span,
    /// Resolved Stainless type.
    pub ty: TypeRef,
    /// Value/place behavior needed by ownership analysis.
    pub category: ValueCategory,
    /// Call classification when this expression is a call.
    pub call: Option<ResolvedCall>,
    /// Struct field selected by this expression, including implicit member
    /// field names.
    pub field: Option<ResolvedField>,
}

/// A field access after inherited-field lookup.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedField {
    /// Rust representation fields traversed from the receiver.
    pub access_path: Vec<String>,
}

/// A resolved local or range-loop binding.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BindingResolution {
    /// Binding source range.
    pub span: Span,
    /// Source name.
    pub name: String,
    /// Resolved type.
    pub ty: TypeRef,
    /// Whether the binding permits mutation.
    pub mutable: bool,
}

/// A resolved callable invocation, including implicit default construction.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedCall {
    /// Call expression or default-constructed declaration range.
    pub span: Span,
    /// Selected callable category.
    pub target: CallTarget,
    /// Concrete return type.
    pub return_type: TypeRef,
    /// Checked exception types that may escape this invocation.
    pub throws: Vec<StructId>,
}

/// Call categories represented by the initial semantic model.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CallTarget {
    /// A Stainless-defined free function.
    Stainless(FunctionId),
    /// A user-defined or synthesized struct constructor.
    Constructor(ConstructorId),
    /// A compiler-described native Rust callable.
    Native(Box<NativeCall>),
    /// A compiler language operation.
    Intrinsic(Intrinsic),
}

/// A native call after generic parameters have been substituted.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NativeCall {
    /// Canonical Stainless type path, such as `rust::Vec`.
    pub type_path: String,
    /// Constructor, associated-function, or method form.
    pub style: CallStyle,
    /// Source-visible callable name.
    pub source_name: String,
    /// Receiver behavior for methods.
    pub receiver: Option<Receiver>,
    /// Concrete receiver type for generated method wrappers.
    pub receiver_type: Option<TypeRef>,
    /// Concrete parameter types.
    pub parameter_types: Vec<TypeRef>,
    /// Rust-boundary argument adaptations.
    pub adaptations: Vec<ArgumentAdaptation>,
    /// Concrete return type.
    pub return_type: TypeRef,
    /// Compiler-inserted checked conversion for a native Rust `Result`.
    pub result_adaptation: Option<NativeCallResultAdaptation>,
    /// Code-generation operation supplied by the registry.
    pub lowering: RustLowering,
    /// Concrete Rust trait obligations retained for later validation.
    pub requirements: Vec<ResolvedTraitRequirement>,
}

/// Checked conversion attached directly to a fallible native callable.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NativeCallResultAdaptation {
    /// Statically selected error-message conversion.
    pub error_message: RustErrorMessage,
    /// Compiler-native checked exception selected from the Rust error type.
    pub exception: NativeResultException,
}

/// A native generic obligation after substitution.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedTraitRequirement {
    /// Concrete type that must implement the trait.
    pub ty: TypeRef,
    /// Fully qualified Rust trait path.
    pub rust_trait: String,
}

/// Compiler intrinsics accepted by the initial resolver.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Intrinsic {
    /// Explicitly consume a named value.
    Move,
    /// Allocate a constructed value into a non-null unique owner.
    MakeOwner {
        /// Unique or shared allocation representation.
        kind: crate::interop::PointerKind,
        /// Allocated pointee type.
        target: TypeRef,
        /// Constructor or direct-initialization operation run before boxing.
        construction: Box<ResolvedCall>,
    },
    /// Default construction of a nullable owner, weak observer, or nullable atomic slot.
    PointerDefault {
        /// Constructed pointer representation.
        kind: crate::interop::PointerKind,
        /// Pointee type.
        target: TypeRef,
    },
    /// Convert between compatible ownership pointer representations.
    PointerConversion {
        /// Source representation.
        from: crate::interop::PointerKind,
        /// Destination representation.
        to: crate::interop::PointerKind,
        /// Pointee type shared by both representations.
        target: TypeRef,
    },
    /// Demote a shared owner to a weak observer.
    DowngradeShared {
        /// Observed pointee type.
        target: TypeRef,
    },
    /// Attempt to promote a weak observer to a nullable shared owner.
    LockWeak {
        /// Observed pointee type.
        target: TypeRef,
    },
    /// Load a shared snapshot from an atomic pointer slot.
    AtomicLoad {
        /// Whether the loaded snapshot is nullable.
        nullable: bool,
        /// Pointee type.
        target: TypeRef,
    },
    /// Store a shared snapshot into an atomic pointer slot.
    AtomicStore {
        /// Whether the stored snapshot is nullable.
        nullable: bool,
        /// Pointee type.
        target: TypeRef,
    },
    /// Swap a shared snapshot through an atomic pointer slot.
    AtomicSwap {
        /// Whether the exchanged snapshot is nullable.
        nullable: bool,
        /// Pointee type.
        target: TypeRef,
    },
    /// Invoke a non-null stored `function` or `function_mut` value.
    StoredFunctionCall {
        /// Whether invocation requires mutable access to the callable.
        mutable: bool,
    },
    /// A constructor-style primitive numeric conversion.
    PrimitiveCast {
        /// Destination primitive type.
        target: TypeRef,
    },
    /// Convert a JSON `var` through its JavaScript-compatible scalar rules.
    JsonCast {
        /// Destination primitive or `rust::String` type.
        target: TypeRef,
    },
    /// Convert one JSON-compatible statically typed value into `var`.
    JsonWrap,
    /// Aggregate construction of a user-defined struct.
    StructAggregate {
        /// Constructed struct.
        structure: StructId,
    },
    /// Compiler-provided `stainless::Exception(message)` construction.
    ExceptionRoot {
        /// Built-in root struct.
        structure: StructId,
    },
    /// Direct initialization from one exact value.
    ValueInitialization {
        /// Constructed value type.
        target: TypeRef,
    },
    /// Consume a native Rust `Result<T, E>` and convert `Err` to `RustError`.
    UnwrapRustResult {
        /// Statically selected error-message conversion.
        error_message: RustErrorMessage,
        /// Compiler-native checked exception chosen from the native error type.
        exception: NativeResultException,
    },
}

/// Statically proven way to obtain a native Rust error message.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RustErrorMessage {
    /// The native error type is known to implement Rust `Display`.
    Display,
    /// The native error type is known to implement Rust `Debug`.
    Debug,
    /// No formatting trait is proven, so use the specified fixed fallback.
    Fallback,
}

/// Checked Stainless exception selected for a native `Result` error type.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeResultException {
    /// Generic failure from Rust interop.
    RustError,
    /// JSON read, parse, or mutation failure.
    JsonError,
}

/// One compiler-inserted exact `Result<T, E>` to `T` conversion.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RustResultAdaptation {
    /// Span of the source expression producing the native Result.
    pub span: Span,
    /// Statically selected error-message conversion.
    pub error_message: RustErrorMessage,
    /// Checked exception raised by the inserted unwrap.
    pub exception: NativeResultException,
}

/// One callback-valued expression selected through contextual binding metadata.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedCallback {
    /// Lambda or named-function expression range.
    pub span: Span,
    /// Exact contextual callback type.
    pub ty: TypeRef,
    /// Source form supplying the callback.
    pub target: CallbackTarget,
}

/// Source operation used to construct a contextual callback.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CallbackTarget {
    /// A Stainless free-function item.
    Function(FunctionId),
    /// An explicit-capture lambda.
    Lambda {
        /// Validated captures in source order.
        captures: Vec<ResolvedLambdaCapture>,
    },
}

/// One validated explicit lambda capture.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedLambdaCapture {
    /// Lambda-local and outer binding name.
    pub name: String,
    /// Resolved captured value type before applying borrow mode.
    pub ty: TypeRef,
    /// Capture ownership operation.
    pub mode: LambdaCaptureMode,
}

/// Ownership operation performed when constructing a lambda.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LambdaCaptureMode {
    /// Clone a Stainless-copyable value into the lambda.
    Copy,
    /// Evaluate an arbitrary owned initializer using normal value semantics.
    Initialize,
    /// Borrow the value for the duration of the native call.
    Borrow {
        /// Whether the outer binding permits mutable access.
        mutable: bool,
    },
}

/// Rust representation retained for one resolved native type.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedNativeType {
    /// Canonical Stainless path below `rust::`.
    pub stainless_path: String,
    /// Fully qualified Rust type path.
    pub rust_path: String,
}

/// Successfully retained semantic facts for one source file.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SemanticModel {
    /// Native type representations copied from the validated binding registry.
    pub native_types: Vec<ResolvedNativeType>,
    /// Resolved Stainless struct definitions.
    pub structs: Vec<StructSymbol>,
    /// Resolved user-defined and synthesized constructors.
    pub constructors: Vec<ConstructorSymbol>,
    /// Resolved Stainless functions.
    pub functions: Vec<FunctionSymbol>,
    /// Expression facts in traversal order.
    pub expressions: Vec<ExpressionResolution>,
    /// Local and range-loop bindings in traversal order.
    pub bindings: Vec<BindingResolution>,
    /// Explicit and implicit calls in traversal order.
    pub calls: Vec<ResolvedCall>,
    /// Target-typed native Result conversions in traversal order.
    pub rust_result_adaptations: Vec<RustResultAdaptation>,
    /// Contextual callback expressions in traversal order.
    pub callbacks: Vec<ResolvedCallback>,
}

impl SemanticModel {
    /// Finds retained native type representation metadata.
    #[must_use]
    pub fn native_type(&self, path: &str) -> Option<&ResolvedNativeType> {
        self.native_types
            .binary_search_by_key(&path, |native| native.stainless_path.as_str())
            .ok()
            .map(|index| &self.native_types[index])
    }

    /// Finds callback metadata for an exact lambda or function-name expression.
    #[must_use]
    pub fn callback(&self, span: Span) -> Option<&ResolvedCallback> {
        self.callbacks.iter().find(|callback| callback.span == span)
    }

    /// Finds a struct by its stable semantic ID.
    #[must_use]
    pub fn structure(&self, id: StructId) -> Option<&StructSymbol> {
        self.structs.get(id.0)
    }

    /// Finds a constructor by stable semantic ID.
    #[must_use]
    pub fn constructor(&self, id: ConstructorId) -> Option<&ConstructorSymbol> {
        self.constructors.get(id.0)
    }

    /// Finds the constructor associated with a declaration or definition.
    #[must_use]
    pub fn constructor_at(&self, span: Span) -> Option<&ConstructorSymbol> {
        self.constructors
            .iter()
            .find(|constructor| constructor.declarations.contains(&span))
    }

    /// Finds the struct declared at an exact source span.
    #[must_use]
    pub fn struct_at(&self, span: Span) -> Option<&StructSymbol> {
        self.structs.iter().find(|structure| structure.span == span)
    }
    /// Finds a function by its stable semantic ID.
    #[must_use]
    pub fn function(&self, id: FunctionId) -> Option<&FunctionSymbol> {
        self.functions.get(id.0)
    }

    /// Finds the function symbol associated with a declaration or definition.
    #[must_use]
    pub fn function_at(&self, span: Span) -> Option<&FunctionSymbol> {
        self.functions
            .iter()
            .find(|function| function.declarations.contains(&span))
    }

    /// Finds the resolution for an exact expression span.
    #[must_use]
    pub fn expression(&self, span: Span) -> Option<&ExpressionResolution> {
        self.expressions
            .iter()
            .find(|expression| expression.span == span)
    }

    /// Finds a local or range-loop binding by its declaration span.
    #[must_use]
    pub fn binding(&self, span: Span) -> Option<&BindingResolution> {
        self.bindings.iter().find(|binding| binding.span == span)
    }

    /// Finds an explicit or implicit call by its source span.
    #[must_use]
    pub fn call(&self, span: Span) -> Option<&ResolvedCall> {
        self.calls.iter().find(|call| call.span == span)
    }

    /// Finds a compiler-inserted native Result conversion by expression span.
    #[must_use]
    pub fn rust_result_adaptation(&self, span: Span) -> Option<&RustResultAdaptation> {
        self.rust_result_adaptations
            .iter()
            .find(|adaptation| adaptation.span == span)
    }
}

/// Result of resolving one compiler AST.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Resolution {
    /// Resolved symbols, expression types, and calls.
    pub model: SemanticModel,
    /// Recoverable resolution diagnostics.
    pub diagnostics: Vec<Diagnostic>,
}
