use std::collections::{BTreeMap, BTreeSet};

use crate::Diagnostic;
use crate::ast::{self, ExpressionKind, ForClause, Item, StatementKind};
use crate::hir;
use crate::interop::{
    ArgumentAdaptation, CallbackKind, PointerKind, Receiver, RustLowering, StoredFunctionKind,
    TypeRef, VAR_TYPE_PATH,
};
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
    let lowered_interfaces = lowerer.lower_interfaces();
    let mut lowered_structs = lowerer.lower_structs(&source.items);
    for structure in semantics.structs.iter().filter(|structure| {
        matches!(
            structure.path.as_slice(),
            [namespace, name]
                if namespace == "stainless"
                    && matches!(
                        name.as_str(),
                        "Exception"
                            | "RustError"
                            | "IoError"
                            | "FormatError"
                            | "JsonError"
                            | "ThreadError"
                    )
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
        interfaces: Vec::new(),
        structs: Vec::new(),
        functions: Vec::new(),
        modules: Vec::new(),
    };
    for interface in lowered_interfaces {
        let module_path =
            interface.source_path[..interface.source_path.len().saturating_sub(1)].to_vec();
        insert_interface(
            &mut program.interfaces,
            &mut program.modules,
            &module_path,
            interface,
        );
    }
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
    fn lower_interfaces(&mut self) -> Vec<hir::Interface> {
        self.semantics
            .structs
            .iter()
            .filter(|structure| structure.kind == ast::UserTypeKind::Interface)
            .cloned()
            .filter_map(|interface| {
                let functions = self
                    .semantics
                    .functions
                    .iter()
                    .filter(|function| {
                        function
                            .receiver
                            .as_ref()
                            .is_some_and(|receiver| receiver.structure == interface.id)
                    })
                    .cloned()
                    .collect::<Vec<_>>();
                let methods = functions
                    .iter()
                    .map(|function| self.lower_interface_method(function))
                    .collect::<Option<Vec<_>>>()?;
                Some(hir::Interface {
                    source_path: interface.path.clone(),
                    rust_name: interface.path.last()?.clone(),
                    bases: interface
                        .interfaces
                        .iter()
                        .filter_map(|base| self.semantics.structure(*base))
                        .map(|base| user_type_path(&base.path))
                        .collect(),
                    methods,
                })
            })
            .collect()
    }

    fn lower_interface_method(
        &mut self,
        function: &FunctionSymbol,
    ) -> Option<hir::InterfaceMethod> {
        let receiver = function.receiver.as_ref()?;
        Some(hir::InterfaceMethod {
            rust_name: function.mangled_name.clone(),
            mutable: receiver.mutable,
            parameters: function
                .parameters
                .iter()
                .map(|parameter| {
                    Some(hir::Parameter {
                        source_name: parameter.name.clone(),
                        rust_name: binding_name(&parameter.name),
                        ty: self.lower_type(&parameter.ty, parameter.span)?,
                        mutable: false,
                    })
                })
                .collect::<Option<Vec<_>>>()?,
            return_type: self.lower_type(&function.return_type, function.declarations[0])?,
            throws: !function.throws.is_empty(),
        })
    }

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
                    if symbol.kind == ast::UserTypeKind::Interface {
                        continue;
                    }
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
        let interface_implementations = self
            .semantics
            .interface_implementations
            .iter()
            .filter(|implementation| implementation.implementer == symbol.id)
            .map(|implementation| {
                let interface = self.semantics.structure(implementation.interface)?;
                let methods = implementation
                    .methods
                    .iter()
                    .map(|(requirement, implementation)| {
                        let requirement = self.semantics.function(*requirement)?;
                        let implementation = self.semantics.function(*implementation)?;
                        Some(hir::InterfaceImplementationMethod {
                            method: self.lower_interface_method(requirement)?,
                            function_modules: function_module_path(implementation, self.semantics)
                                .iter()
                                .map(|name| module_name(name))
                                .collect(),
                            function: implementation.mangled_name.clone(),
                            adapt_self_reference: requirement.return_type
                                != implementation.return_type,
                        })
                    })
                    .collect::<Option<Vec<_>>>()?;
                Some(hir::InterfaceImplementation {
                    interface_path: user_type_path(&interface.path),
                    methods,
                })
            })
            .collect::<Option<Vec<_>>>()?;
        Some(hir::Struct {
            source_path: symbol.path.clone(),
            rust_name: rust_name.to_owned(),
            copyable: symbol.kind == ast::UserTypeKind::Struct,
            fields,
            json_fields: self.lower_json_struct_fields(symbol),
            is_exception,
            exception_base_field,
            interface_implementations,
        })
    }

    fn lower_json_struct_fields(&self, symbol: &StructSymbol) -> Option<Vec<hir::JsonStructField>> {
        if !self.semantics.json_struct_conversions.contains(&symbol.id)
            || symbol.kind != ast::UserTypeKind::Struct
            || !self.json_structure_supported(symbol.id, &mut BTreeSet::new())
        {
            return None;
        }
        let mut fields = Vec::new();
        self.collect_json_struct_fields(symbol.id, &mut Vec::new(), &mut fields)?;
        Some(fields)
    }

    fn json_structure_supported(
        &self,
        structure: crate::resolution::StructId,
        visiting: &mut BTreeSet<crate::resolution::StructId>,
    ) -> bool {
        if !visiting.insert(structure) {
            return false;
        }
        let result = (|| {
            let mut hierarchy = Vec::new();
            let mut current = Some(structure);
            let mut hierarchy_seen = BTreeSet::new();
            while let Some(id) = current {
                if !hierarchy_seen.insert(id) {
                    return false;
                }
                hierarchy.push(id);
                let Some(symbol) = self.semantics.structure(id) else {
                    return false;
                };
                current = symbol.base;
            }
            let mut names = BTreeSet::new();
            hierarchy.into_iter().rev().all(|id| {
                self.semantics.structure(id).is_some_and(|owner| {
                    owner.fields.iter().all(|field| {
                        names.insert(field.name.as_str())
                            && self.json_type_supported(&field.ty, visiting)
                    })
                })
            })
        })();
        visiting.remove(&structure);
        result
    }

    fn json_type_supported(
        &self,
        ty: &TypeRef,
        visiting: &mut BTreeSet<crate::resolution::StructId>,
    ) -> bool {
        let ty = canonical_ref(ty);
        if is_json_scalar_type(ty) {
            return true;
        }
        match ty {
            TypeRef::Native { path, arguments } => match (path.as_str(), arguments.as_slice()) {
                (
                    "rust::Vec" | "rust::List" | "rust::Queue" | "rust::Set" | "rust::Option",
                    [element],
                ) => self.json_type_supported(element, visiting),
                ("rust::Map", [key, value]) => {
                    is_rust_string(key) && self.json_type_supported(value, visiting)
                }
                _ => false,
            },
            TypeRef::Struct { path } => self
                .semantics
                .structs
                .iter()
                .find(|structure| structure.path == *path)
                .is_some_and(|structure| self.json_structure_supported(structure.id, visiting)),
            _ => false,
        }
    }

    fn collect_json_struct_fields(
        &self,
        structure: crate::resolution::StructId,
        prefix: &mut Vec<String>,
        output: &mut Vec<hir::JsonStructField>,
    ) -> Option<()> {
        let symbol = self.semantics.structure(structure)?;
        if let Some(base) = symbol.base {
            let base_symbol = self.semantics.structure(base)?;
            prefix.push(base_field_name(base_symbol));
            self.collect_json_struct_fields(base, prefix, output)?;
            prefix.pop();
        }
        output.extend(symbol.fields.iter().map(|field| {
            let mut access_path = prefix.clone();
            access_path.push(field.name.clone());
            hir::JsonStructField {
                name: field.name.clone(),
                access_path,
            }
        }));
        Some(())
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
        let struct_type = match structure.kind {
            ast::UserTypeKind::Struct => TypeRef::Struct {
                path: structure.path.clone(),
            },
            ast::UserTypeKind::Class => TypeRef::Class {
                path: structure.path.clone(),
            },
            ast::UserTypeKind::Interface => {
                self.push(
                    "HIR016",
                    "an interface reached constructor lowering".to_owned(),
                    span,
                );
                return None;
            }
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
            is_async: false,
            parameters,
            return_type: lowered_type,
            throws: throwing,
            body: hir::Block { statements },
            span,
        })
    }

    #[allow(clippy::too_many_lines)]
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
                            &match structure.kind {
                                ast::UserTypeKind::Struct => TypeRef::Struct {
                                    path: structure.path.clone(),
                                },
                                ast::UserTypeKind::Class => TypeRef::Class {
                                    path: structure.path.clone(),
                                },
                                ast::UserTypeKind::Interface => TypeRef::Interface {
                                    path: structure.path.clone(),
                                },
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
            is_async: symbol.is_async,
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
                let condition = self.lower_condition(&if_statement.condition)?;
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
                    Some(expression) => Some(self.lower_condition(expression)?),
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
                let bindings = range
                    .bindings
                    .iter()
                    .map(|syntax| {
                        let binding = self.semantics.binding(syntax.span).or_else(|| {
                            self.push(
                                "HIR006",
                                format!("resolved range binding `{}` is missing", syntax.name),
                                syntax.span,
                            );
                            None
                        })?;
                        Some((
                            binding.ty.clone(),
                            hir::RangeBinding {
                                name: binding_name(&syntax.name),
                                mutable: binding.mutable && !binding.ty.is_reference(),
                            },
                        ))
                    })
                    .collect::<Option<Vec<_>>>()?;
                let structured = bindings.len() == 2;
                let mode = if bindings
                    .iter()
                    .any(|(ty, _)| matches!(ty, TypeRef::Reference { mutable: true, .. }))
                {
                    hir::RangeMode::Mutable
                } else if bindings.iter().all(|(ty, _)| ty.is_reference()) {
                    hir::RangeMode::Shared
                } else if is_move_call(self.semantics, &range.iterable) {
                    hir::RangeMode::Move
                } else if structured {
                    hir::RangeMode::MapClone
                } else if matches!(bindings[0].0, TypeRef::Struct { .. }) {
                    hir::RangeMode::Clone
                } else {
                    hir::RangeMode::Copy
                };
                let body = self.lower_loop_body(&label, &statement.body)?;
                Some(hir::Statement::RangeFor {
                    label,
                    bindings: bindings.into_iter().map(|(_, binding)| binding).collect(),
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

    fn lower_condition(&mut self, expression: &ast::Expression) -> Option<hir::Expression> {
        let lowered = self.lower_expression(expression, ExpressionMode::Value)?;
        let kind = self
            .semantics
            .expression(expression.span)
            .and_then(|resolution| nullable_test_kind(canonical_ref(&resolution.ty)));
        Some(if let Some(kind) = kind {
            hir::Expression::PointerHasValue {
                kind,
                value: Box::new(lowered),
            }
        } else {
            lowered
        })
    }

    #[allow(clippy::too_many_lines)]
    fn lower_bound_expression(
        &mut self,
        expression: &ast::Expression,
        expected: &TypeRef,
    ) -> Option<hir::Expression> {
        if is_nullptr_literal(expression)
            && let Some(kind) = nullable_test_kind(canonical_ref(expected))
            && kind != PointerKind::Weak
        {
            return Some(hir::Expression::PointerDefault(kind));
        }
        if let Some(adaptation) = self.semantics.rust_result_adaptation(expression.span) {
            return Some(hir::Expression::UnwrapRustResult {
                expression: Box::new(self.lower_expression(expression, ExpressionMode::Value)?),
                exception: lower_native_result_exception(adaptation.exception),
                error_message: lower_rust_error_message(adaptation.error_message),
                target: self.exception_target.clone(),
            });
        }
        let resolution = self.semantics.expression(expression.span);
        if is_json_type(canonical_ref(expected))
            && resolution.is_some_and(|value| !is_json_type(canonical_ref(&value.ty)))
        {
            return self.lower_json_value(expression);
        }
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
            if let Some(kind) = actual_target.and_then(reference_owner_kind) {
                lowered = hir::Expression::PointerPointee {
                    kind,
                    mutable: *mutable
                        && matches!(kind, PointerKind::Unique | PointerKind::UniqueNullable),
                    owner: Box::new(lowered),
                };
            }
            let projection_target = actual_target.map(automatic_pointee_type);
            let projection = match (projection_target, canonical_ref(expected_target)) {
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
            if resolution.is_some_and(|value| is_shared_to_weak_binding(expected, &value.ty)) {
                return Some(hir::Expression::DowngradeShared(Box::new(
                    self.lower_expression(expression, ExpressionMode::Reference)?,
                )));
            }
            let lowered = self.lower_expression(expression, ExpressionMode::Value)?;
            let interface_owner = resolution.and_then(|value| {
                let TypeRef::Pointer {
                    kind: expected_kind,
                    target: expected_target,
                } = canonical_ref(expected)
                else {
                    return None;
                };
                let TypeRef::Pointer {
                    kind: actual_kind,
                    target: actual_target,
                } = canonical_ref(&value.ty)
                else {
                    return None;
                };
                if expected_kind != actual_kind
                    || !matches!(canonical_ref(expected_target), TypeRef::Interface { .. })
                    || !matches!(canonical_ref(actual_target), TypeRef::Class { .. })
                {
                    return None;
                }
                Some((*expected_kind, canonical_ref(expected_target).clone()))
            });
            if let Some((kind, target)) = interface_owner {
                let value = if matches!(kind, PointerKind::Shared | PointerKind::SharedNullable)
                    && resolution.is_some_and(|value| value.category != ValueCategory::Temporary)
                    && !is_move_call(self.semantics, expression)
                {
                    hir::Expression::Clone {
                        expression: Box::new(hir::Expression::Borrow {
                            mutable: false,
                            expression: Box::new(lowered),
                        }),
                    }
                } else {
                    lowered
                };
                return Some(hir::Expression::InterfaceOwnerCoercion {
                    kind,
                    target: self.lower_type(&target, expression.span)?,
                    value: Box::new(value),
                });
            }
            if (matches!(canonical_ref(expected), TypeRef::Struct { .. })
                || matches!(
                    canonical_ref(expected),
                    TypeRef::Function(function)
                        if function.kind == StoredFunctionKind::Shared
                )
                || matches!(
                    canonical_ref(expected),
                    TypeRef::Pointer {
                        kind: PointerKind::Shared | PointerKind::SharedNullable | PointerKind::Weak,
                        ..
                    }
                )
                || is_json_type(canonical_ref(expected)))
                && resolution.is_some_and(|value| value.category != ValueCategory::Temporary)
                && !is_move_call(self.semantics, expression)
                && !is_json_access(expression, self.semantics)
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
                    let item = hir::Expression::FunctionItem {
                        modules: function_module_path(function, self.semantics)
                            .iter()
                            .map(|name| module_name(name))
                            .collect(),
                        function: function.mangled_name.clone(),
                    };
                    return self.store_function_if_needed(item, &callback.ty, expression.span);
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
            ExpressionKind::GenericName { path, .. } => {
                self.push(
                    "HIR007",
                    format!(
                        "generic call target `{}` reached HIR lowering as a value",
                        path.display()
                    ),
                    expression.span,
                );
                return None;
            }
            ExpressionKind::Literal(literal) if literal.kind == ast::LiteralKind::Null => {
                hir::Expression::JsonNull
            }
            ExpressionKind::Literal(literal) => hir::Expression::Literal {
                kind: literal.kind,
                text: literal.text.clone(),
            },
            ExpressionKind::JsonArray { elements } => hir::Expression::JsonArray(
                elements
                    .iter()
                    .map(|element| self.lower_json_value(element))
                    .collect::<Option<Vec<_>>>()?,
            ),
            ExpressionKind::JsonObject { members } => hir::Expression::JsonObject(
                members
                    .iter()
                    .map(|(name, value)| Some((name.clone(), self.lower_json_value(value)?)))
                    .collect::<Option<Vec<_>>>()?,
            ),
            ExpressionKind::Prefix { operator, operand } => match operator {
                ast::PrefixOperator::Increment | ast::PrefixOperator::Decrement => {
                    hir::Expression::Increment {
                        place: Box::new(self.lower_expression(operand, ExpressionMode::Value)?),
                        increment: *operator == ast::PrefixOperator::Increment,
                        prefix: true,
                    }
                }
                ast::PrefixOperator::Not
                    if self
                        .semantics
                        .expression(operand.span)
                        .and_then(|resolution| nullable_test_kind(canonical_ref(&resolution.ty)))
                        .is_some() =>
                {
                    hir::Expression::Prefix {
                        operator: *operator,
                        operand: Box::new(self.lower_condition(operand)?),
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
                if *operator == ast::BinaryOperator::Assign
                    && is_json_mutation_place(left, self.semantics)
                {
                    let json_type = TypeRef::native(VAR_TYPE_PATH, Vec::new());
                    let value = Box::new(self.lower_bound_expression(right, &json_type)?);
                    let mutation = match &left.kind {
                        ExpressionKind::Field { receiver, name } => hir::Expression::JsonSetField {
                            receiver: Box::new(
                                self.lower_expression(receiver, ExpressionMode::Reference)?,
                            ),
                            name: name.segments.first()?.clone(),
                            value,
                        },
                        ExpressionKind::Index { receiver, index } => {
                            hir::Expression::JsonSetIndex {
                                receiver: Box::new(
                                    self.lower_expression(receiver, ExpressionMode::Reference)?,
                                ),
                                index: Box::new(
                                    self.lower_expression(index, ExpressionMode::Value)?,
                                ),
                                value,
                            }
                        }
                        _ => unreachable!("JSON assignment shape was checked"),
                    };
                    hir::Expression::UnwrapRustResult {
                        expression: Box::new(mutation),
                        exception: hir::NativeExceptionKind::JsonError,
                        error_message: hir::RustErrorMessage::Display,
                        target: self.exception_target.clone(),
                    }
                } else if matches!(
                    operator,
                    ast::BinaryOperator::Equal | ast::BinaryOperator::NotEqual
                ) && (is_null_literal(left) || is_null_literal(right))
                    && {
                        let pointer = if is_null_literal(left) { right } else { left };
                        self.semantics
                            .expression(pointer.span)
                            .and_then(|resolution| {
                                nullable_test_kind(canonical_ref(&resolution.ty))
                            })
                            .is_some()
                    }
                {
                    let pointer = if is_null_literal(left) { right } else { left };
                    let tested = self.lower_condition(pointer)?;
                    if *operator == ast::BinaryOperator::Equal {
                        hir::Expression::Prefix {
                            operator: ast::PrefixOperator::Not,
                            operand: Box::new(tested),
                        }
                    } else {
                        tested
                    }
                } else {
                    let logical = matches!(
                        operator,
                        ast::BinaryOperator::LogicalAnd | ast::BinaryOperator::LogicalOr
                    );
                    let lowered_left = if logical {
                        self.lower_condition(left)?
                    } else {
                        self.lower_expression(left, ExpressionMode::Value)?
                    };
                    let right = if *operator == ast::BinaryOperator::Assign {
                        if let Some(resolution) = self.semantics.expression(left.span) {
                            self.lower_bound_expression(right, canonical_ref(&resolution.ty))?
                        } else {
                            self.lower_expression(right, ExpressionMode::Value)?
                        }
                    } else if logical {
                        self.lower_condition(right)?
                    } else {
                        self.lower_expression(right, ExpressionMode::Value)?
                    };
                    hir::Expression::Binary {
                        left: Box::new(lowered_left),
                        operator: *operator,
                        right: Box::new(right),
                    }
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
            ExpressionKind::MacroCall { callee, arguments } => {
                let macro_name = callee.segments.last().map_or("", String::as_str);
                let (kind, destination_index, format_index, format_required) = match macro_name {
                    "println" => (hir::FormatMacroKind::Println, None, 0, false),
                    "eprintln" => (hir::FormatMacroKind::Eprintln, None, 0, false),
                    "format" => (hir::FormatMacroKind::Format, None, 0, true),
                    "write" => (hir::FormatMacroKind::Write, Some(0), 1, true),
                    "writeln" => (hir::FormatMacroKind::Writeln, Some(0), 1, false),
                    _ => {
                        self.push(
                            "HIR020",
                            format!("resolved formatting macro `{macro_name}!` is unsupported"),
                            expression.span,
                        );
                        return None;
                    }
                };
                let destination = match destination_index.and_then(|index| arguments.get(index)) {
                    Some(destination) => Some(Box::new(
                        self.lower_expression(destination, ExpressionMode::Value)?,
                    )),
                    None => None,
                };
                if destination_index.is_some() && destination.is_none() {
                    self.push(
                        "HIR020",
                        format!("resolved `{macro_name}!` has no destination"),
                        expression.span,
                    );
                    return None;
                }
                let format = match arguments.get(format_index) {
                    Some(ast::Expression {
                        kind:
                            ExpressionKind::Literal(ast::Literal {
                                kind: ast::LiteralKind::String,
                                text,
                            }),
                        ..
                    }) => Some(text.clone()),
                    None if !format_required => None,
                    None => {
                        self.push(
                            "HIR020",
                            format!("resolved `{macro_name}!` has no format string"),
                            expression.span,
                        );
                        return None;
                    }
                    Some(_) => {
                        self.push(
                            "HIR020",
                            format!("resolved `{macro_name}!` has no literal format string"),
                            expression.span,
                        );
                        return None;
                    }
                };
                let macro_expression = hir::Expression::FormatMacro {
                    kind,
                    destination,
                    format,
                    arguments: arguments
                        .iter()
                        .skip(format_index + 1)
                        .map(|argument| self.lower_expression(argument, ExpressionMode::Reference))
                        .collect::<Option<Vec<_>>>()?,
                };
                if matches!(
                    kind,
                    hir::FormatMacroKind::Write | hir::FormatMacroKind::Writeln
                ) {
                    hir::Expression::UnwrapRustResult {
                        expression: Box::new(macro_expression),
                        exception: hir::NativeExceptionKind::FormatError,
                        error_message: hir::RustErrorMessage::Display,
                        target: self.exception_target.clone(),
                    }
                } else {
                    macro_expression
                }
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
                let field = self
                    .semantics
                    .expression(expression.span)
                    .and_then(|resolution| resolution.field.as_ref());
                if let Some(field) = field {
                    let mut lowered_receiver =
                        self.lower_expression(receiver, ExpressionMode::Value)?;
                    if let Some(kind) = self
                        .semantics
                        .expression(receiver.span)
                        .and_then(|resolution| nullable_owner_kind(canonical_ref(&resolution.ty)))
                    {
                        let mutable =
                            self.semantics
                                .expression(expression.span)
                                .is_some_and(|resolution| {
                                    resolution.category == ValueCategory::MutablePlace
                                });
                        lowered_receiver = hir::Expression::PointerPointee {
                            kind,
                            mutable: mutable && kind == PointerKind::UniqueNullable,
                            owner: Box::new(lowered_receiver),
                        };
                    }
                    hir::Expression::Field {
                        receiver: Box::new(lowered_receiver),
                        access_path: field.access_path.clone(),
                    }
                } else if self
                    .semantics
                    .expression(receiver.span)
                    .is_some_and(|resolution| is_json_type(canonical_ref(&resolution.ty)))
                {
                    let ExpressionKind::Field { name, .. } = &expression.kind else {
                        unreachable!("field branch has a field expression");
                    };
                    hir::Expression::JsonField {
                        receiver: Box::new(
                            self.lower_expression(receiver, ExpressionMode::Reference)?,
                        ),
                        name: name.segments.first()?.clone(),
                    }
                } else {
                    self.push(
                        "HIR009",
                        "resolved struct field is missing".to_owned(),
                        expression.span,
                    );
                    return None;
                }
            }
            ExpressionKind::Index { receiver, index } => hir::Expression::JsonIndex {
                receiver: Box::new(self.lower_expression(receiver, ExpressionMode::Reference)?),
                index: Box::new(self.lower_expression(index, ExpressionMode::Value)?),
            },
            ExpressionKind::Lambda {
                captures,
                parameters,
                is_mutable,
                is_async,
                body,
            } => self.lower_lambda(
                expression.span,
                captures,
                parameters,
                *is_mutable,
                *is_async,
                body,
            )?,
            ExpressionKind::Await(operand) => {
                self.lower_expression(operand, ExpressionMode::Value)?
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

    fn lower_lambda(
        &mut self,
        span: ast::Span,
        syntax_captures: &[ast::LambdaCapture],
        syntax_parameters: &[ast::Parameter],
        is_mutable: bool,
        is_async: bool,
        body: &ast::Block,
    ) -> Option<hir::Expression> {
        let callback = self.semantics.callback(span)?;
        let (callback_parameters, callback_return) = match &callback.ty {
            TypeRef::Callback(callback_type) => (
                callback_type.parameters.as_slice(),
                callback_type.return_type.as_ref(),
            ),
            TypeRef::Function(function_type) => (
                function_type.parameters.as_slice(),
                function_type.return_type.as_ref(),
            ),
            _ => {
                self.push(
                    "HIR018",
                    "resolved lambda has no callable type".to_owned(),
                    span,
                );
                return None;
            }
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
        if syntax_parameters.len() != callback_parameters.len() {
            self.push(
                "HIR018",
                "resolved callback parameters do not match lambda syntax".to_owned(),
                span,
            );
            return None;
        }
        let parameters = syntax_parameters
            .iter()
            .zip(callback_parameters)
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

        let previous_return_type = self.current_return_type.replace(callback_return.clone());
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

        let lambda = hir::Expression::Lambda {
            captures: lowered_captures,
            is_async,
            repeatable: matches!(
                &callback.ty,
                TypeRef::Callback(callback) if callback.kind == CallbackKind::Fn
            ),
            parameters,
            body: lowered_body?,
        };
        self.store_function_if_needed(lambda, &callback.ty, span)
    }

    fn store_function_if_needed(
        &mut self,
        callable: hir::Expression,
        ty: &TypeRef,
        span: ast::Span,
    ) -> Option<hir::Expression> {
        let TypeRef::Function(function) = ty else {
            return Some(callable);
        };
        Some(hir::Expression::StoreFunction {
            kind: function.kind,
            ty: self.lower_type(ty, span)?,
            callable: Box::new(callable),
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
            CallTarget::Intrinsic(
                Intrinsic::UnwrapRustResult { .. }
                    | Intrinsic::MakeOwner { .. }
                    | Intrinsic::MutexNew { .. }
            )
        ) || matches!(
            &call.target,
            CallTarget::Native(native) if native.result_adaptation.is_some()
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
                                target: Box::new(match structure.kind {
                                    ast::UserTypeKind::Struct => TypeRef::Struct {
                                        path: structure.path.clone(),
                                    },
                                    ast::UserTypeKind::Class => TypeRef::Class {
                                        path: structure.path.clone(),
                                    },
                                    ast::UserTypeKind::Interface => TypeRef::Interface {
                                        path: structure.path.clone(),
                                    },
                                }),
                            },
                        )?,
                    );
                }
                let invocation = hir::Expression::FunctionCall {
                    modules: function_module_path(function, self.semantics)
                        .iter()
                        .map(|name| module_name(name))
                        .collect(),
                    function: function.mangled_name.clone(),
                    arguments: lowered_arguments,
                };
                Some(if function.is_async {
                    hir::Expression::Await(Box::new(invocation))
                } else {
                    invocation
                })
            }
            CallTarget::InterfaceMethod(id) => {
                let function = self.semantics.function(*id)?;
                let receiver = function.receiver.as_ref()?;
                let interface = self.semantics.structure(receiver.structure)?;
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
                        "interface call has no receiver".to_owned(),
                        call.span,
                    );
                    return None;
                };
                let lowered_receiver = self.lower_bound_expression(
                    syntax_receiver,
                    &TypeRef::Reference {
                        mutable: receiver.mutable,
                        target: Box::new(TypeRef::Interface {
                            path: interface.path.clone(),
                        }),
                    },
                )?;
                let lowered_arguments = arguments
                    .iter()
                    .zip(&function.parameters)
                    .map(|(argument, parameter)| {
                        self.lower_bound_expression(argument, &parameter.ty)
                    })
                    .collect::<Option<Vec<_>>>()?;
                let invocation = hir::Expression::InterfaceCall {
                    receiver: Box::new(lowered_receiver),
                    method: function.mangled_name.clone(),
                    arguments: lowered_arguments,
                };
                Some(if function.is_async {
                    hir::Expression::Await(Box::new(invocation))
                } else {
                    invocation
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
                let native_call = self.lower_native_call(native, callee, arguments, call.span)?;
                let native_call = if native.is_async {
                    hir::Expression::Await(Box::new(native_call))
                } else {
                    native_call
                };
                if let Some(adaptation) = native.result_adaptation {
                    Some(hir::Expression::UnwrapRustResult {
                        expression: Box::new(native_call),
                        exception: lower_native_result_exception(adaptation.exception),
                        error_message: lower_rust_error_message(adaptation.error_message),
                        target: self.exception_target.clone(),
                    })
                } else {
                    Some(native_call)
                }
            }
            CallTarget::Intrinsic(Intrinsic::Move) => {
                let argument = arguments.first()?;
                Some(hir::Expression::Move(Box::new(
                    self.lower_expression(argument, ExpressionMode::Value)?,
                )))
            }
            CallTarget::Intrinsic(Intrinsic::MakeOwner {
                kind, construction, ..
            }) => {
                let value = self.lower_resolved_call(construction, None, arguments)?;
                Some(hir::Expression::MakeOwner {
                    kind: *kind,
                    value: Box::new(value),
                })
            }
            CallTarget::Intrinsic(Intrinsic::PointerDefault { kind, .. }) => {
                Some(hir::Expression::PointerDefault(*kind))
            }
            CallTarget::Intrinsic(Intrinsic::PointerConversion {
                from, to, target, ..
            }) => {
                let argument = arguments.first()?;
                Some(hir::Expression::PointerConversion {
                    from: *from,
                    to: *to,
                    value: Box::new(self.lower_bound_expression(
                        argument,
                        &TypeRef::pointer(*from, target.clone()),
                    )?),
                })
            }
            CallTarget::Intrinsic(Intrinsic::DowngradeShared { .. }) => {
                let Some(ast::Expression {
                    kind: ast::ExpressionKind::Field { receiver, .. },
                    ..
                }) = callee
                else {
                    self.push(
                        "HIR011",
                        "shared pointer downgrade has no receiver".to_owned(),
                        call.span,
                    );
                    return None;
                };
                Some(hir::Expression::DowngradeShared(Box::new(
                    self.lower_expression(receiver, ExpressionMode::Reference)?,
                )))
            }
            CallTarget::Intrinsic(Intrinsic::LockWeak { .. }) => {
                let Some(ast::Expression {
                    kind: ast::ExpressionKind::Field { receiver, .. },
                    ..
                }) = callee
                else {
                    self.push(
                        "HIR011",
                        "weak pointer lock has no receiver".to_owned(),
                        call.span,
                    );
                    return None;
                };
                Some(hir::Expression::LockWeak(Box::new(
                    self.lower_expression(receiver, ExpressionMode::Reference)?,
                )))
            }
            CallTarget::Intrinsic(Intrinsic::AtomicLoad { nullable, .. }) => {
                let Some(ast::Expression {
                    kind: ExpressionKind::Field { receiver, .. },
                    ..
                }) = callee
                else {
                    self.push(
                        "HIR011",
                        "atomic pointer load has no receiver".to_owned(),
                        call.span,
                    );
                    return None;
                };
                Some(hir::Expression::AtomicLoad {
                    nullable: *nullable,
                    slot: Box::new(self.lower_expression(receiver, ExpressionMode::Reference)?),
                })
            }
            CallTarget::Intrinsic(Intrinsic::AtomicStore { nullable, target }) => {
                let Some(ast::Expression {
                    kind: ExpressionKind::Field { receiver, .. },
                    ..
                }) = callee
                else {
                    self.push(
                        "HIR011",
                        "atomic pointer store has no receiver".to_owned(),
                        call.span,
                    );
                    return None;
                };
                let value = arguments.first()?;
                let kind = if *nullable {
                    PointerKind::SharedNullable
                } else {
                    PointerKind::Shared
                };
                Some(hir::Expression::AtomicStore {
                    slot: Box::new(self.lower_expression(receiver, ExpressionMode::Reference)?),
                    value: Box::new(
                        self.lower_bound_expression(
                            value,
                            &TypeRef::pointer(kind, target.clone()),
                        )?,
                    ),
                })
            }
            CallTarget::Intrinsic(Intrinsic::AtomicSwap { nullable, target }) => {
                let Some(ast::Expression {
                    kind: ExpressionKind::Field { receiver, .. },
                    ..
                }) = callee
                else {
                    self.push(
                        "HIR011",
                        "atomic pointer swap has no receiver".to_owned(),
                        call.span,
                    );
                    return None;
                };
                let value = arguments.first()?;
                let kind = if *nullable {
                    PointerKind::SharedNullable
                } else {
                    PointerKind::Shared
                };
                Some(hir::Expression::AtomicSwap {
                    slot: Box::new(self.lower_expression(receiver, ExpressionMode::Reference)?),
                    value: Box::new(
                        self.lower_bound_expression(
                            value,
                            &TypeRef::pointer(kind, target.clone()),
                        )?,
                    ),
                })
            }
            CallTarget::Intrinsic(Intrinsic::MutexNew { construction, .. }) => {
                let value = self.lower_resolved_call(construction, None, arguments)?;
                Some(hir::Expression::MutexNew(Box::new(value)))
            }
            CallTarget::Intrinsic(Intrinsic::ConditionNew) => Some(hir::Expression::ConditionNew),
            CallTarget::Intrinsic(Intrinsic::MutexLock { .. }) => {
                let Some(ast::Expression {
                    kind: ExpressionKind::Field { receiver, .. },
                    ..
                }) = callee
                else {
                    self.push("HIR011", "mutex lock has no receiver".to_owned(), call.span);
                    return None;
                };
                Some(hir::Expression::MutexLock(Box::new(
                    self.lower_expression(receiver, ExpressionMode::Reference)?,
                )))
            }
            CallTarget::Intrinsic(Intrinsic::ConditionWait { .. }) => {
                let Some(ast::Expression {
                    kind: ExpressionKind::Field { receiver, .. },
                    ..
                }) = callee
                else {
                    self.push(
                        "HIR011",
                        "condition wait has no receiver".to_owned(),
                        call.span,
                    );
                    return None;
                };
                Some(hir::Expression::ConditionWait {
                    condition: Box::new(
                        self.lower_expression(receiver, ExpressionMode::Reference)?,
                    ),
                    guard: Box::new(
                        self.lower_expression(arguments.first()?, ExpressionMode::Value)?,
                    ),
                })
            }
            CallTarget::Intrinsic(Intrinsic::ConditionNotify { all }) => {
                let Some(ast::Expression {
                    kind: ExpressionKind::Field { receiver, .. },
                    ..
                }) = callee
                else {
                    self.push(
                        "HIR011",
                        "condition notification has no receiver".to_owned(),
                        call.span,
                    );
                    return None;
                };
                Some(hir::Expression::ConditionNotify {
                    condition: Box::new(
                        self.lower_expression(receiver, ExpressionMode::Reference)?,
                    ),
                    all: *all,
                })
            }
            CallTarget::Intrinsic(Intrinsic::ThreadSpawn) => Some(hir::Expression::ThreadSpawn(
                Box::new(self.lower_expression(arguments.first()?, ExpressionMode::Value)?),
            )),
            CallTarget::Intrinsic(Intrinsic::ThreadJoin) => {
                let Some(ast::Expression {
                    kind: ExpressionKind::Field { receiver, .. },
                    ..
                }) = callee
                else {
                    self.push(
                        "HIR011",
                        "thread join has no receiver".to_owned(),
                        call.span,
                    );
                    return None;
                };
                Some(hir::Expression::ThreadJoin(Box::new(
                    self.lower_expression(receiver, ExpressionMode::Value)?,
                )))
            }
            CallTarget::Intrinsic(Intrinsic::ThreadScope) => Some(hir::Expression::ThreadScope(
                Box::new(self.lower_expression(arguments.first()?, ExpressionMode::Value)?),
            )),
            CallTarget::Intrinsic(Intrinsic::ScopedThreadSpawn) => {
                let Some(ast::Expression {
                    kind: ExpressionKind::Field { receiver, .. },
                    ..
                }) = callee
                else {
                    self.push(
                        "HIR011",
                        "scoped thread spawn has no scope receiver".to_owned(),
                        call.span,
                    );
                    return None;
                };
                Some(hir::Expression::ScopedThreadSpawn {
                    scope: Box::new(self.lower_expression(receiver, ExpressionMode::Reference)?),
                    callback: Box::new(
                        self.lower_expression(arguments.first()?, ExpressionMode::Value)?,
                    ),
                })
            }
            CallTarget::Intrinsic(Intrinsic::ScopedThreadJoin) => {
                let Some(ast::Expression {
                    kind: ExpressionKind::Field { receiver, .. },
                    ..
                }) = callee
                else {
                    self.push(
                        "HIR011",
                        "scoped thread join has no receiver".to_owned(),
                        call.span,
                    );
                    return None;
                };
                Some(hir::Expression::ScopedThreadJoin(Box::new(
                    self.lower_expression(receiver, ExpressionMode::Value)?,
                )))
            }
            CallTarget::Intrinsic(Intrinsic::StoredFunctionCall { .. }) => {
                let callee = callee?;
                let TypeRef::Function(function) = self
                    .semantics
                    .expression(callee.span)
                    .map(|resolution| canonical_ref(&resolution.ty))?
                else {
                    self.push(
                        "HIR019",
                        "stored call has no function-typed callee".to_owned(),
                        call.span,
                    );
                    return None;
                };
                let arguments = arguments
                    .iter()
                    .zip(&function.parameters)
                    .map(|(argument, expected)| self.lower_bound_expression(argument, expected))
                    .collect::<Option<Vec<_>>>()?;
                Some(hir::Expression::CallableCall {
                    callable: Box::new(self.lower_expression(callee, ExpressionMode::Reference)?),
                    arguments,
                })
            }
            CallTarget::Intrinsic(Intrinsic::UnwrapRustResult {
                error_message,
                exception,
            }) => {
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
                    exception: lower_native_result_exception(*exception),
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
            CallTarget::Intrinsic(Intrinsic::JsonCast { target }) => {
                let expression = arguments.first()?;
                Some(hir::Expression::JsonCast {
                    expression: Box::new(
                        self.lower_expression(expression, ExpressionMode::Reference)?,
                    ),
                    target: self.lower_type(target, call.span)?,
                })
            }
            CallTarget::Intrinsic(Intrinsic::JsonWrap) => {
                Some(self.lower_json_value(arguments.first()?)?)
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

    fn lower_json_value(&mut self, expression: &ast::Expression) -> Option<hir::Expression> {
        let mut lowered = self.lower_expression(expression, ExpressionMode::Value)?;
        let resolution = self.semantics.expression(expression.span).cloned();
        let ty = resolution
            .as_ref()
            .map(|resolution| canonical_ref(&resolution.ty));
        if ty.is_some_and(is_json_type) {
            if resolution.as_ref().is_some_and(|resolution| {
                resolution.category != ValueCategory::Temporary
                    && !is_move_call(self.semantics, expression)
                    && !is_json_access(expression, self.semantics)
            }) {
                Some(hir::Expression::Clone {
                    expression: Box::new(hir::Expression::Borrow {
                        mutable: false,
                        expression: Box::new(lowered),
                    }),
                })
            } else {
                Some(lowered)
            }
        } else {
            if matches!(ty, Some(TypeRef::Struct { .. }))
                && resolution
                    .as_ref()
                    .is_some_and(|resolution| resolution.category != ValueCategory::Temporary)
                && !is_move_call(self.semantics, expression)
            {
                lowered = hir::Expression::Clone {
                    expression: Box::new(hir::Expression::Borrow {
                        mutable: false,
                        expression: Box::new(lowered),
                    }),
                };
            }
            Some(hir::Expression::JsonFrom(Box::new(lowered)))
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
            is_async: native.is_async,
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
                is_async: callback.is_async,
                kind: callback.kind,
                escape: callback.escape,
                parameters: callback
                    .parameters
                    .iter()
                    .map(|parameter| self.lower_type(parameter, span))
                    .collect::<Option<Vec<_>>>()?,
                return_type: Box::new(self.lower_type(&callback.return_type, span)?),
            },
            TypeRef::Function(function) => hir::Type::Function {
                kind: function.kind,
                parameters: function
                    .parameters
                    .iter()
                    .map(|parameter| self.lower_type(parameter, span))
                    .collect::<Option<Vec<_>>>()?,
                return_type: Box::new(self.lower_type(&function.return_type, span)?),
            },
            TypeRef::Pointer { kind, target } => hir::Type::Pointer {
                kind: *kind,
                target: Box::new(self.lower_type(target, span)?),
            },
            TypeRef::Mutex(target) => hir::Type::Mutex(Box::new(self.lower_type(target, span)?)),
            TypeRef::MutexGuard(target) => {
                hir::Type::MutexGuard(Box::new(self.lower_type(target, span)?))
            }
            TypeRef::Condition => hir::Type::Condition,
            TypeRef::ThreadHandle(target) => {
                hir::Type::ThreadHandle(Box::new(self.lower_type(target, span)?))
            }
            TypeRef::ThreadScope => hir::Type::ThreadScope,
            TypeRef::ScopedThreadHandle(target) => {
                hir::Type::ScopedThreadHandle(Box::new(self.lower_type(target, span)?))
            }
            TypeRef::Struct { path } | TypeRef::Class { path } => hir::Type::User {
                rust_path: user_type_path(path),
            },
            TypeRef::Interface { path } => hir::Type::Interface {
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

fn lower_native_result_exception(
    exception: crate::resolution::NativeResultException,
) -> hir::NativeExceptionKind {
    match exception {
        crate::resolution::NativeResultException::RustError => hir::NativeExceptionKind::RustError,
        crate::resolution::NativeResultException::IoError => hir::NativeExceptionKind::IoError,
        crate::resolution::NativeResultException::JsonError => hir::NativeExceptionKind::JsonError,
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

fn is_json_type(ty: &TypeRef) -> bool {
    matches!(
        ty,
        TypeRef::Native { path, arguments }
            if path == VAR_TYPE_PATH && arguments.is_empty()
    )
}

fn is_rust_string(ty: &TypeRef) -> bool {
    matches!(
        canonical_ref(ty),
        TypeRef::Native { path, arguments }
            if path == "rust::String" && arguments.is_empty()
    )
}

fn is_json_scalar_type(ty: &TypeRef) -> bool {
    is_json_type(ty)
        || is_rust_string(ty)
        || matches!(
            canonical_ref(ty),
            TypeRef::Bool
                | TypeRef::Char
                | TypeRef::I8
                | TypeRef::I16
                | TypeRef::I32
                | TypeRef::I64
                | TypeRef::I128
                | TypeRef::Isize
                | TypeRef::U8
                | TypeRef::U16
                | TypeRef::U32
                | TypeRef::U64
                | TypeRef::U128
                | TypeRef::Usize
                | TypeRef::F32
                | TypeRef::F64
        )
}

fn is_json_access(expression: &ast::Expression, semantics: &SemanticModel) -> bool {
    match &expression.kind {
        ExpressionKind::Field { receiver, .. } | ExpressionKind::Index { receiver, .. } => {
            semantics
                .expression(receiver.span)
                .is_some_and(|resolution| is_json_type(canonical_ref(&resolution.ty)))
        }
        ExpressionKind::Parenthesized(inner) => is_json_access(inner, semantics),
        _ => false,
    }
}

fn is_json_mutation_place(expression: &ast::Expression, semantics: &SemanticModel) -> bool {
    match &expression.kind {
        ExpressionKind::Field { receiver, .. } => {
            semantics
                .expression(expression.span)
                .is_some_and(|resolution| resolution.field.is_none())
                && semantics
                    .expression(receiver.span)
                    .is_some_and(|resolution| is_json_type(canonical_ref(&resolution.ty)))
        }
        ExpressionKind::Index { receiver, .. } => semantics
            .expression(receiver.span)
            .is_some_and(|resolution| is_json_type(canonical_ref(&resolution.ty))),
        _ => false,
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
                interfaces: Vec::new(),
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

fn insert_interface(
    interfaces: &mut Vec<hir::Interface>,
    modules: &mut Vec<hir::Module>,
    module_path: &[String],
    interface: hir::Interface,
) {
    let Some((source_module, remaining_path)) = module_path.split_first() else {
        interfaces.push(interface);
        return;
    };
    let index = modules
        .iter()
        .position(|module| module.source_name == *source_module)
        .unwrap_or_else(|| {
            modules.push(hir::Module {
                source_name: source_module.clone(),
                rust_name: module_name(source_module),
                interfaces: Vec::new(),
                structs: Vec::new(),
                functions: Vec::new(),
                modules: Vec::new(),
            });
            modules.len() - 1
        });
    let module = &mut modules[index];
    insert_interface(
        &mut module.interfaces,
        &mut module.modules,
        remaining_path,
        interface,
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
                interfaces: Vec::new(),
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

fn is_null_literal(expression: &ast::Expression) -> bool {
    match &expression.kind {
        ExpressionKind::Literal(ast::Literal {
            kind: ast::LiteralKind::Null,
            ..
        }) => true,
        ExpressionKind::Parenthesized(inner) => is_null_literal(inner),
        _ => false,
    }
}

fn is_nullptr_literal(expression: &ast::Expression) -> bool {
    match &expression.kind {
        ExpressionKind::Literal(ast::Literal {
            kind: ast::LiteralKind::Null,
            text,
        }) => text == "nullptr",
        ExpressionKind::Parenthesized(inner) => is_nullptr_literal(inner),
        _ => false,
    }
}

fn nullable_test_kind(ty: &TypeRef) -> Option<PointerKind> {
    let TypeRef::Pointer { kind, .. } = canonical_ref(ty) else {
        return None;
    };
    matches!(
        kind,
        PointerKind::UniqueNullable | PointerKind::SharedNullable | PointerKind::Weak
    )
    .then_some(*kind)
}

fn is_shared_to_weak_binding(expected: &TypeRef, actual: &TypeRef) -> bool {
    matches!(
        (canonical_ref(expected), canonical_ref(actual)),
        (
            TypeRef::Pointer {
                kind: PointerKind::Weak,
                target: expected_target,
            },
            TypeRef::Pointer {
                kind: PointerKind::Shared,
                target: actual_target,
            },
        ) if canonical_ref(expected_target) == canonical_ref(actual_target)
    )
}

fn nullable_owner_kind(ty: &TypeRef) -> Option<PointerKind> {
    nullable_test_kind(ty).filter(|kind| *kind != PointerKind::Weak)
}

fn reference_owner_kind(ty: &TypeRef) -> Option<PointerKind> {
    let TypeRef::Pointer { kind, .. } = canonical_ref(ty) else {
        return None;
    };
    matches!(
        kind,
        PointerKind::Unique
            | PointerKind::UniqueNullable
            | PointerKind::Shared
            | PointerKind::SharedNullable
    )
    .then_some(*kind)
}

fn automatic_pointee_type(ty: &TypeRef) -> &TypeRef {
    match canonical_ref(ty) {
        TypeRef::Pointer {
            kind:
                PointerKind::Unique
                | PointerKind::UniqueNullable
                | PointerKind::Shared
                | PointerKind::SharedNullable,
            target,
        }
        | TypeRef::MutexGuard(target) => canonical_ref(target),
        ty => ty,
    }
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
