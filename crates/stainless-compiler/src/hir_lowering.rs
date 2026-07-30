use crate::Diagnostic;
use crate::ast::{self, ExpressionKind, ForClause, Item, StatementKind};
use crate::hir;
use crate::interop::{ArgumentAdaptation, Receiver, RustLowering, TypeRef};
use crate::resolution::{
    CallTarget, FunctionSymbol, Intrinsic, NativeCall, ResolvedCall, SemanticModel, StructSymbol,
    ValueCategory,
};

pub(crate) fn lower(
    source: &ast::SourceFile,
    semantics: &SemanticModel,
) -> Result<hir::Program, Vec<Diagnostic>> {
    let mut lowerer = Lowerer {
        semantics,
        diagnostics: Vec::new(),
        current_return_type: None,
        current_fluent_receiver: false,
        current_constructor: false,
    };
    let lowered_structs = lowerer.lower_structs(&source.items);
    let mut lowered_functions = lowerer.lower_functions(&source.items);
    lowered_functions.extend(lowerer.lower_constructors(&source.items));
    for constructor in semantics
        .constructors
        .iter()
        .filter(|constructor| constructor.synthesized && !constructor.is_deleted)
    {
        if let Some(constructor) = lowerer.lower_constructor_symbol(constructor, None) {
            lowered_functions.push(constructor);
        }
    }
    let mut program = hir::Program {
        structs: Vec::new(),
        functions: Vec::new(),
        modules: Vec::new(),
    };
    for structure in lowered_structs {
        let module_path =
            structure.source_path[..structure.source_path.len().saturating_sub(1)].to_vec();
        insert_struct(
            &mut program.structs,
            &mut program.modules,
            &module_path,
            structure,
        );
    }
    for function in lowered_functions {
        let module_path = function.module_path.clone();
        insert_function(
            &mut program.functions,
            &mut program.modules,
            &module_path,
            function,
        );
    }
    if lowerer.diagnostics.is_empty() {
        Ok(program)
    } else {
        Err(lowerer.diagnostics)
    }
}

struct Lowerer<'a> {
    semantics: &'a SemanticModel,
    diagnostics: Vec<Diagnostic>,
    current_return_type: Option<TypeRef>,
    current_fluent_receiver: bool,
    current_constructor: bool,
}

#[derive(Clone, Copy)]
enum ExpressionMode {
    Value,
    Reference,
}

impl Lowerer<'_> {
    fn lower_structs(&mut self, items: &[Item]) -> Vec<hir::Struct> {
        let mut structs = Vec::new();
        for item in items {
            match item {
                Item::Namespace(namespace) => {
                    structs.extend(self.lower_structs(&namespace.items));
                }
                Item::Struct(structure) => {
                    let Some(symbol) = self.semantics.struct_at(structure.span).cloned() else {
                        self.push(
                            "HIR015",
                            format!("resolved struct `{}` is missing", structure.name),
                            structure.span,
                        );
                        continue;
                    };
                    let mut fields = Vec::new();
                    if let Some(base) = symbol.base {
                        let Some(base_symbol) = self.semantics.structure(base) else {
                            self.push(
                                "HIR015",
                                "resolved data base is missing".to_owned(),
                                structure.span,
                            );
                            continue;
                        };
                        fields.push(hir::Field {
                            rust_name: base_field_name(base_symbol),
                            ty: self
                                .lower_type(
                                    &TypeRef::Struct {
                                        path: base_symbol.path.clone(),
                                    },
                                    structure.span,
                                )
                                .expect("resolved base type lowers"),
                        });
                    }
                    fields.extend(symbol.fields.iter().filter_map(|field| {
                        Some(hir::Field {
                            rust_name: field.name.clone(),
                            ty: self.lower_type(&field.ty, field.span)?,
                        })
                    }));
                    structs.push(hir::Struct {
                        source_path: symbol.path.clone(),
                        rust_name: structure.name.clone(),
                        fields,
                    });
                }
                Item::Use(_) | Item::Constructor(_) | Item::Function(_) => {}
            }
        }
        structs
    }

    fn lower_functions(&mut self, items: &[Item]) -> Vec<hir::Function> {
        let mut functions = Vec::new();
        for item in items {
            match item {
                Item::Function(function) => {
                    if let Some(function) = self.lower_function(function) {
                        functions.push(function);
                    }
                }
                Item::Namespace(namespace) => {
                    functions.extend(self.lower_functions(&namespace.items));
                }
                Item::Struct(structure) => {
                    for function in &structure.functions {
                        if let Some(function) = self.lower_function(function) {
                            functions.push(function);
                        }
                    }
                }
                Item::Use(_) | Item::Constructor(_) => {}
            }
        }
        functions
    }

    fn lower_constructors(&mut self, items: &[Item]) -> Vec<hir::Function> {
        let mut constructors = Vec::new();
        for item in items {
            match item {
                Item::Constructor(constructor) if constructor.body.is_some() => {
                    let Some(symbol) = self.semantics.constructor_at(constructor.span).cloned()
                    else {
                        self.push(
                            "HIR016",
                            "resolved constructor symbol is missing".to_owned(),
                            constructor.span,
                        );
                        continue;
                    };
                    if let Some(lowered) = self.lower_constructor_symbol(&symbol, Some(constructor))
                    {
                        constructors.push(lowered);
                    }
                }
                Item::Namespace(namespace) => {
                    constructors.extend(self.lower_constructors(&namespace.items));
                }
                Item::Struct(structure) => {
                    for constructor in &structure.constructors {
                        if constructor.body.is_some()
                            && let Some(symbol) =
                                self.semantics.constructor_at(constructor.span).cloned()
                            && let Some(lowered) =
                                self.lower_constructor_symbol(&symbol, Some(constructor))
                        {
                            constructors.push(lowered);
                        }
                    }
                }
                Item::Constructor(_) | Item::Use(_) | Item::Function(_) => {}
            }
        }
        constructors
    }

    #[allow(clippy::too_many_lines)]
    fn lower_constructor_symbol(
        &mut self,
        symbol: &crate::resolution::ConstructorSymbol,
        syntax: Option<&ast::Constructor>,
    ) -> Option<hir::Function> {
        let structure = self.semantics.structure(symbol.structure)?.clone();
        let span = syntax.map_or(structure.span, |constructor| constructor.span);
        if syntax.is_some_and(|constructor| !constructor.throws.is_empty()) {
            self.push(
                "HIR001",
                "checked-exception lowering is not implemented yet".to_owned(),
                span,
            );
            return None;
        }
        let parameters = match syntax {
            Some(syntax) => {
                if syntax.parameters.len() != symbol.parameters.len() {
                    self.push(
                        "HIR016",
                        "resolved constructor parameters do not match the syntax tree".to_owned(),
                        span,
                    );
                    return None;
                }
                syntax
                    .parameters
                    .iter()
                    .zip(&symbol.parameters)
                    .map(|(syntax, resolved)| {
                        Some(hir::Parameter {
                            source_name: syntax.name.clone(),
                            rust_name: binding_name(&syntax.name),
                            ty: self.lower_type(&resolved.ty, syntax.ty.span)?,
                            mutable: !resolved.ty.is_reference() && !syntax.ty.is_const,
                        })
                    })
                    .collect::<Option<Vec<_>>>()?
            }
            None => Vec::new(),
        };
        let struct_type = TypeRef::Struct {
            path: structure.path.clone(),
        };
        let fields = symbol
            .initializations
            .iter()
            .map(|initialization| {
                let arguments = match initialization.source {
                    Some(source) => syntax?
                        .initializers
                        .iter()
                        .find(|initializer| initializer.span == source)?
                        .arguments
                        .as_slice(),
                    None => &[],
                };
                Some((
                    initialization.rust_name.clone(),
                    self.lower_resolved_call(&initialization.call, None, arguments)?,
                ))
            })
            .collect::<Option<Vec<_>>>()?;
        let lowered_type = self.lower_type(&struct_type, span)?;
        let mut statements = vec![
            hir::Statement::Let {
                name: "__stainless_constructed".to_owned(),
                ty: lowered_type.clone(),
                mutable: true,
                initializer: hir::Expression::Aggregate {
                    ty: lowered_type.clone(),
                    fields,
                },
            },
            hir::Statement::Let {
                name: "__stainless_self".to_owned(),
                ty: hir::Type::Reference {
                    mutable: true,
                    target: Box::new(lowered_type.clone()),
                },
                mutable: false,
                initializer: hir::Expression::Borrow {
                    mutable: true,
                    expression: Box::new(hir::Expression::Name(
                        "__stainless_constructed".to_owned(),
                    )),
                },
            },
        ];

        let previous_return_type = self.current_return_type.replace(struct_type);
        let previous_fluent_receiver = self.current_fluent_receiver;
        let previous_constructor = self.current_constructor;
        self.current_fluent_receiver = false;
        self.current_constructor = true;
        let lowered_body = match syntax.and_then(|constructor| constructor.body.as_ref()) {
            Some(body) => self.lower_block(body)?,
            None => hir::Block {
                statements: Vec::new(),
            },
        };
        self.current_return_type = previous_return_type;
        self.current_fluent_receiver = previous_fluent_receiver;
        self.current_constructor = previous_constructor;
        statements.extend(lowered_body.statements);
        statements.push(hir::Statement::Return(Some(hir::Expression::Name(
            "__stainless_constructed".to_owned(),
        ))));

        let mut source_path = structure.path.clone();
        source_path.push(
            structure
                .path
                .last()
                .cloned()
                .unwrap_or_else(|| "<missing>".to_owned()),
        );
        Some(hir::Function {
            source_path,
            module_path: structure.path[..structure.path.len().saturating_sub(1)].to_vec(),
            rust_name: symbol.mangled_name.clone(),
            parameters,
            return_type: lowered_type,
            body: hir::Block { statements },
            span,
        })
    }

    fn lower_function(&mut self, function: &ast::Function) -> Option<hir::Function> {
        let body = function.body.as_ref()?;
        if !function.throws.is_empty() {
            self.push(
                "HIR001",
                "checked-exception lowering is not implemented yet".to_owned(),
                function.span,
            );
            return None;
        }
        let Some(symbol) = self.semantics.function_at(function.span) else {
            self.push(
                "HIR002",
                "resolved function symbol is missing".to_owned(),
                function.span,
            );
            return None;
        };
        if symbol.parameters.len() != function.parameters.len() {
            self.push(
                "HIR002",
                "resolved function parameters do not match the syntax tree".to_owned(),
                function.span,
            );
            return None;
        }
        let parameters = function
            .parameters
            .iter()
            .zip(&symbol.parameters)
            .map(|(syntax, resolved)| {
                Some(hir::Parameter {
                    source_name: syntax.name.clone(),
                    rust_name: binding_name(&syntax.name),
                    ty: self.lower_type(&resolved.ty, syntax.ty.span)?,
                    mutable: !resolved.ty.is_reference() && !syntax.ty.is_const,
                })
            })
            .collect::<Option<Vec<_>>>()?;
        let mut parameters = parameters;
        if let Some(receiver) = &symbol.receiver {
            let structure = self.semantics.structure(receiver.structure)?;
            parameters.insert(
                0,
                hir::Parameter {
                    source_name: "self".to_owned(),
                    rust_name: "__stainless_self".to_owned(),
                    ty: hir::Type::Reference {
                        mutable: receiver.mutable,
                        target: Box::new(self.lower_type(
                            &TypeRef::Struct {
                                path: structure.path.clone(),
                            },
                            function.span,
                        )?),
                    },
                    mutable: false,
                },
            );
        }

        let return_type = self.lower_type(&symbol.return_type, function.return_type.span)?;
        let previous_return_type = self.current_return_type.replace(symbol.return_type.clone());
        let previous_fluent_receiver = self.current_fluent_receiver;
        self.current_fluent_receiver = function.return_type.is_void() && symbol.receiver.is_some();
        let mut lowered_body = self.lower_block(body);
        if self.current_fluent_receiver
            && !block_definitely_returns(body)
            && let Some(body) = &mut lowered_body
        {
            body.statements
                .push(hir::Statement::Return(Some(hir::Expression::Name(
                    "__stainless_self".to_owned(),
                ))));
        }
        self.current_return_type = previous_return_type;
        self.current_fluent_receiver = previous_fluent_receiver;
        Some(hir::Function {
            source_path: symbol.path.clone(),
            module_path: function_module_path(symbol, self.semantics),
            rust_name: symbol.mangled_name.clone(),
            parameters,
            return_type,
            body: lowered_body?,
            span: function.span,
        })
    }

    fn lower_block(&mut self, block: &ast::Block) -> Option<hir::Block> {
        let mut statements = Vec::new();
        for statement in &block.statements {
            if matches!(statement.kind, StatementKind::Empty) {
                continue;
            }
            statements.push(self.lower_statement(statement)?);
        }
        Some(hir::Block { statements })
    }

    fn lower_statement(&mut self, statement: &ast::Statement) -> Option<hir::Statement> {
        match &statement.kind {
            StatementKind::Block(block) => Some(hir::Statement::Block(self.lower_block(block)?)),
            StatementKind::Local(local) => {
                self.lower_local(local)
                    .map(|(name, ty, mutable, initializer)| hir::Statement::Let {
                        name,
                        ty,
                        mutable,
                        initializer,
                    })
            }
            StatementKind::Return(value) => {
                let value = match value {
                    Some(value) => {
                        let Some(return_type) = self.current_return_type.clone() else {
                            self.push(
                                "HIR002",
                                "function return type is missing during HIR lowering".to_owned(),
                                statement.span,
                            );
                            return None;
                        };
                        Some(self.lower_bound_expression(value, &return_type)?)
                    }
                    None if self.current_constructor => {
                        Some(hir::Expression::Name("__stainless_constructed".to_owned()))
                    }
                    None if self.current_fluent_receiver => {
                        Some(hir::Expression::Name("__stainless_self".to_owned()))
                    }
                    None => None,
                };
                Some(hir::Statement::Return(value))
            }
            StatementKind::If(if_statement) => {
                let condition =
                    self.lower_expression(&if_statement.condition, ExpressionMode::Value)?;
                let then_branch = self.statement_as_block(&if_statement.then_branch)?;
                let else_branch = match &if_statement.else_branch {
                    Some(branch) => Some(Box::new(self.lower_statement(branch)?)),
                    None => None,
                };
                Some(hir::Statement::If {
                    condition,
                    then_branch,
                    else_branch,
                })
            }
            StatementKind::For(for_statement) => self.lower_for(for_statement),
            StatementKind::Break => Some(hir::Statement::Break),
            StatementKind::Continue => Some(hir::Statement::Continue),
            StatementKind::Expression(expression) => {
                let mode = if self
                    .semantics
                    .expression(expression.span)
                    .is_some_and(|resolution| resolution.ty.is_reference())
                {
                    ExpressionMode::Reference
                } else {
                    ExpressionMode::Value
                };
                Some(hir::Statement::Expression(
                    self.lower_expression(expression, mode)?,
                ))
            }
            StatementKind::Empty => None,
            StatementKind::Error => {
                self.push(
                    "HIR003",
                    "cannot lower a recovered statement error".to_owned(),
                    statement.span,
                );
                None
            }
        }
    }

    fn lower_local(
        &mut self,
        local: &ast::LocalDeclaration,
    ) -> Option<(String, hir::Type, bool, hir::Expression)> {
        let Some(binding) = self.semantics.binding(local.span) else {
            self.push(
                "HIR004",
                format!("resolved local binding `{}` is missing", local.name),
                local.span,
            );
            return None;
        };
        let initializer = if let Some(initializer) = &local.initializer {
            self.lower_bound_expression(initializer, &binding.ty)?
        } else {
            let Some(call) = self.semantics.call(local.span) else {
                self.push(
                    "HIR005",
                    format!(
                        "implicit default constructor for `{}` is missing",
                        local.name
                    ),
                    local.span,
                );
                return None;
            };
            self.lower_resolved_call(call, None, &[])?
        };
        Some((
            binding_name(&local.name),
            self.lower_type(&binding.ty, local.ty.span)?,
            binding.mutable && !binding.ty.is_reference(),
            initializer,
        ))
    }

    fn lower_for(&mut self, statement: &ast::ForStatement) -> Option<hir::Statement> {
        match &statement.clause {
            ForClause::Classic(classic) => {
                let initializer = match &classic.initializer {
                    Some(initializer) => Some(match initializer {
                        ast::ForInitializer::Local(local) => {
                            let (name, ty, mutable, initializer) = self.lower_local(local)?;
                            hir::ForInitializer::Let {
                                name,
                                ty,
                                mutable,
                                initializer,
                            }
                        }
                        ast::ForInitializer::Expression(expression) => {
                            hir::ForInitializer::Expression(
                                self.lower_expression(expression, ExpressionMode::Value)?,
                            )
                        }
                    }),
                    None => None,
                };
                let condition = match &classic.condition {
                    Some(expression) => {
                        Some(self.lower_expression(expression, ExpressionMode::Value)?)
                    }
                    None => None,
                };
                let update = match &classic.update {
                    Some(expression) => {
                        Some(self.lower_expression(expression, ExpressionMode::Value)?)
                    }
                    None => None,
                };
                Some(hir::Statement::ClassicFor {
                    initializer,
                    condition,
                    update,
                    body: self.statement_as_block(&statement.body)?,
                })
            }
            ForClause::Range(range) => {
                let Some(binding) = self.semantics.binding(range.ty.span) else {
                    self.push(
                        "HIR006",
                        format!("resolved range binding `{}` is missing", range.name),
                        range.ty.span,
                    );
                    return None;
                };
                let mode = match &binding.ty {
                    TypeRef::Reference { mutable: true, .. } => hir::RangeMode::Mutable,
                    TypeRef::Reference { mutable: false, .. } => hir::RangeMode::Shared,
                    _ if is_move_call(self.semantics, &range.iterable) => hir::RangeMode::Move,
                    TypeRef::Struct { .. } => hir::RangeMode::Clone,
                    _ => hir::RangeMode::Copy,
                };
                Some(hir::Statement::RangeFor {
                    name: binding_name(&range.name),
                    mutable: binding.mutable && !binding.ty.is_reference(),
                    mode,
                    iterable: self.lower_expression(&range.iterable, ExpressionMode::Reference)?,
                    body: self.statement_as_block(&statement.body)?,
                })
            }
            ForClause::Error => {
                self.push(
                    "HIR003",
                    "cannot lower a recovered loop error".to_owned(),
                    statement.body.span,
                );
                None
            }
        }
    }

    fn statement_as_block(&mut self, statement: &ast::Statement) -> Option<hir::Block> {
        if let StatementKind::Block(block) = &statement.kind {
            self.lower_block(block)
        } else {
            self.lower_statement(statement).map(|statement| hir::Block {
                statements: vec![statement],
            })
        }
    }

    fn lower_bound_expression(
        &mut self,
        expression: &ast::Expression,
        expected: &TypeRef,
    ) -> Option<hir::Expression> {
        let resolution = self.semantics.expression(expression.span);
        if let TypeRef::Reference {
            mutable,
            target: expected_target,
        } = expected
        {
            let actual_is_reference = resolution.is_some_and(|value| value.ty.is_reference());
            let mut lowered = self.lower_expression(
                expression,
                if actual_is_reference {
                    ExpressionMode::Reference
                } else {
                    ExpressionMode::Value
                },
            )?;
            let actual_target = resolution.map(|value| canonical_ref(&value.ty));
            let projection = match (actual_target, canonical_ref(expected_target)) {
                (Some(TypeRef::Struct { path: derived }), TypeRef::Struct { path: base }) => {
                    self.struct_projection(derived, base).unwrap_or_default()
                }
                _ => Vec::new(),
            };
            let projected = !projection.is_empty();
            if projected {
                lowered = hir::Expression::Field {
                    receiver: Box::new(lowered),
                    access_path: projection,
                };
            }
            if actual_is_reference && !projected {
                Some(lowered)
            } else {
                Some(hir::Expression::Borrow {
                    mutable: *mutable,
                    expression: Box::new(lowered),
                })
            }
        } else {
            let lowered = self.lower_expression(expression, ExpressionMode::Value)?;
            if matches!(canonical_ref(expected), TypeRef::Struct { .. })
                && resolution.is_some_and(|value| value.category != ValueCategory::Temporary)
                && !is_move_call(self.semantics, expression)
            {
                Some(hir::Expression::Clone {
                    expression: Box::new(hir::Expression::Borrow {
                        mutable: false,
                        expression: Box::new(lowered),
                    }),
                })
            } else {
                Some(lowered)
            }
        }
    }

    #[allow(clippy::too_many_lines)]
    fn lower_expression(
        &mut self,
        expression: &ast::Expression,
        mode: ExpressionMode,
    ) -> Option<hir::Expression> {
        if let ExpressionKind::Parenthesized(inner) = &expression.kind {
            return Some(hir::Expression::Parenthesized(Box::new(
                self.lower_expression(inner, mode)?,
            )));
        }

        let lowered = match &expression.kind {
            ExpressionKind::Name(path) => {
                if path.segments.len() != 1 {
                    self.push(
                        "HIR007",
                        format!(
                            "unresolved qualified value `{}` reached HIR lowering",
                            path.display()
                        ),
                        expression.span,
                    );
                    return None;
                }
                if let Some(field) = self
                    .semantics
                    .expression(expression.span)
                    .and_then(|resolution| resolution.field.as_ref())
                {
                    hir::Expression::Field {
                        receiver: Box::new(hir::Expression::Dereference(Box::new(
                            hir::Expression::Name("__stainless_self".to_owned()),
                        ))),
                        access_path: field.access_path.clone(),
                    }
                } else {
                    hir::Expression::Name(binding_name(&path.segments[0]))
                }
            }
            ExpressionKind::Literal(literal) => hir::Expression::Literal {
                kind: literal.kind,
                text: literal.text.clone(),
            },
            ExpressionKind::Prefix { operator, operand } => match operator {
                ast::PrefixOperator::Increment | ast::PrefixOperator::Decrement => {
                    hir::Expression::Increment {
                        place: Box::new(self.lower_expression(operand, ExpressionMode::Value)?),
                        increment: *operator == ast::PrefixOperator::Increment,
                        prefix: true,
                    }
                }
                _ => hir::Expression::Prefix {
                    operator: *operator,
                    operand: Box::new(self.lower_expression(operand, ExpressionMode::Value)?),
                },
            },
            ExpressionKind::Postfix { operator, operand } => hir::Expression::Increment {
                place: Box::new(self.lower_expression(operand, ExpressionMode::Value)?),
                increment: *operator == ast::PostfixOperator::Increment,
                prefix: false,
            },
            ExpressionKind::Binary {
                left,
                operator,
                right,
            } => {
                let right = if *operator == ast::BinaryOperator::Assign {
                    if let Some(resolution) = self.semantics.expression(left.span) {
                        self.lower_bound_expression(right, canonical_ref(&resolution.ty))?
                    } else {
                        self.lower_expression(right, ExpressionMode::Value)?
                    }
                } else {
                    self.lower_expression(right, ExpressionMode::Value)?
                };
                hir::Expression::Binary {
                    left: Box::new(self.lower_expression(left, ExpressionMode::Value)?),
                    operator: *operator,
                    right: Box::new(right),
                }
            }
            ExpressionKind::Call { callee, arguments } => {
                let Some(call) = self
                    .semantics
                    .expression(expression.span)
                    .and_then(|resolution| resolution.call.as_ref())
                else {
                    self.push(
                        "HIR008",
                        "resolved call target is missing".to_owned(),
                        expression.span,
                    );
                    return None;
                };
                self.lower_resolved_call(call, Some(callee), arguments)?
            }
            ExpressionKind::Aggregate { initializers, .. } => {
                let Some(call) = self
                    .semantics
                    .expression(expression.span)
                    .and_then(|resolution| resolution.call.as_ref())
                else {
                    self.push(
                        "HIR008",
                        "resolved aggregate constructor is missing".to_owned(),
                        expression.span,
                    );
                    return None;
                };
                self.lower_resolved_call(call, None, initializers)?
            }
            ExpressionKind::Parenthesized(_) => unreachable!("handled above"),
            ExpressionKind::Field { receiver, .. } => {
                let Some(field) = self
                    .semantics
                    .expression(expression.span)
                    .and_then(|resolution| resolution.field.as_ref())
                else {
                    self.push(
                        "HIR009",
                        "resolved struct field is missing".to_owned(),
                        expression.span,
                    );
                    return None;
                };
                hir::Expression::Field {
                    receiver: Box::new(self.lower_expression(receiver, ExpressionMode::Value)?),
                    access_path: field.access_path.clone(),
                }
            }
            ExpressionKind::Index { .. } => {
                self.push(
                    "HIR009",
                    "unresolved field or index expression reached HIR lowering".to_owned(),
                    expression.span,
                );
                return None;
            }
            ExpressionKind::Error => {
                self.push(
                    "HIR003",
                    "cannot lower a recovered expression error".to_owned(),
                    expression.span,
                );
                return None;
            }
        };

        let is_reference = self
            .semantics
            .expression(expression.span)
            .is_some_and(|resolution| resolution.ty.is_reference());
        if is_reference && matches!(mode, ExpressionMode::Value) {
            Some(hir::Expression::Dereference(Box::new(lowered)))
        } else {
            Some(lowered)
        }
    }

    #[allow(clippy::too_many_lines)]
    fn lower_resolved_call(
        &mut self,
        call: &ResolvedCall,
        callee: Option<&ast::Expression>,
        arguments: &[ast::Expression],
    ) -> Option<hir::Expression> {
        match &call.target {
            CallTarget::Stainless(id) => {
                let Some(function) = self.semantics.function(*id) else {
                    self.push(
                        "HIR010",
                        "resolved Stainless call points to a missing function".to_owned(),
                        call.span,
                    );
                    return None;
                };
                let mut lowered_arguments = arguments
                    .iter()
                    .zip(&function.parameters)
                    .map(|(argument, parameter)| {
                        self.lower_bound_expression(argument, &parameter.ty)
                    })
                    .collect::<Option<Vec<_>>>()?;
                if let Some(receiver) = &function.receiver {
                    let Some(ast::Expression {
                        kind:
                            ExpressionKind::Field {
                                receiver: syntax_receiver,
                                ..
                            },
                        ..
                    }) = callee
                    else {
                        self.push(
                            "HIR011",
                            "member call has no receiver".to_owned(),
                            call.span,
                        );
                        return None;
                    };
                    let structure = self.semantics.structure(receiver.structure)?;
                    lowered_arguments.insert(
                        0,
                        self.lower_bound_expression(
                            syntax_receiver,
                            &TypeRef::Reference {
                                mutable: receiver.mutable,
                                target: Box::new(TypeRef::Struct {
                                    path: structure.path.clone(),
                                }),
                            },
                        )?,
                    );
                }
                Some(hir::Expression::FunctionCall {
                    modules: function_module_path(function, self.semantics)
                        .iter()
                        .map(|name| module_name(name))
                        .collect(),
                    function: function.mangled_name.clone(),
                    arguments: lowered_arguments,
                })
            }
            CallTarget::Constructor(id) => {
                let Some(constructor) = self.semantics.constructor(*id) else {
                    self.push(
                        "HIR016",
                        "resolved constructor call points to a missing constructor".to_owned(),
                        call.span,
                    );
                    return None;
                };
                let structure = self.semantics.structure(constructor.structure)?;
                let lowered_arguments = arguments
                    .iter()
                    .zip(&constructor.parameters)
                    .map(|(argument, parameter)| {
                        self.lower_bound_expression(argument, &parameter.ty)
                    })
                    .collect::<Option<Vec<_>>>()?;
                Some(hir::Expression::FunctionCall {
                    modules: structure.path[..structure.path.len().saturating_sub(1)]
                        .iter()
                        .map(|name| module_name(name))
                        .collect(),
                    function: constructor.mangled_name.clone(),
                    arguments: lowered_arguments,
                })
            }
            CallTarget::Native(native) => {
                self.lower_native_call(native, callee, arguments, call.span)
            }
            CallTarget::Intrinsic(Intrinsic::Move) => {
                let argument = arguments.first()?;
                Some(hir::Expression::Move(Box::new(
                    self.lower_expression(argument, ExpressionMode::Value)?,
                )))
            }
            CallTarget::Intrinsic(Intrinsic::PrimitiveCast { target }) => {
                let expression = arguments.first()?;
                Some(hir::Expression::Cast {
                    expression: Box::new(self.lower_expression(expression, ExpressionMode::Value)?),
                    target: self.lower_type(target, call.span)?,
                })
            }
            CallTarget::Intrinsic(Intrinsic::ValueInitialization { target }) => {
                let expression = arguments.first()?;
                self.lower_bound_expression(expression, target)
            }
            CallTarget::Intrinsic(Intrinsic::StructAggregate { structure }) => {
                let symbol = self.semantics.structure(*structure)?;
                let mut expected = Vec::new();
                let mut names = Vec::new();
                if let Some(base) = symbol.base {
                    let base = self.semantics.structure(base)?;
                    expected.push(TypeRef::Struct {
                        path: base.path.clone(),
                    });
                    names.push(base_field_name(base));
                }
                expected.extend(symbol.fields.iter().map(|field| field.ty.clone()));
                names.extend(symbol.fields.iter().map(|field| field.name.clone()));
                let fields = names
                    .into_iter()
                    .zip(arguments)
                    .zip(&expected)
                    .map(|((name, argument), expected)| {
                        Some((name, self.lower_bound_expression(argument, expected)?))
                    })
                    .collect::<Option<Vec<_>>>()?;
                Some(hir::Expression::Aggregate {
                    ty: self.lower_type(
                        &TypeRef::Struct {
                            path: symbol.path.clone(),
                        },
                        call.span,
                    )?,
                    fields,
                })
            }
        }
    }

    fn lower_native_call(
        &mut self,
        native: &NativeCall,
        callee: Option<&ast::Expression>,
        arguments: &[ast::Expression],
        span: ast::Span,
    ) -> Option<hir::Expression> {
        let lowered_arguments = arguments
            .iter()
            .zip(&native.parameter_types)
            .zip(&native.adaptations)
            .map(|((argument, expected), adaptation)| {
                let lowered = self.lower_bound_expression(argument, expected)?;
                Some(match adaptation {
                    ArgumentAdaptation::Identity => lowered,
                    ArgumentAdaptation::StringRefToStr => hir::Expression::MethodCall {
                        receiver: Box::new(lowered),
                        rust_name: "as_str",
                        receiver_mode: Receiver::Shared,
                        arguments: Vec::new(),
                    },
                })
            })
            .collect::<Option<Vec<_>>>()?;

        match &native.lowering {
            RustLowering::AssociatedFunction { rust_path } => {
                Some(hir::Expression::AssociatedCall {
                    rust_path,
                    arguments: lowered_arguments,
                })
            }
            RustLowering::Method { rust_name } => {
                let Some(ast::Expression {
                    kind: ExpressionKind::Field { receiver, .. },
                    ..
                }) = callee
                else {
                    self.push(
                        "HIR011",
                        "native method call has no receiver".to_owned(),
                        span,
                    );
                    return None;
                };
                let receiver_mode = native.receiver.unwrap_or(Receiver::Shared);
                let expression_mode = match receiver_mode {
                    Receiver::Shared | Receiver::Mutable => ExpressionMode::Reference,
                    Receiver::Value => ExpressionMode::Value,
                };
                Some(hir::Expression::MethodCall {
                    receiver: Box::new(self.lower_expression(receiver, expression_mode)?),
                    rust_name,
                    receiver_mode,
                    arguments: lowered_arguments,
                })
            }
            RustLowering::CloneArgument { index } => {
                let Some(argument) = lowered_arguments.get(*index) else {
                    self.push(
                        "HIR012",
                        "native clone lowering refers to a missing argument".to_owned(),
                        span,
                    );
                    return None;
                };
                Some(hir::Expression::Clone {
                    expression: Box::new(argument.clone()),
                })
            }
        }
    }

    fn lower_type(&mut self, ty: &TypeRef, span: ast::Span) -> Option<hir::Type> {
        let lowered = match ty {
            TypeRef::Error | TypeRef::Parameter(_) => {
                self.push(
                    "HIR013",
                    "unresolved type reached HIR lowering".to_owned(),
                    span,
                );
                return None;
            }
            TypeRef::Void => hir::Type::Unit,
            TypeRef::Bool => hir::Type::Primitive("bool"),
            TypeRef::Char => hir::Type::Primitive("char"),
            TypeRef::I8 => hir::Type::Primitive("i8"),
            TypeRef::I16 => hir::Type::Primitive("i16"),
            TypeRef::I32 => hir::Type::Primitive("i32"),
            TypeRef::I64 => hir::Type::Primitive("i64"),
            TypeRef::I128 => hir::Type::Primitive("i128"),
            TypeRef::Isize => hir::Type::Primitive("isize"),
            TypeRef::U8 => hir::Type::Primitive("u8"),
            TypeRef::U16 => hir::Type::Primitive("u16"),
            TypeRef::U32 => hir::Type::Primitive("u32"),
            TypeRef::U64 => hir::Type::Primitive("u64"),
            TypeRef::U128 => hir::Type::Primitive("u128"),
            TypeRef::Usize => hir::Type::Primitive("usize"),
            TypeRef::F32 => hir::Type::Primitive("f32"),
            TypeRef::F64 => hir::Type::Primitive("f64"),
            TypeRef::Native { path, arguments } => {
                let rust_path = native_type_path(path, span, &mut self.diagnostics)?;
                hir::Type::Native {
                    rust_path,
                    arguments: arguments
                        .iter()
                        .map(|argument| self.lower_type(argument, span))
                        .collect::<Option<Vec<_>>>()?,
                }
            }
            TypeRef::Struct { path } => hir::Type::User {
                rust_path: user_type_path(path),
            },
            TypeRef::Reference { mutable, target } => hir::Type::Reference {
                mutable: *mutable,
                target: Box::new(self.lower_type(target, span)?),
            },
        };
        Some(lowered)
    }

    fn struct_projection(&self, derived: &[String], base: &[String]) -> Option<Vec<String>> {
        if derived == base {
            return Some(Vec::new());
        }
        let mut current = self
            .semantics
            .structs
            .iter()
            .find(|structure| structure.path == derived)?;
        let mut fields = Vec::new();
        loop {
            let parent = self.semantics.structure(current.base?)?;
            fields.push(base_field_name(parent));
            if parent.path == base {
                return Some(fields);
            }
            current = parent;
        }
    }

    fn push(&mut self, code: &'static str, message: String, span: ast::Span) {
        self.diagnostics.push(Diagnostic::hir(code, message, span));
    }
}

fn native_type_path(
    path: &'static str,
    span: ast::Span,
    diagnostics: &mut Vec<Diagnostic>,
) -> Option<&'static str> {
    match path {
        "rust::String" => Some("::std::string::String"),
        "rust::Vec" => Some("::std::vec::Vec"),
        _ => {
            diagnostics.push(Diagnostic::hir(
                "HIR014",
                format!("native type `{path}` has no Rust type lowering"),
                span,
            ));
            None
        }
    }
}

fn binding_name(source_name: &str) -> String {
    format!("__stainless_local_{source_name}")
}

fn module_name(source_name: &str) -> String {
    format!("__stainless_namespace_{source_name}")
}

fn base_field_name(structure: &StructSymbol) -> String {
    format!(
        "__stainless_base_{}",
        structure.path.last().map_or("missing", String::as_str)
    )
}

fn canonical_ref(ty: &TypeRef) -> &TypeRef {
    match ty {
        TypeRef::Reference { target, .. } => target,
        _ => ty,
    }
}

fn user_type_path(source_path: &[String]) -> String {
    let Some((name, namespaces)) = source_path.split_last() else {
        return "crate::__stainless_missing_type".to_owned();
    };
    let mut path = String::from("crate");
    for namespace in namespaces {
        path.push_str("::");
        path.push_str(&module_name(namespace));
    }
    path.push_str("::");
    path.push_str(name);
    path
}

fn function_module_path(function: &FunctionSymbol, semantics: &SemanticModel) -> Vec<String> {
    if let Some(receiver) = &function.receiver
        && let Some(structure) = semantics.structure(receiver.structure)
    {
        return structure.path[..structure.path.len().saturating_sub(1)].to_vec();
    }
    function.path[..function.path.len().saturating_sub(1)].to_vec()
}

fn insert_struct(
    structs: &mut Vec<hir::Struct>,
    modules: &mut Vec<hir::Module>,
    module_path: &[String],
    structure: hir::Struct,
) {
    let Some((source_module, remaining_path)) = module_path.split_first() else {
        structs.push(structure);
        return;
    };
    let index = modules
        .iter()
        .position(|module| module.source_name == *source_module)
        .unwrap_or_else(|| {
            modules.push(hir::Module {
                source_name: source_module.clone(),
                rust_name: module_name(source_module),
                structs: Vec::new(),
                functions: Vec::new(),
                modules: Vec::new(),
            });
            modules.len() - 1
        });
    let module = &mut modules[index];
    insert_struct(
        &mut module.structs,
        &mut module.modules,
        remaining_path,
        structure,
    );
}

fn insert_function(
    functions: &mut Vec<hir::Function>,
    modules: &mut Vec<hir::Module>,
    module_path: &[String],
    function: hir::Function,
) {
    let Some((source_module, remaining_path)) = module_path.split_first() else {
        functions.push(function);
        return;
    };
    let index = modules
        .iter()
        .position(|module| module.source_name == *source_module)
        .unwrap_or_else(|| {
            modules.push(hir::Module {
                source_name: source_module.clone(),
                rust_name: module_name(source_module),
                structs: Vec::new(),
                functions: Vec::new(),
                modules: Vec::new(),
            });
            modules.len() - 1
        });
    let module = &mut modules[index];
    insert_function(
        &mut module.functions,
        &mut module.modules,
        remaining_path,
        function,
    );
}

fn is_move_call(semantics: &SemanticModel, expression: &ast::Expression) -> bool {
    semantics
        .expression(expression.span)
        .and_then(|resolution| resolution.call.as_ref())
        .is_some_and(|call| matches!(&call.target, CallTarget::Intrinsic(Intrinsic::Move)))
}

fn block_definitely_returns(block: &ast::Block) -> bool {
    block
        .statements
        .last()
        .is_some_and(statement_definitely_returns)
}

fn statement_definitely_returns(statement: &ast::Statement) -> bool {
    match &statement.kind {
        StatementKind::Return(_) => true,
        StatementKind::Block(block) => block_definitely_returns(block),
        StatementKind::If(statement) => {
            statement_definitely_returns(&statement.then_branch)
                && statement
                    .else_branch
                    .as_deref()
                    .is_some_and(statement_definitely_returns)
        }
        _ => false,
    }
}
