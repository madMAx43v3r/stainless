//! Move, borrow, and direct-reference-return validation.

use std::collections::{BTreeMap, BTreeSet};

use crate::Diagnostic;
use crate::ast::{
    self, BinaryOperator, ExpressionKind, ForClause, ForInitializer, Item, PrefixOperator,
    StatementKind,
};
use crate::interop::{Receiver, TypeRef};
use crate::resolution::{CallTarget, FunctionSymbol, Intrinsic, SemanticModel};

/// Validates ownership rules over an otherwise successfully resolved file.
#[must_use]
pub fn validate(source: &ast::SourceFile, semantics: &SemanticModel) -> Vec<Diagnostic> {
    let last_uses = collect_last_uses(source);
    let mut analyzer = Analyzer {
        semantics,
        last_uses: &last_uses,
        state: FlowState::default(),
        diagnostics: Vec::new(),
        emitted: BTreeSet::new(),
        return_borrow: None,
        returns_reference: false,
        loop_depth: 0,
        loops: Vec::new(),
        exceptions: Vec::new(),
    };
    analyzer.items(&source.items);
    analyzer
        .diagnostics
        .sort_by_key(|diagnostic| diagnostic.span);
    analyzer.diagnostics
}

type BindingId = usize;

#[derive(Clone, Copy, Debug)]
enum Availability {
    Available,
    Moved(ast::Span),
    MaybeMoved(ast::Span),
}

#[derive(Clone, Copy, Debug)]
struct Loan {
    owner: BindingId,
    mutable: bool,
}

#[derive(Clone, Debug)]
struct Binding {
    name: String,
    ty: TypeRef,
    availability: Availability,
    shared_borrows: usize,
    mutably_borrowed: bool,
    reference_loan: Option<Loan>,
    reference_loan_active: bool,
    declaration_span: ast::Span,
    last_use: Option<ast::Span>,
    declaration_loop_depth: usize,
}

#[derive(Clone, Debug, Default)]
struct FlowState {
    bindings: Vec<Binding>,
    scopes: Vec<BTreeMap<String, BindingId>>,
    scope_bindings: Vec<Vec<BindingId>>,
}

impl FlowState {
    fn push_scope(&mut self) {
        self.scopes.push(BTreeMap::new());
        self.scope_bindings.push(Vec::new());
    }

    fn pop_scope(&mut self) {
        let bindings = self
            .scope_bindings
            .pop()
            .expect("ownership scopes stay balanced");
        for id in bindings.into_iter().rev() {
            if self.bindings[id].reference_loan_active
                && let Some(loan) = self.bindings[id].reference_loan
            {
                self.release(loan);
                self.bindings[id].reference_loan_active = false;
            }
        }
        self.scopes.pop();
    }

    fn declare(
        &mut self,
        name: String,
        ty: TypeRef,
        reference_loan: Option<Loan>,
        declaration_span: ast::Span,
        last_use: Option<ast::Span>,
        loop_depth: usize,
    ) -> BindingId {
        let id = self.bindings.len();
        self.bindings.push(Binding {
            name: name.clone(),
            ty,
            availability: Availability::Available,
            shared_borrows: 0,
            mutably_borrowed: false,
            reference_loan,
            reference_loan_active: reference_loan.is_some(),
            declaration_span,
            last_use,
            declaration_loop_depth: loop_depth,
        });
        self.scopes
            .last_mut()
            .expect("a function always has an ownership scope")
            .insert(name, id);
        self.scope_bindings
            .last_mut()
            .expect("a function always has an ownership scope")
            .push(id);
        id
    }

    fn lookup(&self, name: &str) -> Option<BindingId> {
        self.scopes
            .iter()
            .rev()
            .find_map(|scope| scope.get(name).copied())
    }

    fn release(&mut self, loan: Loan) {
        let owner = &mut self.bindings[loan.owner];
        if loan.mutable {
            owner.mutably_borrowed = false;
        } else {
            owner.shared_borrows = owner.shared_borrows.saturating_sub(1);
        }
    }
}

#[derive(Clone, Copy)]
enum Usage {
    Read,
    Mutate,
    BorrowShared,
    BorrowMutable,
}

struct Analyzer<'a> {
    semantics: &'a SemanticModel,
    last_uses: &'a BTreeMap<ast::Span, ast::Span>,
    state: FlowState,
    diagnostics: Vec<Diagnostic>,
    emitted: BTreeSet<(&'static str, ast::Span)>,
    return_borrow: Option<BindingId>,
    returns_reference: bool,
    loop_depth: usize,
    loops: Vec<LoopContext>,
    exceptions: Vec<ExceptionContext>,
}

struct LoopContext {
    scope_depth: usize,
    break_states: Vec<FlowState>,
    continue_states: Vec<FlowState>,
}

struct ExceptionContext {
    scope_depth: usize,
    states: Vec<FlowState>,
}

impl Analyzer<'_> {
    fn items(&mut self, items: &[Item]) {
        for item in items {
            match item {
                Item::Namespace(namespace) => self.items(&namespace.items),
                Item::Struct(structure) => {
                    for constructor in &structure.constructors {
                        if constructor.body.is_some() {
                            self.constructor(constructor);
                        }
                    }
                    for function in &structure.functions {
                        if function.body.is_some() {
                            self.function(function);
                        }
                    }
                }
                Item::Constructor(constructor) if constructor.body.is_some() => {
                    self.constructor(constructor);
                }
                Item::Function(function) if function.body.is_some() => self.function(function),
                Item::Constructor(_) | Item::Function(_) | Item::Use(_) => {}
            }
        }
    }

    fn constructor(&mut self, constructor: &ast::Constructor) {
        let Some(symbol) = self.semantics.constructor_at(constructor.span).cloned() else {
            return;
        };
        self.state = FlowState::default();
        self.state.push_scope();
        for (syntax, resolved) in constructor.parameters.iter().zip(&symbol.parameters) {
            self.state.declare(
                syntax.name.clone(),
                resolved.ty.clone(),
                None,
                syntax.span,
                self.last_uses.get(&syntax.span).copied(),
                self.loop_depth,
            );
        }
        for initializer in &constructor.initializers {
            for argument in &initializer.arguments {
                self.expression(argument, Usage::Read);
            }
        }
        self.return_borrow = None;
        self.returns_reference = false;
        self.exceptions.clear();
        if let Some(body) = &constructor.body {
            self.block(body, false);
        }
        self.state.pop_scope();
    }

    fn function(&mut self, function: &ast::Function) {
        let Some(symbol) = self.semantics.function_at(function.span).cloned() else {
            return;
        };
        self.state = FlowState::default();
        self.state.push_scope();
        let mut parameter_ids = Vec::new();
        for (syntax, resolved) in function.parameters.iter().zip(&symbol.parameters) {
            parameter_ids.push(self.state.declare(
                syntax.name.clone(),
                resolved.ty.clone(),
                None,
                syntax.span,
                self.last_uses.get(&syntax.span).copied(),
                self.loop_depth,
            ));
        }
        self.return_borrow =
            return_borrow_parameter(&symbol).and_then(|index| parameter_ids.get(index).copied());
        self.returns_reference = symbol.return_type.is_reference();
        self.exceptions.clear();
        if let Some(body) = &function.body {
            self.block(body, false);
        }
        self.state.pop_scope();
        self.return_borrow = None;
        self.returns_reference = false;
    }

    fn block(&mut self, block: &ast::Block, create_scope: bool) -> bool {
        if create_scope {
            self.state.push_scope();
        }
        let mut reachable = true;
        for statement in &block.statements {
            if !reachable {
                break;
            }
            reachable = self.statement(statement);
            self.release_expired_loans(statement.span.end);
        }
        if create_scope {
            self.state.pop_scope();
        }
        reachable
    }

    fn statement(&mut self, statement: &ast::Statement) -> bool {
        match &statement.kind {
            StatementKind::Block(block) => self.block(block, true),
            StatementKind::Local(local) => {
                self.local(local);
                true
            }
            StatementKind::Return(value) => {
                if let Some(value) = value {
                    let origin = self.expression(value, Usage::Read);
                    if self.returns_reference {
                        self.validate_reference_return(origin, value.span);
                    }
                }
                false
            }
            StatementKind::Throw(value) => {
                if let Some(value) = value {
                    self.expression(value, Usage::Read);
                }
                self.capture_exception_state();
                false
            }
            StatementKind::Try(try_statement) => {
                let baseline = self.state.clone();
                self.exceptions.push(ExceptionContext {
                    scope_depth: baseline.scopes.len(),
                    states: Vec::new(),
                });
                let (try_state, try_reachable) =
                    self.analyze_block_branch(&try_statement.body, baseline.clone(), None);
                let exception_states = self
                    .exceptions
                    .pop()
                    .expect("a try statement pushed an exception context")
                    .states;
                let catch_baseline = if exception_states.is_empty() {
                    baseline.clone()
                } else {
                    merge_many_states(&baseline, &exception_states)
                };
                let mut continuing = Vec::new();
                if try_reachable {
                    continuing.push(try_state);
                }
                for catch in &try_statement.catches {
                    let binding = catch
                        .binding
                        .as_ref()
                        .and_then(|binding| self.semantics.binding(binding.span).cloned());
                    let (state, reachable) = self.analyze_block_branch(
                        &catch.body,
                        catch_baseline.clone(),
                        binding.as_ref(),
                    );
                    if reachable {
                        continuing.push(state);
                    }
                }
                if continuing.is_empty() {
                    self.state = baseline;
                    false
                } else {
                    self.state = merge_many_states(&baseline, &continuing);
                    true
                }
            }
            StatementKind::If(if_statement) => {
                self.expression(&if_statement.condition, Usage::Read);
                self.if_statement(if_statement)
            }
            StatementKind::For(for_statement) => {
                self.for_statement(for_statement);
                true
            }
            StatementKind::Break => {
                self.capture_loop_exit(false);
                false
            }
            StatementKind::Continue => {
                self.capture_loop_exit(true);
                false
            }
            StatementKind::Expression(expression) => {
                self.expression(expression, Usage::Read);
                true
            }
            StatementKind::Empty | StatementKind::Error => true,
        }
    }

    fn local(&mut self, local: &ast::LocalDeclaration) {
        let Some(binding) = self.semantics.binding(local.span) else {
            return;
        };
        let reference_loan = if binding.ty.is_reference() {
            local.initializer.as_ref().and_then(|initializer| {
                let mutable = matches!(binding.ty, TypeRef::Reference { mutable: true, .. });
                let origin = self.expression(
                    initializer,
                    if mutable {
                        Usage::BorrowMutable
                    } else {
                        Usage::BorrowShared
                    },
                );
                self.acquire_persistent_loan(origin, mutable, initializer.span)
            })
        } else {
            if let Some(initializer) = &local.initializer {
                self.expression(initializer, Usage::Read);
                if self
                    .semantics
                    .rust_result_adaptation(initializer.span)
                    .is_some()
                {
                    self.capture_exception_state();
                }
            } else if self
                .semantics
                .call(local.span)
                .is_some_and(|call| !call.throws.is_empty())
            {
                self.capture_exception_state();
            }
            None
        };
        self.state.declare(
            local.name.clone(),
            binding.ty.clone(),
            reference_loan,
            local.span,
            self.last_uses.get(&local.span).copied(),
            self.loop_depth,
        );
    }

    fn if_statement(&mut self, statement: &ast::IfStatement) -> bool {
        let baseline = self.state.clone();
        let (then_state, then_reachable) =
            self.analyze_branch(&statement.then_branch, baseline.clone());
        let (else_state, else_reachable) = statement.else_branch.as_ref().map_or_else(
            || (baseline.clone(), true),
            |branch| self.analyze_branch(branch, baseline.clone()),
        );
        match (then_reachable, else_reachable) {
            (true, true) => {
                self.state = merge_states(&baseline, &then_state, &else_state);
                true
            }
            (true, false) => {
                self.state = then_state;
                true
            }
            (false, true) => {
                self.state = else_state;
                true
            }
            (false, false) => {
                self.state = baseline;
                false
            }
        }
    }

    fn analyze_branch(
        &mut self,
        statement: &ast::Statement,
        state: FlowState,
    ) -> (FlowState, bool) {
        let saved = std::mem::replace(&mut self.state, state);
        self.state.push_scope();
        let reachable = self.statement(statement);
        self.release_expired_loans(statement.span.end);
        self.state.pop_scope();
        let result = std::mem::replace(&mut self.state, saved);
        (result, reachable)
    }

    fn analyze_block_branch(
        &mut self,
        block: &ast::Block,
        state: FlowState,
        binding: Option<&crate::resolution::BindingResolution>,
    ) -> (FlowState, bool) {
        let saved = std::mem::replace(&mut self.state, state);
        self.state.push_scope();
        if let Some(binding) = binding {
            self.state.declare(
                binding.name.clone(),
                binding.ty.clone(),
                None,
                binding.span,
                self.last_uses.get(&binding.span).copied(),
                self.loop_depth,
            );
        }
        let reachable = self.block(block, false);
        self.state.pop_scope();
        let result = std::mem::replace(&mut self.state, saved);
        (result, reachable)
    }

    fn for_statement(&mut self, statement: &ast::ForStatement) {
        self.state.push_scope();
        self.loop_depth += 1;
        self.loops.push(LoopContext {
            scope_depth: self.state.scopes.len(),
            break_states: Vec::new(),
            continue_states: Vec::new(),
        });
        match &statement.clause {
            ForClause::Classic(classic) => self.classic_for(classic, &statement.body),
            ForClause::Range(range) => self.range_for(range, &statement.body),
            ForClause::Error => {}
        }
        self.loops.pop();
        self.loop_depth -= 1;
        self.state.pop_scope();
        self.release_expired_loans(statement.body.span.end);
    }

    fn classic_for(&mut self, clause: &ast::ClassicForClause, body: &ast::Statement) {
        if let Some(initializer) = &clause.initializer {
            match initializer {
                ForInitializer::Local(local) => self.local(local),
                ForInitializer::Expression(expression) => {
                    self.expression(expression, Usage::Read);
                }
            }
        }
        if let Some(condition) = &clause.condition {
            self.expression(condition, Usage::Read);
        }
        let baseline = self.state.clone();
        let first_state =
            self.loop_iteration(body, clause.update.as_ref(), clause.condition.as_ref());
        let repeated_state = first_state.as_ref().and_then(|first_state| {
            self.state = first_state.clone();
            self.loop_iteration(body, clause.update.as_ref(), clause.condition.as_ref())
        });
        let mut exits = vec![baseline.clone()];
        exits.extend(first_state);
        exits.extend(repeated_state);
        exits.extend(
            self.loops
                .last()
                .expect("classic loop has an ownership context")
                .break_states
                .iter()
                .cloned(),
        );
        self.state = merge_many_states(&baseline, &exits);
    }

    fn loop_iteration(
        &mut self,
        body: &ast::Statement,
        update: Option<&ast::Expression>,
        condition: Option<&ast::Expression>,
    ) -> Option<FlowState> {
        let continue_start = self
            .loops
            .last()
            .expect("loop iteration has an ownership context")
            .continue_states
            .len();
        self.state.push_scope();
        let reachable = self.statement(body);
        self.state.pop_scope();
        let mut continuing = Vec::new();
        if reachable {
            continuing.push(self.state.clone());
        }
        let continue_states = self
            .loops
            .last_mut()
            .expect("loop iteration has an ownership context")
            .continue_states
            .split_off(continue_start);
        continuing.extend(continue_states);
        let baseline = continuing.first()?.clone();
        let continuing = continuing
            .into_iter()
            .map(|state| self.analyze_loop_tail(state, update, condition))
            .collect::<Vec<_>>();
        Some(merge_many_states(&baseline, &continuing))
    }

    fn range_for(&mut self, range: &ast::RangeForClause, body: &ast::Statement) {
        let Some(binding) = self.semantics.binding(range.ty.span).cloned() else {
            return;
        };
        let moved_range = is_move_call(self.semantics, &range.iterable);
        let mutable = matches!(binding.ty, TypeRef::Reference { mutable: true, .. });
        let borrowed = binding.ty.is_reference() || !moved_range;
        let origin = self.expression(
            &range.iterable,
            if moved_range {
                Usage::Read
            } else if mutable {
                Usage::BorrowMutable
            } else {
                Usage::BorrowShared
            },
        );
        let loop_loan = borrowed
            .then(|| self.acquire_persistent_loan(origin, mutable, range.iterable.span))
            .flatten();
        let baseline = self.state.clone();
        let first_state = self.range_iteration(&binding, body);
        let repeated_state = first_state.as_ref().and_then(|first_state| {
            self.state = first_state.clone();
            self.range_iteration(&binding, body)
        });
        let mut exits = vec![baseline.clone()];
        exits.extend(first_state);
        exits.extend(repeated_state);
        exits.extend(
            self.loops
                .last()
                .expect("range loop has an ownership context")
                .break_states
                .iter()
                .cloned(),
        );
        self.state = merge_many_states(&baseline, &exits);
        if let Some(loan) = loop_loan {
            self.state.release(loan);
        }
    }

    fn range_iteration(
        &mut self,
        binding: &crate::resolution::BindingResolution,
        body: &ast::Statement,
    ) -> Option<FlowState> {
        let continue_start = self
            .loops
            .last()
            .expect("range iteration has an ownership context")
            .continue_states
            .len();
        self.state.push_scope();
        self.state.declare(
            binding.name.clone(),
            binding.ty.clone(),
            None,
            binding.span,
            self.last_uses.get(&binding.span).copied(),
            self.loop_depth,
        );
        let reachable = self.statement(body);
        self.state.pop_scope();
        let mut continuing = Vec::new();
        if reachable {
            continuing.push(self.state.clone());
        }
        let continue_states = self
            .loops
            .last_mut()
            .expect("range iteration has an ownership context")
            .continue_states
            .split_off(continue_start);
        continuing.extend(continue_states);
        let baseline = continuing.first()?.clone();
        Some(merge_many_states(&baseline, &continuing))
    }

    fn analyze_loop_tail(
        &mut self,
        state: FlowState,
        update: Option<&ast::Expression>,
        condition: Option<&ast::Expression>,
    ) -> FlowState {
        let saved = std::mem::replace(&mut self.state, state);
        if let Some(update) = update {
            self.expression(update, Usage::Read);
        }
        if let Some(condition) = condition {
            self.expression(condition, Usage::Read);
        }
        std::mem::replace(&mut self.state, saved)
    }

    fn capture_loop_exit(&mut self, is_continue: bool) {
        let Some(scope_depth) = self.loops.last().map(|context| context.scope_depth) else {
            return;
        };
        let mut state = self.state.clone();
        while state.scopes.len() > scope_depth {
            state.pop_scope();
        }
        let context = self
            .loops
            .last_mut()
            .expect("loop context was checked above");
        if is_continue {
            context.continue_states.push(state);
        } else {
            context.break_states.push(state);
        }
    }

    fn capture_exception_state(&mut self) {
        for context in &mut self.exceptions {
            let mut state = self.state.clone();
            while state.scopes.len() > context.scope_depth {
                state.pop_scope();
            }
            context.states.push(state);
        }
    }

    fn expression(&mut self, expression: &ast::Expression, usage: Usage) -> Option<BindingId> {
        match &expression.kind {
            ExpressionKind::Name(path) => {
                let id = path
                    .segments
                    .first()
                    .and_then(|name| self.state.lookup(name))?;
                self.check_usage(id, usage, expression.span);
                Some(id)
            }
            ExpressionKind::GenericName { .. }
            | ExpressionKind::Literal(_)
            | ExpressionKind::Error => None,
            ExpressionKind::Lambda { .. } => {
                let mut loans = Vec::new();
                self.callback_argument(expression, &mut loans);
                for loan in loans.into_iter().rev() {
                    self.state.release(loan);
                }
                None
            }
            ExpressionKind::Parenthesized(inner) => self.expression(inner, usage),
            ExpressionKind::Prefix { operator, operand } => {
                self.expression(
                    operand,
                    if matches!(
                        operator,
                        PrefixOperator::Increment | PrefixOperator::Decrement
                    ) {
                        Usage::Mutate
                    } else {
                        Usage::Read
                    },
                );
                None
            }
            ExpressionKind::Postfix { operand, .. } => {
                self.expression(operand, Usage::Mutate);
                None
            }
            ExpressionKind::Binary {
                left,
                operator,
                right,
            } => {
                self.binary(left, *operator, right);
                None
            }
            ExpressionKind::Call { arguments, .. } => self.call(expression, arguments),
            ExpressionKind::MacroCall { callee, arguments } => {
                let writes_destination = callee
                    .segments
                    .last()
                    .is_some_and(|name| matches!(name.as_str(), "write" | "writeln"));
                for (index, argument) in arguments.iter().enumerate() {
                    self.expression(
                        argument,
                        if writes_destination && index == 0 {
                            Usage::BorrowMutable
                        } else {
                            Usage::Read
                        },
                    );
                }
                if writes_destination {
                    self.capture_exception_state();
                }
                None
            }
            ExpressionKind::Aggregate { initializers, .. } => {
                for initializer in initializers {
                    self.expression(initializer, Usage::Read);
                    if self
                        .semantics
                        .rust_result_adaptation(initializer.span)
                        .is_some()
                    {
                        self.capture_exception_state();
                    }
                }
                None
            }
            ExpressionKind::JsonArray { elements } => {
                for element in elements {
                    self.expression(element, Usage::Read);
                }
                None
            }
            ExpressionKind::JsonObject { members } => {
                for (_, value) in members {
                    self.expression(value, Usage::Read);
                }
                None
            }
            ExpressionKind::Field { receiver, .. } => self.expression(receiver, usage),
            ExpressionKind::Index { receiver, index } => {
                self.expression(receiver, usage);
                self.expression(index, Usage::Read);
                None
            }
        }
    }

    fn binary(
        &mut self,
        left: &ast::Expression,
        operator: BinaryOperator,
        right: &ast::Expression,
    ) {
        if operator == BinaryOperator::Assign {
            self.expression(right, Usage::Read);
            if self.semantics.rust_result_adaptation(right.span).is_some() {
                self.capture_exception_state();
            }
            if is_json_mutation_place(left, self.semantics) {
                self.capture_exception_state();
            }
            self.write(left);
        } else if is_compound_assignment(operator) {
            self.expression(right, Usage::Read);
            self.expression(left, Usage::Mutate);
        } else if matches!(
            operator,
            BinaryOperator::LogicalAnd | BinaryOperator::LogicalOr
        ) {
            self.expression(left, Usage::Read);
            let baseline = self.state.clone();
            self.expression(right, Usage::Read);
            let with_right = self.state.clone();
            self.state = merge_states(&baseline, &baseline, &with_right);
        } else {
            self.expression(left, Usage::Read);
            self.expression(right, Usage::Read);
        }
    }

    fn write(&mut self, expression: &ast::Expression) {
        if let ExpressionKind::Parenthesized(inner) = &expression.kind {
            self.write(inner);
            return;
        }
        if let ExpressionKind::Name(path) = &expression.kind
            && let Some(id) = path
                .segments
                .first()
                .and_then(|name| self.state.lookup(name))
        {
            if self.state.bindings[id].ty.is_reference() {
                self.check_usage(id, Usage::Mutate, expression.span);
            } else if self.check_borrow_conflict(id, Usage::Mutate, expression.span) {
                self.state.bindings[id].availability = Availability::Available;
            }
            return;
        }
        self.expression(expression, Usage::Mutate);
    }

    #[allow(clippy::too_many_lines)]
    fn call(
        &mut self,
        expression: &ast::Expression,
        arguments: &[ast::Expression],
    ) -> Option<BindingId> {
        let call = self
            .semantics
            .expression(expression.span)
            .and_then(|resolution| resolution.call.as_ref())
            .cloned()?;
        let throws = !call.throws.is_empty();
        let result = match &call.target {
            CallTarget::Intrinsic(Intrinsic::Move) => {
                let argument = arguments.first()?;
                let id = named_binding(argument, &self.state)?;
                if is_copyable(&self.state.bindings[id].ty) {
                    self.check_usage(id, Usage::Read, argument.span);
                } else {
                    self.mark_moved(id, expression.span);
                }
                None
            }
            CallTarget::Intrinsic(Intrinsic::MakeUnique { construction, .. }) => {
                self.construction_arguments(construction, arguments);
                None
            }
            CallTarget::Intrinsic(Intrinsic::StoredFunctionCall { mutable }) => {
                let ExpressionKind::Call { callee, .. } = &expression.kind else {
                    return None;
                };
                self.expression(callee, if *mutable { Usage::Mutate } else { Usage::Read });
                let TypeRef::Function(function) = self
                    .semantics
                    .expression(callee.span)
                    .map(|resolution| canonical_ref(&resolution.ty))?
                else {
                    return None;
                };
                self.call_arguments(arguments, function.parameters.iter());
                None
            }
            CallTarget::Intrinsic(Intrinsic::UnwrapRustResult { .. }) => {
                let receiver = call_receiver(expression)?;
                if let Some(id) = named_binding(receiver, &self.state) {
                    self.mark_moved(id, expression.span);
                } else {
                    self.expression(receiver, Usage::Read);
                }
                None
            }
            CallTarget::Intrinsic(
                Intrinsic::PrimitiveCast { .. }
                | Intrinsic::JsonCast { .. }
                | Intrinsic::JsonWrap
                | Intrinsic::ExceptionRoot { .. },
            ) => {
                if let Some(argument) = arguments.first() {
                    self.expression(argument, Usage::Read);
                }
                None
            }
            CallTarget::Intrinsic(Intrinsic::ValueInitialization { .. }) => {
                if let Some(argument) = arguments.first() {
                    self.expression(argument, Usage::Read);
                    if self
                        .semantics
                        .rust_result_adaptation(argument.span)
                        .is_some()
                    {
                        self.capture_exception_state();
                    }
                }
                None
            }
            CallTarget::Intrinsic(Intrinsic::StructAggregate { .. }) => {
                for argument in arguments {
                    self.expression(argument, Usage::Read);
                }
                None
            }
            CallTarget::Stainless(id) => {
                let function = self.semantics.function(*id)?.clone();
                let mut receiver_loan = None;
                let mut receiver_origin = None;
                if let Some(receiver) = &function.receiver
                    && let ExpressionKind::Call { callee, .. } = &expression.kind
                    && let ExpressionKind::Field {
                        receiver: syntax_receiver,
                        ..
                    } = &callee.kind
                {
                    let usage = if receiver.mutable {
                        Usage::BorrowMutable
                    } else {
                        Usage::BorrowShared
                    };
                    let origin = self.expression(syntax_receiver, usage);
                    receiver_origin = origin;
                    receiver_loan =
                        self.acquire_temporary_loan(origin, receiver.mutable, syntax_receiver.span);
                }
                let origins = self.call_arguments(
                    arguments,
                    function.parameters.iter().map(|parameter| &parameter.ty),
                );
                if let Some(loan) = receiver_loan {
                    self.state.release(loan);
                }
                if function.receiver.is_some() && function.return_type.is_reference() {
                    receiver_origin
                } else {
                    return_borrow_parameter(&function)
                        .and_then(|index| origins.get(index).copied().flatten())
                }
            }
            CallTarget::Constructor(id) => {
                let constructor = self.semantics.constructor(*id)?.clone();
                self.call_arguments(
                    arguments,
                    constructor.parameters.iter().map(|parameter| &parameter.ty),
                );
                None
            }
            CallTarget::Native(native) => {
                let mut receiver_loan = None;
                if native.style == crate::interop::CallStyle::Method
                    && let ExpressionKind::Call { callee, .. } = &expression.kind
                    && let ExpressionKind::Field { receiver, .. } = &callee.kind
                {
                    let receiver_mode = native.receiver.unwrap_or(Receiver::Shared);
                    let receiver_usage = match receiver_mode {
                        Receiver::Shared => Usage::BorrowShared,
                        Receiver::Mutable => Usage::BorrowMutable,
                        Receiver::Value => Usage::Read,
                    };
                    let origin = self.expression(receiver, receiver_usage);
                    if receiver_mode != Receiver::Value {
                        receiver_loan = self.acquire_temporary_loan(
                            origin,
                            receiver_mode == Receiver::Mutable,
                            receiver.span,
                        );
                    }
                }
                self.call_arguments(arguments, native.parameter_types.iter());
                if let Some(loan) = receiver_loan {
                    self.state.release(loan);
                }
                None
            }
        };
        if throws {
            self.capture_exception_state();
        }
        result
    }

    fn construction_arguments(
        &mut self,
        construction: &crate::resolution::ResolvedCall,
        arguments: &[ast::Expression],
    ) {
        match &construction.target {
            CallTarget::Constructor(id) => {
                if let Some(constructor) = self.semantics.constructor(*id) {
                    self.call_arguments(
                        arguments,
                        constructor.parameters.iter().map(|parameter| &parameter.ty),
                    );
                }
            }
            CallTarget::Native(native) => {
                self.call_arguments(arguments, native.parameter_types.iter());
            }
            CallTarget::Intrinsic(Intrinsic::ValueInitialization { target }) => {
                self.call_arguments(arguments, std::iter::once(target));
            }
            _ => {
                for argument in arguments {
                    self.expression(argument, Usage::Read);
                }
            }
        }
    }

    fn call_arguments<'a>(
        &mut self,
        arguments: &[ast::Expression],
        expected: impl Iterator<Item = &'a TypeRef>,
    ) -> Vec<Option<BindingId>> {
        let mut loans = Vec::new();
        let origins = arguments
            .iter()
            .zip(expected)
            .map(|(argument, expected)| {
                if expected.is_callback() {
                    self.callback_argument(argument, &mut loans);
                    None
                } else if let TypeRef::Reference { mutable, .. } = expected {
                    let origin = self.expression(
                        argument,
                        if *mutable {
                            Usage::BorrowMutable
                        } else {
                            Usage::BorrowShared
                        },
                    );
                    if let Some(loan) = self.acquire_temporary_loan(origin, *mutable, argument.span)
                    {
                        loans.push(loan);
                    }
                    origin
                } else {
                    self.expression(argument, Usage::Read)
                }
            })
            .collect();
        for loan in loans.into_iter().rev() {
            self.state.release(loan);
        }
        origins
    }

    fn callback_argument(&mut self, argument: &ast::Expression, loans: &mut Vec<Loan>) {
        let Some(callback) = self.semantics.callback(argument.span) else {
            return;
        };
        let crate::resolution::CallbackTarget::Lambda { captures } = &callback.target else {
            return;
        };
        let ExpressionKind::Lambda {
            captures: syntax_captures,
            ..
        } = &argument.kind
        else {
            return;
        };
        for (capture, syntax_capture) in captures.iter().zip(syntax_captures) {
            match capture.mode {
                crate::resolution::LambdaCaptureMode::Copy => {
                    if let Some(id) = self.state.lookup(&capture.name) {
                        self.check_usage(id, Usage::Read, argument.span);
                    }
                }
                crate::resolution::LambdaCaptureMode::Initialize => {
                    if let ast::LambdaCaptureKind::Initialize(initializer) = &syntax_capture.kind {
                        self.expression(initializer, Usage::Read);
                    }
                }
                crate::resolution::LambdaCaptureMode::Borrow { mutable } => {
                    let Some(id) = self.state.lookup(&capture.name) else {
                        continue;
                    };
                    self.check_usage(
                        id,
                        if mutable {
                            Usage::BorrowMutable
                        } else {
                            Usage::BorrowShared
                        },
                        argument.span,
                    );
                    if let Some(loan) =
                        self.acquire_temporary_loan(Some(id), mutable, argument.span)
                    {
                        loans.push(loan);
                    }
                }
            }
        }
    }

    fn check_usage(&mut self, id: BindingId, usage: Usage, span: ast::Span) {
        match self.state.bindings[id].availability {
            Availability::Available => {}
            Availability::Moved(move_span) => {
                self.push(
                    "OWN001",
                    format!(
                        "use of moved binding `{}`; it was moved at byte {}",
                        self.state.bindings[id].name, move_span.start
                    ),
                    span,
                );
                return;
            }
            Availability::MaybeMoved(move_span) => {
                self.push(
                    "OWN002",
                    format!(
                        "binding `{}` may have been moved on another control-flow path at byte {}",
                        self.state.bindings[id].name, move_span.start
                    ),
                    span,
                );
                return;
            }
        }
        self.check_borrow_conflict(id, usage, span);
    }

    fn check_borrow_conflict(&mut self, id: BindingId, usage: Usage, span: ast::Span) -> bool {
        let binding = &self.state.bindings[id];
        let conflict = match usage {
            Usage::Read | Usage::BorrowShared => binding.mutably_borrowed,
            Usage::Mutate | Usage::BorrowMutable => {
                binding.mutably_borrowed || binding.shared_borrows != 0
            }
        };
        if conflict {
            let action = match usage {
                Usage::Read => "read",
                Usage::Mutate => "mutate",
                Usage::BorrowShared => "borrow",
                Usage::BorrowMutable => "mutably borrow",
            };
            self.push(
                "OWN003",
                format!(
                    "cannot {action} `{}` while it has an active conflicting borrow",
                    binding.name
                ),
                span,
            );
            false
        } else {
            true
        }
    }

    fn mark_moved(&mut self, id: BindingId, span: ast::Span) {
        self.check_usage(id, Usage::Mutate, span);
        if matches!(
            self.state.bindings[id].availability,
            Availability::Available
        ) && !self.state.bindings[id].mutably_borrowed
            && self.state.bindings[id].shared_borrows == 0
        {
            self.state.bindings[id].availability = Availability::Moved(span);
        }
    }

    fn acquire_persistent_loan(
        &mut self,
        origin: Option<BindingId>,
        mutable: bool,
        span: ast::Span,
    ) -> Option<Loan> {
        let Some(origin) = origin else {
            self.push(
                "OWN004",
                "a local reference requires a stable source binding".to_owned(),
                span,
            );
            return None;
        };
        let owner = root_origin(&self.state, origin);
        let origin_loan_ends_here = {
            let binding = &self.state.bindings[origin];
            binding.reference_loan_active
                && binding.last_use.is_some_and(|last_use| {
                    span.start <= last_use.start && last_use.end <= span.end
                })
        };
        if origin_loan_ends_here {
            self.end_reference_loan(origin);
        }
        let usage = if mutable {
            Usage::BorrowMutable
        } else {
            Usage::BorrowShared
        };
        if !self.check_borrow_conflict(owner, usage, span) {
            return None;
        }
        let binding = &mut self.state.bindings[owner];
        if mutable {
            binding.mutably_borrowed = true;
        } else {
            binding.shared_borrows += 1;
        }
        Some(Loan { owner, mutable })
    }

    fn acquire_temporary_loan(
        &mut self,
        origin: Option<BindingId>,
        mutable: bool,
        span: ast::Span,
    ) -> Option<Loan> {
        let owner = origin?;
        let usage = if mutable {
            Usage::BorrowMutable
        } else {
            Usage::BorrowShared
        };
        if !self.check_borrow_conflict(owner, usage, span) {
            return None;
        }
        let binding = &mut self.state.bindings[owner];
        if mutable {
            binding.mutably_borrowed = true;
        } else {
            binding.shared_borrows += 1;
        }
        Some(Loan { owner, mutable })
    }

    fn validate_reference_return(&mut self, origin: Option<BindingId>, span: ast::Span) {
        let Some(expected) = self.return_borrow else {
            self.push(
                "OWN005",
                "a reference return requires exactly one reference parameter".to_owned(),
                span,
            );
            return;
        };
        let Some(origin) = origin else {
            self.push(
                "OWN005",
                "returned reference does not originate from the declared input borrow".to_owned(),
                span,
            );
            return;
        };
        if root_origin(&self.state, origin) != root_origin(&self.state, expected) {
            self.push(
                "OWN005",
                "returned reference does not originate from the function's reference parameter"
                    .to_owned(),
                span,
            );
        }
    }

    fn push(&mut self, code: &'static str, message: String, span: ast::Span) {
        if self.emitted.insert((code, span)) {
            self.diagnostics
                .push(Diagnostic::ownership(code, message, span));
        }
    }

    fn release_expired_loans(&mut self, position: u32) {
        let ids = self
            .state
            .scope_bindings
            .iter()
            .flatten()
            .copied()
            .collect::<Vec<_>>();
        for id in ids {
            let binding = &self.state.bindings[id];
            let expired = binding.reference_loan_active
                && (self.loop_depth == 0 || self.loop_depth < binding.declaration_loop_depth)
                && binding
                    .last_use
                    .map_or(binding.declaration_span.end <= position, |last_use| {
                        last_use.end <= position
                    });
            if expired {
                self.end_reference_loan(id);
            }
        }
    }

    fn end_reference_loan(&mut self, id: BindingId) {
        if !self.state.bindings[id].reference_loan_active {
            return;
        }
        if let Some(loan) = self.state.bindings[id].reference_loan {
            self.state.release(loan);
        }
        self.state.bindings[id].reference_loan_active = false;
    }
}

fn merge_states(baseline: &FlowState, left: &FlowState, right: &FlowState) -> FlowState {
    let mut merged = baseline.clone();
    for id in baseline.scope_bindings.iter().flatten().copied() {
        let left_availability = left.bindings[id].availability;
        let right_availability = right.bindings[id].availability;
        merged.bindings[id].availability =
            merge_availability(left_availability, right_availability);
    }
    merged
}

fn merge_many_states(baseline: &FlowState, states: &[FlowState]) -> FlowState {
    let Some((first, remaining)) = states.split_first() else {
        return baseline.clone();
    };
    remaining.iter().fold(first.clone(), |merged, state| {
        merge_states(baseline, &merged, state)
    })
}

fn merge_availability(left: Availability, right: Availability) -> Availability {
    match (left, right) {
        (Availability::Available, Availability::Available) => Availability::Available,
        (Availability::Moved(left), Availability::Moved(_)) => Availability::Moved(left),
        (Availability::MaybeMoved(span), _)
        | (_, Availability::MaybeMoved(span))
        | (Availability::Moved(span), Availability::Available)
        | (Availability::Available, Availability::Moved(span)) => Availability::MaybeMoved(span),
    }
}

fn root_origin(state: &FlowState, mut id: BindingId) -> BindingId {
    while let Some(loan) = state.bindings[id].reference_loan {
        id = loan.owner;
    }
    id
}

fn return_borrow_parameter(function: &FunctionSymbol) -> Option<usize> {
    if !function.return_type.is_reference() {
        return None;
    }
    let mut parameters = function
        .parameters
        .iter()
        .enumerate()
        .filter(|(_, parameter)| parameter.ty.is_reference())
        .map(|(index, _)| index);
    let parameter = parameters.next()?;
    parameters.next().is_none().then_some(parameter)
}

fn named_binding(expression: &ast::Expression, state: &FlowState) -> Option<BindingId> {
    match &expression.kind {
        ExpressionKind::Name(path) if path.segments.len() == 1 => state.lookup(&path.segments[0]),
        ExpressionKind::Parenthesized(inner) => named_binding(inner, state),
        _ => None,
    }
}

fn call_receiver(expression: &ast::Expression) -> Option<&ast::Expression> {
    let ExpressionKind::Call { callee, .. } = &expression.kind else {
        return None;
    };
    let ExpressionKind::Field { receiver, .. } = &callee.kind else {
        return None;
    };
    Some(receiver)
}

fn is_move_call(semantics: &SemanticModel, expression: &ast::Expression) -> bool {
    semantics
        .expression(expression.span)
        .and_then(|resolution| resolution.call.as_ref())
        .is_some_and(|call| matches!(&call.target, CallTarget::Intrinsic(Intrinsic::Move)))
}

fn is_compound_assignment(operator: BinaryOperator) -> bool {
    matches!(
        operator,
        BinaryOperator::AddAssign
            | BinaryOperator::SubtractAssign
            | BinaryOperator::MultiplyAssign
            | BinaryOperator::DivideAssign
            | BinaryOperator::RemainderAssign
    )
}

fn is_json_mutation_place(expression: &ast::Expression, semantics: &SemanticModel) -> bool {
    let is_json_receiver = |receiver: &ast::Expression| {
        semantics
            .expression(receiver.span)
            .is_some_and(|resolution| {
                matches!(
                    canonical_ref(&resolution.ty),
                    TypeRef::Native { path, arguments }
                        if path == "rust::stainless_runtime::Var" && arguments.is_empty()
                )
            })
    };
    match &expression.kind {
        ExpressionKind::Field { receiver, .. } => {
            semantics
                .expression(expression.span)
                .is_some_and(|resolution| resolution.field.is_none())
                && is_json_receiver(receiver)
        }
        ExpressionKind::Index { receiver, .. } => is_json_receiver(receiver),
        _ => false,
    }
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
            | TypeRef::Struct { .. }
    ) || matches!(
        ty,
        TypeRef::Native { path, arguments }
            if path == "rust::stainless_runtime::Var" && arguments.is_empty()
    ) || matches!(
        ty,
        TypeRef::Function(function)
            if function.kind == crate::interop::StoredFunctionKind::Shared
    )
}

fn canonical_ref(ty: &TypeRef) -> &TypeRef {
    match ty {
        TypeRef::Reference { target, .. } => target,
        _ => ty,
    }
}

fn collect_last_uses(source: &ast::SourceFile) -> BTreeMap<ast::Span, ast::Span> {
    let mut collector = UseCollector {
        scopes: Vec::new(),
        last_uses: BTreeMap::new(),
    };
    collector.items(&source.items);
    collector.last_uses
}

struct UseCollector {
    scopes: Vec<BTreeMap<String, ast::Span>>,
    last_uses: BTreeMap<ast::Span, ast::Span>,
}

impl UseCollector {
    fn items(&mut self, items: &[Item]) {
        for item in items {
            match item {
                Item::Namespace(namespace) => self.items(&namespace.items),
                Item::Struct(structure) => {
                    for constructor in &structure.constructors {
                        if constructor.body.is_some() {
                            self.constructor(constructor);
                        }
                    }
                    for function in &structure.functions {
                        if function.body.is_some() {
                            self.function(function);
                        }
                    }
                }
                Item::Constructor(constructor) if constructor.body.is_some() => {
                    self.constructor(constructor);
                }
                Item::Function(function) if function.body.is_some() => self.function(function),
                Item::Constructor(_) | Item::Function(_) | Item::Use(_) => {}
            }
        }
    }

    fn constructor(&mut self, constructor: &ast::Constructor) {
        self.push_scope();
        for parameter in &constructor.parameters {
            self.declare(&parameter.name, parameter.span);
        }
        for initializer in &constructor.initializers {
            for argument in &initializer.arguments {
                self.expression(argument);
            }
        }
        if let Some(body) = &constructor.body {
            self.block(body, false);
        }
        self.scopes.pop();
    }

    fn function(&mut self, function: &ast::Function) {
        self.push_scope();
        for parameter in &function.parameters {
            self.declare(&parameter.name, parameter.span);
        }
        if let Some(body) = &function.body {
            self.block(body, false);
        }
        self.scopes.pop();
    }

    fn block(&mut self, block: &ast::Block, create_scope: bool) {
        if create_scope {
            self.push_scope();
        }
        for statement in &block.statements {
            self.statement(statement);
        }
        if create_scope {
            self.scopes.pop();
        }
    }

    fn statement(&mut self, statement: &ast::Statement) {
        match &statement.kind {
            StatementKind::Block(block) => self.block(block, true),
            StatementKind::Local(local) => {
                if let Some(initializer) = &local.initializer {
                    self.expression(initializer);
                }
                self.declare(&local.name, local.span);
            }
            StatementKind::Return(value) | StatementKind::Throw(value) => {
                if let Some(value) = value {
                    self.expression(value);
                }
            }
            StatementKind::Try(try_statement) => {
                self.block(&try_statement.body, true);
                for catch in &try_statement.catches {
                    self.push_scope();
                    if let Some(binding) = &catch.binding {
                        self.declare(&binding.name, binding.span);
                    }
                    self.block(&catch.body, false);
                    self.scopes.pop();
                }
            }
            StatementKind::If(if_statement) => {
                self.expression(&if_statement.condition);
                self.branch(&if_statement.then_branch);
                if let Some(else_branch) = &if_statement.else_branch {
                    self.branch(else_branch);
                }
            }
            StatementKind::For(for_statement) => self.for_statement(for_statement),
            StatementKind::Expression(expression) => self.expression(expression),
            StatementKind::Break
            | StatementKind::Continue
            | StatementKind::Empty
            | StatementKind::Error => {}
        }
    }

    fn branch(&mut self, statement: &ast::Statement) {
        self.push_scope();
        self.statement(statement);
        self.scopes.pop();
    }

    fn for_statement(&mut self, statement: &ast::ForStatement) {
        self.push_scope();
        match &statement.clause {
            ForClause::Classic(classic) => {
                if let Some(initializer) = &classic.initializer {
                    match initializer {
                        ForInitializer::Local(local) => {
                            if let Some(initializer) = &local.initializer {
                                self.expression(initializer);
                            }
                            self.declare(&local.name, local.span);
                        }
                        ForInitializer::Expression(expression) => {
                            self.expression(expression);
                        }
                    }
                }
                if let Some(condition) = &classic.condition {
                    self.expression(condition);
                }
                if let Some(update) = &classic.update {
                    self.expression(update);
                }
                self.branch(&statement.body);
            }
            ForClause::Range(range) => {
                self.expression(&range.iterable);
                self.declare(&range.name, range.ty.span);
                self.branch(&statement.body);
            }
            ForClause::Error => {}
        }
        self.scopes.pop();
    }

    fn expression(&mut self, expression: &ast::Expression) {
        match &expression.kind {
            ExpressionKind::Name(path) if path.segments.len() == 1 => {
                if let Some(declaration) = self.lookup(&path.segments[0]) {
                    self.last_uses.insert(declaration, expression.span);
                }
            }
            ExpressionKind::Name(_)
            | ExpressionKind::GenericName { .. }
            | ExpressionKind::Literal(_)
            | ExpressionKind::Error => {}
            ExpressionKind::Parenthesized(inner)
            | ExpressionKind::Prefix { operand: inner, .. }
            | ExpressionKind::Postfix { operand: inner, .. } => self.expression(inner),
            ExpressionKind::Binary { left, right, .. } => {
                self.expression(left);
                self.expression(right);
            }
            ExpressionKind::Call { callee, arguments } => {
                self.expression(callee);
                for argument in arguments {
                    self.expression(argument);
                }
            }
            ExpressionKind::MacroCall { arguments, .. } => {
                for argument in arguments {
                    self.expression(argument);
                }
            }
            ExpressionKind::Aggregate { initializers, .. } => {
                for initializer in initializers {
                    self.expression(initializer);
                }
            }
            ExpressionKind::JsonArray { elements } => {
                for element in elements {
                    self.expression(element);
                }
            }
            ExpressionKind::JsonObject { members } => {
                for (_, value) in members {
                    self.expression(value);
                }
            }
            ExpressionKind::Field { receiver, .. } => self.expression(receiver),
            ExpressionKind::Index { receiver, index } => {
                self.expression(receiver);
                self.expression(index);
            }
            ExpressionKind::Lambda {
                captures,
                parameters,
                body,
                ..
            } => {
                for capture in captures {
                    match &capture.kind {
                        ast::LambdaCaptureKind::Copy | ast::LambdaCaptureKind::Borrow => {
                            if let Some(declaration) = self.lookup(&capture.name) {
                                self.last_uses.insert(declaration, capture.span);
                            }
                        }
                        ast::LambdaCaptureKind::Initialize(initializer) => {
                            self.expression(initializer);
                        }
                    }
                }
                self.push_scope();
                for capture in captures {
                    self.declare(&capture.name, capture.span);
                }
                for parameter in parameters {
                    self.declare(&parameter.name, parameter.span);
                }
                self.block(body, false);
                self.scopes.pop();
            }
        }
    }

    fn push_scope(&mut self) {
        self.scopes.push(BTreeMap::new());
    }

    fn declare(&mut self, name: &str, span: ast::Span) {
        self.scopes
            .last_mut()
            .expect("use collection always has a scope")
            .insert(name.to_owned(), span);
    }

    fn lookup(&self, name: &str) -> Option<ast::Span> {
        self.scopes
            .iter()
            .rev()
            .find_map(|scope| scope.get(name).copied())
    }
}
