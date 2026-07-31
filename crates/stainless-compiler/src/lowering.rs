//! Lowering from the lossless typed CST into the compiler-owned AST.

use stainless_syntax::SyntaxKind;
use stainless_syntax::ast::{self as cst, AstNode};

use crate::ast::{
    self, BinaryOperator, Expression, ExpressionKind, ForClause, ForInitializer, Item, LiteralKind,
    PostfixOperator, PrefixOperator, Span, Statement, StatementKind, TypeKind,
};

/// Lowers a parsed source file.
///
/// Parser recovery nodes become explicit `Error` AST forms. Syntax diagnostics
/// remain available from [`stainless_syntax::Parse`].
#[must_use]
pub fn lower(source: &cst::SourceFile) -> ast::SourceFile {
    ast::SourceFile {
        items: source.items().map(lower_item).collect(),
        span: span(source),
    }
}

fn lower_item(item: cst::Item) -> Item {
    match item {
        cst::Item::Namespace(namespace) => Item::Namespace(ast::Namespace {
            name: namespace
                .name_token()
                .map_or_else(missing_name, |token| token.text().to_owned()),
            items: namespace.items().map(lower_item).collect(),
            span: span(&namespace),
        }),
        cst::Item::Use(declaration) => Item::Use(ast::UseDeclaration {
            path: lower_use_path(&declaration),
            span: span(&declaration),
        }),
        cst::Item::Struct(definition) => {
            let definition_span = span(&definition);
            Item::Struct(ast::Struct {
                name: definition
                    .name_token()
                    .map_or_else(missing_name, |token| token.text().to_owned()),
                base: {
                    let path = path_from_tokens(definition.base_tokens());
                    (!path.segments.is_empty()).then_some(path)
                },
                fields: definition
                    .fields()
                    .map(|field| {
                        let field_span = span(&field);
                        ast::Field {
                            ty: field
                                .ty()
                                .map_or_else(|| error_type(field_span), |ty| lower_type(&ty)),
                            name: field
                                .name_token()
                                .map_or_else(missing_name, |token| token.text().to_owned()),
                            span: field_span,
                        }
                    })
                    .collect(),
                functions: definition
                    .functions()
                    .map(|function| lower_function(&function))
                    .collect(),
                constructors: definition
                    .constructors()
                    .map(|constructor| lower_constructor(&constructor))
                    .collect(),
                span: definition_span,
            })
        }
        cst::Item::ConstructorDefinition(constructor) => Item::Constructor(lower_constructor(
            &cst::Constructor::Definition(constructor),
        )),
        cst::Item::ConstructorDeclaration(constructor) => Item::Constructor(lower_constructor(
            &cst::Constructor::Declaration(constructor),
        )),
        cst::Item::FunctionDefinition(function) => {
            Item::Function(lower_function(&cst::Function::Definition(function)))
        }
        cst::Item::FunctionDeclaration(function) => {
            Item::Function(lower_function(&cst::Function::Declaration(function)))
        }
    }
}

fn lower_constructor(constructor: &cst::Constructor) -> ast::Constructor {
    let constructor_span = span(constructor);
    ast::Constructor {
        name: path_from_tokens(constructor.name_tokens()),
        parameters: constructor
            .parameter_list()
            .into_iter()
            .flat_map(|list| list.parameters())
            .map(|parameter| {
                let parameter_span = span(&parameter);
                ast::Parameter {
                    ty: parameter
                        .ty()
                        .map_or_else(|| error_type(parameter_span), |ty| lower_type(&ty)),
                    name: parameter
                        .name_token()
                        .map_or_else(missing_name, |token| token.text().to_owned()),
                    span: parameter_span,
                }
            })
            .collect(),
        throws: constructor
            .throws_clause()
            .into_iter()
            .flat_map(|clause| clause.types())
            .map(|ty| lower_type(&ty))
            .collect(),
        initializers: constructor
            .initializer_list()
            .into_iter()
            .flat_map(|list| list.initializers())
            .map(|initializer| ast::ConstructorInitializer {
                target: path_from_tokens(initializer.name_tokens()),
                arguments: initializer
                    .argument_list()
                    .into_iter()
                    .flat_map(|arguments| arguments.arguments().collect::<Vec<_>>())
                    .map(lower_expression)
                    .collect(),
                span: span(&initializer),
            })
            .collect(),
        body: constructor.body().map(|body| lower_block(&body)),
        is_deleted: constructor.is_deleted(),
        span: constructor_span,
    }
}

fn lower_function(function: &cst::Function) -> ast::Function {
    let function_span = span(function);
    ast::Function {
        name: path_from_tokens(function.name_tokens()),
        return_type: function
            .return_type()
            .map_or_else(|| error_type(function_span), |ty| lower_type(&ty)),
        parameters: function
            .parameter_list()
            .into_iter()
            .flat_map(|list| list.parameters())
            .map(|parameter| {
                let parameter_span = span(&parameter);
                ast::Parameter {
                    ty: parameter
                        .ty()
                        .map_or_else(|| error_type(parameter_span), |ty| lower_type(&ty)),
                    name: parameter
                        .name_token()
                        .map_or_else(missing_name, |token| token.text().to_owned()),
                    span: parameter_span,
                }
            })
            .collect(),
        is_const: function.is_const(),
        throws: function
            .throws_clause()
            .into_iter()
            .flat_map(|clause| clause.types())
            .map(|ty| lower_type(&ty))
            .collect(),
        body: function.body().map(|body| lower_block(&body)),
        span: function_span,
    }
}

fn lower_type(ty: &cst::TypeReference) -> ast::Type {
    let path = path_from_tokens(ty.path_tokens());
    let kind = if ty.is_auto() {
        TypeKind::Inferred
    } else if matches!(path.segments.as_slice(), [name] if name == "function" || name == "function_mut")
    {
        if let Some(signature) = ty.function_signature() {
            let mut types = signature.types();
            let return_type = types
                .next()
                .map_or_else(|| error_type(span(&signature)), |ty| lower_type(&ty));
            TypeKind::Function {
                mutable: path.segments[0] == "function_mut",
                parameters: types.map(|parameter| lower_type(&parameter)).collect(),
                return_type: Box::new(return_type),
            }
        } else {
            TypeKind::Error
        }
    } else if path.segments.is_empty() {
        TypeKind::Error
    } else {
        TypeKind::Named(ast::NamedType {
            path,
            arguments: ty
                .generic_arguments()
                .map(|argument| lower_type(&argument))
                .collect(),
        })
    };

    ast::Type {
        is_const: ty.is_const(),
        is_reference: ty.is_reference(),
        kind,
        span: span(ty),
    }
}

fn lower_block(block: &cst::Block) -> ast::Block {
    ast::Block {
        statements: block.statements().map(lower_statement).collect(),
        span: span(block),
    }
}

fn lower_statement(statement: cst::Statement) -> Statement {
    let statement_span = span(&statement);
    let kind = match statement {
        cst::Statement::Block(block) => StatementKind::Block(lower_block(&block)),
        cst::Statement::Local(local) => StatementKind::Local(lower_local(&local)),
        cst::Statement::Return(return_statement) => {
            StatementKind::Return(return_statement.value().map(lower_expression))
        }
        cst::Statement::Throw(throw_statement) => {
            StatementKind::Throw(throw_statement.value().map(lower_expression))
        }
        cst::Statement::Try(try_statement) => {
            let body = try_statement.body().map_or_else(
                || ast::Block {
                    statements: Vec::new(),
                    span: statement_span,
                },
                |body| lower_block(&body),
            );
            StatementKind::Try(ast::TryStatement {
                body,
                catches: try_statement
                    .catches()
                    .map(|catch| {
                        let catch_span = span(&catch);
                        ast::CatchClause {
                            binding: (!catch.is_catch_all()).then(|| {
                                let ty = catch
                                    .ty()
                                    .map_or_else(|| error_type(catch_span), |ty| lower_type(&ty));
                                ast::CatchBinding {
                                    span: ty.span,
                                    ty,
                                    name: catch
                                        .name_token()
                                        .map_or_else(missing_name, |token| token.text().to_owned()),
                                }
                            }),
                            body: catch.body().map_or_else(
                                || ast::Block {
                                    statements: Vec::new(),
                                    span: catch_span,
                                },
                                |body| lower_block(&body),
                            ),
                            span: catch_span,
                        }
                    })
                    .collect(),
            })
        }
        cst::Statement::If(if_statement) => {
            let condition = if_statement
                .condition()
                .map_or_else(|| error_expression(statement_span), lower_expression);
            let then_branch = if_statement
                .then_branch()
                .map_or_else(|| error_statement(statement_span), lower_statement);
            let else_branch = if_statement
                .else_clause()
                .and_then(|clause| clause.branch())
                .map(lower_statement)
                .map(Box::new);
            StatementKind::If(ast::IfStatement {
                condition,
                then_branch: Box::new(then_branch),
                else_branch,
            })
        }
        cst::Statement::For(for_statement) => {
            let clause = for_statement
                .clause()
                .map_or(ForClause::Error, lower_for_clause);
            let body = for_statement
                .body()
                .map_or_else(|| error_statement(statement_span), lower_statement);
            StatementKind::For(ast::ForStatement {
                clause,
                body: Box::new(body),
            })
        }
        cst::Statement::Break(_) => StatementKind::Break,
        cst::Statement::Continue(_) => StatementKind::Continue,
        cst::Statement::Expression(expression) => expression
            .expression()
            .map_or(StatementKind::Error, |expression| {
                StatementKind::Expression(lower_expression(expression))
            }),
        cst::Statement::Empty(_) => StatementKind::Empty,
        cst::Statement::Error(_) => StatementKind::Error,
    };

    Statement {
        kind,
        span: statement_span,
    }
}

fn lower_local(local: &cst::LocalDeclaration) -> ast::LocalDeclaration {
    let local_span = span(local);
    ast::LocalDeclaration {
        ty: local
            .ty()
            .map_or_else(|| error_type(local_span), |ty| lower_type(&ty)),
        name: local
            .name_token()
            .map_or_else(missing_name, |token| token.text().to_owned()),
        initializer: local.initializer().map(lower_expression),
        span: local_span,
    }
}

fn lower_for_clause(clause: cst::ForClause) -> ForClause {
    match clause {
        cst::ForClause::Range(range) => {
            let range_span = span(&range);
            ForClause::Range(ast::RangeForClause {
                ty: range
                    .ty()
                    .map_or_else(|| error_type(range_span), |ty| lower_type(&ty)),
                name: range
                    .name_token()
                    .map_or_else(missing_name, |token| token.text().to_owned()),
                iterable: range
                    .iterable()
                    .map_or_else(|| error_expression(range_span), lower_expression),
            })
        }
        cst::ForClause::Classic(classic) => {
            let initializer = classic.initializer().map(|initializer| match initializer {
                cst::ClassicForInitializer::Declaration(local) => {
                    ForInitializer::Local(lower_local(&local))
                }
                cst::ClassicForInitializer::Expression(expression) => {
                    ForInitializer::Expression(lower_expression(expression))
                }
            });
            ForClause::Classic(ast::ClassicForClause {
                initializer,
                condition: classic.condition().map(lower_expression),
                update: classic.update().map(lower_expression),
            })
        }
    }
}

#[allow(clippy::too_many_lines)]
fn lower_expression(expression: cst::Expression) -> Expression {
    let expression_span = span(&expression);
    let kind = match expression {
        cst::Expression::Name(name) => ExpressionKind::Name(path_from_tokens(name.path_tokens())),
        cst::Expression::Literal(literal) => {
            lower_literal(&literal).unwrap_or(ExpressionKind::Error)
        }
        cst::Expression::Parenthesized(parenthesized) => parenthesized
            .expression()
            .map_or(ExpressionKind::Error, |inner| {
                ExpressionKind::Parenthesized(Box::new(lower_expression(inner)))
            }),
        cst::Expression::Prefix(prefix) => lower_prefix(&prefix).unwrap_or(ExpressionKind::Error),
        cst::Expression::Postfix(postfix) => {
            lower_postfix(&postfix).unwrap_or(ExpressionKind::Error)
        }
        cst::Expression::Binary(binary) => lower_binary(&binary).unwrap_or(ExpressionKind::Error),
        cst::Expression::Call(call) => {
            let Some(callee) = call.callee() else {
                return error_expression(expression_span);
            };
            ExpressionKind::Call {
                callee: Box::new(lower_expression(callee)),
                arguments: call
                    .argument_list()
                    .into_iter()
                    .flat_map(|arguments| arguments.arguments().collect::<Vec<_>>())
                    .map(lower_expression)
                    .collect(),
            }
        }
        cst::Expression::MacroCall(call) => {
            let Some(callee) = call.callee() else {
                return error_expression(expression_span);
            };
            ExpressionKind::MacroCall {
                callee: path_from_tokens(callee.path_tokens()),
                arguments: call
                    .argument_list()
                    .into_iter()
                    .flat_map(|arguments| arguments.arguments().collect::<Vec<_>>())
                    .map(lower_expression)
                    .collect(),
            }
        }
        cst::Expression::Aggregate(aggregate) => {
            let Some(cst::Expression::Name(name)) = aggregate.ty() else {
                return error_expression(expression_span);
            };
            ExpressionKind::Aggregate {
                ty: path_from_tokens(name.path_tokens()),
                initializers: aggregate
                    .initializer_list()
                    .into_iter()
                    .flat_map(|list| list.initializers().collect::<Vec<_>>())
                    .map(lower_expression)
                    .collect(),
            }
        }
        cst::Expression::Field(field) => {
            let Some(receiver) = field.receiver() else {
                return error_expression(expression_span);
            };
            ExpressionKind::Field {
                receiver: Box::new(lower_expression(receiver)),
                name: path_from_tokens(field.name_tokens()),
            }
        }
        cst::Expression::Index(index) => {
            let mut expressions = index.expressions();
            let Some(receiver) = expressions.next() else {
                return error_expression(expression_span);
            };
            let Some(index) = expressions.next() else {
                return error_expression(expression_span);
            };
            ExpressionKind::Index {
                receiver: Box::new(lower_expression(receiver)),
                index: Box::new(lower_expression(index)),
            }
        }
        cst::Expression::Lambda(lambda) => {
            let captures = lambda
                .capture_list()
                .into_iter()
                .flat_map(|list| list.captures())
                .map(|capture| {
                    let capture_span = span(&capture);
                    let name = capture
                        .name_token()
                        .map_or_else(missing_name, |token| token.text().to_owned());
                    let kind = if capture.is_borrowed() {
                        ast::LambdaCaptureKind::Borrow
                    } else if let Some(initializer) = capture.initializer() {
                        ast::LambdaCaptureKind::Initialize(Box::new(lower_expression(initializer)))
                    } else {
                        ast::LambdaCaptureKind::Copy
                    };
                    ast::LambdaCapture {
                        name,
                        kind,
                        span: capture_span,
                    }
                })
                .collect();
            let parameters = lambda
                .parameter_list()
                .into_iter()
                .flat_map(|list| list.parameters())
                .map(|parameter| {
                    let parameter_span = span(&parameter);
                    ast::Parameter {
                        ty: parameter
                            .ty()
                            .map_or_else(|| error_type(parameter_span), |ty| lower_type(&ty)),
                        name: parameter
                            .name_token()
                            .map_or_else(missing_name, |token| token.text().to_owned()),
                        span: parameter_span,
                    }
                })
                .collect();
            let Some(body) = lambda.body() else {
                return error_expression(expression_span);
            };
            ExpressionKind::Lambda {
                captures,
                parameters,
                is_mutable: lambda.is_mutable(),
                body: lower_block(&body),
            }
        }
        cst::Expression::Error(_) => ExpressionKind::Error,
    };

    Expression {
        kind,
        span: expression_span,
    }
}

fn lower_literal(literal: &cst::LiteralExpression) -> Option<ExpressionKind> {
    let token = literal.literal_token()?;
    let kind = match token.kind() {
        SyntaxKind::Integer => LiteralKind::Integer,
        SyntaxKind::Float => LiteralKind::Float,
        SyntaxKind::String => LiteralKind::String,
        SyntaxKind::Character => LiteralKind::Character,
        SyntaxKind::TrueKw | SyntaxKind::FalseKw => LiteralKind::Boolean,
        _ => return None,
    };
    Some(ExpressionKind::Literal(ast::Literal {
        kind,
        text: token.text().to_owned(),
    }))
}

fn lower_prefix(prefix: &cst::PrefixExpression) -> Option<ExpressionKind> {
    let operator = prefix
        .operator_token()
        .and_then(|token| prefix_operator(token.kind()))?;
    let operand = prefix.operand()?;
    Some(ExpressionKind::Prefix {
        operator,
        operand: Box::new(lower_expression(operand)),
    })
}

fn lower_postfix(postfix: &cst::PostfixExpression) -> Option<ExpressionKind> {
    let operator = postfix
        .operator_token()
        .and_then(|token| postfix_operator(token.kind()))?;
    let operand = postfix.operand()?;
    Some(ExpressionKind::Postfix {
        operator,
        operand: Box::new(lower_expression(operand)),
    })
}

fn lower_binary(binary: &cst::BinaryExpression) -> Option<ExpressionKind> {
    let mut expressions = binary.expressions();
    let left = expressions.next()?;
    let right = expressions.next()?;
    let operator = binary
        .operator_token()
        .and_then(|token| binary_operator(token.kind()))?;
    Some(ExpressionKind::Binary {
        left: Box::new(lower_expression(left)),
        operator,
        right: Box::new(lower_expression(right)),
    })
}

fn prefix_operator(kind: SyntaxKind) -> Option<PrefixOperator> {
    match kind {
        SyntaxKind::Plus => Some(PrefixOperator::Plus),
        SyntaxKind::Minus => Some(PrefixOperator::Negate),
        SyntaxKind::Bang => Some(PrefixOperator::Not),
        SyntaxKind::Tilde => Some(PrefixOperator::BitwiseNot),
        SyntaxKind::PlusPlus => Some(PrefixOperator::Increment),
        SyntaxKind::MinusMinus => Some(PrefixOperator::Decrement),
        _ => None,
    }
}

fn postfix_operator(kind: SyntaxKind) -> Option<PostfixOperator> {
    match kind {
        SyntaxKind::PlusPlus => Some(PostfixOperator::Increment),
        SyntaxKind::MinusMinus => Some(PostfixOperator::Decrement),
        _ => None,
    }
}

fn binary_operator(kind: SyntaxKind) -> Option<BinaryOperator> {
    match kind {
        SyntaxKind::Eq => Some(BinaryOperator::Assign),
        SyntaxKind::PlusEq => Some(BinaryOperator::AddAssign),
        SyntaxKind::MinusEq => Some(BinaryOperator::SubtractAssign),
        SyntaxKind::StarEq => Some(BinaryOperator::MultiplyAssign),
        SyntaxKind::SlashEq => Some(BinaryOperator::DivideAssign),
        SyntaxKind::PercentEq => Some(BinaryOperator::RemainderAssign),
        SyntaxKind::OrOr => Some(BinaryOperator::LogicalOr),
        SyntaxKind::AndAnd => Some(BinaryOperator::LogicalAnd),
        SyntaxKind::Pipe => Some(BinaryOperator::BitwiseOr),
        SyntaxKind::Caret => Some(BinaryOperator::BitwiseXor),
        SyntaxKind::Amp => Some(BinaryOperator::BitwiseAnd),
        SyntaxKind::EqEq => Some(BinaryOperator::Equal),
        SyntaxKind::NotEq => Some(BinaryOperator::NotEqual),
        SyntaxKind::Less => Some(BinaryOperator::Less),
        SyntaxKind::LessEq => Some(BinaryOperator::LessEqual),
        SyntaxKind::Greater => Some(BinaryOperator::Greater),
        SyntaxKind::GreaterEq => Some(BinaryOperator::GreaterEqual),
        SyntaxKind::Plus => Some(BinaryOperator::Add),
        SyntaxKind::Minus => Some(BinaryOperator::Subtract),
        SyntaxKind::Star => Some(BinaryOperator::Multiply),
        SyntaxKind::Slash => Some(BinaryOperator::Divide),
        SyntaxKind::Percent => Some(BinaryOperator::Remainder),
        _ => None,
    }
}

fn path_from_tokens(tokens: impl Iterator<Item = stainless_syntax::SyntaxToken>) -> ast::Path {
    ast::Path {
        segments: tokens
            .filter(|token| matches!(token.kind(), SyntaxKind::Identifier | SyntaxKind::MoveKw))
            .map(|token| token.text().to_owned())
            .collect(),
    }
}

fn lower_use_path(declaration: &cst::UseDeclaration) -> String {
    let mut result = String::new();
    for token in declaration
        .syntax()
        .descendants_with_tokens()
        .filter_map(stainless_syntax::SyntaxElement::into_token)
        .filter(|token| {
            !token.kind().is_trivia()
                && !matches!(token.kind(), SyntaxKind::UseKw | SyntaxKind::Semicolon)
        })
    {
        if token.kind() == SyntaxKind::AsKw {
            result.push_str(" as ");
        } else {
            result.push_str(token.text());
        }
    }
    result
}

fn span(node: &impl AstNode) -> Span {
    Span::from_text_range(node.syntax().text_range())
}

fn error_type(span: Span) -> ast::Type {
    ast::Type {
        is_const: false,
        is_reference: false,
        kind: TypeKind::Error,
        span,
    }
}

fn error_expression(span: Span) -> Expression {
    Expression {
        kind: ExpressionKind::Error,
        span,
    }
}

fn error_statement(span: Span) -> Statement {
    Statement {
        kind: StatementKind::Error,
        span,
    }
}

fn missing_name() -> String {
    "<missing>".to_owned()
}
