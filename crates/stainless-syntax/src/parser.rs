use rowan::{GreenNode, GreenNodeBuilder, TextRange, TextSize};

use crate::ast::AstNode;
use crate::{LexError, SyntaxKind, SyntaxNode, Token, ast, lex};

/// A recoverable parser diagnostic.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParseError {
    /// Human-readable diagnostic.
    pub message: String,
    /// Offending byte range, or an empty range at end of input.
    pub range: TextRange,
}

impl From<LexError> for ParseError {
    fn from(error: LexError) -> Self {
        Self {
            message: error.message,
            range: error.range,
        }
    }
}

/// The lossless result of parsing one Stainless source file.
#[derive(Clone, Debug)]
pub struct Parse {
    green: GreenNode,
    errors: Vec<ParseError>,
}

impl Parse {
    /// Returns a fresh root handle for the immutable concrete syntax tree.
    #[must_use]
    pub fn syntax(&self) -> SyntaxNode {
        SyntaxNode::new_root(self.green.clone())
    }

    /// Returns the typed root of the concrete syntax tree.
    ///
    /// # Panics
    ///
    /// Panics only if an internal parser invariant is violated and the root
    /// node is not a source-file node.
    #[must_use]
    pub fn tree(&self) -> ast::SourceFile {
        ast::SourceFile::cast(self.syntax()).expect("the parser always creates a source-file root")
    }

    /// Returns lexical and syntactic diagnostics in source order.
    #[must_use]
    pub fn errors(&self) -> &[ParseError] {
        &self.errors
    }
}

/// Parses one Stainless source file into a lossless Rowan tree.
#[must_use]
pub fn parse(source: &str) -> Parse {
    let lexed = lex(source);
    let mut parser = Parser {
        source,
        tokens: lexed.tokens,
        position: 0,
        builder: GreenNodeBuilder::new(),
        errors: lexed.errors.into_iter().map(Into::into).collect(),
    };
    parser.parse_source_file();
    let green = parser.builder.finish();
    parser.errors.sort_by_key(|error| error.range.start());
    Parse {
        green,
        errors: parser.errors,
    }
}

struct Parser<'source> {
    source: &'source str,
    tokens: Vec<Token>,
    position: usize,
    builder: GreenNodeBuilder<'static>,
    errors: Vec<ParseError>,
}

impl Parser<'_> {
    fn parse_source_file(&mut self) {
        self.builder.start_node(SyntaxKind::SourceFile.into());
        self.parse_item_list(None);
        self.eat_remaining();
        self.finish();
    }

    fn parse_item_list(&mut self, terminator: Option<SyntaxKind>) {
        while !self.at_end() && terminator.is_none_or(|kind| !self.at(kind)) {
            let previous = self.position;
            match self.current() {
                Some(SyntaxKind::NamespaceKw) => self.parse_namespace(),
                Some(SyntaxKind::UseKw) => self.parse_use_declaration(),
                Some(SyntaxKind::StructKw) => self.parse_struct_definition(),
                Some(SyntaxKind::Identifier) if self.looks_like_constructor() => {
                    self.parse_constructor();
                }
                Some(SyntaxKind::Identifier | SyntaxKind::ConstKw) => self.parse_function(),
                Some(_) => {
                    self.recover_item("expected a namespace, use declaration, struct, or function");
                }
                None => break,
            }
            if self.position == previous {
                self.recover_item("parser could not make progress");
            }
        }
    }

    fn parse_namespace(&mut self) {
        self.start(SyntaxKind::NamespaceDefinition);
        self.bump();
        self.expect(SyntaxKind::Identifier, "expected a namespace name");
        self.expect(SyntaxKind::LBrace, "expected `{` after namespace name");
        self.parse_item_list(Some(SyntaxKind::RBrace));
        self.expect(SyntaxKind::RBrace, "expected `}` to close namespace");
        self.finish();
    }

    fn parse_use_declaration(&mut self) {
        self.start(SyntaxKind::UseDeclaration);
        self.bump();
        let mut brace_depth = 0_u32;
        while !self.at_end() {
            match self.current() {
                Some(SyntaxKind::LBrace) => {
                    brace_depth += 1;
                    self.bump();
                }
                Some(SyntaxKind::RBrace) if brace_depth > 0 => {
                    brace_depth -= 1;
                    self.bump();
                }
                Some(SyntaxKind::Semicolon) if brace_depth == 0 => {
                    self.bump();
                    self.finish();
                    return;
                }
                _ => self.bump(),
            }
        }
        self.error("expected `;` after use declaration");
        self.finish();
    }

    fn parse_struct_definition(&mut self) {
        self.start(SyntaxKind::StructDefinition);
        self.bump();
        self.expect(SyntaxKind::Identifier, "expected a struct name");
        if self.eat(SyntaxKind::Colon) {
            self.parse_qualified_name("expected a data base struct");
        }
        self.expect(SyntaxKind::LBrace, "expected `{` after struct name");
        while !self.at_end() && !self.at(SyntaxKind::RBrace) {
            let previous = self.position;
            if self.at_any(&[SyntaxKind::PublicKw, SyntaxKind::PrivateKw]) {
                self.bump();
                self.expect(SyntaxKind::Colon, "expected `:` after access specifier");
            } else if self.at_any(&[SyntaxKind::Identifier, SyntaxKind::ConstKw]) {
                if self.looks_like_constructor() {
                    self.parse_constructor();
                } else if self.struct_member_is_function() {
                    self.parse_function();
                } else {
                    self.parse_field_declaration();
                }
            } else {
                self.recover_statement("expected a data field or member function declaration");
            }
            if self.position == previous {
                self.recover_statement("parser could not make progress in struct");
            }
        }
        self.expect(SyntaxKind::RBrace, "expected `}` to close struct");
        self.expect(
            SyntaxKind::Semicolon,
            "expected `;` after struct definition",
        );
        self.finish();
    }

    fn struct_member_is_function(&self) -> bool {
        let mut angle_depth = 0_u32;
        let mut offset = 0;
        while let Some(kind) = self.nth(offset) {
            match kind {
                SyntaxKind::Less => angle_depth += 1,
                SyntaxKind::Greater => angle_depth = angle_depth.saturating_sub(1),
                SyntaxKind::LParen if angle_depth == 0 => return true,
                SyntaxKind::Semicolon | SyntaxKind::RBrace if angle_depth == 0 => return false,
                _ => {}
            }
            offset += 1;
        }
        false
    }

    fn parse_field_declaration(&mut self) {
        self.start(SyntaxKind::FieldDeclaration);
        self.parse_type(false);
        self.expect(SyntaxKind::Identifier, "expected a field name");
        self.expect(
            SyntaxKind::Semicolon,
            "expected `;` after field declaration",
        );
        self.finish();
    }

    fn parse_constructor(&mut self) {
        self.start(self.constructor_node_kind());
        self.parse_qualified_name("expected a constructor name");
        self.parse_parameter_list();
        if self.at(SyntaxKind::ThrowsKw) {
            self.parse_throws_clause();
        }
        if self.at(SyntaxKind::Colon) {
            self.parse_constructor_initializer_list();
        }
        if self.at(SyntaxKind::LBrace) {
            self.parse_block();
        } else if self.eat(SyntaxKind::Eq) {
            if self.at(SyntaxKind::Identifier) {
                if self.current_text() != Some("delete") {
                    self.error("expected `delete` after `=`");
                }
                self.bump();
            } else {
                self.error("expected `delete` after `=`");
            }
            self.expect(
                SyntaxKind::Semicolon,
                "expected `;` after deleted constructor",
            );
        } else {
            self.expect(
                SyntaxKind::Semicolon,
                "expected a constructor body, `= delete;`, or `;`",
            );
        }
        self.finish();
    }

    fn constructor_node_kind(&self) -> SyntaxKind {
        let mut paren_depth = 0_u32;
        let mut offset = 0;
        while let Some(kind) = self.nth(offset) {
            match kind {
                SyntaxKind::LParen => paren_depth += 1,
                SyntaxKind::RParen => paren_depth = paren_depth.saturating_sub(1),
                SyntaxKind::LBrace if paren_depth == 0 => {
                    return SyntaxKind::ConstructorDefinition;
                }
                SyntaxKind::Semicolon if paren_depth == 0 => {
                    return SyntaxKind::ConstructorDeclaration;
                }
                _ => {}
            }
            offset += 1;
        }
        SyntaxKind::ConstructorDefinition
    }

    fn parse_constructor_initializer_list(&mut self) {
        self.start(SyntaxKind::ConstructorInitializerList);
        self.bump();
        loop {
            self.start(SyntaxKind::ConstructorInitializer);
            self.parse_qualified_name("expected a base or field initializer name");
            self.parse_argument_list();
            self.finish();
            if !self.eat(SyntaxKind::Comma) {
                break;
            }
        }
        self.finish();
    }

    fn looks_like_constructor(&self) -> bool {
        if self.nth(0) != Some(SyntaxKind::Identifier) {
            return false;
        }
        let mut offset = 1;
        while self.nth(offset) == Some(SyntaxKind::ColonColon)
            && self.nth(offset + 1) == Some(SyntaxKind::Identifier)
        {
            offset += 2;
        }
        self.nth(offset) == Some(SyntaxKind::LParen)
    }

    fn parse_function(&mut self) {
        let node_kind = self.function_node_kind();
        self.start(node_kind);
        self.parse_type(false);
        self.parse_qualified_name("expected a function name");
        self.parse_parameter_list();
        if self.at(SyntaxKind::ConstKw) {
            self.bump();
        }
        if self.at(SyntaxKind::ThrowsKw) {
            self.parse_throws_clause();
        }
        if self.at(SyntaxKind::LBrace) {
            self.parse_block();
        } else {
            self.expect(
                SyntaxKind::Semicolon,
                "expected a function body or `;` after declaration",
            );
        }
        self.finish();
    }

    fn function_node_kind(&self) -> SyntaxKind {
        let mut paren_depth = 0_u32;
        let mut offset = 0;
        while let Some(kind) = self.nth(offset) {
            match kind {
                SyntaxKind::LParen => paren_depth += 1,
                SyntaxKind::RParen => paren_depth = paren_depth.saturating_sub(1),
                SyntaxKind::LBrace if paren_depth == 0 => return SyntaxKind::FunctionDefinition,
                SyntaxKind::Semicolon if paren_depth == 0 => {
                    return SyntaxKind::FunctionDeclaration;
                }
                _ => {}
            }
            offset += 1;
        }
        SyntaxKind::FunctionDefinition
    }

    fn parse_parameter_list(&mut self) {
        self.start(SyntaxKind::ParameterList);
        self.expect(SyntaxKind::LParen, "expected `(` before parameters");
        while !self.at_end() && !self.at(SyntaxKind::RParen) {
            self.start(SyntaxKind::Parameter);
            self.parse_type(false);
            self.expect(SyntaxKind::Identifier, "expected a parameter name");
            self.finish();
            if !self.eat(SyntaxKind::Comma) {
                break;
            }
        }
        self.expect(SyntaxKind::RParen, "expected `)` after parameters");
        self.finish();
    }

    fn parse_throws_clause(&mut self) {
        self.start(SyntaxKind::ThrowsClause);
        self.bump();
        loop {
            self.parse_type(false);
            if !self.eat(SyntaxKind::Comma) {
                break;
            }
        }
        self.finish();
    }

    fn parse_type(&mut self, allow_auto: bool) {
        self.start(SyntaxKind::TypeReference);
        self.eat(SyntaxKind::ConstKw);
        if allow_auto && self.eat(SyntaxKind::AutoKw) {
            self.eat(SyntaxKind::Amp);
            self.finish();
            return;
        }

        if self.at(SyntaxKind::Identifier) {
            self.bump();
            while self.eat(SyntaxKind::ColonColon) {
                self.expect(SyntaxKind::Identifier, "expected a type path segment");
            }
            if self.at(SyntaxKind::Less) {
                self.parse_generic_arguments();
            }
        } else {
            self.error("expected a type");
            if !self.at_any(&[
                SyntaxKind::Identifier,
                SyntaxKind::Comma,
                SyntaxKind::RParen,
                SyntaxKind::Semicolon,
            ]) {
                self.bump();
            }
        }
        self.eat(SyntaxKind::Amp);
        self.finish();
    }

    fn parse_generic_arguments(&mut self) {
        self.start(SyntaxKind::GenericArgumentList);
        self.bump();
        while !self.at_end() && !self.at(SyntaxKind::Greater) {
            self.parse_type(false);
            if !self.eat(SyntaxKind::Comma) {
                break;
            }
        }
        self.expect(SyntaxKind::Greater, "expected `>` after generic arguments");
        self.finish();
    }

    fn parse_qualified_name(&mut self, message: &str) {
        self.expect(SyntaxKind::Identifier, message);
        while self.eat(SyntaxKind::ColonColon) {
            self.expect(SyntaxKind::Identifier, "expected a name after `::`");
        }
    }

    fn parse_block(&mut self) {
        self.start(SyntaxKind::Block);
        self.expect(SyntaxKind::LBrace, "expected `{`");
        while !self.at_end() && !self.at(SyntaxKind::RBrace) {
            let previous = self.position;
            self.parse_statement();
            if self.position == previous {
                self.recover_statement("parser could not make progress in block");
            }
        }
        self.expect(SyntaxKind::RBrace, "expected `}` to close block");
        self.finish();
    }

    fn parse_statement(&mut self) {
        match self.current() {
            Some(SyntaxKind::LBrace) => self.parse_block(),
            Some(SyntaxKind::ReturnKw) => self.parse_return_statement(),
            Some(SyntaxKind::IfKw) => self.parse_if_statement(),
            Some(SyntaxKind::ForKw) => self.parse_for_statement(),
            Some(SyntaxKind::BreakKw) => {
                self.parse_keyword_statement(SyntaxKind::BreakStatement);
            }
            Some(SyntaxKind::ContinueKw) => {
                self.parse_keyword_statement(SyntaxKind::ContinueStatement);
            }
            Some(SyntaxKind::Semicolon) => {
                self.start(SyntaxKind::EmptyStatement);
                self.bump();
                self.finish();
            }
            Some(_) if self.looks_like_declaration() => self.parse_local_declaration(),
            Some(_) => self.parse_expression_statement(),
            None => {}
        }
    }

    fn parse_local_declaration(&mut self) {
        self.start(SyntaxKind::LocalDeclaration);
        self.parse_type(true);
        self.expect(SyntaxKind::Identifier, "expected a local variable name");
        if self.eat(SyntaxKind::Eq) {
            self.parse_expression();
        }
        self.expect(
            SyntaxKind::Semicolon,
            "expected `;` after local declaration",
        );
        self.finish();
    }

    fn parse_return_statement(&mut self) {
        self.start(SyntaxKind::ReturnStatement);
        self.bump();
        if !self.at(SyntaxKind::Semicolon) {
            self.parse_expression();
        }
        self.expect(SyntaxKind::Semicolon, "expected `;` after return");
        self.finish();
    }

    fn parse_if_statement(&mut self) {
        self.start(SyntaxKind::IfStatement);
        self.bump();
        self.expect(SyntaxKind::LParen, "expected `(` after `if`");
        self.parse_expression();
        self.expect(SyntaxKind::RParen, "expected `)` after condition");
        self.parse_statement();
        if self.at(SyntaxKind::ElseKw) {
            self.start(SyntaxKind::ElseClause);
            self.bump();
            self.parse_statement();
            self.finish();
        }
        self.finish();
    }

    fn parse_for_statement(&mut self) {
        self.start(SyntaxKind::ForStatement);
        self.bump();
        self.expect(SyntaxKind::LParen, "expected `(` after `for`");
        if self.for_clause_is_range() {
            self.parse_range_for_clause();
        } else {
            self.parse_classic_for_clause();
        }
        self.expect(SyntaxKind::RParen, "expected `)` after for clause");
        self.parse_statement();
        self.finish();
    }

    fn parse_range_for_clause(&mut self) {
        self.start(SyntaxKind::RangeForClause);
        self.parse_type(true);
        self.expect(SyntaxKind::Identifier, "expected a range binding name");
        self.expect(
            SyntaxKind::Colon,
            "expected `:` between range binding and expression",
        );
        self.parse_expression();
        self.finish();
    }

    fn parse_classic_for_clause(&mut self) {
        self.start(SyntaxKind::ClassicForClause);
        if self.at(SyntaxKind::Semicolon) {
            self.bump();
        } else if self.looks_like_declaration() {
            self.parse_local_declaration();
        } else {
            self.parse_expression();
            self.expect(SyntaxKind::Semicolon, "expected `;` after for initializer");
        }

        if !self.at(SyntaxKind::Semicolon) {
            self.parse_expression();
        }
        self.expect(SyntaxKind::Semicolon, "expected `;` after for condition");

        if !self.at(SyntaxKind::RParen) {
            self.parse_expression();
        }
        self.finish();
    }

    fn parse_keyword_statement(&mut self, kind: SyntaxKind) {
        self.start(kind);
        self.bump();
        self.expect(SyntaxKind::Semicolon, "expected `;`");
        self.finish();
    }

    fn parse_expression_statement(&mut self) {
        self.start(SyntaxKind::ExpressionStatement);
        self.parse_expression();
        self.expect(SyntaxKind::Semicolon, "expected `;` after expression");
        self.finish();
    }

    fn parse_expression(&mut self) {
        self.parse_expression_bp(0);
    }

    fn parse_expression_bp(&mut self, minimum_binding_power: u8) {
        self.eat_trivia();
        let checkpoint = self.builder.checkpoint();
        self.parse_prefix_or_primary();
        self.parse_postfix(checkpoint);

        while let Some((left_power, right_power)) = infix_binding_power(self.current()) {
            if left_power < minimum_binding_power {
                break;
            }
            self.builder
                .start_node_at(checkpoint, SyntaxKind::BinaryExpression.into());
            self.bump();
            self.parse_expression_bp(right_power);
            self.finish();
        }
    }

    fn parse_prefix_or_primary(&mut self) {
        if self.at_any(&[
            SyntaxKind::Plus,
            SyntaxKind::Minus,
            SyntaxKind::Bang,
            SyntaxKind::Tilde,
            SyntaxKind::PlusPlus,
            SyntaxKind::MinusMinus,
        ]) {
            self.start(SyntaxKind::PrefixExpression);
            self.bump();
            self.parse_expression_bp(11);
            self.finish();
        } else {
            self.parse_primary();
        }
    }

    fn parse_primary(&mut self) {
        match self.current() {
            Some(kind) if kind.is_literal() => {
                self.start(SyntaxKind::LiteralExpression);
                self.bump();
                self.finish();
            }
            Some(SyntaxKind::Identifier | SyntaxKind::MoveKw) => {
                self.start(SyntaxKind::NameExpression);
                self.bump();
                while self.eat(SyntaxKind::ColonColon) {
                    self.expect(SyntaxKind::Identifier, "expected a name after `::`");
                }
                self.finish();
            }
            Some(SyntaxKind::LParen) => {
                self.start(SyntaxKind::ParenthesizedExpression);
                self.bump();
                self.parse_expression();
                self.expect(SyntaxKind::RParen, "expected `)`");
                self.finish();
            }
            _ => {
                self.start(SyntaxKind::Error);
                self.error("expected an expression");
                if !self.at_end()
                    && !self.at_any(&[
                        SyntaxKind::Comma,
                        SyntaxKind::Semicolon,
                        SyntaxKind::Colon,
                        SyntaxKind::RParen,
                        SyntaxKind::RBracket,
                        SyntaxKind::RBrace,
                    ])
                {
                    self.bump();
                }
                self.finish();
            }
        }
    }

    fn parse_postfix(&mut self, checkpoint: rowan::Checkpoint) {
        loop {
            match self.current() {
                Some(SyntaxKind::LParen) => {
                    self.builder
                        .start_node_at(checkpoint, SyntaxKind::CallExpression.into());
                    self.parse_argument_list();
                    self.finish();
                }
                Some(SyntaxKind::LBrace) => {
                    self.builder
                        .start_node_at(checkpoint, SyntaxKind::AggregateExpression.into());
                    self.parse_initializer_list();
                    self.finish();
                }
                Some(SyntaxKind::Dot) => {
                    self.builder
                        .start_node_at(checkpoint, SyntaxKind::FieldExpression.into());
                    self.bump();
                    self.expect(SyntaxKind::Identifier, "expected a member name after `.`");
                    while self.eat(SyntaxKind::ColonColon) {
                        self.expect(SyntaxKind::Identifier, "expected a member name after `::`");
                    }
                    self.finish();
                }
                Some(SyntaxKind::LBracket) => {
                    self.builder
                        .start_node_at(checkpoint, SyntaxKind::IndexExpression.into());
                    self.bump();
                    self.parse_expression();
                    self.expect(SyntaxKind::RBracket, "expected `]` after index");
                    self.finish();
                }
                Some(SyntaxKind::PlusPlus | SyntaxKind::MinusMinus) => {
                    self.builder
                        .start_node_at(checkpoint, SyntaxKind::PostfixExpression.into());
                    self.bump();
                    self.finish();
                }
                _ => break,
            }
        }
    }

    fn parse_argument_list(&mut self) {
        self.start(SyntaxKind::ArgumentList);
        self.bump();
        while !self.at_end() && !self.at(SyntaxKind::RParen) {
            self.parse_expression();
            if !self.eat(SyntaxKind::Comma) {
                break;
            }
        }
        self.expect(SyntaxKind::RParen, "expected `)` after arguments");
        self.finish();
    }

    fn parse_initializer_list(&mut self) {
        self.start(SyntaxKind::InitializerList);
        self.bump();
        while !self.at_end() && !self.at(SyntaxKind::RBrace) {
            self.parse_expression();
            if !self.eat(SyntaxKind::Comma) {
                break;
            }
        }
        self.expect(
            SyntaxKind::RBrace,
            "expected `}` after aggregate initializer",
        );
        self.finish();
    }

    fn looks_like_declaration(&self) -> bool {
        let mut offset = 0;
        if self.nth(offset) == Some(SyntaxKind::ConstKw) {
            offset += 1;
        }
        if self.nth(offset) == Some(SyntaxKind::AutoKw) {
            offset += 1;
        } else {
            if self.nth(offset) != Some(SyntaxKind::Identifier) {
                return false;
            }
            offset += 1;
            while self.nth(offset) == Some(SyntaxKind::ColonColon)
                && self.nth(offset + 1) == Some(SyntaxKind::Identifier)
            {
                offset += 2;
            }
            if self.nth(offset) == Some(SyntaxKind::Less) {
                let Some(after_generics) = self.skip_balanced_angles(offset) else {
                    return false;
                };
                offset = after_generics;
            }
        }
        if self.nth(offset) == Some(SyntaxKind::Amp) {
            offset += 1;
        }
        if self.nth(offset) != Some(SyntaxKind::Identifier) {
            return false;
        }
        matches!(
            self.nth(offset + 1),
            Some(SyntaxKind::Eq | SyntaxKind::Semicolon | SyntaxKind::Colon)
        )
    }

    fn skip_balanced_angles(&self, mut offset: usize) -> Option<usize> {
        let mut depth = 0_u32;
        loop {
            match self.nth(offset)? {
                SyntaxKind::Less => depth += 1,
                SyntaxKind::Greater => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(offset + 1);
                    }
                }
                _ => {}
            }
            offset += 1;
        }
    }

    fn for_clause_is_range(&self) -> bool {
        let mut offset = 0;
        let mut nesting = 0_u32;
        while let Some(kind) = self.nth(offset) {
            match kind {
                SyntaxKind::LParen | SyntaxKind::LBracket | SyntaxKind::LBrace => nesting += 1,
                SyntaxKind::RParen | SyntaxKind::Semicolon if nesting == 0 => return false,
                SyntaxKind::RParen | SyntaxKind::RBracket | SyntaxKind::RBrace => {
                    nesting = nesting.saturating_sub(1);
                }
                SyntaxKind::Colon if nesting == 0 => return true,
                _ => {}
            }
            offset += 1;
        }
        false
    }

    fn recover_item(&mut self, message: &str) {
        self.start(SyntaxKind::Error);
        self.error(message);
        let mut brace_depth = 0_u32;
        while !self.at_end() {
            match self.current() {
                Some(SyntaxKind::LBrace) => {
                    brace_depth += 1;
                    self.bump();
                }
                Some(SyntaxKind::RBrace) if brace_depth == 0 => break,
                Some(SyntaxKind::RBrace) => {
                    brace_depth -= 1;
                    self.bump();
                }
                Some(SyntaxKind::Semicolon) if brace_depth == 0 => {
                    self.bump();
                    break;
                }
                _ => self.bump(),
            }
        }
        self.finish();
    }

    fn recover_statement(&mut self, message: &str) {
        self.start(SyntaxKind::Error);
        self.error(message);
        while !self.at_end() && !self.at_any(&[SyntaxKind::Semicolon, SyntaxKind::RBrace]) {
            self.bump();
        }
        self.eat(SyntaxKind::Semicolon);
        self.finish();
    }

    fn start(&mut self, kind: SyntaxKind) {
        self.eat_trivia();
        self.builder.start_node(kind.into());
    }

    fn finish(&mut self) {
        self.builder.finish_node();
    }

    fn current(&self) -> Option<SyntaxKind> {
        self.nth(0)
    }

    fn current_text(&self) -> Option<&str> {
        let token = self.tokens.get(self.significant_position())?;
        let start = usize::from(token.range.start());
        let end = usize::from(token.range.end());
        Some(&self.source[start..end])
    }

    fn nth(&self, significant_offset: usize) -> Option<SyntaxKind> {
        self.tokens[self.position..]
            .iter()
            .filter(|token| !token.kind.is_trivia())
            .nth(significant_offset)
            .map(|token| token.kind)
    }

    fn at(&self, kind: SyntaxKind) -> bool {
        self.current() == Some(kind)
    }

    fn at_any(&self, kinds: &[SyntaxKind]) -> bool {
        self.current().is_some_and(|kind| kinds.contains(&kind))
    }

    fn at_end(&self) -> bool {
        self.current().is_none()
    }

    fn eat(&mut self, kind: SyntaxKind) -> bool {
        if self.at(kind) {
            self.bump();
            true
        } else {
            false
        }
    }

    fn expect(&mut self, kind: SyntaxKind, message: &str) {
        if !self.eat(kind) {
            self.error(message);
        }
    }

    fn bump(&mut self) {
        self.eat_trivia();
        self.bump_raw();
    }

    fn bump_raw(&mut self) {
        let Some(token) = self.tokens.get(self.position) else {
            return;
        };
        let kind = token.kind;
        let start = usize::from(token.range.start());
        let end = usize::from(token.range.end());
        let text = &self.source[start..end];
        self.builder.token(kind.into(), text);
        self.position += 1;
    }

    fn eat_trivia(&mut self) {
        while self
            .tokens
            .get(self.position)
            .is_some_and(|token| token.kind.is_trivia())
        {
            self.bump_raw();
        }
    }

    fn eat_remaining(&mut self) {
        while self.position < self.tokens.len() {
            self.bump_raw();
        }
    }

    fn error(&mut self, message: &str) {
        self.errors.push(ParseError {
            message: message.to_owned(),
            range: self
                .tokens
                .get(self.significant_position())
                .map_or_else(|| self.end_range(), |token| token.range),
        });
    }

    fn significant_position(&self) -> usize {
        self.tokens[self.position..]
            .iter()
            .position(|token| !token.kind.is_trivia())
            .map_or(self.tokens.len(), |offset| self.position + offset)
    }

    fn end_range(&self) -> TextRange {
        let end = TextSize::of(self.source);
        TextRange::empty(end)
    }
}

fn infix_binding_power(kind: Option<SyntaxKind>) -> Option<(u8, u8)> {
    let powers = match kind? {
        SyntaxKind::Eq
        | SyntaxKind::PlusEq
        | SyntaxKind::MinusEq
        | SyntaxKind::StarEq
        | SyntaxKind::SlashEq
        | SyntaxKind::PercentEq => (1, 1),
        SyntaxKind::OrOr => (2, 3),
        SyntaxKind::AndAnd => (3, 4),
        SyntaxKind::Pipe => (4, 5),
        SyntaxKind::Caret => (5, 6),
        SyntaxKind::Amp => (6, 7),
        SyntaxKind::EqEq | SyntaxKind::NotEq => (7, 8),
        SyntaxKind::Less | SyntaxKind::LessEq | SyntaxKind::Greater | SyntaxKind::GreaterEq => {
            (8, 9)
        }
        SyntaxKind::Plus | SyntaxKind::Minus => (9, 10),
        SyntaxKind::Star | SyntaxKind::Slash | SyntaxKind::Percent => (10, 11),
        _ => return None,
    };
    Some(powers)
}
