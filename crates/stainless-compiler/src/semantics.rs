//! Structural semantic checks that do not yet require name or type resolution.

use std::collections::BTreeMap;

use crate::Diagnostic;
use crate::ast::{
    Block, Constructor, ForClause, ForInitializer, Function, Item, LocalDeclaration, Parameter,
    SourceFile, Statement, StatementKind,
};

/// Validates structural rules over a lowered source file.
///
/// This initial pass deliberately does not claim to resolve names, infer
/// expression types, or check ownership. Resolution and ownership validation
/// are separate passes layered on top of this stable AST.
#[must_use]
pub fn validate(source: &SourceFile) -> Vec<Diagnostic> {
    let mut validator = Validator {
        diagnostics: Vec::new(),
    };
    validator.items(&source.items);
    validator
        .diagnostics
        .sort_by_key(|diagnostic| diagnostic.span);
    validator.diagnostics
}

struct Validator {
    diagnostics: Vec<Diagnostic>,
}

impl Validator {
    fn items(&mut self, items: &[Item]) {
        for item in items {
            match item {
                Item::Namespace(namespace) => self.items(&namespace.items),
                Item::Struct(structure) => {
                    for constructor in &structure.constructors {
                        if constructor.body.is_some() {
                            self.push(
                                "SEM010",
                                "constructors must be defined outside the struct body".to_owned(),
                                constructor.span,
                            );
                        }
                        self.constructor(constructor);
                    }
                    for function in &structure.functions {
                        if function.body.is_some() {
                            self.push(
                                "SEM009",
                                "member functions must be defined outside the struct body"
                                    .to_owned(),
                                function.span,
                            );
                        }
                        self.function(function);
                    }
                }
                Item::Constructor(constructor) => self.constructor(constructor),
                Item::Function(function) => self.function(function),
                Item::Use(_) => {}
            }
        }
    }

    fn function(&mut self, function: &Function) {
        self.parameters(&function.parameters);

        if let Some(body) = &function.body {
            self.block(body, 0, function.return_type.is_void());
        }
    }

    fn constructor(&mut self, constructor: &Constructor) {
        self.parameters(&constructor.parameters);
        if let Some(body) = &constructor.body {
            self.block(body, 0, true);
        }
    }

    fn parameters(&mut self, parameters: &[Parameter]) {
        let mut parameter_names = BTreeMap::new();
        for parameter in parameters {
            if parameter.name == "<missing>" {
                continue;
            }
            if parameter_names
                .insert(parameter.name.as_str(), parameter.span)
                .is_some()
            {
                self.push(
                    "SEM001",
                    format!("duplicate parameter name `{}`", parameter.name),
                    parameter.span,
                );
            }
        }
    }

    fn block(&mut self, block: &Block, loop_depth: u32, returns_void: bool) {
        for statement in &block.statements {
            self.statement(statement, loop_depth, returns_void);
        }
    }

    fn statement(&mut self, statement: &Statement, loop_depth: u32, returns_void: bool) {
        match &statement.kind {
            StatementKind::Block(block) => self.block(block, loop_depth, returns_void),
            StatementKind::Local(local) => self.local(local),
            StatementKind::Return(value) => match (returns_void, value.is_some()) {
                (true, true) => self.push(
                    "SEM005",
                    "a void function cannot return a value".to_owned(),
                    statement.span,
                ),
                (false, false) => self.push(
                    "SEM006",
                    "a non-void function must return a value".to_owned(),
                    statement.span,
                ),
                _ => {}
            },
            StatementKind::If(if_statement) => {
                self.statement(&if_statement.then_branch, loop_depth, returns_void);
                if let Some(else_branch) = &if_statement.else_branch {
                    self.statement(else_branch, loop_depth, returns_void);
                }
            }
            StatementKind::For(for_statement) => {
                if let ForClause::Classic(classic) = &for_statement.clause
                    && let Some(ForInitializer::Local(local)) = &classic.initializer
                {
                    self.local(local);
                }
                self.statement(&for_statement.body, loop_depth + 1, returns_void);
            }
            StatementKind::Break if loop_depth == 0 => self.push(
                "SEM007",
                "`break` is only valid inside a loop".to_owned(),
                statement.span,
            ),
            StatementKind::Continue if loop_depth == 0 => self.push(
                "SEM008",
                "`continue` is only valid inside a loop".to_owned(),
                statement.span,
            ),
            StatementKind::Break
            | StatementKind::Continue
            | StatementKind::Expression(_)
            | StatementKind::Empty
            | StatementKind::Error => {}
        }
    }

    fn local(&mut self, local: &LocalDeclaration) {
        if local.ty.is_inferred() && local.initializer.is_none() {
            self.push(
                "SEM002",
                format!(
                    "inferred local `{}` requires an explicit initializer",
                    local.name
                ),
                local.span,
            );
        }
        if local.ty.is_inferred() && local.ty.is_reference {
            self.push(
                "SEM003",
                format!(
                    "ordinary local `{}` cannot use `auto&`; inferred reference bindings are reserved for range loops",
                    local.name
                ),
                local.ty.span,
            );
        }
        if local.ty.is_reference && local.initializer.is_none() {
            self.push(
                "SEM004",
                format!(
                    "reference local `{}` requires an explicit initializer",
                    local.name
                ),
                local.span,
            );
        }
    }

    fn push(&mut self, code: &'static str, message: String, span: crate::ast::Span) {
        self.diagnostics
            .push(Diagnostic::semantic(code, message, span));
    }
}
