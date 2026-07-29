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
