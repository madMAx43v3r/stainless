use std::collections::{BTreeMap, BTreeSet};

use crate::Diagnostic;
use crate::ast::{
    self, BinaryOperator, Expression, ExpressionKind, ForClause, ForInitializer, Item,
    LambdaCaptureKind, LiteralKind, PrefixOperator, SourceFile, Span, Statement, StatementKind,
    TypeKind,
};
use crate::interop::{
    CallStyle, CallableBinding, CallbackKind, NativeBindings, NativeErrorFormat, NativeTypeBinding,
    PointerKind, Receiver, StoredFunctionKind, TypeRef, VAR_TYPE_PATH,
};

use super::imports::ImportTable;
use super::mangle;
use super::{
    BindingResolution, CallTarget, CallbackTarget, ConstructorFieldInitialization, ConstructorId,
    ConstructorSymbol, ExpressionResolution, FieldSymbol, FunctionId, FunctionSymbol,
    InterfaceImplementation, Intrinsic, LambdaCaptureMode, NativeCall, NativeCallResultAdaptation,
    NativeResultException, ParameterSymbol, Resolution, ResolvedCall, ResolvedCallback,
    ResolvedField, ResolvedLambdaCapture, ResolvedNativeType, ResolvedStaticConstant,
    ResolvedTraitRequirement, RustErrorMessage, RustResultAdaptation, SemanticModel,
    StaticConstantSymbol, StructId, StructReceiver, StructSymbol, ValueCategory,
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
        resolving_negated_integer_literal: false,
    };
    if source_uses_exceptions(source) {
        resolver.install_exception_builtins();
    }
    resolver.collect_struct_names(&source.items, &mut Vec::new());
    resolver.resolve_struct_definitions(&source.items, &mut Vec::new());
    resolver.validate_struct_cycles();
    resolver.collect_signatures(&source.items, &mut Vec::new());
    resolver.synthesize_default_constructors();
    resolver.validate_interface_contracts();
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
    resolving_negated_integer_literal: bool,
}

#[derive(Clone, Debug)]
struct Variable {
    ty: TypeRef,
    mutable: bool,
    null_state: NullState,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum NullState {
    Null,
    NonNull,
    Unknown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ConstructionSyntax {
    Parenthesized,
    Braced,
}

struct FunctionContext {
    namespace: Vec<String>,
    type_parameters: Vec<String>,
    return_type: TypeRef,
    scopes: Vec<BTreeMap<String, Variable>>,
    receiver: Option<StructReceiver>,
    declared_throws: Vec<StructId>,
    handled_throws: Vec<Vec<StructId>>,
    current_catch: Option<StructId>,
    is_lambda: bool,
    is_async: bool,
    awaiting_call: Option<Span>,
}

#[derive(Clone, Debug)]
struct ExpressionInfo {
    ty: TypeRef,
    category: ValueCategory,
}

#[derive(Clone, Debug)]
struct StructFieldLookup {
    ty: TypeRef,
    access_path: Vec<String>,
    owner: StructId,
    is_public: bool,
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
    rust_result_error: Option<TypeRef>,
    requirements: Vec<ResolvedTraitRequirement>,
}

impl Resolver<'_> {
    fn install_exception_builtins(&mut self) {
        let root = StructId(self.model.structs.len());
        let root_path = vec!["stainless".to_owned(), "Exception".to_owned()];
        self.model.structs.push(StructSymbol {
            id: root,
            path: root_path.clone(),
            type_parameters: Vec::new(),
            kind: ast::UserTypeKind::Struct,
            base: None,
            interfaces: Vec::new(),
            is_sealed: false,
            fields: vec![FieldSymbol {
                name: "message".to_owned(),
                is_public: true,
                ty: TypeRef::Native {
                    path: "rust::String".to_owned(),
                    arguments: Vec::new(),
                },
                span: Span::default(),
            }],
            static_constants: Vec::new(),
            span: Span::default(),
        });
        self.struct_by_path.insert(root_path, root);

        let rust_error = StructId(self.model.structs.len());
        let rust_error_path = vec!["stainless".to_owned(), "RustError".to_owned()];
        self.model.structs.push(StructSymbol {
            id: rust_error,
            path: rust_error_path.clone(),
            type_parameters: Vec::new(),
            kind: ast::UserTypeKind::Struct,
            base: Some(root),
            interfaces: Vec::new(),
            is_sealed: false,
            fields: Vec::new(),
            static_constants: Vec::new(),
            span: Span::default(),
        });
        self.struct_by_path.insert(rust_error_path, rust_error);

        let io_error = StructId(self.model.structs.len());
        let io_error_path = vec!["stainless".to_owned(), "IoError".to_owned()];
        self.model.structs.push(StructSymbol {
            id: io_error,
            path: io_error_path.clone(),
            type_parameters: Vec::new(),
            kind: ast::UserTypeKind::Struct,
            base: Some(root),
            interfaces: Vec::new(),
            is_sealed: false,
            fields: Vec::new(),
            static_constants: Vec::new(),
            span: Span::default(),
        });
        self.struct_by_path.insert(io_error_path, io_error);

        let format_error = StructId(self.model.structs.len());
        let format_error_path = vec!["stainless".to_owned(), "FormatError".to_owned()];
        self.model.structs.push(StructSymbol {
            id: format_error,
            path: format_error_path.clone(),
            type_parameters: Vec::new(),
            kind: ast::UserTypeKind::Struct,
            base: Some(root),
            interfaces: Vec::new(),
            is_sealed: false,
            fields: Vec::new(),
            static_constants: Vec::new(),
            span: Span::default(),
        });
        self.struct_by_path.insert(format_error_path, format_error);

        let json_error = StructId(self.model.structs.len());
        let json_error_path = vec!["stainless".to_owned(), "JsonError".to_owned()];
        self.model.structs.push(StructSymbol {
            id: json_error,
            path: json_error_path.clone(),
            type_parameters: Vec::new(),
            kind: ast::UserTypeKind::Struct,
            base: Some(root),
            interfaces: Vec::new(),
            is_sealed: false,
            fields: Vec::new(),
            static_constants: Vec::new(),
            span: Span::default(),
        });
        self.struct_by_path.insert(json_error_path, json_error);

        let thread_error = StructId(self.model.structs.len());
        let thread_error_path = vec!["stainless".to_owned(), "ThreadError".to_owned()];
        self.model.structs.push(StructSymbol {
            id: thread_error,
            path: thread_error_path.clone(),
            type_parameters: Vec::new(),
            kind: ast::UserTypeKind::Struct,
            base: Some(root),
            interfaces: Vec::new(),
            is_sealed: false,
            fields: Vec::new(),
            static_constants: Vec::new(),
            span: Span::default(),
        });
        self.struct_by_path.insert(thread_error_path, thread_error);
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
                    let mut parameters = BTreeSet::new();
                    for parameter in &structure.type_parameters {
                        if parameter.starts_with("__") {
                            self.push(
                                "RES124",
                                format!(
                                    "generic parameter `{parameter}` uses the reserved `__` prefix"
                                ),
                                structure.span,
                            );
                        }
                        if !parameters.insert(parameter.clone()) {
                            self.push(
                                "RES124",
                                format!("duplicate generic parameter `{parameter}`"),
                                structure.span,
                            );
                        }
                    }
                    if structure.kind == ast::UserTypeKind::Interface
                        && !structure.type_parameters.is_empty()
                    {
                        self.push(
                            "RES124",
                            "generic interfaces are not implemented yet; generic structs and classes are supported"
                                .to_owned(),
                            structure.span,
                        );
                    }
                    let id = StructId(self.model.structs.len());
                    self.model.structs.push(StructSymbol {
                        id,
                        path: path.clone(),
                        type_parameters: structure.type_parameters.clone(),
                        kind: structure.kind,
                        base: None,
                        interfaces: Vec::new(),
                        is_sealed: structure.is_sealed,
                        fields: Vec::new(),
                        static_constants: Vec::new(),
                        span: structure.span,
                    });
                    self.struct_by_path.insert(path, id);
                    self.struct_by_span.insert(structure.span, id);
                }
                Item::Use(_) | Item::Constructor(_) | Item::Function(_) => {}
            }
        }
    }

    #[allow(clippy::too_many_lines)]
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
                    let mut base = None;
                    let mut interfaces = Vec::new();
                    if !structure.type_parameters.is_empty() && !structure.bases.is_empty() {
                        self.push(
                            "RES124",
                            "inheritance and interface implementation on generic types are deferred"
                                .to_owned(),
                            structure.span,
                        );
                    }
                    for base_syntax in &structure.bases {
                        if !structure.type_parameters.is_empty() {
                            continue;
                        }
                        let TypeKind::Named(named) = &base_syntax.kind else {
                            self.push(
                                "RES040",
                                "a base declaration must name a user-defined type".to_owned(),
                                base_syntax.span,
                            );
                            continue;
                        };
                        if base_syntax.is_const
                            || base_syntax.is_reference
                            || !named.arguments.is_empty()
                        {
                            self.push(
                                "RES040",
                                "base declarations cannot be const, references, or generic instances"
                                    .to_owned(),
                                base_syntax.span,
                            );
                            continue;
                        }
                        let Some(found) = self.lookup_struct_path(&named.path.segments, namespace)
                        else {
                            self.push(
                                "RES040",
                                format!("unresolved base type `{}`", named.path.display()),
                                base_syntax.span,
                            );
                            continue;
                        };
                        if found == id {
                            self.push(
                                "RES041",
                                "a type cannot inherit from or implement itself".to_owned(),
                                base_syntax.span,
                            );
                            continue;
                        }
                        let base_kind = self.model.structs[found.0].kind;
                        let base_path = self.model.structs[found.0].path.clone();
                        let base_sealed = self.model.structs[found.0].is_sealed;
                        if base_sealed
                            && base_path[..base_path.len().saturating_sub(1)]
                                != self.model.structs[id.0].path
                                    [..self.model.structs[id.0].path.len().saturating_sub(1)]
                        {
                            self.push(
                                "RES118",
                                format!(
                                    "sealed type `{}` cannot be inherited or implemented outside its module",
                                    display_path(&base_path)
                                ),
                                base_syntax.span,
                            );
                        }
                        match (structure.kind, base_kind) {
                            (ast::UserTypeKind::Struct, ast::UserTypeKind::Struct) => {
                                if base.replace(found).is_some() {
                                    self.push(
                                        "RES118",
                                        "a struct may have only one data base".to_owned(),
                                        base_syntax.span,
                                    );
                                }
                            }
                            (
                                ast::UserTypeKind::Struct
                                | ast::UserTypeKind::Class
                                | ast::UserTypeKind::Interface,
                                ast::UserTypeKind::Interface,
                            ) => {
                                if interfaces.contains(&found) {
                                    self.push(
                                        "RES118",
                                        format!(
                                            "duplicate interface base `{}`",
                                            display_path(&base_path)
                                        ),
                                        base_syntax.span,
                                    );
                                } else {
                                    interfaces.push(found);
                                }
                            }
                            (ast::UserTypeKind::Class, _) => self.push(
                                "RES118",
                                "a class may inherit only from interfaces".to_owned(),
                                base_syntax.span,
                            ),
                            (ast::UserTypeKind::Interface, _) => self.push(
                                "RES118",
                                "an interface may inherit only from interfaces".to_owned(),
                                base_syntax.span,
                            ),
                            (ast::UserTypeKind::Struct, ast::UserTypeKind::Class) => self.push(
                                "RES118",
                                "a struct cannot inherit from a class".to_owned(),
                                base_syntax.span,
                            ),
                        }
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
                            let ty = self.resolve_type(
                                &field.ty,
                                namespace,
                                &structure.type_parameters,
                                false,
                            );
                            self.reject_bare_interface_type(
                                &ty,
                                field.ty.span,
                                "data field",
                            );
                            if ty.contains_reference() {
                                self.push(
                                    "RES043",
                                    "references are not allowed as data fields".to_owned(),
                                    field.ty.span,
                                );
                            }
                            if structure.kind == ast::UserTypeKind::Struct
                                && contains_move_only_storage(&ty)
                            {
                                self.push(
                                    "RES092",
                                    "move-only ownership cannot be stored in an implicitly copyable struct"
                                        .to_owned(),
                                    field.ty.span,
                                );
                            }
                            FieldSymbol {
                                name: field.name.clone(),
                                is_public: field.is_public,
                                ty,
                                span: field.span,
                            }
                        })
                        .collect();
                    let static_constants = structure
                        .static_constants
                        .iter()
                        .map(|constant| {
                            if !names.insert(constant.name.clone()) {
                                self.push(
                                    "RES125",
                                    format!("duplicate struct member `{}`", constant.name),
                                    constant.span,
                                );
                            }
                            if structure.kind != ast::UserTypeKind::Struct {
                                self.push(
                                    "RES125",
                                    "`static const` members are supported only on structs"
                                        .to_owned(),
                                    constant.span,
                                );
                            }
                            if !structure.type_parameters.is_empty() {
                                self.push(
                                    "RES125",
                                    "`static const` members on generic structs are deferred"
                                        .to_owned(),
                                    constant.span,
                                );
                            }
                            if !constant.ty.is_const {
                                self.push(
                                    "RES125",
                                    "a static struct member must be declared `const`".to_owned(),
                                    constant.ty.span,
                                );
                            }
                            let ty = self.resolve_type(
                                &constant.ty,
                                namespace,
                                &structure.type_parameters,
                                false,
                            );
                            if !is_integer(&ty) && ty != TypeRef::Error {
                                self.push(
                                    "RES125",
                                    format!(
                                        "a static struct constant requires an integer type, found `{}`",
                                        display_type(&ty)
                                    ),
                                    constant.ty.span,
                                );
                            }
                            let value = if let ExpressionKind::Literal(literal) =
                                &constant.initializer.kind
                                && literal.kind == LiteralKind::Integer
                            {
                                let actual = literal_type(
                                    literal.kind,
                                    &literal.text,
                                    Some(&ty),
                                );
                                if is_integer(&ty)
                                    && actual != ty
                                    && actual != TypeRef::Error
                                {
                                    self.push(
                                        "RES125",
                                        format!(
                                            "static constant initializer has type `{}`, expected `{}`",
                                            display_type(&actual),
                                            display_type(&ty)
                                        ),
                                        constant.initializer.span,
                                    );
                                }
                                literal.text.clone()
                            } else {
                                self.push(
                                    "RES125",
                                    "a static struct constant currently requires an integer literal initializer"
                                        .to_owned(),
                                    constant.initializer.span,
                                );
                                "0".to_owned()
                            };
                            StaticConstantSymbol {
                                name: constant.name.clone(),
                                is_public: constant.is_public,
                                ty,
                                value,
                                span: constant.span,
                            }
                        })
                        .collect();
                    let symbol = &mut self.model.structs[id.0];
                    symbol.base = base;
                    symbol.interfaces = interfaces;
                    symbol.fields = fields;
                    symbol.static_constants = static_constants;
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
            if structure.kind == ast::UserTypeKind::Interface {
                self.validate_interface_cycle(structure.id, structure.id, &mut BTreeSet::new());
            }
        }
    }

    fn validate_interface_cycle(
        &mut self,
        root: StructId,
        current: StructId,
        visiting: &mut BTreeSet<StructId>,
    ) {
        if !visiting.insert(current) {
            if current == root {
                self.push(
                    "RES118",
                    format!(
                        "interface inheritance cycle involving `{}`",
                        display_path(&self.model.structs[root.0].path)
                    ),
                    self.model.structs[root.0].span,
                );
            }
            return;
        }
        let bases = self.model.structs[current.0].interfaces.clone();
        for base in bases {
            self.validate_interface_cycle(root, base, visiting);
        }
        visiting.remove(&current);
    }

    fn validate_interface_contracts(&mut self) {
        let structures = self.model.structs.clone();
        for implementer in structures
            .iter()
            .filter(|structure| structure.kind != ast::UserTypeKind::Interface)
        {
            let mut interfaces = BTreeSet::new();
            for interface in &implementer.interfaces {
                self.collect_interface_closure(*interface, &mut interfaces);
            }
            for interface in interfaces {
                let requirements = self
                    .model
                    .functions
                    .iter()
                    .filter(|function| {
                        function
                            .receiver
                            .as_ref()
                            .is_some_and(|receiver| receiver.structure == interface)
                    })
                    .map(|function| function.id)
                    .collect::<Vec<_>>();
                let mut methods = Vec::new();
                let mut complete = true;
                for requirement_id in requirements {
                    let requirement = self.model.functions[requirement_id.0].clone();
                    let method_name = requirement.path.last().cloned().unwrap_or_default();
                    let mut implementation_path = implementer.path.clone();
                    implementation_path.push(method_name.clone());
                    let matches =
                        self.function_sets
                            .get(&implementation_path)
                            .into_iter()
                            .flatten()
                            .copied()
                            .filter(|candidate| {
                                let candidate = &self.model.functions[candidate.0];
                                candidate.has_definition
                                    && candidate
                                        .parameters
                                        .iter()
                                        .map(|parameter| &parameter.ty)
                                        .eq(requirement
                                            .parameters
                                            .iter()
                                            .map(|parameter| &parameter.ty))
                                    && candidate.receiver.as_ref().is_some_and(|receiver| {
                                        receiver.structure == implementer.id
                                            && receiver.mutable
                                                == requirement
                                                    .receiver
                                                    .as_ref()
                                                    .is_some_and(|receiver| receiver.mutable)
                                    })
                                    && self.interface_return_matches(
                                        &requirement.return_type,
                                        &candidate.return_type,
                                        interface,
                                        implementer.id,
                                    )
                                    && candidate.throws.iter().all(|thrown| {
                                        requirement.throws.iter().any(|declared| {
                                            self.exception_covers(*declared, *thrown)
                                        })
                                    })
                            })
                            .collect::<Vec<_>>();
                    if matches.len() == 1 {
                        methods.push((requirement_id, matches[0]));
                    } else {
                        complete = false;
                        self.push(
                            "RES120",
                            format!(
                                "type `{}` does not provide exactly one implementation of `{}::{}`",
                                display_path(&implementer.path),
                                display_path(&self.model.structs[interface.0].path),
                                method_name
                            ),
                            implementer.span,
                        );
                    }
                }
                if complete {
                    self.model
                        .interface_implementations
                        .push(InterfaceImplementation {
                            implementer: implementer.id,
                            interface,
                            methods,
                        });
                }
            }
        }
    }

    fn collect_interface_closure(&self, interface: StructId, collected: &mut BTreeSet<StructId>) {
        if !collected.insert(interface) {
            return;
        }
        for base in &self.model.structs[interface.0].interfaces {
            self.collect_interface_closure(*base, collected);
        }
    }

    fn interface_return_matches(
        &self,
        required: &TypeRef,
        actual: &TypeRef,
        interface: StructId,
        implementer: StructId,
    ) -> bool {
        if required == actual {
            return true;
        }
        match (required, actual) {
            (
                TypeRef::Reference {
                    mutable: required_mutable,
                    target: required_target,
                },
                TypeRef::Reference {
                    mutable: actual_mutable,
                    target: actual_target,
                },
            ) => {
                required_mutable == actual_mutable
                    && canonical_ref(required_target)
                        == &resolved_structure_type(&self.model.structs[interface.0])
                    && canonical_ref(actual_target)
                        == &resolved_structure_type(&self.model.structs[implementer.0])
            }
            _ => false,
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
        self.validate_qualified_owner_arguments(
            &constructor.owner_arguments,
            owner,
            namespace,
            declared_owner.is_some(),
            constructor.span,
        );
        if self.model.structs[owner.0].kind == ast::UserTypeKind::Interface {
            self.push(
                "RES118",
                "interfaces cannot declare constructors".to_owned(),
                constructor.span,
            );
            return;
        }
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
            .map(|parameter| {
                let ty = self.resolve_type(
                    &parameter.ty,
                    type_namespace,
                    &self.model.structs[owner.0].type_parameters.clone(),
                    false,
                );
                self.reject_bare_interface_type(&ty, parameter.ty.span, "parameter");
                ParameterSymbol {
                    name: parameter.name.clone(),
                    ty,
                    span: parameter.span,
                }
            })
            .collect::<Vec<_>>();
        if self.model.structs[owner.0].kind == ast::UserTypeKind::Class
            && matches!(parameters.as_slice(), [parameter]
                if canonical(&parameter.ty)
                    == resolved_structure_type(&self.model.structs[owner.0]))
        {
            self.push(
                "RES119",
                "classes cannot declare copy or move constructors; class relocation is provided only by `move(...)`"
                    .to_owned(),
                constructor.span,
            );
        }
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
            let definition_inherits_throws =
                constructor.body.is_some() && constructor.throws.is_empty();
            let different_throws =
                !definition_inherits_throws && self.model.constructors[id.0].throws != throws;
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
            if declared_owner.is_some() {
                symbol.is_public = constructor.is_public;
            }
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
            is_public: constructor.is_public,
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
            if structure.kind == ast::UserTypeKind::Interface {
                continue;
            }
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
                is_public: true,
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
            TypeRef::Condition
            | TypeRef::Pointer {
                kind:
                    PointerKind::UniqueNullable
                    | PointerKind::SharedNullable
                    | PointerKind::Weak
                    | PointerKind::AtomicNullable,
                ..
            } => true,
            TypeRef::Tuple(elements) => elements
                .iter()
                .all(|element| self.type_has_default_constructor(element, visiting)),
            TypeRef::Native { path, .. } => {
                self.bindings.type_by_path(path).is_some_and(|binding| {
                    binding.callables.iter().any(|callable| {
                        callable.style == CallStyle::Constructor && callable.parameters.is_empty()
                    })
                })
            }
            TypeRef::Struct { path, .. } | TypeRef::Class { path, .. } => self
                .struct_by_path
                .get(path)
                .is_some_and(|id| self.struct_has_default_constructor(*id, visiting)),
            TypeRef::Error
            | TypeRef::Void
            | TypeRef::Bool
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
            | TypeRef::Parameter(_)
            | TypeRef::Callback(_)
            | TypeRef::Function(_)
            | TypeRef::Pointer { .. }
            | TypeRef::Mutex(_)
            | TypeRef::MutexGuard(_)
            | TypeRef::RwLock(_)
            | TypeRef::RwLockReadGuard(_)
            | TypeRef::RwLockWriteGuard(_)
            | TypeRef::ThreadHandle(_)
            | TypeRef::ThreadScope
            | TypeRef::ScopedThreadHandle(_)
            | TypeRef::Interface { .. }
            | TypeRef::Reference { .. } => false,
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
        if let Some(owner) = owner {
            self.validate_qualified_owner_arguments(
                &function.owner_arguments,
                owner,
                namespace,
                declared_owner.is_some(),
                function.span,
            );
        }
        let type_namespace = owner.map_or_else(
            || namespace.to_vec(),
            |owner| {
                let path = &self.model.structs[owner.0].path;
                path[..path.len().saturating_sub(1)].to_vec()
            },
        );
        let owner_type_parameters = owner.map_or_else(Vec::new, |owner| {
            self.model.structs[owner.0].type_parameters.clone()
        });
        let parameters = function
            .parameters
            .iter()
            .map(|parameter| {
                let ty = self.resolve_type(
                    &parameter.ty,
                    &type_namespace,
                    &owner_type_parameters,
                    false,
                );
                self.reject_bare_interface_type(&ty, parameter.ty.span, "parameter");
                ParameterSymbol {
                    name: parameter.name.clone(),
                    ty,
                    span: parameter.span,
                }
            })
            .collect::<Vec<_>>();
        let signature = parameters
            .iter()
            .map(|parameter| canonical(&parameter.ty))
            .collect::<Vec<_>>();
        let existing_ids = self.function_sets.get(&path).cloned().unwrap_or_default();
        let matches_static_declaration = declared_owner.is_none()
            && owner.is_some()
            && existing_ids.iter().any(|id| {
                let existing = &self.model.functions[id.0];
                existing.owner == owner
                    && existing.receiver.is_none()
                    && existing.has_member_declaration
                    && existing
                        .parameters
                        .iter()
                        .map(|parameter| &parameter.ty)
                        .eq(parameters.iter().map(|parameter| &parameter.ty))
            });
        if function.is_static && declared_owner.is_none() {
            self.push(
                "RES045",
                "`static` appears only on an in-body member declaration".to_owned(),
                function.span,
            );
        }
        let is_static = function.is_static || matches_static_declaration;
        if is_static && function.is_const {
            self.push(
                "RES045",
                "a static member function cannot have a trailing `const`".to_owned(),
                function.span,
            );
        }
        if is_static
            && owner.is_some_and(|owner| {
                self.model.structs[owner.0].kind == ast::UserTypeKind::Interface
            })
        {
            self.push(
                "RES045",
                "interfaces cannot declare static member functions".to_owned(),
                function.span,
            );
        }
        let receiver = owner
            .filter(|_| !is_static)
            .map(|structure| StructReceiver {
                structure,
                mutable: !function.is_const,
            });
        let mut return_type = self.resolve_type(
            &function.return_type,
            &type_namespace,
            &owner_type_parameters,
            false,
        );
        self.reject_bare_interface_type(&return_type, function.return_type.span, "return value");
        if function.is_async && return_type.is_reference() {
            self.push(
                "RES123",
                "async functions cannot currently return references".to_owned(),
                function.return_type.span,
            );
        }
        if function.is_async
            && owner.is_some_and(|owner| {
                self.model.structs[owner.0].kind == ast::UserTypeKind::Interface
            })
        {
            self.push(
                "RES123",
                "async interface methods are deferred; use a Rust async callback boundary"
                    .to_owned(),
                function.span,
            );
        }
        let throws = self.resolve_exception_set(&function.throws, &type_namespace, function.span);
        if return_type == TypeRef::Void
            && let Some(receiver) = &receiver
        {
            return_type = TypeRef::Reference {
                mutable: receiver.mutable,
                target: Box::new(resolved_structure_type(
                    &self.model.structs[receiver.structure.0],
                )),
            };
        }
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
            let definition_inherits_throws = function.body.is_some() && function.throws.is_empty();
            let different_throws = !definition_inherits_throws && existing.throws != throws;
            let different_async = existing.is_async != function.is_async;
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
                if different_async {
                    self.push(
                        "RES123",
                        format!(
                            "declarations of `{}` disagree on `async`",
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
            if declared_owner.is_some() {
                symbol.is_public = function.is_public;
            }
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
            is_public: function.is_public,
            is_async: function.is_async,
            path: path.clone(),
            parameters,
            return_type,
            throws,
            owner,
            receiver,
            mangled_name,
            declarations: vec![function.span],
            has_definition: function.body.is_some(),
            has_member_declaration: declared_owner.is_some(),
        });
        self.function_sets.entry(path).or_default().push(id);
        self.function_by_span.insert(function.span, id);
    }

    fn validate_qualified_owner_arguments(
        &mut self,
        arguments: &[ast::Type],
        owner: StructId,
        namespace: &[String],
        is_member_declaration: bool,
        span: Span,
    ) {
        let parameters = self.model.structs[owner.0].type_parameters.clone();
        if is_member_declaration {
            if !arguments.is_empty() {
                self.push(
                    "RES124",
                    "an in-body member declaration must not repeat owner type arguments".to_owned(),
                    span,
                );
            }
            return;
        }
        if arguments.len() != parameters.len() {
            self.push(
                "RES124",
                format!(
                    "qualified definition of `{}` must repeat owner arguments `<{}>`",
                    display_path(&self.model.structs[owner.0].path),
                    parameters.join(", ")
                ),
                span,
            );
            return;
        }
        for (argument, parameter) in arguments.iter().zip(&parameters) {
            let resolved = self.resolve_type(argument, namespace, &parameters, false);
            if resolved != TypeRef::Parameter(parameter.clone()) {
                self.push(
                    "RES124",
                    format!(
                        "qualified definition owner argument must be `{parameter}`, found `{}`",
                        display_type(&resolved)
                    ),
                    argument.span,
                );
            }
        }
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
            if function.owner.is_some() && !function.has_member_declaration {
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
                    null_state: initial_null_state(&parameter.ty),
                },
            );
        }
        let mut initialization_context = FunctionContext {
            namespace: constructor_namespace.clone(),
            type_parameters: structure.type_parameters.clone(),
            return_type: TypeRef::Void,
            scopes: vec![scope.clone()],
            receiver: None,
            declared_throws: symbol.throws.clone(),
            handled_throws: Vec::new(),
            current_catch: None,
            is_lambda: false,
            is_async: false,
            awaiting_call: None,
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
            type_parameters: structure.type_parameters.clone(),
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
            is_async: false,
            awaiting_call: None,
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
                type_parameters: structure.type_parameters.clone(),
                return_type: TypeRef::Void,
                scopes: vec![BTreeMap::new()],
                receiver: None,
                declared_throws: Vec::new(),
                handled_throws: Vec::new(),
                current_catch: None,
                is_lambda: false,
                is_async: false,
                awaiting_call: None,
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
                    arguments: Vec::new(),
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

    #[allow(clippy::too_many_lines)]
    fn resolve_slot_construction(
        &mut self,
        target: &TypeRef,
        arguments: &[Expression],
        span: Span,
        context: &mut FunctionContext,
    ) -> Option<ResolvedCall> {
        match canonical_ref(target) {
            TypeRef::Tuple(elements) => {
                self.resolve_tuple_slot_construction(elements, arguments, span, context)
            }
            TypeRef::Mutex(protected) => {
                self.resolve_mutex_slot_construction(protected, arguments, span, context)
            }
            TypeRef::RwLock(protected) => {
                self.resolve_rwlock_slot_construction(protected, arguments, span, context)
            }
            TypeRef::Condition => self.resolve_condition_construction(arguments, span, context),
            TypeRef::MutexGuard(_) | TypeRef::RwLockReadGuard(_) | TypeRef::RwLockWriteGuard(_) => {
                for argument in arguments {
                    self.resolve_expression(argument, None, context);
                }
                self.push(
                    "RES113",
                    "lock guards can only be produced by `lock()`, `read()`, or `write()`"
                        .to_owned(),
                    span,
                );
                None
            }
            TypeRef::Pointer { .. } => {
                for argument in arguments {
                    self.resolve_expression(argument, None, context);
                }
                self.push(
                    "RES106",
                    "constructing a nested `unique_ptr` pointee is not implemented".to_owned(),
                    span,
                );
                None
            }
            TypeRef::Struct { path, .. } | TypeRef::Class { path, .. } => {
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
                self.resolve_user_constructor(
                    structure,
                    canonical_ref(target).clone(),
                    arguments,
                    span,
                    context,
                )
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
            TypeRef::Parameter(_) if arguments.len() == 1 => {
                self.resolve_direct_initialization(target, &arguments[0], span, context)
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

    fn resolve_mutex_slot_construction(
        &mut self,
        protected: &TypeRef,
        arguments: &[Expression],
        span: Span,
        context: &mut FunctionContext,
    ) -> Option<ResolvedCall> {
        let construction = self.resolve_slot_construction(protected, arguments, span, context)?;
        let throws = construction.throws.clone();
        Some(ResolvedCall {
            span,
            target: CallTarget::Intrinsic(Intrinsic::MutexNew {
                target: protected.clone(),
                construction: Box::new(construction),
            }),
            return_type: TypeRef::Mutex(Box::new(protected.clone())),
            throws,
        })
    }

    fn resolve_tuple_slot_construction(
        &mut self,
        elements: &[TypeRef],
        arguments: &[Expression],
        span: Span,
        context: &mut FunctionContext,
    ) -> Option<ResolvedCall> {
        if !arguments.is_empty() && elements.len() != arguments.len() {
            for argument in arguments {
                self.resolve_expression(argument, None, context);
            }
            self.push(
                "RES119",
                format!(
                    "tuple construction requires {} arguments, found {}",
                    elements.len(),
                    arguments.len()
                ),
                span,
            );
            return None;
        }
        let mut constructions = Vec::with_capacity(elements.len());
        let mut throws = Vec::new();
        for (index, element) in elements.iter().enumerate() {
            let construction = if let Some(argument) = arguments.get(index) {
                self.resolve_slot_construction(
                    element,
                    std::slice::from_ref(argument),
                    argument.span,
                    context,
                )?
            } else {
                self.resolve_slot_construction(element, &[], span, context)?
            };
            throws.extend(construction.throws.iter().copied());
            constructions.push(construction);
        }
        throws.sort_unstable();
        throws.dedup();
        Some(ResolvedCall {
            span,
            target: CallTarget::Intrinsic(Intrinsic::TupleNew { constructions }),
            return_type: TypeRef::Tuple(elements.to_vec()),
            throws,
        })
    }

    fn resolve_rwlock_slot_construction(
        &mut self,
        protected: &TypeRef,
        arguments: &[Expression],
        span: Span,
        context: &mut FunctionContext,
    ) -> Option<ResolvedCall> {
        let construction = self.resolve_slot_construction(protected, arguments, span, context)?;
        let throws = construction.throws.clone();
        Some(ResolvedCall {
            span,
            target: CallTarget::Intrinsic(Intrinsic::RwLockNew {
                target: protected.clone(),
                construction: Box::new(construction),
            }),
            return_type: TypeRef::RwLock(Box::new(protected.clone())),
            throws,
        })
    }

    fn resolve_condition_construction(
        &mut self,
        arguments: &[Expression],
        span: Span,
        context: &mut FunctionContext,
    ) -> Option<ResolvedCall> {
        for argument in arguments {
            self.resolve_expression(argument, None, context);
        }
        if !arguments.is_empty() {
            self.push(
                "RES113",
                format!(
                    "`condition` requires no constructor arguments, found {}",
                    arguments.len()
                ),
                span,
            );
            return None;
        }
        Some(ResolvedCall {
            span,
            target: CallTarget::Intrinsic(Intrinsic::ConditionNew),
            return_type: TypeRef::Condition,
            throws: Vec::new(),
        })
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

    #[allow(clippy::too_many_lines)]
    fn resolve_default_call(
        &mut self,
        target: &TypeRef,
        span: Span,
        context: &mut FunctionContext,
    ) -> Option<ResolvedCall> {
        match canonical_ref(target) {
            TypeRef::Tuple(elements) => {
                let mut constructions = Vec::with_capacity(elements.len());
                let mut throws = Vec::new();
                for element in elements {
                    let construction = self.resolve_default_call(element, span, context)?;
                    throws.extend(construction.throws.iter().copied());
                    constructions.push(construction);
                }
                throws.sort_unstable();
                throws.dedup();
                Some(ResolvedCall {
                    span,
                    target: CallTarget::Intrinsic(Intrinsic::TupleNew { constructions }),
                    return_type: TypeRef::Tuple(elements.clone()),
                    throws,
                })
            }
            TypeRef::Mutex(protected) => {
                let construction = self.resolve_default_call(protected, span, context)?;
                let throws = construction.throws.clone();
                Some(ResolvedCall {
                    span,
                    target: CallTarget::Intrinsic(Intrinsic::MutexNew {
                        target: protected.as_ref().clone(),
                        construction: Box::new(construction),
                    }),
                    return_type: TypeRef::Mutex(protected.clone()),
                    throws,
                })
            }
            TypeRef::RwLock(protected) => {
                let construction = self.resolve_default_call(protected, span, context)?;
                let throws = construction.throws.clone();
                Some(ResolvedCall {
                    span,
                    target: CallTarget::Intrinsic(Intrinsic::RwLockNew {
                        target: protected.as_ref().clone(),
                        construction: Box::new(construction),
                    }),
                    return_type: TypeRef::RwLock(protected.clone()),
                    throws,
                })
            }
            TypeRef::Condition => Some(ResolvedCall {
                span,
                target: CallTarget::Intrinsic(Intrinsic::ConditionNew),
                return_type: TypeRef::Condition,
                throws: Vec::new(),
            }),
            TypeRef::MutexGuard(_) | TypeRef::RwLockReadGuard(_) | TypeRef::RwLockWriteGuard(_) => {
                self.push(
                    "RES113",
                    "lock guards cannot be default-constructed".to_owned(),
                    span,
                );
                None
            }
            TypeRef::Pointer { kind, target }
                if matches!(
                    kind,
                    PointerKind::UniqueNullable
                        | PointerKind::SharedNullable
                        | PointerKind::Weak
                        | PointerKind::AtomicNullable
                ) =>
            {
                Some(ResolvedCall {
                    span,
                    target: CallTarget::Intrinsic(Intrinsic::PointerDefault {
                        kind: *kind,
                        target: target.as_ref().clone(),
                    }),
                    return_type: TypeRef::pointer(*kind, target.as_ref().clone()),
                    throws: Vec::new(),
                })
            }
            TypeRef::Pointer { kind, .. } => {
                self.push(
                    "RES106",
                    format!("`{}<T>` has no default constructor", pointer_name(*kind)),
                    span,
                );
                None
            }
            TypeRef::Struct { path, .. } | TypeRef::Class { path, .. } => {
                let structure = self.struct_by_path.get(path).copied()?;
                self.resolve_user_constructor(
                    structure,
                    canonical_ref(target).clone(),
                    &[],
                    span,
                    context,
                )
                .1
            }
            TypeRef::Interface { path, .. } => {
                self.push(
                    "RES118",
                    format!(
                        "interface `{}` has no value constructor",
                        display_path(path)
                    ),
                    span,
                );
                None
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
        let function_namespace = symbol.owner.map_or_else(
            || symbol.path[..symbol.path.len().saturating_sub(1)].to_vec(),
            |owner| {
                let path = &self.model.structs[owner.0].path;
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
                    null_state: initial_null_state(&symbol_parameter.ty),
                },
            );
        }
        let mut context = FunctionContext {
            namespace: function_namespace,
            type_parameters: symbol.owner.map_or_else(Vec::new, |owner| {
                self.model.structs[owner.0].type_parameters.clone()
            }),
            return_type: symbol.return_type,
            scopes: vec![initial_scope],
            receiver: symbol.receiver,
            declared_throws: symbol.throws,
            handled_throws: Vec::new(),
            current_catch: None,
            is_lambda: false,
            is_async: symbol.is_async,
            awaiting_call: None,
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

    #[allow(clippy::too_many_lines)]
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
                        if !expected.is_reference()
                            && move_call_argument(value)
                                .is_some_and(|source| local_return_move_source(source, context))
                        {
                            self.warn(
                                "RES126",
                                "redundant `move(...)` in return; this local is moved automatically"
                                    .to_owned(),
                                value.span,
                            );
                        }
                        let actual = if !expected.is_reference()
                            && !actual.ty.is_reference()
                            && !self.is_copyable_type(&canonical(&expected))
                            && local_return_move_source(value, context)
                        {
                            ExpressionInfo {
                                ty: actual.ty,
                                category: ValueCategory::Temporary,
                            }
                        } else {
                            actual
                        };
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
                if canonical(&condition.ty) != TypeRef::Bool
                    && !is_nullable_pointer_test(&condition.ty)
                {
                    self.push(
                        "RES110",
                        format!(
                            "if condition requires `bool` or a nullable pointer, found `{}`",
                            display_type(&condition.ty)
                        ),
                        if_statement.condition.span,
                    );
                }

                let baseline = context.scopes.clone();
                context.scopes = baseline.clone();
                Self::refine_null_condition(&if_statement.condition, true, context);
                self.resolve_statement(&if_statement.then_branch, context);
                let then_scopes = context.scopes.clone();
                let then_falls_through = statement_may_fall_through(&if_statement.then_branch);

                context.scopes = baseline;
                Self::refine_null_condition(&if_statement.condition, false, context);
                if let Some(else_branch) = &if_statement.else_branch {
                    self.resolve_statement(else_branch, context);
                }
                let else_scopes = context.scopes.clone();
                let else_falls_through = if_statement
                    .else_branch
                    .as_deref()
                    .is_none_or(statement_may_fall_through);
                context.scopes = merge_null_scopes(
                    &then_scopes,
                    then_falls_through,
                    &else_scopes,
                    else_falls_through,
                );
            }
            StatementKind::For(for_statement) => {
                let before_loop = context.scopes.clone();
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
                            if canonical(&actual.ty) != TypeRef::Bool
                                && !is_nullable_pointer_test(&actual.ty)
                            {
                                self.push(
                                    "RES110",
                                    format!(
                                        "for condition requires `bool` or a nullable pointer, found `{}`",
                                        display_type(&actual.ty)
                                    ),
                                    condition.span,
                                );
                            }
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
                context.scopes = merge_null_scopes(&before_loop, true, &context.scopes, true);
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
            let TypeRef::Struct { path, .. } = canonical(&actual.ty) else {
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

    #[allow(clippy::too_many_lines)]
    fn resolve_try_statement(
        &mut self,
        try_statement: &ast::TryStatement,
        context: &mut FunctionContext,
    ) {
        let baseline_scopes = context.scopes.clone();
        let root = self.exception_root();
        let mut catches = Vec::new();
        let mut resolved_catches = Vec::new();
        for catch in &try_statement.catches {
            let caught = if let Some(binding) = &catch.binding {
                let resolved = self.resolve_type(
                    &binding.ty,
                    &context.namespace,
                    &context.type_parameters,
                    false,
                );
                if let TypeRef::Struct { path, .. } = canonical(&resolved) {
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
        let mut continuing_scopes = block_may_fall_through(&try_statement.body)
            .then(|| context.scopes.clone())
            .into_iter()
            .collect::<Vec<_>>();
        let exceptional_scopes = unknown_nullable_scopes(&baseline_scopes);

        for (catch, caught) in try_statement.catches.iter().zip(resolved_catches) {
            context.scopes.clone_from(&exceptional_scopes);
            context.scopes.push(BTreeMap::new());
            if let Some(binding) = &catch.binding
                && let Some(caught) = caught
            {
                let path = self.model.structs[caught.0].path.clone();
                let ty = TypeRef::Reference {
                    mutable: false,
                    target: Box::new(TypeRef::Struct {
                        path,
                        arguments: Vec::new(),
                    }),
                };
                let variable = Variable {
                    ty: ty.clone(),
                    mutable: false,
                    null_state: NullState::NonNull,
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
            if block_may_fall_through(&catch.body) {
                continuing_scopes.push(context.scopes.clone());
            }
        }
        context.scopes = continuing_scopes
            .into_iter()
            .reduce(|left, right| merge_null_scopes(&left, true, &right, true))
            .unwrap_or(baseline_scopes);
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

    fn format_error_struct(&mut self) -> StructId {
        if self.exception_root().is_none() {
            self.install_exception_builtins();
        }
        let path = vec!["stainless".to_owned(), "FormatError".to_owned()];
        self.struct_by_path
            .get(&path)
            .copied()
            .expect("installing exception builtins creates FormatError")
    }

    fn io_error_struct(&mut self) -> StructId {
        if self.exception_root().is_none() {
            self.install_exception_builtins();
        }
        let path = vec!["stainless".to_owned(), "IoError".to_owned()];
        self.struct_by_path
            .get(&path)
            .copied()
            .expect("installing exception builtins creates IoError")
    }

    fn json_error_struct(&mut self) -> StructId {
        if self.exception_root().is_none() {
            self.install_exception_builtins();
        }
        let path = vec!["stainless".to_owned(), "JsonError".to_owned()];
        self.struct_by_path
            .get(&path)
            .copied()
            .expect("installing exception builtins creates JsonError")
    }

    fn thread_error_struct(&mut self) -> StructId {
        if self.exception_root().is_none() {
            self.install_exception_builtins();
        }
        let path = vec!["stainless".to_owned(), "ThreadError".to_owned()];
        self.struct_by_path
            .get(&path)
            .copied()
            .expect("installing exception builtins creates ThreadError")
    }

    fn native_result_exception(
        &mut self,
        error_type: &TypeRef,
    ) -> (StructId, NativeResultException) {
        match canonical_ref(error_type) {
            TypeRef::Native { path, arguments }
                if path == "rust::stainless_runtime::JsonError" && arguments.is_empty() =>
            {
                (self.json_error_struct(), NativeResultException::JsonError)
            }
            TypeRef::Native { path, arguments }
                if path == "rust::std::io::Error" && arguments.is_empty() =>
            {
                (self.io_error_struct(), NativeResultException::IoError)
            }
            _ => (self.rust_error_struct(), NativeResultException::RustError),
        }
    }

    fn resolve_local(&mut self, local: &ast::LocalDeclaration, context: &mut FunctionContext) {
        let declared = if local.ty.is_inferred() {
            None
        } else {
            let ty = self.resolve_type(
                &local.ty,
                &context.namespace,
                &context.type_parameters,
                false,
            );
            self.reject_bare_interface_type(&ty, local.ty.span, "local variable");
            Some(ty)
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

        let null_state = local.initializer.as_ref().map_or_else(
            || default_constructed_null_state(&resolved_type),
            |initializer| self.expression_null_state(initializer, context),
        );
        let variable = Variable {
            mutable: if resolved_type.is_reference() {
                matches!(resolved_type, TypeRef::Reference { mutable: true, .. })
            } else {
                !local.ty.is_const
            },
            ty: resolved_type,
            null_state,
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

    fn expression_null_state(
        &self,
        expression: &Expression,
        context: &FunctionContext,
    ) -> NullState {
        match &expression.kind {
            ExpressionKind::Name(path) if path.segments.len() == 1 => context
                .scopes
                .iter()
                .rev()
                .find_map(|scope| scope.get(&path.segments[0]))
                .map_or(NullState::Unknown, |variable| variable.null_state),
            ExpressionKind::Parenthesized(inner) => self.expression_null_state(inner, context),
            ExpressionKind::Literal(ast::Literal {
                kind: LiteralKind::Null,
                ..
            }) => NullState::Null,
            ExpressionKind::Call { arguments, .. } => {
                let intrinsic = self
                    .model
                    .expression(expression.span)
                    .and_then(|resolution| resolution.call.as_ref())
                    .and_then(|call| match &call.target {
                        CallTarget::Intrinsic(intrinsic) => Some(intrinsic),
                        _ => None,
                    });
                match intrinsic {
                    Some(Intrinsic::PointerDefault { .. }) => NullState::Null,
                    Some(Intrinsic::MakeOwner { .. }) => NullState::NonNull,
                    Some(
                        Intrinsic::Move
                        | Intrinsic::ValueInitialization { .. }
                        | Intrinsic::PointerConversion { .. },
                    ) => arguments.first().map_or(NullState::Unknown, |argument| {
                        self.expression_null_state(argument, context)
                    }),
                    Some(Intrinsic::LockWeak { .. } | Intrinsic::AtomicLoad { .. }) => {
                        NullState::Unknown
                    }
                    _ => self
                        .model
                        .expression(expression.span)
                        .map_or(NullState::Unknown, |resolution| {
                            initial_null_state(&resolution.ty)
                        }),
                }
            }
            _ => self
                .model
                .expression(expression.span)
                .map_or(NullState::Unknown, |resolution| {
                    initial_null_state(&resolution.ty)
                }),
        }
    }

    fn refine_null_condition(expression: &Expression, truth: bool, context: &mut FunctionContext) {
        match &expression.kind {
            ExpressionKind::Parenthesized(inner) => {
                Self::refine_null_condition(inner, truth, context);
            }
            ExpressionKind::Prefix {
                operator: PrefixOperator::Not,
                operand,
            } => Self::refine_null_condition(operand, !truth, context),
            ExpressionKind::Binary {
                left,
                operator: BinaryOperator::Equal | BinaryOperator::NotEqual,
                right,
            } if is_null_literal(left) || is_null_literal(right) => {
                let pointer = if is_null_literal(left) { right } else { left };
                let equal = matches!(
                    &expression.kind,
                    ExpressionKind::Binary {
                        operator: BinaryOperator::Equal,
                        ..
                    }
                );
                let non_null = if equal { !truth } else { truth };
                set_expression_null_state(
                    pointer,
                    if non_null {
                        NullState::NonNull
                    } else {
                        NullState::Null
                    },
                    context,
                );
            }
            ExpressionKind::Binary {
                left,
                operator: BinaryOperator::LogicalAnd,
                right,
            } if truth => {
                Self::refine_null_condition(left, true, context);
                Self::refine_null_condition(right, true, context);
            }
            ExpressionKind::Binary {
                left,
                operator: BinaryOperator::LogicalOr,
                right,
            } if !truth => {
                Self::refine_null_condition(left, false, context);
                Self::refine_null_condition(right, false, context);
            }
            ExpressionKind::Name(_) => set_expression_null_state(
                expression,
                if truth {
                    NullState::NonNull
                } else {
                    NullState::Null
                },
                context,
            ),
            _ => {}
        }
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
        let (exception_id, exception) = self.native_result_exception(error_type);
        self.validate_checked_effect(exception_id, expression.span, context);
        self.model
            .rust_result_adaptations
            .push(RustResultAdaptation {
                span: expression.span,
                error_message: self.rust_error_message(error_type),
                exception,
            });
        temporary(canonical(expected))
    }

    #[allow(clippy::too_many_lines)]
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
        let (elements, permits_mutable_iteration, structured) =
            match (path.as_str(), arguments.as_slice()) {
                ("rust::Vec" | "rust::List" | "rust::Queue", [element]) => {
                    (vec![element.clone()], true, false)
                }
                ("rust::Set", [element]) => (vec![element.clone()], false, false),
                ("rust::Map" | "rust::MultiMap", [key, value]) => {
                    (vec![key.clone(), value.clone()], true, true)
                }
                _ => {
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
            };
        if range.bindings.len() != elements.len() {
            self.push(
                "RES007",
                if structured {
                    "map iteration requires `[key, value]` structured bindings".to_owned()
                } else {
                    format!("`{path}` iteration requires one element binding")
                },
                range.ty.span,
            );
            return;
        }
        if structured && !range.ty.is_inferred() {
            self.push(
                "RES007",
                "map structured bindings require `auto`, `const auto&`, or `auto&`".to_owned(),
                range.ty.span,
            );
        }

        let binding_types = if structured {
            if range.ty.is_reference {
                vec![
                    TypeRef::shared_ref(elements[0].clone()),
                    TypeRef::Reference {
                        mutable: !range.ty.is_const,
                        target: Box::new(elements[1].clone()),
                    },
                ]
            } else {
                elements.clone()
            }
        } else {
            let element = &elements[0];
            vec![if range.ty.is_inferred() {
                if range.ty.is_reference {
                    TypeRef::Reference {
                        mutable: !range.ty.is_const,
                        target: Box::new(element.clone()),
                    }
                } else {
                    element.clone()
                }
            } else {
                self.resolve_type(
                    &range.ty,
                    &context.namespace,
                    &context.type_parameters,
                    false,
                )
            }]
        };

        for ((syntax, binding_type), element) in
            range.bindings.iter().zip(&binding_types).zip(&elements)
        {
            self.reject_bare_interface_type(binding_type, syntax.span, "range binding");
            if canonical(binding_type) != *element {
                self.push(
                    "RES007",
                    format!(
                        "range binding type `{}` does not exactly match element type `{}`",
                        display_type(binding_type),
                        display_type(element)
                    ),
                    syntax.span,
                );
            }
        }

        let mutable_iteration = binding_types
            .iter()
            .any(|ty| matches!(ty, TypeRef::Reference { mutable: true, .. }));
        if mutable_iteration && iterable.category != ValueCategory::MutablePlace {
            self.push(
                "RES008",
                "mutable range binding requires a mutable range".to_owned(),
                range.iterable.span,
            );
        }
        if mutable_iteration && !permits_mutable_iteration {
            self.push(
                "RES008",
                format!(
                    "mutable range iteration is not supported for ordered collection `{path}` because changing an element could invalidate its order"
                ),
                range.ty.span,
            );
        }
        if binding_types.iter().all(|ty| !ty.is_reference())
            && iterable.category != ValueCategory::Temporary
        {
            for (syntax, element) in range.bindings.iter().zip(&elements) {
                if !self.is_copyable_type(element) {
                    self.push(
                        "RES009",
                        format!(
                            "copying range elements of type `{}` is not implicit; consume the range with `move`",
                            display_type(element)
                        ),
                        syntax.span,
                    );
                }
            }
        }

        let variables = range
            .bindings
            .iter()
            .zip(binding_types)
            .map(|(syntax, binding_type)| {
                let variable = Variable {
                    mutable: matches!(binding_type, TypeRef::Reference { mutable: true, .. })
                        || (!binding_type.is_reference() && !range.ty.is_const),
                    ty: binding_type,
                    null_state: NullState::Unknown,
                };
                self.model.bindings.push(BindingResolution {
                    span: syntax.span,
                    name: syntax.name.clone(),
                    ty: variable.ty.clone(),
                    mutable: variable.mutable,
                });
                (syntax, variable)
            })
            .collect::<Vec<_>>();
        let scope = context
            .scopes
            .last_mut()
            .expect("a function context always has a scope");
        for (syntax, variable) in variables {
            self.insert_variable(scope, &syntax.name, variable, syntax.span);
        }
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
                if matches!(expected, Some(TypeRef::Callback(_) | TypeRef::Function(_)))
                    && !self.value_name_resolves(path, context)
                {
                    (
                        self.resolve_callback_function_name(
                            path,
                            expected.expect("callable expectation was matched"),
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
            ExpressionKind::GenericName { path, .. } => {
                self.push(
                    "RES104",
                    format!(
                        "generic target `{}` is only valid as a compiler-supported call",
                        path.display()
                    ),
                    expression.span,
                );
                (error_info(), None, None)
            }
            ExpressionKind::Literal(literal) => {
                let ty = literal_type(literal.kind, &literal.text, expected);
                if literal.kind == LiteralKind::Integer
                    && let Some(magnitude) = integer_magnitude(&literal.text)
                    && !integer_literal_fits(magnitude, &ty, self.resolving_negated_integer_literal)
                {
                    let sign = if self.resolving_negated_integer_literal {
                        "negative "
                    } else {
                        ""
                    };
                    self.push(
                        "RES128",
                        format!(
                            "{sign}integer literal `{}` does not fit in `{}`",
                            literal.text,
                            display_type(&ty)
                        ),
                        expression.span,
                    );
                }
                if literal.kind == LiteralKind::Null
                    && literal.text == "nullptr"
                    && ty == TypeRef::Error
                {
                    self.push(
                        "RES112",
                        "`nullptr` requires a contextual nullable pointer type".to_owned(),
                        expression.span,
                    );
                }
                (
                    ExpressionInfo {
                        ty,
                        category: ValueCategory::Temporary,
                    },
                    None,
                    None,
                )
            }
            ExpressionKind::JsonArray { elements } => (
                self.resolve_json_values(elements.iter(), context),
                None,
                None,
            ),
            ExpressionKind::JsonObject { members } => {
                let mut keys = BTreeSet::new();
                for (key, _) in members {
                    if !keys.insert(key) {
                        self.push(
                            "RES103",
                            format!("duplicate JSON object key `{key}`"),
                            expression.span,
                        );
                    }
                }
                (
                    self.resolve_json_values(members.iter().map(|(_, value)| value), context),
                    None,
                    None,
                )
            }
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
                if call.as_ref().is_some_and(|call| self.call_is_async(call))
                    && context.awaiting_call != Some(expression.span)
                {
                    self.push(
                        "RES123",
                        "an async call must be followed by `.await`".to_owned(),
                        expression.span,
                    );
                }
                (info, call, None)
            }
            ExpressionKind::MacroCall { callee, arguments } => (
                self.resolve_macro_call(callee, arguments, expression.span, context),
                None,
                None,
            ),
            ExpressionKind::Aggregate { ty, initializers } => {
                let (info, call) =
                    self.resolve_braced_expression(ty, initializers, expression.span, context);
                (info, call, None)
            }
            ExpressionKind::Field { receiver, name } => {
                let (info, field) =
                    self.resolve_struct_field(receiver, name, expression.span, context);
                (info, None, field)
            }
            ExpressionKind::Index { receiver, index } => {
                let receiver = self.resolve_expression(receiver, None, context);
                let index = self.resolve_expression(index, Some(&TypeRef::Usize), context);
                self.require_exact(&TypeRef::Usize, &index.ty, expression.span, "JSON index");
                if is_json_var(&canonical(&receiver.ty)) {
                    (
                        ExpressionInfo {
                            ty: json_var_type(),
                            category: receiver.category,
                        },
                        None,
                        None,
                    )
                } else {
                    if canonical(&receiver.ty) != TypeRef::Error {
                        self.push(
                            "RES011",
                            format!(
                                "indexing requires `var`, found `{}`",
                                display_type(&receiver.ty)
                            ),
                            expression.span,
                        );
                    }
                    (error_info(), None, None)
                }
            }
            ExpressionKind::Lambda {
                captures,
                parameters,
                is_mutable,
                is_async,
                body,
            } => (
                self.resolve_lambda(
                    captures,
                    parameters,
                    *is_mutable,
                    *is_async,
                    body,
                    expected,
                    expression.span,
                    context,
                ),
                None,
                None,
            ),
            ExpressionKind::Await(operand) => {
                if !context.is_async {
                    self.push(
                        "RES123",
                        "`.await` is only allowed inside an async function or async lambda"
                            .to_owned(),
                        expression.span,
                    );
                }
                let direct_span = awaited_call_span(operand);
                let previous = std::mem::replace(&mut context.awaiting_call, direct_span);
                let info = self.resolve_expression(operand, expected, context);
                context.awaiting_call = previous;
                let is_async_call = direct_span.is_some_and(|span| {
                    self.model
                        .calls
                        .iter()
                        .rev()
                        .find(|call| call.span == span)
                        .is_some_and(|call| self.call_is_async(call))
                });
                if !is_async_call {
                    self.push(
                        "RES123",
                        "`.await` requires a direct async Stainless or Rust call".to_owned(),
                        expression.span,
                    );
                }
                (info, None, None)
            }
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

    fn resolve_json_values<'a>(
        &mut self,
        values: impl IntoIterator<Item = &'a Expression>,
        context: &mut FunctionContext,
    ) -> ExpressionInfo {
        for value in values {
            let actual = self.resolve_expression(value, None, context);
            let ty = canonical(&actual.ty);
            if !self.is_json_compatible(&ty) && ty != TypeRef::Error {
                self.push(
                    "RES103",
                    format!(
                        "JSON values must be scalars, collections, or data structs with a supported structural JSON representation; found `{}`",
                        display_type(&ty)
                    ),
                    value.span,
                );
            } else if !self.is_copyable_type(&ty) && actual.category != ValueCategory::Temporary {
                self.push(
                    "RES027",
                    format!(
                        "JSON value of non-copy type `{}` requires `move(...)`",
                        display_type(&ty)
                    ),
                    value.span,
                );
            } else {
                self.record_json_conversions(&ty);
            }
        }
        temporary(json_var_type())
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
                && let Some(field) = self.lookup_struct_field(
                    receiver.structure,
                    &self.model.structs[receiver.structure.0]
                        .type_parameters
                        .iter()
                        .cloned()
                        .map(TypeRef::Parameter)
                        .collect::<Vec<_>>(),
                    path,
                )
            {
                if !Self::member_is_accessible(field.owner, field.is_public, context) {
                    self.push(
                        "RES121",
                        format!("field `{}` is private", path.display()),
                        span,
                    );
                }
                return (
                    ExpressionInfo {
                        ty: field.ty,
                        category: if receiver.mutable {
                            ValueCategory::MutablePlace
                        } else {
                            ValueCategory::SharedPlace
                        },
                    },
                    Some(ResolvedField {
                        access_path: field.access_path,
                    }),
                );
            }
        }
        if let Some((structure, constant)) = self.lookup_static_constant(path, context) {
            let symbol = &self.model.structs[structure.0].static_constants[constant];
            let is_public = symbol.is_public;
            let ty = symbol.ty.clone();
            if !Self::member_is_accessible(structure, is_public, context) {
                self.push(
                    "RES121",
                    format!("static constant `{}` is private", path.display()),
                    span,
                );
            }
            self.model
                .static_constant_references
                .push(ResolvedStaticConstant {
                    span,
                    structure,
                    constant,
                });
            return (temporary(ty), None);
        }
        self.push(
            "RES012",
            format!("unresolved value name `{}`", path.display()),
            span,
        );
        (error_info(), None)
    }

    fn value_name_resolves(&self, path: &ast::Path, context: &FunctionContext) -> bool {
        if self.lookup_static_constant(path, context).is_some() {
            return true;
        }
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
                self.lookup_struct_field(
                    receiver.structure,
                    &self.model.structs[receiver.structure.0]
                        .type_parameters
                        .iter()
                        .cloned()
                        .map(TypeRef::Parameter)
                        .collect::<Vec<_>>(),
                    path,
                )
                .is_some()
            })
    }

    fn lookup_static_constant(
        &self,
        path: &ast::Path,
        context: &FunctionContext,
    ) -> Option<(StructId, usize)> {
        let (structure, name) = if let Some((name, owner)) = path.segments.split_last()
            && !owner.is_empty()
        {
            (self.lookup_struct_path(owner, &context.namespace)?, name)
        } else {
            (context.receiver.as_ref()?.structure, path.segments.first()?)
        };
        self.model.structs[structure.0]
            .static_constants
            .iter()
            .position(|constant| constant.name == *name)
            .map(|constant| (structure, constant))
    }

    fn resolve_callback_function_name(
        &mut self,
        path: &ast::Path,
        expected: &TypeRef,
        span: Span,
        context: &FunctionContext,
    ) -> ExpressionInfo {
        let (parameters, return_type) = callable_signature(expected)
            .expect("named function conversion is only requested for callable expectations");
        let expected_async = matches!(expected, TypeRef::Callback(callback) if callback.is_async);
        let signature_candidates = self
            .function_candidates(path, &context.namespace)
            .into_iter()
            .filter(|id| {
                let function = &self.model.functions[id.0];
                function.receiver.is_none()
                    && function.is_async == expected_async
                    && function
                        .parameters
                        .iter()
                        .map(|parameter| &parameter.ty)
                        .eq(parameters.iter())
                    && &function.return_type == return_type
            })
            .collect::<Vec<_>>();
        if signature_candidates.len() != 1 {
            self.push(
                "RES084",
                format!(
                    "callback function `{}` must resolve to exactly one non-member overload with signature ({}) -> {}",
                    path.display(),
                    parameters
                        .iter()
                        .map(display_type)
                        .collect::<Vec<_>>()
                        .join(", "),
                    display_type(return_type)
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
                "a stored or ordinary Rust callback cannot throw".to_owned(),
                span,
            );
            return error_info();
        }
        let ty = expected.clone();
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
        is_mutable: bool,
        is_async: bool,
        body: &ast::Block,
        expected: Option<&TypeRef>,
        span: Span,
        outer: &mut FunctionContext,
    ) -> ExpressionInfo {
        let Some(expected) = expected else {
            self.push(
                "RES082",
                "a lambda requires a contextual callback or stored function type".to_owned(),
                span,
            );
            return error_info();
        };
        let (
            callback_parameters,
            callback_return,
            stored_kind,
            callback_escape,
            callback_kind,
            expected_async,
        ) = match expected {
            TypeRef::Callback(callback) => {
                if callback.escape == crate::interop::CallbackEscape::Static {
                    self.push(
                        "RES082",
                        "general `'static` callback retention is not implemented".to_owned(),
                        span,
                    );
                    return error_info();
                }
                if callback.escape == crate::interop::CallbackEscape::Thread
                    && callback.kind != CallbackKind::FnOnce
                    && !callback.is_async
                {
                    self.push(
                        "RES115",
                        "a spawned thread requires an owned `FnOnce` callback".to_owned(),
                        span,
                    );
                }
                if callback.kind == CallbackKind::FunctionPointer && !captures.is_empty() {
                    self.push(
                        "RES086",
                        "`fn_ptr` callbacks require a captureless lambda or named function"
                            .to_owned(),
                        span,
                    );
                }
                (
                    &callback.parameters,
                    callback.return_type.as_ref(),
                    None,
                    Some(callback.escape),
                    Some(callback.kind),
                    callback.is_async,
                )
            }
            TypeRef::Function(function) => {
                if function.kind == StoredFunctionKind::Shared && is_mutable {
                    self.push(
                        "RES093",
                        "a `mutable` lambda requires `function_mut`, not shared `function`"
                            .to_owned(),
                        span,
                    );
                }
                (
                    &function.parameters,
                    function.return_type.as_ref(),
                    Some(function.kind),
                    None,
                    None,
                    false,
                )
            }
            _ => {
                self.push(
                    "RES082",
                    "a lambda requires a contextual callback or stored function type".to_owned(),
                    span,
                );
                return error_info();
            }
        };
        if is_async != expected_async {
            self.push(
                "RES123",
                if expected_async {
                    "this Rust callback parameter requires an `async` lambda".to_owned()
                } else {
                    "an async lambda requires an async Rust callback parameter".to_owned()
                },
                span,
            );
        }
        if is_async && is_mutable {
            self.push(
                "RES123",
                "async `mutable` callbacks are not supported; use an async `fn` or `fn_once` callback"
                    .to_owned(),
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
            let resolved = match &capture.kind {
                LambdaCaptureKind::Copy => {
                    let Some(outer_variable) = outer
                        .scopes
                        .iter()
                        .rev()
                        .find_map(|scope| scope.get(&capture.name))
                        .cloned()
                    else {
                        self.push(
                            "RES087",
                            format!("lambda capture `{}` is not a local binding", capture.name),
                            capture.span,
                        );
                        continue;
                    };
                    let scoped_shared_reference = callback_escape
                        == Some(crate::interop::CallbackEscape::Scoped)
                        && matches!(
                            &outer_variable.ty,
                            TypeRef::Reference { mutable: false, .. }
                        );
                    if outer_variable.ty.is_reference() && !scoped_shared_reference {
                        self.push(
                            "RES088",
                            "only a scoped thread may copy-capture a shared reference binding"
                                .to_owned(),
                            capture.span,
                        );
                        continue;
                    }
                    if !scoped_shared_reference
                        && !self.is_copyable_type(&canonical(&outer_variable.ty))
                    {
                        self.push(
                            "RES089",
                            format!(
                                "copy capture `{}` requires a Stainless-copyable value; use `[{} = move({})]`",
                                capture.name, capture.name, capture.name
                            ),
                            capture.span,
                        );
                    }
                    Some((
                        outer_variable.ty.clone(),
                        LambdaCaptureMode::Copy,
                        Variable {
                            ty: outer_variable.ty.clone(),
                            mutable: is_mutable,
                            null_state: outer_variable.null_state,
                        },
                    ))
                }
                LambdaCaptureKind::Borrow => {
                    if is_async && callback_kind == Some(CallbackKind::Fn) {
                        self.push(
                            "RES123",
                            "a repeatable async callback must own its captures".to_owned(),
                            capture.span,
                        );
                        continue;
                    }
                    if callback_escape == Some(crate::interop::CallbackEscape::Thread) {
                        self.push(
                            "RES115",
                            "an unscoped thread cannot borrow a capture; copy it or transfer ownership with an initializer capture"
                                .to_owned(),
                            capture.span,
                        );
                    } else if stored_kind.is_some() {
                        self.push(
                            "RES094",
                            "a stored function must own every capture; reference captures are not allowed"
                                .to_owned(),
                            capture.span,
                        );
                        continue;
                    }
                    let Some(outer_variable) = outer
                        .scopes
                        .iter()
                        .rev()
                        .find_map(|scope| scope.get(&capture.name))
                        .cloned()
                    else {
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
                    let mutable = outer_variable.mutable;
                    Some((
                        outer_variable.ty.clone(),
                        LambdaCaptureMode::Borrow { mutable },
                        Variable {
                            ty: if mutable {
                                TypeRef::mutable_ref(canonical(&outer_variable.ty))
                            } else {
                                TypeRef::shared_ref(canonical(&outer_variable.ty))
                            },
                            mutable,
                            null_state: outer_variable.null_state,
                        },
                    ))
                }
                LambdaCaptureKind::Initialize(initializer) => {
                    let actual = self.resolve_expression(initializer, None, outer);
                    let captured_ty = canonical(&actual.ty);
                    if actual.ty.is_reference() || actual.ty.contains_reference() {
                        self.push(
                            "RES090",
                            "a lambda initializer capture must produce an owned value".to_owned(),
                            capture.span,
                        );
                    }
                    if captured_ty == TypeRef::Void {
                        self.push(
                            "RES090",
                            "a lambda initializer capture cannot have type `void`".to_owned(),
                            capture.span,
                        );
                    }
                    if is_async
                        && callback_kind == Some(CallbackKind::Fn)
                        && !self.is_copyable_type(&captured_ty)
                    {
                        self.push(
                            "RES123",
                            format!(
                                "repeatable async capture `{}` must be copyable so each invocation can own its future state",
                                capture.name
                            ),
                            capture.span,
                        );
                    }
                    self.validate_value_use(
                        &captured_ty,
                        &actual,
                        initializer.span,
                        "lambda capture initializer",
                    );
                    Some((
                        captured_ty.clone(),
                        LambdaCaptureMode::Initialize,
                        Variable {
                            ty: captured_ty,
                            mutable: is_mutable,
                            null_state: self.expression_null_state(initializer, outer),
                        },
                    ))
                }
            };
            let Some((captured_ty, mode, inner_variable)) = resolved else {
                continue;
            };
            if callback_escape == Some(crate::interop::CallbackEscape::Thread)
                && !matches!(mode, LambdaCaptureMode::Borrow { .. })
                && !self.thread_sendable(&captured_ty)
            {
                self.push(
                    "RES115",
                    format!(
                        "thread capture `{}` has non-`Send` type `{}`",
                        capture.name,
                        display_type(&captured_ty)
                    ),
                    capture.span,
                );
            }
            if is_async
                && callback_kind == Some(CallbackKind::Fn)
                && callback_escape == Some(crate::interop::CallbackEscape::Thread)
                && !self.thread_sync(&captured_ty)
            {
                self.push(
                    "RES115",
                    format!(
                        "repeatable threaded async capture `{}` has non-`Sync` type `{}`",
                        capture.name,
                        display_type(&captured_ty)
                    ),
                    capture.span,
                );
            }
            if callback_escape == Some(crate::interop::CallbackEscape::Scoped) {
                let sendable = match mode {
                    LambdaCaptureMode::Borrow { mutable: false } => self.thread_sync(&captured_ty),
                    LambdaCaptureMode::Copy if captured_ty.is_reference() => {
                        self.thread_sync(canonical_ref(&captured_ty))
                    }
                    LambdaCaptureMode::Borrow { mutable: true }
                    | LambdaCaptureMode::Copy
                    | LambdaCaptureMode::Initialize => self.thread_sendable(&captured_ty),
                };
                if !sendable {
                    self.push(
                        "RES116",
                        format!(
                            "scoped thread capture `{}` has a non-`Send` representation `{}`",
                            capture.name,
                            display_type(&captured_ty)
                        ),
                        capture.span,
                    );
                }
            }
            lambda_scope.insert(capture.name.clone(), inner_variable);
            resolved_captures.push(ResolvedLambdaCapture {
                name: capture.name.clone(),
                ty: captured_ty,
                mode,
            });
        }

        if parameters.len() != callback_parameters.len() {
            self.push(
                "RES091",
                format!(
                    "callback requires {} lambda parameter(s), found {}",
                    callback_parameters.len(),
                    parameters.len()
                ),
                span,
            );
        }
        for (index, parameter) in parameters.iter().enumerate() {
            let resolved = self.resolve_type(
                &parameter.ty,
                &outer.namespace,
                &outer.type_parameters,
                false,
            );
            self.reject_bare_interface_type(&resolved, parameter.ty.span, "lambda parameter");
            if let Some(expected) = callback_parameters.get(index)
                && *expected != resolved
                && *expected != TypeRef::Error
                && resolved != TypeRef::Error
            {
                self.push(
                    "RES028",
                    format!(
                        "lambda parameter requires exact type `{}`, found `{}`",
                        display_type(expected),
                        display_type(&resolved)
                    ),
                    parameter.ty.span,
                );
            }
            let variable = Variable {
                mutable: parameter_mutability(parameter, &resolved),
                null_state: initial_null_state(&resolved),
                ty: resolved,
            };
            self.insert_variable(&mut lambda_scope, &parameter.name, variable, parameter.span);
        }

        let mut context = FunctionContext {
            namespace: outer.namespace.clone(),
            type_parameters: outer.type_parameters.clone(),
            return_type: callback_return.clone(),
            scopes: vec![lambda_scope],
            receiver: None,
            declared_throws: Vec::new(),
            handled_throws: Vec::new(),
            current_catch: None,
            is_lambda: true,
            is_async,
            awaiting_call: None,
        };
        self.resolve_block(body, &mut context, false);

        let ty = expected.clone();
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
        id: StructId,
        structure_type: TypeRef,
        initializers: &[Expression],
        span: Span,
        context: &mut FunctionContext,
    ) -> (ExpressionInfo, Option<ResolvedCall>) {
        let structure = self.model.structs[id.0].clone();
        if structure.kind != ast::UserTypeKind::Struct {
            for initializer in initializers {
                self.resolve_expression(initializer, None, context);
            }
            self.push(
                "RES118",
                format!(
                    "field-wise aggregate initialization requires a struct, found `{}`",
                    display_path(&structure.path)
                ),
                span,
            );
            return (error_info(), None);
        }
        let substitutions = user_type_substitutions(&structure, &structure_type);
        let mut expected = Vec::new();
        if let Some(base) = structure.base {
            expected.push(TypeRef::Struct {
                path: self.model.structs[base.0].path.clone(),
                arguments: Vec::new(),
            });
        }
        expected.extend(
            structure
                .fields
                .iter()
                .map(|field| substitute_type(&field.ty, &substitutions)),
        );
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
        let return_type = structure_type;
        let call = ResolvedCall {
            span,
            target: CallTarget::Intrinsic(Intrinsic::StructAggregate { structure: id }),
            return_type: return_type.clone(),
            throws: Vec::new(),
        };
        (temporary(return_type), Some(call))
    }

    fn resolve_braced_expression(
        &mut self,
        ty: &ast::Type,
        initializers: &[Expression],
        span: Span,
        context: &mut FunctionContext,
    ) -> (ExpressionInfo, Option<ResolvedCall>) {
        let TypeKind::Named(named) = &ty.kind else {
            for initializer in initializers {
                self.resolve_expression(initializer, None, context);
            }
            self.push(
                "RES046",
                "brace construction requires a named target".to_owned(),
                span,
            );
            return (error_info(), None);
        };
        if ty.is_const || ty.is_reference {
            for initializer in initializers {
                self.resolve_expression(initializer, None, context);
            }
            self.push(
                "RES046",
                "brace construction target cannot be const or a reference".to_owned(),
                span,
            );
            return (error_info(), None);
        }
        if matches!(named.path.segments.as_slice(), [name] if name == "make_unique") {
            return self.resolve_make_owner(
                PointerKind::Unique,
                &named.arguments,
                initializers,
                span,
                ConstructionSyntax::Braced,
                context,
            );
        }
        if matches!(named.path.segments.as_slice(), [name] if name == "make_shared") {
            return self.resolve_make_owner(
                PointerKind::Shared,
                &named.arguments,
                initializers,
                span,
                ConstructionSyntax::Braced,
                context,
            );
        }
        let resolved = self.resolve_type(ty, &context.namespace, &context.type_parameters, false);
        let (path, kind) = match &resolved {
            TypeRef::Struct { path, .. } => (path, ast::UserTypeKind::Struct),
            TypeRef::Class { path, .. } => (path, ast::UserTypeKind::Class),
            TypeRef::Interface { path, .. } => (path, ast::UserTypeKind::Interface),
            _ => {
                for initializer in initializers {
                    self.resolve_expression(initializer, None, context);
                }
                return (error_info(), None);
            }
        };
        let Some(id) = self.struct_by_path.get(path).copied() else {
            return (error_info(), None);
        };
        match kind {
            ast::UserTypeKind::Struct => {
                self.resolve_struct_aggregate(id, resolved, initializers, span, context)
            }
            ast::UserTypeKind::Class => {
                self.resolve_user_constructor(id, resolved, initializers, span, context)
            }
            ast::UserTypeKind::Interface => {
                for initializer in initializers {
                    self.resolve_expression(initializer, None, context);
                }
                self.push(
                    "RES118",
                    format!("interface `{}` cannot be constructed", named.path.display()),
                    span,
                );
                (error_info(), None)
            }
        }
    }

    fn resolve_struct_field(
        &mut self,
        receiver: &Expression,
        name: &ast::Path,
        span: Span,
        context: &mut FunctionContext,
    ) -> (ExpressionInfo, Option<ResolvedField>) {
        let receiver_info = self.resolve_expression(receiver, None, context);
        if is_json_var(&canonical(&receiver_info.ty)) {
            if name.segments.len() != 1 {
                self.push(
                    "RES010",
                    "a JSON member name cannot use explicit-base qualification".to_owned(),
                    span,
                );
                return (error_info(), None);
            }
            return (
                ExpressionInfo {
                    ty: json_var_type(),
                    category: receiver_info.category,
                },
                None,
            );
        }
        let pointee = self.refined_automatic_pointee(receiver, &receiver_info, context, span);
        if let TypeRef::Tuple(elements) = &pointee {
            return self.resolve_tuple_field(&receiver_info, &pointee, elements, name, span);
        }
        let (TypeRef::Struct { path, arguments } | TypeRef::Class { path, arguments }) = &pointee
        else {
            if pointee != TypeRef::Error {
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
        let Some(structure) = self.struct_by_path.get(path).copied() else {
            return (error_info(), None);
        };
        let Some(field) = self.lookup_struct_field(structure, arguments, name) else {
            self.push(
                "RES010",
                format!(
                    "struct `{}` has no data field `{}`",
                    display_path(path),
                    name.display()
                ),
                span,
            );
            return (error_info(), None);
        };
        if !Self::member_is_accessible(field.owner, field.is_public, context) {
            self.push(
                "RES121",
                format!("field `{}` is private", name.display()),
                span,
            );
        }
        (
            ExpressionInfo {
                ty: field.ty,
                category: pointee_category(&receiver_info.ty, receiver_info.category),
            },
            Some(ResolvedField {
                access_path: field.access_path,
            }),
        )
    }

    fn resolve_tuple_field(
        &mut self,
        receiver: &ExpressionInfo,
        tuple: &TypeRef,
        elements: &[TypeRef],
        name: &ast::Path,
        span: Span,
    ) -> (ExpressionInfo, Option<ResolvedField>) {
        let Some(index_text) = name.segments.first().filter(|_| name.segments.len() == 1) else {
            self.push(
                "RES010",
                "tuple projection requires one numeric field such as `.0` or `.1`".to_owned(),
                span,
            );
            return (error_info(), None);
        };
        let Ok(index) = index_text.parse::<usize>() else {
            self.push(
                "RES010",
                format!(
                    "tuple projection `{}` is not an unsuffixed decimal index",
                    name.display()
                ),
                span,
            );
            return (error_info(), None);
        };
        let Some(element) = elements.get(index) else {
            self.push(
                "RES010",
                format!(
                    "tuple type `{}` has no element at index {index}",
                    display_type(tuple)
                ),
                span,
            );
            return (error_info(), None);
        };
        (
            ExpressionInfo {
                ty: element.clone(),
                category: pointee_category(&receiver.ty, receiver.category),
            },
            Some(ResolvedField {
                access_path: vec![index.to_string()],
            }),
        )
    }

    fn resolve_prefix(
        &mut self,
        operator: PrefixOperator,
        operand: &Expression,
        expected: Option<&TypeRef>,
        context: &mut FunctionContext,
    ) -> ExpressionInfo {
        let negative_default = (operator == PrefixOperator::Negate && expected.is_none())
            .then(|| unsuffixed_integer_literal_text(operand).map(default_negative_integer_type))
            .flatten();
        let resolving_negated_integer_literal = self.resolving_negated_integer_literal;
        self.resolving_negated_integer_literal =
            operator == PrefixOperator::Negate && integer_literal_text(operand).is_some();
        let actual = if let Some(negative_default) = negative_default.as_ref() {
            self.resolve_expression(operand, Some(negative_default), context)
        } else {
            let operand_expected = match operator {
                PrefixOperator::Not => None,
                _ => expected,
            };
            self.resolve_expression(operand, operand_expected, context)
        };
        self.resolving_negated_integer_literal = resolving_negated_integer_literal;
        match operator {
            PrefixOperator::Not => {
                if canonical(&actual.ty) != TypeRef::Bool && !is_nullable_pointer_test(&actual.ty) {
                    self.push(
                        "RES110",
                        format!(
                            "`!` requires `bool`, a nullable owner, or `weak_ptr`, found `{}`",
                            display_type(&actual.ty)
                        ),
                        operand.span,
                    );
                }
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

    #[allow(clippy::too_many_lines)]
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
                if matches!(left_type, TypeRef::Class { .. }) {
                    self.push(
                        "RES119",
                        "class values cannot be assigned; replace an ownership pointer instead"
                            .to_owned(),
                        left.span,
                    );
                }
                right_info = self.adapt_rust_result(&left_type, right_info, right, context);
                self.validate_binding(&left_type, &right_info, right.span, "assignment");
                if is_json_mutation_place(left, &self.model) {
                    let json_error = self.json_error_struct();
                    self.validate_checked_effect(json_error, left.span, context);
                }
                let null_state = self.expression_null_state(right, context);
                set_expression_null_state(left, null_state, context);
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
            let baseline = context.scopes.clone();
            Self::refine_null_condition(left, operator == BinaryOperator::LogicalAnd, context);
            let right_info = self.resolve_expression(right, Some(&TypeRef::Bool), context);
            let with_right = context.scopes.clone();
            context.scopes = merge_null_scopes(&baseline, true, &with_right, true);
            for (syntax, actual) in [(left, &left_info), (right, &right_info)] {
                if canonical(&actual.ty) != TypeRef::Bool && !is_nullable_pointer_test(&actual.ty) {
                    self.push(
                        "RES110",
                        format!(
                            "logical operand requires `bool` or a nullable pointer, found `{}`",
                            display_type(&actual.ty)
                        ),
                        syntax.span,
                    );
                }
            }
            return temporary(TypeRef::Bool);
        }

        let infer_left_from_right = is_null_literal(left)
            || is_unsuffixed_integer_literal(left) && !is_unsuffixed_integer_literal(right);
        let (left_info, right_info) = if infer_left_from_right {
            let right_info = self.resolve_expression(right, expected, context);
            let left_info =
                self.resolve_expression(left, Some(&canonical(&right_info.ty)), context);
            (left_info, right_info)
        } else {
            let left_info = self.resolve_expression(left, expected, context);
            let right_info =
                self.resolve_expression(right, Some(&canonical(&left_info.ty)), context);
            (left_info, right_info)
        };
        let left_type = canonical(&left_info.ty);
        self.require_exact(&left_type, &right_info.ty, right.span, "binary operand");
        if is_json_var(&left_type)
            && matches!(operator, BinaryOperator::Equal | BinaryOperator::NotEqual)
        {
            return temporary(TypeRef::Bool);
        }
        if is_nullable_pointer_test(&left_type)
            && matches!(operator, BinaryOperator::Equal | BinaryOperator::NotEqual)
            && (is_null_literal(left) || is_null_literal(right))
        {
            return temporary(TypeRef::Bool);
        }
        if matches!(operator, BinaryOperator::Equal | BinaryOperator::NotEqual)
            && supports_equality(&left_type)
        {
            return temporary(TypeRef::Bool);
        }
        if matches!(
            operator,
            BinaryOperator::Less
                | BinaryOperator::LessEqual
                | BinaryOperator::Greater
                | BinaryOperator::GreaterEqual
        ) && supports_ordering(&left_type)
        {
            return temporary(TypeRef::Bool);
        }
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

    #[allow(clippy::too_many_lines)]
    fn resolve_call(
        &mut self,
        callee: &Expression,
        arguments: &[Expression],
        expected: Option<&TypeRef>,
        span: Span,
        context: &mut FunctionContext,
    ) -> (ExpressionInfo, Option<ResolvedCall>) {
        match &callee.kind {
            ExpressionKind::GenericName {
                path,
                arguments: type_arguments,
            } if matches!(path.segments.as_slice(), [name] if name == "tuple") => {
                self.resolve_tuple_constructor(type_arguments, arguments, span, context)
            }
            ExpressionKind::GenericName {
                path,
                arguments: type_arguments,
            } if matches!(path.segments.as_slice(), [name] if name == "make_unique") => self
                .resolve_make_owner(
                    PointerKind::Unique,
                    type_arguments,
                    arguments,
                    span,
                    ConstructionSyntax::Parenthesized,
                    context,
                ),
            ExpressionKind::GenericName {
                path,
                arguments: type_arguments,
            } if matches!(path.segments.as_slice(), [name] if name == "make_shared") => self
                .resolve_make_owner(
                    PointerKind::Shared,
                    type_arguments,
                    arguments,
                    span,
                    ConstructionSyntax::Parenthesized,
                    context,
                ),
            ExpressionKind::GenericName {
                path,
                arguments: type_arguments,
            } if path.segments.len() == 1 && pointer_kind(&path.segments).is_some() => self
                .resolve_pointer_constructor(
                    pointer_kind(&path.segments).expect("pointer kind was checked"),
                    type_arguments,
                    arguments,
                    span,
                    context,
                ),
            ExpressionKind::GenericName {
                path,
                arguments: type_arguments,
            } if matches!(path.segments.as_slice(), [name] if name == "mutex") => {
                self.resolve_mutex_constructor(type_arguments, arguments, span, context)
            }
            ExpressionKind::GenericName {
                path,
                arguments: type_arguments,
            } if matches!(path.segments.as_slice(), [name] if name == "rwlock") => {
                self.resolve_rwlock_constructor(type_arguments, arguments, span, context)
            }
            ExpressionKind::GenericName {
                path,
                arguments: type_arguments,
            } if self
                .lookup_struct_path(&path.segments, &context.namespace)
                .is_some() =>
            {
                let Some(structure) = self.lookup_struct_path(&path.segments, &context.namespace)
                else {
                    unreachable!("generic user type lookup was checked")
                };
                let resolved_arguments = type_arguments
                    .iter()
                    .map(|argument| {
                        self.resolve_type(
                            argument,
                            &context.namespace,
                            &context.type_parameters,
                            false,
                        )
                    })
                    .collect::<Vec<_>>();
                let symbol = self.model.structs[structure.0].clone();
                if resolved_arguments.len() != symbol.type_parameters.len() {
                    for argument in arguments {
                        self.resolve_expression(argument, None, context);
                    }
                    self.push(
                        "RES050",
                        format!(
                            "type `{}` expects {} type argument(s), found {}",
                            path.display(),
                            symbol.type_parameters.len(),
                            resolved_arguments.len()
                        ),
                        span,
                    );
                    (error_info(), None)
                } else if resolved_arguments.iter().any(|argument| {
                    matches!(argument, TypeRef::Void | TypeRef::Reference { .. })
                        || argument.contains_reference()
                }) {
                    for argument in arguments {
                        self.resolve_expression(argument, None, context);
                    }
                    self.push(
                        "RES124",
                        format!(
                            "type `{}` requires storable value type arguments",
                            path.display()
                        ),
                        span,
                    );
                    (error_info(), None)
                } else {
                    let structure_type = user_type(&symbol, resolved_arguments);
                    self.resolve_user_constructor(
                        structure,
                        structure_type,
                        arguments,
                        span,
                        context,
                    )
                }
            }
            ExpressionKind::GenericName { path, .. } => {
                for argument in arguments {
                    self.resolve_expression(argument, None, context);
                }
                self.push(
                    "RES104",
                    format!("unsupported generic call `{}`", path.display()),
                    span,
                );
                (error_info(), None)
            }
            ExpressionKind::Name(path)
                if self.is_reserved_rust_path(
                    &path.segments,
                    &context.namespace,
                    &["rust", "std", "thread", "spawn"],
                ) =>
            {
                self.resolve_thread_spawn(arguments, expected, span, context)
            }
            ExpressionKind::Name(path)
                if self.is_reserved_rust_path(
                    &path.segments,
                    &context.namespace,
                    &["rust", "std", "thread", "scope"],
                ) =>
            {
                self.resolve_thread_scope(arguments, span, context)
            }
            ExpressionKind::Name(path)
                if path.segments.len() == 1 && path.segments[0] == "move" =>
            {
                self.resolve_move(arguments, span, context)
            }
            ExpressionKind::Name(path) if matches!(path.segments.as_slice(), [name] if name == "var") => {
                self.resolve_json_wrap(arguments, span, context)
            }
            ExpressionKind::Name(path) if matches!(path.segments.as_slice(), [name] if name == "condition") =>
            {
                let call =
                    self.resolve_slot_construction(&TypeRef::Condition, arguments, span, context);
                (temporary(TypeRef::Condition), call)
            }
            ExpressionKind::Name(path) if matches!(path.segments.as_slice(), [name] if name == "make_unique") =>
            {
                for argument in arguments {
                    self.resolve_expression(argument, None, context);
                }
                self.push(
                    "RES105",
                    "`make_unique` requires one explicit pointee type argument".to_owned(),
                    span,
                );
                (error_info(), None)
            }
            ExpressionKind::Name(path) if matches!(path.segments.as_slice(), [name] if name == "make_shared") =>
            {
                for argument in arguments {
                    self.resolve_expression(argument, None, context);
                }
                self.push(
                    "RES105",
                    "`make_shared` requires one explicit pointee type argument".to_owned(),
                    span,
                );
                (error_info(), None)
            }
            ExpressionKind::Field { receiver, name } => {
                let receiver_info = self.resolve_expression(receiver, None, context);
                let pointee =
                    self.refined_automatic_pointee(receiver, &receiver_info, context, span);
                if let TypeRef::Struct {
                    path,
                    arguments: type_arguments,
                }
                | TypeRef::Class {
                    path,
                    arguments: type_arguments,
                } = pointee
                    && let Some(structure) = self.struct_by_path.get(&path).copied()
                    && let Some(
                        field @ StructFieldLookup {
                            ty: TypeRef::Function(_),
                            ..
                        },
                    ) = self.lookup_struct_field(structure, &type_arguments, name)
                {
                    if !Self::member_is_accessible(field.owner, field.is_public, context) {
                        self.push(
                            "RES121",
                            format!("field `{}` is private", name.display()),
                            span,
                        );
                    }
                    let callee_info = ExpressionInfo {
                        ty: field.ty,
                        category: pointee_category(&receiver_info.ty, receiver_info.category),
                    };
                    self.record_expression(
                        callee.span,
                        callee_info.clone(),
                        None,
                        Some(ResolvedField {
                            access_path: field.access_path,
                        }),
                    );
                    return self.resolve_stored_function_call(
                        &callee_info,
                        arguments,
                        span,
                        context,
                    );
                }
                self.resolve_method_call(receiver, name, arguments, span, context, &receiver_info)
            }
            ExpressionKind::Name(path) => {
                if self.value_name_resolves(path, context) {
                    let callee_info = self.resolve_expression(callee, None, context);
                    return self.resolve_stored_function_call(
                        &callee_info,
                        arguments,
                        span,
                        context,
                    );
                }
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
                    if !self.model.structs[structure.0].type_parameters.is_empty() {
                        for argument in arguments {
                            self.resolve_expression(argument, None, context);
                        }
                        self.push(
                            "RES050",
                            format!(
                                "generic type `{}` requires explicit type arguments",
                                path.display()
                            ),
                            span,
                        );
                        return (error_info(), None);
                    }
                    let structure_type = resolved_structure_type(&self.model.structs[structure.0]);
                    return self.resolve_user_constructor(
                        structure,
                        structure_type,
                        arguments,
                        span,
                        context,
                    );
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
                let callee_info = self.resolve_expression(callee, None, context);
                if matches!(canonical_ref(&callee_info.ty), TypeRef::Function(_)) {
                    return self.resolve_stored_function_call(
                        &callee_info,
                        arguments,
                        span,
                        context,
                    );
                }
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

    #[allow(clippy::too_many_lines)]
    fn resolve_macro_call(
        &mut self,
        callee: &ast::Path,
        arguments: &[Expression],
        span: Span,
        context: &mut FunctionContext,
    ) -> ExpressionInfo {
        let macro_name = callee.segments.last().map_or("", String::as_str);
        let supported = matches!(
            macro_name,
            "println" | "eprintln" | "format" | "write" | "writeln"
        );
        let is_qualified = matches!(
            callee.segments.as_slice(),
            [root, name] if root == "rust" && name == macro_name
        );
        let is_imported = callee.segments.len() == 1
            && self
                .imports
                .candidates(&context.namespace, macro_name)
                .iter()
                .any(|candidate| {
                    matches!(candidate.as_slice(), [root, name] if root == "rust" && name == macro_name)
                });
        if !supported || (!is_qualified && !is_imported) {
            for argument in arguments {
                self.resolve_expression(argument, None, context);
            }
            self.push(
                "RES097",
                format!(
                    "unsupported macro `{}!`; supported Rust macros are `println!`, `eprintln!`, `format!`, `write!`, and `writeln!`",
                    callee.display()
                ),
                span,
            );
            return error_info();
        }

        let actual = arguments
            .iter()
            .map(|argument| self.resolve_expression(argument, None, context))
            .collect::<Vec<_>>();

        let (format_index, format_required) = match macro_name {
            "println" | "eprintln" => (0, false),
            "format" => (0, true),
            "write" => (1, true),
            "writeln" => (1, false),
            _ => unreachable!("unsupported macros returned above"),
        };
        let destination_required = matches!(macro_name, "write" | "writeln");
        if destination_required && arguments.is_empty() {
            self.push(
                "RES100",
                format!("`{macro_name}!` requires a mutable `String` destination"),
                span,
            );
        }
        if format_required && arguments.get(format_index).is_none() {
            self.push(
                "RES100",
                format!("`{macro_name}!` requires a format string literal"),
                span,
            );
        }

        if destination_required
            && let Some((destination, info)) = arguments.first().zip(actual.first())
        {
            let ty = canonical_ref(&info.ty);
            if *ty != TypeRef::Error && *ty != TypeRef::native("rust::String", Vec::new()) {
                self.push(
                    "RES101",
                    format!(
                        "`{macro_name}!` initially supports only a `String` destination, found `{}`",
                        display_type(ty)
                    ),
                    destination.span,
                );
            }
            if *ty != TypeRef::Error && info.category != ValueCategory::MutablePlace {
                self.push(
                    "RES102",
                    format!("the `{macro_name}!` destination must be mutable"),
                    destination.span,
                );
            }
        }

        if let Some(format) = arguments.get(format_index)
            && !matches!(
                &format.kind,
                ExpressionKind::Literal(ast::Literal {
                    kind: LiteralKind::String,
                    ..
                })
            )
        {
            self.push(
                "RES098",
                format!("the format argument to `{macro_name}!` must be a string literal"),
                format.span,
            );
        }
        for (argument, info) in arguments
            .iter()
            .skip(format_index + 1)
            .zip(actual.iter().skip(format_index + 1))
        {
            let ty = canonical_ref(&info.ty);
            if *ty != TypeRef::Error && !is_format_value(ty) {
                self.push(
                    "RES099",
                    format!(
                        "type `{}` is not supported as a `{macro_name}!` formatting argument",
                        display_type(ty),
                    ),
                    argument.span,
                );
            }
        }
        let ty = match macro_name {
            "println" | "eprintln" => TypeRef::Void,
            "format" => TypeRef::native("rust::String", Vec::new()),
            "write" | "writeln" => {
                let format_error = self.format_error_struct();
                self.validate_checked_effect(format_error, span, context);
                TypeRef::Void
            }
            _ => unreachable!("unsupported macros returned above"),
        };
        temporary(ty)
    }

    #[allow(clippy::too_many_lines)]
    fn resolve_method_call(
        &mut self,
        receiver: &Expression,
        name: &ast::Path,
        arguments: &[Expression],
        span: Span,
        context: &mut FunctionContext,
        receiver_info: &ExpressionInfo,
    ) -> (ExpressionInfo, Option<ResolvedCall>) {
        if let TypeRef::Pointer { kind, target } = canonical(&receiver_info.ty)
            && matches!(
                name.segments.as_slice(),
                [method]
                    if method == "__downgrade"
                        || (method == "lock" && kind == PointerKind::Weak)
            )
        {
            return self.resolve_weak_pointer_method(
                kind,
                target.as_ref(),
                name,
                arguments,
                span,
                context,
            );
        }
        match self.refined_automatic_pointee(receiver, receiver_info, context, span) {
            ref structure_type @ (TypeRef::Struct { ref path, .. }
            | TypeRef::Class { ref path, .. }
            | TypeRef::Interface { ref path, .. }) => {
                let Some(structure) = self.struct_by_path.get(path).copied() else {
                    return (error_info(), None);
                };
                let mut pointee_receiver = receiver_info.clone();
                pointee_receiver.category =
                    pointee_category(&receiver_info.ty, receiver_info.category);
                self.resolve_struct_method(
                    structure,
                    structure_type,
                    &pointee_receiver,
                    receiver.span,
                    name,
                    arguments,
                    span,
                    context,
                )
            }
            TypeRef::Pointer { kind, target }
                if matches!(kind, PointerKind::Atomic | PointerKind::AtomicNullable) =>
            {
                self.resolve_atomic_pointer_method(
                    kind,
                    target.as_ref(),
                    name,
                    arguments,
                    span,
                    context,
                )
            }
            TypeRef::Mutex(target) => {
                self.resolve_mutex_method(target.as_ref(), name, arguments, span, context)
            }
            TypeRef::RwLock(target) => {
                self.resolve_rwlock_method(target.as_ref(), name, arguments, span, context)
            }
            TypeRef::Condition => self.resolve_condition_method(name, arguments, span, context),
            TypeRef::ThreadHandle(target) => self.resolve_thread_handle_method(
                target.as_ref(),
                receiver,
                receiver_info,
                name,
                arguments,
                span,
                context,
            ),
            TypeRef::ThreadScope => {
                self.resolve_thread_scope_method(name, arguments, span, context)
            }
            TypeRef::ScopedThreadHandle(target) => self.resolve_scoped_thread_handle_method(
                target.as_ref(),
                receiver,
                receiver_info,
                name,
                arguments,
                span,
                context,
            ),
            TypeRef::Native {
                path,
                arguments: type_arguments,
            } if name.segments.len() == 1 => {
                if path == "rust::Result" && name.segments[0] == "unwrap" {
                    return self.resolve_rust_result_unwrap(
                        receiver,
                        receiver_info,
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
                    Some((receiver_info, receiver.span)),
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

    fn refined_automatic_pointee(
        &mut self,
        receiver: &Expression,
        receiver_info: &ExpressionInfo,
        context: &FunctionContext,
        span: Span,
    ) -> TypeRef {
        match canonical_ref(&receiver_info.ty) {
            TypeRef::Pointer {
                kind: PointerKind::UniqueNullable | PointerKind::SharedNullable,
                target,
            } => {
                if self.expression_null_state(receiver, context) == NullState::NonNull {
                    canonical_ref(target).clone()
                } else {
                    self.push(
                        "RES110",
                        format!(
                            "pointee access through `{}` requires a non-null guard",
                            display_type(&receiver_info.ty)
                        ),
                        span,
                    );
                    TypeRef::Error
                }
            }
            _ => automatic_pointee(&receiver_info.ty).clone(),
        }
    }

    fn resolve_mutex_method(
        &mut self,
        target: &TypeRef,
        name: &ast::Path,
        arguments: &[Expression],
        span: Span,
        context: &mut FunctionContext,
    ) -> (ExpressionInfo, Option<ResolvedCall>) {
        if name.segments.as_slice() != ["lock"] || !arguments.is_empty() {
            for argument in arguments {
                self.resolve_expression(argument, None, context);
            }
            self.push(
                "RES113",
                if name.segments.as_slice() == ["lock"] {
                    format!(
                        "`mutex<T>.lock()` requires no arguments, found {}",
                        arguments.len()
                    )
                } else {
                    format!("`mutex<T>` has no method `{}`", name.display())
                },
                span,
            );
            return (error_info(), None);
        }
        let return_type = TypeRef::MutexGuard(Box::new(target.clone()));
        let call = ResolvedCall {
            span,
            target: CallTarget::Intrinsic(Intrinsic::MutexLock {
                target: target.clone(),
            }),
            return_type: return_type.clone(),
            throws: Vec::new(),
        };
        (temporary(return_type), Some(call))
    }

    fn resolve_rwlock_method(
        &mut self,
        target: &TypeRef,
        name: &ast::Path,
        arguments: &[Expression],
        span: Span,
        context: &mut FunctionContext,
    ) -> (ExpressionInfo, Option<ResolvedCall>) {
        let operation = name.segments.as_slice();
        if !matches!(operation, [name] if name == "read" || name == "write")
            || !arguments.is_empty()
        {
            for argument in arguments {
                self.resolve_expression(argument, None, context);
            }
            self.push(
                "RES113",
                if matches!(operation, [name] if name == "read" || name == "write") {
                    format!(
                        "`rwlock<T>.{}()` requires no arguments, found {}",
                        operation[0],
                        arguments.len()
                    )
                } else {
                    format!("`rwlock<T>` has no method `{}`", name.display())
                },
                span,
            );
            return (error_info(), None);
        }
        let read = operation == ["read"];
        let return_type = if read {
            TypeRef::RwLockReadGuard(Box::new(target.clone()))
        } else {
            TypeRef::RwLockWriteGuard(Box::new(target.clone()))
        };
        let intrinsic = if read {
            Intrinsic::RwLockRead {
                target: target.clone(),
            }
        } else {
            Intrinsic::RwLockWrite {
                target: target.clone(),
            }
        };
        let call = ResolvedCall {
            span,
            target: CallTarget::Intrinsic(intrinsic),
            return_type: return_type.clone(),
            throws: Vec::new(),
        };
        (temporary(return_type), Some(call))
    }

    fn resolve_condition_method(
        &mut self,
        name: &ast::Path,
        arguments: &[Expression],
        span: Span,
        context: &mut FunctionContext,
    ) -> (ExpressionInfo, Option<ResolvedCall>) {
        let operation = name.segments.as_slice();
        if operation == ["notify_one"] || operation == ["notify_all"] {
            if !arguments.is_empty() {
                for argument in arguments {
                    self.resolve_expression(argument, None, context);
                }
                self.push(
                    "RES113",
                    format!(
                        "`condition.{}` requires no arguments, found {}",
                        name.display(),
                        arguments.len()
                    ),
                    span,
                );
                return (error_info(), None);
            }
            let call = ResolvedCall {
                span,
                target: CallTarget::Intrinsic(Intrinsic::ConditionNotify {
                    all: operation == ["notify_all"],
                }),
                return_type: TypeRef::Void,
                throws: Vec::new(),
            };
            return (temporary(TypeRef::Void), Some(call));
        }

        if operation != ["wait"] {
            for argument in arguments {
                self.resolve_expression(argument, None, context);
            }
            self.push(
                "RES113",
                format!("`condition` has no method `{}`", name.display()),
                span,
            );
            return (error_info(), None);
        }
        if arguments.len() != 1 {
            for argument in arguments {
                self.resolve_expression(argument, None, context);
            }
            self.push(
                "RES113",
                format!(
                    "`condition.wait(guard)` requires one argument, found {}",
                    arguments.len()
                ),
                span,
            );
            return (error_info(), None);
        }

        let argument = &arguments[0];
        let info = self.resolve_expression(argument, None, context);
        let TypeRef::MutexGuard(target) = canonical_ref(&info.ty) else {
            if info.ty != TypeRef::Error {
                self.push(
                    "RES113",
                    format!(
                        "`condition.wait()` requires a mutex guard, found `{}`",
                        display_type(&info.ty)
                    ),
                    argument.span,
                );
            }
            return (error_info(), None);
        };
        if info.category != ValueCategory::MutablePlace || !is_named_value_expression(argument) {
            self.push(
                "RES114",
                "`condition.wait()` requires a mutable named guard so it can rebind it after waking"
                    .to_owned(),
                argument.span,
            );
            return (error_info(), None);
        }
        let call = ResolvedCall {
            span,
            target: CallTarget::Intrinsic(Intrinsic::ConditionWait {
                target: target.as_ref().clone(),
            }),
            return_type: TypeRef::Void,
            throws: Vec::new(),
        };
        (temporary(TypeRef::Void), Some(call))
    }

    fn resolve_thread_spawn(
        &mut self,
        arguments: &[Expression],
        expected: Option<&TypeRef>,
        span: Span,
        context: &mut FunctionContext,
    ) -> (ExpressionInfo, Option<ResolvedCall>) {
        let result_type = match expected.map(canonical) {
            Some(TypeRef::ThreadHandle(target)) => target.as_ref().clone(),
            _ => TypeRef::Void,
        };
        if !self.thread_sendable(&result_type) {
            self.push(
                "RES115",
                format!(
                    "thread result type `{}` is not `Send`",
                    display_type(&result_type)
                ),
                span,
            );
        }
        let callback_type = TypeRef::callback(
            CallbackKind::FnOnce,
            crate::interop::CallbackEscape::Thread,
            Vec::new(),
            result_type.clone(),
        );
        if arguments.len() != 1 {
            for argument in arguments {
                self.resolve_expression(argument, None, context);
            }
            self.push(
                "RES115",
                format!(
                    "`thread::spawn` requires one `void()` callback, found {} arguments",
                    arguments.len()
                ),
                span,
            );
            return (error_info(), None);
        }
        let actual = self.resolve_expression(&arguments[0], Some(&callback_type), context);
        self.require_exact(
            &callback_type,
            &actual.ty,
            arguments[0].span,
            "thread callback",
        );
        let return_type = TypeRef::ThreadHandle(Box::new(result_type));
        let call = ResolvedCall {
            span,
            target: CallTarget::Intrinsic(Intrinsic::ThreadSpawn),
            return_type: return_type.clone(),
            throws: Vec::new(),
        };
        (temporary(return_type), Some(call))
    }

    fn resolve_thread_scope(
        &mut self,
        arguments: &[Expression],
        span: Span,
        context: &mut FunctionContext,
    ) -> (ExpressionInfo, Option<ResolvedCall>) {
        let callback_type = TypeRef::callback(
            CallbackKind::FnOnce,
            crate::interop::CallbackEscape::Call,
            vec![TypeRef::shared_ref(TypeRef::ThreadScope)],
            TypeRef::Void,
        );
        if arguments.len() != 1 {
            for argument in arguments {
                self.resolve_expression(argument, None, context);
            }
            self.push(
                "RES116",
                format!(
                    "`thread::scope` requires one scope callback, found {} arguments",
                    arguments.len()
                ),
                span,
            );
            return (error_info(), None);
        }
        let actual = self.resolve_expression(&arguments[0], Some(&callback_type), context);
        self.require_exact(
            &callback_type,
            &actual.ty,
            arguments[0].span,
            "thread scope callback",
        );
        let thread_error = self.thread_error_struct();
        self.validate_checked_effect(thread_error, span, context);
        let call = ResolvedCall {
            span,
            target: CallTarget::Intrinsic(Intrinsic::ThreadScope),
            return_type: TypeRef::Void,
            throws: vec![thread_error],
        };
        (temporary(TypeRef::Void), Some(call))
    }

    fn resolve_thread_scope_method(
        &mut self,
        name: &ast::Path,
        arguments: &[Expression],
        span: Span,
        context: &mut FunctionContext,
    ) -> (ExpressionInfo, Option<ResolvedCall>) {
        let callback_type = TypeRef::callback(
            CallbackKind::FnOnce,
            crate::interop::CallbackEscape::Scoped,
            Vec::new(),
            TypeRef::Void,
        );
        if name.segments.as_slice() != ["spawn"] || arguments.len() != 1 {
            for argument in arguments {
                self.resolve_expression(argument, None, context);
            }
            self.push(
                "RES116",
                if name.segments.as_slice() == ["spawn"] {
                    format!(
                        "`Scope.spawn()` requires one `void()` callback, found {} arguments",
                        arguments.len()
                    )
                } else {
                    format!("`thread::Scope` has no method `{}`", name.display())
                },
                span,
            );
            return (error_info(), None);
        }
        let actual = self.resolve_expression(&arguments[0], Some(&callback_type), context);
        self.require_exact(
            &callback_type,
            &actual.ty,
            arguments[0].span,
            "scoped thread callback",
        );
        let return_type = TypeRef::ScopedThreadHandle(Box::new(TypeRef::Void));
        let call = ResolvedCall {
            span,
            target: CallTarget::Intrinsic(Intrinsic::ScopedThreadSpawn),
            return_type: return_type.clone(),
            throws: Vec::new(),
        };
        (temporary(return_type), Some(call))
    }

    #[allow(clippy::too_many_arguments)]
    fn resolve_scoped_thread_handle_method(
        &mut self,
        target: &TypeRef,
        receiver: &Expression,
        receiver_info: &ExpressionInfo,
        name: &ast::Path,
        arguments: &[Expression],
        span: Span,
        context: &mut FunctionContext,
    ) -> (ExpressionInfo, Option<ResolvedCall>) {
        for argument in arguments {
            self.resolve_expression(argument, None, context);
        }
        if name.segments.as_slice() != ["join"] || !arguments.is_empty() {
            self.push(
                "RES116",
                format!(
                    "scoped thread handle has no matching `{}` method",
                    name.display()
                ),
                span,
            );
            return (error_info(), None);
        }
        let owned_name = receiver_info.category == ValueCategory::MutablePlace
            && !receiver_info.ty.is_reference()
            && is_named_value_expression(receiver);
        if receiver_info.category != ValueCategory::Temporary && !owned_name {
            self.push(
                "RES116",
                "scoped `join()` consumes a mutable owned handle".to_owned(),
                receiver.span,
            );
            return (error_info(), None);
        }
        let call = ResolvedCall {
            span,
            target: CallTarget::Intrinsic(Intrinsic::ScopedThreadJoin),
            return_type: target.clone(),
            throws: Vec::new(),
        };
        (info_for_return_type(target.clone()), Some(call))
    }

    #[allow(clippy::too_many_arguments)]
    fn resolve_thread_handle_method(
        &mut self,
        target: &TypeRef,
        receiver: &Expression,
        receiver_info: &ExpressionInfo,
        name: &ast::Path,
        arguments: &[Expression],
        span: Span,
        context: &mut FunctionContext,
    ) -> (ExpressionInfo, Option<ResolvedCall>) {
        for argument in arguments {
            self.resolve_expression(argument, None, context);
        }
        if name.segments.as_slice() != ["join"] || !arguments.is_empty() {
            self.push(
                "RES115",
                if name.segments.as_slice() == ["join"] {
                    format!(
                        "`JoinHandle<T>.join()` requires no arguments, found {}",
                        arguments.len()
                    )
                } else {
                    format!("`JoinHandle<T>` has no method `{}`", name.display())
                },
                span,
            );
            return (error_info(), None);
        }
        let owned_name = receiver_info.category == ValueCategory::MutablePlace
            && !receiver_info.ty.is_reference()
            && is_named_value_expression(receiver);
        if receiver_info.category != ValueCategory::Temporary && !owned_name {
            self.push(
                "RES115",
                "`JoinHandle<T>.join()` consumes a mutable owned handle".to_owned(),
                receiver.span,
            );
            return (error_info(), None);
        }
        let thread_error = self.thread_error_struct();
        self.validate_checked_effect(thread_error, span, context);
        let call = ResolvedCall {
            span,
            target: CallTarget::Intrinsic(Intrinsic::ThreadJoin),
            return_type: target.clone(),
            throws: vec![thread_error],
        };
        (info_for_return_type(target.clone()), Some(call))
    }

    fn resolve_atomic_pointer_method(
        &mut self,
        kind: PointerKind,
        target: &TypeRef,
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
                "RES109",
                "atomic pointer operations cannot be qualified".to_owned(),
                span,
            );
            return (error_info(), None);
        }
        let nullable = kind == PointerKind::AtomicNullable;
        let value_kind = if nullable {
            PointerKind::SharedNullable
        } else {
            PointerKind::Shared
        };
        let value_type = TypeRef::pointer(value_kind, target.clone());
        let (intrinsic, return_type, expected_arity) = match name.segments[0].as_str() {
            "__load" => (
                Intrinsic::AtomicLoad {
                    nullable,
                    target: target.clone(),
                },
                value_type.clone(),
                0,
            ),
            "__store" => (
                Intrinsic::AtomicStore {
                    nullable,
                    target: target.clone(),
                },
                TypeRef::Void,
                1,
            ),
            "__swap" => (
                Intrinsic::AtomicSwap {
                    nullable,
                    target: target.clone(),
                },
                value_type.clone(),
                1,
            ),
            operation => {
                for argument in arguments {
                    self.resolve_expression(argument, None, context);
                }
                self.push(
                    "RES109",
                    format!("unknown atomic pointer operation `{operation}`"),
                    span,
                );
                return (error_info(), None);
            }
        };
        if arguments.len() != expected_arity {
            for argument in arguments {
                self.resolve_expression(argument, None, context);
            }
            self.push(
                "RES109",
                format!(
                    "`{}` requires {expected_arity} argument(s), found {}",
                    name.display(),
                    arguments.len()
                ),
                span,
            );
            return (error_info(), None);
        }
        if let Some(argument) = arguments.first() {
            let actual = self.resolve_expression(argument, Some(&value_type), context);
            self.validate_binding(&value_type, &actual, argument.span, "atomic pointer value");
        }
        let call = ResolvedCall {
            span,
            target: CallTarget::Intrinsic(intrinsic),
            return_type: return_type.clone(),
            throws: Vec::new(),
        };
        (info_for_return_type(return_type), Some(call))
    }

    fn resolve_stored_function_call(
        &mut self,
        callee: &ExpressionInfo,
        arguments: &[Expression],
        span: Span,
        context: &mut FunctionContext,
    ) -> (ExpressionInfo, Option<ResolvedCall>) {
        let TypeRef::Function(function) = canonical_ref(&callee.ty) else {
            for argument in arguments {
                self.resolve_expression(argument, None, context);
            }
            if callee.ty != TypeRef::Error {
                self.push(
                    "RES014",
                    format!("type `{}` is not callable", display_type(&callee.ty)),
                    span,
                );
            }
            return (error_info(), None);
        };
        let function = function.as_ref().clone();
        if arguments.len() != function.parameters.len() {
            self.push(
                "RES095",
                format!(
                    "stored function requires {} argument(s), found {}",
                    function.parameters.len(),
                    arguments.len()
                ),
                span,
            );
        }
        let actual = self.resolve_arguments(arguments, Some(&function.parameters), context);
        for ((expected, actual), syntax) in function.parameters.iter().zip(&actual).zip(arguments) {
            let actual = self.adjusted_binding_actual(expected, actual, syntax, context);
            self.validate_binding(expected, &actual, syntax.span, "stored function argument");
        }
        let mutable = function.kind == StoredFunctionKind::Mutable;
        if mutable && callee.category != ValueCategory::MutablePlace {
            self.push(
                "RES096",
                "calling `function_mut` requires a mutable callable binding".to_owned(),
                span,
            );
        }
        let return_type = function.return_type.as_ref().clone();
        let call = ResolvedCall {
            span,
            target: CallTarget::Intrinsic(Intrinsic::StoredFunctionCall { mutable }),
            return_type: return_type.clone(),
            throws: Vec::new(),
        };
        (info_for_return_type(return_type), Some(call))
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
        let (exception_id, exception) = self.native_result_exception(error_type);
        let call = ResolvedCall {
            span,
            target: CallTarget::Intrinsic(Intrinsic::UnwrapRustResult {
                error_message: self.rust_error_message(error_type),
                exception,
            }),
            return_type: value_type.clone(),
            throws: vec![exception_id],
        };
        (temporary(value_type.clone()), Some(call))
    }

    #[allow(clippy::too_many_arguments, clippy::too_many_lines)]
    fn resolve_struct_method(
        &mut self,
        structure: StructId,
        structure_type: &TypeRef,
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
        let owner_is_interface =
            self.model.structs[structure.0].kind == ast::UserTypeKind::Interface;
        let substitutions =
            user_type_substitutions(&self.model.structs[structure.0], structure_type);
        let mut path = self.model.structs[structure.0].path.clone();
        path.push(name.segments[0].clone());
        let candidates = if owner_is_interface {
            let mut candidates = Vec::new();
            self.collect_interface_member_candidates(
                structure,
                &name.segments[0],
                &mut BTreeSet::new(),
                &mut candidates,
            );
            candidates
        } else {
            self.function_sets.get(&path).cloned().unwrap_or_default()
        };
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
                .map(|parameter| canonical(&substitute_type(&parameter.ty, &substitutions)))
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
                    .map(|parameter| canonical(&substitute_type(&parameter.ty, &substitutions)))
                    .eq(actual.iter().map(|argument| canonical(&argument.ty)))
            })
            .collect::<Vec<_>>();
        let compatible = if exact.is_empty() {
            candidates
                .iter()
                .copied()
                .filter(|id| {
                    self.model.functions[id.0]
                        .parameters
                        .iter()
                        .zip(&actual)
                        .zip(arguments)
                        .all(|((parameter, argument), syntax)| {
                            let parameter_ty = substitute_type(&parameter.ty, &substitutions);
                            let adjusted = self.adjusted_binding_actual(
                                &parameter_ty,
                                argument,
                                syntax,
                                context,
                            );
                            self.is_derived_reference_binding(&parameter_ty, &adjusted.ty)
                                || self.is_interface_owner_binding(&parameter_ty, &adjusted.ty)
                                || Self::is_shared_to_weak_binding(&parameter_ty, &adjusted.ty)
                        })
                })
                .collect::<Vec<_>>()
        } else {
            exact
        };
        if compatible.len() != 1 {
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
        let id = compatible[0];
        let symbol = self.model.functions[id.0].clone();
        let declaring_type = symbol
            .receiver
            .as_ref()
            .expect("member candidate has a receiver")
            .structure;
        if !Self::member_is_accessible(declaring_type, symbol.is_public, context) {
            self.push(
                "RES121",
                format!("member function `{}` is private", name.display()),
                span,
            );
        }
        if !owner_is_interface && !symbol.has_definition {
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
            let parameter_ty = substitute_type(&parameter.ty, &substitutions);
            let argument = self.adjusted_binding_actual(&parameter_ty, argument, syntax, context);
            self.validate_binding(&parameter_ty, &argument, syntax.span, "argument");
        }
        let return_type = substitute_type(&symbol.return_type, &substitutions);
        let call = ResolvedCall {
            span,
            target: if owner_is_interface {
                CallTarget::InterfaceMethod(id)
            } else {
                CallTarget::Stainless(id)
            },
            return_type: return_type.clone(),
            throws: symbol.throws,
        };
        (info_for_return_type(return_type), Some(call))
    }

    fn collect_interface_member_candidates(
        &self,
        interface: StructId,
        name: &str,
        visited: &mut BTreeSet<StructId>,
        candidates: &mut Vec<FunctionId>,
    ) {
        if !visited.insert(interface) {
            return;
        }
        let mut path = self.model.structs[interface.0].path.clone();
        path.push(name.to_owned());
        if let Some(direct) = self.function_sets.get(&path) {
            candidates.extend(direct.iter().copied());
        }
        for base in &self.model.structs[interface.0].interfaces {
            self.collect_interface_member_candidates(*base, name, visited, candidates);
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
        let guard_type = canonical_ref(&argument.ty);
        if matches!(
            guard_type,
            TypeRef::MutexGuard(_) | TypeRef::RwLockReadGuard(_) | TypeRef::RwLockWriteGuard(_)
        ) {
            self.push(
                "RES114",
                if matches!(guard_type, TypeRef::MutexGuard(_)) {
                    "mutex guards cannot be moved explicitly; `condition.wait(guard)` performs the only permitted guard transfer"
                        .to_owned()
                } else {
                    "rwlock guards cannot be moved explicitly".to_owned()
                },
                arguments[0].span,
            );
        }
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

    fn resolve_mutex_constructor(
        &mut self,
        type_arguments: &[ast::Type],
        arguments: &[Expression],
        span: Span,
        context: &mut FunctionContext,
    ) -> (ExpressionInfo, Option<ResolvedCall>) {
        if type_arguments.len() != 1 {
            for argument in arguments {
                self.resolve_expression(argument, None, context);
            }
            self.push(
                "RES113",
                format!(
                    "`mutex` requires one protected type argument, found {}",
                    type_arguments.len()
                ),
                span,
            );
            return (error_info(), None);
        }
        let target = self.resolve_type(
            &type_arguments[0],
            &context.namespace,
            &context.type_parameters,
            false,
        );
        if target == TypeRef::Error {
            for argument in arguments {
                self.resolve_expression(argument, None, context);
            }
            return (error_info(), None);
        }
        if matches!(target, TypeRef::Void | TypeRef::Reference { .. })
            || target.contains_reference()
        {
            for argument in arguments {
                self.resolve_expression(argument, None, context);
            }
            self.push(
                "RES113",
                format!(
                    "`mutex` cannot protect type `{}` because it contains a reference",
                    display_type(&target)
                ),
                type_arguments[0].span,
            );
            return (error_info(), None);
        }
        let return_type = TypeRef::Mutex(Box::new(target.clone()));
        let Some(construction) = self.resolve_slot_construction(&target, arguments, span, context)
        else {
            return (error_info(), None);
        };
        let throws = construction.throws.clone();
        let call = ResolvedCall {
            span,
            target: CallTarget::Intrinsic(Intrinsic::MutexNew {
                target,
                construction: Box::new(construction),
            }),
            return_type: return_type.clone(),
            throws,
        };
        (temporary(return_type), Some(call))
    }

    fn resolve_tuple_constructor(
        &mut self,
        type_arguments: &[ast::Type],
        arguments: &[Expression],
        span: Span,
        context: &mut FunctionContext,
    ) -> (ExpressionInfo, Option<ResolvedCall>) {
        let elements = type_arguments
            .iter()
            .map(|element| {
                self.resolve_type(element, &context.namespace, &context.type_parameters, false)
            })
            .collect::<Vec<_>>();
        if !(2..=12).contains(&elements.len()) {
            for argument in arguments {
                self.resolve_expression(argument, None, context);
            }
            self.push(
                "RES119",
                format!(
                    "`tuple` requires between 2 and 12 element types, found {}",
                    elements.len()
                ),
                span,
            );
            return (error_info(), None);
        }
        if elements.iter().any(|element| {
            matches!(
                element,
                TypeRef::Error | TypeRef::Void | TypeRef::Reference { .. }
            ) || element.contains_reference()
        }) {
            for argument in arguments {
                self.resolve_expression(argument, None, context);
            }
            if !elements.contains(&TypeRef::Error) {
                self.push(
                    "RES119",
                    "tuple elements must be storable value types".to_owned(),
                    span,
                );
            }
            return (error_info(), None);
        }
        let return_type = TypeRef::Tuple(elements.clone());
        let Some(call) = self.resolve_tuple_slot_construction(&elements, arguments, span, context)
        else {
            return (error_info(), None);
        };
        (temporary(return_type), Some(call))
    }

    fn resolve_rwlock_constructor(
        &mut self,
        type_arguments: &[ast::Type],
        arguments: &[Expression],
        span: Span,
        context: &mut FunctionContext,
    ) -> (ExpressionInfo, Option<ResolvedCall>) {
        if type_arguments.len() != 1 {
            for argument in arguments {
                self.resolve_expression(argument, None, context);
            }
            self.push(
                "RES113",
                format!(
                    "`rwlock` requires one protected type argument, found {}",
                    type_arguments.len()
                ),
                span,
            );
            return (error_info(), None);
        }
        let target = self.resolve_type(
            &type_arguments[0],
            &context.namespace,
            &context.type_parameters,
            false,
        );
        if target == TypeRef::Error {
            for argument in arguments {
                self.resolve_expression(argument, None, context);
            }
            return (error_info(), None);
        }
        if matches!(target, TypeRef::Void | TypeRef::Reference { .. })
            || target.contains_reference()
        {
            for argument in arguments {
                self.resolve_expression(argument, None, context);
            }
            self.push(
                "RES113",
                format!(
                    "`rwlock` cannot protect type `{}` because it contains a reference",
                    display_type(&target)
                ),
                type_arguments[0].span,
            );
            return (error_info(), None);
        }
        let return_type = TypeRef::RwLock(Box::new(target.clone()));
        let Some(construction) = self.resolve_slot_construction(&target, arguments, span, context)
        else {
            return (error_info(), None);
        };
        let throws = construction.throws.clone();
        let call = ResolvedCall {
            span,
            target: CallTarget::Intrinsic(Intrinsic::RwLockNew {
                target,
                construction: Box::new(construction),
            }),
            return_type: return_type.clone(),
            throws,
        };
        (temporary(return_type), Some(call))
    }

    fn resolve_make_owner(
        &mut self,
        kind: PointerKind,
        type_arguments: &[ast::Type],
        arguments: &[Expression],
        span: Span,
        syntax: ConstructionSyntax,
        context: &mut FunctionContext,
    ) -> (ExpressionInfo, Option<ResolvedCall>) {
        let builtin_name = match kind {
            PointerKind::Unique => "make_unique",
            PointerKind::Shared => "make_shared",
            _ => "owner allocation",
        };
        if type_arguments.len() != 1 {
            for argument in arguments {
                self.resolve_expression(argument, None, context);
            }
            self.push(
                "RES105",
                format!(
                    "`{builtin_name}` requires one pointee type argument, found {}",
                    type_arguments.len()
                ),
                span,
            );
            return (error_info(), None);
        }
        let target = self.resolve_type(
            &type_arguments[0],
            &context.namespace,
            &context.type_parameters,
            false,
        );
        if matches!(target, TypeRef::Void | TypeRef::Reference { .. }) {
            for argument in arguments {
                self.resolve_expression(argument, None, context);
            }
            self.push(
                "RES105",
                format!(
                    "`{builtin_name}` cannot allocate pointee type `{}`",
                    display_type(&target)
                ),
                type_arguments[0].span,
            );
            return (error_info(), None);
        }
        if target == TypeRef::Error {
            for argument in arguments {
                self.resolve_expression(argument, None, context);
            }
            return (error_info(), None);
        }
        let construction = match syntax {
            ConstructionSyntax::Parenthesized => {
                self.resolve_slot_construction(&target, arguments, span, context)
            }
            ConstructionSyntax::Braced => {
                self.resolve_braced_slot_construction(&target, arguments, span, context)
            }
        };
        let Some(construction) = construction else {
            return (error_info(), None);
        };
        let return_type = TypeRef::pointer(kind, target.clone());
        let throws = construction.throws.clone();
        let call = ResolvedCall {
            span,
            target: CallTarget::Intrinsic(Intrinsic::MakeOwner {
                kind,
                target,
                construction: Box::new(construction),
            }),
            return_type: return_type.clone(),
            throws,
        };
        (temporary(return_type), Some(call))
    }

    fn resolve_braced_slot_construction(
        &mut self,
        target: &TypeRef,
        initializers: &[Expression],
        span: Span,
        context: &mut FunctionContext,
    ) -> Option<ResolvedCall> {
        match canonical_ref(target) {
            TypeRef::Mutex(protected) => {
                let construction =
                    self.resolve_braced_slot_construction(protected, initializers, span, context)?;
                let throws = construction.throws.clone();
                Some(ResolvedCall {
                    span,
                    target: CallTarget::Intrinsic(Intrinsic::MutexNew {
                        target: protected.as_ref().clone(),
                        construction: Box::new(construction),
                    }),
                    return_type: TypeRef::Mutex(protected.clone()),
                    throws,
                })
            }
            TypeRef::RwLock(protected) => {
                let construction =
                    self.resolve_braced_slot_construction(protected, initializers, span, context)?;
                let throws = construction.throws.clone();
                Some(ResolvedCall {
                    span,
                    target: CallTarget::Intrinsic(Intrinsic::RwLockNew {
                        target: protected.as_ref().clone(),
                        construction: Box::new(construction),
                    }),
                    return_type: TypeRef::RwLock(protected.clone()),
                    throws,
                })
            }
            TypeRef::Struct { path, .. } => {
                let structure = self.struct_by_path.get(path).copied()?;
                self.resolve_struct_aggregate(
                    structure,
                    canonical_ref(target).clone(),
                    initializers,
                    span,
                    context,
                )
                .1
            }
            TypeRef::Class { path, .. } => {
                let structure = self.struct_by_path.get(path).copied()?;
                self.resolve_user_constructor(
                    structure,
                    canonical_ref(target).clone(),
                    initializers,
                    span,
                    context,
                )
                .1
            }
            TypeRef::Interface { path, .. } => {
                for initializer in initializers {
                    self.resolve_expression(initializer, None, context);
                }
                self.push(
                    "RES118",
                    format!("interface `{}` cannot be allocated", display_path(path)),
                    span,
                );
                None
            }
            _ => self.resolve_slot_construction(target, initializers, span, context),
        }
    }

    #[allow(clippy::too_many_lines)]
    fn resolve_pointer_constructor(
        &mut self,
        kind: PointerKind,
        type_arguments: &[ast::Type],
        arguments: &[Expression],
        span: Span,
        context: &mut FunctionContext,
    ) -> (ExpressionInfo, Option<ResolvedCall>) {
        if type_arguments.len() != 1 {
            for argument in arguments {
                self.resolve_expression(argument, None, context);
            }
            self.push(
                "RES107",
                format!(
                    "`{}` requires one pointee type argument, found {}",
                    pointer_name(kind),
                    type_arguments.len()
                ),
                span,
            );
            return (error_info(), None);
        }
        let target = self.resolve_type(
            &type_arguments[0],
            &context.namespace,
            &context.type_parameters,
            false,
        );
        if target == TypeRef::Error {
            for argument in arguments {
                self.resolve_expression(argument, None, context);
            }
            return (error_info(), None);
        }
        let return_type = TypeRef::pointer(kind, target.clone());
        if arguments.is_empty() {
            if matches!(
                kind,
                PointerKind::UniqueNullable
                    | PointerKind::SharedNullable
                    | PointerKind::Weak
                    | PointerKind::AtomicNullable
            ) {
                let call = ResolvedCall {
                    span,
                    target: CallTarget::Intrinsic(Intrinsic::PointerDefault { kind, target }),
                    return_type: return_type.clone(),
                    throws: Vec::new(),
                };
                return (temporary(return_type), Some(call));
            }
            self.push(
                "RES106",
                format!("`{}<T>` has no default constructor", pointer_name(kind)),
                span,
            );
            return (error_info(), None);
        }
        if arguments.len() != 1 {
            for argument in arguments {
                self.resolve_expression(argument, None, context);
            }
            self.push(
                "RES107",
                format!(
                    "`{}<T>` pointer conversion requires exactly one argument",
                    pointer_name(kind)
                ),
                span,
            );
            return (error_info(), None);
        }
        if matches!(
            arguments[0].kind,
            ExpressionKind::Literal(ast::Literal {
                kind: LiteralKind::Null,
                ref text,
            }) if text == "nullptr"
        ) && matches!(
            kind,
            PointerKind::UniqueNullable | PointerKind::SharedNullable | PointerKind::AtomicNullable
        ) {
            let call = ResolvedCall {
                span,
                target: CallTarget::Intrinsic(Intrinsic::PointerDefault { kind, target }),
                return_type: return_type.clone(),
                throws: Vec::new(),
            };
            return (temporary(return_type), Some(call));
        }
        let actual = self.resolve_expression(&arguments[0], None, context);
        let TypeRef::Pointer {
            kind: source_kind,
            target: source_target,
        } = canonical(&actual.ty)
        else {
            if actual.ty != TypeRef::Error {
                self.push(
                    "RES107",
                    format!(
                        "`{}<T>` requires a compatible pointer argument, found `{}`",
                        pointer_name(kind),
                        display_type(&actual.ty)
                    ),
                    arguments[0].span,
                );
            }
            return (error_info(), None);
        };
        if canonical(&target) != canonical(&source_target) {
            self.push(
                "RES107",
                format!(
                    "pointer conversion requires pointee `{}`, found `{}`",
                    display_type(&target),
                    display_type(&source_target)
                ),
                arguments[0].span,
            );
            return (error_info(), None);
        }
        if source_kind == kind {
            self.validate_binding(
                &return_type,
                &actual,
                arguments[0].span,
                "pointer constructor",
            );
            let call = ResolvedCall {
                span,
                target: CallTarget::Intrinsic(Intrinsic::ValueInitialization {
                    target: return_type.clone(),
                }),
                return_type: return_type.clone(),
                throws: Vec::new(),
            };
            return (temporary(return_type), Some(call));
        }
        let supported = match source_kind {
            PointerKind::Unique => kind == PointerKind::UniqueNullable,
            PointerKind::UniqueNullable => kind == PointerKind::Unique,
            PointerKind::Shared => matches!(
                kind,
                PointerKind::SharedNullable | PointerKind::Atomic | PointerKind::AtomicNullable
            ),
            PointerKind::SharedNullable => {
                matches!(kind, PointerKind::Shared | PointerKind::AtomicNullable)
            }
            PointerKind::Weak | PointerKind::Atomic | PointerKind::AtomicNullable => false,
        };
        if !supported {
            self.push(
                "RES107",
                format!(
                    "conversion from `{}` to `{}` requires non-null refinement or is not supported",
                    pointer_name(source_kind),
                    pointer_name(kind)
                ),
                span,
            );
            return (error_info(), None);
        }
        if matches!(
            (source_kind, kind),
            (PointerKind::UniqueNullable, PointerKind::Unique)
                | (PointerKind::SharedNullable, PointerKind::Shared)
        ) && self.expression_null_state(&arguments[0], context) != NullState::NonNull
        {
            self.push(
                "RES110",
                format!(
                    "conversion from `{}` to `{}` requires a non-null guard",
                    pointer_name(source_kind),
                    pointer_name(kind)
                ),
                arguments[0].span,
            );
            return (error_info(), None);
        }
        self.validate_value_use(&actual.ty, &actual, arguments[0].span, "pointer conversion");
        let call = ResolvedCall {
            span,
            target: CallTarget::Intrinsic(Intrinsic::PointerConversion {
                from: source_kind,
                to: kind,
                target,
            }),
            return_type: return_type.clone(),
            throws: Vec::new(),
        };
        (temporary(return_type), Some(call))
    }

    fn resolve_weak_pointer_method(
        &mut self,
        kind: PointerKind,
        target: &TypeRef,
        name: &ast::Path,
        arguments: &[Expression],
        span: Span,
        context: &mut FunctionContext,
    ) -> (ExpressionInfo, Option<ResolvedCall>) {
        let method = name.segments.first().map_or("", String::as_str);
        if !arguments.is_empty() {
            for argument in arguments {
                self.resolve_expression(argument, None, context);
            }
            self.push(
                "RES108",
                format!(
                    "`{}<T>.{method}()` requires no arguments, found {}",
                    pointer_name(kind),
                    arguments.len()
                ),
                span,
            );
            return (error_info(), None);
        }
        let (intrinsic, result_kind) = match (method, kind) {
            ("__downgrade", PointerKind::Shared) => (
                Intrinsic::DowngradeShared {
                    target: target.clone(),
                },
                PointerKind::Weak,
            ),
            ("lock", PointerKind::Weak) => (
                Intrinsic::LockWeak {
                    target: target.clone(),
                },
                PointerKind::SharedNullable,
            ),
            _ => {
                self.push(
                    "RES108",
                    format!("`{}<T>` has no method `{method}`", pointer_name(kind)),
                    span,
                );
                return (error_info(), None);
            }
        };
        let return_type = TypeRef::pointer(result_kind, target.clone());
        let call = ResolvedCall {
            span,
            target: CallTarget::Intrinsic(intrinsic),
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
        if is_json_var(&canonical(&argument.ty)) && is_numeric(&target) {
            let call = ResolvedCall {
                span,
                target: CallTarget::Intrinsic(Intrinsic::JsonCast {
                    target: target.clone(),
                }),
                return_type: target.clone(),
                throws: Vec::new(),
            };
            return (temporary(target), Some(call));
        }
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

    fn resolve_json_wrap(
        &mut self,
        arguments: &[Expression],
        span: Span,
        context: &mut FunctionContext,
    ) -> (ExpressionInfo, Option<ResolvedCall>) {
        if arguments.is_empty() {
            let instance = NativeInstance {
                type_path: VAR_TYPE_PATH.to_owned(),
                arguments: Vec::new(),
            };
            return self.resolve_native_callable(
                &instance,
                CallStyle::Constructor,
                "Var",
                arguments,
                span,
                None,
                context,
            );
        }
        if arguments.len() != 1 {
            for argument in arguments {
                self.resolve_expression(argument, None, context);
            }
            self.push(
                "RES103",
                "`var` conversion requires zero or one argument".to_owned(),
                span,
            );
            return (error_info(), None);
        }
        let actual = self.resolve_expression(&arguments[0], None, context);
        let ty = canonical(&actual.ty);
        if let Some(reason) = self.json_conversion_error(&ty) {
            self.push(
                "RES103",
                format!("cannot convert `{}` to `var`: {reason}", display_type(&ty)),
                arguments[0].span,
            );
            return (error_info(), None);
        }
        self.record_json_conversions(&ty);
        self.validate_value_use(&ty, &actual, arguments[0].span, "var conversion");
        let target = json_var_type();
        let call = ResolvedCall {
            span,
            target: CallTarget::Intrinsic(Intrinsic::JsonWrap),
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
            arguments: Vec::new(),
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
        structure_type: TypeRef,
        arguments: &[Expression],
        span: Span,
        context: &mut FunctionContext,
    ) -> (ExpressionInfo, Option<ResolvedCall>) {
        let structure_symbol = self.model.structs[structure.0].clone();
        if structure_symbol.kind == ast::UserTypeKind::Interface {
            for argument in arguments {
                self.resolve_expression(argument, None, context);
            }
            self.push(
                "RES118",
                format!(
                    "interface `{}` cannot be constructed",
                    display_path(&structure_symbol.path)
                ),
                span,
            );
            return (error_info(), None);
        }
        let substitutions = user_type_substitutions(&structure_symbol, &structure_type);
        let candidates = self
            .constructor_sets
            .get(&structure)
            .cloned()
            .unwrap_or_default();
        let mut arity_candidates = candidates
            .into_iter()
            .filter(|id| self.model.constructors[id.0].parameters.len() == arguments.len())
            .collect::<Vec<_>>();
        if arity_candidates.is_empty() {
            if arguments.len() == 1 {
                let target = structure_type.clone();
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

        let integer_literal_candidates = arity_candidates
            .iter()
            .copied()
            .filter(|id| {
                self.model.constructors[id.0]
                    .parameters
                    .iter()
                    .zip(arguments)
                    .all(|(parameter, argument)| {
                        !is_unsuffixed_integer_literal(argument)
                            || is_integer(&canonical(&substitute_type(
                                &parameter.ty,
                                &substitutions,
                            )))
                    })
            })
            .collect::<Vec<_>>();
        if !integer_literal_candidates.is_empty() {
            arity_candidates = integer_literal_candidates;
        }

        let contextual_parameters = (arity_candidates.len() == 1).then(|| {
            self.model.constructors[arity_candidates[0].0]
                .parameters
                .iter()
                .map(|parameter| canonical(&substitute_type(&parameter.ty, &substitutions)))
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
                    .map(|parameter| canonical(&substitute_type(&parameter.ty, &substitutions)))
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
                        .zip(arguments)
                        .all(|((parameter, argument), syntax)| {
                            let parameter_ty = substitute_type(&parameter.ty, &substitutions);
                            let adjusted = self.adjusted_binding_actual(
                                &parameter_ty,
                                argument,
                                syntax,
                                context,
                            );
                            self.is_derived_reference_binding(&parameter_ty, &adjusted.ty)
                                || self.is_interface_owner_binding(&parameter_ty, &adjusted.ty)
                                || Self::is_shared_to_weak_binding(&parameter_ty, &adjusted.ty)
                        })
                })
                .collect::<Vec<_>>()
        } else {
            exact
        };
        if compatible.is_empty()
            && arguments.len() == 1
            && actual
                .first()
                .is_some_and(|argument| canonical(&argument.ty) == structure_type)
        {
            if structure_symbol.kind == ast::UserTypeKind::Class {
                self.push(
                    "RES119",
                    format!(
                        "class `{}` cannot be copied; use `move(...)` or an explicit `clone()` API",
                        display_path(&structure_symbol.path)
                    ),
                    span,
                );
                return (error_info(), None);
            }
            let target = structure_type.clone();
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
        if !Self::member_is_accessible(structure, symbol.is_public, context) {
            self.push(
                "RES121",
                format!(
                    "constructor of `{}` is private",
                    display_path(&structure_symbol.path)
                ),
                span,
            );
        }
        for ((parameter, argument), expression) in
            symbol.parameters.iter().zip(&actual).zip(arguments)
        {
            let parameter_ty = substitute_type(&parameter.ty, &substitutions);
            let argument =
                self.adjusted_binding_actual(&parameter_ty, argument, expression, context);
            self.validate_binding(
                &parameter_ty,
                &argument,
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
        let return_type = structure_type;
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
        let mut arity_candidates = candidates
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

        let integer_literal_candidates = arity_candidates
            .iter()
            .copied()
            .filter(|id| {
                self.model.functions[id.0]
                    .parameters
                    .iter()
                    .zip(arguments)
                    .all(|(parameter, argument)| {
                        !is_unsuffixed_integer_literal(argument)
                            || is_integer(&canonical(&parameter.ty))
                    })
            })
            .collect::<Vec<_>>();
        if !integer_literal_candidates.is_empty() {
            arity_candidates = integer_literal_candidates;
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
                        .zip(arguments)
                        .all(|((parameter, argument), syntax)| {
                            let adjusted = self.adjusted_binding_actual(
                                &parameter.ty,
                                argument,
                                syntax,
                                context,
                            );
                            canonical(&parameter.ty) == canonical(&argument.ty)
                                || self.is_derived_reference_binding(&parameter.ty, &adjusted.ty)
                                || self.is_interface_owner_binding(&parameter.ty, &adjusted.ty)
                                || Self::is_shared_to_weak_binding(&parameter.ty, &adjusted.ty)
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
        if symbol.receiver.is_some() {
            self.push(
                "RES127",
                format!(
                    "non-static member function `{}` requires an object receiver",
                    display_path(&symbol.path)
                ),
                span,
            );
            return (error_info(), None);
        }
        for ((parameter, argument), expression) in
            symbol.parameters.iter().zip(&actual).zip(arguments)
        {
            let argument =
                self.adjusted_binding_actual(&parameter.ty, argument, expression, context);
            self.validate_binding(&parameter.ty, &argument, expression.span, "argument");
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

    #[allow(clippy::too_many_arguments, clippy::too_many_lines)]
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
        let Some(mut candidates) =
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
            if style == CallStyle::Constructor && arguments.len() == 1 {
                let target = TypeRef::Native {
                    path: instance.type_path.clone(),
                    arguments: instance.arguments.clone(),
                };
                let call =
                    self.resolve_direct_initialization(&target, &arguments[0], span, context);
                return (
                    call.as_ref().map_or_else(error_info, |_| temporary(target)),
                    call,
                );
            }
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

        let lambda_arity_candidates = candidates
            .iter()
            .filter(|candidate| {
                candidate
                    .parameter_types
                    .iter()
                    .zip(arguments)
                    .all(|(expected, argument)| {
                        let ExpressionKind::Lambda { parameters, .. } = &argument.kind else {
                            return true;
                        };
                        match canonical_ref(expected) {
                            TypeRef::Callback(callback) => {
                                callback.parameters.len() == parameters.len()
                            }
                            TypeRef::Function(function) => {
                                function.parameters.len() == parameters.len()
                            }
                            _ => false,
                        }
                    })
            })
            .cloned()
            .collect::<Vec<_>>();
        if !lambda_arity_candidates.is_empty() {
            candidates = lambda_arity_candidates;
        }

        let fixed_width_literal_candidates = candidates
            .iter()
            .filter(|candidate| {
                candidate
                    .parameter_types
                    .iter()
                    .zip(arguments)
                    .enumerate()
                    .all(|(index, (parameter, argument))| {
                        if !is_unsuffixed_integer_literal(argument) {
                            return true;
                        }
                        let has_fixed_width_candidate = candidates.iter().any(|other| {
                            other
                                .parameter_types
                                .get(index)
                                .is_some_and(|ty| is_fixed_width_integer(canonical_ref(ty)))
                        });
                        !has_fixed_width_candidate
                            || is_fixed_width_integer(canonical_ref(parameter))
                    })
            })
            .cloned()
            .collect::<Vec<_>>();
        if !fixed_width_literal_candidates.is_empty() {
            candidates = fixed_width_literal_candidates;
        }

        let contextual = (candidates.len() == 1).then(|| {
            candidates[0]
                .parameter_types
                .iter()
                .map(canonical)
                .collect::<Vec<_>>()
        });
        let actual = self.resolve_arguments(arguments, contextual.as_deref(), context);
        if instance.type_path == "rust::String"
            && style == CallStyle::Constructor
            && name == "String"
            && actual.len() == 1
            && is_json_var(&canonical(&actual[0].ty))
        {
            let target = TypeRef::native("rust::String", Vec::new());
            let call = ResolvedCall {
                span,
                target: CallTarget::Intrinsic(Intrinsic::JsonCast {
                    target: target.clone(),
                }),
                return_type: target.clone(),
                throws: Vec::new(),
            };
            return (temporary(target), Some(call));
        }
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
        let compatible = if exact.is_empty() && candidates.len() == 1 {
            candidates
                .iter()
                .filter(|candidate| {
                    candidate
                        .parameter_types
                        .iter()
                        .zip(&actual)
                        .all(|(expected, actual)| {
                            canonical(expected) == canonical(&actual.ty)
                                || is_json_var(&canonical(expected))
                                    && self.is_json_compatible(&canonical(&actual.ty))
                        })
                })
                .cloned()
                .collect::<Vec<_>>()
        } else {
            exact
        };
        if compatible.len() != 1 {
            let displayed_candidates = if compatible.is_empty() {
                &candidates
            } else {
                &compatible
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
        let candidate = compatible
            .into_iter()
            .next()
            .expect("one compatible candidate");
        self.finish_native_call(
            instance, style, candidate, &actual, arguments, span, receiver, name, context,
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
        context: &mut FunctionContext,
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
        let (result_adaptation, throws) = if let Some(error_type) = &candidate.rust_result_error {
            let (exception_id, exception) = self.native_result_exception(error_type);
            self.validate_checked_effect(exception_id, span, context);
            (
                Some(NativeCallResultAdaptation {
                    error_message: self.rust_error_message(error_type),
                    exception,
                }),
                vec![exception_id],
            )
        } else {
            (None, Vec::new())
        };
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
            is_async: candidate.callable.is_async,
            return_type: candidate.return_type.clone(),
            result_adaptation,
            lowering: candidate.callable.lowering,
            requirements: candidate.requirements,
        };
        let call = ResolvedCall {
            span,
            target: CallTarget::Native(Box::new(native_call)),
            return_type: candidate.return_type.clone(),
            throws,
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

    fn adjusted_binding_actual(
        &self,
        expected: &TypeRef,
        actual: &ExpressionInfo,
        expression: &Expression,
        context: &FunctionContext,
    ) -> ExpressionInfo {
        if !expected.is_reference()
            || self.expression_null_state(expression, context) != NullState::NonNull
        {
            return actual.clone();
        }
        let TypeRef::Pointer { kind, target } = canonical_ref(&actual.ty) else {
            return actual.clone();
        };
        let non_null_kind = match kind {
            PointerKind::UniqueNullable => PointerKind::Unique,
            PointerKind::SharedNullable => PointerKind::Shared,
            _ => return actual.clone(),
        };
        ExpressionInfo {
            ty: TypeRef::pointer(non_null_kind, target.as_ref().clone()),
            category: pointee_category(&actual.ty, actual.category),
        }
    }

    fn validate_binding(
        &mut self,
        expected: &TypeRef,
        actual: &ExpressionInfo,
        span: Span,
        description: &str,
    ) {
        if !expected.is_reference()
            && is_json_var(&canonical(expected))
            && self.is_json_compatible(&canonical(&actual.ty))
        {
            let actual_type = canonical(&actual.ty);
            self.record_json_conversions(&actual_type);
            self.validate_value_use(&actual_type, actual, span, description);
            return;
        }
        if !self.is_derived_reference_binding(expected, &actual.ty)
            && !self.is_interface_owner_binding(expected, &actual.ty)
            && !Self::is_shared_to_weak_binding(expected, &actual.ty)
        {
            self.require_exact(expected, &actual.ty, span, description);
        }
        match expected {
            TypeRef::Reference { mutable: true, .. }
                if actual.category != ValueCategory::MutablePlace
                    || is_shared_owner(&actual.ty) =>
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
        if let TypeRef::Interface {
            path: expected_path,
            ..
        } = canonical_ref(expected_target)
        {
            return match automatic_pointee(actual) {
                TypeRef::Class {
                    path: actual_path, ..
                } => self
                    .struct_by_path
                    .get(actual_path)
                    .zip(self.struct_by_path.get(expected_path))
                    .is_some_and(|(class, interface)| {
                        self.class_implements_interface(*class, *interface)
                            && self.class_supports_interface_object(*class)
                    }),
                TypeRef::Interface {
                    path: actual_path, ..
                } => {
                    actual_path == expected_path
                        || self.struct_by_path.get(actual_path).is_some_and(|actual| {
                            self.interface_inherits_path(*actual, expected_path)
                        })
                }
                _ => false,
            };
        }
        let TypeRef::Struct {
            path: expected_path,
            ..
        } = canonical_ref(expected_target)
        else {
            return false;
        };
        let TypeRef::Struct {
            path: actual_path, ..
        } = automatic_pointee(actual)
        else {
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

    fn is_interface_owner_binding(&self, expected: &TypeRef, actual: &TypeRef) -> bool {
        let (
            TypeRef::Pointer {
                kind: expected_kind,
                target: expected_target,
            },
            TypeRef::Pointer {
                kind: actual_kind,
                target: actual_target,
            },
        ) = (canonical_ref(expected), canonical_ref(actual))
        else {
            return false;
        };
        if expected_kind != actual_kind
            || !matches!(
                expected_kind,
                PointerKind::Unique
                    | PointerKind::UniqueNullable
                    | PointerKind::Shared
                    | PointerKind::SharedNullable
            )
        {
            return false;
        }
        let TypeRef::Interface {
            path: interface, ..
        } = canonical_ref(expected_target)
        else {
            return false;
        };
        let TypeRef::Class { path: class, .. } = canonical_ref(actual_target) else {
            return false;
        };
        self.struct_by_path
            .get(class)
            .zip(self.struct_by_path.get(interface))
            .is_some_and(|(class, interface)| {
                self.class_implements_interface(*class, *interface)
                    && self.class_supports_interface_object(*class)
            })
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

    fn class_supports_interface_object(&self, class: StructId) -> bool {
        let ty = resolved_structure_type(&self.model.structs[class.0]);
        self.thread_sendable(&ty) && self.thread_sync(&ty)
    }

    fn class_implements_interface(&self, class: StructId, interface: StructId) -> bool {
        self.model
            .interface_implementations
            .iter()
            .any(|implementation| {
                implementation.implementer == class && implementation.interface == interface
            })
    }

    fn interface_inherits_path(&self, interface: StructId, expected: &[String]) -> bool {
        self.interface_inherits_path_inner(interface, expected, &mut BTreeSet::new())
    }

    fn interface_inherits_path_inner(
        &self,
        interface: StructId,
        expected: &[String],
        visited: &mut BTreeSet<StructId>,
    ) -> bool {
        if !visited.insert(interface) {
            return false;
        }
        self.model.structs[interface.0]
            .interfaces
            .iter()
            .any(|base| {
                self.model.structs[base.0].path == expected
                    || self.interface_inherits_path_inner(*base, expected, visited)
            })
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
            && !self.is_copyable_type(&canonical(expected))
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
            let resolved = self.resolve_type(ty, namespace, &[], false);
            if resolved.is_reference() {
                self.push(
                    "RES070",
                    "a `throws` entry must be an exception struct value type".to_owned(),
                    ty.span,
                );
                continue;
            }
            let TypeRef::Struct { path, .. } = canonical(&resolved) else {
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

    #[allow(clippy::too_many_lines)]
    fn resolve_type(
        &mut self,
        ty: &ast::Type,
        namespace: &[String],
        type_parameters: &[String],
        allow_auto: bool,
    ) -> TypeRef {
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
            TypeKind::Function {
                mutable,
                parameters,
                return_type,
            } => {
                let resolved_parameters = parameters
                    .iter()
                    .map(|parameter| {
                        self.resolve_type(parameter, namespace, type_parameters, false)
                    })
                    .collect::<Vec<_>>();
                for (parameter, resolved) in parameters.iter().zip(&resolved_parameters) {
                    if canonical(resolved) == TypeRef::Void {
                        self.push(
                            "RES092",
                            "`void` is not a valid stored function parameter type".to_owned(),
                            parameter.span,
                        );
                    }
                }
                let return_type = self.resolve_type(return_type, namespace, type_parameters, false);
                if return_type.is_reference() || return_type.contains_reference() {
                    self.push(
                        "RES092",
                        "stored function return references are not supported".to_owned(),
                        ty.span,
                    );
                }
                TypeRef::function(
                    if *mutable {
                        StoredFunctionKind::Mutable
                    } else {
                        StoredFunctionKind::Shared
                    },
                    resolved_parameters,
                    return_type,
                )
            }
            TypeKind::Named(named) => {
                let segments = &named.path.segments;
                if let [name] = segments.as_slice()
                    && type_parameters.contains(name)
                {
                    if !named.arguments.is_empty() {
                        self.push(
                            "RES050",
                            format!("generic parameter `{name}` cannot have type arguments"),
                            ty.span,
                        );
                    }
                    TypeRef::Parameter(name.clone())
                } else if self.is_reserved_rust_path(
                    segments,
                    namespace,
                    &["rust", "std", "thread", "JoinHandle"],
                ) {
                    let arguments = named
                        .arguments
                        .iter()
                        .map(|argument| {
                            self.resolve_type(argument, namespace, type_parameters, false)
                        })
                        .collect::<Vec<_>>();
                    if arguments.len() != 1 {
                        self.push(
                            "RES115",
                            format!(
                                "`JoinHandle` expects one result type argument, found {}",
                                arguments.len()
                            ),
                            ty.span,
                        );
                        TypeRef::Error
                    } else if arguments[0].is_reference() || arguments[0].contains_reference() {
                        self.push(
                            "RES115",
                            "a thread result cannot contain a reference".to_owned(),
                            ty.span,
                        );
                        TypeRef::Error
                    } else {
                        TypeRef::ThreadHandle(Box::new(arguments[0].clone()))
                    }
                } else if self.is_reserved_rust_path(
                    segments,
                    namespace,
                    &["rust", "std", "thread", "Scope"],
                ) {
                    if !named.arguments.is_empty() {
                        self.push(
                            "RES116",
                            "`thread::Scope` does not accept source-level type arguments"
                                .to_owned(),
                            ty.span,
                        );
                    }
                    TypeRef::ThreadScope
                } else if self.is_reserved_rust_path(
                    segments,
                    namespace,
                    &["rust", "std", "thread", "ScopedJoinHandle"],
                ) {
                    let arguments = named
                        .arguments
                        .iter()
                        .map(|argument| {
                            self.resolve_type(argument, namespace, type_parameters, false)
                        })
                        .collect::<Vec<_>>();
                    if arguments.len() != 1 {
                        self.push(
                            "RES116",
                            format!(
                                "`ScopedJoinHandle` expects one result type argument, found {}",
                                arguments.len()
                            ),
                            ty.span,
                        );
                        TypeRef::Error
                    } else if arguments[0].is_reference() || arguments[0].contains_reference() {
                        self.push(
                            "RES116",
                            "a scoped thread result cannot contain a reference".to_owned(),
                            ty.span,
                        );
                        TypeRef::Error
                    } else {
                        TypeRef::ScopedThreadHandle(Box::new(arguments[0].clone()))
                    }
                } else if matches!(segments.as_slice(), [name] if name == "tuple") {
                    let arguments = named
                        .arguments
                        .iter()
                        .map(|argument| {
                            self.resolve_type(argument, namespace, type_parameters, false)
                        })
                        .collect::<Vec<_>>();
                    if !(2..=12).contains(&arguments.len()) {
                        self.push(
                            "RES119",
                            format!(
                                "`tuple` expects between 2 and 12 element types, found {}",
                                arguments.len()
                            ),
                            ty.span,
                        );
                        TypeRef::Error
                    } else if arguments.iter().any(|argument| {
                        matches!(argument, TypeRef::Void | TypeRef::Reference { .. })
                            || argument.contains_reference()
                    }) {
                        self.push(
                            "RES119",
                            "tuple elements must be storable value types".to_owned(),
                            ty.span,
                        );
                        TypeRef::Error
                    } else {
                        TypeRef::Tuple(arguments)
                    }
                } else if let Some(kind) = pointer_kind(segments) {
                    let arguments = named
                        .arguments
                        .iter()
                        .map(|argument| {
                            self.resolve_type(argument, namespace, type_parameters, false)
                        })
                        .collect::<Vec<_>>();
                    if arguments.len() != 1 {
                        self.push(
                            "RES104",
                            format!(
                                "`{}` expects one type argument, found {}",
                                pointer_name(kind),
                                arguments.len()
                            ),
                            ty.span,
                        );
                        TypeRef::Error
                    } else if matches!(arguments[0], TypeRef::Void | TypeRef::Reference { .. }) {
                        self.push(
                            "RES104",
                            format!(
                                "`{}` cannot use pointee type `{}`",
                                pointer_name(kind),
                                display_type(&arguments[0])
                            ),
                            ty.span,
                        );
                        TypeRef::Error
                    } else {
                        TypeRef::pointer(kind, arguments[0].clone())
                    }
                } else if matches!(segments.as_slice(), [name] if name == "mutex") {
                    let arguments = named
                        .arguments
                        .iter()
                        .map(|argument| {
                            self.resolve_type(argument, namespace, type_parameters, false)
                        })
                        .collect::<Vec<_>>();
                    if arguments.len() != 1 {
                        self.push(
                            "RES113",
                            format!(
                                "`mutex` expects one protected type argument, found {}",
                                arguments.len()
                            ),
                            ty.span,
                        );
                        TypeRef::Error
                    } else if matches!(arguments[0], TypeRef::Void | TypeRef::Reference { .. })
                        || arguments[0].contains_reference()
                    {
                        self.push(
                            "RES113",
                            format!(
                                "`mutex` cannot protect type `{}` because it contains a reference",
                                display_type(&arguments[0])
                            ),
                            ty.span,
                        );
                        TypeRef::Error
                    } else {
                        TypeRef::Mutex(Box::new(arguments[0].clone()))
                    }
                } else if matches!(segments.as_slice(), [name] if name == "rwlock") {
                    let arguments = named
                        .arguments
                        .iter()
                        .map(|argument| {
                            self.resolve_type(argument, namespace, type_parameters, false)
                        })
                        .collect::<Vec<_>>();
                    if arguments.len() != 1 {
                        self.push(
                            "RES113",
                            format!(
                                "`rwlock` expects one protected type argument, found {}",
                                arguments.len()
                            ),
                            ty.span,
                        );
                        TypeRef::Error
                    } else if matches!(arguments[0], TypeRef::Void | TypeRef::Reference { .. })
                        || arguments[0].contains_reference()
                    {
                        self.push(
                            "RES113",
                            format!(
                                "`rwlock` cannot protect type `{}` because it contains a reference",
                                display_type(&arguments[0])
                            ),
                            ty.span,
                        );
                        TypeRef::Error
                    } else {
                        TypeRef::RwLock(Box::new(arguments[0].clone()))
                    }
                } else if matches!(segments.as_slice(), [name] if name == "condition") {
                    if !named.arguments.is_empty() {
                        self.push(
                            "RES113",
                            "`condition` does not accept type arguments".to_owned(),
                            ty.span,
                        );
                    }
                    TypeRef::Condition
                } else if matches!(segments.as_slice(), [name] if name == "var") {
                    if !named.arguments.is_empty() {
                        self.push(
                            "RES033",
                            "native type `var` cannot have type arguments".to_owned(),
                            ty.span,
                        );
                    }
                    json_var_type()
                } else if let Some(primitive) = primitive_type(segments) {
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
                        .map(|argument| {
                            self.resolve_type(argument, namespace, type_parameters, false)
                        })
                        .collect::<Vec<_>>();
                    if let Some(id) = self.lookup_struct_path(segments, namespace) {
                        let symbol = self.model.structs[id.0].clone();
                        if arguments.len() != symbol.type_parameters.len() {
                            self.push(
                                "RES050",
                                format!(
                                    "type `{}` expects {} type argument(s), found {}",
                                    named.path.display(),
                                    symbol.type_parameters.len(),
                                    arguments.len()
                                ),
                                ty.span,
                            );
                            return TypeRef::Error;
                        }
                        if arguments.iter().any(|argument| {
                            matches!(argument, TypeRef::Void | TypeRef::Reference { .. })
                                || argument.contains_reference()
                        }) {
                            self.push(
                                "RES124",
                                format!(
                                    "type `{}` requires storable value type arguments",
                                    named.path.display()
                                ),
                                ty.span,
                            );
                            return TypeRef::Error;
                        }
                        match symbol.kind {
                            ast::UserTypeKind::Struct => TypeRef::Struct {
                                path: symbol.path,
                                arguments,
                            },
                            ast::UserTypeKind::Class => TypeRef::Class {
                                path: symbol.path,
                                arguments,
                            },
                            ast::UserTypeKind::Interface => TypeRef::Interface {
                                path: symbol.path,
                                arguments,
                            },
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
                        if path == "rust::Option"
                            && matches!(arguments.as_slice(), [TypeRef::Pointer { .. }])
                        {
                            self.push(
                                "RES111",
                                "`rust::Option<T>` cannot contain a Stainless pointer type; use the corresponding nullable pointer"
                                    .to_owned(),
                                ty.span,
                            );
                            TypeRef::Error
                        } else if expected_arity == Some(arguments.len()) {
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
            } else if matches!(
                value,
                TypeRef::Pointer {
                    kind: PointerKind::Atomic | PointerKind::AtomicNullable,
                    ..
                }
            ) {
                TypeRef::Reference {
                    mutable: !ty.is_const,
                    target: Box::new(value),
                }
            } else if let TypeRef::Pointer { kind, .. } = &value {
                self.push(
                    "RES106",
                    format!(
                        "references to `{}<T>` are not allowed; pass the owner by value or borrow its pointee",
                        pointer_name(*kind)
                    ),
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
        if matches!(segments, [name] if name == "var") {
            return Some(VAR_TYPE_PATH.to_owned());
        }
        let candidates = if segments
            .first()
            .is_some_and(|segment| segment == "rust" || segment == "stainless")
        {
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

    fn reject_bare_interface_type(&mut self, ty: &TypeRef, span: Span, role: &str) {
        if let TypeRef::Interface { path, .. } = ty {
            self.push(
                "RES122",
                format!(
                    "{role} cannot have bare interface type `{}`; use a reference or owning pointer",
                    display_path(path)
                ),
                span,
            );
        }
    }

    fn is_reserved_rust_path(
        &self,
        segments: &[String],
        namespace: &[String],
        expected: &[&str],
    ) -> bool {
        if segments
            .iter()
            .map(String::as_str)
            .eq(expected.iter().copied())
        {
            return true;
        }
        let Some((first, remaining)) = segments.split_first() else {
            return false;
        };
        self.imports
            .candidates(namespace, first)
            .iter()
            .any(|base| {
                base.iter()
                    .map(String::as_str)
                    .chain(remaining.iter().map(String::as_str))
                    .eq(expected.iter().copied())
            })
    }

    fn is_json_compatible(&self, ty: &TypeRef) -> bool {
        self.json_conversion_error(ty).is_none()
    }

    fn json_conversion_error(&self, ty: &TypeRef) -> Option<String> {
        self.json_conversion_error_inner(ty, &mut BTreeSet::new())
    }

    fn json_conversion_error_inner(
        &self,
        ty: &TypeRef,
        visiting: &mut BTreeSet<StructId>,
    ) -> Option<String> {
        let ty = canonical_ref(ty);
        if *ty == TypeRef::Error
            || is_json_var(ty)
            || is_numeric(ty)
            || matches!(ty, TypeRef::Bool | TypeRef::Char)
            || matches!(
                ty,
                TypeRef::Native { path, arguments }
                    if path == "rust::String" && arguments.is_empty()
            )
        {
            return None;
        }
        if let TypeRef::Native { path, arguments } = ty {
            return match (path.as_str(), arguments.as_slice()) {
                (
                    "rust::Vec" | "rust::List" | "rust::Queue" | "rust::Set" | "rust::Option",
                    [element],
                ) => self
                    .json_conversion_error_inner(element, visiting)
                    .map(|reason| format!("collection element {reason}")),
                ("rust::Map", [key, value]) => {
                    if matches!(
                        canonical_ref(key),
                        TypeRef::Native { path, arguments }
                            if path == "rust::String" && arguments.is_empty()
                    ) {
                        self.json_conversion_error_inner(value, visiting)
                            .map(|reason| format!("ordered map value {reason}"))
                    } else {
                        Some(format!(
                            "ordered map keys must be `rust::String`, found `{}`",
                            display_type(key)
                        ))
                    }
                }
                _ => Some(format!(
                    "native type `{}` has no structural JSON representation",
                    display_type(ty)
                )),
            };
        }
        if let TypeRef::Struct { path, arguments } = ty {
            let Some(structure) = self.struct_by_path.get(path).copied() else {
                return Some(format!("struct `{}` is unresolved", display_path(path)));
            };
            if !arguments.is_empty() {
                return Some(format!(
                    "generic struct `{}` automatic JSON conversion is deferred",
                    display_user_type(path, arguments)
                ));
            }
            return self.json_struct_conversion_error(structure, visiting);
        }
        if let TypeRef::Class { path, .. } = ty {
            return Some(format!(
                "`{}` is a class; only data structs have automatic JSON conversion",
                display_path(path)
            ));
        }
        Some(format!(
            "type `{}` has no structural JSON representation",
            display_type(ty)
        ))
    }

    fn json_struct_conversion_error(
        &self,
        structure: StructId,
        visiting: &mut BTreeSet<StructId>,
    ) -> Option<String> {
        if !visiting.insert(structure) {
            return Some(format!(
                "recursive struct `{}` cannot be converted by value",
                display_path(&self.model.structs[structure.0].path)
            ));
        }
        let result = (|| {
            let mut hierarchy = Vec::new();
            let mut current = Some(structure);
            let mut hierarchy_seen = BTreeSet::new();
            while let Some(id) = current {
                if !hierarchy_seen.insert(id) {
                    return Some("data inheritance cycle has no JSON representation".to_owned());
                }
                hierarchy.push(id);
                current = self.model.structs[id.0].base;
            }

            let mut names = BTreeSet::new();
            for id in hierarchy.into_iter().rev() {
                let owner = &self.model.structs[id.0];
                for field in &owner.fields {
                    if !names.insert(field.name.as_str()) {
                        return Some(format!(
                            "inherited field name `{}` is ambiguous in JSON",
                            field.name
                        ));
                    }
                    if let Some(reason) = self.json_conversion_error_inner(&field.ty, visiting) {
                        return Some(format!(
                            "field `{}.{}` has unsupported type `{}`: {reason}",
                            display_path(&owner.path),
                            field.name,
                            display_type(&field.ty)
                        ));
                    }
                }
            }
            None
        })();
        visiting.remove(&structure);
        result
    }

    fn record_json_conversions(&mut self, ty: &TypeRef) {
        let mut structures = BTreeSet::new();
        self.collect_json_conversion_structs(ty, &mut BTreeSet::new(), &mut structures);
        for structure in structures {
            if !self.model.json_struct_conversions.contains(&structure) {
                self.model.json_struct_conversions.push(structure);
            }
        }
    }

    fn collect_json_conversion_structs(
        &self,
        ty: &TypeRef,
        visiting: &mut BTreeSet<StructId>,
        output: &mut BTreeSet<StructId>,
    ) {
        match canonical_ref(ty) {
            TypeRef::Native { path, arguments } => match (path.as_str(), arguments.as_slice()) {
                (
                    "rust::Vec" | "rust::List" | "rust::Queue" | "rust::Set" | "rust::Option",
                    [element],
                ) => self.collect_json_conversion_structs(element, visiting, output),
                ("rust::Map", [_, value]) => {
                    self.collect_json_conversion_structs(value, visiting, output);
                }
                _ => {}
            },
            TypeRef::Struct { path, .. } => {
                let Some(structure) = self.struct_by_path.get(path).copied() else {
                    return;
                };
                if !visiting.insert(structure) {
                    return;
                }
                output.insert(structure);
                let symbol = &self.model.structs[structure.0];
                if let Some(base) = symbol.base {
                    let base = &self.model.structs[base.0];
                    self.collect_json_conversion_structs(
                        &TypeRef::Struct {
                            path: base.path.clone(),
                            arguments: Vec::new(),
                        },
                        visiting,
                        output,
                    );
                }
                for field in &symbol.fields {
                    self.collect_json_conversion_structs(&field.ty, visiting, output);
                }
                visiting.remove(&structure);
            }
            _ => {}
        }
    }

    fn thread_sendable(&self, ty: &TypeRef) -> bool {
        self.thread_auto_trait(ty, false, &mut BTreeSet::new())
    }

    fn is_copyable_type(&self, ty: &TypeRef) -> bool {
        match canonical_ref(ty) {
            TypeRef::Struct { path, arguments } => {
                self.struct_by_path.get(path).is_some_and(|structure| {
                    self.structure_is_cloneable(*structure, arguments, &mut BTreeSet::new())
                })
            }
            concrete => is_copyable(concrete),
        }
    }

    fn structure_is_cloneable(
        &self,
        structure: StructId,
        arguments: &[TypeRef],
        visiting: &mut BTreeSet<(StructId, Vec<TypeRef>)>,
    ) -> bool {
        let key = (structure, arguments.to_vec());
        if !visiting.insert(key.clone()) {
            return true;
        }
        let symbol = &self.model.structs[structure.0];
        if symbol.kind != ast::UserTypeKind::Struct {
            visiting.remove(&key);
            return false;
        }
        let substitutions = symbol
            .type_parameters
            .iter()
            .cloned()
            .zip(arguments.iter().cloned())
            .collect::<BTreeMap<_, _>>();
        let cloneable = symbol
            .base
            .is_none_or(|base| self.structure_is_cloneable(base, &[], visiting))
            && symbol.fields.iter().all(|field| {
                self.type_is_cloneable(&substitute_type(&field.ty, &substitutions), visiting)
            });
        visiting.remove(&key);
        cloneable
    }

    fn type_is_cloneable(
        &self,
        ty: &TypeRef,
        visiting: &mut BTreeSet<(StructId, Vec<TypeRef>)>,
    ) -> bool {
        match canonical_ref(ty) {
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
            | TypeRef::Reference { .. } => true,
            TypeRef::Tuple(elements) => elements
                .iter()
                .all(|element| self.type_is_cloneable(element, visiting)),
            TypeRef::Native { path, arguments } => {
                matches!(
                    path.as_str(),
                    "rust::String"
                        | "rust::Vec"
                        | "rust::List"
                        | "rust::Map"
                        | "rust::MultiMap"
                        | "rust::Queue"
                        | "rust::Set"
                        | "rust::Option"
                        | "rust::Result"
                        | "rust::stainless_runtime::Var"
                ) && arguments
                    .iter()
                    .all(|argument| self.type_is_cloneable(argument, visiting))
            }
            TypeRef::Struct { path, arguments } => {
                self.struct_by_path.get(path).is_some_and(|structure| {
                    self.structure_is_cloneable(*structure, arguments, visiting)
                })
            }
            TypeRef::Pointer { kind, .. } => matches!(
                kind,
                PointerKind::Shared | PointerKind::SharedNullable | PointerKind::Weak
            ),
            TypeRef::Function(function) => function.kind == StoredFunctionKind::Shared,
            TypeRef::Void
            | TypeRef::Parameter(_)
            | TypeRef::Callback(_)
            | TypeRef::Mutex(_)
            | TypeRef::MutexGuard(_)
            | TypeRef::RwLock(_)
            | TypeRef::RwLockReadGuard(_)
            | TypeRef::RwLockWriteGuard(_)
            | TypeRef::Condition
            | TypeRef::ThreadHandle(_)
            | TypeRef::ThreadScope
            | TypeRef::ScopedThreadHandle(_)
            | TypeRef::Class { .. }
            | TypeRef::Interface { .. }
            | TypeRef::Error => false,
        }
    }

    fn thread_sync(&self, ty: &TypeRef) -> bool {
        self.thread_auto_trait(ty, true, &mut BTreeSet::new())
    }

    #[allow(clippy::too_many_lines)]
    fn thread_auto_trait(
        &self,
        ty: &TypeRef,
        sync: bool,
        visiting: &mut BTreeSet<(bool, TypeRef)>,
    ) -> bool {
        let ty = canonical_ref(ty);
        let key = (sync, ty.clone());
        if !visiting.insert(key.clone()) {
            return true;
        }
        let result = match ty {
            TypeRef::Void
            | TypeRef::Bool
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
            | TypeRef::Condition
            | TypeRef::Interface { .. } => true,
            TypeRef::Tuple(elements) => elements
                .iter()
                .all(|element| self.thread_auto_trait(element, sync, visiting)),
            TypeRef::Struct { path, arguments } | TypeRef::Class { path, arguments } => {
                self.struct_by_path.get(path).is_some_and(|id| {
                    let structure = &self.model.structs[id.0];
                    let substitutions = structure
                        .type_parameters
                        .iter()
                        .cloned()
                        .zip(arguments.iter().cloned())
                        .collect::<BTreeMap<_, _>>();
                    structure.base.is_none_or(|base| {
                        self.thread_auto_trait(
                            &resolved_structure_type(&self.model.structs[base.0]),
                            sync,
                            visiting,
                        )
                    }) && structure.fields.iter().all(|field| {
                        self.thread_auto_trait(
                            &substitute_type(&field.ty, &substitutions),
                            sync,
                            visiting,
                        )
                    })
                })
            }
            TypeRef::Native { path, arguments } => {
                matches!(
                    path.as_str(),
                    "rust::String"
                        | "rust::Vec"
                        | "rust::List"
                        | "rust::Map"
                        | "rust::MultiMap"
                        | "rust::Queue"
                        | "rust::Set"
                        | "rust::Option"
                        | "rust::Result"
                        | "rust::std::fs::File"
                        | "rust::std::fs::OpenOptions"
                        | "rust::stainless_runtime::Var"
                ) && arguments
                    .iter()
                    .all(|argument| self.thread_auto_trait(argument, sync, visiting))
            }
            TypeRef::Pointer { kind, target } => match kind {
                PointerKind::Unique | PointerKind::UniqueNullable => {
                    self.thread_auto_trait(target, sync, visiting)
                }
                PointerKind::Shared
                | PointerKind::SharedNullable
                | PointerKind::Weak
                | PointerKind::Atomic
                | PointerKind::AtomicNullable => {
                    self.thread_auto_trait(target, false, visiting)
                        && self.thread_auto_trait(target, true, visiting)
                }
            },
            TypeRef::Mutex(target) => self.thread_auto_trait(target, false, visiting),
            TypeRef::RwLock(target) => {
                self.thread_auto_trait(target, false, visiting)
                    && (!sync || self.thread_auto_trait(target, true, visiting))
            }
            TypeRef::ThreadHandle(target) => self.thread_auto_trait(target, sync, visiting),
            TypeRef::Reference { .. }
            | TypeRef::MutexGuard(_)
            | TypeRef::RwLockReadGuard(_)
            | TypeRef::RwLockWriteGuard(_)
            | TypeRef::ThreadScope
            | TypeRef::ScopedThreadHandle(_)
            | TypeRef::Callback(_)
            | TypeRef::Function(_)
            | TypeRef::Parameter(_)
            | TypeRef::Error => false,
        };
        visiting.remove(&key);
        result
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
        arguments: &[TypeRef],
        requested: &ast::Path,
    ) -> Option<StructFieldLookup> {
        let substitutions = self.model.structs[structure.0]
            .type_parameters
            .iter()
            .cloned()
            .zip(arguments.iter().cloned())
            .collect::<BTreeMap<_, _>>();
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
                    matches.push(StructFieldLookup {
                        ty: substitute_type(&field.ty, &substitutions),
                        access_path: field_path,
                        owner: id,
                        is_public: field.is_public,
                    });
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
                return Some(StructFieldLookup {
                    ty: substitute_type(&field.ty, &substitutions),
                    access_path,
                    owner: id,
                    is_public: field.is_public,
                });
            }
            let base = symbol.base?;
            access_path.push(base_field_name(&self.model.structs[base.0]));
            current = Some(base);
        }
        None
    }

    fn member_is_accessible(owner: StructId, is_public: bool, context: &FunctionContext) -> bool {
        is_public
            || context
                .receiver
                .as_ref()
                .is_some_and(|receiver| receiver.structure == owner)
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

    fn call_is_async(&self, call: &ResolvedCall) -> bool {
        match &call.target {
            CallTarget::Stainless(id) | CallTarget::InterfaceMethod(id) => self
                .model
                .functions
                .get(id.0)
                .is_some_and(|function| function.is_async),
            CallTarget::Native(native) => native.is_async,
            CallTarget::Constructor(_) | CallTarget::Intrinsic(_) => false,
        }
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
            | TypeRef::Tuple(_)
            | TypeRef::Callback(_)
            | TypeRef::Function(_)
            | TypeRef::Pointer { .. }
            | TypeRef::Mutex(_)
            | TypeRef::MutexGuard(_)
            | TypeRef::RwLock(_)
            | TypeRef::RwLockReadGuard(_)
            | TypeRef::RwLockWriteGuard(_)
            | TypeRef::Condition
            | TypeRef::ThreadHandle(_)
            | TypeRef::ThreadScope
            | TypeRef::ScopedThreadHandle(_)
            | TypeRef::Struct { .. }
            | TypeRef::Class { .. }
            | TypeRef::Interface { .. }
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

    fn warn(&mut self, code: &'static str, message: String, span: Span) {
        self.diagnostics
            .push(Diagnostic::semantic_warning(code, message, span));
    }
}

fn local_return_move_source(expression: &Expression, context: &FunctionContext) -> bool {
    match &expression.kind {
        ExpressionKind::Name(path) if path.segments.len() == 1 => context
            .scopes
            .iter()
            .rev()
            .any(|scope| scope.contains_key(&path.segments[0])),
        ExpressionKind::Parenthesized(inner) => local_return_move_source(inner, context),
        _ => false,
    }
}

fn move_call_argument(expression: &Expression) -> Option<&Expression> {
    match &expression.kind {
        ExpressionKind::Parenthesized(inner) => move_call_argument(inner),
        ExpressionKind::Call { callee, arguments }
            if arguments.len() == 1
                && matches!(
                    &callee.kind,
                    ExpressionKind::Name(path)
                        if matches!(path.segments.as_slice(), [name] if name == "move")
                ) =>
        {
            arguments.first()
        }
        _ => None,
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
        rust_result_error: callable
            .rust_result_error
            .as_ref()
            .map(|error| substitute(error, &substitutions)),
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
        TypeRef::Tuple(elements) => TypeRef::Tuple(
            elements
                .iter()
                .map(|element| substitute(element, substitutions))
                .collect(),
        ),
        TypeRef::Callback(callback) => {
            let parameters = callback
                .parameters
                .iter()
                .map(|parameter| substitute(parameter, substitutions))
                .collect();
            let return_type = substitute(&callback.return_type, substitutions);
            if callback.is_async {
                TypeRef::async_callback(callback.kind, callback.escape, parameters, return_type)
            } else {
                TypeRef::callback(callback.kind, callback.escape, parameters, return_type)
            }
        }
        TypeRef::Function(function) => TypeRef::function(
            function.kind,
            function
                .parameters
                .iter()
                .map(|parameter| substitute(parameter, substitutions))
                .collect(),
            substitute(&function.return_type, substitutions),
        ),
        TypeRef::Pointer { kind, target } => {
            TypeRef::pointer(*kind, substitute(target, substitutions))
        }
        TypeRef::Mutex(target) => TypeRef::Mutex(Box::new(substitute(target, substitutions))),
        TypeRef::MutexGuard(target) => {
            TypeRef::MutexGuard(Box::new(substitute(target, substitutions)))
        }
        TypeRef::RwLock(target) => TypeRef::RwLock(Box::new(substitute(target, substitutions))),
        TypeRef::RwLockReadGuard(target) => {
            TypeRef::RwLockReadGuard(Box::new(substitute(target, substitutions)))
        }
        TypeRef::RwLockWriteGuard(target) => {
            TypeRef::RwLockWriteGuard(Box::new(substitute(target, substitutions)))
        }
        TypeRef::ThreadHandle(target) => {
            TypeRef::ThreadHandle(Box::new(substitute(target, substitutions)))
        }
        TypeRef::ScopedThreadHandle(target) => {
            TypeRef::ScopedThreadHandle(Box::new(substitute(target, substitutions)))
        }
        TypeRef::Struct { path, arguments } => TypeRef::Struct {
            path: path.clone(),
            arguments: arguments
                .iter()
                .map(|argument| substitute(argument, substitutions))
                .collect(),
        },
        TypeRef::Class { path, arguments } => TypeRef::Class {
            path: path.clone(),
            arguments: arguments
                .iter()
                .map(|argument| substitute(argument, substitutions))
                .collect(),
        },
        TypeRef::Interface { path, arguments } => TypeRef::Interface {
            path: path.clone(),
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

fn substitute_type(ty: &TypeRef, substitutions: &BTreeMap<String, TypeRef>) -> TypeRef {
    substitute(ty, substitutions)
}

fn user_type(structure: &StructSymbol, arguments: Vec<TypeRef>) -> TypeRef {
    match structure.kind {
        ast::UserTypeKind::Struct => TypeRef::Struct {
            path: structure.path.clone(),
            arguments,
        },
        ast::UserTypeKind::Class => TypeRef::Class {
            path: structure.path.clone(),
            arguments,
        },
        ast::UserTypeKind::Interface => TypeRef::Interface {
            path: structure.path.clone(),
            arguments,
        },
    }
}

fn user_type_substitutions(
    structure: &StructSymbol,
    instance: &TypeRef,
) -> BTreeMap<String, TypeRef> {
    structure
        .type_parameters
        .iter()
        .cloned()
        .zip(
            instance
                .user_arguments()
                .unwrap_or_default()
                .iter()
                .cloned(),
        )
        .collect()
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

fn automatic_pointee(ty: &TypeRef) -> &TypeRef {
    match canonical_ref(ty) {
        TypeRef::Pointer {
            kind: PointerKind::Unique | PointerKind::Shared,
            target,
        }
        | TypeRef::MutexGuard(target)
        | TypeRef::RwLockReadGuard(target)
        | TypeRef::RwLockWriteGuard(target) => canonical_ref(target),
        target => target,
    }
}

fn is_shared_owner(ty: &TypeRef) -> bool {
    matches!(
        canonical_ref(ty),
        TypeRef::Pointer {
            kind: PointerKind::Shared | PointerKind::SharedNullable,
            ..
        }
    )
}

fn pointee_category(ty: &TypeRef, category: ValueCategory) -> ValueCategory {
    if is_shared_owner(ty) || matches!(canonical_ref(ty), TypeRef::RwLockReadGuard(_)) {
        ValueCategory::SharedPlace
    } else {
        category
    }
}

fn initial_null_state(ty: &TypeRef) -> NullState {
    match canonical_ref(ty) {
        TypeRef::Pointer {
            kind: PointerKind::Unique | PointerKind::Shared | PointerKind::Atomic,
            ..
        } => NullState::NonNull,
        _ => NullState::Unknown,
    }
}

fn default_constructed_null_state(ty: &TypeRef) -> NullState {
    match canonical_ref(ty) {
        TypeRef::Pointer {
            kind: PointerKind::UniqueNullable | PointerKind::SharedNullable | PointerKind::Weak,
            ..
        } => NullState::Null,
        _ => initial_null_state(ty),
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

fn pointer_kind(segments: &[String]) -> Option<PointerKind> {
    if segments.len() != 1 {
        return None;
    }
    match segments[0].as_str() {
        "unique_ptr" => Some(PointerKind::Unique),
        "unique_nullptr" => Some(PointerKind::UniqueNullable),
        "shared_ptr" => Some(PointerKind::Shared),
        "shared_nullptr" => Some(PointerKind::SharedNullable),
        "weak_ptr" => Some(PointerKind::Weak),
        "atomic_ptr" => Some(PointerKind::Atomic),
        "atomic_nullptr" => Some(PointerKind::AtomicNullable),
        _ => None,
    }
}

fn literal_type(kind: LiteralKind, text: &str, expected: Option<&TypeRef>) -> TypeRef {
    match kind {
        LiteralKind::Null if text == "nullptr" => expected
            .filter(|ty| is_nullable_pointer_test(ty))
            .cloned()
            .unwrap_or(TypeRef::Error),
        LiteralKind::Null => json_var_type(),
        LiteralKind::Boolean => TypeRef::Bool,
        LiteralKind::Character => TypeRef::Char,
        LiteralKind::String => TypeRef::native("rust::String", vec![]),
        LiteralKind::Float if text.ends_with('f') => TypeRef::F32,
        LiteralKind::Float => TypeRef::F64,
        LiteralKind::Integer => integer_suffix(text)
            .or_else(|| expected.filter(|ty| is_integer(ty)).cloned())
            .unwrap_or_else(|| default_positive_integer_type(text)),
    }
}

fn unsuffixed_integer_literal_text(expression: &Expression) -> Option<&str> {
    integer_literal_text(expression).filter(|text| integer_suffix(text).is_none())
}

fn integer_literal_text(expression: &Expression) -> Option<&str> {
    match &expression.kind {
        ExpressionKind::Literal(ast::Literal {
            kind: LiteralKind::Integer,
            text,
            ..
        }) => Some(text),
        ExpressionKind::Parenthesized(inner) => integer_literal_text(inner),
        _ => None,
    }
}

fn is_unsuffixed_integer_literal(expression: &Expression) -> bool {
    unsuffixed_integer_literal_text(expression).is_some()
}

fn default_positive_integer_type(text: &str) -> TypeRef {
    if integer_magnitude(text).is_none_or(|value| value > u128::from(u32::MAX)) {
        TypeRef::U64
    } else {
        TypeRef::U32
    }
}

fn default_negative_integer_type(text: &str) -> TypeRef {
    if integer_magnitude(text).is_some_and(|value| value > (1_u128 << 31)) {
        TypeRef::I64
    } else {
        TypeRef::I32
    }
}

fn integer_magnitude(text: &str) -> Option<u128> {
    let number = integer_suffix_text(text)
        .and_then(|suffix| text.strip_suffix(suffix))
        .unwrap_or(text);
    let normalized = number.replace('_', "");
    let (digits, radix) = normalized
        .strip_prefix("0x")
        .map(|digits| (digits, 16))
        .or_else(|| normalized.strip_prefix("0o").map(|digits| (digits, 8)))
        .or_else(|| normalized.strip_prefix("0b").map(|digits| (digits, 2)))
        .unwrap_or((normalized.as_str(), 10));
    u128::from_str_radix(digits, radix).ok()
}

fn integer_suffix_text(text: &str) -> Option<&'static str> {
    [
        "i128", "isize", "u128", "usize", "i64", "u64", "i32", "u32", "i16", "u16", "i8", "u8",
    ]
    .into_iter()
    .find(|suffix| text.ends_with(suffix))
}

fn integer_literal_fits(magnitude: u128, ty: &TypeRef, negated: bool) -> bool {
    let (signed, bits) = match canonical_ref(ty) {
        TypeRef::I8 => (true, 8),
        TypeRef::I16 => (true, 16),
        TypeRef::I32 => (true, 32),
        TypeRef::I64 => (true, 64),
        TypeRef::I128 => (true, 128),
        TypeRef::Isize => (true, isize::BITS),
        TypeRef::U8 => (false, 8),
        TypeRef::U16 => (false, 16),
        TypeRef::U32 => (false, 32),
        TypeRef::U64 => (false, 64),
        TypeRef::U128 => (false, 128),
        TypeRef::Usize => (false, usize::BITS),
        _ => return true,
    };
    if negated {
        return signed && magnitude <= 1_u128 << (bits - 1);
    }
    let value_bits = bits - u32::from(signed);
    magnitude <= u128::MAX >> (u128::BITS - value_bits)
}

fn is_fixed_width_integer(ty: &TypeRef) -> bool {
    is_integer(ty) && !matches!(ty, TypeRef::Usize | TypeRef::Isize)
}

fn is_null_literal(expression: &Expression) -> bool {
    match &expression.kind {
        ExpressionKind::Literal(ast::Literal {
            kind: LiteralKind::Null,
            ..
        }) => true,
        ExpressionKind::Parenthesized(inner) => is_null_literal(inner),
        _ => false,
    }
}

fn is_nullable_pointer_test(ty: &TypeRef) -> bool {
    matches!(
        canonical_ref(ty),
        TypeRef::Pointer {
            kind: PointerKind::UniqueNullable | PointerKind::SharedNullable | PointerKind::Weak,
            ..
        }
    )
}

fn set_expression_null_state(
    expression: &Expression,
    null_state: NullState,
    context: &mut FunctionContext,
) {
    let expression = match &expression.kind {
        ExpressionKind::Parenthesized(inner) => inner.as_ref(),
        _ => expression,
    };
    let ExpressionKind::Name(path) = &expression.kind else {
        return;
    };
    let [name] = path.segments.as_slice() else {
        return;
    };
    if let Some(variable) = context
        .scopes
        .iter_mut()
        .rev()
        .find_map(|scope| scope.get_mut(name))
        && is_nullable_pointer_test(&variable.ty)
    {
        variable.null_state = null_state;
    }
}

fn merge_null_scopes(
    left: &[BTreeMap<String, Variable>],
    left_continues: bool,
    right: &[BTreeMap<String, Variable>],
    right_continues: bool,
) -> Vec<BTreeMap<String, Variable>> {
    if !left_continues {
        return right.to_vec();
    }
    if !right_continues {
        return left.to_vec();
    }
    left.iter()
        .zip(right)
        .map(|(left, right)| {
            left.iter()
                .map(|(name, variable)| {
                    let mut merged = variable.clone();
                    if right
                        .get(name)
                        .is_none_or(|other| other.null_state != variable.null_state)
                    {
                        merged.null_state = NullState::Unknown;
                    }
                    (name.clone(), merged)
                })
                .collect()
        })
        .collect()
}

fn unknown_nullable_scopes(
    scopes: &[BTreeMap<String, Variable>],
) -> Vec<BTreeMap<String, Variable>> {
    let mut scopes = scopes.to_vec();
    for variable in scopes.iter_mut().flat_map(BTreeMap::values_mut) {
        if is_nullable_pointer_test(&variable.ty) {
            variable.null_state = NullState::Unknown;
        }
    }
    scopes
}

fn block_may_fall_through(block: &ast::Block) -> bool {
    block
        .statements
        .last()
        .is_none_or(statement_may_fall_through)
}

fn statement_may_fall_through(statement: &Statement) -> bool {
    match &statement.kind {
        StatementKind::Return(_)
        | StatementKind::Throw(_)
        | StatementKind::Break
        | StatementKind::Continue => false,
        StatementKind::Block(block) => block
            .statements
            .last()
            .is_none_or(statement_may_fall_through),
        StatementKind::If(statement) => {
            statement_may_fall_through(&statement.then_branch)
                || statement
                    .else_branch
                    .as_deref()
                    .is_none_or(statement_may_fall_through)
        }
        _ => true,
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
    let path = if matches!(segments.first().map(String::as_str), Some("stainless")) {
        format!("rust::stainless_runtime::{}", segments[1..].join("::"))
    } else {
        segments.join("::")
    };
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

fn json_var_type() -> TypeRef {
    TypeRef::native(VAR_TYPE_PATH, Vec::new())
}

fn is_json_var(ty: &TypeRef) -> bool {
    matches!(
        ty,
        TypeRef::Native { path, arguments }
            if path == VAR_TYPE_PATH && arguments.is_empty()
    )
}

fn is_json_mutation_place(expression: &Expression, model: &SemanticModel) -> bool {
    match &expression.kind {
        ExpressionKind::Field { receiver, .. } => {
            model
                .expression(expression.span)
                .is_some_and(|resolution| resolution.field.is_none())
                && model
                    .expression(receiver.span)
                    .is_some_and(|resolution| is_json_var(canonical_ref(&resolution.ty)))
        }
        ExpressionKind::Index { receiver, .. } => model
            .expression(receiver.span)
            .is_some_and(|resolution| is_json_var(canonical_ref(&resolution.ty))),
        _ => false,
    }
}

fn is_format_value(ty: &TypeRef) -> bool {
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
    ) || is_json_var(ty)
        || matches!(
            ty,
            TypeRef::Native { path, arguments }
                if path == "rust::String" && arguments.is_empty()
        )
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
    ) || matches!(ty, TypeRef::Tuple(elements) if elements.iter().all(is_copyable))
        || is_json_var(ty)
        || matches!(
            ty,
            TypeRef::Pointer {
                kind: PointerKind::Shared | PointerKind::SharedNullable | PointerKind::Weak,
                ..
            }
        )
        || matches!(
            ty,
            TypeRef::Function(function) if function.kind == StoredFunctionKind::Shared
        )
}

fn supports_equality(ty: &TypeRef) -> bool {
    if is_numeric(ty) || matches!(ty, TypeRef::Bool | TypeRef::Char) {
        return true;
    }
    match ty {
        TypeRef::Tuple(elements) => elements.iter().all(supports_equality),
        TypeRef::Native { path, arguments } => match (path.as_str(), arguments.as_slice()) {
            ("rust::String", []) => true,
            ("rust::Vec" | "rust::List" | "rust::Queue" | "rust::Option", [element]) => {
                supports_equality(element)
            }
            ("rust::Map" | "rust::MultiMap", [key, value]) => {
                supports_equality(key) && supports_equality(value)
            }
            ("rust::Set", [element]) => supports_equality(element),
            ("rust::Result", [value, error]) => {
                supports_equality(value) && supports_equality(error)
            }
            _ => false,
        },
        _ => false,
    }
}

fn supports_ordering(ty: &TypeRef) -> bool {
    if is_numeric(ty) || matches!(ty, TypeRef::Bool | TypeRef::Char) {
        return true;
    }
    match ty {
        TypeRef::Tuple(elements) => elements.iter().all(supports_ordering),
        TypeRef::Native { path, arguments } => match (path.as_str(), arguments.as_slice()) {
            ("rust::String", []) => true,
            ("rust::Vec" | "rust::List" | "rust::Queue" | "rust::Option", [element]) => {
                supports_ordering(element)
            }
            ("rust::Map", [key, value]) => supports_ordering(key) && supports_ordering(value),
            ("rust::Set", [element]) => supports_ordering(element),
            ("rust::Result", [value, error]) => {
                supports_ordering(value) && supports_ordering(error)
            }
            _ => false,
        },
        _ => false,
    }
}

fn callable_signature(ty: &TypeRef) -> Option<(&[TypeRef], &TypeRef)> {
    match ty {
        TypeRef::Callback(callback) => Some((&callback.parameters, &callback.return_type)),
        TypeRef::Function(function) => Some((&function.parameters, &function.return_type)),
        _ => None,
    }
}

fn awaited_call_span(expression: &Expression) -> Option<Span> {
    match &expression.kind {
        ExpressionKind::Call { .. } => Some(expression.span),
        ExpressionKind::Parenthesized(inner) => awaited_call_span(inner),
        _ => None,
    }
}

fn contains_move_only_storage(ty: &TypeRef) -> bool {
    match ty {
        TypeRef::Tuple(elements) => elements.iter().any(contains_move_only_storage),
        TypeRef::Mutex(_)
        | TypeRef::MutexGuard(_)
        | TypeRef::RwLock(_)
        | TypeRef::RwLockReadGuard(_)
        | TypeRef::RwLockWriteGuard(_)
        | TypeRef::Condition
        | TypeRef::ThreadHandle(_)
        | TypeRef::ThreadScope
        | TypeRef::ScopedThreadHandle(_)
        | TypeRef::Class { .. }
        | TypeRef::Interface { .. } => true,
        TypeRef::Function(function) => function.kind == StoredFunctionKind::Mutable,
        TypeRef::Pointer { kind, .. } => matches!(
            kind,
            PointerKind::Unique
                | PointerKind::UniqueNullable
                | PointerKind::Atomic
                | PointerKind::AtomicNullable
        ),
        TypeRef::Native { path, arguments } => {
            matches!(
                path.as_str(),
                "rust::std::fs::File" | "rust::std::fs::OpenOptions"
            ) || arguments.iter().any(contains_move_only_storage)
        }
        TypeRef::Reference { target, .. } => contains_move_only_storage(target),
        _ => false,
    }
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
        TypeRef::Tuple(elements) => format!(
            "tuple<{}>",
            elements
                .iter()
                .map(display_type)
                .collect::<Vec<_>>()
                .join(", ")
        ),
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
        TypeRef::Function(function) => format!(
            "{}<{}({})>",
            if function.kind == StoredFunctionKind::Shared {
                "function"
            } else {
                "function_mut"
            },
            display_type(&function.return_type),
            function
                .parameters
                .iter()
                .map(display_type)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        TypeRef::Pointer { kind, target } => {
            format!("{}<{}>", pointer_name(*kind), display_type(target))
        }
        TypeRef::Mutex(target) => format!("mutex<{}>", display_type(target)),
        TypeRef::MutexGuard(target) => format!("<mutex_guard<{}>>", display_type(target)),
        TypeRef::RwLock(target) => format!("rwlock<{}>", display_type(target)),
        TypeRef::RwLockReadGuard(target) => {
            format!("<rwlock_read_guard<{}>>", display_type(target))
        }
        TypeRef::RwLockWriteGuard(target) => {
            format!("<rwlock_write_guard<{}>>", display_type(target))
        }
        TypeRef::Condition => "condition".to_owned(),
        TypeRef::ThreadHandle(target) => {
            format!("rust::std::thread::JoinHandle<{}>", display_type(target))
        }
        TypeRef::ThreadScope => "rust::std::thread::Scope".to_owned(),
        TypeRef::ScopedThreadHandle(target) => format!(
            "rust::std::thread::ScopedJoinHandle<{}>",
            display_type(target)
        ),
        TypeRef::Struct { path, arguments }
        | TypeRef::Class { path, arguments }
        | TypeRef::Interface { path, arguments } => display_user_type(path, arguments),
        TypeRef::Reference { mutable, target } => {
            if *mutable {
                format!("{}&", display_type(target))
            } else {
                format!("const {}&", display_type(target))
            }
        }
    }
}

fn resolved_structure_type(structure: &StructSymbol) -> TypeRef {
    let arguments = structure
        .type_parameters
        .iter()
        .cloned()
        .map(TypeRef::Parameter)
        .collect();
    match structure.kind {
        ast::UserTypeKind::Struct => TypeRef::Struct {
            path: structure.path.clone(),
            arguments,
        },
        ast::UserTypeKind::Class => TypeRef::Class {
            path: structure.path.clone(),
            arguments,
        },
        ast::UserTypeKind::Interface => TypeRef::Interface {
            path: structure.path.clone(),
            arguments,
        },
    }
}

fn pointer_name(kind: PointerKind) -> &'static str {
    match kind {
        PointerKind::Unique => "unique_ptr",
        PointerKind::UniqueNullable => "unique_nullptr",
        PointerKind::Shared => "shared_ptr",
        PointerKind::SharedNullable => "shared_nullptr",
        PointerKind::Weak => "weak_ptr",
        PointerKind::Atomic => "atomic_ptr",
        PointerKind::AtomicNullable => "atomic_nullptr",
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

fn display_user_type(path: &[String], arguments: &[TypeRef]) -> String {
    let path = display_path(path);
    if arguments.is_empty() {
        path
    } else {
        format!(
            "{path}<{}>",
            arguments
                .iter()
                .map(display_type)
                .collect::<Vec<_>>()
                .join(", ")
        )
    }
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
            ExpressionKind::Name(_)
            | ExpressionKind::GenericName { .. }
            | ExpressionKind::Literal(_)
            | ExpressionKind::Error => false,
            ExpressionKind::Parenthesized(inner)
            | ExpressionKind::Await(inner)
            | ExpressionKind::Prefix { operand: inner, .. }
            | ExpressionKind::Postfix { operand: inner, .. } => expression_uses_exceptions(inner),
            ExpressionKind::Binary { left, right, .. } => {
                expression_uses_exceptions(left) || expression_uses_exceptions(right)
            }
            ExpressionKind::Call { callee, arguments } => {
                expression_uses_exceptions(callee)
                    || arguments.iter().any(expression_uses_exceptions)
            }
            ExpressionKind::MacroCall { callee, arguments } => {
                callee
                    .segments
                    .last()
                    .is_some_and(|name| matches!(name.as_str(), "write" | "writeln"))
                    || arguments.iter().any(expression_uses_exceptions)
            }
            ExpressionKind::Aggregate { initializers, .. } => {
                initializers.iter().any(expression_uses_exceptions)
            }
            ExpressionKind::JsonArray { elements } => {
                elements.iter().any(expression_uses_exceptions)
            }
            ExpressionKind::JsonObject { members } => members
                .iter()
                .any(|(_, value)| expression_uses_exceptions(value)),
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
                structure.bases.iter().any(|base| {
                    matches!(
                        &base.kind,
                        TypeKind::Named(base)
                            if matches!(
                                base.path.segments.as_slice(),
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
