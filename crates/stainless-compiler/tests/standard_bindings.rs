use std::collections::BTreeSet;

use stainless_compiler::interop::{
    ArgumentAdaptation, BindingError, CallStyle, CallableBinding, CallbackKind, NativeBindings,
    NativeErrorFormat, NativeTypeBinding, Receiver, RustLowering, TypeRef, WrapperTarget,
    parse_bindings_manifest, standard_bindings,
};

#[test]
fn standard_registry_contains_builtin_types_in_path_order() {
    let bindings = standard_bindings().unwrap();
    let paths = bindings
        .types()
        .map(|binding| binding.stainless_path.as_str())
        .collect::<Vec<_>>();

    assert_eq!(
        paths,
        [
            "rust::List",
            "rust::Map",
            "rust::MultiMap",
            "rust::Queue",
            "rust::Set",
            "rust::String",
            "rust::Vec",
            "rust::stainless_runtime::BigEndian",
            "rust::stainless_runtime::JsonError",
            "rust::stainless_runtime::LittleEndian",
            "rust::stainless_runtime::Var",
            "rust::std::fs",
            "rust::std::fs::File",
            "rust::std::fs::OpenOptions",
            "rust::std::io::Error",
        ]
    );
}

#[test]
fn endian_bindings_use_exact_byte_widths_and_checked_reads() {
    let bindings = standard_bindings().unwrap();
    let bytes = TypeRef::native("rust::Vec", vec![TypeRef::U8]);
    let io_error = TypeRef::native("rust::std::io::Error", vec![]);

    for endian_name in ["BigEndian", "LittleEndian"] {
        let endian = bindings
            .type_by_path(&format!("rust::stainless_runtime::{endian_name}"))
            .unwrap();
        let write_u32 = endian
            .find_callable(
                CallStyle::AssociatedFunction,
                "write_u32",
                &[TypeRef::mutable_ref(bytes.clone()), TypeRef::U32],
            )
            .unwrap();
        assert_eq!(write_u32.return_type, TypeRef::Void);
        assert_eq!(write_u32.rust_result_error, None);

        let write_usize_u32 = endian
            .find_callable(
                CallStyle::AssociatedFunction,
                "write_u32",
                &[TypeRef::mutable_ref(bytes.clone()), TypeRef::Usize],
            )
            .unwrap();
        assert_eq!(write_usize_u32.return_type, TypeRef::Void);
        assert_eq!(write_usize_u32.rust_result_error, Some(io_error.clone()));

        let write_usize_u64 = endian
            .find_callable(
                CallStyle::AssociatedFunction,
                "write_u64",
                &[TypeRef::mutable_ref(bytes.clone()), TypeRef::Usize],
            )
            .unwrap();
        assert_eq!(write_usize_u64.return_type, TypeRef::Void);
        assert_eq!(write_usize_u64.rust_result_error, None);

        for (name, result) in [
            ("read_u8", TypeRef::U8),
            ("read_u32", TypeRef::U32),
            ("read_u64", TypeRef::U64),
        ] {
            let read = endian
                .find_callable(
                    CallStyle::AssociatedFunction,
                    name,
                    &[TypeRef::shared_ref(bytes.clone())],
                )
                .unwrap();
            assert_eq!(read.return_type, result);
            assert_eq!(read.rust_result_error, Some(io_error.clone()));

            let read_at = endian
                .find_callable(
                    CallStyle::AssociatedFunction,
                    &format!("{name}_at"),
                    &[TypeRef::shared_ref(bytes.clone()), TypeRef::Usize],
                )
                .unwrap();
            assert_eq!(read_at.return_type, result);
            assert_eq!(read_at.rust_result_error, Some(io_error.clone()));
        }
    }
}

#[test]
fn positioned_file_binding_uses_one_shared_cursor_free_handle() {
    let bindings = standard_bindings().unwrap();
    let file = bindings.type_by_path("rust::std::fs::File").unwrap();
    let file_type = TypeRef::native("rust::std::fs::File", vec![]);
    let string_ref = TypeRef::shared_ref(TypeRef::native("rust::String", vec![]));
    let io_error = TypeRef::native("rust::std::io::Error", vec![]);

    assert_eq!(file.rust_path, "::stainless_runtime::PositionedFile");
    let open = file
        .find_callable(CallStyle::AssociatedFunction, "open", &[string_ref])
        .unwrap();
    assert_eq!(open.return_type, file_type);
    assert_eq!(open.rust_result_error, Some(io_error.clone()));

    let pread = file
        .find_callable(CallStyle::Method, "pread", &[TypeRef::U64, TypeRef::Usize])
        .unwrap();
    assert_eq!(pread.receiver, Some(Receiver::Shared));
    assert_eq!(
        pread.return_type,
        TypeRef::native("rust::Vec", vec![TypeRef::U8])
    );
    assert_eq!(pread.rust_result_error, Some(io_error));

    for name in [
        "pwrite",
        "pread_exact",
        "pwrite_all",
        "sync_all",
        "sync_data",
        "set_len",
        "len",
        "is_empty",
        "try_clone",
    ] {
        assert!(
            file.callables
                .iter()
                .any(|callable| callable.source_name == name),
            "missing File.{name}"
        );
    }

    let options = bindings.type_by_path("rust::std::fs::OpenOptions").unwrap();
    for name in [
        "read",
        "write",
        "append",
        "truncate",
        "create",
        "create_new",
        "open",
    ] {
        assert!(
            options
                .callables
                .iter()
                .any(|callable| callable.source_name == name),
            "missing OpenOptions.{name}"
        );
    }
}

#[test]
fn filesystem_bindings_have_exact_checked_text_and_byte_overloads() {
    let bindings = standard_bindings().unwrap();
    let fs = bindings.type_by_path("rust::std::fs").unwrap();
    let io_error = TypeRef::native("rust::std::io::Error", vec![]);
    let string_ref = TypeRef::shared_ref(TypeRef::native("rust::String", vec![]));
    let bytes_ref = TypeRef::shared_ref(TypeRef::native("rust::Vec", vec![TypeRef::U8]));

    assert_eq!(fs.rust_path, "::stainless_runtime::Fs");
    let write_text = fs
        .find_callable(
            CallStyle::AssociatedFunction,
            "write",
            &[string_ref.clone(), string_ref.clone()],
        )
        .unwrap();
    let write_bytes = fs
        .find_callable(
            CallStyle::AssociatedFunction,
            "write",
            &[string_ref.clone(), bytes_ref],
        )
        .unwrap();
    assert_eq!(write_text.rust_result_error, Some(io_error.clone()));
    assert_eq!(write_bytes.rust_result_error, Some(io_error));
    assert_eq!(
        write_text
            .parameters
            .iter()
            .map(|parameter| parameter.adaptation)
            .collect::<Vec<_>>(),
        [
            ArgumentAdaptation::StringRefToStr,
            ArgumentAdaptation::StringRefToStr,
        ]
    );
    assert!(matches!(
        write_text.lowering,
        RustLowering::AssociatedFunction { ref rust_path }
            if rust_path == "::stainless_runtime::Fs::write_text"
    ));
    assert!(matches!(
        write_bytes.lowering,
        RustLowering::AssociatedFunction { ref rust_path }
            if rust_path == "::stainless_runtime::Fs::write_bytes"
    ));
    assert_eq!(
        bindings
            .type_by_path("rust::std::io::Error")
            .unwrap()
            .error_format,
        Some(NativeErrorFormat::Display)
    );
}

#[test]
fn collection_bindings_use_ordered_and_sequence_rust_representations() {
    let bindings = standard_bindings().unwrap();
    let expected = [
        ("rust::List", "::std::collections::LinkedList", vec!["T"]),
        ("rust::Map", "::std::collections::BTreeMap", vec!["K", "V"]),
        (
            "rust::MultiMap",
            "::stainless_runtime::MultiMap",
            vec!["K", "V"],
        ),
        ("rust::Queue", "::std::collections::VecDeque", vec!["T"]),
        ("rust::Set", "::std::collections::BTreeSet", vec!["T"]),
    ];

    for (stainless_path, rust_path, parameters) in expected {
        let binding = bindings.type_by_path(stainless_path).unwrap();
        assert_eq!(binding.rust_path, rust_path);
        assert_eq!(binding.type_parameters, parameters);
        let short_name = stainless_path.rsplit("::").next().unwrap();
        let constructor = binding
            .find_callable(CallStyle::Constructor, short_name, &[])
            .unwrap();
        assert!(matches!(
            constructor.lowering,
            RustLowering::AssociatedFunction { .. }
        ));
    }
}

#[test]
fn ordered_collections_preserve_ord_requirements() {
    let bindings = standard_bindings().unwrap();
    let map = bindings.type_by_path("rust::Map").unwrap();
    let multimap = bindings.type_by_path("rust::MultiMap").unwrap();
    let set = bindings.type_by_path("rust::Set").unwrap();
    let k = TypeRef::Parameter("K".to_owned());
    let v = TypeRef::Parameter("V".to_owned());
    let t = TypeRef::Parameter("T".to_owned());

    let map_insert = map
        .find_callable(CallStyle::Method, "insert", &[k.clone(), v.clone()])
        .unwrap();
    assert_eq!(
        map_insert.return_type,
        TypeRef::native("rust::Option", vec![v.clone()])
    );
    assert_eq!(map_insert.requirements[0].parameter, "K");
    assert_eq!(map_insert.requirements[0].rust_trait, "::core::cmp::Ord");

    let multimap_insert = multimap
        .find_callable(CallStyle::Method, "insert", &[k, v])
        .unwrap();
    assert_eq!(multimap_insert.return_type, TypeRef::Void);
    assert_eq!(multimap_insert.requirements[0].parameter, "K");
    assert_eq!(
        multimap_insert.requirements[0].rust_trait,
        "::core::cmp::Ord"
    );

    let set_insert = set
        .find_callable(CallStyle::Method, "insert", std::slice::from_ref(&t))
        .unwrap();
    assert_eq!(set_insert.return_type, TypeRef::Bool);
    assert_eq!(set_insert.requirements[0].parameter, "T");
    assert_eq!(set_insert.requirements[0].rust_trait, "::core::cmp::Ord");
}

#[test]
fn ordered_map_range_and_multimap_callbacks_are_non_escaping() {
    let bindings = standard_bindings().unwrap();
    let map = bindings.type_by_path("rust::Map").unwrap();
    let multimap = bindings.type_by_path("rust::MultiMap").unwrap();
    let k = TypeRef::Parameter("K".to_owned());
    let v = TypeRef::Parameter("V".to_owned());

    let first = map
        .find_callable(
            CallStyle::Method,
            "with_first_in_range",
            &[
                TypeRef::shared_ref(k.clone()),
                TypeRef::shared_ref(k.clone()),
                TypeRef::callback(
                    CallbackKind::FnOnce,
                    stainless_compiler::interop::CallbackEscape::Call,
                    vec![
                        TypeRef::shared_ref(k.clone()),
                        TypeRef::shared_ref(v.clone()),
                    ],
                    TypeRef::Void,
                ),
            ],
        )
        .unwrap();
    assert_eq!(first.return_type, TypeRef::Bool);
    assert!(matches!(
        first.lowering,
        RustLowering::FunctionWithReceiver { ref rust_path }
            if rust_path == "::stainless_runtime::btree_map_with_first_in_range"
    ));

    let last = map
        .find_callable(
            CallStyle::Method,
            "with_last_in_range",
            &[
                TypeRef::shared_ref(k.clone()),
                TypeRef::shared_ref(k.clone()),
                TypeRef::callback(
                    CallbackKind::FnOnce,
                    stainless_compiler::interop::CallbackEscape::Call,
                    vec![
                        TypeRef::shared_ref(k.clone()),
                        TypeRef::shared_ref(v.clone()),
                    ],
                    TypeRef::Void,
                ),
            ],
        )
        .unwrap();
    assert_eq!(last.return_type, TypeRef::Bool);
    assert!(matches!(
        last.lowering,
        RustLowering::FunctionWithReceiver { ref rust_path }
            if rust_path == "::stainless_runtime::btree_map_with_last_in_range"
    ));

    let retain_key = map
        .find_callable(
            CallStyle::Method,
            "retain",
            &[TypeRef::callback(
                CallbackKind::FnMut,
                stainless_compiler::interop::CallbackEscape::Call,
                vec![TypeRef::shared_ref(k.clone())],
                TypeRef::Bool,
            )],
        )
        .unwrap();
    assert!(matches!(
        retain_key.lowering,
        RustLowering::FunctionWithReceiver { ref rust_path }
            if rust_path == "::stainless_runtime::btree_map_retain_keys"
    ));

    let with = multimap
        .callables
        .iter()
        .find(|callable| callable.source_name == "with")
        .unwrap();
    assert_eq!(with.return_type, TypeRef::Usize);
    assert!(matches!(
        &with.parameters[1].ty,
        TypeRef::Callback(callback)
            if callback.kind == CallbackKind::FnMut
                && callback.escape == stainless_compiler::interop::CallbackEscape::Call
                && callback.parameters == [TypeRef::shared_ref(v)]
    ));
}

#[test]
fn list_and_queue_expose_end_operations_with_mutable_receivers() {
    let bindings = standard_bindings().unwrap();
    let t = TypeRef::Parameter("T".to_owned());

    for path in ["rust::List", "rust::Queue"] {
        let binding = bindings.type_by_path(path).unwrap();
        for method_name in ["push_front", "push_back"] {
            let callable = binding
                .find_callable(CallStyle::Method, method_name, std::slice::from_ref(&t))
                .unwrap();
            assert_eq!(callable.receiver, Some(Receiver::Mutable));
            assert_eq!(callable.return_type, TypeRef::Void);
        }
        for method_name in ["pop_front", "pop_back"] {
            let callable = binding
                .find_callable(CallStyle::Method, method_name, &[])
                .unwrap();
            assert_eq!(callable.receiver, Some(Receiver::Mutable));
            assert_eq!(
                callable.return_type,
                TypeRef::native("rust::Option", vec![t.clone()])
            );
        }
    }
}

#[test]
fn regex_uses_generated_wrappers_and_proven_error_formatting() {
    let external = parse_bindings_manifest(include_str!(
        "../../../docs/ref/17_external_regex_wrapper.bindings.toml"
    ))
    .unwrap();
    let bindings = standard_bindings().unwrap().merge(external).unwrap();
    let regex = bindings.type_by_path("rust::regex::Regex").unwrap();
    let error = bindings.type_by_path("rust::regex::Error").unwrap();
    let string_ref = TypeRef::shared_ref(TypeRef::native("rust::String", vec![]));

    assert_eq!(error.error_format, Some(NativeErrorFormat::Display));
    let new = regex
        .find_callable(
            CallStyle::AssociatedFunction,
            "new",
            std::slice::from_ref(&string_ref),
        )
        .unwrap();
    let RustLowering::GeneratedWrapper {
        wrapper_name,
        target,
    } = &new.lowering
    else {
        panic!("Regex::new should use a generated wrapper");
    };
    assert!(wrapper_name.starts_with("__stainless_wrapper_v1_"));
    assert_eq!(
        target,
        &WrapperTarget::Function {
            rust_path: "::regex::Regex::new".to_owned(),
        }
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
    let RustLowering::GeneratedWrapper {
        wrapper_name,
        target,
    } = &is_match.lowering
    else {
        panic!("Regex::is_match should use a generated wrapper");
    };
    assert!(wrapper_name.starts_with("__stainless_wrapper_v1_"));
    assert_eq!(
        target,
        &WrapperTarget::Method {
            rust_name: "is_match".to_owned(),
        }
    );
}

#[test]
fn generated_wrapper_names_are_deterministic() {
    let source = include_str!("../../../docs/ref/17_external_regex_wrapper.bindings.toml");
    let first = parse_bindings_manifest(source).unwrap();
    let second = parse_bindings_manifest(source).unwrap();

    assert_eq!(
        first
            .types()
            .flat_map(|binding| &binding.callables)
            .map(|callable| &callable.lowering)
            .collect::<Vec<_>>(),
        second
            .types()
            .flat_map(|binding| &binding.callables)
            .map(|callable| &callable.lowering)
            .collect::<Vec<_>>()
    );
}

#[test]
fn invalid_generated_wrapper_metadata_is_rejected_before_lowering() {
    let wrapper = |source_name, style, receiver, target| CallableBinding {
        source_name,
        style,
        receiver,
        parameters: vec![],
        is_async: false,
        return_type: TypeRef::Bool,
        rust_result_error: None,
        return_borrow: None,
        requirements: vec![],
        lowering: RustLowering::GeneratedWrapper {
            wrapper_name: "__duplicate_wrapper".to_owned(),
            target,
        },
    };
    let binding = |callables| NativeTypeBinding {
        stainless_path: "rust::fixture::Type".to_owned(),
        rust_path: "::fixture::Type".to_owned(),
        type_parameters: vec![],
        error_format: None,
        callables,
    };

    let mismatched = wrapper(
        "bad".to_owned(),
        CallStyle::Method,
        Some(Receiver::Shared),
        WrapperTarget::Function {
            rust_path: "::fixture::Type::bad".to_owned(),
        },
    );
    assert!(matches!(
        NativeBindings::new(vec![binding(vec![mismatched])]),
        Err(BindingError::WrapperTargetMismatch { .. })
    ));

    let first = wrapper(
        "first".to_owned(),
        CallStyle::AssociatedFunction,
        None,
        WrapperTarget::Function {
            rust_path: "::fixture::Type::first".to_owned(),
        },
    );
    let second = wrapper(
        "second".to_owned(),
        CallStyle::AssociatedFunction,
        None,
        WrapperTarget::Function {
            rust_path: "::fixture::Type::second".to_owned(),
        },
    );
    assert_eq!(
        NativeBindings::new(vec![binding(vec![first, second])]),
        Err(BindingError::DuplicateWrapperName(
            "__duplicate_wrapper".to_owned()
        ))
    );
}

#[test]
fn vec_has_default_and_capacity_construction() {
    let bindings = standard_bindings().unwrap();
    let vec_binding = bindings.type_by_path("rust::Vec").unwrap();
    let vec_t = TypeRef::native("rust::Vec", vec![TypeRef::Parameter("T".to_owned())]);

    let constructor = vec_binding
        .find_callable(CallStyle::Constructor, "Vec", &[])
        .unwrap();
    assert_eq!(constructor.return_type, vec_t);
    assert_eq!(
        constructor.lowering,
        RustLowering::AssociatedFunction {
            rust_path: "::std::vec::Vec::new".to_owned()
        }
    );

    let with_capacity = vec_binding
        .find_callable(
            CallStyle::AssociatedFunction,
            "with_capacity",
            &[TypeRef::Usize],
        )
        .unwrap();
    assert_eq!(with_capacity.return_type, vec_t);
}

#[test]
fn vec_common_methods_have_expected_receiver_effects() {
    let bindings = standard_bindings().unwrap();
    let vec_binding = bindings.type_by_path("rust::Vec").unwrap();
    let t = TypeRef::Parameter("T".to_owned());

    let len = vec_binding
        .find_callable(CallStyle::Method, "len", &[])
        .unwrap();
    assert_eq!(len.receiver, Some(Receiver::Shared));
    assert_eq!(len.return_type, TypeRef::Usize);

    let push = vec_binding
        .find_callable(CallStyle::Method, "push", std::slice::from_ref(&t))
        .unwrap();
    assert_eq!(push.receiver, Some(Receiver::Mutable));
    assert_eq!(push.return_type, TypeRef::Void);

    let pop = vec_binding
        .find_callable(CallStyle::Method, "pop", &[])
        .unwrap();
    assert_eq!(pop.receiver, Some(Receiver::Mutable));
    assert_eq!(pop.return_type, TypeRef::native("rust::Option", vec![t]));

    let range_visitor = TypeRef::callback(
        CallbackKind::FnMut,
        stainless_compiler::interop::CallbackEscape::Call,
        vec![TypeRef::shared_ref(TypeRef::Parameter("T".to_owned()))],
        TypeRef::Void,
    );
    let with_range = vec_binding
        .find_callable(
            CallStyle::Method,
            "with_range",
            &[TypeRef::Usize, TypeRef::Usize, range_visitor],
        )
        .unwrap();
    assert_eq!(with_range.receiver, Some(Receiver::Shared));
    assert_eq!(with_range.return_type, TypeRef::Bool);
    assert!(matches!(
        with_range.lowering,
        RustLowering::FunctionWithReceiver { ref rust_path }
            if rust_path == "::stainless_runtime::vec_with_range"
    ));
}

#[test]
fn vec_trait_requirements_are_preserved() {
    let bindings = standard_bindings().unwrap();
    let vec_binding = bindings.type_by_path("rust::Vec").unwrap();
    let t = TypeRef::Parameter("T".to_owned());

    let extend = vec_binding
        .find_callable(
            CallStyle::Method,
            "extend_from_slice",
            &[TypeRef::shared_ref(TypeRef::native(
                "rust::Vec",
                vec![t.clone()],
            ))],
        )
        .unwrap();
    assert_eq!(extend.receiver, Some(Receiver::Mutable));
    assert_eq!(extend.requirements.len(), 1);
    assert_eq!(extend.requirements[0].rust_trait, "::core::clone::Clone");

    let contains = vec_binding
        .find_callable(CallStyle::Method, "contains", &[TypeRef::shared_ref(t)])
        .unwrap();
    assert_eq!(contains.requirements.len(), 1);
    assert_eq!(contains.requirements[0].parameter, "T");
    assert_eq!(
        contains.requirements[0].rust_trait,
        "::core::cmp::PartialEq"
    );

    let sort = vec_binding
        .find_callable(CallStyle::Method, "sort", &[])
        .unwrap();
    assert_eq!(sort.requirements[0].rust_trait, "::core::cmp::Ord");
}

#[test]
fn string_copy_constructor_is_explicit_clone_lowering() {
    let bindings = standard_bindings().unwrap();
    let string_binding = bindings.type_by_path("rust::String").unwrap();
    let string = TypeRef::native("rust::String", vec![]);

    let copy_constructor = string_binding
        .find_callable(
            CallStyle::Constructor,
            "String",
            &[TypeRef::shared_ref(string.clone())],
        )
        .unwrap();
    assert_eq!(copy_constructor.return_type, string);
    assert_eq!(
        copy_constructor.lowering,
        RustLowering::CloneArgument { index: 0 }
    );
}

#[test]
fn string_borrow_arguments_adapt_to_str_after_resolution() {
    let bindings = standard_bindings().unwrap();
    let string_binding = bindings.type_by_path("rust::String").unwrap();
    let string_ref = TypeRef::shared_ref(TypeRef::native("rust::String", vec![]));

    for method_name in [
        "push_str",
        "contains",
        "starts_with",
        "ends_with",
        "eq_ignore_ascii_case",
    ] {
        let callable = string_binding
            .find_callable(
                CallStyle::Method,
                method_name,
                std::slice::from_ref(&string_ref),
            )
            .unwrap();
        assert_eq!(
            callable.parameters[0].adaptation,
            ArgumentAdaptation::StringRefToStr
        );
    }

    let replace = string_binding
        .find_callable(
            CallStyle::Method,
            "replace",
            &[string_ref.clone(), string_ref],
        )
        .unwrap();
    assert!(
        replace
            .parameters
            .iter()
            .all(|parameter| parameter.adaptation == ArgumentAdaptation::StringRefToStr)
    );
}

#[test]
fn string_into_bytes_consumes_the_receiver() {
    let bindings = standard_bindings().unwrap();
    let string_binding = bindings.type_by_path("rust::String").unwrap();
    let into_bytes = string_binding
        .find_callable(CallStyle::Method, "into_bytes", &[])
        .unwrap();

    assert_eq!(into_bytes.receiver, Some(Receiver::Value));
    assert_eq!(
        into_bytes.return_type,
        TypeRef::native("rust::Vec", vec![TypeRef::U8])
    );
}

#[test]
fn native_call_matching_uses_exact_stainless_types() {
    let bindings = standard_bindings().unwrap();
    let vec_binding = bindings.type_by_path("rust::Vec").unwrap();
    let string_binding = bindings.type_by_path("rust::String").unwrap();
    let t = TypeRef::Parameter("T".to_owned());

    assert!(
        vec_binding
            .find_callable(CallStyle::Method, "push", std::slice::from_ref(&t),)
            .is_some()
    );
    assert!(
        vec_binding
            .find_callable(CallStyle::Method, "push", &[TypeRef::shared_ref(t)],)
            .is_none()
    );
    assert!(
        string_binding
            .find_callable(CallStyle::Method, "push", &[TypeRef::Char])
            .is_some()
    );
    assert!(
        string_binding
            .find_callable(CallStyle::Method, "push", &[TypeRef::U8])
            .is_none()
    );
}

#[test]
fn var_exposes_checked_shared_mutation_methods() {
    let bindings = standard_bindings().unwrap();
    let var_binding = bindings
        .type_by_path("rust::stainless_runtime::Var")
        .unwrap();
    let var = TypeRef::native("rust::stainless_runtime::Var", vec![]);
    let json_error = TypeRef::native("rust::stainless_runtime::JsonError", vec![]);

    let push = var_binding
        .find_callable(CallStyle::Method, "push", std::slice::from_ref(&var))
        .expect("var arrays expose push");
    assert_eq!(push.receiver, Some(Receiver::Mutable));
    assert_eq!(push.return_type, TypeRef::Void);
    assert_eq!(push.rust_result_error, Some(json_error.clone()));

    let string_ref = TypeRef::shared_ref(TypeRef::native("rust::String", vec![]));
    let set_field = var_binding
        .find_callable(CallStyle::Method, "set", &[string_ref, var])
        .expect("var objects expose dynamic member set");
    assert_eq!(set_field.rust_result_error, Some(json_error));
    assert!(matches!(
        &set_field.lowering,
        RustLowering::Method { rust_name } if rust_name == "set_field"
    ));
}

#[test]
fn unsupported_borrowing_and_iterator_methods_are_not_exposed() {
    let bindings = standard_bindings().unwrap();
    let vec_binding = bindings.type_by_path("rust::Vec").unwrap();
    let string_binding = bindings.type_by_path("rust::String").unwrap();

    let vec_methods = method_names(vec_binding);
    for unsupported in ["get", "first", "last", "iter", "iter_mut", "drain"] {
        assert!(!vec_methods.contains(unsupported));
    }

    let string_methods = method_names(string_binding);
    for unsupported in ["as_str", "as_bytes", "trim", "split", "chars", "bytes"] {
        assert!(!string_methods.contains(unsupported));
    }

    for path in [
        "rust::List",
        "rust::Map",
        "rust::MultiMap",
        "rust::Queue",
        "rust::Set",
    ] {
        let methods = method_names(bindings.type_by_path(path).unwrap());
        for unsupported in ["get", "get_mut", "iter", "iter_mut", "front", "back"] {
            assert!(
                !methods.contains(unsupported),
                "{path} exposed {unsupported}"
            );
        }
    }
}

fn method_names(binding: &stainless_compiler::interop::NativeTypeBinding) -> BTreeSet<&str> {
    binding
        .callables
        .iter()
        .filter(|callable| callable.style == CallStyle::Method)
        .map(|callable| callable.source_name.as_str())
        .collect()
}
