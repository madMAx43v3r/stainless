use stainless_syntax::ast::{
    AstNode, ClassicForInitializer, Expression, ForClause, Item, Statement,
};
use stainless_syntax::{SyntaxKind, parse};

#[test]
fn parses_the_basics_reference_file_losslessly() {
    let source = include_str!("../../../docs/ref/01_basics.stl");
    let parsed = parse(source);

    assert!(parsed.errors().is_empty(), "{:?}", parsed.errors());
    assert_eq!(parsed.syntax().to_string(), source);
}

#[test]
fn parses_the_range_for_reference_file_losslessly() {
    let source = include_str!("../../../docs/ref/13_range_for.stl");
    let parsed = parse(source);

    assert!(parsed.errors().is_empty(), "{:?}", parsed.errors());
    assert_eq!(parsed.syntax().to_string(), source);
    assert_eq!(count_kind(&parsed.syntax(), SyntaxKind::RangeForClause), 4);
}

#[test]
fn parses_structs_members_inheritance_and_aggregates_losslessly() {
    let source = include_str!("../../../docs/ref/02_structs_and_data_inheritance.stl");
    let parsed = parse(source);

    assert!(parsed.errors().is_empty(), "{:?}", parsed.errors());
    assert_eq!(parsed.syntax().to_string(), source);
    assert_eq!(
        count_kind(&parsed.syntax(), SyntaxKind::StructDefinition),
        2
    );
    assert_eq!(
        count_kind(&parsed.syntax(), SyntaxKind::FieldDeclaration),
        3
    );
    assert_eq!(
        count_kind(&parsed.syntax(), SyntaxKind::AggregateExpression),
        2
    );
}

#[test]
fn parses_contextual_static_struct_constants_losslessly() {
    let source = r"struct RecordKind {
    static const u8 Insert = 0;
    static const u8 Commit = 1;
};

u8 kind() {
    return RecordKind::Commit;
}
";
    let parsed = parse(source);
    let tree = parsed.tree();
    let constants = tree
        .items()
        .find_map(|item| match item {
            Item::Struct(structure) => Some(structure.fields().collect::<Vec<_>>()),
            _ => None,
        })
        .expect("record kind struct");

    assert!(parsed.errors().is_empty(), "{:?}", parsed.errors());
    assert_eq!(parsed.syntax().to_string(), source);
    assert_eq!(constants.len(), 2);
    assert!(
        constants
            .iter()
            .all(stainless_syntax::ast::FieldDeclaration::is_static)
    );
    assert_eq!(
        constants[0].name_token().expect("constant name").text(),
        "Insert"
    );
    assert!(constants[0].initializer().is_some());
}

#[test]
fn parses_classes_interfaces_inheritance_and_access_labels_losslessly() {
    let source = include_str!("../../../docs/ref/03_interfaces.stl");
    let parsed = parse(source);
    let root = parsed.syntax();

    assert!(parsed.errors().is_empty(), "{:?}", parsed.errors());
    assert_eq!(root.to_string(), source);
    assert_eq!(count_kind(&root, SyntaxKind::InterfaceDefinition), 3);
    assert_eq!(count_kind(&root, SyntaxKind::ClassDefinition), 1);
    assert_eq!(count_kind(&root, SyntaxKind::AccessSpecifier), 1);
}

#[test]
fn parses_braced_owner_allocation_with_nested_generic_targets() {
    let source = "void allocate() {\n    auto shared = make_shared<mutex<State>>{false, 0};\n    auto unique = make_unique<State>{true, 1};\n}\n";
    let parsed = parse(source);

    assert!(parsed.errors().is_empty(), "{:?}", parsed.errors());
    assert_eq!(parsed.syntax().to_string(), source);
    assert_eq!(
        count_kind(&parsed.syntax(), SyntaxKind::AggregateExpression),
        2
    );
}

#[test]
fn parses_constructor_declarations_definitions_deletions_and_initializers() {
    let source = r"struct Base {
    Base(i32 value);
    Base() = delete;
    i32 value;
};

Base::Base(i32 value) : value(value) {
}

struct Derived : Base {
    Derived(i32 value);
};

Derived::Derived(i32 value) : Base(value) {
}
";
    let parsed = parse(source);
    let root = parsed.syntax();

    assert!(parsed.errors().is_empty(), "{:?}", parsed.errors());
    assert_eq!(root.to_string(), source);
    assert_eq!(count_kind(&root, SyntaxKind::ConstructorDeclaration), 3);
    assert_eq!(count_kind(&root, SyntaxKind::ConstructorDefinition), 2);
    assert_eq!(count_kind(&root, SyntaxKind::ConstructorInitializerList), 2);
    assert_eq!(count_kind(&root, SyntaxKind::ConstructorInitializer), 2);
}

#[test]
fn parses_generic_structs_classes_and_qualified_definitions() {
    let source = r"struct Box<T> {
    Box(T value);
    const T& get() const;
    T value;
};

Box<T>::Box(T value) : value(move(value)) {
}

const T& Box<T>::get() const {
    return value;
}

class Holder<T, U> {
public:
    Holder(T first, U second);
private:
    T first;
    U second;
};
";
    let parsed = parse(source);

    assert!(parsed.errors().is_empty(), "{:?}", parsed.errors());
    assert_eq!(parsed.syntax().to_string(), source);
    assert_eq!(
        count_kind(&parsed.syntax(), SyntaxKind::GenericParameterList),
        2
    );
    assert_eq!(
        count_kind(&parsed.syntax(), SyntaxKind::GenericArgumentList),
        2
    );
}

#[test]
fn deleted_constructor_requires_the_contextual_delete_spelling() {
    let parsed = parse("struct Value { Value() = unavailable; };\n");

    assert!(
        parsed
            .errors()
            .iter()
            .any(|error| error.message.contains("expected `delete`"))
    );
}

#[test]
fn parses_throw_try_and_ordered_catches_losslessly() {
    let source = r"Config load() throws IoError {
    try {
        throw IoError{};
    } catch (const IoError& error) {
        throw;
    } catch (...) {
        return Config{};
    }
}
";
    let parsed = parse(source);
    let root = parsed.syntax();

    assert!(parsed.errors().is_empty(), "{:?}", parsed.errors());
    assert_eq!(root.to_string(), source);
    assert_eq!(count_kind(&root, SyntaxKind::TryStatement), 1);
    assert_eq!(count_kind(&root, SyntaxKind::CatchClause), 2);
    assert_eq!(count_kind(&root, SyntaxKind::ThrowStatement), 2);
}

#[test]
fn parses_initial_functions_control_flow_and_both_for_forms_losslessly() {
    let source = r"use rust::Vec;

namespace samples {

usize total(const Vec<i32>& values) {
    usize result = 0;

    for (const auto& value : values) {
        result += value;
    }

    for (i32 index = 0; index < 2; index += 1) {
        result += index;
    }

    if (result > 10) {
        return result;
    } else {
        return 10;
    }
}

} // namespace samples
";
    let parsed = parse(source);
    let root = parsed.syntax();

    assert!(parsed.errors().is_empty(), "{:?}", parsed.errors());
    assert_eq!(root.to_string(), source);
    assert_eq!(count_kind(&root, SyntaxKind::FunctionDefinition), 1);
    assert_eq!(count_kind(&root, SyntaxKind::RangeForClause), 1);
    assert_eq!(count_kind(&root, SyntaxKind::ClassicForClause), 1);
    assert_eq!(count_kind(&root, SyntaxKind::IfStatement), 1);
}

#[test]
fn pratt_parser_preserves_operator_precedence() {
    let source = "i32 calculate() { return 1 + 2 * 3; }\n";
    let parsed = parse(source);
    let root = parsed.syntax();
    let binary_expressions = root
        .descendants()
        .filter(|node| node.kind() == SyntaxKind::BinaryExpression)
        .map(|node| node.to_string())
        .collect::<Vec<_>>();

    assert!(parsed.errors().is_empty(), "{:?}", parsed.errors());
    assert_eq!(root.to_string(), source);
    assert_eq!(binary_expressions, ["1 + 2 * 3", "2 * 3"]);
}

#[test]
fn parses_generic_call_targets_without_consuming_comparisons() {
    let source = r"struct Config { i32 value; };

bool compare(i32 left, i32 right) {
    unique_ptr<Config> owner = make_unique<Config>(Config{left});
    return left < right;
}
";
    let parsed = parse(source);
    let root = parsed.syntax();

    assert!(parsed.errors().is_empty(), "{:?}", parsed.errors());
    assert_eq!(root.to_string(), source);
    assert_eq!(count_kind(&root, SyntaxKind::GenericArgumentList), 2);
    assert_eq!(count_kind(&root, SyntaxKind::CallExpression), 1);
    assert_eq!(count_kind(&root, SyntaxKind::BinaryExpression), 1);
}

#[test]
fn parses_explicit_capture_lambdas_losslessly() {
    let source = r"void callbacks(i32 value, i32& total) {
    apply([](i32 item) { return item; });
    apply([value](i32 item) { return value + item; });
    apply([&total](i32 item) { total += item; });
    apply([count = value + 1](i32 item) mutable {
        count += item;
        return count;
    });
}
";
    let parsed = parse(source);
    let root = parsed.syntax();
    let lambda = root
        .descendants()
        .find_map(stainless_syntax::ast::LambdaExpression::cast)
        .expect("lambda expression");

    assert!(parsed.errors().is_empty(), "{:?}", parsed.errors());
    assert_eq!(root.to_string(), source);
    assert_eq!(count_kind(&root, SyntaxKind::LambdaExpression), 4);
    assert_eq!(count_kind(&root, SyntaxKind::LambdaCapture), 3);
    assert_eq!(
        lambda
            .parameter_list()
            .expect("lambda parameters")
            .parameters()
            .count(),
        1
    );
    assert!(lambda.body().is_some());
    assert_eq!(
        root.descendants()
            .filter_map(stainless_syntax::ast::LambdaExpression::cast)
            .filter(stainless_syntax::ast::LambdaExpression::is_mutable)
            .count(),
        1
    );
}

#[test]
fn parses_async_functions_lambdas_and_await_losslessly() {
    let source = r"async i32 load(i32 value) {
    return run([value](i32 input) async {
        return fetch(input).await + value;
    }).await;
}
";
    let parsed = parse(source);
    let root = parsed.syntax();
    let lambda = root
        .descendants()
        .find_map(stainless_syntax::ast::LambdaExpression::cast)
        .expect("async lambda");

    assert!(parsed.errors().is_empty(), "{:?}", parsed.errors());
    assert_eq!(root.to_string(), source);
    assert!(lambda.is_async());
    assert_eq!(count_kind(&root, SyntaxKind::AwaitExpression), 2);
}

#[test]
fn rejects_borrowed_lambda_capture_initializers() {
    let source = "void invalid(i32 value) { apply([&value = move(value)]() {}); }";
    let parsed = parse(source);

    assert_eq!(parsed.syntax().to_string(), source);
    assert!(
        parsed.errors().iter().any(|error| error
            .message
            .contains("borrowed lambda capture cannot have an initializer")),
        "{:?}",
        parsed.errors()
    );
}

#[test]
fn parses_stored_function_signatures_losslessly() {
    let source = r"function<i32(i32, const String&)> transform(
    function_mut<void()> callback);
";
    let parsed = parse(source);

    assert!(parsed.errors().is_empty(), "{:?}", parsed.errors());
    assert_eq!(parsed.syntax().to_string(), source);
    assert_eq!(
        count_kind(&parsed.syntax(), SyntaxKind::FunctionTypeSignature),
        2
    );
}

#[test]
fn parses_numeric_tuple_projection_losslessly() {
    let source = "u32 version(const tuple<i32, u32>& key) { return key.1; }\n";
    let parsed = parse(source);

    assert!(parsed.errors().is_empty(), "{:?}", parsed.errors());
    assert_eq!(parsed.syntax().to_string(), source);
    assert_eq!(count_kind(&parsed.syntax(), SyntaxKind::FieldExpression), 1);
}

#[test]
fn parses_supported_formatting_macros_losslessly() {
    let source = r#"use rust::{eprintln, format, println, write, writeln, String};

void macros(String& output, i32 value) {
    println!("Hello, {}!", value);
    rust::println!();
    eprintln!("error: {}", value);
    String text = format!("value={}", value);
    write!(output, "{}", text);
    writeln!(output);
}
"#;
    let parsed = parse(source);

    assert!(parsed.errors().is_empty(), "{:?}", parsed.errors());
    assert_eq!(parsed.syntax().to_string(), source);
    assert_eq!(
        count_kind(&parsed.syntax(), SyntaxKind::MacroCallExpression),
        6
    );
}

#[test]
fn parses_json_literals_without_confusing_arrays_with_lambdas() {
    let source = r#"void json() {
    var value = {name: "Stainless", "items": [1, null, {}]};
    var item = value.items[0];
    auto callback = [value](i32 input) { return input; };
}
"#;
    let parsed = parse(source);
    let root = parsed.syntax();

    assert!(parsed.errors().is_empty(), "{:?}", parsed.errors());
    assert_eq!(root.to_string(), source);
    assert_eq!(count_kind(&root, SyntaxKind::JsonObjectExpression), 2);
    assert_eq!(count_kind(&root, SyntaxKind::JsonArrayExpression), 1);
    assert_eq!(count_kind(&root, SyntaxKind::JsonMember), 2);
    assert_eq!(count_kind(&root, SyntaxKind::LambdaExpression), 1);
}

#[test]
fn parser_recovers_and_parses_the_following_function() {
    let source = r"i32 broken(i32 value) {
    i32 missing = ;
    return value
}

i32 intact() {
    return 1;
}
";
    let parsed = parse(source);
    let root = parsed.syntax();

    assert!(parsed.errors().len() >= 2);
    assert_eq!(root.to_string(), source);
    assert_eq!(count_kind(&root, SyntaxKind::FunctionDefinition), 2);
    assert!(root.to_string().contains("intact"));
}

#[test]
fn range_binding_variants_have_one_range_clause_shape() {
    for binding in [
        "const auto& value",
        "auto& value",
        "auto value",
        "i32 value",
    ] {
        let source = format!("void visit(Vec<i32>& values) {{ for ({binding} : values) {{}} }}");
        let parsed = parse(&source);
        let root = parsed.syntax();

        assert!(
            parsed.errors().is_empty(),
            "{binding}: {:?}",
            parsed.errors()
        );
        assert_eq!(root.to_string(), source);
        assert_eq!(count_kind(&root, SyntaxKind::RangeForClause), 1);
    }
}

#[test]
fn parses_map_structured_range_bindings_losslessly() {
    let source = "void visit(Map<i32, i32>& values) { \
        for (const auto& [key, value] : values) {} \
        for (auto& [key, value] : values) { value += key; } \
    }";
    let parsed = parse(source);
    let root = parsed.syntax();

    assert!(parsed.errors().is_empty(), "{:?}", parsed.errors());
    assert_eq!(root.to_string(), source);
    assert_eq!(count_kind(&root, SyntaxKind::RangeForClause), 2);

    let tree = parsed.tree();
    let Item::FunctionDefinition(function) = tree.items().next().unwrap() else {
        panic!("expected function");
    };
    for statement in function.body().unwrap().statements() {
        let Statement::For(statement) = statement else {
            panic!("expected range loop");
        };
        let Some(ForClause::Range(range)) = statement.clause() else {
            panic!("expected range clause");
        };
        assert!(range.is_structured());
        assert_eq!(
            range
                .name_tokens()
                .map(|token| token.text().to_owned())
                .collect::<Vec<_>>(),
            ["key", "value"]
        );
    }
}

#[test]
fn typed_tree_exposes_function_and_range_loop_structure() {
    let source = "i32 sum(const Vec<i32>& values) { \
        for (const auto& value : values) { return value; } \
    }\n";
    let parsed = parse(source);
    let tree = parsed.tree();
    let Item::FunctionDefinition(function) = tree.items().next().expect("function") else {
        panic!("expected a function definition");
    };
    let parameters = function
        .parameter_list()
        .expect("parameter list")
        .parameters()
        .collect::<Vec<_>>();
    let body = function.body().expect("body");
    let Statement::For(for_statement) = body.statements().next().expect("for statement") else {
        panic!("expected a for statement");
    };
    let Some(ForClause::Range(range)) = for_statement.clause() else {
        panic!("expected a range clause");
    };

    assert!(parsed.errors().is_empty(), "{:?}", parsed.errors());
    assert_eq!(function.name_tokens().next().unwrap().text(), "sum");
    assert_eq!(parameters.len(), 1);
    assert!(parameters[0].ty().unwrap().is_reference());
    assert!(range.ty().unwrap().is_auto());
    assert!(range.ty().unwrap().is_const());
    assert_eq!(range.name_token().unwrap().text(), "value");
    assert!(matches!(range.iterable(), Some(Expression::Name(_))));
    assert_eq!(tree.syntax().to_string(), source);
}

#[test]
fn typed_classic_for_slots_preserve_omitted_expressions() {
    let source = "void loops() { \
        for (;; tick()) {} \
        for (i32 i = 0; i < 3; i++) {} \
        for (reset(); ready(); ) {} \
    }";
    let parsed = parse(source);
    let tree = parsed.tree();
    let Item::FunctionDefinition(function) = tree.items().next().expect("function") else {
        panic!("expected a function definition");
    };
    let clauses = function
        .body()
        .expect("body")
        .statements()
        .map(|statement| {
            let Statement::For(statement) = statement else {
                panic!("expected only for statements");
            };
            let Some(ForClause::Classic(clause)) = statement.clause() else {
                panic!("expected a classic clause");
            };
            clause
        })
        .collect::<Vec<_>>();

    assert!(parsed.errors().is_empty(), "{:?}", parsed.errors());
    assert!(clauses[0].initializer().is_none());
    assert!(clauses[0].condition().is_none());
    assert!(clauses[0].update().is_some());
    assert!(matches!(
        clauses[1].initializer(),
        Some(ClassicForInitializer::Declaration(_))
    ));
    assert!(clauses[1].condition().is_some());
    assert!(clauses[1].update().is_some());
    assert!(matches!(
        clauses[2].initializer(),
        Some(ClassicForInitializer::Expression(_))
    ));
    assert!(clauses[2].condition().is_some());
    assert!(clauses[2].update().is_none());
}

fn count_kind(root: &stainless_syntax::SyntaxNode, kind: SyntaxKind) -> usize {
    root.descendants()
        .filter(|node| node.kind() == kind)
        .count()
}
