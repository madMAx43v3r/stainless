use stainless_compiler::ast::{
    ExpressionKind, ForClause, Item, LiteralKind, StatementKind, TypeKind,
};
use stainless_compiler::{DiagnosticPhase, analyze};

#[test]
fn analysis_lowers_namespaces_functions_types_and_range_loops() {
    let source = r"use rust::Vec;

namespace sample {

i32 sum(const Vec<i32>& values) {
    i32 total = 0;
    for (const auto& value : values) {
        total += value;
    }
    return total;
}

}
";
    let analysis = analyze(source);

    assert!(
        analysis.diagnostics.is_empty(),
        "{:?}",
        analysis.diagnostics
    );
    assert!(analysis.parse.errors().is_empty());
    assert_eq!(analysis.parse.syntax().to_string(), source);

    let Item::Use(import) = &analysis.ast.items[0] else {
        panic!("expected import");
    };
    assert_eq!(import.path, "rust::Vec");

    let Item::Namespace(namespace) = &analysis.ast.items[1] else {
        panic!("expected namespace");
    };
    let Item::Function(function) = &namespace.items[0] else {
        panic!("expected function");
    };
    assert_eq!(function.name.display(), "sum");
    assert_eq!(function.parameters[0].name, "values");
    let TypeKind::Named(parameter_type) = &function.parameters[0].ty.kind else {
        panic!("expected a named parameter type");
    };
    assert_eq!(parameter_type.path.display(), "Vec");
    assert_eq!(parameter_type.arguments.len(), 1);
    assert!(function.parameters[0].ty.is_const);
    assert!(function.parameters[0].ty.is_reference);

    let body = function.body.as_ref().expect("function body");
    let StatementKind::Local(total) = &body.statements[0].kind else {
        panic!("expected total declaration");
    };
    let Some(initializer) = &total.initializer else {
        panic!("expected initializer");
    };
    let ExpressionKind::Literal(literal) = &initializer.kind else {
        panic!("expected literal initializer");
    };
    assert_eq!(literal.kind, LiteralKind::Integer);
    assert_eq!(literal.text, "0");

    let StatementKind::For(loop_statement) = &body.statements[1].kind else {
        panic!("expected loop");
    };
    let ForClause::Range(range) = &loop_statement.clause else {
        panic!("expected range loop");
    };
    assert!(range.ty.is_inferred());
    assert!(range.ty.is_const);
    assert!(range.ty.is_reference);
    assert_eq!(range.name, "value");
}

#[test]
fn structural_semantics_reports_initial_binding_and_control_flow_rules() {
    let source = r"i32 broken(i32 value, i32 value) {
    auto missing;
    auto& reference = value;
    String& dangling;
    break;
    continue;
    return;
}

void wrong() {
    return 1;
}
";
    let analysis = analyze(source);
    let diagnostics = analysis
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code.starts_with("SEM"))
        .map(|diagnostic| (diagnostic.code, diagnostic.phase))
        .collect::<Vec<_>>();

    assert!(
        analysis.parse.errors().is_empty(),
        "{:?}",
        analysis.parse.errors()
    );
    assert_eq!(
        diagnostics,
        [
            ("SEM001", DiagnosticPhase::Semantic),
            ("SEM002", DiagnosticPhase::Semantic),
            ("SEM003", DiagnosticPhase::Semantic),
            ("SEM004", DiagnosticPhase::Semantic),
            ("SEM007", DiagnosticPhase::Semantic),
            ("SEM008", DiagnosticPhase::Semantic),
            ("SEM006", DiagnosticPhase::Semantic),
            ("SEM005", DiagnosticPhase::Semantic),
        ]
    );
    assert!(
        analysis
            .diagnostics
            .windows(2)
            .all(|pair| pair[0].span.start <= pair[1].span.start)
    );
}

#[test]
fn syntax_diagnostics_survive_ast_recovery() {
    let analysis = analyze("i32 broken() { return ;");

    assert!(
        analysis
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.phase == DiagnosticPhase::Syntax)
    );
    assert_eq!(analysis.ast.items.len(), 1);
}

#[test]
fn structural_semantics_rejects_invalid_catch_and_rethrow_forms() {
    let source = r"struct Failure : stainless::Exception {};

void invalid() {
    throw;
    try {
        return;
    } catch (...) {
        return;
    } catch (Failure error) {
        return;
    }
}
";
    let analysis = analyze(source);
    let codes = analysis
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code.starts_with("SEM"))
        .map(|diagnostic| diagnostic.code)
        .collect::<Vec<_>>();

    assert!(
        analysis.parse.errors().is_empty(),
        "{:?}",
        analysis.parse.errors()
    );
    assert!(codes.contains(&"SEM011"), "{:?}", analysis.diagnostics);
    assert!(codes.contains(&"SEM012"), "{:?}", analysis.diagnostics);
    assert!(codes.contains(&"SEM013"), "{:?}", analysis.diagnostics);
}

#[test]
fn lowers_json_literals_and_reports_json_specific_errors() {
    let source = r#"var valid() {
    var value = {name: "Stainless", values: [1, null, {}]};
    return move(value);
}

struct Unsupported {
    i32 value;
};

var invalid() {
    var duplicate = {field: 1, field: 2};
    var unsupported = [Unsupported{1}];
    return move(duplicate);
}
"#;
    let analysis = analyze(source);

    assert!(
        analysis.parse.errors().is_empty(),
        "{:?}",
        analysis.parse.errors()
    );
    let Item::Function(valid) = &analysis.ast.items[0] else {
        panic!("expected valid function");
    };
    let body = valid.body.as_ref().expect("valid body");
    let StatementKind::Local(value) = &body.statements[0].kind else {
        panic!("expected JSON local");
    };
    assert!(matches!(
        value.initializer.as_ref().map(|value| &value.kind),
        Some(ExpressionKind::JsonObject { members }) if members.len() == 2
    ));
    assert!(analysis.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "RES103" && diagnostic.message.contains("duplicate JSON object key")
    }));
    assert!(analysis.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "RES103" && diagnostic.message.contains("JSON values must be")
    }));
}

#[test]
fn json_native_results_require_stainless_json_error() {
    let source = r"use rust::String;

void invalid(const String& source) {
    var value = var::parse(source);
}
";
    let analysis = analyze(source);

    assert!(
        analysis.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == "RES075" && diagnostic.message.contains("stainless::JsonError")
        }),
        "{:?}",
        analysis.diagnostics
    );
}
