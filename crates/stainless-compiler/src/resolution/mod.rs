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
    Native(NativeCall),
    /// A compiler language operation.
    Intrinsic(Intrinsic),
}

/// A native call after generic parameters have been substituted.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NativeCall {
    /// Canonical Stainless type path, such as `rust::Vec`.
    pub type_path: &'static str,
    /// Constructor, associated-function, or method form.
    pub style: CallStyle,
    /// Source-visible callable name.
    pub source_name: &'static str,
    /// Receiver behavior for methods.
    pub receiver: Option<Receiver>,
    /// Concrete parameter types.
    pub parameter_types: Vec<TypeRef>,
    /// Rust-boundary argument adaptations.
    pub adaptations: Vec<ArgumentAdaptation>,
    /// Concrete return type.
    pub return_type: TypeRef,
    /// Code-generation operation supplied by the registry.
    pub lowering: RustLowering,
    /// Concrete Rust trait obligations retained for later validation.
    pub requirements: Vec<ResolvedTraitRequirement>,
}

/// A native generic obligation after substitution.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedTraitRequirement {
    /// Concrete type that must implement the trait.
    pub ty: TypeRef,
    /// Fully qualified Rust trait path.
    pub rust_trait: &'static str,
}

/// Compiler intrinsics accepted by the initial resolver.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Intrinsic {
    /// Explicitly consume a named value.
    Move,
    /// A constructor-style primitive numeric conversion.
    PrimitiveCast {
        /// Destination primitive type.
        target: TypeRef,
    },
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
}

/// Successfully retained semantic facts for one source file.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SemanticModel {
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
}

impl SemanticModel {
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
}

/// Result of resolving one compiler AST.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Resolution {
    /// Resolved symbols, expression types, and calls.
    pub model: SemanticModel,
    /// Recoverable resolution diagnostics.
    pub diagnostics: Vec<Diagnostic>,
}
