use std::collections::BTreeMap;

use crate::Diagnostic;
use crate::ast::{self, ExpressionKind, ForClause, Item, StatementKind};
use crate::hir;
use crate::interop::{ArgumentAdaptation, Receiver, RustLowering, TypeRef};
use crate::resolution::{
    CallTarget, CallbackTarget, FunctionSymbol, Intrinsic, LambdaCaptureMode, NativeCall,
    ResolvedCall, ResolvedLambdaCapture, SemanticModel, StructSymbol, ValueCategory,
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
        current_throwing: false,
        exception_target: hir::ExceptionTarget::Function,
        caught_error: None,
        try_index: 0,
        loop_index: 0,
        loop_labels: Vec::new(),
        native_wrappers: BTreeMap::new(),
    };
    let mut lowered_structs = lowerer.lower_structs(&source.items);
    for structure in semantics.structs.iter().filter(|structure| {
        matches!(
            structure.path.as_slice(),
            [namespace, name]
                if namespace == "stainless"
                    && matches!(name.as_str(), "Exception" | "RustError")
        )
    }) {
        if let Some(structure) = lowerer.lower_struct_symbol(
            structure,
            structure.path.last().map_or("<missing>", String::as_str),
            structure.span,
        ) {
            lowered_structs.push(structure);
        }
    }
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
    let native_wrappers = std::mem::take(&mut lowerer.native_wrappers)
        .into_values()
        .collect();
    let mut program = hir::Program {
        native_wrappers,
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
    current_throwing: bool,
    exception_target: hir::ExceptionTarget,
    caught_error: Option<String>,
    try_index: usize,
    loop_index: usize,
    loop_labels: Vec<String>,
    native_wrappers: BTreeMap<String, hir::NativeWrapper>,
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
                    if let Some(lowered) =
                        self.lower_struct_symbol(&symbol, &structure.name, structure.span)
                    {
                        structs.push(lowered);
                    }
                }
                Item::Use(_) | Item::Constructor(_) | Item::Function(_) => {}
            }
        }
        structs
    }

    fn lower_struct_symbol(
        &mut self,
        symbol: &StructSymbol,
        rust_name: &str,
        span: ast::Span,
    ) -> Option<hir::Struct> {
        let mut fields = Vec::new();
        if let Some(base) = symbol.base {
            let base_symbol = self.semantics.structure(base)?;
            fields.push(hir::Field {
                rust_name: base_field_name(base_symbol),
                ty: self.lower_type(
                    &TypeRef::Struct {
                        path: base_symbol.path.clone(),
                    },
                    span,
                )?,
            });
        }
        fields.extend(symbol.fields.iter().filter_map(|field| {
            Some(hir::Field {
                rust_name: field.name.clone(),
                ty: self.lower_type(&field.ty, field.span)?,
            })
        }));
        let is_exception = self.is_exception_structure(symbol.id);
        let exception_base_field = if is_exception {
            symbol
                .base
                .and_then(|base| self.semantics.structure(base))
                .map(base_field_name)
        } else {
            None
        };
        Some(hir::Struct {
            source_path: symbol.path.clone(),
            rust_name: rust_name.to_owned(),
            fields,
            is_exception,
            exception_base_field,
        })
    }

    fn is_exception_structure(&self, structure: crate::resolution::StructId) -> bool {
        let mut current = Some(structure);
        while let Some(id) = current {
            let symbol = &self.semantics.structs[id.0];
            if symbol.path == ["stainless", "Exception"] {
                return true;
            }
            current = symbol.base;
        }
        false
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
        let throwing = !symbol.throws.is_empty();
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
        let previous_throwing = self.current_throwing;
        let previous_target =
            std::mem::replace(&mut self.exception_target, hir::ExceptionTarget::Function);
        let previous_caught = self.caught_error.take();
        self.current_throwing = throwing;
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
            .collect::<Option<Vec<_>>>();
        let lowered_type = self.lower_type(&struct_type, span);
        let (Some(fields), Some(lowered_type)) = (fields, lowered_type) else {
            self.current_throwing = previous_throwing;
            self.exception_target = previous_target;
            self.caught_error = previous_caught;
            return None;
        };
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
            Some(body) => self.lower_block(body),
            None => Some(hir::Block {
                statements: Vec::new(),
            }),
        };
        self.current_return_type = previous_return_type;
        self.current_fluent_receiver = previous_fluent_receiver;
        self.current_constructor = previous_constructor;
        self.current_throwing = previous_throwing;
        self.exception_target = previous_target;
        self.caught_error = previous_caught;
        let lowered_body = lowered_body?;
        statements.extend(lowered_body.statements);
        let completed = hir::Expression::Name("__stainless_constructed".to_owned());
        statements.push(hir::Statement::Return(Some(if throwing {
            hir::Expression::Success(Some(Box::new(completed)))
        } else {
            completed
        })));

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
            throws: throwing,
            body: hir::Block { statements },
            span,
        })
    }

    fn lower_function(&mut self, function: &ast::Function) -> Option<hir::Function> {
        let body = function.body.as_ref()?;
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
        let throwing = !symbol.throws.is_empty();
        let previous_return_type = self.current_return_type.replace(symbol.return_type.clone());
        let previous_fluent_receiver = self.current_fluent_receiver;
        let previous_throwing = self.current_throwing;
        let previous_target =
            std::mem::replace(&mut self.exception_target, hir::ExceptionTarget::Function);
        let previous_caught = self.caught_error.take();
        self.current_throwing = throwing;
        self.current_fluent_receiver = function.return_type.is_void() && symbol.receiver.is_some();
        let mut lowered_body = self.lower_block(body);
        if self.current_fluent_receiver
            && !block_definitely_returns(body)
            && let Some(body) = &mut lowered_body
        {
            let value = hir::Expression::Name("__stainless_self".to_owned());
            body.statements
                .push(hir::Statement::Return(Some(if throwing {
                    hir::Expression::Success(Some(Box::new(value)))
                } else {
                    value
                })));
        } else if throwing
            && function.return_type.is_void()
            && !block_definitely_returns(body)
            && let Some(body) = &mut lowered_body
        {
            body.statements
                .push(hir::Statement::Return(Some(hir::Expression::Success(None))));
        }
        self.current_return_type = previous_return_type;
        self.current_fluent_receiver = previous_fluent_receiver;
        self.current_throwing = previous_throwing;
        self.exception_target = previous_target;
        self.caught_error = previous_caught;
        Some(hir::Function {
            source_path: symbol.path.clone(),
            module_path: function_module_path(symbol, self.semantics),
            rust_name: symbol.mangled_name.clone(),
            parameters,
            return_type,
            throws: throwing,
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
                let mut value = match value {
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
                if self.current_throwing {
                    value = Some(hir::Expression::Success(value.map(Box::new)));
                }
                Some(hir::Statement::Return(value))
            }
            StatementKind::Throw(value) => {
                let value = match value {
                    Some(value) => {
                        let resolution = self.semantics.expression(value.span)?;
                        hir::ExceptionValue::New(
                            self.lower_bound_expression(value, canonical_ref(&resolution.ty))?,
                        )
                    }
                    None => hir::ExceptionValue::Existing(self.caught_error.clone()?),
                };
                Some(hir::Statement::Throw {
                    value,
                    target: self.exception_target.clone(),
                })
            }
            StatementKind::Try(try_statement) => self.lower_try_statement(try_statement),
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
            StatementKind::Break => self.lower_loop_jump(false, statement.span),
            StatementKind::Continue => self.lower_loop_jump(true, statement.span),
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

    fn lower_try_statement(&mut self, try_statement: &ast::TryStatement) -> Option<hir::Statement> {
        let index = self.try_index;
        self.try_index += 1;
        let label = format!("__stainless_try_{index}");
        let error_name = format!("__stainless_error_{index}");
        let unmatched_target =
            if !self.current_throwing && self.exception_target == hir::ExceptionTarget::Function {
                hir::ExceptionTarget::Unreachable
            } else {
                self.exception_target.clone()
            };
        let previous_target = std::mem::replace(
            &mut self.exception_target,
            hir::ExceptionTarget::Try(label.clone()),
        );
        let previous_caught = self.caught_error.take();
        let body = self.lower_block(&try_statement.body);
        self.exception_target = previous_target;
        self.caught_error.clone_from(&previous_caught);
        let body = body?;

        let mut catches = Vec::new();
        for catch in &try_statement.catches {
            let (ty, binding) = if let Some(binding) = &catch.binding {
                let resolution = self.semantics.binding(binding.span)?;
                (
                    Some(self.lower_type(canonical_ref(&resolution.ty), binding.span)?),
                    Some(binding_name(&binding.name)),
                )
            } else {
                (None, None)
            };
            self.caught_error = Some(error_name.clone());
            let body = self.lower_block(&catch.body);
            self.caught_error.clone_from(&previous_caught);
            let body = body?;
            catches.push(hir::Catch { ty, binding, body });
        }

        Some(hir::Statement::Try {
            label,
            error_name,
            body,
            body_falls_through: block_may_fall_through(&try_statement.body),
            catches,
            diverges: block_definitely_returns(&try_statement.body)
                && try_statement
                    .catches
                    .iter()
                    .all(|catch| block_definitely_returns(&catch.body)),
            unmatched_target,
        })
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
        let label = format!("__stainless_loop_{}", self.loop_index);
        self.loop_index += 1;
        match &statement.clause {
            ForClause::Classic(classic) => {
                let initializer = match &classic.initializer {
                    Some(initializer) => Some(Box::new(match initializer {
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
                    })),
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
                let body = self.lower_loop_body(&label, &statement.body)?;
                Some(hir::Statement::ClassicFor {
                    label,
                    initializer,
                    condition,
                    update,
                    body,
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
                let body = self.lower_loop_body(&label, &statement.body)?;
                Some(hir::Statement::RangeFor {
                    label,
                    name: binding_name(&range.name),
                    mutable: binding.mutable && !binding.ty.is_reference(),
                    mode,
                    iterable: self.lower_expression(&range.iterable, ExpressionMode::Reference)?,
                    body,
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

    fn lower_loop_body(&mut self, label: &str, statement: &ast::Statement) -> Option<hir::Block> {
        self.loop_labels.push(label.to_owned());
        let body = self.statement_as_block(statement);
        self.loop_labels.pop();
        body
    }

    fn lower_loop_jump(&mut self, is_continue: bool, span: ast::Span) -> Option<hir::Statement> {
        let Some(label) = self.loop_labels.last().cloned() else {
            self.push(
                "HIR003",
                "loop jump reached HIR lowering without an enclosing loop".to_owned(),
                span,
            );
            return None;
        };
        Some(if is_continue {
            hir::Statement::Continue(label)
        } else {
            hir::Statement::Break(label)
        })
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
        if let Some(adaptation) = self.semantics.rust_result_adaptation(expression.span) {
            return Some(hir::Expression::UnwrapRustResult {
                expression: Box::new(self.lower_expression(expression, ExpressionMode::Value)?),
                error_message: lower_rust_error_message(adaptation.error_message),
                target: self.exception_target.clone(),
            });
        }
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
                if let Some(callback) = self.semantics.callback(expression.span)
                    && let CallbackTarget::Function(id) = callback.target
                {
                    let function = self.semantics.function(id)?;
                    return Some(hir::Expression::FunctionItem {
                        modules: function_module_path(function, self.semantics)
                            .iter()
                            .map(|name| module_name(name))
                            .collect(),
                        function: function.mangled_name.clone(),
                    });
                }
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
            ExpressionKind::Lambda {
                captures,
                parameters,
                is_mutable,
                body,
            } => self.lower_lambda(expression.span, captures, parameters, *is_mutable, body)?,
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

    fn lower_lambda(
        &mut self,
        span: ast::Span,
        syntax_captures: &[ast::LambdaCapture],
        syntax_parameters: &[ast::Parameter],
        is_mutable: bool,
        body: &ast::Block,
    ) -> Option<hir::Expression> {
        let callback = self.semantics.callback(span)?;
        let TypeRef::Callback(callback_type) = &callback.ty else {
            self.push(
                "HIR018",
                "resolved lambda has no callback type".to_owned(),
                span,
            );
            return None;
        };
        let CallbackTarget::Lambda { captures } = &callback.target else {
            self.push(
                "HIR018",
                "lambda expression resolved to a non-lambda callback target".to_owned(),
                span,
            );
            return None;
        };
        let lowered_captures =
            self.lower_lambda_captures(captures, syntax_captures, is_mutable, span)?;
        if syntax_parameters.len() != callback_type.parameters.len() {
            self.push(
                "HIR018",
                "resolved callback parameters do not match lambda syntax".to_owned(),
                span,
            );
            return None;
        }
        let parameters = syntax_parameters
            .iter()
            .zip(&callback_type.parameters)
            .map(|(syntax, ty)| {
                Some(hir::Parameter {
                    source_name: syntax.name.clone(),
                    rust_name: binding_name(&syntax.name),
                    ty: self.lower_type(ty, syntax.ty.span)?,
                    mutable: matches!(ty, TypeRef::Reference { mutable: true, .. })
                        || (!ty.is_reference() && !syntax.ty.is_const),
                })
            })
            .collect::<Option<Vec<_>>>()?;

        let previous_return_type = self
            .current_return_type
            .replace(callback_type.return_type.as_ref().clone());
        let previous_fluent = self.current_fluent_receiver;
        let previous_constructor = self.current_constructor;
        let previous_throwing = self.current_throwing;
        let previous_target = std::mem::replace(
            &mut self.exception_target,
            hir::ExceptionTarget::Unreachable,
        );
        let previous_caught = self.caught_error.take();
        let previous_loops = std::mem::take(&mut self.loop_labels);
        self.current_fluent_receiver = false;
        self.current_constructor = false;
        self.current_throwing = false;
        let lowered_body = self.lower_block(body);
        self.current_return_type = previous_return_type;
        self.current_fluent_receiver = previous_fluent;
        self.current_constructor = previous_constructor;
        self.current_throwing = previous_throwing;
        self.exception_target = previous_target;
        self.caught_error = previous_caught;
        self.loop_labels = previous_loops;

        Some(hir::Expression::Lambda {
            captures: lowered_captures,
            parameters,
            body: lowered_body?,
        })
    }

    fn lower_lambda_captures(
        &mut self,
        captures: &[ResolvedLambdaCapture],
        syntax_captures: &[ast::LambdaCapture],
        is_mutable: bool,
        span: ast::Span,
    ) -> Option<Vec<hir::LambdaCapture>> {
        if syntax_captures.len() != captures.len() {
            self.push(
                "HIR018",
                "resolved callback captures do not match lambda syntax".to_owned(),
                span,
            );
            return None;
        }
        let mut lowered_captures = Vec::with_capacity(captures.len());
        for (capture, syntax) in captures.iter().zip(syntax_captures) {
            if capture.name != syntax.name {
                self.push(
                    "HIR018",
                    "resolved callback capture order does not match lambda syntax".to_owned(),
                    syntax.span,
                );
                return None;
            }
            let initializer = match capture.mode {
                LambdaCaptureMode::Initialize => {
                    let ast::LambdaCaptureKind::Initialize(initializer) = &syntax.kind else {
                        self.push(
                            "HIR018",
                            "resolved initializer capture has no syntax initializer".to_owned(),
                            syntax.span,
                        );
                        return None;
                    };
                    self.lower_bound_expression(initializer, &capture.ty)?
                }
                LambdaCaptureMode::Copy => {
                    let outer = hir::Expression::Name(binding_name(&capture.name));
                    hir::Expression::Clone {
                        expression: Box::new(hir::Expression::Borrow {
                            mutable: false,
                            expression: Box::new(outer),
                        }),
                    }
                }
                LambdaCaptureMode::Borrow { mutable } => {
                    let outer = hir::Expression::Name(binding_name(&capture.name));
                    hir::Expression::Borrow {
                        mutable,
                        expression: Box::new(outer),
                    }
                }
            };
            lowered_captures.push(hir::LambdaCapture {
                rust_name: binding_name(&capture.name),
                mutable: is_mutable && !matches!(capture.mode, LambdaCaptureMode::Borrow { .. }),
                initializer,
            });
        }
        Some(lowered_captures)
    }

    #[allow(clippy::too_many_lines)]
    fn lower_resolved_call(
        &mut self,
        call: &ResolvedCall,
        callee: Option<&ast::Expression>,
        arguments: &[ast::Expression],
    ) -> Option<hir::Expression> {
        let handles_checked_effect = matches!(
            call.target,
            CallTarget::Intrinsic(Intrinsic::UnwrapRustResult { .. })
        );
        let lowered = match &call.target {
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
            CallTarget::Intrinsic(Intrinsic::UnwrapRustResult { error_message }) => {
                let Some(ast::Expression {
                    kind: ExpressionKind::Field { receiver, .. },
                    ..
                }) = callee
                else {
                    self.push(
                        "HIR011",
                        "native Result unwrap has no receiver".to_owned(),
                        call.span,
                    );
                    return None;
                };
                Some(hir::Expression::UnwrapRustResult {
                    expression: Box::new(self.lower_expression(receiver, ExpressionMode::Value)?),
                    error_message: lower_rust_error_message(*error_message),
                    target: self.exception_target.clone(),
                })
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
            CallTarget::Intrinsic(Intrinsic::ExceptionRoot { structure }) => {
                let symbol = self.semantics.structure(*structure)?;
                let message = arguments.first()?;
                let expected = TypeRef::Native {
                    path: "rust::String".to_owned(),
                    arguments: Vec::new(),
                };
                Some(hir::Expression::Aggregate {
                    ty: self.lower_type(
                        &TypeRef::Struct {
                            path: symbol.path.clone(),
                        },
                        call.span,
                    )?,
                    fields: vec![(
                        "message".to_owned(),
                        self.lower_bound_expression(message, &expected)?,
                    )],
                })
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
        }?;
        if call.throws.is_empty() || handles_checked_effect {
            Some(lowered)
        } else {
            Some(hir::Expression::Propagate {
                expression: Box::new(lowered),
                target: self.exception_target.clone(),
            })
        }
    }

    fn lower_native_call(
        &mut self,
        native: &NativeCall,
        callee: Option<&ast::Expression>,
        arguments: &[ast::Expression],
        span: ast::Span,
    ) -> Option<hir::Expression> {
        if let RustLowering::GeneratedWrapper {
            wrapper_name,
            target,
        } = &native.lowering
        {
            return self.lower_generated_wrapper_call(
                native,
                callee,
                arguments,
                span,
                wrapper_name,
                target,
            );
        }
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
                        rust_name: "as_str".to_owned(),
                        receiver_mode: Receiver::Shared,
                        arguments: Vec::new(),
                    },
                })
            })
            .collect::<Option<Vec<_>>>()?;

        match &native.lowering {
            RustLowering::AssociatedFunction { rust_path } => {
                Some(hir::Expression::AssociatedCall {
                    rust_path: rust_path.clone(),
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
                    rust_name: rust_name.clone(),
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
            RustLowering::GeneratedWrapper { .. } => {
                unreachable!("generated wrappers return before direct-call lowering")
            }
        }
    }

    fn lower_generated_wrapper_call(
        &mut self,
        native: &NativeCall,
        callee: Option<&ast::Expression>,
        arguments: &[ast::Expression],
        span: ast::Span,
        wrapper_name: &str,
        target: &crate::interop::WrapperTarget,
    ) -> Option<hir::Expression> {
        let receiver = match (native.receiver, &native.receiver_type) {
            (Some(mode), Some(ty)) => Some(hir::NativeWrapperReceiver {
                ty: self.lower_type(ty, span)?,
                mode,
            }),
            (None, None) => None,
            _ => {
                self.push(
                    "HIR016",
                    "generated wrapper has inconsistent receiver metadata".to_owned(),
                    span,
                );
                return None;
            }
        };
        let parameters = native
            .parameter_types
            .iter()
            .zip(&native.adaptations)
            .map(|(ty, adaptation)| {
                Some(hir::NativeWrapperParameter {
                    ty: self.lower_type(ty, span)?,
                    adaptation: *adaptation,
                })
            })
            .collect::<Option<Vec<_>>>()?;
        let wrapper = hir::NativeWrapper {
            rust_name: wrapper_name.to_owned(),
            target: target.clone(),
            receiver,
            parameters,
            return_type: self.lower_type(&native.return_type, span)?,
        };
        if let Some(previous) = self.native_wrappers.get(wrapper_name) {
            if previous != &wrapper {
                self.push(
                    "HIR017",
                    format!("generated wrapper `{wrapper_name}` has conflicting signatures"),
                    span,
                );
                return None;
            }
        } else {
            self.native_wrappers
                .insert(wrapper_name.to_owned(), wrapper);
        }

        let mut lowered_arguments = Vec::new();
        if let Some(mode) = native.receiver {
            let Some(ast::Expression {
                kind: ExpressionKind::Field { receiver, .. },
                ..
            }) = callee
            else {
                self.push(
                    "HIR011",
                    "generated native method wrapper has no receiver".to_owned(),
                    span,
                );
                return None;
            };
            let receiver_type = native
                .receiver_type
                .as_ref()
                .expect("validated wrapper receiver type");
            let lowered = match mode {
                Receiver::Shared => self.lower_bound_expression(
                    receiver,
                    &TypeRef::shared_ref(receiver_type.clone()),
                )?,
                Receiver::Mutable => self.lower_bound_expression(
                    receiver,
                    &TypeRef::mutable_ref(receiver_type.clone()),
                )?,
                Receiver::Value => self.lower_expression(receiver, ExpressionMode::Value)?,
            };
            lowered_arguments.push(lowered);
        }
        lowered_arguments.extend(
            arguments
                .iter()
                .zip(&native.parameter_types)
                .map(|(argument, expected)| self.lower_bound_expression(argument, expected))
                .collect::<Option<Vec<_>>>()?,
        );
        Some(hir::Expression::WrapperCall {
            rust_name: wrapper_name.to_owned(),
            arguments: lowered_arguments,
        })
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
                let rust_path =
                    native_type_path(path, self.semantics, span, &mut self.diagnostics)?;
                hir::Type::Native {
                    rust_path,
                    arguments: arguments
                        .iter()
                        .map(|argument| self.lower_type(argument, span))
                        .collect::<Option<Vec<_>>>()?,
                }
            }
            TypeRef::Callback(callback) => hir::Type::Callback {
                kind: callback.kind,
                parameters: callback
                    .parameters
                    .iter()
                    .map(|parameter| self.lower_type(parameter, span))
                    .collect::<Option<Vec<_>>>()?,
                return_type: Box::new(self.lower_type(&callback.return_type, span)?),
            },
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
    path: &str,
    semantics: &SemanticModel,
    span: ast::Span,
    diagnostics: &mut Vec<Diagnostic>,
) -> Option<String> {
    match path {
        "rust::Option" => Some("::std::option::Option".to_owned()),
        "rust::Result" => Some("::std::result::Result".to_owned()),
        _ => semantics
            .native_type(path)
            .map(|native| native.rust_path.clone())
            .or_else(|| {
                diagnostics.push(Diagnostic::hir(
                    "HIR014",
                    format!("native type `{path}` has no Rust type lowering"),
                    span,
                ));
                None
            }),
    }
}

fn lower_rust_error_message(message: crate::resolution::RustErrorMessage) -> hir::RustErrorMessage {
    match message {
        crate::resolution::RustErrorMessage::Display => hir::RustErrorMessage::Display,
        crate::resolution::RustErrorMessage::Debug => hir::RustErrorMessage::Debug,
        crate::resolution::RustErrorMessage::Fallback => hir::RustErrorMessage::Fallback,
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

fn block_may_fall_through(block: &ast::Block) -> bool {
    block
        .statements
        .last()
        .is_none_or(statement_may_fall_through)
}

fn statement_may_fall_through(statement: &ast::Statement) -> bool {
    match &statement.kind {
        StatementKind::Return(_)
        | StatementKind::Throw(_)
        | StatementKind::Break
        | StatementKind::Continue => false,
        StatementKind::Block(block) => block_may_fall_through(block),
        StatementKind::If(statement) => {
            statement_may_fall_through(&statement.then_branch)
                || statement
                    .else_branch
                    .as_deref()
                    .is_none_or(statement_may_fall_through)
        }
        StatementKind::Try(statement) => {
            block_may_fall_through(&statement.body)
                || statement
                    .catches
                    .iter()
                    .any(|catch| block_may_fall_through(&catch.body))
        }
        _ => true,
    }
}

fn statement_definitely_returns(statement: &ast::Statement) -> bool {
    match &statement.kind {
        StatementKind::Return(_) | StatementKind::Throw(_) => true,
        StatementKind::Block(block) => block_definitely_returns(block),
        StatementKind::Try(statement) => {
            block_definitely_returns(&statement.body)
                && statement
                    .catches
                    .iter()
                    .all(|catch| block_definitely_returns(&catch.body))
        }
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
