use crate::Diagnostic;
use crate::ast::{self, ExpressionKind, ForClause, Item, StatementKind};
use crate::hir;
use crate::interop::{ArgumentAdaptation, Receiver, RustLowering, TypeRef};
use crate::resolution::{CallTarget, Intrinsic, NativeCall, ResolvedCall, SemanticModel};

pub(crate) fn lower(
    source: &ast::SourceFile,
    semantics: &SemanticModel,
) -> Result<hir::Program, Vec<Diagnostic>> {
    let mut lowerer = Lowerer {
        semantics,
        diagnostics: Vec::new(),
        current_return_type: None,
    };
    let lowered_functions = lowerer.lower_functions(&source.items);
    let mut program = hir::Program {
        functions: Vec::new(),
        modules: Vec::new(),
    };
    for function in lowered_functions {
        let module_path =
            function.source_path[..function.source_path.len().saturating_sub(1)].to_vec();
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
}

#[derive(Clone, Copy)]
enum ExpressionMode {
    Value,
    Reference,
}

impl Lowerer<'_> {
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
                Item::Use(_) => {}
            }
        }
        functions
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

        let return_type = self.lower_type(&symbol.return_type, function.return_type.span)?;
        let previous_return_type = self.current_return_type.replace(symbol.return_type.clone());
        let lowered_body = self.lower_block(body);
        self.current_return_type = previous_return_type;
        Some(hir::Function {
            source_path: symbol.path.clone(),
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
            StatementKind::Expression(expression) => Some(hir::Statement::Expression(
                self.lower_expression(expression, ExpressionMode::Value)?,
            )),
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
        if let TypeRef::Reference { mutable, .. } = expected {
            let actual_is_reference = self
                .semantics
                .expression(expression.span)
                .is_some_and(|resolution| resolution.ty.is_reference());
            let expression = self.lower_expression(
                expression,
                if actual_is_reference {
                    ExpressionMode::Reference
                } else {
                    ExpressionMode::Value
                },
            )?;
            if actual_is_reference {
                Some(expression)
            } else {
                Some(hir::Expression::Borrow {
                    mutable: *mutable,
                    expression: Box::new(expression),
                })
            }
        } else {
            self.lower_expression(expression, ExpressionMode::Value)
        }
    }

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
                hir::Expression::Name(binding_name(&path.segments[0]))
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
            } => hir::Expression::Binary {
                left: Box::new(self.lower_expression(left, ExpressionMode::Value)?),
                operator: *operator,
                right: Box::new(self.lower_expression(right, ExpressionMode::Value)?),
            },
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
            ExpressionKind::Parenthesized(_) => unreachable!("handled above"),
            ExpressionKind::Field { .. } | ExpressionKind::Index { .. } => {
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
                let lowered_arguments = arguments
                    .iter()
                    .zip(&function.parameters)
                    .map(|(argument, parameter)| {
                        self.lower_bound_expression(argument, &parameter.ty)
                    })
                    .collect::<Option<Vec<_>>>()?;
                Some(hir::Expression::FunctionCall {
                    modules: function.path[..function.path.len().saturating_sub(1)]
                        .iter()
                        .map(|name| module_name(name))
                        .collect(),
                    function: function.mangled_name.clone(),
                    arguments: lowered_arguments,
                })
            }
            CallTarget::Native(native) => {
                self.lower_native_call(native, callee, arguments, call.span)
            }
            CallTarget::Intrinsic(Intrinsic::Move) => arguments
                .first()
                .and_then(|argument| self.lower_expression(argument, ExpressionMode::Value)),
            CallTarget::Intrinsic(Intrinsic::PrimitiveCast { target }) => {
                let expression = arguments.first()?;
                Some(hir::Expression::Cast {
                    expression: Box::new(self.lower_expression(expression, ExpressionMode::Value)?),
                    target: self.lower_type(target, call.span)?,
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
            TypeRef::Reference { mutable, target } => hir::Type::Reference {
                mutable: *mutable,
                target: Box::new(self.lower_type(target, span)?),
            },
        };
        Some(lowered)
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
