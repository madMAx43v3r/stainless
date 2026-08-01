use stainless_compiler::interop::{
    ArgumentAdaptation, CallStyle, Receiver, RustLowering, TypeRef, parse_bindings_manifest,
    standard_bindings,
};
use stainless_compiler::resolution::{
    CallTarget, Intrinsic, NativeResultException, RustErrorMessage,
};
use stainless_compiler::{analyze, analyze_with_bindings};

#[test]
fn resolves_reference_parser_fixtures_without_semantic_errors() {
    for source in [
        include_str!("../../../docs/ref/01_basics.stl"),
        include_str!("../../../docs/ref/11_vec_and_string.stl"),
        include_str!("../../../docs/ref/13_range_for.stl"),
        include_str!("../../../docs/ref/15_checked_exception_subset.stl"),
        include_str!("../../../docs/ref/16_native_result_unwrap.stl"),
        include_str!("../../../docs/ref/19_stored_functions.stl"),
        include_str!("../../../docs/ref/20_formatting_macros.stl"),
        include_str!("../../../docs/ref/22_pointer_family.stl"),
        include_str!("../../../docs/ref/23_mutex_and_condition.stl"),
        include_str!("../../../docs/ref/24_threads.stl"),
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
fn resolves_owned_and_scoped_threads_with_checked_join_errors() {
    let analysis = analyze(include_str!("../../../docs/ref/24_threads.stl"));

    assert!(
        analysis.diagnostics.is_empty(),
        "{:#?}",
        analysis.diagnostics
    );
    for expected in ["spawn", "join", "scope", "scoped_spawn"] {
        assert!(analysis.semantics.calls.iter().any(|call| matches!(
            (&call.target, expected),
            (CallTarget::Intrinsic(Intrinsic::ThreadSpawn), "spawn")
                | (CallTarget::Intrinsic(Intrinsic::ThreadJoin), "join")
                | (CallTarget::Intrinsic(Intrinsic::ThreadScope), "scope")
                | (
                    CallTarget::Intrinsic(Intrinsic::ScopedThreadSpawn),
                    "scoped_spawn"
                )
        )));
    }
}

#[test]
fn threads_reject_borrowed_unscoped_and_non_send_captures() {
    let analysis = analyze(
        r"use rust::std::thread;

void invalid_threads() {
    mutex<i32> state = mutex<i32>(0);
    auto borrowed = thread::spawn([&state]() {
    });

    function<void()> callback = []() {
    };
    auto non_send = thread::spawn([callback]() {
        callback();
    });

    auto unhandled = thread::spawn([]() {
    });
    unhandled.join();
}
",
    );
    let codes = analysis
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.code)
        .collect::<Vec<_>>();

    for expected in ["RES075", "RES115"] {
        assert!(
            codes.contains(&expected),
            "missing {expected}: {:#?}",
            analysis.diagnostics
        );
    }
}

#[test]
fn resolves_mutex_guards_condition_waits_and_notifications() {
    let analysis = analyze(include_str!("../../../docs/ref/23_mutex_and_condition.stl"));

    assert!(
        analysis.diagnostics.is_empty(),
        "{:#?}",
        analysis.diagnostics
    );
    assert!(analysis.semantics.bindings.iter().any(|binding| {
        matches!(
            &binding.ty,
            TypeRef::MutexGuard(target)
                if matches!(target.as_ref(), TypeRef::Struct { path } if path == &["SharedState"])
        )
    }));
    for expected in ["lock", "wait", "notify"] {
        assert!(analysis.semantics.calls.iter().any(|call| matches!(
            (&call.target, expected),
            (CallTarget::Intrinsic(Intrinsic::MutexLock { .. }), "lock")
                | (
                    CallTarget::Intrinsic(Intrinsic::ConditionWait { .. }),
                    "wait"
                )
                | (
                    CallTarget::Intrinsic(Intrinsic::ConditionNotify { all: true }),
                    "notify"
                )
        )));
    }
}

#[test]
fn mutex_and_condition_reject_copying_invalid_waits_and_guard_moves() {
    let analysis = analyze(
        r"struct InvalidStorage {
    mutex<i32> state;
    condition changed;
};

void invalid_sync() {
    mutex<i32> state = mutex<i32>(0);
    condition changed;
    mutex<i32> copied_state = state;
    condition copied_condition = changed;
    auto guard = state.lock();
    changed.wait(1);
    auto moved_guard = move(guard);
}
",
    );
    let codes = analysis
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.code)
        .collect::<Vec<_>>();

    for expected in ["RES027", "RES092", "RES113", "RES114"] {
        assert!(
            codes.contains(&expected),
            "missing {expected}: {:#?}",
            analysis.diagnostics
        );
    }
}

#[test]
fn stored_function_resolution_enforces_ownership_and_signature_rules() {
    let analysis = analyze(
        r#"struct InvalidHolder {
    function_mut<i32()> callback;
};

struct Failure : stainless::Exception {};

i32 throwing_target() throws Failure {
    throw Failure{stainless::Exception("failed")};
}

i32 by_value(i32 value) {
    return value;
}

void takes_reference_return(function<const i32&()> callback) {}

void invalid_stored_functions(const i32& borrowed) {
    function<i32()> missing;
    function<i32()> captures_borrow = [&borrowed]() { return borrowed; };
    function<i32()> mutable_shared = [count = 0]() mutable {
        count += 1;
        return count;
    };
    function_mut<i32()> unique = []() { return 1; };
    function_mut<i32()> copied = unique;
    const function_mut<i32()> frozen = []() { return 1; };
    frozen();
    function<i32()> throwing = throwing_target;
    function<i32(const i32&)> wrong_function = by_value;
    function<i32(const i32&)> wrong_lambda = [](i32 value) { return value; };
    missing(1);
}
"#,
    );
    let codes = analysis
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.code)
        .collect::<Vec<_>>();

    for expected in [
        "RES027", "RES028", "RES065", "RES084", "RES085", "RES092", "RES093", "RES094", "RES095",
        "RES096",
    ] {
        assert!(
            codes.contains(&expected),
            "missing {expected}: {:#?}",
            analysis.diagnostics
        );
    }
}

#[test]
fn stored_function_signature_overloads_have_distinct_deterministic_names() {
    let analysis = analyze(
        r"i32 invoke(function<i32(i32)> callback, i32 value) {
    return callback(value);
}

i32 invoke(function<i32(const i32&)> callback, i32 value) {
    return callback(value);
}
",
    );

    assert!(
        analysis.diagnostics.is_empty(),
        "{:?}",
        analysis.diagnostics
    );
    let overloads = analysis
        .semantics
        .functions
        .iter()
        .filter(|function| function.path == ["invoke"])
        .collect::<Vec<_>>();
    assert_eq!(overloads.len(), 2);
    assert_ne!(overloads[0].mangled_name, overloads[1].mangled_name);
}

#[test]
fn resolves_non_null_unique_owners_allocation_borrowing_and_moves() {
    let analysis = analyze(
        r"struct Config {
    i32 version;
};

i32 read_version(const Config& config) {
    return config.version;
}

void bump(Config& config) {
    config.version += 1;
}

unique_ptr<Config> make_config(i32 version) {
    unique_ptr<Config> owner = make_unique<Config>(Config{version});
    return move(owner);
}

i32 consume(unique_ptr<Config> owner) {
    bump(owner);
    return read_version(owner);
}

i32 use_unique() {
    unique_ptr<Config> owner = make_config(4);
    i32 before = read_version(owner);
    return before + consume(move(owner));
}
",
    );

    assert!(
        analysis.diagnostics.is_empty(),
        "{:#?}",
        analysis.diagnostics
    );
    assert!(analysis.semantics.bindings.iter().any(|binding| {
        matches!(
            &binding.ty,
            TypeRef::Pointer {
                kind: stainless_compiler::interop::PointerKind::Unique,
                target,
            } if matches!(target.as_ref(), TypeRef::Struct { path } if path == &["Config"])
        )
    }));
    assert!(analysis.semantics.calls.iter().any(|call| {
        matches!(
            call.target,
            CallTarget::Intrinsic(Intrinsic::MakeOwner { .. })
        )
    }));
}

#[test]
fn unique_owners_reject_copying_default_construction_references_and_use_after_move() {
    let analysis = analyze(
        r"struct Config { i32 version; };

struct InvalidHolder {
    unique_ptr<Config> owner;
};

void take(unique_ptr<Config> owner) {}
void invalid_ref(const unique_ptr<Config>& owner) {}

void invalid_unique() {
    unique_ptr<Config> missing;
    unique_ptr<Config> owner = make_unique<Config>(Config{1});
    unique_ptr<Config> copied = owner;
    take(owner);
    unique_ptr<Config> moved = move(owner);
    i32 stale = owner.version;
    make_unique(Config{2});
}
",
    );
    let codes = analysis
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.code)
        .collect::<Vec<_>>();

    for expected in ["RES027", "RES092", "RES105", "RES106"] {
        assert!(
            codes.contains(&expected),
            "missing {expected}: {:#?}",
            analysis.diagnostics
        );
    }
}

#[test]
fn unique_owner_use_after_move_is_an_ownership_error() {
    let analysis = analyze(
        r"struct Config { i32 version; };

i32 invalid_unique_move() {
    unique_ptr<Config> owner = make_unique<Config>(Config{1});
    unique_ptr<Config> moved = move(owner);
    return owner.version + moved.version;
}
",
    );

    assert!(
        analysis
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "OWN001"),
        "{:#?}",
        analysis.diagnostics
    );
}

#[test]
fn resolves_nullable_shared_weak_and_atomic_pointer_operations() {
    let analysis = analyze(
        r"struct Config { i32 version; };

i32 pointer_family() {
    unique_nullptr<Config> maybe_unique;
    if (!maybe_unique) {
        maybe_unique = unique_nullptr<Config>(make_unique<Config>(Config{3}));
    }
    maybe_unique.version = 4;
    unique_ptr<Config> unique = unique_ptr<Config>(move(maybe_unique));

    shared_ptr<Config> first = make_shared<Config>(Config{5});
    shared_ptr<Config> copied = first;
    shared_nullptr<Config> maybe_shared = shared_nullptr<Config>(first);
    weak_ptr<Config> weak = downgrade(copied);
    shared_nullptr<Config> promoted = lock(weak);
    if (!promoted) {
        return -1;
    }
    shared_ptr<Config> recovered = shared_ptr<Config>(promoted);

    atomic_ptr<Config> slot = atomic_ptr<Config>(first);
    shared_ptr<Config> snapshot = slot.__load();
    slot.__store(recovered);
    shared_ptr<Config> previous = slot.__swap(copied);

    atomic_nullptr<Config> optional_slot;
    optional_slot.__store(move(maybe_shared));
    shared_nullptr<Config> optional_snapshot = optional_slot.__load();
    return unique.version + snapshot.version + previous.version;
}
",
    );

    assert!(
        analysis.diagnostics.is_empty(),
        "{:#?}",
        analysis.diagnostics
    );
    for kind in [
        stainless_compiler::interop::PointerKind::UniqueNullable,
        stainless_compiler::interop::PointerKind::Shared,
        stainless_compiler::interop::PointerKind::SharedNullable,
        stainless_compiler::interop::PointerKind::Weak,
        stainless_compiler::interop::PointerKind::Atomic,
        stainless_compiler::interop::PointerKind::AtomicNullable,
    ] {
        assert!(analysis.semantics.bindings.iter().any(|binding| {
            matches!(&binding.ty, TypeRef::Pointer { kind: actual, .. } if *actual == kind)
        }));
    }
}

#[test]
fn pointer_family_rejects_unchecked_null_access_shared_mutation_and_slot_copying() {
    let analysis = analyze(
        r"use rust::Option;

struct Config {
    i32 version;
    void bump();
};

void Config::bump() {
    version += 1;
}

void mutate(Config& config) {
    config.version += 1;
}

void invalid_pointers() {
    shared_ptr<Config> shared = make_shared<Config>(Config{1});
    shared.version = 2;
    shared.bump();
    mutate(shared);

    shared_nullptr<Config> maybe;
    i32 unchecked = maybe.version;
    shared_ptr<Config> recovered = shared_ptr<Config>(maybe);

    unique_nullptr<Config> unique = unique_nullptr<Config>(
        make_unique<Config>(Config{2}));
    unique_nullptr<Config> copied_unique = unique;

    atomic_ptr<Config> slot = atomic_ptr<Config>(shared);
    atomic_ptr<Config> copied_slot = slot;
    Option<shared_ptr<Config>> nested_pointer;
    auto untyped_null = nullptr;
}
",
    );
    let codes = analysis
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.code)
        .collect::<Vec<_>>();

    for expected in [
        "RES013", "RES024", "RES026", "RES027", "RES110", "RES111", "RES112",
    ] {
        assert!(
            codes.contains(&expected),
            "missing {expected}: {:#?}",
            analysis.diagnostics
        );
    }
}

#[test]
fn formatting_macros_require_imports_literal_formats_and_supported_values() {
    let valid = analyze(
        r#"use rust::{eprintln, format, println, write, writeln, String};

void macros(String& output, i32 value) throws stainless::FormatError {
    println!("value = {}", value);
    eprintln!();
    String rendered = format!("rendered = {}", value);
    write!(output, "{}", rendered);
    writeln!(output);
}
"#,
    );
    assert!(valid.diagnostics.is_empty(), "{:?}", valid.diagnostics);
    let macros = valid
        .semantics
        .functions
        .iter()
        .find(|function| function.path == ["macros"])
        .expect("formatting macro function");
    assert_eq!(macros.throws.len(), 1);
    assert_eq!(
        valid
            .semantics
            .structure(macros.throws[0])
            .expect("FormatError structure")
            .path,
        ["stainless", "FormatError"]
    );

    let invalid = analyze(
        r#"use rust::String;

struct Value { i32 number; };

void bad(const String& output, String format, Value value) {
    println!("not imported");
    eprintln!("also not imported");
    rust::format!();
    rust::format!(format, 1);
    rust::format!("{}", value);
    rust::write!(output, "cannot mutate");
    rust::write!(1, "not a String");
    rust::write!(output);
    rust::writeln!();
}
"#,
    );
    let codes = invalid
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.code)
        .collect::<Vec<_>>();
    for expected in [
        "RES075", "RES097", "RES098", "RES099", "RES100", "RES101", "RES102",
    ] {
        assert!(
            codes.contains(&expected),
            "missing {expected}: {:#?}",
            invalid.diagnostics
        );
    }
}

#[test]
fn resolves_external_reference_fixture_with_its_manifest() {
    let external = parse_bindings_manifest(include_str!(
        "../../../docs/ref/17_external_regex_wrapper.bindings.toml"
    ))
    .unwrap();
    let bindings = standard_bindings().unwrap().merge(external).unwrap();
    let analysis = analyze_with_bindings(
        include_str!("../../../docs/ref/17_external_regex_wrapper.stl"),
        &bindings,
    );

    assert!(
        analysis.diagnostics.is_empty(),
        "{:?}",
        analysis.diagnostics
    );
}

#[test]
fn resolves_external_callback_fixture_with_its_manifest() {
    let bindings = callback_bindings();
    let analysis = analyze_with_bindings(
        include_str!("../../../docs/ref/18_external_callbacks.stl"),
        &bindings,
    );

    assert!(
        analysis.diagnostics.is_empty(),
        "{:?}",
        analysis.diagnostics
    );
    assert_eq!(analysis.semantics.callbacks.len(), 7);
}

#[test]
fn callback_resolution_rejects_ambiguous_unsafe_and_non_contextual_forms() {
    let bindings = callback_bindings();
    let analysis = analyze_with_bindings(
        r#"use rust::String;
use rust::callback_fixture::Processor;

struct CallbackError : stainless::Exception {};

i32 throwing_callback(i32 value) throws CallbackError {
    throw CallbackError{stainless::Exception("callback failure")};
}

void contextless_lambda() {
    auto callback = [](i32 value) {
        return value;
    };
}

void invalid_callbacks(const i32& reference) {
    Processor processor = Processor::new(1);
    i32 captured = 2;
    String owned = "not copyable";

    processor.apply_fn_ptr(1, [captured](i32 value) {
        return captured + value;
    });
    processor.inspect(1, [](i32 value) {
        return captured + value;
    });
    processor.inspect(1, [owned](i32 value) {
        return value;
    });
    processor.inspect(1, [reference](i32 value) {
        return value;
    });
    processor.inspect(1, [copy = reference](i32 value) {
        return copy + value;
    });
    processor.inspect(1, [text = owned](i32 value) {
        return value;
    });
    processor.apply(1, [count = captured](i32 value) {
        count += value;
        return count;
    });
    processor.inspect(1, [](i32 value) {
        return;
    });
    processor.inspect(1, throwing_callback);
}
"#,
        &bindings,
    );
    let codes = analysis
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.code)
        .collect::<Vec<_>>();

    for expected in [
        "RES013", "RES027", "RES082", "RES085", "RES086", "RES088", "RES089", "RES090",
    ] {
        assert!(
            codes.contains(&expected),
            "missing {expected}: {:?}",
            analysis.diagnostics
        );
    }
    assert!(
        analysis.diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("unresolved value name `captured`")),
        "{:?}",
        analysis.diagnostics
    );
}

#[test]
fn moved_callback_capture_invalidates_the_outer_binding() {
    let bindings = callback_bindings();
    let analysis = analyze_with_bindings(
        r#"use rust::String;
use rust::callback_fixture::Processor;

usize invalid_after_capture() {
    Processor processor = Processor::new(1);
    String factor = "two";
    i32 output = processor.inspect(
        1,
        [captured = move(factor)](i32 value) {
            return value;
        }
    );
    return factor.len();
}
"#,
        &bindings,
    );

    assert!(
        analysis
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "OWN001"),
        "{:?}",
        analysis.diagnostics
    );
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
            .map(|call| (call.style, call.source_name.as_str()))
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
            rust_path: "::std::vec::Vec::new".to_owned()
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
            exception: NativeResultException::RustError,
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

fn callback_bindings() -> stainless_compiler::interop::NativeBindings {
    let external = parse_bindings_manifest(include_str!(
        "../../../docs/ref/18_external_callbacks.bindings.toml"
    ))
    .unwrap();
    standard_bindings().unwrap().merge(external).unwrap()
}
