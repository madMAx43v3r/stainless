use std::collections::{BTreeMap, BTreeSet};

use crate::Diagnostic;
use crate::ast::{
    self, BinaryOperator, Expression, ExpressionKind, ForClause, ForInitializer, Item, LiteralKind,
    PrefixOperator, SourceFile, Span, Statement, StatementKind, TypeKind,
};
use crate::interop::{
    CallStyle, CallableBinding, NativeBindings, NativeTypeBinding, Receiver, TypeRef,
};

use super::imports::ImportTable;
use super::mangle;
use super::{
    CallTarget, ExpressionResolution, FunctionId, FunctionSymbol, Intrinsic, NativeCall,
    ParameterSymbol, Resolution, ResolvedCall, ResolvedTraitRequirement, SemanticModel,
    ValueCategory,
};

/// Resolves names and types using an explicit native binding registry.
#[must_use]
pub fn resolve(source: &SourceFile, bindings: &NativeBindings) -> Resolution {
    let mut diagnostics = Vec::new();
    let imports = ImportTable::build(source, &mut diagnostics);
    let mut resolver = Resolver {
        bindings,
        imports,
        diagnostics,
        model: SemanticModel::default(),
        function_sets: BTreeMap::new(),
        function_by_span: BTreeMap::new(),
    };
    resolver.collect_signatures(&source.items, &mut Vec::new());
    resolver.resolve_bodies(&source.items, &mut Vec::new());
    resolver
        .diagnostics
        .sort_by_key(|diagnostic| diagnostic.span);
    Resolution {
        model: resolver.model,
        diagnostics: resolver.diagnostics,
    }
}

struct Resolver<'bindings> {
    bindings: &'bindings NativeBindings,
    imports: ImportTable,
    diagnostics: Vec<Diagnostic>,
    model: SemanticModel,
    function_sets: BTreeMap<Vec<String>, Vec<FunctionId>>,
    function_by_span: BTreeMap<Span, FunctionId>,
}

#[derive(Clone, Debug)]
struct Variable {
    ty: TypeRef,
    mutable: bool,
}

struct FunctionContext {
    namespace: Vec<String>,
    return_type: TypeRef,
    scopes: Vec<BTreeMap<String, Variable>>,
}

#[derive(Clone, Debug)]
struct ExpressionInfo {
    ty: TypeRef,
    category: ValueCategory,
}

#[derive(Clone, Debug)]
struct NativeInstance {
    type_path: &'static str,
    arguments: Vec<TypeRef>,
}

enum NativeInstanceLookup {
    NotNative,
    Resolved(NativeInstance),
    Invalid,
}

#[derive(Clone)]
struct ConcreteNativeCandidate {
    callable: CallableBinding,
    parameter_types: Vec<TypeRef>,
    return_type: TypeRef,
    requirements: Vec<ResolvedTraitRequirement>,
}

impl Resolver<'_> {
    fn collect_signatures(&mut self, items: &[Item], namespace: &mut Vec<String>) {
        for item in items {
            match item {
                Item::Namespace(child) => {
                    namespace.push(child.name.clone());
                    self.collect_signatures(&child.items, namespace);
                    namespace.pop();
                }
                Item::Function(function) => self.collect_function(function, namespace),
                Item::Use(_) => {}
            }
        }
    }

    fn collect_function(&mut self, function: &ast::Function, namespace: &[String]) {
        let path = qualify_declaration_path(namespace, &function.name.segments);
        let parameters = function
            .parameters
            .iter()
            .map(|parameter| ParameterSymbol {
                name: parameter.name.clone(),
                ty: self.resolve_type(&parameter.ty, namespace, false),
                span: parameter.span,
            })
            .collect::<Vec<_>>();
        let return_type = self.resolve_type(&function.return_type, namespace, false);
        let signature = parameters
            .iter()
            .map(|parameter| canonical(&parameter.ty))
            .collect::<Vec<_>>();

        let existing_ids = self.function_sets.get(&path).cloned().unwrap_or_default();
        for id in existing_ids {
            let existing = &self.model.functions[id.0];
            let existing_signature = existing
                .parameters
                .iter()
                .map(|parameter| canonical(&parameter.ty))
                .collect::<Vec<_>>();
            if existing_signature != signature {
                continue;
            }

            let same_passing_modes = existing
                .parameters
                .iter()
                .map(|parameter| &parameter.ty)
                .eq(parameters.iter().map(|parameter| &parameter.ty));
            let different_return_type = existing.return_type != return_type;
            let duplicate_definition = existing.has_definition && function.body.is_some();
            if same_passing_modes {
                if different_return_type {
                    self.push(
                        "RES004",
                        format!(
                            "declarations of `{}` have different return types",
                            display_path(&path)
                        ),
                        function.span,
                    );
                }
                if duplicate_definition {
                    self.push(
                        "RES005",
                        format!("duplicate definition of `{}`", display_path(&path)),
                        function.span,
                    );
                }
            } else {
                self.push(
                    "RES003",
                    format!(
                        "function `{}` conflicts with an overload that differs only by value/reference passing mode",
                        display_path(&path)
                    ),
                    function.span,
                );
            }

            let symbol = &mut self.model.functions[id.0];
            symbol.declarations.push(function.span);
            symbol.has_definition |= function.body.is_some();
            self.function_by_span.insert(function.span, id);
            return;
        }

        let id = FunctionId(self.model.functions.len());
        let mangled_name = mangle::function_name(
            &path,
            &parameters
                .iter()
                .map(|parameter| parameter.ty.clone())
                .collect::<Vec<_>>(),
        );
        self.model.functions.push(FunctionSymbol {
            id,
            path: path.clone(),
            parameters,
            return_type,
            mangled_name,
            declarations: vec![function.span],
            has_definition: function.body.is_some(),
        });
        self.function_sets.entry(path).or_default().push(id);
        self.function_by_span.insert(function.span, id);
    }

    fn resolve_bodies(&mut self, items: &[Item], namespace: &mut Vec<String>) {
        for item in items {
            match item {
                Item::Namespace(child) => {
                    namespace.push(child.name.clone());
                    self.resolve_bodies(&child.items, namespace);
                    namespace.pop();
                }
                Item::Function(function) => {
                    if function.body.is_some() {
                        self.resolve_function_body(function, namespace);
                    }
                }
                Item::Use(_) => {}
            }
        }
    }

    fn resolve_function_body(&mut self, function: &ast::Function, namespace: &[String]) {
        let Some(id) = self.function_by_span.get(&function.span).copied() else {
            return;
        };
        let symbol = self.model.functions[id.0].clone();
        let mut initial_scope = BTreeMap::new();
        for (index, parameter) in function.parameters.iter().enumerate() {
            let Some(symbol_parameter) = symbol.parameters.get(index) else {
                continue;
            };
            initial_scope.insert(
                parameter.name.clone(),
                Variable {
                    ty: symbol_parameter.ty.clone(),
                    mutable: parameter_mutability(parameter, &symbol_parameter.ty),
                },
            );
        }
        let mut context = FunctionContext {
            namespace: namespace.to_vec(),
            return_type: symbol.return_type,
            scopes: vec![initial_scope],
        };
        if let Some(body) = &function.body {
            self.resolve_block(body, &mut context, false);
        }
    }

    fn resolve_block(
        &mut self,
        block: &ast::Block,
        context: &mut FunctionContext,
        create_scope: bool,
    ) {
        if create_scope {
            context.scopes.push(BTreeMap::new());
        }
        for statement in &block.statements {
            self.resolve_statement(statement, context);
        }
        if create_scope {
            context.scopes.pop();
        }
    }

    fn resolve_statement(&mut self, statement: &Statement, context: &mut FunctionContext) {
        match &statement.kind {
            StatementKind::Block(block) => self.resolve_block(block, context, true),
            StatementKind::Local(local) => self.resolve_local(local, context),
            StatementKind::Return(value) => {
                if let Some(value) = value {
                    let expected = context.return_type.clone();
                    if expected == TypeRef::Void {
                        self.resolve_expression(value, None, context);
                    } else {
                        let actual =
                            self.resolve_expression(value, Some(&canonical(&expected)), context);
                        self.validate_binding(&expected, &actual, value.span, "return value");
                    }
                }
            }
            StatementKind::If(if_statement) => {
                let condition =
                    self.resolve_expression(&if_statement.condition, Some(&TypeRef::Bool), context);
                self.require_exact(
                    &TypeRef::Bool,
                    &condition.ty,
                    if_statement.condition.span,
                    "if condition",
                );
                self.resolve_statement(&if_statement.then_branch, context);
                if let Some(else_branch) = &if_statement.else_branch {
                    self.resolve_statement(else_branch, context);
                }
            }
            StatementKind::For(for_statement) => {
                context.scopes.push(BTreeMap::new());
                match &for_statement.clause {
                    ForClause::Classic(classic) => {
                        if let Some(initializer) = &classic.initializer {
                            match initializer {
                                ForInitializer::Local(local) => self.resolve_local(local, context),
                                ForInitializer::Expression(expression) => {
                                    self.resolve_expression(expression, None, context);
                                }
                            }
                        }
                        if let Some(condition) = &classic.condition {
                            let actual =
                                self.resolve_expression(condition, Some(&TypeRef::Bool), context);
                            self.require_exact(
                                &TypeRef::Bool,
                                &actual.ty,
                                condition.span,
                                "for condition",
                            );
                        }
                        if let Some(update) = &classic.update {
                            self.resolve_expression(update, None, context);
                        }
                    }
                    ForClause::Range(range) => self.resolve_range_binding(range, context),
                    ForClause::Error => {}
                }
                self.resolve_statement(&for_statement.body, context);
                context.scopes.pop();
            }
            StatementKind::Expression(expression) => {
                self.resolve_expression(expression, None, context);
            }
            StatementKind::Break
            | StatementKind::Continue
            | StatementKind::Empty
            | StatementKind::Error => {}
        }
    }

    fn resolve_local(&mut self, local: &ast::LocalDeclaration, context: &mut FunctionContext) {
        let declared = if local.ty.is_inferred() {
            None
        } else {
            Some(self.resolve_type(&local.ty, &context.namespace, false))
        };
        let resolved_type = if let Some(initializer) = &local.initializer {
            let expected = declared.as_ref().map(canonical);
            let actual = self.resolve_expression(initializer, expected.as_ref(), context);
            if let Some(declared) = &declared {
                self.validate_binding(declared, &actual, initializer.span, "initializer");
                declared.clone()
            } else {
                let inferred = canonical(&actual.ty);
                self.validate_value_use(&inferred, &actual, initializer.span, "initializer");
                inferred
            }
        } else {
            let ty = declared.unwrap_or(TypeRef::Error);
            self.resolve_default_construction(&ty, local.span);
            ty
        };

        let variable = Variable {
            mutable: if resolved_type.is_reference() {
                matches!(resolved_type, TypeRef::Reference { mutable: true, .. })
            } else {
                !local.ty.is_const
            },
            ty: resolved_type,
        };
        let scope = context
            .scopes
            .last_mut()
            .expect("a function context always has a scope");
        self.insert_variable(scope, &local.name, variable, local.span);
    }

    fn resolve_range_binding(
        &mut self,
        range: &ast::RangeForClause,
        context: &mut FunctionContext,
    ) {
        let iterable = self.resolve_expression(&range.iterable, None, context);
        let canonical_iterable = canonical(&iterable.ty);
        let TypeRef::Native { path, arguments } = &canonical_iterable else {
            if canonical_iterable != TypeRef::Error {
                self.push(
                    "RES006",
                    format!(
                        "range expression has non-iterable type `{}`",
                        display_type(&canonical_iterable)
                    ),
                    range.iterable.span,
                );
            }
            return;
        };
        if *path != "rust::Vec" || arguments.len() != 1 {
            self.push(
                "RES006",
                format!(
                    "range iteration is not implemented for `{}`",
                    display_type(&canonical_iterable)
                ),
                range.iterable.span,
            );
            return;
        }
        let element = arguments[0].clone();
        let binding_type = if range.ty.is_inferred() {
            if range.ty.is_reference {
                TypeRef::Reference {
                    mutable: !range.ty.is_const,
                    target: Box::new(element.clone()),
                }
            } else {
                element.clone()
            }
        } else {
            self.resolve_type(&range.ty, &context.namespace, false)
        };
        if canonical(&binding_type) != element {
            self.push(
                "RES007",
                format!(
                    "range binding type `{}` does not exactly match element type `{}`",
                    display_type(&binding_type),
                    display_type(&element)
                ),
                range.ty.span,
            );
        }
        if matches!(binding_type, TypeRef::Reference { mutable: true, .. })
            && iterable.category != ValueCategory::MutablePlace
        {
            self.push(
                "RES008",
                "mutable range binding requires a mutable range".to_owned(),
                range.iterable.span,
            );
        }
        if !binding_type.is_reference()
            && iterable.category != ValueCategory::Temporary
            && !is_copyable(&element)
        {
            self.push(
                "RES009",
                format!(
                    "copying range elements of type `{}` is not implicit; consume the range with `move`",
                    display_type(&element)
                ),
                range.ty.span,
            );
        }

        let variable = Variable {
            mutable: matches!(binding_type, TypeRef::Reference { mutable: true, .. })
                || (!binding_type.is_reference() && !range.ty.is_const),
            ty: binding_type,
        };
        let scope = context
            .scopes
            .last_mut()
            .expect("a function context always has a scope");
        self.insert_variable(scope, &range.name, variable, range.ty.span);
    }

    fn resolve_expression(
        &mut self,
        expression: &Expression,
        expected: Option<&TypeRef>,
        context: &mut FunctionContext,
    ) -> ExpressionInfo {
        let (info, call) = match &expression.kind {
            ExpressionKind::Name(path) => (
                self.resolve_value_name(path, expression.span, context),
                None,
            ),
            ExpressionKind::Literal(literal) => (
                ExpressionInfo {
                    ty: literal_type(literal.kind, &literal.text, expected),
                    category: ValueCategory::Temporary,
                },
                None,
            ),
            ExpressionKind::Parenthesized(inner) => {
                (self.resolve_expression(inner, expected, context), None)
            }
            ExpressionKind::Prefix { operator, operand } => (
                self.resolve_prefix(*operator, operand, expected, context),
                None,
            ),
            ExpressionKind::Postfix { operand, .. } => {
                let actual = self.resolve_expression(operand, expected, context);
                self.require_mutable_numeric(&actual, operand.span, "postfix operator");
                (
                    ExpressionInfo {
                        ty: canonical(&actual.ty),
                        category: ValueCategory::Temporary,
                    },
                    None,
                )
            }
            ExpressionKind::Binary {
                left,
                operator,
                right,
            } => (
                self.resolve_binary(left, *operator, right, expected, context),
                None,
            ),
            ExpressionKind::Call { callee, arguments } => {
                self.resolve_call(callee, arguments, expected, expression.span, context)
            }
            ExpressionKind::Field { receiver, name } => {
                self.resolve_expression(receiver, None, context);
                self.push(
                    "RES010",
                    format!("field access `{name}` is not implemented in the current type subset"),
                    expression.span,
                );
                (error_info(), None)
            }
            ExpressionKind::Index { receiver, index } => {
                self.resolve_expression(receiver, None, context);
                self.resolve_expression(index, Some(&TypeRef::Usize), context);
                self.push(
                    "RES011",
                    "indexing is not exposed by the initial Vec/String bindings".to_owned(),
                    expression.span,
                );
                (error_info(), None)
            }
            ExpressionKind::Error => (error_info(), None),
        };
        self.record_expression(expression.span, info.clone(), call);
        info
    }

    fn resolve_value_name(
        &mut self,
        path: &ast::Path,
        span: Span,
        context: &FunctionContext,
    ) -> ExpressionInfo {
        if path.segments.len() == 1 {
            let name = &path.segments[0];
            for scope in context.scopes.iter().rev() {
                if let Some(variable) = scope.get(name) {
                    return ExpressionInfo {
                        ty: variable.ty.clone(),
                        category: if variable.mutable {
                            ValueCategory::MutablePlace
                        } else {
                            ValueCategory::SharedPlace
                        },
                    };
                }
            }
        }
        self.push(
            "RES012",
            format!("unresolved value name `{}`", path.display()),
            span,
        );
        error_info()
    }

    fn resolve_prefix(
        &mut self,
        operator: PrefixOperator,
        operand: &Expression,
        expected: Option<&TypeRef>,
        context: &mut FunctionContext,
    ) -> ExpressionInfo {
        let operand_expected = match operator {
            PrefixOperator::Not => Some(&TypeRef::Bool),
            _ => expected,
        };
        let actual = self.resolve_expression(operand, operand_expected, context);
        match operator {
            PrefixOperator::Not => {
                self.require_exact(&TypeRef::Bool, &actual.ty, operand.span, "`!` operand");
                temporary(TypeRef::Bool)
            }
            PrefixOperator::Increment | PrefixOperator::Decrement => {
                self.require_mutable_numeric(&actual, operand.span, "prefix operator");
                temporary(canonical(&actual.ty))
            }
            PrefixOperator::Plus | PrefixOperator::Negate | PrefixOperator::BitwiseNot => {
                let ty = canonical(&actual.ty);
                if is_numeric(&ty) {
                    temporary(ty)
                } else {
                    self.invalid_operand(operator_name(operator), &ty, operand.span);
                    error_info()
                }
            }
        }
    }

    fn resolve_binary(
        &mut self,
        left: &Expression,
        operator: BinaryOperator,
        right: &Expression,
        expected: Option<&TypeRef>,
        context: &mut FunctionContext,
    ) -> ExpressionInfo {
        if is_assignment(operator) {
            let left_info = self.resolve_expression(left, expected, context);
            let left_type = canonical(&left_info.ty);
            let right_info = self.resolve_expression(right, Some(&left_type), context);
            if left_info.category != ValueCategory::MutablePlace {
                self.push(
                    "RES013",
                    "assignment requires a mutable place".to_owned(),
                    left.span,
                );
            }
            if operator == BinaryOperator::Assign {
                self.validate_binding(&left_type, &right_info, right.span, "assignment");
            } else {
                self.require_exact(&left_type, &right_info.ty, right.span, "assignment");
            }
            if operator != BinaryOperator::Assign && !is_numeric(&left_type) {
                self.invalid_operand(binary_name(operator), &left_type, left.span);
            }
            return temporary(TypeRef::Void);
        }

        if matches!(
            operator,
            BinaryOperator::LogicalAnd | BinaryOperator::LogicalOr
        ) {
            let left_info = self.resolve_expression(left, Some(&TypeRef::Bool), context);
            let right_info = self.resolve_expression(right, Some(&TypeRef::Bool), context);
            self.require_exact(&TypeRef::Bool, &left_info.ty, left.span, "logical operand");
            self.require_exact(
                &TypeRef::Bool,
                &right_info.ty,
                right.span,
                "logical operand",
            );
            return temporary(TypeRef::Bool);
        }

        let left_info = self.resolve_expression(left, expected, context);
        let left_type = canonical(&left_info.ty);
        let right_info = self.resolve_expression(right, Some(&left_type), context);
        self.require_exact(&left_type, &right_info.ty, right.span, "binary operand");
        if !is_numeric(&left_type) {
            self.invalid_operand(binary_name(operator), &left_type, left.span);
            return error_info();
        }
        if is_comparison(operator) {
            temporary(TypeRef::Bool)
        } else {
            temporary(left_type)
        }
    }

    fn resolve_call(
        &mut self,
        callee: &Expression,
        arguments: &[Expression],
        expected: Option<&TypeRef>,
        span: Span,
        context: &mut FunctionContext,
    ) -> (ExpressionInfo, Option<ResolvedCall>) {
        match &callee.kind {
            ExpressionKind::Name(path)
                if path.segments.len() == 1 && path.segments[0] == "move" =>
            {
                self.resolve_move(arguments, span, context)
            }
            ExpressionKind::Field { receiver, name } => {
                self.resolve_native_method(receiver, name, arguments, span, context)
            }
            ExpressionKind::Name(path) => {
                if let Some(target) = primitive_type(&path.segments) {
                    return self.resolve_primitive_cast(target, arguments, span, context);
                }
                match self.lookup_native_instance(path, expected, context, span) {
                    NativeInstanceLookup::Resolved(instance) => {
                        let source_name = instance_short_name(instance.type_path);
                        return self.resolve_native_callable(
                            &instance,
                            CallStyle::Constructor,
                            source_name,
                            arguments,
                            span,
                            None,
                            context,
                        );
                    }
                    NativeInstanceLookup::Invalid => {
                        for argument in arguments {
                            self.resolve_expression(argument, None, context);
                        }
                        return (error_info(), None);
                    }
                    NativeInstanceLookup::NotNative => {}
                }
                if path.segments.len() >= 2 {
                    let type_path = ast::Path {
                        segments: path.segments[..path.segments.len() - 1].to_vec(),
                    };
                    match self.lookup_native_instance(&type_path, expected, context, span) {
                        NativeInstanceLookup::Resolved(instance) => {
                            return self.resolve_native_callable(
                                &instance,
                                CallStyle::AssociatedFunction,
                                path.segments.last().expect("non-empty path"),
                                arguments,
                                span,
                                None,
                                context,
                            );
                        }
                        NativeInstanceLookup::Invalid => {
                            for argument in arguments {
                                self.resolve_expression(argument, None, context);
                            }
                            return (error_info(), None);
                        }
                        NativeInstanceLookup::NotNative => {}
                    }
                }
                self.resolve_stainless_call(path, arguments, span, context)
            }
            _ => {
                self.resolve_expression(callee, None, context);
                for argument in arguments {
                    self.resolve_expression(argument, None, context);
                }
                self.push(
                    "RES014",
                    "expression is not callable".to_owned(),
                    callee.span,
                );
                (error_info(), None)
            }
        }
    }

    fn resolve_move(
        &mut self,
        arguments: &[Expression],
        span: Span,
        context: &mut FunctionContext,
    ) -> (ExpressionInfo, Option<ResolvedCall>) {
        if arguments.len() != 1 {
            self.push(
                "RES015",
                "`move` requires exactly one argument".to_owned(),
                span,
            );
            for argument in arguments {
                self.resolve_expression(argument, None, context);
            }
            return (error_info(), None);
        }
        let argument = self.resolve_expression(&arguments[0], None, context);
        if !matches!(
            argument.category,
            ValueCategory::MutablePlace | ValueCategory::SharedPlace
        ) || argument.ty.is_reference()
        {
            self.push(
                "RES015",
                "`move` requires a named value binding".to_owned(),
                arguments[0].span,
            );
        }
        let return_type = canonical(&argument.ty);
        let call = ResolvedCall {
            span,
            target: CallTarget::Intrinsic(Intrinsic::Move),
            return_type: return_type.clone(),
        };
        (temporary(return_type), Some(call))
    }

    fn resolve_primitive_cast(
        &mut self,
        target: TypeRef,
        arguments: &[Expression],
        span: Span,
        context: &mut FunctionContext,
    ) -> (ExpressionInfo, Option<ResolvedCall>) {
        if arguments.len() != 1 {
            self.push(
                "RES016",
                "primitive conversion requires exactly one argument".to_owned(),
                span,
            );
            return (error_info(), None);
        }
        let argument = self.resolve_expression(&arguments[0], None, context);
        if !is_numeric(&canonical(&argument.ty)) || !is_numeric(&target) {
            self.push(
                "RES016",
                format!(
                    "cannot convert `{}` to `{}` with a primitive constructor",
                    display_type(&argument.ty),
                    display_type(&target)
                ),
                span,
            );
            return (error_info(), None);
        }
        let call = ResolvedCall {
            span,
            target: CallTarget::Intrinsic(Intrinsic::PrimitiveCast {
                target: target.clone(),
            }),
            return_type: target.clone(),
        };
        (temporary(target), Some(call))
    }

    fn resolve_stainless_call(
        &mut self,
        path: &ast::Path,
        arguments: &[Expression],
        span: Span,
        context: &mut FunctionContext,
    ) -> (ExpressionInfo, Option<ResolvedCall>) {
        let candidates = self.function_candidates(path, &context.namespace);
        if candidates.is_empty() {
            for argument in arguments {
                self.resolve_expression(argument, None, context);
            }
            self.push(
                "RES017",
                format!("unresolved function `{}`", path.display()),
                span,
            );
            return (error_info(), None);
        }
        let arity_candidates = candidates
            .into_iter()
            .filter(|id| self.model.functions[id.0].parameters.len() == arguments.len())
            .collect::<Vec<_>>();
        if arity_candidates.is_empty() {
            for argument in arguments {
                self.resolve_expression(argument, None, context);
            }
            self.push(
                "RES018",
                format!(
                    "no overload of `{}` accepts {} argument(s)",
                    path.display(),
                    arguments.len()
                ),
                span,
            );
            return (error_info(), None);
        }

        let contextual_parameters = (arity_candidates.len() == 1).then(|| {
            self.model.functions[arity_candidates[0].0]
                .parameters
                .iter()
                .map(|parameter| canonical(&parameter.ty))
                .collect::<Vec<_>>()
        });
        let actual = self.resolve_arguments(arguments, contextual_parameters.as_deref(), context);
        let exact = arity_candidates
            .iter()
            .copied()
            .filter(|id| {
                self.model.functions[id.0]
                    .parameters
                    .iter()
                    .map(|parameter| canonical(&parameter.ty))
                    .eq(actual.iter().map(|argument| canonical(&argument.ty)))
            })
            .collect::<Vec<_>>();
        if exact.len() != 1 {
            let displayed_candidates = if exact.is_empty() {
                &arity_candidates
            } else {
                &exact
            }
            .iter()
            .map(|id| display_function_signature(&self.model.functions[id.0]))
            .collect::<Vec<_>>()
            .join("; ");
            let message = if exact.is_empty() {
                format!(
                    "no exact overload of `{}` matches ({}); candidates: {displayed_candidates}",
                    path.display(),
                    display_argument_types(&actual)
                )
            } else {
                format!(
                    "call to `{}` is ambiguous for ({}); candidates: {displayed_candidates}",
                    path.display(),
                    display_argument_types(&actual)
                )
            };
            self.push("RES019", message, span);
            return (error_info(), None);
        }

        let id = exact[0];
        let symbol = self.model.functions[id.0].clone();
        for ((parameter, argument), expression) in
            symbol.parameters.iter().zip(&actual).zip(arguments)
        {
            self.validate_binding(&parameter.ty, argument, expression.span, "argument");
        }
        let return_type = symbol.return_type;
        let call = ResolvedCall {
            span,
            target: CallTarget::Stainless(id),
            return_type: return_type.clone(),
        };
        (info_for_return_type(return_type), Some(call))
    }

    fn resolve_native_method(
        &mut self,
        receiver: &Expression,
        name: &str,
        arguments: &[Expression],
        span: Span,
        context: &mut FunctionContext,
    ) -> (ExpressionInfo, Option<ResolvedCall>) {
        let receiver_info = self.resolve_expression(receiver, None, context);
        let receiver_type = canonical(&receiver_info.ty);
        let TypeRef::Native {
            path,
            arguments: type_arguments,
        } = receiver_type
        else {
            for argument in arguments {
                self.resolve_expression(argument, None, context);
            }
            if receiver_type != TypeRef::Error {
                self.push(
                    "RES020",
                    format!(
                        "type `{}` has no native method `{name}`",
                        display_type(&receiver_type)
                    ),
                    span,
                );
            }
            return (error_info(), None);
        };
        let instance = NativeInstance {
            type_path: path,
            arguments: type_arguments,
        };
        self.resolve_native_callable(
            &instance,
            CallStyle::Method,
            name,
            arguments,
            span,
            Some((&receiver_info, receiver.span)),
            context,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn resolve_native_callable(
        &mut self,
        instance: &NativeInstance,
        style: CallStyle,
        name: &str,
        arguments: &[Expression],
        span: Span,
        receiver: Option<(&ExpressionInfo, Span)>,
        context: &mut FunctionContext,
    ) -> (ExpressionInfo, Option<ResolvedCall>) {
        let Some(candidates) =
            self.concrete_native_candidates(instance, style, name, arguments.len())
        else {
            for argument in arguments {
                self.resolve_expression(argument, None, context);
            }
            self.push(
                "RES021",
                format!(
                    "native type `{}` has no callable metadata",
                    instance.type_path
                ),
                span,
            );
            return (error_info(), None);
        };
        if candidates.is_empty() {
            for argument in arguments {
                self.resolve_expression(argument, None, context);
            }
            self.push(
                "RES022",
                format!(
                    "`{}` has no supported {} `{name}` with {} argument(s)",
                    display_native_instance(instance),
                    call_style_name(style),
                    arguments.len()
                ),
                span,
            );
            return (error_info(), None);
        }

        let contextual = (candidates.len() == 1).then(|| {
            candidates[0]
                .parameter_types
                .iter()
                .map(canonical)
                .collect::<Vec<_>>()
        });
        let actual = self.resolve_arguments(arguments, contextual.as_deref(), context);
        let exact = candidates
            .iter()
            .filter(|candidate| {
                candidate
                    .parameter_types
                    .iter()
                    .map(canonical)
                    .eq(actual.iter().map(|argument| canonical(&argument.ty)))
            })
            .cloned()
            .collect::<Vec<_>>();
        if exact.len() != 1 {
            let displayed_candidates = if exact.is_empty() {
                &candidates
            } else {
                &exact
            }
            .iter()
            .map(display_native_signature)
            .collect::<Vec<_>>()
            .join("; ");
            self.push(
                "RES023",
                format!(
                    "no exact {} `{name}` on `{}` matches ({}); candidates: {displayed_candidates}",
                    call_style_name(style),
                    display_native_instance(instance),
                    display_argument_types(&actual)
                ),
                span,
            );
            return (error_info(), None);
        }
        let candidate = exact.into_iter().next().expect("one exact candidate");
        self.finish_native_call(
            instance, style, candidate, &actual, arguments, span, receiver, name,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn finish_native_call(
        &mut self,
        instance: &NativeInstance,
        style: CallStyle,
        candidate: ConcreteNativeCandidate,
        actual: &[ExpressionInfo],
        arguments: &[Expression],
        span: Span,
        receiver: Option<(&ExpressionInfo, Span)>,
        name: &str,
    ) -> (ExpressionInfo, Option<ResolvedCall>) {
        if let Some((receiver_info, receiver_span)) = receiver {
            self.validate_receiver(
                candidate.callable.receiver,
                receiver_info,
                receiver_span,
                name,
            );
        }
        for ((expected, argument), expression) in
            candidate.parameter_types.iter().zip(actual).zip(arguments)
        {
            self.validate_binding(expected, argument, expression.span, "argument");
        }
        let native_call = NativeCall {
            type_path: instance.type_path,
            style,
            source_name: candidate.callable.source_name,
            receiver: candidate.callable.receiver,
            parameter_types: candidate.parameter_types,
            adaptations: candidate
                .callable
                .parameters
                .iter()
                .map(|parameter| parameter.adaptation)
                .collect(),
            return_type: candidate.return_type.clone(),
            lowering: candidate.callable.lowering,
            requirements: candidate.requirements,
        };
        let call = ResolvedCall {
            span,
            target: CallTarget::Native(native_call),
            return_type: candidate.return_type.clone(),
        };
        (info_for_return_type(candidate.return_type), Some(call))
    }

    fn concrete_native_candidates(
        &self,
        instance: &NativeInstance,
        style: CallStyle,
        name: &str,
        arity: usize,
    ) -> Option<Vec<ConcreteNativeCandidate>> {
        let binding = self.bindings.type_by_path(instance.type_path)?;
        Some(
            binding
                .callables
                .iter()
                .filter(|callable| {
                    callable.style == style
                        && callable.source_name == name
                        && callable.parameters.len() == arity
                })
                .map(|callable| instantiate_callable(binding, &instance.arguments, callable))
                .collect(),
        )
    }

    fn resolve_arguments(
        &mut self,
        arguments: &[Expression],
        expected: Option<&[TypeRef]>,
        context: &mut FunctionContext,
    ) -> Vec<ExpressionInfo> {
        arguments
            .iter()
            .enumerate()
            .map(|(index, argument)| {
                self.resolve_expression(
                    argument,
                    expected.and_then(|types| types.get(index)),
                    context,
                )
            })
            .collect()
    }

    fn validate_receiver(
        &mut self,
        receiver: Option<Receiver>,
        actual: &ExpressionInfo,
        span: Span,
        name: &str,
    ) {
        match receiver {
            Some(Receiver::Mutable) if actual.category == ValueCategory::SharedPlace => self.push(
                "RES024",
                format!("method `{name}` requires a mutable receiver"),
                span,
            ),
            Some(Receiver::Value) if actual.category != ValueCategory::Temporary => self.push(
                "RES025",
                format!("consuming method `{name}` requires `move(receiver)`"),
                span,
            ),
            _ => {}
        }
    }

    fn validate_binding(
        &mut self,
        expected: &TypeRef,
        actual: &ExpressionInfo,
        span: Span,
        description: &str,
    ) {
        self.require_exact(expected, &actual.ty, span, description);
        match expected {
            TypeRef::Reference { mutable: true, .. }
                if actual.category != ValueCategory::MutablePlace =>
            {
                self.push(
                    "RES026",
                    format!("{description} requires a mutable reference"),
                    span,
                );
            }
            TypeRef::Reference { .. } => {}
            _ => self.validate_value_use(expected, actual, span, description),
        }
    }

    fn validate_value_use(
        &mut self,
        expected: &TypeRef,
        actual: &ExpressionInfo,
        span: Span,
        description: &str,
    ) {
        if canonical(expected) == canonical(&actual.ty)
            && actual.category != ValueCategory::Temporary
            && !is_copyable(&canonical(expected))
        {
            self.push(
                "RES027",
                format!(
                    "{description} of non-copy type `{}` requires `move(...)`",
                    display_type(&canonical(expected))
                ),
                span,
            );
        }
    }

    fn require_exact(
        &mut self,
        expected: &TypeRef,
        actual: &TypeRef,
        span: Span,
        description: &str,
    ) {
        let expected = canonical(expected);
        let actual = canonical(actual);
        if expected != TypeRef::Error && actual != TypeRef::Error && expected != actual {
            self.push(
                "RES028",
                format!(
                    "{description} requires `{}`, found `{}`",
                    display_type(&expected),
                    display_type(&actual)
                ),
                span,
            );
        }
    }

    fn require_mutable_numeric(&mut self, actual: &ExpressionInfo, span: Span, description: &str) {
        if actual.category != ValueCategory::MutablePlace {
            self.push(
                "RES013",
                format!("{description} requires a mutable place"),
                span,
            );
        }
        let ty = canonical(&actual.ty);
        if !is_numeric(&ty) {
            self.invalid_operand(description, &ty, span);
        }
    }

    fn invalid_operand(&mut self, operator: &str, ty: &TypeRef, span: Span) {
        if *ty != TypeRef::Error {
            self.push(
                "RES029",
                format!(
                    "operator `{operator}` is not defined for `{}`",
                    display_type(ty)
                ),
                span,
            );
        }
    }

    fn resolve_default_construction(&mut self, ty: &TypeRef, span: Span) {
        let canonical_type = canonical(ty);
        let TypeRef::Native { path, arguments } = canonical_type else {
            if canonical_type != TypeRef::Error {
                self.push(
                    "RES030",
                    format!(
                        "type `{}` has no implicit default constructor",
                        display_type(&canonical_type)
                    ),
                    span,
                );
            }
            return;
        };
        let instance = NativeInstance {
            type_path: path,
            arguments,
        };
        let Some(binding) = self.bindings.type_by_path(path) else {
            self.push(
                "RES030",
                format!("type `{path}` has no registered default constructor"),
                span,
            );
            return;
        };
        let Some(callable) = binding.callables.iter().find(|callable| {
            callable.style == CallStyle::Constructor && callable.parameters.is_empty()
        }) else {
            self.push(
                "RES030",
                format!(
                    "type `{}` has no registered default constructor",
                    display_native_instance(&instance)
                ),
                span,
            );
            return;
        };
        let candidate = instantiate_callable(binding, &instance.arguments, callable);
        let call = ResolvedCall {
            span,
            target: CallTarget::Native(NativeCall {
                type_path: path,
                style: CallStyle::Constructor,
                source_name: callable.source_name,
                receiver: None,
                parameter_types: Vec::new(),
                adaptations: Vec::new(),
                return_type: candidate.return_type.clone(),
                lowering: callable.lowering.clone(),
                requirements: candidate.requirements,
            }),
            return_type: candidate.return_type,
        };
        self.model.calls.push(call);
    }

    fn lookup_native_instance(
        &mut self,
        path: &ast::Path,
        expected: Option<&TypeRef>,
        context: &FunctionContext,
        span: Span,
    ) -> NativeInstanceLookup {
        let Some(type_path) = self.native_path(&path.segments, &context.namespace, false, span)
        else {
            return NativeInstanceLookup::NotNative;
        };
        let Some(binding) = self.bindings.type_by_path(type_path) else {
            self.push(
                "RES021",
                format!("native type `{type_path}` has no callable metadata"),
                span,
            );
            return NativeInstanceLookup::Invalid;
        };
        if binding.type_parameters.is_empty() {
            return NativeInstanceLookup::Resolved(NativeInstance {
                type_path,
                arguments: Vec::new(),
            });
        }
        if let Some(TypeRef::Native {
            path: expected_path,
            arguments,
        }) = expected.map(canonical_ref)
            && *expected_path == type_path
            && arguments.len() == binding.type_parameters.len()
        {
            return NativeInstanceLookup::Resolved(NativeInstance {
                type_path,
                arguments: arguments.clone(),
            });
        }
        self.push(
            "RES031",
            format!(
                "generic constructor `{}` requires an expected target type",
                path.display()
            ),
            span,
        );
        NativeInstanceLookup::Invalid
    }

    fn resolve_type(&mut self, ty: &ast::Type, namespace: &[String], allow_auto: bool) -> TypeRef {
        let value = match &ty.kind {
            TypeKind::Inferred if allow_auto => TypeRef::Error,
            TypeKind::Inferred => {
                self.push(
                    "RES032",
                    "`auto` is not valid in this type position".to_owned(),
                    ty.span,
                );
                TypeRef::Error
            }
            TypeKind::Error => TypeRef::Error,
            TypeKind::Named(named) => {
                let segments = &named.path.segments;
                if let Some(primitive) = primitive_type(segments) {
                    if !named.arguments.is_empty() {
                        self.push(
                            "RES033",
                            format!(
                                "primitive type `{}` cannot have type arguments",
                                named.path.display()
                            ),
                            ty.span,
                        );
                    }
                    primitive
                } else {
                    let arguments = named
                        .arguments
                        .iter()
                        .map(|argument| self.resolve_type(argument, namespace, false))
                        .collect::<Vec<_>>();
                    let Some(path) = self.native_path(segments, namespace, true, ty.span) else {
                        return TypeRef::Error;
                    };
                    let expected_arity = self.bindings.type_by_path(path).map_or_else(
                        || native_container_arity(path),
                        |binding| Some(binding.type_parameters.len()),
                    );
                    if expected_arity == Some(arguments.len()) {
                        TypeRef::Native { path, arguments }
                    } else {
                        self.push(
                            "RES034",
                            format!(
                                "native type `{path}` expects {} type argument(s), found {}",
                                expected_arity.unwrap_or(0),
                                arguments.len()
                            ),
                            ty.span,
                        );
                        TypeRef::Error
                    }
                }
            }
        };
        if ty.is_reference {
            if value == TypeRef::Void {
                self.push(
                    "RES035",
                    "`void` cannot be used as a reference target".to_owned(),
                    ty.span,
                );
                TypeRef::Error
            } else {
                TypeRef::Reference {
                    mutable: !ty.is_const,
                    target: Box::new(value),
                }
            }
        } else {
            value
        }
    }

    fn native_path(
        &mut self,
        segments: &[String],
        namespace: &[String],
        diagnose_unknown: bool,
        span: Span,
    ) -> Option<&'static str> {
        let candidates = if segments.first().is_some_and(|segment| segment == "rust") {
            vec![segments.to_vec()]
        } else if segments.len() == 1 {
            self.imports.candidates(namespace, &segments[0])
        } else {
            Vec::new()
        };
        let known = candidates
            .iter()
            .filter_map(|candidate| known_native_path(candidate, self.bindings))
            .collect::<BTreeSet<_>>();
        if known.len() == 1 {
            return known.into_iter().next();
        }
        if known.len() > 1 {
            self.push(
                "RES036",
                format!("native name `{}` is ambiguous", segments.join("::")),
                span,
            );
            return None;
        }
        if diagnose_unknown {
            self.push(
                "RES037",
                format!("unresolved type `{}`", segments.join("::")),
                span,
            );
        }
        None
    }

    fn function_candidates(&self, path: &ast::Path, namespace: &[String]) -> Vec<FunctionId> {
        let mut paths = Vec::new();
        if path
            .segments
            .first()
            .is_some_and(|segment| segment == "crate")
        {
            paths.push(path.segments[1..].to_vec());
        } else if path.segments.len() > 1 {
            let mut relative = namespace.to_vec();
            relative.extend(path.segments.iter().cloned());
            paths.push(relative);
            paths.push(path.segments.clone());
        } else if let Some(name) = path.segments.first() {
            paths.extend(self.imports.candidates(namespace, name));
            for depth in (0..=namespace.len()).rev() {
                let mut candidate = namespace[..depth].to_vec();
                candidate.push(name.clone());
                paths.push(candidate);
            }
        }
        paths.sort();
        paths.dedup();
        let mut ids = paths
            .iter()
            .filter_map(|candidate| self.function_sets.get(candidate))
            .flatten()
            .copied()
            .collect::<Vec<_>>();
        ids.sort();
        ids.dedup();
        ids
    }

    fn insert_variable(
        &mut self,
        scope: &mut BTreeMap<String, Variable>,
        name: &str,
        variable: Variable,
        span: Span,
    ) {
        if scope.insert(name.to_owned(), variable).is_some() {
            self.push(
                "RES038",
                format!("duplicate binding `{name}` in the same scope"),
                span,
            );
        }
    }

    fn record_expression(&mut self, span: Span, info: ExpressionInfo, call: Option<ResolvedCall>) {
        if let Some(call) = &call {
            self.model.calls.push(call.clone());
        }
        self.model.expressions.push(ExpressionResolution {
            span,
            ty: info.ty,
            category: info.category,
            call,
        });
    }

    fn push(&mut self, code: &'static str, message: String, span: Span) {
        self.diagnostics
            .push(Diagnostic::semantic(code, message, span));
    }
}

fn instantiate_callable(
    binding: &NativeTypeBinding,
    arguments: &[TypeRef],
    callable: &CallableBinding,
) -> ConcreteNativeCandidate {
    let substitutions = binding
        .type_parameters
        .iter()
        .copied()
        .zip(arguments.iter().cloned())
        .collect::<BTreeMap<_, _>>();
    ConcreteNativeCandidate {
        callable: callable.clone(),
        parameter_types: callable
            .parameters
            .iter()
            .map(|parameter| substitute(&parameter.ty, &substitutions))
            .collect(),
        return_type: substitute(&callable.return_type, &substitutions),
        requirements: callable
            .requirements
            .iter()
            .map(|requirement| ResolvedTraitRequirement {
                ty: substitutions
                    .get(requirement.parameter)
                    .cloned()
                    .unwrap_or(TypeRef::Error),
                rust_trait: requirement.rust_trait,
            })
            .collect(),
    }
}

fn substitute(ty: &TypeRef, substitutions: &BTreeMap<&'static str, TypeRef>) -> TypeRef {
    match ty {
        TypeRef::Parameter(name) => substitutions.get(name).cloned().unwrap_or(TypeRef::Error),
        TypeRef::Native { path, arguments } => TypeRef::Native {
            path,
            arguments: arguments
                .iter()
                .map(|argument| substitute(argument, substitutions))
                .collect(),
        },
        TypeRef::Reference { mutable, target } => TypeRef::Reference {
            mutable: *mutable,
            target: Box::new(substitute(target, substitutions)),
        },
        concrete => concrete.clone(),
    }
}

fn qualify_declaration_path(namespace: &[String], name: &[String]) -> Vec<String> {
    if name.first().is_some_and(|segment| segment == "crate") {
        name[1..].to_vec()
    } else {
        let mut path = namespace.to_vec();
        path.extend(name.iter().cloned());
        path
    }
}

fn parameter_mutability(parameter: &ast::Parameter, ty: &TypeRef) -> bool {
    match ty {
        TypeRef::Reference { mutable, .. } => *mutable,
        _ => !parameter.ty.is_const,
    }
}

fn canonical(ty: &TypeRef) -> TypeRef {
    match ty {
        TypeRef::Reference { target, .. } => target.as_ref().clone(),
        _ => ty.clone(),
    }
}

fn canonical_ref(ty: &TypeRef) -> &TypeRef {
    match ty {
        TypeRef::Reference { target, .. } => target,
        _ => ty,
    }
}

fn primitive_type(segments: &[String]) -> Option<TypeRef> {
    if segments.len() != 1 {
        return None;
    }
    match segments[0].as_str() {
        "void" => Some(TypeRef::Void),
        "bool" => Some(TypeRef::Bool),
        "char" => Some(TypeRef::Char),
        "i8" => Some(TypeRef::I8),
        "i16" => Some(TypeRef::I16),
        "i32" => Some(TypeRef::I32),
        "i64" => Some(TypeRef::I64),
        "i128" => Some(TypeRef::I128),
        "isize" => Some(TypeRef::Isize),
        "u8" => Some(TypeRef::U8),
        "u16" => Some(TypeRef::U16),
        "u32" => Some(TypeRef::U32),
        "u64" => Some(TypeRef::U64),
        "u128" => Some(TypeRef::U128),
        "usize" => Some(TypeRef::Usize),
        "f32" => Some(TypeRef::F32),
        "f64" => Some(TypeRef::F64),
        _ => None,
    }
}

fn literal_type(kind: LiteralKind, text: &str, expected: Option<&TypeRef>) -> TypeRef {
    match kind {
        LiteralKind::Boolean => TypeRef::Bool,
        LiteralKind::Character => TypeRef::Char,
        LiteralKind::String => TypeRef::native("rust::String", vec![]),
        LiteralKind::Float if text.ends_with('f') => TypeRef::F32,
        LiteralKind::Float => TypeRef::F64,
        LiteralKind::Integer => integer_suffix(text)
            .or_else(|| expected.filter(|ty| is_integer(ty)).cloned())
            .unwrap_or(TypeRef::I32),
    }
}

fn integer_suffix(text: &str) -> Option<TypeRef> {
    [
        ("i128", TypeRef::I128),
        ("isize", TypeRef::Isize),
        ("u128", TypeRef::U128),
        ("usize", TypeRef::Usize),
        ("i64", TypeRef::I64),
        ("u64", TypeRef::U64),
        ("i32", TypeRef::I32),
        ("u32", TypeRef::U32),
        ("i16", TypeRef::I16),
        ("u16", TypeRef::U16),
        ("i8", TypeRef::I8),
        ("u8", TypeRef::U8),
    ]
    .into_iter()
    .find_map(|(suffix, ty)| text.ends_with(suffix).then_some(ty))
}

fn known_native_path(segments: &[String], bindings: &NativeBindings) -> Option<&'static str> {
    let path = segments.join("::");
    bindings
        .type_by_path(&path)
        .map(|binding| binding.stainless_path)
        .or(match path.as_str() {
            "rust::Option" => Some("rust::Option"),
            "rust::Result" => Some("rust::Result"),
            _ => None,
        })
}

fn native_container_arity(path: &str) -> Option<usize> {
    match path {
        "rust::Option" => Some(1),
        "rust::Result" => Some(2),
        _ => None,
    }
}

fn is_integer(ty: &TypeRef) -> bool {
    matches!(
        ty,
        TypeRef::I8
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
    )
}

fn is_numeric(ty: &TypeRef) -> bool {
    is_integer(ty) || matches!(ty, TypeRef::F32 | TypeRef::F64)
}

fn is_copyable(ty: &TypeRef) -> bool {
    matches!(
        ty,
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
            | TypeRef::Reference { .. }
    )
}

fn is_assignment(operator: BinaryOperator) -> bool {
    matches!(
        operator,
        BinaryOperator::Assign
            | BinaryOperator::AddAssign
            | BinaryOperator::SubtractAssign
            | BinaryOperator::MultiplyAssign
            | BinaryOperator::DivideAssign
            | BinaryOperator::RemainderAssign
    )
}

fn is_comparison(operator: BinaryOperator) -> bool {
    matches!(
        operator,
        BinaryOperator::Equal
            | BinaryOperator::NotEqual
            | BinaryOperator::Less
            | BinaryOperator::LessEqual
            | BinaryOperator::Greater
            | BinaryOperator::GreaterEqual
    )
}

fn temporary(ty: TypeRef) -> ExpressionInfo {
    ExpressionInfo {
        ty,
        category: ValueCategory::Temporary,
    }
}

fn error_info() -> ExpressionInfo {
    temporary(TypeRef::Error)
}

fn info_for_return_type(ty: TypeRef) -> ExpressionInfo {
    let category = match &ty {
        TypeRef::Reference { mutable: true, .. } => ValueCategory::MutablePlace,
        TypeRef::Reference { mutable: false, .. } => ValueCategory::SharedPlace,
        _ => ValueCategory::Temporary,
    };
    ExpressionInfo { ty, category }
}

fn display_type(ty: &TypeRef) -> String {
    match ty {
        TypeRef::Error => "<error>".to_owned(),
        TypeRef::Void => "void".to_owned(),
        TypeRef::Bool => "bool".to_owned(),
        TypeRef::Char => "char".to_owned(),
        TypeRef::I8 => "i8".to_owned(),
        TypeRef::I16 => "i16".to_owned(),
        TypeRef::I32 => "i32".to_owned(),
        TypeRef::I64 => "i64".to_owned(),
        TypeRef::I128 => "i128".to_owned(),
        TypeRef::Isize => "isize".to_owned(),
        TypeRef::U8 => "u8".to_owned(),
        TypeRef::U16 => "u16".to_owned(),
        TypeRef::U32 => "u32".to_owned(),
        TypeRef::U64 => "u64".to_owned(),
        TypeRef::U128 => "u128".to_owned(),
        TypeRef::Usize => "usize".to_owned(),
        TypeRef::F32 => "f32".to_owned(),
        TypeRef::F64 => "f64".to_owned(),
        TypeRef::Parameter(name) => (*name).to_owned(),
        TypeRef::Native { path, arguments } if arguments.is_empty() => (*path).to_owned(),
        TypeRef::Native { path, arguments } => format!(
            "{path}<{}>",
            arguments
                .iter()
                .map(display_type)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        TypeRef::Reference { mutable, target } => {
            if *mutable {
                format!("{}&", display_type(target))
            } else {
                format!("const {}&", display_type(target))
            }
        }
    }
}

fn display_path(path: &[String]) -> String {
    path.join("::")
}

fn display_argument_types(arguments: &[ExpressionInfo]) -> String {
    arguments
        .iter()
        .map(|argument| display_type(&canonical(&argument.ty)))
        .collect::<Vec<_>>()
        .join(", ")
}

fn display_function_signature(function: &FunctionSymbol) -> String {
    format!(
        "{}({})",
        display_path(&function.path),
        function
            .parameters
            .iter()
            .map(|parameter| display_type(&parameter.ty))
            .collect::<Vec<_>>()
            .join(", ")
    )
}

fn display_native_signature(candidate: &ConcreteNativeCandidate) -> String {
    format!(
        "{}({})",
        candidate.callable.source_name,
        candidate
            .parameter_types
            .iter()
            .map(display_type)
            .collect::<Vec<_>>()
            .join(", ")
    )
}

fn display_native_instance(instance: &NativeInstance) -> String {
    display_type(&TypeRef::Native {
        path: instance.type_path,
        arguments: instance.arguments.clone(),
    })
}

fn instance_short_name(path: &str) -> &str {
    path.rsplit("::").next().unwrap_or(path)
}

fn call_style_name(style: CallStyle) -> &'static str {
    match style {
        CallStyle::Constructor => "constructor",
        CallStyle::AssociatedFunction => "associated function",
        CallStyle::Method => "method",
    }
}

fn operator_name(operator: PrefixOperator) -> &'static str {
    match operator {
        PrefixOperator::Plus => "+",
        PrefixOperator::Negate => "-",
        PrefixOperator::Not => "!",
        PrefixOperator::BitwiseNot => "~",
        PrefixOperator::Increment => "++",
        PrefixOperator::Decrement => "--",
    }
}

fn binary_name(operator: BinaryOperator) -> &'static str {
    match operator {
        BinaryOperator::Assign => "=",
        BinaryOperator::AddAssign => "+=",
        BinaryOperator::SubtractAssign => "-=",
        BinaryOperator::MultiplyAssign => "*=",
        BinaryOperator::DivideAssign => "/=",
        BinaryOperator::RemainderAssign => "%=",
        BinaryOperator::LogicalOr => "||",
        BinaryOperator::LogicalAnd => "&&",
        BinaryOperator::BitwiseOr => "|",
        BinaryOperator::BitwiseXor => "^",
        BinaryOperator::BitwiseAnd => "&",
        BinaryOperator::Equal => "==",
        BinaryOperator::NotEqual => "!=",
        BinaryOperator::Less => "<",
        BinaryOperator::LessEqual => "<=",
        BinaryOperator::Greater => ">",
        BinaryOperator::GreaterEqual => ">=",
        BinaryOperator::Add => "+",
        BinaryOperator::Subtract => "-",
        BinaryOperator::Multiply => "*",
        BinaryOperator::Divide => "/",
        BinaryOperator::Remainder => "%",
    }
}
