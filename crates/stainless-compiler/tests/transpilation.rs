use std::ffi::OsString;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

use stainless_compiler::interop::{load_bindings_manifest, standard_bindings};
use stainless_compiler::{DiagnosticPhase, transpile, transpile_with_bindings};

static TEMPORARY_INDEX: AtomicUsize = AtomicUsize::new(0);

const BEHAVIOR_SOURCE: &str = r#"use rust::{String, Vec};

namespace samples {

struct Pair {
    i32 left;
    i32 right;
};

struct BaseValue {
    i32 value;
    i32 read() const;
};

i32 BaseValue::read() const {
    return value;
}

struct DerivedValue : BaseValue {
    i32 extra;
    i32 read() const;
};

struct ChainValue {
    i32 value;
    void add(i32 delta);
    i32 read() const;
};

void ChainValue::add(i32 delta) {
    value += delta;
}

i32 ChainValue::read() const {
    return value;
}

i32 fluent_member_calls() {
    ChainValue value = ChainValue{1};
    value.add(2).add(3);
    return value.read();
}

i32 DerivedValue::read() const {
    return value + extra;
}

i32 read_base(const BaseValue& value) {
    return value.read();
}

i32 struct_inheritance() {
    DerivedValue value = DerivedValue{BaseValue{4}, 5};
    return value.read() * 10 + read_base(value) + value.BaseValue::value;
}

i32 struct_copy() {
    Pair original = Pair{10, 20};
    Pair copied = original;
    Pair assigned = Pair{0, 0};
    assigned = original;
    return copied.left + assigned.right + original.left;
}

i32 struct_range_copy() {
    Vec<Pair> values;
    values.push(Pair{1, 2});
    values.push(Pair{3, 4});
    i32 total = 0;
    for (auto value : values) {
        total += value.left + value.right;
    }
    return total;
}

i32 sum_to(i32 limit) {
    i32 total = 0;
    for (i32 current = 0; current < limit; current += 1) {
        total += current;
    }
    return total;
}

i32 sum_skipping_two() {
    i32 total = 0;
    for (i32 current = 0; current < 5; current += 1) {
        if (current == 2) {
            continue;
        }
        total += current;
    }
    return total;
}

i32 select(i32 value) {
    return value + 1;
}

u32 select(u32 value) {
    return value + 2u32;
}

i32 exact_overload() {
    return select(30);
}

f32 suffixed_float() {
    return 3.0f;
}

f64 default_float() {
    return 2.0;
}

u32 primitive_cast(i32 value) {
    return u32(value);
}

i32 sum_shared(const Vec<i32>& values) {
    i32 total = 0;
    for (const auto& value : values) {
        total += value;
    }
    return total;
}

i32 mutate_and_sum() {
    Vec<i32> values;
    values.push(1);
    values.push(2);
    values.push(3);
    for (auto& value : values) {
        value += 1;
    }
    return sum_shared(values);
}

String greeting() {
    String text = "hello";
    text.push('!');
    return move(text);
}

usize inspect_text(const String& text) {
    return text.len();
}

usize borrow_moved_text(String text) {
    return inspect_text(move(text));
}

const i32& identity_ref(const i32& value) {
    return value;
}

i32 use_reference_return(i32 value) {
    return identity_ref(value);
}

}

namespace samples {

i32 reopened_namespace() {
    return 23;
}

}

i32 samples::qualified_definition() {
    return 29;
}
"#;

#[test]
fn transpiles_and_compiles_resolved_reference_programs() {
    for (name, source) in [
        ("basics", include_str!("../../../docs/ref/01_basics.stl")),
        (
            "structs",
            include_str!("../../../docs/ref/02_structs_and_data_inheritance.stl"),
        ),
        (
            "vec_and_string",
            include_str!("../../../docs/ref/11_vec_and_string.stl"),
        ),
        (
            "range_for",
            include_str!("../../../docs/ref/13_range_for.stl"),
        ),
        (
            "constructors",
            include_str!("../../../docs/ref/14_constructors.stl"),
        ),
        (
            "checked-exception-subset",
            include_str!("../../../docs/ref/15_checked_exception_subset.stl"),
        ),
        (
            "native-result-unwrap",
            include_str!("../../../docs/ref/16_native_result_unwrap.stl"),
        ),
        (
            "formatting-macros",
            include_str!("../../../docs/ref/20_formatting_macros.stl"),
        ),
        (
            "pointer-family",
            include_str!("../../../docs/ref/22_pointer_family.stl"),
        ),
    ] {
        let result = transpile(source);
        assert!(
            result.analysis.diagnostics.is_empty(),
            "{name}: {:?}",
            result.analysis.diagnostics
        );
        let rust = result.rust.expect("valid source should emit Rust");
        compile_rust(name, &rust, CrateKind::Library);
    }
}

#[test]
fn generated_program_preserves_current_subset_behavior() {
    let result = transpile(BEHAVIOR_SOURCE);
    assert!(
        result.analysis.diagnostics.is_empty(),
        "{:?}",
        result.analysis.diagnostics
    );
    let mut rust = result.rust.expect("valid source should emit Rust");
    let find_name = |source_name: &str| {
        result
            .analysis
            .semantics
            .functions
            .iter()
            .find(|function| function.path == ["samples", source_name])
            .unwrap_or_else(|| panic!("missing `{source_name}`"))
            .mangled_name
            .clone()
    };
    let sum_to = find_name("sum_to");
    let struct_copy = find_name("struct_copy");
    let struct_inheritance = find_name("struct_inheritance");
    let struct_range_copy = find_name("struct_range_copy");
    let fluent_member_calls = find_name("fluent_member_calls");
    let sum_skipping_two = find_name("sum_skipping_two");
    let exact_overload = find_name("exact_overload");
    let suffixed_float = find_name("suffixed_float");
    let default_float = find_name("default_float");
    let primitive_cast = find_name("primitive_cast");
    let mutate_and_sum = find_name("mutate_and_sum");
    let greeting = find_name("greeting");
    let borrow_moved_text = find_name("borrow_moved_text");
    let use_reference_return = find_name("use_reference_return");
    let reopened_namespace = find_name("reopened_namespace");
    let qualified_definition = find_name("qualified_definition");
    write!(
        rust,
        r#"
fn main() {{
    assert_eq!(__stainless_namespace_samples::{sum_to}(5), 10);
    assert_eq!(__stainless_namespace_samples::{struct_copy}(), 40);
    assert_eq!(__stainless_namespace_samples::{struct_inheritance}(), 98);
    assert_eq!(__stainless_namespace_samples::{struct_range_copy}(), 10);
    assert_eq!(__stainless_namespace_samples::{fluent_member_calls}(), 6);
    assert_eq!(__stainless_namespace_samples::{sum_skipping_two}(), 8);
    assert_eq!(__stainless_namespace_samples::{exact_overload}(), 31);
    assert_eq!(__stainless_namespace_samples::{suffixed_float}(), 3.0f32);
    assert_eq!(__stainless_namespace_samples::{default_float}(), 2.0f64);
    assert_eq!(__stainless_namespace_samples::{primitive_cast}(7), 7u32);
    assert_eq!(__stainless_namespace_samples::{mutate_and_sum}(), 9);
    assert_eq!(__stainless_namespace_samples::{greeting}(), "hello!");
    assert_eq!(
        __stainless_namespace_samples::{borrow_moved_text}(
            ::std::string::String::from("moved")
        ),
        5
    );
    assert_eq!(__stainless_namespace_samples::{use_reference_return}(17), 17);
    assert_eq!(__stainless_namespace_samples::{reopened_namespace}(), 23);
    assert_eq!(__stainless_namespace_samples::{qualified_definition}(), 29);
}}
"#
    )
    .expect("writing to a String cannot fail");
    let binary = compile_rust("behavior", &rust, CrateKind::Binary);
    let output = Command::new(&binary)
        .output()
        .unwrap_or_else(|error| panic!("failed to run {}: {error}", binary.display()));
    assert!(
        output.status.success(),
        "generated binary failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    remove_temporary_parent(&binary);
}

#[test]
fn unique_ptr_lowers_to_box_and_preserves_move_and_borrow_behavior() {
    let source = r"struct Config {
    i32 version;
    Config(i32 version);
    void bump(i32 delta);
};

Config::Config(i32 version) : version(version) {
}

void Config::bump(i32 delta) {
    version += delta;
}

i32 read_version(const Config& config) {
    return config.version;
}

unique_ptr<Config> make_config(i32 version) {
    unique_ptr<Config> owner = make_unique<Config>(version);
    return move(owner);
}

i32 unique_behavior() {
    unique_ptr<Config> owner = make_config(4);
    i32 before = read_version(owner);
    owner.bump(3);
    unique_ptr<Config> relocated = move(owner);
    return before * 10 + relocated.version;
}
";
    let result = transpile(source);
    assert!(
        result.analysis.diagnostics.is_empty(),
        "{:#?}",
        result.analysis.diagnostics
    );
    let mut rust = result.rust.expect("unique owner source should emit Rust");
    assert!(rust.contains("::std::boxed::Box<"), "{rust}");
    assert!(rust.contains("::std::boxed::Box::new"), "{rust}");
    let function = result
        .analysis
        .semantics
        .functions
        .iter()
        .find(|function| function.path == ["unique_behavior"])
        .expect("unique behavior function")
        .mangled_name
        .clone();
    write!(
        rust,
        r"
fn main() {{
    assert_eq!({function}(), 47);
}}
"
    )
    .expect("writing to a String cannot fail");
    let binary = compile_rust("unique-ptr", &rust, CrateKind::Binary);
    let output = Command::new(&binary)
        .output()
        .unwrap_or_else(|error| panic!("failed to run {}: {error}", binary.display()));
    assert!(
        output.status.success(),
        "generated unique_ptr binary failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    remove_temporary_parent(&binary);
}

#[test]
fn make_unique_propagates_throwing_constructor_errors_once() {
    let source = r#"struct AllocationError : stainless::Exception {
};

struct Resource {
    i32 value;
    Resource(bool fail) throws AllocationError;
};

Resource::Resource(bool fail) throws AllocationError : value(7) {
    if (fail) {
        throw AllocationError{stainless::Exception("allocation failed")};
    }
}

unique_ptr<Resource> allocate(bool fail) throws AllocationError {
    return make_unique<Resource>(fail);
}

i32 allocation_behavior(bool fail) {
    try {
        unique_ptr<Resource> resource = allocate(fail);
        return resource.value;
    } catch (const AllocationError& error) {
        return -1;
    }
}
"#;
    let result = transpile(source);
    assert!(
        result.analysis.diagnostics.is_empty(),
        "{:#?}",
        result.analysis.diagnostics
    );
    let mut rust = result
        .rust
        .expect("throwing make_unique source should emit Rust");
    let function = result
        .analysis
        .semantics
        .functions
        .iter()
        .find(|function| function.path == ["allocation_behavior"])
        .expect("allocation behavior function")
        .mangled_name
        .clone();
    write!(
        rust,
        r"
fn main() {{
    assert_eq!({function}(false), 7);
    assert_eq!({function}(true), -1);
}}
"
    )
    .expect("writing to a String cannot fail");
    let binary = compile_rust("throwing-make-unique", &rust, CrateKind::Binary);
    let output = Command::new(&binary)
        .output()
        .unwrap_or_else(|error| panic!("failed to run {}: {error}", binary.display()));
    assert!(
        output.status.success(),
        "generated throwing make_unique binary failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    remove_temporary_parent(&binary);
}

#[test]
#[allow(clippy::too_many_lines)]
fn pointer_family_lowers_to_rust_owners_and_preserves_runtime_behavior() {
    let source = r"struct Config { i32 version; };

shared_ptr<Config> load_slot(const atomic_ptr<Config>& slot) {
    return slot.__load();
}

void bump(Config& config) {
    config.version += 1;
}

i32 read_version(const Config& config) {
    return config.version;
}

void store_slot(const atomic_ptr<Config>& slot, shared_ptr<Config> replacement) {
    slot.__store(replacement);
}

i32 pointer_behavior() {
    unique_nullptr<Config> maybe_unique;
    if (!maybe_unique) {
        maybe_unique = unique_nullptr<Config>(make_unique<Config>(Config{3}));
    }
    maybe_unique.version = 4;
    bump(maybe_unique);
    unique_ptr<Config> unique = unique_ptr<Config>(move(maybe_unique));

    shared_ptr<Config> first = make_shared<Config>(Config{5});
    shared_ptr<Config> copied = first;
    weak_ptr<Config> weak = downgrade(copied);
    shared_nullptr<Config> promoted = lock(weak);
    if (!promoted) {
        return -1;
    }
    i32 promoted_version = read_version(promoted);
    shared_ptr<Config> recovered = shared_ptr<Config>(promoted);

    atomic_ptr<Config> slot = atomic_ptr<Config>(first);
    shared_ptr<Config> snapshot = slot.__load();
    slot.__store(recovered);
    shared_ptr<Config> previous = slot.__swap(copied);
    shared_ptr<Config> through_reference = load_slot(slot);
    store_slot(slot, recovered);

    atomic_nullptr<Config> initialized_optional =
        atomic_nullptr<Config>(snapshot);
    shared_nullptr<Config> initialized_snapshot =
        initialized_optional.__load();
    if (!initialized_snapshot) {
        return -4;
    }

    atomic_nullptr<Config> optional_slot = atomic_nullptr<Config>(nullptr);
    shared_nullptr<Config> empty = optional_slot.__load();
    optional_slot.__store(shared_nullptr<Config>(snapshot));
    shared_nullptr<Config> installed = optional_slot.__swap(move(empty));
    if (!installed) {
        return -2;
    }

    weak_ptr<Config> expired;
    {
        shared_ptr<Config> temporary = make_shared<Config>(Config{99});
        expired = downgrade(temporary);
    }
    shared_nullptr<Config> expired_lock = lock(expired);
    if (expired_lock) {
        return -3;
    }
    return unique.version + snapshot.version + previous.version
        + installed.version + through_reference.version
        + initialized_snapshot.version + promoted_version;
}
";
    let result = transpile(source);
    assert!(
        result.analysis.diagnostics.is_empty(),
        "{:#?}",
        result.analysis.diagnostics
    );
    let mut rust = result.rust.expect("pointer family should emit Rust");
    assert!(rust.contains("::std::boxed::Box<"), "{rust}");
    assert!(rust.contains("::std::sync::Arc<"), "{rust}");
    assert!(rust.contains("::std::sync::Weak<"), "{rust}");
    assert!(rust.contains("::std::sync::RwLock<"), "{rust}");
    let function = result
        .analysis
        .semantics
        .functions
        .iter()
        .find(|function| function.path == ["pointer_behavior"])
        .expect("pointer behavior function")
        .mangled_name
        .clone();
    write!(
        rust,
        r"
fn main() {{
    assert_eq!({function}(), 35);
}}
"
    )
    .expect("writing to a String cannot fail");
    let binary = compile_rust("pointer-family", &rust, CrateKind::Binary);
    let output = Command::new(&binary)
        .output()
        .unwrap_or_else(|error| panic!("failed to run {}: {error}", binary.display()));
    assert!(
        output.status.success(),
        "generated pointer-family binary failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    remove_temporary_parent(&binary);
}

#[test]
fn mutex_and_condition_lower_to_thread_safe_rust_and_wake_waiters() {
    let result = transpile(include_str!("../../../docs/ref/23_mutex_and_condition.stl"));
    assert!(
        result.analysis.diagnostics.is_empty(),
        "{:#?}",
        result.analysis.diagnostics
    );
    let mut rust = result
        .rust
        .expect("mutex reference sample should emit Rust");
    assert!(rust.contains("::std::sync::Mutex<"), "{rust}");
    assert!(rust.contains("::std::sync::MutexGuard<'_"), "{rust}");
    assert!(rust.contains("::std::sync::Condvar"), "{rust}");
    assert!(rust.contains("into_inner()"), "{rust}");

    let function = |path: &[&str]| {
        result
            .analysis
            .semantics
            .functions
            .iter()
            .find(|function| {
                function
                    .path
                    .iter()
                    .map(String::as_str)
                    .eq(path.iter().copied())
            })
            .unwrap_or_else(|| panic!("missing function {path:?}"))
            .mangled_name
            .clone()
    };
    let new_state = function(&["new_shared_state"]);
    let new_condition = function(&["new_condition"]);
    let wait = function(&["wait_for_value"]);
    let publish = function(&["publish_value"]);
    write!(
        rust,
        r#"
fn main() {{
    let state = {new_state}();
    let changed = {new_condition}();
    let wait_state = ::std::sync::Arc::clone(&state);
    let wait_changed = ::std::sync::Arc::clone(&changed);
    let waiter = ::std::thread::spawn(move || {wait}(wait_state, wait_changed));
    ::std::thread::sleep(::std::time::Duration::from_millis(10));
    {publish}(state, changed, 42);
    assert_eq!(waiter.join().expect("waiter panicked"), 42);
}}
"#
    )
    .expect("writing to a String cannot fail");
    let binary = compile_rust("mutex-condition", &rust, CrateKind::Binary);
    let output = Command::new(&binary)
        .output()
        .unwrap_or_else(|error| panic!("failed to run {}: {error}", binary.display()));
    assert!(
        output.status.success(),
        "generated mutex-condition binary failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    remove_temporary_parent(&binary);
}

#[test]
fn constructors_initialize_bases_fields_and_synthesized_defaults() {
    let source = r#"use rust::{String, Vec};

namespace samples {

struct Base {
    i32 value;
    Base(i32 value);
};

Base::Base(i32 value) : value(value) {
}

struct Derived : Base {
    String label;
    Derived(i32 initial_value, const String& label);
};

Derived::Derived(i32 initial_value, const String& label)
    : Base(initial_value), label(label) {
    value += 1;
}

struct Defaults {
    Vec<i32> values;
};

i32 constructor_result() {
    Derived value = Derived(7, "abc");
    Defaults defaults;
    defaults.values.push(5);
    return value.value + i32(value.label.len()) + i32(defaults.values.len());
}

}
"#;
    let result = transpile(source);
    assert!(
        result.analysis.diagnostics.is_empty(),
        "{:?}",
        result.analysis.diagnostics
    );
    let function = result
        .analysis
        .semantics
        .functions
        .iter()
        .find(|function| function.path == ["samples", "constructor_result"])
        .expect("constructor_result symbol")
        .mangled_name
        .clone();
    let mut rust = result.rust.expect("constructors should emit Rust");
    write!(
        rust,
        "\nfn main() {{ assert_eq!(__stainless_namespace_samples::{function}(), 12); }}\n"
    )
    .expect("writing to a String cannot fail");
    let binary = compile_rust("constructors", &rust, CrateKind::Binary);
    let output = Command::new(&binary)
        .output()
        .expect("generated constructor program should run");
    assert!(
        output.status.success(),
        "status: {}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn checked_exceptions_propagate_and_match_typed_catches() {
    let source = r#"namespace samples {

struct IoError : stainless::Exception {
    i32 code;
};

struct ParseError : stainless::Exception {
    i32 offset;
};

i32 fail(i32 kind) throws IoError, ParseError {
    if (kind == 1) {
        throw IoError{stainless::Exception("io"), 10};
    }
    if (kind == 2) {
        throw ParseError{stainless::Exception("parse"), 20};
    }
    return 7;
}

i32 feed_forward(i32 kind) throws IoError, ParseError {
    return fail(kind) + 1;
}

i32 handle(i32 kind) {
    try {
        return feed_forward(kind);
    } catch (const IoError& error) {
        return error.code;
    } catch (const ParseError& error) {
        return error.offset;
    }
}

i32 partial(i32 kind) throws IoError {
    try {
        return fail(kind);
    } catch (const ParseError& error) {
        return error.offset + 1;
    }
}

i32 handle_partial(i32 kind) {
    try {
        return partial(kind);
    } catch (const IoError& error) {
        return error.code;
    }
}

} // namespace samples
"#;
    let result = transpile(source);
    assert!(
        result.analysis.diagnostics.is_empty(),
        "{:?}",
        result.analysis.diagnostics
    );
    let handle = result
        .analysis
        .semantics
        .functions
        .iter()
        .find(|function| function.path == ["samples", "handle"])
        .expect("handle symbol")
        .mangled_name
        .clone();
    let handle_partial = result
        .analysis
        .semantics
        .functions
        .iter()
        .find(|function| function.path == ["samples", "handle_partial"])
        .expect("handle_partial symbol")
        .mangled_name
        .clone();
    let mut rust = result.rust.expect("checked exceptions should emit Rust");
    write!(
        rust,
        r"
fn main() {{
    assert_eq!(__stainless_namespace_samples::{handle}(0), 8);
    assert_eq!(__stainless_namespace_samples::{handle}(1), 10);
    assert_eq!(__stainless_namespace_samples::{handle}(2), 20);
    assert_eq!(__stainless_namespace_samples::{handle_partial}(0), 7);
    assert_eq!(__stainless_namespace_samples::{handle_partial}(1), 10);
    assert_eq!(__stainless_namespace_samples::{handle_partial}(2), 21);
}}
"
    )
    .expect("writing to a String cannot fail");
    let binary = compile_rust("checked-exceptions", &rust, CrateKind::Binary);
    let output = Command::new(&binary)
        .output()
        .expect("generated exception program should run");
    assert!(
        output.status.success(),
        "status: {}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    remove_temporary_parent(&binary);
}

#[test]
fn try_blocks_preserve_break_and_continue_targets() {
    let source = r"i32 break_from_try() {
    for (i32 index = 0; index < 2; index += 1) {
        try {
            break;
        } catch (...) {
            break;
        }
    }
    return 9;
}

i32 continue_from_try() {
    for (i32 index = 0; index < 2; index += 1) {
        try {
            continue;
        } catch (...) {
            break;
        }
    }
    return 11;
}
";
    let result = transpile(source);
    assert!(
        result.analysis.diagnostics.is_empty(),
        "{:?}",
        result.analysis.diagnostics
    );
    let find_name = |source_name: &str| {
        result
            .analysis
            .semantics
            .functions
            .iter()
            .find(|function| function.path == [source_name])
            .unwrap_or_else(|| panic!("missing `{source_name}`"))
            .mangled_name
            .clone()
    };
    let break_from_try = find_name("break_from_try");
    let continue_from_try = find_name("continue_from_try");
    let mut rust = result.rust.expect("try loop control should emit Rust");
    write!(
        rust,
        r"
fn main() {{
    assert_eq!({break_from_try}(), 9);
    assert_eq!({continue_from_try}(), 11);
}}
"
    )
    .expect("writing to a String cannot fail");
    let binary = compile_rust("try-loop-control", &rust, CrateKind::Binary);
    let output = Command::new(&binary)
        .output()
        .expect("generated try loop-control program should run");
    assert!(output.status.success(), "{output:?}");
    remove_temporary_parent(&binary);
}

#[test]
fn throwing_constructors_and_bare_rethrows_preserve_exception_identity() {
    let source = r#"namespace samples {

struct DomainError : stainless::Exception {
    i32 code;
};

struct OpenError : DomainError {
    i32 detail;
};

struct Resource {
    i32 value;
    Resource(i32 value, bool fail) throws OpenError;
};

Resource::Resource(i32 value, bool fail) throws OpenError
    : value(value) {
    if (fail) {
        throw OpenError{
            DomainError{stainless::Exception("open"), 30},
            4
        };
    }
}

i32 create(bool fail) throws OpenError {
    Resource resource = Resource(6, fail);
    return resource.value;
}

i32 rethrow_as_base(bool fail) throws DomainError {
    try {
        return create(fail);
    } catch (const OpenError& error) {
        throw;
    }
}

i32 handle(bool fail) {
    try {
        return rethrow_as_base(fail);
    } catch (const DomainError& error) {
        return error.code + 1;
    }
}

} // namespace samples
"#;
    let result = transpile(source);
    assert!(
        result.analysis.diagnostics.is_empty(),
        "{:?}",
        result.analysis.diagnostics
    );
    let handle = result
        .analysis
        .semantics
        .functions
        .iter()
        .find(|function| function.path == ["samples", "handle"])
        .expect("handle symbol")
        .mangled_name
        .clone();
    let mut rust = result
        .rust
        .expect("throwing constructors and rethrows should emit Rust");
    write!(
        rust,
        r"
fn main() {{
    assert_eq!(__stainless_namespace_samples::{handle}(false), 6);
    assert_eq!(__stainless_namespace_samples::{handle}(true), 31);
}}
"
    )
    .expect("writing to a String cannot fail");
    let binary = compile_rust("throwing-constructors", &rust, CrateKind::Binary);
    let output = Command::new(&binary)
        .output()
        .expect("generated throwing-constructor program should run");
    assert!(
        output.status.success(),
        "status: {}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    remove_temporary_parent(&binary);
}

const NATIVE_RESULT_SOURCE: &str = r"use rust::{Result, String, Vec};

namespace samples {

struct Number {
    i32 value;
};

i32 explicit_unwrap(Result<i32, String> result) {
    try {
        return result.unwrap();
    } catch (const stainless::RustError& error) {
        return i32(error.message.len());
    }
}

i32 implicit_local(Result<i32, String> result) {
    try {
        i32 value = move(result);
        return value;
    } catch (const stainless::RustError& error) {
        return i32(error.message.len());
    }
}

i32 implicit_assignment(Result<i32, String> result) {
    try {
        i32 value = 0;
        value = move(result);
        return value;
    } catch (const stainless::RustError& error) {
        return i32(error.message.len());
    }
}

i32 implicit_aggregate(Result<i32, String> result) {
    try {
        Number number = Number{move(result)};
        return number.value;
    } catch (const stainless::RustError& error) {
        return i32(error.message.len());
    }
}

i32 implicit_direct(Result<Number, String> result) {
    try {
        Number number = Number(move(result));
        return number.value;
    } catch (const stainless::RustError& error) {
        return i32(error.message.len());
    }
}

i32 fallback_message(Result<i32, Vec<i32>> result) {
    try {
        return result.unwrap();
    } catch (const stainless::RustError& error) {
        return i32(error.message.len());
    }
}

} // namespace samples
";

#[test]
fn native_results_convert_to_checked_rust_errors_without_panicking() {
    let result = transpile(NATIVE_RESULT_SOURCE);
    assert!(
        result.analysis.diagnostics.is_empty(),
        "{:?}",
        result.analysis.diagnostics
    );
    let find_name = |source_name: &str| {
        result
            .analysis
            .semantics
            .functions
            .iter()
            .find(|function| function.path == ["samples", source_name])
            .unwrap_or_else(|| panic!("missing `{source_name}`"))
            .mangled_name
            .clone()
    };
    let explicit_unwrap = find_name("explicit_unwrap");
    let implicit_local = find_name("implicit_local");
    let implicit_assignment = find_name("implicit_assignment");
    let implicit_aggregate = find_name("implicit_aggregate");
    let implicit_direct = find_name("implicit_direct");
    let fallback_message = find_name("fallback_message");
    let mut rust = result
        .rust
        .expect("native Result adaptation should emit Rust");
    write!(
        rust,
        r#"
fn main() {{
    use ::std::result::Result::{{Err, Ok}};

    assert_eq!(__stainless_namespace_samples::{explicit_unwrap}(
        Ok(17),
    ), 17);
    assert_eq!(__stainless_namespace_samples::{explicit_unwrap}(
        Err(::std::string::String::from("oops")),
    ), 4);
    assert_eq!(__stainless_namespace_samples::{implicit_local}(
        Ok(19),
    ), 19);
    assert_eq!(__stainless_namespace_samples::{implicit_local}(
        Err(::std::string::String::from("local")),
    ), 5);
    assert_eq!(__stainless_namespace_samples::{implicit_assignment}(
        Ok(23),
    ), 23);
    assert_eq!(__stainless_namespace_samples::{implicit_assignment}(
        Err(::std::string::String::from("assign")),
    ), 6);
    assert_eq!(__stainless_namespace_samples::{implicit_aggregate}(
        Ok(29),
    ), 29);
    assert_eq!(__stainless_namespace_samples::{implicit_aggregate}(
        Err(::std::string::String::from("field")),
    ), 5);
    assert_eq!(__stainless_namespace_samples::{implicit_direct}(
        Ok(__stainless_namespace_samples::Number {{ value: 31 }}),
    ), 31);
    assert_eq!(__stainless_namespace_samples::{implicit_direct}(
        Err(::std::string::String::from("direct")),
    ), 6);
    assert_eq!(__stainless_namespace_samples::{fallback_message}(
        Err(::std::vec::Vec::from([1])),
    ), 28);
}}
"#
    )
    .expect("writing to a String cannot fail");
    let binary = compile_rust("native-result", &rust, CrateKind::Binary);
    let output = Command::new(&binary)
        .output()
        .expect("generated native Result program should run");
    assert!(
        output.status.success(),
        "status: {}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    remove_temporary_parent(&binary);
}

#[test]
fn cargo_validates_generated_external_regex_wrappers() {
    let source = include_str!("../../../docs/ref/17_external_regex_wrapper.stl");
    let external = load_bindings_manifest(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../docs/ref/17_external_regex_wrapper.bindings.toml"),
    )
    .unwrap();
    let bindings = standard_bindings().unwrap().merge(external).unwrap();
    let result = transpile_with_bindings(source, &bindings);
    assert!(
        result.analysis.diagnostics.is_empty(),
        "{:?}",
        result.analysis.diagnostics
    );
    let regex_matches = result
        .analysis
        .semantics
        .functions
        .iter()
        .find(|function| function.path == ["samples", "regex_matches"])
        .expect("regex_matches symbol")
        .mangled_name
        .clone();
    let invalid_regex_message = result
        .analysis
        .semantics
        .functions
        .iter()
        .find(|function| function.path == ["samples", "invalid_regex_message"])
        .expect("invalid_regex_message symbol")
        .mangled_name
        .clone();
    let hir = result.hir.as_ref().expect("external wrapper HIR");
    assert_eq!(hir.native_wrappers.len(), 2);
    let mut rust = result.rust.expect("external wrappers should emit Rust");
    write!(
        rust,
        r#"
fn main() {{
    let matching = ::std::string::String::from("stainless");
    let different = ::std::string::String::from("steel");
    assert!(__stainless_namespace_samples::{regex_matches}(&matching));
    assert!(!__stainless_namespace_samples::{regex_matches}(&different));
    assert!(
        __stainless_namespace_samples::{invalid_regex_message}() > 0
    );
}}
"#
    )
    .expect("writing to a String cannot fail");

    let directory = write_external_cargo_fixture("regex-wrapper", &rust);
    let valid = run_fixture_cargo(&directory, "run");
    assert!(
        valid.status.success(),
        "Cargo rejected valid generated wrappers:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&valid.stdout),
        String::from_utf8_lossy(&valid.stderr)
    );

    let stale = rust.replace("::regex::Regex::new", "::regex::Regex::compile");
    assert_ne!(stale, rust, "generated wrapper target should be present");
    fs::write(directory.join("src/main.rs"), stale)
        .expect("stale generated fixture source should be writable");
    let invalid = run_fixture_cargo(&directory, "check");
    assert!(
        !invalid.status.success(),
        "Cargo unexpectedly accepted a stale external wrapper"
    );
    assert!(
        String::from_utf8_lossy(&invalid.stderr).contains("compile"),
        "Cargo diagnostic should name the stale item:\n{}",
        String::from_utf8_lossy(&invalid.stderr)
    );
    fs::remove_dir_all(&directory)
        .unwrap_or_else(|error| panic!("failed to remove {}: {error}", directory.display()));
}

#[test]
fn cargo_validates_generated_non_escaping_callback_wrappers() {
    let source = include_str!("../../../docs/ref/18_external_callbacks.stl");
    let external = load_bindings_manifest(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../docs/ref/18_external_callbacks.bindings.toml"),
    )
    .unwrap();
    let bindings = standard_bindings().unwrap().merge(external).unwrap();
    let result = transpile_with_bindings(source, &bindings);
    assert!(
        result.analysis.diagnostics.is_empty(),
        "{:?}",
        result.analysis.diagnostics
    );
    let external_callbacks = result
        .analysis
        .semantics
        .functions
        .iter()
        .find(|function| function.path == ["samples", "external_callbacks"])
        .expect("external_callbacks symbol")
        .mangled_name
        .clone();
    let hir = result.hir.as_ref().expect("callback wrapper HIR");
    assert_eq!(hir.native_wrappers.len(), 5);
    let mut rust = result.rust.expect("callback wrappers should emit Rust");
    write!(
        rust,
        r"
fn main() {{
    assert_eq!(
        __stainless_namespace_samples::{external_callbacks}(),
        846526,
    );
}}
"
    )
    .expect("writing to a String cannot fail");

    let directory = write_callback_cargo_fixture("callbacks", &rust);
    let output = run_fixture_cargo(&directory, "run");
    assert!(
        output.status.success(),
        "Cargo rejected generated callback wrappers:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    fs::remove_dir_all(&directory)
        .unwrap_or_else(|error| panic!("failed to remove {}: {error}", directory.display()));
}

#[test]
fn rustc_validates_stored_function_and_function_mut_behavior() {
    let source = include_str!("../../../docs/ref/19_stored_functions.stl");
    let result = transpile(source);
    assert!(
        result.analysis.diagnostics.is_empty(),
        "{:?}",
        result.analysis.diagnostics
    );
    let function = result
        .analysis
        .semantics
        .functions
        .iter()
        .find(|function| function.path == ["samples", "stored_functions"])
        .expect("stored function sample symbol")
        .mangled_name
        .clone();
    let mut rust = result.rust.expect("stored callables should emit Rust");
    write!(
        rust,
        "\nfn main() {{ assert_eq!(__stainless_namespace_samples::{function}(), 54); }}\n"
    )
    .expect("writing to a String cannot fail");
    let binary = compile_rust("stored-functions", &rust, CrateKind::Binary);
    let output = Command::new(&binary)
        .output()
        .expect("generated stored-function binary should run");
    assert!(output.status.success());
    remove_temporary_parent(&binary);
}

#[test]
fn cargo_executes_native_json_support_and_json_error_conversion() {
    let source = include_str!("../../../docs/ref/21_json_support.stl");
    let result = transpile(source);
    assert!(
        result.analysis.diagnostics.is_empty(),
        "{:?}",
        result.analysis.diagnostics
    );
    let entry = result
        .analysis
        .semantics
        .functions
        .iter()
        .find(|function| function.path == ["main"])
        .expect("JSON sample main symbol")
        .mangled_name
        .clone();
    let mut rust = result.rust.expect("JSON sample should emit Rust");
    assert!(rust.contains("::stainless_runtime::Var::object"));
    assert!(rust.contains("::stainless_runtime::Var::parse"));
    assert!(rust.contains("__stainless_namespace_stainless::JsonError"));
    write!(
        rust,
        "\nfn main() {{ assert_eq!({entry}().expect(\"valid JSON\"), 0); }}\n"
    )
    .expect("writing to a String cannot fail");

    let directory = write_runtime_cargo_fixture("json-runtime", &rust);
    let output = run_fixture_cargo(&directory, "run");
    assert!(
        output.status.success(),
        "Cargo rejected generated JSON support:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    fs::remove_dir_all(&directory)
        .unwrap_or_else(|error| panic!("failed to remove {}: {error}", directory.display()));
}

#[test]
fn generated_println_macro_executes_with_stainless_values() {
    let source = r#"use rust::println;

void say_hello(i32 value) {
    println!("Hello, {}!", value);
    rust::println!();
}
"#;
    let result = transpile(source);
    assert!(
        result.analysis.diagnostics.is_empty(),
        "{:?}",
        result.analysis.diagnostics
    );
    let function = result
        .analysis
        .semantics
        .functions
        .iter()
        .find(|function| function.path == ["say_hello"])
        .expect("println sample symbol")
        .mangled_name
        .clone();
    let mut rust = result.rust.expect("println should emit Rust");
    write!(rust, "\nfn main() {{ {function}(7); }}\n").expect("writing to a String cannot fail");
    let binary = compile_rust("println", &rust, CrateKind::Binary);
    let output = Command::new(&binary)
        .output()
        .expect("generated println binary should run");
    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "Hello, 7!\n\n");
    remove_temporary_parent(&binary);
}

#[test]
fn generated_formatting_and_string_write_macros_execute() {
    let source = r#"use rust::{eprintln, format, write, writeln, String};

String render(i32 value) {
    String output = format!("value={}, next={}", value, value + 1);
    try {
        write!(output, "!");
        writeln!(output);
    } catch (const stainless::FormatError& error) {
        return format!("formatting failed: {}", error.message);
    }
    return move(output);
}

void report(const String& value) {
    eprintln!("report: {}", value);
}
"#;
    let result = transpile(source);
    assert!(
        result.analysis.diagnostics.is_empty(),
        "{:?}",
        result.analysis.diagnostics
    );
    let find_name = |name: &str| {
        result
            .analysis
            .semantics
            .functions
            .iter()
            .find(|function| function.path == [name])
            .unwrap_or_else(|| panic!("missing `{name}`"))
            .mangled_name
            .clone()
    };
    let render = find_name("render");
    let report = find_name("report");
    let mut rust = result.rust.expect("formatting macros should emit Rust");
    assert!(rust.contains("FormatError"));
    write!(
        rust,
        "\nfn main() {{\n    let rendered = {render}(7);\n    assert_eq!(rendered, \"value=7, next=8!\\n\");\n    {report}(&rendered);\n}}\n"
    )
    .expect("writing to a String cannot fail");
    let binary = compile_rust("formatting_macros", &rust, CrateKind::Binary);
    let output = Command::new(&binary)
        .output()
        .expect("generated formatting macro binary should run");
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        "report: value=7, next=8!\n\n"
    );
    remove_temporary_parent(&binary);
}

#[test]
fn invalid_checked_exception_prevents_rust_emission() {
    let result = transpile(
        r"struct Failure {};

i32 load() throws Failure {
    return 1;
}
",
    );

    assert!(result.hir.is_none());
    assert!(result.rust.is_none());
    assert!(result.analysis.diagnostics.iter().any(|diagnostic| {
        diagnostic.phase == DiagnosticPhase::Semantic && diagnostic.code == "RES070"
    }));
}

#[derive(Clone, Copy)]
enum CrateKind {
    Binary,
    Library,
}

fn compile_rust(name: &str, source: &str, kind: CrateKind) -> PathBuf {
    let directory = temporary_directory(name);
    fs::create_dir_all(&directory)
        .unwrap_or_else(|error| panic!("failed to create {}: {error}", directory.display()));
    let source_path = directory.join("generated.rs");
    fs::write(&source_path, source)
        .unwrap_or_else(|error| panic!("failed to write {}: {error}", source_path.display()));
    let output_path = match kind {
        CrateKind::Binary => directory.join("generated"),
        CrateKind::Library => directory.join("libgenerated.rlib"),
    };
    let rustc = std::env::var_os("RUSTC").unwrap_or_else(|| OsString::from("rustc"));
    let mut command = Command::new(rustc);
    command
        .arg("--edition=2024")
        .arg("--crate-name")
        .arg("stainless_generated")
        .arg("-Dwarnings")
        .arg(&source_path)
        .arg("-o")
        .arg(&output_path);
    if matches!(kind, CrateKind::Library) {
        command.arg("--crate-type=lib");
    }
    let output = command
        .output()
        .unwrap_or_else(|error| panic!("failed to invoke rustc: {error}"));
    assert!(
        output.status.success(),
        "rustc rejected generated Rust for {name}:\n{}\nstdout:\n{}\nstderr:\n{}",
        source,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    if matches!(kind, CrateKind::Library) {
        remove_temporary_parent(&output_path);
    }
    output_path
}

fn temporary_directory(name: &str) -> PathBuf {
    let index = TEMPORARY_INDEX.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "stainless-transpilation-{}-{index}-{name}",
        std::process::id()
    ))
}

fn write_external_cargo_fixture(name: &str, source: &str) -> PathBuf {
    let directory = temporary_directory(name);
    let source_directory = directory.join("src");
    fs::create_dir_all(&source_directory)
        .unwrap_or_else(|error| panic!("failed to create {}: {error}", source_directory.display()));
    fs::write(
        directory.join("Cargo.toml"),
        r#"[package]
name = "stainless-generated-external-fixture"
version = "0.0.0"
edition = "2024"

[dependencies]
regex = "1.12.4"

[lints.rust]
warnings = "deny"
"#,
    )
    .expect("external fixture manifest should be writable");
    fs::write(source_directory.join("main.rs"), source)
        .expect("external fixture source should be writable");
    directory
}

fn write_callback_cargo_fixture(name: &str, source: &str) -> PathBuf {
    let directory = temporary_directory(name);
    let source_directory = directory.join("src");
    let dependency_directory = directory.join("callback-fixture");
    let dependency_source_directory = dependency_directory.join("src");
    fs::create_dir_all(&source_directory)
        .unwrap_or_else(|error| panic!("failed to create {}: {error}", source_directory.display()));
    fs::create_dir_all(&dependency_source_directory).unwrap_or_else(|error| {
        panic!(
            "failed to create {}: {error}",
            dependency_source_directory.display()
        )
    });
    fs::write(
        directory.join("Cargo.toml"),
        r#"[package]
name = "stainless-generated-callback-fixture"
version = "0.0.0"
edition = "2024"

[dependencies]
callback_fixture = { path = "callback-fixture" }

[lints.rust]
warnings = "deny"
"#,
    )
    .expect("callback fixture manifest should be writable");
    fs::write(
        dependency_directory.join("Cargo.toml"),
        r#"[package]
name = "callback_fixture"
version = "0.0.0"
edition = "2024"

[lints.rust]
warnings = "deny"
"#,
    )
    .expect("callback dependency manifest should be writable");
    fs::write(
        dependency_source_directory.join("lib.rs"),
        r"pub struct Processor {
    value: i32,
}

impl Processor {
    pub fn new(value: i32) -> Self {
        Self { value }
    }

    pub fn apply<F>(&mut self, input: i32, mut callback: F) -> i32
    where
        F: FnMut(i32) -> i32,
    {
        self.value = callback(input);
        self.value
    }

    pub fn inspect<F>(&self, input: i32, callback: F) -> i32
    where
        F: Fn(i32) -> i32,
    {
        self.value + callback(input)
    }

    pub fn consume<F>(self, callback: F) -> i32
    where
        F: FnOnce(i32) -> i32,
    {
        callback(self.value)
    }

    pub fn apply_fn_ptr(&self, input: i32, callback: fn(i32) -> i32) -> i32 {
        self.value + callback(input)
    }
}
",
    )
    .expect("callback dependency source should be writable");
    fs::write(source_directory.join("main.rs"), source)
        .expect("callback fixture source should be writable");
    directory
}

fn write_runtime_cargo_fixture(name: &str, source: &str) -> PathBuf {
    let directory = temporary_directory(name);
    let source_directory = directory.join("src");
    fs::create_dir_all(&source_directory)
        .unwrap_or_else(|error| panic!("failed to create {}: {error}", source_directory.display()));
    let runtime = Path::new(env!("CARGO_MANIFEST_DIR")).join("../stainless-runtime");
    fs::write(
        directory.join("Cargo.toml"),
        format!(
            r#"[package]
name = "stainless-generated-json-fixture"
version = "0.0.0"
edition = "2024"

[dependencies]
stainless-runtime = {{ path = "{}" }}

[lints.rust]
warnings = "deny"
"#,
            runtime.display()
        ),
    )
    .expect("JSON fixture manifest should be writable");
    fs::write(source_directory.join("main.rs"), source)
        .expect("JSON fixture source should be writable");
    directory
}

fn run_fixture_cargo(directory: &Path, command: &str) -> std::process::Output {
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| OsString::from("cargo"));
    Command::new(cargo)
        .arg(command)
        .arg("--quiet")
        .arg("--offline")
        .arg("--manifest-path")
        .arg(directory.join("Cargo.toml"))
        .env("CARGO_TARGET_DIR", directory.join("target"))
        .output()
        .unwrap_or_else(|error| panic!("failed to invoke Cargo: {error}"))
}

fn remove_temporary_parent(path: &Path) {
    let parent = path.parent().expect("temporary output has a parent");
    fs::remove_dir_all(parent)
        .unwrap_or_else(|error| panic!("failed to remove {}: {error}", parent.display()));
}
