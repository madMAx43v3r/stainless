//! Compiler-owned abstract syntax for the currently parsed Stainless subset.

use stainless_syntax::TextRange;

/// A half-open UTF-8 byte range in a source file.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Span {
    /// First byte included in the range.
    pub start: u32,
    /// First byte after the range.
    pub end: u32,
}

impl Span {
    /// Converts a Rowan text range into a compiler span.
    #[must_use]
    pub fn from_text_range(range: TextRange) -> Self {
        Self {
            start: range.start().into(),
            end: range.end().into(),
        }
    }
}

/// One lowered source file.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceFile {
    /// Top-level declarations in source order.
    pub items: Vec<Item>,
    /// Range occupied by the complete file.
    pub span: Span,
}

/// A top-level or namespace declaration.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Item {
    /// A namespace definition.
    Namespace(Namespace),
    /// An import declaration.
    Use(UseDeclaration),
    /// A data-only struct definition.
    Struct(Struct),
    /// A qualified constructor definition.
    Constructor(Constructor),
    /// A free or qualified function.
    Function(Function),
}

/// Source-level role of a user-defined type declaration.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UserTypeKind {
    /// Copy-oriented data type with optional single data inheritance.
    Struct,
    /// Move-only identity type with optional single class inheritance.
    Class,
    /// Behavior-only contract lowered to a Rust trait.
    Interface,
}

impl Item {
    /// Returns the declaration's source span.
    #[must_use]
    pub const fn span(&self) -> Span {
        match self {
            Self::Namespace(item) => item.span,
            Self::Use(item) => item.span,
            Self::Struct(item) => item.span,
            Self::Constructor(item) => item.span,
            Self::Function(item) => item.span,
        }
    }
}

/// A `namespace name { ... }` definition.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Namespace {
    /// Namespace identifier.
    pub name: String,
    /// Nested declarations.
    pub items: Vec<Item>,
    /// Complete declaration range.
    pub span: Span,
}

/// A `use` declaration.
///
/// The import grammar is intentionally still permissive, so the body is kept
/// as normalized source text until import-path lowering is implemented.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UseDeclaration {
    /// Text between `use` and the terminating semicolon.
    pub path: String,
    /// Complete declaration range.
    pub span: Span,
}

/// A user-defined struct, class, or interface.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Struct {
    /// Struct, class, or interface semantics.
    pub kind: UserTypeKind,
    /// Unqualified source name.
    pub name: String,
    /// Invariant generic type parameters in declaration order.
    pub type_parameters: Vec<String>,
    /// Parameters from `usize N` generic slots, as a subset of `type_parameters`.
    pub const_parameters: Vec<String>,
    /// Direct data/class-base or interface declarations in source order.
    pub bases: Vec<Type>,
    /// Whether inheritance/implementation is restricted to this module.
    pub is_sealed: bool,
    /// Whether an explicit access label appeared in the declaration.
    pub has_access_specifier: bool,
    /// Direct data fields in declaration order.
    pub fields: Vec<Field>,
    /// Compile-time associated integer constants with no instance storage.
    pub static_constants: Vec<StaticConstant>,
    /// Member function declarations written inside the body.
    pub functions: Vec<Function>,
    /// Constructor declarations written inside the body.
    pub constructors: Vec<Constructor>,
    /// Complete definition range.
    pub span: Span,
}

/// A struct constructor declaration, definition, or deletion.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Constructor {
    /// Whether this constructor is publicly callable.
    pub is_public: bool,
    /// Unqualified declaration name or qualified definition path.
    pub name: Path,
    /// Type arguments repeated on a qualified out-of-body owner.
    pub owner_arguments: Vec<Type>,
    /// Parameters in declaration order.
    pub parameters: Vec<Parameter>,
    /// Checked exception types.
    pub throws: Vec<Type>,
    /// C++-style base and field initializer list.
    pub initializers: Vec<ConstructorInitializer>,
    /// Body, absent for an in-struct declaration.
    pub body: Option<Block>,
    /// Whether this declaration uses `= delete`.
    pub is_deleted: bool,
    /// Complete declaration or definition range.
    pub span: Span,
}

/// One `target(arguments...)` constructor initializer.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConstructorInitializer {
    /// Direct field or data-base name.
    pub target: Path,
    /// Construction arguments.
    pub arguments: Vec<Expression>,
    /// Complete initializer range.
    pub span: Span,
}

/// One direct struct data field.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Field {
    /// Whether this field is publicly accessible.
    pub is_public: bool,
    /// Declared field type.
    pub ty: Type,
    /// Source field name.
    pub name: String,
    /// Complete declaration range.
    pub span: Span,
}

/// One compile-time constant associated with a struct.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StaticConstant {
    /// Whether this constant is publicly accessible.
    pub is_public: bool,
    /// Declared integer type.
    pub ty: Type,
    /// Source member name.
    pub name: String,
    /// Required compile-time initializer.
    pub initializer: Expression,
    /// Complete declaration range.
    pub span: Span,
}

/// A function definition or declaration.
#[allow(clippy::struct_excessive_bools)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Function {
    /// Whether this member is publicly callable.
    pub is_public: bool,
    /// Whether this is an associated function without an instance receiver.
    pub is_static: bool,
    /// Whether calling this function produces an awaitable Rust future.
    pub is_async: bool,
    /// Possibly qualified source name.
    pub name: Path,
    /// Type arguments repeated on a qualified out-of-body owner.
    pub owner_arguments: Vec<Type>,
    /// Declared return type.
    pub return_type: Type,
    /// Parameters in declaration order.
    pub parameters: Vec<Parameter>,
    /// Whether the member function has a trailing `const`.
    pub is_const: bool,
    /// Checked exception types declared after `throws`.
    pub throws: Vec<Type>,
    /// Function body; absent for a declaration ending in `;`.
    pub body: Option<Block>,
    /// Complete declaration or definition range.
    pub span: Span,
}

/// One function parameter.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Parameter {
    /// Declared type.
    pub ty: Type,
    /// Binding name.
    pub name: String,
    /// Complete parameter range.
    pub span: Span,
}

/// A type written in source.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Type {
    /// `const` applies to a reference target and distinguishes shared from
    /// mutable references.
    pub is_const: bool,
    /// Whether the outer type is a reference.
    pub is_reference: bool,
    /// Inferred, named, or malformed type body.
    pub kind: TypeKind,
    /// Complete type range.
    pub span: Span,
}

impl Type {
    /// Returns whether this is the built-in `void` type.
    #[must_use]
    pub fn is_void(&self) -> bool {
        matches!(
            &self.kind,
            TypeKind::Named(named)
                if named.path.segments.len() == 1
                    && named.path.segments[0] == "void"
                    && named.arguments.is_empty()
        )
    }

    /// Returns whether this type uses `auto`.
    #[must_use]
    pub const fn is_inferred(&self) -> bool {
        matches!(self.kind, TypeKind::Inferred)
    }
}

/// The main body of a source type.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TypeKind {
    /// Source `auto`.
    Inferred,
    /// A named type with explicit generic arguments.
    Named(NamedType),
    /// A non-null stored callable with an exact signature.
    Function {
        /// Whether invocation may mutate captured state.
        mutable: bool,
        /// Exact parameter types.
        parameters: Vec<Type>,
        /// Exact value-semantic return type.
        return_type: Box<Type>,
    },
    /// A non-negative integer used only in a compile-time generic value slot.
    ConstUsize(String),
    /// A missing or malformed type retained after parser recovery.
    Error,
}

/// A path plus generic type arguments.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NamedType {
    /// Qualified type path.
    pub path: Path,
    /// Explicit generic arguments.
    pub arguments: Vec<Type>,
}

/// A `::`-separated name represented without punctuation.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
pub struct Path {
    /// Name segments in source order.
    pub segments: Vec<String>,
}

impl Path {
    /// Joins the path using its Stainless spelling.
    #[must_use]
    pub fn display(&self) -> String {
        self.segments.join("::")
    }
}

/// A braced statement list.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Block {
    /// Statements in source order.
    pub statements: Vec<Statement>,
    /// Complete block range.
    pub span: Span,
}

/// One statement with a stable source range.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Statement {
    /// Statement-specific data.
    pub kind: StatementKind,
    /// Complete statement range.
    pub span: Span,
}

/// Statement forms accepted by the initial parser.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StatementKind {
    /// A nested block.
    Block(Block),
    /// A local declaration.
    Local(LocalDeclaration),
    /// A return with an optional value.
    Return(Option<Expression>),
    /// A checked throw or bare rethrow.
    Throw(Option<Expression>),
    /// A checked `try` statement and ordered handlers.
    Try(TryStatement),
    /// Conditional control flow.
    If(IfStatement),
    /// Condition-controlled iteration.
    While(WhileStatement),
    /// Classic or range iteration.
    For(ForStatement),
    /// `break;`.
    Break,
    /// `continue;`.
    Continue,
    /// An expression followed by `;`.
    Expression(Expression),
    /// A lone semicolon.
    Empty,
    /// Malformed syntax retained after recovery.
    Error,
}

/// A checked `try` body followed by one or more catches.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TryStatement {
    /// Protected body.
    pub body: Block,
    /// Handlers in source order.
    pub catches: Vec<CatchClause>,
}

/// One typed or catch-all exception handler.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CatchClause {
    /// Typed binder, absent for `catch (...)`.
    pub binding: Option<CatchBinding>,
    /// Handler body.
    pub body: Block,
    /// Complete handler range.
    pub span: Span,
}

/// A compiler-managed `const Exception&` catch binding.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CatchBinding {
    /// Declared exception-reference type.
    pub ty: Type,
    /// Source binding name.
    pub name: String,
    /// Binding range.
    pub span: Span,
}

/// A local variable declaration.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LocalDeclaration {
    /// Declared or inferred type.
    pub ty: Type,
    /// Binding name.
    pub name: String,
    /// Optional explicit initializer.
    pub initializer: Option<Expression>,
    /// Complete declaration range.
    pub span: Span,
}

/// An `if` statement and optional `else` branch.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IfStatement {
    /// Condition expression.
    pub condition: Expression,
    /// Statement selected when the condition is true.
    pub then_branch: Box<Statement>,
    /// Statement selected when the condition is false.
    pub else_branch: Option<Box<Statement>>,
}

/// A `while` statement.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WhileStatement {
    /// Condition checked before every iteration.
    pub condition: Expression,
    /// Repeated statement.
    pub body: Box<Statement>,
}

/// A `for` statement.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ForStatement {
    /// Range or classic loop header.
    pub clause: ForClause,
    /// Repeated statement.
    pub body: Box<Statement>,
}

/// A range or classic `for` header.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ForClause {
    /// C++-style range iteration.
    Range(RangeForClause),
    /// Three-slot C++-style loop.
    Classic(ClassicForClause),
    /// A malformed loop header retained after recovery.
    Error,
}

/// A `type binding : iterable` range-loop header.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RangeForClause {
    /// Binding type, including supported `auto` forms.
    pub ty: Type,
    /// One ordinary binding or the key/value bindings of a map iteration.
    pub bindings: Vec<RangeBinding>,
    /// Value being iterated.
    pub iterable: Expression,
}

/// One source binding introduced by a range loop.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RangeBinding {
    /// Per-iteration binding name.
    pub name: String,
    /// Source range of the binding identifier.
    pub span: Span,
}

/// A classic `initializer; condition; update` loop header.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClassicForClause {
    /// Optional local declaration or expression.
    pub initializer: Option<ForInitializer>,
    /// Optional loop condition.
    pub condition: Option<Expression>,
    /// Optional end-of-iteration expression.
    pub update: Option<Expression>,
}

/// A classic `for` initializer.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ForInitializer {
    /// A declaration including its initializer.
    Local(LocalDeclaration),
    /// An expression.
    Expression(Expression),
}

/// One expression with its source range.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Expression {
    /// Expression-specific data.
    pub kind: ExpressionKind,
    /// Complete expression range.
    pub span: Span,
}

/// One non-binding pattern accepted by a switch expression.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SwitchPattern {
    /// A scalar or string literal.
    Literal(Literal),
    /// The exhaustive `else` fallback.
    Fallback,
}

/// One pattern and value branch of a switch expression.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SwitchArm {
    pub pattern: SwitchPattern,
    pub value: Expression,
    pub span: Span,
}

/// One explicit lambda capture.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LambdaCapture {
    /// Captured binding name and the lambda-local binding name.
    pub name: String,
    /// Copy, borrow, or explicit initializer capture form.
    pub kind: LambdaCaptureKind,
    /// Complete capture range.
    pub span: Span,
}

/// Ownership syntax used by one lambda capture.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LambdaCaptureKind {
    /// `[value]`.
    Copy,
    /// `[&value]`.
    Borrow,
    /// `[value = expression]` with an inferred owned capture type.
    Initialize(Box<Expression>),
}

/// Expression forms accepted by the initial parser.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ExpressionKind {
    /// A qualified value or callable name.
    Name(Path),
    /// A call target with explicit generic type arguments.
    GenericName {
        /// Qualified target name.
        path: Path,
        /// Source-provided type arguments.
        arguments: Vec<Type>,
    },
    /// A scalar or string literal.
    Literal(Literal),
    /// A parenthesized expression.
    Parenthesized(Box<Expression>),
    /// A prefix operation.
    Prefix {
        /// Operator.
        operator: PrefixOperator,
        /// Operand.
        operand: Box<Expression>,
    },
    /// A postfix operation.
    Postfix {
        /// Operator.
        operator: PostfixOperator,
        /// Operand.
        operand: Box<Expression>,
    },
    /// A binary operation.
    Binary {
        /// Left operand.
        left: Box<Expression>,
        /// Operator.
        operator: BinaryOperator,
        /// Right operand.
        right: Box<Expression>,
    },
    /// Select one value using literal patterns and an exhaustive `_` arm.
    Switch {
        /// Value compared against each literal pattern.
        scrutinee: Box<Expression>,
        /// Arms in source order.
        arms: Vec<SwitchArm>,
    },
    /// A function or constructor call.
    Call {
        /// Called expression.
        callee: Box<Expression>,
        /// Arguments in source order.
        arguments: Vec<Expression>,
    },
    /// A compiler-supported Rust macro invocation retaining its `!` spelling.
    MacroCall {
        /// Qualified or imported macro name.
        callee: Path,
        /// Stainless expressions in the macro argument list.
        arguments: Vec<Expression>,
    },
    /// C++-style brace construction, such as `Point{1, 2}` or
    /// `make_shared<Point>{1, 2}`.
    Aggregate {
        /// Constructed type or generic allocation target.
        ty: Box<Type>,
        /// Initializers in direct layout order.
        initializers: Vec<Expression>,
    },
    /// A dynamically typed JSON array literal.
    JsonArray {
        /// Element expressions in source order.
        elements: Vec<Expression>,
    },
    /// A dynamically typed JSON object literal.
    JsonObject {
        /// Decoded member keys and their value expressions.
        members: Vec<(String, Expression)>,
    },
    /// A dot member access.
    Field {
        /// Receiver.
        receiver: Box<Expression>,
        /// Selected member, optionally qualified by a data base.
        name: Path,
    },
    /// An indexing operation.
    Index {
        /// Indexed value.
        receiver: Box<Expression>,
        /// Index expression.
        index: Box<Expression>,
    },
    /// A C++-style explicit-capture lambda.
    Lambda {
        /// Explicit captures in source order.
        captures: Vec<LambdaCapture>,
        /// Typed lambda parameters.
        parameters: Vec<Parameter>,
        /// Whether by-value captures may be mutated by the body.
        is_mutable: bool,
        /// Whether the lambda produces a Rust future when invoked.
        is_async: bool,
        /// Lambda body.
        body: Block,
    },
    /// Await an async Stainless or compiler-described Rust call.
    Await(Box<Expression>),
    /// Malformed syntax retained after recovery.
    Error,
}

/// A literal spelling and its broad lexical category.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Literal {
    /// Literal category.
    pub kind: LiteralKind,
    /// Exact source spelling.
    pub text: String,
}

/// Lexical literal categories.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LiteralKind {
    Integer,
    Float,
    String,
    Character,
    Boolean,
    /// JSON `null` or pointer `nullptr`; contextual typing distinguishes them.
    Null,
}

/// Prefix operators.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PrefixOperator {
    Plus,
    Negate,
    Not,
    BitwiseNot,
    Increment,
    Decrement,
}

/// Postfix operators.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PostfixOperator {
    Increment,
    Decrement,
}

/// Binary operators in the initial expression grammar.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BinaryOperator {
    Assign,
    AddAssign,
    SubtractAssign,
    MultiplyAssign,
    DivideAssign,
    RemainderAssign,
    LogicalOr,
    LogicalAnd,
    BitwiseOr,
    BitwiseXor,
    BitwiseAnd,
    Equal,
    NotEqual,
    Less,
    LessEqual,
    Greater,
    GreaterEqual,
    Add,
    Subtract,
    Multiply,
    Divide,
    Remainder,
}
