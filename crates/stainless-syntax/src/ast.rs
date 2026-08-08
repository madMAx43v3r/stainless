//! Typed, zero-copy views over the lossless Stainless concrete syntax tree.

use std::marker::PhantomData;

use rowan::SyntaxNodeChildren;

use crate::{StainlessLanguage, SyntaxKind, SyntaxNode, SyntaxToken};

/// A typed wrapper around one concrete-syntax node.
pub trait AstNode: Clone {
    /// Returns whether this wrapper accepts `kind`.
    fn can_cast(kind: SyntaxKind) -> bool;

    /// Wraps `syntax` when it has the expected kind.
    fn cast(syntax: SyntaxNode) -> Option<Self>;

    /// Returns the underlying lossless syntax node.
    fn syntax(&self) -> &SyntaxNode;
}

/// Direct typed children of a concrete-syntax node.
pub struct AstChildren<N> {
    inner: SyntaxNodeChildren<StainlessLanguage>,
    node: PhantomData<N>,
}

impl<N: AstNode> Iterator for AstChildren<N> {
    type Item = N;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.find_map(N::cast)
    }
}

macro_rules! ast_node {
    ($name:ident, $kind:ident) => {
        #[doc = concat!("Typed wrapper for `", stringify!($kind), "`.")]
        #[derive(Clone, Debug, Eq, Hash, PartialEq)]
        pub struct $name {
            syntax: SyntaxNode,
        }

        impl AstNode for $name {
            fn can_cast(kind: SyntaxKind) -> bool {
                kind == SyntaxKind::$kind
            }

            fn cast(syntax: SyntaxNode) -> Option<Self> {
                Self::can_cast(syntax.kind()).then_some(Self { syntax })
            }

            fn syntax(&self) -> &SyntaxNode {
                &self.syntax
            }
        }
    };
}

ast_node!(SourceFile, SourceFile);
ast_node!(NamespaceDefinition, NamespaceDefinition);
ast_node!(UseDeclaration, UseDeclaration);
ast_node!(StructDefinition, StructDefinition);
ast_node!(ClassDefinition, ClassDefinition);
ast_node!(InterfaceDefinition, InterfaceDefinition);
ast_node!(AccessSpecifier, AccessSpecifier);
ast_node!(FieldDeclaration, FieldDeclaration);
ast_node!(ConstructorDefinition, ConstructorDefinition);
ast_node!(ConstructorDeclaration, ConstructorDeclaration);
ast_node!(ConstructorInitializerList, ConstructorInitializerList);
ast_node!(ConstructorInitializer, ConstructorInitializer);
ast_node!(FunctionDefinition, FunctionDefinition);
ast_node!(FunctionDeclaration, FunctionDeclaration);
ast_node!(ParameterList, ParameterList);
ast_node!(Parameter, Parameter);
ast_node!(TypeReference, TypeReference);
ast_node!(GenericParameterList, GenericParameterList);
ast_node!(GenericParameter, GenericParameter);
ast_node!(GenericArgumentList, GenericArgumentList);
ast_node!(FunctionTypeSignature, FunctionTypeSignature);
ast_node!(ThrowsClause, ThrowsClause);
ast_node!(Block, Block);
ast_node!(LocalDeclaration, LocalDeclaration);
ast_node!(ReturnStatement, ReturnStatement);
ast_node!(ThrowStatement, ThrowStatement);
ast_node!(TryStatement, TryStatement);
ast_node!(CatchClause, CatchClause);
ast_node!(IfStatement, IfStatement);
ast_node!(ElseClause, ElseClause);
ast_node!(WhileStatement, WhileStatement);
ast_node!(ForStatement, ForStatement);
ast_node!(RangeForClause, RangeForClause);
ast_node!(ClassicForClause, ClassicForClause);
ast_node!(BreakStatement, BreakStatement);
ast_node!(ContinueStatement, ContinueStatement);
ast_node!(ExpressionStatement, ExpressionStatement);
ast_node!(EmptyStatement, EmptyStatement);
ast_node!(NameExpression, NameExpression);
ast_node!(LiteralExpression, LiteralExpression);
ast_node!(ParenthesizedExpression, ParenthesizedExpression);
ast_node!(PrefixExpression, PrefixExpression);
ast_node!(PostfixExpression, PostfixExpression);
ast_node!(BinaryExpression, BinaryExpression);
ast_node!(SwitchExpression, SwitchExpression);
ast_node!(SwitchArm, SwitchArm);
ast_node!(SwitchPattern, SwitchPattern);
ast_node!(CallExpression, CallExpression);
ast_node!(MacroCallExpression, MacroCallExpression);
ast_node!(ArgumentList, ArgumentList);
ast_node!(AggregateExpression, AggregateExpression);
ast_node!(InitializerList, InitializerList);
ast_node!(JsonArrayExpression, JsonArrayExpression);
ast_node!(JsonObjectExpression, JsonObjectExpression);
ast_node!(JsonMember, JsonMember);
ast_node!(FieldExpression, FieldExpression);
ast_node!(IndexExpression, IndexExpression);
ast_node!(LambdaExpression, LambdaExpression);
ast_node!(AwaitExpression, AwaitExpression);
ast_node!(CaptureList, CaptureList);
ast_node!(LambdaCapture, LambdaCapture);
ast_node!(ErrorNode, Error);

/// A source-file or namespace item.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum Item {
    /// `namespace name { ... }`.
    Namespace(NamespaceDefinition),
    /// `use path;`.
    Use(UseDeclaration),
    /// A data-only `struct`.
    Struct(StructDefinition),
    /// An identity-oriented `class`.
    Class(ClassDefinition),
    /// A behavior-only `interface`.
    Interface(InterfaceDefinition),
    /// A constructor with a body.
    ConstructorDefinition(ConstructorDefinition),
    /// A constructor declaration or deletion.
    ConstructorDeclaration(ConstructorDeclaration),
    /// A function with a body.
    FunctionDefinition(FunctionDefinition),
    /// A function declaration ending in `;`.
    FunctionDeclaration(FunctionDeclaration),
}

impl AstNode for Item {
    fn can_cast(kind: SyntaxKind) -> bool {
        matches!(
            kind,
            SyntaxKind::NamespaceDefinition
                | SyntaxKind::UseDeclaration
                | SyntaxKind::StructDefinition
                | SyntaxKind::ClassDefinition
                | SyntaxKind::InterfaceDefinition
                | SyntaxKind::ConstructorDefinition
                | SyntaxKind::ConstructorDeclaration
                | SyntaxKind::FunctionDefinition
                | SyntaxKind::FunctionDeclaration
        )
    }

    fn cast(syntax: SyntaxNode) -> Option<Self> {
        match syntax.kind() {
            SyntaxKind::NamespaceDefinition => {
                NamespaceDefinition::cast(syntax).map(Self::Namespace)
            }
            SyntaxKind::UseDeclaration => UseDeclaration::cast(syntax).map(Self::Use),
            SyntaxKind::StructDefinition => StructDefinition::cast(syntax).map(Self::Struct),
            SyntaxKind::ClassDefinition => ClassDefinition::cast(syntax).map(Self::Class),
            SyntaxKind::InterfaceDefinition => {
                InterfaceDefinition::cast(syntax).map(Self::Interface)
            }
            SyntaxKind::ConstructorDefinition => {
                ConstructorDefinition::cast(syntax).map(Self::ConstructorDefinition)
            }
            SyntaxKind::ConstructorDeclaration => {
                ConstructorDeclaration::cast(syntax).map(Self::ConstructorDeclaration)
            }
            SyntaxKind::FunctionDefinition => {
                FunctionDefinition::cast(syntax).map(Self::FunctionDefinition)
            }
            SyntaxKind::FunctionDeclaration => {
                FunctionDeclaration::cast(syntax).map(Self::FunctionDeclaration)
            }
            _ => None,
        }
    }

    fn syntax(&self) -> &SyntaxNode {
        match self {
            Self::Namespace(node) => node.syntax(),
            Self::Use(node) => node.syntax(),
            Self::Struct(node) => node.syntax(),
            Self::Class(node) => node.syntax(),
            Self::Interface(node) => node.syntax(),
            Self::ConstructorDefinition(node) => node.syntax(),
            Self::ConstructorDeclaration(node) => node.syntax(),
            Self::FunctionDefinition(node) => node.syntax(),
            Self::FunctionDeclaration(node) => node.syntax(),
        }
    }
}

/// A function definition or declaration.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum Function {
    /// A function with a body.
    Definition(FunctionDefinition),
    /// A signature ending in `;`.
    Declaration(FunctionDeclaration),
}

impl AstNode for Function {
    fn can_cast(kind: SyntaxKind) -> bool {
        matches!(
            kind,
            SyntaxKind::FunctionDefinition | SyntaxKind::FunctionDeclaration
        )
    }

    fn cast(syntax: SyntaxNode) -> Option<Self> {
        match syntax.kind() {
            SyntaxKind::FunctionDefinition => {
                FunctionDefinition::cast(syntax).map(Self::Definition)
            }
            SyntaxKind::FunctionDeclaration => {
                FunctionDeclaration::cast(syntax).map(Self::Declaration)
            }
            _ => None,
        }
    }

    fn syntax(&self) -> &SyntaxNode {
        match self {
            Self::Definition(node) => node.syntax(),
            Self::Declaration(node) => node.syntax(),
        }
    }
}

/// A statement accepted by the initial parser.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum Statement {
    Block(Block),
    Local(LocalDeclaration),
    Return(ReturnStatement),
    Throw(ThrowStatement),
    Try(TryStatement),
    If(IfStatement),
    While(WhileStatement),
    For(ForStatement),
    Break(BreakStatement),
    Continue(ContinueStatement),
    Expression(ExpressionStatement),
    Empty(EmptyStatement),
    Error(ErrorNode),
}

impl AstNode for Statement {
    fn can_cast(kind: SyntaxKind) -> bool {
        matches!(
            kind,
            SyntaxKind::Block
                | SyntaxKind::LocalDeclaration
                | SyntaxKind::ReturnStatement
                | SyntaxKind::ThrowStatement
                | SyntaxKind::TryStatement
                | SyntaxKind::IfStatement
                | SyntaxKind::WhileStatement
                | SyntaxKind::ForStatement
                | SyntaxKind::BreakStatement
                | SyntaxKind::ContinueStatement
                | SyntaxKind::ExpressionStatement
                | SyntaxKind::EmptyStatement
                | SyntaxKind::Error
        )
    }

    fn cast(syntax: SyntaxNode) -> Option<Self> {
        match syntax.kind() {
            SyntaxKind::Block => Block::cast(syntax).map(Self::Block),
            SyntaxKind::LocalDeclaration => LocalDeclaration::cast(syntax).map(Self::Local),
            SyntaxKind::ReturnStatement => ReturnStatement::cast(syntax).map(Self::Return),
            SyntaxKind::ThrowStatement => ThrowStatement::cast(syntax).map(Self::Throw),
            SyntaxKind::TryStatement => TryStatement::cast(syntax).map(Self::Try),
            SyntaxKind::IfStatement => IfStatement::cast(syntax).map(Self::If),
            SyntaxKind::WhileStatement => WhileStatement::cast(syntax).map(Self::While),
            SyntaxKind::ForStatement => ForStatement::cast(syntax).map(Self::For),
            SyntaxKind::BreakStatement => BreakStatement::cast(syntax).map(Self::Break),
            SyntaxKind::ContinueStatement => ContinueStatement::cast(syntax).map(Self::Continue),
            SyntaxKind::ExpressionStatement => {
                ExpressionStatement::cast(syntax).map(Self::Expression)
            }
            SyntaxKind::EmptyStatement => EmptyStatement::cast(syntax).map(Self::Empty),
            SyntaxKind::Error => ErrorNode::cast(syntax).map(Self::Error),
            _ => None,
        }
    }

    fn syntax(&self) -> &SyntaxNode {
        match self {
            Self::Block(node) => node.syntax(),
            Self::Local(node) => node.syntax(),
            Self::Return(node) => node.syntax(),
            Self::Throw(node) => node.syntax(),
            Self::Try(node) => node.syntax(),
            Self::If(node) => node.syntax(),
            Self::While(node) => node.syntax(),
            Self::For(node) => node.syntax(),
            Self::Break(node) => node.syntax(),
            Self::Continue(node) => node.syntax(),
            Self::Expression(node) => node.syntax(),
            Self::Empty(node) => node.syntax(),
            Self::Error(node) => node.syntax(),
        }
    }
}

/// A range or classic `for` clause.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum ForClause {
    Range(RangeForClause),
    Classic(ClassicForClause),
}

impl AstNode for ForClause {
    fn can_cast(kind: SyntaxKind) -> bool {
        matches!(
            kind,
            SyntaxKind::RangeForClause | SyntaxKind::ClassicForClause
        )
    }

    fn cast(syntax: SyntaxNode) -> Option<Self> {
        match syntax.kind() {
            SyntaxKind::RangeForClause => RangeForClause::cast(syntax).map(Self::Range),
            SyntaxKind::ClassicForClause => ClassicForClause::cast(syntax).map(Self::Classic),
            _ => None,
        }
    }

    fn syntax(&self) -> &SyntaxNode {
        match self {
            Self::Range(node) => node.syntax(),
            Self::Classic(node) => node.syntax(),
        }
    }
}

/// The initializer of a classic `for` clause.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum ClassicForInitializer {
    /// A local declaration, such as `i32 index = 0`.
    Declaration(LocalDeclaration),
    /// An expression, such as `index = 0`.
    Expression(Expression),
}

/// An expression accepted by the initial Pratt parser.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum Expression {
    Name(NameExpression),
    Literal(LiteralExpression),
    Parenthesized(ParenthesizedExpression),
    Prefix(PrefixExpression),
    Postfix(PostfixExpression),
    Binary(BinaryExpression),
    Switch(SwitchExpression),
    Call(CallExpression),
    MacroCall(MacroCallExpression),
    Aggregate(AggregateExpression),
    JsonArray(JsonArrayExpression),
    JsonObject(JsonObjectExpression),
    Field(FieldExpression),
    Index(IndexExpression),
    Lambda(LambdaExpression),
    Await(AwaitExpression),
    Error(ErrorNode),
}

impl AstNode for Expression {
    fn can_cast(kind: SyntaxKind) -> bool {
        matches!(
            kind,
            SyntaxKind::NameExpression
                | SyntaxKind::LiteralExpression
                | SyntaxKind::ParenthesizedExpression
                | SyntaxKind::PrefixExpression
                | SyntaxKind::PostfixExpression
                | SyntaxKind::BinaryExpression
                | SyntaxKind::SwitchExpression
                | SyntaxKind::CallExpression
                | SyntaxKind::MacroCallExpression
                | SyntaxKind::AggregateExpression
                | SyntaxKind::JsonArrayExpression
                | SyntaxKind::JsonObjectExpression
                | SyntaxKind::FieldExpression
                | SyntaxKind::IndexExpression
                | SyntaxKind::LambdaExpression
                | SyntaxKind::AwaitExpression
                | SyntaxKind::Error
        )
    }

    fn cast(syntax: SyntaxNode) -> Option<Self> {
        match syntax.kind() {
            SyntaxKind::NameExpression => NameExpression::cast(syntax).map(Self::Name),
            SyntaxKind::LiteralExpression => LiteralExpression::cast(syntax).map(Self::Literal),
            SyntaxKind::ParenthesizedExpression => {
                ParenthesizedExpression::cast(syntax).map(Self::Parenthesized)
            }
            SyntaxKind::PrefixExpression => PrefixExpression::cast(syntax).map(Self::Prefix),
            SyntaxKind::PostfixExpression => PostfixExpression::cast(syntax).map(Self::Postfix),
            SyntaxKind::BinaryExpression => BinaryExpression::cast(syntax).map(Self::Binary),
            SyntaxKind::SwitchExpression => SwitchExpression::cast(syntax).map(Self::Switch),
            SyntaxKind::CallExpression => CallExpression::cast(syntax).map(Self::Call),
            SyntaxKind::MacroCallExpression => {
                MacroCallExpression::cast(syntax).map(Self::MacroCall)
            }
            SyntaxKind::AggregateExpression => {
                AggregateExpression::cast(syntax).map(Self::Aggregate)
            }
            SyntaxKind::JsonArrayExpression => {
                JsonArrayExpression::cast(syntax).map(Self::JsonArray)
            }
            SyntaxKind::JsonObjectExpression => {
                JsonObjectExpression::cast(syntax).map(Self::JsonObject)
            }
            SyntaxKind::FieldExpression => FieldExpression::cast(syntax).map(Self::Field),
            SyntaxKind::IndexExpression => IndexExpression::cast(syntax).map(Self::Index),
            SyntaxKind::LambdaExpression => LambdaExpression::cast(syntax).map(Self::Lambda),
            SyntaxKind::AwaitExpression => AwaitExpression::cast(syntax).map(Self::Await),
            SyntaxKind::Error => ErrorNode::cast(syntax).map(Self::Error),
            _ => None,
        }
    }

    fn syntax(&self) -> &SyntaxNode {
        match self {
            Self::Name(node) => node.syntax(),
            Self::Literal(node) => node.syntax(),
            Self::Parenthesized(node) => node.syntax(),
            Self::Prefix(node) => node.syntax(),
            Self::Postfix(node) => node.syntax(),
            Self::Binary(node) => node.syntax(),
            Self::Switch(node) => node.syntax(),
            Self::Call(node) => node.syntax(),
            Self::MacroCall(node) => node.syntax(),
            Self::Aggregate(node) => node.syntax(),
            Self::JsonArray(node) => node.syntax(),
            Self::JsonObject(node) => node.syntax(),
            Self::Field(node) => node.syntax(),
            Self::Index(node) => node.syntax(),
            Self::Lambda(node) => node.syntax(),
            Self::Await(node) => node.syntax(),
            Self::Error(node) => node.syntax(),
        }
    }
}

impl SourceFile {
    #[must_use]
    pub fn items(&self) -> AstChildren<Item> {
        children(self.syntax())
    }
}

impl NamespaceDefinition {
    #[must_use]
    pub fn name_token(&self) -> Option<SyntaxToken> {
        token(self.syntax(), SyntaxKind::Identifier)
    }

    #[must_use]
    pub fn items(&self) -> AstChildren<Item> {
        children(self.syntax())
    }
}

macro_rules! type_definition_accessors {
    ($name:ident) => {
        impl $name {
            #[must_use]
            pub fn name_token(&self) -> Option<SyntaxToken> {
                direct_tokens(self.syntax())
                    .filter(|token| token.kind() == SyntaxKind::Identifier)
                    .find(|token| token.text() != "sealed")
            }

            #[must_use]
            pub fn is_sealed(&self) -> bool {
                direct_tokens(self.syntax())
                    .any(|token| token.kind() == SyntaxKind::Identifier && token.text() == "sealed")
            }

            pub fn bases(&self) -> AstChildren<TypeReference> {
                children(self.syntax())
            }

            pub fn generic_parameters(&self) -> AstChildren<GenericParameter> {
                child::<GenericParameterList>(self.syntax()).map_or_else(
                    || children(self.syntax()),
                    |parameters| children(parameters.syntax()),
                )
            }

            #[must_use]
            pub fn fields(&self) -> AstChildren<FieldDeclaration> {
                children(self.syntax())
            }

            pub fn functions(&self) -> impl Iterator<Item = Function> + '_ {
                self.syntax().children().filter_map(Function::cast)
            }

            pub fn constructors(&self) -> impl Iterator<Item = Constructor> + '_ {
                self.syntax().children().filter_map(Constructor::cast)
            }

            #[must_use]
            pub fn member_is_public(&self, member: &SyntaxNode, default_public: bool) -> bool {
                let mut public = default_public;
                for child in self.syntax().children() {
                    if child.kind() == SyntaxKind::AccessSpecifier {
                        public = token(&child, SyntaxKind::PublicKw).is_some();
                    } else if child == *member {
                        return public;
                    }
                }
                public
            }

            #[must_use]
            pub fn has_access_specifier(&self) -> bool {
                child::<AccessSpecifier>(self.syntax()).is_some()
            }
        }
    };
}

type_definition_accessors!(StructDefinition);
type_definition_accessors!(ClassDefinition);
type_definition_accessors!(InterfaceDefinition);

impl FieldDeclaration {
    #[must_use]
    pub fn is_static(&self) -> bool {
        self.syntax()
            .children_with_tokens()
            .find(|element| match element {
                rowan::NodeOrToken::Node(_) => true,
                rowan::NodeOrToken::Token(token) => !token.kind().is_trivia(),
            })
            .and_then(rowan::NodeOrToken::into_token)
            .is_some_and(|token| token.kind() == SyntaxKind::Identifier && token.text() == "static")
    }

    #[must_use]
    pub fn ty(&self) -> Option<TypeReference> {
        child(self.syntax())
    }

    #[must_use]
    pub fn name_token(&self) -> Option<SyntaxToken> {
        direct_tokens(self.syntax())
            .filter(|token| token.kind() == SyntaxKind::Identifier)
            .last()
    }

    #[must_use]
    pub fn initializer(&self) -> Option<Expression> {
        expression_children(self.syntax()).next()
    }
}

/// A constructor definition, declaration, or deletion.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum Constructor {
    Definition(ConstructorDefinition),
    Declaration(ConstructorDeclaration),
}

impl AstNode for Constructor {
    fn can_cast(kind: SyntaxKind) -> bool {
        matches!(
            kind,
            SyntaxKind::ConstructorDefinition | SyntaxKind::ConstructorDeclaration
        )
    }

    fn cast(syntax: SyntaxNode) -> Option<Self> {
        match syntax.kind() {
            SyntaxKind::ConstructorDefinition => {
                ConstructorDefinition::cast(syntax).map(Self::Definition)
            }
            SyntaxKind::ConstructorDeclaration => {
                ConstructorDeclaration::cast(syntax).map(Self::Declaration)
            }
            _ => None,
        }
    }

    fn syntax(&self) -> &SyntaxNode {
        match self {
            Self::Definition(node) => node.syntax(),
            Self::Declaration(node) => node.syntax(),
        }
    }
}

macro_rules! constructor_accessors {
    ($name:ident) => {
        impl $name {
            pub fn name_tokens(&self) -> impl Iterator<Item = SyntaxToken> + '_ {
                direct_tokens(self.syntax()).filter(|token| {
                    matches!(
                        token.kind(),
                        SyntaxKind::Identifier | SyntaxKind::ColonColon
                    ) && token.text() != "delete"
                })
            }

            pub fn owner_generic_arguments(&self) -> impl Iterator<Item = TypeReference> + '_ {
                child::<GenericArgumentList>(self.syntax())
                    .into_iter()
                    .flat_map(|arguments| arguments.types().collect::<Vec<_>>())
            }

            #[must_use]
            pub fn parameter_list(&self) -> Option<ParameterList> {
                child(self.syntax())
            }

            #[must_use]
            pub fn throws_clause(&self) -> Option<ThrowsClause> {
                child(self.syntax())
            }
        }
    };
}

constructor_accessors!(ConstructorDefinition);
constructor_accessors!(ConstructorDeclaration);

impl ConstructorDefinition {
    #[must_use]
    pub fn initializer_list(&self) -> Option<ConstructorInitializerList> {
        child(self.syntax())
    }

    #[must_use]
    pub fn body(&self) -> Option<Block> {
        child(self.syntax())
    }
}

impl ConstructorDeclaration {
    #[must_use]
    pub fn is_deleted(&self) -> bool {
        direct_tokens(self.syntax())
            .any(|token| token.kind() == SyntaxKind::Identifier && token.text() == "delete")
    }
}

impl Constructor {
    #[must_use]
    pub fn name_tokens(&self) -> Box<dyn Iterator<Item = SyntaxToken> + '_> {
        match self {
            Self::Definition(node) => Box::new(node.name_tokens()),
            Self::Declaration(node) => Box::new(node.name_tokens()),
        }
    }

    #[must_use]
    pub fn owner_generic_arguments(&self) -> Box<dyn Iterator<Item = TypeReference> + '_> {
        match self {
            Self::Definition(node) => Box::new(node.owner_generic_arguments()),
            Self::Declaration(node) => Box::new(node.owner_generic_arguments()),
        }
    }

    #[must_use]
    pub fn parameter_list(&self) -> Option<ParameterList> {
        match self {
            Self::Definition(node) => node.parameter_list(),
            Self::Declaration(node) => node.parameter_list(),
        }
    }

    #[must_use]
    pub fn throws_clause(&self) -> Option<ThrowsClause> {
        match self {
            Self::Definition(node) => node.throws_clause(),
            Self::Declaration(node) => node.throws_clause(),
        }
    }

    #[must_use]
    pub fn initializer_list(&self) -> Option<ConstructorInitializerList> {
        match self {
            Self::Definition(node) => node.initializer_list(),
            Self::Declaration(_) => None,
        }
    }

    #[must_use]
    pub fn body(&self) -> Option<Block> {
        match self {
            Self::Definition(node) => node.body(),
            Self::Declaration(_) => None,
        }
    }

    #[must_use]
    pub fn is_deleted(&self) -> bool {
        matches!(self, Self::Declaration(node) if node.is_deleted())
    }
}

impl ConstructorInitializerList {
    #[must_use]
    pub fn initializers(&self) -> AstChildren<ConstructorInitializer> {
        children(self.syntax())
    }
}

impl ConstructorInitializer {
    pub fn name_tokens(&self) -> impl Iterator<Item = SyntaxToken> + '_ {
        direct_tokens(self.syntax()).filter(|token| {
            matches!(
                token.kind(),
                SyntaxKind::Identifier | SyntaxKind::ColonColon
            )
        })
    }

    #[must_use]
    pub fn argument_list(&self) -> Option<ArgumentList> {
        child(self.syntax())
    }
}

macro_rules! function_accessors {
    ($name:ident) => {
        impl $name {
            #[must_use]
            pub fn return_type(&self) -> Option<TypeReference> {
                child(self.syntax())
            }

            pub fn name_tokens(&self) -> impl Iterator<Item = SyntaxToken> + '_ {
                direct_tokens(self.syntax()).filter(|token| {
                    matches!(
                        token.kind(),
                        SyntaxKind::Identifier | SyntaxKind::ColonColon
                    ) && token.text() != "static"
                })
            }

            pub fn owner_generic_arguments(&self) -> impl Iterator<Item = TypeReference> + '_ {
                child::<GenericArgumentList>(self.syntax())
                    .into_iter()
                    .flat_map(|arguments| arguments.types().collect::<Vec<_>>())
            }

            #[must_use]
            pub fn parameter_list(&self) -> Option<ParameterList> {
                child(self.syntax())
            }

            #[must_use]
            pub fn throws_clause(&self) -> Option<ThrowsClause> {
                child(self.syntax())
            }

            #[must_use]
            pub fn is_static(&self) -> bool {
                direct_tokens(self.syntax())
                    .find(|token| !token.kind().is_trivia())
                    .is_some_and(|token| {
                        token.kind() == SyntaxKind::Identifier && token.text() == "static"
                    })
            }

            #[must_use]
            pub fn is_const(&self) -> bool {
                token(self.syntax(), SyntaxKind::ConstKw).is_some()
            }

            #[must_use]
            pub fn is_async(&self) -> bool {
                token(self.syntax(), SyntaxKind::AsyncKw).is_some()
            }
        }
    };
}

function_accessors!(FunctionDefinition);
function_accessors!(FunctionDeclaration);

impl FunctionDefinition {
    #[must_use]
    pub fn body(&self) -> Option<Block> {
        child(self.syntax())
    }
}

impl Function {
    #[must_use]
    pub fn return_type(&self) -> Option<TypeReference> {
        match self {
            Self::Definition(node) => node.return_type(),
            Self::Declaration(node) => node.return_type(),
        }
    }

    #[must_use]
    pub fn name_tokens(&self) -> Box<dyn Iterator<Item = SyntaxToken> + '_> {
        match self {
            Self::Definition(node) => Box::new(node.name_tokens()),
            Self::Declaration(node) => Box::new(node.name_tokens()),
        }
    }

    #[must_use]
    pub fn owner_generic_arguments(&self) -> Box<dyn Iterator<Item = TypeReference> + '_> {
        match self {
            Self::Definition(node) => Box::new(node.owner_generic_arguments()),
            Self::Declaration(node) => Box::new(node.owner_generic_arguments()),
        }
    }

    #[must_use]
    pub fn parameter_list(&self) -> Option<ParameterList> {
        match self {
            Self::Definition(node) => node.parameter_list(),
            Self::Declaration(node) => node.parameter_list(),
        }
    }

    #[must_use]
    pub fn throws_clause(&self) -> Option<ThrowsClause> {
        match self {
            Self::Definition(node) => node.throws_clause(),
            Self::Declaration(node) => node.throws_clause(),
        }
    }

    #[must_use]
    pub fn is_const(&self) -> bool {
        match self {
            Self::Definition(node) => node.is_const(),
            Self::Declaration(node) => node.is_const(),
        }
    }

    #[must_use]
    pub fn is_static(&self) -> bool {
        match self {
            Self::Definition(node) => node.is_static(),
            Self::Declaration(node) => node.is_static(),
        }
    }

    #[must_use]
    pub fn is_async(&self) -> bool {
        match self {
            Self::Definition(node) => node.is_async(),
            Self::Declaration(node) => node.is_async(),
        }
    }

    #[must_use]
    pub fn body(&self) -> Option<Block> {
        match self {
            Self::Definition(node) => node.body(),
            Self::Declaration(_) => None,
        }
    }
}

impl ParameterList {
    #[must_use]
    pub fn parameters(&self) -> AstChildren<Parameter> {
        children(self.syntax())
    }
}

impl Parameter {
    #[must_use]
    pub fn ty(&self) -> Option<TypeReference> {
        child(self.syntax())
    }

    #[must_use]
    pub fn name_token(&self) -> Option<SyntaxToken> {
        token(self.syntax(), SyntaxKind::Identifier)
    }
}

impl TypeReference {
    #[must_use]
    pub fn is_const(&self) -> bool {
        token(self.syntax(), SyntaxKind::ConstKw).is_some()
    }

    #[must_use]
    pub fn is_auto(&self) -> bool {
        token(self.syntax(), SyntaxKind::AutoKw).is_some()
    }

    #[must_use]
    pub fn is_reference(&self) -> bool {
        token(self.syntax(), SyntaxKind::Amp).is_some()
    }

    #[must_use]
    pub fn const_integer_token(&self) -> Option<SyntaxToken> {
        direct_tokens(self.syntax()).find(|token| token.kind() == SyntaxKind::Integer)
    }

    pub fn path_tokens(&self) -> impl Iterator<Item = SyntaxToken> + '_ {
        direct_tokens(self.syntax()).filter(|token| {
            matches!(
                token.kind(),
                SyntaxKind::Identifier | SyntaxKind::ColonColon
            )
        })
    }

    pub fn generic_arguments(&self) -> impl Iterator<Item = TypeReference> + '_ {
        child::<GenericArgumentList>(self.syntax())
            .into_iter()
            .flat_map(|arguments| arguments.types().collect::<Vec<_>>())
    }

    #[must_use]
    pub fn function_signature(&self) -> Option<FunctionTypeSignature> {
        child::<GenericArgumentList>(self.syntax()).and_then(|arguments| child(arguments.syntax()))
    }
}

impl GenericArgumentList {
    #[must_use]
    pub fn types(&self) -> AstChildren<TypeReference> {
        children(self.syntax())
    }
}

impl GenericParameter {
    #[must_use]
    pub fn is_const(&self) -> bool {
        self.identifier_tokens().count() == 2
    }

    #[must_use]
    pub fn name_token(&self) -> Option<SyntaxToken> {
        self.identifier_tokens().last()
    }

    #[must_use]
    pub fn const_type_token(&self) -> Option<SyntaxToken> {
        self.is_const()
            .then(|| self.identifier_tokens().next())
            .flatten()
    }

    fn identifier_tokens(&self) -> impl DoubleEndedIterator<Item = SyntaxToken> + '_ {
        direct_tokens(self.syntax())
            .filter(|token| token.kind() == SyntaxKind::Identifier)
            .collect::<Vec<_>>()
            .into_iter()
    }
}

impl FunctionTypeSignature {
    #[must_use]
    pub fn types(&self) -> AstChildren<TypeReference> {
        children(self.syntax())
    }
}

impl ThrowsClause {
    #[must_use]
    pub fn types(&self) -> AstChildren<TypeReference> {
        children(self.syntax())
    }
}

impl Block {
    pub fn statements(&self) -> impl Iterator<Item = Statement> + '_ {
        self.syntax().children().filter_map(Statement::cast)
    }
}

impl LocalDeclaration {
    #[must_use]
    pub fn ty(&self) -> Option<TypeReference> {
        child(self.syntax())
    }

    #[must_use]
    pub fn name_token(&self) -> Option<SyntaxToken> {
        token(self.syntax(), SyntaxKind::Identifier)
    }

    #[must_use]
    pub fn initializer(&self) -> Option<Expression> {
        expression_children(self.syntax()).next()
    }
}

impl ReturnStatement {
    #[must_use]
    pub fn value(&self) -> Option<Expression> {
        expression_children(self.syntax()).next()
    }
}

impl ThrowStatement {
    #[must_use]
    pub fn value(&self) -> Option<Expression> {
        expression_children(self.syntax()).next()
    }
}

impl TryStatement {
    #[must_use]
    pub fn body(&self) -> Option<Block> {
        child(self.syntax())
    }

    #[must_use]
    pub fn catches(&self) -> AstChildren<CatchClause> {
        children(self.syntax())
    }
}

impl CatchClause {
    #[must_use]
    pub fn is_catch_all(&self) -> bool {
        token(self.syntax(), SyntaxKind::Ellipsis).is_some()
    }

    #[must_use]
    pub fn ty(&self) -> Option<TypeReference> {
        child(self.syntax())
    }

    #[must_use]
    pub fn name_token(&self) -> Option<SyntaxToken> {
        direct_tokens(self.syntax()).find(|token| token.kind() == SyntaxKind::Identifier)
    }

    #[must_use]
    pub fn body(&self) -> Option<Block> {
        child(self.syntax())
    }
}

impl IfStatement {
    #[must_use]
    pub fn condition(&self) -> Option<Expression> {
        expression_children(self.syntax()).next()
    }

    #[must_use]
    pub fn then_branch(&self) -> Option<Statement> {
        self.syntax().children().find_map(Statement::cast)
    }

    #[must_use]
    pub fn else_clause(&self) -> Option<ElseClause> {
        child(self.syntax())
    }
}

impl ElseClause {
    #[must_use]
    pub fn branch(&self) -> Option<Statement> {
        self.syntax().children().find_map(Statement::cast)
    }
}

impl WhileStatement {
    #[must_use]
    pub fn condition(&self) -> Option<Expression> {
        expression_children(self.syntax()).next()
    }

    #[must_use]
    pub fn body(&self) -> Option<Statement> {
        self.syntax().children().find_map(Statement::cast)
    }
}

impl ForStatement {
    #[must_use]
    pub fn clause(&self) -> Option<ForClause> {
        self.syntax().children().find_map(ForClause::cast)
    }

    #[must_use]
    pub fn body(&self) -> Option<Statement> {
        self.syntax().children().find_map(Statement::cast)
    }
}

impl RangeForClause {
    #[must_use]
    pub fn ty(&self) -> Option<TypeReference> {
        child(self.syntax())
    }

    #[must_use]
    pub fn name_token(&self) -> Option<SyntaxToken> {
        self.name_tokens().next()
    }

    pub fn name_tokens(&self) -> impl Iterator<Item = SyntaxToken> + '_ {
        direct_tokens(self.syntax()).filter(|token| token.kind() == SyntaxKind::Identifier)
    }

    #[must_use]
    pub fn is_structured(&self) -> bool {
        direct_tokens(self.syntax()).any(|token| token.kind() == SyntaxKind::LBracket)
    }

    #[must_use]
    pub fn iterable(&self) -> Option<Expression> {
        expression_children(self.syntax()).next()
    }
}

impl ClassicForClause {
    #[must_use]
    pub fn initializer(&self) -> Option<ClassicForInitializer> {
        self.parts().initializer
    }

    #[must_use]
    pub fn condition(&self) -> Option<Expression> {
        self.parts().condition
    }

    #[must_use]
    pub fn update(&self) -> Option<Expression> {
        self.parts().update
    }

    fn parts(&self) -> ClassicForParts {
        let mut parts = ClassicForParts::default();
        let mut slot = 0_u8;

        for element in self.syntax().children_with_tokens() {
            match element {
                rowan::NodeOrToken::Node(node) => {
                    if let Some(declaration) = LocalDeclaration::cast(node.clone()) {
                        parts.initializer = Some(ClassicForInitializer::Declaration(declaration));
                        // The declaration owns its terminating semicolon.
                        slot = 1;
                    } else if let Some(expression) = Expression::cast(node) {
                        match slot {
                            0 => {
                                parts.initializer =
                                    Some(ClassicForInitializer::Expression(expression));
                            }
                            1 => parts.condition = Some(expression),
                            _ => parts.update = Some(expression),
                        }
                    }
                }
                rowan::NodeOrToken::Token(token) if token.kind() == SyntaxKind::Semicolon => {
                    slot = slot.saturating_add(1);
                }
                rowan::NodeOrToken::Token(_) => {}
            }
        }

        parts
    }
}

impl ExpressionStatement {
    #[must_use]
    pub fn expression(&self) -> Option<Expression> {
        expression_children(self.syntax()).next()
    }
}

impl NameExpression {
    pub fn path_tokens(&self) -> impl Iterator<Item = SyntaxToken> + '_ {
        direct_tokens(self.syntax()).filter(|token| {
            matches!(
                token.kind(),
                SyntaxKind::Identifier | SyntaxKind::MoveKw | SyntaxKind::ColonColon
            )
        })
    }

    /// Explicit type arguments on a generic call target.
    pub fn generic_arguments(&self) -> impl Iterator<Item = TypeReference> + '_ {
        child::<GenericArgumentList>(self.syntax())
            .into_iter()
            .flat_map(|arguments| arguments.types().collect::<Vec<_>>())
    }
}

impl LiteralExpression {
    #[must_use]
    pub fn literal_token(&self) -> Option<SyntaxToken> {
        direct_tokens(self.syntax()).find(|token| token.kind().is_literal())
    }
}

impl ParenthesizedExpression {
    #[must_use]
    pub fn expression(&self) -> Option<Expression> {
        expression_children(self.syntax()).next()
    }
}

impl PrefixExpression {
    #[must_use]
    pub fn operator_token(&self) -> Option<SyntaxToken> {
        direct_tokens(self.syntax()).find(|token| is_prefix_operator(token.kind()))
    }

    #[must_use]
    pub fn operand(&self) -> Option<Expression> {
        expression_children(self.syntax()).next()
    }
}

impl PostfixExpression {
    #[must_use]
    pub fn operator_token(&self) -> Option<SyntaxToken> {
        direct_tokens(self.syntax())
            .find(|token| matches!(token.kind(), SyntaxKind::PlusPlus | SyntaxKind::MinusMinus))
    }

    #[must_use]
    pub fn operand(&self) -> Option<Expression> {
        expression_children(self.syntax()).next()
    }
}

impl BinaryExpression {
    pub fn expressions(&self) -> impl Iterator<Item = Expression> + '_ {
        expression_children(self.syntax())
    }

    #[must_use]
    pub fn operator_token(&self) -> Option<SyntaxToken> {
        direct_tokens(self.syntax()).find(|token| is_binary_operator(token.kind()))
    }
}

impl SwitchExpression {
    #[must_use]
    pub fn scrutinee(&self) -> Option<Expression> {
        expression_children(self.syntax()).next()
    }

    #[must_use]
    pub fn arms(&self) -> AstChildren<SwitchArm> {
        children(self.syntax())
    }
}

impl SwitchArm {
    #[must_use]
    pub fn pattern(&self) -> Option<SwitchPattern> {
        child(self.syntax())
    }

    #[must_use]
    pub fn value(&self) -> Option<Expression> {
        expression_children(self.syntax()).next()
    }
}

impl SwitchPattern {
    #[must_use]
    pub fn token(&self) -> Option<SyntaxToken> {
        direct_tokens(self.syntax()).find(|token| {
            token.kind().is_literal()
                || (token.kind() == SyntaxKind::Identifier && token.text() == "_")
        })
    }
}

impl CallExpression {
    #[must_use]
    pub fn callee(&self) -> Option<Expression> {
        expression_children(self.syntax()).next()
    }

    #[must_use]
    pub fn argument_list(&self) -> Option<ArgumentList> {
        child(self.syntax())
    }
}

impl MacroCallExpression {
    #[must_use]
    pub fn callee(&self) -> Option<NameExpression> {
        child(self.syntax())
    }

    #[must_use]
    pub fn argument_list(&self) -> Option<ArgumentList> {
        child(self.syntax())
    }
}

impl ArgumentList {
    pub fn arguments(&self) -> impl Iterator<Item = Expression> + '_ {
        expression_children(self.syntax())
    }
}

impl AggregateExpression {
    #[must_use]
    pub fn ty(&self) -> Option<Expression> {
        expression_children(self.syntax()).next()
    }

    #[must_use]
    pub fn initializer_list(&self) -> Option<InitializerList> {
        child(self.syntax())
    }
}

impl InitializerList {
    pub fn initializers(&self) -> impl Iterator<Item = Expression> + '_ {
        expression_children(self.syntax())
    }
}

impl JsonArrayExpression {
    /// JSON element expressions in source order.
    pub fn elements(&self) -> impl Iterator<Item = Expression> + '_ {
        expression_children(self.syntax())
    }
}

impl JsonObjectExpression {
    /// JSON object members in source order.
    #[must_use]
    pub fn members(&self) -> AstChildren<JsonMember> {
        children(self.syntax())
    }
}

impl JsonMember {
    /// Identifier or string-literal key token.
    #[must_use]
    pub fn key_token(&self) -> Option<SyntaxToken> {
        direct_tokens(self.syntax())
            .find(|token| matches!(token.kind(), SyntaxKind::Identifier | SyntaxKind::String))
    }

    /// Member value expression.
    #[must_use]
    pub fn value(&self) -> Option<Expression> {
        expression_children(self.syntax()).next()
    }
}

impl FieldExpression {
    #[must_use]
    pub fn receiver(&self) -> Option<Expression> {
        expression_children(self.syntax()).next()
    }

    pub fn name_tokens(&self) -> impl Iterator<Item = SyntaxToken> + '_ {
        direct_tokens(self.syntax()).filter(|token| {
            matches!(
                token.kind(),
                SyntaxKind::Identifier | SyntaxKind::Integer | SyntaxKind::ColonColon
            )
        })
    }
}

impl IndexExpression {
    pub fn expressions(&self) -> impl Iterator<Item = Expression> + '_ {
        expression_children(self.syntax())
    }
}

impl LambdaExpression {
    #[must_use]
    pub fn capture_list(&self) -> Option<CaptureList> {
        child(self.syntax())
    }

    #[must_use]
    pub fn parameter_list(&self) -> Option<ParameterList> {
        child(self.syntax())
    }

    #[must_use]
    pub fn is_mutable(&self) -> bool {
        token(self.syntax(), SyntaxKind::MutableKw).is_some()
    }

    #[must_use]
    pub fn is_async(&self) -> bool {
        token(self.syntax(), SyntaxKind::AsyncKw).is_some()
    }

    #[must_use]
    pub fn body(&self) -> Option<Block> {
        child(self.syntax())
    }
}

impl AwaitExpression {
    #[must_use]
    pub fn operand(&self) -> Option<Expression> {
        expression_children(self.syntax()).next()
    }
}

impl CaptureList {
    #[must_use]
    pub fn captures(&self) -> AstChildren<LambdaCapture> {
        children(self.syntax())
    }
}

impl LambdaCapture {
    #[must_use]
    pub fn is_borrowed(&self) -> bool {
        token(self.syntax(), SyntaxKind::Amp).is_some()
    }

    #[must_use]
    pub fn name_token(&self) -> Option<SyntaxToken> {
        token(self.syntax(), SyntaxKind::Identifier)
    }

    #[must_use]
    pub fn initializer(&self) -> Option<Expression> {
        expression_children(self.syntax()).next()
    }
}

fn child<N: AstNode>(node: &SyntaxNode) -> Option<N> {
    node.children().find_map(N::cast)
}

fn children<N: AstNode>(node: &SyntaxNode) -> AstChildren<N> {
    AstChildren {
        inner: node.children(),
        node: PhantomData,
    }
}

fn expression_children(node: &SyntaxNode) -> impl Iterator<Item = Expression> + '_ {
    node.children().filter_map(Expression::cast)
}

fn token(node: &SyntaxNode, kind: SyntaxKind) -> Option<SyntaxToken> {
    direct_tokens(node).find(|token| token.kind() == kind)
}

fn direct_tokens(node: &SyntaxNode) -> impl Iterator<Item = SyntaxToken> + '_ {
    node.children_with_tokens()
        .filter_map(rowan::NodeOrToken::into_token)
}

fn is_prefix_operator(kind: SyntaxKind) -> bool {
    matches!(
        kind,
        SyntaxKind::Plus
            | SyntaxKind::Minus
            | SyntaxKind::Bang
            | SyntaxKind::Tilde
            | SyntaxKind::PlusPlus
            | SyntaxKind::MinusMinus
    )
}

fn is_binary_operator(kind: SyntaxKind) -> bool {
    matches!(
        kind,
        SyntaxKind::Eq
            | SyntaxKind::PlusEq
            | SyntaxKind::MinusEq
            | SyntaxKind::StarEq
            | SyntaxKind::SlashEq
            | SyntaxKind::PercentEq
            | SyntaxKind::OrOr
            | SyntaxKind::AndAnd
            | SyntaxKind::Pipe
            | SyntaxKind::Caret
            | SyntaxKind::Amp
            | SyntaxKind::EqEq
            | SyntaxKind::NotEq
            | SyntaxKind::Less
            | SyntaxKind::LessEq
            | SyntaxKind::Greater
            | SyntaxKind::GreaterEq
            | SyntaxKind::Plus
            | SyntaxKind::Minus
            | SyntaxKind::Star
            | SyntaxKind::Slash
            | SyntaxKind::Percent
    )
}

#[derive(Default)]
struct ClassicForParts {
    initializer: Option<ClassicForInitializer>,
    condition: Option<Expression>,
    update: Option<Expression>,
}
