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

fn count_kind(root: &stainless_syntax::SyntaxNode, kind: SyntaxKind) -> usize {
    root.descendants()
        .filter(|node| node.kind() == kind)
        .count()
}
