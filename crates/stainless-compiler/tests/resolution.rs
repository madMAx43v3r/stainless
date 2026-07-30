use stainless_compiler::analyze;
use stainless_compiler::interop::{ArgumentAdaptation, CallStyle, Receiver, RustLowering, TypeRef};
use stainless_compiler::resolution::{CallTarget, Intrinsic, RustErrorMessage};

#[test]
fn resolves_reference_parser_fixtures_without_semantic_errors() {
    for source in [
        include_str!("../../../docs/ref/01_basics.stl"),
        include_str!("../../../docs/ref/11_vec_and_string.stl"),
        include_str!("../../../docs/ref/13_range_for.stl"),
        include_str!("../../../docs/ref/15_checked_exception_subset.stl"),
        include_str!("../../../docs/ref/16_native_result_unwrap.stl"),
        include_str!("../../../docs/ref/17_external_regex_wrapper.stl"),
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
            CallTarget::Stainless(_) | CallTarget::Constructor(_) | CallTarget::Intrinsic(_) => {
                None
            }
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
            CallTarget::Constructor(_) | CallTarget::Native(_) | CallTarget::Intrinsic(_) => None,
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
            CallTarget::Stainless(_) | CallTarget::Constructor(_) | CallTarget::Intrinsic(_) => {
                None
            }
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

#[test]
fn checked_exception_sets_and_handlers_are_validated() {
    let source = r#"struct NotAnError {};

struct BaseError : stainless::Exception {};

struct DerivedError : BaseError {};

i32 fail() throws DerivedError {
    throw DerivedError{BaseError{stainless::Exception("failure")}};
}

i32 uncaught() {
    return fail();
}

i32 malformed_set() throws NotAnError, DerivedError, DerivedError, BaseError;

i32 reference_set() throws const DerivedError&;

void unreachable_handler() {
    try {
        fail();
    } catch (const BaseError& error) {
        return;
    } catch (const DerivedError& error) {
        return;
    }
}

void invalid_handler() {
    try {
        fail();
    } catch (const NotAnError& error) {
        return;
    } catch (...) {
        return;
    }
}
"#;
    let analysis = analyze(source);
    let codes = analysis
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.code)
        .collect::<Vec<_>>();

    assert!(
        analysis.parse.errors().is_empty(),
        "{:?}",
        analysis.parse.errors()
    );
    for expected in ["RES070", "RES071", "RES072", "RES075", "RES076", "RES077"] {
        assert!(
            codes.contains(&expected),
            "missing {expected}: {:?}",
            analysis.diagnostics
        );
    }
}

#[test]
fn declarations_must_agree_on_checked_exception_sets() {
    let source = r"struct Failure : stainless::Exception {};

struct Resource {
    Resource() throws Failure;
};

Resource::Resource() {
}

i32 load() throws Failure;

i32 load() {
    return 1;
}

i32 uncaught_default_construction() {
    Resource resource;
    return 2;
}
";
    let analysis = analyze(source);
    let codes = analysis
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.code)
        .collect::<Vec<_>>();

    assert!(codes.contains(&"RES068"), "{:?}", analysis.diagnostics);
    assert!(codes.contains(&"RES069"), "{:?}", analysis.diagnostics);
    assert!(codes.contains(&"RES075"), "{:?}", analysis.diagnostics);
}

#[test]
fn native_result_unwrap_is_a_checked_consuming_intrinsic() {
    let source = r"use rust::{Result, String};

i32 unwrap_value(Result<i32, String> result) throws stainless::RustError {
    return result.unwrap();
}
";
    let analysis = analyze(source);

    assert!(
        analysis.diagnostics.is_empty(),
        "{:?}",
        analysis.diagnostics
    );
    let call = analysis
        .semantics
        .calls
        .iter()
        .find(|call| {
            matches!(
                call.target,
                CallTarget::Intrinsic(Intrinsic::UnwrapRustResult { .. })
            )
        })
        .expect("native Result unwrap call");
    assert_eq!(call.return_type, TypeRef::I32);
    assert_eq!(call.throws.len(), 1);
    assert_eq!(
        analysis.semantics.structure(call.throws[0]).unwrap().path,
        ["stainless", "RustError"]
    );
    assert_eq!(
        call.target,
        CallTarget::Intrinsic(Intrinsic::UnwrapRustResult {
            error_message: RustErrorMessage::Display,
        })
    );
}

#[test]
fn native_result_adaptation_reports_effect_receiver_and_move_errors() {
    let source = r"use rust::{Result, String};

i32 uncaught(Result<i32, String> result) {
    return result.unwrap();
}

i32 borrowed(const Result<i32, String>& result) throws stainless::RustError {
    return result.unwrap();
}

i32 bad_arity(Result<i32, String> result) throws stainless::RustError {
    return result.unwrap(1);
}

i32 implicit_without_move(
    Result<i32, String> result
) throws stainless::RustError {
    i32 value = result;
    return value;
}
";
    let analysis = analyze(source);
    let codes = analysis
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.code)
        .collect::<Vec<_>>();

    for expected in ["RES075", "RES078", "RES079", "RES080"] {
        assert!(
            codes.contains(&expected),
            "missing {expected}: {:?}",
            analysis.diagnostics
        );
    }
}
