use stainless_compiler::analyze;
use stainless_compiler::interop::{ArgumentAdaptation, CallStyle, Receiver, RustLowering, TypeRef};
use stainless_compiler::resolution::{CallTarget, Intrinsic};

#[test]
fn resolves_reference_parser_fixtures_without_semantic_errors() {
    for source in [
        include_str!("../../../docs/ref/01_basics.stl"),
        include_str!("../../../docs/ref/11_vec_and_string.stl"),
        include_str!("../../../docs/ref/13_range_for.stl"),
    ] {
        let analysis = analyze(source);

        assert!(
            analysis.diagnostics.is_empty(),
            "{:?}",
            analysis.diagnostics
        );
    }
}

#[test]
fn resolves_vec_and_string_calls_through_concrete_native_bindings() {
    let source = r#"use rust::{String, Vec};

usize native_calls(const String& suffix) {
    Vec<String> values;
    values.reserve(3);
    values.push("stainless");

    String text = "hello";
    text.push_str(suffix);
    values.push(move(text));
    return values.len();
}
"#;
    let analysis = analyze(source);

    assert!(
        analysis.diagnostics.is_empty(),
        "{:?}",
        analysis.diagnostics
    );

    let native_calls = analysis
        .semantics
        .calls
        .iter()
        .filter_map(|call| match &call.target {
            CallTarget::Native(native) => Some(native),
            CallTarget::Stainless(_) | CallTarget::Intrinsic(_) => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        native_calls
            .iter()
            .map(|call| (call.style, call.source_name))
            .collect::<Vec<_>>(),
        [
            (CallStyle::Constructor, "Vec"),
            (CallStyle::Method, "reserve"),
            (CallStyle::Method, "push"),
            (CallStyle::Method, "push_str"),
            (CallStyle::Method, "push"),
            (CallStyle::Method, "len"),
        ]
    );

    let default_vec = native_calls[0];
    assert_eq!(
        default_vec.return_type,
        TypeRef::native("rust::Vec", vec![TypeRef::native("rust::String", vec![])])
    );
    assert_eq!(
        default_vec.lowering,
        RustLowering::AssociatedFunction {
            rust_path: "::std::vec::Vec::new"
        }
    );

    let reserve = native_calls[1];
    assert_eq!(reserve.parameter_types, [TypeRef::Usize]);
    assert_eq!(reserve.receiver, Some(Receiver::Mutable));

    let push = native_calls[2];
    assert_eq!(
        push.parameter_types,
        [TypeRef::native("rust::String", vec![])]
    );

    let push_str = native_calls[3];
    assert_eq!(push_str.adaptations, [ArgumentAdaptation::StringRefToStr]);
    assert!(
        analysis
            .semantics
            .calls
            .iter()
            .any(|call| call.target == CallTarget::Intrinsic(Intrinsic::Move))
    );
}

#[test]
fn exact_overloads_resolve_to_distinct_deterministic_function_ids() {
    let source = r#"use rust::String;

namespace samples {

i32 select(i32 value) {
    return value;
}

String select(String value) {
    return move(value);
}

i32 example() {
    i32 number = select(7);
    String text = select("seven");
    return number;
}

}
"#;
    let analysis = analyze(source);

    assert!(
        analysis.diagnostics.is_empty(),
        "{:?}",
        analysis.diagnostics
    );
    let overloads = analysis
        .semantics
        .functions
        .iter()
        .filter(|function| function.path == ["samples", "select"])
        .collect::<Vec<_>>();
    assert_eq!(overloads.len(), 2);
    assert_eq!(
        overloads[0].mangled_name,
        "__stainless_v1_f_2_7_samples_6_select__p_1_i32"
    );
    assert_eq!(
        overloads[1].mangled_name,
        "__stainless_v1_f_2_7_samples_6_select__p_1_n_2_4_rust_6_String"
    );

    let selected_ids = analysis
        .semantics
        .calls
        .iter()
        .filter_map(|call| match call.target {
            CallTarget::Stainless(id) => Some(id),
            CallTarget::Native(_) | CallTarget::Intrinsic(_) => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(selected_ids, [overloads[0].id, overloads[1].id]);
}

#[test]
fn aliases_constructors_and_associated_functions_use_expected_target_types() {
    let source = r"use rust::{String as Text, Vec};

Text copy_text(const Text& source) {
    return Text(source);
}

Vec<i32> allocated() {
    Vec<i32> values = Vec::with_capacity(4);
    return move(values);
}

namespace nested {

use rust::*;

String identity(String value) {
    return move(value);
}

}
";
    let analysis = analyze(source);

    assert!(
        analysis.diagnostics.is_empty(),
        "{:?}",
        analysis.diagnostics
    );
    let native_calls = analysis
        .semantics
        .calls
        .iter()
        .filter_map(|call| match &call.target {
            CallTarget::Native(native) => Some(native),
            CallTarget::Stainless(_) | CallTarget::Intrinsic(_) => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(native_calls.len(), 2);
    assert_eq!(native_calls[0].style, CallStyle::Constructor);
    assert_eq!(native_calls[0].source_name, "String");
    assert_eq!(
        native_calls[0].lowering,
        RustLowering::CloneArgument { index: 0 }
    );
    assert_eq!(native_calls[1].style, CallStyle::AssociatedFunction);
    assert_eq!(native_calls[1].source_name, "with_capacity");
    assert_eq!(native_calls[1].parameter_types, [TypeRef::Usize]);
    assert_eq!(
        native_calls[1].return_type,
        TypeRef::native("rust::Vec", vec![TypeRef::I32])
    );
}

#[test]
fn native_resolution_reports_exact_type_mutability_api_and_move_errors() {
    let source = r"use rust::Vec;

void invalid(const Vec<i32>& read_only) {
    Vec<i32> values;
    values.push(1u32);
    read_only.push(1);
    values.unknown();
    Vec<i32> copied = values;
}
";
    let analysis = analyze(source);
    let codes = analysis
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.code)
        .collect::<Vec<_>>();

    assert!(analysis.parse.errors().is_empty());
    assert!(codes.contains(&"RES023"), "{:?}", analysis.diagnostics);
    assert!(codes.contains(&"RES024"), "{:?}", analysis.diagnostics);
    assert!(codes.contains(&"RES022"), "{:?}", analysis.diagnostics);
    assert!(codes.contains(&"RES027"), "{:?}", analysis.diagnostics);
}

#[test]
fn conflicting_reference_only_overloads_are_rejected() {
    let source = r"i32 inspect(i32 value);
i32 inspect(const i32& value);
";
    let analysis = analyze(source);

    assert!(
        analysis
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "RES003")
    );
    assert_eq!(analysis.semantics.functions.len(), 1);
}
