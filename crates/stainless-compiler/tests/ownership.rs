use stainless_compiler::{DiagnosticPhase, analyze, transpile};

#[test]
fn detects_definite_and_control_flow_dependent_use_after_move() {
    let definite = analyze(
        r"use rust::String;

String invalid(String value) {
    String first = move(value);
    String second = move(value);
    return move(second);
}
",
    );
    assert_codes(&definite, &["OWN001"]);

    let conditional = analyze(
        r"use rust::String;

String invalid(String value, bool condition) {
    if (condition) {
        String consumed = move(value);
    }
    return move(value);
}
",
    );
    assert_codes(&conditional, &["OWN002"]);
}

#[test]
fn moving_function_mut_invalidates_the_source_binding() {
    let analysis = analyze(
        r"void invalid() {
    function_mut<i32()> source = []() { return 1; };
    function_mut<i32()> destination = move(source);
    source();
}
",
    );

    assert_codes(&analysis, &["OWN001"]);
}

#[test]
fn assignment_reinitializes_a_moved_binding_on_all_continuing_paths() {
    let source = r#"use rust::String;

String valid(String value, bool condition) {
    if (condition) {
        String consumed = move(value);
        value = "restored";
    }
    return move(value);
}
"#;
    let result = transpile(source);

    assert!(
        result.analysis.diagnostics.is_empty(),
        "{:?}",
        result.analysis.diagnostics
    );
    assert!(result.rust.is_some());
}

#[test]
fn local_borrows_end_after_their_last_use() {
    let source = r"use rust::String;

void valid(String value) {
    const String& shared = value;
    if (shared.is_empty()) {
    }
    value.push('!');

    String& exclusive = value;
    exclusive.push('?');
    value.push('.');

    String& first = value;
    String& second = first;
    second.push(':');
    value.push(';');

    const String& loop_alias = value;
    for (i32 index = 0; index < 2; index += 1) {
        if (loop_alias.is_empty()) {
        }
    }
    value.push(',');
}
";
    let result = transpile(source);

    assert!(
        result.analysis.diagnostics.is_empty(),
        "{:?}",
        result.analysis.diagnostics
    );
    assert!(result.rust.is_some());
}

#[test]
fn rejects_access_that_conflicts_with_a_live_local_borrow() {
    let shared = analyze(
        r"use rust::String;

usize invalid(String value) {
    const String& alias = value;
    value.push('!');
    return alias.len();
}
",
    );
    assert_codes(&shared, &["OWN003"]);

    let exclusive = analyze(
        r"use rust::String;

usize invalid(String value) {
    String& alias = value;
    usize length = value.len();
    alias.push('!');
    return length;
}
",
    );
    assert_codes(&exclusive, &["OWN003"]);

    let same_expression = analyze(
        r"use rust::String;

void pair(const String& left, String& right) {
}

void invalid(String value) {
    const String& alias = value;
    pair(alias, value);
}
",
    );
    assert_codes(&same_expression, &["OWN003"]);

    let overlapping_arguments = analyze(
        r"use rust::String;

void pair(String& left, String& right) {
}

void invalid(String value) {
    pair(value, value);
}
",
    );
    assert_codes(&overlapping_arguments, &["OWN003"]);

    let receiver_and_argument = analyze(
        r"use rust::Vec;

void invalid(Vec<i32> values) {
    values.append(values);
}
",
    );
    assert_codes(&receiver_and_argument, &["OWN003"]);
}

#[test]
fn rejects_invalid_reference_sources_and_escapes() {
    let temporary = analyze(
        r#"use rust::String;

usize invalid() {
    const String& alias = "temporary";
    return alias.len();
}
"#,
    );
    assert_codes(&temporary, &["OWN004"]);

    let by_value_return = analyze(
        r"use rust::String;

const String& invalid(String value) {
    return value;
}
",
    );
    assert_codes(&by_value_return, &["OWN005"]);

    let unrelated_local = analyze(
        r#"use rust::String;

const String& invalid(const String& input) {
    String local = "local";
    const String& alias = local;
    return alias;
}
"#,
    );
    assert_codes(&unrelated_local, &["OWN005"]);
}

#[test]
fn permits_reference_returns_tied_to_the_single_reference_parameter() {
    let source = r"use rust::String;

const String& identity(const String& input) {
    const String& alias = input;
    return alias;
}
";
    let result = transpile(source);

    assert!(
        result.analysis.diagnostics.is_empty(),
        "{:?}",
        result.analysis.diagnostics
    );
    assert!(result.rust.is_some());
}

#[test]
fn detects_moves_repeated_by_loops_and_consuming_ranges() {
    let repeated = analyze(
        r"use rust::String;

void invalid(String value) {
    for (i32 index = 0; index < 2; index += 1) {
        String consumed = move(value);
    }
}
",
    );
    assert_codes(&repeated, &["OWN001"]);

    let consuming_range = analyze(
        r"use rust::Vec;

Vec<i32> invalid(Vec<i32> values) {
    for (auto value : move(values)) {
    }
    return move(values);
}
",
    );
    assert_codes(&consuming_range, &["OWN001"]);

    let break_after_move = analyze(
        r"use rust::String;

String invalid(String value) {
    for (i32 index = 0; index < 2; index += 1) {
        String consumed = move(value);
        break;
    }
    return move(value);
}
",
    );
    assert_codes(&break_after_move, &["OWN002"]);

    let continue_then_reinitialize = analyze(
        r#"use rust::String;

void valid(String value) {
    for (i32 index = 0; index < 2; value = "restored") {
        String consumed = move(value);
        continue;
    }
}
"#,
    );
    assert_codes(&continue_then_reinitialize, &[]);
}

#[test]
fn catch_paths_preserve_moves_that_happened_before_an_exception() {
    let analysis = analyze(
        r#"use rust::String;

struct Failure : stainless::Exception {};

void invalid(String value) {
    try {
        String consumed = move(value);
        throw Failure{stainless::Exception("failure")};
    } catch (const Failure& error) {
        value.push('!');
    }
}
"#,
    );

    assert_codes(&analysis, &["OWN001"]);
}

#[test]
fn explicitly_unwrapping_a_named_native_result_consumes_it() {
    let analysis = analyze(
        r"use rust::{Result, String};

i32 invalid(Result<i32, String> result) throws stainless::RustError {
    i32 first = result.unwrap();
    return result.unwrap();
}
",
    );

    assert_codes(&analysis, &["OWN001"]);
}

#[test]
fn target_typed_result_conversion_preserves_the_consumed_exception_path() {
    let analysis = analyze(
        r"use rust::{Result, String};

struct Payload {
    i32 value;
};

void invalid(Result<Payload, String> result) {
    try {
        Payload value = Payload(move(result));
    } catch (const stainless::RustError& error) {
        Result<Payload, String> reused = move(result);
    }
}
",
    );

    assert_codes(&analysis, &["OWN001"]);
}

fn assert_codes(analysis: &stainless_compiler::Analysis, expected: &[&str]) {
    let ownership = analysis
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.phase == DiagnosticPhase::Ownership)
        .map(|diagnostic| diagnostic.code)
        .collect::<Vec<_>>();
    assert_eq!(ownership, expected, "{:?}", analysis.diagnostics);
}
