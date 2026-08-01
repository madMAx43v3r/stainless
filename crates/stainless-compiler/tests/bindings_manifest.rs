use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use stainless_compiler::interop::{
    ArgumentAdaptation, BINDINGS_MANIFEST_FILENAME, CallStyle, CallbackEscape, CallbackKind,
    NativeErrorFormat, Receiver, RustLowering, TypeRef, WrapperTarget, load_bindings_manifest,
    load_package_bindings, parse_bindings_manifest, standard_bindings,
};

static TEMPORARY_INDEX: AtomicUsize = AtomicUsize::new(0);

const REGEX_MANIFEST: &str =
    include_str!("../../../docs/ref/17_external_regex_wrapper.bindings.toml");

#[test]
fn parses_owned_external_types_and_exact_callable_signatures() {
    let source = REGEX_MANIFEST.to_owned();
    let bindings = parse_bindings_manifest(&source).unwrap();
    drop(source);

    let paths = bindings
        .types()
        .map(|binding| binding.stainless_path.as_str())
        .collect::<Vec<_>>();
    assert_eq!(paths, ["rust::regex::Error", "rust::regex::Regex"]);

    let error = bindings.type_by_path("rust::regex::Error").unwrap();
    assert_eq!(error.rust_path, "::regex::Error");
    assert_eq!(error.error_format, Some(NativeErrorFormat::Display));

    let regex = bindings.type_by_path("rust::regex::Regex").unwrap();
    let string_ref = TypeRef::shared_ref(TypeRef::native("rust::String", vec![]));
    let new = regex
        .find_callable(
            CallStyle::AssociatedFunction,
            "new",
            std::slice::from_ref(&string_ref),
        )
        .unwrap();
    assert_eq!(
        new.return_type,
        TypeRef::native(
            "rust::Result",
            vec![
                TypeRef::native("rust::regex::Regex", vec![]),
                TypeRef::native("rust::regex::Error", vec![]),
            ],
        )
    );
    assert_eq!(
        new.parameters[0].adaptation,
        ArgumentAdaptation::StringRefToStr
    );

    let is_match = regex
        .find_callable(
            CallStyle::Method,
            "is_match",
            std::slice::from_ref(&string_ref),
        )
        .unwrap();
    assert_eq!(is_match.receiver, Some(Receiver::Shared));
    assert_eq!(is_match.return_type, TypeRef::Bool);
}

#[test]
fn package_loader_merges_optional_manifest_with_compiler_builtins() {
    let empty_package = TemporaryDirectory::new("empty-bindings");
    let builtins = load_package_bindings(empty_package.path()).unwrap();
    assert_eq!(builtins, standard_bindings().unwrap());

    let package = TemporaryDirectory::new("regex-bindings");
    fs::write(
        package.path().join(BINDINGS_MANIFEST_FILENAME),
        REGEX_MANIFEST,
    )
    .unwrap();
    let merged = load_package_bindings(package.path()).unwrap();
    let paths = merged
        .types()
        .map(|binding| binding.stainless_path.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        paths,
        [
            "rust::List",
            "rust::Map",
            "rust::Queue",
            "rust::Set",
            "rust::String",
            "rust::Vec",
            "rust::regex::Error",
            "rust::regex::Regex",
            "rust::stainless_runtime::JsonError",
            "rust::stainless_runtime::Var",
            "rust::std::fs",
            "rust::std::io::Error",
        ]
    );
}

#[test]
fn file_loader_reports_the_manifest_path() {
    let package = TemporaryDirectory::new("invalid-bindings");
    let path = package.path().join(BINDINGS_MANIFEST_FILENAME);
    fs::write(&path, "schema = 2\n").unwrap();

    let error = load_bindings_manifest(&path).unwrap_err();
    assert_eq!(error.path(), Some(path.as_path()));
    assert!(error.to_string().contains("unsupported bindings schema 2"));
}

#[test]
fn package_loader_does_not_treat_an_unreadable_manifest_path_as_missing() {
    let package = TemporaryDirectory::new("non-file-bindings");
    let path = package.path().join(BINDINGS_MANIFEST_FILENAME);
    fs::create_dir(&path).unwrap();

    let error = load_package_bindings(package.path()).unwrap_err();
    assert_eq!(error.path(), Some(path.as_path()));
    assert!(error.message_text().contains("failed to read"));
}

#[test]
fn rejects_unknown_fields_with_a_toml_span() {
    let error =
        parse_bindings_manifest("schema = 1\nunexpected = true\n").expect_err("unknown field");

    assert!(error.message_text().contains("unknown field"));
    assert!(error.span().is_some());
}

#[test]
fn free_rust_function_can_back_a_stainless_associated_function() {
    let bindings = parse_bindings_manifest(
        r#"schema = 1

[[type]]
dependency = "example"
rust_path = "example::Parser"
stainless_path = "rust::example::Parser"
representation = "opaque"

[[function]]
dependency = "example"
rust_path = "example::parse"
stainless_path = "rust::example::Parser::parse"
parameters = []
return = "rust::example::Parser"
"#,
    )
    .unwrap();
    let parser = bindings.type_by_path("rust::example::Parser").unwrap();
    let parse = parser
        .find_callable(CallStyle::AssociatedFunction, "parse", &[])
        .unwrap();

    assert!(matches!(
        &parse.lowering,
        RustLowering::GeneratedWrapper {
            target: WrapperTarget::Function { rust_path },
            ..
        } if rust_path == "::example::parse"
    ));
}

#[test]
fn parses_non_escaping_callback_parameter_metadata() {
    let bindings = parse_bindings_manifest(
        r#"schema = 1

[[type]]
dependency = "example"
rust_path = "example::Processor"
stainless_path = "rust::example::Processor"
representation = "opaque"

[[method]]
receiver_type = "rust::example::Processor"
rust_name = "apply"
stainless_name = "apply"
receiver = "mut"
parameters = [
    { callback = {
        kind = "fn_mut",
        parameters = ["i32"],
        return = "i32",
        escape = "call"
    } }
]
return = "i32"
"#,
    )
    .unwrap();
    let processor = bindings.type_by_path("rust::example::Processor").unwrap();
    let callback = TypeRef::callback(
        CallbackKind::FnMut,
        CallbackEscape::Call,
        vec![TypeRef::I32],
        TypeRef::I32,
    );

    assert!(
        processor
            .find_callable(CallStyle::Method, "apply", &[callback])
            .is_some()
    );
}

#[test]
fn accepts_thread_callbacks_but_rejects_general_static_and_reference_returns() {
    let manifest = |escape: &str, return_type: &str| {
        format!(
            r#"schema = 1

[[type]]
dependency = "example"
rust_path = "example::Processor"
stainless_path = "rust::example::Processor"
representation = "opaque"

[[method]]
receiver_type = "rust::example::Processor"
rust_name = "apply"
stainless_name = "apply"
receiver = "const"
parameters = [
    {{ callback = {{
        kind = "fn",
        parameters = [],
        return = "{return_type}",
        escape = "{escape}"
    }} }}
]
return = "void"
"#
        )
    };

    let threaded = parse_bindings_manifest(&manifest("thread", "void"))
        .expect("thread callbacks should be supported");
    let callback = &threaded
        .type_by_path("rust::example::Processor")
        .expect("processor binding")
        .callables[0]
        .parameters[0]
        .ty;
    assert!(matches!(
        callback,
        TypeRef::Callback(callback) if callback.escape == CallbackEscape::Thread
    ));

    let escaping = parse_bindings_manifest(&manifest("static", "void")).unwrap_err();
    assert!(escaping.message_text().contains("escape = \"static\""));

    let borrowed = parse_bindings_manifest(&manifest("call", "const rust::String&")).unwrap_err();
    assert!(
        borrowed
            .message_text()
            .contains("callback return references")
    );
}

#[test]
fn rejects_overloads_that_differ_only_by_callback_kind() {
    let error = parse_bindings_manifest(
        r#"schema = 1

[[type]]
dependency = "example"
rust_path = "example::Processor"
stainless_path = "rust::example::Processor"
representation = "opaque"

[[method]]
receiver_type = "rust::example::Processor"
rust_name = "apply_fn"
stainless_name = "apply"
receiver = "const"
parameters = [
    { callback = {
        kind = "fn",
        parameters = ["i32"],
        return = "i32",
        escape = "call"
    } }
]
return = "i32"

[[method]]
receiver_type = "rust::example::Processor"
rust_name = "apply_fn_mut"
stainless_name = "apply"
receiver = "const"
parameters = [
    { callback = {
        kind = "fn_mut",
        parameters = ["i32"],
        return = "i32",
        escape = "call"
    } }
]
return = "i32"
"#,
    )
    .unwrap_err();

    assert!(
        error
            .message_text()
            .contains("cannot be overloaded only by callback kind")
    );
}

#[test]
fn rejects_unsupported_schema_and_initial_representation() {
    let schema = parse_bindings_manifest("schema = 9\n").unwrap_err();
    assert!(schema.message_text().contains("expected 1"));

    let adapter = parse_bindings_manifest(
        r#"schema = 1

[[type]]
dependency = "example"
rust_path = "example::Value"
stainless_path = "rust::example::Value"
representation = "frozen_adapter"
"#,
    )
    .unwrap_err();
    assert!(
        adapter
            .message_text()
            .contains("only `opaque` is implemented")
    );
}

#[test]
fn rejects_dependency_escape_and_undeclared_signature_types() {
    let escaped = parse_bindings_manifest(
        r#"schema = 1

[[type]]
dependency = "example"
rust_path = "other::Value"
stainless_path = "rust::example::Value"
representation = "opaque"
"#,
    )
    .unwrap_err();
    assert!(
        escaped
            .message_text()
            .contains("outside dependency `example`")
    );

    let undeclared = parse_bindings_manifest(
        r#"schema = 1

[[type]]
dependency = "example"
rust_path = "example::Value"
stainless_path = "rust::example::Value"
representation = "opaque"

[[function]]
dependency = "example"
rust_path = "example::Value::new"
stainless_path = "rust::example::Value::new"
parameters = ["rust::missing::Input"]
return = "rust::example::Value"
"#,
    )
    .unwrap_err();
    assert!(
        undeclared
            .message_text()
            .contains("undeclared native type `rust::missing::Input`")
    );
}

#[test]
fn rejects_invalid_paths_and_reference_bearing_parameter_values() {
    let invalid_dependency = parse_bindings_manifest(
        r#"schema = 1

[[type]]
dependency = "../example"
rust_path = "example::Value"
stainless_path = "rust::example::Value"
representation = "opaque"
"#,
    )
    .unwrap_err();
    assert!(
        invalid_dependency
            .message_text()
            .contains("invalid Cargo dependency key")
    );

    let keyword_path = parse_bindings_manifest(
        r#"schema = 1

[[type]]
dependency = "example"
rust_path = "example::Value"
stainless_path = "rust::example::for"
representation = "opaque"
"#,
    )
    .unwrap_err();
    assert!(
        keyword_path
            .message_text()
            .contains("not valid Stainless syntax")
    );

    let nested_reference = parse_bindings_manifest(
        r#"schema = 1

[[type]]
dependency = "example"
rust_path = "example::Value"
stainless_path = "rust::example::Value"
representation = "opaque"

[[function]]
dependency = "example"
rust_path = "example::Value::read"
stainless_path = "rust::example::Value::read"
parameters = ["const rust::Option<const rust::String&>&"]
return = "i32"
"#,
    )
    .unwrap_err();
    assert!(
        nested_reference
            .message_text()
            .contains("reference-bearing parameter values")
    );
}

struct TemporaryDirectory {
    path: PathBuf,
}

impl TemporaryDirectory {
    fn new(label: &str) -> Self {
        let index = TEMPORARY_INDEX.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "stainless-bindings-{label}-{}-{index}",
            std::process::id()
        ));
        fs::create_dir(&path).unwrap();
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TemporaryDirectory {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.path).unwrap();
    }
}
