use std::ffi::OsString;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

use stainless_compiler::{DiagnosticPhase, transpile};

static TEMPORARY_INDEX: AtomicUsize = AtomicUsize::new(0);

const BEHAVIOR_SOURCE: &str = r#"use rust::{String, Vec};

namespace samples {

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
            "vec_and_string",
            include_str!("../../../docs/ref/11_vec_and_string.stl"),
        ),
        (
            "range_for",
            include_str!("../../../docs/ref/13_range_for.stl"),
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
fn unsupported_backend_forms_fail_without_emitting_rust() {
    let result = transpile(
        r"i32 load() throws Failure {
    return 1;
}
",
    );

    assert!(result.hir.is_none());
    assert!(result.rust.is_none());
    assert!(result.analysis.diagnostics.iter().any(|diagnostic| {
        diagnostic.phase == DiagnosticPhase::Hir && diagnostic.code == "HIR001"
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

fn remove_temporary_parent(path: &Path) {
    let parent = path.parent().expect("temporary output has a parent");
    fs::remove_dir_all(parent)
        .unwrap_or_else(|error| panic!("failed to remove {}: {error}", parent.display()));
}
