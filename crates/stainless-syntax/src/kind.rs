use rowan::Language;

macro_rules! define_syntax_kinds {
    ($($kind:ident),+ $(,)?) => {
        /// A token or concrete-syntax node kind.
        #[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
        #[repr(u16)]
        pub enum SyntaxKind {
            $($kind),+
        }

        impl SyntaxKind {
            const ALL: &'static [Self] = &[$(Self::$kind),+];

            fn from_raw(raw: rowan::SyntaxKind) -> Self {
                Self::ALL
                    .get(usize::from(raw.0))
                    .copied()
                    .unwrap_or(Self::Error)
            }
        }
    };
}

define_syntax_kinds! {
    Whitespace,
    LineComment,
    BlockComment,
    ErrorToken,
    Identifier,
    Integer,
    Float,
    String,
    Character,

    NamespaceKw,
    UseKw,
    ReturnKw,
    IfKw,
    ElseKw,
    ForKw,
    ConstKw,
    AutoKw,
    TrueKw,
    FalseKw,
    MoveKw,
    StructKw,
    ClassKw,
    InterfaceKw,
    PublicKw,
    PrivateKw,
    ThrowsKw,
    ThrowKw,
    TryKw,
    CatchKw,
    WhileKw,
    BreakKw,
    ContinueKw,
    ModKw,
    AsKw,

    LParen,
    RParen,
    LBrace,
    RBrace,
    LBracket,
    RBracket,
    Less,
    Greater,
    LessEq,
    GreaterEq,
    EqEq,
    NotEq,
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Bang,
    Amp,
    Pipe,
    Caret,
    Tilde,
    AndAnd,
    OrOr,
    Eq,
    PlusEq,
    MinusEq,
    StarEq,
    SlashEq,
    PercentEq,
    PlusPlus,
    MinusMinus,
    Dot,
    Comma,
    Semicolon,
    Colon,
    ColonColon,

    SourceFile,
    NamespaceDefinition,
    UseDeclaration,
    FunctionDefinition,
    FunctionDeclaration,
    ParameterList,
    Parameter,
    TypeReference,
    GenericArgumentList,
    ThrowsClause,
    Block,
    LocalDeclaration,
    ReturnStatement,
    IfStatement,
    ElseClause,
    ForStatement,
    RangeForClause,
    ClassicForClause,
    BreakStatement,
    ContinueStatement,
    ExpressionStatement,
    EmptyStatement,
    NameExpression,
    LiteralExpression,
    ParenthesizedExpression,
    PrefixExpression,
    PostfixExpression,
    BinaryExpression,
    CallExpression,
    ArgumentList,
    FieldExpression,
    IndexExpression,
    Error,
}

impl SyntaxKind {
    /// Returns whether this token is whitespace or a comment.
    #[must_use]
    pub const fn is_trivia(self) -> bool {
        matches!(
            self,
            Self::Whitespace | Self::LineComment | Self::BlockComment
        )
    }

    /// Returns whether this token begins a literal expression.
    #[must_use]
    pub const fn is_literal(self) -> bool {
        matches!(
            self,
            Self::Integer
                | Self::Float
                | Self::String
                | Self::Character
                | Self::TrueKw
                | Self::FalseKw
        )
    }
}

impl From<SyntaxKind> for rowan::SyntaxKind {
    fn from(kind: SyntaxKind) -> Self {
        Self(kind as u16)
    }
}

/// Rowan language marker for Stainless syntax trees.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum StainlessLanguage {}

impl Language for StainlessLanguage {
    type Kind = SyntaxKind;

    fn kind_from_raw(raw: rowan::SyntaxKind) -> Self::Kind {
        SyntaxKind::from_raw(raw)
    }

    fn kind_to_raw(kind: Self::Kind) -> rowan::SyntaxKind {
        kind.into()
    }
}

/// A Stainless concrete-syntax node.
pub type SyntaxNode = rowan::SyntaxNode<StainlessLanguage>;
/// A Stainless concrete-syntax token.
pub type SyntaxToken = rowan::SyntaxToken<StainlessLanguage>;
/// A node or token in a Stainless concrete-syntax tree.
pub type SyntaxElement = rowan::SyntaxElement<StainlessLanguage>;
