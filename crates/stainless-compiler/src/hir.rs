//! Typed, Rust-shaped intermediate representation used by the backend.

use crate::ast::{BinaryOperator, LiteralKind, PrefixOperator, Span};
use crate::interop::{
    ArgumentAdaptation, CallbackKind, PointerKind, Receiver, ReturnAdaptation, StoredFunctionKind,
    WrapperTarget,
};

/// A source file after successful semantic resolution and backend lowering.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Program {
    /// Compile-checked adapters for selected external Rust APIs.
    pub native_wrappers: Vec<NativeWrapper>,
    /// Behavior-only interfaces declared directly at crate scope.
    pub interfaces: Vec<Interface>,
    /// Data-only structs declared directly at crate scope.
    pub structs: Vec<Struct>,
    /// Functions declared directly at crate scope.
    pub functions: Vec<Function>,
    /// Nested Stainless namespaces, emitted as Rust modules.
    pub modules: Vec<Module>,
}

/// One generated adapter around an external Rust callable.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NativeWrapper {
    /// Deterministic private Rust function name.
    pub rust_name: String,
    /// Actual external Rust item invoked by the wrapper.
    pub target: WrapperTarget,
    /// Concrete method receiver, absent for free and associated functions.
    pub receiver: Option<NativeWrapperReceiver>,
    /// Concrete wrapper parameters.
    pub parameters: Vec<NativeWrapperParameter>,
    /// Whether the wrapped Rust callable returns a future.
    pub is_async: bool,
    /// Conversion applied after the wrapped call completes.
    pub return_adaptation: ReturnAdaptation,
    /// Concrete wrapper return type.
    pub return_type: Type,
}

/// Concrete receiver of a generated method wrapper.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NativeWrapperReceiver {
    /// Native receiver value type.
    pub ty: Type,
    /// Value or borrow behavior.
    pub mode: Receiver,
}

/// One generated-wrapper parameter and its boundary adaptation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NativeWrapperParameter {
    /// Stainless-visible concrete Rust representation.
    pub ty: Type,
    /// Conversion performed inside the compile-checked wrapper.
    pub adaptation: ArgumentAdaptation,
}

/// One explicit capture materialized before constructing a Rust closure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LambdaCapture {
    /// Generated Rust binding shadowed inside the closure construction block.
    pub rust_name: String,
    /// Whether the closure body may mutate this by-value capture.
    pub mutable: bool,
    /// Copy, move, or borrow initializer.
    pub initializer: Expression,
}

/// A Stainless namespace represented as a Rust module.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Module {
    /// Source namespace name.
    pub source_name: String,
    /// Collision-resistant Rust identifier.
    pub rust_name: String,
    /// Interfaces declared directly in this namespace.
    pub interfaces: Vec<Interface>,
    /// Data-only structs declared directly in this namespace.
    pub structs: Vec<Struct>,
    /// Functions declared directly in this namespace.
    pub functions: Vec<Function>,
    /// Nested namespaces.
    pub modules: Vec<Module>,
}

/// A Rust-representable data-only Stainless struct.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Struct {
    /// Fully qualified Stainless path.
    pub source_path: Vec<String>,
    /// Rust type identifier.
    pub rust_name: String,
    /// Rust generic type parameters in declaration order.
    pub type_parameters: Vec<String>,
    /// Parameters emitted as Rust `const N: usize` generic parameters.
    pub const_parameters: Vec<String>,
    /// Whether generated Rust may derive `Clone` for Stainless copy semantics.
    pub copyable: bool,
    /// Direct representation fields, including an optional base subobject.
    pub fields: Vec<Field>,
    /// Associated integer constants with no instance storage.
    pub static_constants: Vec<StaticConstant>,
    /// Flattened data fields used by automatic structural JSON conversion.
    pub json_fields: Option<Vec<JsonStructField>>,
    /// Whether this struct participates in the checked-exception hierarchy.
    pub is_exception: bool,
    /// Embedded exception-base field, absent on the compiler-provided root.
    pub exception_base_field: Option<String>,
    /// Static Rust trait implementations proven by interface conformance.
    pub interface_implementations: Vec<InterfaceImplementation>,
}

/// One flattened field in a generated struct-to-JSON conversion.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct JsonStructField {
    /// JSON object member name.
    pub name: String,
    /// Generated Rust fields traversed from the converted struct value.
    pub access_path: Vec<String>,
}

/// A Stainless interface lowered to a Rust trait.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Interface {
    /// Fully qualified Stainless path.
    pub source_path: Vec<String>,
    /// Rust trait identifier.
    pub rust_name: String,
    /// Fully qualified Rust supertrait paths.
    pub bases: Vec<String>,
    /// Directly declared interface methods.
    pub methods: Vec<InterfaceMethod>,
}

/// One object-safe method in a generated interface trait.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InterfaceMethod {
    /// Deterministically mangled Rust method name.
    pub rust_name: String,
    /// Whether the receiver is `&mut self` rather than `&self`.
    pub mutable: bool,
    /// Explicit method parameters.
    pub parameters: Vec<Parameter>,
    /// Value or reference return type.
    pub return_type: Type,
    /// Whether the return is wrapped in the checked exception carrier.
    pub throws: bool,
}

/// One generated Rust trait implementation on a concrete struct or class.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InterfaceImplementation {
    /// Fully qualified generated Rust trait path.
    pub interface_path: String,
    /// Required methods and their concrete free-function delegates.
    pub methods: Vec<InterfaceImplementationMethod>,
}

/// Concrete delegate for one generated trait method.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InterfaceImplementationMethod {
    /// Trait method signature.
    pub method: InterfaceMethod,
    /// Namespace modules containing the concrete free function.
    pub function_modules: Vec<String>,
    /// Concrete generated free-function name.
    pub function: String,
    /// Whether a concrete self reference must be erased inside `Result`.
    pub adapt_self_reference: bool,
}

/// One generated Rust struct field.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Field {
    /// Rust field identifier.
    pub rust_name: String,
    /// Resolved field type.
    pub ty: Type,
}

/// One generated Rust associated constant.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StaticConstant {
    /// Rust member identifier.
    pub rust_name: String,
    /// Whether generated Rust exposes this constant publicly.
    pub is_public: bool,
    /// Exact integer type.
    pub ty: Type,
    /// Integer literal spelling.
    pub value: String,
}

/// One resolved function definition.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Function {
    /// Fully qualified Stainless name.
    pub source_path: Vec<String>,
    /// Namespace modules containing the emitted free Rust function.
    pub module_path: Vec<String>,
    /// Deterministically mangled Rust function name.
    pub rust_name: String,
    /// Rust generic type parameters in declaration order.
    pub type_parameters: Vec<String>,
    /// Generic parameters emitted as Rust `const N: usize` parameters.
    pub const_parameters: Vec<String>,
    /// Whether this emits as a Rust `async fn`.
    pub is_async: bool,
    /// Function parameters.
    pub parameters: Vec<Parameter>,
    /// Resolved return type.
    pub return_type: Type,
    /// Whether the Rust return type is wrapped in the erased exception Result.
    pub throws: bool,
    /// Lowered function body.
    pub body: Block,
    /// Source range used for backend diagnostics.
    pub span: Span,
}

/// One function parameter.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Parameter {
    /// Source binding name.
    pub source_name: String,
    /// Rust binding identifier.
    pub rust_name: String,
    /// Resolved parameter type.
    pub ty: Type,
    /// Whether a by-value binding must be mutable.
    pub mutable: bool,
}

/// A Rust-representable Stainless type.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Type {
    /// Rust unit.
    Unit,
    /// A Rust primitive type.
    Primitive(&'static str),
    /// A concrete const-generic `usize` argument.
    ConstUsize(u64),
    /// A named const-generic `usize` parameter.
    ConstParameter(String),
    /// A fixed-size Rust array `[T; N]`.
    Array {
        /// Element type.
        element: Box<Type>,
        /// Concrete or parameterized length.
        length: Box<Type>,
    },
    /// A heterogeneous Rust tuple value.
    Tuple(Vec<Type>),
    /// A native Rust type with concrete type arguments.
    Native {
        /// Fully qualified Rust path.
        rust_path: String,
        /// Concrete type arguments.
        arguments: Vec<Type>,
    },
    /// Compiler-owned reference-counted storage for a class base subobject.
    ClassBase(Box<Type>),
    /// A contextual callback used only as a generated-wrapper parameter.
    Callback {
        /// Whether invocation returns a future.
        is_async: bool,
        /// Required Rust closure trait or function-pointer representation.
        kind: CallbackKind,
        /// Callback lifetime/thread retention contract.
        escape: crate::interop::CallbackEscape,
        /// Exact callback parameter types.
        parameters: Vec<Type>,
        /// Exact value-semantic callback return type.
        return_type: Box<Type>,
    },
    /// A non-null owning callable represented by a Rust trait object.
    Function {
        /// Shared `Fn` or unique `FnMut` storage.
        kind: StoredFunctionKind,
        /// Exact parameter types.
        parameters: Vec<Type>,
        /// Exact return type.
        return_type: Box<Type>,
    },
    /// A compiler-known ownership pointer.
    Pointer {
        /// Ownership representation.
        kind: PointerKind,
        /// Pointee type.
        target: Box<Type>,
    },
    /// A Rust synchronization mutex owning one value.
    Mutex(Box<Type>),
    /// A scoped Rust mutex guard with an inferred borrow lifetime.
    MutexGuard(Box<Type>),
    /// A Rust reader/writer lock owning one value.
    RwLock(Box<Type>),
    /// A scoped Rust shared reader guard with an inferred borrow lifetime.
    RwLockReadGuard(Box<Type>),
    /// A scoped Rust exclusive writer guard with an inferred borrow lifetime.
    RwLockWriteGuard(Box<Type>),
    /// A Rust condition variable.
    Condition,
    /// A move-only Rust thread handle.
    ThreadHandle(Box<Type>),
    /// A Rust lexical thread scope.
    ThreadScope,
    /// A move-only handle tied to a Rust lexical thread scope.
    ScopedThreadHandle(Box<Type>),
    /// A user-defined Stainless struct.
    User {
        /// Fully qualified generated Rust path.
        rust_path: String,
        /// Concrete or generic Rust type arguments.
        arguments: Vec<Type>,
    },
    /// A dynamically dispatched Stainless interface trait object.
    Interface {
        /// Fully qualified generated Rust trait path.
        rust_path: String,
        /// Concrete or generic Rust type arguments.
        arguments: Vec<Type>,
    },
    /// A Rust generic type parameter.
    Parameter(String),
    /// A borrowed value.
    Reference {
        /// Whether the borrow permits mutation.
        mutable: bool,
        /// Borrowed type.
        target: Box<Type>,
    },
}

/// A braced sequence of statements.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Block {
    /// Statements in source order.
    pub statements: Vec<Statement>,
}

/// A resolved statement.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Statement {
    /// A nested lexical block.
    Block(Block),
    /// An initialized local binding.
    Let {
        /// Rust binding identifier.
        name: String,
        /// Resolved binding type.
        ty: Type,
        /// Whether the binding can be assigned or mutably borrowed.
        mutable: bool,
        /// Explicit or compiler-supplied initializer.
        initializer: Expression,
    },
    /// A return statement.
    Return(Option<Expression>),
    /// Create or rethrow a checked exception toward the active boundary.
    Throw {
        /// New value or existing erased catch allocation.
        value: ExceptionValue,
        /// Enclosing function or generated try boundary.
        target: ExceptionTarget,
    },
    /// Protected execution followed by ordered typed handlers.
    Try {
        /// Unique Rust label for propagation from the protected body.
        label: String,
        /// Hidden erased error binding.
        error_name: String,
        /// Protected body.
        body: Block,
        /// Whether normal control can reach the end of the protected body.
        body_falls_through: bool,
        /// Handlers in source order.
        catches: Vec<Catch>,
        /// Whether every successful and handled path exits the enclosing flow.
        diverges: bool,
        /// Destination for an unmatched exception.
        unmatched_target: ExceptionTarget,
    },
    /// Conditional control flow.
    If {
        /// Boolean condition.
        condition: Expression,
        /// Selected block when true.
        then_branch: Block,
        /// Optional false branch.
        else_branch: Option<Box<Statement>>,
    },
    /// A condition-controlled loop.
    While {
        /// Generated Rust loop label used by `break`, `continue`, and nested try blocks.
        label: String,
        /// Boolean loop condition.
        condition: Expression,
        /// Repeated block.
        body: Block,
    },
    /// A three-clause loop.
    ClassicFor {
        /// Generated Rust loop label used by nested try blocks.
        label: String,
        /// Optional initializer.
        initializer: Option<Box<ForInitializer>>,
        /// Optional condition.
        condition: Option<Expression>,
        /// Optional end-of-iteration update.
        update: Option<Expression>,
        /// Loop body.
        body: Block,
    },
    /// A range loop over a native collection.
    RangeFor {
        /// Generated Rust loop label used by nested try blocks.
        label: String,
        /// One element binding or the key/value bindings of a map loop.
        bindings: Vec<RangeBinding>,
        /// Borrowing or consuming mode.
        mode: RangeMode,
        /// Collection expression.
        iterable: Expression,
        /// Loop body.
        body: Block,
    },
    /// Exit the source loop identified during lowering.
    Break(String),
    /// Continue the source loop identified during lowering.
    Continue(String),
    /// Evaluate an expression for side effects.
    Expression(Expression),
}

/// A newly allocated exception or a catch allocation being rethrown.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ExceptionValue {
    /// Box this concrete exception value.
    New(Expression),
    /// Move an existing erased exception box.
    Existing(String),
}

/// Where checked propagation exits.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ExceptionTarget {
    /// Return `Err` from the current throwing function.
    Function,
    /// Break from a generated labeled try boundary with `Err`.
    Try(String),
    /// Statically exhaustive handlers make this carrier state impossible.
    Unreachable,
}

/// Statically selected conversion from a native Rust error to a message.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RustErrorMessage {
    /// Call the proven Rust `Display`/`ToString` implementation.
    Display,
    /// Format through the proven Rust `Debug` implementation.
    Debug,
    /// Consume the error and use a fixed message.
    Fallback,
}

/// Compiler-native checked exception selected for a Rust error conversion.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeExceptionKind {
    /// Generic failure from a native Rust `Result`.
    RustError,
    /// Failure from Rust filesystem or stream I/O.
    IoError,
    /// Failure while appending formatted text.
    FormatError,
    /// Failure while reading, parsing, or serializing JSON.
    JsonError,
}

/// One checked-exception handler.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Catch {
    /// Caught exception type; absent for `catch (...)`.
    pub ty: Option<Type>,
    /// Rust catch binding; absent for catch-all.
    pub binding: Option<String>,
    /// Handler body.
    pub body: Block,
}

/// The initializer slot of a classic loop.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ForInitializer {
    /// An initialized local.
    Let {
        /// Rust binding identifier.
        name: String,
        /// Resolved type.
        ty: Type,
        /// Whether the binding is mutable.
        mutable: bool,
        /// Initial value.
        initializer: Expression,
    },
    /// An expression initializer.
    Expression(Expression),
}

/// How a C++-style range loop obtains each element.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RangeMode {
    /// `const auto&`, lowered through `iter`.
    Shared,
    /// `auto&`, lowered through `iter_mut`.
    Mutable,
    /// `auto`, lowered through `iter().copied()`.
    Copy,
    /// `auto` for a user struct, lowered through `iter().cloned()`.
    Clone,
    /// Copied map key/value bindings, cloned from each borrowed pair.
    MapClone,
    /// `auto` over `move(range)`, lowered through `into_iter`.
    Move,
}

/// One Rust binding introduced by a generated range loop.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RangeBinding {
    /// Generated Rust binding identifier.
    pub name: String,
    /// Whether a copied value binding is mutable.
    pub mutable: bool,
}

/// A typed expression ready for Rust generation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Expression {
    /// A heterogeneous Rust tuple expression.
    Tuple(Vec<Expression>),
    /// A fixed-size Rust array with an optional default-initialized tail.
    Array {
        /// Explicit leading elements.
        elements: Vec<Expression>,
        /// Expression evaluated once for every missing element.
        default: Option<Box<Expression>>,
    },
    /// Default construct a scalar value.
    DefaultValue(Type),
    /// A local or parameter binding.
    Name(String),
    /// A struct-associated compile-time constant.
    StaticConstant {
        /// Generated namespace modules containing the struct.
        modules: Vec<String>,
        /// Generated struct identifier.
        structure: String,
        /// Associated constant identifier.
        constant: String,
    },
    /// A scalar or owned string literal.
    Literal {
        /// Lexical category.
        kind: LiteralKind,
        /// Source spelling.
        text: String,
    },
    /// A value-selecting switch expression.
    Switch {
        /// Value selected by the patterns.
        scrutinee: Box<Expression>,
        /// Ordered, non-fallthrough arms.
        arms: Vec<SwitchArm>,
        /// Whether Rust lowering must borrow an owned `String` as `str`.
        string_scrutinee: bool,
    },
    /// JSON `null`.
    JsonNull,
    /// Construct a JSON array.
    JsonArray(Vec<Expression>),
    /// Construct a JSON object.
    JsonObject(Vec<(String, Expression)>),
    /// Convert a statically typed scalar, collection, or data struct into `var`.
    JsonFrom(Box<Expression>),
    /// Null-safe JSON object member access.
    JsonField {
        /// JSON receiver.
        receiver: Box<Expression>,
        /// Decoded member name.
        name: String,
    },
    /// Null-safe JSON array indexing.
    JsonIndex {
        /// JSON receiver.
        receiver: Box<Expression>,
        /// Array index.
        index: Box<Expression>,
    },
    /// Fixed-size array or vector indexing.
    SequenceIndex {
        /// Array or vector place or value.
        receiver: Box<Expression>,
        /// Checked `usize` index.
        index: Box<Expression>,
    },
    /// Checked JSON object member assignment.
    JsonSetField {
        /// JSON receiver.
        receiver: Box<Expression>,
        /// Decoded member name.
        name: String,
        /// New JSON value.
        value: Box<Expression>,
    },
    /// Checked JSON array element assignment.
    JsonSetIndex {
        /// JSON receiver.
        receiver: Box<Expression>,
        /// Array index.
        index: Box<Expression>,
        /// New JSON value.
        value: Box<Expression>,
    },
    /// JavaScript-compatible conversion from `var` to a scalar type.
    JsonCast {
        /// JSON source value.
        expression: Box<Expression>,
        /// Destination primitive or String type.
        target: Type,
    },
    /// A compiler-validated Rust formatting macro invocation.
    FormatMacro {
        /// Purpose-built macro operation.
        kind: FormatMacroKind,
        /// Mutable output destination for `write!` and `writeln!`.
        destination: Option<Box<Expression>>,
        /// Format string source spelling, absent for a blank output line.
        format: Option<String>,
        /// Formatting values after the format string.
        arguments: Vec<Expression>,
    },
    /// Explicit parentheses.
    Parenthesized(Box<Expression>),
    /// Borrow a value.
    Borrow {
        /// Whether the borrow is mutable.
        mutable: bool,
        /// Borrowed expression.
        expression: Box<Expression>,
    },
    /// Read or update through a reference binding.
    Dereference(Box<Expression>),
    /// Explicitly consume a binding, even when the surrounding context borrows
    /// the resulting temporary.
    Move(Box<Expression>),
    /// Allocate a value into a non-null unique or shared owner.
    MakeOwner {
        /// Allocation representation.
        kind: PointerKind,
        /// Constructed pointee.
        value: Box<Expression>,
    },
    /// Default-construct a nullable owner, weak observer, or nullable atomic slot.
    PointerDefault(PointerKind),
    /// Convert between compatible pointer representations.
    PointerConversion {
        /// Source representation.
        from: PointerKind,
        /// Destination representation.
        to: PointerKind,
        /// Adapted source value.
        value: Box<Expression>,
    },
    /// Erase a concrete class owner to an implemented interface trait object.
    InterfaceOwnerCoercion {
        /// Unique/shared and nullable/non-null owner representation.
        kind: PointerKind,
        /// Dynamic interface pointee type.
        target: Type,
        /// Concrete class owner being erased.
        value: Box<Expression>,
    },
    /// Project a shared derived owner to one independently retained class base.
    ClassSharedOwnerCoercion {
        /// Derived-to-base representation fields in traversal order.
        projection: Vec<String>,
        /// Whether both source and target owners are nullable.
        nullable: bool,
        /// Concrete derived owner expression.
        value: Box<Expression>,
    },
    /// Wrap a constructed class value as an embedded base subobject.
    ClassBaseNew(Box<Expression>),
    /// Demote an `Arc<T>` to `Weak<T>`.
    DowngradeShared(Box<Expression>),
    /// Promote `Weak<T>` to `Option<Arc<T>>`.
    LockWeak(Box<Expression>),
    /// Test a nullable owner or weak observer for a live pointee.
    PointerHasValue {
        /// Pointer representation being tested.
        kind: PointerKind,
        /// Tested handle.
        value: Box<Expression>,
    },
    /// Project a statically proven non-null nullable owner to its pointee.
    PointerPointee {
        /// Nullable owner representation.
        kind: PointerKind,
        /// Whether exclusive pointee access is required.
        mutable: bool,
        /// Owner expression.
        owner: Box<Expression>,
    },
    /// Clone a snapshot from an atomic pointer slot.
    AtomicLoad {
        /// Whether the stored handle is nullable.
        nullable: bool,
        /// Slot expression.
        slot: Box<Expression>,
    },
    /// Replace an atomic pointer slot without returning its previous value.
    AtomicStore {
        /// Slot expression.
        slot: Box<Expression>,
        /// New stored handle.
        value: Box<Expression>,
    },
    /// Replace an atomic pointer slot and return its previous handle.
    AtomicSwap {
        /// Slot expression.
        slot: Box<Expression>,
        /// New stored handle.
        value: Box<Expression>,
    },
    /// Construct a mutex around an initialized value.
    MutexNew(Box<Expression>),
    /// Construct a condition signal.
    ConditionNew,
    /// Acquire a mutex, recovering its value if another thread panicked.
    MutexLock(Box<Expression>),
    /// Construct a reader/writer lock around an initialized value.
    RwLockNew(Box<Expression>),
    /// Acquire a shared reader guard, recovering after a panic.
    RwLockRead(Box<Expression>),
    /// Acquire an exclusive writer guard, recovering after a panic.
    RwLockWrite(Box<Expression>),
    /// Release a guard while waiting and transparently reacquire it.
    ConditionWait {
        /// Condition variable expression.
        condition: Box<Expression>,
        /// Mutable named guard that is consumed and rebound.
        guard: Box<Expression>,
    },
    /// Notify one or all condition waiters.
    ConditionNotify {
        /// Condition variable expression.
        condition: Box<Expression>,
        /// Whether every waiter is notified.
        all: bool,
    },
    /// Spawn one owned callback on an operating-system thread.
    ThreadSpawn(Box<Expression>),
    /// Consume and join a thread, returning a checked-exception-shaped Result.
    ThreadJoin(Box<Expression>),
    /// Execute a Rust lexical thread scope with checked panic conversion.
    ThreadScope(Box<Expression>),
    /// Spawn one lifetime-confined callback through a scope.
    ScopedThreadSpawn {
        /// Borrowed scope receiver.
        scope: Box<Expression>,
        /// Callback that may borrow from the scope environment.
        callback: Box<Expression>,
    },
    /// Join a scoped worker, forwarding its panic to the outer scope converter.
    ScopedThreadJoin(Box<Expression>),
    /// Consume a native Rust Result, converting `Err` to checked `RustError`.
    UnwrapRustResult {
        /// Native `Result<T, E>` expression.
        expression: Box<Expression>,
        /// Concrete compiler-native exception created for `Err`.
        exception: NativeExceptionKind,
        /// Statically selected error-message conversion.
        error_message: RustErrorMessage,
        /// Active checked boundary.
        target: ExceptionTarget,
    },
    /// Convert a normal return value into `Ok`.
    Success(Option<Box<Expression>>),
    /// Extract a successful checked call or propagate its erased error.
    Propagate {
        /// Result-producing expression.
        expression: Box<Expression>,
        /// Active checked boundary.
        target: ExceptionTarget,
    },
    /// A prefix operation that maps directly to Rust.
    Prefix {
        /// Source operator.
        operator: PrefixOperator,
        /// Operand.
        operand: Box<Expression>,
    },
    /// A prefix/postfix increment or decrement.
    Increment {
        /// Updated place.
        place: Box<Expression>,
        /// Whether this is increment rather than decrement.
        increment: bool,
        /// Whether the updated value rather than prior value is produced.
        prefix: bool,
    },
    /// A binary operation.
    Binary {
        /// Left operand.
        left: Box<Expression>,
        /// Source operator.
        operator: BinaryOperator,
        /// Right operand.
        right: Box<Expression>,
    },
    /// Access one direct or inherited representation field.
    Field {
        /// Base expression.
        receiver: Box<Expression>,
        /// Rust fields traversed in order.
        access_path: Vec<String>,
    },
    /// Construct a user-defined aggregate.
    Aggregate {
        /// Constructed type.
        ty: Type,
        /// Rust field names paired with their values.
        fields: Vec<(String, Expression)>,
    },
    /// A contextually typed explicit-capture Rust closure.
    Lambda {
        /// Capture bindings materialized in source order.
        captures: Vec<LambdaCapture>,
        /// Whether this closure returns a Rust future.
        is_async: bool,
        /// Whether an async closure may be invoked more than once.
        repeatable: bool,
        /// Explicitly typed closure parameters.
        parameters: Vec<Parameter>,
        /// Closure body.
        body: Block,
    },
    /// Await the enclosed Rust future.
    Await(Box<Expression>),
    /// A resolved Stainless function item used as a callback.
    FunctionItem {
        /// Namespace modules containing the target.
        modules: Vec<String>,
        /// Deterministically mangled target.
        function: String,
    },
    /// Allocate a lambda or function item into a stored callable trait object.
    StoreFunction {
        /// Shared or unique storage representation.
        kind: StoredFunctionKind,
        /// Complete target trait-object type used for coercion.
        ty: Type,
        /// Concrete closure or function item.
        callable: Box<Expression>,
    },
    /// Invoke a stored callable value.
    CallableCall {
        /// Callable expression.
        callable: Box<Expression>,
        /// Lowered arguments.
        arguments: Vec<Expression>,
    },
    /// A resolved Stainless free-function call.
    FunctionCall {
        /// Namespace modules containing the target.
        modules: Vec<String>,
        /// Deterministically mangled target.
        function: String,
        /// Lowered arguments.
        arguments: Vec<Expression>,
    },
    /// Dynamically dispatch a Stainless interface method.
    InterfaceCall {
        /// Interface receiver expression.
        receiver: Box<Expression>,
        /// Deterministically mangled Rust trait method name.
        method: String,
        /// Explicit call arguments.
        arguments: Vec<Expression>,
    },
    /// A native associated function or constructor.
    AssociatedCall {
        /// Fully qualified Rust callable path.
        rust_path: String,
        /// Lowered arguments.
        arguments: Vec<Expression>,
    },
    /// A call through a generated external Rust wrapper.
    WrapperCall {
        /// Deterministic wrapper function name.
        rust_name: String,
        /// Receiver followed by ordinary arguments.
        arguments: Vec<Expression>,
    },
    /// A native Rust method.
    MethodCall {
        /// Method receiver.
        receiver: Box<Expression>,
        /// Rust method name.
        rust_name: String,
        /// Ownership behavior retained for inspection.
        receiver_mode: Receiver,
        /// Lowered arguments.
        arguments: Vec<Expression>,
    },
    /// Explicit cloning used by a Stainless constructor.
    Clone {
        /// Borrowed clone source.
        expression: Box<Expression>,
    },
    /// A primitive numeric conversion.
    Cast {
        /// Converted expression.
        expression: Box<Expression>,
        /// Destination type.
        target: Type,
    },
}

/// One lowered switch pattern.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SwitchPattern {
    Literals(Vec<SwitchLiteral>),
    Fallback,
}

/// One literal within a lowered switch pattern.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SwitchLiteral {
    /// Broad literal category.
    pub kind: LiteralKind,
    /// Exact source spelling.
    pub text: String,
}

/// One lowered switch arm.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SwitchArm {
    pub pattern: SwitchPattern,
    pub value: Expression,
}

/// Supported Rust formatting macro with statically defined Stainless syntax.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FormatMacroKind {
    /// Write a line to standard output.
    Println,
    /// Write a line to standard error.
    Eprintln,
    /// Produce a new Rust `String`.
    Format,
    /// Append formatted text to a mutable `String`.
    Write,
    /// Append formatted text and a newline to a mutable `String`.
    Writeln,
}
