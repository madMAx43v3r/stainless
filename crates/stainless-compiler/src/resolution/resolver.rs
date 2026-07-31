use std::collections::{BTreeMap, BTreeSet};

use crate::Diagnostic;
use crate::ast::{
    self, BinaryOperator, Expression, ExpressionKind, ForClause, ForInitializer, Item,
    LambdaCaptureKind, LiteralKind, PrefixOperator, SourceFile, Span, Statement, StatementKind,
    TypeKind,
};
use crate::interop::{
    CallStyle, CallableBinding, CallbackKind, CallbackType, NativeBindings, NativeErrorFormat,
    NativeTypeBinding, Receiver, TypeRef,
};

use super::imports::ImportTable;
use super::mangle;
use super::{
    BindingResolution, CallTarget, CallbackTarget, ConstructorFieldInitialization, ConstructorId,
    ConstructorSymbol, ExpressionResolution, FieldSymbol, FunctionId, FunctionSymbol, Intrinsic,
    LambdaCaptureMode, NativeCall, ParameterSymbol, Resolution, ResolvedCall, ResolvedCallback,
    ResolvedField, ResolvedLambdaCapture, ResolvedNativeType, ResolvedTraitRequirement,
    RustErrorMessage, RustResultAdaptation, SemanticModel, StructId, StructReceiver, StructSymbol,
    ValueCategory,
};

/// Resolves names and types using an explicit native binding registry.
#[must_use]
pub fn resolve(source: &SourceFile, bindings: &NativeBindings) -> Resolution {
    let mut diagnostics = Vec::new();
    let imports = ImportTable::build(source, &mut diagnostics);
    let model = SemanticModel {
        native_types: bindings
            .types()
            .map(|binding| ResolvedNativeType {
                stainless_path: binding.stainless_path.clone(),
                rust_path: binding.rust_path.clone(),
            })
            .collect(),
        ..SemanticModel::default()
    };
    let mut resolver = Resolver {
        bindings,
        imports,
        diagnostics,
        model,
        function_sets: BTreeMap::new(),
        function_by_span: BTreeMap::new(),
        struct_by_path: BTreeMap::new(),
        struct_by_span: BTreeMap::new(),
        constructor_sets: BTreeMap::new(),
        constructor_by_span: BTreeMap::new(),
    };
    if source_uses_exceptions(source) {
        resolver.install_exception_builtins();
    }
    resolver.collect_struct_names(&source.items, &mut Vec::new());
    resolver.resolve_struct_definitions(&source.items, &mut Vec::new());
    resolver.validate_struct_cycles();
    resolver.collect_signatures(&source.items, &mut Vec::new());
    resolver.synthesize_default_constructors();
    resolver.validate_member_declarations();
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
    struct_by_path: BTreeMap<Vec<String>, StructId>,
    struct_by_span: BTreeMap<Span, StructId>,
    constructor_sets: BTreeMap<StructId, Vec<ConstructorId>>,
    constructor_by_span: BTreeMap<Span, ConstructorId>,
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
    receiver: Option<StructReceiver>,
    declared_throws: Vec<StructId>,
    handled_throws: Vec<Vec<StructId>>,
    current_catch: Option<StructId>,
    is_lambda: bool,
}

#[derive(Clone, Debug)]
struct ExpressionInfo {
    ty: TypeRef,
    category: ValueCategory,
}

#[derive(Clone, Debug)]
struct NativeInstance {
    type_path: String,
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
    fn install_exception_builtins(&mut self) {
        let root = StructId(self.model.structs.len());
        let root_path = vec!["stainless".to_owned(), "Exception".to_owned()];
        self.model.structs.push(StructSymbol {
            id: root,
            path: root_path.clone(),
            base: None,
            fields: vec![FieldSymbol {
                name: "message".to_owned(),
                ty: TypeRef::Native {
                    path: "rust::String".to_owned(),
                    arguments: Vec::new(),
                },
                span: Span::default(),
            }],
            span: Span::default(),
        });
        self.struct_by_path.insert(root_path, root);

        let rust_error = StructId(self.model.structs.len());
        let rust_error_path = vec!["stainless".to_owned(), "RustError".to_owned()];
        self.model.structs.push(StructSymbol {
            id: rust_error,
            path: rust_error_path.clone(),
            base: Some(root),
            fields: Vec::new(),
            span: Span::default(),
        });
        self.struct_by_path.insert(rust_error_path, rust_error);
    }

    fn collect_struct_names(&mut self, items: &[Item], namespace: &mut Vec<String>) {
        for item in items {
            match item {
                Item::Namespace(child) => {
                    namespace.push(child.name.clone());
                    self.collect_struct_names(&child.items, namespace);
                    namespace.pop();
                }
                Item::Struct(structure) => {
                    let mut path = namespace.clone();
                    path.push(structure.name.clone());
                    if self.struct_by_path.contains_key(&path) {
                        self.push(
                            "RES039",
                            format!("duplicate struct definition `{}`", display_path(&path)),
                            structure.span,
                        );
                        continue;
                    }
                    let id = StructId(self.model.structs.len());
                    self.model.structs.push(StructSymbol {
                        id,
                        path: path.clone(),
                        base: None,
                        fields: Vec::new(),
                        span: structure.span,
                    });
                    self.struct_by_path.insert(path, id);
                    self.struct_by_span.insert(structure.span, id);
                }
                Item::Use(_) | Item::Constructor(_) | Item::Function(_) => {}
            }
        }
    }

    fn resolve_struct_definitions(&mut self, items: &[Item], namespace: &mut Vec<String>) {
        for item in items {
            match item {
                Item::Namespace(child) => {
                    namespace.push(child.name.clone());
                    self.resolve_struct_definitions(&child.items, namespace);
                    namespace.pop();
                }
                Item::Struct(structure) => {
                    let Some(id) = self.struct_by_span.get(&structure.span).copied() else {
                        continue;
                    };
                    let base = structure.base.as_ref().and_then(|base| {
                        let found = self.lookup_struct_path(&base.segments, namespace);
                        if found.is_none() {
                            self.push(
                                "RES040",
                                format!("unresolved data base struct `{}`", base.display()),
                                structure.span,
                            );
                        }
                        found
                    });
                    if base == Some(id) {
                        self.push(
                            "RES041",
                            "a struct cannot inherit from itself".to_owned(),
                            structure.span,
                        );
                    }
                    let mut names = BTreeSet::new();
                    let fields = structure
                        .fields
                        .iter()
                        .map(|field| {
                            if !names.insert(field.name.clone()) {
                                self.push(
                                    "RES042",
                                    format!("duplicate field `{}`", field.name),
                                    field.span,
                                );
                            }
                            let ty = self.resolve_type(&field.ty, namespace, false);
                            if ty.contains_reference() {
                                self.push(
                                    "RES043",
                                    "references are not allowed as data fields".to_owned(),
                                    field.ty.span,
                                );
                            }
                            FieldSymbol {
                                name: field.name.clone(),
                                ty,
                                span: field.span,
                            }
                        })
                        .collect();
                    let symbol = &mut self.model.structs[id.0];
                    symbol.base = base.filter(|base| *base != id);
                    symbol.fields = fields;
                }
                Item::Use(_) | Item::Constructor(_) | Item::Function(_) => {}
            }
        }
    }

    fn validate_struct_cycles(&mut self) {
        for structure in &self.model.structs.clone() {
            let mut seen = BTreeSet::new();
            let mut current = Some(structure.id);
            while let Some(id) = current {
                if !seen.insert(id) {
                    self.push(
                        "RES044",
                        format!(
                            "data inheritance cycle involving `{}`",
                            display_path(&structure.path)
                        ),
                        structure.span,
                    );
                    break;
                }
                current = self.model.structs[id.0].base;
            }
        }
    }

    fn collect_signatures(&mut self, items: &[Item], namespace: &mut Vec<String>) {
        for item in items {
            match item {
                Item::Namespace(child) => {
                    namespace.push(child.name.clone());
                    self.collect_signatures(&child.items, namespace);
                    namespace.pop();
                }
                Item::Struct(structure) => {
                    let owner = self.struct_by_span.get(&structure.span).copied();
                    if let Some(owner) = owner {
                        for constructor in &structure.constructors {
                            self.collect_constructor(constructor, namespace, Some(owner));
                        }
                    }
                    for function in &structure.functions {
                        self.collect_function(function, namespace, owner);
                    }
                }
                Item::Constructor(constructor) => {
                    self.collect_constructor(constructor, namespace, None);
                }
                Item::Function(function) => self.collect_function(function, namespace, None),
                Item::Use(_) => {}
            }
        }
    }

    #[allow(clippy::too_many_lines)]
    fn collect_constructor(
        &mut self,
        constructor: &ast::Constructor,
        namespace: &[String],
        declared_owner: Option<StructId>,
    ) {
        let (owner, path) = if let Some(owner) = declared_owner {
            let mut path = self.model.structs[owner.0].path.clone();
            path.push(
                constructor
                    .name
                    .segments
                    .last()
                    .cloned()
                    .unwrap_or_default(),
            );
            (Some(owner), path)
        } else {
            let path = qualify_declaration_path(namespace, &constructor.name.segments);
            let owner = (path.len() >= 2)
                .then(|| self.struct_by_path.get(&path[..path.len() - 1]).copied())
                .flatten();
            (owner, path)
        };
        let Some(owner) = owner else {
            self.push(
                "RES053",
                format!(
                    "constructor `{}` does not name a known struct",
                    constructor.name.display()
                ),
                constructor.span,
            );
            return;
        };
        let structure_path = self.model.structs[owner.0].path.clone();
        if path.last() != structure_path.last() {
            self.push(
                "RES054",
                format!(
                    "constructor name must be `{}`",
                    structure_path.last().map_or("<missing>", String::as_str)
                ),
                constructor.span,
            );
        }
        let type_namespace = &structure_path[..structure_path.len().saturating_sub(1)];
        let throws =
            self.resolve_exception_set(&constructor.throws, type_namespace, constructor.span);
        let parameters = constructor
            .parameters
            .iter()
            .map(|parameter| ParameterSymbol {
                name: parameter.name.clone(),
                ty: self.resolve_type(&parameter.ty, type_namespace, false),
                span: parameter.span,
            })
            .collect::<Vec<_>>();
        let signature = parameters
            .iter()
            .map(|parameter| canonical(&parameter.ty))
            .collect::<Vec<_>>();
        let existing = self
            .constructor_sets
            .get(&owner)
            .into_iter()
            .flatten()
            .copied()
            .find(|id| {
                self.model.constructors[id.0]
                    .parameters
                    .iter()
                    .map(|parameter| canonical(&parameter.ty))
                    .eq(signature.iter().cloned())
            });
        if let Some(id) = existing {
            let same_modes = self.model.constructors[id.0]
                .parameters
                .iter()
                .map(|parameter| &parameter.ty)
                .eq(parameters.iter().map(|parameter| &parameter.ty));
            let has_definition = self.model.constructors[id.0].has_definition;
            let is_deleted = self.model.constructors[id.0].is_deleted;
            let different_throws = self.model.constructors[id.0].throws != throws;
            if !same_modes {
                self.push(
                    "RES055",
                    "constructors cannot differ only by value/reference passing mode".to_owned(),
                    constructor.span,
                );
            }
            if has_definition && constructor.body.is_some() {
                self.push(
                    "RES056",
                    "duplicate constructor definition".to_owned(),
                    constructor.span,
                );
            }
            if (is_deleted && constructor.body.is_some())
                || (constructor.is_deleted && has_definition)
            {
                self.push(
                    "RES057",
                    "a deleted constructor cannot have a definition".to_owned(),
                    constructor.span,
                );
            }
            if different_throws {
                self.push(
                    "RES068",
                    "constructor declarations have different checked exception sets".to_owned(),
                    constructor.span,
                );
            }
            let symbol = &mut self.model.constructors[id.0];
            symbol.declarations.push(constructor.span);
            symbol.has_definition |= constructor.body.is_some();
            symbol.has_member_declaration |= declared_owner.is_some();
            symbol.is_deleted |= constructor.is_deleted;
            self.constructor_by_span.insert(constructor.span, id);
            return;
        }
        let id = ConstructorId(self.model.constructors.len());
        let mangled_name = mangle::function_name(
            &path,
            &parameters
                .iter()
                .map(|parameter| parameter.ty.clone())
                .collect::<Vec<_>>(),
        );
        self.model.constructors.push(ConstructorSymbol {
            id,
            structure: owner,
            parameters,
            throws,
            mangled_name,
            declarations: vec![constructor.span],
            has_definition: constructor.body.is_some(),
            has_member_declaration: declared_owner.is_some(),
            is_deleted: constructor.is_deleted,
            synthesized: false,
            initializations: Vec::new(),
        });
        self.constructor_sets.entry(owner).or_default().push(id);
        self.constructor_by_span.insert(constructor.span, id);
    }

    fn synthesize_default_constructors(&mut self) {
        for structure in self.model.structs.clone() {
            if self
                .constructor_sets
                .get(&structure.id)
                .is_some_and(|constructors| !constructors.is_empty())
            {
                continue;
            }
            let id = ConstructorId(self.model.constructors.len());
            let mut path = structure.path.clone();
            path.push(
                structure
                    .path
                    .last()
                    .cloned()
                    .unwrap_or_else(|| "<missing>".to_owned()),
            );
            self.model.constructors.push(ConstructorSymbol {
                id,
                structure: structure.id,
                parameters: Vec::new(),
                throws: Vec::new(),
                mangled_name: mangle::function_name(&path, &[]),
                declarations: vec![structure.span],
                has_definition: true,
                has_member_declaration: true,
                is_deleted: false,
                synthesized: true,
                initializations: Vec::new(),
            });
            self.constructor_sets
                .entry(structure.id)
                .or_default()
                .push(id);
        }
        for id in 0..self.model.structs.len() {
            let structure = StructId(id);
            let available = self.struct_has_default_constructor(structure, &mut BTreeSet::new());
            if let Some(constructor) = self
                .constructor_sets
                .get(&structure)
                .into_iter()
                .flatten()
                .find(|id| self.model.constructors[id.0].synthesized)
                .copied()
            {
                self.model.constructors[constructor.0].is_deleted = !available;
            }
        }
    }

    fn struct_has_default_constructor(
        &self,
        structure: StructId,
        visiting: &mut BTreeSet<StructId>,
    ) -> bool {
        if !visiting.insert(structure) {
            return false;
        }
        let constructors = self
            .constructor_sets
            .get(&structure)
            .cloned()
            .unwrap_or_default();
        if constructors.iter().any(|id| {
            let constructor = &self.model.constructors[id.0];
            !constructor.synthesized
                && constructor.parameters.is_empty()
                && !constructor.is_deleted
                && constructor.has_definition
                && constructor.throws.is_empty()
        }) {
            visiting.remove(&structure);
            return true;
        }
        let synthesized = constructors
            .iter()
            .any(|id| self.model.constructors[id.0].synthesized);
        if !synthesized {
            visiting.remove(&structure);
            return false;
        }
        let symbol = &self.model.structs[structure.0];
        let available = symbol
            .base
            .is_none_or(|base| self.struct_has_default_constructor(base, visiting))
            && symbol
                .fields
                .iter()
                .all(|field| self.type_has_default_constructor(&field.ty, visiting));
        visiting.remove(&structure);
        available
    }

    fn type_has_default_constructor(
        &self,
        ty: &TypeRef,
        visiting: &mut BTreeSet<StructId>,
    ) -> bool {
        match canonical_ref(ty) {
            TypeRef::Native { path, .. } => {
                self.bindings.type_by_path(path).is_some_and(|binding| {
                    binding.callables.iter().any(|callable| {
                        callable.style == CallStyle::Constructor && callable.parameters.is_empty()
                    })
                })
            }
            TypeRef::Struct { path } => self
                .struct_by_path
                .get(path)
                .is_some_and(|id| self.struct_has_default_constructor(*id, visiting)),
            _ => false,
        }
    }

    #[allow(clippy::too_many_lines)]
    fn collect_function(
        &mut self,
        function: &ast::Function,
        namespace: &[String],
        declared_owner: Option<StructId>,
    ) {
        let path = if let Some(owner) = declared_owner {
            let mut path = self.model.structs[owner.0].path.clone();
            path.extend(function.name.segments.iter().cloned());
            path
        } else {
            qualify_declaration_path(namespace, &function.name.segments)
        };
        let owner = declared_owner.or_else(|| {
            (path.len() >= 2)
                .then(|| self.struct_by_path.get(&path[..path.len() - 1]).copied())
                .flatten()
        });
        let receiver = owner.map(|structure| StructReceiver {
            structure,
            mutable: !function.is_const,
        });
        let type_namespace = receiver.as_ref().map_or_else(
            || namespace.to_vec(),
            |receiver| {
                let path = &self.model.structs[receiver.structure.0].path;
                path[..path.len().saturating_sub(1)].to_vec()
            },
        );
        let parameters = function
            .parameters
            .iter()
            .map(|parameter| ParameterSymbol {
                name: parameter.name.clone(),
                ty: self.resolve_type(&parameter.ty, &type_namespace, false),
                span: parameter.span,
            })
            .collect::<Vec<_>>();
        let mut return_type = self.resolve_type(&function.return_type, &type_namespace, false);
        let throws = self.resolve_exception_set(&function.throws, &type_namespace, function.span);
        if return_type == TypeRef::Void
            && let Some(receiver) = &receiver
        {
            return_type = TypeRef::Reference {
                mutable: receiver.mutable,
                target: Box::new(TypeRef::Struct {
                    path: self.model.structs[receiver.structure.0].path.clone(),
                }),
            };
        }
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
            let different_receiver = existing.receiver != receiver;
            let different_throws = existing.throws != throws;
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
                if different_receiver {
                    self.push(
                        "RES045",
                        format!(
                            "declarations of `{}` disagree on member `const` qualification",
                            display_path(&path)
                        ),
                        function.span,
                    );
                }
                if different_throws {
                    self.push(
                        "RES069",
                        format!(
                            "declarations of `{}` have different checked exception sets",
                            display_path(&path)
                        ),
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
            symbol.has_member_declaration |= declared_owner.is_some();
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
            throws,
            receiver,
            mangled_name,
            declarations: vec![function.span],
            has_definition: function.body.is_some(),
            has_member_declaration: declared_owner.is_some(),
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
                Item::Struct(structure) => {
                    for constructor in &structure.constructors {
                        if constructor.body.is_some() {
                            self.resolve_constructor_body(constructor, namespace);
                        }
                    }
                    for function in &structure.functions {
                        if function.body.is_some() {
                            self.resolve_function_body(function, namespace);
                        }
                    }
                }
                Item::Constructor(constructor) => {
                    if constructor.body.is_some() {
                        self.resolve_constructor_body(constructor, namespace);
                    }
                }
                Item::Function(function) => {
                    if function.body.is_some() {
                        self.resolve_function_body(function, namespace);
                    }
                }
                Item::Use(_) => {}
            }
        }
        if namespace.is_empty() {
            self.resolve_synthesized_constructors();
        }
    }

    fn validate_member_declarations(&mut self) {
        for constructor in self.model.constructors.clone() {
            if !constructor.synthesized && !constructor.has_member_declaration {
                self.push(
                    "RES058",
                    "constructor must be declared inside its struct body".to_owned(),
                    constructor.declarations[0],
                );
            }
        }
        for function in self.model.functions.clone() {
            if function.receiver.is_some() && !function.has_member_declaration {
                self.push(
                    "RES051",
                    format!(
                        "member `{}` must be declared inside its struct body",
                        display_path(&function.path)
                    ),
                    function.declarations[0],
                );
            }
        }
    }

    #[allow(clippy::too_many_lines)]
    fn resolve_constructor_body(&mut self, constructor: &ast::Constructor, _namespace: &[String]) {
        let Some(id) = self.constructor_by_span.get(&constructor.span).copied() else {
            return;
        };
        let symbol = self.model.constructors[id.0].clone();
        let structure = self.model.structs[symbol.structure.0].clone();
        let constructor_namespace =
            structure.path[..structure.path.len().saturating_sub(1)].to_vec();
        let mut scope = BTreeMap::new();
        for (syntax, parameter) in constructor.parameters.iter().zip(&symbol.parameters) {
            scope.insert(
                syntax.name.clone(),
                Variable {
                    ty: parameter.ty.clone(),
                    mutable: parameter_mutability(syntax, &parameter.ty),
                },
            );
        }
        let mut initialization_context = FunctionContext {
            namespace: constructor_namespace.clone(),
            return_type: TypeRef::Void,
            scopes: vec![scope.clone()],
            receiver: None,
            declared_throws: symbol.throws.clone(),
            handled_throws: Vec::new(),
            current_catch: None,
            is_lambda: false,
        };
        let slots = self.constructor_slots(symbol.structure);
        let mut explicit = BTreeMap::<usize, &ast::ConstructorInitializer>::new();
        for initializer in &constructor.initializers {
            let Some(slot) =
                self.constructor_initializer_slot(symbol.structure, &initializer.target)
            else {
                for argument in &initializer.arguments {
                    self.resolve_expression(argument, None, &mut initialization_context);
                }
                self.push(
                    "RES061",
                    format!(
                        "`{}` is not a direct field or data base of this constructor",
                        initializer.target.display()
                    ),
                    initializer.span,
                );
                continue;
            };
            if explicit.insert(slot, initializer).is_some() {
                self.push(
                    "RES062",
                    format!(
                        "constructor initializes `{}` more than once",
                        initializer.target.display()
                    ),
                    initializer.span,
                );
            }
        }
        let mut initializations = Vec::new();
        for (index, (rust_name, ty)) in slots.into_iter().enumerate() {
            let (source, call) = if let Some(initializer) = explicit.get(&index) {
                (
                    Some(initializer.span),
                    self.resolve_slot_construction(
                        &ty,
                        &initializer.arguments,
                        initializer.span,
                        &mut initialization_context,
                    ),
                )
            } else {
                (
                    None,
                    self.resolve_default_call(&ty, constructor.span, &mut initialization_context),
                )
            };
            if let Some(call) = call {
                for thrown in &call.throws {
                    self.validate_checked_effect(*thrown, call.span, &initialization_context);
                }
                self.model.calls.push(call.clone());
                initializations.push(ConstructorFieldInitialization {
                    rust_name,
                    ty,
                    source,
                    call,
                });
            }
        }
        self.model.constructors[id.0].initializations = initializations;

        let mut context = FunctionContext {
            namespace: constructor_namespace,
            return_type: TypeRef::Void,
            scopes: vec![scope],
            receiver: Some(StructReceiver {
                structure: symbol.structure,
                mutable: true,
            }),
            declared_throws: symbol.throws,
            handled_throws: Vec::new(),
            current_catch: None,
            is_lambda: false,
        };
        if let Some(body) = &constructor.body {
            self.resolve_block(body, &mut context, false);
        }
    }

    fn resolve_synthesized_constructors(&mut self) {
        let constructors = self
            .model
            .constructors
            .iter()
            .filter(|constructor| constructor.synthesized && !constructor.is_deleted)
            .map(|constructor| constructor.id)
            .collect::<Vec<_>>();
        for id in constructors {
            let symbol = self.model.constructors[id.0].clone();
            let structure = self.model.structs[symbol.structure.0].clone();
            let mut context = FunctionContext {
                namespace: structure.path[..structure.path.len().saturating_sub(1)].to_vec(),
                return_type: TypeRef::Void,
                scopes: vec![BTreeMap::new()],
                receiver: None,
                declared_throws: Vec::new(),
                handled_throws: Vec::new(),
                current_catch: None,
                is_lambda: false,
            };
            let mut initializations = Vec::new();
            for (rust_name, ty) in self.constructor_slots(symbol.structure) {
                if let Some(call) = self.resolve_default_call(&ty, structure.span, &mut context) {
                    self.model.calls.push(call.clone());
                    initializations.push(ConstructorFieldInitialization {
                        rust_name,
                        ty,
                        source: None,
                        call,
                    });
                }
            }
            self.model.constructors[id.0].initializations = initializations;
        }
    }

    fn constructor_slots(&self, structure: StructId) -> Vec<(String, TypeRef)> {
        let symbol = &self.model.structs[structure.0];
        let mut slots = Vec::new();
        if let Some(base) = symbol.base {
            let base = &self.model.structs[base.0];
            slots.push((
                base_field_name(base),
                TypeRef::Struct {
                    path: base.path.clone(),
                },
            ));
        }
        slots.extend(
            symbol
                .fields
                .iter()
                .map(|field| (field.name.clone(), field.ty.clone())),
        );
        slots
    }

    fn constructor_initializer_slot(
        &self,
        structure: StructId,
        target: &ast::Path,
    ) -> Option<usize> {
        let symbol = &self.model.structs[structure.0];
        if let Some(base) = symbol.base {
            let base_path = &self.model.structs[base.0].path;
            if base_path.ends_with(&target.segments) {
                return Some(0);
            }
        }
        if target.segments.len() != 1 {
            return None;
        }
        let offset = usize::from(symbol.base.is_some());
        symbol
            .fields
            .iter()
            .position(|field| field.name == target.segments[0])
            .map(|index| offset + index)
    }

    fn resolve_slot_construction(
        &mut self,
        target: &TypeRef,
        arguments: &[Expression],
        span: Span,
        context: &mut FunctionContext,
    ) -> Option<ResolvedCall> {
        match canonical_ref(target) {
            TypeRef::Struct { path } => {
                let structure = self.struct_by_path.get(path).copied()?;
                let has_arity = self
                    .constructor_sets
                    .get(&structure)
                    .into_iter()
                    .flatten()
                    .any(|id| self.model.constructors[id.0].parameters.len() == arguments.len());
                if !has_arity && arguments.len() == 1 {
                    return self.resolve_direct_initialization(
                        target,
                        &arguments[0],
                        span,
                        context,
                    );
                }
                self.resolve_user_constructor(structure, arguments, span, context)
                    .1
            }
            TypeRef::Native {
                path,
                arguments: type_arguments,
            } => {
                let instance = NativeInstance {
                    type_path: path.clone(),
                    arguments: type_arguments.clone(),
                };
                let (_, call) = self.resolve_native_callable(
                    &instance,
                    CallStyle::Constructor,
                    instance_short_name(path),
                    arguments,
                    span,
                    None,
                    context,
                );
                call
            }
            TypeRef::Reference { .. } | TypeRef::Void | TypeRef::Error | TypeRef::Parameter(_) => {
                for argument in arguments {
                    self.resolve_expression(argument, None, context);
                }
                self.push(
                    "RES063",
                    format!("type `{}` cannot be constructed here", display_type(target)),
                    span,
                );
                None
            }
            _ if arguments.len() == 1 => {
                self.resolve_direct_initialization(target, &arguments[0], span, context)
            }
            _ => {
                for argument in arguments {
                    self.resolve_expression(argument, None, context);
                }
                self.push(
                    "RES064",
                    format!(
                        "primitive `{}` requires exactly one initializer",
                        display_type(target)
                    ),
                    span,
                );
                None
            }
        }
    }

    fn resolve_direct_initialization(
        &mut self,
        target: &TypeRef,
        argument: &Expression,
        span: Span,
        context: &mut FunctionContext,
    ) -> Option<ResolvedCall> {
        let expected = canonical(target);
        let actual = self.resolve_expression(argument, Some(&expected), context);
        let actual = self.adapt_rust_result(target, actual, argument, context);
        self.validate_binding(target, &actual, argument.span, "constructor initializer");
        if canonical(&actual.ty) == expected {
            Some(ResolvedCall {
                span,
                target: CallTarget::Intrinsic(Intrinsic::ValueInitialization {
                    target: target.clone(),
                }),
                return_type: target.clone(),
                throws: Vec::new(),
            })
        } else {
            None
        }
    }

    fn resolve_default_call(
        &mut self,
        target: &TypeRef,
        span: Span,
        context: &mut FunctionContext,
    ) -> Option<ResolvedCall> {
        match canonical_ref(target) {
            TypeRef::Struct { path } => {
                let structure = self.struct_by_path.get(path).copied()?;
                self.resolve_user_constructor(structure, &[], span, context)
                    .1
            }
            TypeRef::Native {
                path,
                arguments: type_arguments,
            } => {
                let instance = NativeInstance {
                    type_path: path.clone(),
                    arguments: type_arguments.clone(),
                };
                let (_, call) = self.resolve_native_callable(
                    &instance,
                    CallStyle::Constructor,
                    instance_short_name(path),
                    &[],
                    span,
                    None,
                    context,
                );
                call
            }
            _ => {
                self.push(
                    "RES065",
                    format!(
                        "field of type `{}` requires an explicit constructor initializer",
                        display_type(target)
                    ),
                    span,
                );
                None
            }
        }
    }

    fn resolve_function_body(&mut self, function: &ast::Function, _namespace: &[String]) {
        let Some(id) = self.function_by_span.get(&function.span).copied() else {
            return;
        };
        let symbol = self.model.functions[id.0].clone();
        let function_namespace = symbol.receiver.as_ref().map_or_else(
            || symbol.path[..symbol.path.len().saturating_sub(1)].to_vec(),
            |receiver| {
                let path = &self.model.structs[receiver.structure.0].path;
                path[..path.len().saturating_sub(1)].to_vec()
            },
        );
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
            namespace: function_namespace,
            return_type: symbol.return_type,
            scopes: vec![initial_scope],
            receiver: symbol.receiver,
            declared_throws: symbol.throws,
            handled_throws: Vec::new(),
            current_catch: None,
            is_lambda: false,
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
                        if context.is_lambda {
                            self.push(
                                "RES083",
                                "a void callback cannot return a value".to_owned(),
                                statement.span,
                            );
                        }
                    } else {
                        let actual =
                            self.resolve_expression(value, Some(&canonical(&expected)), context);
                        self.validate_binding(&expected, &actual, value.span, "return value");
                    }
                } else if context.is_lambda && context.return_type != TypeRef::Void {
                    self.push(
                        "RES083",
                        "a non-void callback must return a value".to_owned(),
                        statement.span,
                    );
                }
            }
            StatementKind::Throw(value) => {
                self.resolve_throw_statement(value.as_ref(), statement.span, context);
            }
            StatementKind::Try(try_statement) => {
                self.resolve_try_statement(try_statement, context);
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

    fn resolve_throw_statement(
        &mut self,
        value: Option<&Expression>,
        span: Span,
        context: &mut FunctionContext,
    ) {
        let thrown = if let Some(value) = value {
            let actual = self.resolve_expression(value, None, context);
            let TypeRef::Struct { path } = canonical(&actual.ty) else {
                if actual.ty != TypeRef::Error {
                    self.push(
                        "RES074",
                        format!(
                            "`throw` requires an exception struct, found `{}`",
                            display_type(&actual.ty)
                        ),
                        value.span,
                    );
                }
                return;
            };
            let Some(id) = self.struct_by_path.get(&path).copied() else {
                return;
            };
            if !self.is_exception_struct(id) {
                self.push(
                    "RES074",
                    format!(
                        "`{}` does not derive from `stainless::Exception`",
                        display_path(&path)
                    ),
                    value.span,
                );
                return;
            }
            id
        } else {
            let Some(id) = context.current_catch else {
                return;
            };
            id
        };
        self.validate_checked_effect(thrown, span, context);
    }

    fn resolve_try_statement(
        &mut self,
        try_statement: &ast::TryStatement,
        context: &mut FunctionContext,
    ) {
        let root = self.exception_root();
        let mut catches = Vec::new();
        let mut resolved_catches = Vec::new();
        for catch in &try_statement.catches {
            let caught = if let Some(binding) = &catch.binding {
                let resolved = self.resolve_type(&binding.ty, &context.namespace, false);
                if let TypeRef::Struct { path } = canonical(&resolved) {
                    self.struct_by_path.get(&path).copied().and_then(|id| {
                        if self.is_exception_struct(id) {
                            Some(id)
                        } else {
                            self.push(
                                "RES076",
                                format!(
                                    "`{}` does not derive from `stainless::Exception`",
                                    display_path(&path)
                                ),
                                binding.span,
                            );
                            None
                        }
                    })
                } else {
                    if resolved != TypeRef::Error {
                        self.push(
                            "RES076",
                            "catch binding must name an exception struct".to_owned(),
                            binding.span,
                        );
                    }
                    None
                }
            } else {
                root
            };
            if let Some(caught) = caught {
                if catches
                    .iter()
                    .any(|previous| self.exception_covers(*previous, caught))
                {
                    self.push(
                        "RES077",
                        format!(
                            "catch for `{}` is unreachable after an earlier base handler",
                            display_path(&self.model.structs[caught.0].path)
                        ),
                        catch.span,
                    );
                }
                catches.push(caught);
            }
            resolved_catches.push(caught);
        }

        context.handled_throws.push(catches.clone());
        self.resolve_block(&try_statement.body, context, true);
        context.handled_throws.pop();

        for (catch, caught) in try_statement.catches.iter().zip(resolved_catches) {
            context.scopes.push(BTreeMap::new());
            if let Some(binding) = &catch.binding
                && let Some(caught) = caught
            {
                let path = self.model.structs[caught.0].path.clone();
                let ty = TypeRef::Reference {
                    mutable: false,
                    target: Box::new(TypeRef::Struct { path }),
                };
                let variable = Variable {
                    ty: ty.clone(),
                    mutable: false,
                };
                self.model.bindings.push(BindingResolution {
                    span: binding.span,
                    name: binding.name.clone(),
                    ty,
                    mutable: false,
                });
                let scope = context.scopes.last_mut().expect("catch scope was pushed");
                self.insert_variable(scope, &binding.name, variable, binding.span);
            }
            let previous = std::mem::replace(&mut context.current_catch, caught.or(root));
            self.resolve_block(&catch.body, context, false);
            context.current_catch = previous;
            context.scopes.pop();
        }
    }

    fn validate_checked_effect(&mut self, thrown: StructId, span: Span, context: &FunctionContext) {
        let handled = context.handled_throws.iter().rev().any(|catches| {
            catches
                .iter()
                .any(|caught| self.exception_covers(*caught, thrown))
        });
        let declared = context
            .declared_throws
            .iter()
            .any(|allowed| self.exception_covers(*allowed, thrown));
        if !handled && !declared {
            self.push(
                "RES075",
                format!(
                    "checked exception `{}` must be caught or declared in `throws`",
                    display_path(&self.model.structs[thrown.0].path)
                ),
                span,
            );
        }
    }

    fn exception_root(&self) -> Option<StructId> {
        let path = vec!["stainless".to_owned(), "Exception".to_owned()];
        self.struct_by_path.get(&path).copied()
    }

    fn rust_error_struct(&mut self) -> StructId {
        if self.exception_root().is_none() {
            self.install_exception_builtins();
        }
        let path = vec!["stainless".to_owned(), "RustError".to_owned()];
        self.struct_by_path
            .get(&path)
            .copied()
            .expect("installing exception builtins creates RustError")
    }

    fn resolve_local(&mut self, local: &ast::LocalDeclaration, context: &mut FunctionContext) {
        let declared = if local.ty.is_inferred() {
            None
        } else {
            Some(self.resolve_type(&local.ty, &context.namespace, false))
        };
        let resolved_type = if let Some(initializer) = &local.initializer {
            let expected = declared.as_ref().map(canonical);
            let mut actual = self.resolve_expression(initializer, expected.as_ref(), context);
            if let Some(declared) = &declared {
                actual = self.adapt_rust_result(declared, actual, initializer, context);
                self.validate_binding(declared, &actual, initializer.span, "initializer");
                declared.clone()
            } else {
                let inferred = canonical(&actual.ty);
                self.validate_value_use(&inferred, &actual, initializer.span, "initializer");
                inferred
            }
        } else {
            let ty = declared.unwrap_or(TypeRef::Error);
            self.resolve_default_construction(&ty, local.span, context);
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
        self.model.bindings.push(BindingResolution {
            span: local.span,
            name: local.name.clone(),
            ty: variable.ty.clone(),
            mutable: variable.mutable,
        });
        let scope = context
            .scopes
            .last_mut()
            .expect("a function context always has a scope");
        self.insert_variable(scope, &local.name, variable, local.span);
    }

    fn adapt_rust_result(
        &mut self,
        expected: &TypeRef,
        actual: ExpressionInfo,
        expression: &Expression,
        context: &FunctionContext,
    ) -> ExpressionInfo {
        if expected.is_reference() {
            return actual;
        }
        let TypeRef::Native { path, arguments } = canonical(&actual.ty) else {
            return actual;
        };
        let [value_type, error_type] = arguments.as_slice() else {
            return actual;
        };
        if path != "rust::Result" || canonical(expected) != canonical(value_type) {
            return actual;
        }
        if actual.category != ValueCategory::Temporary {
            self.push(
                "RES080",
                "implicit native Result conversion requires `move(result)` for a named value"
                    .to_owned(),
                expression.span,
            );
            return error_info();
        }
        let rust_error = self.rust_error_struct();
        self.validate_checked_effect(rust_error, expression.span, context);
        self.model
            .rust_result_adaptations
            .push(RustResultAdaptation {
                span: expression.span,
                error_message: self.rust_error_message(error_type),
            });
        temporary(canonical(expected))
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
        self.model.bindings.push(BindingResolution {
            span: range.ty.span,
            name: range.name.clone(),
            ty: variable.ty.clone(),
            mutable: variable.mutable,
        });
        let scope = context
            .scopes
            .last_mut()
            .expect("a function context always has a scope");
        self.insert_variable(scope, &range.name, variable, range.ty.span);
    }

    #[allow(clippy::too_many_lines)]
    fn resolve_expression(
        &mut self,
        expression: &Expression,
        expected: Option<&TypeRef>,
        context: &mut FunctionContext,
    ) -> ExpressionInfo {
        let (info, call, field) = match &expression.kind {
            ExpressionKind::Name(path) => {
                if let Some(TypeRef::Callback(callback)) = expected
                    && !self.value_name_resolves(path, context)
                {
                    (
                        self.resolve_callback_function_name(
                            path,
                            callback,
                            expression.span,
                            context,
                        ),
                        None,
                        None,
                    )
                } else {
                    let (info, field) = self.resolve_value_name(path, expression.span, context);
                    (info, None, field)
                }
            }
            ExpressionKind::Literal(literal) => (
                ExpressionInfo {
                    ty: literal_type(literal.kind, &literal.text, expected),
                    category: ValueCategory::Temporary,
                },
                None,
                None,
            ),
            ExpressionKind::Parenthesized(inner) => (
                self.resolve_expression(inner, expected, context),
                None,
                None,
            ),
            ExpressionKind::Prefix { operator, operand } => (
                self.resolve_prefix(*operator, operand, expected, context),
                None,
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
                None,
            ),
            ExpressionKind::Call { callee, arguments } => {
                let (info, call) =
                    self.resolve_call(callee, arguments, expected, expression.span, context);
                (info, call, None)
            }
            ExpressionKind::Aggregate { ty, initializers } => {
                let (info, call) =
                    self.resolve_struct_aggregate(ty, initializers, expression.span, context);
                (info, call, None)
            }
            ExpressionKind::Field { receiver, name } => {
                let (info, field) =
                    self.resolve_struct_field(receiver, name, expression.span, context);
                (info, None, field)
            }
            ExpressionKind::Index { receiver, index } => {
                self.resolve_expression(receiver, None, context);
                self.resolve_expression(index, Some(&TypeRef::Usize), context);
                self.push(
                    "RES011",
                    "indexing is not exposed by the initial Vec/String bindings".to_owned(),
                    expression.span,
                );
                (error_info(), None, None)
            }
            ExpressionKind::Lambda {
                captures,
                parameters,
                body,
            } => (
                self.resolve_lambda(
                    captures,
                    parameters,
                    body,
                    expected,
                    expression.span,
                    context,
                ),
                None,
                None,
            ),
            ExpressionKind::Error => (error_info(), None, None),
        };
        if let Some(call) = &call {
            for thrown in &call.throws {
                self.validate_checked_effect(*thrown, expression.span, context);
            }
        }
        self.record_expression(expression.span, info.clone(), call, field);
        info
    }

    fn resolve_value_name(
        &mut self,
        path: &ast::Path,
        span: Span,
        context: &FunctionContext,
    ) -> (ExpressionInfo, Option<ResolvedField>) {
        if path.segments.len() == 1 {
            let name = &path.segments[0];
            for scope in context.scopes.iter().rev() {
                if let Some(variable) = scope.get(name) {
                    return (
                        ExpressionInfo {
                            ty: variable.ty.clone(),
                            category: if variable.mutable {
                                ValueCategory::MutablePlace
                            } else {
                                ValueCategory::SharedPlace
                            },
                        },
                        None,
                    );
                }
            }
            if let Some(receiver) = &context.receiver
                && let Some((ty, access_path)) = self.lookup_struct_field(receiver.structure, path)
            {
                return (
                    ExpressionInfo {
                        ty,
                        category: if receiver.mutable {
                            ValueCategory::MutablePlace
                        } else {
                            ValueCategory::SharedPlace
                        },
                    },
                    Some(ResolvedField { access_path }),
                );
            }
        }
        self.push(
            "RES012",
            format!("unresolved value name `{}`", path.display()),
            span,
        );
        (error_info(), None)
    }

    fn value_name_resolves(&self, path: &ast::Path, context: &FunctionContext) -> bool {
        if path.segments.len() != 1 {
            return false;
        }
        let name = &path.segments[0];
        context
            .scopes
            .iter()
            .rev()
            .any(|scope| scope.contains_key(name))
            || context.receiver.as_ref().is_some_and(|receiver| {
                self.lookup_struct_field(receiver.structure, path).is_some()
            })
    }

    fn resolve_callback_function_name(
        &mut self,
        path: &ast::Path,
        callback: &CallbackType,
        span: Span,
        context: &FunctionContext,
    ) -> ExpressionInfo {
        let signature_candidates = self
            .function_candidates(path, &context.namespace)
            .into_iter()
            .filter(|id| {
                let function = &self.model.functions[id.0];
                function.receiver.is_none()
                    && function
                        .parameters
                        .iter()
                        .map(|parameter| canonical(&parameter.ty))
                        .eq(callback.parameters.iter().map(canonical))
                    && canonical(&function.return_type) == canonical(&callback.return_type)
            })
            .collect::<Vec<_>>();
        if signature_candidates.len() != 1 {
            self.push(
                "RES084",
                format!(
                    "callback function `{}` must resolve to exactly one non-member overload with signature ({}) -> {}",
                    path.display(),
                    callback
                        .parameters
                        .iter()
                        .map(display_type)
                        .collect::<Vec<_>>()
                        .join(", "),
                    display_type(&callback.return_type)
                ),
                span,
            );
            return error_info();
        }
        let id = signature_candidates[0];
        let function = &self.model.functions[id.0];
        if !function.has_definition {
            self.push(
                "RES084",
                format!(
                    "callback function `{}` has no definition",
                    display_path(&function.path)
                ),
                span,
            );
            return error_info();
        }
        if !function.throws.is_empty() {
            self.push(
                "RES085",
                "a callback passed to an ordinary Rust closure parameter cannot throw".to_owned(),
                span,
            );
            return error_info();
        }
        let ty = TypeRef::Callback(Box::new(callback.clone()));
        self.model.callbacks.push(ResolvedCallback {
            span,
            ty: ty.clone(),
            target: CallbackTarget::Function(id),
        });
        temporary(ty)
    }

    #[allow(clippy::too_many_arguments, clippy::too_many_lines)]
    fn resolve_lambda(
        &mut self,
        captures: &[ast::LambdaCapture],
        parameters: &[ast::Parameter],
        body: &ast::Block,
        expected: Option<&TypeRef>,
        span: Span,
        outer: &FunctionContext,
    ) -> ExpressionInfo {
        let Some(TypeRef::Callback(callback)) = expected else {
            self.push(
                "RES082",
                "a lambda is only valid as a contextually typed native callback argument"
                    .to_owned(),
                span,
            );
            return error_info();
        };
        if callback.escape != crate::interop::CallbackEscape::Call {
            self.push(
                "RES082",
                "only non-escaping callback lambdas are implemented".to_owned(),
                span,
            );
            return error_info();
        }
        if callback.kind == CallbackKind::FunctionPointer && !captures.is_empty() {
            self.push(
                "RES086",
                "`fn_ptr` callbacks require a captureless lambda or named function".to_owned(),
                span,
            );
        }

        let mut lambda_scope = BTreeMap::new();
        let mut resolved_captures = Vec::new();
        let mut seen_captures = BTreeSet::new();
        for capture in captures {
            if !seen_captures.insert(capture.name.as_str()) {
                self.push(
                    "RES087",
                    format!("lambda captures `{}` more than once", capture.name),
                    capture.span,
                );
                continue;
            }
            let outer_variable = outer
                .scopes
                .iter()
                .rev()
                .find_map(|scope| scope.get(&capture.name))
                .cloned();
            let Some(outer_variable) = outer_variable else {
                self.push(
                    "RES087",
                    format!("lambda capture `{}` is not a local binding", capture.name),
                    capture.span,
                );
                continue;
            };
            if outer_variable.ty.is_reference() {
                self.push(
                    "RES088",
                    "capturing a reference binding is deferred; capture its owner instead"
                        .to_owned(),
                    capture.span,
                );
                continue;
            }
            let (mode, inner_variable) = match &capture.kind {
                LambdaCaptureKind::Copy => {
                    if !is_copyable(&canonical(&outer_variable.ty)) {
                        self.push(
                            "RES089",
                            format!(
                                "copy capture `{}` requires a Stainless-copyable value; use `[{} = move({})]`",
                                capture.name, capture.name, capture.name
                            ),
                            capture.span,
                        );
                    }
                    (
                        LambdaCaptureMode::Copy,
                        Variable {
                            ty: outer_variable.ty.clone(),
                            mutable: false,
                        },
                    )
                }
                LambdaCaptureKind::Borrow => {
                    let mutable = outer_variable.mutable;
                    (
                        LambdaCaptureMode::Borrow { mutable },
                        Variable {
                            ty: if mutable {
                                TypeRef::mutable_ref(canonical(&outer_variable.ty))
                            } else {
                                TypeRef::shared_ref(canonical(&outer_variable.ty))
                            },
                            mutable,
                        },
                    )
                }
                LambdaCaptureKind::Initialize(initializer) => {
                    if !is_move_of_name(initializer, &capture.name) {
                        self.push(
                            "RES090",
                            format!(
                                "lambda initializer capture must be `[{} = move({})]`",
                                capture.name, capture.name
                            ),
                            capture.span,
                        );
                    }
                    (
                        LambdaCaptureMode::Move,
                        Variable {
                            ty: outer_variable.ty.clone(),
                            mutable: false,
                        },
                    )
                }
            };
            lambda_scope.insert(capture.name.clone(), inner_variable);
            resolved_captures.push(ResolvedLambdaCapture {
                name: capture.name.clone(),
                ty: outer_variable.ty,
                mode,
            });
        }

        if parameters.len() != callback.parameters.len() {
            self.push(
                "RES091",
                format!(
                    "callback requires {} lambda parameter(s), found {}",
                    callback.parameters.len(),
                    parameters.len()
                ),
                span,
            );
        }
        for (index, parameter) in parameters.iter().enumerate() {
            let resolved = self.resolve_type(&parameter.ty, &outer.namespace, false);
            if let Some(expected) = callback.parameters.get(index) {
                self.require_exact(expected, &resolved, parameter.ty.span, "lambda parameter");
            }
            let variable = Variable {
                mutable: parameter_mutability(parameter, &resolved),
                ty: resolved,
            };
            self.insert_variable(&mut lambda_scope, &parameter.name, variable, parameter.span);
        }

        let mut context = FunctionContext {
            namespace: outer.namespace.clone(),
            return_type: callback.return_type.as_ref().clone(),
            scopes: vec![lambda_scope],
            receiver: None,
            declared_throws: Vec::new(),
            handled_throws: Vec::new(),
            current_catch: None,
            is_lambda: true,
        };
        self.resolve_block(body, &mut context, false);

        let ty = TypeRef::Callback(callback.clone());
        self.model.callbacks.push(ResolvedCallback {
            span,
            ty: ty.clone(),
            target: CallbackTarget::Lambda {
                captures: resolved_captures,
            },
        });
        temporary(ty)
    }

    fn resolve_struct_aggregate(
        &mut self,
        path: &ast::Path,
        initializers: &[Expression],
        span: Span,
        context: &mut FunctionContext,
    ) -> (ExpressionInfo, Option<ResolvedCall>) {
        let Some(id) = self.lookup_struct_path(&path.segments, &context.namespace) else {
            for initializer in initializers {
                self.resolve_expression(initializer, None, context);
            }
            self.push(
                "RES046",
                format!("unresolved aggregate type `{}`", path.display()),
                span,
            );
            return (error_info(), None);
        };
        let structure = self.model.structs[id.0].clone();
        let mut expected = Vec::new();
        if let Some(base) = structure.base {
            expected.push(TypeRef::Struct {
                path: self.model.structs[base.0].path.clone(),
            });
        }
        expected.extend(structure.fields.iter().map(|field| field.ty.clone()));
        if initializers.len() != expected.len() {
            self.push(
                "RES047",
                format!(
                    "aggregate `{}` requires {} initializer(s), found {}",
                    display_path(&structure.path),
                    expected.len(),
                    initializers.len()
                ),
                span,
            );
        }
        for (index, initializer) in initializers.iter().enumerate() {
            let expected_type = expected.get(index);
            let mut actual =
                self.resolve_expression(initializer, expected_type.map(canonical_ref), context);
            if let Some(expected_type) = expected_type {
                actual = self.adapt_rust_result(expected_type, actual, initializer, context);
                self.validate_binding(expected_type, &actual, initializer.span, "initializer");
            }
        }
        let return_type = TypeRef::Struct {
            path: structure.path,
        };
        let call = ResolvedCall {
            span,
            target: CallTarget::Intrinsic(Intrinsic::StructAggregate { structure: id }),
            return_type: return_type.clone(),
            throws: Vec::new(),
        };
        (temporary(return_type), Some(call))
    }

    fn resolve_struct_field(
        &mut self,
        receiver: &Expression,
        name: &ast::Path,
        span: Span,
        context: &mut FunctionContext,
    ) -> (ExpressionInfo, Option<ResolvedField>) {
        let receiver_info = self.resolve_expression(receiver, None, context);
        let TypeRef::Struct { path } = canonical(&receiver_info.ty) else {
            if canonical(&receiver_info.ty) != TypeRef::Error {
                self.push(
                    "RES010",
                    format!(
                        "type `{}` has no data field `{}`",
                        display_type(&receiver_info.ty),
                        name.display()
                    ),
                    span,
                );
            }
            return (error_info(), None);
        };
        let Some(structure) = self.struct_by_path.get(&path).copied() else {
            return (error_info(), None);
        };
        let Some((ty, access_path)) = self.lookup_struct_field(structure, name) else {
            self.push(
                "RES010",
                format!(
                    "struct `{}` has no data field `{}`",
                    display_path(&path),
                    name.display()
                ),
                span,
            );
            return (error_info(), None);
        };
        (
            ExpressionInfo {
                ty,
                category: match receiver_info.category {
                    ValueCategory::MutablePlace => ValueCategory::MutablePlace,
                    ValueCategory::Temporary => ValueCategory::Temporary,
                    ValueCategory::SharedPlace => ValueCategory::SharedPlace,
                },
            },
            Some(ResolvedField { access_path }),
        )
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
            let mut right_info = self.resolve_expression(right, Some(&left_type), context);
            if left_info.category != ValueCategory::MutablePlace {
                self.push(
                    "RES013",
                    "assignment requires a mutable place".to_owned(),
                    left.span,
                );
            }
            if operator == BinaryOperator::Assign {
                right_info = self.adapt_rust_result(&left_type, right_info, right, context);
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
                self.resolve_method_call(receiver, name, arguments, span, context)
            }
            ExpressionKind::Name(path) => {
                if let Some(target) = primitive_type(&path.segments) {
                    return self.resolve_primitive_cast(target, arguments, span, context);
                }
                if let Some(structure) = self.lookup_struct_path(&path.segments, &context.namespace)
                {
                    if self.model.structs[structure.0].path == ["stainless", "Exception"] {
                        return self.resolve_exception_root_constructor(
                            structure, arguments, span, context,
                        );
                    }
                    return self.resolve_user_constructor(structure, arguments, span, context);
                }
                match self.lookup_native_instance(path, expected, context, span) {
                    NativeInstanceLookup::Resolved(instance) => {
                        let source_name = instance_short_name(&instance.type_path);
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

    fn resolve_method_call(
        &mut self,
        receiver: &Expression,
        name: &ast::Path,
        arguments: &[Expression],
        span: Span,
        context: &mut FunctionContext,
    ) -> (ExpressionInfo, Option<ResolvedCall>) {
        let receiver_info = self.resolve_expression(receiver, None, context);
        match canonical(&receiver_info.ty) {
            TypeRef::Struct { path } => {
                let Some(structure) = self.struct_by_path.get(&path).copied() else {
                    return (error_info(), None);
                };
                self.resolve_struct_method(
                    structure,
                    &receiver_info,
                    receiver.span,
                    name,
                    arguments,
                    span,
                    context,
                )
            }
            TypeRef::Native {
                path,
                arguments: type_arguments,
            } if name.segments.len() == 1 => {
                if path == "rust::Result" && name.segments[0] == "unwrap" {
                    return self.resolve_rust_result_unwrap(
                        receiver,
                        &receiver_info,
                        &type_arguments,
                        arguments,
                        span,
                        context,
                    );
                }
                let instance = NativeInstance {
                    type_path: path,
                    arguments: type_arguments,
                };
                self.resolve_native_callable(
                    &instance,
                    CallStyle::Method,
                    &name.segments[0],
                    arguments,
                    span,
                    Some((&receiver_info, receiver.span)),
                    context,
                )
            }
            ty => {
                for argument in arguments {
                    self.resolve_expression(argument, None, context);
                }
                if ty != TypeRef::Error {
                    self.push(
                        "RES020",
                        format!(
                            "type `{}` has no method `{}`",
                            display_type(&ty),
                            name.display()
                        ),
                        span,
                    );
                }
                (error_info(), None)
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn resolve_rust_result_unwrap(
        &mut self,
        receiver: &Expression,
        receiver_info: &ExpressionInfo,
        type_arguments: &[TypeRef],
        arguments: &[Expression],
        span: Span,
        context: &mut FunctionContext,
    ) -> (ExpressionInfo, Option<ResolvedCall>) {
        if !arguments.is_empty() {
            for argument in arguments {
                self.resolve_expression(argument, None, context);
            }
            self.push(
                "RES078",
                "native `Result.unwrap()` does not accept arguments".to_owned(),
                span,
            );
            return (error_info(), None);
        }
        let [value_type, error_type] = type_arguments else {
            self.push(
                "RES078",
                "native `Result` must have value and error type arguments".to_owned(),
                span,
            );
            return (error_info(), None);
        };
        let owned_name = receiver_info.category == ValueCategory::MutablePlace
            && !receiver_info.ty.is_reference()
            && is_named_value_expression(receiver);
        if receiver_info.category != ValueCategory::Temporary && !owned_name {
            self.push(
                "RES079",
                "native `Result.unwrap()` requires a temporary or owned mutable binding".to_owned(),
                receiver.span,
            );
            return (error_info(), None);
        }
        let rust_error = self.rust_error_struct();
        let call = ResolvedCall {
            span,
            target: CallTarget::Intrinsic(Intrinsic::UnwrapRustResult {
                error_message: self.rust_error_message(error_type),
            }),
            return_type: value_type.clone(),
            throws: vec![rust_error],
        };
        (temporary(value_type.clone()), Some(call))
    }

    #[allow(clippy::too_many_arguments, clippy::too_many_lines)]
    fn resolve_struct_method(
        &mut self,
        structure: StructId,
        receiver: &ExpressionInfo,
        receiver_span: Span,
        name: &ast::Path,
        arguments: &[Expression],
        span: Span,
        context: &mut FunctionContext,
    ) -> (ExpressionInfo, Option<ResolvedCall>) {
        if name.segments.len() != 1 {
            for argument in arguments {
                self.resolve_expression(argument, None, context);
            }
            self.push(
                "RES048",
                format!(
                    "qualified member call `{}` is not supported",
                    name.display()
                ),
                span,
            );
            return (error_info(), None);
        }
        let mut path = self.model.structs[structure.0].path.clone();
        path.push(name.segments[0].clone());
        let candidates = self.function_sets.get(&path).cloned().unwrap_or_default();
        let candidates = candidates
            .into_iter()
            .filter(|id| self.model.functions[id.0].parameters.len() == arguments.len())
            .collect::<Vec<_>>();
        if candidates.is_empty() {
            for argument in arguments {
                self.resolve_expression(argument, None, context);
            }
            self.push(
                "RES049",
                format!(
                    "struct `{}` has no member `{}` accepting {} argument(s)",
                    display_path(&self.model.structs[structure.0].path),
                    name.display(),
                    arguments.len()
                ),
                span,
            );
            return (error_info(), None);
        }
        let contextual = (candidates.len() == 1).then(|| {
            self.model.functions[candidates[0].0]
                .parameters
                .iter()
                .map(|parameter| canonical(&parameter.ty))
                .collect::<Vec<_>>()
        });
        let actual = self.resolve_arguments(arguments, contextual.as_deref(), context);
        let exact = candidates
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
            self.push(
                "RES019",
                format!(
                    "no exact member overload of `{}` matches ({})",
                    name.display(),
                    display_argument_types(&actual)
                ),
                span,
            );
            return (error_info(), None);
        }
        let id = exact[0];
        let symbol = self.model.functions[id.0].clone();
        if !symbol.has_definition {
            self.push(
                "RES052",
                format!(
                    "member `{}` has no out-of-struct definition",
                    display_path(&symbol.path)
                ),
                span,
            );
            return (error_info(), None);
        }
        let member = symbol
            .receiver
            .as_ref()
            .expect("member candidate has a receiver");
        if member.mutable && receiver.category != ValueCategory::MutablePlace {
            self.push(
                "RES024",
                format!("method `{}` requires a mutable receiver", name.display()),
                receiver_span,
            );
        }
        for ((parameter, argument), syntax) in symbol.parameters.iter().zip(&actual).zip(arguments)
        {
            self.validate_binding(&parameter.ty, argument, syntax.span, "argument");
        }
        let return_type = symbol.return_type;
        let call = ResolvedCall {
            span,
            target: CallTarget::Stainless(id),
            return_type: return_type.clone(),
            throws: symbol.throws,
        };
        (info_for_return_type(return_type), Some(call))
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
            throws: Vec::new(),
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
            throws: Vec::new(),
        };
        (temporary(target), Some(call))
    }

    fn resolve_exception_root_constructor(
        &mut self,
        structure: StructId,
        arguments: &[Expression],
        span: Span,
        context: &mut FunctionContext,
    ) -> (ExpressionInfo, Option<ResolvedCall>) {
        let expected = TypeRef::Native {
            path: "rust::String".to_owned(),
            arguments: Vec::new(),
        };
        if arguments.len() != 1 {
            for argument in arguments {
                self.resolve_expression(argument, None, context);
            }
            self.push(
                "RES073",
                "`stainless::Exception` requires one String message".to_owned(),
                span,
            );
            return (error_info(), None);
        }
        let actual = self.resolve_expression(&arguments[0], Some(&expected), context);
        self.validate_binding(&expected, &actual, arguments[0].span, "exception message");
        let return_type = TypeRef::Struct {
            path: self.model.structs[structure.0].path.clone(),
        };
        let call = ResolvedCall {
            span,
            target: CallTarget::Intrinsic(Intrinsic::ExceptionRoot { structure }),
            return_type: return_type.clone(),
            throws: Vec::new(),
        };
        (temporary(return_type), Some(call))
    }

    #[allow(clippy::too_many_lines)]
    fn resolve_user_constructor(
        &mut self,
        structure: StructId,
        arguments: &[Expression],
        span: Span,
        context: &mut FunctionContext,
    ) -> (ExpressionInfo, Option<ResolvedCall>) {
        let structure_symbol = self.model.structs[structure.0].clone();
        let candidates = self
            .constructor_sets
            .get(&structure)
            .cloned()
            .unwrap_or_default();
        let arity_candidates = candidates
            .into_iter()
            .filter(|id| self.model.constructors[id.0].parameters.len() == arguments.len())
            .collect::<Vec<_>>();
        if arity_candidates.is_empty() {
            if arguments.len() == 1 {
                let target = TypeRef::Struct {
                    path: structure_symbol.path,
                };
                let call =
                    self.resolve_direct_initialization(&target, &arguments[0], span, context);
                return match call {
                    Some(call) => (temporary(target), Some(call)),
                    None => (error_info(), None),
                };
            }
            for argument in arguments {
                self.resolve_expression(argument, None, context);
            }
            self.push(
                "RES059",
                format!(
                    "no constructor of `{}` accepts {} argument(s)",
                    display_path(&structure_symbol.path),
                    arguments.len()
                ),
                span,
            );
            return (error_info(), None);
        }

        let contextual_parameters = (arity_candidates.len() == 1).then(|| {
            self.model.constructors[arity_candidates[0].0]
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
                self.model.constructors[id.0]
                    .parameters
                    .iter()
                    .map(|parameter| canonical(&parameter.ty))
                    .eq(actual.iter().map(|argument| canonical(&argument.ty)))
            })
            .collect::<Vec<_>>();
        let compatible = if exact.is_empty() {
            arity_candidates
                .iter()
                .copied()
                .filter(|id| {
                    self.model.constructors[id.0]
                        .parameters
                        .iter()
                        .zip(&actual)
                        .all(|(parameter, argument)| {
                            self.is_derived_reference_binding(&parameter.ty, &argument.ty)
                        })
                })
                .collect::<Vec<_>>()
        } else {
            exact
        };
        if compatible.is_empty()
            && arguments.len() == 1
            && actual.first().is_some_and(|argument| {
                canonical(&argument.ty)
                    == TypeRef::Struct {
                        path: structure_symbol.path.clone(),
                    }
            })
        {
            let target = TypeRef::Struct {
                path: structure_symbol.path,
            };
            self.validate_binding(
                &target,
                &actual[0],
                arguments[0].span,
                "copy-constructor argument",
            );
            let call = ResolvedCall {
                span,
                target: CallTarget::Intrinsic(Intrinsic::ValueInitialization {
                    target: target.clone(),
                }),
                return_type: target.clone(),
                throws: Vec::new(),
            };
            return (temporary(target), Some(call));
        }
        if compatible.len() != 1 {
            let displayed_candidates = if compatible.is_empty() {
                &arity_candidates
            } else {
                &compatible
            }
            .iter()
            .map(|id| {
                display_constructor_signature(&self.model.constructors[id.0], &structure_symbol)
            })
            .collect::<Vec<_>>()
            .join("; ");
            let message = if compatible.is_empty() {
                format!(
                    "no exact constructor of `{}` matches ({}); candidates: {displayed_candidates}",
                    display_path(&structure_symbol.path),
                    display_argument_types(&actual)
                )
            } else {
                format!(
                    "constructor call for `{}` is ambiguous for ({}); candidates: {displayed_candidates}",
                    display_path(&structure_symbol.path),
                    display_argument_types(&actual)
                )
            };
            self.push("RES060", message, span);
            return (error_info(), None);
        }

        let id = compatible[0];
        let symbol = self.model.constructors[id.0].clone();
        for ((parameter, argument), expression) in
            symbol.parameters.iter().zip(&actual).zip(arguments)
        {
            self.validate_binding(
                &parameter.ty,
                argument,
                expression.span,
                "constructor argument",
            );
        }
        if symbol.is_deleted {
            self.push(
                "RES066",
                format!(
                    "selected constructor of `{}` is deleted",
                    display_path(&structure_symbol.path)
                ),
                span,
            );
            return (error_info(), None);
        }
        if !symbol.has_definition {
            self.push(
                "RES067",
                format!(
                    "selected constructor of `{}` has no out-of-struct definition",
                    display_path(&structure_symbol.path)
                ),
                span,
            );
            return (error_info(), None);
        }
        let return_type = TypeRef::Struct {
            path: structure_symbol.path,
        };
        let call = ResolvedCall {
            span,
            target: CallTarget::Constructor(id),
            return_type: return_type.clone(),
            throws: symbol.throws,
        };
        (temporary(return_type), Some(call))
    }

    #[allow(clippy::too_many_lines)]
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
        let compatible = if exact.is_empty() {
            arity_candidates
                .iter()
                .copied()
                .filter(|id| {
                    self.model.functions[id.0]
                        .parameters
                        .iter()
                        .zip(&actual)
                        .all(|(parameter, argument)| {
                            canonical(&parameter.ty) == canonical(&argument.ty)
                                || self.is_derived_reference_binding(&parameter.ty, &argument.ty)
                        })
                })
                .collect::<Vec<_>>()
        } else {
            exact
        };
        if compatible.len() != 1 {
            let displayed_candidates = if compatible.is_empty() {
                &arity_candidates
            } else {
                &compatible
            }
            .iter()
            .map(|id| display_function_signature(&self.model.functions[id.0]))
            .collect::<Vec<_>>()
            .join("; ");
            let message = if compatible.is_empty() {
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

        let id = compatible[0];
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
            throws: symbol.throws,
        };
        (info_for_return_type(return_type), Some(call))
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
            type_path: instance.type_path.clone(),
            style,
            source_name: candidate.callable.source_name,
            receiver: candidate.callable.receiver,
            receiver_type: candidate.callable.receiver.map(|_| TypeRef::Native {
                path: instance.type_path.clone(),
                arguments: instance.arguments.clone(),
            }),
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
            target: CallTarget::Native(Box::new(native_call)),
            return_type: candidate.return_type.clone(),
            throws: Vec::new(),
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
        let binding = self.bindings.type_by_path(&instance.type_path)?;
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
        if !self.is_derived_reference_binding(expected, &actual.ty) {
            self.require_exact(expected, &actual.ty, span, description);
        }
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

    fn is_derived_reference_binding(&self, expected: &TypeRef, actual: &TypeRef) -> bool {
        let TypeRef::Reference {
            target: expected_target,
            ..
        } = expected
        else {
            return false;
        };
        let TypeRef::Struct {
            path: expected_path,
        } = canonical_ref(expected_target)
        else {
            return false;
        };
        let TypeRef::Struct { path: actual_path } = canonical_ref(actual) else {
            return false;
        };
        if expected_path == actual_path {
            return true;
        }
        let Some(mut current) = self.struct_by_path.get(actual_path).copied() else {
            return false;
        };
        while let Some(base) = self.model.structs[current.0].base {
            if self.model.structs[base.0].path == *expected_path {
                return true;
            }
            current = base;
        }
        false
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

    fn resolve_default_construction(
        &mut self,
        ty: &TypeRef,
        span: Span,
        context: &mut FunctionContext,
    ) {
        if let Some(call) = self.resolve_default_call(ty, span, context) {
            for thrown in &call.throws {
                self.validate_checked_effect(*thrown, call.span, context);
            }
            self.model.calls.push(call);
        }
    }

    fn resolve_exception_set(
        &mut self,
        syntax: &[ast::Type],
        namespace: &[String],
        declaration_span: Span,
    ) -> Vec<StructId> {
        let mut exceptions = Vec::new();
        for ty in syntax {
            let resolved = self.resolve_type(ty, namespace, false);
            if resolved.is_reference() {
                self.push(
                    "RES070",
                    "a `throws` entry must be an exception struct value type".to_owned(),
                    ty.span,
                );
                continue;
            }
            let TypeRef::Struct { path } = canonical(&resolved) else {
                if resolved != TypeRef::Error {
                    self.push(
                        "RES070",
                        format!(
                            "checked exception `{}` is not a struct",
                            display_type(&resolved)
                        ),
                        ty.span,
                    );
                }
                continue;
            };
            let Some(id) = self.struct_by_path.get(&path).copied() else {
                continue;
            };
            if !self.is_exception_struct(id) {
                self.push(
                    "RES070",
                    format!(
                        "`{}` does not derive from `stainless::Exception`",
                        display_path(&path)
                    ),
                    ty.span,
                );
                continue;
            }
            if exceptions.contains(&id) {
                self.push(
                    "RES071",
                    format!("duplicate checked exception `{}`", display_path(&path)),
                    ty.span,
                );
                continue;
            }
            exceptions.push(id);
        }
        exceptions.sort_by(|left, right| {
            self.model.structs[left.0]
                .path
                .cmp(&self.model.structs[right.0].path)
        });
        for (index, derived) in exceptions.iter().enumerate() {
            if exceptions
                .iter()
                .enumerate()
                .any(|(other, base)| index != other && self.exception_covers(*base, *derived))
            {
                self.push(
                    "RES072",
                    format!(
                        "checked exception `{}` is redundant because a listed base covers it",
                        display_path(&self.model.structs[derived.0].path)
                    ),
                    declaration_span,
                );
            }
        }
        exceptions
    }

    fn is_exception_struct(&self, structure: StructId) -> bool {
        let path = vec!["stainless".to_owned(), "Exception".to_owned()];
        let Some(root) = self.struct_by_path.get(&path).copied() else {
            return false;
        };
        self.exception_covers(root, structure)
    }

    fn exception_covers(&self, base: StructId, thrown: StructId) -> bool {
        let mut current = Some(thrown);
        while let Some(id) = current {
            if id == base {
                return true;
            }
            current = self.model.structs[id.0].base;
        }
        false
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
        let Some(binding) = self.bindings.type_by_path(&type_path) else {
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
                    if let Some(id) = self.lookup_struct_path(segments, namespace) {
                        if !arguments.is_empty() {
                            self.push(
                                "RES050",
                                format!(
                                    "struct `{}` cannot have type arguments",
                                    named.path.display()
                                ),
                                ty.span,
                            );
                        }
                        TypeRef::Struct {
                            path: self.model.structs[id.0].path.clone(),
                        }
                    } else {
                        let Some(path) = self.native_path(segments, namespace, true, ty.span)
                        else {
                            return TypeRef::Error;
                        };
                        let expected_arity = self.bindings.type_by_path(&path).map_or_else(
                            || native_container_arity(&path),
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
    ) -> Option<String> {
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

    fn lookup_struct_path(&self, segments: &[String], namespace: &[String]) -> Option<StructId> {
        let mut candidates = Vec::new();
        if segments.first().is_some_and(|segment| segment == "crate") {
            candidates.push(segments[1..].to_vec());
        } else if segments.len() > 1 {
            let mut relative = namespace.to_vec();
            relative.extend(segments.iter().cloned());
            candidates.push(relative);
            candidates.push(segments.to_vec());
        } else if let Some(name) = segments.first() {
            candidates.extend(self.imports.candidates(namespace, name));
            for depth in (0..=namespace.len()).rev() {
                let mut candidate = namespace[..depth].to_vec();
                candidate.push(name.clone());
                candidates.push(candidate);
            }
        }
        candidates.sort();
        candidates.dedup();
        let found = candidates
            .into_iter()
            .filter_map(|candidate| self.struct_by_path.get(&candidate).copied())
            .collect::<BTreeSet<_>>();
        if found.len() == 1 {
            found.into_iter().next()
        } else {
            None
        }
    }

    fn lookup_struct_field(
        &self,
        structure: StructId,
        requested: &ast::Path,
    ) -> Option<(TypeRef, Vec<String>)> {
        let (field_name, qualification) = requested.segments.split_last()?;
        if qualification.is_empty() {
            let mut matches = Vec::new();
            let mut access_path = Vec::new();
            let mut current = Some(structure);
            while let Some(id) = current {
                let symbol = &self.model.structs[id.0];
                if let Some(field) = symbol.fields.iter().find(|field| field.name == *field_name) {
                    let mut field_path = access_path.clone();
                    field_path.push(field.name.clone());
                    matches.push((field.ty.clone(), field_path));
                }
                let Some(base) = symbol.base else {
                    break;
                };
                access_path.push(base_field_name(&self.model.structs[base.0]));
                current = Some(base);
            }
            return (matches.len() == 1).then(|| matches.remove(0));
        }
        let target = { self.find_base_by_suffix(structure, qualification)? };
        let mut access_path = self.base_projection_path(structure, target)?;
        let mut current = Some(target);
        while let Some(id) = current {
            let symbol = &self.model.structs[id.0];
            if let Some(field) = symbol.fields.iter().find(|field| field.name == *field_name) {
                access_path.push(field.name.clone());
                return Some((field.ty.clone(), access_path));
            }
            let base = symbol.base?;
            access_path.push(base_field_name(&self.model.structs[base.0]));
            current = Some(base);
        }
        None
    }

    fn find_base_by_suffix(&self, structure: StructId, suffix: &[String]) -> Option<StructId> {
        let mut current = Some(structure);
        while let Some(id) = current {
            let path = &self.model.structs[id.0].path;
            if path.ends_with(suffix) {
                return Some(id);
            }
            current = self.model.structs[id.0].base;
        }
        None
    }

    fn base_projection_path(&self, derived: StructId, base: StructId) -> Option<Vec<String>> {
        let mut path = Vec::new();
        let mut current = derived;
        while current != base {
            let next = self.model.structs[current.0].base?;
            path.push(base_field_name(&self.model.structs[next.0]));
            current = next;
        }
        Some(path)
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

    fn record_expression(
        &mut self,
        span: Span,
        info: ExpressionInfo,
        call: Option<ResolvedCall>,
        field: Option<ResolvedField>,
    ) {
        if let Some(call) = &call {
            self.model.calls.push(call.clone());
        }
        self.model.expressions.push(ExpressionResolution {
            span,
            ty: info.ty,
            category: info.category,
            call,
            field,
        });
    }

    fn rust_error_message(&self, ty: &TypeRef) -> RustErrorMessage {
        let display = match canonical_ref(ty) {
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
            | TypeRef::F64 => true,
            TypeRef::Native { path, arguments } => {
                if *path == "rust::String" && arguments.is_empty() {
                    return RustErrorMessage::Display;
                }
                return match self
                    .bindings
                    .type_by_path(path)
                    .and_then(|binding| binding.error_format)
                {
                    Some(NativeErrorFormat::Display) => RustErrorMessage::Display,
                    Some(NativeErrorFormat::Debug) => RustErrorMessage::Debug,
                    None => RustErrorMessage::Fallback,
                };
            }
            TypeRef::Void
            | TypeRef::Parameter(_)
            | TypeRef::Callback(_)
            | TypeRef::Struct { .. }
            | TypeRef::Reference { .. }
            | TypeRef::Error => false,
        };
        if display {
            RustErrorMessage::Display
        } else {
            RustErrorMessage::Fallback
        }
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
        .cloned()
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
                    .get(&requirement.parameter)
                    .cloned()
                    .unwrap_or(TypeRef::Error),
                rust_trait: requirement.rust_trait.clone(),
            })
            .collect(),
    }
}

fn substitute(ty: &TypeRef, substitutions: &BTreeMap<String, TypeRef>) -> TypeRef {
    match ty {
        TypeRef::Parameter(name) => substitutions.get(name).cloned().unwrap_or(TypeRef::Error),
        TypeRef::Native { path, arguments } => TypeRef::Native {
            path: path.clone(),
            arguments: arguments
                .iter()
                .map(|argument| substitute(argument, substitutions))
                .collect(),
        },
        TypeRef::Callback(callback) => TypeRef::callback(
            callback.kind,
            callback.escape,
            callback
                .parameters
                .iter()
                .map(|parameter| substitute(parameter, substitutions))
                .collect(),
            substitute(&callback.return_type, substitutions),
        ),
        TypeRef::Struct { .. } => ty.clone(),
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

fn known_native_path(segments: &[String], bindings: &NativeBindings) -> Option<String> {
    let path = segments.join("::");
    bindings
        .type_by_path(&path)
        .map(|binding| binding.stainless_path.clone())
        .or(match path.as_str() {
            "rust::Option" => Some("rust::Option".to_owned()),
            "rust::Result" => Some("rust::Result".to_owned()),
            _ => None,
        })
}

fn is_named_value_expression(expression: &Expression) -> bool {
    match &expression.kind {
        ExpressionKind::Name(path) => path.segments.len() == 1,
        ExpressionKind::Parenthesized(inner) => is_named_value_expression(inner),
        _ => false,
    }
}

fn is_move_of_name(expression: &Expression, name: &str) -> bool {
    let ExpressionKind::Call { callee, arguments } = &expression.kind else {
        return false;
    };
    let ExpressionKind::Name(callee) = &callee.kind else {
        return false;
    };
    let [argument] = arguments.as_slice() else {
        return false;
    };
    let ExpressionKind::Name(argument) = &argument.kind else {
        return false;
    };
    callee.segments == ["move"] && argument.segments == [name]
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
            | TypeRef::Struct { .. }
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
        TypeRef::Parameter(name) => (*name).clone(),
        TypeRef::Native { path, arguments } if arguments.is_empty() => (*path).clone(),
        TypeRef::Native { path, arguments } => format!(
            "{path}<{}>",
            arguments
                .iter()
                .map(display_type)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        TypeRef::Callback(callback) => format!(
            "callback<{:?}({}) -> {}>",
            callback.kind,
            callback
                .parameters
                .iter()
                .map(display_type)
                .collect::<Vec<_>>()
                .join(", "),
            display_type(&callback.return_type)
        ),
        TypeRef::Struct { path } => display_path(path),
        TypeRef::Reference { mutable, target } => {
            if *mutable {
                format!("{}&", display_type(target))
            } else {
                format!("const {}&", display_type(target))
            }
        }
    }
}

fn base_field_name(structure: &StructSymbol) -> String {
    format!(
        "__stainless_base_{}",
        structure.path.last().map_or("missing", String::as_str)
    )
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

fn display_constructor_signature(
    constructor: &crate::resolution::ConstructorSymbol,
    structure: &StructSymbol,
) -> String {
    format!(
        "{}({})",
        display_path(&structure.path),
        constructor
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
        path: instance.type_path.clone(),
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

#[allow(clippy::too_many_lines)]
fn source_uses_exceptions(source: &SourceFile) -> bool {
    fn block_uses_exceptions(block: &ast::Block) -> bool {
        block.statements.iter().any(statement_uses_exceptions)
    }

    fn expression_uses_exceptions(expression: &Expression) -> bool {
        match &expression.kind {
            ExpressionKind::Name(_) | ExpressionKind::Literal(_) | ExpressionKind::Error => false,
            ExpressionKind::Parenthesized(inner)
            | ExpressionKind::Prefix { operand: inner, .. }
            | ExpressionKind::Postfix { operand: inner, .. } => expression_uses_exceptions(inner),
            ExpressionKind::Binary { left, right, .. } => {
                expression_uses_exceptions(left) || expression_uses_exceptions(right)
            }
            ExpressionKind::Call { callee, arguments } => {
                expression_uses_exceptions(callee)
                    || arguments.iter().any(expression_uses_exceptions)
            }
            ExpressionKind::Aggregate { initializers, .. } => {
                initializers.iter().any(expression_uses_exceptions)
            }
            ExpressionKind::Field { receiver, .. } => expression_uses_exceptions(receiver),
            ExpressionKind::Index { receiver, index } => {
                expression_uses_exceptions(receiver) || expression_uses_exceptions(index)
            }
            ExpressionKind::Lambda { captures, body, .. } => {
                captures.iter().any(|capture| {
                    matches!(
                        &capture.kind,
                        LambdaCaptureKind::Initialize(initializer)
                            if expression_uses_exceptions(initializer)
                    )
                }) || block_uses_exceptions(body)
            }
        }
    }

    fn local_uses_exceptions(local: &ast::LocalDeclaration) -> bool {
        local
            .initializer
            .as_ref()
            .is_some_and(expression_uses_exceptions)
    }

    fn statement_uses_exceptions(statement: &Statement) -> bool {
        match &statement.kind {
            StatementKind::Throw(_) | StatementKind::Try(_) => true,
            StatementKind::Block(block) => block_uses_exceptions(block),
            StatementKind::Local(local) => local_uses_exceptions(local),
            StatementKind::Return(value) => value.as_ref().is_some_and(expression_uses_exceptions),
            StatementKind::If(statement) => {
                expression_uses_exceptions(&statement.condition)
                    || statement_uses_exceptions(&statement.then_branch)
                    || statement
                        .else_branch
                        .as_deref()
                        .is_some_and(statement_uses_exceptions)
            }
            StatementKind::For(statement) => {
                let clause_uses_exceptions = match &statement.clause {
                    ast::ForClause::Range(range) => expression_uses_exceptions(&range.iterable),
                    ast::ForClause::Classic(classic) => {
                        classic
                            .initializer
                            .as_ref()
                            .is_some_and(|initializer| match initializer {
                                ast::ForInitializer::Local(local) => local_uses_exceptions(local),
                                ast::ForInitializer::Expression(expression) => {
                                    expression_uses_exceptions(expression)
                                }
                            })
                            || classic
                                .condition
                                .as_ref()
                                .is_some_and(expression_uses_exceptions)
                            || classic
                                .update
                                .as_ref()
                                .is_some_and(expression_uses_exceptions)
                    }
                    ast::ForClause::Error => false,
                };
                clause_uses_exceptions || statement_uses_exceptions(&statement.body)
            }
            StatementKind::Expression(expression) => expression_uses_exceptions(expression),
            StatementKind::Break
            | StatementKind::Continue
            | StatementKind::Empty
            | StatementKind::Error => false,
        }
    }

    fn items_use_exceptions(items: &[Item]) -> bool {
        items.iter().any(|item| match item {
            Item::Namespace(namespace) => items_use_exceptions(&namespace.items),
            Item::Struct(structure) => {
                structure.base.as_ref().is_some_and(|base| {
                    matches!(
                        base.segments.as_slice(),
                        [namespace, name]
                            if namespace == "stainless"
                                && matches!(name.as_str(), "Exception" | "RustError")
                    )
                }) || structure.constructors.iter().any(|constructor| {
                    !constructor.throws.is_empty()
                        || constructor.body.as_ref().is_some_and(block_uses_exceptions)
                }) || structure.functions.iter().any(|function| {
                    !function.throws.is_empty()
                        || function.body.as_ref().is_some_and(block_uses_exceptions)
                })
            }
            Item::Constructor(constructor) => {
                !constructor.throws.is_empty()
                    || constructor.body.as_ref().is_some_and(block_uses_exceptions)
            }
            Item::Function(function) => {
                !function.throws.is_empty()
                    || function.body.as_ref().is_some_and(block_uses_exceptions)
            }
            Item::Use(_) => false,
        })
    }

    items_use_exceptions(&source.items)
}
