//! Typed, Rust-shaped intermediate representation used by the backend.

use crate::ast::{BinaryOperator, LiteralKind, PrefixOperator, Span};
use crate::interop::Receiver;

/// A source file after successful semantic resolution and backend lowering.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Program {
    /// Functions declared directly at crate scope.
    pub functions: Vec<Function>,
    /// Nested Stainless namespaces, emitted as Rust modules.
    pub modules: Vec<Module>,
}

/// A Stainless namespace represented as a Rust module.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Module {
    /// Source namespace name.
    pub source_name: String,
    /// Collision-resistant Rust identifier.
    pub rust_name: String,
    /// Functions declared directly in this namespace.
    pub functions: Vec<Function>,
    /// Nested namespaces.
    pub modules: Vec<Module>,
}

/// One resolved function definition.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Function {
    /// Fully qualified Stainless name.
    pub source_path: Vec<String>,
    /// Deterministically mangled Rust function name.
    pub rust_name: String,
    /// Function parameters.
    pub parameters: Vec<Parameter>,
    /// Resolved return type.
    pub return_type: Type,
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
    /// A native Rust type with concrete type arguments.
    Native {
        /// Fully qualified Rust path.
        rust_path: &'static str,
        /// Concrete type arguments.
        arguments: Vec<Type>,
    },
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
    /// Conditional control flow.
    If {
        /// Boolean condition.
        condition: Expression,
        /// Selected block when true.
        then_branch: Block,
        /// Optional false branch.
        else_branch: Option<Box<Statement>>,
    },
    /// A three-clause loop.
    ClassicFor {
        /// Optional initializer.
        initializer: Option<ForInitializer>,
        /// Optional condition.
        condition: Option<Expression>,
        /// Optional end-of-iteration update.
        update: Option<Expression>,
        /// Loop body.
        body: Block,
    },
    /// A range loop over a native collection.
    RangeFor {
        /// Rust binding identifier.
        name: String,
        /// Whether a copied value binding is mutable.
        mutable: bool,
        /// Borrowing or consuming mode.
        mode: RangeMode,
        /// Collection expression.
        iterable: Expression,
        /// Loop body.
        body: Block,
    },
    /// Exit the nearest loop.
    Break,
    /// Continue the nearest loop.
    Continue,
    /// Evaluate an expression for side effects.
    Expression(Expression),
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
    /// `auto` over `move(range)`, lowered through `into_iter`.
    Move,
}

/// A typed expression ready for Rust generation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Expression {
    /// A local or parameter binding.
    Name(String),
    /// A scalar or owned string literal.
    Literal {
        /// Lexical category.
        kind: LiteralKind,
        /// Source spelling.
        text: String,
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
    /// A resolved Stainless free-function call.
    FunctionCall {
        /// Namespace modules containing the target.
        modules: Vec<String>,
        /// Deterministically mangled target.
        function: String,
        /// Lowered arguments.
        arguments: Vec<Expression>,
    },
    /// A native associated function or constructor.
    AssociatedCall {
        /// Fully qualified Rust callable path.
        rust_path: &'static str,
        /// Lowered arguments.
        arguments: Vec<Expression>,
    },
    /// A native Rust method.
    MethodCall {
        /// Method receiver.
        receiver: Box<Expression>,
        /// Rust method name.
        rust_name: &'static str,
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
